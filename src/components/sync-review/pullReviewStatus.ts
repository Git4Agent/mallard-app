import type { ThreadStatusIconName } from "./ThreadStatusIcon";

/** Pull review contains only storage-side work and two-sided changes. */
export function isPullReviewStatus(status: ThreadStatusIconName): boolean {
  return status === "cloud-outstanding" || status === "divergent";
}

/** Push review contains only work that can publish from this computer. */
export function isPushReviewStatus(status: ThreadStatusIconName): boolean {
  return status === "local-outstanding";
}

export function isPushReviewDecision(decision: {
  action: string;
  selectable: boolean;
}): boolean {
  return decision.selectable
    && (decision.action === "publish" || decision.action === "create_cloud");
}

/** A local session the review could not fit into this push. It stays in the
 *  list, in its place, unselectable: the push is full with newer sessions. */
export function isCaptureBudgetHold(decision: {
  action: string;
  blocker?: string | null;
} | null | undefined): boolean {
  return decision?.action === "blocked" && decision.blocker === "capture_budget";
}

/** A push review row: work this computer can publish, or work that waits
 *  for a later push because this one is full. */
export function isPushReviewRow(
  status: ThreadStatusIconName,
  decision: { action: string; selectable: boolean; blocker?: string | null },
): boolean {
  return isCaptureBudgetHold(decision)
    || (isPushReviewStatus(status) && isPushReviewDecision(decision));
}
