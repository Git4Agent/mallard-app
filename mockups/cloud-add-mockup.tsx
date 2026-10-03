import { StrictMode, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { formatFullDateTime, formatRelativeTime } from "../src/clock";
import Icon, { type IconName } from "../src/components/Icons";
import { FieldMessage } from "../src/components/notice/Notice";
import Notice from "../src/components/notice/Notice";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import "../src/App.css";

/**
 * Specimen: one Mallard Cloud picker for pets and projects — round 2, the
 * style pass.
 *
 * Round 1 settled the flow (owner, 2026-10-02): one field that filters,
 * looks up an address or a pasted link, and joins with a code; three scopes
 * — Shared with you / Yours, Starred, Public; rows with a tile, the address,
 * the description and role · stars · age; the shipped found row and footer.
 *
 * Round 2 draws that flow three ways, each from parts the app already
 * ships, beside the dialog as shipped:
 *
 * - S1 · Tabs + rows: the review strip's tabs over the connect dialog's
 *   choice rows; the pet's tile is the Catalog card's ink-boxed preview.
 * - S2 · Pills + cards: the setup page's pill switch over the Catalog row
 *   page's own cards, Add on each card.
 * - S3 · Headings + rows (recommended): no switch — the row page's section
 *   headings with their count chips, one boxed list under each, Public last
 *   with its sort in the heading and the Activity page's Load-more foot.
 *
 * Every class but `.cloud-add-*` is shipped. `npm run dev`, then
 * /mockups/cloud-add-mockup.html (`?scene=…`, `?style=rows|cards|headings`,
 * `?theme=dark`, `?field=two`, `?previews=off`).
 */

/* ——— data ————————————————————————————————————————————————————————— */

const LOGIN = "Rika321";
const HOST = "mallard-ai.com";
const now = Math.floor(Date.now() / 1000);
const HOUR = 3_600;
const DAY = 86_400;
const capitalize = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

const art = (body: string) => `data:image/svg+xml;utf8,${encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 52">${body}</svg>`,
)}`;
const ART: Record<string, string> = {
  Biscuit: art(`<ellipse cx="11" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="37" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="24" cy="31" rx="14" ry="13" fill="#d9a066"/><ellipse cx="24" cy="38" rx="7" ry="5" fill="#f3e1c4"/><circle cx="19" cy="27" r="2" fill="#2a2a2a"/><circle cx="29" cy="27" r="2" fill="#2a2a2a"/><ellipse cx="24" cy="34.5" rx="2.6" ry="1.8" fill="#2a2a2a"/>`),
  Pebble: art(`<ellipse cx="24" cy="33" rx="17" ry="13" fill="#9aa3ad"/><ellipse cx="24" cy="30" rx="13" ry="8" fill="#b7bfc7"/><circle cx="18" cy="30" r="2" fill="#2a2a2a"/><circle cx="30" cy="30" r="2" fill="#2a2a2a"/><path d="M21 35q3 2.4 6 0" stroke="#4b535b" stroke-width="1.6" fill="none"/>`),
  Mochi: art(`<ellipse cx="24" cy="32" rx="17" ry="14" fill="#f6c6d4"/><circle cx="18" cy="30" r="1.8" fill="#2a2a2a"/><circle cx="30" cy="30" r="1.8" fill="#2a2a2a"/><ellipse cx="14" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><ellipse cx="34" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><path d="M22 34q2 1.6 4 0" stroke="#a24d68" stroke-width="1.4" fill="none"/>`),
  Tofu: art(`<rect x="9" y="18" width="30" height="24" rx="4" fill="#f7f3e8" stroke="#d8d0bf" stroke-width="2"/><circle cx="19" cy="29" r="1.8" fill="#2a2a2a"/><circle cx="29" cy="29" r="1.8" fill="#2a2a2a"/><path d="M21 35q3 2 6 0" stroke="#8a7f66" stroke-width="1.4" fill="none"/>`),
  Sprocket: art(`<circle cx="24" cy="30" r="13" fill="#8f9aa8"/><circle cx="24" cy="30" r="5" fill="#f4f6f8"/><g fill="#8f9aa8"><rect x="22" y="12" width="4" height="7"/><rect x="22" y="41" width="4" height="7"/><rect x="6" y="28" width="7" height="4"/><rect x="35" y="28" width="7" height="4"/></g><circle cx="21" cy="28" r="1.4" fill="#2a2a2a"/><circle cx="27" cy="28" r="1.4" fill="#2a2a2a"/>`),
  Pixel: art(`<rect x="12" y="16" width="24" height="22" rx="5" fill="#5b8def"/><rect x="17" y="22" width="5" height="5" fill="#e9f1ff"/><rect x="26" y="22" width="5" height="5" fill="#e9f1ff"/><rect x="19" y="31" width="10" height="2" fill="#e9f1ff"/><rect x="22" y="9" width="4" height="7" fill="#5b8def"/><circle cx="24" cy="8" r="2.5" fill="#f4c542"/><rect x="15" y="38" width="6" height="6" fill="#3f6fd1"/><rect x="27" y="38" width="6" height="6" fill="#3f6fd1"/>`),
  Clover: art(`<circle cx="18" cy="24" r="7" fill="#5fae5a"/><circle cx="30" cy="24" r="7" fill="#5fae5a"/><circle cx="18" cy="34" r="7" fill="#5fae5a"/><circle cx="30" cy="34" r="7" fill="#5fae5a"/><path d="M24 36v10" stroke="#2f6e2c" stroke-width="2.4"/><circle cx="21" cy="28" r="1.4" fill="#2a2a2a"/><circle cx="27" cy="28" r="1.4" fill="#2a2a2a"/>`),
  Nimbus: art(`<ellipse cx="24" cy="32" rx="16" ry="9" fill="#dfe7f2"/><circle cx="17" cy="27" r="7" fill="#dfe7f2"/><circle cx="27" cy="24" r="9" fill="#dfe7f2"/><circle cx="21" cy="30" r="1.6" fill="#2a2a2a"/><circle cx="29" cy="30" r="1.6" fill="#2a2a2a"/><path d="M22 35q3 2 6 0" stroke="#7b8796" stroke-width="1.4" fill="none"/>`),
  Waffle: art(`<rect x="10" y="18" width="28" height="24" rx="5" fill="#e3a65c"/><g stroke="#b97a33" stroke-width="2"><path d="M19 18v24M29 18v24M10 26h28M10 34h28"/></g><circle cx="20" cy="30" r="1.6" fill="#2a2a2a"/><circle cx="28" cy="30" r="1.6" fill="#2a2a2a"/>`),
  Lantern: art(`<rect x="15" y="14" width="18" height="26" rx="6" fill="#f2b84b"/><rect x="18" y="19" width="12" height="16" rx="3" fill="#fff1c7"/><rect x="20" y="9" width="8" height="5" rx="2" fill="#8a5a1d"/><circle cx="22" cy="26" r="1.4" fill="#2a2a2a"/><circle cx="26" cy="26" r="1.4" fill="#2a2a2a"/>`),
};

type Kind = "pet" | "skill" | "plugin" | "git_project" | "non_git_project";
interface Repo {
  id: string;
  owner: string;
  name: string;
  kind: Kind;
  role: "owner" | "member" | null;
  visibility: "private" | "public";
  description: string | null;
  stars: number;
  starred: boolean;
  updated: number;
  revision: number;
  registration?: boolean;
  github?: string | null;
}
const addressOf = (repo: Repo) => `${repo.owner}/${repo.name}`;

function repo(owner: string, name: string, kind: Kind, role: Repo["role"], visibility: Repo["visibility"], description: string | null,
  stars: number, starred: boolean, updated: number, revision: number, extra: Partial<Repo> = {}): Repo {
  return { id: `${owner}/${name}`.toLowerCase(), owner, name, kind, role, visibility, description, stars, starred, updated, revision, ...extra };
}

/* Pets: Rika321's Catalog row lists Duck and Fable already. */
const BISCUIT = repo("kai", "Biscuit", "pet", "member", "private", "A beagle that fetches finished builds.", 0, false, now - 5 * DAY, 2);
const PEBBLE = repo("sam", "Pebble", "pet", "member", "private", "A smooth stone that rolls over when tests pass.", 0, false, now - 2 * DAY, 1);
const MOCHI = repo("maya", "Mochi", "pet", null, "public", "A round rice cake that wobbles when you type.", 12, true, now - 6 * DAY, 5);
const TOFU = repo("leo", "Tofu", "pet", null, "public", "A quiet cube. Blinks when a run ends.", 4, true, now - 12 * DAY, 3);
const PIXEL = repo("nova", "Pixel", "pet", null, "public", "A tiny robot that salutes green checks.", 31, false, now - 1 * DAY, 8);
const CLOVER = repo("kai", "Clover", "pet", null, "public", "Four leaves; one wilts on a failing test.", 2, false, now - 3 * DAY, 2);
const SPROCKET = repo("ada", "Sprocket", "pet", null, "public", "A gear that spins while Codex thinks.", 9, false, now - 8 * DAY, 4);
const NIMBUS = repo("jun", "Nimbus", "pet", null, "public", null, 0, false, now - 20 * DAY, 1);
const WAFFLE = repo("ivy", "Waffle", "pet", "member", "private", "Golden and square. Joined with a code.", 0, false, now - 4 * HOUR, 1);
const LANTERN = repo("zoe", "Lantern", "pet", null, "public", "Glows while a session is open.", 1, false, now - 40 * DAY, 2);
const DUCK = repo(LOGIN, "Duck", "pet", "owner", "public", "A yellow duck that waddles along the dock.", 3, false, now - 2 * HOUR, 3);
const SKILL_PACK = repo("lin", "skill-pack", "skill", null, "public", "Review and release skills.", 6, false, now - 9 * DAY, 2);

/* Projects: the account's, the ones it stars, the public ones. */
const PORTAL = repo(LOGIN, "portal", "git_project", "owner", "private", "Customer portal.", 0, false, now - 3 * HOUR, 14, { github: "https://github.com/Rika321/portal" });
const MALLARD = repo(LOGIN, "mallard-app", "git_project", "owner", "private", null, 0, false, now - 1 * DAY, 41, { github: "https://github.com/Rika321/mallard-app" });
const BILLING = repo("acme", "billing", "git_project", "member", "private", "Invoicing service.", 0, false, now - 2 * DAY, 9, { github: "https://github.com/acme/billing" });
const NOTES = repo("sam", "notes", "non_git_project", "member", "private", "Meeting notes, no Git.", 0, false, now - 6 * DAY, 3);
const THESIS = repo(LOGIN, "thesis", "non_git_project", "owner", "private", null, 0, false, now - 30 * DAY, 0, { registration: true });
const AGENT_KIT = repo("lin", "agent-kit", "git_project", null, "public", "Sessions from building a small agent harness.", 48, true, now - 2 * DAY, 22, { github: "https://github.com/lin/agent-kit" });
const MAYA_PORTAL = repo("maya", "portal", "git_project", null, "public", "A portal, a different one.", 7, true, now - 9 * DAY, 6, { github: "https://github.com/maya/portal" });
const DOTFILES = repo("ada", "dotfiles", "git_project", null, "public", "How Ada sets up a Mac.", 15, false, now - 1 * DAY, 30, { github: "https://github.com/ada/dotfiles" });
const HANDBOOK = repo("leo", "handbook", "non_git_project", null, "public", "Team handbook, folder only.", 3, false, now - 4 * DAY, 2);
const SITE = repo("nova", "site", "git_project", null, "public", null, 1, false, now - 15 * DAY, 4, { github: null });
const RECIPES = repo("kai", "recipes", "non_git_project", null, "public", "Dinner, planned by an agent.", 0, false, now - 25 * DAY, 1);
const DESIGN = repo("acme", "design", "git_project", "member", "private", "Joined with a code.", 0, false, now - 1 * HOUR, 2, { github: "https://github.com/acme/design" });

type Host = "pets" | "project" | "connect";
type ScopeId = "shared" | "yours" | "starred" | "public";
interface ScopeDef { id: ScopeId; label: string; icon: IconName; }
const SCOPES: Record<Host, ScopeDef[]> = {
  pets: [{ id: "shared", label: "Shared with you", icon: "cloud-user" }, { id: "starred", label: "Starred", icon: "star" }, { id: "public", label: "Public", icon: "cloud" }],
  project: [{ id: "yours", label: "Yours", icon: "user" }, { id: "starred", label: "Starred", icon: "star" }, { id: "public", label: "Public", icon: "cloud" }],
  connect: [{ id: "yours", label: "Yours", icon: "user" }, { id: "starred", label: "Starred", icon: "star" }, { id: "public", label: "Public", icon: "cloud" }],
};
type Rows = Partial<Record<ScopeId, Repo[]>>;
const PROJECT_ROWS: Rows = { yours: [PORTAL, MALLARD, BILLING, NOTES, THESIS], starred: [AGENT_KIT, MAYA_PORTAL], public: [DOTFILES, AGENT_KIT, HANDBOOK, MAYA_PORTAL, SITE, RECIPES] };
const ROWS: Record<Host, Rows> = {
  pets: { shared: [BISCUIT, PEBBLE], starred: [MOCHI, TOFU], public: [PIXEL, CLOVER, MOCHI, SPROCKET, TOFU, NIMBUS] },
  project: PROJECT_ROWS,
  connect: PROJECT_ROWS,
};
/** What an address finds, beyond the rows. */
const PROJECT_DIRECTORY = [PORTAL, MALLARD, BILLING, NOTES, THESIS, AGENT_KIT, MAYA_PORTAL, DOTFILES, HANDBOOK, SITE, RECIPES, DUCK, SKILL_PACK, DESIGN];
const DIRECTORY: Record<Host, Repo[]> = {
  pets: [BISCUIT, PEBBLE, MOCHI, TOFU, PIXEL, CLOVER, SPROCKET, NIMBUS, LANTERN, DUCK, SKILL_PACK, PORTAL],
  project: PROJECT_DIRECTORY,
  connect: PROJECT_DIRECTORY,
};
const JOINS: Record<Host, Repo> = { pets: WAFFLE, project: DESIGN, connect: DESIGN };
const NOUN: Record<Host, string> = { pets: "pet", project: "project", connect: "project" };
const KIND_ICON: Record<Host, IconName> = { pets: "catalog-pet-v2", project: "git-folder", connect: "git-folder" };
const PUBLIC_PAGE = 4;

/* ——— words ———————————————————————————————————————————————————————— */

function isCodeShaped(text: string): boolean {
  const trimmed = text.trim();
  return !!trimmed && !/[/.]/.test(trimmed) && trimmed.replace(/[\s-]+/g, "").length === 20;
}
function isAddressShaped(text: string): boolean {
  const trimmed = text.trim().replace(/^https?:\/\//i, "");
  return /^([a-z0-9.-]+\/)?[a-z0-9._-]+\/[a-z0-9._-]+\/?$/i.test(trimmed) || /^[a-z0-9.-]+\.[a-z]{2,}\//i.test(trimmed);
}
type Entry = "idle" | "filter" | "address" | "code";
const entryKind = (text: string): Entry => (!text.trim() ? "idle" : isCodeShaped(text) ? "code" : isAddressShaped(text) ? "address" : "filter");

const roleWord = (row: Repo) => (row.role === null ? "Public" : `${row.visibility === "public" ? "Public · " : ""}${capitalize(row.role)}`);
const kindWord = (kind: Kind) => ({ pet: "pet", skill: "skill", plugin: "plugin intent", git_project: "Git project", non_git_project: "project" }[kind]);
const kindMark = (kind: Kind): IconName => ({ pet: "catalog-pet-v2", skill: "lightbulb", plugin: "catalog-plugin", git_project: "git-folder", non_git_project: "folder" }[kind] as IconName);
const isProject = (kind: Kind) => kind === "git_project" || kind === "non_git_project";

/** Why the repository the address found does not fit, or null. */
function misfit(host: Host, row: Repo): string | null {
  const address = addressOf(row);
  if (host === "pets") {
    if (isProject(row.kind)) return `${address} is a project, not a pet.`;
    if (row.kind !== "pet") return `${address} is a ${kindWord(row.kind)}, not a pet.`;
    if (row.role === "owner") return `${address} is yours — already listed.`;
    return null;
  }
  if (!isProject(row.kind)) return `${address} is a ${kindWord(row.kind)}, not a project.`;
  return null;
}

type Lookup =
  | { kind: "idle" }
  | { kind: "resolving"; address: string }
  | { kind: "found"; row: Repo; joined: boolean; inRows: ScopeId | null }
  | { kind: "not_found"; address: string }
  | { kind: "error"; message: string };

function resolveAddress(host: Host, typed: string): Lookup {
  const text = typed.trim().replace(/^https?:\/\//i, "").replace(/\/$/, "");
  if (/^(www\.)?github\.com\//i.test(text)) return { kind: "error", message: "That’s a GitHub address." };
  if (/^staging\.mallard-ai\.com\//i.test(text)) return { kind: "error", message: `That’s staging.${HOST}, not this account.` };
  const path = text.replace(/^(www\.)?mallard-ai\.com\//i, "").toLowerCase();
  if (!/^[a-z0-9._-]+\/[a-z0-9._-]+$/.test(path)) return { kind: "error", message: "Paste a link, owner/name, or an invite code." };
  const hit = DIRECTORY[host].find((row) => row.id === path);
  if (!hit) return { kind: "not_found", address: path };
  const why = misfit(host, hit);
  if (why) return { kind: "error", message: why };
  return { kind: "found", row: hit, joined: false, inRows: null };
}

/* ——— the picker ————————————————————————————————————————————————————— */

type Style = "rows" | "cards" | "headings";
type FieldMode = "one" | "two";
interface Options { style: Style; previews: boolean; field: FieldMode; }
type Skin = "choice" | "match";
type Sort = "newest" | "stars";
type Selection = "new" | string | null;

/** A frozen state, for the boards. */
interface Preset {
  query?: string;
  tab?: ScopeId;
  lookup?: Lookup;
  selection?: Selection;
  joined?: Repo[];
  listing?: "loading" | "error" | "ready";
  empty?: ScopeId[];
  sort?: Sort;
  shown?: number;
  /** Cards: the ones Add already took. */
  added?: string[];
}

interface PickerProps {
  host: Host;
  skin: Skin;
  options: Options;
  /** Connect offers a new repository first under Yours. */
  offerNew?: boolean;
  preset?: Preset;
  frozen?: boolean;
  onChange?: (selection: Selection, row: Repo | null) => void;
}

/** The style a host can draw: the setup page has no cards, and Connect
 *  needs a selection, so both fall back to headings. */
function styleFor(host: Host, skin: Skin, style: Style): Style {
  if (skin === "match") return style === "rows" ? "rows" : "headings";
  if (host === "connect" && style === "cards") return "headings";
  return style;
}

function usePicker({ host, skin, options, offerNew = false, preset, frozen = false, onChange }: PickerProps) {
  const scopes = SCOPES[host];
  const style = styleFor(host, skin, options.style);
  const [tab, setTab] = useState<ScopeId>(preset?.tab ?? scopes[0].id);
  const [query, setQuery] = useState(preset?.query ?? "");
  const [lookup, setLookup] = useState<Lookup>(preset?.lookup ?? { kind: "idle" });
  const [joining, setJoining] = useState(false);
  const [joined, setJoined] = useState<Repo[]>(preset?.joined ?? []);
  const [selection, setSelection] = useState<Selection>(preset?.selection ?? (offerNew ? "new" : null));
  const [added, setAdded] = useState<string[]>(preset?.added ?? []);
  const [sort, setSort] = useState<Sort>(preset?.sort ?? "newest");
  const [shown, setShown] = useState(preset?.shown ?? PUBLIC_PAGE);
  const listing = preset?.listing ?? "ready";
  const empty = new Set(preset?.empty ?? []);
  const sequence = useRef(0);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  const rowsOf = (scope: ScopeId): Repo[] => {
    if (empty.has(scope)) return [];
    const base = ROWS[host][scope] ?? [];
    const extra = scope === scopes[0].id ? joined : [];
    const all = [...extra, ...base];
    return scope === "public" && sort === "stars" ? [...all].sort((a, b) => b.stars - a.stars || b.updated - a.updated) : all;
  };
  const entry = entryKind(query);
  const filtering = entry === "filter";
  const needle = query.trim().toLowerCase();
  const matches = (row: Repo) => !filtering || addressOf(row).toLowerCase().includes(needle) || (row.description ?? "").toLowerCase().includes(needle);
  const filtered = (scope: ScopeId) => rowsOf(scope).filter(matches);
  const counts = Object.fromEntries(scopes.map((scope) => [scope.id, filtered(scope.id).length])) as Record<ScopeId, number>;
  const allRows = scopes.flatMap((scope) => rowsOf(scope.id));
  const chosen: Repo | null = selection && selection !== "new" ? allRows.find((row) => row.id === selection) ?? (lookup.kind === "found" && lookup.row.id === selection ? lookup.row : null) : null;

  const pick = (next: Selection, row: Repo | null) => {
    setSelection(next);
    onChangeRef.current?.(next, row);
  };
  const scopeHolding = (row: Repo): ScopeId | null => scopes.find((scope) => rowsOf(scope.id).some((candidate) => candidate.id === row.id))?.id ?? null;

  const submit = async () => {
    if (frozen) return;
    const typed = query.trim();
    if (!typed || joining || lookup.kind === "resolving") return;
    const request = ++sequence.current;
    if (entry === "code") {
      setJoining(true);
      await wait(500);
      if (request !== sequence.current) return;
      const row = JOINS[host];
      setJoining(false);
      setJoined((current) => (current.some((candidate) => candidate.id === row.id) ? current : [row, ...current]));
      setQuery(addressOf(row));
      setTab(scopes[0].id);
      setLookup({ kind: "found", row, joined: true, inRows: scopes[0].id });
      pick(row.id, row);
      return;
    }
    if (entry !== "address") return;
    setLookup({ kind: "resolving", address: typed });
    await wait(450);
    if (request !== sequence.current) return;
    const next = resolveAddress(host, typed);
    if (next.kind === "found") {
      const holder = scopeHolding(next.row);
      if (holder) setTab(holder);
      setLookup({ ...next, inRows: holder });
      pick(next.row.id, next.row);
    } else {
      setLookup(next);
      if (selection !== "new") pick(null, null);
    }
  };

  const chooseRow = (row: Repo) => {
    if (frozen) return;
    sequence.current += 1;
    setJoining(false);
    setLookup({ kind: "idle" });
    pick(row.id, row);
  };
  const addCard = (row: Repo) => {
    if (frozen) return;
    setAdded((current) => [...current, row.id]);
  };
  const chooseNew = () => {
    if (frozen) return;
    sequence.current += 1;
    setLookup({ kind: "idle" });
    pick("new", null);
  };
  const changeQuery = (value: string) => {
    if (frozen) return;
    setQuery(value);
    if (lookup.kind === "error" || lookup.kind === "not_found") setLookup({ kind: "idle" });
    if (lookup.kind === "found" && value.trim().toLowerCase() !== addressOf(lookup.row).toLowerCase()) {
      setLookup({ kind: "idle" });
      if (selection === lookup.row.id && !scopeHolding(lookup.row)) pick(null, null);
    }
  };

  /* ——— the field ——— */
  const verbIcon: IconName = entry === "code" ? "user" : "search";
  const verbLabel = joining ? "Joining…" : lookup.kind === "resolving" ? "Looking up…" : entry === "code" ? "Join" : "Look up";
  const placeholder = options.field === "one" ? "Search, or paste a link, owner/name or an invite code" : "Link, owner/name or invite code";
  const invalid = lookup.kind === "error";
  const lookupField = (
    <label className="sync-folder-field compact">
      <span>Address or invite code</span>
      <div>
        <input value={options.field === "one" ? query : (entry === "filter" ? "" : query)} spellCheck={false} autoComplete="off" placeholder={placeholder}
          aria-invalid={invalid || undefined} readOnly={frozen}
          onChange={(event) => changeQuery(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void submit(); } }} />
        <button type="button" className="btn repo-verb" disabled={frozen || joining || lookup.kind === "resolving" || (entry !== "address" && entry !== "code")} onClick={() => void submit()}>
          <Icon name={verbIcon} size={12} />
          {verbLabel}
        </button>
      </div>
      {entry === "code" && <small>Join to become a member.</small>}
    </label>
  );
  const searchField = options.field === "two" ? (
    <div className="sync-name-search cloud-add-search">
      <div className="sync-name-search-input">
        <Icon name="search" size={14} />
        <input type="search" value={entry === "filter" ? query : ""} placeholder={`Search ${NOUN[host]}s…`} aria-label={`Search ${NOUN[host]}s`} readOnly={frozen}
          onChange={(event) => changeQuery(event.target.value)} />
      </div>
      <span className="sync-visually-hidden" role="status">{filtering ? `${counts[tab]} in ${scopes.find((scope) => scope.id === tab)?.label}` : ""}</span>
    </div>
  ) : null;

  /* ——— what the field answered ——— */
  let found: ReactNode = null;
  if (lookup.kind === "resolving") {
    found = <span className="sync-visually-hidden" role="status">Looking up {lookup.address}…</span>;
  } else if (lookup.kind === "error") {
    found = <FieldMessage blocking>{lookup.message}</FieldMessage>;
  } else if (lookup.kind === "not_found") {
    found = (
      <div className="r2-step-list">
        <div className="storage-found-empty" role="status">
          <Icon name="search" size={24} />
          <strong>Nothing at {lookup.address}.</strong>
          <span>Check the spelling, or paste the link from its page.</span>
          {offerNew && (
            <span className="storage-found-empty-verbs">
              <button type="button" className="btn btn-primary" onClick={chooseNew}>Use a new repository</button>
            </span>
          )}
        </div>
      </div>
    );
  } else if (lookup.kind === "found" && lookup.inRows) {
    const where = scopes.find((scope) => scope.id === lookup.inRows)?.label;
    found = (
      <small className="repo-answer-line" role="status">
        <Icon name="check-circle" size={13} /> {lookup.joined ? `Joined ${addressOf(lookup.row)}. ${style === "cards" ? "Listed below." : "Selected below."}` : `In ${where}. ${style === "cards" ? "Listed below." : "Selected below."}`}
      </small>
    );
  } else if (lookup.kind === "found") {
    const row = lookup.row;
    const selected = style === "cards" ? added.includes(row.id) : selection === row.id;
    found = (
      <div className="r2-step-list">
        <div className="storage-found-list" aria-label="Repository at this address">
          <div className={`storage-found-row with-verb${selected ? " selected" : ""}`}>
            <span className="sync-storage-repository-icon"><Icon name={kindMark(row.kind)} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">{addressOf(row)}{isProject(row.kind) && <code>{kindWord(row.kind).toLowerCase()}</code>}</span>
              <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {roleWord(row)}{row.registration ? "" : ` · v${row.revision}`}</span>
            </span>
            <span className="storage-found-verb">
              {selected
                ? <span className="storage-found-connected"><Icon name="check" size={12} /> {style === "cards" ? "Added" : "Selected"}</span>
                : <button type="button" className="btn btn-primary" onClick={() => (style === "cards" ? addCard(row) : pick(row.id, row))}>{style === "cards" ? "Add" : "Use this"}</button>}
            </span>
          </div>
        </div>
      </div>
    );
  }

  /* ——— the sort ——— */
  const sortControl = (
    <div className="sync-segmented cloud-add-sort" role="radiogroup" aria-label="Sort">
      <button type="button" role="radio" aria-checked={sort === "newest"} className={sort === "newest" ? "active" : undefined} onClick={() => { if (!frozen) setSort("newest"); }}>Newest</button>
      <button type="button" role="radio" aria-checked={sort === "stars"} className={sort === "stars" ? "active" : undefined} onClick={() => { if (!frozen) setSort("stars"); }}>Most stars</button>
    </div>
  );
  const countOf = (scope: ScopeDef): number | null => (listing !== "ready" || (scope.id === "public" && !filtering) ? null : counts[scope.id]);

  /* ——— the switch: tabs (S1) or pills (S2) ——— */
  const tabs = (
    <div className="sync-project-tabs sync-sync-review-tabs cloud-add-tabs" role="tablist" aria-label="Where to look">
      {scopes.map((scope) => {
        const active = tab === scope.id;
        const count = countOf(scope);
        return (
          <button key={scope.id} type="button" role="tab" aria-selected={active} tabIndex={active ? 0 : -1}
            className={active ? "active" : undefined} onClick={() => { if (!frozen) setTab(scope.id); }}>
            <Icon name={scope.icon} size={14} />
            {scope.label}
            {count !== null && count > 0 && <small>{count}</small>}
          </button>
        );
      })}
      {tab === "public" && !filtering && sortControl}
    </div>
  );
  const pills = (
    <div className="setup-pill-row cloud-add-pills" role="radiogroup" aria-label="Where to look">
      {scopes.map((scope) => {
        const active = tab === scope.id;
        const count = countOf(scope);
        return (
          <button key={scope.id} type="button" role="radio" aria-checked={active} className={`setup-pill${active ? " active" : ""}`}
            onClick={() => { if (!frozen) setTab(scope.id); }}>
            <Icon name={scope.icon} size={12} />
            <span className="setup-pill-label">{scope.label}{count !== null && count > 0 ? ` · ${count}` : ""}</span>
            <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
          </button>
        );
      })}
      {tab === "public" && !filtering && sortControl}
    </div>
  );

  /* ——— tiles ——— */
  const petTile = (row: Repo, small: boolean) => (
    <span className={`catalog-pet-preview${small ? " small" : ""}`} aria-hidden="true">
      {options.previews && ART[row.name] ? <img src={ART[row.name]} alt="" /> : <Icon name="catalog-pet-v2" size={16} />}
    </span>
  );
  const tile = (row: Repo) => (row.kind === "pet"
    ? petTile(row, true)
    : skin === "match"
      ? <span className="setup-choice-mark" aria-hidden="true"><Icon name={kindMark(row.kind)} size={24} /></span>
      : <span className="sync-storage-repository-icon" aria-hidden="true"><Icon name={kindMark(row.kind)} size={14} /></span>);

  /* ——— rows ——— */
  const rowClass = skin === "choice" ? "sync-bundle-choice" : "sync-bundle-match-option";
  const copyClass = skin === "choice" ? "sync-bundle-choice-copy" : "sync-bundle-match-copy";
  const metaClass = skin === "choice" ? "sync-bundle-choice-meta" : "sync-bundle-match-meta";
  const listClass = skin === "choice" ? "sync-bundle-choice-list" : "sync-bundle-match-options";
  const ageOf = (row: Repo) => (row.registration ? "not pushed yet" : formatRelativeTime(row.updated));

  const rowOf = (row: Repo) => {
    const active = selection === row.id;
    const blurb = row.description;
    return (
      <button key={row.id} type="button" role="radio" aria-checked={active}
        className={`${rowClass} cloud-add-row${active ? " active" : ""}${row.starred ? " starred" : ""}`}
        onClick={() => chooseRow(row)} disabled={frozen}>
        <span className="sync-bundle-radio"><span /></span>
        {tile(row)}
        <span className={copyClass}>
          <strong>
            <span>{addressOf(row)}</span>
            {row.github && <Icon name="git-branch" size={12} aria-label="On GitHub" />}
            {skin === "match" && isProject(row.kind) && <code>{kindWord(row.kind).toLowerCase()}</code>}
          </strong>
          {blurb ? <span className="cloud-add-blurb">{blurb}</span> : skin === "match" ? <span className="cloud-add-blurb">{roleWord(row)}</span> : null}
        </span>
        {skin === "choice" ? (
          <span className={metaClass}>
            <strong>{roleWord(row)}</strong>
            <small>
              {row.stars > 0 && <><Icon name={row.starred ? "star-filled" : "star"} size={11} /> {row.stars} · </>}
              {ageOf(row)}
            </small>
          </span>
        ) : (
          <span className={metaClass}>{ageOf(row)}</span>
        )}
      </button>
    );
  };

  const newRow = offerNew && !filtering && (
    <button type="button" role="radio" aria-checked={selection === "new"} className={`${rowClass} separate${selection === "new" ? " active" : ""}`} onClick={chooseNew} disabled={frozen}>
      <span className="sync-bundle-radio"><span /></span>
      <span className={copyClass}>
        <strong>New repository</strong>
        <span className="repo-new-address">{LOGIN}/portal-2</span>
        <span>{LOGIN}/portal is already used.</span>
      </span>
      <span className={metaClass}><small>Private</small></span>
    </button>
  );

  /* ——— cards (S2): the Catalog row page's own card, Add in the actions slot ——— */
  const cardOf = (row: Repo, scope: ScopeDef) => {
    const address = addressOf(row);
    const role = roleWord(row);
    const taken = added.includes(row.id);
    const who = `${scope.label} · ${role}`;
    const when = `Updated ${formatRelativeTime(row.updated)}`;
    return (
      <article key={row.id} className="sync-history-thread-card catalog-card catalog-card-pet cloud-add-card" aria-label={`${row.name}, ${who}`}>
        <span className="catalog-pet-preview" data-tooltip={row.description ?? undefined} tabIndex={row.description ? 0 : undefined} aria-hidden={row.description ? undefined : true}>
          {options.previews && ART[row.name] ? <img src={ART[row.name]} alt={`${row.name} preview`} /> : <Icon name="catalog-pet-v2" size={16} />}
        </span>
        <div className="sync-history-thread-topline">
          <div className="sync-history-thread-title">
            <span className="sync-thread-sync-indicator storage" data-tooltip={who} aria-label={who} tabIndex={0}>
              <Icon name={scope.icon} size={18} />
            </span>
            <strong title={row.name}>{row.name}</strong>
            <span className="sync-history-branch-state current" title={who}>{row.role === null ? "Public" : capitalize(row.role)}</span>
          </div>
          <div className="sync-history-thread-actions">
            {taken
              ? <span className="storage-found-connected"><Icon name="check" size={12} /> Added</span>
              : (
                <button type="button" className="btn" title={`Add ${row.name} from ${address} — it keeps this repository`} disabled={frozen} onClick={() => addCard(row)}>
                  <Icon name="plus" size={13} /> Add
                </button>
              )}
          </div>
        </div>
        <div className="sync-history-session-details">
          <div className="sync-history-session-summary">
            <div className="sync-history-thread-meta" aria-label={`${row.name} details`}>
              <span className="sync-history-thread-metric" data-tooltip={`Repository · ${address}`} tabIndex={0}>
                <Icon name="cloud" size={12} /><b>{address}</b>
              </span>
              <span className="sync-history-thread-metric" data-tooltip={`Version ${row.revision} in ${address}`} tabIndex={0}>
                <Icon name="git-commit" size={12} /><b>v{row.revision}</b>
              </span>
              {row.stars > 0 && (
                <span className="sync-history-thread-metric" data-tooltip={`${row.stars} stars${row.starred ? ", yours among them" : ""}`} tabIndex={0}>
                  <Icon name={row.starred ? "star-filled" : "star"} size={12} /><b>{row.stars}</b>
                </span>
              )}
              <span className="sync-history-thread-metric" aria-hidden="true" />
            </div>
            <span className="sync-history-actions-divider" aria-hidden="true" />
            <div className="sync-history-row-clickables">
              <button type="button" className="btn btn-ghost sync-history-launch-action" title={row.description ?? "No description"} aria-label={`${row.name} info`}><Icon name="info" size={13} /></button>
            </div>
            <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={when} aria-label={when} tabIndex={0}>
              <time dateTime={new Date(row.updated * 1000).toISOString()}>{formatFullDateTime(row.updated)}</time>
            </span>
          </div>
        </div>
      </article>
    );
  };

  /* ——— the foot: the Activity page's Load earlier, for the next public page ——— */
  const foot = (more: number) => (
    <div className="sync-history-rail-foot no-rail">
      <span className="sync-history-rail-foot-node" aria-hidden="true"><Icon name="chevron-down" size={12} /></span>
      <button type="button" className="sync-history-rail-step" disabled={frozen} onClick={() => setShown((current) => current + PUBLIC_PAGE)}>
        <strong>Show more</strong>
        <span>· {more} more</span>
      </button>
    </div>
  );

  const emptyWords = (scope: ScopeDef) => (
    scope.id === "shared" ? "Nothing shared with you yet."
      : scope.id === "yours" ? "No repositories yet."
        : scope.id === "starred" ? "Nothing starred yet."
          : `No public ${NOUN[host]}s yet.`
  );

  /* ——— one scope's rows, paged for Public ——— */
  const pageOf = (scope: ScopeDef) => {
    const visible = filtered(scope.id);
    const paged = scope.id === "public" && !filtering ? visible.slice(0, shown) : visible;
    return { visible, paged, more: visible.length - paged.length };
  };

  /* ——— the body, by style ——— */
  let list: ReactNode;
  if (listing === "loading") {
    list = <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Loading…</small>;
  } else if (listing === "error") {
    list = <Notice severity="danger" timing="blocking">Couldn’t reach {HOST}. <button type="button" className="btn-link">Try again</button></Notice>;
  } else if (style === "headings") {
    const sections = scopes.map((scope) => {
      const { visible, paged, more } = pageOf(scope);
      if (filtering && visible.length === 0) return null;
      const count = scope.id === "public" ? null : visible.length;
      return (
        <div key={scope.id} className="cloud-add-section">
          <div className="sync-history-section-heading">
            <h2><Icon name={scope.icon} size={14} />{scope.label}</h2>
            {scope.id === "public" && !filtering
              ? <span className="project-storage-heading-actions">{sortControl}</span>
              : count !== null && count > 0
                ? <span className="sync-history-heading-count" title={`${count} ${NOUN[host]}${count === 1 ? "" : "s"}`} aria-label={`${count} ${NOUN[host]}${count === 1 ? "" : "s"}`}><Icon name={KIND_ICON[host]} size={12} />{count}</span>
                : null}
          </div>
          {visible.length === 0 && !(offerNew && scope.id === "yours")
            ? <p className="repo-list-empty">{emptyWords(scope)}</p>
            : (
              <div className={`${listClass} repo-rows`} role="radiogroup" aria-label={scope.label}>
                {scope.id === "yours" && newRow}
                {paged.map(rowOf)}
              </div>
            )}
          {more > 0 && foot(more)}
        </div>
      );
    });
    list = sections.every((section) => section === null)
      ? (
        <div className={listClass}>
          <div className="storage-found-empty cloud-add-empty" role="status">
            <Icon name="search" size={24} />
            <strong>No match for “{query.trim()}”.</strong>
          </div>
        </div>
      )
      : sections;
  } else {
    const scope = scopes.find((candidate) => candidate.id === tab)!;
    const { paged, more } = pageOf(scope);
    const elsewhere = scopes.filter((candidate) => candidate.id !== tab && counts[candidate.id] > 0);
    if (paged.length === 0 && !(offerNew && tab === "yours")) {
      list = (
        <div className={listClass}>
          <div className="storage-found-empty cloud-add-empty" role="status">
            <Icon name={filtering ? "search" : scope.icon} size={24} />
            <strong>{filtering ? `No match for “${query.trim()}” in ${scope.label}.` : emptyWords(scope)}</strong>
            {filtering && elsewhere.length > 0 && (
              <span>
                {elsewhere.map((candidate, index) => (
                  <span key={candidate.id}>
                    {index > 0 && " · "}
                    <button type="button" className="btn-link" onClick={() => { if (!frozen) setTab(candidate.id); }}>{counts[candidate.id]} in {candidate.label}</button>
                  </span>
                ))}
              </span>
            )}
            {!filtering && tab !== "public" && (
              <span className="storage-found-empty-verbs">
                <button type="button" className="btn" onClick={() => { if (!frozen) setTab("public"); }}><Icon name="cloud" size={13} /> Browse Public</button>
              </span>
            )}
          </div>
        </div>
      );
    } else if (style === "cards") {
      list = (
        <>
          <div className="sync-history-thread-list cloud-add-cards" role="list" aria-label={scope.label}>
            {paged.map((row) => cardOf(row, scope))}
          </div>
          {more > 0 && foot(more)}
        </>
      );
    } else {
      list = (
        <>
          <div className={`${listClass} repo-rows`} role="radiogroup" aria-label={scope.label}>
            {tab === "yours" && newRow}
            {paged.map(rowOf)}
          </div>
          {more > 0 && foot(more)}
        </>
      );
    }
  }

  const switcher = style === "rows" ? tabs : style === "cards" ? pills : null;
  const body = options.field === "one"
    ? <>{lookupField}{found}{switcher}{list}</>
    : <>{searchField}{switcher}{list}<div className="repo-list-head">By address or invite code</div>{lookupField}{found}</>;

  return { style, selection, chosen, body, added };
}

/* ——— hosts ——————————————————————————————————————————————————————————— */

function PetsAddDialog({ options, preset, frozen = true, working = false, failure = null }: {
  options: Options; preset?: Preset; frozen?: boolean; working?: boolean; failure?: string | null;
}) {
  const titleId = useId();
  const { style, selection, chosen, body } = usePicker({ host: "pets", skin: "choice", options, preset, frozen });
  const cards = style === "cards";
  const note = selection && chosen ? "Can’t be changed later." : null;
  return (
    <dialog open className={`sync-modal sync-bundle-connection-dialog mock-frozen-dialog cloud-add cloud-add-${style}`} aria-labelledby={titleId}>
      <header className="sync-modal-header">
        <div><h1 id={titleId}>Add from Mallard Cloud</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close repo chooser"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body repo-connect">
        {body}
        {failure && <Notice severity="danger" timing="blocking">{failure}</Notice>}
      </div>
      <footer className="sync-modal-footer">
        {cards ? <span>Add lists it here; Pull downloads it. Can’t be changed later.</span> : note ? <span>{note}</span> : null}
        <div>
          <button type="button" className="btn" disabled={working}>{cards ? "Close" : "Cancel"}</button>
          {!cards && (
            <button type="button" className="btn btn-primary" disabled={working || !chosen}>{working ? "Adding…" : "Add"}</button>
          )}
        </div>
      </footer>
    </dialog>
  );
}

function ConnectDialog({ options, preset, frozen = true }: { options: Options; preset?: Preset; frozen?: boolean }) {
  const titleId = useId();
  const { style, selection, chosen, body } = usePicker({ host: "connect", skin: "choice", options, preset, frozen, offerNew: true });
  const verb = selection === "new" ? "Create and connect" : "Connect";
  const note = chosen ? (chosen.role === null ? "Anyone signed in can push. Review changes with Pull." : "Review changes with Pull.") : null;
  return (
    <dialog open className={`sync-modal sync-bundle-connection-dialog mock-frozen-dialog cloud-add cloud-add-${style}`} aria-labelledby={titleId}>
      <header className="sync-modal-header">
        <div><h1 id={titleId}>Connect portal</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close repo chooser"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body repo-connect">
        <div className="sync-bundle-context">
          <Icon name="storage-mallard" size={17} />
          <span><strong>Mallard Cloud · {LOGIN}</strong><small>{HOST}</small></span>
        </div>
        {body}
      </div>
      <footer className="sync-modal-footer">
        {note ? <span>{note}</span> : null}
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary" disabled={!selection}>{verb}</button>
        </div>
      </footer>
    </dialog>
  );
}

/** New project › From Mallard Cloud, with the picker as its Repository band. */
function ProjectPage({ options, preset, frozen = true, noAccount = false }: { options: Options; preset?: Preset; frozen?: boolean; noAccount?: boolean }) {
  const titleId = useId();
  const [chosen, setChosen] = useState<Repo | null>(null);
  const picker = usePicker({ host: "project", skin: "match", options, preset, frozen, onChange: (_selection, row) => setChosen(row) });
  const repository = chosen ?? picker.chosen;
  const willClone = !!repository?.github;
  const verb = !repository ? "Connect" : willClone ? "Clone and connect" : repository.kind === "git_project" ? "Create Git project" : "Create folder and connect";
  return (
    <section className={`sync-inline-new-project sync-setup-workspace cloud-add cloud-add-${picker.style}`} role="region" aria-labelledby={titleId}>
      <header className="sync-modal-header">
        <div><h1 id={titleId}>New project</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close; nothing has been saved"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body">
        <div className="storage-mode-doors" role="radiogroup" aria-label="Where the project comes from">
          <div className="storage-mode-option">
            <button type="button" className="storage-mode-door" role="radio" aria-checked="false">
              <span className="storage-mode-door-title"><Icon name="folder" size={17} /><strong>On this computer</strong></span>
              <span className="storage-mode-door-detail">A folder you have.</span>
            </button>
          </div>
          <div className="storage-mode-option">
            <button type="button" className="storage-mode-door" role="radio" aria-checked="true">
              <span className="storage-mode-door-title"><Icon name="storage-mallard" size={17} /><strong>From Mallard Cloud</strong></span>
              <span className="storage-mode-door-detail">A repository you can open.</span>
            </button>
          </div>
        </div>
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Storage</strong></div></div>
          {noAccount ? (
            <div className="r2-step-list">
              <div className="storage-found-empty" role="status">
                <Icon name="storage-mallard" size={24} />
                <strong>No Mallard Cloud account yet.</strong>
                <span className="storage-found-empty-verbs">
                  <button type="button" className="btn btn-primary"><Icon name="link" size={13} /> Set up Mallard Cloud</button>
                </span>
              </div>
            </div>
          ) : (
            <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
              <button type="button" role="radio" aria-checked="true" className="setup-pill active">
                <Icon name="storage-mallard" size={12} />
                <span className="setup-pill-label">Mallard Cloud</span>
                <span className="setup-pill-check" aria-hidden="true"><Icon name="check" size={12} /></span>
              </button>
              <button type="button" role="radio" aria-checked="false" className="setup-pill">
                <Icon name="storage-mallard" size={12} />
                <span className="setup-pill-label">Work</span>
                <span className="setup-pill-check" aria-hidden="true" />
              </button>
            </div>
          )}
        </section>
        {!noAccount && (
          <section className="sync-setup-section repo-connect cloud-project-repository">
            <div className="sync-card-heading"><div><strong>Repository</strong></div></div>
            {picker.body}
          </section>
        )}
        {repository && (
          <section className="sync-setup-section">
            <div className="sync-card-heading"><div><strong>Project</strong></div></div>
            <div className="sync-field-grid sync-setup-project-fields">
              <label>
                <span>Name</span>
                <input value={repository.name} readOnly disabled />
              </label>
              <label className="sync-folder-field">
                <span>Folder</span>
                <div>
                  <input value={`~/code/${repository.name}`} readOnly />
                  <button type="button" className="btn">Browse…</button>
                </div>
                <small>{willClone ? `New folder. Clones ${repository.github!.replace(/^https:\/\//, "")}; empty if that fails.` : repository.kind === "git_project" ? "New, empty folder: not on GitHub." : "New, empty folder."}</small>
              </label>
            </div>
          </section>
        )}
        {repository && (
          <section className="sync-setup-section">
            <div className="sync-card-heading"><div><strong>Agent</strong></div></div>
            <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
              <div className="sync-provider-card enabled">
                <label className="sync-provider-toggle setup-marked-toggle">
                  <input type="checkbox" checked readOnly aria-label="Use Codex in this project" />
                  <ProviderMark provider="codex" displayName="Codex" size={12} />
                  <strong>Codex</strong>
                  <span>Default Codex</span>
                </label>
              </div>
              <div className="sync-provider-card">
                <label className="sync-provider-toggle setup-marked-toggle">
                  <input type="checkbox" readOnly aria-label="Use Claude in this project" />
                  <ProviderMark provider="claude" displayName="Claude" size={12} />
                  <strong>Claude</strong>
                  <span>Not used by this project on this machine.</span>
                </label>
              </div>
            </div>
          </section>
        )}
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary" disabled={!repository}>{verb}</button>
        </div>
      </footer>
    </section>
  );
}

