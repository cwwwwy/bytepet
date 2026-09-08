//! Integration tests for the agent status server, hook installers and the pet
//! zip export/import format.
//!
//! These tests never touch the real `~/.codex` or `~/.claude`: every installer
//! test builds its own HOME inside a `tempfile::tempdir()`.

use std::path::Path;
use std::time::Duration;

use bytepet_core::agent::hooks::{AgentKind, HookInstaller};
use bytepet_core::agent::{spawn_server, AgentEvent, AgentHealth};
use bytepet_core::error::Error;
use bytepet_core::pet::library::PetLibrary;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

// --------------------------------------------------------------- test setup --

fn health() -> AgentHealth {
    AgentHealth {
        ok: true,
        version: "test".to_string(),
        pet: Some("zip".to_string()),
        persona: Some("default".to_string()),
        state: Some("idle".to_string()),
        sources: vec!["codex".to_string()],
    }
}

fn event(state: &str) -> AgentEvent {
    let mut event = AgentEvent::new("codex", state);
    event.message = Some(format!("hello {state}"));
    event
}

// -------------------------------------------------------------- http server --

#[tokio::test]
async fn post_state_validates_and_forwards_events() {
    let (tx, mut rx) = mpsc::channel(16);
    let handle = spawn_server(0, tx, health).await.expect("spawn server");
    let base = handle.url();
    let client = reqwest::Client::new();

    // Valid event -> 202, forwarded to the app channel.
    let response = client
        .post(format!("{base}/state"))
        .json(&event("running"))
        .send()
        .await
        .expect("post");
    assert_eq!(response.status().as_u16(), 202);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["ok"], true);
    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event timeout")
        .expect("event");
    assert_eq!(received.source, "codex");
    assert_eq!(received.state, "running");
    assert_eq!(received.message.as_deref(), Some("hello running"));

    // Invalid state -> 400 with a machine-readable error, nothing forwarded.
    let response = client
        .post(format!("{base}/state"))
        .json(&event("definitely-not-a-state"))
        .send()
        .await
        .expect("post");
    assert_eq!(response.status().as_u16(), 400);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().contains("unknown state"));
    assert!(rx.try_recv().is_err());

    // Empty source -> 400.
    let mut empty_source = event("idle");
    empty_source.source = "   ".to_string();
    let response = client
        .post(format!("{base}/state"))
        .json(&empty_source)
        .send()
        .await
        .expect("post");
    assert_eq!(response.status().as_u16(), 400);

    // Oversized body (> 16 KiB) -> 400, not 413, with a JSON error body.
    let huge = format!(r#"{{"source":"codex","state":"idle","message":"{}"}}"#, "x".repeat(20_000));
    let response = client
        .post(format!("{base}/state"))
        .header("content-type", "application/json")
        .body(huge)
        .send()
        .await
        .expect("post");
    assert_eq!(response.status().as_u16(), 400);
    let body: serde_json::Value = response.json().await.expect("json");
    assert!(body["error"].as_str().unwrap().contains("too large"));

    // Unknown endpoint -> 404 JSON.
    let response = client.get(format!("{base}/nope")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 404);

    handle.stop();
}

#[tokio::test]
async fn health_endpoint_returns_the_snapshot() {
    let (tx, _rx) = mpsc::channel(16);
    let handle = spawn_server(0, tx, health).await.expect("spawn server");
    let body: serde_json::Value = reqwest::get(format!("{}/health", handle.url()))
        .await
        .expect("get health")
        .json()
        .await
        .expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["version"], "test");
    assert_eq!(body["pet"], "zip");
    assert_eq!(body["sources"][0], "codex");
    handle.stop();
}

#[tokio::test]
async fn port_in_use_is_reported_clearly() {
    // Occupy a port with a plain std listener, then ask the server to bind it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let (tx, _rx) = mpsc::channel(16);
    let error = match spawn_server(port, tx, health).await {
        Ok(handle) => {
            handle.stop();
            panic!("binding an occupied port must fail");
        }
        Err(error) => error,
    };
    match error {
        Error::Agent(message) => {
            assert!(message.contains("already in use"), "message: {message}");
            assert!(message.contains(&port.to_string()), "message: {message}");
        }
        other => panic!("expected Error::Agent, got {other:?}"),
    }
    drop(listener);
}

