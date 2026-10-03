import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../src/components/sync-review/ThreadStatusIcon";
import "../src/App.css";

/**
 * Mockup: the Catalog scope page with stored-only rows partitioned to the top.
 *
 * Option A, chosen: the partition and nothing else. The rows themselves are
 * untouched — same card, same borders, same 6px rhythm as the rows below —
 * and the cloud group is an ordinary section of the page rather than the
 * activity page's framed block. That is the whole point of this pass: a
 * resource that is only in the storage should be somewhere predictable, not
 * drawn as a different kind of object.
 *
 * It costs no new CSS. `.sync-history-section-heading` already carries the
 * 18px/6px section rhythm and pushes a trailing chip to the far edge;
 * `.sync-history-thread-list` already sets the 6px gap. The page just draws
 * two of each instead of one.
 *
 * Three variants differ only in what the cloud heading carries. Static data,
 * shipped classes, real leaf components — dev server only.
 *
 * Pet tiles draw the kind glyph: the real page paints the sprite from the
 * atlas, and this harness has no atlas. The tile box is the same 44x48, which
 * is what the layout turns on.
 */

const STORAGE = "Vault";

interface Row {
  id: string;
  name: string;
  status: ThreadStatusIconName;
  tone: string;
  statusLabel: string;
  files?: string;
  size?: string;
  stamp: string;
  stampDetail: string;
  local: boolean;
  pet?: boolean;
  /** Which trailing controls the row draws. Plugins cannot be forked, and a
   *  project file only ever offers Reveal. */
  controls?: "catalog" | "plugin" | "file";
  /** The files face's own state chip, beside the name. */
  chip?: string;
  /** Files are named by their leaf and titled by their path. */
  path?: string;
}

const SYNCED = `Up to date here and in ${STORAGE}.`;
const STORED_ONLY = `Only in ${STORAGE}. Pull to download it here.`;

const localPet = (id: string, name: string, size: string, stamp: string): Row => ({
  id, name, status: "synced", tone: "synced", statusLabel: SYNCED,
  files: "1", size, stamp, stampDetail: `Edited ${stamp}`, local: true, pet: true,
});

const cloudPet = (id: string, name: string, stamp: string): Row => ({
  id, name, status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY,
  stamp, stampDetail: `Stored ${stamp}`, local: false, pet: true,
});

/* The scenario asked for: a couple of pets on this Mac, a herd of them only in
   the storage — what a second computer's Catalog looks like before its first
   Pull. */
const PETS: Row[] = [
  localPet("maple", "Maple", "2.5 MB", "Aug 30, 2026 at 02:53"),
  cloudPet("dahuang", "大黄", "Aug 30, 2026 at 15:38"),
  cloudPet("scout", "Scout", "Aug 29, 2026 at 11:04"),
  localPet("patch", "Patch", "1.6 MB", "Aug 30, 2026 at 02:53"),
  cloudPet("pepper", "Pepper", "Aug 27, 2026 at 19:22"),
  cloudPet("tofu", "Tofu", "Aug 24, 2026 at 08:47"),
  cloudPet("juniper", "Juniper", "Aug 21, 2026 at 16:10"),
];

/* The same page one storage further along: ten stored pets against two local
   ones, which is where a fold starts to earn its keep. */
const MANY_PETS: Row[] = [
  ...PETS,
  cloudPet("biscuit", "Biscuit", "Aug 19, 2026 at 09:31"),
  cloudPet("mochi", "Mochi", "Aug 18, 2026 at 22:05"),
  cloudPet("pip", "Pip", "Aug 17, 2026 at 13:44"),
  cloudPet("clover", "Clover", "Aug 15, 2026 at 07:58"),
  cloudPet("sable", "Sable", "Aug 12, 2026 at 20:16"),
];

/* Skills, to show the partition is not a pet feature: same two sections, same
   spacing, no preview column. */
