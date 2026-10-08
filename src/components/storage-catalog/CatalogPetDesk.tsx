import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "../../i18n";

/*
 * A desktop pet on its page, played the way the Codex app plays it — the
 * player Mallard Cloud's pet page uses (mallardInternal web/src/pet.ts and
 * PetPage.tsx), redrawn: the pet idles beside a Codex window; dragged, or
 * moved with the arrow keys, it runs that way; an atlas with look rows turns
 * toward the pointer. The moment buttons play the row Codex shows at that
 * moment and redraw the window to match. Pause holds the frame, and Reduce
 * Motion starts paused.
 */

/** One atlas cell: 192 × 208 in every format version. */
const CELL_WIDTH = 192;
const CELL_HEIGHT = 208;
const DESK_HEIGHT = 300;
const HALF = CELL_WIDTH / 2;

type StateKey = "idle" | "running-right" | "running-left" | "failed" | "waiting" | "running" | "review";
export type PetMoment = "idle" | "running" | "waiting" | "review" | "failed";

const steady = (frames: number, each: number, last: number): number[] => [...Array<number>(frames - 1).fill(each), last];

/** One row per state, with its own frame durations in milliseconds; a row loops. */
const STATES: Readonly<Record<StateKey, { row: number; durations: readonly number[] }>> = {
  idle: { row: 0, durations: [280, 110, 110, 140, 140, 320] },
  "running-right": { row: 1, durations: steady(8, 120, 220) },
  "running-left": { row: 2, durations: steady(8, 120, 220) },
  failed: { row: 5, durations: steady(8, 140, 240) },
  waiting: { row: 6, durations: steady(6, 150, 260) },
  running: { row: 7, durations: steady(6, 120, 220) },
  review: { row: 8, durations: steady(6, 150, 280) },
};

/** The moments in the order the buttons show them, each by its label's key. */
const MOMENTS = [
  { state: "idle", label: "pet.idle" },
  { state: "running", label: "pet.working" },
  { state: "waiting", label: "pet.waiting" },
  { state: "review", label: "pet.review" },
  { state: "failed", label: "pet.failed" },
] as const satisfies ReadonlyArray<{ state: PetMoment; label: string }>;

/** The look direction (0 = up, clockwise, 0–15) nearest the bearing to a point. */
function lookIndex(dx: number, dy: number): number {
  const degrees = (Math.atan2(dx, -dy) * 180) / Math.PI;
  return Math.round(((degrees + 360) % 360) / 22.5) % 16;
}

function useReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() => typeof matchMedia === "function" && matchMedia(query).matches);
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const media = matchMedia(query);
    const update = () => setReduced(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return reduced;
}

/** Steps through a state's row at its own durations; a new state starts at frame 0. */
function useFrame(key: StateKey, playing: boolean): number {
  const { durations } = STATES[key];
  const [at, setAt] = useState({ key, frame: 0 });
  const frame = at.key === key ? at.frame % durations.length : 0;
  useEffect(() => {
    if (!playing) return;
    const timer = window.setTimeout(() => setAt({ key, frame: (frame + 1) % durations.length }), durations[frame]);
    return () => window.clearTimeout(timer);
  }, [key, frame, playing, durations]);
  return frame;
}

/** A Codex window drawn in blocks; only the approval it asks for has words. */
function CodexWindow({ moment }: { moment: PetMoment }) {
  const { t } = useTranslation("library");
  return (
    <div className="catalog-pet-window" aria-hidden="true">
      <div className="catalog-pet-window-bar"><i /><i /><i /></div>
      <div className="catalog-pet-window-body">
        <span className="catalog-pet-window-line" style={{ width: "58%" }} />
        <span className="catalog-pet-window-line" style={{ width: "74%" }} />
        <span className="catalog-pet-window-line" style={{ width: "36%" }} />
        {moment === "running" && <span className="catalog-pet-window-dots"><i /><i /><i /></span>}
        {moment === "waiting" && <span className="catalog-pet-window-ask"><span className="catalog-pet-window-buttons"><b>{t("pet.allow")}</b><b>{t("pet.deny")}</b></span></span>}
        {moment === "review" && <span className="catalog-pet-window-diff"><i className="is-del" /><i className="is-add" /><i className="is-add" /></span>}
        {moment === "failed" && <span className="catalog-pet-window-fail"><b>!</b></span>}
      </div>
    </div>
  );
}

