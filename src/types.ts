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
  generation?: number;
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

// ---------------------------------------------------------------------------
// Project-scoped sync (local state and portable bundles)
// ---------------------------------------------------------------------------

export type ProjectProvider = "codex" | "claude";

export type ProjectResourceCategory =
  | "conversations"
  | "skills"
  | "plugins"
  | "project_files"
  | "tools";

export type ProjectResourceState =
  | "synced"
  | "local_only"
  | "local_ahead"
  | "remote_only"
  | "remote_ahead"
  | "conflict"
  | "missing"
  | "blocked"
  | "needs_review"
  | "ready";

export type ProjectResourceKind =
  | "codex_conversation"
  | "claude_conversation"
  | "project_content_file"
  | "project_content_directory"
  | "project_memory"
  | "standalone_skill"
  | "plugin"
  | "requirement";

export type ProjectResourceScope = "project" | "provider_state" | "dependency" | "requirement";
export type ProjectApplyPolicy = "safe_file" | "merge" | "explicit_install" | "explicit_review" | "manual_only" | "never";

export interface ProjectResourceDescriptor {
  resource_id: string;
  kind: ProjectResourceKind;
  provider?: ProjectProvider | null;
  scope: ProjectResourceScope;
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
  default_selected?: boolean;
  selected_by_default?: boolean;
  blocked_reason?: string | null;
  provided_by?: string | null;
  install_behavior?: string | null;
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
  resources?: ProjectResourceDescriptor[];
  candidates?: ProjectResourceDescriptor[];
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
  recipe_bases: Record<string, {
    generation: number;
    manifest_sha256: string;
    commit_id?: string | null;
    recipe_revision: number;
    binding_revision?: number | null;
    last_pull_at?: number | null;
    last_push_at?: number | null;
  }>;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface ProjectStorageLink {
  local_project_id: string;
  storage_id: string;
  bundle_id: string;
  /** Last resource selection explicitly published to this storage. */
  recipe?: BundleRecipe | null;
  /** Destination-local exclusions recorded only after a successful Push. */
  project_content_preferences: ProjectContentPreferences;
  pinned: boolean;
  created_at: number;
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
  profile_ids: Partial<Record<ProjectProvider, string>>;
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
  profile_ids?: Partial<Record<ProjectProvider, string>>;
  profile_names?: string[];
  providers?: ProjectProvider[];
  resource_count?: number;
  selected_resource_count?: number;
  linked_storage_ids?: string[];
  readiness_state?: "ready" | "needs_setup" | "blocked" | string;
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
}

export interface MaterializationRecord {
  materialization_id: string;
  plan_id: string;
  host_id: string;
  replica_id: string;
  local_project_id: string;
  storage_id: string;
  bundle_id: string;
  generation: number;
  commit_id: string;
  manifest_sha256: string;
  binding_revision: number;
  status: "partial" | "complete" | "detached";
  applied_at: number;
  receipts: ApplyReceipt[];
}

/** Exact response from the schema-3 `get_project` command. */
export interface ProjectDetail {
  project: LocalProjectRegistration;
  links: ProjectStorageLink[];
  binding?: ProjectBinding | null;
  materializations: MaterializationRecord[];
}

export interface ProjectDiscovery {
  project_root: string;
  display_name: string;
  inventory: ResourceInventory;
  repository_fingerprint?: string | null;
  providers?: ProjectProvider[];
  profile_ids: Partial<Record<ProjectProvider, string>>;
  warnings?: string[];
}

export type DraftProfileSelection =
  | { kind: "existing"; profile_id: string }
  | { kind: "pending"; path: string; display_name: string };

export type DraftStorageSelection =
  | { kind: "existing"; storage_id: string }
  | { kind: "pending"; storage: StorageConfigV3 };

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

/** Machine-local, resumable project setup draft (schema-3 `project_drafts`). */
export interface ProjectSetupDraft {
  schema: number;
  draft_id: string;
  local_project_id: string;
  new_bundle_id: string;
  project_root: string;
  canonical_project_root: string;
  display_name: string;
  repository_fingerprint?: string | null;
  profiles: Partial<Record<ProjectProvider, DraftProfileSelection>>;
  storage?: DraftStorageSelection | null;
  repository: DraftRepositoryChoice;
  selected_resource_ids: string[];
  discovery_signature: string;
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
  section: "project" | "profiles" | "storage" | "repository" | "resources" | string;
  state: "ready" | "attention" | "blocked" | string;
  message?: string | null;
}

export interface SetupDraftInspection {
  draft: ProjectSetupDraft;
  sections: SetupSectionStatus[];
  inventory?: ResourceInventory | null;
  fresh_discovery_signature?: string | null;
  selection_stale: boolean;
  can_finalize: boolean;
  warnings?: string[];
}

export interface ProviderProfile {
  profile_id: string;
  provider: ProjectProvider;
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
  provider: ProjectProvider;
  requested_path: string;
  resolved_path: string;
  canonical_path: string;
  suggested_name: string;
  readable: boolean;
  writable: boolean;
  detected_child: boolean;
  existing_profile_id?: string | null;
}

export interface RegisterLocalProjectRequest {
  display_name: string;
  repository_fingerprint?: string | null;
  bundle_id?: string | null;
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
  profile_ids: Partial<Record<ProjectProvider, string>>;
  expected_revision?: number | null;
}

export interface HostIdentityStatus {
  host_id: string;
  revision: number;
  local_display_name?: string | null;
  created_at: number;
  updated_at: number;
  origin_key_available: boolean;
}

export interface UpdateHostDisplayNameRequest {
  expected_host_id: string;
  expected_revision: number;
  local_display_name?: string | null;
}

export interface RegisterNewHostRequest {
  expected_host_id: string;
  expected_revision: number;
  local_display_name?: string | null;
}

export interface OriginHostAlias {
  bundle_id: string;
  origin_host_token: string;
  display_name: string;
  revision: number;
  updated_at: number;
}

export interface SaveOriginHostAliasRequest {
  bundle_id: string;
  origin_host_token: string;
  display_name: string;
  expected_revision?: number | null;
}

export interface OriginHostLabel {
  origin_host_token: string;
  display_name: string;
  is_this_host: boolean;
}

export interface ProjectOriginTokens {
  bundle_id: string;
  origin_host_token: string;
  origin_replica_token: string;
}

/** Schema of `~/.mallard/config.json`; must match the backend's LOCAL_SCHEMA. */
export const PROJECT_SYNC_SCHEMA = 1;

export interface SyncConfigV3 {
  schema: typeof PROJECT_SYNC_SCHEMA;
  revision: number;
  storages: StorageConfigV3[];
  projects: LocalProjectRegistration[];
  links: ProjectStorageLink[];
}

export interface StorageConfigV3 {
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
  included_default_exclusions: string[];
  supports_conditional_writes?: boolean | null;
}

export interface BundleResourceStatus {
  resource_id: string;
  state: ProjectResourceState | string;
  message?: string | null;
  local_digest?: string | null;
  remote_digest?: string | null;
}

export interface ResourceStatusReport {
  project: string;
  storage: string;
  bundle_id?: string | null;
  generation?: number;
  statuses: Record<string, ProjectResourceState | string> | BundleResourceStatus[];
  warnings?: string[];
}

export type CapabilityStatusState =
  | "synced"
  | "local_only"
  | "local_ahead"
  | "storage_only"
  | "storage_ahead"
  | "diverged"
  | "blocked"
  | "unknown"
  | "not_compared";

export interface CapabilityProfileContext {
  provider: ProjectProvider;
  profile_id: string;
  display_name: string;
  path: string;
  shared_project_count: number;
}

export interface CapabilityStatusItem extends ProjectResourceDescriptor {
  category: "skills" | "plugins" | string;
  state: CapabilityStatusState | string;
  local_present: boolean;
  storage_present: boolean;
  selected_in_recipe: boolean;
  blocked_reason?: string | null;
  logical_paths: string[];
  local_digest?: string | null;
  storage_digest?: string | null;
  local_version?: string | null;
  storage_version?: string | null;
  enabled?: boolean | null;
  provided_skills: string[];
  message?: string | null;
}

export interface CapabilityStatusReport {
  project_id: string;
  project_name: string;
  profiles: CapabilityProfileContext[];
  storage_id?: string | null;
  storage_name?: string | null;
  generation?: number | null;
  base_generation?: number | null;
  compared_at: number;
  items: CapabilityStatusItem[];
  warnings: string[];
}

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
  state: CapabilityStatusState | string;
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
  storage_generation?: number | null;
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
  generation?: number;
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
  generation?: number;
  updated_at?: number;
  resource_count?: number;
  providers?: ProjectProvider[];
  repository_fingerprint?: string | null;
}

