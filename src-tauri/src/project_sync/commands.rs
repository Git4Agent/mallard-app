//! Tauri command surface for the current project-sync implementation.
//!
//! This layer composes provider capture, the bundle engine, and local
//! persistence. Local-folder and S3-compatible storage use the same immutable
//! bundle publication and head-CAS protocol.

use super::bundle_engine::{
    absent_scope_pin, write_immutable_backup, write_target_atomic, BundleBytes, BundleEngine,
    CapturedBundleFile, FetchProgress, ObjectCache, PublishBundleRequest, PublishExpectation,
    RemoteBundlePage, ScopePublishInput,
};
use super::codex_app_refresh::{
    refresh_offer as codex_app_refresh_offer, restart_after_pull, CodexAppKind,
    CodexAppRefreshOffer, CodexAppRefreshState, CodexAppRestartResult,
};
use super::codex_history_graph;
use super::codex_history_graph::CodexSeededRoot;
use super::codex_path_map::{validate_pathmap_root, CodexPathMapper};
use super::codex_rollout::analyze_codex_rollout_structure;
use super::codex_standalone_builder::{
    build_standalone_codex_root, StandaloneBuildMode, StandaloneConversationArtifact,
};
use super::codex_thread_allocator;
use super::conversation_codec::{analyze_codex_transcript, CodecLineage};
use super::conversation_materialization::{
    apply_conversation_cloud_publications_locked, apply_conversation_materialization_locked,
    recover_conversation_materializations, ConversationCloudPublication,
    ConversationMaterializationRequest, ConversationTargetPayload, NoConversationFailpoints,
};
use super::conversation_sync::{
    build_conversation_plan_action, validate_conversation_action_pins, ConversationApplyContext,
    ConversationPlanActionRequest,
};
use super::domain::{
    absent_bundle_head_pin, classify_remote_continuity, classify_scope_continuity,
    compare_conversation_tail, compose_preserved_title, conversation_resource_id,
    conversation_transcript_path, fallback_origin_host_label, generated_named_id,
    generated_uuid_v4, initial_conversation_thread_id, parse_conversation_resource_id,
    retained_conversation_thread_id, validate_absolute_clean_path, ActionId, ActionStatus,
    ApplyPolicy, ApplyReceipt, BindingResolution, BindingState, BundleDirectoryEntry, BundleId,
    BundleIdentity, BundleKind, BundleManifest, BundleRecipe, BundleSnapshot, CapturedWith,
    ClassifiedThread, ConversationActionPins, ConversationBlocker, ConversationCapabilities,
    ConversationCheckpoint, ConversationCloudAnchor, ConversationContextPins, ConversationDecision,
    ConversationFileRef, ConversationFileRole, ConversationLineageStatus,
    ConversationMaterializationOperation, ConversationMaterializationState,
    ConversationOperationId, ConversationOperationState, ConversationPlanAction,
    ConversationPlanActionType, ConversationPlanDirection, ConversationPublicationOperation,
    ConversationPushPlan, ConversationReviewAction, ConversationReviewRow,
    ConversationRevisionPins, ConversationSyncOperation, ConversationSyncOperationKind,
    ConversationTailPins, ConversationTargetPins, ConversationThread, DraftProfileSelection,
    DraftRepositoryChoice, DraftStorageSelection, HostId, KeepBothJournal, KeepBothPlan,
    LocalConversationBinding, LocalProjectId, LocalProjectRegistration, LocalProviderProfileId,
    LogicalPath, MachineProjectState, MaterializationId, MaterializationRecord,
    MaterializationStatus, OriginHostAlias, OriginHostToken, PhysicalMappingState, PlanId,
    PortableConversationTail, PortableOrigin, PreservedProvenance, ProjectBinding,
    ProjectFileSyncEligibility, ProjectFileSyncEligibilityState, ProjectSetupDraft,
    ProjectStorageLink, Provenance, ProviderKey, ProviderProfile, ProviderStateExpectation,
    PullJournalPhase, PushJournalPhase, RawRangePin, RecipeEntry, RemoteContinuity, ReplicaId,
    RepositoryBase, ResourceDescriptor, ResourceId, ResourceKind, RestoreActionKind,
    RestoreActionType, RestorePlan, ReviewPurpose, RootTailLayout, ScopeBase, ScopeContinuity,
    SetupDraftId, SetupTransaction, StorageConfig, StorageId, StorageKind, SyncConfig,
    SyncScopeKey, TailClassification, TailEvidenceBlocker, TailPartitionKind, TailSideEvidence,
    TargetExpectation, ThreadId, TombstoneTarget, VerifiedTailEvidence,
    MAX_CONVERSATION_FILE_BYTES, PUSH_PLAN_SCHEMA, SETUP_DRAFT_SCHEMA, SETUP_TRANSACTION_SCHEMA,
};
use super::persistence::{
    LocalStateRepository, ProjectSyncMetadataResetResult, StorageSyncMetadataResetResult,
};
use super::project_activity::{
    self, ProjectActivity, ProviderAccessState, ResolvedActivityProject,
};
use super::provider_capture::{
    self, CaptureApplyPolicy, CaptureRequest, CaptureResourceKind, Discovery, LocalSessionView,
    ResourceCandidate,
};
use super::providers::codex::first_codex_user_summary;
use super::providers::{
    ProviderDescriptor, ProviderRegistry, SessionEventDetail, SessionTimelineEventKind,
    SessionToolCategory,
};
use super::service::{ProjectSyncService, PushApproval};
use super::session_detail::{
    self, ProjectSessionDetail, ProjectSessionTimelinePage, ProjectSessionTimelineRequest,
    SessionDetailError, SessionDetailErrorCode,
};
use super::storage_core::{
    clear_managed_storage_data, ConfiguredObjectStore, ManagedStorageClearResult, VerifiedBody,
};
use crate::activity_log::{emit_typed_log, ActivityLogScope, ActivityLogType};
use chrono::{TimeZone, Utc};
use mallard_application::storage_setup::{
    connect_r2_storage as connect_r2_storage_service, validate_r2_storage_destinations,
    ConnectR2StorageRequest, ConnectStorageResult,
};
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

/// The Codex provider key, used only by inherently Codex-specific
/// maintenance flows (desktop-app refresh and path repair).
/// Common setup/Activity/continuity/review code never names a provider.
fn codex_key() -> ProviderKey {
    ProviderKey::parse("codex").expect("static provider key")
}

fn provider_registry() -> &'static ProviderRegistry {
    ProviderRegistry::production()
}

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
    pub profile_ids: BTreeMap<ProviderKey, LocalProviderProfileId>,
    /// Required when changing an existing binding.  Creation uses `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct StorageDataClearResult {
    pub storage_id: StorageId,
    pub config_revision: u64,
    pub repository_objects_deleted: u64,
    pub library_objects_deleted: u64,
    pub project_links_reset: u64,
    pub catalog_links_reset: u64,
    pub local_records_removed: u64,
    pub unreadable_plan_files_retained: u64,
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
    pub storage_revision: Option<u64>,
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
pub(super) struct ProjectContentPushReview {
    pub(super) review_token: Option<String>,
    pub(super) removal_ids: BTreeSet<ResourceId>,
    pub(super) acknowledged_warning_digests: BTreeSet<String>,
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
    pub provider: ProviderKey,
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
    pub revision: Option<u64>,
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
    pub action: ConversationReviewAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<ConversationBlocker>,
    pub selectable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

impl From<ConversationDecision> for ThreadSyncDecision {
    fn from(decision: ConversationDecision) -> Self {
        Self {
            action: decision.action,
            blocker: decision.blocker,
            selectable: decision.selectable,
            blocked_reason: decision.blocked_reason,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ThreadSyncEntry {
    pub provider: ProviderKey,
    pub provider_display_name: String,
    pub access: ProviderAccessState,
    pub thread_id: String,
    /// Portable conversation identity shared with Activity rows. The capture
    /// resource ID remains separate because reviewed Push/Pull actions use it.
    pub conversation_resource_id: ResourceId,
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
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    pub compared_at: u64,
    pub entries: Vec<ThreadSyncEntry>,
    pub counts: ThreadSyncCounts,
    /// Per-scope continuity rows: the project scope plus every provider
    /// scope present locally or remotely, each with its own reviewed base.
    #[serde(default)]
    pub scopes: Vec<ScopeSyncRow>,
    /// Per-provider review failures (unreadable profile, missing adapter).
    #[serde(default)]
    pub provider_errors: BTreeMap<ProviderKey, String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Advisory repository-level continuity between the recorded base and
    /// the observed head (DESIGN_REMOTE_HISTORY_CONTINUITY.md). Explains a
    /// wiped-and-recreated storage wherever the comparison is shown.
    pub remote_continuity: RemoteContinuity,
}

/// One backend read shared by the Activity and storage-comparison
/// projections. A comparison failure remains local to that projection so a
/// temporarily unavailable storage cannot hide local Activity.
#[derive(Serialize, Clone, Debug)]
pub struct ProjectActivityWorkspace {
    pub activity: ProjectActivity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ThreadSyncComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_error: Option<String>,
}

/// One sync scope's continuity as shown in Push/Pull review.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ScopeSyncRow {
    /// Canonical scope key string ("project" or "provider:<key>").
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKey>,
    pub access: ProviderAccessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    pub continuity: ScopeContinuity,
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
    pub revision: Option<u64>,
    #[serde(default)]
    pub results: Vec<OperationResourceResult>,
}

#[derive(Serialize, Clone, Debug)]
pub struct RemoteBundleSummaryDto {
    pub bundle_id: BundleId,
    pub display_name: String,
    pub kind: BundleKind,
    pub revision: u64,
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
    pub revision: u64,
    pub updated_at: u64,
    pub resource_count: usize,
    #[serde(default)]
    pub providers: Vec<ProviderKey>,
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
    RecoveryRequired,
}

/// The preserved side of one applied divergent keep-both.
#[derive(Serialize, Clone, Debug)]
pub struct PreservedThreadResult {
    pub action_id: ActionId,
    pub original_thread_id: ThreadId,
    pub preserved_thread_id: ThreadId,
    /// The plan-minted marker title the preserved copy was renamed to.
    pub preserved_display_title: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct RestoreResult {
    pub success: bool,
    pub status: RestoreApplyStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<PlanId>,
    /// Whether this apply advanced the reviewed recipe base to the plan's
    /// revision. `Some(false)` means required actions were skipped, so Push
    /// stays gated on this storage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_base_advanced: Option<bool>,
    #[serde(default)]
    pub applied_action_ids: Vec<ActionId>,
    #[serde(default)]
    pub failed_actions: Vec<FailedAction>,
    #[serde(default)]
    pub preserved_threads: Vec<PreservedThreadResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_app_refresh: Option<CodexAppRefreshOffer>,
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
    pub provider: Option<ProviderKey>,
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
    _app: &tauri::AppHandle<R>,
) -> Result<LocalStateRepository, String> {
    LocalStateRepository::from_user_home()
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

async fn run_session_detail_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, SessionDetailError> + Send + 'static,
) -> Result<T, SessionDetailError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| SessionDetailError {
            code: SessionDetailErrorCode::ProviderError,
            message: "The session-detail worker stopped unexpectedly.".to_string(),
            current_snapshot_id: None,
        })?
}

/// Process-wide verified-object cache shared by planning, apply, and thread
/// comparison. Exact content-coordinate keys make entries immortal-correct;
/// the cache bounds retained staging-disk use.
fn object_cache() -> Arc<ObjectCache> {
    static OBJECT_CACHE: OnceLock<Arc<ObjectCache>> = OnceLock::new();
    OBJECT_CACHE
        .get_or_init(|| Arc::new(ObjectCache::default()))
        .clone()
}

type CachedCodexTranscriptAnalysis = Arc<super::conversation_codec::CodexTranscriptAnalysis>;
type TranscriptAnalysisCache = Mutex<HashMap<(String, u64), CachedCodexTranscriptAnalysis>>;

/// Codex transcript analyses keyed by the exact transcript size/hash
/// coordinate. Analysis is pure on the bytes, so entries never go stale.
/// Sole caller today: Pull planning's stored-subagent refusal, which opens
/// only offerable transcripts.
fn cached_transcript_analysis(
    sha256: &str,
    expected_size: u64,
    bytes: &mut dyn BundleBytes,
    logical_path: &super::domain::LogicalPath,
) -> Result<CachedCodexTranscriptAnalysis, String> {
    if expected_size > MAX_CONVERSATION_FILE_BYTES {
        return Err("cloud conversation transcript exceeds its read limit".to_string());
    }
    static ANALYSES: OnceLock<TranscriptAnalysisCache> = OnceLock::new();
    let cache = ANALYSES.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_key = (sha256.to_string(), expected_size);
    if let Some(hit) = cache
        .lock()
        .ok()
        .and_then(|entries| entries.get(&cache_key).cloned())
    {
        return Ok(hit);
    }
    let transcript = bytes.read_bounded(
        logical_path,
        MAX_CONVERSATION_FILE_BYTES,
        "cloud conversation transcript",
    )?;
    let analysis = Arc::new(analyze_codex_transcript(&transcript));
    if let Ok(mut entries) = cache.lock() {
        if entries.len() >= 4_096 {
            entries.clear();
        }
        entries.insert(cache_key, analysis.clone());
    }
    Ok(analysis)
}

#[tauri::command]
pub async fn get_project_sync_config(app: tauri::AppHandle) -> Result<SyncConfig, String> {
    repository(&app)?.load_config()
}

/// Focused R2 creation path shared with `mallard-cli`. The secret-bearing
/// request cannot be debugged or serialized back out, and every returned
/// service error is fixed, typed, and credential-free.
#[tauri::command]
pub async fn connect_r2_storage(
    app: tauri::AppHandle,
    request: ConnectR2StorageRequest,
) -> Result<ConnectStorageResult, String> {
    let repository = repository(&app)?;
    let runtime = tokio::runtime::Handle::current();
    run_blocking(move || {
        connect_r2_storage_service(&repository, runtime, request).map_err(|error| error.to_string())
    })
    .await
}

fn packaged_cli_path(current_executable: &Path) -> Result<PathBuf, String> {
    let directory = current_executable
        .parent()
        .ok_or_else(|| "Mallard cannot resolve its packaged CLI location.".to_string())?;
    let filename = if cfg!(windows) {
        "mallard-cli.exe"
    } else {
        "mallard-cli"
    };
    let candidate = directory.join(filename);
    let metadata = candidate
        .symlink_metadata()
        .map_err(|_| "mallard-cli is not available in this build.".to_string())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("mallard-cli is not available in this build.".to_string());
    }
    let canonical_directory = directory
        .canonicalize()
        .map_err(|_| "Mallard cannot resolve its packaged CLI location.".to_string())?;
    let canonical_candidate = candidate
        .canonicalize()
        .map_err(|_| "Mallard cannot resolve its packaged CLI location.".to_string())?;
    if canonical_candidate.parent() != Some(canonical_directory.as_path()) {
        return Err("Mallard cannot resolve its packaged CLI location.".to_string());
    }
    Ok(canonical_candidate)
}

/// Resolve the CLI beside the running desktop executable. Tauri's sidecar
/// bundler places it in the same executable directory on macOS and Windows.
#[tauri::command]
pub fn get_mallard_cli_path() -> Result<String, String> {
    let current = std::env::current_exe()
        .map_err(|_| "Mallard cannot resolve its packaged CLI location.".to_string())?;
    packaged_cli_path(&current)?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Mallard cannot represent its packaged CLI path.".to_string())
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
    let previous = repository.load_config().ok();
    if let Some(previous) = &previous {
        for storage in &previous.storages {
            if !config
                .storages
                .iter()
                .any(|candidate| candidate.id == storage.id)
            {
                crate::storage_catalog::ensure_storage_removal_allowed(&storage.id)?;
            }
        }
    }
    let storage_change = previous
        .as_ref()
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
    validate_r2_storage_destinations(&config).map_err(|error| error.to_string())?;
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
    run_blocking(move || project_activity::list_project_repository_kinds(&repository)).await
}

#[tauri::command]
pub async fn list_agent_providers(
    app: tauri::AppHandle,
) -> Result<Vec<ProviderDescriptor>, String> {
    let _ = app;
    Ok(ProviderRegistry::production().descriptors())
}

#[allow(clippy::too_many_arguments)]
fn build_activity_from_local_read(
    repository: &LocalStateRepository,
    local: &ProjectLocalRead,
    remote_snapshot: Option<&BundleSnapshot>,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: bool,
    mut warnings: Vec<String>,
) -> Result<ProjectActivity, String> {
    let local_sessions = project_activity::scan_local_session_views(
        repository,
        provider_registry(),
        &local.activity_project,
        &local.views,
        force_revalidate,
        &mut warnings,
    )?;
    let remote_sessions = remote_snapshot
        .map(project_activity::remote_sessions_from_snapshot)
        .unwrap_or_default();
    project_activity::build_project_activity(
        repository,
        provider_registry(),
        &local.activity_project,
        local_sessions,
        remote_sessions,
        branch,
        before_time,
        window_days,
        warnings,
    )
}

#[allow(clippy::too_many_arguments)]
fn get_project_activity_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: Option<&StorageId>,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: bool,
) -> Result<ProjectActivity, String> {
    let local = load_project_local_read(repository, local_project_id)?;
    let mut warnings = Vec::new();
    let remote = match storage_id {
        Some(storage_id) => match project_activity::fetch_activity_snapshot(
            repository,
            storage_id,
            &local.activity_project.bundle_id,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warnings.push(format!(
                    "remote sessions are unavailable from '{}': {}",
                    storage_id, error
                ));
                None
            }
        },
        None => None,
    };
    build_activity_from_local_read(
        repository,
        &local,
        remote.as_ref(),
        branch,
        before_time,
        window_days,
        force_revalidate,
        warnings,
    )
}

#[allow(clippy::too_many_arguments)]
fn get_project_activity_workspace_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: Option<&StorageId>,
    branch: Option<&str>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: bool,
) -> Result<ProjectActivityWorkspace, String> {
    let local = load_project_local_read(repository, local_project_id)?;
    let mut warnings = Vec::new();
    let mut remote_error = None;
    let remote = match storage_id {
        Some(storage_id) => {
            match load_project_remote_read(repository, local_project_id, storage_id) {
                Ok(remote) => Some(remote),
                Err(error) => {
                    warnings.push(format!(
                        "remote sessions are unavailable from '{}': {}",
                        storage_id, error
                    ));
                    remote_error = Some(error);
                    None
                }
            }
        }
        None => None,
    };
    let activity = build_activity_from_local_read(
        repository,
        &local,
        remote.as_ref().and_then(|remote| remote.current.as_ref()),
        branch,
        before_time,
        window_days,
        force_revalidate,
        warnings,
    )?;
    let (comparison, comparison_error) = match (storage_id, remote.as_ref()) {
        (Some(storage_id), Some(remote)) => {
            match build_comparison_from_reads(
                repository,
                local_project_id,
                storage_id,
                &local,
                remote,
                &mut |_, _| {},
            ) {
                Ok(comparison) => (Some(comparison), None),
                Err(error) => (None, Some(error)),
            }
        }
        (Some(_), None) => (None, remote_error),
        (None, _) => (None, None),
    };
    Ok(ProjectActivityWorkspace {
        activity,
        comparison,
        comparison_error,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_activity(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
    branch: Option<String>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: Option<bool>,
) -> Result<ProjectActivity, String> {
    let repository = repository(&app)?;
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    let force_revalidate = force_revalidate.unwrap_or(false);
    let log_routine_scan = should_log_history_scan(&local_project_id, force_revalidate);
    if log_routine_scan {
        log.emit(
            &app,
            "info",
            "history.scan_started",
            &format!("Scanning agent session activity for project {local_project_id}…"),
        );
    }
    let result = run_blocking(move || {
        get_project_activity_with_repository(
            &repository,
            &local_project_id,
            storage_id.as_ref(),
            branch.as_deref(),
            before_time,
            window_days,
            force_revalidate,
        )
    })
    .await;
    match &result {
        Ok(activity) => {
            for warning in &activity.warnings {
                log.emit(&app, "warning", "history.scan_warning", warning);
            }
            if log_routine_scan {
                log.emit(
                    &app,
                    "info",
                    "history.scan_completed",
                    &format!(
                        "Session activity scan complete: {} sessions in this {}-day window",
                        activity.sessions.len(),
                        activity.window_end.saturating_sub(activity.window_start) / (24 * 60 * 60)
                    ),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "history.scan_failed",
            &format!("Session activity scan failed: {error}"),
        ),
    }
    result
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_activity_workspace(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
    branch: Option<String>,
    before_time: Option<u64>,
    window_days: Option<u64>,
    force_revalidate: Option<bool>,
) -> Result<ProjectActivityWorkspace, String> {
    let repository = repository(&app)?;
    let log = ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    let force_revalidate = force_revalidate.unwrap_or(false);
    let log_routine_scan = should_log_history_scan(&local_project_id, force_revalidate);
    if log_routine_scan {
        log.emit(
            &app,
            "info",
            "history.scan_started",
            &format!("Scanning agent session activity for project {local_project_id}…"),
        );
    }
    let result = run_blocking(move || {
        get_project_activity_workspace_with_repository(
            &repository,
            &local_project_id,
            storage_id.as_ref(),
            branch.as_deref(),
            before_time,
            window_days,
            force_revalidate,
        )
    })
    .await;
    match &result {
        Ok(workspace) => {
            for warning in &workspace.activity.warnings {
                log.emit(&app, "warning", "history.scan_warning", warning);
            }
            if log_routine_scan {
                log.emit(
                    &app,
                    "info",
                    "history.scan_completed",
                    &format!(
                        "Session activity scan complete: {} sessions in this {}-day window",
                        workspace.activity.sessions.len(),
                        workspace
                            .activity
                            .window_end
                            .saturating_sub(workspace.activity.window_start)
                            / (24 * 60 * 60)
                    ),
                );
            }
        }
        Err(error) => log.emit(
            &app,
            "error",
            "history.scan_failed",
            &format!("Session activity scan failed: {error}"),
        ),
    }
    result
}

fn session_detail_error(
    code: SessionDetailErrorCode,
    message: impl Into<String>,
) -> SessionDetailError {
    SessionDetailError {
        code,
        message: message.into(),
        current_snapshot_id: None,
    }
}

fn load_project_session_views(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    resource_id: &str,
) -> Result<Vec<LocalSessionView>, SessionDetailError> {
    let (provider, thread_id) = parse_conversation_resource_id(resource_id).map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::InvalidRequest,
            "The session resource ID is invalid.",
        )
    })?;
    let activity_project = project_activity::resolve_activity_project(repository, local_project_id)
        .map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::OwnershipChanged,
                "The project or its provider profile changed; open Project Settings.",
            )
        })?;
    let home = activity_project.homes.get(&provider).ok_or_else(|| {
        session_detail_error(
            SessionDetailErrorCode::NotLocal,
            "This session is not materialized in a readable local provider profile.",
        )
    })?;
    let home_path = home.home.to_string_lossy().into_owned();
    let bound_provider_session_id = repository
        .load_conversation_bindings()
        .map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::OwnershipChanged,
                "Mallard could not revalidate this session's local ownership.",
            )
        })?
        .bindings
        .into_iter()
        .find(|binding| {
            binding.local_project_id == *local_project_id
                && binding.thread_id == thread_id
                && binding.provider_profile_id == home.profile_id
        })
        .and_then(|binding| binding.provider_thread_id);
    let resolved = ResolvedBinding {
        binding: activity_project.binding.clone(),
        homes: [(provider, home_path)].into_iter().collect(),
    };
    let capture_request =
        capture_request_for_resolved_binding(repository, &resolved).map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::OwnershipChanged,
                "Mallard could not revalidate this session's local ownership.",
            )
        })?;
    let (_, views) = super::codex_watch_service::session_watch_service()
        .discovery_for(provider_registry(), local_project_id, &capture_request)
        .map_err(|_| {
            session_detail_error(
                SessionDetailErrorCode::ProviderError,
                "The provider could not safely discover local sessions.",
            )
        })?;
    let mut matching = Vec::new();
    for view in views {
        let identity_matches = match bound_provider_session_id.as_ref() {
            Some(expected) => expected == &view.provider_thread_id,
            None => {
                project_activity::resolve_local_provider_alias_thread(
                    repository,
                    &activity_project,
                    &view.provider,
                    &view.provider_thread_id,
                    &view.source_path,
                )
                .map_err(|_| {
                    session_detail_error(
                        SessionDetailErrorCode::OwnershipChanged,
                        "Mallard could not revalidate this session's local ownership.",
                    )
                })?
                .as_ref()
                    == Some(&thread_id)
            }
        };
        if identity_matches {
            matching.push(view);
        }
    }
    Ok(matching)
}

pub(super) fn get_project_session_detail_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    resource_id: &str,
) -> Result<ProjectSessionDetail, SessionDetailError> {
    rebaseline_project_provider_caches(repository, local_project_id).map_err(|_| {
        session_detail_error(
            SessionDetailErrorCode::OwnershipChanged,
            "The project or its provider profile changed; open Project Settings.",
        )
    })?;
    let views = load_project_session_views(repository, local_project_id, resource_id)?;
    session_detail::get_project_session_detail(
        repository,
        provider_registry(),
        local_project_id,
        resource_id,
        &views,
    )
}

pub(super) fn get_project_session_timeline_page_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    request: ProjectSessionTimelineRequest,
) -> Result<ProjectSessionTimelinePage, SessionDetailError> {
    let views = load_project_session_views(repository, local_project_id, resource_id)?;
    session_detail::get_project_session_timeline_page(
        repository,
        provider_registry(),
        local_project_id,
        resource_id,
        request,
        &views,
    )
}

pub(super) fn get_project_session_event_detail_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    resource_id: &str,
    snapshot_id: &str,
    event_id: &str,
) -> Result<SessionEventDetail, SessionDetailError> {
    let views = load_project_session_views(repository, local_project_id, resource_id)?;
    session_detail::get_project_session_event_detail(
        repository,
        provider_registry(),
        local_project_id,
        resource_id,
        snapshot_id,
        event_id,
        &views,
    )
}

#[tauri::command]
pub async fn get_project_session_detail(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    resource_id: String,
) -> Result<ProjectSessionDetail, SessionDetailError> {
    let repository = repository(&app).map_err(|_| SessionDetailError {
        code: SessionDetailErrorCode::ProviderError,
        message: "Mallard could not open its local state repository.".to_string(),
        current_snapshot_id: None,
    })?;
    run_session_detail_blocking(move || {
        get_project_session_detail_with_repository(&repository, &local_project_id, &resource_id)
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_session_timeline_page(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    resource_id: String,
    snapshot_id: String,
    cursor: Option<String>,
    anchor_event_id: Option<String>,
    limit: Option<usize>,
    included_event_kinds: Vec<SessionTimelineEventKind>,
    included_tool_categories: Vec<SessionToolCategory>,
    turn_id: Option<String>,
) -> Result<ProjectSessionTimelinePage, SessionDetailError> {
    let repository = repository(&app).map_err(|_| SessionDetailError {
        code: SessionDetailErrorCode::ProviderError,
        message: "Mallard could not open its local state repository.".to_string(),
        current_snapshot_id: None,
    })?;
    run_session_detail_blocking(move || {
        get_project_session_timeline_page_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
            ProjectSessionTimelineRequest {
                snapshot_id,
                cursor,
                anchor_event_id,
                limit,
                included_event_kinds: included_event_kinds.into_iter().collect(),
                included_tool_categories: included_tool_categories.into_iter().collect(),
                turn_id,
            },
        )
    })
    .await
}

#[tauri::command]
pub async fn get_project_session_event_detail(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    resource_id: String,
    snapshot_id: String,
    event_id: String,
) -> Result<SessionEventDetail, SessionDetailError> {
    let repository = repository(&app).map_err(|_| SessionDetailError {
        code: SessionDetailErrorCode::ProviderError,
        message: "Mallard could not open its local state repository.".to_string(),
        current_snapshot_id: None,
    })?;
    run_session_detail_blocking(move || {
        get_project_session_event_detail_with_repository(
            &repository,
            &local_project_id,
            &resource_id,
            &snapshot_id,
            &event_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn open_project_markdown_path(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    relative_path: String,
) -> Result<(), SessionDetailError> {
    let repository = repository(&app).map_err(|_| SessionDetailError {
        code: SessionDetailErrorCode::ProviderError,
        message: "Mallard could not open its local state repository.".to_string(),
        current_snapshot_id: None,
    })?;
    run_session_detail_blocking(move || {
        let path = session_detail::resolve_project_markdown_path(
            &repository,
            &local_project_id,
            &relative_path,
        )?;
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|_| SessionDetailError {
            code: SessionDetailErrorCode::ProviderError,
            message: "Mallard could not open the linked project path.".to_string(),
            current_snapshot_id: None,
        })
    })
    .await
}

/// Bounded inline embedding for Markdown-referenced project images: the same
/// canonical/no-symlink descendant validation `open_project_markdown_path`
/// uses, plus a regular-file check, size cap, and magic-byte format sniff.
/// Only paths inside the registered project are ever served.
#[tauri::command]
pub async fn read_project_markdown_image(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    relative_path: String,
) -> Result<session_detail::ProjectMarkdownImage, SessionDetailError> {
    let repository = repository(&app).map_err(|_| SessionDetailError {
        code: SessionDetailErrorCode::ProviderError,
        message: "Mallard could not open its local state repository.".to_string(),
        current_snapshot_id: None,
    })?;
    run_session_detail_blocking(move || {
        session_detail::read_project_markdown_image(&repository, &local_project_id, &relative_path)
    })
    .await
}

#[tauri::command]
pub async fn execute_project_session_action(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    resource_id: String,
    action_id: String,
) -> Result<(), String> {
    let repository = repository(&app)?;
    let worker_app = app.clone();
    let worker_log =
        ActivityLogScope::new(ActivityLogType::History).project(&local_project_id, None);
    run_blocking(move || {
        project_activity::execute_project_session_action(
            &repository,
            ProviderRegistry::production(),
            &local_project_id,
            &resource_id,
            &action_id,
            &mut |command| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "history.session_action",
                    &format!("$ {command}"),
                )
            },
        )
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

/// Explicit escape hatch for derived machine-local sync state. The project
/// remains configured and no project/provider/cloud bytes are touched.
#[tauri::command]
pub async fn reset_project_sync_metadata(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    expected_project_revision: u64,
) -> Result<ProjectSyncMetadataResetResult, String> {
    let repository = repository(&app)?;
    let project_for_work = local_project_id.clone();
    let log = ActivityLogScope::new(ActivityLogType::Repair).project(&local_project_id, None);
    log.emit(
        &app,
        "warning",
        "repair.project_sync_metadata_reset_started",
        "Resetting this project's local sync metadata…",
    );
    let result = run_blocking(move || {
        repository.reset_project_sync_metadata(&project_for_work, expected_project_revision)
    })
    .await;
    match &result {
        Ok(summary) => log.emit(
            &app,
            "ok",
            "repair.project_sync_metadata_reset_completed",
            &format!(
                "Reset local sync metadata: {} conversation mappings, {} checkpoints, and {} saved results removed",
                summary.conversation_bindings_removed,
                summary.conversation_checkpoints_removed,
                summary.materializations_removed,
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "repair.project_sync_metadata_reset_failed",
            &format!("Local sync metadata reset failed: {error}"),
        ),
    }
    result
}

fn clear_storage_data_with_repositories(
    repository: &LocalStateRepository,
    catalog_repository: &crate::storage_catalog::persistence::CatalogStateRepository,
    storage_id: &StorageId,
    expected_config_revision: u64,
) -> Result<StorageDataClearResult, String> {
    let ((remote, local), catalog) = catalog_repository.clear_storage_data(storage_id, || {
        repository.clear_storage_data(storage_id, expected_config_revision, |storage| {
            let store = ConfiguredObjectStore::from_config(storage)?;
            clear_managed_storage_data(&store)
        })
    })?;
    Ok(storage_data_clear_result(remote, local, catalog))
}

fn storage_data_clear_result(
    remote: ManagedStorageClearResult,
    local: StorageSyncMetadataResetResult,
    catalog: crate::storage_catalog::persistence::CatalogStorageDataClearResult,
) -> StorageDataClearResult {
    let local_records_removed = local
        .scope_bases_removed
        .saturating_add(local.conversation_checkpoints_removed)
        .saturating_add(local.materializations_removed)
        .saturating_add(local.journal_operations_removed)
        .saturating_add(local.restore_plans_removed)
        .saturating_add(local.push_plans_removed)
        .saturating_add(catalog.plans_removed)
        .saturating_add(catalog.claims_removed)
        .saturating_add(catalog.preservation_witnesses_removed)
        .saturating_add(catalog.terminal_operations_removed);
    StorageDataClearResult {
        storage_id: local.storage_id,
        config_revision: local.config_revision,
        repository_objects_deleted: remote.repository_objects_deleted,
        library_objects_deleted: remote.library_objects_deleted,
        project_links_reset: local.links_reset,
        catalog_links_reset: catalog.links_reset,
        local_records_removed,
        unreadable_plan_files_retained: local.unreadable_plan_files_retained,
    }
}

/// Permanently delete every Mallard-managed repository and Library object in
/// one configured destination. The storage marker, connection settings,
/// project/provider files, and unrelated bucket/folder contents are retained.
#[tauri::command]
pub async fn clear_storage_data(
    app: tauri::AppHandle,
    storage_id: StorageId,
    expected_config_revision: u64,
) -> Result<StorageDataClearResult, String> {
    let repository = repository(&app)?;
    let storage_name = repository
        .load_config()?
        .storages
        .iter()
        .find(|storage| storage.id == storage_id)
        .map(|storage| storage.name.clone())
        .ok_or_else(|| format!("unknown storage '{}'", storage_id))?;
    let catalog_repository =
        crate::storage_catalog::persistence::CatalogStateRepository::from_user_home()?;
    let storage_for_work = storage_id.clone();
    let log =
        ActivityLogScope::new(ActivityLogType::Storage).storage(&storage_id, Some(&storage_name));
    log.emit(
        &app,
        "warning",
        "storage.data_clear_started",
        "Clearing all Mallard-managed data from storage…",
    );
    let result = run_blocking(move || {
        clear_storage_data_with_repositories(
            &repository,
            &catalog_repository,
            &storage_for_work,
            expected_config_revision,
        )
    })
    .await;
    match &result {
        Ok(summary) => log.emit(
            &app,
            "ok",
            "storage.data_clear_completed",
            &format!(
                "Cleared {} repository objects and {} Library objects",
                summary.repository_objects_deleted, summary.library_objects_deleted
            ),
        ),
        Err(error) => log.emit(
            &app,
            "error",
            "storage.data_clear_failed",
            &format!("Storage data could not be cleared: {error}"),
        ),
    }
    result
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
        // Scope bases live on the link itself and are removed with it.
        let removed = before != config.links.len();
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
    provider: ProviderKey,
    path: String,
) -> Result<ProviderProfileProbe, String> {
    let repository = repository(&app)?;
    run_blocking(move || probe_provider_profile_with_repository(&repository, &provider, &path))
        .await
}

#[tauri::command]
pub async fn create_provider_profile(
    app: tauri::AppHandle,
    provider: ProviderKey,
    display_name: String,
    path: String,
) -> Result<ProviderProfile, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        create_provider_profile_with_repository(&repository, &provider, &display_name, &path)
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
pub async fn discard_restore_plan(app: tauri::AppHandle, plan_id: PlanId) -> Result<bool, String> {
    repository(&app)?.discard_restore_plan(&plan_id)
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

/// Explicit durability audit: downloads and re-verifies every object the
/// current manifest references. This is the read-back Push deliberately does
/// not perform — it runs only when invoked.
#[tauri::command]
pub async fn verify_bundle_objects(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<super::bundle_engine::BundleObjectAudit, String> {
    let repository = repository(&app)?;
    run_blocking(move || {
        let config = repository.load_config()?;
        let project = config
            .project(&local_project_id)
            .cloned()
            .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
        require_project_link(&config, &project, &storage_id)?;
        let (_, engine) = storage_engine(&repository, &storage_id)?;
        engine.verify_objects(&project.bundle_id)
    })
    .await
}

#[tauri::command]
// The argument list is the frozen Tauri command contract.
#[allow(clippy::too_many_arguments)]
pub async fn push_bundle(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
    recipe: BundleRecipe,
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
    let selected_count = recipe.entries.len();
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
    let total_started = Instant::now();
    let result = run_blocking(move || {
        worker_log.emit(
            &worker_app,
            "info",
            "push.scan_started",
            &format!(
                "   Scanning {} resources selected for this storage…",
                selected_count
            ),
        );
        ProjectSyncService::new(&repository).push_reviewed_with_progress(
            &local_project_id,
            &storage_id,
            PushApproval {
                recipe,
                project_content_review_token,
                project_content_removal_ids: project_content_removal_ids.unwrap_or_default(),
                acknowledged_warning_digests: acknowledged_warning_digests.unwrap_or_default(),
                conversation_push_plan_id,
                approved_conversation_action_ids: approved_conversation_action_ids
                    .unwrap_or_default(),
            },
            &mut |update| {
                if let Some(emitter) = upload_progress.as_mut() {
                    emitter.observe(update);
                }
            },
            &mut |phase, elapsed| {
                worker_log.emit(
                    &worker_app,
                    "info",
                    "push.phase_timing",
                    &format!("   Push phase — {phase}: {:.3}s", elapsed.as_secs_f64()),
                );
            },
        )
    })
    .await;
    log.emit(
        &app,
        "info",
        "push.timing",
        &format!(
            "   Push total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
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
        ConversationPlanActionType::AdoptCloudMetadata
            | ConversationPlanActionType::MaterializeCloud
            | ConversationPlanActionType::KeepBoth
            | ConversationPlanActionType::CreateLocal
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
                .or_else(|| conversation_resource_id(&action.provider, &action.thread_id).ok())
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
                .or_else(|| conversation_resource_id(&action.provider, &action.thread_id).ok())
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
        ProjectSyncService::new(&repository).plan_bundle_restore_with_progress(
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
                    "✓  Pull plan ready — revision {}, {} applicable actions ({} resources, {} conversations)",
                    plan.revision,
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
        ProjectSyncService::new(&repository).apply_bundle_restore_with_progress(
            &plan_id,
            approved_action_ids,
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
                        if binding.homes.contains_key(&codex_key()) {
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

/// Provider facts come from the registered adapter descriptor; setup code
/// never hard-codes a provider list or name.
fn provider_descriptor(provider: &ProviderKey) -> Result<ProviderDescriptor, String> {
    provider_registry()
        .get(provider)
        .map(|adapter| adapter.descriptor())
        .ok_or_else(|| format!("no provider adapter is registered for '{}'", provider))
}

fn provider_name(provider: &ProviderKey) -> String {
    provider_descriptor(provider)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or_else(|_| provider.to_string())
}

fn provider_default_directory(provider: &ProviderKey) -> Option<String> {
    provider_descriptor(provider)
        .ok()
        .and_then(|descriptor| descriptor.default_profile_directory)
}

fn suggested_profile_name(provider: &ProviderKey, path: &Path) -> String {
    let default_directory = provider_default_directory(provider);
    let default = dirs::home_dir()
        .zip(default_directory.as_ref())
        .map(|(home, directory)| home.join(directory));
    if default.as_ref().is_some_and(|default| {
        prospective_canonical(default).ok().as_ref() == prospective_canonical(path).ok().as_ref()
    }) {
        return format!("Default {}", provider_name(provider));
    }
    let stem = if default_directory.as_ref().is_some_and(|directory| {
        path.file_name()
            .is_some_and(|name| name == directory.as_str())
    }) {
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
    provider: &ProviderKey,
    requested_path: &str,
) -> Result<ProviderProfileProbe, String> {
    // Probing creates local provider state authority, so the key must
    // resolve through the registry — unlike remote viewing, which accepts
    // any structurally valid key.
    provider_descriptor(provider)?;
    validate_absolute_clean_path("provider profile path", requested_path)?;
    let requested = PathBuf::from(requested_path);
    let default_directory = provider_default_directory(provider);
    let child = default_directory
        .as_ref()
        .map(|directory| requested.join(directory));
    let (resolved, detected_child) = match (child, &default_directory) {
        (Some(child), Some(directory))
            if requested
                .file_name()
                .is_none_or(|name| name != directory.as_str())
                && child.is_dir() =>
        {
            (child, true)
        }
        _ => (requested, false),
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
            &profile.provider == provider && Path::new(&profile.canonical_path) == canonical
        })
        .map(|profile| profile.profile_id.clone());
    Ok(ProviderProfileProbe {
        provider: provider.clone(),
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
    provider: &ProviderKey,
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
        provider: provider.clone(),
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

/// Auto-register default profiles for every provider the registry declares a
/// default home directory for. Registration-driven: a new provider needs no
/// change here.
fn ensure_default_provider_profiles(repository: &LocalStateRepository) -> Result<(), String> {
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    for descriptor in provider_registry().descriptors() {
        let Some(directory) = &descriptor.default_profile_directory else {
            continue;
        };
        let path = home.join(directory);
        if path.is_dir() {
            if probe_provider_profile_with_repository(
                repository,
                &descriptor.key,
                &path.to_string_lossy(),
            )
            .is_err()
            {
                continue;
            }
            let _ = create_provider_profile_with_repository(
                repository,
                &descriptor.key,
                &format!("Default {}", descriptor.display_name),
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
        if canonical != profile.canonical_path {
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
    profile_ids: &BTreeMap<ProviderKey, LocalProviderProfileId>,
) -> Result<BTreeMap<ProviderKey, String>, String> {
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
                provider_name(provider),
                provider_name(&profile.provider),
                profile.display_name
            ));
        }
        let (available, readable, _, error) = inspect_provider_profile(profile);
        if !available || !readable {
            return Err(error.unwrap_or_else(|| {
                format!(
                    "{} profile '{}' is not readable",
                    provider_name(provider),
                    profile.path
                )
            }));
        }
        // Every machine operation — capture, planning pins, and the
        // materialization boundary check — must agree on one spelling, so
        // the runtime home is always the profile's canonical path.
        resolved.insert(provider.clone(), profile.canonical_path.clone());
    }
    Ok(resolved)
}

/// Validate the project and provider-home boundaries setup will persist
/// without scanning any syncable resources.
fn validate_setup_paths(
    repository: &LocalStateRepository,
    selected_path: &str,
    profile_paths: &BTreeMap<ProviderKey, String>,
) -> Result<PathBuf, String> {
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
                provider_name(provider),
                path.display()
            ));
        }
    }
    Ok(project_root)
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
        Some(provider_capture::discover_full(
            provider_registry(),
            &request,
        )?)
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
    let (base_versions, base_unavailable) = match link.scope_bases.get(&SyncScopeKey::Project) {
        Some(base) => match engine.inspect_manifest_version(
            &project.bundle_id,
            base.reviewed_in.revision,
            &base.reviewed_in.commit_id,
            base.reviewed_in.manifest_size,
            &base.reviewed_in.manifest_sha256,
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
        storage_revision: remote.map(|snapshot| snapshot.head.revision),
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
    let Some(project_scope) = manifest.scope(&SyncScopeKey::Project) else {
        return Ok(versions);
    };
    for (logical_path, entry) in &project_scope.files {
        let Some(descriptor) = project_scope.resources.get(&entry.resource_id) else {
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
    for (logical_path, entry) in &project_scope.directories {
        let Some(descriptor) = project_scope.resources.get(&entry.resource_id) else {
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
        hasher.update(remote.head.revision.to_be_bytes());
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
    if canonical != binding.canonical_project_root {
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
    let discovery = provider_capture::discover_full(provider_registry(), &request)?;
    build_resource_inventory(repository, local_project_id, binding, &discovery)
}

/// Project/link recipes are machine-local selection state. A remote manifest's
/// generic resources can seed that state, but its conversation entries remain
/// cloud identities and are reviewed through `ThreadSyncComparison` instead.
fn machine_local_recipe_from_manifest(manifest: &BundleManifest) -> Result<BundleRecipe, String> {
    let mut recipe = BundleRecipe::default();
    for scope in manifest.scopes.values() {
        for (resource_id, entry) in &scope.recipe.entries {
            if parse_conversation_resource_id(resource_id.as_str()).is_err() {
                recipe.entries.insert(resource_id.clone(), entry.clone());
            }
        }
    }
    recipe.validate_machine_local()?;
    Ok(recipe)
}

/// Inventory section over a shared discovery. Inventory is read-only: a
/// reviewed Push is the only flow that changes a destination recipe.
fn build_resource_inventory(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    binding: &ProjectBinding,
    discovery: &provider_capture::Discovery,
) -> Result<ResourceInventory, String> {
    let discovered = discovery.project_inventory();
    let project = repository
        .load_config()?
        .project(local_project_id)
        .cloned()
        .ok_or_else(|| format!("unknown local project '{}'", local_project_id))?;
    let mut recipe = project.recipe.clone();
    recipe.validate_machine_local()?;
    remove_excluded_codex_subagents_from_recipe(
        &mut recipe,
        discovery.excluded_subagent_resource_ids(),
    );
    let mut inventory = inventory_from_candidates(
        binding.project_root.clone(),
        Some(project.bundle_id.clone()),
        &discovered.resources,
        recipe,
        discovered.warnings,
    )?;
    let visible: BTreeSet<_> = inventory
        .resources
        .iter()
        .map(|resource| resource.descriptor.resource_id.clone())
        .collect();
    let selected_entries = inventory.recipe.entries.clone();
    for (resource_id, entry) in &selected_entries {
        if visible.contains(resource_id) {
            continue;
        }
        inventory.resources.push(InventoryResource {
            descriptor: ResourceDescriptor {
                resource_id: resource_id.clone(),
                kind: ResourceKind::Requirement,
                provider: None,
                provider_resource_type: None,
                display_name: resource_id.to_string(),
                provenance: Provenance::Unknown,
                apply_policy: entry.apply_policy,
                relative_cwd: None,
                codec_version: 1,
                metadata: BTreeMap::new(),
            },
            category: "tools".to_string(),
            // One sentence, not two. This carried a `description` as well —
            // "Selected resource is unavailable on this machine" — which the
            // row printed above the blocker line that said the same thing in
            // different words. The blocked reason is the one that also drives
            // the row's state, so it is the one that speaks.
            description: None,
            logical_paths: Vec::new(),
            default_selected: true,
            blocked_reason: Some(
                "Selected for sync, but unavailable at the mapped paths on this machine"
                    .to_string(),
            ),
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
        let provider = candidate.provider.clone();
        let kind = capture_resource_kind(&candidate.kind);
        let provider_resource_type = candidate
            .provider_resource_type
            .as_deref()
            .map(super::domain::ResourceTypeKey::parse)
            .transpose()?;
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
            provider_resource_type,
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

fn capture_resource_kind(kind: &CaptureResourceKind) -> ResourceKind {
    match kind {
        CaptureResourceKind::ProjectContentFile => ResourceKind::ProjectContentFile,
        CaptureResourceKind::ProjectContentDirectory => ResourceKind::ProjectContentDirectory,
        CaptureResourceKind::Conversation => ResourceKind::Conversation,
        CaptureResourceKind::ProviderState => ResourceKind::ProviderState,
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
        ResourceKind::Conversation | ResourceKind::ProviderState => "conversations",
        ResourceKind::ProjectContentFile | ResourceKind::ProjectContentDirectory => "project_files",
        ResourceKind::Requirement => "tools",
    }
}

type StorageEngine = BundleEngine<ConfiguredObjectStore>;

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
                revision: bundle.revision,
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
        .provider_scopes()
        .map(|(provider, _)| provider.clone())
        .collect::<Vec<_>>();
    let resources = manifest
        .all_resources()
        .map(|(_, descriptor)| descriptor.clone())
        .collect::<Vec<_>>();
    let mut recipe = BundleRecipe::default();
    for scope in manifest.scopes.values() {
        for (resource_id, entry) in &scope.recipe.entries {
            recipe.entries.insert(resource_id.clone(), entry.clone());
        }
    }
    Ok(BundleSnapshotSummary {
        storage_id,
        bundle_id: manifest.bundle.bundle_id,
        display_name: manifest.bundle.display_name,
        kind: manifest.bundle.kind,
        revision: manifest.revision,
        updated_at: manifest.updated_at,
        resource_count: resources.len(),
        providers,
        repository_fingerprint: manifest.bundle.repository_fingerprint,
        resources,
        recipe,
        fetched_at,
        warnings: Vec::new(),
    })
}

/// The provider-native session id inside a `<provider>:session:<component>`
/// capture resource id.
fn provider_session_id_from_resource(resource_id: &str) -> Option<&str> {
    let (_, rest) = resource_id.split_once(':')?;
    rest.strip_prefix("session:")
        .filter(|session_id| !session_id.is_empty())
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

fn relation_sync_state(relation: TailClassification) -> ThreadSyncState {
    match relation {
        TailClassification::Sync(_) | TailClassification::MissingBoth => ThreadSyncState::Synced,
        TailClassification::LocalOutstanding => ThreadSyncState::LocalAhead,
        TailClassification::MissingCloud => ThreadSyncState::LocalOnly,
        TailClassification::CloudOutstanding => ThreadSyncState::StorageAhead,
        TailClassification::MissingLocal => ThreadSyncState::StorageOnly,
        TailClassification::Divergent => ThreadSyncState::Diverged,
        TailClassification::Unknown => ThreadSyncState::Unknown,
    }
}

struct ThreadReview {
    entries: Vec<ThreadSyncEntry>,
    push_actions: Vec<ConversationPlanAction>,
    pull_actions: Vec<ConversationPlanAction>,
    pull_review: Vec<ConversationReviewRow>,
    /// Provider aliases displaced by Codex copy-on-write materialization and
    /// proven byte-identical to their immutable retained-source checkpoint.
    /// Their files remain available for physical `history_base` dependencies;
    /// changed or unproven retained aliases are independent logical rows and
    /// therefore never enter this suppression set.
    suppressed_provider_ids: BTreeSet<(ProviderKey, String)>,
    /// Per-provider review failures. A failed provider's rows fail closed
    /// without blocking any other provider (D8).
    provider_errors: BTreeMap<ProviderKey, String>,
}

struct EmptyBundleBytes;

impl BundleBytes for EmptyBundleBytes {
    fn body(&mut self, logical_path: &LogicalPath) -> Result<VerifiedBody, String> {
        Err(format!(
            "fresh bundle has no stored payload for '{}'",
            logical_path
        ))
    }

    fn read_bounded(
        &mut self,
        logical_path: &LogicalPath,
        _max_bytes: u64,
        _label: &str,
    ) -> Result<Vec<u8>, String> {
        Err(format!(
            "fresh bundle has no stored payload for '{}'",
            logical_path
        ))
    }
}

#[derive(Clone)]
pub(crate) struct LocalThreadReview {
    pub(crate) binding: Option<LocalConversationBinding>,
    pub(crate) binding_resolution: BindingResolution,
    pub(crate) resource_id: ResourceId,
    pub(crate) descriptor: Option<ResourceDescriptor>,
    pub(crate) provider_thread_id: String,
    pub(crate) source_path: PathBuf,
    pub(crate) source_mtime: u64,
    pub(crate) captured: Option<CapturedThread>,
}

/// Captured transcript bytes attached to a reviewed local thread when a
/// capture selected it: always for providers without a cursor codec, and
/// only for byte-pinning work on providers with one.
#[derive(Clone)]
pub(crate) struct CapturedThread {
    pub(crate) conversation: provider_capture::CapturedConversation,
    pub(crate) transcript: provider_capture::CapturedFile,
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

/// The machine-local transcript source pinned by capture. Every provider
/// records it during discovery, so reviewed plans never re-derive machine
/// paths from portable ones.
fn captured_transcript_source(
    conversation: &provider_capture::CapturedConversation,
) -> Result<PathBuf, String> {
    conversation
        .source_physical_path
        .clone()
        .ok_or_else(|| "captured conversation has no machine-local transcript source".to_string())
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

/// Review every conversation of every bound provider, isolating provider
/// failures: an unreadable profile fails that provider's rows closed and
/// never blocks another provider's safe review (D8).
#[allow(clippy::too_many_arguments)]
fn thread_review(
    repository: &LocalStateRepository,
    snapshot: Option<&BundleSnapshot>,
    bytes: &mut dyn BundleBytes,
    views: &[provider_capture::LocalSessionView],
    discovery: Option<&provider_capture::Discovery>,
    captured: &provider_capture::CapturedResources,
    candidate_pins: &BTreeMap<String, TargetExpectation>,
    resolved: &ResolvedBinding,
    storage_id: Option<&StorageId>,
    created_at: u64,
    expires_at: u64,
    purpose: ReviewPurpose,
) -> Result<ThreadReview, String> {
    let mut merged = ThreadReview {
        entries: Vec::new(),
        push_actions: Vec::new(),
        pull_actions: Vec::new(),
        pull_review: Vec::new(),
        suppressed_provider_ids: BTreeSet::new(),
        provider_errors: BTreeMap::new(),
    };
    let registry = provider_registry();
    for provider in resolved.binding.profile_ids.keys() {
        if registry.sessions(provider).is_none() {
            merged.provider_errors.insert(
                provider.clone(),
                format!("no provider adapter is registered for '{}'", provider),
            );
            continue;
        }
        let review = match resolved.homes.get(provider) {
            Some(home) => provider_thread_review(
                repository,
                snapshot,
                bytes,
                views,
                discovery,
                captured,
                candidate_pins,
                resolved,
                provider,
                Path::new(home),
                storage_id,
                created_at,
                expires_at,
                purpose,
            ),
            None => Err(format!(
                "provider '{}' profile is unavailable on this machine",
                provider
            )),
        };
        match review {
            Ok(review) => {
                merged.entries.extend(review.entries);
                merged.push_actions.extend(review.push_actions);
                merged.pull_actions.extend(review.pull_actions);
                merged.pull_review.extend(review.pull_review);
                merged
                    .suppressed_provider_ids
                    .extend(review.suppressed_provider_ids);
            }
            Err(error) => {
                merged.provider_errors.insert(provider.clone(), error);
            }
        }
    }
    merged.entries.sort_by(|left, right| {
        left.cloud_thread_id
            .cmp(&right.cloud_thread_id)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    merged
        .push_actions
        .sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    merged
        .pull_actions
        .sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    merged.pull_review.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    Ok(merged)
}

/// The reviewed provider-scope coordinates from a snapshot, or the absent
/// pin for a repository/scope that does not exist yet.
fn provider_scope_pin(snapshot: Option<&BundleSnapshot>, provider: &ProviderKey) -> (u64, String) {
    snapshot
        .and_then(|snapshot| snapshot.manifest.provider_scope(provider))
        .map(|scope| (scope.revision, scope.content_sha256.clone()))
        .unwrap_or_else(absent_scope_pin)
}

/// Locate one Codex rollout by thread id under the profile's canonical
/// session roots. Codex embeds the thread UUID in the rollout file name, so
/// the walk matches on the suffix without opening unrelated files.
fn codex_rollout_lookup(
    provider_home: &Path,
) -> impl Fn(&str) -> Result<Option<PathBuf>, String> + '_ {
    move |thread_id: &str| {
        let suffix = format!("-{thread_id}.jsonl");
        for root in ["sessions", "archived_sessions"] {
            let root = provider_home.join(root);
            if !root.is_dir() {
                continue;
            }
            let mut stack = vec![root];
            let mut visited = 0_usize;
            while let Some(directory) = stack.pop() {
                visited += 1;
                if visited > 4_096 {
                    return Err("Codex session walk exceeded its directory bound".to_string());
                }
                let entries = match fs::read_dir(&directory) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let file_type = match entry.file_type() {
                        Ok(file_type) => file_type,
                        Err(_) => continue,
                    };
                    if file_type.is_dir() {
                        stack.push(path);
                    } else if file_type.is_file()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.ends_with(&suffix))
                    {
                        return Ok(Some(path));
                    }
                }
            }
        }
        Ok(None)
    }
}

/// Historical provider aliases remain in applied receipts even after a
/// binding moves to a fresh alias. The receipt ledger is authoritative for
/// imported logical IDs as well as deterministic first-contact IDs, and it
/// naturally supports repeated moves without adding schema state.
fn applied_provider_alias_owners(
    repository: &LocalStateRepository,
    binding: &ProjectBinding,
    provider: &ProviderKey,
    profile_id: &LocalProviderProfileId,
) -> Result<BTreeMap<String, BTreeSet<ThreadId>>, String> {
    let mut owners = BTreeMap::<String, BTreeSet<ThreadId>>::new();
    for record in repository.load_materializations()?.records {
        if record.host_id != binding.host_id
            || record.replica_id != binding.replica_id
            || record.local_project_id != binding.local_project_id
            || record.bundle_id != binding.bundle_id
        {
            continue;
        }
        for receipt in record
            .receipts
            .into_iter()
            .filter(|receipt| receipt.status == ActionStatus::Applied)
        {
            let Some(conversation) = receipt.conversation else {
                continue;
            };
            if conversation.host_id != binding.host_id
                || conversation.replica_id != binding.replica_id
                || conversation.provider_profile_id != *profile_id
                || conversation.materialization_state != ConversationMaterializationState::Resumable
            {
                continue;
            }
            let Ok((receipt_provider, receipt_thread_id)) =
                parse_conversation_resource_id(receipt.resource_id.as_str())
            else {
                continue;
            };
            if &receipt_provider != provider || receipt_thread_id != conversation.thread_id {
                continue;
            }
            if let Some(provider_thread_id) = conversation.provider_thread_id {
                owners
                    .entry(provider_thread_id)
                    .or_default()
                    .insert(conversation.thread_id);
            }
        }
    }
    Ok(owners)
}

fn displaced_provider_alias_owners(
    bundle_id: &BundleId,
    provider: &ProviderKey,
    provider_thread_id: &str,
    scoped_bindings: &[LocalConversationBinding],
    historical_alias_owners: &BTreeMap<String, BTreeSet<ThreadId>>,
) -> Result<BTreeSet<ThreadId>, String> {
    let mut owners = historical_alias_owners
        .get(provider_thread_id)
        .cloned()
        .unwrap_or_default();
    let initial_owner = initial_conversation_thread_id(bundle_id, provider, provider_thread_id)?;
    owners.insert(initial_owner);
    owners.retain(|owner| {
        scoped_bindings.iter().any(|binding| {
            binding.thread_id == *owner
                && binding
                    .provider_thread_id
                    .as_deref()
                    .is_some_and(|current| current != provider_thread_id)
        })
    });
    Ok(owners)
}

/// Synthesized `WholeThread`-equivalent evidence for providers that have not
/// adopted the portable tail codec: field-wise equality over
/// `(codec, whole-thread, 0, count, semantic digest)` reproduces their
/// existing digest-pair comparison semantics.
fn synthesized_whole_thread_tail(
    codec: &str,
    message_count: u64,
    semantic_digest: &str,
) -> PortableConversationTail {
    PortableConversationTail {
        codec: codec.to_string(),
        partition_kind: TailPartitionKind::WholeThread,
        start_record_count: 0,
        accepted_end_record_count: message_count,
        accepted_tail_state: semantic_digest.to_string(),
    }
}

/// A worker's complete observation outcome.
pub(crate) struct CodexLocalObservation {
    observation: LocalTailObservation,
    seeded: Option<CodexSeededRoot>,
    /// Physical files the derivation read; empty when the outcome must not
    /// be retained by the watch service.
    depends_on: Vec<PathBuf>,
}

/// A parallel observation worker's slot value.
type ObservationOutcome = Result<CodexLocalObservation, String>;

fn take_observation_outcome(
    thread_id: &ThreadId,
    slot: &mut std::sync::OnceLock<ObservationOutcome>,
) -> ObservationOutcome {
    slot.take().unwrap_or_else(|| {
        Err(format!(
            "thread observation worker produced no outcome for thread '{}'",
            thread_id
        ))
    })
}

/// One reviewed thread's derived local state.
#[derive(Clone)]
pub(crate) struct LocalTailObservation {
    evidence: TailSideEvidence,
    /// The checkpoint-format raw range pin for a publishing plan.
    raw_range_pin: Option<RawRangePin>,
    /// Acceptance evidence re-established by first-contact adoption: the
    /// reviewed cloud wire pin plus its local-form twin at the same end,
    /// proven by encoding this local chain under the binding root and
    /// folding the wire state at the cloud pin's recorded end. Set only on
    /// the full-resolve path; it never comes from a cursor ride or any
    /// non-verifying branch.
    verified_acceptance: Option<VerifiedAdoption>,
}

/// The §6.4 encode-compare adoption outcome: dual pins for this review only;
/// the durable pins are written by the Push or Pull the adoption enables.
#[derive(Clone)]
pub(crate) struct VerifiedAdoption {
    wire_tail: PortableConversationTail,
    local_tail: PortableConversationTail,
}

/// The §6.4 encode-compare: build the wire form of the resolved local thread
/// under its own binding root, fold the wire state at the cloud pin's end,
/// and compare. Equal ⇒ dual pins re-established (cloud wire pin + the
/// local-form twin at the same end). Every failure path returns `None` —
/// unproven, never an error — matching the old probe's fail-closed posture.
fn adopt_codex_acceptance(
    resolved: &codex_history_graph::ResolvedCodexRoot,
    probe: &PortableConversationTail,
    effective_kind: TailPartitionKind,
    effective_start: u64,
    layout: Option<&RootTailLayout>,
    project_root: &str,
    canonical_project_root: &str,
) -> Option<VerifiedAdoption> {
    if probe.codec != mallard_domain::PORTABLE_TAIL_CODEC_WIRE_V1
        || probe.partition_kind != effective_kind
        || probe.start_record_count != effective_start
        || probe.accepted_end_record_count < effective_start
        || probe.accepted_end_record_count > resolved.chain.len() as u64
    {
        return None;
    }
    let artifact = build_standalone_codex_root(
        resolved,
        &StandaloneBuildMode::Encode {
            root: project_root.to_string(),
            canonical_root: (canonical_project_root != project_root)
                .then(|| canonical_project_root.to_string()),
        },
        layout,
    )
    .ok()?;
    let wire_state_at_probe_end =
        if probe.accepted_end_record_count == artifact.portable_tail.accepted_end_record_count {
            artifact.portable_tail.accepted_tail_state.clone()
        } else {
            // The cloud pin is a strict prefix: fold the encoded wire chain
            // at exactly its recorded end.
            let analysis = analyze_codex_transcript(&artifact.root_bytes);
            let wire_chain = analysis.lineage.chain()?;
            mallard_domain::compute_conversation_tail_state(
                effective_kind,
                effective_start,
                probe.accepted_end_record_count,
                wire_chain,
            )
            .ok()?
        };
    if wire_state_at_probe_end != probe.accepted_tail_state {
        return None;
    }
    let local_state = mallard_domain::compute_conversation_tail_state(
        effective_kind,
        effective_start,
        probe.accepted_end_record_count,
        &resolved.chain,
    )
    .ok()?;
    let local_tail = PortableConversationTail {
        codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.to_string(),
        partition_kind: effective_kind,
        start_record_count: effective_start,
        accepted_end_record_count: probe.accepted_end_record_count,
        accepted_tail_state: local_state,
    };
    local_tail.validate().ok()?;
    Some(VerifiedAdoption {
        wire_tail: probe.clone(),
        local_tail,
    })
}

/// Derive local Codex evidence: ride the checkpoint cursor when it is valid,
/// fall back to a full re-resolve, and seed first-contact state. Returns the
/// observation plus the refreshed checkpoint value to persist.
///
/// `adoption_probe` carries the reviewed cloud wire tail when the thread has
/// no accepted pin. It forces the full-resolve path so the local thread can
/// be **encoded** under its own binding root and the wire fold compared to
/// the cloud pin at its recorded end: equality proves the cloud version is a
/// prefix of local and re-establishes the dual acceptance pins
/// (first-contact adoption, e.g. after a removed project is re-registered);
/// anything else leaves the pair unproven and blocked. Encoding is a pure
/// function of (bytes, binding root) — there is no table to infer.
#[allow(clippy::too_many_arguments)]
fn observe_codex_local(
    local: Option<&LocalThreadReview>,
    checkpoint: Option<&ConversationCheckpoint>,
    layout: Option<&RootTailLayout>,
    provider_home: &Path,
    adoption_probe: Option<&PortableConversationTail>,
    project_root: &str,
    canonical_project_root: &str,
) -> Result<CodexLocalObservation, String> {
    let Some(local) = local else {
        return Ok(CodexLocalObservation {
            observation: LocalTailObservation {
                evidence: TailSideEvidence::ConfirmedAbsent,
                raw_range_pin: None,
                verified_acceptance: None,
            },
            seeded: None,
            depends_on: Vec::new(),
        });
    };
    let source = local.source_path.clone();
    let lookup = codex_rollout_lookup(provider_home);

    // A valid Ready checkpoint under a matching layout rides the cursor;
    // an adoption probe needs the canonical chain and takes the full
    // resolve below instead.
    if let (Some(checkpoint), Some(layout), None) = (checkpoint, layout, adoption_probe) {
        if let PhysicalMappingState::Ready {
            root_mapping,
            raw_range_pin,
        } = &checkpoint.physical
        {
            if layout.matches(&checkpoint.observed_tail)
                && root_mapping.root_path == source.to_string_lossy()
            {
                let tail =
                    match codex_history_graph::read_rollout_tail_window(&source, root_mapping) {
                        Ok(tail) => tail,
                        Err(_) => {
                            return Ok(CodexLocalObservation {
                                observation: LocalTailObservation {
                                    evidence: TailSideEvidence::Invalid,
                                    raw_range_pin: None,
                                    verified_acceptance: None,
                                },
                                seeded: None,
                                depends_on: Vec::new(),
                            })
                        }
                    };
                match codex_history_graph::refresh_codex_tail(
                    layout,
                    &checkpoint.observed_tail,
                    root_mapping,
                    &tail,
                )? {
                    codex_history_graph::CodexTailRefresh::Unchanged => {
                        return Ok(CodexLocalObservation {
                            observation: LocalTailObservation {
                                evidence: TailSideEvidence::Present(
                                    checkpoint.observed_tail.clone(),
                                ),
                                raw_range_pin: Some(raw_range_pin.clone()),
                                verified_acceptance: None,
                            },
                            seeded: None,
                            depends_on: vec![source.clone()],
                        });
                    }
                    codex_history_graph::CodexTailRefresh::Extended {
                        observed_tail,
                        mapping,
                    } => {
                        let range_start = raw_range_pin
                            .start_byte_offset
                            .min(mapping.consumed_byte_offset);
                        // Re-prove the accepted-to-current interval. Bytes at
                        // or past the tail window are already trusted from
                        // the fold; an earlier range start needs one bounded
                        // range read whose overlap with the window must match
                        // byte-for-byte, else the file raced and the
                        // observation fails closed.
                        let range_bytes = if range_start >= tail.window_offset {
                            tail.bytes[(range_start - tail.window_offset) as usize
                                ..(mapping.consumed_byte_offset - tail.window_offset) as usize]
                                .to_vec()
                        } else {
                            let range = match codex_history_graph::read_rollout_range(
                                &source,
                                range_start,
                                mapping.consumed_byte_offset,
                            ) {
                                Ok(range) => range,
                                Err(_) => {
                                    return Ok(CodexLocalObservation {
                                        observation: LocalTailObservation {
                                            evidence: TailSideEvidence::Invalid,
                                            raw_range_pin: None,
                                            verified_acceptance: None,
                                        },
                                        seeded: None,
                                        depends_on: Vec::new(),
                                    })
                                }
                            };
                            let overlap_from = (tail.window_offset - range_start) as usize;
                            let trusted = &tail.bytes
                                [..(mapping.consumed_byte_offset - tail.window_offset) as usize];
                            if &range[overlap_from..] != trusted {
                                return Ok(CodexLocalObservation {
                                    observation: LocalTailObservation {
                                        evidence: TailSideEvidence::Invalid,
                                        raw_range_pin: None,
                                        verified_acceptance: None,
                                    },
                                    seeded: None,
                                    depends_on: Vec::new(),
                                });
                            }
                            range
                        };
                        super::sync_metrics::record_bytes_hashed(&source, range_bytes.len() as u64);
                        let pin = RawRangePin {
                            start_byte_offset: range_start,
                            end_byte_offset: mapping.consumed_byte_offset,
                            range_sha256: hex_digest(Sha256::digest(&range_bytes).as_slice()),
                        };
                        return Ok(CodexLocalObservation {
                            observation: LocalTailObservation {
                                evidence: TailSideEvidence::Present(observed_tail.clone()),
                                raw_range_pin: Some(pin.clone()),
                                verified_acceptance: None,
                            },
                            seeded: Some(CodexSeededRoot {
                                layout: layout.clone(),
                                observed_tail,
                                prefix_witness: checkpoint.prefix_witness.clone(),
                                mapping,
                                lineage_digest: String::new(),
                                semantic_digest: String::new(),
                            }),
                            depends_on: vec![source.clone()],
                        });
                    }
                    codex_history_graph::CodexTailRefresh::NeedsFullResolve => {}
                    codex_history_graph::CodexTailRefresh::Invalid { .. } => {
                        return Ok(CodexLocalObservation {
                            observation: LocalTailObservation {
                                evidence: TailSideEvidence::Invalid,
                                raw_range_pin: None,
                                verified_acceptance: None,
                            },
                            seeded: None,
                            depends_on: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    // First contact or unusable cursor: fully resolve and seed.
    let resolved = match codex_history_graph::resolve_codex_root(&source, &lookup) {
        Ok(resolved) => resolved,
        Err(_) => {
            return Ok(CodexLocalObservation {
                observation: LocalTailObservation {
                    evidence: TailSideEvidence::Invalid,
                    raw_range_pin: None,
                    verified_acceptance: None,
                },
                seeded: None,
                depends_on: Vec::new(),
            })
        }
    };
    // The stored layout stays authoritative — including a paginated layout
    // imported at Pull install over a null-base file; a derivation that
    // cannot agree with it is corruption, never a reseed.
    let Ok((effective_kind, effective_start)) =
        codex_history_graph::effective_tail_partition(layout, &resolved)
    else {
        return Ok(CodexLocalObservation {
            observation: LocalTailObservation {
                evidence: TailSideEvidence::Invalid,
                raw_range_pin: None,
                verified_acceptance: None,
            },
            seeded: None,
            depends_on: Vec::new(),
        });
    };
    // First-contact adoption (§6.4): encode the local thread under its own
    // binding root and compare the wire fold at the cloud pin's recorded
    // end. Equality proves the cloud version is a prefix of local and
    // re-establishes the dual pins; any failure — including a root the
    // encoder refuses (non-POSIX, reserved-token collision) — leaves the
    // pair unproven and blocked.
    let verified_acceptance = adoption_probe.and_then(|probe| {
        adopt_codex_acceptance(
            &resolved,
            probe,
            effective_kind,
            effective_start,
            layout,
            project_root,
            canonical_project_root,
        )
    });
    let leaf_bytes = codex_history_graph::read_rollout_bounded(&source)?;
    let seeded = codex_history_graph::seed_codex_root(&resolved, &leaf_bytes, layout)?;
    if checkpoint
        .and_then(|checkpoint| checkpoint.prefix_witness.as_ref())
        .is_some_and(|stored| seeded.prefix_witness.as_ref() != Some(stored))
    {
        return Ok(CodexLocalObservation {
            observation: LocalTailObservation {
                evidence: TailSideEvidence::Invalid,
                raw_range_pin: None,
                verified_acceptance: None,
            },
            seeded: None,
            depends_on: Vec::new(),
        });
    }
    let accepted_start = checkpoint
        .and_then(|checkpoint| match &checkpoint.physical {
            PhysicalMappingState::Ready { raw_range_pin, .. } => {
                Some(raw_range_pin.start_byte_offset)
            }
            PhysicalMappingState::Detached { .. } => None,
        })
        .unwrap_or(0)
        .min(seeded.mapping.consumed_byte_offset);
    let pin = RawRangePin {
        start_byte_offset: accepted_start,
        end_byte_offset: seeded.mapping.consumed_byte_offset,
        range_sha256: hex_digest(
            Sha256::digest(
                &leaf_bytes[accepted_start as usize..seeded.mapping.consumed_byte_offset as usize],
            )
            .as_slice(),
        ),
    };
    Ok(CodexLocalObservation {
        observation: LocalTailObservation {
            evidence: TailSideEvidence::Present(seeded.observed_tail.clone()),
            raw_range_pin: Some(pin),
            verified_acceptance,
        },
        seeded: Some(seeded),
        depends_on: resolved
            .segments
            .iter()
            .map(|segment| segment.path.clone())
            .collect(),
    })
}

#[allow(clippy::too_many_arguments)]
fn provider_thread_review(
    repository: &LocalStateRepository,
    snapshot: Option<&BundleSnapshot>,
    bytes: &mut dyn BundleBytes,
    views: &[provider_capture::LocalSessionView],
    discovery: Option<&provider_capture::Discovery>,
    captured: &provider_capture::CapturedResources,
    candidate_pins: &BTreeMap<String, TargetExpectation>,
    resolved: &ResolvedBinding,
    provider: &ProviderKey,
    provider_home: &Path,
    storage_id: Option<&StorageId>,
    created_at: u64,
    expires_at: u64,
    purpose: ReviewPurpose,
) -> Result<ThreadReview, String> {
    let binding = &resolved.binding;
    let adapter = provider_registry().require_sessions(provider)?;
    let provider_label = provider_registry()
        .get(provider)
        .map(|registered| registered.descriptor().display_name)
        .unwrap_or_else(|| provider.to_string());
    let profile_id = binding.profile_ids.get(provider).cloned().ok_or_else(|| {
        format!(
            "{} conversations require a mapped {} profile",
            provider_label, provider_label
        )
    })?;
    let is_codex = provider.as_str() == "codex";
    let codex_inventory = is_codex
        .then(|| discovery.and_then(|discovery| discovery.provider_session_inventory(provider)))
        .flatten();
    let historical_alias_owners = if is_codex {
        applied_provider_alias_owners(repository, binding, provider, &profile_id)?
    } else {
        BTreeMap::new()
    };
    let identity_config = repository.load_config()?;
    let (pinned_scope_revision, pinned_scope_sha256) = provider_scope_pin(snapshot, provider);
    let domain_resources = provider_capture::domain_resources(captured)?;
    let provider_scope = snapshot.and_then(|snapshot| snapshot.manifest.provider_scope(provider));

    // Threads owned by a journaled sync operation are review-blocked in both
    // directions until that operation reaches a terminal phase.
    let pending_resolution_threads = repository
        .load_conversation_sync_journal()?
        .operations
        .iter()
        .filter(|operation| {
            operation.local_project_id == binding.local_project_id
                && &operation.provider == provider
        })
        .map(|operation| operation.thread_id.clone())
        .collect::<BTreeSet<_>>();

    let build_local_threads = |document: &mallard_domain::ConversationBindings| -> Result<
        (BTreeMap<ThreadId, LocalThreadReview>, BTreeSet<String>),
        String,
    > {
        let scoped = document
            .bindings
            .iter()
            .filter(|conversation| {
                conversation.host_id == binding.host_id
                    && conversation.replica_id == binding.replica_id
                    && conversation.local_project_id == binding.local_project_id
                    && conversation.provider_profile_id == profile_id
            })
            .cloned()
            .collect::<Vec<_>>();
        let by_provider = scoped
            .iter()
            .filter_map(|conversation| {
                conversation
                    .provider_thread_id
                    .as_ref()
                    .map(|provider_id| (provider_id.clone(), conversation.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut local_threads = BTreeMap::<ThreadId, LocalThreadReview>::new();
        let mut suppressed_provider_ids = BTreeSet::new();
        // Every local row starts from bounded discovery metadata; captured
        // bytes attach when a capture selected the session. Identity comes
        // from the captured analysis when bytes are present (the stricter
        // reading) and from the bounded head scan otherwise.
        for view in views {
            if &view.provider != provider || view.descriptor.blocked_reason.is_some() {
                continue;
            }
            let resource_id_text = view.descriptor.resource_id.as_str();
            let captured_thread = captured
                .conversations
                .get(resource_id_text)
                .map(|conversation| -> Result<CapturedThread, String> {
                    let transcript = captured
                        .files
                        .get(&conversation.transcript_logical_path)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "captured conversation '{}' has no transcript payload",
                                resource_id_text
                            )
                        })?;
                    Ok(CapturedThread {
                        conversation: conversation.clone(),
                        transcript,
                    })
                })
                .transpose()?;
            let provider_thread_id = match &captured_thread {
                Some(captured_thread) => captured_thread
                    .conversation
                    .source_thread_id
                    .clone()
                    .ok_or_else(|| {
                        format!(
                            "captured {} conversation lacks an active session id",
                            provider_label
                        )
                    })?,
                None => view.provider_thread_id.clone(),
            };
            let local_binding = by_provider.get(&provider_thread_id).cloned();
            let (logical_thread_id, binding_resolution) = match &local_binding {
                Some(local_binding) => (local_binding.thread_id.clone(), BindingResolution::Bound),
                None => {
                    // Copy-on-write deliberately leaves the old provider
                    // rollout in place. It receives a separate, stable local
                    // branch identity: exact checkpoint evidence below hides
                    // it only while its bytes remain unchanged. If Codex later
                    // appends to the retained entry, that same identity becomes
                    // ordinary reviewable local work instead of colliding with
                    // the logical thread that moved to a fresh alias.
                    let displaced_owners = displaced_provider_alias_owners(
                        &binding.bundle_id,
                        provider,
                        &provider_thread_id,
                        &scoped,
                        &historical_alias_owners,
                    )?;
                    if !displaced_owners.is_empty() {
                        let (retained_thread_id, unchanged) =
                            project_activity::retained_provider_alias_identity(
                                &binding.bundle_id,
                                &binding.local_project_id,
                                provider,
                                &provider_thread_id,
                                &view.source_path,
                                &displaced_owners,
                                &scoped
                                    .iter()
                                    .map(|binding| binding.thread_id.clone())
                                    .collect(),
                                &identity_config,
                            )?;
                        if unchanged {
                            suppressed_provider_ids.insert(provider_thread_id);
                            continue;
                        }
                        (retained_thread_id, BindingResolution::Missing)
                    } else {
                        // Binding recovery by canonical containment is
                        // deliberately gone: any other unbound local
                        // conversation publishes as a fresh thread.
                        (
                            initial_conversation_thread_id(
                                &binding.bundle_id,
                                provider,
                                &provider_thread_id,
                            )?,
                            BindingResolution::Missing,
                        )
                    }
                }
            };
            if local_threads.contains_key(&logical_thread_id) {
                return Err(format!(
                    "multiple local {} conversations resolve to logical thread '{}'",
                    provider_label, logical_thread_id
                ));
            }
            let resource_id = ResourceId::parse(resource_id_text.to_string())?;
            let descriptor = match domain_resources.get(&resource_id).cloned() {
                Some(descriptor) => Some(descriptor),
                None => Some(provider_capture::domain_resource_descriptor(
                    &view.descriptor,
                    None,
                )?),
            };
            local_threads.insert(
                logical_thread_id.clone(),
                LocalThreadReview {
                    binding: local_binding,
                    binding_resolution,
                    resource_id,
                    descriptor,
                    provider_thread_id,
                    source_path: view.source_path.clone(),
                    source_mtime: view.source_mtime,
                    captured: captured_thread,
                },
            );
        }
        Ok((local_threads, suppressed_provider_ids))
    };

    // Phase A: first-contact seeding. Bound Codex tasks without a layout are
    // fully parsed once; the immutable layout lands on the binding and the
    // first checkpoint lands on the destination link. Observation-class
    // work: nothing here builds artifacts or touches cloud state.
    if is_codex {
        let document = repository.load_conversation_bindings()?;
        let (local_threads, _) = build_local_threads(&document)?;
        for (thread_id, local) in &local_threads {
            let Some(local_binding) = &local.binding else {
                continue;
            };
            if local_binding.root_tail_layout.is_some() {
                continue;
            }
            let source = local.source_path.clone();
            let lookup = codex_rollout_lookup(provider_home);
            let Ok(resolved_root) = codex_history_graph::resolve_codex_root(&source, &lookup)
            else {
                continue;
            };
            let Ok(leaf_bytes) = codex_history_graph::read_rollout_bounded(&source) else {
                continue;
            };
            let Ok(seeded) =
                codex_history_graph::seed_codex_root(&resolved_root, &leaf_bytes, None)
            else {
                continue;
            };
            let target_thread = local_binding.thread_id.clone();
            let seeded_layout = seeded.layout.clone();
            let binding_revision = repository.mutate_conversation_bindings(|document| {
                let row = document
                    .bindings
                    .iter_mut()
                    .find(|row| {
                        row.host_id == binding.host_id
                            && row.replica_id == binding.replica_id
                            && row.local_project_id == binding.local_project_id
                            && row.provider_profile_id == profile_id
                            && row.thread_id == target_thread
                    })
                    .ok_or_else(|| "seeded conversation binding disappeared".to_string())?;
                if row.root_tail_layout.is_none() {
                    row.root_tail_layout = Some(seeded_layout.clone());
                    row.revision = row.revision.saturating_add(1);
                    row.updated_at = created_at.max(row.updated_at);
                }
                Ok(row.revision)
            })?;
            if let Some(storage_id) = storage_id {
                let consumed = seeded.mapping.consumed_byte_offset;
                let checkpoint = ConversationCheckpoint {
                    generation: 0,
                    accepted: None,
                    accepted_local_tail: None,
                    observed_tail: seeded.observed_tail.clone(),
                    physical: PhysicalMappingState::Ready {
                        root_mapping: seeded.mapping.clone(),
                        raw_range_pin: RawRangePin {
                            start_byte_offset: 0,
                            end_byte_offset: consumed,
                            range_sha256: hex_digest(
                                Sha256::digest(&leaf_bytes[..consumed as usize]).as_slice(),
                            ),
                        },
                    },
                    prefix_witness: seeded.prefix_witness.clone(),
                    binding_revision,
                };
                let scope = SyncScopeKey::Provider(provider.clone());
                let thread_id = thread_id.clone();
                repository.mutate_config(|config| {
                    let Some(link) = config.links.iter_mut().find(|link| {
                        link.local_project_id == binding.local_project_id
                            && &link.storage_id == storage_id
                    }) else {
                        return Ok(());
                    };
                    link.conversation_checkpoints
                        .entry(scope.clone())
                        .or_default()
                        .entry(thread_id.clone())
                        .or_insert(checkpoint.clone());
                    Ok(())
                })?;
            }
        }
    }

    // Phase B: pure reads. Every revision pinned below is the post-seeding
    // value, so plan preconditions stay coherent with what review persisted.
    let machine = repository.load_bindings()?;
    let profile = machine
        .profiles
        .iter()
        .find(|profile| profile.profile_id == profile_id)
        .ok_or_else(|| format!("mapped {} profile disappeared", provider_label))?;
    let document = repository.load_conversation_bindings()?;
    let host_identity = repository.load_or_create_host_identity()?;
    let config = repository.load_config()?;
    let checkpoints: BTreeMap<ThreadId, ConversationCheckpoint> = storage_id
        .and_then(|storage_id| {
            config
                .links
                .iter()
                .find(|link| {
                    link.local_project_id == binding.local_project_id
                        && &link.storage_id == storage_id
                })
                .and_then(|link| {
                    link.conversation_checkpoints
                        .get(&SyncScopeKey::Provider(provider.clone()))
                        .cloned()
                })
        })
        .unwrap_or_default();
    let (local_threads, suppressed_provider_ids) = build_local_threads(&document)?;
    let bindings_by_thread = document
        .bindings
        .iter()
        .filter(|conversation| {
            conversation.host_id == binding.host_id
                && conversation.replica_id == binding.replica_id
                && conversation.local_project_id == binding.local_project_id
                && conversation.provider_profile_id == profile_id
        })
        .map(|conversation| (conversation.thread_id.clone(), conversation.clone()))
        .collect::<BTreeMap<_, _>>();

    let thread_ids = provider_scope
        .into_iter()
        .flat_map(|scope| scope.conversations.iter())
        .map(|(thread_id, _)| thread_id.clone())
        .chain(bindings_by_thread.keys().cloned())
        .chain(local_threads.keys().cloned())
        .collect::<BTreeSet<_>>();

    // The only payload reads planning performs: Pull opens the transcripts
    // of threads it could offer to materialize locally in one batched
    // prefetch, to refuse stored internal-subagent rollouts before review.
    let mut offerable_cloud_transcripts = BTreeMap::new();
    if purpose.builds_pull_plan() {
        for (thread_id, thread) in provider_scope
            .into_iter()
            .flat_map(|scope| scope.conversations.iter())
            .filter(|(thread_id, _)| !local_threads.contains_key(*thread_id))
        {
            let Some(entry) = thread
                .files
                .iter()
                .find(|file| file.role == ConversationFileRole::Transcript)
                .and_then(|file| {
                    provider_scope
                        .and_then(|scope| scope.files.get(&file.logical_path))
                        .map(|entry| (file.logical_path.clone(), entry.sha256.clone(), entry.size))
                })
            else {
                continue;
            };
            offerable_cloud_transcripts.insert(thread_id.clone(), entry);
        }
        let prefetch_paths = offerable_cloud_transcripts
            .values()
            .filter(|(_, _, size)| *size <= MAX_CONVERSATION_FILE_BYTES)
            .map(|(logical_path, _, _)| logical_path.clone())
            .collect::<Vec<_>>();
        let _ = bytes.prefetch(&prefetch_paths);
    }
    let index_path =
        (adapter.index_fragment_limit() > 0).then(|| provider_home.join("session_index.jsonl"));
    let index_expectation = match (purpose.builds_pull_plan(), &index_path) {
        (true, Some(path)) => Some(bounded_file_expectation(path)?),
        _ => None,
    };
    let mut occupied_provider_ids = codex_inventory
        .map(|inventory| inventory.provider_session_ids().clone())
        .unwrap_or_else(|| {
            local_threads
                .values()
                .map(|local| local.provider_thread_id.clone())
                .collect()
        });
    let mut entries = Vec::new();
    let mut push_actions = Vec::new();
    let mut pull_actions = Vec::new();
    let mut pull_review = Vec::new();
    let (pinned_revision, pinned_commit_id, pinned_manifest_sha256) = snapshot
        .map(|snapshot| {
            (
                snapshot.head.revision,
                snapshot.head.commit_id.clone(),
                snapshot.head.manifest_sha256.clone(),
            )
        })
        .unwrap_or_else(absent_bundle_head_pin);

    // Codex local observation is the one expensive Phase B stage — full
    // resolves on first contact, adoption probes, and imported layouts. It
    // is pure per-thread filesystem work, so it runs through the bounded
    // worker pool; everything consuming the results below (evidence
    // upgrades, classification, decisions, plan pins) stays serial and
    // looks the observations up by thread ID, so worker count never changes
    // the review's output. Tasks are dispatched in thread-ID order: an
    // error at task k proves every earlier task was already dispatched and
    // completes, so the earliest failing thread's error is reported at
    // every worker count, exactly like the serial loop this replaces.
    let mut codex_observations: BTreeMap<ThreadId, LocalTailObservation> = BTreeMap::new();
    let mut codex_refreshed: BTreeMap<ThreadId, CodexSeededRoot> = BTreeMap::new();
    let mut codex_deps: BTreeMap<ThreadId, Vec<PathBuf>> = BTreeMap::new();
    if is_codex {
        let observation_tasks: Vec<_> = thread_ids
            .iter()
            .filter_map(|thread_id| {
                let local = local_threads.get(thread_id);
                let local_binding = local
                    .and_then(|local| local.binding.as_ref())
                    .or_else(|| bindings_by_thread.get(thread_id));
                let expected_local_missing = local.is_none()
                    && local_binding.is_some_and(|binding| binding.provider_thread_id.is_some());
                if expected_local_missing {
                    return None;
                }
                let checkpoint = checkpoints.get(thread_id);
                let layout = local_binding.and_then(|binding| binding.root_tail_layout.as_ref());
                let adoption_probe = checkpoint
                    .and_then(|checkpoint| checkpoint.accepted.as_ref())
                    .is_none()
                    .then(|| {
                        provider_scope
                            .and_then(|scope| scope.conversations.get(thread_id))
                            .and_then(|thread| thread.portable_tail.as_ref())
                    })
                    .flatten();
                Some((
                    thread_id,
                    local,
                    checkpoint,
                    layout,
                    adoption_probe,
                    local_binding.map(|binding| binding.revision),
                ))
            })
            .collect();
        // Cache-eligible tasks (a local source, no adoption probe) consult
        // the watch service first: a served thread performs no filesystem
        // work at all. Misses and ineligible tasks run the bounded worker
        // pool exactly as before, in thread-ID order.
        let cached: BTreeMap<ThreadId, LocalTailObservation> = match storage_id {
            Some(storage_id) => {
                let queries: Vec<_> = observation_tasks
                    .iter()
                    .filter(|(_, local, _, _, probe, _)| local.is_some() && probe.is_none())
                    .map(|(thread_id, _, checkpoint, _, _, binding_revision)| {
                        super::codex_watch_service::ObservationQuery {
                            thread_id: (**thread_id).clone(),
                            checkpoint_generation: checkpoint
                                .map(|checkpoint| checkpoint.generation),
                            binding_revision: *binding_revision,
                        }
                    })
                    .collect();
                super::codex_watch_service::session_watch_service().take_reusable_observations(
                    provider_home,
                    &binding.local_project_id,
                    storage_id,
                    &queries,
                )
            }
            None => BTreeMap::new(),
        };
        let observation_tasks: Vec<_> = observation_tasks
            .into_iter()
            .filter(|(thread_id, ..)| !cached.contains_key(*thread_id))
            .collect();
        codex_observations.extend(cached);
        let mut slots: Vec<std::sync::OnceLock<ObservationOutcome>> = observation_tasks
            .iter()
            .map(|_| std::sync::OnceLock::new())
            .collect();
        if !observation_tasks.is_empty() {
            let workers = provider_capture::bounded_refresh_workers(observation_tasks.len());
            let cursor = std::sync::atomic::AtomicUsize::new(0);
            let failed = std::sync::atomic::AtomicBool::new(false);
            let tasks = &observation_tasks;
            let task_slots = &slots;
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        if failed.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((_, local, checkpoint, layout, probe, _)) = tasks.get(index)
                        else {
                            break;
                        };
                        let outcome = observe_codex_local(
                            *local,
                            *checkpoint,
                            *layout,
                            provider_home,
                            *probe,
                            &binding.project_root,
                            &binding.canonical_project_root,
                        );
                        if outcome.is_err() {
                            failed.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        let _ = task_slots[index].set(outcome);
                    });
                }
            });
        }
        // Ordered merge preserves the earliest real observation error. An
        // unexpectedly unset slot is a controlled stage error, never a panic.
        for ((thread_id, ..), slot) in observation_tasks.iter().zip(slots.iter_mut()) {
            let CodexLocalObservation {
                observation,
                seeded,
                depends_on,
            } = take_observation_outcome(thread_id, slot)?;
            if let Some(seeded) = seeded {
                codex_refreshed.insert((*thread_id).clone(), seeded);
            }
            if !depends_on.is_empty() {
                codex_deps.insert((*thread_id).clone(), depends_on);
            }
            codex_observations.insert((*thread_id).clone(), observation);
        }
    }

    // Checkpoint maintenance: persist each advanced observation so the next
    // Refresh rides the new cursor instead of re-deriving from the last
    // applied pin. Generation compare-and-swap skips any checkpoint a
    // concurrent operation moved, a Detached mapping is never reattached by
    // observation, byte-identical replacements are skipped, and `accepted`
    // never changes here.
    let mut maintained: BTreeMap<ThreadId, u64> = BTreeMap::new();
    if let Some(storage_id) = storage_id {
        for (thread_id, seeded) in &codex_refreshed {
            let Some(pin) = codex_observations
                .get(thread_id)
                .and_then(|observation| observation.raw_range_pin.clone())
            else {
                continue;
            };
            let Some(local_binding) = local_threads
                .get(thread_id)
                .and_then(|local| local.binding.as_ref())
                .or_else(|| bindings_by_thread.get(thread_id))
            else {
                continue;
            };
            let stored = checkpoints.get(thread_id);
            if matches!(
                stored.map(|checkpoint| &checkpoint.physical),
                Some(PhysicalMappingState::Detached { .. })
            ) {
                continue;
            }
            let replacement = ConversationCheckpoint {
                generation: stored
                    .map(|checkpoint| checkpoint.generation.saturating_add(1))
                    .unwrap_or(0),
                accepted: stored.and_then(|checkpoint| checkpoint.accepted.clone()),
                accepted_local_tail: stored
                    .and_then(|checkpoint| checkpoint.accepted_local_tail.clone()),
                observed_tail: seeded.observed_tail.clone(),
                physical: PhysicalMappingState::Ready {
                    root_mapping: seeded.mapping.clone(),
                    raw_range_pin: pin,
                },
                prefix_witness: seeded.prefix_witness.clone(),
                binding_revision: local_binding.revision,
            };
            if stored.is_some_and(|checkpoint| {
                checkpoint.observed_tail == replacement.observed_tail
                    && checkpoint.physical == replacement.physical
                    && checkpoint.prefix_witness == replacement.prefix_witness
                    && checkpoint.binding_revision == replacement.binding_revision
            }) {
                continue;
            }
            let expected_generation = stored.map(|checkpoint| checkpoint.generation);
            let scope = SyncScopeKey::Provider(provider.clone());
            let thread_id = thread_id.clone();
            let replacement_generation = replacement.generation;
            let persisted = repository.mutate_config(|config| {
                let Some(link) = config.links.iter_mut().find(|link| {
                    link.local_project_id == binding.local_project_id
                        && &link.storage_id == storage_id
                }) else {
                    return Ok(false);
                };
                let threads = link
                    .conversation_checkpoints
                    .entry(scope.clone())
                    .or_default();
                match (threads.get(&thread_id), expected_generation) {
                    (Some(stored), Some(expected)) if stored.generation == expected => {
                        threads.insert(thread_id.clone(), replacement.clone());
                        Ok(true)
                    }
                    (None, None) => {
                        threads.insert(thread_id.clone(), replacement.clone());
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            })?;
            if persisted {
                maintained.insert(thread_id, replacement_generation);
            }
        }
    }

    // Retain freshly derived observations for the next operation, keyed by
    // the post-maintenance checkpoint generation and validated against the
    // watcher's dirty paths. Adoption-probe threads are never retained: a
    // thread without accepted evidence re-proves its refold every review
    // while a cloud tail exists for it.
    if let Some(storage_id) = storage_id {
        if is_codex && !codex_deps.is_empty() {
            let stored: Vec<_> = codex_deps
                .iter()
                .filter_map(|(thread_id, depends_on)| {
                    let observation = codex_observations.get(thread_id)?;
                    if !matches!(observation.evidence, TailSideEvidence::Present(_)) {
                        return None;
                    }
                    let local = local_threads.get(thread_id)?;
                    let checkpoint = checkpoints.get(thread_id);
                    let probe_active = checkpoint
                        .and_then(|checkpoint| checkpoint.accepted.as_ref())
                        .is_none()
                        && provider_scope
                            .and_then(|scope| scope.conversations.get(thread_id))
                            .and_then(|thread| thread.portable_tail.as_ref())
                            .is_some();
                    if probe_active {
                        return None;
                    }
                    let local_binding = local
                        .binding
                        .as_ref()
                        .or_else(|| bindings_by_thread.get(thread_id));
                    let checkpoint_generation = match maintained.get(thread_id) {
                        Some(generation) => Some(*generation),
                        None => checkpoint.map(|checkpoint| checkpoint.generation),
                    };
                    Some(super::codex_watch_service::StoredObservation {
                        thread_id: thread_id.clone(),
                        source_path: local.source_path.clone(),
                        depends_on: depends_on.clone(),
                        checkpoint_generation,
                        binding_revision: local_binding.map(|binding| binding.revision),
                        observation: observation.clone(),
                    })
                })
                .collect();
            if !stored.is_empty() {
                super::codex_watch_service::session_watch_service().store_observations(
                    provider_home,
                    &binding.local_project_id,
                    storage_id,
                    stored,
                );
            }
        }
    }

    for thread_id in thread_ids {
        let thread = provider_scope.and_then(|scope| scope.conversations.get(&thread_id));
        let local = local_threads.get(&thread_id);
        let local_binding = local
            .and_then(|local| local.binding.as_ref())
            .or_else(|| bindings_by_thread.get(&thread_id));
        let checkpoint = checkpoints.get(&thread_id);
        let cloud_descriptor = thread.and_then(|thread| {
            provider_scope.and_then(|scope| scope.resources.get(&thread.resource_id))
        });
        let descriptor = local
            .and_then(|local| local.descriptor.as_ref())
            .or(cloud_descriptor);

        // Accepted dual pins: the wire pin and its local-form twin are
        // recorded atomically; evidence mirrors both columns.
        let accepted_evidence = match checkpoint.and_then(|checkpoint| checkpoint.accepted.as_ref())
        {
            Some(accepted) => TailSideEvidence::Present(accepted.portable_tail.clone()),
            None => TailSideEvidence::ConfirmedAbsent,
        };
        let accepted_local_evidence =
            match checkpoint.and_then(|checkpoint| checkpoint.accepted_local_tail.as_ref()) {
                Some(tail) => TailSideEvidence::Present(tail.clone()),
                None => TailSideEvidence::ConfirmedAbsent,
            };
        // Cloud side.
        let cloud_evidence = match thread {
            Some(thread) => match (&thread.portable_tail, is_codex) {
                (Some(tail), _) => TailSideEvidence::Present(tail.clone()),
                (None, true) => TailSideEvidence::Invalid,
                (None, false) => TailSideEvidence::Present(synthesized_whole_thread_tail(
                    &thread.lineage_codec,
                    thread.message_count,
                    &thread.semantic_digest,
                )),
            },
            None => TailSideEvidence::ConfirmedAbsent,
        };
        // Local side. A binding that names a provider alias promises a
        // local copy: its absence from capture is unproven loss, never a
        // confirmed absence.
        let expected_local_missing = local.is_none()
            && local_binding.is_some_and(|binding| binding.provider_thread_id.is_some());
        let local_observation = if expected_local_missing {
            LocalTailObservation {
                evidence: TailSideEvidence::Invalid,
                raw_range_pin: None,
                verified_acceptance: None,
            }
        } else if is_codex {
            codex_observations
                .remove(&thread_id)
                .ok_or_else(|| "codex local observation missing for reviewed thread".to_string())?
        } else {
            // Providers without a cursor codec prove their tail from
            // captured bytes; a local row without them cannot happen in a
            // correctly assembled review and fails the provider closed.
            let captured = match local {
                None => None,
                Some(local) => Some(local.captured.as_ref().ok_or_else(|| {
                    format!(
                        "{} conversation was reviewed without captured bytes",
                        provider_label
                    )
                })?),
            };
            let evidence = match captured {
                None => TailSideEvidence::ConfirmedAbsent,
                Some(captured) => match &captured.conversation.lineage {
                    CodecLineage::Complete {
                        chain,
                        semantic_digest,
                        ..
                    } => TailSideEvidence::Present(synthesized_whole_thread_tail(
                        adapter.codec_key(),
                        chain.len() as u64,
                        semantic_digest,
                    )),
                    CodecLineage::Unknown { .. } => TailSideEvidence::Invalid,
                },
            };
            LocalTailObservation {
                evidence,
                raw_range_pin: captured.map(|captured| RawRangePin {
                    start_byte_offset: 0,
                    end_byte_offset: captured.transcript.bytes.len() as u64,
                    range_sha256: hex_digest(Sha256::digest(&captured.transcript.bytes).as_slice()),
                }),
                verified_acceptance: None,
            }
        };
        // A proven adoption re-establishes both acceptance pins for this
        // review only; the durable pins are written by the Push or Pull it
        // enables.
        let (accepted_evidence, accepted_local_evidence) =
            match (&accepted_evidence, &local_observation.verified_acceptance) {
                (TailSideEvidence::ConfirmedAbsent, Some(adoption)) => (
                    TailSideEvidence::Present(adoption.wire_tail.clone()),
                    TailSideEvidence::Present(adoption.local_tail.clone()),
                ),
                _ => (accepted_evidence, accepted_local_evidence),
            };
        let physical_detached = matches!(
            checkpoint.map(|checkpoint| &checkpoint.physical),
            Some(PhysicalMappingState::Detached { .. })
        );

        let relation_result: Result<TailClassification, TailEvidenceBlocker> =
            if descriptor.is_none() && (local.is_some() || thread.is_some()) {
                Err(TailEvidenceBlocker::InvalidSide)
            } else {
                VerifiedTailEvidence::new(
                    accepted_evidence.clone(),
                    accepted_local_evidence.clone(),
                    local_observation.evidence.clone(),
                    cloud_evidence.clone(),
                )
                .map(|evidence| compare_conversation_tail(&evidence))
            };
        let evidence_blocker = relation_result.as_ref().err().copied();
        let relation = if let Ok(relation) = relation_result {
            relation
        } else {
            TailClassification::Unknown
        };
        // Codex capturability is proven by the resolved logical root, not by
        // the append-bearing leaf in isolation. A valid paginated thread can
        // have an empty leaf while all of its messages live in history_base.
        // `observe_codex_local` validates that complete chain (or a trusted
        // checkpoint continuation) before returning Present evidence.
        let local_capturable = if is_codex {
            local.is_some() && matches!(&local_observation.evidence, TailSideEvidence::Present(_))
        } else {
            local.is_some_and(|local| {
                local
                    .captured
                    .as_ref()
                    .is_some_and(|captured| captured.conversation.lineage.is_complete())
            }) && !matches!(local_observation.evidence, TailSideEvidence::Invalid)
        };
        let cloud_readable = thread.is_some_and(|thread| {
            matches!(thread.lineage_status, ConversationLineageStatus::Complete)
        }) && !matches!(cloud_evidence, TailSideEvidence::Invalid);
        let capabilities = ConversationCapabilities {
            binding: if local_binding.is_some() {
                BindingResolution::Bound
            } else {
                local
                    .map(|local| local.binding_resolution)
                    .unwrap_or(BindingResolution::Missing)
            },
            local_safely_capturable: local_capturable && local_observation.raw_range_pin.is_some(),
            cloud_materializable: cloud_readable,
            operation_state: if pending_resolution_threads.contains(&thread_id) {
                ConversationOperationState::Pending
            } else if physical_detached {
                ConversationOperationState::Detached
            } else {
                ConversationOperationState::Ready
            },
            // The durable pin's presence, not the reviewed evidence: the
            // adoption probe upgrades evidence for this review only, and the
            // Pull it enables is what writes the pin.
            acceptance_durably_pinned: checkpoint
                .and_then(|checkpoint| checkpoint.accepted.as_ref())
                .is_some(),
        };
        let classified = ClassifiedThread::new(relation, capabilities);
        let push = classified.decision(ConversationPlanDirection::Push);
        let pull = classified.decision(ConversationPlanDirection::Pull);

        // Stored internal-subagent rollouts are never offered by Pull.
        if purpose.builds_pull_plan() && pull.action == ConversationReviewAction::CreateLocal {
            if let Some((logical_path, sha256, size)) = offerable_cloud_transcripts.get(&thread_id)
            {
                if cached_transcript_analysis(sha256, *size, bytes, logical_path)?
                    .is_internal_subagent
                {
                    continue;
                }
            }
        }

        let source_resource_id = local
            .map(|local| local.resource_id.clone())
            .or_else(|| thread.map(|thread| thread.resource_id.clone()));
        let provider_thread_id = local
            .map(|local| local.provider_thread_id.clone())
            .or_else(|| local_binding.and_then(|binding| binding.provider_thread_id.clone()));
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
            .or_else(|| descriptor.map(|descriptor| descriptor.display_name.clone()))
            .unwrap_or_else(|| thread_id.to_string());
        let conversation_id = conversation_resource_id(provider, &thread_id)?;
        let review_resource_id = match source_resource_id
            .clone()
            .or_else(|| descriptor.map(|descriptor| descriptor.resource_id.clone()))
        {
            Some(resource_id) => resource_id,
            None => conversation_id.clone(),
        };

        entries.push(ThreadSyncEntry {
            provider: provider.clone(),
            provider_display_name: provider_label.clone(),
            access: ProviderAccessState::BoundReady,
            thread_id: provider_thread_id
                .clone()
                .or_else(|| thread.and_then(|thread| thread.source_thread_id.clone()))
                .unwrap_or_else(|| thread_id.to_string()),
            conversation_resource_id: conversation_id,
            resource_id: review_resource_id.clone(),
            display_name,
            state: relation_sync_state(relation),
            local_present: local.is_some(),
            storage_present: thread.is_some(),
            status_detail: capabilities
                .operation_state
                .eq(&ConversationOperationState::Pending)
                .then(|| {
                    format!(
                        "A {} sync operation is still pending for this conversation. \
                         Finish or recover it first.",
                        provider_label
                    )
                })
                .or_else(|| {
                    physical_detached.then(|| {
                        "This conversation's local mapping was replaced. Pull to \
                         re-establish it."
                            .to_string()
                    })
                })
                .or_else(|| {
                    evidence_blocker
                        .is_some()
                        .then(|| "Conversation state could not be proven safely.".to_string())
                }),
            local_updated_at: local.map(|local| local.source_mtime),
            storage_updated_at: thread.map(|thread| thread.updated_at),
            cloud_thread_id: Some(thread_id.clone()),
            forked_from: thread
                .and_then(|thread| thread.forked_from.clone())
                .or_else(|| local_binding.and_then(|binding| binding.forked_from.clone())),
            provider_thread_id: provider_thread_id.clone(),
            push_decision: push.clone().into(),
            pull_decision: pull.clone().into(),
            origin_label,
        });

        if purpose == ReviewPurpose::Observe {
            continue;
        }
        // Executable actions freeze the local target from captured bytes,
        // or from the watcher-clean pin a previous capture froze. A local
        // row reviewed without either can only produce non-selectable
        // actions; its Absent pin makes any out-of-contract apply attempt
        // fail the stale-pin check instead of running.
        let expected_local_target = local
            .and_then(|local| local.captured.as_ref())
            .map(|captured| {
                TargetExpectation::Sha256(hex_digest(
                    Sha256::digest(&captured.transcript.bytes).as_slice(),
                ))
            })
            .or_else(|| {
                local.and_then(|local| candidate_pins.get(local.resource_id.as_str()).cloned())
            })
            .unwrap_or(TargetExpectation::Absent);
        let required_codec = thread
            .map(|thread| thread.lineage_codec.clone())
            .or_else(|| {
                local
                    .and_then(|local| local.captured.as_ref())
                    .map(|captured| captured.conversation.lineage_codec.clone())
            })
            .unwrap_or_else(|| adapter.codec_key().to_string());
        let accepted_tail_pin = match &accepted_evidence {
            TailSideEvidence::Present(tail) => Some(tail.clone()),
            _ => None,
        };
        let accepted_local_tail_pin = match &accepted_local_evidence {
            TailSideEvidence::Present(tail) => Some(tail.clone()),
            _ => None,
        };
        let local_tail_pin = match &local_observation.evidence {
            TailSideEvidence::Present(tail) => Some(tail.clone()),
            _ => None,
        };
        let cloud_tail_pin = match &cloud_evidence {
            TailSideEvidence::Present(tail) => Some(tail.clone()),
            _ => None,
        };
        // The post-maintenance checkpoint generation this review saw; apply
        // requires the live checkpoint to still carry exactly this value.
        let checkpoint_generation_pin = maintained
            .get(&thread_id)
            .copied()
            .or_else(|| checkpoint.map(|checkpoint| checkpoint.generation));

        if purpose.builds_push_plan()
            && classified
                .planned_action(ConversationPlanDirection::Push)
                .is_some()
        {
            let publishes = matches!(
                push.action,
                ConversationReviewAction::Publish | ConversationReviewAction::CreateCloud
            );
            let push_action = build_conversation_plan_action(ConversationPlanActionRequest {
                action_id: ActionId::parse(generated_named_id("action")?)?,
                direction: ConversationPlanDirection::Push,
                classified,
                provider: provider.clone(),
                thread_id: thread_id.clone(),
                source_resource_id: source_resource_id.clone(),
                proposed_provider_thread_id: None,
                keep_both: None,
                pins: ConversationActionPins {
                    context: ConversationContextPins {
                        host_id: binding.host_id.clone(),
                        replica_id: binding.replica_id.clone(),
                        local_project_id: binding.local_project_id.clone(),
                        provider_profile_id: profile_id.clone(),
                        host_identity_revision: host_identity.revision,
                        profile_revision: profile.revision,
                        project_binding_revision: binding.revision,
                        bindings_document_revision: document.revision,
                        binding_revision: local_binding.map(|binding| binding.revision),
                    },
                    revision: ConversationRevisionPins {
                        bundle_revision: pinned_revision,
                        commit_id: pinned_commit_id.clone(),
                        manifest_sha256: pinned_manifest_sha256.clone(),
                        scope_revision: pinned_scope_revision,
                        scope_sha256: pinned_scope_sha256.clone(),
                    },
                    tail: ConversationTailPins {
                        accepted: accepted_tail_pin.clone(),
                        accepted_local: accepted_local_tail_pin.clone(),
                        local: local_tail_pin.clone(),
                        cloud: cloud_tail_pin.clone(),
                        checkpoint_generation: checkpoint_generation_pin,
                        lineage_digest: None,
                        semantic_digest: None,
                        raw_range: publishes
                            .then(|| local_observation.raw_range_pin.clone())
                            .flatten(),
                    },
                    target: ConversationTargetPins {
                        local: expected_local_target.clone(),
                        provider_state: Vec::new(),
                    },
                    codec: required_codec.clone(),
                    expires_at,
                },
            })?
            .expect("selectable Push classification produces an action");
            push_actions.push(push_action);
        }

        // A durably pinned current thread produces no Pull row at all: the
        // plan-level apply advances scope bases, so the review only carries
        // rows that change something. `NoChanges` also stays a Push-only
        // action inside validated plans.
        if purpose.builds_pull_plan() && pull.action != ConversationReviewAction::NoChanges {
            let mut proposed_provider_thread_id = None;
            let mut expected_provider_state = Vec::new();
            let mut keep_both = None;
            let materializing_digests = matches!(
                pull.action,
                ConversationReviewAction::MaterializeCloud
                    | ConversationReviewAction::KeepBoth
                    | ConversationReviewAction::CreateLocal
            );
            match pull.action {
                ConversationReviewAction::MaterializeCloud => {
                    let local = local.ok_or_else(|| {
                        "cloud materialization lacks a captured local state".to_string()
                    })?;
                    if local.binding.is_none() {
                        return Err(
                            "cloud materialization requires a bound provider session".to_string()
                        );
                    }
                    let existing_provider_id = local.provider_thread_id.clone();
                    // Codex can create a physical history_base child at any
                    // time without changing the parent's bytes or binding.
                    // No target digest or dependency rescan can make an
                    // in-place replacement atomic with that external writer.
                    // Every Codex cloud update is therefore copy-on-write:
                    // install under a reviewed fresh identity and retain the
                    // prior rollout as a possible native dependency. Other
                    // providers have no physical rollout graph and retain the
                    // ordinary in-place recipe.
                    let relocate = is_codex;
                    let provider_id = if relocate {
                        mint_provider_thread_id(is_codex, &mut occupied_provider_ids)?
                    } else {
                        existing_provider_id
                    };
                    let sidecars = thread
                        .map(|thread| conversation_sidecar_refs(provider, &thread_id, thread))
                        .transpose()?
                        .unwrap_or_default();
                    let targets = adapter.materialization_targets(
                        &super::providers::MaterializationTargetRequest {
                            provider_home,
                            project_root: &binding.project_root,
                            provider_session_id: &provider_id,
                            source_provider_session_id: thread
                                .and_then(|thread| thread.source_thread_id.as_deref()),
                            existing_transcript_path: (!relocate)
                                .then_some(local.source_path.as_path()),
                            sidecars: &sidecars,
                            planned_at: created_at,
                        },
                    )?;
                    for planned in &targets {
                        let expectation = bounded_file_expectation(&planned.target_path)?;
                        if relocate && expectation != TargetExpectation::Absent {
                            return Err(format!(
                                "reserved {} target '{}' is already occupied",
                                provider_label,
                                planned.target_path.display()
                            ));
                        }
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: planned.target_path.to_string_lossy().into_owned(),
                            expectation: if relocate {
                                TargetExpectation::Absent
                            } else {
                                expectation
                            },
                        });
                    }
                    proposed_provider_thread_id = Some(provider_id);
                    if let Some(index_path) = &index_path {
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: index_path.to_string_lossy().into_owned(),
                            expectation: index_expectation
                                .clone()
                                .unwrap_or(TargetExpectation::Absent),
                        });
                    }
                }
                ConversationReviewAction::KeepBoth => {
                    // The binding move: the local rollout is never edited,
                    // only pinned byte-identical; the remote branch
                    // materializes under fresh identities.
                    let local = local.ok_or_else(|| {
                        "conversation keep-both lacks a captured local branch".to_string()
                    })?;
                    let target = local.source_path.clone();
                    expected_provider_state.push(ProviderStateExpectation {
                        target_path: target.to_string_lossy().into_owned(),
                        expectation: bounded_file_expectation(&target)?,
                    });
                    let remote_provider_thread_id =
                        mint_provider_thread_id(is_codex, &mut occupied_provider_ids)?;
                    let sidecars = thread
                        .map(|thread| conversation_sidecar_refs(provider, &thread_id, thread))
                        .transpose()?
                        .unwrap_or_default();
                    let targets = adapter.materialization_targets(
                        &super::providers::MaterializationTargetRequest {
                            provider_home,
                            project_root: &binding.project_root,
                            provider_session_id: &remote_provider_thread_id,
                            source_provider_session_id: thread
                                .and_then(|thread| thread.source_thread_id.as_deref()),
                            existing_transcript_path: None,
                            sidecars: &sidecars,
                            planned_at: created_at,
                        },
                    )?;
                    for planned in &targets {
                        if planned.target_path.exists() {
                            return Err(format!(
                                "reserved {} target '{}' is already occupied",
                                provider_label,
                                planned.target_path.display()
                            ));
                        }
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: planned.target_path.to_string_lossy().into_owned(),
                            expectation: TargetExpectation::Absent,
                        });
                    }
                    if let Some(index_path) = &index_path {
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: index_path.to_string_lossy().into_owned(),
                            expectation: index_expectation
                                .clone()
                                .unwrap_or(TargetExpectation::Absent),
                        });
                    }
                    // The preservation ordinal is max+1 over every provenance
                    // record naming this origin, across the pinned cloud
                    // snapshot and the local bindings document. Both inputs
                    // are re-validated at apply (scope sha + bindings
                    // revision), so a same-device advance forces a replan;
                    // concurrent keep-boths of the same origin on other
                    // devices can still mint a duplicate ordinal, which is
                    // accepted — the device label disambiguates the marker.
                    let next_ordinal = provider_scope
                        .into_iter()
                        .flat_map(|scope| scope.conversations.values())
                        .filter_map(|thread| thread.preserved.as_ref())
                        .chain(
                            bindings_by_thread
                                .values()
                                .filter_map(|binding| binding.preserved.as_ref()),
                        )
                        .filter(|preserved| preserved.preserved_from == thread_id)
                        .map(|preserved| preserved.ordinal)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1);
                    let device_label = match host_identity
                        .local_display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|label| !label.is_empty())
                    {
                        Some(label) => label.to_string(),
                        None => {
                            let (origin_host_token, _) = repository
                                .derive_origin_tokens(&binding.bundle_id, &binding.replica_id)?;
                            fallback_origin_host_label(&origin_host_token)
                        }
                    };
                    let base_title = local
                        .descriptor
                        .as_ref()
                        .map(|descriptor| descriptor.display_name.clone())
                        .filter(|name| !name.trim().is_empty() && name != &local.provider_thread_id)
                        .or_else(|| {
                            thread.and_then(|thread| {
                                thread.display_title.clone().filter(|title| {
                                    !title.trim().is_empty()
                                        && thread.source_thread_id.as_deref()
                                            != Some(title.as_str())
                                })
                            })
                        });
                    keep_both = Some(KeepBothPlan {
                        preserved_thread_id: ThreadId::parse(generated_named_id("thread")?)?,
                        remote_provider_thread_id,
                        preserved: PreservedProvenance {
                            preserved_from: thread_id.clone(),
                            ordinal: next_ordinal,
                        },
                        preserved_display_title: compose_preserved_title(
                            base_title.as_deref(),
                            next_ordinal,
                            &device_label,
                        ),
                    });
                }
                ConversationReviewAction::CreateLocal => {
                    let provider_id =
                        mint_provider_thread_id(is_codex, &mut occupied_provider_ids)?;
                    let sidecars = thread
                        .map(|thread| conversation_sidecar_refs(provider, &thread_id, thread))
                        .transpose()?
                        .unwrap_or_default();
                    let targets = adapter.materialization_targets(
                        &super::providers::MaterializationTargetRequest {
                            provider_home,
                            project_root: &binding.project_root,
                            provider_session_id: &provider_id,
                            source_provider_session_id: thread
                                .and_then(|thread| thread.source_thread_id.as_deref()),
                            existing_transcript_path: None,
                            sidecars: &sidecars,
                            planned_at: created_at,
                        },
                    )?;
                    for planned in &targets {
                        if planned.target_path.exists() {
                            return Err(format!(
                                "reserved {} target '{}' is already occupied",
                                provider_label,
                                planned.target_path.display()
                            ));
                        }
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: planned.target_path.to_string_lossy().into_owned(),
                            expectation: TargetExpectation::Absent,
                        });
                    }
                    proposed_provider_thread_id = Some(provider_id);
                    if let Some(index_path) = &index_path {
                        expected_provider_state.push(ProviderStateExpectation {
                            target_path: index_path.to_string_lossy().into_owned(),
                            expectation: index_expectation
                                .clone()
                                .unwrap_or(TargetExpectation::Absent),
                        });
                    }
                }
                _ => {}
            }
            let action_id = classified
                .planned_action(ConversationPlanDirection::Pull)
                .map(|_| ActionId::parse(generated_named_id("action")?))
                .transpose()?;
            if let Some(action_id) = action_id.clone() {
                let pull_action = build_conversation_plan_action(ConversationPlanActionRequest {
                    action_id,
                    direction: ConversationPlanDirection::Pull,
                    classified,
                    provider: provider.clone(),
                    thread_id: thread_id.clone(),
                    source_resource_id: source_resource_id.clone(),
                    proposed_provider_thread_id,
                    keep_both,
                    pins: ConversationActionPins {
                        context: ConversationContextPins {
                            host_id: binding.host_id.clone(),
                            replica_id: binding.replica_id.clone(),
                            local_project_id: binding.local_project_id.clone(),
                            provider_profile_id: profile_id.clone(),
                            host_identity_revision: host_identity.revision,
                            profile_revision: profile.revision,
                            project_binding_revision: binding.revision,
                            bindings_document_revision: document.revision,
                            binding_revision: local_binding.map(|binding| binding.revision),
                        },
                        revision: ConversationRevisionPins {
                            bundle_revision: pinned_revision,
                            commit_id: pinned_commit_id.clone(),
                            manifest_sha256: pinned_manifest_sha256.clone(),
                            scope_revision: pinned_scope_revision,
                            scope_sha256: pinned_scope_sha256.clone(),
                        },
                        tail: ConversationTailPins {
                            accepted: accepted_tail_pin,
                            accepted_local: accepted_local_tail_pin,
                            local: local_tail_pin,
                            cloud: cloud_tail_pin,
                            checkpoint_generation: checkpoint_generation_pin,
                            lineage_digest: materializing_digests
                                .then(|| thread.map(|thread| thread.lineage_digest.clone()))
                                .flatten(),
                            semantic_digest: materializing_digests
                                .then(|| thread.map(|thread| thread.semantic_digest.clone()))
                                .flatten(),
                            raw_range: None,
                        },
                        target: ConversationTargetPins {
                            local: expected_local_target,
                            provider_state: expected_provider_state,
                        },
                        codec: required_codec,
                        expires_at,
                    },
                })?
                .expect("selectable Pull classification produces an action");
                pull_actions.push(pull_action);
            }
            pull_review.push(ConversationReviewRow {
                provider: provider.clone(),
                thread_id: thread_id.clone(),
                resource_id: review_resource_id,
                action: pull.action,
                blocker: pull.blocker,
                blocked_reason: pull.blocked_reason.clone(),
                action_id,
            });
        }
    }
    Ok(ThreadReview {
        entries,
        push_actions,
        pull_actions,
        pull_review,
        suppressed_provider_ids: suppressed_provider_ids
            .into_iter()
            .map(|provider_id| (provider.clone(), provider_id))
            .collect(),
        provider_errors: BTreeMap::new(),
    })
}

/// Mint one provider thread ID for a new local copy (CreateLocal or the
/// remote branch of KeepBoth): the CLI-first UUIDv7 allocator for Codex and
/// the adapter's UUID convention for other providers. Minted IDs join the
/// occupied set so one review never proposes the same identity twice.
fn mint_provider_thread_id(
    is_codex: bool,
    occupied: &mut BTreeSet<String>,
) -> Result<String, String> {
    if is_codex {
        let taken = |candidate: &str| occupied.contains(candidate);
        let minted = codex_thread_allocator::allocate_codex_thread_ids(1, &taken)?
            .into_iter()
            .next()
            .ok_or_else(|| "Codex thread allocation returned no id".to_string())?;
        occupied.insert(minted.clone());
        Ok(minted)
    } else {
        loop {
            let candidate = generated_uuid_v4()?;
            if occupied.insert(candidate.clone()) {
                return Ok(candidate);
            }
        }
    }
}

/// The sidecar references (`(kind, kind-relative path)`) carried by one
/// portable conversation, decoded from its canonical sidecar namespace.
fn conversation_sidecar_refs(
    provider: &ProviderKey,
    thread_id: &ThreadId,
    thread: &ConversationThread,
) -> Result<Vec<(String, String)>, String> {
    let prefix = format!("state/conversations/{}/{}/sidecars/", provider, thread_id);
    let mut sidecars = Vec::new();
    for file in &thread.files {
        if let ConversationFileRole::Sidecar { sidecar_kind } = &file.role {
            let remainder = file
                .logical_path
                .as_str()
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_prefix(&format!("{}/", sidecar_kind)))
                .ok_or_else(|| {
                    format!(
                        "conversation sidecar '{}' is outside its namespace",
                        file.logical_path
                    )
                })?;
            sidecars.push((sidecar_kind.clone(), remainder.to_string()));
        }
    }
    Ok(sidecars)
}

#[cfg(test)]
impl LocalTailObservation {
    /// Bare observation for the watch service's own suite.
    pub(crate) fn for_watch_service_tests(evidence: TailSideEvidence) -> Self {
        Self {
            evidence,
            raw_range_pin: None,
            verified_acceptance: None,
        }
    }
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
    let resolved = resolve_project_binding(repository, binding)?;
    // Synthesize metadata views from the hand-built capture so tests drive
    // the review through the same assembly production uses.
    let views = captured
        .conversations
        .iter()
        .map(
            |(resource_id, conversation)| provider_capture::LocalSessionView {
                provider: conversation.provider.clone(),
                provider_thread_id: conversation.source_thread_id.clone().unwrap_or_default(),
                transcript_logical_path: conversation.transcript_logical_path.clone(),
                source_path: conversation
                    .source_physical_path
                    .clone()
                    .unwrap_or_default(),
                source_mtime: captured
                    .files
                    .get(&conversation.transcript_logical_path)
                    .map(|file| file.source_mtime)
                    .unwrap_or(0),
                source_size: captured
                    .files
                    .get(&conversation.transcript_logical_path)
                    .map(|file| file.bytes.len() as u64)
                    .unwrap_or(0),
                descriptor: provider_capture::ResourceCandidate {
                    metadata: BTreeMap::new(),
                    resource_id: resource_id.clone(),
                    kind: CaptureResourceKind::Conversation,
                    provider: Some(conversation.provider.clone()),
                    provider_resource_type: Some("session".to_string()),
                    display_name: conversation
                        .source_thread_id
                        .clone()
                        .unwrap_or_else(|| resource_id.clone()),
                    relative_cwd: None,
                    apply_policy: provider_capture::CaptureApplyPolicy::SafeFile,
                    selected_by_default: true,
                    blocked_reason: None,
                    logical_paths: vec![conversation.transcript_logical_path.clone()],
                },
            },
        )
        .collect::<Vec<_>>();
    let review = thread_review(
        repository,
        Some(snapshot),
        bytes,
        &views,
        None,
        captured,
        &BTreeMap::new(),
        &resolved,
        Some(&snapshot.storage_id),
        created_at,
        created_at.saturating_add(PLAN_LIFETIME_SECS),
        ReviewPurpose::Observe,
    )?;
    if let Some((provider, error)) = review.provider_errors.into_iter().next() {
        return Err(format!("{}: {}", provider, error));
    }
    Ok(review.entries)
}

#[cfg(test)]
fn get_project_thread_sync_comparison_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ThreadSyncComparison, String> {
    get_project_thread_sync_comparison_with_progress(
        repository,
        local_project_id,
        storage_id,
        &mut |_, _| {},
    )
}

struct ProjectLocalRead {
    activity_project: ResolvedActivityProject,
    resolved: ResolvedBinding,
    capture_request: CaptureRequest,
    discovery: Discovery,
    views: Vec<LocalSessionView>,
}

fn load_project_local_read(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<ProjectLocalRead, String> {
    let activity_project =
        project_activity::resolve_activity_project(repository, local_project_id)?;
    let resolved = ResolvedBinding {
        binding: activity_project.binding.clone(),
        homes: activity_project
            .homes
            .iter()
            .map(|(provider, home)| (provider.clone(), home.home.to_string_lossy().into_owned()))
            .collect(),
    };
    let capture_request = capture_request_for_resolved_binding(repository, &resolved)?;
    let (discovery, views) = super::codex_watch_service::session_watch_service().discovery_for(
        provider_registry(),
        local_project_id,
        &capture_request,
    )?;
    Ok(ProjectLocalRead {
        activity_project,
        resolved,
        capture_request,
        discovery,
        views,
    })
}

struct ProjectRemoteRead {
    storage_name: String,
    link: ProjectStorageLink,
    engine: StorageEngine,
    current: Option<BundleSnapshot>,
    remote_continuity: RemoteContinuity,
}

fn load_project_remote_read(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ProjectRemoteRead, String> {
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
    let link = project_link(&config, local_project_id, storage_id)
        .cloned()
        .ok_or_else(|| "project is not linked to this storage".to_string())?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let current = engine.inspect_optional(&project.bundle_id)?;
    let remote_continuity = classify_remote_continuity(
        current.as_ref().map(|snapshot| &snapshot.head),
        repository_base_from_link(&link).as_ref(),
        &mut continuity_probe(&engine, &project.bundle_id),
    );
    Ok(ProjectRemoteRead {
        storage_name,
        link,
        engine,
        current,
        remote_continuity,
    })
}

fn build_comparison_from_reads(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    local: &ProjectLocalRead,
    remote: &ProjectRemoteRead,
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ThreadSyncComparison, String> {
    let comparison = build_thread_sync_comparison(
        repository,
        local_project_id,
        storage_id,
        remote.storage_name.clone(),
        &remote.link,
        &local.resolved,
        &local.capture_request,
        &local.discovery,
        &local.views,
        &remote.engine,
        remote.current.as_ref(),
        false,
        remote.remote_continuity.clone(),
        on_phase,
    )?
    .comparison;
    Ok(comparison)
}

pub(super) fn get_project_thread_sync_comparison_with_progress(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ThreadSyncComparison, String> {
    let metrics_before = super::sync_metrics::global_snapshot();
    let phase_started = Instant::now();
    let local = load_project_local_read(repository, local_project_id)?;
    on_phase(
        "load binding and discover local state",
        phase_started.elapsed(),
    );
    let phase_started = Instant::now();
    let remote = load_project_remote_read(repository, local_project_id, storage_id)?;
    on_phase("inspect remote head", phase_started.elapsed());
    let comparison = build_comparison_from_reads(
        repository,
        local_project_id,
        storage_id,
        &local,
        &remote,
        on_phase,
    )?;
    super::sync_metrics::log_operation_delta("refresh", &metrics_before);
    Ok(comparison)
}

struct ThreadComparisonBuild {
    comparison: ThreadSyncComparison,
    push_actions: Vec<ConversationPlanAction>,
}

/// Split reviewed candidates into watcher-served byte pins and the set that
/// still needs authoritative capture. Non-Codex candidates always capture.
fn reuse_candidate_pins(
    capture_request: &CaptureRequest,
    local_project_id: &LocalProjectId,
    views: &[provider_capture::LocalSessionView],
    base_selection: &BTreeSet<String>,
    selection: BTreeSet<String>,
) -> (BTreeSet<String>, BTreeMap<String, TargetExpectation>) {
    let Some(home) = codex_home_of(capture_request) else {
        return (selection, BTreeMap::new());
    };
    let codex_candidates: BTreeSet<String> = selection
        .iter()
        .filter(|resource_id| !base_selection.contains(*resource_id))
        .filter(|resource_id| {
            views.iter().any(|view| {
                view.descriptor.resource_id == **resource_id && view.provider.as_str() == "codex"
            })
        })
        .cloned()
        .collect();
    let served = super::codex_watch_service::session_watch_service().take_reusable_candidate_pins(
        &home,
        local_project_id,
        &codex_candidates,
    );
    let remaining = selection
        .into_iter()
        .filter(|resource_id| !served.contains_key(resource_id))
        .collect();
    (remaining, served)
}

/// Freeze the byte pins of every freshly captured Codex candidate for the
/// next quiet review.
fn store_candidate_pins_from_capture(
    capture_request: &CaptureRequest,
    local_project_id: &LocalProjectId,
    views: &[provider_capture::LocalSessionView],
    base_selection: &BTreeSet<String>,
    captured: &provider_capture::CapturedResources,
) {
    let Some(home) = codex_home_of(capture_request) else {
        return;
    };
    let stored: Vec<_> = captured
        .conversations
        .iter()
        .filter(|(resource_id, conversation)| {
            conversation.provider.as_str() == "codex" && !base_selection.contains(*resource_id)
        })
        .filter_map(|(resource_id, conversation)| {
            let transcript = captured.files.get(&conversation.transcript_logical_path)?;
            let source_path = views
                .iter()
                .find(|view| view.descriptor.resource_id == *resource_id)
                .map(|view| view.source_path.clone())?;
            Some(super::codex_watch_service::StoredCandidatePin {
                resource_id: resource_id.clone(),
                depends_on: vec![source_path],
                expected_local_target: TargetExpectation::Sha256(hex_digest(
                    Sha256::digest(&transcript.bytes).as_slice(),
                )),
            })
        })
        .collect();
    if !stored.is_empty() {
        super::codex_watch_service::session_watch_service().store_candidate_pins(
            &home,
            local_project_id,
            stored,
        );
    }
}

/// The bound Codex home of a capture request, when one is bound.
fn codex_home_of(capture_request: &CaptureRequest) -> Option<PathBuf> {
    capture_request
        .provider_homes
        .iter()
        .find(|(provider, _)| provider.as_str() == "codex")
        .map(|(_, home)| home.clone())
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
    link: &ProjectStorageLink,
    resolved: &ResolvedBinding,
    capture_request: &CaptureRequest,
    discovery: &provider_capture::Discovery,
    views: &[provider_capture::LocalSessionView],
    engine: &StorageEngine,
    current: Option<&BundleSnapshot>,
    build_push_actions: bool,
    remote_continuity: RemoteContinuity,
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ThreadComparisonBuild, String> {
    let phase_started = Instant::now();
    let discovered = discovery.project_inventory();
    let mut warnings = discovered.warnings.clone();
    // Every bound provider's conversations enter the typed pipeline; no
    // provider filter and no generic fallback. Codex status rows classify
    // from bounded metadata plus checkpoint observation, so a plain
    // comparison reads no Codex transcript; providers without a cursor
    // codec still prove their tail from captured bytes, and action-building
    // reviews capture everything their byte pins require.
    // Providers without a cursor codec always prove their tail from
    // captured bytes; Codex rows classify from bounded metadata plus
    // checkpoint observation and are captured only as byte-pinning
    // candidates below.
    let base_selection = discovered
        .resources
        .iter()
        .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
        .filter(|candidate| {
            candidate
                .provider
                .as_ref()
                .map(|provider| provider.as_str())
                != Some("codex")
        })
        .map(|candidate| candidate.resource_id.clone())
        .collect::<BTreeSet<_>>();
    let created_at = now_secs();
    let run_review = |captured: &provider_capture::CapturedResources,
                      candidate_pins: &BTreeMap<String, TargetExpectation>,
                      push_actions: bool|
     -> Result<ThreadReview, String> {
        match current {
            Some(snapshot) => {
                let mut bytes = engine.byte_source(snapshot, Some(object_cache()), None);
                thread_review(
                    repository,
                    Some(snapshot),
                    &mut bytes,
                    views,
                    Some(discovery),
                    captured,
                    candidate_pins,
                    resolved,
                    Some(storage_id),
                    created_at,
                    created_at.saturating_add(PLAN_LIFETIME_SECS),
                    if push_actions {
                        ReviewPurpose::PushPlan
                    } else {
                        ReviewPurpose::Observe
                    },
                )
            }
            None => {
                let mut bytes = EmptyBundleBytes;
                thread_review(
                    repository,
                    None,
                    &mut bytes,
                    views,
                    Some(discovery),
                    captured,
                    candidate_pins,
                    resolved,
                    Some(storage_id),
                    created_at,
                    created_at.saturating_add(PLAN_LIFETIME_SECS),
                    if push_actions {
                        ReviewPurpose::PushPlan
                    } else {
                        ReviewPurpose::Observe
                    },
                )
            }
        }
    };
    let captured = provider_capture::capture_from_discovery(
        provider_registry(),
        capture_request,
        discovery,
        &base_selection,
    )?;
    let no_pins = BTreeMap::new();
    let (captured, mut review) = if build_push_actions {
        // Candidate selection is a conservative work scheduler, never a
        // second policy engine: one classification pass (served from the
        // caches when quiet) picks every selectable push row; candidates
        // the watcher proves unchanged reuse their frozen byte pin, the
        // rest are captured authoritatively, and the final decisions come
        // from the same shared decision path.
        let preliminary = run_review(&captured, &no_pins, false)?;
        let mut selection = base_selection.clone();
        for entry in preliminary
            .entries
            .iter()
            .filter(|entry| entry.push_decision.selectable && entry.local_present)
        {
            selection.insert(entry.resource_id.to_string());
        }
        let (selection, candidate_pins) = reuse_candidate_pins(
            capture_request,
            local_project_id,
            views,
            &base_selection,
            selection,
        );
        let captured = if selection == base_selection {
            captured
        } else {
            provider_capture::capture_from_discovery(
                provider_registry(),
                capture_request,
                discovery,
                &selection,
            )?
        };
        store_candidate_pins_from_capture(
            capture_request,
            local_project_id,
            views,
            &base_selection,
            &captured,
        );
        let review = run_review(&captured, &candidate_pins, true)?;
        (captured, review)
    } else {
        let review = run_review(&captured, &no_pins, false)?;
        (captured, review)
    };
    warnings.extend(captured.warnings.clone());
    on_phase("capture local conversations", phase_started.elapsed());
    let phase_started = Instant::now();

    // Locally discovered but uncapturable sessions of any bound provider
    // hold their own row instead of failing the review.
    let represented_provider_ids = review
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .provider_thread_id
                .clone()
                .map(|provider_id| (entry.provider.clone(), provider_id))
        })
        .collect::<BTreeSet<_>>();
    let conversation_bindings = repository.load_conversation_bindings()?;
    let fallback_config = repository.load_config()?;
    let codex_fallback_state = resolved
        .binding
        .profile_ids
        .get(&codex_key())
        .map(|profile_id| {
            let scoped = conversation_bindings
                .bindings
                .iter()
                .filter(|binding| {
                    binding.host_id == resolved.binding.host_id
                        && binding.replica_id == resolved.binding.replica_id
                        && binding.local_project_id == resolved.binding.local_project_id
                        && binding.provider_profile_id == *profile_id
                })
                .cloned()
                .collect::<Vec<_>>();
            Ok::<_, String>((
                applied_provider_alias_owners(
                    repository,
                    &resolved.binding,
                    &codex_key(),
                    profile_id,
                )?,
                scoped,
            ))
        })
        .transpose()?;
    for candidate in discovered
        .resources
        .iter()
        .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
    {
        let Some(provider) = candidate.provider.clone() else {
            continue;
        };
        let provider_thread_id = provider_session_id_from_resource(&candidate.resource_id)
            .unwrap_or(&candidate.display_name)
            .to_string();
        if represented_provider_ids.contains(&(provider.clone(), provider_thread_id.clone()))
            || review
                .suppressed_provider_ids
                .contains(&(provider.clone(), provider_thread_id.clone()))
        {
            continue;
        }
        let detail = thread_capture_unavailable_detail(candidate.blocked_reason.as_deref());
        let blocked = ThreadSyncDecision {
            action: ConversationReviewAction::Blocked,
            blocker: Some(ConversationBlocker::CodecUnsupported),
            selectable: false,
            blocked_reason: Some(detail.clone()),
        };
        let provider_display_name = provider_registry()
            .get(&provider)
            .map(|adapter| adapter.descriptor().display_name)
            .unwrap_or_else(|| provider.to_string());
        let bound_thread_id = conversation_bindings
            .bindings
            .iter()
            .find(|binding| {
                binding.local_project_id == *local_project_id
                    && binding.provider_thread_id.as_deref() == Some(&provider_thread_id)
                    && resolved.binding.profile_ids.get(&provider)
                        == Some(&binding.provider_profile_id)
            })
            .map(|binding| binding.thread_id.clone());
        let thread_id = if let Some(thread_id) = bound_thread_id {
            thread_id
        } else if let (true, Some((owners, scoped)), Some(view)) = (
            provider == codex_key(),
            codex_fallback_state.as_ref(),
            views.iter().find(|view| {
                view.provider == provider && view.provider_thread_id == provider_thread_id
            }),
        ) {
            let moved_owners = displaced_provider_alias_owners(
                &resolved.binding.bundle_id,
                &provider,
                &provider_thread_id,
                scoped,
                owners,
            )?;
            if moved_owners.is_empty() {
                initial_conversation_thread_id(
                    &resolved.binding.bundle_id,
                    &provider,
                    &provider_thread_id,
                )?
            } else {
                project_activity::retained_provider_alias_identity(
                    &resolved.binding.bundle_id,
                    &resolved.binding.local_project_id,
                    &provider,
                    &provider_thread_id,
                    &view.source_path,
                    &moved_owners,
                    &scoped
                        .iter()
                        .map(|binding| binding.thread_id.clone())
                        .collect(),
                    &fallback_config,
                )?
                .0
            }
        } else {
            initial_conversation_thread_id(
                &resolved.binding.bundle_id,
                &provider,
                &provider_thread_id,
            )?
        };
        review.entries.push(ThreadSyncEntry {
            provider: provider.clone(),
            provider_display_name,
            access: ProviderAccessState::BoundReady,
            thread_id: provider_thread_id.clone(),
            conversation_resource_id: conversation_resource_id(&provider, &thread_id)?,
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
            push_decision: blocked.clone(),
            pull_decision: blocked,
            origin_label: None,
        });
    }

    // Remote provider scopes without a local binding stay visible as
    // view-only rows: mixed repositories remain fully visible from a
    // single-provider binding, and viewing never advances a base.
    if let Some(snapshot) = current {
        for (provider, scope) in snapshot.manifest.provider_scopes() {
            if resolved.binding.profile_ids.contains_key(provider) {
                continue;
            }
            let registered = provider_registry().contains(provider);
            let provider_display_name = provider_registry()
                .get(provider)
                .map(|adapter| adapter.descriptor().display_name)
                .unwrap_or_else(|| provider.to_string());
            let access = if registered {
                ProviderAccessState::ViewOnly
            } else {
                ProviderAccessState::Unsupported
            };
            let view_only = ThreadSyncDecision {
                action: ConversationReviewAction::Blocked,
                blocker: Some(ConversationBlocker::MissingBinding),
                selectable: false,
                blocked_reason: Some(if registered {
                    format!(
                        "View only — connect a {} profile to sync this session.",
                        provider_display_name
                    )
                } else {
                    format!(
                        "View only — this Mallard build has no '{}' provider.",
                        provider
                    )
                }),
            };
            for (thread_id, thread) in &scope.conversations {
                review.entries.push(ThreadSyncEntry {
                    provider: provider.clone(),
                    provider_display_name: provider_display_name.clone(),
                    access,
                    thread_id: thread
                        .source_thread_id
                        .clone()
                        .unwrap_or_else(|| thread_id.to_string()),
                    conversation_resource_id: conversation_resource_id(provider, thread_id)?,
                    resource_id: thread.resource_id.clone(),
                    display_name: thread
                        .display_title
                        .clone()
                        .unwrap_or_else(|| thread_id.to_string()),
                    state: ThreadSyncState::StorageOnly,
                    local_present: false,
                    storage_present: true,
                    status_detail: view_only.blocked_reason.clone(),
                    local_updated_at: None,
                    storage_updated_at: Some(thread.updated_at),
                    cloud_thread_id: Some(thread_id.clone()),
                    forked_from: thread.forked_from.clone(),
                    provider_thread_id: thread.source_thread_id.clone(),
                    push_decision: view_only.clone(),
                    pull_decision: view_only.clone(),
                    origin_label: None,
                });
            }
        }
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
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let mut counts = ThreadSyncCounts::default();
    for entry in &review.entries {
        count_thread_sync_state(&mut counts, entry.state);
    }
    for error in review.provider_errors.values() {
        warnings.push(error.clone());
    }
    warnings.sort();
    warnings.dedup();
    on_phase(
        "classify threads and build actions",
        phase_started.elapsed(),
    );

    let scopes = scope_sync_rows(link, resolved, current);
    Ok(ThreadComparisonBuild {
        comparison: ThreadSyncComparison {
            project_id: local_project_id.clone(),
            storage_id: storage_id.clone(),
            storage_name,
            revision: current.map(|snapshot| snapshot.head.revision),
            base_revision: repository_base_from_link(link).map(|base| base.revision),
            compared_at: created_at,
            entries: review.entries,
            counts,
            scopes,
            provider_errors: review.provider_errors,
            warnings,
            remote_continuity,
        },
        push_actions: review.push_actions,
    })
}

/// One continuity row per sync scope: the project scope plus every provider
/// scope present locally (bound) or remotely.
fn scope_sync_rows(
    link: &ProjectStorageLink,
    resolved: &ResolvedBinding,
    current: Option<&BundleSnapshot>,
) -> Vec<ScopeSyncRow> {
    let mut scope_keys = BTreeSet::new();
    scope_keys.insert(SyncScopeKey::Project);
    for provider in resolved.binding.profile_ids.keys() {
        scope_keys.insert(SyncScopeKey::Provider(provider.clone()));
    }
    if let Some(snapshot) = current {
        for key in snapshot.manifest.scopes.keys() {
            scope_keys.insert(key.clone());
        }
    }
    for key in link.scope_bases.keys() {
        scope_keys.insert(key.clone());
    }
    scope_keys
        .into_iter()
        .map(|key| {
            let remote = current
                .and_then(|snapshot| snapshot.manifest.scope(&key))
                .map(|scope| (scope.revision, scope.content_sha256.clone()));
            let base = link.scope_bases.get(&key);
            let continuity = classify_scope_continuity(
                remote
                    .as_ref()
                    .map(|(revision, digest)| (*revision, digest.as_str())),
                base,
            );
            let provider = key.provider_key().cloned();
            let access = match &provider {
                None => ProviderAccessState::BoundReady,
                Some(provider) if resolved.homes.contains_key(provider) => {
                    ProviderAccessState::BoundReady
                }
                Some(provider) if resolved.binding.profile_ids.contains_key(provider) => {
                    ProviderAccessState::BoundUnavailable
                }
                Some(provider) if provider_registry().contains(provider) => {
                    ProviderAccessState::ViewOnly
                }
                Some(_) => ProviderAccessState::Unsupported,
            };
            ScopeSyncRow {
                scope: key.canonical(),
                provider,
                access,
                remote_revision: remote.as_ref().map(|(revision, _)| *revision),
                base_revision: base.map(|base| base.scope_revision),
                continuity,
            }
        })
        .collect()
}
fn get_bundle_status_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<ResourceStatusReport, String> {
    let config = repository.load_config()?;
    let project = config
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
    let discovery = provider_capture::discover_full(provider_registry(), &capture_request)?;
    let mut effective_recipe = destination_recipe.unwrap_or_else(|| project.recipe.clone());
    effective_recipe.validate_machine_local()?;
    remove_excluded_codex_subagents_from_recipe(
        &mut effective_recipe,
        discovery.excluded_subagent_resource_ids(),
    );
    let capture = provider_capture::capture_recipe_from_discovery(
        provider_registry(),
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
    let revision = remote.as_ref().map(|snapshot| snapshot.head.revision);
    let remote_resources = remote.as_ref().map(|snapshot| {
        snapshot
            .manifest
            .all_resources()
            .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
            .collect::<BTreeMap<_, _>>()
    });
    let mut statuses = Vec::with_capacity(effective_recipe.entries.len());
    for resource_id in effective_recipe.entries.keys() {
        let local_digest = local_resources
            .get(resource_id)
            .and_then(|resource| resource.metadata.get("content_sha256"))
            .cloned();
        let remote_digest = remote_resources
            .as_ref()
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
        revision,
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
        revision: Option<u64>,
    },
    RemoteHistoryMissing {
        base_revision: u64,
    },
    /// The remote holds a history that does not contain this replica's base:
    /// the repository was wiped and recreated, or otherwise reset. Blocks
    /// Push like `RemoteHistoryMissing`; the remedy is a pull (which adopts
    /// the new history and rewrites the base) or an unlink-and-relink.
    RemoteHistoryReplaced {
        base_revision: u64,
        remote_revision: u64,
        base_manifest_sha256: String,
        remote_manifest_sha256: String,
    },
    RequiresPull {
        reason: PushDestinationPullReason,
        remote_revision: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_revision: Option<u64>,
        /// The writable scopes that actually block this Push. Unbound and
        /// unknown provider scopes never appear here.
        #[serde(default)]
        blocking_scopes: Vec<String>,
    },
    Unavailable {
        message: String,
    },
}

/// The sync scopes a Push from this binding may write: the project scope
/// plus one scope per bound provider.
fn writable_scopes_for_binding(binding: &ProjectBinding) -> BTreeSet<SyncScopeKey> {
    let mut scopes = BTreeSet::new();
    scopes.insert(SyncScopeKey::Project);
    for provider in binding.profile_ids.keys() {
        scopes.insert(SyncScopeKey::Provider(provider.clone()));
    }
    scopes
}

/// Push-side destination status, decided per writable scope so unrelated
/// provider changes never block each other (D6): repository identity and
/// global history continuity remain whole-operation blockers, then each
/// writable scope compares its reviewed base with the verified current
/// manifest. A remote change in an unbound or unknown provider scope is
/// carried, not blocking.
fn classify_push_destination(
    current: Option<&BundleSnapshot>,
    link: &ProjectStorageLink,
    binding: &ProjectBinding,
    probe: &mut dyn FnMut(&RepositoryBase) -> Result<bool, String>,
) -> PushDestinationStatus {
    let observed_head = current.map(|snapshot| &snapshot.head);
    let repository_base = repository_base_from_link(link);
    let continuity = classify_remote_continuity(observed_head, repository_base.as_ref(), probe);
    match &continuity {
        RemoteContinuity::RemoteMissing { base_revision } => {
            return PushDestinationStatus::RemoteHistoryMissing {
                base_revision: *base_revision,
            };
        }
        RemoteContinuity::Replaced {
            base_revision,
            remote_revision,
        } => {
            return PushDestinationStatus::RemoteHistoryReplaced {
                base_revision: *base_revision,
                remote_revision: *remote_revision,
                base_manifest_sha256: repository_base
                    .map(|base| base.manifest_sha256)
                    .unwrap_or_default(),
                remote_manifest_sha256: observed_head
                    .map(|head| head.manifest_sha256.clone())
                    .unwrap_or_default(),
            };
        }
        RemoteContinuity::Unknown { message } => {
            return PushDestinationStatus::Unavailable {
                message: message.clone(),
            };
        }
        RemoteContinuity::Fresh
        | RemoteContinuity::Unreviewed { .. }
        | RemoteContinuity::InSync { .. }
        | RemoteContinuity::Advanced { .. } => {}
    }
    let remote_revision = observed_head.map(|head| head.revision);
    // The binding-revision overlay is push policy, not continuity: a base
    // reviewed under an older binding cannot authorize this Push.
    let stale_binding_scopes = writable_scopes_for_binding(binding)
        .into_iter()
        .filter(|scope| {
            link.scope_bases
                .get(scope)
                .is_some_and(|base| base.binding_revision != binding.revision)
        })
        .map(|scope| scope.canonical())
        .collect::<Vec<_>>();
    if !stale_binding_scopes.is_empty() {
        return PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::BindingChanged,
            remote_revision: remote_revision.unwrap_or(0),
            base_revision: repository_base_from_link(link).map(|base| base.revision),
            blocking_scopes: stale_binding_scopes,
        };
    }
    let mut unreviewed_scopes = Vec::new();
    let mut advanced_scopes = Vec::new();
    for scope in writable_scopes_for_binding(binding) {
        let remote = current
            .and_then(|snapshot| snapshot.manifest.scope(&scope))
            .map(|remote| (remote.revision, remote.content_sha256.clone()));
        let base = link.scope_bases.get(&scope);
        match classify_scope_continuity(
            remote
                .as_ref()
                .map(|(revision, digest)| (*revision, digest.as_str())),
            base,
        ) {
            ScopeContinuity::Fresh | ScopeContinuity::InSync { .. } => {}
            // The scope disappeared remotely while global lineage held: a
            // pull re-reviews before this replica may republish it.
            ScopeContinuity::RemoteMissing { .. } => advanced_scopes.push(scope.canonical()),
            ScopeContinuity::Unreviewed { .. } => unreviewed_scopes.push(scope.canonical()),
            ScopeContinuity::Advanced { .. } | ScopeContinuity::Replaced { .. } => {
                advanced_scopes.push(scope.canonical())
            }
        }
    }
    if !advanced_scopes.is_empty() {
        return PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteAdvanced,
            remote_revision: remote_revision.unwrap_or(0),
            base_revision: repository_base_from_link(link).map(|base| base.revision),
            blocking_scopes: advanced_scopes,
        };
    }
    if !unreviewed_scopes.is_empty() {
        return PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteWithoutBase,
            remote_revision: remote_revision.unwrap_or(0),
            base_revision: None,
            blocking_scopes: unreviewed_scopes,
        };
    }
    PushDestinationStatus::Ready {
        revision: remote_revision,
    }
}

/// Head-end of a sha256 for log and error lines; full digests stay in the
/// serialized status for the UI.
fn short_digest(digest: &str) -> &str {
    digest.get(..8).unwrap_or(digest)
}

/// Store-read continuity probe over a storage engine. Present with the
/// recorded digest ⇒ `Ok(true)`; absent or different bytes ⇒ `Ok(false)`;
/// transport failure ⇒ `Err`, which classifies as `Unknown`. Reads the
/// store, never a cache — a local copy witnesses nothing about the remote.
fn continuity_probe<'a>(
    engine: &'a StorageEngine,
    bundle_id: &'a BundleId,
) -> impl FnMut(&RepositoryBase) -> Result<bool, String> + 'a {
    move |base: &RepositoryBase| {
        engine
            .fetch_manifest_version_optional(
                bundle_id,
                base.revision,
                &base.commit_id,
                base.manifest_size,
                &base.manifest_sha256,
            )
            .map(|manifest| manifest.is_some())
    }
}

fn push_destination_error(status: &PushDestinationStatus) -> Option<String> {
    match status {
        PushDestinationStatus::Ready { .. } => None,
        PushDestinationStatus::RemoteHistoryMissing { .. } => Some(
            "remote history is missing at the configured storage; if this is expected, unlink and relink the storage before pushing again"
                .to_string(),
        ),
        PushDestinationStatus::RemoteHistoryReplaced {
            base_revision,
            remote_revision,
            base_manifest_sha256,
            remote_manifest_sha256,
        } => Some(format!(
            "remote history at this storage was replaced: remote revision {} (manifest {}) does not contain this replica's base revision {} (manifest {}); pull and review to adopt the new history, or unlink and relink the storage",
            remote_revision,
            short_digest(remote_manifest_sha256),
            base_revision,
            short_digest(base_manifest_sha256),
        )),
        // These reach the user verbatim in the Push error banner when the head
        // moves between the review's classification and the publish re-check,
        // so they lead with what happened and what to do; the lineage numbers
        // stay at the end for support and log greps.
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteWithoutBase,
            ..
        } => Some(
            "this storage already holds data for this project that has never been reviewed on this computer; pull and review it before pushing"
                .to_string(),
        ),
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::BindingChanged,
            ..
        } => Some(
            "the project folder or an agent profile changed since this computer last synced with this storage; pull and review before pushing"
                .to_string(),
        ),
        PushDestinationStatus::RequiresPull {
            reason: PushDestinationPullReason::RemoteAdvanced,
            remote_revision,
            base_revision,
            blocking_scopes,
        } => Some(format!(
            "this storage has changes saved after this computer last synced ({}); pull and review them before pushing (base revision {}, remote revision {})",
            blocking_scopes.join(", "),
            base_revision
                .map(|revision| revision.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            remote_revision,
        )),
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

#[cfg(test)]
fn prepare_push_review_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Result<PushReviewContext, String> {
    prepare_push_review_with_progress(repository, local_project_id, storage_id, &mut |_, _| {})
}

pub(super) fn prepare_push_review_with_progress(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<PushReviewContext, String> {
    let phase_started = Instant::now();
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
    let resolved = resolve_project_binding(repository, &binding)?;

    on_phase("load config and binding", phase_started.elapsed());
    let phase_started = Instant::now();

    // Project content stays out of the dialog-open walk: eligible (non-Git)
    // projects defer it to the explicit Scan, and Git-managed projects get a
    // storage-side inventory below without any local content walk.
    let eligibility = project_file_sync_eligibility(&binding);
    let capture_request = capture_request_for_binding(repository, &binding)?;
    let (discovery, views) = super::codex_watch_service::session_watch_service().discovery_for(
        provider_registry(),
        local_project_id,
        &capture_request,
    )?;
    on_phase("discover local state", phase_started.elapsed());
    let phase_started = Instant::now();

    let inventory = build_resource_inventory(repository, local_project_id, &binding, &discovery)?;

    on_phase("build resource inventory", phase_started.elapsed());
    let phase_started = Instant::now();

    let remote = storage_engine(repository, storage_id).and_then(|(_, engine)| {
        let current = engine.inspect_optional(&project.bundle_id)?;
        Ok((engine, current))
    });
    // Continuity is classified once per operation — at most one probe read —
    // then the destination status maps it push-side and the thread
    // comparison carries it verbatim.
    let (destination_status, remote_continuity) = match &remote {
        Ok((engine, current)) => {
            let observed_head = current.as_ref().map(|snapshot| &snapshot.head);
            let continuity = classify_remote_continuity(
                observed_head,
                repository_base_from_link(&link).as_ref(),
                &mut continuity_probe(engine, &project.bundle_id),
            );
            let destination = classify_push_destination(
                current.as_ref(),
                &link,
                &binding,
                &mut continuity_probe(engine, &project.bundle_id),
            );
            (destination, Some(continuity))
        }
        Err(error) => (
            PushDestinationStatus::Unavailable {
                message: error.clone(),
            },
            None,
        ),
    };
    on_phase("inspect remote head", phase_started.elapsed());

    // Every bound provider participates in the typed thread comparison.
    let wants_thread_comparison = !binding.profile_ids.is_empty();
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
                let phase_started = Instant::now();
                match build_thread_sync_comparison(
                    repository,
                    local_project_id,
                    storage_id,
                    storage_name.clone(),
                    &link,
                    &resolved,
                    &capture_request,
                    &discovery,
                    &views,
                    engine,
                    current.as_ref(),
                    true,
                    remote_continuity
                        .clone()
                        .expect("continuity is classified whenever the storage engine opened"),
                    &mut *on_phase,
                ) {
                    Ok(build) => {
                        let compared_at = build.comparison.compared_at;
                        let identity = repository.load_or_create_host_identity()?;
                        let bindings_document = repository.load_conversation_bindings()?;
                        let (revision, commit_id, manifest_sha256) = current
                            .as_ref()
                            .map(|snapshot| {
                                (
                                    snapshot.head.revision,
                                    snapshot.head.commit_id.clone(),
                                    snapshot.head.manifest_sha256.clone(),
                                )
                            })
                            .unwrap_or_else(absent_bundle_head_pin);
                        let plan_id = PlanId::parse(generated_named_id("plan")?)?;
                        let plan = ConversationPushPlan {
                            schema_version: PUSH_PLAN_SCHEMA,
                            plan_id: plan_id.clone(),
                            storage_id: storage_id.clone(),
                            bundle_id: project.bundle_id.clone(),
                            host_id: binding.host_id.clone(),
                            replica_id: binding.replica_id.clone(),
                            local_project_id: binding.local_project_id.clone(),
                            expected_host_identity_revision: identity.revision,
                            expected_project_binding_revision: binding.revision,
                            expected_bindings_document_revision: bindings_document.revision,
                            revision,
                            commit_id,
                            manifest_sha256,
                            created_at: compared_at,
                            expires_at: compared_at.saturating_add(PLAN_LIFETIME_SECS),
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
                        thread_comparison = Some(build.comparison);
                    }
                    Err(error) => thread_comparison_error = Some(error),
                }
                on_phase("thread comparison and push plan", phase_started.elapsed());
            }
            if wants_project_files {
                let phase_started = Instant::now();
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
                on_phase("project files inventory", phase_started.elapsed());
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
    let link = project_link(&config, local_project_id, storage_id)
        .cloned()
        .ok_or_else(|| "project is not linked to this storage".to_string())?;
    let (_, engine) = storage_engine(repository, storage_id)?;
    let current = engine.inspect_optional(&project.bundle_id)?;
    let status = classify_push_destination(
        current.as_ref(),
        &link,
        &binding,
        &mut continuity_probe(&engine, &project.bundle_id),
    );
    Ok(status)
}

#[tauri::command]
pub async fn prepare_push_review(
    app: tauri::AppHandle,
    local_project_id: LocalProjectId,
    storage_id: StorageId,
) -> Result<PushReviewContext, String> {
    let repository = repository(&app)?;
    let (project, storage) =
        project_storage_log_labels(&repository, &local_project_id, &storage_id);
    let log = ActivityLogScope::new(ActivityLogType::Push)
        .project(&local_project_id, Some(&project))
        .storage(&storage_id, Some(&storage));
    log.emit(
        &app,
        "info",
        "push.review_started",
        &format!("↑  Preparing Push review — {} → {}", project, storage),
    );
    let phase_app = app.clone();
    let phase_log = log.clone();
    let total_started = Instant::now();
    let result = run_blocking(move || {
        ProjectSyncService::new(&repository).prepare_push_review_with_phases(
            &local_project_id,
            &storage_id,
            &mut |phase, elapsed| {
                phase_log.emit(
                    &phase_app,
                    "info",
                    "push.phase_timing",
                    &format!(
                        "   Push review phase — {phase}: {:.3}s",
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
        "push.review_timing",
        &format!(
            "   Push review total: {:.3}s",
            total_started.elapsed().as_secs_f64()
        ),
    );
    match &result {
        Ok(context) => {
            let threads = context
                .thread_comparison
                .as_ref()
                .map(|comparison| comparison.entries.len())
                .unwrap_or(0);
            log.emit(
                &app,
                "ok",
                "push.review_ready",
                &format!(
                    "✓  Push review ready — {} threads compared, {} pushable conversations",
                    threads,
                    context.conversation_action_ids.len()
                ),
            );
        }
        Err(error) => log.emit(
            &app,
            "error",
            "push.review_failed",
            &format!("✗  Push review failed: {}", error),
        ),
    }
    result
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
    let review = prepare_push_review_with_repository(repository, local_project_id, storage_id)?;
    let recipe = recommended_review_recipe(&review, storage_id);
    push_reviewed_test_context(repository, local_project_id, storage_id, review, recipe)
}

#[cfg(test)]
fn push_bundle_with_recipe_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: BundleRecipe,
) -> Result<ProjectOperationResult, String> {
    let review = prepare_push_review_with_repository(repository, local_project_id, storage_id)?;
    push_reviewed_test_context(
        repository,
        local_project_id,
        storage_id,
        review,
        requested_recipe,
    )
}

pub(super) fn recommended_review_recipe(
    review: &PushReviewContext,
    storage_id: &StorageId,
) -> BundleRecipe {
    let mut recipe = review
        .project
        .links
        .iter()
        .find(|link| link.storage_id == *storage_id)
        .and_then(|link| link.recipe.clone())
        .unwrap_or_else(|| review.inventory.recipe.clone());
    let Some(comparison) = &review.thread_comparison else {
        return recipe;
    };
    for entry in comparison
        .entries
        .iter()
        .filter(|entry| entry.push_decision.selectable)
    {
        let Some(resource) = review
            .inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.resource_id == entry.resource_id)
        else {
            continue;
        };
        recipe.entries.insert(
            entry.resource_id.clone(),
            RecipeEntry {
                resource_id: entry.resource_id.clone(),
                apply_policy: resource.descriptor.apply_policy,
                required: false,
            },
        );
    }
    if comparison
        .entries
        .iter()
        .any(|entry| recipe.entries.contains_key(&entry.resource_id))
    {
        if let Some(resource) = review
            .inventory
            .resources
            .iter()
            .find(|resource| resource.descriptor.resource_id.as_str() == "codex:session-index")
        {
            recipe.entries.insert(
                resource.descriptor.resource_id.clone(),
                RecipeEntry {
                    resource_id: resource.descriptor.resource_id.clone(),
                    apply_policy: resource.descriptor.apply_policy,
                    required: false,
                },
            );
        }
    }
    recipe
}

#[cfg(test)]
fn push_reviewed_test_context(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    review: PushReviewContext,
    requested_recipe: BundleRecipe,
) -> Result<ProjectOperationResult, String> {
    let selected_resources = requested_recipe
        .entries
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let approved_conversation_action_ids = review
        .thread_comparison
        .as_ref()
        .into_iter()
        .flat_map(|comparison| comparison.entries.iter())
        .filter(|entry| {
            entry.push_decision.selectable && selected_resources.contains(&entry.resource_id)
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
            ConversationPlanActionType::Publish | ConversationPlanActionType::CreateCloud
        )
    }) {
        return Err(format!(
            "conversation Push action '{}' is not pushable",
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

#[allow(clippy::too_many_arguments)]
fn conversation_thread_from_capture(
    previous: Option<&ConversationThread>,
    action: &ConversationPlanAction,
    result_thread_id: &ThreadId,
    local_binding: Option<&LocalConversationBinding>,
    descriptor: &ResourceDescriptor,
    conversation: &provider_capture::CapturedConversation,
    artifact: Option<&StandaloneConversationArtifact>,
    published_bytes: &[u8],
    published_by: &PortableOrigin,
    updated_at: u64,
) -> Result<(ConversationThread, ResourceDescriptor, LogicalPath), String> {
    let resource_id = conversation_resource_id(&action.provider, result_thread_id)?;
    let logical_path = conversation_transcript_path(&action.provider, result_thread_id)?;
    // Codex publishes the complete flattened standalone graph: identity,
    // counts, digests, and the portable tail all come from the verified
    // artifact, never from the leaf-only capture.
    let (lineage_status, head_message_id, message_count, lineage_digest, semantic_digest) =
        match artifact {
            Some(artifact) => (
                ConversationLineageStatus::Complete,
                None,
                artifact.message_count,
                artifact.lineage_digest.clone(),
                artifact.semantic_digest.clone(),
            ),
            None => {
                // Providers without a standalone artifact still publish from
                // their directly captured conversation chain.
                let CodecLineage::Complete {
                    chain,
                    lineage_digest,
                    semantic_digest,
                } = &conversation.lineage
                else {
                    return Err(format!(
                        "conversation '{}' has unproven lineage and cannot be published",
                        result_thread_id
                    ));
                };
                (
                    ConversationLineageStatus::Complete,
                    chain.last().map(|message| message.message_id.clone()),
                    chain.len() as u64,
                    lineage_digest.clone(),
                    semantic_digest.clone(),
                )
            }
        };
    let head_message_id = match (artifact, head_message_id) {
        (Some(_), _) => {
            // The artifact's head is the last canonical message of the
            // flattened chain; re-derive it from the analyzed output rather
            // than the leaf capture.
            let analysis = analyze_codex_transcript(published_bytes);
            match analysis.lineage {
                CodecLineage::Complete { chain, .. } => {
                    chain.last().map(|message| message.message_id.clone())
                }
                CodecLineage::Unknown { .. } => None,
            }
        }
        (None, head) => head,
    };
    let forked_from = match action.action {
        ConversationPlanActionType::CreateCloud => {
            local_binding.and_then(|binding| binding.forked_from.clone())
        }
        _ => previous.and_then(|thread| thread.forked_from.clone()),
    };
    // Preservation provenance travels exactly like fork provenance: a
    // preserved copy's first push is CreateCloud (the local binding is the
    // only source); later pushes carry the manifest record forward.
    let preserved = match action.action {
        ConversationPlanActionType::CreateCloud => {
            local_binding.and_then(|binding| binding.preserved.clone())
        }
        _ => previous.and_then(|thread| thread.preserved.clone()),
    };
    let sidecar_refs = conversation
        .sidecar_files
        .iter()
        .map(|sidecar| {
            mallard_domain::conversation_sidecar_path(
                &action.provider,
                result_thread_id,
                &sidecar.sidecar_kind,
                &sidecar.relative_path,
            )
            .map(|logical_path| ConversationFileRef {
                logical_path,
                role: ConversationFileRole::Sidecar {
                    sidecar_kind: sidecar.sidecar_kind.clone(),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Explicit provider titles win. Preserve an existing portable title when
    // the local index no longer contains it, then derive a bounded fallback
    // from the reconstructed logical root. The reconstructed bytes include
    // history_base ancestors, so an empty paginated leaf inherits the same
    // first-user title as its complete conversation.
    let display_title = Some(descriptor.display_name.clone())
        .filter(|name| Some(name.as_str()) != conversation.source_thread_id.as_deref())
        .or_else(|| previous.and_then(|thread| thread.display_title.clone()))
        .or_else(|| artifact.and_then(|_| first_codex_user_summary(published_bytes)));
    let thread = ConversationThread {
        thread_id: result_thread_id.clone(),
        provider: action.provider.clone(),
        display_title,
        presentation: conversation.presentation.clone(),
        resource_id: resource_id.clone(),
        files: std::iter::once(ConversationFileRef {
            logical_path: logical_path.clone(),
            role: ConversationFileRole::Transcript,
        })
        .chain(sidecar_refs)
        .collect(),
        lineage_status,
        lineage_codec: conversation.lineage_codec.clone(),
        head_message_id,
        message_count,
        lineage_digest,
        semantic_digest,
        portable_tail: artifact.map(|artifact| artifact.portable_tail.clone()),
        forked_from,
        preserved,
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
    portable_descriptor.kind = ResourceKind::Conversation;
    portable_descriptor.provider = Some(action.provider.clone());
    portable_descriptor.apply_policy = ApplyPolicy::SafeFile;
    portable_descriptor.metadata.insert(
        "content_sha256".to_string(),
        hex_digest(Sha256::digest(published_bytes).as_slice()),
    );
    Ok((thread, portable_descriptor, logical_path))
}

/// Split one flat reviewed recipe into per-scope recipes by resource
/// ownership: project content and requirements to the project scope,
/// provider-owned resources to their provider's scope. Conversation entries
/// are appended later from the reviewed conversation set.
fn split_recipe_by_scope(
    recipe: &BundleRecipe,
    captured_descriptors: &BTreeMap<ResourceId, ResourceDescriptor>,
    previous: Option<&BundleManifest>,
) -> Result<BTreeMap<SyncScopeKey, BundleRecipe>, String> {
    let mut by_scope: BTreeMap<SyncScopeKey, BundleRecipe> = BTreeMap::new();
    for (resource_id, entry) in &recipe.entries {
        let provider = captured_descriptors
            .get(resource_id)
            .map(|descriptor| descriptor.provider.clone())
            .or_else(|| {
                previous.and_then(|manifest| {
                    manifest
                        .find_resource(resource_id)
                        .map(|(_, descriptor)| descriptor.provider.clone())
                })
            })
            .ok_or_else(|| {
                format!(
                    "selected resource '{}' is unavailable and has no previous captured version",
                    resource_id
                )
            })?;
        let scope = match provider {
            Some(provider) => SyncScopeKey::Provider(provider),
            None => SyncScopeKey::Project,
        };
        let scoped = by_scope.entry(scope).or_insert_with(|| BundleRecipe {
            schema_version: recipe.schema_version,
            revision: recipe.revision,
            entries: BTreeMap::new(),
        });
        scoped.entries.insert(resource_id.clone(), entry.clone());
    }
    Ok(by_scope)
}

/// Pull seeds machine-local selection from generic resources only and resets
/// its local concurrency revision. Reuse the published scope recipe revision
/// when that generic selection is unchanged so a converged Pull cannot create
/// a recipe-metadata-only remote revision on the next Push.
fn preserve_unchanged_scope_recipe_revision(
    recipe: &mut BundleRecipe,
    previous: Option<&BundleRecipe>,
) {
    let Some(previous) = previous else {
        return;
    };
    let previous_generic_entries = previous
        .entries
        .iter()
        .filter(|(resource_id, _)| parse_conversation_resource_id(resource_id.as_str()).is_err())
        .map(|(resource_id, entry)| (resource_id.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    if recipe.schema_version == previous.schema_version
        && recipe.entries == previous_generic_entries
    {
        recipe.revision = previous.revision;
    }
}

fn captured_with_for_provider(provider: &ProviderKey) -> CapturedWith {
    let mut captured_with = CapturedWith {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        provider_versions: BTreeMap::new(),
        codec_versions: BTreeMap::new(),
    };
    if let Some(adapter) = provider_registry().sessions(provider) {
        captured_with
            .codec_versions
            .insert(adapter.codec_key().to_string(), adapter.codec_version());
    }
    captured_with
}

/// Build a scoped publish request from the reviewed flat selection. Only the
/// binding's writable scopes appear; every other scope — unbound providers
/// and unknown future providers alike — is carried verbatim by the engine's
/// mutation firewall.
#[allow(clippy::too_many_arguments)]
fn build_publish_request(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    storage_id: &StorageId,
    project: &LocalProjectRegistration,
    binding: &ProjectBinding,
    homes: &BTreeMap<ProviderKey, String>,
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
        shared_label: repository
            .load_or_create_host_identity()?
            .local_display_name
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string),
    };
    let previous_manifest = previous.map(|snapshot| &snapshot.manifest);
    let captured_descriptors = provider_capture::domain_resources(captured)?;
    // Conversation resources never travel through the generic file path:
    // strip them from the flat selection before scope reconciliation and add
    // the reviewed conversation set per provider scope below.
    let conversation_capture_ids = captured
        .conversations
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut generic_recipe = recipe.clone();
    generic_recipe
        .entries
        .retain(|resource_id, _| !conversation_capture_ids.contains(resource_id.as_str()));
    if let Some(previous_manifest) = previous_manifest {
        let previous_conversation_resources = previous_manifest
            .all_conversations()
            .map(|(_, _, thread)| thread.resource_id.clone())
            .collect::<BTreeSet<_>>();
        generic_recipe
            .entries
            .retain(|resource_id, _| !previous_conversation_resources.contains(resource_id));
    }
    let generic_descriptors = captured_descriptors
        .iter()
        .filter(|(resource_id, _)| generic_recipe.entries.contains_key(*resource_id))
        .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
        .collect::<BTreeMap<_, _>>();
    let recipes_by_scope =
        split_recipe_by_scope(&generic_recipe, &generic_descriptors, previous_manifest)?;

    let document = repository.load_conversation_bindings()?;
    let domain_captured = &captured_descriptors;
    let mut writable_scopes: BTreeMap<SyncScopeKey, ScopePublishInput> = BTreeMap::new();
    let mut reviewed_scope_pins: BTreeMap<SyncScopeKey, (u64, String)> = BTreeMap::new();
    for scope_key in writable_scopes_for_binding(binding) {
        let previous_scope =
            previous_manifest.and_then(|manifest| manifest.scope(&scope_key).cloned());
        reviewed_scope_pins.insert(
            scope_key.clone(),
            previous_scope
                .as_ref()
                .map(|scope| (scope.revision, scope.content_sha256.clone()))
                .unwrap_or_else(absent_scope_pin),
        );
        let mut scope_recipe = recipes_by_scope
            .get(&scope_key)
            .cloned()
            .unwrap_or_else(|| BundleRecipe {
                schema_version: recipe.schema_version,
                revision: recipe.revision,
                entries: BTreeMap::new(),
            });
        preserve_unchanged_scope_recipe_revision(
            &mut scope_recipe,
            previous_scope.as_ref().map(|scope| &scope.recipe),
        );
        let scope_descriptors = generic_descriptors
            .iter()
            .filter(|(resource_id, _)| scope_recipe.entries.contains_key(*resource_id))
            .map(|(resource_id, descriptor)| (resource_id.clone(), descriptor.clone()))
            .collect::<BTreeMap<_, _>>();
        let (mut resources, mut retained_files, mut directories, mut tombstones) =
            super::bundle_engine::reconcile_scope_content(
                &scope_recipe,
                &scope_descriptors,
                captured,
                previous_scope.as_ref(),
                updated_at,
            )?;
        let mut captured_files = BTreeMap::new();
        for captured_file in captured.files.values() {
            if !scope_recipe
                .entries
                .contains_key(&ResourceId::parse(captured_file.resource_id.clone())?)
            {
                continue;
            }
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
        for captured_directory in captured.directories.values() {
            if !scope_recipe
                .entries
                .contains_key(&ResourceId::parse(captured_directory.resource_id.clone())?)
            {
                continue;
            }
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
        let conversations = previous_scope
            .as_ref()
            .map(|scope| scope.conversations.clone())
            .unwrap_or_default();
        let captured_with = match &scope_key {
            SyncScopeKey::Provider(provider) => captured_with_for_provider(provider),
            SyncScopeKey::Project => CapturedWith {
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                provider_versions: BTreeMap::new(),
                codec_versions: BTreeMap::new(),
            },
        };
        // Carry previous conversation content: resources, files, and
        // conversation tombstones stay unless an approved action changes
        // them below.
        if let Some(previous_scope) = &previous_scope {
            for thread in previous_scope.conversations.values() {
                if let Some(descriptor) = previous_scope.resources.get(&thread.resource_id) {
                    resources.insert(thread.resource_id.clone(), descriptor.clone());
                }
                for file in &thread.files {
                    if let Some(entry) = previous_scope.files.get(&file.logical_path) {
                        retained_files.insert(file.logical_path.clone(), entry.clone());
                    }
                }
            }
            for (key, tombstone) in &previous_scope.tombstones {
                if matches!(tombstone.target, TombstoneTarget::ConversationThread { .. }) {
                    tombstones.insert(key.clone(), tombstone.clone());
                }
            }
        }
        writable_scopes.insert(
            scope_key,
            ScopePublishInput {
                recipe: scope_recipe,
                captured_with,
                conversations,
                resources,
                retained_files,
                captured_files,
                directories,
                tombstones,
            },
        );
    }

    // Apply the reviewed conversation actions inside their provider scopes.
    let reviewed_config = repository.load_config()?;
    let mut operations = Vec::new();
    let mut planned_checkpoints: BTreeMap<ConversationOperationId, ConversationCheckpoint> =
        BTreeMap::new();
    for action in actions {
        let scope_key = SyncScopeKey::Provider(action.provider.clone());
        let scope = writable_scopes.get_mut(&scope_key).ok_or_else(|| {
            format!(
                "conversation Push action '{}' targets unbound provider '{}'",
                action.action_id, action.provider
            )
        })?;
        let (conversation, transcript) = captured_conversation_for_action(captured, action)?;
        let result_thread_id = action.thread_id.clone();
        let previous_thread = scope.conversations.get(&result_thread_id).cloned();
        let local_binding = document.bindings.iter().find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.pins.context.provider_profile_id
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
        // Publishing apply is the single reconstruction point: the complete
        // logical graph becomes one verified standalone artifact whose
        // cumulative tail must equal the planned local value.
        let is_codex = action.provider.as_str() == "codex";
        let (artifact, seeded) = if is_codex {
            // Operation-level Windows gate: the path-map root grammar is
            // checked before any rollout bytes are parsed.
            validate_pathmap_root("project root", &binding.project_root)?;
            let provider_home = homes.get(&action.provider).ok_or_else(|| {
                format!(
                    "conversation Push action '{}' has no provider home",
                    action.action_id
                )
            })?;
            let source = captured_transcript_source(conversation)?;
            let lookup = codex_rollout_lookup(Path::new(provider_home));
            let resolved_root = codex_history_graph::resolve_codex_root(&source, &lookup)?;
            let leaf_bytes = codex_history_graph::read_rollout_bounded(&source)?;
            let stored_layout = local_binding.and_then(|binding| binding.root_tail_layout.as_ref());
            let seeded =
                codex_history_graph::seed_codex_root(&resolved_root, &leaf_bytes, stored_layout)?;
            if Some(&seeded.observed_tail) != action.pins.tail.local.as_ref() {
                return Err(format!(
                    "conversation '{}' changed after review; refresh and push again",
                    result_thread_id
                ));
            }
            // The stale check is pin-vs-checkpoint equality: any checkpoint
            // advance between review and apply is a typed stale error, so
            // the witness read below never trusts state the review did not
            // pin.
            let stored_checkpoint = reviewed_config
                .links
                .iter()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && &link.storage_id == storage_id
                })
                .and_then(|link| link.conversation_checkpoints.get(&scope_key))
                .and_then(|threads| threads.get(&action.thread_id));
            if stored_checkpoint.map(|checkpoint| checkpoint.generation)
                != action.pins.tail.checkpoint_generation
            {
                return Err(format!(
                    "conversation '{}' checkpoint advanced after review; refresh and push again",
                    result_thread_id
                ));
            }
            // The prefix witness is the append-only trust anchor for the
            // fixed partition: the reviewed checkpoint's local-form witness
            // must survive this full local reconstruction bit for bit, or
            // content before the accepted start was mutated in place —
            // invisible to cursor-riding review, and never publishable.
            let stored_witness =
                stored_checkpoint.and_then(|checkpoint| checkpoint.prefix_witness.clone());
            if let Some(stored) = stored_witness {
                if seeded.prefix_witness.as_ref() != Some(&stored) {
                    return Err(format!(
                        "conversation '{}' content before its fixed start changed; \
                         the prefix witness no longer matches",
                        result_thread_id
                    ));
                }
            }
            let artifact = build_standalone_codex_root(
                &resolved_root,
                &StandaloneBuildMode::Encode {
                    root: binding.project_root.clone(),
                    canonical_root: (binding.canonical_project_root != binding.project_root)
                        .then(|| binding.canonical_project_root.clone()),
                },
                stored_layout,
            )?;
            (Some(artifact), Some(seeded))
        } else {
            (None, None)
        };
        let published_bytes: Vec<u8> = artifact
            .as_ref()
            .map(|artifact| artifact.root_bytes.clone())
            .unwrap_or_else(|| transcript.bytes.clone());
        let (thread, descriptor, logical_path) = conversation_thread_from_capture(
            previous_thread.as_ref(),
            action,
            &result_thread_id,
            local_binding,
            descriptor,
            conversation,
            artifact.as_ref(),
            &published_bytes,
            &published_by,
            updated_at,
        )?;
        if let Some(old_thread) = scope.conversations.insert(result_thread_id.clone(), thread) {
            for file in old_thread.files {
                scope.retained_files.remove(&file.logical_path);
                scope.captured_files.remove(&file.logical_path);
            }
            scope.resources.remove(&old_thread.resource_id);
        }
        let resource_id = descriptor.resource_id.clone();
        scope.resources.insert(resource_id.clone(), descriptor);
        let digest = hex_digest(Sha256::digest(&published_bytes).as_slice());
        let retained_entry = previous_manifest
            .and_then(|manifest| manifest.scope(&scope_key))
            .and_then(|previous_scope| previous_scope.files.get(&logical_path));
        let can_retain = retained_entry.is_some_and(|entry| {
            entry.resource_id == resource_id
                && entry.sha256 == digest
                && entry.size == published_bytes.len() as u64
        });
        if can_retain {
            scope
                .retained_files
                .insert(logical_path.clone(), retained_entry.unwrap().clone());
        } else {
            scope.retained_files.remove(&logical_path);
            scope.captured_files.insert(
                logical_path.clone(),
                CapturedBundleFile {
                    resource_id: resource_id.clone(),
                    bytes: published_bytes.clone(),
                    source_mtime: transcript.source_mtime,
                    mode: Some(transcript.mode),
                },
            );
        }
        // Sidecars travel with the conversation resource.
        for sidecar in &conversation.sidecar_files {
            let Some(sidecar_file) = captured.files.get(&sidecar.logical_path) else {
                return Err(format!(
                    "captured conversation '{}' lost sidecar '{}'",
                    resource_id, sidecar.logical_path
                ));
            };
            let sidecar_logical = mallard_domain::conversation_sidecar_path(
                &action.provider,
                &result_thread_id,
                &sidecar.sidecar_kind,
                &sidecar.relative_path,
            )?;
            scope.captured_files.insert(
                sidecar_logical,
                CapturedBundleFile {
                    resource_id: resource_id.clone(),
                    bytes: sidecar_file.bytes.clone(),
                    source_mtime: sidecar_file.source_mtime,
                    mode: Some(sidecar_file.mode),
                },
            );
        }
        scope.tombstones.retain(|_, tombstone| {
            !matches!(
                &tombstone.target,
                TombstoneTarget::ConversationThread { thread_id }
                    if thread_id == &result_thread_id
            )
        });
        scope.recipe.entries.insert(
            resource_id.clone(),
            RecipeEntry {
                resource_id: resource_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        let provider_thread_id = conversation
            .source_thread_id
            .clone()
            .ok_or_else(|| "captured conversation lacks a provider session id".to_string())?;
        // The after-state this publication commits: the wire column accepts
        // the artifact's wire tail, the local column pins the reconstructed
        // local value the review verified. For providers without a
        // standalone artifact the two columns carry one shared value.
        // Candidate and accepted-at identifiers are placeholders until the
        // engine patches them at HeadAdvanced from the fused publish result.
        let published_tail = artifact
            .as_ref()
            .map(|artifact| artifact.portable_tail.clone())
            .or_else(|| action.pins.tail.local.clone())
            .ok_or_else(|| {
                format!(
                    "conversation Push action '{}' has no publishable tail",
                    action.action_id
                )
            })?;
        let accepted_local_tail = seeded
            .as_ref()
            .map(|seeded| seeded.observed_tail.clone())
            .or_else(|| action.pins.tail.local.clone())
            .ok_or_else(|| {
                format!(
                    "conversation Push action '{}' has no local accepted tail",
                    action.action_id
                )
            })?;
        let artifact_sha256 = artifact
            .as_ref()
            .map(|artifact| artifact.artifact_sha256.clone())
            .unwrap_or_else(|| hex_digest(Sha256::digest(&published_bytes).as_slice()));
        let operation_id = ConversationOperationId::parse(generated_named_id("operation")?)?;
        let checkpoint = ConversationCheckpoint {
            generation: 0,
            accepted: Some(mallard_domain::AcceptedConversationVersion {
                artifact_sha256,
                accepted_at_commit_id: action.pins.revision.commit_id.clone(),
                accepted_at_manifest_sha256: action.pins.revision.manifest_sha256.clone(),
                portable_tail: published_tail.clone(),
            }),
            accepted_local_tail: Some(accepted_local_tail.clone()),
            observed_tail: accepted_local_tail.clone(),
            physical: match &seeded {
                Some(seeded) => PhysicalMappingState::Ready {
                    root_mapping: seeded.mapping.clone(),
                    raw_range_pin: RawRangePin {
                        start_byte_offset: seeded.mapping.consumed_byte_offset,
                        end_byte_offset: seeded.mapping.consumed_byte_offset,
                        range_sha256: hex_digest(Sha256::digest([]).as_slice()),
                    },
                },
                None => {
                    let source = captured_transcript_source(conversation)?;
                    let consumed = transcript.bytes.len() as u64;
                    let window = consumed.min(4_096);
                    PhysicalMappingState::Ready {
                        root_mapping: mallard_domain::RootPhysicalMapping {
                            provider_thread_id: provider_thread_id.clone(),
                            root_path: source.to_string_lossy().into_owned(),
                            source_file_size: consumed,
                            consumed_byte_offset: consumed,
                            consumed_record_ordinal: transcript
                                .bytes
                                .iter()
                                .filter(|byte| **byte == b'\n')
                                .count()
                                as u64,
                            probe_window_len: window,
                            probe_window_sha256: hex_digest(
                                Sha256::digest(&transcript.bytes[(consumed - window) as usize..])
                                    .as_slice(),
                            ),
                            last_canonical_message_id: None,
                            last_synthetic_turn_id: None,
                            next_synthetic_turn_ordinal: 0,
                        },
                        raw_range_pin: RawRangePin {
                            start_byte_offset: consumed,
                            end_byte_offset: consumed,
                            range_sha256: hex_digest(Sha256::digest([]).as_slice()),
                        },
                    }
                }
            },
            prefix_witness: seeded
                .as_ref()
                .and_then(|seeded| seeded.prefix_witness.clone()),
            binding_revision: action.pins.context.binding_revision.unwrap_or(0),
        };
        planned_checkpoints.insert(operation_id.clone(), checkpoint);
        operations.push(ConversationSyncOperation {
            operation_id,
            plan_id: Some(plan_id.clone()),
            plan_action_id: Some(action.action_id.clone()),
            cloud: ConversationCloudAnchor {
                storage_id: storage_id.clone(),
                bundle_id: project.bundle_id.clone(),
                revision: action.pins.revision.bundle_revision,
                commit_id: action.pins.revision.commit_id.clone(),
                manifest_sha256: action.pins.revision.manifest_sha256.clone(),
            },
            host_id: binding.host_id.clone(),
            replica_id: binding.replica_id.clone(),
            local_project_id: binding.local_project_id.clone(),
            provider_profile_id: action.pins.context.provider_profile_id.clone(),
            provider: action.provider.clone(),
            thread_id: action.thread_id.clone(),
            operation: ConversationSyncOperationKind::CloudPublication(
                ConversationPublicationOperation {
                    phase: PushJournalPhase::CandidateUploaded,
                    candidate_commit_id: action.pins.revision.commit_id.clone(),
                    candidate_manifest_sha256: action.pins.revision.manifest_sha256.clone(),
                    artifact_sha256: artifact
                        .as_ref()
                        .map(|artifact| artifact.artifact_sha256.clone())
                        .unwrap_or_else(|| hex_digest(Sha256::digest(&published_bytes).as_slice())),
                    tail_before: action.pins.tail.accepted.clone(),
                    tail_after: published_tail,
                    planned_relation: action.pins.tail.classification()?,
                    // Chained after the loop in operation-id order.
                    sync_config_before: SyncConfig::default(),
                    sync_config_after: SyncConfig::default(),
                    bindings_before: document.clone(),
                    bindings_after: document.clone(),
                },
            ),
            provider_thread_id: Some(provider_thread_id),
            materialization_state: ConversationMaterializationState::Resumable,
            expected_host_identity_revision: action.pins.context.host_identity_revision,
            expected_bindings_document_revision: action.pins.context.bindings_document_revision,
            expected_binding_revision: action.pins.context.binding_revision,
            expected_profile_revision: action.pins.context.profile_revision,
            expected_project_binding_revision: action.pins.context.project_binding_revision,
            targets: Vec::new(),
            created_at: updated_at,
            updated_at,
        });
    }
    // Chain the whole-document snapshots in the exact order the engine
    // installs them (sorted operation id): each operation's before-state is
    // its predecessor's after-state, so sequential installation is exact.
    operations.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
    let mut running = reviewed_config;
    let mut running_bindings = repository.load_conversation_bindings()?;
    for operation in &mut operations {
        let checkpoint = planned_checkpoints
            .get(&operation.operation_id)
            .cloned()
            .ok_or_else(|| "planned conversation checkpoint disappeared".to_string())?;
        let before = running.clone();
        let bindings_before = running_bindings.clone();
        let layout = RootTailLayout {
            codec: checkpoint.observed_tail.codec.clone(),
            partition_kind: checkpoint.observed_tail.partition_kind,
            start_record_count: checkpoint.observed_tail.start_record_count,
        };
        match running_bindings.bindings.iter_mut().find(|row| {
            row.host_id == operation.host_id
                && row.replica_id == operation.replica_id
                && row.local_project_id == operation.local_project_id
                && row.provider_profile_id == operation.provider_profile_id
                && row.thread_id == operation.thread_id
        }) {
            Some(row) => {
                row.provider_thread_id = operation.provider_thread_id.clone();
                row.materialization_state = ConversationMaterializationState::Resumable;
                if row.root_tail_layout.is_none() {
                    row.root_tail_layout = Some(layout);
                }
                row.revision = row.revision.saturating_add(1);
                row.updated_at = updated_at;
            }
            None => running_bindings.bindings.push(LocalConversationBinding {
                host_id: operation.host_id.clone(),
                replica_id: operation.replica_id.clone(),
                local_project_id: operation.local_project_id.clone(),
                provider_profile_id: operation.provider_profile_id.clone(),
                thread_id: operation.thread_id.clone(),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(layout),
                provider_thread_id: operation.provider_thread_id.clone(),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: updated_at,
                updated_at,
            }),
        }
        running_bindings.revision = running_bindings.revision.saturating_add(1);
        let scope = SyncScopeKey::Provider(operation.provider.clone());
        let link = running
            .links
            .iter_mut()
            .find(|link| {
                link.local_project_id == operation.local_project_id
                    && link.storage_id == operation.cloud.storage_id
            })
            .ok_or_else(|| "conversation Push requires a registered storage link".to_string())?;
        let threads = link.conversation_checkpoints.entry(scope).or_default();
        let generation = threads
            .get(&operation.thread_id)
            .map(|existing| existing.generation.saturating_add(1))
            .unwrap_or(0);
        let mut checkpoint = checkpoint;
        checkpoint.generation = generation;
        threads.insert(operation.thread_id.clone(), checkpoint);
        running.revision = running.revision.saturating_add(1);
        let ConversationSyncOperationKind::CloudPublication(payload) = &mut operation.operation
        else {
            return Err("conversation Push built a non-publication operation".to_string());
        };
        payload.sync_config_before = before;
        payload.sync_config_after = running.clone();
        payload.bindings_before = bindings_before;
        payload.bindings_after = running_bindings.clone();
    }
    // Every carried conversation stays selected in its scope's recipe.
    for scope in writable_scopes.values_mut() {
        for thread in scope.conversations.values() {
            scope.recipe.entries.insert(
                thread.resource_id.clone(),
                RecipeEntry {
                    resource_id: thread.resource_id.clone(),
                    apply_policy: ApplyPolicy::SafeFile,
                    required: false,
                },
            );
        }
    }
    Ok((
        PublishBundleRequest {
            identity: BundleIdentity {
                bundle_id: project.bundle_id.clone(),
                display_name: project.display_name.clone(),
                kind: BundleKind::Project,
                repository_fingerprint: project.repository_fingerprint.clone(),
            },
            published_by_origin: published_by,
            writable_scopes,
            reviewed_scope_pins,
            expected_head,
            updated_at,
        },
        operations,
    ))
}

fn conversation_raw_fallback(transcript: &provider_capture::CapturedFile) -> Vec<u8> {
    transcript.bytes.clone()
}

fn cloud_publication_from_snapshot(snapshot: &BundleSnapshot) -> ConversationCloudPublication {
    ConversationCloudPublication {
        revision: snapshot.head.revision,
        commit_id: snapshot.head.commit_id.clone(),
        manifest_sha256: snapshot.head.manifest_sha256.clone(),
        manifest_size: snapshot.head.manifest_size,
        scope_pins: snapshot
            .manifest
            .provider_scopes()
            .map(|(provider, scope)| {
                (
                    SyncScopeKey::Provider(provider.clone()).canonical(),
                    (scope.revision, scope.content_sha256.clone()),
                )
            })
            .collect(),
    }
}

/// The push-recovery content witness: identify a published candidate by
/// comparing each operation's journaled `tail_after` (the artifact's wire
/// tail) against the manifest thread's wire tail — wire against wire, never
/// a local value against the manifest.
fn snapshot_contains_publications(
    snapshot: &BundleSnapshot,
    expected: &[(ThreadId, PortableConversationTail)],
) -> bool {
    expected.iter().all(|(thread_id, tail_after)| {
        snapshot
            .manifest
            .find_conversation(thread_id)
            .is_some_and(|(_, thread)| match &thread.portable_tail {
                Some(tail) => tail == tail_after,
                // Providers without the portable codec are witnessed by the
                // synthesized whole-thread equivalence.
                None => {
                    thread.message_count == tail_after.accepted_end_record_count
                        && thread.semantic_digest == tail_after.accepted_tail_state
                }
            })
    })
}

/// Prove every provider thread can be committed to its reviewed binding
/// before immutable publication and the remote head CAS.  The binding
/// document's global uniqueness check remains the final invariant, but it is
/// too late to discover an ownership conflict after the cloud commit.
fn validate_cloud_publication_binding_owners(
    bindings: &[LocalConversationBinding],
    operations: &[ConversationSyncOperation],
) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    for binding in bindings {
        let Some(provider_thread_id) = binding.provider_thread_id.as_ref() else {
            continue;
        };
        owners.insert(
            (
                binding.host_id.clone(),
                binding.provider_profile_id.clone(),
                provider_thread_id.clone(),
            ),
            (
                binding.replica_id.clone(),
                binding.local_project_id.clone(),
                binding.thread_id.clone(),
            ),
        );
    }
    for operation in operations {
        let Some(provider_thread_id) = operation.provider_thread_id.as_deref() else {
            continue;
        };
        let owner_key = (
            operation.host_id.clone(),
            operation.provider_profile_id.clone(),
            provider_thread_id.to_string(),
        );
        let target = (
            operation.replica_id.clone(),
            operation.local_project_id.clone(),
            operation.thread_id.clone(),
        );
        if owners
            .get(&owner_key)
            .is_some_and(|current| current != &target)
        {
            return Err(
                "conversation Push cannot publish because a selected provider thread belongs to another local project registration; re-register that project before pushing"
                    .to_string(),
            );
        }
        owners.insert(owner_key, target);
    }
    Ok(())
}

struct ReviewedPublishOutcome {
    published: bool,
    snapshot: BundleSnapshot,
    /// Reviewed actions whose apply-time re-proof failed; each kept its
    /// previous cloud state and reports its own reason.
    stale_conversations: Vec<(ConversationPlanAction, String)>,
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
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ReviewedPublishOutcome, String> {
    let phase_started = Instant::now();
    let previous = observed.as_ref().map(|(snapshot, _)| snapshot);
    let expected_head = observed
        .as_ref()
        .map(|(_, token)| PublishExpectation::Match(token.clone()))
        .unwrap_or(PublishExpectation::Absent);
    let (revision, commit_id, manifest_sha256) = previous
        .map(|snapshot| {
            (
                snapshot.head.revision,
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
                || plan.revision != revision
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
    on_phase("load reviewed conversation plan", phase_started.elapsed());
    let phase_started = Instant::now();
    let document = repository.load_conversation_bindings()?;
    let machine = repository.load_bindings()?;
    let identity = repository.load_or_create_host_identity()?;
    // Apply-time re-proof, one item at a time: an action whose local
    // conversation or shared state moved after review fails closed by
    // itself; the other reviewed actions still publish, and every skipped
    // thread keeps its previous cloud state in the manifest.
    let revalidate = |action: &ConversationPlanAction| -> Result<(), String> {
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
                && candidate.provider_profile_id == action.pins.context.provider_profile_id
                && candidate.thread_id == action.thread_id
        });
        let profile = machine
            .profiles
            .iter()
            .find(|profile| profile.profile_id == action.pins.context.provider_profile_id)
            .ok_or_else(|| "reviewed provider profile disappeared".to_string())?;
        let _ = codec_digests(&conversation.lineage);
        let (scope_revision, scope_sha256) = previous
            .and_then(|snapshot| snapshot.manifest.provider_scope(&action.provider))
            .map(|scope| (scope.revision, scope.content_sha256.clone()))
            .unwrap_or_else(absent_scope_pin);
        // Re-prove the raw unaccepted interval byte-for-byte; the canonical
        // re-proof happens in build_publish_request's reconstruction.
        let raw_range = match &action.pins.tail.raw_range {
            Some(pin) => {
                let source = captured_transcript_source(conversation)?;
                let bytes = codex_history_graph::read_rollout_bounded(&source)
                    .unwrap_or_else(|_| conversation_raw_fallback(transcript));
                if pin.end_byte_offset > bytes.len() as u64 {
                    None
                } else {
                    Some(RawRangePin {
                        start_byte_offset: pin.start_byte_offset,
                        end_byte_offset: pin.end_byte_offset,
                        range_sha256: hex_digest(
                            Sha256::digest(
                                &bytes
                                    [pin.start_byte_offset as usize..pin.end_byte_offset as usize],
                            )
                            .as_slice(),
                        ),
                    })
                }
            }
            None => None,
        };
        // The live checkpoint generation is the honest staleness read here;
        // the tail fields mirror the action's own pins because the byte-level
        // re-proofs (raw range here, full reconstruction in the build) carry
        // the real content verification.
        let live_checkpoint_generation = repository.load_config().ok().and_then(|config| {
            config
                .links
                .iter()
                .find(|link| {
                    link.local_project_id == binding.local_project_id
                        && &link.storage_id == storage_id
                })
                .and_then(|link| {
                    link.conversation_checkpoints
                        .get(&SyncScopeKey::Provider(action.provider.clone()))
                })
                .and_then(|threads| threads.get(&action.thread_id))
                .map(|checkpoint| checkpoint.generation)
        });
        let context = ConversationApplyContext {
            now: updated_at,
            pins: ConversationActionPins {
                context: ConversationContextPins {
                    host_id: binding.host_id.clone(),
                    replica_id: binding.replica_id.clone(),
                    local_project_id: binding.local_project_id.clone(),
                    provider_profile_id: action.pins.context.provider_profile_id.clone(),
                    host_identity_revision: identity.revision,
                    profile_revision: profile.revision,
                    project_binding_revision: binding.revision,
                    bindings_document_revision: document.revision,
                    binding_revision: local_binding.map(|binding| binding.revision),
                },
                revision: ConversationRevisionPins {
                    bundle_revision: revision,
                    commit_id: commit_id.clone(),
                    manifest_sha256: manifest_sha256.clone(),
                    scope_revision,
                    scope_sha256,
                },
                tail: ConversationTailPins {
                    accepted: action.pins.tail.accepted.clone(),
                    accepted_local: action.pins.tail.accepted_local.clone(),
                    local: action.pins.tail.local.clone(),
                    cloud: action.pins.tail.cloud.clone(),
                    checkpoint_generation: live_checkpoint_generation,
                    lineage_digest: action.pins.tail.lineage_digest.clone(),
                    semantic_digest: action.pins.tail.semantic_digest.clone(),
                    raw_range,
                },
                target: ConversationTargetPins {
                    local: TargetExpectation::Sha256(hex_digest(
                        Sha256::digest(&transcript.bytes).as_slice(),
                    )),
                    provider_state: Vec::new(),
                },
                codec: conversation.lineage_codec.clone(),
                expires_at: action.pins.expires_at,
            },
        };
        validate_conversation_action_pins(action, &context).map_err(|pin| {
            format!(
                "conversation Push action '{}' is stale at {:?}",
                action.action_id, pin
            )
        })
    };
    let mut stale_conversations: Vec<(ConversationPlanAction, String)> = Vec::new();
    let mut validated_actions = Vec::with_capacity(selected_actions.len());
    for action in selected_actions {
        match revalidate(&action) {
            Ok(()) => validated_actions.push(action),
            Err(message) => stale_conversations.push((action, message)),
        }
    }
    let selected_actions = validated_actions;
    on_phase("revalidate reviewed conversations", phase_started.elapsed());
    let phase_started = Instant::now();
    let resolved_binding = resolve_project_binding(repository, binding)?;
    let (request, operations) = build_publish_request(
        repository,
        &plan_id,
        storage_id,
        project,
        binding,
        &resolved_binding.homes,
        recipe,
        captured,
        previous,
        expected_head,
        &selected_actions,
        updated_at,
    )?;
    validate_cloud_publication_binding_owners(&document.bindings, &operations)?;
    let published_expectations: Vec<(ThreadId, PortableConversationTail)> = operations
        .iter()
        .filter_map(|operation| match &operation.operation {
            ConversationSyncOperationKind::CloudPublication(payload) => {
                Some((operation.thread_id.clone(), payload.tail_after.clone()))
            }
            _ => None,
        })
        .collect();
    on_phase("build publish request", phase_started.elapsed());
    let phase_started = Instant::now();
    if operations.is_empty() {
        let outcome = engine.publish_with_progress(request, on_progress, on_phase)?;
        if let Some(plan_id) = conversation_push_plan_id {
            let _ = repository.discard_conversation_push_plan(plan_id);
        }
        return Ok(ReviewedPublishOutcome {
            published: outcome.published,
            snapshot: outcome.snapshot,
            stale_conversations,
        });
    }
    let published = std::cell::Cell::new(false);
    let mut failpoints = NoConversationFailpoints;
    apply_conversation_cloud_publications_locked(
        repository,
        operations,
        || {
            let outcome = engine.publish_with_progress(request, on_progress, &mut *on_phase)?;
            published.set(outcome.published);
            Ok(cloud_publication_from_snapshot(&outcome.snapshot))
        },
        || {
            let snapshot = engine.inspect(&project.bundle_id)?;
            Ok(
                snapshot_contains_publications(&snapshot, &published_expectations).then(|| {
                    published.set(previous.is_none_or(|previous| {
                        snapshot.head.revision != previous.head.revision
                            || snapshot.head.commit_id != previous.head.commit_id
                    }));
                    cloud_publication_from_snapshot(&snapshot)
                }),
            )
        },
        &mut failpoints,
    )?;
    on_phase(
        "publish and journal conversation receipts",
        phase_started.elapsed(),
    );
    let phase_started = Instant::now();
    let snapshot = engine.inspect(&project.bundle_id)?;
    if !snapshot_contains_publications(&snapshot, &published_expectations) {
        return Err(
            "published bundle head does not contain the reviewed conversations".to_string(),
        );
    }
    if let Some(plan_id) = conversation_push_plan_id {
        let _ = repository.discard_conversation_push_plan(plan_id);
    }
    on_phase("confirm published head", phase_started.elapsed());
    Ok(ReviewedPublishOutcome {
        published: published.get(),
        snapshot,
        stale_conversations,
    })
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn push_bundle_reviewed_with_repository(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: BundleRecipe,
    project_content_review: ProjectContentPushReview,
    conversation_push_plan_id: Option<PlanId>,
    approved_conversation_action_ids: Vec<ActionId>,
    on_progress: &mut dyn FnMut(FetchProgress),
) -> Result<ProjectOperationResult, String> {
    push_bundle_reviewed_with_progress(
        repository,
        local_project_id,
        storage_id,
        requested_recipe,
        project_content_review,
        conversation_push_plan_id,
        approved_conversation_action_ids,
        on_progress,
        &mut |_, _| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn push_bundle_reviewed_with_progress(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
    requested_recipe: BundleRecipe,
    project_content_review: ProjectContentPushReview,
    conversation_push_plan_id: Option<PlanId>,
    approved_conversation_action_ids: Vec<ActionId>,
    on_progress: &mut dyn FnMut(FetchProgress),
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ProjectOperationResult, String> {
    requested_recipe.validate_machine_local()?;
    let phase_started = Instant::now();
    let _host_operation = repository.acquire_host_operation()?;
    let config = repository.load_config()?;
    let project = config
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
    let link = stored_link.clone();
    on_phase("load config and binding", phase_started.elapsed());
    let phase_started = Instant::now();
    // One provider walk feeds the review-token freshness check and byte
    // capture below.
    let wants_project_content = project_content_review.review_token.is_some();
    let eligibility = project_file_sync_eligibility(&binding);
    if wants_project_content && eligibility.state != ProjectFileSyncEligibilityState::Eligible {
        return Err(eligibility.reason);
    }
    let mut capture_request = capture_request_for_binding(repository, &binding)?;
    capture_request.include_project_content = wants_project_content;
    let discovery = provider_capture::discover_full(provider_registry(), &capture_request)?;
    let discovered = discovery.project_inventory();
    let local_conversations = discovered
        .resources
        .iter()
        .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
        .collect::<Vec<_>>();
    if let Some(candidate) = local_conversations
        .iter()
        .find(|candidate| candidate.blocked_reason.is_some())
    {
        return Err(format!(
            "conversation Push is blocked because '{}' cannot be reviewed safely; resolve the local capture problem before pushing",
            candidate.display_name
        ));
    }
    on_phase("discover local state", phase_started.elapsed());
    let phase_started = Instant::now();
    let mut push_recipe = requested_recipe;
    remove_excluded_codex_subagents_from_recipe(
        &mut push_recipe,
        discovery.excluded_subagent_resource_ids(),
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
    on_phase("reconcile selection and recipe", phase_started.elapsed());
    let phase_started = Instant::now();
    let (_, engine) = storage_engine(repository, storage_id)?;
    let observed = engine.inspect_optional_with_token(&project.bundle_id)?;
    let has_stored_bound_conversation = observed.as_ref().is_some_and(|(snapshot, _)| {
        snapshot
            .manifest
            .provider_scopes()
            .any(|(provider, scope)| {
                binding.profile_ids.contains_key(provider) && !scope.conversations.is_empty()
            })
    });
    if conversation_push_plan_id.is_none()
        && (!local_conversations.is_empty() || has_stored_bound_conversation)
    {
        return Err(
            "conversation Push requires a current Push review; refresh and review before pushing"
                .to_string(),
        );
    }
    on_phase("inspect remote head", phase_started.elapsed());
    let mut phase_started = Instant::now();
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
        on_phase("verify reviewed project files", phase_started.elapsed());
        phase_started = Instant::now();
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
        provider_registry(),
        &capture_request,
        &discovery,
        &push_recipe,
    )?;
    on_phase("capture selected payloads", phase_started.elapsed());
    let destination_status = classify_push_destination(
        observed.as_ref().map(|(snapshot, _)| snapshot),
        &link,
        &binding,
        &mut continuity_probe(&engine, &project.bundle_id),
    );
    if let Some(error) = push_destination_error(&destination_status) {
        return Err(error);
    }
    let pushed_at = now_secs();
    let publish_outcome = publish_reviewed(
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
        &mut *on_phase,
    )?;
    let was_published = publish_outcome.published;
    let published_snapshot = publish_outcome.snapshot;
    let phase_started = Instant::now();
    let revision = published_snapshot.head.revision;
    let commit_id = published_snapshot.head.commit_id.clone();
    let manifest_size = published_snapshot.head.manifest_size;
    let manifest_sha256 = published_snapshot.head.manifest_sha256.clone();
    let expected_project_revision = project.revision;
    let expected_project_recipe_revision = project.recipe.revision;
    let bundle_id = project.bundle_id.clone();
    // Advance a base for every writable scope that published — and only for
    // those. Viewed, unbound, and unknown scopes never move.
    let published_scope_coordinates = published_snapshot.manifest.scope_coordinates();
    let writable_scopes = writable_scopes_for_binding(&binding);
    let published_repository_base = RepositoryBase {
        revision,
        commit_id: commit_id.clone(),
        manifest_sha256: manifest_sha256.clone(),
        manifest_size,
    };
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
        {
            let current_link = config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == *local_project_id && link.storage_id == *storage_id
                })
                .ok_or_else(|| {
                    "project storage link was removed while publishing".to_string()
                })?;
            for scope in &writable_scopes {
                let last_pull_at = current_link
                    .scope_bases
                    .get(scope)
                    .and_then(|base| base.last_pull_at);
                match published_scope_coordinates.get(scope) {
                    Some((scope_revision, scope_sha256)) => {
                        current_link.scope_bases.insert(
                            scope.clone(),
                            ScopeBase {
                                scope: scope.clone(),
                                scope_revision: *scope_revision,
                                scope_sha256: scope_sha256.clone(),
                                reviewed_in: published_repository_base.clone(),
                                binding_revision: binding.revision,
                                last_pull_at,
                                last_push_at: Some(pushed_at),
                            },
                        );
                    }
                    None => {
                        current_link.scope_bases.remove(scope);
                    }
                }
            }
        }
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
    on_phase("persist push receipts and config", phase_started.elapsed());
    let state = if was_published { "synced" } else { "unchanged" };
    let mut results = published_snapshot
        .manifest
        .all_resources()
        .map(|(resource_id, _)| OperationResourceResult {
            resource_id: resource_id.clone(),
            state: state.to_string(),
            message: None,
        })
        .collect::<Vec<_>>();
    let resources_changed = Some(if was_published { results.len() } else { 0 });
    for (action, message) in &publish_outcome.stale_conversations {
        results.push(OperationResourceResult {
            resource_id: action
                .source_resource_id
                .clone()
                .map(Ok)
                .unwrap_or_else(|| conversation_resource_id(&action.provider, &action.thread_id))?,
            state: "failed".to_string(),
            message: Some(message.clone()),
        });
    }
    let skipped = publish_outcome.stale_conversations.len();
    let skipped_note = match skipped {
        0 => String::new(),
        1 => "; 1 reviewed conversation changed during review, kept its stored state, and needs \
              a fresh Push review"
            .to_string(),
        _ => format!(
            "; {} reviewed conversations changed during review, kept their stored state, and \
             need a fresh Push review",
            skipped
        ),
    };
    Ok(ProjectOperationResult {
        success: true,
        message: if was_published {
            format!("Published revision {}{}", revision, skipped_note)
        } else {
            format!("Already up to date — revision {}{}", revision, skipped_note)
        },
        operation_id: Some(commit_id),
        resources_changed,
        revision: Some(revision),
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

pub(super) fn plan_bundle_restore_with_progress(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
    on_progress: &mut dyn FnMut(FetchProgress),
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<RestorePlan, String> {
    let phase_started = Instant::now();
    let _host_operation = repository.acquire_host_operation()?;
    let resolved = require_current_binding(repository, binding)?;
    let binding = resolved.binding.clone();
    if &binding.bundle_id != bundle_id {
        return Err("binding and requested bundle IDs differ".to_string());
    }
    require_codex_conversation_paths_ready(repository, &binding.local_project_id)?;
    let (engine, snapshot) = inspect_from_storage(repository, storage_id, bundle_id)?;
    let config = repository.load_config()?;
    if config.project(&binding.local_project_id).is_none() {
        return Err(format!(
            "unknown local project '{}'",
            binding.local_project_id
        ));
    }
    let link = project_link(&config, &binding.local_project_id, storage_id)
        .cloned()
        .ok_or_else(|| "project is not linked to this storage".to_string())?;
    let repository_base = repository_base_from_link(&link);
    let remote_continuity = classify_remote_continuity(
        Some(&snapshot.head),
        repository_base.as_ref(),
        &mut continuity_probe(&engine, bundle_id),
    );
    on_phase("inspect remote head and manifest", phase_started.elapsed());
    let phase_started = Instant::now();
    let identity = repository.load_or_create_host_identity()?;
    if identity.host_id != binding.host_id {
        return Err("host identity changed while preparing the Pull review".to_string());
    }
    let mut plan = engine.build_restore_plan_from_snapshot(
        provider_registry(),
        &snapshot,
        &binding,
        &resolved.homes,
        now_secs(),
        identity.revision,
    )?;
    plan.remote_continuity = Some(remote_continuity);
    on_phase("build file restore plan", phase_started.elapsed());
    let has_bound_provider_conversations =
        snapshot
            .manifest
            .provider_scopes()
            .any(|(provider, scope)| {
                resolved.homes.contains_key(provider) && !scope.conversations.is_empty()
            })
            || !resolved.homes.is_empty();
    if has_bound_provider_conversations {
        let phase_started = Instant::now();
        let mut byte_source =
            engine.byte_source(&snapshot, Some(object_cache()), Some(on_progress));
        let capture_request = capture_request_for_binding(repository, &binding)?;
        let (discovery, views) = super::codex_watch_service::session_watch_service()
            .discovery_for(
                provider_registry(),
                &binding.local_project_id,
                &capture_request,
            )?;
        // Providers without a cursor codec capture up front; Codex rows are
        // captured only when one classification pass (cache-served when
        // quiet) marks them byte-pinning Pull candidates. Adopting cloud
        // metadata pins no local bytes and CreateLocal has none, so
        // storage-only, synced, and unrelated sessions read nothing.
        let base_selection = discovery
            .project_inventory()
            .resources
            .into_iter()
            .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
            .filter(|candidate| {
                candidate
                    .provider
                    .as_ref()
                    .map(|provider| provider.as_str())
                    != Some("codex")
            })
            .map(|candidate| candidate.resource_id)
            .collect::<BTreeSet<_>>();
        let captured = provider_capture::capture_from_discovery(
            provider_registry(),
            &capture_request,
            &discovery,
            &base_selection,
        )?;
        let preliminary = thread_review(
            repository,
            Some(&snapshot),
            &mut byte_source,
            &views,
            Some(&discovery),
            &captured,
            &BTreeMap::new(),
            &resolved,
            Some(&plan.storage_id),
            plan.created_at,
            plan.expires_at,
            ReviewPurpose::Observe,
        )?;
        let mut selection = base_selection.clone();
        for entry in preliminary.entries.iter().filter(|entry| {
            entry.pull_decision.selectable
                && entry.local_present
                && entry.pull_decision.action != ConversationReviewAction::AdoptCloudMetadata
        }) {
            selection.insert(entry.resource_id.to_string());
        }
        let (selection, candidate_pins) = reuse_candidate_pins(
            &capture_request,
            &binding.local_project_id,
            &views,
            &base_selection,
            selection,
        );
        let captured = if selection == base_selection {
            captured
        } else {
            provider_capture::capture_from_discovery(
                provider_registry(),
                &capture_request,
                &discovery,
                &selection,
            )?
        };
        store_candidate_pins_from_capture(
            &capture_request,
            &binding.local_project_id,
            &views,
            &base_selection,
            &captured,
        );
        on_phase(
            "discover and capture local conversations",
            phase_started.elapsed(),
        );
        let phase_started = Instant::now();
        let review = thread_review(
            repository,
            Some(&snapshot),
            &mut byte_source,
            &views,
            Some(&discovery),
            &captured,
            &candidate_pins,
            &resolved,
            Some(&plan.storage_id),
            plan.created_at,
            plan.expires_at,
            ReviewPurpose::PullPlan,
        )?;
        plan.conversation_actions = review.pull_actions;
        plan.conversation_review = review.pull_review;
        on_phase("classify and build pull actions", phase_started.elapsed());
    }
    let phase_started = Instant::now();
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
    apply_bundle_restore_with_progress(
        repository,
        plan_id,
        approved_action_ids,
        &mut |_| {},
        &mut |_, _| {},
    )
}

struct PreparedPullConversation {
    action: ConversationPlanAction,
    cloud_transcript: Vec<u8>,
    cloud_file_sha256: String,
    cloud_provider_thread_id: String,
    cloud_cwd: String,
    cloud_forked_from: Option<ThreadId>,
    cloud_preserved: Option<PreservedProvenance>,
    cloud_display_title: Option<String>,
    cloud_updated_at: u64,
}

#[derive(Clone, Debug)]
struct SelectedPullConversation {
    action: ConversationPlanAction,
}

fn capture_all_local_conversations(
    repository: &LocalStateRepository,
    resolved: &ResolvedBinding,
) -> Result<
    (
        Vec<provider_capture::LocalSessionView>,
        provider_capture::CapturedResources,
    ),
    String,
> {
    let capture_request = capture_request_for_binding(repository, &resolved.binding)?;
    let discovery = provider_capture::discover_full(provider_registry(), &capture_request)?;
    let selected = discovery
        .project_inventory()
        .resources
        .into_iter()
        .filter(|candidate| candidate.kind == CaptureResourceKind::Conversation)
        .map(|candidate| candidate.resource_id)
        .collect::<BTreeSet<_>>();
    let captured = provider_capture::capture_from_discovery(
        provider_registry(),
        &capture_request,
        &discovery,
        &selected,
    )?;
    Ok((provider_capture::local_session_views(&discovery), captured))
}

fn local_target_expectation_for_action(
    action: &ConversationPlanAction,
    binding: &ProjectBinding,
    document: &super::domain::ConversationBindings,
    captured: &provider_capture::CapturedResources,
) -> TargetExpectation {
    // Metadata-only actions neither read nor write the local transcript:
    // their stale-pin guard covers everything except the local file, so
    // review never captures them and apply derives no local digest.
    if action.action == ConversationPlanActionType::AdoptCloudMetadata {
        return TargetExpectation::Absent;
    }
    let provider_thread_id = document
        .bindings
        .iter()
        .find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.pins.context.provider_profile_id
                && candidate.thread_id == action.thread_id
        })
        .and_then(|candidate| candidate.provider_thread_id.as_deref());
    provider_thread_id
        .and_then(|provider_thread_id| {
            captured.conversations.values().find(|conversation| {
                conversation.provider == action.provider
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
    resolved: &ResolvedBinding,
    snapshot: &BundleSnapshot,
    bytes: &mut dyn BundleBytes,
    actions: &[SelectedPullConversation],
    storage_id: &StorageId,
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
    let (_, captured) = capture_all_local_conversations(repository, resolved)?;
    let binding = &resolved.binding;
    let document = repository.load_conversation_bindings()?;
    let machine = repository.load_bindings()?;
    let identity = repository.load_or_create_host_identity()?;
    // One batched prefetch covers every approved in-policy transcript; the
    // per-action bounded read below is then a verified cache hit. Best-effort:
    // a failed batch falls back to per-action reads, which fail one item, not
    // the pass. Oversized transcripts are rejected before transfer.
    let transcript_paths = actions
        .iter()
        .filter_map(|selected| {
            snapshot
                .manifest
                .find_conversation(&selected.action.thread_id)
                .and_then(|(_, thread)| {
                    thread
                        .files
                        .iter()
                        .find(|file| file.role == ConversationFileRole::Transcript)
                })
                .map(|file| file.logical_path.clone())
        })
        .filter(|path| {
            snapshot
                .manifest
                .find_file(path)
                .is_some_and(|(_, entry)| entry.size <= MAX_CONVERSATION_FILE_BYTES)
        })
        .collect::<Vec<_>>();
    let _ = bytes.prefetch(&transcript_paths);
    let mut prepared = Vec::with_capacity(actions.len());
    let mut failures = Vec::new();
    let prepare_one = |selected: &SelectedPullConversation,
                       bytes: &mut dyn BundleBytes|
     -> Result<PreparedPullConversation, String> {
        let action = &selected.action;
        let (_, thread) = snapshot
            .manifest
            .find_conversation(&action.thread_id)
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
            .find_file(&transcript_ref.logical_path)
            .ok_or_else(|| {
                format!(
                    "cloud conversation '{}' transcript is absent from the manifest",
                    action.thread_id
                )
            })?
            .1
            .sha256
            .clone();
        let cloud_transcript = bytes.read_bounded(
            &transcript_ref.logical_path,
            MAX_CONVERSATION_FILE_BYTES,
            "cloud conversation transcript",
        )?;
        let adapter = provider_registry().require_sessions(&action.provider)?;
        let analysis = adapter.analyze(&cloud_transcript);
        let cloud_provider_thread_id = analysis
            .source_provider_session_id
            .clone()
            .ok_or_else(|| "cloud transcript lacks an active provider session id".to_string())?;
        let cloud_cwd = analysis
            .source_cwd
            .clone()
            .ok_or_else(|| "cloud transcript lacks a structural cwd".to_string())?;
        let (lineage_digest, semantic_digest) = codec_digests(&analysis.lineage);
        let local_binding = document.bindings.iter().find(|candidate| {
            candidate.host_id == binding.host_id
                && candidate.replica_id == binding.replica_id
                && candidate.local_project_id == binding.local_project_id
                && candidate.provider_profile_id == action.pins.context.provider_profile_id
                && candidate.thread_id == action.thread_id
        });
        let profile = machine
            .profiles
            .iter()
            .find(|profile| profile.profile_id == action.pins.context.provider_profile_id)
            .ok_or_else(|| "reviewed provider profile disappeared".to_string())?;
        let provider_state = action
            .pins
            .target
            .provider_state
            .iter()
            .map(|target| {
                let current = bounded_file_expectation(Path::new(&target.target_path))?;
                Ok(ProviderStateExpectation {
                    target_path: target.target_path.clone(),
                    expectation: current,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (scope_revision, scope_sha256) = provider_scope_pin(Some(snapshot), &action.provider);
        let live_checkpoint_generation = repository.load_config().ok().and_then(|config| {
            config
                .links
                .iter()
                .find(|link| {
                    link.local_project_id == binding.local_project_id
                        && &link.storage_id == storage_id
                })
                .and_then(|link| {
                    link.conversation_checkpoints
                        .get(&SyncScopeKey::Provider(action.provider.clone()))
                })
                .and_then(|threads| threads.get(&action.thread_id))
                .map(|checkpoint| checkpoint.generation)
        });
        let context = ConversationApplyContext {
            now,
            pins: ConversationActionPins {
                context: ConversationContextPins {
                    host_id: binding.host_id.clone(),
                    replica_id: binding.replica_id.clone(),
                    local_project_id: binding.local_project_id.clone(),
                    provider_profile_id: action.pins.context.provider_profile_id.clone(),
                    host_identity_revision: identity.revision,
                    profile_revision: profile.revision,
                    project_binding_revision: binding.revision,
                    bindings_document_revision: document.revision,
                    binding_revision: local_binding.map(|binding| binding.revision),
                },
                revision: ConversationRevisionPins {
                    bundle_revision: snapshot.head.revision,
                    commit_id: snapshot.head.commit_id.clone(),
                    manifest_sha256: snapshot.head.manifest_sha256.clone(),
                    scope_revision,
                    scope_sha256,
                },
                tail: ConversationTailPins {
                    accepted: action.pins.tail.accepted.clone(),
                    accepted_local: action.pins.tail.accepted_local.clone(),
                    local: action.pins.tail.local.clone(),
                    cloud: action.pins.tail.cloud.clone(),
                    checkpoint_generation: live_checkpoint_generation,
                    // The downloaded transcript's own analysis digests: pin equality
                    // here is the transcript-versus-manifest content verification.
                    lineage_digest,
                    semantic_digest,
                    raw_range: action.pins.tail.raw_range.clone(),
                },
                target: ConversationTargetPins {
                    local: local_target_expectation_for_action(
                        action, binding, &document, &captured,
                    ),
                    provider_state,
                },
                codec: thread.lineage_codec.clone(),
                expires_at: action.pins.expires_at,
            },
        };
        validate_conversation_action_pins(action, &context).map_err(|pin| {
            format!(
                "conversation Pull action '{}' is stale at {:?}",
                action.action_id, pin
            )
        })?;
        Ok(PreparedPullConversation {
            action: action.clone(),
            cloud_transcript,
            cloud_file_sha256,
            cloud_provider_thread_id,
            cloud_cwd,
            cloud_forked_from: thread.forked_from.clone(),
            cloud_preserved: thread.preserved.clone(),
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
            .unwrap_or_else(|| conversation_resource_id(&action.provider, &action.thread_id))?,
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

/// One reviewed Codex session-index row rewrite. `replace_existing` marks an
/// identity that already owns index rows (MaterializeCloud's destination,
/// keep-both's preserved rename): its old rows are dropped and one current
/// row is appended. Fresh identities reject collisions instead.
struct CodexIndexRowSpec {
    provider_thread_id: String,
    thread_name: String,
    updated_at: u64,
    replace_existing: bool,
}

/// Decode a portable Codex title through the same path codec as its
/// transcript before the title enters machine-local provider state. The
/// round-trip gate also rejects a cloud title that already contains the
/// destination root: accepting that collision would capture unrelated text
/// into the reserved wire namespace on the next Push.
fn decode_codex_portable_display_title(
    title: &str,
    target_root: &str,
    canonical_target_root: &str,
) -> Result<String, String> {
    validate_pathmap_root("project root", target_root)?;
    let home = std::env::var("HOME").ok();
    let mapper = CodexPathMapper::new(
        target_root,
        (canonical_target_root != target_root).then_some(canonical_target_root),
        home.as_deref(),
    )?;
    let wire = serde_json::Value::String(title.to_string());
    mapper
        .verify_wire_record(&wire)
        .map_err(|error| format!("portable Codex display title: {error}"))?;
    match mapper.decode_record(&wire)? {
        serde_json::Value::String(decoded) => Ok(decoded),
        _ => Err("portable Codex display title did not decode to text".to_string()),
    }
}

/// Read the local Codex session index and build its reviewed replacement,
/// rewriting one row per spec. Returns no payload for an empty spec list: a
/// titleless task leaves provider presentation metadata untouched.
fn codex_index_payload(
    path: &Path,
    rows: &[CodexIndexRowSpec],
) -> Result<(TargetExpectation, Option<Vec<u8>>), String> {
    let expectation = bounded_file_expectation(path)?;
    if rows.is_empty() {
        return Ok((expectation, None));
    }
    let bytes = match &expectation {
        TargetExpectation::Absent => Vec::new(),
        TargetExpectation::Sha256(_) => fs::read(path)
            .map_err(|error| format!("read Codex session index '{}': {error}", path.display()))?,
    };
    let mut output = Vec::with_capacity(bytes.len().saturating_add(256 * rows.len()));
    for raw in bytes.split_inclusive(|byte| *byte == b'\n') {
        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            output.extend_from_slice(raw);
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(line)
            .map_err(|_| "Codex session index contains malformed JSON".to_string())?;
        let id = value
            .get("id")
            .or_else(|| value.get("session_id"))
            .and_then(serde_json::Value::as_str);
        if let Some(spec) = rows
            .iter()
            .find(|spec| id == Some(spec.provider_thread_id.as_str()))
        {
            if !spec.replace_existing {
                return Err("reserved Codex provider ID is already indexed".to_string());
            }
            continue;
        }
        output.extend_from_slice(raw);
    }
    if !output.is_empty() && !output.ends_with(b"\n") {
        output.push(b'\n');
    }
    for spec in rows {
        let updated_at = Utc
            .timestamp_opt(spec.updated_at as i64, 0)
            .single()
            .ok_or_else(|| "cloud conversation timestamp is out of range".to_string())?
            .format("%Y-%m-%dT%H:%M:%S%.6fZ")
            .to_string();
        let mut row = serde_json::to_vec(&serde_json::json!({
            "id": spec.provider_thread_id,
            "thread_name": spec.thread_name,
            "updated_at": updated_at,
        }))
        .map_err(|error| format!("serialize Codex session index row: {error}"))?;
        row.push(b'\n');
        output.extend(row);
    }
    Ok((expectation, Some(output)))
}

// The receipt dwarfs the checkpoint by design; boxing it would ripple
// through every caller for no behavioral gain.
/// Apply one prepared materializing Pull action through the journaled
/// engine: reconstruct the verified cloud graph exactly once under fresh
/// identities, journal the exact whole-document after-states, and install
/// no-clobber. Metadata-only actions never reach this function.
fn apply_prepared_pull_conversation(
    repository: &LocalStateRepository,
    plan: &RestorePlan,
    resolved: &ResolvedBinding,
    prepared: PreparedPullConversation,
    expected_bindings_document_revision: u64,
    applied_at: u64,
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<ApplyReceipt, String> {
    let binding = &resolved.binding;
    let action = prepared.action;
    let document_before = repository.load_conversation_bindings()?;
    if document_before.revision != expected_bindings_document_revision {
        return Err("conversation bindings changed during Pull apply".to_string());
    }
    let live_binding = document_before.bindings.iter().find(|candidate| {
        candidate.host_id == binding.host_id
            && candidate.replica_id == binding.replica_id
            && candidate.local_project_id == binding.local_project_id
            && candidate.provider_profile_id == action.pins.context.provider_profile_id
            && candidate.thread_id == action.thread_id
    });
    if live_binding.map(|binding| binding.revision) != action.pins.context.binding_revision {
        return Err("conversation binding changed after review".to_string());
    }
    let live_provider_thread_id =
        live_binding.and_then(|binding| binding.provider_thread_id.as_deref());
    if action.action == ConversationPlanActionType::MaterializeCloud
        && live_provider_thread_id.is_none()
    {
        return Err(
            "conversation cloud update lost its provider identity after review".to_string(),
        );
    }
    let adapter = provider_registry().require_sessions(&action.provider)?;
    let is_codex = action.provider.as_str() == "codex";
    let phase_started = Instant::now();

    let cloud_tail = action
        .pins
        .tail
        .cloud
        .clone()
        .ok_or_else(|| "conversation Pull action lacks its pinned cloud tail".to_string())?;
    let destination_provider_thread_id = action
        .keep_both
        .as_ref()
        .map(|keep_both| keep_both.remote_provider_thread_id.clone())
        .or_else(|| action.proposed_provider_thread_id.clone())
        .ok_or_else(|| "conversation Pull action lacks a destination provider id".to_string())?;
    let materialize_in_place = action.action == ConversationPlanActionType::MaterializeCloud
        && live_provider_thread_id == Some(destination_provider_thread_id.as_str());
    let materialize_relocated =
        action.action == ConversationPlanActionType::MaterializeCloud && !materialize_in_place;
    if action.action == ConversationPlanActionType::MaterializeCloud
        && !materialize_in_place
        && !is_codex
    {
        return Err("only Codex history dependencies can relocate a cloud update".to_string());
    }
    let relocation_source_provider_thread_id = action.source_resource_id.as_ref().and_then(|id| {
        let (source_provider, _) = id.as_str().split_once(':')?;
        (source_provider == action.provider.as_str())
            .then(|| provider_session_id_from_resource(id.as_str()))
            .flatten()
    });
    if materialize_relocated && relocation_source_provider_thread_id != live_provider_thread_id {
        return Err("conversation relocation source identity changed after review".to_string());
    }

    // Reconstruct and reidentify exactly once, verifying typed invariants
    // before any journal or visible mutation. Codex order per §6.2: staged
    // wire bytes resolve as a normal rollout → the wire chain refolds to the
    // accepted cloud pin → the Decode build runs the §4.4 paths gate on the
    // staged body **before** its identity stage assigns the reviewed provider
    // id, folds the head, and strips the path-map marker. Everything the
    // checkpoint and journal persist from this pull is sourced from the
    // correct form: the wire pin from the manifest, and the local twin,
    // prefix witness, and materialization digest pins from the decode
    // build's re-analysis of the final materialized local bytes.
    let (installed_bytes, imported_layout, checkpoint_seed) = if is_codex {
        // Operation-level Windows gate: the path-map root grammar is
        // checked before any rollout bytes are parsed.
        validate_pathmap_root("project root", &binding.project_root)?;
        let staging_dir = repository.root().join("staging").join("pull-conversations");
        fs::create_dir_all(&staging_dir)
            .map_err(|error| format!("create pull staging directory: {error}"))?;
        let staged_source = staging_dir.join(format!("{}.jsonl", action.action_id));
        fs::write(&staged_source, &prepared.cloud_transcript)
            .map_err(|error| format!("stage cloud transcript: {error}"))?;
        let no_bases = |_: &str| Ok(None);
        let resolved_root = codex_history_graph::resolve_codex_root(&staged_source, &no_bases)?;
        // The imported layout is the binding's local-form partition: the
        // coordinates come from the verified cloud value, the codec is the
        // local fold's.
        let imported_layout = RootTailLayout {
            codec: super::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
            partition_kind: cloud_tail.partition_kind,
            start_record_count: cloud_tail.start_record_count,
        };
        if cloud_tail.start_record_count > resolved_root.chain.len() as u64 {
            return Err("cloud tail start is outside the reconstructed conversation".to_string());
        }
        // Wire-form verification ends here: the staged wire chain must
        // refold to the pinned cloud tail.
        let refolded = super::domain::compute_conversation_tail_state(
            cloud_tail.partition_kind,
            cloud_tail.start_record_count,
            cloud_tail.accepted_end_record_count,
            &resolved_root.chain,
        )?;
        if refolded != cloud_tail.accepted_tail_state
            || cloud_tail.accepted_end_record_count != resolved_root.chain.len() as u64
        {
            return Err(
                "reconstructed conversation does not refold to the accepted cloud tail".to_string(),
            );
        }
        let artifact = build_standalone_codex_root(
            &resolved_root,
            &StandaloneBuildMode::Decode {
                root_provider_thread_id: destination_provider_thread_id.clone(),
                target_root: binding.project_root.clone(),
                canonical_target_root: (binding.canonical_project_root != binding.project_root)
                    .then(|| binding.canonical_project_root.clone()),
            },
            Some(&imported_layout),
        )?;
        let _ = fs::remove_file(&staged_source);
        if artifact.portable_tail.partition_kind != cloud_tail.partition_kind
            || artifact.portable_tail.start_record_count != cloud_tail.start_record_count
            || artifact.portable_tail.accepted_end_record_count
                != cloud_tail.accepted_end_record_count
        {
            return Err(
                "decoded conversation disagrees with the accepted cloud coordinates".to_string(),
            );
        }
        (
            artifact.root_bytes.clone(),
            imported_layout,
            (
                artifact.prefix_witness.clone(),
                artifact.artifact_sha256.clone(),
                artifact.portable_tail.clone(),
                artifact.lineage_digest.clone(),
                artifact.semantic_digest.clone(),
                artifact.last_canonical_message_id.clone(),
                artifact.last_synthetic_turn_id.clone(),
                artifact.next_synthetic_turn_ordinal,
            ),
        )
    } else {
        let rewritten = adapter.rewrite(&super::providers::SessionRewriteRequest {
            bytes: &prepared.cloud_transcript,
            expected_source_session_id: Some(&prepared.cloud_provider_thread_id),
            destination_session_id: &destination_provider_thread_id,
            expected_source_cwd: Some(&prepared.cloud_cwd),
            destination_cwd: &binding.project_root,
        })?;
        let layout = RootTailLayout {
            codec: cloud_tail.codec.clone(),
            partition_kind: cloud_tail.partition_kind,
            start_record_count: cloud_tail.start_record_count,
        };
        let sha = hex_digest(Sha256::digest(&rewritten).as_slice());
        let lineage_digest = action.pins.tail.lineage_digest.clone().ok_or_else(|| {
            "conversation Pull action lacks its pinned lineage digest".to_string()
        })?;
        let semantic_digest = action.pins.tail.semantic_digest.clone().ok_or_else(|| {
            "conversation Pull action lacks its pinned semantic digest".to_string()
        })?;
        (
            rewritten,
            layout,
            (
                None,
                sha,
                cloud_tail.clone(),
                lineage_digest,
                semantic_digest,
                None,
                None,
                0,
            ),
        )
    };
    let (
        prefix_witness,
        artifact_sha256,
        accepted_local_tail,
        staged_lineage_digest,
        staged_semantic_digest,
        last_canonical_message_id,
        last_synthetic_turn_id,
        next_synthetic_turn_ordinal,
    ) = checkpoint_seed;
    on_phase(
        "reconstruct and verify cloud graph",
        phase_started.elapsed(),
    );
    let phase_started = Instant::now();

    // Role assignment for the reviewed targets comes from the adapter's
    // declared index path. Non-Codex MaterializeCloud updates replace their
    // reviewed target; every Codex MaterializeCloud, CreateLocal, and
    // KeepBoth consumes a reviewed absent target.
    let index_path = (adapter.index_fragment_limit() > 0)
        .then(|| {
            resolved
                .home(&action.provider)
                .map(|home| Path::new(home).join("session_index.jsonl"))
        })
        .transpose()?;
    let is_index_target = |target: &ProviderStateExpectation| {
        index_path
            .as_ref()
            .is_some_and(|path| Path::new(&target.target_path) == path.as_path())
    };
    let replace_existing = materialize_in_place;
    let transcript_target = action
        .pins
        .target
        .provider_state
        .iter()
        .find(|target| {
            !is_index_target(target)
                && (replace_existing || target.expectation == TargetExpectation::Absent)
        })
        .cloned()
        .ok_or_else(|| "conversation Pull action lacks a transcript target".to_string())?;
    if materialize_relocated && transcript_target.expectation != TargetExpectation::Absent {
        return Err(
            "conversation relocation destination is not a reviewed absent target".to_string(),
        );
    }
    let mut targets = vec![ConversationTargetPayload {
        target_path: PathBuf::from(&transcript_target.target_path),
        role: ConversationFileRole::Transcript,
        expected: transcript_target.expectation.clone(),
        index_rows: vec![],
        bytes: installed_bytes.clone(),
    }];
    if let Some(index_target) = action
        .pins
        .target
        .provider_state
        .iter()
        .find(|target| is_index_target(target))
    {
        let index_path = PathBuf::from(&index_target.target_path);
        let mut index_specs = Vec::new();
        if let Some(keep_both) = &action.keep_both {
            // The preserved copy keeps the original provider session id; its
            // reviewed rename carries the plan-minted marker title. The id
            // comes from the revision-pinned bindings document, so a moved
            // binding forces a stale replan instead of renaming the wrong
            // session.
            let preserved_provider_thread_id = document_before
                .bindings
                .iter()
                .find(|row| {
                    row.host_id == binding.host_id
                        && row.replica_id == binding.replica_id
                        && row.local_project_id == binding.local_project_id
                        && row.provider_profile_id == action.pins.context.provider_profile_id
                        && row.thread_id == action.thread_id
                })
                .and_then(|row| row.provider_thread_id.clone())
                .ok_or_else(|| {
                    "conversation keep-both requires a bound provider session".to_string()
                })?;
            index_specs.push(CodexIndexRowSpec {
                provider_thread_id: preserved_provider_thread_id,
                thread_name: keep_both.preserved_display_title.clone(),
                updated_at: applied_at,
                replace_existing: true,
            });
        }
        // A real title is one a user could have set: unnamed captures
        // publish the provider thread ID as their display title.
        let cloud_title = prepared
            .cloud_display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| {
                !title.is_empty() && *title != prepared.cloud_provider_thread_id.as_str()
            });
        let cloud_title = if is_codex {
            cloud_title
                .map(|title| {
                    decode_codex_portable_display_title(
                        title,
                        &binding.project_root,
                        &binding.canonical_project_root,
                    )
                })
                .transpose()?
        } else {
            cloud_title.map(str::to_string)
        };
        if let Some(cloud_title) = cloud_title {
            index_specs.push(CodexIndexRowSpec {
                provider_thread_id: destination_provider_thread_id.clone(),
                thread_name: cloud_title,
                updated_at: prepared.cloud_updated_at,
                replace_existing,
            });
        }
        let index_rows = index_specs
            .iter()
            .map(|spec| spec.provider_thread_id.clone())
            .collect::<Vec<_>>();
        let (expected, bytes) = codex_index_payload(&index_path, &index_specs)?;
        if let Some(bytes) = bytes {
            targets.push(ConversationTargetPayload {
                target_path: index_path,
                role: ConversationFileRole::ProviderIndexFragment,
                expected,
                index_rows,
                bytes,
            });
        }
    }

    // The exact whole-document after-states this operation commits.
    let mut document_after = document_before.clone();
    let config_before = repository.load_config()?;
    let mut config_after = config_before.clone();
    let scope = SyncScopeKey::Provider(action.provider.clone());
    let retained_source_checkpoint = if materialize_relocated && is_codex {
        let old_provider_thread_id = live_provider_thread_id
            .ok_or_else(|| "conversation relocation lost its source provider id".to_string())?;
        let retained_thread_id = retained_conversation_thread_id(
            &binding.bundle_id,
            &action.provider,
            &action.thread_id,
            old_provider_thread_id,
        )?;
        if document_before.bindings.iter().any(|row| {
            row.host_id == binding.host_id
                && row.replica_id == binding.replica_id
                && row.local_project_id == binding.local_project_id
                && row.provider_profile_id == action.pins.context.provider_profile_id
                && row.thread_id == retained_thread_id
        }) {
            return Err(
                "conversation retained-source identity collides with an existing binding"
                    .to_string(),
            );
        }
        let mut checkpoint = config_before
            .links
            .iter()
            .find(|link| {
                link.local_project_id == binding.local_project_id
                    && link.storage_id == plan.storage_id
            })
            .and_then(|link| link.conversation_checkpoints.get(&scope))
            .and_then(|threads| threads.get(&action.thread_id))
            .cloned()
            .ok_or_else(|| {
                "conversation relocation lacks its reviewed source checkpoint".to_string()
            })?;
        let PhysicalMappingState::Ready {
            root_mapping,
            raw_range_pin,
        } = &mut checkpoint.physical
        else {
            return Err("conversation relocation source checkpoint is detached".to_string());
        };
        if root_mapping.provider_thread_id != old_provider_thread_id {
            return Err(
                "conversation relocation source checkpoint names another provider id".to_string(),
            );
        }
        let source_path = Path::new(&root_mapping.root_path);
        let source_bytes = codex_history_graph::read_rollout_bounded(source_path)?;
        let source_sha256 = hex_digest(Sha256::digest(&source_bytes).as_slice());
        if action.pins.target.local != TargetExpectation::Sha256(source_sha256.clone()) {
            return Err("conversation relocation source bytes changed after review".to_string());
        }
        if root_mapping.source_file_size != source_bytes.len() as u64
            || root_mapping.consumed_byte_offset > source_bytes.len() as u64
        {
            return Err(
                "conversation relocation source checkpoint disagrees with its file".to_string(),
            );
        }
        raw_range_pin.start_byte_offset = 0;
        raw_range_pin.end_byte_offset = root_mapping.consumed_byte_offset;
        raw_range_pin.range_sha256 = hex_digest(
            Sha256::digest(&source_bytes[..root_mapping.consumed_byte_offset as usize]).as_slice(),
        );
        checkpoint.accepted = None;
        checkpoint.accepted_local_tail = None;
        checkpoint.generation = checkpoint.generation.saturating_add(1);
        checkpoint.binding_revision = 0;
        Some((retained_thread_id, checkpoint))
    } else {
        None
    };
    let installed_mapping = |bytes: &[u8]| -> mallard_domain::RootPhysicalMapping {
        let consumed = bytes.len() as u64;
        let window = consumed.min(4_096);
        mallard_domain::RootPhysicalMapping {
            provider_thread_id: destination_provider_thread_id.clone(),
            root_path: transcript_target.target_path.clone(),
            source_file_size: consumed,
            consumed_byte_offset: consumed,
            consumed_record_ordinal: bytes.iter().filter(|byte| **byte == b'\n').count() as u64,
            probe_window_len: window,
            probe_window_sha256: hex_digest(
                Sha256::digest(&bytes[(consumed - window) as usize..]).as_slice(),
            ),
            last_canonical_message_id: last_canonical_message_id.clone(),
            last_synthetic_turn_id: last_synthetic_turn_id.clone(),
            next_synthetic_turn_ordinal,
        }
    };
    let accepted_checkpoint = ConversationCheckpoint {
        generation: 0,
        accepted: Some(mallard_domain::AcceptedConversationVersion {
            artifact_sha256: artifact_sha256.clone(),
            accepted_at_commit_id: action.pins.revision.commit_id.clone(),
            accepted_at_manifest_sha256: action.pins.revision.manifest_sha256.clone(),
            portable_tail: cloud_tail.clone(),
        }),
        accepted_local_tail: Some(accepted_local_tail.clone()),
        observed_tail: accepted_local_tail.clone(),
        physical: PhysicalMappingState::Ready {
            root_mapping: installed_mapping(&installed_bytes),
            raw_range_pin: RawRangePin {
                start_byte_offset: installed_bytes.len() as u64,
                end_byte_offset: installed_bytes.len() as u64,
                range_sha256: hex_digest(Sha256::digest([]).as_slice()),
            },
        },
        prefix_witness,
        binding_revision: 0,
    };
    let mut keep_both_journal = None;
    {
        let find_row = |document: &mut mallard_domain::ConversationBindings,
                        thread_id: &ThreadId|
         -> Option<usize> {
            document.bindings.iter().position(|row| {
                row.host_id == binding.host_id
                    && row.replica_id == binding.replica_id
                    && row.local_project_id == binding.local_project_id
                    && row.provider_profile_id == action.pins.context.provider_profile_id
                    && &row.thread_id == thread_id
            })
        };
        match (&action.action, &action.keep_both) {
            (ConversationPlanActionType::KeepBoth, Some(keep_both)) => {
                // The binding move: the existing alias and its unchanged
                // layout transfer to the preserved thread; the tracked
                // thread imports the verified remote layout under fresh
                // identities. No rollout byte changes.
                let index = find_row(&mut document_after, &action.thread_id).ok_or_else(|| {
                    "conversation keep-both requires an existing binding".to_string()
                })?;
                let row = &mut document_after.bindings[index];
                row.thread_id = keep_both.preserved_thread_id.clone();
                row.forked_from = None;
                row.preserved = Some(keep_both.preserved.clone());
                row.revision = row.revision.saturating_add(1);
                row.updated_at = applied_at;
                // The re-created origin row restores the published cloud
                // provenance: an origin that is itself a preserved copy
                // keeps its marker on every device.
                document_after.bindings.push(LocalConversationBinding {
                    host_id: binding.host_id.clone(),
                    replica_id: binding.replica_id.clone(),
                    local_project_id: binding.local_project_id.clone(),
                    provider_profile_id: action.pins.context.provider_profile_id.clone(),
                    thread_id: action.thread_id.clone(),
                    forked_from: None,
                    preserved: prepared.cloud_preserved.clone(),
                    root_tail_layout: Some(imported_layout.clone()),
                    provider_thread_id: Some(destination_provider_thread_id.clone()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    revision: 0,
                    created_at: applied_at,
                    updated_at: applied_at,
                });
                keep_both_journal = Some(KeepBothJournal {
                    preserved_thread_id: keep_both.preserved_thread_id.clone(),
                    remote_provider_thread_id: keep_both.remote_provider_thread_id.clone(),
                });
            }
            (ConversationPlanActionType::MaterializeCloud, None) => {
                let index = find_row(&mut document_after, &action.thread_id).ok_or_else(|| {
                    "conversation materialization requires an existing binding".to_string()
                })?;
                let row = &mut document_after.bindings[index];
                row.provider_thread_id = Some(destination_provider_thread_id.clone());
                row.root_tail_layout = Some(imported_layout.clone());
                row.materialization_state = ConversationMaterializationState::Resumable;
                row.revision = row.revision.saturating_add(1);
                row.updated_at = applied_at;
            }
            (ConversationPlanActionType::CreateLocal, None) => {
                document_after.bindings.push(LocalConversationBinding {
                    host_id: binding.host_id.clone(),
                    replica_id: binding.replica_id.clone(),
                    local_project_id: binding.local_project_id.clone(),
                    provider_profile_id: action.pins.context.provider_profile_id.clone(),
                    thread_id: action.thread_id.clone(),
                    forked_from: prepared.cloud_forked_from.clone(),
                    preserved: prepared.cloud_preserved.clone(),
                    root_tail_layout: Some(imported_layout.clone()),
                    provider_thread_id: Some(destination_provider_thread_id.clone()),
                    materialization_state: ConversationMaterializationState::Resumable,
                    revision: 0,
                    created_at: applied_at,
                    updated_at: applied_at,
                });
            }
            _ => {
                return Err(
                    "conversation Pull apply received a non-materializing action".to_string(),
                );
            }
        }
    }
    document_after.revision = document_after.revision.saturating_add(1);
    {
        let link = config_after
            .links
            .iter_mut()
            .find(|link| {
                link.local_project_id == binding.local_project_id
                    && link.storage_id == plan.storage_id
            })
            .ok_or_else(|| "conversation Pull requires a registered storage link".to_string())?;
        let threads = link
            .conversation_checkpoints
            .entry(scope.clone())
            .or_default();
        if let Some((retained_thread_id, retained_checkpoint)) = retained_source_checkpoint {
            if threads.contains_key(&retained_thread_id) {
                return Err("conversation retained-source checkpoint already exists".to_string());
            }
            threads.insert(retained_thread_id, retained_checkpoint);
        }
        if let Some(keep_both) = &action.keep_both {
            // Re-key the destination's checkpoint to the preserved thread as
            // local-only observed state: observed tail, mapping, and witness
            // survive; the accepted pin and accepted physical end clear.
            if let Some(mut moved) = threads.remove(&action.thread_id) {
                moved.accepted = None;
                moved.accepted_local_tail = None;
                moved.generation = moved.generation.saturating_add(1);
                moved.binding_revision = moved.binding_revision.saturating_add(1);
                threads.insert(keep_both.preserved_thread_id.clone(), moved);
            }
        }
        let generation = threads
            .get(&action.thread_id)
            .map(|existing| existing.generation.saturating_add(1))
            .unwrap_or(0);
        let mut checkpoint = accepted_checkpoint.clone();
        checkpoint.generation = generation;
        threads.insert(action.thread_id.clone(), checkpoint);
        link.scope_bases.entry(scope.clone()).and_modify(|base| {
            base.scope_revision = action.pins.revision.scope_revision.max(base.scope_revision);
            base.scope_sha256 = action.pins.revision.scope_sha256.clone();
            base.reviewed_in = RepositoryBase {
                revision: action.pins.revision.bundle_revision,
                commit_id: action.pins.revision.commit_id.clone(),
                manifest_sha256: action.pins.revision.manifest_sha256.clone(),
                manifest_size: base.reviewed_in.manifest_size.max(1),
            };
            base.last_pull_at = Some(applied_at);
        });
    }
    config_after.revision = config_after.revision.saturating_add(1);
    on_phase("journal document after-states", phase_started.elapsed());

    let operation = ConversationSyncOperation {
        operation_id: ConversationOperationId::parse(generated_named_id("operation")?)?,
        plan_id: Some(plan.plan_id.clone()),
        plan_action_id: Some(action.action_id.clone()),
        cloud: ConversationCloudAnchor {
            storage_id: plan.storage_id.clone(),
            bundle_id: plan.bundle_id.clone(),
            revision: action.pins.revision.bundle_revision,
            commit_id: action.pins.revision.commit_id.clone(),
            manifest_sha256: action.pins.revision.manifest_sha256.clone(),
        },
        host_id: binding.host_id.clone(),
        replica_id: binding.replica_id.clone(),
        local_project_id: binding.local_project_id.clone(),
        provider_profile_id: action.pins.context.provider_profile_id.clone(),
        provider: action.provider.clone(),
        thread_id: action.thread_id.clone(),
        operation: ConversationSyncOperationKind::ProviderMaterialization(
            ConversationMaterializationOperation {
                phase: PullJournalPhase::TargetStaged,
                planned_relation: action.pins.tail.classification()?,
                accepted_tail_after: cloud_tail,
                // The engine's verify_provider_payload hashes the staged
                // **local** target bytes, so these pins are the decode
                // build's local digests (§3.2 form ledger); the manifest's
                // wire digests were verified at prepare.
                pinned_lineage_digest: staged_lineage_digest,
                pinned_semantic_digest: staged_semantic_digest,
                keep_both: keep_both_journal,
                sync_config_before: config_before,
                sync_config_after: config_after,
                bindings_before: document_before,
                bindings_after: document_after,
            },
        ),
        provider_thread_id: Some(destination_provider_thread_id),
        materialization_state: ConversationMaterializationState::Resumable,
        expected_host_identity_revision: action.pins.context.host_identity_revision,
        // Every action was validated against the reviewed document before
        // the batch began. Earlier approved actions in this same Pull may
        // have advanced that shared document, so journal this action against
        // the exact rolling revision it is about to replace.
        expected_bindings_document_revision,
        expected_binding_revision: action.pins.context.binding_revision,
        expected_profile_revision: action.pins.context.profile_revision,
        expected_project_binding_revision: action.pins.context.project_binding_revision,
        targets: Vec::new(),
        created_at: applied_at,
        updated_at: applied_at,
    };
    let mut failpoints = NoConversationFailpoints;
    let receipt = apply_conversation_materialization_locked(
        repository,
        ConversationMaterializationRequest { operation, targets },
        &mut failpoints,
    )?;
    let action_type = match action.action {
        ConversationPlanActionType::KeepBoth => RestoreActionType::MaterializeConversation,
        _ => RestoreActionType::MaterializeConversation,
    };
    Ok(ApplyReceipt {
        action_id: action.action_id.clone(),
        resource_id: conversation_resource_id(&action.provider, &action.thread_id)?,
        action_type,
        logical_path: None,
        source_sha256: Some(prepared.cloud_file_sha256.clone()),
        target_path: Some(transcript_target.target_path.clone()),
        target_sha256_after: receipt.local_file_sha256_after.clone(),
        status: ActionStatus::Applied,
        applied_at,
        error: None,
        conversation: Some(receipt),
    })
}

pub(super) fn apply_bundle_restore_with_progress(
    repository: &LocalStateRepository,
    plan_id: &PlanId,
    approved_action_ids: Vec<ActionId>,
    on_progress: &mut dyn FnMut(FetchProgress),
    on_phase: &mut dyn FnMut(&str, Duration),
) -> Result<RestoreResult, String> {
    let phase_started = Instant::now();
    let _host_operation = repository.acquire_host_operation()?;
    let plan = repository.load_restore_plan(plan_id)?;
    let resolved = current_binding_for_restore_plan(repository, &plan)?;
    let binding = resolved.binding.clone();
    let (mut approved, mut approved_conversation_actions) =
        split_approved_restore_actions(&approved_action_ids, &plan)?;
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
            || record.revision != plan.revision
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
    let applied_at = now_secs();
    on_phase("validate reviewed plan", phase_started.elapsed());
    let phase_started = Instant::now();
    let mut byte_source = engine.byte_source(&snapshot, Some(object_cache()), Some(on_progress));
    // Metadata-only adoptions never download or journal: each is one atomic
    // whole-document SyncConfig replacement.
    let (metadata_actions, materializing_actions): (Vec<_>, Vec<_>) = approved_conversation_actions
        .into_iter()
        .partition(|selected| {
            selected.action.action == ConversationPlanActionType::AdoptCloudMetadata
        });
    let mut conversation_receipts = Vec::new();
    for selected in metadata_actions {
        match adopt_cloud_metadata_action(repository, &plan, &selected.action, applied_at) {
            Ok(receipt) => conversation_receipts.push(receipt),
            Err(message) => conversation_receipts.push(failed_conversation_receipt(
                &selected.action,
                message,
                applied_at,
            )?),
        }
    }
    let (prepared_conversations, prepare_failures) = prepare_pull_conversation_actions(
        repository,
        &resolved,
        &snapshot,
        &mut byte_source,
        &materializing_actions,
        &plan.storage_id,
        applied_at,
    )?;
    on_phase("prepare conversations", phase_started.elapsed());
    for failure in prepare_failures {
        conversation_receipts.push(failed_conversation_receipt(
            &failure.action,
            failure.message,
            applied_at,
        )?);
    }
    // Threads settle independently. Successful actions advance the shared
    // bindings document, so the next already-reviewed action chains from the
    // revision this batch just committed instead of invalidating itself.
    let mut expected_bindings_document_revision = prepared_conversations
        .first()
        .map(|prepared| prepared.action.pins.context.bindings_document_revision);
    for prepared in prepared_conversations {
        let action = prepared.action.clone();
        let resource = action
            .source_resource_id
            .clone()
            .or_else(|| conversation_resource_id(&action.provider, &action.thread_id).ok())
            .map(|resource_id| resource_id.to_string())
            .unwrap_or_else(|| action.thread_id.to_string());
        let phase_label = format!("apply conversation {resource}");
        let phase_started = Instant::now();
        let expected_revision = expected_bindings_document_revision
            .unwrap_or(action.pins.context.bindings_document_revision);
        match apply_prepared_pull_conversation(
            repository,
            &plan,
            &resolved,
            prepared,
            expected_revision,
            applied_at,
            on_phase,
        ) {
            Ok(receipt) => {
                conversation_receipts.push(receipt);
                expected_bindings_document_revision =
                    Some(repository.load_conversation_bindings()?.revision);
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
        &resolved.homes,
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
    let files_complete = reviewed_files_complete(&applied.receipts, &BTreeSet::new());
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
        revision: plan.revision,
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
                || current.revision != record.revision
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
            let _ = project;
            {
                let link = config
                    .links
                    .iter_mut()
                    .find(|link| {
                        link.local_project_id == binding.local_project_id
                            && link.storage_id == plan.storage_id
                    })
                    .ok_or_else(|| {
                        "project storage link was removed while applying restore".to_string()
                    })?;
                let reviewed_in = RepositoryBase {
                    revision: plan.revision,
                    commit_id: plan.commit_id.clone(),
                    manifest_sha256: plan.manifest_sha256.clone(),
                    manifest_size: snapshot.head.manifest_size,
                };
                // Pull advances only the approved bound scopes: the project
                // scope plus each bound provider's scope, exactly as the
                // review presented them. Unbound and unknown scopes never
                // move.
                for scope in writable_scopes_for_binding(&binding) {
                    let previous = link.scope_bases.get(&scope).cloned();
                    let last_push_at = previous.as_ref().and_then(|base| base.last_push_at);
                    let previous_pull_at = previous.as_ref().and_then(|base| base.last_pull_at);
                    match snapshot.manifest.scope(&scope) {
                        Some(remote) => {
                            link.scope_bases.insert(
                                scope.clone(),
                                ScopeBase {
                                    scope: scope.clone(),
                                    scope_revision: remote.revision,
                                    scope_sha256: remote.content_sha256.clone(),
                                    reviewed_in: reviewed_in.clone(),
                                    binding_revision: binding.revision,
                                    last_pull_at: successful_pull_timestamp(
                                        previous_pull_at,
                                        applied_at,
                                        overall_success,
                                    ),
                                    last_push_at,
                                },
                            );
                        }
                        None => {
                            link.scope_bases.remove(&scope);
                        }
                    }
                }
            }
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
            link.recipe = Some(machine_local_recipe_from_manifest(&snapshot.manifest)?);
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
    let preserved_threads = applied
        .receipts
        .iter()
        .filter_map(|receipt| {
            receipt.conversation.as_ref().and_then(|conversation| {
                conversation.preserved_thread_id.as_ref().map(|preserved| {
                    // The marker title lives only in the plan's keep-both
                    // record; a receipt without its action is a
                    // plan/receipt mismatch, not a displayable result.
                    let preserved_display_title = plan
                        .conversation_actions
                        .iter()
                        .find(|action| action.action_id == receipt.action_id)
                        .and_then(|action| action.keep_both.as_ref())
                        .map(|keep_both| keep_both.preserved_display_title.clone())
                        .ok_or_else(|| {
                            "applied keep-both receipt has no matching plan action".to_string()
                        })?;
                    Ok(PreservedThreadResult {
                        action_id: receipt.action_id.clone(),
                        original_thread_id: conversation.thread_id.clone(),
                        preserved_thread_id: preserved.clone(),
                        preserved_display_title,
                    })
                })
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let success = overall_success;
    let message = if success {
        format!("Applied {} restore actions", applied_action_ids.len())
    } else {
        format!(
            "Applied {} actions; {} require attention",
            applied_action_ids.len(),
            failed_actions.len()
        )
    };
    let blocked_operations = repository
        .load_conversation_sync_journal()?
        .operations
        .iter()
        .any(|operation| operation.is_blocked());
    let result = RestoreResult {
        success,
        status: if blocked_operations {
            RestoreApplyStatus::RecoveryRequired
        } else {
            RestoreApplyStatus::Complete
        },
        message,
        plan_id: Some(plan.plan_id),
        reviewed_base_advanced: Some(files_complete),
        applied_action_ids,
        failed_actions,
        preserved_threads,
        // Every committed conversation materialization leaves a running
        // Codex app unaware of the thread until its next thread/list rescan.
        codex_app_refresh: applied
            .receipts
            .iter()
            .find_map(|receipt| receipt.conversation.as_ref())
            .and_then(|receipt| {
                codex_app_refresh_offer(
                    receipt.journal_operation_id.clone(),
                    resolved.homes.get(&codex_key()).map(Path::new),
                )
            }),
    };
    on_phase("persist receipts and finalize", phase_started.elapsed());
    Ok(result)
}

/// Apply one metadata-only adoption: accept the verified cloud value into
/// the destination checkpoint and advance the scope base, as a single
/// journal-free atomic `SyncConfig` replacement.
fn adopt_cloud_metadata_action(
    repository: &LocalStateRepository,
    plan: &RestorePlan,
    action: &ConversationPlanAction,
    applied_at: u64,
) -> Result<ApplyReceipt, String> {
    let cloud_tail = action
        .pins
        .tail
        .cloud
        .clone()
        .ok_or_else(|| "conversation adoption lacks its pinned cloud tail".to_string())?;
    // The local column accepts the reviewed local observation: adoption runs
    // only on Sync relations, where classification proved it equivalent to
    // the accepted cloud value in its own form.
    let accepted_local_tail = action
        .pins
        .tail
        .local
        .clone()
        .ok_or_else(|| "conversation adoption lacks its pinned local tail".to_string())?;
    let scope = SyncScopeKey::Provider(action.provider.clone());
    let mut config = repository.load_config()?;
    {
        let link = config
            .links
            .iter_mut()
            .find(|link| {
                link.local_project_id == action.pins.context.local_project_id
                    && link.storage_id == plan.storage_id
            })
            .ok_or_else(|| {
                "conversation adoption requires a registered storage link".to_string()
            })?;
        let threads = link
            .conversation_checkpoints
            .entry(scope.clone())
            .or_default();
        let checkpoint = threads.get_mut(&action.thread_id).ok_or_else(|| {
            "conversation adoption requires an existing destination checkpoint".to_string()
        })?;
        if Some(checkpoint.generation) != action.pins.tail.checkpoint_generation {
            return Err(
                "conversation adoption checkpoint advanced after review; refresh and pull again"
                    .to_string(),
            );
        }
        let accepted = mallard_domain::AcceptedConversationVersion {
            artifact_sha256: checkpoint
                .accepted
                .as_ref()
                .map(|accepted| accepted.artifact_sha256.clone())
                .unwrap_or_else(|| hex_digest(Sha256::digest([]).as_slice())),
            accepted_at_commit_id: action.pins.revision.commit_id.clone(),
            accepted_at_manifest_sha256: action.pins.revision.manifest_sha256.clone(),
            portable_tail: cloud_tail.clone(),
        };
        checkpoint.accepted = Some(accepted);
        checkpoint.accepted_local_tail = Some(accepted_local_tail.clone());
        checkpoint.observed_tail = accepted_local_tail;
        checkpoint.generation = checkpoint.generation.saturating_add(1);
        link.scope_bases.entry(scope).and_modify(|base| {
            base.scope_revision = action.pins.revision.scope_revision.max(base.scope_revision);
            base.scope_sha256 = action.pins.revision.scope_sha256.clone();
            base.reviewed_in = RepositoryBase {
                revision: action.pins.revision.bundle_revision,
                commit_id: action.pins.revision.commit_id.clone(),
                manifest_sha256: action.pins.revision.manifest_sha256.clone(),
                manifest_size: base.reviewed_in.manifest_size.max(1),
            };
            base.last_pull_at = Some(applied_at);
        });
    }
    config.revision = config.revision.saturating_add(1);
    repository.install_config_snapshot(config)?;
    Ok(ApplyReceipt {
        action_id: action.action_id.clone(),
        resource_id: conversation_resource_id(&action.provider, &action.thread_id)?,
        action_type: RestoreActionType::MaterializeConversation,
        logical_path: None,
        source_sha256: None,
        target_path: None,
        target_sha256_after: None,
        status: ActionStatus::Applied,
        applied_at,
        error: None,
        conversation: None,
    })
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

fn successful_pull_timestamp(
    previous: Option<u64>,
    applied_at: u64,
    overall_success: bool,
) -> Option<u64> {
    overall_success.then_some(applied_at).or(previous)
}

/// Load the exact storage snapshot approved by a RestorePlan. Dependency and
/// readiness support must never discover a different linked storage on their
/// own, because each storage advances its bundle revisions independently.
/// Load the plan's binding and re-inspect the bundle head it pins. This is
/// deliberately metadata-only — supporting views (dependencies, readiness)
/// must never download payload objects.
/// Whether this apply may advance the reviewed recipe base to its plan's
/// revision. Declining a project-content decision (`Skipped`) settles the
/// review, a `Manual` keep-local receipt is itself the decision, and a
/// declined conversation materialization is the Claude spelling of an
/// unapproved typed Codex action (which never produces a receipt): the
/// thread stays in storage, Push carries it forward by reference, and the
/// next Pull re-offers it. Only a skipped `WriteFile`/`MergeFile` leaves
/// reviewed generic content behind that the baseline must not move past.
/// An empty receipt set is a completed review of nothing: the
/// acknowledge-only Pull that advances the baseline without materializing
/// anything.
fn reviewed_files_complete(
    receipts: &[ApplyReceipt],
    kept_local_conversation_actions: &BTreeSet<ActionId>,
) -> bool {
    receipts.iter().all(|receipt| match receipt.action_type {
        RestoreActionType::WriteProjectFile
        | RestoreActionType::EnsureProjectDirectory
        | RestoreActionType::DeleteProjectFile
        | RestoreActionType::DeleteProjectDirectory
        | RestoreActionType::MaterializeConversation
        | RestoreActionType::PublishConversation => {
            matches!(
                receipt.status,
                ActionStatus::Applied | ActionStatus::Skipped
            )
        }
        RestoreActionType::WriteFile | RestoreActionType::MergeFile => {
            receipt.status == ActionStatus::Applied
        }
        RestoreActionType::Manual => {
            receipt.status == ActionStatus::Applied
                || (receipt.status == ActionStatus::Skipped
                    && kept_local_conversation_actions.contains(&receipt.action_id))
        }
    })
}

fn restore_support_context(
    repository: &LocalStateRepository,
    restore_plan_id: &PlanId,
) -> Result<(RestorePlan, ResolvedBinding, BundleSnapshot), String> {
    let plan = repository.load_restore_plan(restore_plan_id)?;
    let now = now_secs();
    if now < plan.created_at || now > plan.expires_at {
        return Err("restore plan has expired or is not active yet".to_string());
    }
    let resolved = current_binding_for_restore_plan(repository, &plan)?;
    let (_, snapshot) = inspect_from_storage(repository, &plan.storage_id, &plan.bundle_id)?;
    let head = &snapshot.head;
    let binding = &resolved.binding;
    if plan.storage_id != snapshot.storage_id
        || plan.bundle_id != head.bundle_id
        || plan.host_id != binding.host_id
        || plan.replica_id != binding.replica_id
        || plan.revision != head.revision
        || plan.commit_id != head.commit_id
        || plan.manifest_sha256 != head.manifest_sha256
        || plan.binding_revision != binding.revision
    {
        return Err(
            "bundle head changed after restore planning; refresh the Pull review".to_string(),
        );
    }
    Ok((plan, resolved, snapshot))
}

fn get_bundle_readiness_with_repository(
    repository: &LocalStateRepository,
    storage_id: &StorageId,
    bundle_id: &BundleId,
    binding: &ProjectBinding,
) -> Result<BundleReadiness, String> {
    let resolved = require_current_binding(repository, binding)?;
    if &resolved.binding.bundle_id != bundle_id {
        return Err("binding and requested bundle IDs differ".to_string());
    }
    let config = repository.load_config()?;
    let project = config
        .project(&resolved.binding.local_project_id)
        .ok_or_else(|| "bound project is no longer registered".to_string())?;
    require_project_link(&config, project, storage_id)?;
    let (_, snapshot) = inspect_from_storage(repository, storage_id, bundle_id)?;
    bundle_readiness_for_snapshot(repository, &resolved, &snapshot)
}

fn get_restore_readiness_with_repository(
    repository: &LocalStateRepository,
    restore_plan_id: &PlanId,
) -> Result<BundleReadiness, String> {
    let (_, resolved, snapshot) = restore_support_context(repository, restore_plan_id)?;
    bundle_readiness_for_snapshot(repository, &resolved, &snapshot)
}

fn bundle_readiness_for_snapshot(
    repository: &LocalStateRepository,
    resolved: &ResolvedBinding,
    snapshot: &BundleSnapshot,
) -> Result<BundleReadiness, String> {
    let binding = &resolved.binding;
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
                && record.revision == head.revision
                && record.manifest_sha256 == head.manifest_sha256
                && record.binding_revision == binding.revision
                && record.status != MaterializationStatus::Detached
        });
    let mut issues = Vec::new();
    // Provider-home readiness is informational per provider scope: an
    // unbound provider's remote sessions stay view-only rather than
    // blocking anything.
    for (provider, scope) in snapshot.manifest.provider_scopes() {
        let needs_home = !scope.conversations.is_empty() || !scope.resources.is_empty();
        add_provider_home_issue(
            &mut issues,
            provider.clone(),
            needs_home,
            resolved.homes.get(provider).map(String::as_str),
        );
    }
    if snapshot.manifest.all_files().next().is_some() {
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
        .filter(|receipt| {
            snapshot
                .manifest
                .find_resource(&receipt.resource_id)
                .is_some_and(|(_, resource)| resource.kind == ResourceKind::Conversation)
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
            .and_then(|resource_id| snapshot.manifest.find_resource(resource_id))
            .and_then(|(_, resource)| resource.provider.clone());
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
                return !manual_conversations.contains(&receipt.resource_id);
            }
            match receipt.action_type {
                RestoreActionType::WriteFile
                | RestoreActionType::MergeFile
                | RestoreActionType::MaterializeConversation
                | RestoreActionType::PublishConversation => true,
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

/// One active binding together with its resolved provider homes. Machine
/// state persists profile IDs; commands resolve homes per operation through
/// the profile catalog and never store them on the binding.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedBinding {
    pub binding: ProjectBinding,
    pub homes: BTreeMap<ProviderKey, String>,
}

impl ResolvedBinding {
    fn home(&self, provider: &ProviderKey) -> Result<&str, String> {
        self.homes.get(provider).map(String::as_str).ok_or_else(|| {
            format!(
                "provider '{}' is not bound with a readable profile on this machine",
                provider
            )
        })
    }
}

/// The repository-level reviewed base for one project/storage link: the
/// newest manifest coordinates any scope base was reviewed in. Global
/// history continuity is classified against this; per-scope continuity is
/// classified against each scope base separately.
fn repository_base_from_link(link: &ProjectStorageLink) -> Option<RepositoryBase> {
    link.scope_bases
        .values()
        .map(|base| base.reviewed_in.clone())
        .max_by_key(|reviewed| reviewed.revision)
}

fn project_link<'a>(
    config: &'a SyncConfig,
    local_project_id: &LocalProjectId,
    storage_id: &StorageId,
) -> Option<&'a ProjectStorageLink> {
    config
        .links
        .iter()
        .find(|link| &link.local_project_id == local_project_id && &link.storage_id == storage_id)
}

fn require_current_binding(
    repository: &LocalStateRepository,
    supplied: &ProjectBinding,
) -> Result<ResolvedBinding, String> {
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
) -> Result<ResolvedBinding, String> {
    let homes = resolve_profile_paths(repository, &binding.profile_ids)?;
    let resolved = ResolvedBinding {
        binding: binding.clone(),
        homes,
    };
    resolved.binding.validate_structure()?;
    Ok(resolved)
}

pub(super) fn rebaseline_project_provider_caches(
    repository: &LocalStateRepository,
    local_project_id: &LocalProjectId,
) -> Result<(), String> {
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
    let resolved = resolve_project_binding(repository, &binding)?;
    for home in resolved.homes.values() {
        super::codex_watch_service::session_watch_service().force_rebaseline(Path::new(home));
    }
    Ok(())
}

fn current_binding_for_restore_plan(
    repository: &LocalStateRepository,
    plan: &RestorePlan,
) -> Result<ResolvedBinding, String> {
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

fn project_open_commands(resolved: &ResolvedBinding) -> Vec<(&'static str, String)> {
    let project_root = shell_quote(&resolved.binding.project_root);
    let mut commands = Vec::new();
    if let Some(codex_home) = resolved.homes.get(&codex_key()) {
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
        if let Some(command) = super::providers::codex::codex_app_project_open_command(
            Path::new(codex_home),
            Path::new(&resolved.binding.project_root),
        ) {
            commands.push(("Codex app — new", command));
        }
    }
    if let Some(claude_home) = resolved
        .homes
        .get(&ProviderKey::parse("claude").expect("static provider key"))
    {
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
    approved_action_ids: &[ActionId],
    plan: &RestorePlan,
) -> Result<(BTreeSet<ActionId>, Vec<SelectedPullConversation>), String> {
    let mut approved = BTreeSet::new();
    for action_id in approved_action_ids {
        if !approved.insert(action_id.clone()) {
            return Err(format!(
                "restore action '{}' was approved more than once",
                action_id
            ));
        }
    }
    let plan_action_ids = plan
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .chain(
            plan.conversation_actions
                .iter()
                .map(|action| action.action_id.clone()),
        )
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = approved
        .iter()
        .find(|action_id| !plan_action_ids.contains(*action_id))
    {
        return Err(format!(
            "restore action '{}' is not in the reviewed plan",
            unknown
        ));
    }
    let mut selected_conversations = Vec::new();
    for action in &plan.conversation_actions {
        if !approved.contains(&action.action_id) {
            continue;
        }
        if !is_selectable_pull_conversation_action(action) {
            return Err(format!(
                "conversation Pull action '{}' cannot be selected",
                action.action_id
            ));
        }
        approved.remove(&action.action_id);
        selected_conversations.push(SelectedPullConversation {
            action: action.clone(),
        });
    }
    Ok((approved, selected_conversations))
}

fn add_provider_home_issue(
    issues: &mut Vec<BundleReadinessIssue>,
    provider: ProviderKey,
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
    let name = provider_name(&provider);
    issues.push(BundleReadinessIssue {
        issue_id: format!("provider-home-{}", provider),
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
    let resolved = resolve_project_binding(repository, binding)?;
    capture_request_for_resolved_binding(repository, &resolved)
}

fn capture_request_for_resolved_binding(
    repository: &LocalStateRepository,
    resolved: &ResolvedBinding,
) -> Result<CaptureRequest, String> {
    let binding = &resolved.binding;
    let resolved_project = fs_canonicalize(Path::new(&binding.project_root))?;
    if resolved_project != binding.canonical_project_root {
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
        provider_homes: resolved
            .homes
            .iter()
            .map(|(provider, home)| (provider.clone(), PathBuf::from(home)))
            .collect(),
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
    for home in resolved.homes.values() {
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
    let canonical_github = project_activity::normalize_github_repository_url(value);
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

pub(super) fn storage_engine(
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
    BundleEngine::open(
        ConfiguredObjectStore::from_config(storage)?,
        storage.id.clone(),
    )
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
                    provider_name(&profile.provider),
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
    recipe.validate_machine_local()?;
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
        let bundle_id = config
            .project(&request.local_project_id)
            .ok_or_else(|| format!("unknown local project '{}'", request.local_project_id))?
            .bundle_id
            .clone();
        if !config
            .storages
            .iter()
            .any(|storage| storage.id == request.storage_id)
        {
            return Err(format!("unknown storage '{}'", request.storage_id));
        }
        if let Some(existing) = config.links.iter_mut().find(|existing| {
            existing.local_project_id == request.local_project_id
                && existing.storage_id == request.storage_id
        }) {
            // The request edits only user intent. Destination-specific review
            // state belongs to the existing link and is dropped only by unlink.
            existing.pinned = request.pinned;
            return Ok(existing.clone());
        }
        let link = ProjectStorageLink {
            local_project_id: request.local_project_id.clone(),
            storage_id: request.storage_id.clone(),
            bundle_id,
            recipe: None,
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::new(),
            pinned: request.pinned,
            created_at: now_secs(),
        };
        config.links.push(link.clone());
        Ok(link)
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
    if false {
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

    let remote_recipe = machine_local_recipe_from_manifest(&remote.manifest)?;
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
                scope_bases: BTreeMap::new(),
                project_content_preferences: Default::default(),
                conversation_checkpoints: BTreeMap::new(),
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
            canonical_project_root: validated.clone(),
            profile_ids: request.profile_ids.clone(),
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

/// First active binding (excluding `exclude`) that already claims one of the
/// requested provider profiles for this checkout. Project identity is
/// (canonical root, provider profile): the same folder with a different
/// config is a separate project, only a shared profile is a collision.
fn root_profile_collision<'a>(
    machine: &'a MachineProjectState,
    exclude: Option<&LocalProjectId>,
    canonical_root: &str,
    profile_ids: &BTreeMap<ProviderKey, LocalProviderProfileId>,
) -> Option<(&'a ProjectBinding, ProviderKey)> {
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
                    .then_some((binding, provider.clone()))
            })
        })
}

/// Select the one current project identity that setup is replacing.
///
/// Removing a project deliberately detaches its machine-local replica instead
/// of erasing conversation identity.  Re-registering that exact
/// (checkout, bundle, provider profiles) tuple must therefore reactivate the
/// detached replica; minting a second replica would make the retained provider
/// thread bindings look like a second owner.
fn detached_binding_for_reregistration(
    machine: &MachineProjectState,
    config: &SyncConfig,
    host_id: &HostId,
    bundle_id: &BundleId,
    canonical_root: &str,
    profile_ids: &BTreeMap<ProviderKey, LocalProviderProfileId>,
) -> Result<Option<ProjectBinding>, String> {
    let matches = machine
        .bindings
        .iter()
        .filter(|binding| binding.state == BindingState::Detached)
        .filter(|binding| &binding.host_id == host_id)
        .filter(|binding| &binding.bundle_id == bundle_id)
        .filter(|binding| {
            binding
                .canonical_project_root
                .eq_ignore_ascii_case(canonical_root)
        })
        .filter(|binding| &binding.profile_ids == profile_ids)
        .filter(|binding| config.project(&binding.local_project_id).is_none())
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [binding] => Ok(Some(binding.clone())),
        _ => Err(
            "multiple detached registrations match this project; repair local project state before finishing setup"
                .to_string(),
        ),
    }
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

/// Validate a binding request and return the canonical project root.
fn validate_binding_request(
    repository: &LocalStateRepository,
    request: &SaveProjectBindingRequest,
) -> Result<String, String> {
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
        if paths_overlap(&canonical, storage_path) {
            return Err(format!(
                "project root '{}' overlaps local storage '{}'",
                canonical.display(),
                storage_name
            ));
        }
    }
    let mut provider_roots: Vec<PathBuf> = Vec::new();
    for (provider, path) in &profile_paths {
        let label = format!("{} profile", provider_name(provider));
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
            return Err("selected provider homes must not overlap".to_string());
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
            provider_name(&provider)
        ));
    }
    Ok(canonical.to_string_lossy().into_owned())
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
    pub can_finalize: bool,
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
    run_blocking(move || {
        ProjectSyncService::new(&repository).create_setup_draft(Path::new(&project_root))
    })
    .await
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
    run_blocking(move || ProjectSyncService::new(&repository).update_setup_draft(draft)).await
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
    run_blocking(move || ProjectSyncService::new(&repository).inspect_setup_draft(&draft_id)).await
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
        ProjectSyncService::new(&repository).finalize_project_setup(&draft_id, expected_revision)
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

pub(super) fn create_setup_draft_with_repository(
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
        .filter(|profile| profile.provider == codex_key())
        .filter(|profile| !claimed_profiles.contains(&profile.profile_id));
    if let (Some(only), None) = (codex_candidates.next(), codex_candidates.next()) {
        profiles.insert(
            codex_key(),
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

pub(super) fn update_setup_draft_with_repository(
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
    paths: BTreeMap<ProviderKey, String>,
    profile_ids: BTreeMap<ProviderKey, LocalProviderProfileId>,
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
                        provider_name(provider),
                        provider_name(&profile.provider),
                        profile.display_name
                    ));
                }
                let (available, readable, _, error) = inspect_provider_profile(profile);
                if !available || !readable {
                    return Err(error.unwrap_or_else(|| {
                        format!(
                            "{} profile '{}' is not readable",
                            provider_name(provider),
                            profile.path
                        )
                    }));
                }
                resolved
                    .paths
                    .insert(provider.clone(), profile.path.clone());
                resolved
                    .profile_ids
                    .insert(provider.clone(), profile.profile_id.clone());
            }
            DraftProfileSelection::Pending { path, display_name } => {
                let probe = probe_provider_profile_with_repository(repository, provider, path)?;
                if !probe.readable {
                    return Err(format!(
                        "{} profile '{}' is not readable",
                        provider_name(provider),
                        probe.resolved_path
                    ));
                }
                if let Some(profile_id) = probe.existing_profile_id {
                    resolved.paths.insert(provider.clone(), probe.resolved_path);
                    resolved.profile_ids.insert(provider.clone(), profile_id);
                    continue;
                }
                let profile = ProviderProfile {
                    profile_id: LocalProviderProfileId::parse(generated_named_id("profile")?)?,
                    provider: provider.clone(),
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
                resolved.paths.insert(provider.clone(), probe.resolved_path);
                resolved
                    .profile_ids
                    .insert(provider.clone(), profile.profile_id.clone());
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

pub(super) fn inspect_setup_draft_with_repository(
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

    // Profiles and their isolation from the project and Mallard-owned paths.
    match resolve_draft_profiles(repository, &draft) {
        Ok(resolved) => {
            match validate_setup_paths(repository, &draft.project_root, &resolved.paths) {
                Ok(_) => {
                    sections.push(SetupSectionStatus {
                        section: "profiles".to_string(),
                        state: "ready".to_string(),
                        message: None,
                    });
                }
                Err(error) => {
                    blocked = true;
                    sections.push(SetupSectionStatus {
                        section: "profiles".to_string(),
                        state: "blocked".to_string(),
                        message: Some(error),
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
    })
}

fn build_setup_transaction(
    repository: &LocalStateRepository,
    draft: &ProjectSetupDraft,
) -> Result<SetupTransaction, String> {
    let host_id = repository.load_or_create_host_identity()?.host_id;
    let canonical = fs_canonicalize(Path::new(&draft.project_root))?;
    if canonical.to_string_lossy() != draft.canonical_project_root {
        return Err(
            "the project folder moved since this draft was saved; choose it again".to_string(),
        );
    }
    let resolved_profiles = resolve_draft_profiles(repository, draft)?;
    validate_setup_paths(repository, &draft.project_root, &resolved_profiles.paths)?;
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
            provider_name(&provider)
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
        DraftRepositoryChoice::New => (
            draft.new_bundle_id.clone(),
            draft.display_name.clone(),
            local_fingerprint,
            BundleRecipe::default(),
        ),
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
            let machine_local_recipe = machine_local_recipe_from_manifest(&snapshot.manifest)?;
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
                machine_local_recipe,
            )
        }
    };

    let config = repository.load_config()?;
    let detached = detached_binding_for_reregistration(
        &machine,
        &config,
        &host_id,
        &bundle_id,
        &draft.canonical_project_root,
        &resolved_profiles.profile_ids,
    )?;
    let local_project_id = detached
        .as_ref()
        .map(|binding| binding.local_project_id.clone())
        .unwrap_or_else(|| draft.local_project_id.clone());
    let now = now_secs();
    let project = LocalProjectRegistration {
        local_project_id: local_project_id.clone(),
        bundle_id: bundle_id.clone(),
        display_name,
        local_alias: None,
        repository_fingerprint: fingerprint,
        recipe,
        revision: 0,
        created_at: now,
        updated_at: now,
    };
    project.validate()?;

    let links = resolved_storage
        .as_ref()
        .map(|resolved| ProjectStorageLink {
            local_project_id: local_project_id.clone(),
            storage_id: resolved.storage.id.clone(),
            bundle_id: bundle_id.clone(),
            recipe: None,
            scope_bases: BTreeMap::new(),
            project_content_preferences: Default::default(),
            conversation_checkpoints: BTreeMap::new(),
            pinned: true,
            created_at: now,
        })
        .into_iter()
        .collect();

    let binding = match detached {
        Some(previous) => ProjectBinding {
            host_id: previous.host_id,
            replica_id: previous.replica_id,
            local_project_id,
            bundle_id,
            project_root: draft.project_root.clone(),
            canonical_project_root: draft.canonical_project_root.clone(),
            profile_ids: resolved_profiles.profile_ids.clone(),
            state: BindingState::Active,
            revision: previous.revision.saturating_add(1),
            updated_at: now,
        },
        None => ProjectBinding {
            host_id: host_id.clone(),
            replica_id: ReplicaId::parse(generated_named_id("replica")?)?,
            local_project_id,
            bundle_id,
            project_root: draft.project_root.clone(),
            canonical_project_root: draft.canonical_project_root.clone(),
            profile_ids: resolved_profiles.profile_ids.clone(),
            state: BindingState::Active,
            revision: 0,
            updated_at: now,
        },
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
    match prospective_bindings
        .iter()
        .position(|binding| binding.replica_id == transaction.binding.replica_id)
    {
        Some(index) => prospective_bindings[index] = transaction.binding.clone(),
        None => prospective_bindings.push(transaction.binding.clone()),
    }
    validate_config_storage_isolation(
        repository,
        &prospective_config,
        &prospective_bindings,
        &prospective_profiles,
    )?;
    Ok(transaction)
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
    let alias_qualifier_keys = provider_registry()
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.key)
        .collect::<Vec<_>>();
    let alias_qualifier = alias_qualifier_keys
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
        if let Some(index) = machine
            .bindings
            .iter()
            .position(|binding| binding.local_project_id == expected_binding.local_project_id)
        {
            let existing = &machine.bindings[index];
            if existing == &expected_binding {
                return Ok(());
            }
            let reactivates_detached = existing.state == BindingState::Detached
                && expected_binding.state == BindingState::Active
                && existing.host_id == expected_binding.host_id
                && existing.replica_id == expected_binding.replica_id
                && existing.bundle_id == expected_binding.bundle_id
                && existing.profile_ids == expected_binding.profile_ids
                && existing
                    .canonical_project_root
                    .eq_ignore_ascii_case(&expected_binding.canonical_project_root)
                && expected_binding.revision == existing.revision.saturating_add(1);
            if reactivates_detached {
                machine.bindings[index] = expected_binding.clone();
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

pub(super) fn finalize_project_setup_with_repository(
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
        let transaction = build_setup_transaction(repository, &draft)?;
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
    let resolved = resolve_project_binding(repository, &stored_binding)?;
    let binding = resolved.binding.clone();
    let profile_id = binding
        .profile_ids
        .get(&codex_key())
        .map(ToString::to_string);
    let mut audit = CodexConversationPathAudit {
        local_project_id: local_project_id.clone(),
        profile_id,
        profile_path: resolved.homes.get(&codex_key()).cloned(),
        project_root: binding.project_root.clone(),
        assigned_thread_count: 0,
        matching_thread_count: 0,
        issues: Vec::new(),
        blockers: Vec::new(),
        warnings: Vec::new(),
        ready: true,
        can_repair: false,
    };
    let Some(codex_home_text) = resolved.homes.get(&codex_key()).map(String::as_str) else {
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
            let metadata = match provider_registry()
                .require_sessions(&codex_key())?
                .scan_metadata(entry.path())
            {
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
        // Repair moves exactly one old root to the current root (§6.5); a
        // thread recording several distinct out-of-project roots is
        // ambiguous — which one was the project? — and paths outside the
        // project working directory are never remapped, so it needs manual
        // review instead of a blanket rewrite.
        if stale_cwds.len() > 1 {
            audit.blockers.push(format!(
                "Codex thread '{}' records {} distinct out-of-project roots; repair requires \
                 manual review",
                thread_id,
                stale_cwds.len()
            ));
            continue;
        }
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
    let resolved = resolve_project_binding(repository, &stored_binding)?;
    let binding = resolved.binding.clone();
    let codex_home = resolved
        .homes
        .get(&codex_key())
        .map(String::as_str)
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

    // Repair transforms the whole matcher scope (values and keys — the old
    // structural-cwd-only rewrite missed sandbox/permission entries and all
    // content mentions), so the current binding root must satisfy the
    // path-map grammar before any file is touched.
    validate_pathmap_root("project root", &binding.project_root)?;
    let repair_home = std::env::var("HOME").ok();
    let mapper_new = CodexPathMapper::new(
        &binding.project_root,
        (binding.canonical_project_root != binding.project_root)
            .then_some(binding.canonical_project_root.as_str()),
        repair_home.as_deref(),
    )?;

    // Which repaired threads are pinned (accepted on any storage link):
    // pinned repairs must preserve the published wire form byte-exactly and
    // rebase their checkpoints; unpinned repairs run the explicit new-root
    // collision pre-scan instead.
    let codex_profile_id = stored_binding.profile_ids.get(&codex_key()).cloned();
    let bindings_document = repository.load_conversation_bindings()?;
    let logical_by_provider: BTreeMap<String, (ThreadId, Option<RootTailLayout>)> =
        bindings_document
            .bindings
            .iter()
            .filter(|row| {
                row.host_id == binding.host_id
                    && row.replica_id == binding.replica_id
                    && row.local_project_id == binding.local_project_id
                    && Some(&row.provider_profile_id) == codex_profile_id.as_ref()
            })
            .filter_map(|row| {
                row.provider_thread_id.as_ref().map(|provider_id| {
                    (
                        provider_id.clone(),
                        (row.thread_id.clone(), row.root_tail_layout.clone()),
                    )
                })
            })
            .collect();
    let repair_config = repository.load_config()?;
    let codex_scope = SyncScopeKey::Provider(codex_key());
    let thread_is_pinned = |provider_thread_id: &str| -> bool {
        let Some((logical, _)) = logical_by_provider.get(provider_thread_id) else {
            return false;
        };
        repair_config
            .links
            .iter()
            .filter(|link| link.local_project_id == binding.local_project_id)
            .filter_map(|link| link.conversation_checkpoints.get(&codex_scope))
            .filter_map(|threads| threads.get(logical))
            .any(|checkpoint| checkpoint.accepted.is_some())
    };

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
        // §6.5: repair ≡ decode_new(encode_old(x)) — the pull pipeline
        // applied locally. The old root is the recorded stale cwd; a value
        // the path-map grammar rejects cannot be repaired mechanically.
        validate_pathmap_root("stale Codex conversation root", &issue.recorded_cwd)
            .map_err(|error| format!("Codex conversation '{}': {}", issue.thread_id, error))?;
        let mapper_old = CodexPathMapper::new(
            &issue.recorded_cwd,
            derive_canonical_alias(&issue.recorded_cwd).as_deref(),
            repair_home.as_deref(),
        )?;
        let pinned = thread_is_pinned(&issue.thread_id);
        if !pinned {
            // Unpinned threads have no wire to gate against: run the
            // explicit collision pre-scan so foreign content mentioning the
            // new root can never be silently captured into remap scope.
            let mut collision = None;
            for raw in original.split(|byte| *byte == b'\n') {
                let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
                if raw.is_empty() {
                    continue;
                }
                let value: serde_json::Value = serde_json::from_slice(raw)
                    .map_err(|_| "Codex conversation contains malformed JSON".to_string())?;
                if let Some(found) = mapper_new.find_source_match(&value) {
                    collision = Some(found);
                    break;
                }
            }
            if let Some(found) = collision {
                return Err(format!(
                    "Codex conversation '{}' already mentions the current project root \
                     ('{}'); repair would capture pre-existing content into remap scope \
                     and requires manual review",
                    issue.thread_id, found
                ));
            }
        }
        let repaired = remap_codex_conversation_paths(
            &original,
            &issue.thread_id,
            &mapper_old,
            &mapper_new,
            pinned,
        )
        .map_err(|error| format!("Codex conversation '{}': {}", issue.thread_id, error))?;
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

    // §6.5 checkpoint rebase, atomic across every storage link: the wire
    // pin (`accepted`) is untouched — the gate proved the wire form is
    // identical — while the local column (accepted-local twin, observed
    // fold, prefix witness, physical mapping, raw-range pin) re-derives
    // from the repaired bytes. With no unpushed suffix the thread lands in
    // Sync with nothing to republish; a genuine local suffix rides through
    // as LocalOutstanding. Unpinned checkpoints re-baseline the observed
    // column the same way.
    struct RebasedThread {
        seeded: CodexSeededRoot,
        chain: Vec<super::domain::CanonicalConversationMessage>,
        consumed_prefix_sha256: String,
    }
    let mut rebased: BTreeMap<ThreadId, RebasedThread> = BTreeMap::new();
    for repair in &pending {
        let Some((logical, layout)) = logical_by_provider.get(&repair.thread_id) else {
            continue;
        };
        let has_checkpoint = repair_config
            .links
            .iter()
            .filter(|link| link.local_project_id == binding.local_project_id)
            .filter_map(|link| link.conversation_checkpoints.get(&codex_scope))
            .any(|threads| threads.contains_key(logical));
        if !has_checkpoint {
            continue;
        }
        let lookup = codex_rollout_lookup(&canonical_codex_home);
        let derived = codex_history_graph::resolve_codex_root(&repair.path, &lookup).and_then(
            |resolved_root| {
                let leaf_bytes = codex_history_graph::read_rollout_bounded(&repair.path)?;
                let seeded = codex_history_graph::seed_codex_root(
                    &resolved_root,
                    &leaf_bytes,
                    layout.as_ref(),
                )?;
                let consumed = seeded.mapping.consumed_byte_offset as usize;
                Ok(RebasedThread {
                    consumed_prefix_sha256: hex_digest(
                        Sha256::digest(&leaf_bytes[..consumed]).as_slice(),
                    ),
                    chain: resolved_root.chain,
                    seeded,
                })
            },
        );
        match derived {
            Ok(thread) => {
                rebased.insert(logical.clone(), thread);
            }
            Err(error) => {
                let rollback_errors = rollback(pending.len());
                return Err(if rollback_errors.is_empty() {
                    format!(
                        "repaired Codex conversation '{}' failed re-observation: {}; changes \
                         were rolled back",
                        repair.thread_id, error
                    )
                } else {
                    format!(
                        "repaired Codex conversation '{}' failed re-observation: {} and \
                         rollback failed for {}",
                        repair.thread_id,
                        error,
                        rollback_errors.join("; ")
                    )
                });
            }
        }
    }
    if !rebased.is_empty() {
        let rebase_outcome = repository.mutate_config(|config| {
            for link in config
                .links
                .iter_mut()
                .filter(|link| link.local_project_id == binding.local_project_id)
            {
                let Some(threads) = link.conversation_checkpoints.get_mut(&codex_scope) else {
                    continue;
                };
                for (logical, thread) in &rebased {
                    let Some(checkpoint) = threads.get_mut(logical) else {
                        continue;
                    };
                    checkpoint.accepted_local_tail = match &checkpoint.accepted {
                        Some(accepted) => {
                            // The local twin refolds at this link's accepted
                            // end over the repaired chain; positions are
                            // form-independent.
                            let end = accepted.portable_tail.accepted_end_record_count;
                            if end > thread.chain.len() as u64 {
                                return Err(
                                    "repaired conversation is shorter than its accepted end"
                                        .to_string(),
                                );
                            }
                            let state = super::domain::compute_conversation_tail_state(
                                thread.seeded.layout.partition_kind,
                                thread.seeded.layout.start_record_count,
                                end,
                                &thread.chain,
                            )?;
                            let local_twin = PortableConversationTail {
                                codec: super::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                                partition_kind: thread.seeded.layout.partition_kind,
                                start_record_count: thread.seeded.layout.start_record_count,
                                accepted_end_record_count: end,
                                accepted_tail_state: state,
                            };
                            local_twin.validate()?;
                            Some(local_twin)
                        }
                        None => None,
                    };
                    checkpoint.observed_tail = thread.seeded.observed_tail.clone();
                    checkpoint.physical = PhysicalMappingState::Ready {
                        root_mapping: thread.seeded.mapping.clone(),
                        raw_range_pin: RawRangePin {
                            start_byte_offset: 0,
                            end_byte_offset: thread.seeded.mapping.consumed_byte_offset,
                            range_sha256: thread.consumed_prefix_sha256.clone(),
                        },
                    };
                    checkpoint.prefix_witness = thread.seeded.prefix_witness.clone();
                    checkpoint.generation = checkpoint.generation.saturating_add(1);
                }
            }
            Ok(())
        });
        if let Err(error) = rebase_outcome {
            let rollback_errors = rollback(pending.len());
            return Err(if rollback_errors.is_empty() {
                format!(
                    "conversation checkpoint rebase failed: {}; changes were rolled back",
                    error
                )
            } else {
                format!(
                    "conversation checkpoint rebase failed: {} and rollback failed for {}",
                    error,
                    rollback_errors.join("; ")
                )
            });
        }
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

/// §6.5's shared-module rewrite: repair ≡ `decode_new(encode_old(x))` per
/// record — the pull pipeline applied locally, covering the full matcher
/// scope (string values and object keys; the retired structural-only
/// rewrite missed sandbox/permission entries and every content mention).
/// For pinned threads `gate_wire_equality` proves
/// `encode_new(repaired) == encode_old(original)` per record: the published
/// wire form is unchanged byte-exactly, and the gate doubles as the
/// collision detector for content that would newly tokenize under the
/// current root. Untouched records keep their original line bytes.
fn remap_codex_conversation_paths(
    source: &[u8],
    expected_thread_id: &str,
    mapper_old: &CodexPathMapper,
    mapper_new: &CodexPathMapper,
    gate_wire_equality: bool,
) -> Result<Vec<u8>, String> {
    let before = analyze_codex_rollout_structure(source, Some(expected_thread_id))
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(source.len());
    let mut changed = 0_usize;
    let mut record_number = 0_usize;
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
        record_number += 1;
        let value: serde_json::Value = serde_json::from_slice(json)
            .map_err(|_| "Codex conversation contains malformed JSON".to_string())?;
        let encoded_old = mapper_old
            .encode_record(&value)
            .map_err(|error| format!("record {record_number}: {error}"))?;
        let repaired = mapper_new.decode_record(&encoded_old.wire)?;
        if gate_wire_equality {
            let re_encoded = mapper_new.encode_record(&repaired).map_err(|error| {
                format!("record {record_number} collides with the current project root: {error}")
            })?;
            if re_encoded.wire != encoded_old.wire {
                return Err(format!(
                    "record {record_number} contains content that collides with the current \
                     project root; repair cannot preserve the published wire form and \
                     requires manual review"
                ));
            }
        }
        if repaired == value {
            output.extend_from_slice(line);
            continue;
        }
        changed = changed.saturating_add(1);
        output.extend_from_slice(
            &serde_json::to_vec(&repaired)
                .map_err(|error| format!("serialize repaired Codex session row: {}", error))?,
        );
        output.extend_from_slice(ending);
    }
    if changed == 0 {
        return Err("conversation no longer contains a stale project path".to_string());
    }
    let after = analyze_codex_rollout_structure(&output, Some(expected_thread_id))
        .map_err(|error| error.to_string())?;
    let expected_cwds: BTreeSet<String> = before
        .structural_cwds
        .iter()
        .map(|cwd| {
            let encoded = mapper_old.encode_record(&serde_json::Value::String(cwd.clone()))?;
            match mapper_new.decode_record(&encoded.wire)? {
                serde_json::Value::String(decoded) => Ok(decoded),
                _ => Err("structural cwd decode changed its type".to_string()),
            }
        })
        .collect::<Result<_, String>>()?;
    if before.current.id != after.current.id
        || before.fork_ancestry != after.fork_ancestry
        || before.accepted_metadata_ids != after.accepted_metadata_ids
        || after.structural_cwds != expected_cwds
    {
        return Err("repaired Codex conversation changed structural identity".to_string());
    }
    Ok(output)
}

/// The macOS textual firmlink aliases the binding's canonical-root handling
/// models, derivable without touching the filesystem (the old root may no
/// longer exist): `/var`, `/tmp`, and `/etc` each alias `/private/...`.
fn derive_canonical_alias(root: &str) -> Option<String> {
    for prefix in ["/var/", "/tmp/", "/etc/"] {
        if root.starts_with(prefix) {
            return Some(format!("/private{root}"));
        }
    }
    root.strip_prefix("/private")
        .filter(|rest| {
            ["/var/", "/tmp/", "/etc/"]
                .iter()
                .any(|prefix| rest.starts_with(prefix))
        })
        .map(str::to_string)
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

    type ContinuityProbe = Box<dyn FnMut(&RepositoryBase) -> Result<bool, String>>;

    use crate::project_sync::domain::{
        BundleHead, BundleScope, OriginReplicaToken, ProjectContentPreferences, RestoreActionKind,
        StorageConfig, StorageKind, TombstoneTarget,
    };
    use crate::project_sync::storage_core::{
        library_object_key, repository_object_key, LocalObjectStore, ObjectStore,
        STORAGE_MARKER_KEY,
    };
    use crate::storage_catalog::persistence::CatalogStateRepository;
    use crate::sync_tests::thread_variants::harness::{
        publish_request, ThreadSpec, TranscriptBuilder,
    };

    #[test]
    fn unchanged_scope_recipe_preserves_remote_revision_after_pull() {
        let generic_resource_id = ResourceId::parse("codex:memory").unwrap();
        let generic_entry = RecipeEntry {
            resource_id: generic_resource_id.clone(),
            apply_policy: ApplyPolicy::SafeFile,
            required: false,
        };
        let conversation_resource_id = conversation_resource_id(
            &codex_key(),
            &ThreadId::parse("thread-recipe-revision").unwrap(),
        )
        .unwrap();
        let conversation_entry = RecipeEntry {
            resource_id: conversation_resource_id.clone(),
            apply_policy: ApplyPolicy::Merge,
            required: false,
        };

        let mut previous = BundleRecipe {
            revision: 7,
            ..BundleRecipe::default()
        };
        previous
            .entries
            .insert(generic_resource_id.clone(), generic_entry.clone());
        previous
            .entries
            .insert(conversation_resource_id, conversation_entry);

        let mut unchanged = BundleRecipe::default();
        unchanged
            .entries
            .insert(generic_resource_id.clone(), generic_entry);
        preserve_unchanged_scope_recipe_revision(&mut unchanged, Some(&previous));
        assert_eq!(unchanged.revision, 7);

        let mut changed = BundleRecipe::default();
        changed.entries.insert(
            generic_resource_id.clone(),
            RecipeEntry {
                resource_id: generic_resource_id,
                apply_policy: ApplyPolicy::ExplicitReview,
                required: false,
            },
        );
        preserve_unchanged_scope_recipe_revision(&mut changed, Some(&previous));
        assert_eq!(changed.revision, 0);
    }

    #[test]
    fn observation_worker_outcome_preserves_errors_and_rejects_missing_slots() {
        let thread_id = ThreadId::parse("thread-worker-test").unwrap();
        let mut failed = std::sync::OnceLock::new();
        assert!(failed
            .set(Err("sentinel observation error".to_string()))
            .is_ok());
        assert_eq!(
            take_observation_outcome(&thread_id, &mut failed)
                .err()
                .as_deref(),
            Some("sentinel observation error")
        );

        let mut missing = std::sync::OnceLock::new();
        assert_eq!(
            take_observation_outcome(&thread_id, &mut missing)
                .err()
                .as_deref(),
            Some("thread observation worker produced no outcome for thread 'thread-worker-test'")
        );
    }

    #[test]
    fn packaged_cli_is_resolved_beside_the_desktop_executable() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join(if cfg!(windows) {
            "mallard.exe"
        } else {
            "mallard"
        });
        let cli = temp.path().join(if cfg!(windows) {
            "mallard-cli.exe"
        } else {
            "mallard-cli"
        });
        std::fs::write(&desktop, b"desktop").unwrap();
        std::fs::write(&cli, b"cli").unwrap();

        assert_eq!(
            packaged_cli_path(&desktop).unwrap(),
            cli.canonicalize().unwrap()
        );
        std::fs::remove_file(&cli).unwrap();
        assert!(packaged_cli_path(&desktop)
            .unwrap_err()
            .contains("not available"));
    }

    #[cfg(unix)]
    #[test]
    fn packaged_cli_path_rejects_a_sibling_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("mallard");
        let outside = temp.path().join("outside-cli");
        let cli = temp.path().join("mallard-cli");
        std::fs::write(&desktop, b"desktop").unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, &cli).unwrap();

        assert!(packaged_cli_path(&desktop)
            .unwrap_err()
            .contains("not available"));
    }

    #[test]
    fn codex_portable_display_title_decodes_tokens_and_rejects_collisions() {
        let root = "/tmp/mallard-target-project";
        let title = "Open /__mallard_root__/src/main.rs; keep /tmp/project1_external/outside.txt";
        assert_eq!(
            decode_codex_portable_display_title(title, root, root).unwrap(),
            "Open /tmp/mallard-target-project/src/main.rs; keep /tmp/project1_external/outside.txt"
        );
        assert!(decode_codex_portable_display_title(
            "Keep /__mallard_root___external/outside.txt",
            root,
            root,
        )
        .is_err());
        assert!(decode_codex_portable_display_title(
            "Already mentions /tmp/mallard-target-project/src/main.rs",
            root,
            root,
        )
        .is_err());
    }

    #[test]
    fn codex_index_payload_writes_provider_schema_rows_only_for_titled_threads() {
        let temp = tempfile::tempdir().unwrap();
        let index_path = temp.path().join("session_index.jsonl");

        let (expectation, bytes) = codex_index_payload(&index_path, &[]).unwrap();
        assert_eq!(expectation, TargetExpectation::Absent);
        assert!(bytes.is_none(), "titleless threads write no index row");

        let named = |name: &str, replace_existing: bool| CodexIndexRowSpec {
            provider_thread_id: "thread-new".to_string(),
            thread_name: name.to_string(),
            updated_at: 1_784_332_800,
            replace_existing,
        };
        let (_, bytes) = codex_index_payload(&index_path, &[named("Named", false)]).unwrap();
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
        assert!(codex_index_payload(&index_path, &[named("Named", false)])
            .unwrap_err()
            .contains("already indexed"));
        // A keep-both rewrite carries two rows in one payload: the preserved
        // rename replaces the existing row while the fresh destination row
        // appends alongside it.
        let (_, replacement) = codex_index_payload(
            &index_path,
            &[
                named("Updated", true),
                CodexIndexRowSpec {
                    provider_thread_id: "thread-destination".to_string(),
                    thread_name: "Cloud".to_string(),
                    updated_at: 1_784_332_800,
                    replace_existing: false,
                },
            ],
        )
        .unwrap();
        let rows = replacement
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["id"], "thread-new");
        assert_eq!(rows[0]["thread_name"], "Updated");
        assert_eq!(rows[1]["id"], "thread-destination");
        assert_eq!(rows[1]["thread_name"], "Cloud");
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

    fn continuity_head(revision: u64, manifest_sha256: &str) -> BundleHead {
        BundleHead {
            schema_version: crate::project_sync::domain::BUNDLE_SCHEMA,
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            kind: BundleKind::Project,
            revision,
            commit_id: format!("commit-{}", revision),
            manifest_key: format!("_manifests/{}.json", revision),
            manifest_size: 1,
            manifest_sha256: manifest_sha256.to_string(),
            updated_at: revision,
        }
    }

    fn continuity_repository_base(revision: u64, manifest_sha256: &str) -> RepositoryBase {
        RepositoryBase {
            revision,
            commit_id: format!("commit-{}", revision),
            manifest_sha256: manifest_sha256.to_string(),
            manifest_size: 1,
        }
    }

    fn continuity_scope_base(
        scope: SyncScopeKey,
        scope_revision: u64,
        scope_sha256: &str,
        reviewed_in: RepositoryBase,
        binding_revision: u64,
    ) -> ScopeBase {
        ScopeBase {
            scope,
            scope_revision,
            scope_sha256: scope_sha256.to_string(),
            reviewed_in,
            binding_revision,
            last_pull_at: None,
            last_push_at: Some(99),
        }
    }

    fn continuity_link(bases: Vec<ScopeBase>) -> ProjectStorageLink {
        ProjectStorageLink {
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            storage_id: StorageId::parse("storage-a").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            recipe: None,
            scope_bases: bases
                .into_iter()
                .map(|base| (base.scope.clone(), base))
                .collect(),
            conversation_checkpoints: BTreeMap::new(),
            project_content_preferences: ProjectContentPreferences::default(),
            pinned: false,
            created_at: 1,
        }
    }

    /// A binding whose only bound provider is Codex, so the writable scopes
    /// are exactly `project` and `provider:codex`.
    fn continuity_binding(revision: u64) -> ProjectBinding {
        ProjectBinding {
            host_id: HostId::parse("1".repeat(32)).unwrap(),
            replica_id: ReplicaId::parse("replica-a").unwrap(),
            local_project_id: LocalProjectId::parse("project-a").unwrap(),
            bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            project_root: "/tmp/project-a".to_string(),
            canonical_project_root: "/tmp/project-a".to_string(),
            profile_ids: BTreeMap::from([(
                ProviderKey::parse("codex").unwrap(),
                LocalProviderProfileId::parse("profile-codex").unwrap(),
            )]),
            state: BindingState::Active,
            revision,
            updated_at: 1,
        }
    }

    fn continuity_snapshot(
        head: BundleHead,
        scopes: Vec<(SyncScopeKey, u64, &str)>,
    ) -> BundleSnapshot {
        let manifest = BundleManifest {
            schema_version: crate::project_sync::domain::BUNDLE_SCHEMA,
            revision: head.revision,
            commit_id: head.commit_id.clone(),
            updated_at: head.updated_at,
            bundle: BundleIdentity {
                bundle_id: head.bundle_id.clone(),
                display_name: "Continuity".to_string(),
                kind: BundleKind::Project,
                repository_fingerprint: None,
            },
            published_by_origin: PortableOrigin {
                origin_host_token: OriginHostToken::parse("a".repeat(64)).unwrap(),
                origin_replica_token: OriginReplicaToken::parse("b".repeat(64)).unwrap(),
                shared_label: None,
            },
            scopes: scopes
                .into_iter()
                .map(|(scope, revision, content_sha256)| {
                    (
                        scope,
                        BundleScope {
                            revision,
                            content_sha256: content_sha256.to_string(),
                            recipe: BundleRecipe::default(),
                            captured_with: CapturedWith::default(),
                            conversations: BTreeMap::new(),
                            resources: BTreeMap::new(),
                            files: BTreeMap::new(),
                            directories: BTreeMap::new(),
                            tombstones: BTreeMap::new(),
                        },
                    )
                })
                .collect(),
        };
        BundleSnapshot {
            storage_id: StorageId::parse("storage-a").unwrap(),
            head,
            manifest,
            fetched_at: 1,
        }
    }

    /// One fixture per row of the continuity table
    /// (DESIGN_REMOTE_HISTORY_CONTINUITY.md), asserting the exact verdict and
    /// that the probe runs only in the revision-ahead arm.
    #[test]
    fn remote_continuity_rows_classify_from_content_witnesses() {
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);
        let base = continuity_repository_base(3, &digest_a);
        let mut forbidden: ContinuityProbe =
            Box::new(|_: &RepositoryBase| panic!("probe must not run for this row"));

        // Row 1 — no head, no base.
        assert_eq!(
            classify_remote_continuity(None, None, &mut forbidden),
            RemoteContinuity::Fresh
        );
        // Row 2 — no head, base recorded.
        assert_eq!(
            classify_remote_continuity(None, Some(&base), &mut forbidden),
            RemoteContinuity::RemoteMissing { base_revision: 3 }
        );
        // Row 3 — head, no base.
        assert_eq!(
            classify_remote_continuity(Some(&continuity_head(3, &digest_a)), None, &mut forbidden),
            RemoteContinuity::Unreviewed { remote_revision: 3 }
        );
        // Row 4 — revisions equal, digests equal.
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(3, &digest_a)),
                Some(&base),
                &mut forbidden
            ),
            RemoteContinuity::InSync { revision: 3 }
        );
        // Row 5 — revisions equal, digests differ: one lineage commits a
        // revision exactly once, so this is proof of replacement, free.
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(3, &digest_b)),
                Some(&base),
                &mut forbidden
            ),
            RemoteContinuity::Replaced {
                base_revision: 3,
                remote_revision: 3
            }
        );
        // Row 6 — head revision regressed below the base.
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(2, &digest_b)),
                Some(&base),
                &mut forbidden
            ),
            RemoteContinuity::Replaced {
                base_revision: 3,
                remote_revision: 2
            }
        );

        // Rows 7 and 8 — revision ahead: the probe decides, exactly once.
        let mut probe_calls = 0_usize;
        let mut present = |_: &RepositoryBase| {
            probe_calls += 1;
            Ok::<_, String>(true)
        };
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(5, &digest_b)),
                Some(&base),
                &mut present
            ),
            RemoteContinuity::Advanced {
                base_revision: 3,
                remote_revision: 5
            }
        );
        assert_eq!(probe_calls, 1);

        let mut probe_calls = 0_usize;
        let mut absent = |_: &RepositoryBase| {
            probe_calls += 1;
            Ok::<_, String>(false)
        };
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(5, &digest_b)),
                Some(&base),
                &mut absent
            ),
            RemoteContinuity::Replaced {
                base_revision: 3,
                remote_revision: 5
            }
        );
        assert_eq!(probe_calls, 1);

        // Probe transport failure claims no verdict.
        let mut failing = |_: &RepositoryBase| Err::<bool, _>("storage offline".to_string());
        assert_eq!(
            classify_remote_continuity(
                Some(&continuity_head(5, &digest_b)),
                Some(&base),
                &mut failing
            ),
            RemoteContinuity::Unknown {
                message: "storage offline".to_string()
            }
        );
    }

    #[test]
    fn push_destination_classification_preserves_remote_history_safety() {
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);
        let scope_c = "c".repeat(64);
        let scope_d = "d".repeat(64);
        let scope_e = "e".repeat(64);
        let codex_scope = SyncScopeKey::Provider(ProviderKey::parse("codex").unwrap());
        let claude_scope = SyncScopeKey::Provider(ProviderKey::parse("claude").unwrap());
        let binding = continuity_binding(7);
        // The reviewed link: the codex scope was reviewed at scope revision 1
        // (digest c) inside repository revision 3 (manifest digest a).
        let reviewed = continuity_link(vec![continuity_scope_base(
            codex_scope.clone(),
            1,
            &scope_c,
            continuity_repository_base(3, &digest_a),
            7,
        )]);
        let empty = continuity_link(vec![]);
        let current = continuity_snapshot(
            continuity_head(3, &digest_a),
            vec![(codex_scope.clone(), 1, &scope_c)],
        );
        let mut forbidden: ContinuityProbe =
            Box::new(|_: &RepositoryBase| panic!("probe must not run for this case"));

        assert_eq!(
            classify_push_destination(None, &empty, &binding, &mut forbidden),
            PushDestinationStatus::Ready { revision: None }
        );
        assert_eq!(
            classify_push_destination(None, &reviewed, &binding, &mut forbidden),
            PushDestinationStatus::RemoteHistoryMissing { base_revision: 3 }
        );
        // Remote data in a writable scope that this replica never reviewed
        // blocks with the exact blocking scope named.
        assert_eq!(
            classify_push_destination(Some(&current), &empty, &binding, &mut forbidden),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::RemoteWithoutBase,
                remote_revision: 3,
                base_revision: None,
                blocking_scopes: vec!["provider:codex".to_string()],
            }
        );
        // Binding-revision mismatch is push policy and wins before continuity
        // — and therefore before any probe read is spent.
        assert_eq!(
            classify_push_destination(
                Some(&current),
                &reviewed,
                &continuity_binding(8),
                &mut forbidden
            ),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::BindingChanged,
                remote_revision: 3,
                base_revision: Some(3),
                blocking_scopes: vec!["provider:codex".to_string()],
            }
        );
        assert_eq!(
            classify_push_destination(Some(&current), &reviewed, &binding, &mut forbidden),
            PushDestinationStatus::Ready { revision: Some(3) }
        );
        // Equal revision with a different digest is replacement, no probe.
        assert_eq!(
            classify_push_destination(
                Some(&continuity_snapshot(
                    continuity_head(3, &digest_b),
                    vec![(codex_scope.clone(), 1, &scope_c)],
                )),
                &reviewed,
                &binding,
                &mut forbidden
            ),
            PushDestinationStatus::RemoteHistoryReplaced {
                base_revision: 3,
                remote_revision: 3,
                base_manifest_sha256: digest_a.clone(),
                remote_manifest_sha256: digest_b.clone(),
            }
        );
        // Revision ahead + base manifest still present + the codex scope
        // advanced ⇒ a true advance of a writable scope.
        let mut present = |_: &RepositoryBase| Ok::<_, String>(true);
        assert_eq!(
            classify_push_destination(
                Some(&continuity_snapshot(
                    continuity_head(4, &digest_b),
                    vec![(codex_scope.clone(), 2, &scope_d)],
                )),
                &reviewed,
                &binding,
                &mut present
            ),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::RemoteAdvanced,
                remote_revision: 4,
                base_revision: Some(3),
                blocking_scopes: vec!["provider:codex".to_string()],
            }
        );
        // Revision ahead where only an unbound provider scope changed ⇒ the
        // repository advanced but no writable scope did, so a Codex-only
        // binding may push without pulling the Claude change (D6).
        let mut present = |_: &RepositoryBase| Ok::<_, String>(true);
        assert_eq!(
            classify_push_destination(
                Some(&continuity_snapshot(
                    continuity_head(4, &digest_b),
                    vec![
                        (codex_scope.clone(), 1, &scope_c),
                        (claude_scope.clone(), 5, &scope_e),
                    ],
                )),
                &reviewed,
                &binding,
                &mut present
            ),
            PushDestinationStatus::Ready { revision: Some(4) }
        );
        // Revision ahead + base manifest gone ⇒ replaced history.
        let mut absent = |_: &RepositoryBase| Ok::<_, String>(false);
        let replaced = classify_push_destination(
            Some(&continuity_snapshot(
                continuity_head(4, &digest_b),
                vec![(codex_scope.clone(), 2, &scope_d)],
            )),
            &reviewed,
            &binding,
            &mut absent,
        );
        assert_eq!(
            replaced,
            PushDestinationStatus::RemoteHistoryReplaced {
                base_revision: 3,
                remote_revision: 4,
                base_manifest_sha256: digest_a.clone(),
                remote_manifest_sha256: digest_b.clone(),
            }
        );
        // A probe transport failure fails closed as a storage error.
        let mut failing = |_: &RepositoryBase| Err::<bool, _>("storage offline".to_string());
        assert_eq!(
            classify_push_destination(
                Some(&continuity_snapshot(
                    continuity_head(4, &digest_b),
                    vec![(codex_scope.clone(), 2, &scope_d)],
                )),
                &reviewed,
                &binding,
                &mut failing
            ),
            PushDestinationStatus::Unavailable {
                message: "storage offline".to_string()
            }
        );

        let missing = classify_push_destination(None, &reviewed, &binding, &mut forbidden);
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({
                "state": "remote_history_missing",
                "base_revision": 3,
            })
        );
        let message = push_destination_error(&missing).unwrap();
        assert!(message.contains("remote history is missing"));
        assert!(message.contains("unlink and relink"));
        assert!(!message.contains("deleted"));

        assert_eq!(
            serde_json::to_value(&replaced).unwrap(),
            serde_json::json!({
                "state": "remote_history_replaced",
                "base_revision": 3,
                "remote_revision": 4,
                "base_manifest_sha256": digest_a,
                "remote_manifest_sha256": digest_b,
            })
        );
        let message = push_destination_error(&replaced).unwrap();
        assert!(message.contains("was replaced"));
        assert!(message.contains("base revision 3"));
        assert!(message.contains("remote revision 4"));
        assert!(message.contains(&digest_a[..8]));
        assert!(message.contains(&digest_b[..8]));
        assert!(message.contains("pull and review"));
        assert!(message.contains("unlink and relink"));

        let advanced = classify_push_destination(
            Some(&continuity_snapshot(
                continuity_head(4, &digest_b),
                vec![(codex_scope.clone(), 2, &scope_d)],
            )),
            &reviewed,
            &binding,
            &mut |_: &RepositoryBase| Ok::<_, String>(true),
        );
        let message = push_destination_error(&advanced).unwrap();
        assert!(message.contains("base revision 3"));
        assert!(message.contains("remote revision 4"));
        assert!(message.contains("provider:codex"));
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
        provider: &ProviderKey,
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
        let profile_id = add_profile(repo, &codex_key(), codex_home);
        let binding = save_project_binding_with_repository(
            repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
        let legacy = builder.build();
        let mut current = Vec::new();
        for (ordinal, raw) in legacy
            .split(|byte| *byte == b'\n')
            .filter(|raw| !raw.is_empty())
            .enumerate()
        {
            let mut record: serde_json::Value = serde_json::from_slice(raw).unwrap();
            if ordinal == 0 {
                record["payload"]["history_mode"] = serde_json::json!("paginated");
            }
            record["ordinal"] = serde_json::json!(ordinal as u64);
            serde_json::to_writer(&mut current, &record).unwrap();
            current.push(b'\n');
        }
        current
    }

    #[test]
    fn attachment_linked_claude_session_is_a_create_cloud_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("claude-attachment-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let claude_home = temp.path().join("claude-home");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(claude_home.join("projects/source/memory")).unwrap();
        let claude = ProviderKey::parse("claude").unwrap();
        let profile_id = add_profile(&repo, &claude, &claude_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(claude, profile_id)]),
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

        let cwd = project_root.to_string_lossy().into_owned();
        let records = [
            serde_json::json!({
                "type": "attachment",
                "uuid": "attachment-a",
                "parentUuid": null,
                "sessionId": "session-a",
                "cwd": cwd,
                "attachment": {"type": "hook_success"}
            }),
            serde_json::json!({
                "type": "user",
                "uuid": "user-a",
                "parentUuid": "attachment-a",
                "sessionId": "session-a",
                "cwd": cwd,
                "message": {"role": "user", "content": "test request"}
            }),
        ];
        let mut transcript = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        transcript.push('\n');
        std::fs::write(
            claude_home.join("projects/source/session-a.jsonl"),
            transcript,
        )
        .unwrap();

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            review.destination_status,
            PushDestinationStatus::Ready { revision: None }
        );
        let comparison = review.thread_comparison.as_ref().unwrap();
        let session = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some("session-a"))
            .unwrap();
        assert_eq!(session.state, ThreadSyncState::LocalOnly);
        assert_eq!(
            session.push_decision.action,
            ConversationReviewAction::CreateCloud
        );
        assert!(session.push_decision.selectable);
        assert!(review
            .conversation_action_ids
            .contains_key(&session.resource_id));
    }

    #[test]
    fn project_sync_service_rebaselines_before_loading_a_session_header() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(codex_home.join("sessions/2026/08/18")).unwrap();
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        let binding = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();

        let provider_thread_id = "019f7800-0000-7000-8000-000000000098";
        let thread_id =
            initial_conversation_thread_id(&project.bundle_id, &codex_key(), provider_thread_id)
                .unwrap();
        let resource_id = conversation_resource_id(&codex_key(), &thread_id).unwrap();
        let capture_request = capture_request_for_binding(&repo, &binding).unwrap();
        let (_, empty_views) = super::super::codex_watch_service::session_watch_service()
            .discovery_for(
                provider_registry(),
                &project.local_project_id,
                &capture_request,
            )
            .unwrap();
        assert!(empty_views.is_empty(), "the primed inventory must be empty");

        let session_path = codex_home.join(format!(
            "sessions/2026/08/18/rollout-2026-08-18T00-00-00-{provider_thread_id}.jsonl"
        ));
        std::fs::write(
            &session_path,
            command_test_transcript(
                provider_thread_id,
                &project_root,
                &[
                    ("message-user", "user", "Open the new session"),
                    ("message-agent", "assistant", "The session is ready."),
                ],
            ),
        )
        .unwrap();

        // Model the watcher event being consumed by another project sharing
        // this profile. Its fresh inventory makes the profile clean while the
        // original project's empty inventory remains cached.
        let cache_consumer = LocalProjectId::parse("session-detail-cache-consumer").unwrap();
        let (_, refreshed_views) = super::super::codex_watch_service::session_watch_service()
            .discovery_for(provider_registry(), &cache_consumer, &capture_request)
            .unwrap();
        assert_eq!(refreshed_views.len(), 1);
        let stale_views =
            load_project_session_views(&repo, &project.local_project_id, resource_id.as_str())
                .unwrap();
        assert!(stale_views.is_empty());

        let detail = ProjectSyncService::new(&repo)
            .get_project_session_detail(&project.local_project_id, &resource_id)
            .unwrap();
        assert_eq!(detail.resource_id, resource_id.as_str());
        assert_eq!(detail.provider_session_id, provider_thread_id);
    }

    #[test]
    fn project_sync_service_uses_repository_backed_session_detail_pipeline() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(project_root.join("src")).unwrap();
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
                expected_revision: None,
            },
        )
        .unwrap();

        let provider_thread_id = "019f7800-0000-7000-8000-000000000099";
        let inside_path = project_root.join("src/lib.rs");
        let arguments = serde_json::to_string(&serde_json::json!({
            "file_path": inside_path,
        }))
        .unwrap();
        let records = vec![
            serde_json::json!({
                "timestamp": "2026-08-18T00:00:00Z",
                "type": "session_meta",
                "payload": {"id": provider_thread_id, "cwd": project_root},
            }),
            serde_json::json!({
                "timestamp": "2026-08-18T00:00:01Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "Inspect the source file"},
            }),
            serde_json::json!({
                "timestamp": "2026-08-18T00:00:02Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "call_id": "read-1",
                    "name": "read_file",
                    "arguments": arguments,
                },
            }),
            serde_json::json!({
                "timestamp": "2026-08-18T00:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "read-1",
                    "output": "source contents",
                },
            }),
            serde_json::json!({
                "timestamp": "2026-08-18T00:00:04Z",
                "type": "event_msg",
                "payload": {"type": "agent_message", "message": "Inspection complete."},
            }),
        ];
        let mut transcript = Vec::new();
        for record in records {
            transcript.extend_from_slice(&serde_json::to_vec(&record).unwrap());
            transcript.push(b'\n');
        }
        let session_path = codex_home.join(format!(
            "sessions/2026/08/18/rollout-2026-08-18T00-00-00-{provider_thread_id}.jsonl"
        ));
        std::fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        std::fs::write(&session_path, transcript).unwrap();
        std::fs::write(
            codex_home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{provider_thread_id}\",\"thread_name\":\"Service seam session\",\"updated_at\":\"2026-08-18T00:00:04Z\"}}\n"
            ),
        )
        .unwrap();

        let thread_id =
            initial_conversation_thread_id(&project.bundle_id, &codex_key(), provider_thread_id)
                .unwrap();
        let resource_id = conversation_resource_id(&codex_key(), &thread_id).unwrap();
        let service = ProjectSyncService::new(&repo);

        let detail = service
            .get_project_session_detail(&project.local_project_id, &resource_id)
            .unwrap();
        let direct_detail = get_project_session_detail_with_repository(
            &repo,
            &project.local_project_id,
            resource_id.as_str(),
        )
        .unwrap();
        assert_eq!(detail, direct_detail);
        assert_eq!(detail.title, "Service seam session");

        let request = ProjectSessionTimelineRequest {
            snapshot_id: detail.snapshot_id.clone(),
            cursor: None,
            anchor_event_id: None,
            limit: Some(1),
            included_event_kinds: BTreeSet::from([SessionTimelineEventKind::ToolCall]),
            included_tool_categories: BTreeSet::from([SessionToolCategory::Read]),
            turn_id: None,
        };
        let page = service
            .get_project_session_timeline_page(
                &project.local_project_id,
                &resource_id,
                request.clone(),
            )
            .unwrap();
        let direct_page = get_project_session_timeline_page_with_repository(
            &repo,
            &project.local_project_id,
            resource_id.as_str(),
            request,
        )
        .unwrap();
        assert_eq!(page, direct_page);
        assert_eq!(page.events.len(), 1);
        let event_id = page.events[0].event_id.clone();

        let event_detail = service
            .get_project_session_event_detail(
                &project.local_project_id,
                &resource_id,
                &detail.snapshot_id,
                &event_id,
            )
            .unwrap();
        let direct_event_detail = get_project_session_event_detail_with_repository(
            &repo,
            &project.local_project_id,
            resource_id.as_str(),
            &detail.snapshot_id,
            &event_id,
        )
        .unwrap();
        assert_eq!(event_detail, direct_event_detail);
        assert_eq!(event_detail.result.as_deref(), Some("source contents"));
        assert!(event_detail
            .arguments
            .as_deref()
            .is_some_and(|value| value.contains("src/lib.rs")));
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
        record_reviewed_scope_bases(repo, project, binding, storage_id, &snapshot, 12);
        (snapshot, t1, t11, t111)
    }

    /// The reviewed state a link records after Push/Pull: the repository-level
    /// base (newest reviewed manifest coordinates) plus every scope base.
    fn reviewed_link_state(
        repo: &LocalStateRepository,
        local_project_id: &LocalProjectId,
        storage_id: &StorageId,
    ) -> (RepositoryBase, BTreeMap<SyncScopeKey, ScopeBase>) {
        let config = repo.load_config().unwrap();
        let link = project_link(&config, local_project_id, storage_id).unwrap();
        (
            repository_base_from_link(link).unwrap(),
            link.scope_bases.clone(),
        )
    }

    /// Record every scope of the verified snapshot as reviewed on the
    /// project/storage link, exactly as an approved Pull would: one
    /// `ScopeBase` per manifest scope, all reviewed in the snapshot's
    /// repository coordinates under the current binding revision.
    fn record_reviewed_scope_bases(
        repo: &LocalStateRepository,
        project: &LocalProjectRegistration,
        binding: &ProjectBinding,
        storage_id: &StorageId,
        snapshot: &BundleSnapshot,
        reviewed_at: u64,
    ) {
        let reviewed_in = RepositoryBase {
            revision: snapshot.head.revision,
            commit_id: snapshot.head.commit_id.clone(),
            manifest_sha256: snapshot.head.manifest_sha256.clone(),
            manifest_size: snapshot.head.manifest_size,
        };
        let coordinates = snapshot.manifest.scope_coordinates();
        repo.mutate_config(|config| {
            let link = config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && &link.storage_id == storage_id
                })
                .ok_or_else(|| "test project link disappeared".to_string())?;
            for (scope, (scope_revision, scope_sha256)) in &coordinates {
                link.scope_bases.insert(
                    scope.clone(),
                    ScopeBase {
                        scope: scope.clone(),
                        scope_revision: *scope_revision,
                        scope_sha256: scope_sha256.clone(),
                        reviewed_in: reviewed_in.clone(),
                        binding_revision: binding.revision,
                        last_pull_at: Some(reviewed_at),
                        last_push_at: None,
                    },
                );
            }
            Ok(())
        })
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_local_conversation(
        repo: &LocalStateRepository,
        binding: &ProjectBinding,
        codex_home: &Path,
        logical_thread_id: &ThreadId,
        logical_parent: Option<ThreadId>,
        provider_thread_id: &str,
        transcript: &[u8],
        _source: &BundleSnapshot,
    ) -> PathBuf {
        let target = codex_home.join("sessions/2026/07/27").join(format!(
            "rollout-2026-07-27T12-00-00-{}.jsonl",
            provider_thread_id
        ));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, transcript).unwrap();
        let analysis = analyze_codex_transcript(transcript);
        assert!(
            analysis.lineage.is_complete(),
            "command test transcript must be resumable",
        );
        let profile_id = binding.profile_ids[&codex_key()].clone();
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_id,
                thread_id: logical_thread_id.clone(),
                forked_from: logical_parent,
                preserved: None,
                root_tail_layout: Some(RootTailLayout {
                    codec: crate::project_sync::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: crate::project_sync::domain::TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: Some(provider_thread_id.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
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

    #[test]
    fn clean_push_review_separates_one_divergence_from_local_recipe_resources() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("clean-conversation-recipe-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        let binding =
            bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let first_provider_id = "019fa660-0000-7000-8000-000000000001";
        let second_provider_id = "019fa660-0000-7000-8000-000000000002";
        let sessions = codex_home.join("sessions/2026/07/28");
        std::fs::create_dir_all(&sessions).unwrap();
        let first_path = sessions.join(format!(
            "rollout-2026-07-28T12-00-00-{first_provider_id}.jsonl"
        ));
        let second_path = sessions.join(format!(
            "rollout-2026-07-28T12-01-00-{second_provider_id}.jsonl"
        ));
        std::fs::write(
            &first_path,
            command_test_transcript(
                first_provider_id,
                &project_root,
                &[("first-base", "user", "first base")],
            ),
        )
        .unwrap();
        std::fs::write(
            &second_path,
            command_test_transcript(
                second_provider_id,
                &project_root,
                &[("second-base", "user", "second base")],
            ),
        )
        .unwrap();

        // The first Push is the clean cutover producer: local capture IDs are
        // persisted in the link recipe while portable conversation IDs exist
        // only in the provider scope of the bundle manifest.
        push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let initial = engine.inspect(&project.bundle_id).unwrap();
        let first_thread = initial
            .manifest
            .all_conversations()
            .find_map(|(_, thread_id, thread)| {
                (thread.source_thread_id.as_deref() == Some(first_provider_id))
                    .then_some(thread_id.clone())
            })
            .unwrap();
        let second_thread = initial
            .manifest
            .all_conversations()
            .find_map(|(_, thread_id, thread)| {
                (thread.source_thread_id.as_deref() == Some(second_provider_id))
                    .then_some(thread_id.clone())
            })
            .unwrap();
        assert_ne!(first_thread, second_thread);

        let config = repo.load_config().unwrap();
        config
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .validate_machine_local()
            .unwrap();
        let link_recipe = project_link(&config, &project.local_project_id, &storage_id)
            .unwrap()
            .recipe
            .as_ref()
            .unwrap();
        link_recipe.validate_machine_local().unwrap();
        assert!(link_recipe
            .entries
            .keys()
            .all(|resource_id| { parse_conversation_resource_id(resource_id.as_str()).is_err() }));

        // Both local sessions advance. Only the first also advances remotely,
        // producing one real divergence and leaving the second safe to Push.
        std::fs::write(
            &first_path,
            command_test_transcript(
                first_provider_id,
                &project_root,
                &[
                    ("first-base", "user", "first base"),
                    ("first-local", "user", "first local branch"),
                ],
            ),
        )
        .unwrap();
        std::fs::write(
            &second_path,
            command_test_transcript(
                second_provider_id,
                &project_root,
                &[
                    ("second-base", "user", "second base"),
                    ("second-local", "user", "second local change"),
                ],
            ),
        )
        .unwrap();
        let (origin_host_token, origin_replica_token) = repo
            .derive_origin_tokens(&project.bundle_id, &binding.replica_id)
            .unwrap();
        let origin = PortableOrigin {
            origin_host_token,
            origin_replica_token,
            shared_label: None,
        };
        let mut first_remote = ThreadSpec::new(
            first_thread.as_str(),
            first_provider_id,
            command_test_transcript(
                first_provider_id,
                &project_root,
                &[
                    ("first-base", "user", "first base"),
                    ("first-remote", "user", "first remote branch"),
                ],
            ),
            origin.clone(),
            20,
        );
        first_remote.title = "First".to_string();
        let mut second_remote = ThreadSpec::new(
            second_thread.as_str(),
            second_provider_id,
            command_test_transcript(
                second_provider_id,
                &project_root,
                &[("second-base", "user", "second base")],
            ),
            origin.clone(),
            20,
        );
        second_remote.title = "Second".to_string();
        let (_, head_token) = engine.read_head(&project.bundle_id).unwrap().unwrap();
        let remote = engine
            .publish(publish_request(
                project.bundle_id.clone(),
                &[first_remote, second_remote],
                origin,
                20,
                Some(&initial.manifest),
                PublishExpectation::Match(head_token),
            ))
            .unwrap()
            .snapshot;
        // The repository head and scope were explicitly reviewed, while the
        // per-thread accepted checkpoint remains the common first-Push base.
        record_reviewed_scope_bases(&repo, &project, &binding, &storage_id, &remote, 20);

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            review.destination_status,
            PushDestinationStatus::Ready { revision: Some(2) }
        );
        let comparison = review.thread_comparison.as_ref().unwrap();
        assert_eq!(comparison.counts.diverged, 1);
        assert_eq!(comparison.counts.local, 1);
        let divergent = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(first_provider_id))
            .unwrap();
        assert_eq!(divergent.state, ThreadSyncState::Diverged);
        assert_eq!(
            divergent.push_decision.action,
            ConversationReviewAction::RequirePull
        );
        assert!(!divergent.push_decision.selectable);
        let pushable = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(second_provider_id))
            .unwrap();
        assert_eq!(pushable.state, ThreadSyncState::LocalAhead);
        assert_eq!(
            pushable.push_decision.action,
            ConversationReviewAction::Publish
        );
        assert!(pushable.push_decision.selectable);
        assert!(!review
            .conversation_action_ids
            .contains_key(&divergent.resource_id));
        assert!(review
            .conversation_action_ids
            .contains_key(&pushable.resource_id));

        assert!(review.inventory.resources.iter().all(|resource| {
            parse_conversation_resource_id(resource.descriptor.resource_id.as_str()).is_err()
        }));
        assert!(review
            .inventory
            .resources
            .iter()
            .all(|resource| { resource.descriptor.kind != ResourceKind::Requirement }));
        review.inventory.recipe.validate_machine_local().unwrap();
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
        assert_eq!(tip.push_decision.action, ConversationReviewAction::Blocked);
        assert_eq!(
            tip.push_decision.blocker,
            Some(ConversationBlocker::UnknownEvidence)
        );
        assert!(!tip.push_decision.selectable);
        let fresh = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(fresh_provider_id))
            .unwrap();
        assert_eq!(
            fresh.push_decision.action,
            ConversationReviewAction::CreateCloud
        );
        assert!(fresh.push_decision.selectable);
        let plan_id = review.conversation_push_plan_id.clone().unwrap();
        let fresh_action_id = review.conversation_action_ids[&fresh.resource_id].clone();
        let plan = repo.load_conversation_push_plan(&plan_id).unwrap();
        assert!(
            !review
                .conversation_action_ids
                .contains_key(&tip.resource_id),
            "blocked review rows must not enter executable plan state"
        );
        assert_eq!(plan.actions.len(), 1);

        let omitted_plan = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            recommended_review_recipe(&review, &storage_id),
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
            recommended_review_recipe(&review, &storage_id),
            empty_project_content_review(),
            Some(plan_id),
            vec![fresh_action_id],
            &mut |_| {},
        )
        .unwrap();
        assert!(pushed.success, "{}", pushed.message);

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let published = engine.inspect(&project.bundle_id).unwrap();
        let published_scope = published.manifest.provider_scope(&codex_key()).unwrap();
        let source_scope = source.manifest.provider_scope(&codex_key()).unwrap();
        assert_eq!(published_scope.conversations.len(), 4);
        // The divergent thread keeps its cloud version, byte for byte.
        let republished_tip = published_scope.conversations.get(&t111).unwrap();
        let source_tip = source_scope.conversations.get(&t111).unwrap();
        assert_eq!(republished_tip.forked_from, source_tip.forked_from);
        let tip_transcript_path = republished_tip
            .files
            .iter()
            .find(|file| file.role == ConversationFileRole::Transcript)
            .unwrap()
            .logical_path
            .clone();
        assert_eq!(
            published_scope
                .files
                .get(&tip_transcript_path)
                .unwrap()
                .sha256,
            source_scope.files.get(&tip_transcript_path).unwrap().sha256,
        );
        // The local divergent branch was neither pushed nor modified.
        assert_eq!(std::fs::read(&local_path).unwrap(), local_tip);
        assert!(published_scope
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
            recipe,
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
        let first_project_scope = first_snapshot
            .manifest
            .scope(&SyncScopeKey::Project)
            .unwrap();
        assert!(first_project_scope.files.contains_key(
            &super::super::domain::LogicalPath::parse("project/docs/specs/a.md").unwrap()
        ));
        assert!(!first_project_scope.files.contains_key(
            &super::super::domain::LogicalPath::parse("project/excluded.txt").unwrap()
        ));
        for directory in ["project/docs", "project/docs/specs", "project/docs/empty"] {
            assert!(first_project_scope
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
            removal_recipe,
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
        assert_eq!(deletion_snapshot.manifest.all_files().count(), 0);
        assert_eq!(deletion_snapshot.manifest.all_directories().count(), 0);
        assert_eq!(
            deletion_snapshot
                .manifest
                .all_tombstones()
                .filter(|(_, _, tombstone)| matches!(
                    tombstone.target,
                    TombstoneTarget::ProjectContentFile { .. }
                ))
                .count(),
            2
        );
        assert_eq!(
            deletion_snapshot
                .manifest
                .all_tombstones()
                .filter(|(_, _, tombstone)| matches!(
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
            reviewed_project_content_recipe(&scanned),
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
            recipe.clone(),
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
            recipe,
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
            reviewed_project_content_recipe(&refreshed),
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
            format!("Published revision {}", published.revision.unwrap())
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
            profile_ids: BTreeMap::from([
                (
                    codex_key(),
                    LocalProviderProfileId::parse("profile-codex").unwrap(),
                ),
                (
                    ProviderKey::parse("claude").unwrap(),
                    LocalProviderProfileId::parse("profile-claude").unwrap(),
                ),
            ]),
            state: BindingState::Active,
            revision: 1,
            updated_at: 1,
        };
        let resolved = ResolvedBinding {
            binding,
            homes: BTreeMap::from([
                (codex_key(), "/tmp/custom codex/.codex".to_string()),
                (
                    ProviderKey::parse("claude").unwrap(),
                    "/tmp/custom claude/.claude".to_string(),
                ),
            ]),
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
        assert_eq!(project_open_commands(&resolved), expected);
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
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        let binding = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"id\":\"m-repair\",\"content\":\"keep the literal {} unchanged\"}}}}\n",
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

        // Pin the stale thread on a storage link so repair must preserve the
        // published wire form and rebase the checkpoint's local column.
        let storage_id = StorageId::parse("repair-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("repair-store"));
        let logical_thread = ThreadId::parse("thread-stale-logical").unwrap();
        let profile_for_binding = binding.profile_ids.get(&codex_key()).cloned().unwrap();
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_for_binding.clone(),
                thread_id: logical_thread.clone(),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(RootTailLayout {
                    codec: mallard_domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: Some(stale_thread.to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: 1,
                updated_at: 1,
            });
            Ok(())
        })
        .unwrap();
        let no_bases = |_: &str| Ok(None);
        let resolved_stale =
            codex_history_graph::resolve_codex_root(&stale_path, &no_bases).unwrap();
        let stale_leaf_bytes = codex_history_graph::read_rollout_bounded(&stale_path).unwrap();
        let seeded_stale =
            codex_history_graph::seed_codex_root(&resolved_stale, &stale_leaf_bytes, None).unwrap();
        let published = build_standalone_codex_root(
            &resolved_stale,
            &StandaloneBuildMode::Encode {
                root: stale_root.to_str().unwrap().to_string(),
                canonical_root: None,
            },
            None,
        )
        .unwrap();
        let published_wire_tail = published.portable_tail.clone();
        repo.mutate_config(|config| {
            let link_checkpoint = ConversationCheckpoint {
                generation: 3,
                accepted: Some(mallard_domain::AcceptedConversationVersion {
                    artifact_sha256: published.artifact_sha256.clone(),
                    accepted_at_commit_id: "commit-repair".to_string(),
                    accepted_at_manifest_sha256: "d".repeat(64),
                    portable_tail: published_wire_tail.clone(),
                }),
                accepted_local_tail: Some(seeded_stale.observed_tail.clone()),
                observed_tail: seeded_stale.observed_tail.clone(),
                physical: PhysicalMappingState::Ready {
                    root_mapping: seeded_stale.mapping.clone(),
                    raw_range_pin: RawRangePin {
                        start_byte_offset: 0,
                        end_byte_offset: seeded_stale.mapping.consumed_byte_offset,
                        range_sha256: hex_digest(
                            Sha256::digest(
                                &stale_leaf_bytes
                                    [..seeded_stale.mapping.consumed_byte_offset as usize],
                            )
                            .as_slice(),
                        ),
                    },
                },
                prefix_witness: None,
                binding_revision: 0,
            };
            config.links.push(ProjectStorageLink {
                local_project_id: binding.local_project_id.clone(),
                storage_id: storage_id.clone(),
                bundle_id: project.bundle_id.clone(),
                recipe: None,
                scope_bases: BTreeMap::new(),
                project_content_preferences: Default::default(),
                conversation_checkpoints: BTreeMap::from([(
                    SyncScopeKey::Provider(codex_key()),
                    BTreeMap::from([(logical_thread.clone(), link_checkpoint)]),
                )]),
                pinned: false,
                created_at: 1,
            });
            Ok(())
        })
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
        // Structural root fields remap. An exact-root prose mention followed
        // by whitespace remains byte-identical because POSIX permits spaces
        // in an outside sibling's component; only end-of-value or `/` proves
        // an exact root or descendant for portable remapping.
        assert!(repaired_source.contains(&format!(
            "keep the literal {} unchanged",
            stale_root.display()
        )));
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

        // §6.5 rebase: the wire pin is untouched, the local column is
        // recomputed from the repaired bytes, the generation advanced, and
        // with no unpushed suffix the thread sits in Sync — nothing to
        // republish. Structural cwd records are outside the semantic fold, so
        // preserving the ambiguous prose literal leaves the fold unchanged.
        let rebased = repo
            .load_config()
            .unwrap()
            .links
            .iter()
            .find(|link| link.storage_id == storage_id)
            .and_then(|link| {
                link.conversation_checkpoints
                    .get(&SyncScopeKey::Provider(codex_key()))
            })
            .and_then(|threads| threads.get(&logical_thread))
            .cloned()
            .unwrap();
        assert_eq!(rebased.generation, 4);
        assert_eq!(
            rebased.accepted.as_ref().unwrap().portable_tail,
            published_wire_tail
        );
        let repaired_resolved =
            codex_history_graph::resolve_codex_root(&stale_path, &no_bases).unwrap();
        let repaired_local_tail = mallard_domain::compute_portable_conversation_tail(
            TailPartitionKind::WholeThread,
            0,
            &repaired_resolved.chain,
        )
        .unwrap();
        assert_eq!(
            rebased.accepted_local_tail,
            Some(repaired_local_tail.clone())
        );
        assert_eq!(rebased.observed_tail, repaired_local_tail);
        assert_eq!(
            rebased.observed_tail.accepted_tail_state,
            seeded_stale.observed_tail.accepted_tail_state,
            "structural-only remap must preserve the local fold"
        );
        rebased.validate().unwrap();
        // Wire invariance end-to-end: re-encoding the repaired bytes under
        // the *new* root reproduces the accepted wire tail exactly.
        let republished = build_standalone_codex_root(
            &repaired_resolved,
            &StandaloneBuildMode::Encode {
                root: binding.project_root.clone(),
                canonical_root: (binding.canonical_project_root != binding.project_root)
                    .then(|| binding.canonical_project_root.clone()),
            },
            None,
        )
        .unwrap();
        assert_eq!(republished.portable_tail, published_wire_tail);
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
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
    fn nested_fork_path_repair_remaps_matcher_scope_under_one_old_root() {
        let active = "019f7500-0000-7000-8000-000000000111";
        let parent = "019f7500-0000-7000-8000-000000000011";
        let old_root = "/tmp/old-project";
        // A realistic moved project: every segment recorded the same old
        // root; sandbox entries and content mentions carry it too, and
        // out-of-root paths must survive byte-identical.
        let source = [
            serde_json::json!({"type":"session_meta","payload":{"id":active,"forked_from_id":parent,"cwd":old_root}}),
            serde_json::json!({"type":"session_meta","payload":{"id":parent,"cwd":old_root}}),
            serde_json::json!({"type":"turn_context","payload":{
                "cwd":"/tmp/old-project/frontend",
                "sandbox_policy": {"writable_roots": [old_root, "/tmp"]},
            }}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","id":"m-a","role":"user","content":[{"type":"input_text","text":"edit /tmp/old-project/src/main.rs and keep /tmp/other/x"}]}}),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();

        let stale =
            inspect_codex_structural_cwds(&source, active, Path::new("/tmp/new-project")).unwrap();
        assert_eq!(stale.len(), 2);
        let mapper_old = CodexPathMapper::new(old_root, None, None).unwrap();
        let mapper_new = CodexPathMapper::new("/tmp/new-project", None, None).unwrap();
        let repaired =
            remap_codex_conversation_paths(&source, active, &mapper_old, &mapper_new, false)
                .unwrap();
        let structure = analyze_codex_rollout_structure(&repaired, Some(active)).unwrap();
        assert_eq!(structure.fork_ancestry, vec![parent.to_string()]);
        assert_eq!(
            structure.structural_cwds,
            BTreeSet::from([
                "/tmp/new-project".to_string(),
                "/tmp/new-project/frontend".to_string()
            ])
        );
        let text = String::from_utf8(repaired.clone()).unwrap();
        assert!(text.contains("edit /tmp/new-project/src/main.rs"));
        assert!(text.contains("keep /tmp/other/x"));
        assert!(
            text.contains("\"/tmp/new-project\",\"/tmp\"")
                || text.contains("\"writable_roots\":[\"/tmp/new-project\",\"/tmp\"]")
        );

        // Pinned-thread gate: the repair transform preserves the wire form
        // byte-exactly, so the gated variant succeeds on the same input …
        remap_codex_conversation_paths(&source, active, &mapper_old, &mapper_new, true).unwrap();
        // … and blocks when content already names the new root (collision).
        let mut colliding = source.clone();
        colliding.push(b'\n');
        colliding.extend_from_slice(
            serde_json::to_string(&serde_json::json!({
                "type":"response_item",
                "payload":{"type":"message","id":"m-b","role":"user",
                            "content":[{"type":"input_text","text":"see /tmp/new-project/readme"}]}
            }))
            .unwrap()
            .as_bytes(),
        );
        let error =
            remap_codex_conversation_paths(&colliding, active, &mapper_old, &mapper_new, true)
                .unwrap_err();
        assert!(
            error.contains("collides with the current project root"),
            "{error}"
        );

        // Distinct out-of-project roots are an audit blocker, not a blanket
        // rewrite: the multi-root fixture from the retired flow now reports
        // ambiguity instead of flattening foreign cwds.
        let multi_root = [
            serde_json::json!({"type":"session_meta","payload":{"id":active,"forked_from_id":parent,"cwd":"/tmp/old-active"}}),
            serde_json::json!({"type":"session_meta","payload":{"id":parent,"cwd":"/tmp/old-parent"}}),
        ]
        .into_iter()
        .map(|record| serde_json::to_string(&record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
        let stale =
            inspect_codex_structural_cwds(&multi_root, active, Path::new("/tmp/new-project"))
                .unwrap();
        assert_eq!(stale.len(), 2);
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
    fn inventory_is_read_only_and_reviewed_push_owns_destination_selection() {
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
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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

        // Inventory reports the saved recipe but does not select or persist
        // newly discovered resources.
        let revision_before_inventory = repo.load_config().unwrap().revision;
        let inventory =
            get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        assert!(inventory.recipe.entries.is_empty());
        assert_eq!(
            repo.load_config().unwrap().revision,
            revision_before_inventory
        );
        get_bundle_inventory_with_repository(&repo, &project.local_project_id).unwrap();
        assert_eq!(
            repo.load_config().unwrap().revision,
            revision_before_inventory,
            "inventory must never mutate the config revision"
        );

        // A newly-created conversation appears in the later Push review. The
        // test helper approves that review's recommendations explicitly.
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
        let portable_conversation_ids = snapshot
            .manifest
            .all_conversations()
            .map(|(_, _, thread)| thread.resource_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(portable_conversation_ids.len(), 3);
        assert!(portable_conversation_ids
            .iter()
            .all(|resource_id| { parse_conversation_resource_id(resource_id.as_str()).is_ok() }));
        let third = snapshot
            .manifest
            .all_conversations()
            .find_map(|(_, _, thread)| {
                (thread.source_thread_id.as_deref() == Some(third_id)).then_some(thread)
            })
            .expect("third conversation was published");
        assert_eq!(third.display_title.as_deref(), Some("Third"));
        let project_recipe = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .clone();
        assert!(project_recipe.entries.is_empty());
        project_recipe.validate_machine_local().unwrap();
        let selected_destination_recipe = repo
            .load_config()
            .unwrap()
            .links
            .iter()
            .find(|link| {
                link.local_project_id == project.local_project_id && link.storage_id == storage_id
            })
            .and_then(|link| link.recipe.clone())
            .unwrap();
        selected_destination_recipe
            .validate_machine_local()
            .unwrap();
        assert!(selected_destination_recipe
            .entries
            .contains_key(&ResourceId::parse(format!("codex:session:{}", third_id)).unwrap()));
        assert!(selected_destination_recipe
            .entries
            .keys()
            .all(|resource_id| { parse_conversation_resource_id(resource_id.as_str()).is_err() }));

        let clean_review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert!(clean_review.inventory.resources.iter().all(|resource| {
            parse_conversation_resource_id(resource.descriptor.resource_id.as_str()).is_err()
        }));
        assert!(clean_review
            .inventory
            .recipe
            .entries
            .keys()
            .all(|resource_id| { parse_conversation_resource_id(resource_id.as_str()).is_err() }));
        // A later reviewed selection belongs only to this destination.
        let pushed_empty = push_bundle_with_recipe_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            BundleRecipe::default(),
        )
        .unwrap();
        assert!(
            !pushed_empty.results.is_empty()
                && pushed_empty
                    .results
                    .iter()
                    .all(|result| result.state == "synced" || result.state == "unchanged"),
            "clearing the destination selection must retain previously published conversations"
        );
        assert!(engine
            .inspect(&project.bundle_id)
            .unwrap()
            .manifest
            .all_conversations()
            .any(|(_, _, thread)| thread.source_thread_id.as_deref() == Some(third_id)));
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
            .is_empty());
        assert!(
            get_bundle_status_with_repository(&repo, &project.local_project_id, &storage_id,)
                .unwrap()
                .statuses
                .is_empty()
        );
        let base = link
            .scope_bases
            .get(&SyncScopeKey::Provider(codex_key()))
            .unwrap();
        assert!(base.last_push_at.is_some());
        assert_eq!(base.last_pull_at, None);
        assert!(!base.reviewed_in.commit_id.is_empty());
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
    fn machine_local_recipe_write_boundaries_reject_portable_conversations() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("recipe-boundary-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        bind_and_link_test_replica(
            &repo,
            &project,
            &storage_id,
            &temp.path().join("project"),
            &temp.path().join("codex-home"),
        );
        let portable_id = conversation_resource_id(
            &codex_key(),
            &ThreadId::parse("thread-machine-local-recipe-boundary").unwrap(),
        )
        .unwrap();
        let mut invalid = BundleRecipe::default();
        invalid.entries.insert(
            portable_id.clone(),
            RecipeEntry {
                resource_id: portable_id.clone(),
                apply_policy: ApplyPolicy::SafeFile,
                required: false,
            },
        );
        let before = repo.load_config().unwrap();

        let save_error =
            save_bundle_recipe_with_repository(&repo, &project.local_project_id, invalid.clone())
                .unwrap_err();
        assert!(
            save_error.contains("machine-local recipe contains portable conversation resource"),
            "{save_error}"
        );
        assert_eq!(repo.load_config().unwrap(), before);

        let push_error = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            invalid,
            empty_project_content_review(),
            None,
            Vec::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            push_error.contains("machine-local recipe contains portable conversation resource"),
            "{push_error}"
        );
        assert_eq!(repo.load_config().unwrap(), before);
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        assert!(engine
            .inspect_optional(&project.bundle_id)
            .unwrap()
            .is_none());
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
    fn saving_existing_project_link_changes_only_pinned() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let project = register(&repo);
        let storage_id = StorageId::parse("backup").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: false,
            },
        )
        .unwrap();

        let observed_tail = mallard_domain::compute_portable_conversation_tail(
            TailPartitionKind::WholeThread,
            0,
            &[mallard_domain::CanonicalConversationMessage {
                message_id: mallard_domain::MessageId::parse("message-link-checkpoint").unwrap(),
                parent_message_id: None,
                semantic_fingerprint: "d".repeat(64),
            }],
        )
        .unwrap();
        let checkpoint = ConversationCheckpoint {
            generation: 5,
            accepted: None,
            accepted_local_tail: None,
            observed_tail,
            physical: PhysicalMappingState::Detached {
                expected_provider_thread_id: "0199a1b2-0000-7000-8000-000000000001".to_string(),
                expected_binding_revision: 8,
            },
            prefix_witness: None,
            binding_revision: 8,
        };
        let excluded_resource_id =
            ResourceId::parse(format!("project:content-file:{}", "e".repeat(64))).unwrap();
        let before = repo
            .mutate_config(|config| {
                let link = config
                    .links
                    .iter_mut()
                    .find(|link| {
                        link.local_project_id == project.local_project_id
                            && link.storage_id == storage_id
                    })
                    .unwrap();
                link.recipe = Some(BundleRecipe {
                    revision: 7,
                    ..BundleRecipe::default()
                });
                link.scope_bases.insert(
                    SyncScopeKey::Project,
                    ScopeBase {
                        scope: SyncScopeKey::Project,
                        scope_revision: 3,
                        scope_sha256: "b".repeat(64),
                        reviewed_in: RepositoryBase {
                            revision: 3,
                            commit_id: "commit-link-state".to_string(),
                            manifest_sha256: "a".repeat(64),
                            manifest_size: 1,
                        },
                        binding_revision: 8,
                        last_pull_at: Some(10),
                        last_push_at: Some(11),
                    },
                );
                link.project_content_preferences.revision = 4;
                link.project_content_preferences
                    .excluded_resource_ids
                    .insert(excluded_resource_id);
                link.conversation_checkpoints.insert(
                    SyncScopeKey::Provider(codex_key()),
                    BTreeMap::from([(
                        ThreadId::parse("thread-link-checkpoint").unwrap(),
                        checkpoint,
                    )]),
                );
                Ok(link.clone())
            })
            .unwrap();
        assert!(!before.conversation_checkpoints.is_empty());

        let updated = save_project_link_with_repository(
            &repo,
            SaveProjectLinkRequest {
                local_project_id: project.local_project_id.clone(),
                storage_id: storage_id.clone(),
                pinned: true,
            },
        )
        .unwrap();
        let mut expected = before;
        expected.pinned = true;
        assert_eq!(updated, expected);

        let persisted = repo
            .load_config()
            .unwrap()
            .links
            .into_iter()
            .find(|link| {
                link.local_project_id == project.local_project_id && link.storage_id == storage_id
            })
            .unwrap();
        assert_eq!(persisted, expected);
        assert!(!persisted.conversation_checkpoints.is_empty());
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
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap()
                .scope_bases
                .insert(
                    SyncScopeKey::Project,
                    ScopeBase {
                        scope: SyncScopeKey::Project,
                        scope_revision: 1,
                        scope_sha256: "b".repeat(64),
                        reviewed_in: RepositoryBase {
                            revision: 1,
                            commit_id: "commit-1".to_string(),
                            manifest_sha256: "a".repeat(64),
                            manifest_size: 1,
                        },
                        binding_revision: 1,
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
        // The reviewed scope bases live on the link, so unlinking removes the
        // machine-local review state with it.
        assert!(config.links.is_empty());
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
    fn storage_clear_deletes_only_managed_remote_namespaces_and_keeps_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let catalog =
            CatalogStateRepository::from_home_dir(temp.path().join("catalog-home")).unwrap();
        let storage_id = StorageId::parse("storage-clear").unwrap();
        let storage_root = temp.path().join("storage");
        std::fs::create_dir_all(&storage_root).unwrap();
        let mut config = repo.load_config().unwrap();
        config.storages.push(StorageConfig {
            id: storage_id.clone(),
            name: "Clear me".to_string(),
            kind: StorageKind::Local,
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            account_id: String::new(),
            s3_endpoint: String::new(),
            region: String::new(),
            local_dir: storage_root.to_string_lossy().into_owned(),
            supports_conditional_writes: None,
        });
        let saved = repo.save_config(config).unwrap();
        let store = LocalObjectStore::open(&storage_root).unwrap();
        let repository_key = repository_object_key(
            &BundleId::parse("11111111111111111111111111111111").unwrap(),
            "_head.json",
        )
        .unwrap();
        let library_key =
            library_object_key("library-22222222222222222222222222222222", "_head.json").unwrap();
        store.put_immutable(&repository_key, b"repository").unwrap();
        store.put_immutable(&library_key, b"library").unwrap();
        std::fs::write(storage_root.join("keep.txt"), b"user data").unwrap();

        let result =
            clear_storage_data_with_repositories(&repo, &catalog, &storage_id, saved.revision)
                .unwrap();

        assert_eq!(result.repository_objects_deleted, 1);
        assert_eq!(result.library_objects_deleted, 1);
        assert!(store.get_metadata(&repository_key, 64).unwrap().is_none());
        assert!(store.get_metadata(&library_key, 64).unwrap().is_none());
        assert!(storage_root.join(STORAGE_MARKER_KEY).is_file());
        assert_eq!(
            std::fs::read(storage_root.join("keep.txt")).unwrap(),
            b"user data"
        );
        assert!(repo
            .load_config()
            .unwrap()
            .storages
            .iter()
            .any(|storage| storage.id == storage_id));
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
        let source_codex = temp.path().join("codex-home");
        std::fs::create_dir_all(&source_root).unwrap();
        std::fs::create_dir_all(source_codex.join("sessions/2026/07/18")).unwrap();
        let source_session_id = "019f742a-a206-7932-876c-9db8d8ce575a";
        std::fs::write(
            source_codex
                .join("sessions/2026/07/18")
                .join(format!("rollout-{source_session_id}.jsonl")),
            command_test_transcript(
                source_session_id,
                &source_root,
                &[("message-source", "user", "hello")],
            ),
        )
        .unwrap();
        std::fs::write(
            source_codex.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"Source conversation\"}}\n",
                source_session_id
            ),
        )
        .unwrap();
        let profile_id = add_profile(&repo, &codex_key(), &source_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id.clone())]),
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
        let (_, source_engine) = storage_engine(&repo, &storage_id).unwrap();
        let source_snapshot = source_engine.inspect(&source_bundle).unwrap();
        let portable_conversation_id = source_snapshot
            .manifest
            .all_conversations()
            .next()
            .map(|(_, _, conversation)| conversation.resource_id.clone())
            .expect("source Push publishes its conversation");
        assert!(source_snapshot
            .manifest
            .provider_scope(&codex_key())
            .unwrap()
            .recipe
            .entries
            .contains_key(&portable_conversation_id));

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
                profile_ids: BTreeMap::from([(codex_key(), profile_id.clone())]),
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
                profile_ids: BTreeMap::from([(codex_key(), profile_id.clone())]),
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
        assert!(!connected
            .project
            .recipe
            .entries
            .contains_key(&portable_conversation_id));
        let binding = connected.binding.unwrap();
        assert_eq!(binding.bundle_id, connected.project.bundle_id);
        assert_eq!(binding.project_root, replica_root.to_string_lossy());
        assert_eq!(binding.profile_ids.get(&codex_key()), Some(&profile_id));
        assert_eq!(
            get_bundle_status_with_repository(&repo, &replica.local_project_id, &storage_id)
                .unwrap()
                .revision,
            Some(1)
        );
        let inventory =
            get_bundle_inventory_with_repository(&repo, &replica.local_project_id).unwrap();
        assert!(inventory.resources.iter().all(|resource| {
            resource.descriptor.resource_id != portable_conversation_id
                || resource.descriptor.kind != ResourceKind::Requirement
        }));
        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &replica.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert!(comparison.entries.iter().any(|entry| {
            entry.resource_id == portable_conversation_id
                && entry.state == ThreadSyncState::StorageOnly
        }));

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
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
    fn pull_support_uses_the_restore_plans_storage_when_linked_revisions_differ() {
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
                supports_conditional_writes: None,
            },
        ]);
        repo.save_config(config).unwrap();

        let project = register(&repo);
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let profile_id = add_profile(&repo, &codex_key(), &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
        // revision on its storage.
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
                        "ordinal": turn as u64,
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
        assert_eq!(restore.revision, 3);
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
        assert_eq!(first.revision, Some(1));
        assert!(
            first.message.contains("Published revision 1"),
            "{}",
            first.message
        );
        let (first_base, first_scopes) =
            reviewed_link_state(&repo, &project.local_project_id, &storage_id);

        let repeat =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert!(repeat.success);
        assert_eq!(repeat.revision, Some(1));
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

        let (repeat_base, repeat_scopes) =
            reviewed_link_state(&repo, &project.local_project_id, &storage_id);
        assert_eq!(repeat_base.revision, first_base.revision);
        assert_eq!(repeat_base.manifest_sha256, first_base.manifest_sha256);
        let codex_scope = SyncScopeKey::Provider(codex_key());
        assert!(
            repeat_scopes[&codex_scope].last_push_at >= first_scopes[&codex_scope].last_push_at
        );

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let snapshot = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(snapshot.head.revision, 1);
        assert_eq!(repeat_base.manifest_size, snapshot.head.manifest_size);
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
        assert_eq!(first.revision, Some(1));
        let reviewed = reviewed_link_state(&repo, &project.local_project_id, &storage_id);
        let prepared_before_remote_loss =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            prepared_before_remote_loss.destination_status,
            PushDestinationStatus::Ready { revision: Some(1) }
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
            PushDestinationStatus::RemoteHistoryMissing { base_revision: 1 }
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

        let retained = reviewed_link_state(&repo, &project.local_project_id, &storage_id);
        assert_eq!(retained, reviewed);

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
            PushDestinationStatus::Ready { revision: None }
        );

        let republished =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(republished.revision, Some(1));
    }

    /// The two-replica incident from DESIGN_REMOTE_HISTORY_CONTINUITY.md:
    /// replica A loses the remote, unlinks, relinks, and republishes under
    /// the same bundle id. Replica B — simulated by restoring the pre-wipe
    /// base — must see the recreated history as replaced on every surface,
    /// be blocked from pushing, and be healed by an ordinary pull.
    #[test]
    fn replaced_remote_history_blocks_push_on_every_surface_and_pull_heals_it() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("replaced-history-store").unwrap();
        let storage_root = temp.path().join("store");
        add_local_storage(&repo, &storage_id, &storage_root);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        write_codex_session(
            &codex_home,
            "019f7500-0000-7000-8000-0000000000bd",
            &project_root,
        );

        let first =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));
        let (pre_wipe_base, pre_wipe_scopes) =
            reviewed_link_state(&repo, &project.local_project_id, &storage_id);

        // The wipe, then replica A's sanctioned recovery: unlink (drops the
        // base), relink, republish — same bundle id, brand-new lineage.
        let repository_root = storage_root
            .join(".mallard/v1/repositories")
            .join(project.bundle_id.as_str());
        std::fs::remove_dir_all(&repository_root).unwrap();
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
        let recreated =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(recreated.revision, Some(1));
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let recreated_head = engine.inspect(&project.bundle_id).unwrap().head;
        assert_ne!(
            recreated_head.manifest_sha256, pre_wipe_base.manifest_sha256,
            "the recreated lineage must not reproduce the wiped manifest"
        );

        // Replica B: same binding, but its recorded bases name the wiped
        // lineage.
        repo.mutate_config(|config| {
            config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap()
                .scope_bases = pre_wipe_scopes.clone();
            Ok(())
        })
        .unwrap();

        // Push review blocks with the replaced-history status (row 5: equal
        // revisions, different digests — no probe read needed)...
        let context =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        assert_eq!(
            context.destination_status,
            PushDestinationStatus::RemoteHistoryReplaced {
                base_revision: 1,
                remote_revision: 1,
                base_manifest_sha256: pre_wipe_base.manifest_sha256.clone(),
                remote_manifest_sha256: recreated_head.manifest_sha256.clone(),
            }
        );
        // ...the comparison shown by refresh carries the same continuity...
        assert_eq!(
            context
                .thread_comparison
                .as_ref()
                .expect("thread comparison")
                .remote_continuity,
            RemoteContinuity::Replaced {
                base_revision: 1,
                remote_revision: 1
            }
        );
        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            comparison.remote_continuity,
            RemoteContinuity::Replaced {
                base_revision: 1,
                remote_revision: 1
            }
        );
        // ...and the push preflight fails with the same diagnosis.
        let error =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap_err();
        assert!(error.contains("was replaced"), "got: {error}");
        assert!(error.contains("pull and review"), "got: {error}");

        // The pull plan carries the verdict, and applying it heals the base.
        let binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&project.local_project_id)
            .cloned()
            .unwrap();
        let plan =
            plan_bundle_restore_with_repository(&repo, &storage_id, &project.bundle_id, &binding)
                .unwrap();
        assert_eq!(
            plan.remote_continuity,
            Some(RemoteContinuity::Replaced {
                base_revision: 1,
                remote_revision: 1
            })
        );
        apply_bundle_restore_with_repository(
            &repo,
            &plan.plan_id,
            plan.actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect(),
        )
        .unwrap();
        assert_eq!(
            get_push_destination_status_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap(),
            PushDestinationStatus::Ready { revision: Some(1) }
        );
        assert_eq!(
            get_project_thread_sync_comparison_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap()
            .remote_continuity,
            RemoteContinuity::InSync { revision: 1 }
        );
    }

    /// Revision-ahead disambiguation: a recreated lineage that overtook
    /// the base classifies `Replaced` (probe misses the wiped manifest),
    /// while a base from the live lineage classifies a plain advance (probe
    /// finds its manifest still present).
    #[test]
    fn replaced_history_that_overtook_the_base_is_distinguished_from_an_advance() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("overtaken-history-store").unwrap();
        let storage_root = temp.path().join("store");
        add_local_storage(&repo, &storage_id, &storage_root);
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let session_id = "019f7500-0000-7000-8000-0000000000be";
        write_codex_session(&codex_home, session_id, &project_root);

        let first =
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));
        let (pre_wipe_base, pre_wipe_scopes) =
            reviewed_link_state(&repo, &project.local_project_id, &storage_id);

        // Wipe and recreate under the same bundle id.
        std::fs::remove_dir_all(
            storage_root
                .join(".mallard/v1/repositories")
                .join(project.bundle_id.as_str()),
        )
        .unwrap();
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
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap()
                .revision,
            Some(1)
        );
        let (_, new_lineage_scopes) =
            reviewed_link_state(&repo, &project.local_project_id, &storage_id);

        // Grow the session so the new lineage overtakes the base revision.
        std::fs::write(
            codex_home
                .join("sessions/2026/07/22")
                .join(format!("rollout-{}.jsonl", session_id)),
            command_test_transcript(
                session_id,
                &project_root,
                &[
                    ("message-test", "user", "hello"),
                    ("message-more", "user", "and more"),
                ],
            ),
        )
        .unwrap();
        assert_eq!(
            push_bundle_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap()
                .revision,
            Some(2)
        );

        // A base from the wiped lineage: the probe misses ⇒ replaced.
        repo.mutate_config(|config| {
            config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap()
                .scope_bases = pre_wipe_scopes.clone();
            Ok(())
        })
        .unwrap();
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let live_head = engine.inspect(&project.bundle_id).unwrap().head;
        assert_eq!(
            get_push_destination_status_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap(),
            PushDestinationStatus::RemoteHistoryReplaced {
                base_revision: 1,
                remote_revision: 2,
                base_manifest_sha256: pre_wipe_base.manifest_sha256.clone(),
                remote_manifest_sha256: live_head.manifest_sha256.clone(),
            }
        );

        // A base from the live lineage: the probe finds its manifest still
        // present ⇒ an ordinary advance that a pull fast-forwards.
        repo.mutate_config(|config| {
            config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap()
                .scope_bases = new_lineage_scopes.clone();
            Ok(())
        })
        .unwrap();
        assert_eq!(
            get_push_destination_status_with_repository(
                &repo,
                &project.local_project_id,
                &storage_id,
            )
            .unwrap(),
            PushDestinationStatus::RequiresPull {
                reason: PushDestinationPullReason::RemoteAdvanced,
                remote_revision: 2,
                base_revision: Some(1),
                blocking_scopes: vec!["provider:codex".to_string()],
            }
        );
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
    fn activity_workspace_projects_one_local_and_remote_read_into_both_views() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("activity-workspace-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let session_id = "019f7500-0000-7000-8000-0000000000ab";
        write_codex_session(&codex_home, session_id, &project_root);

        let workspace = get_project_activity_workspace_with_repository(
            &repo,
            &project.local_project_id,
            Some(&storage_id),
            None,
            None,
            Some(30),
            false,
        )
        .unwrap();
        assert!(workspace.comparison_error.is_none());
        let comparison = workspace.comparison.unwrap();
        assert_eq!(workspace.activity.sessions.len(), 1);
        assert_eq!(comparison.entries.len(), 1);
        assert_eq!(
            comparison.entries[0].conversation_resource_id.as_str(),
            workspace.activity.sessions[0].resource_id
        );
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

        // Standalone inventory and the composite review are both read-only and
        // must build the same sections from the same saved recipe.
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
        // Preparing either view must not mutate the project recipe.
        let recipe = repo
            .load_config()
            .unwrap()
            .project(&project.local_project_id)
            .unwrap()
            .recipe
            .clone();
        assert!(recipe.entries.is_empty());
        assert_eq!(
            context.project.project.local_project_id,
            project.local_project_id
        );
        assert_eq!(
            context.destination_status,
            PushDestinationStatus::Ready { revision: None }
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
        assert!(!review
            .inventory
            .recipe
            .entries
            .contains_key(&child_resource));
        // Inventory sanitizes the effective recipe without rewriting project
        // configuration; the reviewed destination recipe is what Push saves.
        assert!(repo
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
        let codex_scope = snapshot.manifest.provider_scope(&codex_key()).unwrap();
        assert_eq!(codex_scope.conversations.len(), 1);
        assert!(codex_scope
            .conversations
            .values()
            .all(|thread| { thread.source_thread_id.as_deref() != Some(child_id) }));
        assert!(!codex_scope.recipe.entries.contains_key(&child_resource));
    }

    /// Push excludes subagent rollouts at capture, so post-cutover manifests
    /// cannot legitimately contain one.  This fixture writes one directly
    /// into storage to pin the split introduced by manifest-only refresh:
    /// the comparison trusts the manifest and surfaces the stored thread
    /// without opening it, while Pull planning — which does read payloads —
    /// still refuses to offer it.  If refresh ever starts hiding the row
    /// again, payload reads have crept back into the manifest-only path.
    /// Pinned by docs/PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md §5:
    /// a rollout crossing the conversation bound by append surfaces the
    /// identical blocked row whether the cached-inventory path or a cold
    /// rediscovery detects it.
    #[test]
    fn an_oversize_append_blocks_identically_under_cache_and_rediscovery() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("oversize-parity-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let provider_id = "019fb004-0000-7000-8000-00000000dd05";
        let base = command_test_transcript(provider_id, &project_root, &[("m-1", "user", "hi")]);
        let session_path = codex_home
            .join("sessions/2026/07/28")
            .join(format!("rollout-2026-07-28T12-00-00-{provider_id}.jsonl"));
        std::fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        std::fs::write(&session_path, &base).unwrap();

        // Warm the caches, then grow the file past the bound in place.
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&session_path)
            .unwrap()
            .set_len(MAX_CONVERSATION_FILE_BYTES + 1)
            .unwrap();
        let cached_path = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        let canonical_home = std::fs::canonicalize(&codex_home).unwrap();
        super::super::codex_watch_service::session_watch_service()
            .force_rebaseline(&canonical_home);
        let rediscovered = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&cached_path.entries).unwrap(),
            serde_json::to_value(&rediscovered.entries).unwrap(),
            "oversize detection is stage-independent"
        );
        let row = cached_path
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(provider_id))
            .unwrap();
        assert_eq!(row.state, ThreadSyncState::Unavailable);
        assert!(
            row.status_detail
                .as_deref()
                .unwrap_or_default()
                .contains("exceeds the 100 MiB per-file sync limit"),
            "blocked detail comes from the discovery mapping: {:?}",
            row.status_detail
        );
    }

    /// Pinned by docs/PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md §4.6:
    /// Push review captures exactly its byte-pinning candidates, Pull
    /// review captures none for synced/local-only rows (adopting cloud
    /// metadata pins no local bytes), and a quiet second review does no
    /// observation or discovery filesystem work at all.
    #[test]
    fn reviews_capture_only_byte_pinning_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("lazy-candidate-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        let binding =
            bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let synced_id = "019fb003-0000-7000-8000-00000000cc03";
        let synced = command_test_transcript(synced_id, &project_root, &[("s-1", "user", "hi")]);
        let synced_path = codex_home
            .join("sessions/2026/07/28")
            .join(format!("rollout-2026-07-28T12-00-00-{synced_id}.jsonl"));
        std::fs::create_dir_all(synced_path.parent().unwrap()).unwrap();
        std::fs::write(&synced_path, &synced).unwrap();
        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let entry_id = review
            .thread_comparison
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(synced_id))
            .map(|entry| review.conversation_action_ids[&entry.resource_id].clone())
            .unwrap();
        push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            recommended_review_recipe(&review, &storage_id),
            empty_project_content_review(),
            review.conversation_push_plan_id.clone(),
            vec![entry_id],
            &mut |_| {},
        )
        .unwrap();

        // A second, local-only session is the sole push candidate.
        let local_only_id = "019fb003-0000-7000-8000-00000000cc04";
        let local_only =
            command_test_transcript(local_only_id, &project_root, &[("l-1", "user", "new")]);
        let local_only_path = codex_home
            .join("sessions/2026/07/28")
            .join(format!("rollout-2026-07-28T12-00-00-{local_only_id}.jsonl"));
        std::fs::write(&local_only_path, &local_only).unwrap();

        let canonical_home = std::fs::canonicalize(&codex_home).unwrap();
        let metrics = super::super::sync_metrics::register_root(&canonical_home);
        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let comparison = review.thread_comparison.as_ref().unwrap();
        assert_eq!(comparison.entries.len(), 2);
        let counted = metrics.snapshot();
        assert_eq!(
            counted.selected_captures, 1,
            "only the CreateCloud candidate is captured"
        );

        // Quiet second review: inventory, observations, and the candidate's
        // frozen byte pin all come from the caches — the review does no
        // local filesystem work at all.
        let before = metrics.snapshot();
        prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id).unwrap();
        let delta = metrics.snapshot().delta_since(&before);
        assert_eq!(delta.selected_captures, 0, "candidate pin reused");
        assert_eq!(delta.files_head_read, 0, "inventory reused");
        assert_eq!(delta.probe_bytes_read, 0, "observations reused");
        assert_eq!(delta.full_graph_resolves, 0);
        assert_eq!(delta.transcript_bytes_read, 0, "no content read at all");

        // Pull review: the durably pinned synced row plans no action at all,
        // so only the local-only informational row remains — zero captures.
        let before = metrics.snapshot();
        let restore =
            plan_bundle_restore_with_repository(&repo, &storage_id, &project.bundle_id, &binding)
                .unwrap();
        assert!(restore.conversation_actions.is_empty());
        assert_eq!(
            restore.conversation_review[0].action,
            ConversationReviewAction::ReportLocalOnly
        );
        let delta = metrics.snapshot().delta_since(&before);
        assert_eq!(delta.selected_captures, 0, "pull review captures nothing");
        assert_eq!(delta.transcript_bytes_read, 0);
    }

    /// Pinned by docs/PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md §4.3:
    /// Refresh persists the advanced observed checkpoint under a generation
    /// bump, preserves `accepted` byte-for-byte, skips no-op replacements,
    /// and never reattaches a Detached mapping.
    #[test]
    fn refresh_persists_the_advanced_observed_checkpoint_and_never_touches_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("observe-maintenance-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let provider_id = "019fb001-0000-7000-8000-00000000aa01";
        let base = command_test_transcript(provider_id, &project_root, &[("m-1", "user", "hello")]);
        let session_path = codex_home
            .join("sessions/2026/07/28")
            .join(format!("rollout-2026-07-28T12-00-00-{provider_id}.jsonl"));
        std::fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        std::fs::write(&session_path, &base).unwrap();

        // Push binds the session and writes acceptance evidence + checkpoint.
        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let comparison = review.thread_comparison.as_ref().unwrap();
        let entry = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(provider_id))
            .unwrap();
        let action_id = review.conversation_action_ids[&entry.resource_id].clone();
        push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            recommended_review_recipe(&review, &storage_id),
            empty_project_content_review(),
            review.conversation_push_plan_id.clone(),
            vec![action_id],
            &mut |_| {},
        )
        .unwrap();

        let checkpoint_of = |repo: &LocalStateRepository| {
            let config = repo.load_config().unwrap();
            let link = config
                .links
                .iter()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap();
            let threads = link
                .conversation_checkpoints
                .get(&SyncScopeKey::Provider(codex_key()))
                .unwrap();
            assert_eq!(threads.len(), 1);
            threads.values().next().unwrap().clone()
        };
        let after_push = checkpoint_of(&repo);
        let accepted = after_push
            .accepted
            .clone()
            .expect("push records acceptance");

        // A no-change refresh replaces nothing.
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(checkpoint_of(&repo), after_push);

        // One appended record: refresh persists the advanced cursor under
        // the next generation while preserving accepted exactly.
        let extended = command_test_transcript(
            provider_id,
            &project_root,
            &[("m-1", "user", "hello"), ("m-2", "assistant", "appended")],
        );
        assert_eq!(&extended[..base.len()], &base[..]);
        std::fs::write(&session_path, &extended).unwrap();
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        let after_append = checkpoint_of(&repo);
        assert_eq!(after_append.generation, after_push.generation + 1);
        assert_eq!(after_append.accepted, Some(accepted));
        assert!(
            after_append.observed_tail.accepted_end_record_count
                > after_push.observed_tail.accepted_end_record_count
        );
        let PhysicalMappingState::Ready {
            root_mapping,
            raw_range_pin,
        } = &after_append.physical
        else {
            panic!("advanced checkpoint keeps a ready mapping");
        };
        assert_eq!(root_mapping.consumed_byte_offset, extended.len() as u64);
        assert_eq!(raw_range_pin.end_byte_offset, extended.len() as u64);

        // The next refresh rides the persisted cursor: no further churn.
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(checkpoint_of(&repo), after_append);

        // A Detached mapping is never reattached by observation.
        repo.mutate_config(|config| {
            let link = config
                .links
                .iter_mut()
                .find(|link| {
                    link.local_project_id == project.local_project_id
                        && link.storage_id == storage_id
                })
                .unwrap();
            let threads = link
                .conversation_checkpoints
                .get_mut(&SyncScopeKey::Provider(codex_key()))
                .unwrap();
            for checkpoint in threads.values_mut() {
                checkpoint.physical = PhysicalMappingState::Detached {
                    expected_provider_thread_id: provider_id.to_string(),
                    expected_binding_revision: checkpoint.binding_revision,
                };
            }
            Ok(())
        })
        .unwrap();
        let detached = checkpoint_of(&repo);
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(checkpoint_of(&repo), detached);
    }

    /// Pinned by docs/PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md §6.1: once
    /// a Codex thread rides a ready checkpoint, a plain comparison performs
    /// no capture and reads no transcript bytes beyond the probe window —
    /// and one appended record costs only the probe plus the append.
    #[test]
    fn a_comparison_reads_no_codex_transcript_bytes_beyond_the_probe_window() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("metadata-refresh-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let provider_id = "019fb002-0000-7000-8000-00000000bb02";
        let base = command_test_transcript(
            provider_id,
            &project_root,
            &[("m-1", "user", "hello"), ("m-2", "assistant", "answer")],
        );
        let session_path = codex_home
            .join("sessions/2026/07/28")
            .join(format!("rollout-2026-07-28T12-00-00-{provider_id}.jsonl"));
        std::fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        std::fs::write(&session_path, &base).unwrap();

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let comparison = review.thread_comparison.as_ref().unwrap();
        let entry = comparison
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(provider_id))
            .unwrap();
        let action_id = review.conversation_action_ids[&entry.resource_id].clone();
        push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            recommended_review_recipe(&review, &storage_id),
            empty_project_content_review(),
            review.conversation_push_plan_id.clone(),
            vec![action_id],
            &mut |_| {},
        )
        .unwrap();

        // Warm no-change comparison: no capture, no full resolve, no
        // transcript bytes, and the push review's inventory is reused so no
        // head scan runs either. The applied push moved the checkpoint
        // generation, so the cursor revalidates through one probe window.
        let canonical_home = std::fs::canonicalize(&codex_home).unwrap();
        let metrics = super::super::sync_metrics::register_root(&canonical_home);
        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(comparison.entries.len(), 1);
        let counted = metrics.snapshot();
        assert_eq!(counted.selected_captures, 0, "no codex capture on refresh");
        assert_eq!(counted.transcript_bytes_read, 0, "no whole-file reads");
        assert_eq!(counted.full_graph_resolves, 0, "cursor rides the probe");
        assert_eq!(
            counted.files_head_read, 0,
            "push review's inventory is reused"
        );
        assert!(counted.probe_bytes_read > 0);
        assert_eq!(counted.append_bytes_read, 0);

        // One appended record: the refresh reads the probe plus exactly the
        // appended bytes and nothing else.
        let extended = command_test_transcript(
            provider_id,
            &project_root,
            &[
                ("m-1", "user", "hello"),
                ("m-2", "assistant", "answer"),
                ("m-3", "user", "appended"),
            ],
        );
        assert_eq!(&extended[..base.len()], &base[..]);
        std::fs::write(&session_path, &extended).unwrap();
        // A distinct mtime makes presentation-staleness deterministic: the
        // dirty root's row must carry this stamp, cached or rebaselined.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&session_path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_800_000_000))
            .unwrap();
        let before_append = metrics.snapshot();
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        let delta = metrics.snapshot().delta_since(&before_append);
        assert_eq!(delta.selected_captures, 0);
        assert_eq!(delta.transcript_bytes_read, 0);
        assert_eq!(delta.full_graph_resolves, 0);
        assert_eq!(
            delta.append_bytes_read,
            (extended.len() - base.len()) as u64,
            "the append is the only content read"
        );
        assert_eq!(delta.records_parsed, 1);

        let appended_row = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            appended_row.entries[0].local_updated_at,
            Some(1_800_000_000),
            "a dirty root's presentation stamp is fresh even under reuse"
        );

        // Quiet watcher: the next comparison serves inventory and
        // observation from the caches with zero local filesystem work.
        let before_quiet = metrics.snapshot();
        let quiet = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        let delta = metrics.snapshot().delta_since(&before_quiet);
        assert_eq!(delta.files_head_read, 0, "inventory reused, no head scan");
        assert_eq!(delta.directories_visited, 0, "no tree walk");
        assert_eq!(delta.probe_bytes_read, 0, "observation reused, no probe");
        assert_eq!(delta.append_bytes_read, 0);
        assert_eq!(delta.transcript_bytes_read, 0);
        assert_eq!(delta.inventory_reuse_hits, 1);
        assert_eq!(delta.observation_reuse_hits, 1);

        // A forced rebaseline (the reliability-signal path) runs the
        // bounded pipeline and produces identical rows.
        let canonical = std::fs::canonicalize(&codex_home).unwrap();
        super::super::codex_watch_service::session_watch_service().force_rebaseline(&canonical);
        let before_cold = metrics.snapshot();
        let rebaselined = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        let delta = metrics.snapshot().delta_since(&before_cold);
        assert_eq!(delta.inventory_reuse_hits, 0);
        assert_eq!(delta.observation_reuse_hits, 0);
        assert!(delta.files_head_read > 0, "rebaseline rediscovers");
        assert_eq!(
            serde_json::to_value(&quiet.entries).unwrap(),
            serde_json::to_value(&rebaselined.entries).unwrap(),
            "cached and rebaselined comparisons produce identical rows"
        );
    }

    #[test]
    fn previously_stored_subagent_tasks_are_hidden_from_pull_but_not_refresh() {
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
        assert_eq!(published.snapshot.manifest.all_conversations().count(), 2);

        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(comparison.entries.len(), 2);
        let mut cloud_ids = comparison
            .entries
            .iter()
            .map(|entry| entry.cloud_thread_id.as_ref().map(ThreadId::as_str))
            .collect::<Vec<_>>();
        cloud_ids.sort_unstable();
        assert_eq!(
            cloud_ids,
            vec![Some("stored-parent"), Some("stored-subagent")]
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

    /// Counts payload traffic through the planning byte source. Pinned by
    /// docs/DESIGN_PUSH_PULL_FLOW.md: refresh and Push review read no cloud
    /// payloads at all; Pull planning opens exactly the transcripts it could
    /// offer to materialize, through one batched prefetch.
    struct CountingBytes<'a> {
        inner: &'a mut dyn BundleBytes,
        gets: Vec<LogicalPath>,
        prefetches: Vec<Vec<LogicalPath>>,
    }

    impl BundleBytes for CountingBytes<'_> {
        fn body(&mut self, logical_path: &LogicalPath) -> Result<VerifiedBody, String> {
            self.gets.push(logical_path.clone());
            self.inner.body(logical_path)
        }

        fn read_bounded(
            &mut self,
            logical_path: &LogicalPath,
            max_bytes: u64,
            label: &str,
        ) -> Result<Vec<u8>, String> {
            self.gets.push(logical_path.clone());
            self.inner.read_bounded(logical_path, max_bytes, label)
        }

        fn prefetch(&mut self, paths: &[LogicalPath]) -> Result<(), String> {
            self.prefetches.push(paths.to_vec());
            self.inner.prefetch(paths)
        }
    }

    #[test]
    fn planning_reads_cloud_payloads_only_for_offerable_pull_threads() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("planning-cost-store").unwrap();
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
        // Two stored threads with no local copy — the offerable set — and
        // one local-only session, which needs no cloud bytes in any mode.
        let stored_a = ThreadSpec::new(
            "stored-a",
            "019f7600-0000-7000-8000-0000000000a0",
            command_test_transcript(
                "019f7600-0000-7000-8000-0000000000a0",
                &project_root,
                &[("message-a", "user", "stored a")],
            ),
            origin.clone(),
            10,
        );
        let stored_b = ThreadSpec::new(
            "stored-b",
            "019f7600-0000-7000-8000-0000000000b0",
            command_test_transcript(
                "019f7600-0000-7000-8000-0000000000b0",
                &project_root,
                &[("message-b", "user", "stored b")],
            ),
            origin.clone(),
            11,
        );
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        engine
            .publish(publish_request(
                project.bundle_id.clone(),
                &[stored_a, stored_b],
                origin,
                11,
                None,
                PublishExpectation::Absent,
            ))
            .unwrap();
        write_codex_session(
            &codex_home,
            "019f7600-0000-7000-8000-0000000000c0",
            &project_root,
        );

        let snapshot = engine.inspect(&project.bundle_id).unwrap();
        let resolved = resolve_project_binding(&repo, &binding).unwrap();
        let (views, captured) = capture_all_local_conversations(&repo, &resolved).unwrap();
        let now = now_secs();
        let offerable = snapshot
            .manifest
            .all_conversations()
            .flat_map(|(_, _, thread)| thread.files.iter())
            .filter(|file| file.role == ConversationFileRole::Transcript)
            .map(|file| file.logical_path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(offerable.len(), 2);

        for purpose in [ReviewPurpose::Observe, ReviewPurpose::PushPlan] {
            let mut inner = engine.byte_source(&snapshot, None, None);
            let mut bytes = CountingBytes {
                inner: &mut inner,
                gets: Vec::new(),
                prefetches: Vec::new(),
            };
            thread_review(
                &repo,
                Some(&snapshot),
                &mut bytes,
                &views,
                None,
                &captured,
                &BTreeMap::new(),
                &resolved,
                Some(&snapshot.storage_id),
                now,
                now.saturating_add(PLAN_LIFETIME_SECS),
                purpose,
            )
            .unwrap();
            assert!(
                bytes.gets.is_empty() && bytes.prefetches.is_empty(),
                "refresh and Push review read no cloud payloads ({purpose:?}): \
                 gets={:?} prefetches={:?}",
                bytes.gets,
                bytes.prefetches,
            );
        }

        let mut inner = engine.byte_source(&snapshot, None, None);
        let mut bytes = CountingBytes {
            inner: &mut inner,
            gets: Vec::new(),
            prefetches: Vec::new(),
        };
        let review = thread_review(
            &repo,
            Some(&snapshot),
            &mut bytes,
            &views,
            None,
            &captured,
            &BTreeMap::new(),
            &resolved,
            Some(&snapshot.storage_id),
            now,
            now.saturating_add(PLAN_LIFETIME_SECS),
            ReviewPurpose::PullPlan,
        )
        .unwrap();
        assert_eq!(review.pull_actions.len(), 2);
        assert_eq!(review.pull_review.len(), 3);
        assert_eq!(
            bytes.prefetches.len(),
            1,
            "one batched prefetch covers the offerable set"
        );
        assert_eq!(
            bytes.prefetches[0].iter().cloned().collect::<BTreeSet<_>>(),
            offerable
        );
        assert!(
            bytes
                .gets
                .iter()
                .all(|logical_path| offerable.contains(logical_path)),
            "pull planning opens only offerable transcripts: {:?}",
            bytes.gets,
        );

        let oversized_path = offerable.iter().next().unwrap().clone();
        let mut oversized_snapshot = snapshot.clone();
        oversized_snapshot
            .manifest
            .scopes
            .get_mut(&SyncScopeKey::Provider(codex_key()))
            .unwrap()
            .files
            .get_mut(&oversized_path)
            .unwrap()
            .size = MAX_CONVERSATION_FILE_BYTES + 1;
        let mut inner = engine.byte_source(&oversized_snapshot, None, None);
        let mut bytes = CountingBytes {
            inner: &mut inner,
            gets: Vec::new(),
            prefetches: Vec::new(),
        };
        // Provider failures are isolated (D8): the over-limit transcript
        // fails the codex provider row closed instead of erasing the whole
        // review.
        let review = thread_review(
            &repo,
            Some(&oversized_snapshot),
            &mut bytes,
            &views,
            None,
            &captured,
            &BTreeMap::new(),
            &resolved,
            Some(&snapshot.storage_id),
            now,
            now.saturating_add(PLAN_LIFETIME_SECS),
            ReviewPurpose::PullPlan,
        )
        .unwrap();
        let error = review
            .provider_errors
            .get(&codex_key())
            .expect("over-limit transcript should fail the codex provider closed");
        assert!(error.contains("exceeds its read limit"), "{error}");
        assert!(
            review.pull_actions.is_empty(),
            "a failed provider must not contribute pull actions"
        );
        assert_eq!(bytes.prefetches.len(), 1);
        assert!(
            !bytes.prefetches[0].contains(&oversized_path),
            "an over-limit transcript must not enter the prefetch batch"
        );
    }

    /// The apply-time re-proof is per item: a conversation that changes
    /// while the review dialog is open fails closed by itself and keeps its
    /// stored state, while the other reviewed conversations publish. Pinned
    /// by docs/DESIGN_PUSH_PULL_FLOW.md (Push step 5).
    #[test]
    fn push_publishes_surviving_items_when_one_changes_after_review() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("review-gap-store").unwrap();
        add_local_storage(&repo, &storage_id, &temp.path().join("store"));
        let project = register(&repo);
        let project_root = temp.path().join("project");
        let codex_home = temp.path().join("codex-home");
        bind_and_link_test_replica(&repo, &project, &storage_id, &project_root, &codex_home);
        let stable_id = "019f7601-0000-7000-8000-0000000000aa";
        let drifting_id = "019f7601-0000-7000-8000-0000000000bb";
        write_codex_session(&codex_home, stable_id, &project_root);
        write_codex_session(&codex_home, drifting_id, &project_root);

        let review =
            prepare_push_review_with_repository(&repo, &project.local_project_id, &storage_id)
                .unwrap();
        let plan_id = review.conversation_push_plan_id.clone().unwrap();
        let approved = review
            .conversation_action_ids
            .values()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(approved.len(), 2);

        // The user keeps chatting in one thread while the review dialog is
        // open: the rollout stays valid but its bytes and chain move.
        std::fs::write(
            codex_home
                .join("sessions/2026/07/22")
                .join(format!("rollout-{}.jsonl", drifting_id)),
            command_test_transcript(
                drifting_id,
                &project_root,
                &[
                    ("message-test", "user", "hello"),
                    ("message-drift", "assistant", "kept chatting"),
                ],
            ),
        )
        .unwrap();

        let pushed = push_bundle_reviewed_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
            recommended_review_recipe(&review, &storage_id),
            empty_project_content_review(),
            Some(plan_id),
            approved,
            &mut |_| {},
        )
        .unwrap();
        assert!(pushed.success, "{}", pushed.message);
        assert!(
            pushed.message.contains("changed during review"),
            "{}",
            pushed.message
        );
        let failed = pushed
            .results
            .iter()
            .filter(|result| result.state == "failed")
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1, "exactly the drifted item fails closed");
        assert!(
            failed[0]
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("is stale at"),
            "{:?}",
            failed[0].message
        );

        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let published = engine.inspect(&project.bundle_id).unwrap();
        assert_eq!(
            published.manifest.all_conversations().count(),
            1,
            "the drifted thread kept its (absent) stored state"
        );
        let (_, _, stored) = published.manifest.all_conversations().next().unwrap();
        assert_eq!(stored.source_thread_id.as_deref(), Some(stable_id));
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

    /// A bound thread that is neither capturable locally nor present in
    /// storage — for example a fork Codex preserved into a different
    /// workspace — holds its own row instead of failing the whole refresh.
    #[test]
    fn refresh_holds_unresolvable_bound_threads_instead_of_failing() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage_id = StorageId::parse("phantom-binding-store").unwrap();
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
        let stored_provider_id = "019f7500-0000-7000-8000-0000000000a0";
        let stored = ThreadSpec::new(
            "stored-thread",
            stored_provider_id,
            command_test_transcript(
                stored_provider_id,
                &project_root,
                &[("message-stored", "user", "stored")],
            ),
            origin.clone(),
            10,
        );
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        engine
            .publish(publish_request(
                project.bundle_id.clone(),
                &[stored],
                origin,
                10,
                None,
                PublishExpectation::Absent,
            ))
            .unwrap();

        // The phantom: bound on this replica, no rollout under the Codex
        // home, never pushed.
        let profile_id = binding.profile_ids[&codex_key()].clone();
        repo.mutate_conversation_bindings(|document| {
            document.bindings.push(LocalConversationBinding {
                host_id: binding.host_id.clone(),
                replica_id: binding.replica_id.clone(),
                local_project_id: binding.local_project_id.clone(),
                provider_profile_id: profile_id,
                thread_id: ThreadId::parse("thread-phantom").unwrap(),
                forked_from: None,
                preserved: None,
                root_tail_layout: Some(RootTailLayout {
                    codec: crate::project_sync::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                    partition_kind: crate::project_sync::domain::TailPartitionKind::WholeThread,
                    start_record_count: 0,
                }),
                provider_thread_id: Some("019f7500-0000-7000-8000-0000000000a1".to_string()),
                materialization_state: ConversationMaterializationState::Resumable,
                revision: 0,
                created_at: 11,
                updated_at: 11,
            });
            Ok(())
        })
        .unwrap();

        let comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &project.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(comparison.entries.len(), 2);
        let phantom = comparison
            .entries
            .iter()
            .find(|entry| {
                entry.cloud_thread_id.as_ref().map(ThreadId::as_str) == Some("thread-phantom")
            })
            .expect("phantom binding surfaces as its own row");
        assert_eq!(
            phantom.push_decision.blocker,
            Some(ConversationBlocker::UnknownEvidence)
        );
        assert!(!phantom.push_decision.selectable);
        assert!(!phantom.pull_decision.selectable);
        let stored_entry = comparison
            .entries
            .iter()
            .find(|entry| {
                entry.cloud_thread_id.as_ref().map(ThreadId::as_str) == Some("stored-thread")
            })
            .expect("stored thread still classifies normally");
        assert_eq!(
            stored_entry.pull_decision.action,
            ConversationReviewAction::CreateLocal
        );
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
        let second_session_id = "019f0fb9-b140-7af3-8b7c-5d75c974b231";
        let session_relative = format!(
            "sessions/2026/07/18/rollout-2026-07-18T00-00-00-{}.jsonl",
            session_id
        );
        let second_session_relative = format!(
            "sessions/2026/07/18/rollout-2026-07-18T00-00-01-{}.jsonl",
            second_session_id
        );
        let transcript = command_test_transcript(
            session_id,
            &source_root,
            &[("message-source", "user", "change the color")],
        );
        let second_transcript = command_test_transcript(
            second_session_id,
            &source_root,
            &[("message-source-2", "user", "change the camera")],
        );
        let source_session = source_codex.join(&session_relative);
        let second_source_session = source_codex.join(&second_session_relative);
        std::fs::create_dir_all(source_session.parent().unwrap()).unwrap();
        std::fs::write(&source_session, &transcript).unwrap();
        std::fs::write(&second_source_session, &second_transcript).unwrap();
        std::fs::write(
            source_codex.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"Change glb color to yellow\"}}\n\
                 {{\"id\":\"{}\",\"thread_name\":\"Change camera angle\"}}\n",
                session_id, second_session_id
            ),
        )
        .unwrap();
        let source_profile = add_profile(&repo, &codex_key(), &source_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: source.local_project_id.clone(),
                project_root: source_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), source_profile)]),
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
        let target_profile = add_profile(&repo, &codex_key(), &target_codex);
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: target.local_project_id.clone(),
                project_root: target_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), target_profile)]),
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
            conversation_resource_id(&codex_key(), &cloud_thread_id).unwrap()
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
        assert_eq!(restore.conversation_actions.len(), 2);
        let conversation_action = restore
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == cloud_thread_id)
            .cloned()
            .unwrap();
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
            restore
                .conversation_actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect(),
        )
        .unwrap();
        assert!(
            applied.success,
            "{} — failed: {:?}",
            applied.message, applied.failed_actions
        );
        assert_eq!(applied.applied_action_ids.len(), 2);
        assert!(applied
            .applied_action_ids
            .contains(&conversation_action.action_id));

        let restored_session_path = conversation_action
            .pins
            .target
            .provider_state
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
        assert_eq!(index_rows.len(), 2);
        let restored_index_row = index_rows
            .iter()
            .find(|row| row["id"] == rows[0]["payload"]["id"])
            .unwrap();
        assert_eq!(
            restored_index_row["thread_name"],
            "Change glb color to yellow"
        );
        assert_eq!(
            restored_index_row["id"], rows[0]["payload"]["id"],
            "the index row must name the rewritten provider thread"
        );
        assert!(
            restored_index_row.get("cwd").is_none(),
            "index rows use the provider schema, not Mallard-invented fields"
        );
        chrono::DateTime::parse_from_rfc3339(restored_index_row["updated_at"].as_str().unwrap())
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
        let (target_base, target_scopes) =
            reviewed_link_state(&repo, &target.local_project_id, &storage_id);
        let codex_base = &target_scopes[&SyncScopeKey::Provider(codex_key())];
        assert!(codex_base.last_pull_at.is_some());
        assert_eq!(codex_base.last_push_at, None);
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        assert_eq!(
            target_base.manifest_size,
            engine.inspect(&bundle_id).unwrap().head.manifest_size
        );
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

        // Pull must install a complete resume cursor so one parentless Codex
        // message can advance incrementally and still match Push's full
        // reconstruction.
        let config = repo.load_config().unwrap();
        let target_link = config
            .links
            .iter()
            .find(|link| {
                link.local_project_id == target.local_project_id && link.storage_id == storage_id
            })
            .unwrap();
        let cloud_resource_id = conversation_resource_id(&codex_key(), &cloud_thread_id).unwrap();
        assert!(!target_link
            .recipe
            .as_ref()
            .unwrap()
            .entries
            .contains_key(&cloud_resource_id));
        let checkpoint = config
            .links
            .iter()
            .find(|link| {
                link.local_project_id == target.local_project_id && link.storage_id == storage_id
            })
            .and_then(|link| {
                link.conversation_checkpoints
                    .get(&SyncScopeKey::Provider(codex_key()))
            })
            .and_then(|threads| threads.get(&cloud_thread_id))
            .unwrap();
        let PhysicalMappingState::Ready { root_mapping, .. } = &checkpoint.physical else {
            panic!("pulled conversation has no ready mapping");
        };
        assert!(root_mapping.last_canonical_message_id.is_some());

        use std::io::Write as _;
        let mut restored_session = std::fs::OpenOptions::new()
            .append(true)
            .open(&restored_session_path)
            .unwrap();
        serde_json::to_writer(
            &mut restored_session,
            &serde_json::json!({
                "ordinal": root_mapping.consumed_record_ordinal,
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": "message-after-pull",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "continue"}]
                }
            }),
        )
        .unwrap();
        restored_session.write_all(b"\n").unwrap();
        restored_session.sync_all().unwrap();

        let pushed =
            push_bundle_with_repository(&repo, &target.local_project_id, &storage_id).unwrap();
        assert_eq!(pushed.revision, Some(2));

        // The source replica is now one revision behind. Codex cloud updates
        // are always copy-on-write because a native fork can begin depending
        // on the old rollout concurrently without changing its digest.
        let source_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&source.local_project_id)
            .cloned()
            .unwrap();
        let source_pull =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &source_binding)
                .unwrap();
        let update = source_pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == cloud_thread_id)
            .unwrap();
        assert_eq!(update.action, ConversationPlanActionType::MaterializeCloud);
        let relocated_source_id = update
            .proposed_provider_thread_id
            .as_deref()
            .expect("Codex cloud update mints a fresh provider id");
        assert_ne!(relocated_source_id, session_id);
        let transcript_targets = update
            .pins
            .target
            .provider_state
            .iter()
            .filter(|target| {
                target.target_path.ends_with(".jsonl")
                    && !target.target_path.ends_with("session_index.jsonl")
            })
            .collect::<Vec<_>>();
        assert_eq!(transcript_targets.len(), 1);
        let relocated_source_session = PathBuf::from(&transcript_targets[0].target_path);
        assert_ne!(
            relocated_source_session,
            std::fs::canonicalize(&source_session).unwrap()
        );
        assert_eq!(transcript_targets[0].expectation, TargetExpectation::Absent);
        let source_bytes_before_update = std::fs::read(&source_session).unwrap();
        let source_applied = apply_bundle_restore_with_repository(
            &repo,
            &source_pull.plan_id,
            vec![update.action_id.clone()],
        )
        .unwrap();
        assert!(
            source_applied.success,
            "{:?}",
            source_applied.failed_actions
        );
        let source_row = repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .find(|binding| {
                binding.local_project_id == source.local_project_id
                    && binding.thread_id == cloud_thread_id
            })
            .unwrap();
        assert_eq!(
            source_row.provider_thread_id.as_deref(),
            Some(relocated_source_id)
        );
        assert_eq!(
            std::fs::read(&source_session).unwrap(),
            source_bytes_before_update,
            "copy-on-write changed the displaced Codex rollout"
        );
        assert!(relocated_source_session.is_file());
        let source_index =
            std::fs::read_to_string(source_codex.join("session_index.jsonl")).unwrap();
        let source_index_rows = source_index
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            source_index_rows
                .iter()
                .filter(|row| row["id"] == session_id)
                .count(),
            1
        );
        assert_eq!(
            source_index_rows
                .iter()
                .filter(|row| row["id"] == relocated_source_id)
                .count(),
            1
        );
        let source_comparison = get_project_thread_sync_comparison_with_repository(
            &repo,
            &source.local_project_id,
            &storage_id,
        )
        .unwrap();
        assert_eq!(
            source_comparison
                .entries
                .iter()
                .find(|entry| entry.cloud_thread_id.as_ref() == Some(&cloud_thread_id))
                .unwrap()
                .state,
            ThreadSyncState::Synced
        );

        // Both replicas now branch from revision 2. The target publishes its
        // branch; the source Pull must retain its local rollout under a new
        // logical thread and mark only that moved binding as preserved.
        let append_user_message = |path: &Path, id: &str, text: &str| {
            let ordinal = std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .next_back()
                .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .and_then(|record| record.get("ordinal").and_then(serde_json::Value::as_u64))
                .unwrap()
                + 1;
            let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            serde_json::to_writer(
                &mut file,
                &serde_json::json!({
                    "ordinal": ordinal,
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "id": id,
                        "role": "user",
                        "content": [{"type": "input_text", "text": text}]
                    }
                }),
            )
            .unwrap();
            file.write_all(b"\n").unwrap();
            file.sync_all().unwrap();
        };
        append_user_message(
            &relocated_source_session,
            "message-source-branch",
            "source branch",
        );
        append_user_message(
            Path::new(&restored_session_path),
            "message-target-branch",
            "target branch",
        );
        let pushed =
            push_bundle_with_repository(&repo, &target.local_project_id, &storage_id).unwrap();
        assert_eq!(pushed.revision, Some(3));

        // The keep-both marker names this device; provenance and the marker
        // title are minted into the plan itself.
        let identity = repo.load_or_create_host_identity().unwrap();
        repo.update_host_display_name(
            &identity.host_id,
            identity.revision,
            Some("Machine A".to_string()),
        )
        .unwrap();
        let divergent_pull =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &source_binding)
                .unwrap();
        let divergent_action = divergent_pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == cloud_thread_id)
            .unwrap();
        assert_eq!(
            divergent_action.action,
            ConversationPlanActionType::KeepBoth
        );
        let keep_both = divergent_action.keep_both.as_ref().unwrap();
        let preserved_thread_id = keep_both.preserved_thread_id.clone();
        let remote_provider_thread_id = keep_both.remote_provider_thread_id.clone();
        let marker_title = "Change glb color to yellow (Preserved 1 · Machine A)";
        assert_eq!(
            keep_both.preserved,
            PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 1,
            }
        );
        assert_eq!(keep_both.preserved_display_title, marker_title);
        let divergent_result = apply_bundle_restore_with_repository(
            &repo,
            &divergent_pull.plan_id,
            vec![divergent_action.action_id.clone()],
        )
        .unwrap();
        assert!(
            divergent_result.success,
            "{:?}",
            divergent_result.failed_actions
        );
        assert_eq!(divergent_result.preserved_threads.len(), 1);
        assert_eq!(
            divergent_result.preserved_threads[0].preserved_thread_id,
            preserved_thread_id
        );
        assert_eq!(
            divergent_result.preserved_threads[0].preserved_display_title,
            marker_title
        );

        let source_bindings = repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .filter(|binding| binding.local_project_id == source.local_project_id)
            .collect::<Vec<_>>();
        let preserved = source_bindings
            .iter()
            .find(|binding| binding.thread_id == preserved_thread_id)
            .unwrap();
        let pulled = source_bindings
            .iter()
            .find(|binding| binding.thread_id == cloud_thread_id)
            .unwrap();
        assert_eq!(
            preserved.preserved,
            Some(PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 1,
            })
        );
        assert!(pulled.preserved.is_none());
        assert_eq!(
            preserved.provider_thread_id.as_deref(),
            Some(relocated_source_id)
        );
        assert_eq!(
            pulled.provider_thread_id.as_deref(),
            Some(remote_provider_thread_id.as_str())
        );
        assert_ne!(preserved.thread_id, pulled.thread_id);

        // One transaction renamed the preserved copy's existing index row to
        // the marker title and appended the destination row.
        let divergent_index =
            std::fs::read_to_string(source_codex.join("session_index.jsonl")).unwrap();
        let divergent_rows = divergent_index
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let marker_rows = divergent_rows
            .iter()
            .filter(|row| row["id"] == relocated_source_id)
            .collect::<Vec<_>>();
        assert_eq!(marker_rows.len(), 1);
        assert_eq!(marker_rows[0]["thread_name"], marker_title);
        let destination_rows = divergent_rows
            .iter()
            .filter(|row| row["id"] == remote_provider_thread_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(destination_rows.len(), 1);
        assert_eq!(
            destination_rows[0]["thread_name"],
            "Change glb color to yellow"
        );

        // Session Detail leads with the index title, so the marker shows.
        let preserved_resource_id =
            conversation_resource_id(&codex_key(), &preserved_thread_id).unwrap();
        let detail_views = load_project_session_views(
            &repo,
            &source.local_project_id,
            preserved_resource_id.as_str(),
        )
        .unwrap();
        let detail = session_detail::get_project_session_detail(
            &repo,
            provider_registry(),
            &source.local_project_id,
            preserved_resource_id.as_str(),
            &detail_views,
        )
        .unwrap();
        assert_eq!(detail.title, marker_title);

        // The preserved copy's first Push is CreateCloud: the manifest
        // record carries the provenance, the marker title, and this
        // device's shared origin label.
        let pushed_preserved =
            push_bundle_with_repository(&repo, &source.local_project_id, &storage_id).unwrap();
        assert_eq!(pushed_preserved.revision, Some(4));
        let (_, engine) = storage_engine(&repo, &storage_id).unwrap();
        let published = engine.inspect(&bundle_id).unwrap();
        let (_, preserved_record) = published
            .manifest
            .find_conversation(&preserved_thread_id)
            .unwrap();
        assert_eq!(
            preserved_record.preserved,
            Some(PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 1,
            })
        );
        assert_eq!(
            preserved_record.display_title.as_deref(),
            Some(marker_title)
        );
        assert_eq!(
            preserved_record
                .last_updated_by_origin
                .as_ref()
                .and_then(|origin| origin.shared_label.as_deref()),
            Some("Machine A")
        );

        // Pulling the preserved thread elsewhere restores the provenance in
        // the created binding and lands the marker title in that index.
        let target_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&target.local_project_id)
            .cloned()
            .unwrap();
        let target_pull =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &target_binding)
                .unwrap();
        let preserved_action = target_pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == preserved_thread_id)
            .unwrap();
        assert_eq!(
            preserved_action.action,
            ConversationPlanActionType::CreateLocal
        );
        let target_applied = apply_bundle_restore_with_repository(
            &repo,
            &target_pull.plan_id,
            vec![preserved_action.action_id.clone()],
        )
        .unwrap();
        assert!(
            target_applied.success,
            "{:?}",
            target_applied.failed_actions
        );
        let target_preserved_row = repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .find(|binding| {
                binding.local_project_id == target.local_project_id
                    && binding.thread_id == preserved_thread_id
            })
            .unwrap();
        assert_eq!(
            target_preserved_row.preserved,
            Some(PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 1,
            })
        );
        let target_index =
            std::fs::read_to_string(target_codex.join("session_index.jsonl")).unwrap();
        let target_marker_rows = target_index
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|row| row["thread_name"] == marker_title)
            .count();
        assert_eq!(target_marker_rows, 1);

        // A second divergence of the same origin mints past the max visible
        // ordinal — max+1 over cloud and binding provenance, not a count of
        // preserved copies — and composes an un-nested marker.
        fn find_rollout(root: &Path, provider_id: &str) -> Option<PathBuf> {
            let mut stack = vec![root.to_path_buf()];
            while let Some(directory) = stack.pop() {
                for entry in std::fs::read_dir(&directory).ok()?.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.contains(provider_id))
                    {
                        return Some(path);
                    }
                }
            }
            None
        }
        append_user_message(
            Path::new(&restored_session_path),
            "message-target-branch-2",
            "target branch again",
        );
        let pushed_again =
            push_bundle_with_repository(&repo, &target.local_project_id, &storage_id).unwrap();
        assert_eq!(pushed_again.revision, Some(5));
        let source_pulled_rollout = find_rollout(
            &source_codex.join("sessions"),
            remote_provider_thread_id.as_str(),
        )
        .expect("keep-both materialized the pulled branch on the source");
        append_user_message(
            &source_pulled_rollout,
            "message-source-branch-2",
            "source branch again",
        );
        repo.mutate_conversation_bindings(|document| {
            let row = document
                .bindings
                .iter_mut()
                .find(|row| {
                    row.local_project_id == source.local_project_id
                        && row.thread_id == preserved_thread_id
                })
                .unwrap();
            row.preserved = Some(PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 5,
            });
            row.revision = row.revision.saturating_add(1);
            Ok(())
        })
        .unwrap();
        let source_binding = repo
            .load_bindings()
            .unwrap()
            .active_for(&source.local_project_id)
            .cloned()
            .unwrap();
        let second_divergent =
            plan_bundle_restore_with_repository(&repo, &storage_id, &bundle_id, &source_binding)
                .unwrap();
        let second_action = second_divergent
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == cloud_thread_id)
            .unwrap();
        assert_eq!(second_action.action, ConversationPlanActionType::KeepBoth);
        let second_keep_both = second_action.keep_both.as_ref().unwrap();
        assert_eq!(
            second_keep_both.preserved,
            PreservedProvenance {
                preserved_from: cloud_thread_id.clone(),
                ordinal: 6,
            }
        );
        assert_eq!(
            second_keep_both.preserved_display_title,
            "Change glb color to yellow (Preserved 6 · Machine A)"
        );
    }

    fn receipt(action_type: RestoreActionType, status: ActionStatus, suffix: &str) -> ApplyReceipt {
        ApplyReceipt {
            action_id: ActionId::parse(format!("action-{:0>24}", suffix)).unwrap(),
            resource_id: ResourceId::parse("codex:session-index").unwrap(),
            action_type,
            logical_path: None,
            source_sha256: None,
            target_path: None,
            target_sha256_after: None,
            status,
            applied_at: 1,
            error: None,
            conversation: None,
        }
    }

    #[test]
    fn reviewed_files_complete_gates_the_baseline_on_skipped_content() {
        let none = BTreeSet::new();
        // A review of nothing is complete: the acknowledge-only Pull.
        assert!(reviewed_files_complete(&[], &none));
        // Declined project-content decisions still settle the review.
        assert!(reviewed_files_complete(
            &[
                receipt(
                    RestoreActionType::WriteProjectFile,
                    ActionStatus::Skipped,
                    "a1"
                ),
                receipt(
                    RestoreActionType::EnsureProjectDirectory,
                    ActionStatus::Skipped,
                    "a2"
                ),
            ],
            &none,
        ));
        // A skipped generic write leaves reviewed content behind: no advance.
        assert!(!reviewed_files_complete(
            &[receipt(
                RestoreActionType::WriteFile,
                ActionStatus::Skipped,
                "a3"
            )],
            &none,
        ));
        assert!(!reviewed_files_complete(
            &[receipt(
                RestoreActionType::MergeFile,
                ActionStatus::Skipped,
                "a4"
            )],
            &none,
        ));
        assert!(reviewed_files_complete(
            &[
                receipt(RestoreActionType::WriteFile, ActionStatus::Applied, "a5"),
                receipt(
                    RestoreActionType::MaterializeConversation,
                    ActionStatus::Applied,
                    "a6"
                ),
            ],
            &none,
        ));
        // A declined Claude materialization is a settled decision — parity
        // with unapproved typed Codex actions, which produce no receipt.
        assert!(reviewed_files_complete(
            &[receipt(
                RestoreActionType::MaterializeConversation,
                ActionStatus::Skipped,
                "a8"
            )],
            &none,
        ));
        // A keep-local Manual skip is itself the reviewed decision.
        let kept = receipt(RestoreActionType::Manual, ActionStatus::Skipped, "a7");
        let kept_ids = BTreeSet::from([kept.action_id.clone()]);
        assert!(reviewed_files_complete(
            std::slice::from_ref(&kept),
            &kept_ids
        ));
        assert!(!reviewed_files_complete(&[kept], &BTreeSet::new()));
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
            &codex_key(),
            &container.to_string_lossy(),
        )
        .unwrap();
        assert!(probe.detected_child);
        assert_eq!(Path::new(&probe.resolved_path), codex_home);

        let first = create_provider_profile_with_repository(
            &repo,
            &codex_key(),
            "myconf2",
            &container.to_string_lossy(),
        )
        .unwrap();
        let duplicate = create_provider_profile_with_repository(
            &repo,
            &codex_key(),
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
            probe_provider_profile_with_repository(&repo, &codex_key(), &link.to_string_lossy())
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
        let codex_profile = add_profile(&repo, &codex_key(), &temp.path().join("codex-home"));

        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(
                    ProviderKey::parse("claude").expect("static provider key"),
                    codex_profile,
                )]),
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
        let profile_id = add_profile(&repo, &codex_key(), &temp.path().join("codex-home"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id.clone())]),
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
        let profile_id = add_profile(&repo, &codex_key(), &temp.path().join("codex-home"));
        let profile_ids = BTreeMap::from([(codex_key(), profile_id)]);
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
        let first_profile = add_profile(&repo, &codex_key(), &temp.path().join("codex-one"));
        let second_profile = add_profile(&repo, &codex_key(), &temp.path().join("codex-two"));
        let first = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), first_profile.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), second_profile)]),
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
        assert_eq!(current.profile_ids.get(&codex_key()), Some(&first_profile));
    }

    #[test]
    fn binding_rejects_local_store_overlap() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let home = temp.path().join("home");
        let repo = LocalStateRepository::from_home_dir(&home).unwrap();
        let project = register(&repo);
        let profile_id = add_profile(&repo, &codex_key(), &temp.path().join("codex-home"));
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
            supports_conditional_writes: None,
        });
        repo.save_config(config).unwrap();
        let error = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id)]),
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
        let profile_id = add_profile(&repo, &codex_key(), &codex_home);
        let binding = save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), profile_id.clone())]),
                expected_revision: None,
            },
        )
        .unwrap();

        assert_eq!(binding.profile_ids.get(&codex_key()), Some(&profile_id));
        // The binding persists profile IDs only; the runtime home resolves
        // through the profile catalog to the profile's recorded path.
        let persisted = repo.load_bindings().unwrap();
        let profile = persisted
            .profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
            .unwrap();
        let resolved = resolve_project_binding(&repo, &binding).unwrap();
        assert_eq!(
            resolved.homes.get(&codex_key()),
            Some(&profile.canonical_path)
        );
        assert_eq!(persisted.bindings[0].profile_ids, binding.profile_ids);
    }

    #[test]
    fn one_checkout_hosts_a_second_project_through_a_different_codex_config() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("healthGame");
        std::fs::create_dir_all(&project_root).unwrap();
        let repo = LocalStateRepository::from_home_dir(temp.path().join("home")).unwrap();
        let first = register(&repo);
        let second = register(&repo);
        let main_profile = add_profile(&repo, &codex_key(), &temp.path().join(".codex"));
        let alt_profile = add_profile(&repo, &codex_key(), &temp.path().join("conf2/.codex"));
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: first.local_project_id.clone(),
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([(codex_key(), main_profile.clone())]),
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
                profile_ids: BTreeMap::from([(codex_key(), main_profile)]),
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
                profile_ids: BTreeMap::from([(codex_key(), alt_profile)]),
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
        let codex_profile = add_profile(&repo, &codex_key(), &codex_home);
        let claude_profile = add_profile(
            &repo,
            &ProviderKey::parse("claude").expect("static provider key"),
            &claude_home,
        );
        save_project_binding_with_repository(
            &repo,
            SaveProjectBindingRequest {
                local_project_id: project.local_project_id,
                project_root: project_root.to_string_lossy().into_owned(),
                profile_ids: BTreeMap::from([
                    (codex_key(), codex_profile),
                    (
                        ProviderKey::parse("claude").expect("static provider key"),
                        claude_profile,
                    ),
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
            codex_key(),
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
        assert!(!first
            .draft
            .profiles
            .contains_key(&ProviderKey::parse("claude").expect("static provider key")));
        assert!(second.resumed);
        assert_eq!(first.draft.draft_id, second.draft.draft_id);
        let (drafts, warnings) = fixture.repo.list_setup_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn setup_inspection_only_reports_configuration_sections() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let inspection =
            inspect_setup_draft_with_repository(&fixture.repo, &draft.draft_id).unwrap();
        let section_names = inspection
            .sections
            .iter()
            .map(|section| section.section.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            section_names,
            vec!["project", "profiles", "storage", "repository"]
        );
        assert!(inspection.can_finalize);
        assert!(inspection
            .sections
            .iter()
            .all(|section| section.state == "ready"));
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
        assert_eq!(detail.project.recipe, BundleRecipe::default());
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
            profile.provider == codex_key()
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
    fn repeated_session_metadata_after_a_checkpoint_is_revalidated_and_pushed() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let project =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let project_id = project.project.local_project_id;
        let storage_id = project.links[0].storage_id.clone();
        let provider_thread_id = "019f7500-0000-7000-8000-00000000a901";
        let rollout = fixture
            .codex_home
            .join("sessions/2026/08/08")
            .join(format!("rollout-{provider_thread_id}.jsonl"));
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();

        let initial = command_test_transcript(
            provider_thread_id,
            &fixture.project_dir,
            &[("message-1", "user", "first")],
        );
        std::fs::write(&rollout, &initial).unwrap();
        let first = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));

        let advanced = command_test_transcript(
            provider_thread_id,
            &fixture.project_dir,
            &[
                ("message-1", "user", "first"),
                ("message-2", "assistant", "second"),
            ],
        );
        assert!(advanced.starts_with(&initial));
        let metadata_end = initial.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        let next_ordinal = initial.iter().filter(|byte| **byte == b'\n').count() as u64;
        let mut resumed = initial.clone();
        let mut repeated_meta: serde_json::Value =
            serde_json::from_slice(&initial[..metadata_end - 1]).unwrap();
        repeated_meta["ordinal"] = serde_json::json!(next_ordinal);
        serde_json::to_writer(&mut resumed, &repeated_meta).unwrap();
        resumed.push(b'\n');
        let mut appended_message: serde_json::Value =
            serde_json::from_slice(&advanced[initial.len()..advanced.len() - 1]).unwrap();
        appended_message["ordinal"] = serde_json::json!(next_ordinal + 1);
        serde_json::to_writer(&mut resumed, &appended_message).unwrap();
        resumed.push(b'\n');
        std::fs::write(&rollout, resumed).unwrap();

        let second = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(second.revision, Some(2));
    }

    #[test]
    fn reregistering_removed_project_reclaims_conversation_identity_and_pushes() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let first =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let first_binding = first.binding.clone().expect("initial binding");
        let project_id = first.project.local_project_id.clone();
        let bundle_id = first.project.bundle_id.clone();
        let storage_id = first.links[0].storage_id.clone();
        let profile_id = first_binding.profile_ids[&codex_key()].clone();

        let provider_thread_id = "019f7500-0000-7000-8000-00000000aa01";
        let rollout = fixture
            .codex_home
            .join("sessions/2026/08/08")
            .join(format!("rollout-{provider_thread_id}.jsonl"));
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(
            &rollout,
            command_test_transcript(
                provider_thread_id,
                &fixture.project_dir,
                &[("message-1", "user", "first")],
            ),
        )
        .unwrap();

        let initial_push =
            push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(initial_push.revision, Some(1));
        let initial_conversations = fixture.repo.load_conversation_bindings().unwrap();
        assert_eq!(initial_conversations.bindings.len(), 1);
        assert_eq!(
            initial_conversations.bindings[0].replica_id,
            first_binding.replica_id
        );

        assert!(remove_local_project_with_repository(&fixture.repo, &project_id).unwrap());
        let detached = fixture
            .repo
            .load_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .find(|binding| binding.replica_id == first_binding.replica_id)
            .expect("detached binding retained");
        assert_eq!(detached.state, BindingState::Detached);

        let created = create_setup_draft_with_repository(
            &fixture.repo,
            &fixture.project_dir.to_string_lossy(),
        )
        .unwrap();
        let mut reregister = created.draft;
        reregister.profiles = BTreeMap::from([(
            codex_key(),
            DraftProfileSelection::Existing {
                profile_id: profile_id.clone(),
            },
        )]);
        reregister.storage = Some(DraftStorageSelection::Existing {
            storage_id: storage_id.clone(),
        });
        reregister.repository = DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: first.project.display_name.clone(),
            repository_fingerprint: first.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        };
        let reregister = update_setup_draft_with_repository(&fixture.repo, reregister).unwrap();
        let second = finalize_project_setup_with_repository(
            &fixture.repo,
            &reregister.draft_id,
            reregister.revision,
        )
        .unwrap();
        let second_binding = second.binding.expect("reactivated binding");
        assert_eq!(second.project.local_project_id, project_id);
        second.project.recipe.validate_machine_local().unwrap();
        assert!(second
            .project
            .recipe
            .entries
            .keys()
            .all(|resource_id| parse_conversation_resource_id(resource_id.as_str()).is_err()));
        assert_eq!(second_binding.replica_id, first_binding.replica_id);
        assert_eq!(second_binding.revision, detached.revision + 1);

        let preserved = fixture.repo.load_conversation_bindings().unwrap();
        assert_eq!(preserved.bindings.len(), 1);
        assert_eq!(preserved.bindings[0].local_project_id, project_id);
        assert_eq!(preserved.bindings[0].replica_id, second_binding.replica_id);

        let (_, engine) = storage_engine(&fixture.repo, &storage_id).unwrap();
        let reviewed = engine.inspect(&bundle_id).unwrap();
        record_reviewed_scope_bases(
            &fixture.repo,
            &second.project,
            &second_binding,
            &storage_id,
            &reviewed,
            now_secs(),
        );

        std::fs::write(
            &rollout,
            command_test_transcript(
                provider_thread_id,
                &fixture.project_dir,
                &[
                    ("message-1", "user", "first"),
                    ("message-2", "assistant", "second"),
                ],
            ),
        )
        .unwrap();
        let next_push =
            push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(next_push.revision, Some(2));
        let committed = fixture.repo.load_conversation_bindings().unwrap();
        assert_eq!(committed.bindings.len(), 1);
        assert!(fixture
            .repo
            .load_conversation_sync_journal()
            .unwrap()
            .operations
            .is_empty());
    }

    #[test]
    fn paginated_thread_with_an_empty_leaf_pushes_its_resolved_history() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let project =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let project_id = project.project.local_project_id.clone();
        let bundle_id = project.project.bundle_id.clone();
        let storage_id = project.links[0].storage_id.clone();

        let base_id = "019f7500-0000-7000-8000-00000000ba01";
        let leaf_id = "019f7500-0000-7000-8000-00000000ba02";
        let sessions = fixture.codex_home.join("sessions/2026/08/09");
        std::fs::create_dir_all(&sessions).unwrap();
        let base_bytes = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("base-1", "user", "inherited question"),
                ("base-2", "assistant", "inherited answer"),
            ],
        );
        std::fs::write(
            sessions.join(format!("rollout-{base_id}.jsonl")),
            &base_bytes,
        )
        .unwrap();
        let base_record_count = base_bytes.iter().filter(|byte| **byte == b'\n').count() as u64;

        // This is a valid Codex fork with no messages of its own. The
        // physical leaf codec is intentionally incomplete; its logical root
        // becomes complete only after resolving history_base.
        let leaf_meta = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-09T00:00:00Z",
            "ordinal": base_record_count,
            "type": "session_meta",
            "payload": {
                "id": leaf_id,
                "cwd": fixture.project_dir.to_string_lossy(),
                "history_mode": "paginated",
                "history_base": {
                    "thread_id": base_id,
                    "end_ordinal_exclusive": base_record_count,
                    "end_byte_offset": base_bytes.len() as u64,
                },
            },
        }))
        .unwrap();
        let settings = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-09T00:00:01Z",
            "ordinal": base_record_count + 1,
            "type": "event_msg",
            "payload": { "type": "thread_settings_applied" },
        }))
        .unwrap();
        let leaf_bytes = format!("{leaf_meta}\n{settings}\n").into_bytes();
        assert!(!analyze_codex_transcript(&leaf_bytes).is_resumable());
        std::fs::write(
            sessions.join(format!("rollout-{leaf_id}.jsonl")),
            &leaf_bytes,
        )
        .unwrap();

        let pushed = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(pushed.revision, Some(1));

        let (_, engine) = storage_engine(&fixture.repo, &storage_id).unwrap();
        let snapshot = engine.inspect(&bundle_id).unwrap();
        let leaf = snapshot
            .manifest
            .all_conversations()
            .find_map(|(_, _, thread)| {
                (thread.source_thread_id.as_deref() == Some(leaf_id)).then_some(thread)
            })
            .expect("empty paginated leaf was published");
        let tail = leaf.portable_tail.as_ref().expect("portable root tail");
        assert_eq!(tail.partition_kind, TailPartitionKind::FixedPaginatedTail);
        assert_eq!(tail.start_record_count, 2);
        assert_eq!(tail.accepted_end_record_count, tail.start_record_count);
        assert_eq!(leaf.display_title.as_deref(), Some("inherited question"));
    }

    #[test]
    fn cloud_update_relocates_a_referenced_codex_ancestor() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let project =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let project_id = project.project.local_project_id.clone();
        let bundle_id = project.project.bundle_id.clone();
        let storage_id = project.links[0].storage_id.clone();

        let base_id = "019f7500-0000-7000-8000-00000000bc01";
        let leaf_id = "019f7500-0000-7000-8000-00000000bc02";
        let sessions = fixture.codex_home.join("sessions/2026/08/10");
        std::fs::create_dir_all(&sessions).unwrap();
        let base_rollout = sessions.join(format!("rollout-{base_id}.jsonl"));
        let leaf_rollout = sessions.join(format!("rollout-{leaf_id}.jsonl"));
        let base_before = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("base-q", "user", "base question"),
                ("base-a", "assistant", "base answer"),
            ],
        );
        std::fs::write(&base_rollout, &base_before).unwrap();
        let base_end_ordinal = base_before.iter().filter(|byte| **byte == b'\n').count() as u64;
        let leaf_meta = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:00Z",
            "ordinal": base_end_ordinal,
            "type": "session_meta",
            "payload": {
                "id": leaf_id,
                "cwd": fixture.project_dir.to_string_lossy(),
                "history_mode": "paginated",
                "history_base": {
                    "thread_id": base_id,
                    "end_ordinal_exclusive": base_end_ordinal,
                    "end_byte_offset": base_before.len() as u64,
                },
            },
        }))
        .unwrap();
        let leaf_message = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:01Z",
            "ordinal": base_end_ordinal + 1,
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "leaf-q",
                "role": "user",
                "content": [{"type": "input_text", "text": "leaf question"}],
            },
        }))
        .unwrap();
        std::fs::write(
            &leaf_rollout,
            format!("{leaf_meta}\n{leaf_message}\n").as_bytes(),
        )
        .unwrap();

        let first = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));
        let config_at_first = fixture.repo.load_config().unwrap();
        let bindings_at_first = fixture.repo.load_conversation_bindings().unwrap();
        let base_thread_id = bindings_at_first
            .bindings
            .iter()
            .find(|binding| binding.provider_thread_id.as_deref() == Some(base_id))
            .unwrap()
            .thread_id
            .clone();

        // Publish a cloud advance, then restore the first revision's local
        // documents and bytes to model a second device that has not pulled.
        let base_after = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("base-q", "user", "base question"),
                ("base-a", "assistant", "base answer"),
                ("base-next", "user", "cloud advance"),
            ],
        );
        assert!(base_after.starts_with(&base_before));
        std::fs::write(&base_rollout, &base_after).unwrap();
        let second = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(second.revision, Some(2));
        std::fs::write(&base_rollout, &base_before).unwrap();
        fixture
            .repo
            .install_config_snapshot(config_at_first)
            .unwrap();
        fixture
            .repo
            .install_conversation_bindings_snapshot(bindings_at_first)
            .unwrap();

        let binding = fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&project_id)
            .cloned()
            .unwrap();
        let pull =
            plan_bundle_restore_with_repository(&fixture.repo, &storage_id, &bundle_id, &binding)
                .unwrap();
        let action = pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == base_thread_id)
            .expect("cloud-advanced base is materializable");
        assert_eq!(action.action, ConversationPlanActionType::MaterializeCloud);
        let fresh_id = action
            .proposed_provider_thread_id
            .as_deref()
            .expect("relocation mints a provider id");
        assert_ne!(fresh_id, base_id);
        let transcript_target = action
            .pins
            .target
            .provider_state
            .iter()
            .find(|target| !target.target_path.ends_with("session_index.jsonl"))
            .unwrap();
        assert_eq!(transcript_target.expectation, TargetExpectation::Absent);
        assert_ne!(Path::new(&transcript_target.target_path), base_rollout);

        let applied = apply_bundle_restore_with_repository(
            &fixture.repo,
            &pull.plan_id,
            vec![action.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{:?}", applied.failed_actions);
        assert_eq!(std::fs::read(&base_rollout).unwrap(), base_before);
        let moved = fixture
            .repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .find(|binding| binding.thread_id == base_thread_id)
            .unwrap();
        assert_eq!(moved.provider_thread_id.as_deref(), Some(fresh_id));
        let retained_thread_id =
            retained_conversation_thread_id(&bundle_id, &codex_key(), &base_thread_id, base_id)
                .unwrap();
        let retained_checkpoint = fixture
            .repo
            .load_config()
            .unwrap()
            .links
            .into_iter()
            .find(|link| link.local_project_id == project_id && link.storage_id == storage_id)
            .and_then(|link| {
                link.conversation_checkpoints
                    .get(&SyncScopeKey::Provider(codex_key()))
                    .and_then(|threads| threads.get(&retained_thread_id))
                    .cloned()
            })
            .expect("copy-on-write persists the retained source witness");
        assert!(retained_checkpoint.accepted.is_none());
        assert!(retained_checkpoint.accepted_local_tail.is_none());
        let PhysicalMappingState::Ready {
            root_mapping,
            raw_range_pin,
        } = &retained_checkpoint.physical
        else {
            panic!("retained source witness must keep its physical mapping");
        };
        assert_eq!(root_mapping.provider_thread_id, base_id);
        assert_eq!(
            Path::new(&root_mapping.root_path),
            std::fs::canonicalize(&base_rollout).unwrap()
        );
        assert_eq!(root_mapping.source_file_size, base_before.len() as u64);
        assert_eq!(root_mapping.consumed_byte_offset, base_before.len() as u64);
        assert_eq!(raw_range_pin.start_byte_offset, 0);
        assert_eq!(raw_range_pin.end_byte_offset, base_before.len() as u64);
        assert_eq!(
            raw_range_pin.range_sha256,
            hex_digest(Sha256::digest(&base_before).as_slice())
        );

        // The retained ancestor still satisfies the child's exact native
        // cutoff, while reconciliation exposes only the moved logical base.
        let resolved_leaf = codex_history_graph::resolve_codex_root(
            &leaf_rollout,
            &codex_rollout_lookup(&fixture.codex_home),
        )
        .unwrap();
        assert!(resolved_leaf
            .segments
            .iter()
            .any(|segment| segment.provider_thread_id == base_id));
        let comparison = get_project_thread_sync_comparison_with_repository(
            &fixture.repo,
            &project_id,
            &storage_id,
        )
        .unwrap();
        assert!(comparison
            .entries
            .iter()
            .all(|entry| entry.provider_thread_id.as_deref() != Some(base_id)));
        let current_binding = fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&project_id)
            .cloned()
            .unwrap();
        let fixed_point = plan_bundle_restore_with_repository(
            &fixture.repo,
            &storage_id,
            &bundle_id,
            &current_binding,
        )
        .unwrap();
        assert!(fixed_point.conversation_actions.is_empty());
        let unchanged_activity = get_project_activity_with_repository(
            &fixture.repo,
            &project_id,
            Some(&storage_id),
            None,
            None,
            Some(90),
            true,
        )
        .unwrap();
        assert!(unchanged_activity
            .sessions
            .iter()
            .all(|session| session.provider_session_id.as_deref() != Some(base_id)));

        // The retained physical dependency is hidden only while its bytes
        // still match the exact copy-on-write witness. If Codex reopens that
        // visible native alias and appends user work, it must reappear under
        // the stable retained identity on every review and in Activity and
        // Session Detail; the moved cloud thread remains bound to `fresh_id`.
        let appended_record = serde_json::to_vec(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:02Z",
            "ordinal": base_end_ordinal,
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "retained-local-q",
                "parent_id": "base-a",
                "role": "user",
                "content": [{"type": "input_text", "text": "retained local question"}],
            },
        }))
        .unwrap();
        let mut retained_after = base_before.clone();
        retained_after.extend_from_slice(&appended_record);
        retained_after.push(b'\n');
        std::fs::write(&base_rollout, &retained_after).unwrap();
        let retained_resource_id =
            conversation_resource_id(&codex_key(), &retained_thread_id).unwrap();
        for _ in 0..2 {
            let changed = get_project_thread_sync_comparison_with_repository(
                &fixture.repo,
                &project_id,
                &storage_id,
            )
            .unwrap();
            let retained = changed
                .entries
                .iter()
                .find(|entry| entry.provider_thread_id.as_deref() == Some(base_id))
                .expect("changed retained alias remains reviewable");
            assert_eq!(retained.conversation_resource_id, retained_resource_id);
            assert_eq!(retained.state, ThreadSyncState::LocalOnly);
            assert_eq!(
                retained.push_decision.action,
                ConversationReviewAction::CreateCloud
            );
            assert!(retained.push_decision.selectable);
        }
        let detail = get_project_session_detail_with_repository(
            &fixture.repo,
            &project_id,
            retained_resource_id.as_str(),
        )
        .unwrap();
        assert_eq!(detail.provider_session_id, base_id);
        let activity = get_project_activity_with_repository(
            &fixture.repo,
            &project_id,
            Some(&storage_id),
            None,
            None,
            Some(90),
            true,
        )
        .unwrap();
        assert!(activity.sessions.iter().any(|session| {
            session.provider_session_id.as_deref() == Some(base_id)
                && session.logical_thread_id.as_deref() == Some(retained_thread_id.as_str())
        }));

        let retained_push =
            push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(retained_push.revision, Some(3));
        let bindings = fixture.repo.load_conversation_bindings().unwrap();
        let retained_binding = bindings
            .bindings
            .iter()
            .find(|binding| binding.thread_id == retained_thread_id)
            .expect("publishing retained work creates its direct binding");
        assert_eq!(
            retained_binding.provider_thread_id.as_deref(),
            Some(base_id)
        );
        let moved_binding = bindings
            .bindings
            .iter()
            .find(|binding| binding.thread_id == base_thread_id)
            .unwrap();
        assert_eq!(moved_binding.provider_thread_id.as_deref(), Some(fresh_id));

        // Reusing and later copy-on-writing the retained branch must advance
        // its identity generation. R1 moving away from the same native alias
        // leaves R2 behind; it must never collide with R1 now bound to a
        // second fresh alias, even when binding timestamps have equal
        // second-level precision.
        let config_at_retained_push = fixture.repo.load_config().unwrap();
        let bindings_at_retained_push = fixture.repo.load_conversation_bindings().unwrap();
        let retained_cloud_record = serde_json::to_vec(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:03Z",
            "ordinal": base_end_ordinal + 1,
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "retained-cloud-a",
                "parent_id": "retained-local-q",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "retained cloud answer"}],
            },
        }))
        .unwrap();
        let mut retained_cloud = retained_after.clone();
        retained_cloud.extend_from_slice(&retained_cloud_record);
        retained_cloud.push(b'\n');
        std::fs::write(&base_rollout, &retained_cloud).unwrap();
        let retained_cloud_push =
            push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(retained_cloud_push.revision, Some(4));
        std::fs::write(&base_rollout, &retained_after).unwrap();
        fixture
            .repo
            .install_config_snapshot(config_at_retained_push)
            .unwrap();
        fixture
            .repo
            .install_conversation_bindings_snapshot(bindings_at_retained_push)
            .unwrap();

        let current_binding = fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&project_id)
            .cloned()
            .unwrap();
        let second_pull = plan_bundle_restore_with_repository(
            &fixture.repo,
            &storage_id,
            &bundle_id,
            &current_binding,
        )
        .unwrap();
        let retained_action = second_pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == retained_thread_id)
            .expect("cloud-advanced retained branch is materializable")
            .clone();
        assert_eq!(
            retained_action.action,
            ConversationPlanActionType::MaterializeCloud
        );
        let retained_fresh_id = retained_action
            .proposed_provider_thread_id
            .as_deref()
            .expect("retained branch update also uses copy-on-write")
            .to_string();
        assert_ne!(retained_fresh_id, base_id);
        let second_applied = apply_bundle_restore_with_repository(
            &fixture.repo,
            &second_pull.plan_id,
            vec![retained_action.action_id],
        )
        .unwrap();
        assert!(
            second_applied.success,
            "{:?}",
            second_applied.failed_actions
        );
        assert_eq!(std::fs::read(&base_rollout).unwrap(), retained_after);
        let second_retained_thread_id =
            retained_conversation_thread_id(&bundle_id, &codex_key(), &retained_thread_id, base_id)
                .unwrap();
        assert_ne!(second_retained_thread_id, retained_thread_id);
        let bindings = fixture.repo.load_conversation_bindings().unwrap();
        assert_eq!(
            bindings
                .bindings
                .iter()
                .find(|binding| binding.thread_id == retained_thread_id)
                .and_then(|binding| binding.provider_thread_id.as_deref()),
            Some(retained_fresh_id.as_str())
        );
        let unchanged_again = get_project_thread_sync_comparison_with_repository(
            &fixture.repo,
            &project_id,
            &storage_id,
        )
        .unwrap();
        assert!(unchanged_again
            .entries
            .iter()
            .all(|entry| entry.provider_thread_id.as_deref() != Some(base_id)));

        let second_local_record = serde_json::to_vec(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:04Z",
            "ordinal": base_end_ordinal + 1,
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "retained-second-a",
                "parent_id": "retained-local-q",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "second retained local answer"}],
            },
        }))
        .unwrap();
        let mut second_local = retained_after;
        second_local.extend_from_slice(&second_local_record);
        second_local.push(b'\n');
        std::fs::write(&base_rollout, second_local).unwrap();
        let second_changed = get_project_thread_sync_comparison_with_repository(
            &fixture.repo,
            &project_id,
            &storage_id,
        )
        .unwrap();
        let second_retained = second_changed
            .entries
            .iter()
            .find(|entry| entry.provider_thread_id.as_deref() == Some(base_id))
            .expect("second changed retained alias remains reviewable");
        assert_eq!(
            second_retained.conversation_resource_id,
            conversation_resource_id(&codex_key(), &second_retained_thread_id).unwrap()
        );
        assert_eq!(second_retained.state, ThreadSyncState::LocalOnly);
        assert_eq!(
            second_retained.push_decision.action,
            ConversationReviewAction::CreateCloud
        );
    }

    #[test]
    fn codex_cloud_update_stays_safe_when_a_fork_appears_after_review() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let project =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let project_id = project.project.local_project_id.clone();
        let bundle_id = project.project.bundle_id.clone();
        let storage_id = project.links[0].storage_id.clone();

        let base_id = "019f7500-0000-7000-8000-00000000bd01";
        let leaf_id = "019f7500-0000-7000-8000-00000000bd02";
        let sessions = fixture.codex_home.join("sessions/2026/08/10");
        std::fs::create_dir_all(&sessions).unwrap();
        let base_rollout = sessions.join(format!("rollout-{base_id}.jsonl"));
        let leaf_rollout = sessions.join(format!("rollout-{leaf_id}.jsonl"));
        let base_before = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("base-q", "user", "base question"),
                ("base-a", "assistant", "base answer"),
            ],
        );
        std::fs::write(&base_rollout, &base_before).unwrap();

        let first = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));
        let config_at_first = fixture.repo.load_config().unwrap();
        let bindings_at_first = fixture.repo.load_conversation_bindings().unwrap();
        let base_thread_id = bindings_at_first
            .bindings
            .iter()
            .find(|binding| binding.provider_thread_id.as_deref() == Some(base_id))
            .unwrap()
            .thread_id
            .clone();

        let base_after = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("base-q", "user", "base question"),
                ("base-a", "assistant", "base answer"),
                ("base-next", "user", "cloud advance"),
            ],
        );
        assert!(base_after.starts_with(&base_before));
        std::fs::write(&base_rollout, &base_after).unwrap();
        let second = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(second.revision, Some(2));
        std::fs::write(&base_rollout, &base_before).unwrap();
        fixture
            .repo
            .install_config_snapshot(config_at_first)
            .unwrap();
        fixture
            .repo
            .install_conversation_bindings_snapshot(bindings_at_first)
            .unwrap();

        let binding = fixture
            .repo
            .load_bindings()
            .unwrap()
            .active_for(&project_id)
            .cloned()
            .unwrap();
        let pull =
            plan_bundle_restore_with_repository(&fixture.repo, &storage_id, &bundle_id, &binding)
                .unwrap();
        let action = pull
            .conversation_actions
            .iter()
            .find(|action| action.thread_id == base_thread_id)
            .expect("cloud-advanced base is materializable")
            .clone();
        assert_eq!(action.action, ConversationPlanActionType::MaterializeCloud);
        let fresh_id = action
            .proposed_provider_thread_id
            .as_deref()
            .expect("Codex cloud update mints a fresh provider id");
        assert_ne!(fresh_id, base_id);
        let transcript_target = action
            .pins
            .target
            .provider_state
            .iter()
            .find(|target| !target.target_path.ends_with("session_index.jsonl"))
            .unwrap();
        assert_eq!(transcript_target.expectation, TargetExpectation::Absent);
        assert_ne!(
            Path::new(&transcript_target.target_path),
            std::fs::canonicalize(&base_rollout).unwrap()
        );

        // A native fork is created after the reviewed target and binding
        // pins were captured. Copy-on-write never targets the ancestor, so
        // the reviewed action remains safe without a racy dependency rescan.
        let base_end_ordinal = base_before.iter().filter(|byte| **byte == b'\n').count() as u64;
        let leaf_meta = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:00Z",
            "ordinal": base_end_ordinal,
            "type": "session_meta",
            "payload": {
                "id": leaf_id,
                "cwd": fixture.project_dir.to_string_lossy(),
                "history_mode": "paginated",
                "history_base": {
                    "thread_id": base_id,
                    "end_ordinal_exclusive": base_end_ordinal,
                    "end_byte_offset": base_before.len() as u64,
                },
            },
        }))
        .unwrap();
        let leaf_message = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-10T00:00:01Z",
            "ordinal": base_end_ordinal + 1,
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "leaf-q",
                "role": "user",
                "content": [{"type": "input_text", "text": "leaf question"}],
            },
        }))
        .unwrap();
        std::fs::write(
            &leaf_rollout,
            format!("{leaf_meta}\n{leaf_message}\n").as_bytes(),
        )
        .unwrap();
        codex_history_graph::resolve_codex_root(
            &leaf_rollout,
            &codex_rollout_lookup(&fixture.codex_home),
        )
        .unwrap();

        let applied = apply_bundle_restore_with_repository(
            &fixture.repo,
            &pull.plan_id,
            vec![action.action_id.clone()],
        )
        .unwrap();
        assert!(applied.success, "{:?}", applied.failed_actions);
        assert_eq!(std::fs::read(&base_rollout).unwrap(), base_before);
        let moved = fixture
            .repo
            .load_conversation_bindings()
            .unwrap()
            .bindings
            .into_iter()
            .find(|binding| binding.thread_id == base_thread_id)
            .unwrap();
        assert_eq!(moved.provider_thread_id.as_deref(), Some(fresh_id));
        assert!(Path::new(&transcript_target.target_path).is_file());
        codex_history_graph::resolve_codex_root(
            &leaf_rollout,
            &codex_rollout_lookup(&fixture.codex_home),
        )
        .unwrap();
    }

    /// Plan section 5.4: a same-length in-place mutation before the fixed
    /// start is invisible to cursor-riding review, and the publish
    /// reconstruction blocks it with the prefix witness.
    #[test]
    fn a_prefix_mutated_paginated_thread_is_blocked_by_the_witness_at_publish() {
        let fixture = setup_fixture();
        let draft = draft_ready_to_finalize(&fixture);
        let project =
            finalize_project_setup_with_repository(&fixture.repo, &draft.draft_id, draft.revision)
                .unwrap();
        let project_id = project.project.local_project_id.clone();
        let storage_id = project.links[0].storage_id.clone();

        // A paginated pair: the base rollout carries the fixed prefix, the
        // leaf continues at the base's exact native cutoff.
        let base_id = "019f7500-0000-7000-8000-00000000bb01";
        let leaf_id = "019f7500-0000-7000-8000-00000000bb02";
        let sessions = fixture.codex_home.join("sessions/2026/08/09");
        std::fs::create_dir_all(&sessions).unwrap();
        let base_rollout = sessions.join(format!("rollout-{base_id}.jsonl"));
        let base_bytes = command_test_transcript(
            base_id,
            &fixture.project_dir,
            &[
                ("b-1", "user", "prefix question"),
                ("b-2", "assistant", "prefix answer"),
            ],
        );
        std::fs::write(&base_rollout, &base_bytes).unwrap();

        let record = |ordinal: u64, id: &str, parent: Option<&str>, role: &str, text: &str| {
            serde_json::to_string(&serde_json::json!({
                "timestamp": "2026-08-09T00:00:01Z",
                "ordinal": ordinal,
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "id": id,
                    "parent_id": parent,
                    "role": role,
                    "content": [{
                        "type": if role == "user" { "input_text" } else { "output_text" },
                        "text": text,
                    }],
                },
            }))
            .unwrap()
        };
        let leaf_meta = serde_json::to_string(&serde_json::json!({
            "timestamp": "2026-08-09T00:00:00Z",
            "ordinal": 3,
            "type": "session_meta",
            "payload": {
                "id": leaf_id,
                "cwd": fixture.project_dir.to_string_lossy(),
                "history_mode": "paginated",
                "history_base": {
                    "thread_id": base_id,
                    "end_ordinal_exclusive": 3,
                    "end_byte_offset": base_bytes.len() as u64,
                },
            },
        }))
        .unwrap();
        let leaf_rollout = sessions.join(format!("rollout-{leaf_id}.jsonl"));
        std::fs::write(
            &leaf_rollout,
            format!(
                "{leaf_meta}\n{}\n",
                record(4, "m-1", None, "user", "tail start")
            ),
        )
        .unwrap();

        let first = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(first.revision, Some(1));

        // The thread advances locally, and the base suffers a same-length
        // in-place mutation before the fixed start.
        let mut appended = std::fs::read(&leaf_rollout).unwrap();
        appended.extend_from_slice(
            record(5, "m-2", Some("m-1"), "assistant", "tail answer").as_bytes(),
        );
        appended.push(b'\n');
        std::fs::write(&leaf_rollout, &appended).unwrap();
        let mutated = String::from_utf8(base_bytes.clone())
            .unwrap()
            .replace("prefix question", "prefix questioX");
        assert_eq!(mutated.len(), base_bytes.len());
        std::fs::write(&base_rollout, &mutated).unwrap();

        let error =
            push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap_err();
        assert!(
            error.contains("prefix witness no longer matches"),
            "{error}"
        );

        // Restoring the original prefix bytes publishes the advance cleanly.
        std::fs::write(&base_rollout, &base_bytes).unwrap();
        let second = push_bundle_with_repository(&fixture.repo, &project_id, &storage_id).unwrap();
        assert_eq!(second.revision, Some(2));
    }

    #[test]
    fn provider_thread_owner_conflicts_are_rejected_before_publication() {
        let host_id = HostId::parse("11111111111111111111111111111111").unwrap();
        let profile_id = LocalProviderProfileId::parse("profile-owner-check").unwrap();
        let provider_thread_id = "provider-owner-check";
        let thread_id = ThreadId::parse("thread-owner-check").unwrap();
        let operation = ConversationSyncOperation {
            operation_id: ConversationOperationId::parse("operation-owner-check").unwrap(),
            plan_id: None,
            plan_action_id: None,
            cloud: ConversationCloudAnchor {
                storage_id: StorageId::parse("storage-owner-check").unwrap(),
                bundle_id: BundleId::parse("0123456789abcdef0123456789abcdef").unwrap(),
                revision: 0,
                commit_id: "commit-owner-check".to_string(),
                manifest_sha256: "0".repeat(64),
            },
            host_id: host_id.clone(),
            replica_id: ReplicaId::parse("replica-new-owner").unwrap(),
            local_project_id: LocalProjectId::parse("project-new-owner").unwrap(),
            provider_profile_id: profile_id.clone(),
            provider: codex_key(),
            thread_id: thread_id.clone(),
            operation: ConversationSyncOperationKind::CloudPublication(
                ConversationPublicationOperation {
                    phase: PushJournalPhase::CandidateUploaded,
                    candidate_commit_id: "commit-owner-check".to_string(),
                    candidate_manifest_sha256: "0".repeat(64),
                    artifact_sha256: "a".repeat(64),
                    tail_before: None,
                    tail_after: PortableConversationTail {
                        codec: crate::project_sync::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                        partition_kind: crate::project_sync::domain::TailPartitionKind::WholeThread,
                        start_record_count: 0,
                        accepted_end_record_count: 1,
                        accepted_tail_state: "e".repeat(64),
                    },
                    planned_relation: TailClassification::MissingCloud,
                    sync_config_before: SyncConfig::default(),
                    sync_config_after: SyncConfig::default(),
                    bindings_before: crate::project_sync::domain::ConversationBindings::empty(
                        host_id.clone(),
                    ),
                    bindings_after: crate::project_sync::domain::ConversationBindings::empty(
                        host_id.clone(),
                    ),
                },
            ),
            provider_thread_id: Some(provider_thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            expected_host_identity_revision: 0,
            expected_bindings_document_revision: 0,
            expected_binding_revision: None,
            expected_profile_revision: 0,
            expected_project_binding_revision: 0,
            targets: Vec::new(),
            created_at: 1,
            updated_at: 1,
        };
        let mut owner = LocalConversationBinding {
            host_id,
            replica_id: ReplicaId::parse("replica-old-owner").unwrap(),
            local_project_id: LocalProjectId::parse("project-old-owner").unwrap(),
            provider_profile_id: profile_id,
            thread_id,
            forked_from: None,
            preserved: None,
            root_tail_layout: Some(RootTailLayout {
                codec: crate::project_sync::domain::PORTABLE_TAIL_CODEC_V1.to_string(),
                partition_kind: crate::project_sync::domain::TailPartitionKind::WholeThread,
                start_record_count: 0,
            }),
            provider_thread_id: Some(provider_thread_id.to_string()),
            materialization_state: ConversationMaterializationState::Resumable,
            revision: 0,
            created_at: 1,
            updated_at: 1,
        };

        let error = validate_cloud_publication_binding_owners(
            std::slice::from_ref(&owner),
            std::slice::from_ref(&operation),
        )
        .unwrap_err();
        assert!(error.contains("another local project registration"));
        assert!(!error.contains(provider_thread_id));

        owner.replica_id = operation.replica_id.clone();
        owner.local_project_id = operation.local_project_id.clone();
        assert!(validate_cloud_publication_binding_owners(&[owner], &[operation]).is_ok());
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
            second.draft.profiles.get(&codex_key())
        {
            assert_ne!(profile_id, &claimed_profile_id);
        }

        let mut colliding = second.draft.clone();
        colliding.profiles = BTreeMap::from([(
            codex_key(),
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
            codex_key(),
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
        let transaction = build_setup_transaction(&fixture.repo, &draft).unwrap();
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
            &ProviderKey::parse("claude").expect("static provider key"),
            &fixture._temp.path().join("claude-profile/.claude"),
        );
        let nested = fixture._temp.path().join("claude-profile/.claude/nested");
        std::fs::create_dir_all(&nested).unwrap();
        let _ = existing;
        let now = now_secs();
        let bad_profile = ProviderProfile {
            profile_id: LocalProviderProfileId::parse("profile-bad").unwrap(),
            provider: ProviderKey::parse("claude").expect("static provider key"),
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
        let mut transaction = build_setup_transaction(&fixture.repo, &draft).unwrap();
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
        let transaction = build_setup_transaction(&fixture.repo, &draft).unwrap();
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
