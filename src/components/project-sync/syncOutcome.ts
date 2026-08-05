import type { IconName } from "../Icons";
import type {
  BundleReadiness,
  ProjectOperationResult,
  RestorePlan,
  RestoreResult,
  ThreadSyncComparison,
} from "../../types";
import { compactThreadTitle, syncHoldDetail } from "./ProjectChatHistoryPage";
import { pushReviewPullHolds } from "./PushResourceWorkspace";
import type { PullReviewItem } from "./pullReviewModel";

/**
 * What a finished Push or Pull leaves behind. The flow closes when it is done
 * — the outcome is not a step, so it does not hold a review open — and lands
 * on the storage card as one foldable banner: a summary line that reads
 * without opening, and the sessions themselves inside.
 */
export type SyncOutcomeRowStatus = "synced" | "held" | "forked" | "failed";

export interface SyncOutcomeRow {
  key: string;
  title: string;
  status: SyncOutcomeRowStatus;
  /** Why this session did not land the way the others did. */
  reason?: string | null;
  threadId?: string | null;
  updatedAt?: number | null;
  /** How many fork stamps the display name carried; said once, not printed N times. */
  forkCount?: number;
}

/**
 * Every fork appends "— fork · <timestamp>" to a session's display name, so a
 * re-forked session accumulates a wall of dates that says nothing. The row
 * strips every stamp and counts them instead; the trailing date stamp goes the
 * way the history list already sends it.
 */
export function sessionName(value: string): { name: string; forks: number } {
  const stamp = /\s*—\s*fork\s*·\s*\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?Z?)?/g;
  const forks = (value.match(stamp) ?? []).length;
  return {
    name: compactThreadTitle(value.replace(stamp, "")).trim() || "Untitled session",
    forks,
  };
}

export interface SyncOutcomeStat {
  icon: IconName;
  value?: string;
  label: string;
  /** Renders "Generation 6" rather than "6 generation". */
  lead?: boolean;
}

export type SyncOutcomeAction =
  | { kind: "restart_codex"; label: string; title: string; operationId: string; appKind: "codex" | "codex_plus_plus" }
  | { kind: "retry_fork"; label: string; operationId: string }
  /** Nothing failed to apply — the environment did. Re-ask, don't re-pull. */
  | { kind: "recheck"; label: string; planId: string }
  | { kind: "open_pull"; label: string };

export interface SyncOutcome {
  projectId: string;
  storageId: string;
  tone: "success" | "warning";
  title: string;
  stats: SyncOutcomeStat[];
  rows: SyncOutcomeRow[];
  action?: SyncOutcomeAction;
  /** A clean result folds itself away; one holding a decision opens. */
  defaultOpen: boolean;
  /** Support detail, behind its own disclosure inside the fold. */
  technical: Array<{ label: string; value: string }>;
}

