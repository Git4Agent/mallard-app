//! Non-Tauri application boundary for scope-rowed Library resources.
//!
//! Owns catalog scopes, per-storage links, and reviewed Push, Pull, and Fork
//! operations. It reads the neutral configured-storage list but never loads a
//! project registration, project binding, thread, or repository bundle.
//! Every provider-, kind-, or format-specific step dispatches through the
//! registry: a scope whose coordinate this build does not register lists as
//! unavailable and cannot be operated on.

use bytes::Bytes;
use mallard_domain::{now_secs, StorageConfig, StorageId, SyncConfig};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_core::ConfiguredObjectStore;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::adapters::{NativeProcessRunner, PluginProcessRunner, ResolvedCatalogTarget};
use super::apply::{
    apply_resource_fork, execute_scope_pull_item, recover_catalog_operations, PullApplyEnvironment,
};
use super::domain::{
    CatalogActionOutcome, CatalogActionResult, CatalogCapturedWith, CatalogCoordinate,
    CatalogCoordinateLabels, CatalogFormatKey, CatalogLinkId, CatalogLinkState,
    CatalogPlanActionId, CatalogPlanId, CatalogProviderConfig, CatalogResourceEnvelope,
    CatalogResourceForkPlan, CatalogResourceId, CatalogResourceKindKey,
    CatalogResourcePlanDocument, CatalogResourceWarning, CatalogScope, CatalogScopeId,
    CatalogScopeLink, CatalogScopePullPlan, CatalogScopePushPlan, CatalogTargetConfig,
    LibraryResourceManifest, LibraryResourceTag, LocalCatalogProfile, LocalCatalogProfileId,
    ProviderKey, PublishedResourceEntry,
};
use super::inventory::{inventory_local, CatalogLocalInventory, LocalResourceRecord};
use super::persistence::CatalogStateRepository;
use super::plan_store;
use super::profiles::{
    default_profile_home, resolve_profile_context, validate_new_profile_home,
    ResolvedCatalogContext,
};
use super::registry::{CatalogCapability, CatalogCapabilityDescriptor, CatalogRegistry};
use super::resource_engine::{
    head_coordinate, storage_identity_for_config, verify_library_continuity, LibraryResourceEngine,
    ObservedLibraryHead, PublishLibraryResourceRequest,
};
use super::resource_planning::{
    build_resource_fork_plan, build_scope_pull_plan, build_scope_push_plan, conflicting_claim_pins,
    scope_resource_state, validate_scope_pull_selection, ScopeForkPlanInputs, ScopePullPlanInputs,
    ScopePushPlanInputs,
};

fn pull_outcome_accepts_remote(outcome: CatalogActionOutcome) -> bool {
    !matches!(
        outcome,
        CatalogActionOutcome::Failed | CatalogActionOutcome::Partial
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
    Ok(LibraryResourceEngine::new(ConfiguredObjectStore::shared(
        storage,
    )?))
}

/// Machine-local, digest-keyed, and disposable: wiping `~/.mallard` wipes it,
/// and every entry can be re-derived from its source.
fn preview_cache_dir(user_home: &Path) -> PathBuf {
    user_home.join(".mallard").join("preview_cache")
}

/// One Library profile as the frontend sees it: the record plus the
/// registered provider's label. The provider's own configuration envelope
/// stays machine-local.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogProfileDto {
    pub profile_id: LocalCatalogProfileId,
    pub provider: ProviderKey,
    pub provider_label: String,
    pub display_name: String,
    pub configured_home: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

fn profile_dto(registry: &CatalogRegistry, profile: &LocalCatalogProfile) -> CatalogProfileDto {
    CatalogProfileDto {
        profile_id: profile.profile_id.clone(),
        provider: profile.provider.clone(),
        provider_label: registry
            .provider(&profile.provider)
            .map(|adapter| adapter.descriptor().label)
            .unwrap_or_else(|| profile.provider.to_string()),
        display_name: profile.display_name.clone(),
        configured_home: profile.configured_home.clone(),
        revision: profile.revision,
        created_at: profile.created_at,
        updated_at: profile.updated_at,
    }
}

/// One Catalog menu row plus the profile its label derives from and the
/// registered capability's presentation. A row whose coordinate this build
/// does not register is listed as unavailable: it keeps its raw coordinate,
/// names itself, and cannot be operated on.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeDto {
    pub scope: CatalogScope,
    pub profile: CatalogProfileDto,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    /// The registered capability's label, or the raw coordinate.
    pub label: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

fn scope_dto(
    registry: &CatalogRegistry,
    scope: CatalogScope,
    profile: &LocalCatalogProfile,
) -> CatalogScopeDto {
    let coordinate = scope.coordinate();
    let (label, available, unavailable_reason) = match registry.require_capability(&coordinate) {
        Ok(capability) => (capability.descriptor.label.clone(), true, None),
        Err(reason) => (coordinate.to_string(), false, Some(reason)),
    };
    CatalogScopeDto {
        labels: registry.labels_for(&coordinate),
        scope,
        profile: profile_dto(registry, profile),
        label,
        available,
        unavailable_reason,
    }
}

/// One row on a scope page: a resource of this coordinate, compared against
/// the one storage the page is scoped to.
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
    /// The codec's secondary label for this resource (a root class, a
    /// format version), when the coordinate does not say it all.
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
    /// A small PNG preview as a data URI, derived from local capture or read
    /// from the digest cache using the verified stored resource. Stored-only
    /// cache misses fetch lazily through `get_catalog_resource_preview`.
    pub preview_png: Option<String>,
    /// The codec's one-line description from the descriptor, when the
    /// format has one.
    pub description: Option<String>,
    pub warnings: Vec<CatalogResourceWarning>,
    /// Why this resource cannot be synced at all — a symlinked directory,
    /// an unreadable package. Set only with `state` Blocked.
    pub blocker: Option<String>,
}

/// The page-level scope link, when this scope is linked to the page's storage.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopePageLinkDto {
    pub link_id: CatalogLinkId,
    pub revision: u64,
}

/// The complete content of one scope page: the storage it compares against,
/// the scope's link to it, and one row per resource of this coordinate on
/// either side.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopeResourcesDto {
    pub scope: CatalogScope,
    pub profile: CatalogProfileDto,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    /// The caller's choice when it made one; otherwise the storage this
    /// scope's links already point at, so the page opens on the pairing that
    /// exists rather than on nothing.
    pub storage_id: Option<StorageId>,
    pub storage_name: Option<String>,
    pub link: Option<CatalogScopePageLinkDto>,
    /// The stored repository's current revision, when one exists.
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

/// One Library repository as the storage page lists it: the coordinate it
/// derives from with its labels, the head facts when the repository reads,
/// and the refusal when it does not. Absent repositories produce no entry.
#[derive(Serialize, Clone, Debug)]
pub struct CatalogStorageRepositoryDto {
    pub repository_id: String,
    #[serde(flatten)]
    pub coordinate: CatalogCoordinate,
    #[serde(flatten)]
    pub labels: CatalogCoordinateLabels,
    pub revision: Option<u64>,
    pub updated_at: Option<u64>,
    /// Display names of everything the set holds, sorted for the fold.
    pub resource_names: Vec<String>,
    /// Why the repository cannot be read; the row states it instead of chips.
    pub error: Option<String>,
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

/// Production Catalog use cases over explicit machine and provider homes and
/// one compiled registry.
pub struct StorageCatalogService {
    state: LocalStateRepository,
    catalog: CatalogStateRepository,
    registry: Arc<CatalogRegistry>,
    runner: Box<dyn PluginProcessRunner>,
    user_home: PathBuf,
}

impl StorageCatalogService {
    pub fn from_user_home() -> Result<Self, String> {
        let user_home = mallard_infrastructure::persistence::user_home_dir()
            .ok_or_else(|| "could not resolve the user home directory".to_string())?;
        let registry = CatalogRegistry::production();
        Self::with_registry(
            user_home.clone(),
            user_home,
            Box::new(NativeProcessRunner::new(registry.clone())),
            registry,
        )
    }

    pub fn from_home_dirs(
        machine_home: impl Into<PathBuf>,
        user_home: impl Into<PathBuf>,
        runner: Box<dyn PluginProcessRunner>,
    ) -> Result<Self, String> {
        Self::with_registry(
            machine_home,
            user_home,
            runner,
            CatalogRegistry::production(),
        )
    }

