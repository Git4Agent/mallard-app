import { useState } from "react";
import Icon from "./Icons";
import ThreadStatusIcon from "./sync-review/ThreadStatusIcon";

type LayoutVariant =
  | "column"
  | "top"
  | "lower"
  | "edge"
  | "table"
  | "hover"
  | "swap"
  | "split"
  | "lane"
  | "stacked"
  | "stamp"
  | "compact";

interface CloudThread {
  id: string;
  title: string;
  updated: string;
}

interface Session {
  id: string;
  title: string;
  userTurns: number;
  tokens: string;
  messages: number;
  toolCalls?: number;
  duration?: string;
  updated: string;
}

interface CommitGroup {
  sha: string;
  title: string;
  updated: string;
  sessions: Session[];
}

const cloudThreads: CloudThread[] = [
  {
    id: "019fc94c-23b7-7422-a3d1-765c9ac90b2e",
    title: "Test conversation — fork · 2026-08-01 20:35Z — fork · 2026-08-02 02:36Z — fork",
    updated: "Aug 5, 2026 · 12:47 PM",
  },
  {
    id: "019fc956-9b3f-7992-9352-0376b0d8193c",
    title: "Test conversation — fork · 2026-08-02 02:04Z — fork · 2026-08-02 02:08Z — fork",
    updated: "Aug 5, 2026 · 12:01 AM",
  },
  {
    id: "019fc9d8-75c1-7b5f-a901-565d75083b78",
    title: "Respond to greeting — local before Pull · 2026-08-02 07:54Z — fork",
    updated: "Aug 4, 2026 · 9:48 PM",
  },
  {
    id: "019fb592-7219-77bc-93fc-20397c620210",
    title: "Test conversation — local before Pull",
    updated: "Aug 2, 2026 · 2:30 AM",
  },
  {
    id: "019fb5ab-22bb-72d4-af63-c0dff8a486e1",
    title: "Test conversation — local before Pull",
    updated: "Aug 2, 2026 · 2:30 AM",
  },
  {
    id: "019fb5fe-9a9e-77e0-a7ae-e6c4d9568737",
    title: "Test conversation — local before Pull",
    updated: "Aug 2, 2026 · 2:30 AM",
  },
  {
    id: "019fc176-bdcb-7319-8310-440139d7d3b3",
    title: "Respond to greeting — fork",
    updated: "Aug 2, 2026 · 2:30 AM",
  },
  {
    id: "019fc178-fdbc-7af4-a86f-1078e85a0c44",
    title: "Respond to greeting — local before Pull",
    updated: "Aug 2, 2026 · 2:30 AM",
  },
];

const uncommittedSessions: Session[] = [
  {
    id: "local-1",
    title: "Test conversation",
    userTurns: 8,
    tokens: "199.6K",
    messages: 8,
    duration: "5d 12h",
    updated: "Aug 5, 2026 · 2:37 AM",
  },
  {
    id: "local-2",
    title: "Test conversation — fork · 2026-08-01 20:35Z — fork · 2026-08-02 02:36Z — fork · 2026-08-03 08:25Z",
    userTurns: 6,
    tokens: "136.3K",
    messages: 6,
    toolCalls: 1,
    updated: "Aug 4, 2026 · 10:04 AM",
  },
  {
    id: "local-3",
    title: "Respond to greeting — fork",
    userTurns: 3,
    tokens: "57.3K",
    messages: 3,
    updated: "Aug 3, 2026 · 5:57 PM",
  },
  {
    id: "local-4",
    title: "Test conversation — fork · 2026-08-01 20:35Z — fork · 2026-08-02 02:36Z — fork",
    userTurns: 6,
    tokens: "136.3K",
    messages: 6,
    updated: "Aug 3, 2026 · 1:32 AM",
  },
  {
    id: "local-5",
    title: "Respond to greeting",
    userTurns: 3,
    tokens: "57.3K",
    messages: 3,
    duration: "1h 50m",
    updated: "Aug 2, 2026 · 2:40 AM",
  },
];

