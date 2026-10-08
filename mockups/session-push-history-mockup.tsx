import { StrictMode, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import type { ThreadSyncComparison, ThreadSyncEntry } from "../src/types";
import Icon from "../src/components/Icons";
import { SessionPushRow, type PushSessionFacts } from "../src/components/project-sync/ProjectActivityPage";
import SessionPushReview from "../src/components/project-sync/SessionPushReview";
import {
  PushApplyButton,
  pushBudgetTotal,
  pushReviewDisabledReason,
} from "../src/components/project-sync/PushResourceWorkspace";
import { ReviewBudgetGauge } from "../src/components/sync-review/ReviewBudgetMeter";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatBytes } from "../src/format";
import { formatFullDateTime } from "../src/clock";
import "../src/App.css";

/**
 * Mockup: where, if anywhere, a session's own Push review says that the
 * Push also carries the project's Git history to Mallard Cloud
 * (docs/PLAN_SESSION_PUSH_GIT_HISTORY.md). Seven directions, one rule for
 * all of them: a mark appears only when storage shares a commit and this
 * Push adds at least one (`git_history_outgoing.new_commits > 0`), the
 * same rule as the project review's ↑ chip. Words live in tooltips.
 *
 *   A  none — today's face
 *   B  ↑N pill after the session title (IMPLEMENTED: the shipped component draws it)
 *   C  branch + ↑N flag in the header, beside the redaction flag
 *   D  ↑N count on the Push button
 *   E  the project review's rail: the new commits, the session under its own
 *   F  the gauge's tooltip gains a Git history line (drawn open)
 *   G  ↑N as a metric in the row's fact line
 *
 * `npm run dev`, then /mockups/session-push-history-mockup.html
 * (`?history=three|one|rewritten|branch|uptodate|first&width=wide|narrow&theme=light|dark&only=all|a..g&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type HistoryId = "three" | "one" | "rewritten" | "branch" | "uptodate" | "first";
type Dir = "a" | "b" | "c" | "d" | "e" | "f" | "g";
type Width = "wide" | "narrow";
interface Knobs {
  history: HistoryId;
  width: Width;
  theme: "light" | "dark";
  only: Dir | "all";
  /** Draws the frames without the specimen's own title and controls. */
  shot: "off" | "on";
}

const HISTORIES: readonly HistoryId[] = ["three", "one", "rewritten", "branch", "uptodate", "first"];
const DIRS: readonly Dir[] = ["a", "b", "c", "d", "e", "f", "g"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    history: pick("history", HISTORIES, "three"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    shot: pick("shot", ["off", "on"] as const, "off"),
  };
}

const HISTORY_LABEL: Record<HistoryId, string> = {
  three: "3 new commits",
  one: "1 new commit",
  rewritten: "Rewritten (amend)",
  branch: "Other branch",
  uptodate: "Up to date",
  first: "No stored history",
};

const DIR_LABEL: Record<Dir, string> = {
  a: "A · None",
  b: "B · Pill on the row",
  c: "C · Header flag",
  d: "D · Count on Push",
  e: "E · The rail",
  f: "F · Gauge tooltip",
  g: "G · Row metric",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  a: <>Today’s face. The history rides along and the review says nothing about it: it is presentation on the Cloud and never gates the Push.</>,
  b: <>The rail’s commit chip, <b>↑3</b>, after the session title. Risk: on a session row the arrow can read as the session’s own count.</>,
  c: <>A second flag beside the redaction flag: branch glyph, branch name, <b>↑3</b>. The count gets its noun. Costs header width at 800 px.</>,
  d: <>The button that sends it carries the count, as GitHub Desktop’s <i>Push origin ↑3</i>. Risk: the path bar’s Push counts sessions, so a ↑3 here can read as three sessions.</>,
  e: <>The project review’s own timeline: the new commits with their ↑ chips, the session under the commit it maps to. Shows what goes up; the body grows from one row to one per commit.</>,
  f: <>The gauge’s title gains a line (drawn open here). Nothing visible until hover: the review still looks like A.</>,
  g: <>In the row’s fact line, in the row’s grammar: commit glyph, <b>↑3</b>. Sits with messages, tools, tokens and length, which describe the session; this one does not.</>,
};

