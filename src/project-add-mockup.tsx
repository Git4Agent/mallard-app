import { StrictMode, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import ProjectPicker from "./components/project-sync/ProjectPicker";
import LandingPicker from "./components/project-sync/LandingPicker";
import type {
  LocalProjectSummary,
  ProviderDescriptor,
  ProviderProfileSummary,
  SetupDraftSummary,
  StorageSummary,
} from "./types";
import "./App.css";

/**
 * Mockup: adding a project.
 *
 * Today `Add project` calls the system folder dialog before anything in
 * Mallard is on screen (ProjectSync.beginAddProject), the pick creates a
 * setup draft straight away, and the setup page's Folder is read-only with a
 * picker-only Change…. Nothing anywhere takes a typed or pasted path.
 *
 * Two bases and three layers that stack on either:
 *
 *   A · Folder sheet        GitHub Desktop's Add Local Repository: a sheet
 *                           with one path field, Browse, a verdict, Continue.
 *   B · Folder in the form  JetBrains' New Project: the setup page opens at
 *                           once, Folder first; the rest wakes on a folder.
 *   C · Suggested folders   VS Code Open Recent / JetBrains Recent Projects,
 *                           fed by where Codex and Claude sessions ran.
 *   D · Completion          VS Code's simple file dialog, Zed's built-in
 *                           keyboard-first pickers: Tab through subfolders.
 *   E · Drop a folder       GitHub Desktop and JetBrains: drop onto the window.
 *
 * Real ProjectSidebar, ProjectPicker/LandingPicker and App.css; the setup page
 * is static markup in its shipped classes. A canned machine answers the field
 * the way `create_setup_draft` would (absolute clean path, exists, directory,
 * no overlap with ~/.mallard, symlinks resolved, one draft per folder).
 * `npm run dev`, then /project-add-mockup.html
 * (`?base=before|a|b&layers=suggest,complete,drop&platform=mac|linux&stock=some|none&scan=done|loading|empty&theme=dark`).
 */

/* ——— the machine ——————————————————————————————————————————————— */

type Platform = "mac" | "linux";
const HOMES: Record<Platform, string> = { mac: "/Users/you", linux: "/home/you" };

interface FsEntry { kind: "dir" | "file" | "link"; git?: boolean; target?: string }

function machine(platform: Platform): Map<string, FsEntry> {
  const home = HOMES[platform];
  const fs = new Map<string, FsEntry>();
  const dir = (path: string, git = false) => fs.set(path, { kind: "dir", git });
  const volumes = platform === "mac" ? "/Volumes" : "/mnt";
  for (const path of ["/", platform === "mac" ? "/Users" : "/home", home, volumes, `${volumes}/backup`, `${volumes}/backup/old-work`]) dir(path);
  for (const rel of ["code", "code/scratch", "notes", "Desktop", "Documents", "Downloads", "sites", ".codex", ".claude", ".mallard", "code/mallard-app/src", "code/mallard-app/docs"]) dir(`${home}/${rel}`);
  for (const rel of ["code/mallard-app", "code/api", "code/blog", "code/mallet", "sites/website"]) dir(`${home}/${rel}`, true);
  for (const rel of ["code/mallard-app/README.md", "code/mallard-app/package.json", "notes/todo.md"]) fs.set(`${home}/${rel}`, { kind: "file" });
  fs.set(`${home}/app`, { kind: "link", target: `${home}/code/mallard-app` });
  return fs;
}

const parentOf = (path: string): string | null => (path === "/" ? null : path.slice(0, path.lastIndexOf("/")) || "/");
const baseName = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
const compact = (path: string, home: string) => (path === home ? "~" : path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path);
const isWithin = (path: string, root: string) => path === root || path.startsWith(root === "/" ? "/" : `${root}/`);

interface Env {
  platform: Platform;
  home: string;
  fs: Map<string, FsEntry>;
  projects: LocalProjectSummary[];
  drafts: SetupDraftSummary[];
}

/** What the field holds, before the machine is asked anything. */
type Reading =
  | { kind: "empty" }
  | { kind: "search"; query: string }
  | { kind: "dots" }
  | { kind: "path"; path: string };

function read(raw: string, home: string): Reading {
  // A terminal copy arrives quoted, a Finder or file-manager drag as a URL.
  let text = raw.trim().replace(/^(['"])(.*)\1$/, "$2");
  if (!text) return { kind: "empty" };
  if (text.startsWith("file://")) {
    try { text = decodeURIComponent(text.slice(7)); } catch { /* keep as typed */ }
  }
  if (text === "~") text = home;
  else if (text.startsWith("~/")) text = home + text.slice(1);
  if (!text.startsWith("/")) return { kind: "search", query: text };
  const path = text.replace(/\/+/g, "/").replace(/(.)\/$/, "$1");
  if (path.split("/").some((part) => part === "." || part === "..")) return { kind: "dots" };
  return { kind: "path", path };
}

type Verdict =
  | { kind: "empty" }
  | { kind: "search"; query: string }
  | { kind: "relative" }
  | { kind: "dots" }
  | { kind: "checking"; path: string }
  | { kind: "missing" }
  | { kind: "file"; parent: string }
  | { kind: "mallard"; contains: boolean }
  | { kind: "new"; path: string; git: boolean; via: boolean }
  | { kind: "draft"; path: string; draft: SetupDraftSummary }
  | { kind: "project"; path: string; project: LocalProjectSummary };

/** The read-only probe a real build would add: `create_setup_draft`'s own
 *  checks, answered without writing a draft. */
function judge(path: string, env: Env): Verdict {
  const entry = env.fs.get(path);
  if (!entry) return { kind: "missing" };
  const canonical = entry.kind === "link" ? entry.target ?? path : path;
  const resolved = env.fs.get(canonical);
  if (!resolved) return { kind: "missing" };
  if (resolved.kind === "file") return { kind: "file", parent: parentOf(canonical) ?? "/" };
  const data = `${env.home}/.mallard`;
  if (isWithin(canonical, data)) return { kind: "mallard", contains: false };
  if (isWithin(data, canonical)) return { kind: "mallard", contains: true };
  const project = env.projects.find((candidate) => candidate.canonical_project_root === canonical);
  if (project) return { kind: "project", path: canonical, project };
  const draft = env.drafts.find((candidate) => candidate.project_root === canonical);
  if (draft) return { kind: "draft", path: canonical, draft };
  return { kind: "new", path: canonical, git: Boolean(resolved.git), via: entry.kind === "link" };
}

/** The verdict as the field shows it: judged a beat after typing stops. */
function useVerdict(text: string, env: Env, searchable: boolean): Verdict {
  const reading = read(text, env.home);
  const [settled, setSettled] = useState(text);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(text), 320);
    return () => window.clearTimeout(timer);
  }, [text]);
  if (reading.kind === "empty") return { kind: "empty" };
  if (reading.kind === "dots") return { kind: "dots" };
  if (reading.kind === "search") return searchable ? { kind: "search", query: reading.query } : { kind: "relative" };
  if (settled !== text) return { kind: "checking", path: reading.path };
  return judge(reading.path, env);
}

/** A search that names exactly one live suggestion is a pick. */
function soleMatch(verdict: Verdict, env: Env): SuggestionRow | null {
  if (verdict.kind !== "search") return null;
  const rows = suggestionRows(env).filter((row) => row.state !== "missing" && matchesSearch(row, verdict.query, env));
  return rows.length === 1 ? rows[0] : null;
}

const committable = (verdict: Verdict) => verdict.kind === "new" || verdict.kind === "draft" || verdict.kind === "project";

/* ——— registry data ——————————————————————————————————————————————— */

const PROVIDERS: ProviderDescriptor[] = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", default_profile_directory: ".codex", setup_default: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", default_profile_directory: ".claude", setup_default: true, maintenance_actions: [] },
];

