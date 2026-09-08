//! Conversation runtime: owns in-flight turns, forwards deltas to the UI,
//! persists history and triggers background summarisation.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use bytepet_core::chat::{run_turn, summarize_conversation, TurnContext};
use bytepet_core::llm::{build_provider, ChatDelta, ChatProvider, ProviderConfig};
use bytepet_core::memory::{MemoryConfig, MemoryStore};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::events::{ChatDeltaEvent, ChatDoneEvent, ChatErrorEvent, CHAT_DELTA, CHAT_DONE, CHAT_ERROR};

/// Tracks the cancellable turns currently streaming.
#[derive(Default)]
pub struct ChatManager {
    active: Mutex<HashMap<String, CancellationToken>>,
}

impl ChatManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_streaming(&self, conversation_id: &str) -> bool {
        self.active.lock().contains_key(conversation_id)
    }

    pub fn cancel(&self, conversation_id: &str) {
        if let Some(token) = self.active.lock().remove(conversation_id) {
            token.cancel();
        }
    }

    pub fn cancel_all(&self) {
        for (_, token) in self.active.lock().drain() {
            token.cancel();
        }
    }

    /// Start a turn. Returns immediately; the reply streams via events.
    pub fn send<R: Runtime>(
        &self,
        app: AppHandle<R>,
        memory: Arc<MemoryStore>,
        conversation_id: String,
        text: String,
    ) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("消息不能为空".into());
        }
        if self.is_streaming(&conversation_id) {
            return Err("上一条回复还在生成中".into());
        }

        let (persona, provider_cfg, model, pet_name, memory_cfg) = {
            let state = app
                .try_state::<crate::state::AppState>()
                .ok_or("应用状态未初始化")?;
            let config = state.config();
            let conversation = memory
                .conversations(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|c| c.id == conversation_id)
                .ok_or("对话不存在")?;
            let persona = state
                .personas
                .get(&conversation.persona_id)
                .map_err(|e| e.to_string())?
                .or_else(|| state.personas.list().ok().and_then(|p| p.into_iter().next()))
                .ok_or("没有可用人格")?;
            let provider_id = persona
                .model
                .as_ref()
                .map(|m| m.provider.clone())
                .or_else(|| config.default_provider.clone())
                .ok_or("还没有配置模型服务，请先在设置里添加")?;
            let provider_cfg = config
                .providers
                .get(&provider_id)
                .cloned()
                .ok_or_else(|| format!("模型服务 '{provider_id}' 不存在"))?;
            let model = persona
                .model
                .as_ref()
                .and_then(|m| m.model.clone())
                .unwrap_or_else(|| provider_cfg.model.clone());
            let pet_name = config
                .active_pet
                .as_ref()
                .and_then(|id| state.library.get(id))
                .map(|p| p.display_name);
            let memory_cfg = MemoryConfig {
                enabled: persona.memory.enabled,
                window_turns: persona.memory.window_turns,
                long_term: persona.memory.long_term,
                summarize_after_turns: persona.memory.summarize_after_turns,
                ..MemoryConfig::default()
            };
            (persona, provider_cfg, model, pet_name, memory_cfg)
        };

        let provider = build_chat_provider(&app, &provider_cfg)?;
        let cancel = CancellationToken::new();
        self.active
            .lock()
            .insert(conversation_id.clone(), cancel.clone());

        // The pet reacts while the model works (10 minute safety TTL).
        crate::window::pet_window::raise_state(
            &app,
            bytepet_core::pet::state::PetState::Running,
            "chat",
            None,
            Some(std::time::Duration::from_secs(600)),
        );

        let app_for_task = app.clone();
        let memory_for_task = memory.clone();
        let conv = conversation_id.clone();
        let manager_ptr = app.clone();

        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::channel::<ChatDelta>(64);
            let forward = {
                let app = app_for_task.clone();
                let conv = conv.clone();
                tokio::spawn(async move {
                    while let Some(delta) = rx.recv().await {
                        let (kind, text) = match delta {
                            ChatDelta::Text { text } => ("text", text),
                            ChatDelta::Reasoning { text } => ("reasoning", text),
                            ChatDelta::Status { message } => ("status", message),
                            ChatDelta::Usage { .. } | ChatDelta::Done { .. } => continue,
                        };
                        let _ = app.emit(
                            CHAT_DELTA,
                            ChatDeltaEvent {
                                conversation_id: conv.clone(),
                                kind: kind.to_string(),
                                text,
                            },
                        );
                    }
                })
            };

            let outcome = run_turn(
                TurnContext {
                    persona: &persona,
                    conversation_id: &conv,
                    pet_name: pet_name.as_deref(),
                    pet_state: Some("工作中"),
                    model: model.clone(),
                    memory_config: memory_cfg.clone(),
                },
                &text,
                &memory_for_task,
                provider.clone(),
                cancel.clone(),
                tx,
            )
            .await;

            let _ = forward.await;

            // Clear the in-flight marker.
            if let Some(state) = manager_ptr.try_state::<crate::state::AppState>() {
                state.chat.cancel(&conv);
            }

            match outcome {
                Ok(outcome) => {
                    let _ = app_for_task.emit(
                        CHAT_DONE,
                        ChatDoneEvent {
                            conversation_id: conv.clone(),
                            message_id: outcome.message_id,
                            content: outcome.content.clone(),
                            input_tokens: outcome.input_tokens,
                            output_tokens: outcome.output_tokens,
                            finish_reason: outcome.finish_reason,
                        },
                    );
                    crate::window::pet_window::raise_state(
                        &app_for_task,
                        bytepet_core::pet::state::PetState::Review,
                        "chat",
                        None,
                        Some(std::time::Duration::from_secs(3)),
                    );
                    if let Some(state) = app_for_task.try_state::<crate::state::AppState>() {
                        state.speak_reply(&app_for_task, &outcome.content);
                    }
                    if outcome.should_summarize {
                        let app = app_for_task.clone();
                        let memory = memory_for_task.clone();
                        let persona = persona.clone();
                        let model = model.clone();
                        tokio::spawn(async move {
                            if let Err(err) = summarize_conversation(
                                &persona,
                                &conv,
                                &model,
                                &memory,
                                provider,
                                CancellationToken::new(),
                            )
                            .await
                            {
                                tracing::warn!(%err, "memory summarisation failed");
                            }
                            let _ = app.emit("memory://updated", conv);
                        });
                    }
                }
                Err(err) => {
                    let cancelled = matches!(err, bytepet_core::Error::Cancelled) || cancel.is_cancelled();
                    let message = if cancelled {
                        "已取消".to_string()
                    } else {
                        bytepet_core::secrets::redact(&err.to_string())
                    };
                    let _ = app_for_task.emit(
                        CHAT_ERROR,
                        ChatErrorEvent {
                            conversation_id: conv.clone(),
                            message: message.clone(),
                        },
                    );
                    if cancelled {
                        crate::window::pet_window::clear_source(&app_for_task, "chat");
                    } else {
                        crate::window::pet_window::raise_state(
                            &app_for_task,
                            bytepet_core::pet::state::PetState::Failed,
                            "chat",
                            Some(message.clone()),
                            Some(std::time::Duration::from_secs(5)),
                        );
                    }
                }
            }
        });

        Ok(())
    }
}

fn build_chat_provider<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &ProviderConfig,
) -> Result<Arc<dyn ChatProvider>, String> {
    let state = app
        .try_state::<crate::state::AppState>()
        .ok_or("应用状态未初始化")?;
    build_provider(cfg, state.secrets.as_ref())
        .map_err(|e| bytepet_core::secrets::redact(&e.to_string()))
}