    pub fn with_registry(
        machine_home: impl Into<PathBuf>,
        user_home: impl Into<PathBuf>,
        runner: Box<dyn PluginProcessRunner>,
        registry: Arc<CatalogRegistry>,
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
            registry,
            runner,
            user_home,
        })
    }

    /// Every registered capability, in display order. The frontend lists
    /// these; it holds no provider, kind, format, or target table of its own.
    pub fn capabilities(&self) -> Vec<CatalogCapabilityDescriptor> {
        self.registry.descriptors()
    }

    pub fn list_profiles(&self) -> Result<Vec<CatalogProfileDto>, String> {
        Ok(list_catalog_resource_profiles_with_repository(
            &self.registry,
            &self.catalog,
            &self.user_home,
            now_secs(),
        )?
        .iter()
        .map(|profile| profile_dto(&self.registry, profile))
        .collect())
    }

    pub fn save_profile(
        &self,
        provider: ProviderKey,
        configured_home: String,
    ) -> Result<CatalogProfileDto, String> {
        let profile = save_catalog_resource_profile_with_repository(
            &self.registry,
            &self.catalog,
            provider,
            configured_home,
            now_secs(),
        )?;
        Ok(profile_dto(&self.registry, &profile))
    }

    pub fn rename_profile(
        &self,
        profile_id: &LocalCatalogProfileId,
        display_name: String,
        expected_revision: u64,
    ) -> Result<CatalogProfileDto, String> {
        let profile = rename_catalog_resource_profile_with_repository(
            &self.catalog,
            profile_id,
            display_name,
            expected_revision,
            now_secs(),
        )?;
        Ok(profile_dto(&self.registry, &profile))
    }

    pub fn open_profile_in_app(&self, profile_id: &LocalCatalogProfileId) -> Result<(), String> {
        let (provider, home) = profile_launch_home(&self.catalog, profile_id)?;
        self.registry
            .require_provider(&provider)?
            .launch_home_app(&home, &self.user_home)
    }

    pub fn open_profile_in_terminal(
        &self,
        profile_id: &LocalCatalogProfileId,
    ) -> Result<(), String> {
        let (provider, home) = profile_launch_home(&self.catalog, profile_id)?;
        self.registry
            .require_provider(&provider)?
            .launch_home_terminal(&home)
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
                    .ok_or("a Catalog row lost its provider profile")?;
                Ok(scope_dto(&self.registry, scope, profile))
            })
            .collect()
    }

    pub fn create_scope(
        &self,
        provider: ProviderKey,
        profile_id: LocalCatalogProfileId,
        resource_kind: CatalogResourceKindKey,
        format: CatalogFormatKey,
        format_version: u32,
    ) -> Result<CatalogScopeDto, String> {
        create_catalog_scope_with_repositories(
            &self.registry,
            &self.catalog,
            CatalogCoordinate {
                provider,
                resource_kind,
                format,
                format_version,
            },
            profile_id,
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
            &self.registry,
            &self.state,
            &self.catalog,
            scope_id,
            storage_id,
            &self.user_home,
        )
    }

    pub fn resource_preview(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
        resource_id: &CatalogResourceId,
    ) -> Result<String, String> {
        get_catalog_resource_preview_with_repositories(
            &self.registry,
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
        list_catalog_storage_repositories_with_repository(&self.registry, &self.state, storage_id)
    }

    pub fn link_scope_storage(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<CatalogScopeLinkOutcomeDto, String> {
        link_catalog_scope_storage_with_repositories(
            &self.registry,
            &self.state,
            &self.catalog,
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
            &self.registry,
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
            &self.registry,
            &self.state,
            &self.catalog,
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
            &self.registry,
            &self.state,
            &self.catalog,
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
            &self.registry,
            &self.state,
            &self.catalog,
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
            &self.registry,
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
            &self.registry,
            &self.catalog,
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
        let capability = require_capability(&self.registry, &scope)?;
        let context = resolve_profile_context(
            &self.registry,
            &self.catalog.load_profiles()?,
            &scope.profile_id,
            &scope.provider,
            &self.user_home,
        )
        .map_err(|blocker| blocker.to_string())?;
        context.verify_pins(&plan.profile_pins, &plan.target_pins)?;
        let result = apply_resource_fork(
            &self.registry,
            &self.catalog,
            &capability,
            &plan,
            now_secs(),
        )?;
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
    if profile.provider != scope.provider {
        return Err("this Catalog row's profile changed its provider".to_string());
    }
    Ok(ResolvedScope { scope, profile })
}

/// Registry validation for one scope: a coordinate this build does not
/// register loads and lists, but nothing operates on it.
fn require_capability(
    registry: &CatalogRegistry,
    scope: &CatalogScope,
) -> Result<Arc<CatalogCapability>, String> {
    registry
        .require_capability(&scope.coordinate())
        .map_err(|reason| format!("this Catalog row cannot be operated on: {}", reason))
}

fn list_catalog_resource_profiles_with_repository(
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    user_home: &Path,
    now: u64,
) -> Result<Vec<LocalCatalogProfile>, String> {
    let providers = registry.providers();
    let candidates = providers
        .iter()
        .filter_map(|descriptor| {
            let adapter = registry.provider(&descriptor.key)?;
            let home = default_profile_home(adapter, user_home)?;
            home.is_dir().then_some((descriptor, home))
        })
        .map(|(descriptor, home)| {
            let canonical = validate_new_profile_home(catalog.root(), &home.to_string_lossy())?;
            Ok((descriptor.key.clone(), descriptor.label.clone(), canonical))
        })
        .collect::<Result<Vec<_>, String>>()?;

    if !candidates.is_empty() {
        let current = catalog.load_profiles()?;
        let has_missing = candidates.iter().any(|(provider, _, canonical)| {
            !current.profiles.iter().any(|profile| {
                &profile.provider == provider && Path::new(&profile.configured_home) == canonical
            })
        });
        if has_missing {
            catalog.mutate_profiles(|document| {
                for (provider, label, canonical) in candidates {
                    if document.profiles.iter().any(|profile| {
                        profile.provider == provider
                            && Path::new(&profile.configured_home) == canonical
                    }) {
                        continue;
                    }
                    document.profiles.push(LocalCatalogProfile {
                        profile_id: LocalCatalogProfileId::generate()?,
                        provider,
                        display_name: format!("Default {}", label),
                        configured_home: canonical.to_string_lossy().to_string(),
                        provider_config: CatalogProviderConfig::version_1(),
                        revision: 1,
                        created_at: now,
                        updated_at: now,
                    });
                }
                Ok(())
            })?;
        }
    }

    let rank: BTreeMap<ProviderKey, usize> = providers
        .iter()
        .enumerate()
        .map(|(index, descriptor)| (descriptor.key.clone(), index))
        .collect();
    let rank_of = |provider: &ProviderKey| rank.get(provider).copied().unwrap_or(usize::MAX);
    let mut profiles = catalog.load_profiles()?.profiles;
    profiles.sort_by(|left, right| {
        rank_of(&left.provider)
            .cmp(&rank_of(&right.provider))
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    Ok(profiles)
}

/// The name a profile home gets without anyone typing one: its folder name,
/// de-duplicated against the names already taken. Setup never asks for a
/// name — renaming afterwards is `rename_catalog_resource_profile`.
fn derived_profile_name(
    provider_label: &str,
    canonical: &Path,
    taken: &[LocalCatalogProfile],
) -> String {
    let base = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} profile", provider_label));
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
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    path: String,
    now: u64,
) -> Result<LocalCatalogProfile, String> {
    let provider_label = registry.require_provider(&provider)?.descriptor().label;
    let canonical = validate_new_profile_home(catalog.root(), &path)?;
    let canonical_home = canonical.to_string_lossy().to_string();
    catalog.mutate_profiles(|document| {
        // Re-picking a home that is already a profile selects it rather than
        // failing the submission.
        if let Some(existing) = document.profiles.iter().find(|profile| {
            profile.provider == provider && profile.configured_home == canonical_home
        }) {
            return Ok(existing.clone());
        }
        let profile = LocalCatalogProfile {
            profile_id: LocalCatalogProfileId::generate()?,
            provider: provider.clone(),
            display_name: derived_profile_name(&provider_label, &canonical, &document.profiles),
            configured_home: canonical_home.clone(),
            provider_config: CatalogProviderConfig::version_1(),
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
    Ok((profile.provider.clone(), home))
}

fn create_catalog_scope_with_repositories(
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    coordinate: CatalogCoordinate,
    profile_id: LocalCatalogProfileId,
    now: u64,
) -> Result<CatalogScopeDto, String> {
    coordinate.validate()?;
    registry.require_capability(&coordinate)?;
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(&profile_id)
        .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?
        .clone();
    if profile.provider != coordinate.provider {
        return Err(format!(
            "catalog profile '{}' is a {} profile, not {}",
            profile_id, profile.provider, coordinate.provider
        ));
    }
    let scope = catalog.mutate_scopes(|document| {
        if document
            .scopes
            .iter()
            .any(|scope| scope.profile_id == profile_id && scope.coordinate() == coordinate)
        {
            return Err("this Catalog row already exists".to_string());
        }
        let scope = CatalogScope {
            scope_id: CatalogScopeId::generate()?,
            provider: coordinate.provider.clone(),
            profile_id: profile_id.clone(),
            resource_kind: coordinate.resource_kind.clone(),
            format: coordinate.format.clone(),
            format_version: coordinate.format_version,
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        scope.validate()?;
        document.scopes.push(scope.clone());
        Ok(scope)
    })?;
    Ok(scope_dto(registry, scope, &profile))
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
/// registered capability, its profile context and inventory, and — when a
/// storage participates — the storage config, the scope link, and the
/// verified repository view.
struct ResolvedScopeWorkspace {
    scope: CatalogScope,
    profile: LocalCatalogProfile,
    capability: Arc<CatalogCapability>,
    context: ResolvedCatalogContext,
    inventory: CatalogLocalInventory,
    storage: Option<StorageConfig>,
    link: Option<CatalogScopeLink>,
    observed: Option<ObservedLibraryHead>,
    manifest: Option<LibraryResourceManifest>,
}

impl ResolvedScopeWorkspace {
    fn remote_map(&self) -> BTreeMap<CatalogResourceId, CatalogResourceEnvelope> {
        self.manifest
            .as_ref()
            .map(LibraryResourceManifest::resource_map)
            .unwrap_or_default()
    }
}

fn resolve_scope_workspace(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    requested_storage: Option<&StorageId>,
    user_home: &Path,
) -> Result<ResolvedScopeWorkspace, String> {
    let ResolvedScope { scope, profile } = resolve_scope(catalog, scope_id)?;
    let capability = require_capability(registry, &scope)?;
    let context = resolve_profile_context(
        registry,
        &catalog.load_profiles()?,
        &scope.profile_id,
        &scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory =
        inventory_local(registry, &capability, &context).map_err(|blocker| blocker.to_string())?;

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

    let coordinate = scope.coordinate();
    let repository_id = capability.repository_id();
    if let Some(link) = &link {
        if link.repository_id != repository_id {
            return Err(
                "this scope link points at a repository its coordinate no longer derives"
                    .to_string(),
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
            &storage_identity_for_config(storage)?,
        )?;
        if let Some(observed) = &observed {
            // The tag freezes the coordinate; read_tag validates the
            // derivation, so a repository written under a foreign identity
            // fails the page closed here.
            let (tag, _) = engine
                .read_tag(&repository_id)?
                .ok_or("the stored repository is missing its discovery tag")?;
            if tag.coordinate() != coordinate {
                return Err("the stored repository belongs to another coordinate".to_string());
            }
            let read = engine.read_manifest(&observed.head)?;
            // Structural validation proved the set; the registered codec
            // proves every entry is a resource of this exact coordinate.
            for entry in read.resources.values() {
                if entry.resource.coordinate() != coordinate {
                    return Err(
                        "the stored repository holds a resource of another coordinate".to_string(),
                    );
                }
                capability
                    .codec
                    .validate(&entry.resource)
                    .map_err(|error| format!("a stored resource failed validation: {}", error))?;
            }
            manifest = Some(read);
        }
    }
    Ok(ResolvedScopeWorkspace {
        scope,
        profile,
        capability,
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
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: Option<&StorageId>,
    user_home: &Path,
) -> Result<CatalogScopeResourcesDto, String> {
    let workspace =
        resolve_scope_workspace(registry, state, catalog, scope_id, storage_id, user_home)?;
    let codec = workspace.capability.codec.as_ref();
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

    let mut resource_ids: BTreeSet<&CatalogResourceId> =
        workspace.inventory.resources.keys().collect();
    resource_ids.extend(remote.keys());

    let mut rows: Vec<CatalogScopeRowDto> = Vec::new();
    for resource_id in resource_ids {
        let local = workspace.inventory.resources.get(resource_id);
        let remote_resource = remote.get(resource_id);
        let local_sha = local.map(|record| record.resource.content_sha256.as_str());
        let remote_sha = remote_resource.map(|resource| resource.content_sha256.as_str());
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
        let Some(presented) = local.map(|record| &record.resource).or(remote_resource) else {
            continue;
        };
        let mut row = CatalogScopeRowDto {
            row_id: format!("resource:{}", resource_id),
            display_name: codec.display_name(presented)?,
            resource_id: Some((*resource_id).clone()),
            scope_label: codec.resource_label(presented),
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
            description: codec.description(presented),
            warnings: local
                .map(|record| record.warnings.clone())
                .unwrap_or_default(),
            blocker: None,
        };
        decorate_local_preview(&mut row, &workspace.capability, local, user_home);
        if local.is_none() {
            // The scope listing already verified this manifest. Read cached
            // artwork here so each card avoids another head and manifest read.
            // A cache miss stays lazy and must not fetch payloads in the list.
            row.preview_png = remote_resource.and_then(|resource| {
                codec
                    .preview(resource, &preview_cache_dir(user_home), &|_, _| {
                        Err("the stored preview is not cached".to_string())
                    })
                    .ok()
                    .flatten()
            });
        }
        rows.push(row);
    }

    // Present on disk and impossible to capture — a symlinked directory, an
    // unreadable package. Hiding them would make the page disagree with the
    // folder it claims to describe.
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
            description: None,
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

    let recovery = recover_catalog_operations(registry, catalog, now_secs())?;
    Ok(CatalogScopeResourcesDto {
        scope: workspace.scope,
        profile: profile_dto(registry, &workspace.profile),
        labels: workspace.capability.labels(),
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

/// A local row's preview comes from the codec over the bytes capture already
/// holds, derived once per digest and cached. Previews are decoration, so a
/// failed derivation leaves the row without one rather than failing the
/// page; the card draws the kind glyph instead. Stored-only cache misses use
/// `get_catalog_resource_preview`, so payload fetches stay out of the list.
fn decorate_local_preview(
    row: &mut CatalogScopeRowDto,
    capability: &CatalogCapability,
    local: Option<&LocalResourceRecord>,
    user_home: &Path,
) {
    let Some(record) = local else {
        return;
    };
    let Some(captured) = &record.captured else {
        return;
    };
    let payloads = captured.payloads();
    row.preview_png = capability
        .codec
        .preview(
            &record.resource,
            &preview_cache_dir(user_home),
            &|sha256, size| {
                payloads
                    .get(sha256)
                    .filter(|bytes| bytes.len() as u64 == size)
                    .map(|bytes| bytes.to_vec())
                    .ok_or_else(|| "the captured payload is missing".to_string())
            },
        )
        .ok()
        .flatten();
}

/// The preview for a stored-only row, fetched from the storage on demand and
/// cached by payload digest. Kept out of the list call on purpose: a page
/// must never wait on one storage round-trip per row, and this Mac may be
/// offline — the card asks lazily and keeps its glyph when this fails.
fn get_catalog_resource_preview_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    resource_id: &CatalogResourceId,
    user_home: &Path,
) -> Result<String, String> {
    let ResolvedScope { scope, .. } = resolve_scope(catalog, scope_id)?;
    let capability = require_capability(registry, &scope)?;
    let storage = storage_for(state, storage_id)?;
    let engine = engine_for(&storage)?;
    let repository_id = capability.repository_id();
    let (_, manifest) = engine
        .inspect(&repository_id)?
        .ok_or("the stored resource disappeared")?;
    let resource = manifest
        .resources
        .get(resource_id)
        .map(|entry| &entry.resource)
        .ok_or("the stored resource disappeared")?;
    if resource.coordinate() != scope.coordinate() {
        return Err("the stored resource does not belong to this Catalog row".to_string());
    }
    capability.codec.validate(resource)?;
    capability
        .codec
        .preview(resource, &preview_cache_dir(user_home), &|sha256, size| {
            engine.fetch_payload(&repository_id, sha256, size)
        })?
        .ok_or_else(|| "this resource has no preview".to_string())
}

/// The storage page's Catalog listing: every repository this build can
/// derive — one per registered capability — inspected with one head read
/// each. Nothing is scanned: a repository is either derivable and read, or
/// invisible to this build by construction. A derived id that answers but
/// fails its tag or manifest checks is returned as a refusal row, never
/// omitted — on a listing, a silent hole reads as "nothing stored".
fn list_catalog_storage_repositories_with_repository(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    storage_id: &StorageId,
) -> Result<Vec<CatalogStorageRepositoryDto>, String> {
    let storage = storage_for(state, storage_id)?;
    let engine = engine_for(&storage)?;
    Ok(registry
        .capabilities()
        .filter_map(|capability| read_storage_repository(&engine, capability))
        .collect())
}

/// `None` when the repository does not exist in this storage; a refusal row
/// when something exists under the derived id but cannot be read.
fn read_storage_repository(
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    capability: &CatalogCapability,
) -> Option<CatalogStorageRepositoryDto> {
    let repository_id = capability.repository_id();
    let coordinate = capability.coordinate();
    let refusal = |message: String| CatalogStorageRepositoryDto {
        repository_id: repository_id.as_str().to_string(),
        coordinate: coordinate.clone(),
        labels: capability.labels(),
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
            refusal("the stored repository has a discovery tag but no head".to_string())
        });
    };
    let Some((tag, _)) = tag else {
        return Some(refusal(
            "the stored repository is missing its discovery tag".to_string(),
        ));
    };
    // read_tag already enforced the derivation; this is the same
    // belt-and-braces identity check the scope workspace keeps.
    if tag.coordinate() != coordinate {
        return Some(refusal(
            "the stored repository belongs to another coordinate".to_string(),
        ));
    }
    let manifest = match engine.read_manifest(&observed.head) {
        Ok(manifest) => manifest,
        Err(message) => return Some(refusal(message)),
    };
    let names = manifest
        .resources
        .values()
        .map(|entry| {
            if entry.resource.coordinate() != coordinate {
                return Err(
                    "the stored repository holds a resource of another coordinate".to_string(),
                );
            }
            capability.codec.validate(&entry.resource)?;
            capability.codec.display_name(&entry.resource)
        })
        .collect::<Result<Vec<String>, String>>();
    let mut resource_names = match names {
        Ok(names) => names,
        Err(message) => {
            return Some(refusal(format!(
                "a stored resource failed validation: {}",
                message
            )))
        }
    };
    resource_names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    Some(CatalogStorageRepositoryDto {
        repository_id: repository_id.as_str().to_string(),
        coordinate,
        labels: capability.labels(),
        revision: Some(observed.head.revision),
        updated_at: Some(manifest.updated_at),
        resource_names,
        error: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn link_catalog_scope_storage_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopeLinkOutcomeDto, String> {
    let _operation = state.acquire_host_operation()?;
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
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
    let repository_id = workspace.capability.repository_id();

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
            .is_some_and(|record| record.resource.content_sha256 == remote_resource.content_sha256);
        if matches {
            accepted_resources.insert(resource_id.clone(), remote_resource.content_sha256.clone());
        }
    }
    let full_agreement = accepted_resources.len() == remote.len();
    let accepted_head = match (&workspace.observed, full_agreement) {
        (Some(observed), true) => Some(head_coordinate(
            &observed.head,
            &storage_identity_for_config(storage)?,
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

/// Whether a machine-local record belongs to this scope: the same coordinate
/// and a target root this profile currently resolves for the scope's
/// capability. Two profiles of one provider share a coordinate but never a
/// root, so their claims stay apart.
fn record_in_scope(
    coordinate: &CatalogCoordinate,
    target: &CatalogTargetConfig,
    scope_coordinate: &CatalogCoordinate,
    roots: &[&ResolvedCatalogTarget],
) -> bool {
    coordinate == scope_coordinate
        && target.decode().is_ok_and(|identity| {
            roots.iter().any(|root| {
                root.target == target.target
                    && root.canonical_root == Path::new(&identity.canonical_root)
            })
        })
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
    registry: &CatalogRegistry,
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
    let capability = require_capability(registry, &scope)?;
    let context = resolve_profile_context(
        registry,
        &catalog.load_profiles()?,
        &scope.profile_id,
        &scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let roots = context.targets_for(&capability);
    let scope_coordinate = scope.coordinate();
    let in_scope = |coordinate: &CatalogCoordinate, target: &CatalogTargetConfig| {
        record_in_scope(coordinate, target, &scope_coordinate, &roots)
    };

    let storage_id = link.storage_id.clone();
    let active_operation = catalog
        .load_operations()?
        .operations
        .into_iter()
        .any(|operation| {
            !operation.phase.is_terminal()
                && operation.storage_id.as_ref() == Some(&storage_id)
                && in_scope(&operation.coordinate, &operation.target)
        });
    if active_operation {
        return Err("an operation for this link is still in progress".to_string());
    }
    discard_scope_plans(catalog, &link.scope_id, &storage_id);
    catalog.mutate_materializations(|claims| {
        claims.claims.retain(|claim| {
            claim.storage_id != storage_id || !in_scope(&claim.coordinate, &claim.target)
        });
        Ok(())
    })?;
    catalog.mutate_preservations(|preservations| {
        preservations.witnesses.retain(|witness| {
            witness.storage_id != storage_id
                || !in_scope(&witness.coordinate, &witness.source_target)
        });
        Ok(())
    })?;
    catalog.mutate_operations(|operations| {
        operations.operations.retain(|operation| {
            !(operation.phase.is_terminal()
                && operation.storage_id.as_ref() == Some(&storage_id)
                && in_scope(&operation.coordinate, &operation.target))
        });
        Ok(())
    })?;
    catalog.mutate_scope_links(|document| {
        let before = document.links.len();
        document.links.retain(|link| &link.link_id != link_id);
        Ok(document.links.len() != before)
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_push_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePushPlan, String> {
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let storage_identity = storage_identity_for_config(storage)?;
    let storage_is_local = mallard_application::storage_setup::storage_local_root(
        mallard_infrastructure::storage_providers::StorageProviderRegistry::production(),
        storage,
    )?
    .is_some();
    let remote = workspace.remote_map();
    let plan = build_scope_push_plan(ScopePushPlanInputs {
        scope: &workspace.scope,
        capability: &workspace.capability,
        link,
        storage_is_local,
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
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
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
        registry,
        state,
        catalog,
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
    let storage_identity = storage_identity_for_config(storage)?;
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
        .verify_pins(&plan.profile_pins, &plan.target_pins)?;

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
    let mut payloads: BTreeMap<String, Bytes> = BTreeMap::new();
    for (resource_id, record) in &selected_records {
        let unchanged = resources
            .get(*resource_id)
            .is_some_and(|entry| entry.resource.content_sha256 == record.resource.content_sha256);
        if !unchanged {
            resources.insert(
                (*resource_id).clone(),
                PublishedResourceEntry {
                    resource: record.resource.clone(),
                    updated_at: now,
                },
            );
        }
        if let Some(captured) = &record.captured {
            payloads.extend(captured.payloads());
        }
    }

    let engine = engine_for(storage)?;
    let repository_id = link.repository_id.clone();
    let tag = match engine.read_tag(&repository_id)? {
        Some((tag, _)) => tag,
        None => LibraryResourceTag::new(
            &workspace.scope.coordinate(),
            repository_id.clone(),
            link.created_at,
        ),
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
                entry.resource.content_sha256.clone(),
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
            format!("Published version {}", published.head.revision)
        } else {
            "The linked storage already had this content".to_string()
        },
    })
}

/// Whether a copy name is already taken beside a target: by an inventoried
/// occupant of the same root, or by anything else sitting in that directory.
fn name_taken_index(
    inventory: &CatalogLocalInventory,
) -> impl Fn(&CatalogTargetConfig, &str) -> bool + '_ {
    move |target, candidate| {
        let folded = candidate.to_lowercase();
        let Ok(identity) = target.decode() else {
            return true;
        };
        let inventoried = inventory.occupants().any(|record| {
            record.target.decode().is_ok_and(|occupant| {
                occupant.canonical_root == identity.canonical_root
                    && occupant.member.to_lowercase() == folded
            })
        });
        if inventoried {
            return true;
        }
        match fs::read_dir(&identity.canonical_root) {
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
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    requested_copy_names: BTreeMap<CatalogResourceId, String>,
    user_home: &Path,
    now: u64,
) -> Result<CatalogScopePullPlan, String> {
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
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
        .ok_or("the storage has nothing of this coordinate to Pull")?;
    let storage_identity = storage_identity_for_config(storage)?;
    let remote = workspace.remote_map();
    let claims = catalog.load_materializations()?.claims;
    let name_taken = name_taken_index(&workspace.inventory);
    let plan = build_scope_pull_plan(ScopePullPlanInputs {
        scope: &workspace.scope,
        capability: &workspace.capability,
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
    registry: &CatalogRegistry,
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
        registry,
        state,
        catalog,
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
    if storage_identity_for_config(storage)? != plan.storage_identity_sha256 {
        return Err("the storage changed after review".to_string());
    }
    workspace
        .context
        .verify_pins(&plan.profile_pins, &plan.target_pins)?;
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

    let manifest_resources: BTreeMap<CatalogResourceId, CatalogResourceEnvelope> = plan
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
        registry,
        capability: workspace.capability.clone(),
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
        let acted =
            selection.action.is_some() || result.outcome == CatalogActionOutcome::AlreadyMatching;
        if pull_outcome_accepts_remote(result.outcome) {
            if acted {
                if let Some(resource) = &selection.item.resource {
                    accepted_updates.push((
                        selection.item.item.resource_id.clone(),
                        resource.content_sha256.clone(),
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

#[allow(clippy::too_many_arguments)]
fn prepare_fork_with_repositories(
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    resource_id: &CatalogResourceId,
    requested_name: &str,
    user_home: &Path,
    now: u64,
) -> Result<CatalogResourceForkPlan, String> {
    let ResolvedScope { scope, .. } = resolve_scope(catalog, scope_id)?;
    let capability = require_capability(registry, &scope)?;
    let context = resolve_profile_context(
        registry,
        &catalog.load_profiles()?,
        &scope.profile_id,
        &scope.provider,
        user_home,
    )
    .map_err(|blocker| blocker.to_string())?;
    let inventory =
        inventory_local(registry, &capability, &context).map_err(|blocker| blocker.to_string())?;
    let name_taken = name_taken_index(&inventory);
    let plan = build_resource_fork_plan(ScopeForkPlanInputs {
        scope: &scope,
        capability: &capability,
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
    use super::super::adapters::CmdOutput;
    use super::super::domain::{
        CatalogResourceRelation, CatalogResourceRepositoryId, CATALOG_SCHEMA,
    };
    use super::super::registry::test_support;
    use super::*;
    use mallard_domain::SyncConfig;
    use mallard_infrastructure::storage_providers::local::local_storage_config;

    /// Any CLI invocation fails these file-based Library flows.
    struct PanicRunner;

    impl PluginProcessRunner for PanicRunner {
        fn run(
            &self,
            _provider: &ProviderKey,
            _home: &Path,
            argv: &[String],
        ) -> Result<CmdOutput, String> {
            panic!("provider command ran unexpectedly: {}", argv.join(" "));
        }
    }

    fn storage(value: &str) -> StorageId {
        StorageId::parse(value).unwrap()
    }

    fn provider(key: &str) -> ProviderKey {
        ProviderKey::parse(key).unwrap()
    }

    fn coordinate(provider: &str, kind: &str, format: &str, version: u32) -> CatalogCoordinate {
        CatalogCoordinate::new(provider, kind, format, version).unwrap()
    }

    fn codex_skills() -> CatalogCoordinate {
        coordinate("codex", "agent_skill", "agentskills.io.skill", 1)
    }

    fn codex_pets(version: u32) -> CatalogCoordinate {
        coordinate("codex", "desktop_pet", "openai.codex-pet", version)
    }

    fn registry() -> Arc<CatalogRegistry> {
        CatalogRegistry::production()
    }

    fn create_scope(
        fixture_registry: &CatalogRegistry,
        catalog: &CatalogStateRepository,
        coordinate: CatalogCoordinate,
        profile_id: &LocalCatalogProfileId,
        now: u64,
    ) -> Result<CatalogScopeDto, String> {
        create_catalog_scope_with_repositories(
            fixture_registry,
            catalog,
            coordinate,
            profile_id.clone(),
            now,
        )
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

        let service =
            StorageCatalogService::from_home_dirs(&machine_home, &user_home, Box::new(PanicRunner))
                .expect("construct isolated service");
        assert_eq!(service.capabilities().len(), 6);
        let profile = service
            .save_profile(provider("codex"), codex_home.to_string_lossy().to_string())
            .expect("save profile");
        assert_eq!(profile.display_name, "myconf1");
        assert_eq!(profile.provider_label, "Codex");
        let scope = service
            .create_scope(
                provider("codex"),
                profile.profile_id,
                codex_skills().resource_kind,
                codex_skills().format,
                1,
            )
            .expect("create scope");
        assert!(scope.available);
        assert_eq!(scope.label, "Skills");
        assert_eq!(scope.labels.format_label, "Agent Skill");
        let resources = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .expect("list resources");

        assert_eq!(resources.rows.len(), 1);
        assert_eq!(resources.rows[0].display_name, "mallard-alpha");
        assert_eq!(
            resources.rows[0].scope_label.as_deref(),
            Some("Provider home")
        );
        assert!(resources.rows[0].local_present);
        assert!(resources.link.is_none());
        assert!(!resources.rows[0].remote_present);
        assert_eq!(resources.labels.kind_label, "Skills");
    }

    #[test]
    fn isolated_service_rejects_relative_homes() {
        assert!(StorageCatalogService::from_home_dirs(
            "relative-machine",
            "/absolute-user",
            Box::new(PanicRunner),
        )
        .is_err());
        assert!(StorageCatalogService::from_home_dirs(
            "/absolute-machine",
            "relative-user",
            Box::new(PanicRunner),
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

    struct LibraryFixture {
        _temporary: tempfile::TempDir,
        registry: Arc<CatalogRegistry>,
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

        let registry = registry();
        let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
        let profile = save_catalog_resource_profile_with_repository(
            &registry,
            &catalog,
            provider("codex"),
            codex_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let storage = local_storage_config(
            storage("library-test"),
            "Library test storage",
            &storage_path,
        );
        let mut config = SyncConfig::default();
        config.storages.push(storage.clone());
        state.save_config(config).unwrap();

        LibraryFixture {
            _temporary: temporary,
            registry,
            user_home,
            skill_path,
            storage_path,
            state,
            catalog,
            profile,
            storage,
        }
    }

    fn write_plugin_fixture(home: &Path) {
        fs::write(home.join("config.toml"), "[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n[plugins.\"tools@acme\"]\nenabled=true\n").unwrap();
        let manifest = home.join("plugins/cache/acme/tools/1/.codex-plugin/plugin.json");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(manifest, r#"{"name":"tools","version":"1"}"#).unwrap();
    }

    struct LocalPluginInstaller {
        write_state: bool,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl PluginProcessRunner for LocalPluginInstaller {
        fn run(
            &self,
            provider: &ProviderKey,
            home: &Path,
            argv: &[String],
        ) -> Result<CmdOutput, String> {
            assert_eq!(provider.as_str(), "codex");
            assert_eq!(argv, &["plugin", "add", "tools@acme"]);
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.write_state {
                write_plugin_fixture(home);
            }
            Ok(CmdOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn plugin_file_flows_need_no_cli_and_install_success_requires_local_state() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for write_state in [false, true] {
            let fixture = library_fixture();
            let home = Path::new(&fixture.profile.configured_home);
            write_plugin_fixture(home);
            let service = StorageCatalogService::from_home_dirs(
                &fixture.user_home,
                &fixture.user_home,
                Box::new(PanicRunner),
            )
            .unwrap();
            let scope = service
                .create_scope(
                    provider("codex"),
                    fixture.profile.profile_id.clone(),
                    CatalogResourceKindKey::parse("plugin_install_intent").unwrap(),
                    CatalogFormatKey::parse("mallard.plugin-install-intent").unwrap(),
                    1,
                )
                .unwrap()
                .scope
                .scope_id;
            let page = service.list_scope_resources(&scope, None).unwrap();
            assert_eq!(page.rows.len(), 1);
            assert!(page.rows[0].local_present);
            let resource_id = page.rows[0].resource_id.clone().unwrap();
            // Plugin intent has no directory to fork; refusal also needs no CLI.
            assert!(service.prepare_fork(&scope, &resource_id, "copy").is_err());
            service
                .link_scope_storage(&scope, &fixture.storage.id)
                .unwrap();
            let push = service.prepare_push(&scope, &fixture.storage.id).unwrap();
            assert_eq!(service.catalog.load_profiles().unwrap().schema, 1);
            assert!(
                service
                    .apply_push(CatalogPushApproval {
                        plan_id: push.plan_id,
                        selected_resource_ids: vec![resource_id],
                        acknowledged_warning_digests: push.warning_digests,
                    })
                    .unwrap()
                    .published
            );
            let noop = service
                .prepare_pull(&scope, &fixture.storage.id, BTreeMap::new())
                .unwrap();
            assert!(
                service
                    .apply_pull(CatalogPullApproval {
                        plan_id: noop.plan_id,
                        choices: BTreeMap::new(),
                        destructive_acknowledged: Vec::new()
                    })
                    .unwrap()
                    .remote_accepted
            );

            let target = sibling_machine(&fixture, "target-user", b"---\nname: review\n---\n");
            let target_home = Path::new(&target.profile.configured_home);
            fs::write(target_home.join("config.toml"), "[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n").unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let target_service = StorageCatalogService::from_home_dirs(
                &target.user_home,
                &target.user_home,
                Box::new(LocalPluginInstaller {
                    write_state,
                    calls: calls.clone(),
                }),
            )
            .unwrap();
            let target_scope = target_service
                .create_scope(
                    provider("codex"),
                    target.profile.profile_id.clone(),
                    CatalogResourceKindKey::parse("plugin_install_intent").unwrap(),
                    CatalogFormatKey::parse("mallard.plugin-install-intent").unwrap(),
                    1,
                )
                .unwrap()
                .scope
                .scope_id;
            target_service
                .link_scope_storage(&target_scope, &fixture.storage.id)
                .unwrap();
            let pull = target_service
                .prepare_pull(&target_scope, &fixture.storage.id, BTreeMap::new())
                .unwrap();
            let item = &pull.items[0].item;
            let choices = BTreeMap::from([(
                item.resource_id.clone(),
                item.default_action_id.clone().unwrap(),
            )]);
            assert_eq!(
                calls.load(Ordering::Relaxed),
                0,
                "listing and review must not execute a CLI"
            );
            let outcome = target_service
                .apply_pull(CatalogPullApproval {
                    plan_id: pull.plan_id,
                    choices,
                    destructive_acknowledged: Vec::new(),
                })
                .unwrap();
            assert_eq!(
                calls.load(Ordering::Relaxed),
                1,
                "only the install command can execute"
            );
            assert_eq!(
                outcome.results[0].outcome,
                if write_state {
                    CatalogActionOutcome::Installed
                } else {
                    CatalogActionOutcome::Partial
                }
            );
            assert_eq!(outcome.remote_accepted, write_state);
            assert_eq!(
                target_service
                    .list_scope_resources(&target_scope, Some(&fixture.storage.id))
                    .unwrap()
                    .rows[0]
                    .local_present,
                write_state
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                1,
                "post-install listing must not execute a CLI"
            );
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
            list_catalog_resource_profiles_with_repository(&registry(), &catalog, &user_home, 7)
                .unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].provider.as_str(), "codex");
        assert_eq!(first[0].display_name, "Default Codex");
        assert_eq!(first[0].configured_home, codex_home.to_string_lossy());
        assert_eq!(first[0].provider_config, CatalogProviderConfig::version_1());
        assert_eq!(first[1].provider.as_str(), "claude");
        assert_eq!(first[1].display_name, "Default Claude");
        assert_eq!(first[1].configured_home, claude_home.to_string_lossy());

        let revision = catalog.load_profiles().unwrap().revision;
        let second =
            list_catalog_resource_profiles_with_repository(&registry(), &catalog, &user_home, 9)
                .unwrap();
        assert_eq!(second, first);
        assert_eq!(catalog.load_profiles().unwrap().revision, revision);

        // The profile DTO carries the registered label and never the
        // provider's configuration envelope.
        let dto = profile_dto(&registry(), &first[0]);
        assert_eq!(dto.provider_label, "Codex");
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("provider_config").is_none());
        assert_eq!(json["provider"], "codex");
    }

    fn second_storage(fixture: &LibraryFixture) -> StorageConfig {
        let path = fixture.user_home.join("second-storage");
        fs::create_dir_all(&path).unwrap();
        let storage =
            local_storage_config(storage("library-second"), "Second library storage", &path);
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
        create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_skills(),
            &fixture.profile.profile_id,
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
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            scope_id,
            storage_id,
            &fixture.user_home,
            now,
        )
        .unwrap()
    }

    fn prepare_push(
        fixture: &LibraryFixture,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> CatalogScopePushPlan {
        prepare_push_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            scope_id,
            storage_id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap()
    }

    fn apply_push(
        fixture: &LibraryFixture,
        plan: &CatalogScopePushPlan,
        selected: Vec<CatalogResourceId>,
    ) -> Result<CatalogScopePushOutcomeDto, String> {
        apply_push_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            plan.plan_id.clone(),
            selected,
            plan.warning_digests.clone(),
            &fixture.user_home,
            now_secs(),
        )
    }

    /// Push everything selectable through the production prepare/apply pair.
    fn push_all(fixture: &LibraryFixture, scope_id: &CatalogScopeId, storage_id: &StorageId) {
        let plan = prepare_push(fixture, scope_id, storage_id);
        let selected: Vec<CatalogResourceId> = plan
            .rows
            .iter()
            .filter(|row| row.row.selectable)
            .map(|row| row.row.resource_id.clone())
            .collect();
        let outcome = apply_push(fixture, &plan, selected).unwrap();
        assert!(outcome.published, "{}", outcome.message);
    }

    fn page(
        fixture: &LibraryFixture,
        scope_id: &CatalogScopeId,
        storage_id: Option<&StorageId>,
    ) -> Result<CatalogScopeResourcesDto, String> {
        list_catalog_scope_resources_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            scope_id,
            storage_id,
            &fixture.user_home,
        )
    }

    fn resource_id_of(plan: &CatalogScopePushPlan, name: &str) -> CatalogResourceId {
        plan.rows
            .iter()
            .find(|row| row.row.display_name == name)
            .unwrap_or_else(|| panic!("no row named '{name}'"))
            .row
            .resource_id
            .clone()
    }

    #[test]
    fn profile_launches_resolve_the_home_and_fail_closed_when_it_vanishes() {
        let fixture = library_fixture();
        let missing = LocalCatalogProfileId::generate().unwrap();
        let unknown = profile_launch_home(&fixture.catalog, &missing).unwrap_err();
        assert!(unknown.contains("unknown catalog profile"), "{unknown}");

        let (launch_provider, home) =
            profile_launch_home(&fixture.catalog, &fixture.profile.profile_id).unwrap();
        assert_eq!(launch_provider.as_str(), "codex");
        assert_eq!(home.to_string_lossy(), fixture.profile.configured_home);

        fs::remove_dir_all(&home).unwrap();
        let gone = profile_launch_home(&fixture.catalog, &fixture.profile.profile_id).unwrap_err();
        assert!(gone.contains("home folder is missing"), "{gone}");
    }

    #[test]
    fn creating_a_scope_is_metadata_only_and_unique_per_profile_and_coordinate() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        assert_eq!(row.scope.coordinate(), codex_skills());
        assert_eq!(row.profile.profile_id, fixture.profile.profile_id);
        assert_eq!(row.labels.provider_label, "Codex");

        let duplicate = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_skills(),
            &fixture.profile.profile_id,
            3,
        )
        .unwrap_err();
        assert_eq!(duplicate, "this Catalog row already exists");

        // Pet v1 and v2 are distinct coordinates and therefore distinct rows.
        for version in [1, 2] {
            let pets = create_scope(
                &fixture.registry,
                &fixture.catalog,
                codex_pets(version),
                &fixture.profile.profile_id,
                4,
            )
            .unwrap();
            assert_eq!(pets.label, format!("Pets v{version}"));
        }
        assert_eq!(fixture.catalog.load_scopes().unwrap().scopes.len(), 3);

        // An invalid provider/kind/format combination is refused by the
        // registry, as is a coordinate of an unregistered provider, and a
        // profile of another provider.
        let rejected = create_scope(
            &fixture.registry,
            &fixture.catalog,
            coordinate("claude", "desktop_pet", "openai.codex-pet", 1),
            &fixture.profile.profile_id,
            5,
        )
        .unwrap_err();
        assert!(
            rejected.contains("is not supported by this build"),
            "{rejected}"
        );
        let unknown = create_scope(
            &fixture.registry,
            &fixture.catalog,
            coordinate("cursor", "agent_skill", "cursor.skill", 1),
            &fixture.profile.profile_id,
            5,
        )
        .unwrap_err();
        assert!(
            unknown.contains("no Library provider is registered"),
            "{unknown}"
        );
        let foreign_profile = create_scope(
            &fixture.registry,
            &fixture.catalog,
            coordinate("claude", "agent_skill", "agentskills.io.skill", 1),
            &fixture.profile.profile_id,
            5,
        )
        .unwrap_err();
        assert!(
            foreign_profile.contains("is a codex profile, not claude"),
            "{foreign_profile}"
        );
        assert_eq!(fixture.catalog.load_scopes().unwrap().scopes.len(), 3);
    }

    #[test]
    fn an_unregistered_scope_coordinate_loads_lists_unavailable_and_stays_inert() {
        let fixture = library_fixture();
        let foreign = coordinate("cursor", "agent_skill", "cursor.skill", 1);
        // Written past the registry, as a newer build would have: a profile
        // of a provider this build does not register, and a scope under it.
        // Both records are structurally valid and load beside the known
        // ones.
        let foreign_profile = fixture
            .catalog
            .mutate_profiles(|document| {
                let profile = LocalCatalogProfile {
                    profile_id: LocalCatalogProfileId::generate().unwrap(),
                    provider: foreign.provider.clone(),
                    display_name: "Cursor".to_string(),
                    configured_home: fixture.profile.configured_home.clone(),
                    provider_config: CatalogProviderConfig::version_1(),
                    revision: 1,
                    created_at: 1,
                    updated_at: 1,
                };
                document.profiles.push(profile.clone());
                Ok(profile)
            })
            .unwrap();
        let scope = fixture
            .catalog
            .mutate_scopes(|document| {
                let scope = CatalogScope {
                    scope_id: CatalogScopeId::generate().unwrap(),
                    provider: foreign.provider.clone(),
                    profile_id: foreign_profile.profile_id.clone(),
                    resource_kind: foreign.resource_kind.clone(),
                    format: foreign.format.clone(),
                    format_version: foreign.format_version,
                    revision: 1,
                    created_at: 1,
                    updated_at: 1,
                };
                document.scopes.push(scope.clone());
                Ok(scope)
            })
            .unwrap();
        let listed = {
            let profiles = fixture.catalog.load_profiles().unwrap();
            let scopes = fixture.catalog.load_scopes().unwrap();
            assert_eq!(profiles.profiles.len(), 2);
            assert_eq!(scopes.scopes.len(), 1);
            scope_dto(
                &fixture.registry,
                scopes.scopes[0].clone(),
                profiles.profile(&foreign_profile.profile_id).unwrap(),
            )
        };
        assert!(!listed.available);
        assert_eq!(listed.label, "cursor/agent_skill/cursor.skill@1");
        assert_eq!(listed.labels.provider_label, "cursor");
        assert_eq!(listed.labels.kind_label, "agent_skill");
        assert_eq!(listed.labels.format_label, "cursor.skill@1");
        assert_eq!(listed.profile.provider_label, "cursor");
        assert!(listed
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("no Library provider is registered for 'cursor'"));
        let json = serde_json::to_value(&listed).unwrap();
        assert_eq!(json["scope"]["format"], "cursor.skill");
        assert_eq!(json["available"], false);
        // The known rows keep working beside the unavailable one.
        let known = skill_scope(&fixture);
        assert!(known.available);
        assert_eq!(
            page(&fixture, &known.scope.scope_id, None)
                .unwrap()
                .rows
                .len(),
            1
        );

        // Nothing operates on it: no page, no link, no Push, no Pull, no
        // Fork. The row cannot be selected or applied anywhere.
        let inert = page(&fixture, &scope.scope_id, None).unwrap_err();
        assert!(inert.contains("cannot be operated on"), "{inert}");
        assert!(link_catalog_scope_storage_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &scope.scope_id,
            &fixture.storage.id,
            &fixture.user_home,
            3,
        )
        .unwrap_err()
        .contains("cannot be operated on"));
        assert!(prepare_push_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &scope.scope_id,
            &fixture.storage.id,
            &fixture.user_home,
            now_secs(),
        )
        .unwrap_err()
        .contains("cannot be operated on"));
        assert!(prepare_fork_with_repositories(
            &fixture.registry,
            &fixture.catalog,
            &scope.scope_id,
            &CatalogResourceId::parse(format!("resource-{}", "a".repeat(64))).unwrap(),
            "copy",
            &fixture.user_home,
            now_secs(),
        )
        .unwrap_err()
        .contains("cannot be operated on"));
        assert!(fixture.catalog.load_scope_links().unwrap().links.is_empty());
        // The records stay exactly as loaded.
        assert_eq!(
            fixture.catalog.load_scopes().unwrap().scopes[0].coordinate(),
            foreign
        );
        assert_eq!(
            fixture
                .catalog
                .load_profiles()
                .unwrap()
                .profile(&foreign_profile.profile_id)
                .unwrap()
                .provider,
            foreign.provider
        );
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
        let page = page(&fixture, &row.scope.scope_id, None).unwrap();
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
        assert_eq!(page.rows[0].local_files, Some(1));
    }

    #[test]
    fn a_scope_page_pairs_local_and_stored_resources_into_one_row() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);

        // Nothing linked yet: the local skill is still a row, and it is the
        // storage's problem that it has never seen it.
        let before = page(&fixture, &row.scope.scope_id, None).unwrap();
        assert_eq!(before.rows.len(), 1);
        assert!(before.link.is_none());
        assert_eq!(before.rows[0].state, Some(CatalogLinkState::New));

        // Link and push, then read the page again: the same one row, now
        // paired with the stored copy and in sync.
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let after = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap();
        assert_eq!(after.rows.len(), 1);
        assert!(after.rows[0].local_present && after.rows[0].remote_present);
        assert_eq!(after.rows[0].state, Some(CatalogLinkState::InSync));
        assert_eq!(after.storage_revision, Some(1));
        assert!(after.rows[0].remote_updated_at.is_some());

        // A second storage has never held it, so the same resource reads as
        // New there — the page's state belongs to the pairing, not the row.
        let second = second_storage(&fixture);
        let elsewhere = page(&fixture, &row.scope.scope_id, Some(&second.id)).unwrap();
        assert_eq!(elsewhere.rows.len(), 1);
        assert!(elsewhere.link.is_none());
        assert_eq!(elsewhere.rows[0].state, Some(CatalogLinkState::New));
    }

    /// A second machine, built beside the fixture's home and pointed at the
    /// same storage. Its skill carries whatever body the test wants to
    /// compare against the first machine's push.
    fn sibling_machine(fixture: &LibraryFixture, name: &str, skill_body: &[u8]) -> LibraryFixture {
        let user_home = fixture.user_home.parent().unwrap().join(name);
        let codex_home = user_home.join("codex-home");
        let skill_path = codex_home.join("skills/review");
        fs::create_dir_all(&skill_path).unwrap();
        fs::write(skill_path.join("SKILL.md"), skill_body).unwrap();

        let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
        let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
        let profile = save_catalog_resource_profile_with_repository(
            &fixture.registry,
            &catalog,
            provider("codex"),
            codex_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let mut config = SyncConfig::default();
        config.storages.push(fixture.storage.clone());
        state.save_config(config).unwrap();
        LibraryFixture {
            _temporary: tempfile::tempdir().unwrap(),
            registry: fixture.registry.clone(),
            user_home,
            skill_path,
            storage_path: fixture.storage_path.clone(),
            state,
            catalog,
            profile,
            storage: fixture.storage.clone(),
        }
    }

    /// The convergence rule end to end: every replica of one coordinate
    /// lands on the same repository, and the manifest is the union of what
    /// they push.
    #[test]
    fn replicas_of_one_coordinate_converge_on_one_repository() {
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
        let twin = sibling_machine(
            &fixture,
            "twin-user",
            b"---\nname: review\n---\nDraft-linked skill.\n",
        );
        let third_skill = twin.user_home.join("codex-home/skills/extra");
        fs::create_dir_all(&third_skill).unwrap();
        fs::write(
            third_skill.join("SKILL.md"),
            b"---\nname: extra\n---\nThird skill.\n",
        )
        .unwrap();
        let twin_scope = skill_scope(&twin);
        let linked = link_scope(&twin, &twin_scope.scope.scope_id, &fixture.storage.id, 5);
        assert_eq!(linked.completion, "review_pull");
        let twin_page = page(&twin, &twin_scope.scope.scope_id, Some(&fixture.storage.id)).unwrap();
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
        assert_eq!(state_of("notes"), Some(CatalogLinkState::LocalMissing));
        assert_eq!(state_of("extra"), Some(CatalogLinkState::New));

        // B pushes its own skill: one repository, a manifest of three.
        push_all(&twin, &twin_scope.scope.scope_id, &fixture.storage.id);

        let library = fixture.storage_path.join(".mallard/v1/library");
        assert_eq!(fs::read_dir(&library).unwrap().count(), 1);
        let engine = engine_for(&fixture.storage).unwrap();
        let repository_id = CatalogResourceRepositoryId::for_coordinate(&codex_skills());
        let (head, manifest) = engine.inspect(&repository_id).unwrap().unwrap();
        assert_eq!(head.head.revision, 2);
        assert_eq!(manifest.resources.len(), 3);

        // A pulls B's skill through one review.
        let pull = prepare_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
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
        assert_eq!(extra.item.labels.kind_label, "Skills");
        let choices = BTreeMap::from([(
            extra.item.resource_id.clone(),
            extra.item.default_action_id.clone().unwrap(),
        )]);
        let outcome = apply_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
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
        let page = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap();
        assert!(page
            .rows
            .iter()
            .all(|row| row.state == Some(CatalogLinkState::InSync)));
    }

    #[test]
    fn repository_ids_depend_on_the_coordinate_alone() {
        let fixture = library_fixture();
        // Two profiles of one provider with different homes (and therefore
        // different target roots), two storages: one repository id.
        let other_home = fixture.user_home.join("codex-other");
        fs::create_dir_all(other_home.join("skills")).unwrap();
        let other_profile = save_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            other_home.to_string_lossy().to_string(),
            1,
        )
        .unwrap();
        let first = skill_scope(&fixture);
        let second = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_skills(),
            &other_profile.profile_id,
            2,
        )
        .unwrap();
        let second_storage = second_storage(&fixture);
        link_scope(&fixture, &first.scope.scope_id, &fixture.storage.id, 3);
        link_scope(&fixture, &second.scope.scope_id, &second_storage.id, 4);
        let links = fixture.catalog.load_scope_links().unwrap();
        let ids: BTreeSet<_> = links
            .links
            .iter()
            .map(|link| link.repository_id.clone())
            .collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(
            ids.into_iter().next().unwrap(),
            CatalogResourceRepositoryId::for_coordinate(&codex_skills())
        );
        // Another version of one format is another repository.
        assert_ne!(
            CatalogResourceRepositoryId::for_coordinate(&codex_pets(1)),
            CatalogResourceRepositoryId::for_coordinate(&codex_pets(2))
        );
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

        let plan = prepare_push(&fixture, &row.scope.scope_id, &fixture.storage.id);
        assert_eq!(plan.rows.len(), 2);
        let review_id = resource_id_of(&plan, "review");
        let outcome = apply_push(&fixture, &plan, vec![review_id.clone()]).unwrap();
        assert!(outcome.published);

        let engine = engine_for(&fixture.storage).unwrap();
        let repository_id = CatalogResourceRepositoryId::for_coordinate(&codex_skills());
        let (_, manifest) = engine.inspect(&repository_id).unwrap().unwrap();
        assert_eq!(manifest.resources.len(), 1);
        assert!(manifest.resources.contains_key(&review_id));

        // The selected resource remains visible as InSync while the
        // unselected resource still reads as New on the same page.
        let page = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap();
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
        let next_plan = prepare_push(&fixture, &row.scope.scope_id, &fixture.storage.id);
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
        // Review rows carry the coordinate and its labels, never a target.
        let json = serde_json::to_value(&next_plan).unwrap();
        assert_eq!(json["rows"][0]["row"]["format"], "agentskills.io.skill");
        assert_eq!(json["rows"][0]["row"]["provider_label"], "Codex");
        assert!(json.get("target_pins").is_none());
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
            .replace("agent_skill", "desktop_pet");
        fs::write(&tag_path, tag_text).unwrap();

        let page = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap_err();
        assert!(
            page.contains("does not match its coordinate derivation"),
            "{page}"
        );
    }

    #[test]
    fn the_storage_listing_derives_repositories_and_never_scans() {
        let fixture = library_fixture();
        // An empty storage: no derivable id answers, so nothing is listed.
        assert!(list_catalog_storage_repositories_with_repository(
            &fixture.registry,
            &fixture.state,
            &fixture.storage.id,
        )
        .unwrap()
        .is_empty());

        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);

        let listed = list_catalog_storage_repositories_with_repository(
            &fixture.registry,
            &fixture.state,
            &fixture.storage.id,
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        let repo = &listed[0];
        assert_eq!(repo.coordinate, codex_skills());
        assert_eq!(repo.labels.kind_label, "Skills");
        assert_eq!(
            repo.repository_id,
            CatalogResourceRepositoryId::for_coordinate(&codex_skills()).as_str()
        );
        assert_eq!(repo.revision, Some(1));
        assert!(repo.updated_at.is_some());
        assert_eq!(repo.resource_names, vec!["review".to_string()]);
        assert!(repo.error.is_none());
        let json = serde_json::to_value(repo).unwrap();
        assert_eq!(json["format_version"], 1);
        assert_eq!(json["provider"], "codex");
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
            .replace("agent_skill", "desktop_pet");
        fs::write(&tag_path, tag_text).unwrap();

        let listed = list_catalog_storage_repositories_with_repository(
            &fixture.registry,
            &fixture.state,
            &fixture.storage.id,
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        let repo = &listed[0];
        assert_eq!(repo.coordinate, codex_skills());
        assert!(repo.revision.is_none());
        assert!(repo.resource_names.is_empty());
        let error = repo.error.as_deref().unwrap();
        assert!(
            error.contains("does not match its coordinate derivation"),
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
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            first_home.to_string_lossy().to_string(),
            10,
        )
        .unwrap();
        assert_eq!(first.display_name, "myconf");

        // Same folder name somewhere else: the derived name steps aside.
        let second = save_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            second_home.to_string_lossy().to_string(),
            11,
        )
        .unwrap();
        assert_eq!(second.display_name, "myconf 2");

        // Re-picking a saved home selects it rather than failing or forking.
        let again = save_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
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

        // A provider this build does not register cannot get a profile.
        let unknown = save_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("cursor"),
            first_home.to_string_lossy().to_string(),
            13,
        )
        .unwrap_err();
        assert!(
            unknown.contains("no Library provider is registered"),
            "{unknown}"
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
            &fixture.registry,
            &fixture.catalog,
            &linked.link_id,
            linked.revision + 5,
            &fixture.user_home,
        )
        .unwrap_err();
        assert!(stale.contains("refresh before unlinking"), "{stale}");

        assert!(unlink_catalog_scope_storage_with_repositories(
            &fixture.registry,
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
        let plan = prepare_push(&fixture, &row.scope.scope_id, &fixture.storage.id);

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

        let review_id = resource_id_of(&plan, "review");
        let outcome = apply_push(&fixture, &plan, vec![review_id]).unwrap();
        assert!(outcome.published, "{}", outcome.message);
    }

    #[test]
    fn a_pets_scope_lists_without_provider_clis() {
        let fixture = library_fixture();
        let codex_home = fixture.user_home.join("codex-home");
        let atlas = super::super::codex_pet::fixtures::webp_atlas(1);
        fs::create_dir_all(codex_home.join("pets/duck")).unwrap();
        fs::write(
            codex_home.join("pets/duck/pet.json"),
            br#"{"id":"duck","displayName":"Duck","description":"A duck.","spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();
        fs::write(codex_home.join("pets/duck/spritesheet.webp"), &atlas).unwrap();

        let row = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(1),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap();
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
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
        assert_eq!(page.rows[0].description.as_deref(), Some("A duck."));

        // The sibling v2 scope shares the pets directory but not the row: a
        // v1 pet never renders inside the v2 scope's page.
        let sibling = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(2),
            &fixture.profile.profile_id,
            3,
        )
        .unwrap();
        let sibling_page = list_catalog_scope_resources_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &sibling.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert!(sibling_page.rows.is_empty());
    }

    #[test]
    fn a_pet_scope_lists_with_a_preview_derived_from_its_atlas() {
        let fixture = library_fixture();
        let codex_home = fixture.user_home.join("codex-home");
        let atlas = super::super::codex_pet::fixtures::decodable_png_atlas(1);
        fs::create_dir_all(codex_home.join("pets/drake")).unwrap();
        fs::write(
            codex_home.join("pets/drake/pet.json"),
            br#"{"id":"drake","displayName":"Drake","description":"A local pet.","spritesheetPath":"spritesheet.png"}"#,
        )
        .unwrap();
        fs::write(codex_home.join("pets/drake/spritesheet.png"), &atlas).unwrap();

        let row = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(1),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap();
        let page = list_catalog_scope_resources_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            None,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(page.rows.len(), 1);
        let listed = &page.rows[0];
        assert_eq!(listed.description.as_deref(), Some("A local pet."));
        let preview = listed.preview_png.as_deref().unwrap();
        assert!(preview.starts_with("data:image/png;base64,"), "{}", preview);
        // Derived once, cached by the atlas digest under `~/.mallard`.
        let cache = preview_cache_dir(&fixture.user_home);
        let digest = super::super::domain::sha256_hex(&atlas);
        assert!(cache.join(format!("{}.png", digest)).is_file());
    }

    #[test]
    fn a_stored_only_pet_serves_its_preview_from_the_storage_then_the_cache() {
        use super::super::codec::CatalogFormatCodec;
        use super::super::codex_pet::CodexPetCodec;
        use super::super::domain::PortableCatalogOrigin;

        let fixture = library_fixture();
        // Capture a v2 pet from a scratch directory that is not the profile's
        // home, so the resource exists only in the storage.
        let scratch = fixture.user_home.join("scratch/biscuit");
        fs::create_dir_all(&scratch).unwrap();
        let atlas = super::super::codex_pet::fixtures::decodable_png_atlas(2);
        fs::write(
            scratch.join("pet.json"),
            super::super::codex_pet::fixtures::manifest(
                2,
                "biscuit",
                "Biscuit",
                "A stored-only pet.",
                "spritesheet.png",
            ),
        )
        .unwrap();
        fs::write(scratch.join("spritesheet.png"), &atlas).unwrap();
        let captured = CodexPetCodec::for_version(2)
            .unwrap()
            .capture_directory(&scratch, "provider-home")
            .unwrap();
        let resource = captured.to_envelope(&codex_pets(2));
        let resource_id = resource.resource_id.clone();
        let atlas_sha = super::super::domain::sha256_hex(&atlas);

        let repository_id = CatalogResourceRepositoryId::for_coordinate(&codex_pets(2));
        let engine = engine_for(&fixture.storage).unwrap();
        engine
            .publish(PublishLibraryResourceRequest {
                repository_id: repository_id.clone(),
                tag: LibraryResourceTag::new(&codex_pets(2), repository_id.clone(), 1),
                observed: None,
                resources: BTreeMap::from([(
                    resource_id.clone(),
                    PublishedResourceEntry {
                        resource,
                        updated_at: 1,
                    },
                )]),
                payloads: captured.payloads(),
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

        let row = create_scope(
            &fixture.registry,
            &fixture.catalog,
            codex_pets(2),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap();
        let uncached = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap();
        assert!(uncached.rows[0].preview_png.is_none());
        let first = get_catalog_resource_preview_with_repositories(
            &fixture.registry,
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
        let second = get_catalog_resource_preview_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            &fixture.storage.id,
            &resource_id,
            &fixture.user_home,
        )
        .unwrap();
        assert_eq!(second, first);

        // A refreshed scope page supplies the cached image from its verified
        // manifest. The UI then skips the per-card preview request entirely.
        let cached = page(&fixture, &row.scope.scope_id, Some(&fixture.storage.id)).unwrap();
        assert!(!cached.rows[0].local_present);
        assert!(cached.rows[0].remote_present);
        assert_eq!(cached.rows[0].preview_png.as_deref(), Some(first.as_str()));

        // A skills scope has no preview to serve.
        let skills = skill_scope(&fixture);
        link_scope(&fixture, &skills.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &skills.scope.scope_id, &fixture.storage.id);
        let plan = prepare_push(&fixture, &skills.scope.scope_id, &fixture.storage.id);
        let error = get_catalog_resource_preview_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &skills.scope.scope_id,
            &fixture.storage.id,
            &resource_id_of(&plan, "review"),
            &fixture.user_home,
        )
        .unwrap_err();
        assert_eq!(error, "this resource has no preview");
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
            &fixture.registry,
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

    /// The extension proof: a provider, kind, format, version, and target
    /// that exist only in the test registry travel through every common
    /// path — profile, scope, page, link, Push, Pull, storage listing —
    /// without any common-code change.
    #[test]
    fn the_test_capability_syncs_end_to_end_through_common_code() {
        let temporary = tempfile::tempdir().unwrap();
        let storage_path = temporary.path().join("storage");
        fs::create_dir_all(&storage_path).unwrap();
        let storage =
            local_storage_config(storage("notes-storage"), "Notes storage", &storage_path);
        let registry = CatalogRegistry::with_test_capability();
        let key = test_support::provider_key();
        let coordinate = test_support::coordinate();

        let replica = |name: &str| {
            let user_home = temporary.path().join(name);
            let home = user_home.join("testprov-home");
            fs::create_dir_all(&home).unwrap();
            let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
            let mut config = SyncConfig::default();
            config.storages.push(storage.clone());
            state.save_config(config).unwrap();
            let service = StorageCatalogService::with_registry(
                &user_home,
                &user_home,
                Box::new(PanicRunner),
                registry.clone(),
            )
            .unwrap();
            let profile = service
                .save_profile(key.clone(), home.to_string_lossy().to_string())
                .unwrap();
            assert_eq!(profile.provider_label, "Test Provider");
            let scope = service
                .create_scope(
                    key.clone(),
                    profile.profile_id,
                    coordinate.resource_kind.clone(),
                    coordinate.format.clone(),
                    coordinate.format_version,
                )
                .unwrap();
            assert!(scope.available);
            assert_eq!(scope.label, "Notes");
            (service, home, scope.scope.scope_id)
        };

        let (service_a, home_a, scope_a) = replica("a");
        fs::create_dir_all(home_a.join("notes/todo")).unwrap();
        fs::write(
            home_a.join("notes/todo/NOTE.md"),
            b"# Things to do\n- ship the registry\n",
        )
        .unwrap();
        let page = service_a.list_scope_resources(&scope_a, None).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].display_name, "Things to do");
        assert_eq!(page.labels.format_label, "Test note");
        assert_eq!(
            service_a
                .link_scope_storage(&scope_a, &storage.id)
                .unwrap()
                .completion,
            "open"
        );
        let push = service_a.prepare_push(&scope_a, &storage.id).unwrap();
        assert_eq!(push.rows.len(), 1);
        assert!(push.rows[0].row.selectable);
        assert_eq!(push.rows[0].row.coordinate, coordinate);
        let outcome = service_a
            .apply_push(CatalogPushApproval {
                plan_id: push.plan_id.clone(),
                selected_resource_ids: vec![push.rows[0].row.resource_id.clone()],
                acknowledged_warning_digests: push.warning_digests.clone(),
            })
            .unwrap();
        assert!(outcome.published);

        let (service_b, home_b, scope_b) = replica("b");
        assert_eq!(
            service_b
                .link_scope_storage(&scope_b, &storage.id)
                .unwrap()
                .completion,
            "review_pull"
        );
        let pull = service_b
            .prepare_pull(&scope_b, &storage.id, BTreeMap::new())
            .unwrap();
        assert_eq!(pull.items.len(), 1);
        let item = &pull.items[0];
        assert!(matches!(
            item.item.actions[0].kind,
            super::super::domain::CatalogPullActionKind::Install
        ));
        let outcome = service_b
            .apply_pull(CatalogPullApproval {
                plan_id: pull.plan_id.clone(),
                choices: BTreeMap::from([(
                    item.item.resource_id.clone(),
                    item.item.default_action_id.clone().unwrap(),
                )]),
                destructive_acknowledged: Vec::new(),
            })
            .unwrap();
        assert!(outcome.remote_accepted);
        assert_eq!(outcome.results[0].outcome, CatalogActionOutcome::Installed);
        assert_eq!(
            fs::read(home_b.join("notes/todo/NOTE.md")).unwrap(),
            b"# Things to do\n- ship the registry\n"
        );

        let listed = service_b.list_storage_repositories(&storage.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].coordinate, coordinate);
        assert_eq!(listed[0].labels.provider_label, "Test Provider");
        assert_eq!(listed[0].resource_names, vec!["Things to do".to_string()]);
        assert!(service_b
            .capabilities()
            .iter()
            .any(|descriptor| descriptor.coordinate() == coordinate));
        assert_eq!(CATALOG_SCHEMA, 1);
    }
}
