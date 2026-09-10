//! Shared application state, owned by Tauri and injected into commands.

use std::sync::Arc;

use bytepet_core::config::{AppConfig, AppPaths};
use bytepet_core::memory::MemoryStore;
use bytepet_core::persona::PersonaStore;
use bytepet_core::pet::PetLibrary;
use bytepet_core::secrets::{FileSecretStore, KeyringStore, SecretStore};
use parking_lot::RwLock;
use tauri::{AppHandle, Manager, Runtime};

use crate::chat::ChatManager;
use crate::tts::Tts;

pub struct AppState {
    pub paths: AppPaths,
    pub config: RwLock<AppConfig>,
    pub library: PetLibrary,
    pub personas: PersonaStore,
    pub secrets: Arc<dyn SecretStore>,
    /// True when secrets are in the OS keychain rather than the file fallback.
    pub keyring_available: bool,
    pub memory: Arc<MemoryStore>,
    pub chat: ChatManager,
    pub tts: Tts,
    /// Runtime for the currently displayed pet (engine + hit test + walk).
    pub pet: RwLock<Option<Arc<crate::window::pet_window::PetRuntime>>>,
    /// Background loops (hit test + walk) start only once per process.
    pub loops_started: std::sync::atomic::AtomicBool,
    /// Handle to the loopback agent status server, if running.
    pub agent_server: RwLock<Option<bytepet_core::agent::ServerHandle>>,
}

impl AppState {
    pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<Self> {
        // `BYTEPET_CONFIG_DIR` enables a portable install and hermetic tests.
        let config_dir = match std::env::var_os("BYTEPET_CONFIG_DIR") {
            Some(dir) => std::path::PathBuf::from(dir),
            None => app.path().app_config_dir()?,
        };
        Self::initialize_with_dir(config_dir)
    }

    /// Initialize with an explicit data directory (tests, portable mode).
    pub fn initialize_with_dir(config_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        let paths = AppPaths::resolve(config_dir);
        paths.ensure()?;

        let config = AppConfig::load(&paths.config_file).unwrap_or_default();
        let library = PetLibrary::discover(paths.pets_dir.clone());
        let personas = PersonaStore::new(paths.personas_dir.clone());
        personas.ensure()?;

        // First run: detect locally installed agent CLIs so the app is usable
        // without any API key, and pick a default persona.
        let mut config = config;
        let seeded = seed_default_providers(&mut config);
        if config.active_persona.is_none() {
            config.active_persona = personas
                .list()
                .ok()
                .and_then(|p| p.first().map(|p| p.id.clone()));
        }
        if seeded || !paths.config_file.is_file() {
            let _ = config.save(&paths.config_file);
        }

        let keyring = KeyringStore::new("com.bytepet.desktop");
        let (secrets, keyring_available): (Arc<dyn SecretStore>, bool) =
            match keyring.get("__probe__") {
                Ok(_) => (Arc::new(keyring), true),
                Err(err) => {
                    tracing::warn!(%err, "OS keychain unavailable, falling back to file secrets");
                    (
                        Arc::new(FileSecretStore::new(paths.config_dir.join("secrets.json"))?),
                        false,
                    )
                }
            };

        let memory = Arc::new(MemoryStore::open(&paths.db_path)?);

        Ok(Self {
            paths,
            config: RwLock::new(config),
            library,
            personas,
            secrets,
            keyring_available,
            memory,
            chat: ChatManager::new(),
            tts: Tts::new(),
            pet: RwLock::new(None),
            loops_started: std::sync::atomic::AtomicBool::new(false),
            agent_server: RwLock::new(None),
        })
    }

