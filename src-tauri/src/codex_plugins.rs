//! Managed plugin repair for provider homes.
//!
//! Applies approved plugin install intents (`CodexPluginLock` shape: plugin
//! ids, marketplace repository+ref — never cache payloads, absolute
//! source-machine paths, or credentials) against a bound Codex home. Replay
//! goes through Codex's own CLI (`codex plugin marketplace add` /
//! `codex plugin add`); the companion config codec rebuilds only
//! fingerprint-matched Codex-managed paths and never rewrites arbitrary
//! plugin files or user-authored config. `claude_inventory` reads Claude's
//! manager files into the same `Inventory` shape for the global inventory.
//!
//! Everything here is Tauri-free and driven through the `CodexRunner` trait
//! so tests can feed recorded CLI JSON without a real Codex installation.

use crate::codex_config;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const LOCK_SCHEMA: u32 = 1;
const MAX_LOCK_BYTES: u64 = 1024 * 1024;
const MAX_CLI_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;
const MAX_STRING: usize = 1024;
// ponytail: one flat timeout for every child; per-command budgets when a
// slow marketplace clone actually hits it.
const CHILD_TIMEOUT_SECS: u64 = 300;

// ── Lock file model ──────────────────────────────────────────────────────────
// Field order is the canonical serialization order; keep it stable.

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CodexPluginLock {
    pub schema: u32,
    #[serde(default)]
    pub captured_with: CapturedWith,
    #[serde(default)]
    pub marketplaces: Vec<CodexMarketplaceIntent>,
    #[serde(default)]
    pub plugins: Vec<CodexPluginIntent>,
    #[serde(default)]
    pub manual: Vec<CodexManualEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapturedWith {
    #[serde(default)]
    pub agent_version: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodexMarketplaceIntent {
    pub name: String,
    /// `owner/repo` shorthand or a Git URL — never a local path.
    pub repository: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodexPluginIntent {
    /// `<plugin>@<marketplace>`.
    pub id: String,
    /// Informational only: `codex plugin add` installs whatever version the
    /// target marketplace snapshot holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodexManualEntry {
    pub id: String,
    pub reason: String,
}

// ── Frontend-facing plan / report ────────────────────────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
pub struct CodexPluginPlan {
    pub missing_marketplaces: Vec<String>,
    pub missing_plugins: Vec<String>,
    pub missing_managed_marketplaces: Vec<CodexRepairIssue>,
    pub blocked_plugins: Vec<CodexRepairIssue>,
    pub config_repairs: Vec<CodexRepairIssue>,
    pub present: Vec<String>,
    pub drift: Vec<String>,
    pub disabled: Vec<String>,
    pub manual: Vec<String>,
    pub warnings: Vec<String>,
    /// Set when the plan cannot be computed at all (no usable codex CLI,
    /// unreadable lock); the UI shows this instead of counts.
    pub blocked: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodexRepairIssue {
    pub id: String,
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexRepairState {
    Ready,
    Partial,
    Failed,
}

impl Default for CodexRepairState {
    fn default() -> Self {
        // A report becomes Ready only after explicit final verification.
        CodexRepairState::Failed
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct CodexPluginRepairReport {
    pub state: CodexRepairState,
    pub marketplaces_added: Vec<String>,
    pub managed_marketplaces_provisioned: Vec<String>,
    pub plugins_installed: Vec<String>,
    pub already_present: Vec<String>,
    pub failed: Vec<String>,
    pub blocked_plugins: Vec<CodexRepairIssue>,
    pub config_paths_repaired: Vec<String>,
    pub manual: Vec<String>,
    pub verified: bool,
}

// ── Command runner ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait CodexRunner {
    fn run(&self, args: &[&str]) -> Result<CmdOutput, String>;

    /// Config used by this runner. Recovery reads it only to prove that a
    /// broken registered marketplace still points at the exact source in the
    /// synced lock before asking Codex to recreate its machine-local clone.
    fn config_path(&self) -> Option<PathBuf> {
        None
    }
}

/// Runs the real binary with piped output and a hard timeout; the child is
/// killed when the deadline passes so a hung clone cannot wedge the app.
pub struct ProcessRunner {
    pub program: PathBuf,
    pub timeout: Duration,
    /// A custom `.codex` mount is a real Codex home; point the CLI at it
    /// (mirrors CLAUDE_CONFIG_DIR in the Claude repair path).
    pub codex_home: Option<PathBuf>,
}

impl ProcessRunner {
    pub fn for_binary(program: PathBuf) -> ProcessRunner {
        ProcessRunner {
            program,
            timeout: Duration::from_secs(CHILD_TIMEOUT_SECS),
            codex_home: None,
        }
    }

    pub fn with_codex_home(mut self, codex_home: Option<PathBuf>) -> ProcessRunner {
        self.codex_home = codex_home;
        self
    }
}

fn drain<R: Read + Send + 'static>(stream: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        if let Some(mut stream) = stream {
            let mut chunk = [0_u8; 8 * 1024];
            loop {
                let read = match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                if captured.len() < MAX_CLI_BYTES {
                    let remaining = MAX_CLI_BYTES - captured.len();
                    captured.extend_from_slice(&chunk[..read.min(remaining)]);
                }
                // Keep draining after the capture limit so a noisy child
                // cannot fill its pipe and deadlock while we wait for it.
            }
        }
        String::from_utf8_lossy(&captured).into_owned()
    })
}

impl CodexRunner for ProcessRunner {
    fn run(&self, args: &[&str]) -> Result<CmdOutput, String> {
        let mut command = std::process::Command::new(&self.program);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(home) = &self.codex_home {
            command.env("CODEX_HOME", home);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("spawn {}: {}", self.program.display(), e))?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "'{} {}' timed out after {}s",
                            self.program.display(),
                            args.join(" "),
                            self.timeout.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("wait for codex: {}", e)),
            }
        };
        Ok(CmdOutput {
            success: status.success(),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
    }

    fn config_path(&self) -> Option<PathBuf> {
        let codex_home = self
            .codex_home
            .clone()
            .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))?;
        Some(codex_home.join("config.toml"))
    }
}

/// GUI apps on macOS launch with a minimal PATH; ask the login shell, then
/// fall back to the usual install locations. Shared with the Claude repair
/// path in `lib.rs`.
pub fn find_binary(name: &str) -> Result<PathBuf, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    if let Ok(out) = std::process::Command::new(&shell)
        .args(["-lc", &format!("command -v {}", name)])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    let home = dirs::home_dir().unwrap_or_default();
    for candidate in [
        home.join(format!(".local/bin/{}", name)),
        PathBuf::from(format!("/opt/homebrew/bin/{}", name)),
        PathBuf::from(format!("/usr/local/bin/{}", name)),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find the `{}` binary — install it or add it to PATH",
        name
    ))
}

/// Drop any child-output line that smells like a secret before it reaches
/// persistent logs. Coarse by design: a lost log line is cheaper than a
/// leaked token.
pub fn redact(line: &str) -> String {
    const MARKERS: &[&str] = &[
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "api key",
        "bearer",
        "authorization",
        "credential",
    ];
    let lower = line.to_ascii_lowercase();
    if MARKERS.iter().any(|m| lower.contains(m)) {
        "[redacted]".to_string()
    } else {
        line.to_string()
    }
}

// ── CLI inventory ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct MarketplaceInfo {
    pub name: String,
    pub root: String,
    pub source_type: String,
    pub source: String,
    pub git_ref: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub marketplace: String,
    pub version: String,
    pub installed: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub marketplaces: Vec<MarketplaceInfo>,
    pub plugins: Vec<PluginInfo>,
}

pub fn fetch_marketplaces(runner: &dyn CodexRunner) -> Result<Vec<MarketplaceInfo>, String> {
    let marketplaces = runner.run(&["plugin", "marketplace", "list", "--json"])?;
    if !marketplaces.success {
        let detail = redact(marketplaces.stderr.trim());
        if is_broken_marketplace_clone_error(&detail) {
            return Err(format!(
                "codex plugin marketplace list failed because synced config registers a marketplace whose machine-local clone is missing or invalid: {}",
                detail
            ));
        }
        return Err(format!(
            "codex plugin marketplace list failed — upgrade codex if this version has no plugin support: {}",
            detail
        ));
    }
    parse_marketplaces(&marketplaces.stdout)
}

pub fn fetch_plugins(runner: &dyn CodexRunner) -> Result<Vec<PluginInfo>, String> {
    let plugins = runner.run(&["plugin", "list", "--json"])?;
    if !plugins.success {
        return Err(format!(
            "codex plugin list failed: {}",
            redact(plugins.stderr.trim())
        ));
    }
    parse_plugins(&plugins.stdout)
}

pub fn fetch_inventory(runner: &dyn CodexRunner) -> Result<Inventory, String> {
    Ok(Inventory {
        marketplaces: fetch_marketplaces(runner)?,
        plugins: fetch_plugins(runner)?,
    })
}

#[derive(Deserialize, Default)]
struct CodexConfigFile {
    #[serde(default)]
    marketplaces: BTreeMap<String, RegisteredMarketplace>,
}

#[derive(Deserialize)]
struct RegisteredMarketplace {
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    source: String,
    #[serde(default, alias = "ref")]
    git_ref: Option<String>,
}

/// Codex currently reports an unloadable registered clone in this form. Keep
/// the text match narrow: an old CLI, policy denial, or unrelated failure must
/// never trigger a mutating recovery attempt.
fn is_broken_marketplace_clone_error(error: &str) -> bool {
    error.contains("failed to load marketplace(s)")
        && error.contains("marketplace root does not contain a supported manifest")
}

/// Extract the backtick-quoted marketplace names from the current CLI error.
/// No recognized name means no safe automatic recovery.
fn broken_marketplace_names(error: &str) -> BTreeSet<String> {
    if !is_broken_marketplace_clone_error(error) {
        return BTreeSet::new();
    }
    error
        .split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, value)| value)
        .filter(|value| ok_component(value))
        .map(str::to_string)
        .collect()
}

/// Return only broken marketplaces whose persisted registration exactly
/// matches the synced lock. This preserves the normal same-name/different-
/// source spoofing guard even though the CLI cannot produce an inventory.
fn marketplace_recovery_candidates(
    runner: &dyn CodexRunner,
    lock: &CodexPluginLock,
    inventory_error: &str,
) -> Result<Vec<CodexMarketplaceIntent>, String> {
    let names = broken_marketplace_names(inventory_error);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let config_path = runner
        .config_path()
        .ok_or("cannot verify marketplace registrations: Codex config path is unavailable")?;
    let raw = fs::read_to_string(&config_path).map_err(|e| {
        format!(
            "read {} before marketplace recovery: {}",
            config_path.display(),
            e
        )
    })?;
    if raw.len() as u64 > MAX_LOCK_BYTES * 4 {
        return Err("Codex config is too large to verify safely".to_string());
    }
    let config: CodexConfigFile = toml::from_str(&raw).map_err(|e| {
        format!(
            "parse {} before marketplace recovery: {}",
            config_path.display(),
            e
        )
    })?;

    let intents: BTreeMap<&str, &CodexMarketplaceIntent> = lock
        .marketplaces
        .iter()
        .map(|intent| (intent.name.as_str(), intent))
        .collect();
    let mut candidates = Vec::new();
    for name in names {
        let Some(intent) = intents.get(name.as_str()).copied() else {
            return Err(format!(
                "marketplace '{}' failed to load but is absent from the synced lock — refusing automatic recovery",
                name
            ));
        };
        let Some(registered) = config.marketplaces.get(&name) else {
            return Err(format!(
                "marketplace '{}' failed to load but is not registered in {} — refusing automatic recovery",
                name,
                config_path.display()
            ));
        };
        if registered.source_type != "git"
            || registered.source != intent.repository
            || registered.git_ref != intent.git_ref
        {
            return Err(format!(
                "marketplace '{}' is registered with a different source or ref — refusing automatic recovery",
                name
            ));
        }
        candidates.push(intent.clone());
    }
    Ok(candidates)
}

/// Tolerant parse of the two `--json` inventories (shapes observed on
/// codex-cli 0.144.1): `{"marketplaces":[{name, marketplaceSource:{sourceType,
/// source, ref?}}]}` and `{"installed":[{pluginId, marketplaceName, version,
/// installed, enabled}]}`. Entries missing required keys are skipped, not
/// fatal — one odd plugin must not hide the rest.
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_inventory(marketplaces_json: &str, plugins_json: &str) -> Result<Inventory, String> {
    if marketplaces_json.len() > MAX_CLI_BYTES || plugins_json.len() > MAX_CLI_BYTES {
        return Err("codex CLI output exceeds size cap".to_string());
    }
    Ok(Inventory {
        marketplaces: parse_marketplaces(marketplaces_json)?,
        plugins: parse_plugins(plugins_json)?,
    })
}

