import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import type { BundleFetchProgress, InventoryResource, ThreadSyncComparison, ThreadSyncEntry } from "../src/types";
import Icon from "../src/components/Icons";
import type { PushSessionFacts } from "../src/components/project-sync/ProjectActivityPage";
import ResourceInventory from "../src/components/project-sync/ResourceInventory";
import SessionPushReview from "../src/components/project-sync/SessionPushReview";
import { sessionNoun } from "../src/components/project-sync/model";
import NameSearchField from "../src/components/sync-review/NameSearchField";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import { ReviewBudgetGauge, ReviewBudgetGlance, reviewBudgetSentence } from "../src/components/sync-review/ReviewBudgetMeter";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatBytes } from "../src/format";
import "../src/App.css";

/**
 * Mockup: the Push review that opens from a session's own page.
 *
 * It ships as the project review holding one row: a search field, a 1/1
 * count, Unselect, a trash, a group header with its own 1/1 and Clear, and a
 * checkbox, all acting on a list of one. Unticking the row only disables
 * Push. The size prints twice and the row states nothing but the title.
 *
 *   Before  what ships
 *   A       the review's frame; the body is the project review's session row
 *           (picked 2026-10-02 and shipped: this frame mounts the shipped
 *           `SessionPushReview`)
 *   B       the review's frame; the body is a ledger of what the Push writes
 *   C       no body: the header names the session, the footer commits
 *   D       C's facts in one row, docked on the session page (the session
 *           stays open; today the review leaves it for the project page)
 *
 * `npm run dev`, then /mockups/single-push-mockup.html
 * (`?session=new|agents|omitted|over|busy&redaction=off|on&width=wide|narrow&theme=light|dark&only=before|a|b|c|d&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type SessionId = "new" | "agents" | "omitted" | "over" | "busy";
type Dir = "before" | "a" | "b" | "c" | "d";
type Width = "wide" | "narrow";
interface Knobs {
  session: SessionId;
  redaction: "off" | "on";
  width: Width;
  theme: "light" | "dark";
  only: Dir | "all";
  /** Draws the frames without the specimen's own title and controls. */
  shot: "off" | "on";
}

const SESSIONS: readonly SessionId[] = ["new", "agents", "omitted", "over", "busy"];
const DIRS: readonly Dir[] = ["before", "a", "b", "c", "d"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    session: pick("session", SESSIONS, "new"),
    redaction: pick("redaction", ["off", "on"] as const, "off"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    shot: pick("shot", ["off", "on"] as const, "off"),
  };
}

const SESSION_LABEL: Record<SessionId, string> = {
  new: "1.3 MB, no agents",
  agents: "3 agents travel",
  omitted: "Agents off in Settings",
  over: "Over the limit",
  busy: "Pushing",
};

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (shipped)",
  a: "A · the row, alone",
  b: "B · packing list",
  c: "C · no body",
  d: "D · in place, on the session page",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>The project review holding one row. Search, <b>1/1</b>, Unselect, the trash, the group’s own 1/1 and Clear, and the checkbox all act on a list of one, and unticking it only disables Push. The size prints twice. The row states the title and nothing else.</>,
  a: <>The same header and footer. The body is the project review’s own session row without its checkbox: status, provider, messages, tool calls, tokens, length, size, time. × and Esc leave. The size prints once, at the button.</>,
  b: <>A’s frame. The body lists what this Push writes: transcript, agents, requirements. The rows add up to the gauge, and the agents row is where the Settings switch shows.</>,
  c: <>The header names the session in place of “Push review”. The footer carries the redaction flag, the agents chip, the gauge and Push. Two rows, nothing between them.</>,
  d: <><b>Behaviour change.</b> C’s facts in one row, docked on the session page. The session stays open under it and its crumb is the title; after the Push you are still there. Today the review leaves the session for the project page.</>,
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
const WHEN = "Oct 2, 21:03";

interface Scenario {
  transcript: number;
  /** Agents this session delegated to, on this computer. */
  agentCount: number;
  agentBytes: number;
  /** Settings has "Push Claude subagents" off: the agents stay behind. */
  agentsOmitted: boolean;
  busy: boolean;
}

