/** Scope-rowed Library contracts. These mirror the Rust command DTOs and
 * remain independent from project-sync DTOs. */

export type CatalogProviderKey = "codex" | "claude";
export type CatalogResourceKind =
  | "agent_skill"
  | "plugin_install_intent"
  | "codex_pet_v1"
  | "codex_pet_v2";
export type CatalogLinkState =
  | "new"
  | "in_sync"
  | "local_changed"
  | "storage_changed"
  | "diverged"
  | "local_missing"
  | "remote_missing"
  | "unreviewed_remote"
  | "blocked";
export type CatalogResourceRelation =
  | "local_only"
  | "storage_only"
  | "matching"
  | "local_changed"
  | "storage_changed"
  | "changed_both"
  | "unavailable_locally"
  | "blocked";

export interface CatalogCapability {
  provider: CatalogProviderKey;
  kinds: CatalogResourceKind[];
}

export interface LocalCatalogProfile {
  profile_id: string;
  provider_key: CatalogProviderKey;
  display_name: string;
  configured_home: string;
  revision: number;
  created_at: number;
  updated_at: number;
}

/** One Catalog menu row: every resource of one kind under one provider
 * profile. Its label always derives from the kind plus the profile name. */
export interface CatalogScope {
  scope_id: string;
  provider: CatalogProviderKey;
  profile_id: string;
  resource_kind: CatalogResourceKind;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface CatalogScopeView {
  scope: CatalogScope;
  profile: LocalCatalogProfile;
}

/** One row on a scope page: a resource of this kind compared against the one
 * storage the page is scoped to. Local-only, stored-only and both-sides
 * resources share this shape, because the page draws them the same way —
 * which one a row is reads off `local_present` and `remote_present`. */
export interface CatalogScopeRow {
  row_id: string;
  display_name: string;
  resource_id?: string | null;
  scope_label?: string | null;
  /** Null only when no storage is chosen: nothing to compare against. */
  state?: CatalogLinkState | null;
  local_present: boolean;
  local_path?: string | null;
  local_modified_at?: number | null;
  /** Portable payload facts from this Mac's capture. Null when nothing local
   * was captured — stored-only rows, and rows whose capture is blocked. */
  local_files?: number | null;
  local_bytes?: number | null;
  remote_present: boolean;
  /** When the stored copy last changed, from its manifest entry. */
  remote_updated_at?: number | null;
  /** The digest the comparison used: the local capture's when one exists,
   * otherwise the stored version's. */
  content_sha256?: string | null;
  /** A small PNG of the pet's idle frame, as a data URI. Pet rows only;
   * null when nothing local can be drawn — stored-only rows fetch theirs
   * lazily through `getPetPreview`. */
  preview_png?: string | null;
  /** The pet descriptor's own sentence about itself. Pet rows only. */
  pet_description?: string | null;
  warnings: CatalogResourceWarning[];
  blocker?: string | null;
}

/** The page-level scope link, when this scope is linked to the page's
 * storage. One link per (scope, storage) — resources never link alone. */
export interface CatalogScopePageLink {
  link_id: string;
  revision: number;
}

export interface CatalogScopeResources {
  scope: CatalogScope;
  profile: LocalCatalogProfile;
  storage_id?: string | null;
  storage_name?: string | null;
  link?: CatalogScopePageLink | null;
  /** The stored kind repository's current revision, when one exists. */
  storage_revision?: number | null;
  rows: CatalogScopeRow[];
  warnings: string[];
  /** Startup-recovery messages from the operations journal. */
  recovery: string[];
}

export interface CatalogScopeLinkOutcome {
  link_id: string;
  revision: number;
  completion: "open" | "review_pull";
}

export interface CatalogResourceWarning {
  code: string;
  message: string;
  digest: string;
}

export interface CatalogPushRow {
  resource_id: string;
  kind: CatalogResourceKind;
  display_name: string;
  provider: CatalogProviderKey;
  scope_label?: string | null;
  relation: CatalogResourceRelation;
  selectable: boolean;
  selected_by_default: boolean;
  held_for_pull: boolean;
  removal_candidate: boolean;
  local_content_sha256?: string | null;
  storage_content_sha256?: string | null;
  blocked_reason?: string | null;
  warnings: CatalogResourceWarning[];
  detail?: string | null;
}

/** One row of the scope Push review. The captured resource rides along for
 * the backend's re-verification; the review reads only `row`. */
export interface CatalogScopePushRow {
  row: CatalogPushRow;
  resource?: unknown;
}

export interface CatalogScopePushPlan {
  schema_version: 1;
  plan_id: string;
  scope_id: string;
  link_id: string;
  link_revision: number;
  storage_id: string;
  repository_id: string;
  rows: CatalogScopePushRow[];
  warning_digests: string[];
  created_at: number;
  expires_at: number;
}

export interface CatalogScopePushOutcome {
  plan_id: string;
  published: boolean;
  revision?: number | null;
  message: string;
}

export interface CatalogPreservationPin {
  copy_name: string;
  copy_resource_id: string;
  copy_target_path: string;
  transformed_content_sha256: string;
  original_content_sha256: string;
  storage_content_sha256: string;
  reusable_witness: boolean;
}

export type CatalogPullActionKind =
  | { kind: "install_skill" }
  | { kind: "update_skill" }
  | { kind: "preserve_skill_as_copy_and_overwrite"; copy: CatalogPreservationPin }
  | { kind: "overwrite_skill_with_storage" }
  | { kind: "install_plugin"; plan: unknown }
  | { kind: "update_plugin"; plan: unknown }
  | { kind: "overwrite_plugin_with_storage"; plan: unknown }
  | { kind: "install_pet" }
  | { kind: "update_pet" }
  | { kind: "preserve_pet_as_copy_and_overwrite"; copy: CatalogPreservationPin }
  | { kind: "overwrite_pet_with_storage" };

export interface CatalogPullAction {
  action_id: string;
  resource_id: string;
  kind: CatalogPullActionKind;
  requires_destructive_approval: boolean;
  incoming_content_sha256: string;
}

export type CatalogPullDisposition =
  | { kind: "executable" }
  | { kind: "already_matching" }
  | { kind: "kept_local" }
  | { kind: "manual"; message: string }
  | { kind: "blocked"; reason: string };

export interface CatalogPullItem {
  resource_id: string;
  kind: CatalogResourceKind;
  display_name: string;
  provider: CatalogProviderKey;
  scope_label?: string | null;
  relation: CatalogResourceRelation;
  disposition: CatalogPullDisposition;
  actions: CatalogPullAction[];
  default_action_id?: string | null;
  detail?: string | null;
}

export interface CatalogClaimPin {
  storage_id: string;
  content_sha256: string;
}

/** One item of the scope Pull review: the presentation item plus its
 * cross-storage claim pins. The stored resource rides along for apply. */
export interface CatalogScopePullItem {
  item: CatalogPullItem;
  resource?: unknown;
  conflicting_claims: CatalogClaimPin[];
}

export interface CatalogScopePullPlan {
  schema_version: 1;
  plan_id: string;
  scope_id: string;
  link_id: string;
  link_revision: number;
  storage_id: string;
  repository_id: string;
  items: CatalogScopePullItem[];
  created_at: number;
  expires_at: number;
}

export type CatalogActionOutcome =
  | "installed"
  | "updated"
  | "preserved_and_updated"
  | "forked"
  | "already_matching"
  | "kept_local"
  | "skipped"
  | "manual"
  | "partial"
  | "failed";

export interface CatalogActionResult {
  resource_id: string;
  action_id?: string | null;
  display_name: string;
  outcome: CatalogActionOutcome;
  message?: string | null;
}

export interface CatalogScopePullOutcome {
  plan_id: string;
  /** Whether the whole review advanced the accepted head — false when any
   * row failed, keeping the comparison base honest. */
  remote_accepted: boolean;
  results: CatalogActionResult[];
}

export interface CatalogResourceForkPlan {
  schema_version: 1;
  plan_id: string;
  scope_id: string;
  source_resource_id: string;
  source_display_name: string;
  copy_name: string;
  copy_resource_id: string;
  copy_target_path: string;
  created_at: number;
  expires_at: number;
}

/** One Catalog kind repository as the storage page lists it: the derived
 * identity plus its head facts, or — when something answers under the id but
 * cannot be read — the refusal. Absent repositories are simply not listed. */
export interface CatalogStorageRepository {
  repository_id: string;
  provider: CatalogProviderKey;
  resource_kind: CatalogResourceKind;
  revision: number | null;
  updated_at: number | null;
  /** Display names of everything the set holds, sorted for the fold. */
  resource_names: string[];
  /** Why the repository can't be read; the row states it instead of chips. */
  error: string | null;
}
