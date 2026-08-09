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
  eventAnchorId,
  formatSessionCount,
  formatSessionDetailTimestamp,
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
      <span className="session-detail-checkpoint-icon"><Icon name="git-commit" size={14} /></span>
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
        <Icon name="tool" size={12} />
        <strong>{formatSessionCount(summary.tool_calls)} tool calls</strong>
        {summary.tool_calls.quality !== "exact" && (
          <span className={`session-detail-quality quality-${summary.tool_calls.quality}`}>
            {summary.tool_calls.quality === "lower_bound" ? "At least" : "Inferred"}
          </span>
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
            <Icon name="alert-triangle" size={13} />
            {state.error}
          </div>
        )}
        {state?.page?.events.map((toolEvent) => (
          <SessionDetailToolCall
            key={toolEvent.event_id}
            event={toolEvent}
            compact
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
  expandedToolIds,
  toolDetails,
  turnToolState,
  onToolExpandedChange,
  onTurnToolExpandedChange,
  onLoadMoreTurnTools,
}: Props) {
  const label = event.kind === "response" ? providerDisplayName : SESSION_EVENT_KIND_LABELS[event.kind];
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
      <span className="session-detail-rail-mark" aria-hidden="true" />

      {event.kind === "tool_call" ? (
        <SessionDetailToolCall
          event={event}
          expanded={expandedToolIds.has(event.event_id)}
          detailState={toolDetails[event.event_id]}
          onExpandedChange={onToolExpandedChange}
        />
      ) : event.kind === "checkpoint" ? (
        <>
          <div className="session-detail-event-heading">
            <h3>{label}</h3>
            <EventQuality event={event} />
            {event.timestamp != null && (
              <time dateTime={sessionDetailDateTime(event.timestamp)}>
                {formatSessionDetailTimestamp(event.timestamp)}
              </time>
            )}
          </div>
          <CheckpointBody event={event} />
        </>
      ) : (
        <>
          <div className="session-detail-event-heading">
            <h3>{label}</h3>
            <EventQuality event={event} />
            {event.timestamp != null && (
              <time dateTime={sessionDetailDateTime(event.timestamp)}>
                {formatSessionDetailTimestamp(event.timestamp)}
              </time>
            )}
          </div>
          <div className={`session-detail-event-copy ${event.kind === "prompt"
            ? "session-detail-prompt-card"
            : event.kind === "response"
              ? "session-detail-response-prose"
              : "session-detail-intermediate-prose"}`}>
            {event.text ? <SessionDetailMarkdown text={event.text} /> : (
              <p className="session-detail-event-empty">No visible text is available for this event.</p>
            )}
          </div>
        </>
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
    </article>
  );
}