export interface BundlePage {
  bundles: RemoteBundleSummary[];
  next_cursor?: string | null;
}

export interface BundleSnapshotSummary extends RemoteBundleSummary {
  storage_id: string;
  resources?: ProjectResourceDescriptor[];
  recipe?: BundleRecipe;
  fetched_at?: number;
  warnings?: string[];
}

export type RestoreActionKind =
  | { kind: "write_file"; logical_path: string }
  | { kind: "write_project_file"; logical_path: string; mode: number; source_mtime: number }
  | { kind: "ensure_project_directory"; logical_path: string; mode: number; source_mtime: number }
  | { kind: "delete_project_file"; logical_path: string; last_sha256: string }
  | { kind: "delete_project_directory"; logical_path: string }
  | { kind: "merge_file"; logical_path: string }
  | { kind: "materialize_conversation"; provider: ProjectProvider; logical_path: string }
  | { kind: "install_custom_skill"; provider: ProjectProvider; skill_name: string }
  | { kind: "overwrite_custom_skill"; provider: ProjectProvider; skill_name: string }
  | { kind: "manual"; message: string };

export interface RestoreAction {
  action_id: string;
  resource_id: string;
  kind: RestoreActionKind;
  target_path?: string | null;
  source_sha256?: string | null;
  expected_target_sha256?: string | null;
  expected_target_mode?: number | null;
  requires_explicit_approval: boolean;
}

