import { StrictMode, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import StartupLoadingScreen, { type StartupStage } from "../src/components/project-sync/StartupLoadingScreen";
import "../src/App.css";
import "./startup-loading-mockup.css";

/**
 * Mockup: the startup screen, the one loading surface the 2026-08-25 loading
 * recipe and the 2026-09-26 review-loading pass did not touch.
 *
 *   Before  the shipped StartupLoadingScreen, rendered as is
 *   A Bob        the pixel mark bobs on a waterline; six stage cells
 *   B Paddle     the same duck and cells, laid out as a route: the duck's
 *                position is the stage
 *   C Ride       the welcome ride plate loops under the brand row
 *   D Checklist  the six stages as rows in an ink box
 *
 * Every direction moves by repaint at whole pixels (fill, background-position,
 * left in 8 px steps). Nothing eases and nothing promotes a compositing layer,
 * because App.css's motion note records crisp art flickering on WebKit layers.
 * Stage timings are ASSUMED: nobody has measured a real boot yet.
 */

const STAGES: ReadonlyArray<{ id: StartupStage; status: string; short: string; row: string }> = [
  { id: "installation", status: "Checking installation…", short: "Install", row: "Installation" },
  { id: "settings", status: "Loading settings…", short: "Settings", row: "Settings" },
  { id: "repair", status: "Finishing interrupted work…", short: "Repair", row: "Recovery check" },
  { id: "projects", status: "Finding projects…", short: "Projects", row: "Projects" },
  { id: "providers", status: "Checking profiles…", short: "Profiles", row: "Profiles" },
  { id: "workspace", status: "Opening workspace…", short: "Workspace", row: "Workspace" },
];
const LAST = STAGES.length - 1;

type Direction = "before" | "a" | "b" | "c" | "d";
type View = Direction | "all";
type Boot = "warm" | "typical" | "slow";
type Gate = "none" | "gate" | "hold";
type Ride = "bicycle" | "rocket" | "submarine";

const BOOTS: Record<Boot, { label: string; durations: number[] }> = {
  warm: { label: "Warm ≈0.3 s", durations: [30, 25, 45, 90, 60, 50] },
  typical: { label: "Typical ≈1.2 s", durations: [90, 80, 260, 380, 240, 150] },
  slow: { label: "Slow repair ≈7 s", durations: [120, 100, 5600, 520, 380, 180] },
};
const GATES: Record<Gate, string> = {
  none: "None (today)",
  gate: "150 ms gate",
  hold: "Gate + 600 ms hold",
};
const GATE_MS = 150;
const HOLD_MS = 600;

const DIRECTIONS: Record<Direction, { name: string; tag?: string }> = {
  before: { name: "Before · today" },
  a: { name: "A · Bob" },
  b: { name: "B · Paddle", tag: "Recommended" },
  c: { name: "C · Ride" },
  d: { name: "D · Checklist" },
};

/* ——— the pixel mark, split so the head can dip ——————————————————————— */

type Cell = readonly [number, number, number, number];
const HEAD: Cell[] = [[13, 3, 3, 1], [12, 4, 5, 2], [12, 6, 7, 1], [12, 7, 3, 3]];
const BODY: Cell[] = [[7, 10, 9, 1], [2, 11, 15, 1], [1, 12, 16, 2], [2, 14, 15, 1], [3, 15, 13, 1], [5, 16, 10, 1]];
const cells = (list: Cell[]) => list.map(([x, y, w, h]) => `M${x} ${y}h${w}v${h}h${-w}Z`).join("");

/** The brand mark as inline SVG on currentColor: no invert filter in dark. */
function MallardMark({ size }: { size: 20 | 40 | 60 | 80 }) {
  return (
    <svg className="mk-mark" viewBox="0 0 20 20" width={size} height={size} shapeRendering="crispEdges" aria-hidden="true">
      <path d={cells([...HEAD, ...BODY])} />
    </svg>
  );
}

/** A duck that bobs one cell on the 0.9 s app clock and dips its head every
 *  other beat. `wake` adds two blinking cells behind it (B). */
function BobbingDuck({ wake = false }: { wake?: boolean }) {
  return (
    <g className="mk-bob">
      {wake && (
        <g className="mk-wake">
          <rect x={-6} y={15} width={2} height={1} />
          <rect x={-3} y={15} width={2} height={1} />
        </g>
      )}
      <path className="mk-head" d={cells(HEAD)} />
      <path d={cells(BODY)} />
    </g>
  );
}

/* ——— shared parts ——————————————————————————————————————————————— */

function StageCells({ index }: { index: number }) {
  return (
    <div
      className="mk-cells"
      role="progressbar"
      aria-label="Mallard startup progress"
      aria-valuemin={1}
      aria-valuemax={STAGES.length}
      aria-valuenow={index + 1}
      aria-valuetext={STAGES[index].status}
    >
      {STAGES.map((stage, i) => (
        <i key={stage.id} className={i < index ? "done" : i === index ? "now" : undefined} />
      ))}
    </div>
  );
}

function Status({ index, hidden = false }: { index: number; hidden?: boolean }) {
  return (
    <p className={hidden ? "mk-status mk-visually-hidden" : "mk-status"} role="status" aria-live="polite" aria-atomic="true">
      {STAGES[index].status}
    </p>
  );
}

function Screen({ children, label }: { children?: ReactNode; label: string }) {
  return (
    <main className="sync-startup-screen mk-screen" aria-label={label}>
      <div className="sync-startup-drag-region" data-tauri-drag-region />
      {children && <section className="mk-content">{children}</section>}
    </main>
  );
}

/* ——— A · Bob ——————————————————————————————————————————————————— */

function DirectionA({ index }: { index: number }) {
  return (
    <>
      <svg className="mk-a-scene" viewBox="0 0 30 20" width={120} height={80} shapeRendering="crispEdges" aria-hidden="true">
        <g transform="translate(5 0)">
          <BobbingDuck />
        </g>
        {/* Paper below the waterline hides the duck's bottom row on the
            down beat, so it sinks instead of poking out under the line. */}
        <rect className="mk-under" x={0} y={17} width={30} height={1} />
        <g className="mk-water">
          {Array.from({ length: 10 }, (_, i) => (
            <rect key={i} x={i * 4} y={16} width={3} height={1} />
          ))}
        </g>
        <g className="mk-ripple">
          {Array.from({ length: 6 }, (_, i) => (
            <rect key={i} x={2 + i * 7} y={18} width={2} height={1} />
          ))}
        </g>
      </svg>
      <h1 className="mk-wordmark">Mallard</h1>
      <StageCells index={index} />
      <Status index={index} />
    </>
  );
}

/* ——— B · Paddle ———————————————————————————————————————————————— */

const POND_STEP = 64;
const POND_FIRST = 32;
const markerX = (i: number) => POND_FIRST + i * POND_STEP;

/** Moves toward `target` in whole-pixel steps every 20 ms, at least 8 px and
 *  at most 8 ticks for any gap (≈160 ms), so the duck is never more than a
 *  beat behind the real stage even when stages land faster than it moves.
 *  A CSS steps() transition cannot promise whole pixels when retargeted. */
function useStepper(target: number, reduced: boolean) {
  const [x, setX] = useState(target);
  useEffect(() => {
    if (reduced) {
      setX(target);
      return;
    }
    const id = window.setInterval(() => {
      setX((current) => {
        if (current === target) return current;
        const gap = Math.abs(target - current);
        const step = Math.min(gap, Math.max(8, Math.ceil(gap / 8)));
        return current + Math.sign(target - current) * step;
      });
    }, 20);
    return () => window.clearInterval(id);
  }, [target, reduced]);
  return x;
}

function PondFlag() {
  return (
    <svg viewBox="0 0 6 7" width={18} height={21} shapeRendering="crispEdges" aria-hidden="true">
      <path d="M0 0h1v7h-1Z M1 0h4v1h-4Z M1 1h3v1h-3Z M1 2h2v1h-2Z" />
    </svg>
  );
}

function DirectionB({ index, reduced }: { index: number; reduced: boolean }) {
  // The duck's body centre is cell 9: 27 px at 3 px a cell.
  const duckX = useStepper(markerX(index) - 27, reduced);
  return (
    <>
      <h1 className="mk-wordmark">Mallard</h1>
      <div
        className="mk-pond"
        role="progressbar"
        aria-label="Mallard startup progress"
        aria-valuemin={1}
        aria-valuemax={STAGES.length}
        aria-valuenow={index + 1}
        aria-valuetext={STAGES[index].status}
      >
        <svg
          className="mk-pond-duck"
          style={{ left: `${duckX - 18}px` }}
          viewBox="-6 0 26 20"
          width={78}
          height={60}
          shapeRendering="crispEdges"
          aria-hidden="true"
        >
          <BobbingDuck wake />
        </svg>
        <span className="mk-pond-water" aria-hidden="true" />
        {STAGES.map((stage, i) =>
          i === LAST ? (
            <span key={stage.id} className={`mk-pond-flag${i === index ? " now" : ""}`} style={{ left: `${markerX(i) - 1}px` }} aria-hidden="true">
              <PondFlag />
            </span>
          ) : (
            <span
              key={stage.id}
              className={`mk-pond-marker${i < index ? " done" : i === index ? " now" : ""}`}
              style={{ left: `${markerX(i) - 4}px` }}
              aria-hidden="true"
            />
          ),
        )}
      </div>
      <Status index={index} />
    </>
  );
}

/* ——— C · Ride ——————————————————————————————————————————————————— */

function DirectionC({ index, reduced, ride, play }: { index: number; reduced: boolean; ride: Ride; play: number }) {
  return (
    <>
      <div className="mk-brand">
        <MallardMark size={40} />
        <h1 className="mk-wordmark">Mallard</h1>
      </div>
      <div className="projects-welcome-art mk-ride" aria-hidden="true">
        <img src={`/welcome/mallard-${ride}-still.svg`} alt="" width={1200} height={560} />
        {!reduced && <img src={`/welcome/mallard-${ride}-ride.gif?play=${play}`} alt="" width={1200} height={560} />}
      </div>
      <StageCells index={index} />
      <Status index={index} />
    </>
  );
}

/* ——— D · Checklist ———————————————————————————————————————————————— */

const SPIN_CELLS: Array<[number, number]> = [[0, 0], [4, 0], [8, 0], [8, 4], [8, 8], [4, 8], [0, 8], [0, 4]];

function PixelSpinner() {
  return (
    <svg className="mk-spin" viewBox="0 0 11 11" width={11} height={11} shapeRendering="crispEdges" aria-hidden="true">
      {SPIN_CELLS.map(([x, y], i) => (
        <rect key={i} x={x} y={y} width={3} height={3} style={{ animationDelay: `${(i - SPIN_CELLS.length) * 0.1125}s` }} />
      ))}
    </svg>
  );
}

function PixelTick() {
  return (
    <svg className="mk-tick" viewBox="0 0 6 5" width={12} height={10} shapeRendering="crispEdges" aria-hidden="true">
      <path d="M5 0h1v1h-1Z M4 1h1v1h-1Z M0 2h1v1h-1Z M3 2h1v1h-1Z M1 3h2v1h-2Z" />
    </svg>
  );
}

function DirectionD({ index, elapsedMs }: { index: number; elapsedMs: number }) {
  return (
    <>
      <div className="mk-brand">
        <MallardMark size={40} />
        <h1 className="mk-wordmark">Mallard</h1>
      </div>
      <ol className="mk-list" aria-hidden="true">
        {STAGES.map((stage, i) => (
          <li key={stage.id} className={i < index ? "done" : i === index ? "now" : undefined}>
            <span className="mk-list-glyph">{i < index ? <PixelTick /> : i === index ? <PixelSpinner /> : <i />}</span>
            <span>{stage.row}</span>
            {i === index && elapsedMs >= 2000 && <time>{`${Math.floor(elapsedMs / 1000)} s`}</time>}
          </li>
        ))}
      </ol>
      <Status index={index} hidden />
    </>
  );
}

/* ——— the window, the shell stand-in, the clock ——————————————————————— */

function ShellStandIn() {
  return (
    <div className="mk-shell">
      <aside className="mk-shell-side">
        <div className="mk-shell-brand">
          <MallardMark size={20} />
          <strong>Mallard</strong>
        </div>
        <i />
        <i />
        <i />
      </aside>
      <section className="mk-shell-main">
        <div className="mk-shell-bar" />
        <div className="mk-shell-rows">
          <i />
          <i />
          <i />
          <i />
        </div>
      </section>
      <span className="mk-shell-tag">Workspace (stand-in)</span>
    </div>
  );
}

function Window({ children, size, reduced }: { children: ReactNode; size: "full" | "small"; reduced: boolean }) {
  return (
    <div className={`mk-window ${size}${reduced ? " mk-reduced" : ""}`}>
      <span className="mk-lights" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
      {children}
    </div>
  );
}

interface Moment {
  index: number;
  visible: boolean;
  done: boolean;
  elapsedMs: number;
}

function shellAt(durations: number[], gate: Gate) {
  const total = durations.reduce((sum, ms) => sum + ms, 0);
  if (gate !== "hold" || total <= GATE_MS) return total;
  return Math.max(total, GATE_MS + HOLD_MS);
}

function momentAt(t: number, durations: number[], gate: Gate): Moment {
  let start = 0;
  let index = LAST;
  for (let i = 0; i < durations.length; i += 1) {
    if (t < start + durations[i]) {
      index = i;
      break;
    }
    start += durations[i];
  }
  const total = durations.reduce((sum, ms) => sum + ms, 0);
  if (t >= total) start = total - durations[LAST];
  return {
    index,
    visible: gate === "none" || t >= GATE_MS,
    done: t >= shellAt(durations, gate),
    elapsedMs: t - start,
  };
}

function useClock(running: boolean, endMs: number, runKey: number) {
  const [t, setT] = useState(0);
  useEffect(() => {
    if (!running) return;
    setT(0);
    const start = performance.now();
    const id = window.setInterval(() => {
      const elapsed = performance.now() - start;
      setT(elapsed);
      if (elapsed >= endMs + 200) window.clearInterval(id);
    }, 16);
    return () => window.clearInterval(id);
  }, [running, endMs, runKey]);
  return t;
}

function DirectionFrame({
  direction,
  moment,
  size,
  reduced,
  ride,
  play,
}: {
  direction: Direction;
  moment: Moment;
  size: "full" | "small";
  reduced: boolean;
  ride: Ride;
  play: number;
}) {
  let body: ReactNode;
  if (moment.done) body = <ShellStandIn />;
  else if (direction === "before") body = <StartupLoadingScreen stage={STAGES[moment.index].id} />;
  else if (!moment.visible) body = <Screen label="Mallard is starting" />;
  else {
    const scene = {
      a: <DirectionA index={moment.index} />,
      b: <DirectionB index={moment.index} reduced={reduced} />,
      c: <DirectionC index={moment.index} reduced={reduced} ride={ride} play={play} />,
      d: <DirectionD index={moment.index} elapsedMs={moment.elapsedMs} />,
    }[direction];
    body = <Screen label="Mallard is starting">{scene}</Screen>;
  }
  return (
    <Window size={size} reduced={reduced}>
      {body}
    </Window>
  );
}

/* ——— notes ——————————————————————————————————————————————————————— */

const NOTES: Record<Direction, { idea: string; points: Array<[string, string]> }> = {
  before: {
    idea: "Today's screen: the mark (an <img>, inverted by filter in dark) and the wordmark, a 2 px striped bar, and a status line.",
    points: [
      ["Progress", "The bar's fill is never drawn. The track is 2 px tall with a 1 px border under border-box, so the fill span measures 0 px high, and so does its glint. What shows is an empty outline. The widths it would fill to (12 / 28 / 46 / 64 / 80 / 96 %) are invented anyway."],
      ["Motion", "A 7 px pulse dot and a 22 px glint on a 2 px track. When repair runs for seconds, nothing on screen reads as working."],
      ["Pixels", "The content rises 5 px on an eased cubic-bezier. That is a sub-pixel transform on crisp pixel art, the WebKit flicker the App.css motion note warns about."],
      ["Gate", "There is none. On a warm boot the screen flashes for a few hundred ms and then cuts to the shell."],
    ],
  },
  a: {
    idea: "The brand mark comes alive. The duck bobs on a pixel waterline, and six cells under the wordmark stand for the six real stages.",
    points: [
      ["Moves", "The duck bobs 1 cell (4 px) on the 0.9 s app clock, and the head dips one cell every other beat. The water drifts left 1 cell per beat, and the current cell blinks. Every move uses steps() at whole cells."],
      ["Progress", "Done cells are filled, the current one blinks, pending ones are hollow. The count comes from STARTUP_STAGES.length, and no fill creeps between stages."],
      ["Warm boot", "The duck and the name read in the first frame, and the cells fill in a blink."],
      ["Slow repair", "The duck keeps bobbing, cell 3 blinks, and the status line names the stage."],
      ["Reduced motion", "Rest pose: duck up, still water. The cells and the status still update."],
      ["Cost", "About 1 KB of inline SVG, with paint-only animation. Lowest risk."],
    ],
  },
  b: {
    idea: "A's duck and cells, laid out as a route. The duck paddles from marker to marker, so its position is the stage. The last marker is a flag: the workspace.",
    points: [
      ["Moves", "The duck moves in whole-pixel steps every 20 ms and closes any gap within about 160 ms (one stage is 64 px in 8 px steps), so it never runs more than a beat behind the real stage. Parked, it bobs, its wake blinks, and the water drifts under it. The duck is the 20-cell mark at 3 px a cell."],
      ["Progress", "Markers under the waterline fill as the duck passes. They come from the same stage index as A's cells."],
      ["Warm boot", "When the gate opens at 150 ms, the duck is already at the current marker (measured: marker 4). It is mid-hop toward the flag when the shell cuts in at about 0.3 s. On a warm boot B reads as \"almost at the flag\", not as a journey; the journey shows only on typical and slow boots (typical: it trails the real stage by at most about 160 ms). Its first frame (name, duck, flag) still reads at a glance."],
      ["Slow repair", "The duck parks at marker 3 and paddles in place. It looks alive, not frozen, and the status line names the stage."],
      ["Reduced motion", "The duck jumps straight to its marker. No bob, no wake, still water."],
      ["Cost", "A's parts plus one interval timer while the duck travels. Still under 2 KB, all paint."],
    ],
  },
  c: {
    idea: "The welcome ride plate (bicycle, rocket, or submarine) loops under the brand row, with A's cells and the status line below it.",
    points: [
      ["Moves", "The ride GIF. All three loop forever (NETSCAPE2.0 loop count 0, checked)."],
      ["Weight", "0.8–1.7 MB of GIF, decoded while the backend is busiest."],
      ["First run", "Startup hands straight to ProjectsWelcome, which plays another ride, so a first run shows two rides back to back."],
      ["Dark", "The bright day plate sits on Lamplight sepia (compare in Dark). The GIFs are opaque with near-white corners and .projects-welcome-art does not clip, so the shipped welcome plate shows white corner nubs in dark mode while it rides. The mock clips its own plate."],
      ["WCAG 2.2.2", "Auto-moving content that runs past 5 s beside other content needs a pause control. A loading-only screen is arguably exempt, but a slow repair can run long."],
      ["Reduced motion", "Shows the still SVG, the same swap ProjectsWelcome does."],
    ],
  },
  d: {
    idea: "The six stages as rows in an ink box: a pixel tick for done, a pixel spinner for current, a hollow cell for pending. After 2 s the current row shows its elapsed seconds.",
    points: [
      ["Moves", "Only the spinner: 8 cells on the 0.9 s clock."],
      ["Strength", "The most honest version. \"Profiles · 14 s\" says exactly where a slow start is, which helps a bug report."],
      ["Warm boot", "Six ticks flash in about 150 ms, which looks busy."],
      ["Every boot", "Lists a recovery check even when nothing was interrupted. It reads as a log, not the brand."],
      ["Screen readers", "The rows are aria-hidden. The one live region is a visually hidden status line, so nobody hears six stages in 300 ms."],
    ],
  },
};

const SHARED: Array<[string, string]> = [
  ["Honest progress", "Drop the percentage table and drive everything from the stage index. Keep the progressbar contract: valuemin 1, valuemax STARTUP_STAGES.length, valuenow index + 1, valuetext = the status."],
  ["150 ms gate", "The background paints at once and the content waits 150 ms, the same rule as LoadingScaffold. A boot under 150 ms shows no screen at all. The specimen's Gate control also tries a 600 ms minimum hold (open question)."],
  ["Pixel-safe motion", "Use steps() only, whole cells, and paint-only properties (fill, background-position, left). No transform or opacity on HTML boxes, and sync-startup-rise goes."],
  ["Reduced motion", "Every loop is authored so that its base style is the rest pose: duck up, still water, current cell filled. The loops set no fill mode, so the global 0.01 ms rule already ends each one on that base, and no extra rule is needed (docs/PLAN_STARTUP_LOADING.md R5). B's duck jumps straight to its marker."],
  ["Mark as SVG", "A MallardMark component on currentColor replaces the <img> plus invert filter. Cells can then move and follow the theme."],
  ["Hand-off", "Cut to the shell at once (\"states land at once\"). The only motion precedent is retro-workspace-enter: 150 ms, steps(4)."],
  ["Unverified", "index.html paints nothing before the bundle's CSS loads, so a dark-theme user may see a white frame first. The CSP allows inline <style> but not script. Check the packaged app before fixing anything."],
];

/* ——— the page —————————————————————————————————————————————————————— */

function readParams() {
  const p = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = p.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    view: pick<View>("dir", ["before", "a", "b", "c", "d", "all"], "all"),
    moment: pick("moment", ["1", "2", "3", "4", "5", "6", "live"], "3"),
    boot: pick<Boot>("boot", ["warm", "typical", "slow"], "warm"),
    gate: pick<Gate>("gate", ["none", "gate", "hold"], "gate"),
    theme: pick("theme", ["light", "dark"], "light"),
    motion: pick("motion", ["full", "reduced"], "full"),
    ride: pick<Ride>("ride", ["bicycle", "rocket", "submarine"], "bicycle"),
  };
}

