//! Ownership-ordered local catalog inventory for one registered capability.
//!
//! Discovery inputs are exactly the resolved profile's adapter-declared
//! target roots and, when a root or the capability needs it, the provider's
//! native inventory. Project checkouts, working directories, Git roots, and
//! lookalike trees are never inputs. Nothing here names a provider, kind, or
//! format: every directory is offered to the scope's codec and then to the
//! sibling versions that share its root, and every native resource comes
//! from the adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::adapters::{ProviderInventory, ResolvedCatalogTarget};
use super::codec::CapturedResource;
use super::domain::{
    warning_digest, CatalogBlocker, CatalogResourceEnvelope, CatalogResourceId,
    CatalogResourceWarning, CatalogTargetConfig, CatalogTargetIdentity,
};
use super::profiles::ResolvedCatalogContext;
use super::registry::{CaptureSupport, CatalogCapability, CatalogRegistry};

/// One discovered local resource in portable form plus the capability that
/// captured it, its machine-local target, and its capture payload.
pub struct LocalResourceRecord {
    pub resource: CatalogResourceEnvelope,
    /// The scope's own capability, or the sibling version that claimed a
    /// directory in a shared root.
    pub capability: Arc<CatalogCapability>,
    pub target: CatalogTargetConfig,
    pub target_path: Option<PathBuf>,
    /// Newest modification time anywhere under `target_path`, in Unix
    /// seconds. Reported so a row can say when the resource last changed on
    /// this machine; never part of any identity or digest.
    pub local_modified_at: Option<u64>,
    pub warnings: Vec<CatalogResourceWarning>,
    /// The exact captured payload for directory resources; `None` for
    /// provider-inventory resources.
    pub captured: Option<CapturedResource>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedCandidate {
    pub display_name: String,
    pub scope_label: Option<String>,
    pub reason: String,
}

pub struct CatalogLocalInventory {
    /// Resources of the inventoried coordinate, by resource id.
    pub resources: BTreeMap<CatalogResourceId, LocalResourceRecord>,
    /// Directory occupants a sibling version of the same format claimed in a
    /// shared target root. They are never rows of this coordinate, but
    /// conflict handling must see what sits where a Pull wants to land.
    pub siblings: BTreeMap<CatalogResourceId, LocalResourceRecord>,
    pub blocked: Vec<BlockedCandidate>,
    pub warnings: Vec<String>,
    /// The provider's native inventory snapshot, when discovery observed it.
    pub native: Option<ProviderInventory>,
}

impl CatalogLocalInventory {
    fn empty() -> Self {
        Self {
            resources: BTreeMap::new(),
            siblings: BTreeMap::new(),
            blocked: Vec::new(),
            warnings: Vec::new(),
            native: None,
        }
    }

    /// Every record occupying a target under this scope's roots, own
    /// coordinate or sibling.
    pub fn occupants(&self) -> impl Iterator<Item = &LocalResourceRecord> {
        self.resources.values().chain(self.siblings.values())
    }

