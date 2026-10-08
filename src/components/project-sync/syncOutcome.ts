import type { SyncOutcomeView, SyncOutcomeRow, SyncOutcomeStat, OutcomeActionPresentation } from "../sync-review/outcome";
import type {
  AfterPullActionOffer,
  BundleReadiness,
  BundleReadinessIssue,
  MaintenanceActionResult,
  ProjectOperationResult,
  PullReview,
  RestoreResult,
  ThreadSyncComparison,
} from "../../types";
import { compactThreadTitle, syncHoldDetail } from "./ProjectActivityPage";
import { pushReviewPullHolds } from "./PushResourceWorkspace";
import { pullActionResourceIds, type PullReviewItem } from "./pullReviewModel";
import { t } from "../../i18n";
import { backendText } from "../../i18n/backend";

export type SyncOutcomeAction =
  /** A follow-up the provider declared for a finished Pull. Available offers
   *  run through the generic command; unavailable offers show their message. */
  | {
    kind: "after_pull_action";
    label: string;
    title: string;
    offer: AfterPullActionOffer;
  }
  /** Nothing failed to apply — the environment did. Recheck without another Pull. */
  | { kind: "recheck"; label: string }
  | { kind: "open_pull"; label: string };

export interface SyncOutcome extends SyncOutcomeView<SyncOutcomeAction> {
  projectId: string;
  storageId: string;
}

/* An outcome is built once, when its Push or Pull lands, in the language
   active then; its titles and reasons from the backend stay as written. */

/* Readiness issues, after-Pull offers and maintenance results are the
   backend's English, each with a code and the values it was built from. */

/** The `readiness.<key>` an issue is worded by, or null when the values
 *  its sentence needs are missing. */
function readinessKey(issue: BundleReadinessIssue): string | null {
  if (issue.issue_id === `provider-home-${issue.provider}`) return issue.provider_name ? "provider-home" : null;
  if (issue.issue_id === "conversation-conflicts" || issue.issue_id === "restore-project-data") {
    return issue.count == null ? null : issue.issue_id;
  }
  return issue.issue_id;
}

export function readinessIssueText(issue: BundleReadinessIssue): { title: string; detail?: string | null } {
  const key = readinessKey(issue);
  if (!key) return { title: issue.title, detail: issue.detail };
  const values = { count: issue.count ?? "", provider: issue.provider_name ?? "" };
  const detail = key === "restore-project-data" ? `detail.${issue.severity}` : "detail";
  return {
    title: backendText(`readiness.${key}.title`, issue.title, values),
    detail: issue.detail ? backendText(`readiness.${key}.${detail}`, issue.detail, values) : issue.detail,
  };
}

function offerText(offer: AfterPullActionOffer, part: "label" | "message"): string {
  return offer.message_code && offer.app_name
    ? backendText(`afterPull.${offer.message_code}.${part}`, offer[part], { app: offer.app_name, path: offer.home_path ?? "" })
    : offer[part];
}

/** What a maintenance action did. A failed one's summary is an error and
 *  stays as the backend wrote it. */
export function maintenanceSummary(result: MaintenanceActionResult): string {
  return result.summary_code
    ? backendText(`maintenanceResult.${result.summary_code}`, result.summary, {
      app: result.app_name ?? "",
      path: result.home_path ?? "",
    })
    : result.summary;
}

function displayThreadTitle(value: string): string {
  return compactThreadTitle(value).trim() || t("reviews:review.untitledSession");
}

