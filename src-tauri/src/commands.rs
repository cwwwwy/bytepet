//! Tauri command surface (frontend -> Rust).
//!
//! Every command is thin: it reads/writes [`crate::state::AppState`] and returns
//! serialisable data. Model streaming reports back through events.
//!
//! Argument convention: command parameters are flat and named in camelCase on
//! the JavaScript side (`invoke("use_pet", { id })`); Tauri maps `newId` to the
//! Rust `new_id`. Optional parameters may be omitted entirely.

use pet_core::config::AppConfig;
use pet_core::llm::ProviderConfig;
use pet_core::memory::{Conversation, StoredMessage};
use pet_core::persona::Persona;
use pet_core::pet::library::{LibraryRoot, ValidationReport};
use pet_core::pet::{PetEntry, PetState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::state::{state as app_state, AppState};

/// Everything the frontend needs on boot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub config: AppConfig,
    pub pets: Vec<PetEntry>,
    pub personas: Vec<Persona>,
    pub active_pet: Option<PetEntry>,
    pub active_persona: Option<Persona>,
    pub library_roots: Vec<LibraryRoot>,
    pub agent_url: String,
    pub data_dir: std::path::PathBuf,
    pub keyring_available: bool,
    /// State currently shown by the engine, so a freshly loaded renderer can
    /// resume mid-animation instead of waiting for the next event.
    pub current_state: Option<crate::events::PetStateEvent>,
}

fn bootstrap(st: &AppState) -> BootstrapState {
    let config = st.config();
    let pets = st.library.list();
    let personas = st.personas.list().unwrap_or_default();
    let active_pet = config
        .active_pet
        .as_ref()
        .and_then(|id| pets.iter().find(|p| &p.id == id).cloned())
        .or_else(|| pets.first().cloned());
    let active_persona = config
        .active_persona
        .as_ref()
        .and_then(|id| personas.iter().find(|p| &p.id == id).cloned())
        .or_else(|| personas.first().cloned());
    BootstrapState {
        library_roots: st.library.roots().to_vec(),
        agent_url: st.agent_url(),
        data_dir: st.paths.config_dir.clone(),
        keyring_available: st.keyring_available,
        current_state: None,
        config,
        pets,
        personas,
        active_pet,
        active_persona,
    }
}

#[tauri::command]
pub fn get_bootstrap_state<R: Runtime>(app: AppHandle<R>) -> Result<BootstrapState, String> {
    tracing::debug!("bootstrap requested by a window");
    let mut boot = bootstrap(&app_state(&app));
    boot.current_state = crate::window::pet_window::current_state(&app);
    Ok(boot)
}

// --- pets -------------------------------------------------------------------

#[tauri::command]
pub fn list_pets<R: Runtime>(app: AppHandle<R>) -> Vec<PetEntry> {
    app_state(&app).library.list()
}

#[tauri::command]
pub fn validate_pet(path: String) -> ValidationReport {
    pet_core::pet::PetLibrary::validate_dir(std::path::Path::new(&path))
}

#[tauri::command]
pub fn import_pet<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    overwrite: Option<bool>,
) -> Result<BootstrapState, String> {
    let st = app_state(&app);
    let path = std::path::Path::new(&path);
    if path.is_file() {
        st.library
            .import_zip(path, overwrite.unwrap_or(false))
            .map_err(|e| e.to_string())?;
    } else {
        st.library
            .import_dir(path, overwrite.unwrap_or(false))
            .map_err(|e| e.to_string())?;
    }
    Ok(bootstrap(&st))
}

