import type {
  BundleReadiness,
  ConversationResolutionSelection,
  DependencyPlan,
  DependencyResult,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../types";
import { errorMessage } from "./model";

export interface PullReviewApi {
  planRestore: (storageId: string, bundleId: string, binding: ProjectBinding) => Promise<RestorePlan>;
  planDependencies: (restorePlanId: string) => Promise<DependencyPlan>;
  getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
}

export interface PullReviewRequest {
  storageId: string;
  bundleId: string;
  binding: ProjectBinding;
}

export interface PullReviewSupport {
  dependencyPlan: DependencyPlan | null;
  readiness: BundleReadiness | null;
  errors: string[];
}

export interface StartedPullReview {
  restorePlan: RestorePlan;
  support: Promise<PullReviewSupport>;
}

export type PullApplyPhase = "idle" | "restoring" | "installing" | "verifying" | "complete";

export interface PullReviewSelection {
  resourceIds: string[];
  restoreActionIds: string[];
  dependencyActionIds: string[];
  conversationResolutions: ConversationResolutionSelection[];
}

export interface PullReviewApplyApi {
  applyRestore: (
    planId: string,
    actionIds: string[],
    conversationResolutions: ConversationResolutionSelection[],
  ) => Promise<RestoreResult>;
  applyDependencies: (planId: string, actionIds: string[]) => Promise<DependencyResult>;
  getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
}

export interface PullReviewApplyResult {
  restoreResult: RestoreResult | null;
  dependencyResult: DependencyResult | null;
  readiness: BundleReadiness | null;
  success: boolean;
  error: string | null;
  failedPhase: "restoring" | "installing" | "verifying" | null;
}

function hasKeepLocalConversationDecision(plan: RestorePlan): boolean {
  return plan.actions.some((action) => (
    action.kind.kind === "manual"
    && /^(codex|claude):session:/.test(action.resource_id)
    && Boolean(action.target_path)
    && Boolean(action.source_sha256)
    && Boolean(action.expected_target_sha256)
  ));
}

/**
 * A post-checkpoint replan can contain only informational conversation rows
 * (`NoOp` for the restored original and `KeepLocal` for its new fork). In that
 * case there is no second user decision to collect, but the backend still
 * needs one empty restore apply to advance the reviewed recipe base.
 */
export function hasPendingRestoreMutations(plan: RestorePlan): boolean {
  if (plan.actions.length > 0) return true;
  return plan.conversation_actions.some((action) => (
    action.action === "fast_forward_local"
    || action.action === "resolve_divergence"
    || action.action === "create_local"
    || action.action === "replace_local"
  ));
}

/**
 * The two backend plans are immutable approval surfaces. Refuse to combine
 * them unless every pin identifies the same remote snapshot and local
 * binding; otherwise a single Apply click could approve two different states.
 */
export function assertPullPlansAligned(
  restorePlan: RestorePlan,
  dependencyPlan: DependencyPlan | null,
  selection: PullReviewSelection,
): void {
  const restoreIds = new Set([
    ...restorePlan.actions.map((action) => action.action_id),
    ...(restorePlan.conversation_actions ?? []).map((action) => action.action_id),
  ]);
  if (selection.restoreActionIds.some((actionId) => !restoreIds.has(actionId))) {
    throw new Error("The selected restore actions no longer belong to this Pull review.");
  }
  const selectedDivergences = new Set(
    (restorePlan.conversation_actions ?? [])
      .filter((action) => (
        action.action === "resolve_divergence"
        && selection.restoreActionIds.includes(action.action_id)
      ))
      .map((action) => action.action_id),
  );
  const resolutionIds = new Set<string>();
  for (const selectionEntry of selection.conversationResolutions) {
    if (
      !selectedDivergences.has(selectionEntry.action_id)
      || !resolutionIds.add(selectionEntry.action_id)
    ) {
      throw new Error("The selected conversation resolution no longer belongs to this Pull review.");
    }
  }
  if (resolutionIds.size !== selectedDivergences.size) {
    throw new Error("Choose one resolution for every selected conversation conflict.");
  }
  if (!dependencyPlan && selection.dependencyActionIds.length > 0) {
    throw new Error("The selected tools do not have an installation plan yet.");
  }
  if (dependencyPlan) {
    const sameSnapshot =
      dependencyPlan.storage_id === restorePlan.storage_id &&
      dependencyPlan.bundle_id === restorePlan.bundle_id &&
      dependencyPlan.host_id === restorePlan.host_id &&
      dependencyPlan.replica_id === restorePlan.replica_id &&
      dependencyPlan.expected_host_identity_revision === restorePlan.expected_host_identity_revision &&
      dependencyPlan.generation === restorePlan.generation &&
      dependencyPlan.commit_id === restorePlan.commit_id &&
      dependencyPlan.manifest_sha256 === restorePlan.manifest_sha256 &&
      dependencyPlan.binding_revision === restorePlan.binding_revision;
    if (!sameSnapshot) {
      throw new Error("The project and tool plans describe different bundle generations. Refresh the Pull review.");
    }
    const dependencyIds = new Set(dependencyPlan.actions.map((action) => action.action_id));
    if (selection.dependencyActionIds.some((actionId) => !dependencyIds.has(actionId))) {
      throw new Error("The selected tool installations no longer belong to this Pull review.");
    }
  }
}

/** Execute the one-button Pull workflow in the order presented by the UI. */
export async function applyPullReview(
  api: PullReviewApplyApi,
  restorePlan: RestorePlan,
  dependencyPlan: DependencyPlan | null,
  selection: PullReviewSelection,
  onPhase: (phase: PullApplyPhase) => void,
): Promise<PullReviewApplyResult> {
  assertPullPlansAligned(restorePlan, dependencyPlan, selection);
  let restoreResult: RestoreResult | null = null;
  let dependencyResult: DependencyResult | null = null;
  let readiness: BundleReadiness | null = null;
  let executionError: string | null = null;
  let failedPhase: PullReviewApplyResult["failedPhase"] = null;
  const hasProjectContentDecisions = restorePlan.actions.some((action) => (
    action.kind.kind === "write_project_file"
    || action.kind.kind === "ensure_project_directory"
    || action.kind.kind === "delete_project_file"
    || action.kind.kind === "delete_project_directory"
  ));
  const hasConversationKeepLocalDecisions = hasKeepLocalConversationDecision(restorePlan);

  onPhase("restoring");
  if (selection.restoreActionIds.length > 0 || hasProjectContentDecisions || hasConversationKeepLocalDecisions) {
    try {
      restoreResult = await api.applyRestore(
        restorePlan.plan_id,
        selection.restoreActionIds,
        selection.conversationResolutions,
      );
    } catch (reason) {
      executionError = `Restore failed: ${errorMessage(reason)}`;
      failedPhase = "restoring";
    }
  }

  if (restoreResult && restoreResult.status !== "complete") {
    onPhase("complete");
    return {
      restoreResult,
      dependencyResult: null,
      readiness: null,
      success: false,
      error: null,
      failedPhase: null,
    };
  }

  onPhase("installing");
  if (!executionError && selection.dependencyActionIds.length > 0 && dependencyPlan) {
    try {
      dependencyResult = await api.applyDependencies(
        dependencyPlan.plan_id,
        selection.dependencyActionIds,
      );
    } catch (reason) {
      executionError = `Tool installation failed: ${errorMessage(reason)}`;
      failedPhase = "installing";
    }
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
    (dependencyResult?.success ?? true) &&
    readiness?.state === "ready";
  onPhase("complete");
  return {
    restoreResult,
    dependencyResult,
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
 * supporting checks so a dependency/readiness error cannot hide a valid plan
 * or its Apply button. Planning fetches the bundle itself — no separate
 * prefetch, the payload must only be downloaded once.
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
  const [dependencies, readiness] = await Promise.allSettled([
    api.planDependencies(restorePlan.plan_id),
    api.getRestoreReadiness(restorePlan.plan_id),
  ]);
  const errors: string[] = [];
  if (dependencies.status === "rejected") {
    errors.push(`Dependency checks: ${errorMessage(dependencies.reason)}`);
  }
  if (readiness.status === "rejected") {
    errors.push(`Readiness checks: ${errorMessage(readiness.reason)}`);
  }
  return {
    dependencyPlan: dependencies.status === "fulfilled" ? dependencies.value : null,
    readiness: readiness.status === "fulfilled" ? readiness.value : null,
    errors,
  };
}
