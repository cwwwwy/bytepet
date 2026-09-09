//! Hook installation for Codex and Claude Code.
//!
//! Both installers are non-destructive: they back up the target config, only
//! touch their own entries, and can restore the original file byte-for-byte.
//! Codex's `notify` is a single argv array, so when it is already in use we
//! generate a chaining wrapper script that forwards the payload to this app and
//! then calls the previously configured command.
//!
//! # Codex
//!
//! `~/.codex/config.toml`:
//!
//! ```toml
//! notify = ["/path/to/codex-notify.sh"]
//! ```
//!
//! Codex invokes `notify` with a single JSON argument, for example
//! `{"type":"agent-turn-complete","thread-id":"…","turn-id":"…","cwd":"…",
//! "input-messages":[…],"last-assistant-message":"…"}`. The generated wrapper
//! accepts the payload either on stdin or as its last argument and always
//! exits 0, even when the pet app is not running.
//!
//! # Claude Code
//!
//! Assumed schema (`~/.claude/settings.json`, documented at
//! <https://code.claude.com/docs/en/hooks>):
//!
//! ```json
//! {
//!   "hooks": {
//!     "Stop": [
//!       {
//!         "matcher": "*",
//!         "hooks": [{ "type": "command", "command": "/path/to/claude-hook.sh" }]
//!       }
//!     ]
//!   }
//! }
//! ```
//!
//! Events are `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
//! `Notification`, `Stop` and `SubagentStop`. Older/community configs
//! sometimes store plain handler objects (`{"type":"command","command":"…"}`)
//! directly in the event array; that shape is detected and supported too.
//! Hook payloads arrive as JSON on stdin with a `hook_event_name` field, which
//! the wrapper maps to a bytepet state with the same table as
//! [`AgentEvent::state_for_lifecycle`](crate::agent::AgentEvent::state_for_lifecycle).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Claude Code hook events pet installs, together with the first CLI version
/// that supported them. Events newer than the installed CLI are skipped.
const CLAUDE_EVENTS: &[(&str, (u64, u64, u64))] = &[
    ("SessionStart", (1, 0, 62)),
    ("UserPromptSubmit", (1, 0, 54)),
    ("PreToolUse", (1, 0, 38)),
    ("PostToolUse", (1, 0, 38)),
    ("Notification", (1, 0, 38)),
    ("Stop", (1, 0, 38)),
    ("SubagentStop", (1, 0, 38)),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Codex => "Codex",
            AgentKind::ClaudeCode => "Claude Code",
        }
    }

    /// Stable key used in `state.json`.
    pub fn key(self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::ClaudeCode => "claude-code",
        }
    }

    pub fn config_path(self, home: &std::path::Path) -> PathBuf {
        match self {
            AgentKind::Codex => home.join(".codex").join("config.toml"),
            AgentKind::ClaudeCode => home.join(".claude").join("settings.json"),
        }
    }

    /// Wrapper file name installed under `<data_dir>/hooks/`.
    fn wrapper_name(self) -> &'static str {
        match self {
            AgentKind::Codex => {
                if cfg!(windows) {
                    "codex-notify.cmd"
                } else {
                    "codex-notify.sh"
                }
            }
            AgentKind::ClaudeCode => {
                if cfg!(windows) {
                    "claude-hook.cmd"
                } else {
                    "claude-hook.sh"
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookReport {
    pub kind: AgentKind,
    pub installed: bool,
    pub config_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub wrapper_path: Option<PathBuf>,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub kind: AgentKind,
    pub config_path: PathBuf,
    pub exists: bool,
    pub installed: bool,
    pub detail: String,
}

/// Machine-readable record of one installation, stored in
/// `<data_dir>/hooks/state.json` so uninstall can be exact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookRecord {
    kind: AgentKind,
    config_path: PathBuf,
    #[serde(default)]
    backup_path: Option<PathBuf>,
    wrapper_path: PathBuf,
    /// Codex only: the `notify` argv that was configured before pet.
    #[serde(default)]
    original_notify: Option<Vec<String>>,
    /// Claude Code only: events pet added handlers to.
    #[serde(default)]
    events: Vec<String>,
    /// Fingerprint of the config as written by `install`. When the file still
    /// matches, uninstall restores the pristine backup byte-for-byte.
    #[serde(default)]
    installed_fingerprint: Option<String>,
    /// False when pet created the config file.
    #[serde(default)]
    config_existed: bool,
    installed_at: String,
    port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HookStateFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    records: BTreeMap<String, HookRecord>,
}

/// Installs and removes agent hooks.
pub struct HookInstaller {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
}

impl HookInstaller {
    pub fn new(home: PathBuf, data_dir: PathBuf, port: u16) -> Self {
        Self {
            home,
            data_dir,
            port,
        }
    }

    /// Directory that holds the generated wrappers and `state.json`.
    pub fn hooks_dir(&self) -> PathBuf {
        self.data_dir.join("hooks")
    }

    /// Path of the wrapper this platform runs.
    pub fn wrapper_path(&self, kind: AgentKind) -> PathBuf {
        self.hooks_dir().join(kind.wrapper_name())
    }

    pub fn install(&self, kind: AgentKind) -> Result<HookReport> {
        match kind {
            AgentKind::Codex => self.install_codex(),
            AgentKind::ClaudeCode => self.install_claude(),
        }
    }

    pub fn uninstall(&self, kind: AgentKind) -> Result<HookReport> {
        match kind {
            AgentKind::Codex => self.uninstall_codex(),
            AgentKind::ClaudeCode => self.uninstall_claude(),
        }
    }

    pub fn status(&self, kind: AgentKind) -> Result<HookStatus> {
        let config_path = kind.config_path(&self.home);
        let exists = config_path.is_file();
        let wrapper = self.wrapper_path(kind);
        let wrapper_str = wrapper.to_string_lossy().to_string();
        let text = std::fs::read_to_string(&config_path).unwrap_or_default();
        let installed = exists && text_references_path(&text, &wrapper_str);
        let detail = if !exists {
            format!(
                "no config at {}; install will create it",
                config_path.display()
            )
        } else if installed {
            match kind {
                AgentKind::Codex => format!("installed: notify -> {}", wrapper.display()),
                AgentKind::ClaudeCode => {
                    let count = claude_handler_count(&text, &wrapper_str);
                    format!("installed: {count} hook event(s) -> {}", wrapper.display())
                }
            }
        } else {
            "not installed".to_string()
        };
        Ok(HookStatus {
            kind,
            config_path,
            exists,
            installed,
            detail,
        })
    }

    // ---------------------------------------------------------------- Codex --

    fn install_codex(&self) -> Result<HookReport> {
        let kind = AgentKind::Codex;
        let config_path = kind.config_path(&self.home);
        let wrapper = self.wrapper_path(kind);
        let wrapper_str = wrapper.to_string_lossy().to_string();
        let original = read_text(&config_path)?;
        let original_notify = codex_notify(original.as_deref())?;
        let record = self.load_record(kind);
        let already = original_notify
            .as_ref()
            .is_some_and(|argv| argv.len() == 1 && argv[0] == wrapper_str);

        let mut messages = Vec::new();

        // The chain baked into the wrapper: whatever notify pointed at before
        // pet, recovered from state.json when the config already points at us.
        let chain: Vec<String> = if already {
            match &record {
                Some(record) => record.original_notify.clone().unwrap_or_default(),
                None => {
                    messages.push(
                        "state.json was missing; the previous notify command could not be recovered"
                            .to_string(),
                    );
                    Vec::new()
                }
            }
        } else {
            original_notify.clone().unwrap_or_default()
        };
        self.write_wrappers(kind, &chain)?;

        if already {
            let backup = backup_path(&config_path);
            if record.is_none() {
                // Repair a missing record so uninstall stays exact.
                let bytes = std::fs::read(&config_path).unwrap_or_default();
                self.save_record(HookRecord {
                    kind,
                    config_path: config_path.clone(),
                    backup_path: backup.is_file().then(|| backup.clone()),
                    wrapper_path: wrapper.clone(),
                    original_notify: None,
                    events: Vec::new(),
                    installed_fingerprint: Some(fingerprint(&bytes)),
                    config_existed: true,
                    installed_at: now_rfc3339(),
                    port: self.port,
                })?;
            }
            messages.push("Codex notify already points at the pet wrapper".to_string());
            return Ok(HookReport {
                kind,
                installed: true,
                config_path,
                backup_path: backup.is_file().then_some(backup),
                wrapper_path: Some(wrapper),
                messages,
            });
        }

        if !chain.is_empty() {
            messages.push(format!(
                "previous notify command preserved through the wrapper: {}",
                chain.join(" ")
            ));
        }

        let mut doc: toml_edit::DocumentMut = match original.as_deref() {
            Some(text) if !text.trim().is_empty() => text.parse().map_err(|err| {
                Error::Agent(format!("cannot parse {}: {err}", config_path.display()))
            })?,
            _ => toml_edit::DocumentMut::new(),
        };
        let mut array = toml_edit::Array::new();
        array.push(wrapper_str.clone());
        doc["notify"] = toml_edit::value(array);

        let backup = self.write_backup(&config_path, original.as_deref())?;
        let new_text = doc.to_string();
        write_atomic(&config_path, new_text.as_bytes())?;
        messages.push(format!("notify = [{}]", wrapper.display()));

        self.save_record(HookRecord {
            kind,
            config_path: config_path.clone(),
            backup_path: backup.clone(),
            wrapper_path: wrapper.clone(),
            original_notify,
            events: Vec::new(),
            installed_fingerprint: Some(fingerprint(new_text.as_bytes())),
            config_existed: original.is_some(),
            installed_at: now_rfc3339(),
            port: self.port,
        })?;

        Ok(HookReport {
            kind,
            installed: true,
            config_path,
            backup_path: backup,
            wrapper_path: Some(wrapper),
            messages,
        })
    }

    fn uninstall_codex(&self) -> Result<HookReport> {
        let kind = AgentKind::Codex;
        let config_path = kind.config_path(&self.home);
        let wrapper = self.wrapper_path(kind);
        let wrapper_str = wrapper.to_string_lossy().to_string();
        let record = self.load_record(kind);
        let current = std::fs::read(&config_path).ok();
        let installed = current
            .as_ref()
            .is_some_and(|bytes| text_references_path(&String::from_utf8_lossy(bytes), &wrapper_str));

        if !installed {
            self.remove_wrappers(kind);
            return Ok(HookReport {
                kind,
                installed: false,
                config_path,
                backup_path: None,
                wrapper_path: None,
                messages: vec!["not installed; nothing to do".to_string()],
            });
        }

        let mut messages = Vec::new();
        let unchanged = match (&record, &current) {
            (Some(record), Some(bytes)) => {
                record.installed_fingerprint.as_deref() == Some(fingerprint(bytes).as_str())
            }
            _ => false,
        };

        if unchanged {
            let record = record.as_ref().expect("record checked above");
            match &record.backup_path {
                Some(backup) if backup.is_file() => {
                    let bytes = std::fs::read(backup)?;
                    write_atomic(&config_path, &bytes)?;
                    messages.push(format!("restored {}", config_path.display()));
                }
                _ if !record.config_existed => {
                    let only_ours = current
                        .as_ref()
                        .and_then(|bytes| String::from_utf8_lossy(bytes).parse::<toml_edit::DocumentMut>().ok())
                        .is_some_and(|doc| doc.as_table().len() <= 1);
                    if only_ours {
                        std::fs::remove_file(&config_path)?;
                        messages.push("removed the config file created by pet".to_string());
                    } else {
                        strip_codex_notify(&config_path, current.as_deref(), record.original_notify.as_deref())?;
                    }
                }
                _ => {
                    strip_codex_notify(&config_path, current.as_deref(), record.original_notify.as_deref())?;
                }
            }
        } else {
            strip_codex_notify(
                &config_path,
                current.as_deref(),
                record.as_ref().and_then(|record| record.original_notify.clone()).as_deref(),
            )?;
            messages.push("config was edited after install; removed only the pet notify entry".to_string());
        }

        if let Some(backup) = record.as_ref().and_then(|record| record.backup_path.clone()) {
            let _ = std::fs::remove_file(backup);
        }
        self.remove_wrappers(kind);
        self.remove_record(kind)?;
        messages.push("Codex notify restored".to_string());

        Ok(HookReport {
            kind,
            installed: true,
            config_path,
            backup_path: None,
            wrapper_path: None,
            messages,
        })
    }

    // ---------------------------------------------------------- Claude Code --

    fn install_claude(&self) -> Result<HookReport> {
        let kind = AgentKind::ClaudeCode;
        let config_path = kind.config_path(&self.home);
        let wrapper = self.wrapper_path(kind);
        let wrapper_str = wrapper.to_string_lossy().to_string();
        let original = read_text(&config_path)?;
        let mut root: serde_json::Value = match original.as_deref() {
            Some(text) if !text.trim().is_empty() => serde_json::from_str(text).map_err(|err| {
                Error::Agent(format!("cannot parse {}: {err}", config_path.display()))
            })?,
            _ => serde_json::json!({}),
        };
        if !root.is_object() {
            return Err(Error::Agent(format!(
                "{} must contain a JSON object",
                config_path.display()
            )));
        }

        let record = self.load_record(kind);
        let already = original
            .as_deref()
            .is_some_and(|text| text_references_path(text, &wrapper_str));
        self.write_wrappers(kind, &[])?;

        if already {
            let mut messages = vec!["Claude Code hooks already installed".to_string()];
            let backup = backup_path(&config_path);
            if record.is_none() {
                let bytes = std::fs::read(&config_path).unwrap_or_default();
                self.save_record(HookRecord {
                    kind,
                    config_path: config_path.clone(),
                    backup_path: backup.is_file().then(|| backup.clone()),
                    wrapper_path: wrapper.clone(),
                    original_notify: None,
                    events: all_claude_events()
                        .iter()
                        .map(|event| (*event).to_string())
                        .collect(),
                    installed_fingerprint: Some(fingerprint(&bytes)),
                    config_existed: true,
                    installed_at: now_rfc3339(),
                    port: self.port,
                })?;
                messages.push("rebuilt a missing state.json record".to_string());
            }
            return Ok(HookReport {
                kind,
                installed: true,
                config_path,
                backup_path: backup.is_file().then_some(backup),
                wrapper_path: Some(wrapper),
                messages,
            });
        }

        let events = self.supported_claude_events();
        if events.is_empty() {
            return Err(Error::Agent(
                "the installed Claude Code version does not support any hook event pet needs"
                    .to_string(),
            ));
        }

        let hooks = root
            .as_object_mut()
            .expect("checked above")
            .entry("hooks".to_string())
            .or_insert_with(|| serde_json::json!({}));
        let Some(hooks) = hooks.as_object_mut() else {
            return Err(Error::Agent(format!(
                "`hooks` in {} must be a JSON object",
                config_path.display()
            )));
        };
        for event in &events {
            insert_claude_handler(hooks, event, &wrapper_str);
        }

        let mut new_text = serde_json::to_string_pretty(&root)?;
        if original.as_deref().is_none_or(|text| text.ends_with('\n')) {
            new_text.push('\n');
        }

        let backup = self.write_backup(&config_path, original.as_deref())?;
        write_atomic(&config_path, new_text.as_bytes())?;

        self.save_record(HookRecord {
            kind,
            config_path: config_path.clone(),
            backup_path: backup.clone(),
            wrapper_path: wrapper.clone(),
            original_notify: None,
            events: events.iter().map(|event| (*event).to_string()).collect(),
            installed_fingerprint: Some(fingerprint(new_text.as_bytes())),
            config_existed: original.is_some(),
            installed_at: now_rfc3339(),
            port: self.port,
        })?;

        let mut messages = vec![format!("installed {} hook event(s)", events.len())];
        if let Some(skipped) = self.skipped_claude_events(&events) {
            messages.push(skipped);
        }
        messages.push(format!("wrapper: {}", wrapper.display()));

        Ok(HookReport {
            kind,
            installed: true,
            config_path,
            backup_path: backup,
            wrapper_path: Some(wrapper),
            messages,
        })
    }

    fn uninstall_claude(&self) -> Result<HookReport> {
        let kind = AgentKind::ClaudeCode;
        let config_path = kind.config_path(&self.home);
        let wrapper = self.wrapper_path(kind);
        let wrapper_str = wrapper.to_string_lossy().to_string();
        let record = self.load_record(kind);
        let current = std::fs::read(&config_path).ok();
        let installed = current
            .as_ref()
            .is_some_and(|bytes| text_references_path(&String::from_utf8_lossy(bytes), &wrapper_str));

        if !installed {
            self.remove_wrappers(kind);
            return Ok(HookReport {
                kind,
                installed: false,
                config_path,
                backup_path: None,
                wrapper_path: None,
                messages: vec!["not installed; nothing to do".to_string()],
            });
        }

        let mut messages = Vec::new();
        let unchanged = match (&record, &current) {
            (Some(record), Some(bytes)) => {
                record.installed_fingerprint.as_deref() == Some(fingerprint(bytes).as_str())
            }
            _ => false,
        };

        if unchanged {
            let record = record.as_ref().expect("record checked above");
            match &record.backup_path {
                Some(backup) if backup.is_file() => {
                    let bytes = std::fs::read(backup)?;
                    write_atomic(&config_path, &bytes)?;
                    messages.push(format!("restored {}", config_path.display()));
                }
                _ if !record.config_existed => {
                    std::fs::remove_file(&config_path)?;
                    messages.push("removed the settings file created by pet".to_string());
                }
                _ => {
                    strip_claude_handlers(&config_path, current.as_deref(), &wrapper_str)?;
                }
            }
        } else {
            strip_claude_handlers(&config_path, current.as_deref(), &wrapper_str)?;
            messages.push(
                "settings were edited after install; removed only the pet hook entries".to_string(),
            );
        }

        if let Some(backup) = record.as_ref().and_then(|record| record.backup_path.clone()) {
            let _ = std::fs::remove_file(backup);
        }
        self.remove_wrappers(kind);
        self.remove_record(kind)?;
        messages.push("Claude Code hooks removed".to_string());

        Ok(HookReport {
            kind,
            installed: true,
            config_path,
            backup_path: None,
            wrapper_path: None,
            messages,
        })
    }

    /// Events supported by the installed Claude Code CLI. When the CLI cannot
    /// be probed (not installed, timeout), every known event is assumed to work.
    fn supported_claude_events(&self) -> Vec<&'static str> {
        let version = self.claude_version();
        match version {
            Some(version) => CLAUDE_EVENTS
                .iter()
                .filter(|(_, minimum)| version >= *minimum)
                .map(|(event, _)| *event)
                .collect(),
            None => CLAUDE_EVENTS.iter().map(|(event, _)| *event).collect(),
        }
    }

    fn skipped_claude_events(&self, installed: &[&'static str]) -> Option<String> {
        let skipped: Vec<&str> = CLAUDE_EVENTS
            .iter()
            .map(|(event, _)| *event)
            .filter(|event| !installed.contains(event))
            .collect();
        if skipped.is_empty() {
            None
        } else {
            Some(format!(
                "skipped events unsupported by this Claude Code version: {}",
                skipped.join(", ")
            ))
        }
    }

    fn claude_version(&self) -> Option<(u64, u64, u64)> {
        let binary = self.claude_binary()?;
        let output = probe_command(&binary, &["--version"], Duration::from_millis(1500))?;
        parse_semver(&output)
    }

    fn claude_binary(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) { "claude.exe" } else { "claude" };
        let local = self.home.join(".local").join("bin").join(name);
        if local.is_file() {
            return Some(local);
        }
        let path = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    // -------------------------------------------------------------- helpers --

    fn write_wrappers(&self, kind: AgentKind, chain: &[String]) -> Result<()> {
        let dir = self.hooks_dir();
        std::fs::create_dir_all(&dir)?;
        let (sh_name, cmd_name, ps1_name) = match kind {
            AgentKind::Codex => ("codex-notify.sh", "codex-notify.cmd", "codex-notify.ps1"),
            AgentKind::ClaudeCode => ("claude-hook.sh", "claude-hook.cmd", "claude-hook.ps1"),
        };
        let sh = match kind {
            AgentKind::Codex => codex_wrapper_script(self.port, chain),
            AgentKind::ClaudeCode => claude_wrapper_script(self.port),
        };
        let ps1 = match kind {
            AgentKind::Codex => codex_wrapper_ps1(self.port, chain),
            AgentKind::ClaudeCode => claude_wrapper_ps1(self.port),
        };
        let cmd = wrapper_cmd(ps1_name);
        write_script(&dir.join(sh_name), sh.as_bytes())?;
        write_script(&dir.join(cmd_name), cmd.as_bytes())?;
        write_script(&dir.join(ps1_name), ps1.as_bytes())?;
        Ok(())
    }

    fn remove_wrappers(&self, kind: AgentKind) {
        let dir = self.hooks_dir();
        let names = match kind {
            AgentKind::Codex => ["codex-notify.sh", "codex-notify.cmd", "codex-notify.ps1"],
            AgentKind::ClaudeCode => ["claude-hook.sh", "claude-hook.cmd", "claude-hook.ps1"],
        };
        for name in names {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }

    fn write_backup(&self, config_path: &Path, original: Option<&str>) -> Result<Option<PathBuf>> {
        let Some(text) = original else {
            return Ok(None);
        };
        let path = backup_path(config_path);
        if !path.is_file() {
            write_atomic(&path, text.as_bytes())?;
        }
        Ok(Some(path))
    }

    fn state_path(&self) -> PathBuf {
        self.hooks_dir().join("state.json")
    }

    fn load_state(&self) -> HookStateFile {
        std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn load_record(&self, kind: AgentKind) -> Option<HookRecord> {
        self.load_state().records.remove(kind.key())
    }

    fn save_record(&self, record: HookRecord) -> Result<()> {
        let mut state = self.load_state();
        state.version = 1;
        state.records.insert(record.kind.key().to_string(), record);
        std::fs::create_dir_all(self.hooks_dir())?;
        let text = serde_json::to_string_pretty(&state)?;
        write_atomic(&self.state_path(), text.as_bytes())
    }

    fn remove_record(&self, kind: AgentKind) -> Result<()> {
        let mut state = self.load_state();
        state.records.remove(kind.key());
        if state.records.is_empty() {
            let _ = std::fs::remove_file(self.state_path());
            Ok(())
        } else {
            let text = serde_json::to_string_pretty(&state)?;
            write_atomic(&self.state_path(), text.as_bytes())
        }
    }
}

// ------------------------------------------------------------------ config --

/// Every Claude Code hook event pet knows about, regardless of CLI version.
fn all_claude_events() -> Vec<&'static str> {
    CLAUDE_EVENTS.iter().map(|(event, _)| *event).collect()
}

/// Read the configured Codex `notify` argv. `Ok(None)` means the key is absent.
fn codex_notify(text: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(text) = text else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| Error::Agent(format!("cannot parse Codex config.toml: {err}")))?;
    let Some(item) = doc.get("notify") else {
        return Ok(None);
    };
    let Some(array) = item.as_array() else {
        return Err(Error::Agent(
            "Codex `notify` is not an array of strings; refusing to modify it".to_string(),
        ));
    };
    let mut argv = Vec::new();
    for value in array.iter() {
        let Some(value) = value.as_str() else {
            return Err(Error::Agent(
                "Codex `notify` contains a non-string entry; refusing to modify it".to_string(),
            ));
        };
        argv.push(value.to_string());
    }
    Ok(Some(argv))
}

/// Put the original `notify` value back (or drop the key when it was absent).
fn strip_codex_notify(
    config_path: &Path,
    current: Option<&[u8]>,
    original: Option<&[String]>,
) -> Result<()> {
    let Some(bytes) = current else {
        return Ok(());
    };
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|err| Error::Agent(format!("{} is not valid UTF-8: {err}", config_path.display())))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| Error::Agent(format!("cannot parse {}: {err}", config_path.display())))?;
    match original {
        Some(argv) => {
            let mut array = toml_edit::Array::new();
            for arg in argv {
                array.push(arg.as_str());
            }
            doc["notify"] = toml_edit::value(array);
        }
        None => {
            doc.as_table_mut().remove("notify");
        }
    }
    write_atomic(config_path, doc.to_string().as_bytes())
}

