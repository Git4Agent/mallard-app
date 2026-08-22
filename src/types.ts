export type AppTheme = "light" | "dark";

export type ActivityLogLevel = "info" | "success" | "warning" | "error";

export type ActivityLogType =
  | "push"
  | "pull"
  | "repair"
  | "storage"
  | "configuration"
  | "history"
  | "system";

export interface ActivityLogContext {
  project_id?: string;
  project_name?: string;
  profile_id?: string;
  storage_id?: string;
  storage_name?: string;
  resource_id?: string;
  revision?: number;
}

export interface LogLine {
  schema?: number;
  id?: string;
  level: ActivityLogLevel;
  type?: ActivityLogType;
  event?: string;
  message: string;
  ts: number; // epoch ms
  run_id?: string;
  context?: ActivityLogContext;
}

export interface ActivityLogPolicy {
  schema: number;
  retention_days: number;
  max_total_bytes: number;
}

export interface ActivityLogStats {
  total_bytes: number;
  file_count: number;
  oldest_ts?: number | null;
  newest_ts?: number | null;
  policy: ActivityLogPolicy;
}

export interface ActivityLogQuery {
  types: ActivityLogType[];
  levels: ActivityLogLevel[];
  search?: string | null;
  cursor?: string | null;
  limit?: number;
}

export interface ActivityLogPage {
  entries: LogLine[];
  next_cursor?: string | null;
}

export interface ActivityLogCleanupRequest {
  delete_all?: boolean;
  older_than_days?: number | null;
}

export interface ActivityLogCleanupResult {
  removed_files: number;
  reclaimed_bytes: number;
  stats: ActivityLogStats;
}

/** What lives under ~/.mallard right now; backups called out because a wipe
 * deletes the only local copies of files Pull replaced. */
export interface LocalStateSummary {
  root_path: string;
  file_count: number;
  total_bytes: number;
  backups_bytes: number;
}

export interface LocalStateWipeResult {
  existed: boolean;
  files_removed: number;
  bytes_removed: number;
}

// ---------------------------------------------------------------------------
// Project-scoped sync (local state and portable bundles)
// ---------------------------------------------------------------------------

/**
 * Validated provider key. Providers are registered adapters, not a closed
 * union: the frontend renders whatever `list_agent_providers` returns and
 * never holds a fixed provider array.
 */
export type ProviderKey = string;

export type ProviderCapabilityLevel =
  | "activity_only"
  | "backup_only"
  | "resumable"
  | "resumable_with_conflict_preservation";

export interface ProviderDescriptor {
  key: ProviderKey;
  display_name: string;
  profile_label: string;
  default_profile_directory?: string | null;
  setup_default: boolean;
  capabilities: ProviderCapabilityLevel;
  adapter_version: number;
}

export type ProviderAccessState =
  | "bound_ready"
  | "bound_unavailable"
  | "view_only"
  | "unsupported";

export type ProjectResourceCategory =
  | "conversations"
  | "project_files"
  | "tools";

export type ProjectResourceKind =
  | "conversation"
  | "provider_state"
  | "project_content_file"
  | "project_content_directory"
  | "requirement";
export type ProjectApplyPolicy = "safe_file" | "merge" | "explicit_review" | "manual_only" | "never";

export interface ProjectResourceDescriptor {
  resource_id: string;
  kind: ProjectResourceKind;
  provider?: ProviderKey | null;
  provider_resource_type?: string | null;
  display_name: string;
  provenance: Record<string, unknown>;
  apply_policy: ProjectApplyPolicy;
  relative_cwd?: string | null;
  codec_version: number;
  metadata: Record<string, string>;
  // Inventory DTO presentation hints; these are not manifest identity.
  category?: ProjectResourceCategory | string;
  description?: string | null;
  logical_paths?: string[];
  /** Always emitted (`InventoryResource.default_selected`, no serde skip). */
  default_selected: boolean;
  blocked_reason?: string | null;
}

export interface RecipeEntry {
  resource_id: string;
  apply_policy: ProjectApplyPolicy;
  required: boolean;
}

export interface BundleRecipe {
  schema_version: 1;
  revision: number;
  entries: Record<string, RecipeEntry>;
}

export interface ResourceInventory {
  project: string;
  bundle_id?: string | null;
  resources: ProjectResourceDescriptor[];
  recipe: BundleRecipe;
  generated_at?: number;
  warnings?: string[];
}

