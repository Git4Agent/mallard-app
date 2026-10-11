//! Compiled catalog provider adapters: `codex` and `claude` in production.
//!
//! An adapter owns everything provider-specific about the Library: the
//! machine-local target roots a profile home exposes, the native process
//! contract, launch behavior, and the native (provider-inventory) protocol
//! for plug-in install intents. Common code reaches an adapter only through
//! the registry and the [`CatalogProviderAdapter`] trait; it never branches
//! on a provider key.
//!
//! Inventory uses provider-home files. Native mutation argv were verified
//! against codex-cli 0.145.0 and Claude Code 2.1.220:
//!
//! - Codex inventory: bounded local config, install-marker, and manifest reads
//!   under the selected `CODEX_HOME`. Inventory never launches a process.
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
//! Unsupported mutation contracts fail through a typed
//! [`CatalogBlocker::NativeContractUnverified`] rather than guessed syntax.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mallard_infrastructure::cli_discovery::ResolvedCli;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::providers::{claude, codex, AgentProviderRegistry};

use super::domain::{
    CatalogBlocker, CatalogResourceEnvelope, CatalogTargetKey, NativePluginPlan, PluginApplyMode,
    ProviderKey,
};
use super::plugin_intent::{
    validate_canonical_plugin_id, validate_git_location, GitMarketplaceSource, PluginInstallIntent,
    PluginInstallIntentDescriptor, PluginInstallIntentIdentity, PluginResolutionPolicy,
    PortablePluginSource,
};
use super::profiles::prospective_canonical;
use super::provider_state::read_optional_json;
use super::registry::CatalogRegistry;

pub const MAX_CLI_BYTES: usize = 2 * 1024 * 1024;
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
    fn run(
        &self,
        provider: &ProviderKey,
        home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String>;
}

/// The native CLI an adapter declares: its program name, for messages, and
/// the environment variable that pins the provider home. Which executable
/// runs is the provider's CLI resolution, through [`CatalogProviderAdapter::resolve_cli`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCli {
    pub program: String,
    pub home_env_var: String,
}

/// Launches the real provider CLI with an explicit home environment and the
/// home as working directory, so behavior never depends on where Mallard was
/// started. The executable comes from the provider's CLI selection, read
/// from this machine's app settings for each run; the child gets the
/// discovery `PATH`, since Mallard's own may be a desktop launch's minimum.
pub struct NativeProcessRunner {
    registry: Arc<CatalogRegistry>,
    machine_home: PathBuf,
}

impl NativeProcessRunner {
    pub fn new(registry: Arc<CatalogRegistry>, machine_home: PathBuf) -> Self {
        Self {
            registry,
            machine_home,
        }
    }
}

impl PluginProcessRunner for NativeProcessRunner {
    fn run(
        &self,
        provider: &ProviderKey,
        home: &Path,
        argv: &[String],
    ) -> Result<CmdOutput, String> {
        let adapter = self.registry.require_provider(provider)?;
        let cli = adapter
            .native_cli()
            .ok_or_else(|| format!("{} declares no native CLI", provider))?;
        let repository = LocalStateRepository::from_home_dir(&self.machine_home)?;
        let settings = mallard_infrastructure::app_settings::load_app_settings(&repository)?;
        let resolved = adapter.resolve_cli(settings.cli_path(provider))?;
        let mut command = std::process::Command::new(&resolved.executable);
        command
            .args(argv)
            .current_dir(home)
            .env("PATH", &resolved.search_path)
            .env(&cli.home_env_var, home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("start {}: {}", cli.program, error))?;
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
                        return Err(format!("{} plugin operation timed out", cli.program));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(format!("wait for {}: {}", cli.program, error)),
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

pub fn redact(line: &str) -> String {
    mallard_infrastructure::credential_redaction::shared()
        .redact_str(line)
        .unwrap_or_else(|| line.to_string())
}

/// Machine-local native inventory facts. These never enter portable intent
/// or its semantic digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginObservation {
    pub provider: ProviderKey,
    pub namespace: String,
    pub plugin_id: String,
    pub installed: bool,
    pub enabled: Option<bool>,
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

    /// Canonical ownership roots: any directory canonically inside one of
    /// these belongs to a plugin, never to the standalone class.
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

/// Safe provider facts for the Library. The frontend iterates these; it
/// never holds a fixed provider array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogProviderDescriptor {
    pub key: ProviderKey,
    pub label: String,
    /// Home-relative default directory name, e.g. `.codex`.
    pub default_profile_directory: Option<String>,
    /// Display order among providers: lower first.
    pub display_rank: u32,
}

/// One machine-local target root a profile home exposes, resolved by the
/// adapter. `canonical_root` is the directory scanned or operated on (or the
/// home itself for native resources); `root_class` is the portable class
/// token codecs may record inside a resource identity; `summary` is the only
/// form that may reach the frontend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCatalogTarget {
    pub target: CatalogTargetKey,
    pub canonical_root: PathBuf,
    pub root_class: String,
    pub summary: String,
    /// Child directories canonically inside a plugin install or marketplace
    /// root belong to that plugin and are excluded from this root's scan.
    pub exclude_provider_owned: bool,
}

