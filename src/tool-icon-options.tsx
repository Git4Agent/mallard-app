import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * Candidate marks for the activity row's tool-call readout, against the shipped
 * skin. Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 *
 * Every candidate is authored on a **12-cell** grid and doubled onto the 24-cell
 * grid the icon set draws on. That is not a convenience — it is the constraint.
 * `Icons.tsx` keeps one uniform stem only when `size × dpr ÷ 24` is whole, and
 * every shipped mark uses 2-cell stems so a 12px icon on a Retina display comes
 * out one device pixel per stem. Authoring at 12 makes an odd stem impossible.
 */

/** Greedy rectangle cover of a bitmap, doubled onto the 24-grid, emitted in the
 *  same `M x yh w v h H x Z` runs the shipped paths use — so the winner can be
 *  pasted into `Icons.tsx` verbatim. */
function pathFromBitmap(rows: string[]): string {
  const grid = rows.map((row) => [...row].map((cell) => cell === "#"));
  const height = grid.length;
  const width = grid[0].length;
  const used = grid.map((row) => row.map(() => false));
  const parts: string[] = [];
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      if (!grid[y][x] || used[y][x]) continue;
      let w = 0;
      while (x + w < width && grid[y][x + w] && !used[y][x + w]) w += 1;
      let h = 1;
      grow: while (y + h < height) {
        for (let i = 0; i < w; i += 1) {
          if (!grid[y + h][x + i] || used[y + h][x + i]) break grow;
        }
        h += 1;
      }
      for (let dy = 0; dy < h; dy += 1) {
        for (let dx = 0; dx < w; dx += 1) used[y + dy][x + dx] = true;
      }
      parts.push(`M${x * 2} ${y * 2}h${w * 2}v${h * 2}H${x * 2}Z`);
    }
  }
  return parts.join("");
}

/** The crossed hammer-and-wrench `tool` shipped with before the wrench cutover,
 *  verbatim — kept here so the comparison that decided it still renders. */
const RETIRED_TOOL_PATH = "M19 11v-1h1V9h1V8h1V7h1V4h-1V3h-1V2h-1V1h-3v1h-1v1h-1v1h-1v1h-1v1h-2V5h-1V4H9V3H8V2H7V1H5v1H4v1H3v1H2v1H1v2h1v1h1v1h1v1h1v1h1v2H5v1H4v1H3v1H2v1H1v6h6v-1h1v-1h1v-1h1v-1h1v-1h2v1h1v1h1v1h1v1h1v1h2v-1h1v-1h1v-1h1v-1h1v-2h-1v-1h-1v-1h-1v-1h-1v-1h-1v-2h1ZM7 10V9H6V8H5V7H4V5h1V4h2v1h1v1H7v1h1v1h1V7h1v2H9v1H7Zm6 3v1h-1v1h-1v1h-1v1H9v1H8v1H7v1H6v1H3v-3h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1v-1h1V9h1V8h1V7h1v1h1v1h1v1h-1v1h-1v1h-1v1h-1Zm6 3v1h1v2h-1v1h-2v-1h-1v-1h-1v-1h-1v-2h1v-1h2v1h-1v1h1v1h1v-1h1Zm-2-9V6h-1V5h1V4h1V3h1v1h1v1h1v1h-1v1h-1v1h-1V7h-1Z";

function RetiredIcon({ size }: { size: number }) {
  return (
    <svg aria-hidden="true" fill="none" height={size} shapeRendering="crispEdges" viewBox="0 0 24 24" width={size}>
      <path className="icon-pixel-fill" fill="currentColor" stroke="none" d={RETIRED_TOOL_PATH} />
    </svg>
  );
}

interface Candidate {
  id: string;
  name: string;
  blurb: string;
  bitmap: string[];
}

const CANDIDATES: Candidate[] = [
  {
    id: "wrench",
    name: "A · Wrench",
    blurb: "one tool instead of two crossed ones — shipped briefly, then swapped for B",
    bitmap: [
      "............",
      "...##..##...",
      "...##..##...",
      "...######...",
      "...######...",
      ".....##.....",
      ".....##.....",
      ".....##.....",
      ".....##.....",
      ".....##.....",
      ".....##.....",
      "............",
    ],
  },
  {
    id: "cog",
    name: "B · Cog",
    blurb: "the round silhouette in a row of square ones — chosen, ships as `tool`",
    bitmap: [
      "............",
      ".....##.....",
      ".....##.....",
      "...######...",
      "...######...",
      ".####..####.",
      ".####..####.",
      "...######...",
      "...######...",
      ".....##.....",
      ".....##.....",
      "............",
    ],
  },
  {
    id: "bolt",
    name: "C · Bolt",
    blurb: "fewest cells of any candidate — legible at 12px with room to spare",
    bitmap: [
      "............",
      "......##....",
      ".....##.....",
      "....##......",
      "...#####....",
      "......##....",
      ".....##.....",
      "....##......",
      "...##.......",
      "............",
      "............",
      "............",
    ],
  },
  {
    id: "call",
    name: "D · Call",
    blurb: "brackets round a dot — a call, not a hammer; the literal reading",
    bitmap: [
      "............",
      "............",
      "..###..###..",
      "..##....##..",
      "..##....##..",
      "..##.##.##..",
      "..##.##.##..",
      "..##....##..",
      "..##....##..",
      "..###..###..",
      "............",
      "............",
    ],
  },
];