/** Where each form is already in use. Checked first-hand or in the product's own source. */
const DIR_SEEN: Record<Dir, ReactNode> = {
  a: <>NN/g: an indicator belongs on the element it describes — here the history is not on screen, so there is nothing to mark.</>,
  b: <>GitHub’s branches page (read in the pane): every branch row carries <i>N ahead, M behind</i>. Primer <i>counter label</i> (a count beside what it counts).</>,
  c: <>VS Code status bar: branch · <code>0↓ 3↑</code>. GitHub Desktop toolbar: branch tab beside <i>Push origin ↑3</i>.</>,
  d: <>GitHub Desktop: <i>Push origin</i> with ↑N on the button. VS Code: the sync button carries <code>↓ ↑</code> counts.</>,
  e: <>VS Code Source Control view: incoming and outgoing commits listed before you sync. Mallard’s own project Push review (B1).</>,
  f: <>ARIA APG tooltip pattern: hover/focus only, so it must not be the only place a fact lives — the pattern argues against this one.</>,
  g: <>Primer counter label on GitHub’s PR header tabs (<i>Commits 3</i>). GitLab’s fork box: ahead/behind under the branch picker.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const KB = 1024;
const MB = 1024 * KB;
const LIMIT = 512 * MB;
const RESERVED = 4 * KB;
const PROJECT = "mallard-app";
const STORAGE = "Mallard Cloud";
const TITLE = "Shorten automatic updates copy";
const PROVIDER = "claude";
const PROVIDER_NAME = "Claude Code";
const TRANSCRIPT = 1.3 * MB - RESERVED;

const THREAD_ID = "019f7798-5437-7632-9dbc-5b589cf68b10";
const RESOURCE_ID = `claude:session:${THREAD_ID}`;
/* 2026-10-03T04:03:00Z, 21:03 on Oct 2 in the owner's zone. */
const ENDED_AT = 1_791_000_180;

interface Commit { sha: string; subject: string; at: number }

const C1: Commit = { sha: "1787e59c4531a0f7d2b6e3c19a84f5d07b2c6e11", subject: "Shorten the automatic updates copy", at: ENDED_AT + 7 * 60 };
const C2: Commit = { sha: "53414b33bb13e8a2c47f9d061b5e2a3c8d7f4e90", subject: "Move the install note into the button tooltip", at: ENDED_AT - 5 * 60 };
const C3: Commit = { sha: "3d91603aab13c5e7f80b2d4a6c9e1f3b5d7a9c02", subject: "Update the two settings tests", at: ENDED_AT - 11 * 60 };
/** Storage's head: yesterday's commit. */
const C4: Commit = { sha: "e41b9d07c2a85f3e19b0d4c6a7e8f2b1c3d5e7f9", subject: "Add the update corner", at: ENDED_AT - 26 * 3600 };
/** C1 after `git commit --amend`: a new sha, the same subject. */
const C1_AMENDED: Commit = { ...C1, sha: "9b2f4c6e8a0d1f3b5c7e9a1d3f5b7c9e1a3d5f70", at: ENDED_AT + 9 * 60 };

interface Scenario {
  branch: string;
  /** Newest first: the commits storage does not have. */
  newCommits: Commit[];
  rewritten: boolean;
  /** Null when storage holds no history yet. */
  stored: { branch: string; head: string } | null;
}

const SCENARIOS: Record<HistoryId, Scenario> = {
  three: { branch: "main", newCommits: [C1, C2, C3], rewritten: false, stored: { branch: "main", head: C4.sha } },
  one: { branch: "main", newCommits: [C1], rewritten: false, stored: { branch: "main", head: C2.sha } },
  rewritten: { branch: "main", newCommits: [C1_AMENDED], rewritten: true, stored: { branch: "main", head: C1.sha } },
  branch: { branch: "updates-copy", newCommits: [C1, C2], rewritten: false, stored: { branch: "main", head: C4.sha } },
  uptodate: { branch: "main", newCommits: [], rewritten: false, stored: { branch: "main", head: C1.sha } },
  first: { branch: "main", newCommits: [C1, C2, C3], rewritten: false, stored: null },
};

/** What `git_history_outgoing` gives the review. Null when storage has no
 *  history, or none this branch shares: nothing is marked, as on the
 *  project review's rail. */
interface Outgoing { count: number; rewritten: boolean; branch: string; storedBranch: string }

function outgoingOf(s: Scenario): Outgoing | null {
  if (!s.stored || s.newCommits.length === 0) return null;
  return { count: s.newCommits.length, rewritten: s.rewritten, branch: s.branch, storedBranch: s.stored.branch };
}

/** One sentence for every mark, in the shipped chip's words. */
function markTitle(o: Outgoing): string {
  const commits = o.count === 1 ? "1 commit" : `${o.count} commits`;
  const lines = [`${commits} new since the last Push to ${STORAGE} · ${o.branch}`];
  if (o.storedBranch !== o.branch) lines.push(`Replaces the ${o.storedBranch} history ${STORAGE} holds.`);
  if (o.rewritten) lines.push(`History was rewritten: ${STORAGE}’s latest commit is no longer on this branch.`);
  return lines.join("\n");
}

const FACTS: PushSessionFacts = {
  title: TITLE,
  metrics: {
    started_at: ENDED_AT - 22 * 60,
    ended_at: ENDED_AT,
    user_round_count: 23,
    agent_message_count: 23,
    tool_call_count: 31,
    total_tokens: 184_000,
    metrics_complete: true,
  },
};

const ENTRY: ThreadSyncEntry = {
  provider: PROVIDER,
  provider_display_name: PROVIDER_NAME,
  access: "bound_ready",
  thread_id: THREAD_ID,
  conversation_resource_id: `claude:conversation:${THREAD_ID}`,
  resource_id: RESOURCE_ID,
  display_name: TITLE,
  state: "local_only",
  local_present: true,
  storage_present: false,
  compared_against_storage: true,
  subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
  local_updated_at: ENDED_AT,
  push_decision: { action: "create_cloud", selectable: true },
  pull_decision: { action: "no_changes", selectable: false },
  capture_bytes: TRANSCRIPT,
};

const COMPARISON: ThreadSyncComparison = {
  project_id: "project-mallard",
  storage_id: "storage-cloud",
  storage_name: STORAGE,
  compared_at: ENDED_AT,
  entries: [ENTRY],
  counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
  scopes: [],
  provider_errors: {},
  warnings: [],
  capture_budget: { limit_bytes: LIMIT, reserved_bytes: RESERVED },
  remote_continuity: { state: "in_sync", revision: 1 },
};

const SELECTED: ReadonlySet<string> = new Set([RESOURCE_ID]);
const noop = () => undefined;

/* ——— the marks ——————————————————————————————————————————————————— */

function warn(o: Outgoing): string {
  return o.rewritten ? " history-only" : "";
}

/** C · the redaction flag's pill: branch glyph, branch, count. */
function Flag({ o }: { o: Outgoing }) {
  const title = markTitle(o);
  return (
    <span className={`sync-settings-flag mk-flag${warn(o)}`} title={title} aria-label={title} tabIndex={0}>
      <Icon name="git-branch" size={11} />
      <code>{o.branch}</code>
      <b>↑{o.count}</b>
    </span>
  );
}

/** G · a metric in the row's fact line, with the row's own tooltip. */
function Metric({ o }: { o: Outgoing }) {
  const title = markTitle(o);
  return (
    <span className={`sync-history-thread-metric mk-metric${warn(o)}`} data-tooltip={title} aria-label={title} tabIndex={0}>
      <Icon name="git-commit" size={12} />
      <b>↑{o.count}</b>
    </span>
  );
}

/** D · the apply button with the count the path bar's buttons wear. */
function PushButtonWithCount({ o, disabledReason }: { o: Outgoing | null; disabledReason: string | null }) {
  return (
    <button
      type="button"
      className="btn btn-primary sync-pull-apply-button"
      disabled={disabledReason !== null}
      title={disabledReason ?? (o ? markTitle(o) : undefined)}
    >
      <ThreadStatusIcon status="local-outstanding" size={16} />
      Push
      {o && <span className="mk-btn-count" aria-label={markTitle(o)}>↑{o.count}</span>}
    </button>
  );
}

/* ——— a slot inside the shipped row ————————————————————————————————— */

/** Puts a span after (or inside) the first match under `host`, for a mark to
 *  portal into. The shipped component is not changed; the specimen only
 *  needs the mark where the change would put it. */
function useSlot(host: RefObject<HTMLDivElement | null>, selector: string, where: "after" | "append", key: string) {
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    const target = host.current?.querySelector<HTMLElement>(selector);
    if (!target) return undefined;
    const element = document.createElement("span");
    element.className = "mk-slot";
    if (where === "after") target.after(element);
    else target.append(element);
    setSlot(element);
    return () => {
      element.remove();
      setSlot(null);
    };
  }, [host, selector, where, key]);
  return slot;
}

