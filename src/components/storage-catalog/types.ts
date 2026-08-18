/** Scope-rowed Library contracts. These mirror the Rust command DTOs and
 * remain independent from project-sync DTOs. */

export type CatalogProviderKey = "codex" | "claude";
export type CatalogResourceKind =
  | "agent_skill"
  | "plugin_install_intent"
  | "codex_pet_v1"
  | "codex_pet_v2";
export type CatalogPetFormat = "v1" | "v2";
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

export type LocalCatalogTarget =
  | { kind: "skill_directory"; scope: string; canonical_root: string; install_directory: string }
  | { kind: "plugin"; provider: CatalogProviderKey; canonical_home: string; namespace: string; plugin_id: string }
  | { kind: "pet_directory"; canonical_home: string; pet_id: string };

export interface CatalogResourceRegistration {
  registration_id: string;
  provider: CatalogProviderKey;
  profile_id: string;
  resource_kind: CatalogResourceKind;
  resource_id: string;
  local_target: LocalCatalogTarget;
  display_name: string;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface LibraryHeadCoordinate {
  repository_id: string;
  revision: number;
  commit_id: string;
  manifest_sha256: string;
  manifest_size: number;
  storage_identity_sha256: string;
}

export interface CatalogResourceLink {
  link_id: string;
  registration_id: string;
  storage_id: string;
  repository_id: string;
  accepted_head?: LibraryHeadCoordinate | null;
  accepted_content_sha256?: string | null;
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface CatalogResourceLinkView {
  link: CatalogResourceLink;
  storage_name: string;
}

export interface CatalogResourceView {
  registration: CatalogResourceRegistration;
  scope_label?: string | null;
  pet_format?: CatalogPetFormat | null;
  links: CatalogResourceLinkView[];
}

/** One row on a scope page: a resource of this kind compared against the one
 * storage the page is scoped to. Registered, discovered-but-unlinked and
 * stored-but-not-installed resources share this shape, because the page draws
 * them the same way — which one a row is reads off `linked`, `local_present`
 * and `remote_present`. */
export interface CatalogScopeRow {
  row_id: string;
  display_name: string;
  resource_id?: string | null;
  scope_label?: string | null;
  /** Null only when no storage is chosen: nothing to compare against. */
  state?: CatalogLinkState | null;
  linked: boolean;
  local_present: boolean;
  local_path?: string | null;
  local_modified_at?: number | null;
  remote_present: boolean;
  remote_revision?: number | null;
  remote_updated_at?: number | null;
  registration_id?: string | null;
  registration_revision?: number | null;
  link_id?: string | null;
  link_revision?: number | null;
  repository_id?: string | null;
  other_storage_names: string[];
  warnings: CatalogResourceWarning[];
  blocker?: string | null;
}

export interface CatalogScopeResources {
  scope: CatalogScope;
  profile: LocalCatalogProfile;
  storage_id?: string | null;
  storage_name?: string | null;
  rows: CatalogScopeRow[];
  warnings: string[];
}

/** Exactly one of `local_resource_id` and `repository_id` identifies the
 * resource being linked. */
export interface LinkCatalogScopeResourceRequest {
  scope_id: string;
  storage_id: string;
  local_resource_id?: string | null;
  repository_id?: string | null;
}

export interface CatalogScopeLinkOutcome {
  resource: CatalogResourceView;
  completion: "open" | "review_pull";
  link_id: string;
}

export interface CatalogLinkInspection {
  registration_id: string;
  link_id: string;
  storage_id: string;
  repository_id: string;
  state: CatalogLinkState;
  local_present: boolean;
  remote_present: boolean;
  local_content_sha256?: string | null;
  remote_content_sha256?: string | null;
  remote_revision?: number | null;
  blocker?: string | null;
  generated_at: number;
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

export interface CatalogResourcePushPlan {
  schema_version: 1;
  plan_id: string;
  registration_id: string;
  link_id: string;
  storage_id: string;
  repository_id: string;
  row: CatalogPushRow;
  warning_digests: string[];
  created_at: number;
  expires_at: number;
}

export interface CatalogResourcePushOutcome {
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

export interface CatalogResourcePullPlan {
  schema_version: 1;
  plan_id: string;
  registration_id: string;
  link_id: string;
  storage_id: string;
  repository_id: string;
  item: CatalogPullItem;
  conflicting_claims: CatalogClaimPin[];
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

export interface CatalogResourcePullOutcome {
  plan_id: string;
  remote_accepted: boolean;
  result: CatalogActionResult;
}

export interface CatalogResourceForkPlan {
  schema_version: 1;
  plan_id: string;
  registration_id: string;
  source_resource_id: string;
  source_display_name: string;
  copy_name: string;
  copy_resource_id: string;
  copy_target_path: string;
  created_at: number;
  expires_at: number;
}

export interface CatalogResourceStatus {
  registration_id: string;
  link_id?: string | null;
  active_plan_ids: string[];
  recovery_messages: string[];
}
