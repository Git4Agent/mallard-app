import type { ReactNode, RefObject } from "react";
import type { ProjectSessionDetail, SessionActionDescriptor } from "../../../types";
import Icon, { type IconName } from "../../Icons";
import {
  formatCompactCount,
  formatSessionCount,
  formatSessionDuration,
  formatSessionRange,
} from "./sessionDetailModel";

const ACTION_ICONS: Record<SessionActionDescriptor["kind"], IconName> = {
  open_in_app: "play",
  open_in_terminal: "terminal",
};

interface Props {
  detail: ProjectSessionDetail;
  headingRef: RefObject<HTMLHeadingElement | null>;
  actionBusyId: string | null;
  actionError: string | null;
  onRefresh: () => void;
  onAction: (action: SessionActionDescriptor) => void;
}

function MetaItem({ label, children, title }: {
  label: string;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className="session-detail-meta-item">
      <span className="session-detail-meta-label">{label}</span>
      <span className="session-detail-meta-value" title={title}>{children}</span>
    </span>
  );
}

export default function SessionDetailHeader({
  detail,
  headingRef,
  actionBusyId,
  actionError,
  onRefresh,
  onAction,
}: Props) {
  // No breadcrumb and no back button: the workspace path bar above carries the
  // trail, and its section crumb is the way back.
  const title = detail.title || "Untitled session";
  // For most providers the summary is derived from the opening prompt, which is
  // also the title — printing both put the same sentence on screen twice before
  // the transcript printed it a third time.
  const summary = detail.summary && detail.summary.trim() !== title.trim()
    ? detail.summary
    : null;
  const range = formatSessionRange(detail.started_at, detail.ended_at);
  const duration = formatSessionDuration(detail.started_at, detail.ended_at);

  return (
    <header className="session-detail-header">
      <div className="session-detail-heading-row">
        <div className="session-detail-heading-copy">
          <div className="session-detail-provider-line">
            <span className="session-detail-provider-badge">{detail.provider_display_name}</span>
            {detail.is_active && <span className="session-detail-active-badge">Active</span>}
          </div>
          <h1 id="session-detail-heading" ref={headingRef} tabIndex={-1}>{title}</h1>
          {summary && <p>{summary}</p>}
        </div>

        <div className="session-detail-header-actions" aria-label="Session actions">
          {detail.actions.map((action) => {
            const pending = actionBusyId === action.action_id;
            return (
              <button
                key={action.action_id}
                type="button"
                className="btn btn-ghost"
                disabled={!action.enabled || actionBusyId !== null}
                title={!action.enabled ? action.disabled_reason ?? action.label : action.label}
                onClick={() => onAction(action)}
              >
                <Icon
                  name={pending ? "refresh" : ACTION_ICONS[action.kind]}
                  size={12}
                  className={pending ? "icon-spin" : undefined}
                />
                {action.label}
              </button>
            );
          })}
          {/* Refresh is maintenance, not one of the two things you came to do,
              so it keeps the column without competing for the same reading. */}
          <button
            type="button"
            className="btn btn-ghost session-detail-refresh"
            aria-label="Refresh session detail"
            title="Refresh session detail"
            onClick={onRefresh}
          >
            <Icon name="refresh" size={12} />
          </button>
        </div>
      </div>

      <div className="session-detail-metadata" aria-label="Session metadata">
        {detail.branch && (
          <MetaItem label="Branch"><code>{detail.branch}</code></MetaItem>
        )}
        {detail.recorded_sha && (
          <MetaItem label="Commit" title={detail.recorded_sha}>
            <code>#{detail.recorded_sha.slice(0, 7)}</code>
          </MetaItem>
        )}
        {range && (
          <MetaItem label="Ran">
            <time dateTime={detail.started_at != null ? String(detail.started_at) : undefined}>{range}</time>
            {duration && <b className="session-detail-meta-duration">{duration}</b>}
          </MetaItem>
        )}
        <MetaItem
          label="Tokens"
          title={detail.tokens.explanation ?? formatSessionCount(detail.tokens)}
        >
          <code className={`quality-${detail.tokens.quality}`}>{formatCompactCount(detail.tokens)}</code>
        </MetaItem>
      </div>

      {actionError && (
        <div className="sync-callout error session-detail-action-error" role="alert">
          <Icon name="alert-triangle" size={12} />
          <span>{actionError}</span>
        </div>
      )}
    </header>
  );
}
