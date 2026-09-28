import { StrictMode, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import { formatBytes } from "./format";
import "./App.css";

/**
 * Mockup: where the push limit meter lives.
 *
 * The shipped meter is a nowrap sentence plus a 4px/280px hairline in the
 * footer's sentence slot, far to the left of the button it governs. It is
 * legible only at the bottom of a review that is mostly scrolling, and the
 * over-limit case — the one that stops the push — reads as a thin red rule.
 *
 * Six frames on the same shell, switched in place so the comparison is
 * apples to apples:
 *   Before  the shipped footer line
 *   A       gauge pill fused to the Push button
 *   B       sticky cap strip at the top of the list + a chip beside Push
 *   C       framed gauge block docked left of Push, segments broken out
 *   D       the same compact gauge twice: header and footer, both pinned
 *   E       a full-height rail down the panel's right edge
 *   F       D and A together: header glance, gauge pill at the button
 *
 * Static data, shipped CSS, real leaf components. Every figure is derived:
 * the fit is the shipped policy (newest first, stop at the first row that
 * does not fit) and the meter sums the ticked rows. Not part of the app
 * bundle — reachable only from the dev server.
 */

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1180px; margin: 0 auto; padding: 24px 22px 80px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 92ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; align-items: center; margin: 0 0 18px; flex-wrap: wrap; }
  .mk-bar > span { color: var(--text-3); font-size: var(--type-2xs); margin-left: 10px; text-transform: uppercase; letter-spacing: .08em; }
  .mk-bar > span:first-child { margin-left: 0; }
  .mk-bar button { padding: 5px 12px; border: 1px solid var(--button-border); border-radius: 999px; background: var(--button-bg); color: var(--text-1); font: inherit; font-size: var(--type-sm); cursor: pointer; }
  .mk-bar button.on { background: var(--button-primary-bg); color: var(--button-primary-text); border-color: var(--button-primary-border); }
  .mk-head { margin: 10px 0 10px; }
  .mk-head h2 { margin: 0 0 3px; color: var(--text-0); font-size: var(--type-lg); }
  .mk-head p { margin: 0 0 4px; max-width: 96ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-head p b { color: var(--text-1); font-weight: var(--weight-semibold); }
  .mk-cost { color: var(--text-3); font-size: var(--type-xs); }
  .mk-frame { width: 100%; padding: 3px; border: 1px dashed var(--border-mid); border-radius: 6px; background: var(--retro-frame-gutter); }
  .mk-frame > section { border-radius: 4px; overflow: hidden; }
  /* Mock-only: the shipped review scrolls at min(54vh, 590px). Capped here so
     the whole shell — header, list, footer — fits one screen of this page and
     the top/bottom question can be answered by scrolling the list itself. */
  .mk-frame .sync-sync-review-scroll { max-height: 340px; }
  .mk-foot-note { margin: 8px 0 0; color: var(--text-3); font-size: var(--type-2xs); }

  /* ---- the shipped meter, reproduced for the Before frame -------------- */
  .mk-budget { gap: 5px; }
  .mk-budget > small { white-space: nowrap; }
  .mk-budget-bar { display: block; width: 100%; max-width: 280px; height: 4px; border-radius: 2px; background: var(--border-mid); overflow: hidden; }
  .mk-budget-bar > i { display: block; height: 100%; background: var(--thread-status-local); }
  .mk-budget.over > small { color: var(--danger-text); }
  .mk-budget.over .mk-budget-bar > i { background: var(--danger-text); }

  /* ---- shared track ---------------------------------------------------
     One bar scaled to max(total, limit): under the limit the track end IS
     the limit, over it the limit becomes a tick the fill visibly runs past,
     which is what turns "how much over" into a distance instead of a
     number. Three segments so the footer stops hiding the breakdown in a
     title attribute: sessions, the requirements every push carries, and
     project files. */
  .mk-track { position: relative; display: flex; width: 100%; height: 8px; border-radius: 999px; background: var(--border-mid); overflow: hidden; }
  .mk-track > i { display: block; height: 100%; }
  .mk-track-sessions { background: var(--thread-status-local); }
  .mk-track-reserved { background: var(--text-3); }
  .mk-track-files { background: color-mix(in srgb, var(--thread-status-local) 45%, var(--bg-2)); }
  .mk-track.over .mk-track-sessions { background: var(--danger-text); }
  .mk-track.over .mk-track-reserved { background: color-mix(in srgb, var(--danger-text) 55%, var(--bg-2)); }
  .mk-track.over .mk-track-files { background: repeating-linear-gradient(135deg, var(--danger-text) 0 3px, color-mix(in srgb, var(--danger-text) 45%, var(--bg-2)) 3px 6px); }
  .mk-track-tick { position: absolute; top: -2px; bottom: -2px; width: 2px; background: var(--text-0); border-radius: 1px; }
  .mk-figure { font-family: var(--font-mono); font-size: var(--type-xs); font-variant-numeric: tabular-nums; color: var(--text-1); white-space: nowrap; }
  .mk-figure b { color: var(--text-0); font-weight: var(--weight-semibold); }
  .over .mk-figure, .mk-figure.over { color: var(--danger-text); }
  .mk-figure.over b { color: var(--danger-text); }

  /* ---- A · gauge pill fused to the button ------------------------------
     The readout is the gauge: a pill the height of the button, filling left
     to right, set flush against Push so the quantity and the act that
     spends it are one object. Over, the pill takes the danger wash and the
     sentence prints on the row above it, right-aligned under the button. */
  .mk-gauge { position: relative; display: inline-flex; align-items: center; height: 34px; padding: 0 14px; border: var(--box-border); border-radius: 999px; background: var(--surface-card); overflow: hidden; }
  .mk-gauge-fill { position: absolute; inset: 0 auto 0 0; background: color-mix(in srgb, var(--thread-status-local) 34%, transparent); }
  .mk-gauge .mk-figure { position: relative; }
  .mk-gauge.over { border-color: var(--danger-text); background: linear-gradient(var(--danger-soft), var(--danger-soft)), var(--surface-card); }
  .mk-gauge.over .mk-gauge-fill { background: color-mix(in srgb, var(--danger-text) 26%, transparent); }
  /* The sentence takes a row of its own above the cluster: the footer wraps
     only in this direction, so nothing moves until the push is refused. */
  .mk-review[data-dir="gauge"] .sync-sync-review-footer,
  .mk-review[data-dir="both"] .sync-sync-review-footer { flex-wrap: wrap; padding-top: 6px; }
  .mk-gauge-sentence { flex: 1 0 100%; order: -1; margin: 0; text-align: right; color: var(--danger-text); font-size: var(--type-2xs); }

  /* ---- B · sticky cap strip -------------------------------------------
     Pinned to the top of the scrolling list, so the limit is the first
     thing read on entering the review and never leaves while excluding
     rows. The footer keeps a compact chip so the button still carries its
     own number. z-index 8 is load-bearing: the commit rail isolates at 5 and
     its spine folds sit at 6, so anything lower is painted over by the cards
     as soon as the list scrolls. */
  .mk-cap { position: sticky; top: 0; z-index: 8; display: flex; align-items: center; gap: 14px; margin: 0 0 8px; padding: 8px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-footer); box-shadow: 0 4px 10px -8px rgba(0,0,0,.55); }
  .mk-cap .mk-track { flex: 1 1 auto; min-width: 120px; }
  .mk-cap-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .mk-cap-text small { color: var(--text-2); font-size: var(--type-2xs); }
  .mk-cap.over { border-color: color-mix(in srgb, var(--red) 45%, var(--border)); background: linear-gradient(var(--danger-soft), var(--danger-soft)), var(--surface-footer); }
  .mk-cap.over .mk-cap-text small { color: var(--danger-text); }
  .mk-chip { display: inline-flex; align-items: center; gap: 7px; height: 30px; padding: 0 12px; border: 1px solid var(--border-strong); border-radius: 999px; background: var(--chip-bg); }
  .mk-chip.over { border-color: var(--danger-text); background: linear-gradient(var(--danger-soft), var(--danger-soft)), var(--surface-card); }

  /* ---- C · framed gauge block -----------------------------------------
     The meter becomes a piece of furniture in the button cluster: figure,
     8px segmented track, and the limit named at its right end, boxed in the
     app's own 2px grammar so it reads as one control beside Push. */
  .mk-block { display: flex; flex-direction: column; gap: 5px; min-width: 260px; max-width: 340px; padding: 7px 13px 8px; border: var(--box-border); border-radius: 14px; background: var(--surface-card); }
  .mk-block-line { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; }
  .mk-block-line em { color: var(--text-3); font-size: var(--type-2xs); font-style: normal; white-space: nowrap; }
  .mk-block.over { border-color: var(--danger-text); background: linear-gradient(var(--danger-soft), var(--danger-soft)), var(--surface-card); }
  .mk-block.over .mk-block-line em { color: var(--danger-text); }
  .mk-block-out { color: var(--danger-text); font-size: var(--type-2xs); }
  .mk-legend { display: flex; gap: 10px; color: var(--text-3); font-size: var(--type-3xs); }
  .mk-legend span { display: inline-flex; align-items: center; gap: 4px; white-space: nowrap; }
  .mk-legend i { width: 7px; height: 7px; border-radius: 2px; }

  /* ---- D · header + footer twin ---------------------------------------
     One compact gauge rendered twice. Both zones sit outside the scrolling
     list, so the limit is on screen at the top and at the bottom without a
     single sticky rule or a second design. */
  .mk-twin { display: inline-flex; align-items: center; gap: 9px; }
  .mk-twin .mk-track { width: 96px; height: 6px; }
  .mk-twin-foot .mk-track { width: 150px; height: 8px; }
  .mk-twin-head { padding-right: 12px; border-right: 1px solid var(--border); }
  .mk-twin-foot { flex-direction: column; align-items: flex-end; gap: 4px; }
  .mk-twin-foot .mk-twin-row { display: inline-flex; align-items: center; gap: 9px; }
  .mk-twin-foot small { color: var(--danger-text); font-size: var(--type-2xs); }

  /* ---- E · edge rail ---------------------------------------------------
     A gauge the height of the panel, filling from the footer upward toward
     the button it feeds. Spans top to bottom by construction. */
  .mk-review[data-dir="rail"] .sync-sync-review-content { position: relative; }
  .mk-review[data-dir="rail"] .sync-sync-review-scroll { padding-right: 34px; }
  .mk-rail { position: absolute; top: 10px; bottom: 8px; right: 12px; width: 10px; display: flex; flex-direction: column-reverse; border: 1px solid var(--border-strong); border-radius: 999px; background: var(--surface-subtle); overflow: hidden; }
  .mk-rail > i { display: block; width: 100%; }
  .mk-rail-tick { position: absolute; left: -4px; right: -4px; height: 2px; background: var(--text-0); }
  .mk-rail-cap { position: absolute; right: 26px; top: 4px; color: var(--text-3); font-size: var(--type-3xs); writing-mode: vertical-rl; }
  .mk-railfoot { display: inline-flex; align-items: center; gap: 8px; }
