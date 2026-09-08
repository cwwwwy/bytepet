//! Pet overlay window: geometry, position persistence, and the shared runtime
//! (animation state machine + hit state + walk controller).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use bytepet_core::config::AppConfig;
use bytepet_core::pet::state::{PetEngine, PetState};
use bytepet_core::pet::{PetAtlas, PetEntry};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Runtime};

use super::hit_test::HitState;

/// Shared per-pet runtime.
pub struct PetRuntime {
    pub engine: Mutex<PetEngine>,
    pub hit: Arc<HitState>,
    pub walk: Arc<WalkController>,
    /// The frontend already received its greeting for this pet.
    pub greeted: AtomicBool,
}

/// Auto-walk direction/state shared with the walk loop.
#[derive(Default)]
pub struct WalkController {
    pub enabled: AtomicBool,
    /// Epoch millis of the last user interaction.
    pub last_user_action_ms: AtomicU64,
}

impl WalkController {
    pub fn note_user_action(&self) {
        self.last_user_action_ms
            .store(bytepet_core::memory::now_ms() as u64, Ordering::Relaxed);
    }

    pub fn recently_interacted(&self, grace_seconds: f32) -> bool {
        let last = self.last_user_action_ms.load(Ordering::Relaxed) as i64;
        let grace = (grace_seconds.max(0.0) * 1000.0) as i64;
        bytepet_core::memory::now_ms() - last < grace
    }
}

/// Load a pet and make it the one on screen. Starts the hit-test and walk
/// loops if they are not running yet.
pub fn activate<R: Runtime>(app: &AppHandle<R>, entry: &PetEntry) -> anyhow::Result<()> {
    let (atlas, warnings) = PetAtlas::open(&entry.dir, &entry.manifest)?;
    for warning in &warnings {
        tracing::warn!(pet = %entry.id, %warning, "pet atlas warning");
    }
    let frame = atlas.frame;

    let mut resolved = entry.clone();
    resolved.frame = frame;
    let runtime = Arc::new(PetRuntime {
        engine: Mutex::new(PetEngine::new(frame, &resolved.manifest)),
        hit: Arc::new(HitState::default()),
        walk: Arc::new(WalkController::default()),
        greeted: AtomicBool::new(false),
    });
    runtime.hit.set_atlas(atlas.mask.clone(), frame);

    let config = {
        let state = app
            .try_state::<crate::state::AppState>()
            .ok_or_else(|| anyhow::anyhow!("app state is not initialized"))?;
        runtime
            .walk
            .enabled
            .store(state.config().pet.auto_walk.enabled, Ordering::Relaxed);
        runtime.hit.set_mode(state.config().pet.click_through);
        state.set_pet_runtime(runtime.clone());
        state.config()
    };

    apply_config(app, &config, frame);

    let _ = app.emit(
        crate::events::PET_STATE,
        crate::events::PetStateEvent {
            state: PetState::Idle.name().to_string(),
            source: "system".to_string(),
            message: None,
            one_shot: false,
        },
    );

    tracing::info!(pet = %resolved.id, "pet activated");
    Ok(())
}

/// Activate whichever pet the config selects, falling back to the first one.
pub fn activate_active_pet<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let state = app
        .try_state::<crate::state::AppState>()
        .ok_or_else(|| anyhow::anyhow!("app state is not initialized"))?;
    let config = state.config();
    let entry = config
        .active_pet
        .as_ref()
        .and_then(|id| state.library.get(id))
        .or_else(|| state.library.list().into_iter().next());
    match entry {
        Some(entry) => {
            if config.active_pet.as_deref() != Some(entry.id.as_str()) {
                let id = entry.id.clone();
                state.update_config(|cfg| cfg.active_pet = Some(id))?;
            }
            activate(app, &entry)
        }
        None => {
            tracing::warn!(
                roots = ?state.library.roots().iter().map(|r| r.path.display().to_string()).collect::<Vec<_>>(),
                "no pet found; drop a Codex pet into ~/.codex/pets or import one"
            );
            // Do not leave an empty, click-blocking window on screen.
            if let Some(win) = app.get_webview_window("pet") {
                let _ = win.set_ignore_cursor_events(true);
            }
            Ok(())
        }
    }
}