const SCENARIOS: Record<SessionId, Scenario> = {
  new: { transcript: 1.3 * MB - RESERVED, agentCount: 0, agentBytes: 0, agentsOmitted: false, busy: false },
  agents: { transcript: 2.1 * MB, agentCount: 3, agentBytes: 1.2 * MB, agentsOmitted: false, busy: false },
  omitted: { transcript: 2.1 * MB, agentCount: 3, agentBytes: 0, agentsOmitted: true, busy: false },
  over: { transcript: 540 * MB, agentCount: 0, agentBytes: 0, agentsOmitted: false, busy: false },
  busy: { transcript: 1.3 * MB - RESERVED, agentCount: 0, agentBytes: 0, agentsOmitted: false, busy: true },
};

function budgetOf(s: Scenario) {
  return { total: s.transcript + s.agentBytes + RESERVED, limit: LIMIT };
}

function breakdown(s: Scenario): string {
  return `Sessions ${formatBytes(s.transcript + s.agentBytes)} · Requirements ${formatBytes(RESERVED)} · Project files 0 B`;
}

/** The refusal, reworded for one session: there is nothing to exclude. */
function sessionOverSentence(s: Scenario): string {
  const budget = budgetOf(s);
  return `${formatBytes(budget.total - budget.limit)} over the ${formatBytes(budget.limit)} push limit.`;
}

const noop = () => undefined;

/* ——— parts every direction shares ———————————————————————————————— */

function RedactionFlag({ on }: { on: boolean }) {
  return <span className="sync-settings-flag">Credential redaction: {on ? "On" : "Off"}</span>;
}

function CloseCluster({ busy }: { busy: boolean }) {
  return (
    <div className="sync-push-resource-actions">
      <button type="button" className="btn btn-ghost sync-inline-action-close" disabled={busy} aria-label="Close push review">
        <Icon name="x" size={15} />
      </button>
    </div>
  );
}

function PushButton({ s, sentence }: { s: Scenario; sentence: string | null }) {
  return (
    <button
      type="button"
      className="btn btn-primary sync-pull-apply-button"
      disabled={s.busy || sentence !== null}
      title={sentence ?? undefined}
    >
      {s.busy
        ? <Icon name="refresh" size={16} className="icon-spin" />
        : <ThreadStatusIcon status="local-outstanding" size={16} />}
      {s.busy ? <span className="sync-btn-readout">3/7 · 0.8 MB…</span> : "Push"}
    </button>
  );
}

const BUSY_SENTENCE = "A running push finishes or fails whole — it can’t be cancelled midway.";

/** The review footer as it ships, with one optional leading group of facts. */
function ReviewFooter({ s, sentence, facts }: { s: Scenario; sentence: string | null; facts?: ReactNode }) {
  return (
    <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
      {sentence && <p className="sync-review-budget-sentence">{sentence}</p>}
      {facts && <div className="mk-foot-facts">{facts}</div>}
      {s.busy && <span role="status"><small>{BUSY_SENTENCE}</small></span>}
      {facts ? (
        /* Beside facts the gauge and the button wrap as one unit, so a
           narrow pane never parts the quantity from the act that spends it. */
        <div className="mk-foot-commit">
          <ReviewBudgetGauge budget={budgetOf(s)} verb="push" breakdown={breakdown(s)} />
          <PushButton s={s} sentence={sentence} />
        </div>
      ) : (
        <>
          <ReviewBudgetGauge budget={budgetOf(s)} verb="push" breakdown={breakdown(s)} />
          <PushButton s={s} sentence={sentence} />
        </>
      )}
    </footer>
  );
}

/** How many agents travel, or that the setting keeps them back. */
function AgentsFact({ s }: { s: Scenario }) {
  if (s.agentCount === 0) return null;
  return s.agentsOmitted ? (
    <span className="mk-fact off" title="Subagents stay on this computer: “Push Claude subagents” is off in Settings.">
      <Icon name="git-fork" size={12} />off
    </span>
  ) : (
    <span className="mk-fact" title={`${s.agentCount} subagents travel with this session · ${formatBytes(s.agentBytes)}`}>
      <Icon name="git-fork" size={12} /><b>{s.agentCount}</b>
    </span>
  );
}

