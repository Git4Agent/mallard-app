import { useEffect, useState } from "react";
import type { BundleFetchProgress, ThreadSyncComparison, ThreadSyncEntry } from "../../types";
import Icon from "../Icons";
import { ReviewBudgetGauge } from "../sync-review/ReviewBudgetMeter";
import { formatBytes } from "../../format";
import { SessionPushRow, type PushSessionFacts } from "./ProjectActivityPage";
import {
  PUSH_RUNNING_NOTE,
  PushApplyButton,
  pushBudgetTotal,
  pushReviewDisabledReason,
} from "./PushResourceWorkspace";

/**
 * The Push review a session's own page opens: that session, and nothing to
 * select. It keeps the project review's frame — the title, the redaction
 * flag, the gauge and Push — and draws its list of one as the review's
 * session row. A search field, a count, a recommended selection and a
 * checkbox would each act on that one row, and unticking it could only
 * disable Push, so closing the review is the one way not to push.
 */
export default function SessionPushReview({
  credentialRedactionEnabled,
  entry,
  facts,
  storageName,
  selected,
  threadComparison,
  busy,
  fetchProgress = null,
  onClose,
  onPush,
}: {
  credentialRedactionEnabled: boolean;
  /** The session under review, as the storage comparison sees it. */
  entry: ThreadSyncEntry;
  /** What the session's page had loaded when Push was pressed. */
  facts: PushSessionFacts | null;
  storageName: string;
  /** Sanitized: `sanitizePushSelection`, which is what the push carries. */
  selected: ReadonlySet<string>;
  threadComparison: ThreadSyncComparison;
  busy: boolean;
  /** Live upload progress while the push publishes; renders in the footer button. */
  fetchProgress?: BundleFetchProgress | null;
  onClose: () => void;
  onPush: () => void;
}) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, onClose]);

  /* Said once, after mount, so a review ready from its first render still
     changes the region. */
  const [readyAnnounced, setReadyAnnounced] = useState(false);
  useEffect(() => setReadyAnnounced(true), []);

  const budget = pushBudgetTotal(selected, threadComparison);
  /* The path bar's Push publishes this same review and reads this same rule. */
  const disabledReason = pushReviewDisabledReason({
    selected,
    threadComparison,
    deletionApproved: true,
    sessionScope: true,
  });
  const overLimit = budget !== null && budget.total > budget.limit;

  return (
    <section
      className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace"
      aria-labelledby="sync-push-resource-title"
    >
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2 id="sync-push-resource-title">Push review</h2>
          <span className="sync-settings-flag">Credential redaction: {credentialRedactionEnabled ? "On" : "Off"}</span>
          <span className="sync-visually-hidden" role="status">
            {readyAnnounced ? "Push review ready." : ""}
          </span>
        </div>
        <div className="sync-push-resource-actions">
          <button
            type="button"
            className="btn btn-ghost sync-inline-action-close"
            onClick={onClose}
            disabled={busy}
            aria-label="Close push review"
          >
            <Icon name="x" size={15} />
          </button>
        </div>
      </header>

      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section className="sync-history-content" aria-label="Session">
              <div className="sync-history-thread-list flat">
                <SessionPushRow entry={entry} storageName={storageName} facts={facts} />
              </div>
            </section>
          </div>
        </div>
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {overLimit && (
          /* The refusal takes a row of its own above the gauge that caused it
             and the button it stops. It does not announce: the gauge is the
             status region. */
          <p className="sync-review-budget-sentence">{disabledReason}</p>
        )}
        {busy && <span role="status"><small>{PUSH_RUNNING_NOTE}</small></span>}
        {budget && (
          <ReviewBudgetGauge
            budget={budget}
            verb="push"
            breakdown={`Session ${formatBytes(budget.sessions)} · Requirements ${formatBytes(budget.reserved)}`}
          />
        )}
        <PushApplyButton
          busy={busy}
          fetchProgress={fetchProgress}
          disabledReason={disabledReason}
          onPush={onPush}
        />
      </footer>
    </section>
  );
}