    pub fn occupant(&self, target: &CatalogTargetConfig) -> Option<&LocalResourceRecord> {
        self.occupants().find(|record| &record.target == target)
    }
}

/// Bounded so a pathological tree cannot turn a list refresh into an
/// unbounded walk.
const MAX_MODIFIED_WALK_ENTRIES: usize = 1_024;

fn canonical_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The newest modification time at or under `path`, in Unix seconds.
///
/// A directory's own timestamp only moves when its direct entries change, so
/// a resource whose declaration was edited today would otherwise report the
/// day its folder was created. Symlinks are never followed — capture already
/// refuses them, and this walk must not leave the tree it was handed.
/// Entries that cannot be inspected are skipped: a missing date is honest, a
/// wrong one is not.
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
    let mut visited = 0usize;
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_MODIFIED_WALK_ENTRIES {
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

/// Classify one directory under a target root: the scope's own codec
/// captures it, or one of the sibling versions sharing that root does. The
/// same classification runs at inventory time and again before every
/// mutation, so what a review saw is what apply observes.
pub fn classify_directory(
    registry: &CatalogRegistry,
    capability: &Arc<CatalogCapability>,
    root_class: &str,
    path: &Path,
) -> Result<(Arc<CatalogCapability>, CapturedResource), String> {
    let error = match capability.codec.capture_directory(path, root_class) {
        Ok(captured) => return Ok((capability.clone(), captured)),
        Err(error) => error,
    };
    for sibling in registry.siblings(capability) {
        if let Ok(captured) = sibling.codec.capture_directory(path, root_class) {
            return Ok((sibling, captured));
        }
    }
    Err(error)
}

fn target_config(
    capability: &CatalogCapability,
    target: &ResolvedCatalogTarget,
    member: &str,
) -> Result<CatalogTargetConfig, String> {
    CatalogTargetConfig::new(
        capability.provider().clone(),
        target.target.clone(),
        &CatalogTargetIdentity {
            canonical_root: target.canonical_root.to_string_lossy().to_string(),
            root_class: target.root_class.clone(),
            member: member.to_string(),
        },
    )
}

fn directory_record(
    capability: Arc<CatalogCapability>,
    target: &ResolvedCatalogTarget,
    member: &str,
    path: PathBuf,
    captured: CapturedResource,
) -> Result<LocalResourceRecord, String> {
    let envelope = captured.to_envelope(&capability.coordinate());
    capability.codec.validate(&envelope)?;
    let target = target_config(&capability, target, member)?;
    let warnings = captured
        .warnings
        .iter()
        .map(|warning| CatalogResourceWarning {
            code: warning.code.clone(),
            digest: warning_digest(&warning.code, &envelope.resource_id, &warning.message),
            message: warning.message.clone(),
        })
        .collect();
    Ok(LocalResourceRecord {
        resource: envelope,
        capability,
        target,
        local_modified_at: latest_modified_secs(&path),
        target_path: Some(path),
        warnings,
        captured: Some(captured),
    })
}

fn block(
    inventory: &mut CatalogLocalInventory,
    target: &ResolvedCatalogTarget,
    display_name: String,
    reason: String,
) {
    inventory.blocked.push(BlockedCandidate {
        display_name,
        scope_label: Some(target.summary.clone()),
        reason,
    });
}

fn scan_target_root(
    registry: &CatalogRegistry,
    capability: &Arc<CatalogCapability>,
    target: &ResolvedCatalogTarget,
    ownership_roots: &[PathBuf],
    inventory: &mut CatalogLocalInventory,
) -> Result<(), String> {
    let root = &target.canonical_root;
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("read target root '{}': {}", root.display(), error)),
    };

