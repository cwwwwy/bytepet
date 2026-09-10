//! Application configuration and paths.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DeepSeekConfig {
    /// DeepSeek exposes an OpenAI-compatible endpoint at `/v1/chat/completions`.
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub timeout_seconds: u64,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Ask DeepSeek to skip reasoning for short greetings.
    pub thinking_disabled: bool,
}

impl Default for DeepSeekConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-v4-flash".to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            timeout_seconds: 20,
            max_tokens: 80,
            temperature: 0.9,
            thinking_disabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GreetingConfig {
    pub enabled: bool,
    pub idle_minutes: u32,
    pub cooldown_minutes: u32,
    pub max_chars: usize,
}

impl Default for GreetingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_minutes: 30,
            cooldown_minutes: 120,
            max_chars: 40,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub recent_events: usize,
    pub retention_days: u32,
    pub fact_limit: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            recent_events: 5,
            retention_days: 90,
            fact_limit: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PetWindowConfig {
    pub scale: f32,
    pub opacity: f32,
    pub always_on_top: bool,
    pub click_through: bool,
    pub auto_walk: AutoWalkConfig,
    pub start_position: Option<WindowPosition>,
}

impl Default for PetWindowConfig {
    fn default() -> Self {
        Self {
            scale: 1.0,
            opacity: 1.0,
            always_on_top: true,
            click_through: true,
            auto_walk: AutoWalkConfig::default(),
            start_position: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoWalkConfig {
    pub enabled: bool,
    /// Minutes between short activity reminders.
    pub interval_minutes: u32,
    /// How long each short walk lasts.
    pub walk_seconds: f32,
    /// Logical pixels per second.
    pub speed_px_s: f32,
    /// Total wandering range in logical pixels.
    pub range_px: f32,
    pub user_grace_seconds: f32,
}

impl Default for AutoWalkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 45,
            walk_seconds: 8.0,
            speed_px_s: 18.0,
            range_px: 120.0,
            user_grace_seconds: 30.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowPosition {
    pub x: f32,
    pub y: f32,
}

/// The local state protocol hooks use to drive the pet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct StateServerConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for StateServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: crate::state_server::DEFAULT_PORT,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub active_pet: Option<String>,
    pub active_persona: Option<String>,
    pub first_run: bool,
    pub default_pet_seeded: bool,
    pub window: PetWindowConfig,
    pub deepseek: DeepSeekConfig,
    pub greeting: GreetingConfig,
    pub memory: MemoryConfig,
    pub state_server: StateServerConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            active_pet: None,
            active_persona: None,
            first_run: true,
            default_pet_seeded: false,
            window: PetWindowConfig::default(),
            deepseek: DeepSeekConfig::default(),
            greeting: GreetingConfig::default(),
            memory: MemoryConfig::default(),
            state_server: StateServerConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let mut config: AppConfig = serde_json::from_str(&text)
            .map_err(|error| Error::config(format!("cannot parse {}: {error}", path.display())))?;
        if config.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(Error::config(format!(
                "config schema {} is newer than this build supports ({CONFIG_SCHEMA_VERSION})",
                config.schema_version
            )));
        }
        config.schema_version = CONFIG_SCHEMA_VERSION;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub pets_dir: PathBuf,
    pub personas_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub config_file: PathBuf,
    pub memory_file: PathBuf,
}

impl AppPaths {
    /// `BYTEPET_HOME` overrides the platform config directory, which keeps
    /// portable installs and smoke tests out of the user's real data.
    pub fn default_dir() -> PathBuf {
        if let Some(dir) = std::env::var_os("BYTEPET_HOME") {
            return PathBuf::from(dir);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("BytePet")
    }

    pub fn resolve(config_dir: PathBuf) -> Self {
        Self {
            pets_dir: config_dir.join("pets"),
            personas_dir: config_dir.join("personas"),
            logs_dir: config_dir.join("logs"),
            config_file: config_dir.join("config.json"),
            memory_file: config_dir.join("memory.json"),
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
            if !dir.is_dir() {
                std::fs::create_dir_all(dir)?;
            }
        }
        Ok(())
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::resolve(Self::default_dir())
    }
}
