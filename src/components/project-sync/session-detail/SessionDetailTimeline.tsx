import type { SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import SessionDetailEvent from "./SessionDetailEvent";
import type { SessionToolDetailState } from "./SessionDetailToolCall";
import {
  formatSessionGap,
  groupSessionEventsByDay,
  sessionTurnDurationLabel,
  sessionTurnSpans,
} from "./sessionDetailModel";

interface Props {
  events: SessionTimelineEvent[];
  provider: string;
  providerDisplayName: string;
  loading: boolean;
  loadingMore: boolean;
  nextCursor: string | null;
  error: string | null;
  expandedToolIds: ReadonlySet<string>;
  toolDetails: Readonly<Record<string, SessionToolDetailState>>;
  onRetry: () => void;
  onLoadMore: () => void;
  onToolExpandedChange: (eventId: string, expanded: boolean) => void;
  onOpenProjectPath: (relativePath: string) => void;
  projectRoot: string;
}

export default function SessionDetailTimeline({
  events,
  provider,
  providerDisplayName,
  loading,
  loadingMore,
  nextCursor,
  error,
  expandedToolIds,
  toolDetails,
  onRetry,
  onLoadMore,
  onToolExpandedChange,
  onOpenProjectPath,
  projectRoot,
}: Props) {
  // The record is the page; it needs no eyebrow announcing that it is a record,
  // and no count of its own now that the toolbar states one.
  const days = groupSessionEventsByDay(events);
  const spans = sessionTurnSpans(events);

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
          {/* h2, directly under the page's h1: an h3 here implied a section
              level that does not exist (WCAG 1.3.1). */}
          <h2 className="session-detail-day-divider"><span>{day.label}</span></h2>
          <ol className="session-detail-event-list">
            {day.events.map((event, index) => {
              const previousDay = days[dayIndex - 1]?.events;
              const previousTimestamp = index === 0
                ? previousDay?.[previousDay.length - 1]?.timestamp
                : day.events[index - 1].timestamp;
              // The wait sits on the rail between the events it separates.
              // Ticks stay quiet: a wait before a call reads as the call's own
              // runtime, not the transcript stalling.
              const gap = event.kind !== "tool_call"
                ? formatSessionGap(previousTimestamp, event.timestamp)
                : null;
              return (
                <li
                  key={event.event_id}
                  className={event.kind === "tool_call" ? "session-detail-row-tool" : undefined}
                >
                  {gap && (
                    <div className="session-detail-event-gap"><span>{gap}</span></div>
                  )}
                  <SessionDetailEvent
                    event={event}
                    provider={provider}
                    providerDisplayName={providerDisplayName}
                    turnDuration={event.kind === "prompt" && event.turn_id
                      ? sessionTurnDurationLabel(spans.get(event.turn_id))
                      : null}
                    expandedToolIds={expandedToolIds}
                    toolDetails={toolDetails}
                    onToolExpandedChange={onToolExpandedChange}
                    onOpenProjectPath={onOpenProjectPath}
                    projectRoot={projectRoot}
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