export interface RestorePlan {
  schema_version: 1;
  plan_id: string;
  storage_id: string;
  bundle_id: string;
  host_id: string;
  replica_id: string;
  expected_host_identity_revision: number;
  generation: number;
  commit_id: string;
  manifest_sha256: string;
  binding_revision: number;
  created_at: number;
  expires_at: number;
  actions: RestoreAction[];
  project_content_eligibility?: ProjectFileSyncEligibility | null;
}

export interface RestoreResult {
  success: boolean;
  message: string;
  plan_id?: string;
  applied_action_ids?: string[];
  failed_actions?: Array<{ action_id: string; display_name?: string | null; message: string }>;
}

export interface DependencyPlan {
  schema_version: 1;
  plan_id: string;
  storage_id: string;
  bundle_id: string;
  host_id: string;
  replica_id: string;
  expected_host_identity_revision: number;
  generation: number;
  commit_id: string;
  manifest_sha256: string;
  binding_revision: number;
  created_at: number;
  expires_at: number;
  actions: DependencyAction[];
  blockers?: string[];
  warnings?: string[];
}

export interface DependencyAction {
  action_id: string;
  resource_id: string;
  kind: "install_codex_plugin" | "install_claude_plugin" | "install_standalone_skill" | "check_binary" | "check_environment" | "manual";
  display_name: string;
  provider?: ProjectProvider | null;
  argv: string[];
  requires_explicit_approval: boolean;
}

export interface DependencyResult {
  success: boolean;
  message: string;
  applied_action_ids?: string[];
  failed_actions?: Array<{ action_id: string; display_name?: string | null; message: string }>;
}

export interface BundleReadinessIssue {
  issue_id: string;
  category: ProjectResourceCategory | string;
  title: string;
  detail?: string | null;
  severity?: "info" | "warning" | "error" | string;
  provider?: ProjectProvider | null;
  resource_id?: string | null;
  action?: string | null;
}

