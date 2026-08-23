import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ProjectSessionDetail, SessionCount } from "./types";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SessionDetailHeader from "./components/project-sync/session-detail/SessionDetailHeader";
import {
  formatSessionDuration,
} from "./components/project-sync/session-detail/sessionDetailModel";
import "./App.css";
import "./RetroTheme.css";

/**
 * Four candidate shapes for the session-detail header's "when" chip, against
 * the shipped skin. C was chosen and now ships; the baseline column keeps a
 * copy of the retired `formatSessionRange` so the comparison that decided it
 * still renders.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const NOW = new Date(2026, 7, 20, 15, 20, 0); // Thursday 20 Aug 2026, 15:20

function at(year: number, month: number, day: number, hour: number, minute: number): number {
  return Math.floor(new Date(year, month - 1, day, hour, minute).getTime() / 1000);
}

/* ---- shared pieces ----------------------------------------------------- */

function clock(seconds: number): string {
  return new Date(seconds * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit", minute: "2-digit", hour12: false,
  });
}

function monthDay(seconds: number): string {
  return new Date(seconds * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** The year is a fact, not decoration: it appears exactly when it isn't this
 *  one. Today's chip strips it unconditionally, so a 2024 session and a 2026
 *  session print the same string. */
function calendarDay(seconds: number): string {
  const when = new Date(seconds * 1000);
  return when.getFullYear() === NOW.getFullYear()
    ? monthDay(seconds)
    : `${monthDay(seconds)} ${when.getFullYear()}`;
}

function dayKey(seconds: number): string {
  const when = new Date(seconds * 1000);
  return `${when.getFullYear()}-${when.getMonth()}-${when.getDate()}`;
}

function sameDay(a: number, b: number): boolean {
  return dayKey(a) === dayKey(b);
}

/** Whole calendar days back — not elapsed hours, so 23:50 last night is one
 *  day old at 00:10, the way a reader counts. */
function daysBack(seconds: number): number {
  const when = new Date(seconds * 1000);
  const a = new Date(when.getFullYear(), when.getMonth(), when.getDate()).getTime();
  const b = new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate()).getTime();
  return Math.round((b - a) / 86_400_000);
}

/** Optional across every candidate: name the two days a reader never wants
 *  spelled as a date. Off by default so each option can be judged on its own. */
function namedDay(seconds: number): string {
  const back = daysBack(seconds);
  if (back === 0) return "Today";
  if (back === 1) return "Yesterday";
  return calendarDay(seconds);
}

interface Sample {
  note: string;
  started: number | null;
  ended: number | null;
  running?: boolean;
}

/** One chip's worth of content: the parts a candidate wants rendered, so the
 *  bold duration stays bold instead of being flattened into a string. */
interface ChipPlan {
  chips: { text: string; strong?: string; icon?: "clock" | "activity" }[];
}

type Candidate = {
  id: string;
  name: string;
  blurb: string;
  plan: (sample: Sample, named: boolean) => ChipPlan;
};

const day = (sample: Sample, named: boolean) =>
  named ? namedDay(sample.started!) : calendarDay(sample.started!);

/** `formatSessionRange` as it stood before this change, verbatim — including
 *  the unconditional year strip that made a 2024 session unreadable. */
function retiredRange(started: number | null, ended: number | null): string | null {
  if (started == null) return null;
  const dayOf = (seconds: number) => new Date(seconds * 1000)
    .toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })
    .replace(/,\s*\d{4}$/, "");
  if (ended == null) return `${dayOf(started)} · ${clock(started)}`;
  if (sameDay(started, ended)) return `${dayOf(started)} · ${clock(started)} → ${clock(ended)}`;
  return `${dayOf(started)} ${clock(started)} → ${dayOf(ended)} ${clock(ended)}`;
}

