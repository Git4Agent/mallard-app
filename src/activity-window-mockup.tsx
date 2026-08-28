import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * Mockup for the Activity page's empty 30-day window.
 *
 * Today that state is one dashed box reading "No commits are available in this
 * 30-day window." and a "Load previous 30 days" button floating at the page
 * bottom. It is a dead end: it names what is missing, never what is there, and
 * the button gropes backwards a fixed 30 days at a time with no way to know
 * whether the next click lands on anything.
 *
 * Everything the page would need to do better is already in memory when the
 * response is built — `build_project_activity` loads every commit's timestamp
 * before it filters by window, and holds the full session list too. Nothing
 * below asks git a new question.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SHELL_CSS = `
  html, body { height: auto; overflow: auto; }
  #root { height: auto; min-height: 100%; overflow: auto; }
  .mk-page { max-width: 1180px; margin: 0 auto; padding: 28px 24px 96px; font-family: var(--font-sans); }
  /* The toggle is fixed at the top right, so the title reserves its column
     rather than sliding under it on a narrow window. */
  .mk-title { margin: 0 0 4px; padding-right: 96px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 26px; max-width: 74ch; color: var(--text-2); line-height: var(--leading-prose); }
  .mk-panel { margin: 0 0 30px; border: 1px solid var(--border-mid); border-radius: 6px; background: var(--surface-card); }
  .mk-panel > header { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;
    padding: 10px 16px; border-bottom: 1px solid var(--border-mid); background: var(--surface-subtle); }
  .mk-panel > header h2 { margin: 0; color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .mk-panel > header p { margin: 0; max-width: 78ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-tag { padding: 1px 7px; border: 1px solid var(--border-mid); border-radius: 999px;
    color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); text-transform: uppercase; letter-spacing: 0.06em; }
  .mk-tag.now { color: var(--text-3); }
  /* --accent is a near-black sepia in this palette, so it cannot mark a pick.
     This is mockup chrome, not app styling, so it borrows --blue outright. */
  .mk-tag.pick { border-color: var(--blue); color: var(--blue); }
  .mk-stage { padding: 18px 20px 22px; background: var(--bg-0); }
  .mk-notes { margin: 0; padding: 12px 16px; border-top: 1px solid var(--border-mid);
    color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-notes li + li { margin-top: 5px; }
  .mk-notes code { font-family: var(--font-mono); font-size: var(--type-2xs); color: var(--text-1); }
  .mk-split { display: grid; gap: 18px; }
  @media (min-width: 900px) { .mk-split.two { grid-template-columns: 1fr 1fr; } }
  .mk-caption { margin: 0 0 8px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); text-transform: uppercase; letter-spacing: 0.06em; }
  /* Fixed, because the page is five screens tall and the palette is the thing
     you most want to flip while looking at a panel halfway down it. */
  .mk-theme { position: fixed; z-index: 20; top: 18px; right: 20px;
    display: inline-flex; align-items: center; gap: 7px;
    padding: 6px 12px; border: 1px solid var(--border-mid); border-radius: 999px;
    background: var(--surface-card); color: var(--text-1);
    font-family: var(--font-mono); font-size: var(--type-2xs); text-transform: uppercase; letter-spacing: 0.06em;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.16); cursor: pointer; }
  .mk-theme:hover, .mk-theme:focus-visible { border-color: var(--border); background: var(--bg-hover); color: var(--text-0); }
  .mk-theme > svg { color: var(--text-2); }
`;

/* ------------------------------------------------------------------ *
 * Proposed CSS. Every class the shipped page already owns is used as
 * shipped; only the rules below are new, and each one says why.
 * ------------------------------------------------------------------ */