#[tokio::test]
async fn stop_releases_the_port() {
    let (tx, _rx) = mpsc::channel(16);
    let handle = spawn_server(0, tx, health).await.expect("spawn server");
    let port = handle.addr.port();
    handle.stop();

    // The listener is dropped by `stop`; retry briefly in case the runtime
    // needs a tick to finish the aborted task.
    for _ in 0..50 {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("port {port} was not released after stop()");
}

#[tokio::test]
async fn websocket_pushes_health_then_events_and_accepts_inbound() {
    let (tx, mut rx) = mpsc::channel(16);
    let handle = spawn_server(0, tx, health).await.expect("spawn server");
    let port = handle.addr.port();

    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let handshake = format!(
        "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    socket.write_all(handshake.as_bytes()).await.unwrap();

    let (headers, mut pending) = read_http_headers(&mut socket).await;
    assert!(
        headers.starts_with("HTTP/1.1 101"),
        "upgrade failed: {headers}"
    );

    // 1. The first frame is the current health snapshot.
    let snapshot: serde_json::Value =
        serde_json::from_str(&read_text_frame(&mut socket, &mut pending).await).unwrap();
    assert_eq!(snapshot["ok"], true);
    assert_eq!(snapshot["state"], "idle");

    // 2. An HTTP POST is pushed to the WebSocket client.
    reqwest::Client::new()
        .post(format!("{}/state", handle.url()))
        .json(&event("waiting"))
        .send()
        .await
        .unwrap();
    let pushed: serde_json::Value =
        serde_json::from_str(&read_text_frame(&mut socket, &mut pending).await).unwrap();
    assert_eq!(pushed["state"], "waiting");
    assert_eq!(pushed["source"], "codex");
    assert!(rx.recv().await.is_some());

    // 3. An inbound text frame behaves like POST /state.
    let inbound = serde_json::to_string(&event("failed")).unwrap();
    send_text_frame(&mut socket, &inbound).await;
    let echoed: serde_json::Value =
        serde_json::from_str(&read_text_frame(&mut socket, &mut pending).await).unwrap();
    assert_eq!(echoed["state"], "failed");
    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("event timeout")
        .expect("event");
    assert_eq!(received.state, "failed");

    // 4. An invalid inbound frame produces a JSON error frame.
    send_text_frame(&mut socket, r#"{"source":"codex","state":"nope"}"#).await;
    let error: serde_json::Value =
        serde_json::from_str(&read_text_frame(&mut socket, &mut pending).await).unwrap();
    assert_eq!(error["ok"], false);
    assert!(error["error"].as_str().unwrap().contains("unknown state"));

    handle.stop();
}

async fn read_http_headers(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await.unwrap();
        assert!(read > 0, "connection closed during handshake");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_subslice(&buffer, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buffer[..position]).to_string();
            let rest = buffer[position + 4..].to_vec();
            return (headers, rest);
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn read_exact(socket: &mut TcpStream, pending: &mut Vec<u8>, count: usize) -> Vec<u8> {
    while pending.len() < count {
        let mut chunk = [0u8; 4096];
        let read = socket.read(&mut chunk).await.expect("read");
        assert!(read > 0, "connection closed while reading a frame");
        pending.extend_from_slice(&chunk[..read]);
    }
    let out = pending[..count].to_vec();
    pending.drain(..count);
    out
}

/// Minimal RFC 6455 client frame reader (server frames are unmasked).
async fn read_text_frame(socket: &mut TcpStream, pending: &mut Vec<u8>) -> String {
    loop {
        let head = read_exact(socket, pending, 2).await;
        let opcode = head[0] & 0x0f;
        let masked = head[1] & 0x80 != 0;
        let mut length = u64::from(head[1] & 0x7f);
        if length == 126 {
            let extended = read_exact(socket, pending, 2).await;
            length = u64::from(u16::from_be_bytes([extended[0], extended[1]]));
        } else if length == 127 {
            let extended = read_exact(socket, pending, 8).await;
            length = u64::from_be_bytes(extended.try_into().unwrap());
        }
        let mask = if masked {
            Some(read_exact(socket, pending, 4).await)
        } else {
            None
        };
        let mut payload = read_exact(socket, pending, length as usize).await;
        if let Some(mask) = mask {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        match opcode {
            0x1 => return String::from_utf8(payload).expect("utf-8 frame"),
            0x8 => panic!("server closed the websocket"),
            _ => continue,
        }
    }
}

async fn send_text_frame(socket: &mut TcpStream, text: &str) {
    let payload = text.as_bytes();
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut frame = vec![0x81u8];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    socket.write_all(&frame).await.unwrap();
}

// ------------------------------------------------------------------- hooks --

fn write_home(home: &Path) {
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
}

#[test]
fn codex_install_and_uninstall_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".codex").join("config.toml");
    let original = "model = \"gpt-5\"\nnotify = [\"/bin/true\", \"x\"]\n\n[projects.\"/tmp/x\"]\ntrust_level = \"trusted\"\n";
    std::fs::write(&config, original).unwrap();

    let data_dir = tmp.path().join("data");
    let installer = HookInstaller::new(home.clone(), data_dir.clone(), 17872);

    let before = installer.status(AgentKind::Codex).unwrap();
    assert!(before.exists);
    assert!(!before.installed);

    let report = installer.install(AgentKind::Codex).unwrap();
    assert!(report.installed);
    let backup = report.backup_path.clone().unwrap();
    assert!(backup.is_file());
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
    let wrapper = report.wrapper_path.clone().unwrap();
    assert!(wrapper.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&wrapper).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("model = \"gpt-5\""), "other keys survive");
    assert!(text.contains("[projects.\"/tmp/x\"]"), "tables survive");
    assert!(
        text.contains(wrapper.to_str().unwrap()),
        "notify must point at the wrapper: {text}"
    );
    assert!(!text.contains("\"/bin/true\", \"x\"]"), "notify was replaced");

    // The original argv is recorded machine-readably and baked into the wrapper.
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(data_dir.join("hooks/state.json")).unwrap())
            .unwrap();
    let record = &state["records"]["codex"];
    assert_eq!(record["originalNotify"][0], "/bin/true");
    assert_eq!(record["originalNotify"][1], "x");
    assert_eq!(record["port"], 17872);
    let script = std::fs::read_to_string(&wrapper).unwrap();
    assert!(script.contains("'/bin/true'"));
    assert!(script.contains("'x'"));
    assert!(script.contains("17872"));

    let after = installer.status(AgentKind::Codex).unwrap();
    assert!(after.installed);
    assert!(after.detail.contains("notify"));

    // Installing twice must not change anything or lose the pristine backup.
    let second = installer.install(AgentKind::Codex).unwrap();
    assert!(second.installed);
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&config).unwrap(), text);

    installer.uninstall(AgentKind::Codex).unwrap();
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes());
    assert!(!wrapper.exists());
    assert!(!backup.exists());
    assert!(!data_dir.join("hooks/state.json").exists());
    assert!(!installer.status(AgentKind::Codex).unwrap().installed);

    // Idempotent second uninstall.
    let again = installer.uninstall(AgentKind::Codex).unwrap();
    assert!(!again.installed);
}