    struct Candidate {
        name: String,
        path: PathBuf,
        capability: Arc<CatalogCapability>,
        captured: CapturedResource,
    }
    let mut candidates: Vec<Candidate> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("walk target root: {}", error))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            block(
                inventory,
                target,
                entry.file_name().to_string_lossy().to_string(),
                "directory name is not UTF-8".to_string(),
            );
            continue;
        };
        // Provider-managed hidden namespaces (`.system`, dot-prefixed
        // bundles) are excluded, not candidates.
        if name.starts_with('.') {
            continue;
        }
        let meta = match entry.path().symlink_metadata() {
            Ok(meta) => meta,
            Err(error) => {
                block(
                    inventory,
                    target,
                    name,
                    format!("directory is unreadable: {}", error),
                );
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            block(
                inventory,
                target,
                name,
                "directory is a symlink".to_string(),
            );
            continue;
        }
        if !meta.is_dir() {
            // Loose files in a target root are not candidates.
            continue;
        }
        // Ownership: a directory canonically inside a plugin install or
        // marketplace root belongs to that plugin. Name or content equality
        // alone is never ownership evidence.
        let canonical = canonical_or_self(&entry.path());
        if ownership_roots
            .iter()
            .any(|owner| canonical.starts_with(owner))
        {
            continue;
        }
        match classify_directory(registry, capability, &target.root_class, &entry.path()) {
            Ok((claimed, captured)) => candidates.push(Candidate {
                name,
                path: entry.path(),
                capability: claimed,
                captured,
            }),
            Err(reason) => block(inventory, target, name, reason),
        }
    }

    // Collisions inside one target root block every colliding candidate;
    // equal names across two roots are reported, never collapsed.
    let mut keyed: BTreeMap<String, usize> = BTreeMap::new();
    for candidate in &candidates {
        if let Some(key) = candidate
            .capability
            .codec
            .collision_key(&candidate.captured)
        {
            *keyed.entry(key).or_default() += 1;
        }
    }
    let coordinate = capability.coordinate();
    for candidate in candidates {
        if candidate
            .capability
            .codec
            .collision_key(&candidate.captured)
            .is_some_and(|key| keyed[&key] > 1)
        {
            block(
                inventory,
                target,
                candidate.captured.display_name,
                "name collides with another resource in the same root".to_string(),
            );
            continue;
        }
        inventory
            .warnings
            .extend(candidate.captured.notes.iter().cloned());
        let own = candidate.capability.coordinate() == coordinate;
        let name = candidate.name;
        match directory_record(
            candidate.capability,
            target,
            &name,
            candidate.path,
            candidate.captured,
        ) {
            Ok(record) => {
                let resource_id = record.resource.resource_id.clone();
                if own {
                    inventory.resources.insert(resource_id, record);
                } else {
                    inventory.siblings.insert(resource_id, record);
                }
            }
            Err(reason) => block(inventory, target, name, reason),
        }
    }
    Ok(())
}

fn native_records(
    capability: &Arc<CatalogCapability>,
    target: &ResolvedCatalogTarget,
    native: &ProviderInventory,
    inventory: &mut CatalogLocalInventory,
) {
    let coordinate = capability.coordinate();
    for envelope in capability.adapter.native_resources(native) {
        if envelope.coordinate() != coordinate {
            continue;
        }
        let member = match capability
            .codec
            .validate(&envelope)
            .and_then(|()| capability.codec.member_name(&envelope))
            .and_then(|member| target_config(capability, target, &member))
        {
            Ok(target) => target,
            Err(reason) => {
                let display_name = capability
                    .codec
                    .display_name(&envelope)
                    .unwrap_or_else(|_| envelope.resource_id.to_string());
                block(inventory, target, display_name, reason);
                continue;
            }
        };
        inventory.resources.insert(
            envelope.resource_id.clone(),
            LocalResourceRecord {
                resource: envelope,
                capability: capability.clone(),
                target: member,
                target_path: None,
                local_modified_at: None,
                warnings: Vec::new(),
                captured: None,
            },
        );
    }
}

