import type { IconName } from "../Icons";
import type { ManagedCloudLink, ManagedCloudPublicationStatus, ManagedCloudReleaseState, ManagedCloudStatus } from "../../types";

/** Whether the repository the link saved is gone: removed on the website,
 *  being deleted, or out of this account's reach. It reads as empty, Push and
 *  Pull stay open, and a Push creates it again. */
export function cloudRepositoryGone(status: ManagedCloudStatus | null | undefined): boolean {
  return status?.state === "gone" || status?.state === "deleting";
}

/** Whether another repository of the project is read in place of the one the
 *  link saved: another computer of this account created it again. The status
 *  names the saved one and its link the one that was read, until a Push or a
 *  Pull saves it. The verbs that name a repository by the link wait for that. */
export function cloudPinMoved(status: ManagedCloudStatus | null | undefined): boolean {
  return status?.state === "ready" && !!status.link && status.link.repository_id !== status.repository_id;
}

/** Whether Discard draft is offered: to the Owner, on the repository the link
 *  saved, while it is public and its draft differs from the public version. A
 *  draft with no public version under it is discarded on the website. */
export function canDiscardDraft(status: ManagedCloudStatus | null | undefined): boolean {
  return status?.state === "ready"
    && !cloudPinMoved(status)
    && status.link?.role === "owner"
    && status.link.visibility === "public"
    && status.publication?.state === "unpublished_draft";
}

/** Whether Push and Pull stand down for the repository's sake. A repository
 *  that is gone reads as empty, and a Push creates it again; every other
 *  state that is not ready stops both. */
export function cloudSyncBlocked(status: ManagedCloudStatus | null | undefined): boolean {
  return !!status && status.state !== "ready" && !cloudRepositoryGone(status);
}

/** The verbs the Cloud chip's panel offers on the repository. Opening a page
 *  follows the repository that was read. The release and the discard name the
 *  repository by the link, so they wait while another one is read in its place.
 *  The release request's page is offered to a collaborator once there is a
 *  request to read: waiting, or turned down with a reason. */
export function cloudVerbs(status: ManagedCloudStatus | null | undefined) {
  const link = status?.state === "ready" ? status.link : null;
  const requestRelease = !!link && link.can_request_release && !cloudPinMoved(status);
  const asked = link?.release.kind === "waiting_owner" || link?.release.kind === "waiting_super_admin" || link?.release.kind === "rejected";
  return {
    viewDraft: !!link,
    viewPublished: link?.visibility === "public",
    requestRelease,
    viewRelease: !requestRelease && asked && link?.role != null,
    discardDraft: canDiscardDraft(status),
  };
}

/** The workspace's note for a repository that is not ready, and the one
 *  control it offers. A gone repository offers none: there is nothing to
 *  retry, Push is open, and the note ends with that Push. */
export function cloudStatusNotice(status: ManagedCloudStatus | null | undefined): { title: string; message: string; action: "retry" | "storage_settings" | null } | null {
  if (!status || status.state === "ready") return null;
  if (cloudRepositoryGone(status)) {
    return {
      title: status.state === "deleting" ? "Being deleted" : "Repository gone",
      message: `— ${status.message ?? "Push creates it again."}`,
      action: null,
    };
  }
  return {
    title: status.state === "sign_in_required" ? "Sign-in required" : status.state === "inaccessible" ? "Repository unavailable" : "Status unavailable",
    message: status.message ?? "Could not check Mallard Cloud.",
    action: status.state === "sign_in_required" ? "storage_settings" : "retry",
  };
}

/** What Discard draft asks before it sends anything. */
export function discardDraftQuestion(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null) {
  const kept = publication?.public_revision == null ? "the public version" : `public version ${publication.public_revision}`;
  const requested = link.release.kind === "waiting_owner" || link.release.kind === "waiting_super_admin";
  return {
    kept,
    title: "Discard draft?",
    message: `Discard the draft of “${link.owner_login}/${link.name}”?\n\nRemoves every push after ${kept} from Mallard Cloud. The draft returns to the public version.${requested ? " The open release request is withdrawn." : ""}\n\nNothing is deleted on this computer. This cannot be undone.`,
    okLabel: "Discard",
    cancelLabel: "Cancel",
  };
}

/** The Owner's discard, from the menu row to the refresh: ask, send the one
 *  request with the repository revision the status showed, report a refusal,
 *  compare again. Cancel sends nothing. */
export async function discardCloudDraft(
  status: ManagedCloudStatus | null | undefined,
  actions: {
    confirm: (message: string, options: { title: string; kind: "warning"; okLabel: string; cancelLabel: string }) => Promise<boolean>;
    discard: (expectedRepositoryRevision: number) => Promise<unknown>;
    report: (refusal: string | null) => void;
    refresh: () => void;
  },
): Promise<void> {
  const link = status?.link;
  if (!link || !canDiscardDraft(status)) return;
  const { title, message, okLabel, cancelLabel } = discardDraftQuestion(link, status.publication);
  if (!(await actions.confirm(message, { title, kind: "warning", okLabel, cancelLabel }))) return;
  try {
    await actions.discard(link.repository_revision);
    actions.report(null);
  } catch (reason) {
    actions.report(reason instanceof Error ? reason.message : String(reason));
  }
  actions.refresh();
}

export type CloudTone = "plain" | "success" | "info" | "warning" | "danger";

