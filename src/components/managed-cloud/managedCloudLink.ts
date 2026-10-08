import type { ManagedCloudLink, ManagedCloudPublicationStatus, ManagedCloudStatus } from "./types";
import type { IconName } from "../Icons";
import { t } from "../../i18n";

/* The words of a Mallard Cloud repository (docs/PLAN_CLOUD_VERSIONS.md). Push
   moves the draft. Publish freezes the draft as the next version, for the
   team. A release makes one version public. Publish never means public, and
   nothing the chip or its panel prints says "revision". */

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

/** Where the draft stands, when the answer said: null for a status that is
 *  not ready, and for one whose answer carried no publication. */
function cloudStanding(status: ManagedCloudStatus | null | undefined): ManagedCloudPublicationStatus | null {
  const publication = status?.state === "ready" ? status.publication : null;
  return publication && publication.draft !== "unavailable" ? publication : null;
}

/** Whether Discard draft is on: the service says it is open to this account
 *  and this draft (the Owner or an Admin, with something to remove), on any
 *  repository. It names the repository by the link, so it waits while
 *  another one is read in its place. */
export function canDiscardDraft(status: ManagedCloudStatus | null | undefined): boolean {
  return status?.state === "ready"
    && !cloudPinMoved(status)
    && !!status.link?.can_discard_draft
    && cloudStanding(status) !== null;
}

/** Why Discard draft is off, for the row's title, when the status says:
 *  nothing pushed, or the draft is the latest version. */
export function discardDraftHold(status: ManagedCloudStatus | null | undefined): string | null {
  const standing = cloudStanding(status);
  if (!standing || canDiscardDraft(status)) return null;
  if (standing.draft === "none") return t("cloud:hold.nothingPushed");
  if (standing.latest_version !== null && standing.draft_ahead === 0) return t("cloud:hold.nothingToRemove", { version: standing.latest_version });
  return null;
}

/** Whether Publish version is on: the service says it is open to this
 *  account and this draft. Publish names the repository by the link, so it
 *  waits while another one is read in its place. */
export function canPublishVersion(status: ManagedCloudStatus | null | undefined): boolean {
  return status?.state === "ready"
    && !cloudPinMoved(status)
    && !!status.link?.can_publish_version
    && cloudStanding(status) !== null;
}

/** Why Publish version is off, for the row's title, when the status says. */
export function publishVersionHold(status: ManagedCloudStatus | null | undefined): string | null {
  const standing = cloudStanding(status);
  if (!standing || canPublishVersion(status)) return null;
  if (standing.draft === "none") return t("cloud:hold.nothingPushed");
  if (standing.draft_ahead === 0 && standing.latest_version !== null) return t("cloud:hold.nothingNew", { version: standing.latest_version });
  return null;
}

/** Whether Push and Pull stand down for the repository's sake. A repository
 *  that is gone reads as empty, and a Push creates it again; every other
 *  state that is not ready stops both. */
export function cloudSyncBlocked(status: ManagedCloudStatus | null | undefined): boolean {
  return !!status && status.state !== "ready" && !cloudRepositoryGone(status);
}

/** The verbs the Cloud chip's panel offers on the repository, in the panel's
 *  order. Opening a page follows the repository that was read; a release is
 *  asked for on the website's Release tab, so that row opens a page too.
 *  Publish and the discard name the repository by the link, so they wait
 *  while another one is read in its place. The release request's page is
 *  offered to a collaborator once there is a request to read: waiting, or
 *  turned down with a reason. The discard row is the Owner's and an Admin's
 *  in every state, on or off as `canDiscardDraft` says. */
export function cloudVerbs(status: ManagedCloudStatus | null | undefined) {
  const link = status?.state === "ready" ? status.link : null;
  const standing = link ? cloudStanding(status) : null;
  const requestRelease = !!link && link.can_request_release;
  const asked = link?.release.kind === "waiting_owner" || link?.release.kind === "waiting_super_admin" || link?.release.kind === "rejected";
  return {
    /** The number Publish would give: a collaborator's row, on or off as
     *  `canPublishVersion` says. Null when the row is not offered. */
    publishVersion: link?.role != null && standing ? standing.next_version : null,
    requestRelease,
    viewRelease: !requestRelease && asked && link?.role != null,
    viewDraft: !!link,
    /** The website's Versions tab, once there is a version to list. */
    versions: standing?.latest_version != null,
    /** The public version, by its number; null when there is none to open. */
    viewPublic: link?.visibility === "public" ? standing?.public_version ?? null : null,
    discardDraft: link?.role === "owner" || link?.role === "admin",
  };
}

/** The workspace's note for a repository that is not ready, and the one
 *  control it offers. A gone repository offers none: there is nothing to
 *  retry, Push is open, and the note ends with that Push. */
