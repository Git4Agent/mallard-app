import type { ReactNode, RefObject } from "react";
import type { ProjectSessionDetail, SessionActionDescriptor } from "../../../types";
import Icon, { type IconName } from "../../Icons";
import ProviderMark from "../../sync-review/ProviderMark";
import {
  formatCompactCount,
  formatSessionCount,
  formatSessionDuration,
  formatSessionLengthDetail,
  formatSessionStart,
  sessionDetailDateTime,
} from "./sessionDetailModel";

// The activity rows' vocabulary, reused verbatim: opening the provider wears
// the provider's own mark, everything else its action glyph. open_in_app is
// excluded because it renders <ProviderMark>, not an Icon.
const ACTION_ICONS: Record<Exclude<SessionActionDescriptor["kind"], "open_in_app">, IconName> = {
  open_in_terminal: "terminal",
};

function FactChip({ children, title }: { children: ReactNode; title?: string }) {
  return <span className="session-detail-chip" title={title}>{children}</span>;
}

export default function SessionDetailHeader({
  detail,
  headingRef,
  actionBusyId,
  onAction,
}: {
  detail: ProjectSessionDetail;
  headingRef: RefObject<HTMLHeadingElement | null>;
  actionBusyId: string | null;
  onAction: (action: SessionActionDescriptor) => void;
}) {
  // The page's visible title is the path bar's leaf crumb — stated once, and
  // sticky, so it survives scrolling in a way an in-page poster never did. The
  // h1 keeps the page's structure: it is what the section labels itself by and
  // where focus lands on open, so it stays in the DOM, visually hidden.
  const title = detail.title || "Untitled session";
  const start = formatSessionStart(detail.started_at);
  const length = formatSessionDuration(detail.started_at, detail.ended_at);
  const tokensKnown = detail.tokens.quality !== "unsupported" && detail.tokens.value != null;

  return (
    <header className="session-detail-header">
      <h1
        id="session-detail-heading"
        className="sync-visually-hidden"
        ref={headingRef}
        tabIndex={-1}
      >{title}</h1>

      <div className="session-detail-strip">
        {/* Facts state themselves: a branch after the branch glyph and a
            7-char sha need no caption. Only the token count keeps a word. */}
        <div className="session-detail-chips" aria-label="Session facts">
          {detail.branch && (
            <FactChip title={`Branch ${detail.branch}`}>
              <Icon name="git-branch" size={12} />
              <code>{detail.branch}</code>
            </FactChip>
          )}
          {detail.recorded_sha && (
            <FactChip title={detail.recorded_sha}>
              <Icon name="git-commit" size={12} />
              <code>{detail.recorded_sha.slice(0, 7)}</code>
            </FactChip>
          )}
          {/* Each chip carries one fact: start or duration. The duration
              tooltip includes the end time; the full start needs no tooltip. */}
          {start && (
            <FactChip>
              <time dateTime={sessionDetailDateTime(detail.started_at)}>{start}</time>
            </FactChip>
          )}
          {/* A live session has no end, so label it running. */}
          {(detail.is_active || length) && (
            <FactChip title={formatSessionLengthDetail(detail.started_at, detail.ended_at, detail.is_active) ?? undefined}>
              <Icon name={detail.is_active ? "activity" : "clock"} size={12} />
              <b>{detail.is_active ? "running" : length}</b>
            </FactChip>
          )}
          <FactChip title={detail.tokens.explanation ?? formatSessionCount(detail.tokens)}>
            {tokensKnown
              ? <><code className={`quality-${detail.tokens.quality}`}>{formatCompactCount(detail.tokens)}</code> tokens</>
              : "Tokens unavailable"}
          </FactChip>
        </div>

        <div className="session-detail-header-actions" aria-label="Session actions">
          {detail.actions.map((action) => {
            const pending = actionBusyId === action.action_id;
            return (
              <button
                key={action.action_id}
                type="button"
                className="btn btn-ghost session-detail-launch"
                disabled={!action.enabled || actionBusyId !== null}
                title={!action.enabled
                  ? action.disabled_reason ?? action.label
                  : pending ? `${action.label}…` : action.label}
                aria-label={action.label}
                onClick={() => onAction(action)}
              >
                {pending
                  ? <Icon name="refresh" size={12} className="icon-spin" />
                  : action.kind === "open_in_app"
                    ? <ProviderMark provider={detail.provider} displayName={detail.provider_display_name} size={12} />
                    : <Icon name={ACTION_ICONS[action.kind]} size={12} />}
              </button>
            );
          })}
        </div>
      </div>

    </header>
  );
}