function PixelIcon({ bitmap, size }: { bitmap: string[]; size: number }) {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height={size}
      shapeRendering="crispEdges"
      viewBox="0 0 24 24"
      width={size}
    >
      <path className="icon-pixel-fill" fill="currentColor" stroke="none" d={pathFromBitmap(bitmap)} />
    </svg>
  );
}

/** The shipped row, with only the tool glyph swapped. Everything else — the
 *  classes, the sizes, the neighbours — is what `SessionMetrics` renders. */
function MetricRow({ candidate }: { candidate: Candidate | null }) {
  return (
    <div className="sync-history-thread-meta" aria-label="Session details">
      <span className="sync-history-thread-metric" tabIndex={0}>
        <Icon name="message" size={12} /><b>13</b>
      </span>
      <span className="sync-history-thread-metric" tabIndex={0}>
        {candidate ? <PixelIcon bitmap={candidate.bitmap} size={12} /> : <RetiredIcon size={12} />}
        <b>10</b>
      </span>
      <span className="sync-history-thread-metric" tabIndex={0}>
        <Icon name="token" size={12} /><b>340K</b>
      </span>
      <span className="sync-history-thread-metric sync-history-thread-duration" tabIndex={0}>
        <Icon name="clock" size={12} /><b>3m</b>
      </span>
    </div>
  );
}

function Specimen() {
  const [theme, setTheme] = useState<"dark" | "light">(
    new URLSearchParams(window.location.search).get("theme") === "light" ? "light" : "dark",
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  return (
    <div className="specimen-page">
      <header className="specimen-head">
        <div>
          <h1>A mark for the tool call</h1>
          <p>
            Four candidates for the row’s third readout. Each is drawn on the
            12-cell grid and doubled, so every stem is two cells — the rule that
            keeps a 12px icon one device pixel per stem on a Retina display.
          </p>
        </div>
        <div className="specimen-controls">
          <label>
            <input type="checkbox" checked={theme === "light"} onChange={(e) => setTheme(e.target.checked ? "light" : "dark")} />
            {" "}light
          </label>
        </div>
      </header>

      <section>
        <h2>1 · In the row, at 12px</h2>
        <p className="specimen-lead">
          The size it actually ships at, beside the neighbours it has to hold its
          own against. Judge it here, not in the big view below.
        </p>
        <div className="specimen-rows">
          <div className="specimen-row">
            <div className="specimen-row-label"><b>Retired</b><span>the crossed hammer and wrench; B shipped in its place</span></div>
            <div className="specimen-surface"><MetricRow candidate={null} /></div>
          </div>
          {CANDIDATES.map((candidate) => (
            <div className="specimen-row" key={candidate.id}>
              <div className="specimen-row-label"><b>{candidate.name}</b><span>{candidate.blurb}</span></div>
              <div className="specimen-surface"><MetricRow candidate={candidate} /></div>
            </div>
          ))}
        </div>
      </section>

      <section>
        <h2>2 · The shape</h2>
        <p className="specimen-lead">
          12px, 24px, and blown up so the cell grid is visible. A candidate that
          only works at 96 is not a candidate.
        </p>
        <div className="specimen-shapes">
          <div className="specimen-shape">
            <div className="specimen-shape-marks">
              <RetiredIcon size={12} /><RetiredIcon size={24} />
              <span className="specimen-big"><RetiredIcon size={96} /></span>
            </div>
            <b>Retired</b>
          </div>
          {CANDIDATES.map((candidate) => (
            <div className="specimen-shape" key={candidate.id}>
              <div className="specimen-shape-marks">
                <PixelIcon bitmap={candidate.bitmap} size={12} />
                <PixelIcon bitmap={candidate.bitmap} size={24} />
                <span className="specimen-big"><PixelIcon bitmap={candidate.bitmap} size={96} /></span>
              </div>
              <b>{candidate.name}</b>
            </div>
          ))}
        </div>
      </section>

      <section>
        <h2>3 · The paths</h2>
        <p className="specimen-lead">Ready to paste into <code>Icons.tsx</code>.</p>
        <div className="specimen-paths">
          {CANDIDATES.map((candidate) => (
            <div key={candidate.id}>
              <b>{candidate.name}</b>
              <code>{pathFromBitmap(candidate.bitmap)}</code>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
