//! Compiled provider adapters: exactly `codex` and `claude`.
//!
//! Native contract evidence (frozen for this delivery, verified against
//! codex-cli 0.145.0 and Claude Code 2.1.220 plus the previously shipped
//! implementation archived under `docs/archive/skill-plugin-sync/`):
//!
//! - Codex inventory: `codex plugin marketplace list --json` returning
//!   `{"marketplaces":[{name, marketplaceSource:{sourceType, source, ref?},
//!   root?}]}` and `codex plugin list --json` returning
//!   `{"installed":[{pluginId, marketplaceName, version, installed,
//!   enabled}]}` under an explicit `CODEX_HOME`.
//! - Codex install/overwrite: `codex plugin marketplace add <SOURCE>` (a
//!   Git URL or `owner/repo` shorthand), `codex plugin marketplace upgrade`
//!   to refresh snapshots, then `codex plugin add PLUGIN@MARKETPLACE`. There
//!   is no plugin scope flag: `CODEX_HOME` pins the user-global home, and
//!   `plugin add` installs whatever the marketplace snapshot holds, so exact
//!   version requirements are not native-enforceable.
//! - Claude inventory: bounded reads of `plugins/known_marketplaces.json`,
//!   `plugins/installed_plugins.json`, and `settings.json` inside the
//!   selected `CLAUDE_CONFIG_DIR` (the user-global scope by location).
//! - Claude install/overwrite: `claude plugin install <id> --scope user` and
//!   `claude plugin update <id> --scope user`; marketplaces register with
//!   `claude plugin marketplace add <source> --scope user`. The `project`,
//!   `local`, and `managed` scopes are never used.
//!
//! Anything outside this evidence fails through a typed
//! [`CatalogBlocker::NativeContractUnverified`] rather than guessed syntax.

use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::domain::{
    provider_owns_kind, validate_canonical_plugin_id, validate_git_location, CatalogBlocker,
    CatalogResourceKindTag, GitMarketplaceSource, NativePluginPlan, PluginApplyMode,
    PluginInstallIntentDescriptor, PluginInstallIntentResource, PluginResolutionPolicy,
    PortablePluginSource, ProviderKey,
};

pub const MAX_CLI_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLUGIN_STATE_BYTES: u64 = 8 * 1024 * 1024;
const PLUGIN_PROCESS_TIMEOUT: Duration = Duration::from_secs(300);

/// Claude's synthetic marketplace for bare directories under
/// `<CLAUDE_CONFIG_DIR>/skills`. Entries there are exactly the standalone
/// skill class, not portable marketplace install intent, so the plugin
/// adapter reports them without offering them as catalog plugin resources
/// and without claiming ownership over the skills root.
pub const CLAUDE_SKILLS_DIR_NAMESPACE: &str = "skills-dir";

#[derive(Clone, Debug)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Structured, shell-free process execution. The runner is injectable so
/// tests never launch a real provider CLI.
pub trait PluginProcessRunner: Send + Sync {
    fn run(&self, provider: ProviderKey, home: &Path, argv: &[String])
        -> Result<CmdOutput, String>;
}

/// Launches the real provider CLI with an explicit home environment and the
/// home as working directory, so behavior never depends on where Mallard was
/// started.
pub struct NativeProcessRunner;

impl PluginProcessRunner for NativeProcessRunner {
    fn run(
        &self,
        provider: ProviderKey,
        home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String> {
        let program = mallard_infrastructure::codex_binary::find_binary(provider.as_str())
            .map_err(|error| format!("locate {}: {}", provider.as_str(), error))?;
        let mut command = std::process::Command::new(program);
        command
            .args(argv)
            .current_dir(home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        match provider {
            ProviderKey::Codex => {
                command.env("CODEX_HOME", home);
            }
            ProviderKey::Claude => {
                command.env("CLAUDE_CONFIG_DIR", home);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("start {}: {}", provider.as_str(), error))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_thread = std::thread::spawn(move || read_bounded_pipe(stdout));
        let stderr_thread = std::thread::spawn(move || read_bounded_pipe(stderr));
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started.elapsed() > PLUGIN_PROCESS_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("{} plugin operation timed out", provider.as_str()));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(format!("wait for {}: {}", provider.as_str(), error)),
            }
        };
        let stdout = stdout_thread.join().unwrap_or_default();
        let stderr = stderr_thread.join().unwrap_or_default();
        Ok(CmdOutput {
            success: status.success(),
            stdout,
            stderr,
        })
    }
}

