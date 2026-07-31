//! Global provider-home inventory for current project bundles.
//!
//! Discovers installed global plugins (portable install intent only, never
//! payload) and global custom skills under a mapped provider home's
//! `skills/` directory, classifying every skill candidate against plugin
//! ownership evidence before it may become a standalone snapshot.
//!
//! Everything here is Tauri-free and filesystem-driven so tests run on
//! fixtures. Ownership order is authoritative: plugins are inventoried
//! first, then skill candidates are classified against the plugin install
//! roots; name or content equality is never ownership evidence.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::domain::validate_skill_name;
use super::provider_capture::{Provider, StandaloneSkillSource};
use crate::codex_plugins;

/// Version of the naming/ownership contract implemented by this adapter.
/// Recorded in descriptor metadata so pull can refuse to reinterpret keys
/// produced by an incompatible rule.
pub const PROVIDER_ADAPTER_VERSION: &str = "2";

const MAX_SKILL_DIR_ENTRIES: usize = 512;
const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;

/// Portable install intent for one inventoried global plugin. Contains no
/// file payload; marketplace/source data is validated before it may enter a
/// bundle and local paths never do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalPluginSource {
    pub provider: Provider,
    pub plugin_id: String,
    pub marketplace: Option<String>,
    /// "git" or "local"; local sources stay machine-local and block sync of
    /// the marketplace provenance (the plugin id itself may still sync).
    pub source_type: Option<String>,
    pub source: Option<String>,
    pub observed_version: Option<String>,
    pub enabled: bool,
    /// Effective skill names this plugin is proven to export, used to derive
    /// non-selectable plugin-provided capabilities.
    pub provided_skills: Vec<String>,
}

/// A skill candidate proven to be lifecycle-managed by a plugin. It is never
/// separately snapshotted; selecting the owning plugin is the only transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginProvidedSkill {
    pub provider: Provider,
    pub name: String,
    /// Normalized owning plugin id when the evidence identifies one; `None`
    /// means "inside a plugin-managed root but not attributable", which still
    /// excludes the candidate from standalone capture.
    pub owner_plugin_id: Option<String>,
    pub evidence: String,
}

/// A candidate that cannot be classified automatically. Blocked candidates
/// are surfaced with their reason and are never captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedSkillCandidate {
    pub provider: Provider,
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct GlobalInventory {
    pub standalone_skills: Vec<StandaloneSkillSource>,
    pub plugin_provided_skills: Vec<PluginProvidedSkill>,
    pub blocked_skills: Vec<BlockedSkillCandidate>,
    pub plugins: Vec<GlobalPluginSource>,
    pub warnings: Vec<String>,
}

/// Inventory one mapped provider home in ownership order: resolve the home,
/// inventory plugins, build the ownership index, then classify `skills/*`.
pub fn inventory_provider_home(provider: Provider, home: &Path) -> GlobalInventory {
    let mut inventory = GlobalInventory::default();
    let canonical_home = match canonical_dir(home) {
        Some(path) => path,
        None => {
            inventory
                .warnings
                .push(format!("{} home is not readable", provider_label(provider)));
            return inventory;
        }
    };

    let ownership = match provider {
        Provider::Claude => inventory_claude_plugins(&canonical_home, &mut inventory),
        Provider::Codex => inventory_codex_plugins(&canonical_home, &mut inventory),
    };
    classify_global_skills(provider, &canonical_home, &ownership, &mut inventory);
    inventory
}

/// Canonical plugin install roots for the bound provider home. Any skill
/// candidate whose canonical path or symlink target falls inside one of
/// these roots is plugin payload, never a standalone snapshot.
struct OwnershipIndex {
    /// (canonical root, owning plugin id when attributable)
    roots: Vec<(PathBuf, Option<String>)>,
}

impl OwnershipIndex {
    fn owner_of(&self, canonical: &Path) -> Option<(Option<String>, String)> {
        let mut best: Option<(&PathBuf, &Option<String>)> = None;
        for (root, owner) in &self.roots {
            if canonical.starts_with(root)
                && best.is_none_or(|(current, _)| {
                    root.components().count() > current.components().count()
                })
            {
                best = Some((root, owner));
            }
        }
        best.map(|(root, owner)| {
            (
                owner.clone(),
                format!("canonical path is inside plugin root '{}'", root.display()),
            )
        })
    }
}

