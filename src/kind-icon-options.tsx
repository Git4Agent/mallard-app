import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import "./App.css";
import "./RetroTheme.css";

/**
 * Candidate marks for the Library's three resource kinds — Skill, Plugin, Pet —
 * against the shipped skin. Not part of the app bundle: the build has one entry,
 * and this file is reachable only from the dev server.
 *
 * Every candidate is authored on a **12-cell** grid and doubled onto the 24-cell
 * grid the icon set draws on. That is not a convenience — it is the constraint.
 * `Icons.tsx` keeps one uniform stem only when `size x dpr / 24` is whole, and
 * authoring at 12 makes an odd stem impossible.
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

interface Candidate {
  id: string;
  name: string;
  blurb: string;
  /** Where the metaphor is already standard. The point of the exercise. */
  cite: string;
  /** Whether the twelve-cell grid takes the metaphor without a fight. */
  verdict: "ok" | "risk";
  /** What the measurement below means for this shape, in one sentence. */
  note: string;
  bitmap: string[];
}

interface Concept {
  id: string;
  /** The candidate id that shipped. The specimen is the record of the choice,
   *  so it names the winner rather than leaving three open options. */
  shippedId: string;
  title: string;
  lead: string;
  /** What ships today, and why it is being replaced. */
  shipped: { icon: IconName; why: string };
  /** The section heading / sidebar / repo-row label this kind wears. */
  plural: string;
  candidates: Candidate[];
}