/* ——— the review, shipped and mocked ——————————————————————————————— */

/** The shipped review. B is implemented: with `history`, it hands the
 *  component the review context's Git history and its outgoing delta, and
 *  the component draws the pill itself. */
function Shipped({ s, o, history = false }: { s: Scenario; o: Outgoing | null; history?: boolean }) {
  return (
    <SessionPushReview
      credentialRedactionEnabled={false}
      entry={ENTRY}
      facts={FACTS}
      storageName={STORAGE}
      selected={SELECTED}
      threadComparison={COMPARISON}
      busy={false}
      gitHistory={history && s.stored ? { branch: s.branch, head_sha: s.newCommits[0]?.sha ?? s.stored.head, commits: [], uncommitted_session_refs: [], outside_history_session_refs: [], truncated: false } : null}
      gitHistoryOutgoing={history && o ? { new_commits: o.count, rewritten: o.rewritten } : history && s.stored ? { new_commits: 0, rewritten: false } : null}
      onClose={noop}
      onPush={noop}
    />
  );
}

/** A: the shipped review with no history. G: the shipped review with one
 *  mark portalled into its row's fact line. */
function ShippedWithMark({ s, o, mark }: { s: Scenario; o: Outgoing | null; mark: "none" | "metric" }) {
  const host = useRef<HTMLDivElement>(null);
  const slot = useSlot(host, ".sync-history-thread-meta", "append", `${mark}:${o ? markTitle(o) : ""}`);
  return (
    <div ref={host} className="mk-host">
      <Shipped s={s} o={o} />
      {o && slot && mark === "metric" && createPortal(<Metric o={o} />, slot)}
    </div>
  );
}

