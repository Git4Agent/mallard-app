import { openUrl } from "@tauri-apps/plugin-opener";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type {
  ProjectSessionTimelinePage,
  SessionTimelineEvent,
} from "../../../types";
import Icon from "../../Icons";
import SessionDetailToolCall, { type SessionToolDetailState } from "./SessionDetailToolCall";
import {
  countSessionToolFailures,
  eventAnchorId,
  formatSessionCount,
  formatSessionDetailClock,
  formatSessionDetailTimestamp,
  formatSessionGap,
  isSafeExternalMarkdownUrl,
  safeMarkdownUrl,
  SESSION_EVENT_KIND_LABELS,
  sessionDetailDateTime,
} from "./sessionDetailModel";

export interface SessionTurnToolState {
  open: boolean;
  loading: boolean;
  error: string | null;
  page: ProjectSessionTimelinePage | null;
}

interface Props {
  event: SessionTimelineEvent;
  providerDisplayName: string;
  /** The event above this one, so the gutter can show the wait between them. */
  previousTimestamp?: number | null;
  expandedToolIds: ReadonlySet<string>;
  toolDetails: Readonly<Record<string, SessionToolDetailState>>;
  turnToolState?: SessionTurnToolState;
  onToolExpandedChange: (eventId: string, expanded: boolean) => void;
  onTurnToolExpandedChange: (turnId: string, expanded: boolean) => void;
  onLoadMoreTurnTools: (turnId: string) => void;
}

async function openMarkdownLink(url: string): Promise<void> {
  if (!isSafeExternalMarkdownUrl(url)) return;
  await openUrl(url);
}

export function SessionDetailMarkdown({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      urlTransform={safeMarkdownUrl}
      components={{
        a: ({ href, children, ...props }) => {
          const safeHref = href ? safeMarkdownUrl(href) : "";
          if (!safeHref) return <span>{children}</span>;
          const external = isSafeExternalMarkdownUrl(safeHref);
          return (
            <a
              {...props}
              href={safeHref}
              rel={external ? "noopener noreferrer" : undefined}
              onClick={external ? (clickEvent) => {
                clickEvent.preventDefault();
                void openMarkdownLink(safeHref);
              } : undefined}
            >
              {children}
            </a>
          );
        },
      }}
    >
      {text}
    </ReactMarkdown>
  );
}

function EventQuality({ event }: { event: SessionTimelineEvent }) {
  if (event.quality === "exact") return null;
  const label = event.quality === "lower_bound"
    ? "Partial"
    : event.quality === "inferred"
      ? "Inferred"
      : "Unavailable";
  return <span className={`session-detail-quality quality-${event.quality}`}>{label}</span>;
}

function CheckpointBody({ event }: { event: SessionTimelineEvent }) {
  const checkpoint = event.checkpoint;
  const subject = checkpoint?.subject ?? event.text ?? "Git checkpoint";
  const sha = checkpoint?.short_sha ?? checkpoint?.sha.slice(0, 7);
  return (
    <div className="session-detail-checkpoint-body">
      <span className="session-detail-checkpoint-icon"><Icon name="git-commit" size={12} /></span>
      <div>
        <strong>{subject}</strong>
        <span>
          {sha && <code title={checkpoint?.sha}>{sha}</code>}
          {checkpoint?.branch && <span>{checkpoint.branch}</span>}
          {checkpoint && (
            <time dateTime={sessionDetailDateTime(checkpoint.committed_at)}>
              {formatSessionDetailTimestamp(checkpoint.committed_at)}
            </time>
          )}
        </span>
      </div>
    </div>
  );
}

