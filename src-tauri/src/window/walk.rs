//! Auto-walk: the pet strolls along the screen and turns around at the edges.
//!
//! The engine's *base* state carries locomotion (`running-left` /
//! `running-right`), so any higher-priority override (chat, agent status)
//! naturally pauses the walk.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytepet_core::pet::state::PetState;
use tauri::{AppHandle, Manager, PhysicalPosition, Runtime};

const TICK: Duration = Duration::from_millis(16);

/// Start the walk loop. It re-reads the active runtime each tick so switching
/// pets never accumulates threads.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let mut last = Instant::now();
        let mut rest_until: Option<Instant> = None;
        let mut direction: f64 = 1.0;
        let mut last_log = Instant::now();

        loop {
            std::thread::sleep(TICK);
            let now = Instant::now();
            let dt = (now - last).as_secs_f64().clamp(0.0, 0.1);
            last = now;

            let Some(state) = app.try_state::<crate::state::AppState>() else {
                continue;
            };
            let Some(runtime) = state.pet_runtime() else {
                continue;
            };
            let cfg = state.config();
            let walk_cfg = cfg.pet.auto_walk.clone();

            if !walk_cfg.enabled || !runtime.walk.enabled.load(Ordering::Relaxed) {
                set_base(&app, &runtime, PetState::Idle);
                continue;
            }

            // Yield to chat/agent overrides and to recent user interaction.
            let busy = {
                let engine = runtime.engine.lock();
                engine.current() != engine.base()
            };
            if busy
                || runtime
                    .walk
                    .recently_interacted(walk_cfg.user_grace_seconds)
            {
                set_base(&app, &runtime, PetState::Idle);
                continue;
            }

            if let Some(until) = rest_until {
                if now < until {
                    set_base(&app, &runtime, PetState::Idle);
                    continue;
                }
                rest_until = None;
            }

            let Some(win) = app.get_webview_window("pet") else {
                continue;
            };
            let Ok(monitor) = win.current_monitor() else {
                continue;
            };
            let Some(monitor) = monitor else { continue };
            let Ok(pos) = win.outer_position() else {
                continue;
            };
            let Ok(size) = win.outer_size() else { continue };
            let scale = monitor.scale_factor();

            let margin = (walk_cfg.edge_margin.max(0.0) as f64 * scale) as i32;
            let min_x = monitor.position().x + margin;
            let max_x =
                monitor.position().x + monitor.size().width as i32 - size.width as i32 - margin;
            if max_x <= min_x {
                set_base(&app, &runtime, PetState::Idle);
                continue;
            }

            let step = (walk_cfg.speed_px_s.max(1.0) as f64 * scale * dt) * direction;
            let mut next_x = pos.x as f64 + step;

            if next_x <= min_x as f64 {
                next_x = min_x as f64;
                direction = 1.0;
                rest_until = Some(now + Duration::from_secs_f32(walk_cfg.pause_seconds.max(0.0)));
            } else if next_x >= max_x as f64 {
                next_x = max_x as f64;
                direction = -1.0;
                rest_until = Some(now + Duration::from_secs_f32(walk_cfg.pause_seconds.max(0.0)));
            }

            let _ = win.set_position(PhysicalPosition::new(next_x.round() as i32, pos.y));
            if last_log.elapsed() >= Duration::from_secs(2) {
                last_log = Instant::now();
                tracing::debug!(
                    x = next_x.round() as i32,
                    direction = if direction >= 0.0 { "right" } else { "left" },
                    "auto-walk"
                );
            }
            let locomotion = if direction >= 0.0 {
                PetState::RunningRight
            } else {
                PetState::RunningLeft
            };
            set_base(&app, &runtime, locomotion);
        }
    });
}

/// Switch the passive base state and tell the renderer about the *visible*
/// state (which may still be a higher-priority override).
fn set_base<R: Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &super::pet_window::PetRuntime,
    state: PetState,
) {
    use tauri::Emitter;
    let mut engine = runtime.engine.lock();
    if engine.base() == state {
        return;
    }
    if !engine.set_base(state) {
        return;
    }
    let current = engine.current();
    let source = engine.source().unwrap_or("system").to_string();
    let message = engine.bubble().map(str::to_string);
    let one_shot = current.is_one_shot();
    drop(engine);
    let _ = app.emit(
        crate::events::PET_STATE,
        crate::events::PetStateEvent {
            state: current.name().to_string(),
            source,
            message,
            one_shot,
        },
    );
}
