import type {
  BundleReadiness,
  ProjectBinding,
  PullReview,
  RestoreResult,
} from "../../types";
import { errorMessage } from "../../storageError";
import { buildPullReviewItems, restoreActionIds } from "./pullReviewModel";

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

/** A review opens only when it contains storage-side or divergent work. */
export function hasPendingRestoreMutations(plan: PullReview): boolean {
  return buildPullReviewItems(plan).some((item) => restoreActionIds(item).length > 0);
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
 * user takes none of its changes. `hasPendingRestoreMutations` remains the
 * single predicate for callers deciding whether a review is worth showing.
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