fn handler_object(command: &str) -> serde_json::Value {
    serde_json::json!({ "type": "command", "command": command })
}

/// Add our command to one Claude Code event, handling both the nested
/// (`{"matcher": …, "hooks": […]}`) and the legacy plain-handler shape.
fn insert_claude_handler(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    command: &str,
) {
    let entry = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = serde_json::Value::Array(Vec::new());
    }
    let array = entry.as_array_mut().expect("array checked above");

    if array.is_empty() {
        array.push(serde_json::json!({ "hooks": [handler_object(command)] }));
        return;
    }

    let has_groups = array.iter().any(|item| item.get("hooks").is_some());
    if has_groups {
        for item in array.iter_mut() {
            if let Some(handlers) = item.get_mut("hooks").and_then(|value| value.as_array_mut()) {
                let present = handlers
                    .iter()
                    .any(|handler| handler.get("command").and_then(|c| c.as_str()) == Some(command));
                if !present {
                    handlers.push(handler_object(command));
                }
                return;
            }
        }
        array.push(serde_json::json!({ "hooks": [handler_object(command)] }));
    } else {
        let present = array
            .iter()
            .any(|handler| handler.get("command").and_then(|c| c.as_str()) == Some(command));
        if !present {
            array.push(handler_object(command));
        }
    }
}

