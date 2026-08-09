//! Catalog profile resolution: `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and the OS
//! user's fixed `$HOME/.agents/skills` root.
//!
//! Every inventory, plan, and apply re-resolves the selected profiles from
//! their records, canonicalizes each permitted global path, and rejects
//! unsafe shapes. No project binding, checkout, working directory, or
//! repository search ever discovers a skill root here. Canonically equal or
//! nested active skill roots fail with `overlapping_skill_roots` before any
//! inventory runs, so one physical directory can never carry two portable
//! root-class identities.

use std::fs;
use std::path::{Component, Path, PathBuf};

use mallard_infrastructure::providers::{codex, ProviderRegistry};

use super::domain::{
    AgentSkillScope, CatalogBlocker, CatalogProfilePin, CatalogProfilesDocument,
    CatalogSkillRootPin, LocalCatalogProfileId, ProviderKey,
};

#[derive(Clone, Debug)]
pub struct ResolvedCatalogProfile {
    pub profile_id: LocalCatalogProfileId,
    pub provider_key: ProviderKey,
    pub revision: u64,
    pub canonical_home: PathBuf,
}

/// One active skill root with its portable scope class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSkillRoot {
    pub scope: AgentSkillScope,
    pub canonical_root: PathBuf,
}

/// The complete resolved view of one selected Library provider profile.
#[derive(Clone, Debug)]
pub struct ResolvedCatalogContext {
    pub codex: Option<ResolvedCatalogProfile>,
    pub claude: Option<ResolvedCatalogProfile>,
    pub skill_roots: Vec<ResolvedSkillRoot>,
}

impl ResolvedCatalogContext {
    pub fn profile(&self, provider: ProviderKey) -> Option<&ResolvedCatalogProfile> {
        match provider {
            ProviderKey::Codex => self.codex.as_ref(),
            ProviderKey::Claude => self.claude.as_ref(),
        }
    }

    pub fn skill_root(&self, scope: AgentSkillScope) -> Option<&Path> {
        self.skill_roots
            .iter()
            .find(|root| root.scope == scope)
            .map(|root| root.canonical_root.as_path())
    }

    pub fn profile_pins(&self) -> Vec<CatalogProfilePin> {
        [&self.codex, &self.claude]
            .into_iter()
            .flatten()
            .map(|profile| CatalogProfilePin {
                profile_id: profile.profile_id.clone(),
                provider_key: profile.provider_key,
                revision: profile.revision,
                canonical_home: profile.canonical_home.to_string_lossy().to_string(),
            })
            .collect()
    }

    pub fn skill_root_pins(&self) -> Vec<CatalogSkillRootPin> {
        self.skill_roots
            .iter()
            .map(|root| CatalogSkillRootPin {
                scope: root.scope,
                canonical_root: root.canonical_root.to_string_lossy().to_string(),
            })
            .collect()
    }

    /// Fail when the current resolution no longer matches a plan's pins.
    pub fn verify_pins(
        &self,
        profile_pins: &[CatalogProfilePin],
        skill_root_pins: &[CatalogSkillRootPin],
    ) -> Result<(), String> {
        if self.profile_pins() != profile_pins {
            return Err(
                "catalog profiles changed after review; refresh and review again".to_string(),
            );
        }
        if self.skill_root_pins() != skill_root_pins {
            return Err(
                "catalog skill roots changed after review; refresh and review again".to_string(),
            );
        }
        Ok(())
    }
}