function Seg<T extends string>({ value, current, onPick, children, disabled }: {
  value: T;
  current: T;
  onPick: (value: T) => void;
  children: ReactNode;
  disabled?: boolean;
}) {
  return (
    <button type="button" className="mk-seg" aria-pressed={value === current} disabled={disabled} onClick={() => onPick(value)}>
      {children}
    </button>
  );
}

function Timeline({ t, durations, gate, moment }: { t: number; durations: number[]; gate: Gate; moment: Moment }) {
  const end = shellAt(durations, gate);
  const span = Math.max(end, durations.reduce((sum, ms) => sum + ms, 0));
  const pct = (ms: number) => `${Math.min(100, (ms / span) * 100)}%`;
  const total = durations.reduce((sum, ms) => sum + ms, 0);
  const contentShown = gate === "none" || total > GATE_MS;
  return (
    <div className="mk-timeline">
      <div className="mk-timeline-track">
        {durations.map((ms, i) => (
          <span key={STAGES[i].id} style={{ flexGrow: ms }} className={i < moment.index || moment.done ? "done" : i === moment.index ? "now" : undefined}>
            {STAGES[i].short}
          </span>
        ))}
        {end > total && <span className="hold" style={{ flexGrow: end - total }}>hold</span>}
        {gate !== "none" && <b className="mk-timeline-gate" style={{ left: pct(GATE_MS) }} title="Content appears" />}
        <b className="mk-timeline-head" style={{ left: pct(Math.min(t, span)) }} />
      </div>
      <p className="mk-timeline-readout">
        t = {(Math.min(t, end) / 1000).toFixed(2)} s · {moment.done ? "shell" : `stage ${moment.index + 1}/${STAGES.length}`} · shell at {(end / 1000).toFixed(2)} s
        {gate !== "none" && (contentShown ? ` · content from ${(GATE_MS / 1000).toFixed(2)} s` : " · content never shown (boot beat the gate)")}
      </p>
    </div>
  );
}