fn inventory_claude_plugins(home: &Path, inventory: &mut GlobalInventory) -> OwnershipIndex {
    let mut roots = Vec::new();
    // The whole plugin manager subtree is provider-owned regardless of
    // whether the specific plugin can be attributed.
    if let Some(managed) = canonical_dir(&home.join("plugins")) {
        roots.push((managed, None));
    }
    match codex_plugins::claude_inventory(home) {
        Ok(native) => {
            let marketplaces: BTreeMap<&str, &codex_plugins::MarketplaceInfo> = native
                .marketplaces
                .iter()
                .map(|marketplace| (marketplace.name.as_str(), marketplace))
                .collect();
            for plugin in &native.plugins {
                if !plugin.installed {
                    continue;
                }
                let marketplace = marketplaces.get(plugin.marketplace.as_str());
                let install_root = claude_plugin_install_root(home, plugin);
                let provided_skills = install_root
                    .as_deref()
                    .map(exported_skill_names)
                    .unwrap_or_default();
                if let Some(root) = install_root {
                    roots.push((root, Some(plugin.id.clone())));
                }
                inventory.plugins.push(GlobalPluginSource {
                    provider: Provider::Claude,
                    plugin_id: plugin.id.clone(),
                    marketplace: Some(plugin.marketplace.clone()),
                    source_type: marketplace.map(|info| info.source_type.clone()),
                    source: marketplace.and_then(|info| portable_source(info)),
                    observed_version: (!plugin.version.is_empty()).then(|| plugin.version.clone()),
                    enabled: plugin.enabled,
                    provided_skills,
                });
            }
        }
        Err(error) => inventory
            .warnings
            .push(format!("could not inventory Claude plugins: {}", error)),
    }
    OwnershipIndex { roots }
}

fn inventory_codex_plugins(home: &Path, inventory: &mut GlobalInventory) -> OwnershipIndex {
    let mut roots = Vec::new();
    if let Some(managed) = canonical_dir(&home.join("plugins")) {
        roots.push((managed, None));
    }
    if let Some(temporary) = canonical_dir(&home.join(".tmp")) {
        roots.push((temporary, None));
    }
    let config_path = home.join("config.toml");
    let bytes = match fs::read(&config_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return OwnershipIndex { roots }
        }
        Err(error) => {
            inventory
                .warnings
                .push(format!("could not read Codex config.toml: {}", error));
            return OwnershipIndex { roots };
        }
    };
    let table = match std::str::from_utf8(&bytes)
        .map_err(|e| e.to_string())
        .and_then(|text| text.parse::<toml::Table>().map_err(|e| e.to_string()))
    {
        Ok(table) => table,
        Err(error) => {
            inventory
                .warnings
                .push(format!("could not parse Codex config.toml: {}", error));
            return OwnershipIndex { roots };
        }
    };
    let marketplaces = table
        .get("marketplaces")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    let Some(plugins) = table.get("plugins").and_then(toml::Value::as_table) else {
        return OwnershipIndex { roots };
    };
    for (id, definition) in plugins {
        if !portable_plugin_identifier(id) {
            inventory
                .warnings
                .push(format!("skipped non-portable Codex plugin id '{}'", id));
            continue;
        }
        let enabled = definition
            .get("enabled")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        let marketplace_name = id.rsplit_once('@').map(|(_, name)| name.to_string());
        let marketplace_entry = marketplace_name
            .as_deref()
            .and_then(|name| marketplaces.get(name))
            .and_then(toml::Value::as_table);
        let source_type = marketplace_entry
            .and_then(|entry| entry.get("source_type"))
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        let source = marketplace_entry
            .and_then(|entry| entry.get("source"))
            .and_then(toml::Value::as_str)
            .filter(|_| source_type.as_deref() == Some("git"))
            .filter(|value| !contains_url_credentials(value))
            .map(str::to_string);
        inventory.plugins.push(GlobalPluginSource {
            provider: Provider::Codex,
            plugin_id: id.clone(),
            marketplace: marketplace_name,
            source_type,
            source,
            observed_version: definition
                .get("version")
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            enabled,
            provided_skills: Vec::new(),
        });
    }
    OwnershipIndex { roots }
}