/** The shipped review's frame, markup for markup, with four places a
 *  direction may fill: the header, the body, the gauge and the button. */
function Shell({ headerExtra, body, gauge, button }: {
  headerExtra?: ReactNode;
  body: ReactNode;
  gauge?: ReactNode;
  button?: ReactNode;
}) {
  const budget = pushBudgetTotal(SELECTED, COMPARISON);
  const disabledReason = pushReviewDisabledReason({ selected: SELECTED, threadComparison: COMPARISON, deletionApproved: true, sessionScope: true });
  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-label="Push review">
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <span className="sync-settings-flag">Credential redaction: Off</span>
          {headerExtra}
        </div>
        <div className="sync-push-resource-actions">
          <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close push review">
            <Icon name="x" size={15} />
          </button>
        </div>
      </header>
      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section className="sync-history-content" aria-label="Session">{body}</section>
          </div>
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {gauge ?? (budget && (
          <ReviewBudgetGauge budget={budget} verb="push" breakdown={`Session ${formatBytes(budget.sessions)} · Requirements ${formatBytes(budget.reserved)}`} />
        ))}
        {button ?? <PushApplyButton busy={false} fetchProgress={null} disabledReason={disabledReason} onPush={noop} />}
      </footer>
    </section>
  );
}

const ROW = (
  <div className="sync-history-thread-list flat">
    <SessionPushRow entry={ENTRY} storageName={STORAGE} facts={FACTS} />
  </div>
);

/** E · the project review's rail: each new commit a card with its ↑ chip,
 *  the session under the commit at or after its last prompt. */
function Rail({ s, o }: { s: Scenario; o: Outgoing | null }) {
  if (!o) return ROW;
  const title = markTitle(o);
  /* The closest commit at or after the session's end, as `map_sessions`. */
  const holder = [...s.newCommits].reverse().find((commit) => commit.at >= ENDED_AT)?.sha ?? null;
  return (
    <div className="sync-history-timeline">
      <section className="sync-history-commit-section" aria-label={`Commits this Push adds on ${s.branch}`}>
        <ol className="sync-history-commit-rail mk-rail">
          {s.newCommits.map((commit) => (
            <li key={commit.sha} className="sync-history-commit">
              <span className="sync-history-spine-fold inert" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
              <div className="sync-history-commit-heading">
                <span className="sync-history-commit-title">
                  <span className="sync-history-commit-icon" aria-hidden="true"><Icon name="git-commit" size={14} /></span>
                  <strong title={commit.subject}>{commit.subject}</strong>
                </span>
                <span className="sync-history-commit-meta">
                  <span className={`sync-history-branch-state sync-history-outgoing${warn(o)}`} title={title} aria-label={title}>↑</span>
                  <code title={commit.sha}>{commit.sha.slice(0, 7)}</code>
                  <time dateTime={new Date(commit.at * 1_000).toISOString()}>{formatFullDateTime(commit.at)}</time>
                </span>
              </div>
              {commit.sha === holder && (
                <div className="sync-history-thread-list">
                  <SessionPushRow entry={ENTRY} storageName={STORAGE} facts={FACTS} />
                </div>
              )}
            </li>
          ))}
        </ol>
      </section>
      {holder === null && ROW}
    </div>
  );
}