fn strip_claude_handlers(
    config_path: &Path,
    current: Option<&[u8]>,
    command: &str,
) -> Result<()> {
    let Some(bytes) = current else {
        return Ok(());
    };
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|err| Error::Agent(format!("{} is not valid UTF-8: {err}", config_path.display())))?;
    let mut root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| Error::Agent(format!("cannot parse {}: {err}", config_path.display())))?;
    remove_claude_handlers(&mut root, command);
    let mut new_text = serde_json::to_string_pretty(&root)?;
    if text.ends_with('\n') {
        new_text.push('\n');
    }
    write_atomic(config_path, new_text.as_bytes())
}

/// Remove every handler whose `command` is ours. Returns the number removed.
fn remove_claude_handlers(root: &mut serde_json::Value, command: &str) -> usize {
    let Some(hooks) = root.get_mut("hooks").and_then(|value| value.as_object_mut()) else {
        return 0;
    };
    let mut removed = 0;
    let events: Vec<String> = hooks.keys().cloned().collect();
    for event in events {
        let Some(array) = hooks.get_mut(&event).and_then(|value| value.as_array_mut()) else {
            continue;
        };
        for item in array.iter_mut() {
            if let Some(handlers) = item.get_mut("hooks").and_then(|value| value.as_array_mut()) {
                let before = handlers.len();
                handlers.retain(|handler| {
                    handler.get("command").and_then(|c| c.as_str()) != Some(command)
                });
                removed += before - handlers.len();
            }
        }
        // Drop matcher groups that no longer have any handler. These are
        // containers, so they are not counted as removed handlers.
        array.retain(|item| match item.get("hooks").and_then(|value| value.as_array()) {
            Some(handlers) => !handlers.is_empty(),
            None => true,
        });
        let before = array.len();
        array.retain(|item| item.get("command").and_then(|c| c.as_str()) != Some(command));
        removed += before - array.len();
        if array.is_empty() {
            hooks.remove(&event);
        }
    }
    removed
}

