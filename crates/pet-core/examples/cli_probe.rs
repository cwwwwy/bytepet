//! End-to-end probe for the local CLI providers.
//!
//! ```text
//! cargo run -p pet-core --example cli_probe -- claude "回复：你好"
//! cargo run -p pet-core --example cli_probe -- codex  "回复：你好"
//! ```
//!
//! Prints every `ChatDelta` so the CLI JSONL mapping can be verified against a
//! real installed agent, not just fixtures.

use std::sync::Arc;

use pet_core::llm::{ChatProvider, ChatRequest, ProviderConfig, ProviderKind};
use pet_core::secrets::MemorySecretStore;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "claude".to_string());
    let prompt = args
        .next()
        .unwrap_or_else(|| "回复：你好".to_string());

    let cfg = match which.as_str() {
        "codex" => ProviderConfig::new("codex", ProviderKind::CodexCli, ""),
        "claude" => ProviderConfig::new("claude", ProviderKind::ClaudeCli, "sonnet"),
        other => anyhow::bail!("unknown provider '{other}' (expected codex|claude)"),
    };

    let provider: Arc<dyn ChatProvider> =
        pet_core::llm::build_provider(&cfg, &MemorySecretStore::new())?;
    println!("provider={} kind={:?}", provider.id(), provider.kind());

    let mut request = ChatRequest::new(cfg.model.clone(), "你是一只简洁的桌宠，回答不超过一句话。");
    request.messages = vec![pet_core::llm::ChatMessage::user(prompt)];
    request.max_tokens = Some(200);

    let (tx, mut rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let handle = tokio::spawn(async move { provider.stream(request, tx, cancel).await });

    let mut text = String::new();
    while let Some(delta) = rx.recv().await {
        match &delta {
            pet_core::llm::ChatDelta::Text { text: t } => {
                text.push_str(t);
                println!("TEXT      {:?}", t);
            }
            pet_core::llm::ChatDelta::Reasoning { text: t } => {
                println!("REASONING {:?}", t.chars().take(60).collect::<String>())
            }
            pet_core::llm::ChatDelta::Status { message } => println!("STATUS    {message}"),
            pet_core::llm::ChatDelta::Usage {
                input_tokens,
                output_tokens,
            } => println!("USAGE     in={input_tokens} out={output_tokens}"),
            pet_core::llm::ChatDelta::Done { finish_reason } => {
                println!("DONE      {finish_reason:?}")
            }
        }
    }
    handle.await??;
    println!("---\nfinal text: {text}");
    Ok(())
}
