import { useId, useRef, useState } from "react";
import Icon, { type IconName } from "../Icons";
import type { SyncOutcome, SyncOutcomeRowStatus } from "./syncOutcome";

const ROW_PRESENTATION: Record<SyncOutcomeRowStatus, { icon: IconName; className: string; label: string }> = {
  synced: { icon: "check-circle", className: "synced", label: "Synced" },
  held: { icon: "git-compare-arrows", className: "diverged", label: "Held for Pull" },
  forked: { icon: "git-fork", className: "diverged-forked", label: "Forked" },
  failed: { icon: "octagon-x", className: "unavailable", label: "Failed" },
};

function formatTime(seconds: number): string {
  return new Date(seconds * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * The completed Push or Pull, as a section of the storage card: full width, no
 * outline of its own, one divider above, and its tint to say where it starts.
 * The summary line reads without opening; the sessions are lines inside the
 * same box, divided rather than boxed. The fold control is the history table's
 * own toggle — chevron-down closed, chevron-up open.
 */
export default function SyncOutcomeNotice({
  outcome,
  busy = false,
  onAction,
  onDismiss,
}: {
  outcome: SyncOutcome;
  busy?: boolean;
  onAction: (outcome: SyncOutcome) => void;
  onDismiss: () => void;
}) {
  const [open, setOpen] = useState(outcome.defaultOpen);
  // A new outcome re-arms the fold to its own default: a clean push after a
  // warning pull must fold itself away even though the component survives.
  const rendered = useRef(outcome);
  if (rendered.current !== outcome) {
    rendered.current = outcome;
    setOpen(outcome.defaultOpen);
  }
  const bodyId = `sync-outcome-body-${useId().replace(/:/g, "")}`;

  return (
    <div
      role="status"
      className={`sync-callout sync-outcome-notice ${outcome.tone}${open ? "" : " collapsed"}`}
    >
      <div className="sync-outcome-head">
        <Icon name={outcome.tone === "success" ? "check-circle" : "alert-triangle"} size={18} />
        <span className="sync-outcome-summary">
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
        {outcome.action && (
          <span className="sync-outcome-actions">
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy}
              title={outcome.action.kind === "restart_codex" ? outcome.action.title : undefined}
              onClick={() => onAction(outcome)}
            >
              <Icon name={outcome.action.kind === "open_pull" ? "download" : "refresh"} size={13} />
              {outcome.action.label}
            </button>
          </span>
        )}
        <span className="sync-outcome-head-actions">
          <button
            type="button"
            className="sync-history-commit-toggle"
            aria-expanded={open}
            aria-controls={bodyId}
            aria-label={`${open ? "Collapse" : "Expand"} the session list`}
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
      <div className="sync-outcome-body" id={bodyId}>
        <div className="sync-history-thread-list sync-outcome-threads">
          {outcome.rows.map((row) => {
            const presentation = ROW_PRESENTATION[row.status];
            const forkCount = row.forkCount ?? 0;
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
                    {row.reason && <span className="sync-outcome-reason">{row.reason}</span>}
                  </div>
                </div>
                {(row.threadId || row.updatedAt != null || forkCount > 0) && (
                  <div className="sync-history-session-details">
                    <div className="sync-history-session-summary">
                      {/* Line 2 is which one it is: the whole thread id — never
                          an 8-character stub — with the fork provenance and the
                          stamp beside it. */}
                      <div className="sync-history-thread-meta">
                        {row.threadId && (
                          <span className="sync-outcome-threadid">
                            <code>{row.threadId}</code>
                          </span>
                        )}
                        {forkCount > 0 && (
                          <span
                            className="sync-outcome-forkmark"
                            title={`Forked ${forkCount} ${forkCount === 1 ? "time" : "times"} before this sync`}
                            aria-label={`Forked ${forkCount} ${forkCount === 1 ? "time" : "times"}`}
                          >
                            <Icon name="git-fork" size={11} />
                            {forkCount}
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
          <details className="sync-fork-technical">
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