/// The compiled adapter contract. Tauri-free; the registry keys dispatch by
/// [`ProviderKey`], so the adapter carries no self-describing key beyond its
/// descriptor.
pub trait CatalogProviderAdapter: Send + Sync {
    fn descriptor(&self) -> CatalogProviderDescriptor;

    /// The native CLI, when the provider has one the Library drives.
    fn native_cli(&self) -> Option<NativeCli> {
        None
    }

    /// The machine-local target roots for one canonical profile home. Never
    /// scans a project checkout, working directory, or repository.
    fn resolve_targets(
        &self,
        canonical_home: &Path,
        os_user_home: &Path,
    ) -> Result<Vec<ResolvedCatalogTarget>, CatalogBlocker>;

    fn launch_home_app(&self, home: &Path, os_user_home: &Path) -> Result<(), String>;

    /// `cli_path` is the CLI executable saved in app settings, if any.
    fn launch_home_terminal(&self, home: &Path, cli_path: Option<&Path>) -> Result<(), String>;

    /// The CLI an operation runs, by the provider's selection policy.
    fn resolve_cli(&self, _cli_path: Option<&Path>) -> Result<ResolvedCli, String> {
        Err(format!("{} declares no native CLI", self.descriptor().key))
    }

    /// Read provider inventory from local files. This interface cannot run a CLI.
    fn discover_local(&self, home: &Path) -> Result<ProviderInventory, CatalogBlocker>;

    /// Portable envelopes for every installed native resource in one
    /// inventory snapshot.
    fn native_resources(&self, inventory: &ProviderInventory) -> Vec<CatalogResourceEnvelope>;

    /// The content digest of the installed native resource with this
    /// envelope's identity, or `None` when it is absent. Presence and intent
    /// are read from one snapshot.
    fn native_state(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
    ) -> Result<Option<String>, String>;

    /// A provider-owned uninstall. Unknown adapters refuse rather than guess syntax.
    fn plan_native_remove(
        &self,
        _envelope: &CatalogResourceEnvelope,
    ) -> Result<Vec<String>, String> {
        Err("this provider does not support plugin removal".to_string())
    }

