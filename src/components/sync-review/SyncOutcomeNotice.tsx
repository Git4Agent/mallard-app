import { useId, useRef, useState } from "react";
import Icon, { type IconName } from "../Icons";

/** Per-row status treatment supplied by the owning feature's view model. */
export interface SyncOutcomeRowPresentation {
  icon: IconName;
  className: string;
  label: string;
}

/**
 * One outcome row. The shared renderer knows nothing about threads,
 * resources, or catalogs: an optional monospace identifier and timestamp are
 * the whole vocabulary.
 */
export interface SyncOutcomeRowView {
  key: string;
  title: string;
  presentation: SyncOutcomeRowPresentation;
  /** Why this row did not land the way the others did. */
  reason?: string | null;
  identifier?: string | null;
  updatedAt?: number | null;
}

export interface SyncOutcomeStatView {
  icon: IconName;
  value?: string;
  label: string;
  /** Renders "Revision 6" rather than "6 revision". */
  lead?: boolean;
}

export interface SyncOutcomeView {
  tone: "success" | "warning";
  title: string;
  stats: SyncOutcomeStatView[];
  rows: SyncOutcomeRowView[];
  action?: { label: string; icon: IconName; title?: string } | null;
  /** A clean result folds itself away; one holding a decision opens. */
  defaultOpen: boolean;
  /** Support detail, behind its own disclosure inside the fold. */
  technical: Array<{ label: string; value: string }>;
}

function formatTime(seconds: number): string {
  return new Date(seconds * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * The completed operation, as a section of the storage card: full width, no
 * outline of its own, one divider above, and its tint to say where it starts.
 * The summary line reads without opening; the rows are lines inside the same
 * box, divided rather than boxed. The fold control is the history table's own
 * toggle — chevron-down closed, chevron-up open.
 */
export default function SyncOutcomeNotice({
  view,
  busy = false,
  onAction,
  onDismiss,
}: {
  view: SyncOutcomeView;
  busy?: boolean;
  onAction?: () => void;
  onDismiss: () => void;
}) {
  const [open, setOpen] = useState(view.defaultOpen);
  // A new outcome re-arms the fold to its own default: a clean push after a
  // warning pull must fold itself away even though the component survives.
  const rendered = useRef(view);
  if (rendered.current !== view) {
    rendered.current = view;
    setOpen(view.defaultOpen);
  }
  const bodyId = `sync-outcome-body-${useId().replace(/:/g, "")}`;

  return (
    <div
      role="status"
      className={`sync-callout sync-outcome-notice ${view.tone}${open ? "" : " collapsed"}`}
    >
      <div className="sync-outcome-head">
        <Icon name={view.tone === "success" ? "check-circle" : "alert-triangle"} size={18} />
        <span className="sync-outcome-summary">
          <strong>{view.title}</strong>
          <span className="sync-outcome-stats">
            {view.stats.map((stat) => (
              <span key={`${stat.label}:${stat.value ?? ""}`} className="sync-outcome-stat">
                <Icon name={stat.icon} size={12} />
                {stat.lead
                  ? <>{stat.label}<b>{stat.value}</b></>
                  : <>{stat.value != null && <b>{stat.value}</b>}{stat.label}</>}
              </span>
            ))}
          </span>
        </span>
        {view.action && onAction && (
          <span className="sync-outcome-actions">
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy}
              title={view.action.title}
              onClick={onAction}
            >
              <Icon name={view.action.icon} size={13} />
              {view.action.label}
            </button>
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
      <div className="sync-outcome-body" id={bodyId}>
        <div className="sync-history-thread-list sync-outcome-threads">
          {view.rows.map((row) => {
            const presentation = row.presentation;
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
                    {/* The status icon already says what happened; the why is
                        a mark at the line's end carrying the words on hover,
                        not a sentence competing with the title. */}
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
                {(row.identifier || row.updatedAt != null) && (
                  <div className="sync-history-session-details">
                    <div className="sync-history-session-summary">
                      {/* Line 2 is which one it is: the whole identifier —
                          never a truncated stub — and the stamp beside it. */}
                      <div className="sync-history-thread-meta">
                        {row.identifier && (
                          <span className="sync-outcome-threadid">
                            <code>{row.identifier}</code>
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
        {view.tone === "warning" && view.technical.length > 0 && (
          <details className="sync-outcome-technical">
            <summary>Technical results</summary>
            <dl>
              {view.technical.map((entry) => (
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