`;

/* ---------------------------------------------------------------- data --- */

type Provider = "claude" | "codex";

interface MockSession {
  id: string;
  title: string;
  provider: Provider;
  /** Capture bytes: transcript plus sidecars, what the push preflight sums. */
  bytes: number;
  /** Ordering key, newest first. */
  age: number;
  when: string;
  messages: number;
  tools: number;
  tokens: string;
  length: string;
  commit: string;
}

interface MockCommit { id: string; sha: string; subject: string; when: string }

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
/** Requirements the push always carries: skills and provider state. */
const RESERVED = Math.round(3.2 * MB);
const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude Code", codex: "Codex" };

const COMMITS: MockCommit[] = [
  { id: "c1", sha: "dd8d420", subject: "Finalize mainland storage research pagination", when: "Sep 1, 10:12" },
  { id: "c3", sha: "789eea5", subject: "Optimize Claude session metadata scans", when: "Aug 31, 23:58" },
  { id: "c4", sha: "ec82ce1", subject: "Optimize provider scans and activity logs", when: "Aug 30, 23:40" },
  { id: "c5", sha: "c002338", subject: "Add storage reset and refine sync reviews", when: "Aug 29, 20:14" },
  { id: "c6", sha: "4c1d9e2", subject: "Ship the Lamplight dark palette", when: "Aug 28, 23:50" },
  { id: "c7", sha: "9be0f31", subject: "Add catalog profile launch buttons", when: "Aug 27, 21:30" },
  { id: "c8", sha: "2f7a8c4", subject: "Make sidebar sections collapsible", when: "Aug 26, 19:02" },
  { id: "c9", sha: "51ac2b9", subject: "Files review cutover and loading scaffold", when: "Aug 23, 22:31" },
  { id: "c10", sha: "a0e77d3", subject: "One notice system", when: "Aug 21, 18:20" },
];

const SESSIONS: MockSession[] = [
  { id: "s1", title: "Refine sync review UX and bound session capture", provider: "claude", bytes: 96 * MB, age: 0, when: "Sep 1, 14:02", messages: 412, tools: 388, tokens: "3.1M", length: "6h 12m", commit: "c1" },
  { id: "s5", title: "Finalize mainland storage research pagination", provider: "claude", bytes: 58 * MB, age: 1, when: "Aug 31, 09:08", messages: 188, tools: 140, tokens: "1.7M", length: "2h 44m", commit: "c1" },
  { id: "s2", title: "Optimize Claude session metadata scans", provider: "claude", bytes: 84 * MB, age: 1, when: "Aug 31, 22:40", messages: 356, tools: 301, tokens: "2.6M", length: "4h 51m", commit: "c3" },
  { id: "s3", title: "Optimize provider scans and activity logs", provider: "claude", bytes: 71 * MB, age: 2, when: "Aug 30, 19:15", messages: 298, tools: 264, tokens: "2.2M", length: "3h 58m", commit: "c4" },
  { id: "s4", title: "Add storage reset and refine sync reviews", provider: "codex", bytes: 63 * MB, age: 3, when: "Aug 29, 16:33", messages: 240, tools: 211, tokens: "1.9M", length: "3h 20m", commit: "c5" },
  { id: "s6", title: "Dark mode Lamplight palette", provider: "claude", bytes: 49 * MB, age: 4, when: "Aug 28, 21:27", messages: 174, tools: 133, tokens: "1.5M", length: "2h 31m", commit: "c6" },
  { id: "s7", title: "Catalog profile launch buttons", provider: "codex", bytes: 118 * MB, age: 5, when: "Aug 27, 18:02", messages: 468, tools: 402, tokens: "3.8M", length: "7h 05m", commit: "c7" },
  { id: "s8", title: "Sidebar collapsible sections", provider: "claude", bytes: 96 * MB, age: 6, when: "Aug 26, 15:44", messages: 390, tools: 355, tokens: "3.0M", length: "5h 36m", commit: "c8" },
  { id: "s10", title: "Loading system scaffold", provider: "claude", bytes: 38 * MB, age: 9, when: "Aug 23, 20:11", messages: 141, tools: 96, tokens: "1.1M", length: "1h 58m", commit: "c9" },
  { id: "s11", title: "Files review cutover A1+B1", provider: "claude", bytes: 57 * MB, age: 10, when: "Aug 22, 17:49", messages: 205, tools: 170, tokens: "1.8M", length: "2h 57m", commit: "c9" },
  { id: "s12", title: "Notice system cutover", provider: "claude", bytes: 52 * MB, age: 11, when: "Aug 21, 13:05", messages: 196, tools: 152, tokens: "1.6M", length: "2h 40m", commit: "c10" },
];

/* -------------------------------------------------------------- policy --- */

/** The shipped fit: newest first, stopping at the first row that does not
 *  fit, so a push is a block of time and no older row goes before a newer
 *  one. Rows past the stop are visible and inert with a "Later" chip. */
function fit(rows: readonly MockSession[], budget: number): Set<string> {
  const admitted = new Set<string>();
  let spent = 0;
  for (const row of [...rows].sort((a, b) => a.age - b.age)) {
    if (spent + row.bytes > budget) break;
    spent += row.bytes;
    admitted.add(row.id);
  }
  return admitted;
}

/** Everything the meter needs, derived from the ticked rows. `scale` is
 *  max(total, limit): under the limit the track ends at the limit, over it
 *  the limit is a tick the fill runs past. */
function measure(sessions: number, files: number) {
  const total = sessions + RESERVED + files;
  const scale = Math.max(total, LIMIT);
  const over = Math.max(0, total - LIMIT);
  const pct = (value: number) => `${(value / scale) * 100}%`;
  return {
    sessions, files, reserved: RESERVED, total, over, scale,
    limit: LIMIT,
    pct,
    limitPct: `${(LIMIT / scale) * 100}%`,
    sentence: over > 0
      ? `${formatBytes(over)} over the ${formatBytes(LIMIT)} push limit — exclude sessions or files to push.`
      : `This push · ${formatBytes(total)} of ${formatBytes(LIMIT)}`,
    breakdown: `Sessions ${formatBytes(sessions)} · Requirements ${formatBytes(RESERVED)} · Project files ${formatBytes(files)}`,
  };
}

type Load = ReturnType<typeof measure>;

/* ------------------------------------------------------------- fittings --- */

/** The segmented track every new direction shares. */
function Track({ load, className }: { load: Load; className?: string }) {
  const over = load.over > 0;
  return (
    <span
      className={`mk-track${over ? " over" : ""}${className ? ` ${className}` : ""}`}
      role="meter"
      aria-valuemin={0}
      aria-valuemax={load.limit}
      aria-valuenow={Math.min(load.total, load.limit)}
      aria-valuetext={load.sentence}
      title={load.breakdown}
    >
      <i className="mk-track-sessions" style={{ width: load.pct(load.sessions) }} />
      <i className="mk-track-reserved" style={{ width: load.pct(load.reserved) }} />
      <i className="mk-track-files" style={{ width: load.pct(load.files) }} />
      {over && <b className="mk-track-tick" style={{ left: load.limitPct }} aria-hidden="true" />}
    </span>
  );
}

/** `424 MB / 512 MB` under the limit, `+2.1 GB over 512 MB` past it. */
function Figure({ load, short = false }: { load: Load; short?: boolean }) {
  const over = load.over > 0;
  return (
    <span className={`mk-figure${over ? " over" : ""}`}>
      {over
        ? <><b>+{formatBytes(load.over)}</b>{short ? "" : ` over ${formatBytes(load.limit)}`}</>
        : <><b>{formatBytes(load.total)}</b> / {formatBytes(load.limit)}</>}
    </span>
  );
}

/** The shipped figure, class for class. */
function ShippedFigure({ load }: { load: Load }) {
  const over = load.over > 0;
  return (
    <span className={`sync-review-budget-figure${over ? " over" : ""}`} role="meter" aria-valuemin={0} aria-valuemax={load.limit} aria-valuenow={Math.min(load.total, load.limit)} aria-valuetext={load.sentence}>
      {over
        ? <><b>+{formatBytes(load.over)}</b></>
        : <><b>{formatBytes(load.total)}</b> / {formatBytes(load.limit)}</>}
    </span>
  );
}

function Legend() {
  return (
    <span className="mk-legend" aria-hidden="true">
      <span><i style={{ background: "var(--thread-status-local)" }} />Sessions</span>
      <span><i style={{ background: "var(--text-3)" }} />Requirements</span>
      <span><i style={{ background: "color-mix(in srgb, var(--thread-status-local) 45%, var(--bg-2))" }} />Files</span>
    </span>
  );
}

/* ---------------------------------------------------------------- rows --- */

function Row({ session, admitted, selected, onToggle }: {
  session: MockSession;
  admitted: boolean;
  selected: boolean;
  onToggle: () => void;
}) {
  const tip = admitted
    ? (selected ? "Update storage. Only on this computer. Push to save it in Attic R2." : "Not selected. Only on this computer.")
    : `Doesn't fit in this push — the ${formatBytes(LIMIT)} limit is full with newer sessions. Push again after this one.`;
  return (
    <article className={`sync-history-thread-card${admitted ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {admitted && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${session.title}`}>
              <input type="checkbox" checked={selected} aria-label={`${selected ? "Exclude" : "Include"} ${session.title}`} onChange={onToggle} />
            </label>
          )}
          <span className="sync-thread-sync-indicator local" data-tooltip={tip} aria-label={tip} tabIndex={0}>
            <ThreadStatusIcon status="local-outstanding" size={18} />
          </span>
          <strong>{session.title}</strong>
          {!admitted && <span className="sync-history-branch-state history-only" title={tip}>Later</span>}
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
            <span className="sync-history-thread-metric" data-tooltip={`Push size · ${formatBytes(session.bytes)} (transcript and sidecars)`} tabIndex={0}><Icon name="drive" size={12} /><b>{formatBytes(session.bytes)}</b></span>
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

