import type { SyncReviewTabDefinition } from "../sync-review/SyncReviewTabs";

/** The project review steps: Git & sessions, then optional project files. */
export type SyncReviewStep = "history" | "project_files";

export type SyncReviewTabCounts = Record<SyncReviewStep, number>;

export function syncReviewSteps(includeProjectFiles: boolean): SyncReviewStep[] {
  return includeProjectFiles ? ["history", "project_files"] : ["history"];
}

/**
 * The first tab's name follows the eligibility probe: a folder the backend
 * declared eligible for file sync is not inside a Git work tree, so there is
 * no Git history to co-bill the tab for — it holds sessions alone.
 */
export function historyStepLabel(eligibleForProjectFiles: boolean): string {
  return eligibleForProjectFiles ? "Sessions" : "Git & sessions";
}

const STEP_DEFINITIONS: Record<SyncReviewStep, { label: string; icon: "git-branch" | "file" }> = {
  history: { label: "Git & sessions", icon: "git-branch" },
  project_files: { label: "Project files", icon: "file" },
};

/** Build the neutral tab definitions for the shared review strip. */
export function projectReviewTabs(
  steps: readonly SyncReviewStep[],
  counts: SyncReviewTabCounts,
  warningSteps: ReadonlySet<SyncReviewStep> = new Set(),
  historyLabel: string = STEP_DEFINITIONS.history.label,
): SyncReviewTabDefinition[] {
  return steps.map((step) => ({
    id: step,
    label: step === "history" ? historyLabel : STEP_DEFINITIONS[step].label,
    icon: STEP_DEFINITIONS[step].icon,
    count: counts[step],
    warning: warningSteps.has(step),
  }));
}
