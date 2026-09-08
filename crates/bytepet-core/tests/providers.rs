//! Integration tests for the five LLM providers.
//!
//! The HTTP providers are driven against a local axum server that replays
//! recorded SSE bytes from `tests/fixtures/streams/*.sse`; the CLI providers
//! are driven against a fake executable that replays
//! `tests/fixtures/cli/*.jsonl`. Both paths assert the exact
//! [`ChatDelta`] sequence, the request payload, cancellation and error
//! redaction.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Router;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use bytepet_core::llm::providers::anthropic::AnthropicProvider;
use bytepet_core::llm::providers::openai_chat::OpenAiChatProvider;
use bytepet_core::llm::providers::openai_responses::OpenAiResponsesProvider;
use bytepet_core::llm::{
    ChatDelta, ChatMessage, ChatProvider, ChatRequest, ProviderConfig, ProviderKind,
};
use bytepet_core::secrets::{MemorySecretStore, SecretStore};
use bytepet_core::Error;

const API_KEY: &str = "sk-test-secret-do-not-log";

// ---------------------------------------------------------------------------
// Mock HTTP server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct RecordedRequest {
    headers: HeaderMap,
    body: Option<Value>,
}

type Recorded = Arc<Mutex<Option<RecordedRequest>>>;

#[derive(Clone)]
struct MockState {
    status: StatusCode,
    content_type: &'static str,
    body: Arc<String>,
    recorded: Recorded,
    /// When set, send the body as one chunk and then never finish the response.
    hang: bool,
}

async fn mock_handler(State(state): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20)
        .await
        .unwrap_or_default();
    let parsed = serde_json::from_slice(&bytes).ok();
    *state.recorded.lock() = Some(RecordedRequest {
        headers: parts.headers,
        body: parsed,
    });

    if state.hang {
        let first = state.body.to_string();
        let stream = futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(axum::body::Bytes::from(first))
        })
        .chain(futures_util::stream::pending());
        return Response::builder()
            .status(state.status)
            .header("content-type", state.content_type)
            .body(Body::from_stream(stream))
            .expect("valid response");
    }

    Response::builder()
        .status(state.status)
        .header("content-type", state.content_type)
        .body(Body::from(state.body.to_string()))
        .expect("valid response")
}

async fn spawn_mock(
    status: StatusCode,
    content_type: &'static str,
    body: &str,
    hang: bool,
) -> (String, Recorded) {
    let recorded: Recorded = Arc::new(Mutex::new(None));
    let state = MockState {
        status,
        content_type,
        body: Arc::new(body.to_string()),
        recorded: Arc::clone(&recorded),
        hang,
    };
    let app = Router::new().fallback(mock_handler).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let address = listener.local_addr().expect("mock server address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}"), recorded)
}

// ---------------------------------------------------------------------------
// Provider helpers
// ---------------------------------------------------------------------------

fn provider_config(kind: ProviderKind, base_url: &str) -> ProviderConfig {
    let mut config = ProviderConfig::new("test", kind, "test-model");
    config.base_url = Some(base_url.to_string());
    config
}

fn secrets() -> MemorySecretStore {
    let secrets = MemorySecretStore::new();
    secrets.set("provider/test", API_KEY).expect("store api key");
    secrets
}

/// Run a provider to completion and drain every delta it produced.
async fn collect(provider: Arc<dyn ChatProvider>, request: ChatRequest) -> Vec<ChatDelta> {
    let (tx, mut rx) = mpsc::channel(256);
    provider
        .stream(request, tx, CancellationToken::new())
        .await
        .expect("stream should succeed");
    let mut deltas = Vec::new();
    while let Ok(delta) = rx.try_recv() {
        deltas.push(delta);
    }
    deltas
}

