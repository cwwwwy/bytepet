//! Typed event names and payloads shared between Rust and the frontend.

use serde::{Deserialize, Serialize};

/// Pet state changed (row switch + optional bubble text).
pub const PET_STATE: &str = "pet://state";
/// The active pet skin changed.
pub const PET_LIBRARY_CHANGED: &str = "pet://library-changed";
/// Settings were saved.
pub const SETTINGS_CHANGED: &str = "settings://changed";
/// Active persona changed.
pub const PERSONA_CHANGED: &str = "persona://changed";
/// Streaming chat delta.
pub const CHAT_DELTA: &str = "chat://delta";
/// Chat turn finished.
pub const CHAT_DONE: &str = "chat://done";
/// Chat turn failed.
pub const CHAT_ERROR: &str = "chat://error";
/// TTS playback state changed.
pub const TTS_STATE: &str = "tts://state";
/// Agent status events (for the debug panel).
pub const AGENT_EVENT: &str = "agent://event";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PetStateEvent {
    pub state: String,
    pub source: String,
    pub message: Option<String>,
    pub one_shot: bool,
}

impl From<&bytepet_core::agent::AgentEvent> for PetStateEvent {
    fn from(event: &bytepet_core::agent::AgentEvent) -> Self {
        Self {
            state: event.state.clone(),
            source: event.source.clone(),
            message: event.message.clone(),
            one_shot: event
                .pet_state()
                .map(|s| s.is_one_shot())
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatDeltaEvent {
    pub conversation_id: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatDoneEvent {
    pub conversation_id: String,
    pub message_id: i64,
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatErrorEvent {
    pub conversation_id: String,
    pub message: String,
}
