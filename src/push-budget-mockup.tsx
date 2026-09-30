import { StrictMode, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import Notice, { NoticeZone, type NoticeItem } from "./components/notice/Notice";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import { formatBytes } from "./format";
import "./App.css";

/**
 * Mockup: the Push review when the backlog is bigger than one push.
 *
 * Before — the shipped review: a 30-day page of the commit rail, held rows
 * filtered out, Load earlier at the foot, a separate "Older activity" box
 * once everything is loaded, and a footer that says nothing about size.
 * A — the same commit rail, but the whole timeline at once: every commit and
 * every session, newest first, no paging and no box. Rows that don't fit sit
 * under their commits without a checkbox, with a "Later" chip, a size
 * readout per row, a push meter in the footer, and a banner that says what
 * this push carries and that the rest waits.
 * B — after the push: the outcome banner says how many didn't fit and offers
 * Push again. Nothing is remembered between pushes; the next review simply
 * recomputes from the current state.
 *
 * Static data, shipped CSS, real leaf components. The fit is the planned
 * policy (plan I6: newest first, stop at the first row that does not fit) in
 * a few lines, so every count on the page is derived, not typed. Not part of
 * the app bundle: the build has one entry, and this file is reachable only
 * from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1160px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 20px; flex-wrap: wrap; }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-2xs); margin-left: 8px; }
  .mk-bar > span:first-child { margin-left: 0; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 5px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--bg-active); color: var(--text-0); border-color: var(--border-mid); }
  .mk-section { margin: 34px 0 10px; }
  .mk-section > h2 { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-lg); }
  .mk-section > p { margin: 0 0 14px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-variant { margin: 0 0 26px; }
  .mk-variant > h3 { margin: 0 0 2px; color: var(--text-0); font-size: var(--type-md); }
  .mk-variant > p { margin: 0 0 10px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-frame { width: 100%; max-width: 980px; padding: 3px; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame > section { border-radius: 4px; overflow: hidden; }
  /* Mock-only: the push meter in the footer's sentence slot. The slot is
     already a flex column (footer > span); the bar is a 4px track under the
     sentence, in the local-outstanding ink because it measures what this
     push carries, danger ink once it does not fit. */
  .mk-budget { gap: 5px; }
  .mk-budget > small { white-space: nowrap; }
  .mk-budget-bar { display: block; width: 100%; max-width: 280px; height: 4px; border-radius: 2px; background: var(--border-mid); overflow: hidden; }
  .mk-budget-bar > i { display: block; height: 100%; background: var(--thread-status-local); }
  .mk-budget.over > small { color: var(--danger-text); }
  .mk-budget.over .mk-budget-bar > i { background: var(--danger-text); }
  /* Mock-only: the outcome strip stands for the storage card's outcome
     banner, which is where it lives in the app. */
  .mk-strip { max-width: 980px; }
  .mk-strip .notice { margin: 0; }
  .mk-note { margin: 6px 0 0; color: var(--text-3); font-size: var(--type-2xs); }
`;

/* ---------------------------------------------------------------- data --- */

type Provider = "claude" | "codex";

interface MockCommit {
  id: string;
  sha: string;
  subject: string;
  when: string;
  /** Outside the shipped review's first 30-day page. */
  older?: boolean;
}

interface MockSession {
  id: string;
  title: string;
  provider: Provider;
  /** Capture bytes: transcript plus sidecars, what the push preflight sums. */
  bytes: number;
  /** Ordering key, newest first. Days before 2026-09-01. */
  age: number;
  when: string;
  messages: number;
  tools: number;
  tokens: string;
  length: string;
  /** The selected rail's successor commit, or null when this rail has none. */
  commit: string | null;
  /** Outside the shipped review's first 30-day page: hidden there until Load
   *  earlier. In the whole-timeline review it is just another row. */
  older?: boolean;
}

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
/** Requirements the push always carries: skills and provider state. */
const RESERVED = Math.round(3.2 * MB);
const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude Code", codex: "Codex" };

const COMMITS: MockCommit[] = [
  { id: "c1", sha: "dd8d420", subject: "Finalize mainland storage research pagination", when: "Sep 1, 10:12" },
  { id: "c2", sha: "e444782", subject: "Refine sync review UX and bound session capture", when: "Sep 1, 02:05" },
  { id: "c3", sha: "789eea5", subject: "Optimize Claude session metadata scans", when: "Aug 31, 23:58" },
  { id: "c4", sha: "ec82ce1", subject: "Optimize provider scans and activity logs", when: "Aug 30, 23:40" },
  { id: "c5", sha: "c002338", subject: "Add storage reset and refine sync reviews", when: "Aug 29, 20:14" },
  { id: "c6", sha: "4c1d9e2", subject: "Ship the Lamplight dark palette", when: "Aug 28, 23:50" },
  { id: "c7", sha: "9be0f31", subject: "Add catalog profile launch buttons", when: "Aug 27, 21:30" },
  { id: "c8", sha: "2f7a8c4", subject: "Make sidebar sections collapsible", when: "Aug 26, 19:02" },
  { id: "c9", sha: "7d31e0a", subject: "Storage setup drafts and Test button", when: "Aug 24, 16:45" },
  { id: "c10", sha: "51ac2b9", subject: "Files review cutover and loading scaffold", when: "Aug 23, 22:31" },
  { id: "c11", sha: "a0e77d3", subject: "One notice system", when: "Aug 21, 18:20" },
  { id: "c12", sha: "3e9f0c1", subject: "Claude cursor parity and bundle convergence", when: "Jul 30, 19:44", older: true },
  { id: "c13", sha: "c4b2d17", subject: "Credential redaction and in-root remap", when: "Jul 26, 22:10", older: true },
  { id: "c14", sha: "8a6e5f0", subject: "Settings page and cloud thread ids", when: "Jul 19, 15:37", older: true },
  { id: "c15", sha: "15d3b7e", subject: "Pet previews", when: "Jul 11, 16:02", older: true },
];

const SESSIONS: MockSession[] = [
  { id: "s1", title: "Refine sync review UX and bound session capture", provider: "claude", bytes: 96 * MB, age: 0, when: "Sep 1, 14:02", messages: 412, tools: 388, tokens: "3.1M", length: "6h 12m", commit: null },
  { id: "s5", title: "Finalize mainland storage research pagination", provider: "claude", bytes: 58 * MB, age: 1, when: "Aug 31, 09:08", messages: 188, tools: 140, tokens: "1.7M", length: "2h 44m", commit: "c1" },
  { id: "s2", title: "Optimize Claude session metadata scans", provider: "claude", bytes: 84 * MB, age: 1, when: "Aug 31, 22:40", messages: 356, tools: 301, tokens: "2.6M", length: "4h 51m", commit: "c3" },
  { id: "session-3", title: "Optimize provider scans and activity logs", provider: "claude", bytes: 71 * MB, age: 2, when: "Aug 30, 19:15", messages: 298, tools: 264, tokens: "2.2M", length: "3h 58m", commit: "c4" },
  { id: "s4", title: "Add storage reset and refine sync reviews", provider: "codex", bytes: 63 * MB, age: 3, when: "Aug 29, 16:33", messages: 240, tools: 211, tokens: "1.9M", length: "3h 20m", commit: "c5" },
  { id: "s6", title: "Dark mode Lamplight palette", provider: "claude", bytes: 49 * MB, age: 4, when: "Aug 28, 21:27", messages: 174, tools: 133, tokens: "1.5M", length: "2h 31m", commit: "c6" },
  { id: "s7", title: "Catalog profile launch buttons", provider: "codex", bytes: 118 * MB, age: 5, when: "Aug 27, 18:02", messages: 468, tools: 402, tokens: "3.8M", length: "7h 05m", commit: "c7" },
  { id: "s8", title: "Sidebar collapsible sections", provider: "claude", bytes: 96 * MB, age: 6, when: "Aug 26, 15:44", messages: 390, tools: 355, tokens: "3.0M", length: "5h 36m", commit: "c8" },
  { id: "s9", title: "Storage setup drafts and Test button", provider: "claude", bytes: 91 * MB, age: 8, when: "Aug 24, 11:20", messages: 372, tools: 318, tokens: "2.9M", length: "5h 02m", commit: "c9" },
  { id: "s10", title: "Loading system scaffold", provider: "claude", bytes: 38 * MB, age: 9, when: "Aug 23, 20:11", messages: 141, tools: 96, tokens: "1.1M", length: "1h 58m", commit: "c10" },
  { id: "s11", title: "Files review cutover A1+B1", provider: "claude", bytes: 57 * MB, age: 10, when: "Aug 22, 17:49", messages: 205, tools: 170, tokens: "1.8M", length: "2h 57m", commit: "c10" },
  { id: "s12", title: "Notice system cutover", provider: "claude", bytes: 52 * MB, age: 11, when: "Aug 21, 13:05", messages: 196, tools: 152, tokens: "1.6M", length: "2h 40m", commit: "c11" },
  { id: "o1", title: "Claude cursor parity", provider: "claude", bytes: 132 * MB, age: 33, when: "Jul 30, 16:10", messages: 512, tools: 470, tokens: "4.2M", length: "8h 14m", commit: "c12", older: true },
  { id: "o2", title: "Repo bundle convergence", provider: "codex", bytes: 88 * MB, age: 35, when: "Jul 28, 10:52", messages: 344, tools: 290, tokens: "2.8M", length: "4h 47m", commit: "c12", older: true },
  { id: "o3", title: "Credential redaction", provider: "claude", bytes: 76 * MB, age: 37, when: "Jul 26, 19:33", messages: 301, tools: 262, tokens: "2.4M", length: "4h 03m", commit: "c13", older: true },
  { id: "o4", title: "Claude in-root path remap", provider: "claude", bytes: 143 * MB, age: 41, when: "Jul 22, 14:26", messages: 560, tools: 509, tokens: "4.6M", length: "9h 30m", commit: "c13", older: true },
  { id: "o5", title: "Settings page", provider: "claude", bytes: 61 * MB, age: 44, when: "Jul 19, 09:41", messages: 233, tools: 184, tokens: "1.9M", length: "3h 12m", commit: "c14", older: true },
  { id: "o6", title: "Cloud thread id decoupling", provider: "codex", bytes: 95 * MB, age: 48, when: "Jul 15, 17:58", messages: 380, tools: 334, tokens: "3.1M", length: "5h 20m", commit: "c14", older: true },
  { id: "o7", title: "Pet previews", provider: "codex", bytes: 58 * MB, age: 52, when: "Jul 11, 12:15", messages: 210, tools: 168, tokens: "1.7M", length: "2h 49m", commit: "c15", older: true },
];

/* -------------------------------------------------------------- policy --- */

/** The planned fit (plan I6): base first (empty here), then newest first —
 *  and the walk STOPS at the first row that does not fit, so a push is a
 *  block of time and no older row goes before a newer one. A row too large
 *  for any push is not a hold: it is unsyncable, blocked like an unreadable
 *  session, absent from the review, and the walk continues past it. */
function fit(rows: MockSession[], budget: number): { admitted: Set<string>; held: MockSession[]; unsyncable: Set<string> } {
  const ordered = [...rows].sort((a, b) => a.age - b.age);
  let spent = 0;
  let full = false;
  const admitted = new Set<string>();
  const held: MockSession[] = [];
  const unsyncable = new Set<string>();
  for (const row of ordered) {
    if (row.bytes > budget) {
      unsyncable.add(row.id);
      continue;
    }
    if (!full && spent + row.bytes <= budget) {
      spent += row.bytes;
      admitted.add(row.id);
    } else {
      full = true;
      held.push(row);
    }
  }
  return { admitted, held, unsyncable };
}

/* ---------------------------------------------------------------- rows --- */

type RowState =
  | { kind: "admitted"; selected: boolean }
  | { kind: "waiting" };

function Row({ session, state, sizeChip, onToggle }: {
  session: MockSession;
  state: RowState;
  sizeChip: boolean;
  onToggle?: () => void;
}) {
  const selectable = state.kind === "admitted";
  const selected = state.kind === "admitted" && state.selected;
  const tip = state.kind === "admitted"
    ? (state.selected ? "Update storage. Only on this computer. Push to save it in Attic R2." : "Not selected. Only on this computer.")
    : `Doesn't fit in this push — the ${formatBytes(LIMIT)} limit is full with newer sessions. Push again after this one.`;
  return (
    <article className={`sync-history-thread-card${selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${session.title}`}>
              <input type="checkbox" checked={selected} aria-label={`${selected ? "Exclude" : "Include"} ${session.title}`} onChange={onToggle} />
            </label>
          )}
          <span className="sync-thread-sync-indicator local" data-tooltip={tip} aria-label={tip} tabIndex={0}>
            <ThreadStatusIcon status="local-outstanding" size={18} />
          </span>
          <strong>{session.title}</strong>
          {state.kind === "waiting" && (
            <span className="sync-history-branch-state history-only" title={tip}>Later</span>
          )}
        </div>
        <div className="sync-history-thread-actions">
          <button type="button" className="btn btn-ghost sync-history-launch-action" title={`Open in ${PROVIDER_NAME[session.provider]}`} aria-label={`Open in ${PROVIDER_NAME[session.provider]}`}>
            <ProviderMark provider={session.provider} displayName={PROVIDER_NAME[session.provider]} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Session details">
            <span className="sync-history-thread-metric" data-tooltip={`Messages · ${session.messages}`} tabIndex={0}><Icon name="message" size={12} /><b>{session.messages}</b></span>
            <span className="sync-history-thread-metric" data-tooltip={`Tool calls · ${session.tools}`} tabIndex={0}><Icon name="tool" size={12} /><b>{session.tools}</b></span>
            <span className="sync-history-thread-metric" data-tooltip={`Total tokens · ${session.tokens}`} tabIndex={0}><Icon name="token" size={12} /><b>{session.tokens}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-duration" data-tooltip={`Ran for ${session.length}`} tabIndex={0}><Icon name="clock" size={12} /><b>{session.length}</b></span>
            {sizeChip && (
              <span className="sync-history-thread-metric" data-tooltip={`Push size · ${formatBytes(session.bytes)} (transcript and sidecars)`} tabIndex={0}><Icon name="drive" size={12} /><b>{formatBytes(session.bytes)}</b></span>
            )}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="View session details" title="View session details">
              <Icon name="chevron-right" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Started · ${session.when}`} tabIndex={0}>
            <time>{session.when}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

/** The rail's fold control, outside the card so nothing in the row can move it. */
function Fold({ collapsed, controls, label, disabled = false, onToggle }: {
  collapsed: boolean;
  controls: string;
  label: string;
  disabled?: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className="sync-history-spine-fold"
      aria-label={label}
      title={label}
      aria-expanded={!collapsed}
      aria-controls={controls}
      disabled={disabled}
      onClick={onToggle}
    >
      <Icon name={collapsed ? "chevron-right" : "chevron-down"} size={12} />
    </button>
  );
}

/** The rail's foot in the shipped review: Load earlier, hung from the spine. */
function RailFoot({ label }: { label: string }) {
  return (
    <div className="sync-history-rail-foot">
      <span className="sync-history-rail-foot-node" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
      <button type="button" className="sync-history-rail-step" title={label} aria-label={label}>
        <strong>Load earlier</strong>
        <time>· Jul 30</time>
      </button>
    </div>
  );
}

/* -------------------------------------------------------------- review --- */

type Direction = "before" | "after";

function Meter({ total, limit, breakdown }: { total: number; limit: number; breakdown: string }) {
  const over = total > limit;
  const sentence = over
    ? `${formatBytes(total - limit)} over the ${formatBytes(limit)} push limit — exclude sessions or files to push.`
    : `This push · ${formatBytes(total)} of ${formatBytes(limit)}`;
  return (
    <span className={`mk-budget${over ? " over" : ""}`} role="status" title={breakdown}>
      <small>{sentence}</small>
      <span className="mk-budget-bar" role="meter" aria-valuemin={0} aria-valuemax={limit} aria-valuenow={Math.min(total, limit)} aria-valuetext={sentence}>
        <i style={{ width: `${Math.min(100, (total / limit) * 100)}%` }} />
      </span>
    </span>
  );
}

function MockReview({ direction, projectFilesBytes }: {
  direction: Direction;
  projectFilesBytes: number;
}) {
  const budget = LIMIT - RESERVED;
  const plan = useMemo(() => fit(SESSIONS, budget), [budget]);
  const [deselected, setDeselected] = useState<Set<string>>(new Set());
  const [collapsedCommits, setCollapsedCommits] = useState<Set<string>>(new Set());
  const [branchUnmappedCollapsed, setBranchUnmappedCollapsed] = useState(false);
  const isSelected = (id: string) => plan.admitted.has(id) && !deselected.has(id);
  const toggle = (id: string) => setDeselected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const toggleCommit = (id: string) => setCollapsedCommits((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  const admittedRows = SESSIONS.filter((row) => plan.admitted.has(row.id));
  const selectedCount = admittedRows.filter((row) => isSelected(row.id)).length;
  const admittedBytes = admittedRows.reduce((sum, row) => sum + row.bytes, 0);
  const selectedBytes = admittedRows.filter((row) => isSelected(row.id)).reduce((sum, row) => sum + row.bytes, 0);
  const heldBytes = plan.held.reduce((sum, row) => sum + row.bytes, 0);
  const total = selectedBytes + RESERVED + projectFilesBytes;
  const over = total > LIMIT;
  const breakdown = `Sessions ${formatBytes(selectedBytes)} · Requirements ${formatBytes(RESERVED)} · Project files ${formatBytes(projectFilesBytes)}`;
  /* The cut line, as a date: the oldest session this push carries. */
  const oldestAdmitted = admittedRows.reduce<MockSession | null>(
    (oldest, row) => (oldest == null || row.age > oldest.age ? row : oldest),
    null,
  );

  const showWaiting = direction !== "before";
  /* Before: the shipped review pages the rail by 30-day windows and hides the
   * holds, so the first page shows only the commits of the window and only
   * the sessions that fit. After: the whole timeline, every commit and every
   * session, newest first — no paging, no separate box. */
  const visible = (row: MockSession) => !plan.unsyncable.has(row.id) && (showWaiting || (!row.older && plan.admitted.has(row.id)));
  const commits = COMMITS.filter((commit) => showWaiting || !commit.older);
  const sessionsOf = (commit: MockCommit) => SESSIONS.filter((row) => row.commit === commit.id && visible(row)).sort((a, b) => a.age - b.age);
  const branchUnmapped = SESSIONS.filter((row) => row.commit === null && visible(row)).sort((a, b) => a.age - b.age);

  const banners: NoticeItem[] = [];
  if (direction === "before") {
    // The pre-cutover copy, verbatim, for contrast — including its own
    // noun/verb disagreement. Historical; never port it back into the app.
    banners.push({
      key: "capture-budget",
      severity: "warning",
      title: `${plan.held.length} conversations wait for the next round`,
      message: "This review's 512 MB capture budget filled before their turn. Push the selected conversations first, then open Push review again.",
    });
  } else if (plan.held.length > 0) {
    banners.push({
      key: "capture-budget",
      severity: "warning",
      title: "Too much to push at once",
      message: `This push carries the newest ${admittedRows.length} sessions (${formatBytes(admittedBytes)}), back to ${oldestAdmitted?.when.split(",")[0] ?? "today"}. The ${plan.held.length} older sessions (${formatBytes(heldBytes)}) don't fit — push again after this one.`,
    });
  }

  const stateOf = (row: MockSession): RowState => plan.admitted.has(row.id)
    ? { kind: "admitted", selected: isSelected(row.id) }
    : { kind: "waiting" };

  const disabledReason = over
    ? `${formatBytes(total - LIMIT)} over the ${formatBytes(LIMIT)} push limit — exclude sessions or files to push.`
    : selectedCount === 0 && projectFilesBytes === 0 ? "Nothing is selected" : null;

  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-label={`Push review mock — ${direction}`}>
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>Push review</h2></div>
        <SyncReviewSelectionBar
          count={selectedCount}
          total={admittedRows.length}
          recommendedTitle={`Use the recommended session selection (${admittedRows.length})`}
          recommendedCount={admittedRows.length}
          recommendedDisabled={deselected.size === 0}
          onRecommended={() => setDeselected(new Set())}
          clearDisabled={selectedCount === 0}
          onClear={() => setDeselected(new Set(admittedRows.map((row) => row.id)))}
          close={{ ariaLabel: "Close push review", onClose: () => undefined }}
        />
      </header>
      <SyncReviewTabs
        tabs={[
          { id: "history", label: "Git history", icon: "git-commit", count: selectedCount },
          { id: "project_files", label: "Project files", icon: "file", count: projectFilesBytes > 0 ? 1 : 0 },
        ]}
        activeId="history"
        onChange={() => undefined}
      />
      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {banners.length > 0 && (
            <div className="sync-review-banners">
              <NoticeZone items={banners} subject="notes about this Push review" timing="standing" label="Notes about this Push review" />
            </div>
          )}
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section aria-label="Git history">
              <div className={`sync-history-timeline${branchUnmapped.length > 0 ? " has-branch-unmapped" : ""}`}>
                {branchUnmapped.length > 0 && (
                  <section className={`sync-history-branch-unmapped${branchUnmappedCollapsed ? " collapsed" : ""}`} aria-labelledby={`branch-unmapped-heading-${direction}`}>
                    <Fold
                      collapsed={branchUnmappedCollapsed}
                      controls={`branch-unmapped-list-${direction}`}
                      label={`${branchUnmappedCollapsed ? "Expand" : "Collapse"} ${branchUnmapped.length} uncommitted session${branchUnmapped.length === 1 ? "" : "s"}`}
                      onToggle={() => setBranchUnmappedCollapsed((current) => !current)}
                    />
                    <div className="sync-history-section-heading">
                      <h2 id={`branch-unmapped-heading-${direction}`}><Icon name="edit" size={13} />Uncommitted</h2>
                    </div>
                    {!branchUnmappedCollapsed && (
                      <div id={`branch-unmapped-list-${direction}`} className="sync-history-thread-list flat">
                        {branchUnmapped.map((row) => (
                          <Row key={row.id} session={row} state={stateOf(row)} sizeChip={showWaiting} onToggle={() => toggle(row.id)} />
                        ))}
                      </div>
                    )}
                  </section>
                )}
                <section className="sync-history-commit-section" aria-label="First-parent commits on main">
                  <ol className="sync-history-commit-rail">
                    {commits.map((commit) => {
                      const rows = sessionsOf(commit);
                      const collapsed = collapsedCommits.has(commit.id);
                      const listId = `commit-sessions-${direction}-${commit.sha}`;
                      return (
                        <li key={commit.id} className={`sync-history-commit${collapsed ? " collapsed" : ""}`}>
                          <Fold
                            collapsed={collapsed}
                            controls={listId}
                            label={`${collapsed ? "Expand" : "Collapse"} commit ${commit.sha}`}
                            disabled={rows.length === 0}
                            onToggle={() => toggleCommit(commit.id)}
                          />
                          <div className="sync-history-commit-heading">
                            <span className="sync-history-commit-title">
                              <span className="sync-history-commit-icon" aria-hidden="true"><Icon name="git-commit" size={14} /></span>
                              <strong title={commit.subject}>{commit.subject}</strong>
                            </span>
                            <span className="sync-history-commit-meta">
                              <a className="sync-history-commit-link" href={`https://github.com/hequ/mallard-app/commit/${commit.sha}`} title={`Open ${commit.sha} on GitHub`} aria-label={`Open commit ${commit.sha} on GitHub`} onClick={(event) => event.preventDefault()}>
                                <Icon name="external-link" size={12} />
                                <code>{commit.sha}</code>
                              </a>
                              <time>{commit.when}</time>
                            </span>
                          </div>
                          {!collapsed && rows.length > 0 && (
                            <div id={listId} className="sync-history-thread-list">
                              {rows.map((row) => (
                                <Row key={row.id} session={row} state={stateOf(row)} sizeChip={showWaiting} onToggle={() => toggle(row.id)} />
                              ))}
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ol>
                </section>
                {!showWaiting && <RailFoot label="Load earlier history — through Jul 30, 4 commits and 7 sessions" />}
              </div>
            </section>
          </div>
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {showWaiting && <Meter total={total} limit={LIMIT} breakdown={breakdown} />}
        <button type="button" className="btn btn-ghost" title="Review project files next">
          Project files<Icon name="chevron-right" size={14} />
        </button>
        <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={disabledReason != null} title={disabledReason ?? undefined}>
          <ThreadStatusIcon status="local-outstanding" size={16} />
          Push
        </button>
      </footer>
    </section>
  );
}

/** After the push: the storage card's outcome banner says how many sessions
 *  didn't fit and offers Push again. It opens Push review the ordinary way;
 *  nothing is carried over. */
function AfterPush({ pushed, pushedBytes, left }: { pushed: number; pushedBytes: number; left: number }) {
  return (
    <div className="mk-strip">
      <Notice
        severity="success"
        title={`${pushed} sessions pushed`}
        timing="standing"
        label="Push outcome"
        action={<button type="button" className="btn btn-ghost"><Icon name="upload" size={13} />Push again</button>}
      >
        {formatBytes(pushedBytes)} · Attic R2 version 12. {left} sessions didn&apos;t fit — push again to continue.
      </Notice>
      <p className="mk-note">In the app this is the storage card&apos;s outcome banner. &quot;Push again&quot; opens Push review as usual; the new review recomputes what fits from the current state.</p>
    </div>
  );
}

/* ---------------------------------------------------------------- page --- */

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [projectFilesBytes, setProjectFilesBytes] = useState(0);
  document.documentElement.dataset.theme = theme;
  const plan = fit(SESSIONS, LIMIT - RESERVED);
  const admitted = SESSIONS.filter((row) => plan.admitted.has(row.id));
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Push review — when the backlog is bigger than one push</h1>
      <p className="mk-lede">
        Nineteen unsynced sessions across fifteen commits, 1.5 GB, one 512 MB push. The fit is the
        planned policy: newest first, stopping at the first session that no longer fits, so a push is a
        block of time and no older session goes before a newer one. Nothing is remembered between
        pushes. It runs over the mock data, so every count on this page is derived. Project files and
        Requirements ride the same push, which is how the meter can go over the limit while every
        session still fits.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        <span>Theme</span>
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
        <span>Project files</span>
        <button type="button" className={projectFilesBytes === 0 ? "on" : undefined} onClick={() => setProjectFilesBytes(0)}>None</button>
        <button type="button" className={projectFilesBytes === 38 * MB ? "on" : undefined} onClick={() => setProjectFilesBytes(38 * MB)}>38 MB</button>
        <button type="button" className={projectFilesBytes === 140 * MB ? "on" : undefined} onClick={() => setProjectFilesBytes(140 * MB)}>140 MB (over)</button>
      </div>

      <div className="mk-section">
        <h2>Before — the shipped review</h2>
        <p>
          A 30-day page of the commit rail. The held sessions are filtered out, so commits whose only
          sessions don&apos;t fit stand empty, the banner counts rows nobody can see, and Load earlier
          only ever reveals older sessions that happen to be pushable; whatever has no activity row at
          all lands in a separate &quot;Older activity&quot; box once everything is loaded. Nothing
          states how much this push carries. Rows have no size.
        </p>
        <div className="mk-variant">
          <div className="mk-frame"><MockReview direction="before" projectFilesBytes={projectFilesBytes} /></div>
        </div>
      </div>

      <div className="mk-section">
        <h2>A — The whole timeline: what fits, what waits</h2>
        <p>
          The same commit rail, no window: every commit and every session, newest first, with the
          Uncommitted sessions on top as on the activity page — no Load earlier, no separate box. The cut is
          a line in time: the six newest sessions fit, the seventh does not, and everything from there
          down sits under its commit without a checkbox, with a &quot;Later&quot; chip and a tooltip
          that says the push is full and to push again after this one. Review rows gain a size readout
          after the duration, so the cut is legible row by row. The footer&apos;s sentence slot becomes
          the push meter: sessions plus Requirements plus project files against the limit, live as you
          tick. Switch Project files to 140 MB to see the over-limit state — the only way to exceed once
          every admitted session fits by construction — with Push disabled and the sentence as its
          title. The banner says what this push carries, back to which date, and that the rest
          doesn&apos;t fit. Sessions too large for any push are not here at all: they are unsyncable and
          show as unavailable on the activity page.
        </p>
        <div className="mk-variant">
          <div className="mk-frame"><MockReview direction="after" projectFilesBytes={projectFilesBytes} /></div>
        </div>
      </div>

      <div className="mk-section">
        <h2>B — After the push</h2>
        <p>
          The outcome banner on the storage card reports the push and, in the same breath, how many
          sessions didn&apos;t fit. &quot;Push again&quot; opens Push review as usual. Mallard keeps no
          record of what was left out: the next review recomputes the fit from whatever is outstanding
          then, newest first.
        </p>
        <div className="mk-variant">
          <AfterPush pushed={admitted.length} pushedBytes={admitted.reduce((sum, row) => sum + row.bytes, 0)} left={plan.held.length} />
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
