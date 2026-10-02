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
export interface ManagedCloudPublicationStatus {
  state: "no_draft" | "not_public" | "matches_published" | "unpublished_draft" | "unavailable";
  private_revision: number | null;
  public_revision: number | null;
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
  kept_revision: number | null;
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
  /** The account's membership role; null for a public repository the
   *  account does not belong to, where it still pushes as a contributor. */
  role: string | null;
  /** Whether Push is accepted: a collaborator, or anyone signed in on a
   *  public repository. */
  can_push: boolean;
}