function StatusMark() {
  return (
    <span className="sync-thread-sync-indicator local" tabIndex={0} aria-label="Newer on this computer">
      <ThreadStatusIcon status="local-outstanding" size={18} />
    </span>
  );
}

const REVIEW_CLASS = "sync-inline-action-review sync-inline-push-review sync-sync-review-workspace";
const HEADER_CLASS = "sync-inline-action-header sync-push-resource-header sync-sync-review-header";
const CONTENT_CLASS = "sync-inline-action-content sync-push-resource-content sync-sync-review-content";

/* ——— Before: the shipped components, as the session scope mounts them ——— */

const RESOURCE = {
  resource_id: "claude:session:shorten-automatic-updates-copy",
  display_name: TITLE,
  provider: PROVIDER,
  category: "conversations",
  kind_label: "Claude Code session",
  apply_policy_label: "Added beside existing sessions",
  selectable: true,
  default_selected: true,
} as unknown as InventoryResource;

function BeforeReview({ s, redaction }: { s: Scenario; redaction: boolean }) {
  const budget = budgetOf(s);
  const selected = new Set([RESOURCE.resource_id]);
  const sentence = budget.total > budget.limit ? reviewBudgetSentence(budget, "push") : null;
  return (
    <section className={REVIEW_CLASS} aria-label="Push review">
      <header className={HEADER_CLASS}>
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <RedactionFlag on={redaction} />
        </div>
        <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown(s)} />
        <SyncReviewSelectionBar
          count={1}
          total={1}
          busy={s.busy}
          recommendedTitle="Use the recommended session selection (1)"
          recommendedCount={1}
          recommendedDisabled
          onRecommended={noop}
          clearDisabled={false}
          onClear={noop}
          close={{ ariaLabel: "Close push review", onClose: noop }}
        />
      </header>
      <div className={CONTENT_CLASS}>
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <NameSearchField query="" onChange={noop} label="Search sessions" matches={1} total={1} noun={sessionNoun} />
          <ResourceInventory
            resources={[RESOURCE]}
            selected={selected}
            selectableResourceIds={selected}
            disabled={s.busy}
            onToggle={noop}
          />
        </div>
      </div>
      <ReviewFooter s={s} sentence={sentence} />
    </section>
  );
}

/* ——— A: the shipped review, on a fixture comparison ———————————————————— */

const THREAD_ID = "019f7798-5437-7632-9dbc-5b589cf68b10";
/* 2026-10-03T04:03:00Z, 21:03 on Oct 2 in the owner's zone. */
const ENDED_AT = 1_791_000_180;

const FACTS: PushSessionFacts = {
  title: TITLE,
  metrics: {
    started_at: ENDED_AT - 22 * 60,
    ended_at: ENDED_AT,
    is_active: false,
    user_round_count: 23,
    agent_message_count: 23,
    tool_call_count: 31,
    total_tokens: 184_000,
    metrics_complete: true,
  },
};

function entryOf(s: Scenario): ThreadSyncEntry {
  return {
    provider: PROVIDER,
    provider_display_name: PROVIDER_NAME,
    access: "bound_ready",
    thread_id: THREAD_ID,
    conversation_resource_id: `claude:conversation:${THREAD_ID}`,
    resource_id: RESOURCE.resource_id,
    display_name: TITLE,
    state: "local_only",
    local_present: true,
    storage_present: false,
    compared_against_storage: true,
    subagent_selection: {
      wire_children: s.agentsOmitted ? "off" : "on",
      keys: s.agentsOmitted ? [] : Array.from({ length: s.agentCount }, (_, index) => `agent-${index}`),
      omitted: s.agentsOmitted,
      estimated_bytes: s.agentBytes,
    },
    local_updated_at: ENDED_AT,
    push_decision: { action: "create_cloud", selectable: true },
    pull_decision: { action: "no_changes", selectable: false },
    capture_bytes: s.transcript + s.agentBytes,
  };
}