function profilesFor(home: string): ProviderProfileSummary[] {
  const profile = (id: string, provider: string, name: string, path: string): ProviderProfileSummary => ({
    profile_id: id, provider, display_name: name, path, canonical_path: path, revision: 1, created_at: 0, updated_at: 0,
    available: true, readable: true, writable: true, used_by_projects: [],
  });
  return [profile("p-codex", "codex", "Codex", `${home}/.codex`), profile("p-claude", "claude", "Claude", `${home}/.claude`)];
}

function projectsFor(home: string): LocalProjectSummary[] {
  const project = (id: string, rel: string, profiles: Record<string, string>, storages: string[]): LocalProjectSummary => ({
    local_project_id: id, bundle_id: `b-${id}`, display_name: baseName(rel), revision: 1,
    project_root: `${home}/${rel}`, canonical_project_root: `${home}/${rel}`,
    profile_ids: profiles, linked_storage_ids: storages, is_git_repository: true,
  });
  return [
    project("blog", "code/blog", { codex: "p-codex", claude: "p-claude" }, ["s-r2"]),
    project("website", "sites/website", { claude: "p-claude" }, []),
  ];
}

const draftFor = (id: string, root: string): SetupDraftSummary => ({
  draft_id: id, display_name: baseName(root), project_root: root, updated_at: 0, revision: 1, status: "draft",
});

const STORAGES: StorageSummary[] = [
  { storage_id: "s-r2", name: "Team R2", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [], settings: {}, secrets: [] },
  { storage_id: "s-local", name: "Backup drive", provider: "local", config_version: 1, provider_label: "Local folder", icon: "drive", available: true, summary: [], settings: {}, secrets: [] },
];

/* C's source: each session's authorized home, grouped. */
interface AgentFolder { rel: string; providers: string[]; sessions: number; when: string }
const AGENT_FOLDERS: AgentFolder[] = [
  { rel: "code/mallard-app", providers: ["codex", "claude"], sessions: 59, when: "Today" },
  { rel: "code/api", providers: ["codex"], sessions: 12, when: "Yesterday" },
  { rel: "code/blog", providers: ["codex", "claude"], sessions: 31, when: "3 days ago" },
  { rel: "notes", providers: ["claude"], sessions: 4, when: "2 weeks ago" },
  { rel: "sites/website", providers: ["claude"], sessions: 8, when: "Last month" },
  { rel: "code/old-prototype", providers: ["codex"], sessions: 22, when: "March" },
];

/* ——— the specimen's knobs ———————————————————————————————————————— */

type Base = "before" | "a" | "b";
type Layer = "suggest" | "complete" | "drop";
type Stock = "some" | "none";
type Scan = "done" | "loading" | "empty";

