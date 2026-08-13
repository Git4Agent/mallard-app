import type { RefObject } from "react";
import type { ProjectSessionDetail, SessionActionDescriptor } from "../../../types";
import Icon, { type IconName } from "../../Icons";
import {
  formatSessionCount,
  formatSessionDetailTimestamp,
  sessionDetailDateTime,
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

function HeaderTime({ label, value }: { label: string; value?: number | null }) {
  if (value == null) return null;
  return (
    <span className="session-detail-meta-item">
      <span>{label}</span>
      <time dateTime={sessionDetailDateTime(value)}>{formatSessionDetailTimestamp(value)}</time>
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
  return (
    <header className="session-detail-header">
      <div className="session-detail-heading-row">
        <div className="session-detail-heading-copy">
          <div className="session-detail-provider-line">
            <span className="session-detail-provider-badge">{detail.provider_display_name}</span>
            {detail.is_active && <span className="session-detail-active-badge">Active</span>}
          </div>
          <h1 id="session-detail-heading" ref={headingRef} tabIndex={-1}>
            {detail.title || "Untitled session"}
          </h1>
          {detail.summary && <p>{detail.summary}</p>}
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
                  size={14}
                  className={pending ? "icon-spin" : undefined}
                />
                {action.label}
              </button>
            );
          })}
          <button
            type="button"
            className="btn btn-ghost session-detail-refresh"
            onClick={onRefresh}
            title="Refresh session detail"
          >
            <Icon name="refresh" size={14} />
            Refresh
          </button>
        </div>
      </div>

      <div className="session-detail-metadata" aria-label="Session metadata">
        {detail.branch && (
          <span className="session-detail-meta-item">
            <Icon name="git-branch" size={12} />
            <span>Branch</span>
            <code>{detail.branch}</code>
          </span>
        )}
        {detail.recorded_sha && (
          <span className="session-detail-meta-item">
            <Icon name="git-commit" size={12} />
            <span>Recorded at</span>
            <code>{detail.recorded_sha.slice(0, 7)}</code>
          </span>
        )}
        <HeaderTime label="Started" value={detail.started_at} />
        <HeaderTime label={detail.is_active ? "Last activity" : "Ended"} value={detail.ended_at} />
        <span
          className={`session-detail-meta-item quality-${detail.tokens.quality}`}
          title={detail.tokens.explanation ?? undefined}
        >
          <Icon name="token" size={12} />
          <span>Tokens</span>
          <code>{formatSessionCount(detail.tokens)}</code>
        </span>
      </div>

      {actionError && (
        <div className="sync-callout error session-detail-action-error" role="alert">
          <Icon name="alert-triangle" size={14} />
          <span>{actionError}</span>
        </div>
      )}
    </header>
  );
}