function Notes({ direction }: { direction: Direction }) {
  const note = NOTES[direction];
  return (
    <div className="mk-notes">
      <p className="mk-notes-idea">{note.idea}</p>
      <dl>
        {note.points.map(([label, text]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{text}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function Specimen() {
  const initial = useRef(readParams()).current;
  const [view, setView] = useState<View>(initial.view);
  const [momentKey, setMomentKey] = useState(initial.moment);
  const [boot, setBoot] = useState<Boot>(initial.boot);
  const [gate, setGate] = useState<Gate>(initial.gate);
  const [theme, setTheme] = useState(initial.theme);
  const [motion, setMotion] = useState(initial.motion);
  const [ride, setRide] = useState<Ride>(initial.ride);
  const [runKey, setRunKey] = useState(0);

  const live = momentKey === "live";
  const reduced = motion === "reduced";
  const durations = BOOTS[boot].durations;
  const end = shellAt(durations, gate);
  const t = useClock(live, end, runKey);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  useEffect(() => {
    const p = new URLSearchParams({ dir: view, moment: momentKey, boot, gate, theme, motion, ride });
    window.history.replaceState(null, "", `?${p.toString()}`);
  }, [view, momentKey, boot, gate, theme, motion, ride]);

  const fixedIndex = live ? 0 : Number(momentKey) - 1;
  const momentFor = (direction: Direction): Moment =>
    live
      ? momentAt(t, durations, direction === "before" ? "none" : gate)
      : { index: fixedIndex, visible: true, done: false, elapsedMs: fixedIndex === 2 ? 4200 : 0 };

  const replay = () => {
    setMomentKey("live");
    setRunKey((key) => key + 1);
  };

  const frames: Direction[] = view === "all" ? ["before", "a", "b", "c", "d"] : [view];

  return (
    <div className="mk-page">
      <header className="mk-head">
        <h1>Startup loading — directions</h1>
        <p>
          The screen between launching Mallard and the workspace (<code>StartupLoadingScreen</code>). In-page loading was
          redone on 2026-08-25 and 2026-09-26, so it is out of scope here. Before renders the shipped component. A–D are mock
          only. All of them are driven by the same six real stages. <b>Stage timings are assumed</b>: no real boot has been
          measured.
        </p>
      </header>

      <div className="mk-controls">
        <div className="mk-group">
          <span>Direction</span>
          <Seg value="all" current={view} onPick={setView}>Side by side</Seg>
          {(Object.keys(DIRECTIONS) as Direction[]).map((key) => (
            <Seg key={key} value={key} current={view} onPick={setView}>
              {DIRECTIONS[key].name}
            </Seg>
          ))}
        </div>
        <div className="mk-group">
          <span>Moment</span>
          {STAGES.map((stage, i) => (
            <Seg key={stage.id} value={String(i + 1) as typeof momentKey} current={momentKey} onPick={setMomentKey}>
              {i + 1} {stage.short}
            </Seg>
          ))}
          <button type="button" className="mk-seg mk-live" aria-pressed={live} onClick={replay}>
            {live ? "↻ Replay" : "▶ Live boot"}
          </button>
        </div>
        <div className="mk-group">
          <span>Boot</span>
          {(Object.keys(BOOTS) as Boot[]).map((key) => (
            <Seg key={key} value={key} current={boot} onPick={(value) => { setBoot(value); if (live) setRunKey((k) => k + 1); }}>
              {BOOTS[key].label}
            </Seg>
          ))}
        </div>
        <div className="mk-group">
          <span>Gate</span>
          {(Object.keys(GATES) as Gate[]).map((key) => (
            <Seg key={key} value={key} current={gate} onPick={(value) => { setGate(value); if (live) setRunKey((k) => k + 1); }}>
              {GATES[key]}
            </Seg>
          ))}
        </div>
        <div className="mk-group">
          <span>Theme</span>
          <Seg value="light" current={theme} onPick={setTheme}>Light</Seg>
          <Seg value="dark" current={theme} onPick={setTheme}>Dark</Seg>
          <span>Motion</span>
          <Seg value="full" current={motion} onPick={setMotion}>Full</Seg>
          <Seg value="reduced" current={motion} onPick={setMotion}>Reduced</Seg>
          <span>Ride (C)</span>
          {(["bicycle", "rocket", "submarine"] as Ride[]).map((key) => (
            <Seg key={key} value={key} current={ride} onPick={setRide}>
              {key[0].toUpperCase() + key.slice(1)}
            </Seg>
          ))}
        </div>
      </div>

      {live && <Timeline t={t} durations={durations} gate={gate} moment={momentFor("a")} />}

      {view === "all" ? (
        <div className="mk-grid">
          {frames.map((direction) => (
            <figure key={direction} className="mk-figure">
              <figcaption>
                <button type="button" className="mk-figure-title" onClick={() => setView(direction)}>
                  {DIRECTIONS[direction].name}
                </button>
                {DIRECTIONS[direction].tag && <em>{DIRECTIONS[direction].tag}</em>}
                <span>{NOTES[direction].idea}</span>
              </figcaption>
              <DirectionFrame direction={direction} moment={momentFor(direction)} size="small" reduced={reduced} ride={ride} play={runKey} />
            </figure>
          ))}
        </div>
      ) : (
        <div className="mk-single">
          <DirectionFrame direction={view} moment={momentFor(view)} size="full" reduced={reduced} ride={ride} play={runKey} />
          <aside>
            <h2>
              {DIRECTIONS[view].name}
              {DIRECTIONS[view].tag && <em>{DIRECTIONS[view].tag}</em>}
            </h2>
            <Notes direction={view} />
          </aside>
        </div>
      )}

      <section className="mk-panel">
        <h2>Shared plan (any direction)</h2>
        <dl>
          {SHARED.map(([label, text]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd>{text}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}

// Like main.tsx: the theme lands before the first paint, never after it.
document.documentElement.dataset.theme = readParams().theme;
document.documentElement.style.colorScheme = readParams().theme;

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