function plural(count: number, one: string, many = `${one}s`): string {
  return count === 1 ? one : many;
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

  // The backend counts every resource it wrote; the sessions are the part a
  // reader recognises, so they lead and the raw count never contradicts them.
  const pushedCount = pushed.length > 0 ? pushed.length : (result.resources_changed ?? 0);
  const rows: SyncOutcomeRow[] = [
    ...pushed.map((entry) => {
      const named = sessionName(entry.display_name);
      return {
        key: entry.resource_id,
        title: named.name,
        status: "synced" as const,
        forkCount: named.forks,
        threadId: entry.thread_id,
        updatedAt: entry.local_updated_at ?? entry.storage_updated_at,
      };
    }),
    ...held.map((entry) => {
      const named = sessionName(entry.display_name);
      return {
        key: entry.resource_id,
        title: named.name,
        status: "held" as const,
        forkCount: named.forks,
        reason: syncHoldDetail("push", entry),
        threadId: entry.thread_id,
        updatedAt: entry.storage_updated_at ?? entry.local_updated_at,
      };
    }),
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
    ...(result.generation != null
      ? [{ icon: "git-branch" as const, label: "Generation", value: String(result.generation), lead: true }]
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
  const titleByResource = new Map(items.map((item) => [item.resourceId, item.title]));
  const divergences = "codex_divergences" in result ? (result.codex_divergences ?? []) : [];
  // A fork only counts when the child thread exists: that is the one outcome
  // Codex cannot show until it reloads, so it alone earns the restart offer.
  const forked = divergences.filter((divergence) => (
    divergence.resolution === "preserve_local_with_native_fork"
    && Boolean(divergence.preserved_provider_thread_id)
  ));
  // A divergence result names its action, not its resource; the plan is what
  // ties the two together.
  const resourceByAction = new Map((plan.conversation_actions ?? []).map((action) => [
    action.action_id,
    action.source_resource_id ?? `${action.provider}:conversation:${action.thread_id}`,
  ]));
  const forkedResourceIds = new Set(forked
    .map((divergence) => resourceByAction.get(divergence.action_id))
    .filter((resourceId): resourceId is string => Boolean(resourceId)));
  const checkpoints = (result.codex_fork_checkpoints ?? []).filter(
    (checkpoint) => checkpoint.origin !== "manual",
  );
  const failures = result.failed_actions ?? [];
  const readinessProblems = (readiness?.issues ?? []).filter((issue) => issue.severity === "error");

  // A restore action names a conversation only by its logical path, so the
  // plan's item title is the generic "Conversation". The thread's own pinned
  // title is the best name available; failing that the id at least tells two
  // rows apart, which a wall of "Conversation" does not.
  const threadByResource = new Map((plan.conversation_actions ?? []).map((action) => [
    action.source_resource_id ?? `${action.provider}:conversation:${action.thread_id}`,
    { threadId: action.thread_id, pinnedTitle: action.divergence?.pinned_cloud_title ?? action.divergence?.pinned_local_title },
  ]));
  const nameFor = (resourceId: string): string => {
    const planned = threadByResource.get(resourceId);
    const itemTitle = titleByResource.get(resourceId);
    if (planned?.pinnedTitle) return planned.pinnedTitle;
    if (itemTitle && itemTitle !== "Conversation") return itemTitle;
    return planned ? `Session ${planned.threadId.slice(0, 8)}` : (itemTitle ?? resourceId);
  };

  const restored = [...completedResourceIds].filter((resourceId) => !forkedResourceIds.has(resourceId));
  const rows: SyncOutcomeRow[] = [
    ...restored.map((resourceId) => {
      const named = sessionName(nameFor(resourceId));
      return {
        key: resourceId,
        title: named.name,
        status: "synced" as const,
        forkCount: named.forks,
        threadId: threadByResource.get(resourceId)?.threadId,
      };
    }),
    ...forked.map((divergence) => {
      const named = sessionName(divergence.desired_original_name
        ?? nameFor(resourceByAction.get(divergence.action_id) ?? ""));
      return {
        key: divergence.action_id,
        title: named.name,
        status: "forked" as const,
        forkCount: named.forks,
        // The fork itself landed, but Codex may not list it yet. Saying so on
        // the row is the difference between "it worked" and "it worked, and
        // here is why you cannot see it".
        reason: divergence.inventory_refresh_status === "failed"
          ? "local session kept as a Codex fork — Codex's conversation list did not refresh"
          : divergence.inventory_refresh_status === "deferred"
            ? "local session kept as a Codex fork — Codex is still finishing discovery"
            : "local session kept as a Codex fork",
      };
    }),
    ...checkpoints.map((checkpoint) => {
      const named = sessionName(checkpoint.suggested_fork_name);
      return {
        key: checkpoint.operation_id,
        title: named.name,
        status: "failed" as const,
        forkCount: named.forks,
        // The row says the thing the user needs first — nothing of theirs was
        // lost. The backend's own sentence is support detail, so it goes in the
        // technical block; the thread id rides along for the same reason.
        reason: "Codex didn't confirm the fork — your local session is untouched",
        threadId: checkpoint.source_provider_thread_id,
      };
    }),
    ...failures.map((failure) => {
      const named = sessionName(failure.display_name ?? failure.action_id);
      return {
        key: failure.action_id,
        title: named.name,
        status: "failed" as const,
        forkCount: named.forks,
        reason: failure.message,
      };
    }),
    ...readinessProblems.map((issue) => ({
      key: issue.issue_id,
      title: issue.title,
      status: "failed" as const,
      reason: issue.detail,
    })),
  ];

  // The apply ran, but required changes were skipped, so this computer is
  // still behind the storage and Push stays gated. Nothing else on the banner
  // would tell the user why their next Push refuses to start.
  const baselineStalled = result.reviewed_base_advanced === false;
  if (baselineStalled) {
    rows.push({
      key: `${result.plan_id ?? "plan"}:baseline`,
      title: "Push is still blocked",
      status: "held",
      reason: "Required changes were skipped, so this computer stays behind storage. Pull them to unblock Push.",
    });
  }

  const problemCount = checkpoints.length + failures.length + readinessProblems.length
    + (error ? 1 : 0) + (baselineStalled ? 1 : 0);
  const appRefresh = result.codex_app_refresh ?? null;
  const restartable = forked.length > 0
    && appRefresh?.state === "restart_available"
    && result.status === "complete";

  const stats: SyncOutcomeStat[] = [
    { icon: "download", value: String(restored.length), label: "restored" },
    ...(forked.length > 0
      ? [{ icon: "git-fork" as const, value: String(forked.length), label: `${plural(forked.length, "session")} forked` }]
      : []),
    ...(checkpoints.length > 0
      ? [{ icon: "git-fork" as const, value: String(checkpoints.length), label: "fork pending" }]
      : []),
    ...(failures.length + readinessProblems.length > 0
      ? [{
        icon: "octagon-x" as const,
        value: String(failures.length + readinessProblems.length),
        label: "failed",
      }]
      : []),
    { icon: "git-branch", label: "Generation", value: String(plan.generation), lead: true },
  ];

  const action: SyncOutcomeAction | undefined = restartable && appRefresh
    ? {
      kind: "restart_codex",
      label: `Restart ${appRefresh.app_name}`,
      title: `Shows the restored fork — active ${appRefresh.app_name} chats are interrupted`,
      operationId: appRefresh.operation_id,
      appKind: appRefresh.app_kind,
    }
    // Only an unacknowledged fork may be retried: a checkpoint in recovery
    // means a fork may already exist, and firing another would be the one
    // action that can duplicate a conversation.
    : checkpoints[0]?.status === "fork_unacknowledged"
      ? { kind: "retry_fork", label: "Retry fork", operationId: checkpoints[0].operation_id }
      // Every change applied and only the environment is unhappy — a read-only
      // profile, a missing binary. Pulling again would change nothing; asking
      // again after the user fixes it is the whole remedy.
      : failures.length === 0 && checkpoints.length === 0 && readinessProblems.length > 0 && result.plan_id
        ? { kind: "recheck", label: "Recheck", planId: result.plan_id }
        : problemCount > 0
          ? { kind: "open_pull", label: "Open Pull review" }
          : undefined;

  return {
    projectId,
    storageId: plan.storage_id,
    tone: problemCount > 0 ? "warning" : "success",
    title: checkpoints.length > 0
      ? "Pull complete — fork not confirmed"
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
      // Each parked checkpoint explains itself; the rows stay short.
      ...checkpoints.map((checkpoint) => ({
        label: "Fork checkpoint",
        value: checkpoint.message,
      })),
      ...(result.plan_id ? [{ label: "Plan", value: result.plan_id }] : []),
    ],
  };
}
