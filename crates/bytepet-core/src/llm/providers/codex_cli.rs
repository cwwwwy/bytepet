//! Codex CLI provider: drives the locally installed `codex exec --json` and
//! consumes its JSONL event stream. Requires no API key when the user is
//! already logged in to Codex.
//!
//! Session continuity: when `options["resumeSessionId"]` is set the provider
//! runs `codex exec resume <id> ...` instead of starting a new thread. The
//! session id discovered in `thread.started` / `session.created` is surfaced as
//! [`ChatDelta::Status`] with the [`SESSION_STATUS_PREFIX`] prefix (the caller
//! can store it and feed it back through `resumeSessionId` on the next turn);
//! it is also kept in the provider's stream state for the caller to read from
//! the emitted status.
//!
//! The JSONL parser is deliberately tolerant: unknown `type` values, unknown
//! item types and even non-JSON lines are ignored, because Codex adds event
//! kinds between releases.

use std::collections::HashSet;
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

pub struct CodexCliProvider {
    id: String,
    model: String,
    binary: PathBuf,
    extra_args: Vec<String>,
    sandbox: String,
    resume_session_id: Option<String>,
    timeout: Duration,
}

impl CodexCliProvider {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        Ok(Self {
            id: cfg.id.clone(),
            model: cfg.model.clone(),
            binary: cfg
                .options
                .get("binary")
                .and_then(|v| v.as_str())
                .unwrap_or("codex")
                .into(),
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
            sandbox: cfg
                .options
                .get("sandbox")
                .and_then(|v| v.as_str())
                .unwrap_or("read-only")
                .to_string(),
            resume_session_id: cfg
                .options
                .get("resumeSessionId")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string),
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
        command.arg("exec");
        if let Some(session_id) = &self.resume_session_id {
            command.arg("resume").arg(session_id);
        }
        command.arg("--json");
        if !self.model.trim().is_empty() {
            command.arg("--model").arg(&self.model);
        }
        // `codex exec resume` does not accept `--sandbox`, so it is only passed
        // when starting a fresh thread.
        if self.resume_session_id.is_none() && !self.sandbox.trim().is_empty() {
            command.arg("--sandbox").arg(&self.sandbox);
        }
        command.args(&self.extra_args);
        // `-` makes `codex exec` read the prompt from stdin.
        command.arg("-");
        command
    }
}