export function cloudStatusNotice(status: ManagedCloudStatus | null | undefined): { title: string; message: string; action: "retry" | "storage_settings" | null } | null {
  if (!status || status.state === "ready") return null;
  if (cloudRepositoryGone(status)) {
    return {
      title: status.state === "deleting" ? t("cloud:notice.beingDeleted") : t("cloud:notice.repositoryGone"),
      message: t("cloud:notice.goneMessage", { message: status.message ?? t("cloud:line.pushCreates") }),
      action: null,
    };
  }
  return {
    title: status.state === "sign_in_required" ? t("cloud:notice.signInRequired")
      : status.state === "inaccessible" ? t("cloud:notice.repositoryUnavailable") : t("cloud:notice.statusUnavailable"),
    message: status.message ?? t("cloud:line.couldNotCheck"),
    action: status.state === "sign_in_required" ? "storage_settings" : "retry",
  };
}

/** What Discard draft asks before it sends anything. The draft returns to
 *  the latest version; a repository without one keeps no push. A release
 *  request that names the draft goes with it; one that names a version stays. */
export function discardDraftQuestion(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null) {
  const version = publication?.latest_version ?? null;
  const waiting = link.release.kind === "waiting_owner" || link.release.kind === "waiting_super_admin";
  const requested = waiting && publication?.review_version == null;
  const removes = version !== null
    ? t(requested ? "cloud:discard.removesAfterWithdrawn" : "cloud:discard.removesAfter", { version })
    : t(requested ? "cloud:discard.removesAllWithdrawn" : "cloud:discard.removesAll");
  return {
    /** The version the draft returns to, in words; null when none is kept. */
    kept: version === null ? null : t("cloud:discard.kept", { version }),
    title: t("cloud:discard.title"),
    message: t("cloud:discard.message", { address: `${link.owner_login}/${link.name}`, removes }),
    okLabel: t("cloud:discard.ok"),
    cancelLabel: t("common:cancel"),
  };
}

/** The discard, from the panel's row to the refresh: ask, send the one
 *  request with the repository revision the status showed, report a refusal,
 *  compare again. Cancel sends nothing; a row that is off sends nothing. */
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
 *  service, then the repository, then the release, then the draft against
 *  the versions. A status not read yet is the chip checking; it is inert
 *  until the answer lands, so the bar never shifts under it. */
export function cloudHeadline(status: ManagedCloudStatus | null | undefined): CloudHeadline {
  if (!status) return { rank: 0, word: t("cloud:word.checking"), line: t("cloud:line.checking"), mark: "cloud", tone: "plain" };
  switch (status.state) {
    case "sign_in_required":
      return { rank: 1, word: t("cloud:word.signIn"), line: status.message ?? t("cloud:line.signedOut"), mark: "cloud-user", tone: "warning" };
    case "inaccessible":
      return { rank: 2, word: t("cloud:word.noAccess"), line: status.message ?? t("cloud:line.refused"), mark: "cloud-alert", tone: "warning" };
    case "unavailable":
      return { rank: 2, word: t("cloud:word.cantCheck"), line: status.message ?? t("cloud:line.couldNotCheck"), mark: "cloud-alert", tone: "warning" };
    case "deleting":
      return { rank: 3, word: t("cloud:word.beingDeleted"), line: status.message ?? t("cloud:line.pushCreates"), mark: "cloud-off", tone: "warning" };
    case "gone":
      return { rank: 3, word: t("cloud:word.gone"), line: status.message ?? t("cloud:line.pushCreates"), mark: "cloud-off", tone: "warning" };
    case "ready":
      break;
  }
  const link = status.link;
  if (!link) return { rank: 2, word: t("cloud:word.cantCheck"), line: status.message ?? t("cloud:line.couldNotCheck"), mark: "cloud-alert", tone: "warning" };
  const standing = cloudStanding(status);
  // What the open request names: a version by its number, else the draft.
  const review = standing?.review_version ?? null;
  switch (link.release.kind) {
    case "rejected":
      return { rank: 4, word: t("cloud:word.rejected"), line: link.release.reason ?? t("cloud:line.rejected"), mark: "cloud-x", tone: "danger" };
    case "waiting_owner": {
      // `requested_by` is the service's user id, not a login: not for print.
      const line = !standing ? t("cloud:line.waitingOwner")
        : review === null ? t("cloud:line.draftWaitingOwner") : t("cloud:line.versionWaitingOwner", { version: review });
      return { rank: 5, word: t("cloud:word.inReview"), line, mark: "cloud-clock", tone: "info" };
    }
    case "waiting_super_admin": {
      const line = !standing ? t("cloud:line.waitingAdmin")
        : review === null ? t("cloud:line.draftWaitingAdmin") : t("cloud:line.versionWaitingAdmin", { version: review });
      return { rank: 5, word: t("cloud:word.inReview"), line, mark: "cloud-clock", tone: "info" };
    }
    case "not_pushed":
      return { rank: 9, word: t("cloud:word.notPushed"), line: t("cloud:line.nothingPushed"), mark: "cloud", tone: "plain" };
    case "private":
    case "public":
    case "snapshot_changed":
      break;
  }
  const cannotCompare: CloudHeadline = { rank: 2, word: t("cloud:word.cantCompare"), line: t("cloud:line.cantCompare"), mark: "cloud-alert", tone: "warning" };
  if (!standing) return cannotCompare;
  if (standing.draft === "none") return { rank: 9, word: t("cloud:word.notPushed"), line: t("cloud:line.nothingPushed"), mark: "cloud", tone: "plain" };
  const latest = standing.latest_version;
  const ahead = standing.draft_ahead ?? 0;
  if (link.visibility === "private") {
    const line = latest === null ? t("cloud:line.draftOnly")
      : ahead > 0 ? t("cloud:line.privateAhead", { count: ahead, version: latest })
        : t("cloud:line.privateVersion", { version: latest });
    return { rank: 8, word: t("cloud:word.private"), line, mark: "cloud-lock", tone: "plain" };
  }
  // A public repository has a version, and the public reads one of them.
  const shown = standing.public_version;
  if (latest === null || shown === null) return cannotCompare;
  if (ahead > 0) {
    const line = latest === shown
      ? t("cloud:line.publicAhead", { count: ahead, version: latest })
      : t("cloud:line.publicAheadOther", { count: ahead, version: latest, shown });
    return { rank: 6, word: t("cloud:word.draft"), line, mark: "cloud-dot", tone: "plain" };
  }
  if (latest !== shown) {
    return { rank: 6, word: t("common:version", { version: latest }), line: t("cloud:line.notPublic", { shown }), mark: "cloud-dot", tone: "plain" };
  }
  return { rank: 7, word: t("cloud:word.publicVersion", { version: shown }), line: t("cloud:line.public", { version: shown }), mark: "cloud-check", tone: "success" };
}

