//! Conversation orchestration: assemble the prompt, stream the model reply,
//! persist both sides and keep long-term memory up to date.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::llm::{ChatDelta, ChatProvider, ChatRequest, Role};
use crate::memory::{MemoryConfig, MemoryStore, NewMessage};
use crate::persona::Persona;

/// Everything a turn needs beyond the provider.
pub struct TurnContext<'a> {
    pub persona: &'a Persona,
    pub conversation_id: &'a str,
    /// Display name of the pet skin currently on screen.
    pub pet_name: Option<&'a str>,
    /// Human readable bytepet state (e.g. "工作中").
    pub pet_state: Option<&'a str>,
    /// Model id to send to the provider (overrides the provider default).
    pub model: String,
    pub memory_config: MemoryConfig,
}

/// Result of a completed turn.
#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub message_id: i64,
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: Option<String>,
    /// True when the caller should refresh the long-term summary.
    pub should_summarize: bool,
}

/// Run one user turn end to end.
///
/// The user message is persisted first; the assistant message is persisted only
/// after a clean stream, so a cancelled turn never leaves a half answer in
/// history. Deltas are forwarded to `tx` as they arrive (dropping the receiver
/// does not abort the turn).
pub async fn run_turn(
    ctx: TurnContext<'_>,
    user_text: &str,
    memory: &MemoryStore,
    provider: Arc<dyn ChatProvider>,
    cancel: CancellationToken,
    tx: mpsc::Sender<ChatDelta>,
) -> Result<TurnOutcome> {
    memory.append_message(NewMessage {
        conversation_id: ctx.conversation_id,
        role: Role::User,
        content: user_text,
        provider: None,
        model: None,
        tokens: None,
    })?;

    let memory_ctx = memory.build_context(
        &ctx.persona.id,
        ctx.conversation_id,
        &ctx.memory_config,
        user_text,
    )?;

    let mut system = ctx
        .persona
        .effective_system_prompt(ctx.pet_name, ctx.pet_state);
    let memory_block = memory_ctx.render();
    if !memory_block.is_empty() {
        system.push_str("\n\n");
        system.push_str(&memory_block);
    }

    let mut request = ChatRequest::new(ctx.model.clone(), system);
    request.temperature = Some(ctx.persona.sampling.temperature);
    request.max_tokens = Some(ctx.persona.sampling.max_tokens.max(1));
    request.messages = memory_ctx
        .recent
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect();

    let (inner_tx, mut inner_rx) = mpsc::channel::<ChatDelta>(64);
    let provider_for_task = provider.clone();
    let provider_task = {
        let cancel = cancel.clone();
        tokio::spawn(async move { provider_for_task.stream(request, inner_tx, cancel).await })
    };

    let mut content = String::new();
    let mut input_tokens = 0u32;
    let mut output_tokens = 0u32;
    let mut finish_reason = None;

    while let Some(delta) = inner_rx.recv().await {
        match &delta {
            ChatDelta::Text { text } => content.push_str(text),
            ChatDelta::Usage {
                input_tokens: i,
                output_tokens: o,
            } => {
                input_tokens = *i;
                output_tokens = *o;
            }
            ChatDelta::Done { finish_reason: r } => finish_reason = r.clone(),
            ChatDelta::Reasoning { .. } | ChatDelta::Status { .. } => {}
        }
        // A closed UI channel must not abort the turn.
        let _ = tx.send(delta).await;
    }

    let stream_result = provider_task
        .await
        .map_err(|e| Error::provider(format!("provider task panicked: {e}")))?;
    stream_result?;

    if content.trim().is_empty() {
        return Err(Error::provider("模型没有返回任何内容"));
    }

    let stored = memory.append_message(NewMessage {
        conversation_id: ctx.conversation_id,
        role: Role::Assistant,
        content: &content,
        provider: Some(provider_id(provider.as_ref()).as_str()),
        model: Some(ctx.model.as_str()),
        tokens: Some(output_tokens),
    })?;

    let should_summarize = ctx.memory_config.long_term
        && ctx.memory_config.summarize_after_turns > 0
        && memory.unsummarized_turns(ctx.conversation_id)?
            >= ctx.memory_config.summarize_after_turns;

    Ok(TurnOutcome {
        message_id: stored.id,
        content,
        input_tokens,
        output_tokens,
        finish_reason,
        should_summarize,
    })
}

fn provider_id(provider: &dyn ChatProvider) -> String {
    provider.id().to_string()
}