fn parse_marketplaces(marketplaces_json: &str) -> Result<Vec<MarketplaceInfo>, String> {
    if marketplaces_json.len() > MAX_CLI_BYTES {
        return Err("codex marketplace output exceeds size cap".to_string());
    }
    let m: serde_json::Value = serde_json::from_str(marketplaces_json)
        .map_err(|e| format!("parse marketplace list JSON: {}", e))?;
    let m_list = m
        .get("marketplaces")
        .and_then(|v| v.as_array())
        .ok_or("marketplace list JSON has no 'marketplaces' array — codex too old?")?;
    let mut marketplaces = Vec::new();
    for entry in m_list {
        let Some(name) = entry.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let source = entry.get("marketplaceSource");
        let get = |key: &str| {
            source
                .and_then(|s| s.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        marketplaces.push(MarketplaceInfo {
            name: name.to_string(),
            root: entry
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            source_type: get("sourceType").unwrap_or_default(),
            source: get("source").unwrap_or_default(),
            git_ref: get("ref")
                .or_else(|| get("gitRef"))
                .or_else(|| get("pinnedRef"))
                .or_else(|| get("commit")),
        });
    }
    Ok(marketplaces)
}

fn parse_plugins(plugins_json: &str) -> Result<Vec<PluginInfo>, String> {
    if plugins_json.len() > MAX_CLI_BYTES {
        return Err("codex plugin output exceeds size cap".to_string());
    }
    let p: serde_json::Value =
        serde_json::from_str(plugins_json).map_err(|e| format!("parse plugin list JSON: {}", e))?;
    let p_list = p
        .get("installed")
        .and_then(|v| v.as_array())
        .ok_or("plugin list JSON has no 'installed' array — codex too old?")?;
    let mut plugins = Vec::new();
    for entry in p_list {
        let Some(id) = entry.get("pluginId").and_then(|v| v.as_str()) else {
            continue;
        };
        let marketplace = entry
            .get("marketplaceName")
            .and_then(|v| v.as_str())
            .or_else(|| id.split_once('@').map(|(_, m)| m))
            .unwrap_or("")
            .to_string();
        plugins.push(PluginInfo {
            id: id.to_string(),
            marketplace,
            version: entry
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            installed: entry
                .get("installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            enabled: entry
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    Ok(plugins)
}

// ── Validation (trust boundary: the lock arrives from the cloud) ─────────────

fn windows_safe_component(s: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = s.split('.').next().unwrap_or("");
    !s.ends_with('.')
        && !s.ends_with(' ')
        && !RESERVED.iter().any(|name| stem.eq_ignore_ascii_case(name))
}

fn ok_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_STRING
        && !s.starts_with('-')
        && !s.starts_with('.')
        && windows_safe_component(s)
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn ok_marketplace_name(s: &str) -> bool {
    ok_component(s) && !s.bytes().any(|byte| byte.is_ascii_uppercase())
}

fn ok_plugin_id(id: &str) -> bool {
    id.split_once('@').is_some_and(|(plugin, marketplace)| {
        ok_component(plugin) && ok_marketplace_name(marketplace)
    })
}

/// `owner/repo` shorthand or a Git URL. Leading `-` (flag injection),
/// absolute/relative local paths, whitespace, and control chars are rejected.
fn ok_repository(s: &str) -> bool {
    if s.is_empty()
        || s.len() > MAX_STRING
        || s.starts_with('-')
        || s.starts_with('/')
        || s.starts_with('.')
        || s.contains('\\')
        || s.contains("::")
        || s.contains(['?', '#'])
        || s.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return false;
    }
    if let Some((scheme, rest)) = s.split_once("://") {
        if !matches!(scheme, "https" | "ssh" | "git") {
            return false;
        }
        let Some((authority, path)) = rest.split_once('/') else {
            return false;
        };
        if authority.is_empty()
            || path.is_empty()
            || path.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || !windows_safe_component(part)
            })
        {
            return false;
        }
        if let Some((userinfo, host)) = authority.rsplit_once('@') {
            if scheme != "ssh" || userinfo.is_empty() || userinfo.contains(':') || host.is_empty() {
                return false;
            }
        }
        return true;
    }
    if let Some(rest) = s.strip_prefix("git@") {
        return rest.split_once(':').is_some_and(|(host, path)| {
            !host.is_empty()
                && !path.is_empty()
                && !path.starts_with('/')
                && !path.split('/').any(|part| {
                    part.is_empty() || part == "." || part == ".." || !windows_safe_component(part)
                })
        });
    }
    let mut parts = s.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(repo), None) if ok_component(owner) && ok_component(repo))
}

fn ok_git_ref(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_STRING
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

fn ok_text(s: &str) -> bool {
    s.len() <= MAX_STRING && s.chars().all(|c| !c.is_control())
}

pub fn validate_lock(lock: &CodexPluginLock) -> Result<(), String> {
    if lock.schema != LOCK_SCHEMA {
        return Err(format!(
            "unsupported plugin lock schema {} (this app understands {})",
            lock.schema, LOCK_SCHEMA
        ));
    }
    if lock.marketplaces.len() > MAX_ENTRIES
        || lock.plugins.len() > MAX_ENTRIES
        || lock.manual.len() > MAX_ENTRIES
    {
        return Err(format!("plugin lock exceeds {} entries", MAX_ENTRIES));
    }
    if !ok_text(&lock.captured_with.agent_version) {
        return Err("invalid captured_with.agent_version".to_string());
    }
    let mut names = HashSet::new();
    for m in &lock.marketplaces {
        if !ok_marketplace_name(&m.name) {
            return Err(format!("invalid marketplace name '{}'", m.name));
        }
        if managed_marketplace(&m.name).is_some() {
            return Err(format!(
                "managed marketplace '{}' must not carry a source in the portable lock",
                m.name
            ));
        }
        if !ok_repository(&m.repository) {
            return Err(format!("invalid marketplace repository for '{}'", m.name));
        }
        if m.git_ref.as_deref().is_some_and(|r| !ok_git_ref(r)) {
            return Err(format!("invalid git ref for marketplace '{}'", m.name));
        }
        if !names.insert(m.name.clone()) {
            return Err(format!("duplicate marketplace '{}'", m.name));
        }
    }
    let mut ids = HashSet::new();
    for p in &lock.plugins {
        if !ok_plugin_id(&p.id) {
            return Err(format!("invalid plugin id '{}'", p.id));
        }
        let marketplace =
            p.id.split_once('@')
                .map(|(_, marketplace)| marketplace)
                .unwrap_or_default();
        if managed_marketplace(marketplace).is_none() && !names.contains(marketplace) {
            return Err(format!(
                "plugin '{}' references custom marketplace '{}' without a portable source",
                p.id, marketplace
            ));
        }
        if p.observed_version.as_deref().is_some_and(|v| {
            v.len() > MAX_STRING || v.chars().any(|c| c.is_control() || c.is_whitespace())
        }) {
            return Err(format!("invalid observed_version for '{}'", p.id));
        }
        if !ids.insert(p.id.clone()) {
            return Err(format!("duplicate plugin '{}'", p.id));
        }
    }
    let mut manual_ids = HashSet::new();
    for m in &lock.manual {
        if m.id.is_empty() || !ok_text(&m.id) || !ok_text(&m.reason) {
            return Err("invalid manual entry".to_string());
        }
        if !manual_ids.insert(m.id.clone()) {
            return Err(format!("duplicate manual entry '{}'", m.id));
        }
    }
    Ok(())
}

pub fn empty_lock() -> CodexPluginLock {
    CodexPluginLock {
        schema: LOCK_SCHEMA,
        ..CodexPluginLock::default()
    }
}

// ── Managed marketplaces ─────────────────────────────────────────────────────

const CURATED_MARKETPLACE: &str = "openai-curated";
const BUNDLED_MARKETPLACE: &str = "openai-bundled";
const PRIMARY_RUNTIME_MARKETPLACE: &str = "openai-primary-runtime";
const CURATED_OWNER_MARKER: &[u8] = b"managed by Agent Sync\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ManagedMarketplace {
    Curated,
    Bundled,
    PrimaryRuntime,
}

impl ManagedMarketplace {
    fn name(self) -> &'static str {
        match self {
            ManagedMarketplace::Curated => CURATED_MARKETPLACE,
            ManagedMarketplace::Bundled => BUNDLED_MARKETPLACE,
            ManagedMarketplace::PrimaryRuntime => PRIMARY_RUNTIME_MARKETPLACE,
        }
    }
}

fn managed_marketplace(name: &str) -> Option<ManagedMarketplace> {
    match name {
        CURATED_MARKETPLACE => Some(ManagedMarketplace::Curated),
        BUNDLED_MARKETPLACE => Some(ManagedMarketplace::Bundled),
        PRIMARY_RUNTIME_MARKETPLACE => Some(ManagedMarketplace::PrimaryRuntime),
        _ => None,
    }
}

fn explicitly_disabled_plugin_ids(config_path: Option<&Path>) -> Result<BTreeSet<String>, String> {
    let Some(path) = config_path.filter(|path| path.is_file()) else {
        return Ok(BTreeSet::new());
    };
    let metadata = fs::metadata(path)
        .map_err(|error| format!("read {} for plugin policy: {}", path.display(), error))?;
    if metadata.len() > MAX_LOCK_BYTES * 4 {
        return Err("Codex config is too large to inspect safely".to_string());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("read {} for plugin policy: {}", path.display(), error))?;
    Ok(
        codex_config::explicitly_disabled_plugin_ids_from_bytes(&bytes)?
            .into_iter()
            .filter(|id| ok_plugin_id(id))
            .collect(),
    )
}

// ── Plan ─────────────────────────────────────────────────────────────────────

fn issue(id: impl Into<String>, code: &str, message: impl Into<String>) -> CodexRepairIssue {
    CodexRepairIssue {
        id: id.into(),
        code: code.to_string(),
        message: message.into(),
    }
}

fn build_plan_for_intents(
    lock: &CodexPluginLock,
    intents: &[CodexPluginIntent],
    inventory: &Inventory,
    explicitly_disabled: &BTreeSet<String>,
) -> CodexPluginPlan {
    let target_marketplaces: BTreeMap<&str, &MarketplaceInfo> = inventory
        .marketplaces
        .iter()
        .map(|m| (m.name.as_str(), m))
        .collect();
    let target_plugins: BTreeMap<&str, &PluginInfo> = inventory
        .plugins
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();
    let mut plan = CodexPluginPlan::default();
    // Same marketplace name with a different source on the target is the
    // spoofing vector: never install through it, surface it instead.
    let mut conflicted: BTreeSet<&str> = BTreeSet::new();
    for m in &lock.marketplaces {
        match target_marketplaces.get(m.name.as_str()) {
            None => plan.missing_marketplaces.push(m.name.clone()),
            Some(t)
                if t.source_type == "git" && t.source == m.repository && t.git_ref == m.git_ref => {
            }
            Some(t) => {
                conflicted.insert(m.name.as_str());
                plan.warnings.push(format!(
                    "marketplace '{}' already exists here with a different source or ref ({} '{}' {:?}) — skipped, its plugins are blocked",
                    m.name, t.source_type, t.source, t.git_ref
                ));
            }
        }
    }
    let mut missing_managed = BTreeSet::new();
    for p in intents {
        if explicitly_disabled.contains(&p.id) {
            plan.disabled.push(p.id.clone());
            continue;
        }
        let marketplace = p.id.split_once('@').map(|(_, m)| m).unwrap_or("");
        match target_plugins.get(p.id.as_str()) {
            Some(t) if t.installed && t.enabled => {
                plan.present.push(p.id.clone());
                if let Some(v) = &p.observed_version {
                    if !t.version.is_empty() && &t.version != v {
                        plan.drift
                            .push(format!("{}: lock {} vs installed {}", p.id, v, t.version));
                    }
                }
            }
            // Deliberately disabled on this machine — report, never re-enable.
            Some(t) if t.installed => plan.disabled.push(p.id.clone()),
            _ => {
                if conflicted.contains(marketplace) {
                    let message = format!(
                        "'{}' is blocked because marketplace '{}' has a different source or ref",
                        p.id, marketplace
                    );
                    plan.warnings.push(message.clone());
                    plan.blocked_plugins.push(issue(
                        p.id.clone(),
                        "marketplace_source_mismatch",
                        message,
                    ));
                } else if target_marketplaces.contains_key(marketplace)
                    || lock.marketplaces.iter().any(|m| m.name == marketplace)
                {
                    plan.missing_plugins.push(p.id.clone());
                } else if managed_marketplace(marketplace).is_some() {
                    if missing_managed.insert(marketplace.to_string()) {
                        plan.missing_managed_marketplaces.push(issue(
                            marketplace,
                            "managed_catalog_missing",
                            format!(
                                "Managed marketplace '{}' is not initialized for this Codex home",
                                marketplace
                            ),
                        ));
                    }
                    plan.blocked_plugins.push(issue(
                        p.id.clone(),
                        "managed_catalog_missing",
                        format!("'{}' requires managed marketplace '{}'", p.id, marketplace),
                    ));
                } else {
                    let message = format!(
                        "'{}' is blocked: marketplace '{}' is neither configured here nor in the lock",
                        p.id, marketplace
                    );
                    plan.warnings.push(message.clone());
                    plan.blocked_plugins
                        .push(issue(p.id.clone(), "marketplace_missing", message));
                }
            }
        }
    }
    plan.manual = lock
        .manual
        .iter()
        .map(|m| format!("{} — {}", m.id, m.reason))
        .collect();
    plan
}

