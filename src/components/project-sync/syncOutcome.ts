import type { IconName } from "../Icons";
import type {
  BundleReadiness,
  ProjectOperationResult,
  RestorePlan,
  RestoreResult,
  ThreadSyncComparison,
} from "../../types";
import { compactThreadTitle, syncHoldDetail } from "./ProjectActivityPage";
import { pushReviewPullHolds } from "./PushResourceWorkspace";
import type { PullReviewItem } from "./pullReviewModel";
import { isIrrecoverableSyncError } from "./syncError";

/** A finished operation rendered on the storage card after its review closes. */
export type SyncOutcomeRowStatus = "synced" | "held" | "preserved" | "failed";

export interface SyncOutcomeRow {
  key: string;
  title: string;
  status: SyncOutcomeRowStatus;
  reason?: string | null;
  threadId?: string | null;
  updatedAt?: number | null;
}

export interface SyncOutcomeStat {
  icon: IconName;
  value?: string;
  label: string;
  /** Renders "Revision 6" rather than "6 revision". */
  lead?: boolean;
}

export type SyncOutcomeAction =
  | {
    kind: "restart_codex";
    label: string;
    title: string;
    operationId: string;
    appKind: "codex" | "codex_plus_plus";
  }
  /** Nothing failed to apply — the environment did. Recheck without another Pull. */
  | { kind: "recheck"; label: string; planId: string }
  | { kind: "open_pull"; label: string }
  | { kind: "reset_metadata"; label: string };

export interface SyncOutcome {
  projectId: string;
  storageId: string;
  tone: "success" | "warning";
  title: string;
  stats: SyncOutcomeStat[];
  rows: SyncOutcomeRow[];
  action?: SyncOutcomeAction;
  defaultOpen: boolean;
  technical: Array<{ label: string; value: string }>;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return count === 1 ? one : many;
}