/// Does `text` reference `path`?
///
/// Config files escape backslashes (`C:\\dir\\file.cmd` in JSON/TOML), so a
/// raw substring search for the platform path fails on Windows. Accept the
/// verbatim form, the backslash-escaped form and the forward-slash form.
fn text_references_path(text: &str, path: &str) -> bool {
    if text.contains(path) {
        return true;
    }
    if path.contains('\\') && text.contains(&path.replace('\\', "\\\\")) {
        return true;
    }
    text.contains(&path.replace('\\', "/"))
}

fn claude_handler_count(text: &str, command: &str) -> usize {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return 0;
    };
    let Some(hooks) = root.get("hooks").and_then(|value| value.as_object()) else {
        return 0;
    };
    let mut count = 0;
    for array in hooks.values().filter_map(|value| value.as_array()) {
        for item in array {
            match item.get("hooks").and_then(|value| value.as_array()) {
                Some(handlers) => {
                    count += handlers
                        .iter()
                        .filter(|handler| {
                            handler.get("command").and_then(|c| c.as_str()) == Some(command)
                        })
                        .count();
                }
                None => {
                    if item.get("command").and_then(|c| c.as_str()) == Some(command) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

// -------------------------------------------------------------------- files --

fn backup_path(config_path: &Path) -> PathBuf {
    let mut name = config_path.as_os_str().to_os_string();
    name.push(".pet-backup");
    PathBuf::from(name)
}

fn read_text(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8(bytes).map_err(|err| {
            Error::Agent(format!("{} is not valid UTF-8: {err}", path.display()))
        })?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::Agent(format!("cannot read {}: {err}", path.display()))),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".pet-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    #[cfg(windows)]
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn write_script(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn fingerprint(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}:{}", hasher.finish(), bytes.len())
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Run a command with a hard timeout, returning its stdout.
fn probe_command(binary: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let binary = binary.to_path_buf();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    std::thread::spawn(move || {
        let output = std::process::Command::new(&binary).args(&args).output();
        let _ = tx.send(output);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).to_string())
        }
        _ => None,
    }
}

fn parse_semver(text: &str) -> Option<(u64, u64, u64)> {
    for token in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let mut parts = token.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(major), Ok(minor), Ok(patch)) =
            (major.parse::<u64>(), minor.parse::<u64>(), patch.parse::<u64>())
        else {
            continue;
        };
        return Some((major, minor, patch));
    }
    None
}

// ----------------------------------------------------------------- scripts --

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn wrapper_cmd(ps1_name: &str) -> String {
    format!(
        "@echo off\r\n\
         rem Generated by BytePet. Runs the PowerShell wrapper next to this file.\r\n\
         powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0{ps1_name}\" %*\r\n\
         exit /b 0\r\n"
    )
}

const SH_JSON_HELPERS: &str = r#"
pet_json_field() {
  # $1 = JSON text, $2 = key, $3 = max decoded chars (0 = unlimited).
  # Prints the raw (still JSON-escaped) string content, or nothing when absent.
  # awk instead of sed so escaped quotes survive and EOF does not drop output.
  printf '%s' "$1" | awk -v key="$2" -v maxlen="${3:-0}" '
    BEGIN { pattern = "\"" key "\"[[:space:]]*:[[:space:]]*\"" }
    {
      if (found) next
      idx = match($0, pattern)
      if (idx == 0) next
      rest = substr($0, idx + RLENGTH)
      out = ""; count = 0; i = 1
      while (i <= length(rest)) {
        c = substr(rest, i, 1)
        if (c == "\\") {
          esc = substr(rest, i, 2)
          if (maxlen > 0 && count >= maxlen) break
          out = out esc; count += 1; i += 2; continue
        }
        if (c == "\"") { print out; found = 1; exit }
        if (maxlen > 0 && count >= maxlen) break
        out = out c; count += 1; i += 1
      }
      print out; found = 1; exit
    }'
}

pet_post_state() {
  # $1 = state, $2 = message already JSON-escaped (may be empty).
  # Never fails and never blocks for long.
  pet_state="$1"
  pet_message="$2"
  if [ -z "$pet_state" ]; then return 0; fi
  pet_body="{\"source\":\"${PET_SOURCE}\",\"state\":\"${pet_state}\""
  if [ -n "$pet_message" ]; then
    pet_body="${pet_body},\"message\":\"${pet_message}\""
  fi
  pet_body="${pet_body}}"
  if command -v curl >/dev/null 2>&1; then
    curl -sS -m 2 -o /dev/null -X POST -H 'Content-Type: application/json' --data-binary "$pet_body" "$PET_STATE_URL" >/dev/null 2>&1 || true
  elif command -v wget >/dev/null 2>&1; then
    wget -q -T 2 -O /dev/null --header='Content-Type: application/json' --post-data="$pet_body" "$PET_STATE_URL" >/dev/null 2>&1 || true
  fi
  return 0
}
"#;

const CODEX_SH_BODY: &str = r#"
# Codex passes the notification JSON as the last argument; some builds pipe it
# on stdin, so accept both.
pet_payload=''
if [ ! -t 0 ]; then pet_payload="$(cat 2>/dev/null || true)"; fi
if [ -z "$pet_payload" ] && [ "$#" -gt 0 ]; then
  for pet_last in "$@"; do :; done
  pet_payload="$pet_last"
fi

pet_state=''
if [ -n "$pet_payload" ]; then
  pet_event="$(pet_json_field "$pet_payload" type)"
  pet_event="$(printf '%s' "$pet_event" | tr '[:upper:]' '[:lower:]')"
  case "$pet_event" in
    *session-start*|*session_start*|*sessionstart*) pet_state='waving' ;;
    *turn-complete*|*turn_completed*|*turn-ended*|*turn_end*|*complete*|*done*|*stop*|*session-end*|*session_end*) pet_state='review' ;;
    *turn-start*|*turn_start*|*turn-begin*|*started*|*start*) pet_state='running' ;;
    *prompt*|*submit*) pet_state='running' ;;
    *tool-use*|*tool_use*) pet_state='running' ;;
    *notification*|*notify*|*approval*|*permission*|*waiting*) pet_state='waiting' ;;
    *error*|*fail*) pet_state='failed' ;;
  esac
