import type { SessionMessageSettings, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import Notice from "../../notice/Notice";
import SessionDetailEvent from "./SessionDetailEvent";
import { SessionModelNote, sessionModelNotes } from "./SessionModel";
import type { SessionToolDetailState } from "./SessionDetailToolCall";
import {
  formatSessionGap,
  groupSessionEventsByDay,
  type ProjectMarkdownMediaState,
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
  projectMedia: Readonly<Record<string, ProjectMarkdownMediaState>>;
  onLoadProjectMedia: (relativePath: string) => void;
  projectRoot: string;
  repositoryUrl?: string | null;
  /** False when the session holds no events at all, so the empty state can
   *  stop blaming filters that do not exist. */
  hasFilterToggles: boolean;
  /** Opens an agent's transcript as its own session page. */
  onOpenSession?: (resourceId: string) => void;
  /** What the session started with; a message that differs gets a note. */
  startSettings?: SessionMessageSettings | null;
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
  projectMedia,
  onLoadProjectMedia,
  projectRoot,
  repositoryUrl,
  hasFilterToggles,
  onOpenSession,
  startSettings,
}: Props) {
  const days = groupSessionEventsByDay(events);
  const spans = sessionTurnSpans(events);
  const modelNotes = sessionModelNotes(events, startSettings);

  return (
    <section className="session-detail-timeline" aria-label="Session timeline">
      {/* Both of these land after the view's first commit — the first page
          request fires from an effect once the detail has resolved — so they
          are events, not content the timeline renders with, and live regions
          are the right shape. `role="status"` already carries
          `aria-live="polite"`; do not duplicate the attribute. */}
      {loading && (
        <div className="session-detail-state" role="status">
          <span className="status-loader" />
          <strong>Loading the filtered timeline…</strong>
          <span>Conversation text and tool bodies are read only when needed.</span>
        </div>
      )}

      {/* A failed page leaves the transcript with nothing to show, so it stops
          the flow rather than advising on it: `blocking`, and the box prints
          the severity as a word instead of relying on tint and glyph alone. */}
      {error && (
        <Notice
          severity="danger"
          timing="blocking"
          title="Timeline unavailable"
          action={<button type="button" className="btn btn-ghost" onClick={onRetry}>Try again</button>}
        >
          {error}
        </Notice>
      )}

      {!loading && !error && events.length === 0 && (
        <div className="session-detail-state empty">
          <Icon name="message" size={24} />
          {hasFilterToggles ? (
            <>
              <strong>No events match these filters.</strong>
              <span>Turn another kind back on in the row above.</span>
            </>
          ) : (
            <>
              <strong>This session recorded no events.</strong>
              <span>Nothing was captured for it, so there is nothing to filter.</span>
            </>
          )}
        </div>
      )}

      <div className="session-detail-days" aria-busy={loading || undefined} data-refiltering={loading || undefined}>
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
                  className={event.kind === "tool_call"
                    ? "session-detail-row-tool"
                    : event.kind === "agent_call"
                      ? "session-detail-row-agent"
                      : undefined}
                >
                  {gap && (
                    <div className="session-detail-event-gap"><span>{gap}</span></div>
                  )}
                  {/* A change of model or thinking level, before the message
                      it applies from. */}
                  {modelNotes.has(event.event_id) && (
                    <SessionModelNote eventId={event.event_id} settings={modelNotes.get(event.event_id)!} />
                  )}
                  <SessionDetailEvent
                    onOpenSession={onOpenSession}
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
                    projectMedia={projectMedia}
                    onLoadProjectMedia={onLoadProjectMedia}
                    projectRoot={projectRoot}
                    repositoryUrl={repositoryUrl}
                  />
                </li>
              );
            })}
          </ol>
        </div>
      ))}
      </div>

      {nextCursor && !error && (
        <button
          type="button"
          className="btn session-detail-load-more"
          disabled={loadingMore || loading}
          onClick={onLoadMore}
        >
          {loadingMore ? <span className="status-loader" /> : <Icon name="chevron-down" size={12} />}
          {loadingMore ? "Loading more…" : "Load 50 more"}
        </button>
      )}
    </section>
  );
}