const SKILLS: Row[] = [
  { id: "api-designer", name: "api-designer", status: "synced", tone: "synced", statusLabel: SYNCED, files: "4", size: "21.4 KB", stamp: "Aug 19, 2026 at 14:03", stampDetail: "Edited Aug 19, 2026 at 14:03", local: true },
  { id: "release-notes", name: "release-notes", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 30, 2026 at 08:15", stampDetail: "Stored Aug 30, 2026 at 08:15", local: false },
  { id: "changelog-writer", name: "changelog-writer", status: "local-outstanding", tone: "local", statusLabel: `Newer on this computer. Push to update ${STORAGE}.`, files: "3", size: "12.8 KB", stamp: "Aug 29, 2026 at 21:54", stampDetail: "Edited Aug 29, 2026 at 21:54", local: true },
  { id: "sql-review", name: "sql-review", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 16, 2026 at 23:54", stampDetail: "Stored Aug 16, 2026 at 23:54", local: false },
  { id: "trace-download", name: "exascale-trace-download", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 14, 2026 at 10:44", stampDetail: "Stored Aug 14, 2026 at 10:44", local: false },
];

/* Plugins: the one Catalog kind that cannot be forked, so its rows draw three
   controls rather than four. */
const PLUGINS: Row[] = [
  { id: "context7", name: "context7", status: "synced", tone: "synced", statusLabel: SYNCED, files: "2", size: "8.4 KB", stamp: "Aug 26, 2026 at 12:31", stampDetail: "Edited Aug 26, 2026 at 12:31", local: true, controls: "plugin" },
  { id: "playwright-mcp", name: "playwright-mcp", status: "local-outstanding", tone: "local", statusLabel: `Only on this computer. Push to save it in ${STORAGE}.`, files: "3", size: "11.2 KB", stamp: "Aug 28, 2026 at 17:09", stampDetail: "Edited Aug 28, 2026 at 17:09", local: true, controls: "plugin" },
  { id: "linear-mcp", name: "linear-mcp", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 29, 2026 at 10:02", stampDetail: "Stored Aug 29, 2026 at 10:02", local: false, controls: "plugin" },
  { id: "sentry-mcp", name: "sentry-mcp", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 25, 2026 at 14:48", stampDetail: "Stored Aug 25, 2026 at 14:48", local: false, controls: "plugin" },
  { id: "blender-mcp", name: "blender-mcp", status: "cloud-outstanding", tone: "storage", statusLabel: STORED_ONLY, stamp: "Aug 20, 2026 at 21:37", stampDetail: "Stored Aug 20, 2026 at 21:37", local: false, controls: "plugin" },
];

/* Project files. A file row carries one control (Reveal), a size the storage
   knows as well as this Mac, and the files face's own state chip. */
interface FileGroup { directory: string; rows: Row[] }

const file = (path: string, size: string, stamp: string, local: boolean, chip?: string): Row => ({
  id: path,
  name: path.split("/").pop()!,
  path,
  status: local ? "synced" : "cloud-outstanding",
  tone: local ? "synced" : "storage",
  statusLabel: local ? SYNCED : STORED_ONLY,
  size,
  stamp,
  stampDetail: `${local ? "Edited" : "Stored"} ${stamp}`,
  local,
  controls: "file",
  chip,
});

const CLOUD_FILES: FileGroup[] = [
  { directory: "docs", rows: [
    file("docs/architecture.md", "18.2 KB", "Aug 30, 2026 at 09:12", false),
    file("docs/rollout.md", "6.1 KB", "Aug 29, 2026 at 16:40", false),
  ] },
  { directory: "scripts", rows: [
    file("scripts/deploy.sh", "2.4 KB", "Aug 28, 2026 at 08:03", false, "Executable"),
  ] },
];

const LOCAL_FILES: FileGroup[] = [
  { directory: "src", rows: [
    file("src/App.css", "212 KB", "Aug 30, 2026 at 02:11", true),
    file("src/main.tsx", "1.8 KB", "Aug 22, 2026 at 19:55", true),
  ] },
];

const stored = (rows: Row[]) => rows.filter((row) => !row.local);
const here = (rows: Row[]) => rows.filter((row) => row.local);

// ---------------------------------------------------------------------------
// The card, exactly as it ships — nothing about a row changes in this option
// ---------------------------------------------------------------------------

function Metric({ icon, value, detail }: { icon: IconName; value: string; detail: string }) {
  return (
    <span className="sync-history-thread-metric" data-tooltip={detail} aria-label={detail} tabIndex={0}>
      <Icon name={icon} size={12} /><b>{value}</b>
    </span>
  );
}

function CatalogCard({ row }: { row: Row }) {
  const dead = !row.local;
  const controls = row.controls ?? "catalog";
  return (
    <article className={`sync-history-thread-card catalog-card${row.pet ? " catalog-card-pet" : ""}`}>
      {row.pet && (
        <span className="catalog-pet-preview" aria-hidden="true">
          <Icon name="catalog-pet-v1" size={16} />
        </span>
      )}
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span className={`sync-thread-sync-indicator ${row.tone}`} data-tooltip={row.statusLabel} aria-label={row.statusLabel} tabIndex={0}>
            <ThreadStatusIcon status={row.status} size={18} />
          </span>
          <strong title={row.path ?? row.name}>{row.name}</strong>
          {row.chip && <span className="sync-history-branch-state history-only">{row.chip}</span>}
        </div>
        <div className="sync-history-thread-actions">
          {controls !== "file" && (
            <>
              <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Codex">
                <ProviderMark provider="codex" displayName="Codex" />
              </button>
              <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Terminal">
                <Icon name="terminal" size={14} />
              </button>
            </>
          )}
          {controls === "catalog" && (
            <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={dead} title={dead ? "Not on this Mac" : `Fork ${row.name}`}>
              <Icon name="git-fork" size={14} />
            </button>
          )}
          <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={dead} title={dead ? "Not on this Mac" : `Reveal ${row.name} in Finder`}>
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${row.name} details`}>
            {controls !== "file" && (
              <Metric icon="file" value={row.files ?? "—"} detail={row.files ? `Files · ${row.files}` : "Not on this Mac, so nothing was counted"} />
            )}
            <Metric icon="drive" value={row.size ?? "—"} detail={row.size ? `${row.local ? "On disk" : `In ${STORAGE}`} · ${row.size}` : "Not on this Mac, so nothing was measured"} />
            <span className="sync-history-thread-metric" aria-hidden="true" />
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <div className="sync-history-debug-menu">
              <button type="button" className="btn btn-ghost sync-history-session-detail-action" title={`Paths and digests for ${row.name}`}>
                <Icon name="info" size={13} />
              </button>
            </div>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={row.stampDetail} aria-label={row.stampDetail} tabIndex={0}>
            <time>{row.stamp}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

// ---------------------------------------------------------------------------
// Sections — the page's own heading and list, drawn twice
// ---------------------------------------------------------------------------

/** One section: the heading the page already uses, then the list it already
 *  uses. `count` and `fold` are the only variables between the variants. */
function Section({ heading, rows, count, countLabel, mark, foldable }: {
  heading: ReactNode;
  rows: Row[];
  count?: boolean;
  countLabel?: string;
  mark?: IconName;
  foldable?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const label = countLabel ?? "";
  return (
    <>
      <div className="sync-history-section-heading">
        <h2>
          {foldable ? (
            <button
              type="button"
              className="catalog-section-fold"
              aria-expanded={!collapsed}
              aria-label={`${collapsed ? "Expand" : "Collapse"} ${label}`}
              title={`${collapsed ? "Expand" : "Collapse"} ${label}`}
              onClick={() => setCollapsed(!collapsed)}
            >
              <Icon name={collapsed ? "chevron-right" : "chevron-down"} size={13} />
            </button>
          ) : null}
          {heading}
        </h2>
        {count && mark && (
          <span className="sync-history-heading-count" title={label} aria-label={label}>
            <Icon name={mark} size={12} />{rows.length}
          </span>
        )}
      </div>
      {!collapsed && (
        <div className="sync-history-thread-list">
          {rows.map((row) => <CatalogCard key={row.id} row={row} />)}
        </div>
      )}
    </>
  );
}

const cloudHeading = <><Icon name="cloud" size={14} />Only in <span className="sync-history-storage-name">{STORAGE}</span></>;

function PageFrame({ children }: { children: ReactNode }) {
  return (
    <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page mock-page">
      <section className="profile-links-section">{children}</section>
    </main>
  );
}

/** What ships today: one list, scope order, stored-only rows wherever they
 *  happen to fall. */
function Today({ rows, kind, mark }: { rows: Row[]; kind: string; mark: IconName }) {
  return (
    <PageFrame>
      <Section heading={<><Icon name={mark} size={14} />{kind}</>} rows={rows} />
    </PageFrame>
  );
}

/** Option A: two sections, identical rows. The variants differ only in what
 *  the cloud heading carries. */
function Partitioned({ rows, kind, mark, count = false, foldable = false }: {
  rows: Row[];
  kind: string;
  mark: IconName;
  count?: boolean;
  foldable?: boolean;
}) {
  const cloud = stored(rows);
  const local = here(rows);
  const noun = kind.toLowerCase();
  return (
    <PageFrame>
      {cloud.length > 0 && (
        <Section
          heading={cloudHeading}
          rows={cloud}
          count={count}
          mark={mark}
          countLabel={`${cloud.length} ${noun} not on this computer`}
          foldable={foldable}
        />
      )}
      <Section
        heading={<><Icon name={mark} size={14} />{kind}</>}
        rows={local}
        count={count}
        mark={mark}
        countLabel={`${local.length} ${noun} on this computer`}
      />
    </PageFrame>
  );
}

/** The project-files face, which already partitions cloud-first — with its own
 *  heading grammar. `unified` swaps that block heading for the section heading
 *  the sessions page and this option use, so all three faces say it one way. */
function FilesFace({ unified }: { unified: boolean }) {
  const cloudCount = CLOUD_FILES.reduce((sum, group) => sum + group.rows.length, 0);
  const groups = (list: FileGroup[]) => list.map((group) => (
    <div key={group.directory}>
      <div className="sync-project-files-group-heading">
        <Icon name="folder" size={14} />
        <strong>{group.directory}/</strong>
        <small>{group.rows.length} file{group.rows.length === 1 ? "" : "s"}</small>
      </div>
      <div className="sync-history-thread-list">
        {group.rows.map((row) => <CatalogCard key={row.id} row={row} />)}
      </div>
    </div>
  ));
  /* The real page's own section element, so the shipped spacing rules apply
     to the shipped variant exactly as they do in the app. */
  return (
    <section className="sync-project-files-page">
      {unified ? (
        <>
          <div className="sync-history-section-heading">
            <h2>{cloudHeading}</h2>
          </div>
          {groups(CLOUD_FILES)}
          <div className="sync-history-section-heading">
            <h2><Icon name="file" size={14} />Project files</h2>
          </div>
          {groups(LOCAL_FILES)}
        </>
      ) : (
        <>
          <div className="sync-project-files-cloud-block">
            <div className="sync-project-files-cloud-heading">
              <Icon name="cloud" size={14} />
              <strong>Only in {STORAGE}</strong>
              <small>{cloudCount} items · Pull to download</small>
            </div>
            {groups(CLOUD_FILES)}
          </div>
          {groups(LOCAL_FILES)}
        </>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Only in {STORAGE}, on top</h1>
            <p>
              Option A: the partition, and nothing else. Rows keep the card they have — same
              border, same background, same 6px gap between them — and the stored-only ones sit
              in their own section above the rest instead of being scattered through it. The
              cloud section is an ordinary section of this page, not the activity page’s framed
              block, so both halves read as one list broken in two. It needs no new CSS: the
              18px/6px heading rhythm, the trailing chip slot and the 6px card gap are already
              what a section here does.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel
          title="Today — five stored pets, scattered"
          note={<>
            The scenario you asked for, on the page as it ships: two pets on this Mac and five
            that are only in {STORAGE}, interleaved in scope order. Every stored row shows the
            same two em-dashes and two dead controls, and there is nothing to tell you at a
            glance that most of this list is not here.
          </>}
        >
          <Today rows={PETS} kind="Pets v2" mark="catalog-pet-v2" />
        </Panel>

        <Panel
          title="A1 — two plain sections (SHIPPED)"
          note={<>
            The minimum: the stored rows lift into a section headed
            “Only in {STORAGE}”, the rest keeps the kind heading it has now. Same heading
            weight, same 18px above and 6px below, same cards. Nothing else on the page changes,
            and no heading grows furniture it does not have today.
          </>}
        >
          <Partitioned rows={PETS} kind="Pets v2" mark="catalog-pet-v2" />
        </Panel>

        <Panel
          title="A2 — the headings carry a count"
          note={<>
            The same two sections, each closing its heading line with the count chip the app
            already draws elsewhere. “How many are not here” is exactly the question this
            partition exists to answer, and a chip answers it without opening anything. It is a
            small break with the shipped rule that this page’s heading carries no count — that
            rule was written when there was one heading and the number would have read as a row.
          </>}
        >
          <Partitioned rows={PETS} kind="Pets v2" mark="catalog-pet-v2" count />
          <div className="mockup-substage">
            <p>Skills, same treatment — the partition is not a pet feature:</p>
            <Partitioned rows={SKILLS} kind="Skills" mark="lightbulb" count />
          </div>
        </Panel>

        <Panel
          title="Skills and plugins — the same two sections"
          note={<>
            The partition is a property of the page, not of pets. Skills carry no preview column
            and plugins cannot be forked — three controls rather than four — but both draw the
            same two sections at the same rhythm, with the kind’s own glyph in the second
            heading and the storage’s name in the first.
          </>}
        >
          <Partitioned rows={SKILLS} kind="Skills" mark="lightbulb" count />
          <div className="mockup-substage">
            <p>Plugins — no Fork control on the row, everything else unchanged:</p>
            <Partitioned rows={PLUGINS} kind="Plugins" mark="catalog-plugin" count />
          </div>
        </Panel>

        <Panel
          title="Project files — unified (SHIPPED)"
          note={<>
            What the Files face used to say — a bold label and a phrase, styled by a block this
            cutover retired (its rules live on only in this harness). The rows’ own badges
            already carry “Pull to download it here”, so the phrase said it twice.
          </>}
        >
          <FilesFace unified={false} />
          <div className="mockup-substage">
            <p>Shipped: the same files under the catalog’s heading — three faces, one grammar,
            and the local half headed “Project files” only while the partition exists:</p>
            <FilesFace unified />
          </div>
        </Panel>

        <Panel
          title="A3 — the cloud section can fold"
          note={<>
            Ten stored pets against two local ones — a second Mac before its first Pull. The
            count alone stops helping once the section is longer than the screen, so the cloud
            heading takes a chevron and remembers being shut. The local section never folds:
            it is the page’s subject. Click the chevron to see the folded state.
          </>}
        >
          <Partitioned rows={MANY_PETS} kind="Pets v2" mark="catalog-pet-v2" count foldable />
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles: the harness shell, and the one rule the variants add.
// ---------------------------------------------------------------------------

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mockup-substage { padding: 0 0 4px; border-top: 1px solid var(--border); }
.mockup-substage > p { margin: 12px 16px 0; color: var(--text-2); font-size: var(--type-xs); }
/* The page's own top padding stands in for the window chrome the app draws. */
.mock-page { min-height: 0; padding: 0 16px 16px; overflow: visible; }
body { background: var(--app-shell); }
`;

const PROPOSED_CSS = `
/* The only new rule, and only for A3: a fold that sits inside the heading,
   because this page has no rail for a spine fold to live in. The heading is
   already an inline-flex row with an 8px gap, so the button just joins it. */
.catalog-section-fold {
  width: 20px;
  height: 20px;
  margin-left: -3px;
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: 4px;
  color: var(--text-3);
  background: transparent;
  cursor: pointer;
}

.catalog-section-fold:hover {
  color: var(--text-0);
  background: var(--button-bg-hover);
}
`;

/* The block the files cutover retired, kept verbatim so the before-panel
   renders what it replaced. App.css no longer carries these rules. */
const ARCHIVED_CSS = `
.sync-project-files-cloud-block {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding-bottom: 14px;
  border-bottom: 1px dashed var(--border-mid);
}
.sync-project-files-cloud-heading {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  margin: 2px 2px 0;
  padding: 0 6px;
  color: var(--text-1);
}
.sync-project-files-cloud-heading > svg { flex: 0 0 auto; color: var(--thread-status-cloud); }
.sync-project-files-cloud-heading > strong {
  color: var(--text-0);
  font-size: var(--type-sm);
  font-weight: var(--weight-semibold);
}
.sync-project-files-cloud-heading > small { color: var(--text-3); font-size: var(--type-2xs); }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS + ARCHIVED_CSS;
document.head.appendChild(style);
/* One root across hot reloads: re-running the module must reuse it, or the
   dev server's second pass warns and mounts a second tree. */
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