/// Regenerate the rolling summary for a conversation.
///
/// Called asynchronously after a turn; failures are non-fatal for the chat.
pub async fn summarize_conversation(
    persona: &Persona,
    conversation_id: &str,
    model: &str,
    memory: &MemoryStore,
    provider: Arc<dyn ChatProvider>,
    cancel: CancellationToken,
) -> Result<()> {
    let messages = memory.messages(conversation_id, 200)?;
    if messages.len() < 4 {
        return Ok(());
    }
    let through = messages.last().map(|m| m.id).unwrap_or(0);
    let transcript = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let who = if m.role == "user" { "用户" } else { "助手" };
            format!("{who}：{}", m.content.replace('\n', " "))
        })
        .collect::<Vec<_>>()
        .join("\n");

    let system = "你是一个记忆整理助手。请把对话压缩成简洁的要点，保留用户的事实、偏好、\
        正在进行的项目和未完成的事项。用第三人称陈述，不要评论，不要加标题，最多 200 字。";
    let mut request = ChatRequest::new(model.to_string(), system);
    request.messages = vec![crate::llm::ChatMessage::user(transcript)];
    request.temperature = Some(0.2);
    request.max_tokens = Some(400);

    let (tx, mut rx) = mpsc::channel::<ChatDelta>(32);
    let task = {
        let cancel = cancel.clone();
        tokio::spawn(async move { provider.stream(request, tx, cancel).await })
    };
    let mut summary = String::new();
    while let Some(delta) = rx.recv().await {
        if let ChatDelta::Text { text } = delta {
            summary.push_str(&text);
        }
    }
    task.await
        .map_err(|e| Error::provider(format!("summarizer panicked: {e}")))??;

    if !summary.trim().is_empty() {
        memory.upsert_summary(conversation_id, through, summary.trim())?;
        tracing::debug!(persona = %persona.id, conversation = %conversation_id, "memory summary updated");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderKind;

    struct EchoProvider {
        id: String,
        text: String,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl ChatProvider for EchoProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn kind(&self) -> ProviderKind {
            ProviderKind::OpenAiChat
        }
        async fn stream(
            &self,
            _request: ChatRequest,
            tx: mpsc::Sender<ChatDelta>,
            _cancel: CancellationToken,
        ) -> Result<()> {
            if self.fail {
                return Err(Error::provider("boom"));
            }
            for chunk in self.text.chars() {
                let _ = tx
                    .send(ChatDelta::Text {
                        text: chunk.to_string(),
                    })
                    .await;
            }
            let _ = tx
                .send(ChatDelta::Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                })
                .await;
            let _ = tx
                .send(ChatDelta::Done {
                    finish_reason: Some("stop".into()),
                })
                .await;
            Ok(())
        }
    }

    fn persona() -> Persona {
        let mut p = Persona::default();
        p.memory.window_turns = 4;
        p.memory.summarize_after_turns = 100;
        p
    }

    fn memory_config(p: &Persona) -> MemoryConfig {
        MemoryConfig {
            enabled: p.memory.enabled,
            window_turns: p.memory.window_turns,
            long_term: p.memory.long_term,
            summarize_after_turns: p.memory.summarize_after_turns,
            ..MemoryConfig::default()
        }
    }

    #[tokio::test]
    async fn persists_both_sides_and_reports_usage() {
        let memory = MemoryStore::open_in_memory().unwrap();
        let conv = memory.create_conversation("default", None).unwrap();
        let p = persona();
        let provider = Arc::new(EchoProvider {
            id: "test".into(),
            text: "你好".into(),
            fail: false,
        });
        let (tx, mut rx) = mpsc::channel(16);
        let outcome = run_turn(
            TurnContext {
                persona: &p,
                conversation_id: &conv.id,
                pet_name: Some("珍珠小子"),
                pet_state: Some("工作中"),
                model: "test-model".to_string(),
                memory_config: memory_config(&p),
            },
            "在吗",
            &memory,
            provider,
            CancellationToken::new(),
            tx,
        )
        .await
        .unwrap();

        assert_eq!(outcome.content, "你好");
        assert_eq!(outcome.input_tokens, 11);
        assert_eq!(outcome.output_tokens, 7);
        assert!(!outcome.should_summarize);
        let mut seen = String::new();
        while let Ok(delta) = rx.try_recv() {
            if let ChatDelta::Text { text } = delta {
                seen.push_str(&text);
            }
        }
        assert_eq!(seen, "你好");
        let stored = memory.messages(&conv.id, 10).unwrap();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].role, "user");
        assert_eq!(stored[1].role, "assistant");
    }

    #[tokio::test]
    async fn failed_stream_does_not_persist_assistant() {
        let memory = MemoryStore::open_in_memory().unwrap();
        let conv = memory.create_conversation("default", None).unwrap();
        let p = persona();
        let provider = Arc::new(EchoProvider {
            id: "test".into(),
            text: String::new(),
            fail: true,
        });
        let (tx, _rx) = mpsc::channel(16);
        let err = run_turn(
            TurnContext {
                persona: &p,
                conversation_id: &conv.id,
                pet_name: None,
                pet_state: None,
                model: "m".to_string(),
                memory_config: memory_config(&p),
            },
            "hi",
            &memory,
            provider,
            CancellationToken::new(),
            tx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Provider(_)));
        let stored = memory.messages(&conv.id, 10).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].role, "user");
    }

    #[tokio::test]
    async fn summary_written_when_threshold_reached() {
        let memory = MemoryStore::open_in_memory().unwrap();
        let conv = memory.create_conversation("default", None).unwrap();
        let p = persona();
        let provider = Arc::new(EchoProvider {
            id: "test".into(),
            text: "摘要".into(),
            fail: false,
        });
        for _ in 0..3 {
            let (tx, _rx) = mpsc::channel(16);
            run_turn(
                TurnContext {
                    persona: &p,
                    conversation_id: &conv.id,
                    pet_name: None,
                    pet_state: None,
                    model: "m".to_string(),
                    memory_config: memory_config(&p),
                },
                "问题",
                &memory,
                provider.clone(),
                CancellationToken::new(),
                tx,
            )
            .await
            .unwrap();
        }
        summarize_conversation(
            &p,
            &conv.id,
            "m",
            &memory,
            provider,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let summary = memory.latest_summary(&conv.id).unwrap().unwrap();
        assert_eq!(summary.content, "摘要");
        assert_eq!(memory.unsummarized_turns(&conv.id).unwrap(), 0);
    }
}
