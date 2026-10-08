import type { ManagedCloudStatus } from "../managed-cloud/types";
/** Scope-rowed Library contracts. These mirror the Rust command DTOs
 * (`storage_catalog/service.rs`, `registry.rs`, `domain.rs`) and remain
 * independent from project-sync DTOs. Providers, kinds, formats, and targets
 * are open keys served beside backend labels: the frontend never holds a
 * fixed provider, kind, or format table. */

/** The four independent Library axes that address one portable resource
 * shape. Structurally any well-formed coordinate parses; only the backend's
 * registry decides whether this build can operate on it. */
export interface CatalogCoordinate {
  provider: string;
  resource_kind: string;
  format: string;
  format_version: number;
}

/** Backend-supplied labels for one coordinate, served flattened beside the
 * raw keys on every DTO that names a coordinate. */
export interface CatalogCoordinateLabels {
  provider_label: string;
  kind_label: string;
  format_label: string;
}

/** How the common workflow discovers and captures resources of a
 * capability: every child directory of each target root, or the provider's
 * own inventory with nothing read from a directory. A protocol shape, never
 * a provider, kind, or format identity. */
export type CatalogCaptureSupport = "directory_tree" | "provider_inventory";

export type CatalogApplySupport = "unsupported" | "manual" | "automatic";

/** Icon and label hints the frontend renders without holding its own
 * provider, kind, or format tables. `icon` is a hint (`skill`, `pet`) that
 * `catalogPresentation` in model.ts resolves to a glyph. */
export interface CatalogPresentation {
  icon: string;
  kind_label: string;
  format_label: string;
}

/** One registered capability as `list_catalog_capabilities` serves it, in
 * the backend's display order: provider rank first, then registration. */
export interface CatalogCapabilityDescriptor extends CatalogCoordinate {
  provider_label: string;
  label: string;
  singular_label: string;
  presentation: CatalogPresentation;
  capture: CatalogCaptureSupport;
  apply: CatalogApplySupport;
  targets: string[];
}

/** `CatalogProfileDto`: one Library profile plus its provider's label. */
export interface CatalogProfile {
  profile_id: string;
  provider: string;
  provider_label: string;
  display_name: string;
  configured_home: string;
  revision: number;
  created_at: number;
  updated_at: number;
}

/** `CatalogProfileProbeDto`: what a typed home would become, answered
 * without writing — the project's probe shape in the Catalog's words. */
export interface CatalogProfileProbe {
  canonical_path: string;
  readable: boolean;
  /** The saved profile this folder already is, when it is one. */
  existing_profile_id?: string | null;
  /** The name Set up would give it: the saved one, else its folder's. */
  suggested_name: string;
}

/** `CatalogProfileHoldingDto`: what one saved home holds of one capability
 * its provider registers, and the row that already lists it. */
export interface CatalogProfileHolding {
  capability: CatalogCapabilityDescriptor;
  /** Null when this capability's inventory could not run for the home. */
  count?: number | null;
  newest_modified_at?: number | null;
  /** The target roots as the adapter names them — summaries, never paths. */
  roots: string[];
  scope_id?: string | null;
}

/** `CatalogProfileInspectionDto`: one saved home read for its setup page. */
export interface CatalogProfileInspection {
  profile: CatalogProfile;
  /** The storage a Catalog row used last, when one did. */
  last_used_storage_id?: string | null;
  /** In the registry's order. */
  holdings: CatalogProfileHolding[];
}

/** One Catalog menu row: every resource of one coordinate under one provider
 * profile. */
export interface CatalogScope extends CatalogCoordinate {
  scope_id: string;
  profile_id: string;
  revision: number;
  created_at: number;
  updated_at: number;
}

/** `CatalogScopeDto`: a menu row plus the profile its label derives from and
 * the registered capability's labels. A row whose coordinate this build does
 * not register is listed as unavailable: it keeps its raw coordinate, names
 * itself by it, and cannot be operated on. */
export interface CatalogScopeDto extends CatalogCoordinateLabels {
  scope: CatalogScope;
  profile: CatalogProfile;
  /** The registered capability's label, or the raw coordinate. */
  label: string;
  available: boolean;
  unavailable_reason?: string;
}