/** F · the gauge with its title drawn open. */
function GaugeWithTip({ o }: { o: Outgoing | null }) {
  const budget = pushBudgetTotal(SELECTED, COMPARISON);
  if (!budget) return null;
  const base = `Session ${formatBytes(budget.sessions)} · Requirements ${formatBytes(budget.reserved)}`;
  const breakdown = o ? `${base}\nGit history ↑${o.count} · ${o.branch}${o.rewritten ? " · rewritten" : ""}` : base;
  return (
    <div className="mk-tip-host">
      {o && <span className="mk-tip" aria-hidden="true">{breakdown}</span>}
      <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} />
    </div>
  );
}

function Review({ dir, s }: { dir: Dir; s: Scenario }) {
  const o = outgoingOf(s);
  const disabledReason = pushReviewDisabledReason({ selected: SELECTED, threadComparison: COMPARISON, deletionApproved: true, sessionScope: true });
  switch (dir) {
    case "a": return <ShippedWithMark s={s} o={o} mark="none" />;
    case "b": return <Shipped s={s} o={o} history />;
    case "g": return <ShippedWithMark s={s} o={o} mark="metric" />;
    case "c": return <Shell headerExtra={o && <Flag o={o} />} body={ROW} />;
    case "d": return <Shell body={ROW} button={<PushButtonWithCount o={o} disabledReason={disabledReason} />} />;
    case "e": return <Shell body={<Rail s={s} o={o} />} />;
    case "f": return <Shell body={ROW} gauge={<GaugeWithTip o={o} />} />;
  }
}

/* ——— the app around the review ———————————————————————————————————— */

const CRUMBS: PathBarCrumb[] = [
  { id: "storage", label: STORAGE, icon: "storage-mallard" },
  { id: "project", label: PROJECT, icon: "git-folder" },
];

function PathActions() {
  return (
    <>
      <button type="button" className="sync-path-icon-action" disabled aria-label="Refresh session" title="Refresh session">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" disabled>
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
        </button>
        <button type="button" className="sync-path-sync active" aria-expanded>
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
        </button>
      </div>
    </>
  );
}