fi

if [ -n "$pet_state" ]; then
  pet_message="$(pet_json_field "$pet_payload" last-assistant-message 200)"
  [ -n "$pet_message" ] || pet_message="$(pet_json_field "$pet_payload" message 200)"
  pet_post_state "$pet_state" "$pet_message"
fi

# Chain to the notify command that was configured before pet.
if [ "${#PET_CHAIN[@]}" -gt 0 ]; then
  "${PET_CHAIN[@]}" "$@" >/dev/null 2>&1 || true
fi
exit 0
"#;

const CLAUDE_SH_BODY: &str = r#"
# Claude Code sends the hook payload as JSON on stdin.
pet_payload="$(cat 2>/dev/null || true)"

pet_state=''
pet_event="$(pet_json_field "$pet_payload" hook_event_name)"
case "$pet_event" in
  SessionStart) pet_state='waving' ;;
  UserPromptSubmit) pet_state='running' ;;
  PreToolUse|PostToolUse) pet_state='running' ;;
  Notification) pet_state='waiting' ;;
  Stop|SubagentStop|SessionEnd) pet_state='review' ;;
  *) pet_state='' ;;
esac

if [ -n "$pet_state" ]; then
  pet_message="$(pet_json_field "$pet_payload" message 200)"
  [ -n "$pet_message" ] || pet_message="$(pet_json_field "$pet_payload" last_assistant_message 200)"
  pet_post_state "$pet_state" "$pet_message"
