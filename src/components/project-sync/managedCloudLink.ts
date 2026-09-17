import type { ManagedCloudReleaseState } from "../../types";

/** The chip the path bar wears for a project on Mallard Cloud: what the
 *  release status says, and how loudly. Waiting states are neutral facts;
 *  a rejection is the one that asks for attention. */
export function releaseChip(state: ManagedCloudReleaseState): { label: string; severity: "plain" | "warning" | "danger"; detail: string } {
  switch (state.kind) {
    case "not_pushed":
      return { label: "Not pushed", severity: "plain", detail: "Registered on Mallard Cloud; nothing has been pushed yet." };
    case "private":
      return { label: "Private", severity: "plain", detail: "Only you and your collaborators can read this snapshot." };
    case "waiting_owner":
      return { label: "Pending approval", severity: "plain", detail: "Public release awaits Owner approval." };
    case "waiting_super_admin":
      return { label: "Pending approval", severity: "plain", detail: "Public release awaits Admin approval." };
    case "public":
      return { label: "Public", severity: "plain", detail: "The public version is the current snapshot." };
    case "snapshot_changed":
      return { label: "Snapshot changed", severity: "warning", detail: "A newer private snapshot was pushed since the last release." };
    case "rejected":
      return { label: "Rejected", severity: "danger", detail: state.reason ? `The release request was rejected: ${state.reason}` : "The release request was rejected." };
  }
}