/// Claude plugin payloads live under `plugins/repos/<marketplace>/<plugin>`
/// (manager-owned). Attribution requires the exact directory to exist.
fn claude_plugin_install_root(home: &Path, plugin: &codex_plugins::PluginInfo) -> Option<PathBuf> {
    let (name, _) = plugin
        .id
        .rsplit_once('@')
        .unwrap_or((plugin.id.as_str(), ""));
    if name.is_empty() || name.contains(['/', '\\']) || plugin.marketplace.contains(['/', '\\']) {
        return None;
    }
    canonical_dir(
        &home
            .join("plugins/repos")
            .join(&plugin.marketplace)
            .join(name),
    )
}

/// Effective names of skills exported by a plugin install root, read in a
/// bounded no-follow way. Directory names are the runtime identity for both
/// supported providers (adapter contract v1); payload bytes are never read.
fn exported_skill_names(install_root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(install_root.join("skills")) else {
        return names;
    };
    for entry in entries.flatten().take(MAX_SKILL_DIR_ENTRIES) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || validate_skill_name("plugin skill", &name).is_err() {
            continue;
        }
        if entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
        {
            names.push(name);
        }
    }
    names.sort();
    names
}

fn classify_global_skills(
    provider: Provider,
    home: &Path,
    ownership: &OwnershipIndex,
    inventory: &mut GlobalInventory,
) {
    let skills_dir = home.join("skills");
    let entries = match fs::read_dir(&skills_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            inventory
                .warnings
                .push(format!("could not read global skills directory: {}", error));
            return;
        }
    };
    let mut count = 0_usize;
    for entry in entries.flatten() {
        count += 1;
        if count > MAX_SKILL_DIR_ENTRIES {
            inventory
                .warnings
                .push("global skills directory has too many entries; remainder skipped".into());
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = skills_dir.join(&name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            match fs::canonicalize(&path) {
                Ok(target) => match ownership.owner_of(&target) {
                    Some((owner, evidence)) => {
                        inventory.plugin_provided_skills.push(PluginProvidedSkill {
                            provider,
                            name,
                            owner_plugin_id: owner,
                            evidence: format!("symlink: {}", evidence),
                        })
                    }
                    None => inventory.blocked_skills.push(BlockedSkillCandidate {
                        provider,
                        name,
                        reason: "symlink to a target outside every known plugin root".into(),
                    }),
                },
                Err(_) => inventory.blocked_skills.push(BlockedSkillCandidate {
                    provider,
                    name,
                    reason: "broken symlink".into(),
                }),
            }
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&path) else {
            inventory.blocked_skills.push(BlockedSkillCandidate {
                provider,
                name,
                reason: "directory cannot be resolved".into(),
            });
            continue;
        };
        if let Some((owner, evidence)) = ownership.owner_of(&canonical) {
            inventory.plugin_provided_skills.push(PluginProvidedSkill {
                provider,
                name,
                owner_plugin_id: owner,
                evidence,
            });
            continue;
        }
        match validate_standalone_candidate(&name, &canonical) {
            Ok(effective_name) => inventory.standalone_skills.push(StandaloneSkillSource {
                provider,
                stable_key: format!(
                    "custom-skill:v1:{}:{}",
                    provider_label(provider),
                    effective_name
                ),
                effective_name,
                install_dir_name: name,
                source_dir: canonical,
            }),
            Err(reason) => inventory.blocked_skills.push(BlockedSkillCandidate {
                provider,
                name,
                reason,
            }),
        }
    }

    // Runtime identity and install path are independent. Two physical
    // directories may therefore declare the same effective capability; do
    // not let either become selectable until the user resolves that
    // ambiguity locally.
    let mut declarations = BTreeMap::<String, Vec<String>>::new();
    for skill in &inventory.standalone_skills {
        declarations
            .entry(skill.effective_name.to_ascii_lowercase())
            .or_default()
            .push(skill.install_dir_name.clone());
    }
    let duplicates = declarations
        .iter()
        .filter(|(_, directories)| directories.len() > 1)
        .map(|(effective_name, directories)| {
            let mut directories = directories.clone();
            directories.sort();
            (effective_name.clone(), directories)
        })
        .collect::<BTreeMap<_, _>>();
    if !duplicates.is_empty() {
        let mut blocked = Vec::new();
        inventory.standalone_skills.retain(|skill| {
            let key = skill.effective_name.to_ascii_lowercase();
            let Some(directories) = duplicates.get(&key) else {
                return true;
            };
            blocked.push(BlockedSkillCandidate {
                provider: skill.provider,
                name: skill.install_dir_name.clone(),
                reason: format!(
                    "effective skill name '{}' is declared by multiple directories: {}",
                    skill.effective_name,
                    directories.join(", ")
                ),
            });
            false
        });
        inventory.blocked_skills.extend(blocked);
    }
}