    /// Plan the exact native operation for one approved stored resource.
    /// Fails through a typed blocker when the native contract cannot express
    /// the intent or the resource restores manually.
    fn plan_native_apply(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker>;
}

fn resolved_target(
    key: &str,
    root: &Path,
    root_class: &str,
    summary: &str,
    exclude_provider_owned: bool,
) -> Result<ResolvedCatalogTarget, CatalogBlocker> {
    Ok(ResolvedCatalogTarget {
        target: CatalogTargetKey::parse(key).expect("static target keys are valid"),
        canonical_root: prospective_canonical(root)
            .map_err(|detail| CatalogBlocker::ProfileUnavailable { detail })?,
        root_class: root_class.to_string(),
        summary: summary.to_string(),
        exclude_provider_owned,
    })
}

/// True when the canonical home is the provider's default `$HOME/<dir>`.
fn is_default_home(canonical_home: &Path, os_user_home: &Path, directory: &str) -> bool {
    prospective_canonical(&os_user_home.join(directory))
        .is_ok_and(|default_home| default_home == canonical_home)
}

/// The infrastructure registry is the source of truth for a provider's
/// display name and default home; the catalog adapter only relays it.
fn infrastructure_descriptor(
    key: &ProviderKey,
) -> Option<mallard_infrastructure::providers::ProviderDescriptor> {
    AgentProviderRegistry::production()
        .get(key)
        .map(|adapter| adapter.descriptor())
}

fn portable_marketplace_source(
    marketplace: Option<&MarketplaceObservation>,
) -> (Option<GitMarketplaceSource>, PluginResolutionPolicy) {
    let Some(marketplace) = marketplace else {
        return (None, PluginResolutionPolicy::ManualOnly);
    };
    if marketplace.source_type != "git" {
        return (None, PluginResolutionPolicy::ManualOnly);
    }
    let Ok(location) = normalized_git_location(&marketplace.source) else {
        return (None, PluginResolutionPolicy::ManualOnly);
    };
    (
        Some(GitMarketplaceSource {
            location,
            git_ref: marketplace.git_ref.clone(),
        }),
        PluginResolutionPolicy::ProviderResolved,
    )
}

fn normalized_git_location(value: &str) -> Result<String, String> {
    validate_git_location("plugin marketplace source", value)?;
    let value = value.strip_suffix('/').unwrap_or(value);
    Ok(value.strip_suffix(".git").unwrap_or(value).to_string())
}

fn validate_marketplace_binding(
    intent: &PluginInstallIntent,
    inventory: &ProviderInventory,
) -> Result<bool, String> {
    let PortablePluginSource::ProviderMarketplace { git, .. } = &intent.descriptor.source;
    let Some(expected) = git else {
        return Ok(inventory.marketplace(&intent.identity.namespace).is_some());
    };
    let Some(observed) = inventory.marketplace(&intent.identity.namespace) else {
        return Ok(false);
    };
    let matches = observed.source_type == "git"
        && normalized_git_location(&observed.source).is_ok_and(|source| {
            normalized_git_location(&expected.location).is_ok_and(|expected| source == expected)
        })
        && observed.git_ref == expected.git_ref;
    if !matches {
        return Err(format!(
            "plugin marketplace '{}' no longer matches the approved source and ref",
            intent.identity.namespace
        ));
    }
    Ok(true)
}

/// Portable install intent for one observation, or `None` when the
/// observation is not a catalog plugin resource (unqualified identity).
fn intent_envelope(
    provider: &ProviderKey,
    observation: &PluginObservation,
    inventory: &ProviderInventory,
) -> Option<CatalogResourceEnvelope> {
    validate_canonical_plugin_id("plugin id", &observation.plugin_id).ok()?;
    let (git, resolution) =
        portable_marketplace_source(inventory.marketplace(&observation.namespace));
    PluginInstallIntent {
        identity: PluginInstallIntentIdentity {
            namespace: observation.namespace.clone(),
            plugin_id: observation.plugin_id.clone(),
        },
        descriptor: PluginInstallIntentDescriptor {
            source: PortablePluginSource::ProviderMarketplace {
                namespace: observation.namespace.clone(),
                git,
            },
            resolution,
        },
    }
    .envelope(provider)
    .ok()
}

fn native_state_for(
    provider: &ProviderKey,
    envelope: &CatalogResourceEnvelope,
    inventory: &ProviderInventory,
    accept: impl Fn(&PluginObservation) -> bool,
) -> Result<Option<String>, String> {
    let intent = PluginInstallIntent::decode(envelope)?;
    validate_marketplace_binding(&intent, inventory)?;
    Ok(inventory
        .observation(&intent.identity.plugin_id)
        .filter(|observation| observation.installed && accept(observation))
        .and_then(|observation| intent_envelope(provider, observation, inventory))
        .map(|rebuilt| rebuilt.content_sha256))
}

fn manual_install_blocker(provider_label: &str, plugin_id: &str) -> CatalogBlocker {
    CatalogBlocker::ManualInstall {
        detail: format!(
            "'{}' has no portable marketplace source; install it with the {} CLI",
            plugin_id, provider_label
        ),
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

pub struct CodexCatalogAdapter;

impl CatalogProviderAdapter for CodexCatalogAdapter {
    fn descriptor(&self) -> CatalogProviderDescriptor {
        let key = codex::provider_key();
        let infrastructure = infrastructure_descriptor(&key);
        CatalogProviderDescriptor {
            key,
            label: infrastructure
                .as_ref()
                .map(|descriptor| descriptor.display_name.clone())
                .unwrap_or_else(|| "Codex".to_string()),
            default_profile_directory: infrastructure
                .and_then(|descriptor| descriptor.default_profile_directory),
            display_rank: 10,
        }
    }

    fn native_cli(&self) -> Option<NativeCli> {
        Some(NativeCli {
            program: "codex".to_string(),
            home_env_var: "CODEX_HOME".to_string(),
        })
    }

    /// Codex exposes its own home skill root — plus the OS user's fixed
    /// `$HOME/.agents/skills` root when the profile is the default `~/.codex`
    /// config, so one shared directory is never registered under two
    /// profiles — its plugin home, and its pets directory.
    fn resolve_targets(
        &self,
        canonical_home: &Path,
        os_user_home: &Path,
    ) -> Result<Vec<ResolvedCatalogTarget>, CatalogBlocker> {
        let mut targets = vec![resolved_target(
            "provider-home.skills",
            &canonical_home.join("skills"),
            "provider-home",
            "Codex home",
            true,
        )?];
        let default_directory = self
            .descriptor()
            .default_profile_directory
            .unwrap_or_else(|| ".codex".to_string());
        if is_default_home(canonical_home, os_user_home, &default_directory) {
            targets.push(resolved_target(
                "shared-user.skills",
                &os_user_home.join(".agents").join("skills"),
                "shared-user",
                "Shared user",
                true,
            )?);
        }
        targets.push(resolved_target(
            "user.plugins",
            canonical_home,
            "user",
            "Plugins",
            false,
        )?);
        targets.push(resolved_target(
            "desktop.pets",
            &canonical_home.join("pets"),
            "provider-home",
            "Pets",
            false,
        )?);
        Ok(targets)
    }

    fn launch_home_app(&self, home: &Path, os_user_home: &Path) -> Result<(), String> {
        codex::actions::launch_codex_home_app(home, os_user_home)
    }

    fn launch_home_terminal(&self, home: &Path, cli_path: Option<&Path>) -> Result<(), String> {
        codex::actions::launch_codex_home_terminal(home, cli_path)
    }

    fn resolve_cli(&self, cli_path: Option<&Path>) -> Result<ResolvedCli, String> {
        codex::cli::resolve(cli_path)
    }

    fn discover_local(&self, home: &Path) -> Result<ProviderInventory, CatalogBlocker> {
        super::codex_inventory::discover(home)
            .map_err(|detail| CatalogBlocker::InventoryFailed { detail })
    }

    fn native_resources(&self, inventory: &ProviderInventory) -> Vec<CatalogResourceEnvelope> {
        let key = codex::provider_key();
        inventory
            .plugins
            .iter()
            .filter(|observation| observation.installed)
            .filter_map(|observation| intent_envelope(&key, observation, inventory))
            .collect()
    }

    fn native_state(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
    ) -> Result<Option<String>, String> {
        native_state_for(&codex::provider_key(), envelope, inventory, |_| true)
    }

    // Verified with isolated CLI help: Codex 0.156.0 / Claude plugin uninstall.
    fn plan_native_remove(
        &self,
        envelope: &CatalogResourceEnvelope,
    ) -> Result<Vec<String>, String> {
        let intent = PluginInstallIntent::decode(envelope)?;
        Ok(vec![
            "plugin".to_string(),
            "remove".to_string(),
            intent.identity.plugin_id,
        ])
    }

    fn plan_native_apply(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker> {
        let key = codex::provider_key();
        let intent = PluginInstallIntent::decode(envelope)
            .map_err(|detail| CatalogBlocker::InventoryFailed { detail })?;
        let PortablePluginSource::ProviderMarketplace { git, .. } = &intent.descriptor.source;
        let registered = validate_marketplace_binding(&intent, inventory).map_err(|detail| {
            CatalogBlocker::NativeContractUnverified {
                provider: key.clone(),
                operation: "plugin marketplace binding".to_string(),
                detail,
            }
        })?;
        let mut marketplace_argv = Vec::new();
        match git {
            Some(git) if !registered => {
                if git.git_ref.is_some() {
                    return Err(CatalogBlocker::NativeContractUnverified {
                        provider: key,
                        operation: "plugin marketplace add".to_string(),
                        detail: "the Codex CLI contract cannot bind a new marketplace to the approved ref"
                            .to_string(),
                    });
                }
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
                return Err(manual_install_blocker(
                    &self.descriptor().label,
                    &intent.identity.plugin_id,
                ));
            }
        }
        Ok(NativePluginPlan {
            provider: key,
            argv: vec![
                "plugin".to_string(),
                "add".to_string(),
                intent.identity.plugin_id.clone(),
            ],
            marketplace_argv,
            mode,
        })
    }
}

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

pub struct ClaudeCatalogAdapter;

fn claude_source_location(source: &serde_json::Value) -> Option<String> {
    source
        .get("repo")
        .or_else(|| source.get("url"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

impl CatalogProviderAdapter for ClaudeCatalogAdapter {
    fn descriptor(&self) -> CatalogProviderDescriptor {
        let key = claude::provider_key();
        let infrastructure = infrastructure_descriptor(&key);
        CatalogProviderDescriptor {
            key,
            label: infrastructure
                .as_ref()
                .map(|descriptor| descriptor.display_name.clone())
                .unwrap_or_else(|| "Claude".to_string()),
            default_profile_directory: infrastructure
                .and_then(|descriptor| descriptor.default_profile_directory),
            display_rank: 20,
        }
    }

    fn native_cli(&self) -> Option<NativeCli> {
        Some(NativeCli {
            program: "claude".to_string(),
            home_env_var: "CLAUDE_CONFIG_DIR".to_string(),
        })
    }

    /// Claude exposes only its own user-global skills root and plugin home.
    fn resolve_targets(
        &self,
        canonical_home: &Path,
        _os_user_home: &Path,
    ) -> Result<Vec<ResolvedCatalogTarget>, CatalogBlocker> {
        Ok(vec![
            resolved_target(
                "provider-home.skills",
                &canonical_home.join("skills"),
                "provider-home",
                "Claude",
                true,
            )?,
            resolved_target("user.plugins", canonical_home, "user", "Plugins", false)?,
        ])
    }

    fn launch_home_app(&self, home: &Path, _os_user_home: &Path) -> Result<(), String> {
        claude::actions::launch_claude_home_app(home)
    }

    fn launch_home_terminal(&self, home: &Path, cli_path: Option<&Path>) -> Result<(), String> {
        claude::actions::launch_claude_home_terminal(home, cli_path)
    }

    fn resolve_cli(&self, cli_path: Option<&Path>) -> Result<ResolvedCli, String> {
        claude::cli::resolve(cli_path)
    }

    fn discover_local(&self, home: &Path) -> Result<ProviderInventory, CatalogBlocker> {
        let key = claude::provider_key();
        let into_blocker = |detail: String| CatalogBlocker::InventoryFailed { detail };
        let settings = read_optional_json(home, &["settings.json"])
            .map_err(into_blocker)?
            .unwrap_or_default();
        let known = read_optional_json(home, &["plugins", "known_marketplaces.json"])
            .map_err(into_blocker)?
            .unwrap_or_default();
        let installed = read_optional_json(home, &["plugins", "installed_plugins.json"])
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
                    provider: key.clone(),
                    namespace,
                    plugin_id: plugin_id.clone(),
                    installed: present,
                    enabled: Some(enabled),
                    observed_version: version,
                    install_root,
                });
            }
        }
        Ok(inventory)
    }

    fn native_resources(&self, inventory: &ProviderInventory) -> Vec<CatalogResourceEnvelope> {
        let key = claude::provider_key();
        inventory
            .plugins
            .iter()
            .filter(|observation| {
                observation.installed && observation.namespace != CLAUDE_SKILLS_DIR_NAMESPACE
            })
            .filter_map(|observation| intent_envelope(&key, observation, inventory))
            .collect()
    }

    fn native_state(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
    ) -> Result<Option<String>, String> {
        native_state_for(
            &claude::provider_key(),
            envelope,
            inventory,
            |observation| observation.namespace != CLAUDE_SKILLS_DIR_NAMESPACE,
        )
    }

    fn plan_native_remove(
        &self,
        envelope: &CatalogResourceEnvelope,
    ) -> Result<Vec<String>, String> {
        let intent = PluginInstallIntent::decode(envelope)?;
        Ok(vec![
            "plugin".to_string(),
            "uninstall".to_string(),
            intent.identity.plugin_id,
            "--scope".to_string(),
            "user".to_string(),
        ])
    }

    fn plan_native_apply(
        &self,
        envelope: &CatalogResourceEnvelope,
        inventory: &ProviderInventory,
        mode: PluginApplyMode,
    ) -> Result<NativePluginPlan, CatalogBlocker> {
        let key = claude::provider_key();
        let intent = PluginInstallIntent::decode(envelope)
            .map_err(|detail| CatalogBlocker::InventoryFailed { detail })?;
        let PortablePluginSource::ProviderMarketplace { git, .. } = &intent.descriptor.source;
        let registered = validate_marketplace_binding(&intent, inventory).map_err(|detail| {
            CatalogBlocker::NativeContractUnverified {
                provider: key.clone(),
                operation: "plugin marketplace binding".to_string(),
                detail,
            }
        })?;
        let mut marketplace_argv = Vec::new();
        match git {
            Some(git) if !registered => {
                if git.git_ref.is_some() {
                    return Err(CatalogBlocker::NativeContractUnverified {
                        provider: key,
                        operation: "plugin marketplace add".to_string(),
                        detail: "the Claude CLI contract cannot bind a new marketplace to the approved ref"
                            .to_string(),
                    });
                }
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
                return Err(manual_install_blocker(
                    &self.descriptor().label,
                    &intent.identity.plugin_id,
                ));
            }
        }
        let verb = match mode {
            PluginApplyMode::InstallMissing => "install",
            PluginApplyMode::OverwriteExisting => "update",
        };
        Ok(NativePluginPlan {
            provider: key,
            argv: vec![
                "plugin".to_string(),
                verb.to_string(),
                intent.identity.plugin_id.clone(),
                "--scope".to_string(),
                "user".to_string(),
            ],
            marketplace_argv,
            mode,
        })
    }
}

/// A stub runner for tests: canned outputs per argv.
#[cfg(test)]
mod tests {
    use super::*;

    fn intent(provider: &str, git: Option<&str>) -> CatalogResourceEnvelope {
        PluginInstallIntent {
            identity: PluginInstallIntentIdentity {
                namespace: "acme".to_string(),
                plugin_id: "tools@acme".to_string(),
            },
            descriptor: PluginInstallIntentDescriptor {
                source: PortablePluginSource::ProviderMarketplace {
                    namespace: "acme".to_string(),
                    git: git.map(|location| GitMarketplaceSource {
                        location: location.to_string(),
                        git_ref: None,
                    }),
                },
                resolution: if git.is_some() {
                    PluginResolutionPolicy::ProviderResolved
                } else {
                    PluginResolutionPolicy::ManualOnly
                },
            },
        }
        .envelope(&ProviderKey::parse(provider).unwrap())
        .unwrap()
    }

    #[test]
    fn removal_uses_provider_uninstall_and_pins_claude_user_scope() {
        assert_eq!(
            CodexCatalogAdapter
                .plan_native_remove(&intent("codex", None))
                .unwrap(),
            vec!["plugin", "remove", "tools@acme"]
        );
        assert_eq!(
            ClaudeCatalogAdapter
                .plan_native_remove(&intent("claude", None))
                .unwrap(),
            vec!["plugin", "uninstall", "tools@acme", "--scope", "user"]
        );
    }

    #[test]
    fn codex_inventory_reads_local_state_into_portable_intent() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::write(home.join("config.toml"), "[plugins.\"tools@acme\"]\nenabled=true\n[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n").unwrap();
        let manifest = home.join("plugins/cache/acme/tools/1.2.0/.codex-plugin/plugin.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(manifest, r#"{"name":"tools","version":"1.2.0"}"#).unwrap();
        let inventory = CodexCatalogAdapter.discover_local(home).unwrap();
        assert_eq!(inventory.marketplaces.len(), 1);
        assert_eq!(inventory.plugins.len(), 1);
        let resources = CodexCatalogAdapter.native_resources(&inventory);
        assert_eq!(resources.len(), 1);
        let decoded = PluginInstallIntent::decode(&resources[0]).unwrap();
        assert_eq!(decoded.identity.plugin_id, "tools@acme");
        assert_eq!(
            decoded.descriptor.resolution,
            PluginResolutionPolicy::ProviderResolved
        );
        assert_eq!(
            CodexCatalogAdapter
                .native_state(&resources[0], &inventory)
                .unwrap()
                .as_deref(),
            Some(resources[0].content_sha256.as_str())
        );
        assert!(CodexCatalogAdapter
            .native_state(&intent("codex", None), &inventory)
            .unwrap()
            .is_some());
        assert_eq!(
            inventory.ownership_roots(),
            vec![home.join("plugins/cache/acme/tools")]
        );
    }

    #[test]
    fn codex_plans_use_the_verified_native_argv() {
        let envelope = intent("codex", Some("https://example.com/acme/market"));
        let empty = ProviderInventory::default();
        let plan = CodexCatalogAdapter
            .plan_native_apply(&envelope, &empty, PluginApplyMode::InstallMissing)
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
        let registered = ProviderInventory {
            marketplaces: vec![MarketplaceObservation {
                name: "acme".to_string(),
                source_type: "git".to_string(),
                source: "https://example.com/acme/market".to_string(),
                git_ref: None,
                root: None,
            }],
            ..Default::default()
        };
        let overwrite = CodexCatalogAdapter
            .plan_native_apply(&envelope, &registered, PluginApplyMode::OverwriteExisting)
            .unwrap();
        assert_eq!(overwrite.argv, vec!["plugin", "add", "tools@acme"]);
        assert_eq!(
            overwrite.marketplace_argv,
            vec![vec!["plugin", "marketplace", "upgrade"]]
        );
        let manual = CodexCatalogAdapter
            .plan_native_apply(
                &intent("codex", None),
                &empty,
                PluginApplyMode::InstallMissing,
            )
            .unwrap_err();
        assert!(matches!(manual, CatalogBlocker::ManualInstall { .. }));
        assert!(manual.to_string().contains("Codex CLI"));
    }

    #[test]
    fn native_plans_and_state_reject_changed_marketplace_coordinates() {
        for (provider, adapter) in [
            ("codex", &CodexCatalogAdapter as &dyn CatalogProviderAdapter),
            (
                "claude",
                &ClaudeCatalogAdapter as &dyn CatalogProviderAdapter,
            ),
        ] {
            let envelope = PluginInstallIntent {
                identity: PluginInstallIntentIdentity {
                    namespace: "acme".to_string(),
                    plugin_id: "tools@acme".to_string(),
                },
                descriptor: PluginInstallIntentDescriptor {
                    source: PortablePluginSource::ProviderMarketplace {
                        namespace: "acme".to_string(),
                        git: Some(GitMarketplaceSource {
                            location: "https://example.com/acme/approved".to_string(),
                            git_ref: Some("release".to_string()),
                        }),
                    },
                    resolution: PluginResolutionPolicy::ProviderResolved,
                },
            }
            .envelope(&ProviderKey::parse(provider).unwrap())
            .unwrap();
            let approved = ProviderInventory {
                marketplaces: vec![MarketplaceObservation {
                    name: "acme".to_string(),
                    source_type: "git".to_string(),
                    source: "https://example.com/acme/approved.git/".to_string(),
                    git_ref: Some("release".to_string()),
                    root: None,
                }],
                ..Default::default()
            };
            assert!(adapter
                .plan_native_apply(&envelope, &approved, PluginApplyMode::InstallMissing)
                .is_ok());
            assert!(adapter.native_state(&envelope, &approved).is_ok());

            let mut changed_type = approved.clone();
            changed_type.marketplaces[0].source_type = "local".to_string();
            let mut changed_source = approved.clone();
            changed_source.marketplaces[0].source = "https://example.com/acme/other".to_string();
            let mut changed_ref = approved;
            changed_ref.marketplaces[0].git_ref = Some("main".to_string());
            for changed in [changed_type, changed_source, changed_ref] {
                assert!(adapter
                    .plan_native_apply(&envelope, &changed, PluginApplyMode::InstallMissing)
                    .is_err());
                assert!(adapter.native_state(&envelope, &changed).is_err());
            }
        }
    }

    #[test]
    fn diagnostics_and_content_warnings_share_credential_shape_detection() {
        use crate::storage_catalog::agent_skill::AgentSkillCodec;
        use crate::storage_catalog::codec::CatalogFormatCodec;

        let token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let temp = tempfile::tempdir().unwrap();
        let skill = temp.path().join("example-skill");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: example-skill\n---\n").unwrap();
        std::fs::write(skill.join("credential.txt"), token).unwrap();

        let captured = AgentSkillCodec
            .capture_directory(&skill, "provider-home")
            .unwrap();
        let diagnostic = redact(&format!("provider failed with {token}"));

        assert!(!diagnostic.contains(token));
        assert!(captured
            .warnings
            .iter()
            .any(|warning| warning.code == "secret_shaped"));
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
            serde_json::to_vec(&serde_json::json!({"plugins": {
                "tools@acme": [{"installPath": install_root, "version": "2.0.0"}],
                "proj@acme": [{"installPath": install_root, "scope": "project"}],
                "draft@skills-dir": [{"installPath": install_root}]
            }}))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            home.join("settings.json"),
            r#"{"enabledPlugins":{"tools@acme":false}}"#,
        )
        .unwrap();

        let inventory = ClaudeCatalogAdapter.discover_local(home).unwrap();
        // Project-scoped entries are excluded before resource construction.
        let ids: Vec<&str> = inventory
            .plugins
            .iter()
            .map(|observation| observation.plugin_id.as_str())
            .collect();
        assert_eq!(ids, vec!["draft@skills-dir", "tools@acme"]);
        let tools = inventory.observation("tools@acme").unwrap();
        assert!(tools.installed);
        assert_eq!(
            tools.enabled,
            Some(false),
            "local disablement is observed, not changed"
        );

        // skills-dir entries are never portable marketplace intent.
        let resources = ClaudeCatalogAdapter.native_resources(&inventory);
        assert_eq!(resources.len(), 1);
        assert_eq!(
            PluginInstallIntent::decode(&resources[0])
                .unwrap()
                .identity
                .plugin_id,
            "tools@acme"
        );
        // A local-path marketplace yields manual-only intent.
        let manual_observation = PluginObservation {
            provider: claude::provider_key(),
            namespace: "secrets".to_string(),
            plugin_id: "vault@secrets".to_string(),
            installed: true,
            enabled: Some(true),
            observed_version: None,
            install_root: None,
        };
        let manual =
            intent_envelope(&claude::provider_key(), &manual_observation, &inventory).unwrap();
        assert_eq!(
            PluginInstallIntent::decode(&manual)
                .unwrap()
                .descriptor
                .resolution,
            PluginResolutionPolicy::ManualOnly
        );
    }

    #[cfg(unix)]
    #[test]
    fn claude_inventory_rejects_a_symlinked_state_parent() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("known_marketplaces.json"),
            r#"{"acme":{"source":{"repo":"acme/market"}}}"#,
        )
        .unwrap();
        symlink(&outside, home.join("plugins")).unwrap();

        assert!(ClaudeCatalogAdapter.discover_local(&home).is_err());
    }