/** The frontend's one join over two backend lists: a scope row and the
 * capability descriptor its coordinate registers, `null` when this build
 * registers none. The descriptor is where the presentation lives; the row
 * DTO carries labels and availability only. Built by `catalogScopeViews`. */
export interface CatalogScopeView extends CatalogScopeDto {
  capability: CatalogCapabilityDescriptor | null;
}

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

/** One row on a scope page: a resource of this coordinate compared against
 * the one storage the page is scoped to. Local-only, stored-only and
 * both-sides resources share this shape, because the page draws them the
 * same way — which one a row is reads off `local_present` and
 * `remote_present`. */
export interface CatalogScopeRow {
  row_id: string;
  display_name: string;
  resource_id?: string | null;
  /** The codec's secondary label for this resource (a root class, a format
   * version), when the coordinate does not say it all. */
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
  /** A small PNG preview as a data URI, derived from this Mac's capture by
   * the format's codec. Null when the format has no preview or nothing local
   * can be drawn — stored-only rows fetch theirs lazily through
   * `getPreview`. */
  preview_png?: string | null;
  /** The codec's one-line description of the resource, when the format has
   * one. */
  description?: string | null;
  warnings: CatalogResourceWarning[];
  blocker?: string | null;
}

/** The page-level scope link, when this scope is linked to the page's
 * storage. One link per (scope, storage) — resources never link alone. */
export interface CatalogScopePageLink {
  link_id: string;
  revision: number;
}

