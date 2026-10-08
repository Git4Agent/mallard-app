import { createContext, memo, useContext, useEffect, useMemo, useState } from "react";
import type { Components } from "react-markdown";
import type { Element } from "hast";
import MarkdownText, { MarkdownImagePlaceholder } from "../../markdown/MarkdownText";
import { isSafeExternalMarkdownUrl, safeMarkdownUrl, safeRelativeMarkdownPath } from "../../markdown/markdownUrl";
import type { SessionAgentPreview, SessionCount, SessionTimelineEvent } from "../../../types";
import Icon from "../../Icons";
import ProviderMark from "../../sync-review/ProviderMark";
import { githubCommitUrl, openExternalUrl } from "../externalLink";
import ClaudeSvgWidget from "../../widgets/ClaudeSvgWidget";
import { isClaudeWidgetTool } from "../../widgets/claudeWidget";
import { writeClipboardText } from "../../sync-review/clipboard";
import SessionDetailToolCall, {
  PayloadSection,
  type SessionToolDetailState,
} from "./SessionDetailToolCall";
import {
  eventAnchorId,
  formatCompactNumber,
  formatSessionCount,
  formatSessionDetailClock,
  formatSessionDuration,
  markdownMediaFailureMessage,
  markdownPlayerKind,
  projectMarkdownDisplayPath,
  type ProjectMarkdownMediaState,
  SESSION_AGENT_STATUS_LABELS,
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
  projectMedia?: Readonly<Record<string, ProjectMarkdownMediaState>>;
  onLoadProjectMedia?: (relativePath: string) => void;
  projectRoot: string;
  /** The project's GitHub remote, when it has one: a checkpoint sha links there. */
  repositoryUrl?: string | null;
  /** Opens another session page — an agent's transcript — in this workspace. */
  onOpenSession?: (resourceId: string) => void;
}

/* The page-level cache reaches each picture and player through context, not
   the renderers: a renderer rebuilt whenever one file settles is a new
   component type to ReactMarkdown, which would remount every player under it
   and stop a playing track. */
const ProjectMediaContext = createContext<Readonly<Record<string, ProjectMarkdownMediaState>> | undefined>(undefined);

/** Requests one validated in-project image on mount, rendering the loading
 * chip until the page-level cache resolves it. */
function ProjectMarkdownImage({
  relativePath,
  alt,
  onLoadProjectMedia,
}: {
  relativePath: string;
  alt: string;
  onLoadProjectMedia: (relativePath: string) => void;
}) {
  const state = useContext(ProjectMediaContext)?.[relativePath];
  const settled = state != null;
  useEffect(() => {
    if (!settled) onLoadProjectMedia(relativePath);
  }, [settled, relativePath, onLoadProjectMedia]);
  if (state?.status === "loaded") {
    return <img className="session-detail-markdown-image" src={state.src} alt={alt} />;
  }
  if (state?.status === "failed") {
    return <MarkdownImagePlaceholder label={alt || relativePath} detail={state.message} showDetail />;
  }
  return <MarkdownImagePlaceholder label={alt || relativePath} loading />;
}

/** A track or a clip: the webview's own player where a picture would be, bare
 * as pictures are drawn, read once through the validated command. A file the
 * player refuses after loading says so in the chip. */
function ProjectMarkdownPlayer({
  relativePath,
  kind,
  alt,
  onLoadProjectMedia,
}: {
  relativePath: string;
  kind: "audio" | "video";
  alt: string;
  onLoadProjectMedia: (relativePath: string) => void;
}) {
  const state = useContext(ProjectMediaContext)?.[relativePath];
  const settled = state != null;
  const [refused, setRefused] = useState<string | null>(null);
  useEffect(() => {
    if (!settled) onLoadProjectMedia(relativePath);
  }, [settled, relativePath, onLoadProjectMedia]);
  const label = alt || relativePath;
  if (state?.status === "loaded" && state.src !== refused) {
    const refuse = () => setRefused(state.src);
    return kind === "video"
      ? <video className="session-detail-markdown-video" controls playsInline preload="metadata" src={state.src} aria-label={label} onError={refuse} />
      : <audio className="session-detail-markdown-audio" controls preload="metadata" src={state.src} aria-label={label} onError={refuse} />;
  }
  if (state) {
    const message = state.status === "failed"
      ? state.message
      : markdownMediaFailureMessage(kind, relativePath, { code: "invalid_request", message: "" });
    return <MarkdownImagePlaceholder label={label} kind={kind} detail={message} showDetail />;
  }
  return <MarkdownImagePlaceholder label={label} kind={kind} loading />;
}