fi
exit 0
"#;

fn codex_wrapper_script(port: u16, chain: &[String]) -> String {
    let mut out = String::new();
    out.push_str("#!/usr/bin/env bash\n");
    out.push_str("# Generated by BytePet. Do not edit: `bytepet hooks install codex` regenerates it.\n");
    out.push_str("# Codex `notify` wrapper: forwards the payload to the pet status server, then\n");
    out.push_str("# chains to the notify command that was configured before pet was installed.\n\n");
    out.push_str(&format!("PET_PORT={port}\n"));
    out.push_str("PET_SOURCE='codex'\n");
    out.push_str("PET_STATE_URL=\"http://127.0.0.1:${PET_PORT}/state\"\n\n");
    out.push_str("# Previously configured `notify` argv (empty when notify was unset).\n");
    out.push_str("PET_CHAIN=(\n");
    for arg in chain {
        out.push_str("  ");
        out.push_str(&sh_single_quote(arg));
        out.push('\n');
    }
    out.push_str(")\n");
    out.push_str(SH_JSON_HELPERS);
    out.push_str(CODEX_SH_BODY);
    out
}

fn claude_wrapper_script(port: u16) -> String {
    let mut out = String::new();
    out.push_str("#!/usr/bin/env bash\n");
    out.push_str("# Generated by BytePet. Do not edit: `bytepet hooks install claude-code` regenerates it.\n");
    out.push_str("# Claude Code hook wrapper: maps `hook_event_name` to a bytepet state and posts it.\n");
    out.push_str("# Always exits 0 and prints nothing, so it never blocks or steers Claude Code.\n\n");
    out.push_str(&format!("PET_PORT={port}\n"));
    out.push_str("PET_SOURCE='claude-code'\n");
    out.push_str("PET_STATE_URL=\"http://127.0.0.1:${PET_PORT}/state\"\n");
    out.push_str(SH_JSON_HELPERS);
    out.push_str(CLAUDE_SH_BODY);
    out
}