const PROPOSED_CSS = `
  /* 1 · Signpost. The dashed box keeps its frame and borrows the shape the
     page already uses for its failed-load state: glyph, two-line stack, one
     verb. Dashed, because absence is still what it marks. */
  .sync-history-window-state { justify-content: flex-start; text-align: left; }
  .sync-history-window-state > svg { flex-shrink: 0; color: var(--text-3); }
  .sync-history-window-state > div { min-width: 0; display: grid; flex: 1; gap: 3px; }
  .sync-history-window-state strong { color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .sync-history-window-state span { color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }

  /* 2 · The empty window as a rail card. Same frame, same fold column, same
     row geometry as a commit — so the page keeps its spine where the data
     stops. Muted fill and a dashed edge are what mark it as an absence; the
     rail must not break, because the rail is the page's structure. */
  .sync-history-commit.is-gap { border-style: dashed; background: var(--surface-subtle); }
  .sync-history-commit.is-gap .sync-history-commit-icon { color: var(--text-3); }
  .sync-history-commit.is-gap .sync-history-commit-title > strong { color: var(--text-2); font-weight: var(--weight-normal); }

  /* 2 · The rail terminus. The paging control belongs at the bottom of the
     rail it pages, in the rail's own column — not floating under the page
     after the Older activity fold, which is where it sits today. */
  .sync-history-rail-end {
    position: relative;
    /* A button shrink-wraps even as a flex container, so the width is stated:
       the card's width exactly, or the terminus reads as a different object
       from the rail it ends. */
    width: calc(100% - var(--sync-history-card-offset));
    min-height: 40px; display: flex; align-items: center; gap: 10px;
    margin: 0 0 0 var(--sync-history-card-offset); padding: 6px 12px;
    border: 1px dashed var(--border-mid); border-radius: 4px;
    background: transparent; color: var(--text-1); text-align: left;
  }
  .sync-history-rail-end:hover:not(:disabled) { border-color: var(--border); background: var(--bg-hover); }
  /* The rail runs out of the last card and stops 12px above this control's own
     node — the same gap every fold on the page is given. Stated as top+bottom
     rather than a height so it survives the container query's anchor change. */
  .sync-history-rail-end::before {
    content: ""; position: absolute; z-index: 0;
    top: calc(-1 * var(--sync-history-row-anchor-y));
    bottom: calc(100% - var(--sync-history-row-anchor-y) + 12px);
    left: calc(var(--sync-history-rail-x) - var(--sync-history-card-offset));
    width: 1px; background: var(--border-mid); pointer-events: none;
  }
  .sync-history-rail-end-node {
    position: absolute; z-index: 6;
    top: calc(var(--sync-history-row-anchor-y) - 12px);
    left: calc(var(--sync-history-rail-x) - var(--sync-history-card-offset) - 12px);
    width: 24px; height: 24px; display: grid; place-items: center;
    border-radius: 3px; background: var(--surface-card); color: var(--text-2);
  }
  .sync-history-rail-end-text { min-width: 0; display: grid; gap: 2px; }
  .sync-history-rail-end-text > strong { color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-bold); }
  .sync-history-rail-end-text > span { color: var(--text-2); font-size: var(--type-2xs); font-variant-numeric: tabular-nums; }
  /* Nothing older: no control, so no rail. The shipped rail already ends at
     the last commit's fold — a segment below it would be a connector to a
     commit that does not exist. This is a caption under the timeline, sharing
     only the card's left edge. */
  .sync-history-rail-floor {
    margin: 2px 0 0 var(--sync-history-card-offset); padding: 0 12px;
    /* --text-2, not --text-3: this is a statement the reader has to read, and
       --text-3 at --type-2xs measures 3.77:1 on the page ground in dark. */
    color: var(--text-2); font-size: var(--type-2xs); font-variant-numeric: tabular-nums;
  }

  /* 3 · Window bar. The loaded range printed on every page, empty or not,
     with the paging control at its right edge. */
  .sync-history-window-bar {
    display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap;
    min-height: 34px; margin: 0 0 10px var(--sync-history-card-offset); padding: 4px 6px 4px 12px;
    border: 1px solid var(--border-mid); border-radius: 4px; background: var(--surface-subtle);
  }
  .sync-history-window-bar > span { display: inline-flex; align-items: center; gap: 8px;
    color: var(--text-1); font-size: var(--type-sm); font-variant-numeric: tabular-nums; }
  .sync-history-window-bar > span > svg { color: var(--text-3); }
  .sync-history-window-bar em { color: var(--text-3); font-style: normal; }

  /* 4 · Auto-reach. No empty state exists, so there is nothing to style: the
     rail arrives full. All this adds is one line saying the window was widened
     to reach it, so the dates below are never a surprise. */
  .sync-history-reach-note {
    display: flex; align-items: center; gap: 8px;
    margin: 0 0 10px var(--sync-history-card-offset); padding: 0 12px;
    color: var(--text-2); font-size: var(--type-2xs); font-variant-numeric: tabular-nums;
  }
  .sync-history-reach-note > svg { flex: 0 0 auto; color: var(--text-3); }

  /* 5 · Range control. The window stops being a hidden scan bound and becomes
     a stated choice, so the empty state can point at the control that fixes
     it instead of owning a verb of its own. */
  .sync-history-range { display: inline-flex; align-items: center; gap: 0;
    border: 1px solid var(--border-mid); border-radius: 4px; overflow: hidden; }
  .sync-history-range > button {
    padding: 3px 11px; border: 0; border-radius: 0; background: transparent;
    color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); cursor: pointer;
  }
  .sync-history-range > button + button { border-left: 1px solid var(--border-mid); }
  .sync-history-range > button[aria-pressed="true"] { background: var(--surface-row); color: var(--text-0); font-weight: var(--weight-bold); }
  .sync-history-range > button:hover:not([aria-pressed="true"]) { background: var(--bg-hover); color: var(--text-0); }

  /* 7 · Activity strip. Paging becomes navigation: one bucket per month, the
     loaded window inked, everything else in the muted tone — so where the work
     actually is can be seen before anything is clicked. */
  .sync-history-strip {
    display: flex; align-items: flex-end; gap: 2px; height: 40px;
    margin: 0 0 4px var(--sync-history-card-offset); padding: 0 12px;
  }
  .sync-history-strip > button {
    flex: 1; min-width: 0; height: 100%; display: flex; align-items: flex-end;
    padding: 0; border: 0; border-radius: 2px; background: transparent; cursor: pointer;
  }
  .sync-history-strip > button > span {
    width: 100%; min-height: 2px; border-radius: 1px 1px 0 0; background: var(--text-3);
  }
  /* The loaded window is a band behind the whole column, not just a darker
     bar: the window this page is showing is very often the one with no
     commits in it, and a 2px bar cannot mark a place that holds nothing. */
  .sync-history-strip > button[aria-current="true"] { background: var(--bg-hover); }
  .sync-history-strip > button[aria-current="true"] > span { background: var(--accent); }
  .sync-history-strip > button:hover > span { background: var(--text-1); }
  .sync-history-strip-axis {
    display: flex; justify-content: space-between;
    margin: 0 0 10px var(--sync-history-card-offset); padding: 0 12px;
    color: var(--text-2); font-size: var(--type-2xs); font-variant-numeric: tabular-nums;
  }
`;