export interface BundleReadiness {
  bundle_id: string;
  state: "ready" | "needs_setup" | "blocked" | string;
  issues: BundleReadinessIssue[];
  generated_at?: number;
}

export interface CodexThreadSummary {
  thread_id: string;
  title: string;
  summary: string;
  started_at: number;
  ended_at: number;
  branch?: string | null;
  recorded_sha?: string | null;
  is_active?: boolean;
  user_round_count: number;
  agent_message_count: number;
  tool_call_count: number;
  total_tokens?: number | null;
  metrics_complete: boolean;
  commit_occurrence_count: number;
}

export interface CommitThreadReference {
  thread_id: string;
}

export type ChatTurnRole = "user" | "assistant";

export interface ChatTurnPreview {
  ordinal: number;
  role: ChatTurnRole;
  timestamp?: number | null;
  preview: string;
}

export interface CodexThreadDetailsPage {
  thread_id: string;
  turns: ChatTurnPreview[];
  next_cursor?: number | null;
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

export type ConversationRelation =
  | "identical"
  | "cloud_prefix_of_local"
  | "local_prefix_of_cloud"
  | "diverged"
  | "unknown"
  | "local_missing"
  | "cloud_missing"
  | "both_missing";

export type ConversationPlanActionType =
  | "no_op"
  | "fast_forward_local"
  | "fast_forward_cloud"
  | "keep_local"
  | "require_pull"
  | "fork"
  | "create_cloud"
  | "create_local"
  | "store_only"
  | "replace_local"
  | "replace_cloud"
  | "tombstone_thread"
  | "blocked";

export type ConversationReasonCode =
  | "identical"
  | "structural_rewrite_only"
  | "cloud_prefix_of_local"
  | "local_prefix_of_cloud"
  | "diverged"
  | "unknown_lineage"
  | "local_missing"
  | "cloud_missing"
  | "both_missing"
  | "provider_not_quiescent"
  | "codec_unsupported"
  | "ambiguous_binding"
  | "missing_binding"
  | "tombstone_unchanged"
  | "tombstone_modified"
  | "stale_plan";

export type ConversationMaterializationState = "pending" | "resumable" | "stored_only";

export interface ThreadSyncEntry {
  thread_id: string;
  resource_id: string;
  display_name: string;
  state: ThreadSyncState;
  local_present: boolean;
  storage_present: boolean;
  status_detail?: string | null;
  local_updated_at?: number | null;
  storage_updated_at?: number | null;
  cloud_thread_id?: string | null;
  forked_from?: string | null;
  provider_thread_id?: string | null;
  relation?: ConversationRelation | null;
  proposed_action?: ConversationPlanActionType | null;
  reason_code?: ConversationReasonCode | null;
  materialization_state?: ConversationMaterializationState | null;
  resumable?: boolean;
  origin_label?: string | null;
  available_actions?: ConversationPlanActionType[];
  requires_explicit_approval?: boolean;
}

export interface ThreadSyncCounts {
  synced: number;
  local: number;
  storage: number;
  diverged: number;
  unavailable: number;
  unknown: number;
}

export interface ThreadSyncComparison {
  project_id: string;
  storage_id: string;
  storage_name: string;
  generation?: number | null;
  base_generation?: number | null;
  compared_at: number;
  entries: ThreadSyncEntry[];
  counts: ThreadSyncCounts;
  warnings: string[];
}

export interface GitCommitSummary {
  sha: string;
  short_sha: string;
  committed_at: number;
  subject: string;
  thread_refs: CommitThreadReference[];
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
  next_cursor?: string | null;
  unique_thread_count: number;
  reference_count: number;
}

export interface UnmappedThreadReference {
  thread_id: string;
  reason: string;
}

export interface ProjectChatHistory {
  project_id: string;
  codex_home: string;
  threads: CodexThreadSummary[];
  git?: GitHistoryPage | null;
  unmapped: UnmappedThreadReference[];
  warnings: string[];
  window_start: number;
  window_end: number;
  next_before?: number | null;
  storage_sync: StorageSyncSummary[];
}
