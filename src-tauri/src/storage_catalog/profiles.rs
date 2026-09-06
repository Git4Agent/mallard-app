//! Catalog profile resolution: the selected provider home, canonicalized, and
//! the machine-local target roots its registered adapter exposes for it.
//!
//! Every inventory, plan, and apply re-resolves the selected profile from its
//! record, canonicalizes the home, asks the adapter for the target roots, and
//! rejects unsafe shapes. No project binding, checkout, working directory, or
//! repository search ever discovers a root here. Canonically equal or nested
//! roots declared by one capability fail with `overlapping_targets` before
//! any inventory runs, so one physical directory can never carry two
//! portable root-class identities.

use std::fs;
use std::path::{Component, Path, PathBuf};

use super::adapters::{CatalogProviderAdapter, ResolvedCatalogTarget};
use super::domain::{
    CatalogBlocker, CatalogProfilePin, CatalogProfilesDocument, CatalogTargetKey, CatalogTargetPin,
    LocalCatalogProfileId, ProviderKey,
};
use super::registry::{CatalogCapability, CatalogRegistry};

#[derive(Clone, Debug)]
pub struct ResolvedCatalogProfile {
    pub profile_id: LocalCatalogProfileId,
    pub provider: ProviderKey,
    pub revision: u64,
    pub canonical_home: PathBuf,
}

/// The complete resolved view of one selected Library provider profile: the
/// canonical home and every target root the adapter exposes for it.
#[derive(Clone, Debug)]
pub struct ResolvedCatalogContext {
    pub profile: ResolvedCatalogProfile,
    pub targets: Vec<ResolvedCatalogTarget>,
}

impl ResolvedCatalogContext {
    pub fn target(&self, key: &CatalogTargetKey) -> Option<&ResolvedCatalogTarget> {
        self.targets.iter().find(|target| &target.target == key)
    }

    /// The resolved roots one capability may operate on, in the capability's
    /// declared order. A declared target the adapter did not resolve for
    /// this home (a shared root that belongs to another profile) is absent.
    pub fn targets_for(&self, capability: &CatalogCapability) -> Vec<&ResolvedCatalogTarget> {
        capability
            .descriptor
            .targets
            .iter()
            .filter_map(|key| self.target(key))
            .collect()
    }

    pub fn profile_pins(&self) -> Vec<CatalogProfilePin> {
        vec![CatalogProfilePin {
            profile_id: self.profile.profile_id.clone(),
            provider: self.profile.provider.clone(),
            revision: self.profile.revision,
            canonical_home: self.profile.canonical_home.to_string_lossy().to_string(),
        }]
    }

    pub fn target_pins(&self) -> Vec<CatalogTargetPin> {
        self.targets
            .iter()
            .map(|target| CatalogTargetPin {
                target: target.target.clone(),
                canonical_root: target.canonical_root.to_string_lossy().to_string(),
            })
            .collect()
    }

    /// Fail when the current resolution no longer matches a plan's pins.
    pub fn verify_pins(
        &self,
        profile_pins: &[CatalogProfilePin],
        target_pins: &[CatalogTargetPin],
    ) -> Result<(), String> {
        if self.profile_pins() != profile_pins {
            return Err(
                "catalog profiles changed after review; refresh and review again".to_string(),
            );
        }
        if self.target_pins() != target_pins {
            return Err(
                "catalog target roots changed after review; refresh and review again".to_string(),
            );
        }
        Ok(())
    }
}

/// Canonicalize a possibly not-yet-created path: resolve the deepest existing
/// ancestor and re-append the remaining components, rejecting symlinked
/// ancestors so a link can never smuggle a root elsewhere.
pub fn prospective_canonical(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("'{}' must be absolute", path.display()));
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(_) => {
                // The final component may itself be a symlinked home;
                // canonicalize resolves it. Symlinks are only rejected as
                // missing-ancestor traversal below.
                let mut resolved = fs::canonicalize(&cursor)
                    .map_err(|error| format!("resolve '{}': {}", cursor.display(), error))?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = cursor.parent() else {
                    return Err(format!("'{}' has no existing ancestor", path.display()));
                };
                let Some(name) = cursor.file_name() else {
                    return Err(format!("'{}' has an unusable component", path.display()));
                };
                missing.push(name.to_os_string());
                cursor = parent.to_path_buf();
            }
            Err(error) => {
                return Err(format!("inspect '{}': {}", cursor.display(), error));
            }
        }
    }
}

