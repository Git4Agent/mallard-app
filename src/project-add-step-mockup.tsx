import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import NewProjectFolderStep from "./components/project-sync/NewProjectFolderStep";
import ProjectFolderField from "./components/project-sync/ProjectFolderField";
import type { FolderVerdict } from "./components/project-sync/projectFolder";
import type { AgentFolder, LocalProjectSummary, ProjectFolderProbe, ProviderDescriptor } from "./types";
import "./App.css";

/**
 * Mockup: the New project page, after the owner's screenshot of the first
 * build. "Now" is the real NewProjectFolderStep on a mocked backend, with
 * this round's field alignment and fewer words. A, B and C redraw the folder
 * list in grammars the setup pages already speak:
 *
 *   A · Repository rows  the Repository band's option rows, radio and all:
 *                        pick one, then Use this folder, as a repository is
 *                        picked and then Finish.
 *   B · Choice cards     Add Catalog resource's cards: the name in the display
 *                        face, one click chooses.
 *   C · Dated list       one line per folder under Today / This week /
 *                        Earlier, the way editors list recent projects.
 *
 * All three: the band is named Folder (no second label), no sentence under an
 * empty field, unusable folders (missing, Mallard's own data) leave the list
 * for one quiet line, short dates, six rows then "Show all". Static data with
 * generic paths. `npm run dev` (or `tauri dev`), then
 * /project-add-step-mockup.html (`?dir=now|a|b|c&typed=empty|search|draft&scratch=list|fold|hide&theme=dark`).
 */

/* ——— data ——————————————————————————————————————————————————————— */

const HOME = "/Users/you";
type Kind = "new" | "added" | "draft" | "missing" | "mallard";
interface Row {
  name: string;
  rel: string;
  providers: string[];
  when: string;
  ago: number;
  group: "Today" | "This week" | "Earlier";
  git: boolean;
  kind: Kind;
  scratch?: boolean;
}
const ROWS: Row[] = [
  { name: "mallard-internal", rel: "Desktop/project/mallard-internal", providers: ["claude", "codex"], when: "1m ago", ago: 60, group: "Today", git: true, kind: "new" },
  { name: "mallard-app", rel: "Desktop/project/mallard-app", providers: ["claude", "codex"], when: "2h ago", ago: 7_200, group: "Today", git: true, kind: "added" },
  { name: "mirage", rel: "Desktop/project/mirage", providers: ["codex"], when: "Sep 23", ago: 172_800, group: "This week", git: true, kind: "new" },
  { name: "you", rel: "", providers: ["claude", "codex"], when: "Sep 22", ago: 259_200, group: "This week", git: false, kind: "mallard" },
  { name: "oci", rel: "Desktop/oci", providers: ["codex"], when: "Sep 21", ago: 345_600, group: "This week", git: false, kind: "new" },
  { name: "scratch-2026-09-10-8586f3", rel: "Library/Application Support/Claude/scratch-workspaces/56540ac4-11f1-473f-a91a-133940c2e7d1", providers: ["claude"], when: "Sep 20", ago: 432_000, group: "This week", git: false, kind: "new", scratch: true },
  { name: "hatch-pet-codex-skills", rel: "Documents/Codex/2026-09-16/hatch-pet-codex-skills", providers: ["codex"], when: "Sep 16", ago: 777_600, group: "Earlier", git: false, kind: "new" },
  { name: "realtime-voice-chat", rel: "Documents/Codex/2026-09-10/realtime-voice-chat", providers: ["codex"], when: "Sep 10", ago: 1_296_000, group: "Earlier", git: true, kind: "new" },
  { name: "myconf2", rel: "Desktop/myconf2", providers: ["codex"], when: "Sep 8", ago: 1_468_800, group: "Earlier", git: false, kind: "draft" },
  { name: "scratch-2026-09-06-1c2d9e", rel: "Library/Application Support/Claude/scratch-workspaces/0b9e7c1a-3f2d-4e1b-9a55-6d2c8e41f0aa", providers: ["claude"], when: "Sep 6", ago: 1_641_600, group: "Earlier", git: false, kind: "new", scratch: true },
  { name: "blog", rel: "code/blog", providers: ["claude"], when: "Sep 2", ago: 1_987_200, group: "Earlier", git: true, kind: "added" },
  { name: "notes", rel: "notes", providers: ["claude"], when: "Aug 30", ago: 2_246_400, group: "Earlier", git: false, kind: "new" },
  { name: "api-server", rel: "code/api-server", providers: ["codex"], when: "Aug 28", ago: 2_419_200, group: "Earlier", git: true, kind: "new" },
  { name: "landing-page", rel: "code/landing-page", providers: ["claude", "codex"], when: "Aug 21", ago: 3_024_000, group: "Earlier", git: true, kind: "new" },
  { name: "old-prototype", rel: "code/old-prototype", providers: ["codex"], when: "Aug 12", ago: 3_801_600, group: "Earlier", git: false, kind: "missing" },
  { name: "dotfiles", rel: "dotfiles", providers: ["codex"], when: "Aug 9", ago: 4_060_800, group: "Earlier", git: true, kind: "new" },
];
const pathOf = (row: Row) => (row.rel ? `~/${row.rel}` : "~");
const usable = (row: Row) => row.kind !== "missing" && row.kind !== "mallard";

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const PROJECTS = ROWS.filter((row) => row.kind === "added").map((row) => ({
  local_project_id: `project-${row.name}`, bundle_id: `bundle-${row.name}`, display_name: row.name, revision: 1,
  project_root: `${HOME}/${row.rel}`, canonical_project_root: `${HOME}/${row.rel}`,
})) as LocalProjectSummary[];

