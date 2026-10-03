import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "../src/App.css";

/**
 * Diagonal plug, redrawn.
 *
 * The earlier sweep failed on proportion, not resolution: a big body with short
 * prongs turns into a diamond wearing two nubs, and a diamond is not a plug at
 * any size. The reference has the opposite proportion — a compact body, long
 * prongs, a long lead — so the whole mark is a long thing lying on the diagonal
 * and the prongs are what you read first.
 *
 * Written as ASCII so the shape is legible in the source and tunable a cell at
 * a time. Judge the 14px plate; the rest is for shape.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

interface Study { key: string; name: string; note: string; rows: string[] }

const STUDIES: Study[] = [
  {
    key: "i1",
    name: "I · as drawn",
    note: "Axis-aligned body, diagonal lead. The body's edges land on the pixel grid, which is the whole reason it survives 14px where a tilted body does not.",
    rows: [
      "...#..#.....",
      "...#..#.....",
      "..######....",
      "..######....",
      "..#....#....",
      "..######....",
      "....##......",
      ".....#......",
      "......#.....",
      ".......#....",
      "........#...",
      "............",
    ],
  },
  {
    key: "i2",
    name: "I2 · hollow body",
    note: "The same shape drawn as an outline rather than a mass, which is the set's grammar — folder, file, message and clock are all one-cell outlines.",
    rows: [
      "...#..#.....",
      "...#..#.....",
      "..######....",
      "..#....#....",
      "..#....#....",
      "..######....",
      "....##......",
      ".....#......",
      "......#.....",
      ".......#....",
      "........#...",
      "............",
    ],
  },
  {
    key: "i3",
    name: "I3 · longer lead, tighter body",
    note: "Body pulled in a cell each way to buy lead. The longer diagonal is what carries the tilt, so lengthening it is the cheapest way to read as diagonal.",
    rows: [
      "...#..#.....",
      "...#..#.....",
      "..######....",
      "..#....#....",
      "..######....",
      "....#.......",
      ".....#......",
      "......#.....",
      ".......#....",
      "........#...",
      ".........#..",
      "............",
    ],
  },
  {
    key: "i4",
    name: "I4 · two-cell lead",
    note: "Lead thickened so it holds its own against the body instead of fading to grey at 14px — the failure that killed every fully-diagonal attempt.",
    rows: [
      "..#..#......",
      "..#..#......",
      ".######.....",
      ".#....#.....",
      ".######.....",
      "...##.......",
      "....##......",
      ".....##.....",
      "......##....",
      ".......##...",
      "........##..",
      "............",
    ],
  },
];

function pathsOf(rows: string[]): string[] {
  const paths: string[] = [];
  rows.forEach((row, y) => {
    let x = 0;
    while (x < 12) {
      if (row[x] === "#") {
        let w = 0;
        while (row[x + w] === "#") w += 1;
        paths.push(`M${x * 2} ${y * 2}h${w * 2}v2h-${w * 2}Z`);
        x += w;
      } else x += 1;
    }
  });
  return paths;
}
const inkOf = (rows: string[]) => rows.join("").split("#").length - 1;

const SHELL = `
  html, body, #root { height: auto; min-height: 100%; overflow: visible; }
  body { background: var(--retro-frame-gutter); }
  .mk { max-width: 1120px; margin: 0 auto; padding: 20px; font-family: var(--font-sans); }
  .mk-title { margin: 0 0 4px; color: var(--text-0); font-size: var(--type-xl); font-weight: var(--weight-bold); }
  .mk-lede { margin: 0 0 16px; max-width: 84ch; color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-prose); }
  .mk-sheet { display: grid; grid-template-columns: repeat(4, 1fr); gap: 14px; }
  .mk-cell { border: 1px solid var(--retro-frame-edge); border-radius: 8px; background: var(--bg-1); padding: 10px; }
  .mk-cell h2 { margin: 0 0 6px; color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-bold); }
  .mk-cell p { margin: 6px 0 0; color: var(--text-2); font-size: var(--type-2xs); line-height: var(--leading-prose); }
  .mk-plates { display: flex; gap: 8px; align-items: flex-start; }
  .mk-plates canvas { image-rendering: pixelated; color: var(--text-0);
    border: 1px solid var(--border); background: var(--bg-2); }
  .mk-ink { color: var(--text-3); font-family: var(--font-mono); font-size: 10px; }
`;

function Plate({ paths, px, box }: { paths: string[]; px: number; box: number }) {
  const ref = (node: HTMLCanvasElement | null) => {
    if (!node) return;
    const small = document.createElement("canvas");
    small.width = px;
    small.height = px;
    const sctx = small.getContext("2d")!;
    sctx.scale(px / 24, px / 24);
    sctx.fillStyle = getComputedStyle(node).color;
    for (const d of paths) sctx.fill(new Path2D(d));
    const ctx = node.getContext("2d")!;
    ctx.clearRect(0, 0, box, box);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(small, 0, 0, box, box);
  };
  return <canvas ref={ref} width={box} height={box} style={{ width: box, height: box }} />;
}

function Mockup() {
  const [theme, setTheme] = useState<"light" | "dark">("light");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  return (
    <div className="mk">
      <style>{SHELL}</style>
      <h1 className="mk-title">Diagonal plug — redrawn</h1>
      <p className="mk-lede">
        The last attempt had the proportion backwards: a big body with short prongs is a diamond wearing
        nubs. These give the prongs and the lead the length, and keep the body small. Left plate is the
        shape at 24; right plate is 14px — the size it ships at — blown up with smoothing off.
      </p>
      <button type="button" className="btn btn" style={{ marginBottom: 14 }}
        onClick={() => setTheme(theme === "light" ? "dark" : "light")}>
        {theme === "light" ? "Switch to dark" : "Switch to light"}
      </button>
      <div className="mk-sheet">
        {STUDIES.map((s) => {
          const paths = pathsOf(s.rows);
          return (
            <div key={s.key} className="mk-cell">
              <h2>{s.name}</h2>
              <div className="mk-plates">
                <Plate paths={paths} px={24} box={96} />
                <Plate paths={paths} px={14} box={56} />
              </div>
              <span className="mk-ink">ink {inkOf(s.rows)}</span>
              <p>{s.note}</p>
            </div>
          );
        })}
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Mockup /></StrictMode>);