fn resolve_profile(
    document: &CatalogProfilesDocument,
    profile_id: &LocalCatalogProfileId,
    expected_provider: &ProviderKey,
) -> Result<ResolvedCatalogProfile, CatalogBlocker> {
    let Some(profile) = document.profile(profile_id) else {
        return Err(CatalogBlocker::ProfileUnavailable {
            detail: format!("catalog profile '{}' does not exist", profile_id),
        });
    };
    if &profile.provider != expected_provider {
        return Err(CatalogBlocker::ProfileUnavailable {
            detail: format!(
                "catalog profile '{}' is a {} profile, not {}",
                profile_id, profile.provider, expected_provider
            ),
        });
    }
    let configured = PathBuf::from(&profile.configured_home);
    let meta =
        fs::symlink_metadata(&configured).map_err(|error| CatalogBlocker::ProfileUnavailable {
            detail: format!(
                "catalog profile home '{}' is unavailable: {}",
                configured.display(),
                error
            ),
        })?;
    if !meta.is_dir() && !meta.file_type().is_symlink() {
        return Err(CatalogBlocker::ProfileUnavailable {
            detail: format!(
                "catalog profile home '{}' is not a directory",
                configured.display()
            ),
        });
    }
    let canonical_home =
        fs::canonicalize(&configured).map_err(|error| CatalogBlocker::ProfileUnavailable {
            detail: format!(
                "resolve catalog profile home '{}': {}",
                configured.display(),
                error
            ),
        })?;
    Ok(ResolvedCatalogProfile {
        profile_id: profile.profile_id.clone(),
        provider: profile.provider.clone(),
        revision: profile.revision,
        canonical_home,
    })
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

/// The adapter descriptor is the source of truth for the default home name.
/// Catalog profiles remain catalog-owned; this only resolves the same local
/// directory that project setup presents by default.
pub(super) fn default_profile_home(
    adapter: &dyn CatalogProviderAdapter,
    os_user_home: &Path,
) -> Option<PathBuf> {
    adapter
        .descriptor()
        .default_profile_directory
        .map(|directory| os_user_home.join(directory))
}

/// Resolve exactly one provider profile for a scope: the canonical home plus
/// every target root the registered adapter exposes for it. This is the
/// production shape for catalog resources and never scans a project
/// directory.
pub fn resolve_profile_context(
    registry: &CatalogRegistry,
    profiles: &CatalogProfilesDocument,
    profile_id: &LocalCatalogProfileId,
    provider: &ProviderKey,
    os_user_home: &Path,
) -> Result<ResolvedCatalogContext, CatalogBlocker> {
    let adapter =
        registry
            .provider(provider)
            .ok_or_else(|| CatalogBlocker::UnsupportedProvider {
                provider: provider.to_string(),
            })?;
    let profile = resolve_profile(profiles, profile_id, provider)?;
    let targets = adapter.resolve_targets(&profile.canonical_home, os_user_home)?;
    let context = ResolvedCatalogContext { profile, targets };
    // Two roots one capability may scan must never coincide or nest, so one
    // physical directory can never carry two portable root-class identities.
    for capability in registry
        .capabilities()
        .filter(|capability| capability.provider() == provider)
    {
        let roots: Vec<&Path> = context
            .targets_for(capability)
            .iter()
            .map(|target| target.canonical_root.as_path())
            .collect();
        for (index, left) in roots.iter().enumerate() {
            if roots[index + 1..]
                .iter()
                .any(|right| paths_overlap(left, right))
            {
                return Err(CatalogBlocker::OverlappingTargets {
                    roots: roots
                        .iter()
                        .map(|root| root.to_string_lossy().to_string())
                        .collect(),
                });
            }
        }
    }
    Ok(context)
}

/// Validate a profile home path for creation: absolute, clean components, and
/// not inside the Mallard state root.
pub fn validate_new_profile_home(mallard_root: &Path, home: &str) -> Result<PathBuf, String> {
    mallard_domain::validate_absolute_clean_path("catalog profile home", home)?;
    let path = PathBuf::from(home);
    if path.components().any(|component| {
        !matches!(
            component,
            Component::RootDir | Component::Prefix(_) | Component::Normal(_)
        )
    }) {
        return Err("catalog profile home contains unsupported components".to_string());
    }
    let prospective = prospective_canonical(&path)?;
    if paths_overlap(&prospective, mallard_root) {
        return Err("catalog profile home overlaps the Mallard state root".to_string());
    }
    Ok(prospective)
}

#[cfg(test)]
mod tests {
    use super::super::domain::{CatalogCoordinate, CatalogProviderConfig, LocalCatalogProfile};
    use super::super::registry::test_support;
    use super::*;

    fn provider(key: &str) -> ProviderKey {
        ProviderKey::parse(key).unwrap()
    }

    fn profile(
        provider: &ProviderKey,
        home: &Path,
    ) -> (LocalCatalogProfileId, LocalCatalogProfile) {
        let profile_id = LocalCatalogProfileId::generate().unwrap();
        (
            profile_id.clone(),
            LocalCatalogProfile {
                profile_id,
                provider: provider.clone(),
                display_name: provider.to_string(),
                configured_home: home.to_string_lossy().to_string(),
                provider_config: CatalogProviderConfig::version_1(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            },
        )
    }

    fn document(profile: LocalCatalogProfile) -> CatalogProfilesDocument {
        CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![profile],
        }
    }

    fn target_keys(context: &ResolvedCatalogContext) -> Vec<String> {
        context
            .targets
            .iter()
            .map(|target| target.target.to_string())
            .collect()
    }

    #[test]
    fn claude_profiles_resolve_only_their_own_roots() {
        let temp = tempfile::tempdir().unwrap();
        let claude_home = temp.path().join("claude");
        std::fs::create_dir_all(&claude_home).unwrap();
        let (claude_id, claude_profile) = profile(&provider("claude"), &claude_home);
        let context = resolve_profile_context(
            &CatalogRegistry::production(),
            &document(claude_profile),
            &claude_id,
            &provider("claude"),
            &temp.path().join("user-home"),
        )
        .unwrap();
        assert_eq!(
            target_keys(&context),
            vec!["provider-home.skills", "user.plugins"]
        );
        assert_eq!(context.targets[0].root_class, "provider-home");
    }

    #[test]
    fn the_default_codex_home_resolves_the_shared_root() {
        let temp = tempfile::tempdir().unwrap();
        let user_home = temp.path().join("user-home");
        let codex_home = user_home.join(".codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        let (codex_id, codex_profile) = profile(&provider("codex"), &codex_home);
        let registry = CatalogRegistry::production();
        let context = resolve_profile_context(
            &registry,
            &document(codex_profile),
            &codex_id,
            &provider("codex"),
            &user_home,
        )
        .unwrap();
        assert_eq!(
            target_keys(&context),
            vec![
                "provider-home.skills",
                "shared-user.skills",
                "user.plugins",
                "desktop.pets"
            ]
        );
        assert!(context
            .target(&CatalogTargetKey::parse("shared-user.skills").unwrap())
            .unwrap()
            .canonical_root
            .ends_with(".agents/skills"));

        // A capability sees exactly its declared roots, in declared order.
        let skills = registry
            .capability(
                &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 1).unwrap(),
            )
            .unwrap();
        let roots: Vec<&str> = context
            .targets_for(&skills)
            .iter()
            .map(|target| target.root_class.as_str())
            .collect();
        assert_eq!(roots, vec!["provider-home", "shared-user"]);
        assert_eq!(context.target_pins().len(), 4);
    }

    #[test]
    fn a_non_default_codex_home_resolves_only_its_own_root() {
        let temp = tempfile::tempdir().unwrap();
        let user_home = temp.path().join("user-home");
        let conf2_home = user_home.join("codex-conf2");
        std::fs::create_dir_all(&conf2_home).unwrap();
        let (conf2_id, conf2_profile) = profile(&provider("codex"), &conf2_home);
        let context = resolve_profile_context(
            &CatalogRegistry::production(),
            &document(conf2_profile),
            &conf2_id,
            &provider("codex"),
            &user_home,
        )
        .unwrap();
        assert_eq!(
            target_keys(&context),
            vec!["provider-home.skills", "user.plugins", "desktop.pets"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn equal_or_nested_active_roots_block_before_inventory() {
        let temp = tempfile::tempdir().unwrap();
        // `~/.codex` as a symlink to `~/.agents` makes the default Codex-home
        // skills root equal to the shared-user root: one physical directory,
        // two identities.
        let user_home = temp.path().join("user-home");
        std::fs::create_dir_all(user_home.join(".agents")).unwrap();
        std::os::unix::fs::symlink(user_home.join(".agents"), user_home.join(".codex")).unwrap();
        let (codex_id, codex_profile) = profile(&provider("codex"), &user_home.join(".codex"));
        let error = resolve_profile_context(
            &CatalogRegistry::production(),
            &document(codex_profile),
            &codex_id,
            &provider("codex"),
            &user_home,
        )
        .unwrap_err();
        assert!(matches!(error, CatalogBlocker::OverlappingTargets { .. }));
        assert!(error.to_string().contains("overlapping_targets"));
    }

    #[test]
    fn missing_profile_homes_and_unknown_providers_fail_as_typed_blockers() {
        let temp = tempfile::tempdir().unwrap();
        let (codex_id, codex_profile) = profile(&provider("codex"), &temp.path().join("missing"));
        let document = document(codex_profile);
        let error = resolve_profile_context(
            &CatalogRegistry::production(),
            &document,
            &codex_id,
            &provider("codex"),
            temp.path(),
        )
        .unwrap_err();
        assert!(matches!(error, CatalogBlocker::ProfileUnavailable { .. }));

        // A provider this build does not register cannot resolve at all,
        // even though its profile record is structurally fine.
        let error = resolve_profile_context(
            &CatalogRegistry::production(),
            &document,
            &codex_id,
            &provider("cursor"),
            temp.path(),
        )
        .unwrap_err();
        assert!(matches!(error, CatalogBlocker::UnsupportedProvider { .. }));
    }

    #[test]
    fn the_test_provider_resolves_its_own_target_through_the_same_path() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("testprov-home");
        std::fs::create_dir_all(&home).unwrap();
        let (profile_id, record) = profile(&test_support::provider_key(), &home);
        let context = resolve_profile_context(
            &CatalogRegistry::with_test_capability(),
            &document(record),
            &profile_id,
            &test_support::provider_key(),
            temp.path(),
        )
        .unwrap();
        assert_eq!(target_keys(&context), vec![test_support::TARGET]);
        assert!(context.targets[0].canonical_root.ends_with("notes"));
        assert!(CatalogRegistry::production()
            .provider(&test_support::provider_key())
            .is_none());
    }
}