/* Local time, so the stamps read the way the app's do. */
const RANGE = "Jul 28 – Aug 27, 2026";
const NEWEST_OLDER = "Mar 12, 2026";
const DAYS_BACK = 168;

function Panel({ tag, tagKind, title, blurb, children, notes }: {
  tag: string;
  tagKind?: "now" | "pick";
  title: string;
  blurb: string;
  children?: ReactNode;
  notes?: ReactNode;
}) {
  return (
    <section className="mk-panel">
      <header>
        <span className={`mk-tag${tagKind ? ` ${tagKind}` : ""}`}>{tag}</span>
        <h2>{title}</h2>
        <p>{blurb}</p>
      </header>
      {children && (
        <div className="mk-stage">
          <div className="sync-history-content">{children}</div>
        </div>
      )}
      {notes && <ul className="mk-notes">{notes}</ul>}
    </section>
  );
}

/** A real commit card, shipped markup, so the variants can be judged in place. */
function CommitCard({ subject, sha, when, sessions = 0 }: {
  subject: string;
  sha: string;
  when: string;
  sessions?: number;
}) {
  return (
    <li className="sync-history-commit">
      <button type="button" className="sync-history-spine-fold" aria-label={`Collapse commit ${sha}`}>
        <Icon name="chevron-down" size={12} />
      </button>
      <div className="sync-history-commit-heading">
        <span className="sync-history-commit-title">
          <span className="sync-history-commit-icon" aria-hidden="true"><Icon name="git-commit" size={14} /></span>
          <strong>{subject}</strong>
        </span>
        <span className="sync-history-commit-meta">
          <code>{sha}</code>
          <time>{when}</time>
        </span>
      </div>
      {sessions > 0 && (
        <div className="sync-history-thread-list">
          {Array.from({ length: sessions }, (_, index) => (
            <div key={index} className="sync-history-thread-card">
              <span className="sync-history-thread-title">
                <Icon name="message" size={18} />
                <strong>Session {index + 1}</strong>
              </span>
            </div>
          ))}
        </div>
      )}
    </li>
  );
}

