//! Ownership-ordered local catalog inventory.
//!
//! Discovery inputs are exactly the resolved machine-global profiles and the
//! fixed provider contracts: `<CODEX_HOME>/skills`, the OS user's
//! `$HOME/.agents/skills`, `<CLAUDE_CONFIG_DIR>/skills`, `<CODEX_HOME>/pets`,
//! and native plugin state. Project checkouts, working directories, Git
//! roots, and project-local lookalike trees are never inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::adapters::{adapter_for, PluginProcessRunner, ProviderInventory};
use super::codex_pet::{capture_pet, pet_resource_from_capture, CapturedPet};
use super::domain::{
    validate_skill_name, warning_digest, AgentSkillScope, CatalogBlocker, CatalogResource,
    CatalogResourceId, CatalogResourceKindTag, CatalogResourceWarning, CodexPetFormat,
    LocalCatalogTargetId, ProviderKey, MAX_SKILL_FILES,
};
use super::profiles::ResolvedCatalogContext;
use super::skill_tree::{capture_skill_tree, skill_resource_from_tree, CapturedSkillTree};

/// One discovered local resource in portable form plus its machine-local
/// target and capture payload.
pub struct LocalResourceRecord {
    pub resource: CatalogResource,
    pub target: LocalCatalogTargetId,
    pub target_path: Option<PathBuf>,
    /// Newest modification time anywhere under `target_path`, in Unix
    /// seconds. Reported so a row can say when the resource last changed on
    /// this machine; never part of any identity or digest.
    pub local_modified_at: Option<u64>,
    pub warnings: Vec<CatalogResourceWarning>,
    pub skill_tree: Option<CapturedSkillTree>,
    pub pet: Option<CapturedPet>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedCandidate {
    pub display_name: String,
    pub provider: ProviderKey,
    pub scope_label: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginOwnedSkill {
    pub name: String,
    pub provider: ProviderKey,
    pub owner: String,
}

pub struct CatalogLocalInventory {
    pub resources: BTreeMap<CatalogResourceId, LocalResourceRecord>,
    pub blocked: Vec<BlockedCandidate>,
    pub plugin_owned_skills: Vec<PluginOwnedSkill>,
    pub warnings: Vec<String>,
    pub plugin_inventories: BTreeMap<ProviderKey, ProviderInventory>,
}

fn canonical_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The newest modification time at or under `path`, in Unix seconds.
///
/// A directory's own timestamp only moves when its direct entries change, so
/// a skill whose SKILL.md was edited today would otherwise report the day its
/// folder was created. Symlinks are never followed — capture already refuses
/// them, and this walk must not leave the tree it was handed. Entries that
/// cannot be inspected are skipped: a missing date is honest, a wrong one is
/// not.
fn latest_modified_secs(path: &Path) -> Option<u64> {
    fn modified_secs(meta: &fs::Metadata) -> Option<u64> {
        meta.modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|elapsed| elapsed.as_secs())
    }

    let root = path.symlink_metadata().ok()?;
    let mut newest = modified_secs(&root);
    if !root.is_dir() {
        return newest;
    }
    let mut pending = vec![path.to_path_buf()];
    // The same ceiling capture works under, so a pathological tree cannot turn
    // a list refresh into an unbounded walk.
    let mut visited = 0usize;
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_SKILL_FILES {
                return newest;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            newest = newest.max(modified_secs(&meta));
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    newest
}

fn scan_skill_root(
    scope: AgentSkillScope,
    root: &Path,
    only_install_directory: Option<&str>,
    ownership_roots: &[PathBuf],
    owner_labels: &BTreeMap<PathBuf, String>,
    inventory: &mut CatalogLocalInventory,
) -> Result<(), String> {
    let provider = scope.provider();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read skill root '{}': {}", root.display(), error)),
    };

    struct Candidate {
        directory_name: String,
        effective_name: String,
        tree: CapturedSkillTree,
        path: PathBuf,
    }
    let mut candidates: Vec<Candidate> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("walk skill root: {}", error))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            if only_install_directory.is_some() {
                continue;
            }
            inventory.blocked.push(BlockedCandidate {
                display_name: entry.file_name().to_string_lossy().to_string(),
                provider,
                scope_label: Some(scope.display_label().to_string()),
                reason: "skill directory name is not UTF-8".to_string(),
            });
            continue;
        };
        if only_install_directory.is_some_and(|wanted| name != wanted) {
            continue;
        }
        // Provider-managed hidden namespaces (`.system`, dot-prefixed
        // bundles) are excluded, not candidates.
        if name.starts_with('.') {
            continue;
        }
        let meta = match entry.path().symlink_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                inventory.blocked.push(BlockedCandidate {
                    display_name: name,
                    provider,
                    scope_label: Some(scope.display_label().to_string()),
                    reason: format!("skill directory is unreadable: {}", error),
                });
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            inventory.blocked.push(BlockedCandidate {
                display_name: name,
                provider,
                scope_label: Some(scope.display_label().to_string()),
                reason: "skill directory is a symlink".to_string(),
            });
            continue;
        }
        if !meta.is_dir() {
            // Loose files in a skills root are not skill candidates.
            continue;
        }
        if let Err(error) = validate_skill_name("skill install directory", &name) {
            inventory.blocked.push(BlockedCandidate {
                display_name: name,
                provider,
                scope_label: Some(scope.display_label().to_string()),
                reason: error,
            });
            continue;
        }
        // Ownership: a directory canonically inside a plugin install or
        // marketplace root belongs to that plugin. Name or content equality
        // alone is never ownership evidence.
        let canonical = canonical_or_self(&entry.path());
        if let Some(owner_root) = ownership_roots
            .iter()
            .find(|owner| canonical.starts_with(owner))
        {
            inventory.plugin_owned_skills.push(PluginOwnedSkill {
                name,
                provider,
                owner: owner_labels
                    .get(owner_root)
                    .cloned()
                    .unwrap_or_else(|| owner_root.to_string_lossy().to_string()),
            });
            continue;
        }
        match capture_skill_tree(&entry.path()) {
            Ok(tree) => {
                let effective_name = tree.declared_name.clone().unwrap_or_else(|| name.clone());
                candidates.push(Candidate {
                    directory_name: name,
                    effective_name,
                    tree,
                    path: entry.path(),
                });
            }
            Err(reason) => {
                inventory.blocked.push(BlockedCandidate {
                    display_name: name,
                    provider,
                    scope_label: Some(scope.display_label().to_string()),
                    reason,
                });
            }
        }
    }

    // Exact-name and case-folded collisions inside one target root block
    // every colliding candidate; equal names across the two Codex roots are
    // reported, never collapsed.
    let mut folded_counts: BTreeMap<String, usize> = BTreeMap::new();
    for candidate in &candidates {
        *folded_counts
            .entry(candidate.effective_name.to_lowercase())
            .or_default() += 1;
    }
    for candidate in candidates {
        if folded_counts[&candidate.effective_name.to_lowercase()] > 1 {
            inventory.blocked.push(BlockedCandidate {
                display_name: candidate.effective_name,
                provider,
                scope_label: Some(scope.display_label().to_string()),
                reason: "skill name collides with another skill in the same root".to_string(),
            });
            continue;
        }
        let resource = match skill_resource_from_tree(
            scope,
            &candidate.effective_name,
            &candidate.directory_name,
            &candidate.tree.files,
        ) {
            Ok(resource) => resource,
            Err(reason) => {
                inventory.blocked.push(BlockedCandidate {
                    display_name: candidate.effective_name,
                    provider,
                    scope_label: Some(scope.display_label().to_string()),
                    reason,
                });
                continue;
            }
        };
        let wrapped = CatalogResource::AgentSkill(resource);
        let resource_id = wrapped.resource_id();
        let mut warnings = Vec::new();
        if !candidate.tree.warnings.executable_paths.is_empty() {
            let message = format!(
                "contains executable content: {}",
                candidate.tree.warnings.executable_paths.join(", ")
            );
            warnings.push(CatalogResourceWarning {
                code: "executable_content".to_string(),
                digest: warning_digest("executable_content", &resource_id, &message),
                message,
            });
        }
        if !candidate.tree.warnings.secret_shaped_paths.is_empty() {
            let message = format!(
                "contains credential-shaped content: {}",
                candidate.tree.warnings.secret_shaped_paths.join(", ")
            );
            warnings.push(CatalogResourceWarning {
                code: "secret_shaped".to_string(),
                digest: warning_digest("secret_shaped", &resource_id, &message),
                message,
            });
        }
        inventory.resources.insert(
            resource_id,
            LocalResourceRecord {
                resource: wrapped,
                target: LocalCatalogTargetId::SkillDirectory {
                    scope,
                    canonical_root: root.to_string_lossy().to_string(),
                    install_directory: candidate.directory_name,
                },
                local_modified_at: latest_modified_secs(&candidate.path),
                target_path: Some(candidate.path),
                warnings,
                skill_tree: Some(candidate.tree),
                pet: None,
            },
        );
    }
    Ok(())
}