function probeOf(row: Row): ProjectFolderProbe {
  const path = row.rel ? `${HOME}/${row.rel}` : HOME;
  if (row.kind === "missing") return { kind: "missing" };
  if (row.kind === "mallard") return { kind: "mallard_data", contains: true };
  return {
    kind: "folder", canonical_path: path, is_git_repository: row.git,
    existing_draft_id: row.kind === "draft" ? `draft-${row.name}` : null,
    existing_project_ids: row.kind === "added" ? [`project-${row.name}`] : [],
  };
}

/* The real step runs against these. */
(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (command === "plugin:path|resolve_directory") return HOME;
  if (command === "list_agent_folders") {
    return {
      folders: ROWS.map((row): AgentFolder => ({
        path: row.rel ? `${HOME}/${row.rel}` : HOME,
        providers: row.providers,
        last_active: Math.floor(Date.now() / 1000) - row.ago,
        probe: probeOf(row),
      })),
    };
  }
  if (command === "probe_project_folder") {
    const path = String(args.projectRoot);
    const row = ROWS.find((candidate) => `${HOME}/${candidate.rel}` === path);
    return row ? probeOf(row) : { kind: "missing" };
  }
  return null;
});

/* ——— knobs ———————————————————————————————————————————————————————— */

type Direction = "now" | "a" | "b" | "c";
type Typed = "empty" | "search" | "draft";
type Scratch = "list" | "fold" | "hide";
interface Knobs { dir: Direction | "all"; typed: Typed; scratch: Scratch; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick("dir", ["all", "now", "a", "b", "c"] as const, "all"),
    typed: pick("typed", ["empty", "search", "draft"] as const, "empty"),
    scratch: pick("scratch", ["list", "fold", "hide"] as const, "fold"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/** What the static directions show for the chosen field state. */
function visibleRows(knobs: Knobs) {
  const query = knobs.typed === "search" ? "mall" : "";
  const matched = ROWS.filter((row) => !query || row.name.includes(query) || pathOf(row).includes(query));
  const shown = matched.filter((row) => usable(row) && !(row.scratch && knobs.scratch !== "list"));
  const unusable = matched.filter((row) => !usable(row)).length;
  const scratch = matched.filter((row) => usable(row) && row.scratch).length;
  return { shown, unusable, scratch, filtered: Boolean(query) };
}

const FIELD_TEXT: Record<Typed, string> = { empty: "", search: "mall", draft: "~/Desktop/myconf2" };
function fieldVerdict(typed: Typed): FolderVerdict | null {
  return typed === "draft" ? { tone: "ok", message: "Setup already started for this folder." } : null;
}

/* ——— shared pieces ———————————————————————————————————————————————— */

const marks = (row: Row, size = 12) => (
  <span className="nps-marks">
    {row.providers.map((provider) => (
      <ProviderMark key={provider} provider={provider} displayName={provider === "codex" ? "Codex" : "Claude"} size={size} />
    ))}
  </span>
);
const chip = (row: Row) => row.kind === "added"
  ? <span className="sync-draft-badge nps-chip">Added</span>
  : row.kind === "draft" ? <span className="sync-draft-badge nps-chip">Draft</span> : null;

function Page({ children, primary, primaryEnabled }: { children: ReactNode; primary: string; primaryEnabled: boolean }) {
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="New project">
          <header className="sync-modal-header">
            <div><h1>New project</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close"><Icon name="x" size={17} /></button>
          </header>
          <div className="sync-modal-body">{children}</div>
          <footer className="sync-modal-footer sync-setup-footer">
            <div>
              <button type="button" className="btn btn-ghost">Cancel</button>
              <button type="button" className="btn btn-primary" disabled={!primaryEnabled}>{primary}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

/** The Folder band every direction shares: the heading names the field. */
function FolderBand({ knobs, children }: { knobs: Knobs; children: ReactNode }) {
  const [text, setText] = useState(FIELD_TEXT[knobs.typed]);
  return (
    <section className="sync-setup-section nps-band">
      <div className="sync-card-heading"><div><strong>Folder</strong></div></div>
      <ProjectFolderField
        value={text}
        verdict={fieldVerdict(knobs.typed)}
        disabled={false}
        placeholder="Path or search"
        onChange={setText}
        onEnter={() => undefined}
        onBrowsed={() => undefined}
      />
      {children}
    </section>
  );
}

function ListHead({ total, shown, filtered }: { total: number; shown: number; filtered: boolean }) {
  return (
    <div className="nps-list-head">
      Recent
      <small>{filtered ? `${shown} of ${total}` : total}</small>
    </div>
  );
}

function HiddenNote({ unusable, scratch, knobs }: { unusable: number; scratch: number; knobs: Knobs }) {
  const parts = [
    unusable > 0 ? `${unusable} not usable` : null,
    scratch > 0 && knobs.scratch === "fold" ? `${scratch} app scratch folders` : null,
  ].filter(Boolean);
  if (parts.length === 0) return null;
  return (
    <p className="nps-hidden-note">
      Not shown: {parts.join(" · ")}
      {knobs.scratch === "fold" && scratch > 0 && <> · <button type="button" className="btn-link">Show</button></>}
    </p>
  );
}

/* ——— A · Repository rows ——————————————————————————————————————————— */

function DirectionA({ knobs }: { knobs: Knobs }) {
  const { shown, unusable, scratch, filtered } = visibleRows(knobs);
  const [picked, setPicked] = useState<string | null>(knobs.typed === "draft" ? "myconf2" : null);
  const limit = filtered ? shown.length : 6;
  const active = shown.find((row) => row.name === picked) ?? null;
  const primary = active?.kind === "added" ? `Open ${active.name}` : active?.kind === "draft" ? "Resume setup" : "Use this folder";
  return (
    <Page primary={primary} primaryEnabled={Boolean(active) || (filtered && shown.length === 1)}>
      <FolderBand knobs={knobs}>
        <ListHead total={ROWS.filter(usable).length} shown={shown.length} filtered={filtered} />
        <div className="nps-a">
          <div className="sync-bundle-match-options" role="radiogroup" aria-label="Recent folders">
            {shown.slice(0, limit).map((row) => {
              const on = picked === row.name;
              return (
                <button key={row.name} type="button" role="radio" aria-checked={on} className={`sync-bundle-match-option${on ? " active" : ""}`} onClick={() => setPicked(row.name)}>
                  <span className="sync-bundle-radio"><span /></span>
                  <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
                  <span className="sync-bundle-match-copy">
                    <strong>{row.name}{chip(row)}</strong>
                    <span>{pathOf(row)}</span>
                  </span>
                  <span className="sync-bundle-match-meta">
                    {row.when}
                    {marks(row)}
                  </span>
                </button>
              );
            })}
            {shown.length > limit && <button type="button" className="nps-more">Show all {shown.length}</button>}
          </div>
        </div>
        <HiddenNote unusable={unusable} scratch={scratch} knobs={knobs} />
      </FolderBand>
    </Page>
  );
}

/* ——— B · Choice cards ——————————————————————————————————————————————— */

function DirectionB({ knobs }: { knobs: Knobs }) {
  const { shown, unusable, scratch, filtered } = visibleRows(knobs);
  const limit = filtered ? shown.length : 6;
  return (
    <Page primary={knobs.typed === "draft" ? "Resume setup" : "Use this folder"} primaryEnabled={knobs.typed !== "empty"}>
      <FolderBand knobs={knobs}>
        <ListHead total={ROWS.filter(usable).length} shown={shown.length} filtered={filtered} />
        <div className="nps-b">
          <div className="setup-choice-grid">
            {shown.slice(0, limit).map((row) => (
              <button key={row.name} type="button" className="setup-choice-card">
                <span className="setup-choice-card-head">
                  <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.git ? "git-folder" : "folder"} size={24} /></span>
                  <strong className="nps-card-name">{row.name}</strong>
                  {chip(row)}
                </span>
                <span className="nps-card-path">{pathOf(row)}</span>
                <span className="nps-card-foot">{marks(row)}<span>{row.when}</span></span>
              </button>
            ))}
          </div>
        </div>
        {shown.length > limit && (
          <div className="nps-list-head"><button type="button" className="btn-link">Show all {shown.length}</button></div>
        )}
        <HiddenNote unusable={unusable} scratch={scratch} knobs={knobs} />
      </FolderBand>
    </Page>
  );
}

/* ——— C · Dated list ————————————————————————————————————————————————— */

function DirectionC({ knobs }: { knobs: Knobs }) {
  const { shown, unusable, scratch, filtered } = visibleRows(knobs);
  const limit = filtered ? shown.length : 9;
  const visible = shown.slice(0, limit);
  const groups = (["Today", "This week", "Earlier"] as const)
    .map((group) => ({ group, rows: visible.filter((row) => row.group === group) }))
    .filter((entry) => entry.rows.length > 0);
  return (
    <Page primary={knobs.typed === "draft" ? "Resume setup" : "Use this folder"} primaryEnabled={knobs.typed !== "empty"}>
      <FolderBand knobs={knobs}>
        <ListHead total={ROWS.filter(usable).length} shown={shown.length} filtered={filtered} />
        <div className="nps-c">
          <div className="sync-bundle-choice-list" role="listbox" aria-label="Recent folders">
            {groups.map(({ group, rows }) => (
              <div key={group} role="group" aria-label={group}>
                <div className="nps-group">{group}</div>
                {rows.map((row, index) => (
                  <button key={row.name} type="button" role="option" aria-selected={false} className={`nps-line${index === 0 && group === groups[0].group && knobs.typed !== "empty" ? " active" : ""}`}>
                    <Icon name={row.git ? "git-folder" : "folder"} size={16} />
                    <strong>{row.name}{chip(row)}</strong>
                    <span className="nps-line-path">{pathOf(row)}</span>
                    {marks(row)}
                    <span className="nps-line-when">{row.when}</span>
                  </button>
                ))}
              </div>
            ))}
            {shown.length > limit && <button type="button" className="nps-more">Show all {shown.length}</button>}
          </div>
        </div>
        <HiddenNote unusable={unusable} scratch={scratch} knobs={knobs} />
      </FolderBand>
    </Page>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const DIRECTIONS: { id: Direction; title: string; note: string }[] = [
  { id: "now", title: "Now · the real step", note: "This round's fixes: the input box on the 32px setup control, no sentence under an empty field, shorter placeholder and heading. Live: type, search, click." },
  { id: "a", title: "A · Repository rows", note: "The Repository band's option rows: radio, 24px mark, name over path, date and marks on the right. Pick, then Use this folder — as a repository is picked, then Finish." },
  { id: "b", title: "B · Choice cards", note: "Add Catalog resource's cards: name in the display face, path under it, marks and date at the foot. One click chooses; best for scanning names." },
  { id: "c", title: "C · Dated list", note: "One line per folder under Today / This week / Earlier, like an editor's recent projects. Densest: nine rows before Show all, and ↑↓ from the field walk it." },
];

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ dir: next.dir, typed: next.typed, scratch: next.scratch, theme: next.theme });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = DIRECTIONS.filter((direction) => knobs.dir === "all" || knobs.dir === direction.id);
  const key = `${knobs.typed}-${knobs.scratch}`;
  return (
    <div className="spec">
      <h1 className="spec-title">New project — directions</h1>
      <div className="spec-controls">
        <div className="spec-group"><span>Show</span>{seg("dir", "all", "All")}{seg("dir", "now", "Now")}{seg("dir", "a", "A")}{seg("dir", "b", "B")}{seg("dir", "c", "C")}</div>
        <div className="spec-group"><span>Field (A–C)</span>{seg("typed", "empty", "Empty")}{seg("typed", "search", "Search “mall”")}{seg("typed", "draft", "A draft’s folder")}</div>
        <div className="spec-group"><span>App scratch folders</span>{seg("scratch", "list", "Listed")}{seg("scratch", "fold", "Folded")}{seg("scratch", "hide", "Hidden")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <div className="spec-common">
        <strong>In every direction</strong>
        <ul>
          <li>The band is named <em>Folder</em> and the field has no second label; the page title already says New project.</li>
          <li>No sentence under an empty field. The line appears only once something is typed.</li>
          <li>Folders that can’t be used (not found, Mallard’s own data) leave the list for one quiet “Not shown” line.</li>
          <li>Short dates (1m ago, 2h ago, Sep 23) instead of 9/23/2026; six rows, then Show all — no scroll box inside the page.</li>
          <li>App scratch folders (Claude’s <code>~/Library/Application Support/…/scratch-workspaces</code>) are a knob above: listed, folded into the Not shown line, or hidden.</li>
        </ul>
      </div>
      <div className="spec-row">
        {shown.map((direction) => (
          <div key={direction.id}>
            <h2 className="spec-col-title">{direction.title}</h2>
            <p className="spec-col-note">{direction.note}</p>
            <div className="spec-frame">
              {direction.id === "now" ? (
                <main className="sync-main sync-project-links-page sync-project-setup-page">
                  <section className="profile-links-section sync-project-overview">
                <NewProjectFolderStep
                  workflowEpoch={1}
                  isWorkflowCurrent={() => true}
                  projects={PROJECTS}
                  providers={PROVIDERS}
                  busy={false}
                  onCancel={() => undefined}
                  onChoose={() => undefined}
                  onOpenProject={() => undefined}
                />
                  </section>
                </main>
              ) : direction.id === "a" ? <DirectionA key={key} knobs={knobs} />
                : direction.id === "b" ? <DirectionB key={key} knobs={knobs} />
                  : <DirectionC key={key} knobs={knobs} />}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