/* ——— Before: the two parts as shipped ———————————————————————————————— */

function BeforePetsDialog() {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog mock-frozen-dialog cloud-add" aria-label="Add from Mallard Cloud, as shipped">
      <header className="sync-modal-header">
        <div><h1>Add from Mallard Cloud</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close repo chooser"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body repo-connect">
        <label className="sync-folder-field compact">
          <span>Address or invite code</span>
          <div>
            <input placeholder="owner/name or invite code" readOnly />
            <button type="button" className="btn repo-verb" disabled><Icon name="search" size={12} />Look up</button>
          </div>
        </label>
        <div className="repo-list-head">Shared with you<small>2</small></div>
        <div className="sync-bundle-choice-list repo-rows" role="radiogroup" aria-label="Shared with you">
          {[BISCUIT, PEBBLE].map((row) => (
            <button key={row.id} type="button" role="radio" aria-checked="false" className="sync-bundle-choice">
              <span className="sync-bundle-radio"><span /></span>
              <span className="sync-bundle-choice-copy">
                <strong>{addressOf(row)}</strong>
                <span>{roleWord(row)}</span>
              </span>
              <span className="sync-bundle-choice-meta">
                <strong>v{row.revision}</strong>
                <small>{formatRelativeTime(row.updated)}</small>
              </span>
            </button>
          ))}
        </div>
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary" disabled>Add</button>
        </div>
      </footer>
    </dialog>
  );
}