fn scan_pets(
    codex_home: &Path,
    only_pet_id: Option<&str>,
    inventory: &mut CatalogLocalInventory,
) -> Result<(), String> {
    let pets_root = codex_home.join("pets");
    let entries = match fs::read_dir(&pets_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "read pets root '{}': {}",
                pets_root.display(),
                error
            ))
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("walk pets root: {}", error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if only_pet_id.is_some_and(|wanted| name != wanted) {
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let meta = match entry.path().symlink_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                inventory.blocked.push(BlockedCandidate {
                    display_name: name,
                    provider: ProviderKey::Codex,
                    scope_label: Some("Pets".to_string()),
                    reason: format!("pet directory is unreadable: {}", error),
                });
                continue;
            }
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            if meta.file_type().is_symlink() {
                inventory.blocked.push(BlockedCandidate {
                    display_name: name,
                    provider: ProviderKey::Codex,
                    scope_label: Some("Pets".to_string()),
                    reason: "pet directory is a symlink".to_string(),
                });
            }
            continue;
        }
        match capture_pet(&entry.path()) {
            Ok(captured) => {
                if !captured.extra_files.is_empty() {
                    inventory.warnings.push(format!(
                        "pet '{}' has extra files that will not sync: {}",
                        captured.pet_id,
                        captured.extra_files.join(", ")
                    ));
                }
                match pet_resource_from_capture(&captured) {
                    Ok(resource) => {
                        let wrapped = match captured.format {
                            CodexPetFormat::V1 => CatalogResource::CodexPetV1(resource),
                            CodexPetFormat::V2 => CatalogResource::CodexPetV2(resource),
                        };
                        inventory.resources.insert(
                            wrapped.resource_id(),
                            LocalResourceRecord {
                                resource: wrapped,
                                target: LocalCatalogTargetId::PetDirectory {
                                    canonical_home: codex_home.to_string_lossy().to_string(),
                                    pet_id: captured.pet_id.clone(),
                                },
                                local_modified_at: latest_modified_secs(&entry.path()),
                                target_path: Some(entry.path()),
                                warnings: Vec::new(),
                                skill_tree: None,
                                pet: Some(captured),
                            },
                        );
                    }
                    Err(reason) => inventory.blocked.push(BlockedCandidate {
                        display_name: name,
                        provider: ProviderKey::Codex,
                        scope_label: Some("Pets".to_string()),
                        reason,
                    }),
                }
            }
            Err(reason) => inventory.blocked.push(BlockedCandidate {
                display_name: name,
                provider: ProviderKey::Codex,
                scope_label: Some("Pets".to_string()),
                reason,
            }),
        }
    }
    Ok(())
}

