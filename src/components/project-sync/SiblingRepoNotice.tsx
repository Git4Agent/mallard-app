import type { BundleSnapshotSummary, SiblingBundleStatus } from "../../types";
import Icon from "../Icons";

interface Props {
  status: SiblingBundleStatus;
  busy: boolean;
  onConnect: (matches: BundleSnapshotSummary[]) => void;
}

/**
 * The post-setup convergence banner (`docs/PLAN_REPO_BUNDLE_CONVERGENCE.md`
 * D3): another repo in the linked storage carries this project's repository,
 * and connecting to it is how pushes and pulls from every machine converge.
 * Renders nothing for the `none` state so callers can pass the status
 * through unfiltered.
 */
export default function SiblingRepoNotice({ status, busy, onConnect }: Props) {
  if (status.state === "none") return null;
  const sibling = status.state === "sibling_found"
    ? status.matches[0] ?? null
    : status.survivor;
  if (!sibling) return null;
  const matches = status.state === "sibling_found" ? status.matches : [status.survivor];
  const body = status.state === "registration_twin"
    ? "Two repos were registered for this repository at the same time, both still empty. Connect to the surviving one to converge."
    : sibling.registration_only
      ? "Another machine registered this repository here without pushing yet. Connect so both sides share one repo."
      : "It carries updates this link cannot see. Connect to receive them instead of publishing a separate repo.";
  return (
    <div className="sync-callout sync-storage-notice sync-sibling-repo-notice" role="status">
      <Icon name="git-compare-arrows" size={15} />
      <span>
        <strong>
          This storage already has this repository — “{sibling.display_name}”
        </strong>
        {body}
      </span>
      <div className="sync-sync-review-recovery-actions">
        <button
          type="button"
          className="btn btn-small btn-primary"
          disabled={busy}
          onClick={() => onConnect(matches)}
        >
          Connect…
        </button>
      </div>
    </div>
  );
}
