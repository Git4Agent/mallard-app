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
  getRestoreReadiness: (review: PullReview) => Promise<BundleReadiness>;
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

export type PullApplyPhase = "idle" | "restoring" | "verifying" | "complete";

export interface PullReviewSelection {
  resourceIds: string[];
  restoreActionIds: string[];
}

export interface PullReviewApplyApi {
  applyRestore: (
    review: PullReview,
    actionIds: string[],
  ) => Promise<RestoreResult>;
  getRestoreReadiness: (review: PullReview) => Promise<BundleReadiness>;
}

export interface PullReviewApplyResult {
  restoreResult: RestoreResult | null;
  readiness: BundleReadiness | null;
  success: boolean;
  error: string | null;
  failedPhase: "restoring" | "verifying" | null;
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
  let restoreResult: RestoreResult | null = null;
  let readiness: BundleReadiness | null = null;
  let executionError: string | null = null;
  let failedPhase: PullReviewApplyResult["failedPhase"] = null;

  onPhase("restoring");
  try {
    restoreResult = await api.applyRestore(
      restorePlan,
      selection.restoreActionIds,
    );
  } catch (reason) {
    executionError = `Restore failed: ${errorMessage(reason)}`;
    failedPhase = "restoring";
  }

  onPhase("verifying");
  try {
    readiness = await api.getRestoreReadiness(restorePlan);
  } catch (reason) {
    const verificationError = `Readiness verification failed: ${errorMessage(reason)}`;
    executionError = executionError ? `${executionError} ${verificationError}` : verificationError;
    if (!failedPhase) failedPhase = "verifying";
  }
  const success =
    executionError === null &&
    (restoreResult?.success ?? true) &&
    readiness?.state === "ready";
  onPhase("complete");
  return {
    restoreResult,
    readiness,
    success,
    error: executionError,
    failedPhase,
  };
}

/**
 * Start the complete review flow without awaiting non-blocking support. The
 * caller can render `restorePlan` immediately and observe `support` later.
 */
export async function beginPullReview(
  api: PullReviewApi,
  request: PullReviewRequest,
): Promise<StartedPullReview> {
  const restorePlan = await preparePullReview(api, request);
  return {
    restorePlan,
    support: loadPullReviewSupport(api, restorePlan),
  };
}

/**
 * Load only the mutation review. This is deliberately separate from optional
 * supporting checks so a readiness error cannot hide a valid review or its
 * Apply button. Compare fetches the manifest itself; selected payloads remain
 * Apply-time work.
 */
export async function preparePullReview(
  api: PullReviewApi,
  request: PullReviewRequest,
): Promise<PullReview> {
  return api.planRestore(request.storageId, request.bundleId, request.binding);
}

/** Load non-blocking context shown alongside an already-visible review. */
export async function loadPullReviewSupport(
  api: PullReviewApi,
  restorePlan: PullReview,
): Promise<PullReviewSupport> {
  try {
    return { readiness: await api.getRestoreReadiness(restorePlan), errors: [] };
  } catch (reason) {
    return { readiness: null, errors: [`Readiness checks: ${errorMessage(reason)}`] };
  }
}