/// Which portion of the local surface one inventory call observes. Narrowing
/// is a cost gate, not a security boundary: a pets inventory must not shell
/// out to the plugin CLIs, while a skills inventory still runs plugin
/// discovery because plugin ownership classifies (excludes) plugin-owned
/// skill directories — that exclusion is not optional.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogInventoryFilter {
    #[cfg(test)]
    Everything,
    Kind(CatalogResourceKindTag),
    /// Observe one registered target. A changed resource ID is retained when
    /// it still occupies that target; pet v1/v2 occupants are both relevant
    /// because the formats share a directory.
    Resource {
        kind: CatalogResourceKindTag,
        target: LocalCatalogTargetId,
    },
}

impl CatalogInventoryFilter {
    fn selected_kind(&self) -> Option<CatalogResourceKindTag> {
        match self {
            #[cfg(test)]
            CatalogInventoryFilter::Everything => None,
            CatalogInventoryFilter::Kind(kind) | CatalogInventoryFilter::Resource { kind, .. } => {
                Some(*kind)
            }
        }
    }

    fn wants_plugin_discovery(&self) -> bool {
        match self {
            #[cfg(test)]
            CatalogInventoryFilter::Everything => true,
            CatalogInventoryFilter::Kind(kind) | CatalogInventoryFilter::Resource { kind, .. } => {
                matches!(
                    kind,
                    CatalogResourceKindTag::AgentSkill
                        | CatalogResourceKindTag::PluginInstallIntent
                )
            }
        }
    }