/// Standalone skill identity contract: a valid declared name
/// is the runtime-visible capability, while the directory name remains the
/// physical installation path. Providers also accept legacy skills without a
/// declaration, for which the directory name remains the effective name.
fn validate_standalone_candidate(name: &str, dir: &Path) -> Result<String, String> {
    validate_skill_name("skill install directory", name)?;
    let skill_md = dir.join("SKILL.md");
    let metadata = fs::symlink_metadata(&skill_md)
        .map_err(|_| "no SKILL.md declaration; not a recognizable skill".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("SKILL.md is not a regular file".to_string());
    }
    if metadata.len() > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md exceeds the supported size".to_string());
    }
    let contents = fs::read_to_string(&skill_md)
        .map_err(|error| format!("SKILL.md is unreadable: {}", error))?;
    match declared_skill_name(&contents) {
        Some(declared) => {
            validate_skill_name("declared skill name", &declared)?;
            Ok(declared)
        }
        None => Ok(name.to_string()),
    }
}

/// The `name:` declaration in SKILL.md's leading YAML frontmatter block, if
/// one exists.
fn declared_skill_name(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn portable_source(info: &codex_plugins::MarketplaceInfo) -> Option<String> {
    (info.source_type == "git"
        && !info.source.is_empty()
        && !contains_url_credentials(&info.source))
    .then(|| info.source.clone())
}

fn portable_plugin_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && !value.contains("..")
        && !value.starts_with(['/', '.', '~'])
        && !value.contains(['\\', ' '])
        && !contains_url_credentials(value)
}

fn contains_url_credentials(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    remainder[..authority_end].contains('@')
}