/** The account's role on a repository, in a word. A role this build has no
 *  word for prints as the service names it. */
export function cloudRoleWord(role: string): string {
  switch (role) {
    case "owner": return t("cloud:role.owner");
    case "admin": return t("cloud:role.admin");
    case "member": return t("cloud:role.member");
    case "collaborator": return t("cloud:role.collaborator");
    default: return `${role.charAt(0).toUpperCase()}${role.slice(1)}`;
  }
}

/** Who reads a repository, in a word: private or public, as the service
 *  names it otherwise. */
export function cloudVisibilityWord(visibility: string): string {
  switch (visibility) {
    case "private": return t("cloud:visibility.private");
    case "public": return t("cloud:visibility.public");
    default: return `${visibility.charAt(0).toUpperCase()}${visibility.slice(1)}`;
  }
}

/** Anyone signed in pushes to a public repository; one who is not a
 *  collaborator does so as a contributor, and the Owner decides what is
 *  released. */
export function cloudContributor(link: ManagedCloudLink): boolean {
  return link.role === null && link.visibility === "public" && link.can_push;
}

/** The four facts under the panel's headline: where the draft stands, the
 *  latest version, the version the public reads, and the account's standing.
 *  The headline names a review, so no fact repeats it. */
export function cloudFacts(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null): Array<{ label: string; value: string }> {
  const standing = publication && publication.draft !== "unavailable" ? publication : null;
  const latest = standing?.latest_version ?? null;
  const draft = standing?.draft !== "pushed" ? "—"
    : latest === null ? t("cloud:fact.notPublished")
      : standing.draft_ahead ? t("cloud:fact.ahead", { count: standing.draft_ahead }) : t("cloud:fact.sameAs", { version: latest });
  const role = link.role === null ? t("cloud:role.contributor") : cloudRoleWord(link.role);
  return [
    { label: t("cloud:fact.draft"), value: draft },
    { label: t("cloud:fact.version"), value: latest === null ? "—" : String(latest) },
    { label: t("cloud:fact.public"), value: link.visibility === "private" ? t("cloud:fact.private") : standing?.public_version != null ? String(standing.public_version) : "—" },
    { label: t("cloud:fact.you"), value: role },
  ];
}

/** The repository's page reading one of its copies: the draft, or a version
 *  by its number. A version's address keeps meaning that version. */
export function cloudVersionUrl(link: ManagedCloudLink, version: "draft" | number): string {
  const url = new URL(link.website_url);
  url.searchParams.set("version", String(version));
  return url.toString();
}

/** The repository's Versions tab on the website: every version, newest first. */
export function cloudVersionsUrl(link: ManagedCloudLink): string {
  const url = new URL(link.website_url);
  url.searchParams.set("tab", "versions");
  return url.toString();
}

/** The repository's Release tab on the website: where a release is asked
 *  for, where the Owner or an Admin decides, and where a request's answer is
 *  read. */
export function cloudReleaseUrl(link: ManagedCloudLink): string {
  const url = new URL(link.website_url);
  url.searchParams.set("tab", "release");
  return url.toString();
}