function displayThreadTitle(value: string): string {
  return compactThreadTitle(value).trim() || "Untitled session";
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
  const held = pushReviewPullHolds(threadComparison);
  const heldIds = new Set(held.map((entry) => entry.resource_id));
  const pushed = entries.filter((entry) => (
    selectedResourceIds.has(entry.resource_id) && !heldIds.has(entry.resource_id)
  ));

  const pushedCount = pushed.length > 0 ? pushed.length : (result.resources_changed ?? 0);
  const rows: SyncOutcomeRow[] = [
    ...pushed.map((entry) => ({
      key: entry.resource_id,
      title: displayThreadTitle(entry.display_name),
      status: "synced" as const,
      threadId: entry.thread_id,
      updatedAt: entry.local_updated_at ?? entry.storage_updated_at,
    })),
    ...held.map((entry) => ({
      key: entry.resource_id,
      title: displayThreadTitle(entry.display_name),
      status: "held" as const,
      reason: syncHoldDetail("push", entry),
      threadId: entry.thread_id,
      updatedAt: entry.storage_updated_at ?? entry.local_updated_at,
    })),
  ];

  const stats: SyncOutcomeStat[] = [
    {
      icon: "upload",
      value: String(pushedCount),
      label: held.length > 0 ? "pushed" : plural(pushedCount, "session"),
    },
    ...(held.length > 0
      ? [{ icon: "git-compare-arrows" as const, value: String(held.length), label: "held for Pull" }]
      : []),
    ...(result.revision != null
      ? [{ icon: "git-branch" as const, label: "Revision", value: String(result.revision), lead: true }]
      : []),
  ];

  return {
    projectId,
    storageId,
    tone: held.length > 0 ? "warning" : "success",
    title: held.length > 0
      ? `Pushed ${pushedCount} of ${pushedCount + held.length} sessions`
      : "Push complete",
    stats,
    rows,
    action: held.length > 0 ? { kind: "open_pull", label: "Open Pull review" } : undefined,
    defaultOpen: held.length > 0,
    technical: [
      ...(result.message ? [{ label: "Message", value: result.message }] : []),
      ...(result.operation_id ? [{ label: "Operation", value: result.operation_id }] : []),
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
  plan: RestorePlan;
  result: RestoreResult;
  readiness: BundleReadiness | null;
  items: PullReviewItem[];
  completedResourceIds: ReadonlySet<string>;
  error?: string | null;
}): SyncOutcome {
  const itemByResource = new Map(items.map((item) => [item.resourceId, item]));
  const actionRowKey = (action: RestorePlan["conversation_actions"][number]): string =>
    action.source_resource_id
      ?? `${action.provider}:conversation:${action.thread_id ?? action.cloud_thread_id}`;
  const resourceByAction = new Map(plan.conversation_actions.map((action) => [
    action.action_id,
    actionRowKey(action),
  ]));
  const threadByResource = new Map(plan.conversation_actions.map((action) => [
    actionRowKey(action),
    action.thread_id ?? action.cloud_thread_id,
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
  const readinessProblems = (readiness?.issues ?? []).filter((issue) => issue.severity === "error");

  const rows: SyncOutcomeRow[] = [
    ...restored.map((resourceId) => ({
      key: resourceId,
      title: displayThreadTitle(nameFor(resourceId)),
      status: "synced" as const,
      threadId: threadByResource.get(resourceId),
    })),
    ...preserved.map((thread) => ({
      key: thread.action_id,
      // The renamed marker title, so the row names the session exactly as
      // it now appears in the provider's session list.
      title: displayThreadTitle(thread.preserved_display_title),
      status: "preserved" as const,
      reason: "Storage history was restored and the previous local history was kept as an independent session.",
      threadId: thread.preserved_thread_id,
    })),
    ...failures.map((failure) => ({
      key: failure.action_id,
      title: displayThreadTitle(failure.display_name ?? failure.action_id),
      status: "failed" as const,
      reason: failure.message,
    })),
    ...readinessProblems.map((issue) => ({
      key: issue.issue_id,
      title: issue.title,
      status: "failed" as const,
      reason: issue.detail,
    })),
  ];

  const baselineStalled = result.reviewed_base_advanced === false;
  if (baselineStalled) {
    rows.push({
      key: `${result.plan_id ?? "plan"}:baseline`,
      title: "Push is still blocked",
      status: "held",
      reason: "Required changes were skipped, so this computer stays behind storage. Pull them to unblock Push.",
    });
  }

  const recoveryRequired = result.status === "recovery_required";
  if (recoveryRequired) {
    rows.push({
      key: `${result.plan_id ?? "plan"}:recovery`,
      title: "Sync recovery is required",
      status: "failed",
      reason: "Mallard preserved the operation journal. Open Pull again to recover before syncing this session.",
    });
  }

  const problemCount = failures.length
    + readinessProblems.length
    + (error ? 1 : 0)
    + (baselineStalled ? 1 : 0)
    + (recoveryRequired ? 1 : 0);
  const appRefresh = result.codex_app_refresh ?? null;
  const irrecoverable = [error, ...failures.map((failure) => failure.message)]
    .some((message) => isIrrecoverableSyncError(message));
  const restartable = problemCount === 0
    && appRefresh?.state === "restart_available"
    && result.status === "complete";

  const stats: SyncOutcomeStat[] = [
    { icon: "download", value: String(restored.length), label: "restored" },
    ...(preserved.length > 0
      ? [{ icon: "git-fork" as const, value: String(preserved.length), label: "kept independently" }]
      : []),
    ...(failures.length + readinessProblems.length > 0
      ? [{
        icon: "octagon-x" as const,
        value: String(failures.length + readinessProblems.length),
        label: "failed",
      }]
      : []),
    { icon: "git-branch", label: "Revision", value: String(plan.revision), lead: true },
  ];

  const action: SyncOutcomeAction | undefined = irrecoverable
    ? { kind: "reset_metadata", label: "Reset local sync state" }
    : restartable && appRefresh
    ? {
      kind: "restart_codex",
      label: `Restart ${appRefresh.app_name}`,
      title: `Shows restored sessions — active ${appRefresh.app_name} chats are interrupted`,
      operationId: appRefresh.operation_id,
      appKind: appRefresh.app_kind,
    }
    : failures.length === 0
      && !recoveryRequired
      && !baselineStalled
      && readinessProblems.length > 0
      && result.plan_id
      ? { kind: "recheck", label: "Recheck", planId: result.plan_id }
      : problemCount > 0
        ? { kind: "open_pull", label: "Open Pull review" }
        : undefined;

  return {
    projectId,
    storageId: plan.storage_id,
    tone: problemCount > 0 ? "warning" : "success",
    title: recoveryRequired
      ? "Pull paused for recovery"
      : baselineStalled && problemCount === 1
        ? "Pull complete — Push still blocked"
        : problemCount > 0
          ? `${problemCount} ${plural(problemCount, "change")} ${problemCount === 1 ? "needs" : "need"} attention`
          : "Pull complete",
    stats,
    rows,
    action,
    defaultOpen: problemCount > 0,
    technical: [
      ...(error ? [{ label: "Error", value: error }] : []),
      ...(result.message ? [{ label: "Message", value: result.message }] : []),
      ...(result.plan_id ? [{ label: "Plan", value: result.plan_id }] : []),
    ],
  };
}