function BeforeProjectBand() {
  const rows = ROWS.project.yours!;
  return (
    <section className="sync-inline-new-project sync-setup-workspace cloud-add" aria-label="New project › From Mallard Cloud, as shipped">
      <div className="sync-modal-body">
        <section className="sync-setup-section repo-connect cloud-project-repository">
          <div className="sync-card-heading"><div><strong>Repository</strong></div></div>
          <label className="sync-folder-field compact">
            <span>Address or invite code</span>
            <div>
              <input placeholder="owner/name or invite code" readOnly />
              <button type="button" className="btn repo-verb" disabled><Icon name="search" size={12} />Look up</button>
            </div>
          </label>
          <div className="repo-list-head">Repositories<small>{rows.length}</small></div>
          <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Repositories you belong to">
            {rows.map((row) => (
              <button key={row.id} type="button" role="radio" aria-checked="false" className="sync-bundle-match-option">
                <span className="sync-bundle-radio"><span /></span>
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={kindMark(row.kind)} size={24} /></span>
                <span className="sync-bundle-match-copy">
                  <strong>{addressOf(row)}</strong>
                  <span>{capitalize(row.role ?? "public")}</span>
                </span>
                <span className="sync-bundle-match-meta">{row.registration ? "not pushed yet" : formatRelativeTime(row.updated)}</span>
              </button>
            ))}
          </div>
        </section>
      </div>
    </section>
  );
}