#[test]
fn codex_uninstall_removes_notify_when_it_was_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".codex").join("config.toml");
    let original = "model = \"gpt-5\"\n";
    std::fs::write(&config, original).unwrap();

    let installer = HookInstaller::new(home.clone(), tmp.path().join("data"), 17872);
    installer.install(AgentKind::Codex).unwrap();
    assert!(std::fs::read_to_string(&config).unwrap().contains("notify"));

    installer.uninstall(AgentKind::Codex).unwrap();
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes());
}

#[test]
fn codex_refuses_to_clobber_an_unparsable_notify() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".codex").join("config.toml");
    std::fs::write(&config, "notify = \"not-an-array\"\n").unwrap();
    let installer = HookInstaller::new(home, tmp.path().join("data"), 17872);
    let error = installer.install(AgentKind::Codex).unwrap_err();
    assert!(error.to_string().contains("refusing"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "notify = \"not-an-array\"\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn generated_codex_wrapper_forwards_then_chains() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    // The wrapper needs curl or wget to reach the server; skip without either.
    let http_tool = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1")
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !http_tool {
        eprintln!("skipping: neither curl nor wget is available");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".codex").join("config.toml");

    // A fake "previous notify" that records the argv it received.
    let marker = tmp.path().join("chain-ran.txt");
    let chain_script = tmp.path().join("previous-notify.sh");
    std::fs::write(
        &chain_script,
        format!(
            "#!/bin/sh\nprintf '%s' \"$*\" > '{}'\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&chain_script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        &config,
        format!(
            "notify = [\"{}\", \"turn-ended\"]\n",
            chain_script.display()
        ),
    )
    .unwrap();

    let (tx, mut rx) = mpsc::channel(16);
    let handle = spawn_server(0, tx, health).await.expect("spawn server");
    let installer = HookInstaller::new(home, tmp.path().join("data"), handle.addr.port());
    installer.install(AgentKind::Codex).unwrap();
    let wrapper = installer.wrapper_path(AgentKind::Codex);

    let payload = r#"{"type":"agent-turn-complete","last-assistant-message":"All done \"quoted\" ok"}"#;
    let status = tokio::process::Command::new(&wrapper)
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .expect("run wrapper");
    assert!(status.success(), "wrapper must exit 0");

    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("wrapper did not post an event")
        .expect("event");
    assert_eq!(received.source, "codex");
    assert_eq!(received.state, "review");
    assert_eq!(
        received.message.as_deref(),
        Some(r#"All done "quoted" ok"#),
        "escaped quotes must survive the translation"
    );

    let chained = std::fs::read_to_string(&marker).expect("chain script did not run");
    assert!(chained.contains("turn-ended"), "chain argv: {chained}");

    // With the server gone the wrapper still exits 0 (and still chains).
    handle.stop();
    std::fs::remove_file(&marker).unwrap();
    let status = tokio::process::Command::new(&wrapper)
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .expect("run wrapper");
    assert!(status.success(), "wrapper must exit 0 when the app is down");
    assert!(marker.is_file(), "chaining must survive a stopped app");
}

#[test]
fn claude_install_and_uninstall_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".claude").join("settings.json");
    let original = "{\n  \"env\": {\n    \"A\": \"1\"\n  },\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"matcher\": \"*\",\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"/usr/bin/other\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";
    std::fs::write(&config, original).unwrap();

    let data_dir = tmp.path().join("data");
    let installer = HookInstaller::new(home.clone(), data_dir.clone(), 17872);

    assert!(!installer.status(AgentKind::ClaudeCode).unwrap().installed);
    let report = installer.install(AgentKind::ClaudeCode).unwrap();
    assert!(report.installed);
    assert!(report.backup_path.as_ref().unwrap().is_file());
    let wrapper = report.wrapper_path.clone().unwrap();
    assert!(wrapper.is_file());

    let installed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(installed["env"]["A"], "1");
    let wrapper_str = wrapper.to_str().unwrap();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Notification",
        "Stop",
        "SubagentStop",
    ] {
        let array = installed["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("missing event {event}"));
        let has_ours = array.iter().any(|item| {
            item.get("hooks")
                .and_then(|hooks| hooks.as_array())
                .is_some_and(|handlers| {
                    handlers.iter().any(|handler| {
                        handler.get("command").and_then(|c| c.as_str()) == Some(wrapper_str)
                    })
                })
        });
        assert!(has_ours, "event {event} does not contain our handler");
    }
    // The pre-existing handler is preserved.
    let stop = installed["hooks"]["Stop"].as_array().unwrap();
    assert!(stop
        .iter()
        .any(|item| item["hooks"][0]["command"] == "/usr/bin/other"));

    let status = installer.status(AgentKind::ClaudeCode).unwrap();
    assert!(status.installed);
    assert!(status.detail.contains("hook event"));

    installer.uninstall(AgentKind::ClaudeCode).unwrap();
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes());
    assert!(!wrapper.exists());
    let again = installer.uninstall(AgentKind::ClaudeCode).unwrap();
    assert!(!again.installed);
}

