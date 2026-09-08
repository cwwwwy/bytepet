//! Agent status integration: a local HTTP/WebSocket protocol plus hook
//! installers for Codex and Claude Code.
//!
//! The event shape is intentionally compatible with UniPet
//! (`source` / `state` / `message` / `action` / `ttl`), so existing community
//! hooks can drive this pet without changes.

pub mod hooks;
pub mod server;

pub use hooks::{AgentKind, HookInstaller, HookReport, HookStatus};
pub use server::{spawn as spawn_server, ServerHandle};

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::state::PetState;

/// Incoming status event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    /// Who sent it, e.g. `codex`, `claude-code`, `my-script`.
    pub source: String,
    /// State name (aliases accepted, see [`PetState::from_name`]).
    pub state: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    /// Time to live in milliseconds; `None` uses the configured default.
    #[serde(default)]
    pub ttl_ms: Option<u64>,
}

impl AgentEvent {
    pub fn new(source: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            state: state.into(),
            ..Self::default()
        }
    }

    pub fn pet_state(&self) -> Option<PetState> {
        PetState::from_name(&self.state)
    }

    pub fn ttl(&self, default_secs: u64) -> Option<Duration> {
        match self.ttl_ms {
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(ms)),
            None if default_secs == 0 => None,
            None => Some(Duration::from_secs(default_secs)),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.source.trim().is_empty() {
            return Err(Error::Agent("source must not be empty".into()));
        }
        if self.source.len() > 64 {
            return Err(Error::Agent("source is too long (max 64)".into()));
        }
        if self.pet_state().is_none() {
            return Err(Error::Agent(format!(
                "unknown state '{}' (expected one of: idle, running, waiting, failed, review, waving, jumping, running-left, running-right)",
                self.state
            )));
        }
        if let Some(msg) = &self.message {
            if msg.chars().count() > 500 {
                return Err(Error::Agent("message is too long (max 500 chars)".into()));
            }
        }
        Ok(())
    }

    /// Map an agent lifecycle event name to a pet state.
    pub fn state_for_lifecycle(event: &str) -> Option<PetState> {
        let e = event.trim().to_ascii_lowercase().replace(['_', '.'], "-");
        match e.as_str() {
            "session-start" | "sessionstart" | "start" => Some(PetState::Waving),
            "user-prompt-submit" | "userpromptsubmit" | "prompt" | "submit" => {
                Some(PetState::Running)
            }
            "pre-tool-use" | "pretooluse" | "tool-start" => Some(PetState::Running),
            "post-tool-use" | "posttooluse" | "tool-end" => Some(PetState::Running),
            "notification" | "notify" | "waiting" | "approval" => Some(PetState::Waiting),
            "stop" | "session-end" | "sessionend" | "done" | "turn-ended" | "turn-completed" => {
                Some(PetState::Review)
            }
            "error" | "failed" | "fail" => Some(PetState::Failed),
            _ => None,
        }
    }
}

/// Snapshot returned by `GET /health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHealth {
    pub ok: bool,
    pub version: String,
    pub pet: Option<String>,
    pub persona: Option<String>,
    pub state: Option<String>,
    pub sources: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_states_and_aliases() {
        let ev = AgentEvent::new("codex", "running");
        assert_eq!(ev.pet_state(), Some(PetState::Running));
        let ev = AgentEvent::new("codex", "waiting");
        assert_eq!(ev.pet_state(), Some(PetState::Waiting));
        assert!(AgentEvent::new("codex", "bogus").validate().is_err());
        assert!(AgentEvent::new("", "idle").validate().is_err());
    }

    #[test]
    fn ttl_semantics() {
        let mut ev = AgentEvent::new("x", "running");
        assert_eq!(ev.ttl(120), Some(Duration::from_secs(120)));
        ev.ttl_ms = Some(0);
        assert_eq!(ev.ttl(120), None);
        ev.ttl_ms = Some(1500);
        assert_eq!(ev.ttl(120), Some(Duration::from_millis(1500)));
    }

    #[test]
    fn lifecycle_mapping() {
        assert_eq!(
            AgentEvent::state_for_lifecycle("SessionStart"),
            Some(PetState::Waving)
        );
        assert_eq!(
            AgentEvent::state_for_lifecycle("PreToolUse"),
            Some(PetState::Running)
        );
        assert_eq!(
            AgentEvent::state_for_lifecycle("Stop"),
            Some(PetState::Review)
        );
        assert_eq!(AgentEvent::state_for_lifecycle("nope"), None);
    }
}