// ── Target-owned managed marketplaces ──────────────────────────────────────

#[derive(Clone, Debug)]
struct ManagedSource {
    root: PathBuf,
    sha_sidecar: Option<PathBuf>,
}

#[derive(Default)]
struct ManagedDiscovery {
    sources: BTreeMap<ManagedMarketplace, ManagedSource>,
    issues: BTreeMap<ManagedMarketplace, CodexRepairIssue>,
}

struct ManagedMarketplaceResolver {
    default_home: PathBuf,
    target_home: PathBuf,
}

fn ensure_no_symlink_below(base: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(base).map_err(|_| {
        format!(
            "managed path '{}' is outside approved root '{}'",
            path.display(),
            base.display()
        )
    })?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(format!(
                "managed path '{}' contains an unsafe component",
                path.display()
            ));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("inspect '{}': {}", current.display(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "managed path '{}' traverses symlink '{}'",
                path.display(),
                current.display()
            ));
        }
    }
    Ok(())
}

fn ensure_no_symlink_ancestors_below(base: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(base).map_err(|_| {
        format!(
            "managed path '{}' is outside approved root '{}'",
            path.display(),
            base.display()
        )
    })?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(format!(
                "managed path '{}' contains an unsafe component",
                path.display()
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "managed path '{}' traverses symlink '{}'",
                    path.display(),
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("inspect '{}': {}", current.display(), error)),
        }
    }
    Ok(())
}

fn has_curated_owner_marker(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
        && fs::read(path).is_ok_and(|contents| contents == CURATED_OWNER_MARKER)
}

fn validate_curated_tree(root: &Path) -> Result<(), String> {
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| format!("inspect curated catalog: {}", error))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "curated catalog contains unsupported symlink '{}'",
                entry.path().display()
            ));
        }
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(format!(
                "curated catalog contains unsupported file type '{}'",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

impl ManagedMarketplaceResolver {
    fn new(default_home: PathBuf, target_home: PathBuf) -> Self {
        Self {
            default_home,
            target_home,
        }
    }

    fn discover(
        &self,
        runner: &dyn CodexRunner,
        required: &BTreeSet<ManagedMarketplace>,
    ) -> ManagedDiscovery {
        let mut discovery = ManagedDiscovery::default();
        let inventory = match fetch_marketplaces(runner) {
            Ok(inventory) => inventory,
            Err(error) => {
                for marketplace in required {
                    discovery.issues.insert(
                        *marketplace,
                        issue(
                            marketplace.name(),
                            "managed_catalog_missing",
                            format!(
                                "Could not inspect this machine's managed marketplaces: {}",
                                error
                            ),
                        ),
                    );
                }
                return discovery;
            }
        };
        let by_name: BTreeMap<&str, &MarketplaceInfo> = inventory
            .iter()
            .map(|marketplace| (marketplace.name.as_str(), marketplace))
            .collect();
        for marketplace in required {
            let Some(info) = by_name.get(marketplace.name()).copied() else {
                discovery.issues.insert(
                    *marketplace,
                    issue(
                        marketplace.name(),
                        "managed_catalog_missing",
                        format!(
                            "Managed marketplace '{}' is not initialized in the default Codex home",
                            marketplace.name()
                        ),
                    ),
                );
                continue;
            };
            match self.validate_source(*marketplace, info) {
                Ok(source) => {
                    discovery.sources.insert(*marketplace, source);
                }
                Err(error) => {
                    discovery.issues.insert(
                        *marketplace,
                        issue(marketplace.name(), "managed_catalog_invalid", error),
                    );
                }
            }
        }
        discovery
    }

    fn validate_source(
        &self,
        marketplace: ManagedMarketplace,
        info: &MarketplaceInfo,
    ) -> Result<ManagedSource, String> {
        let root = match marketplace {
            ManagedMarketplace::Curated => PathBuf::from(&info.root),
            _ => PathBuf::from(if info.source.is_empty() {
                &info.root
            } else {
                &info.source
            }),
        };
        if root.as_os_str().is_empty() {
            return Err(format!(
                "Managed marketplace '{}' has no source root",
                marketplace.name()
            ));
        }
        if marketplace != ManagedMarketplace::Curated && info.source_type != "local" {
            return Err(format!(
                "Managed marketplace '{}' has unexpected source type '{}'",
                marketplace.name(),
                info.source_type
            ));
        }
        let trusted_base = match marketplace {
            ManagedMarketplace::Curated | ManagedMarketplace::Bundled => &self.default_home,
            ManagedMarketplace::PrimaryRuntime => {
                self.default_home.parent().unwrap_or(&self.default_home)
            }
        };
        ensure_no_symlink_below(trusted_base, &root)?;
        let canonical = root.canonicalize().map_err(|error| {
            format!(
                "Managed marketplace '{}' source '{}' is unavailable: {}",
                marketplace.name(),
                root.display(),
                error
            )
        })?;
        let expected = match marketplace {
            ManagedMarketplace::Curated => self.default_home.join(".tmp/plugins"),
            ManagedMarketplace::Bundled => self
                .default_home
                .join(".tmp/bundled-marketplaces/openai-bundled"),
            ManagedMarketplace::PrimaryRuntime => self
                .default_home
                .parent()
                .unwrap_or(&self.default_home)
                .join(".cache/codex-runtimes"),
        };
        let expected = expected.canonicalize().map_err(|error| {
            format!(
                "Approved root for '{}' is unavailable: {}",
                marketplace.name(),
                error
            )
        })?;
        let approved = match marketplace {
            ManagedMarketplace::PrimaryRuntime => canonical.starts_with(&expected),
            _ => canonical == expected,
        };
        if !approved {
            return Err(format!(
                "Managed marketplace '{}' source '{}' is outside its approved target-machine root",
                marketplace.name(),
                canonical.display()
            ));
        }
        validate_marketplace_manifest(&canonical, marketplace.name())?;
        let sha_sidecar = if marketplace == ManagedMarketplace::Curated {
            let sha = self.default_home.join(".tmp/plugins.sha");
            validate_curated_pair(&root, &sha)?;
            Some(sha)
        } else {
            None
        };
        Ok(ManagedSource {
            root: canonical,
            sha_sidecar,
        })
    }

    fn provision_curated(&self, source: &ManagedSource) -> Result<bool, String> {
        let target_root = self.target_home.join(".tmp/plugins");
        let target_sha = self.target_home.join(".tmp/plugins.sha");
        let marker = self.target_home.join(".tmp/plugins.agent-sync-owned");
        // Never follow a target-home staging symlink. Otherwise `.tmp` could
        // redirect validation or the later atomic renames into another
        // profile even though the leaf catalog itself contains no symlinks.
        ensure_no_symlink_ancestors_below(&self.target_home, &target_root)?;
        ensure_no_symlink_ancestors_below(&self.target_home, &target_sha)?;
        ensure_no_symlink_ancestors_below(&self.target_home, &marker)?;
        if validate_curated_pair(&target_root, &target_sha).is_ok() {
            return Ok(false);
        }
        if self.target_home == self.default_home {
            return Err("Default Codex curated catalog is incomplete or invalid".to_string());
        }
        let source_sha = source
            .sha_sidecar
            .as_ref()
            .ok_or("validated curated source has no SHA sidecar")?;
        let sha_before = read_curated_sha(source_sha)?;
        let parent = target_root
            .parent()
            .ok_or("target curated catalog has no parent")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
        let staging = tempfile::Builder::new()
            .prefix(".agent-sync-curated-stage-")
            .tempdir_in(parent)
            .map_err(|error| format!("stage curated catalog: {}", error))?;
        let staged_root = staging.path().join("plugins");
        let staged_sha = staging.path().join("plugins.sha");
        copy_tree(&source.root, &staged_root)?;
        fs::copy(source_sha, &staged_sha)
            .map_err(|error| format!("stage curated SHA: {}", error))?;
        let sha_after = read_curated_sha(source_sha)?;
        if sha_before != sha_after {
            return Err(
                "Curated catalog changed while it was being staged; retry Repair".to_string(),
            );
        }
        validate_curated_pair(&staged_root, &staged_sha)?;

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut old_root = None;
        if fs::symlink_metadata(&target_root).is_ok() {
            if !has_curated_owner_marker(&marker) {
                return Err(format!(
                    "Refusing to replace unowned directory or link at '{}'",
                    target_root.display()
                ));
            }
            let backup = parent.join(format!("plugins.agent-sync-backup-{}", stamp));
            fs::rename(&target_root, &backup)
                .map_err(|error| format!("backup stale curated catalog: {}", error))?;
            old_root = Some(backup);
        }
        let mut old_sha = None;
        if fs::symlink_metadata(&target_sha).is_ok() {
            let backup = parent.join(format!("plugins.sha.agent-sync-backup-{}", stamp));
            if let Err(error) = fs::rename(&target_sha, &backup) {
                if let Some(old) = &old_root {
                    let _ = fs::rename(old, &target_root);
                }
                return Err(format!("backup stale curated SHA: {}", error));
            }
            old_sha = Some(backup);
        }
        if let Err(error) = fs::rename(&staged_root, &target_root) {
            if let Some(old) = &old_root {
                let _ = fs::rename(old, &target_root);
            }
            if let Some(old) = &old_sha {
                let _ = fs::rename(old, &target_sha);
            }
            return Err(format!("publish curated catalog: {}", error));
        }
        if let Err(error) = fs::rename(&staged_sha, &target_sha) {
            let _ = fs::rename(&target_root, &staged_root);
            if let Some(old) = &old_root {
                let _ = fs::rename(old, &target_root);
            }
            if let Some(old) = &old_sha {
                let _ = fs::rename(old, &target_sha);
            }
            return Err(format!("publish curated SHA: {}", error));
        }
        if let Err(error) = validate_curated_pair(&target_root, &target_sha) {
            // The staged pair validated before publication, but a concurrent
            // mutation or filesystem fault can still invalidate it. Restore
            // the previous pair instead of leaving a broken active catalog.
            let _ = fs::rename(&target_root, &staged_root);
            let _ = fs::rename(&target_sha, &staged_sha);
            if let Some(old) = &old_root {
                let _ = fs::rename(old, &target_root);
            }
            if let Some(old) = &old_sha {
                let _ = fs::rename(old, &target_sha);
            }
            return Err(format!(
                "published curated catalog failed validation: {}",
                error
            ));
        }
        fs::write(&marker, CURATED_OWNER_MARKER)
            .map_err(|error| format!("mark curated catalog ownership: {}", error))?;
        Ok(true)
    }
}

fn validate_marketplace_manifest(root: &Path, expected_name: &str) -> Result<(), String> {
    let path = root.join(".agents/plugins/marketplace.json");
    ensure_no_symlink_below(root, &path)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("read marketplace manifest '{}': {}", path.display(), error))?;
    if metadata.len() > MAX_CLI_BYTES as u64 {
        return Err(format!(
            "marketplace manifest '{}' is too large",
            path.display()
        ));
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("read marketplace manifest '{}': {}", path.display(), error))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| format!("parse marketplace manifest '{}': {}", path.display(), error))?;
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if name != expected_name {
        return Err(format!(
            "marketplace manifest '{}' declares '{}' instead of '{}'",
            path.display(),
            name,
            expected_name
        ));
    }
    Ok(())
}

fn valid_git_oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_curated_sha(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read curated SHA '{}': {}", path.display(), error))?;
    let value = raw.trim().to_ascii_lowercase();
    if !valid_git_oid(&value) {
        return Err(format!(
            "curated SHA '{}' is not a Git commit id",
            path.display()
        ));
    }
    Ok(value)
}

fn read_git_head(root: &Path) -> Result<String, String> {
    let git = root.join(".git");
    if fs::symlink_metadata(&git)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("curated .git directory must not be a symlink".to_string());
    }
    let head = fs::read_to_string(git.join("HEAD"))
        .map_err(|error| format!("read curated Git HEAD: {}", error))?;
    let head = head.trim();
    let value = if let Some(reference) = head.strip_prefix("ref: ") {
        if reference.starts_with('/') || reference.split('/').any(|component| component == "..") {
            return Err("curated Git HEAD contains an unsafe ref".to_string());
        }
        match fs::read_to_string(git.join(reference)) {
            Ok(value) => value.trim().to_string(),
            Err(_) => {
                let packed = fs::read_to_string(git.join("packed-refs"))
                    .map_err(|error| format!("resolve curated Git HEAD: {}", error))?;
                packed
                    .lines()
                    .filter(|line| !line.starts_with('#') && !line.starts_with('^'))
                    .filter_map(|line| line.split_once(' '))
                    .find_map(|(oid, name)| (name == reference).then(|| oid.to_string()))
                    .ok_or("curated Git HEAD ref is unresolved")?
            }
        }
    } else {
        head.to_string()
    };
    let value = value.trim().to_ascii_lowercase();
    if !valid_git_oid(&value) {
        return Err("curated Git HEAD is not a commit id".to_string());
    }
    Ok(value)
}