const CANDIDATES: Candidate[] = [
  {
    id: "now",
    name: "Ships today",
    blurb: "range plus duration — start, end and length, three numbers for two facts",
    plan: (sample) => {
      const range = retiredRange(sample.started, sample.ended);
      if (!range) return { chips: [{ text: "Time unavailable" }] };
      const duration = formatSessionDuration(sample.started, sample.ended);
      return { chips: [{ text: range, strong: duration ?? undefined }] };
    },
  },
  {
    id: "a",
    name: "A · Start and length",
    blurb: "the end is arithmetic; drop it and the arrow with it",
    plan: (sample, named) => {
      if (sample.started == null) return { chips: [{ text: "Time unavailable" }] };
      const duration = formatSessionDuration(sample.started, sample.ended);
      return {
        chips: [{
          text: `${day(sample, named)}, ${clock(sample.started)}${sample.running || duration ? " ·" : ""}`,
          strong: sample.running ? "running" : duration ?? undefined,
        }],
      };
    },
  },
  {
    id: "b",
    name: "B · Interval only",
    blurb: "one continuous fact, one shape; the length is in the numbers",
    plan: (sample, named) => {
      if (sample.started == null) return { chips: [{ text: "Time unavailable" }] };
      if (sample.ended == null || sample.running) {
        return { chips: [{ text: `${day(sample, named)} · ${clock(sample.started)} –`, strong: "now" }] };
      }
      const tail = sameDay(sample.started, sample.ended)
        ? clock(sample.ended)
        : `${calendarDay(sample.ended)} ${clock(sample.ended)}`;
      return { chips: [{ text: `${day(sample, named)} · ${clock(sample.started)}–${tail}` }] };
    },
  },
  {
    id: "c",
    name: "C · Two chips",
    blurb: "one fact per box, like every other chip on the strip",
    plan: (sample, named) => {
      if (sample.started == null) return { chips: [{ text: "Time unavailable" }] };
      const duration = formatSessionDuration(sample.started, sample.ended);
      const chips: ChipPlan["chips"] = [
        { text: `${day(sample, named)}, ${clock(sample.started)}` },
      ];
      if (sample.running) chips.push({ text: "", strong: "running", icon: "activity" });
      else if (duration) chips.push({ text: "", strong: duration, icon: "clock" });
      return { chips };
    },
  },
  {
    id: "d",
    name: "D · Said in words",
    blurb: "no symbols; “ran” names what the second number is",
    plan: (sample, named) => {
      if (sample.started == null) return { chips: [{ text: "Time unavailable" }] };
      const duration = formatSessionDuration(sample.started, sample.ended);
      if (sample.running) return { chips: [{ text: `Started ${day(sample, named).toLowerCase() === "today" ? "today" : day(sample, named)} ${clock(sample.started)} ·`, strong: "still running" }] };
      return {
        chips: [{
          text: duration
            ? `${day(sample, named)}, ${clock(sample.started)} · ran`
            : `${day(sample, named)}, ${clock(sample.started)}`,
          strong: duration ?? undefined,
        }],
      };
    },
  },
];

/* ---- the session shapes that have to survive --------------------------- */

const SAMPLES: Sample[] = [
  { note: "the screenshot — two days back", started: at(2026, 8, 18, 21, 57), ended: at(2026, 8, 18, 22, 1) },
  { note: "this morning, 42m", started: at(2026, 8, 20, 9, 4), ended: at(2026, 8, 20, 9, 46) },
  { note: "yesterday, 1h 38m", started: at(2026, 8, 19, 20, 22), ended: at(2026, 8, 19, 22, 0) },
  { note: "crosses midnight", started: at(2026, 8, 15, 23, 41), ended: at(2026, 8, 16, 0, 56) },
  { note: "three months back, 2h 07m", started: at(2026, 5, 19, 9, 12), ended: at(2026, 5, 19, 11, 19) },
  { note: "another year — 2024", started: at(2024, 11, 3, 14, 2), ended: at(2024, 11, 3, 14, 48) },
  { note: "still running", started: at(2026, 8, 20, 15, 6), ended: at(2026, 8, 20, 15, 20), running: true },
  { note: "no timestamps recorded", started: null, ended: null },
];

/* ---- the strip, as it ships -------------------------------------------- */

function Chip({ children, title }: { children: React.ReactNode; title?: string }) {
  return <span className="session-detail-chip" title={title}>{children}</span>;
}

function WhenChips({ plan }: { plan: ChipPlan }) {
  return (
    <>
      {plan.chips.map((chip, index) => (
        <Chip key={index}>
          {chip.icon && <Icon name={chip.icon} size={12} />}
          {chip.text && <time>{chip.text}</time>}
          {chip.strong && <b>{chip.strong}</b>}
        </Chip>
      ))}
    </>
  );
}

/** The real header strip: branch, sha, the candidate's when, tokens, verbs. */
function HeaderStrip({ plan }: { plan: ChipPlan }) {
  return (
    <header className="session-detail-header">
      <div className="session-detail-strip">
        <div className="session-detail-chips" aria-label="Session facts">
          <Chip title="Branch main"><Icon name="git-branch" size={12} /><code>main</code></Chip>
          <Chip title="86a03c1"><Icon name="git-commit" size={12} /><code>86a03c1</code></Chip>
          <WhenChips plan={plan} />
          <Chip><code className="quality-exact">376</code> tokens</Chip>
        </div>
        <div className="session-detail-header-actions" aria-label="Session actions">
          <button type="button" className="btn btn-ghost session-detail-launch" title="Open in Claude Code">
            <ProviderMark provider="claude" displayName="Claude Code" size={12} />
          </button>
          <button type="button" className="btn btn-ghost session-detail-launch" title="Open in terminal">
            <Icon name="terminal" size={12} />
          </button>
        </div>
      </div>
    </header>
  );
}

/* ---- what actually ships ----------------------------------------------- */

/* The chosen candidate, rendered by the component itself rather than by a
   hand-built copy of it — the only rendering here that can be wrong in the
   same way the app is. */

const count = (value: number): SessionCount => ({ value, quality: "exact" });

