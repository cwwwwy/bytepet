//! `pet` — command line companion for the desktop pet.
//!
//! Talks to the running app over its loopback status server (`POST /state`,
//! `GET /health`) with a tiny dependency-free HTTP client, and manages the pet
//! library, the Codex/Claude Code hooks and a `doctor` diagnosis.
//!
//! Exit codes: `0` success, `1` error, `2` the pet app is not running.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytepet_core::agent::hooks::{AgentKind, HookInstaller, HookReport};
use bytepet_core::agent::AgentEvent;
use bytepet_core::config::{AppConfig, AppPaths};
use bytepet_core::pet::library::{PetEntry, PetLibrary};
use clap::{Args, Parser, Subcommand, ValueEnum};

const DEFAULT_PORT: u16 = 17872;
const APP_IDENTIFIER: &str = "com.bytepet.desktop";

#[derive(Parser)]
#[command(
    name = "pet",
    version,
    about = "Desktop pet companion: report agent state, manage pets and hooks",
    disable_help_subcommand = true
)]
struct Cli {
    /// Port of the local pet status server.
    #[arg(long, global = true, default_value_t = DEFAULT_PORT, env = "BYTEPET_AGENT_PORT")]
    port: u16,

    /// Application data directory (defaults to the Tauri app config dir).
    #[arg(long, global = true, env = "BYTEPET_DATA_DIR", default_value_os_t = default_data_dir())]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn paths(&self) -> AppPaths {
        AppPaths::resolve(self.data_dir.clone())
    }

    fn library(&self) -> PetLibrary {
        PetLibrary::discover(self.paths().pets_dir)
    }

    fn hooks(&self) -> HookInstaller {
        HookInstaller::new(home_dir(), self.data_dir.clone(), self.port)
    }
}

#[derive(Subcommand)]
enum Command {
    /// Report an agent state to the running pet app.
    State(StateArgs),
    /// Clear the state reported by a source.
    Clear(ClearArgs),
    /// Manage the pet library.
    #[command(subcommand)]
    Pet(PetCommand),
    /// Install, remove or inspect agent hooks.
    #[command(subcommand)]
    Hooks(HooksCommand),
    /// Diagnose paths, library, server and hooks.
    Doctor,
    /// Explain where text-to-speech lives.
    Speak(SpeakArgs),
}

#[derive(Args)]
struct StateArgs {
    /// State: idle, running, waiting, failed, review, waving, jumping,
    /// running-left, running-right.
    state: String,
    /// Optional bubble message.
    message: Option<String>,
    /// Event source.
    #[arg(long, default_value = "cli")]
    source: String,
    /// Time to live, e.g. `500ms`, `5s`, `2m` (bare numbers are seconds).
    #[arg(long, value_parser = parse_duration_ms)]
    ttl: Option<u64>,
    /// Optional action hint carried by the event.
    #[arg(long)]
    action: Option<String>,
}

#[derive(Args)]
struct ClearArgs {
    /// Source whose override should be cleared.
    #[arg(long, default_value = "cli")]
    source: String,
}

#[derive(Args)]
struct SpeakArgs {
    /// Text the pet would speak (TTS is app-side).
    text: String,
}

#[derive(Subcommand)]
enum PetCommand {
    /// List pets found in every library root.
    List,
    /// Set the active pet.
    Use { id: String },
    /// Import a pet directory (or a `.zip` package).
    Import {
        path: PathBuf,
        /// Replace an existing pet with the same id.
        #[arg(long)]
        overwrite: bool,
    },
    /// Import a pet zip package.
    ImportZip {
        path: PathBuf,
        /// Replace an existing pet with the same id.
        #[arg(long)]
        overwrite: bool,
    },
    /// Export a pet as the Codex upload zip.
    Export { id: String, out: PathBuf },
    /// Validate a pet directory.
    Validate { path: PathBuf },
    /// Remove a pet from the local library.
    Remove { id: String },
}

