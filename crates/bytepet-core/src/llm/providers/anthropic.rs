//! Anthropic Messages API provider (`POST /v1/messages`, SSE).
//!
//! Protocol notes:
//! - `max_tokens` is required by the API; when the caller leaves it unset we
//!   send 1024.
//! - Anthropic only accepts `user`/`assistant` roles inside `messages`; the
//!   system prompt is a separate top-level field.
//! - Frames carry both an SSE `event:` name and a JSON `type`; we key off the
//!   JSON field and fall back to the event name, so a proxy that drops the
//!   `event:` line still decodes.

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::llm::providers::{apply_extra_headers, check_status, json_str, json_u32, send_delta};
use crate::llm::sse::{parse_json, SseDecoder, SseEvent};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, ProviderConfig, ProviderKind, Role};
use crate::secrets::SecretStore;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 1024;

pub struct AnthropicProvider {
    id: String,
    model: String,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(cfg: &ProviderConfig, secrets: &dyn SecretStore) -> Result<Self> {
        let api_key = crate::llm::resolve_api_key(cfg, secrets)?;
        Ok(Self {
            id: cfg.id.clone(),
            model: cfg.model.clone(),
            base_url: cfg.base_url(),
            api_key,
            extra_headers: cfg
                .extra_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            client: super::http_client()
                .map_err(|e| Error::provider(format!("cannot build http client: {e}")))?,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn body(&self, request: &ChatRequest) -> Value {
        let model = if request.model.trim().is_empty() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                let role = match message.role {
                    Role::Assistant => "assistant",
                    // The Messages API has no system role inside `messages`.
                    Role::System | Role::User => "user",
                };
                json!({ "role": role, "content": message.content })
            })
            .collect();
        let mut body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            "stream": true,
        });
        if !request.system.trim().is_empty() {
            body["system"] = json!(request.system);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if !request.stop.is_empty() {
            body["stop_sequences"] = json!(request.stop);
        }
        body
    }
}

/// Mutable state carried across one Anthropic SSE stream.
#[derive(Debug, Default)]
struct StreamState {
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<String>,
    done: bool,
}

/// Translate one decoded SSE frame into zero or more [`ChatDelta`]s.
async fn handle_event(
    event: SseEvent,
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
) -> Result<()> {
    if event.is_done() {
        return Ok(());
    }
    let data = event.data.trim();
    if data.is_empty() {
        return Ok(());
    }
    // Tolerate keep-alives and non-JSON frames; the stream terminator decides.
    let value: Value = match parse_json(data) {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(%error, "ignoring undecodable anthropic stream frame");
            return Ok(());
        }
    };
    let kind = json_str(&value, &["type"])
        .or(event.event.as_deref())
        .unwrap_or_default();

    match kind {
        "message_start" => {
            if let Some(input) = json_u32(&value, &["message", "usage", "input_tokens"]) {
                state.input_tokens = input;
            }
            if value.get("message").and_then(|m| m.get("usage")).is_some() {
                send_delta(
                    tx,
                    ChatDelta::Usage {
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                    },
                )
                .await?;
            }
        }
        "content_block_start" => match json_str(&value, &["content_block", "type"]) {
            // Real streams start blocks empty, but a gateway that inlines the
            // first chunk here must not lose it.
            Some("text") => {
                let text = json_str(&value, &["content_block", "text"])
                    .filter(|text| !text.is_empty());
                if let Some(text) = text {
                    send_delta(tx, ChatDelta::Text { text: text.to_string() }).await?;
                }
            }
            Some("thinking") => {
                let text = json_str(&value, &["content_block", "thinking"])
                    .filter(|text| !text.is_empty());
                if let Some(text) = text {
                    send_delta(tx, ChatDelta::Reasoning { text: text.to_string() }).await?;
                }
            }
            Some("tool_use") => {
                let name = json_str(&value, &["content_block", "name"]).unwrap_or("tool");
                send_delta(
                    tx,
                    ChatDelta::Status {
                        message: format!("调用工具：{name}"),
                    },
                )
                .await?;
            }
            _ => {}
        },
        "content_block_delta" => match json_str(&value, &["delta", "type"]) {
            Some("text_delta") => {
                if let Some(text) = json_str(&value, &["delta", "text"]) {
                    send_delta(tx, ChatDelta::Text { text: text.to_string() }).await?;
                }
            }
            Some("thinking_delta") => {
                if let Some(text) = json_str(&value, &["delta", "thinking"]) {
                    send_delta(tx, ChatDelta::Reasoning { text: text.to_string() }).await?;
                }
            }
            // `input_json_delta` streams tool arguments; nothing to render.
            _ => {}
        },
        "message_delta" => {
            if let Some(input) = json_u32(&value, &["usage", "input_tokens"]) {
                state.input_tokens = input;
            }
            if let Some(output) = json_u32(&value, &["usage", "output_tokens"]) {
                state.output_tokens = output;
            }
            if let Some(reason) = json_str(&value, &["delta", "stop_reason"]) {
                state.stop_reason = Some(reason.to_string());
            }
            if value.get("usage").is_some() {
                send_delta(
                    tx,
                    ChatDelta::Usage {
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                    },
                )
                .await?;
            }
        }
        "message_stop" => {
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: state.stop_reason.take(),
                },
            )
            .await?;
        }
        "error" => {
            let error_type = json_str(&value, &["error", "type"]).unwrap_or("error");
            let message = json_str(&value, &["error", "message"]).unwrap_or("unknown error");
            return Err(Error::provider(format!(
                "anthropic stream error ({error_type}): {message}"
            )));
        }
        _ => {}
    }
    Ok(())
}

