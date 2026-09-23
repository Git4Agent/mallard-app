import type { ThreadSyncComparison } from "../../types";
import type { NoticeItem } from "../notice/Notice";

/**
 * The standing notice of a storage whose history this computer is no longer
 * on: another computer cleared it, or another computer's push displaced the
 * publication this computer last pushed or last pulled.
 *
 * It shows while the comparison reports a missing, a replaced or a displaced
 * history, and it ends by itself once a Push or a Pull has replaced this
 * computer's stale bases. It asks for no unlink and no reset, and it carries
 * no action: Push and Pull are already on the page. Nothing is rolled back
 * for a displaced push: recovery is a user action, and the user compares and
 * pushes again.
 *
 * Push is open when every area this computer writes, the project's files and
 * each connected agent, reads `fresh` or `in_sync`. Otherwise another computer
 * has pushed into one of them since, and a Pull comes first.
 */
export function clearedStorageNotice(
  comparison: ThreadSyncComparison | null | undefined,
  storageName: string,
): NoticeItem | null {
  const continuity = comparison?.remote_continuity;
  if (!comparison || !continuity) return null;
  if (continuity.state === "displaced") {
    const saved = continuity.saved_by === "push" ? "push" : "pull";
    return {
      key: "storage-displaced",
      severity: "warning",
      message: `Another computer pushed to ${storageName} first. This computer's last ${saved} is no longer part of its history. Compare again.`,
    };
  }
  if (continuity.state !== "remote_missing" && continuity.state !== "replaced") return null;
  const pushOpen = comparison.scopes
    .filter((scope) => scope.access === "bound_ready" || scope.access === "bound_unavailable")
    .every((scope) => scope.continuity.state === "fresh" || scope.continuity.state === "in_sync");
  return {
    key: "storage-cleared",
    severity: "warning",
    message: pushOpen
      ? `Another computer cleared ${storageName}. Push again.`
      : `Another computer cleared ${storageName}. Pull, then push again.`,
  };
}