function Desk({ atlas, name, columns, rows, moment, playing }: {
  atlas: string;
  name: string;
  columns: number;
  rows: number;
  moment: PetMoment;
  playing: boolean;
}) {
  const { t } = useTranslation("library");
  const desk = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [x, setX] = useState<number | null>(null);
  const [run, setRun] = useState<"left" | "right" | null>(null);
  const [look, setLook] = useState<number | null>(null);
  const grab = useRef<{ offset: number; last: number } | null>(null);
  const stop = useRef<number | undefined>(undefined);
  const glance = useRef<number | undefined>(undefined);
  /* Format v2 adds two rows of look directions after the nine state rows. */
  const looks = rows >= 11;

  useEffect(() => {
    const element = desk.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => () => {
    window.clearTimeout(stop.current);
    window.clearTimeout(glance.current);
  }, []);

  const clamp = (value: number) => Math.min(Math.max(value, HALF), Math.max(HALF, width - HALF));
  // The pet starts at the window's lower right corner, as on a real desktop.
  const center = clamp(x ?? width * 0.78);
  const state: StateKey = run === "right" ? "running-right" : run === "left" ? "running-left" : moment;
  const frame = useFrame(state, (playing || run !== null) && look === null);
  const cell = look !== null
    ? { row: 9 + Math.floor(look / columns), column: look % columns }
    : { row: STATES[state].row, column: frame };

  const runTo = (next: number, direction: "left" | "right") => {
    setX(clamp(next));
    setRun(direction);
    setLook(null);
    window.clearTimeout(stop.current);
    stop.current = window.setTimeout(() => setRun(null), 220);
  };
  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    grab.current = { offset: event.clientX - desk.current!.getBoundingClientRect().left - center, last: event.clientX };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const rect = desk.current!.getBoundingClientRect();
    if (grab.current) {
      const delta = event.clientX - grab.current.last;
      if (Math.abs(delta) < 1) return;
      grab.current.last = event.clientX;
      runTo(event.clientX - rect.left - grab.current.offset, delta > 0 ? "right" : "left");
      return;
    }
    if (!looks || run || !playing) return;
    // From the pet's eyes, about a third of the way down its cell.
    const dx = event.clientX - rect.left - center;
    const dy = event.clientY - rect.top - (DESK_HEIGHT - 14 - CELL_HEIGHT * 0.66);
    if (Math.hypot(dx, dy) < 28) return;
    setLook(lookIndex(dx, dy));
    window.clearTimeout(glance.current);
    glance.current = window.setTimeout(() => setLook(null), 1400);
  };
  const release = () => {
    grab.current = null;
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const right = event.key === "ArrowRight";
    runTo(center + (right ? 18 : -18), right ? "right" : "left");
  };

  const sprite: CSSProperties = {
    width: CELL_WIDTH,
    height: CELL_HEIGHT,
    backgroundImage: `url(${atlas})`,
    // i18n-ignore: CSS value
    backgroundSize: `${columns * CELL_WIDTH}px ${rows * CELL_HEIGHT}px`,
    // i18n-ignore: CSS value
    backgroundPosition: `${-cell.column * CELL_WIDTH}px ${-cell.row * CELL_HEIGHT}px`,
  };
  return (
    <div
      ref={desk}
      className="catalog-pet-desk"
      style={{ height: DESK_HEIGHT }}
      onPointerMove={onPointerMove}
      onPointerUp={release}
      onPointerCancel={release}
      onPointerLeave={() => {
        if (!grab.current) setLook(null);
      }}
    >
      <CodexWindow moment={moment} />
      {width > 0 && (
        <button
          type="button"
          className={`catalog-pet-desk-pet${run ? " is-running" : ""}`}
          style={{ left: center - HALF }}
          aria-label={t("pet.deskLabel", { name })}
          onPointerDown={onPointerDown}
          onPointerUp={release}
          onKeyDown={onKeyDown}
        >
          <span className="catalog-pet-sprite" style={sprite} aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

export default function CatalogPetDesk({ atlas, name, columns, rows }: {
  /** The whole atlas as a data URI, verified by the backend. */
  atlas: string;
  name: string;
  columns: number;
  rows: number;
}) {
  const { t } = useTranslation("library");
  const reduced = useReducedMotion();
  const [moment, setMoment] = useState<PetMoment>("idle");
  const [paused, setPaused] = useState<boolean | null>(null);
  /* Until a button says otherwise, Reduce Motion decides. */
  const playing = !(paused ?? reduced);
  return (
    <div className="catalog-pet">
      <Desk atlas={atlas} name={name} columns={columns} rows={rows} moment={moment} playing={playing} />
      <div className="catalog-pet-tools">
        <div className="catalog-pet-moments" role="group" aria-label={t("pet.moments")}>
          {MOMENTS.map((entry) => (
            <button
              key={entry.state}
              type="button"
              className="catalog-pet-moment"
              aria-pressed={moment === entry.state}
              onClick={() => {
                setMoment(entry.state);
                setPaused(false);
              }}
            >
              {t(entry.label)}
            </button>
          ))}
        </div>
        <button type="button" className="btn btn-ghost catalog-pet-play" aria-pressed={!playing} onClick={() => setPaused(playing)}>
          {playing ? t("pet.pause") : t("pet.play")}
        </button>
      </div>
    </div>
  );
}
