//! Non-Tauri application boundary for scope-rowed Library resources.
//!
//! Owns catalog scopes, per-storage links, and reviewed Push, Pull, and Fork
//! operations. It reads the neutral configured-storage list but never loads a
//! project registration, project binding, thread, or repository bundle.
//! Every provider-, kind-, or format-specific step dispatches through the
//! registry: a scope whose coordinate this build does not register lists as
//! unavailable and cannot be operated on.

pub mod cloud;
mod removal;
pub use removal::CatalogRemovalResult;

use bytes::Bytes;
use mallard_domain::{
    now_secs, sha256_hex, Mainline, RecordName, StorageConfig, StorageId, SyncConfig,
};
use mallard_infrastructure::bundle_engine::{NOT_PULLED, VERSION_INCOMPLETE};
use mallard_infrastructure::managed_cloud::client::{
    CoordinateQuery, ManagedCloudStatus, ManagedCloudStatusState,
};
use mallard_infrastructure::managed_cloud::names as cloud_names;
use mallard_infrastructure::managed_cloud::ManagedObjectStore;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_core::publication::{
    unfinished_push_sentence, RecordStream, REMOTE_CHANGED,
};
use mallard_infrastructure::storage_core::ConfiguredObjectStore;
use mallard_infrastructure::storage_core::ObjectStore;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::adapters::{
    NativeProcessRunner, PluginProcessRunner, ProviderInventory, ResolvedCatalogTarget,
};
use super::apply::{
    apply_resource_fork, execute_scope_pull_item, recover_catalog_operations, PullApplyEnvironment,
};
use super::codec::{CapturedResource, CatalogDetailPluginListing, CatalogResourceDetail};
use super::domain::{
    CatalogActionOutcome, CatalogActionResult, CatalogCapturedWith, CatalogCoordinate,
    CatalogCoordinateLabels, CatalogFormatKey, CatalogLinkId, CatalogLinkState,
    CatalogPlanActionId, CatalogPlanId, CatalogProviderConfig, CatalogPullItem, CatalogPullSource,
    CatalogPushDestination, CatalogResourceEnvelope, CatalogResourceForkPlan, CatalogResourceId,
    CatalogResourceKindKey, CatalogResourcePlanDocument, CatalogResourceRepositoryId,
    CatalogResourceWarning, CatalogScope, CatalogScopeId, CatalogScopeLink, CatalogScopePullPlan,
    CatalogScopePushPlan, CatalogTargetConfig, LibraryHeadCoordinate, LibraryResourceManifest,
    LibraryResourceTag, LocalCatalogProfile, LocalCatalogProfileId, PluginApplyMode, ProviderKey,
    PublishedResourceEntry,
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
    head_coordinate, parse_library_record, storage_identity_for_config, verify_library_continuity,
    DiscoveredLibraryRepository, LibraryContinuity, LibraryResourceEngine, ObservedLibraryHead,
    PublishLibraryResourceRequest, PublishedLibraryResource,
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
    Ok(LibraryResourceEngine::new(
        ConfiguredObjectStore::shared(storage)?.for_catalog_operation(),
    ))
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

/// What a typed home would become, answered without writing: the project's
/// probe shape in the Catalog's words.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogProfileProbeDto {
    pub canonical_path: String,
    pub readable: bool,
    /// The saved profile this folder already is, when it is one.
    pub existing_profile_id: Option<LocalCatalogProfileId>,
    /// The name Add would give it: the saved one, else its folder's.
    pub suggested_name: String,
}

/// What one saved home holds of one capability its provider registers: a
/// count, the newest stamp, the roots as the adapter names them, and the
/// row that already lists it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogProfileHoldingDto {
    pub capability: CatalogCapabilityDescriptor,
    /// Null when this capability's inventory could not run for the home.
    pub count: Option<u32>,
    pub newest_modified_at: Option<u64>,
    pub roots: Vec<String>,
    pub scope_id: Option<CatalogScopeId>,
}

/// One saved home read for its setup page: its holdings, in the registry's
/// order, and the storage a Catalog row used last.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CatalogProfileInspectionDto {
    pub profile: CatalogProfileDto,
    pub last_used_storage_id: Option<StorageId>,
    pub holdings: Vec<CatalogProfileHoldingDto>,
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
    /// On Mallard Cloud: each item's repository, saved or learned from the
    /// account's listing.
    pub cloud_repositories: BTreeMap<CatalogResourceId, String>,
    /// The repositories this row has saved; the rest were learned.
    pub cloud_connections: BTreeMap<CatalogResourceId, String>,
    /// On Mallard Cloud, when the page's one request was refused for the
    /// account's sake: the rows list only what is on this Mac, and Push and
    /// Pull stay closed.
    pub cloud_status: Option<ManagedCloudStatus>,
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
pub struct CatalogPushResult {
    pub resource_id: CatalogResourceId,
    pub display_name: String,
    pub state: String,
    pub message: Option<String>,
    /// On Mallard Cloud, where each item has its own repository: the
    /// version this item's repository holds after the Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CatalogSyncProgress {
    pub plan_id: CatalogPlanId,
    pub completed: usize,
    pub total: usize,
    pub message: String,
}
type ProgressReporter = dyn Fn(CatalogSyncProgress) + Send + Sync;

#[derive(Serialize, Clone, Debug)]
pub struct CatalogScopePushOutcomeDto {
    pub plan_id: CatalogPlanId,
    pub published: bool,
    pub results: Vec<CatalogPushResult>,
    pub stopped: Option<String>,
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
    /// Why the Pull stopped at a payload it could not read, when it did. The
    /// resources installed before it stay, with their claims; the rows it
    /// did not reach read "Not pulled"; the accepted version does not
    /// advance (PLAN_PULL_PINNED_VERSION.md, G3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

/// The refusal of a Library Pull whose reviewed version is no longer part of
/// the selected history: it lost its race, or the repository was cleared,
/// with or without a push since (PLAN_PULL_PINNED_VERSION.md, G2).
pub const LIBRARY_REVIEW_SUPERSEDED: &str = "The storage changed since this review. Review again.";

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
    progress: Arc<ProgressReporter>,
    user_home: PathBuf,
}

impl StorageCatalogService {
    pub fn with_progress(
        mut self,
        report: impl Fn(CatalogSyncProgress) + Send + Sync + 'static,
    ) -> Self {
        self.progress = Arc::new(report);
        self
    }

