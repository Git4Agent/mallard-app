import type { ReactNode, RefObject } from "react";
import type { ProjectSessionDetail, SessionActionDescriptor, SessionDetailWarning } from "../../../types";
import Icon, { type IconName } from "../../Icons";
import ProviderMark from "../../sync-review/ProviderMark";
import ThreadStatusIcon from "../../sync-review/ThreadStatusIcon";
import PageBack, { type PageBackTarget } from "../../sync-review/PageBack";
import SessionDetailNotes from "./SessionDetailNotes";
import { SessionModelChip } from "./SessionModel";
import {
  formatCompactCount,
  formatSessionCount,
  formatSessionDuration,
  formatSessionLengthDetail,
  formatSessionStart,
  sessionDetailDateTime,
  type SessionDetailSource,
  type SessionPushGate,
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
  back,
  headingRef,
  actionBusyId,
  onAction,
  pushGate,
  notes,
  onShowToolCalls,
  source,
}: {
  detail: ProjectSessionDetail;
  back: PageBackTarget;
  headingRef: RefObject<HTMLHeadingElement | null>;
  actionBusyId: string | null;
  onAction: (action: SessionActionDescriptor) => void;
  /** A hold on Push the user clears; null when Push says all there is. */
  pushGate: SessionPushGate | null;
  /** How the transcript was read. */
  notes: SessionDetailWarning[];
  onShowToolCalls: (() => void) | null;
  /** Where it was read from: this computer, or the storage a stored row
   *  sits under. */
  source: SessionDetailSource;
}) {
  // The page's visible title is the path bar's leaf crumb — stated once, and
  // sticky, so it survives scrolling in a way an in-page poster never did. The
  // h1 keeps the page's structure: it is what the section labels itself by and
  // where focus lands on open, so it stays in the DOM, visually hidden.
  const title = detail.title || "Untitled session";
  const start = formatSessionStart(detail.started_at);
  const length = formatSessionDuration(detail.started_at, detail.ended_at);
  const tokensKnown = detail.tokens.quality !== "unsupported" && detail.tokens.value != null;
  const agents = detail.event_counts.agent_calls;
  const agentsKnown = agents.quality !== "unsupported" && agents.value != null && agents.value > 0;
  const agent = detail.agent?.preview ?? null;

  return (
    <header className="session-detail-header">
      <h1
        id="session-detail-heading"
        className="sync-visually-hidden"
        ref={headingRef}
        tabIndex={-1}
      >{title}</h1>

      <div className="session-detail-strip">
        {/* The way back leads the row, and the facts wrap beside it. */}
        <div className="session-detail-lead">
          <PageBack target={back} />
          {/* Facts state themselves: a branch after the branch glyph and a
              7-char sha need no caption. Only the token count keeps a word. */}
          <div className="session-detail-chips" aria-label="Session facts">
            {/* A stored session's one difference from a local one, said the
                way its row says it: the row's glyph, with the row's sentence
                as the tooltip. The face names the fact, not the storage,
                which is the bar's root crumb. */}
            {source.kind === "stored" && (
              <FactChip title={`Only in ${source.storageName}. Pull to download it here.`}>
                <ThreadStatusIcon status="cloud-outstanding" size={14} />
                Not on this computer
              </FactChip>
            )}
            {detail.branch && (
              <FactChip title={`Recorded branch ${detail.branch}`}>
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
            {length && (
              <FactChip title={formatSessionLengthDetail(detail.started_at, detail.ended_at) ?? undefined}>
                <Icon name="clock" size={12} />
                <b>{length}</b>
              </FactChip>
            )}
            <FactChip title={detail.tokens.explanation ?? formatSessionCount(detail.tokens)}>
              {tokensKnown
                ? <><code className={`quality-${detail.tokens.quality}`}>{formatCompactCount(detail.tokens)}</code> tokens</>
                : "Tokens unavailable"}
            </FactChip>
            {/* What the session started with: `model · level`; the transcript
                notes where it changed. */}
            {detail.settings && <SessionModelChip settings={detail.settings} />}
            {/* How many agents this session delegated to; the rows say which. */}
            {agentsKnown && (
              <FactChip title={agents.explanation ?? `${formatSessionCount(agents)} agents spawned in this session`}>
                <Icon name="git-fork" size={12} />
                <code className={`quality-${agents.quality}`}>{formatSessionCount(agents)}</code>
                {agents.value === 1 ? "agent" : "agents"}
              </FactChip>
            )}
            {/* A child page states what it was: its role, model, and how the
                parent ran it. The parent itself is the crumb before the title. */}
            {agent?.role && (
              <FactChip title={`Agent role ${agent.role}`}>
                <Icon name="git-fork" size={12} />
                <code>{agent.role}</code>
              </FactChip>
            )}
            {agent?.model && !detail.settings && <FactChip title={`Model ${agent.model}`}><code>{agent.model}</code></FactChip>}
            {agent && <FactChip title={`Run ${agent.mode}`}>{agent.mode}</FactChip>}
            <SessionDetailNotes notes={notes} onShowToolCalls={onShowToolCalls} />
          </div>
        </div>

        <div className="session-detail-header-actions" aria-label="Session actions">
          {/* Right under Push in the bar: why it is held, when the user can
              clear it. The sentence is the tooltip, and printed for a screen
              reader. */}
          {pushGate && (
            <span className={`session-detail-chip session-detail-push-gate ${pushGate.tone}`} title={pushGate.detail}>
              <Icon name={pushGate.tone === "danger" ? "octagon-x" : "download"} size={12} />
              {pushGate.label}
              <span className="sync-visually-hidden"> {pushGate.detail}</span>
            </span>
          )}
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