/* ——— standards ——————————————————————————————————————————————————————— */

interface Standard { who: string; what: string; url: string; says: string; here: string; }
const STANDARDS: Standard[] = [
  {
    who: "Mallard · Catalog row page", what: "Section headings over boxed lists", url: "http://localhost:1420/mockups/catalog-cloud-add-mockup.html?scene=head",
    says: "The page the dialog lands on draws “Not downloaded” and “Pets v2” as headings with a count chip, each over its own boxed list; a pet is a card with its ink-boxed preview, name, role chip and readouts.",
    here: "S3 draws the dialog in the page’s own grammar; S2 borrows the page’s card.",
  },
  {
    who: "Mallard · Project setup", what: "Pill switches", url: "http://localhost:1420/mockups/cloud-project-existing-mockup.html",
    says: "Setup chooses between a few named things with the pill row (Storage accounts; New folder | Existing folder), the checked pill in accent.",
    here: "S2’s scope switch is that pill row.",
  },
  {
    who: "GitHub Desktop", what: "Clone a repository", url: "https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop",
    says: "One filter field over every repository you can see, grouped under owner headings in one scrolling list; the URL way in a tab beside it.",
    here: "S3: one list under headings, filtered by one field; the link way stays in the same field.",
  },
  {
    who: "VS Code", what: "Git: Clone", url: "https://code.visualstudio.com/docs/sourcecontrol/intro-to-git#_clone-a-repository",
    says: "One box: paste a URL, or type to search the repositories you have access to.",
    here: "The one field, in every style.",
  },
  {
    who: "Slack", what: "Browse channels", url: "https://slack.com/help/articles/205239967-Join-a-channel",
    says: "A search, a sort, and a Join on every row.",
    here: "S2’s Add on each card; the Public sort in every style.",
  },
  {
    who: "Discord", what: "Join a server", url: "https://support.discord.com/hc/en-us/articles/360034842871-Server-Discovery",
    says: "The invite and the discovery are one screen, each pointing at the other.",
    here: "The code joins from the same field; an empty Shared with you sits over Public (S3) or offers it (S1, S2).",
  },
  {
    who: "Nielsen Norman Group", what: "Tabs, used right", url: "https://www.nngroup.com/articles/tabs-used-right/",
    says: "Tabs hide content; use them for alternate views of parallel content, and never when the person needs to see categories together.",
    here: "Why S3 is recommended: three short lists are read together, nothing hidden, the counts always in view.",
  },
  {
    who: "Nielsen Norman Group", what: "Empty states", url: "https://www.nngroup.com/articles/empty-state-interface-design/",
    says: "An empty list says what would be there and shows the way to fill it.",
    here: "“Nothing shared with you yet.” over a Public list that is right there (S3); Browse Public (S1, S2).",
  },
  {
    who: "WAI-ARIA APG", what: "Tabs · Radio group", url: "https://www.w3.org/WAI/ARIA/apg/patterns/tabs/",
    says: "A tablist moves with arrows and takes one tab stop; a radiogroup likewise.",
    here: "S1’s tabs reuse SyncReviewTabs’ contract; S2’s pills and every row list are radiogroups, as setup’s are.",
  },
  {
    who: "Material Design 3", what: "Dialogs", url: "https://m3.material.io/components/dialogs/guidelines",
    says: "One task per dialog, actions in the footer.",
    here: "Rows select, the footer adds (S1, S3). Cards add in place and the footer only closes (S2).",
  },
];