    #[test]
    fn claude_plans_pin_the_user_scope_explicitly() {
        let envelope = intent("claude", Some("acme/market"));
        let registered = ProviderInventory {
            marketplaces: vec![MarketplaceObservation {
                name: "acme".to_string(),
                source_type: "git".to_string(),
                source: "acme/market".to_string(),
                git_ref: None,
                root: None,
            }],
            ..Default::default()
        };
        let install = ClaudeCatalogAdapter
            .plan_native_apply(&envelope, &registered, PluginApplyMode::InstallMissing)
            .unwrap();
        assert_eq!(
            install.argv,
            vec!["plugin", "install", "tools@acme", "--scope", "user"]
        );
        let update = ClaudeCatalogAdapter
            .plan_native_apply(&envelope, &registered, PluginApplyMode::OverwriteExisting)
            .unwrap();
        assert_eq!(
            update.argv,
            vec!["plugin", "update", "tools@acme", "--scope", "user"]
        );
        // A manual-only source never becomes an executable plan.
        assert!(matches!(
            ClaudeCatalogAdapter.plan_native_apply(
                &intent("claude", None),
                &registered,
                PluginApplyMode::InstallMissing
            ),
            Err(CatalogBlocker::ManualInstall { .. })
        ));
    }

    #[test]
    fn adapters_declare_their_targets_and_native_clis() {
        let temp = tempfile::tempdir().unwrap();
        let user_home = temp.path().join("user");
        let codex_home = user_home.join(".codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        let codex_home = std::fs::canonicalize(&codex_home).unwrap();
        let targets = CodexCatalogAdapter
            .resolve_targets(&codex_home, &user_home)
            .unwrap();
        let keys: Vec<&str> = targets
            .iter()
            .map(|target| target.target.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "provider-home.skills",
                "shared-user.skills",
                "user.plugins",
                "desktop.pets"
            ]
        );
        assert!(targets[1].canonical_root.ends_with(".agents/skills"));
        assert_eq!(targets[1].root_class, "shared-user");
        assert!(targets[0].exclude_provider_owned);
        assert!(!targets[3].exclude_provider_owned);

        // A second Codex config sees only its own roots.
        let other = user_home.join("codex-conf2");
        std::fs::create_dir_all(&other).unwrap();
        let other = std::fs::canonicalize(&other).unwrap();
        let keys: Vec<String> = CodexCatalogAdapter
            .resolve_targets(&other, &user_home)
            .unwrap()
            .iter()
            .map(|target| target.target.to_string())
            .collect();
        assert_eq!(
            keys,
            vec!["provider-home.skills", "user.plugins", "desktop.pets"]
        );

        let claude_home = user_home.join(".claude");
        std::fs::create_dir_all(&claude_home).unwrap();
        let keys: Vec<String> = ClaudeCatalogAdapter
            .resolve_targets(&std::fs::canonicalize(&claude_home).unwrap(), &user_home)
            .unwrap()
            .iter()
            .map(|target| target.target.to_string())
            .collect();
        assert_eq!(keys, vec!["provider-home.skills", "user.plugins"]);

        assert_eq!(
            CodexCatalogAdapter.native_cli().unwrap().home_env_var,
            "CODEX_HOME"
        );
        assert_eq!(
            ClaudeCatalogAdapter.native_cli().unwrap().home_env_var,
            "CLAUDE_CONFIG_DIR"
        );
        assert_eq!(CodexCatalogAdapter.descriptor().label, "Codex");
        assert_eq!(
            ClaudeCatalogAdapter
                .descriptor()
                .default_profile_directory
                .as_deref(),
            Some(".claude")
        );
    }
}