fn read_bounded_pipe(pipe: Option<impl Read>) -> String {
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.take(MAX_CLI_BYTES as u64).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).to_string()
}

/// Redact credential-shaped lines before diagnostics travel anywhere.
pub fn redact(line: &str) -> String {
    const MARKERS: [&str; 7] = [
        "secret",
        "api_key",
        "apikey",
        "api key",
        "bearer",
        "authorization",
        "credential",
    ];
    let lower = line.to_ascii_lowercase();
    if MARKERS.iter().any(|marker| lower.contains(marker)) {
        "[redacted]".to_string()
    } else {
        line.to_string()
    }
}

/// Machine-local native inventory facts. These never enter portable intent
/// or its semantic digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginObservation {
    pub provider: ProviderKey,
    pub namespace: String,
    pub plugin_id: String,
    pub installed: bool,
    pub enabled: bool,
    pub observed_version: Option<String>,
    pub install_root: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketplaceObservation {
    pub name: String,
    pub source_type: String,
    pub source: String,
    pub git_ref: Option<String>,
    pub root: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderInventory {
    pub marketplaces: Vec<MarketplaceObservation>,
    pub plugins: Vec<PluginObservation>,
    pub warnings: Vec<String>,
}

impl ProviderInventory {
    pub fn marketplace(&self, name: &str) -> Option<&MarketplaceObservation> {
        self.marketplaces
            .iter()
            .find(|marketplace| marketplace.name == name)
    }

    pub fn observation(&self, plugin_id: &str) -> Option<&PluginObservation> {
        self.plugins
            .iter()
            .find(|observation| observation.plugin_id == plugin_id)
    }

    /// Canonical ownership roots: any skill directory canonically inside one
    /// of these belongs to a plugin, never to the standalone class.
    pub fn ownership_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self
            .marketplaces
            .iter()
            .filter_map(|marketplace| marketplace.root.clone())
            .chain(
                self.plugins
                    .iter()
                    .filter_map(|observation| observation.install_root.clone()),
            )
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginVerification {
    pub verified: bool,
    pub detail: String,
    pub observation: Option<PluginObservation>,
}

/// The compiled adapter contract. Tauri-free; the registry keys
/// dispatch by [`ProviderKey`], so the adapter carries no self-描述 key.
pub trait CatalogProviderAdapter: Send + Sync {
    fn discover_plugins(
        &self,
        home: &Path,
        runner: &dyn PluginProcessRunner,
    ) -> Result<ProviderInventory, CatalogBlocker>;

    /// Build portable install intent for one observation, or `None` when the
    /// observation is not a catalog plugin resource (unqualified identity or
    /// a non-marketplace pseudo-namespace).
    fn portable_intent(
        &self,
        observation: &PluginObservation,
        inventory: &ProviderInventory,
    ) -> Option<PluginInstallIntentResource>;

    /// Plan the exact native operation for an approved intent. Fails through
    /// a typed blocker when the native contract cannot express the intent.
    fn plan_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        currently_registered_marketplace: bool,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker>;

    /// Post-operation verification through fresh native inventory. A zero
    /// exit code with failed verification is `partial`, never success.
    fn verify_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        home: &Path,
        runner: &dyn PluginProcessRunner,
    ) -> Result<PluginVerification, CatalogBlocker>;
}

/// The compiled registry: exactly `codex` and `claude`.
pub fn adapter_for(key: ProviderKey) -> &'static dyn CatalogProviderAdapter {
    match key {
        ProviderKey::Codex => &CodexCatalogAdapter,
        ProviderKey::Claude => &ClaudeCatalogAdapter,
    }
}

