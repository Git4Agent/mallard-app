//! Non-Tauri application boundary for scope-rowed Library resources.
//!
//! Owns catalog scopes, per-storage links, and reviewed Push, Pull, and Fork
//! operations. It reads the neutral configured-storage list but never loads a
//! project registration, project binding, thread, or repository bundle.

use bytes::Bytes;
use mallard_domain::{now_secs, StorageConfig, StorageId, SyncConfig};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_core::ConfiguredObjectStore;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::adapters::{capability_matrix, NativeProcessRunner, PluginProcessRunner};
use super::apply::{
    apply_resource_fork, execute_scope_pull_item, recover_catalog_operations, PullApplyEnvironment,
};
use super::domain::{
    pet_resource_id, provider_owns_kind, sha256_hex, CatalogActionResult, CatalogCapturedWith,
    CatalogLinkId, CatalogLinkState, CatalogPlanActionId, CatalogPlanId, CatalogResource,
    CatalogResourceForkPlan, CatalogResourceId, CatalogResourceKindTag,
    CatalogResourcePlanDocument, CatalogResourceRepositoryId, CatalogResourceWarning, CatalogScope,
    CatalogScopeId, CatalogScopeLink, CatalogScopePullPlan, CatalogScopePushPlan, CodexPetFormat,
    LibraryResourceManifest, LibraryResourceTag, LocalCatalogProfile, LocalCatalogProfileId,
    LocalCatalogTargetId, ProviderKey, PublishedResourceEntry, CATALOG_SCHEMA, MAX_PET_ATLAS_BYTES,
};
use super::inventory::{inventory_local, CatalogLocalInventory, LocalResourceRecord};
use super::persistence::CatalogStateRepository;
use super::plan_store;
use super::profiles::{
    default_profile_home, resolve_profile_context, validate_new_profile_home,
    ResolvedCatalogContext,
};
use super::resource_engine::{
    head_coordinate, storage_identity_for_config, verify_library_continuity, LibraryResourceEngine,
    ObservedLibraryHead, PublishLibraryResourceRequest,
};
use super::resource_planning::{
    build_resource_fork_plan, build_scope_pull_plan, build_scope_push_plan, conflicting_claim_pins,
    scope_label, scope_local_records, scope_resource_state, validate_scope_pull_selection,
    ScopeForkPlanInputs, ScopePullPlanInputs, ScopePushPlanInputs,
};

fn pull_outcome_accepts_remote(outcome: super::domain::CatalogActionOutcome) -> bool {
    !matches!(
        outcome,
        super::domain::CatalogActionOutcome::Failed | super::domain::CatalogActionOutcome::Partial
    )
}

fn storage_for(
    state: &LocalStateRepository,
    storage_id: &StorageId,
) -> Result<StorageConfig, String> {
    state
        .load_config()?
        .storages
        .into_iter()
        .find(|storage| &storage.id == storage_id)
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))
}

fn engine_for(
    storage: &StorageConfig,
) -> Result<LibraryResourceEngine<ConfiguredObjectStore>, String> {
    storage.validate()?;
    Ok(LibraryResourceEngine::new(
        ConfiguredObjectStore::from_config(storage)?,
    ))
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogCapabilityDto {
    pub provider: ProviderKey,
    pub kinds: Vec<CatalogResourceKindTag>,
}

/// One Catalog menu row plus the profile its label derives from.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeDto {
    pub scope: CatalogScope,
    pub profile: LocalCatalogProfile,
}

/// One row on a scope page: a resource of this kind, compared against the one
/// storage the page is scoped to.
///
/// Local-only, stored-only, and both-sides resources are all this shape,
/// because the page draws them the same way; which one a row happens to be is
/// readable from `local_present` and `remote_present`.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeRowDto {
    /// Stable across refreshes so selections survive a re-read.
    pub row_id: String,
    pub display_name: String,
    pub resource_id: Option<CatalogResourceId>,
    pub scope_label: Option<String>,
    /// `None` only when no storage is chosen: with nothing to compare
    /// against, no state can be claimed.
    pub state: Option<CatalogLinkState>,
    pub local_present: bool,
    pub local_path: Option<String>,
    pub local_modified_at: Option<u64>,
    /// Portable payload facts from this Mac's capture: how many files the
    /// resource owns, and how many bytes they total. `None` when nothing
    /// local was captured — stored-only rows, and rows whose capture is
    /// blocked.
    pub local_files: Option<u64>,
    pub local_bytes: Option<u64>,
    pub remote_present: bool,
    /// When the stored copy last changed, from its manifest entry.
    pub remote_updated_at: Option<u64>,
    /// The digest the comparison used: the local capture's when one exists,
    /// otherwise the stored version's. `None` when neither side is captured.
    pub content_sha256: Option<String>,
    /// A small PNG of the pet's idle frame, as a data URI, derived from this
    /// Mac's atlas and cached by digest. Pet rows only; `None` when nothing
    /// local can be drawn — stored-only rows fetch theirs lazily through
    /// `get_catalog_pet_preview`.
    pub preview_png: Option<String>,
    /// The pet descriptor's own sentence about itself. Pet rows only.
    pub pet_description: Option<String>,
    pub warnings: Vec<CatalogResourceWarning>,
    /// Why this resource cannot be synced at all — a symlinked skill
    /// directory, an unreadable package. Set only with `state` Blocked.
    pub blocker: Option<String>,
}

/// The page-level scope link, when this scope is linked to the page's storage.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopePageLinkDto {
    pub link_id: CatalogLinkId,
    pub revision: u64,
}

