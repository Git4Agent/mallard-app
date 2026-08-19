import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { SessionCount, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import ProviderMark from "../../sync-review/ProviderMark";
import SessionDetailToolCall, { type SessionToolDetailState } from "./SessionDetailToolCall";
import {
  eventAnchorId,
  formatSessionCount,
  formatSessionDetailClock,
  isSafeExternalMarkdownUrl,
  projectMarkdownDisplayPath,
  type ProjectMarkdownImageState,
  safeMarkdownUrl,
  safeProjectMarkdownPath,
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
  onOpenProjectPath: (relativePath: string) => void;
  projectImages?: Readonly<Record<string, ProjectMarkdownImageState>>;
  onLoadProjectImage?: (relativePath: string) => void;
  projectRoot: string;
}

async function openMarkdownLink(url: string): Promise<void> {
  if (!isSafeExternalMarkdownUrl(url)) return;
  await openUrl(url);
}

/** Alt text (or the path) inside a bordered chip: the still-loading and
 * cannot-embed states share one calm shape instead of the webview's
 * broken-image glyph. */
function MarkdownImagePlaceholder({
  label,
  detail,
  loading,
}: {
  label: string;
  detail?: string;
  loading?: boolean;
}) {
  return (
    <span
      className={`session-detail-image-placeholder${loading ? " is-loading" : ""}`}
      role="img"
      aria-label={label}
      {...(detail ? { title: detail } : {})}
    >
      <Icon name="file" size={12} />
      <span className="session-detail-image-placeholder-label">{label}</span>
    </span>
  );
}

/** Requests one validated in-project image on mount, rendering the loading
 * chip until the page-level cache resolves it. */
function ProjectMarkdownImage({
  relativePath,
  alt,
  state,
  onLoadProjectImage,
}: {
  relativePath: string;
  alt: string;
  state: ProjectMarkdownImageState | undefined;
  onLoadProjectImage: (relativePath: string) => void;
}) {
  const settled = state != null;
  useEffect(() => {
    if (!settled) onLoadProjectImage(relativePath);
  }, [settled, relativePath, onLoadProjectImage]);
  if (state?.status === "loaded") {
    return <img className="session-detail-markdown-image" src={state.src} alt={alt} />;
  }
  if (state?.status === "failed") {
    return <MarkdownImagePlaceholder label={alt || relativePath} detail={state.message} />;
  }
  return <MarkdownImagePlaceholder label={alt || relativePath} loading />;
}

export function SessionDetailMarkdown({
  text,
  onOpenProjectPath,
  projectImages,
  onLoadProjectImage,
  projectRoot = "",
}: {
  text: string;
  onOpenProjectPath?: (relativePath: string) => void;
  projectImages?: Readonly<Record<string, ProjectMarkdownImageState>>;
  onLoadProjectImage?: (relativePath: string) => void;
  projectRoot?: string;
}) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      skipHtml
      urlTransform={safeMarkdownUrl}
      components={{
        img: ({ src, alt }) => {
          const url = typeof src === "string" ? src : "";
          const label = alt ?? "";
          // External https images render as-is, exactly as before this
          // renderer existed; the webview loads them directly.
          if (isSafeExternalMarkdownUrl(url)) {
            return (
              <img
                className="session-detail-markdown-image"
                src={url}
                alt={label}
                loading="lazy"
              />
            );
          }
          // Everything else must be a clean relative descendant of the
          // project root, served through the bounded validated command.
          // Absolute paths, traversal, and foreign schemes arrive here as
          // "" from the shared urlTransform and fall to the plain chip.
          const projectPath = url ? safeProjectMarkdownPath(url) : null;
          if (!projectPath || !onLoadProjectImage) {
            return (
              <MarkdownImagePlaceholder
                label={label || "image"}
                detail={projectPath
                  ? undefined
                  : "Only images inside the project can be embedded."}
              />
            );
          }
          return (
            <ProjectMarkdownImage
              relativePath={projectPath}
              alt={label}
              state={projectImages?.[projectPath]}
              onLoadProjectImage={onLoadProjectImage}
            />
          );
        },
        a: ({ href, children, ...props }) => {
          const safeHref = href ? safeMarkdownUrl(href) : "";
          if (!safeHref) return <span>{children}</span>;
          const external = isSafeExternalMarkdownUrl(safeHref);
          const projectPath = safeProjectMarkdownPath(safeHref);
          if (projectPath && !onOpenProjectPath) return <span>{children}</span>;
          const displayPath = projectPath
            ? projectMarkdownDisplayPath(projectRoot, projectPath)
            : null;
          const className = [
            props.className,
            projectPath ? "session-detail-project-link" : null,
          ].filter(Boolean).join(" ") || undefined;
          return (
            <a
              {...props}
              className={className}
              href={safeHref}
              rel={external ? "noopener noreferrer" : undefined}
              {...(displayPath ? {
                "aria-label": `Open ${displayPath}`,
                "data-tooltip": displayPath,
              } : {})}
              onClick={external
                ? (clickEvent) => {
                  clickEvent.preventDefault();
                  void openMarkdownLink(safeHref);
                }
                : projectPath
                  ? (clickEvent) => {
                    clickEvent.preventDefault();
                    onOpenProjectPath?.(projectPath);
                  }
                  : undefined}
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
  onOpenProjectPath,
  projectImages,
  onLoadProjectImage,
  projectRoot,
}: {
  event: SessionTimelineEvent;
  turnDuration?: string | null;
  onOpenProjectPath: (relativePath: string) => void;
  projectImages?: Readonly<Record<string, ProjectMarkdownImageState>>;
  onLoadProjectImage?: (relativePath: string) => void;
  projectRoot: string;
}) {
  const callCount = speakableCount(event.turn_tool_summary?.tool_calls);
  const clock = event.timestamp != null ? formatSessionDetailClock(event.timestamp) : null;
  return (
    <div className="session-detail-prompt-card">
      <div className="session-detail-event-copy">
        {event.text ? (
          <SessionDetailMarkdown
            text={event.text}
            onOpenProjectPath={onOpenProjectPath}
            projectImages={projectImages}
            onLoadProjectImage={onLoadProjectImage}
            projectRoot={projectRoot}
          />
        ) : (
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
  onOpenProjectPath,
  projectImages,
  onLoadProjectImage,
  projectRoot,
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
          <PromptCard
            event={event}
            turnDuration={turnDuration}
            onOpenProjectPath={onOpenProjectPath}
            projectImages={projectImages}
            onLoadProjectImage={onLoadProjectImage}
            projectRoot={projectRoot}
          />
        ) : (
          <div className={`session-detail-event-copy ${event.kind === "response"
            ? "session-detail-response-prose"
            : "session-detail-intermediate-prose"}`}>
            {event.text ? (
              <SessionDetailMarkdown
                text={event.text}
                onOpenProjectPath={onOpenProjectPath}
                projectImages={projectImages}
                onLoadProjectImage={onLoadProjectImage}
                projectRoot={projectRoot}
              />
            ) : (
              <p className="session-detail-event-empty">No visible text is available for this event.</p>
            )}
          </div>
        )}
      </div>
    </article>
  );
}