export function buildPushOutcome({
  projectId,
  storageId,
  selectedResourceIds,
  threadComparison,
  result,
}: {
  projectId: string;
  storageId: string;
  selectedResourceIds: ReadonlySet<string>;
  threadComparison?: ThreadSyncComparison | null;
  result: ProjectOperationResult;
}): SyncOutcome {
  const entries = threadComparison?.entries ?? [];
  const entryByResource = new Map(entries.map((entry) => [entry.resource_id, entry]));
  // The backend reports every selected conversation it did not publish, by
  // the review's resource id; the comparison it was reviewed against
  // supplies the titles. Rows the comparison already held for Pull stay
  // held unless the backend reported them itself.
  const reported = (result.results ?? []).filter((row) => (
    row.state === "failed" || row.state === "held"
  ));
  const reportedIds = new Set(reported.map((row) => row.resource_id));
  const failed = reported.filter((row) => row.state === "failed");
  const reportedHeld = reported.filter((row) => row.state === "held");
  const comparisonHeld = pushReviewPullHolds(threadComparison)
    .filter((entry) => !reportedIds.has(entry.resource_id));
  const heldIds = new Set([
    ...reportedHeld.map((row) => row.resource_id),
    ...comparisonHeld.map((entry) => entry.resource_id),
  ]);
  const pushed = entries.filter((entry) => (
    selectedResourceIds.has(entry.resource_id)
    && !heldIds.has(entry.resource_id)
    && !reportedIds.has(entry.resource_id)
  ));
  const titleFor = (resourceId: string): string => (
    displayThreadTitle(entryByResource.get(resourceId)?.display_name ?? resourceId)
  );
  const threadFor = (resourceId: string): string | undefined => (
    entryByResource.get(resourceId)?.thread_id
  );

  const pushedCount = pushed.length > 0 || reported.length > 0
    ? pushed.length
    : (result.resources_changed ?? 0);
  const heldCount = heldIds.size;
  const attention = heldCount + failed.length;
  const rows: SyncOutcomeRow[] = [
    ...pushed.map((entry) => ({
      key: entry.resource_id,
      title: displayThreadTitle(entry.display_name),
      status: "synced" as const,
      threadId: entry.thread_id,
      updatedAt: entry.local_updated_at ?? entry.storage_updated_at,
    })),
    ...reportedHeld.map((row) => ({
      key: row.resource_id,
      title: titleFor(row.resource_id),
      status: "held" as const,
      reason: row.message ?? null,
      threadId: threadFor(row.resource_id),
    })),
    ...comparisonHeld.map((entry) => ({
      key: entry.resource_id,
      title: displayThreadTitle(entry.display_name),
      status: "held" as const,
      reason: syncHoldDetail("push", entry),
      threadId: entry.thread_id,
      updatedAt: entry.storage_updated_at ?? entry.local_updated_at,
    })),
    ...failed.map((row) => ({
      key: row.resource_id,
      title: titleFor(row.resource_id),
      status: "failed" as const,
      reason: row.message ?? null,
      threadId: threadFor(row.resource_id),
    })),
  ];

  const stats: SyncOutcomeStat[] = [
    {
      icon: "upload",
      value: String(pushedCount),
      label: attention > 0 ? t("reviews:outcome.pushed") : t("reviews:nouns.session", { count: pushedCount }),
    },
    ...(heldCount > 0
      ? [{ icon: "git-compare-arrows" as const, value: String(heldCount), label: t("reviews:outcome.heldForPull") }]
      : []),
    ...(failed.length > 0
      ? [{ icon: "octagon-x" as const, value: String(failed.length), label: t("reviews:outcome.notPushed") }]
      : []),
    ...(result.revision != null
      ? [{ icon: "git-branch" as const, label: "Version", value: String(result.revision), lead: true }]
      : []),
  ];

  return {
    projectId,
    storageId,
    tone: attention > 0 ? "warning" : "success",
    title: attention > 0
      ? t("reviews:outcome.pushedSome", { pushed: pushedCount, total: pushedCount + attention })
      : t("reviews:outcome.pushComplete"),
    stats,
    rows,
    actions: heldCount > 0
      ? [{ kind: "open_pull", label: t("reviews:review.openPull") }]
      : [],
    defaultOpen: attention > 0,
    technical: [
      ...(result.message ? [{ label: t("reviews:outcome.message"), value: result.message }] : []),
      ...(result.operation_id ? [{ label: t("reviews:outcome.operation"), value: result.operation_id }] : []),
    ],
  };
}

