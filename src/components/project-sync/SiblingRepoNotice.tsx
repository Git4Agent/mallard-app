import type { BundleSnapshotSummary, SiblingBundleStatus } from "../../types";
import Notice from "../notice/Notice";

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
  /* `warning`, not `info`: every branch of the body says the reader is seeing
   * less than the whole truth — a repo they cannot see carries updates, or two
   * repos are racing for the same repository — and every branch asks for the
   * same act to fix it. Nothing is broken, so it is not `danger`.
   *
   * `standing`, not an event: the status is one of the answers the project
   * load returns, so the banner is on screen the moment the storage block
   * first paints. Content the view renders with is not a status message
   * (WCAG 2.2 SC 4.1.3), and the old role="status" was announcing a fact the
   * reader had never been without. */
  return (
    <Notice
      severity="warning"
      timing="standing"
      label="Notes about this storage"
      className="sync-storage-notice sync-sibling-repo-notice"
      title={`This storage already has this repository — “${sibling.display_name}”`}
      action={(
        <button
          type="button"
          className="btn btn-small btn-primary"
          disabled={busy}
          onClick={() => onConnect(matches)}
        >
          Connect…
        </button>
      )}
    >
      {body}
    </Notice>
  );
}