fn canonical_dir(path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    fs::metadata(&canonical)
        .ok()
        .filter(fs::Metadata::is_dir)
        .map(|_| canonical)
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn skill(home: &Path, name: &str, declared: Option<&str>) {
        let body = match declared {
            Some(declared) => format!("---\nname: {}\n---\nBody\n", declared),
            None => "No frontmatter\n".to_string(),
        };
        write(
            &home.join("skills").join(name).join("SKILL.md"),
            body.as_bytes(),
        );
    }

    #[test]
    fn regular_global_skill_is_standalone() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        skill(&home, "review", Some("review"));
        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert_eq!(inventory.standalone_skills.len(), 1);
        let discovered = &inventory.standalone_skills[0];
        assert_eq!(discovered.effective_name, "review");
        assert_eq!(discovered.install_dir_name, "review");
        assert_eq!(discovered.stable_key, "custom-skill:v1:claude:review");
        assert!(inventory.blocked_skills.is_empty());
    }

    #[test]
    fn declared_name_may_differ_from_install_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        skill(
            &home,
            "capture-lsservice-detail",
            Some("get-real-hardware-rh-service"),
        );
        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert_eq!(inventory.standalone_skills.len(), 1);
        let discovered = &inventory.standalone_skills[0];
        assert_eq!(discovered.effective_name, "get-real-hardware-rh-service");
        assert_eq!(discovered.install_dir_name, "capture-lsservice-detail");
        assert!(inventory.blocked_skills.is_empty());
    }

    #[test]
    fn duplicate_effective_names_block_every_claiming_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        skill(&home, "review-one", Some("review"));
        skill(&home, "review-two", Some("review"));
        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert!(inventory.standalone_skills.is_empty());
        assert_eq!(inventory.blocked_skills.len(), 2);
        assert!(inventory
            .blocked_skills
            .iter()
            .all(|blocked| blocked.reason.contains("multiple directories")));
    }

    #[test]
    fn missing_skill_md_blocks_classification() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        write(&home.join("skills/mystery/notes.txt"), b"not a skill");
        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert!(inventory.standalone_skills.is_empty());
        assert_eq!(inventory.blocked_skills.len(), 1);
        assert!(inventory.blocked_skills[0].reason.contains("SKILL.md"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_into_plugin_root_is_plugin_provided_and_external_symlink_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        // Plugin-managed payload with an exported skill.
        write(
            &home.join("plugins/repos/acme/tools/skills/security/SKILL.md"),
            b"---\nname: security\n---\n",
        );
        fs::create_dir_all(home.join("skills")).unwrap();
        std::os::unix::fs::symlink(
            home.join("plugins/repos/acme/tools/skills/security"),
            home.join("skills/security"),
        )
        .unwrap();
        // Symlink escaping every known root.
        let external = temp.path().join("elsewhere/thing");
        write(&external.join("SKILL.md"), b"---\nname: thing\n---\n");
        std::os::unix::fs::symlink(&external, home.join("skills/thing")).unwrap();

        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert!(inventory.standalone_skills.is_empty());
        assert_eq!(inventory.plugin_provided_skills.len(), 1);
        assert_eq!(inventory.plugin_provided_skills[0].name, "security");
        assert_eq!(inventory.blocked_skills.len(), 1);
        assert_eq!(inventory.blocked_skills[0].name, "thing");
        assert!(inventory.blocked_skills[0].reason.contains("outside"));
    }

    #[test]
    fn claude_native_inventory_attributes_plugins_and_exported_skills() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        write(
            &home.join("plugins/known_marketplaces.json"),
            br#"{"acme":{"source":{"source":"github","repo":"acme/marketplace"}}}"#,
        );
        write(
            &home.join("plugins/repos/acme/tools/skills/security/SKILL.md"),
            b"---\nname: security\n---\n",
        );
        let install_path = home.join("plugins/repos/acme/tools");
        write(
            &home.join("plugins/installed_plugins.json"),
            format!(
                r#"{{"plugins":{{"tools@acme":[{{"installPath":{},"version":"1.2.0"}}]}}}}"#,
                serde_json::to_string(install_path.to_str().unwrap()).unwrap()
            )
            .as_bytes(),
        );
        let inventory = inventory_provider_home(Provider::Claude, &home);
        assert_eq!(inventory.plugins.len(), 1);
        let plugin = &inventory.plugins[0];
        assert_eq!(plugin.plugin_id, "tools@acme");
        assert_eq!(plugin.marketplace.as_deref(), Some("acme"));
        assert_eq!(plugin.provided_skills, vec!["security".to_string()]);
        assert!(plugin.source.as_deref().is_some_and(|s| s.contains("acme")));
    }

    #[test]
    fn codex_config_plugins_are_inventoried_without_local_sources() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write(
            &home.join("config.toml"),
            concat!(
                "[plugins.\"tool@team\"]\nenabled = true\n",
                "[plugins.\"off@team\"]\nenabled = false\n",
                "[marketplaces.team]\nsource_type = 'git'\nsource = 'acme/team'\n",
                "[marketplaces.private]\nsource_type = 'local'\nsource = '/home/user/mkt'\n",
                "[plugins.\"secret@private\"]\nenabled = true\n",
            )
            .as_bytes(),
        );
        let inventory = inventory_provider_home(Provider::Codex, &home);
        let ids: Vec<&str> = inventory
            .plugins
            .iter()
            .map(|plugin| plugin.plugin_id.as_str())
            .collect();
        assert_eq!(ids, vec!["off@team", "secret@private", "tool@team"]);
        let tool = inventory
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == "tool@team")
            .unwrap();
        assert!(tool.enabled);
        assert_eq!(tool.source.as_deref(), Some("acme/team"));
        let private = inventory
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == "secret@private")
            .unwrap();
        // A local-path marketplace never contributes a portable source.
        assert_eq!(private.source, None);
        assert_eq!(private.source_type.as_deref(), Some("local"));
    }

    #[test]
    fn name_or_content_equality_is_not_ownership_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("claude");
        // A plugin exports "review"; an independent skill with the same name
        // and identical bytes lives in the global skills directory.
        write(
            &home.join("plugins/repos/acme/tools/skills/review/SKILL.md"),
            b"---\nname: review\n---\nSame bytes\n",
        );
        write(
            &home.join("plugins/installed_plugins.json"),
            br#"{"acme":{"tools":[{"version":"1.0.0"}]}}"#,
        );
        skill(&home, "review", Some("review"));
        let inventory = inventory_provider_home(Provider::Claude, &home);
        // The independent directory stays selectable as a standalone skill.
        assert_eq!(inventory.standalone_skills.len(), 1);
        assert_eq!(inventory.standalone_skills[0].effective_name, "review");
    }
}