const BLOCKS = new Set(["p", "ul", "ol", "li", "blockquote", "pre", "table", "div", "h1", "h2", "h3", "h4", "h5", "h6"]);

/** The project tracks and clips a line links to, once each, that the line does
 * not already embed: Claude links what it made, and the player goes under the
 * line. Nested blocks keep their own links; a link around a picture is that
 * picture. */
function linkedPlayers(node: Element | undefined): string[] {
  if (!node) return [];
  const linked = new Set<string>();
  const embedded = new Set<string>();
  const visit = (parent: Element) => {
    for (const child of parent.children) {
      if (child.type !== "element" || BLOCKS.has(child.tagName)) continue;
      if (child.tagName === "img" && typeof child.properties.src === "string") {
        const path = safeRelativeMarkdownPath(child.properties.src);
        if (path) embedded.add(path);
      } else if (child.tagName === "a" && typeof child.properties.href === "string") {
        const picture = child.children.some((inner) => inner.type === "element" && inner.tagName === "img");
        const path = picture ? null : safeRelativeMarkdownPath(child.properties.href);
        if (path && markdownPlayerKind(path)) linked.add(path);
        visit(child);
      } else visit(child);
    }
  };
  visit(node);
  return [...linked].filter((path) => !embedded.has(path));
}

/* Parsing a markdown body is the most expensive thing this page does, and the
   page re-renders whenever any tool call expands or a tool detail resolves —
   none of which changes any body's text. Memoized on its own inputs, each body
   is parsed once and skipped thereafter. */
