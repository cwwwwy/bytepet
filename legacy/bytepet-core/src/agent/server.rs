//! Loopback HTTP + WebSocket status server.
//!
//! Endpoints:
//! * `POST /state`  `{source, state, message?, action?, ttlMs?}`
//! * `GET  /health` -> [`AgentHealth`]
//! * `GET  /ws`     WebSocket; the server pushes the current health snapshot on
//!   connect and then every accepted event. Text frames sent by the client are
//!   parsed as [`AgentEvent`] and treated exactly like `POST /state`.
//!
//! Security notes: the listener is bound to `127.0.0.1` only (never
//! `0.0.0.0`), bodies are capped at [`MAX_BODY_BYTES`] and every inbound event
//! is validated with [`AgentEvent::validate`] before it reaches the app.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::agent::{AgentEvent, AgentHealth};
use crate::error::{Error, Result};

/// Maximum accepted `POST /state` / WebSocket text frame size (16 KiB).
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// How many events a slow WebSocket client may fall behind before it is told
/// that it lagged instead of blocking the server.
const BROADCAST_CAPACITY: usize = 256;

/// Handle to a running server.
pub struct ServerHandle {
    pub addr: SocketAddr,
    pub cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn stop(self) {
        self.cancel.cancel();
        self.join.abort();
    }
}

/// Health snapshot provider supplied by the app.
type HealthFn = Arc<dyn Fn() -> AgentHealth + Send + Sync>;

struct Inner {
    events: mpsc::Sender<AgentEvent>,
    broadcast: broadcast::Sender<AgentEvent>,
    health: HealthFn,
}

/// Start the loopback status server.
///
/// `port == 0` asks the OS for a free port; the real port is reported through
/// [`ServerHandle::addr`].
pub async fn spawn(
    port: u16,
    events: mpsc::Sender<AgentEvent>,
    health: impl Fn() -> AgentHealth + Send + Sync + 'static,
) -> Result<ServerHandle> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrInUse => Error::Agent(format!(
                "port {port} is already in use on 127.0.0.1; another pet instance (or a \
                 different program) is listening there. Start with `--port <other>` or set \
                 BYTEPET_AGENT_PORT."
            )),
            std::io::ErrorKind::PermissionDenied => Error::Agent(format!(
                "not allowed to bind 127.0.0.1:{port} ({err}); ports below 1024 need elevated \
                 privileges on some systems"
            )),
            _ => Error::Agent(format!("cannot bind 127.0.0.1:{port}: {err}")),
        })?;

    let addr = listener
        .local_addr()
        .map_err(|err| Error::Agent(format!("cannot read the bound address: {err}")))?;

    let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
    let inner = Arc::new(Inner {
        events,
        broadcast: broadcast_tx,
        health: Arc::new(health),
    });

    let app = Router::new()
        .route("/state", post(post_state))
        .route("/health", get(get_health))
        .route("/ws", get(get_ws))
        .fallback(fallback)
        .with_state(inner);

    let cancel = CancellationToken::new();
    let shutdown = cancel.clone();
    let join = tokio::spawn(async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
        if let Err(err) = served {
            tracing::warn!(%err, "agent status server stopped with an error");
        }
    });

    tracing::info!(%addr, "agent status server listening");
    Ok(ServerHandle { addr, cancel, join })
}

async fn fallback() -> Response {
    json_error(StatusCode::NOT_FOUND, "unknown endpoint")
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "ok": false, "error": message.into() })),
    )
        .into_response()
}

/// Decode, validate and fan out one event. Returns the error message on failure.
fn accept_event(inner: &Inner, bytes: &[u8]) -> std::result::Result<AgentEvent, String> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(format!(
            "request body is too large (max {MAX_BODY_BYTES} bytes)"
        ));
    }
    let event: AgentEvent =
        serde_json::from_slice(bytes).map_err(|err| format!("invalid AgentEvent JSON: {err}"))?;
    event.validate().map_err(|err| err.to_string())?;
    // `try_send` keeps a stuck app from blocking the HTTP/WS endpoint. A full
    // channel is backpressure, not an error for the caller: the event is still
    // broadcast to WebSocket clients.
    match inner.events.try_send(event.clone()) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!(source = %event.source, "agent event channel is full; dropping event");
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            return Err("pet app event channel is closed".to_string());
        }
    }
    let _ = inner.broadcast.send(event.clone());
    Ok(event)
}

async fn post_state(State(inner): State<Arc<Inner>>, request: Request) -> Response {
    let body = match to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                format!("request body is too large (max {MAX_BODY_BYTES} bytes)"),
            )
        }
    };
    match accept_event(&inner, &body) {
        Ok(_) => (StatusCode::ACCEPTED, Json(json!({ "ok": true }))).into_response(),
        Err(message) => json_error(StatusCode::BAD_REQUEST, message),
    }
}

async fn get_health(State(inner): State<Arc<Inner>>) -> Response {
    Json((inner.health)()).into_response()
}

async fn get_ws(State(inner): State<Arc<Inner>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| ws_session(socket, inner))
}

async fn ws_session(mut socket: WebSocket, inner: Arc<Inner>) {
    // Snapshot first so a client always sees the current state before events.
    let snapshot = match serde_json::to_string(&(inner.health)()) {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(%err, "cannot serialize agent health");
            return;
        }
    };
    if socket.send(Message::Text(snapshot.into())).await.is_err() {
        return;
    }

    let mut rx = inner.broadcast.subscribe();
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(event) => {
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "websocket client lagged behind the event stream");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Err(message) = accept_event(&inner, text.as_bytes()) {
                        let payload = json!({ "ok": false, "error": message }).to_string();
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_snapshot_serializes_camel_case() {
        let health = AgentHealth {
            ok: true,
            version: "test".to_string(),
            pet: Some("zip".to_string()),
            persona: None,
            state: Some("idle".to_string()),
            sources: vec![],
        };
        let text = serde_json::to_string(&health).unwrap();
        assert!(text.contains("\"ok\":true"));
        assert!(text.contains("\"version\":\"test\""));
    }
}