function Workspace({ dir, s }: { dir: Dir; s: Scenario }) {
  return (
    <>
      <SyncPathBar crumbs={CRUMBS} actions={<PathActions />} ariaLabel={`${PROJECT} path`} />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
        <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
          <div className="profile-links-list sync-combined-settings-panel">
            <article className="profile-link-card sync-project-link-card">
              <div className="profile-link-connections">
                <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                  <div className="project-profile-storage-list">
                    <div className="storage-link-block selected sync-review-open">
                      <div className="sync-storage-inline-review" tabIndex={-1}><Review dir={dir} s={s} /></div>
                    </div>
                  </div>
                </section>
              </div>
            </article>
          </div>
        </section>
      </main>
    </>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

/** The main pane beside a 318px sidebar: a 1100px window (the default) and
 *  an 800px one (the minimum). */
const PANE_WIDTH: Record<Width, number> = { wide: 782, narrow: 482 };

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ ...next });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const width = PANE_WIDTH[knobs.width];
  const s = SCENARIOS[knobs.history];
  const shown = knobs.only === "all" ? DIRS : [knobs.only];
  return (
    <div className="spec">
      {knobs.shot === "off" && (
        <>
          <h1 className="spec-title">Session Push review: saying that the Git history travels too</h1>
          <p className="spec-sub">
            Every frame is the main pane at the chosen window width, drawn with the shipped path bar, header, row, gauge and
            button on the shipped CSS. A and B are the shipped <code>SessionPushReview</code> (B with the review context’s Git
            history, which it now draws itself); G is the same component with one mark added to its row; C–F rebuild its frame
            markup for markup. Only the classes prefixed <code>mk-</code> are new.
            One rule for every mark: it appears only when {STORAGE} already holds a history this branch shares and this Push adds
            commits to it, so <i>Up to date</i> and <i>No stored history</i> show nothing anywhere, as on the project review’s rail.
          </p>

          <div className="spec-controls">
            <div className="spec-group"><span>History</span>{HISTORIES.map((id) => seg("history", id, HISTORY_LABEL[id]))}</div>
            <div className="spec-group"><span>Pane</span>{seg("width", "wide", "1100 px window")}{seg("width", "narrow", "800 px window")}</div>
            <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
          </div>
        </>
      )}

      <div className="spec-grid">
        {shown.map((dir) => (
          <div key={dir} data-dir={dir} style={{ width }}>
            <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "e" && <em>body grows</em>}{dir === "f" && <em>hover only</em>}</h2>
            {knobs.shot === "off" && <p className="spec-col-note">{DIR_NOTE[dir]}</p>}
            <div className="spec-frame">
              <Workspace dir={dir} s={s} />
            </div>
            {knobs.shot === "off" && <p className="spec-seen"><b>Seen in</b> · {DIR_SEEN[dir]}</p>}
          </div>
        ))}
      </div>

      {knobs.shot === "off" && (
        <>
          <table className="spec-table">
            <thead>
              <tr><th>Reference</th><th>What it asks</th><th>Meets it</th><th>Argues against</th></tr>
            </thead>
            <tbody>
              <tr><td>NN/g · Indicators, validations, notifications</td><td>A passive fact about an element is an indicator placed on that element, not a notification.</td><td>B, G (on the row), D (on the act), C (in the review’s standing facts)</td><td>A says nothing; F hides it.</td></tr>
              <tr><td>Primer · Counter label</td><td>A count sits beside text that names what is counted.</td><td>C (branch beside the count), E (the commits themselves)</td><td>B, D, G: a bare ↑N; the noun is only in the tooltip.</td></tr>
              <tr><td>GitHub Desktop · push-pull-button</td><td>The ahead count rides on the Push button, arrow then number.</td><td>D</td><td>—</td></tr>
              <tr><td>VS Code · git status bar</td><td>Branch and <code>behind↓ ahead↑</code> stand together; the Source Control view lists outgoing commits.</td><td>C, E</td><td>—</td></tr>
              <tr><td>Material · Badge</td><td>A count badge attaches to an icon or action; long counts cap (999+).</td><td>D, G (cap at 99+)</td><td>—</td></tr>
              <tr><td>Apple HIG · Badging</td><td>A badge counts pending items only, never other numbers.</td><td>D: pending commits are pending items</td><td>—</td></tr>
              <tr><td>WCAG 2.1 · 1.4.1 Use of color</td><td>A state may not be told by color alone.</td><td>—</td><td>Every direction’s <i>rewritten</i> state is the warning tint alone, as the shipped ↑ chip is. Open for all of them: a glyph or a word.</td></tr>
              <tr><td>ARIA APG · Tooltip</td><td>A tooltip shows on hover and focus and is supplementary.</td><td>B, C, D, G keep words in tooltips</td><td>F: the fact exists only in a tooltip.</td></tr>
              <tr><td>Carbon · Tag</td><td>A read-only tag labels and categorizes.</td><td>C (the branch is a label)</td><td>A tag is not a counter.</td></tr>
            </tbody>
          </table>
          <p className="spec-foot">
            Sources: <a href="https://www.nngroup.com/articles/indicators-validations-notifications/">NN/g</a> ·
            <a href="https://primer.style/components/counter-label"> Primer counter label</a> ·
            <a href="https://github.com/desktop/desktop/blob/development/app/src/ui/toolbar/push-pull-button.tsx"> GitHub Desktop source</a> ·
            <a href="https://github.com/microsoft/vscode/blob/main/extensions/git/src/repository.ts"> VS Code git source</a> ·
            <a href="https://code.visualstudio.com/docs/sourcecontrol/overview"> VS Code docs</a> ·
            <a href="https://api.flutter.dev/flutter/material/Badge-class.html"> Material badge</a> ·
            <a href="https://developer.apple.com/design/human-interface-guidelines/notifications"> HIG notifications</a> ·
            <a href="https://www.w3.org/WAI/WCAG21/Understanding/use-of-color.html"> WCAG 1.4.1</a> ·
            <a href="https://www.w3.org/WAI/ARIA/apg/patterns/tooltip/"> ARIA APG tooltip</a> ·
            <a href="https://www.carbondesignsystem.com/components/tag/usage"> Carbon tag</a> ·
            <a href="https://docs.gitlab.com/user/project/repository/forking_workflow/"> GitLab forks</a> ·
            <a href="https://github.com/desktop/desktop/branches"> GitHub branches page</a> (its rows are labelled “Ahead and behind counts: 12 ahead, 0 behind”).
            Not read: Material 3’s own badge page and GitHub Desktop’s docs render without the text (JS); the Material rule is taken from Flutter’s badge, which implements it.
          </p>
        </>
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
