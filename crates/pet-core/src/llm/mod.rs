//! LLM provider abstraction: one streaming interface over five transports.
//!
//! | kind | transport |
//! |---|---|
//! | `anthropic` | Anthropic Messages API (`POST /v1/messages`, SSE) |
//! | `openai-chat` | OpenAI-compatible `POST /chat/completions` (SSE) |
//! | `openai-responses` | OpenAI Responses API `POST /responses` (SSE) |
//! | `codex-cli` | local `codex exec --json` (JSONL) |
//! | `claude-cli` | local `claude -p --output-format stream-json` (JSONL) |

pub mod providers;
pub mod sse;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::secrets::SecretStore;

pub use sse::{SseDecoder, SseEvent};

/// Transport family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Anthropic,
    #[default]
    OpenAiChat,
    OpenAiResponses,
    CodexCli,
    ClaudeCli,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 5] = [
        ProviderKind::Anthropic,
        ProviderKind::OpenAiChat,
        ProviderKind::OpenAiResponses,
        ProviderKind::CodexCli,
        ProviderKind::ClaudeCli,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic (Claude API)",
            ProviderKind::OpenAiChat => "OpenAI 兼容 (/chat/completions)",
            ProviderKind::OpenAiResponses => "OpenAI Responses (/responses)",
            ProviderKind::CodexCli => "Codex CLI (codex exec --json)",
            ProviderKind::ClaudeCli => "Claude Code CLI (claude -p)",
        }
    }

    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderKind::Anthropic => Some("https://api.anthropic.com"),
            ProviderKind::OpenAiChat | ProviderKind::OpenAiResponses => {
                Some("https://api.openai.com/v1")
            }
            ProviderKind::CodexCli | ProviderKind::ClaudeCli => None,
        }
    }

    pub fn needs_api_key(self) -> bool {
        matches!(
            self,
            ProviderKind::Anthropic | ProviderKind::OpenAiChat | ProviderKind::OpenAiResponses
        )
    }

    pub fn is_cli(self) -> bool {
        matches!(self, ProviderKind::CodexCli | ProviderKind::ClaudeCli)
    }
}

/// Non-secret provider configuration. Secrets live in the OS keychain and are
/// referenced by [`ProviderConfig::api_key_ref`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub id: String,
    pub label: String,
    pub kind: ProviderKind,
    pub base_url: Option<String>,
    pub model: String,
    /// `provider/<id>` by convention.
    pub api_key_ref: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub enabled: bool,
    /// Provider-specific knobs (CLI path, permission mode, timeouts, ...).
    pub options: BTreeMap<String, serde_json::Value>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            kind: ProviderKind::default(),
            base_url: None,
            model: String::new(),
            api_key_ref: None,
            extra_headers: BTreeMap::new(),
            enabled: true,
            options: BTreeMap::new(),
        }
    }
}

impl ProviderConfig {
    pub fn new(id: impl Into<String>, kind: ProviderKind, model: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            label: kind.label().to_string(),
            base_url: kind.default_base_url().map(|s| s.to_string()),
            api_key_ref: kind
                .needs_api_key()
                .then(|| format!("provider/{id}")),
            id,
            kind,
            model: model.into(),
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(Error::config("provider id must not be empty"));
        }
        if self.kind.needs_api_key() && self.base_url.as_deref().unwrap_or("").trim().is_empty() {
            return Err(Error::config("provider base_url must not be empty"));
        }
        // CLI providers may leave the model empty to use the CLI's own default.
        if self.model.trim().is_empty() && !self.kind.is_cli() {
            return Err(Error::config("provider model must not be empty"));
        }
        Ok(())
    }

    pub fn base_url(&self) -> String {
        self.base_url
            .clone()
            .or_else(|| self.kind.default_base_url().map(|s| s.to_string()))
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A single model call.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop: Vec<String>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, system: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: system.into(),
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            stop: Vec::new(),
        }
    }
}

/// Incremental output of a streaming call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ChatDelta {
    /// Visible assistant text.
    Text { text: String },
    /// Chain-of-thought / reasoning text (only shown when enabled).
    Reasoning { text: String },
    /// Progress narration, e.g. "调用工具：bash".
    Status { message: String },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    Done {
        finish_reason: Option<String>,
    },
}

/// Streaming chat provider.
#[async_trait::async_trait]
pub trait ChatProvider: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> ProviderKind;

    /// Stream a completion. Must respect `cancel` promptly and must never log
    /// credentials.
    async fn stream(
        &self,
        request: ChatRequest,
        tx: mpsc::Sender<ChatDelta>,
        cancel: CancellationToken,
    ) -> Result<()>;

    /// Cheap credential / availability check used by the "test" button.
    async fn probe(&self) -> Result<()> {
        Ok(())
    }
}

/// Resolve the API key for a provider, or a clear error if it is missing.
pub fn resolve_api_key(cfg: &ProviderConfig, secrets: &dyn SecretStore) -> Result<String> {
    let key_ref = cfg
        .api_key_ref
        .clone()
        .unwrap_or_else(|| format!("provider/{}", cfg.id));
    match secrets.get(&key_ref)? {
        Some(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(Error::ProviderNotConfigured(format!(
            "provider '{}' has no API key stored",
            cfg.id
        ))),
    }
}

/// Build a provider from configuration.
pub fn build_provider(
    cfg: &ProviderConfig,
    secrets: &dyn SecretStore,
) -> Result<Arc<dyn ChatProvider>> {
    cfg.validate()?;
    providers::build(cfg, secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_config_defaults() {
        let cfg = ProviderConfig::new("deepseek", ProviderKind::OpenAiChat, "deepseek-chat");
        assert_eq!(cfg.base_url(), "https://api.openai.com/v1");
        assert_eq!(cfg.api_key_ref.as_deref(), Some("provider/deepseek"));
        cfg.validate().unwrap();
    }

    #[test]
    fn cli_providers_need_no_key() {
        let cfg = ProviderConfig::new("codex", ProviderKind::CodexCli, "gpt-5-codex");
        assert!(cfg.api_key_ref.is_none());
        assert_eq!(cfg.base_url(), "");
        cfg.validate().unwrap();
    }

    #[test]
    fn missing_key_is_reported_clearly() {
        let cfg = ProviderConfig::new("x", ProviderKind::Anthropic, "claude-sonnet-4-5");
        let secrets = crate::secrets::MemorySecretStore::new();
        let err = resolve_api_key(&cfg, &secrets).unwrap_err();
        assert!(matches!(err, Error::ProviderNotConfigured(_)));
    }

    #[test]
    fn api_key_round_trip() {
        let cfg = ProviderConfig::new("x", ProviderKind::Anthropic, "claude-sonnet-4-5");
        let secrets = crate::secrets::MemorySecretStore::new();
        secrets.set("provider/x", "sk-test").unwrap();
        assert_eq!(resolve_api_key(&cfg, &secrets).unwrap(), "sk-test");
    }
}
