import type { ThreadSyncComparison } from "../../types";
import type { NoticeItem } from "../notice/Notice";
import { cloudPinMoved } from "../managed-cloud/managedCloudLink";
import { t } from "../../i18n";

/**
 * The standing notice of a storage whose history this computer is no longer
 * on: another computer cleared it, or another computer's push displaced the
 * publication this computer last pushed or last pulled. On Mallard Cloud a
 * history moves for reasons of its own, and the notice names them: the
 * repository was deleted and created again, or its draft was discarded.
 *
 * It shows while the comparison reports a missing, a replaced or a displaced
 * history, and it ends by itself once a Push or a Pull has replaced this
 * computer's stale bases. It asks for no unlink and no reset, and it carries
 * no recovery action: Push and Pull are already on the page. Nothing is rolled back
 * for a displaced push: recovery is a user action, and the user compares and
 * pushes again.
 *
 * Push is open when every area this computer writes, the project's files and
 * each connected agent, reads `fresh` or `in_sync`. Otherwise another computer
 * has pushed into one of them since, and a Pull comes first.
 *
 * | Mallard Cloud | History | Draft | Notice |
 * | --- | --- | --- | --- |
 * | gone, being deleted | any | | none: the workspace's status notice speaks |
 * | another repository is read | any | | Created again |
 * | the link's repository | replaced | the latest version | Draft discarded: Mallard Cloud holds version N |
 * | the link's repository | replaced | any other | Draft discarded and pushed again |
 * | the link's repository | missing | | Draft discarded: Push creates a new draft |
 * | the link's repository | displaced | | as on any storage |
 * | not Mallard Cloud | any | | as written below |
 */
export function clearedStorageNotice(
  comparison: ThreadSyncComparison | null | undefined,
  storageName: string,
): NoticeItem | null {
  const continuity = comparison?.remote_continuity;
  if (!comparison || !continuity) return null;
  const cloud = comparison.managed_cloud_status ?? null;
  if (cloud && cloud.state !== "ready") return null;
  const pushOpen = comparison.scopes
    .filter((scope) => scope.access === "bound_ready" || scope.access === "bound_unavailable")
    .every((scope) => scope.continuity.state === "fresh" || scope.continuity.state === "in_sync");
  if (cloud?.link && cloudPinMoved(cloud)) {
    const repository = { owner: cloud.link.owner_login, name: cloud.link.name };
    return {
      key: "storage-cleared",
      severity: "warning",
      title: t("reviews:cleared.createdAgainTitle"),
      message: pushOpen
        ? t("reviews:cleared.createdAgain", repository)
        : t("reviews:cleared.createdAgainPullFirst", repository),
    };
  }
  if (continuity.state === "displaced") {
    return {
      key: "storage-displaced",
      severity: "warning",
      message: continuity.saved_by === "push"
        ? t("reviews:cleared.displacedPush", { storage: storageName })
        : t("reviews:cleared.displacedPull", { storage: storageName }),
    };
  }
  if (continuity.state !== "remote_missing" && continuity.state !== "replaced") return null;
  if (cloud) {
    if (continuity.state === "remote_missing") {
      return { key: "storage-cleared", severity: "warning", title: t("reviews:cleared.draftDiscarded"), message: t("reviews:cleared.newDraft") };
    }
    // A discard returns the draft to the latest version.
    const version = cloud.publication?.draft_ahead === 0 ? cloud.publication.latest_version : null;
    if (version !== null) {
      return {
        key: "storage-cleared",
        severity: "warning",
        title: t("reviews:cleared.draftDiscarded"),
        message: pushOpen
          ? t("reviews:cleared.holdsVersion", { version })
          : t("reviews:cleared.holdsVersionPullFirst", { version }),
      };
    }
    return {
      key: "storage-cleared",
      severity: "warning",
      title: t("reviews:cleared.pushedAgainTitle"),
      message: pushOpen ? "" : t("reviews:cleared.pullFirst"),
    };
  }
  return {
    key: "storage-cleared",
    severity: "warning",
    message: pushOpen
      ? t("reviews:cleared.cleared", { storage: storageName })
      : t("reviews:cleared.clearedPullFirst", { storage: storageName }),
  };
}