/// The exact capability matrix served by `list_catalog_capabilities`,
/// kind-typed and derived from the single `provider_owns_kind` rule so the
/// served matrix can never drift from scope validation.
pub fn capability_matrix() -> Vec<(ProviderKey, Vec<CatalogResourceKindTag>)> {
    const ALL_KINDS: [CatalogResourceKindTag; 4] = [
        CatalogResourceKindTag::AgentSkill,
        CatalogResourceKindTag::PluginInstallIntent,
        CatalogResourceKindTag::CodexPetV1,
        CatalogResourceKindTag::CodexPetV2,
    ];
    [ProviderKey::Codex, ProviderKey::Claude]
        .into_iter()
        .map(|provider| {
            (
                provider,
                ALL_KINDS
                    .into_iter()
                    .filter(|kind| provider_owns_kind(provider, *kind))
                    .collect(),
            )
        })
        .collect()
}

fn portable_marketplace_source(
    marketplace: Option<&MarketplaceObservation>,
) -> (Option<GitMarketplaceSource>, PluginResolutionPolicy) {
    let Some(marketplace) = marketplace else {
        return (None, PluginResolutionPolicy::ManualOnly);
    };
    if marketplace.source_type == "git"
        && !marketplace.source.is_empty()
        && validate_git_location("plugin marketplace source", &marketplace.source).is_ok()
    {
        (
            Some(GitMarketplaceSource {
                location: marketplace.source.clone(),
                git_ref: marketplace.git_ref.clone(),
            }),
            PluginResolutionPolicy::ProviderResolved,
        )
    } else {
        (None, PluginResolutionPolicy::ManualOnly)
    }
}

