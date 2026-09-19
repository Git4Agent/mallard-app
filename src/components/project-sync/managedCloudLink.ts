import type { ManagedCloudLink, ManagedCloudPublicationStatus, ManagedCloudReleaseState } from "../../types";

/** The chip the path bar wears for a project on Mallard Cloud: what the
 *  release status says, and how loudly. Waiting states are neutral facts;
 *  a rejection is the one that asks for attention. */
export function releaseChip(state: ManagedCloudReleaseState): { label: string; severity: "plain" | "warning" | "danger"; detail: string } {
  switch (state.kind) {
    case "not_pushed":
      return { label: "Not pushed", severity: "plain", detail: "Registered on Mallard Cloud; nothing has been pushed yet." };
    case "private":
      return { label: "Not requested", severity: "plain", detail: "The draft is not under review." };
    case "waiting_owner":
      return { label: "Pending approval", severity: "plain", detail: "Public release awaits Owner approval." };
    case "waiting_super_admin":
      return { label: "Pending approval", severity: "plain", detail: "Public release awaits Admin approval." };
    case "public":
      return { label: "Published", severity: "plain", detail: "The draft matches the published version." };
    case "snapshot_changed":
      return { label: "Not requested", severity: "plain", detail: "The current draft is not under review." };
    case "rejected":
      return { label: "Rejected", severity: "danger", detail: state.reason ? `The release request was rejected: ${state.reason}` : "The release request was rejected." };
  }
}

export function cloudVersionUrl(link: ManagedCloudLink, version: "private" | "public"): string {
  const url = new URL(link.website_url);
  url.searchParams.set("version", version);
  return url.toString();
}

export function cloudStatusChip(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null) {
  const review = releaseChip(link.release);
  const visibility = link.visibility === "public"
    ? publication?.public_revision != null ? `Public version ${publication.public_revision}` : "Public"
    : "Private";
  const state = publication?.state ?? "unavailable";
  const publicationLabel = {
    no_draft: "No draft yet",
    not_public: "Not public",
    matches_published: "Draft matches published version",
    unpublished_draft: "Unpublished draft",
    unavailable: "Publication status unavailable",
  }[state];
  const labels = [visibility];
  if (state === "unpublished_draft" || state === "unavailable") labels.push(publicationLabel);
  labels.push(review.label);
  const draft = publication?.private_revision != null ? `Draft version ${publication.private_revision}. ` : "";
  return {
    label: labels.join(" · "),
    severity: review.severity === "danger" ? "danger" as const : state === "unavailable" ? "warning" as const : "plain" as const,
    detail: `${visibility}. ${draft}${publicationLabel}. ${review.detail}`,
  };
}
