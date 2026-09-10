//! Claude Code CLI provider: drives `claude -p --output-format stream-json`.
//! Requires no API key when Claude Code is already logged in.
//!
//! Each stdout line is a JSON object. `stream_event` lines wrap the raw
//! Anthropic SSE events, so text/thinking deltas arrive incrementally; the
//! `assistant` and `result` lines then repeat the accumulated content, which is
//! why every text path goes through a [`TextAccumulator`] and the parser never
//! emits the same content twice.
//!
//! The session id from the `system`/`init` line is surfaced as
//! [`ChatDelta::Status`] with the [`SESSION_STATUS_PREFIX`] prefix.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::llm::providers::{
    format_cli_prompt, json_str, json_u32, send_delta, truncate_chars, TextAccumulator,
};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, ProviderConfig, ProviderKind};

/// Prefix used by the [`ChatDelta::Status`] message that carries a session id.
pub const SESSION_STATUS_PREFIX: &str = "session:";

const STDERR_LIMIT: usize = 8 * 1024;

pub struct ClaudeCliProvider {
    id: String,
    model: String,
    binary: PathBuf,
    permission_mode: String,
    extra_args: Vec<String>,
    timeout: Duration,
}

impl ClaudeCliProvider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        Ok(Self {
            id: cfg.id.clone(),
            model: cfg.model.clone(),
            binary: cfg
                .options
                .get("binary")
                .and_then(|v| v.as_str())
                .unwrap_or("claude")
                .into(),
            permission_mode: cfg
                .options
                .get("permissionMode")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string(),
            extra_args: cfg
                .options
                .get("extraArgs")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            timeout: Duration::from_secs(
                cfg.options
                    .get("timeoutSecs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(300),
            ),
        })
    }

    pub fn binary(&self) -> &PathBuf {
        &self.binary
    }

    fn command(&self) -> Command {
        let mut command = super::cli_command(&self.binary.to_string_lossy());
        command
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--include-partial-messages")
            .arg("--verbose");
        if !self.model.trim().is_empty() {
            command.arg("--model").arg(&self.model);
        }
        if !self.permission_mode.trim().is_empty() {
            command.arg("--permission-mode").arg(&self.permission_mode);
        }
        command.args(&self.extra_args);
        // No positional prompt: `claude -p` reads it from stdin.
        command
    }
}

#[derive(Debug, Default)]
struct StreamState {
    text: TextAccumulator,
    reasoning: TextAccumulator,
    session_id: Option<String>,
    finish_reason: Option<String>,
    done: bool,
}

async fn emit_text(
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
    text: &str,
    incremental: bool,
) -> Result<()> {
    let out = if incremental {
        state.text.push_delta(text)
    } else {
        state.text.push_final(text)
    };
    if let Some(text) = out {
        send_delta(tx, ChatDelta::Text { text }).await?;
    }
    Ok(())
}

async fn emit_reasoning(
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
    text: &str,
    incremental: bool,
) -> Result<()> {
    let out = if incremental {
        state.reasoning.push_delta(text)
    } else {
        state.reasoning.push_final(text)
    };
    if let Some(text) = out {
        send_delta(tx, ChatDelta::Reasoning { text }).await?;
    }
    Ok(())
}