const CONCEPTS: Concept[] = [
  {
    id: "skill",
    shippedId: "bulb",
    title: "Skill",
    plural: "Skills",
    lead:
      "An agent_skill: a named, reusable instruction bundle Claude or Codex loads to do a task. " +
      "No icon system anywhere has a mark for this \u2014 zero hits for \u201cskill\u201d or \u201ccapability\u201d across Material Symbols (3,899), " +
      "SF Symbols (8,302), Codicons (649), Octicons (743), Fluent (19k) and every set on Iconify. Whatever ships here is greenfield.",
    shipped: {
      icon: "file",
      why:
        "`file` is the generic document mark. It says where a skill is kept, not what it is, and it is the same glyph " +
        "the project's resource rows already spend on every other document.",
    },
    candidates: [
      {
        id: "bulb",
        name: "A \u00b7 Lightbulb",
        blurb:
          "The only glyph any shipped developer tool has ever bound to a skill. Nothing else in this row is a dome over a stem.",
        cite:
          "Material Icon Theme \u2014 the most-installed VS Code file-icon theme \u2014 binds `skill.md` and `folder-skills` to an amber bulb \u00b7 Material `lightbulb` (11,807 uses) \u00b7 SF, Lucide, Phosphor and Bootstrap all ship one",
        verdict: "ok",
        note: "Lightest mark of the nine and the least like anything already in the row.",
        bitmap: [
          "............",
          "....####....",
          "...#....#...",
          "..#......#..",
          "..#......#..",
          "..#......#..",
          "...#....#...",
          "....####....",
          "....####....",
          ".....##.....",
          "............",
          "............",
        ],
      },
      {
        id: "blocks",
        name: "B \u00b7 Modules",
        blurb:
          "One block with a second offset off its corner: a loadable, composable unit rather than a document. Two blocks, deliberately \u2014 four in a square is `catalog`.",
        cite:
          "Lucide `blocks` and Tabler `blocks` \u00b7 Material `sdk`, whose tag list (add-on, module, kit, building block, integration) describes an agent_skill more exactly than any drawing in the set does",
        verdict: "ok",
        note: "Two hollow rectangles, diagonally offset. Reads as structure, not as a container.",
        bitmap: [
          "............",
          ".......#####",
          ".......#...#",
          ".......#...#",
          ".......#####",
          "............",
          "#######.....",
          "#.....#.....",
          "#.....#.....",
          "#.....#.....",
          "#######.....",
          "............",
        ],
      },
      {
        id: "sparkle",
        name: "C \u00b7 Sparkle",
        blurb:
          "By far the most established metaphor \u2014 the mark every assistant surface now spends on \u201ca capability the model can invoke\u201d. Off-centre on purpose: a centred star lands on the cog.",
        cite:
          "Google standardised an AI sparkle across its products in 2023 (~100 variants; it fronts Gemini) \u00b7 Material `auto_awesome` / `wand_stars` \u00b7 SF `sparkle` \u00b7 Codicon `sparkle` and eleven compounds \u00b7 Lucide, Phosphor, Tabler",
        verdict: "risk",
        note:
          "The collision is family, not pixels: `claude` is a radiating burst and `ProviderMark` paints it on these very rows, so a star beside it reads as provider branding rather than as a kind.",
        bitmap: [
          "............",
          ".........#..",
          "........###.",
          ".........#..",
          "...##.......",
          "...##.......",
          "..####......",
          "########....",
          "..####......",
          "...##.......",
          "...##.......",
          "............",
        ],
      },
    ],
  },
  {
    id: "plugin",
    shippedId: "puzzle",
    title: "Plugin",
    plural: "Plugins",
    lead:
      "A plugin_install_intent: an installable extension package. The one concept of the three with settled marks \u2014 and the " +
      "settled one does not fit. Three vendor lineages: the jigsaw piece (Chrome, Material, Fluent, Apple), the four-square " +
      "module grid (VS Code, Office add-ins) and the two-pin plug (WordPress, JetBrains, Bootstrap).",
    shipped: {
      icon: "tool",
      why:
        "`tool` is the cog this app already spends on a tool call, in the activity row and in the session detail. Two meanings, one glyph.",
    },
    candidates: [
      {
        id: "plug",
        name: "A \u00b7 Plug",
        blurb:
          "Two prongs rising off a body. The most distinct silhouette of everything drawn here, and the lineage is this app's own register: IDEs, not browsers.",
        cite:
          "JetBrains ships a plug as a plugin's default identity (`expui/nodes/pluginLogo.svg`, the 40\u00d740 fallback) \u00b7 WordPress `dashicons-admin-plugins`, fifteen years and the one vendor whose word and drawing agree \u00b7 Bootstrap's glyph literally named `plugin` is a plug \u00b7 Codicon `plug` \u00b7 SF `powerplug` \u00b7 Material `power`",
        verdict: "ok",
        note: "Nothing else in the row forks upward. Prongs are two cells with a two-cell gap \u2014 no feature below the floor.",
        bitmap: [
          "............",
          "...##..##...",
          "...##..##...",
          "..########..",
          "..#......#..",
          "..#......#..",
          "..#......#..",
          "..#......#..",
          "...######...",
          "....####....",
          ".....##.....",
          "............",
        ],
      },
      {
        id: "chip",
        name: "B \u00b7 Chip",
        blurb:
          "A module with legs \u2014 the thing you seat in a socket. Wider than it is tall, which is a proportion nothing else here has.",
        cite:
          "Material `memory` \u00b7 SF `memorychip` \u00b7 Codicon `chip` \u00b7 Lucide `cpu` \u00b7 Phosphor `circuitry` \u00b7 Tabler and Bootstrap `cpu`",
        verdict: "ok",
        note: "Reads hardware, and the row already has `drive` and `computer` in that register. Silhouette is clean; the meaning is one step literal.",
        bitmap: [
          "............",
          "...#..#..#..",
          "..########..",
          "..#......#..",
          "###......###",
          "..#......#..",
          "###......###",
          "..#......#..",
          "..########..",
          "...#..#..#..",
          "............",
          "............",
        ],
      },
      {
        id: "puzzle",
        name: "C \u00b7 Puzzle piece",
        blurb:
          "The industry default, in the reduced form Apple ships: one socket bitten out of the left edge, one knob on the right, top and bottom straight.",
        cite:
          "Chrome's toolbar extensions button \u00b7 Material `extension`, the only icon Google tags `plugin` \u00b7 Apple ships `puzzlepiece.extension.fill` in the Extensions and System Extensions panes \u2014 the only SF name containing \u201cextension\u201d \u00b7 Fluent's `extension` is literally an alias of `puzzle-piece`",
        verdict: "risk",
        note:
          "Shown because it is what the whole industry agreed on, and because you should see it fail rather than take my word. A jigsaw knob is about a fifth of the icon's width, which halves to two and a half cells here and snaps to a bump; the metaphor needs two tabs and a socket, and that wants a sixteen-cell grid. Apple already reduced it once for the same reason. Judge it in the 14px rows.",
        bitmap: [
          "............",
          ".########...",
          ".#......#...",
          ".#......#...",
          ".###....###.",
          "...#......#.",
          "...#......#.",
          ".###....###.",
          ".#......#...",
          ".#......#...",
          ".########...",
          "............",
        ],
      },
    ],
  },
  {
    id: "pet",
    shippedId: "paw",
    title: "Pet",
    plural: "Pets v2",
    lead:
      "A codex_pet: an animated companion that lives in the agent's UI. No icon set anywhere has an abstract mark for one \u2014 " +
      "`mascot`, `virtual-pet` and `screenmate` return nothing across ~200 sets, and every shipped desktop pet (Neko, Shimeji, " +
      "Bongo Cat, Desktop Goose) is identified by its own idle frame. This app already does that on the card, which changes the " +
      "job: this glyph should name the kind in the heading and the sidebar without competing with the sprite two inches away.",
    shipped: {
      icon: "openai",
      why:
        "The OpenAI mark names the *provider*, not the kind \u2014 and `ProviderMark` already paints it on the same rows. " +
        "As a kind glyph it carries no information at all, in three places at once: the section heading, the sidebar row, and the empty pet tile.",
    },
    candidates: [
      {
        id: "ghost",
        name: "A \u00b7 Creature",
        blurb:
          "A dome, two eyes, a scalloped hem. The only candidate native to the pixel idiom rather than translated into it \u2014 and the only serrated bottom edge in the row.",
        cite:
          "Pixelarticons ships `ghost` \u00b7 Lucide, Phosphor and Tabler all ship one with near-identical geometry \u00b7 HackerNoon's set has six `face-*` glyphs and no animals at all \u2014 both pixel sets fall back to faces precisely because animals do not fit",
        verdict: "ok",
        note: "Two eyes in a hollow silhouette is the most forgiving creature form at this size. Its risk is semantic, not structural: it depicts a particular creature, and the sprite beside it is a different one.",
        bitmap: [
          "............",
          "....####....",
          "..##....##..",
          ".#........#.",
          ".#.##..##.#.",
          ".#.##..##.#.",
          ".#........#.",
          ".#........#.",
          ".##########.",
          ".##..##..##.",
          "............",
          "............",
        ],
      },
      {
        id: "egg",
        name: "B \u00b7 Egg",
        blurb:
          "The one mark that names the kind without drawing a creature \u2014 so it never argues with the sprite next to it. Narrow crown over a round base; nothing else in the set is that shape.",
        cite:
          "Four of five OSS sets ship an egg with near-identical geometry \u00b7 Material `egg` \u00b7 the Tamagotchi shell and its on-screen hatch are why \u201cegg\u201d reads as a virtual pet at all",
        verdict: "ok",
        note: "Lightest mark here and the most legible small shape measured \u2014 it needs no interior detail to hold. Semantics lean \u201cnot yet hatched\u201d, which is exactly right for the empty tile and arguable for a stored pet.",
        bitmap: [
          "............",
          ".....##.....",
          "....#..#....",
          "...#....#...",
          "..#......#..",
          "..#......#..",
          "..#......#..",
          "..#......#..",
          "...#....#...",
          "....####....",
          "............",
          "............",
        ],
      },
      {
        id: "paw",
        name: "C \u00b7 Paw print",
        blurb:
          "The category mark \u2014 what \u201cpet\u201d means in every general-purpose set. Four toes on an arc over a pad.",
        cite:
          "Material `pets`, literally the name \u00b7 SF `pawprint`, one of exactly two symbols Apple tags `pet friendly` \u00b7 Lucide, Phosphor, Tabler and Fluent all ship `paw-print`",
        verdict: "risk",
        note:
          "The most established metaphor, and the one the grid argues with hardest. A paw is five elements with four gaps; no pixel-art set on Iconify ships one, and on an arc the toes want to touch at the corners and merge. The outer pair drops two rows rather than one for exactly that reason \u2014 at one row they meet the inner toes at the corner and fuse. Two rows is Material's own proportion for `pets`. Drawn natively at twelve rather than shrunk from twenty-four, which is its best case, so this row is the fair test.",
        bitmap: [
          "............",
          "...##..##...",
          "...##..##...",
          "............",
          ".##......##.",
          ".##......##.",
          "....####....",
          "..########..",
          "..########..",
          "...######...",
          "............",
          "............",
        ],
      },
    ],
  },
];

