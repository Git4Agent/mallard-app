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

/** A push review row that this computer can publish. */
export function isPushReviewRow(
  status: ThreadStatusIconName,
  decision: { action: string; selectable: boolean },
): boolean {
  return isPushReviewStatus(status) && isPushReviewDecision(decision);
}