export const SessionDetailMarkdown = memo(function SessionDetailMarkdown({
  text,
  onOpenProjectPath,
  projectMedia,
  onLoadProjectMedia,
  projectRoot = "",
}: {
  text: string;
  onOpenProjectPath?: (relativePath: string) => void;
  projectMedia?: Readonly<Record<string, ProjectMarkdownMediaState>>;
  onLoadProjectMedia?: (relativePath: string) => void;
  projectRoot?: string;
}) {
  const components = useMemo<Components>(() => {
    const players = (node: Element | undefined) => onLoadProjectMedia
      ? linkedPlayers(node).map((path) => (
        <ProjectMarkdownPlayer
          key={path}
          relativePath={path}
          kind={markdownPlayerKind(path)!}
          alt={path.slice(path.lastIndexOf("/") + 1)}
          onLoadProjectMedia={onLoadProjectMedia}
        />
      ))
      : [];
    return {
        p: ({ node, children, ...props }) => {
          const linked = players(node);
          if (linked.length === 0) return <p {...props}>{children}</p>;
          return <><p {...props}>{children}</p><p className="session-detail-markdown-linked">{linked}</p></>;
        },
        li: ({ node, children, ...props }) => <li {...props}>{children}{players(node)}</li>,
        img: ({ src, alt }) => {
          const url = typeof src === "string" ? src : "";
          const label = alt ?? "";
          // The WebView loads safe external HTTPS images directly.
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
          const projectPath = url ? safeRelativeMarkdownPath(url) : null;
          if (!projectPath || !onLoadProjectMedia) {
            return (
              <MarkdownImagePlaceholder
                label={label || "image"}
                kind={(projectPath && markdownPlayerKind(projectPath)) || "image"}
                detail={projectPath
                  ? undefined
                  : "Only images inside the project can be embedded."}
              />
            );
          }
          const player = markdownPlayerKind(projectPath);
          if (player) {
            return (
              <ProjectMarkdownPlayer
                relativePath={projectPath}
                kind={player}
                alt={label}
                onLoadProjectMedia={onLoadProjectMedia}
              />
            );
          }
          return (
            <ProjectMarkdownImage
              relativePath={projectPath}
              alt={label}
              onLoadProjectMedia={onLoadProjectMedia}
            />
          );
        },
        a: ({ href, children, ...props }) => {
          const safeHref = href ? safeMarkdownUrl(href) : "";
          if (!safeHref) return <span>{children}</span>;
          const external = isSafeExternalMarkdownUrl(safeHref);
          const projectPath = safeRelativeMarkdownPath(safeHref);
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
                  // The shared opener provides a window.open fallback when the
                  // WebView rejects the plugin call.
                  if (isSafeExternalMarkdownUrl(safeHref)) void openExternalUrl(safeHref);
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
    };
  }, [onOpenProjectPath, onLoadProjectMedia, projectRoot]);

  return (
    <ProjectMediaContext.Provider value={projectMedia}>
      <MarkdownText text={text} components={components} />
    </ProjectMediaContext.Provider>
  );
});

function EventQuality({ event, title }: { event: SessionTimelineEvent; title?: string }) {
  if (event.quality === "exact") return null;
  const label = event.quality === "lower_bound"
    ? "Partial"
    : event.quality === "inferred"
      ? "Inferred"
      : "Unavailable";
  /* The chip stays a chip — one word on the row, no box. But its explanation
     lived only in `title`, which on a role-less span is neither an accessible
     name nor a description: nothing announces it and no keyboard reaches it.
     The sentence is printed as text and hidden visually, so the chip's meaning
     survives the pointer being absent while the tooltip keeps working. */
  return (
    <span className={`session-detail-quality quality-${event.quality}`} title={title}>
      {label}
      {title ? <span className="sync-visually-hidden"> {title}</span> : null}
    </span>
  );
}

/* One word is not enough to explain a matched commit, and the row has no space
   for the sentence. Both arms of the match land after the session ended, so
   this holds whichever one produced the checkpoint. */
const CHECKPOINT_QUALITY_HINT =
  "Matched from Git after the session ended. The transcript does not record a commit.";

/**
 * The rail tile is the speaker, and its label carries name and minute. Prompts
 * wear the user glyph, checkpoints the commit glyph, and the agent
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
      : event.kind === "agent_call"
        ? `${SESSION_EVENT_KIND_LABELS.agent_call} · ${event.agent?.label ?? ""}`.replace(/ · $/, "")
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
          : event.kind === "agent_call"
            ? <Icon name="git-fork" size={12} />
            : <ProviderMark provider={provider} displayName={providerDisplayName} size={12} />}
    </span>
  );
}

/* What the pane says where a text would be, by what the run proves. A spawned
   collaborator's brief is not readable from the parent rollout — the child
   transcript opens with it — so that section is not drawn at all: the verb in
   the head band is the way to it, and an apology in its place only sent the
   reader looking for a second control. */
const AGENT_BRIEF_NOTE = "No brief was recorded for this agent.";

function agentReportNote(agent: SessionAgentPreview): string {
  switch (agent.status) {
    case "running": return "Still running — no report yet.";
    case "failed": return "The call failed before a report.";
    case "interrupted": return "Interrupted before a final message.";
    case "unavailable": return "No transcript is available for this agent.";
    default: return "The transcript ends before a final message.";
  }
}

/**
 * A delegation row on the tool-row grammar with a 24px tile: `AGENT` stamp,
 * the task's label, the role, then calls and length. Status prints only when
 * the run is not simply done. Opening the row shows a head band — the facts
 * as chips and, at its right edge, the one verb, Open transcript, on the
 * app's bordered pill — then the brief and the report as bounded payload
 * boxes. The verb leads because it is the only thing the pane can do; as a
 * ghost under the report it was the last thing a reader found. The row keeps
 * its own open state: nothing is fetched on expand, the page already carries
 * the texts.
 */
function AgentRow({
  event,
  onOpenSession,
}: {
  event: SessionTimelineEvent;
  onOpenSession?: (resourceId: string) => void;
}) {
  const agent = event.agent;
  const [expanded, setExpanded] = useState(false);
  if (!agent) {
    return (
      <div className="session-detail-tool-malformed" role="note">
        <Icon name="alert-triangle" size={12} />
        This agent call could not be projected safely.
      </div>
    );
  }
  const length = formatSessionDuration(agent.started_at, agent.ended_at);
  const start = agent.started_at != null ? formatSessionDetailClock(agent.started_at) : null;
  const end = agent.ended_at != null ? formatSessionDetailClock(agent.ended_at) : null;
  const transcript = agent.transcript_resource_id ?? null;
  return (
    <details
      className={`session-detail-tool-call session-detail-agent status-${agent.status}`}
      open={expanded}
      onToggle={(toggleEvent) => setExpanded(toggleEvent.currentTarget.open)}
    >
      <summary>
        <span className="session-detail-tool-name session-detail-agent-stamp">Agent</span>
        <span className="session-detail-agent-label" title={agent.label}>{agent.label}</span>
        {agent.task && <span className="session-detail-agent-task" title={agent.task}>{agent.task}</span>}
        {agent.role && <span className="session-detail-agent-pill">{agent.role}</span>}
        {agent.mode === "async" && <span className="session-detail-agent-pill mode">async</span>}
        {/* One end cluster, so the length sits against the chevron on every
            row instead of splitting the free space with it. */}
        <span className="session-detail-agent-end">
          <span className="session-detail-agent-meta">
            {agent.tool_calls != null && (
              <span>{formatCompactNumber(agent.tool_calls)} {agent.tool_calls === 1 ? "call" : "calls"}</span>
            )}
            {length && <span className="session-detail-agent-length">{length}</span>}
          </span>
          {agent.status !== "completed" && (
            <span className={`session-detail-agent-status status-${agent.status}`}>
              {SESSION_AGENT_STATUS_LABELS[agent.status]}
            </span>
          )}
          <span className="session-detail-tool-chevron" aria-hidden="true">
            <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
          </span>
        </span>
      </summary>
      {expanded && (
        <div className="session-detail-tool-body">
          <div className="session-detail-tool-payload">
            <div className="session-detail-agent-head">
              {/* The facts the row could not hold, one chip each: role, model,
                  mode, the span, and what the parent recorded of the child's
                  own accounting. */}
              <div className="session-detail-agent-facts">
                {agent.role && <span><Icon name="git-fork" size={12} />{agent.role}</span>}
                {agent.model && <span>{agent.model}</span>}
                <span>{agent.mode}</span>
                {start && (
                  <span>
                    <Icon name="clock" size={12} />
                    {start} → {end ?? "…"}{length ? ` · ${length}` : ""}
                  </span>
                )}
                {agent.tool_calls != null && (
                  <span>{formatCompactNumber(agent.tool_calls)} {agent.tool_calls === 1 ? "call" : "calls"}</span>
                )}
                {agent.tokens != null && <span>{formatCompactNumber(agent.tokens)} tokens</span>}
              </div>
              <button
                type="button"
                className="btn session-detail-agent-open"
                disabled={!transcript || !onOpenSession}
                title={transcript ? "Open this agent's transcript as its own page" : "No transcript is available for this agent."}
                onClick={() => { if (transcript) onOpenSession?.(transcript); }}
              >
                <Icon name="external-link" size={12} />
                Open transcript
              </button>
            </div>
            {agent.brief
              ? <PayloadSection label="Brief" value={agent.brief} truncated={agent.brief_truncated} />
              : agent.mode !== "spawn" && (
                <section className="session-detail-payload-section">
                  <div className="session-detail-payload-head"><span className="session-detail-payload-label">Brief</span></div>
                  <p className="session-detail-agent-note">{AGENT_BRIEF_NOTE}</p>
                </section>
              )}
            {agent.report
              ? <PayloadSection label="Report" value={agent.report} truncated={agent.report_truncated} />
              : (
                <section className="session-detail-payload-section">
                  <div className="session-detail-payload-head"><span className="session-detail-payload-label">Report</span></div>
                  <p className="session-detail-agent-note">{agentReportNote(agent)}</p>
                </section>
              )}
          </div>
        </div>
      )}
    </details>
  );
}

/** A count that can actually be spoken; `Unavailable calls` is not a meta line. */
function speakableCount(count: SessionCount | undefined): SessionCount | null {
  if (!count || count.value == null || count.quality === "unsupported") return null;
  return count;
}

/**
 * Both speakers answer in a card; the rail node is what says who is talking,
 * so the card carries no name or mark of its own. Its head is a stamp band
 * reading clock · duration · calls: absolute clock because the divider already
 * owns the date, duration measured from the turn's loaded extent.
 */
function PromptCard({
  event,
  turnDuration,
  onOpenProjectPath,
  projectMedia,
  onLoadProjectMedia,
  projectRoot,
}: {
  event: SessionTimelineEvent;
  turnDuration?: string | null;
  onOpenProjectPath: (relativePath: string) => void;
  projectMedia?: Readonly<Record<string, ProjectMarkdownMediaState>>;
  onLoadProjectMedia?: (relativePath: string) => void;
  projectRoot: string;
}) {
  const callCount = speakableCount(event.turn_tool_summary?.tool_calls);
  const clock = event.timestamp != null ? formatSessionDetailClock(event.timestamp) : null;
  return (
    <div className="session-detail-card session-detail-card-prompt">
      {(clock || turnDuration || callCount) && (
        <div className="session-detail-card-head">
          {clock && <time dateTime={sessionDetailDateTime(event.timestamp)}>{clock}</time>}
          {turnDuration && <span>{turnDuration}</span>}
          {callCount && (
            <span>{formatSessionCount(callCount)} {callCount.value === 1 ? "call" : "calls"}</span>
          )}
        </div>
      )}
      <div className="session-detail-event-copy markdown-copy">
        {event.text ? (
          <SessionDetailMarkdown
            text={event.text}
            onOpenProjectPath={onOpenProjectPath}
            projectMedia={projectMedia}
            onLoadProjectMedia={onLoadProjectMedia}
            projectRoot={projectRoot}
          />
        ) : (
          <p className="session-detail-event-empty">No visible text is available for this event.</p>
        )}
      </div>
    </div>
  );
}

/** The agent's reply, in the same card as the prompt: a clock stamp, then prose. */
function ReplyCard({
  event,
  onOpenProjectPath,
  projectMedia,
  onLoadProjectMedia,
  projectRoot,
}: {
  event: SessionTimelineEvent;
  onOpenProjectPath: (relativePath: string) => void;
  projectMedia?: Readonly<Record<string, ProjectMarkdownMediaState>>;
  onLoadProjectMedia?: (relativePath: string) => void;
  projectRoot: string;
}) {
  const clock = event.timestamp != null ? formatSessionDetailClock(event.timestamp) : null;
  return (
    <div className="session-detail-card session-detail-card-reply">
      {clock && (
        <div className="session-detail-card-head">
          <time dateTime={sessionDetailDateTime(event.timestamp)}>{clock}</time>
        </div>
      )}
      <div className="session-detail-event-copy markdown-copy">
        {event.text ? (
          <SessionDetailMarkdown
            text={event.text}
            onOpenProjectPath={onOpenProjectPath}
            projectMedia={projectMedia}
            onLoadProjectMedia={onLoadProjectMedia}
            projectRoot={projectRoot}
          />
        ) : (
          <p className="session-detail-event-empty">No visible text is available for this event.</p>
        )}
      </div>
    </div>
  );
}

/** sha · subject · branch · clock — the fields the provider records; no more. */
function CheckpointBody({ event, repositoryUrl }: {
  event: SessionTimelineEvent;
  repositoryUrl?: string | null;
}) {
  const checkpoint = event.checkpoint;
  const subject = checkpoint?.subject ?? event.text ?? "Git checkpoint";
  const sha = checkpoint?.short_sha ?? checkpoint?.sha.slice(0, 7);
  // A sha links only where a remote can resolve it, the same rule the activity
  // list follows; the glyph is what marks it as an off-app target.
  const commitUrl = checkpoint ? githubCommitUrl(repositoryUrl, checkpoint.sha) : null;
  return (
    <div className="session-detail-checkpoint-card">
      {sha && commitUrl && (
        <a
          className="session-detail-checkpoint-link"
          href={commitUrl}
          target="_blank"
          rel="noopener noreferrer"
          title={`Open ${checkpoint?.sha} on GitHub`}
          onClick={(clickEvent) => { clickEvent.preventDefault(); void openExternalUrl(commitUrl); }}
        >
          <Icon name="external-link" size={12} />
          <code className="session-detail-checkpoint-sha">{sha}</code>
        </a>
      )}
      {sha && !commitUrl && (
        <code className="session-detail-checkpoint-sha" title={checkpoint?.sha}>{sha}</code>
      )}
      <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
      {/* The flag rides with branch and clock instead of taking a line of its
          own above the card. Every checkpoint is matched rather than recorded,
          so a stacked chip resized the row on the one kind that always carries
          one — 62px for the same commit a plain row states in 36px. */}
      <span className="session-detail-checkpoint-end">
        <EventQuality event={event} title={CHECKPOINT_QUALITY_HINT} />
        {checkpoint?.branch && (
          <span className="session-detail-checkpoint-branch">{checkpoint.branch}</span>
        )}
        {checkpoint && (
          <time dateTime={sessionDetailDateTime(checkpoint.committed_at)}>
            {formatSessionDetailClock(checkpoint.committed_at)}
          </time>
        )}
      </span>
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
  projectMedia,
  onLoadProjectMedia,
  projectRoot,
  repositoryUrl,
  onOpenSession,
}: Props) {
  const classes = [
    "session-detail-event",
    `event-${event.kind.replace(/_/g, "-")}`,
    event.kind === "tool_call" && event.tool ? `tool-${event.tool.status}` : "",
    event.kind === "agent_call" && event.agent ? `agent-${event.agent.status}` : "",
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
        {/* Checkpoints carry their own flag inside the card; every other kind
            has nowhere on its row to put one. */}
        {event.kind !== "checkpoint" && event.quality !== "exact" && (
          <div className="session-detail-event-flags"><EventQuality event={event} /></div>
        )}

        {event.kind === "tool_call" && provider === "claude" && event.tool && isClaudeWidgetTool(event.tool) ? (
          <ClaudeSvgWidget tool={event.tool} onCopy={writeClipboardText} />
        ) : event.kind === "tool_call" ? (
          <SessionDetailToolCall
            event={event}
            expanded={expandedToolIds.has(event.event_id)}
            detailState={toolDetails[event.event_id]}
            onExpandedChange={onToolExpandedChange}
          />
        ) : event.kind === "agent_call" ? (
          <AgentRow event={event} onOpenSession={onOpenSession} />
        ) : event.kind === "checkpoint" ? (
          <CheckpointBody event={event} repositoryUrl={repositoryUrl} />
        ) : event.kind === "prompt" ? (
          <PromptCard
            event={event}
            turnDuration={turnDuration}
            onOpenProjectPath={onOpenProjectPath}
            projectMedia={projectMedia}
            onLoadProjectMedia={onLoadProjectMedia}
            projectRoot={projectRoot}
          />
        ) : event.kind === "response" ? (
          <ReplyCard
            event={event}
            onOpenProjectPath={onOpenProjectPath}
            projectMedia={projectMedia}
            onLoadProjectMedia={onLoadProjectMedia}
            projectRoot={projectRoot}
          />
        ) : (
          <div className="session-detail-event-copy markdown-copy session-detail-intermediate-prose">
            {event.text ? (
              <SessionDetailMarkdown
                text={event.text}
                onOpenProjectPath={onOpenProjectPath}
                projectMedia={projectMedia}
                onLoadProjectMedia={onLoadProjectMedia}
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