/** The marks a new glyph has to be found *against*: what already sits in the
 *  Library's headings, the sidebar rows and the repository list. */
const NEIGHBOURS: IconName[] = [
  "catalog", "file", "folder", "tool", "plus", "drive", "cloud", "git-fork",
  "badge-check", "settings", "user", "message", "clock", "activity",
];

/**
 * The three numbers the craft literature says decide whether a twelve-cell mark
 * survives, measured off the bitmap rather than asserted:
 *
 * - **ink** — filled cells. Counter-intuitively this is *anti*-findability: at
 *   twelve cells every solid convex blob converges on the same mask, so the
 *   heavy marks are the confusable ones and the sparse ones stand out.
 * - **blobs** — 8-connected components. A detached blob under four cells reads
 *   as a stray pixel, which is what makes a shrunk paw print look broken.
 * - **holes** — enclosed 4-connected gaps. A hole under four cells vanishes,
 *   which is why a dog's eyes and a magnifier's lens do not survive.
 *
 * The budget the measurements are read against: one silhouette plus at most two
 * interior events of four cells or more, or one silhouette plus three or four
 * detached marks of four cells or more. Not both, and not more.
 */
function measure(rows: string[]) {
  const H = rows.length;
  const W = rows[0].length;
  const at = (x: number, y: number) => y >= 0 && y < H && x >= 0 && x < W && rows[y][x] === "#";
  let ink = 0;
  for (let y = 0; y < H; y += 1) for (let x = 0; x < W; x += 1) if (at(x, y)) ink += 1;

  const flood = (want: boolean, diagonal: boolean) => {
    const seen = rows.map((r) => [...r].map(() => false));
    const sizes: number[] = [];
    const steps = diagonal
      ? [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [1, -1], [-1, 1], [-1, -1]]
      : [[1, 0], [-1, 0], [0, 1], [0, -1]];
    for (let y = 0; y < H; y += 1) {
      for (let x = 0; x < W; x += 1) {
        if (seen[y][x] || at(x, y) !== want) continue;
        let size = 0;
        let touchesEdge = false;
        const stack = [[x, y]];
        seen[y][x] = true;
        while (stack.length) {
          const [cx, cy] = stack.pop()!;
          size += 1;
          if (cx === 0 || cy === 0 || cx === W - 1 || cy === H - 1) touchesEdge = true;
          for (const [dx, dy] of steps) {
            const nx = cx + dx;
            const ny = cy + dy;
            if (nx < 0 || ny < 0 || nx >= W || ny >= H) continue;
            if (seen[ny][nx] || at(nx, ny) !== want) continue;
            seen[ny][nx] = true;
            stack.push([nx, ny]);
          }
        }
        if (want || !touchesEdge) sizes.push(size);
      }
    }
    return sizes.sort((a, b) => b - a);
  };

  return { ink, blobs: flood(true, true), holes: flood(false, false) };
}

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

