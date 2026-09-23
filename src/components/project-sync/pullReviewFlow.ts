import type {
  BundleReadiness,
  ProjectBinding,
  PullReview,
  RestoreResult,
} from "../../types";
import { errorMessage } from "../../storageError";

export interface PullReviewApi {
  planRestore: (storageId: string, bundleId: string, binding: ProjectBinding) => Promise<PullReview>;
  getRestoreReadiness: (localProjectId: string, storageId: string) => Promise<BundleReadiness>;
}

export interface PullReviewRequest {
  storageId: string;
  bundleId: string;
  binding: ProjectBinding;
}

export interface PullReviewSupport {
  readiness: BundleReadiness | null;
  errors: string[];
}

export interface StartedPullReview {
  restorePlan: PullReview;
  support: Promise<PullReviewSupport>;
}

export type PullApplyPhase = "idle" | "restoring";

export interface PullReviewSelection {
  resourceIds: string[];
  restoreActionIds: string[];
}

export interface PullReviewApplyApi {
  applyRestore: (
    review: PullReview,
    actionIds: string[],
  ) => Promise<RestoreResult>;
}

export interface PullReviewApplyResult {
  restoreResult: RestoreResult | null;
  readiness: BundleReadiness | null;
  success: boolean;
  error: string | null;
}

/** The backend's refusal of a Pull whose review no longer matches the
 *  storage: the pinned version lost a race, the storage was cleared, or this
 *  computer synced past the review from another window. Printed verbatim in
 *  the review's banner, with "Compare again". */
export const REVIEW_SUPERSEDED = "The storage changed since this review. Compare again.";
/** The refusal of a review older than its one-day validity. */
export const REVIEW_EXPIRED = "This review is older than a day. Compare again.";

/** Whether an apply error is one that a new review resolves. */
export function compareAgainOffered(message: string | null | undefined): boolean {
  return !!message && (message.includes(REVIEW_SUPERSEDED) || message.includes(REVIEW_EXPIRED));
}

/** Inspect the complete plan, including rows hidden from the file review. */
export function canAcceptPullMetadata(plan: PullReview): boolean {
  if (plan.actions.length > 0) return false;
  // A cleared or recreated storage is an ordinary first Pull; only a storage
  // that could not be listed claims no verdict.
  if (plan.remote_continuity?.state === "unknown") return false;
  return plan.conversation_review.every((row) => {
    if (row.blocker || row.blocked_reason) return false;
    switch (row.action) {
      case "adopt_cloud_metadata":
        return !!row.action_id;
      case "no_changes":
      case "report_push_eligible":
      case "report_local_only":
        return true;
      default:
        return false;
    }
  });
}

/** Link and Pull accept matching data without changing files or publishing a version. */
export async function acceptPullMetadata(api: PullReviewApplyApi, plan: PullReview): Promise<void> {
  if (!canAcceptPullMetadata(plan)) {
    throw new Error("Review the changes before accepting this storage version.");
  }
  const metadataIds = plan.conversation_review.flatMap((row) => (
    row.action === "adopt_cloud_metadata" && row.action_id ? [row.action_id] : []
  ));
  const result = await api.applyRestore(plan, metadataIds);
  if (!result.success || result.reviewed_base_advanced !== true) {
    throw new Error(result.message || "The storage version could not be accepted. Try again.");
  }
}

/**
 * The in-memory Pull review is the approval surface. Refuse a selection whose
 * ids no longer belong to it; otherwise a single Apply click could approve a
 * state the user never reviewed.
 */
export function assertPullPlansAligned(
  restorePlan: PullReview,
  selection: PullReviewSelection,
): void {
  const restoreIds = new Set([
    ...restorePlan.actions.map((action) => action.action_id),
    ...restorePlan.conversation_review.flatMap((row) => row.action_id ? [row.action_id] : []),
  ]);
  if (selection.restoreActionIds.some((actionId) => !restoreIds.has(actionId))) {
    throw new Error("The selected restore actions no longer belong to this Pull review.");
  }
}

/**
 * Execute the one-button Pull workflow in the order presented by the UI.
 *
 * The apply is always sent, even for an empty selection: the backend writes
 * receipts and advances the reviewed recipe base from the plan, which is the
 * acknowledge-only Pull that unblocks Push when the storage is ahead and the
 * user takes none of its changes.
 */
export async function applyPullReview(
  api: PullReviewApplyApi,
  restorePlan: PullReview,
  selection: PullReviewSelection,
  onPhase: (phase: PullApplyPhase) => void,
): Promise<PullReviewApplyResult> {
  assertPullPlansAligned(restorePlan, selection);
  onPhase("restoring");
  try {
    const restoreResult = await api.applyRestore(restorePlan, selection.restoreActionIds);
    const error = restoreResult.readiness_error
      ? `Readiness verification failed: ${restoreResult.readiness_error}`
      : null;
    return {
      restoreResult,
      readiness: restoreResult.readiness,
      success: restoreResult.success && error === null && restoreResult.readiness?.state === "ready",
      error,
    };
  } catch (reason) {
    return {
      restoreResult: null,
      readiness: null,
      success: false,
      error: `Restore failed: ${errorMessage(reason)}`,
    };
  } finally {
    onPhase("idle");
  }
}

/**
 * Start the complete review flow without awaiting non-blocking support. The
 * caller can render `restorePlan` immediately and observe `support` later.
 */
export async function beginPullReview(
  api: PullReviewApi,
  request: PullReviewRequest,
): Promise<StartedPullReview> {
  const restorePlan = await api.planRestore(request.storageId, request.bundleId, request.binding);
  return {
    restorePlan,
    support: loadPullReviewSupport(api, request.binding.local_project_id, request.storageId),
  };
}

/** Load non-blocking context shown alongside an already-visible review. */
export async function loadPullReviewSupport(
  api: PullReviewApi,
  localProjectId: string,
  storageId: string,
): Promise<PullReviewSupport> {
  try {
    return { readiness: await api.getRestoreReadiness(localProjectId, storageId), errors: [] };
  } catch (reason) {
    return { readiness: null, errors: [`Readiness checks: ${errorMessage(reason)}`] };
  }
}