    pub fn from_user_home() -> Result<Self, String> {
        let user_home = mallard_infrastructure::persistence::user_home_dir()
            .ok_or_else(|| "could not resolve the user home directory".to_string())?;
        let registry = CatalogRegistry::production();
        let runner = Box::new(NativeProcessRunner::new(
            registry.clone(),
            user_home.clone(),
        ));
        Self::with_registry(user_home.clone(), user_home, runner, registry)
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
            progress: Arc::new(|_| {}),
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

    pub fn probe_profile(
        &self,
        provider: ProviderKey,
        path: String,
    ) -> Result<CatalogProfileProbeDto, String> {
        probe_catalog_resource_profile_with_repository(
            &self.registry,
            &self.catalog,
            provider,
            path,
        )
    }

    pub fn inspect_profile(
        &self,
        profile_id: &LocalCatalogProfileId,
    ) -> Result<CatalogProfileInspectionDto, String> {
        inspect_catalog_resource_profile_with_repository(
            &self.registry,
            &self.catalog,
            profile_id,
            &self.user_home,
        )
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
        let settings = mallard_infrastructure::app_settings::load_app_settings(&self.state)?;
        self.registry
            .require_provider(&provider)?
            .launch_home_terminal(&home, settings.cli_path(&provider))
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

    pub fn resource_detail(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: Option<&StorageId>,
        resource_id: &CatalogResourceId,
    ) -> Result<CatalogResourceDetail, String> {
        get_catalog_resource_detail_with_repositories(
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

    /// *Finish that push*: the record an earlier push claimed and never
    /// wrote is written once, and the result is reported. The user's verb.
    pub fn finish_unfinished_push(
        &self,
        scope_id: &CatalogScopeId,
        storage_id: &StorageId,
    ) -> Result<(), String> {
        finish_unfinished_push_with_repositories(
            &self.registry,
            &self.state,
            &self.catalog,
            scope_id,
            storage_id,
            &self.user_home,
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
            self.progress.as_ref(),
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
            self.progress.as_ref(),
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

/// What a typed home would become, answered without writing: the canonical
/// folder, whether it can be read, the saved profile it already is, and the
/// name Add would give it. Validation is the save's own, so what the hint
/// accepts is what Set up saves. Never scans a project folder.
fn probe_catalog_resource_profile_with_repository(
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    provider: ProviderKey,
    path: String,
) -> Result<CatalogProfileProbeDto, String> {
    let provider_label = registry.require_provider(&provider)?.descriptor().label;
    let canonical = validate_new_profile_home(catalog.root(), &path)?;
    if !canonical.exists() {
        return Err(format!(
            "inspect catalog profile '{}': No such file or directory",
            canonical.display()
        ));
    }
    if !canonical.is_dir() {
        return Err(format!(
            "inspect catalog profile '{}': not a directory",
            canonical.display()
        ));
    }
    let canonical_home = canonical.to_string_lossy().to_string();
    let profiles = catalog.load_profiles()?.profiles;
    let existing = profiles
        .iter()
        .find(|profile| profile.provider == provider && profile.configured_home == canonical_home);
    Ok(CatalogProfileProbeDto {
        readable: fs::read_dir(&canonical).is_ok(),
        existing_profile_id: existing.map(|profile| profile.profile_id.clone()),
        suggested_name: existing
            .map(|profile| profile.display_name.clone())
            .unwrap_or_else(|| derived_profile_name(&provider_label, &canonical, &profiles)),
        canonical_path: canonical_home,
    })
}

/// One saved home for its setup page: per capability its provider
/// registers, the row page's own inventory reduced to a count, the newest
/// stamp, the target summaries, and the row that already exists — plus the
/// storage a Catalog row used last. Nothing captured crosses IPC. A
/// capability whose inventory cannot run counts as unknown rather than
/// failing the page.
fn inspect_catalog_resource_profile_with_repository(
    registry: &CatalogRegistry,
    catalog: &CatalogStateRepository,
    profile_id: &LocalCatalogProfileId,
    user_home: &Path,
) -> Result<CatalogProfileInspectionDto, String> {
    let profiles = catalog.load_profiles()?;
    let profile = profiles
        .profile(profile_id)
        .ok_or_else(|| format!("unknown catalog profile '{}'", profile_id))?
        .clone();
    let scopes = catalog.load_scopes()?.scopes;
    let last_used_storage_id = catalog
        .load_scope_links()?
        .links
        .iter()
        .max_by_key(|link| (link.updated_at, link.created_at))
        .map(|link| link.storage_id.clone());
    let context = resolve_profile_context(
        registry,
        &profiles,
        profile_id,
        &profile.provider,
        user_home,
    )
    .ok();
    let mut holdings = Vec::new();
    for descriptor in registry.descriptors() {
        if descriptor.provider != profile.provider {
            continue;
        }
        let Some(capability) = registry.capability(&descriptor.coordinate()) else {
            continue;
        };
        let inventory = context
            .as_ref()
            .and_then(|context| inventory_local(registry, &capability, context).ok());
        holdings.push(CatalogProfileHoldingDto {
            count: inventory
                .as_ref()
                .map(|inventory| inventory.resources.len() as u32),
            newest_modified_at: inventory.as_ref().and_then(|inventory| {
                inventory
                    .resources
                    .values()
                    .filter_map(|record| record.local_modified_at)
                    .max()
            }),
            roots: context
                .as_ref()
                .map(|context| {
                    context
                        .targets_for(&capability)
                        .iter()
                        .map(|target| target.summary.clone())
                        .collect()
                })
                .unwrap_or_default(),
            scope_id: scopes
                .iter()
                .find(|scope| {
                    scope.profile_id == profile.profile_id
                        && scope.coordinate() == descriptor.coordinate()
                })
                .map(|scope| scope.scope_id.clone()),
            capability: descriptor,
        });
    }
    Ok(CatalogProfileInspectionDto {
        profile: profile_dto(registry, &profile),
        last_used_storage_id,
        holdings,
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
    engine: Option<LibraryResourceEngine<ConfiguredObjectStore>>,
    cloud_repositories: BTreeMap<CatalogResourceId, String>,
    scope: CatalogScope,
    profile: LocalCatalogProfile,
    capability: Arc<CatalogCapability>,
    context: ResolvedCatalogContext,
    inventory: CatalogLocalInventory,
    storage: Option<StorageConfig>,
    link: Option<CatalogScopeLink>,
    /// The saved accepted state names a history the repository no longer
    /// has: `link` carries it as absent, nothing was written, and the next
    /// Push or Pull drops the saved acceptances and saves its own.
    stale_acceptance: bool,
    /// The same, per item, on a storage that holds one item per repository.
    stale_items: BTreeSet<CatalogResourceId>,
    /// The selected chain of the one listing this workspace made, for a Pull
    /// apply to look its reviewed version up on.
    mainline: Option<Mainline>,
    observed: Option<ObservedLibraryHead>,
    manifest: Option<LibraryResourceManifest>,
    /// One item per repository (Mallard Cloud): each stored item's stream,
    /// selection, selected record and manifest, from the one coordinate
    /// request the refresh made. `None` on a coordinate repository.
    items: Option<BTreeMap<CatalogResourceId, ObservedItem>>,
    /// Every item stream the storage answered for the coordinate, stored
    /// or not yet.
    item_streams: BTreeMap<CatalogResourceId, CatalogResourceRepositoryId>,
    /// Items several repositories hold and none is saved for: blocked
    /// until the user chooses one.
    several: BTreeMap<CatalogResourceId, SeveralRepositories>,
    /// Mallard Cloud refused the page's request for the account's sake
    /// (signed out, or the account refused): only this Mac's side is known,
    /// so nothing may be reviewed.
    cloud_status: Option<ManagedCloudStatus>,
    /// The stored resources, whichever model the storage holds.
    remote: BTreeMap<CatalogResourceId, CatalogResourceEnvelope>,
    remote_updated: BTreeMap<CatalogResourceId, u64>,
}

/// One item's stream on a storage that holds one item per repository, as
/// the coordinate answer observed it.
struct ObservedItem {
    stream: CatalogResourceRepositoryId,
    mainline: Mainline,
    observed: ObservedLibraryHead,
    manifest: LibraryResourceManifest,
}

impl ResolvedScopeWorkspace {
    fn remote_map(&self) -> BTreeMap<CatalogResourceId, CatalogResourceEnvelope> {
        self.remote.clone()
    }

    /// The reviewed head of every stored item, on a per-item storage.
    fn item_heads(
        &self,
        storage_identity: &str,
    ) -> BTreeMap<CatalogResourceId, LibraryHeadCoordinate> {
        self.items
            .as_ref()
            .map(|items| {
                items
                    .iter()
                    .map(|(resource_id, item)| {
                        (
                            resource_id.clone(),
                            head_coordinate(&item.observed.head, storage_identity),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The stream an item publishes to: its own on a per-item storage, the
    /// coordinate's repository otherwise.
    fn stream_for(&self, resource_id: &CatalogResourceId) -> CatalogResourceRepositoryId {
        match &self.items {
            Some(_) => CatalogResourceRepositoryId::for_item(&self.scope.coordinate(), resource_id),
            None => self.capability.repository_id(),
        }
    }

    /// Push, Pull, and Finish that push need both sides.
    fn require_cloud_answered(&self) -> Result<(), String> {
        match &self.cloud_status {
            Some(status) => Err(status
                .message
                .clone()
                .unwrap_or_else(|| "Mallard Cloud refused this account.".to_string())),
            None => Ok(()),
        }
    }

    fn storage_revision(&self) -> Option<u64> {
        match &self.items {
            Some(_) => None, // Independent item repositories have no shared version.
            None => self
                .observed
                .as_ref()
                .map(|observed| observed.head.revision),
        }
    }
}

/// What the one Mallard Cloud request answered for a coordinate.
struct ManagedItems {
    items: BTreeMap<CatalogResourceId, ObservedItem>,
    /// Every item stream the storage answered for, stored or not yet.
    streams: BTreeMap<CatalogResourceId, CatalogResourceRepositoryId>,
    /// Items more than one repository answered for, none of them saved:
    /// left out of the comparison until the user chooses one.
    several: BTreeMap<CatalogResourceId, SeveralRepositories>,
}

/// An item that two or more repositories the account can see hold.
struct SeveralRepositories {
    repositories: Vec<String>,
    /// The item as the first of them stored it, to name its row when it is
    /// not on this Mac.
    presented: Option<(CatalogResourceEnvelope, u64)>,
}

/// The one request a Library refresh makes on Mallard Cloud: the account's
/// item repositories of the coordinate, each with its stream's names and
/// the selected record and manifest inline. Names and bytes are R2's; the
/// selection is this desktop's; a stream the answer could not carry whole
/// is read the ordinary way. Nothing here is a stream listing: a later
/// Push still lists before it writes. An item two repositories answer for
/// is reported, not compared: a saved repository is the one the item uses.
fn observe_managed_items(
    cloud: &ManagedObjectStore,
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    capability: &CatalogCapability,
    coordinate: &CatalogCoordinate,
    pins: &BTreeMap<CatalogResourceId, String>,
) -> Result<ManagedItems, String> {
    for (resource, repository) in pins {
        cloud.set_library_pin(
            CatalogResourceRepositoryId::for_item(coordinate, resource).as_str(),
            Some(repository),
        )?;
    }
    let mut answers = cloud.library_coordinate(&CoordinateQuery {
        provider: coordinate.provider.as_str().to_string(),
        resource_kind: coordinate.resource_kind.as_str().to_string(),
        format: coordinate.format.as_str().to_string(),
        format_version: coordinate.format_version,
    })?;
    answers.retain(|answer| {
        pins.iter().all(|(resource, id)| {
            resource.as_str() != answer.resource_id || id == &answer.repository_id
        })
    });
    for (resource, repository) in pins {
        if !answers
            .iter()
            .any(|answer| &answer.repository_id == repository)
        {
            answers.push(
                mallard_infrastructure::managed_cloud::client::LibraryItemAnswer {
                    repository_id: repository.clone(),
                    resource_id: resource.as_str().to_owned(),
                    library_repository_id: CatalogResourceRepositoryId::for_item(
                        coordinate, resource,
                    )
                    .as_str()
                    .to_owned(),
                    names: Vec::new(),
                    truncated: false,
                    record: None,
                    manifest: None,
                },
            );
        }
    }
    let mut holders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for answer in &answers {
        holders
            .entry(answer.resource_id.clone())
            .or_default()
            .push(answer.repository_id.clone());
    }
    let mut observed = ManagedItems {
        items: BTreeMap::new(),
        streams: BTreeMap::new(),
        several: BTreeMap::new(),
    };
    for answer in answers {
        let resource_id = CatalogResourceId::parse(answer.resource_id.clone())?;
        let stream = CatalogResourceRepositoryId::parse(answer.library_repository_id.clone())?;
        if stream != CatalogResourceRepositoryId::for_item(coordinate, &resource_id) {
            return Err(
                "Mallard Cloud answered with an item stream its coordinate does not derive"
                    .to_string(),
            );
        }
        let repositories = &holders[&answer.resource_id];
        if repositories.len() > 1 {
            if observed.several.contains_key(&resource_id) {
                continue;
            }
            // Only to name the row: an item that cannot be read here is
            // still one to choose a repository for.
            let presented = observe_item_answer(
                engine,
                capability,
                coordinate,
                &answer,
                &resource_id,
                &stream,
            )
            .ok()
            .flatten()
            .and_then(|item| {
                item.manifest
                    .resources
                    .get(&resource_id)
                    .map(|entry| (entry.resource.clone(), entry.updated_at))
            });
            observed.several.insert(
                resource_id,
                SeveralRepositories {
                    repositories: repositories.clone(),
                    presented,
                },
            );
            continue;
        }
        observed.streams.insert(resource_id.clone(), stream.clone());
        if let Some(item) = observe_item_answer(
            engine,
            capability,
            coordinate,
            &answer,
            &resource_id,
            &stream,
        )? {
            observed.items.insert(resource_id, item);
        }
    }
    Ok(observed)
}

/// One item's stream as the coordinate answer carried it, or as the
/// ordinary read finds it when the answer could not carry it whole. `None`
/// when the stream holds nothing yet.
fn observe_item_answer(
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    capability: &CatalogCapability,
    coordinate: &CatalogCoordinate,
    answer: &mallard_infrastructure::managed_cloud::client::LibraryItemAnswer,
    resource_id: &CatalogResourceId,
    stream: &CatalogResourceRepositoryId,
) -> Result<Option<ObservedItem>, String> {
    let inline = answer.record.as_ref().filter(|_| !answer.truncated);
    let (mainline, observed, manifest) = if let Some(record) = inline {
        let names = answer
            .names
            .iter()
            .filter_map(|name| RecordName::parse(name))
            .collect::<Vec<_>>();
        let mainline = Mainline::select(names.iter().cloned());
        let Some(selected) = mainline.selected().cloned() else {
            return Ok(None);
        };
        if sha256_hex(record) != selected.token().record_sha256() {
            return Err(format!(
                "the record of item '{}' does not match the digest in its name",
                stream
            ));
        }
        let commit = parse_library_record(stream, &selected, record)?;
        let head = commit.head();
        head.validate()?;
        let manifest = match &answer.manifest {
            Some(bytes)
                if bytes.len() as u64 == head.manifest_size
                    && sha256_hex(bytes) == head.manifest_sha256 =>
            {
                let manifest: LibraryResourceManifest = serde_json::from_slice(bytes)
                    .map_err(|error| format!("parse Library resource manifest: {}", error))?;
                manifest.validate()?;
                if manifest.repository_id != head.repository_id
                    || manifest.revision != head.revision
                    || manifest.commit_id != head.commit_id
                {
                    return Err("Library manifest coordinates do not match its head".to_string());
                }
                manifest
            }
            Some(_) => {
                return Err(format!(
                    "the manifest of item '{}' does not match its record",
                    stream
                ))
            }
            None => engine.read_manifest(&head)?,
        };
        if let Some(cache) = engine.store().publication_cache() {
            cache.prime(
                &RecordStream::library(stream.as_str())?,
                &names,
                Some((selected.token(), record.as_slice())),
            );
        }
        let observed = ObservedLibraryHead {
            record_sha256: selected.token().record_sha256().to_string(),
            head,
        };
        (mainline, observed, manifest)
    } else {
        let (mainline, observed) = engine.observe(stream)?;
        let Some(observed) = observed else {
            return Ok(None);
        };
        let manifest = engine.read_manifest(&observed.head)?;
        (mainline, observed, manifest)
    };
    if !manifest.is_item_stream() {
        return Err(format!(
            "the item stream '{}' holds a set that is not one item",
            stream
        ));
    }
    let Some(entry) = manifest.resources.get(resource_id) else {
        return Err(format!(
            "the item stream '{}' holds another item than the one it is derived from",
            stream
        ));
    };
    if entry.resource.coordinate() != *coordinate {
        return Err("the stored repository holds a resource of another coordinate".to_string());
    }
    capability
        .codec
        .validate(&entry.resource)
        .map_err(|error| format!("a stored resource failed validation: {}", error))?;
    Ok(Some(ObservedItem {
        stream: stream.clone(),
        mainline,
        observed,
        manifest,
    }))
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
    let mut link = storage
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
    let mut stale_acceptance = false;
    let mut stale_items = BTreeSet::new();
    let mut chain = None;
    let mut items = None;
    let mut item_streams = BTreeMap::new();
    let mut several = BTreeMap::new();
    let mut cloud_status = None;
    let mut cloud_repositories = BTreeMap::new();
    let engine = storage.as_ref().map(engine_for).transpose()?;
    let mut remote = BTreeMap::new();
    let mut remote_updated = BTreeMap::new();
    if let Some(storage) = &storage {
        let engine = engine.as_ref().expect("a storage has an engine");
        let storage_identity = storage_identity_for_config(storage)?;
        if let Some(cloud) = engine.store().managed_cloud() {
            // One item per repository: one request observes every item of
            // the coordinate. Continuity is per item; an accepted item head
            // off its stream's chain, or of a stream the storage no longer
            // answers for, is read as absent, and nothing is written.
            let answered = observe_managed_items(
                cloud,
                engine,
                &capability,
                &coordinate,
                &link
                    .as_ref()
                    .map(|link| link.cloud_repositories.clone())
                    .unwrap_or_default(),
            );
            // A sign-in the service rejected, or an account it refused, is
            // the account's state, not the page's: the rows still list what
            // is on this Mac, and the page says why nothing else is known.
            let ManagedItems {
                items: observed_items,
                streams,
                several: held,
            } = match answered {
                Ok(answered) => answered,
                Err(error) => {
                    let Some(health) = cloud.client().health().filter(|health| health.suspends())
                    else {
                        return Err(error);
                    };
                    cloud_status = Some(ManagedCloudStatus {
                        repository_id: String::new(),
                        state: if health.state
                            == mallard_infrastructure::storage_core::ConnectionState::PermissionDenied
                        {
                            ManagedCloudStatusState::Inaccessible
                        } else {
                            ManagedCloudStatusState::SignInRequired
                        },
                        link: None,
                        publication: None,
                        message: Some(health.describe()),
                    });
                    ManagedItems {
                        items: BTreeMap::new(),
                        streams: BTreeMap::new(),
                        several: BTreeMap::new(),
                    }
                }
            };
            for (resource, stream) in &streams {
                if let Some(id) = cloud.library_repository_in_use(stream.as_str())? {
                    cloud_repositories.insert(resource.clone(), id);
                }
            }
            if let Some(link) = &link {
                for (resource, id) in &link.cloud_repositories {
                    cloud_repositories
                        .entry(resource.clone())
                        .or_insert_with(|| id.clone());
                }
            }
            if let Some(link) = link.as_mut().filter(|_| cloud_status.is_none()) {
                let accepted: Vec<_> = link.accepted_items.keys().cloned().collect();
                for resource_id in accepted {
                    if held.contains_key(&resource_id) {
                        // Waiting for a choice, not replaced: its acceptance
                        // stays for the repository the user picks.
                        continue;
                    }
                    let replaced = match observed_items.get(&resource_id) {
                        Some(item) => {
                            verify_library_continuity(
                                link.accepted_items.get(&resource_id),
                                &item.mainline,
                                &item.stream,
                                &storage_identity,
                            ) == LibraryContinuity::Replaced
                        }
                        None => true,
                    };
                    if replaced {
                        link.accepted_items.remove(&resource_id);
                        link.accepted_resources.remove(&resource_id);
                        stale_items.insert(resource_id);
                    }
                }
            }
            for (resource_id, item) in &observed_items {
                let entry = &item.manifest.resources[resource_id];
                remote.insert(resource_id.clone(), entry.resource.clone());
                remote_updated.insert(resource_id.clone(), entry.updated_at);
            }
            items = Some(observed_items);
            item_streams = streams;
            several = held;
        } else {
            let (mainline, selected) = engine.observe(&repository_id)?;
            observed = selected;
            let continuity = verify_library_continuity(
                link.as_ref().and_then(|link| link.accepted_head.as_ref()),
                &mainline,
                &repository_id,
                &storage_identity,
            );
            // The accepted state names a history this storage does not hold:
            // it is read as absent, and nothing is written. Every row then
            // compares as unreviewed, and the next Push or Pull drops the saved
            // acceptances and saves its own. Nothing is reset (CAS-free plan,
            // decision 12: recovery is a user action).
            stale_acceptance = continuity == LibraryContinuity::Replaced;
            if stale_acceptance {
                if let Some(link) = link.as_mut() {
                    link.accepted_head = None;
                    link.accepted_resources.clear();
                }
            }
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
                for (resource_id, entry) in &read.resources {
                    if entry.resource.coordinate() != coordinate {
                        return Err(
                            "the stored repository holds a resource of another coordinate"
                                .to_string(),
                        );
                    }
                    capability
                        .codec
                        .validate(&entry.resource)
                        .map_err(|error| {
                            format!("a stored resource failed validation: {}", error)
                        })?;
                    remote.insert(resource_id.clone(), entry.resource.clone());
                    remote_updated.insert(resource_id.clone(), entry.updated_at);
                }
                manifest = Some(read);
            }
            chain = Some(mainline);
        }
    }
    Ok(ResolvedScopeWorkspace {
        engine,
        cloud_repositories,
        scope,
        profile,
        capability,
        context,
        inventory,
        storage,
        link,
        stale_acceptance,
        stale_items,
        mainline: chain,
        observed,
        manifest,
        items,
        item_streams,
        several,
        cloud_status,
        remote,
        remote_updated,
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
    let remote_updated = &workspace.remote_updated;

    let mut resource_ids: BTreeSet<&CatalogResourceId> =
        workspace.inventory.resources.keys().collect();
    resource_ids.extend(remote.keys());
    resource_ids.extend(workspace.several.keys());

    let mut rows: Vec<CatalogScopeRowDto> = Vec::new();
    for resource_id in resource_ids {
        let local = workspace.inventory.resources.get(resource_id);
        let several = workspace.several.get(resource_id);
        let remote_resource = remote.get(resource_id);
        // Several repositories hold it and none is chosen: what they hold is
        // shown, never compared.
        let stored = remote_resource.or(several
            .and_then(|several| several.presented.as_ref())
            .map(|(resource, _)| resource));
        let local_sha = local.map(|record| record.resource.content_sha256.as_str());
        let remote_sha = remote_resource.map(|resource| resource.content_sha256.as_str());
        // Nothing is compared without a storage, or while Mallard Cloud
        // refuses the account.
        let state = if workspace.storage.is_none() || workspace.cloud_status.is_some() {
            None
        } else if several.is_some() {
            Some(CatalogLinkState::Blocked)
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
        let Some(presented) = local.map(|record| &record.resource).or(stored) else {
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
            remote_present: stored.is_some(),
            remote_updated_at: remote_updated.get(resource_id).copied().or(several
                .and_then(|several| several.presented.as_ref())
                .map(|(_, updated_at)| *updated_at)),
            content_sha256: local_sha
                .or(stored.map(|resource| resource.content_sha256.as_str()))
                .map(str::to_string),
            preview_png: None,
            description: codec.description(presented),
            warnings: local
                .map(|record| record.warnings.clone())
                .unwrap_or_default(),
            blocker: several.map(|several| {
                format!(
                    "In {} repositories you can see. Choose one on its page.",
                    several.repositories.len()
                )
            }),
        };
        decorate_local_preview(&mut row, &workspace.capability, local, user_home);
        if local.is_none() {
            // The scope listing already verified this manifest. Read cached
            // artwork here so each card avoids another head and manifest read.
            // A cache miss stays lazy and must not fetch payloads in the list.
            row.preview_png = stored.and_then(|resource| {
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
    let storage_revision = workspace.storage_revision();
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
        storage_revision,
        rows,
        warnings: workspace.inventory.warnings,
        recovery,
        cloud_connections: workspace
            .link
            .as_ref()
            .map(|link| link.cloud_repositories.clone())
            .unwrap_or_default(),
        cloud_repositories: workspace.cloud_repositories,
        cloud_status: workspace.cloud_status,
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
    let (workspace, repository_id, resource) = stored_scope_resource(
        registry,
        state,
        catalog,
        scope_id,
        storage_id,
        resource_id,
        user_home,
    )?;
    let engine = workspace.engine.as_ref().expect("a storage has an engine");
    workspace
        .capability
        .codec
        .preview(&resource, &preview_cache_dir(user_home), &|sha256, size| {
            engine.fetch_payload(&repository_id, sha256, size)
        })?
        .ok_or_else(|| "this resource has no preview".to_string())
}

/// One stored resource of a scope, read from its repository and held to the
/// scope's coordinate and codec: the first half of every read that serves a
/// resource this Mac does not have (its preview, its page).
fn stored_scope_resource(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    resource_id: &CatalogResourceId,
    user_home: &Path,
) -> Result<
    (
        ResolvedScopeWorkspace,
        CatalogResourceRepositoryId,
        CatalogResourceEnvelope,
    ),
    String,
> {
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let engine = workspace.engine.as_ref().expect("a storage has an engine");
    let repository_id = workspace.stream_for(resource_id);
    let (_, manifest) = engine
        .inspect(&repository_id)?
        .ok_or("the stored resource disappeared")?;
    let resource = manifest
        .resources
        .get(resource_id)
        .map(|entry| entry.resource.clone())
        .ok_or("the stored resource disappeared")?;
    if resource.coordinate() != workspace.scope.coordinate() {
        return Err("the stored resource does not belong to this Catalog row".to_string());
    }
    workspace.capability.codec.validate(&resource)?;
    Ok((workspace, repository_id, resource))
}

/// A resource's page. This Mac's copy when the inventory holds one, read
/// from the capture the inventory already made — no storage is asked;
/// otherwise the storage's copy, fetched and verified as a stored-only
/// preview is. A plugin installed here adds the version the provider
/// observed and what its own manifest says about it.
fn get_catalog_resource_detail_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: Option<&StorageId>,
    resource_id: &CatalogResourceId,
    user_home: &Path,
) -> Result<CatalogResourceDetail, String> {
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
    if let Some(record) = inventory.resources.get(resource_id) {
        let payloads = record
            .captured
            .as_ref()
            .map(CapturedResource::payloads)
            .unwrap_or_default();
        let mut detail = record
            .capability
            .codec
            .detail(&record.resource, &|sha256, size| {
                payloads
                    .get(sha256)
                    .filter(|bytes| bytes.len() as u64 == size)
                    .map(|bytes| bytes.to_vec())
                    .ok_or_else(|| "the captured payload is missing".to_string())
            })?;
        fill_install_lines(&mut detail, &record.capability, &record.resource);
        if let (
            CatalogResourceDetail::Install {
                plugin_id,
                installed_version,
                listing,
                ..
            },
            Some(native),
        ) = (&mut detail, inventory.native.as_ref())
        {
            let profile = &context.profile;
            *installed_version = native
                .observation(plugin_id)
                .filter(|observation| observation.installed)
                .and_then(|observation| observation.observed_version.clone());
            *listing = super::plugin_listing::installed_listing(
                profile.provider.as_str(),
                &profile.canonical_home,
                native,
                plugin_id,
            )
            .map(|listing| {
                Box::new(CatalogDetailPluginListing {
                    display_name: listing.display_name,
                    developer: listing.developer,
                    description: listing.description,
                    category: listing.category,
                    website_url: listing.website_url,
                    privacy_url: listing.privacy_url,
                    terms_url: listing.terms_url,
                })
            });
        }
        return Ok(detail);
    }
    let storage_id = storage_id.ok_or("this resource is not on this Mac")?;
    let (workspace, repository_id, resource) = stored_scope_resource(
        registry,
        state,
        catalog,
        scope_id,
        storage_id,
        resource_id,
        user_home,
    )?;
    let engine = workspace.engine.as_ref().expect("a storage has an engine");
    let mut detail = workspace
        .capability
        .codec
        .detail(&resource, &|sha256, size| {
            engine.fetch_payload(&repository_id, sha256, size)
        })?;
    fill_install_lines(&mut detail, &workspace.capability, &resource);
    Ok(detail)
}

/// An install intent's page carries the lines that install it by hand: its
/// adapter's own plan for a home that has nothing yet — the argv a Pull runs
/// — with the provider's CLI in front. A plan the adapter refuses (a
/// marketplace that is not portable, a ref its CLI cannot bind) prints no
/// lines, and neither does one with a word that is not plain enough to paste
/// into a shell: the page never hands over a line that does more than install.
fn fill_install_lines(
    detail: &mut CatalogResourceDetail,
    capability: &CatalogCapability,
    envelope: &CatalogResourceEnvelope,
) {
    let CatalogResourceDetail::Install { install, .. } = detail else {
        return;
    };
    let Some(cli) = capability.adapter.native_cli() else {
        return;
    };
    let Ok(plan) = capability.adapter.plan_native_apply(
        envelope,
        &ProviderInventory::default(),
        PluginApplyMode::InstallMissing,
    ) else {
        return;
    };
    let commands: Vec<Vec<String>> = plan
        .marketplace_argv
        .into_iter()
        .chain(std::iter::once(plan.argv))
        .collect();
    let plain = |word: &String| {
        !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"@%+=:,./_-".contains(&byte))
    };
    if !commands.iter().flatten().all(plain) {
        return;
    }
    *install = commands
        .into_iter()
        .map(|argv| format!("{} {}", cli.program, argv.join(" ")))
        .collect();
}

/// The storage page's Catalog listing: every repository this build can
/// derive — one per registered capability — inspected in one metadata batch.
/// Only derived repositories are read; other repositories are
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
    if let Some(cloud) = engine.store().managed_cloud() {
        return Ok(registry
            .capabilities()
            .filter_map(|capability| read_managed_storage_repository(cloud, &engine, capability))
            .collect());
    }
    let repositories = registry
        .capabilities()
        .map(|capability| capability.repository_id())
        .collect::<Vec<_>>();
    let discovered = engine.discover_repositories(&repositories)?;
    Ok(registry
        .capabilities()
        .zip(discovered)
        .filter_map(|(capability, discovered)| {
            read_storage_repository(&engine, capability, discovered)
        })
        .collect())
}

/// One coordinate's items on a storage that holds one item per repository,
/// from the coordinate answer: `None` when the account holds none.
fn read_managed_storage_repository(
    cloud: &ManagedObjectStore,
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
    let ManagedItems { items, several, .. } =
        match observe_managed_items(cloud, engine, capability, &coordinate, &BTreeMap::new()) {
            Ok(observed) => observed,
            Err(message) => return Some(refusal(message)),
        };
    if items.is_empty() && several.is_empty() {
        return None;
    }
    let names = items
        .iter()
        .map(|(resource_id, item)| &item.manifest.resources[resource_id].resource)
        .chain(
            several
                .values()
                .filter_map(|several| several.presented.as_ref())
                .map(|(resource, _)| resource),
        )
        .map(|resource| capability.codec.display_name(resource))
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
        revision: items.values().map(|item| item.observed.head.revision).max(),
        updated_at: items.values().map(|item| item.manifest.updated_at).max(),
        resource_names,
        error: None,
    })
}

/// `None` when the repository does not exist in this storage; a refusal row
/// when something exists under the derived id but cannot be read.
fn read_storage_repository(
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    capability: &CatalogCapability,
    discovered: Result<DiscoveredLibraryRepository, String>,
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
    let DiscoveredLibraryRepository {
        head: observed,
        tag,
    } = match discovered {
        Ok(discovered) => discovered,
        Err(message) => return Some(refusal(message)),
    };
    let Some(observed) = observed else {
        return tag.map(|_| {
            // A tag with no publication is what a failed first publish leaves
            // behind; say so rather than passing it off as absent.
            refusal("the stored repository has a discovery tag but no publication".to_string())
        });
    };
    let Some(tag) = tag else {
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
    let storage = storage_for(state, storage_id)?;
    if storage.provider.as_str() == "managed_cloud" {
        // Connecting must work even when several accessible repositories
        // contain the same item. Only a later review observes content;
        // repository selection is metadata and never accepts a remote base.
        let resolved = resolve_scope(catalog, scope_id)?;
        let repository_id = require_capability(registry, &resolved.scope)?.repository_id();
        return catalog.mutate_scope_links(|document| {
            let link = match document.link_for(scope_id, storage_id) {
                Some(link) => link.clone(),
                None => {
                    let link = CatalogScopeLink {
                        link_id: CatalogLinkId::generate()?,
                        scope_id: scope_id.clone(),
                        storage_id: storage_id.clone(),
                        repository_id,
                        accepted_head: None,
                        accepted_resources: BTreeMap::new(),
                        accepted_items: BTreeMap::new(),
                        cloud_repositories: BTreeMap::new(),
                        revision: 1,
                        created_at: now,
                        updated_at: now,
                    };
                    document.links.push(link.clone());
                    link
                }
            };
            Ok(CatalogScopeLinkOutcomeDto {
                link_id: link.link_id,
                revision: link.revision,
                completion: "open".to_string(),
            })
        });
    }
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
    let storage_identity = storage_identity_for_config(storage)?;
    // On a per-item storage every agreeing item pins its own head; on a
    // coordinate repository the head is pinned only when every stored
    // resource agreed.
    let accepted_items: BTreeMap<CatalogResourceId, LibraryHeadCoordinate> = workspace
        .item_heads(&storage_identity)
        .into_iter()
        .filter(|(resource_id, _)| accepted_resources.contains_key(resource_id))
        .collect();
    let accepted_head = match (&workspace.observed, full_agreement) {
        (Some(observed), true) => Some(head_coordinate(&observed.head, &storage_identity)),
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
            accepted_items: accepted_items.clone(),
            cloud_repositories: BTreeMap::new(),
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
        CatalogResourcePlanDocument::Fork(_) | CatalogResourcePlanDocument::Removal(_) => false,
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
    // Supersede when the request starts. A cancelled request can finish
    // after its replacement; its late result must never discard that review.
    discard_scope_plans(catalog, scope_id, storage_id);
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    workspace.require_cloud_answered()?;
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
    let mut plan = build_scope_push_plan(ScopePushPlanInputs {
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
        observed_items: workspace.item_heads(&storage_identity),
        blocked: workspace
            .several
            .iter()
            .map(|(resource_id, several)| {
                (
                    resource_id.clone(),
                    format!(
                        "In {} repositories you can see. Choose one on its page.",
                        several.repositories.len()
                    ),
                )
            })
            .collect(),
        now,
    })?;
    plan.destinations = push_destinations(&workspace, storage, &plan);
    plan_store::save(
        catalog.root(),
        CatalogResourcePlanDocument::Push(plan.clone()),
        now,
    )?;
    Ok(plan)
}

/// The account's catalog repositories, for a review to name where each item
/// goes or comes from: one listing, and one read for a saved repository the
/// account does not belong to. `None` off Mallard Cloud, or when the listing
/// fails: the review still opens, its rows unnamed.
struct CloudNames<'a> {
    cloud: &'a ManagedObjectStore,
    login: String,
    summaries: Vec<mallard_infrastructure::managed_cloud::client::RepositorySummary>,
}

impl<'a> CloudNames<'a> {
    fn load(workspace: &'a ResolvedScopeWorkspace, storage: &StorageConfig) -> Option<Self> {
        let cloud = workspace.engine.as_ref()?.store().managed_cloud()?;
        let (settings, _) = mallard_infrastructure::storage_providers::managed_cloud::client_for(
            &storage.provider_config,
        )
        .ok()?;
        Some(Self {
            cloud,
            login: settings.account_login,
            summaries: cloud.list_catalog_repositories().ok()?,
        })
    }

    fn summary(
        &self,
        repository_id: &str,
    ) -> Option<mallard_infrastructure::managed_cloud::client::RepositorySummary> {
        self.summaries
            .iter()
            .find(|summary| summary.id == repository_id)
            .cloned()
            .or_else(|| self.cloud.repository_summary(repository_id).ok())
    }
}

/// Where each pushable item goes on Mallard Cloud: the repository it has,
/// saved or learned, or the one this Push creates, named as the service is
/// expected to name it. One listing of the account's catalog repositories
/// serves every row; a listing that fails names none, and the review still
/// opens.
fn push_destinations(
    workspace: &ResolvedScopeWorkspace,
    storage: &StorageConfig,
    plan: &CatalogScopePushPlan,
) -> BTreeMap<CatalogResourceId, CatalogPushDestination> {
    let pushable = plan
        .rows
        .iter()
        .filter(|row| row.row.selectable)
        .collect::<Vec<_>>();
    if pushable.is_empty() {
        return BTreeMap::new();
    }
    let Some(names) = CloudNames::load(workspace, storage) else {
        return BTreeMap::new();
    };
    // The keys of the names the account owns, and of the names this Push takes as it goes.
    let mut taken = names
        .summaries
        .iter()
        .filter(|summary| summary.role.as_deref() == Some("owner"))
        .map(|summary| cloud_names::name_key(&summary.name))
        .collect::<BTreeSet<_>>();
    let mut destinations = BTreeMap::new();
    for row in pushable {
        let resource_id = &row.row.resource_id;
        let destination = match workspace.cloud_repositories.get(resource_id) {
            Some(repository_id) => {
                let Some(summary) = names.summary(repository_id) else {
                    continue;
                };
                CatalogPushDestination {
                    repository_id: Some(repository_id.clone()),
                    address: format!("{}/{}", summary.owner.github_login, summary.name),
                    create: false,
                }
            }
            None => {
                // A display name that derives no address has no destination
                // to show: the service refuses the create, and the outcome
                // carries its sentence.
                let Some(name) = cloud::predicted_repository_name(&row.row.display_name, &taken)
                else {
                    continue;
                };
                taken.insert(cloud_names::name_key(&name));
                CatalogPushDestination {
                    repository_id: None,
                    address: format!("{}/{}", names.login, name),
                    create: true,
                }
            }
        };
        destinations.insert(resource_id.clone(), destination);
    }
    destinations
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
/// The user's verb for a push onto the selected version that did not
/// finish (CAS-free plan, section 13): write the record its claim holds,
/// once, under the same local single-flight lock a Push takes, and report
/// with the sentences of the Push review. Nothing local moves.
fn finish_unfinished_push_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    scope_id: &CatalogScopeId,
    storage_id: &StorageId,
    user_home: &Path,
) -> Result<(), String> {
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    workspace.require_cloud_answered()?;
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let engine = workspace
        .engine
        .as_ref()
        .expect("a linked workspace has its engine");
    if workspace.items.is_some() {
        // One item per repository: finish the unfinished push of every item
        // stream that holds one; a stream that holds none is not a change.
        let mut finished = 0;
        for stream in workspace.item_streams.values() {
            let _publication = catalog.acquire_publication_lock(storage_id, stream)?;
            match engine.finish_unfinished_push(stream) {
                Ok(()) => finished += 1,
                Err(error) if error == REMOTE_CHANGED => {}
                Err(error) => return Err(unfinished_push_sentence(&storage.name, error)),
            }
        }
        return if finished == 0 {
            Err(REMOTE_CHANGED.to_string())
        } else {
            Ok(())
        };
    }
    let _publication = catalog.acquire_publication_lock(storage_id, &link.repository_id)?;
    engine
        .finish_unfinished_push(&link.repository_id)
        .map_err(|error| unfinished_push_sentence(&storage.name, error))
}

fn apply_push_with_repositories(
    registry: &CatalogRegistry,
    state: &LocalStateRepository,
    catalog: &CatalogStateRepository,
    plan_id: CatalogPlanId,
    selected_resource_ids: Vec<CatalogResourceId>,
    acknowledged_warning_digests: Vec<String>,
    user_home: &Path,
    now: u64,
    report: &ProgressReporter,
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
            results: Vec::new(),
            stopped: None,
            revision: plan.observed_head.map(|head| head.revision),
            message: "Nothing was pushed".to_string(),
        });
    }

    // Local single-flight per repository: a second Apply of this review must
    // not list the same childless parent and write a sibling record.
    let _publication = catalog.acquire_publication_lock(&plan.storage_id, &plan.repository_id)?;
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        &plan.scope_id,
        Some(&plan.storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    workspace.require_cloud_answered()?;
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

    if workspace.items.is_some() {
        return apply_push_items(
            catalog,
            &workspace,
            &plan,
            storage,
            &storage_identity,
            &selected_records,
            state.load_or_create_host_identity()?.host_id.as_str(),
            now,
            report,
        );
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

    let engine = workspace
        .engine
        .as_ref()
        .expect("a linked workspace has its engine");
    let repository_id = link.repository_id.clone();
    let host = state.load_or_create_host_identity()?;
    report(CatalogSyncProgress {
        plan_id: plan.plan_id.clone(),
        completed: 0,
        total: selected_records.len(),
        message: "Publishing reviewed items…".to_string(),
    });
    let published = engine
        .publish(PublishLibraryResourceRequest {
            repository_id: repository_id.clone(),
            tag: LibraryResourceTag::new(&workspace.scope.coordinate(), repository_id.clone()),
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
        })
        .map_err(|error| unfinished_push_sentence(&storage.name, error))?;

    // A stored publication is the selected one: a push whose record ranked
    // behind another computer's failed above, before any accepted state moved.
    let coordinate = head_coordinate(published.head(), &storage_identity);
    // Accepted bases come from the manifest that actually landed, which on
    // the no-op path is the previous manifest already holding this content.
    let accepted: Vec<(CatalogResourceId, String)> = selected_records
        .iter()
        .map(|(resource_id, _)| {
            let entry = published
                .manifest()
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
    let stale_acceptance = workspace.stale_acceptance;
    let acceptance = catalog.mutate_scope_links(|document| {
        let link = document
            .links
            .iter_mut()
            .find(|link| link.link_id == link_id)
            .ok_or("the scope link disappeared while publishing")?;
        if link.revision != link_revision {
            return Err("the scope link changed while publishing".to_string());
        }
        if stale_acceptance {
            // The saved acceptances describe a history the repository no
            // longer has: from here on only what this Push accepted counts.
            link.accepted_resources.clear();
        }
        link.accepted_head = Some(coordinate.clone());
        for (resource_id, content) in &accepted {
            link.accepted_resources
                .insert(resource_id.clone(), content.clone());
        }
        link.revision = link.revision.saturating_add(1);
        link.updated_at = now;
        Ok(())
    });
    plan_store::discard(catalog.root(), &plan_id);
    let revision = published.head().revision;
    Ok(CatalogScopePushOutcomeDto {
        plan_id,
        published: published.stored(),
        results: selected_records
            .iter()
            .map(|(id, record)| CatalogPushResult {
                resource_id: (*id).clone(),
                display_name: workspace
                    .capability
                    .codec
                    .display_name(&record.resource)
                    .unwrap_or_else(|_| id.as_str().to_owned()),
                state: if published.stored() {
                    "published"
                } else {
                    "unchanged"
                }
                .to_string(),
                message: None,
                revision: None,
            })
            .collect(),
        stopped: acceptance.err().map(|error| {
            format!(
                "Published, but local comparison state could not be saved: {error}. Compare again."
            )
        }),
        revision: Some(revision),
        message: match published {
            PublishedLibraryResource::Stored { .. } => format!("Published version {}", revision),
            PublishedLibraryResource::NoChange { .. } => {
                "The linked storage already had this content".to_string()
            }
        },
    })
}

/// After a plugin lands on Mallard Cloud: the installed copy's own words for
/// the commit (`plugin_listing.rs`), so the plugin's page can say what it is.
/// Sent once per push, after the publication lock is released; it is display
/// only, so whatever it answers, the push and its acceptance stand.
fn send_plugin_listing(
    engine: &LibraryResourceEngine<ConfiguredObjectStore>,
    workspace: &ResolvedScopeWorkspace,
    record: &LocalResourceRecord,
    repository_id: &str,
    commit_id: &str,
) {
    if record.resource.resource_kind.as_str() != super::plugin_intent::KIND_KEY {
        return;
    }
    let (Some(cloud), Some(inventory)) = (
        engine.store().managed_cloud(),
        workspace.inventory.native.as_ref(),
    ) else {
        return;
    };
    let Ok(intent) = super::plugin_intent::PluginInstallIntent::decode(&record.resource) else {
        return;
    };
    let profile = &workspace.context.profile;
    if let Some(listing) = super::plugin_listing::installed_listing(
        profile.provider.as_str(),
        &profile.canonical_home,
        inventory,
        &intent.identity.plugin_id,
    ) {
        let _ = cloud.put_plugin_listing(repository_id, commit_id, &listing);
    }
}

/// The Push apply on a storage that holds one item per repository: every
/// selected item is published on its own stream, in row order, through the
/// Library engine and the shared protocol; each item's accepted state
/// advances as its record lands. A row that fails stops the apply and is
/// named; the rows before it stay published, each being its own
/// repository, and their acceptance is saved before the error is reported.
#[allow(clippy::too_many_arguments)]
fn apply_push_items(
    catalog: &CatalogStateRepository,
    workspace: &ResolvedScopeWorkspace,
    plan: &CatalogScopePushPlan,
    storage: &StorageConfig,
    storage_identity: &str,
    selected_records: &[(&CatalogResourceId, &LocalResourceRecord)],
    host_id: &str,
    now: u64,
    report: &ProgressReporter,
) -> Result<CatalogScopePushOutcomeDto, String> {
    let link = require_link(workspace)?;
    let observed_items = workspace.item_heads(storage_identity);
    for (resource_id, _) in selected_records {
        if plan.observed_items.get(*resource_id) != observed_items.get(*resource_id) {
            return Err("the storage head changed after review".to_string());
        }
    }
    let items = workspace.items.as_ref().expect("per-item workspace");
    let engine = workspace.engine.as_ref().expect("linked workspace engine");
    let mut accepted = Vec::new();
    let mut results = Vec::new();
    let mut published = false;
    let mut stopped = None;
    for row in &plan.rows {
        let Some((resource_id, record)) = selected_records
            .iter()
            .find(|(id, _)| **id == row.row.resource_id)
        else {
            continue;
        };
        let mut result = CatalogPushResult {
            resource_id: (*resource_id).clone(),
            display_name: row.row.display_name.clone(),
            state: "not_attempted".to_string(),
            message: None,
            revision: None,
        };
        if stopped.is_none() {
            report(CatalogSyncProgress {
                plan_id: plan.plan_id.clone(),
                completed: results.len(),
                total: selected_records.len(),
                message: format!("Pushing {}…", row.row.display_name),
            });
            let stream = workspace.stream_for(resource_id);
            let attempt = (|| {
                let _publication = catalog.acquire_publication_lock(&plan.storage_id, &stream)?;
                let origin =
                    super::domain::derive_library_origin(host_id, link.link_id.as_str(), &stream)?;
                engine.publish(PublishLibraryResourceRequest {
                    repository_id: stream.clone(),
                    tag: LibraryResourceTag::for_item(&workspace.scope.coordinate(), resource_id),
                    observed: items.get(*resource_id).map(|item| item.observed.clone()),
                    resources: BTreeMap::from([(
                        (*resource_id).clone(),
                        PublishedResourceEntry {
                            resource: record.resource.clone(),
                            updated_at: now,
                        },
                    )]),
                    payloads: record
                        .captured
                        .as_ref()
                        .map(|captured| captured.payloads())
                        .unwrap_or_default(),
                    captured_with: CatalogCapturedWith {
                        app: "mallard".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                    published_by_origin: origin,
                    updated_at: now,
                })
            })();
            match attempt {
                Ok(outcome) => {
                    published |= outcome.stored();
                    result.state = if outcome.stored() {
                        "published"
                    } else {
                        "unchanged"
                    }
                    .to_string();
                    result.revision = Some(outcome.head().revision);
                    let repository = engine.store().managed_cloud().and_then(|cloud| {
                        cloud
                            .library_repository_in_use(stream.as_str())
                            .ok()
                            .flatten()
                    });
                    if let Some(repository_id) = repository.as_deref() {
                        send_plugin_listing(
                            engine,
                            workspace,
                            record,
                            repository_id,
                            outcome.manifest().commit_id.as_str(),
                        );
                    }
                    accepted.push((
                        (*resource_id).clone(),
                        record.resource.content_sha256.clone(),
                        head_coordinate(outcome.head(), storage_identity),
                        repository,
                    ));
                }
                Err(error) => {
                    result.state = "failed".to_string();
                    result.message = Some(unfinished_push_sentence(&storage.name, error));
                    stopped = result.message.clone();
                }
            }
        }
        results.push(result);
    }
    if !accepted.is_empty() || !workspace.stale_items.is_empty() {
        let saved = catalog.mutate_scope_links(|document| {
            let saved = document
                .links
                .iter_mut()
                .find(|saved| saved.link_id == link.link_id)
                .ok_or("the scope link disappeared while publishing")?;
            if saved.revision != plan.link_revision {
                return Err("the scope link changed while publishing".to_string());
            }
            for id in &workspace.stale_items {
                saved.accepted_items.remove(id);
                saved.accepted_resources.remove(id);
            }
            for (id, digest, head, repository) in &accepted {
                saved.accepted_resources.insert(id.clone(), digest.clone());
                saved.accepted_items.insert(id.clone(), head.clone());
                if let Some(repository) = repository {
                    saved
                        .cloud_repositories
                        .insert(id.clone(), repository.clone());
                }
            }
            saved.revision = saved.revision.saturating_add(1);
            saved.updated_at = now;
            Ok(())
        });
        if let Err(error) = saved {
            stopped = Some(format!("Cloud publications completed, but local comparison state could not be saved: {error}. Compare again."));
        }
    }
    plan_store::discard(catalog.root(), &plan.plan_id);
    report(CatalogSyncProgress {
        plan_id: plan.plan_id.clone(),
        completed: results.len(),
        total: results.len(),
        message: "Push finished".to_string(),
    });
    Ok(CatalogScopePushOutcomeDto {
        plan_id: plan.plan_id.clone(),
        published,
        revision: None, // Each item has its own version; a batch has no version.
        message: format!("{} of {} items completed", accepted.len(), results.len()),
        results,
        stopped,
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
    discard_scope_plans(catalog, scope_id, storage_id);
    let workspace = resolve_scope_workspace(
        registry,
        state,
        catalog,
        scope_id,
        Some(storage_id),
        user_home,
    )?;
    let link = require_link(&workspace)?;
    workspace.require_cloud_answered()?;
    let storage = workspace
        .storage
        .as_ref()
        .expect("a linked workspace always has its storage");
    let storage_identity = storage_identity_for_config(storage)?;
    let item_heads = workspace.item_heads(&storage_identity);
    let head = match (&workspace.observed, &workspace.items) {
        (Some(observed), _) => Some(head_coordinate(&observed.head, &storage_identity)),
        (None, Some(items)) if !items.is_empty() => None,
        _ => return Err("the storage has nothing of this coordinate to Pull".to_string()),
    };
    let remote = workspace.remote_map();
    let claims = catalog.load_materializations()?.claims;
    let name_taken = name_taken_index(&workspace.inventory);
    let mut plan = build_scope_pull_plan(ScopePullPlanInputs {
        scope: &workspace.scope,
        capability: &workspace.capability,
        link,
        storage_identity_sha256: storage_identity.clone(),
        context: &workspace.context,
        inventory: &workspace.inventory,
        remote: &remote,
        head,
        item_heads,
        claims: &claims,
        requested_copy_names: &requested_copy_names,
        name_taken: &name_taken,
        now,
    })?;
    plan.sources = pull_sources(&workspace, storage, &plan);
    plan_store::save(
        catalog.root(),
        CatalogResourcePlanDocument::Pull(plan.clone()),
        now,
    )?;
    Ok(plan)
}

/// Where each item of a Pull review reads on Mallard Cloud: its repository's
/// address, whether that is its draft or its public version, and the version
/// the review read. Named from the same listing a Push review uses.
fn pull_sources(
    workspace: &ResolvedScopeWorkspace,
    storage: &StorageConfig,
    plan: &CatalogScopePullPlan,
) -> BTreeMap<CatalogResourceId, CatalogPullSource> {
    let read = plan
        .items
        .iter()
        .map(|item| &item.item.resource_id)
        .filter(|resource_id| workspace.cloud_repositories.contains_key(*resource_id))
        .collect::<Vec<_>>();
    if read.is_empty() {
        return BTreeMap::new();
    }
    let Some(names) = CloudNames::load(workspace, storage) else {
        return BTreeMap::new();
    };
    let mut sources = BTreeMap::new();
    for resource_id in read {
        let repository_id = &workspace.cloud_repositories[resource_id];
        let (Some(summary), Some(head)) = (
            names.summary(repository_id),
            plan.item_heads.get(resource_id),
        ) else {
            continue;
        };
        sources.insert(
            resource_id.clone(),
            CatalogPullSource {
                repository_id: repository_id.clone(),
                address: format!("{}/{}", summary.owner.github_login, summary.name),
                reads: if summary.role.is_some() {
                    "draft"
                } else {
                    "public"
                }
                .to_string(),
                revision: head.revision,
            },
        );
    }
    sources
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
    report: &ProgressReporter,
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
    workspace.require_cloud_answered()?;
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
    // The reviewed version must still be part of the selected history. A
    // newer version pushed on top does not matter: every resource
    // description comes from the review, and every payload is fetched by
    // its digest. A version that lost its race, or a cleared repository,
    // refuses the Pull before anything changes (PLAN_PULL_PINNED_VERSION.md,
    // section 4.7).
    let acknowledged: BTreeSet<CatalogResourceId> = destructive_acknowledged.into_iter().collect();
    let selections = validate_scope_pull_selection(&plan, &choices, &acknowledged)?;
    let reviewed_on_chain = match (&plan.head, &workspace.items) {
        (Some(head), _) => workspace
            .mainline
            .as_ref()
            .is_some_and(|mainline| mainline.contains(head.revision, head.commit_id.as_str())),
        // One item per repository: every item acted on must still have its
        // reviewed version on its own stream's chain.
        (None, Some(items)) => selections.iter().all(|selection| {
            selection.action.is_none()
                || plan
                    .item_heads
                    .get(&selection.item.item.resource_id)
                    .zip(items.get(&selection.item.item.resource_id))
                    .is_some_and(|(head, item)| {
                        item.mainline
                            .contains(head.revision, head.commit_id.as_str())
                    })
        }),
        (None, None) => false,
    };
    if !reviewed_on_chain {
        return Err(LIBRARY_REVIEW_SUPERSEDED.to_string());
    }
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
    let engine = workspace
        .engine
        .as_ref()
        .expect("a linked workspace has its engine");
    let mut results = Vec::new();
    let mut accepted_updates: Vec<(CatalogResourceId, String)> = Vec::new();
    let mut all_accepted = true;
    let mut stopped = None;
    let not_pulled = |item: &CatalogPullItem| CatalogActionResult {
        resource_id: item.resource_id.clone(),
        action_id: None,
        display_name: item.display_name.clone(),
        outcome: CatalogActionOutcome::Failed,
        message: Some(NOT_PULLED.to_string()),
    };
    for selection in &selections {
        if stopped.is_some() {
            results.push(not_pulled(&selection.item.item));
            all_accepted = false;
            continue;
        }
        report(CatalogSyncProgress {
            plan_id: plan.plan_id.clone(),
            completed: results.len(),
            total: selections.len(),
            message: format!("Pulling {}…", selection.item.item.display_name),
        });
        // Payloads come from the stream the item was published on: its
        // own on a per-item storage, the coordinate's repository otherwise.
        let stream = workspace.stream_for(&selection.item.item.resource_id);
        let fetch = |payloads: &[(&str, u64)]| engine.fetch_payloads(&stream, payloads);
        let environment = PullApplyEnvironment {
            repository: catalog,
            runner,
            registry,
            capability: workspace.capability.clone(),
            fetch_payload: &fetch,
        };
        let mut result = execute_scope_pull_item(
            &environment,
            &plan.storage_id,
            &plan.plan_id,
            &manifest_resources,
            &selection.item.item,
            selection.action,
            now,
        );
        if result.outcome == CatalogActionOutcome::Failed
            && result.message.as_deref() == Some(VERSION_INCOMPLETE)
        {
            // The storage cannot be read: this resource and the rest are not
            // pulled, what was installed before stays, and the accepted
            // version does not advance (rule 3 of the Pull plan).
            stopped = Some(format!(
                "Part of this version is missing from {}. Compare again.",
                storage.name
            ));
            result = not_pulled(&selection.item.item);
        }
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

    // A Pull from an accepted state that is read as absent drops the saved
    // acceptances and always saves the head it reviewed: otherwise what it
    // accepted would sit under a head off the chain, be read as absent
    // again, and the rows it installed would ask for review once more.
    let stale_acceptance = workspace.stale_acceptance;
    let stale_items = workspace.stale_items.clone();
    let advance_head = all_accepted || stale_acceptance;
    // On a per-item storage each accepted item pins its own reviewed head.
    let accepted_items: Vec<(CatalogResourceId, LibraryHeadCoordinate)> = accepted_updates
        .iter()
        .filter_map(|(resource_id, _)| {
            plan.item_heads
                .get(resource_id)
                .map(|head| (resource_id.clone(), head.clone()))
        })
        .collect();
    let base_changed = stale_acceptance
        || !stale_items.is_empty()
        || accepted_updates.iter().any(|(resource_id, content)| {
            link.accepted_resources.get(resource_id) != Some(content)
        })
        || accepted_items
            .iter()
            .any(|(resource_id, head)| link.accepted_items.get(resource_id) != Some(head))
        || (advance_head && plan.head.is_some() && link.accepted_head != plan.head);
    if base_changed {
        let link_id = link.link_id.clone();
        let link_revision = plan.link_revision;
        let head = plan.head.clone();
        let saved = catalog.mutate_scope_links(|document| {
            let link = document
                .links
                .iter_mut()
                .find(|link| link.link_id == link_id)
                .ok_or("the scope link disappeared while accepting Pull")?;
            if link.revision != link_revision {
                return Err("the scope link changed while accepting Pull".to_string());
            }
            if stale_acceptance {
                link.accepted_resources.clear();
            }
            for resource_id in &stale_items {
                link.accepted_items.remove(resource_id);
                link.accepted_resources.remove(resource_id);
            }
            for (resource_id, content) in &accepted_updates {
                link.accepted_resources
                    .insert(resource_id.clone(), content.clone());
            }
            for (resource_id, item_head) in &accepted_items {
                if let Some(repository) = workspace.cloud_repositories.get(resource_id) {
                    link.cloud_repositories
                        .insert(resource_id.clone(), repository.clone());
                }
                link.accepted_items
                    .insert(resource_id.clone(), item_head.clone());
            }
            if advance_head {
                if let Some(head) = &head {
                    link.accepted_head = Some(head.clone());
                }
            }
            link.revision = link.revision.saturating_add(1);
            link.updated_at = now;
            Ok(())
        });
        if let Err(error) = saved {
            stopped = Some(format!("Applied changes were kept, but local comparison state could not be saved: {error}. Compare again."));
        }
    }
    plan_store::discard(catalog.root(), &plan_id);
    Ok(CatalogScopePullOutcomeDto {
        plan_id,
        remote_accepted: advance_head && stopped.is_none(),
        results,
        stopped,
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
    fn simultaneous_removal_applies_one_review_once() {
        let fixture = library_fixture();
        let scope = skill_scope(&fixture);
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let rows = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .unwrap();
        let plan = service
            .prepare_removal(
                &scope.scope.scope_id,
                rows.rows[0].resource_id.as_ref().unwrap(),
            )
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|threads| {
            let run = || {
                barrier.wait();
                service.apply_removal(&plan.plan_id)
            };
            let first = threads.spawn(run);
            let second = threads.spawn(run);
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_ok_and(|r| r.removed))
                .count(),
            1
        );
        assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
    }

    #[test]
    fn removal_recovery_skips_active_work_and_never_replays_deletion() {
        use super::super::apply::journal_entry;
        use super::super::domain::{
            CatalogOperationId, CatalogOperationKind, CatalogOperationPhase,
        };
        let fixture = library_fixture();
        let scope = skill_scope(&fixture);
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let rows = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .unwrap();
        let plan = service
            .prepare_removal(
                &scope.scope.scope_id,
                rows.rows[0].resource_id.as_ref().unwrap(),
            )
            .unwrap();
        let root = plan.target.decode().unwrap().canonical_root;
        let lock = fixture
            .catalog
            .acquire_target_lock(Path::new(&root))
            .unwrap();
        let mut entry = journal_entry(
            CatalogOperationId::generate().unwrap(),
            CatalogOperationKind::LocalRemoval,
            scope.scope.coordinate(),
            plan.target,
            now_secs(),
        );
        entry.plan_id = Some(plan.plan_id);
        entry.expected_target_sha256 = Some(plan.resource.content_sha256);
        entry.target_path = Some(
            fixture
                .skill_path
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        );
        fixture
            .catalog
            .mutate_operations(|doc| {
                doc.operations.push(entry.clone());
                Ok(())
            })
            .unwrap();
        assert!(
            recover_catalog_operations(&fixture.registry, &fixture.catalog, now_secs())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fixture.catalog.load_operations().unwrap().operations[0].phase,
            CatalogOperationPhase::Journaled
        );
        drop(lock);
        assert_eq!(
            recover_catalog_operations(&fixture.registry, &fixture.catalog, now_secs())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            fixture.catalog.load_operations().unwrap().operations[0].phase,
            CatalogOperationPhase::Failed
        );
        assert!(fixture.skill_path.join("SKILL.md").exists());
    }

    #[test]
    fn plugin_version_change_invalidates_removal_even_when_portable_intent_matches() {
        let fixture = library_fixture();
        write_plugin_fixture(Path::new(&fixture.profile.configured_home));
        let scope = create_scope(
            &fixture.registry,
            &fixture.catalog,
            coordinate(
                "codex",
                "plugin_install_intent",
                "mallard.plugin-install-intent",
                1,
            ),
            &fixture.profile.profile_id,
            2,
        )
        .unwrap();
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let rows = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .unwrap();
        let plan = service
            .prepare_removal(
                &scope.scope.scope_id,
                rows.rows[0].resource_id.as_ref().unwrap(),
            )
            .unwrap();
        let manifest = Path::new(&fixture.profile.configured_home)
            .join("plugins/cache/acme/tools/1/.codex-plugin/plugin.json");
        fs::write(manifest, r#"{"name":"tools","version":"2"}"#).unwrap();
        assert!(service
            .apply_removal(&plan.plan_id)
            .unwrap_err()
            .contains("Plugin state changed"));
        assert!(fixture
            .catalog
            .load_operations()
            .unwrap()
            .operations
            .is_empty());
    }

    #[test]
    fn removal_keeps_storage_and_clears_only_the_exact_local_claims() {
        let fixture = library_fixture();
        let scope = skill_scope(&fixture);
        link_scope(&fixture, &scope.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope.scope.scope_id, &fixture.storage.id);
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let before = service
            .list_scope_resources(&scope.scope.scope_id, Some(&fixture.storage.id))
            .unwrap();
        let id = before.rows[0].resource_id.as_ref().unwrap();
        let plan = service.prepare_removal(&scope.scope.scope_id, id).unwrap();
        fixture
            .catalog
            .mutate_materializations(|doc| {
                for name in ["one", "two"] {
                    doc.claims.push(super::super::domain::CatalogClaim {
                        target: plan.target.clone(),
                        coordinate: scope.scope.coordinate(),
                        storage_id: storage(name),
                        resource_id: id.clone(),
                        content_sha256: plan.resource.content_sha256.clone(),
                        outcome: CatalogActionOutcome::Installed,
                        recorded_at: now_secs(),
                        plan_id: None,
                    });
                }
                let mut other = doc.claims[0].clone();
                other.target = other.target.with_member("other").unwrap();
                doc.claims.push(other);
                Ok(())
            })
            .unwrap();
        let links_before = fixture.catalog.load_scope_links().unwrap();
        let result = service.apply_removal(&plan.plan_id).unwrap();
        assert!(result.removed, "{}", result.message);
        assert!(!fixture.skill_path.exists());
        assert!(service.apply_removal(&plan.plan_id).is_err());
        assert_eq!(
            fixture
                .catalog
                .load_materializations()
                .unwrap()
                .claims
                .len(),
            1
        );
        assert_eq!(fixture.catalog.load_scope_links().unwrap(), links_before);
        let after = service
            .list_scope_resources(&scope.scope.scope_id, Some(&fixture.storage.id))
            .unwrap();
        assert!(!after.rows[0].local_present);
        assert!(after.rows[0].remote_present);
        assert_eq!(after.rows[0].state, Some(CatalogLinkState::LocalMissing));
        assert!(service.prepare_removal(&scope.scope.scope_id, id).is_err());
    }

    #[test]
    fn removal_refuses_stale_expired_and_changed_profile_reviews() {
        let fixture = library_fixture();
        let scope = skill_scope(&fixture);
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let rows = service
            .list_scope_resources(&scope.scope.scope_id, None)
            .unwrap();
        let id = rows.rows[0].resource_id.as_ref().unwrap();
        let plan = service.prepare_removal(&scope.scope.scope_id, id).unwrap();
        fs::write(fixture.skill_path.join("new.txt"), "new file").unwrap();
        assert!(service
            .apply_removal(&plan.plan_id)
            .unwrap_err()
            .contains("changed"));
        assert!(fixture.skill_path.join("SKILL.md").exists());
        let mut plan = service.prepare_removal(&scope.scope.scope_id, id).unwrap();
        plan.created_at = 1;
        plan.expires_at = 2;
        plan_store::save(
            fixture.catalog.root(),
            CatalogResourcePlanDocument::Removal(plan.clone()),
            1,
        )
        .unwrap();
        assert!(service
            .apply_removal(&plan.plan_id)
            .unwrap_err()
            .contains("expired"));
        let plan = service.prepare_removal(&scope.scope.scope_id, id).unwrap();
        service
            .rename_profile(
                &fixture.profile.profile_id,
                "Renamed".to_string(),
                fixture.profile.revision,
            )
            .unwrap();
        assert!(service
            .apply_removal(&plan.plan_id)
            .unwrap_err()
            .contains("changed"));
        assert!(fixture.skill_path.exists());
    }

    #[test]
    fn removal_of_each_pet_version_counts_the_entire_local_folder() {
        use super::super::codex_pet::fixtures::{manifest, webp_atlas};
        for version in [1, 2] {
            let fixture = library_fixture();
            let path = Path::new(&fixture.profile.configured_home).join("pets/duck");
            fs::create_dir_all(path.join("notes/empty")).unwrap();
            fs::write(
                path.join("pet.json"),
                manifest(version, "duck", "Duck", "Test pet", "spritesheet.webp"),
            )
            .unwrap();
            fs::write(path.join("spritesheet.webp"), webp_atlas(version)).unwrap();
            fs::write(path.join("notes/private.txt"), b"extra local content").unwrap();
            let scope = create_scope(
                &fixture.registry,
                &fixture.catalog,
                codex_pets(version),
                &fixture.profile.profile_id,
                2,
            )
            .unwrap();
            let service = StorageCatalogService::from_home_dirs(
                &fixture.user_home,
                &fixture.user_home,
                Box::new(PanicRunner),
            )
            .unwrap();
            let rows = service
                .list_scope_resources(&scope.scope.scope_id, None)
                .unwrap();
            let id = rows.rows[0].resource_id.as_ref().unwrap();
            let plan = service.prepare_removal(&scope.scope.scope_id, id).unwrap();
            assert_eq!(plan.file_count, 3);
            assert!(service.apply_removal(&plan.plan_id).unwrap().removed);
            assert!(!path.exists());
            assert!(fixture.skill_path.exists());
        }
    }

    struct PluginRemover {
        remove: bool,
        success: bool,
    }
    impl PluginProcessRunner for PluginRemover {
        fn run(
            &self,
            provider: &ProviderKey,
            home: &Path,
            argv: &[String],
        ) -> Result<CmdOutput, String> {
            assert_eq!(provider.as_str(), "codex");
            assert_eq!(argv, &["plugin", "remove", "tools@acme"]);
            if self.remove {
                fs::write(home.join("config.toml"), "[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n").unwrap();
            }
            Ok(CmdOutput {
                success: self.success,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn plugin_removal_requires_verified_absence_and_never_replays() {
        for (remove, success) in [(true, true), (false, true), (false, false), (true, false)] {
            let fixture = library_fixture();
            write_plugin_fixture(Path::new(&fixture.profile.configured_home));
            let coordinate = coordinate(
                "codex",
                "plugin_install_intent",
                "mallard.plugin-install-intent",
                1,
            );
            let scope = create_scope(
                &fixture.registry,
                &fixture.catalog,
                coordinate,
                &fixture.profile.profile_id,
                2,
            )
            .unwrap();
            let service = StorageCatalogService::from_home_dirs(
                &fixture.user_home,
                &fixture.user_home,
                Box::new(PluginRemover { remove, success }),
            )
            .unwrap();
            let rows = service
                .list_scope_resources(&scope.scope.scope_id, None)
                .unwrap();
            let plan = service
                .prepare_removal(
                    &scope.scope.scope_id,
                    rows.rows[0].resource_id.as_ref().unwrap(),
                )
                .unwrap();
            assert!(plan.uninstall);
            assert_eq!(plan.file_count, 0);
            let result = service.apply_removal(&plan.plan_id).unwrap();
            assert_eq!(result.removed, remove && success);
            assert!(service.apply_removal(&plan.plan_id).is_err());
            assert!(fixture.skill_path.exists());
        }
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

    #[test]
    fn probing_a_home_names_what_set_up_would_save() {
        let fixture = library_fixture();
        // The saved home answers with its profile and its name.
        let saved = probe_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            fixture.profile.configured_home.clone(),
        )
        .unwrap();
        assert_eq!(
            saved.existing_profile_id.as_ref(),
            Some(&fixture.profile.profile_id)
        );
        assert_eq!(saved.suggested_name, fixture.profile.display_name);
        assert!(saved.readable);
        assert_eq!(saved.canonical_path, fixture.profile.configured_home);

        // A folder with no profile yet answers with the name it would get.
        let other = fixture.user_home.join("other-home");
        fs::create_dir_all(&other).unwrap();
        let fresh = probe_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            other.to_string_lossy().to_string(),
        )
        .unwrap();
        assert_eq!(fresh.existing_profile_id, None);
        assert_eq!(fresh.suggested_name, "other-home");
        assert_eq!(
            fresh.canonical_path,
            fs::canonicalize(&other).unwrap().to_string_lossy()
        );

        // A missing folder is a miss, said the way the hint expects.
        let missing = probe_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("codex"),
            fixture.user_home.join("nope").to_string_lossy().to_string(),
        )
        .unwrap_err();
        assert!(missing.contains("No such file or directory"), "{missing}");

        // An unknown provider never resolves. (The Mallard state root is
        // refused by `validate_new_profile_home`, the save's own check,
        // tested beside it.)
        assert!(probe_catalog_resource_profile_with_repository(
            &fixture.registry,
            &fixture.catalog,
            provider("cursor"),
            other.to_string_lossy().to_string(),
        )
        .is_err());
    }

    #[test]
    fn inspecting_a_home_counts_each_capability_and_names_the_last_storage() {
        let fixture = library_fixture();
        let service = StorageCatalogService::from_home_dirs(
            &fixture.user_home,
            &fixture.user_home,
            Box::new(PanicRunner),
        )
        .unwrap();
        let before = service
            .inspect_profile(&fixture.profile.profile_id)
            .unwrap();
        assert_eq!(before.profile.profile_id, fixture.profile.profile_id);
        assert_eq!(before.last_used_storage_id, None);
        // Every capability the provider registers, in the registry's order.
        assert_eq!(
            before
                .holdings
                .iter()
                .map(|holding| holding.capability.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Skills", "Plugins", "Pets v1", "Pets v2"]
        );
        let skills = &before.holdings[0];
        assert_eq!(skills.count, Some(1));
        assert!(skills.newest_modified_at.is_some());
        // Roots are the adapter's summaries, never paths.
        assert_eq!(skills.roots.len(), 1);
        assert!(!skills.roots[0].contains(&fixture.profile.configured_home));
        assert_eq!(skills.scope_id, None);

        // A row for Skills, linked to the fixture's storage: the inspection
        // joins the row and names the storage as the one used last.
        let scope = service
            .create_scope(
                provider("codex"),
                fixture.profile.profile_id.clone(),
                codex_skills().resource_kind,
                codex_skills().format,
                1,
            )
            .unwrap()
            .scope
            .scope_id;
        service
            .link_scope_storage(&scope, &fixture.storage.id)
            .unwrap();
        let after = service
            .inspect_profile(&fixture.profile.profile_id)
            .unwrap();
        assert_eq!(after.holdings[0].scope_id.as_ref(), Some(&scope));
        assert_eq!(
            after.last_used_storage_id.as_ref(),
            Some(&fixture.storage.id)
        );
        assert_eq!(after.holdings[0].count, Some(1));

        // An unknown profile is refused.
        assert!(service
            .inspect_profile(&LocalCatalogProfileId::generate().unwrap())
            .is_err());
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
            &|_| {},
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
            &|_| {},
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
    fn a_tag_without_a_publication_remains_a_refusal_in_batch_discovery() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &row.scope.scope_id, &fixture.storage.id);
        let repository = CatalogResourceRepositoryId::for_coordinate(&codex_skills());
        fs::remove_dir_all(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(repository.as_str())
                .join("_commits"),
        )
        .unwrap();
        let listed = list_catalog_storage_repositories_with_repository(
            &fixture.registry,
            &fixture.state,
            &fixture.storage.id,
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].repository_id, repository.as_str());
        assert!(listed[0]
            .error
            .as_deref()
            .unwrap()
            .contains("discovery tag but no publication"));
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

    fn stored_records(fixture: &LibraryFixture, repository_id: &str) -> usize {
        fs::read_dir(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(repository_id)
                .join("_commits"),
        )
        .map(|entries| entries.count())
        .unwrap_or(0)
    }

    #[test]
    fn a_second_apply_of_one_review_is_stopped_and_one_record_is_written() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        link_scope(&fixture, &row.scope.scope_id, &fixture.storage.id, 3);
        let plan = prepare_push(&fixture, &row.scope.scope_id, &fixture.storage.id);
        let selected = vec![resource_id_of(&plan, "review")];
        let repository_id = plan.repository_id.as_str().to_string();

        // The first Apply is still running: the second is refused at once,
        // before it lists the storage, and writes nothing.
        let running = fixture
            .catalog
            .acquire_publication_lock(&plan.storage_id, &plan.repository_id)
            .unwrap();
        let error = apply_push(&fixture, &plan, selected.clone()).unwrap_err();
        assert!(error.contains("already in progress"), "{error}");
        assert_eq!(stored_records(&fixture, &repository_id), 0);
        drop(running);

        assert!(
            apply_push(&fixture, &plan, selected.clone())
                .unwrap()
                .published
        );
        // The review was consumed by the Apply that ran, so a repeat has
        // nothing to apply; exactly one record exists.
        assert!(apply_push(&fixture, &plan, selected).is_err());
        assert_eq!(stored_records(&fixture, &repository_id), 1);
    }

    /// Another computer's publication on the repository's first record: the
    /// same resources under its own commit id, in a record that claims the
    /// millisecond after its parent. With one record it is a rival of the
    /// next push; with more, it outranks the second publication.
    fn plant_rival_library_publication(repository: &Path) {
        use super::super::domain::{
            library_manifest_relative_key, sha256_hex, CatalogCommitId, LibraryResourceCommit,
            LibraryResourceManifest,
        };
        let parent_file = fs::read_dir(repository.join("_commits"))
            .unwrap()
            .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
            .filter(|name| mallard_domain::RecordName::parse(name).is_some())
            .min_by_key(|name| mallard_domain::RecordName::parse(name).unwrap().revision())
            .unwrap();
        let parent_name = mallard_domain::RecordName::parse(&parent_file).unwrap();
        let mut commit: LibraryResourceCommit = serde_json::from_slice(
            &fs::read(repository.join("_commits").join(&parent_file)).unwrap(),
        )
        .unwrap();
        let mut manifest: LibraryResourceManifest =
            serde_json::from_slice(&fs::read(repository.join(&commit.manifest_key)).unwrap())
                .unwrap();
        let rival = CatalogCommitId::generate().unwrap();
        manifest.revision += 1;
        manifest.commit_id = rival.clone();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_key = library_manifest_relative_key(manifest.revision, &rival);
        fs::write(repository.join(&manifest_key), &manifest_bytes).unwrap();
        commit.previous_commit_id = Some(commit.commit_id.clone());
        commit.previous_record_sha256 = Some(parent_name.token().record_sha256().to_string());
        commit.revision = manifest.revision;
        commit.commit_id = rival.clone();
        commit.created_at_ms += 1;
        commit.manifest_key = manifest_key;
        commit.manifest_sha256 = sha256_hex(&manifest_bytes);
        commit.manifest_size = manifest_bytes.len() as u64;
        let record_bytes = serde_json::to_vec(&commit).unwrap();
        let name = mallard_domain::RecordName::new(
            commit.revision,
            mallard_domain::PublicationToken::new(rival.as_str(), sha256_hex(&record_bytes))
                .unwrap(),
            Some(parent_name.token().clone()),
            commit.created_at_ms,
        )
        .unwrap();
        fs::write(
            repository.join("_commits").join(name.file_name()),
            &record_bytes,
        )
        .unwrap();
    }

    #[test]
    fn a_push_that_loses_the_fork_window_fails_and_accepts_nothing() {
        use mallard_infrastructure::storage_core::publication::{test_support, REMOTE_CHANGED};
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let accepted = link_of(&fixture);
        assert!(accepted.accepted_head.is_some());
        let repository_id = accepted.repository_id.as_str().to_string();

        // A second skill is pushed while another computer pushes on the same
        // version. The other one's record ranks first and lands between this
        // push's listing and its write.
        let second = fixture.skill_path.parent().unwrap().join("second");
        fs::create_dir_all(&second).unwrap();
        fs::write(
            second.join("SKILL.md"),
            b"---\nname: second\n---\nA second skill.\n",
        )
        .unwrap();
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        let selected = vec![resource_id_of(&plan, "second")];
        std::thread::sleep(std::time::Duration::from_millis(5));
        test_support::before_next_record_request({
            let repository = fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id);
            move || plant_rival_library_publication(&repository)
        });
        let error = apply_push(&fixture, &plan, selected).unwrap_err();

        // The push failed as a whole: both records are in the storage, and
        // the link accepted nothing.
        assert_eq!(error, REMOTE_CHANGED);
        assert_eq!(stored_records(&fixture, &repository_id), 3);
        assert_eq!(link_of(&fixture), accepted);

        // The accepted head is an ancestor of the selected publication, so
        // nothing is reset. The next review offers the row again, and the
        // push lands on the selected publication.
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        assert_eq!(link_of(&fixture), accepted);
        let offered = plan.push_row(&resource_id_of(&plan, "second")).unwrap();
        assert!(offered.row.selectable, "{:?}", offered.row.blocked_reason);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let (head, manifest) = engine_for(&fixture.storage)
            .unwrap()
            .inspect(&plan.repository_id)
            .unwrap()
            .unwrap();
        assert_eq!(head.head.revision, 3);
        assert_eq!(manifest.resources.len(), 2);
        assert_eq!(stored_records(&fixture, &repository_id), 4);
    }

    /// B2, and the Push half of the save rule (cleared plan, section 4.5):
    /// the repository was cleared and another computer pushed its own skill.
    /// The accepted state names a history the repository no longer has, so
    /// the page reads it as absent and writes nothing: both of this
    /// computer's skills read as new. A Push of one of them drops every
    /// saved acceptance and keeps only its own, so the other one still reads
    /// as new afterwards instead of counting as accepted on a history the
    /// storage no longer has.
    #[test]
    fn a_recreated_library_repository_reads_the_link_as_absent_and_a_push_keeps_its_own_acceptance()
    {
        let fixture = library_fixture();
        let notes = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let state_of = |fixture: &LibraryFixture, name: &str| {
            page(fixture, &scope_id, Some(&fixture.storage.id))
                .unwrap()
                .rows
                .into_iter()
                .find(|row| row.display_name == name)
                .unwrap_or_else(|| panic!("no row named '{name}'"))
                .state
        };
        let accepted = link_of(&fixture);
        assert!(accepted.accepted_head.is_some());
        assert_eq!(accepted.accepted_resources.len(), 2);
        let repository_id = accepted.repository_id.as_str().to_string();

        // The repository is cleared and another computer pushes its own
        // skill: a new history that does not contain this computer's push.
        fs::remove_dir_all(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id),
        )
        .unwrap();
        let other = sibling_machine(&fixture, "other-user", b"unused");
        fs::remove_dir_all(&other.skill_path).unwrap();
        let extra = other.user_home.join("codex-home/skills/extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(
            extra.join("SKILL.md"),
            b"---\nname: extra\n---\nAnother computer's skill.\n",
        )
        .unwrap();
        let other_row = skill_scope(&other);
        link_scope(&other, &other_row.scope.scope_id, &other.storage.id, 4);
        push_all(&other, &other_row.scope.scope_id, &other.storage.id);

        // The page loads and writes nothing, and reads the rows without the
        // stale acceptances.
        assert_eq!(state_of(&fixture, "review"), Some(CatalogLinkState::New));
        assert_eq!(state_of(&fixture, "notes"), Some(CatalogLinkState::New));
        assert_eq!(
            state_of(&fixture, "extra"),
            Some(CatalogLinkState::LocalMissing)
        );
        assert_eq!(link_of(&fixture), accepted);

        // One of the two skills is pushed, onto the selected history.
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        for name in ["review", "notes"] {
            let offered = plan.push_row(&resource_id_of(&plan, name)).unwrap();
            assert!(offered.row.selectable, "{:?}", offered.row.blocked_reason);
        }
        let review_id = resource_id_of(&plan, "review");
        let outcome = apply_push(&fixture, &plan, vec![review_id.clone()]).unwrap();
        assert!(outcome.published, "{}", outcome.message);
        let engine = engine_for(&fixture.storage).unwrap();
        let (head, manifest) = engine.inspect(&plan.repository_id).unwrap().unwrap();
        assert_eq!(head.head.revision, 2);
        assert_eq!(manifest.resources.len(), 2);

        // The link accepted the new history and the skill it pushed, and
        // nothing else: the other skill's acceptance from the cleared history
        // is gone, so it still reads as new.
        let saved = link_of(&fixture);
        assert!(saved.accepted_head.is_some());
        assert_eq!(
            saved.accepted_resources.keys().cloned().collect::<Vec<_>>(),
            vec![review_id]
        );
        assert_eq!(state_of(&fixture, "review"), Some(CatalogLinkState::InSync));
        assert_eq!(state_of(&fixture, "notes"), Some(CatalogLinkState::New));
        assert_eq!(
            state_of(&fixture, "extra"),
            Some(CatalogLinkState::LocalMissing)
        );
    }

    /// B3, no longer a refusal: an accepted head whose storage identity is
    /// not this storage's describes a history this storage does not hold,
    /// and is read like a cleared repository's. The page loads and writes
    /// nothing, a skill this computer changed compares as unreviewed instead
    /// of as changed here, and the next Push saves a fresh accepted state
    /// under this storage's identity. The refusal this replaces left the page
    /// unable to load, and with it Unlink disabled and the row's deletion and
    /// the storage's removal refused. A head that names another repository
    /// never reaches the page: the links document refuses it.
    #[test]
    fn an_accepted_head_of_another_storage_or_repository_reads_as_absent_and_the_next_push_replaces_it(
    ) {
        use super::super::domain::{
            CatalogCoordinate, CatalogResourceRepositoryId, LibraryHeadCoordinate,
        };
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let state_of = |fixture: &LibraryFixture, name: &str| {
            page(fixture, &scope_id, Some(&fixture.storage.id))
                .unwrap()
                .rows
                .into_iter()
                .find(|row| row.display_name == name)
                .unwrap_or_else(|| panic!("no row named '{name}'"))
                .state
        };
        let accepted = link_of(&fixture);
        let identity = storage_identity_for_config(&fixture.storage).unwrap();
        assert_eq!(
            accepted
                .accepted_head
                .as_ref()
                .map(|head| head.storage_identity_sha256.as_str()),
            Some(identity.as_str())
        );
        // The skill changes here: with its acceptance it reads changed here.
        fs::write(
            fixture.skill_path.join("SKILL.md"),
            b"---\nname: review\n---\nChanged here.\n",
        )
        .unwrap();
        assert_eq!(
            state_of(&fixture, "review"),
            Some(CatalogLinkState::LocalChanged)
        );
        let edit_head = |fixture: &LibraryFixture, edit: &dyn Fn(&mut LibraryHeadCoordinate)| {
            fixture
                .catalog
                .mutate_scope_links(|document| {
                    let link = document
                        .links
                        .iter_mut()
                        .find(|link| link.link_id == accepted.link_id)
                        .expect("the link");
                    edit(link.accepted_head.as_mut().expect("an accepted head"));
                    Ok(())
                })
                .unwrap();
        };

        // Another storage's identity on the accepted head: the page loads,
        // writes nothing, and reads the acceptance as absent, so the changed
        // skill compares as unreviewed.
        edit_head(&fixture, &|head| {
            head.storage_identity_sha256 = "b".repeat(64)
        });
        let foreign_identity = link_of(&fixture);
        assert_eq!(
            state_of(&fixture, "review"),
            Some(CatalogLinkState::UnreviewedRemote)
        );
        assert_eq!(link_of(&fixture), foreign_identity);

        // An accepted head that names another repository cannot exist in a
        // loadable links document: the document refuses it at save and at
        // load, so the classifier's reading of it as absent is a second line
        // that never fires. Nothing is written by the refused edit.
        let refused = fixture.catalog.mutate_scope_links(|document| {
            let link = document
                .links
                .iter_mut()
                .find(|link| link.link_id == accepted.link_id)
                .expect("the link");
            link.accepted_head
                .as_mut()
                .expect("an accepted head")
                .repository_id = CatalogResourceRepositoryId::for_coordinate(
                &CatalogCoordinate::new("codex", "agent_skill", "agentskills.io.skill", 2).unwrap(),
            );
            Ok(())
        });
        assert!(refused
            .unwrap_err()
            .contains("accepted a different repository"));
        assert_eq!(link_of(&fixture), foreign_identity);

        // A new skill is pushed. The changed one is not pushable without a
        // Pull, because no base says this computer's copy came from the
        // stored one. The saved acceptance is this storage's and this
        // push's, and nothing of the foreign head remains.
        let notes = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        let notes_id = resource_id_of(&plan, "notes");
        let review_id = resource_id_of(&plan, "review");
        assert!(plan.push_row(&notes_id).unwrap().row.selectable);
        assert!(!plan.push_row(&review_id).unwrap().row.selectable);
        let outcome = apply_push(&fixture, &plan, vec![notes_id.clone()]).unwrap();
        assert!(outcome.published, "{}", outcome.message);
        let saved = link_of(&fixture);
        let saved_head = saved.accepted_head.expect("the push accepted a head");
        assert_eq!(saved_head.storage_identity_sha256, identity);
        assert_eq!(saved_head.repository_id, accepted.repository_id);
        assert_eq!(saved_head.revision, 2);
        assert_eq!(
            saved.accepted_resources.keys().cloned().collect::<Vec<_>>(),
            vec![notes_id]
        );
        assert_eq!(state_of(&fixture, "notes"), Some(CatalogLinkState::InSync));
        assert_eq!(
            state_of(&fixture, "review"),
            Some(CatalogLinkState::UnreviewedRemote)
        );
    }

    /// A displaced accepted head: another computer's record on the same
    /// parent, ranked first, shows up after this computer's second push was
    /// confirmed. The accepted head is off the selected chain, so the page
    /// reads the accepted state as absent and writes nothing, the Library's
    /// counterpart of a displaced project push. The skill the selected
    /// version also holds reads in sync by its content, the skill only this
    /// computer has reads as new, and pushing it saves the selected history
    /// with that one acceptance; the displaced record stays in the storage.
    #[test]
    fn a_displaced_library_push_reads_the_link_as_absent_and_is_pushed_again() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let notes = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        // The second record must rank behind a rival stamped one millisecond
        // after the first.
        std::thread::sleep(std::time::Duration::from_millis(5));
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let state_of = |fixture: &LibraryFixture, name: &str| {
            page(fixture, &scope_id, Some(&fixture.storage.id))
                .unwrap()
                .rows
                .into_iter()
                .find(|row| row.display_name == name)
                .unwrap_or_else(|| panic!("no row named '{name}'"))
                .state
        };
        let accepted = link_of(&fixture);
        assert_eq!(
            accepted.accepted_head.as_ref().map(|head| head.revision),
            Some(2)
        );
        assert_eq!(accepted.accepted_resources.len(), 2);
        let repository_id = accepted.repository_id.as_str().to_string();
        plant_rival_library_publication(
            &fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id),
        );
        assert_eq!(stored_records(&fixture, &repository_id), 3);

        // The page loads, writes nothing, and reads the rows without the
        // displaced acceptances.
        assert_eq!(state_of(&fixture, "review"), Some(CatalogLinkState::InSync));
        assert_eq!(state_of(&fixture, "notes"), Some(CatalogLinkState::New));
        assert_eq!(link_of(&fixture), accepted);

        // Pushing the skill again lands on the selected history, with the
        // acceptance of that one skill.
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        let notes_id = resource_id_of(&plan, "notes");
        let offered = plan.push_row(&notes_id).unwrap();
        assert!(offered.row.selectable, "{:?}", offered.row.blocked_reason);
        let outcome = apply_push(&fixture, &plan, vec![notes_id.clone()]).unwrap();
        assert!(outcome.published, "{}", outcome.message);
        let engine = engine_for(&fixture.storage).unwrap();
        let (head, manifest) = engine.inspect(&plan.repository_id).unwrap().unwrap();
        assert_eq!(head.head.revision, 3);
        assert_eq!(manifest.resources.len(), 2);
        assert_eq!(stored_records(&fixture, &repository_id), 4);
        let saved = link_of(&fixture);
        assert_eq!(
            saved.accepted_head.as_ref().map(|head| head.revision),
            Some(3)
        );
        assert_eq!(
            saved.accepted_resources.keys().cloned().collect::<Vec<_>>(),
            vec![notes_id]
        );
        assert_eq!(state_of(&fixture, "review"), Some(CatalogLinkState::InSync));
        assert_eq!(state_of(&fixture, "notes"), Some(CatalogLinkState::InSync));
    }

    /// The Pull half of the save rule (cleared plan, section 4.5), from B2:
    /// the repository was cleared and another computer pushed its own skill.
    /// A Pull that installs it saves the head it reviewed with that one
    /// acceptance, and drops the acceptance this computer's own skill had on
    /// the cleared history. Afterwards the installed skill reads in sync,
    /// and the own skill still reads as new, ready to push.
    #[test]
    fn a_pull_from_an_accepted_state_read_as_absent_saves_the_reviewed_head_and_what_it_accepted() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let state_of = |fixture: &LibraryFixture, name: &str| {
            page(fixture, &scope_id, Some(&fixture.storage.id))
                .unwrap()
                .rows
                .into_iter()
                .find(|row| row.display_name == name)
                .unwrap_or_else(|| panic!("no row named '{name}'"))
                .state
        };
        let accepted = link_of(&fixture);
        let repository_id = accepted.repository_id.as_str().to_string();
        fs::remove_dir_all(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id),
        )
        .unwrap();
        let other = sibling_machine(&fixture, "other-user", b"unused");
        fs::remove_dir_all(&other.skill_path).unwrap();
        let extra = other.user_home.join("codex-home/skills/extra");
        fs::create_dir_all(&extra).unwrap();
        fs::write(
            extra.join("SKILL.md"),
            b"---\nname: extra\n---\nAnother computer's skill.\n",
        )
        .unwrap();
        let other_row = skill_scope(&other);
        link_scope(&other, &other_row.scope.scope_id, &other.storage.id, 4);
        push_all(&other, &other_row.scope.scope_id, &other.storage.id);
        assert_eq!(link_of(&fixture), accepted);

        // The Pull installs the other computer's skill.
        let plan = prepare_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &scope_id,
            &fixture.storage.id,
            BTreeMap::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap();
        let item = plan
            .items
            .iter()
            .find(|item| item.item.display_name == "extra")
            .expect("the other computer's skill is offered");
        let extra_id = item.item.resource_id.clone();
        let outcome = apply_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            plan.plan_id.clone(),
            BTreeMap::from([(
                extra_id.clone(),
                item.item
                    .default_action_id
                    .clone()
                    .expect("an install action"),
            )]),
            Vec::new(),
            &fixture.user_home,
            now_secs(),
            &|_| {},
        )
        .unwrap();
        assert!(outcome.remote_accepted);
        assert!(fixture
            .user_home
            .join("codex-home/skills/extra/SKILL.md")
            .exists());

        // The link is on the new history with the one acceptance the Pull
        // made; the own skill's acceptance from the cleared history is gone.
        let saved = link_of(&fixture);
        assert_eq!(
            saved.accepted_head.as_ref().map(|head| head.revision),
            Some(1)
        );
        assert_ne!(saved.accepted_head, accepted.accepted_head);
        assert_eq!(
            saved.accepted_resources.keys().cloned().collect::<Vec<_>>(),
            vec![extra_id]
        );
        assert_eq!(state_of(&fixture, "extra"), Some(CatalogLinkState::InSync));
        assert_eq!(state_of(&fixture, "review"), Some(CatalogLinkState::New));
        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        let offered = plan.push_row(&resource_id_of(&plan, "review")).unwrap();
        assert!(offered.row.selectable, "{:?}", offered.row.blocked_reason);
    }

    /// The Pull review of one scope on the fixture's storage.
    fn prepare_pull(fixture: &LibraryFixture, scope_id: &CatalogScopeId) -> CatalogScopePullPlan {
        prepare_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            scope_id,
            &fixture.storage.id,
            BTreeMap::new(),
            &fixture.user_home,
            now_secs(),
        )
        .unwrap()
    }

    /// Apply `plan` with every item's default action.
    fn apply_pull_defaults(
        fixture: &LibraryFixture,
        plan: &CatalogScopePullPlan,
    ) -> Result<CatalogScopePullOutcomeDto, String> {
        let choices = plan
            .items
            .iter()
            .filter_map(|item| {
                Some((
                    item.item.resource_id.clone(),
                    item.item.default_action_id.clone()?,
                ))
            })
            .collect();
        apply_pull_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &PanicRunner,
            plan.plan_id.clone(),
            choices,
            Vec::new(),
            &fixture.user_home,
            now_secs(),
            &|_| {},
        )
    }

    /// A second computer on the same storage, without skills of its own,
    /// linked to the scope the fixture pushed.
    fn puller(fixture: &LibraryFixture) -> (LibraryFixture, CatalogScopeId) {
        let other = sibling_machine(fixture, "other-user", b"unused");
        fs::remove_dir_all(&other.skill_path).unwrap();
        let row = skill_scope(&other);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&other, &scope_id, &other.storage.id, 4);
        (other, scope_id)
    }

    fn link_of(fixture: &LibraryFixture, scope_id: &CatalogScopeId) -> Option<CatalogScopeLink> {
        fixture
            .catalog
            .load_scope_links()
            .unwrap()
            .link_for(scope_id, &fixture.storage.id)
            .cloned()
    }

    /// G1: another computer pushed a newer version between the review and
    /// the Pull. The Pull installs the reviewed resources and saves the
    /// reviewed version as accepted; the next page reads the row the newer
    /// version changed as changed in the storage.
    #[test]
    fn a_library_pull_applies_the_version_it_reviewed_when_a_newer_one_was_pushed_since() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let (other, other_scope) = puller(&fixture);
        let plan = prepare_pull(&other, &other_scope);
        assert_eq!(plan.head.as_ref().unwrap().revision, 1);

        // Version 2: the skill revised on the first computer.
        fs::write(
            fixture.skill_path.join("SKILL.md"),
            b"---\nname: review\n---\nDraft-linked skill, revised.\n",
        )
        .unwrap();
        push_all(&fixture, &scope_id, &fixture.storage.id);

        let outcome = apply_pull_defaults(&other, &plan).unwrap();
        assert!(outcome.remote_accepted);
        assert_eq!(outcome.stopped, None);
        assert_eq!(
            fs::read(other.skill_path.join("SKILL.md")).unwrap(),
            b"---\nname: review\n---\nDraft-linked skill.\n"
        );
        let link = link_of(&other, &other_scope).unwrap();
        assert_eq!(
            link.accepted_head.as_ref().map(|head| head.revision),
            Some(1)
        );
        let loaded = page(&other, &other_scope, Some(&other.storage.id)).unwrap();
        assert_eq!(loaded.rows.len(), 1);
        assert_eq!(loaded.rows[0].state, Some(CatalogLinkState::StorageChanged));
    }

    /// G2: the reviewed version lost its race, and then the repository was
    /// cleared, before the Pull starts. Both refuse the Pull before anything
    /// changes: no local file and no link.
    #[test]
    fn a_library_pull_is_refused_before_anything_changes_when_its_reviewed_version_is_off_the_chain(
    ) {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let notes = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let repository_id = link_of(&fixture, &scope_id)
            .unwrap()
            .repository_id
            .as_str()
            .to_string();
        let repository = fixture
            .storage_path
            .join(".mallard/v1/library")
            .join(&repository_id);

        let (other, other_scope) = puller(&fixture);
        let plan = prepare_pull(&other, &other_scope);
        assert_eq!(plan.head.as_ref().unwrap().revision, 2);
        let link_before = link_of(&other, &other_scope);

        // The reviewed version loses its race to a record stamped earlier.
        plant_rival_library_publication(&repository);
        let error = apply_pull_defaults(&other, &plan).unwrap_err();
        assert_eq!(error, LIBRARY_REVIEW_SUPERSEDED);
        assert!(!other.skill_path.exists());
        assert_eq!(link_of(&other, &other_scope), link_before);

        // The repository is cleared after a new review.
        let plan = prepare_pull(&other, &other_scope);
        fs::remove_dir_all(&repository).unwrap();
        let error = apply_pull_defaults(&other, &plan).unwrap_err();
        assert_eq!(error, LIBRARY_REVIEW_SUPERSEDED);
        assert!(!other.skill_path.exists());
        assert_eq!(link_of(&other, &other_scope), link_before);
    }

    /// G3: the repository lost a payload after the review, or a synced
    /// folder has not replicated it yet. The Pull installs the resources
    /// before it, stops at the first payload it cannot read, reads the rest
    /// as not pulled, and does not advance the accepted version.
    #[test]
    fn a_library_pull_stops_at_the_first_payload_it_cannot_read() {
        let fixture = library_fixture();
        let notes = fixture.user_home.join("codex-home/skills/notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("SKILL.md"),
            b"---\nname: notes\n---\nSecond skill.\n",
        )
        .unwrap();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let repository_id = link_of(&fixture, &scope_id)
            .unwrap()
            .repository_id
            .as_str()
            .to_string();

        let (other, other_scope) = puller(&fixture);
        let plan = prepare_pull(&other, &other_scope);
        assert_eq!(plan.items.len(), 2);
        // The payload of the resource the apply reaches second is gone.
        let second = plan.items[1].resource.as_ref().expect("a stored resource");
        fs::remove_file(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id)
                .join("_objects")
                .join(&second.files[0].sha256),
        )
        .unwrap();

        let outcome = apply_pull_defaults(&other, &plan).unwrap();
        assert_eq!(
            outcome.stopped.as_deref(),
            Some("Part of this version is missing from Library test storage. Compare again.")
        );
        assert!(!outcome.remote_accepted);
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(outcome.results[0].outcome, CatalogActionOutcome::Installed);
        assert_eq!(outcome.results[1].outcome, CatalogActionOutcome::Failed);
        assert_eq!(outcome.results[1].message.as_deref(), Some("Not pulled"));
        let first = &plan.items[0].item.display_name;
        assert!(other
            .user_home
            .join(format!("codex-home/skills/{first}/SKILL.md"))
            .exists());
        assert!(!other
            .user_home
            .join(format!(
                "codex-home/skills/{}/SKILL.md",
                plan.items[1].item.display_name
            ))
            .exists());
        assert!(!other
            .catalog
            .load_materializations()
            .unwrap()
            .claims
            .is_empty());
        let link = link_of(&other, &other_scope).unwrap();
        assert!(link.accepted_head.is_none());
        assert_eq!(link.accepted_resources.len(), 1);
    }

    /// B1: the repository was cleared, and nobody has pushed since. The page
    /// loads instead of refusing and writes nothing: the accepted state is
    /// read as absent, the local row reads as new and is selectable, and one
    /// Push repopulates the repository from its first version.
    #[test]
    fn a_cleared_library_repository_loads_and_one_push_repopulates_it() {
        let fixture = library_fixture();
        let row = skill_scope(&fixture);
        let scope_id = row.scope.scope_id.clone();
        link_scope(&fixture, &scope_id, &fixture.storage.id, 3);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let link_of = |fixture: &LibraryFixture| {
            fixture
                .catalog
                .load_scope_links()
                .unwrap()
                .link_for(&scope_id, &fixture.storage.id)
                .cloned()
                .unwrap()
        };
        let accepted = link_of(&fixture);
        assert!(accepted.accepted_head.is_some());
        let repository_id = accepted.repository_id.as_str().to_string();

        fs::remove_dir_all(
            fixture
                .storage_path
                .join(".mallard/v1/library")
                .join(&repository_id),
        )
        .unwrap();
        assert_eq!(stored_records(&fixture, &repository_id), 0);

        let loaded = page(&fixture, &scope_id, Some(&fixture.storage.id)).unwrap();
        assert_eq!(loaded.rows.len(), 1);
        assert_eq!(loaded.rows[0].state, Some(CatalogLinkState::New));
        assert_eq!(link_of(&fixture), accepted);

        let plan = prepare_push(&fixture, &scope_id, &fixture.storage.id);
        let review = plan.push_row(&resource_id_of(&plan, "review")).unwrap();
        assert!(review.row.selectable, "{:?}", review.row.blocked_reason);
        push_all(&fixture, &scope_id, &fixture.storage.id);
        let engine = engine_for(&fixture.storage).unwrap();
        let (head, manifest) = engine.inspect(&plan.repository_id).unwrap().unwrap();
        assert_eq!(head.head.revision, 1);
        assert_eq!(manifest.resources.len(), 1);
        assert_eq!(stored_records(&fixture, &repository_id), 1);
        let saved = link_of(&fixture);
        assert!(saved.accepted_head.is_some());
        assert_eq!(saved.accepted_resources.len(), 1);
        assert_eq!(
            page(&fixture, &scope_id, Some(&fixture.storage.id))
                .unwrap()
                .rows[0]
                .state,
            Some(CatalogLinkState::InSync)
        );
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
    fn a_plugin_page_prints_only_the_adapters_own_plain_install_lines() {
        use super::super::plugin_intent::{
            GitMarketplaceSource, PluginInstallIntent, PluginInstallIntentDescriptor,
            PluginInstallIntentIdentity, PluginResolutionPolicy, PortablePluginSource,
        };
        let registry = registry();
        let lines = |provider: &str, location: Option<&str>, git_ref: Option<&str>| {
            let provider = ProviderKey::parse(provider).unwrap();
            let envelope = PluginInstallIntent {
                identity: PluginInstallIntentIdentity {
                    namespace: "acme".to_string(),
                    plugin_id: "tools@acme".to_string(),
                },
                descriptor: PluginInstallIntentDescriptor {
                    source: PortablePluginSource::ProviderMarketplace {
                        namespace: "acme".to_string(),
                        git: location.map(|location| GitMarketplaceSource {
                            location: location.to_string(),
                            git_ref: git_ref.map(str::to_string),
                        }),
                    },
                    resolution: if location.is_some() {
                        PluginResolutionPolicy::ProviderResolved
                    } else {
                        PluginResolutionPolicy::ManualOnly
                    },
                },
            }
            .envelope(&provider)
            .unwrap();
            let capability = registry.capability(&envelope.coordinate()).unwrap();
            let mut detail = capability
                .codec
                .detail(&envelope, &|_, _| Err("no payloads".to_string()))
                .unwrap();
            fill_install_lines(&mut detail, &capability, &envelope);
            let CatalogResourceDetail::Install { install, .. } = detail else {
                panic!("a plugin's page is an install");
            };
            install
        };
        // The argv a Pull runs into an empty home, behind the provider's CLI.
        assert_eq!(
            lines("claude", Some("acme/tools"), None),
            vec![
                "claude plugin marketplace add acme/tools --scope user",
                "claude plugin install tools@acme --scope user",
            ]
        );
        assert_eq!(
            lines("codex", Some("https://example.com/acme/tools"), None),
            vec![
                "codex plugin marketplace add https://example.com/acme/tools",
                "codex plugin add tools@acme",
            ]
        );
        // The adapter refuses a ref it cannot bind and a marketplace that is
        // not portable; a word a shell would read twice is never printed.
        assert!(lines("claude", Some("acme/tools"), Some("v1")).is_empty());
        assert!(lines("claude", None, None).is_empty());
        assert!(lines("codex", Some("https://example.com/a;touch-x"), None).is_empty());
    }

    #[test]
    fn a_local_skill_page_reads_this_macs_copy_without_asking_the_storage() {
        let fixture = library_fixture();
        let skills = skill_scope(&fixture);
        let listed = page(&fixture, &skills.scope.scope_id, None).unwrap();
        let resource_id = listed.rows[0].resource_id.clone().unwrap();
        // The storage is gone; this Mac's copy is all the page needs.
        fs::remove_dir_all(&fixture.storage_path).unwrap();
        let CatalogResourceDetail::Document { path, text, files } =
            get_catalog_resource_detail_with_repositories(
                &fixture.registry,
                &fixture.state,
                &fixture.catalog,
                &skills.scope.scope_id,
                Some(&fixture.storage.id),
                &resource_id,
                &fixture.user_home,
            )
            .unwrap()
        else {
            panic!("a skill's page is a document");
        };
        assert_eq!(path, "SKILL.md");
        assert_eq!(
            text.as_deref(),
            Some("---\nname: review\n---\nDraft-linked skill.\n")
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "SKILL.md");
        assert!(!files[0].executable);
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
                tag: LibraryResourceTag::new(&codex_pets(2), repository_id.clone()),
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

        // Its page reads the same stored copy: the whole atlas, verified, with
        // the version's grid. Without a storage there is nothing to read.
        let CatalogResourceDetail::Sprite {
            atlas: served,
            columns,
            rows,
            description,
        } = get_catalog_resource_detail_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            Some(&fixture.storage.id),
            &resource_id,
            &fixture.user_home,
        )
        .unwrap()
        else {
            panic!("a pet's page is a sprite");
        };
        assert!(served.starts_with("data:image/png;base64,"), "{}", served);
        assert_eq!((columns, rows), (8, 11));
        assert_eq!(description.as_deref(), Some("A stored-only pet."));
        assert!(get_catalog_resource_detail_with_repositories(
            &fixture.registry,
            &fixture.state,
            &fixture.catalog,
            &row.scope.scope_id,
            None,
            &resource_id,
            &fixture.user_home,
        )
        .is_err());

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

    /// The Library on Mallard Cloud: one item per repository, one request per
    /// refresh, the shared protocol per item (`PLAN_MANAGED_CLOUD_CAS_FREE.md`
    /// section 6.6), against the in-process stub of the service.
    #[cfg(test)]
    mod managed_cloud_tests {
        use super::*;
        include!("cloud_flow_tests.rs");
        use mallard_domain::{BoundedConfigObject, SecretMap, SecretValue, StorageProviderConfig};
        use mallard_infrastructure::managed_cloud::stub::StubManagedCloud;
        use mallard_infrastructure::storage_providers::managed_cloud::{
            provider_key, token_key, ManagedCloudSettingsV1,
        };
        use std::sync::Arc;

        /// A machine signed in to the stub as `login`, with one Codex profile
        /// holding the skills in `skills`, and Mallard Cloud as its storage.
        fn managed_machine(
            root: &Path,
            stub: &StubManagedCloud,
            login: &str,
            skills: &[(&str, &[u8])],
        ) -> Arc<LibraryFixture> {
            let (user, token) = stub.state.lock().unwrap().add_user(login);
            let user_home = root.join(login);
            let codex_home = user_home.join("codex-home");
            fs::create_dir_all(codex_home.join("skills")).unwrap();
            for (name, body) in skills {
                let skill = codex_home.join("skills").join(name);
                fs::create_dir_all(&skill).unwrap();
                fs::write(skill.join("SKILL.md"), body).unwrap();
            }
            let registry = CatalogRegistry::production();
            let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
            let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
            let profile = save_catalog_resource_profile_with_repository(
                &registry,
                &catalog,
                ProviderKey::parse("codex").unwrap(),
                codex_home.to_string_lossy().to_string(),
                1,
            )
            .unwrap();
            let mut secrets = SecretMap::empty();
            secrets
                .insert(token_key(), SecretValue::parse(&token).unwrap())
                .unwrap();
            let storage = StorageConfig {
                id: StorageId::parse(format!("managed-{login}")).unwrap(),
                name: "Mallard Cloud".to_string(),
                provider: provider_key(),
                provider_config: StorageProviderConfig::new(
                    1,
                    BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
                        service_origin: stub.endpoint.clone(),
                        github_user_id: user.github_user_id.clone(),
                        account_login: login.to_string(),
                    })
                    .unwrap(),
                    secrets,
                ),
            };
            let mut config = SyncConfig::default();
            config.storages.push(storage.clone());
            state.save_config(config).unwrap();
            Arc::new(LibraryFixture {
                _temporary: tempfile::tempdir().unwrap(),
                registry,
                user_home,
                skill_path: codex_home.join("skills/review"),
                storage_path: root.join("unused"),
                state,
                catalog,
                profile,
                storage,
            })
        }

        async fn blocking<T: Send + 'static>(
            fixture: &Arc<LibraryFixture>,
            work: impl FnOnce(&LibraryFixture) -> T + Send + 'static,
        ) -> T {
            let fixture = fixture.clone();
            tokio::task::spawn_blocking(move || work(&fixture))
                .await
                .unwrap()
        }

        fn coordinate_request(kind: &str, format: &str, version: u32) -> String {
            format!(
                "GET /v1/repositories/library?provider=codex&resource_kind={kind}&format={format}&format_version={version}"
            )
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn items_push_one_per_repository_and_a_refresh_is_one_coordinate_request() {
            let stub = StubManagedCloud::start().await;
            let root = tempfile::tempdir().unwrap();
            let machine = managed_machine(
                root.path(),
                &stub,
                "octocat",
                &[
                    ("review", b"---\nname: review\n---\nReview.\n"),
                    ("extra", b"---\nname: extra\n---\nExtra.\n"),
                ],
            );
            let (scope_id, storage_id) = blocking(&machine, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                let link = link_scope(fixture, &scope, &fixture.storage.id, 3);
                assert_eq!(link.completion, "open");
                push_all(fixture, &scope, &fixture.storage.id);
                (scope, fixture.storage.id.clone())
            })
            .await;

            // Two items, two repositories, each with one record and one claim
            // under its own item stream, and the website's names.
            {
                let state = stub.state.lock().unwrap();
                let mut names = state
                    .repositories
                    .values()
                    .map(|repository| (repository.class.clone(), repository.name.clone()))
                    .collect::<Vec<_>>();
                names.sort();
                assert_eq!(
                    names,
                    vec![
                        ("catalog".to_string(), "extra".to_string()),
                        ("catalog".to_string(), "review".to_string())
                    ]
                );
                for repository in state.repositories.values() {
                    let stream = &repository.catalog.as_ref().unwrap().library_repository_id;
                    assert_eq!(
                        repository
                            .keys_under(&format!(".mallard/v1/library/{stream}/_commits"))
                            .len(),
                        1
                    );
                    assert_eq!(
                        repository
                            .keys_under(&format!(".mallard/v1/library/{stream}/_claims"))
                            .len(),
                        1
                    );
                    assert!(repository
                        .objects
                        .contains_key(&format!(".mallard/v1/library/{stream}/_tag.json")));
                    assert_eq!(repository.private_revision, 1);
                }
            }
            let links = machine.catalog.load_scope_links().unwrap();
            assert_eq!(links.links[0].accepted_items.len(), 2);
            assert_eq!(links.links[0].accepted_resources.len(), 2);
            assert!(links.links[0].accepted_head.is_none());

            // A refresh with nothing changed: one request, naming the
            // coordinate, and nothing read or decoded beyond it.
            stub.clear_log();
            let refreshed = blocking(&machine, {
                let scope_id = scope_id.clone();
                let storage_id = storage_id.clone();
                move |fixture| page(fixture, &scope_id, Some(&storage_id)).unwrap()
            })
            .await;
            assert_eq!(refreshed.rows.len(), 2);
            assert!(refreshed
                .rows
                .iter()
                .all(|row| row.state == Some(CatalogLinkState::InSync)));
            assert_eq!(refreshed.storage_revision, None);
            assert_eq!(
                stub.request_lines(),
                vec![coordinate_request("agent_skill", "agentskills.io.skill", 1)]
            );

            // Another coordinate's page names only its own coordinate.
            stub.clear_log();
            let pets = blocking(&machine, {
                let storage_id = storage_id.clone();
                move |fixture| {
                    let scope = create_catalog_scope_with_repositories(
                        &fixture.registry,
                        &fixture.catalog,
                        CatalogCoordinate::new("codex", "desktop_pet", "openai.codex-pet", 2)
                            .unwrap(),
                        fixture.profile.profile_id.clone(),
                        5,
                    )
                    .unwrap()
                    .scope
                    .scope_id;
                    page(fixture, &scope, Some(&storage_id)).unwrap()
                }
            })
            .await;
            assert!(pets.rows.is_empty());
            assert_eq!(
                stub.request_lines(),
                vec![coordinate_request("desktop_pet", "openai.codex-pet", 2)]
            );

            // The Storages page lists the coordinate's items from the same answer.
            let repositories = blocking(&machine, {
                let storage_id = storage_id.clone();
                move |fixture| {
                    list_catalog_storage_repositories_with_repository(
                        &fixture.registry,
                        &fixture.state,
                        &storage_id,
                    )
                    .unwrap()
                }
            })
            .await;
            assert_eq!(repositories.len(), 1);
            assert_eq!(repositories[0].resource_names, vec!["extra", "review"]);

            // A second push of an unchanged item publishes nothing; the plan
            // offers nothing selectable.
            let plan = blocking(&machine, {
                let scope_id = scope_id.clone();
                let storage_id = storage_id.clone();
                move |fixture| prepare_push(fixture, &scope_id, &storage_id)
            })
            .await;
            assert!(plan.rows.iter().all(|row| !row.row.selectable));
            assert_eq!(plan.observed_items.len(), 2);
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_plugin_push_describes_its_commit_from_the_installed_manifest() {
            let stub = StubManagedCloud::start().await;
            let root = tempfile::tempdir().unwrap();
            let machine = managed_machine(root.path(), &stub, "octocat", &[]);
            let home = root.path().join("octocat").join("codex-home");
            fs::write(home.join("config.toml"), "[marketplaces.acme]\nsource_type=\"git\"\nsource=\"https://example.com/acme/market\"\n[plugins.\"tools@acme\"]\nenabled=true\n").unwrap();
            let manifest = home.join("plugins/cache/acme/tools/1/.codex-plugin/plugin.json");
            fs::create_dir_all(manifest.parent().unwrap()).unwrap();
            fs::write(
                manifest,
                r#"{"name":"tools","version":"1","description":"Tools for acme.","repository":"https://github.com/acme/secret","interface":{"displayName":"Tools","developerName":"Acme","websiteURL":"https://acme.example/","logo":"./assets/logo.png"}}"#,
            )
            .unwrap();
            blocking(&machine, |fixture| {
                let scope = create_catalog_scope_with_repositories(
                    &fixture.registry,
                    &fixture.catalog,
                    CatalogCoordinate::new(
                        "codex",
                        "plugin_install_intent",
                        "mallard.plugin-install-intent",
                        1,
                    )
                    .unwrap(),
                    fixture.profile.profile_id.clone(),
                    2,
                )
                .unwrap()
                .scope
                .scope_id;
                link_scope(fixture, &scope, &fixture.storage.id, 3);
                push_all(fixture, &scope, &fixture.storage.id);
            })
            .await;

            // One listing, for the commit just pushed, with the allowlisted
            // display fields only: no repository field, path or logo.
            let state = stub.state.lock().unwrap();
            let repository = state.repositories.values().next().unwrap();
            let commit = repository.publications.last().unwrap().commit_id.clone();
            assert_eq!(state.plugin_listings.len(), 1);
            assert_eq!(
                state.plugin_listings[&(repository.id.clone(), commit)],
                serde_json::json!({
                    "display_name": "Tools",
                    "developer": "Acme",
                    "description": "Tools for acme.",
                    "category": null,
                    "website_url": "https://acme.example/",
                    "privacy_url": null,
                    "terms_url": null,
                })
            );
            let sent =
                serde_json::to_string(&state.plugin_listings.values().collect::<Vec<_>>()).unwrap();
            assert!(
                !sent.contains("secret") && !sent.contains("logo") && !sent.contains("codex-home")
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn an_item_pushed_by_one_account_is_pulled_by_another_and_a_stale_writer_is_refused()
        {
            let stub = StubManagedCloud::start().await;
            let root = tempfile::tempdir().unwrap();
            let alice = managed_machine(
                root.path(),
                &stub,
                "alice",
                &[("review", b"---\nname: review\n---\nAlice's review.\n")],
            );
            let bob = managed_machine(root.path(), &stub, "bob", &[]);
            let alice_scope = blocking(&alice, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                link_scope(fixture, &scope, &fixture.storage.id, 3);
                push_all(fixture, &scope, &fixture.storage.id);
                scope
            })
            .await;
            let (repository_id, bob_id) = {
                let state = stub.state.lock().unwrap();
                let repository = state.repositories.values().next().unwrap();
                let bob_id = state
                    .users
                    .values()
                    .find(|user| user.login == "bob")
                    .unwrap()
                    .id
                    .clone();
                (repository.id.clone(), bob_id)
            };
            stub.state
                .lock()
                .unwrap()
                .add_member(&repository_id, &bob_id, "member");

            // Bob links, reviews the Pull, and installs Alice's skill.
            let bob_scope = blocking(&bob, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                let link = link_scope(fixture, &scope, &fixture.storage.id, 4);
                assert_eq!(
                    link.completion, "open",
                    "Cloud linking saves metadata; the next step explicitly prepares Pull"
                );
                let plan = prepare_pull(fixture, &scope);
                assert!(plan.head.is_none());
                assert_eq!(plan.item_heads.len(), 1);
                assert_eq!(plan.items.len(), 1);
                let outcome = apply_pull_defaults(fixture, &plan).unwrap();
                assert!(outcome.remote_accepted, "{:?}", outcome.results);
                assert_eq!(
                    outcome.results[0].outcome,
                    CatalogActionOutcome::Installed,
                    "{:?}",
                    outcome.results
                );
                scope
            })
            .await;
            assert_eq!(
                fs::read(bob.user_home.join("codex-home/skills/review/SKILL.md")).unwrap(),
                b"---\nname: review\n---\nAlice's review.\n"
            );
            let bob_links = bob.catalog.load_scope_links().unwrap();
            assert_eq!(bob_links.links[0].accepted_items.len(), 1);

            // Alice reviews a change; Bob pushes his own before she applies:
            // her review is stale, nothing of hers is written, and her page
            // reads the storage's change.
            let alice_plan = blocking(&alice, {
                let scope = alice_scope.clone();
                move |fixture| {
                    fs::write(
                        fixture.skill_path.join("SKILL.md"),
                        b"---\nname: review\n---\nAlice's second review.\n",
                    )
                    .unwrap();
                    prepare_push(fixture, &scope, &fixture.storage.id)
                }
            })
            .await;
            assert!(alice_plan.rows[0].row.selectable);
            blocking(&bob, {
                let scope = bob_scope.clone();
                move |fixture| {
                    fs::write(
                        fixture.user_home.join("codex-home/skills/review/SKILL.md"),
                        b"---\nname: review\n---\nBob's review.\n",
                    )
                    .unwrap();
                    push_all(fixture, &scope, &fixture.storage.id);
                }
            })
            .await;
            let refused = blocking(&alice, {
                let plan = alice_plan.clone();
                move |fixture| {
                    apply_push(fixture, &plan, vec![plan.rows[0].row.resource_id.clone()])
                        .unwrap_err()
                }
            })
            .await;
            assert!(refused.contains("changed after review"), "{refused}");
            {
                let state = stub.state.lock().unwrap();
                let repository = &state.repositories[&repository_id];
                assert_eq!(repository.private_revision, 2);
                assert_eq!(
                    state.users[&repository.head().unwrap().created_by].login,
                    "bob"
                );
            }
            let alice_page = blocking(&alice, {
                let scope = alice_scope.clone();
                move |fixture| page(fixture, &scope, Some(&fixture.storage.id)).unwrap()
            })
            .await;
            assert_eq!(alice_page.rows[0].state, Some(CatalogLinkState::Diverged));

            // A Pull review pinned on Bob's version installs it even after
            // Bob pushes again: the reviewed version is still on the chain.
            let alice_pull = blocking(&alice, {
                let scope = alice_scope.clone();
                move |fixture| prepare_pull(fixture, &scope)
            })
            .await;
            blocking(&bob, {
                let scope = bob_scope.clone();
                move |fixture| {
                    fs::write(
                        fixture.user_home.join("codex-home/skills/review/SKILL.md"),
                        b"---\nname: review\n---\nBob's third review.\n",
                    )
                    .unwrap();
                    push_all(fixture, &scope, &fixture.storage.id);
                }
            })
            .await;
            let outcome = blocking(&alice, {
                let plan = alice_pull.clone();
                move |fixture| apply_pull_defaults(fixture, &plan).unwrap()
            })
            .await;
            assert!(outcome.remote_accepted, "{:?}", outcome.results);
            assert!(
                fs::read_to_string(alice.skill_path.join("SKILL.md"))
                    .unwrap()
                    .contains("Bob's review."),
                "the reviewed version, not the newest"
            );
        }

        /// An item's repository removed on the website while the app runs
        /// (PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md, rule R9): the store
        /// learned its id when it pushed the item. The item reads as not in
        /// the storage, being deleted or gone, and its Push creates the
        /// repository again at the same address and saves the item's
        /// accepted head. Nothing is relinked and nothing restarts.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn an_item_whose_repository_was_removed_is_pushed_again_without_a_restart() {
            let stub = StubManagedCloud::start().await;
            let root = tempfile::tempdir().unwrap();
            let machine = managed_machine(
                root.path(),
                &stub,
                "octocat",
                &[("review", b"---\nname: review\n---\nReview.\n")],
            );
            let (scope_id, storage_id) = blocking(&machine, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                link_scope(fixture, &scope, &fixture.storage.id, 3);
                push_all(fixture, &scope, &fixture.storage.id);
                (scope, fixture.storage.id.clone())
            })
            .await;
            let active = |stub: &StubManagedCloud| {
                stub.state
                    .lock()
                    .unwrap()
                    .repositories
                    .values()
                    .find(|repository| repository.state == "active")
                    .cloned()
            };
            let accepted = |machine: &Arc<LibraryFixture>| {
                machine.catalog.load_scope_links().unwrap().links[0]
                    .accepted_items
                    .clone()
            };
            let mut previous = accepted(&machine);
            assert_eq!(previous.len(), 1);

            for finished in [false, true] {
                let gone = active(&stub).unwrap();
                {
                    let mut state = stub.state.lock().unwrap();
                    state.remove(&gone.id);
                    if finished {
                        state.finish_removal(&gone.id);
                    }
                }
                // The page: the item is on this computer and not in the
                // storage. The refresh saves nothing.
                let refreshed = blocking(&machine, {
                    let scope_id = scope_id.clone();
                    let storage_id = storage_id.clone();
                    move |fixture| page(fixture, &scope_id, Some(&storage_id)).unwrap()
                })
                .await;
                assert_eq!(refreshed.rows.len(), 1);
                assert_eq!(
                    refreshed.rows[0].state,
                    Some(CatalogLinkState::New),
                    "{:?}",
                    refreshed.rows[0]
                );
                assert_eq!(accepted(&machine), previous);

                // The Push creates the repository again, under its name.
                stub.clear_log();
                blocking(&machine, {
                    let scope_id = scope_id.clone();
                    let storage_id = storage_id.clone();
                    move |fixture| push_all(fixture, &scope_id, &storage_id)
                })
                .await;
                let created = active(&stub).unwrap();
                assert_ne!(created.id, gone.id);
                assert_eq!(created.name, "review");
                assert_eq!(created.catalog, gone.catalog);
                assert_eq!(created.private_revision, 1);
                assert!(stub
                    .requests()
                    .iter()
                    .filter(|request| request.method == "PUT")
                    .all(|request| request.path.contains(&created.id)));
                let saved = accepted(&machine);
                assert_eq!(saved.len(), 1);
                assert_ne!(saved, previous, "the accepted head names the new history");
                previous = saved;

                let settled = blocking(&machine, {
                    let scope_id = scope_id.clone();
                    let storage_id = storage_id.clone();
                    move |fixture| page(fixture, &scope_id, Some(&storage_id)).unwrap()
                })
                .await;
                assert_eq!(settled.rows[0].state, Some(CatalogLinkState::InSync));
            }
        }

        /// An item's draft discarded on the website: its repository stays,
        /// empty. The item's accepted head names a removed push, so it reads
        /// as absent and the item as not in the storage; its Push starts a
        /// first version in the same repository.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn an_item_whose_draft_was_discarded_is_pushed_again_into_the_same_repository() {
            let stub = StubManagedCloud::start().await;
            let root = tempfile::tempdir().unwrap();
            let machine = managed_machine(
                root.path(),
                &stub,
                "octocat",
                &[("review", b"---\nname: review\n---\nReview.\n")],
            );
            let (scope_id, storage_id) = blocking(&machine, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                link_scope(fixture, &scope, &fixture.storage.id, 3);
                push_all(fixture, &scope, &fixture.storage.id);
                (scope, fixture.storage.id.clone())
            })
            .await;
            let repository_id = stub
                .state
                .lock()
                .unwrap()
                .repositories
                .values()
                .next()
                .unwrap()
                .id
                .clone();
            let accepted = machine.catalog.load_scope_links().unwrap().links[0]
                .accepted_items
                .clone();
            assert_eq!(stub.state.lock().unwrap().discard(&repository_id), Ok(None));

            let refreshed = blocking(&machine, {
                let scope_id = scope_id.clone();
                let storage_id = storage_id.clone();
                move |fixture| page(fixture, &scope_id, Some(&storage_id)).unwrap()
            })
            .await;
            assert_eq!(refreshed.rows[0].state, Some(CatalogLinkState::New));
            assert_eq!(
                machine.catalog.load_scope_links().unwrap().links[0].accepted_items,
                accepted,
                "a refresh saves nothing"
            );

            stub.clear_log();
            blocking(&machine, {
                let scope_id = scope_id.clone();
                let storage_id = storage_id.clone();
                move |fixture| push_all(fixture, &scope_id, &storage_id)
            })
            .await;
            assert!(!stub
                .request_lines()
                .contains(&"POST /v1/repositories".to_string()));
            {
                let state = stub.state.lock().unwrap();
                assert_eq!(state.repositories.len(), 1);
                let repository = &state.repositories[&repository_id];
                assert_eq!(repository.stream().1.names().len(), 1);
                assert_eq!(repository.head().unwrap().manifest_revision, 1);
            }
            let settled = blocking(&machine, {
                let scope_id = scope_id.clone();
                let storage_id = storage_id.clone();
                move |fixture| page(fixture, &scope_id, Some(&storage_id)).unwrap()
            })
            .await;
            assert_eq!(settled.rows[0].state, Some(CatalogLinkState::InSync));
            assert_ne!(
                machine.catalog.load_scope_links().unwrap().links[0].accepted_items,
                accepted
            );
        }

        /// The same Library flow against a running Worker (a local `wrangler
        /// dev` or staging), on an account whose token the environment
        /// supplies; ignored by default. It pushes two skills of a fresh
        /// machine, refreshes with one coordinate request, and reviews a
        /// Pull on a second machine of the same account:
        ///
        /// ```sh
        /// MALLARD_LIVE_CLOUD_ORIGIN=http://localhost:8787 MALLARD_LIVE_CLOUD_TOKEN=… \
        /// MALLARD_LIVE_CLOUD_GITHUB_USER_ID=123 MALLARD_LIVE_CLOUD_LOGIN=octocat \
        ///   cargo test --manifest-path src-tauri/Cargo.toml --lib managed_cloud_live_library -- --ignored --nocapture
        /// ```
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "needs a running Mallard Cloud Worker and a desktop token"]
        async fn managed_cloud_live_library_round_trip() {
            let origin = std::env::var("MALLARD_LIVE_CLOUD_ORIGIN").unwrap();
            let token = std::env::var("MALLARD_LIVE_CLOUD_TOKEN").unwrap();
            let github_user_id = std::env::var("MALLARD_LIVE_CLOUD_GITHUB_USER_ID").unwrap();
            let login = std::env::var("MALLARD_LIVE_CLOUD_LOGIN").unwrap();
            let root = tempfile::tempdir().unwrap();
            let stamp = now_secs();
            let machine = |name: &str, skills: &[(&str, &[u8])]| {
                let user_home = root.path().join(name);
                let codex_home = user_home.join("codex-home");
                fs::create_dir_all(codex_home.join("skills")).unwrap();
                for (skill, body) in skills {
                    let path = codex_home.join("skills").join(skill);
                    fs::create_dir_all(&path).unwrap();
                    fs::write(path.join("SKILL.md"), body).unwrap();
                }
                let registry = CatalogRegistry::production();
                let state = LocalStateRepository::from_home_dir(&user_home).unwrap();
                let catalog = CatalogStateRepository::from_home_dir(&user_home).unwrap();
                let profile = save_catalog_resource_profile_with_repository(
                    &registry,
                    &catalog,
                    ProviderKey::parse("codex").unwrap(),
                    codex_home.to_string_lossy().to_string(),
                    1,
                )
                .unwrap();
                let mut secrets = SecretMap::empty();
                secrets
                    .insert(token_key(), SecretValue::parse(&token).unwrap())
                    .unwrap();
                let storage = StorageConfig {
                    id: StorageId::parse(format!("managed-live-{name}")).unwrap(),
                    name: "Mallard Cloud".to_string(),
                    provider: provider_key(),
                    provider_config: StorageProviderConfig::new(
                        1,
                        BoundedConfigObject::encode(&ManagedCloudSettingsV1 {
                            service_origin: origin.clone(),
                            github_user_id: github_user_id.clone(),
                            account_login: login.clone(),
                        })
                        .unwrap(),
                        secrets,
                    ),
                };
                let mut config = SyncConfig::default();
                config.storages.push(storage.clone());
                state.save_config(config).unwrap();
                Arc::new(LibraryFixture {
                    _temporary: tempfile::tempdir().unwrap(),
                    registry,
                    user_home,
                    skill_path: codex_home.join(format!("skills/live-{stamp}-a")),
                    storage_path: root.path().join("unused"),
                    state,
                    catalog,
                    profile,
                    storage,
                })
            };
            let first_name = format!("live-{stamp}-a");
            let second_name = format!("live-{stamp}-b");
            let first_body = format!("---\nname: {first_name}\n---\nLive skill A.\n");
            let second_body = format!("---\nname: {second_name}\n---\nLive skill B.\n");
            let pusher = machine(
                "pusher",
                &[
                    (first_name.as_str(), first_body.as_bytes()),
                    (second_name.as_str(), second_body.as_bytes()),
                ],
            );
            let (scope_id, rows) = blocking(&pusher, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                link_scope(fixture, &scope, &fixture.storage.id, 3);
                push_all(fixture, &scope, &fixture.storage.id);
                let refreshed = page(fixture, &scope, Some(&fixture.storage.id)).unwrap();
                (scope, refreshed.rows)
            })
            .await;
            let mine = rows
                .iter()
                .filter(|row| row.display_name.starts_with(&format!("live-{stamp}-")))
                .collect::<Vec<_>>();
            assert_eq!(mine.len(), 2, "{rows:?}");
            assert!(mine
                .iter()
                .all(|row| row.state == Some(CatalogLinkState::InSync)));
            let plan = blocking(&pusher, {
                let scope_id = scope_id.clone();
                move |fixture| prepare_push(fixture, &scope_id, &fixture.storage.id)
            })
            .await;
            assert!(plan
                .rows
                .iter()
                .filter(|row| row.row.display_name.starts_with(&format!("live-{stamp}-")))
                .all(|row| !row.row.selectable));

            // A second machine of the same account reads both items in one
            // request and reviews a Pull of them.
            let puller = machine("puller", &[]);
            let pull = blocking(&puller, |fixture| {
                let scope = skill_scope(fixture).scope.scope_id;
                let link = link_scope(fixture, &scope, &fixture.storage.id, 4);
                assert_eq!(link.completion, "open");
                let plan = prepare_pull(fixture, &scope);
                let outcome = apply_pull_defaults(fixture, &plan).unwrap();
                assert!(outcome.remote_accepted, "{:?}", outcome.results);
                plan
            })
            .await;
            assert!(pull.head.is_none());
            assert!(pull.item_heads.len() >= 2);
            assert!(puller
                .user_home
                .join(format!("codex-home/skills/{first_name}/SKILL.md"))
                .is_file());
            eprintln!(
                "live Library round trip complete: {} items pushed, pulled on a second machine",
                mine.len()
            );
        }
    }
}