function comparisonOf(entry: ThreadSyncEntry): ThreadSyncComparison {
  return {
    project_id: "project-mallard",
    storage_id: "storage-cloud",
    storage_name: STORAGE,
    compared_at: ENDED_AT,
    entries: [entry],
    counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    capture_budget: { limit_bytes: LIMIT, reserved_bytes: RESERVED },
    remote_continuity: { state: "in_sync", revision: 1 },
  };
}

const PUSHING: BundleFetchProgress = {
  phase: "push",
  storage_id: "storage-cloud",
  bundle_id: "bundle-mallard",
  index: 3,
  total_objects: 7,
  fetched_bytes: 800_000,
  total_bytes: 1_300_000,
};

function ReviewA({ s, redaction }: { s: Scenario; redaction: boolean }) {
  const entry = entryOf(s);
  return (
    <SessionPushReview
      credentialRedactionEnabled={redaction}
      entry={entry}
      facts={FACTS}
      storageName={STORAGE}
      selected={new Set([entry.resource_id])}
      threadComparison={comparisonOf(entry)}
      busy={s.busy}
      fetchProgress={s.busy ? PUSHING : null}
      onClose={noop}
      onPush={noop}
    />
  );
}

/* ——— B: what the Push writes, as a ledger ——————————————————————————— */

function ReviewB({ s, redaction }: { s: Scenario; redaction: boolean }) {
  const budget = budgetOf(s);
  const sentence = budget.total > budget.limit ? sessionOverSentence(s) : null;
  return (
    <section className={`${REVIEW_CLASS} mk-b`} aria-label="Push review">
      <header className={HEADER_CLASS}>
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <RedactionFlag on={redaction} />
        </div>
        <CloseCluster busy={s.busy} />
      </header>
      <div className={CONTENT_CLASS}>
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <div className="mk-pack">
            <div className="mk-pack-head">
              <StatusMark />
              <strong>{TITLE}</strong>
              <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} />
              <time>{WHEN}</time>
            </div>
            <dl className="mk-pack-rows">
              <div>
                <dt><Icon name="message" size={14} />Transcript</dt>
                <dd>{formatBytes(s.transcript)}</dd>
              </div>
              {s.agentCount > 0 && (
                <div className={s.agentsOmitted ? "off" : undefined}>
                  <dt title={s.agentsOmitted ? "“Push Claude subagents” is off in Settings." : undefined}>
                    <Icon name="git-fork" size={14} />
                    {s.agentsOmitted ? "Agents off" : `${s.agentCount} agents`}
                  </dt>
                  <dd>{s.agentsOmitted ? "—" : formatBytes(s.agentBytes)}</dd>
                </div>
              )}
              <div>
                <dt><Icon name="settings" size={14} />Requirements</dt>
                <dd>{formatBytes(RESERVED)}</dd>
              </div>
            </dl>
          </div>
        </div>
      </div>
      <ReviewFooter s={s} sentence={sentence} />
    </section>
  );
}

/* ——— C: no body ———————————————————————————————————————————————————— */

function ReviewC({ s, redaction }: { s: Scenario; redaction: boolean }) {
  const budget = budgetOf(s);
  const sentence = budget.total > budget.limit ? sessionOverSentence(s) : null;
  return (
    <section className={`${REVIEW_CLASS} mk-c`} aria-label={`Push review: ${TITLE}`}>
      <header className={HEADER_CLASS}>
        <div className="sync-sync-review-title">
          <StatusMark />
          <h2 title={TITLE}>{TITLE}</h2>
          <span className="mk-c-provider" title={`${PROVIDER_NAME} session`}>
            <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} />
          </span>
        </div>
        <CloseCluster busy={s.busy} />
      </header>
      <ReviewFooter
        s={s}
        sentence={sentence}
        facts={<><RedactionFlag on={redaction} /><AgentsFact s={s} /></>}
      />
    </section>
  );
}

/* ——— D: one row, docked on the session page ————————————————————————— */

