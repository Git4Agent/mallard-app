import { useId, useRef, useState } from "react";
import { clockOptions } from "../../clock";
import Icon, { type IconName } from "../Icons";
import type {
  SyncOutcome,
  SyncOutcomeAction,
  SyncOutcomeRowStatus,
} from "./syncOutcome";

const ROW_PRESENTATION: Record<
  SyncOutcomeRowStatus,
  { icon: IconName; className: string; label: string }
> = {
  synced: { icon: "check-circle", className: "synced", label: "Synced" },
  held: { icon: "git-compare-arrows", className: "diverged", label: "Held for Pull" },
  preserved: { icon: "git-fork", className: "preserved", label: "Kept independently" },
  failed: { icon: "octagon-x", className: "unavailable", label: "Failed" },
  not_pulled: { icon: "git-compare-arrows", className: "diverged", label: "Not pulled" },
};

function actionIcon(action: SyncOutcomeAction): IconName {
  if (action.kind === "open_pull") return "download";
  return "refresh";
}

function actionTitle(action: SyncOutcomeAction): string | undefined {
  if (action.kind === "after_pull_action") return action.title;
  return undefined;
}

function actionKey(action: SyncOutcomeAction, index: number): string {
  return action.kind === "after_pull_action"
    ? `${action.kind}:${action.offer.provider}:${action.offer.action_id}:${index}`
    : `${action.kind}:${index}`;
}

function formatTime(seconds: number): string {
  return new Date(seconds * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    ...clockOptions(),
  });
}

/**
 * A completed project sync rendered as a full-width section of its storage
 * card. The summary announces the result; the per-session rows stay behind a
 * disclosure so assistive technology does not repeat the entire result list.
 */
export default function SyncOutcomeNotice({
  outcome,
  busy = false,
  onAction,
  onDismiss,
}: {
  outcome: SyncOutcome;
  busy?: boolean;
  onAction?: (action: SyncOutcomeAction) => void;
  onDismiss: () => void;
}) {
  const [open, setOpen] = useState(outcome.defaultOpen);
  const rendered = useRef(outcome);
  if (rendered.current !== outcome) {
    rendered.current = outcome;
    setOpen(outcome.defaultOpen);
  }
  const bodyId = `sync-outcome-body-${useId().replace(/:/g, "")}`;
  const runnableActions = outcome.actions.filter((action) => (
    action.kind !== "after_pull_action" || action.offer.available
  ));
  const manualActions = outcome.actions.filter((action): action is Extract<
    SyncOutcomeAction,
    { kind: "after_pull_action" }
  > => (
    action.kind === "after_pull_action" && !action.offer.available
  ));

  return (
    <div
      className={`sync-outcome-notice ${outcome.tone}${open ? "" : " collapsed"}`}
    >
      <div className="sync-outcome-head">
        <Icon name={outcome.tone === "success" ? "check-circle" : "alert-triangle"} size={18} />
        <span className="sync-outcome-summary" role="status">
          <strong>{outcome.title}</strong>
          <span className="sync-outcome-stats">
            {outcome.stats.map((stat) => (
              <span key={`${stat.label}:${stat.value ?? ""}`} className="sync-outcome-stat">
                <Icon name={stat.icon} size={12} />
                {stat.lead
                  ? <>{stat.label}<b>{stat.value}</b></>
                  : <>{stat.value != null && <b>{stat.value}</b>}{stat.label}</>}
              </span>
            ))}
          </span>
        </span>
        {runnableActions.length > 0 && onAction && (
          <span className="sync-outcome-actions">
            {runnableActions.map((action, index) => (
              <button
                key={actionKey(action, index)}
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                title={actionTitle(action)}
                onClick={() => onAction(action)}
              >
                <Icon name={actionIcon(action)} size={13} />
                {action.label}
              </button>
            ))}
          </span>
        )}
        <span className="sync-outcome-head-actions">
          <button
            type="button"
            className="sync-history-commit-toggle"
            aria-expanded={open}
            aria-controls={bodyId}
            aria-label={`${open ? "Collapse" : "Expand"} the result list`}
            onClick={() => setOpen((value) => !value)}
          >
            <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />
          </button>
          <button
            type="button"
            className="sync-notice-dismiss"
            title="Dismiss"
            aria-label="Dismiss"
            onClick={onDismiss}
          >
            <Icon name="x" size={12} />
          </button>
        </span>
      </div>
      {outcome.message && (
        <div className="sync-outcome-guidance">
          <p>{outcome.message}</p>
        </div>
      )}
      {manualActions.length > 0 && (
        <div className="sync-outcome-guidance">
          {manualActions.map((action, index) => (
            <p key={actionKey(action, index)}>{action.title}</p>
          ))}
        </div>
      )}
      <div className="sync-outcome-body" id={bodyId}>
        <div className="sync-history-thread-list sync-outcome-threads">
          {outcome.rows.map((row) => {
            const presentation = ROW_PRESENTATION[row.status];
            return (
              <article key={row.key} className="sync-history-thread-card">
                <div className="sync-history-thread-topline">
                  <div className="sync-history-thread-title">
                    <span
                      className={`sync-thread-sync-indicator ${presentation.className}`}
                      data-tooltip={row.reason ? `${presentation.label}\n${row.reason}` : presentation.label}
                      aria-label={row.reason ? `${presentation.label}. ${row.reason}` : presentation.label}
                      tabIndex={0}
                    >
                      <Icon name={presentation.icon} size={13} />
                    </span>
                    <strong>{row.title}</strong>
                    {row.reason && (
                      <span
                        className="sync-history-thread-metric sync-outcome-why"
                        data-tooltip={row.reason}
                        aria-label={row.reason}
                        tabIndex={0}
                      >
                        <Icon name="info" size={13} />
                      </span>
                    )}
                  </div>
                </div>
                {(row.threadId || row.updatedAt != null) && (
                  <div className="sync-history-session-details">
                    <div className="sync-history-session-summary">
                      <div className="sync-history-thread-meta">
                        {row.threadId && (
                          <span className="sync-outcome-threadid">
                            <code>{row.threadId}</code>
                          </span>
                        )}
                        {row.updatedAt != null && (
                          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end">
                            <time dateTime={new Date(row.updatedAt * 1_000).toISOString()}>
                              {formatTime(row.updatedAt)}
                            </time>
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                )}
              </article>
            );
          })}
        </div>
        {outcome.tone === "warning" && outcome.technical.length > 0 && (
          <details className="sync-outcome-technical">
            <summary>Technical results</summary>
            <dl>
              {outcome.technical.map((entry) => (
                <div key={`${entry.label}:${entry.value}`}>
                  <dt>{entry.label}</dt>
                  <dd>{entry.value}</dd>
                </div>
              ))}
            </dl>
          </details>
        )}
      </div>
    </div>
  );
}