/** `CatalogScopeResourcesDto`: the complete content of one scope page. */
export interface CatalogScopeResources extends CatalogCoordinateLabels {
  scope: CatalogScope;
  profile: CatalogProfile;
  storage_id?: string | null;
  storage_name?: string | null;
  link?: CatalogScopePageLink | null;
  /** The stored repository's current revision, when one exists. */
  storage_revision?: number | null;
  rows: CatalogScopeRow[];
  warnings: string[];
  /** Startup-recovery messages from the operations journal. */
  recovery: string[];
  /** On Mallard Cloud: each item's repository, saved or learned from the
   *  account's listing. */
  cloud_repositories: Record<string, string>;
  /** The repositories this row has saved; the rest were learned. */
  cloud_connections: Record<string, string>;
  /** On Mallard Cloud, when the page's request was refused for the
   *  account's sake: the rows list only this Mac, and nothing is compared. */
  cloud_status?: ManagedCloudStatus | null;
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

/** One mutable repository head as the plan observed it. */
export interface LibraryHeadCoordinate {
  repository_id: string;
  revision: number;
  commit_id: string;
  manifest_sha256: string;
  manifest_size: number;
  storage_identity_sha256: string;
}

export interface CatalogPushRow extends CatalogCoordinate, CatalogCoordinateLabels {
  resource_id: string;
  display_name: string;
  scope_label?: string | null;
  relation: CatalogResourceRelation;
  selectable: boolean;
  selected_by_default: boolean;
  held_for_pull: boolean;
  local_content_sha256?: string | null;
  storage_content_sha256?: string | null;
  blocked_reason?: string | null;
  warnings: CatalogResourceWarning[];
  detail?: string | null;
}

/** One row of the scope Push review. The captured resource the backend
 * re-verifies against stays in its in-memory plan store and never crosses
 * IPC; the review renders `row`. */
export interface CatalogScopePushRow {
  row: CatalogPushRow;
}

export interface CatalogScopePushPlan {
  plan_id: string;
  scope_id: string;
  link_id: string;
  link_revision: number;
  storage_id: string;
  repository_id: string;
  storage_identity_sha256: string;
  /** Absent when the repository had no head yet. */
  observed_head?: LibraryHeadCoordinate;
  /** On Mallard Cloud, which holds one item per repository: each stored item's head as the review observed it. */
  observed_items?: Record<string, LibraryHeadCoordinate>;
  /** On Mallard Cloud: where each pushable item goes, new repositories
   *  included. */
  destinations?: Record<string, CatalogPushDestination>;
  rows: CatalogScopePushRow[];
  warning_digests: string[];
  created_at: number;
  expires_at: number;
}

/** Where one item's Pull reads on Mallard Cloud. */
export interface CatalogPullSource {
  repository_id: string;
  /** `owner/name`. */
  address: string;
  /** The draft for a collaborator; the public version for anyone else. */
  reads: "draft" | "public";
  /** The version the review read. */
  revision: number;
}

/** Where one item's Push goes on Mallard Cloud. */
export interface CatalogPushDestination {
  /** Null when the Push creates the repository. */
  repository_id: string | null;
  /** `owner/name`; for a new repository, the name it is expected to get. */
  address: string;
  create: boolean;
}

export interface CatalogScopePushOutcome {
  plan_id: string;
  published: boolean;
  results: CatalogPushResult[];
  stopped: string | null;
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

/** The fixed protocol outcomes an executable Pull action can take. They
 * describe the common workflow, never a provider, kind, or format. */
export type CatalogPullActionKind =
  | { kind: "install" }
  | { kind: "update" }
  | { kind: "preserve_as_copy_and_overwrite"; copy: CatalogPreservationPin }
  | { kind: "overwrite_with_storage" };

/** What the plan expects to find at the local target before it applies. */
export type CatalogTargetExpectation =
  | { state: "absent" }
  | { state: "sha256"; sha256: string };

export interface CatalogPullAction {
  action_id: string;
  resource_id: string;
  kind: CatalogPullActionKind;
  requires_destructive_approval: boolean;
  expected_target_state: CatalogTargetExpectation;
  incoming_content_sha256: string;
}

export type CatalogPullDisposition =
  | { kind: "executable" }
  | { kind: "already_matching" }
  | { kind: "kept_local" }
  | { kind: "manual"; message: string }
  | { kind: "blocked"; reason: string };

export interface CatalogPullItem extends CatalogCoordinate, CatalogCoordinateLabels {
  resource_id: string;
  display_name: string;
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
 * cross-storage claim pins. The stored resource apply materializes stays in
 * the backend's in-memory plan store and never crosses IPC. */
export interface CatalogScopePullItem {
  item: CatalogPullItem;
  conflicting_claims: CatalogClaimPin[];
}

export interface CatalogScopePullPlan {
  plan_id: string;
  scope_id: string;
  link_id: string;
  link_revision: number;
  storage_id: string;
  repository_id: string;
  storage_identity_sha256: string;
  /** The reviewed version of the coordinate repository; absent on Mallard Cloud, where `item_heads` carries each item's. */
  head?: LibraryHeadCoordinate;
  item_heads?: Record<string, LibraryHeadCoordinate>;
  /** On Mallard Cloud: the repository each item reads, and which version. */
  sources?: Record<string, CatalogPullSource>;
  items: CatalogScopePullItem[];
  created_at: number;
  expires_at: number;
}

export type CatalogActionOutcome =
  | "removed"
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
  /** English. A fixed message is translated by `message_code`, with the
   *  name it carries (a fork's source, the kept copy) in `message_name`. */
  message?: string | null;
  message_code?: "skipped" | "forked" | "kept_as" | null;
  message_name?: string | null;
}

export interface CatalogScopePullOutcome {
  plan_id: string;
  /** Whether the whole review advanced the accepted head — false when any
   * row failed, keeping the comparison base honest. */
  remote_accepted: boolean;
  results: CatalogActionResult[];
  /** Why the Pull stopped at a payload it could not read, when it did. The
   *  rows it did not reach read "Not pulled". */
  stopped?: string | null;
}

export interface CatalogResourceForkPlan {
  plan_id: string;
  scope_id: string;
  source_coordinate: CatalogCoordinate;
  source_resource_id: string;
  source_display_name: string;
  source_content_sha256: string;
  copy_name: string;
  copy_resource_id: string;
  copy_target_path: string;
  transformed_content_sha256: string;
  created_at: number;
  expires_at: number;
}

/** `CatalogStorageRepositoryDto`: one Library repository as the storage page
 * lists it — the coordinate it derives from with its labels, the head facts
 * when the repository reads, and the refusal when it does not. Absent
 * repositories are simply not listed. */
export interface CatalogStorageRepository extends CatalogCoordinate, CatalogCoordinateLabels {
  repository_id: string;
  revision: number | null;
  updated_at: number | null;
  /** Display names of everything the set holds, sorted for the fold. */
  resource_names: string[];
  /** Why the repository can't be read; the row states it instead of chips. */
  error: string | null;
}

/** A repository row joined to the capability its coordinate registers, the
 * same join `CatalogScopeView` is. Built by `catalogStorageRepositoryViews`. */
export interface CatalogStorageRepositoryView extends CatalogStorageRepository {
  capability: CatalogCapabilityDescriptor | null;
}

export interface CatalogPushResult {
  resource_id: string;
  display_name: string;
  state: "published" | "unchanged" | "failed" | "not_attempted";
  message: string | null;
  /** On Mallard Cloud: the version this item's repository holds after the
   *  Push. */
  revision?: number | null;
}
export interface CatalogSyncProgress {
  plan_id: string;
  completed: number;
  total: number;
  /** English; translated by `stage`, with `name` for `pushing`/`pulling`. */
  message: string;
  stage?: "publishing" | "pushing" | "push_finished" | "pulling";
  name?: string | null;
}
/** One Mallard Cloud repository of a Catalog item, as Connect and Add list
 *  or resolve it. */
export interface CatalogCloudRepository extends CatalogCoordinate, CatalogCoordinateLabels {
  repository_id: string;
  owner_login: string;
  name: string;
  /** `owner/name`, the service's spelling. */
  address: string;
  website_url: string;
  visibility: "private" | "public";
  /** Null for a public repository the account does not belong to. */
  role: string | null;
  can_push: boolean;
  /** The draft's version for a collaborator, the public one otherwise. */
  revision: number;
  updated_at: number;
  /** Created, nothing pushed yet. */
  registration_only: boolean;
  /** The Owner's line, else GitHub's. */
  description: string | null;
  star_count: number;
  viewer_starred: boolean;
  resource_id: string;
  display_name: string;
  /** Whether this build registers the coordinate. */
  available: boolean;
}

/** Where a repository is saved on this computer: the row that connected or
 *  added it, and the item it is there. */
export interface CatalogCloudPlace {
  scope_id: string;
  storage_id: string;
  resource_id: string;
}

/** What one typed address or invitation code answered. Whether the item
 *  fits is the chooser's to say. */
export type CatalogCloudResolution =
  | { kind: "found"; repository: CatalogCloudRepository }
  | { kind: "not_found"; address: string }
  | { kind: "not_catalog"; address: string }
  | { kind: "wrong_service"; host: string; expected: string }
  | { kind: "github" }
  | { kind: "invalid" };

/** One payload file as an item's page lists it. */
export interface CatalogDetailFile {
  path: string;
  size: number;
  executable: boolean;
}

/** `CatalogResourceFile`: one payload file as an item's page opens it, read
 *  from the same copy as the page and checked against its digest. */
export interface CatalogResourceFile {
  path: string;
  size: number;
  executable: boolean;
  /** Absent when the bytes are not text: invalid UTF-8, or a NUL byte. */
  text: string | null;
  /** This Mac's copy of the file, for Reveal in Finder; absent for the storage's. */
  local_path: string | null;
}

/** The display words an installed plugin's own manifest gives it. */
export interface CatalogDetailPluginListing {
  display_name: string | null;
  developer: string | null;
  description: string | null;
  category: string | null;
  website_url: string | null;
  privacy_url: string | null;
  terms_url: string | null;
}

/** `CatalogResourceDetail`: an item's page, by the shape its format presents
 *  — read from this Mac's copy when it is here, else from the storage's. */
export type CatalogResourceDetail =
  | {
    kind: "document";
    path: string;
    /** Absent when the document is not UTF-8. */
    text: string | null;
    files: CatalogDetailFile[];
  }
  | {
    kind: "install";
    plugin_id: string;
    marketplace: string;
    location: string | null;
    git_ref: string | null;
    /** The provider CLI's own lines that install it by hand — the argv a
     *  Pull runs — or none when they cannot be printed safely. */
    install: string[];
    /** This Mac's: present only when the plugin is installed here. */
    installed_version: string | null;
    listing: CatalogDetailPluginListing | null;
  }
  | {
    kind: "sprite";
    /** The whole atlas as a data URI. */
    atlas: string;
    columns: number;
    rows: number;
    description: string | null;
  };

export interface CatalogResourceRemovalPlan {
  plan_id: string;
  scope_id: string;
  resource_id: string;
  display_name: string;
  uninstall: boolean;
  file_count: number;
  created_at: number;
  expires_at: number;
}

export interface CatalogRemovalResult {
  removed: boolean;
  /** English. A finished removal is translated by `removed`, with
   *  `display_name`; a failed one's message is an error. */
  message: string;
  display_name?: string;
}