/** The one state the path bar's Cloud chip shows. */
export interface CloudHeadline {
  /** Precedence: the chip shows the lowest rank that applies. */
  rank: number;
  word: string;
  line: string;
  mark: IconName;
  tone: CloudTone;
}

/** What the Cloud chip says, most urgent first: the account and the
 *  service, then the repository, then the release, then the draft. A status
 *  not read yet is the chip checking; it is inert until the answer lands, so
 *  the bar never shifts under it. */
export function cloudHeadline(status: ManagedCloudStatus | null | undefined): CloudHeadline {
  if (!status) return { rank: 0, word: "Checking…", line: "Checking Mallard Cloud…", mark: "cloud", tone: "plain" };
  switch (status.state) {
    case "sign_in_required":
      return { rank: 1, word: "Sign in", line: status.message ?? "Signed out of Mallard Cloud.", mark: "cloud-user", tone: "warning" };
    case "inaccessible":
      return { rank: 2, word: "No access", line: status.message ?? "Mallard Cloud refused this account.", mark: "cloud-alert", tone: "warning" };
    case "unavailable":
      return { rank: 2, word: "Can't check", line: status.message ?? "Could not check Mallard Cloud.", mark: "cloud-alert", tone: "warning" };
    case "deleting":
      return { rank: 3, word: "Being deleted", line: status.message ?? "Push creates it again.", mark: "cloud-off", tone: "warning" };
    case "gone":
      return { rank: 3, word: "Gone", line: status.message ?? "Push creates it again.", mark: "cloud-off", tone: "warning" };
    case "ready":
      break;
  }
  const link = status.link;
  if (!link) return { rank: 2, word: "Can't check", line: status.message ?? "Could not check Mallard Cloud.", mark: "cloud-alert", tone: "warning" };
  const publication = status.publication;
  switch (link.release.kind) {
    case "rejected":
      return { rank: 4, word: "Rejected", line: link.release.reason ?? "The release request was rejected.", mark: "cloud-x", tone: "danger" };
    case "waiting_owner":
      // `requested_by` is the service's user id, not a login: not for print.
      return { rank: 5, word: "In review", line: "Waiting for the Owner.", mark: "cloud-clock", tone: "info" };
    case "waiting_super_admin":
      return { rank: 5, word: "In review", line: "Waiting for a Mallard Cloud admin.", mark: "cloud-clock", tone: "info" };
    case "not_pushed":
      return { rank: 9, word: "Not pushed", line: "Nothing pushed yet.", mark: "cloud", tone: "plain" };
    case "private":
    case "public":
    case "snapshot_changed":
      break;
  }
  if (!publication || publication.state === "unavailable") {
    return { rank: 2, word: "Can't compare", line: "Couldn't compare the draft with the public version.", mark: "cloud-alert", tone: "warning" };
  }
  if (publication.state === "no_draft") return { rank: 9, word: "Not pushed", line: "Nothing pushed yet.", mark: "cloud", tone: "plain" };
  const draft = publication.private_revision != null ? ` v${publication.private_revision}` : "";
  if (link.visibility === "private") {
    return { rank: 8, word: "Private", line: draft ? `Draft${draft}. Collaborators only.` : "Collaborators only.", mark: "cloud-lock", tone: "plain" };
  }
  const live = publication.public_revision != null ? ` v${publication.public_revision}` : "";
  if (publication.state === "unpublished_draft") {
    return { rank: 6, word: `Draft${draft}`, line: live ? `Newer than public${live}. Not in review.` : "Not in review.", mark: "cloud-dot", tone: "plain" };
  }
  return { rank: 7, word: `Published${live}`, line: "The draft is the public version.", mark: "cloud-check", tone: "success" };
}

/** Where the release request stands, in the panel's words. */
export function cloudReviewWord(state: ManagedCloudReleaseState): string {
  switch (state.kind) {
    case "not_pushed":
      return "—";
    case "private":
    case "snapshot_changed":
      return "Not asked";
    case "waiting_owner":
      return "Owner's turn";
    case "waiting_super_admin":
      return "Admin's turn";
    case "public":
      return "Approved";
    case "rejected":
      return "Rejected";
  }
}

/** Anyone signed in pushes to a public repository; one who is not a
 *  collaborator does so as a contributor, and the Owner decides what is
 *  released. */
export function cloudContributor(link: ManagedCloudLink): boolean {
  return link.role === null && link.visibility === "public" && link.can_push;
}

/** The four facts under the panel's headline. */
export function cloudFacts(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null): Array<{ label: string; value: string }> {
  const role = link.role === null ? "Contributor" : `${link.role.charAt(0).toUpperCase()}${link.role.slice(1)}`;
  return [
    { label: "Draft", value: publication?.private_revision != null ? `v${publication.private_revision}` : "—" },
    { label: "Public", value: link.visibility === "private" ? "Private" : publication?.public_revision != null ? `v${publication.public_revision}` : "—" },
    { label: "Review", value: cloudReviewWord(link.release) },
    { label: "You", value: role },
  ];
}

export function cloudVersionUrl(link: ManagedCloudLink, version: "private" | "public"): string {
  const url = new URL(link.website_url);
  url.searchParams.set("version", version);
  return url.toString();
}

/** The repository's Release tab on the website: where the Owner or an Admin
 *  decides, and where a request's answer is read. */
export function cloudReleaseUrl(link: ManagedCloudLink): string {
  const url = new URL(link.website_url);
  url.searchParams.set("tab", "release");
  return url.toString();
}