/// Run a provider that is expected to fail.
async fn collect_error(provider: Arc<dyn ChatProvider>, request: ChatRequest) -> Error {
    let (tx, _rx) = mpsc::channel(64);
    provider
        .stream(request, tx, CancellationToken::new())
        .await
        .expect_err("stream should fail")
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("missing header {name}"))
        .to_str()
        .expect("header is ascii")
        .to_string()
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_maps_sse_and_sends_expected_body() {
    let (base, recorded) = spawn_mock(
        StatusCode::OK,
        "text/event-stream",
        include_str!("fixtures/streams/anthropic.sse"),
        false,
    )
    .await;
    let provider: Arc<dyn ChatProvider> = Arc::new(
        AnthropicProvider::new(&provider_config(ProviderKind::Anthropic, &base), &secrets())
            .expect("provider"),
    );

    let mut request = ChatRequest::new("claude-test", "You are a desktop pet.");
    request.messages.push(ChatMessage::user("hi"));
    request.stop.push("STOP".into());

    let deltas = collect(provider, request).await;
    assert_eq!(
        deltas,
        vec![
            ChatDelta::Usage {
                input_tokens: 42,
                output_tokens: 0
            },
            ChatDelta::Reasoning {
                text: "Let me think.".into()
            },
            ChatDelta::Text {
                text: "Hello".into()
            },
            ChatDelta::Text {
                text: " there".into()
            },
            ChatDelta::Usage {
                input_tokens: 42,
                output_tokens: 7
            },
            ChatDelta::Done {
                finish_reason: Some("end_turn".into())
            },
        ]
    );

    let recorded = recorded.lock().clone().expect("request recorded");
    let body = recorded.body.expect("json body");
    assert_eq!(body["model"], "claude-test");
    assert_eq!(body["system"], "You are a desktop pet.");
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["stop_sequences"][0], "STOP");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "hi");
    assert_eq!(body["stream"], true);
    assert_eq!(header(&recorded.headers, "x-api-key"), API_KEY);
    assert_eq!(header(&recorded.headers, "anthropic-version"), "2023-06-01");
}

// ---------------------------------------------------------------------------
// OpenAI chat completions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_chat_maps_sse_and_sends_expected_body() {
    let (base, recorded) = spawn_mock(
        StatusCode::OK,
        "text/event-stream",
        include_str!("fixtures/streams/openai-chat.sse"),
        false,
    )
    .await;
    let mut config = provider_config(ProviderKind::OpenAiChat, &base);
    config
        .extra_headers
        .insert("x-tenant".into(), "pet".into());
    let provider: Arc<dyn ChatProvider> =
        Arc::new(OpenAiChatProvider::new(&config, &secrets()).expect("provider"));

    let mut request = ChatRequest::new("deepseek-chat", "be terse");
    request.messages.push(ChatMessage::user("hi"));
    request.messages.push(ChatMessage::assistant("hello"));
    request.max_tokens = Some(64);
    request.temperature = Some(0.25);
    request.stop.push("STOP".into());

    let deltas = collect(provider, request).await;
    assert_eq!(
        deltas,
        vec![
            ChatDelta::Reasoning {
                text: "Let me think.".into()
            },
            ChatDelta::Text {
                text: "Hello".into()
            },
            ChatDelta::Text {
                text: " there".into()
            },
            ChatDelta::Done {
                finish_reason: Some("stop".into())
            },
            ChatDelta::Usage {
                input_tokens: 9,
                output_tokens: 7
            },
        ]
    );

    let recorded = recorded.lock().clone().expect("request recorded");
    let body = recorded.body.expect("json body");
    assert_eq!(body["model"], "deepseek-chat");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], "be terse");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["messages"][2]["role"], "assistant");
    assert_eq!(body["max_tokens"], 64);
    assert_eq!(body["temperature"], 0.25);
    assert_eq!(body["stop"][0], "STOP");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(
        header(&recorded.headers, "authorization"),
        format!("Bearer {API_KEY}")
    );
    assert_eq!(header(&recorded.headers, "x-tenant"), "pet");
}