interface Knobs { base: Base; layers: Layer[]; platform: Platform; stock: Stock; scan: Scan; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  const layers = (query.get("layers") ?? "suggest").split(",").filter((layer): layer is Layer => ["suggest", "complete", "drop"].includes(layer));
  return {
    base: pick("base", ["before", "a", "b"] as const, "a"),
    layers,
    platform: pick("platform", ["mac", "linux"] as const, "mac"),
    stock: pick("stock", ["some", "none"] as const, "some"),
    scan: pick("scan", ["done", "loading", "empty"] as const, "done"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

function writeKnobs(knobs: Knobs) {
  const query = new URLSearchParams({
    base: knobs.base, layers: knobs.layers.join(","), platform: knobs.platform, stock: knobs.stock, scan: knobs.scan, theme: knobs.theme,
  });
  window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
  document.documentElement.dataset.theme = knobs.theme;
}

/* ——— the folder field ——————————————————————————————————————————— */

function hintFor(verdict: Verdict, env: Env, actions: {
  useFolder: (path: string) => void;
  anotherProfile?: (path: string) => void;
}): { tone: "mute" | "ok" | "warn" | "bad"; body: ReactNode } | null {
  switch (verdict.kind) {
    case "empty": return { tone: "mute", body: <>Type or paste a path, or Browse. <code>~</code> is your home folder.</> };
    case "search": return null;
    case "relative": return { tone: "mute", body: <>A path starts with <code>~/</code> or <code>/</code>.</> };
    case "dots": return { tone: "mute", body: <>Write the path without <code>.</code> or <code>..</code> parts.</> };
    case "checking": return { tone: "mute", body: <>Checking <code>{compact(verdict.path, env.home)}</code></> };
    case "missing": return { tone: "bad", body: <>No folder at this path yet.</> };
    case "file": return {
      tone: "bad",
      body: <>This is a file.<button type="button" className="btn-link" onClick={() => actions.useFolder(verdict.parent)}>Use {compact(verdict.parent, env.home)}</button></>,
    };
    case "mallard": return {
      tone: "bad",
      body: verdict.contains
        ? <>This folder holds Mallard’s own data, <code>~/.mallard</code>. Choose a folder inside it.</>
        : <>This is Mallard’s own data folder.</>,
    };
    case "new": return {
      tone: "ok",
      body: verdict.via
        ? <>{verdict.git ? "Git repository" : "Folder"} · resolves to <code>{compact(verdict.path, env.home)}</code></>
        : <>{verdict.git ? "Git repository" : "Folder · not a Git repository"}</>,
    };
    case "draft": return { tone: "ok", body: <>Setup already started for this folder.</> };
    case "project": return {
      tone: "ok",
      body: (
        <>
          Already added as “{verdict.project.display_name}”.
          {actions.anotherProfile && (
            <button type="button" className="btn-link" onClick={() => actions.anotherProfile?.(verdict.path)}>Set it up with another agent profile</button>
          )}
        </>
      ),
    };
  }
}

interface Completion { label: string; value: string; git: boolean }

function completionsFor(text: string, env: Env): Completion[] {
  const slash = text.lastIndexOf("/");
  if (slash < 0 || !(text.startsWith("/") || text.startsWith("~"))) return [];
  const typedParent = text.slice(0, slash + 1);
  const prefix = text.slice(slash + 1).toLowerCase();
  const reading = read(typedParent, env.home);
  if (reading.kind !== "path") return [];
  const parent = reading.path;
  if (env.fs.get(parent)?.kind !== "dir") return [];
  return [...env.fs.entries()]
    .filter(([path, entry]) => parentOf(path) === parent && path !== parent && entry.kind !== "file")
    .map(([path, entry]) => ({ name: baseName(path), entry }))
    .filter(({ name }) => name.toLowerCase().startsWith(prefix) && (prefix.startsWith(".") || !name.startsWith(".")))
    .sort((left, right) => left.name.localeCompare(right.name))
    .slice(0, 8)
    .map(({ name, entry }) => ({
      label: `${name}/`,
      value: `${typedParent}${name}/`,
      git: Boolean(entry.git || (entry.kind === "link" && env.fs.get(entry.target ?? "")?.git)),
    }));
}

function FolderField({
  id, label, text, verdict, env, complete, autoFocus, browseLabel = "Browse…", placeholder, onText, onEnter, onBrowse, onBlur, anotherProfile,
}: {
  id: string;
  label?: string;
  text: string;
  verdict: Verdict;
  env: Env;
  complete: boolean;
  autoFocus?: boolean;
  browseLabel?: string;
  placeholder: string;
  onText: (text: string) => void;
  onEnter: () => void;
  onBrowse: () => void;
  onBlur?: () => void;
  anotherProfile?: (path: string) => void;
}) {
  const [focused, setFocused] = useState(false);
  const [active, setActive] = useState(-1);
  const [dismissed, setDismissed] = useState(false);
  const options = useMemo(() => (complete ? completionsFor(text, env) : []), [complete, text, env]);
  const open = complete && focused && !dismissed && options.length > 0
    && !(options.length === 1 && options[0].value === text);
  useEffect(() => { setActive(-1); setDismissed(false); }, [text]);
  const hint = hintFor(verdict, env, { useFolder: (path) => onText(compact(path, env.home)), anotherProfile });
  const accept = (option: Completion) => { onText(option.value); setActive(-1); };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (open && event.key === "Tab" && !event.shiftKey) {
      event.preventDefault();
      accept(options[Math.max(active, 0)]);
    } else if (open && event.key === "ArrowDown") {
      event.preventDefault();
      setActive((current) => (current + 1) % options.length);
    } else if (open && event.key === "ArrowUp") {
      event.preventDefault();
      setActive((current) => (current <= 0 ? options.length - 1 : current - 1));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (open && active >= 0) accept(options[active]);
      else onEnter();
    } else if (event.key === "Escape" && open) {
      event.stopPropagation();
      setDismissed(true);
    }
  };
  const bad = hint?.tone === "bad";
  return (
    <div className={`pa-field${label ? "" : " pa-dialog-field"}`} style={label ? { ["--pa-label" as string]: "21px" } : undefined}>
      {label && <label className="pa-field-label" htmlFor={id}>{label}</label>}
      <div className="sync-profile-select-row">
        <input
          id={id}
          value={text}
          placeholder={placeholder}
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          autoFocus={autoFocus}
          aria-label={label ? undefined : "Project folder"}
          aria-invalid={bad || undefined}
          aria-describedby={`${id}-hint`}
          aria-autocomplete={complete ? "list" : undefined}
          aria-expanded={complete ? open : undefined}
          onChange={(event) => onText(event.target.value)}
          onKeyDown={onKeyDown}
          onFocus={() => setFocused(true)}
          onBlur={() => { window.setTimeout(() => setFocused(false), 120); onBlur?.(); }}
        />
        <button type="button" className="btn" onClick={onBrowse}>
          <Icon name="folder" size={13} /> {browseLabel}
        </button>
      </div>
      {open && (
        <ul className="pa-complete" role="listbox" aria-label="Folders">
          {options.map((option, index) => (
            <li key={option.value}>
              <button
                type="button"
                role="option"
                aria-selected={index === active}
                className={index === active ? "active" : undefined}
                onMouseDown={(event) => { event.preventDefault(); accept(option); }}
              >
                <Icon name={option.git ? "git-folder" : "folder"} size={13} />
                {option.label}
                {index === Math.max(active, 0) && <small>Tab</small>}
              </button>
            </li>
          ))}
          <li className="pa-complete-foot">Tab completes · ↑↓ choose · Enter uses the path</li>
        </ul>
      )}
      {hint && (
        <p id={`${id}-hint`} className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">{hint.body}</p>
      )}
    </div>
  );
}

/* ——— C: folders the agents have worked in ————————————————————————— */

type SuggestionRow = AgentFolder & { path: string; state: "new" | "draft" | "added" | "missing" };

/** The one search rule, for the list and for Enter alike. */
function matchesSearch(row: SuggestionRow, query: string, env: Env): boolean {
  const needle = query.toLowerCase();
  return baseName(row.path).toLowerCase().includes(needle) || compact(row.path, env.home).toLowerCase().includes(needle);
}

function suggestionRows(env: Env): SuggestionRow[] {
  return AGENT_FOLDERS.map((folder) => {
    const path = `${env.home}/${folder.rel}`;
    const verdict = judge(path, env);
    const state = verdict.kind === "project" ? "added" : verdict.kind === "draft" ? "draft" : verdict.kind === "missing" ? "missing" : "new";
    return { ...folder, path, state };
  });
}

function Suggestions({ env, text, scan, onPick }: {
  env: Env;
  text: string;
  scan: Scan;
  onPick: (row: SuggestionRow) => void;
}) {
  const reading = read(text, env.home);
  const rows = suggestionRows(env).filter((row) => {
    if (reading.kind === "search") return matchesSearch(row, reading.query, env);
    if (reading.kind === "path") return isWithin(row.path, reading.path) || row.path.startsWith(reading.path);
    return true;
  });
  if (scan === "empty") return null;
  return (
    <section className="pa-suggest" aria-label="Folders your agents have worked in">
      <div className="pa-suggest-head">
        Where your agents have worked
        {scan === "done" && <small>{rows.length === AGENT_FOLDERS.length ? `${rows.length} folders` : `${rows.length} of ${AGENT_FOLDERS.length}`}</small>}
      </div>
      {scan === "loading" ? (
        <div className="pa-suggest-state"><span className="status-loader" /> Reading Codex and Claude history…</div>
      ) : rows.length === 0 ? (
        <div className="pa-suggest-state">{reading.kind === "path" ? "No folder here matches. Enter uses the path as typed." : "No folder here matches. Type a path, or Browse."}</div>
      ) : (
        <ul className="sync-storage-link-list">
          {rows.map((row) => {
            const name = baseName(row.path);
            const git = env.fs.get(row.path)?.git;
            return (
              <li key={row.path}>
                <button
                  type="button"
                  className="sync-storage-index-row"
                  disabled={row.state === "missing"}
                  onClick={() => onPick(row)}
                  aria-label={`${row.state === "added" ? "Open" : row.state === "draft" ? "Resume" : "Add"} ${name}, ${compact(row.path, env.home)}`}
                >
                  <span className="sync-storage-link-mark" aria-hidden="true">
                    <Icon name={git ? "git-folder" : "folder"} size={13} />
                  </span>
                  <span className="sync-storage-index-copy">
                    <strong>{name}</strong>
                    <small className="path">
                      {compact(row.path, env.home)}
                      <span className="pa-facts"> · {row.state === "missing" ? "folder not found" : `${row.sessions} sessions · ${row.when}`}</span>
                    </small>
                  </span>
                  <span className="pa-row-marks">
                    {row.providers.map((provider) => (
                      <ProviderMark key={provider} provider={provider} displayName={provider === "codex" ? "Codex" : "Claude"} size={12} />
                    ))}
                  </span>
                  {row.state === "added" ? <span className="sync-draft-badge">Added</span>
                    : row.state === "draft" ? <span className="sync-draft-badge">Draft</span>
                      : <span />}
                  <Icon name="chevron-right" size={12} className="sync-storage-index-open" aria-hidden="true" />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/* ——— the system dialog, drawn foreign ——————————————————————————— */

function SystemDialog({ env, start, onPick, onCancel }: {
  env: Env;
  start: string;
  onPick: (path: string) => void;
  onCancel: () => void;
}) {
  const [folder, setFolder] = useState(start);
  const [selected, setSelected] = useState<string | null>(null);
  const children = [...env.fs.entries()]
    .filter(([path]) => parentOf(path) === folder && path !== folder && !baseName(path).startsWith("."))
    .sort(([left], [right]) => left.localeCompare(right));
  const gtk = env.platform === "linux";
  const choose = () => onPick(selected ?? folder);
  const cancelButton = <button type="button" onClick={onCancel}>Cancel</button>;
  const openButton = <button type="button" className="primary" onClick={choose}>{gtk ? "Select" : "Open"}</button>;
  return (
    <>
      <span className="pa-os-label">{gtk ? "GTK file chooser (Tauri’s default on Linux)" : "macOS open panel"} — outside Mallard, nothing typed in the app</span>
      <div className={`pa-os${gtk ? " gtk" : ""}`} role="dialog" aria-label="System folder dialog">
        <div className="pa-os-bar">
          {gtk ? <>{cancelButton}<strong>Select Folder</strong>{openButton}</> : <><span className="dots"><i /><i /><i /></span><strong>Open</strong></>}
        </div>
        <div className="pa-os-crumb">
          {parentOf(folder) && <button type="button" className="btn-link" onClick={() => { setFolder(parentOf(folder) ?? "/"); setSelected(null); }}>‹ </button>}
          {compact(folder, env.home)}
        </div>
        <ul>
          {children.map(([path, entry]) => (
            <li key={path}>
              <button
                type="button"
                className={selected === path ? "sel" : undefined}
                disabled={entry.kind === "file"}
                onClick={() => setSelected(path)}
                onDoubleClick={() => { if (entry.kind === "dir") { setFolder(path); setSelected(null); } }}
              >
                {entry.kind === "file" ? "📄" : "📁"} {baseName(path)}
              </button>
            </li>
          ))}
        </ul>
        {!gtk && <div className="pa-os-foot">{cancelButton}{openButton}</div>}
      </div>
    </>
  );
}

/* ——— the setup page, in its shipped classes ————————————————————— */

function SetupBands({ disabled }: { disabled: boolean }) {
  return (
    <div className={disabled ? "pa-waiting" : undefined} inert={disabled || undefined}>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>Agent</strong></div></div>
        <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
          {PROVIDERS.map((descriptor) => (
            <div key={descriptor.key} className="sync-provider-card enabled">
              <label className="sync-provider-toggle setup-marked-toggle">
                <input type="checkbox" defaultChecked aria-label={`Use ${descriptor.display_name} in this project`} />
                <ProviderMark provider={descriptor.key} displayName={descriptor.display_name} size={12} />
                <strong>{descriptor.display_name}</strong>
                <span>{descriptor.profile_label}</span>
              </label>
              <div className="sync-agent-profile-list">
                <div className="sync-agent-profile-row">
                  <span className="sync-select-shell">
                    <select defaultValue="p" aria-label={`${descriptor.display_name} profile`}>
                      <option value="p">{descriptor.display_name}</option>
                      <option value="__pending">Custom path…</option>
                    </select>
                  </span>
                  <button type="button" className="btn"><Icon name="folder" size={13} /> Browse</button>
                </div>
              </div>
            </div>
          ))}
        </div>
      </section>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>Storage</strong></div></div>
        <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
          <button type="button" role="radio" aria-checked className="setup-pill active">
            <Icon name="computer" size={12} /><span className="setup-pill-label">None yet</span>
            <span className="setup-pill-check" aria-hidden="true"><Icon name="check" size={12} /></span>
          </button>
          {STORAGES.map((storage) => (
            <button key={storage.storage_id} type="button" role="radio" aria-checked={false} className="setup-pill">
              <Icon name={storage.icon === "cloud" ? "cloud" : "drive"} size={12} /><span className="setup-pill-label">{storage.name}</span>
              <span className="setup-pill-check" aria-hidden="true" />
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}

function SetupPage({ title, project, after, footer }: {
  title: string;
  project: ReactNode;
  after?: ReactNode;
  footer: ReactNode;
}) {
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section sync-project-overview" aria-label="Project setup">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby="pa-setup-title">
          <header className="sync-modal-header">
            <div><h1 id="pa-setup-title">{title}</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close setup"><Icon name="x" size={17} /></button>
          </header>
          <div className="sync-modal-body">
            {project}
            {after}
          </div>
          <footer className="sync-modal-footer sync-setup-footer">{footer}</footer>
        </section>
      </section>
    </main>
  );
}

/* ——— one live frame ————————————————————————————————————————————— */

type Screen =
  | { kind: "landing" }
  | { kind: "sheet" }
  | { kind: "form" }
  | { kind: "setup"; draftId: string }
  | { kind: "project"; projectId: string };

interface Pending { pick: (path: string) => void; start: string }

function Frame({ knobs, fillRef, dropRef }: {
  knobs: Knobs;
  fillRef: { current: ((text: string) => void) | null };
  dropRef: { current: (() => void) | null };
}) {
  const home = HOMES[knobs.platform];
  const fs = useMemo(() => machine(knobs.platform), [knobs.platform]);
  const profiles = useMemo(() => profilesFor(home), [home]);
  const projects = useMemo(() => (knobs.stock === "some" ? projectsFor(home) : []), [home, knobs.stock]);
  const [drafts, setDrafts] = useState<SetupDraftSummary[]>(() => (knobs.stock === "some" ? [draftFor("d-api", `${home}/code/api`)] : []));
  const [screen, setScreen] = useState<Screen>({ kind: "landing" });
  const [text, setText] = useState("");
  const [setupText, setSetupText] = useState("");
  const [system, setSystem] = useState<Pending | null>(null);
  const [dropping, setDropping] = useState(false);
  const env: Env = useMemo(() => ({ platform: knobs.platform, home, fs, projects, drafts }), [knobs.platform, home, fs, projects, drafts]);
  const layers = new Set(knobs.layers);
  const suggest = layers.has("suggest") && knobs.base !== "before";
  const complete = layers.has("complete") && knobs.base !== "before";
  const verdict = useVerdict(text, env, suggest);
  const sole = soleMatch(verdict, env);
  const activeDraft = screen.kind === "setup" ? drafts.find((draft) => draft.draft_id === screen.draftId) ?? null : null;
  const setupVerdict = useVerdict(setupText, env, false);
  const placeholder = knobs.platform === "mac" ? "~/code/my-app or /Users/you/code/my-app" : "~/code/my-app or /home/you/code/my-app";

  /* The only place a draft is born: a committed folder, never a keystroke. */
  const commit = (target: Verdict) => {
    if (target.kind === "new") {
      const id = `d-${Date.now()}`;
      setDrafts((current) => [...current, draftFor(id, target.path)]);
      setSetupText(compact(target.path, home));
      setScreen({ kind: "setup", draftId: id });
    } else if (target.kind === "draft") {
      setSetupText(compact(target.path, home));
      setScreen({ kind: "setup", draftId: target.draft.draft_id });
    } else if (target.kind === "project") {
      setScreen({ kind: "project", projectId: target.project.local_project_id });
    }
  };
  const commitPath = (path: string) => commit(judge(path, env));
  const anotherProfile = (path: string) => commit({ kind: "new", path, git: Boolean(fs.get(path)?.git), via: false });

  const addProject = () => {
    setText("");
    if (knobs.base === "before") {
      setSystem({ start: `${home}/code`, pick: (path) => commitPath(path) });
    } else {
      setScreen({ kind: knobs.base === "a" ? "sheet" : "form" });
    }
  };
  const browse = (then: (path: string) => void) => setSystem({ start: `${home}/code`, pick: then });

  fillRef.current = (value: string) => {
    if (screen.kind === "landing") setScreen({ kind: knobs.base === "b" ? "form" : "sheet" });
    if (screen.kind === "setup") setSetupText(value);
    else setText(value);
  };
  dropRef.current = () => {
    setDropping(true);
    window.setTimeout(() => {
      setDropping(false);
      const path = `${home}/code/scratch`;
      /* A drop is a Browse result: the sheet opens filled in for review; the
         form has no review step, so the folder is committed. */
      if (knobs.base === "a") { setText(compact(path, home)); setScreen({ kind: "sheet" }); }
      else if (knobs.base === "b") commitPath(path);
    }, 900);
  };

  const sidebar = (
    <ProjectSidebar
      projects={projects}
      drafts={drafts}
      activeDraftId={activeDraft?.draft_id ?? null}
      storages={STORAGES}
      storageDrafts={[]}
      storageProviders={[]}
      storageUsage={{}}
      activeProjectId={screen.kind === "project" ? screen.projectId : null}
      activeStorageId={null}
      catalogScopes={[]}
      activeCatalogScopeId={null}
      addingCatalogScope={false}
      catalogIndexOpen={false}
      storageIndexOpen={false}
      settingsOpen={false}
      loading={false}
      busy={false}
      activityOpen={false}
      unreadLogs={0}
      onOpenProjects={() => setScreen({ kind: "landing" })}
      onSelectProject={(projectId) => setScreen({ kind: "project", projectId })}
      onRemoveProject={() => undefined}
      onSelectDraft={(draftId) => {
        const draft = drafts.find((candidate) => candidate.draft_id === draftId);
        if (draft) setSetupText(compact(draft.project_root, home));
        setScreen({ kind: "setup", draftId });
      }}
      onDiscardDraft={(draftId) => {
        setDrafts((current) => current.filter((draft) => draft.draft_id !== draftId));
        setScreen({ kind: "landing" });
      }}
      onToggleActivity={() => undefined}
      onAddProject={addProject}
      onOpenStorage={() => undefined}
      onOpenStorageDraft={() => undefined}
      onDiscardStorageDraft={() => undefined}
      onRemoveStorage={() => undefined}
      onAddStorage={() => undefined}
      onOpenCatalogIndex={() => undefined}
      onOpenCatalogScope={() => undefined}
      onAddCatalogScope={() => undefined}
      onRemoveCatalogScope={() => undefined}
      onOpenStorageIndex={() => undefined}
      onOpenSettings={() => undefined}
    />
  );

  const landing = knobs.base === "before" ? (
    <ProjectPicker projects={projects} profiles={profiles} providers={PROVIDERS} storages={STORAGES} onAddProject={addProject} onSelectProject={(projectId) => setScreen({ kind: "project", projectId })} />
  ) : (
    <LandingPicker
      title="Projects"
      count={projects.length === 1 ? "1 on this machine" : `${projects.length} on this machine`}
      actionLabel="Add project…"
      onAction={addProject}
      doors={[{
        key: "folder",
        icon: "folder",
        label: "Add a folder…",
        ariaLabel: "Add project: type, pick or drop a folder",
        marks: PROVIDERS.map((provider) => <ProviderMark key={provider.key} provider={provider.key} displayName={provider.display_name} />),
        onOpen: addProject,
      }]}
      caption={{ icons: ["cloud"], text: "Sessions & memory travel with it" }}
      cards={projects.map((project) => ({
        key: project.local_project_id,
        icon: "git-folder",
        title: project.display_name,
        marks: Object.keys(project.profile_ids ?? {}).map((provider) => (
          <ProviderMark key={provider} provider={provider} displayName={provider === "codex" ? "Codex" : "Claude"} />
        )),
        location: compact(project.project_root ?? "", home),
        detail: <><Icon name="storage" size={13} />{project.linked_storage_ids?.length ? "Team R2" : "No storage linked"}</>,
        detailMuted: !project.linked_storage_ids?.length,
        onOpen: () => setScreen({ kind: "project", projectId: project.local_project_id }),
      }))}
    />
  );

  /* The setup page once a draft exists: Folder is typable in every new base. */
  const setupPage = activeDraft && (() => {
    const folderRow = knobs.base === "before" ? (
      <label>
        <span>Folder</span>
        <div className="sync-profile-select-row">
          <input value={compact(activeDraft.project_root, home)} readOnly title={activeDraft.project_root} />
          <button type="button" className="btn" onClick={() => browse((path) => {
            setDrafts((current) => current.map((draft) => draft.draft_id === activeDraft.draft_id ? { ...draft, project_root: path, display_name: baseName(path) } : draft));
          })}>
            <Icon name="folder" size={13} /> Change…
          </button>
        </div>
      </label>
    ) : (
      <FolderField
        id="pa-setup-folder"
        label="Folder"
        text={setupText}
        verdict={setupVerdict.kind === "draft" && setupVerdict.draft.draft_id === activeDraft.draft_id
          ? { kind: "new", path: setupVerdict.path, git: Boolean(fs.get(setupVerdict.path)?.git), via: false }
          : setupVerdict}
        env={env}
        complete={complete}
        placeholder={placeholder}
        onText={setSetupText}
        onEnter={() => {
          /* The folder moves on Enter or a Browse result, never mid-word:
             every prefix of a real path is itself a folder. */
          if (setupVerdict.kind !== "new") return;
          const path = setupVerdict.path;
          setDrafts((current) => current.map((draft) => draft.draft_id === activeDraft.draft_id ? { ...draft, project_root: path, display_name: baseName(path) } : draft));
          setSetupText(compact(path, home));
        }}
        onBrowse={() => browse((path) => {
          setDrafts((current) => current.map((draft) => draft.draft_id === activeDraft.draft_id ? { ...draft, project_root: path, display_name: baseName(path) } : draft));
          setSetupText(compact(path, home));
        })}
      />
    );
    const name = (
      <label>
        <span>Name</span>
        <input value={activeDraft.display_name} readOnly />
      </label>
    );
    return (
      <SetupPage
        title={`Set up ${activeDraft.display_name}`}
        project={(
          <>
            <section className="sync-setup-section">
              <div className="sync-card-heading"><div><strong>Project</strong></div></div>
              <div className="sync-field-grid sync-setup-project-fields">
                {knobs.base === "b" ? <>{folderRow}{name}</> : <>{name}{folderRow}</>}
              </div>
            </section>
            <SetupBands disabled={false} />
          </>
        )}
        footer={(
          <div>
            <button type="button" className="btn btn-ghost" onClick={() => {
              setDrafts((current) => current.filter((draft) => draft.draft_id !== activeDraft.draft_id));
              setScreen({ kind: "landing" });
            }}>Discard</button>
            <button type="button" className="btn btn-primary">Finish locally</button>
          </div>
        )}
      />
    );
  })();

  /* The commit verb says what it will do. */
  const primary = verdict.kind === "draft" ? "Resume setup"
    : verdict.kind === "project" ? `Open ${verdict.project.display_name}`
      : sole?.state === "added" ? `Open ${baseName(sole.path)}`
        : sole?.state === "draft" ? "Resume setup"
          : "Continue";

  /* B: the form before a folder is committed — no draft, no sidebar row. */
  const formPage = screen.kind === "form" && (
    <SetupPage
      title="New project"
      project={(
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Project</strong></div></div>
          <div className="sync-field-grid sync-setup-project-fields">
            <FolderField
              id="pa-form-folder"
              label="Folder"
              text={text}
              verdict={verdict}
              env={env}
              complete={complete}
              autoFocus
              placeholder={suggest ? "Type a path or search your agents’ folders" : placeholder}
              onText={setText}
              onEnter={() => {
                if (committable(verdict)) commit(verdict);
                else if (sole) commitPath(sole.path);
              }}
              onBrowse={() => browse(commitPath)}
              anotherProfile={anotherProfile}
            />
          </div>
          {suggest && (
            <div className="pa-form-suggest">
              <Suggestions env={env} text={text} scan={knobs.scan} onPick={(row) => commitPath(row.path)} />
            </div>
          )}
        </section>
      )}
      after={<SetupBands disabled />}
      /* Blur never commits: Tab-ing out of "~/code/" would add ~/code. The
         footer's verb is the commit until a draft exists, then it is Finish. */
      footer={(
        <>
          <span>{committable(verdict) || sole ? "Enter ↵" : "Choose a folder to continue"}</span>
          <div>
            <button type="button" className="btn btn-ghost" onClick={() => setScreen({ kind: "landing" })}>Cancel</button>
            <button type="button" className="btn btn-primary" disabled={!committable(verdict) && !sole} onClick={() => (sole ? commitPath(sole.path) : commit(verdict))}>
              {primary === "Continue" ? "Use this folder" : primary}
            </button>
          </div>
        </>
      )}
    />
  );
  /* A: the sheet. */
  const sheet = screen.kind === "sheet" && (
    <>
      <div className="pa-backdrop" onClick={() => setScreen({ kind: "landing" })} />
      <dialog open className="sync-modal pa-add-dialog" aria-labelledby="pa-add-title" onKeyDown={(event) => { if (event.key === "Escape") setScreen({ kind: "landing" }); }}>
        <header className="sync-modal-header">
          <div><h1 id="pa-add-title">Add project</h1></div>
          <button type="button" className="btn btn-ghost" aria-label="Close" onClick={() => setScreen({ kind: "landing" })}><Icon name="x" size={17} /></button>
        </header>
        <div className="sync-modal-body">
          <FolderField
            id="pa-sheet-folder"
            text={text}
            verdict={verdict}
            env={env}
            complete={complete}
            autoFocus
            placeholder={suggest ? "Type a path or search the folders below" : placeholder}
            onText={setText}
            onEnter={() => {
              if (committable(verdict)) commit(verdict);
              else if (sole) commitPath(sole.path);
            }}
            onBrowse={() => browse((path) => setText(compact(path, home)))}
            anotherProfile={anotherProfile}
          />
          {suggest && <Suggestions env={env} text={text} scan={knobs.scan} onPick={(row) => commitPath(row.path)} />}
        </div>
        <footer className="sync-modal-footer">
          <span>{layers.has("drop") ? "You can also drop a folder on the window." : ""}</span>
          <div>
            <button type="button" className="btn" onClick={() => setScreen({ kind: "landing" })}>Cancel</button>
            <button type="button" className="btn btn-primary" disabled={!committable(verdict) && !sole} onClick={() => (sole ? commitPath(sole.path) : commit(verdict))}>{primary}</button>
          </div>
        </footer>
      </dialog>
    </>
  );

  const title = screen.kind === "setup" && activeDraft
    ? <><Icon name="folder" size={16} /><strong>{activeDraft.display_name}</strong><span>Draft setup</span></>
    : screen.kind === "form"
      ? <><Icon name="folder" size={16} /><strong>New project</strong></>
      : <><Icon name="folder" size={16} /><strong>Projects</strong></>;

  const project = screen.kind === "project" ? projects.find((candidate) => candidate.local_project_id === screen.projectId) : null;

  return (
    <div className="sync-app" style={{ ["--sync-sidebar-width" as string]: "260px" }}>
      {sidebar}
      <div className="sync-workspace">
        <header className="sync-titlebar"><div className="sync-titlebar-context">{title}</div></header>
        {screen.kind === "setup" ? setupPage
          : screen.kind === "form" ? formPage
            : screen.kind === "project" ? (
              <main className="sync-main"><div className="pa-placeholder"><strong>{project?.display_name}</strong>The project’s workspace opens here — not part of this specimen.</div></main>
            ) : (
              <main className="sync-main sync-project-links-page sync-landing-page">{landing}</main>
            )}
      </div>
      {sheet}
      {dropping && <div className="pa-drop"><span><Icon name="folder" size={18} /> Drop a folder to add it as a project</span></div>}
      {system && (
        <SystemDialog
          env={env}
          start={system.start}
          onCancel={() => setSystem(null)}
          onPick={(path) => { const then = system.pick; setSystem(null); then(path); }}
        />
      )}
    </div>
  );
}

/* ——— the static gallery ———————————————————————————————————————— */

function StaticField({ title, why, text, verdict, env, children }: {
  title: string;
  why?: string;
  text: string;
  verdict: Verdict;
  env: Env;
  children?: ReactNode;
}) {
  return (
    <div className="pa-cell">
      <h3>{title}</h3>
      <FolderField
        id={`pa-static-${title.replace(/\W+/g, "-")}`}
        text={text}
        verdict={verdict}
        env={env}
        complete={false}
        placeholder="~/code/my-app"
        onText={() => undefined}
        onEnter={() => undefined}
        onBrowse={() => undefined}
        anotherProfile={() => undefined}
      />
      {children}
      {why && <p className="pa-why">{why}</p>}
    </div>
  );
}

function Gallery({ env }: { env: Env }) {
  const home = env.home;
  const at = (rel: string) => `${home}/${rel}`;
  const blog = projectsFor(home)[0];
  return (
    <div className="pa-gallery">
      <StaticField title="Empty" text="" verdict={{ kind: "empty" }} env={env} />
      <StaticField title="Checking (a beat after typing stops)" text="~/code/mallard-app" verdict={{ kind: "checking", path: at("code/mallard-app") }} env={env} />
      <StaticField title="Git repository" text="~/code/mallard-app" verdict={judge(at("code/mallard-app"), env)} env={env} />
      <StaticField title="Plain folder" why="Allowed. Non-Git projects can also carry reviewed project files." text="~/notes" verdict={judge(at("notes"), env)} env={env} />
      <StaticField title="Symlink" why="create_setup_draft canonicalizes; the verdict says where it lands." text="~/app" verdict={judge(at("app"), env)} env={env} />
      <StaticField title="Pasted as file:// or 'quoted'" why="A file-manager copy or a terminal copy works as pasted." text={`file://${at("code/blog")}`} verdict={judge(at("code/blog"), env)} env={env} />
      <StaticField title="Nothing there" text="~/code/mallard-ap" verdict={{ kind: "missing" }} env={env} />
      <StaticField title="A file" why="One click to its folder instead of a dead end." text="~/code/mallard-app/README.md" verdict={judge(at("code/mallard-app/README.md"), env)} env={env} />
      <StaticField title="Home folder" why="Refused today too: paths_overlap(~, ~/.mallard)." text="~" verdict={judge(home, env)} env={env} />
      <StaticField title="Relative path (no search layer)" text="code/api" verdict={{ kind: "relative" }} env={env} />
      <StaticField title="Dot segments" why="validate_absolute_clean_path refuses . and .." text="~/code/../notes" verdict={{ kind: "dots" }} env={env} />
      <StaticField title="Setup already started" why="Not an error: the primary button reads Resume setup." text="~/code/api" verdict={judge(at("code/api"), env)} env={env} />
      <StaticField title="Already a project" why="Not a refusal: projects key on (folder, profile). Primary reads Open blog." text="~/code/blog" verdict={{ kind: "project", path: at("code/blog"), project: blog }} env={env} />
      <div className="pa-cell">
        <h3>Browse failed (the system dialog did not open)</h3>
        <div className="pa-field pa-dialog-field">
          <div className="sync-profile-select-row">
            <input value="" placeholder="~/code/my-app" readOnly aria-label="Project folder" />
            <button type="button" className="btn"><Icon name="folder" size={13} /> Browse…</button>
          </div>
          <p className="sync-setup-path-hint warn">The folder dialog didn’t open. Type or paste the path instead.</p>
        </div>
        <p className="pa-why">Today a failed open() leaves Add project doing nothing at all.</p>
      </div>
    </div>
  );
}

/* ——— the page ————————————————————————————————————————————————— */

const DIRECTIONS: Record<Base, { title: string; precedent: ReactNode; flow: string; backend: ReactNode; changes: ReactNode }> = {
  before: {
    title: "Before · today",
    precedent: <>—</>,
    flow: "Add project → system folder dialog → draft appears in the sidebar → setup page → Finish",
    backend: <>—</>,
    changes: <>No field to type or paste into. The draft exists the moment the dialog returns, so a wrong pick leaves a Draft row to discard. The setup page’s Folder is read-only; <em>Change…</em> reopens the dialog.</>,
  },
  a: {
    title: "A · Folder sheet",
    precedent: <>GitHub Desktop <em>Add Local Repository</em> (path field + Choose…, Add Repository).</>,
    flow: "Add project → sheet: type, paste or Browse → verdict as you type → Continue ↵ → setup page → Finish",
    backend: <>One read-only <code>probe_project_folder</code> — <code>create_setup_draft</code>’s checks without the write (canonical path, directory, Git, Mallard-data overlap, existing project or draft).</>,
    changes: <><code>beginAddProject</code> opens the sheet instead of <code>open()</code>; the draft is born on Continue. The setup page barely changes: its Folder row becomes this field (commits on Enter/Browse). Smallest base.</>,
  },
  b: {
    title: "B · Folder in the form",
    precedent: <>JetBrains <em>New Project</em>: Name and a Location you “type or select” on one form.</>,
    flow: "Add project → setup page opens at once, Folder first → Enter ↵ or Use this folder → the rest wakes → Finish",
    backend: <>Same probe as A.</>,
    changes: <>One page, one stop fewer. <code>ProjectSetupWorkspace</code> is keyed by <code>draftId</code> and loads it on mount, so it needs a pre-draft state: the page renders before any draft exists and calls <code>createSetupDraft</code> at the folder commit.</>,
  },
};

const LAYERS: Record<Layer, { title: string; precedent: ReactNode; backend: ReactNode }> = {
  suggest: {
    title: "C · Suggested folders",
    precedent: <>VS Code <em>Open Recent</em>; JetBrains Welcome <em>Recent Projects</em> + search. Mallard already knows where the agents ran.</>,
    backend: <>New <code>list_agent_folders</code>: a bounded head read of every session file under every registered profile, grouped by <code>scan_metadata().authorized_home</code>. Needs a loading state; a fresh machine gets no list (the base alone).</>,
  },
  complete: {
    title: "D · Completion",
    precedent: <>VS Code simple file dialog (<code>files.simpleDialog.enable</code>); Zed’s built-in keyboard-first pickers (<code>use_system_path_prompts: false</code>); shell Tab.</>,
    backend: <>New bounded <code>list_child_folders(path)</code>: no-follow, directories only, capped.</>,
  },
  drop: {
    title: "E · Drop a folder",
    precedent: <>GitHub Desktop (drag onto the window); JetBrains <em>Open</em>.</>,
    backend: <>None — Tauri’s drag-drop event carries the paths. A drop is a Browse result.</>,
  },
};

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  useEffect(() => writeKnobs(knobs), [knobs]);
  const fillRef = useRef<((text: string) => void) | null>(null);
  const dropRef = useRef<(() => void) | null>(null);
  const home = HOMES[knobs.platform];
  const galleryEnv: Env = useMemo(() => ({
    platform: knobs.platform, home, fs: machine(knobs.platform), projects: projectsFor(home), drafts: [draftFor("d-api", `${home}/code/api`)],
  }), [knobs.platform, home]);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => setKnobs((current) => ({ ...current, [key]: value }));
  const toggleLayer = (layer: Layer) => setKnobs((current) => ({
    ...current,
    layers: current.layers.includes(layer) ? current.layers.filter((candidate) => candidate !== layer) : [...current.layers, layer],
  }));
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string, disabled = false) => (
    <button type="button" className="pa-seg" aria-pressed={knobs[key] === value} disabled={disabled} onClick={() => set(key, value)}>{label}</button>
  );
  const direction = DIRECTIONS[knobs.base];
  const tries = knobs.base === "before" ? [] : [
    "~/code/mallard-app", "~/notes", "~/app", "~/code/api", "~/code/blog", "~/code/mallard-app/README.md", "~", "mall", `file://${home}/code/mallet`, "~/code/m",
  ];
  const frameKey = JSON.stringify({ ...knobs, theme: undefined });
  return (
    <div className="pa-spec">
      <h1 className="pa-title">Add project — typed paths, a natural flow</h1>

      <div className="pa-controls">
        <div className="pa-group"><span>Base</span>{seg("base", "before", "Before")}{seg("base", "a", "A · Sheet")}{seg("base", "b", "B · In the form")}</div>
        <div className="pa-group"><span>Layers</span>
          {(["suggest", "complete", "drop"] as Layer[]).map((layer) => (
            <button key={layer} type="button" className="pa-seg" aria-pressed={knobs.layers.includes(layer)} disabled={knobs.base === "before"} onClick={() => toggleLayer(layer)}>
              {LAYERS[layer].title}
            </button>
          ))}
        </div>
        <div className="pa-group"><span>Machine</span>{seg("platform", "mac", "macOS")}{seg("platform", "linux", "Linux")}</div>
        <div className="pa-group"><span>Projects</span>{seg("stock", "some", "Some")}{seg("stock", "none", "None yet")}</div>
        <div className="pa-group"><span>History scan</span>{seg("scan", "done", "Done", !knobs.layers.includes("suggest"))}{seg("scan", "loading", "Loading", !knobs.layers.includes("suggest"))}{seg("scan", "empty", "Nothing", !knobs.layers.includes("suggest"))}</div>
        <div className="pa-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="pa-card">
        <strong>{direction.title}</strong>
        <div className="pa-flow">{direction.flow}</div>
        <dl>
          <dt>Precedent</dt><dd>{direction.precedent}</dd>
          <dt>Backend</dt><dd>{direction.backend}</dd>
          <dt>What changes</dt><dd>{direction.changes}</dd>
          {knobs.base !== "before" && knobs.layers.map((layer) => (
            <FragmentRow key={layer} title={LAYERS[layer].title} body={<>{LAYERS[layer].precedent} <em>Backend:</em> {LAYERS[layer].backend}</>} />
          ))}
        </dl>
      </div>

      <div className="pa-try">
        <span>{knobs.base === "before" ? "Click Add project — the system dialog is the only way in." : "Try (fills the field):"}</span>
        {tries.map((value) => (
          <button key={value} type="button" onClick={() => fillRef.current?.(value)}>{value}</button>
        ))}
        {knobs.base !== "before" && knobs.layers.includes("drop") && (
          <button type="button" className="act" onClick={() => dropRef.current?.()}>Drop ~/code/scratch on the window</button>
        )}
      </div>

      <div className="pa-frame">
        <Frame key={frameKey} knobs={knobs} fillRef={fillRef} dropRef={dropRef} />
      </div>

      <h2 className="pa-h2">Every verdict the field gives</h2>
      <p>Judged {`~320 ms`} after typing stops, by the same checks <code>create_setup_draft</code> runs. Nothing is written until Enter, Continue, a Browse result, a picked row or a drop.</p>
      <Gallery env={galleryEnv} />

      <h2 className="pa-h2">Entry points that change</h2>
      <ul className="pa-notes">
        <li>Projects landing head <em>Add project…</em> and the empty landing door (<em>Pick a folder…</em> → <em>Add a folder…</em>) — <code>ProjectPicker.tsx</code></li>
        <li>Sidebar rail <em>+</em> and the empty rail door (<em>Add project: pick a folder</em>) — <code>ProjectSidebar.tsx</code></li>
        <li>Project menu <em>Add project…</em> and the two buttons on the links page — <code>ProjectLinksWorkspace.tsx</code></li>
        <li>All route through <code>ProjectSync.beginAddProject</code>, which today calls <code>open()</code> first. The setup page’s <em>Change…</em> (<code>changeProjectFolder</code>) becomes the field.</li>
        <li>Linux: <code>compactProjectPath</code> only shortens <code>/Users/&lt;name&gt;</code>, so <code>/home/&lt;name&gt;</code> paths print in full — fix alongside.</li>
      </ul>
    </div>
  );
}

function FragmentRow({ title, body }: { title: string; body: ReactNode }) {
  return <><dt>{title}</dt><dd>{body}</dd></>;
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
