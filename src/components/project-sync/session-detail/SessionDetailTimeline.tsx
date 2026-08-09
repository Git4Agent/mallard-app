import type { SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import SessionDetailEvent, { type SessionTurnToolState } from "./SessionDetailEvent";
import type { SessionToolDetailState } from "./SessionDetailToolCall";

interface Props {
  events: SessionTimelineEvent[];
  providerDisplayName: string;
  loading: boolean;
  loadingMore: boolean;
  nextCursor: string | null;
  error: string | null;
  expandedToolIds: ReadonlySet<string>;
  toolDetails: Readonly<Record<string, SessionToolDetailState>>;
  turnToolStates: Readonly<Record<string, SessionTurnToolState>>;
  onRetry: () => void;
  onLoadMore: () => void;
  onToolExpandedChange: (eventId: string, expanded: boolean) => void;
  onTurnToolExpandedChange: (turnId: string, expanded: boolean) => void;
  onLoadMoreTurnTools: (turnId: string) => void;
}

export default function SessionDetailTimeline({
  events,
  providerDisplayName,
  loading,
  loadingMore,
  nextCursor,
  error,
  expandedToolIds,
  toolDetails,
  turnToolStates,
  onRetry,
  onLoadMore,
  onToolExpandedChange,
  onTurnToolExpandedChange,
  onLoadMoreTurnTools,
}: Props) {
  return (
    <section className="session-detail-timeline" aria-labelledby="session-detail-timeline-heading">
      <div className="session-detail-section-heading">
        <div>
          <span>Chronological record</span>
          <h2 id="session-detail-timeline-heading">Timeline</h2>
        </div>
        {events.length > 0 && <code>{events.length} loaded</code>}
      </div>

      {loading && events.length === 0 && (
        <div className="session-detail-state" role="status" aria-live="polite">
          <span className="status-loader" />
          <strong>Loading the filtered timeline…</strong>
          <span>Conversation text and tool bodies are read only when needed.</span>
        </div>
      )}

      {error && (
        <div className="session-detail-state error" role="alert">
          <Icon name="alert-triangle" size={17} />
          <strong>Timeline unavailable</strong>
          <span>{error}</span>
          <button type="button" className="btn btn-ghost" onClick={onRetry}>Try again</button>
        </div>
      )}

      {!loading && !error && events.length === 0 && (
        <div className="session-detail-state empty">
          <Icon name="message" size={18} />
          <strong>No events match these filters.</strong>
          <span>Enable another event kind in View options.</span>
        </div>
      )}

      {events.length > 0 && (
        <ol className="session-detail-event-list">
          {events.map((event) => (
            <li key={event.event_id}>
              <SessionDetailEvent
                event={event}
                providerDisplayName={providerDisplayName}
                expandedToolIds={expandedToolIds}
                toolDetails={toolDetails}
                turnToolState={event.turn_id ? turnToolStates[event.turn_id] : undefined}
                onToolExpandedChange={onToolExpandedChange}
                onTurnToolExpandedChange={onTurnToolExpandedChange}
                onLoadMoreTurnTools={onLoadMoreTurnTools}
              />
            </li>
          ))}
        </ol>
      )}

      {nextCursor && !error && (
        <button
          type="button"
          className="btn session-detail-load-more"
          disabled={loadingMore}
          onClick={onLoadMore}
        >
          {loadingMore ? <span className="status-loader" /> : <Icon name="chevron-down" size={13} />}
          {loadingMore ? "Loading more…" : "Load 50 more"}
        </button>
      )}
    </section>
  );
}
