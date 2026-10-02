import { useId, useRef, useState } from "react";
import { clockOptions } from "../../clock";
import Icon, { type IconName } from "../Icons";
import { DismissButton } from "../notice/Notice";
import type { SyncOutcomeView, OutcomeAction, OutcomeActionPresentation, SyncOutcomeRowStatus } from "./outcome";

const ROW_PRESENTATION: Record<
  SyncOutcomeRowStatus,
  { icon: IconName; className: string; label: string }
> = {
  synced: { icon: "check-circle", className: "synced", label: "Synced" },
  held: { icon: "git-compare-arrows", className: "diverged", label: "Held for Pull" },
  preserved: { icon: "git-fork", className: "preserved", label: "Kept independently" },
  failed: { icon: "octagon-x", className: "unavailable", label: "Failed" },
  not_attempted: { icon: "git-compare-arrows", className: "unavailable", label: "Not attempted" },
  skipped: { icon: "git-compare-arrows", className: "unavailable", label: "Skipped" },
  not_pulled: { icon: "git-compare-arrows", className: "diverged", label: "Not pulled" },
};

function formatTime(seconds: number): string {
  return new Date(seconds * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    ...clockOptions(),
  });
}

/**
 * A completed sync rendered as a full-width section of its storage
 * card. The summary announces the result; the per-item rows stay behind a
 * disclosure so assistive technology does not repeat the entire result list.
 */
export default function SyncOutcomeNotice<A extends OutcomeAction>({
  outcome,
  busy = false,
  onAction,
  onDismiss,
  presentAction = (action, index) => ({ key: `${action.kind}:${index}`, icon: "refresh", available: true }),
}: {
  outcome: SyncOutcomeView<A>;
  busy?: boolean;
  onAction?: (action: A) => void;
  onDismiss: () => void;
  presentAction?: (action: A, index: number) => OutcomeActionPresentation;
}) {
  const [open, setOpen] = useState(outcome.defaultOpen);
  const rendered = useRef(outcome);
  if (rendered.current !== outcome) {
    rendered.current = outcome;
    setOpen(outcome.defaultOpen);
  }
  const bodyId = `sync-outcome-body-${useId().replace(/:/g, "")}`;
  /* A strip of the storage card in the notice recipe: the rung's word leads
     the title, the way every notice names its own severity. */
  const word = outcome.tone === "success" ? "Done" : "Warning";
  const presented = outcome.actions.map((action, index) => ({ action, presentation: presentAction(action, index) }));
  const runnableActions = presented.filter(({ presentation }) => presentation.available);
  const manualActions = presented.filter(({ presentation }) => !presentation.available);

  return (
    <div
      className={`sync-outcome-notice ${outcome.tone}${open ? "" : " collapsed"}`}
    >
      <div className="sync-outcome-head">
        <Icon name={outcome.tone === "success" ? "check-circle" : "alert-triangle"} size={13} className="notice-mark" />
        <span className="sync-outcome-summary" role="status">
          <strong><span className="notice-type">{word} ·</span> {outcome.title}</strong>
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
            {runnableActions.map(({ action, presentation }) => (
              <button
                key={presentation.key}
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                title={presentation.title}
                onClick={() => onAction(action)}
              >
                <Icon name={presentation.icon} size={13} />
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
          <DismissButton what={`${word}. ${outcome.title}`} onClick={onDismiss} />
        </span>
      </div>
      {outcome.message && (
        <div className="sync-outcome-guidance">
          <p>{outcome.message}</p>
        </div>
      )}
      {manualActions.length > 0 && (
        <div className="sync-outcome-guidance">
          {manualActions.map(({ presentation }) => (
            <p key={presentation.key}>{presentation.title}</p>
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