fn intent_resource(
    provider: ProviderKey,
    observation: &PluginObservation,
    inventory: &ProviderInventory,
) -> Option<PluginInstallIntentResource> {
    validate_canonical_plugin_id("plugin id", &observation.plugin_id).ok()?;
    let (git, resolution) =
        portable_marketplace_source(inventory.marketplace(&observation.namespace));
    let descriptor = PluginInstallIntentDescriptor {
        provider,
        provider_native_namespace: observation.namespace.clone(),
        plugin_id: observation.plugin_id.clone(),
        source: PortablePluginSource::ProviderMarketplace {
            namespace: observation.namespace.clone(),
            git,
        },
        requirement: None,
        resolution,
    };
    let mut resource = PluginInstallIntentResource {
        descriptor,
        content_sha256: String::new(),
    };
    resource.content_sha256 = resource.compute_content_sha256();
    resource.validate().ok()?;
    Some(resource)
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

pub struct CodexCatalogAdapter;

#[derive(Deserialize, Default)]
struct CodexMarketplaceListDocument {
    #[serde(default)]
    marketplaces: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct CodexPluginListDocument {
    #[serde(default)]
    installed: Vec<serde_json::Value>,
}

impl CatalogProviderAdapter for CodexCatalogAdapter {
    fn discover_plugins(
        &self,
        home: &Path,
        runner: &dyn PluginProcessRunner,
    ) -> Result<ProviderInventory, CatalogBlocker> {
        let marketplace_argv = vec![
            "plugin".to_string(),
            "marketplace".to_string(),
            "list".to_string(),
            "--json".to_string(),
        ];
        let plugin_argv = vec![
            "plugin".to_string(),
            "list".to_string(),
            "--json".to_string(),
        ];
        let marketplaces = runner
            .run(ProviderKey::Codex, home, &marketplace_argv)
            .map_err(|detail| CatalogBlocker::NativeContractUnverified {
                provider: ProviderKey::Codex,
                operation: "plugin marketplace list".to_string(),
                detail,
            })?;
        if !marketplaces.success {
            return Err(CatalogBlocker::NativeContractUnverified {
                provider: ProviderKey::Codex,
                operation: "plugin marketplace list".to_string(),
                detail: redact(marketplaces.stderr.trim()),
            });
        }
        let plugins = runner
            .run(ProviderKey::Codex, home, &plugin_argv)
            .map_err(|detail| CatalogBlocker::NativeContractUnverified {
                provider: ProviderKey::Codex,
                operation: "plugin list".to_string(),
                detail,
            })?;
        if !plugins.success {
            return Err(CatalogBlocker::NativeContractUnverified {
                provider: ProviderKey::Codex,
                operation: "plugin list".to_string(),
                detail: redact(plugins.stderr.trim()),
            });
        }
        if marketplaces.stdout.len() > MAX_CLI_BYTES || plugins.stdout.len() > MAX_CLI_BYTES {
            return Err(CatalogBlocker::InventoryFailed {
                detail: "codex CLI output exceeds its size cap".to_string(),
            });
        }

        let mut inventory = ProviderInventory::default();
        let marketplace_document: CodexMarketplaceListDocument =
            serde_json::from_str(&marketplaces.stdout).map_err(|error| {
                CatalogBlocker::InventoryFailed {
                    detail: format!("parse codex marketplace inventory: {}", error),
                }
            })?;
        for entry in marketplace_document.marketplaces {
            // Tolerant per-entry parsing: one odd marketplace must not hide
            // the rest, but a skipped entry is reported.
            let Some(name) = entry.get("name").and_then(|value| value.as_str()) else {
                inventory
                    .warnings
                    .push("a Codex marketplace entry had no name and was skipped".to_string());
                continue;
            };
            let source = entry.get("marketplaceSource");
            inventory.marketplaces.push(MarketplaceObservation {
                name: name.to_string(),
                source_type: source
                    .and_then(|value| value.get("sourceType"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                source: source
                    .and_then(|value| value.get("source"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                git_ref: source
                    .and_then(|value| value.get("ref"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                root: entry
                    .get("root")
                    .and_then(|value| value.as_str())
                    .map(PathBuf::from),
            });
        }
        let plugin_document: CodexPluginListDocument = serde_json::from_str(&plugins.stdout)
            .map_err(|error| CatalogBlocker::InventoryFailed {
                detail: format!("parse codex plugin inventory: {}", error),
            })?;
        for entry in plugin_document.installed {
            let (Some(plugin), Some(marketplace)) = (
                entry.get("pluginId").and_then(|value| value.as_str()),
                entry
                    .get("marketplaceName")
                    .and_then(|value| value.as_str()),
            ) else {
                inventory
                    .warnings
                    .push("a Codex plugin entry had no id and was skipped".to_string());
                continue;
            };
            let installed = entry
                .get("installed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if !installed {
                // `--available` listings include uninstalled marketplace
                // plugins; only user-installed entries are catalog resources.
                continue;
            }
            inventory.plugins.push(PluginObservation {
                provider: ProviderKey::Codex,
                namespace: marketplace.to_string(),
                plugin_id: format!("{}@{}", plugin, marketplace),
                installed,
                enabled: entry
                    .get("enabled")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true),
                observed_version: entry
                    .get("version")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                install_root: None,
            });
        }
        Ok(inventory)
    }

    fn portable_intent(
        &self,
        observation: &PluginObservation,
        inventory: &ProviderInventory,
    ) -> Option<PluginInstallIntentResource> {
        intent_resource(ProviderKey::Codex, observation, inventory)
    }

    fn plan_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        currently_registered_marketplace: bool,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker> {
        let PortablePluginSource::ProviderMarketplace { git, .. } = &intent.source;
        let mut marketplace_argv = Vec::new();
        match git {
            Some(git) if !currently_registered_marketplace => {
                marketplace_argv.push(vec![
                    "plugin".to_string(),
                    "marketplace".to_string(),
                    "add".to_string(),
                    git.location.clone(),
                ]);
            }
            Some(_) => {
                if mode == PluginApplyMode::OverwriteExisting {
                    // Refresh registered Git snapshots so the overwrite
                    // installs the marketplace's current content.
                    marketplace_argv.push(vec![
                        "plugin".to_string(),
                        "marketplace".to_string(),
                        "upgrade".to_string(),
                    ]);
                }
            }
            None => {
                return Err(CatalogBlocker::NativeContractUnverified {
                    provider: ProviderKey::Codex,
                    operation: "plugin install".to_string(),
                    detail: format!(
                        "plugin '{}' has no portable marketplace source; install it manually",
                        intent.plugin_id
                    ),
                });
            }
        }
        Ok(NativePluginPlan {
            provider: ProviderKey::Codex,
            argv: vec![
                "plugin".to_string(),
                "add".to_string(),
                intent.plugin_id.clone(),
            ],
            marketplace_argv,
            mode,
        })
    }

    fn verify_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        home: &Path,
        runner: &dyn PluginProcessRunner,
    ) -> Result<PluginVerification, CatalogBlocker> {
        let inventory = self.discover_plugins(home, runner)?;
        let observation = inventory.observation(&intent.plugin_id).cloned();
        let verified = observation
            .as_ref()
            .is_some_and(|observation| observation.installed);
        Ok(PluginVerification {
            verified,
            detail: if verified {
                format!("codex reports '{}' installed", intent.plugin_id)
            } else {
                format!(
                    "codex does not report '{}' installed after the operation",
                    intent.plugin_id
                )
            },
            observation,
        })
    }
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

pub struct ClaudeCatalogAdapter;

fn read_optional_json(path: &Path) -> Result<Option<serde_json::Value>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect '{}': {}", path.display(), error)),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!("'{}' is not a regular file", path.display()));
    }
    if meta.len() > MAX_PLUGIN_STATE_BYTES {
        return Err(format!("'{}' exceeds its read limit", path.display()));
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("parse '{}': {}", path.display(), error))
}

fn claude_source_location(source: &serde_json::Value) -> Option<String> {
    source
        .get("repo")
        .or_else(|| source.get("url"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

impl CatalogProviderAdapter for ClaudeCatalogAdapter {
    fn discover_plugins(
        &self,
        home: &Path,
        _runner: &dyn PluginProcessRunner,
    ) -> Result<ProviderInventory, CatalogBlocker> {
        let into_blocker = |detail: String| CatalogBlocker::InventoryFailed { detail };
        let settings = read_optional_json(&home.join("settings.json"))
            .map_err(into_blocker)?
            .unwrap_or_default();
        let known = read_optional_json(&home.join("plugins/known_marketplaces.json"))
            .map_err(into_blocker)?
            .unwrap_or_default();
        let installed = read_optional_json(&home.join("plugins/installed_plugins.json"))
            .map_err(into_blocker)?
            .unwrap_or_default();

        let mut inventory = ProviderInventory::default();
        if let Some(map) = known.as_object() {
            for (name, entry) in map {
                let source = entry.get("source");
                let location = source.and_then(claude_source_location);
                let (source_type, source_value) = match location {
                    Some(location) => ("git".to_string(), location),
                    None => (
                        "local".to_string(),
                        // The path stays local to plan comparison; it never
                        // becomes portable intent.
                        source
                            .and_then(|value| value.get("path"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                };
                inventory.marketplaces.push(MarketplaceObservation {
                    name: name.clone(),
                    source_type,
                    source: source_value,
                    git_ref: None,
                    root: entry
                        .get("installPath")
                        .and_then(|value| value.as_str())
                        .map(PathBuf::from),
                });
            }
        }
        let enabled_map = settings
            .get("enabledPlugins")
            .and_then(|value| value.as_object());
        if let Some(map) = installed.get("plugins").and_then(|value| value.as_object()) {
            for (plugin_id, entries) in map {
                let entries = entries.as_array().cloned().unwrap_or_default();
                // The user config dir is Claude's user-global scope by
                // location; entries carrying another explicit scope are
                // excluded from catalog inventory outright.
                let user_scoped: Vec<&serde_json::Value> = entries
                    .iter()
                    .filter(|entry| {
                        entry
                            .get("scope")
                            .and_then(|value| value.as_str())
                            .map(|scope| scope == "user")
                            .unwrap_or(true)
                    })
                    .collect();
                if user_scoped.is_empty() {
                    continue;
                }
                let install_root = user_scoped.iter().find_map(|entry| {
                    entry
                        .get("installPath")
                        .and_then(|value| value.as_str())
                        .map(PathBuf::from)
                });
                let present = install_root.as_ref().is_some_and(|path| path.exists());
                let version = user_scoped
                    .iter()
                    .find_map(|entry| entry.get("version").and_then(|value| value.as_str()))
                    .map(str::to_string);
                let enabled = enabled_map
                    .and_then(|map| map.get(plugin_id))
                    .and_then(|value| value.as_bool())
                    != Some(false);
                let namespace = plugin_id
                    .split_once('@')
                    .map(|(_, namespace)| namespace)
                    .unwrap_or("")
                    .to_string();
                inventory.plugins.push(PluginObservation {
                    provider: ProviderKey::Claude,
                    namespace,
                    plugin_id: plugin_id.clone(),
                    installed: present,
                    enabled,
                    observed_version: version,
                    install_root,
                });
            }
        }
        Ok(inventory)
    }

    fn portable_intent(
        &self,
        observation: &PluginObservation,
        inventory: &ProviderInventory,
    ) -> Option<PluginInstallIntentResource> {
        if observation.namespace == CLAUDE_SKILLS_DIR_NAMESPACE {
            return None;
        }
        intent_resource(ProviderKey::Claude, observation, inventory)
    }

    fn plan_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        currently_registered_marketplace: bool,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker> {
        let PortablePluginSource::ProviderMarketplace { git, .. } = &intent.source;
        let mut marketplace_argv = Vec::new();
        match git {
            Some(git) if !currently_registered_marketplace => {
                marketplace_argv.push(vec![
                    "plugin".to_string(),
                    "marketplace".to_string(),
                    "add".to_string(),
                    git.location.clone(),
                    "--scope".to_string(),
                    "user".to_string(),
                ]);
            }
            Some(_) => {}
            None => {
                return Err(CatalogBlocker::NativeContractUnverified {
                    provider: ProviderKey::Claude,
                    operation: "plugin install".to_string(),
                    detail: format!(
                        "plugin '{}' has no portable marketplace source; install it manually",
                        intent.plugin_id
                    ),
                });
            }
        }
        let verb = match mode {
            PluginApplyMode::InstallMissing => "install",
            PluginApplyMode::OverwriteExisting => "update",
        };
        Ok(NativePluginPlan {
            provider: ProviderKey::Claude,
            argv: vec![
                "plugin".to_string(),
                verb.to_string(),
                intent.plugin_id.clone(),
                "--scope".to_string(),
                "user".to_string(),
            ],
            marketplace_argv,
            mode,
        })
    }

    fn verify_plugin_apply(
        &self,
        intent: &PluginInstallIntentDescriptor,
        home: &Path,
        runner: &dyn PluginProcessRunner,
    ) -> Result<PluginVerification, CatalogBlocker> {
        let inventory = self.discover_plugins(home, runner)?;
        let observation = inventory.observation(&intent.plugin_id).cloned();
        let verified = observation
            .as_ref()
            .is_some_and(|observation| observation.installed);
        Ok(PluginVerification {
            verified,
            detail: if verified {
                format!("claude reports '{}' installed", intent.plugin_id)
            } else {
                format!(
                    "claude does not report '{}' installed after the operation",
                    intent.plugin_id
                )
            },
            observation,
        })
    }
}

/// Deterministic source-stamp fields for one provider inventory.
pub fn inventory_stamp_fields(inventory: &ProviderInventory) -> Vec<String> {
    let mut fields = Vec::new();
    for marketplace in &inventory.marketplaces {
        fields.push(format!(
            "marketplace:{}:{}:{}",
            marketplace.name, marketplace.source_type, marketplace.source
        ));
    }
    for observation in &inventory.plugins {
        fields.push(format!(
            "plugin:{}:{}:{}",
            observation.plugin_id,
            observation.installed,
            observation.observed_version.as_deref().unwrap_or("")
        ));
    }
    fields.sort();
    fields
}

/// A stub runner for tests: canned outputs per argv.
#[cfg(test)]
pub struct StubRunner {
    pub responses: std::collections::BTreeMap<String, CmdOutput>,
}

#[cfg(test)]
impl StubRunner {
    pub fn new(entries: Vec<(&str, CmdOutput)>) -> Self {
        Self {
            responses: entries
                .into_iter()
                .map(|(key, output)| (key.to_string(), output))
                .collect(),
        }
    }
}

#[cfg(test)]
impl PluginProcessRunner for StubRunner {
    fn run(
        &self,
        _provider: ProviderKey,
        _home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String> {
        let key = argv.join(" ");
        self.responses
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("unexpected native invocation: {}", key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(stdout: &str) -> CmdOutput {
        CmdOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn codex_inventory_parses_the_frozen_json_shapes() {
        let runner = StubRunner::new(vec![
            (
                "plugin marketplace list --json",
                ok(
                    r#"{"marketplaces":[{"name":"acme","marketplaceSource":{"sourceType":"git","source":"https://example.com/acme/market"},"root":"/tmp/mp/acme"}]}"#,
                ),
            ),
            (
                "plugin list --json",
                ok(
                    r#"{"installed":[{"pluginId":"tools","marketplaceName":"acme","version":"1.2.0","installed":true,"enabled":true},{"pluginId":"ghost","marketplaceName":"acme","installed":false}]}"#,
                ),
            ),
        ]);
        let inventory = CodexCatalogAdapter
            .discover_plugins(Path::new("/tmp/codex"), &runner)
            .unwrap();
        assert_eq!(inventory.marketplaces.len(), 1);
        assert_eq!(inventory.plugins.len(), 1);
        let observation = &inventory.plugins[0];
        assert_eq!(observation.plugin_id, "tools@acme");
        let intent = CodexCatalogAdapter
            .portable_intent(observation, &inventory)
            .unwrap();
        assert_eq!(
            intent.descriptor.resolution,
            PluginResolutionPolicy::ProviderResolved
        );
    }

    #[test]
    fn codex_plans_use_the_verified_native_argv() {
        let intent = PluginInstallIntentDescriptor {
            provider: ProviderKey::Codex,
            provider_native_namespace: "acme".to_string(),
            plugin_id: "tools@acme".to_string(),
            source: PortablePluginSource::ProviderMarketplace {
                namespace: "acme".to_string(),
                git: Some(GitMarketplaceSource {
                    location: "https://example.com/acme/market".to_string(),
                    git_ref: None,
                }),
            },
            requirement: None,
            resolution: PluginResolutionPolicy::ProviderResolved,
        };
        let plan = CodexCatalogAdapter
            .plan_plugin_apply(&intent, false, PluginApplyMode::InstallMissing)
            .unwrap();
        assert_eq!(plan.argv, vec!["plugin", "add", "tools@acme"]);
        assert_eq!(
            plan.marketplace_argv,
            vec![vec![
                "plugin",
                "marketplace",
                "add",
                "https://example.com/acme/market"
            ]]
        );
        let overwrite = CodexCatalogAdapter
            .plan_plugin_apply(&intent, true, PluginApplyMode::OverwriteExisting)
            .unwrap();
        assert_eq!(overwrite.argv, vec!["plugin", "add", "tools@acme"]);
        assert_eq!(
            overwrite.marketplace_argv,
            vec![vec!["plugin", "marketplace", "upgrade"]]
        );
    }

    #[test]
    fn claude_inventory_reads_bounded_files_and_filters_scope() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::create_dir_all(home.join("plugins")).unwrap();
        std::fs::write(
            home.join("plugins/known_marketplaces.json"),
            r#"{"acme":{"source":{"repo":"acme/market"}},"secrets":{"source":{"path":"/tmp/local"}}}"#,
        )
        .unwrap();
        let install_root = home.join("plugins/cache/tools");
        std::fs::create_dir_all(&install_root).unwrap();
        std::fs::write(
            home.join("plugins/installed_plugins.json"),
            format!(
                r#"{{"plugins":{{"tools@acme":[{{"installPath":"{}","version":"2.0.0"}}],"proj@acme":[{{"installPath":"{}","scope":"project"}}],"draft@skills-dir":[{{"installPath":"{}"}}]}}}}"#,
                install_root.display(),
                install_root.display(),
                install_root.display(),
            ),
        )
        .unwrap();
        std::fs::write(
            home.join("settings.json"),
            r#"{"enabledPlugins":{"tools@acme":false}}"#,
        )
        .unwrap();

        struct NeverRunner;
        impl PluginProcessRunner for NeverRunner {
            fn run(
                &self,
                _provider: ProviderKey,
                _home: &Path,
                _argv: &[String],
            ) -> Result<CmdOutput, String> {
                panic!("claude inventory must not launch a process");
            }
        }
        let inventory = ClaudeCatalogAdapter
            .discover_plugins(home, &NeverRunner)
            .unwrap();
        // Project-scoped entries are excluded before resource construction.
        let ids: Vec<&str> = inventory
            .plugins
            .iter()
            .map(|observation| observation.plugin_id.as_str())
            .collect();
        assert_eq!(ids, vec!["draft@skills-dir", "tools@acme"]);
        let tools = inventory.observation("tools@acme").unwrap();
        assert!(tools.installed);
        assert!(!tools.enabled, "local disablement is observed, not changed");

        // skills-dir entries are never portable marketplace intent.
        let draft = inventory.observation("draft@skills-dir").unwrap();
        assert!(ClaudeCatalogAdapter
            .portable_intent(draft, &inventory)
            .is_none());
        // A local-path marketplace yields manual-only intent.
        let manual_observation = PluginObservation {
            provider: ProviderKey::Claude,
            namespace: "secrets".to_string(),
            plugin_id: "vault@secrets".to_string(),
            installed: true,
            enabled: true,
            observed_version: None,
            install_root: None,
        };
        let manual = ClaudeCatalogAdapter
            .portable_intent(&manual_observation, &inventory)
            .unwrap();
        assert_eq!(
            manual.descriptor.resolution,
            PluginResolutionPolicy::ManualOnly
        );
    }

    #[test]
    fn claude_plans_pin_the_user_scope_explicitly() {
        let intent = PluginInstallIntentDescriptor {
            provider: ProviderKey::Claude,
            provider_native_namespace: "acme".to_string(),
            plugin_id: "tools@acme".to_string(),
            source: PortablePluginSource::ProviderMarketplace {
                namespace: "acme".to_string(),
                git: Some(GitMarketplaceSource {
                    location: "acme/market".to_string(),
                    git_ref: None,
                }),
            },
            requirement: None,
            resolution: PluginResolutionPolicy::ProviderResolved,
        };
        let install = ClaudeCatalogAdapter
            .plan_plugin_apply(&intent, true, PluginApplyMode::InstallMissing)
            .unwrap();
        assert_eq!(
            install.argv,
            vec!["plugin", "install", "tools@acme", "--scope", "user"]
        );
        let update = ClaudeCatalogAdapter
            .plan_plugin_apply(&intent, true, PluginApplyMode::OverwriteExisting)
            .unwrap();
        assert_eq!(
            update.argv,
            vec!["plugin", "update", "tools@acme", "--scope", "user"]
        );
        // A manual-only source never becomes an executable plan.
        let manual = PluginInstallIntentDescriptor {
            source: PortablePluginSource::ProviderMarketplace {
                namespace: "acme".to_string(),
                git: None,
            },
            resolution: PluginResolutionPolicy::ManualOnly,
            ..intent
        };
        assert!(matches!(
            ClaudeCatalogAdapter.plan_plugin_apply(&manual, true, PluginApplyMode::InstallMissing),
            Err(CatalogBlocker::NativeContractUnverified { .. })
        ));
    }

    #[test]
    fn the_registry_contains_exactly_codex_and_claude() {
        let matrix = capability_matrix();
        assert_eq!(matrix.len(), 2);
        assert_eq!(
            matrix[0],
            (
                ProviderKey::Codex,
                vec![
                    CatalogResourceKindTag::AgentSkill,
                    CatalogResourceKindTag::PluginInstallIntent,
                    CatalogResourceKindTag::CodexPetV1,
                    CatalogResourceKindTag::CodexPetV2,
                ]
            )
        );
        assert_eq!(
            matrix[1],
            (
                ProviderKey::Claude,
                vec![
                    CatalogResourceKindTag::AgentSkill,
                    CatalogResourceKindTag::PluginInstallIntent,
                ]
            )
        );
    }
}
