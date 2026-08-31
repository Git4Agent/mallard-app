import { StrictMode, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";

/**
 * Options for the empty-rail mark: "No commits" in a window that could hold
 * some. The shipped mark is `clock`, which says *time* rather than *empty* and
 * is one of the heaviest strokes in the set (37 ink of a 15–48 band).
 *
 * Every candidate is drawn in the house grammar — hollow outlines on the
 * 24-unit grid, `crispEdges`, no solid mass — and measured the same way the
 * shipped set was: rasterised and sampled one point per coarse cell, the cell
 * being what `size={14}` actually paints.
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
    key: "brackets",
    name: "Empty range",
    paths: ["M5 5h6v2H7v10h4v2H5Z", "M19 5h-6v2h4v10h-4v2h6Z"],
    says: "a span with nothing in it",
    note: "Echoes the bracket glyphs of the mono face the row already sets its dates in, and pairs with the range printed beside it. Nothing in the set uses facing brackets, so the topology is free.",
  },
  {
    key: "span",
    name: "Measured span",
    paths: ["M2 8h2v8H2Z", "M20 8h2v8h-2Z", "M4 10h16v4H4Z"],
    says: "this much time, and nothing in it",
    note: "A dimension line: two end ticks and an unbroken run between them. Says the window explicitly, which is the one thing that separates this state from the other two empty rails.",
  },
  {
    key: "window",
    name: "Window",
    paths: ["M3 6h18v2H5v10h14v2H3Z", "M19 8h2v8h-2Z"],
    says: "this window, held open",
    note: "A frame open on no side. Heaviest of the three; the risk is reading as folder or file, both rectangles in this set — check the neighbour strip below.",
  },
  {
    key: "rail-gap-heavy",
    name: "Rail, no node",
    paths: ["M1 9h7v6H1Z", "M16 9h7v6h-7Z"],
    says: "the timeline runs on; nothing landed",
    note: "git-commit with its node lifted out, thickened until it carries enough ink to hold. Reads as a pair with the sibling state — which is either the point or a confusion.",
  },
];

/* One point per coarse cell of the 12×12 grid the marks are authored on —
   the same sampling the shipped set was measured with. */
function measure(paths: string[]): { ink: number; blobs: number } {
  const canvas = document.createElement("canvas");
  canvas.width = 24;
  canvas.height = 24;
  const ctx = canvas.getContext("2d")!;
  ctx.clearRect(0, 0, 24, 24);
  ctx.fillStyle = "#000";
  for (const d of paths) ctx.fill(new Path2D(d));
  const data = ctx.getImageData(0, 0, 24, 24).data;
  const cell = (cx: number, cy: number) => data[((cy * 2 + 1) * 24 + (cx * 2 + 1)) * 4 + 3] > 127;
  const grid: boolean[][] = [];
  let ink = 0;
  for (let y = 0; y < 12; y++) {
    grid[y] = [];
    for (let x = 0; x < 12; x++) {
      grid[y][x] = cell(x, y);
      if (grid[y][x]) ink++;
    }
  }
  // 8-connected blobs, so a mark that falls apart at size shows up as several.
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

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1080px; margin: 0 auto; padding: 24px 22px 70px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 18px; max-width: 82ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-bar { display: flex; gap: 8px; margin: 0 0 22px; }
  .mk-opt { margin: 0 0 18px; border: 1px solid var(--retro-frame-edge); border-radius: var(--retro-panel-radius); background: var(--bg-1); overflow: hidden; }
  .mk-opt > header { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;
    padding: 9px 14px; border-bottom: 1px solid var(--border); background: var(--surface-subtle); }
  .mk-opt > header h2 { margin: 0; color: var(--text-0); font-size: var(--type-md); font-weight: var(--weight-bold); }
  .mk-opt > header .says { color: var(--text-2); font-size: var(--type-sm); }
  .mk-ink { margin-left: auto; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-ink.out { color: var(--danger-text); }
  .mk-body { display: grid; grid-template-columns: 128px minmax(0, 1fr); gap: 16px; padding: 12px 14px 14px; align-items: start; }
  .mk-zoom { width: 120px; height: 120px; border: 1px solid var(--border); background: var(--bg-2); image-rendering: pixelated; }
  .mk-note { margin: 0 0 10px; color: var(--text-2); font-size: var(--type-xs); line-height: var(--leading-prose); }
  .mk-strip { display: flex; align-items: center; gap: 14px; margin: 0 0 10px; color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
  .mk-strip svg { color: var(--text-1); }
`;

function Row({ paths, title, meta }: { paths?: string[]; title: string; meta: string | null }) {
  return (
    <ol className="sync-history-commit-rail">
      <li className="sync-history-commit sync-history-commit-gap">
        <span className="sync-history-spine-fold inert" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
        <div className="sync-history-commit-heading">
          <span className="sync-history-commit-title">
            <span className="sync-history-commit-icon" aria-hidden="true">
              {paths
                ? (
                  <svg aria-hidden="true" fill="none" height={14} shapeRendering="crispEdges" viewBox="0 0 24 24" width={14}>
                    <g className="icon-pixel-fill" fill="currentColor" stroke="none">
                      {paths.map((d) => <path key={d} d={d} />)}
                    </g>
                  </svg>
                )
                : <Icon name="range" size={14} />}
            </span>
            <strong>{title}</strong>
          </span>
          {meta && <span className="sync-history-commit-meta"><time>{meta}</time></span>}
        </div>
      </li>
    </ol>
  );
}

function Zoom({ paths }: { paths?: string[] }) {
  return (
    <svg className="mk-zoom" viewBox="0 0 24 24" shapeRendering="crispEdges" aria-hidden="true">
      <g stroke="var(--border)" strokeWidth="0.08">
        {Array.from({ length: 13 }, (_, i) => (
          <g key={i}>
            <line x1={i * 2} y1="0" x2={i * 2} y2="24" />
            <line x1="0" y1={i * 2} x2="24" y2={i * 2} />
          </g>
        ))}
      </g>
      <g fill="var(--text-0)" stroke="none">
        {(paths ?? []).map((d) => <path key={d} d={d} />)}
      </g>
    </svg>
  );
}

function Mockup() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  const measured = useMemo(
    () => CANDIDATES.map((c) => ({ ...c, ...measure(c.paths) })),
    [],
  );
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Empty-rail marks</h1>
      <p className="mk-lede">
        <code>range</code> now ships for this rail. It replaced <code>clock</code>, which said <em>time</em>
        rather than <em>empty</em> and at 37 ink was among the heaviest strokes in a set whose band is
        15–48, mean 32. Each candidate is
        drawn in the house grammar — hollow, on the 24-unit grid, <code>crispEdges</code> — and measured
        the same way the shipped set was. Judge them at 14px in the row; the 10× plate is for shape only.
      </p>
      <div className="mk-bar">
        <button type="button" className="btn btn" onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
          {theme === "light" ? "Switch to dark" : "Switch to light"}
        </button>
      </div>

      <div className="mk-opt" data-shipped>
        <header>
          <h2>Shipped now — range (real Icon)</h2>
          <span className="says">says: a span with nothing in it</span>
          <span className="mk-ink">ink 22 · 2 blobs · in band</span>
        </header>
        <div className="mk-body">
          <Zoom paths={["M5 5h6v2H7v10h4v2H5Z", "M19 5h-6v2h4v10h-4v2h6Z"]} />
          <div>
            <p className="mk-note">Rendered through the real <code>Icon</code> component, at the real size.</p>
            <Row title="No commits" meta="Jul 31 — Aug 30, 2026" />
            <div className="mk-strip">
              <Icon name="range" size={14} />
              beside its neighbours:
              <Icon name="git-commit" size={14} />
              <Icon name="git-branch" size={14} />
              <Icon name="clock" size={14} />
              <Icon name="folder" size={14} />
              <Icon name="file" size={14} />
              <Icon name="catalog" size={14} />
            </div>
          </div>
        </div>
      </div>

      {measured.map((c) => {
        const out = c.ink < 20 || c.ink > 40;
        return (
          <div key={c.key} className="mk-opt">
            <header>
              <h2>{c.name}</h2>
              <span className="says">says: {c.says}</span>
              <span className={`mk-ink${out ? " out" : ""}`}>
                ink {c.ink} · {c.blobs} blob{c.blobs === 1 ? "" : "s"} · {out ? "outside 20–40" : "in band"}
              </span>
            </header>
            <div className="mk-body">
              <Zoom paths={c.paths} />
              <div>
                <p className="mk-note">{c.note}</p>
                <Row paths={c.paths} title="No commits" meta="Jul 31 — Aug 30, 2026" />
                <div className="mk-strip">
                  <svg aria-hidden="true" fill="none" height={14} shapeRendering="crispEdges" viewBox="0 0 24 24" width={14}>
                    <g className="icon-pixel-fill" fill="currentColor" stroke="none">
                      {c.paths.map((d) => <path key={d} d={d} />)}
                    </g>
                  </svg>
                  beside its neighbours:
                  <Icon name="git-commit" size={14} />
                  <Icon name="git-branch" size={14} />
                  <Icon name="clock" size={14} />
                  <Icon name="folder" size={14} />
                  <Icon name="catalog" size={14} />
                </div>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);