function RailEnd({ label, detail }: { label: string; detail: string }) {
  return (
    <button type="button" className="sync-history-rail-end">
      <span className="sync-history-rail-end-node" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
      <span className="sync-history-rail-end-text">
        <strong>{label}</strong>
        <span>{detail}</span>
      </span>
    </button>
  );
}

function Mockup() {
  /* `?theme=` still decides the first paint, so the headless captures keep
     working; the button takes over from there and writes its choice back to
     the URL, so a reload — or a capture of what you are looking at — lands on
     the same palette. */
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    const requested = new URLSearchParams(location.search).get("theme");
    if (requested === "light" || requested === "dark") return requested;
    return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  });
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    const url = new URL(location.href);
    url.searchParams.set("theme", theme);
    history.replaceState(null, "", url);
  }, [theme]);

  return (
    <div className="sync-main mk-page">
      <style>{SHELL_CSS}</style>
      <style>{PROPOSED_CSS}</style>

      <button
        type="button"
        className="mk-theme"
        aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} size={12} />
        {theme === "dark" ? "Light" : "Dark"}
      </button>

      <h1 className="mk-title">Activity · the empty window</h1>
      <p className="mk-lede">
        The project has commits and sessions; none of the commits fall in the last 30 days. Today the
        rail collapses to a dashed box that names only what is absent, and the one control on offer
        steps back a blind 30 days. Seven treatments below — the first three redesign the empty
        state, the last four question whether it should exist — then the two states this sentence is
        wrong about, then what has to change underneath for any of them to be honest.
      </p>

      <Panel
        tag="today"
        tagKind="now"
        title="What ships now"
        blurb="One sentence about a window the user never chose, and a button that cannot say whether the next click lands on anything."
        notes={<>
          <li>The window is only ever mentioned when it has failed. A page with commits never prints its range at all — <code>window_start</code> and <code>window_end</code> are in the payload and unused.</li>
          <li>The button is the page's last element, below the Older activity fold — detached from the rail it pages.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <section className="sync-history-commit-section">
            <div className="sync-history-state">No commits are available in this 30-day window.</div>
          </section>
        </div>
        <button type="button" className="btn sync-history-load-more">Load previous 30 days</button>
      </Panel>

      <Panel
        tag="1 · signpost"
        title="Say the fact, carry the verb"
        blurb="Smallest change that stops the dead end: the box names where the commits actually are, and the jump goes there in one click. Nothing structural moves."
        notes={<>
          <li>Only the box's contents and the button's label change. The button keeps its place at the page bottom.</li>
          <li>The date is free — the backend already holds every commit's timestamp before it filters by window.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <section className="sync-history-commit-section">
            <div className="sync-history-state sync-history-window-state">
              <Icon name="git-commit" size={24} />
              <div>
                <strong>No commits on main between Jul 28 and Aug 27.</strong>
                <span>The newest first-parent commit is {NEWEST_OLDER}, {DAYS_BACK} days back. 41 commits and 6 sessions sit older than this window.</span>
              </div>
            </div>
          </section>
        </div>
        <button type="button" className="btn sync-history-load-more">Load through {NEWEST_OLDER}</button>
      </Panel>

      <Panel
        tag="2 · rail terminus"
        tagKind="pick"
        title="The rail does not break where the data stops"
        blurb="The empty window becomes a card on the rail, and the paging control becomes the rail's bottom terminus — in the rail's own column, under the thing it extends."
        notes={<>
          <li>The spine survives an empty window, so the page keeps the structure every other state reads by.</li>
          <li>The terminus replaces the floating button. It states what one click buys before the click: the date it lands on, the distance, and the count waiting there.</li>
          <li>Same fold column, same 12px rail gaps, same anchor as every commit — no new geometry, only a new occupant.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <section className="sync-history-commit-section">
            <ol className="sync-history-commit-rail">
              <li className="sync-history-commit is-gap">
                <button type="button" className="sync-history-spine-fold" disabled aria-label="No sessions in this window">
                  <Icon name="chevron-down" size={12} />
                </button>
                <div className="sync-history-commit-heading">
                  <span className="sync-history-commit-title">
                    <span className="sync-history-commit-icon" aria-hidden="true"><Icon name="clock" size={14} /></span>
                    <strong>No commits on main in this window</strong>
                  </span>
                  <span className="sync-history-commit-meta">
                    <code>30 days</code>
                    <time>{RANGE}</time>
                  </span>
                </div>
              </li>
            </ol>
          </section>
          <RailEnd label={`Load through ${NEWEST_OLDER}`} detail={`${DAYS_BACK} days earlier · 41 commits, 6 sessions`} />
        </div>
      </Panel>

      <Panel
        tag="3 · window bar"
        title="Print the range on every page"
        blurb="A standing strip above the rail carries the loaded range and the paging control, so the window is visible whether or not it is empty. The box shrinks back to one plain line."
        notes={<>
          <li>The only treatment that tells a full page what it is showing — the other two speak up only when the window is empty.</li>
          <li>Costs a strip of chrome on every project, including the ordinary case where nothing was ever wrong.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <div className="sync-history-window-bar">
            <span><Icon name="clock" size={12} />{RANGE}<em>· 30 days</em></span>
            <button type="button" className="btn">Load through {NEWEST_OLDER}</button>
          </div>
          <section className="sync-history-commit-section">
            <div className="sync-history-state">No commits on main in this range. The newest is {NEWEST_OLDER}.</div>
          </section>
        </div>
      </Panel>

      <Panel
        tag="4 · auto-reach"
        tagKind="pick"
        title="Never render an empty window at all"
        blurb="If the requested window catches no commits, widen it on the server until it does, and return that page. The empty state stops existing; the user opened Activity to see history, not to negotiate with a window."
        notes={<>
          <li><strong>This costs nothing.</strong> <code>scan_local_session_views</code> takes no time bound and <code>load_first_parent_commits</code> is capped by count, not date — the 30 days is a filter applied after everything is already in memory. Widening changes how many rows render, not how much is read.</li>
          <li>One line above the rail says the window moved, so the dates below are never a surprise. Everything else on the page is exactly what it always was.</li>
          <li>Composes with 2 rather than competing: this handles "the page opened empty", the terminus handles "now take me further back".</li>
          <li>Needs a stated stop — widen to the newest commit, not indefinitely — or a project dormant for three years silently loads three years of rows.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <p className="sync-history-reach-note">
            <Icon name="clock" size={12} />
            Nothing on main in the last 30 days — reached back to {NEWEST_OLDER} to find the newest commit.
          </p>
          <section className="sync-history-commit-section">
            <ol className="sync-history-commit-rail">
              <CommitCard subject="Port the tail cursor to the codex reader" sha="c41f8ab" when="Mar 12, 2026" sessions={1} />
              <CommitCard subject="Freeze the projection corpus rules" sha="7d20e19" when="Mar 09, 2026" />
            </ol>
          </section>
          <RailEnd label="Load through Feb 02, 2026" detail="35 days earlier · 12 commits, 4 sessions" />
        </div>
      </Panel>

      <Panel
        tag="5 · range control"
        title="Make the window the user's choice"
        blurb="Stop hiding the bound and offer it: 30 days, 90 days, everything. The empty state then states a plain fact and points at the control that fixes it, instead of carrying a verb of its own."
        notes={<>
          <li>The honest framing — the window is a real constraint, so name it and hand it over rather than papering over it with a jump button.</li>
          <li><code>window_days</code> is clamped to <code>1..=90</code> today, so "All" is a backend change (an unbounded window, or a very large cap).</li>
          <li>One more persistent control on a page whose header was deliberately emptied into the path bar.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <div className="sync-history-window-bar">
            <span><Icon name="clock" size={12} />{RANGE}</span>
            <span className="sync-history-range">
              <button type="button" aria-pressed="true">30d</button>
              <button type="button" aria-pressed="false">90d</button>
              <button type="button" aria-pressed="false">All</button>
            </span>
          </div>
          <section className="sync-history-commit-section">
            <div className="sync-history-state sync-history-window-state">
              <Icon name="git-commit" size={24} />
              <div>
                <strong>No commits on main in the last 30 days.</strong>
                <span>41 commits and 6 sessions lie further back, the newest on {NEWEST_OLDER}. Widen the range to reach them.</span>
              </div>
            </div>
          </section>
        </div>
      </Panel>

      <Panel
        tag="6 · vanish"
        title="Delete the empty state instead of designing it"
        blurb="A rail with nothing on it is not worth 116px of dashed placeholder. Drop the commit section entirely and let the groups above be the page, with one quiet line carrying the fact and the verb."
        notes={<>
          <li>The least chrome of the seven, and the only one that removes markup rather than adding it.</li>
          <li>Risk: with the rail gone the page can look like it failed to load, especially when the groups above are empty too — which is exactly when the reassurance is needed most.</li>
        </>}
      >
        <div className="sync-history-storage-only">
          <button type="button" className="sync-history-spine-fold" aria-label="Collapse 2 sessions only in Local storage 1">
            <Icon name="chevron-down" size={12} />
          </button>
          <div className="sync-history-section-heading">
            <h2><Icon name="cloud" size={13} />Only in <span className="sync-history-storage-name">Local storage 1</span></h2>
            <span className="sync-history-heading-count"><Icon name="message" size={12} />2</span>
          </div>
          <div className="sync-history-thread-list flat">
            <div className="sync-history-thread-card"><span className="sync-history-thread-title"><Icon name="message" size={18} /><strong>Reconcile the codex thread allocator</strong></span></div>
            <div className="sync-history-thread-card"><span className="sync-history-thread-title"><Icon name="message" size={18} /><strong>Trace the severed-root projection</strong></span></div>
          </div>
        </div>
        <p className="sync-history-reach-note">
          <Icon name="git-commit" size={12} />
          No commits on main since {NEWEST_OLDER}, {DAYS_BACK} days ago.
          <button type="button" className="btn btn-ghost">Load through {NEWEST_OLDER}</button>
        </p>
      </Panel>

      <Panel
        tag="7 · activity strip"
        title="Show where the work actually is"
        blurb="One bucket per month, height by commit count, the loaded window inked and the rest muted. Paging stops being a guess and becomes navigation — click a month to go there."
        notes={<>
          <li>Answers a question none of the others can: not just "where is the newest commit" but "where is the work concentrated".</li>
          <li>Every count it needs is already in <code>all_commits</code>. The strip is a projection of data the response holds and throws away.</li>
          <li>By far the most to build and maintain, and it earns its keep only on projects with long histories.</li>
        </>}
      >
        <div className="sync-history-timeline">
          <div className="sync-history-strip">
            {[1, 4, 6, 3, 9, 14, 22, 17, 11, 8, 5, 2, 0, 0, 0, 1, 0, 0].map((count, index) => (
              <button
                key={index}
                type="button"
                aria-current={index === 17 ? "true" : "false"}
                aria-label={`${count} commit${count === 1 ? "" : "s"}`}
                title={`${count} commit${count === 1 ? "" : "s"}`}
              >
                <span style={{ height: `${Math.max(2, Math.round((count / 22) * 40))}px` }} />
              </button>
            ))}
          </div>
          <p className="sync-history-strip-axis"><span>Mar 2025</span><span>Mar 2026</span><span>Aug 2026</span></p>
          <section className="sync-history-commit-section">
            <div className="sync-history-state sync-history-window-state">
              <Icon name="git-commit" size={24} />
              <div>
                <strong>No commits on main between Jul 28 and Aug 27.</strong>
                <span>The strip above shows where they are — the last run ended {NEWEST_OLDER}.</span>
              </div>
            </div>
          </section>
        </div>
      </Panel>

      <Panel
        tag="2 · in the common case"
        title="The terminus on a page that has commits"
        blurb="Whatever wins for the empty state, the paging control's home is the same question. Here is treatment 2 where most projects live — and the floor it draws when there is genuinely nothing older."
        notes={<>
          <li>Left: more to load. Right: the end of the history, stated once, so the absence of a button is a fact rather than a missing control.</li>
        </>}
      >
        <div className="mk-split two">
          <div>
            <p className="mk-caption">more to load</p>
            <div className="sync-history-timeline">
              <section className="sync-history-commit-section">
                <ol className="sync-history-commit-rail">
                  <CommitCard subject="Trim low-value frontend assertions" sha="eb04e4f" when="Aug 26, 2026" sessions={1} />
                  <CommitCard subject="Consolidate Rust regression coverage" sha="37e9572" when="Aug 24, 2026" />
                </ol>
              </section>
              <RailEnd label={`Load through ${NEWEST_OLDER}`} detail={`${DAYS_BACK} days earlier · 41 commits, 6 sessions`} />
            </div>
          </div>
          <div>
            <p className="mk-caption">nothing older</p>
            <div className="sync-history-timeline">
              <section className="sync-history-commit-section">
                <ol className="sync-history-commit-rail">
                  <CommitCard subject="Keep test support out of production builds" sha="aa46e84" when="Aug 22, 2026" />
                  <CommitCard subject="Initial commit" sha="9031174" when="Aug 02, 2026" />
                </ol>
              </section>
              <p className="sync-history-rail-floor">Showing {RANGE} — the whole history of main.</p>
            </div>
          </div>
        </div>
      </Panel>

      <Panel
        tag="wrong today"
        title="Two states that get the same sentence"
        blurb="`commits.length === 0` is not one situation. On a branch with no local ref the page blames a window; the window has nothing to do with it, and no amount of paging will help."
        notes={<>
          <li>A branch recorded in session metadata but not checked out is offered in the picker and marked <code>History only</code> there — then the rail contradicts the chip.</li>
          <li>An unborn branch has a symbolic HEAD and no commits at all. Paging is meaningless; the page should not offer it.</li>
        </>}
      >
        <div className="mk-split two">
          <div>
            <p className="mk-caption">branch is history-only</p>
            <div className="sync-history-timeline">
              <section className="sync-history-commit-section">
                <div className="sync-history-state sync-history-window-state">
                  <Icon name="git-branch" size={24} />
                  <div>
                    <strong>feature/tail-cursors is not checked out here.</strong>
                    <span>Its recorded sessions still show. Its commits cannot — the branch has no local ref on this machine.</span>
                  </div>
                </div>
              </section>
            </div>
          </div>
          <div>
            <p className="mk-caption">no commits yet</p>
            <div className="sync-history-timeline">
              <section className="sync-history-commit-section">
                <div className="sync-history-state sync-history-window-state">
                  <Icon name="git-commit" size={24} />
                  <div>
                    <strong>main has no commits yet.</strong>
                    <span>Sessions in this project are listed above until the first commit claims them.</span>
                  </div>
                </div>
              </section>
            </div>
          </div>
        </div>
      </Panel>

      <Panel
        tag="underneath"
        title="What has to change in the backend"
        blurb="Two defects make any of the three treatments impossible to state honestly. Both are in build_project_activity, and neither costs a new git call."
        notes={<>
          <li><strong>The button is wrong about whether anything is older.</strong> <code>next_before</code> is <code>has_older_sessions.then_some(window_start)</code> — commits do not count. A project with older commits and no older sessions gets no button, and its commit history is unreachable from the UI at any window.</li>
          <li><strong>Every step is 30 blind days.</strong> Set <code>next_before</code> to just past the newest thing older than the window, and no click ever returns an empty page — each one lands on data and walks back through it.</li>
          <li>Both facts are already in memory: <code>all_commits</code> is loaded unwindowed (capped at 10,000 first-parent commits, and a truncation already raises its own warning), and <code>all_sessions</code> is the full list the window filters.</li>
        </>}
      />
    </div>
  );
}

/* Vite re-runs this module on every hot update, and a second createRoot on the
   same container is an error. The root is cached so an edit re-renders the
   specimen instead of stacking roots on it. */
declare global {
  interface Window { __mockupRoot?: Root }
}
const container = document.getElementById("root") as HTMLElement;
(window.__mockupRoot ??= createRoot(container)).render(
  <StrictMode><Mockup /></StrictMode>,
);
