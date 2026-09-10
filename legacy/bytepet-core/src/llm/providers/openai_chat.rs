//! OpenAI-compatible chat completions provider (`POST /chat/completions`, SSE).
//!
//! Works with OpenAI, DeepSeek, Moonshot, OpenRouter and the many gateways
//! that clone this wire format. Two gateway quirks are handled explicitly:
//! reasoning deltas (`reasoning_content` for DeepSeek, `reasoning` for
//! OpenRouter) and usage chunks that arrive with an empty `choices` array.

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::llm::providers::{apply_extra_headers, check_status, json_str, json_u32, send_delta};
use crate::llm::sse::{parse_json, SseDecoder, SseEvent};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, ProviderConfig, ProviderKind, Role};
use crate::secrets::SecretStore;

pub struct OpenAiChatProvider {
    id: String,
    model: String,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
}

impl OpenAiChatProvider {
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
        format!("{}/chat/completions", self.base_url)
    }

    fn body(&self, request: &ChatRequest) -> Value {
        let model = if request.model.trim().is_empty() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };
        let mut messages: Vec<Value> = Vec::with_capacity(request.messages.len() + 1);
        if !request.system.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": request.system }));
        }
        for message in &request.messages {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            messages.push(json!({ "role": role, "content": message.content }));
        }
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": true,
            // Ask for a final usage-only chunk; some gateways ignore it.
            "stream_options": { "include_usage": true },
        });
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if !request.stop.is_empty() {
            body["stop"] = json!(request.stop);
        }
        body
    }
}

/// Flatten a `content`-style field: either a plain string or an array of
/// `{ "text": ... }` parts.
fn text_field(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

#[derive(Debug, Default)]
struct StreamState {
    finish_reason: Option<String>,
    done: bool,
}

async fn handle_event(
    event: SseEvent,
    tx: &mpsc::Sender<ChatDelta>,
    state: &mut StreamState,
) -> Result<()> {
    if event.is_done() {
        // `[DONE]` is the normal terminator; only emit if `finish_reason`
        // never arrived.
        if !state.done {
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: state.finish_reason.take(),
                },
            )
            .await?;
        }
        return Ok(());
    }
    let data = event.data.trim();
    if data.is_empty() {
        return Ok(());
    }
    let value: Value = match parse_json(data) {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(%error, "ignoring undecodable chat completions frame");
            return Ok(());
        }
    };

    // Some gateways report errors as an in-stream JSON object instead of a
    // non-2xx status.
    if value.get("error").is_some_and(|error| !error.is_null()) {
        let kind = json_str(&value, &["error", "type"]).unwrap_or("error");
        let message = json_str(&value, &["error", "message"]).unwrap_or("unknown error");
        return Err(Error::provider(format!(
            "chat completions stream error ({kind}): {message}"
        )));
    }

    // Usage may arrive on its own with an empty `choices` array.
    if value.get("usage").is_some_and(|usage| !usage.is_null()) {
        let input = json_u32(&value, &["usage", "prompt_tokens"])
            .or_else(|| json_u32(&value, &["usage", "input_tokens"]))
            .unwrap_or(0);
        let output = json_u32(&value, &["usage", "completion_tokens"])
            .or_else(|| json_u32(&value, &["usage", "output_tokens"]))
            .unwrap_or(0);
        send_delta(
            tx,
            ChatDelta::Usage {
                input_tokens: input,
                output_tokens: output,
            },
        )
        .await?;
    }

    let Some(choice) = value.get("choices").and_then(|choices| choices.get(0)) else {
        return Ok(());
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(text) = delta.get("content").and_then(text_field) {
            send_delta(tx, ChatDelta::Text { text }).await?;
        }
        let reasoning = delta
            .get("reasoning_content")
            .and_then(text_field)
            .or_else(|| delta.get("reasoning").and_then(text_field));
        if let Some(text) = reasoning {
            send_delta(tx, ChatDelta::Reasoning { text }).await?;
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                if let Some(name) = json_str(call, &["function", "name"]) {
                    send_delta(
                        tx,
                        ChatDelta::Status {
                            message: format!("调用工具：{name}"),
                        },
                    )
                    .await?;
                }
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        state.finish_reason = Some(reason.to_string());
        if !state.done {
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: Some(reason.to_string()),
                },
            )
            .await?;
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl ChatProvider for OpenAiChatProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiChat
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
            .bearer_auth(&self.api_key)
            .json(&body);
        let request_builder = apply_extra_headers(request_builder, &self.extra_headers)?;
        let response = request_builder
            .send()
            .await
            .map_err(|e| Error::provider(format!("chat completions request failed: {e}")))?;
        let response = check_status(response, "chat completions", &self.api_key).await?;

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
            let chunk = chunk
                .map_err(|e| Error::provider(format!("chat completions stream failed: {e}")))?;
            let text = String::from_utf8_lossy(&chunk);
            for event in decoder.push(&text) {
                handle_event(event, &tx, &mut state).await?;
            }
        }
        for event in decoder.finish() {
            handle_event(event, &tx, &mut state).await?;
        }
        if !state.done {
            // Connection closed without `finish_reason` or `[DONE]`.
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
    use crate::secrets::MemorySecretStore;

    fn provider() -> OpenAiChatProvider {
        let cfg = ProviderConfig::new("gateway", ProviderKind::OpenAiChat, "gpt-test");
        let secrets = MemorySecretStore::new();
        secrets.set("provider/gateway", "sk-test-secret").unwrap();
        OpenAiChatProvider::new(&cfg, &secrets).unwrap()
    }

    #[test]
    fn body_puts_system_first_and_requests_usage() {
        let mut request = ChatRequest::new("deepseek-chat", "be terse");
        request.messages.push(ChatMessage::user("hi"));
        request.messages.push(ChatMessage::assistant("hello"));
        request.max_tokens = Some(64);
        request.temperature = Some(0.5);
        let body = provider().body(&request);
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be terse");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn maps_reasoning_usage_and_done_once() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::default();
        let frames = [
            r#"{"choices":[{"delta":{"reasoning_content":"think"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"Hi"},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":2}}"#,
            "[DONE]",
        ];
        for data in frames {
            handle_event(
                SseEvent {
                    data: data.into(),
                    ..Default::default()
                },
                &tx,
                &mut state,
            )
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
                ChatDelta::Reasoning {
                    text: "think".into()
                },
                ChatDelta::Text { text: "Hi".into() },
                ChatDelta::Done {
                    finish_reason: Some("stop".into())
                },
                ChatDelta::Usage {
                    input_tokens: 9,
                    output_tokens: 2
                },
            ]
        );
    }

    #[tokio::test]
    async fn done_sentinel_emits_done_when_no_finish_reason() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut state = StreamState::default();
        handle_event(
            SseEvent {
                data: "[DONE]".into(),
                ..Default::default()
            },
            &tx,
            &mut state,
        )
        .await
        .unwrap();
        drop(tx);
        assert_eq!(
            rx.recv().await,
            Some(ChatDelta::Done {
                finish_reason: None
            })
        );
    }
}
