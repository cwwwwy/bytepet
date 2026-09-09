//! End-to-end probe for the local CLI providers.
//!
//! ```text
//! cargo run -p bytepet-core --example cli_probe -- claude "回复：你好"
//! cargo run -p bytepet-core --example cli_probe -- codex  "回复：你好"
//! ```
//!
//! Prints every `ChatDelta` so the CLI JSONL mapping can be verified against a
//! real installed agent, not just fixtures.

use std::sync::Arc;

use bytepet_core::llm::{ChatProvider, ChatRequest, ProviderConfig, ProviderKind};
use bytepet_core::secrets::MemorySecretStore;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "claude".to_string());
    let prompt = args.next().unwrap_or_else(|| "回复：你好".to_string());

    let cfg = match which.as_str() {
        "codex" => ProviderConfig::new("codex", ProviderKind::CodexCli, ""),
        "claude" => ProviderConfig::new("claude", ProviderKind::ClaudeCli, "sonnet"),
        other => anyhow::bail!("unknown provider '{other}' (expected codex|claude)"),
    };

    let provider: Arc<dyn ChatProvider> =
        bytepet_core::llm::build_provider(&cfg, &MemorySecretStore::new())?;
    println!("provider={} kind={:?}", provider.id(), provider.kind());

    let mut request = ChatRequest::new(cfg.model.clone(), "你是一只简洁的桌宠，回答不超过一句话。");
    request.messages = vec![bytepet_core::llm::ChatMessage::user(prompt)];
    request.max_tokens = Some(200);

    let (tx, mut rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let handle = tokio::spawn(async move { provider.stream(request, tx, cancel).await });

    let mut text = String::new();
    while let Some(delta) = rx.recv().await {
        match &delta {
            bytepet_core::llm::ChatDelta::Text { text: t } => {
                text.push_str(t);
                println!("TEXT      {:?}", t);
            }
            bytepet_core::llm::ChatDelta::Reasoning { text: t } => {
                println!("REASONING {:?}", t.chars().take(60).collect::<String>())
            }
            bytepet_core::llm::ChatDelta::Status { message } => println!("STATUS    {message}"),
            bytepet_core::llm::ChatDelta::Usage {
                input_tokens,
                output_tokens,
            } => println!("USAGE     in={input_tokens} out={output_tokens}"),
            bytepet_core::llm::ChatDelta::Done { finish_reason } => {
                println!("DONE      {finish_reason:?}")
            }
        }
    }
    handle.await??;
    println!("---\nfinal text: {text}");
    Ok(())
}
