//! Application configuration: paths, typed config, atomic persistence.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Current config schema version. Bump when the shape changes incompatibly.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClickThroughMode {
    /// Per-pixel alpha hit testing (default).
    #[default]
    Auto,
    /// The whole window rectangle is interactive.
    Rect,
    /// Fully click-through; interact through the tray or a hotkey.
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PetWindowConfig {
    pub scale: f32,
    pub opacity: f32,
    pub always_on_top: bool,
    pub click_through: ClickThroughMode,
    pub start_position: Option<WindowPosition>,
    pub auto_walk: AutoWalkConfig,
}

impl Default for PetWindowConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
            opacity: 1.0,
            always_on_top: true,
            click_through: ClickThroughMode::Auto,
            start_position: None,
            auto_walk: AutoWalkConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoWalkConfig {
    pub enabled: bool,
    /// Logical pixels per second.
    pub speed_px_s: f32,
    /// How long to rest between walks, in seconds.
    pub pause_seconds: f32,
    /// Distance from the screen edge the pet keeps, in logical pixels.
    pub edge_margin: f32,
    /// Ignore auto-walk while the user interacted recently (seconds).
    pub user_grace_seconds: f32,
}

impl Default for AutoWalkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            speed_px_s: 60.0,
            pause_seconds: 6.0,
            edge_margin: 8.0,
            user_grace_seconds: 3.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPosition {
    pub x: f64,
    pub y: f64,
    pub monitor: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatConfig {
    pub send_on_enter: bool,
    pub show_reasoning: bool,
    pub speak_replies: bool,
    pub max_context_turns: u32,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            send_on_enter: true,
            show_reasoning: false,
            speak_replies: false,
            max_context_turns: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentConfig {
    pub enabled: bool,
    pub port: u16,
    /// Install hooks for these agents on startup if they are present.
    pub auto_install_hooks: bool,
    pub default_ttl_seconds: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 17872,
            auto_install_hooks: false,
            default_ttl_seconds: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TtsConfig {
    pub enabled: bool,
    pub voice: Option<String>,
    pub rate: f32,
    pub max_chars: usize,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            voice: None,
            rate: 1.0,
            max_chars: 400,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiConfig {
    /// `zh-CN` or `en`.
    pub language: String,
    /// `dark`, `light` or `system`.
    pub theme: String,
    pub launch_at_login: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            theme: "system".to_string(),
            launch_at_login: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub active_pet: Option<String>,
    pub active_persona: Option<String>,
    pub default_provider: Option<String>,
    pub pet: PetWindowConfig,
    pub chat: ChatConfig,
    pub agent: AgentConfig,
    pub tts: TtsConfig,
    pub ui: UiConfig,
    /// Non-secret provider configuration, keyed by provider id.
    pub providers: BTreeMap<String, crate::llm::ProviderConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            active_pet: None,
            active_persona: None,
            default_provider: None,
            pet: PetWindowConfig::default(),
            chat: ChatConfig::default(),
            agent: AgentConfig::default(),
            tts: TtsConfig::default(),
            ui: UiConfig::default(),
            providers: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let mut cfg: AppConfig = serde_json::from_str(&text).map_err(|e| {
            Error::config(format!("cannot parse {}: {e}", path.display()))
        })?;
        cfg.migrate()?;
        Ok(cfg)
    }

    /// Save atomically: write a temp file in the same directory then rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        if self.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(Error::config(format!(
                "config schema {} is newer than this build supports ({CONFIG_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        // Future migrations go here, e.g. if self.schema_version < 2 { ... }
        self.schema_version = CONFIG_SCHEMA_VERSION;
        Ok(())
    }
}

/// Resolved application directories. Everything the app owns lives here, except
/// the read-only Codex/UniPet libraries.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub pets_dir: PathBuf,
    pub personas_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub db_path: PathBuf,
    pub config_file: PathBuf,
}

impl AppPaths {
    pub fn resolve(config_dir: PathBuf) -> Self {
        Self {
            pets_dir: config_dir.join("pets"),
            personas_dir: config_dir.join("personas"),
            logs_dir: config_dir.join("logs"),
            db_path: config_dir.join("pet.db"),
            config_file: config_dir.join("config.json"),
            config_dir,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.pets_dir,
            &self.personas_dir,
            &self.logs_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_is_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let cfg = AppConfig {
            active_pet: Some("zip".into()),
            pet: PetWindowConfig {
                scale: 1.5,
                ..PetWindowConfig::default()
            },
            ui: UiConfig {
                language: "en".into(),
                ..UiConfig::default()
            },
            ..AppConfig::default()
        };
        cfg.save(&path).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let back = AppConfig::load(&path).unwrap();
        assert_eq!(back.active_pet.as_deref(), Some("zip"));
        assert_eq!(back.pet.scale, 1.5);
        assert_eq!(back.ui.language, "en");
    }

    #[test]
    fn missing_file_yields_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = AppConfig::load(&tmp.path().join("nope.json")).unwrap();
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(cfg.agent.enabled);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, r#"{"activePet":"zip"}"#).unwrap();
        let cfg = AppConfig::load(&path).unwrap();
        assert_eq!(cfg.active_pet.as_deref(), Some("zip"));
        assert_eq!(cfg.pet.scale, 1.0);
    }

    #[test]
    fn future_schema_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.json");
        std::fs::write(&path, r#"{"schemaVersion":999}"#).unwrap();
        assert!(AppConfig::load(&path).is_err());
    }

    #[test]
    fn resolves_paths_under_config_dir() {
        let paths = AppPaths::resolve(PathBuf::from("/tmp/petcfg"));
        assert!(paths.pets_dir.starts_with("/tmp/petcfg"));
        assert_eq!(paths.db_path, PathBuf::from("/tmp/petcfg/pet.db"));
    }
}
