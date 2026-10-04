import type { ManagedCloudListingKind, ManagedCloudStatus } from "./components/managed-cloud/types";
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

/** Backend tuning knobs, persisted in ~/.mallard/settings.json. Every field
 * has a serde default on the Rust side, so a partial file still parses. */
export interface AppSettings {
  push_codex_children: boolean;
  push_claude_children: boolean;
  credential_redaction_enabled: boolean;
  /** Scheduled update checks, which download a newer release in the
   * background; a manual check ignores this. */
  update_check_enabled: boolean;
  schema: number;
  conversation_file_limit_bytes: number;
  timeline_preview_limit_bytes: number;
  comparison_workers: number;
  cloud_transfer_concurrency: number;
}

/** `prepare_app_update`: a newer release is ready once its package is
 * downloaded and its signature verified. It installs on restart or quit. */
export interface AppUpdateStatus {
  current_version: string;
  ready_version: string | null;
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

export interface ProviderCliLauncher {
  /** Executable name resolved by the opened terminal's login shell. */
  program: string;
  /** Human name of the agent CLI, e.g. "Claude Code". */
  display_name: string;
}

/** Where a provider-declared maintenance action applies. */
export type MaintenanceActionScope = "profile" | "after_pull";

/** One provider-declared maintenance action. The frontend renders declared
 *  actions and never checks a provider key to decide whether to show one. */
export interface MaintenanceActionDescriptor {
  action_id: string;
  label: string;
  description: string;
  scope: MaintenanceActionScope;
  requires_confirmation: boolean;
  read_only: boolean;
  /** Display order among one provider's actions. */
  display_rank: number;
}

export interface ProviderDescriptor {
  key: ProviderKey;
  display_name: string;
  /** Lower ranks first, then key order. The backend already sorts by it;
   *  the UI keeps that order. */
  display_rank: number;
  profile_label: string;
  default_profile_directory?: string | null;
  setup_default: boolean;
  /** Present when the provider ships an agent CLI that takes a prompt. */
  cli_launcher?: ProviderCliLauncher | null;
  /** Declared maintenance actions, in display order. */
  maintenance_actions: MaintenanceActionDescriptor[];
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

/**
 * One exact portable contract — a resource kind or an apply policy — as an
 * open versioned coordinate, never a closed union. A registered backend
 * adapter resolves it; the frontend only renders the labels it is handed and
 * never compares a key against a literal.
 */
export interface ContractCoordinate {
  key: string;
  version: number;
}

/** Provenance as a versioned kind plus bounded, codec-typed fields. */
export interface ProvenanceEnvelope {
  kind: string;
  version: number;
  fields: Record<string, unknown>;
}

/** One portable resource exactly as the manifest carries it. */
export interface ProjectResourceDescriptor {
  resource_id: string;
  kind: ContractCoordinate;
  provider?: ProviderKey | null;
  /** Presentation-only provider subtype (`session`, `memory`, `todo`, …). */
  provider_resource_type?: string | null;
  display_name: string;
  provenance: ProvenanceEnvelope;
  apply_policy: ContractCoordinate;
  /** The provider content codec version, independent of every contract
   *  version above. */
  codec_version: number;
  /** Bounded, kind-typed JSON object. */
  attributes: Record<string, unknown>;
}

/**
 * The flattened `InventoryResource` DTO: the descriptor plus the safe
 * presentation the registered contracts supply. The frontend renders these
 * labels; it never dispatches on a kind or policy key.
 */
export interface InventoryResource extends ProjectResourceDescriptor {
  category: ProjectResourceCategory;
  kind_label: string;
  apply_policy_label: string;
  /** The resource can be selected for sync in this build: its contracts are
   *  registered and its policy is not `never`. */
  selectable: boolean;
  description?: string | null;
  logical_paths?: string[];
  /** Always emitted (`InventoryResource.default_selected`, no serde skip). */
  default_selected: boolean;
  blocked_reason?: string | null;
}

export interface RecipeEntry {
  resource_id: string;
  /** Copied verbatim from the resource's descriptor; the backend aligns
   *  recipe policies to the descriptor anyway. */
  apply_policy: ContractCoordinate;
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
  resources: InventoryResource[];
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

/**
 * One sync scope against its reviewed base. A base reviewed in a history the
 * storage no longer has is read as absent, so a cleared or recreated storage
 * shows here as `fresh` or `unreviewed`, never as a state of its own.
 */
export type ScopeContinuity =
  | { state: "fresh" }
  | { state: "remote_missing"; base_revision: number }
  | { state: "unreviewed"; remote_revision: number }
  | { state: "in_sync"; revision: number }
  | { state: "advanced"; base_revision: number; remote_revision: number };

export interface ProjectStorageLink {
  managed_cloud_repository_id?: string | null;
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
}

/** Best-effort deletion in the project's bound Codex and Claude profiles. */
export interface ProjectSessionsDeleteResult {
  sessions_deleted: number;
  sessions_failed: number;
  provider_errors: string[];
  metadata_reset: ProjectSyncMetadataResetResult | null;
  metadata_error: string | null;
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

/** The generic request every maintenance action receives. */
export interface MaintenanceActionRequest {
  provider: ProviderKey;
  action_id: string;
  /** Adapter-typed arguments, for example the ones an after-pull offer
   *  handed out. */
  arguments?: Record<string, unknown>;
  /** The user confirmed the action's declared confirmation prompt. */
  confirmed: boolean;
}

/** The generic report every maintenance action returns. Provider-typed
 *  extras travel in `facts`; the frontend renders the common fields. */
export interface MaintenanceActionResult {
  provider: ProviderKey;
  action_id: string;
  ready: boolean;
  summary: string;
  facts: Record<string, unknown>;
}

/** One action a completed Pull offers, declared by the provider adapter. */
export interface AfterPullActionOffer {
  provider: ProviderKey;
  action_id: string;
  label: string;
  message: string;
  /** The action can run now; otherwise the message explains what to do
   *  by hand. */
  available: boolean;
  /** Safe, adapter-typed arguments the shell hands back on execution. */
  arguments: Record<string, unknown>;
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
  operation_id: string;
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

export interface DraftStorageSelection {
  storage_id: string;
}

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
  status: "draft" | "attention";
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

/** What setup would make of one folder, answered without writing. */
export type ProjectFolderProbe =
  | { kind: "invalid"; message: string }
  | { kind: "missing" }
  | { kind: "unreadable"; message: string }
  | { kind: "not_a_directory"; parent?: string | null }
  | { kind: "mallard_data"; contains: boolean }
  | {
    kind: "folder";
    canonical_path: string;
    is_git_repository: boolean;
    existing_draft_id?: string | null;
    existing_project_ids: string[];
  };

/** One folder recent sessions ran in, and what choosing it would do. */
export interface AgentFolder {
  path: string;
  providers: ProviderKey[];
  /** Unix seconds of the newest session file that named the folder. */
  last_active: number;
  probe: ProjectFolderProbe;
}

export interface AgentFolderList {
  folders: AgentFolder[];
}

export interface SetupSectionStatus {
  section: "project" | "profiles" | "storage" | "repository";
  state: "ready" | "attention" | "blocked";
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
  /** Redacted storage summaries built by each storage adapter. Never a
   *  secret, never a full provider configuration. */
  storages: StorageSummary[];
  projects: LocalProjectRegistration[];
  links: ProjectStorageLink[];
}

/** Stable storage-provider identity (`local`, `r2`, …); never a closed union. */
export type StorageProviderKey = string;

export type StorageFieldKind = "text" | "url" | "directory";

/** One safe, editable configuration field a storage provider declares. */
export interface StorageFieldDescriptor {
  field_id: string;
  label: string;
  kind: StorageFieldKind;
  required: boolean;
  placeholder?: string | null;
  help?: string | null;
  /** Documentation the field's help mark opens. */
  help_url?: string | null;
  /** Filled in from another field as it is typed; the backend still validates. */
  derive?: StorageFieldDerivation | null;
}

/** A suggested storage name: capture group `group` of `pattern` matched
 *  against the `source` field. */
export interface StorageNameSuggestion {
  source: string;
  pattern: string;
  group: number;
  /** Where the capture goes: `"Dropbox {1}"`, or `"{1}"` for the name alone. */
  template: string;
}

/** One declarative extraction: capture group `group` of `pattern` matched
 *  against the `source` field becomes this field's value; no match clears it. */
export interface StorageFieldDerivation {
  source: string;
  pattern: string;
  group: number;
}

/** One secret a storage provider accepts. Its value never returns. */
export interface StorageSecretDescriptor {
  secret_id: string;
  label: string;
  required: boolean;
  placeholder?: string | null;
  help?: string | null;
  help_url?: string | null;
}



/** The New-storage question, Create new or Use existing, for a provider
 *  whose setup lane can reach a destination Mallard already uses — by
 *  looking in the account or by a pasted link. Mirrors `StorageSetupDoors`. */
export interface StorageSetupDoors {
  /** The Create-new door's sentence. */
  create_hint: string;
  /** The Use-existing door's sentence. */
  existing_hint: string;
  /** Optional access guide beside the Use-existing choice. */
  existing_help?: { label: string; url: string } | null;
}

/** The provider can look in the signed-in account for destinations Mallard
 *  already uses. Every sentence the look prints comes from here, so the page
 *  never names a provider. */
export interface StorageDiscoveryDescriptor {
  /** What one candidate is called: "bucket", "shared folder". */
  unit: string;
  unit_plural: string;
  /** Where the look happens, for prose: "your Cloudflare account". */
  place: string;
  /** The lane's verb: "Find in my account". */
  find_label: string;
  /** What the look signs in with, and that it only reads. */
  access_hint: string;
  /** Printed while the scan runs. */
  scanning_hint: string;
  /** Printed after "Approve access to <name> in your browser." while the
   *  chosen destination's credential is being issued. */
  prepare_hint: string;
}

/** What the R2 setup helper reports while it works, as it happens: which
 *  Wrangler passed the check, a page to visit in the browser, the bucket a
 *  setup will authorize. Mirrors `R2SetupProgress`. */
export type R2SetupProgress =
  | { status: "wrangler_ready"; version: string; path: string }
  | { status: "authorization_link"; stage: "login" | "bucket"; url: string }
  | { status: "bucket_ready"; bucket: string };

/** Account choices returned by the local Wrangler check. */
export interface R2Account {
  id: string;
  name: string;
}

/** Where a bucket's data lives. Cloudflare fixes it when the bucket is
 *  created; a scan lists buckets across all four. */
export type R2Jurisdiction = "default" | "eu" | "us" | "fedramp";

export type R2SetupErrorCode =
  | "wrangler_not_found"
  | "wrangler_runtime_unavailable"
  | "wrangler_update_required"
  | "sign_in_required"
  | "authorization_timeout"
  | "authorization_unavailable"
  | "account_ambiguous"
  | "account_unavailable"
  | "draft_changed"
  | "setup_busy"
  | "creation_unknown"
  | "permission_denied"
  | "r2_not_enabled"
  | "authentication_failed"
  | "rate_limited"
  | "service_unavailable"
  | "request_rejected"
  | "identity_mismatch"
  | "bucket_name_taken"
  | "cancelled"
  | "helper_unavailable"
  | "setup_timeout"
  | "setup_failed";

export interface R2SetupFailure {
  code: R2SetupErrorCode;
}

export interface StorageProviderDescriptor {
  key: StorageProviderKey;
  config_version: number;
  display_name: string;
  description: string;
  /** The stem a new draft's name starts from (`{stem} {n}`). */
  default_name?: string | null;
  /** Where a suggested name comes from while the form is filled; applied only
   *  while the name is still the default or the previous suggestion. */
  name_suggestion?: StorageNameSuggestion | null;
  /** Lower ranks first, then key order. */
  display_rank: number;
  /** A generic presentation hint (`cloud`, `drive`). */
  icon: string;
  /** Deletion semantics for the storage-wide clear action. */
  clear_mode: "revision_checked" | "unconditional" | "unavailable";
  fields: StorageFieldDescriptor[];
  secrets: StorageSecretDescriptor[];
  managed_secret_id?: string | null;
  local_setup?: "wrangler" | "dropbox" | "feishu" | "managed_cloud" | null;
  /** The provider's setup documentation, offered as a "Setup guide" link. */
  setup_guide_url?: string | null;
  /** The sentence introducing the manual fields; defaults to a generic one. */
  manual_setup_hint?: string | null;
  /** Absent for a provider whose setup cannot reach an existing
   *  destination: a new draft then opens without the Create new / Use
   *  existing question. */
  setup_doors?: StorageSetupDoors | null;
  /** Absent for a provider that cannot look in an account. */
  discovery?: StorageDiscoveryDescriptor | null;
}

/** What the marker read at a candidate's root said. Kept apart on purpose:
 *  a missing marker is not an unreadable one, and neither is a marker
 *  written by a newer layout. */
export type StorageDiscoveryMarker = "storage" | "missing" | "unsupported" | "invalid" | "unreadable";

/** One provider fact a candidate row shows as a chip. */
export interface StorageDiscoveryFact {
  label: string;
  value: string;
}

/** One destination a scan checked. `candidate_id` is a reference inside its
 *  scan, never a storage id; the destination itself stays with the backend,
 *  which re-verifies it before any credential is issued. */
export interface StorageDiscoveryCandidate {
  candidate_id: string;
  /** The provider's label for the destination — not its identity. */
  name: string;
  /** The identity printed beside the name when the name alone is not one
   *  (a namespace ID); null when the name is the identity within the scope. */
  identity?: string | null;
  marker: StorageDiscoveryMarker;
  /** The layout version found, or the fixed read error. */
  marker_detail?: string | null;
  facts: StorageDiscoveryFact[];
  /** Access limits the row states: a share not mounted, a role that cannot Push. */
  cautions: string[];
  /** The configured storage that already points at this destination. */
  connected_storage_id?: string | null;
}

/** A read-only look at the account for the draft's provider. */
export interface StorageDiscoveryResult {
  /** Names this scan to the prepare step; a later scan retires it. */
  scan_id: string;
  /** Who and where was looked at: the account and jurisdictions, or the
   *  signed-in user and the shares covered. */
  scope: string;
  /** `partial` when a page, a read, or a limit stopped the scan short: the
   *  candidates found still stand, and `issues` says what was skipped. */
  status: "complete" | "partial";
  issues: string[];
  candidates: StorageDiscoveryCandidate[];
}

/** One safe, redacted fact about a configured storage. */
export interface StorageSummaryField {
  field_id: string;
  label: string;
  value: string;
  /** The value is a machine-local path the UI may reveal. */
  is_path: boolean;
}

/** What the open transport last observed about a storage's credential.
 *  `connected` after a success; the three suspending states need a new
 *  credential or consent; `unreachable` is a temporary failure. */
export type StorageConnectionState =
  | "connected"
  | "reconnect_required"
  | "token_expired"
  | "permission_denied"
  | "unreachable";

/** Allowlisted facts about one refused request: the route literal, the
 *  HTTP status, the provider's fixed error identifier, and its request ID.
 *  Never a body, never a token. */
export interface StorageConnectionEvidence {
  endpoint: string;
  http_status?: number | null;
  error_tag?: string | null;
  request_id?: string | null;
}

/** Runtime connection health from the process-shared store. Absent until a
 *  request has been made in this process; never persisted. */
export interface StorageConnectionHealth {
  state: StorageConnectionState;
  code: string;
  message: string;
  observed_at: number;
  evidence?: StorageConnectionEvidence | null;
}

/** One configured storage as every listing shows it. */
export interface StorageSummary {
  storage_id: string;
  name: string;
  provider: StorageProviderKey;
  config_version: number;
  provider_label: string;
  icon: string;
  /** False when this build has no adapter for the provider and version:
   *  the record is preserved and shown, never edited, tested, or synced. */
  available: boolean;
  summary: StorageSummaryField[];
  /** The saved settings, shown read-only: a destination is fixed for the
   *  life of a storage. */
  settings: StorageSettings;
  /** Which declared secrets the storage holds. A value never rides a
   *  listing; `reveal_storage_secret` returns one on an explicit request. */
  secrets: StorageSecretPresence[];
  connection?: StorageConnectionHealth | null;
}

export interface StorageSecretPresence {
  secret_id: string;
  present: boolean;
}

export interface TestStorageResult {
  storage_id: string;
  config_revision: number;
  verified: boolean;
}

/** Safe provider settings: a bounded JSON object of scalar-ish values. */
export type StorageSettings = Record<string, unknown>;

/** A secret presence operation. A saved value is never returned to the
 *  form; the form reports only what should happen to it. */
export type SecretOperation =
  | { op: "set"; value: string }
  | { op: "replace"; value: string }
  | { op: "clear" }
  | { op: "unchanged" };

export interface StorageDraftSecretState {
  secret_id: string;
  present: boolean;
  /** Only on the exact fetch of a draft that creates a new storage: the
   *  machine-local value, shown for review before Finish setup. */
  value?: string | null;
}

/** A storage setup draft as it crosses IPC. Edit drafts expose only secret
 *  presence; an exact fetch of a new draft can include its draft-owned values. */
export interface StorageSetupDraft {
  draft_id: string;
  storage_id: string;
  name: string;
  provider: StorageProviderKey;
  config_version: number;
  provider_label: string;
  available: boolean;
  settings: StorageSettings;
  secrets: StorageDraftSecretState[];
  /** The draft edits a storage that already exists in the configuration. */
  edits_existing: boolean;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface StorageSetupDraftList {
  drafts: StorageSetupDraft[];
  /** Fixed value-free sentences — a draft file may hold a secret. */
  warnings: string[];
}

export interface UpdateStorageSetupDraftRequest {
  draft_id: string;
  revision: number;
  name: string;
  settings: StorageSettings;
  secrets: Record<string, SecretOperation>;
}

export interface TestStorageDraftResult {
  draft_id: string;
  draft_revision: number;
  verified: boolean;
}

export interface CommitStorageDraftResult {
  storage_id: string;
  config_revision: number;
  replaced: boolean;
}

/** Filesystem watcher events contain no credential-bearing config fields. */
export interface ProjectSyncConfigChangedEvent {
  revision: number;
}

/** Value-free signal; draft contents are reloaded through the redacting IPC. */
export type StorageSetupDraftsChangedEvent = Record<string, never>;

/** A storage's runtime connection health changed or was forgotten; the
 *  listing carries the health itself. */
export interface StorageConnectionChangedEvent {
  storage_id: string;
}

/** A checkout or commit moved the watched project's Git refs; Activity
 *  reloads to rebuild its session-to-commit mapping. Names the project only. */
export interface ProjectGitChangedEvent {
  local_project_id: string;
}

export type ResourceSyncState =
  | "synced"
  | "local_only"
  | "local_ahead"
  | "storage_only"
  | "storage_ahead"
  | "diverged"
  | "missing"
  | "blocked"
  | "unknown";

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
  state: ResourceSyncState;
  local_present: boolean;
  storage_present: boolean;
  base_present: boolean;
  local_digest?: string | null;
  storage_digest?: string | null;
  base_digest?: string | null;
  review_digest?: string | null;
  selected_in_recipe: boolean;
  newly_discovered: boolean;
  /** A local file nobody has decided on: not in storage, not in the recipe,
   *  not blocked, not left out at an earlier push. Never ticked for the user. */
  undecided: boolean;
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
  /**
   * Manifest resources as `synced` or `unchanged`, plus one row per selected
   * conversation the Push did not publish: `failed` with its error, or
   * `held` when a Pull resolves it. Those rows are keyed by the review's
   * resource ids, so the comparison the Push was reviewed against supplies
   * their titles.
   */
  results?: Array<{
    resource_id: string;
    state: "synced" | "unchanged" | "held" | "failed";
    message?: string | null;
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
  fetched_at?: number;
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
  /** Another computer's push displaced the publication this computer last
   *  pushed (`saved_by: "push"`) or last pulled (`"pull"`). Nothing is
   *  rolled back: the standing notice asks for a comparison. */
  | { state: "displaced"; base_revision: number; remote_revision: number; saved_by: "push" | "pull" }
  | { state: "unknown"; message: string };

export type PushScope = { kind: "project" } | { kind: "session"; resource_id: string };

/**
 * Composite payload of `prepare_push_review`: one discovery pass and one
 * remote snapshot feed every push-dialog section. Degradable sections carry
 * per-section errors instead of failing the dialog.
 */
export interface PushReviewContext {
  scope: PushScope;
  credential_redaction_enabled: boolean;
  project: ProjectDetail;
  inventory: ResourceInventory;
  destination_status: PushDestinationStatus;
  thread_comparison: ThreadSyncComparison | null;
  thread_comparison_error: string | null;
  project_files: ProjectContentInventory | null;
  project_files_error: string | null;
  git_history: GitHistorySnapshot | null;
  git_history_review_token: string | null;
  git_history_changed: boolean;
  /** Null when storage has no history, or none this branch shares. */
  git_history_outgoing: GitHistoryOutgoing | null;
  prepared_at: number;
}

export interface GitHistorySnapshot {
  branch: string;
  head_sha: string | null;
  commits: { sha: string; committed_at: number; subject: string; session_refs: string[] }[];
  uncommitted_session_refs: string[];
  outside_history_session_refs: string[];
  truncated: boolean;
}

/** What a Push adds to storage's history: the leading `new_commits` of
 *  `git_history.commits`, and whether storage's head left this branch. */
export interface GitHistoryOutgoing {
  new_commits: number;
  rewritten: boolean;
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

/** Pinned at plan time for a diverged project file: the local version is
 * copied to this sibling before the storage version is written. */
export interface ProjectFilePreservationPin {
  copy_logical_path: string;
  local_content_sha256: string;
  storage_content_sha256: string;
  reusable_witness: boolean;
}

export type RestoreActionKind =
  | { kind: "write_file"; logical_path: string }
  | { kind: "write_project_file"; logical_path: string; mode: number; source_mtime: number; preserve?: ProjectFilePreservationPin | null }
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
  /** Size of the storage-side bytes this action would write; absent for deletes and directories. */
  source_size?: number | null;
  requires_explicit_approval: boolean;
}

/**
 * Portable provenance of a divergent keep-both preservation: the origin
 * thread the preserved copy split from and its 1-based per-origin ordinal.
 */
export interface PreservedProvenance {
  preserved_from: string;
  ordinal: number;
}

export interface ConversationReviewRow {
  stored_without_credential_redaction: boolean;
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
  /** The storage-side bytes a pull of this row installs (transcript, sidecars,
   *  child transcripts, index fragment); absent when the action writes no
   *  payload. The review meter sums it with the file actions' source_size. */
  source_size?: number | null;
  /** The stored session's last-message time: the key the recommended
   *  selection walks newest first. Absent when the presentation has none. */
  stored_ended_at?: number | null;
}

/** Pull comparison DTO. Executable conversation actions stay in the backend. */
/** The content a single Pull may change: the project, or one session of it,
 *  named by its stored row's id. Enforced in Rust; never persisted. */
export type PullScope = { kind: "project" } | { kind: "session"; resource_id: string };

export interface PullReview {
  /** The most bytes one Pull Apply installs; the review meter refuses past
   *  it and Apply rechecks the selection against the pinned manifest. */
  apply_limit_bytes: number;
  /** What this review may change. A session's holds its one row, no file
   *  action and no Files step, and its apply moves no reviewed base. */
  scope: PullScope;
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
  conversation_review: ConversationReviewRow[];
  project_content_eligibility?: ProjectFileSyncEligibility | null;
  remote_continuity?: RemoteContinuity | null;
}

export interface PreservedThreadResult {
  action_id: string;
  original_thread_id: string;
  preserved_thread_id: string;
  /** The apply-minted marker title the preserved copy was renamed to. */
  preserved_display_title: string;
}

export interface RestoreResult {
  success: boolean;
  message: string;
  readiness: BundleReadiness | null;
  readiness_error: string | null;
  plan_id?: string | null;
  /**
   * Present only on a project's restore applies. `false` means the reviewed
   * baseline could not advance to the plan's revision because required
   * changes were skipped — Push stays gated until they are pulled. Absent
   * for a session's Pull, which never moves the baseline.
   */
  reviewed_base_advanced?: boolean;
  applied_action_ids: string[];
  failed_actions: Array<{ action_id: string; display_name?: string | null; message: string }>;
  preserved_threads: PreservedThreadResult[];
  /** Provider-declared follow-ups this Pull offers (a desktop app restart,
   *  say), executed through the generic maintenance command. The frontend
   *  renders them; it never checks a provider key. */
  after_pull_actions: AfterPullActionOffer[];
  /** Why the Pull stopped before it reached every ticked item, when it did:
   *  the storage could not be read. What was installed stays, nothing is
   *  retried, and the storage areas are not marked as reviewed. */
  stopped?: string | null;
  /** The ticked items a stopped Pull did not reach. */
  not_pulled_action_ids: string[];
}

export interface BundleReadinessIssue {
  issue_id: string;
  category: "conversations" | "project_files" | "sessions";
  title: string;
  detail?: string | null;
  severity: "warning" | "error";
  provider?: ProviderKey | null;
  resource_id?: string | null;
  action?: string | null;
}

export interface BundleReadiness {
  bundle_id: string;
  state: "ready" | "needs_setup" | "blocked";
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
  /** The last human input, which places the session on its one commit. */
  last_prompt_at?: number | null;
  /** Machine-local provider transcript; absent for storage-only sessions. */
  transcript_path?: string | null;
  working_directory?: string | null;
  /** Machine-local reason this visible session cannot participate in sync. */
  sync_blocked_reason?: string | null;
  recorded_sha?: string | null;
  is_active: boolean;
  user_round_count: number;
  agent_message_count: number;
  tool_call_count: number;
  total_input_tokens?: number | null;
  total_output_tokens?: number | null;
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
  /** Delegations to a subagent, counted apart from tool calls. */
  agent_calls: SessionCount;
}

export type SessionTimelineEventKind =
  | "prompt"
  | "response"
  | "intermediate_step"
  | "checkpoint"
  | "tool_call"
  | "agent_call";

/** How the parent ran the child: waited for it, launched it in the
 * background, or spawned a Codex collaborator thread. */
export type SessionAgentMode = "sync" | "async" | "spawn";

export type SessionAgentStatus =
  | "completed"
  | "running"
  | "failed"
  | "interrupted"
  | "incomplete"
  | "unavailable";

/** One agent row, read from the call, its result, and the child's tail. */
export interface SessionAgentPreview {
  provider_agent_id?: string | null;
  label: string;
  task?: string | null;
  role?: string | null;
  model?: string | null;
  mode: SessionAgentMode;
  status: SessionAgentStatus;
  started_at?: number | null;
  ended_at?: number | null;
  tool_calls?: number | null;
  tokens?: number | null;
  brief?: string | null;
  brief_truncated: boolean;
  report?: string | null;
  report_truncated: boolean;
  /** Opens the child as its own session page, when its transcript exists. */
  transcript_resource_id?: string | null;
}

/** A child page's link back: the parent and the row it shows for this child. */
export interface SessionDetailAgent {
  parent_resource_id: string;
  parent_title: string;
  preview: SessionAgentPreview;
}

export type SessionToolCategory = "file_edit" | "shell" | "read" | "other";

export type SessionToolCallStatus = "completed" | "failed" | "incomplete";

/* How loudly a user-facing notice reads. Mirrors `NoticeSeverity` in
 * mallard-domain; the ladder is success < info < warning < danger, and the
 * frontend prints the rung's name in the notice text so severity never rests
 * on a tint or a glyph. */
export type NoticeSeverity = "success" | "info" | "warning" | "danger";

export interface SessionDetailWarning {
  code: string;
  message: string;
  severity: NoticeSeverity;
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
  actions: SessionActionDescriptor[];
  github_repository_url?: string | null;
  snapshot_id: string;
  warnings: SessionDetailWarning[];
  agent?: SessionDetailAgent | null;
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
  agent?: SessionAgentPreview | null;
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
  | "path_map_codec_mismatch"
  | "observation_failed";

export interface ThreadSyncDecision {
  action: ConversationReviewAction;
  blocker?: ConversationBlocker | null;
  selectable: boolean;
  blocked_reason?: string | null;
}

export type ConversationWireChildren = "on" | "off";

export interface SubagentSelection {
  wire_children: ConversationWireChildren;
  keys: string[];
  omitted: boolean;
  estimated_bytes: number;
}

export interface ThreadSyncEntry {
  subagent_selection: SubagentSelection;
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
   * storage-only row); null for a local-only thread. Comparison provenance is
   * carried by `compared_against_storage`.
   */
  cloud_thread_id?: string | null;
  forked_from?: string | null;
  /** True for rows produced by a storage comparison (sync review). */
  compared_against_storage: boolean;
  provider_thread_id?: string | null;
  /** What a push of this row's local conversation weighs (transcript plus
   *  sidecars); absent for storage-only rows. The review meter sums it. */
  capture_bytes?: number | null;
  /** What a pull of this row's stored conversation installs: every file the
   *  manifest lists for it. Absent for local-only rows and for rows compared
   *  without a storage. The Pull review prints it per row. */
  pull_bytes?: number | null;
  push_decision: ThreadSyncDecision;
  pull_decision: ThreadSyncDecision;
}

/** The aggregate byte limit shown by a Push review. */
export interface CaptureBudget {
  limit_bytes: number;
  /** Bytes the push always carries besides conversations (tools, provider state). */
  reserved_bytes: number;
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
  managed_cloud_status?: ManagedCloudStatus | null;
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
  /** Apply limit and non-conversation bytes reserved within it. */
  capture_budget?: CaptureBudget | null;
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

/** What lies before `window_start`. Absent when the window reaches the end. */
export interface OlderActivity {
  /** Newest thing below the window; the next page ends just past it. */
  newest_at: number;
  /** Commits below the window that are not already drawn on this page. */
  commits: number;
  sessions: number;
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
  older?: OlderActivity | null;
  storage_sync: StorageSyncSummary[];
}

export type DropboxSetupMode = "create" | "existing" | "reconnect";
export interface DropboxSetupLogin {
  session_id: string;
  account: { id: string; name: string; email: string };
}
export interface DropboxSetupProgress {
  status: "authorization_link";
  url: string;
  expires_in: number;
}

/** The Feishu sign-in with Mallard's own app: a new storage takes the
 *  folder the lane creates or the pasted one; a reconnect keeps the
 *  storage's folder and connection. */
export type FeishuSetupMode = "create" | "reconnect";
export interface FeishuSetupFailure {
  code: string;
  evidence?: StorageConnectionEvidence | null;
  provider_code?: number | null;
  pkce?: {
    challenge_present: boolean;
    method_s256: boolean;
    verifier_matches_challenge: boolean;
    callback_state_verified: boolean;
    verifier_length: number;
    challenge_length: number;
  } | null;
}
export interface FeishuSetupLogin {
  session_id: string;
  account: { id: string; name: string };
}
export interface FeishuSetupProgress {
  status: "authorization_link";
  url: string;
  expires_in: number;
}

/** The Mallard Cloud sign-in: a new storage takes whichever GitHub account
 *  signs in; a reconnect keeps the storage's account. */
export type ManagedCloudSetupMode = "create" | "reconnect";
export interface ManagedCloudSetupLogin {
  account: { id: string; login: string };
  service_origin: string;
}
/** A failure after the sign-in itself names the account it is about. */
export interface ManagedCloudSetupFailure {
  code: string;
  account?: { id: string; login: string } | null;
}
export interface ManagedCloudSetupProgress {
  status: "authorization_link";
  url: string;
  expires_in: number;
}

/** How a cloud repository's Git remote compares with the local project's:
 *  the same remote (or both without one), a different one (or one side
 *  without), or withheld by the service. */
export type ManagedCloudGitRemote = "matches" | "differs" | "unknown";

/** One Mallard Cloud repository as the address picker lists or resolves
 *  it. The address is the copy; the bundle id only rides along for the
 *  connect request. */
export interface ManagedCloudRepository {
  repository_id: string;
  owner_login: string;
  name: string;
  /** `{owner_login}/{name}` in the service's spelling. */
  address: string;
  website_url: string;
  bundle_id: string | null;
  repository_class: string;
  visibility: string;
  /** The account's role; null when it is not a collaborator. */
  role: string | null;
  /** Whether Push is accepted: a collaborator, or anyone signed in on a
   *  public repository. */
  can_push: boolean;
  /** The remote's fingerprint when the service lets this account know it. */
  repository_fingerprint: string | null;
  git_remote: ManagedCloudGitRemote;
  /** `https://github.com/{owner}/{name}` once the service has linked it:
   *  the only source a new project is cloned from. */
  github_url: string | null;
  revision: number;
  updated_at: number;
  resources: number;
  registration_only: boolean;
  /** The Owner's line, else GitHub's. */
  description: string | null;
  star_count: number;
  viewer_starred: boolean;
}

/** Where a project got from Mallard Cloud may put its folder. A folder that
 *  holds files carries its own origin — the fingerprint setup's
 *  acknowledgement rule compares, and the remote as host and path — for
 *  connecting it as it is. */
export type CloudProjectFolder =
  | { kind: "refused"; message: string }
  | { kind: "new" }
  | { kind: "empty" }
  | { kind: "checkout" }
  | { kind: "occupied"; suggestion: string; local_fingerprint: string | null; git_remote: string | null }
  | { kind: "project"; project_id: string };

export interface CloudProjectFolderProbe {
  folder: CloudProjectFolder;
  /** Whether Git runs on this computer; gh clones through it too. */
  can_clone: boolean;
}

export interface CloudProjectFolderRequest {
  destination: string;
  is_git_project: boolean;
  github_url: string | null;
  repository_fingerprint: string | null;
  operation_id: string;
}

export type CloneFailureReason = "missing_source" | "refused" | "unreachable" | "failed";

/** Why source code is missing, and a command to fetch it when its URL is known. */
export interface CloneFailure {
  reason: CloneFailureReason;
  command: string | null;
}

export type CloudProjectFolderResult =
  | { kind: "cloned"; canonical_path: string }
  | { kind: "created"; canonical_path: string }
  | { kind: "initialized"; canonical_path: string; failure: CloneFailure }
  | { kind: "existing"; canonical_path: string };

export type CloudProjectFolderProgress =
  | { stage: "cloning"; percent: number | null }
  | { stage: "creating" };

/** What one pasted address resolves to on a Mallard Cloud storage, and
 *  what one entered invitation code joins. A public repository takes
 *  pushes from anyone signed in, so both found kinds can be connected. */
export type ManagedCloudRepositoryResolution =
  | { kind: "collaborator"; repository: ManagedCloudRepository }
  | { kind: "public"; repository: ManagedCloudRepository }
  | { kind: "not_found"; address: string }
  | { kind: "not_a_project"; repository: ManagedCloudRepository }
  | { kind: "wrong_service"; host: string; expected: string }
  | { kind: "github" }
  | { kind: "invalid" };

/** A `mallard://` link once read: the page it names — a repository's, or the
 *  website's listing — or that it is not one. The link's own text never
 *  reaches the window. */
export type AppLinkArrival =
  | { kind: "repository"; host: string; address: string }
  /** One session's page: its repository, and the session a Pull is scoped
   *  to. `resource_id` is the stored row's id. */
  | {
    kind: "session";
    host: string;
    address: string;
    provider: ProviderKey;
    cloud_thread_id: string;
    resource_id: string;
  }
  | {
    kind: "listing";
    host: string;
    /** The kind the listing's chips were filtered to, when one was on. */
    listing_kind: ManagedCloudListingKind | null;
    /** The host is the service a new storage here signs in to. */
    can_set_up: boolean;
  }
  | { kind: "invalid" };

/** The Catalog coordinate and item a repository holds, in the service's words. */
export interface AppLinkCatalogItem {
  provider: string;
  resource_kind: string;
  format: string;
  format_version: number;
  resource_id: string;
}

export interface AppLinkRepository {
  repository_id: string;
  /** `owner/name`, the service's spelling. */
  address: string;
  /** `git_project`, `non_git_project`, or `catalog`. */
  repository_class: string;
  /** A project's bundle; null for a Catalog item. */
  bundle_id: string | null;
  /** A Catalog item's coordinate; null for a project. */
  catalog: AppLinkCatalogItem | null;
  visibility: string;
  /** The account's role; null when it is not a collaborator. */
  role: string | null;
}

/** What one account sees at a link's address. */
export type AppLinkSight =
  | { kind: "found"; repository: AppLinkRepository }
  | { kind: "not_found" }
  | { kind: "failed"; message: string };

/** One Mallard Cloud storage on the link's host, and what it sees. */
export interface AppLinkAccount {
  storage_id: string;
  storage_name: string;
  account_login: string;
  sight: AppLinkSight;
}

export interface AppLinkAnswer {
  host: string;
  address: string;
  /** Empty when no storage signs in to the host: nothing was asked. */
  accounts: AppLinkAccount[];
  /** The host is the service a new storage here signs in to. */
  can_set_up: boolean;
}

/** Results from one local discovery and one remote observation. */
export type ProjectActivityRefreshEvent =
  | { stage: "activity"; activity: ProjectActivity; storage_pending: boolean }
  | { stage: "activity_error"; message: string; storage_pending: boolean }
  | { stage: "comparison"; comparison: ThreadSyncComparison }
  | { stage: "comparison_error"; message: string; managed_cloud_status?: ManagedCloudStatus | null };

/** Best-effort local deletion; failures are independent. */
export interface LocalSessionDeleteResult {
  provider_error: string | null;
  metadata_error: string | null;
}