    /// Seed the bundled default pet exactly once, when the user has no pets.
    ///
    /// Returns the id of the pet that was installed, if seeding happened.
    pub fn ensure_default_pet(&self) -> anyhow::Result<Option<String>> {
        if self.config().default_pet_seeded {
            return Ok(None);
        }
        let installed =
            bytepet_core::pet::default_pet::seed_if_empty(&self.library)?.map(|entry| entry.id);
        let id = installed.clone();
        self.update_config(|cfg| {
            cfg.default_pet_seeded = true;
            if let Some(id) = &id {
                cfg.active_pet = Some(id.clone());
            }
        })?;
        Ok(installed)
    }

    /// The runtime for the pet currently on screen, if any.
    pub fn pet_runtime(&self) -> Option<Arc<crate::window::pet_window::PetRuntime>> {
        self.pet.read().clone()
    }

    pub fn set_pet_runtime(&self, runtime: Arc<crate::window::pet_window::PetRuntime>) {
        *self.pet.write() = Some(runtime);
    }

    pub fn config(&self) -> AppConfig {
        self.config.read().clone()
    }

    /// Update the config under the write lock and persist it.
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) -> anyhow::Result<AppConfig> {
        let snapshot = {
            let mut guard = self.config.write();
            f(&mut guard);
            guard.clone()
        };
        snapshot.save(&self.paths.config_file)?;
        Ok(snapshot)
    }

    pub fn agent_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.config().agent.port)
    }

    /// Persona that is currently active (falls back to the first one).
    pub fn active_persona(&self) -> Option<bytepet_core::persona::Persona> {
        let config = self.config();
        config
            .active_persona
            .as_ref()
            .and_then(|id| self.personas.get(id).ok().flatten())
            .or_else(|| self.personas.list().ok().and_then(|p| p.into_iter().next()))
    }

    /// Speak a reply if TTS is enabled for the active persona.
    pub fn speak_reply<R: Runtime>(&self, app: &AppHandle<R>, text: &str) {
        let config = self.config();
        if !config.chat.speak_replies && !config.tts.enabled {
            return;
        }
        let persona = self.active_persona();
        let (voice, rate) = match &persona {
            Some(p) if p.tts.enabled || config.tts.enabled => (
                p.tts.voice.clone().or_else(|| config.tts.voice.clone()),
                if p.tts.rate > 0.0 {
                    p.tts.rate
                } else {
                    config.tts.rate
                },
            ),
            Some(_) => return,
            None => (config.tts.voice.clone(), config.tts.rate),
        };
        self.tts
            .speak(app, text, voice, rate, config.tts.max_chars.max(50));
    }
}

/// Convenience accessor used by commands.
pub fn state<'a, R: Runtime>(app: &'a AppHandle<R>) -> tauri::State<'a, AppState> {
    app.state::<AppState>()
}

/// Register local `codex` / `claude` CLIs as ready-to-use providers.
///
/// Returns true when anything was added. Only runs when the user has not
/// configured a provider yet, so it never overrides real configuration.
fn seed_default_providers(config: &mut AppConfig) -> bool {
    use bytepet_core::llm::{ProviderConfig, ProviderKind};
    if !config.providers.is_empty() {
        return false;
    }
    let mut added = false;
    if which("codex").is_some() {
        // Empty model: let `codex exec` use whatever ~/.codex/config.toml selects.
        let provider = ProviderConfig::new("codex", ProviderKind::CodexCli, "");
        config.providers.insert(provider.id.clone(), provider);
        config.default_provider = Some("codex".to_string());
        added = true;
    }
    if which("claude").is_some() {
        let provider = ProviderConfig::new("claude-code", ProviderKind::ClaudeCli, "sonnet");
        config.providers.insert(provider.id.clone(), provider);
        if config.default_provider.is_none() {
            config.default_provider = Some("claude-code".to_string());
        }
        added = true;
    }
    added
}

/// Minimal `which`: look for an executable on PATH, including Windows shims.
fn which(binary: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    #[cfg(windows)]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|ext| ext.to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect();
    #[cfg(not(windows))]
    let exts: Vec<String> = Vec::new();
    bytepet_core::llm::providers::find_in_dirs(binary, &dirs, &exts)
}