// ---------------------------------------------------------------------------
// OpenAI Responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_responses_maps_sse_and_sends_expected_body() {
    let (base, recorded) = spawn_mock(
        StatusCode::OK,
        "text/event-stream",
        include_str!("fixtures/streams/openai-responses.sse"),
        false,
    )
    .await;
    let provider: Arc<dyn ChatProvider> = Arc::new(
        OpenAiResponsesProvider::new(
            &provider_config(ProviderKind::OpenAiResponses, &base),
            &secrets(),
        )
        .expect("provider"),
    );

    let mut request = ChatRequest::new("gpt-5", "be terse");
    request.messages.push(ChatMessage::user("hi"));
    request.messages.push(ChatMessage::assistant("hello"));
    request.max_tokens = Some(256);
    // 0.25 is exact in f32, so the JSON round-trip is lossless.
    request.temperature = Some(0.25);

    let deltas = collect(provider, request).await;
    assert_eq!(
        deltas,
        vec![
            ChatDelta::Reasoning {
                text: "Let me think.".into()
            },
            ChatDelta::Text {
                text: "Hello".into()
            },
            ChatDelta::Text {
                text: " there".into()
            },
            ChatDelta::Usage {
                input_tokens: 11,
                output_tokens: 7
            },
            ChatDelta::Done {
                finish_reason: None
            },
        ]
    );

    let recorded = recorded.lock().clone().expect("request recorded");
    let body = recorded.body.expect("json body");
    assert_eq!(body["model"], "gpt-5");
    assert_eq!(body["instructions"], "be terse");
    assert_eq!(body["max_output_tokens"], 256);
    assert_eq!(body["temperature"], 0.25);
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
    assert_eq!(body["input"][0]["content"][0]["text"], "hi");
    assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    assert_eq!(body["stream"], true);
    assert_eq!(
        header(&recorded.headers, "authorization"),
        format!("Bearer {API_KEY}")
    );
}

// ---------------------------------------------------------------------------
// Cancellation and error redaction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancelling_token_stops_http_stream_within_200ms() {
    let (base, _recorded) = spawn_mock(
        StatusCode::OK,
        "text/event-stream",
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        true,
    )
    .await;
    let provider: Arc<dyn ChatProvider> = Arc::new(
        OpenAiChatProvider::new(&provider_config(ProviderKind::OpenAiChat, &base), &secrets())
            .expect("provider"),
    );

    let (tx, mut rx) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let handle = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            provider
                .stream(ChatRequest::new("m", "s"), tx, cancel)
                .await
        }
    });

    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first delta arrives")
        .expect("delta");
    assert_eq!(
        first,
        ChatDelta::Text {
            text: "hi".into()
        }
    );

    let started = Instant::now();
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_millis(200), handle)
        .await
        .expect("stream must stop within 200ms")
        .expect("task joins");
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "expected cancellation, got {result:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn http_401_error_never_contains_the_api_key() {
    let body = format!(
        r#"{{"error":{{"type":"authentication_error","message":"invalid key {API_KEY}"}}}}"#
    );
    let (base, _recorded) =
        spawn_mock(StatusCode::UNAUTHORIZED, "application/json", &body, false).await;
    let provider: Arc<dyn ChatProvider> = Arc::new(
        AnthropicProvider::new(&provider_config(ProviderKind::Anthropic, &base), &secrets())
            .expect("provider"),
    );

    let error = collect_error(provider, ChatRequest::new("claude-test", "sys")).await;
    let message = error.to_string();
    assert!(message.contains("401"), "status missing: {message}");
    assert!(!message.contains(API_KEY), "api key leaked: {message}");
    assert!(!message.contains("sk-"), "credential leaked: {message}");
}