const commits: CommitGroup[] = [
  {
    sha: "7350274",
    title: "Test Again",
    updated: "Aug 2, 2026 · 12:11 AM",
    sessions: [
      {
        id: "commit-1",
        title: "Validate thread restore behavior",
        userTurns: 4,
        tokens: "68.2K",
        messages: 5,
        toolCalls: 2,
        duration: "44m",
        updated: "Aug 2, 2026 · 12:04 AM",
      },
    ],
  },
  {
    sha: "58d19ac",
    title: "Keep fork lineage through sync",
    updated: "Aug 1, 2026 · 8:58 PM",
    sessions: [
      {
        id: "commit-2",
        title: "Review fork lineage edge cases",
        userTurns: 7,
        tokens: "121.4K",
        messages: 8,
        toolCalls: 3,
        duration: "2h 18m",
        updated: "Aug 1, 2026 · 8:40 PM",
      },
    ],
  },
];

function CollapseButton({
  expanded,
  label,
  onClick,
}: {
  expanded: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="history-mockup-icon-button"
      aria-label={`${expanded ? "Collapse" : "Expand"} ${label}`}
      aria-expanded={expanded}
      onClick={onClick}
    >
      <Icon name={expanded ? "chevron-up" : "chevron-down"} size={13} />
    </button>
  );
}

function Collapsible({
  expanded,
  children,
}: {
  expanded: boolean;
  children: React.ReactNode;
}) {
  return (
    <div
      className="history-mockup-collapse"
      data-expanded={expanded}
      aria-hidden={!expanded}
      inert={!expanded}
    >
      <div>
        {children}
      </div>
    </div>
  );
}

function Timestamp({ value }: { value: string }) {
  const [date, clock] = value.split(" · ");

  return (
    <time className="history-mockup-timestamp" aria-label={value}>
      <span>{date}</span>
      <span className="history-mockup-time-separator" aria-hidden="true">·</span>
      <span>{clock}</span>
    </time>
  );
}

function CloudThreadRow({ thread }: { thread: CloudThread }) {
  return (
    <article className="history-mockup-cloud-row">
      <span className="history-mockup-state-icon cloud" aria-label="Only in cloud storage">
        <ThreadStatusIcon status="cloud-outstanding" size={18} />
      </span>
      <div className="history-mockup-cloud-copy">
        <strong title={thread.title}>{thread.title}</strong>
        <span className="history-mockup-thread-identity">
          <code>{thread.id}</code>
        </span>
      </div>
      <Timestamp value={thread.updated} />
    </article>
  );
}

function SessionMetrics({ session }: { session: Session }) {
  return (
    <div className="history-mockup-metrics" aria-label="Session details">
      <span title="User turns"><Icon name="user" size={12} /><b>{session.userTurns}</b></span>
      <span title="Total tokens"><Icon name="token" size={12} /><b>{session.tokens}</b></span>
      <span title="Agent messages"><Icon name="message" size={12} /><b>{session.messages}</b></span>
      {session.toolCalls != null && (
        <span title="Tool calls"><Icon name="tool" size={12} /><b>{session.toolCalls}</b></span>
      )}
      {session.duration && (
        <span title="Duration"><Icon name="clock" size={12} /><b>{session.duration}</b></span>
      )}
    </div>
  );
}

function SessionRow({ session }: { session: Session }) {
  const [detailsOpen, setDetailsOpen] = useState(false);

  return (
    <article className="history-mockup-session-row" data-expanded={detailsOpen}>
      <span className="history-mockup-state-icon local" aria-label="Local conversation">
        <ThreadStatusIcon status="local-outstanding" size={18} />
      </span>
      <strong className="history-mockup-session-title" title={session.title}>{session.title}</strong>
      <div className="history-mockup-session-actions" aria-label="Conversation actions">
        <button type="button" aria-label="Fork in Codex" title="Fork in Codex">
          <Icon name="git-fork" size={13} />
        </button>
        <button type="button" aria-label="Open in Codex" title="Open in Codex">
          <Icon name="openai" size={13} />
        </button>
        <button type="button" aria-label="Open in Terminal" title="Open in Terminal">
          <Icon name="terminal" size={13} />
        </button>
      </div>
      <SessionMetrics session={session} />
      <Timestamp value={session.updated} />
      <button
        type="button"
        className="history-mockup-row-arrow"
        aria-label={`${detailsOpen ? "Hide" : "Show"} details for ${session.title}`}
        aria-expanded={detailsOpen}
        onClick={() => setDetailsOpen(!detailsOpen)}
      >
        <Icon name={detailsOpen ? "chevron-up" : "chevron-down"} size={13} />
      </button>
      <Collapsible expanded={detailsOpen}>
        <div className="history-mockup-session-detail">
          <span>User</span>
          <p>Check the latest conversation state.</p>
          <time>2:31 AM</time>
        </div>
      </Collapsible>
    </article>
  );
}

