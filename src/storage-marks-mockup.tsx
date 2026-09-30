import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import { PathBarMenu, type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import "./App.css";
import "./storage-marks-mockup.css";

/**
 * Mockup: the storage marks in the path bar — the storage crumb and its
 * switcher — made easier to see without touching the artwork.
 *
 * Today every storage mark there is the provider's full-colour artwork drawn
 * bare at 14px on the bar and the menu, and on the honey current row. The
 * sidebar and the landing cards already stand the same artwork on a pale
 * provider tile; the bar never got it.
 *
 *   Before  what shipped until 2026-09-28: bare 14px artwork
 *   A       Tile — the rail's storage tile (pale provider tile, 2px ink box); SHIPPED in App.css
 *   B       Plate — the pale provider plate with a hairline ring, no ink box
 *   C       Sticker — no box; 18px artwork with a paper rim and a 1px hard shadow
 *
 * Bars are hand-built from the real path-bar classes; the switcher is the real
 * `PathBarMenu`, held open. A comes from App.css itself; src/storage-marks-mockup.css
 * restores Before and draws B and C.
 *
 * `npm run dev`, then /storage-marks-mockup.html (`?theme=light|dark&glyph=16|14`).
 */

type Dir = "before" | "a" | "b" | "c";
const DIRS: readonly Dir[] = ["before", "a", "b", "c"];
const DIR_META: Record<Dir, { title: string; tag: string; note: string }> = {
  before: {
    title: "Before",
    tag: "until 2026-09-28",
    note: "The artwork drawn bare at 14px. On the honey current row and the cream popover the yellow folder, the teal duck and the pale Feishu wing have nothing behind them.",
  },
  a: {
    title: "A · Tile",
    tag: "shipped",
    note: "The rail's storage tile, carried into the bar: the provider's pale tile in the 2px ink box the sidebar already draws around the same artwork. 24px in the menu, 22px in the crumb. Rows stay 34px.",
  },
  b: {
    title: "B · Plate",
    tag: "softer",
    note: "The same pale provider plate with a 1px ring instead of the ink box — five soft squares rather than five boxes. The ring is what keeps the pale folder plate from dissolving into the honey row.",
  },
  c: {
    title: "C · Sticker",
    tag: "no box",
    note: "No tile. The artwork grows to 18px and its own outline gets a paper rim and the skin's 1px hard shadow, so it lifts off the row like a sticker.",
  },
};

interface Storage { id: string; name: string; icon: IconName; detail: string; path: boolean }
const STORAGES: Storage[] = [
  { id: "cloud", name: "Mallard Cloud", icon: "storage-mallard", detail: "rika321", path: false },
  { id: "local", name: "my-storage", icon: "storage-local", detail: "~/Documents/my-storage", path: true },
  { id: "feishu", name: "Feishu", icon: "storage-feishu", detail: "Mallard sync", path: false },
  { id: "dropbox", name: "Dropbox", icon: "storage-dropbox", detail: "/Apps/Mallard", path: false },
  { id: "r2", name: "team-backups", icon: "storage-r2", detail: "Bucket · team-backups", path: false },
];
const PROJECT = "mallard-app";
const noop = () => {};

function storageMenu(current: Storage): PathBarMenuGroup[] {
  return [
    {
      items: [
        { id: "storage-details", label: "Storage details…", icon: "settings", onSelect: noop },
        { id: "unlink", label: `Unlink from ${PROJECT}…`, icon: "link-off", danger: true, onSelect: noop },
      ],
    },
    {
      label: "Switch to",
      items: STORAGES.filter((storage) => storage.id !== current.id).map((storage) => ({
        id: storage.id,
        label: storage.name,
        detail: storage.detail,
        detailIsPath: storage.path,
        icon: storage.icon,
        onSelect: noop,
      })),
    },
    {
      items: [
        { id: "link-storage", label: "Link storage…", icon: "link", onSelect: noop },
        { id: "add-storage", label: "Add storage…", icon: "plus", onSelect: noop },
      ],
    },
  ];
}

/* ——— the bar ——————————————————————————————————————————————————— */

function StorageCrumb({ storage, expanded, label = true }: { storage: Storage; expanded: boolean; label?: boolean }) {
  return (
    <button
      type="button"
      className="sync-path-crumb root"
      title={`${storage.name} · ${storage.detail}`}
      aria-label={`${storage.name}. Storage menu`}
      aria-haspopup="menu"
      aria-expanded={expanded}
    >
      <Icon name={storage.icon} size={14} />
      {label && <span>{storage.name}</span>}
      <Icon name="chevron-down" size={11} className="sync-path-chevron" />
    </button>
  );
}

function Bar({ storage, open, pull = 12, push = 40 }: { storage: Storage; open: boolean; pull?: number; push?: number }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label={`${PROJECT} path`}>
        <li className="sync-path-item">
          <StorageCrumb storage={storage} expanded={open} />
          {open && (
            <PathBarMenu
              id={`spec-storage-menu-${storage.id}`}
              groups={storageMenu(storage)}
              header={{ label: storage.name, detail: storage.detail, detailIsPath: storage.path, icon: storage.icon }}
              label="Storage"
              align="start"
              open
              registerRef={noop}
              onClose={noop}
            />
          )}
        </li>
        <li className="sync-path-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <button type="button" className="sync-path-crumb" aria-haspopup="menu" aria-expanded={false}>
            <Icon name="git-folder" size={14} />
            <span>{PROJECT}</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
        </li>
        <li className="sync-path-item leaf-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <span className="sync-path-crumb section leaf" aria-current="page"><span>Git history</span></span>
        </li>
      </ol>
      <div className="sync-path-actions">
        {storage.id === "cloud" && (
          <span className="sync-path-popover">
            <button type="button" className="sync-path-attention sync-path-cloud-status success" aria-label="Mallard Cloud: Published">
              <Icon name="cloud-check" size={16} />
              <b>Published</b>
              <Icon name="chevron-down" size={11} className="sync-path-chevron" />
            </button>
          </span>
        )}
        <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
          <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
        </button>
        <div className="sync-path-action-group" role="group" aria-label="Sync actions">
          <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span><span className="sync-path-count">{pull}</span></button>
          <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span><span className="sync-path-count">{push}</span></button>
        </div>
        <span className="sync-path-divider" aria-hidden="true" />
        <div className="sync-path-overflow">
          <button type="button" className="sync-path-more" title="App actions" aria-label="App actions"><Icon name="more" size={16} /></button>
        </div>
      </div>
    </header>
  );
}