fn codex_wrapper_ps1(port: u16, chain: &[String]) -> String {
    let mut out = String::new();
    out.push_str("# Generated by BytePet. Do not edit: `bytepet hooks install codex` regenerates it.\n");
    out.push_str("$ErrorActionPreference = 'SilentlyContinue'\n");
    out.push_str(&format!("$PetPort = {port}\n"));
    out.push_str("$PetSource = 'codex'\n");
    out.push_str("$PetStateUrl = \"http://127.0.0.1:$PetPort/state\"\n");
    out.push_str("$PetChain = @(\n");
    for arg in chain {
        out.push_str("  ");
        out.push_str(&ps_single_quote(arg));
        out.push('\n');
    }
    out.push_str(")\n");
    out.push_str(PS1_HELPERS);
    out.push_str(
        r#"
$petPayload = ''
try { if ([Console]::IsInputRedirected) { $petPayload = [Console]::In.ReadToEnd() } } catch { }
if ([string]::IsNullOrWhiteSpace($petPayload) -and $args.Count -gt 0) { $petPayload = [string]$args[-1] }

$petState = ''
if (-not [string]::IsNullOrWhiteSpace($petPayload)) {
  $petType = (Pet-Field $petPayload 'type').ToLowerInvariant()
  switch -Wildcard ($petType) {
    '*session-start*' { $petState = 'waving' }
    '*turn-complete*' { $petState = 'review' }
    '*turn-ended*'    { $petState = 'review' }
    '*complete*'      { $petState = 'review' }
    '*turn-start*'    { $petState = 'running' }
    '*start*'         { $petState = 'running' }
    '*prompt*'        { $petState = 'running' }
    '*notification*'  { $petState = 'waiting' }
    '*notify*'        { $petState = 'waiting' }
    '*approval*'      { $petState = 'waiting' }
    '*fail*'          { $petState = 'failed' }
    '*error*'         { $petState = 'failed' }
  }
}
if (-not [string]::IsNullOrWhiteSpace($petState)) {
  $petMessage = Pet-Field $petPayload 'last-assistant-message'
  if ([string]::IsNullOrWhiteSpace($petMessage)) { $petMessage = Pet-Field $petPayload 'message' }
  if ($petMessage.Length -gt 200) { $petMessage = $petMessage.Substring(0, 200) }
  Pet-Post $petState $petMessage
}

if ($PetChain.Count -gt 0) {
  $petRest = @()
  if ($PetChain.Count -gt 1) { $petRest = $PetChain[1..($PetChain.Count - 1)] }
  try { & $PetChain[0] @petRest @args *> $null } catch { }
}
exit 0
"#,
    );
    out
}