function ReviewD({ s, redaction }: { s: Scenario; redaction: boolean }) {
  const budget = budgetOf(s);
  const sentence = budget.total > budget.limit ? sessionOverSentence(s) : null;
  return (
    <section className={`${REVIEW_CLASS} mk-d`} aria-label="Push review">
      {/* The review's own header, carrying the footer's cluster. */}
      <header className={HEADER_CLASS}>
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <RedactionFlag on={redaction} />
          <AgentsFact s={s} />
        </div>
        <div className="mk-d-end">
          <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown(s)} />
          <PushButton s={s} sentence={sentence} />
        </div>
        <CloseCluster busy={s.busy} />
        {sentence && <p className="sync-review-budget-sentence">{sentence}</p>}
        {s.busy && <p className="mk-d-note" role="status">{BUSY_SENTENCE}</p>}
      </header>
    </section>
  );
}

/** The session page at rest: its fact strip, the SHOW bar, the first turn. */
function SessionPage({ s }: { s: Scenario }) {
  const toggle = (label: string, count: string, on: boolean) => (
    <button type="button" className={`session-detail-filter-toggle${on ? " is-on" : ""}`} aria-pressed={on}>
      <span className="session-detail-filter-box" aria-hidden="true" />
      <span className="session-detail-filter-name">{label}</span>
      <span className="session-detail-filter-count quality-exact">{count}</span>
    </button>
  );
  return (
    <section className="session-detail-page" aria-label={TITLE}>
      <header className="session-detail-header">
        <div className="session-detail-strip">
          <div className="session-detail-chips" aria-label="Session facts">
            <span className="session-detail-chip"><Icon name="git-branch" size={12} /><code>main</code></span>
            <span className="session-detail-chip"><Icon name="git-commit" size={12} /><code>4c1e9a2</code></span>
            <span className="session-detail-chip"><time>Oct 2, 2026 · 20:41</time></span>
            <span className="session-detail-chip"><Icon name="clock" size={12} /><b>22m</b></span>
            <span className="session-detail-chip"><code>184K</code> tokens</span>
            {s.agentCount > 0 && (
              <span className="session-detail-chip"><Icon name="git-fork" size={12} /><code>{s.agentCount}</code>agents</span>
            )}
          </div>
          <div className="session-detail-header-actions" aria-label="Session actions">
            <button type="button" className="btn btn-ghost session-detail-launch" aria-label={`Open in ${PROVIDER_NAME}`}>
              <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} size={12} />
            </button>
          </div>
        </div>
      </header>
      <div className="session-detail-layout">
        <div className="session-detail-toolbar">
          <span className="session-detail-toolbar-label">Show</span>
          <div className="session-detail-toolbar-toggles" role="group" aria-label="Show">
            {toggle("Prompts", "4", true)}
            {toggle("Replies", "4", true)}
            {toggle("Tool calls", "31", false)}
          </div>
          <output className="session-detail-toolbar-readout">8 of 39 events</output>
        </div>
        <div className="session-detail-days">
          <div className="session-detail-day">
            <h2 className="session-detail-day-divider"><span>Oct 2, 2026</span></h2>
            <ol className="session-detail-event-list">
              <li>
                <article className="session-detail-event event-prompt">
                  <span className="session-detail-event-node" role="img" aria-label="You · 20:41"><Icon name="user" size={12} /></span>
                  <div className="session-detail-event-body">
                    <div className="session-detail-card session-detail-card-prompt">
                      <div className="session-detail-card-head"><time>20:41</time><span>6m</span><span>12 calls</span></div>
                      <div className="session-detail-event-copy">
                        <p>The automatic updates section in Settings runs to three sentences. Cut it to one line per switch.</p>
                      </div>
                    </div>
                  </div>
                </article>
              </li>
              <li>
                <article className="session-detail-event event-response">
                  <span className="session-detail-event-node" role="img" aria-label={`${PROVIDER_NAME} · 20:47`}>
                    <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} size={12} />
                  </span>
                  <div className="session-detail-event-body">
                    <div className="session-detail-card session-detail-card-reply">
                      <div className="session-detail-card-head"><time>20:47</time></div>
                      <div className="session-detail-event-copy">
                        <p>Each switch now carries one line. The install note moved to the button’s tooltip, and the two tests that quoted the old sentences read the new ones.</p>
                      </div>
                    </div>
                  </div>
                </article>
              </li>
            </ol>
          </div>
        </div>
      </div>
    </section>
  );
}