const DETAIL: ProjectSessionDetail = {
  resource_id: "claude:conversation:session-1",
  provider: "claude",
  provider_display_name: "Claude Code",
  provider_session_id: "session-1",
  title: "Add horror story document",
  summary: null,
  started_at: null,
  ended_at: null,
  is_active: false,
  branch: "main",
  recorded_sha: "86a03c1d4f2b9e7a1c3d5f8b2e4a6c9d1f3b5e7a",
  tokens: count(376),
  event_counts: {
    prompts: count(2), responses: count(3), intermediate_steps: count(1),
    checkpoints: count(0), tool_calls: count(4), file_edits: count(1),
    shell_calls: count(2), reads: count(3), other_tools: count(1),
  },
  capabilities: { intermediate_steps: true, checkpoints: true, tool_calls: true, event_detail: true },
  actions: [
    { action_id: "claude.open_in_app", kind: "open_in_app", label: "Open in Claude Code", enabled: true },
    { action_id: "claude.open_in_terminal", kind: "open_in_terminal", label: "Open in terminal", enabled: true },
  ],
  snapshot_id: "snapshot-1",
  warnings: [],
};

function Shipped({ sample }: { sample: Sample }) {
  return (
    <SessionDetailHeader
      detail={{
        ...DETAIL,
        started_at: sample.started,
        ended_at: sample.ended,
        is_active: sample.running ?? false,
      }}
      headingRef={{ current: null }}
      actionBusyId={null}
      onAction={() => undefined}
    />
  );
}

/* ---- the page ---------------------------------------------------------- */

function Ladder({ named }: { named: boolean }) {
  return (
    <table className="specimen-table">
      <thead>
        <tr>
          <th>the session</th>
          {CANDIDATES.map((candidate) => <th key={candidate.id}>{candidate.name}</th>)}
        </tr>
      </thead>
      <tbody>
        {SAMPLES.map((sample) => (
          <tr key={sample.note}>
            <td className="specimen-note">{sample.note}</td>
            {CANDIDATES.map((candidate) => {
              const plan = candidate.plan(sample, named && candidate.id !== "now");
              return (
                <td key={candidate.id} className={candidate.id === "now" ? "specimen-cell was" : "specimen-cell"}>
                  {plan.chips.map((chip, index) => (
                    <span className="specimen-chip" key={index}>
                      {chip.icon ? "◷ " : ""}{chip.text}{chip.text && chip.strong ? " " : ""}
                      {chip.strong ? <b>{chip.strong}</b> : null}
                    </span>
                  ))}
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Specimen() {
  const [theme, setTheme] = useState<"dark" | "light">(
    new URLSearchParams(window.location.search).get("theme") === "light" ? "light" : "dark",
  );
  const [named, setNamed] = useState(false);
  const [sampleIndex, setSampleIndex] = useState(0);
  // The strip reflows twice on the way down: the verbs wrap under the chips at
  // 720, and at 500 the chips become one per line — which is where a candidate
  // that adds a chip has to pay for it.
  const [width, setWidth] = useState(900);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  const sample = SAMPLES[sampleIndex];

  return (
    <div className="specimen-page">
      <header className="specimen-head">
        <div>
          <h1>The session-detail “when” chip</h1>
          <p>
            Four shapes for the one chip that currently carries a start, an end and a length.
            “Now” is frozen at <b>Thu Aug 20 2026, 15:20</b>; the baseline column calls the
            retired <code>formatSessionRange</code>. C is what ships now.
          </p>
        </div>
        <div className="specimen-controls">
          <label>
            <input type="checkbox" checked={theme === "light"} onChange={(e) => setTheme(e.target.checked ? "light" : "dark")} />
            {" "}light
          </label>
          <label>
            <input type="checkbox" checked={named} onChange={(e) => setNamed(e.target.checked)} />
            {" "}name Today / Yesterday
          </label>
          <label>
            width <input type="range" min={360} max={1000} value={width} onChange={(e) => setWidth(Number(e.target.value))} />
          </label>
          <b>{width}px</b>
        </div>
      </header>

      <section>
        <h2>1 · Every session shape</h2>
        <p className="specimen-lead">
          Eight shapes a real session takes. Read down a column: the one you want keeps its
          shape when the session does something unusual — crosses midnight, is still running,
          happened in another year.
        </p>
        <div className="specimen-scroll"><Ladder named={named} /></div>
      </section>

      <section>
        <h2>2 · In the strip</h2>
        <div className="specimen-picker">
          {SAMPLES.map((entry, index) => (
            <button
              key={entry.note}
              type="button"
              className={index === sampleIndex ? "on" : undefined}
              onClick={() => setSampleIndex(index)}
            >{entry.note}</button>
          ))}
        </div>
        <div className="specimen-frames" style={{ width }}>
          <div className="specimen-frame">
            <div className="specimen-frame-label">
              <b>What ships</b><span>C, rendered by SessionDetailHeader itself</span>
            </div>
            <div className="specimen-surface session-detail-page">
              <Shipped sample={sample} />
            </div>
          </div>
          {CANDIDATES.map((candidate) => (
            <div className="specimen-frame" key={candidate.id}>
              <div className="specimen-frame-label">
                <b>{candidate.name}</b><span>{candidate.blurb}</span>
              </div>
              <div className="specimen-surface session-detail-page">
                <HeaderStrip plan={candidate.plan(sample, named && candidate.id !== "now")} />
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