fn claude_wrapper_ps1(port: u16) -> String {
    let mut out = String::new();
    out.push_str("# Generated by BytePet. Do not edit: `bytepet hooks install claude-code` regenerates it.\n");
    out.push_str("$ErrorActionPreference = 'SilentlyContinue'\n");
    out.push_str(&format!("$PetPort = {port}\n"));
    out.push_str("$PetSource = 'claude-code'\n");
    out.push_str("$PetStateUrl = \"http://127.0.0.1:$PetPort/state\"\n");
    out.push_str(PS1_HELPERS);
    out.push_str(
        r#"
$petPayload = ''
try { if ([Console]::IsInputRedirected) { $petPayload = [Console]::In.ReadToEnd() } } catch { }

$petState = ''
switch (Pet-Field $petPayload 'hook_event_name') {
  'SessionStart'     { $petState = 'waving' }
  'UserPromptSubmit' { $petState = 'running' }
  'PreToolUse'       { $petState = 'running' }
  'PostToolUse'      { $petState = 'running' }
  'Notification'     { $petState = 'waiting' }
  'Stop'             { $petState = 'review' }
  'SubagentStop'     { $petState = 'review' }
  'SessionEnd'       { $petState = 'review' }
}
if (-not [string]::IsNullOrWhiteSpace($petState)) {
  $petMessage = Pet-Field $petPayload 'message'
  if ([string]::IsNullOrWhiteSpace($petMessage)) { $petMessage = Pet-Field $petPayload 'last_assistant_message' }
  if ($petMessage.Length -gt 200) { $petMessage = $petMessage.Substring(0, 200) }
  Pet-Post $petState $petMessage
}
exit 0
"#,
    );
    out
}

const PS1_HELPERS: &str = r#"
function Pet-Field($json, $name) {
  if ([string]::IsNullOrWhiteSpace($json)) { return '' }
  try { $obj = $json | ConvertFrom-Json } catch { return '' }
  if ($null -eq $obj) { return '' }
  $prop = $obj.PSObject.Properties[$name]
  if ($null -eq $prop -or $null -eq $prop.Value) { return '' }
  return [string]$prop.Value
}

function Pet-Post($state, $message) {
  if ([string]::IsNullOrWhiteSpace($state)) { return }
  $body = @{ source = $PetSource; state = $state }
  if (-not [string]::IsNullOrWhiteSpace($message)) { $body['message'] = $message }
  try {
    Invoke-RestMethod -Uri $PetStateUrl -Method Post -ContentType 'application/json' `
      -Body ($body | ConvertTo-Json -Compress) -TimeoutSec 2 | Out-Null
  } catch { }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semver_from_cli_banner() {
        assert_eq!(parse_semver("2.1.251 (Claude Code)"), Some((2, 1, 251)));
        assert_eq!(parse_semver("v1.0.62"), Some((1, 0, 62)));
        assert_eq!(parse_semver("no version here"), None);
    }

    #[test]
    fn codex_wrapper_bakes_port_and_chain() {
        let script = codex_wrapper_script(17872, &["/bin/true".to_string(), "x".to_string()]);
        assert!(script.contains("PET_PORT=17872"));
        assert!(script.contains("http://127.0.0.1:${PET_PORT}/state"));
        assert!(script.contains("'/bin/true'"));
        assert!(script.contains("'x'"));
        assert!(script.contains("exit 0"));
    }

    #[test]
    fn codex_wrapper_escapes_single_quotes() {
        let script = codex_wrapper_script(1, &["/tmp/it's here".to_string()]);
        assert!(script.contains(r"'/tmp/it'\''s here'"));
    }

    #[test]
    fn claude_wrapper_maps_every_event() {
        let script = claude_wrapper_script(17872);
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "Stop",
            "SubagentStop",
        ] {
            assert!(script.contains(event), "missing {event}");
        }
    }

    #[test]
    fn insert_handles_nested_and_plain_shapes() {
        let mut hooks = serde_json::Map::new();
        hooks.insert(
            "Stop".to_string(),
            serde_json::json!([{ "matcher": "*", "hooks": [{ "type": "command", "command": "/other" }] }]),
        );
        insert_claude_handler(&mut hooks, "Stop", "/pet/claude-hook.sh");
        let stop = hooks.get("Stop").unwrap().as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"].as_array().unwrap().len(), 2);

        hooks.insert(
            "PreToolUse".to_string(),
            serde_json::json!([{ "type": "command", "command": "/legacy" }]),
        );
        insert_claude_handler(&mut hooks, "PreToolUse", "/pet/claude-hook.sh");
        let pre = hooks.get("PreToolUse").unwrap().as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[1]["command"], "/pet/claude-hook.sh");

        // Empty arrays use the canonical nested form.
        insert_claude_handler(&mut hooks, "Notification", "/pet/claude-hook.sh");
        let notification = hooks.get("Notification").unwrap().as_array().unwrap();
        assert!(notification[0].get("hooks").is_some());
    }

    #[test]
    fn removal_is_exact() {
        let mut root = serde_json::json!({
            "env": {"A": "1"},
            "hooks": {
                "Stop": [
                    {"matcher": "*", "hooks": [
                        {"type": "command", "command": "/other"},
                        {"type": "command", "command": "/pet/claude-hook.sh"}
                    ]}
                ],
                "Notification": [
                    {"hooks": [{"type": "command", "command": "/pet/claude-hook.sh"}]}
                ]
            }
        });
        let removed = remove_claude_handlers(&mut root, "/pet/claude-hook.sh");
        assert_eq!(removed, 2);
        assert!(root["hooks"].get("Notification").is_none());
        assert_eq!(root["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(root["env"]["A"], "1");
    }

    #[test]
    fn text_references_path_handles_windows_escaping() {
        let win = r"C:\Users\me\AppData\Roaming\bytepet\hooks\codex-notify.cmd";
        assert!(text_references_path(win, win));
        let escaped = r#""C:\\Users\\me\\AppData\\Roaming\\bytepet\\hooks\\codex-notify.cmd""#;
        assert!(
            text_references_path(escaped, win),
            "JSON/TOML-escaped paths must match"
        );
        assert!(text_references_path(
            "C:/Users/me/AppData/Roaming/bytepet/hooks/codex-notify.cmd",
            win
        ));
        assert!(!text_references_path("/other/path/hook.sh", win));
    }

    #[test]
    fn status_is_reported_for_missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let installer = HookInstaller::new(tmp.path().to_path_buf(), tmp.path().join("data"), 17872);
        let status = installer.status(AgentKind::Codex).unwrap();
        assert!(!status.exists);
        assert!(!status.installed);
        assert!(status.detail.contains("no config"));
    }
}
