//! Tauri command surface for the current project-sync implementation.
//!
//! This layer composes provider capture, the bundle engine, and local
//! persistence. Local-folder and S3-compatible storage use the same immutable
//! bundle publication and head-CAS protocol.

use super::bundle_engine::{
    write_immutable_backup, write_target_atomic, BundleBytes, BundleEngine, BundleObjectStore,
    CapturedBundleFile, CasExpectation, CasOutcome, FetchProgress, ImmutablePutOutcome,
    LocalBundleObjectStore, ObjectCache, ObjectKey, ObjectPrefix, PublishBundleRequest,
    PublishExpectation, RemoteBundlePage, StoreListPage, StoredObject,
};
use super::chat_history::{self, CodexThreadDetailsPage, ProjectChatHistory};
use super::codex_app_refresh::{
    refresh_offer as codex_app_refresh_offer, restart_after_pull, CodexAppKind,
    CodexAppRefreshOffer, CodexAppRefreshState, CodexAppRestartResult,
};
use super::codex_native_fork::{build_divergence_plan, CodexForkExecutionRequest};
use super::codex_rollout::{analyze_codex_rollout_structure, structural_record_type};
use super::conversation_codec::{
    analyze_codex_transcript, rewrite_codex_transcript, CodecLineage, CODEX_CODEC_VERSION,
    CODEX_LINEAGE_CODEC_KEY,
};
use super::conversation_materialization::{
    apply_codex_divergence_locked, apply_conversation_cloud_publications_locked,
    apply_conversation_materialization_locked, cancel_codex_divergence_locked,
    codex_fork_request_for_operation, recover_conversation_materializations,
    resume_codex_divergence_locked, CodexDivergenceApplyOutcome,
    CodexDivergenceMaterializationRequest, ConversationCloudPublication,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use super::conversation_sync::{
    build_conversation_plan_action, default_conversation_decision,
    recover_unique_compatible_binding, validate_conversation_action_pins, BindingRecoveryCandidate,
    BindingRecoveryResult, BindingResolution, ConversationApplyContext, ConversationCapabilities,
    ConversationDecision, ConversationPlanActionRequest,
};
use super::domain::{
    absent_bundle_head_pin, conversation_resource_id, conversation_transcript_path,
    generated_named_id, generated_uuid_v4, initial_conversation_thread_id,
    validate_absolute_clean_path, ActionId, ActionStatus, ApplyPolicy, ApplyReceipt, BindingState,
    BundleDirectoryEntry, BundleHead, BundleId, BundleIdentity, BundleKind, BundleManifest,
    BundleRecipe, BundleSnapshot, CapturedWith, CodexDivergenceOperation, CodexForkExecutor,
    CodexInventoryRefreshStatus, ConversationDivergencePlan, ConversationDivergenceResolution,
    ConversationFileRef, ConversationFileRole, ConversationLineageStatus,
    ConversationMaterializationState, ConversationNameStatus, ConversationOperationId,
    ConversationPlanAction, ConversationPlanActionType, ConversationPlanDirection,
    ConversationPushPlan, ConversationReasonCode, ConversationRelation,
    ConversationResolutionSelection, ConversationSyncOperation, ConversationSyncOperationKind,
    ConversationSyncPhase, ConversationThread, DraftProfileSelection, DraftRepositoryChoice,
    DraftStorageSelection, HostId, LocalConversationBinding, LocalProjectId,
    LocalProjectRegistration, LocalProviderProfileId, LogicalPath, MachineProjectState,
    MaterializationId, MaterializationRecord, MaterializationStatus, OriginHostAlias,
    OriginHostToken, PlanId, PortableOrigin, ProjectBinding, ProjectFileSyncEligibility,
    ProjectFileSyncEligibilityState, ProjectSetupDraft, ProjectStorageLink, Provenance, Provider,
    ProviderProfile, ProviderStateExpectation, RecipeBase, RecipeEntry, ReplicaId,
    ResourceDescriptor, ResourceId, ResourceKind, ResourceScope, RestoreAction, RestoreActionKind,
    RestoreActionType, RestorePlan, SetupDraftId, SetupTransaction, StorageConfig, StorageId,
    StorageKind, SyncConfig, TargetExpectation, ThreadId, TombstoneTarget,
    MAX_CONVERSATION_FILE_BYTES, PUSH_PLAN_SCHEMA, SETUP_DRAFT_SCHEMA, SETUP_TRANSACTION_SCHEMA,
};
use super::persistence::LocalStateRepository;
use super::provider_capture::{
    self, CaptureApplyPolicy, CaptureRequest, CaptureResourceKind, Provider as CaptureProvider,
    ResourceCandidate,
};
use super::s3_store::S3BundleObjectStore;
use crate::activity_log::{emit_typed_log, ActivityLogScope, ActivityLogType};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::Emitter as _;
use tokio::time::Duration;
use walkdir::WalkDir;

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
pub struct ThreadSyncDecision {
    pub action: ConversationPlanActionType,
    pub reason_code: ConversationReasonCode,
    pub selectable: bool,
    pub selected_by_default: bool,
    pub requires_explicit_approval: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

impl From<ConversationDecision> for ThreadSyncDecision {
    fn from(decision: ConversationDecision) -> Self {
        Self {
            action: decision.action,
            reason_code: decision.reason_code,
            selectable: decision.selectable,
            selected_by_default: decision.selected_by_default,
            requires_explicit_approval: decision.requires_explicit_approval,
            blocked_reason: decision.blocked_reason,
        }
    }
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
    pub push_decision: ThreadSyncDecision,
    pub pull_decision: ThreadSyncDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_label: Option<String>,
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

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreApplyStatus {
    Complete,
    AwaitingManualFork,
    RecoveryRequired,
}

/// One paused divergence resolution. A Pull pass emits one checkpoint per
/// divergence that could not finish automatically; each is verified or
/// canceled independently while the rest of the Pull stays applied.
#[derive(Serialize, Clone, Debug)]
pub struct CodexForkCheckpoint {
    pub operation_id: ConversationOperationId,
    pub plan_id: PlanId,
    pub action_id: ActionId,
    pub thread_id: ThreadId,
    pub source_provider_thread_id: String,
    pub suggested_fork_name: String,
    pub status: RestoreApplyStatus,
    pub message: String,
}

/// Overall status of a Pull result is the most actionable pending
/// checkpoint state: any manual-fork wait outranks recovery, and no
/// checkpoints means the pass is complete.
fn restore_status_for_checkpoints(checkpoints: &[CodexForkCheckpoint]) -> RestoreApplyStatus {
    if checkpoints.is_empty() {
        RestoreApplyStatus::Complete
    } else if checkpoints
        .iter()
        .any(|checkpoint| checkpoint.status == RestoreApplyStatus::AwaitingManualFork)
    {
        RestoreApplyStatus::AwaitingManualFork
    } else {
        RestoreApplyStatus::RecoveryRequired
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct CodexDivergenceResult {
    pub action_id: ActionId,
    pub original_thread_id: ThreadId,
    pub original_provider_thread_id: String,
    pub resolution: ConversationDivergenceResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserved_thread_id: Option<ThreadId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserved_provider_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executor: Option<CodexForkExecutor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_fork_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_name_status: Option<ConversationNameStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_original_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name_status: Option<ConversationNameStatus>,
    pub inventory_refresh_status: CodexInventoryRefreshStatus,
}

#[derive(Serialize, Clone, Debug)]
pub struct RestoreResult {
    pub success: bool,
    pub status: RestoreApplyStatus,
    pub message: String,
    pub plan_id: PlanId,
    #[serde(default)]
    pub applied_action_ids: Vec<ActionId>,
    #[serde(default)]
    pub failed_actions: Vec<FailedAction>,
    #[serde(default)]
    pub codex_fork_checkpoints: Vec<CodexForkCheckpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_app_refresh: Option<CodexAppRefreshOffer>,
    #[serde(default)]
    pub codex_divergences: Vec<CodexDivergenceResult>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RestartCodexAppAfterPullRequest {
    pub operation_id: ConversationOperationId,
    pub app_kind: CodexAppKind,
    pub active_chats_complete: bool,
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

fn repository<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<LocalStateRepository, String> {
    LocalStateRepository::from_app(app)
}

fn project_storage_log_labels(
    repository: &LocalStateRepository,
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

fn storage_log_label(repository: &LocalStateRepository, storage_id: &StorageId) -> String {
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
    previous: &[StorageConfig],
    next: &[StorageConfig],
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

/// Process-wide verified-object cache shared by planning, apply, and thread
/// comparison. Content-hash keys make entries immortal-correct; the cache
/// only bounds memory.
fn object_cache() -> Arc<ObjectCache> {
    static OBJECT_CACHE: OnceLock<Arc<ObjectCache>> = OnceLock::new();
    OBJECT_CACHE
        .get_or_init(|| Arc::new(ObjectCache::default()))
        .clone()
}

/// Codex transcript analyses keyed by transcript sha256. Analysis is pure on
/// the bytes, so entries never go stale; repeated thread-sync comparisons
/// stop re-downloading unchanged rollouts entirely.
fn cached_transcript_analysis(
    sha256: &str,
    bytes: &mut dyn BundleBytes,
    logical_path: &super::domain::LogicalPath,
) -> Result<Arc<super::conversation_codec::CodexTranscriptAnalysis>, String> {
    static ANALYSES: OnceLock<
        Mutex<HashMap<String, Arc<super::conversation_codec::CodexTranscriptAnalysis>>>,
    > = OnceLock::new();
    let cache = ANALYSES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache
        .lock()
        .ok()
        .and_then(|entries| entries.get(sha256).cloned())
    {
        return Ok(hit);
    }
    let analysis = Arc::new(analyze_codex_transcript(bytes.get(logical_path)?));
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= 4_096 {
            entries.clear();
        }
        entries.insert(sha256.to_string(), analysis.clone());
    }
    Ok(analysis)
}

#[tauri::command]
pub async fn get_project_sync_config(app: tauri::AppHandle) -> Result<SyncConfig, String> {
    repository(&app)?.load_config()
}

fn host_identity_status(identity: super::domain::HostIdentity) -> HostIdentityStatus {
    HostIdentityStatus {
        host_id: identity.host_id,
        revision: identity.revision,
        local_display_name: identity.local_display_name,
        created_at: identity.created_at,
        updated_at: identity.updated_at,
    }
}

/// Loading the status on app open auto-registers a fresh host whenever no
/// valid identity exists, so this never surfaces a recovery decision.
#[tauri::command]
pub async fn get_host_identity_status(app: tauri::AppHandle) -> Result<HostIdentityStatus, String> {
    let identity = repository(&app)?.load_or_create_host_identity()?;
    Ok(host_identity_status(identity))
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
    Ok(host_identity_status(identity))
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
    Ok(host_identity_status(identity))
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
    config: SyncConfig,
) -> Result<SyncConfig, String> {
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
    repository: &LocalStateRepository,
    config: SyncConfig,
) -> Result<SyncConfig, String> {
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
    run_blocking(move || Ok(repository.load_config()?.projects)).await
}

/// Heal interrupted setup finalizations and conversation materializations.
///
/// The shell runs this once per launch, before any project data is rendered.
/// It is deliberately not folded into `list_local_projects`: recovery rescans
/// every provider rollout and can replace threads, which is far too much work
/// to ride along on a routine listing.
#[tauri::command]
pub async fn recover_startup_state(app: tauri::AppHandle) -> Result<(), String> {
    let repository = repository(&app)?;
    // Both passes are blocking filesystem work, so they belong on the blocking
    // pool rather than a runtime worker.  Logging stays out here: `run_blocking`
    // turns a panic in the worker into a command error, whereas panicking
    // inline would leave the caller's promise unresolved forever.
    let (warnings, recovery) = run_blocking(move || {
        let warnings = recover_setup_state(&repository);
        Ok((warnings, recover_conversation_materializations(&repository)))
    })
    .await?;
    for warning in warnings {
        emit_typed_log(&app, ActivityLogType::Configuration, "warning", &warning);
    }
    match recovery {
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
    Ok(())
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
    repository: &LocalStateRepository,
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
    conversation_push_plan_id: Option<PlanId>,
    approved_conversation_action_ids: Option<Vec<ActionId>>,
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
    let mut upload_progress = repository
        .load_config()
        .ok()
        .and_then(|config| config.project(&local_project_id).cloned())
        .map(|project| {
            FetchProgressEmitter::new(
                app.clone(),
                log.clone(),
                "push",
                storage_id.clone(),
                storage.clone(),
                project.bundle_id,
            )
        });
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
            conversation_push_plan_id,
            approved_conversation_action_ids.unwrap_or_default(),
            &mut |update| {
                if let Some(emitter) = upload_progress.as_mut() {
                    emitter.observe(update);
                }
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

/// Payload for the `project-sync-fetch-progress` Tauri event. Byte counts
/// are manifest-declared, so the UI can render a determinate progress bar.
#[derive(Serialize, Clone)]
struct FetchProgressEventPayload {
    phase: &'static str,
    storage_id: String,
    bundle_id: String,
    index: usize,
    total_objects: usize,
    fetched_bytes: u64,
    total_bytes: u64,
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// Bridges engine transfer progress to the UI: one started log line when the
/// first object lands, a throttled `project-sync-fetch-progress` event stream
/// while the transfer runs, and a completed line with byte and throughput
/// totals at the end. Pull phases (`plan`/`apply`) log as downloads; the
/// `push` phase logs as uploads.
struct FetchProgressEmitter {
    app: tauri::AppHandle,
    log: ActivityLogScope,
    phase: &'static str,
    storage_id: StorageId,
    storage_name: String,
    bundle_id: BundleId,
    started: Option<Instant>,
    last_event: Option<Instant>,
}

impl FetchProgressEmitter {
    fn new(
        app: tauri::AppHandle,
        log: ActivityLogScope,
        phase: &'static str,
        storage_id: StorageId,
        storage_name: String,
        bundle_id: BundleId,
    ) -> Self {
        Self {
            app,
            log,
            phase,
            storage_id,
            storage_name,
            bundle_id,
            started: None,
            last_event: None,
        }
    }

    fn observe(&mut self, progress: FetchProgress) {
        let now = Instant::now();
        let upload = self.phase == "push";
        if self.started.is_none() {
            self.started = Some(now);
            self.log.emit(
                &self.app,
                "info",
                if upload {
                    "push.upload_started"
                } else {
                    "pull.fetch_started"
                },
                &if upload {
                    format!(
                        "↑  Uploading {} objects ({}) to {}",
                        progress.total_objects,
                        format_mb(progress.total_bytes),
                        self.storage_name
                    )
                } else {
                    format!(
                        "↓  Downloading {} objects ({}) from {}",
                        progress.total_objects,
                        format_mb(progress.total_bytes),
                        self.storage_name
                    )
                },
            );
        }
        let last = progress.index == progress.total_objects;
        let due = self
            .last_event
            .is_none_or(|previous| now.duration_since(previous).as_millis() >= 150);
        if last || due {
            self.last_event = Some(now);
            let _ = self.app.emit(
                "project-sync-fetch-progress",
                &FetchProgressEventPayload {
                    phase: self.phase,
                    storage_id: self.storage_id.to_string(),
                    bundle_id: self.bundle_id.to_string(),
                    index: progress.index,
                    total_objects: progress.total_objects,
                    fetched_bytes: progress.fetched_bytes,
                    total_bytes: progress.total_bytes,
                },
            );
        }
        if last {
            let elapsed = self
                .started
                .map(|start| now.duration_since(start).as_secs_f64())
                .unwrap_or_default();
            let rate = if elapsed > 0.0 {
                progress.total_bytes as f64 / 1_000_000.0 / elapsed
            } else {
                0.0
            };
            self.log.emit(
                &self.app,
                "ok",
                if upload {
                    "push.upload_completed"
                } else {
                    "pull.fetch_completed"
                },
                &format!(
                    "{}  {} {} in {:.1}s ({:.1} MB/s)",
                    if upload { "↑" } else { "↓" },
                    if upload { "Uploaded" } else { "Downloaded" },
                    format_mb(progress.total_bytes),
                    elapsed,
                    rate
                ),
            );
        }
    }
}

fn is_selectable_pull_conversation_action(action: &ConversationPlanAction) -> bool {
    matches!(
        action.action,
        ConversationPlanActionType::FastForwardLocal
            | ConversationPlanActionType::ResolveDivergence
            | ConversationPlanActionType::CreateLocal
            | ConversationPlanActionType::ReplaceLocal
    )
}

fn pull_plan_action_counts(plan: &RestorePlan) -> (usize, usize) {
    let ordinary = plan
        .actions
        .iter()
        .filter(|action| !matches!(&action.kind, RestoreActionKind::Manual { .. }))
        .count();
    let conversations = plan
        .conversation_actions
        .iter()
        .filter(|action| is_selectable_pull_conversation_action(action))
        .count();
    (ordinary, conversations)
}

fn pull_action_resource_label(plan: &RestorePlan, action_id: &ActionId) -> Option<String> {
    if let Some(action) = plan
        .actions
        .iter()
        .find(|action| &action.action_id == action_id)
    {
        return Some(action.resource_id.to_string());
    }
    plan.conversation_actions
        .iter()
        .find(|action| &action.action_id == action_id)
        .and_then(|action| {
            action
                .source_resource_id
                .clone()
                .or_else(|| conversation_resource_id(action.provider, &action.thread_id).ok())
        })
        .map(|resource_id| resource_id.to_string())
}

fn pull_review_resource_labels(plan: &RestorePlan) -> Vec<String> {
    let ordinary = plan
        .actions
        .iter()
        .filter(|action| !matches!(&action.kind, RestoreActionKind::Manual { .. }))
        .map(|action| action.resource_id.to_string());
    let conversations = plan
        .conversation_actions
        .iter()
        .filter(|action| is_selectable_pull_conversation_action(action))
        .filter_map(|action| {
            action
                .source_resource_id
                .clone()
                .or_else(|| conversation_resource_id(action.provider, &action.thread_id).ok())
        })
        .map(|resource_id| resource_id.to_string());
    ordinary
        .chain(conversations)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
    let mut progress = FetchProgressEmitter::new(
        app.clone(),
        log.clone(),
        "plan",
        storage_id.clone(),
        storage.clone(),
        bundle_id.clone(),
    );
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        plan_bundle_restore_with_progress(
            &repository,
            &storage_id,
            &bundle_id,
            &binding,
            &mut |update| progress.observe(update),
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "pull.phase_timing",
                    &format!(
                        "   Pull review phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "pull.review_timing",
        &format!(
            "   Pull review total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(plan) => {
            for resource in pull_review_resource_labels(plan) {
                log.emit(
                    &app,
                    "info",
                    "pull.resource_review",
                    &format!("↓  Review {resource}"),
                );
            }
            let (ordinary_actions, conversation_actions) = pull_plan_action_counts(plan);
            let applicable_actions = ordinary_actions.saturating_add(conversation_actions);
            log.emit(
                &app,
                "ok",
                "pull.plan_ready",
                &format!(
                    "✓  Pull plan ready — generation {}, {} applicable actions ({} resources, {} conversations)",
                    plan.generation,
                    applicable_actions,
                    ordinary_actions,
                    conversation_actions
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
    conversation_resolutions: Vec<ConversationResolutionSelection>,
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
    let mut fetch_progress = plan_for_log.as_ref().map(|plan| {
        let storage = storage_log_label(&repository, &plan.storage_id);
        FetchProgressEmitter::new(
            app.clone(),
            log.clone().storage(&plan.storage_id, Some(&storage)),
            "apply",
            plan.storage_id.clone(),
            storage,
            plan.bundle_id.clone(),
        )
    });
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        apply_bundle_restore_with_progress(
            &repository,
            &plan_id,
            approved_action_ids,
            conversation_resolutions,
            &mut |update| {
                if let Some(emitter) = fetch_progress.as_mut() {
                    emitter.observe(update);
                }
            },
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "pull.phase_timing",
                    &format!(
                        "   Pull apply phase — {phase}: {:.3}s",
                        elapsed.as_secs_f64()
                    ),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "pull.apply_timing",
        &format!(
            "   Pull apply total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(operation) => {
            for action_id in &operation.applied_action_ids {
                let resource = plan_for_log
                    .as_ref()
                    .and_then(|plan| pull_action_resource_label(plan, action_id))
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
                        // Resuming a thread in the app takes two sequenced deep
                        // links, so it has no pasteable form; point at the
                        // button that performs them instead.
                        #[cfg(target_os = "macos")]
                        if binding.codex_home.is_some() {
                            log.emit(
                                &app,
                                "info",
                                "pull.open_project_command",
                                "   Codex app — resume: use Open in Codex (one deep link cannot carry both project and chat)",
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
pub async fn get_pending_codex_divergence(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
) -> Result<Option<RestoreResult>, String> {
    let repository = repository(&app)?;
    run_blocking(move || pending_codex_divergence_with_repository(&repository, &local_project_id))
        .await
}

pub(crate) fn pending_codex_divergence_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<Option<RestoreResult>, String> {
    recover_conversation_materializations(repository)?;
    let checkpoints = pending_checkpoints_from_journal(repository, local_project_id)?;
    let Some(first) = checkpoints.first() else {
        return Ok(None);
    };
    let message = if checkpoints.len() == 1 {
        first.message.clone()
    } else {
        format!(
            "{} Codex fork checkpoints need attention; verify or cancel each one",
            checkpoints.len()
        )
    };
    Ok(Some(RestoreResult {
        success: false,
        status: restore_status_for_checkpoints(&checkpoints),
        message,
        plan_id: first.plan_id.clone(),
        applied_action_ids: Vec::new(),
        failed_actions: Vec::new(),
        codex_fork_checkpoints: checkpoints,
        codex_app_refresh: None,
        codex_divergences: Vec::new(),
    }))
}

/// Every pending divergence operation of one project, oldest first, as
/// checkpoints the review can verify or cancel one at a time. This is a
/// plain journal read so callers already holding the host-operation lock
/// (resume, cancel) can reuse it.
fn pending_checkpoints_from_journal(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<Vec<CodexForkCheckpoint>, String> {
    let mut operations = repository
        .load_conversation_sync_journal()?
        .operations
        .into_iter()
        .filter(|operation| {
            operation.local_project_id == *local_project_id
                && operation.divergence().is_some_and(|divergence| {
                    divergence.resolution
                        == ConversationDivergenceResolution::PreserveLocalWithNativeFork
                })
        })
        .collect::<Vec<_>>();
    operations.sort_by_key(|operation| (operation.created_at, operation.operation_id.to_string()));
    operations
        .iter()
        .map(pending_checkpoint_for_operation)
        .collect()
}

fn pending_checkpoint_for_operation(
    operation: &ConversationSyncOperation,
) -> Result<CodexForkCheckpoint, String> {
    let plan_id = operation
        .plan_id
        .clone()
        .ok_or_else(|| "Codex divergence operation lacks a restore plan".to_string())?;
    let action_id = operation
        .plan_action_id
        .clone()
        .ok_or_else(|| "Codex divergence operation lacks a plan action".to_string())?;
    let divergence = operation
        .divergence()
        .ok_or_else(|| "conversation operation is not a Codex divergence".to_string())?;
    let (status, message) = if operation.phase == ConversationSyncPhase::AwaitingManualFork {
        (
            RestoreApplyStatus::AwaitingManualFork,
            "Create the reviewed fork in Codex, then verify it here".to_string(),
        )
    } else {
        (
            RestoreApplyStatus::RecoveryRequired,
            "The interrupted Codex fork must be verified before Pull can continue".to_string(),
        )
    };
    Ok(CodexForkCheckpoint {
        operation_id: operation.operation_id.clone(),
        plan_id,
        action_id,
        thread_id: operation.thread_id.clone(),
        source_provider_thread_id: divergence.source_provider_thread_id.clone(),
        suggested_fork_name: divergence.desired_fork_name.clone(),
        status,
        message,
    })
}

#[tauri::command]
pub async fn resume_codex_divergence(
    app: tauri::AppHandle,
    operation_id: ConversationOperationId,
    provider_thread_id: Option<String>,
) -> Result<RestoreResult, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        let _host_operation = repository.acquire_host_operation()?;
        let operation = repository
            .load_conversation_sync_journal()?
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| format!("conversation operation '{}' does not exist", operation_id))?;
        let plan_id = operation
            .plan_id
            .clone()
            .ok_or_else(|| "Codex divergence operation lacks a restore plan".to_string())?;
        let action_id = operation
            .plan_action_id
            .clone()
            .ok_or_else(|| "Codex divergence operation lacks a plan action".to_string())?;
        let local_project_id = operation.local_project_id.clone();
        let fork = codex_fork_request_for_operation(&repository, &operation)?;
        let resumed_codex_home = fork.codex_home.clone();
        let mut failpoints = NoConversationFailpoints;
        match resume_codex_divergence_locked(
            &repository,
            &operation_id,
            provider_thread_id.as_deref(),
            fork,
            &mut failpoints,
        )? {
            CodexDivergenceApplyOutcome::Committed(receipt) => {
                let codex_divergences = codex_divergence_result(action_id.clone(), &receipt)
                    .into_iter()
                    .collect();
                let remaining = pending_checkpoints_from_journal(&repository, &local_project_id)?;
                let message = if remaining.is_empty() {
                    "Verified the Codex fork and restored the cloud conversation".to_string()
                } else {
                    format!(
                        "Verified the Codex fork and restored the cloud conversation; \
                         {} more checkpoint(s) need attention",
                        remaining.len()
                    )
                };
                Ok(RestoreResult {
                    success: true,
                    status: restore_status_for_checkpoints(&remaining),
                    message,
                    plan_id,
                    applied_action_ids: vec![action_id],
                    failed_actions: Vec::new(),
                    codex_fork_checkpoints: remaining,
                    codex_app_refresh: codex_app_refresh_offer(
                        operation_id.clone(),
                        Some(&resumed_codex_home),
                    ),
                    codex_divergences,
                })
            }
            CodexDivergenceApplyOutcome::AwaitingManualFork { operation_id } => {
                pending_queue_result(&repository, &local_project_id, plan_id, &operation_id, None)
            }
            CodexDivergenceApplyOutcome::RecoveryRequired {
                operation_id,
                message,
            } => pending_queue_result(
                &repository,
                &local_project_id,
                plan_id,
                &operation_id,
                Some(message),
            ),
        }
    })
    .await
}

/// Rebuild the full checkpoint queue after a resume attempt left its
/// operation pending, optionally replacing that operation's generic
/// message with the attempt-specific one.
fn pending_queue_result(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    plan_id: PlanId,
    pending_operation_id: &ConversationOperationId,
    pending_message: Option<String>,
) -> Result<RestoreResult, String> {
    let mut checkpoints = pending_checkpoints_from_journal(repository, local_project_id)?;
    if let Some(message) = pending_message {
        if let Some(checkpoint) = checkpoints
            .iter_mut()
            .find(|checkpoint| &checkpoint.operation_id == pending_operation_id)
        {
            checkpoint.message = message;
        }
    }
    let message = checkpoints
        .iter()
        .find(|checkpoint| &checkpoint.operation_id == pending_operation_id)
        .map(|checkpoint| checkpoint.message.clone())
        .unwrap_or_else(|| "The Codex fork checkpoint is still pending".to_string());
    Ok(RestoreResult {
        success: false,
        status: restore_status_for_checkpoints(&checkpoints),
        message,
        plan_id,
        applied_action_ids: Vec::new(),
        failed_actions: Vec::new(),
        codex_fork_checkpoints: checkpoints,
        codex_app_refresh: None,
        codex_divergences: Vec::new(),
    })
}

#[tauri::command]
pub async fn cancel_codex_divergence(
    app: tauri::AppHandle,
    operation_id: ConversationOperationId,
) -> Result<RestoreResult, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        let _host_operation = repository.acquire_host_operation()?;
        let operation = repository
            .load_conversation_sync_journal()?
            .operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| format!("conversation operation '{}' does not exist", operation_id))?;
        let plan_id = operation
            .plan_id
            .clone()
            .ok_or_else(|| "Codex divergence operation lacks a restore plan".to_string())?;
        let local_project_id = operation.local_project_id.clone();
        cancel_codex_divergence_locked(&repository, &operation_id)?;
        let remaining = pending_checkpoints_from_journal(&repository, &local_project_id)?;
        let message = if remaining.is_empty() {
            "Codex fork checkpoint canceled. The original conversation was not replaced."
                .to_string()
        } else {
            format!(
                "Codex fork checkpoint canceled; {} more checkpoint(s) need attention",
                remaining.len()
            )
        };
        Ok(RestoreResult {
            success: true,
            status: restore_status_for_checkpoints(&remaining),
            message,
            plan_id,
            applied_action_ids: Vec::new(),
            failed_actions: Vec::new(),
            codex_fork_checkpoints: remaining,
            codex_app_refresh: None,
            codex_divergences: Vec::new(),
        })
    })
    .await
}

#[tauri::command]
pub async fn restart_codex_app_after_pull(
    app: tauri::AppHandle,
    request: RestartCodexAppAfterPullRequest,
) -> Result<CodexAppRestartResult, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        if !request.active_chats_complete {
            return Err(
                "confirm that active Codex chats are complete before restarting".to_string(),
            );
        }
        let receipt = committed_conversation_receipt(&repository, &request.operation_id)?;
        let codex_home = codex_home_for_profile(&repository, &receipt.provider_profile_id)?;
        let offer = codex_app_refresh_offer(request.operation_id.clone(), Some(&codex_home))
            .ok_or_else(|| {
                "Codex App no longer needs or supports an automatic restart".to_string()
            })?;
        if offer.state != CodexAppRefreshState::RestartAvailable
            || offer.app_kind != request.app_kind
        {
            return Err("Codex App restart capability changed after Pull".to_string());
        }
        Ok(match restart_after_pull(request.app_kind, &codex_home) {
            Ok(result) => result,
            Err(message) => CodexAppRestartResult {
                success: false,
                message,
            },
        })
    })
    .await
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
    repository: &LocalStateRepository,
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
            "{} profile '{}' overlaps Mallard application data",
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
    repository: &LocalStateRepository,
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

fn ensure_default_provider_profiles(repository: &LocalStateRepository) -> Result<(), String> {
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
    selected_path: &str,
    profile_ids: &BTreeMap<Provider, LocalProviderProfileId>,
) -> Result<ProjectDiscovery, String> {
    let profile_paths = resolve_profile_paths(repository, profile_ids)?;
    discover_project_at(repository, selected_path, &profile_paths, profile_ids)
}

/// Discovery core shared by profile-ID discovery and setup drafts, which may
/// mix existing profiles with pending, not-yet-created profile paths.
fn discover_project_at(
    repository: &LocalStateRepository,
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
        return Err("project root overlaps Mallard application data".to_string());
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
    let request = CaptureRequest {
        project_root: project_root.clone(),
        codex_home: profile_paths.get(&Provider::Codex).map(PathBuf::from),
        claude_home: profile_paths.get(&Provider::Claude).map(PathBuf::from),
        excluded_project_roots: Vec::new(),
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    };
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
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectContentInventory, String> {
    let _host_operation = repository.acquire_host_operation()?;
    inspect_project_files_guarded(repository, local_project_id, storage_id)
}

fn inspect_project_files_guarded(
    repository: &LocalStateRepository,
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
    let discovery = if eligibility.state == ProjectFileSyncEligibilityState::Eligible {
        let mut request = capture_request_for_binding(repository, &binding)?;
        request.include_project_content = true;
        Some(provider_capture::discover_full(&request)?)
    } else {
        None
    };
    build_project_files_inventory(
        local_project_id,
        storage_id,
        &project,
        &link,
        &binding,
        eligibility,
        &engine,
        remote.as_ref(),
        discovery.as_ref(),
    )
}

/// Project-files section over a shared Discovery (walked with project
/// content included) and one already-read remote snapshot. The review token
/// minted here binds current disk state and the observed remote head.
#[allow(clippy::too_many_arguments)]
fn build_project_files_inventory(
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    project: &LocalProjectRegistration,
    link: &ProjectStorageLink,
    binding: &ProjectBinding,
    eligibility: ProjectFileSyncEligibility,
    engine: &StorageEngine,
    remote: Option<&BundleSnapshot>,
    discovery: Option<&provider_capture::Discovery>,
) -> Result<ProjectContentInventory, String> {
    let remote_versions = remote
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
        let discovered = discovery
            .ok_or_else(|| {
                "project-files discovery is missing for an eligible binding".to_string()
            })?
            .project_inventory();
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
                binding,
                &eligibility,
                remote,
                preferences.revision,
                &entries,
            )
        });
    Ok(ProjectContentInventory {
        local_project_id: local_project_id.clone(),
        storage_id: storage_id.clone(),
        project_root: binding.project_root.clone(),
        eligibility,
        review_token,
        storage_generation: remote.map(|snapshot| snapshot.head.generation),
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
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<ResourceInventory, String> {
    let config = repository.load_config()?;
    config
        .project(local_project_id)
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    let bindings = repository.load_bindings()?;
    let binding = bindings.active_for(local_project_id).ok_or_else(|| {
        format!(
            "project '{}' is not mapped on this machine",
            local_project_id
        )
    })?;
    let request = capture_request_for_binding(repository, binding)?;
    let discovery = provider_capture::discover_full(&request)?;
    build_resource_inventory(repository, local_project_id, binding, &discovery)
}

/// Inventory section over a shared Discovery. Persists auto-selected
/// conversations as a side effect, exactly like the standalone command.
fn build_resource_inventory(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    binding: &ProjectBinding,
    discovery: &provider_capture::Discovery,
) -> Result<ResourceInventory, String> {
    let discovered = discovery.project_inventory();
    let project = persist_auto_selected_conversations(
        repository,
        local_project_id,
        &discovered.resources,
        discovery.excluded_codex_subagent_resource_ids(),
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
            CaptureResourceKind::Conversation | CaptureResourceKind::Memory => {
                ResourceScope::ProviderState
            }
            _ => ResourceScope::Project,
        };
        let provenance = candidate
            .logical_paths
            .first()
            .and_then(|path| path.strip_prefix("project/"))
            .map(|relative_path| Provenance::ProjectLocal {
                relative_path: relative_path.to_string(),
            })
            .unwrap_or(Provenance::Unknown);
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
            description: None,
            logical_paths: candidate.logical_paths.clone(),
            default_selected: candidate.selected_by_default,
            blocked_reason: candidate.blocked_reason.clone(),
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
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    candidates: &[ResourceCandidate],
    excluded_codex_subagent_resource_ids: &BTreeSet<String>,
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
    let additions_are_present = additions
        .iter()
        .all(|(resource_id, _)| current.recipe.entries.contains_key(resource_id));
    let has_excluded_subagents = current
        .recipe
        .entries
        .keys()
        .any(|resource_id| excluded_codex_subagent_resource_ids.contains(resource_id.as_str()));
    if additions_are_present && !has_excluded_subagents {
        return Ok(current);
    }
    repository.mutate_config(|config| {
        let project = config
            .projects
            .iter_mut()
            .find(|project| &project.local_project_id == local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        let mut changed = false;
        let previous_entry_count = project.recipe.entries.len();
        project.recipe.entries.retain(|resource_id, _| {
            !excluded_codex_subagent_resource_ids.contains(resource_id.as_str())
        });
        changed |= project.recipe.entries.len() != previous_entry_count;
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

fn remove_excluded_codex_subagents_from_recipe(
    recipe: &mut BundleRecipe,
    excluded_resource_ids: &BTreeSet<String>,
) -> bool {
    let previous_entry_count = recipe.entries.len();
    recipe
        .entries
        .retain(|resource_id, _| !excluded_resource_ids.contains(resource_id.as_str()));
    recipe.entries.len() != previous_entry_count
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
        CaptureResourceKind::ProjectContentFile => ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => ResourceKind::ProjectContentDirectory,
        CaptureResourceKind::Conversation => match provider {
            Some(CaptureProvider::Claude) => ResourceKind::ClaudeConversation,
            _ => ResourceKind::CodexConversation,
        },
        CaptureResourceKind::Memory => ResourceKind::ProjectMemory,
    }
}

fn capture_apply_policy(policy: &CaptureApplyPolicy) -> ApplyPolicy {
    match policy {
        CaptureApplyPolicy::SafeFile => ApplyPolicy::SafeFile,
        CaptureApplyPolicy::Merge => ApplyPolicy::Merge,
        CaptureApplyPolicy::Review => ApplyPolicy::ExplicitReview,
    }
}

fn resource_category(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::CodexConversation
        | ResourceKind::ClaudeConversation
        | ResourceKind::ProjectMemory => "conversations",
        ResourceKind::ProjectContentFile | ResourceKind::ProjectContentDirectory => "project_files",
        ResourceKind::Requirement => "tools",
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

    fn get_many(&self, keys: &[ObjectKey]) -> Result<Vec<Option<StoredObject>>, String> {
        match self {
            Self::Local(store) => store.get_many(keys),
            Self::S3(store) => store.get_many(keys),
        }
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        match self {
            Self::Local(store) => store.put_immutable(key, bytes),
            Self::S3(store) => store.put_immutable(key, bytes),
        }
    }

    fn put_many(
        &self,
        objects: Vec<(ObjectKey, bytes::Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        match self {
            Self::Local(store) => store.put_many(objects),
            Self::S3(store) => store.put_many(objects),
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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

/// Verified head + manifest, no payload objects. Every command goes through
/// this: metadata answers status/readiness/dependency views outright, and
/// plan/apply/thread-comparison stream just the objects they need through a
/// [`BundleBytes`] source on top of the returned snapshot.
fn inspect_from_storage(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
) -> Result<(StorageEngine, BundleSnapshot), String> {
    let (_, engine) = storage_engine(repository, storage_id)?;
    let snapshot = engine.inspect(bundle_id)?;
    Ok((engine, snapshot))
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

fn is_conversation_keep_local_action(action: &RestoreAction, manifest: &BundleManifest) -> bool {
    matches!(action.kind, RestoreActionKind::Manual { .. })
        && manifest
            .resources
            .get(&action.resource_id)
            .is_some_and(|resource| resource.kind == ResourceKind::ClaudeConversation)
        && action.target_path.is_some()
        && action.source_sha256.is_some()
        && action.expected_target_sha256.is_some()
}

fn is_kept_local_conversation_receipt(receipt: &ApplyReceipt) -> bool {
    receipt.action_type == RestoreActionType::Manual
        && receipt.status == ActionStatus::Skipped
        && receipt.resource_id.as_str().starts_with("claude:session:")
        && receipt.source_sha256.is_some()
        && receipt.target_sha256_after.is_some()
}

const CODEX_SESSION_RESOURCE_PREFIX: &str = "codex:session:";

fn codex_session_id(resource_id: &str) -> Option<&str> {
    resource_id
        .strip_prefix(CODEX_SESSION_RESOURCE_PREFIX)
        .filter(|thread_id| !thread_id.is_empty())
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

fn thread_capture_unavailable_detail(blocked_reason: Option<&str>) -> String {
    let Some(reason) = blocked_reason else {
        return "Session could not be read safely during this scan.".to_string();
    };
    if reason.contains(&format!("exceeds {} bytes", MAX_CONVERSATION_FILE_BYTES)) {
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

struct ThreadReview {
    entries: Vec<ThreadSyncEntry>,
    push_actions: Vec<ConversationPlanAction>,
    pull_actions: Vec<ConversationPlanAction>,
}

struct EmptyBundleBytes;

impl BundleBytes for EmptyBundleBytes {
    fn get(&mut self, logical_path: &LogicalPath) -> Result<&[u8], String> {
        Err(format!(
            "fresh bundle has no stored payload for '{}'",
            logical_path
        ))
    }
}

#[derive(Clone)]
struct LocalThreadReview {
    binding: Option<LocalConversationBinding>,
    binding_resolution: BindingResolution,
    resource_id: ResourceId,
    descriptor: Option<ResourceDescriptor>,
    conversation: provider_capture::CapturedConversation,
    transcript: provider_capture::CapturedFile,
}

fn bounded_file_expectation(path: &Path) -> Result<TargetExpectation, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TargetExpectation::Absent)
        }
        Err(error) => {
            return Err(format!(
                "inspect provider target '{}': {error}",
                path.display()
            ))
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "provider target '{}' is not a regular no-follow file",
            path.display()
        ));
    }
    if metadata.len() > MAX_CONVERSATION_REPAIR_BYTES {
        return Err(format!(
            "provider target '{}' exceeds the conversation limit",
            path.display()
        ));
    }
    let content = fs::read(path)
        .map_err(|error| format!("read provider target '{}': {error}", path.display()))?;
    Ok(TargetExpectation::Sha256(hex_digest(
        Sha256::digest(&content).as_slice(),
    )))
}

fn codex_target_from_logical_path(
    codex_home: &Path,
    logical_path: &str,
) -> Result<PathBuf, String> {
    let relative = logical_path
        .strip_prefix("state/codex/")
        .ok_or_else(|| "captured Codex transcript has an unexpected logical path".to_string())?;
    let target = codex_home.join(relative);
    validate_absolute_clean_path("Codex transcript target", &target.to_string_lossy())?;
    Ok(target)
}

fn reserved_codex_transcript_path(
    codex_home: &Path,
    provider_thread_id: &str,
    created_at: u64,
) -> Result<PathBuf, String> {
    let timestamp = Utc
        .timestamp_opt(created_at as i64, 0)
        .single()
        .ok_or_else(|| "conversation plan timestamp is out of range".to_string())?;
    let path = codex_home
        .join("sessions")
        .join(timestamp.format("%Y").to_string())
        .join(timestamp.format("%m").to_string())
        .join(timestamp.format("%d").to_string())
        .join(format!(
            "rollout-{}-{}.jsonl",
            timestamp.format("%Y-%m-%dT%H-%M-%S"),
            provider_thread_id
        ));
    validate_absolute_clean_path("reserved Codex transcript", &path.to_string_lossy())?;
    Ok(path)
}

fn codec_digests(lineage: &CodecLineage) -> (Option<String>, Option<String>) {
    match lineage {
        CodecLineage::Complete {
            lineage_digest,
            semantic_digest,
            ..
        } => (Some(lineage_digest.clone()), Some(semantic_digest.clone())),
        CodecLineage::Unknown { .. } => (None, None),
    }
}

fn thread_review(
    repository: &LocalStateRepository,
    snapshot: Option<&BundleSnapshot>,
    bytes: &mut dyn BundleBytes,
    captured: &provider_capture::CapturedResources,
    binding: &ProjectBinding,
    created_at: u64,
    expires_at: u64,
    build_push_actions: bool,
    build_pull_actions: bool,
) -> Result<ThreadReview, String> {
    let profile_id = binding
        .profile_ids
        .get(&Provider::Codex)
        .cloned()
        .ok_or_else(|| "Codex conversations require a mapped Codex profile".to_string())?;
    let machine = repository.load_bindings()?;
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == profile_id)
        .ok_or_else(|| "mapped Codex profile disappeared".to_string())?;
    let codex_home = PathBuf::from(&profile.canonical_path);
    let document = repository.load_conversation_bindings()?;
    let host_identity = repository.load_or_create_host_identity()?;
    let domain_resources = provider_capture::domain_resources(captured)?;
    // Threads owned by a journaled sync operation (for example a paused
    // Codex fork checkpoint) are review-blocked in both directions until
    // that operation is verified or canceled — never re-planned into a
    // second concurrent operation.
    let pending_resolution_threads = repository
        .load_conversation_sync_journal()?
        .operations
        .iter()
        .filter(|operation| {
            operation.local_project_id == binding.local_project_id
                && operation.provider == Provider::Codex
        })
        .map(|operation| operation.thread_id.clone())
        .collect::<BTreeSet<_>>();

    let scoped_bindings = document
        .bindings
        .iter()
        .filter(|conversation| {
            conversation.host_id == binding.host_id
                && conversation.replica_id == binding.replica_id
                && conversation.local_project_id == binding.local_project_id
                && conversation.provider_profile_id == profile_id
                && conversation
                    .provider_thread_id
                    .as_ref()
                    .is_none_or(|provider_id| {
                        !captured.excluded_codex_subagent_ids.contains(provider_id)
                    })
        })
        .cloned()
        .collect::<Vec<_>>();
    let bindings_by_thread = scoped_bindings
        .iter()
        .map(|conversation| (conversation.thread_id.clone(), conversation.clone()))
        .collect::<BTreeMap<_, _>>();
    let bindings_by_provider = scoped_bindings
        .iter()
        .filter_map(|conversation| {
            conversation
                .provider_thread_id
                .as_ref()
                .map(|provider_id| (provider_id.clone(), conversation.clone()))
        })
        .collect::<BTreeMap<_, _>>();

    let manifest = snapshot.map(|snapshot| &snapshot.manifest);
    let mut cloud_analyses = BTreeMap::new();
    let mut excluded_cloud_thread_ids = BTreeSet::new();
    for (thread_id, thread) in manifest
        .into_iter()
        .flat_map(|manifest| manifest.conversations.iter())
        .filter(|(_, thread)| thread.provider == Provider::Codex)
    {
        let analysis = thread
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .and_then(|file| {
                manifest
                    .and_then(|manifest| manifest.files.get(&file.logical_path))
                    .map(|entry| (file, entry))
            })
            .map(|(file, entry)| {
                cached_transcript_analysis(&entry.sha256, bytes, &file.logical_path)
            })
            .transpose()?;
        if analysis
            .as_ref()
            .is_some_and(|analysis| analysis.is_internal_subagent)
        {
            excluded_cloud_thread_ids.insert(thread_id.clone());
            continue;
        }
        cloud_analyses.insert(thread_id.clone(), analysis);
    }

    let mut local_threads = BTreeMap::<ThreadId, LocalThreadReview>::new();
    for (resource_id_text, conversation) in &captured.conversations {
        if conversation.provider != provider_capture::Provider::Codex {
            continue;
        }
        let provider_thread_id = conversation
            .source_thread_id
            .as_deref()
            .ok_or_else(|| "captured Codex conversation lacks an active thread id".to_string())?;
        let local_binding = bindings_by_provider.get(provider_thread_id).cloned();
        let (logical_thread_id, binding_resolution) = match &local_binding {
            Some(local_binding) => (local_binding.thread_id.clone(), BindingResolution::Bound),
            None => {
                let candidates = cloud_analyses
                    .iter()
                    .map(|(thread_id, analysis)| BindingRecoveryCandidate {
                        thread_id: thread_id.clone(),
                        chain: analysis
                            .as_ref()
                            .and_then(|analysis| analysis.lineage.chain()),
                    })
                    .collect::<Vec<_>>();
                match recover_unique_compatible_binding(conversation.lineage.chain(), &candidates) {
                    BindingRecoveryResult::Unique(thread_id) => {
                        (thread_id, BindingResolution::UniqueCompatibleAncestor)
                    }
                    BindingRecoveryResult::Ambiguous => (
                        initial_conversation_thread_id(
                            &binding.bundle_id,
                            Provider::Codex,
                            provider_thread_id,
                        )?,
                        BindingResolution::Ambiguous,
                    ),
                    BindingRecoveryResult::Missing | BindingRecoveryResult::Unknown => (
                        initial_conversation_thread_id(
                            &binding.bundle_id,
                            Provider::Codex,
                            provider_thread_id,
                        )?,
                        BindingResolution::Missing,
                    ),
                }
            }
        };
        if local_threads.contains_key(&logical_thread_id) {
            return Err(format!(
                "multiple local Codex conversations resolve to logical thread '{}'",
                logical_thread_id
            ));
        }
        let resource_id = ResourceId::parse(resource_id_text.clone())?;
        let descriptor = domain_resources.get(&resource_id).cloned();
        let transcript = captured
            .files
            .get(&conversation.transcript_logical_path)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "captured conversation '{}' has no transcript payload",
                    resource_id
                )
            })?;
        local_threads.insert(
            logical_thread_id.clone(),
            LocalThreadReview {
                binding: local_binding,
                binding_resolution,
                resource_id,
                descriptor,
                conversation: conversation.clone(),
                transcript,
            },
        );
    }

    let thread_ids = manifest
        .into_iter()
        .flat_map(|manifest| manifest.conversations.iter())
        .filter(|(_, thread)| thread.provider == Provider::Codex)
        .map(|(thread_id, _)| thread_id.clone())
        .chain(bindings_by_thread.keys().cloned())
        .chain(local_threads.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|thread_id| !excluded_cloud_thread_ids.contains(thread_id))
        .collect::<BTreeSet<_>>();
    let index_path = codex_home.join("session_index.jsonl");
    let index_expectation = build_pull_actions
        .then(|| bounded_file_expectation(&index_path))
        .transpose()?;
    let mut occupied_provider_ids = captured
        .conversations
        .values()
        .filter_map(|conversation| conversation.source_thread_id.clone())
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    let mut push_actions = Vec::new();
    let mut pull_actions = Vec::new();
    let (pinned_generation, pinned_commit_id, pinned_manifest_sha256) = snapshot
        .map(|snapshot| {
            (
                snapshot.head.generation,
                snapshot.head.commit_id.clone(),
                snapshot.head.manifest_sha256.clone(),
            )
        })
        .unwrap_or_else(absent_bundle_head_pin);

    for thread_id in thread_ids {
        let thread = manifest.and_then(|manifest| manifest.conversations.get(&thread_id));
        let local = local_threads.get(&thread_id);
        let local_binding = local
            .and_then(|local| local.binding.as_ref())
            .or_else(|| bindings_by_thread.get(&thread_id));
        let cloud_analysis = cloud_analyses
            .get(&thread_id)
            .and_then(|analysis| analysis.clone());
        let local_chain = local.and_then(|local| local.conversation.lineage.chain());
        let cloud_chain = cloud_analysis
            .as_ref()
            .and_then(|analysis| analysis.lineage.chain());
        let relation = match (local, cloud_analysis.as_ref()) {
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
        let local_capturable = local.is_some_and(|local| local.conversation.lineage.is_complete());
        let cloud_readable = cloud_analysis
            .as_ref()
            .is_some_and(|analysis| analysis.lineage.is_complete());
        let mut capabilities = ConversationCapabilities {
            binding: if local_binding.is_some() {
                BindingResolution::Bound
            } else {
                local
                    .map(|local| local.binding_resolution)
                    .unwrap_or(BindingResolution::Missing)
            },
            local_safely_capturable: local_capturable,
            cloud_safely_readable: cloud_readable,
            destination_codec_resumable: cloud_readable,
            provider_quiescent: true,
            structural_rewrite_only: false,
            resolution_pending: pending_resolution_threads.contains(&thread_id),
        };
        let push =
            default_conversation_decision(ConversationPlanDirection::Push, relation, capabilities);
        let mut pull =
            default_conversation_decision(ConversationPlanDirection::Pull, relation, capabilities);

        let cloud_descriptor = thread.and_then(|thread| {
            manifest.and_then(|manifest| manifest.resources.get(&thread.resource_id))
        });
        let source_resource_id = local
            .map(|local| local.resource_id.clone())
            .or_else(|| thread.map(|thread| thread.resource_id.clone()));
        let provider_thread_id = local
            .and_then(|local| local.conversation.source_thread_id.clone())
            .or_else(|| local_binding.and_then(|binding| binding.provider_thread_id.clone()));
        let descriptor = local
            .and_then(|local| local.descriptor.as_ref())
            .or(cloud_descriptor)
            .ok_or_else(|| format!("conversation thread '{}' has no descriptor", thread_id))?;
        let origin_label = thread
            .and_then(|thread| {
                thread
                    .last_updated_by_origin
                    .as_ref()
                    .or(thread.created_by_origin.as_ref())
            })
            .map(|origin| {
                let bundle_id = &snapshot
                    .expect("stored conversation requires a bundle snapshot")
                    .manifest
                    .bundle
                    .bundle_id;
                repository
                    .resolve_origin_host_label(bundle_id, &origin.origin_host_token)
                    .map(|(label, _)| label)
            })
            .transpose()?;
        let display_name = thread
            .and_then(|thread| thread.display_title.clone())
            .unwrap_or_else(|| descriptor.display_name.clone());

        // A divergence is only offered when its fork contract can be built
        // right now (captured local branch, resolvable codex binary, idle
        // source thread). Otherwise this one thread downgrades to a
        // provider-quiescence hold and the rest of the review proceeds.
        let mut divergence_plan = None;
        let mut divergence_unavailable: Option<String> = None;
        if build_pull_actions && pull.action == ConversationPlanActionType::ResolveDivergence {
            match prebuild_thread_divergence_plan(
                &thread_id,
                local,
                descriptor,
                thread,
                created_at,
                &codex_home,
                binding,
            ) {
                Ok(plan) => divergence_plan = Some(plan),
                Err(message) => {
                    capabilities.provider_quiescent = false;
                    pull = default_conversation_decision(
                        ConversationPlanDirection::Pull,
                        relation,
                        capabilities,
                    );
                    divergence_unavailable = Some(message);
                }
            }
        }

        entries.push(ThreadSyncEntry {
            thread_id: provider_thread_id
                .clone()
                .or_else(|| thread.and_then(|thread| thread.source_thread_id.clone()))
                .unwrap_or_else(|| thread_id.to_string()),
            resource_id: source_resource_id
                .clone()
                .unwrap_or_else(|| descriptor.resource_id.clone()),
            display_name,
            state: relation_sync_state(relation),
            local_present: local.is_some(),
            storage_present: thread.is_some(),
            status_detail: capabilities
                .resolution_pending
                .then(|| {
                    "A Codex fork checkpoint from an earlier Pull is pending. \
                     Verify or cancel it in the Pull review."
                        .to_string()
                })
                .or_else(|| {
                    divergence_unavailable.as_ref().map(|message| {
                        format!("Conflict resolution is unavailable right now: {message}")
                    })
                })
                .or_else(|| {
                    (relation == ConversationRelation::Unknown)
                        .then(|| "Conversation ancestry could not be proven safely.".to_string())
                }),
            local_updated_at: local.map(|local| local.transcript.source_mtime),
            storage_updated_at: thread.map(|thread| thread.updated_at),
            cloud_thread_id: Some(thread_id.clone()),
            forked_from: thread
                .and_then(|thread| thread.forked_from.clone())
                .or_else(|| local_binding.and_then(|binding| binding.forked_from.clone())),
            provider_thread_id: provider_thread_id.clone(),
            relation: Some(relation),
            push_decision: push.clone().into(),
            pull_decision: pull.clone().into(),
            origin_label,
        });

        if !build_push_actions && !build_pull_actions {
            continue;
        }
        let expected_local_target = local
            .map(|local| {
                TargetExpectation::Sha256(hex_digest(
                    Sha256::digest(&local.transcript.bytes).as_slice(),
                ))
            })
            .unwrap_or(TargetExpectation::Absent);
        let (push_lineage, push_semantic) = local
            .map(|local| codec_digests(&local.conversation.lineage))
            .or_else(|| {
                cloud_analysis
                    .as_ref()
                    .map(|analysis| codec_digests(&analysis.lineage))
            })
            .unwrap_or((None, None));
        let (pull_lineage, pull_semantic) = cloud_analysis
            .as_ref()
            .map(|analysis| codec_digests(&analysis.lineage))
            .unwrap_or((None, None));
        let required_codec = thread
            .map(|thread| thread.lineage_codec.clone())
            .or_else(|| local.map(|local| local.conversation.lineage_codec.clone()))
            .unwrap_or_else(|| CODEX_LINEAGE_CODEC_KEY.to_string());

        if build_push_actions {
            let push_action = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: ActionId::parse(generated_named_id("action")?)?,
                direction: ConversationPlanDirection::Push,
                local_chain,
                cloud_chain,
                capabilities,
                provider: Provider::Codex,
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_id.clone(),
                thread_id: thread_id.clone(),
                divergence: None,
                source_resource_id: source_resource_id.clone(),
                proposed_provider_thread_id: None,
                required_codec: required_codec.clone(),
                pinned_generation,
                pinned_commit_id: pinned_commit_id.clone(),
                pinned_manifest_sha256: pinned_manifest_sha256.clone(),
                pinned_lineage_digest: push_lineage,
                pinned_semantic_digest: push_semantic,
                expected_local_target: expected_local_target.clone(),
                expected_provider_state: Vec::new(),
                expected_profile_revision: profile.revision,
                expected_project_binding_revision: binding.revision,
                expected_host_identity_revision: host_identity.revision,
                expected_bindings_document_revision: document.revision,
                expected_binding_revision: local_binding.map(|binding| binding.revision),
                expires_at,
            })?;
            push_actions.push(push_action);
        }

        if build_pull_actions {
            let mut proposed_provider_thread_id = None;
            let mut expected_provider_state = Vec::new();
            let mut divergence = None;
            match pull.action {
                ConversationPlanActionType::FastForwardLocal
                | ConversationPlanActionType::ReplaceLocal => {
                    let local = local.ok_or_else(|| {
                        "local conversation replacement lacks a captured target".to_string()
                    })?;
                    let provider_id =
                        local.conversation.source_thread_id.clone().ok_or_else(|| {
                            "captured Codex conversation lacks a provider id".to_string()
                        })?;
                    let target = codex_target_from_logical_path(
                        &codex_home,
                        &local.transcript.logical_path,
                    )?;
                    proposed_provider_thread_id = Some(provider_id);
                    expected_provider_state.push(ProviderStateExpectation {
                        target_path: target.to_string_lossy().into_owned(),
                        expectation: bounded_file_expectation(&target)?,
                    });
                }
                ConversationPlanActionType::ResolveDivergence => {
                    let local = local.ok_or_else(|| {
                        "conversation divergence lacks a captured local branch".to_string()
                    })?;
                    let target = codex_target_from_logical_path(
                        &codex_home,
                        &local.transcript.logical_path,
                    )?;
                    expected_provider_state.push(ProviderStateExpectation {
                        target_path: target.to_string_lossy().into_owned(),
                        expectation: bounded_file_expectation(&target)?,
                    });
                    divergence = Some(divergence_plan.take().ok_or_else(|| {
                        "conversation divergence plan was not prebuilt".to_string()
                    })?);
                }
                ConversationPlanActionType::CreateLocal => {
                    let provider_id = loop {
                        let candidate = generated_uuid_v4()?;
                        if occupied_provider_ids.insert(candidate.clone()) {
                            break candidate;
                        }
                    };
                    let target =
                        reserved_codex_transcript_path(&codex_home, &provider_id, created_at)?;
                    if target.exists() {
                        return Err(
                            "reserved Codex transcript target is already occupied".to_string()
                        );
                    }
                    proposed_provider_thread_id = Some(provider_id);
                    expected_provider_state.push(ProviderStateExpectation {
                        target_path: target.to_string_lossy().into_owned(),
                        expectation: TargetExpectation::Absent,
                    });
                    expected_provider_state.push(ProviderStateExpectation {
                        target_path: index_path.to_string_lossy().into_owned(),
                        expectation: index_expectation
                            .clone()
                            .unwrap_or(TargetExpectation::Absent),
                    });
                }
                _ => {}
            }
            let pull_action = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: ActionId::parse(generated_named_id("action")?)?,
                direction: ConversationPlanDirection::Pull,
                local_chain,
                cloud_chain,
                capabilities,
                provider: Provider::Codex,
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_id.clone(),
                thread_id: thread_id.clone(),
                divergence,
                source_resource_id,
                proposed_provider_thread_id,
                required_codec,
                pinned_generation,
                pinned_commit_id: pinned_commit_id.clone(),
                pinned_manifest_sha256: pinned_manifest_sha256.clone(),
                pinned_lineage_digest: pull_lineage,
                pinned_semantic_digest: pull_semantic,
                expected_local_target,
                expected_provider_state,
                expected_profile_revision: profile.revision,
                expected_project_binding_revision: binding.revision,
                expected_host_identity_revision: host_identity.revision,
                expected_bindings_document_revision: document.revision,
                expected_binding_revision: local_binding.map(|binding| binding.revision),
                expires_at,
            })?;
            pull_actions.push(pull_action);
        }
    }
    entries.sort_by(|left, right| {
        left.cloud_thread_id
            .cmp(&right.cloud_thread_id)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    push_actions.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    pull_actions.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    Ok(ThreadReview {
        entries,
        push_actions,
        pull_actions,
    })
}

/// Build the reviewed fork contract for one divergent thread. A failure
/// here (no resolvable codex binary, active source turn, incomplete local
/// capture) is a per-thread hold, not a planning error: the caller
/// downgrades the thread to a quiescence hold and the review proceeds.
fn prebuild_thread_divergence_plan(
    thread_id: &ThreadId,
    local: Option<&LocalThreadReview>,
    descriptor: &ResourceDescriptor,
    thread: Option<&ConversationThread>,
    created_at: u64,
    codex_home: &Path,
    binding: &ProjectBinding,
) -> Result<ConversationDivergencePlan, String> {
    let local =
        local.ok_or_else(|| "conversation divergence lacks a captured local branch".to_string())?;
    let provider_id = local
        .conversation
        .source_thread_id
        .as_deref()
        .ok_or_else(|| "captured Codex conversation lacks a provider id".to_string())?;
    let (local_lineage, local_semantic) = codec_digests(&local.conversation.lineage);
    let local_display_name = local
        .descriptor
        .as_ref()
        .unwrap_or(descriptor)
        .display_name
        .as_str();
    let explicit_local_title = (local_display_name != provider_id).then_some(local_display_name);
    let local_summary = chat_history::first_codex_user_summary(&local.transcript.bytes);
    build_divergence_plan(
        thread_id,
        provider_id,
        local_lineage
            .as_deref()
            .ok_or_else(|| "conversation divergence lacks local lineage".to_string())?,
        local_semantic
            .as_deref()
            .ok_or_else(|| "conversation divergence lacks local semantics".to_string())?,
        explicit_local_title,
        thread.and_then(|thread| thread.display_title.as_deref()),
        local_summary.as_deref(),
        created_at,
        codex_home,
        Path::new(&binding.project_root),
    )
}

#[cfg(test)]
pub(crate) fn thread_entries(
    repository: &LocalStateRepository,
    snapshot: &BundleSnapshot,
    bytes: &mut dyn BundleBytes,
    captured: &provider_capture::CapturedResources,
    binding: &ProjectBinding,
) -> Result<Vec<ThreadSyncEntry>, String> {
    let created_at = now_secs();
    Ok(thread_review(
        repository,
        Some(snapshot),
        bytes,
        captured,
        binding,
        created_at,
        created_at.saturating_add(PLAN_LIFETIME_SECS),
        false,
        false,
    )?
    .entries)
}

fn get_project_thread_sync_comparison_with_repository(
    repository: &LocalStateRepository,
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
    let discovery = provider_capture::discover_full(&capture_request)?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let current = engine.inspect_optional(&project.bundle_id)?;
    Ok(build_thread_sync_comparison(
        repository,
        local_project_id,
        storage_id,
        storage_name,
        &project,
        &binding,
        &capture_request,
        &discovery,
        &engine,
        current.as_ref(),
        false,
    )?
    .comparison)
}

struct ThreadComparisonBuild {
    comparison: ThreadSyncComparison,
    push_actions: Vec<ConversationPlanAction>,
}

/// Thread-comparison section over a shared Discovery and one already-read
/// remote snapshot. The Codex transcripts are read and hashed here, from the
/// same walk the other sections use.
#[allow(clippy::too_many_arguments)]
fn build_thread_sync_comparison(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    storage_name: String,
    project: &LocalProjectRegistration,
    binding: &ProjectBinding,
    capture_request: &CaptureRequest,
    discovery: &provider_capture::Discovery,
    engine: &StorageEngine,
    current: Option<&BundleSnapshot>,
    build_push_actions: bool,
) -> Result<ThreadComparisonBuild, String> {
    let discovered = discovery.project_inventory();
    let mut warnings = discovered.warnings.clone();
    let selected_resource_ids = discovered
        .resources
        .iter()
        .filter(|candidate| {
            candidate.kind == CaptureResourceKind::Conversation
                && candidate.provider == Some(CaptureProvider::Codex)
        })
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let captured = provider_capture::capture_from_discovery(
        capture_request,
        discovery,
        &selected_resource_ids,
    )?;
    warnings.extend(captured.warnings.clone());

    let created_at = now_secs();
    let mut review = match current {
        Some(snapshot) => {
            let mut bytes = engine.byte_source(snapshot, Some(object_cache()), None);
            thread_review(
                repository,
                Some(snapshot),
                &mut bytes,
                &captured,
                binding,
                created_at,
                created_at.saturating_add(PLAN_LIFETIME_SECS),
                build_push_actions,
                false,
            )?
        }
        None => {
            let mut bytes = EmptyBundleBytes;
            thread_review(
                repository,
                None,
                &mut bytes,
                &captured,
                binding,
                created_at,
                created_at.saturating_add(PLAN_LIFETIME_SECS),
                build_push_actions,
                false,
            )?
        }
    };

    let represented_provider_ids = review
        .entries
        .iter()
        .filter_map(|entry| entry.provider_thread_id.clone())
        .collect::<BTreeSet<_>>();
    for candidate in discovered.resources.iter().filter(|candidate| {
        candidate.kind == CaptureResourceKind::Conversation
            && candidate.provider == Some(CaptureProvider::Codex)
    }) {
        let provider_thread_id = codex_session_id(&candidate.resource_id)
            .unwrap_or(&candidate.display_name)
            .to_string();
        if represented_provider_ids.contains(&provider_thread_id) {
            continue;
        }
        let detail = thread_capture_unavailable_detail(candidate.blocked_reason.as_deref());
        let blocked = ThreadSyncDecision {
            action: ConversationPlanActionType::Blocked,
            reason_code: ConversationReasonCode::CodecUnsupported,
            selectable: false,
            selected_by_default: false,
            requires_explicit_approval: false,
            blocked_reason: Some(detail.clone()),
        };
        review.entries.push(ThreadSyncEntry {
            thread_id: provider_thread_id.clone(),
            resource_id: ResourceId::parse(candidate.resource_id.clone())?,
            display_name: candidate.display_name.clone(),
            state: ThreadSyncState::Unavailable,
            local_present: true,
            storage_present: false,
            status_detail: Some(detail),
            local_updated_at: None,
            storage_updated_at: None,
            cloud_thread_id: None,
            forked_from: None,
            provider_thread_id: Some(provider_thread_id),
            relation: Some(ConversationRelation::Unknown),
            push_decision: blocked.clone(),
            pull_decision: blocked,
            origin_label: None,
        });
    }

    review.entries.sort_by(|left, right| {
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
    let mut counts = ThreadSyncCounts::default();
    for entry in &review.entries {
        count_thread_sync_state(&mut counts, entry.state);
    }
    warnings.sort();
    warnings.dedup();

    Ok(ThreadComparisonBuild {
        comparison: ThreadSyncComparison {
            project_id: local_project_id.clone(),
            storage_id: storage_id.clone(),
            storage_name,
            generation: current.map(|snapshot| snapshot.head.generation),
            base_generation: project
                .recipe_bases
                .get(storage_id)
                .map(|base| base.generation),
            compared_at: created_at,
            entries: review.entries,
            counts,
            warnings,
        },
        push_actions: review.push_actions,
    })
}
fn get_bundle_status_with_repository(
    repository: &LocalStateRepository,
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
    let discovery = provider_capture::discover_full(&capture_request)?;
    let discovered = discovery.project_inventory();
    project = persist_auto_selected_conversations(
        repository,
        &project.local_project_id,
        &discovered.resources,
        discovery.excluded_codex_subagent_resource_ids(),
    )?;
    let mut effective_recipe = destination_recipe.unwrap_or_else(|| project.recipe.clone());
    remove_excluded_codex_subagents_from_recipe(
        &mut effective_recipe,
        discovery.excluded_codex_subagent_resource_ids(),
    );
    let capture = provider_capture::capture_recipe_from_discovery(
        &capture_request,
        &discovery,
        &effective_recipe,
    )?;
    let local_resources = provider_capture::domain_resources(&capture)?;
    let unavailable: BTreeSet<_> = capture
        .unavailable_resource_ids
        .iter()
        .map(|id| ResourceId::parse(id.clone()))
        .collect::<Result<_, _>>()?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let remote = engine.inspect_optional(&project.bundle_id)?;
    let generation = remote.as_ref().map(|snapshot| snapshot.head.generation);
    let remote_resources = remote.as_ref().map(|snapshot| &snapshot.manifest.resources);
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

/// One-call preparation for the Push dialog: a single provider walk, one
/// engine construction, and one remote head+manifest read feed every section.
/// Sections that can degrade (thread comparison, capabilities, project files)
/// carry per-section errors instead of failing the whole dialog, preserving
/// the storage-offline behavior of the standalone commands.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushDestinationPullReason {
    RemoteWithoutBase,
    BindingChanged,
    RemoteAdvanced,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PushDestinationStatus {
    Ready {
        #[serde(skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
    },
    RemoteHistoryMissing {
        base_generation: u64,
    },
    RequiresPull {
        reason: PushDestinationPullReason,
        remote_generation: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_generation: Option<u64>,
    },
    Unavailable {
        message: String,
    },
}

fn classify_push_destination(
    observed_head: Option<&BundleHead>,
    base: Option<&RecipeBase>,
    binding_revision: u64,
) -> PushDestinationStatus {
    match (observed_head, base) {
        (None, None) => PushDestinationStatus::Ready { generation: None },
        (None, Some(base)) => PushDestinationStatus::RemoteHistoryMissing {
            base_generation: base.generation,
        },
        (Some(head), None) => PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteWithoutBase,
            remote_generation: head.generation,
            base_generation: None,
        },
        (Some(head), Some(base)) if base.binding_revision != Some(binding_revision) => {
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::BindingChanged,
                remote_generation: head.generation,
                base_generation: Some(base.generation),
            }
        }
        (Some(head), Some(base))
            if head.generation == base.generation
                && head.manifest_sha256 == base.manifest_sha256 =>
        {
            PushDestinationStatus::Ready {
                generation: Some(head.generation),
            }
        }
        (Some(head), Some(base)) => PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteAdvanced,
            remote_generation: head.generation,
            base_generation: Some(base.generation),
        },
    }
}

fn push_destination_error(status: &PushDestinationStatus) -> Option<String> {
    match status {
        PushDestinationStatus::Ready { .. } => None,
        PushDestinationStatus::RemoteHistoryMissing { .. } => Some(
            "remote history is missing at the configured storage; if this is expected, unlink and relink the storage before pushing again"
                .to_string(),
        ),
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteWithoutBase,
            ..
        } => Some(
            "remote bundle already exists and this replica has no reviewed base; pull and review it before pushing"
                .to_string(),
        ),
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::BindingChanged,
            ..
        } => Some(
            "the project checkout or provider profile changed after the last reviewed base; pull and review before pushing"
                .to_string(),
        ),
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteAdvanced,
            ..
        } => Some(
            "remote bundle advanced since this replica's base; pull and review before pushing"
                .to_string(),
        ),
        PushDestinationStatus::Unavailable { message } => Some(message.clone()),
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct PushReviewContext {
    pub project: ProjectDetail,
    pub inventory: ResourceInventory,
    pub destination_status: PushDestinationStatus,
    pub thread_comparison: Option<ThreadSyncComparison>,
    pub thread_comparison_error: Option<String>,
    pub project_files: Option<ProjectContentInventory>,
    pub project_files_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_push_plan_id: Option<PlanId>,
    #[serde(default)]
    pub conversation_action_ids: BTreeMap<ResourceId, ActionId>,
    pub prepared_at: u64,
}

fn prepare_push_review_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<PushReviewContext, String> {
    // The review token minted here is what a later push validates, so mint it
    // while pushes, pulls, and repairs are excluded.
    let _host_operation = repository.acquire_host_operation()?;
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
    let link = config
        .links
        .iter()
        .find(|link| link.local_project_id == *local_project_id && link.storage_id == *storage_id)
        .cloned()
        .ok_or_else(|| "project storage link disappeared".to_string())?;
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

    // Project content stays out of the dialog-open walk: eligible (non-Git)
    // projects defer it to the explicit Scan, and Git-managed projects get a
    // storage-side inventory below without any local content walk.
    let eligibility = project_file_sync_eligibility(&binding);
    let capture_request = capture_request_for_binding(repository, &binding)?;
    let discovery = provider_capture::discover_full(&capture_request)?;

    let inventory = build_resource_inventory(repository, local_project_id, &binding, &discovery)?;
    // Auto-selection may have advanced the stored recipe; the remaining
    // sections and the returned detail must see the persisted state.
    let project = repository
        .load_config()?
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;

    let remote = storage_engine(repository, storage_id).and_then(|(_, engine)| {
        let current = engine.inspect_optional(&project.bundle_id)?;
        Ok((engine, current))
    });
    let destination_status = match &remote {
        Ok((_, current)) => classify_push_destination(
            current.as_ref().map(|snapshot| &snapshot.head),
            project.recipe_bases.get(storage_id),
            binding.revision,
        ),
        Err(error) => PushDestinationStatus::Unavailable {
            message: error.clone(),
        },
    };

    let wants_thread_comparison = binding.profile_ids.contains_key(&Provider::Codex);
    // Mirrors the old dialog-open flow: only Git-managed projects surface a
    // stored project-files inventory here (remote/base entries, no local
    // walk); eligible projects use the explicit Scan button instead.
    let wants_project_files = eligibility.state == ProjectFileSyncEligibilityState::GitManaged;
    let mut thread_comparison = None;
    let mut thread_comparison_error = None;
    let mut project_files = None;
    let mut project_files_error = None;
    let mut conversation_push_plan_id = None;
    let mut conversation_action_ids = BTreeMap::new();
    match &remote {
        Ok((engine, current)) => {
            if wants_thread_comparison {
                match build_thread_sync_comparison(
                    repository,
                    local_project_id,
                    storage_id,
                    storage_name.clone(),
                    &project,
                    &binding,
                    &capture_request,
                    &discovery,
                    engine,
                    current.as_ref(),
                    true,
                ) {
                    Ok(build) => {
                        thread_comparison = Some(build.comparison);
                        if let Some(first) = build.push_actions.first() {
                            let plan_id = PlanId::parse(generated_named_id("plan")?)?;
                            let plan = ConversationPushPlan {
                                schema_version: PUSH_PLAN_SCHEMA,
                                plan_id: plan_id.clone(),
                                storage_id: storage_id.clone(),
                                bundle_id: project.bundle_id.clone(),
                                host_id: binding.host_id.clone(),
                                replica_id: binding.replica_id.clone(),
                                local_project_id: binding.local_project_id.clone(),
                                provider_profile_id: first.provider_profile_id.clone(),
                                expected_host_identity_revision: first
                                    .expected_host_identity_revision,
                                expected_project_binding_revision: first
                                    .expected_project_binding_revision,
                                expected_bindings_document_revision: first
                                    .expected_bindings_document_revision,
                                expected_profile_revision: first.expected_profile_revision,
                                generation: first.pinned_generation,
                                commit_id: first.pinned_commit_id.clone(),
                                manifest_sha256: first.pinned_manifest_sha256.clone(),
                                created_at: thread_comparison
                                    .as_ref()
                                    .map(|comparison| comparison.compared_at)
                                    .unwrap_or_else(now_secs),
                                expires_at: first.expires_at,
                                actions: build.push_actions,
                            };
                            plan.validate()?;
                            conversation_action_ids = plan
                                .actions
                                .iter()
                                .filter_map(|action| {
                                    action.source_resource_id.as_ref().map(|resource_id| {
                                        (resource_id.clone(), action.action_id.clone())
                                    })
                                })
                                .collect();
                            repository.save_conversation_push_plan(&plan)?;
                            conversation_push_plan_id = Some(plan_id);
                        }
                    }
                    Err(error) => thread_comparison_error = Some(error),
                }
            }
            if wants_project_files {
                match build_project_files_inventory(
                    local_project_id,
                    storage_id,
                    &project,
                    &link,
                    &binding,
                    eligibility.clone(),
                    engine,
                    current.as_ref(),
                    // Git-managed inventories never walk local content, so the
                    // shared discovery (walked without it) is not offered.
                    None,
                ) {
                    Ok(inventory) => project_files = Some(inventory),
                    Err(error) => project_files_error = Some(error),
                }
            }
        }
        Err(error) => {
            if wants_thread_comparison {
                thread_comparison_error = Some(error.clone());
            }
            if wants_project_files {
                project_files_error = Some(error.clone());
            }
        }
    }

    let detail = get_project_with_repository(repository, local_project_id)?
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;

    Ok(PushReviewContext {
        project: detail,
        inventory,
        destination_status,
        thread_comparison,
        thread_comparison_error,
        project_files,
        project_files_error,
        conversation_push_plan_id,
        conversation_action_ids,
        prepared_at: now_secs(),
    })
}

fn get_push_destination_status_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<PushDestinationStatus, String> {
    let _host_operation = repository.acquire_host_operation()?;
    let config = repository.load_config()?;
    let project = config
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    require_project_link(&config, &project, storage_id)?;
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
    let (_, engine) = storage_engine(repository, storage_id)?;
    let current = engine.inspect_optional(&project.bundle_id)?;
    Ok(classify_push_destination(
        current.as_ref().map(|snapshot| &snapshot.head),
        project.recipe_bases.get(storage_id),
        binding.revision,
    ))
}

#[tauri::command]
pub async fn prepare_push_review(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<PushReviewContext, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        prepare_push_review_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[tauri::command]
pub async fn get_push_destination_status(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<PushDestinationStatus, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        get_push_destination_status_with_repository(&repository, &local_project_id, &storage_id)
    })
    .await
}

#[cfg(test)]
fn push_bundle_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectOperationResult, String> {
    push_bundle_with_recipe_with_repository(repository, local_project_id, storage_id, None)
}

#[cfg(test)]
fn push_bundle_with_recipe_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: Option<BundleRecipe>,
) -> Result<ProjectOperationResult, String> {
    let review = prepare_push_review_with_repository(repository, local_project_id, storage_id)?;
    let selected_resources = requested_recipe
        .as_ref()
        .map(|recipe| recipe.entries.keys().cloned().collect::<BTreeSet<_>>());
    let approved_conversation_action_ids = review
        .thread_comparison
        .as_ref()
        .into_iter()
        .flat_map(|comparison| comparison.entries.iter())
        .filter(|entry| {
            entry.push_decision.selected_by_default
                && selected_resources
                    .as_ref()
                    .is_none_or(|selected| selected.contains(&entry.resource_id))
        })
        .filter_map(|entry| {
            review
                .conversation_action_ids
                .get(&entry.resource_id)
                .cloned()
        })
        .collect();
    push_bundle_reviewed_with_repository(
        repository,
        local_project_id,
        storage_id,
        requested_recipe,
        ProjectContentPushReview::default(),
        review.conversation_push_plan_id,
        approved_conversation_action_ids,
        &mut |_| {},
    )
}

/// A divergent or blocked thread holds back only itself: its row is not
/// selectable, and its cloud version is retained untouched by the publish.
/// Every other approved thread still pushes. Approving a held row
/// explicitly is the only hard error.
fn selected_conversation_actions(
    plan: &ConversationPushPlan,
    approved_action_ids: &[ActionId],
) -> Result<Vec<ConversationPlanAction>, String> {
    let mut approved = BTreeSet::new();
    for action_id in approved_action_ids {
        if !approved.insert(action_id.clone()) {
            return Err(format!(
                "conversation Push action '{}' was approved more than once",
                action_id
            ));
        }
    }
    let known = plan
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = approved
        .iter()
        .find(|action_id| !known.contains(*action_id))
    {
        return Err(format!(
            "conversation Push action '{}' is not in the reviewed plan",
            unknown
        ));
    }
    let selected = plan
        .actions
        .iter()
        .filter(|action| approved.contains(&action.action_id))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(action) = selected.iter().find(|action| {
        !matches!(
            action.action,
            ConversationPlanActionType::FastForwardCloud
                | ConversationPlanActionType::CreateCloud
                | ConversationPlanActionType::ReplaceCloud
                | ConversationPlanActionType::TombstoneThread
        )
    }) {
        let instruction = match action.action {
            ConversationPlanActionType::RequirePull => {
                "; Pull and resolve the conversation before pushing"
            }
            ConversationPlanActionType::Blocked => {
                "; resolve the blocked conversation before pushing"
            }
            _ => " and cannot be selected",
        };
        return Err(format!(
            "conversation Push action '{}' is not pushable{instruction}",
            action.action_id
        ));
    }
    Ok(selected)
}

fn captured_conversation_for_action<'a>(
    captured: &'a provider_capture::CapturedResources,
    action: &ConversationPlanAction,
) -> Result<
    (
        &'a provider_capture::CapturedConversation,
        &'a provider_capture::CapturedFile,
    ),
    String,
> {
    let resource_id = action
        .source_resource_id
        .as_ref()
        .ok_or_else(|| "conversation Push action lacks a captured source".to_string())?;
    let conversation = captured
        .conversations
        .get(resource_id.as_str())
        .ok_or_else(|| {
            format!(
                "reviewed conversation source '{}' is no longer capturable",
                resource_id
            )
        })?;
    let transcript = captured
        .files
        .get(&conversation.transcript_logical_path)
        .ok_or_else(|| {
            format!(
                "reviewed conversation source '{}' lost its transcript",
                resource_id
            )
        })?;
    Ok((conversation, transcript))
}

fn conversation_thread_from_capture(
    previous: Option<&ConversationThread>,
    action: &ConversationPlanAction,
    result_thread_id: &ThreadId,
    local_binding: Option<&LocalConversationBinding>,
    descriptor: &ResourceDescriptor,
    conversation: &provider_capture::CapturedConversation,
    transcript: &provider_capture::CapturedFile,
    published_by: &PortableOrigin,
    updated_at: u64,
) -> Result<(ConversationThread, ResourceDescriptor, LogicalPath), String> {
    let resource_id = conversation_resource_id(action.provider, result_thread_id)?;
    let logical_path = conversation_transcript_path(action.provider, result_thread_id)?;
    let (lineage_status, head_message_id, message_count, lineage_digest, semantic_digest) =
        match &conversation.lineage {
            CodecLineage::Complete {
                chain,
                lineage_digest,
                semantic_digest,
            } => (
                ConversationLineageStatus::Complete,
                chain.last().map(|message| message.message_id.clone()),
                Some(chain.len() as u64),
                Some(lineage_digest.clone()),
                Some(semantic_digest.clone()),
            ),
            CodecLineage::Unknown { reason } => (
                ConversationLineageStatus::Unknown {
                    reason_code: reason.as_str().to_string(),
                },
                None,
                None,
                None,
                None,
            ),
        };
    let forked_from = match action.action {
        ConversationPlanActionType::CreateCloud => {
            local_binding.and_then(|binding| binding.forked_from.clone())
        }
        _ => previous.and_then(|thread| thread.forked_from.clone()),
    };
    let thread = ConversationThread {
        thread_id: result_thread_id.clone(),
        provider: action.provider,
        // A display name equal to the provider thread ID is the unnamed
        // default, not a user title. Publishing it would clobber a real
        // cloud title on the next fast-forward from a machine whose local
        // index no longer names the thread (Codex prunes its index).
        display_title: Some(descriptor.display_name.clone())
            .filter(|name| Some(name.as_str()) != conversation.source_thread_id.as_deref())
            .or_else(|| previous.and_then(|thread| thread.display_title.clone())),
        resource_id: resource_id.clone(),
        files: vec![ConversationFileRef {
            logical_path: logical_path.clone(),
            role: ConversationFileRole::Transcript,
        }],
        lineage_status,
        lineage_codec: conversation.lineage_codec.clone(),
        head_message_id,
        message_count,
        lineage_digest,
        semantic_digest,
        forked_from,
        source_thread_id: conversation.source_thread_id.clone(),
        created_by_origin: previous
            .and_then(|thread| thread.created_by_origin.clone())
            .or_else(|| Some(published_by.clone())),
        last_updated_by_origin: Some(published_by.clone()),
        created_at: previous
            .map(|thread| thread.created_at)
            .unwrap_or(updated_at),
        updated_at,
    };
    let mut portable_descriptor = descriptor.clone();
    portable_descriptor.resource_id = resource_id;
    portable_descriptor.kind = match action.provider {
        Provider::Codex => ResourceKind::CodexConversation,
        Provider::Claude => ResourceKind::ClaudeConversation,
    };
    portable_descriptor.provider = Some(action.provider);
    portable_descriptor.apply_policy = ApplyPolicy::SafeFile;
    portable_descriptor.metadata.insert(
        "content_sha256".to_string(),
        hex_digest(Sha256::digest(&transcript.bytes).as_slice()),
    );
    Ok((thread, portable_descriptor, logical_path))
}

fn generic_capture(
    captured: &provider_capture::CapturedResources,
    recipe: &BundleRecipe,
) -> (provider_capture::CapturedResources, BundleRecipe) {
    let conversation_ids = captured
        .resources
        .iter()
        .filter(|(_, resource)| {
            resource.descriptor.kind == CaptureResourceKind::Conversation
                && resource.descriptor.provider == Some(CaptureProvider::Codex)
        })
        .map(|(resource_id, _)| resource_id.clone())
        .collect::<BTreeSet<_>>();
    let mut generic = captured.clone();
    generic
        .resources
        .retain(|resource_id, _| !conversation_ids.contains(resource_id));
    generic
        .files
        .retain(|_, file| !conversation_ids.contains(&file.resource_id));
    generic
        .directories
        .retain(|_, directory| !conversation_ids.contains(&directory.resource_id));
    generic.conversations.clear();
    generic
        .unavailable_resource_ids
        .retain(|resource_id| !conversation_ids.contains(resource_id));
    let mut generic_recipe = recipe.clone();
    generic_recipe
        .entries
        .retain(|resource_id, _| !conversation_ids.contains(resource_id.as_str()));
    (generic, generic_recipe)
}

fn previous_generic_manifest(previous: &BundleManifest) -> BundleManifest {
    let conversation_ids = previous
        .conversations
        .values()
        .map(|thread| thread.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let mut generic = previous.clone();
    generic.conversations.clear();
    generic
        .resources
        .retain(|resource_id, _| !conversation_ids.contains(resource_id));
    generic
        .files
        .retain(|_, file| !conversation_ids.contains(&file.resource_id));
    generic
        .directories
        .retain(|_, directory| !conversation_ids.contains(&directory.resource_id));
    generic
        .recipe
        .entries
        .retain(|resource_id, _| !conversation_ids.contains(resource_id));
    generic.tombstones.retain(|_, tombstone| {
        !matches!(tombstone.target, TombstoneTarget::ConversationThread { .. })
    });
    generic
}

#[allow(clippy::too_many_arguments)]
fn build_publish_request(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    storage_id: &StorageId,
    project: &LocalProjectRegistration,
    binding: &ProjectBinding,
    recipe: &BundleRecipe,
    captured: &provider_capture::CapturedResources,
    previous: Option<&BundleSnapshot>,
    expected_head: PublishExpectation,
    actions: &[ConversationPlanAction],
    updated_at: u64,
) -> Result<(PublishBundleRequest, Vec<ConversationSyncOperation>), String> {
    let (origin_host_token, origin_replica_token) =
        repository.derive_origin_tokens(&project.bundle_id, &binding.replica_id)?;
    let published_by = PortableOrigin {
        origin_host_token,
        origin_replica_token,
        shared_label: None,
    };
    let (generic_capture, generic_recipe) = generic_capture(captured, recipe);
    let captured_descriptors = provider_capture::domain_resources(&generic_capture)?;
    let previous_manifest = previous.map(|snapshot| &snapshot.manifest);
    let generic_previous = previous_manifest.map(previous_generic_manifest);
    let (mut resources, mut retained_files, mut directories, mut tombstones) =
        super::bundle_engine::reconcile_manifest_content(
            &generic_recipe,
            &captured_descriptors,
            &generic_capture,
            generic_previous.as_ref(),
            updated_at,
        )?;
    let mut captured_files = BTreeMap::new();
    for captured_file in generic_capture.files.values() {
        let logical_path = LogicalPath::parse(captured_file.logical_path.clone())?;
        let resource_id = ResourceId::parse(captured_file.resource_id.clone())?;
        let digest = hex_digest(Sha256::digest(&captured_file.bytes).as_slice());
        let can_retain = retained_files.get(&logical_path).is_some_and(|entry| {
            entry.resource_id == resource_id
                && entry.sha256 == digest
                && entry.size == captured_file.bytes.len() as u64
                && entry.source_mtime == captured_file.source_mtime
                && entry.mode == Some(captured_file.mode)
        });
        if !can_retain {
            retained_files.remove(&logical_path);
            captured_files.insert(
                logical_path,
                CapturedBundleFile {
                    resource_id,
                    bytes: captured_file.bytes.clone(),
                    source_mtime: captured_file.source_mtime,
                    mode: Some(captured_file.mode),
                },
            );
        }
    }
    for captured_directory in generic_capture.directories.values() {
        let logical_path = LogicalPath::parse(captured_directory.logical_path.clone())?;
        directories.insert(
            logical_path,
            BundleDirectoryEntry {
                resource_id: ResourceId::parse(captured_directory.resource_id.clone())?,
                mode: Some(captured_directory.mode),
                source_mtime: captured_directory.source_mtime,
            },
        );
    }

    let mut conversations = previous_manifest
        .map(|manifest| manifest.conversations.clone())
        .unwrap_or_default();
    let conversation_resource_ids = conversations
        .values()
        .map(|thread| thread.resource_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(previous_manifest) = previous_manifest {
        for resource_id in &conversation_resource_ids {
            if let Some(descriptor) = previous_manifest.resources.get(resource_id) {
                resources.insert(resource_id.clone(), descriptor.clone());
            }
        }
        for (path, file) in &previous_manifest.files {
            if conversation_resource_ids.contains(&file.resource_id) {
                retained_files.insert(path.clone(), file.clone());
            }
        }
        for (key, tombstone) in &previous_manifest.tombstones {
            if matches!(tombstone.target, TombstoneTarget::ConversationThread { .. }) {
                tombstones.insert(key.clone(), tombstone.clone());
            }
        }
    }
    let domain_captured = provider_capture::domain_resources(captured)?;
    let document = repository.load_conversation_bindings()?;
    let mut operations = Vec::new();
    for action in actions {
        let (conversation, transcript) = captured_conversation_for_action(captured, action)?;
        let result_thread_id = action.thread_id.clone();
        let previous_thread = conversations.get(&result_thread_id);
        let local_binding = document.bindings.iter().find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.provider_profile_id
                && candidate.thread_id == action.thread_id
        });
        let descriptor = action
            .source_resource_id
            .as_ref()
            .and_then(|resource_id| domain_captured.get(resource_id))
            .ok_or_else(|| {
                format!(
                    "conversation Push action '{}' lost its descriptor",
                    action.action_id
                )
            })?;
        let (thread, descriptor, logical_path) = conversation_thread_from_capture(
            previous_thread,
            action,
            &result_thread_id,
            local_binding,
            descriptor,
            conversation,
            transcript,
            &published_by,
            updated_at,
        )?;
        if let Some(old_thread) = conversations.insert(result_thread_id.clone(), thread) {
            for file in old_thread.files {
                retained_files.remove(&file.logical_path);
                captured_files.remove(&file.logical_path);
            }
            resources.remove(&old_thread.resource_id);
        }
        let resource_id = descriptor.resource_id.clone();
        resources.insert(resource_id.clone(), descriptor);
        let digest = hex_digest(Sha256::digest(&transcript.bytes).as_slice());
        let retained_entry =
            previous_manifest.and_then(|manifest| manifest.files.get(&logical_path));
        let can_retain = retained_entry.is_some_and(|entry| {
            entry.resource_id == resource_id
                && entry.sha256 == digest
                && entry.size == transcript.bytes.len() as u64
                && entry.source_mtime == transcript.source_mtime
                && entry.mode == Some(transcript.mode)
        });
        if can_retain {
            retained_files.insert(logical_path.clone(), retained_entry.unwrap().clone());
        } else {
            retained_files.remove(&logical_path);
            captured_files.insert(
                logical_path.clone(),
                CapturedBundleFile {
                    resource_id: resource_id.clone(),
                    bytes: transcript.bytes.clone(),
                    source_mtime: transcript.source_mtime,
                    mode: Some(transcript.mode),
                },
            );
        }
        tombstones.retain(|_, tombstone| {
            !matches!(
                &tombstone.target,
                TombstoneTarget::ConversationThread { thread_id }
                    if thread_id == &result_thread_id
            )
        });
        let provider_thread_id = conversation
            .source_thread_id
            .clone()
            .ok_or_else(|| "captured Codex conversation lacks a provider id".to_string())?;
        operations.push(ConversationSyncOperation {
            operation_id: ConversationOperationId::parse(generated_named_id("operation")?)?,
            plan_id: Some(plan_id.clone()),
            plan_action_id: Some(action.action_id.clone()),
            direction: ConversationPlanDirection::Push,
            phase: ConversationSyncPhase::Prepared,
            storage_id: storage_id.clone(),
            bundle_id: project.bundle_id.clone(),
            generation: action.pinned_generation,
            commit_id: action.pinned_commit_id.clone(),
            manifest_sha256: action.pinned_manifest_sha256.clone(),
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: action.provider_profile_id.clone(),
            provider: action.provider,
            thread_id: action.thread_id.clone(),
            operation: ConversationSyncOperationKind::CloudPublication,
            binding_forked_from: local_binding.and_then(|binding| binding.forked_from.clone()),
            provider_thread_id: Some(provider_thread_id),
            materialization_state: ConversationMaterializationState::Resumable,
            lineage_digest: action.pinned_lineage_digest.clone(),
            semantic_digest: action.pinned_semantic_digest.clone(),
            expected_host_identity_revision: action.expected_host_identity_revision,
            expected_bindings_document_revision: action.expected_bindings_document_revision,
            expected_binding_revision: action.expected_binding_revision,
            expected_profile_revision: action.expected_profile_revision,
            expected_project_binding_revision: action.expected_project_binding_revision,
            expected_local_digest: match &action.expected_local_target {
                TargetExpectation::Sha256(digest) => Some(digest.clone()),
                TargetExpectation::Absent => None,
            },
            targets: Vec::new(),
            created_at: updated_at,
            updated_at,
        });
    }
    let mut final_recipe = generic_recipe;
    for thread in conversations.values() {
        final_recipe.entries.insert(
            thread.resource_id.clone(),
            RecipeEntry {
                resource_id: thread.resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
    }
    final_recipe.revision = recipe.revision;
    Ok((
        PublishBundleRequest {
            identity: BundleIdentity {
                bundle_id: project.bundle_id.clone(),
                display_name: project.display_name.clone(),
                kind: BundleKind::Project,
                repository_fingerprint: project.repository_fingerprint.clone(),
            },
            recipe: final_recipe,
            captured_with: {
                let mut captured_with = previous_manifest
                    .map(|manifest| manifest.captured_with.clone())
                    .unwrap_or(CapturedWith {
                        app_version: env!("CARGO_PKG_VERSION").to_string(),
                        codex_version: None,
                        claude_version: None,
                        codec_versions: BTreeMap::new(),
                    });
                captured_with.app_version = env!("CARGO_PKG_VERSION").to_string();
                captured_with
                    .codec_versions
                    .insert(CODEX_LINEAGE_CODEC_KEY.to_string(), CODEX_CODEC_VERSION);
                captured_with
            },
            published_by_origin: published_by,
            conversations,
            resources,
            retained_files,
            captured_files,
            directories,
            tombstones,
            expected_head,
            updated_at,
        },
        operations,
    ))
}

fn snapshot_contains_actions(
    snapshot: &BundleSnapshot,
    actions: &[ConversationPlanAction],
) -> bool {
    actions.iter().all(|action| {
        let result_thread_id = &action.thread_id;
        snapshot
            .manifest
            .conversations
            .get(result_thread_id)
            .is_some_and(|thread| {
                thread.lineage_digest == action.pinned_lineage_digest
                    && thread.semantic_digest == action.pinned_semantic_digest
            })
    })
}

#[allow(clippy::too_many_arguments)]
fn publish_reviewed(
    repository: &LocalStateRepository,
    engine: &StorageEngine,
    storage_id: &StorageId,
    project: &LocalProjectRegistration,
    binding: &ProjectBinding,
    recipe: &BundleRecipe,
    captured: &provider_capture::CapturedResources,
    observed: Option<(BundleSnapshot, super::bundle_engine::HeadToken)>,
    conversation_push_plan_id: Option<&PlanId>,
    approved_conversation_action_ids: &[ActionId],
    updated_at: u64,
    on_progress: &mut dyn FnMut(FetchProgress),
) -> Result<(bool, BundleSnapshot), String> {
    let previous = observed.as_ref().map(|(snapshot, _)| snapshot);
    let expected_head = observed
        .as_ref()
        .map(|(_, token)| PublishExpectation::Match(token.clone()))
        .unwrap_or(PublishExpectation::Absent);
    let (generation, commit_id, manifest_sha256) = previous
        .map(|snapshot| {
            (
                snapshot.head.generation,
                snapshot.head.commit_id.clone(),
                snapshot.head.manifest_sha256.clone(),
            )
        })
        .unwrap_or_else(absent_bundle_head_pin);
    let selected_actions = match conversation_push_plan_id {
        Some(plan_id) => {
            let plan = repository.load_conversation_push_plan(plan_id)?;
            plan.validate()?;
            if plan.storage_id != *storage_id
                || plan.bundle_id != project.bundle_id
                || plan.host_id != binding.host_id
                || plan.replica_id != binding.replica_id
                || plan.local_project_id != binding.local_project_id
                || plan.generation != generation
                || plan.commit_id != commit_id
                || plan.manifest_sha256 != manifest_sha256
                || updated_at < plan.created_at
                || updated_at > plan.expires_at
            {
                return Err("conversation Push plan is stale; refresh the Push review".to_string());
            }
            selected_conversation_actions(&plan, approved_conversation_action_ids)?
        }
        None if approved_conversation_action_ids.is_empty() => Vec::new(),
        None => {
            return Err("selected conversation Push actions lack their reviewed plan".to_string())
        }
    };
    let plan_id = conversation_push_plan_id
        .cloned()
        .unwrap_or(PlanId::parse(generated_named_id("plan")?)?);
    let document = repository.load_conversation_bindings()?;
    let machine = repository.load_bindings()?;
    let identity = repository.load_or_create_host_identity()?;
    for action in &selected_actions {
        let (conversation, transcript) = captured_conversation_for_action(captured, action)?;
        if action
            .source_resource_id
            .as_ref()
            .is_some_and(|resource_id| !recipe.entries.contains_key(resource_id))
        {
            return Err(format!(
                "conversation Push action '{}' is no longer selected",
                action.action_id
            ));
        }
        let local_binding = document.bindings.iter().find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.provider_profile_id
                && candidate.thread_id == action.thread_id
        });
        let profile = machine
            .profiles
            .iter()
            .find(|profile| profile.profile_id == action.provider_profile_id)
            .ok_or_else(|| "reviewed Codex profile disappeared".to_string())?;
        let (lineage_digest, semantic_digest) = codec_digests(&conversation.lineage);
        let context = ConversationApplyContext {
            now: updated_at,
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: action.provider_profile_id.clone(),
            host_identity_revision: identity.revision,
            generation,
            commit_id: commit_id.clone(),
            manifest_sha256: manifest_sha256.clone(),
            lineage_digest,
            semantic_digest,
            local_target: TargetExpectation::Sha256(hex_digest(
                Sha256::digest(&transcript.bytes).as_slice(),
            )),
            provider_state: BTreeMap::new(),
            profile_revision: profile.revision,
            project_binding_revision: binding.revision,
            bindings_document_revision: document.revision,
            binding_revision: local_binding.map(|binding| binding.revision),
            codec: conversation.lineage_codec.clone(),
            provider_quiescent: true,
        };
        validate_conversation_action_pins(action, &context).map_err(|pin| {
            format!(
                "conversation Push action '{}' is stale at {:?}",
                action.action_id, pin
            )
        })?;
    }
    let (request, operations) = build_publish_request(
        repository,
        &plan_id,
        storage_id,
        project,
        binding,
        recipe,
        captured,
        previous,
        expected_head,
        &selected_actions,
        updated_at,
    )?;
    if operations.is_empty() {
        let outcome = engine.publish_with_progress(request, on_progress)?;
        if let Some(plan_id) = conversation_push_plan_id {
            let _ = repository.discard_conversation_push_plan(plan_id);
        }
        return Ok((outcome.published, outcome.snapshot));
    }
    let published = std::cell::Cell::new(false);
    let mut failpoints = NoConversationFailpoints;
    apply_conversation_cloud_publications_locked(
        repository,
        operations,
        || {
            let outcome = engine.publish_with_progress(request, on_progress)?;
            published.set(outcome.published);
            Ok(ConversationCloudPublication {
                generation: outcome.snapshot.head.generation,
                commit_id: outcome.snapshot.head.commit_id,
                manifest_sha256: outcome.snapshot.head.manifest_sha256,
            })
        },
        || {
            let snapshot = engine.inspect(&project.bundle_id)?;
            Ok(
                snapshot_contains_actions(&snapshot, &selected_actions).then(|| {
                    published.set(previous.is_none_or(|previous| {
                        snapshot.head.generation != previous.head.generation
                            || snapshot.head.commit_id != previous.head.commit_id
                    }));
                    ConversationCloudPublication {
                        generation: snapshot.head.generation,
                        commit_id: snapshot.head.commit_id,
                        manifest_sha256: snapshot.head.manifest_sha256,
                    }
                }),
            )
        },
        &mut failpoints,
    )?;
    let snapshot = engine.inspect(&project.bundle_id)?;
    if !snapshot_contains_actions(&snapshot, &selected_actions) {
        return Err(
            "published bundle head does not contain the reviewed conversations".to_string(),
        );
    }
    if let Some(plan_id) = conversation_push_plan_id {
        let _ = repository.discard_conversation_push_plan(plan_id);
    }
    Ok((published.get(), snapshot))
}

fn push_bundle_reviewed_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: Option<BundleRecipe>,
    project_content_review: ProjectContentPushReview,
    conversation_push_plan_id: Option<PlanId>,
    approved_conversation_action_ids: Vec<ActionId>,
    on_progress: &mut dyn FnMut(FetchProgress),
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
    // One provider walk feeds selection persistence, the review-token
    // freshness check, and the byte capture below.
    let wants_project_content = project_content_review.review_token.is_some();
    let eligibility = project_file_sync_eligibility(&binding);
    if wants_project_content && eligibility.state != ProjectFileSyncEligibilityState::Eligible {
        return Err(eligibility.reason);
    }
    let mut capture_request = capture_request_for_binding(repository, &binding)?;
    capture_request.include_project_content = wants_project_content;
    let discovery = provider_capture::discover_full(&capture_request)?;
    let discovered = discovery.project_inventory();
    let local_codex_conversations = discovered
        .resources
        .iter()
        .filter(|candidate| {
            candidate.kind == CaptureResourceKind::Conversation
                && candidate.provider == Some(CaptureProvider::Codex)
        })
        .collect::<Vec<_>>();
    if let Some(candidate) = local_codex_conversations
        .iter()
        .find(|candidate| candidate.blocked_reason.is_some())
    {
        return Err(format!(
            "conversation Push is blocked because '{}' cannot be reviewed safely; resolve the local capture problem before pushing",
            candidate.display_name
        ));
    }
    project = persist_auto_selected_conversations(
        repository,
        &project.local_project_id,
        &discovered.resources,
        discovery.excluded_codex_subagent_resource_ids(),
    )?;
    let mut push_recipe = match requested_recipe {
        Some(requested) => {
            requested.validate()?;
            requested
        }
        None => project.recipe.clone(),
    };
    remove_excluded_codex_subagents_from_recipe(
        &mut push_recipe,
        discovery.excluded_codex_subagent_resource_ids(),
    );
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
    let (_, engine) = storage_engine(repository, storage_id)?;
    let observed = engine.inspect_optional_with_token(&project.bundle_id)?;
    let has_stored_codex_conversation = observed.as_ref().is_some_and(|(snapshot, _)| {
        snapshot
            .manifest
            .conversations
            .values()
            .any(|thread| thread.provider == Provider::Codex)
    });
    if conversation_push_plan_id.is_none()
        && (!local_codex_conversations.is_empty() || has_stored_codex_conversation)
    {
        return Err(
            "Codex conversation Push requires a current Push review; refresh and review before pushing"
                .to_string(),
        );
    }
    let mut reviewed_inventory = None;
    if let Some(review_token) = project_content_review.review_token.as_deref() {
        // The freshness check derives from the same walk instant and the same
        // observed head as the capture and CAS expectation below.
        let inventory = build_project_files_inventory(
            local_project_id,
            storage_id,
            &project,
            &stored_link,
            &binding,
            eligibility.clone(),
            &engine,
            observed.as_ref().map(|(snapshot, _)| snapshot),
            Some(&discovery),
        )?;
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
    let capture = provider_capture::capture_recipe_from_discovery(
        &capture_request,
        &discovery,
        &push_recipe,
    )?;
    let destination_status = classify_push_destination(
        observed.as_ref().map(|(snapshot, _)| &snapshot.head),
        project.recipe_bases.get(storage_id),
        binding.revision,
    );
    if let Some(error) = push_destination_error(&destination_status) {
        return Err(error);
    }
    let pushed_at = now_secs();
    let (was_published, published_snapshot) = publish_reviewed(
        repository,
        &engine,
        storage_id,
        &project,
        &binding,
        &push_recipe,
        &capture,
        observed,
        conversation_push_plan_id.as_ref(),
        &approved_conversation_action_ids,
        pushed_at,
        on_progress,
    )?;
    let generation = published_snapshot.head.generation;
    let commit_id = published_snapshot.head.commit_id.clone();
    let manifest_sha256 = published_snapshot.head.manifest_sha256.clone();
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
    let state = if was_published { "synced" } else { "unchanged" };
    let results = published_snapshot
        .manifest
        .resources
        .keys()
        .cloned()
        .map(|resource_id| OperationResourceResult {
            resource_id,
            state: state.to_string(),
            message: None,
        })
        .collect::<Vec<_>>();
    Ok(ProjectOperationResult {
        success: true,
        message: if was_published {
            format!("Published generation {}", generation)
        } else {
            format!("Already up to date — generation {}", generation)
        },
        operation_id: Some(commit_id),
        resources_changed: Some(if was_published { results.len() } else { 0 }),
        generation: Some(generation),
        results,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn plan_bundle_restore_with_repository(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) -> Result<RestorePlan, String> {
    plan_bundle_restore_with_progress(
        repository,
        storage_id,
        bundle_id,
        binding,
        &mut |_| {},
        &mut |_, _| {},
    )
}

fn plan_bundle_restore_with_progress(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
    on_progress: &mut dyn FnMut(FetchProgress),
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<RestorePlan, String> {
    let phase_started = Instant::now();
    let _host_operation = repository.acquire_host_operation()?;
    let binding = require_current_binding(repository, binding)?;
    if &binding.bundle_id != bundle_id {
        return Err("binding and requested bundle IDs differ".to_string());
    }
    require_codex_conversation_paths_ready(repository, &binding.local_project_id)?;
    let (engine, snapshot) = inspect_from_storage(repository, storage_id, bundle_id)?;
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
            if snapshot.head.generation == base.generation
                && snapshot.head.manifest_sha256 == base.manifest_sha256 =>
        {
            Some(snapshot.manifest.clone())
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
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != binding.host_id {
        return Err("host identity changed while preparing the Pull review".to_string());
    }
    let mut plan = engine.build_restore_plan_from_snapshot(
        &snapshot,
        &binding,
        now_secs(),
        identity.revision,
    )?;
    on_phase("load bundle and validate inputs", phase_started.elapsed());
    let phase_started = Instant::now();
    if snapshot
        .manifest
        .conversations
        .values()
        .any(|thread| thread.provider == Provider::Codex)
    {
        let mut byte_source =
            engine.byte_source(&snapshot, Some(object_cache()), Some(on_progress));
        let capture_request = capture_request_for_binding(repository, &binding)?;
        let discovery = provider_capture::discover_full(&capture_request)?;
        let selected_resource_ids = discovery
            .project_inventory()
            .resources
            .into_iter()
            .filter(|candidate| {
                candidate.kind == CaptureResourceKind::Conversation
                    && candidate.provider == Some(CaptureProvider::Codex)
            })
            .map(|candidate| candidate.resource_id)
            .collect::<BTreeSet<_>>();
        let captured = provider_capture::capture_from_discovery(
            &capture_request,
            &discovery,
            &selected_resource_ids,
        )?;
        let review = thread_review(
            repository,
            Some(&snapshot),
            &mut byte_source,
            &captured,
            &binding,
            plan.created_at,
            plan.expires_at,
            false,
            true,
        )?;
        plan.conversation_actions = review.pull_actions;
    }
    on_phase("review Codex conversations", phase_started.elapsed());
    let phase_started = Instant::now();
    plan.actions.retain(|action| {
        if !is_conversation_keep_local_action(action, &snapshot.manifest) {
            return true;
        }
        let current_storage_digest =
            single_manifest_resource_file_digest(&snapshot.manifest, &action.resource_id);
        let known_storage_digest = base_manifest.as_ref().and_then(|manifest| {
            single_manifest_resource_file_digest(manifest, &action.resource_id)
        });
        current_storage_digest != known_storage_digest
    });
    plan.project_content_eligibility = Some(project_file_sync_eligibility(&binding));
    plan.validate()?;
    repository.save_restore_plan(&plan)?;
    on_phase("finalize and save review", phase_started.elapsed());
    Ok(plan)
}

#[cfg_attr(not(test), allow(dead_code))]
fn apply_bundle_restore_with_repository(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    approved_action_ids: Vec<ActionId>,
) -> Result<RestoreResult, String> {
    let plan = repository.load_restore_plan(plan_id)?;
    let selected = approved_action_ids.iter().collect::<BTreeSet<_>>();
    let conversation_resolutions = plan
        .conversation_actions
        .iter()
        .filter(|action| {
            action.action == ConversationPlanActionType::ResolveDivergence
                && selected.contains(&action.action_id)
        })
        .map(|action| ConversationResolutionSelection {
            action_id: action.action_id.clone(),
            resolution: ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            destructive_approved: false,
        })
        .collect();
    apply_bundle_restore_with_progress(
        repository,
        plan_id,
        approved_action_ids,
        conversation_resolutions,
        &mut |_| {},
        &mut |_, _| {},
    )
}

struct PreparedPullConversation {
    action: ConversationPlanAction,
    resolution: Option<ConversationDivergenceResolution>,
    cloud_transcript: Vec<u8>,
    cloud_file_sha256: String,
    cloud_provider_thread_id: String,
    cloud_cwd: String,
    cloud_forked_from: Option<ThreadId>,
    cloud_display_title: Option<String>,
    cloud_updated_at: u64,
}

#[derive(Clone, Debug)]
struct SelectedPullConversation {
    action: ConversationPlanAction,
    resolution: Option<ConversationDivergenceResolution>,
}

fn capture_local_codex_conversations(
    repository: &LocalStateRepository,
    binding: &ProjectBinding,
) -> Result<provider_capture::CapturedResources, String> {
    let capture_request = capture_request_for_binding(repository, binding)?;
    let discovery = provider_capture::discover_full(&capture_request)?;
    let selected = discovery
        .project_inventory()
        .resources
        .into_iter()
        .filter(|candidate| {
            candidate.kind == CaptureResourceKind::Conversation
                && candidate.provider == Some(CaptureProvider::Codex)
        })
        .map(|candidate| candidate.resource_id)
        .collect::<BTreeSet<_>>();
    provider_capture::capture_from_discovery(&capture_request, &discovery, &selected)
}

fn local_target_expectation_for_action(
    action: &ConversationPlanAction,
    binding: &ProjectBinding,
    document: &super::domain::ConversationBindings,
    captured: &provider_capture::CapturedResources,
) -> TargetExpectation {
    let provider_thread_id = document
        .bindings
        .iter()
        .find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.provider_profile_id
                && candidate.thread_id == action.thread_id
        })
        .and_then(|candidate| candidate.provider_thread_id.as_deref());
    provider_thread_id
        .and_then(|provider_thread_id| {
            captured.conversations.values().find(|conversation| {
                conversation.provider == CaptureProvider::Codex
                    && conversation.source_thread_id.as_deref() == Some(provider_thread_id)
            })
        })
        .and_then(|conversation| captured.files.get(&conversation.transcript_logical_path))
        .map(|file| TargetExpectation::Sha256(hex_digest(Sha256::digest(&file.bytes).as_slice())))
        .unwrap_or(TargetExpectation::Absent)
}

struct PreparedPullConversationFailure {
    action: ConversationPlanAction,
    message: String,
}

/// Prepare every approved conversation action. Shared-state loads are
/// fatal, but each action prepares independently: one stale or unreadable
/// thread becomes a per-thread failure receipt instead of blocking the
/// other approved threads.
fn prepare_pull_conversation_actions(
    repository: &LocalStateRepository,
    binding: &ProjectBinding,
    snapshot: &BundleSnapshot,
    bytes: &mut dyn BundleBytes,
    actions: &[SelectedPullConversation],
    now: u64,
) -> Result<
    (
        Vec<PreparedPullConversation>,
        Vec<PreparedPullConversationFailure>,
    ),
    String,
> {
    if actions.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let captured = capture_local_codex_conversations(repository, binding)?;
    let document = repository.load_conversation_bindings()?;
    let machine = repository.load_bindings()?;
    let identity = repository.load_or_create_host_identity()?;
    let mut prepared = Vec::with_capacity(actions.len());
    let mut failures = Vec::new();
    let prepare_one = |selected: &SelectedPullConversation,
                       bytes: &mut dyn BundleBytes|
     -> Result<PreparedPullConversation, String> {
        let action = &selected.action;
        let thread = snapshot
            .manifest
            .conversations
            .get(&action.thread_id)
            .ok_or_else(|| {
                format!(
                    "reviewed cloud conversation '{}' disappeared",
                    action.thread_id
                )
            })?;
        let transcript_ref = thread
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .ok_or_else(|| {
                format!(
                    "cloud conversation '{}' lacks a transcript",
                    action.thread_id
                )
            })?;
        let cloud_file_sha256 = snapshot
            .manifest
            .files
            .get(&transcript_ref.logical_path)
            .ok_or_else(|| {
                format!(
                    "cloud conversation '{}' transcript is absent from the manifest",
                    action.thread_id
                )
            })?
            .sha256
            .clone();
        let cloud_transcript = bytes.get(&transcript_ref.logical_path)?.to_vec();
        let analysis = analyze_codex_transcript(&cloud_transcript);
        let cloud_provider_thread_id = analysis
            .source_thread_id
            .clone()
            .ok_or_else(|| "cloud Codex transcript lacks an active thread id".to_string())?;
        let cloud_cwd = analysis
            .source_cwd
            .clone()
            .ok_or_else(|| "cloud Codex transcript lacks a structural cwd".to_string())?;
        let (lineage_digest, semantic_digest) = codec_digests(&analysis.lineage);
        let local_binding = document.bindings.iter().find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.provider_profile_id
                && candidate.thread_id == action.thread_id
        });
        let profile = machine
            .profiles
            .iter()
            .find(|profile| profile.profile_id == action.provider_profile_id)
            .ok_or_else(|| "reviewed Codex profile disappeared".to_string())?;
        let provider_state = action
            .expected_provider_state
            .iter()
            .map(|target| {
                let current = bounded_file_expectation(Path::new(&target.target_path))?;
                Ok((target.target_path.clone(), current))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let context = ConversationApplyContext {
            now,
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: action.provider_profile_id.clone(),
            host_identity_revision: identity.revision,
            generation: snapshot.head.generation,
            commit_id: snapshot.head.commit_id.clone(),
            manifest_sha256: snapshot.head.manifest_sha256.clone(),
            lineage_digest,
            semantic_digest,
            local_target: local_target_expectation_for_action(
                action, binding, &document, &captured,
            ),
            provider_state,
            profile_revision: profile.revision,
            project_binding_revision: binding.revision,
            bindings_document_revision: document.revision,
            binding_revision: local_binding.map(|binding| binding.revision),
            codec: thread.lineage_codec.clone(),
            provider_quiescent: true,
        };
        validate_conversation_action_pins(action, &context).map_err(|pin| {
            format!(
                "conversation Pull action '{}' is stale at {:?}",
                action.action_id, pin
            )
        })?;
        Ok(PreparedPullConversation {
            action: action.clone(),
            resolution: selected.resolution,
            cloud_transcript,
            cloud_file_sha256,
            cloud_provider_thread_id,
            cloud_cwd,
            cloud_forked_from: thread.forked_from.clone(),
            cloud_display_title: thread.display_title.clone(),
            cloud_updated_at: thread.updated_at,
        })
    };
    for selected in actions {
        match prepare_one(selected, bytes) {
            Ok(conversation) => prepared.push(conversation),
            Err(message) => failures.push(PreparedPullConversationFailure {
                action: selected.action.clone(),
                message,
            }),
        }
    }
    Ok((prepared, failures))
}

/// A per-thread conversation failure keeps its receipt so the rest of the
/// Pull pass stays applied and retryable action-by-action.
fn failed_conversation_receipt(
    action: &ConversationPlanAction,
    message: String,
    applied_at: u64,
) -> Result<ApplyReceipt, String> {
    Ok(ApplyReceipt {
        action_id: action.action_id.clone(),
        resource_id: action
            .source_resource_id
            .clone()
            .map(Ok)
            .unwrap_or_else(|| conversation_resource_id(action.provider, &action.thread_id))?,
        action_type: RestoreActionType::MaterializeConversation,
        logical_path: None,
        source_sha256: None,
        target_path: None,
        target_sha256_after: None,
        status: ActionStatus::Failed,
        applied_at,
        error: Some(message),
        conversation: None,
    })
}

/// Read the local Codex session index, reject a colliding reserved thread
/// ID, and build the appended index payload. Returns no payload without a
/// title: every row the provider writes is `{id, thread_name, updated_at}`,
/// so a titleless thread gets no row and stays invisible until Codex
/// rebuilds its own index from the materialized rollout.
fn codex_index_payload(
    path: &Path,
    provider_thread_id: &str,
    thread_name: Option<&str>,
    updated_at: u64,
) -> Result<(TargetExpectation, Option<Vec<u8>>), String> {
    let expectation = bounded_file_expectation(path)?;
    let mut bytes = match &expectation {
        TargetExpectation::Absent => Vec::new(),
        TargetExpectation::Sha256(_) => fs::read(path)
            .map_err(|error| format!("read Codex session index '{}': {error}", path.display()))?,
    };
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| "Codex session index contains malformed JSON".to_string())?;
        let id = value
            .get("id")
            .or_else(|| value.get("session_id"))
            .and_then(serde_json::Value::as_str);
        if id == Some(provider_thread_id) {
            return Err("reserved Codex provider ID is already indexed".to_string());
        }
    }
    let Some(thread_name) = thread_name else {
        return Ok((expectation, None));
    };
    let updated_at = Utc
        .timestamp_opt(updated_at as i64, 0)
        .single()
        .ok_or_else(|| "cloud conversation timestamp is out of range".to_string())?
        .format("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let mut row = serde_json::to_vec(&serde_json::json!({
        "id": provider_thread_id,
        "thread_name": thread_name,
        "updated_at": updated_at,
    }))
    .map_err(|error| format!("serialize Codex session index row: {error}"))?;
    row.push(b'\n');
    bytes.extend(row);
    Ok((expectation, Some(bytes)))
}

enum PreparedPullConversationOutcome {
    Applied(ApplyReceipt),
    Checkpoint(CodexForkCheckpoint),
}

fn apply_prepared_pull_conversation(
    repository: &LocalStateRepository,
    plan: &RestorePlan,
    binding: &ProjectBinding,
    prepared: PreparedPullConversation,
    applied_at: u64,
) -> Result<PreparedPullConversationOutcome, String> {
    let action = prepared.action;
    let action_id = action.action_id.clone();
    let action_thread_id = action.thread_id.clone();
    let action_provider = action.provider;
    let source_resource_id = action.source_resource_id.clone();
    let divergence_plan = action.divergence.clone();
    let provider_thread_id = divergence_plan
        .as_ref()
        .map(|divergence| divergence.source_provider_thread_id.clone())
        .or_else(|| action.proposed_provider_thread_id.clone())
        .ok_or_else(|| "conversation Pull action lacks a destination provider id".to_string())?;
    let rewritten = rewrite_codex_transcript(
        &prepared.cloud_transcript,
        &prepared.cloud_provider_thread_id,
        &provider_thread_id,
        &prepared.cloud_cwd,
        &binding.project_root,
    )?;
    let transcript_target = action
        .expected_provider_state
        .iter()
        .find(|target| {
            Path::new(&target.target_path)
                .file_name()
                .and_then(|name| name.to_str())
                != Some("session_index.jsonl")
        })
        .cloned()
        .ok_or_else(|| "conversation Pull action lacks a transcript target".to_string())?;
    let current_transcript_expectation =
        bounded_file_expectation(Path::new(&transcript_target.target_path))?;
    if current_transcript_expectation != transcript_target.expectation {
        return Err("conversation transcript target changed after review".to_string());
    }
    let mut targets = vec![ConversationTargetPayload {
        target_path: PathBuf::from(&transcript_target.target_path),
        role: ConversationFileRole::Transcript,
        expected: current_transcript_expectation,
        bytes: rewritten,
    }];
    if let Some(index_target) = action.expected_provider_state.iter().find(|target| {
        Path::new(&target.target_path)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("session_index.jsonl")
    }) {
        let index_path = PathBuf::from(&index_target.target_path);
        // A real title is one a user could have set: unnamed captures
        // publish the provider thread ID as their display title.
        let cloud_title = prepared
            .cloud_display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| {
                !title.is_empty() && *title != prepared.cloud_provider_thread_id.as_str()
            });
        let (expected, bytes) = codex_index_payload(
            &index_path,
            &provider_thread_id,
            cloud_title,
            prepared.cloud_updated_at,
        )?;
        if let Some(bytes) = bytes {
            targets.push(ConversationTargetPayload {
                target_path: index_path,
                role: ConversationFileRole::ProviderIndexFragment,
                expected,
                bytes,
            });
        }
    }
    let current_document = repository.load_conversation_bindings()?;
    let current_binding = current_document.bindings.iter().find(|candidate| {
        candidate.host_id == binding.host_id
            && candidate.replica_id == binding.replica_id
            && candidate.local_project_id == binding.local_project_id
            && candidate.provider_profile_id == action.provider_profile_id
            && candidate.thread_id == action.thread_id
    });
    let operation_kind = match (&divergence_plan, prepared.resolution) {
        (Some(divergence), Some(resolution)) => {
            ConversationSyncOperationKind::CodexDivergence(CodexDivergenceOperation {
                resolution,
                available_resolutions: divergence.available_resolutions.clone(),
                preserved_thread_id: divergence.preserved_thread_id.clone(),
                source_provider_thread_id: divergence.source_provider_thread_id.clone(),
                fork_provider_thread_id: None,
                fork_transcript_path: None,
                fork_transcript_sha256: None,
                local_lineage_digest: divergence.local_lineage_digest.clone(),
                local_semantic_digest: divergence.local_semantic_digest.clone(),
                pinned_local_title: divergence.pinned_local_title.clone(),
                pinned_cloud_title: divergence.pinned_cloud_title.clone(),
                desired_fork_name: divergence.desired_fork_name.clone(),
                desired_original_name: divergence.desired_original_name.clone(),
                codex_executable: divergence.codex_executable.clone(),
                codex_executable_sha256: divergence.codex_executable_sha256.clone(),
                available_executors: divergence.available_executors.clone(),
                pre_existing_children: divergence.pre_existing_children.clone(),
                attempts: Vec::new(),
                current_executor: None,
                successful_executor: None,
                fork_name_status: None,
                original_name_status: None,
                inventory_refresh_status: CodexInventoryRefreshStatus::NotAttempted,
            })
        }
        (Some(_), None) => {
            return Err("conversation divergence lacks a selected resolution".to_string())
        }
        (None, Some(_)) => {
            return Err("non-divergence conversation carries a resolution".to_string())
        }
        (None, None) => ConversationSyncOperationKind::ProviderMaterialization,
    };
    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(generated_named_id("operation")?)?,
        plan_id: Some(plan.plan_id.clone()),
        plan_action_id: Some(action.action_id.clone()),
        direction: ConversationPlanDirection::Pull,
        phase: ConversationSyncPhase::Prepared,
        storage_id: plan.storage_id.clone(),
        bundle_id: plan.bundle_id.clone(),
        generation: plan.generation,
        commit_id: plan.commit_id.clone(),
        manifest_sha256: plan.manifest_sha256.clone(),
        host_id: binding.host_id.clone(),
        replica_id: binding.replica_id.clone(),
        local_project_id: binding.local_project_id.clone(),
        provider_profile_id: action.provider_profile_id.clone(),
        provider: action.provider,
        thread_id: action.thread_id.clone(),
        operation: operation_kind,
        binding_forked_from: prepared.cloud_forked_from,
        provider_thread_id: Some(provider_thread_id),
        materialization_state: ConversationMaterializationState::Resumable,
        lineage_digest: action.pinned_lineage_digest.clone(),
        semantic_digest: action.pinned_semantic_digest.clone(),
        expected_host_identity_revision: action.expected_host_identity_revision,
        expected_bindings_document_revision: current_document.revision,
        expected_binding_revision: current_binding.map(|binding| binding.revision),
        expected_profile_revision: action.expected_profile_revision,
        expected_project_binding_revision: action.expected_project_binding_revision,
        expected_local_digest: match action.expected_local_target {
            TargetExpectation::Sha256(ref digest) => Some(digest.clone()),
            TargetExpectation::Absent => None,
        },
        targets: Vec::new(),
        created_at: applied_at,
        updated_at: applied_at,
    };
    let mut failpoints = NoConversationFailpoints;
    let conversation = if let Some(divergence) = divergence_plan {
        let codex_home = binding
            .codex_home
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| "Codex divergence requires a bound Codex home".to_string())?;
        let source_transcript_sha256 = match &action.expected_local_target {
            TargetExpectation::Sha256(digest) => digest.clone(),
            TargetExpectation::Absent => {
                return Err("Codex divergence source transcript is absent".to_string())
            }
        };
        let fork = CodexForkExecutionRequest {
            codex_home,
            project_directory: PathBuf::from(&binding.project_root),
            codex_executable: PathBuf::from(&divergence.codex_executable),
            codex_executable_sha256: divergence.codex_executable_sha256.clone(),
            source_provider_thread_id: divergence.source_provider_thread_id.clone(),
            source_transcript_sha256,
            local_lineage_digest: divergence.local_lineage_digest.clone(),
            local_semantic_digest: divergence.local_semantic_digest.clone(),
            pinned_local_title: divergence.pinned_local_title.clone(),
            desired_fork_name: divergence.desired_fork_name.clone(),
            available_executors: divergence.available_executors.clone(),
            pre_existing_children: divergence.pre_existing_children.clone(),
        };
        match apply_codex_divergence_locked(
            repository,
            CodexDivergenceMaterializationRequest {
                operation,
                targets,
                fork,
            },
            &mut failpoints,
        )? {
            CodexDivergenceApplyOutcome::Committed(conversation) => conversation,
            CodexDivergenceApplyOutcome::AwaitingManualFork { operation_id } => {
                return Ok(PreparedPullConversationOutcome::Checkpoint(
                    CodexForkCheckpoint {
                        operation_id,
                        plan_id: plan.plan_id.clone(),
                        action_id,
                        thread_id: action_thread_id,
                        source_provider_thread_id: divergence.source_provider_thread_id,
                        suggested_fork_name: divergence.desired_fork_name,
                        status: RestoreApplyStatus::AwaitingManualFork,
                        message: "Automatic Codex fork methods were unavailable. Create the fork in Codex, then verify it here.".to_string(),
                    },
                ));
            }
            CodexDivergenceApplyOutcome::RecoveryRequired {
                operation_id,
                message,
            } => {
                return Ok(PreparedPullConversationOutcome::Checkpoint(
                    CodexForkCheckpoint {
                        operation_id,
                        plan_id: plan.plan_id.clone(),
                        action_id,
                        thread_id: action_thread_id,
                        source_provider_thread_id: divergence.source_provider_thread_id,
                        suggested_fork_name: divergence.desired_fork_name,
                        status: RestoreApplyStatus::RecoveryRequired,
                        message,
                    },
                ));
            }
        }
    } else {
        apply_conversation_materialization_locked(
            repository,
            ConversationMaterializationRequest { operation, targets },
            &mut failpoints,
        )?
    };
    Ok(PreparedPullConversationOutcome::Applied(ApplyReceipt {
        action_id,
        resource_id: source_resource_id.unwrap_or(conversation_resource_id(
            action_provider,
            &action_thread_id,
        )?),
        action_type: RestoreActionType::MaterializeConversation,
        logical_path: None,
        source_sha256: Some(prepared.cloud_file_sha256),
        target_path: Some(transcript_target.target_path),
        target_sha256_after: conversation.local_file_sha256_after.clone(),
        status: ActionStatus::Applied,
        applied_at,
        error: None,
        conversation: Some(conversation),
    }))
}

fn apply_bundle_restore_with_progress(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    approved_action_ids: Vec<ActionId>,
    conversation_resolutions: Vec<ConversationResolutionSelection>,
    on_progress: &mut dyn FnMut(FetchProgress),
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<RestoreResult, String> {
    let phase_started = Instant::now();
    let _host_operation = repository.acquire_host_operation()?;
    let plan = repository.load_restore_plan(plan_id)?;
    let binding = current_binding_for_restore_plan(repository, &plan)?;
    let (mut approved, mut approved_conversation_actions) =
        split_approved_restore_actions(&approved_action_ids, &conversation_resolutions, &plan)?;
    let existing_materialization = repository
        .load_materializations()?
        .records
        .into_iter()
        .find(|record| record.plan_id == plan.plan_id);
    if let Some(record) = &existing_materialization {
        if record.host_id != binding.host_id
            || record.replica_id != binding.replica_id
            || record.local_project_id != binding.local_project_id
            || record.storage_id != plan.storage_id
            || record.bundle_id != plan.bundle_id
            || record.generation != plan.generation
            || record.commit_id != plan.commit_id
            || record.manifest_sha256 != plan.manifest_sha256
            || record.binding_revision != plan.binding_revision
        {
            return Err("existing restore receipts disagree with the reviewed plan".to_string());
        }
        let completed = record
            .receipts
            .iter()
            .filter(|receipt| {
                matches!(
                    receipt.status,
                    ActionStatus::Applied | ActionStatus::Skipped
                )
            })
            .map(|receipt| receipt.action_id.clone())
            .collect::<BTreeSet<_>>();
        approved.retain(|action_id| !completed.contains(action_id));
        approved_conversation_actions
            .retain(|selected| !completed.contains(&selected.action.action_id));
    }
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
    let (engine, snapshot) = inspect_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    let restored_recipe = snapshot.manifest.recipe.clone();
    let applied_at = now_secs();
    on_phase("validate reviewed plan", phase_started.elapsed());
    let phase_started = Instant::now();
    let mut byte_source = engine.byte_source(&snapshot, Some(object_cache()), Some(on_progress));
    let (mut prepared_conversations, prepare_failures) = prepare_pull_conversation_actions(
        repository,
        &binding,
        &snapshot,
        &mut byte_source,
        &approved_conversation_actions,
        applied_at,
    )?;
    on_phase("prepare conversations", phase_started.elapsed());
    prepared_conversations.sort_by_key(|prepared| {
        usize::from(prepared.action.action != ConversationPlanActionType::ResolveDivergence)
    });
    let mut conversation_receipts = Vec::new();
    let mut fork_checkpoints = Vec::new();
    for failure in prepare_failures {
        conversation_receipts.push(failed_conversation_receipt(
            &failure.action,
            failure.message,
            applied_at,
        )?);
    }
    // Threads settle independently: a divergence that pauses at a fork
    // checkpoint or fails outright never blocks the remaining approved
    // conversations or the file actions below.
    for prepared in prepared_conversations {
        let action = prepared.action.clone();
        let resource = action
            .source_resource_id
            .clone()
            .or_else(|| conversation_resource_id(action.provider, &action.thread_id).ok())
            .map(|resource_id| resource_id.to_string())
            .unwrap_or_else(|| action.thread_id.to_string());
        let phase_label = format!("apply conversation {resource}");
        let phase_started = Instant::now();
        match apply_prepared_pull_conversation(repository, &plan, &binding, prepared, applied_at) {
            Ok(PreparedPullConversationOutcome::Applied(receipt)) => {
                conversation_receipts.push(receipt);
            }
            Ok(PreparedPullConversationOutcome::Checkpoint(checkpoint)) => {
                fork_checkpoints.push(checkpoint);
            }
            Err(message) => {
                conversation_receipts
                    .push(failed_conversation_receipt(&action, message, applied_at)?);
            }
        }
        on_phase(&phase_label, phase_started.elapsed());
    }
    let phase_started = Instant::now();
    let mut applied = engine.apply_restore_plan_lazy(
        &snapshot,
        &mut byte_source,
        &binding,
        &plan,
        &approved,
        &repository.backups_dir()?,
        applied_at,
    )?;
    on_phase("restore files and provider data", phase_started.elapsed());
    let phase_started = Instant::now();
    applied.receipts.extend(conversation_receipts);
    if let Some(record) = repository
        .load_materializations()?
        .records
        .into_iter()
        .find(|record| record.plan_id == plan.plan_id)
    {
        let recorded = applied
            .receipts
            .iter()
            .map(|receipt| receipt.action_id.clone())
            .collect::<BTreeSet<_>>();
        applied.receipts.extend(
            record
                .receipts
                .into_iter()
                .filter(|receipt| !recorded.contains(&receipt.action_id)),
        );
    }
    let kept_local_conversation_actions = plan
        .actions
        .iter()
        .filter(|action| is_conversation_keep_local_action(action, &snapshot.manifest))
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
            | RestoreActionType::MaterializeConversation => receipt.status == ActionStatus::Applied,
            RestoreActionType::Manual => {
                receipt.status == ActionStatus::Applied
                    || (receipt.status == ActionStatus::Skipped
                        && kept_local_conversation_actions.contains(&receipt.action_id))
            }
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
        if let Some(current) = materializations
            .records
            .iter_mut()
            .find(|current| current.plan_id == record.plan_id)
        {
            if current.host_id != record.host_id
                || current.replica_id != record.replica_id
                || current.local_project_id != record.local_project_id
                || current.storage_id != record.storage_id
                || current.bundle_id != record.bundle_id
                || current.generation != record.generation
                || current.commit_id != record.commit_id
                || current.manifest_sha256 != record.manifest_sha256
                || current.binding_revision != record.binding_revision
            {
                return Err("restore receipts disagree on their reviewed plan scope".to_string());
            }
            let existing_action_ids = current
                .receipts
                .iter()
                .map(|receipt| receipt.action_id.clone())
                .collect::<BTreeSet<_>>();
            current.receipts.extend(
                record
                    .receipts
                    .iter()
                    .filter(|receipt| !existing_action_ids.contains(&receipt.action_id))
                    .cloned(),
            );
            current.status = record.status;
            current.applied_at = current.applied_at.max(record.applied_at);
            return Ok(());
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
    let codex_divergences = applied
        .receipts
        .iter()
        .filter_map(|receipt| {
            receipt.conversation.as_ref().and_then(|conversation| {
                codex_divergence_result(receipt.action_id.clone(), conversation)
            })
        })
        .collect::<Vec<_>>();
    let success = overall_success;
    let mut message = if success {
        format!("Applied {} restore actions", applied_action_ids.len())
    } else {
        format!(
            "Applied {} actions; {} require attention",
            applied_action_ids.len(),
            failed_actions.len()
        )
    };
    if !fork_checkpoints.is_empty() {
        message = format!(
            "{message}; {} Codex fork checkpoint(s) need verification",
            fork_checkpoints.len()
        );
    }
    let result = RestoreResult {
        success,
        status: restore_status_for_checkpoints(&fork_checkpoints),
        message,
        plan_id: plan.plan_id,
        applied_action_ids,
        failed_actions,
        // Every committed conversation materialization — not just a
        // divergence — leaves a running Codex app unaware of the thread
        // until its next thread/list rescan. While fork checkpoints are
        // pending the offer waits: the final resume returns its own.
        codex_app_refresh: if fork_checkpoints.is_empty() {
            applied
                .receipts
                .iter()
                .find_map(|receipt| receipt.conversation.as_ref())
                .and_then(|receipt| {
                    codex_app_refresh_offer(
                        receipt.journal_operation_id.clone(),
                        binding.codex_home.as_deref().map(Path::new),
                    )
                })
        } else {
            None
        },
        codex_fork_checkpoints: fork_checkpoints,
        codex_divergences,
    };
    on_phase("persist receipts and finalize", phase_started.elapsed());
    Ok(result)
}

fn committed_conversation_receipt(
    repository: &LocalStateRepository,
    operation_id: &ConversationOperationId,
) -> Result<super::domain::ConversationApplyReceipt, String> {
    if repository
        .load_conversation_sync_journal()?
        .operations
        .iter()
        .any(|operation| &operation.operation_id == operation_id)
    {
        return Err("conversation sync has not finished LocalCommitted cleanup".to_string());
    }
    let materializations = repository.load_materializations()?;
    materializations
        .records
        .iter()
        .flat_map(|record| record.receipts.iter())
        .filter(|receipt| receipt.status == ActionStatus::Applied)
        .find_map(|receipt| {
            receipt
                .conversation
                .clone()
                .filter(|conversation| &conversation.journal_operation_id == operation_id)
        })
        .ok_or_else(|| "committed conversation receipt was not found".to_string())
}

/// The exact Codex home the pulled profile reads. An app restart must
/// target this home, never whatever the GUI session environment happens to
/// select.
fn codex_home_for_profile(
    repository: &LocalStateRepository,
    profile_id: &LocalProviderProfileId,
) -> Result<PathBuf, String> {
    let machine = repository.load_bindings()?;
    let profile = machine
        .profiles
        .iter()
        .find(|profile| &profile.profile_id == profile_id)
        .ok_or_else(|| "the pulled Codex profile is no longer registered".to_string())?;
    Ok(PathBuf::from(&profile.canonical_path))
}

fn codex_divergence_result(
    action_id: ActionId,
    receipt: &super::domain::ConversationApplyReceipt,
) -> Option<CodexDivergenceResult> {
    let divergence = receipt.divergence.as_ref()?;
    Some(CodexDivergenceResult {
        action_id,
        original_thread_id: receipt.thread_id.clone(),
        original_provider_thread_id: receipt.provider_thread_id.clone()?,
        resolution: divergence.resolution,
        preserved_thread_id: divergence.preserved_thread_id.clone(),
        preserved_provider_thread_id: divergence.preserved_provider_thread_id.clone(),
        executor: divergence.executor,
        desired_fork_name: divergence.desired_fork_name.clone(),
        fork_name_status: divergence.fork_name_status,
        desired_original_name: divergence.desired_original_name.clone(),
        original_name_status: divergence.original_name_status,
        inventory_refresh_status: divergence.inventory_refresh_status,
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
/// Load the plan's binding and re-inspect the bundle head it pins. This is
/// deliberately metadata-only — supporting views (dependencies, readiness)
/// must never download payload objects.
fn restore_support_context(
    repository: &LocalStateRepository,
    restore_plan_id: &PlanId,
) -> Result<(RestorePlan, ProjectBinding, BundleSnapshot), String> {
    let plan = repository.load_restore_plan(restore_plan_id)?;
    let now = now_secs();
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    let binding = current_binding_for_restore_plan(repository, &plan)?;
    let (_, snapshot) = inspect_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    let head = &snapshot.head;
    if plan.storage_id != snapshot.storage_id
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
    Ok((plan, binding, snapshot))
}

fn get_bundle_readiness_with_repository(
    repository: &LocalStateRepository,
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
    let (_, snapshot) = inspect_from_storage(repository, storage_id, bundle_id)?;
    bundle_readiness_for_snapshot(repository, &binding, &snapshot)
}

fn get_restore_readiness_with_repository(
    repository: &LocalStateRepository,
    restore_plan_id: &PlanId,
) -> Result<BundleReadiness, String> {
    let (_, binding, snapshot) = restore_support_context(repository, restore_plan_id)?;
    bundle_readiness_for_snapshot(repository, &binding, &snapshot)
}

fn bundle_readiness_for_snapshot(
    repository: &LocalStateRepository,
    binding: &ProjectBinding,
    snapshot: &BundleSnapshot,
) -> Result<BundleReadiness, String> {
    let head = &snapshot.head;
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
    let mut issues = Vec::new();
    let needs_codex_home = snapshot
        .manifest
        .resources
        .values()
        .any(|resource| resource.kind == ResourceKind::CodexConversation);
    let needs_claude_home = snapshot
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
    if !snapshot.manifest.files.is_empty() {
        match materialization.as_ref() {
            None => issues.push(BundleReadinessIssue {
                issue_id: "restore-files".to_string(),
                category: "project_files".to_string(),
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
                add_partial_materialization_issues(&mut issues, record, snapshot);
            }
            Some(_) => {}
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
    snapshot: &BundleSnapshot,
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
            snapshot
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
            .and_then(|resource_id| snapshot.manifest.resources.get(resource_id))
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
                | RestoreActionType::MaterializeConversation => true,
                RestoreActionType::WriteProjectFile
                | RestoreActionType::EnsureProjectDirectory
                | RestoreActionType::DeleteProjectFile
                | RestoreActionType::DeleteProjectDirectory => {
                    matches!(receipt.status, ActionStatus::Failed | ActionStatus::Blocked)
                }
                RestoreActionType::Manual => false,
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
            category: "project_files".to_string(),
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
        // The desktop app is launched through a deep link, not the CLI, so it
        // needs its own line rather than a variation on the two above.
        #[cfg(target_os = "macos")]
        if let Some(command) = chat_history::codex_app_project_open_command(
            Path::new(codex_home),
            Path::new(&binding.project_root),
        ) {
            commands.push(("Codex app — new", command));
        }
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

fn split_approved_restore_actions(
    requested: &[ActionId],
    conversation_resolutions: &[ConversationResolutionSelection],
    plan: &RestorePlan,
) -> Result<(BTreeSet<ActionId>, Vec<SelectedPullConversation>), String> {
    let selected = requested.iter().cloned().collect::<BTreeSet<_>>();
    if selected.len() != requested.len() {
        return Err("approved restore actions contain duplicates".to_string());
    }
    let ordinary = plan
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    let conversation = plan
        .conversation_actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = selected
        .iter()
        .find(|action_id| !ordinary.contains(*action_id) && !conversation.contains(*action_id))
    {
        return Err(format!("restore plan has no action '{}'", unknown));
    }
    let selected_conversation_actions = plan
        .conversation_actions
        .iter()
        .filter(|action| selected.contains(&action.action_id))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(action) = selected_conversation_actions
        .iter()
        .find(|action| !is_selectable_pull_conversation_action(action))
    {
        return Err(format!(
            "conversation Pull action '{}' is informational and cannot be selected",
            action.action_id
        ));
    }
    let mut resolution_by_action = BTreeMap::new();
    for selection in conversation_resolutions {
        if resolution_by_action
            .insert(selection.action_id.clone(), selection)
            .is_some()
        {
            return Err(format!(
                "conversation resolution '{}' was selected more than once",
                selection.action_id
            ));
        }
        let action = plan
            .conversation_actions
            .iter()
            .find(|action| action.action_id == selection.action_id)
            .ok_or_else(|| {
                format!(
                    "conversation resolution '{}' is not in this Pull plan",
                    selection.action_id
                )
            })?;
        if action.action != ConversationPlanActionType::ResolveDivergence
            || !selected.contains(&selection.action_id)
        {
            return Err(format!(
                "conversation resolution '{}' does not belong to a selected divergence",
                selection.action_id
            ));
        }
        let divergence = action.divergence.as_ref().ok_or_else(|| {
            format!(
                "conversation divergence '{}' lacks its reviewed options",
                selection.action_id
            )
        })?;
        if !divergence
            .available_resolutions
            .contains(&selection.resolution)
        {
            return Err(format!(
                "conversation resolution '{}' was not advertised by this Pull plan",
                selection.action_id
            ));
        }
        if selection.resolution == ConversationDivergenceResolution::UseCloudOnly
            && !selection.destructive_approved
        {
            return Err(format!(
                "cloud-only resolution '{}' requires destructive approval",
                selection.action_id
            ));
        }
        if selection.resolution == ConversationDivergenceResolution::PreserveLocalWithNativeFork
            && selection.destructive_approved
        {
            return Err(format!(
                "keep-both resolution '{}' must not carry destructive approval",
                selection.action_id
            ));
        }
    }
    let mut selected_conversations = Vec::with_capacity(selected_conversation_actions.len());
    for action in selected_conversation_actions {
        let resolution = if action.action == ConversationPlanActionType::ResolveDivergence {
            Some(
                resolution_by_action
                    .remove(&action.action_id)
                    .ok_or_else(|| {
                        format!(
                            "conversation divergence '{}' lacks a resolution",
                            action.action_id
                        )
                    })?
                    .resolution,
            )
        } else {
            if resolution_by_action.remove(&action.action_id).is_some() {
                return Err(format!(
                    "non-divergence conversation '{}' has a resolution",
                    action.action_id
                ));
            }
            None
        };
        selected_conversations.push(SelectedPullConversation { action, resolution });
    }
    if let Some(action_id) = resolution_by_action.keys().next() {
        return Err(format!(
            "conversation resolution '{}' was not consumed by this Pull",
            action_id
        ));
    }
    Ok((
        selected
            .into_iter()
            .filter(|action_id| ordinary.contains(action_id))
            .collect(),
        selected_conversations,
    ))
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
    repository: &LocalStateRepository,
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
    let mut request = CaptureRequest {
        project_root: resolved_project,
        codex_home: binding.codex_home.as_ref().map(PathBuf::from),
        claude_home: binding.claude_home.as_ref().map(PathBuf::from),
        excluded_project_roots,
        include_project_content: false,
        excluded_content_roots: Vec::new(),
    };
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct RepositoryIdentity {
    fingerprint: String,
}

impl RepositoryIdentity {
    fn matches(&self, fingerprint: Option<&str>) -> bool {
        fingerprint == Some(self.fingerprint.as_str())
    }
}

fn repository_identity(project_root: &Path) -> Option<RepositoryIdentity> {
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
                return Some(repository_identity_from_remote_url(value.trim()));
            }
        }
    }
    None
}

fn repository_fingerprint(project_root: &Path) -> Option<String> {
    repository_identity(project_root).map(|identity| identity.fingerprint)
}

fn repository_identity_from_remote_url(value: &str) -> RepositoryIdentity {
    let sanitized = strip_url_userinfo(value.trim());
    let canonical_github = chat_history::normalize_github_repository_url(value);
    let canonical = canonical_github.as_deref().unwrap_or(&sanitized);
    RepositoryIdentity {
        fingerprint: repository_url_fingerprint(canonical),
    }
}

fn repository_url_fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex_digest(&digest)
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
    config: &SyncConfig,
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
    repository: &LocalStateRepository,
    storage_id: &StorageId,
) -> Result<(StorageConfig, StorageEngine), String> {
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
fn engine_for_storage_config(storage: &StorageConfig) -> Result<StorageEngine, String> {
    let store = match storage.kind {
        StorageKind::Local => {
            ConfiguredStore::Local(LocalBundleObjectStore::open(&storage.local_dir)?)
        }
        StorageKind::S3 => ConfiguredStore::S3(S3BundleObjectStore::from_current_runtime(storage)?),
    };
    BundleEngine::open(store, storage.id.clone())
}

fn validate_config_storage_isolation(
    repository: &LocalStateRepository,
    config: &SyncConfig,
    bindings: &[ProjectBinding],
    profiles: &[ProviderProfile],
) -> Result<(), String> {
    let repository_root = prospective_canonical(repository.root())?;
    let mut local_roots = Vec::<(&StorageConfig, PathBuf)>::new();
    for storage in config
        .storages
        .iter()
        .filter(|storage| storage.kind == StorageKind::Local)
    {
        let root = prospective_canonical(Path::new(&storage.local_dir))?;
        if paths_overlap(&root, &repository_root) {
            return Err(format!(
                "local storage '{}' overlaps Mallard application data",
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
    request: ConnectProjectBundleRequest,
) -> Result<ProjectDetail, String> {
    let _host_operation = repository.acquire_host_operation()?;
    connect_project_to_remote_bundle_guarded(repository, request)
}

fn connect_project_to_remote_bundle_guarded(
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
    request: SaveProjectBindingRequest,
) -> Result<ProjectBinding, String> {
    let _host_operation = repository.acquire_host_operation()?;
    save_project_binding_guarded(repository, request)
}

fn save_project_binding_guarded(
    repository: &LocalStateRepository,
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
fn project_label(config: &SyncConfig, id: &LocalProjectId) -> String {
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
    repository: &LocalStateRepository,
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
            "project root '{}' overlaps Mallard app data '{}'",
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
                "{} '{}' overlaps Mallard app data",
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
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    let _host_operation = repository.acquire_host_operation()?;
    detach_project_binding_guarded(repository, local_project_id)
}

fn detach_project_binding_guarded(
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<bool, String> {
    let _host_operation = repository.acquire_host_operation()?;
    remove_local_project_guarded(repository, local_project_id)
}

fn remove_local_project_guarded(
    repository: &LocalStateRepository,
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
    /// Signature of the provider homes this discovery ran against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fresh_profile_signature: Option<String>,
    /// The selection was saved against different provider homes, so it should
    /// be re-derived rather than reported as drift.  Suppresses
    /// `selection_stale`: the two are never both true.
    pub profile_changed: bool,
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

fn list_setup_drafts_with_repository(
    repository: &LocalStateRepository,
) -> Result<SetupDraftList, String> {
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
    config: &SyncConfig,
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
    repository: &LocalStateRepository,
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
        return Err("project root overlaps Mallard application data".to_string());
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
        schema: SETUP_DRAFT_SCHEMA,
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
        discovery_profile_signature: String::new(),
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
    repository: &LocalStateRepository,
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
        schema: SETUP_DRAFT_SCHEMA,
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
        discovery_profile_signature: submitted.discovery_profile_signature,
        revision: submitted.revision,
        created_at: stored.created_at,
        updated_at: now_secs(),
        last_error: stored.last_error,
    };
    repository.save_setup_draft(draft)
}

fn discard_setup_draft_with_repository(
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    storage: StorageConfig,
    pending: bool,
}

fn resolve_draft_storage(
    repository: &LocalStateRepository,
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

fn verified_repository_identity(
    remote: &Option<String>,
    local: Option<&RepositoryIdentity>,
) -> bool {
    local.is_some_and(|local| local.matches(remote.as_deref()))
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

/// Digest over the resolved provider homes discovery ran against.  Paired with
/// `discovery_signature` so a repointed profile reads as "different sources"
/// rather than "the same sources changed underneath you".
fn profile_signature(paths: &BTreeMap<Provider, String>) -> String {
    let mut hasher = Sha256::new();
    for (provider, path) in paths {
        hasher.update(provider_name(*provider).as_bytes());
        hasher.update(b"\0");
        hasher.update(path.as_bytes());
        hasher.update(b"\n");
    }
    hex_digest(&hasher.finalize())
}

fn inspect_setup_draft_with_repository(
    repository: &LocalStateRepository,
    draft_id: &SetupDraftId,
) -> Result<SetupDraftInspection, String> {
    let mut draft = repository
        .load_setup_draft(draft_id)?
        .ok_or_else(|| format!("setup draft '{}' does not exist", draft_id))?;
    let local_repository_identity = repository_identity(Path::new(&draft.canonical_project_root));
    draft.repository_fingerprint = local_repository_identity
        .as_ref()
        .map(|identity| identity.fingerprint.clone());
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
    let mut fresh_profile = None;
    let mut profile_changed = false;
    let mut selection_stale = false;
    match resolve_draft_profiles(repository, &draft) {
        Ok(resolved) => {
            sections.push(SetupSectionStatus {
                section: "profiles".to_string(),
                state: "ready".to_string(),
                message: None,
            });
            let profile = profile_signature(&resolved.paths);
            profile_changed = !draft.discovery_profile_signature.is_empty()
                && draft.discovery_profile_signature != profile;
            fresh_profile = Some(profile);
            match discover_project_at(
                repository,
                &draft.project_root,
                &resolved.paths,
                &resolved.profile_ids,
            ) {
                Ok(discovery) => {
                    let signature = discovery_signature(&discovery.inventory);
                    // A repointed profile discovers a different set by
                    // definition; that is a re-derive the client performs, not
                    // drift for the user to adjudicate.
                    selection_stale = !profile_changed
                        && !draft.discovery_signature.is_empty()
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
            } else if !verified_repository_identity(
                remote_fingerprint,
                local_repository_identity.as_ref(),
            ) && !mismatch_acknowledged
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
        fresh_profile_signature: fresh_profile,
        profile_changed,
        selection_stale,
        warnings,
    })
}

fn build_setup_transaction(
    repository: &LocalStateRepository,
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
    let local_repository_identity = repository_identity(&canonical);
    let local_fingerprint = local_repository_identity
        .as_ref()
        .map(|identity| identity.fingerprint.clone());

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
            let snapshot = engine.inspect(bundle_id)?;
            let summary = bundle_snapshot_summary_from_snapshot(snapshot)?;
            if !verified_repository_identity(
                &summary.repository_fingerprint,
                local_repository_identity.as_ref(),
            ) && !mismatch_acknowledged
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
        schema: SETUP_TRANSACTION_SCHEMA,
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
pub(crate) fn recover_setup_state(repository: &LocalStateRepository) -> Vec<String> {
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

fn record_draft_error(repository: &LocalStateRepository, draft_id: &SetupDraftId, error: &str) {
    let Ok(Some(mut draft)) = repository.load_setup_draft(draft_id) else {
        return;
    };
    let mut message: String = error.chars().filter(|c| !c.is_control()).collect();
    message.truncate(4_000);
    draft.last_error = Some(message);
    let _ = repository.save_setup_draft(draft);
}

fn finalize_project_setup_with_repository(
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    let mut oversized_thread_ids = BTreeSet::new();
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
            let file_size = match entry.metadata() {
                Ok(metadata) => metadata.len(),
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
            if file_size > MAX_CONVERSATION_FILE_BYTES {
                oversized_thread_ids.extend(
                    assigned_thread_ids
                        .iter()
                        .filter(|thread_id| filename.contains(thread_id.as_str()))
                        .cloned(),
                );
                continue;
            }
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
        if oversized_thread_ids.contains(&thread_id) {
            audit.warnings.push(format!(
                "Codex thread '{}' was not audited because its session file exceeds the {} MiB per-file sync limit; the conversation remains unavailable for sync",
                thread_id,
                MAX_CONVERSATION_FILE_BYTES / (1024 * 1024)
            ));
            continue;
        }
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
        let transcript_metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                audit.blockers.push(format!(
                    "inspect assigned Codex conversation '{}': {}",
                    path.display(),
                    error
                ));
                continue;
            }
        };
        if transcript_metadata.len() > MAX_CONVERSATION_FILE_BYTES {
            audit.warnings.push(format!(
                "Codex thread '{}' was not audited because its session file exceeds the {} MiB per-file sync limit; the conversation remains unavailable for sync",
                thread_id,
                MAX_CONVERSATION_FILE_BYTES / (1024 * 1024)
            ));
            continue;
        }
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
    repository: &LocalStateRepository,
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
    repository: &LocalStateRepository,
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
    let structure = analyze_codex_rollout_structure(source, Some(expected_thread_id))
        .map_err(|error| error.to_string())?;
    Ok(structure
        .structural_cwds
        .into_iter()
        .filter(|cwd| !path_belongs_to_project(Path::new(cwd), project_root))
        .collect())
}

fn remap_codex_structural_cwds(
    source: &[u8],
    expected_thread_id: &str,
    target_cwd: &str,
) -> Result<Vec<u8>, String> {
    validate_absolute_clean_path("target Codex conversation cwd", target_cwd)?;
    let before = analyze_codex_rollout_structure(source, Some(expected_thread_id))
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(source.len());
    let mut changed = 0_usize;
    for line in source.split_inclusive(|byte| *byte == b'\n') {
        let (json, ending): (&[u8], &[u8]) = if let Some(json) = line.strip_suffix(b"\r\n") {
            (json, b"\r\n")
        } else if let Some(json) = line.strip_suffix(b"\n") {
            (json, b"\n")
        } else {
            (line, b"")
        };
        if json.is_empty() {
            output.extend_from_slice(line);
            continue;
        }
        let mut value: serde_json::Value = serde_json::from_slice(json)
            .map_err(|_| "Codex conversation contains malformed JSON".to_string())?;
        let record_type = structural_record_type(&value).map(str::to_string);
        if !matches!(
            record_type.as_deref(),
            Some("session_meta" | "turn_context")
        ) {
            output.extend_from_slice(line);
            continue;
        }
        let payload = value
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| {
                format!(
                    "{} row has no payload",
                    record_type.as_deref().unwrap_or("structural")
                )
            })?;
        let mut row_changed = false;
        for key in ["cwd", "working_directory", "workingDirectory"] {
            let Some(cwd) = payload.get_mut(key) else {
                continue;
            };
            let observed = cwd
                .as_str()
                .ok_or_else(|| format!("Codex structural field '{key}' is not a string"))?;
            if observed != target_cwd {
                *cwd = serde_json::Value::String(target_cwd.to_string());
                changed = changed.saturating_add(1);
                row_changed = true;
            }
        }
        if !row_changed {
            output.extend_from_slice(line);
            continue;
        }
        output.extend_from_slice(
            &serde_json::to_vec(&value)
                .map_err(|error| format!("serialize repaired Codex session row: {}", error))?,
        );
        output.extend_from_slice(ending);
    }
    if changed == 0 {
        return Err("conversation no longer contains a stale structural cwd".to_string());
    }
    let after = analyze_codex_rollout_structure(&output, Some(expected_thread_id))
        .map_err(|error| error.to_string())?;
    if before.current.id != after.current.id
        || before.fork_ancestry != after.fork_ancestry
        || before.accepted_metadata_ids != after.accepted_metadata_ids
        || after.structural_cwds != BTreeSet::from([target_cwd.to_string()])
    {
        return Err("repaired Codex conversation changed structural identity".to_string());
    }
    Ok(output)
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
    use crate::project_sync::domain::{
        RestoreActionKind, StorageConfig, StorageKind, TombstoneTarget,
    };
    use crate::sync_tests::thread_variants::harness::{
        publish_request, ThreadSpec, TranscriptBuilder,
    };

    #[test]
    fn codex_index_payload_writes_provider_schema_rows_only_for_titled_threads() {
        let temp = tempfile::tempdir().unwrap();
        let index_path = temp.path().join("session_index.jsonl");

        let (expectation, bytes) =
            codex_index_payload(&index_path, "thread-new", None, 1_784_332_800).unwrap();
        assert_eq!(expectation, TargetExpectation::Absent);
        assert!(bytes.is_none(), "titleless threads write no index row");

        let (_, bytes) =
            codex_index_payload(&index_path, "thread-new", Some("Named"), 1_784_332_800).unwrap();
        let row: serde_json::Value =
            serde_json::from_slice(bytes.unwrap().trim_ascii_end()).unwrap();
        assert_eq!(
            row,
            serde_json::json!({
                "id": "thread-new",
                "thread_name": "Named",
                "updated_at": "2026-07-18T00:00:00.000000Z",
            })
        );

        std::fs::write(
            &index_path,
            "{\"id\":\"thread-new\",\"thread_name\":\"Old\",\"updated_at\":\"2026-01-01T00:00:00.000000Z\"}\n",
        )
        .unwrap();
        for title in [None, Some("Named")] {
            assert!(
                codex_index_payload(&index_path, "thread-new", title, 1_784_332_800)
                    .unwrap_err()
                    .contains("already indexed"),
                "the collision scan runs whether or not a row will be written"
            );
        }
    }

    #[test]
    fn host_sensitive_workflows_share_the_host_operation_lock() {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path()).unwrap();
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
        assert!(prepare_push_review_with_repository(
            &repo,
            &LocalProjectId::parse("project-a").unwrap(),
            &StorageId::parse("storage-a").unwrap(),
        )
        .unwrap_err()
        .contains("host-sensitive operation"));
        drop(guard);
    }

    #[test]
    fn push_destination_classification_preserves_remote_history_safety() {
        let head = |generation: u64, manifest_sha256: &str| BundleHead {
            schema_version: crate::project_sync::domain::BUNDLE_SCHEMA,
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            kind: BundleKind::Project,
            generation,
            commit_id: format!("commit-{}", generation),
            manifest_key: format!("_manifests/{}.json", generation),
            manifest_sha256: manifest_sha256.to_string(),
            updated_at: generation,
        };
        let base = |generation: u64, manifest_sha256: &str, binding_revision: u64| RecipeBase {
            generation,
            manifest_sha256: manifest_sha256.to_string(),
            commit_id: Some(format!("commit-{}", generation)),
            recipe_revision: 1,
            binding_revision: Some(binding_revision),
            last_pull_at: None,
            last_push_at: Some(99),
        };
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);
        let reviewed = base(3, &digest_a, 7);
        let current = head(3, &digest_a);

        assert_eq!(
            classify_push_destination(None, None, 7),
            PushDestinationStatus::Ready { generation: None }
        );
        assert_eq!(
            classify_push_destination(None, Some(&reviewed), 7),
            PushDestinationStatus::RemoteHistoryMissing { base_generation: 3 }
        );
        assert_eq!(
            classify_push_destination(Some(&current), None, 7),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::RemoteWithoutBase,
                remote_generation: 3,
                base_generation: None,
            }
        );
        assert_eq!(
            classify_push_destination(Some(&current), Some(&reviewed), 8),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::BindingChanged,
                remote_generation: 3,
                base_generation: Some(3),
            }
        );
        assert_eq!(
            classify_push_destination(Some(&current), Some(&reviewed), 7),
            PushDestinationStatus::Ready {
                generation: Some(3),
            }
        );
        assert_eq!(
            classify_push_destination(Some(&head(4, &digest_b)), Some(&reviewed), 7),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::RemoteAdvanced,
                remote_generation: 4,
                base_generation: Some(3),
            }
        );

        let missing = classify_push_destination(None, Some(&reviewed), 7);
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({
                "state": "remote_history_missing",
                "base_generation": 3,
            })
        );
        let message = push_destination_error(&missing).unwrap();
        assert!(message.contains("remote history is missing"));
        assert!(message.contains("unlink and relink"));
        assert!(!message.contains("deleted"));
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

    fn repository(temp: &tempfile::TempDir) -> LocalStateRepository {
        LocalStateRepository::from_home_dir(temp.path()).unwrap()
    }

    fn register(repo: &LocalStateRepository) -> LocalProjectRegistration {
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

    fn add_profile(
        repo: &LocalStateRepository,
        provider: Provider,
        path: &Path,
    ) -> LocalProviderProfileId {
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

    fn add_local_storage(repo: &LocalStateRepository, storage_id: &StorageId, storage_root: &Path) {
        std::fs::create_dir_all(storage_root).unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
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
        repo: &LocalStateRepository,
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

    fn command_test_transcript(
        provider_thread_id: &str,
        cwd: &Path,
        messages: &[(&str, &str, &str)],
    ) -> Vec<u8> {
        let mut builder =
            TranscriptBuilder::new(provider_thread_id, cwd.to_string_lossy().as_ref());
        for (message_id, role, text) in messages {
            builder = builder.message(message_id, role, text);
        }
        builder.build()
    }

    fn command_test_subagent_transcript(
        provider_thread_id: &str,
        parent_thread_id: &str,
        cwd: &Path,
        messages: &[(&str, &str, &str)],
    ) -> Vec<u8> {
        let transcript = command_test_transcript(provider_thread_id, cwd, messages);
        let first_newline = transcript.iter().position(|byte| *byte == b'\n').unwrap();
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&transcript[..first_newline]).unwrap();
        metadata["payload"]["session_id"] = serde_json::Value::String(parent_thread_id.to_string());
        metadata["payload"]["parent_thread_id"] =
            serde_json::Value::String(parent_thread_id.to_string());
        metadata["payload"]["source"] = serde_json::json!({
            "subagent": {
                "thread_spawn": {
                    "parent_thread_id": parent_thread_id,
                    "depth": 1
                }
            }
        });
        let mut marked = serde_json::to_vec(&metadata).unwrap();
        marked.push(b'\n');
        marked.extend_from_slice(&transcript[first_newline + 1..]);
        marked
    }

    fn publish_three_level_chain(
        repo: &LocalStateRepository,
        project: &LocalProjectRegistration,
        binding: &ProjectBinding,
        storage_id: &StorageId,
        remote_cwd: &Path,
        tip_transcript: Vec<u8>,
    ) -> (BundleSnapshot, ThreadId, ThreadId, ThreadId) {
        let t1 = ThreadId::parse("thread-t1").unwrap();
        let t11 = ThreadId::parse("thread-t11").unwrap();
        let t111 = ThreadId::parse("thread-t111").unwrap();
        let (origin_host_token, origin_replica_token) = repo
            .derive_origin_tokens(&project.bundle_id, &binding.replica_id)
            .unwrap();
        let origin = PortableOrigin {
            origin_host_token,
            origin_replica_token,
            shared_label: None,
        };
        let mut root = ThreadSpec::new(
            t1.as_str(),
            "provider-t1",
            command_test_transcript(
                "provider-t1",
                remote_cwd,
                &[("root-message", "user", "root")],
            ),
            origin.clone(),
            10,
        );
        root.title = "Root".to_string();
        let mut child = ThreadSpec::new(
            t11.as_str(),
            "provider-t11",
            command_test_transcript(
                "provider-t11",
                remote_cwd,
                &[
                    ("root-message", "user", "root"),
                    ("child-message", "assistant", "child"),
                ],
            ),
            origin.clone(),
            11,
        );
        child.title = "Child".to_string();
        child.forked_from = Some(t1.clone());
        let mut tip = ThreadSpec::new(
            t111.as_str(),
            "provider-t111",
            tip_transcript,
            origin.clone(),
            12,
        );
        tip.title = "Tip".to_string();
        tip.forked_from = Some(t11.clone());
        let (_, engine) = storage_engine(repo, storage_id).unwrap();
        let published = engine
            .publish(publish_request(
                project.bundle_id.clone(),
                &[root, child, tip],
                origin,
                12,
                None,
                PublishExpectation::Absent,
            ))
            .unwrap();
        let snapshot = published.snapshot;
        repo.mutate_config(|config| {
            let current = config
                .projects
                .iter_mut()
                .find(|candidate| candidate.local_project_id == project.local_project_id)
                .ok_or_else(|| "test project disappeared".to_string())?;
            current.recipe_bases.insert(
                storage_id.clone(),
                RecipeBase {
                    generation: snapshot.head.generation,
                    manifest_sha256: snapshot.head.manifest_sha256.clone(),
                    commit_id: Some(snapshot.head.commit_id.clone()),
                    recipe_revision: snapshot.manifest.recipe.revision,
                    binding_revision: Some(binding.revision),
                    last_pull_at: Some(12),
                    last_push_at: None,
                },
            );
            Ok(())
        })
        .unwrap();
        (snapshot, t1, t11, t111)
    }

    fn bind_local_conversation(
        repo: &LocalStateRepository,
        binding: &ProjectBinding,
        codex_home: &Path,
        logical_thread_id: &ThreadId,
        logical_parent: Option<ThreadId>,
        provider_thread_id: &str,
        transcript: &[u8],
        source: &BundleSnapshot,
    ) -> PathBuf {
        let target = codex_home.join("sessions/2026/07/27").join(format!(
            "rollout-2026-07-27T12-00-00-{}.jsonl",
            provider_thread_id
        ));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, transcript).unwrap();
        let analysis = analyze_codex_transcript(transcript);
        let (head_message_id, lineage_digest, semantic_digest) = match analysis.lineage {
            CodecLineage::Complete {
                chain,
                lineage_digest,
                semantic_digest,
            } => (
                chain.last().map(|message| message.message_id.to_string()),
                lineage_digest,
                semantic_digest,
            ),
            other => panic!("command test transcript is not resumable: {other:?}"),
        };
        let profile_id = binding.profile_ids[&Provider::Codex].clone();
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_id,
                thread_id: logical_thread_id.clone(),
                forked_from: logical_parent,
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                last_observed_file_sha256: Some(hex_digest(Sha256::digest(transcript).as_slice())),
                last_observed_head_message_id: head_message_id,
                last_observed_lineage_digest: Some(lineage_digest),
                last_observed_semantic_digest: Some(semantic_digest),
                source_generation: Some(source.head.generation),
                source_manifest_sha256: Some(source.head.manifest_sha256.clone()),
                revision: 0,
                created_at: 12,
                updated_at: 13,
            });
            Ok(())
        })
        .unwrap();
        target
    }

    fn empty_project_content_review() -> ProjectContentPushReview {
        ProjectContentPushReview {
            review_token: None,
            removal_ids: BTreeSet::new(),
            acknowledged_warning_digests: BTreeSet::new(),
        }
    }

    fn divergence_selection_test_plan() -> RestorePlan {
        let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
        let replica_id = ReplicaId::parse("replica-divergence").unwrap();
        let local_project_id = LocalProjectId::parse("project-divergence").unwrap();
        let profile_id = LocalProviderProfileId::parse("profile-divergence").unwrap();
        let action = ConversationPlanAction {
            action_id: ActionId::parse("action-divergence").unwrap(),
            direction: ConversationPlanDirection::Pull,
            action: ConversationPlanActionType::ResolveDivergence,
            reason_code: ConversationReasonCode::Diverged,
            relation: ConversationRelation::Diverged,
            provider: Provider::Codex,
            host_id: host_id.clone(),
            replica_id: replica_id.clone(),
            local_project_id,
            provider_profile_id: profile_id,
            thread_id: ThreadId::parse("thread-shared").unwrap(),
            divergence: Some(crate::project_sync::domain::ConversationDivergencePlan {
                available_resolutions: vec![
                    ConversationDivergenceResolution::PreserveLocalWithNativeFork,
                    ConversationDivergenceResolution::UseCloudOnly,
                ],
                preserved_thread_id: ThreadId::parse("thread-preserved").unwrap(),
                source_provider_thread_id: "source-native".to_string(),
                local_lineage_digest: "b".repeat(64),
                local_semantic_digest: "c".repeat(64),
                pinned_local_title: Some("Local title".to_string()),
                pinned_cloud_title: Some("Cloud title".to_string()),
                desired_fork_name: "Local title — local before Pull · 2026-07-29 18:42Z"
                    .to_string(),
                desired_original_name: Some("Cloud title".to_string()),
                codex_executable: "/tmp/codex".to_string(),
                codex_executable_sha256: "d".repeat(64),
                available_executors: vec![
                    crate::project_sync::domain::CodexForkExecutor::AppServerStdio,
                    crate::project_sync::domain::CodexForkExecutor::NativePty,
                    crate::project_sync::domain::CodexForkExecutor::Manual,
                ],
                pre_existing_children: Vec::new(),
            }),
            source_resource_id: Some(
                ResourceId::parse("codex:conversation:thread-shared").unwrap(),
            ),
            proposed_provider_thread_id: None,
            required_codec: CODEX_LINEAGE_CODEC_KEY.to_string(),
            pinned_generation: 1,
            pinned_commit_id: "commit-divergence".to_string(),
            pinned_manifest_sha256: "a".repeat(64),
            pinned_lineage_digest: Some("f".repeat(64)),
            pinned_semantic_digest: Some("1".repeat(64)),
            expected_local_target: TargetExpectation::Sha256("2".repeat(64)),
            expected_provider_state: vec![ProviderStateExpectation {
                target_path: "/tmp/source-native.jsonl".to_string(),
                expectation: TargetExpectation::Sha256("2".repeat(64)),
            }],
            expected_profile_revision: 0,
            expected_project_binding_revision: 0,
            expected_host_identity_revision: 0,
            expected_bindings_document_revision: 0,
            expected_binding_revision: Some(0),
            expires_at: 2_000,
        };
        RestorePlan {
            schema_version: crate::project_sync::domain::RESTORE_PLAN_SCHEMA,
            plan_id: PlanId::parse("plan-divergence").unwrap(),
            storage_id: StorageId::parse("storage-divergence").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            host_id,
            replica_id,
            expected_host_identity_revision: 0,
            generation: 1,
            commit_id: "commit-divergence".to_string(),
            manifest_sha256: "a".repeat(64),
            binding_revision: 0,
            created_at: 1_000,
            expires_at: 2_000,
            actions: Vec::new(),
            conversation_actions: vec![action],
            project_content_eligibility: None,
        }
    }

    #[test]
    fn pull_log_helpers_count_and_label_selectable_conversations() {
        let plan = divergence_selection_test_plan();
        let action = &plan.conversation_actions[0];

        assert_eq!(pull_plan_action_counts(&plan), (0, 1));
        assert_eq!(
            pull_review_resource_labels(&plan),
            vec!["codex:conversation:thread-shared".to_string()]
        );
        assert_eq!(
            pull_action_resource_label(&plan, &action.action_id).as_deref(),
            Some("codex:conversation:thread-shared")
        );
    }

    #[test]
    fn divergence_resolution_selection_is_exact_and_cloud_only_is_destructive() {
        let plan = divergence_selection_test_plan();
        let action_id = plan.conversation_actions[0].action_id.clone();
        let approved = vec![action_id.clone()];

        let missing = split_approved_restore_actions(&approved, &[], &plan).unwrap_err();
        assert!(missing.contains("lacks a resolution"), "{missing}");

        let keep_both = ConversationResolutionSelection {
            action_id: action_id.clone(),
            resolution: ConversationDivergenceResolution::PreserveLocalWithNativeFork,
            destructive_approved: false,
        };
        let duplicate = split_approved_restore_actions(
            &approved,
            &[keep_both.clone(), keep_both.clone()],
            &plan,
        )
        .unwrap_err();
        assert!(duplicate.contains("more than once"), "{duplicate}");

        let unconfirmed = split_approved_restore_actions(
            &approved,
            &[ConversationResolutionSelection {
                action_id: action_id.clone(),
                resolution: ConversationDivergenceResolution::UseCloudOnly,
                destructive_approved: false,
            }],
            &plan,
        )
        .unwrap_err();
        assert!(
            unconfirmed.contains("requires destructive approval"),
            "{unconfirmed}"
        );

        let (_, selected) = split_approved_restore_actions(
            &approved,
            &[ConversationResolutionSelection {
                action_id,
                resolution: ConversationDivergenceResolution::UseCloudOnly,
                destructive_approved: true,
            }],
            &plan,
        )
        .unwrap();
        assert_eq!(
            selected[0].resolution,
            Some(ConversationDivergenceResolution::UseCloudOnly)
        );
    }

    #[test]
    fn production_push_holds_a_divergent_tip_and_pushes_the_rest() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("conversation-push-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        let binding =
            bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let remote_cwd = temp.path().join("remote-project");
        let remote_tip = command_test_transcript(
            "provider-t111",
            &remote_cwd,
            &[
                ("base-user", "user", "base"),
                ("base-agent", "assistant", "answer"),
                ("remote-tip", "user", "remote branch"),
            ],
        );
        let (source, _t1, t11, t111) = publish_three_level_chain(
            &repo,
            &project,
            &binding,
            &storage_id,
            &remote_cwd,
            remote_tip,
        );
        let local_provider_id = "019fa661-49d6-7642-8aee-ebc2ef0929e6";
        let local_tip = command_test_transcript(
            local_provider_id,
            &project_root,
            &[
                ("base-user", "user", "base"),
                ("base-agent", "assistant", "answer"),
                ("local-tip", "user", "local branch"),
            ],
        );
        let local_path = bind_local_conversation(
            &repo,
            &binding,
            &codex_home,
            &t111,
            Some(t11.clone()),
            local_provider_id,
            &local_tip,
            &source,
        );
        // A second, unrelated local-only conversation stays pushable while
        // the divergent tip is held.
        let fresh_provider_id = "019fa661-49d6-7642-8aee-ebc2ef092900";
        let fresh_transcript = command_test_transcript(
            fresh_provider_id,
            &project_root,
            &[("fresh-user", "user", "a brand new local thread")],
        );
        let fresh_path = codex_home.join("sessions/2026/07/28").join(format!(
            "rollout-2026-07-28T12-00-00-{}.jsonl",
            fresh_provider_id
        ));
        std::fs::create_dir_all(fresh_path.parent().unwrap()).unwrap();
        std::fs::write(&fresh_path, &fresh_transcript).unwrap();

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let comparison = review.thread_comparison.as_ref().unwrap();
        let tip = comparison
            .entries
            .iter()
            .find(|entry| entry.cloud_thread_id.as_ref() == Some(&t111))
            .unwrap();
        assert_eq!(tip.relation, Some(ConversationRelation::Diverged));
        assert_eq!(
            tip.push_decision.action,
            ConversationPlanActionType::RequirePull
        );
        assert!(!tip.push_decision.selectable);
        assert!(!tip.push_decision.selected_by_default);
        let fresh = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(fresh_provider_id))
            .unwrap();
        assert_eq!(
            fresh.push_decision.action,
            ConversationPlanActionType::CreateCloud
        );
        assert!(fresh.push_decision.selectable);
        let plan_id = review.conversation_push_plan_id.clone().unwrap();
        let divergent_action_id = review.conversation_action_ids[&tip.resource_id].clone();
        let fresh_action_id = review.conversation_action_ids[&fresh.resource_id].clone();
        let plan = repo.load_conversation_push_plan(&plan_id).unwrap();
        let action = plan
            .actions
            .iter()
            .find(|action| action.action_id == divergent_action_id)
            .unwrap()
            .clone();
        assert_eq!(action.action, ConversationPlanActionType::RequirePull);

        // Approving the held thread is the only hard error.
        let selected_blocker = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            None,
            empty_project_content_review(),
            Some(plan_id.clone()),
            vec![divergent_action_id],
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            selected_blocker.contains("Pull and resolve"),
            "{selected_blocker}"
        );

        let omitted_plan = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            None,
            empty_project_content_review(),
            None,
            Vec::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            omitted_plan.contains("requires a current Push review"),
            "{omitted_plan}"
        );

        // The rest of the project pushes while the divergent tip is held.
        let pushed = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            None,
            empty_project_content_review(),
            Some(plan_id),
            vec![fresh_action_id],
            &mut |_| {},
        )
        .unwrap();
        assert!(pushed.success, "{}", pushed.message);

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let published = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(published.manifest.conversations.len(), 4);
        // The divergent thread keeps its cloud version, byte for byte.
        let republished_tip = published.manifest.conversations.get(&t111).unwrap();
        let source_tip = source.manifest.conversations.get(&t111).unwrap();
        assert_eq!(republished_tip.forked_from, source_tip.forked_from);
        let tip_transcript_path = republished_tip
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .unwrap()
            .logical_path
            .clone();
        assert_eq!(
            published
                .manifest
                .files
                .get(&tip_transcript_path)
                .unwrap()
                .sha256,
            source
                .manifest
                .files
                .get(&tip_transcript_path)
                .unwrap()
                .sha256,
        );
        // The local divergent branch was neither pushed nor modified.
        assert_eq!(std::fs::read(&local_path).unwrap(), local_tip);
        assert!(published
            .manifest
            .conversations
            .values()
            .any(|thread| thread.source_thread_id.as_deref() == Some(fresh_provider_id)));
        let bindings = repo.load_conversation_bindings().unwrap();
        let original_binding = bindings
            .bindings
            .iter()
            .find(|candidate| {
                candidate.local_project_id == project.local_project_id
                    && candidate.thread_id == t111
            })
            .unwrap();
        assert_eq!(original_binding.forked_from, Some(t11));
        assert_eq!(
            original_binding.provider_thread_id.as_deref(),
            Some(local_provider_id)
        );
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
            None,
            Vec::new(),
            &mut |_| {},
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
            None,
            Vec::new(),
            &mut |_| {},
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
            None,
            Vec::new(),
            &mut |_| {},
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
            None,
            Vec::new(),
            &mut |_| {},
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
            None,
            Vec::new(),
            &mut |_| {},
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
            None,
            Vec::new(),
            &mut |_| {},
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

        let mut expected = vec![
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
        ];
        // The deep link carries the path percent-encoded, so the quoting rules
        // differ from the CLI lines above.
        #[cfg(target_os = "macos")]
        expected.push((
            "Codex app — new",
            "'/usr/bin/open' -n -a '/Applications/ChatGPT.app' --env 'CODEX_HOME=/tmp/custom codex/.codex' 'codex://threads/new?path=%2Ftmp%2Fclient%27s%20project'"
                .to_string(),
        ));
        expected.extend([
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
        ]);
        assert_eq!(project_open_commands(&binding), expected);
    }

    #[test]
    fn conversation_path_repair_only_rewrites_desktop_assigned_threads() {
        let temp = tempfile::tempdir().unwrap();
        let repo =
            LocalStateRepository::from_home_dir(temp.path().join("agent-sync-home")).unwrap();
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
    fn oversized_conversation_is_scoped_to_the_conversation_instead_of_blocking_the_project() {
        let temp = tempfile::tempdir().unwrap();
        let repo =
            LocalStateRepository::from_home_dir(temp.path().join("agent-sync-home")).unwrap();
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join(".codex");
        let sessions = codex_home.join("sessions/2026/07/28");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
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

        let thread_id = "019f7500-0000-7000-8000-000000000099";
        let transcript = sessions.join(format!("rollout-{}.jsonl", thread_id));
        let mut transcript_header = serde_json::to_vec(&serde_json::json!({
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "cwd": project_root.to_str().unwrap(),
            },
        }))
        .unwrap();
        transcript_header.push(b'\n');
        std::fs::write(&transcript, transcript_header).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&transcript)
            .unwrap()
            .set_len(MAX_CONVERSATION_FILE_BYTES + 1)
            .unwrap();
        std::fs::write(
            codex_home.join(".codex-global-state.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "thread-writable-roots": {
                    (thread_id): [project_root.to_str().unwrap()]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let audit =
            audit_codex_conversation_paths_with_repository(&repo, &project.local_project_id)
                .unwrap();
        assert_eq!(audit.assigned_thread_count, 1);
        assert_eq!(audit.matching_thread_count, 0);
        assert!(audit.issues.is_empty());
        assert!(audit.blockers.is_empty());
        assert!(audit.ready);
        assert!(!audit.can_repair);
        assert_eq!(audit.warnings.len(), 1);
        assert!(audit.warnings[0].contains("unavailable for sync"));
        require_codex_conversation_paths_ready(&repo, &project.local_project_id).unwrap();
    }

    #[test]
    fn nested_fork_path_repair_accepts_ancestor_metadata_and_rewrites_its_cwds() {
        let active = "019f7500-0000-7000-8000-000000000111";
        let parent = "019f7500-0000-7000-8000-000000000011";
        let grandparent = "019f7500-0000-7000-8000-000000000001";
        let root = "019f7500-0000-7000-8000-000000000000";
        let source = [
            serde_json::json!({"type":"session_meta","payload":{"id":active,"forked_from_id":parent,"cwd":"/tmp/old-active"}}),
            serde_json::json!({"type":"session_meta","payload":{"id":parent,"forked_from_id":grandparent,"cwd":"/tmp/old-parent"}}),
            serde_json::json!({"type":"session_meta","payload":{"id":grandparent,"forked_from_id":root,"cwd":"/tmp/old-grandparent"}}),
            serde_json::json!({"type":"session_meta","payload":{"id":root,"cwd":"/tmp/old-root"}}),
            serde_json::json!({"type":"turn_context","payload":{"cwd":"/tmp/old-turn"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","id":"m-a","role":"user","content":[{"type":"input_text","text":"unchanged"}]}}),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();

        let stale =
            inspect_codex_structural_cwds(&source, active, Path::new("/tmp/new-project")).unwrap();
        assert_eq!(stale.len(), 5);
        let repaired = remap_codex_structural_cwds(&source, active, "/tmp/new-project").unwrap();
        let structure = analyze_codex_rollout_structure(&repaired, Some(active)).unwrap();
        assert_eq!(
            structure.fork_ancestry,
            vec![
                parent.to_string(),
                grandparent.to_string(),
                root.to_string()
            ]
        );
        assert_eq!(
            structure.structural_cwds,
            BTreeSet::from(["/tmp/new-project".to_string()])
        );
        assert!(String::from_utf8(repaired).unwrap().contains("unchanged"));
    }

    #[test]
    fn nested_fork_path_audit_rejects_unrelated_metadata() {
        let source = [
            serde_json::json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent","cwd":"/tmp/project"}}),
            serde_json::json!({"type":"session_meta","payload":{"id":"unrelated","cwd":"/tmp/project"}}),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
        let error =
            inspect_codex_structural_cwds(&source, "child", Path::new("/tmp/project")).unwrap_err();
        assert!(error.contains("unrelated"));
    }

    #[test]
    fn refresh_and_push_auto_select_new_project_conversations() {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("shared").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
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
                command_test_transcript(
                    id,
                    &project_root,
                    &[(&format!("message-{id}"), "user", "hello")],
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
        ] {
            assert!(inventory
                .recipe
                .entries
                .contains_key(&ResourceId::parse(resource_id).unwrap()));
        }
        assert!(!inventory
            .recipe
            .entries
            .contains_key(&ResourceId::parse("codex:session-index").unwrap()));

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
            command_test_transcript(
                third_id,
                &project_root,
                &[("message-third", "user", "hello")],
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
        assert!(pushed.success);
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let snapshot = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(
            snapshot.head.schema_version,
            crate::project_sync::domain::BUNDLE_SCHEMA
        );
        snapshot.manifest.published_by_origin.validate().unwrap();
        assert!(snapshot
            .manifest
            .conversations
            .values()
            .any(|thread| thread.source_thread_id.as_deref() == Some(third_id)));
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
        assert!(
            !pushed_empty.results.is_empty()
                && pushed_empty
                    .results
                    .iter()
                    .all(|result| result.state == "synced" || result.state == "unchanged"),
            "clearing local defaults must retain previously published conversations"
        );
        assert!(engine
            .inspect(&project.bundle_id)
            .unwrap()
            .manifest
            .conversations
            .values()
            .any(|thread| thread.source_thread_id.as_deref() == Some(third_id)));
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
        assert_eq!(comparison.counts.synced, 3);
        assert_eq!(comparison.counts.local, 0);
        assert_eq!(comparison.counts.storage, 0);
        assert_eq!(comparison.counts.diverged, 0);
        assert!(comparison
            .entries
            .iter()
            .all(|entry| entry.state == ThreadSyncState::Synced));
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
        config.storages.push(StorageConfig {
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
        config.storages.push(StorageConfig {
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
    fn unlinked_project_can_adopt_a_matching_remote_bundle_when_storage_is_added_later() {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let storage_id = StorageId::parse("shared").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
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

    #[test]
    fn pull_support_uses_the_restore_plans_storage_when_linked_generations_differ() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let first_storage_id = StorageId::parse("first-linked-store").unwrap();
        let selected_storage_id = StorageId::parse("selected-pull-store").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.extend([
            StorageConfig {
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
            StorageConfig {
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

        // Identical content republishes as a no-op, so each push appends a
        // turn to an owned Codex session first to genuinely advance the
        // generation on its storage.
        let session_id = "019f7500-0000-7000-8000-0000000000aa";
        let sessions = temp.path().join("codex-home/sessions/2026/07/20");
        std::fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join(format!("rollout-{}.jsonl", session_id));
        std::fs::write(
            &rollout,
            command_test_transcript(session_id, &project_root, &[]),
        )
        .unwrap();
        let mut turn = 0_usize;
        let mut advance_local_content =
            move || {
                turn += 1;
                std::fs::OpenOptions::new()
                .append(true)
                .open(&rollout)
                .and_then(|mut file| {
                    use std::io::Write as _;
                    let parent_id = (turn > 1).then(|| format!("turn-{}", turn - 1));
                    writeln!(file, "{}", serde_json::json!({
                        "type": "response_item",
                        "payload": {
                            "type": "message",
                            "id": format!("turn-{turn}"),
                            "parent_id": parent_id,
                            "role": "user",
                            "content": [{"type": "input_text", "text": format!("turn {turn}")}],
                        },
                    }))
                })
                .unwrap();
            };
        for _ in 0..2 {
            advance_local_content();
            push_bundle_with_repository(&repo, &project.local_project_id, &first_storage_id)
                .unwrap();
        }
        for _ in 0..3 {
            advance_local_content();
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

        assert_eq!(restore.storage_id, selected_storage_id);
        assert_eq!(restore.generation, 3);
        assert_eq!(
            get_restore_readiness_with_repository(&repo, &restore.plan_id)
                .unwrap()
                .bundle_id,
            restore.bundle_id
        );

        advance_local_content();
        push_bundle_with_repository(&repo, &project.local_project_id, &selected_storage_id)
            .unwrap();
        assert!(
            get_restore_readiness_with_repository(&repo, &restore.plan_id)
                .unwrap_err()
                .contains("bundle head changed after restore planning")
        );
    }

    #[test]
    fn push_without_changes_is_a_noop_that_records_the_base() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("noop-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);

        let session_id = "019f7500-0000-7000-8000-0000000000bb";
        let sessions = codex_home.join("sessions/2026/07/21");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join(format!("rollout-{}.jsonl", session_id)),
            command_test_transcript(
                session_id,
                &project_root,
                &[("message-noop", "user", "hello")],
            ),
        )
        .unwrap();

        let first =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert!(first.success);
        assert_eq!(first.generation, Some(1));
        assert!(
            first.message.contains("Published generation 1"),
            "{}",
            first.message
        );
        let first_base = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .cloned()
            .unwrap();

        let repeat =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert!(repeat.success);
        assert_eq!(repeat.generation, Some(1));
        assert!(
            repeat.message.contains("Already up to date"),
            "{}",
            repeat.message
        );
        assert!(!repeat.results.is_empty());
        assert!(repeat
            .results
            .iter()
            .all(|result| result.state == "unchanged"));
        assert_eq!(repeat.resources_changed, Some(0));

        let repeat_base = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .cloned()
            .unwrap();
        assert_eq!(repeat_base.generation, first_base.generation);
        assert_eq!(repeat_base.manifest_sha256, first_base.manifest_sha256);
        assert!(repeat_base.last_push_at >= first_base.last_push_at);

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        assert_eq!(
            engine.inspect(&project.bundle_id).unwrap().head.generation,
            1
        );
    }

    #[test]
    fn missing_remote_history_requires_an_explicit_unlink_before_republishing() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("missing-history-store").unwrap();
        let storage_root = temp.path().join("store");
        add_local_storage(&repo, &storage_id, &storage_root);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        write_codex_session(
            &codex_home,
            "019f7500-0000-7000-8000-0000000000bc",
            &project_root,
        );

        let first =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(first.generation, Some(1));
        let reviewed_base = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .cloned()
            .unwrap();
        let prepared_before_remote_loss =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            prepared_before_remote_loss.destination_status,
            PushDestinationStatus::Ready {
                generation: Some(1),
            }
        );

        let repository_root = storage_root
            .join(".mallard/v1/repositories")
            .join(project.bundle_id.as_str());
        std::fs::remove_dir_all(&repository_root).unwrap();

        let error =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap_err();
        assert!(error.contains("remote history is missing"));
        let context =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            context.destination_status,
            PushDestinationStatus::RemoteHistoryMissing { base_generation: 1 }
        );
        assert_eq!(
            get_push_destination_status_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap(),
            context.destination_status
        );

        let retained_base = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .cloned()
            .unwrap();
        assert_eq!(retained_base, reviewed_base);

        assert!(
            remove_project_link_with_repository(&repo, &project.local_project_id, &storage_id,)
                .unwrap()
        );
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        assert_eq!(
            get_push_destination_status_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap(),
            PushDestinationStatus::Ready { generation: None }
        );

        let republished =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(republished.generation, Some(1));
    }

    /// Timestamps differ between two builds of the same section; everything
    /// else must match exactly.
    fn strip_volatile(mut value: serde_json::Value) -> serde_json::Value {
        fn strip(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(map) => {
                    map.remove("compared_at");
                    map.remove("generated_at");
                    map.remove("prepared_at");
                    map.remove("scanned_at");
                    for child in map.values_mut() {
                        strip(child);
                    }
                }
                serde_json::Value::Array(items) => {
                    for child in items {
                        strip(child);
                    }
                }
                _ => {}
            }
        }
        strip(&mut value);
        value
    }

    fn write_codex_session(codex_home: &Path, session_id: &str, project_root: &Path) {
        let sessions = codex_home.join("sessions/2026/07/22");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join(format!("rollout-{}.jsonl", session_id)),
            command_test_transcript(
                session_id,
                project_root,
                &[("message-test", "user", "hello")],
            ),
        )
        .unwrap();
    }

    #[test]
    fn prepare_push_review_matches_standalone_commands() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("composite-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let session_id = "019f7500-0000-7000-8000-0000000000cc";
        write_codex_session(&codex_home, session_id, &project_root);

        // Standalone commands run first so auto-selection persistence is
        // settled before both sides build their sections.
        let standalone_inventory =
            get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        let standalone_threads = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();

        let context =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            strip_volatile(serde_json::to_value(&context.inventory).unwrap()),
            strip_volatile(serde_json::to_value(&standalone_inventory).unwrap()),
        );
        assert_eq!(
            strip_volatile(serde_json::to_value(&context.thread_comparison).unwrap()),
            strip_volatile(serde_json::to_value(Some(standalone_threads)).unwrap()),
        );
        assert!(context.thread_comparison_error.is_none());
        // A non-git binding keeps the project-files section absent by design.
        assert!(context.project_files.is_none());
        assert!(context.project_files_error.is_none());
        // Auto-selection persisted through the composite as well.
        let recipe = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .clone();
        assert!(recipe
            .entries
            .contains_key(&ResourceId::parse(format!("codex:session:{}", session_id)).unwrap()));
        assert_eq!(
            context.project.project.local_project_id,
            project.local_project_id
        );
        assert_eq!(
            context.destination_status,
            PushDestinationStatus::Ready { generation: None }
        );
    }

    #[test]
    fn subagent_tasks_are_removed_from_inventory_recipe_and_push() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("subagent-push-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);

        let parent_id = "019f7500-0000-7000-8000-0000000000ee";
        let child_id = "019f7500-0000-7000-8000-0000000000ef";
        write_codex_session(&codex_home, parent_id, &project_root);
        let child_path = codex_home
            .join("sessions/2026/07/22")
            .join(format!("rollout-{}.jsonl", child_id));
        std::fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        std::fs::write(
            &child_path,
            command_test_subagent_transcript(
                child_id,
                parent_id,
                &project_root,
                &[("message-child", "assistant", "internal work")],
            ),
        )
        .unwrap();

        let child_resource = ResourceId::parse(format!("codex:session:{}", child_id)).unwrap();
        repo.mutate_config(|config| {
            let current = config
                .projects
                .iter_mut()
                .find(|candidate| candidate.local_project_id == project.local_project_id)
                .unwrap();
            current.recipe.entries.insert(
                child_resource.clone(),
                RecipeEntry {
                    resource_id: child_resource.clone(),
                    apply_policy: ApplyPolicy::SafeFile,
                    required: false,
                },
            );
            current.recipe.revision = current.recipe.revision.saturating_add(1);
            current.revision = current.revision.saturating_add(1);
            Ok(())
        })
        .unwrap();

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(review.inventory.resources.iter().all(|resource| {
            resource.descriptor.resource_id.as_str() != child_resource.as_str()
        }));
        assert!(review
            .thread_comparison
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .all(|entry| entry.provider_thread_id.as_deref() != Some(child_id)));
        assert!(!repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .entries
            .contains_key(&child_resource));

        push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let snapshot = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(snapshot.manifest.conversations.len(), 1);
        assert!(snapshot
            .manifest
            .conversations
            .values()
            .all(|thread| { thread.source_thread_id.as_deref() != Some(child_id) }));
        assert!(!snapshot
            .manifest
            .recipe
            .entries
            .contains_key(&child_resource));
    }

    #[test]
    fn previously_stored_subagent_tasks_are_hidden_from_comparison_and_pull() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("subagent-pull-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        let binding =
            bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let (origin_host_token, origin_replica_token) = repo
            .derive_origin_tokens(&project.bundle_id, &binding.replica_id)
            .unwrap();
        let origin = PortableOrigin {
            origin_host_token,
            origin_replica_token,
            shared_label: None,
        };
        let parent_provider_id = "019f7500-0000-7000-8000-0000000000f0";
        let child_provider_id = "019f7500-0000-7000-8000-0000000000f1";
        let parent = ThreadSpec::new(
            "stored-parent",
            parent_provider_id,
            command_test_transcript(
                parent_provider_id,
                &project_root,
                &[("message-parent", "user", "parent")],
            ),
            origin.clone(),
            10,
        );
        let child = ThreadSpec::new(
            "stored-subagent",
            child_provider_id,
            command_test_subagent_transcript(
                child_provider_id,
                parent_provider_id,
                &project_root,
                &[("message-child", "assistant", "internal work")],
            ),
            origin.clone(),
            11,
        );
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let published = engine
            .publish(publish_request(
                project.bundle_id.clone(),
                &[parent, child],
                origin,
                11,
                None,
                PublishExpectation::Absent,
            ))
            .unwrap();
        assert_eq!(published.snapshot.manifest.conversations.len(), 2);

        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(comparison.entries.len(), 1);
        assert_eq!(comparison.entries[0].thread_id.as_str(), parent_provider_id);
        assert_eq!(
            comparison.entries[0]
                .cloud_thread_id
                .as_ref()
                .map(ThreadId::as_str),
            Some("stored-parent")
        );

        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &project.bundle_id, &binding)
                .unwrap();
        assert!(restore.actions.is_empty());
        assert_eq!(restore.conversation_actions.len(), 1);
        assert_eq!(
            restore.conversation_actions[0].thread_id.as_str(),
            "stored-parent"
        );
    }

    #[test]
    fn prepare_push_review_degrades_per_section_when_storage_unreachable() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("broken-store").unwrap();
        let storage_root = temp.path().join("breakable-store");
        add_local_storage(&repo, &storage_id, &storage_root);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        write_codex_session(
            &codex_home,
            "019f7500-0000-7000-8000-0000000000dd",
            &project_root,
        );
        // Break the storage after linking: a file where the store directory
        // was makes every engine construction fail.
        std::fs::remove_dir_all(&storage_root).unwrap();
        std::fs::write(&storage_root, b"not a directory").unwrap();

        let context =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(!context.inventory.resources.is_empty());
        assert!(context.thread_comparison.is_none());
        assert!(
            context.thread_comparison_error.is_some(),
            "storage failure surfaces as a section error"
        );
        assert!(context.project_files.is_none());
        assert!(matches!(
            context.destination_status,
            PushDestinationStatus::Unavailable { .. }
        ));
    }

    #[test]
    fn pull_review_apply_flow_materializes_a_codex_session_for_the_replica() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("pull-store").unwrap();
        let bundle_id = BundleId::parse("df29babc833808e68ad0efa4d01d7d6d").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
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
        let transcript = command_test_transcript(
            session_id,
            &source_root,
            &[("message-source", "user", "change the color")],
        );
        let source_session = source_codex.join(&session_relative);
        std::fs::create_dir_all(source_session.parent().unwrap()).unwrap();
        std::fs::write(&source_session, &transcript).unwrap();
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

        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &target.local_project_id,
            &storage_id,
        )
        .unwrap();
        let stored_thread = comparison
            .entries
            .iter()
            .find(|entry| entry.thread_id == session_id)
            .unwrap();
        let cloud_thread_id = stored_thread.cloud_thread_id.clone().unwrap();
        assert_eq!(stored_thread.state, ThreadSyncState::StorageOnly);
        assert_eq!(stored_thread.thread_id, session_id);
        assert_eq!(
            stored_thread.resource_id,
            conversation_resource_id(Provider::Codex, &cloud_thread_id).unwrap()
        );
        assert_eq!(stored_thread.display_name, "Change glb color to yellow");
        assert!(
            !target_codex.join(&session_relative).exists(),
            "title lookup must not materialize the stored transcript"
        );
        assert!(
            !target_codex.join("session_index.jsonl").exists(),
            "title lookup must not apply the stored session index"
        );

        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert!(
            restore.actions.is_empty(),
            "typed conversations do not use generic file actions"
        );
        assert_eq!(restore.conversation_actions.len(), 1);
        let conversation_action = restore.conversation_actions[0].clone();
        assert_eq!(
            conversation_action.action,
            ConversationPlanActionType::CreateLocal
        );
        let before =
            get_bundle_readiness_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(before.state, "needs_setup");

        let applied = apply_bundle_restore_with_repository(
            &repo,
            &restore.plan_id,
            vec![conversation_action.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{}", applied.message);
        assert_eq!(
            applied.applied_action_ids,
            vec![conversation_action.action_id.clone()]
        );

        let restored_session_path = conversation_action
            .expected_provider_state
            .iter()
            .find(|target| !target.target_path.ends_with("session_index.jsonl"))
            .unwrap()
            .target_path
            .clone();
        let restored_session = std::fs::read_to_string(&restored_session_path).unwrap();
        let rows = restored_session
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows[0]["payload"]["cwd"], target_root.to_str().unwrap());
        let index = std::fs::read_to_string(target_codex.join("session_index.jsonl")).unwrap();
        let index_rows = index
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(index_rows.len(), 1);
        assert_eq!(index_rows[0]["thread_name"], "Change glb color to yellow");
        assert_eq!(
            index_rows[0]["id"], rows[0]["payload"]["id"],
            "the index row must name the rewritten provider thread"
        );
        assert!(
            index_rows[0].get("cwd").is_none(),
            "index rows use the provider schema, not Mallard-invented fields"
        );
        chrono::DateTime::parse_from_rfc3339(index_rows[0]["updated_at"].as_str().unwrap())
            .expect("index row carries an RFC 3339 update time");
        let materializations = repo.load_materializations().unwrap();
        let target_materialization = materializations
            .records
            .iter()
            .find(|record| record.local_project_id == target.local_project_id)
            .unwrap();
        assert_eq!(
            target_materialization.status,
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
            .find(|entry| entry.cloud_thread_id.as_ref() == Some(&cloud_thread_id))
            .unwrap();
        assert_eq!(
            restored_thread.state,
            ThreadSyncState::Synced,
            "replica-specific cwd rewriting must not look like a local edit"
        );
    }

    #[test]
    fn pull_applies_the_rest_while_a_divergent_thread_stays_unresolved() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("mixed-pull-store").unwrap();
        let bundle_id = BundleId::parse("df29babc833808e68ad0efa4d01d7d6e").unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Mixed pull store".to_string(),
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
                display_name: "sourceProject".to_string(),
                repository_fingerprint: Some("b".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let source_root = temp.path().join("sourceProject");
        let source_codex = temp.path().join("source-codex");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(&source_codex).unwrap();
        let shared_session_id = "019f0fb9-b140-7af3-8b7c-5d75c974b231";
        let shared_relative = format!(
            "sessions/2026/07/18/rollout-2026-07-18T00-00-00-{}.jsonl",
            shared_session_id
        );
        let shared_base = [
            ("message-a", "user", "hello"),
            ("message-b", "assistant", "world"),
        ];
        let source_session = source_codex.join(&shared_relative);
        std::fs::create_dir_all(source_session.parent().unwrap()).unwrap();
        std::fs::write(
            &source_session,
            command_test_transcript(shared_session_id, &source_root, &shared_base),
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
                display_name: "targetProject".to_string(),
                repository_fingerprint: Some("b".repeat(64)),
                bundle_id: Some(bundle_id.clone()),
            },
        )
        .unwrap();
        let target_root = temp.path().join("targetProject");
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

        // First pull materializes the shared thread on the target replica.
        let first =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        assert_eq!(first.conversation_actions.len(), 1);
        let materialize_action = first.conversation_actions[0].clone();
        assert_eq!(
            materialize_action.action,
            ConversationPlanActionType::CreateLocal
        );
        let applied = apply_bundle_restore_with_repository(
            &repo,
            &first.plan_id,
            vec![materialize_action.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{}", applied.message);
        let target_session_path = materialize_action
            .expected_provider_state
            .iter()
            .find(|state| !state.target_path.ends_with("session_index.jsonl"))
            .unwrap()
            .target_path
            .clone();
        let materialized = std::fs::read_to_string(&target_session_path).unwrap();
        let first_row: serde_json::Value =
            serde_json::from_str(materialized.lines().next().unwrap()).unwrap();
        let target_provider_id = first_row["payload"]["id"].as_str().unwrap().to_string();

        // The thread now diverges: a local target branch and a newer cloud
        // branch, while the source also adds a second, unrelated thread.
        let divergent_local = command_test_transcript(
            &target_provider_id,
            &target_root,
            &[
                shared_base[0],
                shared_base[1],
                ("target-tip", "user", "local branch"),
            ],
        );
        std::fs::write(&target_session_path, &divergent_local).unwrap();
        std::fs::write(
            &source_session,
            command_test_transcript(
                shared_session_id,
                &source_root,
                &[
                    shared_base[0],
                    shared_base[1],
                    ("cloud-tip", "user", "cloud branch"),
                ],
            ),
        )
        .unwrap();
        let fresh_session_id = "019f0fb9-b140-7af3-8b7c-5d75c974b232";
        let fresh_session = source_codex.join(format!(
            "sessions/2026/07/19/rollout-2026-07-19T00-00-00-{}.jsonl",
            fresh_session_id
        ));
        std::fs::create_dir_all(fresh_session.parent().unwrap()).unwrap();
        std::fs::write(
            &fresh_session,
            command_test_transcript(
                fresh_session_id,
                &source_root,
                &[("fresh-message", "user", "another thread")],
            ),
        )
        .unwrap();
        push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();

        // The divergent thread is offered for optional resolution (or held
        // when no fork contract can be built on this machine); the fresh
        // thread stays independently pullable either way.
        let mixed =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        let divergent_action = mixed
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == materialize_action.thread_id)
            .unwrap();
        assert_eq!(divergent_action.relation, ConversationRelation::Diverged);
        assert!(
            matches!(
                divergent_action.action,
                ConversationPlanActionType::ResolveDivergence
                    | ConversationPlanActionType::StoreOnly
            ),
            "divergent thread must stay reviewable, got {:?}",
            divergent_action.action
        );
        let fresh_action = mixed
            .conversation_actions
            .iter()
            .find(|action| action.action == ConversationPlanActionType::CreateLocal)
            .unwrap()
            .clone();
        assert_ne!(fresh_action.thread_id, divergent_action.thread_id);

        // Pull only the fresh thread; the divergence stays unresolved and
        // the pass still settles completely.
        let partial = apply_bundle_restore_with_repository(
            &repo,
            &mixed.plan_id,
            vec![fresh_action.action_id.clone()],
        )
        .unwrap();
        assert!(partial.success, "{}", partial.message);
        assert!(matches!(partial.status, RestoreApplyStatus::Complete));
        assert!(partial.codex_fork_checkpoints.is_empty());
        assert_eq!(partial.applied_action_ids, vec![fresh_action.action_id]);
        assert!(partial.failed_actions.is_empty());

        // The divergent local branch is untouched; the fresh thread landed.
        assert_eq!(
            std::fs::read(&target_session_path).unwrap(),
            divergent_local
        );
        let fresh_target_path = fresh_action
            .expected_provider_state
            .iter()
            .find(|state| !state.target_path.ends_with("session_index.jsonl"))
            .unwrap()
            .target_path
            .clone();
        assert!(Path::new(&fresh_target_path).exists());

        // The reviewed baseline advances even though a divergence remains.
        let config = repo.load_config().unwrap();
        let base = config
            .project(&target.local_project_id)
            .unwrap()
            .recipe_bases
            .get(&storage_id)
            .unwrap();
        assert_eq!(base.generation, mixed.generation);
        assert!(base.last_pull_at.is_some());

        // A fresh review still offers the unresolved divergence and asks
        // nothing further for the settled thread.
        let replanned =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        let replanned_divergent = replanned
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == materialize_action.thread_id)
            .unwrap();
        assert_eq!(replanned_divergent.relation, ConversationRelation::Diverged);
        assert!(!replanned
            .conversation_actions
            .iter()
            .any(|action| action.action == ConversationPlanActionType::CreateLocal));
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
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(&home).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let profile_id = add_profile(&repo, Provider::Codex, &temp.path().join("codex-home"));
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
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
        let repo = LocalStateRepository::from_home_dir(&home).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
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
        let repo = LocalStateRepository::from_home_dir(&home).unwrap();
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
        config.storages.push(StorageConfig {
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
        repo: LocalStateRepository,
        project_dir: PathBuf,
        codex_home: PathBuf,
        storage_dir: PathBuf,
    }

    fn setup_fixture() -> SetupFixture {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
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

    fn local_storage(fixture: &SetupFixture, id: &str) -> StorageConfig {
        StorageConfig {
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

    /// Persist the signatures the client would stamp after reviewing a
    /// discovery, so later inspections compare against a settled selection.
    fn accept_current_discovery(
        fixture: &SetupFixture,
        draft: ProjectSetupDraft,
    ) -> ProjectSetupDraft {
        let inspection =
            inspect_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap();
        let mut accepted = draft;
        accepted.discovery_signature = inspection.fresh_discovery_signature.unwrap();
        accepted.discovery_profile_signature = inspection.fresh_profile_signature.unwrap();
        update_setup_draft_with_repository(&fixture.repo, accepted).unwrap()
    }

    #[test]
    fn repointing_a_profile_reads_as_rediscovery_not_drift() {
        let fixture = setup_fixture();
        let draft = accept_current_discovery(&fixture, draft_ready_to_finalize(&fixture));

        let settled = inspect_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap();
        assert!(!settled.profile_changed);
        assert!(!settled.selection_stale);

        let other_home = fixture
            .storage_dir
            .parent()
            .unwrap()
            .join("codex-alt/.codex");
        std::fs::create_dir_all(&other_home).unwrap();
        let mut repointed = draft;
        repointed.profiles = BTreeMap::from([(
            Provider::Codex,
            DraftProfileSelection::Pending {
                path: other_home.to_string_lossy().into_owned(),
                display_name: String::new(),
            },
        )]);
        let repointed = update_setup_draft_with_repository(&fixture.repo, repointed).unwrap();

        let inspection =
            inspect_setup_draft_with_repository(&fixture.repo, &repointed.draft_id).unwrap();
        assert!(inspection.profile_changed);
        // The two are mutually exclusive: a different home is not drift.
        assert!(!inspection.selection_stale);
        assert_ne!(
            inspection.fresh_profile_signature.as_deref(),
            Some(repointed.discovery_profile_signature.as_str())
        );
        let resources = inspection
            .sections
            .iter()
            .find(|section| section.section == "resources")
            .unwrap();
        assert_eq!(resources.state, "ready");
        assert!(inspection.can_finalize);
    }

    #[test]
    fn drift_under_an_unchanged_profile_still_flags_the_selection() {
        let fixture = setup_fixture();
        let mut draft = accept_current_discovery(&fixture, draft_ready_to_finalize(&fixture));
        // Same home, different discovered set.
        draft.discovery_signature = "b".repeat(64);
        let draft = update_setup_draft_with_repository(&fixture.repo, draft).unwrap();

        let inspection =
            inspect_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap();
        assert!(inspection.selection_stale);
        assert!(!inspection.profile_changed);
        let resources = inspection
            .sections
            .iter()
            .find(|section| section.section == "resources")
            .unwrap();
        assert_eq!(resources.state, "attention");
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
    fn github_repository_identity_ignores_dot_git_and_transport() {
        let remotes = [
            "https://github.com/Owner/repository",
            "https://github.com/Owner/repository.git",
            "ssh://git@github.com/Owner/repository.git",
            "git@github.com:Owner/repository.git",
        ];
        let expected = repository_identity_from_remote_url(remotes[0]).fingerprint;
        for remote in remotes {
            assert_eq!(
                repository_identity_from_remote_url(remote).fingerprint,
                expected,
                "remote should canonicalize: {remote}"
            );
        }
    }

    #[test]
    fn github_repository_identity_requires_the_current_canonical_fingerprint() {
        let identity =
            repository_identity_from_remote_url("https://github.com/Owner/repository.git");
        assert!(identity.matches(Some(&repository_url_fingerprint(
            "https://github.com/Owner/repository"
        ))));
        assert!(!identity.matches(Some(&repository_url_fingerprint(
            "https://github.com/Owner/repository.git"
        ))));
        assert!(!identity.matches(Some(&repository_url_fingerprint(
            "https://github.com/Owner/different-repository"
        ))));
        assert!(!identity.matches(None));
    }
}
