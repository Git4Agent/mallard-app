//! Tauri command surface for the schema-3 local vertical slice.
//!
//! This layer composes provider capture, the bundle engine, and local
//! persistence. Schema 3 currently has a complete local-folder storage path;
//! S3 is rejected explicitly until a CAS-capable schema-3 adapter is wired in.

use super::bundle_engine::{
    write_immutable_backup, write_target_atomic, BundleEngine, BundleObjectStore, CasExpectation,
    CasOutcome, FetchedBundle, ImmutablePutOutcome, LocalBundleObjectStore, ObjectKey,
    ObjectPrefix, PublishBundleRequest, PublishExpectation, RemoteBundlePage, StoreListPage,
    StoredObject,
};
use super::chat_history::{self, CodexThreadDetailsPage, ProjectChatHistory};
use super::conversation_codec::analyze_codex_transcript;
use super::conversation_materialization::recover_conversation_materializations;
use super::conversation_sync::{
    default_conversation_decision, BindingResolution, ConversationCapabilities,
};
use super::domain::{
    generated_named_id, validate_absolute_clean_path, ActionId, ActionStatus, ApplyPolicy,
    ApplyReceipt, BindingState, BundleId, BundleIdentity, BundleKind, BundleManifest, BundleRecipe,
    BundleSnapshot, CapturedWith, ThreadId, ConversationFileRole,
    ConversationMaterializationState, ConversationPlanActionType, ConversationPlanDirection,
    ConversationReasonCode, ConversationRelation, DependencyAction, DependencyActionKind,
    DependencyApplicationRecord, DependencyApplyReceipt, DependencyPlan, DraftProfileSelection,
    DraftRepositoryChoice, DraftStorageSelection, HostId, LocalProjectId, LocalProjectRegistration,
    LocalProviderProfileId, MachineProjectState, MaterializationId, MaterializationRecord,
    MaterializationStatus, OriginHostAlias, OriginHostToken, PlanId, ProjectBinding,
    ProjectFileSyncEligibility, ProjectFileSyncEligibilityState, ProjectSetupDraft,
    ProjectStorageLink, Provenance, Provider, ProviderProfile, RecipeBase, RecipeEntry, ReplicaId,
    ResourceDescriptor, ResourceId, ResourceKind, ResourceScope, RestoreAction, RestoreActionKind,
    RestoreActionType, RestorePlan, SetupDraftId, SetupTransaction, StorageConfigV3, StorageId,
    StorageKind, SyncConfigV3, BUNDLE_SCHEMA_V6, DEPENDENCY_PLAN_SCHEMA_V2,
    MAX_CONVERSATION_FILE_BYTES, SETUP_DRAFT_SCHEMA_V1, SETUP_TRANSACTION_SCHEMA_V2,
};
use super::global_inventory;
use super::persistence::V3Repository;
use super::provider_capture::{
    self, CaptureApplyPolicy, CaptureRequest, CaptureResourceKind, Provider as CaptureProvider,
    ResourceCandidate,
};
use super::s3_store::S3BundleObjectStore;
use crate::activity_log::{emit_typed_log, ActivityLogScope, ActivityLogType};
use crate::codex_plugins;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use walkdir::WalkDir;

const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
const PLAN_LIFETIME_SECS: u64 = 24 * 60 * 60;
const MAX_CODEX_GLOBAL_STATE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CONVERSATION_REPAIR_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CODEX_SESSION_FILES: usize = 20_000;
const HISTORY_SCAN_LOG_THROTTLE_SECS: u64 = 5 * 60;
static HISTORY_SCAN_LOGGED_AT: std::sync::Mutex<BTreeMap<String, u64>> =
    std::sync::Mutex::new(BTreeMap::new());
const AGENT_HOME_LOCKED_MESSAGE: &str =
    "agent home is fixed after project setup; remove and set up the project again to use a different agent home";

fn claim_history_scan_log_at(
    logged_at: &mut BTreeMap<String, u64>,
    local_project_id: &str,
    force: bool,
    now: u64,
) -> bool {
    let allowed = force
        || logged_at
            .get(local_project_id)
            .is_none_or(|previous| now.saturating_sub(*previous) >= HISTORY_SCAN_LOG_THROTTLE_SECS);
    if allowed {
        logged_at.insert(local_project_id.to_string(), now);
    }
    allowed
}

fn should_log_history_scan(local_project_id: &LocalProjectId, force: bool) -> bool {
    HISTORY_SCAN_LOGGED_AT
        .lock()
        .map(|mut logged_at| {
            claim_history_scan_log_at(&mut logged_at, local_project_id.as_str(), force, now_secs())
        })
        // A poisoned throttle must not hide operational diagnostics.
        .unwrap_or(true)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisterLocalProjectRequest {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    /// Linking a remote bundle supplies its ID; a local-first project omits
    /// it and gets one generated before any storage publication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<BundleId>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveProjectLinkRequest {
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectProjectBundleRequest {
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    /// Reject a stale chooser instead of replacing an identity that changed
    /// while the user was deciding which remote bundle to connect.
    pub expected_bundle_id: BundleId,
    #[serde(default)]
    pub pinned: bool,
    /// A manual chooser may connect a checkout to a repo with a different or
    /// missing Git fingerprint after presenting that mismatch to the user.
    #[serde(default)]
    pub allow_repository_mismatch: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveProjectBindingRequest {
    pub local_project_id: LocalProjectId,
    pub project_root: String,
    #[serde(default)]
    pub profile_ids: BTreeMap<Provider, LocalProviderProfileId>,
    /// Required when changing an existing binding.  Creation uses `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct HostIdentityStatus {
    pub host_id: HostId,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_display_name: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub origin_key_available: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateHostDisplayNameRequest {
    pub expected_host_id: HostId,
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_display_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisterNewHostRequest {
    pub expected_host_id: HostId,
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_display_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveOriginHostAliasRequest {
    pub bundle_id: BundleId,
    pub origin_host_token: OriginHostToken,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct OriginHostLabel {
    pub origin_host_token: OriginHostToken,
    pub display_name: String,
    pub is_this_host: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ProjectOriginTokens {
    pub bundle_id: BundleId,
    pub origin_host_token: OriginHostToken,
    pub origin_replica_token: super::domain::OriginReplicaToken,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectDetail {
    pub project: LocalProjectRegistration,
    pub links: Vec<ProjectStorageLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<ProjectBinding>,
    pub materializations: Vec<MaterializationRecord>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexConversationPathIssue {
    pub thread_id: String,
    pub transcript_path: String,
    pub recorded_cwd: String,
    pub target_cwd: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CodexConversationPathAudit {
    pub local_project_id: LocalProjectId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_path: Option<String>,
    pub project_root: String,
    pub assigned_thread_count: usize,
    pub matching_thread_count: usize,
    #[serde(default)]
    pub issues: Vec<CodexConversationPathIssue>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub ready: bool,
    pub can_repair: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct CodexConversationPathRepairResult {
    pub audit: CodexConversationPathAudit,
    #[serde(default)]
    pub repaired_thread_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct InventoryResource {
    #[serde(flatten)]
    pub descriptor: ResourceDescriptor,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    pub default_selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provided_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_behavior: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ResourceInventory {
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<BundleId>,
    pub resources: Vec<InventoryResource>,
    pub recipe: BundleRecipe,
    pub generated_at: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectContentEntryType {
    File,
    Directory,
    Blocked,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectContentEntry {
    pub descriptor: ResourceDescriptor,
    pub entry_type: ProjectContentEntryType,
    pub relative_path: String,
    pub logical_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_mtime: Option<u64>,
    pub state: String,
    pub local_present: bool,
    pub storage_present: bool,
    pub base_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_digest: Option<String>,
    pub selected_in_recipe: bool,
    pub newly_discovered: bool,
    pub selected_after_scan: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_digest: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectContentInventory {
    pub local_project_id: LocalProjectId,
    pub storage_id: StorageId,
    pub project_root: String,
    pub eligibility: ProjectFileSyncEligibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_generation: Option<u64>,
    pub preference_revision: u64,
    #[serde(default)]
    pub excluded_resource_ids: Vec<ResourceId>,
    #[serde(default)]
    pub entries: Vec<ProjectContentEntry>,
    pub ignored_count: usize,
    pub blocked_count: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub scanned_at: u64,
}

#[derive(Clone, Debug, Default)]
struct ProjectContentPushReview {
    review_token: Option<String>,
    removal_ids: BTreeSet<ResourceId>,
    acknowledged_warning_digests: BTreeSet<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectDiscovery {
    pub project_root: String,
    pub display_name: String,
    pub inventory: ResourceInventory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub profile_ids: BTreeMap<Provider, LocalProviderProfileId>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProviderProfileSummary {
    #[serde(flatten)]
    pub profile: ProviderProfile,
    pub available: bool,
    pub readable: bool,
    pub writable: bool,
    #[serde(default)]
    pub used_by_projects: Vec<LocalProjectId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProviderProfileProbe {
    pub provider: Provider,
    pub requested_path: String,
    pub resolved_path: String,
    pub canonical_path: String,
    pub suggested_name: String,
    pub readable: bool,
    pub writable: bool,
    pub detected_child: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_profile_id: Option<LocalProviderProfileId>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BundleResourceStatus {
    pub resource_id: ResourceId,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_digest: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ResourceStatusReport {
    pub project: String,
    pub storage: String,
    pub bundle_id: BundleId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    pub statuses: Vec<BundleResourceStatus>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct CapabilityProfileContext {
    pub provider: Provider,
    pub profile_id: LocalProviderProfileId,
    pub display_name: String,
    pub path: String,
    pub shared_project_count: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct CapabilityStatusItem {
    #[serde(flatten)]
    pub descriptor: ResourceDescriptor,
    pub category: String,
    pub state: String,
    pub local_present: bool,
    pub storage_present: bool,
    pub selected_in_recipe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub logical_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provided_skills: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CapabilityStatusReport {
    pub project_id: LocalProjectId,
    pub project_name: String,
    #[serde(default)]
    pub profiles: Vec<CapabilityProfileContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_id: Option<StorageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_generation: Option<u64>,
    pub compared_at: u64,
    pub items: Vec<CapabilityStatusItem>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadSyncState {
    Synced,
    LocalOnly,
    LocalAhead,
    StorageOnly,
    StorageAhead,
    Diverged,
    Unavailable,
    Unknown,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ThreadSyncEntry {
    pub thread_id: String,
    pub resource_id: ResourceId,
    pub display_name: String,
    pub state: ThreadSyncState,
    pub local_present: bool,
    pub storage_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_updated_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_updated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<ConversationRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_action: Option<ConversationPlanActionType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ConversationReasonCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization_state: Option<ConversationMaterializationState>,
    #[serde(default)]
    pub resumable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_label: Option<String>,
    #[serde(default)]
    pub available_actions: Vec<ConversationPlanActionType>,
    #[serde(default)]
    pub requires_explicit_approval: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadSyncCounts {
    pub synced: usize,
    pub local: usize,
    pub storage: usize,
    pub diverged: usize,
    pub unavailable: usize,
    pub unknown: usize,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ThreadSyncComparison {
    pub project_id: LocalProjectId,
    pub storage_id: StorageId,
    pub storage_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_generation: Option<u64>,
    pub compared_at: u64,
    pub entries: Vec<ThreadSyncEntry>,
    pub counts: ThreadSyncCounts,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OperationResourceResult {
    pub resource_id: ResourceId,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProjectOperationResult {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources_changed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default)]
    pub results: Vec<OperationResourceResult>,
}

#[derive(Serialize, Clone, Debug)]
pub struct RemoteBundleSummaryDto {
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    pub generation: u64,
    pub updated_at: u64,
    pub resource_count: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct BundlePage {
    pub bundles: Vec<RemoteBundleSummaryDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BundleSnapshotSummary {
    pub storage_id: StorageId,
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    pub generation: u64,
    pub updated_at: u64,
    pub resource_count: usize,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_fingerprint: Option<String>,
    pub resources: Vec<ResourceDescriptor>,
    pub recipe: BundleRecipe,
    pub fetched_at: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FailedAction {
    pub action_id: ActionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub message: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct RestoreResult {
    pub success: bool,
    pub message: String,
    pub plan_id: PlanId,
    #[serde(default)]
    pub applied_action_ids: Vec<ActionId>,
    #[serde(default)]
    pub failed_actions: Vec<FailedAction>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DependencyResult {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub applied_action_ids: Vec<ActionId>,
    #[serde(default)]
    pub failed_actions: Vec<FailedAction>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BundleReadinessIssue {
    pub issue_id: String,
    pub category: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<ResourceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BundleReadiness {
    pub bundle_id: BundleId,
    pub state: String,
    pub issues: Vec<BundleReadinessIssue>,
    pub generated_at: u64,
}

fn repository<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<V3Repository, String> {
    V3Repository::from_app(app)
}

fn project_storage_log_labels(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> (String, String) {
    let config = repository.load_config().ok();
    let project = config
        .as_ref()
        .and_then(|config| config.project(local_project_id))
        .map(|project| project.display_name.clone())
        .unwrap_or_else(|| local_project_id.to_string());
    let storage = config
        .as_ref()
        .and_then(|config| {
            config
                .storages
                .iter()
                .find(|storage| &storage.id == storage_id)
        })
        .map(|storage| storage.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| storage_id.to_string());
    (project, storage)
}

fn storage_log_label(repository: &V3Repository, storage_id: &StorageId) -> String {
    repository
        .load_config()
        .ok()
        .and_then(|config| {
            config
                .storages
                .into_iter()
                .find(|storage| &storage.id == storage_id)
        })
        .map(|storage| storage.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| storage_id.to_string())
}

fn storage_change_summary(
    previous: &[StorageConfigV3],
    next: &[StorageConfigV3],
) -> Option<(&'static str, String, StorageId, String)> {
    let added = next
        .iter()
        .filter(|storage| !previous.iter().any(|candidate| candidate.id == storage.id))
        .collect::<Vec<_>>();
    let updated = next
        .iter()
        .filter(|storage| {
            previous
                .iter()
                .find(|candidate| candidate.id == storage.id)
                .is_some_and(|candidate| candidate != *storage)
        })
        .collect::<Vec<_>>();
    let removed = previous
        .iter()
        .filter(|storage| !next.iter().any(|candidate| candidate.id == storage.id))
        .collect::<Vec<_>>();
    let total = added.len() + updated.len() + removed.len();
    if total == 0 {
        return None;
    }

    let (event, message, primary) = if total == 1 && added.len() == 1 {
        let storage = added[0];
        (
            "storage.added",
            format!("Storage added — {}", storage.name),
            storage,
        )
    } else if total == 1 && updated.len() == 1 {
        let storage = updated[0];
        (
            "storage.settings_updated",
            format!("Storage settings updated — {}", storage.name),
            storage,
        )
    } else if total == 1 {
        let storage = removed[0];
        (
            "storage.removed",
            format!("Storage removed — {}", storage.name),
            storage,
        )
    } else {
        let primary = added
            .first()
            .or_else(|| updated.first())
            .or_else(|| removed.first())?;
        (
            "storage.settings_updated",
            format!(
                "Storage settings updated — {} added, {} changed, {} removed",
                added.len(),
                updated.len(),
                removed.len()
            ),
            *primary,
        )
    };
    let name = if primary.name.trim().is_empty() {
        primary.id.to_string()
    } else {
        primary.name.clone()
    };
    Some((event, message, primary.id.clone(), name))
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("project-sync worker failed: {}", error))?
}

#[tauri::command]
pub async fn get_project_sync_config(app: tauri::AppHandle) -> Result<SyncConfigV3, String> {
    repository(&app)?.load_config()
}

fn host_identity_status(
    identity: super::domain::HostIdentity,
    origin_key_available: bool,
) -> HostIdentityStatus {
    HostIdentityStatus {
        host_id: identity.host_id,
        revision: identity.revision,
        local_display_name: identity.local_display_name,
        created_at: identity.created_at,
        updated_at: identity.updated_at,
        origin_key_available,
    }
}

#[tauri::command]
pub async fn get_host_identity_status(app: tauri::AppHandle) -> Result<HostIdentityStatus, String> {
    let (identity, key_available) = repository(&app)?.inspect_host_identity()?;
    Ok(host_identity_status(identity, key_available))
}

#[tauri::command]
pub async fn update_host_display_name(
    app: tauri::AppHandle,
    request: UpdateHostDisplayNameRequest,
) -> Result<HostIdentityStatus, String> {
    let identity = repository(&app)?.update_host_display_name(
        &request.expected_host_id,
        request.expected_revision,
        request.local_display_name,
    )?;
    Ok(host_identity_status(identity, true))
}

fn register_new_host_with_repository(
    repository: &V3Repository,
    request: RegisterNewHostRequest,
) -> Result<HostIdentityStatus, String> {
    let identity = repository.register_as_new_host(
        Some((&request.expected_host_id, request.expected_revision)),
        request.local_display_name,
    )?;
    Ok(host_identity_status(identity, true))
}

#[tauri::command]
pub async fn register_as_new_host(
    app: tauri::AppHandle,
    request: RegisterNewHostRequest,
) -> Result<HostIdentityStatus, String> {
    register_new_host_with_repository(&repository(&app)?, request)
}

#[tauri::command]
pub async fn rotate_host_identity(
    app: tauri::AppHandle,
    request: RegisterNewHostRequest,
) -> Result<HostIdentityStatus, String> {
    let identity = repository(&app)?.rotate_host_identity(
        Some((&request.expected_host_id, request.expected_revision)),
        request.local_display_name,
    )?;
    Ok(host_identity_status(identity, true))
}

#[tauri::command]
pub async fn get_project_origin_tokens(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<ProjectOriginTokens, String> {
    let repository = repository(&app)?;
    let (bundle_id, origin_host_token, origin_replica_token) =
        repository.derive_project_origin_tokens(&local_project_id)?;
    Ok(ProjectOriginTokens {
        bundle_id,
        origin_host_token,
        origin_replica_token,
    })
}

#[tauri::command]
pub async fn list_origin_host_aliases(
    app: tauri::AppHandle,
    bundle_id: Option<BundleId>,
) -> Result<Vec<OriginHostAlias>, String> {
    let mut aliases = repository(&app)?.load_origin_aliases()?;
    if let Some(bundle_id) = bundle_id {
        aliases.retain(|alias| alias.bundle_id == bundle_id);
    }
    aliases.sort_by(|left, right| {
        left.bundle_id
            .cmp(&right.bundle_id)
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.origin_host_token.cmp(&right.origin_host_token))
    });
    Ok(aliases)
}

#[tauri::command]
pub async fn save_origin_host_alias(
    app: tauri::AppHandle,
    request: SaveOriginHostAliasRequest,
) -> Result<OriginHostAlias, String> {
    let repository = repository(&app)?;
    let now = now_secs();
    repository.mutate_origin_aliases(|aliases| {
        let position = aliases.iter().position(|alias| {
            alias.bundle_id == request.bundle_id
                && alias.origin_host_token == request.origin_host_token
        });
        let alias = match position {
            Some(index) => {
                let current = &aliases[index];
                let expected = request.expected_revision.ok_or_else(|| {
                    "expected_revision is required when changing an origin alias".to_string()
                })?;
                if expected != current.revision {
                    return Err(format!(
                        "origin alias changed (expected revision {}, current {})",
                        expected, current.revision
                    ));
                }
                OriginHostAlias {
                    bundle_id: request.bundle_id.clone(),
                    origin_host_token: request.origin_host_token.clone(),
                    display_name: request.display_name.clone(),
                    revision: current.revision.saturating_add(1),
                    updated_at: now,
                }
            }
            None => {
                if request.expected_revision.is_some() {
                    return Err("new origin alias must not provide expected_revision".to_string());
                }
                OriginHostAlias {
                    bundle_id: request.bundle_id.clone(),
                    origin_host_token: request.origin_host_token.clone(),
                    display_name: request.display_name.clone(),
                    revision: 0,
                    updated_at: now,
                }
            }
        };
        alias.validate()?;
        match position {
            Some(index) => aliases[index] = alias.clone(),
            None => aliases.push(alias.clone()),
        }
        Ok(alias)
    })
}

#[tauri::command]
pub async fn resolve_origin_host_label(
    app: tauri::AppHandle,
    bundle_id: BundleId,
    origin_host_token: OriginHostToken,
) -> Result<OriginHostLabel, String> {
    let repository = repository(&app)?;
    let (display_name, is_this_host) =
        repository.resolve_origin_host_label(&bundle_id, &origin_host_token)?;
    Ok(OriginHostLabel {
        origin_host_token,
        display_name,
        is_this_host,
    })
}

#[tauri::command]
pub async fn save_project_sync_config(
    app: tauri::AppHandle,
    config: SyncConfigV3,
) -> Result<SyncConfigV3, String> {
    let repository = repository(&app)?;
    let storage_change = repository
        .load_config()
        .ok()
        .and_then(|previous| storage_change_summary(&previous.storages, &config.storages));
    let result = save_project_sync_config_with_repository(&repository, config);
    if let Some((event, message, storage_id, storage_name)) = storage_change {
        let log = ActivityLogScope::new(ActivityLogType::Storage)
            .storage(&storage_id, Some(&storage_name));
        match &result {
            Ok(_) => log.emit(&app, "ok", event, &message),
            Err(error) => log.emit(
                &app,
                "error",
                "storage.settings_update_failed",
                &format!("Storage settings could not be saved: {error}"),
            ),
        }
    }
    result
}

fn save_project_sync_config_with_repository(
    repository: &V3Repository,
    config: SyncConfigV3,
) -> Result<SyncConfigV3, String> {
    let bindings = repository.load_bindings()?;
    bindings.validate(&config)?;
    validate_config_storage_isolation(repository, &config, &bindings.bindings, &bindings.profiles)?;
    repository.save_config(config)
}

#[tauri::command]
pub async fn list_local_projects(
    app: tauri::AppHandle,
) -> Result<Vec<LocalProjectRegistration>, String> {
    let repository = repository(&app)?;
    // The shell lists projects first on every launch, so an interrupted
    // finalization heals here before any project data is rendered.
    for warning in recover_setup_state(&repository) {
        emit_typed_log(&app, ActivityLogType::Configuration, "warning", &warning);
    }
    match recover_conversation_materializations(&repository) {
        Ok(report) if !report.recovery_required.is_empty() => emit_typed_log(
            &app,
            ActivityLogType::Repair,
            "warning",
            "One or more conversation sync operations need manual recovery.",
        ),
        Ok(_) => {}
        Err(error) => emit_typed_log(
            &app,
            ActivityLogType::Repair,
            "warning",
            &format!("Conversation recovery is unavailable: {error}"),
        ),
    }
    Ok(repository.load_config()?.projects)
}

#[tauri::command]
pub async fn get_project(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<Option<ProjectDetail>, String> {
    get_project_with_repository(&repository(&app)?, &local_project_id)
}

#[tauri::command]
pub async fn get_local_project(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<Option<LocalProjectRegistration>, String> {
    Ok(repository(&app)?
        .load_config()?
        .projects
        .into_iter()
        .find(|project| project.local_project_id == local_project_id))
}

#[tauri::command]
pub async fn list_project_repository_kinds(
    app: tauri::AppHandle,
) -> Result<BTreeMap<String, bool>, String> {
    let repository = repository(&app)?;
    run_blocking(move || chat_history::list_project_repository_kinds(&repository)).await
}

#[tauri::command]
pub async fn get_project_chat_history(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    branch: Option<String>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: Option<bool>,
) -> Result<ProjectChatHistory, String> {
    let repository = repository(&app)?;
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    let force_revalidate = force_revalidate.unwrap_or(false);
    let log_routine_scan = should_log_history_scan(&local_project_id, force_revalidate);
    if log_routine_scan {
        log.emit(
            &app,
            "info",
            "history.scan_started",
            &format!("Scanning Codex session history for project {local_project_id}…"),
        );
    }
    let result = run_blocking(move || {
        chat_history::get_project_chat_history(
            &repository,
            &local_project_id,
            branch.as_deref(),
            before_time,
            window_days,
            force_revalidate,
        )
    })
    .await;
    match &result {
        Ok(history) => {
            for warning in &history.warnings {
                log.emit(&app, "warning", "history.scan_warning", warning);
            }
            if log_routine_scan {
                log.emit(
                    &app,
                    "info",
                    "history.scan_completed",
                    &format!(
                        "Session history scan complete: {} sessions in this {}-day window",
                        history.threads.len(),
                        history.window_end.saturating_sub(history.window_start) / (24 * 60 * 60)
                    ),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "history.scan_failed",
            &format!("Session history scan failed: {error}"),
        ),
    }
    result
}

#[tauri::command]
pub async fn get_project_chat_thread_details(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    thread_id: String,
    cursor: Option<usize>,
    limit: Option<usize>,
) -> Result<CodexThreadDetailsPage, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        chat_history::get_project_chat_thread_details(
            &repository,
            &local_project_id,
            &thread_id,
            cursor,
            limit,
        )
    })
    .await
}

#[tauri::command]
pub async fn open_codex_thread_in_terminal(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    thread_id: String,
) -> Result<(), String> {
    let repository = repository(&app)?;
    let worker_app = app.clone();
    let worker_log =
        ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        chat_history::open_codex_thread_in_terminal(
            &repository,
            &local_project_id,
            &thread_id,
            |command| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "history.opened_in_terminal",
                    &format!("$ {command}"),
                )
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn open_codex_thread_in_app(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    thread_id: String,
) -> Result<(), String> {
    let repository = repository(&app)?;
    let worker_app = app.clone();
    let worker_log =
        ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        chat_history::open_codex_thread_in_app(
            &repository,
            &local_project_id,
            &thread_id,
            |command| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "history.opened_in_app",
                    &format!("$ {command}"),
                )
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn validate_codex_thread_ownership(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    thread_id: String,
) -> Result<(), String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        chat_history::validate_codex_thread_ownership(&repository, &local_project_id, &thread_id)
    })
    .await
}

#[tauri::command]
pub async fn register_local_project(
    app: tauri::AppHandle,
    request: RegisterLocalProjectRequest,
) -> Result<LocalProjectRegistration, String> {
    register_local_project_with_repository(&repository(&app)?, request)
}

#[tauri::command]
pub async fn remove_local_project(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<bool, String> {
    remove_local_project_with_repository(&repository(&app)?, &local_project_id)
}

#[tauri::command]
pub async fn rename_local_project(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    local_alias: Option<String>,
    expected_revision: u64,
) -> Result<LocalProjectRegistration, String> {
    rename_local_project_with_repository(
        &repository(&app)?,
        &local_project_id,
        local_alias,
        expected_revision,
    )
}

#[tauri::command]
pub async fn save_bundle_recipe(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    recipe: BundleRecipe,
) -> Result<LocalProjectRegistration, String> {
    save_bundle_recipe_with_repository(&repository(&app)?, &local_project_id, recipe)
}

#[tauri::command]
pub async fn save_project_link(
    app: tauri::AppHandle,
    request: SaveProjectLinkRequest,
) -> Result<ProjectStorageLink, String> {
    save_project_link_with_repository(&repository(&app)?, request)
}

#[tauri::command]
pub async fn connect_project_to_remote_bundle(
    app: tauri::AppHandle,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let repository = repository(&app)?;
    let storage = storage_log_label(&repository, &request.storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Configuration)
        .project(&request.local_project_id, None)
        .storage(&request.storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "configuration.remote_connected_started",
        &format!(
            "Connecting project to bundle {} in {}…",
            request.bundle_id, storage
        ),
    );
    let result = run_blocking(move || {
        connect_project_to_remote_bundle_with_repository(&repository, request)
    })
    .await;
    match &result {
        Ok(detail) => log.emit(
            &app,
            "ok",
            "configuration.remote_connected",
            &format!(
                "Connected {} to remote bundle {}",
                detail.project.display_name, detail.project.bundle_id
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "configuration.remote_connect_failed",
            &format!("Failed to connect remote bundle: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn remove_project_link(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<bool, String> {
    remove_project_link_with_repository(&repository(&app)?, &local_project_id, &storage_id)
}

fn remove_project_link_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<bool, String> {
    let _host_operation = repository.acquire_host_operation()?;
    repository.mutate_config(|config| {
        let before = config.links.len();
        config.links.retain(|link| {
            &link.local_project_id != local_project_id || &link.storage_id != storage_id
        });
        let removed = before != config.links.len();
        if removed {
            let project = config
                .projects
                .iter_mut()
                .find(|project| &project.local_project_id == local_project_id)
                .ok_or_else(|| "linked project no longer exists".to_string())?;
            if project.recipe_bases.remove(storage_id).is_some() {
                project.revision = project.revision.saturating_add(1);
                project.updated_at = now_secs();
            }
        }
        Ok(removed)
    })
}

#[tauri::command]
pub async fn list_provider_profiles(
    app: tauri::AppHandle,
) -> Result<Vec<ProviderProfileSummary>, String> {
    let repository = repository(&app)?;
    run_blocking(move || list_provider_profiles_with_repository(&repository)).await
}

#[tauri::command]
pub async fn probe_provider_profile(
    app: tauri::AppHandle,
    provider: Provider,
    path: String,
) -> Result<ProviderProfileProbe, String> {
    let repository = repository(&app)?;
    run_blocking(move || probe_provider_profile_with_repository(&repository, provider, &path)).await
}

#[tauri::command]
pub async fn create_provider_profile(
    app: tauri::AppHandle,
    provider: Provider,
    display_name: String,
    path: String,
) -> Result<ProviderProfile, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        create_provider_profile_with_repository(&repository, provider, &display_name, &path)
    })
    .await
}

#[tauri::command]
pub async fn rename_provider_profile(
    app: tauri::AppHandle,
    profile_id: LocalProviderProfileId,
    display_name: String,
    expected_revision: u64,
) -> Result<ProviderProfile, String> {
    rename_provider_profile_with_repository(
        &repository(&app)?,
        &profile_id,
        &display_name,
        expected_revision,
    )
}

#[tauri::command]
pub async fn remove_provider_profile(
    app: tauri::AppHandle,
    profile_id: LocalProviderProfileId,
    expected_revision: u64,
) -> Result<bool, String> {
    remove_provider_profile_with_repository(&repository(&app)?, &profile_id, expected_revision)
}

#[tauri::command]
pub async fn list_project_bindings(app: tauri::AppHandle) -> Result<Vec<ProjectBinding>, String> {
    Ok(repository(&app)?.load_bindings()?.bindings)
}

/// Binding lookup is by local project/replica registration, never by bundle
/// ID: one bundle may have multiple local replicas.
#[tauri::command]
pub async fn get_project_binding(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<Option<ProjectBinding>, String> {
    Ok(repository(&app)?
        .load_bindings()?
        .for_project(&local_project_id)
        .cloned())
}

#[tauri::command]
pub async fn audit_codex_conversation_paths(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<CodexConversationPathAudit, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        audit_codex_conversation_paths_with_repository(&repository, &local_project_id)
    })
    .await
}

#[tauri::command]
pub async fn repair_codex_conversation_paths(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<CodexConversationPathRepairResult, String> {
    let repository = repository(&app)?;
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(&local_project_id, None);
    log.emit(
        &app,
        "info",
        "repair.conversation_paths_started",
        "Repairing Codex conversation paths…",
    );
    let result = run_blocking(move || {
        repair_codex_conversation_paths_with_repository(&repository, &local_project_id)
    })
    .await;
    match &result {
        Ok(report) => log.emit(
            &app,
            "ok",
            "repair.conversation_paths_completed",
            &format!(
                "Repaired {} conversation path{}",
                report.repaired_thread_ids.len(),
                if report.repaired_thread_ids.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "repair.conversation_paths_failed",
            &format!("Conversation path repair failed: {error}"),
        ),
    }
    result
}

#[tauri::command]
pub async fn save_project_binding(
    app: tauri::AppHandle,
    request: SaveProjectBindingRequest,
) -> Result<ProjectBinding, String> {
    save_project_binding_with_repository(&repository(&app)?, request)
}

/// Detach rather than erase: apply receipts and per-replica baselines remain
/// addressable if the user later rebinds the project.
#[tauri::command]
pub async fn remove_project_binding(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<bool, String> {
    detach_project_binding_with_repository(&repository(&app)?, &local_project_id)
}

#[tauri::command]
pub async fn list_project_materializations(
    app: tauri::AppHandle,
    local_project_id: Option<LocalProjectId>,
) -> Result<Vec<MaterializationRecord>, String> {
    let records = repository(&app)?.load_materializations()?.records;
    Ok(match local_project_id {
        Some(id) => records
            .into_iter()
            .filter(|record| record.local_project_id == id)
            .collect(),
        None => records,
    })
}

#[tauri::command]
pub async fn get_restore_plan(
    app: tauri::AppHandle,
    plan_id: PlanId,
) -> Result<RestorePlan, String> {
    repository(&app)?.load_restore_plan(&plan_id)
}

#[tauri::command]
pub async fn discard_restore_plan(app: tauri::AppHandle, plan_id: PlanId) -> Result<bool, String> {
    repository(&app)?.discard_restore_plan(&plan_id)
}

#[tauri::command]
pub async fn discover_project(
    app: tauri::AppHandle,
    path: String,
    profile_ids: BTreeMap<Provider, LocalProviderProfileId>,
) -> Result<ProjectDiscovery, String> {
    let repository = repository(&app)?;
    run_blocking(move || discover_project_with_repository(&repository, &path, &profile_ids)).await
}

#[tauri::command]
pub async fn get_bundle_inventory(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<ResourceInventory, String> {
    let repository = repository(&app)?;
    run_blocking(move || get_bundle_inventory_with_repository(&repository, &local_project_id)).await
}

#[tauri::command]
pub async fn inspect_project_files(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ProjectContentInventory, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        inspect_project_files_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn list_remote_bundles(
    app: tauri::AppHandle,
    storage_id: StorageId,
    cursor: Option<String>,
) -> Result<BundlePage, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        list_remote_bundles_with_repository(&repository, &storage_id, cursor.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn list_remote_bundle_snapshots(
    app: tauri::AppHandle,
    storage_id: StorageId,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    let repository = repository(&app)?;
    run_blocking(move || list_remote_bundle_snapshots_with_repository(&repository, &storage_id))
        .await
}

#[tauri::command]
pub async fn find_remote_bundle_matches(
    app: tauri::AppHandle,
    storage_id: StorageId,
    repository_fingerprint: String,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        find_remote_bundle_matches_with_repository(
            &repository,
            &storage_id,
            &repository_fingerprint,
        )
    })
    .await
}

#[tauri::command]
pub async fn fetch_bundle(
    app: tauri::AppHandle,
    storage_id: StorageId,
    bundle_id: BundleId,
) -> Result<BundleSnapshotSummary, String> {
    let repository = repository(&app)?;
    let storage = storage_log_label(&repository, &storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Pull).storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "pull.fetch_started",
        &format!("↓  Fetching bundle {} from {}", bundle_id, storage),
    );
    let result = run_blocking(move || {
        let (_, fetched) = fetch_from_storage(&repository, &storage_id, &bundle_id)?;
        bundle_snapshot_summary(fetched)
    })
    .await;
    match &result {
        Ok(snapshot) => log.emit(
            &app,
            "ok",
            "pull.fetch_completed",
            &format!(
                "↓  Fetched {} generation {} ({} resources)",
                snapshot.display_name, snapshot.generation, snapshot.resource_count
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "pull.fetch_failed",
            &format!("✗  Pull fetch failed: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn get_bundle_status(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ResourceStatusReport, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_bundle_status_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn get_project_capability_status(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
) -> Result<CapabilityStatusReport, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_project_capability_status_with_repository(
            &repository,
            &local_project_id,
            storage_id.as_ref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn get_project_thread_sync_comparison(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<ThreadSyncComparison, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_project_thread_sync_comparison_with_repository(
            &repository,
            &local_project_id,
            &storage_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn push_bundle(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    recipe: Option<BundleRecipe>,
    project_content_review_token: Option<String>,
    project_content_removal_ids: Option<Vec<ResourceId>>,
    acknowledged_warning_digests: Option<Vec<String>>,
) -> Result<ProjectOperationResult, String> {
    let repository = repository(&app)?;
    let (project, storage) =
        project_storage_log_labels(&repository, &local_project_id, &storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(&local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "push.started",
        &format!("↑  Push started — {} → {}", project, storage),
    );
    let worker_app = app.clone();
    let worker_log = log.clone();
    let selected_count = recipe.as_ref().map(|recipe| recipe.entries.len());
    let result = run_blocking(move || {
        worker_log.emit(
            &worker_app,
            "info",
            "push.scan_started",
            &selected_count
                .map(|count| format!("   Scanning {} resources selected for this storage…", count))
                .unwrap_or_else(|| "   Scanning selected project resources…".to_string()),
        );
        push_bundle_reviewed_with_repository(
            &repository,
            &local_project_id,
            &storage_id,
            recipe,
            ProjectContentPushReview {
                review_token: project_content_review_token,
                removal_ids: project_content_removal_ids
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                acknowledged_warning_digests: acknowledged_warning_digests
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            },
        )
    })
    .await;
    match &result {
        Ok(operation) => {
            for resource in &operation.results {
                log.emit(
                    &app,
                    if resource.state == "synced" {
                        "ok"
                    } else {
                        "info"
                    },
                    if resource.state == "synced" {
                        "push.resource_synced"
                    } else {
                        "push.resource_checked"
                    },
                    &format!("↑  {} ({})", resource.resource_id, resource.state),
                );
            }
            log.emit(
                &app,
                "ok",
                "push.completed",
                &format!("✓  Push complete — {}", operation.message),
            );
        }
        Err(error) => log.emit(
            &app,
            "error",
            "push.failed",
            &format!("✗  Push failed: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn plan_bundle_restore(
    app: tauri::AppHandle,
    storage_id: StorageId,
    bundle_id: BundleId,
    binding: ProjectBinding,
) -> Result<RestorePlan, String> {
    let repository = repository(&app)?;
    let (project, storage) =
        project_storage_log_labels(&repository, &binding.local_project_id, &storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Pull)
        .project(&binding.local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "pull.review_started",
        &format!("↓  Preparing Pull review — {} ← {}", project, storage),
    );
    let result = run_blocking(move || {
        plan_bundle_restore_with_repository(&repository, &storage_id, &bundle_id, &binding)
    })
    .await;
    match &result {
        Ok(plan) => {
            for action in &plan.actions {
                log.emit(
                    &app,
                    "info",
                    "pull.resource_review",
                    &format!("↓  Review {}", action.resource_id),
                );
            }
            log.emit(
                &app,
                "ok",
                "pull.plan_ready",
                &format!(
                    "✓  Pull plan ready — generation {}, {} actions",
                    plan.generation,
                    plan.actions.len()
                ),
            );
            log.emit(
                &app,
                "info",
                "pull.awaiting_approval",
                "   Nothing has been applied yet — use the Apply button in the Pull review.",
            );
        }
        Err(error) => log.emit(
            &app,
            "error",
            "pull.planning_failed",
            &format!("✗  Pull planning failed: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn apply_bundle_restore(
    app: tauri::AppHandle,
    plan_id: PlanId,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let repository = repository(&app)?;
    let log_repository = repository.clone();
    let plan_for_log = repository.load_restore_plan(&plan_id).ok();
    let log = ActivityLogScope::new(ActivityLogType::Pull);
    log.emit(
        &app,
        "info",
        "pull.apply_started",
        &format!(
            "↓  Applying {} approved Pull actions…",
            approved_action_ids.len()
        ),
    );
    let result = run_blocking(move || {
        apply_bundle_restore_with_repository(&repository, &plan_id, approved_action_ids)
    })
    .await;
    match &result {
        Ok(operation) => {
            for action_id in &operation.applied_action_ids {
                let resource = plan_for_log
                    .as_ref()
                    .and_then(|plan| {
                        plan.actions
                            .iter()
                            .find(|action| &action.action_id == action_id)
                    })
                    .map(|action| action.resource_id.to_string())
                    .unwrap_or_else(|| action_id.to_string());
                log.emit(
                    &app,
                    "ok",
                    "pull.resource_applied",
                    &format!("↓  {}", resource),
                );
            }
            for failure in &operation.failed_actions {
                log.emit(
                    &app,
                    "error",
                    "pull.resource_failed",
                    &format!("✗  {}: {}", failure.action_id, failure.message),
                );
            }
            log.emit(
                &app,
                if operation.success { "ok" } else { "error" },
                if operation.success {
                    "pull.completed"
                } else {
                    "pull.completed_with_errors"
                },
                &format!(
                    "{}  Pull finished — {}",
                    if operation.success { "✓" } else { "✗" },
                    operation.message
                ),
            );
            if operation.success {
                match plan_for_log
                    .as_ref()
                    .ok_or_else(|| "restore plan is unavailable".to_string())
                    .and_then(|plan| current_binding_for_restore_plan(&log_repository, plan))
                {
                    Ok(binding) => {
                        log.emit(
                            &app,
                            "info",
                            "pull.open_project_hint",
                            "   Open this project after Pull:",
                        );
                        for (label, command) in project_open_commands(&binding) {
                            log.emit(
                                &app,
                                "info",
                                "pull.open_project_command",
                                &format!("   {}: {}", label, command),
                            );
                        }
                    }
                    Err(error) => log.emit(
                        &app,
                        "info",
                        "pull.open_project_unavailable",
                        &format!("   Open command unavailable: {}", error),
                    ),
                }
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "pull.failed",
            &format!("✗  Pull failed: {}", error),
        ),
    }
    result
}

#[tauri::command]
pub async fn plan_dependencies(
    app: tauri::AppHandle,
    restore_plan_id: PlanId,
) -> Result<DependencyPlan, String> {
    let repository = repository(&app)?;
    run_blocking(move || plan_dependencies_with_repository(&repository, &restore_plan_id)).await
}

#[tauri::command]
pub async fn apply_dependency_actions(
    app: tauri::AppHandle,
    plan_id: PlanId,
    action_ids: Vec<ActionId>,
) -> Result<DependencyResult, String> {
    apply_dependency_actions_with_repository(&repository(&app)?, &plan_id, action_ids).await
}

#[tauri::command]
pub async fn get_bundle_readiness(
    app: tauri::AppHandle,
    storage_id: StorageId,
    bundle_id: BundleId,
    binding: ProjectBinding,
) -> Result<BundleReadiness, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_bundle_readiness_with_repository(&repository, &storage_id, &bundle_id, &binding)
    })
    .await
}

#[tauri::command]
pub async fn get_restore_readiness(
    app: tauri::AppHandle,
    restore_plan_id: PlanId,
) -> Result<BundleReadiness, String> {
    let repository = repository(&app)?;
    run_blocking(move || get_restore_readiness_with_repository(&repository, &restore_plan_id)).await
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "Codex",
        Provider::Claude => "Claude",
    }
}

fn provider_default_directory(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => ".codex",
        Provider::Claude => ".claude",
    }
}

fn suggested_profile_name(provider: Provider, path: &Path) -> String {
    let default = dirs::home_dir().map(|home| home.join(provider_default_directory(provider)));
    if default.as_ref().is_some_and(|default| {
        prospective_canonical(default).ok().as_ref() == prospective_canonical(path).ok().as_ref()
    }) {
        return format!("Default {}", provider_name(provider));
    }
    let stem = if path
        .file_name()
        .is_some_and(|name| name == provider_default_directory(provider))
    {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_name()
    };
    stem.and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("{} · {}", name, provider_name(provider)))
        .unwrap_or_else(|| format!("{} profile", provider_name(provider)))
}

fn probe_provider_profile_with_repository(
    repository: &V3Repository,
    provider: Provider,
    requested_path: &str,
) -> Result<ProviderProfileProbe, String> {
    validate_absolute_clean_path("provider profile path", requested_path)?;
    let requested = PathBuf::from(requested_path);
    let child = requested.join(provider_default_directory(provider));
    let (resolved, detected_child) = if requested
        .file_name()
        .is_none_or(|name| name != provider_default_directory(provider))
        && child.is_dir()
    {
        (child, true)
    } else {
        (requested, false)
    };
    let metadata = fs_profile_metadata(&resolved)?;
    if !metadata.is_dir() {
        return Err(format!(
            "{} profile '{}' is not a directory",
            provider_name(provider),
            resolved.display()
        ));
    }
    let canonical = fs_canonicalize(&resolved)?;
    let repository_root = prospective_canonical(repository.root())?;
    if paths_overlap(&canonical, &repository_root) {
        return Err(format!(
            "{} profile '{}' overlaps schema-3 application data",
            provider_name(provider),
            canonical.display()
        ));
    }
    let config = repository.load_config()?;
    for storage in config
        .storages
        .iter()
        .filter(|storage| storage.kind == StorageKind::Local)
    {
        let storage_root = prospective_canonical(Path::new(&storage.local_dir))?;
        if paths_overlap(&canonical, &storage_root) {
            return Err(format!(
                "{} profile '{}' overlaps local storage '{}'",
                provider_name(provider),
                canonical.display(),
                storage.name
            ));
        }
    }
    let readable = fs::read_dir(&canonical).is_ok();
    let writable = !metadata.permissions().readonly();
    let state = repository.load_bindings()?;
    for binding in state
        .bindings
        .iter()
        .filter(|binding| binding.state == BindingState::Active)
    {
        let project_root = Path::new(&binding.canonical_project_root);
        if paths_overlap(&canonical, project_root) {
            return Err(format!(
                "{} profile '{}' overlaps project '{}'",
                provider_name(provider),
                canonical.display(),
                binding.local_project_id
            ));
        }
    }
    let existing_profile_id = state
        .profiles
        .iter()
        .find(|profile| {
            profile.provider == provider && Path::new(&profile.canonical_path) == canonical
        })
        .map(|profile| profile.profile_id.clone());
    Ok(ProviderProfileProbe {
        provider,
        requested_path: requested_path.to_string(),
        resolved_path: resolved.to_string_lossy().into_owned(),
        canonical_path: canonical.to_string_lossy().into_owned(),
        suggested_name: suggested_profile_name(provider, &resolved),
        readable,
        writable,
        detected_child,
        existing_profile_id,
    })
}

fn create_provider_profile_with_repository(
    repository: &V3Repository,
    provider: Provider,
    display_name: &str,
    path: &str,
) -> Result<ProviderProfile, String> {
    let probe = probe_provider_profile_with_repository(repository, provider, path)?;
    if !probe.readable {
        return Err(format!(
            "{} profile '{}' is not readable",
            provider_name(provider),
            probe.resolved_path
        ));
    }
    if let Some(profile_id) = &probe.existing_profile_id {
        return repository
            .load_bindings()?
            .profiles
            .into_iter()
            .find(|profile| &profile.profile_id == profile_id)
            .ok_or_else(|| "provider profile disappeared during creation".to_string());
    }
    let now = now_secs();
    let profile = ProviderProfile {
        profile_id: LocalProviderProfileId::parse(generated_named_id("profile")?)?,
        provider,
        display_name: if display_name.trim().is_empty() {
            probe.suggested_name
        } else {
            display_name.trim().to_string()
        },
        path: probe.resolved_path,
        canonical_path: probe.canonical_path,
        revision: 0,
        created_at: now,
        updated_at: now,
    };
    profile.validate_structure()?;
    repository.mutate_bindings(|_, state| {
        if let Some(existing) = state.profiles.iter().find(|existing| {
            existing.provider == profile.provider
                && existing.canonical_path == profile.canonical_path
        }) {
            return Ok(existing.clone());
        }
        state.profiles.push(profile.clone());
        Ok(profile)
    })
}

fn ensure_default_provider_profiles(repository: &V3Repository) -> Result<(), String> {
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    for provider in [Provider::Codex, Provider::Claude] {
        let path = home.join(provider_default_directory(provider));
        if path.is_dir() {
            if probe_provider_profile_with_repository(repository, provider, &path.to_string_lossy())
                .is_err()
            {
                continue;
            }
            let _ = create_provider_profile_with_repository(
                repository,
                provider,
                &format!("Default {}", provider_name(provider)),
                &path.to_string_lossy(),
            )?;
        }
    }
    Ok(())
}

fn inspect_provider_profile(profile: &ProviderProfile) -> (bool, bool, bool, Option<String>) {
    let path = Path::new(&profile.path);
    let result = (|| {
        let metadata = fs_profile_metadata(path)?;
        if !metadata.is_dir() {
            return Err("profile path is not a directory".to_string());
        }
        let canonical = fs_canonicalize(path)?;
        if canonical != PathBuf::from(&profile.canonical_path) {
            return Err("profile path now resolves to a different directory".to_string());
        }
        let readable = fs::read_dir(&canonical).is_ok();
        Ok((readable, !metadata.permissions().readonly()))
    })();
    match result {
        Ok((readable, writable)) => (true, readable, writable, None),
        Err(error) => (false, false, false, Some(error)),
    }
}

fn list_provider_profiles_with_repository(
    repository: &V3Repository,
) -> Result<Vec<ProviderProfileSummary>, String> {
    ensure_default_provider_profiles(repository)?;
    let state = repository.load_bindings()?;
    let mut profiles = state
        .profiles
        .iter()
        .cloned()
        .map(|profile| {
            let (available, readable, writable, error) = inspect_provider_profile(&profile);
            let used_by_projects = state
                .bindings
                .iter()
                .filter(|binding| {
                    binding.state == BindingState::Active
                        && binding
                            .profile_ids
                            .values()
                            .any(|id| id == &profile.profile_id)
                })
                .map(|binding| binding.local_project_id.clone())
                .collect();
            ProviderProfileSummary {
                profile,
                available,
                readable,
                writable,
                used_by_projects,
                error,
            }
        })
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        left.profile
            .provider
            .cmp(&right.profile.provider)
            .then_with(|| left.profile.display_name.cmp(&right.profile.display_name))
    });
    Ok(profiles)
}

fn rename_provider_profile_with_repository(
    repository: &V3Repository,
    profile_id: &LocalProviderProfileId,
    display_name: &str,
    expected_revision: u64,
) -> Result<ProviderProfile, String> {
    let name = display_name.trim();
    if name.is_empty() {
        return Err("provider profile name cannot be empty".to_string());
    }
    repository.mutate_bindings(|_, state| {
        let profile = state
            .profiles
            .iter_mut()
            .find(|profile| &profile.profile_id == profile_id)
            .ok_or_else(|| format!("unknown provider profile '{}'", profile_id))?;
        if profile.revision != expected_revision {
            return Err(format!(
                "provider profile changed (expected revision {}, current {})",
                expected_revision, profile.revision
            ));
        }
        profile.display_name = name.to_string();
        profile.revision = profile.revision.saturating_add(1);
        profile.updated_at = now_secs();
        profile.validate_structure()?;
        Ok(profile.clone())
    })
}

fn remove_provider_profile_with_repository(
    repository: &V3Repository,
    profile_id: &LocalProviderProfileId,
    expected_revision: u64,
) -> Result<bool, String> {
    repository.mutate_bindings(|_, state| {
        let Some(profile) = state
            .profiles
            .iter()
            .find(|profile| &profile.profile_id == profile_id)
        else {
            return Ok(false);
        };
        if profile.revision != expected_revision {
            return Err(format!(
                "provider profile changed (expected revision {}, current {})",
                expected_revision, profile.revision
            ));
        }
        let used_by = state
            .bindings
            .iter()
            .filter(|binding| {
                binding.state == BindingState::Active
                    && binding.profile_ids.values().any(|id| id == profile_id)
            })
            .map(|binding| binding.local_project_id.to_string())
            .collect::<Vec<_>>();
        if !used_by.is_empty() {
            return Err(format!(
                "provider profile '{}' is used by project(s): {}",
                profile.display_name,
                used_by.join(", ")
            ));
        }
        state
            .profiles
            .retain(|profile| &profile.profile_id != profile_id);
        Ok(true)
    })
}

fn resolve_profile_paths(
    repository: &V3Repository,
    profile_ids: &BTreeMap<Provider, LocalProviderProfileId>,
) -> Result<BTreeMap<Provider, String>, String> {
    if profile_ids.is_empty() {
        return Err("choose at least one local provider profile".to_string());
    }
    let state = repository.load_bindings()?;
    let mut resolved = BTreeMap::new();
    for (provider, profile_id) in profile_ids {
        let profile = state
            .profiles
            .iter()
            .find(|profile| &profile.profile_id == profile_id)
            .ok_or_else(|| format!("unknown provider profile '{}'", profile_id))?;
        if &profile.provider != provider {
            return Err(format!(
                "{} cannot use {} profile '{}'",
                provider_name(*provider),
                provider_name(profile.provider),
                profile.display_name
            ));
        }
        let (available, readable, _, error) = inspect_provider_profile(profile);
        if !available || !readable {
            return Err(error.unwrap_or_else(|| {
                format!(
                    "{} profile '{}' is not readable",
                    provider_name(*provider),
                    profile.path
                )
            }));
        }
        resolved.insert(*provider, profile.path.clone());
    }
    Ok(resolved)
}

fn discover_project_with_repository(
    repository: &V3Repository,
    selected_path: &str,
    profile_ids: &BTreeMap<Provider, LocalProviderProfileId>,
) -> Result<ProjectDiscovery, String> {
    let profile_paths = resolve_profile_paths(repository, profile_ids)?;
    discover_project_at(repository, selected_path, &profile_paths, profile_ids)
}

/// Discovery core shared by profile-ID discovery and setup drafts, which may
/// mix existing profiles with pending, not-yet-created profile paths.
fn discover_project_at(
    repository: &V3Repository,
    selected_path: &str,
    profile_paths: &BTreeMap<Provider, String>,
    profile_ids: &BTreeMap<Provider, LocalProviderProfileId>,
) -> Result<ProjectDiscovery, String> {
    validate_absolute_clean_path("project root", selected_path)?;
    let project_root = fs_canonicalize(Path::new(selected_path))?;
    if !project_root.is_dir() {
        return Err(format!(
            "project root '{}' is not a directory",
            selected_path
        ));
    }
    let repository_root = prospective_canonical(repository.root())?;
    if paths_overlap(&project_root, &repository_root) {
        return Err("project root overlaps schema-3 application data".to_string());
    }
    let config = repository.load_config()?;
    for storage in config
        .storages
        .iter()
        .filter(|storage| storage.kind == StorageKind::Local)
    {
        let storage_root = prospective_canonical(Path::new(&storage.local_dir))?;
        if paths_overlap(&project_root, &storage_root) {
            return Err(format!(
                "project root '{}' overlaps local storage '{}'",
                project_root.display(),
                storage.local_dir
            ));
        }
    }

    if profile_paths.is_empty() {
        return Err("choose at least one local provider profile".to_string());
    }
    for (provider, path) in profile_paths {
        let path = fs_canonicalize(Path::new(path))?;
        if paths_overlap(&project_root, &path) {
            return Err(format!(
                "project root '{}' overlaps {} profile '{}'",
                project_root.display(),
                provider_name(*provider),
                path.display()
            ));
        }
    }
    let request = capture_request_with_global_inventory(
        project_root.clone(),
        profile_paths.get(&Provider::Codex).map(PathBuf::from),
        profile_paths.get(&Provider::Claude).map(PathBuf::from),
        Vec::new(),
    );
    let discovered = provider_capture::discover_project(&request)?;
    let recipe = default_recipe(&discovered.resources)?;
    let inventory = inventory_from_candidates(
        project_root.to_string_lossy().into_owned(),
        None,
        &discovered.resources,
        recipe,
        discovered.warnings.clone(),
    )?;
    let providers = inventory_providers(&inventory.resources);
    let display_name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Project")
        .to_string();
    Ok(ProjectDiscovery {
        project_root: project_root.to_string_lossy().into_owned(),
        display_name,
        inventory,
        repository_fingerprint: repository_fingerprint(&project_root),
        providers,
        profile_ids: profile_ids.clone(),
        warnings: discovered.warnings,
    })
}

#[derive(Clone)]
struct ProjectContentVersion {
    descriptor: ResourceDescriptor,
    entry_type: ProjectContentEntryType,
    relative_path: String,
    logical_path: String,
    size: Option<u64>,
    mode: Option<u32>,
    source_mtime: Option<u64>,
    digest: String,
}

fn inspect_project_files_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectContentInventory, String> {
    let _host_operation = repository.acquire_host_operation()?;
    inspect_project_files_guarded(repository, local_project_id, storage_id)
}

fn inspect_project_files_guarded(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectContentInventory, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    require_project_link(&config, &project, storage_id)?;
    let link = config
        .links
        .iter()
        .find(|link| link.local_project_id == *local_project_id && link.storage_id == *storage_id)
        .cloned()
        .ok_or_else(|| "project storage link disappeared".to_string())?;
    let binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    let eligibility = project_file_sync_eligibility(&binding);
    let (_, engine) = storage_engine(repository, storage_id)?;
    let remote = engine.inspect_optional(&project.bundle_id)?;
    let remote_versions = remote
        .as_ref()
        .map(|snapshot| project_content_versions(&snapshot.manifest))
        .transpose()?
        .unwrap_or_default();
    let mut warnings = Vec::new();
    let (base_versions, base_unavailable) = match project.recipe_bases.get(storage_id) {
        Some(base) => match base.commit_id.as_deref() {
            Some(commit_id) => match engine.inspect_manifest_version(
                &project.bundle_id,
                base.generation,
                commit_id,
                &base.manifest_sha256,
            ) {
                Ok(manifest) => (project_content_versions(&manifest)?, false),
                Err(error) => {
                    warnings.push(format!(
                        "Reviewed project-content base is unavailable: {}",
                        error
                    ));
                    (BTreeMap::new(), true)
                }
            },
            None => {
                warnings
                    .push("Reviewed project-content base has no immutable commit ID".to_string());
                (BTreeMap::new(), true)
            }
        },
        None => (BTreeMap::new(), false),
    };
    let effective_recipe = link
        .recipe
        .clone()
        .unwrap_or_else(|| project.recipe.clone());
    let selected_ids = effective_recipe.selected_ids();
    let preferences = link.project_content_preferences.clone();

    let mut local_versions = BTreeMap::<ResourceId, ProjectContentVersion>::new();
    let mut local_blocked_versions = BTreeMap::<ResourceId, ProjectContentVersion>::new();
    let mut local_blockers = BTreeMap::<ResourceId, String>::new();
    let mut local_warnings = BTreeMap::<ResourceId, String>::new();
    let mut ignored_count = 0;
    let mut blocked_count = 0;
    if eligibility.state == ProjectFileSyncEligibilityState::Eligible {
        let mut request = capture_request_for_binding(repository, &binding)?;
        request.include_project_content = true;
        let discovered = provider_capture::discover_project(&request)?;
        ignored_count = discovered.ignored_count;
        blocked_count = discovered.blocked_count;
        warnings.extend(discovered.warnings.clone());
        let candidates = discovered
            .resources
            .into_iter()
            .filter(|candidate| {
                matches!(
                    candidate.kind,
                    CaptureResourceKind::ProjectContentFile
                        | CaptureResourceKind::ProjectContentDirectory
                )
            })
            .collect::<Vec<_>>();
        let local_inventory = inventory_from_candidates(
            binding.project_root.clone(),
            Some(project.bundle_id.clone()),
            &candidates,
            BundleRecipe::default(),
            Vec::new(),
        )?;
        for item in local_inventory.resources {
            let resource_id = item.descriptor.resource_id.clone();
            let relative_path = item
                .descriptor
                .metadata
                .get("_local_relative_path")
                .cloned()
                .or_else(|| match &item.descriptor.provenance {
                    Provenance::ProjectLocal { relative_path } => Some(relative_path.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    format!("project-content resource '{}' lacks a path", resource_id)
                })?;
            if let Some(reason) = item.blocked_reason {
                local_blockers.insert(resource_id.clone(), reason);
                local_blocked_versions.insert(
                    resource_id,
                    ProjectContentVersion {
                        logical_path: format!("project/{}", relative_path),
                        relative_path,
                        entry_type: ProjectContentEntryType::Blocked,
                        size: None,
                        mode: None,
                        source_mtime: None,
                        descriptor: item.descriptor,
                        digest: String::new(),
                    },
                );
                continue;
            }
            let entry_type = match item.descriptor.kind {
                ResourceKind::ProjectContentFile => ProjectContentEntryType::File,
                ResourceKind::ProjectContentDirectory => ProjectContentEntryType::Directory,
                _ => continue,
            };
            let digest = item
                .descriptor
                .metadata
                .get("_local_review_digest")
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "project-content resource '{}' lacks a review digest",
                        resource_id
                    )
                })?;
            if let Some(warning) = item.descriptor.metadata.get("_local_warning_code") {
                local_warnings.insert(resource_id.clone(), warning.clone());
            }
            local_versions.insert(
                resource_id,
                ProjectContentVersion {
                    logical_path: format!("project/{}", relative_path),
                    relative_path,
                    entry_type,
                    size: parse_optional_metadata_u64(&item.descriptor, "_local_size")?,
                    mode: parse_optional_metadata_u32(&item.descriptor, "_local_mode")?,
                    source_mtime: parse_optional_metadata_u64(
                        &item.descriptor,
                        "_local_source_mtime",
                    )?,
                    descriptor: item.descriptor,
                    digest,
                },
            );
        }
    }

    let all_ids = local_versions
        .keys()
        .chain(local_blocked_versions.keys())
        .chain(local_blockers.keys())
        .chain(remote_versions.keys())
        .chain(base_versions.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::with_capacity(all_ids.len());
    for resource_id in all_ids {
        let local = local_versions.get(&resource_id);
        let local_blocked = local_blocked_versions.get(&resource_id);
        let remote_version = remote_versions.get(&resource_id);
        let base = base_versions.get(&resource_id);
        let blocked_reason = local_blockers.get(&resource_id).cloned();
        let representative = local.or(local_blocked).or(remote_version).or(base);
        let Some(representative) = representative else {
            continue;
        };
        let local_digest = local.map(|version| version.digest.clone());
        let storage_digest = remote_version.map(|version| version.digest.clone());
        let base_digest = base.map(|version| version.digest.clone());
        let state = if blocked_reason.is_some() {
            "blocked"
        } else if base_unavailable
            && (remote_version.is_some() || selected_ids.contains(&resource_id))
        {
            "unknown"
        } else {
            classify_project_content_state(
                local_digest.as_deref(),
                storage_digest.as_deref(),
                base_digest.as_deref(),
            )
        };
        let selected_in_recipe = selected_ids.contains(&resource_id);
        let newly_discovered = local.is_some() && remote_version.is_none() && !selected_in_recipe;
        let selected_after_scan = newly_discovered
            && blocked_reason.is_none()
            && !preferences.excluded_resource_ids.contains(&resource_id);
        let warning_code = local_warnings.get(&resource_id).cloned();
        let warning_digest = warning_code.as_ref().and_then(|warning| {
            local_digest
                .as_ref()
                .map(|digest| project_content_warning_digest(&resource_id, digest, warning))
        });
        entries.push(ProjectContentEntry {
            descriptor: representative.descriptor.clone(),
            entry_type: if blocked_reason.is_some() {
                ProjectContentEntryType::Blocked
            } else {
                representative.entry_type
            },
            relative_path: representative.relative_path.clone(),
            logical_path: representative.logical_path.clone(),
            size: local
                .or(local_blocked)
                .or(remote_version)
                .and_then(|version| version.size),
            mode: local
                .or(local_blocked)
                .or(remote_version)
                .and_then(|version| version.mode),
            source_mtime: local
                .or(local_blocked)
                .or(remote_version)
                .and_then(|version| version.source_mtime),
            state: state.to_string(),
            local_present: local.is_some() || local_blocked.is_some(),
            storage_present: remote_version.is_some(),
            base_present: base.is_some(),
            local_digest: local_digest.clone(),
            storage_digest,
            base_digest,
            review_digest: local_digest,
            selected_in_recipe,
            newly_discovered,
            selected_after_scan,
            blocked_reason,
            warning_code,
            warning_digest,
        });
    }
    entries.sort_by(|left, right| {
        left.relative_path.cmp(&right.relative_path).then_with(|| {
            project_content_entry_rank(left.entry_type)
                .cmp(&project_content_entry_rank(right.entry_type))
        })
    });
    warnings.sort();
    warnings.dedup();
    let review_token =
        (eligibility.state == ProjectFileSyncEligibilityState::Eligible).then(|| {
            project_content_review_token(
                &binding,
                &eligibility,
                remote.as_ref(),
                preferences.revision,
                &entries,
            )
        });
    Ok(ProjectContentInventory {
        local_project_id: local_project_id.clone(),
        storage_id: storage_id.clone(),
        project_root: binding.project_root,
        eligibility,
        review_token,
        storage_generation: remote.as_ref().map(|snapshot| snapshot.head.generation),
        preference_revision: preferences.revision,
        excluded_resource_ids: preferences.excluded_resource_ids.into_iter().collect(),
        entries,
        ignored_count,
        blocked_count,
        warnings,
        scanned_at: now_secs(),
    })
}

fn project_content_entry_rank(entry_type: ProjectContentEntryType) -> u8 {
    match entry_type {
        ProjectContentEntryType::Directory => 0,
        ProjectContentEntryType::File => 1,
        ProjectContentEntryType::Blocked => 2,
    }
}

fn parse_optional_metadata_u64(
    descriptor: &ResourceDescriptor,
    key: &str,
) -> Result<Option<u64>, String> {
    descriptor
        .metadata
        .get(key)
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                format!(
                    "resource '{}' has invalid {}: {}",
                    descriptor.resource_id, key, error
                )
            })
        })
        .transpose()
}

fn parse_optional_metadata_u32(
    descriptor: &ResourceDescriptor,
    key: &str,
) -> Result<Option<u32>, String> {
    descriptor
        .metadata
        .get(key)
        .map(|value| {
            value.parse::<u32>().map_err(|error| {
                format!(
                    "resource '{}' has invalid {}: {}",
                    descriptor.resource_id, key, error
                )
            })
        })
        .transpose()
}

fn project_content_versions(
    manifest: &BundleManifest,
) -> Result<BTreeMap<ResourceId, ProjectContentVersion>, String> {
    let mut versions = BTreeMap::new();
    for (logical_path, entry) in &manifest.files {
        let Some(descriptor) = manifest.resources.get(&entry.resource_id) else {
            continue;
        };
        if descriptor.kind != ResourceKind::ProjectContentFile {
            continue;
        }
        let relative_path = project_content_relative_path(logical_path)?;
        let mode = entry.mode.unwrap_or(0o600) & 0o777;
        let version = ProjectContentVersion {
            descriptor: descriptor.clone(),
            entry_type: ProjectContentEntryType::File,
            logical_path: logical_path.to_string(),
            relative_path: relative_path.clone(),
            size: Some(entry.size),
            mode: Some(mode),
            source_mtime: Some(entry.source_mtime),
            digest: project_content_version_digest("file", &relative_path, &entry.sha256, mode),
        };
        if versions
            .insert(entry.resource_id.clone(), version)
            .is_some()
        {
            return Err(format!(
                "project-content resource '{}' owns more than one entry",
                entry.resource_id
            ));
        }
    }
    for (logical_path, entry) in &manifest.directories {
        let Some(descriptor) = manifest.resources.get(&entry.resource_id) else {
            continue;
        };
        if descriptor.kind != ResourceKind::ProjectContentDirectory {
            continue;
        }
        let relative_path = project_content_relative_path(logical_path)?;
        let mode = entry.mode.unwrap_or(0o700) & 0o777;
        let version = ProjectContentVersion {
            descriptor: descriptor.clone(),
            entry_type: ProjectContentEntryType::Directory,
            logical_path: logical_path.to_string(),
            relative_path: relative_path.clone(),
            size: None,
            mode: Some(mode),
            source_mtime: Some(entry.source_mtime),
            digest: project_content_version_digest("dir", &relative_path, "", mode),
        };
        if versions
            .insert(entry.resource_id.clone(), version)
            .is_some()
        {
            return Err(format!(
                "project-content resource '{}' owns more than one entry",
                entry.resource_id
            ));
        }
    }
    Ok(versions)
}

fn project_content_relative_path(
    logical_path: &super::domain::LogicalPath,
) -> Result<String, String> {
    logical_path
        .as_str()
        .strip_prefix("project/")
        .filter(|relative| !relative.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("project-content path '{}' is invalid", logical_path))
}

fn project_content_version_digest(
    entry_type: &str,
    relative_path: &str,
    content_sha256: &str,
    mode: u32,
) -> String {
    let digest = Sha256::digest(
        format!(
            "{}\0{}\0{}\0{:03o}",
            entry_type,
            relative_path,
            content_sha256,
            mode & 0o777
        )
        .as_bytes(),
    );
    hex_digest(&digest)
}

fn project_content_ancestor_paths(relative_path: &str) -> Vec<String> {
    let components = relative_path.split('/').collect::<Vec<_>>();
    (1..components.len())
        .map(|end| components[..end].join("/"))
        .collect()
}

fn classify_project_content_state(
    local: Option<&str>,
    storage: Option<&str>,
    base: Option<&str>,
) -> &'static str {
    match (local, storage, base) {
        (Some(local), Some(storage), _) if local == storage => "synced",
        (Some(_), None, None) => "local_only",
        (None, Some(_), None) => "storage_only",
        (Some(local), Some(storage), Some(base)) if storage == base && local != base => {
            "local_ahead"
        }
        (Some(local), Some(storage), Some(base)) if local == base && storage != base => {
            "storage_ahead"
        }
        (Some(_), Some(_), _) => "diverged",
        (None, Some(storage), Some(base)) if storage == base => "missing",
        (None, Some(_), Some(_)) => "storage_ahead",
        (Some(local), None, Some(base)) if local == base => "storage_ahead",
        (Some(_), None, Some(_)) => "diverged",
        (None, None, Some(_)) => "storage_ahead",
        (None, None, None) => "unknown",
    }
}

fn project_content_review_token(
    binding: &ProjectBinding,
    eligibility: &ProjectFileSyncEligibility,
    remote: Option<&BundleSnapshot>,
    preference_revision: u64,
    entries: &[ProjectContentEntry],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"project-content-review-v1\0");
    hasher.update(binding.local_project_id.as_str().as_bytes());
    hasher.update(binding.host_id.as_str().as_bytes());
    hasher.update(binding.replica_id.as_str().as_bytes());
    hasher.update(binding.revision.to_be_bytes());
    hasher.update(format!("{:?}", eligibility.state).as_bytes());
    hasher.update(preference_revision.to_be_bytes());
    if let Some(remote) = remote {
        hasher.update(remote.head.generation.to_be_bytes());
        hasher.update(remote.head.commit_id.as_bytes());
        hasher.update(remote.head.manifest_sha256.as_bytes());
    }
    for entry in entries {
        hasher.update(entry.descriptor.resource_id.as_str().as_bytes());
        hasher.update(entry.relative_path.as_bytes());
        hasher.update(entry.state.as_bytes());
        if let Some(digest) = &entry.review_digest {
            hasher.update(digest.as_bytes());
        }
        if let Some(reason) = &entry.blocked_reason {
            hasher.update(reason.as_bytes());
        }
        if let Some(warning) = &entry.warning_code {
            hasher.update(warning.as_bytes());
        }
    }
    hex_digest(&hasher.finalize())
}

fn project_content_warning_digest(
    resource_id: &ResourceId,
    review_digest: &str,
    warning_code: &str,
) -> String {
    let digest = Sha256::digest(
        format!(
            "project-content-warning-v1\0{}\0{}\0{}",
            resource_id, review_digest, warning_code
        )
        .as_bytes(),
    );
    hex_digest(&digest)
}

fn project_file_sync_eligibility(binding: &ProjectBinding) -> ProjectFileSyncEligibility {
    let root = Path::new(&binding.project_root);
    let canonical = match fs::canonicalize(root) {
        Ok(canonical) => canonical,
        Err(error) => {
            return ProjectFileSyncEligibility {
                state: ProjectFileSyncEligibilityState::Unknown,
                reason: format!("Project folder cannot be resolved: {}", error),
                detected_root: None,
            }
        }
    };
    if canonical != PathBuf::from(&binding.canonical_project_root) {
        return ProjectFileSyncEligibility {
            state: ProjectFileSyncEligibilityState::Unknown,
            reason: "Project binding resolves to a different folder".to_string(),
            detected_root: None,
        };
    }
    match std::process::Command::new("git")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let detected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            ProjectFileSyncEligibility {
                state: ProjectFileSyncEligibilityState::GitManaged,
                reason: "Git manages files in this project folder".to_string(),
                detected_root: (!detected.is_empty()).then_some(detected),
            }
        }
        Ok(output)
            if String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("not a git repository") =>
        {
            ProjectFileSyncEligibility {
                state: ProjectFileSyncEligibilityState::Eligible,
                reason: "Project folder is not inside a Git work tree".to_string(),
                detected_root: None,
            }
        }
        Ok(output) => ProjectFileSyncEligibility {
            state: ProjectFileSyncEligibilityState::Unknown,
            reason: format!(
                "Git eligibility probe failed with status {}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            detected_root: None,
        },
        Err(error) => ProjectFileSyncEligibility {
            state: ProjectFileSyncEligibilityState::Unknown,
            reason: format!("Git eligibility probe could not run: {}", error),
            detected_root: None,
        },
    }
}

fn get_bundle_inventory_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<ResourceInventory, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    let bindings = repository.load_bindings()?;
    let binding = bindings.active_for(local_project_id).ok_or_else(|| {
        format!(
            "project '{}' is not mapped on this machine",
            local_project_id
        )
    })?;
    let request = capture_request_for_binding(repository, binding)?;
    let discovered = provider_capture::discover_project(&request)?;
    let project = persist_auto_selected_conversations(
        repository,
        &project.local_project_id,
        &discovered.resources,
    )?;
    let mut inventory = inventory_from_candidates(
        binding.project_root.clone(),
        Some(project.bundle_id.clone()),
        &discovered.resources,
        project.recipe.clone(),
        discovered.warnings,
    )?;
    let visible: BTreeSet<_> = inventory
        .resources
        .iter()
        .map(|resource| resource.descriptor.resource_id.clone())
        .collect();
    for (resource_id, entry) in &project.recipe.entries {
        if visible.contains(resource_id) {
            continue;
        }
        inventory.resources.push(InventoryResource {
            descriptor: ResourceDescriptor {
                resource_id: resource_id.clone(),
                kind: ResourceKind::Requirement,
                provider: None,
                scope: ResourceScope::Requirement,
                display_name: resource_id.to_string(),
                provenance: Provenance::Unknown,
                apply_policy: entry.apply_policy,
                relative_cwd: None,
                codec_version: 1,
                metadata: BTreeMap::new(),
            },
            category: "tools".to_string(),
            description: Some("Selected resource is unavailable on this machine".to_string()),
            logical_paths: Vec::new(),
            default_selected: true,
            blocked_reason: Some("Resource is unavailable at the mapped paths".to_string()),
            provided_by: None,
            install_behavior: None,
        });
    }
    inventory.resources.sort_by(|left, right| {
        left.descriptor
            .resource_id
            .cmp(&right.descriptor.resource_id)
    });
    Ok(inventory)
}

fn inventory_from_candidates(
    project: String,
    bundle_id: Option<BundleId>,
    candidates: &[ResourceCandidate],
    recipe: BundleRecipe,
    warnings: Vec<String>,
) -> Result<ResourceInventory, String> {
    let mut resources = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let resource_id = ResourceId::parse(candidate.resource_id.clone())?;
        let provider = candidate.provider.map(capture_provider);
        let kind = capture_resource_kind(&candidate.kind, candidate.provider);
        let scope = match candidate.kind {
            CaptureResourceKind::Conversation
            | CaptureResourceKind::Memory
            | CaptureResourceKind::StandaloneSkill => ResourceScope::ProviderState,
            CaptureResourceKind::Plugin => ResourceScope::Dependency,
            _ => ResourceScope::Project,
        };
        let provenance = match candidate.kind {
            CaptureResourceKind::Plugin => Provenance::Plugin {
                provider: provider
                    .ok_or_else(|| "plugin candidate lacks a provider".to_string())?,
                plugin_id: candidate.display_name.clone(),
            },
            _ => candidate
                .logical_paths
                .first()
                .and_then(|path| path.strip_prefix("project/"))
                .map(|relative_path| Provenance::ProjectLocal {
                    relative_path: relative_path.to_string(),
                })
                .unwrap_or(Provenance::Unknown),
        };
        let description = match candidate.kind {
            CaptureResourceKind::StandaloneSkill => Some(
                "Global custom skill from the mapped provider home. Restoring installs it \
                 for every project sharing that provider profile."
                    .to_string(),
            ),
            CaptureResourceKind::Plugin if candidate.metadata.contains_key("plugin_origin") => {
                Some(
                    "Installed global plugin. Only portable install intent syncs; pull \
                     reinstalls through the provider's own CLI."
                        .to_string(),
                )
            }
            _ => None,
        };
        let apply_policy = recipe
            .entries
            .get(&resource_id)
            .map(|entry| entry.apply_policy)
            .unwrap_or_else(|| capture_apply_policy(&candidate.apply_policy));
        let descriptor = ResourceDescriptor {
            resource_id,
            kind,
            provider,
            scope,
            display_name: candidate.display_name.clone(),
            provenance,
            apply_policy,
            relative_cwd: candidate.relative_cwd.clone(),
            codec_version: 1,
            metadata: candidate.metadata.clone(),
        };
        descriptor.validate()?;
        resources.push(InventoryResource {
            category: resource_category(kind).to_string(),
            description,
            logical_paths: candidate.logical_paths.clone(),
            default_selected: candidate.selected_by_default,
            blocked_reason: candidate.blocked_reason.clone(),
            provided_by: provider.map(|provider| format!("{:?}", provider)),
            install_behavior: candidate
                .dependency
                .as_ref()
                .map(|_| "Requires explicit approval during restore".to_string()),
            descriptor,
        });
    }
    resources.sort_by(|left, right| {
        left.descriptor
            .resource_id
            .cmp(&right.descriptor.resource_id)
    });
    Ok(ResourceInventory {
        project,
        bundle_id,
        resources,
        recipe,
        generated_at: now_secs(),
        warnings,
    })
}

fn default_recipe(candidates: &[ResourceCandidate]) -> Result<BundleRecipe, String> {
    let mut recipe = BundleRecipe::default();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.selected_by_default && candidate.blocked_reason.is_none())
    {
        let resource_id = ResourceId::parse(candidate.resource_id.clone())?;
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id,
                apply_policy: capture_apply_policy(&candidate.apply_policy),
                required: false,
            },
        );
    }
    recipe.validate()?;
    Ok(recipe)
}

fn persist_auto_selected_conversations(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    candidates: &[ResourceCandidate],
) -> Result<LocalProjectRegistration, String> {
    let additions = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind == CaptureResourceKind::Conversation
                && candidate.selected_by_default
                && candidate.blocked_reason.is_none()
        })
        .map(|candidate| {
            let resource_id = ResourceId::parse(candidate.resource_id.clone())?;
            Ok((
                resource_id.clone(),
                RecipeEntry {
                    resource_id,
                    apply_policy: capture_apply_policy(&candidate.apply_policy),
                    required: false,
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let current = repository
        .load_config()?
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    if additions
        .iter()
        .all(|(resource_id, _)| current.recipe.entries.contains_key(resource_id))
    {
        return Ok(current);
    }
    repository.mutate_config(|config| {
        let project = config
            .projects
            .iter_mut()
            .find(|project| &project.local_project_id == local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        let mut changed = false;
        for (resource_id, entry) in &additions {
            if !project.recipe.entries.contains_key(resource_id) {
                project
                    .recipe
                    .entries
                    .insert(resource_id.clone(), entry.clone());
                changed = true;
            }
        }
        if changed {
            project.recipe.revision = project.recipe.revision.saturating_add(1);
            project.revision = project.revision.saturating_add(1);
            project.updated_at = now_secs();
        }
        Ok(project.clone())
    })
}

fn capture_provider(provider: CaptureProvider) -> Provider {
    match provider {
        CaptureProvider::Codex => Provider::Codex,
        CaptureProvider::Claude => Provider::Claude,
    }
}

fn capture_resource_kind(
    kind: &CaptureResourceKind,
    provider: Option<CaptureProvider>,
) -> ResourceKind {
    match kind {
        CaptureResourceKind::ProjectFile => ResourceKind::ProjectFile,
        CaptureResourceKind::ProjectContentFile => ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => ResourceKind::ProjectContentDirectory,
        CaptureResourceKind::ProjectSettings => ResourceKind::Setting,
        CaptureResourceKind::Conversation => match provider {
            Some(CaptureProvider::Claude) => ResourceKind::ClaudeConversation,
            _ => ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => ResourceKind::ProjectMemory,
        CaptureResourceKind::Agent => ResourceKind::Agent,
        CaptureResourceKind::Command => ResourceKind::Command,
        CaptureResourceKind::Rule => ResourceKind::Rule,
        CaptureResourceKind::Skill => ResourceKind::ProjectSkill,
        CaptureResourceKind::StandaloneSkill => ResourceKind::StandaloneSkill,
        CaptureResourceKind::Plugin => ResourceKind::Plugin,
        CaptureResourceKind::Hook => ResourceKind::Hook,
        CaptureResourceKind::McpServer => ResourceKind::McpServer,
    }
}

fn capture_apply_policy(policy: &CaptureApplyPolicy) -> ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => ApplyPolicy::ExplicitReview,
        CaptureApplyPolicy::Dependency => ApplyPolicy::ExplicitInstall,
    }
}

fn resource_category(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::CodexConversation
        | ResourceKind::ClaudeConversation
        | ResourceKind::ProjectMemory => "conversations",
        ResourceKind::ProjectSkill | ResourceKind::StandaloneSkill => "skills",
        ResourceKind::Plugin => "plugins",
        ResourceKind::ProjectContentFile | ResourceKind::ProjectContentDirectory => "project_files",
        ResourceKind::McpServer | ResourceKind::Hook | ResourceKind::Requirement => "tools",
        _ => "project_setup",
    }
}

fn inventory_providers(resources: &[InventoryResource]) -> Vec<Provider> {
    resources
        .iter()
        .filter_map(|resource| resource.descriptor.provider)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

enum ConfiguredStore {
    Local(LocalBundleObjectStore),
    S3(S3BundleObjectStore),
}

impl BundleObjectStore for ConfiguredStore {
    fn get(&self, key: &ObjectKey) -> Result<Option<StoredObject>, String> {
        match self {
            Self::Local(store) => store.get(key),
            Self::S3(store) => store.get(key),
        }
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        match self {
            Self::Local(store) => store.put_immutable(key, bytes),
            Self::S3(store) => store.put_immutable(key, bytes),
        }
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        match self {
            Self::Local(store) => store.compare_and_swap(key, expectation, bytes),
            Self::S3(store) => store.compare_and_swap(key, expectation, bytes),
        }
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        match self {
            Self::Local(store) => store.list(prefix, cursor, limit),
            Self::S3(store) => store.list(prefix, cursor, limit),
        }
    }

    fn local_root(&self) -> Option<&Path> {
        match self {
            Self::Local(store) => store.local_root(),
            Self::S3(store) => store.local_root(),
        }
    }
}

type StorageEngine = BundleEngine<ConfiguredStore>;

fn list_remote_bundles_with_repository(
    repository: &V3Repository,
    storage_id: &StorageId,
    cursor: Option<&str>,
) -> Result<BundlePage, String> {
    let (_, engine) = storage_engine(repository, storage_id)?;
    let RemoteBundlePage {
        bundles,
        next_cursor,
    } = engine.list_remote_bundles(cursor, 100)?;
    Ok(BundlePage {
        bundles: bundles
            .into_iter()
            .map(|bundle| RemoteBundleSummaryDto {
                bundle_id: bundle.bundle_id,
                display_name: bundle.display_name,
                kind: bundle.kind,
                generation: bundle.generation,
                updated_at: bundle.updated_at,
                resource_count: bundle.resources,
            })
            .collect(),
        next_cursor,
    })
}

fn find_remote_bundle_matches_with_repository(
    repository: &V3Repository,
    storage_id: &StorageId,
    repository_fingerprint: &str,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    if repository_fingerprint.len() != 64
        || !repository_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("repository fingerprint must be a SHA-256 hex digest".to_string());
    }
    Ok(
        list_remote_bundle_snapshots_with_repository(repository, storage_id)?
            .into_iter()
            .filter(|snapshot| {
                snapshot.repository_fingerprint.as_deref() == Some(repository_fingerprint)
            })
            .collect(),
    )
}

fn list_remote_bundle_snapshots_with_repository(
    repository: &V3Repository,
    storage_id: &StorageId,
) -> Result<Vec<BundleSnapshotSummary>, String> {
    let (_, engine) = storage_engine(repository, storage_id)?;
    let mut cursor = None;
    let mut inspected = 0_usize;
    let mut snapshots = Vec::new();
    loop {
        let page = engine.list_remote_bundles(cursor.as_deref(), 100)?;
        for bundle in page.bundles {
            inspected = inspected.saturating_add(1);
            if inspected > 10_000 {
                return Err(
                    "remote bundle inspection exceeds the 10,000 bundle safety limit".to_string(),
                );
            }
            let snapshot = engine.inspect(&bundle.bundle_id)?;
            snapshots.push(bundle_snapshot_summary_from_snapshot(snapshot)?);
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if cursor.as_deref() == Some(next.as_str()) {
            return Err("remote bundle cursor did not advance".to_string());
        }
        cursor = Some(next);
    }
    snapshots.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.bundle_id.cmp(&right.bundle_id))
    });
    Ok(snapshots)
}

fn fetch_from_storage(
    repository: &V3Repository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
) -> Result<(StorageEngine, FetchedBundle), String> {
    let (_, engine) = storage_engine(repository, storage_id)?;
    let fetched = engine.fetch(bundle_id)?;
    Ok((engine, fetched))
}

fn bundle_snapshot_summary(fetched: FetchedBundle) -> Result<BundleSnapshotSummary, String> {
    bundle_snapshot_summary_from_snapshot(fetched.snapshot)
}

fn bundle_snapshot_summary_from_snapshot(
    snapshot: BundleSnapshot,
) -> Result<BundleSnapshotSummary, String> {
    snapshot.validate()?;
    let storage_id = snapshot.storage_id;
    let fetched_at = snapshot.fetched_at;
    let manifest = snapshot.manifest;
    let providers = manifest
        .resources
        .values()
        .filter_map(|resource| resource.provider)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(BundleSnapshotSummary {
        storage_id,
        bundle_id: manifest.bundle.bundle_id,
        display_name: manifest.bundle.display_name,
        kind: manifest.bundle.kind,
        generation: manifest.generation,
        updated_at: manifest.updated_at,
        resource_count: manifest.resources.len(),
        providers,
        repository_fingerprint: manifest.bundle.repository_fingerprint,
        resources: manifest.resources.into_values().collect(),
        recipe: manifest.recipe,
        fetched_at,
        warnings: Vec::new(),
    })
}

#[derive(Clone, Debug)]
struct ThreadVersion {
    thread_id: String,
    display_name: String,
    digest: Option<String>,
    updated_at: Option<u64>,
}

#[derive(Clone, Debug)]
struct LocalThreadDescriptor {
    thread_id: String,
    display_name: String,
    blocked_reason: Option<String>,
}

#[derive(Clone, Debug)]
struct MaterializedThreadBase {
    generation: u64,
    manifest_sha256: String,
    applied_at: u64,
    storage_digest: String,
    local_digest: String,
    kept_local: bool,
}

fn is_conversation_keep_local_action(action: &RestoreAction, manifest: &BundleManifest) -> bool {
    matches!(action.kind, RestoreActionKind::Manual { .. })
        && manifest
            .resources
            .get(&action.resource_id)
            .is_some_and(|resource| {
                matches!(
                    resource.kind,
                    ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
                )
            })
        && action.target_path.is_some()
        && action.source_sha256.is_some()
        && action.expected_target_sha256.is_some()
}

fn is_kept_local_conversation_receipt(receipt: &ApplyReceipt) -> bool {
    receipt.action_type == RestoreActionType::Manual
        && receipt.status == ActionStatus::Skipped
        && (receipt.resource_id.as_str().starts_with("codex:session:")
            || receipt.resource_id.as_str().starts_with("claude:session:"))
        && receipt.source_sha256.is_some()
        && receipt.target_sha256_after.is_some()
}

fn is_codex_thread_resource(resource_id: &ResourceId, kind: ResourceKind) -> bool {
    kind == ResourceKind::CodexConversation && resource_id.as_str().starts_with("codex:session:")
}

fn manifest_thread_versions(manifest: &BundleManifest) -> BTreeMap<ResourceId, ThreadVersion> {
    manifest
        .resources
        .iter()
        .filter(|(resource_id, descriptor)| is_codex_thread_resource(resource_id, descriptor.kind))
        .map(|(resource_id, descriptor)| {
            let updated_at = manifest
                .files
                .values()
                .filter(|file| &file.resource_id == resource_id)
                .map(|file| file.source_mtime)
                .max();
            (
                resource_id.clone(),
                ThreadVersion {
                    thread_id: descriptor.display_name.clone(),
                    display_name: descriptor.display_name.clone(),
                    digest: single_manifest_resource_file_digest(manifest, resource_id),
                    updated_at,
                },
            )
        })
        .collect()
}

fn single_manifest_resource_file_digest(
    manifest: &BundleManifest,
    resource_id: &ResourceId,
) -> Option<String> {
    let mut files = manifest
        .files
        .values()
        .filter(|file| &file.resource_id == resource_id);
    let digest = files.next()?.sha256.clone();
    files.next().is_none().then_some(digest)
}

fn single_captured_resource_file_digest(
    captured: &provider_capture::CapturedResources,
    resource_id: &str,
) -> Option<String> {
    let mut files = captured
        .files
        .values()
        .filter(|file| file.resource_id == resource_id);
    let digest = hex_digest(Sha256::digest(&files.next()?.bytes).as_slice());
    files.next().is_none().then_some(digest)
}

fn materialized_thread_bases(
    repository: &V3Repository,
    project: &LocalProjectRegistration,
    storage_id: &StorageId,
    binding: &ProjectBinding,
) -> Result<BTreeMap<ResourceId, MaterializedThreadBase>, String> {
    let materializations = repository.load_materializations()?;
    let mut bases = BTreeMap::<ResourceId, MaterializedThreadBase>::new();
    for record in materializations.records.iter().filter(|record| {
        record.host_id == binding.host_id
            && record.replica_id == binding.replica_id
            && record.local_project_id == project.local_project_id
            && record.storage_id == *storage_id
            && record.bundle_id == project.bundle_id
            && record.binding_revision == binding.revision
            && record.status != MaterializationStatus::Detached
    }) {
        for receipt in record
            .receipts
            .iter()
            .filter(|receipt| receipt.resource_id.as_str().starts_with("codex:session:"))
        {
            let kept_local = is_kept_local_conversation_receipt(receipt);
            if !kept_local
                && !(receipt.action_type == RestoreActionType::MaterializeConversation
                    && receipt.status == ActionStatus::Applied)
            {
                continue;
            }
            let (Some(storage_digest), Some(local_digest)) = (
                receipt.source_sha256.as_ref(),
                receipt.target_sha256_after.as_ref(),
            ) else {
                continue;
            };
            let candidate = MaterializedThreadBase {
                generation: record.generation,
                manifest_sha256: record.manifest_sha256.clone(),
                applied_at: record.applied_at,
                storage_digest: storage_digest.clone(),
                local_digest: local_digest.clone(),
                kept_local,
            };
            let replace = bases
                .get(&receipt.resource_id)
                .is_none_or(|current| candidate.applied_at >= current.applied_at);
            if replace {
                bases.insert(receipt.resource_id.clone(), candidate);
            }
        }
    }
    Ok(bases)
}

fn materialized_thread_base_supersedes_recipe_base(
    materialized: &MaterializedThreadBase,
    base: Option<&RecipeBase>,
) -> bool {
    let Some(base) = base else {
        return true;
    };
    if materialized.generation != base.generation {
        return materialized.generation > base.generation;
    }
    if materialized.manifest_sha256 != base.manifest_sha256 {
        return false;
    }
    let base_applied_at = base
        .last_pull_at
        .into_iter()
        .chain(base.last_push_at)
        .max()
        .unwrap_or(0);
    materialized.applied_at >= base_applied_at
}

fn classify_thread_sync_state(
    local_digest: Option<&str>,
    storage_digest: Option<&str>,
    base_known: bool,
    base_digest: Option<&str>,
) -> ThreadSyncState {
    classify_thread_sync_state_with_side_bases(
        local_digest,
        storage_digest,
        base_known,
        base_digest,
        base_digest,
    )
}

fn classify_thread_sync_state_with_side_bases(
    local_digest: Option<&str>,
    storage_digest: Option<&str>,
    base_known: bool,
    local_base_digest: Option<&str>,
    storage_base_digest: Option<&str>,
) -> ThreadSyncState {
    if local_digest == storage_digest {
        return ThreadSyncState::Synced;
    }
    if base_known {
        let local_changed = local_digest != local_base_digest;
        let storage_changed = storage_digest != storage_base_digest;
        return match (local_changed, storage_changed) {
            (true, false) => ThreadSyncState::LocalAhead,
            (false, true) => ThreadSyncState::StorageAhead,
            (true, true) => ThreadSyncState::Diverged,
            (false, false) => ThreadSyncState::Synced,
        };
    }
    match (local_digest, storage_digest) {
        (Some(_), None) => ThreadSyncState::LocalOnly,
        (None, Some(_)) => ThreadSyncState::StorageOnly,
        _ => ThreadSyncState::Unknown,
    }
}

fn thread_capture_unavailable_detail(blocked_reason: Option<&str>) -> String {
    let Some(reason) = blocked_reason else {
        return "Session could not be read safely during this scan.".to_string();
    };
    if reason.contains(&format!(
        "exceeds {} bytes",
        MAX_CONVERSATION_FILE_BYTES
    )) {
        return format!(
            "Session file exceeds the {} MiB per-file sync limit.",
            MAX_CONVERSATION_FILE_BYTES / (1024 * 1024)
        );
    }
    if reason.contains("hard-linked") {
        return "Session file is hard-linked and cannot be synced safely.".to_string();
    }
    if reason.contains("not a regular no-follow file") {
        return "Session path is not a regular file and cannot be synced safely.".to_string();
    }
    "Session is unavailable for safe capture.".to_string()
}

fn count_thread_sync_state(counts: &mut ThreadSyncCounts, state: ThreadSyncState) {
    match state {
        ThreadSyncState::Synced => counts.synced += 1,
        ThreadSyncState::LocalOnly | ThreadSyncState::LocalAhead => counts.local += 1,
        ThreadSyncState::StorageOnly | ThreadSyncState::StorageAhead => counts.storage += 1,
        ThreadSyncState::Diverged => counts.diverged += 1,
        ThreadSyncState::Unavailable => counts.unavailable += 1,
        ThreadSyncState::Unknown => counts.unknown += 1,
    }
}

fn relation_sync_state(relation: ConversationRelation) -> ThreadSyncState {
    match relation {
        ConversationRelation::Identical | ConversationRelation::BothMissing => {
            ThreadSyncState::Synced
        }
        ConversationRelation::CloudPrefixOfLocal => ThreadSyncState::LocalAhead,
        ConversationRelation::CloudMissing => ThreadSyncState::LocalOnly,
        ConversationRelation::LocalPrefixOfCloud => ThreadSyncState::StorageAhead,
        ConversationRelation::LocalMissing => ThreadSyncState::StorageOnly,
        ConversationRelation::Diverged => ThreadSyncState::Diverged,
        ConversationRelation::Unknown => ThreadSyncState::Unknown,
    }
}

pub(crate) fn schema6_thread_entries(
    repository: &V3Repository,
    fetched: &FetchedBundle,
    captured: &provider_capture::CapturedResources,
    binding: &ProjectBinding,
) -> Result<Vec<ThreadSyncEntry>, String> {
    if fetched.snapshot.manifest.schema_version != BUNDLE_SCHEMA_V6 {
        return Ok(Vec::new());
    }
    let document = repository.load_conversation_bindings()?;
    let profile_id = binding.profile_ids.get(&Provider::Codex);
    let mut entries = Vec::new();
    for (thread_id, thread) in &fetched.snapshot.manifest.conversations {
        if thread.provider != Provider::Codex {
            continue;
        }
        let local_binding = profile_id.and_then(|profile_id| {
            document.bindings.iter().find(|conversation| {
                conversation.host_id == binding.host_id
                    && conversation.replica_id == binding.replica_id
                    && conversation.local_project_id == binding.local_project_id
                    && conversation.provider_profile_id == *profile_id
                    && conversation.thread_id == *thread_id
            })
        });
        let local_conversation = local_binding
            .and_then(|conversation| conversation.provider_thread_id.as_deref())
            .and_then(|provider_thread_id| {
                captured.conversations.values().find(|conversation| {
                    conversation.provider == provider_capture::Provider::Codex
                        && conversation.source_thread_id.as_deref() == Some(provider_thread_id)
                })
            });
        let transcript = thread
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .and_then(|file| fetched.files.get(&file.logical_path));
        let cloud_analysis = transcript.map(|bytes| analyze_codex_transcript(bytes));
        let local_chain = local_conversation.and_then(|conversation| conversation.lineage.chain());
        let cloud_chain = cloud_analysis
            .as_ref()
            .and_then(|analysis| analysis.lineage.chain());
        let relation = match (local_conversation, cloud_analysis.as_ref()) {
            (Some(_), Some(_)) if local_chain.is_none() || cloud_chain.is_none() => {
                ConversationRelation::Unknown
            }
            (Some(_), Some(_)) => {
                super::domain::classify_conversation_relation(local_chain, cloud_chain)
            }
            (None, Some(_)) => ConversationRelation::LocalMissing,
            (Some(_), None) => ConversationRelation::CloudMissing,
            (None, None) => ConversationRelation::BothMissing,
        };
        let cloud_readable = cloud_analysis
            .as_ref()
            .is_some_and(|analysis| analysis.lineage.is_complete());
        let local_capturable =
            local_conversation.is_some_and(|conversation| conversation.lineage.is_complete());
        let capabilities = ConversationCapabilities {
            binding: if local_binding.is_some() {
                BindingResolution::Bound
            } else {
                BindingResolution::Missing
            },
            local_safely_capturable: local_capturable,
            cloud_safely_readable: cloud_readable,
            destination_codec_resumable: cloud_readable,
            provider_quiescent: true,
            structural_rewrite_only: false,
        };
        let push =
            default_conversation_decision(ConversationPlanDirection::Push, relation, capabilities);
        let pull =
            default_conversation_decision(ConversationPlanDirection::Pull, relation, capabilities);
        let state = relation_sync_state(relation);
        let proposed = match state {
            ThreadSyncState::LocalOnly | ThreadSyncState::LocalAhead => push,
            _ => pull,
        };
        let mut available_actions = vec![push.action, pull.action];
        available_actions.sort_by_key(|action| format!("{action:?}"));
        available_actions.dedup();
        let descriptor = fetched
            .snapshot
            .manifest
            .resources
            .get(&thread.resource_id)
            .ok_or_else(|| format!("conversation thread '{}' has no descriptor", thread_id))?;
        let origin_label = thread
            .last_updated_by_origin
            .as_ref()
            .or(thread.created_by_origin.as_ref())
            .map(|origin| {
                repository
                    .resolve_origin_host_label(
                        &fetched.snapshot.manifest.bundle.bundle_id,
                        &origin.origin_host_token,
                    )
                    .map(|(label, _)| label)
            })
            .transpose()?;
        let provider_thread_id =
            local_binding.and_then(|conversation| conversation.provider_thread_id.clone());
        let local_updated_at = provider_thread_id.as_deref().and_then(|thread_id| {
            captured
                .files
                .values()
                .filter(|file| {
                    captured
                        .conversations
                        .get(&file.resource_id)
                        .and_then(|conversation| conversation.source_thread_id.as_deref())
                        == Some(thread_id)
                })
                .map(|file| file.source_mtime)
                .max()
        });
        let materialization_state = local_binding
            .map(|conversation| conversation.materialization_state.clone())
            .unwrap_or(ConversationMaterializationState::StoredOnly);
        let resumable = materialization_state == ConversationMaterializationState::Resumable
            && local_conversation.is_some_and(|conversation| conversation.lineage.is_complete());
        entries.push(ThreadSyncEntry {
            thread_id: provider_thread_id
                .clone()
                .or_else(|| thread.source_thread_id.clone())
                .unwrap_or_else(|| thread_id.to_string()),
            resource_id: thread.resource_id.clone(),
            display_name: thread
                .display_title
                .clone()
                .unwrap_or_else(|| descriptor.display_name.clone()),
            state,
            local_present: local_conversation.is_some(),
            storage_present: true,
            status_detail: (relation == ConversationRelation::Unknown)
                .then(|| "Conversation ancestry could not be proven safely.".to_string()),
            local_updated_at,
            storage_updated_at: Some(thread.updated_at),
            cloud_thread_id: Some(thread_id.clone()),
            forked_from: thread.forked_from.clone(),
            provider_thread_id,
            relation: Some(relation),
            proposed_action: Some(proposed.action),
            reason_code: Some(proposed.reason_code),
            materialization_state: Some(materialization_state),
            resumable,
            origin_label,
            available_actions,
            requires_explicit_approval: matches!(
                proposed.action,
                ConversationPlanActionType::ReplaceLocal
                    | ConversationPlanActionType::ReplaceCloud
                    | ConversationPlanActionType::TombstoneThread
            ),
        });
    }
    Ok(entries)
}

fn get_project_thread_sync_comparison_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ThreadSyncComparison, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    require_project_link(&config, &project, storage_id)?;
    let storage_name = config
        .storages
        .iter()
        .find(|storage| storage.id == *storage_id)
        .map(|storage| storage.name.clone())
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;
    let binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;

    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovered = provider_capture::discover_project(&capture_request)?;
    let mut warnings = discovered.warnings.clone();
    let local_descriptors = discovered
        .resources
        .iter()
        .filter(|candidate| {
            candidate.kind == CaptureResourceKind::Conversation
                && candidate.provider == Some(CaptureProvider::Codex)
                && candidate.resource_id.starts_with("codex:session:")
        })
        .map(|candidate| {
            Ok((
                ResourceId::parse(candidate.resource_id.clone())?,
                LocalThreadDescriptor {
                    thread_id: candidate.display_name.clone(),
                    display_name: candidate.display_name.clone(),
                    blocked_reason: candidate.blocked_reason.clone(),
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let selected_resource_ids = local_descriptors
        .keys()
        .map(|resource_id| resource_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let captured = provider_capture::capture_selected(&capture_request, &selected_resource_ids)?;
    warnings.extend(captured.warnings.clone());
    let unavailable = captured
        .unavailable_resource_ids
        .iter()
        .map(|resource_id| ResourceId::parse(resource_id.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let local_versions = captured
        .resources
        .iter()
        .filter(|(resource_id, resource)| {
            resource_id.starts_with("codex:session:")
                && resource.descriptor.kind == CaptureResourceKind::Conversation
                && resource.descriptor.provider == Some(CaptureProvider::Codex)
        })
        .map(|(resource_id, resource)| {
            let parsed_id = ResourceId::parse(resource_id.clone())?;
            let updated_at = captured
                .files
                .values()
                .filter(|file| file.resource_id == *resource_id)
                .map(|file| file.source_mtime)
                .max();
            Ok((
                parsed_id,
                ThreadVersion {
                    thread_id: resource.descriptor.display_name.clone(),
                    display_name: resource.descriptor.display_name.clone(),
                    digest: single_captured_resource_file_digest(&captured, resource_id),
                    updated_at,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;

    let (_, engine) = storage_engine(repository, storage_id)?;
    let current = engine.inspect_optional(&project.bundle_id)?;
    let schema6_fetched = current
        .as_ref()
        .filter(|snapshot| snapshot.manifest.schema_version == BUNDLE_SCHEMA_V6)
        .map(|_| engine.fetch(&project.bundle_id))
        .transpose()?;
    let generation = current.as_ref().map(|snapshot| snapshot.head.generation);
    let remote_versions = current
        .as_ref()
        .map(|snapshot| manifest_thread_versions(&snapshot.manifest))
        .unwrap_or_default();

    let base = project.recipe_bases.get(storage_id);
    let base_manifest = if let Some(base) = base {
        if base.binding_revision != Some(binding.revision) {
            warnings.push(
                "The project folder or agent profile changed after the last reviewed sync; thread direction is unavailable until the next Pull or Push."
                    .to_string(),
            );
            None
        } else if current.as_ref().is_some_and(|snapshot| {
            snapshot.head.generation == base.generation
                && snapshot.head.manifest_sha256 == base.manifest_sha256
        }) {
            current.as_ref().map(|snapshot| snapshot.manifest.clone())
        } else if let Some(commit_id) = base.commit_id.as_deref() {
            match engine.inspect_manifest_version(
                &project.bundle_id,
                base.generation,
                commit_id,
                &base.manifest_sha256,
            ) {
                Ok(manifest) => Some(manifest),
                Err(error) => {
                    warnings.push(format!(
                        "The reviewed thread base could not be loaded: {error}"
                    ));
                    None
                }
            }
        } else {
            warnings.push(
                "The reviewed thread base predates directional comparison; Pull or Push once to establish it."
                    .to_string(),
            );
            None
        }
    } else {
        None
    };
    let base_versions = base_manifest
        .as_ref()
        .map(manifest_thread_versions)
        .unwrap_or_default();
    // A Pull may intentionally leave an optional global tool unapplied. That
    // keeps the whole-bundle materialization partial (and must not authorize a
    // later Push), but each applied or reviewed keep-local conversation
    // receipt is still a precise, two-sided comparison base. Use those
    // per-thread receipts independently from the project recipe base so
    // History reflects the successful part of the Pull without weakening
    // Push safety.
    let materialized_bases = materialized_thread_bases(repository, &project, storage_id, &binding)?;
    let comparison_base_generation = base
        .map(|base| base.generation)
        .into_iter()
        .chain(
            materialized_bases
                .values()
                .filter(|materialized| {
                    materialized_thread_base_supersedes_recipe_base(materialized, base)
                })
                .map(|materialized| materialized.generation),
        )
        .max();

    let resource_ids = local_descriptors
        .keys()
        .chain(remote_versions.keys())
        .chain(base_versions.keys())
        .chain(materialized_bases.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    let mut counts = ThreadSyncCounts::default();
    for resource_id in resource_ids {
        let local = local_versions.get(&resource_id);
        let remote = remote_versions.get(&resource_id);
        let base_version = base_versions.get(&resource_id);
        let local_present = local_descriptors.contains_key(&resource_id);
        let storage_present = remote.is_some();
        if !local_present && !storage_present {
            continue;
        }
        let local_digest = local.and_then(|version| version.digest.as_deref());
        let storage_digest = remote.and_then(|version| version.digest.as_deref());
        let materialized_base = materialized_bases.get(&resource_id).filter(|materialized| {
            materialized_thread_base_supersedes_recipe_base(materialized, base)
        });
        let base_resource_known = materialized_base.is_some()
            || (base_manifest.is_some()
                && (base_version.is_none()
                    || base_version
                        .and_then(|version| version.digest.as_deref())
                        .is_some()));
        let local_descriptor = local_descriptors.get(&resource_id);
        let local_unavailable =
            unavailable.contains(&resource_id) || (local_present && local_digest.is_none());
        let storage_unavailable = storage_present && storage_digest.is_none();
        let status_detail = if local_unavailable {
            Some(thread_capture_unavailable_detail(
                local_descriptor.and_then(|descriptor| descriptor.blocked_reason.as_deref()),
            ))
        } else if storage_unavailable {
            Some("Stored session metadata is incomplete and cannot be compared safely.".to_string())
        } else {
            None
        };
        let state = if status_detail.is_some() {
            ThreadSyncState::Unavailable
        } else {
            let storage_base_digest = materialized_base
                .map(|materialized| materialized.storage_digest.as_str())
                .or_else(|| base_version.and_then(|version| version.digest.as_deref()));
            let local_base_digest = materialized_base
                .map(|materialized| materialized.local_digest.as_str())
                .or(storage_base_digest);
            if materialized_base.is_some_and(|materialized| materialized.kept_local)
                && local_digest != storage_digest
            {
                if storage_digest == storage_base_digest {
                    ThreadSyncState::LocalAhead
                } else {
                    ThreadSyncState::Diverged
                }
            } else {
                classify_thread_sync_state_with_side_bases(
                    local_digest,
                    storage_digest,
                    base_resource_known,
                    local_base_digest,
                    storage_base_digest,
                )
            }
        };
        let descriptor = local.or(remote).or(base_version);
        let thread_id = descriptor
            .map(|version| version.thread_id.clone())
            .or_else(|| local_descriptor.map(|descriptor| descriptor.thread_id.clone()))
            .unwrap_or_else(|| resource_id.as_str().to_string());
        let display_name = descriptor
            .map(|version| version.display_name.clone())
            .or_else(|| local_descriptor.map(|descriptor| descriptor.display_name.clone()))
            .unwrap_or_else(|| thread_id.clone());
        count_thread_sync_state(&mut counts, state);
        entries.push(ThreadSyncEntry {
            provider_thread_id: Some(thread_id.clone()),
            thread_id,
            resource_id,
            display_name,
            state,
            local_present,
            storage_present,
            status_detail,
            local_updated_at: local.and_then(|version| version.updated_at),
            storage_updated_at: remote.and_then(|version| version.updated_at),
            cloud_thread_id: None,
            forked_from: None,
            relation: None,
            proposed_action: None,
            reason_code: None,
            materialization_state: None,
            resumable: local_present && !local_unavailable,
            origin_label: None,
            available_actions: Vec::new(),
            requires_explicit_approval: false,
        });
    }
    entries.sort_by(|left, right| {
        right
            .local_updated_at
            .into_iter()
            .chain(right.storage_updated_at)
            .max()
            .cmp(
                &left
                    .local_updated_at
                    .into_iter()
                    .chain(left.storage_updated_at)
                    .max(),
            )
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    if let Some(fetched) = &schema6_fetched {
        let thread_entries = schema6_thread_entries(repository, fetched, &captured, &binding)?;
        let bound_provider_ids = thread_entries
            .iter()
            .filter_map(|entry| entry.provider_thread_id.clone())
            .collect::<BTreeSet<_>>();
        entries.retain(|entry| {
            entry.cloud_thread_id.is_some() || !bound_provider_ids.contains(&entry.thread_id)
        });
        entries.extend(thread_entries);
        entries.sort_by(|left, right| {
            left.cloud_thread_id
                .cmp(&right.cloud_thread_id)
                .then_with(|| left.thread_id.cmp(&right.thread_id))
        });
        counts = ThreadSyncCounts::default();
        for entry in &entries {
            count_thread_sync_state(&mut counts, entry.state);
        }
    }
    warnings.sort();
    warnings.dedup();
    Ok(ThreadSyncComparison {
        project_id: local_project_id.clone(),
        storage_id: storage_id.clone(),
        storage_name,
        generation,
        base_generation: comparison_base_generation,
        compared_at: now_secs(),
        entries,
        counts,
        warnings,
    })
}

fn get_bundle_status_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ResourceStatusReport, String> {
    let config = repository.load_config()?;
    let mut project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    require_project_link(&config, &project, storage_id)?;
    let destination_recipe = config
        .links
        .iter()
        .find(|link| link.local_project_id == *local_project_id && link.storage_id == *storage_id)
        .and_then(|link| link.recipe.clone());
    let binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovered = provider_capture::discover_project(&capture_request)?;
    project = persist_auto_selected_conversations(
        repository,
        &project.local_project_id,
        &discovered.resources,
    )?;
    let effective_recipe = destination_recipe.unwrap_or_else(|| project.recipe.clone());
    let capture = provider_capture::capture_recipe(&capture_request, &effective_recipe)?;
    let local_resources = provider_capture::domain_resources(&capture)?;
    let unavailable: BTreeSet<_> = capture
        .unavailable_resource_ids
        .iter()
        .map(|id| ResourceId::parse(id.clone()))
        .collect::<Result<_, _>>()?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let remote = match engine.read_head(&project.bundle_id)? {
        Some(_) => Some(engine.fetch(&project.bundle_id)?),
        None => None,
    };
    let generation = remote
        .as_ref()
        .map(|fetched| fetched.snapshot.head.generation);
    let remote_resources = remote
        .as_ref()
        .map(|fetched| &fetched.snapshot.manifest.resources);
    let mut statuses = Vec::with_capacity(effective_recipe.entries.len());
    for resource_id in effective_recipe.entries.keys() {
        let local_digest = local_resources
            .get(resource_id)
            .and_then(|resource| resource.metadata.get("content_sha256"))
            .cloned();
        let remote_digest = remote_resources
            .and_then(|resources| resources.get(resource_id))
            .and_then(|resource| resource.metadata.get("content_sha256"))
            .cloned();
        let (state, message) = match (&local_digest, &remote_digest) {
            (Some(local), Some(remote)) if local == remote => ("synced", None),
            (Some(_), Some(_)) => (
                "conflict",
                Some("Local and remote resource versions differ".to_string()),
            ),
            (Some(_), None) => ("local_only", None),
            (None, Some(_)) if unavailable.contains(resource_id) => (
                "remote_only",
                Some("Selected resource is unavailable locally".to_string()),
            ),
            (None, Some(_)) => ("remote_only", None),
            (None, None) => (
                "missing",
                Some("Selected resource is unavailable locally and remotely".to_string()),
            ),
        };
        statuses.push(BundleResourceStatus {
            resource_id: resource_id.clone(),
            state: state.to_string(),
            message,
            local_digest,
            remote_digest,
        });
    }
    Ok(ResourceStatusReport {
        project: local_project_id.to_string(),
        storage: storage_id.to_string(),
        bundle_id: project.bundle_id,
        generation,
        statuses,
        warnings: capture.warnings,
    })
}

fn is_capability_kind(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::ProjectSkill | ResourceKind::StandaloneSkill | ResourceKind::Plugin
    )
}

fn capability_state_name(state: ThreadSyncState) -> &'static str {
    match state {
        ThreadSyncState::Synced => "synced",
        ThreadSyncState::LocalOnly => "local_only",
        ThreadSyncState::LocalAhead => "local_ahead",
        ThreadSyncState::StorageOnly => "storage_only",
        ThreadSyncState::StorageAhead => "storage_ahead",
        ThreadSyncState::Diverged => "diverged",
        ThreadSyncState::Unavailable => "unavailable",
        ThreadSyncState::Unknown => "unknown",
    }
}

fn descriptor_capability_digest(descriptor: Option<&ResourceDescriptor>) -> Option<String> {
    let descriptor = descriptor?;
    if descriptor.kind != ResourceKind::Plugin {
        return descriptor.metadata.get("content_sha256").cloned();
    }

    // Plugin payloads remain installer-owned. Compare only normalized,
    // portable installation intent; observed versions are diagnostic and may
    // legitimately resolve differently on two machines.
    let mut hasher = Sha256::new();
    hasher.update(descriptor.resource_id.as_str().as_bytes());
    if let Some(provider) = descriptor.provider {
        hasher.update(provider_name(provider).as_bytes());
    }
    for key in [
        "plugin_marketplace",
        "plugin_source_type",
        "plugin_source",
        "dependency_program",
        "dependency_argv_json",
    ] {
        if let Some(value) = descriptor.metadata.get(key) {
            hasher.update((key.len() as u64).to_be_bytes());
            hasher.update(key.as_bytes());
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
    Some(hex_digest(hasher.finalize().as_slice()))
}

fn classify_capability_sync_state(
    local_digest: Option<&str>,
    storage_digest: Option<&str>,
    base_known: bool,
    base_digest: Option<&str>,
) -> ThreadSyncState {
    let state = classify_thread_sync_state(local_digest, storage_digest, base_known, base_digest);
    if state == ThreadSyncState::Unknown && local_digest.is_some() && storage_digest.is_some() {
        ThreadSyncState::Diverged
    } else {
        state
    }
}

fn descriptor_version(descriptor: Option<&ResourceDescriptor>) -> Option<String> {
    descriptor.and_then(|descriptor| descriptor.metadata.get("plugin_observed_version").cloned())
}

fn descriptor_provided_skills(descriptor: Option<&ResourceDescriptor>) -> Vec<String> {
    descriptor
        .and_then(|descriptor| descriptor.metadata.get("plugin_provided_skills_json"))
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn get_project_capability_status_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: Option<&StorageId>,
) -> Result<CapabilityStatusReport, String> {
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    let machine = repository.load_bindings()?;
    let binding = machine
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;

    let mut warnings = Vec::new();
    let mut profiles = Vec::new();
    for (provider, profile_id) in &binding.profile_ids {
        let Some(profile) = machine
            .profiles
            .iter()
            .find(|candidate| &candidate.profile_id == profile_id)
        else {
            warnings.push(format!("provider profile '{}' is unavailable", profile_id));
            continue;
        };
        let shared_project_count = machine
            .bindings
            .iter()
            .filter(|candidate| {
                candidate.state == BindingState::Active
                    && candidate.profile_ids.values().any(|id| id == profile_id)
            })
            .map(|candidate| candidate.local_project_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        profiles.push(CapabilityProfileContext {
            provider: *provider,
            profile_id: profile_id.clone(),
            display_name: profile.display_name.clone(),
            path: profile.path.clone(),
            shared_project_count,
        });
    }
    profiles.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovered = provider_capture::discover_project(&capture_request)?;
    let inventory = inventory_from_candidates(
        binding.project_root.clone(),
        Some(project.bundle_id.clone()),
        &discovered.resources,
        project.recipe.clone(),
        discovered.warnings.clone(),
    )?;
    warnings.extend(inventory.warnings.clone());

    let selected_capabilities = discovered
        .resources
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.kind,
                CaptureResourceKind::Skill
                    | CaptureResourceKind::StandaloneSkill
                    | CaptureResourceKind::Plugin
            )
        })
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let captured =
        match provider_capture::capture_selected(&capture_request, &selected_capabilities) {
            Ok(captured) => captured,
            Err(error) => {
                warnings.push(format!(
                    "Skill and plugin content could not be fully compared: {error}"
                ));
                provider_capture::CapturedResources::default()
            }
        };
    warnings.extend(captured.warnings.clone());
    let local_captured = provider_capture::domain_resources(&captured)?;
    let unavailable = captured
        .unavailable_resource_ids
        .iter()
        .map(|resource_id| ResourceId::parse(resource_id.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;

    let mut local_inventory = inventory
        .resources
        .into_iter()
        .filter(|resource| is_capability_kind(resource.descriptor.kind))
        .map(|mut resource| {
            if let Some(captured) = local_captured.get(&resource.descriptor.resource_id) {
                resource.descriptor = captured.clone();
            }
            (resource.descriptor.resource_id.clone(), resource)
        })
        .collect::<BTreeMap<_, _>>();

    let mut storage_name = None;
    let mut current = None;
    let mut base_manifest = None;
    let mut effective_recipe = project.recipe.clone();
    let mut storage_comparison_available = false;
    let base_generation = storage_id
        .and_then(|storage_id| project.recipe_bases.get(storage_id))
        .map(|base| base.generation);

    if let Some(storage_id) = storage_id {
        require_project_link(&config, &project, storage_id)?;
        storage_name = config
            .storages
            .iter()
            .find(|storage| storage.id == *storage_id)
            .map(|storage| storage.name.clone());
        effective_recipe = config
            .links
            .iter()
            .find(|link| {
                link.local_project_id == *local_project_id && link.storage_id == *storage_id
            })
            .and_then(|link| link.recipe.clone())
            .unwrap_or_else(|| project.recipe.clone());

        match storage_engine(repository, storage_id) {
            Ok((_, engine)) => match engine.inspect_optional(&project.bundle_id) {
                Ok(snapshot) => {
                    current = snapshot;
                    storage_comparison_available = true;
                    if let Some(base) = project.recipe_bases.get(storage_id) {
                        if base.binding_revision != Some(binding.revision) {
                            warnings.push(
                                "The project folder or agent profile changed after the last reviewed sync; skill and plugin direction is unavailable until the next Pull or Push."
                                    .to_string(),
                            );
                        } else if current.as_ref().is_some_and(|snapshot| {
                            snapshot.head.generation == base.generation
                                && snapshot.head.manifest_sha256 == base.manifest_sha256
                        }) {
                            base_manifest =
                                current.as_ref().map(|snapshot| snapshot.manifest.clone());
                        } else if let Some(commit_id) = base.commit_id.as_deref() {
                            match engine.inspect_manifest_version(
                                &project.bundle_id,
                                base.generation,
                                commit_id,
                                &base.manifest_sha256,
                            ) {
                                Ok(manifest) => base_manifest = Some(manifest),
                                Err(error) => warnings.push(format!(
                                    "The reviewed skill and plugin base could not be loaded: {error}"
                                )),
                            }
                        } else {
                            warnings.push(
                                "The reviewed base predates directional comparison; Pull or Push once to establish it."
                                    .to_string(),
                            );
                        }
                    }
                }
                Err(error) => warnings.push(format!(
                    "Skills and plugins could not be compared with storage: {error}"
                )),
            },
            Err(error) => warnings.push(format!(
                "Skills and plugins could not be compared with storage: {error}"
            )),
        }
    }

    let remote_resources = current
        .as_ref()
        .map(|snapshot| {
            snapshot
                .manifest
                .resources
                .iter()
                .filter(|(_, descriptor)| is_capability_kind(descriptor.kind))
                .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let base_resources = base_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .resources
                .iter()
                .filter(|(_, descriptor)| is_capability_kind(descriptor.kind))
                .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut remote_paths = BTreeMap::<ResourceId, Vec<String>>::new();
    if let Some(snapshot) = &current {
        for (path, file) in &snapshot.manifest.files {
            if remote_resources.contains_key(&file.resource_id) {
                remote_paths
                    .entry(file.resource_id.clone())
                    .or_default()
                    .push(path.as_str().to_string());
            }
        }
    }

    let resource_ids = local_inventory
        .keys()
        .chain(remote_resources.keys())
        .chain(base_resources.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut items = Vec::new();
    for resource_id in resource_ids {
        let local_inventory_resource = local_inventory.remove(&resource_id);
        let local_descriptor = local_captured.get(&resource_id).or_else(|| {
            local_inventory_resource
                .as_ref()
                .map(|resource| &resource.descriptor)
        });
        let remote_descriptor = remote_resources.get(&resource_id);
        let base_descriptor = base_resources.get(&resource_id);
        let local_present = local_inventory_resource.is_some();
        let storage_present = remote_descriptor.is_some();
        if !local_present && !storage_present {
            continue;
        }
        let descriptor = local_descriptor
            .or(remote_descriptor)
            .or(base_descriptor)
            .cloned()
            .ok_or_else(|| format!("capability '{}' has no descriptor", resource_id))?;
        let blocked_reason = local_inventory_resource
            .as_ref()
            .and_then(|resource| resource.blocked_reason.clone());
        let local_digest = descriptor_capability_digest(local_descriptor);
        let storage_digest = descriptor_capability_digest(remote_descriptor);
        let base_digest = descriptor_capability_digest(base_descriptor);
        let incomplete = unavailable.contains(&resource_id)
            || (local_present && local_digest.is_none())
            || (storage_present && storage_digest.is_none());
        let state = if blocked_reason.is_some() {
            "blocked"
        } else if !storage_comparison_available {
            "not_compared"
        } else if incomplete {
            "unknown"
        } else {
            let base_resource_known =
                base_manifest.is_some() && (base_descriptor.is_none() || base_digest.is_some());
            capability_state_name(classify_capability_sync_state(
                local_digest.as_deref(),
                storage_digest.as_deref(),
                base_resource_known,
                base_digest.as_deref(),
            ))
        };
        let local_version = descriptor_version(local_descriptor);
        let storage_version = descriptor_version(remote_descriptor);
        let enabled = local_descriptor
            .and_then(|descriptor| descriptor.metadata.get("plugin_enabled"))
            .and_then(|value| value.parse::<bool>().ok());
        let mut provided_skills = descriptor_provided_skills(local_descriptor);
        provided_skills.extend(descriptor_provided_skills(remote_descriptor));
        provided_skills.sort();
        provided_skills.dedup();
        let message = blocked_reason.clone().or_else(|| {
            if descriptor.kind == ResourceKind::Plugin && enabled == Some(false) {
                Some("Installed locally but disabled in the provider configuration".to_string())
            } else if descriptor.kind == ResourceKind::Plugin
                && local_version.is_some()
                && storage_version.is_some()
                && local_version != storage_version
            {
                Some(
                    "Observed plugin versions differ; native installation may resolve a newer version"
                        .to_string(),
                )
            } else {
                None
            }
        });
        let logical_paths = local_inventory_resource
            .as_ref()
            .map(|resource| resource.logical_paths.clone())
            .filter(|paths| !paths.is_empty())
            .or_else(|| remote_paths.get(&resource_id).cloned())
            .unwrap_or_default();

        items.push(CapabilityStatusItem {
            category: resource_category(descriptor.kind).to_string(),
            descriptor,
            state: state.to_string(),
            local_present,
            storage_present,
            selected_in_recipe: effective_recipe.entries.contains_key(&resource_id),
            blocked_reason,
            logical_paths,
            local_digest,
            storage_digest,
            local_version,
            storage_version,
            enabled,
            provided_skills,
            message,
        });
    }
    items.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.descriptor.provider.cmp(&right.descriptor.provider))
            .then_with(|| {
                left.descriptor
                    .display_name
                    .to_lowercase()
                    .cmp(&right.descriptor.display_name.to_lowercase())
            })
    });
    warnings.sort();
    warnings.dedup();

    Ok(CapabilityStatusReport {
        project_id: project.local_project_id,
        project_name: project.local_alias.unwrap_or(project.display_name),
        profiles,
        storage_id: storage_id.cloned(),
        storage_name,
        generation: current.as_ref().map(|snapshot| snapshot.head.generation),
        base_generation,
        compared_at: now_secs(),
        items,
        warnings,
    })
}

#[cfg(test)]
fn push_bundle_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectOperationResult, String> {
    push_bundle_with_recipe_with_repository(repository, local_project_id, storage_id, None)
}

#[cfg(test)]
fn push_bundle_with_recipe_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: Option<BundleRecipe>,
) -> Result<ProjectOperationResult, String> {
    push_bundle_reviewed_with_repository(
        repository,
        local_project_id,
        storage_id,
        requested_recipe,
        ProjectContentPushReview::default(),
    )
}

fn push_bundle_reviewed_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: Option<BundleRecipe>,
    project_content_review: ProjectContentPushReview,
) -> Result<ProjectOperationResult, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let config = repository.load_config()?;
    let mut project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    require_project_link(&config, &project, storage_id)?;
    let stored_link = config
        .links
        .iter()
        .find(|link| link.local_project_id == *local_project_id && link.storage_id == *storage_id)
        .cloned()
        .ok_or_else(|| "project storage link disappeared".to_string())?;
    let stored_destination_recipe = stored_link.recipe.clone();
    let expected_preference_revision = stored_link.project_content_preferences.revision;
    let binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    require_codex_conversation_paths_ready(repository, &binding.local_project_id)?;
    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovered = provider_capture::discover_project(&capture_request)?;
    project = persist_auto_selected_conversations(
        repository,
        &project.local_project_id,
        &discovered.resources,
    )?;
    let mut push_recipe = match requested_recipe {
        Some(requested) => {
            requested.validate()?;
            requested
        }
        None => project.recipe.clone(),
    };
    let stored_generic_entries = stored_destination_recipe
        .as_ref()
        .map(|recipe| {
            recipe
                .entries
                .iter()
                .filter(|(resource_id, _)| {
                    super::domain::is_project_content_resource_id(resource_id)
                })
                .map(|(resource_id, entry)| (resource_id.clone(), entry.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for resource_id in &project_content_review.removal_ids {
        if !stored_generic_entries.contains_key(resource_id) {
            return Err(format!(
                "project-content removal '{}' is not currently stored for this destination",
                resource_id
            ));
        }
        if push_recipe.entries.contains_key(resource_id) {
            return Err(format!(
                "project-content removal '{}' is also selected for publication",
                resource_id
            ));
        }
    }
    for (resource_id, entry) in &stored_generic_entries {
        if !push_recipe.entries.contains_key(resource_id)
            && !project_content_review.removal_ids.contains(resource_id)
        {
            push_recipe
                .entries
                .insert(resource_id.clone(), entry.clone());
        }
    }

    let requested_generic_ids = push_recipe
        .entries
        .keys()
        .filter(|resource_id| super::domain::is_project_content_resource_id(resource_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    let generic_selection_changed = requested_generic_ids
        != stored_generic_entries
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
        || !project_content_review.removal_ids.is_empty();
    let mut reviewed_inventory = None;
    if let Some(review_token) = project_content_review.review_token.as_deref() {
        let inventory = inspect_project_files_guarded(repository, local_project_id, storage_id)?;
        if inventory.eligibility.state != ProjectFileSyncEligibilityState::Eligible {
            return Err(inventory.eligibility.reason);
        }
        if inventory.review_token.as_deref() != Some(review_token) {
            return Err(
                "project files changed after Scan; rescan and review them before pushing"
                    .to_string(),
            );
        }
        let entries_by_id = inventory
            .entries
            .iter()
            .map(|entry| (entry.descriptor.resource_id.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        for resource_id in requested_generic_ids.clone() {
            let entry = entries_by_id.get(&resource_id).ok_or_else(|| {
                format!(
                    "selected project-content resource '{}' was not in the reviewed scan",
                    resource_id
                )
            })?;
            if entry.blocked_reason.is_some() {
                return Err(format!(
                    "selected project-content resource '{}' is blocked",
                    resource_id
                ));
            }
            if !entry.local_present && !stored_generic_entries.contains_key(&resource_id) {
                return Err(format!(
                    "new project-content resource '{}' disappeared after review",
                    resource_id
                ));
            }
            if let Some(warning_digest) = &entry.warning_digest {
                if entry.local_present
                    && !project_content_review
                        .acknowledged_warning_digests
                        .contains(warning_digest)
                {
                    return Err(format!(
                        "project-content warning for '{}' requires explicit acknowledgement",
                        entry.relative_path
                    ));
                }
            }
            if entry.entry_type == ProjectContentEntryType::File && entry.local_present {
                for ancestor in project_content_ancestor_paths(&entry.relative_path) {
                    let required = inventory.entries.iter().find(|candidate| {
                        candidate.entry_type == ProjectContentEntryType::Directory
                            && candidate.relative_path == ancestor
                    });
                    let required = required.ok_or_else(|| {
                        format!(
                            "selected file '{}' lacks reviewed directory '{}'",
                            entry.relative_path, ancestor
                        )
                    })?;
                    let required_id = required.descriptor.resource_id.clone();
                    push_recipe
                        .entries
                        .entry(required_id.clone())
                        .or_insert(RecipeEntry {
                            resource_id: required_id,
                            apply_policy: ApplyPolicy::ExplicitReview,
                            required: true,
                        });
                }
            }
        }
        for removed_directory in project_content_review.removal_ids.iter().filter_map(|id| {
            entries_by_id
                .get(id)
                .filter(|entry| entry.entry_type == ProjectContentEntryType::Directory)
        }) {
            if inventory.entries.iter().any(|candidate| {
                candidate.storage_present
                    && candidate
                        .relative_path
                        .starts_with(&format!("{}/", removed_directory.relative_path))
                    && push_recipe
                        .entries
                        .contains_key(&candidate.descriptor.resource_id)
            }) {
                return Err(format!(
                    "directory '{}' cannot be removed while stored descendants remain selected",
                    removed_directory.relative_path
                ));
            }
        }
        reviewed_inventory = Some(inventory);
    } else if generic_selection_changed {
        return Err(
            "Scan and review Project files before changing stored project content".to_string(),
        );
    }

    push_recipe.validate()?;
    push_recipe.revision = match &stored_destination_recipe {
        Some(current) if current.entries == push_recipe.entries => current.revision,
        Some(current) => current.revision.saturating_add(1),
        None => 1,
    };
    let mut capture_request = capture_request;
    capture_request.include_project_content = reviewed_inventory.is_some();
    if capture_request.include_project_content {
        let eligibility = project_file_sync_eligibility(&binding);
        if eligibility.state != ProjectFileSyncEligibilityState::Eligible {
            return Err(eligibility.reason);
        }
    }
    let capture = provider_capture::capture_recipe(&capture_request, &push_recipe)?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let expected_head = match (
        engine.read_head(&project.bundle_id)?,
        project.recipe_bases.get(storage_id),
    ) {
        (None, None) => PublishExpectation::Absent,
        (None, Some(_)) => {
            return Err(
                "remote bundle head is missing but this project has a prior base; review before republishing"
                    .to_string(),
            )
        }
        (Some((_, _)), None) => {
            return Err(
                "remote bundle already exists and this replica has no reviewed base; pull and review it before pushing"
                    .to_string(),
            )
        }
        (Some(_), Some(base)) if base.binding_revision != Some(binding.revision) => {
            return Err(
                "the project checkout or provider profile changed after the last reviewed base; pull and review before pushing"
                    .to_string(),
            )
        }
        (Some((head, token)), Some(base))
            if head.generation == base.generation
                && head.manifest_sha256 == base.manifest_sha256 =>
        {
            PublishExpectation::Match(token)
        }
        (Some(_), Some(_)) => {
            return Err(
                "remote bundle advanced since this replica's base; pull and review before pushing"
                    .to_string(),
            )
        }
    };
    let pushed_at = now_secs();
    let published = engine.publish(PublishBundleRequest {
        identity: BundleIdentity {
            bundle_id: project.bundle_id.clone(),
            display_name: project.display_name.clone(),
            kind: BundleKind::Project,
            repository_fingerprint: project.repository_fingerprint.clone(),
        },
        recipe: push_recipe.clone(),
        captured_with: CapturedWith {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            codex_version: None,
            claude_version: None,
            codec_versions: BTreeMap::new(),
        },
        captured: capture,
        expected_head,
        updated_at: pushed_at,
    })?;
    let generation = published.snapshot.head.generation;
    let commit_id = published.snapshot.head.commit_id.clone();
    let manifest_sha256 = published.snapshot.head.manifest_sha256.clone();
    let recipe_revision = push_recipe.revision;
    let expected_project_revision = project.revision;
    let expected_project_recipe_revision = project.recipe.revision;
    let bundle_id = project.bundle_id.clone();
    repository.mutate_config(|config| {
        let current = config
            .projects
            .iter_mut()
            .find(|current| current.local_project_id == *local_project_id)
            .ok_or_else(|| "project was removed while publishing".to_string())?;
        if current.revision != expected_project_revision
            || current.recipe.revision != expected_project_recipe_revision
            || current.bundle_id != bundle_id
        {
            return Err(
                "project recipe changed while publishing; remote head was written, refresh before pushing again"
                    .to_string(),
            );
        }
        let last_pull_at = current
            .recipe_bases
            .get(storage_id)
            .and_then(|base| base.last_pull_at);
        current.recipe_bases.insert(
            storage_id.clone(),
            RecipeBase {
                generation,
                manifest_sha256: manifest_sha256.clone(),
                commit_id: Some(commit_id.clone()),
                recipe_revision,
                binding_revision: Some(binding.revision),
                last_pull_at,
                last_push_at: Some(pushed_at),
            },
        );
        let current_link = config
            .links
            .iter_mut()
            .find(|link| {
                link.local_project_id == *local_project_id && link.storage_id == *storage_id
            })
            .ok_or_else(|| "project storage link was removed while publishing".to_string())?;
        if current_link.recipe != stored_destination_recipe
            || current_link.project_content_preferences.revision != expected_preference_revision
        {
            return Err(
                "project-file choices changed while publishing; remote head was written, refresh before pushing again"
                    .to_string(),
            );
        }
        current_link.recipe = Some(push_recipe.clone());
        if let Some(inventory) = &reviewed_inventory {
            let previous_exclusions = current_link
                .project_content_preferences
                .excluded_resource_ids
                .clone();
            let mut exclusions = previous_exclusions.clone();
            for entry in &inventory.entries {
                if !entry.local_present || entry.entry_type == ProjectContentEntryType::Blocked {
                    continue;
                }
                if push_recipe
                    .entries
                    .contains_key(&entry.descriptor.resource_id)
                {
                    exclusions.remove(&entry.descriptor.resource_id);
                } else {
                    exclusions.insert(entry.descriptor.resource_id.clone());
                }
            }
            exclusions.extend(project_content_review.removal_ids.iter().cloned());
            if exclusions != previous_exclusions {
                current_link.project_content_preferences.revision = current_link
                    .project_content_preferences
                    .revision
                    .saturating_add(1);
                current_link
                    .project_content_preferences
                    .excluded_resource_ids = exclusions;
            }
            current_link.project_content_preferences.validate()?;
        }
        current.revision = current.revision.saturating_add(1);
        current.updated_at = pushed_at;
        Ok(())
    })?;
    let results = published
        .snapshot
        .manifest
        .resources
        .keys()
        .cloned()
        .map(|resource_id| OperationResourceResult {
            resource_id,
            state: "synced".to_string(),
            message: None,
        })
        .collect::<Vec<_>>();
    Ok(ProjectOperationResult {
        success: true,
        message: format!("Published generation {}", generation),
        operation_id: Some(commit_id),
        resources_changed: Some(results.len()),
        generation: Some(generation),
        results,
    })
}

fn plan_bundle_restore_with_repository(
    repository: &V3Repository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) -> Result<RestorePlan, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let binding = require_current_binding(repository, binding)?;
    if &binding.bundle_id != bundle_id {
        return Err("binding and requested bundle IDs differ".to_string());
    }
    require_codex_conversation_paths_ready(repository, &binding.local_project_id)?;
    let (engine, fetched) = fetch_from_storage(repository, storage_id, bundle_id)?;
    let config = repository.load_config()?;
    let project = config
        .project(&binding.local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", binding.local_project_id))?;
    let recipe_base = project
        .recipe_bases
        .get(storage_id)
        .filter(|base| base.binding_revision == Some(binding.revision));
    let base_manifest = match recipe_base {
        Some(base)
            if fetched.snapshot.head.generation == base.generation
                && fetched.snapshot.head.manifest_sha256 == base.manifest_sha256 =>
        {
            Some(fetched.snapshot.manifest.clone())
        }
        Some(base) => base.commit_id.as_deref().and_then(|commit_id| {
            engine
                .inspect_manifest_version(
                    bundle_id,
                    base.generation,
                    commit_id,
                    &base.manifest_sha256,
                )
                .ok()
        }),
        None => None,
    };
    let materialized_bases = materialized_thread_bases(repository, &project, storage_id, &binding)?;
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != binding.host_id {
        return Err("host identity changed while preparing the Pull review".to_string());
    }
    let mut plan = engine.build_restore_plan(&fetched, &binding, now_secs(), identity.revision)?;
    plan.actions.retain(|action| {
        if !is_conversation_keep_local_action(action, &fetched.snapshot.manifest) {
            return true;
        }
        let current_storage_digest =
            single_manifest_resource_file_digest(&fetched.snapshot.manifest, &action.resource_id);
        let known_storage_digest = materialized_bases
            .get(&action.resource_id)
            .filter(|materialized| {
                materialized_thread_base_supersedes_recipe_base(materialized, recipe_base)
            })
            .map(|materialized| materialized.storage_digest.clone())
            .or_else(|| {
                base_manifest.as_ref().and_then(|manifest| {
                    single_manifest_resource_file_digest(manifest, &action.resource_id)
                })
            });
        current_storage_digest != known_storage_digest
    });
    plan.project_content_eligibility = Some(project_file_sync_eligibility(&binding));
    plan.validate()?;
    repository.save_restore_plan(&plan)?;
    Ok(plan)
}

fn apply_bundle_restore_with_repository(
    repository: &V3Repository,
    plan_id: &PlanId,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let plan = repository.load_restore_plan(plan_id)?;
    let binding = current_binding_for_restore_plan(repository, &plan)?;
    let approved = unique_approved_actions(&approved_action_ids, &plan.actions)?;
    let approves_project_content = plan.actions.iter().any(|action| {
        approved.contains(&action.action_id)
            && matches!(
                &action.kind,
                RestoreActionKind::WriteProjectFile { .. }
                    | RestoreActionKind::EnsureProjectDirectory { .. }
                    | RestoreActionKind::DeleteProjectFile { .. }
                    | RestoreActionKind::DeleteProjectDirectory { .. }
            )
    });
    if approves_project_content {
        let eligibility = project_file_sync_eligibility(&binding);
        if eligibility.state != ProjectFileSyncEligibilityState::Eligible {
            return Err(eligibility.reason);
        }
    }
    if repository
        .load_materializations()?
        .records
        .iter()
        .any(|record| record.plan_id == plan.plan_id)
    {
        return Err(format!(
            "restore plan '{}' was already applied",
            plan.plan_id
        ));
    }
    let (engine, fetched) = fetch_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    let restored_recipe = fetched.snapshot.manifest.recipe.clone();
    let applied_at = now_secs();
    let applied = engine.apply_restore_plan(
        &fetched,
        &binding,
        &plan,
        &approved,
        &repository.backups_dir()?,
        applied_at,
    )?;
    let kept_local_conversation_actions = plan
        .actions
        .iter()
        .filter(|action| is_conversation_keep_local_action(action, &fetched.snapshot.manifest))
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    let files_complete = applied
        .receipts
        .iter()
        .all(|receipt| match receipt.action_type {
            RestoreActionType::WriteProjectFile
            | RestoreActionType::EnsureProjectDirectory
            | RestoreActionType::DeleteProjectFile
            | RestoreActionType::DeleteProjectDirectory => {
                matches!(
                    receipt.status,
                    ActionStatus::Applied | ActionStatus::Skipped
                )
            }
            RestoreActionType::WriteFile
            | RestoreActionType::MergeFile
            | RestoreActionType::MaterializeConversation
            | RestoreActionType::InstallCustomSkill
            | RestoreActionType::OverwriteCustomSkill
            | RestoreActionType::ReviewHook
            | RestoreActionType::ReviewMcp
            | RestoreActionType::ApplySetting => receipt.status == ActionStatus::Applied,
            RestoreActionType::Manual => {
                receipt.status == ActionStatus::Applied
                    || (receipt.status == ActionStatus::Skipped
                        && kept_local_conversation_actions.contains(&receipt.action_id))
            }
            _ => true,
        });
    let status = if files_complete {
        MaterializationStatus::Complete
    } else {
        MaterializationStatus::Partial
    };
    let record = MaterializationRecord {
        materialization_id: MaterializationId::parse(generated_named_id("materialization")?)?,
        plan_id: plan.plan_id.clone(),
        host_id: binding.host_id.clone(),
        replica_id: binding.replica_id.clone(),
        local_project_id: binding.local_project_id.clone(),
        storage_id: plan.storage_id.clone(),
        bundle_id: plan.bundle_id.clone(),
        generation: plan.generation,
        commit_id: plan.commit_id.clone(),
        manifest_sha256: plan.manifest_sha256.clone(),
        binding_revision: binding.revision,
        status,
        applied_at,
        receipts: applied.receipts.clone(),
    };
    repository.mutate_materializations(|_, materializations| {
        if materializations
            .records
            .iter()
            .any(|current| current.plan_id == record.plan_id)
        {
            return Err(format!(
                "restore plan '{}' was already recorded",
                record.plan_id
            ));
        }
        materializations.records.push(record);
        Ok(())
    })?;
    let overall_success = !applied
        .receipts
        .iter()
        .any(|receipt| matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked));
    if files_complete {
        repository.mutate_config(|config| {
            let project = config
                .projects
                .iter_mut()
                .find(|project| project.local_project_id == binding.local_project_id)
                .ok_or_else(|| "bound project was removed while applying restore".to_string())?;
            let last_push_at = project
                .recipe_bases
                .get(&plan.storage_id)
                .and_then(|base| base.last_push_at);
            let previous_pull_at = project
                .recipe_bases
                .get(&plan.storage_id)
                .and_then(|base| base.last_pull_at);
            project.recipe_bases.insert(
                plan.storage_id.clone(),
                RecipeBase {
                    generation: plan.generation,
                    manifest_sha256: plan.manifest_sha256.clone(),
                    commit_id: Some(plan.commit_id.clone()),
                    recipe_revision: restored_recipe.revision,
                    binding_revision: Some(binding.revision),
                    last_pull_at: successful_pull_timestamp(
                        previous_pull_at,
                        applied_at,
                        overall_success,
                    ),
                    last_push_at,
                },
            );
            let link = config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == binding.local_project_id
                        && link.storage_id == plan.storage_id
                })
                .ok_or_else(|| {
                    "project storage link was removed while applying Pull".to_string()
                })?;
            link.recipe = Some(restored_recipe.clone());
            project.revision = project.revision.saturating_add(1);
            project.updated_at = applied_at;
            Ok(())
        })?;
    }
    let applied_action_ids = applied
        .receipts
        .iter()
        .filter(|receipt| receipt.status == ActionStatus::Applied)
        .map(|receipt| receipt.action_id.clone())
        .collect::<Vec<_>>();
    let failed_actions = applied
        .receipts
        .iter()
        .filter(|receipt| matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked))
        .map(|receipt| FailedAction {
            action_id: receipt.action_id.clone(),
            display_name: None,
            message: receipt
                .error
                .clone()
                .unwrap_or_else(|| "Action was not materialized".to_string()),
        })
        .collect::<Vec<_>>();
    let success = overall_success;
    Ok(RestoreResult {
        success,
        message: if success {
            format!("Applied {} restore actions", applied_action_ids.len())
        } else {
            format!(
                "Applied {} actions; {} require attention",
                applied_action_ids.len(),
                failed_actions.len()
            )
        },
        plan_id: plan.plan_id,
        applied_action_ids,
        failed_actions,
    })
}

fn successful_pull_timestamp(
    previous: Option<u64>,
    applied_at: u64,
    overall_success: bool,
) -> Option<u64> {
    overall_success.then_some(applied_at).or(previous)
}

/// Load the exact storage snapshot approved by a RestorePlan. Dependency and
/// readiness support must never discover a different linked storage on their
/// own, because each storage advances its bundle generations independently.
fn restore_support_context(
    repository: &V3Repository,
    restore_plan_id: &PlanId,
) -> Result<(RestorePlan, ProjectBinding, FetchedBundle), String> {
    let plan = repository.load_restore_plan(restore_plan_id)?;
    let now = now_secs();
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    let binding = current_binding_for_restore_plan(repository, &plan)?;
    let (_, fetched) = fetch_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    let head = &fetched.snapshot.head;
    if plan.storage_id != fetched.snapshot.storage_id
        || plan.bundle_id != head.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.generation != head.generation
        || plan.commit_id != head.commit_id
        || plan.manifest_sha256 != head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err(
            "bundle head changed after restore planning; refresh the Pull review".to_string(),
        );
    }
    Ok((plan, binding, fetched))
}

fn plan_dependencies_with_repository(
    repository: &V3Repository,
    restore_plan_id: &PlanId,
) -> Result<DependencyPlan, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let (restore_plan, binding, fetched) = restore_support_context(repository, restore_plan_id)?;
    let created_at = now_secs();
    let plan = DependencyPlan {
        schema_version: DEPENDENCY_PLAN_SCHEMA_V2,
        // `generated_named_id` accepts only lowercase alphanumeric prefixes.
        // The plan type and its persistence directory already distinguish
        // dependency plans from restore plans.
        plan_id: PlanId::parse(generated_named_id("plan")?)?,
        storage_id: restore_plan.storage_id,
        bundle_id: restore_plan.bundle_id,
        host_id: binding.host_id.clone(),
        replica_id: binding.replica_id,
        expected_host_identity_revision: restore_plan.expected_host_identity_revision,
        generation: restore_plan.generation,
        commit_id: restore_plan.commit_id,
        manifest_sha256: restore_plan.manifest_sha256,
        binding_revision: binding.revision,
        created_at,
        expires_at: restore_plan
            .expires_at
            .min(created_at.saturating_add(PLAN_LIFETIME_SECS)),
        actions: fetched.dependency_actions,
        blockers: Vec::new(),
        warnings: Vec::new(),
    };
    plan.validate()?;
    repository.save_dependency_plan(&plan)?;
    Ok(plan)
}

struct PreparedDependencyApplication {
    plan: DependencyPlan,
    binding: ProjectBinding,
    selected: BTreeSet<ActionId>,
    previous: Option<DependencyApplicationRecord>,
    resources: BTreeMap<ResourceId, ResourceDescriptor>,
    applied_at: u64,
}

fn prepare_dependency_application(
    repository: &V3Repository,
    plan_id: &PlanId,
    action_ids: Vec<ActionId>,
) -> Result<PreparedDependencyApplication, String> {
    let plan = repository.load_dependency_plan(plan_id)?;
    let applied_at = now_secs();
    if applied_at < plan.created_at || applied_at > plan.expires_at {
        return Err("dependency plan has expired or is not active yet".to_string());
    }
    let bindings = repository.load_bindings()?;
    let binding = bindings
        .bindings
        .iter()
        .find(|binding| {
            binding.host_id == plan.host_id
                && binding.replica_id == plan.replica_id
                && binding.state == BindingState::Active
        })
        .cloned()
        .ok_or_else(|| "dependency plan's project binding is no longer active".to_string())?;
    if binding.revision != plan.binding_revision || binding.bundle_id != plan.bundle_id {
        return Err("dependency plan's binding changed after planning".to_string());
    }
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != plan.host_id || identity.revision != plan.expected_host_identity_revision
    {
        return Err("dependency plan's host identity changed after planning".to_string());
    }
    let previous = repository
        .load_dependency_applications()?
        .records
        .into_iter()
        .find(|record| record.plan_id == plan.plan_id);
    if previous.as_ref().is_some_and(|record| {
        record.local_project_id != binding.local_project_id
            || record.storage_id != plan.storage_id
            || record.bundle_id != plan.bundle_id
            || record.host_id != plan.host_id
            || record.replica_id != plan.replica_id
            || record.generation != plan.generation
            || record.commit_id != plan.commit_id
            || record.manifest_sha256 != plan.manifest_sha256
            || record.binding_revision != plan.binding_revision
    }) {
        return Err(
            "stored dependency receipt does not match this plan's host and snapshot".to_string(),
        );
    }
    let binding = resolve_project_binding(repository, &binding)?;
    let (_, fetched) = fetch_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    validate_dependency_plan_pin(&plan, &fetched, &binding)?;
    let selected = unique_dependency_actions(&action_ids, &plan.actions)?;

    Ok(PreparedDependencyApplication {
        plan,
        binding,
        selected,
        previous,
        resources: fetched.snapshot.manifest.resources,
        applied_at,
    })
}

async fn apply_dependency_actions_with_repository(
    repository: &V3Repository,
    plan_id: &PlanId,
    action_ids: Vec<ActionId>,
) -> Result<DependencyResult, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let prepare_repository = repository.clone();
    let prepare_plan_id = plan_id.clone();
    let prepared = run_blocking(move || {
        prepare_dependency_application(&prepare_repository, &prepare_plan_id, action_ids)
    })
    .await?;
    let mut receipts = Vec::with_capacity(prepared.plan.actions.len());
    for action in &prepared.plan.actions {
        if let Some(applied) = prepared.previous.as_ref().and_then(|record| {
            record.receipts.iter().find(|receipt| {
                receipt.action_id == action.action_id && receipt.status == ActionStatus::Applied
            })
        }) {
            // A retry never re-runs an action that this exact pinned plan
            // already applied successfully.
            receipts.push(applied.clone());
            continue;
        }
        if !prepared.selected.contains(&action.action_id) {
            receipts.push(DependencyApplyReceipt {
                action_id: action.action_id.clone(),
                status: ActionStatus::Skipped,
                applied_at: prepared.applied_at,
                error: None,
            });
            continue;
        }
        let resource = prepared.resources.get(&action.resource_id).ok_or_else(|| {
            format!(
                "dependency action '{}' references a missing bundle resource",
                action.action_id
            )
        })?;
        let result = execute_dependency_action(
            repository,
            &prepared.plan,
            &prepared.binding,
            action,
            resource,
        )
        .await;
        receipts.push(DependencyApplyReceipt {
            action_id: action.action_id.clone(),
            status: if result.is_ok() {
                ActionStatus::Applied
            } else {
                ActionStatus::Failed
            },
            applied_at: prepared.applied_at,
            error: result.err(),
        });
    }

    let finalize_repository = repository.clone();
    run_blocking(move || finalize_dependency_application(&finalize_repository, prepared, receipts))
        .await
}

fn finalize_dependency_application(
    repository: &V3Repository,
    prepared: PreparedDependencyApplication,
    receipts: Vec<DependencyApplyReceipt>,
) -> Result<DependencyResult, String> {
    let PreparedDependencyApplication {
        plan,
        binding,
        applied_at,
        ..
    } = prepared;
    let record = DependencyApplicationRecord {
        plan_id: plan.plan_id.clone(),
        local_project_id: binding.local_project_id,
        storage_id: plan.storage_id.clone(),
        bundle_id: plan.bundle_id.clone(),
        host_id: binding.host_id,
        replica_id: binding.replica_id,
        generation: plan.generation,
        commit_id: plan.commit_id.clone(),
        manifest_sha256: plan.manifest_sha256.clone(),
        binding_revision: plan.binding_revision,
        applied_at,
        receipts: receipts.clone(),
    };
    repository.mutate_dependency_applications(|_, applications| {
        if let Some(current) = applications
            .records
            .iter_mut()
            .find(|current| current.plan_id == record.plan_id)
        {
            if current.local_project_id != record.local_project_id
                || current.storage_id != record.storage_id
                || current.bundle_id != record.bundle_id
                || current.host_id != record.host_id
                || current.replica_id != record.replica_id
                || current.generation != record.generation
                || current.commit_id != record.commit_id
                || current.manifest_sha256 != record.manifest_sha256
                || current.binding_revision != record.binding_revision
            {
                return Err(
                    "dependency receipt scope changed while actions were running".to_string(),
                );
            }
            *current = record;
        } else {
            applications.records.push(record);
        }
        Ok(())
    })?;
    let applied_action_ids = receipts
        .iter()
        .filter(|receipt| receipt.status == ActionStatus::Applied)
        .map(|receipt| receipt.action_id.clone())
        .collect::<Vec<_>>();
    let failed_actions = receipts
        .iter()
        .filter(|receipt| receipt.status == ActionStatus::Failed)
        .map(|receipt| FailedAction {
            action_id: receipt.action_id.clone(),
            display_name: plan
                .actions
                .iter()
                .find(|action| action.action_id == receipt.action_id)
                .map(|action| action.display_name.clone()),
            message: receipt
                .error
                .clone()
                .unwrap_or_else(|| "Dependency action failed".to_string()),
        })
        .collect::<Vec<_>>();
    let success = failed_actions.is_empty();
    Ok(DependencyResult {
        success,
        message: if success {
            format!("Applied {} dependency actions", applied_action_ids.len())
        } else {
            format!(
                "Applied {} dependencies; {} failed",
                applied_action_ids.len(),
                failed_actions.len()
            )
        },
        applied_action_ids,
        failed_actions,
    })
}

fn get_bundle_readiness_with_repository(
    repository: &V3Repository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) -> Result<BundleReadiness, String> {
    let binding = require_current_binding(repository, binding)?;
    if &binding.bundle_id != bundle_id {
        return Err("binding and requested bundle IDs differ".to_string());
    }
    let config = repository.load_config()?;
    let project = config
        .project(&binding.local_project_id)
        .ok_or_else(|| "bound project is no longer registered".to_string())?;
    require_project_link(&config, project, storage_id)?;
    let (_, fetched) = fetch_from_storage(repository, storage_id, bundle_id)?;
    bundle_readiness_for_fetched(repository, &binding, &fetched)
}

fn get_restore_readiness_with_repository(
    repository: &V3Repository,
    restore_plan_id: &PlanId,
) -> Result<BundleReadiness, String> {
    let (_, binding, fetched) = restore_support_context(repository, restore_plan_id)?;
    bundle_readiness_for_fetched(repository, &binding, &fetched)
}

fn bundle_readiness_for_fetched(
    repository: &V3Repository,
    binding: &ProjectBinding,
    fetched: &FetchedBundle,
) -> Result<BundleReadiness, String> {
    let head = &fetched.snapshot.head;
    let bundle_id = &head.bundle_id;
    let materialization = repository
        .load_materializations()?
        .records
        .into_iter()
        .rev()
        .find(|record| {
            record.host_id == binding.host_id
                && record.replica_id == binding.replica_id
                && record.bundle_id == *bundle_id
                && record.generation == head.generation
                && record.manifest_sha256 == head.manifest_sha256
                && record.binding_revision == binding.revision
                && record.status != MaterializationStatus::Detached
        });
    let dependency_applications = repository.load_dependency_applications()?;
    let mut applied_dependencies = BTreeSet::new();
    for application in dependency_applications
        .records
        .iter()
        .filter(|application| {
            application.host_id == binding.host_id
                && application.replica_id == binding.replica_id
                && application.bundle_id == *bundle_id
                && application.generation == head.generation
                && application.manifest_sha256 == head.manifest_sha256
                && application.binding_revision == binding.revision
        })
    {
        applied_dependencies.extend(
            application
                .receipts
                .iter()
                .filter(|receipt| receipt.status == ActionStatus::Applied)
                .map(|receipt| receipt.action_id.clone()),
        );
    }
    let mut issues = Vec::new();
    let needs_codex_home = fetched
        .snapshot
        .manifest
        .resources
        .values()
        .any(|resource| resource.kind == ResourceKind::CodexConversation);
    let needs_claude_home = fetched
        .snapshot
        .manifest
        .resources
        .values()
        .any(|resource| resource.kind == ResourceKind::ClaudeConversation);
    add_provider_home_issue(
        &mut issues,
        Provider::Codex,
        needs_codex_home,
        binding.codex_home.as_deref(),
    );
    add_provider_home_issue(
        &mut issues,
        Provider::Claude,
        needs_claude_home,
        binding.claude_home.as_deref(),
    );
    if !fetched.snapshot.manifest.files.is_empty() {
        match materialization.as_ref() {
            None => issues.push(BundleReadinessIssue {
                issue_id: "restore-files".to_string(),
                category: "project_setup".to_string(),
                title: "Pull has not been applied yet".to_string(),
                detail: Some(
                    "Open the Pull review and apply the changes you want on this machine."
                        .to_string(),
                ),
                severity: "warning".to_string(),
                provider: None,
                resource_id: None,
                action: Some("plan_restore".to_string()),
            }),
            Some(record) if record.status == MaterializationStatus::Partial => {
                add_partial_materialization_issues(&mut issues, record, fetched);
            }
            Some(_) => {}
        }
    }
    for action in &fetched.dependency_actions {
        if !applied_dependencies.contains(&action.action_id) {
            issues.push(BundleReadinessIssue {
                issue_id: format!("dependency-{}", action.action_id),
                category: match action.kind {
                    DependencyActionKind::InstallCodexPlugin
                    | DependencyActionKind::InstallClaudePlugin => "plugins",
                    DependencyActionKind::InstallStandaloneSkill => "skills",
                    _ => "tools",
                }
                .to_string(),
                title: format!("{} needs setup", action.display_name),
                detail: Some("Review and approve the pinned dependency action.".to_string()),
                severity: "warning".to_string(),
                provider: action.provider,
                resource_id: Some(action.resource_id.clone()),
                action: Some("apply_dependency".to_string()),
            });
        }
    }
    let blocked = issues.iter().any(|issue| issue.severity == "error");
    Ok(BundleReadiness {
        bundle_id: bundle_id.clone(),
        state: if blocked {
            "blocked"
        } else if issues.is_empty() {
            "ready"
        } else {
            "needs_setup"
        }
        .to_string(),
        issues,
        generated_at: now_secs(),
    })
}

fn add_partial_materialization_issues(
    issues: &mut Vec<BundleReadinessIssue>,
    materialization: &MaterializationRecord,
    fetched: &FetchedBundle,
) {
    let unresolved = materialization
        .receipts
        .iter()
        .filter(|receipt| receipt.status != ActionStatus::Applied)
        .collect::<Vec<_>>();

    let manual_conversations = unresolved
        .iter()
        .filter(|receipt| receipt.action_type == RestoreActionType::Manual)
        .filter(|receipt| !is_kept_local_conversation_receipt(receipt))
        .filter(|receipt| {
            fetched
                .snapshot
                .manifest
                .resources
                .get(&receipt.resource_id)
                .is_some_and(|resource| {
                    matches!(
                        resource.kind,
                        ResourceKind::CodexConversation | ResourceKind::ClaudeConversation
                    )
                })
        })
        .map(|receipt| receipt.resource_id.clone())
        .collect::<BTreeSet<_>>();
    if !manual_conversations.is_empty() {
        let count = manual_conversations.len();
        let resource_id = (count == 1)
            .then(|| manual_conversations.iter().next().cloned())
            .flatten();
        let provider = resource_id
            .as_ref()
            .and_then(|resource_id| fetched.snapshot.manifest.resources.get(resource_id))
            .and_then(|resource| resource.provider);
        issues.push(BundleReadinessIssue {
            issue_id: "conversation-conflicts".to_string(),
            category: "sessions".to_string(),
            title: if count == 1 {
                "1 conversation conflict still needs manual resolution".to_string()
            } else {
                format!("{count} conversation conflicts still need manual resolution")
            },
            detail: Some(
                "The other selected changes were restored. The conflicted local conversation was left unchanged."
                    .to_string(),
            ),
            severity: "warning".to_string(),
            provider,
            resource_id,
            action: Some("resolve_conflict".to_string()),
        });
    }

    let incomplete_project_data = unresolved
        .iter()
        .filter(|receipt| {
            if receipt.action_type == RestoreActionType::Manual {
                if is_kept_local_conversation_receipt(receipt) {
                    return false;
                }
                return !manual_conversations.contains(&receipt.resource_id);
            }
            match receipt.action_type {
                RestoreActionType::WriteFile
                | RestoreActionType::MergeFile
                | RestoreActionType::MaterializeConversation
                | RestoreActionType::ReviewHook
                | RestoreActionType::ReviewMcp
                | RestoreActionType::ApplySetting => true,
                RestoreActionType::WriteProjectFile
                | RestoreActionType::EnsureProjectDirectory
                | RestoreActionType::DeleteProjectFile
                | RestoreActionType::DeleteProjectDirectory => {
                    matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked)
                }
                RestoreActionType::InstallStandaloneSkill
                | RestoreActionType::InstallCustomSkill
                | RestoreActionType::OverwriteCustomSkill
                | RestoreActionType::InstallPlugin
                | RestoreActionType::Manual => false,
            }
        })
        .copied()
        .collect::<Vec<&ApplyReceipt>>();
    if !incomplete_project_data.is_empty() {
        let resource_ids = incomplete_project_data
            .iter()
            .map(|receipt| receipt.resource_id.clone())
            .collect::<BTreeSet<_>>();
        let count = resource_ids.len();
        let failed = incomplete_project_data
            .iter()
            .any(|receipt| matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked));
        issues.push(BundleReadinessIssue {
            issue_id: "restore-project-data".to_string(),
            category: "project_setup".to_string(),
            title: if count == 1 {
                "1 project data change was left unchanged".to_string()
            } else {
                format!("{count} project data changes were left unchanged")
            },
            detail: Some(if failed {
                "Review the technical result, fix the reported problem, and try Pull again."
                    .to_string()
            } else {
                "Review this change separately before applying it on this machine.".to_string()
            }),
            severity: if failed { "error" } else { "warning" }.to_string(),
            provider: None,
            resource_id: (count == 1)
                .then(|| resource_ids.iter().next().cloned())
                .flatten(),
            action: Some("review_restore".to_string()),
        });
    }
}

fn require_current_binding(
    repository: &V3Repository,
    supplied: &ProjectBinding,
) -> Result<ProjectBinding, String> {
    if supplied.state != BindingState::Active {
        return Err("project binding is not active".to_string());
    }
    let bindings = repository.load_bindings()?;
    let current = bindings
        .bindings
        .into_iter()
        .find(|binding| {
            binding.host_id == supplied.host_id && binding.replica_id == supplied.replica_id
        })
        .ok_or_else(|| "project binding is not registered on this machine".to_string())?;
    if &current != supplied {
        return Err("project binding changed; refresh before continuing".to_string());
    }
    resolve_project_binding(repository, &current)
}

fn resolve_project_binding(
    repository: &V3Repository,
    binding: &ProjectBinding,
) -> Result<ProjectBinding, String> {
    let profile_paths = resolve_profile_paths(repository, &binding.profile_ids)?;
    let mut resolved = binding.clone();
    resolved.codex_home = profile_paths.get(&Provider::Codex).cloned();
    resolved.claude_home = profile_paths.get(&Provider::Claude).cloned();
    resolved.validate_structure()?;
    Ok(resolved)
}

fn current_binding_for_restore_plan(
    repository: &V3Repository,
    plan: &RestorePlan,
) -> Result<ProjectBinding, String> {
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != plan.host_id || identity.revision != plan.expected_host_identity_revision
    {
        return Err("restore plan's host identity changed after planning".to_string());
    }
    let binding = repository
        .load_bindings()?
        .bindings
        .into_iter()
        .find(|binding| {
            binding.host_id == plan.host_id
                && binding.replica_id == plan.replica_id
                && binding.state == BindingState::Active
        })
        .ok_or_else(|| "restore plan's project binding is no longer active".to_string())?;
    if binding.revision != plan.binding_revision || binding.bundle_id != plan.bundle_id {
        return Err("restore plan's project binding changed after planning".to_string());
    }
    resolve_project_binding(repository, &binding)
}

fn project_open_commands(binding: &ProjectBinding) -> Vec<(&'static str, String)> {
    let project_root = shell_quote(&binding.project_root);
    let mut commands = Vec::new();
    if let Some(codex_home) = &binding.codex_home {
        let environment = format!("CODEX_HOME={}", shell_quote(codex_home));
        commands.push((
            "Codex — new",
            format!("{} codex -C {}", environment, project_root),
        ));
        commands.push((
            "Codex — resume",
            format!("{} codex resume -C {}", environment, project_root),
        ));
    }
    if let Some(claude_home) = &binding.claude_home {
        let environment = format!("CLAUDE_CONFIG_DIR={}", shell_quote(claude_home));
        commands.push((
            "Claude — new",
            format!("cd {} && {} claude", project_root, environment),
        ));
        commands.push((
            "Claude — resume",
            format!("cd {} && {} claude --resume", project_root, environment),
        ));
    }
    commands
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn unique_approved_actions(
    requested: &[ActionId],
    actions: &[super::domain::RestoreAction],
) -> Result<BTreeSet<ActionId>, String> {
    let selected = requested.iter().cloned().collect::<BTreeSet<_>>();
    if selected.len() != requested.len() {
        return Err("approved restore actions contain duplicates".to_string());
    }
    let available = actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = selected.iter().find(|id| !available.contains(*id)) {
        return Err(format!("restore plan has no action '{}'", unknown));
    }
    Ok(selected)
}

fn unique_dependency_actions(
    requested: &[ActionId],
    actions: &[DependencyAction],
) -> Result<BTreeSet<ActionId>, String> {
    let selected = requested.iter().cloned().collect::<BTreeSet<_>>();
    if selected.len() != requested.len() {
        return Err("approved dependency actions contain duplicates".to_string());
    }
    let available = actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = selected.iter().find(|id| !available.contains(*id)) {
        return Err(format!("dependency plan has no action '{}'", unknown));
    }
    Ok(selected)
}

fn validate_dependency_plan_pin(
    plan: &DependencyPlan,
    fetched: &FetchedBundle,
    binding: &ProjectBinding,
) -> Result<(), String> {
    if plan.storage_id != fetched.snapshot.storage_id
        || plan.bundle_id != fetched.snapshot.head.bundle_id
        || plan.bundle_id != binding.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.generation != fetched.snapshot.head.generation
        || plan.commit_id != fetched.snapshot.head.commit_id
        || plan.manifest_sha256 != fetched.snapshot.head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err("dependency plan no longer matches the bundle or binding".to_string());
    }
    Ok(())
}

async fn execute_dependency_action(
    repository: &V3Repository,
    plan: &DependencyPlan,
    binding: &ProjectBinding,
    action: &DependencyAction,
    resource: &ResourceDescriptor,
) -> Result<(), String> {
    action.validate()?;
    match action.kind {
        DependencyActionKind::InstallStandaloneSkill => {
            let materialized = repository
                .load_materializations()?
                .records
                .iter()
                .any(|record| {
                    record.host_id == binding.host_id
                        && record.replica_id == binding.replica_id
                        && record.bundle_id == plan.bundle_id
                        && record.generation == plan.generation
                        && record.manifest_sha256 == plan.manifest_sha256
                        && record.binding_revision == binding.revision
                        && record.receipts.iter().any(|receipt| {
                            receipt.resource_id == action.resource_id
                                && receipt.status == ActionStatus::Applied
                        })
                });
            if materialized {
                Ok(())
            } else {
                Err("restore the standalone-skill payload before approving it".to_string())
            }
        }
        DependencyActionKind::InstallCodexPlugin => {
            run_codex_plugin_install(action, binding, resource).await
        }
        DependencyActionKind::InstallClaudePlugin => {
            run_plugin_install("claude", action, binding).await
        }
        DependencyActionKind::CheckBinary
        | DependencyActionKind::CheckEnvironment
        | DependencyActionKind::Manual => {
            Err("this dependency requires a provider-specific manual check".to_string())
        }
    }
}

fn validate_plugin_install_intent(action: &DependencyAction) -> Result<(), String> {
    if !portable_plugin_id(&action.display_name) {
        return Err("plugin identifier is not safe for native installation".to_string());
    }
    // Global plugins install without a scope flag; project-declared Claude
    // plugins keep their project scope. Anything else is rejected before a
    // process is launched.
    let supported_argument_sets: Vec<Vec<String>> = match action.kind {
        DependencyActionKind::InstallCodexPlugin => vec![vec![
            "plugin".to_string(),
            "add".to_string(),
            action.display_name.clone(),
        ]],
        DependencyActionKind::InstallClaudePlugin => vec![
            vec![
                "plugin".to_string(),
                "install".to_string(),
                action.display_name.clone(),
                "--scope".to_string(),
                "project".to_string(),
            ],
            vec![
                "plugin".to_string(),
                "install".to_string(),
                action.display_name.clone(),
            ],
        ],
        _ => return Err("dependency is not a plugin installation".to_string()),
    };
    if !supported_argument_sets.contains(&action.argv) {
        return Err("plugin install arguments differ from the supported intent".to_string());
    }
    Ok(())
}

fn codex_plugin_lock_for_action(
    action: &DependencyAction,
    resource: &ResourceDescriptor,
) -> Result<codex_plugins::CodexPluginLock, String> {
    validate_plugin_install_intent(action)?;
    if action.kind != DependencyActionKind::InstallCodexPlugin
        || resource.resource_id != action.resource_id
        || resource.kind != ResourceKind::Plugin
        || resource.provider != Some(Provider::Codex)
    {
        return Err("Codex plugin dependency does not match its bundle resource".to_string());
    }
    let (_, marketplace) = action
        .display_name
        .rsplit_once('@')
        .ok_or_else(|| "Codex plugin identifier has no marketplace".to_string())?;
    if resource
        .metadata
        .get("plugin_marketplace")
        .is_some_and(|captured| captured != marketplace)
    {
        return Err("Codex plugin marketplace metadata does not match its identifier".to_string());
    }

    let mut lock = codex_plugins::empty_lock();
    lock.plugins.push(codex_plugins::CodexPluginIntent {
        id: action.display_name.clone(),
        observed_version: resource.metadata.get("plugin_observed_version").cloned(),
    });
    match resource
        .metadata
        .get("plugin_source_type")
        .map(String::as_str)
    {
        Some("git") => {
            let source = resource.metadata.get("plugin_source").ok_or_else(|| {
                format!(
                    "Codex plugin '{}' is missing its portable marketplace source",
                    action.display_name
                )
            })?;
            lock.marketplaces
                .push(codex_plugins::CodexMarketplaceIntent {
                    name: marketplace.to_string(),
                    repository: source.clone(),
                    git_ref: None,
                });
        }
        Some("local") | None => {}
        Some(source_type) => {
            return Err(format!(
                "Codex plugin '{}' has unsupported marketplace source type '{}'",
                action.display_name, source_type
            ));
        }
    }
    codex_plugins::validate_lock(&lock)?;
    Ok(lock)
}

// Dependency receipt errors are capped at 8 KiB. Reserve room for the plugin
// identifier and the stable error prefix wrapped around these diagnostics.
const MAX_PLUGIN_DIAGNOSTIC_BYTES: usize = 7 * 1024;

fn append_plugin_diagnostic(diagnostics: &mut String, level: &str, message: &str) {
    if diagnostics.len() >= MAX_PLUGIN_DIAGNOSTIC_BYTES {
        return;
    }
    let message = codex_plugins::redact(message.trim());
    if message.is_empty() {
        return;
    }
    let line = format!("{}: {}\n", level, message);
    let remaining = MAX_PLUGIN_DIAGNOSTIC_BYTES - diagnostics.len();
    let mut end = line.len().min(remaining);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    diagnostics.push_str(&line[..end]);
}

fn plugin_failure_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let mut diagnostics = String::new();
    for line in String::from_utf8_lossy(stderr)
        .lines()
        .chain(String::from_utf8_lossy(stdout).lines())
    {
        append_plugin_diagnostic(&mut diagnostics, "installer", line);
    }
    diagnostics.trim().to_string()
}

async fn run_codex_plugin_install(
    action: &DependencyAction,
    binding: &ProjectBinding,
    resource: &ResourceDescriptor,
) -> Result<(), String> {
    let lock = codex_plugin_lock_for_action(action, resource)?;
    let plugin_id = action.display_name.clone();
    let target_home = binding
        .codex_home
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "Codex plugin installation requires a configured Codex home".to_string())?;
    let default_home = dirs::home_dir()
        .map(|home| home.join(".codex"))
        .ok_or_else(|| "could not resolve the default Codex home".to_string())?;

    tokio::task::spawn_blocking(move || {
        let codex = codex_plugins::find_binary("codex")?;
        let target_runner = codex_plugins::ProcessRunner::for_binary(codex.clone())
            .with_codex_home(Some(target_home.clone()));
        let default_runner = codex_plugins::ProcessRunner::for_binary(codex)
            .with_codex_home(Some(default_home.clone()));
        let mut diagnostics = String::new();
        let mut log =
            |level: &str, message: &str| append_plugin_diagnostic(&mut diagnostics, level, message);
        let repair = codex_plugins::apply_approved_managed_plan(
            &target_runner,
            &default_runner,
            &lock,
            &target_home,
            &default_home,
            &mut log,
        );
        drop(log);
        let report = repair.map_err(|error| {
            append_plugin_diagnostic(&mut diagnostics, "error", &error);
            format!(
                "codex plugin '{}' setup failed: {}",
                plugin_id,
                diagnostics.trim()
            )
        })?;
        let satisfied = report
            .plugins_installed
            .iter()
            .chain(&report.already_present)
            .any(|installed| installed == &plugin_id);
        let failed = report.failed.iter().any(|failed| failed == &plugin_id);
        let blocked = report
            .blocked_plugins
            .iter()
            .any(|blocked| blocked.id == plugin_id);
        if satisfied && !failed && !blocked {
            return Ok(());
        }
        let detail = diagnostics.trim();
        Err(if detail.is_empty() {
            format!(
                "codex plugin '{}' was not present after installation",
                plugin_id
            )
        } else {
            format!("codex plugin '{}' was not installed: {}", plugin_id, detail)
        })
    })
    .await
    .map_err(|error| format!("Codex plugin installer task failed: {}", error))?
}

async fn run_plugin_install(
    program: &str,
    action: &DependencyAction,
    binding: &ProjectBinding,
) -> Result<(), String> {
    validate_plugin_install_intent(action)?;
    let mut command = Command::new(program);
    command
        .args(&action.argv)
        .current_dir(&binding.project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(codex_home) = &binding.codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    if let Some(claude_home) = &binding.claude_home {
        command.env("CLAUDE_CONFIG_DIR", claude_home);
    }
    let output = timeout(DEFAULT_OPERATION_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("{} plugin installation timed out", program))?
        .map_err(|error| format!("start {} plugin installer: {}", program, error))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = plugin_failure_detail(&output.stdout, &output.stderr);
        let status = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated".to_string());
        Err(if detail.is_empty() {
            format!("{} plugin installer exited with status {}", program, status)
        } else {
            format!(
                "{} plugin installer exited with status {}: {}",
                program, status, detail
            )
        })
    }
}

fn portable_plugin_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.starts_with('-')
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'/' | b'.' | b'_' | b'-' | b':')
        })
}

fn add_provider_home_issue(
    issues: &mut Vec<BundleReadinessIssue>,
    provider: Provider,
    required: bool,
    home: Option<&str>,
) {
    if !required {
        return;
    }
    let valid = home.is_some_and(|path| {
        fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    });
    if valid {
        return;
    }
    let name = match provider {
        Provider::Codex => "Codex",
        Provider::Claude => "Claude",
    };
    issues.push(BundleReadinessIssue {
        issue_id: format!("provider-home-{}", name.to_ascii_lowercase()),
        category: "conversations".to_string(),
        title: format!("{} home is not mapped", name),
        detail: Some(format!(
            "Choose an existing {} home before restoring provider state.",
            name
        )),
        severity: "error".to_string(),
        provider: Some(provider),
        resource_id: None,
        action: Some("edit_binding".to_string()),
    });
}

fn capture_request_for_binding(
    repository: &V3Repository,
    binding: &ProjectBinding,
) -> Result<CaptureRequest, String> {
    let binding = resolve_project_binding(repository, binding)?;
    let resolved_project = fs_canonicalize(Path::new(&binding.project_root))?;
    if resolved_project != PathBuf::from(&binding.canonical_project_root) {
        return Err("project binding resolves to a different checkout".to_string());
    }
    let excluded_project_roots = repository
        .load_bindings()?
        .bindings
        .into_iter()
        .filter(|other| {
            other.state == BindingState::Active && other.replica_id != binding.replica_id
        })
        .map(|other| PathBuf::from(other.canonical_project_root))
        .filter(|root| root.starts_with(&resolved_project) && root != &resolved_project)
        .collect();
    let mut request = capture_request_with_global_inventory(
        resolved_project,
        binding.codex_home.as_ref().map(PathBuf::from),
        binding.claude_home.as_ref().map(PathBuf::from),
        excluded_project_roots,
    );
    let config = repository.load_config()?;
    request
        .excluded_content_roots
        .push(repository.root().to_path_buf());
    request.excluded_content_roots.extend(
        config
            .storages
            .iter()
            .filter(|storage| storage.kind == StorageKind::Local)
            .map(|storage| PathBuf::from(&storage.local_dir)),
    );
    if let Some(home) = &binding.codex_home {
        request.excluded_content_roots.push(PathBuf::from(home));
    }
    if let Some(home) = &binding.claude_home {
        request.excluded_content_roots.push(PathBuf::from(home));
    }
    Ok(request)
}

/// Build a capture request whose global plugin and custom-skill candidates
/// come from an ownership-ordered inventory of the mapped provider homes.
/// Discovery keeps every global resource unselected by default; the recipe
/// remains the only selection authority.
fn capture_request_with_global_inventory(
    project_root: PathBuf,
    codex_home: Option<PathBuf>,
    claude_home: Option<PathBuf>,
    excluded_project_roots: Vec<PathBuf>,
) -> CaptureRequest {
    let mut standalone_skills = Vec::new();
    let mut global_plugins = Vec::new();
    let mut blocked_global_skills = Vec::new();
    for (provider, home) in [
        (CaptureProvider::Codex, codex_home.as_deref()),
        (CaptureProvider::Claude, claude_home.as_deref()),
    ] {
        let Some(home) = home else { continue };
        let inventory = global_inventory::inventory_provider_home(provider, home);
        standalone_skills.extend(inventory.standalone_skills);
        global_plugins.extend(inventory.plugins);
        blocked_global_skills.extend(inventory.blocked_skills);
    }
    CaptureRequest {
        project_root,
        codex_home,
        claude_home,
        excluded_project_roots,
        standalone_skills,
        global_plugins,
        blocked_global_skills,
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    }
}

fn repository_fingerprint(project_root: &Path) -> Option<String> {
    let config_path = project_root.join(".git/config");
    let metadata = fs::symlink_metadata(&config_path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return None;
    }
    let contents = fs::read_to_string(config_path).ok()?;
    let mut in_origin = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.eq_ignore_ascii_case("[remote \"origin\"]");
            continue;
        }
        if in_origin {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("url") {
                let normalized = strip_url_userinfo(value.trim());
                let digest = Sha256::digest(normalized.as_bytes());
                return Some(hex_digest(&digest));
            }
        }
    }
    None
}

fn strip_url_userinfo(value: &str) -> String {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return value.to_string();
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let Some(at) = authority.rfind('@') else {
        return value.to_string();
    };
    format!("{}://{}", scheme, &remainder[at + 1..])
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{:02x}", byte);
    }
    output
}

fn require_project_link(
    config: &SyncConfigV3,
    project: &LocalProjectRegistration,
    storage_id: &StorageId,
) -> Result<(), String> {
    if config.links.iter().any(|link| {
        link.local_project_id == project.local_project_id
            && &link.storage_id == storage_id
            && link.bundle_id == project.bundle_id
    }) {
        Ok(())
    } else {
        Err(format!(
            "project '{}' is not linked to storage '{}'",
            project.local_project_id, storage_id
        ))
    }
}

fn storage_engine(
    repository: &V3Repository,
    storage_id: &StorageId,
) -> Result<(StorageConfigV3, StorageEngine), String> {
    let config = repository.load_config()?;
    let storage = config
        .storages
        .iter()
        .find(|storage| &storage.id == storage_id)
        .cloned()
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;
    let machine = repository.load_bindings()?;
    validate_config_storage_isolation(repository, &config, &machine.bindings, &machine.profiles)?;
    let engine = engine_for_storage_config(&storage)?;
    Ok((storage, engine))
}

/// Build an engine directly from a storage configuration.  Setup drafts use
/// this for pending storage that is not part of the saved config yet.
fn engine_for_storage_config(storage: &StorageConfigV3) -> Result<StorageEngine, String> {
    let store = match storage.kind {
        StorageKind::Local => {
            ConfiguredStore::Local(LocalBundleObjectStore::open(&storage.local_dir)?)
        }
        StorageKind::S3 => ConfiguredStore::S3(S3BundleObjectStore::from_current_runtime(storage)?),
    };
    BundleEngine::open(store, storage.id.clone())
}

fn validate_config_storage_isolation(
    repository: &V3Repository,
    config: &SyncConfigV3,
    bindings: &[ProjectBinding],
    profiles: &[ProviderProfile],
) -> Result<(), String> {
    let repository_root = prospective_canonical(repository.root())?;
    let mut local_roots = Vec::<(&StorageConfigV3, PathBuf)>::new();
    for storage in config
        .storages
        .iter()
        .filter(|storage| storage.kind == StorageKind::Local)
    {
        let root = prospective_canonical(Path::new(&storage.local_dir))?;
        if paths_overlap(&root, &repository_root) {
            return Err(format!(
                "local storage '{}' overlaps schema-3 application data",
                storage.name
            ));
        }
        for (other, other_root) in &local_roots {
            if paths_overlap(&root, other_root) {
                return Err(format!(
                    "local storages '{}' and '{}' overlap",
                    storage.name, other.name
                ));
            }
        }
        for binding in bindings
            .iter()
            .filter(|binding| binding.state == BindingState::Active)
        {
            let target = prospective_canonical(Path::new(&binding.canonical_project_root))?;
            if paths_overlap(&root, &target) {
                return Err(format!(
                    "local storage '{}' overlaps project root '{}'",
                    storage.name,
                    target.display()
                ));
            }
        }
        for profile in profiles {
            let target = prospective_canonical(Path::new(&profile.canonical_path))?;
            if paths_overlap(&root, &target) {
                return Err(format!(
                    "local storage '{}' overlaps {} profile '{}'",
                    storage.name,
                    provider_name(profile.provider),
                    target.display()
                ));
            }
        }
        local_roots.push((storage, root));
    }
    Ok(())
}

fn prospective_canonical(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(format!(
            "path '{}' is not absolute and clean",
            path.display()
        ));
    }
    let mut cursor = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(_) => {
                let mut resolved = fs::canonicalize(&cursor)
                    .map_err(|error| format!("resolve '{}': {}", cursor.display(), error))?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    format!("cannot resolve prospective path '{}'", path.display())
                })?;
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| format!("'{}' has no existing ancestor", path.display()))?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(format!("inspect '{}': {}", cursor.display(), error));
            }
        }
    }
}

fn get_project_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<Option<ProjectDetail>, String> {
    let config = repository.load_config()?;
    let Some(project) = config.project(local_project_id).cloned() else {
        return Ok(None);
    };
    let links = config
        .links
        .into_iter()
        .filter(|link| &link.local_project_id == local_project_id)
        .collect();
    let binding = repository
        .load_bindings()?
        .for_project(local_project_id)
        .cloned();
    let materializations = repository
        .load_materializations()?
        .records
        .into_iter()
        .filter(|record| &record.local_project_id == local_project_id)
        .collect();
    Ok(Some(ProjectDetail {
        project,
        links,
        binding,
        materializations,
    }))
}

/// Machine-local default nickname for a new project: repo name plus the
/// provider config it uses ("healthGame (conf2)"). The hostname stands in
/// when no config is known (registration without a binding), and a counter
/// is the last resort, so sibling projects stay distinguishable out of the
/// box.
fn default_local_alias(
    display_name: &str,
    config_qualifier: Option<&str>,
    existing: &[LocalProjectRegistration],
) -> Option<String> {
    let qualifier = config_qualifier
        .map(str::trim)
        .filter(|qualifier| !qualifier.is_empty())
        .map(|qualifier| {
            // Abbreviate generated config names to their distinctive part:
            // "conf2 · Codex" → "conf2", "Default Codex" → "Codex".
            // User-chosen names pass through unchanged.
            let stem = qualifier.split(" · ").next().unwrap_or(qualifier);
            stem.strip_prefix("Default ")
                .filter(|rest| !rest.trim().is_empty())
                .unwrap_or(stem)
                .to_string()
        })
        .unwrap_or_else(default_machine_name);
    let taken: BTreeSet<&str> = existing
        .iter()
        .map(|project| {
            project
                .local_alias
                .as_deref()
                .unwrap_or(&project.display_name)
                .trim()
        })
        .collect();
    let stem = format!("{} ({})", display_name.trim(), qualifier);
    let mut candidate = stem.clone();
    let mut counter = 2u64;
    while taken.contains(candidate.as_str()) {
        candidate = format!("{} {}", stem, counter);
        counter += 1;
    }
    // Fall back to the bare display name rather than letting a pathological
    // hostname fail display-text validation and block registration.
    Some(candidate).filter(|value| value.len() <= 1_024 && !value.chars().any(char::is_control))
}

fn register_local_project_with_repository(
    repository: &V3Repository,
    request: RegisterLocalProjectRequest,
) -> Result<LocalProjectRegistration, String> {
    let now = now_secs();
    let mut project = LocalProjectRegistration {
        local_project_id: LocalProjectId::parse(generated_named_id("project")?)?,
        bundle_id: match request.bundle_id {
            Some(bundle_id) => bundle_id,
            None => BundleId::generate()?,
        },
        display_name: request.display_name,
        local_alias: None,
        repository_fingerprint: request.repository_fingerprint,
        recipe: BundleRecipe::default(),
        recipe_bases: Default::default(),
        revision: 0,
        created_at: now,
        updated_at: now,
    };
    repository.mutate_config(|config| {
        if config
            .projects
            .iter()
            .any(|existing| existing.local_project_id == project.local_project_id)
        {
            return Err("generated local project id collision; try again".to_string());
        }
        project.local_alias = default_local_alias(&project.display_name, None, &config.projects);
        project.validate()?;
        config.projects.push(project.clone());
        Ok(project)
    })
}

fn save_bundle_recipe_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    mut recipe: BundleRecipe,
) -> Result<LocalProjectRegistration, String> {
    recipe.validate()?;
    repository.mutate_config(|config| {
        let project = config
            .projects
            .iter_mut()
            .find(|project| &project.local_project_id == local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        if recipe.revision != project.recipe.revision {
            return Err(format!(
                "bundle recipe changed (expected revision {}, current {})",
                recipe.revision, project.recipe.revision
            ));
        }
        recipe.revision = recipe.revision.saturating_add(1);
        project.recipe = recipe;
        project.revision = project.revision.saturating_add(1);
        project.updated_at = now_secs();
        Ok(project.clone())
    })
}

fn save_project_link_with_repository(
    repository: &V3Repository,
    request: SaveProjectLinkRequest,
) -> Result<ProjectStorageLink, String> {
    repository.mutate_config(|config| {
        let project = config
            .project(&request.local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", request.local_project_id))?;
        if !config
            .storages
            .iter()
            .any(|storage| storage.id == request.storage_id)
        {
            return Err(format!("unknown storage '{}'", request.storage_id));
        }
        let link = ProjectStorageLink {
            local_project_id: request.local_project_id.clone(),
            storage_id: request.storage_id.clone(),
            bundle_id: project.bundle_id.clone(),
            recipe: None,
            project_content_preferences: Default::default(),
            pinned: request.pinned,
            created_at: now_secs(),
        };
        if let Some(existing) = config.links.iter_mut().find(|existing| {
            existing.local_project_id == link.local_project_id
                && existing.storage_id == link.storage_id
        }) {
            // Preserve creation time while updating user intent.
            let created_at = existing.created_at;
            let recipe = existing.recipe.clone();
            let project_content_preferences = existing.project_content_preferences.clone();
            *existing = link.clone();
            existing.created_at = created_at;
            existing.recipe = recipe.clone();
            existing.project_content_preferences = project_content_preferences.clone();
            let mut returned = link;
            returned.created_at = created_at;
            returned.recipe = recipe;
            returned.project_content_preferences = project_content_preferences;
            Ok(returned)
        } else {
            config.links.push(link.clone());
            Ok(link)
        }
    })
}

fn connect_project_to_remote_bundle_with_repository(
    repository: &V3Repository,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let _host_operation = repository.acquire_host_operation()?;
    connect_project_to_remote_bundle_guarded(repository, request)
}

fn connect_project_to_remote_bundle_guarded(
    repository: &V3Repository,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let config = repository.load_config()?;
    let project = config
        .project(&request.local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", request.local_project_id))?;
    if project.bundle_id != request.expected_bundle_id {
        return Err(format!(
            "project bundle changed (expected '{}', current '{}')",
            request.expected_bundle_id, project.bundle_id
        ));
    }
    if !config
        .storages
        .iter()
        .any(|storage| storage.id == request.storage_id)
    {
        return Err(format!("unknown storage '{}'", request.storage_id));
    }
    if project.bundle_id == request.bundle_id {
        save_project_link_with_repository(
            repository,
            SaveProjectLinkRequest {
                local_project_id: request.local_project_id.clone(),
                storage_id: request.storage_id,
                pinned: request.pinned,
            },
        )?;
        return get_project_with_repository(repository, &request.local_project_id)?
            .ok_or_else(|| "project disappeared while linking storage".to_string());
    }

    let other_links = config
        .links
        .iter()
        .filter(|link| {
            link.local_project_id == request.local_project_id
                && link.storage_id != request.storage_id
        })
        .count();
    if other_links > 0 {
        return Err(
            "cannot change bundle identity while the project is linked to another storage; unlink the other storage first"
                .to_string(),
        );
    }
    if !project.recipe_bases.is_empty() {
        return Err(
            "cannot change bundle identity after this project established a reviewed sync base"
                .to_string(),
        );
    }
    if repository
        .load_materializations()?
        .records
        .iter()
        .any(|record| record.local_project_id == request.local_project_id)
    {
        return Err(
            "cannot change bundle identity after project resources were materialized".to_string(),
        );
    }
    if repository
        .load_dependency_applications()?
        .records
        .iter()
        .any(|record| record.local_project_id == request.local_project_id)
    {
        return Err(
            "cannot change bundle identity after project dependencies were applied".to_string(),
        );
    }

    let (_, engine) = storage_engine(repository, &request.storage_id)?;
    let remote = engine.inspect(&request.bundle_id)?;
    let remote_fingerprint = remote.manifest.bundle.repository_fingerprint.clone();
    if let (Some(local), Some(remote)) = (
        project.repository_fingerprint.as_deref(),
        remote_fingerprint.as_deref(),
    ) {
        if local != remote && !request.allow_repository_mismatch {
            return Err("remote bundle belongs to a different repository".to_string());
        }
    } else if project.repository_fingerprint.is_some() && !request.allow_repository_mismatch {
        return Err("remote bundle does not declare a repository fingerprint".to_string());
    }
    if engine.read_head(&project.bundle_id)?.is_some() {
        return Err(
            "the current bundle already exists in this storage; refusing to abandon published history"
                .to_string(),
        );
    }

    let machine = repository.load_bindings()?;
    let active_binding = machine.active_for(&request.local_project_id).cloned();
    if let Some(binding) = &active_binding {
        validate_binding_request(
            repository,
            &SaveProjectBindingRequest {
                local_project_id: request.local_project_id.clone(),
                project_root: binding.project_root.clone(),
                profile_ids: binding.profile_ids.clone(),
                expected_revision: None,
            },
        )?;
    }
    let removed_bindings = machine
        .bindings
        .iter()
        .filter(|binding| binding.local_project_id == request.local_project_id)
        .cloned()
        .collect::<Vec<_>>();
    repository.mutate_bindings(|_, bindings| {
        bindings
            .bindings
            .retain(|binding| binding.local_project_id != request.local_project_id);
        Ok(())
    })?;

    let remote_recipe = remote.manifest.recipe.clone();
    let new_bundle_id = request.bundle_id.clone();
    let config_result = repository.mutate_config(|config| {
        let project = config
            .projects
            .iter_mut()
            .find(|project| project.local_project_id == request.local_project_id)
            .ok_or_else(|| "project was removed while connecting the remote bundle".to_string())?;
        if project.bundle_id != request.expected_bundle_id {
            return Err("project bundle changed while connecting remote storage".to_string());
        }
        project.bundle_id = new_bundle_id.clone();
        project.recipe = remote_recipe.clone();
        if project.repository_fingerprint.is_none()
            || (request.allow_repository_mismatch && remote_fingerprint.is_some())
        {
            project.repository_fingerprint = remote_fingerprint.clone();
        }
        project.revision = project.revision.saturating_add(1);
        project.updated_at = now_secs();
        for link in config
            .links
            .iter_mut()
            .filter(|link| link.local_project_id == request.local_project_id)
        {
            link.bundle_id = new_bundle_id.clone();
            link.recipe = None;
            link.pinned = request.pinned;
        }
        if !config.links.iter().any(|link| {
            link.local_project_id == request.local_project_id
                && link.storage_id == request.storage_id
        }) {
            config.links.push(ProjectStorageLink {
                local_project_id: request.local_project_id.clone(),
                storage_id: request.storage_id.clone(),
                bundle_id: new_bundle_id.clone(),
                recipe: None,
                project_content_preferences: Default::default(),
                pinned: request.pinned,
                created_at: now_secs(),
            });
        }
        Ok(())
    });
    if let Err(error) = config_result {
        let rollback = repository.mutate_bindings(|_, bindings| {
            bindings.bindings.extend(removed_bindings.clone());
            Ok(())
        });
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback_error) => format!(
                "{}; restoring the previous project binding also failed: {}",
                error, rollback_error
            ),
        });
    }

    if let Some(binding) = active_binding {
        save_project_binding_guarded(
            repository,
            SaveProjectBindingRequest {
                local_project_id: request.local_project_id.clone(),
                project_root: binding.project_root,
                profile_ids: binding.profile_ids,
                expected_revision: None,
            },
        )?;
    }
    get_project_with_repository(repository, &request.local_project_id)?
        .ok_or_else(|| "project disappeared after connecting remote storage".to_string())
}

fn save_project_binding_with_repository(
    repository: &V3Repository,
    request: SaveProjectBindingRequest,
) -> Result<ProjectBinding, String> {
    let _host_operation = repository.acquire_host_operation()?;
    save_project_binding_guarded(repository, request)
}

fn save_project_binding_guarded(
    repository: &V3Repository,
    request: SaveProjectBindingRequest,
) -> Result<ProjectBinding, String> {
    let validated = validate_binding_request(repository, &request)?;
    let host_id = repository.load_or_create_host_identity()?.host_id;
    let binding = repository.mutate_bindings(|config, bindings| {
        let project = config
            .project(&request.local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", request.local_project_id))?;
        let position = bindings
            .bindings
            .iter()
            .position(|binding| binding.local_project_id == request.local_project_id);
        let (replica_id, revision) = match position {
            Some(index) => {
                let current = &bindings.bindings[index];
                let expected = request.expected_revision.ok_or_else(|| {
                    "expected_revision is required when changing a binding".to_string()
                })?;
                if expected != current.revision {
                    return Err(format!(
                        "project binding changed (expected revision {}, current {})",
                        expected, current.revision
                    ));
                }
                // Profile paths are immutable in the catalog, so pinning the
                // profile IDs pins each agent home while still allowing a
                // checkout path remap.
                if current.state == BindingState::Active
                    && current.profile_ids != request.profile_ids
                {
                    return Err(AGENT_HOME_LOCKED_MESSAGE.to_string());
                }
                (
                    current.replica_id.clone(),
                    current.revision.saturating_add(1),
                )
            }
            None => {
                if request.expected_revision.is_some() {
                    return Err("new binding must not provide expected_revision".to_string());
                }
                (ReplicaId::parse(generated_named_id("replica")?)?, 0)
            }
        };
        let next = ProjectBinding {
            host_id: host_id.clone(),
            replica_id,
            local_project_id: request.local_project_id.clone(),
            bundle_id: project.bundle_id.clone(),
            project_root: request.project_root.clone(),
            canonical_project_root: validated.canonical_project_root.clone(),
            profile_ids: request.profile_ids.clone(),
            codex_home: validated.profile_paths.get(&Provider::Codex).cloned(),
            claude_home: validated.profile_paths.get(&Provider::Claude).cloned(),
            state: BindingState::Active,
            revision,
            updated_at: now_secs(),
        };
        next.validate_structure()?;
        match position {
            Some(index) => bindings.bindings[index] = next.clone(),
            None => bindings.bindings.push(next.clone()),
        }
        Ok(next)
    })?;

    // A remap never erases history, but materializations produced for an
    // older binding revision are detached and cannot satisfy a new plan.
    repository.mutate_materializations(|_, materializations| {
        for record in &mut materializations.records {
            if record.host_id == binding.host_id
                && record.replica_id == binding.replica_id
                && record.binding_revision != binding.revision
                && record.status != MaterializationStatus::Detached
            {
                record.status = MaterializationStatus::Detached;
            }
        }
        Ok(())
    })?;
    Ok(binding)
}

struct ValidatedBindingRequest {
    canonical_project_root: String,
    profile_paths: BTreeMap<Provider, String>,
}

/// First active binding (excluding `exclude`) that already claims one of the
/// requested provider profiles for this checkout. Project identity is
/// (canonical root, provider profile): the same folder with a different
/// config is a separate project, only a shared profile is a collision.
fn root_profile_collision<'a>(
    machine: &'a MachineProjectState,
    exclude: Option<&LocalProjectId>,
    canonical_root: &str,
    profile_ids: &BTreeMap<Provider, LocalProviderProfileId>,
) -> Option<(&'a ProjectBinding, Provider)> {
    let folded = canonical_root.to_lowercase();
    machine
        .bindings
        .iter()
        .filter(|binding| binding.state == BindingState::Active)
        .filter(|binding| exclude != Some(&binding.local_project_id))
        .filter(|binding| binding.canonical_project_root.to_lowercase() == folded)
        .find_map(|binding| {
            profile_ids.iter().find_map(|(provider, profile_id)| {
                binding
                    .profile_ids
                    .values()
                    .any(|used| used == profile_id)
                    .then_some((binding, *provider))
            })
        })
}

/// Visible label for collision messages: the machine-local alias when one
/// exists, else the shared display name, else the raw id.
fn project_label(config: &SyncConfigV3, id: &LocalProjectId) -> String {
    config
        .project(id)
        .map(|project| {
            project
                .local_alias
                .clone()
                .unwrap_or_else(|| project.display_name.clone())
        })
        .unwrap_or_else(|| id.to_string())
}

fn validate_binding_request(
    repository: &V3Repository,
    request: &SaveProjectBindingRequest,
) -> Result<ValidatedBindingRequest, String> {
    validate_absolute_clean_path("project root", &request.project_root)?;
    let profile_paths = resolve_profile_paths(repository, &request.profile_ids)?;
    let metadata = fs_metadata_no_final_symlink(Path::new(&request.project_root))?;
    if !metadata.is_dir() {
        return Err(format!(
            "project root '{}' is not a directory",
            request.project_root
        ));
    }
    let canonical = fs_canonicalize(Path::new(&request.project_root))?;
    let repository_root = prospective_canonical(repository.root())?;
    if paths_overlap(&canonical, &repository_root) {
        return Err(format!(
            "project root '{}' overlaps schema-3 app data '{}'",
            canonical.display(),
            repository_root.display()
        ));
    }
    let config = repository.load_config()?;
    let local_stores = config
        .storages
        .iter()
        .filter(|storage| storage.kind == super::domain::StorageKind::Local)
        .map(|storage| {
            prospective_canonical(Path::new(&storage.local_dir))
                .map(|path| (storage.name.as_str(), path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (storage_name, storage_path) in &local_stores {
        if paths_overlap(&canonical, &storage_path) {
            return Err(format!(
                "project root '{}' overlaps local storage '{}'",
                canonical.display(),
                storage_name
            ));
        }
    }
    let mut provider_roots: Vec<PathBuf> = Vec::new();
    for (provider, path) in &profile_paths {
        let label = format!("{} profile", provider_name(*provider));
        let path = prospective_canonical(Path::new(path))?;
        if paths_overlap(&path, &repository_root) {
            return Err(format!(
                "{} '{}' overlaps schema-3 app data",
                label,
                path.display()
            ));
        }
        if paths_overlap(&path, &canonical) {
            return Err(format!(
                "{} '{}' overlaps the project root",
                label,
                path.display()
            ));
        }
        for (storage_name, storage_path) in &local_stores {
            if paths_overlap(&path, storage_path) {
                return Err(format!(
                    "{} '{}' overlaps local storage '{}'",
                    label,
                    path.display(),
                    storage_name
                ));
            }
        }
        if provider_roots
            .iter()
            .any(|other| paths_overlap(&path, other))
        {
            return Err("Codex and Claude homes must not overlap".to_string());
        }
        provider_roots.push(path);
    }
    let machine = repository.load_bindings()?;
    if let Some((other, provider)) = root_profile_collision(
        &machine,
        Some(&request.local_project_id),
        &canonical.to_string_lossy(),
        &request.profile_ids,
    ) {
        return Err(format!(
            "'{}' already uses this {} config for this folder; pick a different config",
            project_label(&config, &other.local_project_id),
            provider_name(provider)
        ));
    }
    Ok(ValidatedBindingRequest {
        canonical_project_root: canonical.to_string_lossy().into_owned(),
        profile_paths,
    })
}

fn detach_project_binding_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    let _host_operation = repository.acquire_host_operation()?;
    detach_project_binding_guarded(repository, local_project_id)
}

fn detach_project_binding_guarded(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    let detached_scope = repository.mutate_bindings(|_, bindings| {
        let Some(binding) = bindings.bindings.iter_mut().find(|binding| {
            &binding.local_project_id == local_project_id && binding.state == BindingState::Active
        }) else {
            return Ok(None);
        };
        binding.state = BindingState::Detached;
        binding.revision = binding.revision.saturating_add(1);
        binding.updated_at = now_secs();
        Ok(Some((binding.host_id.clone(), binding.replica_id.clone())))
    })?;
    let Some((host_id, replica_id)) = detached_scope else {
        return Ok(false);
    };
    repository.mutate_materializations(|_, materializations| {
        for record in &mut materializations.records {
            if record.host_id == host_id && record.replica_id == replica_id {
                record.status = MaterializationStatus::Detached;
            }
        }
        Ok(())
    })?;
    Ok(true)
}

/// Sets or clears the machine-local alias.  The synced `display_name` stays
/// untouched so a rename here never propagates to the remote bundle or other
/// replicas.
fn rename_local_project_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
    local_alias: Option<String>,
    expected_revision: u64,
) -> Result<LocalProjectRegistration, String> {
    let alias = local_alias
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    repository.mutate_config(|config| {
        let project = config
            .projects
            .iter_mut()
            .find(|project| &project.local_project_id == local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        if project.revision != expected_revision {
            return Err(format!(
                "project changed (expected revision {}, current {})",
                expected_revision, project.revision
            ));
        }
        project.local_alias = alias;
        project.revision = project.revision.saturating_add(1);
        project.updated_at = now_secs();
        project.validate()?;
        Ok(project.clone())
    })
}

fn remove_local_project_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    let _host_operation = repository.acquire_host_operation()?;
    remove_local_project_guarded(repository, local_project_id)
}

fn remove_local_project_guarded(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    if repository
        .load_config()?
        .project(local_project_id)
        .is_none()
    {
        return Ok(false);
    }
    // Detach local application state before removing the registration.  A
    // crash between files leaves a valid unbound project, never an active
    // binding to an unknown project.
    let _ = detach_project_binding_guarded(repository, local_project_id)?;
    repository.mutate_materializations(|_, materializations| {
        for record in &mut materializations.records {
            if &record.local_project_id == local_project_id {
                record.status = MaterializationStatus::Detached;
            }
        }
        Ok(())
    })?;
    repository.mutate_config(|config| {
        config
            .projects
            .retain(|project| &project.local_project_id != local_project_id);
        config
            .links
            .retain(|link| &link.local_project_id != local_project_id);
        Ok(true)
    })
}

// ---------------------------------------------------------------------------
// Project setup drafts and transactional finalization
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct SetupDraftSummary {
    pub draft_id: SetupDraftId,
    pub display_name: String,
    pub project_root: String,
    pub updated_at: u64,
    pub revision: u64,
    /// `draft` when every referenced record still exists; `attention` when a
    /// referenced profile or storage disappeared since the draft was saved.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SetupDraftList {
    pub drafts: Vec<SetupDraftSummary>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CreateSetupDraftResult {
    pub draft: ProjectSetupDraft,
    /// True when an existing draft for the same canonical folder was resumed
    /// instead of creating a duplicate.
    pub resumed: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct SetupSectionStatus {
    pub section: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SetupDraftInspection {
    pub draft: ProjectSetupDraft,
    pub sections: Vec<SetupSectionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<ResourceInventory>,
    /// Signature of the fresh discovery; differs from the draft's stored
    /// signature when the discovered candidate set changed since selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fresh_discovery_signature: Option<String>,
    pub selection_stale: bool,
    pub can_finalize: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn list_setup_drafts(app: tauri::AppHandle) -> Result<SetupDraftList, String> {
    list_setup_drafts_with_repository(&repository(&app)?)
}

#[tauri::command]
pub async fn create_setup_draft(
    app: tauri::AppHandle,
    project_root: String,
) -> Result<CreateSetupDraftResult, String> {
    let repository = repository(&app)?;
    run_blocking(move || create_setup_draft_with_repository(&repository, &project_root)).await
}

#[tauri::command]
pub async fn get_setup_draft(
    app: tauri::AppHandle,
    draft_id: SetupDraftId,
) -> Result<Option<ProjectSetupDraft>, String> {
    repository(&app)?.load_setup_draft(&draft_id)
}

#[tauri::command]
pub async fn update_setup_draft(
    app: tauri::AppHandle,
    draft: ProjectSetupDraft,
) -> Result<ProjectSetupDraft, String> {
    let repository = repository(&app)?;
    run_blocking(move || update_setup_draft_with_repository(&repository, draft)).await
}

#[tauri::command]
pub async fn discard_setup_draft(
    app: tauri::AppHandle,
    draft_id: SetupDraftId,
) -> Result<bool, String> {
    discard_setup_draft_with_repository(&repository(&app)?, &draft_id)
}

#[tauri::command]
pub async fn inspect_setup_draft(
    app: tauri::AppHandle,
    draft_id: SetupDraftId,
) -> Result<SetupDraftInspection, String> {
    let repository = repository(&app)?;
    run_blocking(move || inspect_setup_draft_with_repository(&repository, &draft_id)).await
}

#[tauri::command]
pub async fn finalize_project_setup(
    app: tauri::AppHandle,
    draft_id: SetupDraftId,
    expected_revision: u64,
) -> Result<ProjectDetail, String> {
    let repository = repository(&app)?;
    let log = ActivityLogScope::new(ActivityLogType::Configuration);
    log.emit(
        &app,
        "info",
        "configuration.project_setup_started",
        "Finalizing project setup…",
    );
    let result = run_blocking(move || {
        finalize_project_setup_with_repository(&repository, &draft_id, expected_revision)
    })
    .await;
    match &result {
        Ok(detail) => log.emit(
            &app,
            "ok",
            "configuration.project_setup_completed",
            &format!("Project {} is set up", detail.project.display_name),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "configuration.project_setup_failed",
            &format!("Project setup failed: {}", error),
        ),
    }
    result
}

fn list_setup_drafts_with_repository(repository: &V3Repository) -> Result<SetupDraftList, String> {
    let (drafts, warnings) = repository.list_setup_drafts()?;
    let config = repository.load_config()?;
    let machine = repository.load_bindings()?;
    let summaries = drafts
        .into_iter()
        .map(|draft| {
            let status = if draft_references_are_present(&draft, &config, &machine) {
                "draft"
            } else {
                "attention"
            };
            SetupDraftSummary {
                status: status.to_string(),
                draft_id: draft.draft_id,
                display_name: draft.display_name,
                project_root: draft.project_root,
                updated_at: draft.updated_at,
                revision: draft.revision,
                last_error: draft.last_error,
            }
        })
        .collect();
    Ok(SetupDraftList {
        drafts: summaries,
        warnings,
    })
}

fn draft_references_are_present(
    draft: &ProjectSetupDraft,
    config: &SyncConfigV3,
    machine: &MachineProjectState,
) -> bool {
    let profiles_present = draft.profiles.values().all(|selection| match selection {
        DraftProfileSelection::Existing { profile_id } => machine
            .profiles
            .iter()
            .any(|profile| &profile.profile_id == profile_id),
        DraftProfileSelection::Pending { .. } => true,
    });
    let storage_present = match &draft.storage {
        Some(DraftStorageSelection::Existing { storage_id }) => config
            .storages
            .iter()
            .any(|storage| &storage.id == storage_id),
        _ => true,
    };
    profiles_present && storage_present
}

fn create_setup_draft_with_repository(
    repository: &V3Repository,
    project_root: &str,
) -> Result<CreateSetupDraftResult, String> {
    validate_absolute_clean_path("project root", project_root)?;
    let canonical = fs_canonicalize(Path::new(project_root))?;
    if !canonical.is_dir() {
        return Err(format!(
            "project root '{}' is not a directory",
            project_root
        ));
    }
    let repository_root = prospective_canonical(repository.root())?;
    if paths_overlap(&canonical, &repository_root) {
        return Err("project root overlaps schema-3 application data".to_string());
    }
    let canonical_text = canonical.to_string_lossy().into_owned();
    let (existing, _) = repository.list_setup_drafts()?;
    if let Some(found) = existing
        .into_iter()
        .find(|draft| draft.canonical_project_root == canonical_text)
    {
        return Ok(CreateSetupDraftResult {
            draft: found,
            resumed: true,
        });
    }

    ensure_default_provider_profiles(repository)?;
    let machine = repository.load_bindings()?;
    let config = repository.load_config()?;
    // Profiles already claimed by an active project on this exact folder are
    // taken: the composite project key (root, profile) reserves them, so a
    // second setup here must use a different config.
    let claimed_profiles: BTreeSet<&LocalProviderProfileId> = machine
        .bindings
        .iter()
        .filter(|binding| binding.state == BindingState::Active)
        .filter(|binding| {
            binding.canonical_project_root.to_lowercase() == canonical_text.to_lowercase()
        })
        .flat_map(|binding| binding.profile_ids.values())
        .collect();
    // Start setup with at most one agent enabled. Codex may use its single
    // unambiguous unclaimed profile as a convenience default; Claude remains
    // opt-in so a machine with both default homes does not scan both
    // automatically.
    let mut profiles = BTreeMap::new();
    let mut codex_candidates = machine
        .profiles
        .iter()
        .filter(|profile| profile.provider == Provider::Codex)
        .filter(|profile| !claimed_profiles.contains(&profile.profile_id));
    if let (Some(only), None) = (codex_candidates.next(), codex_candidates.next()) {
        profiles.insert(
            Provider::Codex,
            DraftProfileSelection::Existing {
                profile_id: only.profile_id.clone(),
            },
        );
    }
    let storage = if config.storages.len() == 1 {
        Some(DraftStorageSelection::Existing {
            storage_id: config.storages[0].id.clone(),
        })
    } else {
        None
    };
    let display_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Project")
        .to_string();
    let now = now_secs();
    let draft = ProjectSetupDraft {
        schema: SETUP_DRAFT_SCHEMA_V1,
        draft_id: SetupDraftId::parse(generated_named_id("draft")?)?,
        local_project_id: LocalProjectId::parse(generated_named_id("project")?)?,
        new_bundle_id: BundleId::generate()?,
        project_root: project_root.to_string(),
        canonical_project_root: canonical_text,
        display_name,
        repository_fingerprint: repository_fingerprint(&canonical),
        profiles,
        storage,
        repository: DraftRepositoryChoice::New,
        selected_resource_ids: Vec::new(),
        discovery_signature: String::new(),
        revision: 0,
        created_at: now,
        updated_at: now,
        last_error: None,
    };
    let saved = repository.save_setup_draft(draft)?;
    Ok(CreateSetupDraftResult {
        draft: saved,
        resumed: false,
    })
}

fn update_setup_draft_with_repository(
    repository: &V3Repository,
    submitted: ProjectSetupDraft,
) -> Result<ProjectSetupDraft, String> {
    let stored = repository
        .load_setup_draft(&submitted.draft_id)?
        .ok_or_else(|| format!("setup draft '{}' does not exist", submitted.draft_id))?;
    validate_absolute_clean_path("project root", &submitted.project_root)?;
    let canonical = fs_canonicalize(Path::new(&submitted.project_root))?;
    if !canonical.is_dir() {
        return Err(format!(
            "project root '{}' is not a directory",
            submitted.project_root
        ));
    }
    // Editable fields come from the client; identity, timestamps, and derived
    // path facts stay server-owned.
    let draft = ProjectSetupDraft {
        schema: SETUP_DRAFT_SCHEMA_V1,
        draft_id: stored.draft_id,
        local_project_id: stored.local_project_id,
        new_bundle_id: stored.new_bundle_id,
        project_root: submitted.project_root,
        canonical_project_root: canonical.to_string_lossy().into_owned(),
        display_name: submitted.display_name,
        repository_fingerprint: repository_fingerprint(&canonical),
        profiles: submitted.profiles,
        storage: submitted.storage,
        repository: submitted.repository,
        selected_resource_ids: submitted.selected_resource_ids,
        discovery_signature: submitted.discovery_signature,
        revision: submitted.revision,
        created_at: stored.created_at,
        updated_at: now_secs(),
        last_error: stored.last_error,
    };
    repository.save_setup_draft(draft)
}

fn discard_setup_draft_with_repository(
    repository: &V3Repository,
    draft_id: &SetupDraftId,
) -> Result<bool, String> {
    if repository.load_setup_transaction(draft_id)?.is_some() {
        return Err(
            "a finalization for this draft is still recovering; retry Finish setup first"
                .to_string(),
        );
    }
    repository.delete_setup_draft(draft_id)
}

/// Resolve each draft profile selection to the concrete provider home path,
/// plus the profile record to create when the selection is still pending.
struct ResolvedDraftProfiles {
    paths: BTreeMap<Provider, String>,
    profile_ids: BTreeMap<Provider, LocalProviderProfileId>,
    pending_records: Vec<ProviderProfile>,
}

fn resolve_draft_profiles(
    repository: &V3Repository,
    draft: &ProjectSetupDraft,
) -> Result<ResolvedDraftProfiles, String> {
    if draft.profiles.is_empty() {
        return Err("choose at least one local provider profile".to_string());
    }
    let machine = repository.load_bindings()?;
    let now = now_secs();
    let mut resolved = ResolvedDraftProfiles {
        paths: BTreeMap::new(),
        profile_ids: BTreeMap::new(),
        pending_records: Vec::new(),
    };
    for (provider, selection) in &draft.profiles {
        match selection {
            DraftProfileSelection::Existing { profile_id } => {
                let profile = machine
                    .profiles
                    .iter()
                    .find(|profile| &profile.profile_id == profile_id)
                    .ok_or_else(|| format!("unknown provider profile '{}'", profile_id))?;
                if &profile.provider != provider {
                    return Err(format!(
                        "{} cannot use {} profile '{}'",
                        provider_name(*provider),
                        provider_name(profile.provider),
                        profile.display_name
                    ));
                }
                let (available, readable, _, error) = inspect_provider_profile(profile);
                if !available || !readable {
                    return Err(error.unwrap_or_else(|| {
                        format!(
                            "{} profile '{}' is not readable",
                            provider_name(*provider),
                            profile.path
                        )
                    }));
                }
                resolved.paths.insert(*provider, profile.path.clone());
                resolved
                    .profile_ids
                    .insert(*provider, profile.profile_id.clone());
            }
            DraftProfileSelection::Pending { path, display_name } => {
                let probe = probe_provider_profile_with_repository(repository, *provider, path)?;
                if !probe.readable {
                    return Err(format!(
                        "{} profile '{}' is not readable",
                        provider_name(*provider),
                        probe.resolved_path
                    ));
                }
                if let Some(profile_id) = probe.existing_profile_id {
                    resolved.paths.insert(*provider, probe.resolved_path);
                    resolved.profile_ids.insert(*provider, profile_id);
                    continue;
                }
                let profile = ProviderProfile {
                    profile_id: LocalProviderProfileId::parse(generated_named_id("profile")?)?,
                    provider: *provider,
                    display_name: if display_name.trim().is_empty() {
                        probe.suggested_name
                    } else {
                        display_name.trim().to_string()
                    },
                    path: probe.resolved_path.clone(),
                    canonical_path: probe.canonical_path,
                    revision: 0,
                    created_at: now,
                    updated_at: now,
                };
                profile.validate_structure()?;
                resolved.paths.insert(*provider, probe.resolved_path);
                resolved
                    .profile_ids
                    .insert(*provider, profile.profile_id.clone());
                resolved.pending_records.push(profile);
            }
        }
    }
    Ok(resolved)
}

/// The storage a draft finalization will link, if any.
struct ResolvedDraftStorage {
    storage: StorageConfigV3,
    pending: bool,
}

fn resolve_draft_storage(
    repository: &V3Repository,
    draft: &ProjectSetupDraft,
) -> Result<Option<ResolvedDraftStorage>, String> {
    match &draft.storage {
        None => Ok(None),
        Some(DraftStorageSelection::Existing { storage_id }) => {
            let config = repository.load_config()?;
            let storage = config
                .storages
                .iter()
                .find(|storage| &storage.id == storage_id)
                .cloned()
                .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;
            Ok(Some(ResolvedDraftStorage {
                storage,
                pending: false,
            }))
        }
        Some(DraftStorageSelection::Pending { storage }) => {
            storage.validate()?;
            let config = repository.load_config()?;
            if config
                .storages
                .iter()
                .any(|existing| existing.id == storage.id)
            {
                // The preallocated ID landed in an earlier finalize attempt;
                // treat it as existing so retries stay idempotent.
                return Ok(Some(ResolvedDraftStorage {
                    storage: storage.clone(),
                    pending: false,
                }));
            }
            if storage.kind == StorageKind::Local && !Path::new(&storage.local_dir).is_dir() {
                return Err(format!(
                    "local storage folder '{}' does not exist",
                    storage.local_dir
                ));
            }
            Ok(Some(ResolvedDraftStorage {
                storage: storage.clone(),
                pending: true,
            }))
        }
    }
}

/// Only a matching pair of fingerprints verifies that the remote repo and
/// the local checkout describe the same Git repository.  A missing
/// fingerprint on either side is "unidentified", never a silent match.
fn verified_repository_match(remote: &Option<String>, local: &Option<String>) -> bool {
    matches!((remote, local), (Some(remote), Some(local)) if remote == local)
}

fn discovery_signature(inventory: &ResourceInventory) -> String {
    let mut ids: Vec<&str> = inventory
        .resources
        .iter()
        .map(|resource| resource.descriptor.resource_id.as_str())
        .collect();
    ids.sort_unstable();
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    hex_digest(&hasher.finalize())
}

fn inspect_setup_draft_with_repository(
    repository: &V3Repository,
    draft_id: &SetupDraftId,
) -> Result<SetupDraftInspection, String> {
    let draft = repository
        .load_setup_draft(draft_id)?
        .ok_or_else(|| format!("setup draft '{}' does not exist", draft_id))?;
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut blocked = false;

    // Project folder.
    let project_state = match fs_canonicalize(Path::new(&draft.project_root)) {
        Ok(canonical) if canonical.to_string_lossy() == draft.canonical_project_root => {
            SetupSectionStatus {
                section: "project".to_string(),
                state: "ready".to_string(),
                message: None,
            }
        }
        Ok(_) => SetupSectionStatus {
            section: "project".to_string(),
            state: "blocked".to_string(),
            message: Some(
                "The folder now resolves to a different location; choose it again.".to_string(),
            ),
        },
        Err(error) => SetupSectionStatus {
            section: "project".to_string(),
            state: "blocked".to_string(),
            message: Some(format!("Project folder is unavailable: {}", error)),
        },
    };
    blocked |= project_state.state == "blocked";
    sections.push(project_state);

    // Profiles, discovery, and the resource selection.
    let mut inventory = None;
    let mut fresh_signature = None;
    let mut selection_stale = false;
    match resolve_draft_profiles(repository, &draft) {
        Ok(resolved) => {
            sections.push(SetupSectionStatus {
                section: "profiles".to_string(),
                state: "ready".to_string(),
                message: None,
            });
            match discover_project_at(
                repository,
                &draft.project_root,
                &resolved.paths,
                &resolved.profile_ids,
            ) {
                Ok(discovery) => {
                    let signature = discovery_signature(&discovery.inventory);
                    selection_stale = !draft.discovery_signature.is_empty()
                        && draft.discovery_signature != signature;
                    let state = if selection_stale {
                        "attention"
                    } else {
                        "ready"
                    };
                    sections.push(SetupSectionStatus {
                        section: "resources".to_string(),
                        state: state.to_string(),
                        message: selection_stale.then(|| {
                            "Discovered resources changed since this selection was saved; review it."
                                .to_string()
                        }),
                    });
                    warnings.extend(discovery.warnings.clone());
                    fresh_signature = Some(signature);
                    inventory = Some(discovery.inventory);
                }
                Err(error) => {
                    blocked = true;
                    sections.push(SetupSectionStatus {
                        section: "resources".to_string(),
                        state: "blocked".to_string(),
                        message: Some(format!("Discovery failed: {}", error)),
                    });
                }
            }
        }
        Err(error) => {
            blocked = true;
            sections.push(SetupSectionStatus {
                section: "profiles".to_string(),
                state: "blocked".to_string(),
                message: Some(error),
            });
            sections.push(SetupSectionStatus {
                section: "resources".to_string(),
                state: "blocked".to_string(),
                message: Some(
                    "Resources are discovered once agent profiles are chosen.".to_string(),
                ),
            });
        }
    }

    // Storage.
    match resolve_draft_storage(repository, &draft) {
        Ok(Some(_)) => sections.push(SetupSectionStatus {
            section: "storage".to_string(),
            state: "ready".to_string(),
            message: None,
        }),
        Ok(None) => sections.push(SetupSectionStatus {
            section: "storage".to_string(),
            state: "attention".to_string(),
            message: Some(
                "No storage linked; the project will not sync until one is added.".to_string(),
            ),
        }),
        Err(error) => {
            blocked = true;
            sections.push(SetupSectionStatus {
                section: "storage".to_string(),
                state: "blocked".to_string(),
                message: Some(error),
            });
        }
    }

    // Repository choice.
    let repository_state = match &draft.repository {
        DraftRepositoryChoice::New => SetupSectionStatus {
            section: "repository".to_string(),
            state: "ready".to_string(),
            message: None,
        },
        DraftRepositoryChoice::Existing {
            storage_id,
            repository_fingerprint: remote_fingerprint,
            mismatch_acknowledged,
            ..
        } => {
            let selected_storage_id = match &draft.storage {
                Some(DraftStorageSelection::Existing { storage_id }) => Some(storage_id.clone()),
                Some(DraftStorageSelection::Pending { storage }) => Some(storage.id.clone()),
                None => None,
            };
            if selected_storage_id.as_ref() != Some(storage_id) {
                SetupSectionStatus {
                    section: "repository".to_string(),
                    state: "blocked".to_string(),
                    message: Some(
                        "The chosen remote repo lives in a different storage than the one selected."
                            .to_string(),
                    ),
                }
            } else if !verified_repository_match(remote_fingerprint, &draft.repository_fingerprint)
                && !mismatch_acknowledged
            {
                SetupSectionStatus {
                    section: "repository".to_string(),
                    state: "blocked".to_string(),
                    message: Some(
                        "The remote repo is not verified to match this folder's Git remote; acknowledge this to continue."
                            .to_string(),
                    ),
                }
            } else {
                SetupSectionStatus {
                    section: "repository".to_string(),
                    state: "ready".to_string(),
                    message: Some(
                        "The remote repo is revalidated against storage during Finish setup."
                            .to_string(),
                    ),
                }
            }
        }
    };
    blocked |= repository_state.state == "blocked";
    sections.push(repository_state);

    Ok(SetupDraftInspection {
        can_finalize: !blocked,
        draft,
        sections,
        inventory,
        fresh_discovery_signature: fresh_signature,
        selection_stale,
        warnings,
    })
}

fn build_setup_transaction(
    repository: &V3Repository,
    draft: &ProjectSetupDraft,
) -> Result<(SetupTransaction, Vec<String>), String> {
    let mut warnings = Vec::new();
    let host_id = repository.load_or_create_host_identity()?.host_id;
    let canonical = fs_canonicalize(Path::new(&draft.project_root))?;
    if canonical.to_string_lossy() != draft.canonical_project_root {
        return Err(
            "the project folder moved since this draft was saved; choose it again".to_string(),
        );
    }
    let resolved_profiles = resolve_draft_profiles(repository, draft)?;
    // Pending selections resolve to existing profile ids when their path is
    // already cataloged, so an id-level collision check covers both kinds.
    // Excluding the draft's own project keeps finalize retries idempotent.
    let machine = repository.load_bindings()?;
    if let Some((other, provider)) = root_profile_collision(
        &machine,
        Some(&draft.local_project_id),
        &draft.canonical_project_root,
        &resolved_profiles.profile_ids,
    ) {
        let config = repository.load_config()?;
        return Err(format!(
            "'{}' already syncs this folder with that {} config; open it instead or pick a different config",
            project_label(&config, &other.local_project_id),
            provider_name(provider)
        ));
    }
    let resolved_storage = resolve_draft_storage(repository, draft)?;
    let local_fingerprint = repository_fingerprint(&canonical);

    // Existing remote repos are revalidated against storage now, not at draft
    // time: the bundle must still exist and mismatches must be acknowledged.
    let (bundle_id, display_name, fingerprint, recipe) = match &draft.repository {
        DraftRepositoryChoice::New => {
            let discovery = discover_project_at(
                repository,
                &draft.project_root,
                &resolved_profiles.paths,
                &resolved_profiles.profile_ids,
            )?;
            let candidates: BTreeMap<&str, &InventoryResource> = discovery
                .inventory
                .resources
                .iter()
                .map(|resource| (resource.descriptor.resource_id.as_str(), resource))
                .collect();
            let mut recipe = BundleRecipe::default();
            for resource_id in &draft.selected_resource_ids {
                match candidates.get(resource_id.as_str()) {
                    Some(resource) if resource.blocked_reason.is_none() => {
                        recipe.entries.insert(
                            resource_id.clone(),
                            RecipeEntry {
                                resource_id: resource_id.clone(),
                                apply_policy: resource.descriptor.apply_policy,
                                required: false,
                            },
                        );
                    }
                    Some(resource) => warnings.push(format!(
                        "'{}' is blocked and was left out: {}",
                        resource.descriptor.display_name,
                        resource
                            .blocked_reason
                            .clone()
                            .unwrap_or_else(|| "blocked".to_string())
                    )),
                    None => warnings.push(format!(
                        "selected resource '{}' is not available right now and was left out",
                        resource_id
                    )),
                }
            }
            recipe.revision = 1;
            recipe.validate()?;
            (
                draft.new_bundle_id.clone(),
                draft.display_name.clone(),
                local_fingerprint,
                recipe,
            )
        }
        DraftRepositoryChoice::Existing {
            storage_id,
            bundle_id,
            mismatch_acknowledged,
            ..
        } => {
            let storage = resolved_storage
                .as_ref()
                .filter(|resolved| &resolved.storage.id == storage_id)
                .ok_or_else(|| {
                    "the chosen remote repo lives in a different storage than the one selected"
                        .to_string()
                })?;
            let engine = engine_for_storage_config(&storage.storage)?;
            let fetched = engine.fetch(bundle_id)?;
            let summary = bundle_snapshot_summary(fetched)?;
            if !verified_repository_match(&summary.repository_fingerprint, &local_fingerprint)
                && !mismatch_acknowledged
            {
                return Err(
                    "the remote repo is not verified to match this folder's Git remote; acknowledge the mismatch first"
                        .to_string(),
                );
            }
            (
                bundle_id.clone(),
                summary.display_name.clone(),
                summary.repository_fingerprint.clone().or(local_fingerprint),
                summary.recipe.clone(),
            )
        }
    };

    let now = now_secs();
    let project = LocalProjectRegistration {
        local_project_id: draft.local_project_id.clone(),
        bundle_id: bundle_id.clone(),
        display_name,
        local_alias: None,
        repository_fingerprint: fingerprint,
        recipe,
        recipe_bases: BTreeMap::new(),
        revision: 0,
        created_at: now,
        updated_at: now,
    };
    project.validate()?;

    let links = resolved_storage
        .as_ref()
        .map(|resolved| ProjectStorageLink {
            local_project_id: draft.local_project_id.clone(),
            storage_id: resolved.storage.id.clone(),
            bundle_id: bundle_id.clone(),
            recipe: None,
            project_content_preferences: Default::default(),
            pinned: true,
            created_at: now,
        })
        .into_iter()
        .collect();

    let binding = ProjectBinding {
        host_id: host_id.clone(),
        replica_id: ReplicaId::parse(generated_named_id("replica")?)?,
        local_project_id: draft.local_project_id.clone(),
        bundle_id,
        project_root: draft.project_root.clone(),
        canonical_project_root: draft.canonical_project_root.clone(),
        profile_ids: resolved_profiles.profile_ids.clone(),
        codex_home: None,
        claude_home: None,
        state: BindingState::Active,
        revision: 0,
        updated_at: now,
    };
    binding.validate_structure()?;

    let transaction = SetupTransaction {
        schema: SETUP_TRANSACTION_SCHEMA_V2,
        draft_id: draft.draft_id.clone(),
        draft_revision: draft.revision,
        created_at: now,
        host_id,
        replica_id: binding.replica_id.clone(),
        profiles: resolved_profiles.pending_records,
        storage: resolved_storage
            .filter(|resolved| resolved.pending)
            .map(|resolved| resolved.storage),
        project,
        links,
        binding,
    };
    transaction.validate()?;

    // Prove prospective isolation before anything is written: the new records
    // must not overlap application data, local storages, or each other.
    let mut prospective_config = repository.load_config()?;
    if let Some(storage) = &transaction.storage {
        prospective_config.storages.push(storage.clone());
    }
    let machine = repository.load_bindings()?;
    let mut prospective_profiles = machine.profiles.clone();
    prospective_profiles.extend(transaction.profiles.iter().cloned());
    let mut prospective_bindings = machine.bindings.clone();
    prospective_bindings.push(transaction.binding.clone());
    validate_config_storage_isolation(
        repository,
        &prospective_config,
        &prospective_bindings,
        &prospective_profiles,
    )?;
    Ok((transaction, warnings))
}

/// Apply a setup transaction's records in dependency order, skipping records
/// that already exist so every retry reconciles instead of duplicating.
fn apply_setup_transaction(
    repository: &V3Repository,
    transaction: &SetupTransaction,
) -> Result<(), String> {
    let _host_operation = repository.acquire_host_operation()?;
    transaction.validate()?;
    let identity = repository.load_or_create_host_identity()?;
    if transaction.host_id != identity.host_id {
        return Err(
            "setup transaction belongs to a different host; review the draft again".to_string(),
        );
    }

    // 1. Provider profiles (machine state; no dependency on config).
    let profile_id_map = repository.mutate_bindings(|_, machine| {
        let mut map: BTreeMap<LocalProviderProfileId, LocalProviderProfileId> = BTreeMap::new();
        for profile in &transaction.profiles {
            if machine
                .profiles
                .iter()
                .any(|existing| existing.profile_id == profile.profile_id)
            {
                continue;
            }
            if let Some(existing) = machine.profiles.iter().find(|existing| {
                existing.provider == profile.provider
                    && existing.canonical_path == profile.canonical_path
            }) {
                // Someone created the same profile concurrently; reconcile to
                // the surviving record instead of failing on overlap.
                map.insert(profile.profile_id.clone(), existing.profile_id.clone());
                continue;
            }
            machine.profiles.push(profile.clone());
        }
        Ok(map)
    })?;

    // Alias qualifier: the binding's Codex (else Claude) config name, so a
    // second project on one checkout defaults to "repo · config (host)".
    let machine_profiles = repository.load_bindings()?.profiles;
    let alias_qualifier = [Provider::Codex, Provider::Claude]
        .iter()
        .find_map(|provider| transaction.binding.profile_ids.get(provider))
        .and_then(|profile_id| {
            let profile_id = profile_id_map.get(profile_id).unwrap_or(profile_id);
            machine_profiles
                .iter()
                .find(|profile| &profile.profile_id == profile_id)
                .map(|profile| profile.display_name.clone())
        });

    // 2. Storage, project, and links (one atomic config mutation).
    repository.mutate_config(|config| {
        if let Some(storage) = &transaction.storage {
            if !config
                .storages
                .iter()
                .any(|existing| existing.id == storage.id)
            {
                config.storages.push(storage.clone());
            }
        }
        if config
            .project(&transaction.project.local_project_id)
            .is_none()
        {
            let mut project = transaction.project.clone();
            if project.local_alias.is_none() {
                project.local_alias = default_local_alias(
                    &project.display_name,
                    alias_qualifier.as_deref(),
                    &config.projects,
                );
            }
            config.projects.push(project);
        }
        for link in &transaction.links {
            if !config.links.iter().any(|existing| {
                existing.local_project_id == link.local_project_id
                    && existing.storage_id == link.storage_id
            }) {
                config.links.push(link.clone());
            }
        }
        Ok(())
    })?;

    // 3. Machine binding (requires the project to exist in config).
    let mut expected_binding = transaction.binding.clone();
    for profile_id in expected_binding.profile_ids.values_mut() {
        if let Some(mapped) = profile_id_map.get(profile_id) {
            *profile_id = mapped.clone();
        }
    }
    repository.mutate_bindings(|_, machine| {
        if let Some(existing) = machine
            .bindings
            .iter()
            .find(|binding| binding.local_project_id == expected_binding.local_project_id)
        {
            if existing == &expected_binding {
                return Ok(());
            }
            return Err(
                "setup transaction conflicts with a different host or project replica".to_string(),
            );
        }
        machine.bindings.push(expected_binding.clone());
        Ok(())
    })?;

    // Confirm both documents contain the expected records before the caller
    // removes the transaction.
    let config = repository.load_config()?;
    if config
        .project(&transaction.project.local_project_id)
        .is_none()
    {
        return Err("project registration did not persist".to_string());
    }
    let persisted_binding = repository
        .load_bindings()?
        .bindings
        .into_iter()
        .find(|binding| binding.local_project_id == transaction.project.local_project_id);
    if persisted_binding.as_ref() != Some(&expected_binding) {
        return Err("project binding did not persist".to_string());
    }
    Ok(())
}

/// True when any of the transaction's records already landed in a document.
fn setup_transaction_partially_applied(
    repository: &V3Repository,
    transaction: &SetupTransaction,
) -> Result<bool, String> {
    let config = repository.load_config()?;
    if config
        .project(&transaction.project.local_project_id)
        .is_some()
    {
        return Ok(true);
    }
    if let Some(storage) = &transaction.storage {
        if config
            .storages
            .iter()
            .any(|existing| existing.id == storage.id)
        {
            return Ok(true);
        }
    }
    let machine = repository.load_bindings()?;
    if machine
        .bindings
        .iter()
        .any(|binding| binding.local_project_id == transaction.project.local_project_id)
    {
        return Ok(true);
    }
    if transaction.profiles.iter().any(|profile| {
        machine
            .profiles
            .iter()
            .any(|existing| existing.profile_id == profile.profile_id)
    }) {
        return Ok(true);
    }
    Ok(false)
}

/// Complete or clean up interrupted finalizations.  Runs before the shell's
/// first project listing and before every finalize, so a crash between the
/// config and binding writes heals on the next app start.
pub(crate) fn recover_setup_state(repository: &V3Repository) -> Vec<String> {
    let (transactions, mut warnings) = match repository.list_setup_transactions() {
        Ok(listed) => listed,
        Err(error) => return vec![format!("setup recovery unavailable: {}", error)],
    };
    for transaction in transactions {
        match repository.load_or_create_host_identity() {
            Ok(identity) if identity.host_id != transaction.host_id => {
                let project_exists = repository.load_config().ok().is_some_and(|config| {
                    config
                        .project(&transaction.project.local_project_id)
                        .is_some()
                });
                if let Err(error) = repository.delete_setup_transaction(&transaction.draft_id) {
                    warnings.push(error);
                    continue;
                }
                if project_exists {
                    if let Err(error) = repository.delete_setup_draft(&transaction.draft_id) {
                        warnings.push(error);
                    }
                    warnings.push(format!(
                        "setup for draft '{}' was retained as a detached project after host registration; review its local binding",
                        transaction.draft_id
                    ));
                } else {
                    let message =
                        "host identity changed during setup; review the draft and finish again";
                    record_draft_error(repository, &transaction.draft_id, message);
                    warnings.push(format!(
                        "setup for draft '{}' must be reviewed again because the host identity changed",
                        transaction.draft_id
                    ));
                }
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                warnings.push(format!(
                    "setup for draft '{}' cannot verify host identity: {}",
                    transaction.draft_id, error
                ));
                continue;
            }
        }
        match apply_setup_transaction(repository, &transaction) {
            Ok(()) => {
                if let Err(error) = repository.delete_setup_transaction(&transaction.draft_id) {
                    warnings.push(error);
                    continue;
                }
                if let Err(error) = repository.delete_setup_draft(&transaction.draft_id) {
                    warnings.push(error);
                }
            }
            Err(error) => match setup_transaction_partially_applied(repository, &transaction) {
                // Nothing landed; return to the draft state and keep the
                // reason on the draft instead of wedging future finalizes.
                Ok(false) => {
                    if let Err(delete_error) =
                        repository.delete_setup_transaction(&transaction.draft_id)
                    {
                        warnings.push(delete_error);
                    }
                    record_draft_error(repository, &transaction.draft_id, &error);
                    warnings.push(format!(
                        "setup for draft '{}' was rolled back: {}",
                        transaction.draft_id, error
                    ));
                }
                _ => warnings.push(format!(
                    "setup for draft '{}' is incomplete and will retry: {}",
                    transaction.draft_id, error
                )),
            },
        }
    }
    warnings
}

fn record_draft_error(repository: &V3Repository, draft_id: &SetupDraftId, error: &str) {
    let Ok(Some(mut draft)) = repository.load_setup_draft(draft_id) else {
        return;
    };
    let mut message: String = error.chars().filter(|c| !c.is_control()).collect();
    message.truncate(4_000);
    draft.last_error = Some(message);
    let _ = repository.save_setup_draft(draft);
}

fn finalize_project_setup_with_repository(
    repository: &V3Repository,
    draft_id: &SetupDraftId,
    expected_revision: u64,
) -> Result<ProjectDetail, String> {
    // Complete anything interrupted first; this may finish this very draft.
    let recovery_warnings = recover_setup_state(repository);
    let Some(draft) = repository.load_setup_draft(draft_id)? else {
        return Err(format!(
            "setup draft '{}' does not exist (it may have just finished; refresh the project list)",
            draft_id
        ));
    };
    if let Some(warning) = recovery_warnings
        .iter()
        .find(|warning| warning.contains(draft_id.as_str()))
    {
        return Err(warning.clone());
    }
    if draft.revision != expected_revision {
        return Err(format!(
            "setup draft changed (expected revision {}, current {})",
            expected_revision, draft.revision
        ));
    }

    let result = (|| {
        let (transaction, _warnings) = build_setup_transaction(repository, &draft)?;
        repository.save_setup_transaction(&transaction)?;
        apply_setup_transaction(repository, &transaction).map(|()| transaction)
    })();
    let transaction = match result {
        Ok(transaction) => transaction,
        Err(error) => {
            record_draft_error(repository, draft_id, &error);
            return Err(error);
        }
    };
    repository.delete_setup_transaction(draft_id)?;
    repository.delete_setup_draft(draft_id)?;
    get_project_with_repository(repository, &transaction.project.local_project_id)?
        .ok_or_else(|| "project disappeared after setup".to_string())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn fs_metadata_no_final_symlink(path: &Path) -> Result<std::fs::Metadata, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect '{}': {}", path.display(), error))?;
    if metadata.file_type().is_symlink() {
        // Binding spelling may itself be a symlink because provider cwd
        // identity uses the spelling.  Follow it only after proving the final
        // component is a link; canonical containment is stored separately.
        std::fs::metadata(path).map_err(|error| format!("follow '{}': {}", path.display(), error))
    } else {
        Ok(metadata)
    }
}

fn fs_profile_metadata(path: &Path) -> Result<std::fs::Metadata, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect provider profile '{}': {}", path.display(), error))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "provider profile '{}' cannot be a symlink; choose the resolved directory",
            path.display()
        ));
    }
    Ok(metadata)
}

fn fs_canonicalize(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|error| format!("resolve '{}': {}", path.display(), error))
}

fn audit_codex_conversation_paths_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<CodexConversationPathAudit, String> {
    let stored_binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    let binding = resolve_project_binding(repository, &stored_binding)?;
    let profile_id = binding
        .profile_ids
        .get(&Provider::Codex)
        .map(ToString::to_string);
    let mut audit = CodexConversationPathAudit {
        local_project_id: local_project_id.clone(),
        profile_id,
        profile_path: binding.codex_home.clone(),
        project_root: binding.project_root.clone(),
        assigned_thread_count: 0,
        matching_thread_count: 0,
        issues: Vec::new(),
        blockers: Vec::new(),
        warnings: Vec::new(),
        ready: true,
        can_repair: false,
    };
    let Some(codex_home_text) = binding.codex_home.as_deref() else {
        return Ok(audit);
    };
    let codex_home = match fs_canonicalize(Path::new(codex_home_text)) {
        Ok(path) => path,
        Err(error) => {
            audit.blockers.push(error);
            return Ok(finalize_conversation_path_audit(audit));
        }
    };
    let canonical_project_root = PathBuf::from(&binding.canonical_project_root);
    let state_path = codex_home.join(".codex-global-state.json");
    if !state_path.exists() {
        audit.warnings.push(format!(
            "Codex Desktop project assignments were not found in '{}'",
            state_path.display()
        ));
        return Ok(finalize_conversation_path_audit(audit));
    }
    let state_metadata = match fs::symlink_metadata(&state_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            audit.blockers.push(format!(
                "inspect Codex Desktop project assignments '{}': {}",
                state_path.display(),
                error
            ));
            return Ok(finalize_conversation_path_audit(audit));
        }
    };
    if !state_metadata.is_file() || state_metadata.file_type().is_symlink() {
        audit.blockers.push(format!(
            "Codex Desktop project assignments '{}' must be a regular file",
            state_path.display()
        ));
        return Ok(finalize_conversation_path_audit(audit));
    }
    if state_metadata.len() > MAX_CODEX_GLOBAL_STATE_BYTES {
        audit.blockers.push(format!(
            "Codex Desktop project assignments '{}' exceed the {} byte safety limit",
            state_path.display(),
            MAX_CODEX_GLOBAL_STATE_BYTES
        ));
        return Ok(finalize_conversation_path_audit(audit));
    }
    let state_bytes = match fs::read(&state_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            audit.blockers.push(format!(
                "read Codex Desktop project assignments '{}': {}",
                state_path.display(),
                error
            ));
            return Ok(finalize_conversation_path_audit(audit));
        }
    };
    let state: serde_json::Value = match serde_json::from_slice(&state_bytes) {
        Ok(state) => state,
        Err(error) => {
            audit.blockers.push(format!(
                "parse Codex Desktop project assignments '{}': {}",
                state_path.display(),
                error
            ));
            return Ok(finalize_conversation_path_audit(audit));
        }
    };

    let mut desktop_project_ids = BTreeSet::new();
    for projects in state_object_maps(&state, "local-projects") {
        for (project_key, project) in projects {
            let owns_root = json_path_values(project, &["rootPaths", "root_paths"])
                .into_iter()
                .any(|root| path_belongs_to_project(Path::new(root), &canonical_project_root));
            if !owns_root {
                continue;
            }
            desktop_project_ids.insert(project_key.clone());
            if let Some(project_id) = project.get("id").and_then(serde_json::Value::as_str) {
                desktop_project_ids.insert(project_id.to_string());
            }
        }
    }

    let mut assigned_thread_ids = BTreeSet::new();
    for assignments in state_object_maps(&state, "thread-project-assignments") {
        for (thread_id, assignment) in assignments {
            let local_assignment = assignment
                .get("projectKind")
                .or_else(|| assignment.get("project_kind"))
                .and_then(serde_json::Value::as_str)
                .map(|kind| kind == "local")
                .unwrap_or(true);
            let assigned_project = assignment
                .get("projectId")
                .or_else(|| assignment.get("project_id"))
                .and_then(serde_json::Value::as_str);
            let legacy_cwd_matches = assigned_project.is_none()
                && json_path_values(assignment, &["cwd"])
                    .into_iter()
                    .any(|cwd| path_belongs_to_project(Path::new(cwd), &canonical_project_root));
            if local_assignment
                && (assigned_project
                    .map(|project_id| desktop_project_ids.contains(project_id))
                    .unwrap_or(false)
                    || legacy_cwd_matches)
            {
                insert_safe_assigned_thread(
                    thread_id,
                    &mut assigned_thread_ids,
                    &mut audit.blockers,
                );
            }
        }
    }
    for writable_roots in state_object_maps(&state, "thread-writable-roots") {
        for (thread_id, roots) in writable_roots {
            if json_path_values(roots, &["roots", "rootPaths", "root_paths"])
                .into_iter()
                .chain(json_direct_path_values(roots))
                .any(|root| path_belongs_to_project(Path::new(root), &canonical_project_root))
            {
                insert_safe_assigned_thread(
                    thread_id,
                    &mut assigned_thread_ids,
                    &mut audit.blockers,
                );
            }
        }
    }
    for root_hints in state_object_maps(&state, "thread-workspace-root-hints") {
        for (thread_id, hint) in root_hints {
            if json_direct_path_values(hint)
                .any(|root| path_belongs_to_project(Path::new(root), &canonical_project_root))
            {
                insert_safe_assigned_thread(
                    thread_id,
                    &mut assigned_thread_ids,
                    &mut audit.blockers,
                );
            }
        }
    }
    audit.assigned_thread_count = assigned_thread_ids.len();
    if assigned_thread_ids.is_empty() {
        return Ok(finalize_conversation_path_audit(audit));
    }

    let mut transcript_paths: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut visited_files = 0_usize;
    let mut limit_reached = false;
    for directory in ["sessions", "archived_sessions"] {
        let session_root = codex_home.join(directory);
        if !session_root.exists() {
            continue;
        }
        let root_metadata = match fs::symlink_metadata(&session_root) {
            Ok(metadata) => metadata,
            Err(error) => {
                audit.blockers.push(format!(
                    "inspect Codex {} directory '{}': {}",
                    directory,
                    session_root.display(),
                    error
                ));
                continue;
            }
        };
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            audit.blockers.push(format!(
                "Codex {} directory '{}' must be a real directory",
                directory,
                session_root.display()
            ));
            continue;
        }
        for entry in WalkDir::new(&session_root).follow_links(false).max_depth(8) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    audit
                        .blockers
                        .push(format!("walk Codex {} sessions: {}", directory, error));
                    continue;
                }
            };
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("jsonl")
            {
                continue;
            }
            visited_files = visited_files.saturating_add(1);
            if visited_files > MAX_CODEX_SESSION_FILES {
                audit.blockers.push(format!(
                    "Codex profile contains more than {} session files; repair requires manual review",
                    MAX_CODEX_SESSION_FILES
                ));
                limit_reached = true;
                break;
            }
            let filename = entry.file_name().to_string_lossy();
            let filename_matches_assignment = assigned_thread_ids
                .iter()
                .any(|thread_id| filename.contains(thread_id));
            let metadata =
                match provider_capture::scan_jsonl_metadata(entry.path(), CaptureProvider::Codex) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        if filename_matches_assignment {
                            audit.blockers.push(format!(
                                "inspect assigned Codex conversation '{}': {}",
                                entry.path().display(),
                                error
                            ));
                        }
                        continue;
                    }
                };
            if !assigned_thread_ids.contains(&metadata.session_id) {
                continue;
            }
            let canonical_path = match fs_canonicalize(entry.path()) {
                Ok(path) => path,
                Err(error) => {
                    audit.blockers.push(error);
                    continue;
                }
            };
            if !canonical_path.starts_with(&codex_home) {
                audit.blockers.push(format!(
                    "assigned Codex conversation '{}' resolves outside its profile",
                    entry.path().display()
                ));
                continue;
            }
            transcript_paths
                .entry(metadata.session_id)
                .or_default()
                .push(canonical_path);
        }
        if limit_reached {
            break;
        }
    }

    for thread_id in assigned_thread_ids {
        let Some(paths) = transcript_paths.get(&thread_id) else {
            audit.warnings.push(format!(
                "Codex Desktop assigns thread '{}' to this project, but its rollout file was not found",
                thread_id
            ));
            continue;
        };
        if paths.len() != 1 {
            audit.blockers.push(format!(
                "Codex thread '{}' has {} rollout files; repair requires manual review",
                thread_id,
                paths.len()
            ));
            continue;
        }
        let path = &paths[0];
        let source = match read_bounded_conversation(path) {
            Ok(source) => source,
            Err(error) => {
                audit.blockers.push(error);
                continue;
            }
        };
        let stale_cwds =
            match inspect_codex_structural_cwds(&source, &thread_id, &canonical_project_root) {
                Ok(cwds) => cwds,
                Err(error) => {
                    audit.blockers.push(format!(
                        "inspect assigned Codex conversation '{}': {}",
                        path.display(),
                        error
                    ));
                    continue;
                }
            };
        if let Some(recorded_cwd) = stale_cwds.into_iter().next() {
            let Some(transcript_path) = path.to_str() else {
                audit.blockers.push(format!(
                    "assigned Codex conversation path '{}' is not valid UTF-8",
                    path.display()
                ));
                continue;
            };
            audit.issues.push(CodexConversationPathIssue {
                thread_id,
                transcript_path: transcript_path.to_string(),
                recorded_cwd,
                target_cwd: binding.project_root.clone(),
            });
        } else {
            audit.matching_thread_count = audit.matching_thread_count.saturating_add(1);
        }
    }

    Ok(finalize_conversation_path_audit(audit))
}

fn repair_codex_conversation_paths_with_repository(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<CodexConversationPathRepairResult, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let audit = audit_codex_conversation_paths_with_repository(repository, local_project_id)?;
    if !audit.blockers.is_empty() {
        return Err(format!(
            "conversation paths cannot be repaired safely: {}",
            audit.blockers.join("; ")
        ));
    }
    if audit.issues.is_empty() {
        return Ok(CodexConversationPathRepairResult {
            audit,
            repaired_thread_ids: Vec::new(),
            backup_dir: None,
        });
    }

    let stored_binding = repository
        .load_bindings()?
        .active_for(local_project_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "project '{}' is not mapped on this machine",
                local_project_id
            )
        })?;
    let binding = resolve_project_binding(repository, &stored_binding)?;
    let codex_home = binding
        .codex_home
        .as_deref()
        .ok_or_else(|| "this project has no Codex profile to repair".to_string())?;
    let canonical_codex_home = fs_canonicalize(Path::new(codex_home))?;
    let sessions_root = canonical_codex_home.join("sessions");
    let archived_root = canonical_codex_home.join("archived_sessions");

    struct PendingRepair {
        thread_id: String,
        path: PathBuf,
        original: Vec<u8>,
        repaired: Vec<u8>,
        mode: u32,
    }

    let mut pending = Vec::with_capacity(audit.issues.len());
    let mut staged_source_bytes = 0_u64;
    for issue in &audit.issues {
        let path = PathBuf::from(&issue.transcript_path);
        let canonical_path = fs_canonicalize(&path)?;
        if !canonical_path.starts_with(&sessions_root)
            && !canonical_path.starts_with(&archived_root)
        {
            return Err(format!(
                "refusing to repair '{}' outside the bound Codex profile",
                path.display()
            ));
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect '{}': {}", path.display(), error))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "refusing to repair non-regular conversation file '{}'",
                path.display()
            ));
        }
        let original = read_bounded_conversation(&path)?;
        staged_source_bytes = staged_source_bytes.saturating_add(original.len() as u64);
        if staged_source_bytes > MAX_CONVERSATION_REPAIR_BYTES {
            return Err(format!(
                "assigned Codex conversations exceed the {} byte aggregate repair limit",
                MAX_CONVERSATION_REPAIR_BYTES
            ));
        }
        let stale = inspect_codex_structural_cwds(
            &original,
            &issue.thread_id,
            Path::new(&binding.canonical_project_root),
        )?;
        if stale.is_empty() {
            return Err(format!(
                "Codex conversation '{}' changed after the audit; refresh and try again",
                issue.thread_id
            ));
        }
        let repaired =
            remap_codex_structural_cwds(&original, &issue.thread_id, &binding.project_root)?;
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = 0o600;
        pending.push(PendingRepair {
            thread_id: issue.thread_id.clone(),
            path,
            original,
            repaired,
            mode,
        });
    }

    let backup_root = repository.backups_dir()?;
    let backup_dir = backup_root.join(generated_named_id("pathrepair")?);
    fs::create_dir(&backup_dir).map_err(|error| {
        format!(
            "create conversation path backup '{}': {}",
            backup_dir.display(),
            error
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "secure conversation path backup '{}': {}",
                backup_dir.display(),
                error
            )
        })?;
    }
    for (index, repair) in pending.iter().enumerate() {
        let backup_path = backup_dir.join(format!("{:04}-{}.jsonl", index + 1, repair.thread_id));
        write_immutable_backup(&backup_path, &repair.original)?;
    }
    for repair in &pending {
        let current = read_bounded_conversation(&repair.path)?;
        if current != repair.original {
            return Err(format!(
                "Codex conversation '{}' changed while its backup was being prepared; no files were modified",
                repair.thread_id
            ));
        }
    }

    let rollback = |count: usize| {
        pending[..count]
            .iter()
            .filter_map(|completed| {
                write_target_atomic(&completed.path, &completed.original, completed.mode)
                    .err()
                    .map(|error| format!("{}: {}", completed.thread_id, error))
            })
            .collect::<Vec<_>>()
    };
    let mut applied = 0_usize;
    for repair in &pending {
        let current = match read_bounded_conversation(&repair.path) {
            Ok(current) => current,
            Err(error) => {
                let rollback_errors = rollback(applied);
                return Err(if rollback_errors.is_empty() {
                    format!(
                        "repair stopped before '{}': {}; earlier changes were rolled back",
                        repair.thread_id, error
                    )
                } else {
                    format!(
                        "repair stopped before '{}': {}; rollback also failed for {}",
                        repair.thread_id,
                        error,
                        rollback_errors.join("; ")
                    )
                });
            }
        };
        if current != repair.original {
            let rollback_errors = rollback(applied);
            return Err(if rollback_errors.is_empty() {
                format!(
                    "Codex conversation '{}' changed during repair; earlier changes were rolled back",
                    repair.thread_id
                )
            } else {
                format!(
                    "Codex conversation '{}' changed during repair and rollback failed for {}",
                    repair.thread_id,
                    rollback_errors.join("; ")
                )
            });
        }
        if let Err(error) = write_target_atomic(&repair.path, &repair.repaired, repair.mode) {
            let rollback_errors = rollback(applied);
            return Err(if rollback_errors.is_empty() {
                format!(
                    "repair failed for '{}': {}; earlier changes were rolled back",
                    repair.thread_id, error
                )
            } else {
                format!(
                    "repair failed for '{}': {}; rollback also failed for {}",
                    repair.thread_id,
                    error,
                    rollback_errors.join("; ")
                )
            });
        }
        applied = applied.saturating_add(1);
    }

    let repaired_thread_ids = pending
        .iter()
        .map(|repair| repair.thread_id.clone())
        .collect::<Vec<_>>();
    let final_audit =
        match audit_codex_conversation_paths_with_repository(repository, local_project_id) {
            Ok(audit) => audit,
            Err(error) => {
                let rollback_errors = rollback(pending.len());
                return Err(if rollback_errors.is_empty() {
                    format!(
                        "conversation path verification failed: {}; changes were rolled back",
                        error
                    )
                } else {
                    format!(
                        "conversation path verification failed: {}; rollback also failed for {}",
                        error,
                        rollback_errors.join("; ")
                    )
                });
            }
        };
    if !final_audit.ready {
        let rollback_errors = rollback(pending.len());
        return Err(if rollback_errors.is_empty() {
            "conversation paths did not pass verification; changes were rolled back".to_string()
        } else {
            format!(
                "conversation paths did not pass verification and rollback failed for {}",
                rollback_errors.join("; ")
            )
        });
    }
    Ok(CodexConversationPathRepairResult {
        audit: final_audit,
        repaired_thread_ids,
        backup_dir: backup_dir.to_str().map(ToString::to_string),
    })
}

fn require_codex_conversation_paths_ready(
    repository: &V3Repository,
    local_project_id: &LocalProjectId,
) -> Result<(), String> {
    let audit = audit_codex_conversation_paths_with_repository(repository, local_project_id)?;
    if audit.ready {
        return Ok(());
    }
    if !audit.blockers.is_empty() {
        return Err(format!(
            "Push and Pull are paused because Codex conversation ownership could not be verified: {}",
            audit.blockers.join("; ")
        ));
    }
    Err(format!(
        "{} Codex conversation path{} must be repaired before Push or Pull",
        audit.issues.len(),
        if audit.issues.len() == 1 { "" } else { "s" }
    ))
}

fn finalize_conversation_path_audit(
    mut audit: CodexConversationPathAudit,
) -> CodexConversationPathAudit {
    audit
        .issues
        .sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    audit.blockers.sort();
    audit.blockers.dedup();
    audit.warnings.sort();
    audit.warnings.dedup();
    audit.ready = audit.issues.is_empty() && audit.blockers.is_empty();
    audit.can_repair = !audit.issues.is_empty() && audit.blockers.is_empty();
    audit
}

fn state_object_maps<'a>(
    state: &'a serde_json::Value,
    key: &str,
) -> Vec<&'a serde_json::Map<String, serde_json::Value>> {
    let mut maps = Vec::new();
    if let Some(map) = state.get(key).and_then(serde_json::Value::as_object) {
        maps.push(map);
    }
    if let Some(map) = state
        .get("electron-persisted-atom-state")
        .and_then(|persisted| persisted.get(key))
        .and_then(serde_json::Value::as_object)
    {
        maps.push(map);
    }
    maps
}

fn json_path_values<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Vec<&'a str> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    keys.iter()
        .filter_map(|key| object.get(*key))
        .flat_map(json_direct_path_values)
        .collect()
}

fn json_direct_path_values(value: &serde_json::Value) -> impl Iterator<Item = &str> {
    let direct = value.as_str().into_iter();
    let array = value
        .as_array()
        .into_iter()
        .flat_map(|values| values.iter().filter_map(serde_json::Value::as_str));
    direct.chain(array)
}

fn path_belongs_to_project(candidate: &Path, project_root: &Path) -> bool {
    let Some(candidate_text) = candidate.to_str() else {
        return false;
    };
    if validate_absolute_clean_path("Codex project path", candidate_text).is_err() {
        return false;
    }
    let candidate = fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    let project_root =
        fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    candidate == project_root || candidate.starts_with(&project_root)
}

fn insert_safe_assigned_thread(
    thread_id: &str,
    assigned: &mut BTreeSet<String>,
    blockers: &mut Vec<String>,
) {
    if thread_id.is_empty()
        || thread_id.len() > 128
        || !thread_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        blockers.push("Codex Desktop contains an unsafe assigned thread ID".to_string());
        return;
    }
    assigned.insert(thread_id.to_string());
}

fn read_bounded_conversation(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect '{}': {}", path.display(), error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("'{}' is not a regular file", path.display()));
    }
    if metadata.len() > MAX_CONVERSATION_REPAIR_BYTES {
        return Err(format!(
            "Codex conversation '{}' exceeds the {} byte repair limit",
            path.display(),
            MAX_CONVERSATION_REPAIR_BYTES
        ));
    }
    fs::read(path).map_err(|error| format!("read '{}': {}", path.display(), error))
}

fn inspect_codex_structural_cwds(
    source: &[u8],
    expected_thread_id: &str,
    project_root: &Path,
) -> Result<BTreeSet<String>, String> {
    let mut found_session_meta = false;
    let mut stale_cwds = BTreeSet::new();
    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let json = line
            .strip_suffix(b"\r\n")
            .or_else(|| line.strip_suffix(b"\n"))
            .unwrap_or(line);
        if !contains_byte_sequence(json, b"session_meta")
            && !contains_byte_sequence(json, b"turn_context")
        {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_slice(json) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let record_type = value.get("type").and_then(serde_json::Value::as_str);
        if !matches!(record_type, Some("session_meta" | "turn_context")) {
            continue;
        }
        let payload = value
            .get("payload")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{} row has no payload", record_type.unwrap_or("structural")))?;
        if record_type == Some("session_meta") {
            let recorded_id = payload
                .get("id")
                .or_else(|| payload.get("thread_id"))
                .or_else(|| payload.get("threadId"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "session_meta row has no thread ID".to_string())?;
            if recorded_id != expected_thread_id {
                return Err(format!(
                    "session_meta thread ID '{}' does not match assigned thread '{}'",
                    recorded_id, expected_thread_id
                ));
            }
            found_session_meta = true;
        }
        let Some(cwd) = payload.get("cwd").and_then(serde_json::Value::as_str) else {
            if record_type == Some("session_meta") {
                return Err("session_meta row has no cwd".to_string());
            }
            continue;
        };
        if !path_belongs_to_project(Path::new(cwd), project_root) {
            stale_cwds.insert(cwd.to_string());
        }
    }
    if !found_session_meta {
        return Err("session_meta row was not found".to_string());
    }
    Ok(stale_cwds)
}

fn remap_codex_structural_cwds(
    source: &[u8],
    expected_thread_id: &str,
    target_cwd: &str,
) -> Result<Vec<u8>, String> {
    validate_absolute_clean_path("target Codex conversation cwd", target_cwd)?;
    let mut output = Vec::with_capacity(source.len());
    let mut found_session_meta = false;
    let mut changed = 0_usize;
    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let (json, ending): (&[u8], &[u8]) = if let Some(json) = line.strip_suffix(b"\r\n") {
            (json, b"\r\n")
        } else if let Some(json) = line.strip_suffix(b"\n") {
            (json, b"\n")
        } else {
            (line, b"")
        };
        if !contains_byte_sequence(json, b"session_meta")
            && !contains_byte_sequence(json, b"turn_context")
        {
            output.extend_from_slice(line);
            continue;
        }
        let mut value: serde_json::Value = match serde_json::from_slice(json) {
            Ok(value) => value,
            Err(_) => {
                output.extend_from_slice(line);
                continue;
            }
        };
        let record_type = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if !matches!(
            record_type.as_deref(),
            Some("session_meta" | "turn_context")
        ) {
            output.extend_from_slice(line);
            continue;
        }
        if record_type.as_deref() == Some("session_meta") {
            let recorded_id = value
                .get("payload")
                .and_then(serde_json::Value::as_object)
                .and_then(|payload| {
                    payload
                        .get("id")
                        .or_else(|| payload.get("thread_id"))
                        .or_else(|| payload.get("threadId"))
                })
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "session_meta row has no thread ID".to_string())?;
            if recorded_id != expected_thread_id {
                return Err(format!(
                    "session_meta thread ID '{}' does not match assigned thread '{}'",
                    recorded_id, expected_thread_id
                ));
            }
            found_session_meta = true;
        }
        let Some(cwd) = value
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|payload| payload.get_mut("cwd"))
            .filter(|cwd| cwd.is_string())
        else {
            if record_type.as_deref() == Some("session_meta") {
                return Err("session_meta row has no cwd".to_string());
            }
            output.extend_from_slice(line);
            continue;
        };
        if cwd.as_str() == Some(target_cwd) {
            output.extend_from_slice(line);
            continue;
        }
        *cwd = serde_json::Value::String(target_cwd.to_string());
        changed = changed.saturating_add(1);
        output.extend_from_slice(
            &serde_json::to_vec(&value)
                .map_err(|error| format!("serialize repaired Codex session row: {}", error))?,
        );
        output.extend_from_slice(ending);
    }
    if !found_session_meta {
        return Err("session_meta row was not found".to_string());
    }
    if changed == 0 {
        return Err("conversation no longer contains a stale structural cwd".to_string());
    }
    Ok(output)
}

fn contains_byte_sequence(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn default_machine_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_sync_v3::domain::{
        RestoreActionKind, StorageConfigV3, StorageKind, TombstoneTarget,
    };
    use crate::sync_tests::stub_s3::StubS3;

    #[test]
    fn host_sensitive_workflows_share_the_host_operation_lock() {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path()).unwrap();
        let guard = repo.acquire_host_operation().unwrap();
        let request = SaveProjectBindingRequest {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            project_root: temp.path().join("project").to_string_lossy().into_owned(),
            profile_ids: BTreeMap::new(),
            expected_revision: None,
        };
        assert!(save_project_binding_with_repository(&repo, request)
            .unwrap_err()
            .contains("host-sensitive operation"));
        assert!(detach_project_binding_with_repository(
            &repo,
            &LocalProjectId::parse("project-a").unwrap()
        )
        .unwrap_err()
        .contains("host-sensitive operation"));
        assert!(
            plan_dependencies_with_repository(&repo, &PlanId::parse("plan-a").unwrap())
                .unwrap_err()
                .contains("host-sensitive operation")
        );
        assert!(repair_codex_conversation_paths_with_repository(
            &repo,
            &LocalProjectId::parse("project-a").unwrap()
        )
        .unwrap_err()
        .contains("host-sensitive operation"));
        assert!(inspect_project_files_with_repository(
            &repo,
            &LocalProjectId::parse("project-a").unwrap(),
            &StorageId::parse("storage-a").unwrap(),
        )
        .unwrap_err()
        .contains("host-sensitive operation"));
        assert!(remove_project_link_with_repository(
            &repo,
            &LocalProjectId::parse("project-a").unwrap(),
            &StorageId::parse("storage-a").unwrap(),
        )
        .unwrap_err()
        .contains("host-sensitive operation"));
        drop(guard);
    }

    #[test]
    fn routine_history_scan_logs_are_throttled_per_project() {
        let mut logged_at = BTreeMap::new();
        assert!(claim_history_scan_log_at(
            &mut logged_at,
            "project-one",
            false,
            100,
        ));
        assert!(!claim_history_scan_log_at(
            &mut logged_at,
            "project-one",
            false,
            100 + HISTORY_SCAN_LOG_THROTTLE_SECS - 1,
        ));
        assert!(claim_history_scan_log_at(
            &mut logged_at,
            "project-one",
            false,
            100 + HISTORY_SCAN_LOG_THROTTLE_SECS,
        ));
        assert!(claim_history_scan_log_at(
            &mut logged_at,
            "project-two",
            false,
            101,
        ));

        assert!(claim_history_scan_log_at(
            &mut logged_at,
            "project-one",
            true,
            100 + HISTORY_SCAN_LOG_THROTTLE_SECS + 1,
        ));
        assert!(!claim_history_scan_log_at(
            &mut logged_at,
            "project-one",
            false,
            100 + HISTORY_SCAN_LOG_THROTTLE_SECS + 2,
        ));
    }

    #[test]
    fn failed_pull_preserves_the_previous_success_timestamp() {
        assert_eq!(successful_pull_timestamp(Some(100), 200, false), Some(100));
        assert_eq!(successful_pull_timestamp(None, 200, false), None);
        assert_eq!(successful_pull_timestamp(Some(100), 200, true), Some(200));
    }

    #[test]
    fn thread_sync_direction_uses_the_reviewed_base() {
        assert_eq!(
            classify_thread_sync_state(Some("same"), Some("same"), true, Some("base")),
            ThreadSyncState::Synced
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("base"), true, Some("base")),
            ThreadSyncState::LocalAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("base"), Some("storage"), true, Some("base")),
            ThreadSyncState::StorageAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("storage"), true, Some("base")),
            ThreadSyncState::Diverged
        );
        assert_eq!(
            classify_thread_sync_state(Some("new"), None, true, None),
            ThreadSyncState::LocalAhead
        );
        assert_eq!(
            classify_thread_sync_state(None, Some("new"), true, None),
            ThreadSyncState::StorageAhead
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), Some("storage"), false, None),
            ThreadSyncState::Unknown
        );
        assert_eq!(
            classify_thread_sync_state(Some("local"), None, false, None),
            ThreadSyncState::LocalOnly
        );
        assert_eq!(
            classify_thread_sync_state(None, Some("storage"), false, None),
            ThreadSyncState::StorageOnly
        );
    }

    #[test]
    fn unavailable_thread_state_reports_the_capture_limit() {
        let reason = format!(
            "'/tmp/session.jsonl' exceeds {} bytes",
            MAX_CONVERSATION_FILE_BYTES
        );
        assert_eq!(
            thread_capture_unavailable_detail(Some(&reason)),
            "Session file exceeds the 100 MiB per-file sync limit."
        );

        let mut counts = ThreadSyncCounts::default();
        count_thread_sync_state(&mut counts, ThreadSyncState::Unavailable);
        assert_eq!(counts.unavailable, 1);
        assert_eq!(counts.unknown, 0);
    }

    #[test]
    fn capability_status_marks_two_unbased_versions_for_review() {
        assert_eq!(
            classify_capability_sync_state(Some("local"), Some("storage"), false, None),
            ThreadSyncState::Diverged
        );
        assert_eq!(
            classify_capability_sync_state(Some("same"), Some("same"), false, None),
            ThreadSyncState::Synced
        );
        assert_eq!(
            classify_capability_sync_state(Some("local"), None, false, None),
            ThreadSyncState::LocalOnly
        );
    }

    #[test]
    fn plugin_status_compares_portable_intent_not_observed_version() {
        let plugin = |source: &str, version: &str| ResourceDescriptor {
            resource_id: ResourceId::parse("codex:plugin:tools").unwrap(),
            kind: ResourceKind::Plugin,
            provider: Some(Provider::Codex),
            scope: ResourceScope::Dependency,
            display_name: "tools".to_string(),
            provenance: Provenance::Plugin {
                provider: Provider::Codex,
                plugin_id: "tools".to_string(),
            },
            apply_policy: ApplyPolicy::ExplicitInstall,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::from([
                ("plugin_source_type".to_string(), "git".to_string()),
                ("plugin_source".to_string(), source.to_string()),
                ("plugin_observed_version".to_string(), version.to_string()),
                (
                    "dependency_argv_json".to_string(),
                    "[\"plugin\",\"add\",\"tools\"]".to_string(),
                ),
            ]),
        };
        let first = plugin("https://example.test/tools.git", "1.0.0");
        let newer_observation = plugin("https://example.test/tools.git", "2.0.0");
        let different_source = plugin("https://mirror.test/tools.git", "2.0.0");

        assert_eq!(
            descriptor_capability_digest(Some(&first)),
            descriptor_capability_digest(Some(&newer_observation))
        );
        assert_ne!(
            descriptor_capability_digest(Some(&first)),
            descriptor_capability_digest(Some(&different_source))
        );
    }

    #[test]
    fn codex_dependency_builds_a_portable_managed_or_git_lock() {
        let resource_id = ResourceId::parse("codex:plugin:documents").unwrap();
        let action = DependencyAction {
            action_id: ActionId::parse("dependency:codex:plugin:documents").unwrap(),
            resource_id: resource_id.clone(),
            kind: DependencyActionKind::InstallCodexPlugin,
            display_name: "documents@openai-primary-runtime".to_string(),
            provider: Some(Provider::Codex),
            argv: vec![
                "plugin".to_string(),
                "add".to_string(),
                "documents@openai-primary-runtime".to_string(),
            ],
            requires_explicit_approval: true,
        };
        let mut descriptor = ResourceDescriptor {
            resource_id,
            kind: ResourceKind::Plugin,
            provider: Some(Provider::Codex),
            scope: ResourceScope::Dependency,
            display_name: action.display_name.clone(),
            provenance: Provenance::Plugin {
                provider: Provider::Codex,
                plugin_id: action.display_name.clone(),
            },
            apply_policy: ApplyPolicy::ExplicitInstall,
            relative_cwd: None,
            codec_version: 1,
            metadata: BTreeMap::from([
                (
                    "plugin_marketplace".to_string(),
                    "openai-primary-runtime".to_string(),
                ),
                ("plugin_source_type".to_string(), "local".to_string()),
            ]),
        };

        let managed = codex_plugin_lock_for_action(&action, &descriptor).unwrap();
        assert_eq!(managed.plugins[0].id, "documents@openai-primary-runtime");
        assert!(managed.marketplaces.is_empty());

        let mut git_action = action.clone();
        git_action.display_name = "documents@team-tools".to_string();
        git_action.argv[2] = git_action.display_name.clone();
        descriptor.display_name = git_action.display_name.clone();
        descriptor.metadata = BTreeMap::from([
            ("plugin_marketplace".to_string(), "team-tools".to_string()),
            ("plugin_source_type".to_string(), "git".to_string()),
            ("plugin_source".to_string(), "owner/repo".to_string()),
        ]);
        let git = codex_plugin_lock_for_action(&git_action, &descriptor).unwrap();
        assert_eq!(git.marketplaces[0].name, "team-tools");
        assert_eq!(git.marketplaces[0].repository, "owner/repo");
    }

    #[test]
    fn plugin_failure_diagnostics_are_redacted_and_bounded() {
        let noisy = format!(
            "Authorization: Bearer do-not-log\nactual catalog error\n{}",
            "x".repeat(MAX_PLUGIN_DIAGNOSTIC_BYTES * 2)
        );
        let detail = plugin_failure_detail(&[], noisy.as_bytes());
        assert!(detail.contains("[redacted]"));
        assert!(detail.contains("actual catalog error"));
        assert!(!detail.contains("do-not-log"));
        assert!(detail.len() <= MAX_PLUGIN_DIAGNOSTIC_BYTES);
    }

    fn repository(temp: &tempfile::TempDir) -> V3Repository {
        V3Repository::from_home_dir(temp.path()).unwrap()
    }

    fn register(repo: &V3Repository) -> LocalProjectRegistration {
        register_local_project_with_repository(
            repo,
            RegisterLocalProjectRequest {
                display_name: "Project A".to_string(),
                repository_fingerprint: None,
                bundle_id: Some(BundleId::parse("0123456789abcdef0123456789abcdef").unwrap()),
            },
        )
        .unwrap()
    }

    fn add_profile(repo: &V3Repository, provider: Provider, path: &Path) -> LocalProviderProfileId {
        std::fs::create_dir_all(path).unwrap();
        create_provider_profile_with_repository(
            repo,
            provider,
            &format!("Test {}", provider_name(provider)),
            &path.to_string_lossy(),
        )
        .unwrap()
        .profile_id
    }

    fn add_local_storage(repo: &V3Repository, storage_id: &StorageId, storage_root: &Path) {
        std::fs::create_dir_all(storage_root).unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Project content store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: storage_root.to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();
    }

    fn bind_and_link_test_replica(
        repo: &V3Repository,
        project: &LocalProjectRegistration,
        storage_id: &StorageId,
        project_root: &Path,
        codex_home: &Path,
    ) -> ProjectBinding {
        std::fs::create_dir_all(project_root).unwrap();
        let profile_id = add_profile(repo, Provider::Codex, codex_home);
        let binding = save_project_binding_with_repository(
            repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        repo.load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .cloned()
            .unwrap_or(binding)
    }

    fn reviewed_project_content_recipe(inventory: &ProjectContentInventory) -> BundleRecipe {
        let mut recipe = BundleRecipe::default();
        for entry in &inventory.entries {
            if entry.local_present && entry.blocked_reason.is_none() && entry.selected_after_scan {
                let resource_id = entry.descriptor.resource_id.clone();
                recipe.entries.insert(
                    resource_id.clone(),
                    RecipeEntry {
                        resource_id,
                        apply_policy: ApplyPolicy::ExplicitReview,
                        required: entry.entry_type == ProjectContentEntryType::Directory,
                    },
                );
            }
        }
        recipe
    }

    #[test]
    fn project_content_nested_tree_and_empty_directory_round_trip_then_delete_safely() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("project-files-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));

        let source = register(&repo);
        let source_root = temp.path().join("machine-a/project");
        std::fs::create_dir_all(source_root.join("docs/specs")).unwrap();
        std::fs::create_dir_all(source_root.join("docs/empty")).unwrap();
        std::fs::write(source_root.join("docs/specs/a.md"), b"portable spec\n").unwrap();
        std::fs::write(source_root.join("docs/file_b.md"), b"original b\n").unwrap();
        std::fs::write(source_root.join("excluded.txt"), b"do not publish\n").unwrap();
        bind_and_link_test_replica(
            &repo,
            &source,
            &storage_id,
            &source_root,
            &temp.path().join("machine-a/codex"),
        );

        let scanned =
            inspect_project_files_with_repository(&repo, &source.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            scanned.eligibility.state,
            ProjectFileSyncEligibilityState::Eligible
        );
        assert!(scanned
            .entries
            .iter()
            .all(|entry| entry.selected_after_scan));
        let excluded_id = scanned
            .entries
            .iter()
            .find(|entry| entry.relative_path == "excluded.txt")
            .unwrap()
            .descriptor
            .resource_id
            .clone();
        let mut recipe = reviewed_project_content_recipe(&scanned);
        recipe.entries.remove(&excluded_id);
        let selected_ids = recipe.entries.keys().cloned().collect::<BTreeSet<_>>();
        let pushed = push_bundle_reviewed_with_repository(
            &repo,
            &source.local_project_id,
            &storage_id,
            Some(recipe),
            ProjectContentPushReview {
                review_token: scanned.review_token.clone(),
                removal_ids: BTreeSet::new(),
                acknowledged_warning_digests: BTreeSet::new(),
            },
        )
        .unwrap();
        assert!(pushed.success, "{}", pushed.message);

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let first_snapshot = engine.inspect(&source.bundle_id).unwrap();
        assert!(first_snapshot.manifest.files.contains_key(
            &super::super::domain::LogicalPath::parse("project/docs/specs/a.md").unwrap()
        ));
        assert!(!first_snapshot.manifest.files.contains_key(
            &super::super::domain::LogicalPath::parse("project/excluded.txt").unwrap()
        ));
        for directory in ["project/docs", "project/docs/specs", "project/docs/empty"] {
            assert!(first_snapshot
                .manifest
                .directories
                .contains_key(&super::super::domain::LogicalPath::parse(directory).unwrap()));
        }
        let config_after_push = repo.load_config().unwrap();
        let source_link = config_after_push
            .links
            .iter()
            .find(|link| {
                link.local_project_id == source.local_project_id && link.storage_id == storage_id
            })
            .unwrap();
        assert!(source_link
            .project_content_preferences
            .excluded_resource_ids
            .contains(&excluded_id));
        let relinked = save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: false,
            },
        )
        .unwrap();
        assert!(relinked
            .project_content_preferences
            .excluded_resource_ids
            .contains(&excluded_id));
        let rescanned =
            inspect_project_files_with_repository(&repo, &source.local_project_id, &storage_id)
                .unwrap();
        let excluded = rescanned
            .entries
            .iter()
            .find(|entry| entry.descriptor.resource_id == excluded_id)
            .unwrap();
        assert!(!excluded.selected_after_scan);
        assert!(!excluded.selected_in_recipe);
        let other_storage_id = StorageId::parse("project-files-other-store").unwrap();
        add_local_storage(&repo, &other_storage_id, &temp.path().join("other-store"));
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: other_storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let other_destination_scan = inspect_project_files_with_repository(
            &repo,
            &source.local_project_id,
            &other_storage_id,
        )
        .unwrap();
        assert!(
            other_destination_scan
                .entries
                .iter()
                .find(|entry| entry.descriptor.resource_id == excluded_id)
                .unwrap()
                .selected_after_scan
        );

        let git_race_target = register(&repo);
        let git_race_root = temp.path().join("machine-git-race/project");
        let git_race_binding = bind_and_link_test_replica(
            &repo,
            &git_race_target,
            &storage_id,
            &git_race_root,
            &temp.path().join("machine-git-race/codex"),
        );
        let git_race_plan = plan_bundle_restore_with_repository(
            &repo,
            &storage_id,
            &git_race_target.bundle_id,
            &git_race_binding,
        )
        .unwrap();
        assert_eq!(
            git_race_plan
                .project_content_eligibility
                .as_ref()
                .unwrap()
                .state,
            ProjectFileSyncEligibilityState::Eligible
        );
        let git = std::process::Command::new("git")
            .arg("init")
            .arg(&git_race_root)
            .output()
            .unwrap();
        assert!(git.status.success());
        let error = apply_bundle_restore_with_repository(
            &repo,
            &git_race_plan.plan_id,
            git_race_plan
                .actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect(),
        )
        .unwrap_err();
        assert!(error.contains("Git"), "{error}");
        assert!(!git_race_root.join("docs/specs/a.md").exists());

        let target = register(&repo);
        let target_root = temp.path().join("machine-b/project");
        let target_binding = bind_and_link_test_replica(
            &repo,
            &target,
            &storage_id,
            &target_root,
            &temp.path().join("machine-b/codex"),
        );
        let restore = plan_bundle_restore_with_repository(
            &repo,
            &storage_id,
            &target.bundle_id,
            &target_binding,
        )
        .unwrap();
        assert_eq!(
            restore
                .actions
                .iter()
                .filter(|action| matches!(
                    action.kind,
                    RestoreActionKind::EnsureProjectDirectory { .. }
                ))
                .count(),
            3
        );
        assert_eq!(
            restore
                .actions
                .iter()
                .filter(|action| matches!(action.kind, RestoreActionKind::WriteProjectFile { .. }))
                .count(),
            2
        );
        assert!(restore
            .actions
            .iter()
            .all(|action| action.requires_explicit_approval));
        let approved = restore
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect();
        let applied =
            apply_bundle_restore_with_repository(&repo, &restore.plan_id, approved).unwrap();
        assert!(applied.success, "{}", applied.message);
        assert_eq!(
            std::fs::read(target_root.join("docs/specs/a.md")).unwrap(),
            b"portable spec\n"
        );
        assert_eq!(
            std::fs::read(target_root.join("docs/file_b.md")).unwrap(),
            b"original b\n"
        );
        assert!(target_root.join("docs/empty").is_dir());

        std::fs::remove_file(source_root.join("docs/specs/a.md")).unwrap();
        std::fs::remove_file(source_root.join("docs/file_b.md")).unwrap();
        std::fs::remove_dir(source_root.join("docs/specs")).unwrap();
        std::fs::remove_dir(source_root.join("docs/empty")).unwrap();
        std::fs::remove_dir(source_root.join("docs")).unwrap();
        let removal_scan =
            inspect_project_files_with_repository(&repo, &source.local_project_id, &storage_id)
                .unwrap();
        assert!(removal_scan
            .entries
            .iter()
            .filter(|entry| selected_ids.contains(&entry.descriptor.resource_id))
            .all(|entry| !entry.local_present));
        let mut removal_recipe = repo
            .load_config()
            .unwrap()
            .links
            .iter()
            .find(|link| {
                link.local_project_id == source.local_project_id && link.storage_id == storage_id
            })
            .and_then(|link| link.recipe.clone())
            .unwrap();
        for resource_id in &selected_ids {
            removal_recipe.entries.remove(resource_id);
        }
        push_bundle_reviewed_with_repository(
            &repo,
            &source.local_project_id,
            &storage_id,
            Some(removal_recipe),
            ProjectContentPushReview {
                review_token: removal_scan.review_token.clone(),
                removal_ids: selected_ids.clone(),
                acknowledged_warning_digests: BTreeSet::new(),
            },
        )
        .unwrap();

        let deletion_snapshot = engine.inspect(&source.bundle_id).unwrap();
        assert!(deletion_snapshot.manifest.files.is_empty());
        assert!(deletion_snapshot.manifest.directories.is_empty());
        assert_eq!(
            deletion_snapshot
                .manifest
                .tombstones
                .values()
                .filter(|tombstone| matches!(
                    tombstone.target,
                    TombstoneTarget::ProjectContentFile { .. }
                ))
                .count(),
            2
        );
        assert_eq!(
            deletion_snapshot
                .manifest
                .tombstones
                .values()
                .filter(|tombstone| matches!(
                    tombstone.target,
                    TombstoneTarget::ProjectContentDirectory { .. }
                ))
                .count(),
            3
        );

        let deletion_plan = plan_bundle_restore_with_repository(
            &repo,
            &storage_id,
            &target.bundle_id,
            &target_binding,
        )
        .unwrap();
        assert_eq!(
            deletion_plan
                .actions
                .iter()
                .filter(|action| matches!(action.kind, RestoreActionKind::DeleteProjectFile { .. }))
                .count(),
            2
        );
        assert_eq!(
            deletion_plan
                .actions
                .iter()
                .filter(|action| matches!(
                    action.kind,
                    RestoreActionKind::DeleteProjectDirectory { .. }
                ))
                .count(),
            3
        );
        let approved = deletion_plan
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect();
        std::fs::write(target_root.join("docs/file_b.md"), b"locally changed b\n").unwrap();
        let deleted =
            apply_bundle_restore_with_repository(&repo, &deletion_plan.plan_id, approved).unwrap();
        assert!(
            !deleted.success,
            "a changed local file or non-empty directory was removed"
        );
        assert!(!target_root.join("docs/specs/a.md").exists());
        assert_eq!(
            std::fs::read(target_root.join("docs/file_b.md")).unwrap(),
            b"locally changed b\n"
        );
        assert!(target_root.join("docs").is_dir());
        let backup_contains_file = walkdir::WalkDir::new(repo.backups_dir().unwrap())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .any(|entry| std::fs::read(entry.path()).ok().as_deref() == Some(b"portable spec\n"));
        assert!(
            backup_contains_file,
            "the deleted file did not receive a recoverable backup"
        );

        std::fs::remove_file(target_root.join("docs/file_b.md")).unwrap();
        let retry = plan_bundle_restore_with_repository(
            &repo,
            &storage_id,
            &target.bundle_id,
            &target_binding,
        )
        .unwrap();
        let retry_approved = retry
            .actions
            .iter()
            .filter(|action| {
                matches!(
                    action.kind,
                    RestoreActionKind::DeleteProjectDirectory { .. }
                )
            })
            .map(|action| action.action_id.clone())
            .collect();
        let retried =
            apply_bundle_restore_with_repository(&repo, &retry.plan_id, retry_approved).unwrap();
        assert!(retried.success, "{}", retried.message);
        assert!(!target_root.join("docs").exists());
    }

    #[test]
    fn initializing_git_after_project_content_review_rejects_push_without_a_head() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("git-race-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("notes.md"), b"notes").unwrap();
        bind_and_link_test_replica(
            &repo,
            &project,
            &storage_id,
            &project_root,
            &temp.path().join("codex"),
        );
        let scanned =
            inspect_project_files_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            scanned.eligibility.state,
            ProjectFileSyncEligibilityState::Eligible
        );
        let git = std::process::Command::new("git")
            .arg("init")
            .arg(&project_root)
            .output()
            .unwrap();
        assert!(git.status.success());
        let error = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            Some(reviewed_project_content_recipe(&scanned)),
            ProjectContentPushReview {
                review_token: scanned.review_token,
                removal_ids: BTreeSet::new(),
                acknowledged_warning_digests: BTreeSet::new(),
            },
        )
        .unwrap_err();
        assert!(error.contains("Git"), "{error}");
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        assert!(engine.read_head(&project.bundle_id).unwrap().is_none());
    }

    #[test]
    fn project_content_warning_and_review_tokens_are_bound_to_exact_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("warning-review-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("notes.txt"), b"sample ghp_test_value_one").unwrap();
        bind_and_link_test_replica(
            &repo,
            &project,
            &storage_id,
            &project_root,
            &temp.path().join("codex"),
        );

        let first =
            inspect_project_files_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let first_warning = first.entries[0].warning_digest.clone().unwrap();
        let recipe = reviewed_project_content_recipe(&first);
        let missing_ack = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            Some(recipe.clone()),
            ProjectContentPushReview {
                review_token: first.review_token.clone(),
                removal_ids: BTreeSet::new(),
                acknowledged_warning_digests: BTreeSet::new(),
            },
        )
        .unwrap_err();
        assert!(missing_ack.contains("acknowledgement"));

        std::fs::write(project_root.join("notes.txt"), b"sample ghp_test_value_two").unwrap();
        let stale = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            Some(recipe),
            ProjectContentPushReview {
                review_token: first.review_token,
                removal_ids: BTreeSet::new(),
                acknowledged_warning_digests: BTreeSet::from([first_warning.clone()]),
            },
        )
        .unwrap_err();
        assert!(stale.contains("changed after Scan"), "{stale}");

        let refreshed =
            inspect_project_files_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let refreshed_warning = refreshed.entries[0].warning_digest.clone().unwrap();
        assert_ne!(first_warning, refreshed_warning);
        let published = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            Some(reviewed_project_content_recipe(&refreshed)),
            ProjectContentPushReview {
                review_token: refreshed.review_token,
                removal_ids: BTreeSet::new(),
                acknowledged_warning_digests: BTreeSet::from([refreshed_warning]),
            },
        )
        .unwrap();
        assert!(published.success);
        assert_eq!(
            published.message,
            format!("Published generation {}", published.generation.unwrap())
        );
    }

    #[test]
    fn project_open_commands_pin_the_checkout_and_assigned_profiles() {
        let binding = ProjectBinding {
            host_id: HostId::parse("00112233445566778899aabbccddeeff").unwrap(),
            replica_id: ReplicaId::parse("replica-test").unwrap(),
            local_project_id: LocalProjectId::parse("project-test").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            project_root: "/tmp/client's project".to_string(),
            canonical_project_root: "/tmp/client's project".to_string(),
            profile_ids: BTreeMap::new(),
            codex_home: Some("/tmp/custom codex/.codex".to_string()),
            claude_home: Some("/tmp/custom claude/.claude".to_string()),
            state: BindingState::Active,
            revision: 1,
            updated_at: 1,
        };

        assert_eq!(
            project_open_commands(&binding),
            vec![
                (
                    "Codex — new",
                    "CODEX_HOME='/tmp/custom codex/.codex' codex -C '/tmp/client'\"'\"'s project'"
                        .to_string(),
                ),
                (
                    "Codex — resume",
                    "CODEX_HOME='/tmp/custom codex/.codex' codex resume -C '/tmp/client'\"'\"'s project'"
                        .to_string(),
                ),
                (
                    "Claude — new",
                    "cd '/tmp/client'\"'\"'s project' && CLAUDE_CONFIG_DIR='/tmp/custom claude/.claude' claude"
                        .to_string(),
                ),
                (
                    "Claude — resume",
                    "cd '/tmp/client'\"'\"'s project' && CLAUDE_CONFIG_DIR='/tmp/custom claude/.claude' claude --resume"
                        .to_string(),
                ),
            ]
        );
    }

    #[test]
    fn conversation_path_repair_only_rewrites_desktop_assigned_threads() {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("agent-sync-home")).unwrap();
        let project = register(&repo);
        let project_root = temp.path().join("healthGame");
        let stale_root = temp.path().join("game3");
        let codex_home = temp.path().join(".codex");
        let sessions = codex_home.join("sessions/2026/07/19");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&stale_root).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &codex_home);
        let binding = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();

        let stale_thread = "019f7500-0000-7000-8000-000000000001";
        let matching_thread = "019f7500-0000-7000-8000-000000000002";
        let unrelated_thread = "019f7500-0000-7000-8000-000000000003";
        let stale_path = sessions.join(format!("rollout-{}.jsonl", stale_thread));
        let matching_path = sessions.join(format!("rollout-{}.jsonl", matching_thread));
        let unrelated_path = sessions.join(format!("rollout-{}.jsonl", unrelated_thread));
        let stale_root_json = serde_json::to_string(stale_root.to_str().unwrap()).unwrap();
        let project_root_json = serde_json::to_string(project_root.to_str().unwrap()).unwrap();
        let message_line = format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"text\":\"keep the literal {} unchanged\"}}}}\n",
            stale_root.display()
        );
        let stale_source = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":{}}}}}\n{}{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":{}}}}}\n",
            stale_thread, stale_root_json, message_line, stale_root_json
        );
        std::fs::write(&stale_path, stale_source.as_bytes()).unwrap();
        std::fs::write(
            &matching_path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":{}}}}}\n",
                matching_thread, project_root_json
            ),
        )
        .unwrap();
        let unrelated_source = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":{}}}}}\n",
            unrelated_thread, stale_root_json
        );
        std::fs::write(&unrelated_path, unrelated_source.as_bytes()).unwrap();
        std::fs::write(
            codex_home.join(".codex-global-state.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "local-projects": {
                    "local-health": {
                        "id": "local-health",
                        "name": "healthGame",
                        "rootPaths": [project_root.to_str().unwrap()]
                    }
                },
                "thread-project-assignments": {},
                "thread-writable-roots": {
                    (stale_thread): [project_root.to_str().unwrap()],
                    (matching_thread): [project_root.to_str().unwrap()],
                    (unrelated_thread): [stale_root.to_str().unwrap()]
                },
                "thread-workspace-root-hints": {}
            }))
            .unwrap(),
        )
        .unwrap();

        let audit =
            audit_codex_conversation_paths_with_repository(&repo, &project.local_project_id)
                .unwrap();
        assert_eq!(audit.assigned_thread_count, 2);
        assert_eq!(audit.matching_thread_count, 1);
        assert_eq!(audit.issues.len(), 1);
        assert_eq!(audit.issues[0].thread_id, stale_thread);
        assert!(audit.can_repair);
        assert!(require_codex_conversation_paths_ready(&repo, &project.local_project_id).is_err());

        let repaired =
            repair_codex_conversation_paths_with_repository(&repo, &project.local_project_id)
                .unwrap();
        assert_eq!(repaired.repaired_thread_ids, vec![stale_thread]);
        assert!(repaired.audit.ready);
        assert_eq!(repaired.audit.matching_thread_count, 2);
        let repaired_source = std::fs::read_to_string(&stale_path).unwrap();
        assert!(repaired_source.contains(&message_line));
        for line in repaired_source.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            if matches!(
                value.get("type").and_then(serde_json::Value::as_str),
                Some("session_meta" | "turn_context")
            ) {
                assert_eq!(
                    value["payload"]["cwd"].as_str(),
                    Some(binding.project_root.as_str())
                );
            }
        }
        assert_eq!(
            std::fs::read(&unrelated_path).unwrap(),
            unrelated_source.as_bytes()
        );
        let backup_dir = PathBuf::from(repaired.backup_dir.unwrap());
        let backups = std::fs::read_dir(backup_dir)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(backups[0].path()).unwrap(),
            stale_source.as_bytes()
        );
        require_codex_conversation_paths_ready(&repo, &project.local_project_id).unwrap();
    }

    #[test]
    fn refresh_and_push_auto_select_new_project_conversations() {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("shared").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Shared".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        let first_id = "019f742a-a206-7932-876c-9db8d8ce575a";
        let second_id = "019f742b-0e23-7f95-aab5-124bdbdf6b42";
        let sessions = codex_home.join("sessions/2026/07/18");
        std::fs::create_dir_all(&sessions).unwrap();
        for id in [first_id, second_id] {
            std::fs::write(
                sessions.join(format!("rollout-{}.jsonl", id)),
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":{}}}}}\n",
                    id,
                    serde_json::to_string(project_root.to_str().unwrap()).unwrap()
                ),
            )
            .unwrap();
        }
        std::fs::write(
            codex_home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"First\"}}\n{{\"id\":\"{}\",\"thread_name\":\"Second\"}}\n",
                first_id, second_id
            ),
        )
        .unwrap();

        // Refresh persists default conversation selection without requiring a
        // separate Save project recipe click.
        let inventory =
            get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        for resource_id in [
            format!("codex:session:{}", first_id),
            format!("codex:session:{}", second_id),
            "codex:session-index".to_string(),
        ] {
            assert!(inventory
                .recipe
                .entries
                .contains_key(&ResourceId::parse(resource_id).unwrap()));
        }

        let revision_after_selection = repo.load_config().unwrap().revision;
        get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        assert_eq!(
            repo.load_config().unwrap().revision,
            revision_after_selection,
            "an unchanged inventory must not invalidate the config revision"
        );

        // A newly-created conversation is also reconciled at Push time, even
        // when the UI has not refreshed since it appeared.
        let third_id = "019f742c-0e23-7f95-aab5-124bdbdf6b43";
        std::fs::write(
            sessions.join(format!("rollout-{}.jsonl", third_id)),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":{}}}}}\n",
                third_id,
                serde_json::to_string(project_root.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(codex_home.join("session_index.jsonl"))
            .and_then(|mut file| {
                use std::io::Write as _;
                writeln!(
                    file,
                    "{{\"id\":\"{}\",\"thread_name\":\"Third\"}}",
                    third_id
                )
            })
            .unwrap();

        let pushed =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert!(pushed
            .results
            .iter()
            .any(|result| result.resource_id.as_str() == format!("codex:session:{}", third_id)));
        let persisted = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .entries
            .keys()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        assert!(persisted.contains(&format!("codex:session:{}", third_id)));
        // An explicit Push selection belongs to this destination and overrides
        // the project defaults without rewriting them.
        let pushed_empty = push_bundle_with_recipe_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            Some(BundleRecipe::default()),
        )
        .unwrap();
        assert!(pushed_empty.results.is_empty());
        let config = repo.load_config().unwrap();
        let link = config
            .links
            .iter()
            .find(|link| {
                link.local_project_id == project.local_project_id && link.storage_id == storage_id
            })
            .unwrap();
        assert!(link.recipe.as_ref().unwrap().entries.is_empty());
        assert!(config
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .entries
            .contains_key(&ResourceId::parse(format!("codex:session:{}", third_id)).unwrap()));
        assert!(
            get_bundle_status_with_repository(&repo, &project.local_project_id, &storage_id,)
                .unwrap()
                .statuses
                .is_empty()
        );
        let base = config
            .project(&project.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .unwrap();
        assert!(base.last_push_at.is_some());
        assert_eq!(base.last_pull_at, None);
        assert!(base.commit_id.is_some());
        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(comparison.counts.local, 3);
        assert_eq!(comparison.counts.storage, 0);
        assert_eq!(comparison.counts.diverged, 0);
        assert!(comparison
            .entries
            .iter()
            .all(|entry| entry.state == ThreadSyncState::LocalAhead));
    }

    #[test]
    fn registration_generates_local_identity_and_persists_recipe() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        assert!(project.local_project_id.as_str().starts_with("project-"));
        assert_eq!(project.recipe.revision, 0);
        assert_eq!(repo.load_config().unwrap().projects, vec![project]);
    }

    #[test]
    fn multiple_local_checkouts_can_share_one_remote_bundle_identity() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let bundle_id = BundleId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let first = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Project A".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let second = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Project A replica".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        assert_ne!(first.local_project_id, second.local_project_id);
        assert_eq!(first.bundle_id, bundle_id);
        assert_eq!(second.bundle_id, bundle_id);
        assert_eq!(repo.load_config().unwrap().projects.len(), 2);
    }

    #[test]
    fn default_alias_combines_repo_name_and_hostname_and_deduplicates() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first = register(&repo);
        let base = first
            .local_alias
            .clone()
            .expect("new project gets a default alias");
        assert!(base.starts_with("Project A ("), "unexpected alias '{base}'");
        assert!(base.ends_with(')'));
        // A second checkout of the same repo on this machine gets a counter.
        let second = register(&repo);
        assert_eq!(
            second.local_alias.as_deref(),
            Some(format!("{base} 2").as_str())
        );
        let third = register(&repo);
        assert_eq!(
            third.local_alias.as_deref(),
            Some(format!("{base} 3").as_str())
        );
    }

    #[test]
    fn default_alias_prefers_the_config_name_over_the_hostname() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first = register(&repo);
        let taken = first.local_alias.clone().unwrap();
        let projects = repo.load_config().unwrap().projects;
        // Generated config names abbreviate to their distinctive part.
        assert_eq!(
            default_local_alias("Project A", Some("conf2 · Codex"), &projects).as_deref(),
            Some("Project A (conf2)")
        );
        assert_eq!(
            default_local_alias("Project A", Some("Default Codex"), &projects).as_deref(),
            Some("Project A (Codex)")
        );
        // Without a config the hostname fallback and counter still apply.
        assert_eq!(
            default_local_alias("Project A", None, &projects).as_deref(),
            Some(format!("{taken} 2").as_str())
        );
    }

    #[test]
    fn local_alias_renames_stay_off_the_shared_display_name() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let renamed = rename_local_project_with_repository(
            &repo,
            &project.local_project_id,
            Some("  game3 checkout  ".to_string()),
            project.revision,
        )
        .unwrap();
        assert_eq!(renamed.local_alias.as_deref(), Some("game3 checkout"));
        assert_eq!(renamed.display_name, project.display_name);
        assert_eq!(renamed.revision, project.revision + 1);

        // A stale revision must not clobber a concurrent edit.
        assert!(rename_local_project_with_repository(
            &repo,
            &project.local_project_id,
            Some("other".to_string()),
            project.revision,
        )
        .unwrap_err()
        .contains("changed"));

        // Clearing falls back to the shared name; blank input counts as clearing.
        let cleared = rename_local_project_with_repository(
            &repo,
            &project.local_project_id,
            Some("   ".to_string()),
            renamed.revision,
        )
        .unwrap();
        assert_eq!(cleared.local_alias, None);
        assert_eq!(repo.load_config().unwrap().projects[0].local_alias, None);
    }

    #[test]
    fn recipe_updates_require_the_opened_revision() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let saved = save_bundle_recipe_with_repository(
            &repo,
            &project.local_project_id,
            project.recipe.clone(),
        )
        .unwrap();
        assert_eq!(saved.recipe.revision, 1);
        assert!(save_bundle_recipe_with_repository(
            &repo,
            &project.local_project_id,
            project.recipe,
        )
        .unwrap_err()
        .contains("changed"));
    }

    #[test]
    fn every_storage_link_reuses_the_registration_bundle_id() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: StorageId::parse("backup").unwrap(),
            name: "Backup".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();
        let link = save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id,
                storage_id: StorageId::parse("backup").unwrap(),
                pinned: false,
            },
        )
        .unwrap();
        assert_eq!(link.bundle_id, project.bundle_id);
    }

    #[test]
    fn unlink_removes_the_destination_base_without_deleting_storage_data() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let storage_id = StorageId::parse("backup").unwrap();
        let storage_root = temp.path().join("store");
        std::fs::create_dir_all(&storage_root).unwrap();
        let marker = storage_root.join("remote-object");
        std::fs::write(&marker, b"keep").unwrap();

        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Backup".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: storage_root.to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        repo.mutate_config(|config| {
            config
                .projects
                .iter_mut()
                .find(|candidate| candidate.local_project_id == project.local_project_id)
                .unwrap()
                .recipe_bases
                .insert(
                    storage_id.clone(),
                    RecipeBase {
                        generation: 1,
                        manifest_sha256: "a".repeat(64),
                        commit_id: None,
                        recipe_revision: 1,
                        binding_revision: Some(1),
                        last_pull_at: Some(1),
                        last_push_at: None,
                    },
                );
            Ok(())
        })
        .unwrap();

        assert!(
            remove_project_link_with_repository(&repo, &project.local_project_id, &storage_id,)
                .unwrap()
        );
        let mut config = repo.load_config().unwrap();
        assert!(config.links.is_empty());
        assert!(!config.projects[0].recipe_bases.contains_key(&storage_id));
        assert!(marker.exists());

        config.storages.retain(|storage| storage.id != storage_id);
        repo.save_config(config).unwrap();
        assert!(marker.exists());
        assert!(!remove_project_link_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap());
    }

    #[test]
    fn linked_local_only_project_publishes_with_an_unselected_blocked_skill() {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("local-only-publish").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Local-only destination".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(codex_home.join("skills/_backup/openai-docs")).unwrap();
        std::fs::write(
            codex_home.join("skills/_backup/openai-docs/SKILL.md"),
            "---\nname: openai-docs\n---\n",
        )
        .unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        let inventory =
            get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        let blocked = inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.display_name == "_backup")
            .unwrap();
        assert!(blocked.blocked_reason.is_some());
        assert!(!inventory
            .recipe
            .entries
            .contains_key(&blocked.descriptor.resource_id));

        let status = get_project_capability_status_with_repository(
            &repo,
            &project.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        assert!(status.items.iter().any(|item| {
            item.descriptor.resource_id == blocked.descriptor.resource_id
                && item.state == "blocked"
                && !item.selected_in_recipe
        }));

        let result =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(result.generation, Some(1));
        let remote = list_remote_bundle_snapshots_with_repository(&repo, &storage_id).unwrap();
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].bundle_id, project.bundle_id);
    }

    #[test]
    fn unlinked_project_can_adopt_a_matching_remote_bundle_when_storage_is_added_later() {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("shared").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Shared".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let fingerprint = "a".repeat(64);
        let source_bundle = BundleId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let source = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Project A".to_string(),
                repository_fingerprint: Some(fingerprint.clone()),
                bundle_id: Some(source_bundle.clone()),
            },
        )
        .unwrap();
        let source_root = temp.path().join("project-a");
        std::fs::create_dir_all(&source_root).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();

        let other_bundle = BundleId::parse("cccccccccccccccccccccccccccccccc").unwrap();
        let other = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Other repo".to_string(),
                repository_fingerprint: Some("c".repeat(64)),
                bundle_id: Some(other_bundle.clone()),
            },
        )
        .unwrap();
        let other_root = temp.path().join("other-repo");
        std::fs::create_dir_all(&other_root).unwrap();
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: other.local_project_id.clone(),
                project_root: other_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: other.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        push_bundle_with_repository(&repo, &other.local_project_id, &storage_id).unwrap();

        let available = list_remote_bundle_snapshots_with_repository(&repo, &storage_id).unwrap();
        assert_eq!(available.len(), 2);
        assert!(available.iter().any(|bundle| {
            bundle.bundle_id == source_bundle
                && bundle.repository_fingerprint.as_deref() == Some(fingerprint.as_str())
        }));
        assert!(available
            .iter()
            .any(|bundle| bundle.bundle_id == other_bundle));

        let matches =
            find_remote_bundle_matches_with_repository(&repo, &storage_id, &fingerprint).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].bundle_id, source_bundle);

        let local_bundle = BundleId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let replica = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Project B".to_string(),
                repository_fingerprint: Some(fingerprint.clone()),
                bundle_id: Some(local_bundle.clone()),
            },
        )
        .unwrap();
        let replica_root = temp.path().join("project-b");
        std::fs::create_dir_all(&replica_root).unwrap();
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: replica.local_project_id.clone(),
                project_root: replica_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();
        // This is the reported failure mode: linking first stores a fresh
        // local-only bundle ID which Pull cannot find remotely.
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: replica.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        let connected = connect_project_to_remote_bundle_with_repository(
            &repo,
            ConnectProjectBundleRequest {
                local_project_id: replica.local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: source_bundle.clone(),
                expected_bundle_id: local_bundle,
                pinned: true,
                allow_repository_mismatch: false,
            },
        )
        .unwrap();
        assert_eq!(connected.project.bundle_id, source_bundle);
        assert_eq!(connected.links.len(), 1);
        assert_eq!(connected.links[0].bundle_id, connected.project.bundle_id);
        let binding = connected.binding.unwrap();
        assert_eq!(binding.bundle_id, connected.project.bundle_id);
        assert_eq!(binding.project_root, replica_root.to_string_lossy());
        assert_eq!(binding.profile_ids.get(&Provider::Codex), Some(&profile_id));
        assert_eq!(
            get_bundle_status_with_repository(&repo, &replica.local_project_id, &storage_id)
                .unwrap()
                .generation,
            Some(1)
        );

        let override_local_bundle = BundleId::parse("dddddddddddddddddddddddddddddddd").unwrap();
        let override_replica = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Explicit repo override".to_string(),
                repository_fingerprint: Some(fingerprint.clone()),
                bundle_id: Some(override_local_bundle.clone()),
            },
        )
        .unwrap();
        let override_root = temp.path().join("override-replica");
        std::fs::create_dir_all(&override_root).unwrap();
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: override_replica.local_project_id.clone(),
                project_root: override_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: override_replica.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        let rejected = connect_project_to_remote_bundle_with_repository(
            &repo,
            ConnectProjectBundleRequest {
                local_project_id: override_replica.local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: other_bundle.clone(),
                expected_bundle_id: override_local_bundle.clone(),
                pinned: true,
                allow_repository_mismatch: false,
            },
        )
        .unwrap_err();
        assert!(rejected.contains("different repository"));

        let overridden = connect_project_to_remote_bundle_with_repository(
            &repo,
            ConnectProjectBundleRequest {
                local_project_id: override_replica.local_project_id,
                storage_id,
                bundle_id: other_bundle,
                expected_bundle_id: override_local_bundle,
                pinned: true,
                allow_repository_mismatch: true,
            },
        )
        .unwrap();
        assert_eq!(
            overridden.project.repository_fingerprint.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
    }

    #[tokio::test]
    async fn dependency_plan_generation_uses_a_valid_id_and_supports_retry() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("dependency-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Dependency store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();

        let binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .cloned()
            .unwrap();
        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &project.bundle_id, &binding)
                .unwrap();
        let plan = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();
        assert!(plan.plan_id.as_str().starts_with("plan-"));
        assert_eq!(repo.load_dependency_plan(&plan.plan_id).unwrap(), plan);

        let first = apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new())
            .await
            .unwrap();
        let retry = apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new())
            .await
            .unwrap();
        assert!(first.success);
        assert!(retry.success);
        assert_eq!(
            repo.load_dependency_applications().unwrap().records.len(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dependency_application_fetches_s3_bundle_off_runtime_worker() {
        let temp = tempfile::tempdir().unwrap();
        let cloud_root = temp.path().join("cloud");
        std::fs::create_dir_all(cloud_root.join("bundle-tests")).unwrap();
        let server = StubS3::start(cloud_root).await;
        let repo = repository(&temp);
        let storage_id = StorageId::parse("dependency-s3-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Dependency S3 store".to_string(),
            kind: StorageKind::S3,
            bucket: "bundle-tests".to_string(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret".to_string(),
            account_id: String::new(),
            s3_endpoint: server.endpoint.clone(),
            region: "auto".to_string(),
            local_dir: String::new(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let binding = bind_and_link_test_replica(
            &repo,
            &project,
            &storage_id,
            &temp.path().join("project"),
            &temp.path().join("codex-home"),
        );

        let push_repo = repo.clone();
        let push_project_id = project.local_project_id.clone();
        let push_storage_id = storage_id.clone();
        run_blocking(move || {
            push_bundle_with_repository(&push_repo, &push_project_id, &push_storage_id)
        })
        .await
        .unwrap();

        let restore_repo = repo.clone();
        let restore_storage_id = storage_id.clone();
        let restore_bundle_id = project.bundle_id.clone();
        let restore = run_blocking(move || {
            plan_bundle_restore_with_repository(
                &restore_repo,
                &restore_storage_id,
                &restore_bundle_id,
                &binding,
            )
        })
        .await
        .unwrap();
        let dependency_repo = repo.clone();
        let restore_plan_id = restore.plan_id;
        let plan = run_blocking(move || {
            plan_dependencies_with_repository(&dependency_repo, &restore_plan_id)
        })
        .await
        .unwrap();

        let result =
            apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new()).await;
        assert!(
            result.is_ok(),
            "S3 dependency application failed: {result:?}"
        );
        assert_eq!(
            repo.load_dependency_applications().unwrap().records.len(),
            1
        );
    }

    #[test]
    fn pull_support_uses_the_restore_plans_storage_when_linked_generations_differ() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first_storage_id = StorageId::parse("first-linked-store").unwrap();
        let selected_storage_id = StorageId::parse("selected-pull-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.extend([
            StorageConfigV3 {
                id: first_storage_id.clone(),
                name: "First linked store".to_string(),
                kind: StorageKind::Local,
                bucket: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                account_id: String::new(),
                s3_endpoint: String::new(),
                region: String::new(),
                local_dir: temp
                    .path()
                    .join("first-store")
                    .to_string_lossy()
                    .into_owned(),
                included_default_exclusions: Vec::new(),
                supports_conditional_writes: None,
            },
            StorageConfigV3 {
                id: selected_storage_id.clone(),
                name: "Selected Pull store".to_string(),
                kind: StorageKind::Local,
                bucket: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                account_id: String::new(),
                s3_endpoint: String::new(),
                region: String::new(),
                local_dir: temp
                    .path()
                    .join("selected-store")
                    .to_string_lossy()
                    .into_owned(),
                included_default_exclusions: Vec::new(),
                supports_conditional_writes: None,
            },
        ]);
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();
        for storage_id in [&first_storage_id, &selected_storage_id] {
            save_project_link_with_repository(
                &repo,
                SaveProjectLinkRequest {
                    local_project_id: project.local_project_id.clone(),
                    storage_id: storage_id.clone(),
                    pinned: true,
                },
            )
            .unwrap();
        }

        for _ in 0..2 {
            push_bundle_with_repository(&repo, &project.local_project_id, &first_storage_id)
                .unwrap();
        }
        for _ in 0..3 {
            push_bundle_with_repository(&repo, &project.local_project_id, &selected_storage_id)
                .unwrap();
        }

        let binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .cloned()
            .unwrap();
        let restore = plan_bundle_restore_with_repository(
            &repo,
            &selected_storage_id,
            &project.bundle_id,
            &binding,
        )
        .unwrap();
        let dependencies = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();

        assert_eq!(restore.storage_id, selected_storage_id);
        assert_eq!(restore.generation, 3);
        assert_eq!(dependencies.storage_id, restore.storage_id);
        assert_eq!(dependencies.generation, restore.generation);
        assert_eq!(dependencies.commit_id, restore.commit_id);
        assert_eq!(dependencies.manifest_sha256, restore.manifest_sha256);
        assert_eq!(dependencies.binding_revision, restore.binding_revision);
        assert_eq!(
            get_restore_readiness_with_repository(&repo, &restore.plan_id)
                .unwrap()
                .bundle_id,
            restore.bundle_id
        );

        push_bundle_with_repository(&repo, &project.local_project_id, &selected_storage_id)
            .unwrap();
        assert!(plan_dependencies_with_repository(&repo, &restore.plan_id)
            .unwrap_err()
            .contains("bundle head changed after restore planning"));
        assert!(
            get_restore_readiness_with_repository(&repo, &restore.plan_id)
                .unwrap_err()
                .contains("bundle head changed after restore planning")
        );
    }

    #[test]
    fn pull_review_apply_flow_materializes_a_codex_session_for_the_replica() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("pull-store").unwrap();
        let bundle_id = BundleId::parse("df29babc833808e68ad0efa4d01d7d6d").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Pull store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let source = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "healthGame".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let source_root = temp.path().join("healthGame");
        let source_codex = temp.path().join("source-codex");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(&source_codex).unwrap();
        let session_id = "019f0fb9-b140-7af3-8b7c-5d75c974b230";
        let session_relative = format!(
            "sessions/2026/07/18/rollout-2026-07-18T00-00-00-{}.jsonl",
            session_id
        );
        let source_cwd = source_root.to_string_lossy().into_owned();
        let transcript = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": { "id": session_id, "cwd": source_cwd },
            }),
            serde_json::json!({
                "type": "turn_context",
                "payload": { "cwd": source_cwd },
            }),
        );
        let source_session = source_codex.join(&session_relative);
        std::fs::create_dir_all(source_session.parent().unwrap()).unwrap();
        std::fs::write(&source_session, transcript).unwrap();
        std::fs::write(
            source_codex.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"Change glb color to yellow\"}}\n",
                session_id
            ),
        )
        .unwrap();
        let source_profile = add_profile(&repo, Provider::Codex, &source_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, source_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();

        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "gam2".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let target_root = temp.path().join("gam2");
        let target_codex = temp.path().join("target-codex");
        std::fs::create_dir_all(&target_root).unwrap();
        std::fs::create_dir_all(&target_codex).unwrap();
        let target_profile = add_profile(&repo, Provider::Codex, &target_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: target.local_project_id.clone(),
                project_root: target_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, target_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: target.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let target_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&target.local_project_id)
            .cloned()
            .unwrap();

        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(restore.actions.len(), 2, "session plus filtered index");
        let dependencies = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();
        assert!(dependencies.actions.is_empty());
        let before =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(before.state, "needs_setup");

        let approved = restore
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<Vec<_>>();
        let applied =
            apply_bundle_restore_with_repository(&repo, &restore.plan_id, approved).unwrap();
        assert!(applied.success, "{}", applied.message);
        assert_eq!(applied.applied_action_ids.len(), 2);

        let restored_session =
            std::fs::read_to_string(target_codex.join(&session_relative)).unwrap();
        let rows = restored_session
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows[0]["payload"]["cwd"], target_root.to_str().unwrap());
        assert_eq!(rows[1]["payload"]["cwd"], target_root.to_str().unwrap());
        assert!(target_codex.join("session_index.jsonl").is_file());
        let materializations = repo.load_materializations().unwrap();
        assert_eq!(materializations.records.len(), 1);
        assert_eq!(
            materializations.records[0].status,
            MaterializationStatus::Complete
        );
        let config = repo.load_config().unwrap();
        let target_base = config
            .project(&target.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .unwrap();
        assert!(target_base.last_pull_at.is_some());
        assert_eq!(target_base.last_push_at, None);
        let after =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(after.state, "ready");

        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &target.local_project_id,
            &storage_id,
        )
        .unwrap();
        let restored_thread = comparison
            .entries
            .iter()
            .find(|entry| entry.thread_id == session_id)
            .unwrap();
        assert_eq!(
            restored_thread.state,
            ThreadSyncState::Synced,
            "replica-specific cwd rewriting must not look like a local edit"
        );

        // Skipping an unrelated optional tool keeps the whole materialization
        // partial, so it deliberately does not create a recipe base that can
        // authorize Push. The applied conversation receipt must still provide
        // its own directional History baseline.
        repo.mutate_config(|config| {
            config
                .projects
                .iter_mut()
                .find(|project| project.local_project_id == target.local_project_id)
                .ok_or_else(|| "target project disappeared".to_string())?
                .recipe_bases
                .remove(&storage_id);
            Ok(())
        })
        .unwrap();
        repo.mutate_materializations(|_, materializations| {
            materializations.records[0].status = MaterializationStatus::Partial;
            materializations.records[0].receipts.push(ApplyReceipt {
                action_id: ActionId::parse("action-manual-conflict").unwrap(),
                resource_id: ResourceId::parse(format!("codex:session:{session_id}")).unwrap(),
                action_type: RestoreActionType::Manual,
                logical_path: None,
                source_sha256: None,
                target_path: None,
                target_sha256_after: None,
                status: ActionStatus::Skipped,
                applied_at: now_secs(),
                error: Some("conversation was left unchanged".to_string()),
                conversation: None,
            });
            Ok(())
        })
        .unwrap();
        let partial_readiness =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(partial_readiness.state, "needs_setup");
        assert_eq!(partial_readiness.issues.len(), 1);
        assert_eq!(
            partial_readiness.issues[0].title,
            "1 conversation conflict still needs manual resolution"
        );
        assert_ne!(
            partial_readiness.issues[0].title,
            "Project files are not fully materialized"
        );
        let partial_comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &target.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(partial_comparison.base_generation, Some(1));
        assert_eq!(
            partial_comparison
                .entries
                .iter()
                .find(|entry| entry.thread_id == session_id)
                .unwrap()
                .state,
            ThreadSyncState::Synced,
            "an applied conversation from a partial Pull establishes its own comparison base"
        );

        let mut locally_extended = restored_session;
        locally_extended.push_str(&format!(
            "{}\n",
            serde_json::json!({
                "type": "event_msg",
                "payload": { "message": "continued locally" },
            })
        ));
        std::fs::write(target_codex.join(&session_relative), locally_extended).unwrap();
        let changed = get_project_thread_sync_comparison_with_repository(
            &repo,
            &target.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            changed
                .entries
                .iter()
                .find(|entry| entry.thread_id == session_id)
                .unwrap()
                .state,
            ThreadSyncState::LocalAhead,
            "real local transcript changes must still be detected"
        );

        let repeated_pull =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert!(
            repeated_pull
                .actions
                .iter()
                .all(|action| action.resource_id.as_str() != format!("codex:session:{session_id}")),
            "a known local-ahead conversation must not return as a Pull conflict while storage is unchanged"
        );

        // A first Pull can also encounter the same thread ID with unrelated
        // local bytes and no prior base. Applying the review means "keep
        // local" for that conversation: no bytes are overwritten, the remote
        // generation becomes its reviewed base, and the warning does not
        // return until storage changes again.
        let conflict_target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "conflict-target".to_string(),
                repository_fingerprint: Some("a".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let conflict_root = temp.path().join("conflict-target");
        let conflict_codex = temp.path().join("conflict-codex");
        std::fs::create_dir_all(&conflict_root).unwrap();
        let conflict_session = conflict_codex.join(&session_relative);
        std::fs::create_dir_all(conflict_session.parent().unwrap()).unwrap();
        let local_conflict = format!(
            "{}\n{}\n",
            serde_json::json!({
                "type": "session_meta",
                "payload": { "id": session_id, "cwd": conflict_root.to_string_lossy() },
            }),
            serde_json::json!({
                "type": "event_msg",
                "payload": { "message": "local branch to preserve" },
            }),
        );
        std::fs::write(&conflict_session, &local_conflict).unwrap();
        let conflict_profile = add_profile(&repo, Provider::Codex, &conflict_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: conflict_target.local_project_id.clone(),
                project_root: conflict_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, conflict_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: conflict_target.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let conflict_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&conflict_target.local_project_id)
            .cloned()
            .unwrap();
        let conflict_plan =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &conflict_binding)
                .unwrap();
        let conflict_action = conflict_plan
            .actions
            .iter()
            .find(|action| action.resource_id.as_str() == format!("codex:session:{session_id}"))
            .unwrap();
        assert!(matches!(
            conflict_action.kind,
            RestoreActionKind::Manual { .. }
        ));
        assert!(conflict_action.target_path.is_some());
        assert!(conflict_action.source_sha256.is_some());
        assert!(conflict_action.expected_target_sha256.is_some());
        let approved = conflict_plan
            .actions
            .iter()
            .filter(|action| !matches!(action.kind, RestoreActionKind::Manual { .. }))
            .map(|action| action.action_id.clone())
            .collect::<Vec<_>>();
        let kept =
            apply_bundle_restore_with_repository(&repo, &conflict_plan.plan_id, approved).unwrap();
        assert!(kept.success, "{}", kept.message);
        assert_eq!(
            std::fs::read_to_string(&conflict_session).unwrap(),
            local_conflict,
            "keep-local resolution must not rewrite the conversation"
        );
        let conflict_materialization = repo
            .load_materializations()
            .unwrap()
            .records
            .into_iter()
            .find(|record| record.local_project_id == conflict_target.local_project_id)
            .unwrap();
        assert_eq!(
            conflict_materialization.status,
            MaterializationStatus::Complete
        );
        assert!(repo
            .load_config()
            .unwrap()
            .project(&conflict_target.local_project_id)
            .unwrap()
            .recipe_bases
            .contains_key(&storage_id));
        let conflict_comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &conflict_target.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            conflict_comparison
                .entries
                .iter()
                .find(|entry| entry.thread_id == session_id)
                .unwrap()
                .state,
            ThreadSyncState::LocalAhead
        );
        let after_keep_local =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &conflict_binding)
                .unwrap();
        assert!(after_keep_local
            .actions
            .iter()
            .all(|action| action.resource_id.as_str() != format!("codex:session:{session_id}")));
    }

    #[test]
    fn global_skill_with_distinct_declared_name_round_trips_across_push_and_pull() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("skill-roundtrip-store").unwrap();
        let bundle_id = BundleId::parse("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: storage_id.clone(),
            name: "Skill round-trip store".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: temp.path().join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        // Machine A: the runtime-visible name deliberately differs from its
        // physical directory, and the payload refers to that physical path.
        let source = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Source project".to_string(),
                repository_fingerprint: Some("e".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let source_root = temp.path().join("source-project");
        let source_codex = temp.path().join("source-codex");
        let source_skill = source_codex.join("skills/capture-lsservice-detail/SKILL.md");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(source_skill.parent().unwrap()).unwrap();
        let skill_contents = b"---\nname: get-real-hardware-rh-service\n---\nRun ~/.codex/skills/capture-lsservice-detail/scripts/run.py\n";
        std::fs::write(&source_skill, skill_contents).unwrap();
        let source_profile = add_profile(&repo, Provider::Codex, &source_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, source_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: source.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();

        // Discovery must expose a selectable effective identity while
        // retaining the physical directory in portable metadata and paths.
        let inventory =
            get_bundle_inventory_with_repository(&repo, &source.local_project_id).unwrap();
        let resource_id =
            ResourceId::parse("codex:standalone-skill:get-real-hardware-rh-service").unwrap();
        let skill = inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.resource_id == resource_id)
            .expect("declared-name skill should be discoverable");
        assert!(skill.blocked_reason.is_none());
        assert_eq!(
            skill.descriptor.display_name,
            "get-real-hardware-rh-service"
        );
        assert_eq!(
            skill
                .descriptor
                .metadata
                .get("install_dir_name")
                .map(String::as_str),
            Some("capture-lsservice-detail")
        );
        assert!(skill
            .logical_paths
            .iter()
            .all(|path| path.starts_with("state/codex/skills/capture-lsservice-detail/")));
        assert!(!inventory.recipe.entries.contains_key(&resource_id));

        let mut recipe = inventory.recipe;
        recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        save_bundle_recipe_with_repository(&repo, &source.local_project_id, recipe).unwrap();
        let pushed =
            push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();
        assert!(pushed.success, "{}", pushed.message);
        assert!(pushed
            .results
            .iter()
            .any(|result| result.resource_id == resource_id));
        let source_status = get_project_capability_status_with_repository(
            &repo,
            &source.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let source_skill_status = source_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("source status should include the custom skill");
        assert_eq!(source_skill_status.state, "synced");
        assert!(source_skill_status.local_present);
        assert!(source_skill_status.storage_present);
        assert_eq!(source_status.profiles[0].shared_project_count, 1);

        // Machine B: Pull plans by effective identity but pins the original
        // physical directory as the mutation target.
        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "Target project".to_string(),
                repository_fingerprint: Some("e".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let target_root = temp.path().join("target-project");
        let target_codex = temp.path().join("target-codex");
        std::fs::create_dir_all(&target_root).unwrap();
        let target_profile = add_profile(&repo, Provider::Codex, &target_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: target.local_project_id.clone(),
                project_root: target_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, target_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: target.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let before_status = get_project_capability_status_with_repository(
            &repo,
            &target.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let missing_skill = before_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("target status should include the stored custom skill");
        assert_eq!(missing_skill.state, "storage_only");
        assert!(!missing_skill.local_present);
        assert!(missing_skill.storage_present);
        let target_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&target.local_project_id)
            .cloned()
            .unwrap();
        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        let install = restore
            .actions
            .iter()
            .find(|action| action.resource_id == resource_id)
            .expect("pull should contain the custom-skill action");
        match &install.kind {
            RestoreActionKind::InstallCustomSkill { skill_name, .. } => {
                assert_eq!(skill_name, "get-real-hardware-rh-service")
            }
            other => panic!("expected custom-skill install, got {other:?}"),
        }
        assert!(install.requires_explicit_approval);
        assert!(install
            .target_path
            .as_deref()
            .unwrap()
            .ends_with("skills/capture-lsservice-detail"));
        let dependencies = plan_dependencies_with_repository(&repo, &restore.plan_id).unwrap();
        assert!(dependencies.actions.is_empty());

        let applied = apply_bundle_restore_with_repository(
            &repo,
            &restore.plan_id,
            vec![install.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{}", applied.message);
        assert_eq!(
            std::fs::read(target_codex.join("skills/capture-lsservice-detail/SKILL.md")).unwrap(),
            skill_contents
        );
        assert!(!target_codex
            .join("skills/get-real-hardware-rh-service")
            .exists());
        let readiness =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(readiness.state, "ready");
        let after_status = get_project_capability_status_with_repository(
            &repo,
            &target.local_project_id,
            Some(&storage_id),
        )
        .unwrap();
        let installed_skill = after_status
            .items
            .iter()
            .find(|item| item.descriptor.resource_id == resource_id)
            .expect("installed skill should remain visible in status");
        assert_eq!(installed_skill.state, "synced");
        assert!(installed_skill.local_present);
        assert!(installed_skill.storage_present);
    }

    #[test]
    fn profile_probe_resolves_a_provider_child_and_deduplicates_it() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let container = temp.path().join("myconf2");
        let codex_home = container.join(".codex");
        std::fs::create_dir_all(&codex_home).unwrap();

        let probe = probe_provider_profile_with_repository(
            &repo,
            Provider::Codex,
            &container.to_string_lossy(),
        )
        .unwrap();
        assert!(probe.detected_child);
        assert_eq!(Path::new(&probe.resolved_path), codex_home);

        let first = create_provider_profile_with_repository(
            &repo,
            Provider::Codex,
            "myconf2",
            &container.to_string_lossy(),
        )
        .unwrap();
        let duplicate = create_provider_profile_with_repository(
            &repo,
            Provider::Codex,
            "ignored duplicate name",
            &codex_home.to_string_lossy(),
        )
        .unwrap();
        assert_eq!(first.profile_id, duplicate.profile_id);
        assert_eq!(repo.load_bindings().unwrap().profiles.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn profile_probe_rejects_a_final_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let target = temp.path().join("real-codex-home");
        let link = temp.path().join("linked-codex-home");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error =
            probe_provider_profile_with_repository(&repo, Provider::Codex, &link.to_string_lossy())
                .unwrap_err();
        assert!(error.contains("cannot be a symlink"));
    }

    #[test]
    fn profile_kind_must_match_the_binding_provider() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let project = register(&repo);
        let codex_profile = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));

        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Claude, codex_profile)]),
                expected_revision: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("cannot use Codex profile"));
    }

    #[test]
    fn profile_removal_is_blocked_only_while_an_active_project_uses_it() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let project = register(&repo);
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        let error = remove_provider_profile_with_repository(&repo, &profile_id, 0).unwrap_err();
        assert!(error.contains("used by project"));
        detach_project_binding_with_repository(&repo, &project.local_project_id).unwrap();
        assert!(remove_provider_profile_with_repository(&repo, &profile_id, 0).unwrap());
    }

    #[test]
    fn remap_keeps_replica_identity_and_detaches_old_materializations() {
        let temp = tempfile::tempdir().unwrap();
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let home = temp.path().join("home");
        let repo = V3Repository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        let profile_ids = BTreeMap::from([(Provider::Codex, profile_id)]);
        let first = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: first_root.to_string_lossy().into_owned(),
                profile_ids: profile_ids.clone(),
                expected_revision: None,
            },
        )
        .unwrap();
        let second = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: second_root.to_string_lossy().into_owned(),
                profile_ids,
                expected_revision: Some(first.revision),
            },
        )
        .unwrap();
        assert_eq!(first.replica_id, second.replica_id);
        assert_eq!(second.revision, first.revision + 1);
        assert_eq!(second.project_root, second_root.to_string_lossy());
    }

    #[test]
    fn configured_agent_home_cannot_change_without_re_setup() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let project = register(&repo);
        let first_profile = add_profile(&repo, Provider::Codex, &temp.path().join("codex-one"));
        let second_profile = add_profile(&repo, Provider::Codex, &temp.path().join("codex-two"));
        let first = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, first_profile.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, second_profile)]),
                expected_revision: Some(first.revision),
            },
        )
        .unwrap_err();

        assert!(
            error.contains("agent home is fixed after project setup"),
            "{error}"
        );
        let current = repo
            .load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .unwrap()
            .clone();
        assert_eq!(
            current.profile_ids.get(&Provider::Codex),
            Some(&first_profile)
        );
    }

    #[test]
    fn binding_rejects_local_store_overlap() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let home = temp.path().join("home");
        let repo = V3Repository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: StorageId::parse("inside").unwrap(),
            name: "Inside".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: project_root.join("store").to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();
        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("overlaps local storage"));
    }

    #[test]
    fn binding_requires_and_persists_an_explicit_provider_profile() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let home = temp.path().join("home");
        let repo = V3Repository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::new(),
                expected_revision: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("choose at least one local provider profile"));

        let codex_home = temp.path().join("codex-home");
        let profile_id = add_profile(&repo, Provider::Codex, &codex_home);
        let binding = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        assert_eq!(binding.profile_ids.get(&Provider::Codex), Some(&profile_id));
        assert_eq!(
            binding.codex_home.as_deref(),
            Some(codex_home.to_str().unwrap())
        );
        let persisted = repo.load_bindings().unwrap();
        assert_eq!(persisted.bindings[0].profile_ids, binding.profile_ids);
        assert!(persisted.bindings[0].codex_home.is_none());
    }

    #[test]
    fn one_checkout_hosts_a_second_project_through_a_different_codex_config() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("healthGame");
        std::fs::create_dir_all(&project_root).unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let first = register(&repo);
        let second = register(&repo);
        let main_profile = add_profile(&repo, Provider::Codex, &temp.path().join(".codex"));
        let alt_profile = add_profile(&repo, Provider::Codex, &temp.path().join("conf2/.codex"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: first.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, main_profile.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        // The composite key (folder, config) rejects a second claim on the
        // same Codex config but welcomes a different config on the folder.
        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: second.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, main_profile)]),
                expected_revision: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("already uses this Codex config"), "{error}");

        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: second.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(Provider::Codex, alt_profile)]),
                expected_revision: None,
            },
        )
        .unwrap();
        let machine = repo.load_bindings().unwrap();
        assert!(machine.active_for(&first.local_project_id).is_some());
        assert!(machine.active_for(&second.local_project_id).is_some());
    }

    #[test]
    fn config_save_rejects_store_added_over_an_existing_binding() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        let claude_home = temp.path().join("claude-home");
        for directory in [&project_root, &codex_home, &claude_home] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let home = temp.path().join("home");
        let repo = V3Repository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let codex_profile = add_profile(&repo, Provider::Codex, &codex_home);
        let claude_profile = add_profile(&repo, Provider::Claude, &claude_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([
                    (Provider::Codex, codex_profile),
                    (Provider::Claude, claude_profile),
                ]),
                expected_revision: None,
            },
        )
        .unwrap();

        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfigV3 {
            id: StorageId::parse("overlap").unwrap(),
            name: "Overlap".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: project_root
                .join("bundle-store")
                .to_string_lossy()
                .into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        });
        let error = save_project_sync_config_with_repository(&repo, config).unwrap_err();
        assert!(error.contains("overlaps project root"));
    }

    // ------------------------------------------------------------------
    // Project setup drafts and transactional finalization
    // ------------------------------------------------------------------

    struct SetupFixture {
        _temp: tempfile::TempDir,
        repo: V3Repository,
        project_dir: PathBuf,
        codex_home: PathBuf,
        storage_dir: PathBuf,
    }

    fn setup_fixture() -> SetupFixture {
        let temp = tempfile::tempdir().unwrap();
        let repo = V3Repository::from_home_dir(temp.path().join("home")).unwrap();
        let project_dir = temp.path().join("work/app");
        let codex_home = temp.path().join("codex-profile/.codex");
        let storage_dir = temp.path().join("bucket");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&codex_home).unwrap();
        std::fs::create_dir_all(&storage_dir).unwrap();
        SetupFixture {
            _temp: temp,
            repo,
            project_dir,
            codex_home,
            storage_dir,
        }
    }

    fn local_storage(fixture: &SetupFixture, id: &str) -> StorageConfigV3 {
        StorageConfigV3 {
            id: StorageId::parse(id).unwrap(),
            name: "Local test storage".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: fixture.storage_dir.to_string_lossy().into_owned(),
            included_default_exclusions: Vec::new(),
            supports_conditional_writes: None,
        }
    }

    /// Create a draft and point it at the fixture's pending Codex profile and
    /// pending local storage, ignoring host-dependent default preselection.
    fn draft_ready_to_finalize(fixture: &SetupFixture) -> ProjectSetupDraft {
        let created = create_setup_draft_with_repository(
            &fixture.repo,
            &fixture.project_dir.to_string_lossy(),
        )
        .unwrap();
        assert!(!created.resumed);
        let mut draft = created.draft;
        draft.profiles = BTreeMap::from([(
            Provider::Codex,
            DraftProfileSelection::Pending {
                path: fixture.codex_home.to_string_lossy().into_owned(),
                display_name: String::new(),
            },
        )]);
        draft.storage = Some(DraftStorageSelection::Pending {
            storage: local_storage(fixture, "storage-setup-test"),
        });
        draft.repository = DraftRepositoryChoice::New;
        update_setup_draft_with_repository(&fixture.repo, draft).unwrap()
    }

    #[test]
    fn setup_draft_resumes_for_the_same_canonical_folder() {
        let fixture = setup_fixture();
        let first = create_setup_draft_with_repository(
            &fixture.repo,
            &fixture.project_dir.to_string_lossy(),
        )
        .unwrap();
        let second = create_setup_draft_with_repository(
            &fixture.repo,
            &fixture.project_dir.to_string_lossy(),
        )
        .unwrap();
        assert!(!first.resumed);
        assert!(!first.draft.profiles.contains_key(&Provider::Claude));
        assert!(second.resumed);
        assert_eq!(first.draft.draft_id, second.draft.draft_id);
        let (drafts, warnings) = fixture.repo.list_setup_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn setup_draft_updates_are_revision_guarded() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let stale = draft.clone();
        let saved = update_setup_draft_with_repository(&fixture.repo, draft).unwrap();
        assert_eq!(saved.revision, stale.revision + 1);
        let error = update_setup_draft_with_repository(&fixture.repo, stale).unwrap_err();
        assert!(error.contains("changed"), "unexpected error: {error}");
    }

    #[test]
    fn finalize_creates_every_record_exactly_once() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let expected_project_id = draft.local_project_id.clone();
        let expected_bundle_id = draft.new_bundle_id.clone();

        let detail =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        assert_eq!(detail.project.local_project_id, expected_project_id);
        assert_eq!(detail.project.bundle_id, expected_bundle_id);
        assert_eq!(detail.links.len(), 1);
        assert_eq!(detail.links[0].storage_id.as_str(), "storage-setup-test");
        let binding = detail.binding.expect("binding created");
        assert_eq!(binding.state, BindingState::Active);
        assert_eq!(binding.revision, 0);

        let config = fixture.repo.load_config().unwrap();
        assert_eq!(config.projects.len(), 1);
        assert!(config
            .storages
            .iter()
            .any(|s| s.id.as_str() == "storage-setup-test"));
        let machine = fixture.repo.load_bindings().unwrap();
        assert!(machine.active_for(&expected_project_id).is_some());
        assert!(machine.profiles.iter().any(|profile| {
            profile.provider == Provider::Codex
                && Path::new(&profile.canonical_path)
                    == fs_canonicalize(&fixture.codex_home).unwrap()
        }));

        // The draft and its transaction are consumed by success.
        assert!(fixture
            .repo
            .load_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());
        assert!(fixture
            .repo
            .load_setup_transaction(&draft.draft_id)
            .unwrap()
            .is_none());

        // A retry cannot duplicate anything.
        let error =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap_err();
        assert!(
            error.contains("does not exist"),
            "unexpected error: {error}"
        );
        assert_eq!(fixture.repo.load_config().unwrap().projects.len(), 1);
    }

    #[test]
    fn second_setup_on_one_folder_needs_a_different_codex_config() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
            .unwrap();
        let claimed_profile_id = fixture
            .repo
            .load_bindings()
            .unwrap()
            .profiles
            .iter()
            .find(|profile| {
                Path::new(&profile.canonical_path) == fs_canonicalize(&fixture.codex_home).unwrap()
            })
            .unwrap()
            .profile_id
            .clone();

        // The folder can enter setup again, but the claimed config is never
        // preselected for it.
        let second = create_setup_draft_with_repository(
            &fixture.repo,
            &fixture.project_dir.to_string_lossy(),
        )
        .unwrap();
        assert!(!second.resumed);
        if let Some(DraftProfileSelection::Existing { profile_id }) =
            second.draft.profiles.get(&Provider::Codex)
        {
            assert_ne!(profile_id, &claimed_profile_id);
        }

        let mut colliding = second.draft.clone();
        colliding.profiles = BTreeMap::from([(
            Provider::Codex,
            DraftProfileSelection::Pending {
                path: fixture.codex_home.to_string_lossy().into_owned(),
                display_name: String::new(),
            },
        )]);
        colliding.storage = Some(DraftStorageSelection::Existing {
            storage_id: StorageId::parse("storage-setup-test").unwrap(),
        });
        let colliding = update_setup_draft_with_repository(&fixture.repo, colliding).unwrap();
        let error = finalize_project_setup_with_repository(
            &fixture.repo,
            &colliding.draft_id,
            colliding.revision,
        )
        .unwrap_err();
        assert!(error.contains("already syncs this folder"), "{error}");

        // A different Codex home is a separate project key: it finalizes, and
        // the default alias carries the config name instead of a counter.
        let alt_home = fixture._temp.path().join("conf2/.codex");
        std::fs::create_dir_all(&alt_home).unwrap();
        let mut retry = fixture
            .repo
            .load_setup_draft(&colliding.draft_id)
            .unwrap()
            .unwrap();
        retry.profiles = BTreeMap::from([(
            Provider::Codex,
            DraftProfileSelection::Pending {
                path: alt_home.to_string_lossy().into_owned(),
                display_name: "conf2".to_string(),
            },
        )]);
        let retry = update_setup_draft_with_repository(&fixture.repo, retry).unwrap();
        let detail =
            finalize_project_setup_with_repository(&fixture.repo, &retry.draft_id, retry.revision)
                .unwrap();
        assert_eq!(fixture.repo.load_config().unwrap().projects.len(), 2);
        let alias = detail.project.local_alias.clone().unwrap();
        assert_eq!(alias, "app (conf2)");
        let machine = fixture.repo.load_bindings().unwrap();
        assert!(machine
            .active_for(&detail.project.local_project_id)
            .is_some());
    }

    #[test]
    fn interrupted_finalize_recovers_on_next_project_listing() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let (transaction, warnings) = build_setup_transaction(&fixture.repo, &draft).unwrap();
        assert!(warnings.is_empty());
        fixture.repo.save_setup_transaction(&transaction).unwrap();

        // Simulate a crash after the profile and config writes landed but
        // before the machine binding write.
        fixture
            .repo
            .mutate_bindings(|_, machine| {
                machine
                    .profiles
                    .extend(transaction.profiles.iter().cloned());
                Ok(())
            })
            .unwrap();
        fixture
            .repo
            .mutate_config(|config| {
                config.storages.push(transaction.storage.clone().unwrap());
                config.projects.push(transaction.project.clone());
                config.links.extend(transaction.links.iter().cloned());
                Ok(())
            })
            .unwrap();
        assert!(fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&draft.local_project_id)
            .is_none());

        let recovery_warnings = recover_setup_state(&fixture.repo);
        assert!(recovery_warnings.is_empty(), "{recovery_warnings:?}");

        let machine = fixture.repo.load_bindings().unwrap();
        assert!(machine.active_for(&draft.local_project_id).is_some());
        assert_eq!(fixture.repo.load_config().unwrap().projects.len(), 1);
        assert!(fixture
            .repo
            .load_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());
        assert!(fixture
            .repo
            .load_setup_transaction(&draft.draft_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn failed_transaction_with_nothing_applied_returns_to_draft() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);

        // A profile record nested under an existing profile fails machine
        // validation in the first apply step, before anything persists.
        let existing = add_profile(
            &fixture.repo,
            Provider::Claude,
            &fixture._temp.path().join("claude-profile/.claude"),
        );
        let nested = fixture._temp.path().join("claude-profile/.claude/nested");
        std::fs::create_dir_all(&nested).unwrap();
        let _ = existing;
        let now = now_secs();
        let bad_profile = ProviderProfile {
            profile_id: LocalProviderProfileId::parse("profile-bad").unwrap(),
            provider: Provider::Claude,
            display_name: "Nested".to_string(),
            path: nested.to_string_lossy().into_owned(),
            canonical_path: fs_canonicalize(&nested)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            revision: 0,
            created_at: now,
            updated_at: now,
        };
        let (mut transaction, _) = build_setup_transaction(&fixture.repo, &draft).unwrap();
        transaction.profiles.push(bad_profile);
        fixture.repo.save_setup_transaction(&transaction).unwrap();

        let recovery_warnings = recover_setup_state(&fixture.repo);
        assert!(recovery_warnings
            .iter()
            .any(|warning| warning.contains("rolled back")));
        // Nothing was created; the draft survives and records the failure.
        assert_eq!(fixture.repo.load_config().unwrap().projects.len(), 0);
        assert!(fixture
            .repo
            .load_setup_transaction(&draft.draft_id)
            .unwrap()
            .is_none());
        let draft = fixture
            .repo
            .load_setup_draft(&draft.draft_id)
            .unwrap()
            .expect("draft survives");
        assert!(draft.last_error.is_some());
    }

    #[test]
    fn discard_removes_only_draft_metadata() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        assert!(discard_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap());
        assert!(fixture
            .repo
            .load_setup_draft(&draft.draft_id)
            .unwrap()
            .is_none());
        assert!(fixture.project_dir.is_dir());
        assert!(fixture.codex_home.is_dir());
        // Discarding again reports that nothing was there.
        assert!(!discard_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap());
    }

    #[test]
    fn discard_refuses_while_a_finalization_is_recovering() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let (transaction, _) = build_setup_transaction(&fixture.repo, &draft).unwrap();
        fixture.repo.save_setup_transaction(&transaction).unwrap();
        let error =
            discard_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap_err();
        assert!(error.contains("recovering"), "unexpected error: {error}");
    }

    #[test]
    fn finalize_requires_the_reviewed_draft_revision() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let error = finalize_project_setup_with_repository(
            &fixture.repo,
            &draft.draft_id,
            draft.revision + 7,
        )
        .unwrap_err();
        assert!(error.contains("changed"), "unexpected error: {error}");
        assert_eq!(fixture.repo.load_config().unwrap().projects.len(), 0);
    }

    #[test]
    fn unidentified_remote_repository_requires_acknowledgement() {
        assert!(verified_repository_match(
            &Some("a".repeat(64)),
            &Some("a".repeat(64))
        ));
        assert!(!verified_repository_match(&Some("a".repeat(64)), &None));
        assert!(!verified_repository_match(&None, &None));
        assert!(!verified_repository_match(
            &Some("a".repeat(64)),
            &Some("b".repeat(64))
        ));
    }
}
