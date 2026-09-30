import { StrictMode, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";

/**
 * Options for the Plugins kind mark, currently `puzzle`.
 *
 * The mark never appears alone: Skills (`lightbulb`) and Pets (`paw`) sit in
 * the same lists, so a candidate has to be distinct from those two before it is
 * anything else. Every candidate is drawn in the house grammar — hollow, on the
 * 24-unit grid, `crispEdges` — and measured the way the set was: rasterised,
 * one sample per coarse cell, which is what `size={14}` actually paints.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

interface Candidate {
  key: string;
  name: string;
  paths: string[];
  says: string;
  note: string;
}

const CANDIDATES: Candidate[] = [
  {
    key: "plug",
    name: "Plug",
    paths: ["M8 2h2v6H8Z", "M14 2h2v6h-2Z", "M5 8h14v2H5Z", "M5 10h2v5H5Z", "M17 10h2v5h-2Z",
      "M5 15h14v2H5Z", "M11 17h2v5h-2Z"],
    says: "a thing you plug in",
    note: "The metaphor the word itself carries, and the shape you pointed at. Two prongs, a hollow body, a lead. Nothing else in the set has prongs, so it is unmistakable beside lightbulb and paw.",
  },
  {
    key: "outlet",
    name: "Socket",
    paths: ["M4 4h16v2H4Z", "M4 6h2v12H4Z", "M18 6h2v12h-2Z", "M4 18h16v2H4Z",
      "M9 9h2v6H9Z", "M13 9h2v6h-2Z"],
    says: "the place a plugin goes",
    note: "The receiving half of the same metaphor. Risk: a rounded square holding two upright bars reads as a pause button, and the outer frame is close to `file` and `folder`.",
  },
  {
    key: "module",
    name: "Module",
    paths: ["M4 3h16v2H4Z", "M4 5h2v12H4Z", "M18 5h2v12h-2Z", "M4 17h16v2H4Z",
      "M7 19h2v3H7Z", "M11 19h2v3h-2Z", "M15 19h2v3h-2Z"],
    says: "a card that seats into something",
    note: "A cartridge with contact pins — the thing rather than the act. Reads as hardware, which may be too literal for a Codex plugin.",
  },
  {
    key: "jigsaw4",
    name: "Jigsaw, four-tabbed",
    paths: ["M5 5h14v2H5Z", "M5 7h2v10H5Z", "M17 7h2v10h-2Z", "M5 17h14v2H5Z",
      "M10 2h4v3h-4Z", "M19 10h3v4h-3Z", "M10 19h4v3h-4Z", "M2 10h3v4H2Z"],
    says: "the conventional extension mark",
    note: "The full toy, included to be measured rather than argued about. This is the form the current mark was reduced from, on the grounds that four tabs collapse at small sizes.",
  },
];

/* One point per coarse cell of the 12×12 grid the marks are authored on. */
function measure(paths: string[]): { ink: number; blobs: number } {
  const canvas = document.createElement("canvas");
  canvas.width = 24;
  canvas.height = 24;
  const ctx = canvas.getContext("2d")!;
  ctx.clearRect(0, 0, 24, 24);
  ctx.fillStyle = "#000";
  for (const d of paths) ctx.fill(new Path2D(d));
  const data = ctx.getImageData(0, 0, 24, 24).data;
  const grid: boolean[][] = [];
  let ink = 0;
  for (let y = 0; y < 12; y++) {
    grid[y] = [];
    for (let x = 0; x < 12; x++) {
      grid[y][x] = data[((y * 2 + 1) * 24 + (x * 2 + 1)) * 4 + 3] > 127;
      if (grid[y][x]) ink++;
    }
  }
  const seen = grid.map((r) => r.map(() => false));
  let blobs = 0;
  for (let y = 0; y < 12; y++) {
    for (let x = 0; x < 12; x++) {
      if (!grid[y][x] || seen[y][x]) continue;
      blobs++;
      const stack = [[x, y]];
      seen[y][x] = true;
      while (stack.length) {
        const [cx, cy] = stack.pop()!;
        for (let dy = -1; dy <= 1; dy++) {
          for (let dx = -1; dx <= 1; dx++) {
            const nx = cx + dx, ny = cy + dy;
            if (nx < 0 || ny < 0 || nx > 11 || ny > 11) continue;
            if (grid[ny][nx] && !seen[ny][nx]) { seen[ny][nx] = true; stack.push([nx, ny]); }
          }
        }
      }
    }
  }
  return { ink, blobs };
}

const PUZZLE = ["M2 2h16v2H2ZM2 4h2v6H2ZM16 4h2v6H16ZM4 8h4v2H4ZM18 8h4v2H18ZM6 10h2v6H6ZM20 10h2v6H20ZM2 14h4v2H2ZM16 14h4v2H16ZM2 16h2v6H2ZM16 16h2v6H16ZM4 20h12v2H4Z"];

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1060px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 18px; max-width: 84ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; margin: 0 0 22px; }
  .mk-opt { margin: 0 0 16px; border: 1px solid var(--retro-frame-edge); border-radius: var(--retro-panel-radius); background: var(--bg-1); overflow: hidden; }
  .mk-opt > header { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;
    padding: 9px 14px; border-bottom: 1px solid var(--border); background: var(--surface-subtle); }
  .mk-opt > header h2 { margin: 0; color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .mk-opt > header .says { color: var(--text-2); font-size: var(--type-sm); }
  .mk-ink { margin-left: auto; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-ink.out { color: var(--danger-text); }
  .mk-body { display: grid; grid-template-columns: 116px minmax(0, 1fr); gap: 16px; padding: 12px 14px 14px; align-items: start; }
  .mk-zoom { width: 108px; height: 108px; border: 1px solid var(--border); background: var(--bg-2); }
  .mk-actual { width: 84px; height: 84px; margin-top: 8px; border: 1px solid var(--border);
    background: var(--bg-2); color: var(--text-0); image-rendering: pixelated; }
  .mk-cap { margin: 5px 0 0; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-note { margin: 0 0 10px; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-strip { display: flex; align-items: center; gap: 12px; margin: 10px 0 0; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-strip svg { color: var(--text-1); }
  .mk-sibs { display: flex; align-items: center; gap: 10px; }
`;

function Mark({ paths, size = 14 }: { paths: string[]; size?: number }) {
  return (
    <svg aria-hidden="true" fill="none" height={size} shapeRendering="crispEdges" viewBox="0 0 24 24" width={size}>
      <g className="icon-pixel-fill icon-mallard-pixel" fill="currentColor" stroke="none">
        {paths.map((d) => <path key={d} d={d} />)}
      </g>
    </svg>
  );
}

function IndexRow({ paths }: { paths: string[] }) {
  return (
    <div className="sync-storage-index-list">
      <button type="button" className="sync-storage-index-row">
        <span className="sync-storage-repository-icon"><Mark paths={paths} /></span>
        <span className="sync-storage-index-copy">
          <strong>Plugins · Codex home</strong>
          <small>/Users/hequ/.codex</small>
        </span>
        <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
      </button>
    </div>
  );
}

/* What the eye actually gets: rasterised at the size it ships at, then blown up
   with smoothing off. A vector scale-up flatters a mark by showing detail the
   14px render never resolves. */
function Actual({ paths }: { paths: string[] }) {
  const ref = (node: HTMLCanvasElement | null) => {
    if (!node) return;
    const small = document.createElement("canvas");
    small.width = 14;
    small.height = 14;
    const sctx = small.getContext("2d")!;
    sctx.scale(14 / 24, 14 / 24);
    sctx.fillStyle = getComputedStyle(node).color;
    for (const d of paths) sctx.fill(new Path2D(d));
    const ctx = node.getContext("2d")!;
    ctx.clearRect(0, 0, node.width, node.height);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(small, 0, 0, node.width, node.height);
  };
  return <canvas ref={ref} width={84} height={84} className="mk-actual" />;
}

function Zoom({ paths }: { paths: string[] }) {
  return (
    <svg className="mk-zoom" viewBox="0 0 24 24" shapeRendering="crispEdges" aria-hidden="true">
      <g stroke="var(--border)" strokeWidth="0.09">
        {Array.from({ length: 13 }, (_, i) => (
          <g key={i}>
            <line x1={i * 2} y1="0" x2={i * 2} y2="24" />
            <line x1="0" y1={i * 2} x2="24" y2={i * 2} />
          </g>
        ))}
      </g>
      <g fill="var(--text-0)" stroke="none">{paths.map((d) => <path key={d} d={d} />)}</g>
    </svg>
  );
}

function Card({ name, says, note, paths, ink, blobs }: Omit<Candidate, "key"> & { ink: number; blobs: number }) {
  const out = ink < 20 || ink > 40;
  return (
    <div className="mk-opt">
      <header>
        <h2>{name}</h2>
        <span className="says">says: {says}</span>
        <span className={`mk-ink${out ? " out" : ""}`}>
          ink {ink} · {blobs} blob{blobs === 1 ? "" : "s"} · {out ? "outside 20–40" : "in band"}
        </span>
      </header>
      <div className="mk-body">
        <div>
          <Zoom paths={paths} />
          <Actual paths={paths} />
          <p className="mk-cap">shape · at 14px</p>
        </div>
        <div>
          <p className="mk-note">{note}</p>
          <IndexRow paths={paths} />
          <div className="mk-strip">
            <span className="mk-sibs">
              <Mark paths={paths} /> beside its siblings
              <Icon name="lightbulb" size={14} />
              <Icon name="catalog-pet-v1" size={14} />
            </span>
            · and the set: <Icon name="tool" size={14} /><Icon name="catalog" size={14} />
            <Icon name="file" size={14} /><Icon name="folder" size={14} /><Icon name="token" size={14} />
          </div>
        </div>
      </div>
    </div>
  );
}

function Mockup() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  const shipped = useMemo(() => measure(PUZZLE), []);
  const measured = useMemo(() => CANDIDATES.map((c) => ({ ...c, ...measure(c.paths) })), []);
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Plugin marks</h1>
      <p className="mk-lede">
        The mark never appears alone — Skills and Pets sit in the same lists — so being distinct from
        <Icon name="lightbulb" size={13} /> and <Icon name="catalog-pet-v1" size={13} /> comes before anything else.
        Each candidate is hollow, on the 24-unit grid, and measured against the set's band of 15–48 ink
        (mean 32). Judge them at 14px in the row; the plate is for shape only.
      </p>
      <div className="mk-bar">
        <button type="button" className="btn btn" onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
          {theme === "light" ? "Switch to dark" : "Switch to light"}
        </button>
      </div>
      <Card
        key="shipped"
        name="Shipped — puzzle"
        says="an extension"
        paths={PUZZLE}
        note="Apple's reduced jigsaw: one socket bitten out of the left edge, one knob on the right. Chosen because the four-tabbed form collapses at small sizes — the last card here shows what that costs."
        ink={shipped.ink}
        blobs={shipped.blobs}
      />
      {measured.map(({ key, ...rest }) => <Card key={key} {...rest} />)}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);