export interface LocalProjectRegistration {
  local_project_id: string;
  bundle_id: string;
  display_name: string;
  /** Machine-local nickname; never pushed, so it can differ per checkout. */
  local_alias?: string | null;
  repository_fingerprint?: string | null;
  recipe: BundleRecipe;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface RepositoryBase {
  revision: number;
  commit_id: string;
  manifest_sha256: string;
  manifest_size: number;
}

/** Machine-local reviewed base for one sync scope ("project" or "provider:<key>"). */
export interface ScopeBase {
  scope: string;
  scope_revision: number;
  scope_sha256: string;
  reviewed_in: RepositoryBase;
  binding_revision: number;
  last_pull_at?: number | null;
  last_push_at?: number | null;
}

export type ScopeContinuity =
  | { state: "fresh" }
  | { state: "remote_missing"; base_revision: number }
  | { state: "unreviewed"; remote_revision: number }
  | { state: "in_sync"; revision: number }
  | { state: "advanced"; base_revision: number; remote_revision: number }
  | { state: "replaced"; base_revision: number; remote_revision: number };

export interface ProjectStorageLink {
  local_project_id: string;
  storage_id: string;
  bundle_id: string;
  /** Last resource selection explicitly published to this storage. */
  recipe?: BundleRecipe | null;
  /** Per-scope reviewed bases; a viewed scope has no entry. */
  scope_bases?: Record<string, ScopeBase>;
  /** Destination-local exclusions recorded only after a successful Push. */
  project_content_preferences: ProjectContentPreferences;
  /** Per-scope accepted conversation checkpoints. Machine-local and never
   *  rendered, so the payload stays opaque here — but it is required and
   *  deliberately non-defaulted in Rust, so any code rebuilding a link must
   *  pass it through untouched or the document fails to load. */
  conversation_checkpoints: Record<string, Record<string, unknown>>;
  pinned: boolean;
  created_at: number;
}

/** Derived machine-local sync state removed by an explicit project reset. */
export interface ProjectSyncMetadataResetResult {
  local_project_id: string;
  config_revision: number;
  links_reset: number;
  scope_bases_removed: number;
  conversation_checkpoints_removed: number;
  conversation_bindings_removed: number;
  materializations_removed: number;
  journal_operations_removed: number;
  restore_plans_removed: number;
  push_plans_removed: number;
  unreadable_plan_files_retained: number;
}

/** Result of permanently clearing Mallard-managed objects from one storage. */
export interface StorageDataClearResult {
  storage_id: string;
  config_revision: number;
  repository_objects_deleted: number;
  library_objects_deleted: number;
  project_links_reset: number;
  catalog_links_reset: number;
  local_records_removed: number;
  unreadable_plan_files_retained: number;
}

export interface ProjectContentPreferences {
  schema_version: 1;
  revision: number;
  excluded_resource_ids: string[];
}

export interface ProjectBinding {
  host_id: string;
  replica_id: string;
  local_project_id: string;
  bundle_id: string;
  project_root: string;
  canonical_project_root: string;
  profile_ids: Record<ProviderKey, string>;
  state: "active" | "detached";
  revision: number;
  updated_at: number;
}

export interface CodexConversationPathIssue {
  thread_id: string;
  transcript_path: string;
  recorded_cwd: string;
  target_cwd: string;
}

export interface CodexConversationPathAudit {
  local_project_id: string;
  profile_id?: string | null;
  profile_path?: string | null;
  project_root: string;
  assigned_thread_count: number;
  matching_thread_count: number;
  issues: CodexConversationPathIssue[];
  blockers: string[];
  warnings: string[];
  ready: boolean;
  can_repair: boolean;
}

export interface CodexConversationPathRepairResult {
  audit: CodexConversationPathAudit;
  repaired_thread_ids: string[];
  backup_dir?: string | null;
}

export interface LocalProjectSummary {
  local_project_id: string;
  bundle_id: string;
  display_name: string;
  local_alias?: string | null;
  revision: number;
  repository_fingerprint?: string | null;
  project_root?: string | null;
  canonical_project_root?: string | null;
  profile_ids?: Record<ProviderKey, string>;
  profile_names?: string[];
  providers?: ProviderKey[];
  resource_count?: number;
  selected_resource_count?: number;
  linked_storage_ids?: string[];
  is_git_repository?: boolean;
}

export interface ApplyReceipt {
  action_id: string;
  resource_id: string;
  action_type: string;
  logical_path?: string | null;
  source_sha256?: string | null;
  target_path?: string | null;
  target_sha256_after?: string | null;
  status: "applied" | "skipped" | "failed" | "blocked";
  applied_at: number;
  error?: string | null;
  conversation?: ConversationApplyReceipt | null;
}

export type TailPartitionKind = "whole_thread" | "fixed_paginated_tail";

/** The complete portable conversation identity compared by Refresh, Push, and Pull. */
export interface PortableConversationTail {
  codec: string;
  partition_kind: TailPartitionKind;
  start_record_count: number;
  accepted_end_record_count: number;
  accepted_tail_state: string;
}

export interface ConversationApplyReceipt {
  host_id: string;
  replica_id: string;
  thread_id: string;
  provider_profile_id: string;
  provider_thread_id?: string | null;
  materialization_state: "pending" | "resumable" | "stored_only";
  lineage_digest?: string | null;
  semantic_digest?: string | null;
  local_file_sha256_after?: string | null;
  cloud_revision: number;
  cloud_commit_id: string;
  cloud_manifest_sha256: string;
  published_origin_host_token?: string | null;
  published_origin_replica_token?: string | null;
  journal_operation_id: string;
  accepted_tail?: PortableConversationTail | null;
  preserved_thread_id?: string | null;
  preserved_display_title?: string | null;
  source_disposition?: "retained" | null;
}

export interface MaterializationRecord {
  materialization_id: string;
  plan_id: string;
  host_id: string;
  replica_id: string;
  local_project_id: string;
  storage_id: string;
  bundle_id: string;
  revision: number;
  commit_id: string;
  manifest_sha256: string;
  binding_revision: number;
  status: "partial" | "complete" | "detached";
  applied_at: number;
  receipts: ApplyReceipt[];
}

/** Exact response from the current `get_project` command. */
export interface ProjectDetail {
  project: LocalProjectRegistration;
  links: ProjectStorageLink[];
  binding?: ProjectBinding | null;
  materializations: MaterializationRecord[];
}

export type DraftProfileSelection =
  | { kind: "existing"; profile_id: string }
  | { kind: "pending"; path: string; display_name: string };

export type DraftStorageSelection =
  | { kind: "existing"; storage_id: string }
  | { kind: "pending"; storage: StorageConfig };

export type DraftRepositoryChoice =
  | { kind: "new" }
  | {
    kind: "existing";
    storage_id: string;
    bundle_id: string;
    display_name: string;
    repository_fingerprint?: string | null;
    mismatch_acknowledged: boolean;
  };

/** Machine-local, resumable project setup draft. */
export interface ProjectSetupDraft {
  schema: number;
  draft_id: string;
  local_project_id: string;
  new_bundle_id: string;
  project_root: string;
  canonical_project_root: string;
  display_name: string;
  repository_fingerprint?: string | null;
  profiles: Record<ProviderKey, DraftProfileSelection>;
  storage?: DraftStorageSelection | null;
  repository: DraftRepositoryChoice;
  revision: number;
  created_at: number;
  updated_at: number;
  last_error?: string | null;
}

export interface SetupDraftSummary {
  draft_id: string;
  display_name: string;
  project_root: string;
  updated_at: number;
  revision: number;
  status: "draft" | "attention" | string;
  last_error?: string | null;
}

export interface SetupDraftList {
  drafts: SetupDraftSummary[];
  warnings?: string[];
}

export interface CreateSetupDraftResult {
  draft: ProjectSetupDraft;
  resumed: boolean;
}

export interface SetupSectionStatus {
  section: "project" | "profiles" | "storage" | "repository" | string;
  state: "ready" | "attention" | "blocked" | string;
  message?: string | null;
}

export interface SetupDraftInspection {
  draft: ProjectSetupDraft;
  sections: SetupSectionStatus[];
  can_finalize: boolean;
}

export interface ProviderProfile {
  profile_id: string;
  provider: ProviderKey;
  display_name: string;
  path: string;
  canonical_path: string;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface ProviderProfileSummary extends ProviderProfile {
  available: boolean;
  readable: boolean;
  writable: boolean;
  used_by_projects: string[];
  error?: string | null;
}

export interface ProviderProfileProbe {
  provider: ProviderKey;
  requested_path: string;
  resolved_path: string;
  canonical_path: string;
  suggested_name: string;
  readable: boolean;
  writable: boolean;
  detected_child: boolean;
  existing_profile_id?: string | null;
}

export interface SaveProjectLinkRequest {
  local_project_id: string;
  storage_id: string;
  pinned: boolean;
}

export interface ConnectProjectBundleRequest {
  local_project_id: string;
  storage_id: string;
  bundle_id: string;
  expected_bundle_id: string;
  pinned: boolean;
  allow_repository_mismatch?: boolean;
}

export interface SaveProjectBindingRequest {
  local_project_id: string;
  project_root: string;
  profile_ids: Record<ProviderKey, string>;
  expected_revision?: number | null;
}

export interface HostIdentityStatus {
  host_id: string;
  revision: number;
  local_display_name?: string | null;
  created_at: number;
  updated_at: number;
}

/** Schema of `~/.mallard/sync_config.json`; must match the backend's LOCAL_SCHEMA. */
export const PROJECT_SYNC_SCHEMA = 1;

export interface SyncConfig {
  schema: typeof PROJECT_SYNC_SCHEMA;
  revision: number;
  storages: StorageConfig[];
  projects: LocalProjectRegistration[];
  links: ProjectStorageLink[];
}

/** Secret-bearing request sent only across the local Tauri IPC boundary. */
export interface ConnectR2StorageRequest {
  name: string;
  api_url: string;
  access_key_id: string;
  secret_access_key: string;
  expected_config_revision?: number | null;
}

/** Sanitized result: destination and credential fields are intentionally absent. */
export interface ConnectStorageResult {
  storage_id: string;
  config_revision: number;
  verified: boolean;
}

/** Filesystem watcher events contain no credential-bearing config fields. */
export interface ProjectSyncConfigChangedEvent {
  revision: number;
}

export interface StorageConfig {
  id: string;
  name: string;
  kind: "s3" | "local";
  bucket: string;
  access_key_id: string;
  secret_access_key: string;
  account_id: string;
  s3_endpoint: string;
  region: string;
  local_dir: string;
  supports_conditional_writes?: boolean | null;
}

export type ResourceSyncState =
  | "synced"
  | "local_only"
  | "local_ahead"
  | "storage_only"
  | "storage_ahead"
  | "diverged"
  | "blocked"
  | "unknown"
  | "not_compared";

export type ProjectFileSyncEligibilityState = "eligible" | "git_managed" | "unknown";

export interface ProjectFileSyncEligibility {
  state: ProjectFileSyncEligibilityState;
  reason: string;
  detected_root?: string | null;
}

export type ProjectContentEntryType = "file" | "directory" | "blocked";

export interface ProjectContentEntry {
  descriptor: ProjectResourceDescriptor;
  entry_type: ProjectContentEntryType;
  relative_path: string;
  logical_path: string;
  size?: number | null;
  mode?: number | null;
  source_mtime?: number | null;
  state: ResourceSyncState | string;
  local_present: boolean;
  storage_present: boolean;
  base_present: boolean;
  local_digest?: string | null;
  storage_digest?: string | null;
  base_digest?: string | null;
  review_digest?: string | null;
  selected_in_recipe: boolean;
  newly_discovered: boolean;
  selected_after_scan: boolean;
  blocked_reason?: string | null;
  warning_code?: string | null;
  warning_digest?: string | null;
}

export interface ProjectContentInventory {
  local_project_id: string;
  storage_id: string;
  project_root: string;
  eligibility: ProjectFileSyncEligibility;
  review_token?: string | null;
  storage_revision?: number | null;
  preference_revision: number;
  excluded_resource_ids: string[];
  entries: ProjectContentEntry[];
  ignored_count: number;
  blocked_count: number;
  warnings: string[];
  scanned_at: number;
}

export interface ProjectOperationResult {
  success: boolean;
  message: string;
  operation_id?: string;
  resources_changed?: number;
  revision?: number;
  results?: Array<{
    resource_id: string;
    state: string;
    message?: string;
  }>;
}

export interface RemoteBundleSummary {
  bundle_id: string;
  display_name: string;
  kind?: string;
  revision?: number;
  updated_at?: number;
  resource_count?: number;
  providers?: ProviderKey[];
  repository_fingerprint?: string | null;
}

export interface BundleSnapshotSummary extends RemoteBundleSummary {
  storage_id: string;
  /** A registration-only repo: linked but never pushed — nothing to pull yet. */
  registration_only?: boolean;
  resources?: ProjectResourceDescriptor[];
  recipe?: BundleRecipe;
  fetched_at?: number;
  warnings?: string[];
}

/** Post-setup convergence detection for one project × storage link. */
export type SiblingBundleStatus =
  | { state: "none" }
  | { state: "sibling_found"; matches: BundleSnapshotSummary[] }
  | { state: "registration_twin"; survivor: BundleSnapshotSummary };

export type PushDestinationStatus =
  | {
    state: "ready";
    revision?: number;
  }
  | {
    state: "remote_history_missing";
    base_revision: number;
  }
  | {
    state: "remote_history_replaced";
    base_revision: number;
    remote_revision: number;
    base_manifest_sha256: string;
    remote_manifest_sha256: string;
  }
  | {
    state: "requires_pull";
    reason: "remote_without_base" | "binding_changed" | "remote_advanced";
    remote_revision: number;
    base_revision?: number;
    /** Canonical scope keys that actually block this Push. */
    blocking_scopes?: string[];
  }
  | {
    state: "unavailable";
    message: string;
  };

/**
 * Repository-level continuity between this replica's recorded sync base and
 * the observed bundle head (DESIGN_REMOTE_HISTORY_CONTINUITY.md). Advisory:
 * it gates and explains, never mutates.
 */
export type RemoteContinuity =
  | { state: "fresh" }
  | { state: "remote_missing"; base_revision: number }
  | { state: "unreviewed"; remote_revision: number }
  | { state: "in_sync"; revision: number }
  | { state: "advanced"; base_revision: number; remote_revision: number }
  | { state: "replaced"; base_revision: number; remote_revision: number }
  | { state: "unknown"; message: string };

/**
 * Composite payload of `prepare_push_review`: one discovery pass and one
 * remote snapshot feed every push-dialog section. Degradable sections carry
 * per-section errors instead of failing the dialog.
 */
export interface PushReviewContext {
  project: ProjectDetail;
  inventory: ResourceInventory;
  destination_status: PushDestinationStatus;
  thread_comparison: ThreadSyncComparison | null;
  thread_comparison_error: string | null;
  project_files: ProjectContentInventory | null;
  project_files_error: string | null;
  conversation_push_plan_id?: string | null;
  conversation_action_ids?: Record<string, string>;
  prepared_at: number;
}

/** Payload of the `project-sync-fetch-progress` Tauri event. */
export interface BundleFetchProgress {
  phase: "plan" | "apply" | "push";
  storage_id: string;
  bundle_id: string;
  index: number;
  total_objects: number;
  fetched_bytes: number;
  total_bytes: number;
}

export type RestoreActionKind =
  | { kind: "write_file"; logical_path: string }
  | { kind: "write_project_file"; logical_path: string; mode: number; source_mtime: number }
  | { kind: "ensure_project_directory"; logical_path: string; mode: number; source_mtime: number }
  | { kind: "delete_project_file"; logical_path: string; last_sha256: string }
  | { kind: "delete_project_directory"; logical_path: string }
  | { kind: "merge_file"; logical_path: string }
  | { kind: "materialize_conversation"; provider: ProviderKey; logical_path: string }
  | { kind: "manual"; message: string };

export interface RestoreAction {
  action_id: string;
  resource_id: string;
  kind: RestoreActionKind;
  target_path?: string | null;
  source_sha256?: string | null;
  expected_target_sha256?: string | null;
  expected_target_mode?: number | null;
  /** Digest of the bytes apply would write; equal to expected_target_sha256 for an already-synced no-op. */
  materialized_sha256?: string | null;
  requires_explicit_approval: boolean;
}

export type ConversationTargetExpectation =
  | { state: "absent" }
  | { state: "sha256"; sha256: string };

export interface ConversationProviderStateExpectation {
  target_path: string;
  expectation: ConversationTargetExpectation;
}

/**
 * Portable provenance of a divergent keep-both preservation: the origin
 * thread the preserved copy split from and its 1-based per-origin ordinal.
 */
export interface PreservedProvenance {
  preserved_from: string;
  ordinal: number;
}

export interface ConversationRawRangePin {
  start_byte_offset: number;
  end_byte_offset: number;
  range_sha256: string;
}

export interface ConversationContextPins {
  host_id: string;
  replica_id: string;
  local_project_id: string;
  provider_profile_id: string;
  host_identity_revision: number;
  profile_revision: number;
  project_binding_revision: number;
  bindings_document_revision: number;
  binding_revision?: number | null;
}

export interface ConversationRevisionPins {
  bundle_revision: number;
  commit_id: string;
  manifest_sha256: string;
  scope_revision: number;
  scope_sha256: string;
}

export interface ConversationTailPins {
  accepted?: PortableConversationTail | null;
  accepted_local?: PortableConversationTail | null;
  local?: PortableConversationTail | null;
  cloud?: PortableConversationTail | null;
  checkpoint_generation?: number | null;
  lineage_digest?: string | null;
  semantic_digest?: string | null;
  raw_range?: ConversationRawRangePin | null;
}

export interface ConversationTargetPins {
  local: ConversationTargetExpectation;
  provider_state: ConversationProviderStateExpectation[];
}

export interface ConversationActionPins {
  context: ConversationContextPins;
  revision: ConversationRevisionPins;
  tail: ConversationTailPins;
  target: ConversationTargetPins;
  codec: string;
  expires_at: number;
}

export interface ConversationPlanAction {
  action_id: string;
  action: ConversationPlanActionType;
  provider: ProviderKey;
  /** Local thread; null exactly for create_local (minted at apply). */
  thread_id: string | null;
  /** Reviewed cloud row; null exactly for an unmapped create_cloud. */
  cloud_thread_id: string | null;
  source_resource_id?: string | null;
  pins: ConversationActionPins;
}

export interface ConversationReviewRow {
  provider: ProviderKey;
  thread_id: string | null;
  cloud_thread_id: string | null;
  resource_id: string;
  action: ConversationReviewAction;
  blocker?: ConversationBlocker | null;
  blocked_reason?: string | null;
  action_id?: string | null;
  /** The stored thread's name; absent when the stored thread has none. */
  display_title?: string | null;
}

export interface RestorePlan {
  schema_version: 1;
  plan_id: string;
  storage_id: string;
  bundle_id: string;
  host_id: string;
  replica_id: string;
  expected_host_identity_revision: number;
  revision: number;
  commit_id: string;
  manifest_sha256: string;
  binding_revision: number;
  created_at: number;
  expires_at: number;
  actions: RestoreAction[];
  conversation_actions: ConversationPlanAction[];
  conversation_review: ConversationReviewRow[];
  project_content_eligibility?: ProjectFileSyncEligibility | null;
  remote_continuity?: RemoteContinuity | null;
}

export type RestoreApplyStatus = "complete" | "recovery_required";

export interface PreservedThreadResult {
  action_id: string;
  original_thread_id: string;
  preserved_thread_id: string;
  /** The apply-minted marker title the preserved copy was renamed to. */
  preserved_display_title: string;
}

export interface RestoreResult {
  success: boolean;
  status: RestoreApplyStatus;
  message: string;
  plan_id?: string | null;
  /**
   * Present only on restore applies. `false` means the reviewed baseline
   * could not advance to the plan's revision because required changes
   * were skipped — Push stays gated until they are pulled.
   */
  reviewed_base_advanced?: boolean;
  applied_action_ids: string[];
  failed_actions: Array<{ action_id: string; display_name?: string | null; message: string }>;
  preserved_threads: PreservedThreadResult[];
  codex_app_refresh?: {
    operation_id: string;
    app_kind: "codex" | "codex_plus_plus";
    app_name: string;
    state: "restart_available" | "manual_restart";
    message: string;
  } | null;
}

export interface CodexAppRestartResult {
  success: boolean;
  message: string;
}

export interface BundleReadinessIssue {
  issue_id: string;
  category: ProjectResourceCategory | string;
  title: string;
  detail?: string | null;
  severity?: "info" | "warning" | "error" | string;
  provider?: ProviderKey | null;
  resource_id?: string | null;
  action?: string | null;
}

export interface BundleReadiness {
  bundle_id: string;
  state: "ready" | "needs_setup" | "blocked" | string;
  issues: BundleReadinessIssue[];
  generated_at?: number;
}

// ---------------------------------------------------------------------------
// Provider-neutral project Activity
// ---------------------------------------------------------------------------

export type SessionSourceState = "local_only" | "remote_only" | "local_and_remote";

export type SessionActionKind = "open_in_app" | "open_in_terminal";

export interface SessionActionDescriptor {
  action_id: string;
  kind: SessionActionKind;
  label: string;
  enabled: boolean;
  disabled_reason?: string | null;
}

export interface ProviderActivityState {
  provider: ProviderKey;
  display_name: string;
  access: ProviderAccessState;
  detail?: string | null;
}

export interface ProjectSessionSummary {
  /** Frontend key and selection identity: `<provider>:conversation:<thread>`. */
  resource_id: string;
  provider: ProviderKey;
  provider_display_name: string;
  provider_session_id?: string | null;
  logical_thread_id?: string | null;
  source: SessionSourceState;
  access: ProviderAccessState;
  title: string;
  preserved?: PreservedProvenance | null;
  summary?: string | null;
  started_at: number;
  ended_at: number;
  working_directory?: string | null;
  relative_cwd?: string | null;
  branch?: string | null;
  recorded_sha?: string | null;
  is_active: boolean;
  user_round_count: number;
  agent_message_count: number;
  tool_call_count: number;
  total_tokens?: number | null;
  metrics_complete: boolean;
  actions: SessionActionDescriptor[];
}

export interface CommitSessionReference {
  resource_id: string;
}

export type SessionCountQuality = "exact" | "lower_bound" | "inferred" | "unsupported";

export interface SessionCount {
  value?: number | null;
  quality: SessionCountQuality;
  explanation?: string | null;
}

export interface SessionEventCounts {
  prompts: SessionCount;
  responses: SessionCount;
  intermediate_steps: SessionCount;
  checkpoints: SessionCount;
  tool_calls: SessionCount;
  file_edits: SessionCount;
  shell_calls: SessionCount;
  reads: SessionCount;
  other_tools: SessionCount;
}

export type SessionTimelineEventKind =
  | "prompt"
  | "response"
  | "intermediate_step"
  | "checkpoint"
  | "tool_call";

export type SessionToolCategory = "file_edit" | "shell" | "read" | "other";

export type SessionToolCallStatus = "completed" | "failed" | "incomplete";

export interface SessionDetailWarning {
  code: string;
  message: string;
}

export interface SessionDetailCapabilities {
  intermediate_steps: boolean;
  checkpoints: boolean;
  tool_calls: boolean;
  event_detail: boolean;
}

export interface SessionCheckpoint {
  sha: string;
  short_sha: string;
  subject: string;
  committed_at: number;
  branch: string;
}

export interface ProjectSessionDetail {
  resource_id: string;
  provider: ProviderKey;
  provider_display_name: string;
  provider_session_id: string;
  title: string;
  summary?: string | null;
  started_at?: number | null;
  ended_at?: number | null;
  is_active: boolean;
  branch?: string | null;
  recorded_sha?: string | null;
  tokens: SessionCount;
  event_counts: SessionEventCounts;
  capabilities: SessionDetailCapabilities;
  actions: SessionActionDescriptor[];
  github_repository_url?: string | null;
  snapshot_id: string;
  warnings: SessionDetailWarning[];
}

export interface SessionToolCallPreview {
  provider_call_id: string;
  native_name: string;
  category: SessionToolCategory;
  status: SessionToolCallStatus;
  summary: string;
  argument_preview?: string | null;
  argument_truncated: boolean;
  result_preview?: string | null;
  result_truncated: boolean;
  affected_paths: string[];
}

export interface SessionTurnToolSummary {
  turn_id: string;
  tool_calls: SessionCount;
}

export interface SessionTimelineEvent {
  event_id: string;
  ordinal: number;
  timestamp?: number | null;
  kind: SessionTimelineEventKind;
  quality: SessionCountQuality;
  turn_id?: string | null;
  turn_tool_summary?: SessionTurnToolSummary | null;
  parent_event_id?: string | null;
  text?: string | null;
  tool?: SessionToolCallPreview | null;
  checkpoint?: SessionCheckpoint | null;
}

export interface ProjectSessionTimelinePage {
  resource_id: string;
  snapshot_id: string;
  events: SessionTimelineEvent[];
  next_cursor?: string | null;
  warnings: SessionDetailWarning[];
}

export interface SessionEventDetail {
  event_id: string;
  arguments?: string | null;
  arguments_truncated: boolean;
  result?: string | null;
  result_truncated: boolean;
  affected_paths: string[];
}

/** One embeddable in-project Markdown image, served as bounded base64. */
export interface ProjectMarkdownImage {
  mime_type: string;
  base64_data: string;
}

export interface ProjectSessionTimelineQuery {
  snapshot_id: string;
  cursor?: string | null;
  limit: number;
  included_event_kinds: SessionTimelineEventKind[];
  included_tool_categories: SessionToolCategory[];
  turn_id?: string | null;
}

export interface StorageSyncSummary {
  storage_id: string;
  storage_name: string;
  last_pull_at?: number | null;
  last_push_at?: number | null;
}

export type ThreadSyncState =
  | "synced"
  | "local_only"
  | "local_ahead"
  | "storage_only"
  | "storage_ahead"
  | "diverged"
  | "unavailable"
  | "unknown";


export type ConversationPlanActionType =
  | "publish"
  | "create_cloud"
  | "adopt_cloud_metadata"
  | "materialize_cloud"
  | "keep_both"
  | "create_local";

export type ConversationReviewAction =
  | "publish"
  | "create_cloud"
  | "no_changes"
  | "require_pull_metadata"
  | "require_pull"
  | "not_pushable"
  | "adopt_cloud_metadata"
  | "report_push_eligible"
  | "materialize_cloud"
  | "keep_both"
  | "create_local"
  | "report_local_only"
  | "not_found"
  | "blocked";

export type ConversationBlocker =
  | "unknown_evidence"
  | "detached_requires_pull"
  | "codec_unsupported"
  | "ambiguous_binding"
  | "missing_binding"
  | "busy"
  | "redaction_ruleset_mismatch"
  | "path_map_codec_mismatch";

export interface ThreadSyncDecision {
  action: ConversationReviewAction;
  blocker?: ConversationBlocker | null;
  selectable: boolean;
  blocked_reason?: string | null;
}

export interface ThreadSyncEntry {
  provider: ProviderKey;
  provider_display_name: string;
  access: ProviderAccessState;
  thread_id: string;
  conversation_resource_id: string;
  resource_id: string;
  display_name: string;
  state: ThreadSyncState;
  local_present: boolean;
  storage_present: boolean;
  status_detail?: string | null;
  local_updated_at?: number | null;
  storage_updated_at?: number | null;
  /**
   * The thread's honest cloud mapping (or the cloud row's own id for a
   * storage-only row); null for a local-only thread. No longer a presence
   * proxy for "row came from a storage comparison" — see
   * `compared_against_storage`.
   */
  cloud_thread_id?: string | null;
  forked_from?: string | null;
  /** True for rows produced by a storage comparison (sync review). */
  compared_against_storage: boolean;
  provider_thread_id?: string | null;
  push_decision: ThreadSyncDecision;
  pull_decision: ThreadSyncDecision;
}

export interface ThreadSyncCounts {
  synced: number;
  local: number;
  storage: number;
  diverged: number;
  unavailable: number;
  unknown: number;
}

/** One sync scope's continuity row in Push/Pull review. */
export interface ScopeSyncRow {
  scope: string;
  provider?: ProviderKey | null;
  access: ProviderAccessState;
  remote_revision?: number | null;
  base_revision?: number | null;
  continuity: ScopeContinuity;
}

export interface ThreadSyncComparison {
  project_id: string;
  storage_id: string;
  storage_name: string;
  revision?: number | null;
  base_revision?: number | null;
  compared_at: number;
  entries: ThreadSyncEntry[];
  counts: ThreadSyncCounts;
  scopes: ScopeSyncRow[];
  provider_errors: Record<ProviderKey, string>;
  warnings: string[];
  remote_continuity: RemoteContinuity;
}

export interface GitCommitSummary {
  sha: string;
  short_sha: string;
  committed_at: number;
  subject: string;
  session_refs: CommitSessionReference[];
}

export interface GitBranchSummary {
  name: string;
  is_current: boolean;
  available: boolean;
}

export interface GitHistoryPage {
  selected_branch: string;
  branches: GitBranchSummary[];
  commits: GitCommitSummary[];
  github_repository_url?: string | null;
  unique_session_count: number;
}

export interface UnmappedSessionReference {
  resource_id: string;
  reason: string;
}

export interface ProjectActivity {
  project_id: string;
  sessions: ProjectSessionSummary[];
  git?: GitHistoryPage | null;
  unmapped: UnmappedSessionReference[];
  providers: ProviderActivityState[];
  warnings: string[];
  window_start: number;
  window_end: number;
  next_before?: number | null;
  storage_sync: StorageSyncSummary[];
}

export interface ProjectActivityWorkspace {
  activity: ProjectActivity;
  comparison?: ThreadSyncComparison | null;
  comparison_error?: string | null;
}