function TimelineGroup({
  kind,
  title,
  subtitle,
  updated,
  expanded,
  onToggle,
  children,
  last = false,
}: {
  kind: "working" | "commit";
  title: string;
  subtitle: string;
  updated?: string;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  last?: boolean;
}) {
  return (
    <div className="history-mockup-timeline-group" data-kind={kind} data-last={last}>
      <div className="history-mockup-rail" aria-hidden="true">
        <span />
      </div>
      <section className="history-mockup-timeline-panel">
        <header>
          <div className="history-mockup-timeline-heading">
            <span aria-hidden="true">
              <Icon name={kind === "working" ? "edit" : "git-commit"} size={13} />
            </span>
            <strong>{title}</strong>
          </div>
          {kind === "commit" && (
            <div className="history-mockup-commit-meta">
              <code>{subtitle}</code>
              {updated && <Timestamp value={updated} />}
            </div>
          )}
          <CollapseButton expanded={expanded} label={title} onClick={onToggle} />
        </header>
        <Collapsible expanded={expanded}>
          <div className="history-mockup-session-list">
            {children}
          </div>
        </Collapsible>
      </section>
    </div>
  );
}

export default function HistoryRedesignMockup() {
  const [variant, setVariant] = useState<LayoutVariant>("column");
  const [cloudExpanded, setCloudExpanded] = useState(true);
  const [workingExpanded, setWorkingExpanded] = useState(true);
  const [expandedCommits, setExpandedCommits] = useState<Set<string>>(new Set());

  const toggleCommit = (sha: string) => {
    setExpandedCommits((current) => {
      const next = new Set(current);
      if (next.has(sha)) next.delete(sha);
      else next.add(sha);
      return next;
    });
  };

  return (
    <main className="history-mockup-page">
      <div className="history-mockup-shell" data-variant={variant}>
        <nav className="history-mockup-variants" aria-label="Row layout variations">
          {([
            ["column", "Balanced"],
            ["top", "Title date"],
            ["lower", "Meta date"],
            ["edge", "Right block"],
            ["table", "Details"],
            ["hover", "Quiet"],
            ["swap", "Controls"],
            ["split", "Stacked right"],
            ["lane", "Time rail"],
            ["stacked", "Date stack"],
            ["stamp", "Underline"],
            ["compact", "Compact"],
          ] as const).map(([value, label]) => (
            <button
              key={value}
              type="button"
              aria-pressed={variant === value}
              onClick={() => setVariant(value)}
            >
              {label}
            </button>
          ))}
        </nav>
        <div className="history-mockup-content">
          <section className="history-mockup-cloud-section" aria-labelledby="cloud-only-heading">
            <header>
              <span className="history-mockup-section-icon"><Icon name="cloud" size={13} /></span>
              <h2 id="cloud-only-heading">Only in <strong>Local storage 1</strong></h2>
              <CollapseButton
                expanded={cloudExpanded}
                label="cloud-only conversations"
                onClick={() => setCloudExpanded(!cloudExpanded)}
              />
            </header>
            <Collapsible expanded={cloudExpanded}>
              <div className="history-mockup-cloud-list">
                {cloudThreads.map((thread) => <CloudThreadRow key={thread.id} thread={thread} />)}
              </div>
            </Collapsible>
          </section>

          <section className="history-mockup-working-section" aria-label="Git history">
            <div className="history-mockup-timeline">
              <TimelineGroup
                kind="working"
                title="Uncommitted"
                subtitle={String(uncommittedSessions.length)}
                expanded={workingExpanded}
                onToggle={() => setWorkingExpanded(!workingExpanded)}
              >
                {uncommittedSessions.map((session) => <SessionRow key={session.id} session={session} />)}
              </TimelineGroup>

              {commits.map((commit, index) => (
                <TimelineGroup
                  key={commit.sha}
                  kind="commit"
                  title={commit.title}
                  subtitle={commit.sha}
                  updated={commit.updated}
                  expanded={expandedCommits.has(commit.sha)}
                  onToggle={() => toggleCommit(commit.sha)}
                  last={index === commits.length - 1}
                >
                  {commit.sessions.map((session) => <SessionRow key={session.id} session={session} />)}
                </TimelineGroup>
              ))}
            </div>
          </section>
        </div>
      </div>
    </main>
  );
}