function TurnToolDisclosure({
  event,
  state,
  expandedToolIds,
  toolDetails,
  onToolExpandedChange,
  onExpandedChange,
  onLoadMore,
}: {
  event: SessionTimelineEvent;
  state?: SessionTurnToolState;
  expandedToolIds: ReadonlySet<string>;
  toolDetails: Readonly<Record<string, SessionToolDetailState>>;
  onToolExpandedChange: (eventId: string, expanded: boolean) => void;
  onExpandedChange: (turnId: string, expanded: boolean) => void;
  onLoadMore: (turnId: string) => void;
}) {
  const summary = event.turn_tool_summary;
  if (!summary) return null;
  const open = state?.open ?? false;
  // Only rows that have actually loaded can be judged, so the failure note
  // appears with them rather than being claimed up front.
  const failures = state?.page ? countSessionToolFailures(state.page.events) : 0;
  return (
    <details
      className="session-detail-turn-tools"
      open={open}
      onToggle={(toggleEvent) => {
        const next = toggleEvent.currentTarget.open;
        if (next !== open) onExpandedChange(summary.turn_id, next);
      }}
    >
      <summary>
        <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
        <strong>{formatSessionCount(summary.tool_calls)} tool calls</strong>
        {summary.tool_calls.quality !== "exact" && (
          <span className={`session-detail-quality quality-${summary.tool_calls.quality}`}>
            {summary.tool_calls.quality === "lower_bound" ? "At least" : "Inferred"}
          </span>
        )}
        {failures > 0 && (
          <span className="session-detail-turn-failures">{failures} failed</span>
        )}
      </summary>
      <div className="session-detail-turn-tool-list" aria-busy={state?.loading || undefined}>
        {state?.loading && !state.page && (
          <div className="session-detail-local-state" role="status">
            <span className="status-loader" /> Loading this turn's tool calls…
          </div>
        )}
        {state?.error && (
          <div className="session-detail-local-state error" role="alert">
            <Icon name="alert-triangle" size={12} />
            {state.error}
          </div>
        )}
        {state?.page?.events.map((toolEvent) => (
          <SessionDetailToolCall
            key={toolEvent.event_id}
            event={toolEvent}
            expanded={expandedToolIds.has(toolEvent.event_id)}
            detailState={toolDetails[toolEvent.event_id]}
            onExpandedChange={onToolExpandedChange}
          />
        ))}
        {state?.page && state.page.events.length === 0 && !state.loading && !state.error && (
          <p className="session-detail-tool-empty">Individual calls are unavailable for this turn.</p>
        )}
        {state?.page?.next_cursor && (
          <button
            type="button"
            className="btn btn-ghost session-detail-turn-load-more"
            disabled={state.loading}
            onClick={() => onLoadMore(summary.turn_id)}
          >
            {state.loading ? "Loading…" : "Load more calls"}
          </button>
        )}
      </div>
    </details>
  );
}

export default function SessionDetailEvent({
  event,
  providerDisplayName,
  previousTimestamp,
  expandedToolIds,
  toolDetails,
  turnToolState,
  onToolExpandedChange,
  onTurnToolExpandedChange,
  onLoadMoreTurnTools,
}: Props) {
  const speaker = event.kind === "response"
    ? providerDisplayName
    : SESSION_EVENT_KIND_LABELS[event.kind];
  const gap = formatSessionGap(previousTimestamp, event.timestamp);
  const classes = [
    "session-detail-event",
    `event-${event.kind.replace(/_/g, "-")}`,
    event.parent_event_id ? "has-parent" : "",
  ].filter(Boolean).join(" ");

  return (
    <article
      id={eventAnchorId(event.event_id)}
      className={classes}
      data-event-id={event.event_id}
      data-event-kind={event.kind}
      tabIndex={event.kind === "checkpoint" ? -1 : undefined}
    >
      {/* Clock time in a fixed column, the date stated once by the day divider
          above. What used to be `Aug 13, 2026 at 11:10 AM` on every row is now
          `11:10`, and the space it freed carries the wait since the last turn. */}
      <div className="session-detail-event-gutter">
        {event.timestamp != null ? (
          <time dateTime={sessionDetailDateTime(event.timestamp)}>
            {formatSessionDetailClock(event.timestamp)}
          </time>
        ) : (
          <span className="session-detail-event-undated">--:--</span>
        )}
        {gap && <span className="session-detail-event-gap">{gap}</span>}
      </div>

      <span className="session-detail-event-rail" aria-hidden="true" />

      <div className="session-detail-event-body">
        <div className="session-detail-event-heading">
          <h3>{speaker}</h3>
          <EventQuality event={event} />
        </div>

        {event.kind === "tool_call" ? (
          <SessionDetailToolCall
            event={event}
            expanded={expandedToolIds.has(event.event_id)}
            detailState={toolDetails[event.event_id]}
            onExpandedChange={onToolExpandedChange}
          />
        ) : event.kind === "checkpoint" ? (
          <CheckpointBody event={event} />
        ) : (
          <div className={`session-detail-event-copy ${event.kind === "prompt"
            ? "session-detail-prompt-card"
            : event.kind === "response"
              ? "session-detail-response-prose"
              : "session-detail-intermediate-prose"}`}>
            {event.text ? <SessionDetailMarkdown text={event.text} /> : (
              <p className="session-detail-event-empty">No visible text is available for this event.</p>
            )}
          </div>
        )}

        {event.kind !== "tool_call" && (
          <TurnToolDisclosure
            event={event}
            state={turnToolState}
            expandedToolIds={expandedToolIds}
            toolDetails={toolDetails}
            onToolExpandedChange={onToolExpandedChange}
            onExpandedChange={onTurnToolExpandedChange}
            onLoadMore={onLoadMoreTurnTools}
          />
        )}
      </div>
    </article>
  );
}