#[test]
fn claude_supports_legacy_plain_handler_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".claude").join("settings.json");
    let original = r#"{"hooks":{"PreToolUse":[{"type":"command","command":"/legacy"}]}}"#;
    std::fs::write(&config, original).unwrap();

    let installer = HookInstaller::new(home, tmp.path().join("data"), 17872);
    let report = installer.install(AgentKind::ClaudeCode).unwrap();
    let wrapper = report.wrapper_path.unwrap().to_str().unwrap().to_string();

    let installed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let pre = installed["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2, "legacy entry kept, ours appended");
    assert_eq!(pre[0]["command"], "/legacy");
    assert_eq!(pre[1]["command"], wrapper);

    installer.uninstall(AgentKind::ClaudeCode).unwrap();
    assert_eq!(std::fs::read(&config).unwrap(), original.as_bytes());
}

#[test]
fn claude_uninstall_removes_the_file_pet_created() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write_home(&home);
    let config = home.join(".claude").join("settings.json");
    let installer = HookInstaller::new(home, tmp.path().join("data"), 17872);

    installer.install(AgentKind::ClaudeCode).unwrap();
    assert!(config.is_file());
    installer.uninstall(AgentKind::ClaudeCode).unwrap();
    assert!(!config.exists(), "pet-created settings.json should be gone");
}

// -------------------------------------------------------------- zip package --

fn write_pet(dir: &Path, id: &str) {
    use image::RgbaImage;
    std::fs::create_dir_all(dir).unwrap();
    let frame = bytepet_core::pet::manifest::FrameSpec::new(8, 9);
    let image = RgbaImage::new(frame.atlas_width(), frame.atlas_height());
    image.save(dir.join("spritesheet.webp")).unwrap();
    let json = format!(
        r#"{{"id":"{id}","displayName":"Test {id}","spritesheetPath":"spritesheet.webp"}}"#
    );
    std::fs::write(dir.join("pet.json"), json).unwrap();
}

#[test]
fn zip_export_import_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let library = PetLibrary::discover(tmp.path().join("app-pets"));
    write_pet(&tmp.path().join("src/alpha"), "alpha");
    let imported = library.import_dir(&tmp.path().join("src/alpha"), false).unwrap();

    let out = tmp.path().join("alpha.zip");
    library.export_zip("alpha", &out).unwrap();

    let other = PetLibrary::discover(tmp.path().join("other-pets"));
    let round_tripped = other.import_zip(&out, false).unwrap();
    assert_eq!(round_tripped.id, "alpha");
    assert_eq!(
        std::fs::read(&round_tripped.spritesheet).unwrap(),
        std::fs::read(&imported.spritesheet).unwrap()
    );
}
