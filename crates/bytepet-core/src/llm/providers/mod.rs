//! Provider implementations. Each provider is constructed from a
//! [`ProviderConfig`] plus a [`SecretStore`] and implements [`ChatProvider`].

pub mod anthropic;
pub mod claude_cli;
pub mod codex_cli;
pub mod openai_chat;
pub mod openai_responses;

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, ProviderConfig, ProviderKind, Role};
use crate::secrets::SecretStore;

/// Build the concrete provider for a configuration.
pub fn build(cfg: &ProviderConfig, secrets: &dyn SecretStore) -> Result<Arc<dyn ChatProvider>> {
    match cfg.kind {
        ProviderKind::Anthropic => Ok(Arc::new(anthropic::AnthropicProvider::new(cfg, secrets)?)),
        ProviderKind::OpenAiChat => Ok(Arc::new(openai_chat::OpenAiChatProvider::new(
            cfg, secrets,
        )?)),
        ProviderKind::OpenAiResponses => Ok(Arc::new(
            openai_responses::OpenAiResponsesProvider::new(cfg, secrets)?,
        )),
        ProviderKind::CodexCli => Ok(Arc::new(codex_cli::CodexCliProvider::new(cfg)?)),
        ProviderKind::ClaudeCli => Ok(Arc::new(claude_cli::ClaudeCliProvider::new(cfg)?)),
    }
}

/// Shared HTTP client builder: rustls, sane timeouts, no proxy surprises.
pub(crate) fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("pet/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(300))
        .build()
}

// ---------------------------------------------------------------------------
// Helpers shared by the five providers.
// ---------------------------------------------------------------------------

/// How much of a remote response body an error message may quote.
const ERROR_BODY_LIMIT: usize = 512;

/// Truncate on character boundaries (never panics on multi-byte payloads).
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Replace a credential so it can never leak into an error message or a log.
pub(crate) fn redact_secret(text: &str, secret: &str) -> String {
    if secret.trim().is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

/// Convert a non-2xx response into a clear error carrying the HTTP status and a
/// truncated, redacted body. Request headers (and therefore the API key) are
/// never included.
pub(crate) async fn check_status(
    response: reqwest::Response,
    label: &str,
    api_key: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    // Belt and braces: the exact key, then anything that looks like a credential.
    let body = redact_secret(body.trim(), api_key);
    let body = crate::secrets::redact(&body);
    Err(Error::provider(format!(
        "{label} request failed: HTTP {status}: {}",
        truncate_chars(&body, ERROR_BODY_LIMIT)
    )))
}

/// Apply user-configured extra headers, validating names and values.
pub(crate) fn apply_extra_headers(
    mut request: reqwest::RequestBuilder,
    headers: &[(String, String)],
) -> Result<reqwest::RequestBuilder> {
    for (name, value) in headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| Error::config(format!("invalid provider header name: {e}")))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|e| Error::config(format!("invalid provider header value: {e}")))?;
        request = request.header(name, value);
    }
    Ok(request)
}

/// Forward one delta. A closed receiver means the caller stopped listening.
pub(crate) async fn send_delta(tx: &mpsc::Sender<ChatDelta>, delta: ChatDelta) -> Result<()> {
    tx.send(delta).await.map_err(|_| Error::Cancelled)
}

/// Read a nested unsigned integer from a JSON value, tolerating missing or
/// unexpectedly typed fields (gateways differ in their usage shapes).
pub(crate) fn json_u32(value: &serde_json::Value, path: &[&str]) -> Option<u32> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_u64()
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Read a nested string from a JSON value.
pub(crate) fn json_str<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str()
}

/// Tracks text that was already streamed so that "full message" fallback
/// events (CLI JSONL `assistant`/`result` lines, Responses `output_item.done`)
/// never duplicate content the consumer has already seen.
#[derive(Debug, Default)]
pub(crate) struct TextAccumulator {
    streamed: String,
}

impl TextAccumulator {
    /// Record an incremental delta and return the text to emit.
    pub(crate) fn push_delta(&mut self, text: &str) -> Option<String> {
        if text.is_empty() {
            return None;
        }
        self.streamed.push_str(text);
        Some(text.to_string())
    }

    /// Reconcile a full-text event with what was already streamed. Returns the
    /// missing suffix, the whole text when nothing was streamed yet, or `None`
    /// when emitting would duplicate (or contradict) the deltas.
    pub(crate) fn push_final(&mut self, full: &str) -> Option<String> {
        if full.is_empty() || full == self.streamed {
            return None;
        }
        if let Some(rest) = full.strip_prefix(&self.streamed) {
            self.streamed = full.to_string();
            return if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
        }
        if self.streamed.is_empty() {
            self.streamed = full.to_string();
            return Some(full.to_string());
        }
        // Diverging content: trust the deltas already delivered.
        None
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.streamed.is_empty()
    }
}

/// Render a request as the single prompt string the CLI providers take on
/// stdin (`codex exec` and `claude -p` both accept one prompt, not a message
/// array).
pub(crate) fn format_cli_prompt(request: &ChatRequest) -> String {
    let mut out = String::new();
    let system = request.system.trim();
    if !system.is_empty() {
        out.push_str(system);
        out.push_str("\n\n");
    }
    for message in &request.messages {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(message.content.trim_end());
        out.push('\n');
    }
    if out.trim().is_empty() {
        // Never hand a CLI an empty stdin: it may block waiting for input.
        out.push_str("(empty prompt)");
    }
    out
}