/** Either a candidate bitmap or a shipped `Icons.tsx` name, at one size. */
function Mark({ candidate, shipped, size }: { candidate?: string[]; shipped?: IconName; size: number }) {
  if (candidate) return <PixelIcon bitmap={candidate} size={size} />;
  return <Icon name={shipped!} size={size} />;
}

/* ---- the three places the kind glyph is actually painted ---- */

/** `StorageCatalogPage` — the list's own section heading, at 14px. */
function HeadingContext({ concept, mark }: { concept: Concept; mark: React.ReactNode }) {
  return (
    <div className="sync-history-section-heading">
      <h2>{mark}{concept.plural}</h2>
    </div>
  );
}

/** `ProjectSidebar` — the Catalog pane's scope rows, at 16px. The busiest
 *  surface: several of these stack, and only the glyph tells them apart. */
function SidebarContext({ rows }: { rows: Array<{ mark: React.ReactNode; label: string; active?: boolean }> }) {
  return (
    <div className="sidebar-profile-list" style={{ padding: 0 }}>
      {rows.map((row, i) => (
        <div key={i} className={`sidebar-profile-item${row.active ? " active" : ""}`}>
          <button type="button" className="sidebar-profile-main">
            {row.mark}
            <span>{row.label}</span>
          </button>
          <div className="sidebar-profile-actions">
            <button type="button" className="sidebar-profile-remove" title="Remove">
              <Icon name="trash" size={13} />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}

/** `ProjectLinksWorkspace` — a Catalog repository in the storage listing, 14px
 *  inside its own tile. */
function RepoRowContext({ mark, name }: { mark: React.ReactNode; name: string }) {
  return (
    <div className="sync-storage-repository-item">
      <div className="sync-storage-repository-row">
        <span className="sync-storage-repository-icon">{mark}</span>
        <strong className="sync-storage-repository-name">{name}</strong>
        <span className="sync-storage-repository-chips">
          <span className="sync-storage-repo-chip"><span>Revision</span><b>4</b></span>
          <span className="sync-storage-repo-chip"><span>Updated</span><b>2h ago</b></span>
        </span>
      </div>
    </div>
  );
}

/** `StorageCatalogPage` — the pet card's tile when the row has no sprite yet.
 *  Pet only: 16px, letterboxed in the 44x48 well. */
function PetCardContext({ mark, name }: { mark: React.ReactNode; name: string }) {
  return (
    <article className="sync-history-thread-card catalog-card catalog-card-pet">
      <span className="catalog-pet-preview">{mark}</span>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title"><strong>{name}</strong></div>
      </div>
      <div className="sync-history-thread-meta">
        <span className="sync-history-thread-metric"><Icon name="clock" size={12} /><b>2h ago</b></span>
      </div>
    </article>
  );
}

/**
 * Ink of a *shipped* mark, measured rather than eyeballed: fill its own path on
 * the 24-unit grid and sample the odd coordinates — which is one sample per
 * coarse cell, and exactly what `size={12}` paints on a 2× display.
 *
 * The point is the band. A new mark that sits far outside the range its
 * neighbours occupy will read as heavier or lighter than the row, however good
 * the shape is on its own.
 */
function measureShipped(svg: SVGSVGElement): number {
  const canvas = document.createElement("canvas");
  canvas.width = 24;
  canvas.height = 24;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return 0;
  ctx.fillStyle = "#fff";
  svg.querySelectorAll("path").forEach((path) => {
    const d = path.getAttribute("d");
    if (d) ctx.fill(new Path2D(d));
  });
  const { data } = ctx.getImageData(0, 0, 24, 24);
  let ink = 0;
  for (let y = 1; y < 24; y += 2) {
    for (let x = 1; x < 24; x += 2) {
      if (data[(y * 24 + x) * 4 + 3] > 127) ink += 1;
    }
  }
  return ink;
}

/** The row's ink band, measured off the shipped marks on mount. */
function useNeighbourInk() {
  const [ink, setInk] = useState<Array<[IconName, number]>>([]);
  useEffect(() => {
    const host = document.getElementById("ink-probe");
    if (!host) return;
    setInk(
      NEIGHBOURS.map((name) => {
        const svg = host.querySelector<SVGSVGElement>(`svg[data-icon="${name}"]`);
        return [name, svg ? measureShipped(svg) : 0] as [IconName, number];
      }).sort((a, b) => a[1] - b[1]),
    );
  }, []);
  return ink;
}

/** The measured numbers, and the one-line reading of them. */
function Measured({ candidate }: { candidate: Candidate }) {
  const m = measure(candidate.bitmap);
  const runt = [...m.blobs, ...m.holes].some((n) => n < 4);
  return (
    <div className="specimen-measure">
      <div className="specimen-measure-row">
        <span className={`specimen-verdict is-${candidate.verdict}`}>
          {candidate.verdict === "ok" ? "grid takes it" : "grid argues"}
        </span>
        <span>ink <b>{m.ink}</b>/144</span>
        <span>blobs <b>{m.blobs.join("\u00b7")}</b></span>
        <span>holes <b>{m.holes.length ? m.holes.join("\u00b7") : "\u2014"}</b></span>
      </div>
      <p>
        {candidate.note}
        {runt && " One feature here is under four cells, which is the size at which a blob or a hole stops resolving."}
      </p>
    </div>
  );
}

function CandidateCard({ concept, candidate }: { concept: Concept; candidate: Candidate | null }) {
  const bitmap = candidate?.bitmap;
  const shipped = concept.shipped.icon;
  const mark = (size: number) => <Mark candidate={bitmap} shipped={shipped} size={size} />;
  return (
    <div className={`specimen-card${candidate ? (candidate.id === concept.shippedId ? " is-chosen" : "") : " is-current"}`}>
      <div className="specimen-card-head">
        <b>{candidate ? candidate.name : `Was · ${shipped}`}</b>
        <em>{candidate ? (candidate.id === concept.shippedId ? "chosen — shipped" : "candidate") : "replaced"}</em>
      </div>
      <p className="specimen-card-blurb">{candidate ? candidate.blurb : concept.shipped.why}</p>
      {candidate && <p className="specimen-card-cite">{candidate.cite}</p>}
      {candidate && <Measured candidate={candidate} />}
      <div className="specimen-shape-marks">
        {mark(12)}{mark(14)}{mark(16)}{mark(24)}
        <span className="specimen-big">{mark(72)}</span>
      </div>
      <div>
        <span className="specimen-surface-label">Section heading · 14px</span>
        <div className="specimen-surface">
          <HeadingContext concept={concept} mark={mark(14)} />
        </div>
      </div>
      <div>
        <span className="specimen-surface-label">Storage repository row · 14px</span>
        <div className="specimen-surface">
          <RepoRowContext mark={mark(14)} name={`${concept.plural} · Codex`} />
        </div>
      </div>
      {concept.id === "pet" && (
        <div>
          <span className="specimen-surface-label">Pet card, no sprite yet · 16px</span>
          <div className="specimen-surface">
            <PetCardContext mark={mark(16)} name="tabby" />
          </div>
        </div>
      )}
    </div>
  );
}

/** What the shipped row weighs, and the two collisions that follow from it. */
function InkBand() {
  const ink = useNeighbourInk();
  if (!ink.length) return null;
  const low = ink[0][1];
  const high = ink[ink.length - 1][1];
  const mean = Math.round(ink.reduce((sum, [, n]) => sum + n, 0) / ink.length);
  return (
    <div className="specimen-measure" style={{ marginBottom: 16 }}>
      <div className="specimen-measure-row">
        <span className="specimen-verdict is-ok">the row weighs</span>
        <span>ink <b>{low}–{high}</b>/144</span>
        <span>mean <b>{mean}</b></span>
        {ink.map(([name, n]) => (
          <span key={name}>{name} <b>{n}</b></span>
        ))}
      </div>
      <p>
        Measured on mount by filling each shipped path on the 24-unit grid and sampling
        one point per coarse cell. The house grammar is <b>hollow</b> — folder, message,
        file, clock, drive and badge-check are all one-cell outlines, and the cog is the
        heaviest thing here. Every candidate above is drawn to sit inside this band; the
        earlier solid-fill drafts ran 52 to 81 and read as blots beside their neighbours.
      </p>
      <p>
        Two collisions fall straight out of the row. <b>The cog owns the centred convex
        mass</b> at roughly ten cells square, which is why a solid puzzle piece, a solid
        egg and a centred four-point star all measure closer to `tool` than to anything
        else. And <b>`catalog` owns the square grid of four detached blocks</b> — which
        is the VS Code and Office add-in convention for a plugin, spoken for here by the
        mark of the very page these icons live on.
      </p>
    </div>
  );
}

function Specimen() {
  const [theme, setTheme] = useState<"dark" | "light">(
    new URLSearchParams(window.location.search).get("theme") === "light" ? "light" : "dark",
  );
  const [pick, setPick] = useState<Record<string, number>>(
    Object.fromEntries(CONCEPTS.map((c) => [c.id, c.candidates.findIndex((k) => k.id === c.shippedId)])),
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  const chosen = CONCEPTS.map((c) => ({ concept: c, candidate: c.candidates[pick[c.id] ?? 0] }));

  return (
    <div className="specimen-page">
      <header className="specimen-head">
        <div>
          <h1>Marks for Skill, Plugin and Pet</h1>
          <p>
            The Library's three resource kinds borrow their glyphs today: a skill wears
            the generic document, a plugin wears the cog this app already spends on tool
            calls, and a pet wears the OpenAI provider mark — which `ProviderMark` paints
            on the same row anyway. Each candidate below is a metaphor that is already
            standard somewhere, redrawn on the 12-cell grid this icon set is built on, and
            shown at the four sizes and three surfaces it actually ships at.
          </p>
          <p>
            Two constraints did most of the choosing. The shipped row is <b>hollow and
            light</b> — its marks measure 22 to 48 filled cells and the cog is the ceiling
            — so every candidate here is an outline, not a fill. And a mark has to survive
            twelve cells: a jigsaw knob, a dog's muzzle and a brain's folds all measure out
            below the size at which a feature resolves. Where the canonical metaphor loses
            to the grid, it is still drawn and marked <span className="specimen-verdict is-risk">grid argues</span>,
            so the trade is visible rather than asserted.
          </p>
          <p>
            Judge at 12–16px, in the rows. The magnified view is there to show the cells,
            not to sell the shape.
          </p>
        </div>
        <div className="specimen-controls">
          <label>
            <input
              type="checkbox"
              checked={theme === "light"}
              onChange={(e) => setTheme(e.target.checked ? "light" : "dark")}
            />{" "}
            light
          </label>
        </div>
      </header>

      {CONCEPTS.map((concept) => (
        <section className="specimen-concept" key={concept.id}>
          <h3>{concept.title}</h3>
          <p>{concept.lead}</p>
          <div className="specimen-grid">
            <CandidateCard concept={concept} candidate={null} />
            {concept.candidates.map((candidate) => (
              <CandidateCard key={candidate.id} concept={concept} candidate={candidate} />
            ))}
          </div>
        </section>
      ))}

      <section className="specimen-concept">
        <h3>Against the neighbours</h3>
        <p>
          A glyph in this app is found by silhouette before it is read. Here is each
          concept's current pick beside every mark it has to be distinguished from, all
          at the 14px the headings use.
        </p>
        <InkBand />
        <div className="specimen-controls" style={{ margin: "0 0 14px", flexWrap: "wrap", gap: 22 }}>
          {CONCEPTS.map((concept) => (
            <label key={concept.id} style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
              {concept.title}
              <select
                value={pick[concept.id] ?? 0}
                onChange={(e) => setPick((p) => ({ ...p, [concept.id]: Number(e.target.value) }))}
              >
                {concept.candidates.map((candidate, i) => (
                  <option key={candidate.id} value={i}>{candidate.name}</option>
                ))}
              </select>
            </label>
          ))}
        </div>
        <div className="specimen-strip">
          {chosen.map(({ concept, candidate }) => (
            <span key={concept.id}>
              <PixelIcon bitmap={candidate.bitmap} size={14} />
              <mark>{concept.title}</mark>
            </span>
          ))}
          {NEIGHBOURS.map((name) => (
            <span key={name}>
              <Icon name={name} size={14} />
              {name}
            </span>
          ))}
        </div>
      </section>

      <section className="specimen-concept">
        <h3>The three rows, side by side</h3>
        <p>
          The sidebar's Catalog pane is where all three appear at once, at 16px, with
          nothing but the glyph to tell one row from the next.
        </p>
        <div className="specimen-surface" style={{ maxWidth: 320 }}>
          <SidebarContext
            rows={[
              { mark: <PixelIcon bitmap={chosen[0].candidate.bitmap} size={16} />, label: "Skills · Codex home" },
              { mark: <PixelIcon bitmap={chosen[1].candidate.bitmap} size={16} />, label: "Plugins · Claude home" },
              { mark: <PixelIcon bitmap={chosen[2].candidate.bitmap} size={16} />, label: "Pets v2 · Codex home", active: true },
              { mark: <PixelIcon bitmap={chosen[2].candidate.bitmap} size={16} />, label: "Pets v1 · Codex home" },
            ]}
          />
        </div>
        <p className="specimen-note" style={{ marginTop: 12 }}>
          <b>Note.</b> Pet v1 and Pet v2 are separate rows of the same kind, so they share
          one glyph and are told apart by their label. Nothing below proposes changing that.
        </p>
      </section>

      {/* Off-screen, unstyled, so `measureShipped` has real paths to read. */}
      <div id="ink-probe" aria-hidden="true" style={{ position: "absolute", left: -9999, top: 0 }}>
        {NEIGHBOURS.map((name) => <Icon key={name} name={name} size={24} />)}
      </div>

      <section className="specimen-concept">
        <h3>The paths</h3>
        <p>Ready to paste into <code>Icons.tsx</code> beside the marks they would join.</p>
        <div className="specimen-paths">
          {CONCEPTS.flatMap((concept) =>
            concept.candidates.map((candidate) => (
              <div key={`${concept.id}-${candidate.id}`}>
                <b>{concept.title} — {candidate.name}</b>
                <code>{pathFromBitmap(candidate.bitmap)}</code>
              </div>
            )),
          )}
        </div>
      </section>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