fn validate_curated_pair(root: &Path, sha_path: &Path) -> Result<(), String> {
    for (label, path) in [("catalog", root), ("SHA sidecar", sha_path)] {
        if fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!(
                "curated {} '{}' must not be a symlink",
                label,
                path.display()
            ));
        }
    }
    validate_curated_tree(root)?;
    validate_marketplace_manifest(root, CURATED_MARKETPLACE)?;
    let expected = read_curated_sha(sha_path)?;
    let actual = read_git_head(root)?;
    if expected != actual {
        return Err(format!(
            "curated SHA sidecar '{}' does not match checkout HEAD",
            sha_path.display()
        ));
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| format!("copy curated catalog: {}", error))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| format!("copy curated catalog: {}", error))?;
        let target = destination.join(relative);
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect '{}': {}", entry.path().display(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "curated catalog contains unsupported symlink '{}'",
                entry.path().display()
            ));
        }
        if metadata.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("create '{}': {}", target.display(), error))?;
            let _ = fs::set_permissions(&target, metadata.permissions());
        } else if metadata.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("create '{}': {}", parent.display(), error))?;
            }
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("copy '{}': {}", entry.path().display(), error))?;
            let _ = fs::set_permissions(&target, metadata.permissions());
        } else {
            return Err(format!(
                "curated catalog contains unsupported file type '{}'",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

// ── Apply ────────────────────────────────────────────────────────────────────

fn apply_plan_for_intents(
    runner: &dyn CodexRunner,
    lock: &CodexPluginLock,
    intents: &[CodexPluginIntent],
    explicitly_disabled: &BTreeSet<String>,
    log: &mut dyn FnMut(&str, &str),
) -> Result<CodexPluginRepairReport, String> {
    let inventory = match fetch_inventory(runner) {
        Ok(inventory) => inventory,
        Err(inventory_error) => {
            let candidates = marketplace_recovery_candidates(runner, lock, &inventory_error)
                .map_err(|recovery_error| {
                    format!(
                        "{}; automatic recovery blocked: {}",
                        inventory_error, recovery_error
                    )
                })?;
            if candidates.is_empty() {
                return Err(inventory_error);
            }
            for intent in candidates {
                let mut args = vec!["plugin", "marketplace", "add", intent.repository.as_str()];
                if let Some(git_ref) = &intent.git_ref {
                    args.push("--ref");
                    args.push(git_ref);
                }
                log(
                    "info",
                    &format!(
                        "Re-cloning registered marketplace '{}' from its verified source",
                        intent.name
                    ),
                );
                log("info", &format!("$ codex {}", args.join(" ")));
                let out = runner
                    .run(&args)
                    .map_err(|e| format!("re-clone marketplace '{}': {}", intent.name, e))?;
                for line in out.stdout.lines().chain(out.stderr.lines()) {
                    if !line.trim().is_empty() {
                        log("info", &format!("  {}", redact(line)));
                    }
                }
                if !out.success {
                    return Err(format!(
                        "marketplace '{}' could not be re-cloned from its verified source",
                        intent.name
                    ));
                }
            }
            fetch_inventory(runner).map_err(|retry_error| {
                format!(
                    "marketplace clones were recreated, but Codex inventory still fails: {}",
                    retry_error
                )
            })?
        }
    };
    let plan = build_plan_for_intents(lock, &intents, &inventory, explicitly_disabled);
    let mut already_present = plan.present.clone();
    already_present.extend(plan.disabled.iter().cloned());
    already_present.sort();
    already_present.dedup();
    let mut report = CodexPluginRepairReport {
        already_present,
        manual: plan.manual.clone(),
        blocked_plugins: plan.blocked_plugins.clone(),
        ..CodexPluginRepairReport::default()
    };
    for warning in &plan.warnings {
        log("error", warning);
    }
    for drift in &plan.drift {
        log("info", &format!("version drift (not updated): {}", drift));
    }
    for manual in &plan.manual {
        log("info", &format!("manual follow-up: {}", manual));
    }

    let marketplace_intents: BTreeMap<&str, &CodexMarketplaceIntent> = lock
        .marketplaces
        .iter()
        .map(|m| (m.name.as_str(), m))
        .collect();
    let mut failed_marketplaces: BTreeSet<&str> = BTreeSet::new();
    for name in &plan.missing_marketplaces {
        let intent = marketplace_intents[name.as_str()];
        let mut args = vec!["plugin", "marketplace", "add", intent.repository.as_str()];
        if let Some(git_ref) = &intent.git_ref {
            args.push("--ref");
            args.push(git_ref);
        }
        log("info", &format!("$ codex {}", args.join(" ")));
        let added = match runner.run(&args) {
            Ok(out) => {
                for line in out.stdout.lines().chain(out.stderr.lines()) {
                    if !line.trim().is_empty() {
                        log("info", &format!("  {}", redact(line)));
                    }
                }
                // Trust the marketplace list, not the exit code: the add must
                // actually be visible before plugins install through it.
                out.success
                    && fetch_inventory(runner).is_ok_and(|inv| {
                        inv.marketplaces.iter().any(|marketplace| {
                            &marketplace.name == name
                                && marketplace.source_type == "git"
                                && marketplace.source == intent.repository
                                && marketplace.git_ref == intent.git_ref
                        })
                    })
            }
            Err(e) => {
                log("error", &format!("✗ marketplace {}: {}", name, e));
                false
            }
        };
        if added {
            log("ok", &format!("✓ marketplace {}", name));
            report.marketplaces_added.push(name.clone());
        } else {
            log("error", &format!("✗ marketplace {} did not appear", name));
            report.failed.push(format!("marketplace {}", name));
            failed_marketplaces.insert(name.as_str());
        }
    }

    for id in &plan.missing_plugins {
        let marketplace = id.split_once('@').map(|(_, m)| m).unwrap_or("");
        if failed_marketplaces.contains(marketplace) {
            log(
                "error",
                &format!("✗ {} skipped: its marketplace failed", id),
            );
            report.failed.push(id.clone());
            continue;
        }
        log("info", &format!("$ codex plugin add {}", id));
        match runner.run(&["plugin", "add", id]) {
            Ok(out) if out.success => {
                log("ok", &format!("✓ {}", id));
                report.plugins_installed.push(id.clone());
            }
            Ok(out) => {
                for line in out.stdout.lines().chain(out.stderr.lines()) {
                    if !line.trim().is_empty() {
                        log("info", &format!("  {}", redact(line)));
                    }
                }
                log("error", &format!("✗ {}", id));
                report.failed.push(id.clone());
            }
            Err(e) => {
                log("error", &format!("✗ {}: {}", id, e));
                report.failed.push(id.clone());
            }
        }
    }

    let blocked: BTreeSet<&str> = report
        .blocked_plugins
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    match fetch_inventory(runner) {
        Ok(final_inventory) => {
            for intent in intents {
                let deliberately_disabled = plan.disabled.contains(&intent.id);
                let satisfied = deliberately_disabled
                    || final_inventory
                        .plugins
                        .iter()
                        .any(|plugin| plugin.id == intent.id && plugin.installed && plugin.enabled);
                if !satisfied
                    && !blocked.contains(intent.id.as_str())
                    && !report.failed.contains(&intent.id)
                {
                    log(
                        "error",
                        &format!(
                            "final verification did not find requested plugin {}",
                            intent.id
                        ),
                    );
                    report.failed.push(intent.id.clone());
                }
            }
        }
        Err(e) => {
            log("error", &format!("final verification failed: {}", e));
            let mut unresolved = intents
                .iter()
                .filter(|intent| {
                    !plan.disabled.contains(&intent.id) && !blocked.contains(intent.id.as_str())
                })
                .map(|intent| intent.id.clone())
                .collect::<Vec<_>>();
            if unresolved.is_empty() {
                unresolved.push("final verification".to_string());
            }
            report.failed.extend(unresolved);
        }
    }
    report.failed.sort();
    report.failed.dedup();
    report.state = if !report.failed.is_empty() {
        CodexRepairState::Failed
    } else if !report.blocked_plugins.is_empty() || !report.manual.is_empty() {
        CodexRepairState::Partial
    } else {
        CodexRepairState::Ready
    };
    report.verified = report.state == CodexRepairState::Ready;
    Ok(report)
}

fn validate_existing_target_source(
    marketplace: ManagedMarketplace,
    info: &MarketplaceInfo,
    target_home: &Path,
    default_home: &Path,
) -> Result<ManagedSource, String> {
    let raw_root = if marketplace == ManagedMarketplace::Curated || info.source.is_empty() {
        &info.root
    } else {
        &info.source
    };
    let root = PathBuf::from(raw_root);
    match marketplace {
        ManagedMarketplace::Curated => ensure_no_symlink_below(target_home, &root)?,
        ManagedMarketplace::Bundled => {
            if root.starts_with(target_home) {
                ensure_no_symlink_below(target_home, &root)?;
            } else {
                ensure_no_symlink_below(default_home, &root)?;
            }
        }
        ManagedMarketplace::PrimaryRuntime => {
            ensure_no_symlink_below(default_home.parent().unwrap_or(default_home), &root)?
        }
    }
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("source '{}' is unavailable: {}", root.display(), error))?;
    let approved = match marketplace {
        ManagedMarketplace::Curated => {
            let expected = target_home
                .join(".tmp/plugins")
                .canonicalize()
                .map_err(|error| format!("target curated catalog is unavailable: {}", error))?;
            canonical == expected
        }
        ManagedMarketplace::Bundled => {
            let target = target_home.join(".tmp/bundled-marketplaces/openai-bundled");
            let default = default_home.join(".tmp/bundled-marketplaces/openai-bundled");
            [target, default]
                .into_iter()
                .filter_map(|path| path.canonicalize().ok())
                .any(|path| path == canonical)
        }
        ManagedMarketplace::PrimaryRuntime => default_home
            .parent()
            .unwrap_or(default_home)
            .join(".cache/codex-runtimes")
            .canonicalize()
            .is_ok_and(|root| canonical.starts_with(root)),
    };
    if !approved {
        return Err(format!(
            "source '{}' is outside the approved target/default Codex roots",
            canonical.display()
        ));
    }
    validate_marketplace_manifest(&canonical, marketplace.name())?;
    let sha_sidecar = if marketplace == ManagedMarketplace::Curated {
        let sha = target_home.join(".tmp/plugins.sha");
        validate_curated_pair(&canonical, &sha)?;
        Some(sha)
    } else {
        None
    };
    Ok(ManagedSource {
        root: canonical,
        sha_sidecar,
    })
}

fn target_inventory_matches(
    marketplace: ManagedMarketplace,
    expected: &ManagedSource,
    inventory: &[MarketplaceInfo],
    target_home: &Path,
) -> bool {
    let Some(info) = inventory
        .iter()
        .find(|entry| entry.name == marketplace.name())
    else {
        return false;
    };
    let raw_root = if marketplace == ManagedMarketplace::Curated || info.source.is_empty() {
        &info.root
    } else {
        &info.source
    };
    let Ok(actual) = PathBuf::from(raw_root).canonicalize() else {
        return false;
    };
    let expected_root = if marketplace == ManagedMarketplace::Curated {
        target_home.join(".tmp/plugins")
    } else {
        expected.root.clone()
    };
    expected_root
        .canonicalize()
        .is_ok_and(|expected| actual == expected)
}

fn block_managed_dependents(
    intents: &[CodexPluginIntent],
    marketplace: ManagedMarketplace,
    catalog_issue: &CodexRepairIssue,
    blocked: &mut Vec<CodexRepairIssue>,
) {
    for intent in intents {
        if intent
            .id
            .split_once('@')
            .is_some_and(|(_, name)| name == marketplace.name())
        {
            blocked.push(issue(
                intent.id.clone(),
                &catalog_issue.code,
                format!("{}: {}", intent.id, catalog_issue.message),
            ));
        }
    }
}

/// Apply only the plugin intents explicitly approved by the caller. This
/// never pulls additional enabled plugin intent from the target config, so
/// one project-sync approval cannot install an unrelated plugin. Managed
/// marketplace provisioning and final verification run on every repair.
pub fn apply_approved_managed_plan(
    target_runner: &dyn CodexRunner,
    default_runner: &dyn CodexRunner,
    lock: &CodexPluginLock,
    target_home: &Path,
    default_home: &Path,
    log: &mut dyn FnMut(&str, &str),
) -> Result<CodexPluginRepairReport, String> {
    validate_lock(lock)?;
    let explicitly_disabled =
        explicitly_disabled_plugin_ids(Some(&target_home.join("config.toml")))?;
    apply_managed_plan_for_intents(
        target_runner,
        default_runner,
        lock,
        &lock.plugins,
        &explicitly_disabled,
        target_home,
        default_home,
        log,
    )
}

fn apply_managed_plan_for_intents(
    target_runner: &dyn CodexRunner,
    default_runner: &dyn CodexRunner,
    lock: &CodexPluginLock,
    intents: &[CodexPluginIntent],
    explicitly_disabled: &BTreeSet<String>,
    target_home: &Path,
    default_home: &Path,
    log: &mut dyn FnMut(&str, &str),
) -> Result<CodexPluginRepairReport, String> {
    let target_config = target_home.join("config.toml");
    let initial_config_issues = codex_config::inspect_managed_config_with_default(
        &target_config,
        target_home,
        default_home,
    );
    let needs_mcp_repair = initial_config_issues
        .iter()
        .any(|entry| entry.id.starts_with("mcp_servers."));
    let mut required: BTreeSet<ManagedMarketplace> = intents
        .iter()
        .filter(|intent| !explicitly_disabled.contains(&intent.id))
        .filter_map(|intent| {
            intent
                .id
                .split_once('@')
                .and_then(|(_, name)| managed_marketplace(name))
        })
        .collect();
    for entry in &initial_config_issues {
        if let Some(name) = entry
            .id
            .strip_prefix("marketplaces.")
            .and_then(|tail| tail.split('.').next())
        {
            if let Some(marketplace) = managed_marketplace(name) {
                required.insert(marketplace);
            }
        }
    }
    let resolver =
        ManagedMarketplaceResolver::new(default_home.to_path_buf(), target_home.to_path_buf());
    let mut discovery = resolver.discover(default_runner, &required);

    // A target-owned valid catalog remains usable even if the default
    // inventory is temporarily unavailable. It must still live under an
    // approved target/default location; arbitrary pulled paths never qualify.
    if !discovery.issues.is_empty() {
        if let Ok(target_marketplaces) = fetch_marketplaces(target_runner) {
            for marketplace in required.iter().copied().collect::<Vec<_>>() {
                if !discovery.issues.contains_key(&marketplace) {
                    continue;
                }
                if let Some(info) = target_marketplaces
                    .iter()
                    .find(|info| info.name == marketplace.name())
                {
                    if let Ok(source) = validate_existing_target_source(
                        marketplace,
                        info,
                        target_home,
                        default_home,
                    ) {
                        discovery.sources.insert(marketplace, source);
                        discovery.issues.remove(&marketplace);
                    }
                }
            }
        }
    }

    let mut attempted_failures = Vec::new();
    let mut blocked = Vec::new();
    let mut provisioned = Vec::new();
    let mut config_repairs = Vec::new();
    let mut unavailable: BTreeSet<ManagedMarketplace> = discovery.issues.keys().copied().collect();
    for (marketplace, catalog_issue) in &discovery.issues {
        log("error", &catalog_issue.message);
        block_managed_dependents(intents, *marketplace, catalog_issue, &mut blocked);
    }

    if required.contains(&ManagedMarketplace::Curated)
        && !unavailable.contains(&ManagedMarketplace::Curated)
    {
        let source = &discovery.sources[&ManagedMarketplace::Curated];
        log("info", "Validating the target openai-curated catalog");
        match resolver.provision_curated(source) {
            Ok(true) => {
                log("ok", "✓ openai-curated catalog provisioned");
                provisioned.push(CURATED_MARKETPLACE.to_string());
            }
            Ok(false) => {}
            Err(error) => {
                let catalog_issue = issue(
                    CURATED_MARKETPLACE,
                    "managed_catalog_provision_failed",
                    error,
                );
                log("error", &catalog_issue.message);
                attempted_failures.push(format!("marketplace {}", CURATED_MARKETPLACE));
                unavailable.insert(ManagedMarketplace::Curated);
                block_managed_dependents(
                    intents,
                    ManagedMarketplace::Curated,
                    &catalog_issue,
                    &mut blocked,
                );
            }
        }
    }
    if required.contains(&ManagedMarketplace::Curated)
        && !unavailable.contains(&ManagedMarketplace::Curated)
    {
        match codex_config::remove_explicit_curated_marketplace(&target_config) {
            Ok(changed) => {
                for path in &changed {
                    log("ok", &format!("✓ repaired {}", path));
                }
                config_repairs.extend(changed);
            }
            Err(error) => {
                let catalog_issue = issue(
                    CURATED_MARKETPLACE,
                    "managed_catalog_source_mismatch",
                    format!(
                        "Could not remove stale explicit curated registration: {}",
                        error
                    ),
                );
                log("error", &catalog_issue.message);
                attempted_failures.push(format!("marketplace {} config", CURATED_MARKETPLACE));
                unavailable.insert(ManagedMarketplace::Curated);
                block_managed_dependents(
                    intents,
                    ManagedMarketplace::Curated,
                    &catalog_issue,
                    &mut blocked,
                );
            }
        }
    }

    let mut local_sources = BTreeMap::new();
    for marketplace in [
        ManagedMarketplace::Bundled,
        ManagedMarketplace::PrimaryRuntime,
    ] {
        if required.contains(&marketplace) && !unavailable.contains(&marketplace) {
            local_sources.insert(
                marketplace.name().to_string(),
                discovery.sources[&marketplace].root.clone(),
            );
        }
    }
    if !local_sources.is_empty() {
        match codex_config::rebind_managed_marketplaces(
            &target_home.join("config.toml"),
            &local_sources,
        ) {
            Ok(changed) => {
                for path in &changed {
                    log("ok", &format!("✓ repaired {}", path));
                }
                for marketplace in local_sources.keys() {
                    if changed.iter().any(|path| path.contains(marketplace)) {
                        provisioned.push(marketplace.clone());
                    }
                }
                config_repairs.extend(changed);
            }
            Err(error) => {
                log(
                    "error",
                    &format!("Managed marketplace config repair failed: {}", error),
                );
                attempted_failures.push("managed marketplace config".to_string());
                for marketplace in [
                    ManagedMarketplace::Bundled,
                    ManagedMarketplace::PrimaryRuntime,
                ] {
                    if !local_sources.contains_key(marketplace.name()) {
                        continue;
                    }
                    unavailable.insert(marketplace);
                    let catalog_issue = issue(
                        marketplace.name(),
                        "managed_catalog_provision_failed",
                        format!(
                            "Could not bind managed marketplace '{}': {}",
                            marketplace.name(),
                            error
                        ),
                    );
                    block_managed_dependents(intents, marketplace, &catalog_issue, &mut blocked);
                }
            }
        }
    }

    match fetch_marketplaces(target_runner) {
        Ok(target_marketplaces) => {
            for marketplace in &required {
                if unavailable.contains(marketplace) {
                    continue;
                }
                let expected = &discovery.sources[marketplace];
                if !target_inventory_matches(
                    *marketplace,
                    expected,
                    &target_marketplaces,
                    target_home,
                ) {
                    let catalog_issue = issue(
                        marketplace.name(),
                        "managed_catalog_source_mismatch",
                        format!(
                            "Target inventory did not expose '{}' from the validated source",
                            marketplace.name()
                        ),
                    );
                    log("error", &catalog_issue.message);
                    attempted_failures.push(format!("marketplace {}", marketplace.name()));
                    unavailable.insert(*marketplace);
                    block_managed_dependents(intents, *marketplace, &catalog_issue, &mut blocked);
                }
            }
        }
        Err(error) => {
            log(
                "error",
                &format!("Target marketplace verification failed: {}", error),
            );
            attempted_failures.push("managed marketplace verification".to_string());
            for marketplace in &required {
                if unavailable.insert(*marketplace) {
                    let catalog_issue = issue(
                        marketplace.name(),
                        "managed_catalog_provision_failed",
                        error.clone(),
                    );
                    block_managed_dependents(intents, *marketplace, &catalog_issue, &mut blocked);
                }
            }
        }
    }

    let runnable: Vec<CodexPluginIntent> = intents
        .iter()
        .filter(|intent| {
            intent
                .id
                .split_once('@')
                .and_then(|(_, name)| managed_marketplace(name))
                .is_none_or(|marketplace| !unavailable.contains(&marketplace))
        })
        .cloned()
        .collect();
    let mut report =
        match apply_plan_for_intents(target_runner, lock, &runnable, explicitly_disabled, log) {
            Ok(report) => report,
            Err(error) => {
                log("error", &format!("Codex plugin replay failed: {}", error));
                let mut failed = runnable
                    .iter()
                    .filter(|intent| !explicitly_disabled.contains(&intent.id))
                    .map(|intent| intent.id.clone())
                    .collect::<Vec<_>>();
                if failed.is_empty() {
                    failed.push("plugin replay".to_string());
                }
                CodexPluginRepairReport {
                    failed,
                    manual: lock
                        .manual
                        .iter()
                        .map(|entry| format!("{} — {}", entry.id, entry.reason))
                        .collect(),
                    ..CodexPluginRepairReport::default()
                }
            }
        };
    report.failed.extend(attempted_failures);
    report.blocked_plugins.extend(blocked);
    report.managed_marketplaces_provisioned.extend(provisioned);
    report.config_paths_repaired.extend(config_repairs);

    if needs_mcp_repair
        || [
            ManagedMarketplace::Bundled,
            ManagedMarketplace::PrimaryRuntime,
        ]
        .into_iter()
        .any(|marketplace| required.contains(&marketplace) && !unavailable.contains(&marketplace))
    {
        match codex_config::repair_managed_mcp_from_default(
            &target_home.join("config.toml"),
            &default_home.join("config.toml"),
            target_home,
        ) {
            Ok(changed) => {
                for path in &changed {
                    log("ok", &format!("✓ repaired {}", path));
                }
                report.config_paths_repaired.extend(changed);
            }
            Err(error) => {
                log(
                    "error",
                    &format!("Managed MCP config repair failed: {}", error),
                );
                report.failed.push("managed MCP config".to_string());
            }
        }
    }

    for unresolved in
        codex_config::inspect_managed_config_with_default(&target_config, target_home, default_home)
    {
        log(
            "error",
            &format!(
                "Codex config repair remains unresolved [{}]: {}",
                unresolved.code, unresolved.message
            ),
        );
        report.failed.push(format!("config {}", unresolved.id));
    }

    report.failed.sort();
    report.failed.dedup();
    report.blocked_plugins.sort();
    report.blocked_plugins.dedup();
    report.managed_marketplaces_provisioned.sort();
    report.managed_marketplaces_provisioned.dedup();
    report.config_paths_repaired.sort();
    report.config_paths_repaired.dedup();
    report.state = if !report.failed.is_empty() {
        CodexRepairState::Failed
    } else if !report.blocked_plugins.is_empty() || !report.manual.is_empty() {
        CodexRepairState::Partial
    } else {
        CodexRepairState::Ready
    };
    report.verified = report.state == CodexRepairState::Ready;
    Ok(report)
}

// ── Claude inventory (plain file reads, no CLI) ─────────────────────────────
//
// Claude's plugin inventory lives in files: settings.json carries the intent
// (`enabledPlugins`, `extraKnownMarketplaces`), the manager's
// known_marketplaces.json / installed_plugins.json carry sources and
// versions. Those manager files never sync (machine-local absolute paths);
// they are read-only inputs here.

fn read_optional_capture_json(path: &Path) -> Result<Option<serde_json::Value>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read '{}': {}", path.display(), error)),
    };
    if metadata.len() > MAX_LOCK_BYTES {
        return Err(format!(
            "Claude plugin capture input '{}' exceeds {} bytes",
            path.display(),
            MAX_LOCK_BYTES
        ));
    }
    let bytes = fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("parse '{}': {}", path.display(), error))
}

