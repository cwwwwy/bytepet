//! Minimal local state protocol.
//!
//! Any script, hook or editor plugin can drive the pet over plain HTTP on
//! `127.0.0.1`. The wire format matches the previous BytePet / UniPet protocol
//! (`POST /state` with `{source, state, message, action, ttlMs}`) so existing
//! hooks keep working, but the implementation is a few hundred bytes of
//! `std::net` instead of an async web stack.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pet::state::PetState;

/// Largest accepted request body.
pub const MAX_BODY_BYTES: usize = 16 * 1024;
/// Default port of the local state protocol.
pub const DEFAULT_PORT: u16 = 17872;

/// One state change pushed by an external hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    #[serde(default = "default_source")]
    pub source: String,
    pub state: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default, alias = "ttl_ms")]
    pub ttl_ms: Option<u64>,
}

fn default_source() -> String {
    "hook".to_string()
}

impl StateEvent {
    /// Resolve the wire state name to a renderable [`PetState`].
    pub fn pet_state(&self) -> Option<PetState> {
        PetState::from_name(&self.state)
    }

    pub fn ttl(&self) -> Option<Duration> {
        match self.ttl_ms {
            Some(0) => None,
            Some(ms) => Some(Duration::from_millis(ms)),
            None => None,
        }
    }

    /// Message clipped to the protocol limit.
    pub fn message_clipped(&self) -> Option<String> {
        self.message
            .as_ref()
            .map(|message| message.chars().take(500).collect::<String>())
    }
}

/// Snapshot served by `GET /health` and `GET /pets`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub ok: bool,
    pub version: String,
    pub pet: String,
    pub pet_path: Option<PathBuf>,
    pub persona: String,
    pub state: String,
    pub pets: Vec<String>,
    pub sources: Vec<String>,
}

/// A running state server.
pub struct StateServer {
    address: SocketAddr,
    health: Arc<Mutex<Health>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StateServer {
    /// Bind `127.0.0.1:port` (`0` picks a free port) and start serving.
    pub fn start(port: u16, sender: Sender<StateEvent>) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| {
            Error::config(format!(
                "cannot bind 127.0.0.1:{port} for the state protocol: {error}"
            ))
        })?;
        let address = listener.local_addr().map_err(|error| {
            Error::config(format!("cannot read the state server address: {error}"))
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            Error::config(format!("cannot configure the state server: {error}"))
        })?;

        let health = Arc::new(Mutex::new(Health {
            ok: true,
            ..Health::default()
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let handle = {
            let health = Arc::clone(&health);
            let shutdown = Arc::clone(&shutdown);
            std::thread::Builder::new()
                .name("bytepet-state".to_string())
                .spawn(move || serve(listener, sender, health, shutdown))
                .map_err(|error| {
                    Error::config(format!("cannot spawn the state server thread: {error}"))
                })?
        };

        Ok(Self {
            address,
            health,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Replace the snapshot returned by `GET /health`.
    pub fn set_health(&self, health: Health) {
        if let Ok(mut guard) = self.health.lock() {
            *guard = health;
        }
    }
}

impl Drop for StateServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake the non-blocking accept loop.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(50));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(
    listener: TcpListener,
    sender: Sender<StateEvent>,
    health: Arc<Mutex<Health>>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                // On Windows the accepted socket inherits the listener's
                // non-blocking mode, which made reads return `WouldBlock` and
                // dropped the connection without a reply. Serve it blocking
                // with a timeout instead.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = handle_connection(stream, &sender, &health);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    sender: &Sender<StateEvent>,
    health: &Arc<Mutex<Health>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length.min(MAX_BODY_BYTES)];
    if !body.is_empty() {
        reader.read_exact(&mut body)?;
    }

    let response = match (method.as_str(), path.as_str()) {
        ("POST", "/state") => match serde_json::from_slice::<StateEvent>(&body) {
            Ok(event) => match event.pet_state() {
                Some(_) => {
                    let _ = sender.send(event);
                    (202, r#"{"ok":true}"#.to_string())
                }
                None => (
                    400,
                    format!(
                        r#"{{"ok":false,"error":"unknown state {:?}"}}"#,
                        event.state
                    ),
                ),
            },
            Err(error) => (400, format!(r#"{{"ok":false,"error":"{error}"}}"#)),
        },
        ("GET", "/health") => {
            let snapshot = health.lock().map(|guard| guard.clone()).unwrap_or_default();
            match serde_json::to_string(&snapshot) {
                Ok(json) => (200, json),
                Err(error) => (500, format!(r#"{{"ok":false,"error":"{error}"}}"#)),
            }
        }
        ("GET", "/pets") => {
            let snapshot = health.lock().map(|guard| guard.clone()).unwrap_or_default();
            match serde_json::to_string(&snapshot.pets) {
                Ok(json) => (200, json),
                Err(error) => (500, format!(r#"{{"ok":false,"error":"{error}"}}"#)),
            }
        }
        _ => (404, r#"{"ok":false,"error":"not found"}"#.to_string()),
    };

    let reason = match response.0 {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response.0,
        reason,
        response.1.len(),
        response.1
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    fn post(port: u16, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let request = format!(
            "POST /state HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        use std::io::Read as _;
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn accepts_known_states_and_rejects_unknown_ones() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let server = StateServer::start(0, sender).unwrap();
        let port = server.port();

        let response = post(
            port,
            r#"{"source":"codex","state":"running","message":"跑测试","ttlMs":1000}"#,
        );
        assert!(response.starts_with("HTTP/1.1 202"), "{response}");
        let event = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.source, "codex");
        assert_eq!(event.pet_state(), Some(PetState::Running));
        assert_eq!(event.ttl(), Some(Duration::from_millis(1000)));
        assert_eq!(event.message_clipped().as_deref(), Some("跑测试"));

        let response = post(port, r#"{"state":"nonsense"}"#);
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn serves_the_health_snapshot() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        let server = StateServer::start(0, sender).unwrap();
        server.set_health(Health {
            ok: true,
            version: "test".to_string(),
            pet: "boba".to_string(),
            persona: "default".to_string(),
            state: "idle".to_string(),
            pets: vec!["boba".to_string()],
            ..Health::default()
        });
        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream
            .write_all(b"GET /health HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        use std::io::Read as _;
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(r#""pet":"boba""#), "{response}");
    }

    #[test]
    fn every_request_gets_a_response() {
        // Regression: the accepted socket inherited the listener's non-blocking
        // mode on Windows, so some requests were dropped with no reply at all.
        let (sender, _receiver) = std::sync::mpsc::channel();
        let server = StateServer::start(0, sender).unwrap();
        for attempt in 0..25 {
            let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
            stream
                .write_all(b"GET /pets HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            use std::io::Read as _;
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with("HTTP/1.1 200"),
                "attempt {attempt} got {response:?}"
            );
        }
    }

    #[test]
    fn ttl_zero_means_no_expiry() {
        let event = StateEvent {
            source: "s".to_string(),
            state: "idle".to_string(),
            message: None,
            action: None,
            ttl_ms: Some(0),
        };
        assert_eq!(event.ttl(), None);
    }
}