function DirTitle({ dir }: { dir: Dir }) {
  const meta = DIR_META[dir];
  return (
    <h3 className="spec-col-title">
      {meta.title}
      <em className={dir === "a" ? undefined : "quiet"}>{meta.tag}</em>
    </h3>
  );
}

/* ——— the matrix: every provider in every seat ———————————————————— */

function MatrixColumn({ dir }: { dir: Dir }) {
  return (
    <div className={`spec-matrix-col mk-${dir}`} data-dir={dir}>
      <DirTitle dir={dir} />
      <div className="spec-matrix-crumbs">
        {STORAGES.map((storage) => <StorageCrumb key={storage.id} storage={storage} expanded={storage.id === "cloud"} label={false} />)}
      </div>
      <div className="sync-path-menu switcher spec-matrix-card">
        {STORAGES.map((storage) => (
          <div key={storage.id}>
            <div className="sync-path-menu-current" aria-current="true" data-edge-row>
              <Icon name={storage.icon} size={14} />
              <span className="sync-path-menu-copy">
                <span>{storage.name}</span>
                <small className={storage.path ? undefined : "phrase"}>{storage.detail}</small>
              </span>
              <Icon name="check" size={13} className="sync-path-menu-check" />
            </div>
            <span className="spec-edge" data-edge-for={storage.id} />
          </div>
        ))}
        <div className="sync-path-menu-separator" role="separator" />
        {STORAGES.map((storage, index) => (
          <button
            key={storage.id}
            type="button"
            className={`sync-path-menu-item target${index === 1 ? " mk-hover" : ""}`}
            title={storage.path ? storage.detail : undefined}
          >
            <Icon name={storage.icon} size={14} />
            <span className="sync-path-menu-copy">
              <span>{storage.name}</span>
              {!storage.path && <small className="phrase">{storage.detail}</small>}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/* ——— measurement ——————————————————————————————————————————————— */

const probe = (() => {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  return canvas.getContext("2d", { willReadFrequently: true })!;
})();

/** Any CSS colour the engine can parse, composited over `under`, as sRGB bytes. */
function rgb(color: string, under: [number, number, number] = [255, 255, 255]): [number, number, number] {
  probe.fillStyle = `rgb(${under.join(",")})`;
  probe.fillRect(0, 0, 1, 1);
  probe.fillStyle = color;
  probe.fillRect(0, 0, 1, 1);
  const [r, g, b] = probe.getImageData(0, 0, 1, 1).data;
  return [r, g, b];
}

function luminance([r, g, b]: [number, number, number]): number {
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(a: [number, number, number], b: [number, number, number]): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The first painted background at or above `node`, composited down the tree. */
function surface(node: Element | null): [number, number, number] {
  const chain: string[] = [];
  for (let at = node; at; at = at.parentElement) {
    const bg = getComputedStyle(at).backgroundColor;
    chain.push(bg);
    if (rgb(bg, [0, 0, 0]).join() === rgb(bg, [255, 255, 255]).join()) break;
  }
  return chain.reverse().reduce<[number, number, number]>((under, bg) => rgb(bg, under), [255, 255, 255]);
}

/** Each current row: what separates the mark from the honey row it sits on. */
function measureEdges() {
  document.querySelectorAll<HTMLElement>(".spec-matrix-col").forEach((column) => {
    const dir = column.dataset.dir as Dir;
    column.querySelectorAll<HTMLElement>("[data-edge-for]").forEach((label) => {
      const row = label.previousElementSibling as HTMLElement;
      const mark = row.querySelector<SVGElement>("svg[data-storage-provider]")!;
      const ground = surface(row);
      const style = getComputedStyle(mark);
      const plate = rgb(style.backgroundColor, ground);
      const plateRatio = contrast(plate, ground);
      let text: string;
      let low = false;
      if (dir === "a") {
        const edge = contrast(rgb(style.borderTopColor, ground), plate);
        text = `ink edge ${edge.toFixed(1)}:1 · tile vs row ${plateRatio.toFixed(2)}:1`;
      } else if (dir === "b") {
        const ring = style.boxShadow.match(/(rgba?\([^)]*\)|color\([^)]*\)|#[0-9a-f]+)/i)?.[0] ?? "transparent";
        const edge = contrast(rgb(ring, plate), ground);
        low = edge < 3;
        text = `ring ${edge.toFixed(2)}:1${low ? " (< 3:1)" : ""} · plate vs row ${plateRatio.toFixed(2)}:1`;
      } else {
        text = dir === "c" ? "rim + 1px shadow, no plate" : "no edge, no plate";
      }
      label.textContent = text;
      label.classList.toggle("low", low);
    });
  });
}

interface BudgetRow { dir: Dir; counts: string; li: number; crumb: number; spare: number; overflow: number }

/** The path-bar probe: grow a spacer in the actions until any trail rect
 * passes the list's right edge. The spacer's margin cancels the column gap
 * it would otherwise add. */
function measureBudget(): BudgetRow[] {
  const rows: BudgetRow[] = [];
  document.querySelectorAll<HTMLElement>("[data-budget]").forEach((frame) => {
    const list = frame.querySelector<HTMLElement>(".sync-path")!;
    const actions = frame.querySelector<HTMLElement>(".sync-path-actions")!;
    const first = list.querySelector<HTMLElement>(".sync-path-item")!;
    const crumb = first.querySelector<HTMLElement>(".sync-path-crumb")!;
    const overflowOf = () => {
      const edge = list.getBoundingClientRect().right;
      let worst = -Infinity;
      list.querySelectorAll("*").forEach((node) => {
        const rect = node.getBoundingClientRect();
        if (rect.width > 0) worst = Math.max(worst, rect.right - edge);
      });
      return worst;
    };
    const overflow = overflowOf();
    const spacer = document.createElement("div");
    spacer.style.flex = "0 0 auto";
    spacer.style.height = "1px";
    spacer.style.marginLeft = `-${parseFloat(getComputedStyle(actions).columnGap) || 0}px`;
    actions.prepend(spacer);
    let spare = 0;
    for (let width = 1; width <= 320; width += 1) {
      spacer.style.width = `${width}px`;
      if (overflowOf() > 0.5) break;
      spare = width;
    }
    spacer.remove();
    rows.push({
      dir: frame.dataset.budget as Dir,
      counts: frame.dataset.counts!,
      li: first.getBoundingClientRect().width,
      crumb: crumb.getBoundingClientRect().width,
      spare,
      overflow,
    });
  });
  return rows;
}

/* ——— the page ——————————————————————————————————————————————————— */

type Theme = "light" | "dark";
type Glyph = "16" | "14";

function readKnobs(): { theme: Theme; glyph: Glyph } {
  const query = new URLSearchParams(window.location.search);
  return {
    theme: query.get("theme") === "dark" ? "dark" : "light",
    glyph: query.get("glyph") === "14" ? "14" : "16",
  };
}

function Page() {
  const [knobs, setKnobs] = useState(readKnobs);
  const [budget, setBudget] = useState<BudgetRow[]>([]);

  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
    const query = new URLSearchParams(window.location.search);
    query.set("theme", knobs.theme);
    query.set("glyph", knobs.glyph);
    window.history.replaceState(null, "", `?${query}`);
    let live = true;
    void document.fonts.ready.then(() => {
      if (!live) return;
      measureEdges();
      setBudget(measureBudget());
    });
    return () => { live = false; };
  }, [knobs.theme, knobs.glyph]);

  const seg = <K extends keyof typeof knobs>(key: K, value: (typeof knobs)[K], label: string) => (
    <button
      type="button"
      className="spec-seg"
      aria-pressed={knobs[key] === value}
      onClick={() => setKnobs((current) => ({ ...current, [key]: value }))}
    >
      {label}
    </button>
  );
  const before = (counts: string) => budget.find((row) => row.dir === "before" && row.counts === counts);

  return (
    <div className={`spec${knobs.glyph === "14" ? " glyph-14" : ""}`}>
      <div className="spec-title">Storage marks — directions</div>
      <p className="spec-sub">
        The storage crumb and its switcher, with the artwork untouched. What changes is what stands behind it and how big it is drawn.
        Bars are 580px — the narrowest the bar gets, where the crumb is icon-only, as in the screenshot.
      </p>
      <div className="spec-controls">
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
        <div className="spec-group"><span>Glyph in A/B</span>{seg("glyph", "16", "16px")}{seg("glyph", "14", "14px (rail recipe)")}</div>
      </div>

      <h2 className="spec-h2">1 · The switcher open, as in the screenshot</h2>
      <p className="spec-lede">Current storage Mallard Cloud; the other four as switch targets. Real <code>PathBarMenu</code>.</p>
      <div className="spec-grid">
        {DIRS.map((dir) => (
          <div key={dir}>
            <DirTitle dir={dir} />
            <p className="spec-col-note">{DIR_META[dir].note}</p>
            <div className={`spec-frame open mk-${dir}`} style={{ width: 580 }}>
              <Bar storage={STORAGES[0]} open />
            </div>
          </div>
        ))}
      </div>

      <h2 className="spec-h2">2 · Every provider in every seat</h2>
      <p className="spec-lede">
        The weak pairs are the yellow folder and the pale Feishu wing on the honey current row, not the Mallard Cloud header.
        Top: the icon-only crumb (first one open). Then each provider as the current row, with what separates its mark from the row.
        Then each as a target row, the second one hovered.
      </p>
      <div className="spec-matrix">
        {DIRS.map((dir) => <MatrixColumn key={dir} dir={dir} />)}
      </div>

      <h2 className="spec-h2">3 · The wide bar, with the storage name</h2>
      <div className="spec-budget-bars">
        {DIRS.map((dir) => (
          <div key={dir}>
            <DirTitle dir={dir} />
            <div className={`spec-frame mk-${dir}`} style={{ width: 1100 }}>
              <Bar storage={STORAGES[0]} open={false} />
            </div>
          </div>
        ))}
      </div>

      <h2 className="spec-h2">4 · Path-bar budget at 580px</h2>
      <p className="spec-lede">
        Actions never shrink, so anything the crumb gains comes out of the trail. The storage item has a 3.5rem (56px) floor, so a crumb
        that stays under it costs nothing. Spare = how far a spacer in the actions can grow before any trail box passes the list's edge
        (Mallard Cloud chip shown, as it is for this storage).
      </p>
      <div className="spec-panel">
        <table className="spec-budget">
          <thead>
            <tr><th>Direction</th><th>Pull / Push</th><th>Storage item</th><th>Crumb</th><th>Spare</th><th>vs Before</th><th>Trail overflow</th></tr>
          </thead>
          <tbody>
            {budget.map((row) => {
              const base = before(row.counts);
              const delta = base ? row.spare - base.spare : 0;
              return (
                <tr key={`${row.dir}-${row.counts}`}>
                  <td>{DIR_META[row.dir].title}</td>
                  <td className="num">{row.counts}</td>
                  <td className="num">{row.li.toFixed(1)}px</td>
                  <td className="num">{row.crumb.toFixed(1)}px</td>
                  <td className="num">{row.spare}px</td>
                  <td className={`num${delta < 0 ? " bad" : ""}`}>{delta === 0 ? "±0" : `${delta > 0 ? "+" : ""}${delta}px`}</td>
                  <td className={`num${row.overflow > 0.5 ? " bad" : ""}`}>{row.overflow > 0.5 ? `${row.overflow.toFixed(1)}px` : "none"}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="spec-budget-bars">
        {DIRS.flatMap((dir) => ([["12", "40"], ["128", "240"]] as const).map(([pull, push]) => (
          <div
            key={`${dir}-${pull}`}
            className={`spec-frame mk-${dir}`}
            data-budget={dir}
            data-counts={`${pull} / ${push}`}
          >
            <Bar storage={STORAGES[0]} open={false} pull={Number(pull)} push={Number(push)} />
          </div>
        )))}
      </div>

      <div className="spec-panel">
        <strong>Scope if picked.</strong>
        <ul>
          <li>CSS only, in App.css beside the existing <code>--storage-icon-tile</code> rules. Selectors reach the path bar's storage marks alone: <code>.sync-path-crumb</code>, <code>.sync-path-menu-current</code> and <code>.sync-path-menu-item</code> <code>&gt; svg[data-storage-provider]</code>. The settings plate and landing cards already pad their artwork and are left alone.</li>
          <li>No change to <code>Icons.tsx</code>, the assets, or any component. Project, folder and verb icons in the same menu keep their bare glyphs.</li>
          <li>Other bare storage marks at 12–14px (the link chooser, project setup's storage chip, storage discovery) could take the same rule later; not in this pass.</li>
        </ul>
      </div>
    </div>
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<StrictMode><Page /></StrictMode>);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