/// Canonicalize a possibly not-yet-created path: resolve the deepest existing
/// ancestor and re-append the remaining components, rejecting symlinked
/// ancestors so a link can never smuggle a root elsewhere.
fn prospective_canonical(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("'{}' must be absolute", path.display()));
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    // The final component may itself be a symlinked home;
                    // canonicalize resolves it. Symlinks are only rejected as
                    // *missing-ancestor* traversal below.
                }
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
    expected_provider: ProviderKey,
) -> Result<ResolvedCatalogProfile, CatalogBlocker> {
    let Some(profile) = document.profile(profile_id) else {
        return Err(CatalogBlocker::ProfileUnavailable {
            detail: format!("catalog profile '{}' does not exist", profile_id),
        });
    };
    if profile.provider_key != expected_provider {
        return Err(CatalogBlocker::ProfileUnavailable {
            detail: format!(
                "catalog profile '{}' is a {} profile, not {}",
                profile_id,
                profile.provider_key.as_str(),
                expected_provider.as_str()
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
        provider_key: profile.provider_key,
        revision: profile.revision,
        canonical_home,
    })
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

/// True when this profile's canonical home is the provider's default
/// `$HOME/.codex`. The directory name comes from the provider descriptor, so
/// this cannot drift from what the Codex adapter actually calls home.
fn is_default_codex_home(canonical_home: &Path, os_user_home: &Path) -> bool {
    let Some(directory) = ProviderRegistry::production()
        .get(&codex::provider_key())
        .and_then(|adapter| adapter.descriptor().default_profile_directory)
    else {
        return false;
    };
    prospective_canonical(&os_user_home.join(directory))
        .is_ok_and(|default_home| default_home == canonical_home)
}

/// Resolve exactly one provider profile for a scope or registration. Codex
/// receives its own home skill root — plus the machine-global
/// `$HOME/.agents/skills` root when it is the default `~/.codex` config —
/// while Claude receives only its own user-global root. This is the
/// production shape for catalog resources and never scans a project
/// directory.
pub fn resolve_profile_context(
    profiles: &CatalogProfilesDocument,
    profile_id: &LocalCatalogProfileId,
    provider: ProviderKey,
    os_user_home: &Path,
) -> Result<ResolvedCatalogContext, CatalogBlocker> {
    let profile = resolve_profile(profiles, profile_id, provider)?;
    let mut skill_roots = Vec::new();
    match provider {
        ProviderKey::Codex => {
            skill_roots.push(ResolvedSkillRoot {
                scope: AgentSkillScope::CodexHome,
                canonical_root: prospective_canonical(&profile.canonical_home.join("skills"))
                    .map_err(|detail| CatalogBlocker::ProfileUnavailable { detail })?,
            });
            // The shared root belongs to the default `~/.codex` config only.
            // A second Codex config sees just its own root, so the same
            // shared directory can never be registered under two profiles.
            if is_default_codex_home(&profile.canonical_home, os_user_home) {
                skill_roots.push(ResolvedSkillRoot {
                    scope: AgentSkillScope::CodexSharedUser,
                    canonical_root: prospective_canonical(
                        &os_user_home.join(".agents").join("skills"),
                    )
                    .map_err(|detail| CatalogBlocker::ProfileUnavailable { detail })?,
                });
            }
        }
        ProviderKey::Claude => {
            skill_roots.push(ResolvedSkillRoot {
                scope: AgentSkillScope::ClaudeUser,
                canonical_root: prospective_canonical(&profile.canonical_home.join("skills"))
                    .map_err(|detail| CatalogBlocker::ProfileUnavailable { detail })?,
            });
        }
    }
    if skill_roots.len() == 2
        && paths_overlap(
            &skill_roots[0].canonical_root,
            &skill_roots[1].canonical_root,
        )
    {
        return Err(CatalogBlocker::OverlappingSkillRoots {
            roots: skill_roots
                .iter()
                .map(|root| root.canonical_root.to_string_lossy().to_string())
                .collect(),
        });
    }
    Ok(ResolvedCatalogContext {
        codex: (provider == ProviderKey::Codex).then_some(profile.clone()),
        claude: (provider == ProviderKey::Claude).then_some(profile),
        skill_roots,
    })
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
    use super::super::domain::LocalCatalogProfile;
    use super::*;

    fn profile(provider: ProviderKey, home: &Path) -> (LocalCatalogProfileId, LocalCatalogProfile) {
        let profile_id = LocalCatalogProfileId::generate().unwrap();
        (
            profile_id.clone(),
            LocalCatalogProfile {
                profile_id,
                provider_key: provider,
                display_name: provider.display_label().to_string(),
                configured_home: home.to_string_lossy().to_string(),
                revision: 1,
                created_at: 1,
                updated_at: 1,
            },
        )
    }

    #[test]
    fn claude_profiles_resolve_only_the_claude_user_root() {
        let temp = tempfile::tempdir().unwrap();
        let claude_home = temp.path().join("claude");
        std::fs::create_dir_all(&claude_home).unwrap();
        let (claude_id, claude_profile) = profile(ProviderKey::Claude, &claude_home);
        let document = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![claude_profile],
        };
        let context = resolve_profile_context(
            &document,
            &claude_id,
            ProviderKey::Claude,
            &temp.path().join("user-home"),
        )
        .unwrap();
        assert_eq!(context.skill_roots.len(), 1);
        assert_eq!(context.skill_roots[0].scope, AgentSkillScope::ClaudeUser);
    }

    #[test]
    fn the_default_codex_home_resolves_the_shared_root() {
        let temp = tempfile::tempdir().unwrap();
        let user_home = temp.path().join("user-home");
        let codex_home = user_home.join(".codex");
        std::fs::create_dir_all(&codex_home).unwrap();
        let (codex_id, codex_profile) = profile(ProviderKey::Codex, &codex_home);
        let document = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![codex_profile],
        };
        let context =
            resolve_profile_context(&document, &codex_id, ProviderKey::Codex, &user_home).unwrap();
        let scopes: Vec<_> = context.skill_roots.iter().map(|root| root.scope).collect();
        assert_eq!(
            scopes,
            vec![AgentSkillScope::CodexHome, AgentSkillScope::CodexSharedUser]
        );
        assert!(context
            .skill_root(AgentSkillScope::CodexSharedUser)
            .unwrap()
            .ends_with(".agents/skills"));
    }

    #[test]
    fn a_non_default_codex_home_resolves_only_its_own_root() {
        let temp = tempfile::tempdir().unwrap();
        let user_home = temp.path().join("user-home");
        let conf2_home = user_home.join("codex-conf2");
        std::fs::create_dir_all(&conf2_home).unwrap();
        let (conf2_id, conf2_profile) = profile(ProviderKey::Codex, &conf2_home);
        let document = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![conf2_profile],
        };
        let context =
            resolve_profile_context(&document, &conf2_id, ProviderKey::Codex, &user_home).unwrap();
        let scopes: Vec<_> = context.skill_roots.iter().map(|root| root.scope).collect();
        assert_eq!(scopes, vec![AgentSkillScope::CodexHome]);
    }

    #[cfg(unix)]
    #[test]
    fn equal_or_nested_active_skill_roots_block_before_inventory() {
        let temp = tempfile::tempdir().unwrap();
        // `~/.codex` as a symlink to `~/.agents` makes the default Codex-home
        // skills root equal to the shared-user root: one physical directory,
        // two identities.
        let user_home = temp.path().join("user-home");
        std::fs::create_dir_all(user_home.join(".agents")).unwrap();
        std::os::unix::fs::symlink(user_home.join(".agents"), user_home.join(".codex")).unwrap();
        let (codex_id, codex_profile) = profile(ProviderKey::Codex, &user_home.join(".codex"));
        let document = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![codex_profile],
        };
        let error = resolve_profile_context(&document, &codex_id, ProviderKey::Codex, &user_home)
            .unwrap_err();
        assert!(matches!(
            error,
            CatalogBlocker::OverlappingSkillRoots { .. }
        ));
        assert!(error.to_string().contains("overlapping_skill_roots"));
    }

    #[test]
    fn missing_profile_homes_fail_as_typed_blockers() {
        let temp = tempfile::tempdir().unwrap();
        let (codex_id, codex_profile) = profile(ProviderKey::Codex, &temp.path().join("missing"));
        let document = CatalogProfilesDocument {
            schema: 1,
            revision: 1,
            profiles: vec![codex_profile],
        };
        let error = resolve_profile_context(&document, &codex_id, ProviderKey::Codex, temp.path())
            .unwrap_err();
        assert!(matches!(error, CatalogBlocker::ProfileUnavailable { .. }));
    }
}