/// The complete content of one scope page: the storage it compares against,
/// the scope's link to it, and one row per resource of this kind on either
/// side.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeResourcesDto {
    pub scope: CatalogScope,
    pub profile: LocalCatalogProfile,
    /// The caller's choice when it made one; otherwise the storage this
    /// scope's links already point at, so the page opens on the pairing that
    /// exists rather than on nothing.
    pub storage_id: Option<StorageId>,
    pub storage_name: Option<String>,
    pub link: Option<CatalogScopePageLinkDto>,
    /// The stored kind repository's current revision, when one exists.
    pub storage_revision: Option<u64>,
    pub rows: Vec<CatalogScopeRowDto>,
    /// Scan notes that belong to the whole home rather than to a row.
    pub warnings: Vec<String>,
    /// Startup-recovery messages from the operations journal, surfaced at
    /// page load since no other screen hosts them.
    pub recovery: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeLinkOutcomeDto {
    pub link_id: CatalogLinkId,
    pub revision: u64,
    /// `open` when the page can simply refresh; `review_pull` when the
    /// storage holds content this machine has not reviewed yet.
    pub completion: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopePushOutcomeDto {
    pub plan_id: CatalogPlanId,
    pub published: bool,
    pub revision: Option<u64>,
    pub message: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopePullOutcomeDto {
    pub plan_id: CatalogPlanId,
    /// Whether the whole review advanced the accepted head — false when any
    /// row failed, keeping the comparison base honest.
    pub remote_accepted: bool,
    pub results: Vec<CatalogActionResult>,
}

/// The complete approval captured by a reviewed scope Push.
#[derive(Clone, Debug)]
pub struct CatalogPushApproval {
    pub plan_id: CatalogPlanId,
    pub selected_resource_ids: Vec<CatalogResourceId>,
    pub acknowledged_warning_digests: Vec<String>,
}

/// The exact per-item actions approved from a reviewed scope Pull.
#[derive(Clone, Debug)]
pub struct CatalogPullApproval {
    pub plan_id: CatalogPlanId,
    pub choices: BTreeMap<CatalogResourceId, CatalogPlanActionId>,
    pub destructive_acknowledged: Vec<CatalogResourceId>,
}

/// Production Catalog use cases over explicit machine and provider homes.
pub struct StorageCatalogService {
    state: LocalStateRepository,
    catalog: CatalogStateRepository,
    runner: Box<dyn PluginProcessRunner>,
    user_home: PathBuf,
}

impl StorageCatalogService {
    pub fn from_user_home() -> Result<Self, String> {
        let user_home = dirs::home_dir()
            .ok_or_else(|| "could not resolve the user home directory".to_string())?;
        Self::from_home_dirs(user_home.clone(), user_home, Box::new(NativeProcessRunner))
    }

    pub fn from_home_dirs(
        machine_home: impl Into<PathBuf>,
        user_home: impl Into<PathBuf>,
        runner: Box<dyn PluginProcessRunner>,
    ) -> Result<Self, String> {
        let machine_home = machine_home.into();
        let user_home = user_home.into();
        if !machine_home.is_absolute() {
            return Err(format!(
                "machine home must be absolute: '{}'",
                machine_home.display()
            ));
        }
        if !user_home.is_absolute() {
            return Err(format!(
                "provider user home must be absolute: '{}'",
                user_home.display()
            ));
        }
        Ok(Self {
            state: LocalStateRepository::from_home_dir(machine_home.clone())?,
            catalog: CatalogStateRepository::from_home_dir(machine_home)?,
            runner,
            user_home,
        })
    }

    pub fn capabilities() -> Vec<CatalogCapabilityDto> {
        capability_matrix()
            .into_iter()
            .map(|(provider, kinds)| CatalogCapabilityDto { provider, kinds })
            .collect()
    }

    pub fn list_profiles(&self) -> Result<Vec<LocalCatalogProfile>, String> {
        list_catalog_resource_profiles_with_repository(&self.catalog, &self.user_home, now_secs())
    }

    pub fn save_profile(
        &self,
        provider: ProviderKey,
        configured_home: String,
    ) -> Result<LocalCatalogProfile, String> {
        save_catalog_resource_profile_with_repository(
            &self.catalog,
            provider,
            configured_home,
            now_secs(),
        )
    }

    pub fn rename_profile(
        &self,
        profile_id: &LocalCatalogProfileId,
        display_name: String,
        expected_revision: u64,
    ) -> Result<LocalCatalogProfile, String> {
        rename_catalog_resource_profile_with_repository(
            &self.catalog,
            profile_id,
            display_name,
            expected_revision,
            now_secs(),
        )
    }

    pub fn open_profile_in_app(&self, profile_id: &LocalCatalogProfileId) -> Result<(), String> {
        let (provider, home) = profile_launch_home(&self.catalog, profile_id)?;
        match provider {
            ProviderKey::Codex => {
                mallard_infrastructure::providers::codex::actions::launch_codex_home_app(
                    &home,
                    &self.user_home,
                )
            }
            ProviderKey::Claude => {
                mallard_infrastructure::providers::claude::actions::launch_claude_home_app(&home)
            }
        }
    }

    pub fn open_profile_in_terminal(
        &self,
        profile_id: &LocalCatalogProfileId,
    ) -> Result<(), String> {
        let (provider, home) = profile_launch_home(&self.catalog, profile_id)?;
        match provider {
            ProviderKey::Codex => {
                mallard_infrastructure::providers::codex::actions::launch_codex_home_terminal(&home)
            }
            ProviderKey::Claude => {
                mallard_infrastructure::providers::claude::actions::launch_claude_home_terminal(
                    &home,
                )
            }
        }
    }

    pub fn list_scopes(&self) -> Result<Vec<CatalogScopeDto>, String> {
        let profiles = self.catalog.load_profiles()?;
        let mut scopes = self.catalog.load_scopes()?.scopes;
        scopes.sort_by_key(|scope| (scope.created_at, scope.scope_id.clone()));
        scopes
            .into_iter()
            .map(|scope| {
                let profile = profiles
                    .profile(&scope.profile_id)
                    .ok_or("a Catalog row lost its provider profile")?
                    .clone();
                Ok(CatalogScopeDto { scope, profile })
            })
            .collect()
    }

    pub fn create_scope(
        &self,
        provider: ProviderKey,
        profile_id: LocalCatalogProfileId,
        resource_kind: CatalogResourceKindTag,
    ) -> Result<CatalogScopeDto, String> {
        create_catalog_scope_with_repositories(
            &self.catalog,
            provider,
            profile_id,
            resource_kind,
            now_secs(),
        )
    }

    pub fn delete_scope(
        &self,
        scope_id: &CatalogScopeId,
        expected_revision: u64,
    ) -> Result<bool, String> {
        delete_catalog_scope_with_repositories(&self.catalog, scope_id, expected_revision)
    }

    pub fn list_scope_resources(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: Option<&StorageId>,
    ) -> Result<CatalogScopeResourcesDto, String> {
        list_catalog_scope_resources_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
        )
    }

    pub fn pet_preview(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
    ) -> Result<String, String> {
        get_catalog_pet_preview_with_repositories(
            &self.state,
            &self.catalog,
            scope_id,
            storage_id,
            resource_id,
            &self.user_home,
        )
    }

    pub fn list_storage_repositories(
        &self,
        storage_id: &StorageId,
    ) -> Result<Vec<CatalogStorageRepositoryDto>, String> {
        list_catalog_storage_repositories_with_repository(&self.state, storage_id)
    }

    pub fn link_scope_storage(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<CatalogScopeLinkOutcomeDto, String> {
        link_catalog_scope_storage_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn unlink_scope_storage(
        &self,
        link_id: &CatalogLinkId,
        expected_revision: u64,
    ) -> Result<bool, String> {
        unlink_catalog_scope_storage_with_repositories(
            &self.catalog,
            link_id,
            expected_revision,
            &self.user_home,
        )
    }

    pub fn prepare_push(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<CatalogScopePushPlan, String> {
        prepare_push_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_push(
        &self,
        approval: CatalogPushApproval,
    ) -> Result<CatalogScopePushOutcomeDto, String> {
        apply_push_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            approval.plan_id,
            approval.selected_resource_ids,
            approval.acknowledged_warning_digests,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_pull(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        requested_copy_names: BTreeMap<CatalogResourceId, String>,
    ) -> Result<CatalogScopePullPlan, String> {
        prepare_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            storage_id,
            requested_copy_names,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_pull(
        &self,
        approval: CatalogPullApproval,
    ) -> Result<CatalogScopePullOutcomeDto, String> {
        apply_pull_with_repositories(
            &self.state,
            &self.catalog,
            self.runner.as_ref(),
            approval.plan_id,
            approval.choices,
            approval.destructive_acknowledged,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn prepare_fork(
        &self,
        scope_id: &CatalogScopeId,
        resource_id: &CatalogResourceId,
        requested_name: &str,
    ) -> Result<CatalogResourceForkPlan, String> {
        prepare_fork_with_repositories(
            &self.catalog,
            self.runner.as_ref(),
            scope_id,
            resource_id,
            requested_name,
            &self.user_home,
            now_secs(),
        )
    }

    pub fn apply_fork(
        &self,
        scope_id: &CatalogScopeId,
        plan_id: &CatalogPlanId,
    ) -> Result<CatalogActionResult, String> {
        let CatalogResourcePlanDocument::Fork(plan) =
            plan_store::load(self.catalog.root(), plan_id)?
        else {
            return Err("the referenced plan is not a Fork review".to_string());
        };
        if &plan.scope_id != scope_id {
            return Err("the Fork review belongs to another row".to_string());
        }
        let ResolvedScope { scope, .. } = resolve_scope(&self.catalog, scope_id)?;
        let context = resolve_profile_context(
            &self.catalog.load_profiles()?,
            &scope.profile_id,
            scope.provider,
            &self.user_home,
        )
        .map_err(|blocker| blocker.to_string())?;
        context.verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;
        let result = apply_resource_fork(&self.catalog, &plan, now_secs())?;
        plan_store::discard(self.catalog.root(), plan_id);
        Ok(result)
    }
}

struct ResolvedScope {
    scope: CatalogScope,
    profile: LocalCatalogProfile,
}

fn resolve_scope(
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
) -> Result<ResolvedScope, String> {
    let scopes = catalog.load_scopes()?;
    let scope = scopes
        .scope(scope_id)
        .ok_or_else(|| format!("unknown catalog scope '{}'", scope_id))?
        .clone();
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(&scope.profile_id)
        .ok_or("this Catalog row lost its provider profile")?
        .clone();
    if profile.provider_key != scope.provider {
        return Err("this Catalog row's profile changed its provider".to_string());
    }
    Ok(ResolvedScope { scope, profile })
}

fn list_catalog_resource_profiles_with_repository(
    catalog: &CatalogStateRepository,
    user_home: &Path,
    now: u64,
) -> Result<Vec<LocalCatalogProfile>, String> {
    let candidates = capability_matrix()
        .into_iter()
        .map(|(provider, _)| provider)
        .filter_map(|provider| {
            let home = default_profile_home(provider, user_home)?;
            home.is_dir().then_some((provider, home))
        })
        .map(|(provider, home)| {
            let canonical = validate_new_profile_home(catalog.root(), &home.to_string_lossy())?;
            Ok((provider, canonical))
        })
        .collect::<Result<Vec<_>, String>>()?;

    if !candidates.is_empty() {
        let current = catalog.load_profiles()?;
        let has_missing = candidates.iter().any(|(provider, canonical)| {
            !current.profiles.iter().any(|profile| {
                profile.provider_key == *provider
                    && Path::new(&profile.configured_home) == canonical
            })
        });
        if has_missing {
            catalog.mutate_profiles(|document| {
                for (provider, canonical) in candidates {
                    if document.profiles.iter().any(|profile| {
                        profile.provider_key == provider
                            && Path::new(&profile.configured_home) == canonical
                    }) {
                        continue;
                    }
                    document.profiles.push(LocalCatalogProfile {
                        profile_id: LocalCatalogProfileId::generate()?,
                        provider_key: provider,
                        display_name: format!("Default {}", provider.display_label()),
                        configured_home: canonical.to_string_lossy().to_string(),
                        revision: 1,
                        created_at: now,
                        updated_at: now,
                    });
                }
                Ok(())
            })?;
        }
    }

    let mut profiles = catalog.load_profiles()?.profiles;
    profiles.sort_by(|left, right| {
        left.provider_key
            .cmp(&right.provider_key)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    Ok(profiles)
}

/// The name a profile home gets without anyone typing one: its folder name,
/// de-duplicated against the names already taken. Setup never asks for a
/// name — renaming afterwards is `rename_catalog_resource_profile`.
fn derived_profile_name(
    provider: ProviderKey,
    canonical: &Path,
    taken: &[LocalCatalogProfile],
) -> String {
    let base = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} profile", provider.display_label()));
    let is_taken = |candidate: &str| {
        taken
            .iter()
            .any(|profile| profile.display_name == candidate)
    };
    if !is_taken(&base) {
        return base;
    }
    (2..)
        .map(|suffix| format!("{} {}", base, suffix))
        .find(|candidate| !is_taken(candidate))
        .expect("some numeric suffix is always free")
}

fn save_catalog_resource_profile_with_repository(
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    path: String,
    now: u64,
) -> Result<LocalCatalogProfile, String> {
    let canonical = validate_new_profile_home(catalog.root(), &path)?;
    let canonical_home = canonical.to_string_lossy().to_string();
    catalog.mutate_profiles(|document| {
        // Re-picking a home that is already a profile selects it rather than
        // failing the submission.
        if let Some(existing) = document.profiles.iter().find(|profile| {
            profile.provider_key == provider && profile.configured_home == canonical_home
        }) {
            return Ok(existing.clone());
        }
        let profile = LocalCatalogProfile {
            profile_id: LocalCatalogProfileId::generate()?,
            provider_key: provider,
            display_name: derived_profile_name(provider, &canonical, &document.profiles),
            configured_home: canonical_home.clone(),
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        profile.validate()?;
        document.profiles.push(profile.clone());
        Ok(profile)
    })
}

fn rename_catalog_resource_profile_with_repository(
    catalog: &CatalogStateRepository,
    profile_id: &LocalCatalogProfileId,
    display_name: String,
    expected_revision: u64,
    now: u64,
) -> Result<LocalCatalogProfile, String> {
    let display_name = display_name.trim().to_string();
    if display_name.is_empty() {
        return Err("a profile name cannot be empty".to_string());
    }
    catalog.mutate_profiles(|document| {
        let profile = document
            .profiles
            .iter_mut()
            .find(|profile| &profile.profile_id == profile_id)
            .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?;
        if profile.revision != expected_revision {
            return Err("the profile changed while renaming".to_string());
        }
        profile.display_name = display_name.clone();
        profile.revision = profile.revision.saturating_add(1);
        profile.updated_at = now;
        let profile = profile.clone();
        profile.validate()?;
        Ok(profile)
    })
}

/// Resolve one Library profile's still-existing home for a launch. The
/// stored path was canonicalized at save time, but the folder can vanish
/// afterwards — a launch against a missing home fails closed here instead of
/// inside the spawned app.
fn profile_launch_home(
    catalog: &CatalogStateRepository,
    profile_id: &LocalCatalogProfileId,
) -> Result<(ProviderKey, PathBuf), String> {
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(profile_id)
        .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?;
    let home = PathBuf::from(&profile.configured_home);
    if !home.is_absolute() || !home.is_dir() {
        return Err("this profile's home folder is missing; pick it again in Library".to_string());
    }
    Ok((profile.provider_key, home))
}

fn create_catalog_scope_with_repositories(
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    profile_id: LocalCatalogProfileId,
    resource_kind: CatalogResourceKindTag,
    now: u64,
) -> Result<CatalogScopeDto, String> {
    if !provider_owns_kind(provider, resource_kind) {
        return Err("this provider cannot host that resource kind".to_string());
    }
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(&profile_id)
        .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?
        .clone();
    if profile.provider_key != provider {
        return Err(format!(
            "catalog profile '{}' is a {} profile, not {}",
            profile_id,
            profile.provider_key.as_str(),
            provider.as_str()
        ));
    }
    let scope = catalog.mutate_scopes(|document| {
        if document
            .scopes
            .iter()
            .any(|scope| scope.profile_id == profile_id && scope.resource_kind == resource_kind)
        {
            return Err("this Catalog row already exists".to_string());
        }
        let scope = CatalogScope {
            scope_id: CatalogScopeId::generate()?,
            provider,
            profile_id: profile_id.clone(),
            resource_kind,
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        scope.validate()?;
        document.scopes.push(scope.clone());
        Ok(scope)
    })?;
    Ok(CatalogScopeDto { scope, profile })
}

fn delete_catalog_scope_with_repositories(
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    expected_revision: u64,
) -> Result<bool, String> {
    let scopes = catalog.load_scopes()?;
    if scopes.scope(scope_id).is_none() {
        return Ok(false);
    };
    if catalog
        .load_scope_links()?
        .links_for(scope_id)
        .next()
        .is_some()
    {
        return Err("this row still links storages; unlink them first".to_string());
    }
    catalog.mutate_scopes(|document| {
        let Some(index) = document
            .scopes
            .iter()
            .position(|scope| &scope.scope_id == scope_id)
        else {
            return Ok(false);
        };
        if document.scopes[index].revision != expected_revision {
            return Err("this Catalog row changed; refresh first".to_string());
        }
        document.scopes.remove(index);
        Ok(true)
    })
}

/// The storage a scope page compares against: the caller's choice, else the
/// storage this scope's newest link points at, else the only one configured.
/// Opening on a storage the row has no history with would report every
/// resource as unpushed.
fn scope_storage(
    scope_id: &CatalogScopeId,
    links: &super::domain::CatalogScopeLinksDocument,
    config: &SyncConfig,
    requested: Option<&StorageId>,
) -> Option<StorageId> {
    if let Some(requested) = requested {
        return Some(requested.clone());
    }
    links
        .links_for(scope_id)
        .filter(|link| {
            config
                .storages
                .iter()
                .any(|storage| storage.id == link.storage_id)
        })
        .max_by_key(|link| (link.updated_at, link.link_id.clone()))
        .map(|link| link.storage_id.clone())
        .or_else(|| config.storages.first().map(|storage| storage.id.clone()))
}

/// Everything a scope page or review needs, resolved once: the scope, its
/// profile context and kind inventory, and — when a storage participates —
/// the storage config, the scope link, and the verified repository view.
struct ResolvedScopeWorkspace {
    scope: CatalogScope,
    profile: LocalCatalogProfile,
    context: ResolvedCatalogContext,
    inventory: CatalogLocalInventory,
    storage: Option<StorageConfig>,
    link: Option<CatalogScopeLink>,
    observed: Option<ObservedLibraryHead>,
    manifest: Option<LibraryResourceManifest>,
}

impl ResolvedScopeWorkspace {
    fn remote_map(&self) -> BTreeMap<CatalogResourceId, CatalogResource> {
        self.manifest
            .as_ref()
            .map(LibraryResourceManifest::resource_map)
            .unwrap_or_default()
    }
}

fn resolve_scope_workspace(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    requested_storage: Option<&StorageId>,
    user_home: &Path,
) -> Result<ResolvedScopeWorkspace, String> {
    let ResolvedScope { scope, profile } = resolve_scope(catalog, scope_id)?;
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &scope.profile_id,
        scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(&context, scope.resource_kind, runner)
        .map_err(|blocker| blocker.to_string())?;

    let links = catalog.load_scope_links()?;
    let config = state.load_config()?;
    let storage_id = scope_storage(scope_id, &links, &config, requested_storage);
    let storage = match &storage_id {
        Some(storage_id) => Some(storage_for(state, storage_id)?),
        None => None,
    };
    let link = storage
        .as_ref()
        .and_then(|storage| links.link_for(scope_id, &storage.id))
        .cloned();

    let repository_id = CatalogResourceRepositoryId::for_kind(scope.provider, scope.resource_kind);
    if let Some(link) = &link {
        if link.repository_id != repository_id {
            return Err(
                "this scope link points at a repository its kind no longer derives".to_string(),
            );
        }
    }
    let mut observed = None;
    let mut manifest = None;
    if let Some(storage) = &storage {
        let engine = engine_for(storage)?;
        observed = engine.read_head(&repository_id)?;
        verify_library_continuity(
            link.as_ref().and_then(|link| link.accepted_head.as_ref()),
            observed.as_ref().map(|observed| &observed.head),
            &repository_id,
            &storage_identity_for_config(storage),
        )?;
        if let Some(observed) = &observed {
            // The tag freezes provider+kind; read_tag validates the
            // derivation, so a repository written under a foreign identity
            // fails the page closed here.
            let (tag, _) = engine
                .read_tag(&repository_id)?
                .ok_or("the stored kind repository is missing its discovery tag")?;
            if tag.provider != scope.provider || tag.resource_kind != scope.resource_kind {
                return Err("the stored kind repository belongs to another kind".to_string());
            }
            manifest = Some(engine.read_manifest(&observed.head)?);
        }
    }
    Ok(ResolvedScopeWorkspace {
        scope,
        profile,
        context,
        inventory,
        storage,
        link,
        observed,
        manifest,
    })
}

fn require_link(workspace: &ResolvedScopeWorkspace) -> Result<&CatalogScopeLink, String> {
    workspace
        .link
        .as_ref()
        .ok_or_else(|| "link this storage first".to_string())
}

fn list_catalog_scope_resources_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    storage_id: Option<&StorageId>,
    user_home: &Path,
) -> Result<CatalogScopeResourcesDto, String> {
    let workspace =
        resolve_scope_workspace(state, catalog, runner, scope_id, storage_id, user_home)?;
    let remote = workspace.remote_map();
    let remote_updated: BTreeMap<&CatalogResourceId, u64> = workspace
        .manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .resources
                .iter()
                .map(|(resource_id, entry)| (resource_id, entry.updated_at))
                .collect()
        })
        .unwrap_or_default();

    let locals: BTreeMap<&CatalogResourceId, &LocalResourceRecord> =
        scope_local_records(&workspace.inventory, workspace.scope.resource_kind).collect();
    let mut resource_ids: BTreeSet<&CatalogResourceId> = locals.keys().copied().collect();
    resource_ids.extend(remote.keys());

    let mut rows: Vec<CatalogScopeRowDto> = Vec::new();
    for resource_id in resource_ids {
        let local = locals.get(resource_id).copied();
        let remote_resource = remote.get(resource_id);
        let local_sha = local.map(|record| record.resource.content_sha256());
        let remote_sha = remote_resource.map(|resource| resource.content_sha256());
        let state = if workspace.storage.is_none() {
            None
        } else {
            Some(match &workspace.link {
                Some(link) => scope_resource_state(link, resource_id, local_sha, remote_sha),
                // With no link there is no base, so a resource that exists on
                // both sides is "never reviewed" rather than diverged — the
                // link is what establishes bases.
                None => match (local_sha, remote_sha) {
                    (Some(_), None) => CatalogLinkState::New,
                    (None, _) => CatalogLinkState::LocalMissing,
                    (Some(local), Some(remote)) if local == remote => CatalogLinkState::InSync,
                    (Some(_), Some(_)) => CatalogLinkState::UnreviewedRemote,
                },
            })
        };
        let presented = match local.map(|record| &record.resource).or(remote_resource) {
            Some(resource) => resource,
            None => continue,
        };
        let mut row = CatalogScopeRowDto {
            row_id: format!("resource:{}", resource_id),
            display_name: presented.display_name(),
            resource_id: Some((*resource_id).clone()),
            scope_label: scope_label(presented),
            state,
            local_present: local.is_some(),
            local_path: local
                .and_then(|record| record.target_path.as_ref())
                .map(|path| path.to_string_lossy().to_string()),
            local_modified_at: local.and_then(|record| record.local_modified_at),
            local_files: local.map(|record| record.resource.file_count() as u64),
            local_bytes: local.map(|record| record.resource.payload_bytes()),
            remote_present: remote_resource.is_some(),
            remote_updated_at: remote_updated.get(resource_id).copied(),
            content_sha256: local_sha.or(remote_sha).map(str::to_string),
            preview_png: None,
            pet_description: None,
            warnings: local
                .map(|record| record.warnings.clone())
                .unwrap_or_default(),
            blocker: None,
        };
        match (local, remote_resource) {
            (Some(record), _) => decorate_pet_row(
                &mut row,
                &record.resource,
                record.target_path.as_deref(),
                user_home,
            ),
            (None, Some(resource)) => decorate_pet_row(&mut row, resource, None, user_home),
            (None, None) => {}
        }
        rows.push(row);
    }

    // Present on disk and impossible to capture — a symlinked skill
    // directory, an unreadable pet package. Hiding them would make the page
    // disagree with the folder it claims to describe.
    for blocked in &workspace.inventory.blocked {
        rows.push(CatalogScopeRowDto {
            row_id: format!("blocked:{}", blocked.display_name),
            display_name: blocked.display_name.clone(),
            resource_id: None,
            scope_label: blocked.scope_label.clone(),
            state: Some(CatalogLinkState::Blocked),
            local_present: true,
            local_path: None,
            local_modified_at: None,
            local_files: None,
            local_bytes: None,
            remote_present: false,
            remote_updated_at: None,
            content_sha256: None,
            preview_png: None,
            pet_description: None,
            warnings: Vec::new(),
            blocker: Some(blocked.reason.clone()),
        });
    }

    // Name-ascending, the order every comparable list ships with. Sorting by
    // state would move a row out from under the cursor the moment it synced.
    rows.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.row_id.cmp(&right.row_id))
    });

    let recovery = recover_catalog_operations(catalog, now_secs())?;
    Ok(CatalogScopeResourcesDto {
        scope: workspace.scope,
        profile: workspace.profile,
        storage_id: workspace.storage.as_ref().map(|storage| storage.id.clone()),
        storage_name: workspace
            .storage
            .as_ref()
            .map(|storage| storage.name.clone()),
        link: workspace.link.as_ref().map(|link| CatalogScopePageLinkDto {
            link_id: link.link_id.clone(),
            revision: link.revision,
        }),
        storage_revision: workspace
            .observed
            .as_ref()
            .map(|observed| observed.head.revision),
        rows,
        warnings: workspace.inventory.warnings,
        recovery,
    })
}

/// Pet rows carry two things the scan already holds: the descriptor's own
/// sentence about the pet, and — when the atlas is on this Mac — a small
/// preview of its idle frame, derived once per digest and cached. Previews
/// are decoration, so a failed derivation leaves the row without one rather
/// than failing the page; the card draws the kind glyph instead. Stored-only
/// rows are left to `get_catalog_pet_preview`, fetched lazily so no storage
/// round-trip ever sits in the list's own path.
fn decorate_pet_row(
    row: &mut CatalogScopeRowDto,
    resource: &CatalogResource,
    target_path: Option<&Path>,
    user_home: &Path,
) {
    let (format, pet) = match resource {
        CatalogResource::CodexPetV1(pet) => (CodexPetFormat::V1, pet),
        CatalogResource::CodexPetV2(pet) => (CodexPetFormat::V2, pet),
        _ => return,
    };
    row.pet_description = Some(pet.descriptor.description.clone());
    let Some(directory) = target_path else { return };
    let atlas_path = directory.join(&pet.descriptor.atlas_basename);
    row.preview_png = super::pet_preview::cached_preview_data_uri(
        &super::pet_preview::preview_cache_dir(user_home),
        format,
        &pet.descriptor.atlas.sha256,
        || {
            // Capture bounded this file already, but it can change between
            // scan and list — never trust a path twice.
            let metadata =
                fs::metadata(&atlas_path).map_err(|error| format!("read pet atlas: {}", error))?;
            if metadata.len() > MAX_PET_ATLAS_BYTES {
                return Err("pet atlas exceeds its encoded size limit".to_string());
            }
            fs::read(&atlas_path).map_err(|error| format!("read pet atlas: {}", error))
        },
    )
    .ok();
}

/// The preview for a stored-only pet row, fetched from the storage on demand
/// and cached by the atlas digest. Kept out of the list call on purpose: a
/// page must never wait on one storage round-trip per row, and this Mac may
/// be offline — the card asks lazily and keeps its glyph when this fails.
fn get_catalog_pet_preview_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    resource_id: &CatalogResourceId,
    user_home: &Path,
) -> Result<String, String> {
    let ResolvedScope { scope, .. } = resolve_scope(catalog, scope_id)?;
    let format = match scope.resource_kind {
        CatalogResourceKindTag::CodexPetV1 => CodexPetFormat::V1,
        CatalogResourceKindTag::CodexPetV2 => CodexPetFormat::V2,
        _ => return Err("this Catalog row's resources have no preview".to_string()),
    };
    let storage = storage_for(state, storage_id)?;
    let engine = engine_for(&storage)?;
    let repository_id = CatalogResourceRepositoryId::for_kind(scope.provider, scope.resource_kind);
    let (_, manifest) = engine
        .inspect(&repository_id)?
        .ok_or("the stored resource disappeared")?;
    let resource = manifest
        .resources
        .get(resource_id)
        .map(|entry| &entry.resource)
        .ok_or("the stored resource disappeared")?;
    let pet = match (resource, format) {
        (CatalogResource::CodexPetV1(pet), CodexPetFormat::V1)
        | (CatalogResource::CodexPetV2(pet), CodexPetFormat::V2) => pet,
        _ => return Err("the stored resource is not a pet of this row's kind".to_string()),
    };
    let atlas = pet.descriptor.atlas.clone();
    atlas.validate("stored pet atlas", MAX_PET_ATLAS_BYTES)?;
    super::pet_preview::cached_preview_data_uri(
        &super::pet_preview::preview_cache_dir(user_home),
        format,
        &atlas.sha256,
        || engine.fetch_payload(&repository_id, &atlas.sha256, atlas.size),
    )
}

/// One Catalog kind repository as the storage page lists it: the derived
/// identity, the head facts when the repository reads, and the refusal when
/// it does not. Absent repositories produce no entry at all.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogStorageRepositoryDto {
    pub repository_id: String,
    pub provider: ProviderKey,
    pub resource_kind: CatalogResourceKindTag,
    pub revision: Option<u64>,
    pub updated_at: Option<u64>,
    /// Display names of everything the set holds, sorted for the fold.
    pub resource_names: Vec<String>,
    /// Why the repository cannot be read; the row states it instead of chips.
    pub error: Option<String>,
}

/// The storage page's Catalog listing: every kind repository this build can
/// derive — the (provider, kind) ownership matrix — inspected with one head
/// read each. Nothing is scanned: a repository is either derivable and read,
/// or invisible to this build by construction. A derived id that answers but
/// fails its tag or manifest checks is returned as a refusal row, never
/// omitted — on a listing, a silent hole reads as "nothing stored".
fn list_catalog_storage_repositories_with_repository(
    state: &LocalStateRepository,
    storage_id: &StorageId,
) -> Result<Vec<CatalogStorageRepositoryDto>, String> {
    let storage = storage_for(state, storage_id)?;
    let engine = engine_for(&storage)?;
    let mut rows = Vec::new();
    for kind in [
        CatalogResourceKindTag::AgentSkill,
        CatalogResourceKindTag::PluginInstallIntent,
        CatalogResourceKindTag::CodexPetV1,
        CatalogResourceKindTag::CodexPetV2,
    ] {
        for provider in [ProviderKey::Codex, ProviderKey::Claude] {
            if !provider_owns_kind(provider, kind) {
                continue;
            }
            if let Some(row) = read_storage_repository(&engine, provider, kind) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// `None` when the repository does not exist in this storage; a refusal row
/// when something exists under the derived id but cannot be read.
fn read_storage_repository(
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    provider: ProviderKey,
    kind: CatalogResourceKindTag,
) -> Option<CatalogStorageRepositoryDto> {
    let repository_id = CatalogResourceRepositoryId::for_kind(provider, kind);
    let refusal = |message: String| CatalogStorageRepositoryDto {
        repository_id: repository_id.as_str().to_string(),
        provider,
        resource_kind: kind,
        revision: None,
        updated_at: None,
        resource_names: Vec::new(),
        error: Some(message),
    };
    let observed = match engine.read_head(&repository_id) {
        Ok(observed) => observed,
        Err(message) => return Some(refusal(message)),
    };
    let tag = match engine.read_tag(&repository_id) {
        Ok(tag) => tag,
        Err(message) => return Some(refusal(message)),
    };
    let Some(observed) = observed else {
        return tag.map(|_| {
            // A tag with no head is what a failed first publish leaves
            // behind; say so rather than passing it off as absent.
            refusal("the stored kind repository has a discovery tag but no head".to_string())
        });
    };
    let Some((tag, _)) = tag else {
        return Some(refusal(
            "the stored kind repository is missing its discovery tag".to_string(),
        ));
    };
    // read_tag already enforced the derivation; this is the same
    // belt-and-braces identity check the scope workspace keeps.
    if tag.provider != provider || tag.resource_kind != kind {
        return Some(refusal(
            "the stored kind repository belongs to another kind".to_string(),
        ));
    }
    let manifest = match engine.read_manifest(&observed.head) {
        Ok(manifest) => manifest,
        Err(message) => return Some(refusal(message)),
    };
    let mut resource_names: Vec<String> = manifest
        .resources
        .values()
        .map(|entry| entry.resource.display_name())
        .collect();
    resource_names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    Some(CatalogStorageRepositoryDto {
        repository_id: repository_id.as_str().to_string(),
        provider,
        resource_kind: kind,
        revision: Some(observed.head.revision),
        updated_at: Some(manifest.updated_at),
        resource_names,
        error: None,
    })
}

fn link_catalog_scope_storage_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopeLinkOutcomeDto, String> {
    let workspace = resolve_scope_workspace(
        state,
        catalog,
        runner,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    if let Some(existing) = &workspace.link {
        return Ok(CatalogScopeLinkOutcomeDto {
            link_id: existing.link_id.clone(),
            revision: existing.revision,
            completion: "open".to_string(),
        });
    }
    let storage = workspace
        .storage
        .as_ref()
        .ok_or("no storage is configured to link")?;
    let repository_id = CatalogResourceRepositoryId::for_kind(
        workspace.scope.provider,
        workspace.scope.resource_kind,
    );

    // Pin per-resource bases where the local capture already equals the
    // stored copy; accept the head only when every stored resource agreed.
    // Anything else stays unreviewed and resolves through Pull, never a
    // silent overwrite.
    let remote = workspace.remote_map();
    let mut accepted_resources = BTreeMap::new();
    for (resource_id, remote_resource) in &remote {
        let matches = workspace
            .inventory
            .resources
            .get(resource_id)
            .is_some_and(|record| {
                record.resource.content_sha256() == remote_resource.content_sha256()
            });
        if matches {
            accepted_resources.insert(
                resource_id.clone(),
                remote_resource.content_sha256().to_string(),
            );
        }
    }
    let full_agreement = accepted_resources.len() == remote.len();
    let accepted_head = match (&workspace.observed, full_agreement) {
        (Some(observed), true) => Some(head_coordinate(
            &observed.head,
            &storage_identity_for_config(storage),
        )),
        _ => None,
    };
    let completion = if full_agreement {
        "open"
    } else {
        "review_pull"
    };

    let scope_id = scope_id.clone();
    let storage_id = storage.id.clone();
    let link = catalog.mutate_scope_links(|document| {
        if let Some(existing) = document.link_for(&scope_id, &storage_id) {
            return Ok(existing.clone());
        }
        let link = CatalogScopeLink {
            link_id: CatalogLinkId::generate()?,
            scope_id: scope_id.clone(),
            storage_id: storage_id.clone(),
            repository_id: repository_id.clone(),
            accepted_head: accepted_head.clone(),
            accepted_resources: accepted_resources.clone(),
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        link.validate()?;
        document.links.push(link.clone());
        Ok(link)
    })?;
    Ok(CatalogScopeLinkOutcomeDto {
        link_id: link.link_id,
        revision: link.revision,
        completion: completion.to_string(),
    })
}

/// Whether a machine-local target belongs to this scope's kind under this
/// resolved context. Pets additionally check the format through the claimed
/// resource id, because the v1 and v2 formats share one directory.
fn target_in_scope(
    target: &LocalCatalogTargetId,
    kind: CatalogResourceKindTag,
    context: &ResolvedCatalogContext,
    resource_id: Option<&CatalogResourceId>,
) -> bool {
    match (target, kind) {
        (
            LocalCatalogTargetId::SkillDirectory {
                scope,
                canonical_root,
                ..
            },
            CatalogResourceKindTag::AgentSkill,
        ) => context
            .skill_root(*scope)
            .is_some_and(|root| root == Path::new(canonical_root)),
        (
            LocalCatalogTargetId::Plugin {
                provider,
                canonical_home,
                ..
            },
            CatalogResourceKindTag::PluginInstallIntent,
        ) => context
            .profile(*provider)
            .is_some_and(|profile| profile.canonical_home == Path::new(canonical_home)),
        (
            LocalCatalogTargetId::PetDirectory {
                canonical_home,
                pet_id,
            },
            CatalogResourceKindTag::CodexPetV1 | CatalogResourceKindTag::CodexPetV2,
        ) => {
            let home_matches = context
                .profile(ProviderKey::Codex)
                .is_some_and(|profile| profile.canonical_home == Path::new(canonical_home));
            let format = kind.pet_format().expect("pet kinds always carry a format");
            home_matches
                && resource_id
                    .is_none_or(|resource_id| resource_id == &pet_resource_id(format, pet_id))
        }
        _ => false,
    }
}

/// Push and Pull reviews supersede per pairing: opening a new review for a
/// (scope, storage) pair discards the pair's previous one.
fn discard_scope_plans(
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
) {
    plan_store::discard_where(catalog.root(), |plan| match plan {
        CatalogResourcePlanDocument::Push(plan) => {
            &plan.scope_id == scope_id && &plan.storage_id == storage_id
        }
        CatalogResourcePlanDocument::Pull(plan) => {
            &plan.scope_id == scope_id && &plan.storage_id == storage_id
        }
        CatalogResourcePlanDocument::Fork(_) => false,
    });
}

fn unlink_catalog_scope_storage_with_repositories(
    catalog: &CatalogStateRepository,
    link_id: &CatalogLinkId,
    expected_revision: u64,
    user_home: &Path,
) -> Result<bool, String> {
    let links = catalog.load_scope_links()?;
    let Some(link) = links.link(link_id).cloned() else {
        return Ok(false);
    };
    if link.revision != expected_revision {
        return Err("the catalog link changed; refresh before unlinking".to_string());
    }
    let ResolvedScope { scope, .. } = resolve_scope(catalog, &link.scope_id)?;
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &scope.profile_id,
        scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;

    let storage_id = link.storage_id.clone();
    let active_operation = catalog
        .load_operations()?
        .operations
        .into_iter()
        .any(|operation| {
            !operation.phase.is_terminal()
                && operation.storage_id.as_ref() == Some(&storage_id)
                && target_in_scope(&operation.target, scope.resource_kind, &context, None)
        });
    if active_operation {
        return Err("an operation for this link is still in progress".to_string());
    }
    discard_scope_plans(catalog, &link.scope_id, &storage_id);
    catalog.mutate_materializations(|claims| {
        claims.claims.retain(|claim| {
            claim.storage_id != storage_id
                || !target_in_scope(
                    &claim.target,
                    scope.resource_kind,
                    &context,
                    Some(&claim.resource_id),
                )
        });
        Ok(())
    })?;
    catalog.mutate_preservations(|preservations| {
        preservations.witnesses.retain(|witness| {
            witness.storage_id != storage_id
                || !target_in_scope(
                    &witness.source_target,
                    scope.resource_kind,
                    &context,
                    Some(&witness.source_resource_id),
                )
        });
        Ok(())
    })?;
    catalog.mutate_operations(|operations| {
        operations.operations.retain(|operation| {
            !(operation.phase.is_terminal()
                && operation.storage_id.as_ref() == Some(&storage_id)
                && target_in_scope(&operation.target, scope.resource_kind, &context, None))
        });
        Ok(())
    })?;
    catalog.mutate_scope_links(|document| {
        let before = document.links.len();
        document.links.retain(|link| &link.link_id != link_id);
        Ok(document.links.len() != before)
    })
}

fn prepare_push_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePushPlan, String> {
    let workspace = resolve_scope_workspace(
        state,
        catalog,
        runner,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let storage_identity = storage_identity_for_config(storage);
    let remote = workspace.remote_map();
    let plan = build_scope_push_plan(ScopePushPlanInputs {
        scope: &workspace.scope,
        link,
        storage_kind: storage.kind,
        storage_identity_sha256: storage_identity.clone(),
        context: &workspace.context,
        inventory: &workspace.inventory,
        remote: &remote,
        observed_head: workspace
            .observed
            .as_ref()
            .map(|observed| head_coordinate(&observed.head, &storage_identity)),
        now,
    })?;
    discard_scope_plans(catalog, scope_id, storage_id);
    plan_store::save(
        catalog.root(),
        CatalogResourcePlanDocument::Push(plan.clone()),
        now,
    )?;
    Ok(plan)
}

fn payloads_for_record(record: &LocalResourceRecord) -> BTreeMap<String, Bytes> {
    let mut payloads = BTreeMap::new();
    if let Some(tree) = &record.skill_tree {
        for file in tree.files.values() {
            payloads.insert(sha256_hex(&file.bytes), Bytes::from(file.bytes.clone()));
        }
    }
    if let Some(pet) = &record.pet {
        payloads.insert(
            sha256_hex(&pet.atlas_bytes),
            Bytes::from(pet.atlas_bytes.clone()),
        );
    }
    payloads
}

fn observed_matches(
    pinned: Option<&super::domain::LibraryHeadCoordinate>,
    observed: Option<&ObservedLibraryHead>,
    storage_identity: &str,
) -> bool {
    match (pinned, observed) {
        (None, None) => true,
        (Some(pinned), Some(observed)) => {
            &head_coordinate(&observed.head, storage_identity) == pinned
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_push_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    plan_id: CatalogPlanId,
    selected_resource_ids: Vec<CatalogResourceId>,
    acknowledged_warning_digests: Vec<String>,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePushOutcomeDto, String> {
    let CatalogResourcePlanDocument::Push(plan) = plan_store::load(catalog.root(), &plan_id)?
    else {
        return Err("the referenced plan is not a Push review".to_string());
    };
    if now < plan.created_at || now > plan.expires_at {
        return Err("the Push review expired; review again".to_string());
    }
    if selected_resource_ids.is_empty() {
        plan_store::discard(catalog.root(), &plan_id);
        return Ok(CatalogScopePushOutcomeDto {
            plan_id,
            published: false,
            revision: plan.observed_head.map(|head| head.revision),
            message: "Nothing was pushed".to_string(),
        });
    }

    let workspace = resolve_scope_workspace(
        state,
        catalog,
        runner,
        &plan.scope_id,
        Some(&plan.storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    if link.link_id != plan.link_id || link.revision != plan.link_revision {
        return Err("the scope link changed after review".to_string());
    }
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let storage_identity = storage_identity_for_config(storage);
    if storage_identity != plan.storage_identity_sha256
        || !observed_matches(
            plan.observed_head.as_ref(),
            workspace.observed.as_ref(),
            &storage_identity,
        )
    {
        return Err("the storage head changed after review".to_string());
    }
    workspace
        .context
        .verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;

    let acknowledged: BTreeSet<_> = acknowledged_warning_digests.into_iter().collect();
    let selected: BTreeSet<CatalogResourceId> = selected_resource_ids.into_iter().collect();
    let mut selected_records: Vec<(&CatalogResourceId, &LocalResourceRecord)> = Vec::new();
    for resource_id in &selected {
        let row = plan
            .push_row(resource_id)
            .ok_or("a selected resource is not part of this Push review")?;
        if !row.row.selectable {
            return Err(row
                .row
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "this resource has nothing safe to Push".to_string()));
        }
        for warning in &row.row.warnings {
            if !acknowledged.contains(&warning.digest) {
                return Err(format!("unacknowledged warning: {}", warning.message));
            }
        }
        let record = workspace
            .inventory
            .resources
            .get(resource_id)
            .filter(|record| record.resource.kind_tag() == workspace.scope.resource_kind)
            .ok_or("a selected resource disappeared after review")?;
        let reviewed = row
            .resource
            .as_ref()
            .expect("selectable rows always carry their resource");
        if &record.resource != reviewed {
            return Err("a selected resource changed after review".to_string());
        }
        if record.warnings != row.row.warnings {
            return Err("a selected resource's warnings changed after review".to_string());
        }
        selected_records.push((resource_id, record));
    }

    // The new set is the observed set plus the reviewed upserts — never a
    // replace, so a push can never delete another replica's resources.
    let mut resources = workspace
        .manifest
        .as_ref()
        .map(|manifest| manifest.resources.clone())
        .unwrap_or_default();
    let mut payloads = BTreeMap::new();
    for (resource_id, record) in &selected_records {
        let unchanged = resources.get(*resource_id).is_some_and(|entry| {
            entry.resource.content_sha256() == record.resource.content_sha256()
        });
        if !unchanged {
            resources.insert(
                (*resource_id).clone(),
                PublishedResourceEntry {
                    resource: record.resource.clone(),
                    updated_at: now,
                },
            );
        }
        payloads.extend(payloads_for_record(record));
    }

    let engine = engine_for(storage)?;
    let repository_id = link.repository_id.clone();
    let tag = match engine.read_tag(&repository_id)? {
        Some((tag, _)) => tag,
        None => LibraryResourceTag {
            schema_version: CATALOG_SCHEMA,
            repository_id: repository_id.clone(),
            provider: workspace.scope.provider,
            resource_kind: workspace.scope.resource_kind,
            created_at: link.created_at,
        },
    };
    let host = state.load_or_create_host_identity()?;
    let published = engine.publish(PublishLibraryResourceRequest {
        repository_id: repository_id.clone(),
        tag,
        observed: workspace.observed.clone(),
        resources,
        payloads,
        captured_with: CatalogCapturedWith {
            app: "mallard".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        published_by_origin: super::domain::derive_library_origin(
            host.host_id.as_str(),
            link.link_id.as_str(),
            &repository_id,
        )?,
        updated_at: now,
    })?;

    let coordinate = head_coordinate(&published.head, &storage_identity);
    // Accepted bases come from the manifest that actually landed, which on
    // the no-op path is the previous manifest already holding this content.
    let accepted: Vec<(CatalogResourceId, String)> = selected_records
        .iter()
        .map(|(resource_id, _)| {
            let entry = published
                .manifest
                .resources
                .get(*resource_id)
                .ok_or("the published manifest lost a selected resource")?;
            Ok((
                (*resource_id).clone(),
                entry.resource.content_sha256().to_string(),
            ))
        })
        .collect::<Result<_, String>>()?;
    let link_id = link.link_id.clone();
    let link_revision = plan.link_revision;
    catalog.mutate_scope_links(|document| {
        let link = document
            .links
            .iter_mut()
            .find(|link| link.link_id == link_id)
            .ok_or("the scope link disappeared while publishing")?;
        if link.revision != link_revision {
            return Err("the scope link changed while publishing".to_string());
        }
        link.accepted_head = Some(coordinate.clone());
        for (resource_id, content) in &accepted {
            link.accepted_resources
                .insert(resource_id.clone(), content.clone());
        }
        link.revision = link.revision.saturating_add(1);
        link.updated_at = now;
        Ok(())
    })?;
    plan_store::discard(catalog.root(), &plan_id);
    Ok(CatalogScopePushOutcomeDto {
        plan_id,
        published: published.published,
        revision: Some(published.head.revision),
        message: if published.published {
            format!("Published revision {}", published.head.revision)
        } else {
            "The linked storage already had this content".to_string()
        },
    })
}

fn name_taken_index<'a>(
    inventory: &'a CatalogLocalInventory,
) -> impl Fn(&LocalCatalogTargetId, &str) -> bool + 'a {
    move |target, candidate| {
        let folded = candidate.to_lowercase();
        let inventoried =
            inventory
                .resources
                .values()
                .any(|record| match (&record.target, target) {
                    (
                        LocalCatalogTargetId::SkillDirectory {
                            canonical_root: left_root,
                            install_directory,
                            ..
                        },
                        LocalCatalogTargetId::SkillDirectory {
                            canonical_root: right_root,
                            ..
                        },
                    ) => left_root == right_root && install_directory.to_lowercase() == folded,
                    (
                        LocalCatalogTargetId::PetDirectory {
                            canonical_home: left_home,
                            pet_id,
                        },
                        LocalCatalogTargetId::PetDirectory {
                            canonical_home: right_home,
                            ..
                        },
                    ) => left_home == right_home && pet_id.to_lowercase() == folded,
                    _ => false,
                });
        if inventoried {
            return true;
        }
        let parent = match target {
            LocalCatalogTargetId::SkillDirectory { canonical_root, .. } => {
                Path::new(canonical_root).to_path_buf()
            }
            LocalCatalogTargetId::PetDirectory { canonical_home, .. } => {
                Path::new(canonical_home).join("pets")
            }
            LocalCatalogTargetId::Plugin { .. } => return true,
        };
        match fs::read_dir(parent) {
            Ok(entries) => entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_lowercase() == folded)
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_pull_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    requested_copy_names: BTreeMap<CatalogResourceId, String>,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePullPlan, String> {
    let workspace = resolve_scope_workspace(
        state,
        catalog,
        runner,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let observed = workspace
        .observed
        .as_ref()
        .ok_or("the storage has nothing of this kind to Pull")?;
    let storage_identity = storage_identity_for_config(storage);
    let remote = workspace.remote_map();
    let claims = catalog.load_materializations()?.claims;
    let name_taken = name_taken_index(&workspace.inventory);
    let plan = build_scope_pull_plan(ScopePullPlanInputs {
        scope: &workspace.scope,
        link,
        storage_identity_sha256: storage_identity.clone(),
        context: &workspace.context,
        inventory: &workspace.inventory,
        remote: &remote,
        head: head_coordinate(&observed.head, &storage_identity),
        claims: &claims,
        requested_copy_names: &requested_copy_names,
        name_taken: &name_taken,
        now,
    })?;
    discard_scope_plans(catalog, scope_id, storage_id);
    plan_store::save(
        catalog.root(),
        CatalogResourcePlanDocument::Pull(plan.clone()),
        now,
    )?;
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn apply_pull_with_repositories(
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    plan_id: CatalogPlanId,
    choices: BTreeMap<CatalogResourceId, CatalogPlanActionId>,
    destructive_acknowledged: Vec<CatalogResourceId>,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePullOutcomeDto, String> {
    let CatalogResourcePlanDocument::Pull(plan) = plan_store::load(catalog.root(), &plan_id)?
    else {
        return Err("the referenced plan is not a Pull review".to_string());
    };
    if now < plan.created_at || now > plan.expires_at {
        return Err("the Pull review expired; review again".to_string());
    }
    let workspace = resolve_scope_workspace(
        state,
        catalog,
        runner,
        &plan.scope_id,
        Some(&plan.storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    if link.link_id != plan.link_id || link.revision != plan.link_revision {
        return Err("the scope link changed after review".to_string());
    }
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    if storage_identity_for_config(storage) != plan.storage_identity_sha256 {
        return Err("the storage changed after review".to_string());
    }
    workspace
        .context
        .verify_pins(&plan.profile_pins, &plan.skill_root_pins)?;
    let observed = workspace
        .observed
        .as_ref()
        .ok_or("the stored repository head disappeared")?;
    if head_coordinate(&observed.head, &plan.storage_identity_sha256) != plan.head {
        return Err("the stored content changed after review".to_string());
    }

    let acknowledged: BTreeSet<CatalogResourceId> = destructive_acknowledged.into_iter().collect();
    let selections = validate_scope_pull_selection(&plan, &choices, &acknowledged)?;
    let claims = catalog.load_materializations()?.claims;
    for selection in &selections {
        let Some(action) = selection.action else {
            continue;
        };
        let current_claims = conflicting_claim_pins(
            &claims,
            &workspace.inventory,
            &plan.storage_id,
            &action.target,
            &action.incoming_content_sha256,
        );
        if current_claims != selection.item.conflicting_claims {
            return Err(
                "cross-storage claims changed after review; refresh and review Pull again"
                    .to_string(),
            );
        }
        if !current_claims.is_empty()
            && action.kind.preservation().is_none()
            && !action.kind.is_destructive_overwrite()
        {
            return Err(
                    "this target is claimed by another storage at a different version; choose preservation, acknowledged overwrite, or Skip"
                        .to_string(),
                );
        }
    }

    let manifest_resources: BTreeMap<CatalogResourceId, CatalogResource> = plan
        .items
        .iter()
        .filter_map(|item| {
            item.resource
                .as_ref()
                .map(|resource| (item.item.resource_id.clone(), resource.clone()))
        })
        .collect();
    let engine = engine_for(storage)?;
    let repository_id = plan.repository_id.clone();
    let fetch = |sha256: &str, size: u64| engine.fetch_payload(&repository_id, sha256, size);
    let environment = PullApplyEnvironment {
        repository: catalog,
        runner,
        fetch_payload: &fetch,
    };
    let mut results = Vec::new();
    let mut accepted_updates: Vec<(CatalogResourceId, String)> = Vec::new();
    let mut all_accepted = true;
    for selection in &selections {
        let result = execute_scope_pull_item(
            &environment,
            &plan.storage_id,
            &plan.plan_id,
            &manifest_resources,
            &selection.item.item,
            selection.action,
            now,
        );
        // A row's comparison base advances only when the user actually acted
        // on it, or when both sides already match. A skipped executable row
        // stays unreviewed — silently accepting it would flip a later
        // StorageChanged reading into LocalChanged.
        let acted = selection.action.is_some()
            || result.outcome == super::domain::CatalogActionOutcome::AlreadyMatching;
        if pull_outcome_accepts_remote(result.outcome) {
            if acted {
                if let Some(resource) = &selection.item.resource {
                    accepted_updates.push((
                        selection.item.item.resource_id.clone(),
                        resource.content_sha256().to_string(),
                    ));
                }
            }
        } else {
            all_accepted = false;
        }
        results.push(result);
    }

    let advance_head = all_accepted;
    let base_changed = accepted_updates
        .iter()
        .any(|(resource_id, content)| link.accepted_resources.get(resource_id) != Some(content))
        || (advance_head && link.accepted_head.as_ref() != Some(&plan.head));
    if base_changed {
        let link_id = link.link_id.clone();
        let link_revision = plan.link_revision;
        let head = plan.head.clone();
        catalog.mutate_scope_links(|document| {
            let link = document
                .links
                .iter_mut()
                .find(|link| link.link_id == link_id)
                .ok_or("the scope link disappeared while accepting Pull")?;
            if link.revision != link_revision {
                return Err("the scope link changed while accepting Pull".to_string());
            }
            for (resource_id, content) in &accepted_updates {
                link.accepted_resources
                    .insert(resource_id.clone(), content.clone());
            }
            if advance_head {
                link.accepted_head = Some(head.clone());
            }
            link.revision = link.revision.saturating_add(1);
            link.updated_at = now;
            Ok(())
        })?;
    }
    plan_store::discard(catalog.root(), &plan_id);
    Ok(CatalogScopePullOutcomeDto {
        plan_id,
        remote_accepted: advance_head,
        results,
    })
}

fn prepare_fork_with_repositories(
    catalog: &CatalogStateRepository,
    runner: &dyn PluginProcessRunner,
    scope_id: &CatalogScopeId,
    resource_id: &CatalogResourceId,
    requested_name: &str,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourceForkPlan, String> {
    let ResolvedScope { scope, .. } = resolve_scope(catalog, scope_id)?;
    let context = resolve_profile_context(
        &catalog.load_profiles()?,
        &scope.profile_id,
        scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory = inventory_local(&context, scope.resource_kind, runner)
        .map_err(|blocker| blocker.to_string())?;
    let name_taken = name_taken_index(&inventory);
    let plan = build_resource_fork_plan(ScopeForkPlanInputs {
        scope: &scope,
        resource_id,
        context: &context,
        inventory: &inventory,
        requested_name,
        name_taken: &name_taken,
        now,
    })?;
    // A Fork review supersedes per source: re-asking for the same resource
    // replaces its pending review.
    plan_store::discard_where(catalog.root(), |existing| {
        matches!(
            existing,
            CatalogResourcePlanDocument::Fork(existing)
                if &existing.scope_id == scope_id && &existing.source_resource_id == resource_id
        )
    });
    plan_store::save(
        catalog.root(),
        CatalogResourcePlanDocument::Fork(plan.clone()),
        now,
    )?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::super::domain::{AgentSkillScope, CatalogActionOutcome, CatalogResourceRelation};
    use super::*;
    use mallard_domain::{StorageKind, SyncConfig};

    struct EmptyCodexRunner;

    impl PluginProcessRunner for EmptyCodexRunner {
        fn run(
            &self,
            provider: ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<super::super::adapters::CmdOutput, String> {
            assert_eq!(provider, ProviderKey::Codex);
            let stdout = match argv.join(" ").as_str() {
                "plugin marketplace list --json" => r#"{"marketplaces":[]}"#,
                "plugin list --json" => r#"{"installed":[]}"#,
                other => panic!("unexpected provider command: {other}"),
            };
            Ok(super::super::adapters::CmdOutput {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            })
        }
    }

    fn storage(value: &str) -> StorageId {
        StorageId::parse(value).unwrap()
    }

    #[test]
    fn isolated_service_discovers_a_profile_skill_through_the_production_path() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let machine_home = temporary.path().join("machine");
        let user_home = temporary.path().join("user");
        let codex_home = temporary.path().join("myconf1");
        let skill = codex_home.join("skills").join("mallard-alpha");
        fs::create_dir_all(&skill).expect("create test skill");
        fs::create_dir_all(&machine_home).expect("create machine home");
        fs::create_dir_all(&user_home).expect("create user home");
        fs::write(
            skill.join("SKILL.md"),
            b"---\nname: mallard-alpha\ndescription: isolated test skill\n---\nBody\n",
        )
        .expect("write test skill");

        let service = StorageCatalogService::from_home_dirs(
            &machine_home,
            &user_home,
            Box::new(EmptyCodexRunner),
        )
        .expect("construct isolated service");
        let profile = service
            .save_profile(ProviderKey::Codex, codex_home.to_string_lossy().to_string())
            .expect("save profile");
        assert_eq!(profile.display_name, "myconf1");
        let scope = service
            .create_scope(
                ProviderKey::Codex,
                profile.profile_id,
                CatalogResourceKindTag::AgentSkill,
            )
            .expect("create scope");
        let resources = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .expect("list resources");

        assert_eq!(resources.rows.len(), 1);
        assert_eq!(resources.rows[0].display_name, "mallard-alpha");
        assert!(resources.rows[0].local_present);
        assert!(resources.link.is_none());
        assert!(!resources.rows[0].remote_present);
    }

    #[test]
    fn isolated_service_rejects_relative_homes() {
        assert!(StorageCatalogService::from_home_dirs(
            "relative-machine",
            "/absolute-user",
            Box::new(EmptyCodexRunner),
        )
        .is_err());
        assert!(StorageCatalogService::from_home_dirs(
            "/absolute-machine",
            "relative-user",
            Box::new(EmptyCodexRunner),
        )
        .is_err());
    }

    #[test]
    fn failed_or_partial_pull_never_advances_the_comparison_base() {
        assert!(!pull_outcome_accepts_remote(CatalogActionOutcome::Failed));
        assert!(!pull_outcome_accepts_remote(CatalogActionOutcome::Partial));
        assert!(pull_outcome_accepts_remote(CatalogActionOutcome::Updated));
        assert!(pull_outcome_accepts_remote(CatalogActionOutcome::Skipped));
    }

    /// Panics on any provider CLI invocation, so "this path does no inventory
    /// work" is asserted by construction rather than by timing.
    struct PanicRunner;

    impl PluginProcessRunner for PanicRunner {
        fn run(
            &self,
            _provider: ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<super::super::adapters::CmdOutput, String> {
            panic!("provider inventory ran unexpectedly: {}", argv.join(" "));
        }
    }

    struct LibraryFixture {
        _temporary: tempfile::TempDir,
        user_home: PathBuf,
        skill_path: PathBuf,
        storage_path: PathBuf,
        state: LocalStateRepository,
        catalog: CatalogStateRepository,
        profile: LocalCatalogProfile,
        storage: StorageConfig,
    }

    fn library_fixture() -> LibraryFixture {
        let temporary = tempfile::tempdir().unwrap();
        let user_home = temporary.path().join("user");
        let codex_home = user_home.join("codex-home");
        let skill_path = codex_home.join("skills/review");
        let storage_path = temporary.path().join("storage");
        fs::create_dir_all(&skill_path).unwrap();
        fs::create_dir_all(&storage_path).unwrap();
        fs::write(
            skill_path.join("SKILL.md"),
            b"---\nname: review\n---\nDraft-linked skill.\n",
        )
        .unwrap();

        let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
        let profile = save_catalog_resource_profile_with_repository(
            &catalog,
            ProviderKey::Codex,
            codex_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let storage = StorageConfig {
            id: storage("library-test"),
            name: "Library test storage".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: storage_path.to_string_lossy().to_string(),
            supports_conditional_writes: None,
        };
        let mut config = SyncConfig::default();
        config.storages.push(storage.clone());
        state.save_config(config).unwrap();

        LibraryFixture {
            _temporary: temporary,
            user_home,
            skill_path,
            storage_path,
            state,
            catalog,
            profile,
            storage,
        }
    }

    #[test]
    fn listing_profiles_registers_the_existing_default_codex_and_claude_homes() {
        let temporary = tempfile::tempdir().unwrap();
        let user_home = temporary.path().join("user");
        fs::create_dir_all(user_home.join(".codex")).unwrap();
        fs::create_dir_all(user_home.join(".claude")).unwrap();
        let codex_home = fs::canonicalize(user_home.join(".codex")).unwrap();
        let claude_home = fs::canonicalize(user_home.join(".claude")).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();

        let first =
            list_catalog_resource_profiles_with_repository(&catalog, &user_home, 7).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].provider_key, ProviderKey::Codex);
        assert_eq!(first[0].display_name, "Default Codex");
        assert_eq!(first[0].configured_home, codex_home.to_string_lossy());
        assert_eq!(first[1].provider_key, ProviderKey::Claude);
        assert_eq!(first[1].display_name, "Default Claude");
        assert_eq!(first[1].configured_home, claude_home.to_string_lossy());

        let revision = catalog.load_profiles().unwrap().revision;
        let second =
            list_catalog_resource_profiles_with_repository(&catalog, &user_home, 9).unwrap();
        assert_eq!(second, first);
        assert_eq!(catalog.load_profiles().unwrap().revision, revision);
    }

    fn review_resource_id() -> CatalogResourceId {
        super::super::domain::skill_resource_id(AgentSkillScope::CodexHome, "review", "review")
    }

    fn second_storage(fixture: &LibraryFixture) -> StorageConfig {
        let path = fixture.user_home.join("second-storage");
        fs::create_dir_all(&path).unwrap();
        let storage = StorageConfig {
            id: storage("library-second"),
            name: "Second library storage".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: path.to_string_lossy().to_string(),
            supports_conditional_writes: None,
        };
        fixture
            .state
            .mutate_config(|config| {
                config.storages.push(storage.clone());
                Ok(())
            })
            .unwrap();
        storage
    }

    fn skill_scope(fixture: &LibraryFixture) -> CatalogScopeDto {
        create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::AgentSkill,
            2,
        )
        .unwrap()
    }

    fn link_scope(
        fixture: &LibraryFixture,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        now: u64,
    ) -> CatalogScopeLinkOutcomeDto {
        link_catalog_scope_storage_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            scope_id,
            storage_id,
            &fixture.user_home,
            now,
        )
        .unwrap()
    }

    /// Push everything selectable through the production prepare/apply pair.
    fn push_all(fixture: &LibraryFixture, scope_id: &CatalogScopeId, storage_id: &StorageId) {
        let plan = prepare_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            scope_id,
            storage_id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        let selected: Vec<CatalogResourceId> = plan
            .rows
            .iter()
            .filter(|row| row.row.selectable)
            .map(|row| row.row.resource_id.clone())
            .collect();
        let acknowledged: Vec<String> = plan.warning_digests.clone();
        let outcome = apply_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            plan.plan_id.clone(),
            selected,
            acknowledged,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert!(outcome.published, "{}", outcome.message);
    }

    #[test]
    fn profile_launches_resolve_the_home_and_fail_closed_when_it_vanishes() {
        let fixture = library_fixture();
        let missing = LocalCatalogProfileId::generate().unwrap();
        let unknown = profile_launch_home(&fixture.catalog, &missing).unwrap_err();
        assert!(unknown.contains("unknown catalog profile"), "{unknown}");

        let (provider, home) =
            profile_launch_home(&fixture.catalog, &fixture.profile.profile_id).unwrap();
        assert_eq!(provider, ProviderKey::Codex);
        assert_eq!(home.to_string_lossy(), fixture.profile.configured_home);

        fs::remove_dir_all(&home).unwrap();
        let gone = profile_launch_home(&fixture.catalog, &fixture.profile.profile_id).unwrap_err();
        assert!(gone.contains("home folder is missing"), "{gone}");
    }

    #[test]
    fn creating_a_scope_is_metadata_only_and_unique_per_profile_and_kind() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        assert_eq!(row.scope.resource_kind, CatalogResourceKindTag::AgentSkill);
        assert_eq!(row.profile.profile_id, fixture.profile.profile_id);

        let duplicate = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::AgentSkill,
            3,
        )
        .unwrap_err();
        assert_eq!(duplicate, "this Catalog row already exists");

        // Pet v1 and v2 are distinct kinds and therefore distinct rows.
        for kind in [
            CatalogResourceKindTag::CodexPetV1,
            CatalogResourceKindTag::CodexPetV2,
        ] {
            create_catalog_scope_with_repositories(
                &fixture.catalog,
                ProviderKey::Codex,
                fixture.profile.profile_id.clone(),
                kind,
                4,
            )
            .unwrap();
        }
        assert_eq!(fixture.catalog.load_scopes().unwrap().scopes.len(), 3);

        let rejected = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Claude,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV1,
            5,
        )
        .unwrap_err();
        assert_eq!(rejected, "this provider cannot host that resource kind");
    }

    #[test]
    fn linking_a_scope_storage_links_without_syncing_content() {
        let fixture = library_fixture();
        let original = fs::read(fixture.skill_path.join("SKILL.md")).unwrap();
        let row = skill_scope(&fixture);

        let outcome = link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        assert_eq!(outcome.completion, "open");
        assert_eq!(
            fs::read(fixture.skill_path.join("SKILL.md")).unwrap(),
            original
        );
        assert!(!fixture.storage_path.join(".mallard/v1/library").exists());

        // Linking twice is idempotent: same link, no second record.
        let again = link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 4);
        assert_eq!(again.link_id, outcome.link_id);
        assert_eq!(fixture.catalog.load_scope_links().unwrap().links.len(), 1);

        // The page reflects the link and the local skill's New state.
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.storage_id.as_ref(), Some(&fixture.storage.id));
        assert_eq!(
            page.link.as_ref().map(|link| link.link_id.clone()),
            Some(outcome.link_id)
        );
        assert!(page.rows[0].local_present);
        assert!(!page.rows[0].remote_present);
        assert_eq!(page.rows[0].state, Some(CatalogLinkState::New));
        assert!(page.rows[0].local_modified_at.is_some());
    }

    #[test]
    fn a_scope_page_pairs_local_and_stored_resources_into_one_row() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let page = |storage: Option<&StorageId>| {
            list_catalog_scope_resources_with_repositories(
                &fixture.state,
                &fixture.catalog,
                &EmptyCodexRunner,
                &row.scope.scope_id,
                storage,
                &fixture.user_home,
            )
            .unwrap()
        };

        // Nothing linked yet: the local skill is still a row, and it is the
        // storage's problem that it has never seen it.
        let before = page(None);
        assert_eq!(before.rows.len(), 1);
        assert!(before.link.is_none());
        assert_eq!(before.rows[0].state, Some(CatalogLinkState::New));

        // Link and push, then read the page again: the same one row, now
        // paired with the stored copy and in sync.
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let after = page(Some(&fixture.storage.id));
        assert_eq!(after.rows.len(), 1);
        assert!(after.rows[0].local_present && after.rows[0].remote_present);
        assert_eq!(after.rows[0].state, Some(CatalogLinkState::InSync));
        assert_eq!(after.storage_revision, Some(1));
        assert!(after.rows[0].remote_updated_at.is_some());

        // A second storage has never held it, so the same resource reads as
        // New there — the page's state belongs to the pairing, not the row.
        let second = second_storage(&fixture);
        let elsewhere = page(Some(&second.id));
        assert_eq!(elsewhere.rows.len(), 1);
        assert!(elsewhere.link.is_none());
        assert_eq!(elsewhere.rows[0].state, Some(CatalogLinkState::New));
    }

    /// A second machine, built beside the fixture's home and pointed at the
    /// same storage. Its skill carries whatever body the test wants to
    /// compare against the first machine's push.
    fn sibling_machine(
        fixture: &LibraryFixture,
        name: &str,
        skill_body: &[u8],
    ) -> (
        PathBuf,
        LocalStateRepository,
        CatalogStateRepository,
        CatalogScopeDto,
    ) {
        let user_home = fixture.user_home.parent().unwrap().join(name);
        let codex_home = user_home.join("codex-home");
        let skill_path = codex_home.join("skills/review");
        fs::create_dir_all(&skill_path).unwrap();
        fs::write(skill_path.join("SKILL.md"), skill_body).unwrap();

        let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
        let profile = save_catalog_resource_profile_with_repository(
            &catalog,
            ProviderKey::Codex,
            codex_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let mut config = SyncConfig::default();
        config.storages.push(fixture.storage.clone());
        state.save_config(config).unwrap();
        let scope = create_catalog_scope_with_repositories(
            &catalog,
            ProviderKey::Codex,
            profile.profile_id,
            CatalogResourceKindTag::AgentSkill,
            2,
        )
        .unwrap();
        (user_home, state, catalog, scope)
    }

    /// The convergence rule end to end: every replica of one kind lands on
    /// the same repository, and the manifest is the union of what they push.
    #[test]
    fn replicas_of_one_kind_converge_on_one_repository() {
        let fixture = library_fixture();
        // Machine A owns two skills and pushes both.
        let second_skill = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&second_skill).unwrap();
        fs::write(
            second_skill.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        // Machine B carries its own third skill plus an identical copy of
        // `review`. Linking pins the identical copy silently and routes the
        // stored-only skill to Pull.
        let (twin_home, twin_state, twin_catalog, twin_scope) = sibling_machine(
            &fixture,
            "twin-user",
            b"---\nname: review\n---\nDraft-linked skill.\n",
        );
        let third_skill = twin_home.join("codex-home/skills/extra");
        fs::create_dir_all(&third_skill).unwrap();
        fs::write(
            third_skill.join("SKILL.md"),
            b"---\nname: extra\n---\nThird skill.\n",
        )
        .unwrap();
        let linked = link_catalog_scope_storage_with_repositories(
            &twin_state,
            &twin_catalog,
            &EmptyCodexRunner,
            &twin_scope.scope.scope_id,
            &fixture.storage.id,
            &twin_home,
            5,
        )
        .unwrap();
        assert_eq!(linked.completion, "review_pull");
        let twin_page = list_catalog_scope_resources_with_repositories(
            &twin_state,
            &twin_catalog,
            &EmptyCodexRunner,
            &twin_scope.scope.scope_id,
            Some(&fixture.storage.id),
            &twin_home,
        )
        .unwrap();
        assert_eq!(twin_page.rows.len(), 3);
        let state_of = |name: &str| {
            twin_page
                .rows
                .iter()
                .find(|row| row.display_name == name)
                .unwrap()
                .state
        };
        assert_eq!(state_of("review"), Some(CatalogLinkState::InSync));
        assert_eq!(state_of("notes"), Some(CatalogLinkState::UnreviewedRemote));
        assert_eq!(state_of("extra"), Some(CatalogLinkState::New));

        // B pushes its own skill: one repository, a manifest of three.
        let twin_fixture = LibraryFixture {
            _temporary: tempfile::tempdir().unwrap(),
            user_home: twin_home.clone(),
            skill_path: twin_home.join("codex-home/skills/review"),
            storage_path: fixture.storage_path.clone(),
            state: twin_state,
            catalog: twin_catalog,
            profile: fixture.profile.clone(),
            storage: fixture.storage.clone(),
        };
        push_all(
            &twin_fixture,
            &twin_scope.scope.scope_id,
            &fixture.storage.id,
        );

        let library = fixture.storage_path.join(".mallard/v1/library");
        assert_eq!(fs::read_dir(&library).unwrap().count(), 1);
        let engine = engine_for(&fixture.storage).unwrap();
        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
        let (head, manifest) = engine.inspect(&repository_id).unwrap().unwrap();
        assert_eq!(head.head.revision, 2);
        assert_eq!(manifest.resources.len(), 3);

        // A pulls B's skill through one review.
        let pull = prepare_pull_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            &fixture.storage.id,
            BTreeMap::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert_eq!(pull.items.len(), 3);
        let extra = pull
            .items
            .iter()
            .find(|item| item.item.display_name == "extra")
            .unwrap();
        let choices = BTreeMap::from([(
            extra.item.resource_id.clone(),
            extra.item.default_action_id.clone().unwrap(),
        )]);
        let outcome = apply_pull_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            pull.plan_id.clone(),
            choices,
            Vec::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert!(outcome.remote_accepted);
        assert!(fixture
            .user_home
            .join("codex-home/skills/extra/SKILL.md")
            .is_file());

        // The pulled review accepted every row, so A's page is fully in sync.
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            Some(&fixture.storage.id),
            &fixture.user_home,
        )
        .unwrap();
        assert!(page
            .rows
            .iter()
            .all(|row| row.state == Some(CatalogLinkState::InSync)));
    }

    #[test]
    fn a_push_never_touches_unselected_rows() {
        let fixture = library_fixture();
        let second_skill = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&second_skill).unwrap();
        fs::write(
            second_skill.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);

        let plan = prepare_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            &fixture.storage.id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert_eq!(plan.rows.len(), 2);
        let outcome = apply_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            plan.plan_id.clone(),
            vec![review_resource_id()],
            Vec::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert!(outcome.published);

        let engine = engine_for(&fixture.storage).unwrap();
        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::AgentSkill,
        );
        let (_, manifest) = engine.inspect(&repository_id).unwrap().unwrap();
        assert_eq!(manifest.resources.len(), 1);
        assert!(manifest.resources.contains_key(&review_resource_id()));

        // The selected resource remains visible as InSync while the
        // unselected resource still reads as New on the same page.
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            Some(&fixture.storage.id),
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(
            page.rows
                .iter()
                .map(|row| row.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["notes", "review"]
        );
        let notes = page
            .rows
            .iter()
            .find(|row| row.display_name == "notes")
            .unwrap();
        assert_eq!(notes.state, Some(CatalogLinkState::New));
        let review = page
            .rows
            .iter()
            .find(|row| row.display_name == "review")
            .unwrap();
        assert!(review.local_present && review.remote_present);
        assert_eq!(review.state, Some(CatalogLinkState::InSync));

        // The next review retains both rows. Its stable display-name order
        // matches the page; only the outstanding resource is selectable.
        let next_plan = prepare_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            &fixture.storage.id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert_eq!(
            next_plan
                .rows
                .iter()
                .map(|row| row.row.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["notes", "review"]
        );
        let notes = next_plan
            .rows
            .iter()
            .find(|row| row.row.display_name == "notes")
            .unwrap();
        assert_eq!(notes.row.relation, CatalogResourceRelation::LocalOnly);
        assert!(notes.row.selectable);
        let review = next_plan
            .rows
            .iter()
            .find(|row| row.row.display_name == "review")
            .unwrap();
        assert_eq!(review.row.relation, CatalogResourceRelation::Matching);
        assert!(!review.row.selectable);
        assert!(!review.row.held_for_pull);
    }

    /// A repository whose tag disagrees with its own derivation was not
    /// written by this app's publish path; the page fails closed.
    #[test]
    fn a_noncanonical_stored_repository_fails_closed() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let library = fixture.storage_path.join(".mallard/v1/library");
        let repository = fs::read_dir(&library).unwrap().next().unwrap().unwrap();
        let tag_path = repository.path().join("_tag.json");
        let tag_text = fs::read_to_string(&tag_path)
            .unwrap()
            .replace("agent_skill", "codex_pet_v1");
        fs::write(&tag_path, tag_text).unwrap();

        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            Some(&fixture.storage.id),
            &fixture.user_home,
        )
        .unwrap_err();
        assert!(
            page.contains("does not match its provider and kind derivation"),
            "{page}"
        );
    }

    #[test]
    fn the_storage_listing_derives_repositories_and_never_scans() {
        let fixture = library_fixture();
        // An empty storage: no derivable id answers, so nothing is listed.
        assert!(list_catalog_storage_repositories_with_repository(
            &fixture.state,
            &fixture.storage.id,
        )
        .unwrap()
        .is_empty());

        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let listed =
            list_catalog_storage_repositories_with_repository(&fixture.state, &fixture.storage.id)
                .unwrap();
        assert_eq!(listed.len(), 1);
        let repo = &listed[0];
        assert_eq!(repo.provider, ProviderKey::Codex);
        assert_eq!(repo.resource_kind, CatalogResourceKindTag::AgentSkill);
        assert_eq!(
            repo.repository_id,
            CatalogResourceRepositoryId::for_kind(
                ProviderKey::Codex,
                CatalogResourceKindTag::AgentSkill,
            )
            .as_str()
        );
        assert_eq!(repo.revision, Some(1));
        assert!(repo.updated_at.is_some());
        assert_eq!(repo.resource_names, vec!["review".to_string()]);
        assert!(repo.error.is_none());
    }

    #[test]
    fn an_unreadable_repository_lists_as_a_refusal_row_not_a_hole() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let library = fixture.storage_path.join(".mallard/v1/library");
        let repository = fs::read_dir(&library).unwrap().next().unwrap().unwrap();
        let tag_path = repository.path().join("_tag.json");
        let tag_text = fs::read_to_string(&tag_path)
            .unwrap()
            .replace("agent_skill", "codex_pet_v1");
        fs::write(&tag_path, tag_text).unwrap();

        let listed =
            list_catalog_storage_repositories_with_repository(&fixture.state, &fixture.storage.id)
                .unwrap();
        assert_eq!(listed.len(), 1);
        let repo = &listed[0];
        assert_eq!(repo.resource_kind, CatalogResourceKindTag::AgentSkill);
        assert!(repo.revision.is_none());
        assert!(repo.resource_names.is_empty());
        let error = repo.error.as_deref().unwrap();
        assert!(
            error.contains("does not match its provider and kind derivation"),
            "{error}"
        );
    }

    #[test]
    fn profile_names_derive_from_the_folder_and_deduplicate() {
        let fixture = library_fixture();
        let first_home = fixture.user_home.join("homes/myconf");
        let second_home = fixture.user_home.join("elsewhere/myconf");
        fs::create_dir_all(&first_home).unwrap();
        fs::create_dir_all(&second_home).unwrap();

        let first = save_catalog_resource_profile_with_repository(
            &fixture.catalog,
            ProviderKey::Codex,
            first_home.to_string_lossy().to_string(),
            10,
        )
        .unwrap();
        assert_eq!(first.display_name, "myconf");

        // Same folder name somewhere else: the derived name steps aside.
        let second = save_catalog_resource_profile_with_repository(
            &fixture.catalog,
            ProviderKey::Codex,
            second_home.to_string_lossy().to_string(),
            11,
        )
        .unwrap();
        assert_eq!(second.display_name, "myconf 2");

        // Re-picking a saved home selects it rather than failing or forking.
        let again = save_catalog_resource_profile_with_repository(
            &fixture.catalog,
            ProviderKey::Codex,
            first_home.to_string_lossy().to_string(),
            12,
        )
        .unwrap();
        assert_eq!(again.profile_id, first.profile_id);
        assert_eq!(
            fixture
                .catalog
                .load_profiles()
                .unwrap()
                .profiles
                .iter()
                .filter(|profile| profile.display_name.starts_with("myconf"))
                .count(),
            2
        );
    }

    #[test]
    fn renaming_a_profile_pins_its_revision_and_rejects_empty_names() {
        let fixture = library_fixture();

        let stale = rename_catalog_resource_profile_with_repository(
            &fixture.catalog,
            &fixture.profile.profile_id,
            "Workbench".to_string(),
            fixture.profile.revision + 5,
            20,
        )
        .unwrap_err();
        assert_eq!(stale, "the profile changed while renaming");

        let empty = rename_catalog_resource_profile_with_repository(
            &fixture.catalog,
            &fixture.profile.profile_id,
            "   ".to_string(),
            fixture.profile.revision,
            21,
        )
        .unwrap_err();
        assert_eq!(empty, "a profile name cannot be empty");

        let renamed = rename_catalog_resource_profile_with_repository(
            &fixture.catalog,
            &fixture.profile.profile_id,
            "  Workbench  ".to_string(),
            fixture.profile.revision,
            22,
        )
        .unwrap();
        assert_eq!(renamed.display_name, "Workbench");
        assert_eq!(renamed.revision, fixture.profile.revision + 1);
        assert_eq!(renamed.updated_at, 22);
    }

    #[test]
    fn linking_a_second_storage_creates_an_independent_link() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let second = second_storage(&fixture);

        let first = link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        let again = link_scope(&fixture, &row.scope.scope_id, &second.id, 4);
        assert_ne!(first.link_id, again.link_id);
        assert_eq!(fixture.catalog.load_scope_links().unwrap().links.len(), 2);
    }

    #[test]
    fn unlinking_pins_the_revision_and_forgets_the_link() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let linked = link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);

        let stale = unlink_catalog_scope_storage_with_repositories(
            &fixture.catalog,
            &linked.link_id,
            linked.revision + 5,
            &fixture.user_home,
        )
        .unwrap_err();
        assert!(stale.contains("refresh before unlinking"), "{stale}");

        assert!(unlink_catalog_scope_storage_with_repositories(
            &fixture.catalog,
            &linked.link_id,
            linked.revision,
            &fixture.user_home,
        )
        .unwrap());
        assert!(fixture.catalog.load_scope_links().unwrap().links.is_empty());
        // A storage with no remaining Library reference can be removed.
        assert!(fixture
            .catalog
            .storage_reference_reason(&fixture.storage.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn unrelated_local_changes_do_not_stale_a_push_review() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        let plan = prepare_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            &row.scope.scope_id,
            &fixture.storage.id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();

        // New unrelated resources appear after review: one valid, one whose
        // directory name cannot even capture. Neither belongs to the
        // selected rows, so the apply still lands.
        let skills_root = fixture.skill_path.parent().unwrap();
        fs::create_dir_all(skills_root.join("unrelated")).unwrap();
        fs::write(
            skills_root.join("unrelated/SKILL.md"),
            b"---\nname: unrelated\n---\nChanged elsewhere.\n",
        )
        .unwrap();
        fs::create_dir_all(skills_root.join("bad name")).unwrap();
        fs::write(
            skills_root.join("bad name/SKILL.md"),
            b"---\nname: bad-name\n---\n",
        )
        .unwrap();

        let outcome = apply_push_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &EmptyCodexRunner,
            plan.plan_id.clone(),
            vec![review_resource_id()],
            Vec::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        assert!(outcome.published, "{}", outcome.message);
    }

    #[test]
    fn a_pets_scope_lists_without_provider_clis() {
        let fixture = library_fixture();
        let codex_home = fixture.user_home.join("codex-home");
        let atlas = super::super::codex_pet::fixtures::webp_atlas(CodexPetFormat::V1);
        fs::create_dir_all(codex_home.join("pets/duck")).unwrap();
        fs::write(
            codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();
        fs::write(codex_home.join("pets/duck/spritesheet.webp"), &atlas).unwrap();

        let row = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV1,
            2,
        )
        .unwrap();
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].display_name, "Duck");
        assert_eq!(page.rows[0].scope_label.as_deref(), Some("v1"));
        assert!(page.rows[0].local_present);
        // The header-only fixture atlas cannot decode, and a preview is
        // decoration: the row ships without one, the page does not fail, and
        // the descriptor's sentence still arrives.
        assert!(page.rows[0].preview_png.is_none());
        assert_eq!(page.rows[0].pet_description.as_deref(), Some("A duck."));

        // The sibling v2 scope shares the pets directory but not the row: a
        // v1 pet never renders inside the v2 scope's page.
        let sibling = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV2,
            3,
        )
        .unwrap();
        let sibling_page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            &sibling.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert!(sibling_page.rows.is_empty());
    }

    /// A real, decodable PNG atlas with the format's exact dimensions — what
    /// the header-only fixtures cannot provide.
    fn decodable_png_atlas(format: CodexPetFormat) -> Vec<u8> {
        let (width, height) = format.atlas_dimensions();
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([90, 60, 30, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn a_pet_scope_lists_with_a_preview_derived_from_its_atlas() {
        let fixture = library_fixture();
        let codex_home = fixture.user_home.join("codex-home");
        let atlas = decodable_png_atlas(CodexPetFormat::V1);
        fs::create_dir_all(codex_home.join("pets/drake")).unwrap();
        fs::write(
            codex_home.join("pets/drake/pet.json"),
            br#"{"id":"drake","displayName":"Drake","description":"A local pet.","spritesheetPath":"spritesheet.png"}"#,
        )
        .unwrap();
        fs::write(codex_home.join("pets/drake/spritesheet.png"), &atlas).unwrap();

        let row = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV1,
            2,
        )
        .unwrap();
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.rows.len(), 1);
        let listed = &page.rows[0];
        assert_eq!(listed.pet_description.as_deref(), Some("A local pet."));
        let preview = listed.preview_png.as_deref().unwrap();
        assert!(preview.starts_with("data:image/png;base64,"), "{}", preview);
        // Derived once, cached by the atlas digest under `~/.mallard`.
        let cache = super::super::pet_preview::preview_cache_dir(&fixture.user_home);
        let digest = super::super::domain::sha256_hex(&atlas);
        assert!(cache.join(format!("{}.png", digest)).is_file());
    }

    #[test]
    fn a_stored_only_pet_serves_its_preview_from_the_storage_then_the_cache() {
        use super::super::domain::{
            CatalogFileEntry, CodexPetDescriptor, CodexPetResource, PortableCatalogOrigin,
        };

        let fixture = library_fixture();
        let atlas = decodable_png_atlas(CodexPetFormat::V2);
        let atlas_sha = super::super::domain::sha256_hex(&atlas);
        let descriptor = CodexPetDescriptor {
            pet_id: "biscuit".to_string(),
            display_name: "Biscuit".to_string(),
            description: "A stored-only pet.".to_string(),
            atlas_basename: "spritesheet.png".to_string(),
            atlas: CatalogFileEntry {
                sha256: atlas_sha.clone(),
                size: atlas.len() as u64,
                mode: 0o644,
                object_key: format!("_objects/{}", atlas_sha),
            },
        };
        let mut pet = CodexPetResource {
            descriptor,
            content_sha256: String::new(),
        };
        pet.content_sha256 = pet.compute_content_sha256(CodexPetFormat::V2);
        let resource = CatalogResource::CodexPetV2(pet);
        let resource_id = resource.resource_id();

        let repository_id = CatalogResourceRepositoryId::for_kind(
            ProviderKey::Codex,
            CatalogResourceKindTag::CodexPetV2,
        );
        let engine = engine_for(&fixture.storage).unwrap();
        engine
            .publish(PublishLibraryResourceRequest {
                repository_id: repository_id.clone(),
                tag: LibraryResourceTag {
                    schema_version: CATALOG_SCHEMA,
                    repository_id: repository_id.clone(),
                    provider: ProviderKey::Codex,
                    resource_kind: CatalogResourceKindTag::CodexPetV2,
                    created_at: 1,
                },
                observed: None,
                resources: BTreeMap::from([(
                    resource_id.clone(),
                    PublishedResourceEntry {
                        resource,
                        updated_at: 1,
                    },
                )]),
                payloads: BTreeMap::from([(atlas_sha.clone(), Bytes::from(atlas.clone()))]),
                captured_with: CatalogCapturedWith {
                    app: "mallard".to_string(),
                    version: "test".to_string(),
                },
                published_by_origin: PortableCatalogOrigin {
                    origin_host_token: "a".repeat(64),
                    origin_replica_token: "b".repeat(64),
                },
                updated_at: 1,
            })
            .unwrap();

        let row = create_catalog_scope_with_repositories(
            &fixture.catalog,
            ProviderKey::Codex,
            fixture.profile.profile_id.clone(),
            CatalogResourceKindTag::CodexPetV2,
            2,
        )
        .unwrap();
        let first = get_catalog_pet_preview_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            &fixture.storage.id,
            &resource_id,
            &fixture.user_home,
        )
        .unwrap();
        assert!(first.starts_with("data:image/png;base64,"), "{}", first);

        // Remove the payload object from the storage: the second ask must be
        // answered by the digest-keyed cache, never by another fetch.
        let payload = walkdir::WalkDir::new(&fixture.storage_path)
            .into_iter()
            .filter_map(Result::ok)
            .find(|entry| entry.file_type().is_file() && entry.file_name() == atlas_sha.as_str())
            .expect("the published atlas payload exists in the storage");
        fs::remove_file(payload.path()).unwrap();
        let second = get_catalog_pet_preview_with_repositories(
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            &fixture.storage.id,
            &resource_id,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(second, first);
    }

    /// `delete_catalog_scope` is live and UI-reachable. Its success path and
    /// revision guard are pinned here on their own.
    #[test]
    fn deleting_an_unowned_scope_removes_the_row_and_pins_its_revision() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);

        // A stale revision must not clobber a concurrent edit.
        let stale =
            delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 0)
                .unwrap_err();
        assert_eq!(stale, "this Catalog row changed; refresh first");
        assert!(fixture
            .catalog
            .load_scopes()
            .unwrap()
            .scope(&row.scope.scope_id)
            .is_some());

        let linked = link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        let linked_error = delete_catalog_scope_with_repositories(
            &fixture.catalog,
            &row.scope.scope_id,
            row.scope.revision,
        )
        .unwrap_err();
        assert_eq!(
            linked_error,
            "this row still links storages; unlink them first"
        );
        unlink_catalog_scope_storage_with_repositories(
            &fixture.catalog,
            &linked.link_id,
            linked.revision,
            &fixture.user_home,
        )
        .unwrap();

        assert!(
            delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 1)
                .unwrap()
        );
        assert!(fixture
            .catalog
            .load_scopes()
            .unwrap()
            .scope(&row.scope.scope_id)
            .is_none());

        // A row that is already gone reports false rather than failing.
        assert!(
            !delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 1)
                .unwrap()
        );
    }

    #[test]
    fn deleting_a_scope_refuses_while_it_links_storages() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        let refused =
            delete_catalog_scope_with_repositories(&fixture.catalog, &row.scope.scope_id, 1)
                .unwrap_err();
        assert!(refused.contains("still links storages"), "{refused}");
    }
}
