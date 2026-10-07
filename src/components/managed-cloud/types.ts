/** Where a repository's Mallard Cloud release stands (section 4.3 of the
 *  architecture): read from the service when the project opens. */
export type ManagedCloudReleaseState =
  | { kind: "not_pushed" }
  | { kind: "private" }
  | { kind: "waiting_owner"; requested_by: string }
  | { kind: "waiting_super_admin" }
  | { kind: "public" }
  | { kind: "snapshot_changed" }
  | { kind: "rejected"; reason?: string | null };
/** Where the draft stands against the repository's versions
 *  (docs/PLAN_CLOUD_VERSIONS.md), read beside the link. Push moves the
 *  draft; Publish freezes it as the next version; a release makes one
 *  version public. */
export interface ManagedCloudPublicationStatus {
  /** `unavailable`: the answer carried no status. Never the service's word. */
  draft: "none" | "pushed" | "unavailable";
  /** The highest available version; null when there is none. */
  latest_version: number | null;
  /** The number Publish would give. 0 with `unavailable`. */
  next_version: number;
  /** How many pushes the draft is past `latest_version`; 0 when the draft is
   *  that version. Null with no version or no draft. */
  draft_ahead: number | null;
  /** The version the public reads while the repository is public. */
  public_version: number | null;
  /** The version the open release request names; null when none is open, and
   *  when the open one names the draft. */
  review_version: number | null;
}
export interface ManagedCloudStatus {
  /** The repository the link saved. While it is gone and another one of the
   *  same resource is read in its place, `link.repository_id` names that one. */
  repository_id: string;
  /** `deleting` and `gone`: the saved repository was removed, or this account
   *  lost access to it. It reads as empty, and a Push creates it again. */
  state: "ready" | "deleting" | "gone" | "inaccessible" | "sign_in_required" | "unavailable";
  link: ManagedCloudLink | null;
  publication?: ManagedCloudPublicationStatus | null;
  message: string | null;
}
/** What a discard of the draft answered. */
export interface ManagedCloudDiscardedDraft {
  repository_id: string;
  removal_id: string;
  state: "done" | "deleting";
  /** The version the draft returned to; null when no push is kept. */
  kept_version: number | null;
}
/** What Publish answered: the version the draft was frozen as. */
export interface ManagedCloudPublishedVersion {
  repository_id: string;
  version: number;
}
export interface ManagedCloudLink {
  visibility: "private" | "public";
  can_pull: boolean;
  repository_id: string;
  /** `{origin}/{owner_login}/{name}`: the repository's page on the website. */
  website_url: string;
  owner_login: string;
  name: string;
  release: ManagedCloudReleaseState;
  repository_revision: number;
  can_request_release: boolean;
  /** Whether Publish is open to this account now: a collaborator, with a
   *  draft that holds something and is not already a version. */
  can_publish_version: boolean;
  /** Whether Discard draft is open to this account now: the Owner or an
   *  Admin, with a draft that holds something and something to remove (no
   *  version yet, or the draft past the latest one). */
  can_discard_draft: boolean;
  /** The account's membership role; null for a public repository the
   *  account does not belong to, where it still pushes as a contributor. */
  role: string | null;
  /** Whether Push is accepted: a collaborator, or anyone signed in on a
   *  public repository. */
  can_push: boolean;
}

/** The coordinate a listed Catalog repository is bound to. */
export interface ManagedCloudListingCatalog {
  provider: string;
  resource_kind: string;
  format: string;
  format_version: number;
}

/** One row of a listing the website shows too — Starred, or Public: less
 *  than a repository, enough for a row. A pick resolves its address. */
export interface ManagedCloudListingRow {
  repository_id: string;
  owner_login: string;
  name: string;
  /** `{owner_login}/{name}` in the service's spelling. */
  address: string;
  website_url: string;
  repository_class: string;
  catalog: ManagedCloudListingCatalog | null;
  visibility: string;
  /** `member` when the account belongs to it; null on a public face. */
  role: string | null;
  github_url: string | null;
  description: string | null;
  star_count: number;
  viewer_starred: boolean;
  revision: number;
  updated_at: number;
}

/** The kinds the website's listing filters by, in its own `?type=` words:
 *  what a link from the listing carries when one of its chips was on. */
export type ManagedCloudListingKind = "project" | "agent_skill" | "plugin_install_intent" | "desktop_pet";

/** One page of a listing: its rows, and the cursor of the next page. */
export interface ManagedCloudListingPage {
  rows: ManagedCloudListingRow[];
  next_cursor: string | null;
}

export type ManagedCloudPublicSort = "newest" | "stars";