#[derive(Debug, Default)]
struct StreamState {
    text: TextAccumulator,
    reasoning: TextAccumulator,
    session_id: Option<String>,
    /// Item ids whose tool/command status has already been announced.
    announced: HashSet<String>,
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

/// Announce a command/tool at most once per item id.
async fn announce_once(
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
    item: &Value,
    message: String,
) -> Result<()> {
    let id = json_str(item, &["id"]).unwrap_or_default().to_string();
    if id.is_empty() || state.announced.insert(id) {
        send_delta(tx, ChatDelta::Status { message }).await?;
    }
    Ok(())
}

async fn handle_item(
    value: &Value,
    event_kind: &str,
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
) -> Result<()> {
    let Some(item) = value.get("item") else {
        return Ok(());
    };
    let item_type = json_str(item, &["type"]).unwrap_or_default();
    // An explicit `delta` field is incremental; a `text` field is the
    // accumulated content (Codex only emits `agent_message` on completion).
    let delta = json_str(item, &["delta"]).or_else(|| json_str(value, &["delta"]));
    match item_type {
        "agent_message" | "agentMessage" => {
            if let Some(delta) = delta {
                emit_text(tx, state, delta, true).await?;
            } else if let Some(text) = json_str(item, &["text"]) {
                emit_text(tx, state, text, false).await?;
            }
        }
        "reasoning" | "agent_reasoning" | "agentReasoning" => {
            if let Some(delta) = delta {
                emit_reasoning(tx, state, delta, true).await?;
            } else if let Some(text) = json_str(item, &["text"]) {
                emit_reasoning(tx, state, text, false).await?;
            }
        }
        "command_execution" | "commandExecution" => {
            let announce = event_kind == "item.started" || event_kind == "item.completed";
            if let Some(command) = json_str(item, &["command"]).filter(|_| announce) {
                let message = format!("执行命令：{}", truncate_chars(command, 120));
                announce_once(tx, state, item, message).await?;
            }
        }
        "mcp_tool_call" | "mcpToolCall" => {
            if let Some(tool) = json_str(item, &["tool"]) {
                let message = match json_str(item, &["server"]) {
                    Some(server) => format!("调用工具：{server}/{tool}"),
                    None => format!("调用工具：{tool}"),
                };
                announce_once(tx, state, item, message).await?;
            }
        }
        "file_change" | "fileChange" => {
            let paths = if event_kind == "item.completed" {
                item.get("changes")
                    .and_then(Value::as_array)
                    .map(|changes| {
                        changes
                            .iter()
                            .filter_map(|change| json_str(change, &["path"]))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if !paths.is_empty() {
                send_delta(
                    tx,
                    ChatDelta::Status {
                        message: format!("修改文件：{paths}"),
                    },
                )
                .await?;
            }
        }
        "error" => {
            let message = json_str(item, &["message"]).unwrap_or("unknown error");
            return Err(Error::provider(format!("codex item error: {message}")));
        }
        _ => {}
    }
    Ok(())
}

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
            tracing::debug!(%error, "ignoring non-JSON codex output line");
            return Ok(());
        }
    };
    let kind = json_str(&value, &["type"]).unwrap_or_default();
    match kind {
        "thread.started" | "session.created" | "session.started" => {
            let session_id = json_str(&value, &["thread_id"])
                .or_else(|| json_str(&value, &["session_id"]))
                .or_else(|| json_str(&value, &["sessionId"]))
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
        "turn.completed" => {
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
        }
        "turn.failed" => {
            let message = json_str(&value, &["error", "message"]).unwrap_or("turn failed");
            return Err(Error::provider(format!("codex turn failed: {message}")));
        }
        "error" => {
            let message = json_str(&value, &["message"]).unwrap_or("unknown error");
            // Codex reports transient reconnects as `error`; keep streaming.
            if message.starts_with("Reconnecting") {
                send_delta(
                    tx,
                    ChatDelta::Status {
                        message: message.to_string(),
                    },
                )
                .await?;
            } else {
                return Err(Error::provider(format!("codex stream error: {message}")));
            }
        }
        "agent_message_delta" | "item.delta" => {
            if let Some(delta) = json_str(&value, &["delta"]) {
                emit_text(tx, state, delta, true).await?;
            }
        }
        "reasoning_delta" | "agent_reasoning_delta" => {
            if let Some(delta) = json_str(&value, &["delta"]) {
                emit_reasoning(tx, state, delta, true).await?;
            }
        }
        "item.started" | "item.updated" | "item.completed" => {
            handle_item(&value, kind, tx, state).await?;
        }
        // Unknown event types are ignored on purpose.
        _ => {}
    }
    Ok(())
}

#[async_trait::async_trait]
impl ChatProvider for CodexCliProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::CodexCli
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
                "cannot start codex ({}): {e}; is the CLI installed?",
                self.binary.display()
            ))
        })?;

        // Feed the prompt and close stdin so the CLI knows it is complete.
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
            return Err(Error::provider("codex did not expose stdout"));
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
                        "codex timed out after {}s",
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
                    result = Err(Error::provider(format!("cannot read codex output: {error}")));
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
                    "codex exited with {status}: {stderr_text}"
                )));
            }
            Err(error) => {
                return Err(Error::provider(format!("cannot wait for codex: {error}")));
            }
        }
        if !saw_line {
            return Err(Error::provider(format!(
                "codex produced no output: {stderr_text}"
            )));
        }
        send_delta(&tx, ChatDelta::Done { finish_reason: None }).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    fn provider(options: &[(&str, Value)]) -> CodexCliProvider {
        let mut cfg = ProviderConfig::new("codex", ProviderKind::CodexCli, "gpt-5-codex");
        for (key, value) in options {
            cfg.options.insert((*key).to_string(), value.clone());
        }
        CodexCliProvider::new(&cfg).unwrap()
    }

    #[test]
    fn builds_resume_command_without_sandbox_flag() {
        let provider = provider(&[("resumeSessionId", Value::String("abc-123".into()))]);
        let command = provider.command();
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec!["exec", "resume", "abc-123", "--json", "--model", "gpt-5-codex", "-"]
        );
    }

    #[test]
    fn builds_fresh_command_with_sandbox_flag() {
        let provider = provider(&[("sandbox", Value::String("read-only".into()))]);
        let args: Vec<String> = provider
            .command()
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "exec", "--json", "--model", "gpt-5-codex", "--sandbox", "read-only", "-"
            ]
        );
    }

    #[tokio::test]
    async fn completed_message_is_emitted_exactly_once() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::default();
        let lines = [
            r#"{"type":"thread.started","thread_id":"t-1"}"#,
            r#"{"type":"item.completed","item":{"id":"i0","type":"reasoning","text":"why"}}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"hi"}}"#,
            // A duplicate completion (or a later item.updated with the same
            // accumulated text) must not repeat the message.
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"hi"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":3,"output_tokens":1}}"#,
            "not json at all",
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
                ChatDelta::Status { message: "session:t-1".into() },
                ChatDelta::Reasoning { text: "why".into() },
                ChatDelta::Text { text: "hi".into() },
                ChatDelta::Usage { input_tokens: 3, output_tokens: 1 },
            ]
        );
    }

    #[tokio::test]
    async fn deltas_then_completed_do_not_duplicate() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::default();
        let lines = [
            r#"{"type":"agent_message_delta","delta":"Hel"}"#,
            r#"{"type":"agent_message_delta","delta":"lo"}"#,
            r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"Hello"}}"#,
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
                ChatDelta::Text { text: "Hel".into() },
                ChatDelta::Text { text: "lo".into() },
            ]
        );
    }

    #[test]
    fn formats_prompt_with_system_and_roles() {
        let mut request = ChatRequest::new("gpt-5-codex", "be nice");
        request.messages.push(ChatMessage::user("hi"));
        request.messages.push(ChatMessage::assistant("hello"));
        let prompt = format_cli_prompt(&request);
        assert!(prompt.starts_with("be nice\n\n"));
        assert!(prompt.contains("user: hi\n"));
        assert!(prompt.contains("assistant: hello\n"));
    }
}
