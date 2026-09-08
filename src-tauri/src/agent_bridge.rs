//! Bridges the local agent status protocol to the bytepet state machine.

use bytepet_core::agent::{AgentEvent, AgentHealth};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

/// Start the loopback status server and pump accepted events into the pet.
pub fn start(app: AppHandle) {
    let (enabled, port, default_ttl, auto_install) = {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let config = state.config();
        (
            config.agent.enabled,
            config.agent.port,
            config.agent.default_ttl_seconds,
            config.agent.auto_install_hooks,
        )
    };
    if !enabled {
        tracing::info!("agent status server disabled in settings");
        return;
    }

    if auto_install {
        install_hooks_quietly(&app, port);
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(128);
    let health_app = app.clone();
    let app_for_server = app.clone();

    tauri::async_runtime::spawn(async move {
        let result = bytepet_core::agent::spawn_server(port, tx, move || health(&health_app)).await;
        match result {
            Ok(handle) => {
                tracing::info!(url = %handle.url(), "agent status server listening");
                // Keep the handle alive for the process lifetime.
                if let Some(state) = app_for_server.try_state::<AppState>() {
                    *state.agent_server.write() = Some(handle);
                }
            }
            Err(err) => {
                tracing::warn!(%err, "agent status server failed to start (port in use?)");
            }
        }
    });

    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            handle_event(&app, event, default_ttl);
        }
    });
}

/// Install hooks for both agents when the user opted in. Failures are logged,
/// never fatal: the app must still start if `~/.codex` is read-only.
fn install_hooks_quietly(app: &AppHandle, port: u16) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let home = dirs::home_dir().unwrap_or_else(|| state.paths.config_dir.clone());
    let installer = bytepet_core::agent::HookInstaller::new(home, state.paths.config_dir.clone(), port);
    for kind in [
        bytepet_core::agent::AgentKind::Codex,
        bytepet_core::agent::AgentKind::ClaudeCode,
    ] {
        match installer.install(kind) {
            Ok(report) => tracing::info!(agent = kind.label(), ?report.messages, "agent hooks installed"),
            Err(err) => tracing::warn!(agent = kind.label(), %err, "agent hook install skipped"),
        }
    }
}

fn health(app: &AppHandle) -> AgentHealth {    let mut health = AgentHealth {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        pet: None,
        persona: None,
        state: None,
        sources: Vec::new(),
    };
    if let Some(state) = app.try_state::<AppState>() {
        let config = state.config();
        health.pet = config.active_pet.clone();
        health.persona = config.active_persona.clone();
        if let Some(runtime) = state.pet_runtime() {
            health.state = Some(runtime.engine.lock().current().name().to_string());
        }
    }
    health
}

fn handle_event(app: &AppHandle, event: AgentEvent, default_ttl: u64) {
    tracing::debug!(source = %event.source, state = %event.state, "agent event");
    let _ = app.emit(crate::events::AGENT_EVENT, &event);
    let Some(pet_state) = event.pet_state() else {
        tracing::debug!(state = %event.state, "ignoring unknown agent state");
        return;
    };
    let ttl = event.ttl(default_ttl);
    crate::window::pet_window::raise_state(
        app,
        pet_state,
        &format!("agent:{}", event.source),
        event.message.clone(),
        ttl,
    );
}