/// Apply pet-window geometry and behaviour from config.
pub fn apply_config<R: Runtime>(app: &AppHandle<R>, config: &AppConfig, frame: bytepet_core::pet::FrameSpec) {
    let Some(win) = app.get_webview_window("pet") else {
        return;
    };
    let scale = config.pet.scale.clamp(0.5, 3.0);
    let size = LogicalSize::new(
        frame.width as f64 * scale as f64,
        frame.height as f64 * scale as f64,
    );
    let _ = win.set_size(tauri::Size::Logical(size));
    let _ = win.set_always_on_top(config.pet.always_on_top);
    if let Some(pos) = config.pet.start_position {
        let _ = win.set_position(PhysicalPosition::new(pos.x, pos.y));
    }
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        if let Some(runtime) = state.pet_runtime() {
            runtime.hit.set_mode(config.pet.click_through.clone());
            runtime.hit.set_scale(scale);
            runtime
                .walk
                .enabled
                .store(config.pet.auto_walk.enabled, Ordering::Relaxed);
        }
    }
}

/// Persist the current window position into the config.
pub fn save_position<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("pet") else {
        return;
    };
    let Ok(pos) = win.outer_position() else {
        return;
    };
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        let _ = state.update_config(|cfg| {
            cfg.pet.start_position = Some(bytepet_core::config::WindowPosition {
                x: pos.x as f64,
                y: pos.y as f64,
                monitor: None,
            });
        });
    }
}

/// Suspend hit testing while the user drags the pet.
pub fn set_dragging<R: Runtime>(app: &AppHandle<R>, dragging: bool) {
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        if let Some(runtime) = state.pet_runtime() {
            runtime.hit.set_suspended(dragging);
            if dragging {
                runtime.walk.note_user_action();
            } else {
                save_position(app);
            }
        }
    }
}

/// Raise a transient bytepet state from the UI or agent events.
pub fn raise_state<R: Runtime>(
    app: &AppHandle<R>,
    state: PetState,
    source: &str,
    message: Option<String>,
    ttl: Option<std::time::Duration>,
) {
    let Some(app_state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let Some(runtime) = app_state.pet_runtime() else {
        return;
    };
    let transition = runtime
        .engine
        .lock()
        .raise(state, source, message, ttl, std::time::Instant::now());
    if let Some(transition) = transition {
        let _ = app.emit(
            crate::events::PET_STATE,
            crate::events::PetStateEvent {
                state: transition.state.name().to_string(),
                source: transition.source,
                message: transition.message,
                one_shot: transition.one_shot,
            },
        );
    }
}

/// Clear overrides raised by one source (e.g. a cancelled chat turn).
pub fn clear_source<R: Runtime>(app: &AppHandle<R>, source: &str) {
    let Some(app_state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let Some(runtime) = app_state.pet_runtime() else {
        return;
    };
    let transition = runtime
        .engine
        .lock()
        .clear_source(source, std::time::Instant::now());
    if let Some(transition) = transition {
        let _ = app.emit(
            crate::events::PET_STATE,
            crate::events::PetStateEvent {
                state: transition.state.name().to_string(),
                source: transition.source,
                message: transition.message,
                one_shot: transition.one_shot,
            },
        );
    }
}

/// Called by the pet window once its renderer and event listeners are ready.
///
/// The greeting is raised here rather than during activation so the webview
/// cannot miss the event, and only once per activated pet.
pub fn greet<R: Runtime>(app: &AppHandle<R>) {
    let Some(app_state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    let Some(runtime) = app_state.pet_runtime() else {
        return;
    };
    if runtime.greeted.swap(true, Ordering::SeqCst) {
        return;
    }
    let greeting = app_state
        .active_persona()
        .and_then(|persona| persona.greeting);
    raise_state(app, PetState::Waving, "system", greeting, None);
}

/// Current visible state, used to seed a freshly loaded frontend.
pub fn current_state<R: Runtime>(app: &AppHandle<R>) -> Option<crate::events::PetStateEvent> {
    let app_state = app.try_state::<crate::state::AppState>()?;
    let runtime = app_state.pet_runtime()?;
    let engine = runtime.engine.lock();
    let state = engine.current();
    Some(crate::events::PetStateEvent {
        state: state.name().to_string(),
        source: engine.source().unwrap_or("system").to_string(),
        message: engine.bubble().map(str::to_string),
        one_shot: state.is_one_shot(),
    })
}