/* -------------------------------------------------------------- review --- */

type Direction = "before" | "gauge" | "cap" | "block" | "twin" | "rail" | "both";

function MockReview({ direction, files }: { direction: Direction; files: number }) {
  const budget = LIMIT - RESERVED;
  const admitted = useMemo(() => fit(SESSIONS, budget), [budget]);
  const [dropped, setDropped] = useState<ReadonlySet<string>>(new Set());
  const isSelected = (id: string) => admitted.has(id) && !dropped.has(id);
  const toggle = (id: string) => setDropped((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  const admittedRows = SESSIONS.filter((row) => admitted.has(row.id));
  const selectedRows = admittedRows.filter((row) => isSelected(row.id));
  const load = measure(selectedRows.reduce((sum, row) => sum + row.bytes, 0), files);
  const over = load.over > 0;
  const disabledReason = over
    ? load.sentence
    : selectedRows.length === 0 && files === 0 ? "Nothing is selected" : null;

  return (
    <section
      className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace mk-review"
      data-dir={direction}
      aria-label={`Push review mock — ${direction}`}
    >
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>Push review</h2></div>
        {direction === "twin" && (
          <div className="mk-twin mk-twin-head" role="status" title={load.breakdown}>
            <Track load={load} />
            <Figure load={load} short />
          </div>
        )}
        {/* F is shipped: this frame renders the app's own classes, so the
            specimen keeps measuring the real thing. */}
        {direction === "both" && (
          <div className="sync-review-budget-glance" title={load.breakdown}>
            <span className={`sync-review-budget-track push${over ? " over" : ""}`} aria-hidden="true">
              <i style={{ width: load.pct(load.total) }} />
              {over && <b style={{ left: load.limitPct }} />}
            </span>
            <ShippedFigure load={load} />
          </div>
        )}
        <SyncReviewSelectionBar
          count={selectedRows.length}
          total={admittedRows.length}
          recommendedTitle={`Use the recommended session selection (${admittedRows.length})`}
          recommendedAriaLabel={`Use recommended selection (${admittedRows.length})`}
          recommendedDisabled={dropped.size === 0}
          onRecommended={() => setDropped(new Set())}
          clearDisabled={selectedRows.length === 0}
          onClear={() => setDropped(new Set(admittedRows.map((row) => row.id)))}
          close={{ ariaLabel: "Close push review", onClose: () => undefined }}
        />
      </header>

      <SyncReviewTabs
        tabs={[
          { id: "history", label: "Git history", icon: "git-commit", count: selectedRows.length },
          { id: "project_files", label: "Project files", icon: "file", count: files > 0 ? 4 : 0 },
        ]}
        activeId="history"
        onChange={() => undefined}
      />

      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {direction === "cap" && (
            <div className={`mk-cap${over ? " over" : ""}`} role="status">
              <span className="mk-cap-text">
                <Figure load={load} />
                <small>{over ? load.sentence : load.breakdown}</small>
              </span>
              <Track load={load} />
            </div>
          )}
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section aria-label="Git history">
              <div className="sync-history-timeline">
                <section className="sync-history-commit-section" aria-label="First-parent commits on main">
                  <ol className="sync-history-commit-rail">
                    {COMMITS.map((commit) => {
                      const rows = SESSIONS.filter((row) => row.commit === commit.id).sort((a, b) => a.age - b.age);
                      return (
                        <li key={commit.id} className="sync-history-commit">
                          <button type="button" className="sync-history-spine-fold" aria-label={`Collapse commit ${commit.sha}`} aria-expanded>
                            <Icon name="chevron-down" size={12} />
                          </button>
                          <div className="sync-history-commit-heading">
                            <span className="sync-history-commit-title">
                              <span className="sync-history-commit-icon" aria-hidden="true"><Icon name="git-commit" size={14} /></span>
                              <strong title={commit.subject}>{commit.subject}</strong>
                            </span>
                            <span className="sync-history-commit-meta">
                              <code>{commit.sha}</code>
                              <time>{commit.when}</time>
                            </span>
                          </div>
                          <div className="sync-history-thread-list">
                            {rows.map((row) => (
                              <Row
                                key={row.id}
                                session={row}
                                admitted={admitted.has(row.id)}
                                selected={isSelected(row.id)}
                                onToggle={() => toggle(row.id)}
                              />
                            ))}
                          </div>
                        </li>
                      );
                    })}
                  </ol>
                </section>
              </div>
            </section>
          </div>
        </div>
        {direction === "rail" && (
          <div className="mk-rail" role="meter" aria-valuemin={0} aria-valuemax={load.limit} aria-valuenow={Math.min(load.total, load.limit)} aria-valuetext={load.sentence} title={load.breakdown}>
            <i className="mk-track-sessions" style={{ height: load.pct(load.sessions), background: over ? "var(--danger-text)" : undefined }} />
            <i className="mk-track-reserved" style={{ height: load.pct(load.reserved) }} />
            <i className="mk-track-files" style={{ height: load.pct(load.files), background: over ? "repeating-linear-gradient(135deg, var(--danger-text) 0 3px, color-mix(in srgb, var(--danger-text) 45%, var(--bg-2)) 3px 6px)" : undefined }} />
            {over && <b className="mk-rail-tick" style={{ bottom: load.limitPct }} aria-hidden="true" />}
          </div>
        )}
        {direction === "rail" && <span className="mk-rail-cap" aria-hidden="true">{formatBytes(load.limit)} limit</span>}
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {direction === "both" && over && (
          <p className="sync-review-budget-sentence">{load.sentence}</p>
        )}
        {direction === "before" && (
          <span className={`sync-push-budget mk-budget${over ? " over" : ""}`} role="status" title={load.breakdown}>
            <small>{load.sentence}</small>
            <span className="mk-budget-bar" role="meter" aria-valuemin={0} aria-valuemax={load.limit} aria-valuenow={Math.min(load.total, load.limit)} aria-valuetext={load.sentence}>
              <i style={{ width: `${Math.min(100, (load.total / load.limit) * 100)}%` }} />
            </span>
          </span>
        )}

        <button type="button" className="btn btn-ghost" title="Review project files next">
          Project files<Icon name="chevron-right" size={14} />
        </button>

        {direction === "both" && (
          <div className={`sync-review-budget-gauge push${over ? " over" : ""}`} role="status" title={load.breakdown}>
            <span className="sync-review-budget-fill" style={{ width: `${Math.min(100, (load.total / load.limit) * 100)}%` }} aria-hidden="true" />
            <ShippedFigure load={load} />
          </div>
        )}
        {direction === "gauge" && (
          <>
            {over && <p className="mk-gauge-sentence">{load.sentence}</p>}
            <div className={`mk-gauge${over ? " over" : ""}`} role="status" title={load.breakdown}>
              <span className="mk-gauge-fill" style={{ width: load.pct(load.total) }} aria-hidden="true" />
              {/* The sentence above already names the limit; the pill prints
                  only the overshoot so it never outgrows the button. */}
              <Figure load={load} short={over} />
            </div>
          </>
        )}
        {direction === "cap" && (
          <div className={`mk-chip${over ? " over" : ""}`} role="status" title={load.sentence}>
            <Figure load={load} short />
          </div>
        )}
        {direction === "block" && (
          <div className={`mk-block${over ? " over" : ""}`} role="status" title={load.breakdown}>
            <span className="mk-block-line">
              <Figure load={load} />
              <em>{over ? "over the limit" : `${formatBytes(load.limit)} limit`}</em>
            </span>
            <Track load={load} />
            {/* The legend explains the segments while they still matter; once
                the push is refused the block owes the reader the way out
                instead. */}
            {over ? <small className="mk-block-out">Exclude sessions or files to push.</small> : <Legend />}
          </div>
        )}
        {direction === "twin" && (
          <div className={`mk-twin mk-twin-foot${over ? " over" : ""}`} role="status" title={load.breakdown}>
            {over && <small>{load.sentence}</small>}
            <span className="mk-twin-row">
              <Track load={load} />
              <Figure load={load} />
            </span>
          </div>
        )}
        {direction === "rail" && (
          <div className={`mk-railfoot${over ? " over" : ""}`} role="status" title={load.breakdown}>
            <Figure load={load} />
          </div>
        )}

        <button
          type="button"
          className="btn btn-primary sync-pull-apply-button"
          disabled={disabledReason != null}
          title={disabledReason ?? undefined}
        >
          <ThreadStatusIcon status="local-outstanding" size={16} />
          Push
        </button>
      </footer>
    </section>
  );
}

/* ---------------------------------------------------------------- page --- */

const NOTES: Record<Direction, { name: string; what: string; cost: string }> = {
  before: {
    name: "Before — the shipped footer line",
    what: "A nowrap sentence over a 4px, 280px-wide hairline in the footer's sentence slot, left of everything else. It is the whole signal, and it sits about 700px away from the button it disables.",
    cost: "Visible only at the foot of the review; over the limit the difference between fine and blocked is a thin red rule and a colour change on 10px text.",
  },
  gauge: {
    name: "A · Gauge pill — fused to the button",
    what: "The readout becomes the gauge: a pill the height of Push, filled left to right, set flush against it so the quantity and the act that spends it are one object. Over the limit the pill takes the danger wash, a tick marks where the limit fell, and the sentence prints on the line above, right-aligned under the button.",
    cost: "Bottom of the review only — nothing at the top. The pill competes with Push for attention in the one corner that was quiet.",
  },
  cap: {
    name: "B · Sticky cap — pinned to the top of the list",
    what: "A strip that sticks to the top of the scrolling list: figure, breakdown, and a full-width track. It is the first thing read on entering the review and never leaves while rows are being excluded. The footer keeps a compact chip so the button still carries its own number.",
    cost: "Two places to keep in sync; the strip eats ~46px of list height on every review, including the many nowhere near the limit; and it has to out-rank the commit rail's stacking context (z-index 5) or the cards paint straight over it.",
  },
  block: {
    name: "C · Gauge block — furniture beside Push",
    what: "The meter becomes a boxed control in the button cluster, in the app's own 2px grammar: figure, limit name, an 8px segmented track, and a legend. The breakdown that today hides in a title attribute is on the surface — sessions, requirements, project files.",
    cost: "Bottom only, and it is the heaviest option: ~300px of footer, which crowds the impact counts and the step button on a narrow panel.",
  },
  twin: {
    name: "D · Twin — header and footer, same gauge",
    what: "One compact gauge rendered in two pinned zones: the review header, beside the counts, and the footer, beside Push. Both sit outside the scrolling list, so the limit is on screen at the top and at the bottom with no sticky rule, no new layout, and no second design to maintain.",
    cost: "Neither instance is large; the header one is a glance, not a readout. The sentence still only fits in the footer.",
  },
  both: {
    name: "F · D + A — glance at the top, gauge at the button — SHIPPED",
    what: "Landed in the app 2026-09-04; this frame renders the shipped classes, not mock copies. The shipped track carries one fill, not a segment per source: --text-3 measured 1.05:1 from --thread-status-local on cream, so two of the three segments were the same colour. The combination, and the one that answers the brief with no new mechanism: D's compact gauge in the header for the top of the review, A's filled pill against Push for the bottom. Both zones are already pinned, so the limit is on screen wherever the list is scrolled, and the readout nearest the button is the one that looks like a fuel gauge.",
    cost: "Two instances of the same number on screen at once — deliberate here, but it is the thing to check in the real app before shipping.",
  },
  rail: {
    name: "E · Edge rail — the panel's full height",
    what: "A gauge running the height of the list down the right edge, filling from the footer upward toward the button it feeds, with the limit as a tick and the figure at its foot beside Push. Literally visible from the top and the bottom.",
    cost: "At 10px on the right edge it can be mistaken for a scrollbar, and a vertical bar is a slower read than a horizontal one. Costs list width on every review.",
  },
};

const ORDER: Direction[] = ["before", "gauge", "cap", "block", "twin", "rail", "both"];
const LABEL: Record<Direction, string> = {
  before: "Before", gauge: "A · Gauge pill", cap: "B · Sticky cap", block: "C · Gauge block", twin: "D · Twin", rail: "E · Edge rail", both: "F · D + A",
};

function Page() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [direction, setDirection] = useState<Direction>("both");
  const [files, setFiles] = useState(0);
  document.documentElement.dataset.theme = theme;
  const note = NOTES[direction];
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Push review — where the limit lives</h1>
      <p className="mk-lede">
        One shell, seven treatments of the same meter, switched in place. Eleven outstanding sessions,
        one 512 MB push; the six newest fit and the rest carry a Later chip. Tick rows to move the
        meter, or add project files to cross the limit — the figures are derived, not typed, so the
        over-limit state is the real one the shipped Push button refuses on.
      </p>
      <div className="mk-bar" role="group" aria-label="Mock controls">
        <span>Direction</span>
        {ORDER.map((id) => (
          <button key={id} type="button" className={direction === id ? "on" : undefined} onClick={() => setDirection(id)}>
            {LABEL[id]}
          </button>
        ))}
      </div>
      <div className="mk-bar" role="group" aria-label="State controls">
        <span>Project files</span>
        <button type="button" className={files === 0 ? "on" : undefined} onClick={() => setFiles(0)}>None · fits</button>
        <button type="button" className={files === 80 * MB ? "on" : undefined} onClick={() => setFiles(80 * MB)}>80 MB · nearly full</button>
        <button type="button" className={files === 2200 * MB ? "on" : undefined} onClick={() => setFiles(2200 * MB)}>2.2 GB · over</button>
        <span>Theme</span>
        <button type="button" className={theme === "light" ? "on" : undefined} onClick={() => setTheme("light")}>Light</button>
        <button type="button" className={theme === "dark" ? "on" : undefined} onClick={() => setTheme("dark")}>Dark</button>
      </div>

      <div className="mk-head">
        <h2>{note.name}</h2>
        <p>{note.what}</p>
        <p className="mk-cost"><b>Costs:</b> {note.cost}</p>
      </div>
      <div className="mk-frame"><MockReview direction={direction} files={files} /></div>
      <p className="mk-foot-note">
        Scroll inside the frame to test what stays visible. Requirements ({formatBytes(RESERVED)}) ride
        every push, which is why the meter can pass the limit while every listed session still fits.
      </p>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