/// A portable location for a marketplace source object: GitHub `repo`
/// shorthand or a Git `url`. Local `path` sources return None.
fn claude_source_location(source: &serde_json::Value) -> Option<String> {
    source
        .get("repo")
        .or_else(|| source.get("url"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Claude's local plugin state in the shared `Inventory` shape, so both
/// agents diff identically. Presence = the manager's record points at an
/// existing install path (same rule the repair presence-checks use);
/// enabled = not explicitly disabled in settings.
pub fn claude_inventory(claude_dir: &Path) -> Result<Inventory, String> {
    let settings =
        read_optional_capture_json(&claude_dir.join("settings.json"))?.unwrap_or_default();
    let known = read_optional_capture_json(&claude_dir.join("plugins/known_marketplaces.json"))?
        .unwrap_or_default();
    let installed = read_optional_capture_json(&claude_dir.join("plugins/installed_plugins.json"))?
        .unwrap_or_default();

    let mut marketplaces = Vec::new();
    if let Some(map) = known.as_object() {
        for (name, entry) in map {
            let source = entry.get("source");
            let location = source.and_then(claude_source_location);
            let (source_type, source) = match location {
                Some(location) => ("git".to_string(), location),
                None => (
                    "local".to_string(),
                    // Path stays local to plan comparison; it never syncs.
                    source
                        .and_then(|s| s.get("path"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ),
            };
            marketplaces.push(MarketplaceInfo {
                name: name.clone(),
                root: String::new(),
                source_type,
                source,
                git_ref: None,
            });
        }
    }
    let enabled_map = settings.get("enabledPlugins").and_then(|v| v.as_object());
    let mut plugins = Vec::new();
    if let Some(map) = installed.get("plugins").and_then(|v| v.as_object()) {
        for (id, entries) in map {
            let entries = entries.as_array().cloned().unwrap_or_default();
            let present = entries.iter().any(|e| {
                e.get("installPath")
                    .and_then(|v| v.as_str())
                    .is_some_and(|p| Path::new(p).exists())
            });
            let version = entries
                .iter()
                .find_map(|e| e.get("version").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            let enabled = enabled_map
                .and_then(|m| m.get(id))
                .and_then(|v| v.as_bool())
                != Some(false);
            plugins.push(PluginInfo {
                id: id.clone(),
                marketplace: id.split_once('@').map(|(_, m)| m).unwrap_or("").to_string(),
                version,
                installed: present,
                enabled,
            });
        }
    }
    Ok(Inventory {
        marketplaces,
        plugins,
    })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Recorded responses keyed by the joined argument list; the last
    /// response for a key repeats, so evolving state is scripted by pushing
    /// several responses. Records every invocation for order assertions.
    struct FakeRunner {
        responses: RefCell<HashMap<String, Vec<Result<(bool, String), String>>>>,
        calls: RefCell<Vec<String>>,
        config_path: RefCell<Option<PathBuf>>,
    }

    impl FakeRunner {
        fn new() -> FakeRunner {
            FakeRunner {
                responses: RefCell::new(HashMap::new()),
                calls: RefCell::new(Vec::new()),
                config_path: RefCell::new(None),
            }
        }
        fn on(&self, args: &str, response: Result<(bool, String), String>) -> &Self {
            self.responses
                .borrow_mut()
                .entry(args.to_string())
                .or_default()
                .push(response);
            self
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn use_config(&self, path: PathBuf) -> &Self {
            *self.config_path.borrow_mut() = Some(path);
            self
        }
    }

    impl CodexRunner for FakeRunner {
        fn run(&self, args: &[&str]) -> Result<CmdOutput, String> {
            let key = args.join(" ");
            self.calls.borrow_mut().push(key.clone());
            let mut responses = self.responses.borrow_mut();
            let queue = responses
                .get_mut(&key)
                .unwrap_or_else(|| panic!("unexpected codex invocation: {}", key));
            let response = if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue[0].clone()
            };
            response.map(|(success, output)| CmdOutput {
                success,
                stdout: if success {
                    output.clone()
                } else {
                    String::new()
                },
                stderr: if success { String::new() } else { output },
            })
        }

        fn config_path(&self) -> Option<PathBuf> {
            self.config_path.borrow().clone()
        }
    }

    fn marketplaces_json(entries: &[(&str, &str, &str, Option<&str>)]) -> String {
        let list: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, source_type, source, git_ref)| {
                let mut src = serde_json::json!({"sourceType": source_type, "source": source});
                if let Some(r) = git_ref {
                    src["ref"] = serde_json::json!(r);
                }
                serde_json::json!({"name": name, "root": "/tmp/x", "marketplaceSource": src})
            })
            .collect();
        serde_json::json!({ "marketplaces": list }).to_string()
    }

    fn marketplaces_json_with_roots(entries: &[(&str, &Path, Option<(&str, &Path)>)]) -> String {
        let list: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, root, source)| {
                let mut value = serde_json::json!({
                    "name": name,
                    "root": root.to_string_lossy(),
                });
                if let Some((source_type, source)) = source {
                    value["marketplaceSource"] = serde_json::json!({
                        "sourceType": source_type,
                        "source": source.to_string_lossy(),
                    });
                }
                value
            })
            .collect();
        serde_json::json!({ "marketplaces": list }).to_string()
    }

    fn plugins_json(entries: &[(&str, &str, &str, bool, bool)]) -> String {
        let list: Vec<serde_json::Value> = entries
            .iter()
            .map(|(id, marketplace, version, installed, enabled)| {
                serde_json::json!({
                    "pluginId": id,
                    "marketplaceName": marketplace,
                    "version": version,
                    "installed": installed,
                    "enabled": enabled,
                    "source": {"source": "local", "path": "/Users/a/.codex/plugins/cache/x"}
                })
            })
            .collect();
        serde_json::json!({ "installed": list }).to_string()
    }

    fn sample_inventory() -> Inventory {
        parse_inventory(
            &marketplaces_json(&[
                (
                    "openai-primary-runtime",
                    "local",
                    "/Users/a/.cache/runtime",
                    None,
                ),
                (
                    "openai-bundled",
                    "local",
                    "/Users/a/.codex/.tmp/bundled",
                    None,
                ),
                ("openai-curated", "", "", None),
                ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
                ("personal-local", "local", "/Users/a/marketplace", None),
            ]),
            &plugins_json(&[
                (
                    "pdf@openai-primary-runtime",
                    "openai-primary-runtime",
                    "26.1",
                    true,
                    true,
                ),
                (
                    "chrome@openai-bundled",
                    "openai-bundled",
                    "26.1",
                    true,
                    true,
                ),
                (
                    "linter@openai-curated",
                    "openai-curated",
                    "2.0.0",
                    true,
                    true,
                ),
                ("my-plugin@team-tools", "team-tools", "1.4.2", true, true),
                ("disabled@team-tools", "team-tools", "1.0.0", true, false),
                (
                    "local-helper@personal-local",
                    "personal-local",
                    "0.1.0",
                    true,
                    true,
                ),
            ]),
        )
        .unwrap()
    }

    #[test]
    fn plan_classifies_missing_present_drift_disabled_and_mismatch() {
        let lock = CodexPluginLock {
            schema: 1,
            marketplaces: vec![
                CodexMarketplaceIntent {
                    name: "team-tools".into(),
                    repository: "owner/repo".into(),
                    git_ref: Some("4f2c0d9".into()),
                },
                CodexMarketplaceIntent {
                    name: "spoofed".into(),
                    repository: "owner/other".into(),
                    git_ref: None,
                },
                CodexMarketplaceIntent {
                    name: "new-market".into(),
                    repository: "owner/new".into(),
                    git_ref: Some("abc".into()),
                },
            ],
            plugins: vec![
                CodexPluginIntent {
                    id: "my-plugin@team-tools".into(),
                    observed_version: Some("9.9.9".into()),
                },
                CodexPluginIntent {
                    id: "disabled@team-tools".into(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "gone@team-tools".into(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "evil@spoofed".into(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "fresh@new-market".into(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "orphan@nowhere".into(),
                    observed_version: None,
                },
            ],
            manual: vec![CodexManualEntry {
                id: "local-helper@personal-local".into(),
                reason: "local".into(),
            }],
            ..CodexPluginLock::default()
        };
        let mut inventory = sample_inventory();
        inventory.marketplaces.push(MarketplaceInfo {
            name: "spoofed".into(),
            root: "/Users/b/evil".into(),
            source_type: "local".into(),
            source: "/Users/b/evil".into(),
            git_ref: None,
        });
        let plan = build_plan_for_intents(&lock, &lock.plugins, &inventory, &BTreeSet::new());
        assert_eq!(plan.missing_marketplaces, ["new-market"]);
        assert_eq!(
            plan.missing_plugins,
            ["gone@team-tools", "fresh@new-market"]
        );
        assert_eq!(plan.present, ["my-plugin@team-tools"]);
        assert_eq!(plan.disabled, ["disabled@team-tools"]);
        assert_eq!(plan.drift.len(), 1);
        assert!(plan.drift[0].contains("9.9.9") && plan.drift[0].contains("1.4.2"));
        assert_eq!(plan.manual.len(), 1);
        // Spoofed and missing marketplaces produce structured blocked items.
        assert!(
            plan.blocked_plugins
                .iter()
                .any(|entry| entry.id == "evil@spoofed"
                    && entry.code == "marketplace_source_mismatch")
        );
        assert!(plan
            .blocked_plugins
            .iter()
            .any(|entry| entry.id == "orphan@nowhere" && entry.code == "marketplace_missing"));
        assert!(!plan.missing_plugins.iter().any(|p| p.contains("evil")));
    }

    #[test]
    fn plan_blocks_a_git_marketplace_with_a_different_ref() {
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            marketplaces: vec![CodexMarketplaceIntent {
                name: "team-tools".into(),
                repository: "owner/repo".into(),
                git_ref: Some("expected-commit".into()),
            }],
            plugins: vec![CodexPluginIntent {
                id: "tool@team-tools".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };
        let inventory = Inventory {
            marketplaces: vec![MarketplaceInfo {
                name: "team-tools".into(),
                root: "/tmp/team-tools".into(),
                source_type: "git".into(),
                source: "owner/repo".into(),
                git_ref: Some("different-commit".into()),
            }],
            plugins: Vec::new(),
        };

        let plan = build_plan_for_intents(&lock, &lock.plugins, &inventory, &BTreeSet::new());

        assert!(plan.missing_plugins.is_empty());
        assert!(plan.blocked_plugins.iter().any(|entry| {
            entry.id == "tool@team-tools" && entry.code == "marketplace_source_mismatch"
        }));
    }

    #[test]
    fn missing_managed_marketplace_is_structured_and_not_silently_skipped() {
        let lock = CodexPluginLock {
            schema: 1,
            plugins: vec![CodexPluginIntent {
                id: "sites@openai-bundled".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };
        let plan =
            build_plan_for_intents(&lock, &lock.plugins, &Inventory::default(), &BTreeSet::new());
        assert!(plan.missing_plugins.is_empty());
        assert_eq!(plan.missing_managed_marketplaces.len(), 1);
        assert_eq!(plan.missing_managed_marketplaces[0].id, "openai-bundled");
        assert_eq!(plan.blocked_plugins.len(), 1);
        assert_eq!(plan.blocked_plugins[0].id, "sites@openai-bundled");
        assert_eq!(plan.blocked_plugins[0].code, "managed_catalog_missing");
    }

    #[test]
    fn explicit_false_prevents_install_even_when_payload_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(
            &config,
            "[plugins.\"linter@openai-curated\"]\nenabled = false\n",
        )
        .unwrap();
        let fake = FakeRunner::new();
        fake.use_config(config);
        fake.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[("openai-curated", "", "", None)]))),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            plugins: vec![CodexPluginIntent {
                id: "linter@openai-curated".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };

        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &lock, &mut log).unwrap();

        assert_eq!(report.state, CodexRepairState::Ready);
        assert_eq!(report.already_present, ["linter@openai-curated"]);
        assert!(!fake
            .calls()
            .iter()
            .any(|call| call.starts_with("plugin add")));
    }

    fn seed_curated_catalog(home: &Path, oid: &str) -> ManagedSource {
        let root = home.join(".tmp/plugins");
        fs::create_dir_all(root.join(".agents/plugins")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"openai-curated","plugins":[]}"#,
        )
        .unwrap();
        fs::write(root.join(".git/HEAD"), format!("{}\n", oid)).unwrap();
        let sha = home.join(".tmp/plugins.sha");
        fs::write(&sha, format!("{}\n", oid)).unwrap();
        ManagedSource {
            root,
            sha_sidecar: Some(sha),
        }
    }

    #[test]
    fn curated_catalog_copy_is_validated_and_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "0123456789abcdef0123456789abcdef01234567");
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home.clone());
        assert!(resolver.provision_curated(&source).unwrap());
        assert!(validate_curated_pair(
            &target_home.join(".tmp/plugins"),
            &target_home.join(".tmp/plugins.sha")
        )
        .is_ok());
        assert!(!resolver.provision_curated(&source).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn curated_catalog_rejects_even_marked_symlink_targets() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "13579bdf2468ace013579bdf2468ace013579bdf");
        let target_tmp = target_home.join(".tmp");
        fs::create_dir_all(&target_tmp).unwrap();
        symlink(&source.root, target_tmp.join("plugins")).unwrap();
        symlink(
            source.sha_sidecar.as_ref().unwrap(),
            target_tmp.join("plugins.sha"),
        )
        .unwrap();
        fs::write(
            target_tmp.join("plugins.agent-sync-owned"),
            "managed by Agent Sync\n",
        )
        .unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home.clone());

        let error = resolver.provision_curated(&source).unwrap_err();

        assert!(error.contains("traverses symlink"), "{error}");
        assert!(fs::symlink_metadata(target_tmp.join("plugins"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::symlink_metadata(target_tmp.join("plugins.sha"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn managed_catalog_rejects_symlinked_approved_root_ancestors() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let backing_home = dir.path().join("backing/.codex");
        let source =
            seed_curated_catalog(&backing_home, "2468ace013579bdf2468ace013579bdf2468ace0");
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        fs::create_dir_all(&default_home).unwrap();
        symlink(backing_home.join(".tmp"), default_home.join(".tmp")).unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home.clone(), target_home);
        let info = MarketplaceInfo {
            name: CURATED_MARKETPLACE.into(),
            root: default_home
                .join(".tmp/plugins")
                .to_string_lossy()
                .into_owned(),
            source_type: String::new(),
            source: String::new(),
            git_ref: None,
        };

        let error = resolver
            .validate_source(ManagedMarketplace::Curated, &info)
            .unwrap_err();

        assert!(error.contains("traverses symlink"), "{error}");
        assert!(source.root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn curated_provisioning_rejects_symlinked_target_tmp_ancestor() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let external_tmp = dir.path().join("outside-target");
        let source =
            seed_curated_catalog(&default_home, "13579bdf2468ace013579bdf2468ace013579bdf");
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&external_tmp).unwrap();
        symlink(&external_tmp, target_home.join(".tmp")).unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home);

        let error = resolver.provision_curated(&source).unwrap_err();

        assert!(error.contains("traverses symlink"), "{error}");
        assert!(!external_tmp.join("plugins").exists());
        assert!(!external_tmp.join("plugins.sha").exists());
    }

    #[cfg(unix)]
    #[test]
    fn curated_catalog_repairs_nested_symlinks_before_accepting_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "abcdefabcdefabcdefabcdefabcdefabcdefabcd");
        seed_curated_catalog(&target_home, "abcdefabcdefabcdefabcdefabcdefabcdefabcd");
        symlink(
            &source.root,
            target_home.join(".tmp/plugins/nested-catalog-link"),
        )
        .unwrap();
        fs::write(
            target_home.join(".tmp/plugins.agent-sync-owned"),
            CURATED_OWNER_MARKER,
        )
        .unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home.clone());

        assert!(resolver.provision_curated(&source).unwrap());
        assert!(validate_curated_pair(
            &target_home.join(".tmp/plugins"),
            &target_home.join(".tmp/plugins.sha")
        )
        .is_ok());
        assert!(!target_home
            .join(".tmp/plugins/nested-catalog-link")
            .exists());
    }

    #[test]
    fn curated_catalog_never_replaces_an_unknown_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "89abcdef0123456789abcdef0123456789abcdef");
        fs::create_dir_all(target_home.join(".tmp/plugins")).unwrap();
        fs::write(target_home.join(".tmp/plugins/unknown"), "keep").unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home.clone());
        let error = resolver.provision_curated(&source).unwrap_err();
        assert!(error.contains("Refusing to replace unowned"), "{}", error);
        assert_eq!(
            fs::read_to_string(target_home.join(".tmp/plugins/unknown")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn curated_catalog_never_replaces_an_unowned_catalog_shaped_directory() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "89abcdef0123456789abcdef0123456789abcdef");
        let target = seed_curated_catalog(&target_home, "0123456789abcdef0123456789abcdef01234567");
        fs::write(
            target.sha_sidecar.as_ref().unwrap(),
            "ffffffffffffffffffffffffffffffffffffffff\n",
        )
        .unwrap();
        let resolver = ManagedMarketplaceResolver::new(default_home, target_home.clone());

        let error = resolver.provision_curated(&source).unwrap_err();

        assert!(error.contains("Refusing to replace unowned"), "{error}");
        assert_eq!(
            read_git_head(&target_home.join(".tmp/plugins")).unwrap(),
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert!(!target_home.join(".tmp/plugins.agent-sync-owned").exists());
    }

    #[test]
    fn managed_repair_bootstraps_curated_for_a_custom_home_and_converges() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "fedcba9876543210fedcba9876543210fedcba98");
        fs::create_dir_all(&target_home).unwrap();
        fs::write(default_home.join("config.toml"), "model = 'gpt'\n").unwrap();
        let target_config = target_home.join("config.toml");
        fs::write(
            &target_config,
            "[marketplaces.openai-curated]\nsource_type = 'local'\nsource = '/stale/.codex/.tmp/plugins'\n\
             [plugins.\"slack@openai-curated\"]\nenabled = true\n",
        )
        .unwrap();
        let lock = CodexPluginLock {
            schema: 1,
            plugins: vec![CodexPluginIntent {
                id: "slack@openai-curated".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };

        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json_with_roots(&[(CURATED_MARKETPLACE, &source.root, None)]),
            )),
        );
        let target_runner = FakeRunner::new();
        target_runner.use_config(target_config);
        let target_curated = target_home.join(".tmp/plugins");
        target_runner.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json_with_roots(&[(CURATED_MARKETPLACE, &target_curated, None)]),
            )),
        );
        target_runner.on("plugin list --json", Ok((true, plugins_json(&[]))));
        target_runner.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[(
                    "slack@openai-curated",
                    CURATED_MARKETPLACE,
                    "1.0.0",
                    true,
                    true,
                )]),
            )),
        );
        target_runner.on("plugin add slack@openai-curated", Ok((true, String::new())));

        let mut log = |_: &str, _: &str| {};
        let first = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();
        assert_eq!(first.state, CodexRepairState::Ready);
        assert_eq!(
            first.managed_marketplaces_provisioned,
            [CURATED_MARKETPLACE]
        );
        assert_eq!(first.plugins_installed, ["slack@openai-curated"]);
        assert!(
            validate_curated_pair(&target_curated, &target_home.join(".tmp/plugins.sha")).is_ok()
        );
        assert!(!fs::read_to_string(target_home.join("config.toml"))
            .unwrap()
            .contains("marketplaces.openai-curated"));

        let calls_before = target_runner.calls().len();
        let second = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();
        assert_eq!(second.state, CodexRepairState::Ready);
        assert!(second.managed_marketplaces_provisioned.is_empty());
        assert!(second.plugins_installed.is_empty());
        assert!(!target_runner.calls()[calls_before..]
            .iter()
            .any(|call| call.starts_with("plugin add")));
    }

    #[test]
    fn approved_managed_plan_does_not_install_unapproved_config_intent() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        fs::create_dir_all(&default_home).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let target_config = target_home.join("config.toml");
        fs::write(
            &target_config,
            "[plugins.\"unapproved@openai-curated\"]\nenabled = true\n",
        )
        .unwrap();
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            marketplaces: vec![CodexMarketplaceIntent {
                name: "team-tools".to_string(),
                repository: "owner/repo".to_string(),
                git_ref: None,
            }],
            plugins: vec![CodexPluginIntent {
                id: "approved@team-tools".to_string(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };

        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[]))),
        );
        let target_runner = FakeRunner::new();
        target_runner.use_config(target_config);
        target_runner.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json(&[("team-tools", "git", "owner/repo", None)]),
            )),
        );
        target_runner.on("plugin list --json", Ok((true, plugins_json(&[]))));
        target_runner.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[("approved@team-tools", "team-tools", "1.0.0", true, true)]),
            )),
        );
        target_runner.on("plugin add approved@team-tools", Ok((true, String::new())));

        let mut log = |_: &str, _: &str| {};
        let report = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();

        assert_eq!(report.state, CodexRepairState::Ready);
        assert_eq!(report.plugins_installed, ["approved@team-tools"]);
        assert!(target_runner
            .calls()
            .contains(&"plugin add approved@team-tools".to_string()));
        assert!(!target_runner
            .calls()
            .iter()
            .any(|call| call.contains("unapproved@openai-curated")));
    }

    #[test]
    fn approved_plan_rebinds_primary_runtime_before_installing_documents() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let runtime = default_home
            .parent()
            .unwrap()
            .join(".cache/codex-runtimes/runtime-1");
        fs::create_dir_all(runtime.join(".agents/plugins")).unwrap();
        fs::create_dir_all(&default_home).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        fs::write(
            runtime.join(".agents/plugins/marketplace.json"),
            r#"{"name":"openai-primary-runtime","plugins":[]}"#,
        )
        .unwrap();
        fs::write(default_home.join("config.toml"), "model = 'gpt'\n").unwrap();
        let target_config = target_home.join("config.toml");
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            plugins: vec![CodexPluginIntent {
                id: "documents@openai-primary-runtime".to_string(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };

        let primary_inventory = marketplaces_json_with_roots(&[(
            PRIMARY_RUNTIME_MARKETPLACE,
            &runtime,
            Some(("local", &runtime)),
        )]);
        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((true, primary_inventory.clone())),
        );
        let target_runner = FakeRunner::new();
        target_runner.use_config(target_config.clone());
        target_runner.on(
            "plugin marketplace list --json",
            Ok((true, primary_inventory)),
        );
        target_runner.on("plugin list --json", Ok((true, plugins_json(&[]))));
        target_runner.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[(
                    "documents@openai-primary-runtime",
                    PRIMARY_RUNTIME_MARKETPLACE,
                    "1.0.0",
                    true,
                    true,
                )]),
            )),
        );
        target_runner.on(
            "plugin add documents@openai-primary-runtime",
            Ok((true, String::new())),
        );

        let mut messages = Vec::new();
        let mut log = |level: &str, message: &str| {
            messages.push(format!("{}: {}", level, message));
        };
        let report = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();

        assert_eq!(
            report.state,
            CodexRepairState::Ready,
            "report: {:?}; logs: {:?}",
            report,
            messages
        );
        assert_eq!(
            report.plugins_installed,
            ["documents@openai-primary-runtime"]
        );
        assert_eq!(
            report.managed_marketplaces_provisioned,
            [PRIMARY_RUNTIME_MARKETPLACE]
        );
        let repaired = fs::read_to_string(target_config).unwrap();
        assert!(repaired.contains("marketplaces.openai-primary-runtime"));
        assert!(repaired.contains(runtime.to_string_lossy().as_ref()));
    }

    #[test]
    fn managed_repair_fixes_stale_mcp_config_without_plugin_intent() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let runtime = dir.path().join("runtime");
        fs::create_dir_all(&default_home).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("node_repl"), "runtime").unwrap();
        fs::write(runtime.join("node"), "runtime").unwrap();
        fs::write(
            default_home.join("config.toml"),
            format!(
                "[mcp_servers.node_repl]\ncommand = {:?}\n\n[mcp_servers.node_repl.env]\nCODEX_HOME = {:?}\nNODE_REPL_TRUSTED_CODE_PATHS = {:?}\nNODE_REPL_NODE_PATH = {:?}\n",
                runtime.join("node_repl").to_string_lossy(),
                default_home.to_string_lossy(),
                default_home.to_string_lossy(),
                runtime.join("node").to_string_lossy(),
            ),
        )
        .unwrap();
        let target_config = target_home.join("config.toml");
        fs::write(
            &target_config,
            format!(
                "[mcp_servers.node_repl]\ncommand = {:?}\n\n[mcp_servers.node_repl.env]\nCODEX_HOME = \"/old/.codex\"\nNODE_REPL_TRUSTED_CODE_PATHS = \"/old/.codex\"\nNODE_REPL_NODE_PATH = {:?}\n",
                runtime.join("node_repl").to_string_lossy(),
                runtime.join("node").to_string_lossy(),
            ),
        )
        .unwrap();
        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json_with_roots(&[]))),
        );
        let target_runner = FakeRunner::new();
        target_runner.use_config(target_config.clone());
        target_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json_with_roots(&[]))),
        );
        target_runner.on("plugin list --json", Ok((true, plugins_json(&[]))));

        let mut log = |_: &str, _: &str| {};
        let report = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &empty_lock(),
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();

        assert_eq!(report.state, CodexRepairState::Ready);
        assert!(report
            .config_paths_repaired
            .contains(&"mcp_servers.node_repl".to_string()));
        assert!(codex_config::inspect_managed_config_with_default(
            &target_config,
            &target_home,
            &default_home
        )
        .is_empty());
    }

    #[test]
    fn invalid_default_managed_catalog_blocks_without_installing() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        let source =
            seed_curated_catalog(&default_home, "abcdef0123456789abcdef0123456789abcdef01");
        fs::write(
            source.root.join(".agents/plugins/marketplace.json"),
            r#"{"name":"not-openai-curated","plugins":[]}"#,
        )
        .unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let target_config = target_home.join("config.toml");
        fs::write(
            &target_config,
            "[plugins.\"slack@openai-curated\"]\nenabled = true\n",
        )
        .unwrap();
        let lock = CodexPluginLock {
            schema: 1,
            plugins: vec![CodexPluginIntent {
                id: "slack@openai-curated".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };
        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json_with_roots(&[(CURATED_MARKETPLACE, &source.root, None)]),
            )),
        );
        let target_runner = FakeRunner::new();
        target_runner.use_config(target_config);
        target_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json_with_roots(&[]))),
        );
        target_runner.on("plugin list --json", Ok((true, plugins_json(&[]))));

        let mut log = |_: &str, _: &str| {};
        let report = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.state, CodexRepairState::Partial);
        assert!(!report.verified);
        assert!(report.failed.is_empty());
        assert!(report.blocked_plugins.iter().any(|entry| {
            entry.id == "slack@openai-curated" && entry.code == "managed_catalog_invalid"
        }));
        assert!(!target_runner
            .calls()
            .iter()
            .any(|call| call.starts_with("plugin add")));
    }

    #[test]
    fn managed_replay_inventory_failure_marks_every_unresolved_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let default_home = dir.path().join("default/.codex");
        let target_home = dir.path().join("target/.codex");
        fs::create_dir_all(&default_home).unwrap();
        fs::create_dir_all(&target_home).unwrap();
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            marketplaces: vec![CodexMarketplaceIntent {
                name: "team-tools".to_string(),
                repository: "owner/repo".to_string(),
                git_ref: None,
            }],
            plugins: vec![
                CodexPluginIntent {
                    id: "one@team-tools".to_string(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "two@team-tools".to_string(),
                    observed_version: None,
                },
            ],
            ..CodexPluginLock::default()
        };
        let default_runner = FakeRunner::new();
        default_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[]))),
        );
        let target_runner = FakeRunner::new();
        target_runner.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[]))),
        );
        target_runner.on(
            "plugin list --json",
            Ok((false, "inventory unavailable".to_string())),
        );

        let mut log = |_: &str, _: &str| {};
        let report = apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        )
        .unwrap();

        assert_eq!(report.state, CodexRepairState::Failed);
        assert_eq!(report.failed, ["one@team-tools", "two@team-tools"]);
        assert!(!report.verified);
    }

    fn repair_lock() -> CodexPluginLock {
        CodexPluginLock {
            schema: 1,
            marketplaces: vec![CodexMarketplaceIntent {
                name: "team-tools".into(),
                repository: "owner/repo".into(),
                git_ref: Some("4f2c0d9".into()),
            }],
            plugins: vec![
                CodexPluginIntent {
                    id: "linter@openai-curated".into(),
                    observed_version: None,
                },
                CodexPluginIntent {
                    id: "my-plugin@team-tools".into(),
                    observed_version: None,
                },
            ],
            ..CodexPluginLock::default()
        }
    }

    const BROKEN_TEAM_TOOLS: &str = "Error: failed to load marketplace(s):\n- `team-tools` at /tmp/codex/.tmp/marketplaces/team-tools: marketplace root does not contain a supported manifest";

    fn write_codex_config(dir: &Path, repository: &str, git_ref: Option<&str>) -> PathBuf {
        let config_path = dir.join("config.toml");
        let ref_line = git_ref
            .map(|value| format!("ref = {:?}\n", value))
            .unwrap_or_default();
        fs::write(
            &config_path,
            format!(
                "[marketplaces.team-tools]\nsource_type = \"git\"\nsource = {:?}\n{}",
                repository, ref_line
            ),
        )
        .unwrap();
        config_path
    }

    fn empty_target(fake: &FakeRunner) {
        fake.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[("openai-curated", "", "", None)]))),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
    }

    fn apply_plan(
        runner: &dyn CodexRunner,
        lock: &CodexPluginLock,
        log: &mut dyn FnMut(&str, &str),
    ) -> Result<CodexPluginRepairReport, String> {
        validate_lock(lock)?;
        let explicitly_disabled = explicitly_disabled_plugin_ids(runner.config_path().as_deref())?;
        apply_plan_for_intents(runner, lock, &lock.plugins, &explicitly_disabled, log)
    }

    #[test]
    fn apply_adds_marketplace_before_plugins_and_verifies() {
        let fake = FakeRunner::new();
        empty_target(&fake);
        // After the add, team-tools appears; after installs, plugins appear.
        let with_marketplace = marketplaces_json(&[
            ("openai-curated", "", "", None),
            ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
        ]);
        fake.on(
            "plugin marketplace list --json",
            Ok((true, with_marketplace)),
        );
        fake.on(
            "plugin marketplace add owner/repo --ref 4f2c0d9",
            Ok((true, String::new())),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((true, String::new())),
        );
        fake.on("plugin add my-plugin@team-tools", Ok((true, String::new())));
        fake.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[
                    (
                        "linter@openai-curated",
                        "openai-curated",
                        "2.0.0",
                        true,
                        true,
                    ),
                    ("my-plugin@team-tools", "team-tools", "1.4.2", true, true),
                ]),
            )),
        );
        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();
        assert_eq!(report.marketplaces_added, ["team-tools"]);
        assert_eq!(
            report.plugins_installed,
            ["linter@openai-curated", "my-plugin@team-tools"]
        );
        assert!(report.failed.is_empty());
        assert!(report.verified);
        assert_eq!(report.state, CodexRepairState::Ready);
        let calls = fake.calls();
        let add_marketplace = calls
            .iter()
            .position(|c| c.starts_with("plugin marketplace add"))
            .unwrap();
        let first_plugin_add = calls
            .iter()
            .position(|c| c.starts_with("plugin add"))
            .unwrap();
        assert!(
            add_marketplace < first_plugin_add,
            "marketplace must install first"
        );
        // Arguments are passed as a vector, never through a shell.
        assert!(calls.contains(&"plugin marketplace add owner/repo --ref 4f2c0d9".to_string()));
    }

    #[test]
    fn final_verification_rejects_a_newly_installed_but_disabled_plugin() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[("openai-curated", "", "", None)]))),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        fake.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[(
                    "linter@openai-curated",
                    "openai-curated",
                    "2.0.0",
                    true,
                    false,
                )]),
            )),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((true, String::new())),
        );
        let lock = CodexPluginLock {
            schema: LOCK_SCHEMA,
            plugins: vec![CodexPluginIntent {
                id: "linter@openai-curated".into(),
                observed_version: None,
            }],
            ..CodexPluginLock::default()
        };

        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &lock, &mut log).unwrap();

        assert_eq!(report.state, CodexRepairState::Failed);
        assert!(!report.verified);
        assert_eq!(report.failed, ["linter@openai-curated"]);
    }

    #[test]
    fn final_inventory_failure_marks_every_unresolved_plugin() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json(&[
                    ("openai-curated", "", "", None),
                    ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
                ]),
            )),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        fake.on(
            "plugin list --json",
            Ok((false, "inventory unavailable".to_string())),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((true, String::new())),
        );
        fake.on("plugin add my-plugin@team-tools", Ok((true, String::new())));

        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();

        assert_eq!(report.state, CodexRepairState::Failed);
        assert_eq!(
            report.failed,
            ["linter@openai-curated", "my-plugin@team-tools"]
        );
        assert!(!report.verified);
    }

    #[test]
    fn broken_registered_clone_is_recreated_before_inventory_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeRunner::new();
        fake.use_config(write_codex_config(
            dir.path(),
            "owner/repo",
            Some("4f2c0d9"),
        ));
        fake.on(
            "plugin marketplace list --json",
            Ok((false, BROKEN_TEAM_TOOLS.into())),
        );
        fake.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json(&[
                    ("openai-curated", "", "", None),
                    ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
                ]),
            )),
        );
        fake.on(
            "plugin marketplace add owner/repo --ref 4f2c0d9",
            Ok((true, String::new())),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        fake.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[
                    (
                        "linter@openai-curated",
                        "openai-curated",
                        "2.0.0",
                        true,
                        true,
                    ),
                    ("my-plugin@team-tools", "team-tools", "1.4.2", true, true),
                ]),
            )),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((true, String::new())),
        );
        fake.on("plugin add my-plugin@team-tools", Ok((true, String::new())));

        let mut messages = Vec::new();
        let mut log = |_: &str, message: &str| messages.push(message.to_string());
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();

        assert!(report.verified);
        assert_eq!(report.plugins_installed.len(), 2);
        assert!(messages
            .iter()
            .any(|line| line.contains("Re-cloning registered marketplace")));
        let calls = fake.calls();
        assert_eq!(calls[0], "plugin marketplace list --json");
        assert_eq!(calls[1], "plugin marketplace add owner/repo --ref 4f2c0d9");
        assert_eq!(calls[2], "plugin marketplace list --json");
    }

    #[test]
    fn broken_clone_with_different_registered_source_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeRunner::new();
        fake.use_config(write_codex_config(
            dir.path(),
            "attacker/other",
            Some("4f2c0d9"),
        ));
        fake.on(
            "plugin marketplace list --json",
            Ok((false, BROKEN_TEAM_TOOLS.into())),
        );

        let mut log = |_: &str, _: &str| {};
        let error = apply_plan(&fake, &repair_lock(), &mut log).unwrap_err();
        assert!(error.contains("different source or ref"), "{}", error);
        assert!(!fake.calls().iter().any(|call| call.contains(" add ")));
    }

    #[test]
    fn unrelated_inventory_failure_never_triggers_marketplace_recovery() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((false, "Error: network unavailable".into())),
        );

        let mut log = |_: &str, _: &str| {};
        let error = apply_plan(&fake, &repair_lock(), &mut log).unwrap_err();
        assert!(error.contains("network unavailable"), "{}", error);
        assert!(!fake.calls().iter().any(|call| call.contains(" add ")));
    }

    #[test]
    fn broken_marketplace_absent_from_lock_prevents_partial_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeRunner::new();
        fake.use_config(write_codex_config(
            dir.path(),
            "owner/repo",
            Some("4f2c0d9"),
        ));
        fake.on(
            "plugin marketplace list --json",
            Ok((
                false,
                format!(
                    "{}\n- `untracked` at /tmp/untracked: marketplace root does not contain a supported manifest",
                    BROKEN_TEAM_TOOLS
                ),
            )),
        );

        let mut log = |_: &str, _: &str| {};
        let error = apply_plan(&fake, &repair_lock(), &mut log).unwrap_err();
        assert!(error.contains("absent from the synced lock"), "{}", error);
        assert!(!fake.calls().iter().any(|call| call.contains(" add ")));
    }

    #[test]
    fn apply_continues_after_one_failure_and_reports_unverified() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json(&[
                    ("openai-curated", "", "", None),
                    ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
                ]),
            )),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        fake.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[("my-plugin@team-tools", "team-tools", "1.4.2", true, true)]),
            )),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((false, "boom".into())),
        );
        fake.on("plugin add my-plugin@team-tools", Ok((true, String::new())));
        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();
        assert_eq!(report.failed, ["linter@openai-curated"]);
        assert_eq!(report.plugins_installed, ["my-plugin@team-tools"]);
        assert!(!report.verified);
        assert_eq!(report.state, CodexRepairState::Failed);
    }

    #[test]
    fn apply_skips_plugins_of_a_failed_marketplace() {
        let fake = FakeRunner::new();
        empty_target(&fake);
        fake.on(
            "plugin marketplace add owner/repo --ref 4f2c0d9",
            Ok((false, String::new())),
        );
        fake.on(
            "plugin add linter@openai-curated",
            Ok((true, String::new())),
        );
        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();
        assert!(report
            .failed
            .contains(&"marketplace team-tools".to_string()));
        assert!(report.failed.contains(&"my-plugin@team-tools".to_string()));
        assert!(!fake
            .calls()
            .contains(&"plugin add my-plugin@team-tools".to_string()));
    }

    #[test]
    fn apply_twice_is_a_no_op() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((
                true,
                marketplaces_json(&[
                    ("openai-curated", "", "", None),
                    ("team-tools", "git", "owner/repo", Some("4f2c0d9")),
                ]),
            )),
        );
        fake.on(
            "plugin list --json",
            Ok((
                true,
                plugins_json(&[
                    (
                        "linter@openai-curated",
                        "openai-curated",
                        "2.0.0",
                        true,
                        true,
                    ),
                    ("my-plugin@team-tools", "team-tools", "1.4.2", true, true),
                ]),
            )),
        );
        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &repair_lock(), &mut log).unwrap();
        assert!(report.marketplaces_added.is_empty());
        assert!(report.plugins_installed.is_empty());
        assert_eq!(report.already_present.len(), 2);
        assert!(report.verified);
        assert_eq!(report.state, CodexRepairState::Ready);
        assert!(!fake
            .calls()
            .iter()
            .any(|c| c.contains(" add ") || c.starts_with("plugin add")));
    }

    #[test]
    fn manual_items_prevent_a_false_ready_report() {
        let fake = FakeRunner::new();
        fake.on(
            "plugin marketplace list --json",
            Ok((true, marketplaces_json(&[]))),
        );
        fake.on("plugin list --json", Ok((true, plugins_json(&[]))));
        let lock = CodexPluginLock {
            schema: 1,
            manual: vec![CodexManualEntry {
                id: "local@personal".into(),
                reason: "local source".into(),
            }],
            ..CodexPluginLock::default()
        };
        let mut log = |_: &str, _: &str| {};
        let report = apply_plan(&fake, &lock, &mut log).unwrap();
        assert_eq!(report.state, CodexRepairState::Partial);
        assert!(!report.verified);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn redact_masks_secretish_lines_and_timeout_kills_children() {
        assert_eq!(redact("Authorization: Bearer abc123"), "[redacted]");
        assert_eq!(redact("export API_KEY=xyz"), "[redacted]");
        assert_eq!(
            redact("cloning into plugins/foo"),
            "cloning into plugins/foo"
        );
        let runner = ProcessRunner {
            program: PathBuf::from("/bin/sleep"),
            timeout: Duration::from_millis(200),
            codex_home: None,
        };
        let err = runner.run(&["5"]).unwrap_err();
        assert!(err.contains("timed out"), "{}", err);
    }

}