#[derive(Subcommand)]
enum HooksCommand {
    /// Install the bytepet hooks.
    Install {
        #[arg(value_enum, default_value_t = HookSelection::All)]
        kind: HookSelection,
    },
    /// Remove the bytepet hooks and restore the previous config.
    Uninstall {
        #[arg(value_enum, default_value_t = HookSelection::All)]
        kind: HookSelection,
    },
    /// Show hook status.
    Status {
        #[arg(value_enum, default_value_t = HookSelection::All)]
        kind: HookSelection,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HookSelection {
    Codex,
    ClaudeCode,
    All,
}

impl HookSelection {
    fn kinds(self) -> Vec<AgentKind> {
        match self {
            HookSelection::Codex => vec![AgentKind::Codex],
            HookSelection::ClaudeCode => vec![AgentKind::ClaudeCode],
            HookSelection::All => vec![AgentKind::Codex, AgentKind::ClaudeCode],
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    match &cli.command {
        Command::State(args) => cmd_state(cli, args),
        Command::Clear(args) => cmd_clear(cli, args),
        Command::Pet(command) => cmd_pet(cli, command),
        Command::Hooks(command) => cmd_hooks(cli, command),
        Command::Doctor => cmd_doctor(cli),
        Command::Speak(args) => cmd_speak(args),
    }
}

// ------------------------------------------------------------------- state --

fn cmd_state(cli: &Cli, args: &StateArgs) -> Result<ExitCode> {
    let mut event = AgentEvent::new(args.source.clone(), args.state.clone());
    event.message = args.message.clone();
    event.action = args.action.clone();
    event.ttl_ms = args.ttl;
    event.validate().map_err(|err| anyhow::anyhow!("{err}"))?;

    let body = serde_json::to_string(&event)?;
    match http(cli.port, "POST", "/state", Some(&body)) {
        HttpOutcome::Status(202, _) => {
            let message = match event.message.as_deref() {
                Some(message) => format!("“{message}”"),
                None => String::new(),
            };
            let ttl = match event.ttl_ms {
                Some(ms) => format!("{ms}ms"),
                None => "default".to_string(),
            };
            println!(
                "sent: {} {} (source={}, ttl={})",
                event.state, message, event.source, ttl
            );
            Ok(ExitCode::SUCCESS)
        }
        HttpOutcome::Status(code, body) => {
            eprintln!("pet app rejected the event (HTTP {code}): {}", body.trim());
            Ok(ExitCode::from(1))
        }
        HttpOutcome::Refused => Ok(app_not_running(cli.port)),
        HttpOutcome::Unreachable(err) => {
            eprintln!("cannot reach the pet app on 127.0.0.1:{}: {err}", cli.port);
            Ok(ExitCode::from(2))
        }
    }
}

fn cmd_clear(cli: &Cli, args: &ClearArgs) -> Result<ExitCode> {
    // There is no dedicated clear endpoint; an idle event with a 1 ms TTL drops
    // the override immediately after it is applied.
    let mut event = AgentEvent::new(args.source.clone(), "idle");
    event.ttl_ms = Some(1);
    let body = serde_json::to_string(&event)?;
    match http(cli.port, "POST", "/state", Some(&body)) {
        HttpOutcome::Status(202, _) => {
            println!("cleared state for source '{}'", args.source);
            Ok(ExitCode::SUCCESS)
        }
        HttpOutcome::Status(code, body) => {
            eprintln!("pet app rejected the clear (HTTP {code}): {}", body.trim());
            Ok(ExitCode::from(1))
        }
        HttpOutcome::Refused => Ok(app_not_running(cli.port)),
        HttpOutcome::Unreachable(err) => {
            eprintln!("cannot reach the pet app on 127.0.0.1:{}: {err}", cli.port);
            Ok(ExitCode::from(2))
        }
    }
}

fn app_not_running(port: u16) -> ExitCode {
    eprintln!(
        "桌宠未运行 / pet app is not running (127.0.0.1:{port}).\n\
         Start the desktop app first, or pass --port <port> if it uses another port."
    );
    ExitCode::from(2)
}

fn cmd_speak(args: &SpeakArgs) -> Result<ExitCode> {
    println!("Text-to-speech is handled inside the desktop app; there is no TTS endpoint yet.");
    println!("text: {}", args.text);
    Ok(ExitCode::SUCCESS)
}

// --------------------------------------------------------------- pet library --

fn cmd_pet(cli: &Cli, command: &PetCommand) -> Result<ExitCode> {
    let library = cli.library();
    let paths = cli.paths();
    match command {
        PetCommand::List => {
            let pets = library.list();
            println!("{} pet(s) in {} root(s)", pets.len(), library.roots().len());
            for pet in &pets {
                println!(
                    "  {:<20} {:<24} {:>2} rows  {:<7} {}",
                    pet.id,
                    truncate(&pet.display_name, 24),
                    pet.frame.rows,
                    pet.root.label(),
                    pet.dir.display()
                );
            }
            if pets.is_empty() {
                println!("  (no pets found; `pet pet import <dir>` adds one)");
            }
            Ok(ExitCode::SUCCESS)
        }
        PetCommand::Use { id } => {
            let pet = library
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("pet '{id}' was not found in any library root"))?;
            paths.ensure()?;
            let mut config = AppConfig::load(&paths.config_file).unwrap_or_default();
            config.active_pet = Some(pet.id.clone());
            config.save(&paths.config_file)?;
            println!("active pet: {} ({})", pet.id, pet.display_name);
            Ok(ExitCode::SUCCESS)
        }
        PetCommand::Import { path, overwrite } => {
            let entry = import_any(&library, path, *overwrite)?;
            println!(
                "imported: {} ({}) -> {}",
                entry.id,
                entry.display_name,
                entry.dir.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        PetCommand::ImportZip { path, overwrite } => {
            let entry = library
                .import_zip(path, *overwrite)
                .with_context(|| format!("importing {}", path.display()))?;
            println!(
                "imported zip: {} ({}) -> {}",
                entry.id,
                entry.display_name,
                entry.dir.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        PetCommand::Export { id, out } => {
            library
                .export_zip(id, out)
                .with_context(|| format!("exporting '{id}'"))?;
            println!("exported: {id} -> {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        PetCommand::Validate { path } => {
            let report = PetLibrary::validate_dir(path);
            print_validation(&report);
            Ok(ExitCode::from(if report.ok { 0 } else { 1 }))
        }
        PetCommand::Remove { id } => {
            library
                .remove_local(id)
                .with_context(|| format!("removing '{id}'"))?;
            if let Ok(mut config) = AppConfig::load(&paths.config_file) {
                if config.active_pet.as_deref() == Some(id.as_str()) {
                    config.active_pet = None;
                    let _ = config.save(&paths.config_file);
                }
            }
            println!("removed: {id}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn import_any(library: &PetLibrary, path: &Path, overwrite: bool) -> Result<PetEntry> {
    if path.is_dir() {
        return library
            .import_dir(path, overwrite)
            .with_context(|| format!("importing {}", path.display()));
    }
    let is_zip = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
    if path.is_file() && is_zip {
        return library
            .import_zip(path, overwrite)
            .with_context(|| format!("importing {}", path.display()));
    }
    bail!(
        "{} is neither a pet directory nor a .zip package",
        path.display()
    )
}

fn print_validation(report: &bytepet_core::pet::library::ValidationReport) {
    println!(
        "{}: {}",
        if report.ok { "valid" } else { "invalid" },
        report.dir.display()
    );
    println!("  id           {}", report.id);
    println!("  display name {}", report.display_name);
    println!(
        "  frame        {}x{} cells of {}x{} px",
        report.frame.columns, report.frame.rows, report.frame.width, report.frame.height
    );
    if let Some((width, height)) = report.image_size {
        println!("  spritesheet  {width}x{height} px");
    }
    println!("  animations   {}", report.animations.len());
    for warning in &report.warnings {
        println!("  warning      {warning}");
    }
    for error in &report.errors {
        println!("  error        {error}");
    }
}

// -------------------------------------------------------------------- hooks --

fn cmd_hooks(cli: &Cli, command: &HooksCommand) -> Result<ExitCode> {
    let installer = cli.hooks();
    let (kinds, action) = match command {
        HooksCommand::Install { kind } => (kind.kinds(), "install"),
        HooksCommand::Uninstall { kind } => (kind.kinds(), "uninstall"),
        HooksCommand::Status { kind } => (kind.kinds(), "status"),
    };

    let mut failures = 0;
    for kind in kinds {
        let result: Result<String> = match action {
            "install" => installer
                .install(kind)
                .map(|report| format_report(&report))
                .map_err(|err| anyhow::anyhow!("{err}")),
            "uninstall" => installer
                .uninstall(kind)
                .map(|report| format_report(&report))
                .map_err(|err| anyhow::anyhow!("{err}")),
            _ => installer
                .status(kind)
                .map(|status| {
                    let state = if status.installed {
                        "installed"
                    } else if status.exists {
                        "not installed"
                    } else {
                        "no config"
                    };
                    let detail = if status.installed {
                        status.detail.clone()
                    } else {
                        status.config_path.display().to_string()
                    };
                    format!("{:<12} {:<14} {detail}", kind.label(), state)
                })
                .map_err(|err| anyhow::anyhow!("{err}")),
        };
        match result {
            Ok(text) => println!("{text}"),
            Err(err) => {
                failures += 1;
                eprintln!("{}: error: {err:#}", kind.label());
            }
        }
    }
    Ok(ExitCode::from(if failures == 0 { 0 } else { 1 }))
}

fn format_report(report: &HookReport) -> String {
    let mut out = format!(
        "{:<12} {}",
        report.kind.label(),
        if report.installed {
            "ok"
        } else {
            "not installed"
        }
    );
    out.push_str(&format!("\n  config  {}", report.config_path.display()));
    if let Some(backup) = &report.backup_path {
        out.push_str(&format!("\n  backup  {}", backup.display()));
    }
    if let Some(wrapper) = &report.wrapper_path {
        out.push_str(&format!("\n  wrapper {}", wrapper.display()));
    }
    for message in &report.messages {
        out.push_str(&format!("\n  - {message}"));
    }
    out
}

// ------------------------------------------------------------------- doctor --

fn cmd_doctor(cli: &Cli) -> Result<ExitCode> {
    let paths = cli.paths();
    let library = cli.library();
    let installer = cli.hooks();

    println!("bytepet doctor");
    println!("  data dir    {}", cli.data_dir.display());
    println!(
        "  config      {} ({})",
        paths.config_file.display(),
        if paths.config_file.is_file() {
            "found"
        } else {
            "defaults, not written yet"
        }
    );

    // Library roots + counts.
    let mut total = 0usize;
    let mut counts: Vec<(String, usize)> = Vec::new();
    for root in library.roots() {
        let count = std::fs::read_dir(&root.path)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.path().is_dir())
                    .filter(|entry| PetLibrary::load_entry(&entry.path(), root.kind).is_ok())
                    .count()
            })
            .unwrap_or(0);
        total += count;
        counts.push((root.kind.label().to_string(), count));
    }
    println!(
        "  library     {total} pet(s) [{}]",
        counts
            .iter()
            .map(|(label, count)| format!("{label} {count}"))
            .collect::<Vec<_>>()
            .join(" / ")
    );

    // Server reachability.
    let url = format!("http://127.0.0.1:{}", cli.port);
    let server_up = match http(cli.port, "GET", "/health", None) {
        HttpOutcome::Status(200, body) => {
            let health: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            println!(
                "  server      {url} up (pet={}, state={})",
                health["pet"].as_str().unwrap_or("-"),
                health["state"].as_str().unwrap_or("-")
            );
            true
        }
        HttpOutcome::Status(code, _) => {
            println!("  server      {url} responded with HTTP {code}");
            false
        }
        HttpOutcome::Refused => {
            println!("  server      {url} not running");
            false
        }
        HttpOutcome::Unreachable(err) => {
            println!("  server      {url} unreachable ({err})");
            false
        }
    };

    // Hooks.
    let mut hooks_installed = true;
    for kind in [AgentKind::Codex, AgentKind::ClaudeCode] {
        match installer.status(kind) {
            Ok(status) => {
                hooks_installed &= status.installed;
                println!("  hooks       {:<12} {}", kind.label(), status.detail);
            }
            Err(err) => {
                hooks_installed = false;
                println!("  hooks       {:<12} error: {err}", kind.label());
            }
        }
    }

    // Active pet validity.
    let config = AppConfig::load(&paths.config_file).unwrap_or_default();
    let active_valid = match &config.active_pet {
        Some(id) => match library.get(id) {
            Some(pet) => {
                let report = PetLibrary::validate_dir(&pet.dir);
                println!(
                    "  active pet  {id} — {} ({} rows, {} animations)",
                    if report.ok { "valid" } else { "INVALID" },
                    report.frame.rows,
                    report.animations.len()
                );
                if !report.ok {
                    for error in &report.errors {
                        println!("              error: {error}");
                    }
                }
                report.ok
            }
            None => {
                println!("  active pet  {id} — not found in the library");
                false
            }
        },
        None => {
            println!("  active pet  none selected");
            false
        }
    };

    let next = if !server_up {
        format!(
            "start the desktop app so hooks can reach 127.0.0.1:{}",
            cli.port
        )
    } else if !active_valid {
        "run `bytepet bytepet list` then `pet pet use <id>` to pick a pet".to_string()
    } else if !hooks_installed {
        "run `bytepet hooks install all` to forward Codex / Claude Code activity".to_string()
    } else {
        "everything looks good; try `bytepet state running \"working\"`".to_string()
    };
    println!("  next:       {next}");

    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------- http i/o --

enum HttpOutcome {
    Status(u16, String),
    Refused,
    Unreachable(String),
}

/// Minimal HTTP/1.1 client for the loopback JSON server (no TLS, no redirects).
fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpOutcome {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_millis(1200)) {
        Ok(stream) => stream,
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
            return HttpOutcome::Refused
        }
        Err(err) => return HttpOutcome::Unreachable(err.to_string()),
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(body) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");

    if let Err(err) = stream
        .write_all(request.as_bytes())
        .and_then(|()| match body {
            Some(body) => stream.write_all(body.as_bytes()),
            None => Ok(()),
        })
        .and_then(|()| stream.flush())
    {
        return HttpOutcome::Unreachable(err.to_string());
    }

    let mut response = String::new();
    if let Err(err) = stream.read_to_string(&mut response) {
        return HttpOutcome::Unreachable(err.to_string());
    }
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    HttpOutcome::Status(status, body)
}

// ----------------------------------------------------------------- helpers --

fn parse_duration_ms(raw: &str) -> std::result::Result<u64, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("duration must not be empty".to_string());
    }
    let split = raw
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    let value: f64 = number
        .parse()
        .map_err(|_| format!("invalid duration '{raw}'"))?;
    let millis = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "s" | "sec" | "secs" => value * 1000.0,
        "ms" => value,
        "m" | "min" | "mins" => value * 60_000.0,
        "h" | "hr" | "hrs" => value * 3_600_000.0,
        other => {
            return Err(format!(
                "unknown duration unit '{other}' (use ms, s, m or h)"
            ))
        }
    };
    if !millis.is_finite() || millis < 0.0 {
        return Err(format!("invalid duration '{raw}'"));
    }
    Ok(millis.round() as u64)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profile);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

/// Same directory Tauri uses for `app_config_dir()` with identifier
/// `com.bytepet.desktop` (the CLI cannot depend on `dirs`).
fn default_data_dir() -> PathBuf {
    config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_IDENTIFIER)
}

fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
        })
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration_ms("500ms").unwrap(), 500);
        assert_eq!(parse_duration_ms("5s").unwrap(), 5000);
        assert_eq!(parse_duration_ms("2m").unwrap(), 120_000);
        assert_eq!(parse_duration_ms("1h").unwrap(), 3_600_000);
        assert_eq!(parse_duration_ms("3").unwrap(), 3000);
        assert_eq!(parse_duration_ms("1.5s").unwrap(), 1500);
        assert!(parse_duration_ms("soon").is_err());
        assert!(parse_duration_ms("5w").is_err());
        assert!(parse_duration_ms("").is_err());
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("你好世界", 3), "你好…");
    }

    #[test]
    fn default_data_dir_ends_with_identifier() {
        assert!(default_data_dir().ends_with(APP_IDENTIFIER));
    }

    #[test]
    fn cli_shape_parses() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        let cli = Cli::try_parse_from(["pet", "state", "running", "hello", "--ttl", "5s"]).unwrap();
        match cli.command {
            Command::State(args) => {
                assert_eq!(args.state, "running");
                assert_eq!(args.message.as_deref(), Some("hello"));
                assert_eq!(args.ttl, Some(5000));
                assert_eq!(args.source, "cli");
            }
            _ => panic!("expected state"),
        }
        let cli = Cli::try_parse_from(["pet", "pet", "export", "zip", "out.zip"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Pet(PetCommand::Export { .. })
        ));
        let cli = Cli::try_parse_from(["pet", "hooks", "install", "claude-code"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Hooks(HooksCommand::Install {
                kind: HookSelection::ClaudeCode
            })
        ));
        let cli = Cli::try_parse_from(["pet", "hooks", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Hooks(HooksCommand::Status {
                kind: HookSelection::All
            })
        ));
    }
}
