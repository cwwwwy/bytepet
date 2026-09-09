//! OpenAI Responses API provider (`POST /responses`, SSE).
//!
//! This is the transport used by Codex CLI custom providers configured with
//! `wire_api = "responses"`, which is the common way to point Codex at
//! DeepSeek or another OpenAI-compatible gateway.
//!
//! The stream is a sequence of named events (`event:` line plus a `type` field
//! in the payload). Text normally arrives through `response.output_text.delta`,
//! but we also reconcile the final `response.output_item.done` /
//! `response.output_text.done` payloads so a gateway that batches instead of
//! streaming still produces the full answer exactly once.

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::llm::providers::{
    apply_extra_headers, check_status, json_str, json_u32, send_delta, TextAccumulator,
};
use crate::llm::sse::{parse_json, SseDecoder, SseEvent};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, ProviderConfig, ProviderKind, Role};
use crate::secrets::SecretStore;

pub struct OpenAiResponsesProvider {
    id: String,
    model: String,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
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
        format!("{}/responses", self.base_url)
    }

    fn body(&self, request: &ChatRequest) -> Value {
        let model = if request.model.trim().is_empty() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };
        let input: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                // Assistant turns replay as `output_text`, everything else as
                // `input_text`.
                let (role, content_type) = match message.role {
                    Role::Assistant => ("assistant", "output_text"),
                    Role::System => ("system", "input_text"),
                    Role::User => ("user", "input_text"),
                };
                json!({
                    "role": role,
                    "content": [{ "type": content_type, "text": message.content }],
                })
            })
            .collect();
        let mut body = json!({
            "model": model,
            "input": input,
            "stream": true,
        });
        if !request.system.trim().is_empty() {
            body["instructions"] = json!(request.system);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_output_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        body
    }
}

/// Concatenate the `text` of every element of `container[key]`.
fn collect_text(container: &Value, key: &str) -> String {
    let mut out = String::new();
    if let Some(items) = container.get(key).and_then(Value::as_array) {
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
    }
    out
}

#[derive(Debug, Default)]
struct StreamState {
    text: TextAccumulator,
    reasoning: TextAccumulator,
    done: bool,
}

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
    let value: Value = match parse_json(data) {
        Ok(value) => value,
        Err(error) => {
            tracing::debug!(%error, "ignoring undecodable responses stream frame");
            return Ok(());
        }
    };
    let kind = event
        .event
        .as_deref()
        .or_else(|| json_str(&value, &["type"]))
        .unwrap_or_default();

    match kind {
        "response.output_text.delta" => {
            let text = json_str(&value, &["delta"]).and_then(|delta| state.text.push_delta(delta));
            if let Some(text) = text {
                send_delta(tx, ChatDelta::Text { text }).await?;
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            let text =
                json_str(&value, &["delta"]).and_then(|delta| state.reasoning.push_delta(delta));
            if let Some(text) = text {
                send_delta(tx, ChatDelta::Reasoning { text }).await?;
            }
        }
        // Fallbacks: emit only the part that deltas did not already cover.
        "response.output_text.done" => {
            let missing = json_str(&value, &["text"]).and_then(|text| state.text.push_final(text));
            if let Some(text) = missing {
                send_delta(tx, ChatDelta::Text { text }).await?;
            }
        }
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
            let missing =
                json_str(&value, &["text"]).and_then(|text| state.reasoning.push_final(text));
            if let Some(text) = missing {
                send_delta(tx, ChatDelta::Reasoning { text }).await?;
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                match json_str(item, &["type"]) {
                    Some("message") => {
                        let full = collect_text(item, "content");
                        if let Some(missing) = state.text.push_final(&full) {
                            send_delta(tx, ChatDelta::Text { text: missing }).await?;
                        }
                    }
                    Some("reasoning") => {
                        // Reasoning items expose summaries; some gateways use
                        // `content` instead.
                        let mut full = collect_text(item, "summary");
                        if full.is_empty() {
                            full = collect_text(item, "content");
                        }
                        if let Some(missing) = state.reasoning.push_final(&full) {
                            send_delta(tx, ChatDelta::Reasoning { text: missing }).await?;
                        }
                    }
                    _ => {}
                }
            }
        }
        "response.completed" => {
            if let Some(usage) = value.get("response").and_then(|r| r.get("usage")) {
                let input = json_u32(usage, &["input_tokens"]).unwrap_or(0);
                let output = json_u32(usage, &["output_tokens"]).unwrap_or(0);
                send_delta(
                    tx,
                    ChatDelta::Usage {
                        input_tokens: input,
                        output_tokens: output,
                    },
                )
                .await?;
            }
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: None,
                },
            )
            .await?;
        }
        "response.incomplete" => {
            let reason = json_str(&value, &["response", "incomplete_details", "reason"])
                .unwrap_or("incomplete");
            if state.text.is_empty() && state.reasoning.is_empty() {
                return Err(Error::provider(format!(
                    "responses stream incomplete: {reason}"
                )));
            }
            state.done = true;
            send_delta(
                tx,
                ChatDelta::Done {
                    finish_reason: Some(reason.to_string()),
                },
            )
            .await?;
        }
        "response.failed" => {
            let message =
                json_str(&value, &["response", "error", "message"]).unwrap_or("unknown error");
            return Err(Error::provider(format!(
                "responses stream failed: {message}"
            )));
        }
        "error" => {
            let message = json_str(&value, &["message"])
                .or_else(|| json_str(&value, &["error", "message"]))
                .unwrap_or("unknown error");
            return Err(Error::provider(format!(
                "responses stream error: {message}"
            )));
        }
        _ => {}
    }
    Ok(())
}