/// Equal names across two roots stay distinct resources; the duplication is
/// reported for review labels.
fn report_duplicate_names(context: &ResolvedCatalogContext, inventory: &mut CatalogLocalInventory) {
    let mut by_key: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
    for record in inventory.resources.values() {
        let Some(captured) = &record.captured else {
            continue;
        };
        let Some(key) = record.capability.codec.collision_key(captured) else {
            continue;
        };
        let Some(target) = context.target(&record.target.target) else {
            continue;
        };
        by_key
            .entry(key)
            .or_insert_with(|| (captured.display_name.clone(), BTreeSet::new()))
            .1
            .insert(target.summary.clone());
    }
    for (name, roots) in by_key.into_values() {
        if roots.len() > 1 {
            inventory.warnings.push(format!(
                "'{}' exists in more than one root ({}); each stays a separate resource",
                name,
                roots.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
}

/// Run one ownership-ordered inventory over the resolved context for one
/// capability. Local provider state is read first when the capability captures
/// through the provider or when a directory root excludes provider-owned
/// children, so ownership exists before classification. No inventory path can
/// launch a provider process.
pub fn inventory_local(
    registry: &CatalogRegistry,
    capability: &Arc<CatalogCapability>,
    context: &ResolvedCatalogContext,
) -> Result<CatalogLocalInventory, CatalogBlocker> {
    let targets = context.targets_for(capability);
    let mut inventory = CatalogLocalInventory::empty();
    let needs_native = match capability.descriptor.capture {
        CaptureSupport::ProviderInventory => true,
        CaptureSupport::DirectoryTree => targets.iter().any(|target| target.exclude_provider_owned),
    };
    let mut ownership_roots: Vec<PathBuf> = Vec::new();
    if needs_native {
        let native = capability
            .adapter
            .discover_local(&context.profile.canonical_home)?;
        inventory.warnings.extend(native.warnings.iter().cloned());
        ownership_roots = native
            .ownership_roots()
            .iter()
            .map(|root| canonical_or_self(root))
            .collect();
        inventory.native = Some(native);
    }

    match capability.descriptor.capture {
        CaptureSupport::ProviderInventory => {
            let target = targets
                .first()
                .ok_or_else(|| CatalogBlocker::UnsupportedCoordinate {
                    detail: format!(
                        "{} has no active target under this profile",
                        capability.coordinate()
                    ),
                })?;
            let native = inventory
                .native
                .take()
                .expect("native inventory is observed for provider-inventory capture");
            native_records(capability, target, &native, &mut inventory);
            inventory.native = Some(native);
        }
        CaptureSupport::DirectoryTree => {
            for target in &targets {
                let excluded: &[PathBuf] = if target.exclude_provider_owned {
                    &ownership_roots
                } else {
                    &[]
                };
                scan_target_root(registry, capability, target, excluded, &mut inventory)
                    .map_err(|detail| CatalogBlocker::InventoryFailed { detail })?;
            }
        }
    }
    report_duplicate_names(context, &mut inventory);
    Ok(inventory)
}

#[cfg(test)]
mod tests {
    use super::super::codex_pet::fixtures::webp_atlas;
    use super::super::domain::{
        CatalogCoordinate, CatalogProfilesDocument, CatalogProviderConfig, LocalCatalogProfile,
        LocalCatalogProfileId, ProviderKey,
    };
    use super::super::profiles::resolve_profile_context;
    use super::super::registry::test_support;
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn coordinate(provider: &str, kind: &str, format: &str, version: u32) -> CatalogCoordinate {
        CatalogCoordinate::new(provider, kind, format, version).unwrap()
    }

    fn context_for(
        registry: &CatalogRegistry,
        temp: &tempfile::TempDir,
        provider: &str,
        home: &Path,
    ) -> ResolvedCatalogContext {
        fs::create_dir_all(home).unwrap();
        let provider = ProviderKey::parse(provider).unwrap();
        let profile_id = LocalCatalogProfileId::generate().unwrap();
        let profiles = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![LocalCatalogProfile {
                profile_id: profile_id.clone(),
                provider: provider.clone(),
                display_name: provider.to_string(),
                configured_home: home.to_string_lossy().to_string(),
                provider_config: CatalogProviderConfig::version_1(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            }],
        };
        resolve_profile_context(
            registry,
            &profiles,
            &profile_id,
            &provider,
            &temp.path().join("user-home"),
        )
        .unwrap()
    }

    /// The default `~/.codex` home, so the shared `~/.agents/skills` root
    /// resolves alongside the profile's own root.
    fn codex_context(temp: &tempfile::TempDir) -> (Arc<CatalogRegistry>, ResolvedCatalogContext) {
        let registry = CatalogRegistry::production();
        let home = temp.path().join("user-home").join(".codex");
        let context = context_for(&registry, temp, "codex", &home);
        (registry, context)
    }

    fn skills(registry: &CatalogRegistry) -> Arc<CatalogCapability> {
        registry
            .capability(&coordinate(
                "codex",
                "agent_skill",
                "agentskills.io.skill",
                1,
            ))
            .unwrap()
    }

    fn pets(registry: &CatalogRegistry, version: u32) -> Arc<CatalogCapability> {
        registry
            .capability(&coordinate(
                "codex",
                "desktop_pet",
                "openai.codex-pet",
                version,
            ))
            .unwrap()
    }

    fn roots_of(inventory: &CatalogLocalInventory) -> Vec<(String, String)> {
        let mut roots: Vec<(String, String)> = inventory
            .resources
            .values()
            .map(|record| {
                let identity = record.target.decode().unwrap();
                (identity.root_class, identity.member)
            })
            .collect();
        roots.sort();
        roots
    }

    #[test]
    fn codex_scans_only_its_declared_roots_and_ignores_project_lookalikes() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let codex_home = context.profile.canonical_home.clone();
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

        let inventory = inventory_local(&registry, &skills(&registry), &context).unwrap();
        // Equal names across the two Codex roots stay two resources.
        assert_eq!(
            roots_of(&inventory),
            vec![
                ("provider-home".to_string(), "review".to_string()),
                ("shared-user".to_string(), "review".to_string()),
            ]
        );
        assert!(inventory.siblings.is_empty());
        assert!(inventory
            .warnings
            .iter()
            .any(|warning| warning.contains("'review' exists in more than one root")));
        let record = inventory.resources.values().next().unwrap();
        assert!(record.local_modified_at.is_some());
        assert!(record.captured.is_some());
        assert!(
            inventory.native.is_some(),
            "ownership needs the native view"
        );
    }

    #[test]
    fn pets_are_discovered_per_version_with_cross_version_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let codex_home = context.profile.canonical_home.clone();
        write(
            &codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        );
        write(
            &codex_home.join("pets/duck/spritesheet.webp"),
            &webp_atlas(1),
        );
        write(
            &codex_home.join("pets/drake/pet.json"),
            br#"{"id":"drake","displayName":"Drake","description":"A drake.","spriteVersionNumber":2,"spritesheetPath":"spritesheet.webp"}"#,
        );
        write(
            &codex_home.join("pets/drake/spritesheet.webp"),
            &webp_atlas(2),
        );
        // A skill in the root proves the skills scan is skipped too.
        write(
            &codex_home.join("skills/review/SKILL.md"),
            b"---\nname: review\n---\n",
        );

        // A pets inventory never launches provider CLIs.
        let v1 = inventory_local(&registry, &pets(&registry, 1), &context).unwrap();
        assert_eq!(
            roots_of(&v1),
            vec![("provider-home".to_string(), "duck".to_string())]
        );
        let sibling = v1.siblings.values().next().unwrap();
        assert_eq!(sibling.resource.format_version, 2);
        assert_eq!(sibling.capability.descriptor.format_version, 2);
        assert!(v1.native.is_none());

        let v2 = inventory_local(&registry, &pets(&registry, 2), &context).unwrap();
        assert_eq!(
            roots_of(&v2),
            vec![("provider-home".to_string(), "drake".to_string())]
        );
        assert_eq!(v2.siblings.len(), 1);
        // The v2 pet occupies the target the v1 pull of `drake` would take.
        let drake_target = v2.resources.values().next().unwrap().target.clone();
        assert!(v1.occupant(&drake_target).is_some());
        assert!(v1
            .resources
            .values()
            .all(|record| record.target != drake_target));
    }

    #[test]
    fn case_folded_collisions_within_one_root_block_both_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let codex_home = context.profile.canonical_home.clone();
        write(
            &codex_home.join("skills/Review/SKILL.md"),
            b"---\nname: Review\n---\n",
        );
        write(
            &codex_home.join("skills/review2/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        // "Review" vs declared "review" collide case-folded in one root.
        let inventory = inventory_local(&registry, &skills(&registry), &context).unwrap();
        assert!(inventory.resources.is_empty());
        assert_eq!(inventory.blocked.len(), 2);
        assert!(inventory
            .blocked
            .iter()
            .all(|blocked| blocked.reason.contains("collides")));
        assert_eq!(
            inventory.blocked[0].scope_label.as_deref(),
            Some("Codex home")
        );
    }

    #[test]
    fn unreadable_symlinked_and_unrecognizable_directories_are_blocked_not_hidden() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let codex_home = context.profile.canonical_home.clone();
        write(
            &codex_home.join("skills/review/SKILL.md"),
            b"---\nname: review\n---\n",
        );
        fs::create_dir_all(codex_home.join("skills/empty")).unwrap();
        write(&codex_home.join("skills/loose.md"), b"not a skill");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            codex_home.join("skills/review"),
            codex_home.join("skills/link"),
        )
        .unwrap();
        let inventory = inventory_local(&registry, &skills(&registry), &context).unwrap();
        assert_eq!(inventory.resources.len(), 1);
        let reasons: Vec<&str> = inventory
            .blocked
            .iter()
            .map(|blocked| blocked.reason.as_str())
            .collect();
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("not a recognizable skill")));
        #[cfg(unix)]
        assert!(reasons.iter().any(|reason| reason.contains("symlink")));
    }

    #[test]
    fn a_skills_inventory_excludes_plugin_owned_directories() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let codex_home = context.profile.canonical_home.clone();
        write(
            &codex_home.join("skills/owned/SKILL.md"),
            b"---\nname: owned\n---\n",
        );
        write(
            &codex_home.join("skills/independent/SKILL.md"),
            b"---\nname: independent\n---\n",
        );
        write(
            &codex_home.join("config.toml"),
            format!(
                "[marketplaces.acme]\nsource_type=\"local\"\nsource={}\n",
                serde_json::to_string(&codex_home.join("skills/owned")).unwrap()
            )
            .as_bytes(),
        );
        let inventory = inventory_local(&registry, &skills(&registry), &context).unwrap();
        assert_eq!(
            roots_of(&inventory),
            vec![("provider-home".to_string(), "independent".to_string())]
        );
    }

    #[test]
    fn native_inventories_record_provider_resources_without_directories() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, context) = codex_context(&temp);
        let plugins = registry
            .capability(&coordinate(
                "codex",
                "plugin_install_intent",
                "mallard.plugin-install-intent",
                1,
            ))
            .unwrap();
        write(
            &context.profile.canonical_home.join("config.toml"),
            b"[plugins.\"tools@acme\"]\nenabled=true\n",
        );
        write(
            &context
                .profile
                .canonical_home
                .join("plugins/cache/acme/tools/1/.codex-plugin/plugin.json"),
            br#"{"name":"tools","version":"1"}"#,
        );
        let inventory = inventory_local(&registry, &plugins, &context).unwrap();
        assert_eq!(inventory.resources.len(), 1);
        let record = inventory.resources.values().next().unwrap();
        assert!(record.target_path.is_none());
        assert!(record.captured.is_none());
        let identity = record.target.decode().unwrap();
        assert_eq!(identity.member, "tools@acme");
        assert_eq!(identity.root_class, "user");
        assert!(inventory.native.is_some());
    }

    #[test]
    fn the_test_capability_inventories_through_the_common_path() {
        let temp = tempfile::tempdir().unwrap();
        let registry = CatalogRegistry::with_test_capability();
        let home = temp.path().join("testprov-home");
        let context = context_for(&registry, &temp, test_support::PROVIDER, &home);
        write(&home.join("notes/todo/NOTE.md"), b"# Things to do\n- one\n");
        write(&home.join("notes/bad/README.md"), b"no note here\n");
        let capability = registry
            .require_capability(&test_support::coordinate())
            .unwrap();
        let inventory = inventory_local(&registry, &capability, &context).unwrap();
        assert_eq!(inventory.resources.len(), 1);
        let record = inventory.resources.values().next().unwrap();
        assert_eq!(
            capability.codec.display_name(&record.resource).unwrap(),
            "Things to do"
        );
        assert_eq!(record.target.decode().unwrap().member, "todo");
        assert_eq!(inventory.blocked.len(), 1);
        assert_eq!(
            inventory.blocked[0].scope_label.as_deref(),
            Some("Test notes")
        );
    }
}