/// Handle the wrapped Anthropic SSE event of a `stream_event` line.
async fn handle_stream_event(
    event: &Value,
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
) -> Result<()> {
    match json_str(event, &["type"]).unwrap_or_default() {
        "content_block_delta" => match json_str(event, &["delta", "type"]) {
            Some("text_delta") => {
                if let Some(text) = json_str(event, &["delta", "text"]) {
                    emit_text(tx, state, text, true).await?;
                }
            }
            Some("thinking_delta") => {
                if let Some(text) = json_str(event, &["delta", "thinking"]) {
                    emit_reasoning(tx, state, text, true).await?;
                }
            }
            _ => {}
        },
        "content_block_start" => {
            if json_str(event, &["content_block", "type"]) == Some("tool_use") {
                let name = json_str(event, &["content_block", "name"]).unwrap_or("tool");
                send_delta(
                    tx,
                    ChatDelta::Status {
                        message: format!("调用工具：{name}"),
                    },
                )
                .await?;
            }
        }
        "message_delta" => {
            if let Some(reason) = json_str(event, &["delta", "stop_reason"]) {
                state.finish_reason = Some(reason.to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Handle one JSONL line from `claude --output-format stream-json`.
async fn handle_line(
    line: &str,
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
) -> Result<()> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(());
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(%error, "ignoring non-JSON claude output line");
            return Ok(());
        }
    };
    match json_str(&value, &["type"]).unwrap_or_default() {
        "system" => {
            let session_id = json_str(&value, &["session_id"])
                .filter(|_| json_str(&value, &["subtype"]) == Some("init"))
                .filter(|id| state.session_id.as_deref() != Some(*id));
            if let Some(session_id) = session_id {
                state.session_id = Some(session_id.to_string());
                send_delta(
                    tx,
                    ChatDelta::Status {
                        message: format!("{SESSION_STATUS_PREFIX}{session_id}"),
                    },
                )
                .await?;
            }
        }
        "stream_event" => {
            if let Some(event) = value.get("event") {
                handle_stream_event(event, tx, state).await?;
            }
        }
        "assistant" | "user" => {
            if let Some(content) = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
            {
                for block in content {
                    match json_str(block, &["type"]).unwrap_or_default() {
                        "text" => {
                            if let Some(text) = json_str(block, &["text"]) {
                                emit_text(tx, state, text, false).await?;
                            }
                        }
                        "thinking" => {
                            if let Some(text) = json_str(block, &["thinking"]) {
                                emit_reasoning(tx, state, text, false).await?;
                            }
                        }
                        "tool_use" => {
                            let name = json_str(block, &["name"]).unwrap_or("tool");
                            send_delta(
                                tx,
                                ChatDelta::Status {
                                    message: format!("调用工具：{name}"),
                                },
                            )
                            .await?;
                        }
                        "tool_result" => {
                            let snippet = match block.get("content") {
                                Some(Value::String(text)) => {
                                    format!("：{}", truncate_chars(text, 120))
                                }
                                Some(Value::Array(parts)) => {
                                    let text: String = parts
                                        .iter()
                                        .filter_map(|part| json_str(part, &["text"]))
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    if text.is_empty() {
                                        String::new()
                                    } else {
                                        format!("：{}", truncate_chars(&text, 120))
                                    }
                                }
                                _ => String::new(),
                            };
                            let failed = block
                                .get("is_error")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let label = if failed {
                                "工具失败"
                            } else {
                                "工具结果"
                            };
                            send_delta(
                                tx,
                                ChatDelta::Status {
                                    message: format!("{label}{snippet}"),
                                },
                            )
                            .await?;
                        }
                        _ => {}
                    }
                }
            }
        }
        "result" => {
            let subtype = json_str(&value, &["subtype"]).unwrap_or("success");
            let is_error = value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_error || subtype.starts_with("error") {
                let message = json_str(&value, &["error"])
                    .or_else(|| json_str(&value, &["result"]))
                    .unwrap_or(subtype);
                return Err(Error::provider(format!(
                    "claude failed ({subtype}): {message}"
                )));
            }
            if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
                send_delta(
                    tx,
                    ChatDelta::Usage {
                        input_tokens: json_u32(usage, &["input_tokens"]).unwrap_or(0),
                        output_tokens: json_u32(usage, &["output_tokens"]).unwrap_or(0),
                    },
                )
                .await?;
            }
            // `result` repeats the final answer; only the not-yet-streamed
            // suffix (or the whole thing when partial messages were absent) is
            // emitted.
            if let Some(text) = json_str(&value, &["result"]) {
                emit_text(tx, state, text, false).await?;
            }
            if state.finish_reason.is_none() {
                state.finish_reason = json_str(&value, &["stop_reason"]).map(str::to_string);
            }
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: state.finish_reason.take(),
                },
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

#[async_trait::async_trait]
impl ChatProvider for ClaudeCliProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::ClaudeCli
    }

    async fn stream(
        &self,
        request: ChatRequest,
        tx: mpsc::Sender<ChatDelta>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let prompt = format_cli_prompt(&request);
        let mut command = self.command();
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| {
            Error::provider(format!(
                "cannot start claude ({}): {e}; is the CLI installed?",
                self.binary.display()
            ))
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            // Best effort: a broken pipe means the CLI already exited, and the
            // exit status / stderr handling below reports the real problem.
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }

        let stderr = Arc::new(parking_lot::Mutex::new(String::new()));
        let mut stderr_task = {
            let stderr = Arc::clone(&stderr);
            let pipe = child.stderr.take();
            tokio::spawn(async move {
                let Some(pipe) = pipe else { return };
                let mut reader = BufReader::new(pipe);
                let mut line = String::new();
                while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
                    let mut buffer = stderr.lock();
                    if buffer.len() < STDERR_LIMIT {
                        buffer.push_str(&line);
                    }
                    line.clear();
                }
            })
        };

        let Some(stdout) = child.stdout.take() else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            stderr_task.abort();
            return Err(Error::provider("claude did not expose stdout"));
        };
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let deadline = Instant::now() + self.timeout;
        let mut state = StreamState::default();
        let mut saw_line = false;
        let mut result = Ok(());

        loop {
            line.clear();
            let read = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    result = Err(Error::Cancelled);
                    break;
                }
                () = tokio::time::sleep_until(deadline) => {
                    result = Err(Error::provider(format!(
                        "claude timed out after {}s",
                        self.timeout.as_secs()
                    )));
                    break;
                }
                read = reader.read_line(&mut line) => read,
            };
            match read {
                Ok(0) => break,
                Ok(_) => {
                    saw_line = true;
                    if let Err(error) = handle_line(&line, &tx, &mut state).await {
                        result = Err(error);
                        break;
                    }
                }
                Err(error) => {
                    result = Err(Error::provider(format!(
                        "cannot read claude output: {error}"
                    )));
                    break;
                }
            }
        }

        let status = {
            let _ = child.start_kill();
            child.wait().await
        };
        // Only wait for stderr when the run itself succeeded: on cancellation
        // or timeout a grandchild may still hold the pipe open and promptness
        // matters more than the trailing diagnostics.
        match &result {
            Ok(()) => {
                if tokio::time::timeout(Duration::from_millis(100), &mut stderr_task)
                    .await
                    .is_err()
                {
                    stderr_task.abort();
                }
            }
            Err(_) => stderr_task.abort(),
        }
        let stderr_text = truncate_chars(stderr.lock().trim(), 512);

        result?;
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => {
                return Err(Error::provider(format!(
                    "claude exited with {status}: {stderr_text}"
                )));
            }
            Err(error) => {
                return Err(Error::provider(format!("cannot wait for claude: {error}")));
            }
        }
        if !saw_line {
            return Err(Error::provider(format!(
                "claude produced no output: {stderr_text}"
            )));
        }
        if !state.done {
            // Process exited without a `result` line: close the turn.
            send_delta(
                &tx,
                ChatDelta::Done {
                    finish_reason: state.finish_reason.take(),
                },
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    fn provider(options: &[(&str, Value)]) -> ClaudeCliProvider {
        let mut cfg = ProviderConfig::new("claude", ProviderKind::ClaudeCli, "claude-sonnet-4-5");
        for (key, value) in options {
            cfg.options.insert((*key).to_string(), value.clone());
        }
        ClaudeCliProvider::new(&cfg).unwrap()
    }

    #[test]
    fn builds_expected_argv() {
        let provider = provider(&[("permissionMode", Value::String("plan".into()))]);
        let args: Vec<String> = provider
            .command()
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--verbose",
                "--model",
                "claude-sonnet-4-5",
                "--permission-mode",
                "plan",
            ]
        );
    }

    #[tokio::test]
    async fn stream_events_and_assistant_fallback_do_not_duplicate() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut state = StreamState::default();
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"s-1"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"lo"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok","is_error":false}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}}}"#,
            r#"{"type":"result","subtype":"success","result":"Hello","stop_reason":"end_turn","usage":{"input_tokens":11,"output_tokens":2}}"#,
            "not json",
        ];
        for line in lines {
            handle_line(line, &tx, &mut state).await.unwrap();
        }
        drop(tx);
        let mut deltas = Vec::new();
        while let Some(delta) = rx.recv().await {
            deltas.push(delta);
        }
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Status {
                    message: "session:s-1".into()
                },
                ChatDelta::Reasoning { text: "hmm".into() },
                ChatDelta::Text { text: "Hel".into() },
                ChatDelta::Text { text: "lo".into() },
                ChatDelta::Status {
                    message: "工具结果：ok".into()
                },
                ChatDelta::Usage {
                    input_tokens: 11,
                    output_tokens: 2
                },
                ChatDelta::Done {
                    finish_reason: Some("end_turn".into())
                },
            ]
        );
    }

    #[tokio::test]
    async fn result_only_output_is_emitted_once() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut state = StreamState::default();
        let line = r#"{"type":"result","subtype":"success","result":"Hello there","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}"#;
        handle_line(line, &tx, &mut state).await.unwrap();
        drop(tx);
        let mut deltas = Vec::new();
        while let Some(delta) = rx.recv().await {
            deltas.push(delta);
        }
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Usage {
                    input_tokens: 1,
                    output_tokens: 2
                },
                ChatDelta::Text {
                    text: "Hello there".into()
                },
                ChatDelta::Done {
                    finish_reason: Some("end_turn".into())
                },
            ]
        );
    }

    #[tokio::test]
    async fn error_result_becomes_err() {
        let (tx, _rx) = mpsc::channel(4);
        let mut state = StreamState::default();
        let error = handle_line(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"}"#,
            &tx,
            &mut state,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn formats_prompt_with_system_and_roles() {
        let mut request = ChatRequest::new("claude-sonnet-4-5", "be nice");
        request.messages.push(ChatMessage::user("hi"));
        let prompt = format_cli_prompt(&request);
        assert!(prompt.contains("user: hi"));
    }
}