export function buildPullOutcome({
  projectId,
  plan,
  result,
  readiness,
  items,
  completedResourceIds,
  error,
}: {
  projectId: string;
  plan: PullReview;
  result: RestoreResult;
  readiness: BundleReadiness | null;
  items: PullReviewItem[];
  completedResourceIds: ReadonlySet<string>;
  error?: string | null;
}): SyncOutcome {
  const itemByResource = new Map(items.map((item) => [item.resourceId, item]));
  const resourceByAction = pullActionResourceIds(plan);
  const threadByResource = new Map(plan.conversation_review.map((row) => [
    row.resource_id,
    row.thread_id ?? row.cloud_thread_id,
  ]));
  // A conversation names itself. "" means the stored thread has no name at
  // all, which displayThreadTitle renders as "Untitled session" — the row does
  // not invent one out of a truncated id, because line 2 already prints the
  // whole identifier and a cloud id's first characters are pure namespace.
  const nameFor = (resourceId: string): string => {
    const item = itemByResource.get(resourceId);
    if (!item) return resourceId;
    return item.threadTitle ?? item.title;
  };

  const preserved = result.preserved_threads ?? [];
  const preservedResourceIds = new Set(preserved
    .map((thread) => resourceByAction.get(thread.action_id))
    .filter((resourceId): resourceId is string => Boolean(resourceId)));
  const restored = [...completedResourceIds].filter((resourceId) => (
    !preservedResourceIds.has(resourceId)
  ));
  const failures = result.failed_actions ?? [];
  // A stopped Pull: the storage could not be read after the download. What
  // it installed stays, and every ticked item it did not reach is listed.
  const stopped = result.stopped ?? null;
  const notPulled = result.not_pulled_action_ids ?? [];
  const fileTitle = (actionId: string): string | null => {
    const path = plan.actions.find((action) => action.action_id === actionId)?.target_path;
    return path ? path.split("/").pop() ?? path : null;
  };
  // This readiness issue summarizes the same failed receipts shown below.
  // Keep independent setup errors, but do not report the failures twice.
  const readinessProblems = (readiness?.issues ?? []).filter((issue) => (
    issue.severity === "error"
    && !(issue.issue_id === "restore-project-data" && failures.length > 0)
  ));

  const rows: SyncOutcomeRow[] = [
    ...restored.map((resourceId) => ({
      key: resourceId,
      title: displayThreadTitle(nameFor(resourceId)),
      status: "synced" as const,
      threadId: threadByResource.get(resourceId),
    })),
    ...preserved.map((thread) => ({
      key: thread.action_id,
      // Use the preserved marker title shown in the provider's session list.
      title: displayThreadTitle(thread.preserved_display_title),
      status: "preserved" as const,
      reason: t("reviews:outcome.preservedReason"),
      threadId: thread.preserved_thread_id,
    })),
    ...failures.map((failure) => {
      const resourceId = resourceByAction.get(failure.action_id);
      return {
        key: failure.action_id,
        title: displayThreadTitle(resourceId
          ? nameFor(resourceId)
          : failure.display_name ?? t("reviews:outcome.restoreAction")),
        status: "failed" as const,
        reason: failure.message,
        threadId: resourceId ? threadByResource.get(resourceId) : undefined,
      };
    }),
    ...readinessProblems.map((issue) => {
      const text = readinessIssueText(issue);
      return {
        key: issue.issue_id,
        title: text.title,
        status: "failed" as const,
        reason: text.detail,
      };
    }),
    ...notPulled.map((actionId) => {
      const resourceId = resourceByAction.get(actionId);
      return {
        key: actionId,
        title: resourceId
          ? displayThreadTitle(nameFor(resourceId))
          : fileTitle(actionId) ?? t("reviews:outcome.restoreAction"),
        status: "not_pulled" as const,
        threadId: resourceId ? threadByResource.get(resourceId) : undefined,
      };
    }),
  ];

  // A stopped Pull does not say that Push is blocked: after a clear there is
  // nothing to pull, and the standing note says what to do next.
  const baselineStalled = result.reviewed_base_advanced === false && !stopped;
  if (baselineStalled) {
    rows.push({
      key: `${result.plan_id ?? "plan"}:baseline`,
      title: t("reviews:outcome.stillBlocked"),
      status: "held",
      reason: t("reviews:outcome.stillBlockedReason"),
    });
  }

  const problemCount = failures.length
    + readinessProblems.length
    + (error ? 1 : 0);
  const needsAttention = problemCount > 0 || baselineStalled || stopped !== null;
  const afterPullActions: SyncOutcomeAction[] = !needsAttention
    ? result.after_pull_actions.map((offer) => ({
      kind: "after_pull_action",
      label: offerText(offer, "label"),
      title: offerText(offer, "message"),
      offer,
    }))
    : [];

  const stats: SyncOutcomeStat[] = [
    { icon: "download", value: String(restored.length), label: t("reviews:outcome.restored") },
    ...(preserved.length > 0
      ? [{ icon: "git-fork" as const, value: String(preserved.length), label: t("reviews:outcome.keptIndependently") }]
      : []),
    ...(failures.length > 0
      ? [{
        icon: "octagon-x" as const,
        value: String(failures.length),
        label: t("reviews:outcome.failed"),
      }]
      : []),
    ...(notPulled.length > 0
      ? [{
        icon: "git-compare-arrows" as const,
        value: String(notPulled.length),
        label: t("reviews:outcome.notPulled"),
      }]
      : []),
    { icon: "git-branch", label: "Version", value: String(plan.revision), lead: true },
  ];

  const actions: SyncOutcomeAction[] = afterPullActions.length > 0
    ? afterPullActions
    : failures.length === 0
      && !baselineStalled
      && stopped === null
      && (readinessProblems.length > 0 || error)
      && result.plan_id
      ? [{ kind: "recheck", label: t("reviews:outcome.recheck") }]
      : needsAttention
        ? [{ kind: "open_pull", label: t("reviews:review.openPull") }]
        : [];

  // A session's Pull names the session. Its reviewed baseline never moves,
  // so it says nothing about Push: the project's badge already says what
  // else is in storage.
  const pulledSession = plan.scope.kind === "session" && restored.length + preserved.length > 0
    ? plan.conversation_review[0]
    : undefined;

  return {
    projectId,
    storageId: plan.storage_id,
    tone: needsAttention ? "warning" : "success",
    message: stopped ?? undefined,
    title: stopped !== null
      ? t("reviews:outcome.pullStopped")
      : problemCount > 0
      ? t("reviews:outcome.needsAttention", { count: problemCount })
      : baselineStalled
        ? t("reviews:outcome.pullBlocked")
        : pulledSession
          ? t("reviews:outcome.pulledSession", { title: displayThreadTitle(nameFor(pulledSession.resource_id)) })
        : restored.length + preserved.length === 0
          // The acknowledge-only apply: nothing was taken, the reviewed
          // baseline moved. "Pull complete · 0 restored" would claim a Pull
          // that never happened.
          ? t("reviews:outcome.markedReviewed")
          : t("reviews:outcome.pullComplete"),
    stats,
    rows,
    actions,
    defaultOpen: needsAttention,
    technical: [
      ...(error ? [{ label: t("reviews:outcome.error"), value: error }] : []),
      ...(result.message ? [{ label: t("reviews:outcome.message"), value: result.message }] : []),
      ...(result.plan_id ? [{ label: t("reviews:outcome.plan"), value: result.plan_id }] : []),
    ],
  };
}

export function projectOutcomeAction(action: SyncOutcomeAction, index: number): OutcomeActionPresentation {
  return action.kind === "after_pull_action"
    ? { key: `${action.kind}:${action.offer.provider}:${action.offer.action_id}:${index}`, icon: "refresh", title: action.title, available: action.offer.available }
    : { key: `${action.kind}:${index}`, icon: action.kind === "open_pull" ? "download" : "refresh", available: true };
}