#[tauri::command]
pub fn export_pet<R: Runtime>(app: AppHandle<R>, id: String, out: String) -> Result<(), String> {
    app_state(&app)
        .library
        .export_zip(&id, std::path::Path::new(&out))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_pet<R: Runtime>(app: AppHandle<R>, id: String) -> Result<BootstrapState, String> {
    let st = app_state(&app);
    st.library.remove_local(&id).map_err(|e| e.to_string())?;
    let next = st.library.list().into_iter().next().map(|p| p.id);
    st.update_config(|cfg| cfg.active_pet = next)
        .map_err(|e| e.to_string())?;
    crate::window::pet_window::activate_active_pet(&app).map_err(|e| e.to_string())?;
    Ok(bootstrap(&st))
}

#[tauri::command]
pub fn use_pet<R: Runtime>(app: AppHandle<R>, id: String) -> Result<BootstrapState, String> {
    let st = app_state(&app);
    let entry = st
        .library
        .get(&id)
        .ok_or_else(|| format!("pet '{}' was not found", id))?;
    st.update_config(|cfg| cfg.active_pet = Some(id.clone()))
        .map_err(|e| e.to_string())?;
    crate::window::pet_window::activate(&app, &entry).map_err(|e| e.to_string())?;
    let _ = app.emit(crate::events::PET_LIBRARY_CHANGED, &id);
    Ok(bootstrap(&st))
}

// --- personas ---------------------------------------------------------------

#[tauri::command]
pub fn list_personas<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Persona>, String> {
    app_state(&app).personas.list().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn persona_templates() -> std::collections::BTreeMap<String, Persona> {
    pet_core::persona::templates()
}

#[tauri::command]
pub fn save_persona<R: Runtime>(app: AppHandle<R>, persona: Persona) -> Result<Vec<Persona>, String> {
    let st = app_state(&app);
    st.personas.save(&persona).map_err(|e| e.to_string())?;
    let list = st.personas.list().map_err(|e| e.to_string())?;
    let _ = app.emit(crate::events::PERSONA_CHANGED, &persona);
    Ok(list)
}

#[tauri::command]
pub fn delete_persona<R: Runtime>(app: AppHandle<R>, id: String) -> Result<Vec<Persona>, String> {
    let st = app_state(&app);
    st.personas.delete(&id).map_err(|e| e.to_string())?;
    if st.config().active_persona.as_deref() == Some(id.as_str()) {
        let next = st
            .personas
            .list()
            .map_err(|e| e.to_string())?
            .first()
            .map(|p| p.id.clone());
        st.update_config(|cfg| cfg.active_persona = next)
            .map_err(|e| e.to_string())?;
    }
    st.personas.list().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn duplicate_persona<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    new_id: String,
    new_name: String,
) -> Result<Vec<Persona>, String> {
    let st = app_state(&app);
    st.personas
        .duplicate(&id, &new_id, &new_name)
        .map_err(|e| e.to_string())?;
    st.personas.list().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_persona<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    overwrite: Option<bool>,
) -> Result<Vec<Persona>, String> {
    let st = app_state(&app);
    st.personas
        .import_file(std::path::Path::new(&path), overwrite.unwrap_or(false))
        .map_err(|e| e.to_string())?;
    st.personas.list().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_persona<R: Runtime>(app: AppHandle<R>, id: String, out: String) -> Result<(), String> {
    app_state(&app)
        .personas
        .export_file(&id, std::path::Path::new(&out))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn use_persona<R: Runtime>(app: AppHandle<R>, id: String) -> Result<BootstrapState, String> {
    let st = app_state(&app);
    let persona = st
        .personas
        .get(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("persona '{}' was not found", id))?;
    let avatar = persona.avatar_pet.clone();
    st.update_config(|cfg| cfg.active_persona = Some(id.clone()))
        .map_err(|e| e.to_string())?;
    if let Some(pet_id) = avatar {
        if st.library.get(&pet_id).is_some() {
            st.update_config(|cfg| cfg.active_pet = Some(pet_id.clone()))
                .map_err(|e| e.to_string())?;
            if let Some(entry) = st.library.get(&pet_id) {
                crate::window::pet_window::activate(&app, &entry).map_err(|e| e.to_string())?;
            }
        }
    }
    let _ = app.emit(crate::events::PERSONA_CHANGED, &persona);
    Ok(bootstrap(&st))
}

// --- providers --------------------------------------------------------------

#[tauri::command]
pub fn list_providers<R: Runtime>(app: AppHandle<R>) -> Vec<ProviderConfig> {
    app_state(&app).config().providers.into_values().collect()
}

#[tauri::command]
pub fn save_provider<R: Runtime>(
    app: AppHandle<R>,
    provider: ProviderConfig,
) -> Result<Vec<ProviderConfig>, String> {
    let st = app_state(&app);
    provider.validate().map_err(|e| e.to_string())?;
    let id = provider.id.clone();
    st.update_config(|cfg| {
        cfg.providers.insert(id.clone(), provider.clone());
        if cfg.default_provider.is_none() {
            cfg.default_provider = Some(id.clone());
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(st.config().providers.into_values().collect())
}

#[tauri::command]
pub fn delete_provider<R: Runtime>(app: AppHandle<R>, id: String) -> Result<Vec<ProviderConfig>, String> {
    let st = app_state(&app);
    let _ = st.secrets.delete(&format!("provider/{}", id));
    st.update_config(|cfg| {
        cfg.providers.remove(&id);
        if cfg.default_provider.as_deref() == Some(id.as_str()) {
            cfg.default_provider = cfg.providers.keys().next().cloned();
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(st.config().providers.into_values().collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}

#[tauri::command]
pub async fn test_provider<R: Runtime>(app: AppHandle<R>, id: String) -> Result<ProviderTestResult, String> {
    let (cfg, secrets) = {
        let st = app_state(&app);
        let cfg = st
            .config()
            .providers
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("provider '{}' does not exist", id))?;
        (cfg, st.secrets.clone())
    };
    let provider = match pet_core::llm::build_provider(&cfg, secrets.as_ref()) {
        Ok(p) => p,
        Err(err) => {
            return Ok(ProviderTestResult {
                ok: false,
                message: pet_core::secrets::redact(&err.to_string()),
                latency_ms: None,
            })
        }
    };
    let started = std::time::Instant::now();
    match provider.probe().await {
        Ok(()) => Ok(ProviderTestResult {
            ok: true,
            message: "连接正常".to_string(),
            latency_ms: Some(started.elapsed().as_millis() as u64),
        }),
        Err(err) => Ok(ProviderTestResult {
            ok: false,
            message: pet_core::secrets::redact(&err.to_string()),
            latency_ms: Some(started.elapsed().as_millis() as u64),
        }),
    }
}

#[tauri::command]
pub fn set_api_key<R: Runtime>(app: AppHandle<R>, provider_id: String, key: String) -> Result<(), String> {
    app_state(&app)
        .secrets
        .set(&format!("provider/{}", provider_id), key.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn has_api_key<R: Runtime>(app: AppHandle<R>, provider_id: String) -> Result<bool, String> {
    Ok(app_state(&app)
        .secrets
        .get(&format!("provider/{}", provider_id))
        .map_err(|e| e.to_string())?
        .is_some())
}

// --- conversations ----------------------------------------------------------

#[tauri::command]
pub fn list_conversations<R: Runtime>(
    app: AppHandle<R>,
    persona_id: Option<String>,
) -> Result<Vec<Conversation>, String> {
    let st = app_state(&app);
    let persona = persona_id;
    st.memory
        .conversations(persona.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_conversation<R: Runtime>(
    app: AppHandle<R>,
    persona_id: Option<String>,
) -> Result<Conversation, String> {
    let st = app_state(&app);
    let persona_id = persona_id
        .or_else(|| st.config().active_persona.clone())
        .or_else(|| st.personas.list().ok().and_then(|p| p.into_iter().next().map(|p| p.id)))
        .ok_or("没有可用人格")?;
    st.memory
        .create_conversation(&persona_id, None)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_messages<R: Runtime>(
    app: AppHandle<R>,
    conversation_id: String,
    limit: Option<usize>,
) -> Result<Vec<StoredMessage>, String> {
    app_state(&app)
        .memory
        .messages(&conversation_id, limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_conversation<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    app_state(&app)
        .memory
        .delete_conversation(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_message<R: Runtime>(
    app: AppHandle<R>,
    conversation_id: String,
    text: String,
) -> Result<(), String> {
    let st = app_state(&app);
    let memory = st.memory.clone();
    st.chat
        .send(app.clone(), memory, conversation_id, text)
}

#[tauri::command]
pub fn cancel_message<R: Runtime>(app: AppHandle<R>, conversation_id: String) {
    app_state(&app).chat.cancel(&conversation_id);
}

// --- memory -----------------------------------------------------------------

#[tauri::command]
pub fn list_facts<R: Runtime>(
    app: AppHandle<R>,
    persona_id: String,
) -> Result<Vec<pet_core::memory::Fact>, String> {
    app_state(&app)
        .memory
        .list_facts(&persona_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_fact<R: Runtime>(app: AppHandle<R>, id: i64) -> Result<(), String> {
    app_state(&app)
        .memory
        .delete_fact(id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_memory<R: Runtime>(app: AppHandle<R>, persona_id: String) -> Result<(), String> {
    app_state(&app)
        .memory
        .clear_persona_memory(&persona_id)
        .map_err(|e| e.to_string())
}

// --- settings ---------------------------------------------------------------

#[tauri::command]
pub fn get_settings<R: Runtime>(app: AppHandle<R>) -> AppConfig {
    app_state(&app).config()
}

#[tauri::command]
pub fn save_settings<R: Runtime>(app: AppHandle<R>, config: AppConfig) -> Result<AppConfig, String> {
    let st = app_state(&app);
    st.update_config(|current| *current = config)
        .map_err(|e| e.to_string())?;
    let saved = st.config();
    crate::window::pet_window::activate_active_pet(&app).map_err(|e| e.to_string())?;
    if let Some(state) = app.try_state::<AppState>() {
        if let Some(runtime) = state.pet_runtime() {
            runtime
                .walk
                .enabled
                .store(saved.pet.auto_walk.enabled, std::sync::atomic::Ordering::Relaxed);
            runtime.hit.set_mode(saved.pet.click_through.clone());
        }
    }
    let _ = app.emit(crate::events::SETTINGS_CHANGED, &saved);
    Ok(saved)
}

// --- pet state / tts / hooks ------------------------------------------------

/// Manual state override, used by the UI (and useful for debugging).
#[tauri::command]
pub fn set_pet_state<R: Runtime>(
    app: AppHandle<R>,
    state: String,
    message: Option<String>,
    ttl_ms: Option<u64>,
) -> Result<(), String> {
    let state = PetState::from_name(&state)
        .ok_or_else(|| format!("unknown pet state '{}'", state))?;
    let ttl = ttl_ms.map(std::time::Duration::from_millis);
    crate::window::pet_window::raise_state(&app, state, "ui", message, ttl);
    Ok(())
}

#[tauri::command]
pub fn speak<R: Runtime>(app: AppHandle<R>, text: String) {
    let st = app_state(&app);
    let config = st.config();
    let persona = st.active_persona();
    let voice = persona
        .as_ref()
        .and_then(|p| p.tts.voice.clone())
        .or_else(|| config.tts.voice.clone());
    let rate = persona
        .as_ref()
        .map(|p| if p.tts.rate > 0.0 { p.tts.rate } else { config.tts.rate })
        .unwrap_or(config.tts.rate);
    st.tts
        .speak(&app, &text, voice, rate, config.tts.max_chars.max(50));
}

#[tauri::command]
pub fn stop_speaking<R: Runtime>(app: AppHandle<R>) {
    app_state(&app).tts.stop(&app);
}

#[tauri::command]
pub fn hooks_status<R: Runtime>(app: AppHandle<R>) -> Result<Vec<pet_core::agent::HookStatus>, String> {
    let st = app_state(&app);
    let installer = hook_installer(&st);
    Ok(vec![
        installer.status(pet_core::agent::AgentKind::Codex),
        installer.status(pet_core::agent::AgentKind::ClaudeCode),
    ]
    .into_iter()
    .flatten()
    .collect())
}

fn parse_agent(name: &str) -> Result<pet_core::agent::AgentKind, String> {
    match name {
        "codex" => Ok(pet_core::agent::AgentKind::Codex),
        "claude-code" | "claude" => Ok(pet_core::agent::AgentKind::ClaudeCode),
        other => Err(format!("unknown agent '{other}'")),
    }
}

fn hook_installer(st: &AppState) -> pet_core::agent::HookInstaller {
    let home = dirs::home_dir().unwrap_or_else(|| st.paths.config_dir.clone());
    pet_core::agent::HookInstaller::new(home, st.paths.config_dir.clone(), st.config().agent.port)
}

#[tauri::command]
pub fn install_hooks<R: Runtime>(
    app: AppHandle<R>,
    agent: String,
) -> Result<pet_core::agent::HookReport, String> {
    let st = app_state(&app);
    let kind = parse_agent(&agent)?;
    hook_installer(&st).install(kind).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn uninstall_hooks<R: Runtime>(
    app: AppHandle<R>,
    agent: String,
) -> Result<pet_core::agent::HookReport, String> {
    let st = app_state(&app);
    let kind = parse_agent(&agent)?;
    hook_installer(&st)
        .uninstall(kind)
        .map_err(|e| e.to_string())
}

// --- misc -------------------------------------------------------------------

#[tauri::command]
pub fn open_chat<R: Runtime>(app: AppHandle<R>) {
    tracing::debug!("open_chat requested");
    crate::window::show_chat(&app);
}

#[tauri::command]
pub fn hide_chat<R: Runtime>(app: AppHandle<R>) {
    if let Some(win) = app.get_webview_window("chat") {
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn open_data_dir<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = app_state(&app).paths.config_dir.clone();
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsPath {
    pub path: String,
}

#[tauri::command]
pub fn export_logs<R: Runtime>(app: AppHandle<R>) -> LogsPath {
    let dir = app_state(&app).paths.logs_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    LogsPath {
        path: dir.to_string_lossy().to_string(),
    }
}

// --- pet renderer callbacks -------------------------------------------------

/// Base64 data URL for a pet spritesheet.
///
/// The pet window normally loads the atlas through the Tauri asset protocol;
/// this is the fallback for environments where that protocol is unavailable.
#[tauri::command]
pub fn pet_spritesheet_data_url<R: Runtime>(app: AppHandle<R>, id: String) -> Result<String, String> {
    use base64::Engine;
    let entry = app_state(&app)
        .library
        .get(&id)
        .ok_or_else(|| format!("pet '{}' was not found", id))?;
    let bytes = std::fs::read(&entry.spritesheet).map_err(|e| e.to_string())?;
    let mime = match entry
        .spritesheet
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "image/webp",
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// The pet renderer finished loading and is listening for state events.
#[tauri::command]
pub fn pet_ready<R: Runtime>(app: AppHandle<R>) {
    crate::window::pet_window::greet(&app);
}

/// Report the sprite currently drawn so the hit test can use its alpha mask.
#[tauri::command]
pub fn pet_sprite_index<R: Runtime>(app: AppHandle<R>, index: u32) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Some(runtime) = state.pet_runtime() {
            let previous = runtime
                .hit
                .sprite
                .swap(index, std::sync::atomic::Ordering::Relaxed);
            if previous != index {
                tracing::debug!(sprite = index, "pet frame");
            }
        }
    }
}

/// The frontend finished a one-shot animation; fall back to the base state.
#[tauri::command]
pub fn pet_animation_finished<R: Runtime>(app: AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Some(runtime) = state.pet_runtime() {
            let transition = runtime.engine.lock().on_one_shot_finished();
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
    }
}

#[tauri::command]
pub fn pet_drag_started<R: Runtime>(app: AppHandle<R>) {
    crate::window::pet_window::set_dragging(&app, true);
}

#[tauri::command]
pub fn pet_drag_ended<R: Runtime>(app: AppHandle<R>) {
    crate::window::pet_window::set_dragging(&app, false);
}
