import { StrictMode, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import NameSearchField from "./components/sync-review/NameSearchField";
import ProviderMark from "./components/sync-review/ProviderMark";
import { ReviewBudgetGauge, ReviewBudgetGlance } from "./components/sync-review/ReviewBudgetMeter";
import SyncPathBar from "./components/sync-review/SyncPathBar";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import { formatBytes } from "./format";
import "./App.css";

/**
 * Mockup: the review header's recommended-selection control is easy to miss.
 *
 * It is a 15px check-circle drawn as a ghost button, between the "0/3" count
 * and the trash, with its meaning only in a tooltip. A Pull review opens with
 * nothing ticked, and at zero its primary reads "Mark as reviewed", so a
 * reader who never finds the check marks the version reviewed without pulling
 * anything.
 *
 *   Before  what ships
 *   A       a labelled hollow pill in the same slot: ✓ Select [3]
 *   B       A, inked while nothing is ticked
 *   C       a select-all box at the head of the list's checkbox column
 *           (a behaviour change: the box both applies and clears)
 *
 * A shipped (2026-09-27): the page now runs on the shipped CSS, so the Push
 * header glance has no track and the 620px glance rule applies to every
 * frame, Before included; Before's selection cluster is drawn inline.
 *
 * `npm run dev`, then /review-select-mockup.html
 * (`?verb=pull|push&dir=before|a|b|c&state=empty|some|match|search&width=wide|narrow&view=matrix|context&theme=light|dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Verb = "pull" | "push";
type Dir = "before" | "a" | "b" | "c";
type State = "empty" | "some" | "match" | "search";
type Width = "wide" | "narrow";
type View = "matrix" | "context";
interface Knobs { verb: Verb; dir: Dir; state: State; width: Width; view: View; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    verb: pick("verb", ["pull", "push"] as const, "pull"),
    dir: pick("dir", ["before", "a", "b", "c"] as const, "a"),
    state: pick("state", ["empty", "some", "match", "search"] as const, "empty"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    view: pick("view", ["matrix", "context"] as const, "matrix"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const DIRS: readonly Dir[] = ["before", "a", "b", "c"];
const STATES: readonly State[] = ["empty", "some", "match", "search"];

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (shipped)",
  a: "A · labelled pill",
  b: "B · pill, inked at zero",
  c: "C · select-all box",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>A 15px ghost check-circle between the count and the trash. Its meaning is only in the tooltip.</>,
  a: <>The same control, the same slot, the same behaviour, now with a label: <b>✓ Select [3]</b>. It uses the path bar’s hollow pill (2px ink border, cast shadow), one step smaller, and never the gold. The number is the recommended count, the same one the tooltip gives today, so it doesn’t change as rows are ticked. Clear stays the trash icon.</>,
  b: <>A’s pill, inked while nothing is ticked. That is how a Pull review opens, and the state where the primary reads “Mark as reviewed”. After the first tick it returns to A’s hollow pill.</>,
  c: <><b>Behaviour change.</b> The header loses ✓ and the trash. A tri-state box at the top of the list’s checkbox column does both jobs: empty or partly ticked, it applies the recommended selection; ticked, it clears. This follows the folder bands in the files review, but on a capped Push list “ticked” means “the newest 71 that fit”, not “everything”.</>,
};

const STATE_LABEL: Record<State, string> = {
  empty: "Nothing ticked",
  some: "Some ticked",
  match: "Matches recommended",
  search: "Search active",
};

/* ——— data ——————————————————————————————————————————————————————— */

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
const PROJECT = "echo-park-horror";
const STORAGE = "Attic R2";

type Provider = "claude" | "codex";
const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude Code", codex: "Codex" };

interface MockSession {
  id: string;
  title: string;
  provider: Provider;
  bytes: number;
  when: string;
  messages: number;
  tools: number;
  tokens: string;
  length: string;
}

/** Only in Attic R2: a Pull review lists these, and opens with none ticked. */
const PULL_ROWS: MockSession[] = [
  { id: "p1", title: "Tune the lantern flicker shader", provider: "claude", bytes: 42.3 * MB, when: "Sep 25, 22:14", messages: 164, tools: 131, tokens: "1.2M", length: "1h 52m" },
  { id: "p2", title: "Rework the basement chase AI", provider: "codex", bytes: 118 * MB, when: "Sep 24, 19:03", messages: 402, tools: 377, tokens: "3.4M", length: "5h 10m" },
  { id: "p3", title: "Fix save-slot corruption on quit", provider: "claude", bytes: 9.6 * MB, when: "Sep 23, 16:40", messages: 58, tools: 44, tokens: "380K", length: "41m" },
];

/** The first rows of a long Push backlog: 347 sessions, 2.6 GB, of which the
 *  newest 71 fit one push (the probe's real shape, 2026-09-04). */
const PUSH_ROWS: MockSession[] = [
  { id: "q1", title: "Ghost pathing through locked doors", provider: "claude", bytes: 9.6 * MB, when: "Sep 26, 11:20", messages: 388, tools: 341, tokens: "3.0M", length: "5h 02m" },
  { id: "q2", title: "Menu flow and pause screen", provider: "codex", bytes: 5.8 * MB, when: "Sep 25, 17:02", messages: 190, tools: 152, tokens: "1.7M", length: "2h 48m" },
  { id: "q3", title: "Lantern battery balance", provider: "claude", bytes: 4.1 * MB, when: "Sep 25, 09:47", messages: 133, tools: 97, tokens: "1.1M", length: "1h 36m" },
  { id: "q4", title: "Crash on alt-tab in fullscreen", provider: "claude", bytes: 2.3 * MB, when: "Sep 24, 21:15", messages: 71, tools: 60, tokens: "620K", length: "58m" },
];
const PUSH_TOTAL = 347;
const PUSH_RECOMMENDED = 71;
const PUSH_RECOMMENDED_BYTES = 493 * MB;
const PUSH_RESERVED = 0.4 * MB;
const QUERY = "lantern";

interface Scenario {
  rows: MockSession[];
  /** Rows drawn ticked. */
  ticked: Set<string>;
  count: number;
  total: number;
  /** The recommend button's own count: a property of the plan, not the ticks. */
  recommended: number;
  matchesRecommended: boolean;
  query: string;
  bytes: number;
}

function scenario(verb: Verb, state: State): Scenario {
  const pull = verb === "pull";
  const rows = pull ? PULL_ROWS : PUSH_ROWS;
  const query = state === "search" ? QUERY : "";
  const shown = query ? rows.filter((row) => row.title.toLowerCase().includes(query)) : rows;
  const total = pull ? rows.length : PUSH_TOTAL;
  const ticked = new Set<string>(
    state === "some" ? rows.slice(0, pull ? 1 : 2).map((row) => row.id)
      : state === "match" ? rows.map((row) => row.id)
        : [],
  );
  const count = state === "match" ? (pull ? rows.length : PUSH_RECOMMENDED) : ticked.size;
  const tickedBytes = rows.filter((row) => ticked.has(row.id)).reduce((sum, row) => sum + row.bytes, 0);
  const bytes = pull ? tickedBytes : state === "match" ? PUSH_RECOMMENDED_BYTES : PUSH_RESERVED + tickedBytes;
  return {
    rows: shown,
    ticked,
    count,
    total,
    recommended: query ? shown.length : pull ? rows.length : PUSH_RECOMMENDED,
    matchesRecommended: state === "match",
    query,
    bytes,
  };
}

function recommendedTitle(verb: Verb, searchActive: boolean, count: number): string {
  return verb === "pull"
    ? `Select the newest ${searchActive ? "matching " : ""}sessions that fit this pull (${count})`
    : `Use the recommended ${searchActive ? "matching session" : "session"} selection (${count})`;
}

function breakdown(verb: Verb, bytes: number): string {
  return verb === "pull"
    ? `Sessions ${formatBytes(bytes)} · Setup & tools 0 B · Project files 0 B`
    : `Sessions ${formatBytes(bytes - PUSH_RESERVED)} · Requirements ${formatBytes(PUSH_RESERVED)} · Project files 0 B`;
}

function primaryLabel(verb: Verb, count: number): string {
  if (verb === "pull") return count > 0 ? `Pull ${count}` : "Mark as reviewed";
  return count > 0 ? `Push ${count}` : "Push";
}

/* ——— the header cluster, per direction ————————————————————————————— */

function SelectionCluster({ dir, verb, s }: { dir: Dir; verb: Verb; s: Scenario }) {
  const searchActive = s.query !== "";
  const title = recommendedTitle(verb, searchActive, s.recommended);
  const ariaLabel = `Use recommended selection${searchActive ? " for matching sessions" : ""} (${s.recommended})`;
  const clearDisabled = s.ticked.size === 0 && s.count === 0;
  const close = { ariaLabel: `Close ${verb} review`, onClose: () => undefined };
  if (dir === "before") {
    /* What shipped before A, drawn inline: the live component is A now. */
    return (
      <div className="sync-push-resource-actions">
        <span
          className="sync-sync-review-selected"
          style={{ "--review-count-digits": String(s.total).length } as CSSProperties}
          aria-label={`${s.count} of ${s.total} selected`}
          title={`${s.count} of ${s.total} selected`}
        >
          <b>{s.count}</b>/{s.total}
        </span>
        <button type="button" className="btn btn-ghost sync-sync-review-icon-action" disabled={s.matchesRecommended} title={title} aria-label={ariaLabel}>
          <Icon name="check-circle" size={15} />
        </button>
        <button type="button" className="btn btn-ghost sync-sync-review-icon-action" disabled={clearDisabled} aria-label="Clear current selection">
          <Icon name="trash" size={15} />
        </button>
        <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label={close.ariaLabel}>
          <Icon name="x" size={15} />
        </button>
      </div>
    );
  }
  const count = (
    <span
      className="sync-sync-review-selected"
      style={{ "--review-count-digits": String(s.total).length } as CSSProperties}
      aria-label={`${s.count} of ${s.total} selected`}
      title={`${s.count} of ${s.total} selected`}
    >
      <b>{s.count}</b>/{s.total}
    </span>
  );
  const closeButton = (
    <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label={close.ariaLabel}>
      <Icon name="x" size={15} />
    </button>
  );
  if (dir === "c") return <div className="sync-push-resource-actions">{count}{closeButton}</div>;
  return (
    <div className="sync-push-resource-actions">
      {count}
      <button
        type="button"
        className={`btn mk-select${dir === "b" && s.count === 0 ? " mk-prompt" : ""}`}
        disabled={s.matchesRecommended}
        title={title}
        aria-label={ariaLabel}
      >
        <Icon name="check-circle" size={14} />
        <span className="mk-select-word">Select</span>
        <span className="sync-path-count" aria-hidden="true">{s.recommended}</span>
      </button>
      <button
        type="button"
        className="btn btn-ghost sync-sync-review-icon-action"
        disabled={clearDisabled}
        title={searchActive ? "Clear matching selection" : "Clear current selection"}
        aria-label={searchActive ? "Clear matching selection" : "Clear current selection"}
      >
        <Icon name="trash" size={15} />
      </button>
      {closeButton}
    </div>
  );
}

/** C's select-all row: its box sits on the rows' own checkbox axis. */
function MasterRow({ verb, s }: { verb: Verb; s: Scenario }) {
  const box = useRef<HTMLInputElement>(null);
  const shownTicked = s.rows.filter((row) => s.ticked.has(row.id)).length;
  const all = s.matchesRecommended;
  const some = !all && shownTicked > 0;
  useEffect(() => {
    if (box.current) box.current.indeterminate = some;
  }, [some]);
  const title = all
    ? `Clear the ${s.query ? "matching " : ""}selection`
    : recommendedTitle(verb, s.query !== "", s.recommended);
  return (
    <div className="mk-master">
      <label className="sync-sync-row-choice" title={title}>
        <input ref={box} type="checkbox" checked={all} readOnly aria-label={title} />
      </label>
      <span className="mk-master-label">
        {all
          ? <><b>Newest that fit</b><small>{s.recommended} selected · untick to clear</small></>
          : <><b>Select the newest that fit</b><small>{s.recommended}{s.query ? " matching" : ""}</small></>}
      </span>
    </div>
  );
}

/* ——— rows, in the review's own markup ———————————————————————————— */

function SessionRow({ session, verb, selected }: { session: MockSession; verb: Verb; selected: boolean }) {
  const pull = verb === "pull";
  return (
    <article className={`sync-history-thread-card sync-sync-selectable-row${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <label className="sync-sync-row-choice mk-row-choice" title={`${selected ? "Exclude" : "Include"} ${session.title}`}>
            <input type="checkbox" checked={selected} readOnly aria-label={`${selected ? "Exclude" : "Include"} ${session.title}`} />
          </label>
          <span className={`sync-thread-sync-indicator ${pull ? "storage" : "local"}`} tabIndex={0}>
            <ThreadStatusIcon status={pull ? "cloud-outstanding" : "local-outstanding"} size={18} />
          </span>
          <strong>{session.title}</strong>
        </div>
        <div className="sync-history-thread-actions">
          {pull && <span className="sync-history-branch-state history-only">Add session</span>}
          <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label={`Open in ${PROVIDER_NAME[session.provider]}`}>
            <ProviderMark provider={session.provider} displayName={PROVIDER_NAME[session.provider]} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Session details">
            <span className="sync-history-thread-metric"><Icon name="message" size={12} /><b>{session.messages}</b></span>
            <span className="sync-history-thread-metric"><Icon name="tool" size={12} /><b>{session.tools}</b></span>
            <span className="sync-history-thread-metric"><Icon name="token" size={12} /><b>{session.tokens}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-duration"><Icon name="clock" size={12} /><b>{session.length}</b></span>
            <span className="sync-history-thread-metric"><Icon name="drive" size={12} /><b>{formatBytes(session.bytes)}</b></span>
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="View session details">
              <Icon name="chevron-right" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end">
            <time>{session.when}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

function ReviewList({ dir, verb, s, rows }: { dir: Dir; verb: Verb; s: Scenario; rows: boolean }) {
  const list = useRef<HTMLDivElement>(null);
  /* C: put the select-all box on the rows' checkbox axis, measured. */
  useLayoutEffect(() => {
    const root = list.current;
    if (!root || dir !== "c") return;
    const measure = () => {
      const master = root.querySelector<HTMLElement>(".mk-master");
      const row = root.querySelector<HTMLElement>(".mk-row-choice");
      if (!master || !row) return;
      const inset = row.getBoundingClientRect().left - master.getBoundingClientRect().left;
      master.style.setProperty("--mk-master-inset", `${Math.round(inset)}px`);
    };
    measure();
    void document.fonts.ready.then(measure);
  }, [dir, verb, s, rows]);
  const pull = verb === "pull";
  const drawn = rows ? s.rows : [];
  const body = drawn.map((row) => <SessionRow key={row.id} session={row} verb={verb} selected={s.ticked.has(row.id)} />);
  return (
    <div ref={list} className="sync-history-page sync-history-embedded sync-history-selection-review">
      <section className="sync-history-content" aria-label="Sessions">
        <NameSearchField
          query={s.query}
          onChange={() => undefined}
          label="Search sessions"
          matches={s.rows.length}
          total={pull ? PULL_ROWS.length : PUSH_TOTAL}
        />
        {dir === "c" && <MasterRow verb={verb} s={s} />}
        {rows && (pull ? (
          <section className="sync-history-storage-only" aria-label={`Only in ${STORAGE}`}>
            <div className="sync-history-section-heading">
              <h2><Icon name="cloud" size={13} />Only in <span className="sync-history-storage-name">{STORAGE}</span></h2>
              <span className="sync-history-heading-count"><Icon name="message" size={12} />{s.rows.length}</span>
            </div>
            <div className="sync-history-thread-list flat">{body}</div>
          </section>
        ) : (
          <div className="sync-history-thread-list flat">{body}</div>
        ))}
        {/* In C the measurement needs a row even when only the header is
            drawn; one hidden row keeps the axis. */}
        {!rows && dir === "c" && (
          <div style={{ height: 0, overflow: "hidden" }} aria-hidden="true">
            {pull ? (
              <section className="sync-history-storage-only">
                <div className="sync-history-thread-list flat"><SessionRow session={PULL_ROWS[0]} verb={verb} selected={false} /></div>
              </section>
            ) : <div className="sync-history-thread-list flat"><SessionRow session={PUSH_ROWS[0]} verb={verb} selected={false} /></div>}
          </div>
        )}
      </section>
    </div>
  );
}

/* ——— the review and the workspace ———————————————————————————————— */

function Review({ dir, verb, s, body }: { dir: Dir; verb: Verb; s: Scenario; body: "full" | "head" }) {
  const pull = verb === "pull";
  const budget = { total: s.bytes, limit: LIMIT };
  const label = primaryLabel(verb, s.count);
  return (
    <section
      className={`sync-inline-action-review ${pull ? "sync-inline-pull-review" : "sync-inline-push-review"} sync-sync-review-workspace${dir === "before" ? "" : " mk-proposed"}${dir === "a" || dir === "b" ? " mk-pill" : ""}`}
      aria-label={pull ? "Pull review" : "Push review"}
    >
      <header className={`sync-inline-action-header ${pull ? "sync-inline-pull-header" : "sync-push-resource-header"} sync-sync-review-header`}>
        <div className="sync-inline-action-heading">
          <div className="sync-sync-review-title">
            <h2>{pull ? "Pull review" : "Push review"}</h2>
            {pull
              ? <span className="sync-pull-review-meta">Version 12</span>
              : <span className="sync-settings-flag">Credential redaction: On</span>}
          </div>
        </div>
        <ReviewBudgetGlance budget={budget} verb={verb} breakdown={breakdown(verb, s.bytes)} />
        <SelectionCluster dir={dir} verb={verb} s={s} />
      </header>
      {(body === "full" || dir === "c") && (
        <div className={`${pull ? "sync-pull-review-content" : "sync-push-resource-content"} sync-inline-action-content sync-sync-review-content`}>
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            <ReviewList dir={dir} verb={verb} s={s} rows={body === "full"} />
          </div>
        </div>
      )}
      {body === "full" && (
        <footer className={`${pull ? "sync-pull-apply-bar " : "sync-push-resource-footer "}sync-inline-action-footer sync-sync-review-footer`}>
          <ReviewBudgetGauge budget={budget} verb={verb} breakdown={breakdown(verb, s.bytes)} />
          <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={!pull && s.count === 0}>
            <ThreadStatusIcon status={pull ? "cloud-outstanding" : "local-outstanding"} size={16} />
            {label}
          </button>
        </footer>
      )}
    </section>
  );
}

function PathActions({ verb, s }: { verb: Verb; s: Scenario }) {
  const button = (kind: Verb) => {
    const mine = kind === verb;
    return (
      <button
        type="button"
        className={`sync-path-sync${mine ? " active" : ""}`}
        disabled={mine ? verb === "push" && s.count === 0 : true}
        aria-expanded={mine}
      >
        <Icon name={kind === "pull" ? "download" : "upload"} size={14} />
        <span className="sync-path-sync-label">{mine ? primaryLabel(verb, s.count) : kind === "pull" ? "Pull" : "Push"}</span>
      </button>
    );
  };
  return (
    <>
      <button type="button" className="sync-path-icon-action" disabled aria-label="Refresh activity" title="Refresh activity">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        {button("pull")}
        {button("push")}
      </div>
    </>
  );
}

function Workspace({ dir, verb, s }: { dir: Dir; verb: Verb; s: Scenario }) {
  return (
    <>
      <SyncPathBar
        crumbs={[
          { id: "projects", label: "Projects", tone: "root" },
          { id: "project", label: PROJECT, icon: "folder" },
          { id: "storage", label: STORAGE, icon: "storage-r2" },
          { id: "section", label: "Git history", tone: "section" },
        ]}
        actions={<PathActions verb={verb} s={s} />}
        ariaLabel={`${PROJECT} path`}
      />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
        <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
          <div className="profile-links-list sync-combined-settings-panel">
            <article className="profile-link-card sync-project-link-card">
              <div className="profile-link-connections">
                <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                  <div className="project-profile-storage-list">
                    <div className="storage-link-block selected sync-review-open">
                      <div className="sync-storage-inline-review" tabIndex={-1}>
                        <Review dir={dir} verb={verb} s={s} body="full" />
                      </div>
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
/** The review itself inside that pane (measured): the matrix draws only it. */
const REVIEW_WIDTH: Record<Width, number> = { wide: 684, narrow: 414 };

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
  const frameClass = `spec-frame${knobs.width === "narrow" ? " narrow" : ""}`;
  const s = scenario(knobs.verb, knobs.state);
  return (
    <div className="spec">
      <h1 className="spec-title">Review header: making “use the recommended selection” visible</h1>
      <p className="spec-sub">
        Drawn with the shipped header, meter, rows and path bar on the shipped CSS. Only the selection control is new
        (mock CSS in the page’s <code>&lt;style&gt;</code>). <b>Pull</b> is a short list (3 sessions, all fit).
        <b> Push</b> is a long backlog: 347 sessions, and the newest 71 fit one push, so its button’s number is not the total.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Verb</span>{seg("verb", "pull", "Pull · 3")}{seg("verb", "push", "Push · 347, 71 fit")}</div>
        <div className="spec-group"><span>View</span>{seg("view", "matrix", "Every state")}{seg("view", "context", "In the app")}</div>
        {knobs.view === "context" && (
          <>
            <div className="spec-group"><span>Direction</span>{DIRS.filter((dir) => dir !== "before").map((dir) => seg("dir", dir, DIR_LABEL[dir]))}</div>
            <div className="spec-group"><span>State</span>{STATES.map((state) => seg("state", state, STATE_LABEL[state]))}</div>
          </>
        )}
        <div className="spec-group"><span>Pane</span>{seg("width", "wide", "1100 px window")}{seg("width", "narrow", "800 px window")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel">
        <strong>Why it gets missed today</strong>
        <ol>
          <li><b>It doesn’t look like a button.</b> A 15px outline glyph with no edge, no fill and no word, in the same ink as the “0/3” beside it. It reads as a status mark.</li>
          <li><b>Its meaning is only in the tooltip</b> (“Select the newest sessions that fit this pull (3)”). The number it would tick is shown nowhere.</li>
          <li><b>The costly state is the default.</b> Pull opens with nothing ticked, and at zero the primary, in the path bar and the footer, reads <i>Mark as reviewed</i>. Missing ✓ means accepting the version without pulling.</li>
          <li>The same control ships in four places: Pull, Push, Library Push and Library Pull. Whatever changes here changes all four.</li>
        </ol>
      </div>

      {knobs.view === "matrix" ? (
        <div className="spec-matrix">
          {DIRS.map((dir) => (
            <div key={dir} className="spec-matrix-row">
              <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "a" && <em>recommended</em>}{dir === "c" && <em>behaviour change</em>}</h2>
              <p className="spec-col-note">{DIR_NOTE[dir]}</p>
              <div className="spec-matrix-cells">
                {STATES.map((state) => (
                  <div key={state} className="spec-cell">
                    <span>{STATE_LABEL[state]}</span>
                    <div className={frameClass} style={{ width: REVIEW_WIDTH[knobs.width] }}>
                      <Review dir={dir} verb={knobs.verb} s={scenario(knobs.verb, state)} body="head" />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="spec-pair">
          {(["before", knobs.dir] as const).map((dir) => (
            <div key={dir}>
              <h2 className="spec-col-title">{DIR_LABEL[dir]}</h2>
              <p className="spec-col-note" style={{ maxWidth: width }}>{DIR_NOTE[dir]}</p>
              <div className={frameClass} style={{ width }}>
                <Workspace dir={dir} verb={knobs.verb} s={s} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