/* ——— the app around each review —————————————————————————————————— */

function PathActions({ s, overLimit }: { s: Scenario; overLimit: boolean }) {
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
        <button type="button" className="sync-path-sync active" disabled={s.busy || overLimit} aria-expanded>
          <Icon name={s.busy ? "refresh" : "upload"} size={14} className={s.busy ? "icon-spin" : undefined} />
          <span className="sync-path-sync-label">{s.busy ? "Pushing…" : "Push"}</span>
        </button>
      </div>
    </>
  );
}

const PROJECT_CRUMBS: PathBarCrumb[] = [
  { id: "storage", label: STORAGE, icon: "storage-mallard" },
  { id: "project", label: PROJECT, icon: "git-folder" },
];

const SESSION_CRUMBS: PathBarCrumb[] = [
  ...PROJECT_CRUMBS,
  {
    id: "session",
    label: TITLE,
    leading: (
      <span className="sync-path-leaf-mark">
        <ProviderMark provider={PROVIDER} displayName={PROVIDER_NAME} />
      </span>
    ),
  },
];

function Workspace({ dir, s, redaction }: { dir: Dir; s: Scenario; redaction: boolean }) {
  const budget = budgetOf(s);
  const actions = <PathActions s={s} overLimit={budget.total > budget.limit} />;
  if (dir === "d") {
    return (
      <>
        <SyncPathBar crumbs={SESSION_CRUMBS} actions={actions} ariaLabel={`${PROJECT} path`} />
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <div className="mk-d-dock"><ReviewD s={s} redaction={redaction} /></div>
          <SessionPage s={s} />
        </main>
      </>
    );
  }
  const review = dir === "before" ? <BeforeReview s={s} redaction={redaction} />
    : dir === "a" ? <ReviewA s={s} redaction={redaction} />
      : dir === "b" ? <ReviewB s={s} redaction={redaction} />
        : <ReviewC s={s} redaction={redaction} />;
  return (
    <>
      <SyncPathBar crumbs={PROJECT_CRUMBS} actions={actions} ariaLabel={`${PROJECT} path`} />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
        <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
          <div className="profile-links-list sync-combined-settings-panel">
            <article className="profile-link-card sync-project-link-card">
              <div className="profile-link-connections">
                <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                  <div className="project-profile-storage-list">
                    <div className="storage-link-block selected sync-review-open">
                      <div className="sync-storage-inline-review" tabIndex={-1}>{review}</div>
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
  const s = SCENARIOS[knobs.session];
  const shown = knobs.only === "all" ? DIRS : [knobs.only];
  return (
    <div className="spec">
      {knobs.shot === "off" && (
        <>
          <h1 className="spec-title">Push from a session’s page: the review for one session</h1>
          <p className="spec-sub">
            Every frame is the main pane at the chosen window width, drawn with the shipped path bar, header, gauge and
            button on the shipped CSS. Only the classes prefixed <code>mk-</code> are new.
          </p>

          <div className="spec-controls">
            <div className="spec-group"><span>Session</span>{SESSIONS.map((id) => seg("session", id, SESSION_LABEL[id]))}</div>
            <div className="spec-group"><span>Redaction</span>{seg("redaction", "off", "Off")}{seg("redaction", "on", "On")}</div>
            <div className="spec-group"><span>Pane</span>{seg("width", "wide", "1100 px window")}{seg("width", "narrow", "800 px window")}</div>
            <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
          </div>
        </>
      )}

      <div className="spec-grid">
        {shown.map((dir) => (
          <div key={dir} data-dir={dir} style={{ width }}>
            <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "d" && <em>behaviour change</em>}</h2>
            <p className="spec-col-note">{DIR_NOTE[dir]}</p>
            <div className={`spec-frame${knobs.width === "narrow" ? " narrow" : ""}`}>
              <Workspace dir={dir} s={s} redaction={knobs.redaction === "on"} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