    fn wants_skill_roots(&self) -> bool {
        self.selected_kind()
            .is_none_or(|kind| kind == CatalogResourceKindTag::AgentSkill)
    }

    fn wants_pets(&self) -> bool {
        self.selected_kind()
            .is_none_or(|kind| kind.pet_format().is_some())
    }

    fn keeps(&self, record: &LocalResourceRecord) -> bool {
        match self {
            #[cfg(test)]
            CatalogInventoryFilter::Everything => true,
            CatalogInventoryFilter::Kind(wanted) => *wanted == record.resource.kind_tag(),
            CatalogInventoryFilter::Resource { kind, target } => {
                if &record.target != target {
                    return false;
                }
                *kind == record.resource.kind_tag()
                    || kind.pet_format().is_some()
                        && record.resource.kind_tag().pet_format().is_some()
            }
        }
    }
}

/// Run one ownership-ordered inventory over the resolved context, observing
/// only the surface the filter selects.
pub fn inventory_local(
    context: &ResolvedCatalogContext,
    filter: CatalogInventoryFilter,
    runner: &dyn PluginProcessRunner,
) -> Result<CatalogLocalInventory, CatalogBlocker> {
    let mut inventory = CatalogLocalInventory {
        resources: BTreeMap::new(),
        blocked: Vec::new(),
        plugin_owned_skills: Vec::new(),
        warnings: Vec::new(),
        plugin_inventories: BTreeMap::new(),
    };

    // 1-2. Plugins first, so plugin ownership exists before skill
    // classification.
    let mut ownership_roots: Vec<PathBuf> = Vec::new();
    let mut owner_labels: BTreeMap<PathBuf, String> = BTreeMap::new();
    let discovery_providers: &[ProviderKey] = if filter.wants_plugin_discovery() {
        &[ProviderKey::Codex, ProviderKey::Claude]
    } else {
        &[]
    };
    for &provider in discovery_providers {
        let Some(profile) = context.profile(provider) else {
            continue;
        };
        let adapter = adapter_for(provider);
        let provider_inventory = adapter.discover_plugins(&profile.canonical_home, runner)?;
        inventory
            .warnings
            .extend(provider_inventory.warnings.iter().cloned());
        for root in provider_inventory.ownership_roots() {
            let canonical = canonical_or_self(&root);
            let owner = provider_inventory
                .plugins
                .iter()
                .find(|observation| {
                    observation
                        .install_root
                        .as_ref()
                        .is_some_and(|install_root| canonical_or_self(install_root) == canonical)
                })
                .map(|observation| observation.plugin_id.clone())
                .or_else(|| {
                    provider_inventory
                        .marketplaces
                        .iter()
                        .find(|marketplace| {
                            marketplace.root.as_ref().is_some_and(|marketplace_root| {
                                canonical_or_self(marketplace_root) == canonical
                            })
                        })
                        .map(|marketplace| format!("marketplace {}", marketplace.name))
                })
                .unwrap_or_else(|| "a plugin".to_string());
            owner_labels.insert(canonical.clone(), owner);
            ownership_roots.push(canonical);
        }
        for observation in &provider_inventory.plugins {
            let Some(resource) = adapter.portable_intent(observation, &provider_inventory) else {
                continue;
            };
            let wrapped = CatalogResource::PluginInstallIntent(resource);
            inventory.resources.insert(
                wrapped.resource_id(),
                LocalResourceRecord {
                    resource: wrapped,
                    target: LocalCatalogTargetId::Plugin {
                        provider,
                        canonical_home: profile.canonical_home.to_string_lossy().to_string(),
                        namespace: observation.namespace.clone(),
                        plugin_id: observation.plugin_id.clone(),
                    },
                    target_path: None,
                    local_modified_at: None,
                    warnings: Vec::new(),
                    skill_tree: None,
                    pet: None,
                },
            );
        }
        inventory
            .plugin_inventories
            .insert(provider, provider_inventory);
    }

    // 3-6. Skills from the fixed roots, then pets.
    if filter.wants_skill_roots() {
        for root in &context.skill_roots {
            let scan = match &filter {
                CatalogInventoryFilter::Resource {
                    target:
                        LocalCatalogTargetId::SkillDirectory {
                            scope,
                            canonical_root,
                            install_directory,
                        },
                    ..
                } if *scope == root.scope
                    && Path::new(canonical_root) == root.canonical_root.as_path() =>
                {
                    Some(Some(install_directory.as_str()))
                }
                CatalogInventoryFilter::Resource { .. } => None,
                _ => Some(None),
            };
            let Some(only_install_directory) = scan else {
                continue;
            };
            scan_skill_root(
                root.scope,
                &root.canonical_root,
                only_install_directory,
                &ownership_roots,
                &owner_labels,
                &mut inventory,
            )
            .map_err(|detail| CatalogBlocker::InventoryFailed { detail })?;
        }
    }
    if filter.wants_pets() {
        if let Some(codex) = &context.codex {
            let scan = match &filter {
                CatalogInventoryFilter::Resource {
                    target:
                        LocalCatalogTargetId::PetDirectory {
                            canonical_home,
                            pet_id,
                        },
                    ..
                } if Path::new(canonical_home) == codex.canonical_home.as_path() => {
                    Some(Some(pet_id.as_str()))
                }
                CatalogInventoryFilter::Resource { .. } => None,
                _ => Some(None),
            };
            if let Some(only_pet_id) = scan {
                scan_pets(&codex.canonical_home, only_pet_id, &mut inventory)
                    .map_err(|detail| CatalogBlocker::InventoryFailed { detail })?;
            }
        }
    }

    // A narrowed inventory serves exactly one kind: pet scanning yields both
    // formats and plugin discovery yields intents during a skills pass, so
    // drop every record the filter did not ask for.
    inventory.resources.retain(|_, record| filter.keeps(record));

    // Equal names across the two Codex roots stay distinct resources; the
    // duplication is reported for review labels.
    let mut by_name: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
    for record in inventory.resources.values() {
        if let CatalogResource::AgentSkill(skill) = &record.resource {
            if skill.descriptor.scope.provider() == ProviderKey::Codex {
                by_name
                    .entry(skill.descriptor.effective_name.clone())
                    .or_default()
                    .insert(skill.descriptor.scope.display_label());
            }
        }
    }
    for (name, roots) in by_name {
        if roots.len() > 1 {
            inventory.warnings.push(format!(
                "skill '{}' exists in both the Codex home and shared-user roots; both stay separate resources",
                name
            ));
        }
    }

    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::super::adapters::{CmdOutput, PluginProcessRunner};
    use super::super::codex_pet::fixtures::webp_atlas;
    use super::super::domain::{
        pet_resource_id, skill_resource_id, CatalogProfilesDocument, LocalCatalogProfile,
        LocalCatalogProfileId,
    };
    use super::super::profiles::resolve_profile_context;
    use super::*;
    use std::collections::BTreeSet;

    pub struct EmptyRunner;
    impl PluginProcessRunner for EmptyRunner {
        fn run(
            &self,
            provider: ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<CmdOutput, String> {
            assert_eq!(provider, ProviderKey::Codex);
            let stdout = if argv.contains(&"marketplace".to_string()) {
                r#"{"marketplaces":[]}"#
            } else {
                r#"{"installed":[]}"#
            };
            Ok(CmdOutput {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            })
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn context_for(temp: &tempfile::TempDir) -> ResolvedCatalogContext {
        // The default `~/.codex` home, so the shared `~/.agents/skills` root
        // resolves alongside the profile's own root.
        let user_home = temp.path().join("user-home");
        let codex_home = user_home.join(".codex");
        fs::create_dir_all(&codex_home).unwrap();
        let profile_id = LocalCatalogProfileId::generate().unwrap();
        let profiles = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![LocalCatalogProfile {
                profile_id: profile_id.clone(),
                provider_key: ProviderKey::Codex,
                display_name: "Codex".to_string(),
                configured_home: codex_home.to_string_lossy().to_string(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            }],
        };
        resolve_profile_context(&profiles, &profile_id, ProviderKey::Codex, &user_home).unwrap()
    }

    #[test]
    fn codex_scans_only_its_two_fixed_roots_and_ignores_project_lookalikes() {
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        let user_home = temp.path().join("user-home");

        write(
            &codex_home.join("skills/review/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        write(
            &user_home.join(".agents/skills/review/SKILL.md"),
            b"---\nname: review\n---\nshared variant\n",
        );
        // Provider-managed and hidden namespaces are excluded.
        write(
            &codex_home.join("skills/.system/internal/SKILL.md"),
            b"---\nname: internal\n---\n",
        );
        // Project-local lookalike trees must never be scanned.
        for lookalike in [
            "project/.skill/hidden/SKILL.md",
            "project/.skills/hidden/SKILL.md",
            "project/.agent/ssd/hidden/SKILL.md",
            "project/.agents/skills/hidden/SKILL.md",
            "project/.claude/skills/hidden/SKILL.md",
            "project/skills/hidden/SKILL.md",
        ] {
            write(&temp.path().join(lookalike), b"---\nname: hidden\n---\n");
        }

        let inventory =
            inventory_local(&context, CatalogInventoryFilter::Everything, &EmptyRunner).unwrap();
        let ids: BTreeSet<_> = inventory.resources.keys().cloned().collect();
        // Equal names across the two Codex roots stay two resources.
        assert!(ids.contains(&skill_resource_id(
            AgentSkillScope::CodexHome,
            "review",
            "review"
        )));
        assert!(ids.contains(&skill_resource_id(
            AgentSkillScope::CodexSharedUser,
            "review",
            "review"
        )));
        assert_eq!(ids.len(), 2, "nothing else was discovered: {:?}", ids);
        assert!(inventory
            .warnings
            .iter()
            .any(|warning| warning.contains("both the Codex home and shared-user roots")));
    }

    #[test]
    fn pets_are_discovered_with_distinct_v1_and_v2_identities() {
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        let atlas = webp_atlas(CodexPetFormat::V1);
        write(
            &codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        );
        write(&codex_home.join("pets/duck/spritesheet.webp"), &atlas);
        let inventory =
            inventory_local(&context, CatalogInventoryFilter::Everything, &EmptyRunner).unwrap();
        assert!(inventory
            .resources
            .contains_key(&pet_resource_id(CodexPetFormat::V1, "duck")));
        assert!(!inventory
            .resources
            .contains_key(&pet_resource_id(CodexPetFormat::V2, "duck")));
    }

    #[test]
    fn case_folded_collisions_within_one_root_block_both_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        write(
            &codex_home.join("skills/Review/SKILL.md"),
            b"---\nname: Review\n---\n",
        );
        write(
            &codex_home.join("skills/review2/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        // "Review" vs declared "review" collide case-folded in one root.
        let inventory =
            inventory_local(&context, CatalogInventoryFilter::Everything, &EmptyRunner).unwrap();
        assert_eq!(inventory.blocked.len(), 2);
        assert!(inventory
            .blocked
            .iter()
            .all(|blocked| blocked.reason.contains("collides")));
    }

    #[test]
    fn a_pets_inventory_never_launches_provider_clis() {
        struct PanicRunner;
        impl PluginProcessRunner for PanicRunner {
            fn run(
                &self,
                _provider: ProviderKey,
                _home: &Path,
                _argv: &[String],
            ) -> Result<CmdOutput, String> {
                panic!("a pets inventory must not launch provider CLIs");
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        write(
            &codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        );
        write(
            &codex_home.join("pets/duck/spritesheet.webp"),
            &webp_atlas(CodexPetFormat::V1),
        );
        write(
            &codex_home.join("pets/drake/pet.json"),
            br#"{"id":"drake","displayName":"Drake","description":"A drake.","spritesheetPath":"spritesheet.webp"}"#,
        );
        write(
            &codex_home.join("pets/drake/spritesheet.webp"),
            &webp_atlas(CodexPetFormat::V2),
        );
        // A skill in the root proves the skills scan is skipped too.
        write(
            &codex_home.join("skills/review/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        let inventory = inventory_local(
            &context,
            CatalogInventoryFilter::Kind(CatalogResourceKindTag::CodexPetV1),
            &PanicRunner,
        )
        .unwrap();
        // One kind means exactly one format: the v2 drake is dropped.
        assert!(inventory
            .resources
            .contains_key(&pet_resource_id(CodexPetFormat::V1, "duck")));
        assert_eq!(inventory.resources.len(), 1, "only the v1 pet survives");
    }

    #[test]
    fn a_skills_inventory_still_excludes_plugin_owned_directories() {
        struct OwningRunner {
            owned_root: PathBuf,
        }
        impl PluginProcessRunner for OwningRunner {
            fn run(
                &self,
                provider: ProviderKey,
                _home: &Path,
                argv: &[String],
            ) -> Result<CmdOutput, String> {
                assert_eq!(provider, ProviderKey::Codex);
                let stdout = if argv.contains(&"marketplace".to_string()) {
                    format!(
                        r#"{{"marketplaces":[{{"name":"acme","marketplaceSource":{{"sourceType":"git","source":"https://example.com/acme.git"}},"root":"{}"}}]}}"#,
                        self.owned_root.display()
                    )
                } else {
                    r#"{"installed":[]}"#.to_string()
                };
                Ok(CmdOutput {
                    success: true,
                    stdout,
                    stderr: String::new(),
                })
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        write(
            &codex_home.join("skills/owned/SKILL.md"),
            b"---\nname: owned\n---\n",
        );
        write(
            &codex_home.join("skills/independent/SKILL.md"),
            b"---\nname: independent\n---\n",
        );
        let inventory = inventory_local(
            &context,
            CatalogInventoryFilter::Kind(CatalogResourceKindTag::AgentSkill),
            &OwningRunner {
                owned_root: codex_home.join("skills/owned"),
            },
        )
        .unwrap();
        assert!(inventory
            .plugin_owned_skills
            .iter()
            .any(|owned| owned.name == "owned"));
        let ids: BTreeSet<_> = inventory.resources.keys().cloned().collect();
        assert!(ids.contains(&skill_resource_id(
            AgentSkillScope::CodexHome,
            "independent",
            "independent"
        )));
        assert!(!ids.contains(&skill_resource_id(
            AgentSkillScope::CodexHome,
            "owned",
            "owned"
        )));
    }

    #[test]
    fn a_resource_inventory_reads_only_the_registered_target() {
        let temp = tempfile::tempdir().unwrap();
        let context = context_for(&temp);
        let codex_home = context.codex.as_ref().unwrap().canonical_home.clone();
        write(
            &codex_home.join("skills/review/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        write(
            &codex_home.join("skills/unrelated/SKILL.md"),
            b"---\nname: unrelated\n---\n",
        );
        // This would be reported as a blocked candidate by a broad scan.
        write(
            &codex_home.join("skills/bad name/SKILL.md"),
            b"---\nname: bad-name\n---\n",
        );

        let inventory = inventory_local(
            &context,
            CatalogInventoryFilter::Resource {
                kind: CatalogResourceKindTag::AgentSkill,
                target: LocalCatalogTargetId::SkillDirectory {
                    scope: AgentSkillScope::CodexHome,
                    canonical_root: codex_home.join("skills").to_string_lossy().to_string(),
                    install_directory: "review".to_string(),
                },
            },
            &EmptyRunner,
        )
        .unwrap();

        assert_eq!(inventory.resources.len(), 1);
        assert!(inventory.resources.contains_key(&skill_resource_id(
            AgentSkillScope::CodexHome,
            "review",
            "review"
        )));
        assert!(inventory.blocked.is_empty());
    }
}