function StandardsBoard() {
  return (
    <div className="standards">
      <p>What Mallard’s own pages and the references say, and what each means for the picker. Links open the source.</p>
      <table>
        <thead><tr><th>Source</th><th>What it says</th><th>Here</th></tr></thead>
        <tbody>
          {STANDARDS.map((standard) => (
            <tr key={standard.url}>
              <td><a href={standard.url} target="_blank" rel="noreferrer">{standard.who}</a><small>{standard.what}</small></td>
              <td>{standard.says}</td>
              <td><b>{standard.here}</b></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/* ——— scenes ——————————————————————————————————————————————————————————— */

interface Scene {
  id: string;
  title: string;
  shows: string;
  mirrors?: string;
  today?: string;
  tradeoff?: string;
  render: (options: Options) => ReactNode;
}

const Fig = ({ n, title, note, children }: { n?: string; title: string; note?: string; children: ReactNode }) => (
  <figure>
    <figcaption>{n && <i>{n}</i>}{title}{note && <small>{note}</small>}</figcaption>
    {children}
  </figure>
);

const STYLE_WORDS: Record<Style, string> = {
  rows: "S1 · Tabs + rows",
  cards: "S2 · Pills + cards",
  headings: "S3 · Headings + rows",
};
const CODE = "mallard-waffle-ivy-2026";
const SCENES: Scene[] = [
  {
    id: "styles", title: "Styles",
    shows: "The same flow drawn three ways, each from parts the app ships, beside the dialog as shipped. S1 keeps round 1: the review strip’s tabs over the connect dialog’s choice rows; the pet’s tile is now the Catalog card’s ink-boxed preview. S2 is the Catalog row page in a dialog: the setup page’s pill row switches the scope, each pet is the page’s own card with Add in its actions slot. S3 (recommended) drops the switch: the row page’s section headings with their count chips, one boxed list under each, Public last with its sort in the heading and the Activity page’s Show-more foot. The Style toggle sets what every other scene draws.",
    mirrors: "Mallard’s Catalog row page (headings, count chips, pet cards), its setup pills, its connect-dialog rows.",
    today: "One field and one list with bare address-and-role rows.",
    tradeoff: "S3 lists Public on open (one more request, paged). S2’s card has no line for the description (it lives in the preview’s tooltip, as on the page) and Add is immediate. S1 hides two scopes behind tabs.",
    render: (options) => (
      <div className="board pair">
        <Fig title="Before · as shipped" note="CloudRepositoryChoice: the field, the Shared with you head, bare rows."><BeforePetsDialog /></Fig>
        <Fig title="S1 · Tabs + rows" note="Round 1 with the shipped pet tile."><PetsAddDialog options={{ ...options, style: "rows" }} /></Fig>
        <Fig title="S2 · Pills + cards" note="Setup’s pill switch over the Catalog page’s cards; Add on each."><PetsAddDialog options={{ ...options, style: "cards" }} /></Fig>
        <Fig title="S3 · Headings + rows — recommended" note="The row page’s section grammar; nothing hidden; Public paged at the end."><PetsAddDialog options={{ ...options, style: "headings" }} /></Fig>
      </div>
    ),
  },
  {
    id: "a", title: "Live",
    shows: "The picker in the current Style, live. Try: type “bis”; type maya/Mochi and Look up; paste https://mallard-ai.com/zoe/Lantern; paste the code and Join; sort Public; Show more.",
    mirrors: "GitHub Desktop’s one filter over grouped repositories; VS Code’s one box for a URL or a search.",
    today: "The field only looks up; the list is Shared with you alone.",
    tradeoff: "Filtering is over loaded rows: the service has no text search. A pet preview costs one manifest read per row (Previews: off defers it).",
    render: (options) => <div className="stage"><PetsAddDialog options={options} frozen={false} /></div>,
  },
  {
    id: "answers", title: "The field answers",
    shows: "What the one field does with what it holds, each frozen in the current Style: plain words filter; a complete address, or the page’s link pasted from the website, looks up — found in a scope, found beyond the loaded rows (the shipped found row), not found, a misfit, a GitHub address; a code joins.",
    mirrors: "The shipped found row, empty state and field messages, unchanged.",
    today: "The shipped answers, minus the filter.",
    render: (options) => (
      <div className="board">
        <Fig n="1" title="Filtering “bis”" note="Only the scopes with a match stay; counts are the filter’s answer."><PetsAddDialog options={options} preset={{ query: "bis" }} /></Fig>
        <Fig n="2" title="Filtering “tofu”" note="A match in two scopes; Shared with you steps aside."><PetsAddDialog options={options} preset={{ query: "tofu", tab: "starred" }} /></Fig>
        <Fig n="3" title="Found in a scope" note="maya/Mochi looked up: it is in Starred, selected, and one line says so."><PetsAddDialog options={options} preset={{ query: "maya/Mochi", tab: "starred", selection: MOCHI.id, lookup: { kind: "found", row: MOCHI, joined: false, inRows: "starred" } }} /></Fig>
        <Fig n="4" title="A pasted link" note="The page’s address from the website: the host is dropped and the lookup runs on owner/name; zoe/Lantern is past the loaded page, so the shipped found row."><PetsAddDialog options={options} preset={{ query: `https://${HOST}/zoe/Lantern`, lookup: { kind: "found", row: LANTERN, joined: false, inRows: null } }} /></Fig>
        <Fig n="5" title="Not found" note="The shipped empty answer; the lists stay under it."><PetsAddDialog options={options} preset={{ query: "zed/ghost", lookup: { kind: "not_found", address: "zed/ghost" } }} /></Fig>
        <Fig n="6" title="A misfit" note="Rika321/portal is a project; the message is the shipped one."><PetsAddDialog options={options} preset={{ query: "Rika321/portal", lookup: { kind: "error", message: "Rika321/portal is a project, not a pet." } }} /></Fig>
        <Fig n="7" title="Joined with a code" note="The code joined ivy/Waffle: first under Shared with you, selected; the field holds its address."><PetsAddDialog options={options} preset={{ query: "ivy/Waffle", joined: [WAFFLE], selection: WAFFLE.id, lookup: { kind: "found", row: WAFFLE, joined: true, inRows: "shared" } }} /></Fig>
        <Fig n="8" title="Looking up" note="The verb’s own busy label is the progress."><PetsAddDialog options={options} preset={{ query: "zoe/Lantern", lookup: { kind: "resolving", address: "zoe/Lantern" } }} /></Fig>
        <Fig n="9" title="Adding" note="Chosen, the verb running."><PetsAddDialog options={options} preset={{ tab: "shared", selection: BISCUIT.id }} working /></Fig>
      </div>
    ),
  },
  {
    id: "public", title: "Public, sorted",
    shows: "The service’s public listing of this kind, newest first, 50 a page (4 here), Show more under the last row; Most stars reorders. Starred rows wear the filled star. Public has no count: the service does not say how many until the pages are read.",
    mirrors: "Slack’s sort over Browse channels; the website’s Most stars; the Activity page’s Load earlier foot.",
    today: "Not reachable: a public pet can only be typed by address.",
    render: (options) => (
      <div className="board pair">
        <Fig title="Newest" note="The first page, with Show more."><PetsAddDialog options={options} preset={{ tab: "public" }} /></Fig>
        <Fig title="Most stars" note="The same rows, reordered; no pages (the service sorts the whole set)."><PetsAddDialog options={options} preset={{ tab: "public", sort: "stars", shown: 6 }} /></Fig>
      </div>
    ),
  },
  {
    id: "project", title: "Project page",
    shows: "The same part as the Repository band of New project › From Mallard Cloud, in the page’s match-option skin: Yours (Owner then Member), Starred, Public. S1 draws tabs; S2 and S3 draw the headings (the page has no cards). Picking a row opens Project and Agent under it, as today. Live.",
    mirrors: "GitHub Desktop’s GitHub.com tab: your login’s repositories first, then the ones you belong to.",
    today: "One list named Repositories; no Starred, no Public, no filter.",
    tradeoff: "The page keeps the owner’s M1 decisions (Storage pills first, the Folder field, one verb). Only the band changes.",
    render: (options) => (
      <div className="board pair">
        <Fig title="Before · as shipped" note="RepositoryAddressPicker in clone mode: the field, the Repositories head, match-option rows."><BeforeProjectBand /></Fig>
        <Fig title="The Repository band, live" note="Pick a row: Project and Agent open under it."><ProjectPage options={options} frozen={false} /></Fig>
      </div>
    ),
  },
  {
    id: "connect", title: "Connect dialog",
    shows: "Connect {project}, for a project this Mac already has: the same picker, New repository first under Yours as today. S2 falls back to S3 here: Connect needs a selection and a footer verb.",
    mirrors: "The shipped Connect dialog, with the picker swapped in.",
    today: "Address field + Repositories list, New repository first.",
    render: (options) => <div className="stage"><ConnectDialog options={options} frozen={false} /></div>,
  },
  {
    id: "empty", title: "Empty and failed",
    shows: "The states with no rows: nothing shared, nothing starred, still loading, the service unreachable, no account on the project page (as shipped), and an Add the service refused.",
    mirrors: "Discord’s two ways in pointing at each other; NN/g on empty states.",
    today: "“Nothing shared with you yet.” and a dead end.",
    render: (options) => (
      <div className="board">
        <Fig n="1" title="Nothing shared" note="The screenshot’s state."><PetsAddDialog options={options} preset={{ empty: ["shared"] }} /></Fig>
        <Fig n="2" title="Nothing shared, nothing starred" note="Public remains."><PetsAddDialog options={options} preset={{ tab: "starred", empty: ["shared", "starred"] }} /></Fig>
        <Fig n="3" title="Loading" note="The shipped inline status."><PetsAddDialog options={options} preset={{ listing: "loading" }} /></Fig>
        <Fig n="4" title="Service unreachable" note="The shipped blocking notice with Try again; the field still looks up."><PetsAddDialog options={options} preset={{ listing: "error" }} /></Fig>
        <Fig n="5" title="No account (project page)" note="As shipped: the Storage band offers setup; no band under it."><ProjectPage options={options} noAccount /></Fig>
        <Fig n="6" title="Add refused" note="The shipped failure notice under the lists."><PetsAddDialog options={options} preset={{ tab: "shared", selection: PEBBLE.id }} failure="sam/Pebble refused: the invite was revoked." /></Fig>
      </div>
    ),
  },
  {
    id: "standards", title: "Standards",
    shows: "Mallard’s own pages first, then the references, and what each means here.",
    render: () => <StandardsBoard />,
  },
];

function Harness() {
  const params = new URLSearchParams(window.location.search);
  const initial = Math.max(0, SCENES.findIndex((scene) => scene.id === (params.get("scene") ?? "styles")));
  const [sceneIndex, setSceneIndex] = useState(initial);
  const [theme, setTheme] = useState(params.get("theme") === "dark" ? "dark" : "light");
  const [style, setStyle] = useState<Style>((["rows", "cards", "headings"] as Style[]).find((candidate) => candidate === params.get("style")) ?? "headings");
  const [previews, setPreviews] = useState(params.get("previews") !== "off");
  const [field, setField] = useState<FieldMode>(params.get("field") === "two" ? "two" : "one");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const url = new URL(window.location.href);
    url.searchParams.set("scene", SCENES[sceneIndex].id);
    url.searchParams.set("style", style);
    window.history.replaceState(null, "", url);
  }, [sceneIndex, style]);
  const scene = SCENES[sceneIndex];
  const options: Options = { style, previews, field };
  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Add from Mallard Cloud — the style pass</strong>
        <nav className="harness-scenes" aria-label="Scenes">
          {SCENES.map((candidate, index) => (
            <button key={candidate.id} type="button"
              className={[index === sceneIndex ? "active" : "", candidate.id === "styles" ? "pick" : ""].filter(Boolean).join(" ") || undefined}
              aria-current={index === sceneIndex ? "step" : undefined} onClick={() => setSceneIndex(index)}>
              <b>{index}</b> {candidate.title}
            </button>
          ))}
        </nav>
        <div className="harness-toggles">
          <span><b>Style</b>
            <button type="button" aria-pressed={style === "rows"} onClick={() => setStyle("rows")}>S1 Tabs</button>
            <button type="button" aria-pressed={style === "cards"} onClick={() => setStyle("cards")}>S2 Cards</button>
            <button type="button" aria-pressed={style === "headings"} onClick={() => setStyle("headings")}>S3 Headings</button>
          </span>
          <span><b>Field</b>
            <button type="button" aria-pressed={field === "one"} onClick={() => setField("one")}>One</button>
            <button type="button" aria-pressed={field === "two"} onClick={() => setField("two")}>Two</button>
          </span>
          <span><b>Previews</b>
            <button type="button" aria-pressed={previews} onClick={() => setPreviews(true)}>On</button>
            <button type="button" aria-pressed={!previews} onClick={() => setPreviews(false)}>Off</button>
          </span>
          <label><input type="checkbox" checked={theme === "dark"} onChange={(event) => setTheme(event.target.checked ? "dark" : "light")} /> Dark</label>
        </div>
      </div>
      <div className="harness-notes">
        <p><b>{sceneIndex} · {scene.title}.</b> {scene.shows}</p>
        {scene.mirrors && <p><span>Mirrors</span>{scene.mirrors}</p>}
        {scene.today && <p><span>Today</span>{scene.today}</p>}
        {scene.tradeoff && <p><span>Trade-off</span>{scene.tradeoff}</p>}
        {scene.id !== "styles" && scene.id !== "standards" && <p><span>Drawn as</span>{STYLE_WORDS[style]}. The invite code: <code>{CODE}</code>.</p>}
      </div>
      <div key={`${scene.id}:${style}:${field}:${previews}`}>{scene.render(options)}</div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Harness />
  </StrictMode>,
);
