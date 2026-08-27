import type {
  BundleReadiness,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../types";
import { errorMessage } from "../../storageError";

export interface PullReviewApi {
  planRestore: (storageId: string, bundleId: string, binding: ProjectBinding) => Promise<RestorePlan>;
  getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
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
  restorePlan: RestorePlan;
  support: Promise<PullReviewSupport>;
}

export type PullApplyPhase = "idle" | "restoring" | "verifying" | "complete";

export interface PullReviewSelection {
  resourceIds: string[];
  restoreActionIds: string[];
}

export interface PullReviewApplyApi {
  applyRestore: (
    planId: string,
    actionIds: string[],
  ) => Promise<RestoreResult>;
  getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
}

export interface PullReviewApplyResult {
  restoreResult: RestoreResult | null;
  readiness: BundleReadiness | null;
  success: boolean;
  error: string | null;
  failedPhase: "restoring" | "verifying" | null;
}

/**
 * Informational rows need no approval, but blocked rows still need a visible
 * review. Metadata adoption is a real local mutation and must be selected.
 */
export function hasPendingRestoreMutations(plan: RestorePlan): boolean {
  if (plan.actions.length > 0 || plan.conversation_actions.length > 0) return true;
  return plan.conversation_review.some((row) => row.action === "blocked");
}

/**
 * The restore plan is an immutable approval surface. Refuse a selection whose
 * ids no longer belong to it; otherwise a single Apply click could approve a
 * state the user never reviewed.
 */
export function assertPullPlansAligned(
  restorePlan: RestorePlan,
  selection: PullReviewSelection,
): void {
  const restoreIds = new Set([
    ...restorePlan.actions.map((action) => action.action_id),
    ...(restorePlan.conversation_actions ?? []).map((action) => action.action_id),
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
  restorePlan: RestorePlan,
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
      restorePlan.plan_id,
      selection.restoreActionIds,
    );
  } catch (reason) {
    executionError = `Restore failed: ${errorMessage(reason)}`;
    failedPhase = "restoring";
  }

  onPhase("verifying");
  try {
    readiness = await api.getRestoreReadiness(restorePlan.plan_id);
  } catch (reason) {
    const verificationError = `Readiness verification failed: ${errorMessage(reason)}`;
    executionError = executionError ? `${executionError} ${verificationError}` : verificationError;
    if (!failedPhase) failedPhase = "verifying";
  }
  const success =
    executionError === null &&
    (restoreResult?.success ?? true) &&
    (restoreResult ? restoreResult.status === "complete" : true) &&
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
 * supporting checks so a readiness error cannot hide a valid plan or its
 * Apply button. Planning fetches the bundle itself — no separate prefetch,
 * the payload must only be downloaded once.
 */
export async function preparePullReview(
  api: PullReviewApi,
  request: PullReviewRequest,
): Promise<RestorePlan> {
  return api.planRestore(request.storageId, request.bundleId, request.binding);
}

/** Load non-blocking context shown alongside an already-visible review. */
export async function loadPullReviewSupport(
  api: PullReviewApi,
  restorePlan: RestorePlan,
): Promise<PullReviewSupport> {
  try {
    return { readiness: await api.getRestoreReadiness(restorePlan.plan_id), errors: [] };
  } catch (reason) {
    return { readiness: null, errors: [`Readiness checks: ${errorMessage(reason)}`] };
  }
}
