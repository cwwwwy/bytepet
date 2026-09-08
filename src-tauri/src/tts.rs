//! Text-to-speech playback.
//!
//! Uses the platform's built-in synthesizer so there is no extra runtime
//! dependency: `say` on macOS, `System.Speech` on Windows, `spd-say` on Linux.
//! Playback is cancellable and only ever speaks the final reply.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

use crate::events::TTS_STATE;

struct Inner {
    speaking: AtomicBool,
    /// Incremented for every new utterance so stale watchers exit.
    generation: AtomicU64,
    child: Mutex<Option<std::process::Child>>,
}

/// Cloneable handle to the TTS player.
#[derive(Clone)]
pub struct Tts {
    inner: Arc<Inner>,
}

impl Default for Tts {
    fn default() -> Self {
        Self::new()
    }
}

impl Tts {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                speaking: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                child: Mutex::new(None),
            }),
        }
    }

    /// Speak `text`, stopping any current utterance.
    pub fn speak<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        text: &str,
        voice: Option<String>,
        rate: f32,
        max_chars: usize,
    ) {
        let cleaned = prepare_for_speech(text, max_chars);
        if cleaned.is_empty() {
            return;
        }
        self.stop(app);

        let child = match spawn_speaker(&cleaned, voice.as_deref(), rate.clamp(0.5, 2.5)) {
            Ok(child) => child,
            Err(err) => {
                tracing::warn!(%err, "TTS is unavailable on this system");
                return;
            }
        };

        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.inner.child.lock() = Some(child);
        self.inner.speaking.store(true, Ordering::Relaxed);
        let _ = app.emit(TTS_STATE, serde_json::json!({ "speaking": true }));

        // Watcher: report completion without blocking the caller.
        let inner = self.inner.clone();
        let app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(120));
            if inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let mut guard = inner.child.lock();
            let finished = match guard.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(_) => true,
                },
                None => true,
            };
            if finished {
                *guard = None;
                drop(guard);
                inner.speaking.store(false, Ordering::Relaxed);
                let _ = app.emit(TTS_STATE, serde_json::json!({ "speaking": false }));
                return;
            }
        });
    }

    /// Stop playback immediately.
    pub fn stop<R: Runtime>(&self, app: &AppHandle<R>) {
        // Invalidate any watcher first.
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(mut child) = self.inner.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if self.inner.speaking.swap(false, Ordering::Relaxed) {
            let _ = app.emit(TTS_STATE, serde_json::json!({ "speaking": false }));
        }
    }
}

/// Strip Markdown noise and cap the length so the synthesizer stays responsive.
pub fn prepare_for_speech(text: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let mut line_out = String::new();
        for ch in trimmed.chars() {
            match ch {
                '`' | '*' | '_' | '#' | '>' | '~' | '|' | '[' | ']' => {}
                _ => line_out.push(ch),
            }
        }
        let line_out = line_out.replace("https://", "").replace("http://", "");
        if !line_out.trim().is_empty() {
            out.push_str(line_out.trim());
            out.push('\n');
        }
    }
    let mut result = out.trim().to_string();
    if result.chars().count() > max_chars {
        result = result.chars().take(max_chars).collect::<String>();
        result.push('…');
    }
    result
}

fn spawn_speaker(
    text: &str,
    voice: Option<&str>,
    rate: f32,
) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("say");
        if let Some(voice) = voice.filter(|v| !v.trim().is_empty()) {
            cmd.arg("-v").arg(voice);
        }
        let wpm = (175.0 * rate).round() as i64;
        cmd.arg("-r").arg(wpm.to_string());
        cmd.arg(text);
        return cmd.spawn();
    }

    #[cfg(target_os = "windows")]
    {
        // Write the text to a temp file so no shell quoting is involved.
        let path = std::env::temp_dir().join(format!("pet-tts-{}.txt", std::process::id()));
        std::fs::write(&path, text)?;
        let rate_arg = ((rate - 1.0) * 5.0).round() as i64;
        let select = voice
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!("$s.SelectVoice('{}');", v.replace('\'', "")))
            .unwrap_or_default();
        let script = format!(
            "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $s.Rate = {rate_arg}; {select} $s.Speak([IO.File]::ReadAllText('{}'))",
            path.display()
        );
        return std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let rate_arg = (rate * 100.0).round() as i64;
        let mut cmd = std::process::Command::new("spd-say");
        cmd.arg("-r").arg(rate_arg.to_string());
        if let Some(voice) = voice.filter(|v| !v.trim().is_empty()) {
            cmd.arg("-y").arg(voice);
        }
        cmd.arg(text);
        cmd.spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markdown_and_code() {
        let text =
            "# 标题\n\n这是 **重点** 和 `代码`。\n\n```rust\nfn main() {}\n```\n\n链接 https://example.com/x";
        let out = prepare_for_speech(text, 400);
        assert!(out.contains("标题"));
        assert!(out.contains("重点"));
        assert!(!out.contains("**"));
        assert!(!out.contains("fn main"));
        assert!(!out.contains("https://"));
    }

    #[test]
    fn truncates_long_text() {
        let out = prepare_for_speech(&"啊".repeat(500), 100);
        assert!(out.chars().count() <= 101);
        assert!(out.ends_with('…'));
    }
}
