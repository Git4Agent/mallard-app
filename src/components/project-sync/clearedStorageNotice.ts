import type { ThreadSyncComparison } from "../../types";
import type { NoticeItem } from "../notice/Notice";

/**
 * The standing notice of a storage whose history another computer cleared.
 *
 * It shows while the comparison reports a missing or a replaced history, and
 * it ends by itself once a Push or a Pull has replaced this computer's stale
 * bases. It asks for no unlink and no reset, and it carries no action: Push
 * and Pull are already on the page.
 *
 * Push is open when every area this computer writes, the project's files and
 * each connected agent, reads `fresh` or `in_sync`. Otherwise another computer
 * has pushed into one of them since, and a Pull comes first.
 */
export function clearedStorageNotice(
  comparison: ThreadSyncComparison | null | undefined,
  storageName: string,
): NoticeItem | null {
  const state = comparison?.remote_continuity?.state;
  if (!comparison || (state !== "remote_missing" && state !== "replaced")) return null;
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