#[async_trait::async_trait]
impl ChatProvider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    async fn stream(
        &self,
        request: ChatRequest,
        tx: mpsc::Sender<ChatDelta>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let body = self.body(&request);
        let request_builder = self
            .client
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body);
        let request_builder = apply_extra_headers(request_builder, &self.extra_headers)?;
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::provider(format!("anthropic request failed: {e}")))?;
        let response = check_status(response, "anthropic", &self.api_key).await?;

        let mut state = StreamState::default();
        let mut decoder = SseDecoder::new();
        let mut stream = response.bytes_stream();

        loop {
            let chunk = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(Error::Cancelled),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| Error::provider(format!("anthropic stream failed: {e}")))?;
            let text = String::from_utf8_lossy(&chunk);
            for event in decoder.push(&text) {
                handle_event(event, &tx, &mut state).await?;
            }
        }
        for event in decoder.finish() {
            handle_event(event, &tx, &mut state).await?;
        }
        if !state.done {
            // The connection closed without `message_stop`: close the turn.
            send_delta(
                &tx,
                ChatDelta::Done {
                    finish_reason: state.stop_reason.take(),
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
    use crate::secrets::MemorySecretStore;

    fn provider() -> AnthropicProvider {
        let cfg = ProviderConfig::new("anthropic", ProviderKind::Anthropic, "claude-test");
        let secrets = MemorySecretStore::new();
        secrets.set("provider/anthropic", "sk-ant-secret").unwrap();
        AnthropicProvider::new(&cfg, &secrets).unwrap()
    }

    #[test]
    fn body_uses_default_max_tokens_and_splits_system() {
        let mut request = ChatRequest::new("claude-x", "be nice");
        request.messages.push(crate::llm::ChatMessage::user("hi"));
        request.messages.push(crate::llm::ChatMessage::assistant("hello"));
        request.stop.push("STOP".into());
        let body = provider().body(&request);
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "be nice");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stop_sequences"][0], "STOP");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
        assert_eq!(body["messages"][1]["role"], "assistant");
    }

    #[tokio::test]
    async fn maps_thinking_text_usage_and_done() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::default();
        let frames = [
            r#"{"type":"message_start","message":{"usage":{"input_tokens":7,"output_tokens":1}}}"#,
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        for data in frames {
            handle_event(SseEvent { data: data.into(), ..Default::default() }, &tx, &mut state)
                .await
                .unwrap();
        }
        drop(tx);
        let mut deltas = Vec::new();
        while let Some(delta) = rx.recv().await {
            deltas.push(delta);
        }
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Usage { input_tokens: 7, output_tokens: 0 },
                ChatDelta::Reasoning { text: "hmm".into() },
                ChatDelta::Text { text: "hi".into() },
                ChatDelta::Usage { input_tokens: 7, output_tokens: 3 },
                ChatDelta::Done { finish_reason: Some("end_turn".into()) },
            ]
        );
    }

    #[tokio::test]
    async fn error_frame_becomes_err() {
        let (tx, _rx) = mpsc::channel(4);
        let mut state = StreamState::default();
        let error = handle_event(
            SseEvent {
                data: r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#
                    .into(),
                ..Default::default()
            },
            &tx,
            &mut state,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("overloaded_error"));
        assert!(error.to_string().contains("busy"));
    }
}