// ---------------------------------------------------------------------------
// CLI providers
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod cli {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::*;
    use bytepet_core::llm::providers::claude_cli::ClaudeCliProvider;
    use bytepet_core::llm::providers::codex_cli::CodexCliProvider;

    /// Write an executable that records its argv and stdin, then replays
    /// `fixture` on stdout (one JSON object per line).
    fn fake_cli(dir: &Path, fixture: &str) -> PathBuf {
        let replay = dir.join("replay.jsonl");
        std::fs::write(&replay, fixture).expect("write replay fixture");
        let args = dir.join("args.txt");
        let stdin = dir.join("stdin.txt");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{args}'\ncat > '{stdin}'\ncat '{replay}'\n",
            args = args.display(),
            stdin = stdin.display(),
            replay = replay.display(),
        );
        let path = dir.join("fake-cli");
        std::fs::write(&path, script).expect("write fake cli");
        let mut permissions = std::fs::metadata(&path)
            .expect("stat fake cli")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod fake cli");
        path
    }

    fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).expect("write script");
        let mut permissions = std::fs::metadata(&path).expect("stat").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod");
        path
    }

    fn codex_config(script: &Path) -> ProviderConfig {
        let mut config = ProviderConfig::new("codex", ProviderKind::CodexCli, "gpt-5-codex");
        config
            .options
            .insert("binary".into(), json!(script.display().to_string()));
        config
            .options
            .insert("sandbox".into(), json!("read-only"));
        config
    }

    fn recorded_args(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("args.txt"))
            .expect("args recorded")
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn codex_cli_replays_fixture_and_feeds_prompt_on_stdin() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = fake_cli(dir.path(), include_str!("fixtures/cli/codex-exec.jsonl"));
        let provider: Arc<dyn ChatProvider> = Arc::new(
            CodexCliProvider::new(&codex_config(&script)).expect("provider"),
        );

        let mut request = ChatRequest::new("gpt-5-codex", "be nice");
        request.messages.push(ChatMessage::user("hi"));
        let deltas = collect(provider, request).await;
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Status {
                    message: "session:0199a213-81c0-7800-8aa1-bbab2a035a53".into()
                },
                ChatDelta::Reasoning {
                    text: "用户只是打了个招呼，直接回复即可。".into()
                },
                ChatDelta::Status {
                    message: "执行命令：bash -lc true".into()
                },
                ChatDelta::Text {
                    text: "你好！有什么可以帮你的吗？".into()
                },
                ChatDelta::Usage {
                    input_tokens: 1234,
                    output_tokens: 12
                },
                ChatDelta::Done {
                    finish_reason: None
                },
            ]
        );

        assert_eq!(
            recorded_args(dir.path()),
            vec![
                "exec",
                "--json",
                "--model",
                "gpt-5-codex",
                "--sandbox",
                "read-only",
                "-"
            ]
        );
        let stdin = std::fs::read_to_string(dir.path().join("stdin.txt")).expect("stdin recorded");
        assert!(stdin.starts_with("be nice\n\n"), "stdin was {stdin:?}");
        assert!(stdin.contains("user: hi\n"), "stdin was {stdin:?}");
    }

    #[tokio::test]
    async fn codex_cli_resume_uses_session_and_skips_sandbox_flag() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = fake_cli(
            dir.path(),
            "{\"type\":\"thread.started\",\"thread_id\":\"session-abc\"}\n",
        );
        let mut config = codex_config(&script);
        config
            .options
            .insert("resumeSessionId".into(), json!("session-abc"));
        let provider: Arc<dyn ChatProvider> =
            Arc::new(CodexCliProvider::new(&config).expect("provider"));

        let deltas = collect(provider, ChatRequest::new("gpt-5-codex", "sys")).await;
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Status {
                    message: "session:session-abc".into()
                },
                ChatDelta::Done {
                    finish_reason: None
                },
            ]
        );
        assert_eq!(
            recorded_args(dir.path()),
            vec![
                "exec",
                "resume",
                "session-abc",
                "--json",
                "--model",
                "gpt-5-codex",
                "-"
            ]
        );
    }

    #[tokio::test]
    async fn codex_cli_does_not_duplicate_completed_text() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = fake_cli(
            dir.path(),
            concat!(
                "{\"type\":\"agent_message_delta\",\"delta\":\"Hel\"}\n",
                "{\"type\":\"agent_message_delta\",\"delta\":\"lo\"}\n",
                "{\"type\":\"item.completed\",\"item\":{\"id\":\"i1\",\"type\":\"agent_message\",\"text\":\"Hello\"}}\n",
            ),
        );
        let provider: Arc<dyn ChatProvider> = Arc::new(
            CodexCliProvider::new(&codex_config(&script)).expect("provider"),
        );
        let deltas = collect(provider, ChatRequest::new("gpt-5-codex", "sys")).await;
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Text {
                    text: "Hel".into()
                },
                ChatDelta::Text {
                    text: "lo".into()
                },
                ChatDelta::Done {
                    finish_reason: None
                },
            ]
        );
    }

    #[tokio::test]
    async fn codex_cli_enforces_timeout() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = executable(dir.path(), "slow-cli", "#!/bin/sh\ncat > /dev/null\nsleep 30\n");
        let mut config = codex_config(&script);
        config.options.insert("timeoutSecs".into(), json!(1));
        let provider: Arc<dyn ChatProvider> =
            Arc::new(CodexCliProvider::new(&config).expect("provider"));

        let started = Instant::now();
        let error = collect_error(provider, ChatRequest::new("gpt-5-codex", "sys")).await;
        assert!(error.to_string().contains("timed out"), "got {error}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn codex_cli_cancellation_is_prompt() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = executable(
            dir.path(),
            "hang-cli",
            "#!/bin/sh\ncat > /dev/null\necho '{\"type\":\"agent_message_delta\",\"delta\":\"hi\"}'\nsleep 30\n",
        );
        let provider: Arc<dyn ChatProvider> = Arc::new(
            CodexCliProvider::new(&codex_config(&script)).expect("provider"),
        );

        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        let handle = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                provider
                    .stream(ChatRequest::new("gpt-5-codex", "sys"), tx, cancel)
                    .await
            }
        });
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("delta arrives")
            .expect("delta");
        assert_eq!(
            first,
            ChatDelta::Text {
                text: "hi".into()
            }
        );
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("cli stream must stop within 200ms")
            .expect("task joins");
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "expected cancellation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn claude_cli_replays_real_fixture_without_duplication() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = fake_cli(dir.path(), include_str!("fixtures/cli/claude-stream.jsonl"));
        let mut config = ProviderConfig::new("claude", ProviderKind::ClaudeCli, "claude-sonnet-4-5");
        config
            .options
            .insert("binary".into(), json!(script.display().to_string()));
        config
            .options
            .insert("permissionMode".into(), json!("default"));
        let provider: Arc<dyn ChatProvider> =
            Arc::new(ClaudeCliProvider::new(&config).expect("provider"));

        let mut request = ChatRequest::new("claude-sonnet-4-5", "be nice");
        request.messages.push(ChatMessage::user("回复：你好"));
        let deltas = collect(provider, request).await;

        let statuses: Vec<&ChatDelta> = deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Status { .. }))
            .collect();
        assert_eq!(statuses.len(), 1, "exactly one session status");
        assert!(matches!(
            statuses[0],
            ChatDelta::Status { message } if message.starts_with("session:")
        ));

        let reasoning: String = deltas
            .iter()
            .filter_map(|delta| match delta {
                ChatDelta::Reasoning { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            reasoning,
            "The user just said \"回复：你好\" (Reply: Hello). This is a simple greeting request. \
             I should just respond with a greeting in Chinese. No need for tools here."
        );
        let text: String = deltas
            .iter()
            .filter_map(|delta| match delta {
                ChatDelta::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "你好！有什么可以帮你的吗？");

        // The fixture contains 37 thinking deltas, 8 text deltas, two
        // `assistant` lines and one `result` line that repeat the content. The
        // counts prove the fallbacks were deduplicated.
        let reasoning_count = deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Reasoning { .. }))
            .count();
        let text_count = deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Text { .. }))
            .count();
        assert_eq!(reasoning_count, 37);
        assert_eq!(text_count, 8);
        assert_eq!(
            deltas.last(),
            Some(&ChatDelta::Done {
                finish_reason: Some("end_turn".into())
            })
        );
        assert!(deltas.contains(&ChatDelta::Usage {
            input_tokens: 19668,
            output_tokens: 46
        }));

        assert_eq!(
            recorded_args(dir.path()),
            vec![
                "-p",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--verbose",
                "--model",
                "claude-sonnet-4-5",
                "--permission-mode",
                "default"
            ]
        );
        let stdin = std::fs::read_to_string(dir.path().join("stdin.txt")).expect("stdin recorded");
        assert!(stdin.contains("user: 回复：你好"), "stdin was {stdin:?}");
    }
}
