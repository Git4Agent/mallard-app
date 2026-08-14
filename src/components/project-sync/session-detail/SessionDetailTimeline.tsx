import type { SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import SessionDetailEvent, { type SessionTurnToolState } from "./SessionDetailEvent";
import type { SessionToolDetailState } from "./SessionDetailToolCall";
import { groupSessionEventsByDay } from "./sessionDetailModel";

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
  // The record is the page; it needs no eyebrow announcing that it is a record,
  // and no count of its own now that the toolbar states one.
  const days = groupSessionEventsByDay(events);

  return (
    <section className="session-detail-timeline" aria-label="Session timeline">
      {loading && events.length === 0 && (
        <div className="session-detail-state" role="status" aria-live="polite">
          <span className="status-loader" />
          <strong>Loading the filtered timeline…</strong>
          <span>Conversation text and tool bodies are read only when needed.</span>
        </div>
      )}

      {error && (
        <div className="session-detail-state error" role="alert">
          <Icon name="alert-triangle" size={24} />
          <strong>Timeline unavailable</strong>
          <span>{error}</span>
          <button type="button" className="btn btn-ghost" onClick={onRetry}>Try again</button>
        </div>
      )}

      {!loading && !error && events.length === 0 && (
        <div className="session-detail-state empty">
          <Icon name="message" size={24} />
          <strong>No events match these filters.</strong>
          <span>Turn another kind back on in the row above.</span>
        </div>
      )}

      {days.map((day, dayIndex) => (
        <div className="session-detail-day" key={`${day.key}:${dayIndex}`}>
          <h3 className="session-detail-day-divider"><span>{day.label}</span></h3>
          <ol className="session-detail-event-list">
            {day.events.map((event, index) => {
              const previousDay = days[dayIndex - 1]?.events;
              return (
              <li key={event.event_id}>
                <SessionDetailEvent
                  event={event}
                  providerDisplayName={providerDisplayName}
                  previousTimestamp={index === 0
                    ? previousDay?.[previousDay.length - 1]?.timestamp
                    : day.events[index - 1].timestamp}
                  expandedToolIds={expandedToolIds}
                  toolDetails={toolDetails}
                  turnToolState={event.turn_id ? turnToolStates[event.turn_id] : undefined}
                  onToolExpandedChange={onToolExpandedChange}
                  onTurnToolExpandedChange={onTurnToolExpandedChange}
                  onLoadMoreTurnTools={onLoadMoreTurnTools}
                />
              </li>
              );
            })}
          </ol>
        </div>
      ))}

      {nextCursor && !error && (
        <button
          type="button"
          className="btn session-detail-load-more"
          disabled={loadingMore}
          onClick={onLoadMore}
        >
          {loadingMore ? <span className="status-loader" /> : <Icon name="chevron-down" size={12} />}
          {loadingMore ? "Loading more…" : "Load 50 more"}
        </button>
      )}
    </section>
  );
}