#[async_trait::async_trait]
impl ChatProvider for OpenAiResponsesProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiResponses
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
            .map_err(|e| Error::provider(format!("responses request failed: {e}")))?;
        let response = check_status(response, "responses", &self.api_key).await?;

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
            let chunk =
                chunk.map_err(|e| Error::provider(format!("responses stream failed: {e}")))?;
            let text = String::from_utf8_lossy(&chunk);
            for event in decoder.push(&text) {
                handle_event(event, &tx, &mut state).await?;
            }
        }
        for event in decoder.finish() {
            handle_event(event, &tx, &mut state).await?;
        }
        if !state.done {
            send_delta(
                &tx,
                ChatDelta::Done {
                    finish_reason: None,
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

    fn provider() -> OpenAiResponsesProvider {
        let cfg = ProviderConfig::new("gateway", ProviderKind::OpenAiResponses, "gpt-test");
        let secrets = MemorySecretStore::new();
        secrets.set("provider/gateway", "sk-test-secret").unwrap();
        OpenAiResponsesProvider::new(&cfg, &secrets).unwrap()
    }

    #[test]
    fn body_uses_instructions_and_typed_content() {
        let mut request = ChatRequest::new("gpt-5", "be terse");
        request.messages.push(ChatMessage::user("hi"));
        request.messages.push(ChatMessage::assistant("hello"));
        request.max_tokens = Some(128);
        let body = provider().body(&request);
        assert_eq!(body["model"], "gpt-5");
        assert_eq!(body["instructions"], "be terse");
        assert_eq!(body["max_output_tokens"], 128);
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    }

    #[tokio::test]
    async fn output_item_done_fills_gaps_without_duplicating() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = StreamState::default();
        let frames = [
            r#"{"type":"response.output_text.delta","delta":"Hel"}"#,
            // A gateway that skips deltas must still deliver the whole text.
            r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"Hello"}]}}"#,
            // A repeated final payload must not duplicate anything.
            r#"{"type":"response.output_text.done","text":"Hello"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
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
                ChatDelta::Text { text: "Hel".into() },
                ChatDelta::Text { text: "lo".into() },
                ChatDelta::Usage {
                    input_tokens: 5,
                    output_tokens: 2
                },
                ChatDelta::Done {
                    finish_reason: None
                },
            ]
        );
    }

    #[tokio::test]
    async fn failed_event_becomes_err() {
        let (tx, _rx) = mpsc::channel(4);
        let mut state = StreamState::default();
        let error = handle_event(
            SseEvent {
                data: r#"{"type":"response.failed","response":{"error":{"message":"boom"}}}"#
                    .into(),
                ..Default::default()
            },
            &tx,
            &mut state,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("boom"));
    }
}
