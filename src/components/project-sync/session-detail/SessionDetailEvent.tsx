import { openUrl } from "@tauri-apps/plugin-opener";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { SessionCount, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import ProviderMark from "../ProviderMark";
import SessionDetailToolCall, { type SessionToolDetailState } from "./SessionDetailToolCall";
import {
  eventAnchorId,
  formatSessionCount,
  formatSessionDetailClock,
  isSafeExternalMarkdownUrl,
  safeMarkdownUrl,
  SESSION_EVENT_KIND_LABELS,
  sessionDetailDateTime,
} from "./sessionDetailModel";

interface Props {
  event: SessionTimelineEvent;
  provider: string;
  providerDisplayName: string;
  /** This event's turn duration, shown on prompt cards once a later event of the turn has loaded. */
  turnDuration?: string | null;
  expandedToolIds: ReadonlySet<string>;
  toolDetails: Readonly<Record<string, SessionToolDetailState>>;
  onToolExpandedChange: (eventId: string, expanded: boolean) => void;
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

/**
 * The rail tile is the speaker: the headings that spelled `You` and `Codex` on
 * every event are gone, and the tile's label carries name and minute instead.
 * Prompts wear the user glyph, checkpoints the commit glyph, and the agent
 * wears its provider's own mark — the same registry the activity list draws
 * from, monogram fallback included.
 */
function EventNode({
  event,
  provider,
  providerDisplayName,
}: {
  event: SessionTimelineEvent;
  provider: string;
  providerDisplayName: string;
}) {
  const speaker = event.kind === "prompt"
    ? SESSION_EVENT_KIND_LABELS.prompt
    : event.kind === "checkpoint"
      ? SESSION_EVENT_KIND_LABELS.checkpoint
      : providerDisplayName || SESSION_EVENT_KIND_LABELS[event.kind];
  const label = event.timestamp != null
    ? `${speaker} · ${formatSessionDetailClock(event.timestamp)}`
    : speaker;
  return (
    <span className="session-detail-event-node" role="img" aria-label={label} title={label}>
      {event.kind === "prompt"
        ? <Icon name="user" size={12} />
        : event.kind === "checkpoint"
          ? <Icon name="git-commit" size={12} />
          : <ProviderMark provider={provider} displayName={providerDisplayName} size={12} />}
    </span>
  );
}

/** A count that can actually be spoken; `Unavailable calls` is not a meta line. */
function speakableCount(count: SessionCount | undefined): SessionCount | null {
  if (!count || count.value == null || count.quality === "unsupported") return null;
  return count;
}

/**
 * You ask in a card, the agent answers in prose — shape is the speaker. The
 * meta line reads clock · duration · calls: absolute clock because the divider
 * already owns the date, duration measured from the turn's loaded extent.
 */
function PromptCard({
  event,
  turnDuration,
}: {
  event: SessionTimelineEvent;
  turnDuration?: string | null;
}) {
  const callCount = speakableCount(event.turn_tool_summary?.tool_calls);
  const clock = event.timestamp != null ? formatSessionDetailClock(event.timestamp) : null;
  return (
    <div className="session-detail-prompt-card">
      <div className="session-detail-event-copy">
        {event.text ? <SessionDetailMarkdown text={event.text} /> : (
          <p className="session-detail-event-empty">No visible text is available for this event.</p>
        )}
      </div>
      {(clock || turnDuration || callCount) && (
        <div className="session-detail-card-meta">
          {clock && <time dateTime={sessionDetailDateTime(event.timestamp)}>{clock}</time>}
          {turnDuration && <span>{turnDuration}</span>}
          {callCount && (
            <span>{formatSessionCount(callCount)} {callCount.value === 1 ? "call" : "calls"}</span>
          )}
        </div>
      )}
    </div>
  );
}

/** sha · subject · branch · clock — the fields the provider records; no more. */
function CheckpointBody({ event }: { event: SessionTimelineEvent }) {
  const checkpoint = event.checkpoint;
  const subject = checkpoint?.subject ?? event.text ?? "Git checkpoint";
  const sha = checkpoint?.short_sha ?? checkpoint?.sha.slice(0, 7);
  return (
    <div className="session-detail-checkpoint-card">
      {sha && <code className="session-detail-checkpoint-sha" title={checkpoint?.sha}>{sha}</code>}
      <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
      {checkpoint && (
        <span className="session-detail-checkpoint-end">
          {checkpoint.branch && (
            <span className="session-detail-checkpoint-branch">{checkpoint.branch}</span>
          )}
          <time dateTime={sessionDetailDateTime(checkpoint.committed_at)}>
            {formatSessionDetailClock(checkpoint.committed_at)}
          </time>
        </span>
      )}
    </div>
  );
}

export default function SessionDetailEvent({
  event,
  provider,
  providerDisplayName,
  turnDuration,
  expandedToolIds,
  toolDetails,
  onToolExpandedChange,
}: Props) {
  const classes = [
    "session-detail-event",
    `event-${event.kind.replace(/_/g, "-")}`,
    event.kind === "tool_call" && event.tool ? `tool-${event.tool.status}` : "",
  ].filter(Boolean).join(" ");

  return (
    <article
      id={eventAnchorId(event.event_id)}
      className={classes}
      data-event-id={event.event_id}
      data-event-kind={event.kind}
      tabIndex={event.kind === "checkpoint" ? -1 : undefined}
    >
      {/* A tile per turn, a punch per call: runs of ticks read as bursts of
          work, and a failed call punches in the failure colour so it is
          findable from the rail alone. */}
      {event.kind === "tool_call"
        ? <span className="session-detail-event-tick" aria-hidden="true" />
        : <EventNode event={event} provider={provider} providerDisplayName={providerDisplayName} />}

      <div className="session-detail-event-body">
        {event.quality !== "exact" && (
          <div className="session-detail-event-flags"><EventQuality event={event} /></div>
        )}

        {event.kind === "tool_call" ? (
          <SessionDetailToolCall
            event={event}
            expanded={expandedToolIds.has(event.event_id)}
            detailState={toolDetails[event.event_id]}
            onExpandedChange={onToolExpandedChange}
          />
        ) : event.kind === "checkpoint" ? (
          <CheckpointBody event={event} />
        ) : event.kind === "prompt" ? (
          <PromptCard event={event} turnDuration={turnDuration} />
        ) : (
          <div className={`session-detail-event-copy ${event.kind === "response"
            ? "session-detail-response-prose"
            : "session-detail-intermediate-prose"}`}>
            {event.text ? <SessionDetailMarkdown text={event.text} /> : (
              <p className="session-detail-event-empty">No visible text is available for this event.</p>
            )}
          </div>
        )}
      </div>
    </article>
  );
}
