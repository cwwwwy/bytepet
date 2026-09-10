//! End-to-end test of the chat path through the real command layer.
//!
//! Not run on Windows: the Tauri mock runtime cannot initialise inside a test
//! binary there, because `tauri-build` only embeds the Common-Controls v6
//! application manifest into binaries (`link-arg-bins`), and without it the
//! runtime load fails with `STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139). See
//! tauri-apps/tauri#11028. The equivalent coverage on Windows comes from the
//! `bytepet-core` suite; the app itself is still built and bundled there.
#![cfg(not(target_os = "windows"))]

//!
//! A local mock of the OpenAI-compatible `/chat/completions` endpoint replaces
//! the model, so this runs hermetically in CI while still exercising the whole
//! real path: `send_message` command -> persona/provider lookup -> HTTP
//! provider -> SSE parsing -> `run_turn` -> SQLite persistence -> events.

use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use bytepet_core::llm::{ProviderConfig, ProviderKind};
use bytepet_core::secrets::{MemorySecretStore, SecretStore};
use tauri::Manager;

const SSE_BODY: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"，世界\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],",
    "\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
    "data: [DONE]\n\n",
);

/// Start a mock OpenAI-compatible server; returns its base URL.
async fn mock_openai() -> String {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                SSE_BODY,
            )
                .into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn build_app() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_message_streams_persists_and_reports_usage() {
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app();
    let mut state = bytepet_lib::state::AppState::initialize_with_dir(tmp.path().to_path_buf())
        .expect("app state");

    // Point the provider at the mock and keep the key out of the keychain.
    let base_url = mock_openai().await;
    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("provider/mock", "sk-test-key").unwrap();
    state.secrets = secrets;

    let provider = ProviderConfig {
        id: "mock".to_string(),
        label: "Mock".to_string(),
        kind: ProviderKind::OpenAiChat,
        base_url: Some(base_url),
        model: "mock-model".to_string(),
        api_key_ref: Some("provider/mock".to_string()),
        ..ProviderConfig::default()
    };
    state
        .update_config(|cfg| {
            cfg.providers.insert("mock".to_string(), provider);
            cfg.default_provider = Some("mock".to_string());
        })
        .unwrap();
    app.manage(state);

    let conversation = bytepet_lib::commands::create_conversation(app.handle().clone(), None)
        .expect("conversation");
    bytepet_lib::commands::send_message(
        app.handle().clone(),
        conversation.id.clone(),
        "打个招呼".to_string(),
    )
    .expect("send_message accepted");

    // Poll until the assistant message is persisted (the turn runs in a task).
    let state = app.state::<bytepet_lib::state::AppState>();
    let mut messages = Vec::new();
    for _ in 0..200 {
        messages = state.memory.messages(&conversation.id, 10).unwrap();
        if messages.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(messages.len(), 2, "expected user + assistant messages");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "打个招呼");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "你好，世界");
    assert_eq!(messages[1].provider.as_deref(), Some("mock"));
    assert_eq!(messages[1].model.as_deref(), Some("mock-model"));
    assert_eq!(messages[1].tokens, 3);

    // The turn must be cancellable-tracked and then released.
    assert!(!state.chat.is_streaming(&conversation.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_message_without_provider_fails_without_persisting_assistant() {
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app();
    let state = bytepet_lib::state::AppState::initialize_with_dir(tmp.path().to_path_buf())
        .expect("app state");
    // No providers configured at all.
    state
        .update_config(|cfg| {
            cfg.providers.clear();
            cfg.default_provider = None;
        })
        .unwrap();
    app.manage(state);

    let conversation = bytepet_lib::commands::create_conversation(app.handle().clone(), None)
        .expect("conversation");
    let err = bytepet_lib::commands::send_message(
        app.handle().clone(),
        conversation.id.clone(),
        "在吗".to_string(),
    )
    .expect_err("should reject when no provider is configured");
    assert!(
        err.contains("模型服务") || err.contains("provider"),
        "got: {err}"
    );

    let state = app.state::<bytepet_lib::state::AppState>();
    assert!(state
        .memory
        .messages(&conversation.id, 10)
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bootstrap_state_exposes_pets_personas_and_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let app = build_app();
    let state = bytepet_lib::state::AppState::initialize_with_dir(tmp.path().to_path_buf())
        .expect("app state");
    app.manage(state);

    let boot = bytepet_lib::commands::get_bootstrap_state(app.handle().clone()).expect("bootstrap");
    assert_eq!(
        boot.config.schema_version,
        bytepet_core::config::CONFIG_SCHEMA_VERSION
    );
    assert!(!boot.personas.is_empty(), "a default persona is seeded");
    assert_eq!(
        boot.agent_url,
        format!("http://127.0.0.1:{}", boot.config.agent.port)
    );
    assert!(boot.data_dir.starts_with(tmp.path()));
    // No pet is installed in the temp library, and none must be invented.
    assert!(boot
        .pets
        .iter()
        .all(|p| p.root != bytepet_core::pet::RootKind::AppData));
}
