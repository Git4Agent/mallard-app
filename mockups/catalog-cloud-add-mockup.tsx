import { StrictMode, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { formatFullDateTime, formatRelativeTime } from "../src/clock";
import { formatBytes } from "../src/format";
import Icon from "../src/components/Icons";
import { type NoticeItem } from "../src/components/notice/Notice";
import ProjectSidebar from "../src/components/project-sync/ProjectSidebar";
import CatalogConnectDialog from "../src/components/storage-catalog/CatalogConnectDialog";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import { rowStamp, rowStatus, rowStatusLabel } from "../src/components/storage-catalog/model";
import type {
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogLinkState,
  CatalogScopeRow,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import type { ManagedCloudListingRow } from "../src/components/managed-cloud/types";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import "../src/App.css";

/**
 * Specimen: where "Add from Mallard Cloud" lives on the Catalog row page.
 *
 * Today the verb is a round, icon-only + beside refresh in the path bar
 * (StorageCatalogPage.tsx, `sync-path-icon-action`). Scene 0 draws it as
 * shipped. The owner picked B (2026-10-02): the labeled verb at the head of
 * the list it adds to, beside the search. Refined on the pick: the search
 * keeps its own rule, which stops before the verb, and the head's count
 * ("4 pets v2") leaves the page. A (the verb labeled in the bar) and C (B
 * plus a Shared with you section) stay at the end of the scene list as the
 * directions not picked.
 *
 * Every part is shipped — the path bar and its verbs, the list head
 * (`sync-name-search` with a `trailing` verb on its `sync-list-head` row),
 * the Catalog card, the section headings, the connect dialog.
 *
 * Every Add opens the shipped CatalogConnectDialog on a stand-in backend:
 * pick kai/Biscuit or sam/Pebble (shared with you), or type maya/Mochi
 * (public), Rika321/Duck (yours, already listed), Rika321/portal (a
 * project), a github.com address, staging.mallard-ai.com/… (another
 * service), or any 20 letters as an invite code (joins kai/Biscuit).
 *
 * `npm run dev`, then /mockups/catalog-cloud-add-mockup.html (`?scene=head|today|
 * compare|added|quiet|empty|standards|bar|shared`, `?theme=dark`,
 * `?width=narrow`, `?weight=primary`).
 */

/* ——— data ————————————————————————————————————————————————————————— */

const LOGIN = "Rika321";
const HOST = "mallard-ai.com";
const STORAGE_NAME = "Mallard Cloud";
const STORAGE_ID = "storage-cloud";
const SCOPE_ID = "scope-pets-v2";
const PROFILE_NAME = "Default Codex";
const ROW_LABEL = `Pets v2 · ${PROFILE_NAME}`;
const HOME = "~/.codex";
const now = Math.floor(Date.now() / 1000);
const HOUR = 3_600;
const DAY = 86_400;
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const noop = () => {};
const nounOf = (count: number) => (count === 1 ? "pet v2" : "pets v2");
const capitalize = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);

const art = (body: string) => `data:image/svg+xml;utf8,${encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 52">${body}</svg>`,
)}`;

type PetId = "fable" | "duck" | "pip" | "mossy" | "biscuit" | "pebble" | "mochi";
interface Pet {
  id: PetId;
  name: string;
  description: string;
  files: number;
  bytes: number;
  edited: number;
  art: string;
}
const pet = (id: PetId, name: string, description: string, files: number, bytes: number, edited: number, body: string): Pet =>
  ({ id, name, description, files, bytes, edited, art: art(body) });
const PETS: Record<PetId, Pet> = {
  fable: pet("fable", "Fable", "An owl that watches long runs.", 2, 131_072, now - 9 * DAY,
    `<ellipse cx="24" cy="30" rx="15" ry="16" fill="#8a6345"/><circle cx="18" cy="25" r="5.5" fill="#f3e6cf"/><circle cx="30" cy="25" r="5.5" fill="#f3e6cf"/><circle cx="18" cy="25" r="2.4" fill="#2a2a2a"/><circle cx="30" cy="25" r="2.4" fill="#2a2a2a"/><path d="M22 30l2 3 2-3z" fill="#e5a23a"/><path d="M11 15l5 5 M37 15l-5 5" stroke="#8a6345" stroke-width="3"/>`),
  duck: pet("duck", "Duck", "A yellow duck that waddles along the dock.", 2, 151_552, now - 2 * HOUR,
    `<ellipse cx="22" cy="35" rx="15" ry="11" fill="#f4c542"/><circle cx="28" cy="20" r="9" fill="#f4c542"/><path d="M36 19l9 2.5-9 3z" fill="#ec8a23"/><circle cx="30" cy="18" r="1.8" fill="#2a2a2a"/><path d="M12 34q8 7 16 0" stroke="#d6a52a" stroke-width="2" fill="none"/>`),
  pip: pet("pip", "Pip", "An orange kitten that naps on the menu bar.", 2, 98_304, now - 3 * DAY,
    `<path d="M12 18l4-9 6 7z M36 18l-4-9-6 7z" fill="#e98a3c"/><ellipse cx="24" cy="31" rx="15" ry="14" fill="#f09a4a"/><circle cx="19" cy="28" r="2" fill="#2a2a2a"/><circle cx="29" cy="28" r="2" fill="#2a2a2a"/><path d="M22 33q2 2 4 0" stroke="#7a3e12" stroke-width="1.6" fill="none"/>`),
  mossy: pet("mossy", "Mossy", "A frog that hops when a build finishes.", 3, 212_992, now - 40 * 60,
    `<ellipse cx="24" cy="34" rx="17" ry="12" fill="#5fae5a"/><circle cx="16" cy="22" r="6" fill="#5fae5a"/><circle cx="32" cy="22" r="6" fill="#5fae5a"/><circle cx="16" cy="22" r="2.6" fill="#fff"/><circle cx="32" cy="22" r="2.6" fill="#fff"/><circle cx="16.6" cy="22.4" r="1.4" fill="#2a2a2a"/><circle cx="32.6" cy="22.4" r="1.4" fill="#2a2a2a"/><path d="M17 36q7 5 14 0" stroke="#2f6e2c" stroke-width="1.8" fill="none"/>`),
  biscuit: pet("biscuit", "Biscuit", "A beagle that fetches finished builds.", 2, 104_448, now - 5 * DAY,
    `<ellipse cx="11" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="37" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="24" cy="31" rx="14" ry="13" fill="#d9a066"/><ellipse cx="24" cy="38" rx="7" ry="5" fill="#f3e1c4"/><circle cx="19" cy="27" r="2" fill="#2a2a2a"/><circle cx="29" cy="27" r="2" fill="#2a2a2a"/><ellipse cx="24" cy="34.5" rx="2.6" ry="1.8" fill="#2a2a2a"/>`),
  pebble: pet("pebble", "Pebble", "A smooth stone that rolls over when tests pass.", 2, 92_160, now - 2 * DAY,
    `<ellipse cx="24" cy="33" rx="17" ry="13" fill="#9aa3ad"/><ellipse cx="24" cy="30" rx="13" ry="8" fill="#b7bfc7"/><circle cx="18" cy="30" r="2" fill="#2a2a2a"/><circle cx="30" cy="30" r="2" fill="#2a2a2a"/><path d="M21 35q3 2.4 6 0" stroke="#4b535b" stroke-width="1.6" fill="none"/>`),
  mochi: pet("mochi", "Mochi", "A round rice cake that wobbles when you type.", 2, 87_040, now - 6 * DAY,
    `<ellipse cx="24" cy="32" rx="17" ry="14" fill="#f6c6d4"/><circle cx="18" cy="30" r="1.8" fill="#2a2a2a"/><circle cx="30" cy="30" r="1.8" fill="#2a2a2a"/><ellipse cx="14" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><ellipse cx="34" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><path d="M22 34q2 1.6 4 0" stroke="#a24d68" stroke-width="1.4" fill="none"/>`),
};

/** The row page's inventory, as the screenshot has it: one pet only in
 *  Mallard Cloud (Pull 1), one up to date, two newer here (Push 2). */
const INVENTORY: Array<[PetId, CatalogLinkState]> = [["fable", "local_missing"], ["duck", "in_sync"], ["pip", "new"], ["mossy", "local_changed"]];
/** Nothing in Mallard Cloud that is not here, and nothing shared. */
const QUIET_INVENTORY: Array<[PetId, CatalogLinkState]> = [["duck", "in_sync"], ["pip", "new"], ["mossy", "in_sync"]];

function rowOf(id: PetId, state: CatalogLinkState): CatalogScopeRow {
  const it = PETS[id];
  const local = state !== "local_missing";
  return {
    row_id: `resource:${id}`,
    display_name: it.name,
    resource_id: id,
    state,
    local_present: local,
    local_path: local ? `${HOME}/pets/${id}` : null,
    local_modified_at: local ? it.edited : null,
    local_files: local ? it.files : null,
    local_bytes: local ? it.bytes : null,
    remote_present: state !== "new",
    remote_updated_at: state === "new" ? null : it.edited - 600,
    preview_png: it.art,
    description: it.description,
    warnings: [],
    blocker: null,
  };
}

const COORDINATE = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const CAPABILITY: CatalogCapabilityDescriptor = {
  ...COORDINATE, provider_label: "Codex", label: "Pets v2", singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const SKILLS: CatalogCapabilityDescriptor = {
  provider: "codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, provider_label: "Codex", label: "Skills", singular_label: "Skill",
  presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const PROFILE = { profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: PROFILE_NAME, configured_home: HOME, revision: 1, created_at: 0, updated_at: 0 };
const scopeView = (id: string, cap: CatalogCapabilityDescriptor): CatalogScopeView => ({
  scope: { scope_id: id, profile_id: PROFILE.profile_id, provider: cap.provider, resource_kind: cap.resource_kind, format: cap.format, format_version: cap.format_version, revision: 1, created_at: 0, updated_at: 0 },
  profile: PROFILE, label: cap.label, available: true, provider_label: "Codex", kind_label: cap.label, format_label: cap.format, capability: cap,
});
const SCOPES = [scopeView(SCOPE_ID, CAPABILITY), scopeView("scope-skills", SKILLS)];

function repository(id: PetId, owner: string, role: string | null, visibility: "private" | "public", revision: number, updatedAt: number): CatalogCloudRepository {
  const it = PETS[id];
  return {
    ...COORDINATE, provider_label: "Codex", kind_label: "Pets v2", format_label: "Codex pet v2",
    repository_id: `repo_${owner.toLowerCase()}_${id}`, owner_login: owner, name: it.name, address: `${owner}/${it.name}`, description: null, star_count: 0, viewer_starred: false,
    website_url: `https://${HOST}/${owner}/${it.name}`, visibility, role, can_push: role !== null, revision, updated_at: updatedAt,
    registration_only: false, resource_id: id, display_name: it.name, available: true,
  };
}
/** Repositories you belong to whose pet no row has yet: what Shared with
 *  you lists, on the page and in the dialog. */
const SHARED = [
  repository("biscuit", "kai", "member", "private", 2, now - 5 * DAY),
  repository("pebble", "sam", "member", "private", 1, now - 2 * DAY),
];
/** What an address finds, beyond the shared ones. */
const BY_ADDRESS = [
  ...SHARED,
  repository("mochi", "maya", null, "public", 5, now - 6 * DAY),
  repository("duck", LOGIN, "owner", "private", 3, now - 2 * HOUR),
  repository("fable", LOGIN, "owner", "private", 1, now - 9 * DAY),
];

/* ——— the stand-in backend the shipped dialog calls ———————————————————— */

mockIPC(() => null);
/** What the page lists right now, so the listing leaves it out. */
let listedNow: ReadonlySet<string> = new Set();
storageCatalogApi.listCloudShared = async () => {
  await wait(300);
  return SHARED.filter((row) => !listedNow.has(row.resource_id));
};
const listedPet = (id: PetId, owner: string, stars: number, starred: boolean, revision: number, updatedAt: number): ManagedCloudListingRow => ({
  repository_id: `repo_${owner.toLowerCase()}_${id}`, owner_login: owner, name: PETS[id].name, address: `${owner}/${PETS[id].name}`, display_name: PETS[id].name,
  website_url: `https://${HOST}/${owner}/${PETS[id].name}`, repository_class: "catalog", catalog: COORDINATE, visibility: "public",
  role: null, github_url: null, description: PETS[id].description, star_count: stars, viewer_starred: starred, revision, updated_at: updatedAt,
});
const STARRED_PETS = [listedPet("mochi", "maya", 12, true, 5, now - 6 * DAY)];
const PUBLIC_PETS = [listedPet("pebble", "nova", 31, false, 8, now - DAY), ...STARRED_PETS, listedPet("fable", "jun", 2, false, 2, now - 20 * DAY)];
storageCatalogApi.listCloudStarred = async () => { await wait(300); return { rows: STARRED_PETS, next_cursor: null }; };
storageCatalogApi.listCloudPublic = async (_scopeId, _storageId, sort) => {
  await wait(400);
  const rows = sort === "stars" ? [...PUBLIC_PETS].sort((a, b) => b.star_count - a.star_count) : PUBLIC_PETS;
  return { rows, next_cursor: null };
};
storageCatalogApi.resolveCloudRepository = async (_storageId, address) => {
  await wait(400);
  const text = address.trim().replace(/^https?:\/\//i, "");
  if (/^(www\.)?github\.com\//i.test(text)) return { kind: "github" };
  if (/^staging\.mallard-ai\.com\//i.test(text)) return { kind: "wrong_service", host: "staging.mallard-ai.com", expected: HOST };
  const path = text.replace(/^mallard-ai\.com\//i, "").toLowerCase();
  if (!/^[a-z0-9._-]+\/[a-z0-9._-]+$/.test(path)) return { kind: "invalid" };
  if (path === `${LOGIN.toLowerCase()}/portal`) return { kind: "not_catalog", address: `${LOGIN}/portal` };
  const hit = BY_ADDRESS.find((candidate) => candidate.address.toLowerCase() === path);
  return hit ? { kind: "found", repository: hit } : { kind: "not_found", address: path };
};
storageCatalogApi.joinCloudRepository = async () => {
  await wait(500);
  return { kind: "found", repository: SHARED[0] };
};
storageCatalogApi.linkScopeStorage = async () => ({ link_id: "link-pets-cloud", revision: 1, completion: "open" });
storageCatalogApi.connectCloudRepository = async () => { await wait(300); };

/* ——— the view ———————————————————————————————————————————————————————— */

type Variant = "today" | "bar" | "head" | "shared";
type Board = "frame" | "compare" | "standards";
type Weight = "boxed" | "primary";
interface View {
  board: Board;
  variant: Variant;
  quiet: boolean;
  /** A row with nothing on this Mac and nothing stored. */
  empty: boolean;
  added: PetId[];
  note: string | null;
}
type Go = (next: Partial<View>) => void;
const START: View = { board: "frame", variant: "today", quiet: false, empty: false, added: [], note: null };

const inventoryOf = (view: View) => (view.empty ? [] : view.quiet ? QUIET_INVENTORY : INVENTORY);
const listedIds = (view: View): PetId[] => [...inventoryOf(view).map(([id]) => id), ...view.added];
const sharedFor = (view: View) => (view.quiet || view.empty ? [] : SHARED.filter((row) => !view.added.includes(row.resource_id as PetId)));

/* ——— the shell ————————————————————————————————————————————————————— */

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="sync-app" style={{ "--sync-sidebar-width": "248px" } as CSSProperties}>
      <ProjectSidebar
        projects={[]} drafts={[]} activeDraftId={null} storages={[]} storageDrafts={[]} storageProviders={[]} storageUsage={{}}
        activeProjectId={null} activeStorageId={null} catalogScopes={SCOPES} activeCatalogScopeId={SCOPE_ID}
        addingProject={false} addingCatalogScope={false} addingStorage={false} catalogIndexOpen={false} storageIndexOpen={false} settingsOpen={false}
        loading={false} busy={false} activityOpen={false} unreadLogs={0}
        onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop} onDiscardDraft={noop}
        onToggleActivity={noop} onAddProject={noop} onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
        onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop} onOpenCatalogScope={noop} onAddCatalogScope={noop}
        onRemoveCatalogScope={noop} onOpenStorageIndex={noop} onOpenSettings={noop}
      />
      <div className="sync-workspace">{children}</div>
    </div>
  );
}

/** The shell's own note, under the bar, as the app places it. */
function ShellNote({ note, onDismiss, children }: { note: string | null; onDismiss: () => void; children: ReactNode }) {
  const items: NoticeItem[] = note ? [{ key: note, severity: "success", message: note, onDismiss }] : [];
  return (
    <PathBarBelow.Provider value={{ items, subject: "messages from Mallard", className: "sync-backend-banner", timing: "progress" }}>
      {children}
    </PathBarBelow.Provider>
  );
}

/* ——— the bar ——————————————————————————————————————————————————————— */

const STORAGE_CRUMB: PathBarCrumb = {
  id: "storage", label: STORAGE_NAME, icon: "storage-mallard", tone: "root",
  title: `${STORAGE_NAME} · ${LOGIN} · ${HOST}`, ariaLabel: `${STORAGE_NAME}. Storage menu`, menuLabel: "Storage",
  menuHeader: { label: STORAGE_NAME, detail: `${LOGIN} · ${HOST}`, icon: "storage-mallard" },
  menu: [
    { items: [
      { id: "details", label: "Storage details…", icon: "settings", onSelect: noop },
      { id: "unlink", label: `Unlink from ${ROW_LABEL}…`, icon: "link-off", danger: true, onSelect: noop },
    ] },
    { items: [{ id: "add-storage", label: "Add storage…", icon: "plus", onSelect: noop }] },
  ],
};
const ROW_CRUMB: PathBarCrumb = {
  id: "scope", label: ROW_LABEL, icon: "catalog-pet-v2", title: `${ROW_LABEL}\n${HOME}`,
  ariaLabel: `${ROW_LABEL}. Catalog row menu`, menuLabel: "Catalog row",
  menuHeader: { label: ROW_LABEL, detail: HOME, detailIsPath: true, icon: "catalog-pet-v2" },
  menu: [
    { items: [
      { id: "rescan", label: "Rescan this provider home", icon: "refresh", onSelect: noop },
      { id: "rename", label: "Rename profile…", icon: "edit", onSelect: noop },
      { id: "remove", label: "Remove this Catalog row…", icon: "trash", danger: true, onSelect: noop },
    ] },
    { items: [{ id: "add-scope", label: "Add Catalog resource…", icon: "plus", onSelect: noop }] },
  ],
};

/** What the verb explains once its label says what it is. */
const ADD_TITLE = "Someone else’s pet: shared with you, or by address or invite code";

function BarActions({ variant, weight, pull, push, onAdd }: {
  variant: Variant; weight: Weight; pull: number; push: number; onAdd: () => void;
}) {
  return (
    <>
      {variant === "today" && (
        /* As shipped: the bar's round action, as refresh is, its words in
           the tooltip. */
        <button type="button" className="sync-path-icon-action" title="Add from Mallard Cloud…" aria-label="Add from Mallard Cloud" aria-haspopup="dialog" onClick={onAdd}>
          <Icon name="plus" size={16} />
        </button>
      )}
      {variant === "bar" && (
        /* A: the bar's verb box, as Pull and Push wear it. Primary borrows
           the open-review fill (`active`) only to try the weight. */
        <button type="button" className={`sync-path-sync${weight === "primary" ? " active" : ""}`} aria-haspopup="dialog" title={ADD_TITLE} onClick={onAdd}>
          <Icon name="plus" size={14} />
          <span className="sync-path-sync-label">Add from Mallard Cloud…</span>
        </button>
      )}
      <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" disabled={pull === 0} title={pull === 0 ? `Nothing newer in ${STORAGE_NAME}` : undefined}>
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
          {pull > 0 && <span className="sync-path-count" aria-hidden="true">{pull}</span>}
        </button>
        <button type="button" className="sync-path-sync" disabled={push === 0} title={push === 0 ? "Nothing newer on this computer" : undefined}>
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {push > 0 && <span className="sync-path-count" aria-hidden="true">{push}</span>}
        </button>
      </div>
    </>
  );
}

/* ——— the list head —————————————————————————————————————————————————— */

/** The page's own boxed button; Primary tries the filled one. The label is
 *  the dialog's title, and the ellipsis says a dialog follows. On C the
 *  count is what Shared with you holds. */
function AddVerb({ weight, badge, onAdd }: { weight: Weight; badge: number; onAdd: () => void }) {
  return (
    <button type="button" className={`btn${weight === "primary" ? " btn-primary" : ""}`} aria-haspopup="dialog" title={ADD_TITLE} onClick={onAdd}>
      <Icon name="plus" size={13} />
      Add from Mallard Cloud…
      {badge > 0 && (
        <span className="sync-path-count" title={`${badge} shared with you`} aria-label={`${badge} shared with you`}>{badge}</span>
      )}
    </button>
  );
}

/** The shipped search row (`NameSearchField`), drawn by hand. B puts Add
 *  from Mallard Cloud… beside it, outside its rule, in a row of its own
 *  (`sync-list-head`); the count leaves the page, and the filter's answer
 *  stays for screen readers only — the shipped field's `count={false}`.
 *  The head before the cutover keeps the count. */
function ListHead({ variant, weight, query, onQuery, shown, total, shared, onAdd }: {
  variant: Variant; weight: Weight; query: string; onQuery: (query: string) => void; shown: number; total: number; shared: number; onAdd: () => void;
}) {
  const verb = variant === "head" || variant === "shared";
  const filtering = query.trim().length > 0;
  const field = (
    <div className="sync-name-search">
      <div className="sync-name-search-input">
        <Icon name="search" size={14} />
        <input type="search" value={query} placeholder="Search pets v2…" aria-label="Search pets v2" onChange={(event) => onQuery(event.target.value)} />
      </div>
      {verb ? (
        <span className="sync-visually-hidden" role="status">{filtering ? `${shown} of ${total} ${nounOf(total)}` : ""}</span>
      ) : (
        <div className="sync-name-search-results" aria-live="polite" aria-atomic="true">
          <span className="sync-name-search-count">
            {filtering ? <><b>{shown}</b><span> of {total} {nounOf(total)}</span></> : <>{total} {nounOf(total)}</>}
          </span>
        </div>
      )}
    </div>
  );
  if (!verb) return field;
  return (
    <div className="sync-list-head">
      {field}
      <AddVerb weight={weight} badge={variant === "shared" ? shared : 0} onAdd={onAdd} />
    </div>
  );
}

/* ——— the Catalog card, as shipped ——————————————————————————————————————— */

const TONE: Record<string, string> = {
  synced: "synced", "local-outstanding": "local", "cloud-outstanding": "storage", divergent: "diverged", unavailable: "unavailable", untracked: "untracked",
};

function Badge({ row }: { row: CatalogScopeRow }) {
  const status = rowStatus(row);
  const label = rowStatusLabel(row, STORAGE_NAME);
  return (
    <span className={`sync-thread-sync-indicator ${TONE[status] ?? "unknown"}`} data-tooltip={label} aria-label={label} tabIndex={0}>
      <ThreadStatusIcon status={status} size={18} />
    </span>
  );
}

function Launches({ row }: { row: CatalogScopeRow }) {
  return (
    <div className="sync-history-thread-actions">
      <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Codex" aria-label="Open in Codex">
        <ProviderMark provider="codex" displayName="Codex" />
      </button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Terminal" aria-label="Open in Terminal"><Icon name="terminal" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!row.local_present} title={row.local_present ? `Fork ${row.display_name} — duplicate this pet v2 under a new name` : "Not on this Mac, so there is nothing to fork"} aria-label={`Fork ${row.display_name}`}><Icon name="git-fork" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!row.local_path} title={row.local_path ? `Reveal ${row.local_path} in Finder` : "Not on this Mac"} aria-label={`Reveal ${row.display_name} in Finder`}><Icon name="folder" size={14} /></button>
    </div>
  );
}

function Metrics({ row }: { row: CatalogScopeRow }) {
  const stamp = rowStamp(row);
  return (
    <div className="sync-history-session-details">
      <div className="sync-history-session-summary">
        <div className="sync-history-thread-meta" aria-label={`${row.display_name} details`}>
          <span className="sync-history-thread-metric" data-tooltip={row.local_files == null ? "Not on this Mac, so nothing was counted" : `Files · ${row.local_files}`} tabIndex={0}>
            <Icon name="file" size={12} /><b>{row.local_files == null ? "—" : row.local_files}</b>
          </span>
          <span className="sync-history-thread-metric" data-tooltip={row.local_bytes == null ? "Not on this Mac, so nothing was measured" : `On disk · ${formatBytes(row.local_bytes)}`} tabIndex={0}>
            <Icon name="drive" size={12} /><b>{row.local_bytes == null ? "—" : formatBytes(row.local_bytes)}</b>
          </span>
          <span className="sync-history-thread-metric" aria-hidden="true" />
        </div>
        <span className="sync-history-actions-divider" aria-hidden="true" />
        <div className="sync-history-row-clickables">
          <button type="button" className="btn btn-ghost sync-history-launch-action" title="Pet v2 info" aria-label="Pet v2 info"><Icon name="info" size={13} /></button>
        </div>
        <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={stamp?.detail} aria-label={stamp?.detail} tabIndex={0}>
          <time>{stamp ? formatFullDateTime(stamp.seconds) : "—"}</time>
        </span>
      </div>
    </div>
  );
}

function ItemCard({ row }: { row: CatalogScopeRow }) {
  return (
    <article className="sync-history-thread-card catalog-card catalog-card-pet">
      <span className="catalog-pet-preview" data-tooltip={row.description ?? undefined} tabIndex={0}>
        <img src={row.preview_png!} alt={`${row.display_name} preview`} />
      </span>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <Badge row={row} />
          <button type="button" className="sync-history-session-title-button" title={`${row.display_name} · View pet details`}>
            {row.display_name}
          </button>
        </div>
        <Launches row={row} />
      </div>
      <Metrics row={row} />
    </article>
  );
}

/** A repository shared with you, as a Catalog card: the pet's preview, its
 *  name, who shares it, and Add — the dialog's Shared with you row, on the
 *  page. Add lists the pet; nothing is pulled, and the pet keeps this
 *  repository. */
function SharedCard({ repository, onAdd }: { repository: CatalogCloudRepository; onAdd: () => void }) {
  const it = PETS[repository.resource_id as PetId];
  const role = capitalize(repository.role ?? "contributor");
  const who = `Shared by ${repository.owner_login} · ${capitalize(repository.visibility)} · ${role}`;
  const sentence = `${who}. Add lists it here; Pull downloads it.`;
  const when = `Updated ${formatRelativeTime(repository.updated_at)}`;
  return (
    <article className="sync-history-thread-card catalog-card catalog-card-pet" aria-label={`${it.name}, shared with you`}>
      <span className="catalog-pet-preview" data-tooltip={it.description} tabIndex={0}>
        <img src={it.art} alt={`${it.name} preview`} />
      </span>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <span className="sync-thread-sync-indicator storage" data-tooltip={sentence} aria-label={sentence} tabIndex={0}>
            <Icon name="cloud-user" size={18} />
          </span>
          <strong title={it.name}>{it.name}</strong>
          <span className="sync-history-branch-state current" title={who}>{role}</span>
        </div>
        <div className="sync-history-thread-actions">
          <button type="button" className="btn" title={`Add ${it.name} from ${repository.address} — it keeps this repository`} onClick={onAdd}>
            <Icon name="plus" size={13} /> Add
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${it.name} details`}>
            <span className="sync-history-thread-metric" data-tooltip={`Repository · ${repository.address}`} tabIndex={0}>
              <Icon name="cloud" size={12} /><b>{repository.address}</b>
            </span>
            <span className="sync-history-thread-metric" data-tooltip={`Version ${repository.revision} in ${repository.address}`} tabIndex={0}>
              <Icon name="git-commit" size={12} /><b>v{repository.revision}</b>
            </span>
            <span className="sync-history-thread-metric" aria-hidden="true" />
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button type="button" className="btn btn-ghost sync-history-launch-action" title="Pet v2 info" aria-label="Pet v2 info"><Icon name="info" size={13} /></button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={when} aria-label={when} tabIndex={0}>
            <time dateTime={new Date(repository.updated_at * 1000).toISOString()}>{formatFullDateTime(repository.updated_at)}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

/* ——— the sections —————————————————————————————————————————————————————— */

function Sections({ rows, shared, variant, onAddShared }: {
  rows: CatalogScopeRow[]; shared: CatalogCloudRepository[]; variant: Variant; onAddShared: (repository: CatalogCloudRepository) => void;
}) {
  const cloudRows = rows.filter((row) => !row.local_present && row.remote_present);
  const localRows = rows.filter((row) => row.local_present || !row.remote_present);
  const sharedWords = `${shared.length} ${nounOf(shared.length)} shared with you, not listed here yet`;
  return (
    <>
      {cloudRows.length > 0 && (
        <>
          <div className="sync-history-section-heading">
            <h2 title={`Only in ${STORAGE_NAME}`}><Icon name="storage-mallard" size={14} />Not downloaded</h2>
            <span className="sync-history-heading-count" title={`${cloudRows.length} ${nounOf(cloudRows.length)} only in ${STORAGE_NAME}`} aria-label={`${cloudRows.length} ${nounOf(cloudRows.length)} only in ${STORAGE_NAME}`}>
              <Icon name="catalog-pet-v2" size={12} />{cloudRows.length}
            </span>
          </div>
          <div className="sync-history-thread-list">{cloudRows.map((row) => <ItemCard key={row.row_id} row={row} />)}</div>
        </>
      )}
      {localRows.length > 0 && (
        <>
          <div className="sync-history-section-heading">
            <h2><Icon name="catalog-pet-v2" size={14} />Pets v2</h2>
          </div>
          <div className="sync-history-thread-list">{localRows.map((row) => <ItemCard key={row.row_id} row={row} />)}</div>
        </>
      )}
      {variant === "shared" && shared.length > 0 && (
        /* C: offers after the inventory, under the page's own heading
           grammar, with the storage-only heading's count. */
        <>
          <div className="sync-history-section-heading">
            <h2 title="Repositories you belong to whose pet isn’t listed here yet"><Icon name="cloud-user" size={14} />Shared with you</h2>
            <span className="sync-history-heading-count" title={sharedWords} aria-label={sharedWords}>
              <Icon name="catalog-pet-v2" size={12} />{shared.length}
            </span>
          </div>
          <div className="sync-history-thread-list">
            {shared.map((row) => <SharedCard key={row.repository_id} repository={row} onAdd={() => onAddShared(row)} />)}
          </div>
        </>
      )}
    </>
  );
}

/* ——— the Catalog row page ——————————————————————————————————————————————— */

function ListPage({ view, weight, go, onOpenDialog }: { view: View; weight: Weight; go: Go; onOpenDialog: () => void }) {
  const [query, setQuery] = useState("");
  const rows = [...view.added.map((id) => rowOf(id, "local_missing")), ...inventoryOf(view).map(([id, state]) => rowOf(id, state))];
  const shown = rows.filter((row) => row.display_name.toLowerCase().includes(query.trim().toLowerCase()));
  const shared = sharedFor(view);
  const counts = rows.reduce((totals, row) => {
    const status = rowStatus(row);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    return totals;
  }, { push: 0, pull: 0 });
  const addShared = (repository: CatalogCloudRepository) => go({
    added: [...view.added, repository.resource_id as PetId],
    note: `${repository.display_name} added from ${repository.address}.`,
  });
  return (
    <ShellNote note={view.note} onDismiss={() => go({ note: null })}>
      <SyncPathBar
        crumbs={[STORAGE_CRUMB, ROW_CRUMB]}
        ariaLabel={`${ROW_LABEL} path`}
        actions={<BarActions variant={view.variant} weight={weight} pull={counts.pull} push={counts.push} onAdd={onOpenDialog} />}
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        {rows.length === 0 ? (
          /* The shipped empty state, with the verb beside its sentence: the
             way in never depends on there being a list to head. */
          <div className="sync-history-state">
            No pet v2 on this Mac, and none stored yet.
            <AddVerb weight={weight} badge={0} onAdd={onOpenDialog} />
          </div>
        ) : (
          <section className="profile-links-section" aria-label={ROW_LABEL}>
            <ListHead variant={view.variant} weight={weight} query={query} onQuery={setQuery} shown={shown.length} total={rows.length} shared={shared.length} onAdd={onOpenDialog} />
            <Sections rows={shown} shared={shared} variant={view.variant} onAddShared={addShared} />
          </section>
        )}
      </main>
    </ShellNote>
  );
}

/* ——— the boards ———————————————————————————————————————————————————————— */

function CompareBoard({ weight, onAdd }: { weight: Weight; onAdd: () => void }) {
  const strips: Array<{ variant: Variant; title: string; note: string }> = [
    { variant: "today", title: "Today", note: "Round and icon-only, in refresh’s box, among the sync verbs. The head ends in a count." },
    { variant: "head", title: "B · List head, picked", note: "The verb at the head’s end, beside the search. The search keeps its rule, which stops before the verb; the count is gone." },
  ];
  return (
    <div className="compare">
      {strips.map((strip) => (
        <figure key={strip.variant}>
          <figcaption>{strip.title}<small>{strip.note}</small></figcaption>
          <div className="compare-strip">
            <div className="sync-app">
              <div className="sync-workspace">
                <SyncPathBar crumbs={[STORAGE_CRUMB, ROW_CRUMB]} ariaLabel={`${strip.title} path`}
                  actions={<BarActions variant={strip.variant} weight={weight} pull={1} push={2} onAdd={onAdd} />} />
                <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
                  <section className="profile-links-section" aria-label={strip.title}>
                    <ListHead variant={strip.variant} weight={weight} query="" onQuery={noop} shown={4} total={4} shared={0} onAdd={onAdd} />
                  </section>
                </main>
              </div>
            </div>
          </div>
        </figure>
      ))}
    </div>
  );
}

interface Standard {
  who: string;
  what: string;
  url: string;
  says: string;
  here: string;
}
const STANDARDS: Standard[] = [
  {
    who: "Nielsen Norman Group", what: "Icon Usability · Yes, Icons Need Text Labels", url: "https://www.nngroup.com/articles/icon-usability/",
    says: "Almost every icon is ambiguous; a visible text label resolves it. Labels must show without hover — a tooltip raises the interaction cost and never shows on touch. In their study, labeled navigation was used 20% faster and with more confidence.",
    here: "Today the + is labeled only by its tooltip. A, B and C put the words on screen.",
  },
  {
    who: "Apple Human Interface Guidelines", what: "Toolbars", url: "https://developer.apple.com/design/human-interface-guidelines/toolbars",
    says: "Make the meaning of each toolbar item clear; people shouldn’t have to guess or experiment. Provide a symbol and a label, and prefer verbs and verb phrases.",
    here: "“Add from Mallard Cloud…” is a verb phrase. The ellipsis says a dialog follows, as menus say it.",
  },
  {
    who: "Material Design 3", what: "FAB and extended FAB", url: "https://m3.material.io/components/extended-fab/guidelines",
    says: "A FAB promotes an important, constructive action such as Create. Use the extended FAB, with a text label, when an icon alone is not enough to show what the action is.",
    here: "Add is this page’s constructive action, and + alone does not say what it adds or from where. It earns a label.",
  },
  {
    who: "Microsoft Fluent / WinUI", what: "Command bar", url: "https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/command-bar",
    says: "Primary commands are labeled and ordered by importance so the important ones stay visible; secondary commands go to the overflow.",
    here: "For the list, Add is a primary command and refresh is maintenance. Today the + wears refresh’s box, so it reads as maintenance.",
  },
  {
    who: "GitHub Desktop", what: "The repository list", url: "https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-and-forking-repositories-from-github-desktop",
    says: "A labeled Add ▾ (Clone repository…, Create new repository…, Add existing repository…) heads the repository list; Fetch, Pull and Push live in the toolbar.",
    here: "B: list verbs in the list head, sync verbs in the bar.",
  },
  {
    who: "Spotify", what: "Your Library", url: "https://community.spotify.com/t5/Your-Library/Desktop-New-Your-Library-sidebar/td-p/5571384",
    says: "The + that creates playlists and folders sits at the top of Your Library — the list it adds to — not among the player’s controls.",
    here: "B’s placement: the verb beside the list’s search and count, where the eye asks “how do I get more of these?”.",
  },
  {
    who: "Google Drive, Dropbox, Slack", what: "Shared with me", url: "https://drive.google.com/drive/shared-with-me",
    says: "What others shared with you is a place you can see and act on, not something you discover by typing an address.",
    here: "C: Shared with you on the page, each row with its own Add. The dialog keeps the address and invite-code way in.",
  },
  {
    who: "WCAG 2.2", what: "2.5.3 Label in Name · 4.1.2 Name, Role, Value", url: "https://www.w3.org/WAI/WCAG22/Understanding/label-in-name.html",
    says: "An icon-only control needs an accessible name. Once a control shows text, its accessible name must contain that text, so voice users can say what they see.",
    here: "With a visible label the button’s name is its label, “Add from Mallard Cloud”, and the tooltip is free to explain rather than to name.",
  },
];

function StandardsBoard() {
  return (
    <div className="standards">
      <p>What the references say about a verb like this one, and what each means for the Catalog row page. Links open the source.</p>
      <table>
        <thead>
          <tr><th>Source</th><th>What it says</th><th>Here</th></tr>
        </thead>
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
  view: View;
  shows: string;
  mirrors: string;
  today: string;
  tradeoff?: string;
  /** A direction not picked, kept for the record. */
  aside?: boolean;
}
const SCENES: Scene[] = [
  {
    id: "today", title: "Before", view: START,
    shows: "The page as shipped until 2026-10-02: the round + (Add from Mallard Cloud…) in the bar, then refresh, Pull 1, Push 2. The + wears refresh’s box, carries no label, and sits in the sync cluster; its words live in a tooltip. The list head carries the search and a count.",
    mirrors: "—",
    today: "What B replaced.",
  },
  {
    id: "head", title: "B · List head", view: { ...START, variant: "head" },
    shows: "Picked (owner, 2026-10-02), refined: the labeled verb — + Add from Mallard Cloud… — beside the search, in the list it adds to. The search keeps its own rule, which stops before the verb; the count (“4 pets v2”) leaves the page, and the filter still tells a screen reader “2 of 4 pets v2”. The bar is sync only: refresh, Pull, Push. Boxed by default; Weight: Primary tries the filled button; Width: Narrow checks the squeeze. Click the verb: the dialog is the shipped one.",
    mirrors: "GitHub Desktop’s Add ▾ over the repository list; GitHub’s New button on a repositories list; Fluent’s command bar: the list’s primary command, labeled.",
    today: "The + sits among the sync verbs, and the head ends in the count.",
    tradeoff: "NameSearchField gains a trailing slot and a way to hold its count back; two App.css rules give the row its shape (this page’s own style block holds them for now).",
  },
  {
    id: "compare", title: "Side by side", view: { ...START, board: "compare" },
    shows: "Today’s bar and head over B’s, at a glance: the + leaves the sync cluster, the verb takes the head’s end, the rule stops before it, and the count goes.",
    mirrors: "The Catalog row page’s own bar and list head.",
    today: "The first strip.",
  },
  {
    id: "added", title: "Added", view: { ...START, variant: "head", added: ["biscuit"], note: "Biscuit added from kai/Biscuit." },
    shows: "After Add from the dialog: the shell note, Biscuit under Not downloaded with Pull 2. Nothing is pulled on its own, and Biscuit keeps kai/Biscuit.",
    mirrors: "Add from Mallard Cloud as the plan lands it: Only in Mallard Cloud, Pull to download.",
    today: "The same landing, from the bar’s + only.",
  },
  {
    id: "quiet", title: "Nothing to add", view: { ...START, variant: "head", quiet: true },
    shows: "B when Mallard Cloud holds nothing this Mac lacks and nothing is shared: no Not downloaded, Pull closed. The head’s verb is the stable way in — by address or invite code — and never disappears.",
    mirrors: "An empty Shared with me still has New.",
    today: "The round + in the bar, enabled, saying nothing.",
  },
  {
    id: "empty", title: "Empty row", view: { ...START, variant: "head", empty: true },
    shows: "A row with nothing on this Mac and nothing stored: the shipped empty state keeps the verb beside its sentence, so the way in never depends on there being a list to head. Added at the cutover; B’s head needs rows to exist.",
    mirrors: "An empty list that still offers New.",
    today: "The bar’s + was the only way in.",
  },
  {
    id: "standards", title: "Standards", view: { ...START, board: "standards" },
    shows: "The references behind the pick, and what each means here.",
    mirrors: "—",
    today: "—",
  },
  {
    id: "bar", title: "A · Label in the bar", aside: true, view: { ...START, variant: "bar" },
    shows: "Not picked. The same verb with its label in the bar, boxed as Pull and Push are, before refresh. Kept for the record: three labeled verbs in the bar, Add among the sync verbs, and a cluster that never shrinks, so the trail pays on a narrow window (Width: Narrow).",
    mirrors: "Apple HIG toolbars: a symbol and a verb label.",
    today: "Icon only; the label is a tooltip.",
  },
  {
    id: "shared", title: "C · Shared with you", aside: true, view: { ...START, variant: "shared" },
    shows: "Not picked. B, and a Shared with you section listing the repositories you belong to whose pet no row has yet, each with its own Add; the head’s verb carries the count. Kept for the record: it can sit on B later, at the cost of one listing per page load.",
    mirrors: "Google Drive and Dropbox’s Shared with me; Slack’s channels you can join.",
    today: "Shared with you exists only inside the dialog.",
  },
];

/** Whether two views show the same step; the shell note does not count. */
const sameView = (a: View, b: View) => a.board === b.board && a.variant === b.variant && a.quiet === b.quiet && a.empty === b.empty && a.added.join() === b.added.join();

function Harness() {
  const params = new URLSearchParams(window.location.search);
  const initial = Math.max(0, SCENES.findIndex((scene) => scene.id === (params.get("scene") ?? "head")));
  const [sceneIndex, setSceneIndex] = useState(initial);
  const [view, setView] = useState<View>(SCENES[initial].view);
  const [theme, setTheme] = useState(params.get("theme") === "dark" ? "dark" : "light");
  const [width, setWidth] = useState<"wide" | "narrow">(params.get("width") === "narrow" ? "narrow" : "wide");
  const [weight, setWeight] = useState<Weight>(params.get("weight") === "primary" ? "primary" : "boxed");
  const [dialog, setDialog] = useState(false);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => { listedNow = new Set(listedIds(view)); }, [view]);
  const scene = SCENES[sceneIndex];

  /* Clicking through lands on later steps: the tabs and the notes follow. */
  const matched = SCENES.findIndex((candidate) => sameView(candidate.view, view));
  useEffect(() => {
    if (matched < 0) return;
    setSceneIndex(matched);
    const url = new URL(window.location.href);
    url.searchParams.set("scene", SCENES[matched].id);
    window.history.replaceState(null, "", url);
  }, [matched]);

  const jump = (index: number) => {
    setDialog(false);
    setView(SCENES[index].view);
  };
  const go: Go = (next) => setView((current) => ({ ...current, ...next }));
  const added = (resourceId: string, name: string, address: string) => {
    setDialog(false);
    go({ added: [...view.added, resourceId as PetId], note: `${name} added from ${address}.` });
  };

  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Add from Mallard Cloud — B, refined</strong>
        <nav className="harness-scenes" aria-label="Scenes">
          {SCENES.map((candidate, index) => (
            <button key={candidate.id} type="button"
              className={[index === sceneIndex ? "active" : "", candidate.id === "today" ? "today" : "", candidate.id === "head" ? "pick" : "", candidate.aside ? "aside" : ""].filter(Boolean).join(" ") || undefined}
              aria-current={index === sceneIndex ? "step" : undefined} onClick={() => jump(index)}>
              <b>{index}</b> {candidate.title}
            </button>
          ))}
        </nav>
        <div className="harness-toggles">
          <span><b>Width</b>
            <button type="button" aria-pressed={width === "wide"} onClick={() => setWidth("wide")}>Wide</button>
            <button type="button" aria-pressed={width === "narrow"} onClick={() => setWidth("narrow")}>Narrow</button>
          </span>
          <span><b>Weight</b>
            <button type="button" aria-pressed={weight === "boxed"} onClick={() => setWeight("boxed")}>Boxed</button>
            <button type="button" aria-pressed={weight === "primary"} onClick={() => setWeight("primary")}>Primary</button>
          </span>
          <label><input type="checkbox" checked={theme === "dark"} onChange={(event) => setTheme(event.target.checked ? "dark" : "light")} /> Dark</label>
        </div>
      </div>
      <div className="harness-notes">
        <p><b>{sceneIndex} · {scene.title}.</b> {scene.shows}</p>
        {scene.mirrors !== "—" && <p><span>Mirrors</span>{scene.mirrors}</p>}
        {scene.today !== "—" && <p><span>Today</span>{scene.today}</p>}
        {scene.tradeoff && <p><span>Trade-off</span>{scene.tradeoff}</p>}
      </div>
      {view.board === "compare" ? <CompareBoard weight={weight} onAdd={() => setDialog(true)} />
        : view.board === "standards" ? <StandardsBoard />
          : (
            <div className={`harness-frame${width === "narrow" ? " narrow" : ""}`}>
              <Shell>
                <ListPage key={`${view.variant}:${view.quiet}:${view.empty}`} view={view} weight={weight} go={go} onOpenDialog={() => setDialog(true)} />
              </Shell>
            </div>
          )}
      {dialog && (
        <CatalogConnectDialog
          reason={{ kind: "add" }} scopeId={SCOPE_ID} storageId={STORAGE_ID} accountLogin={LOGIN} coordinate={COORDINATE}
          noun="pet v2" rowLabel={ROW_LABEL} listed={new Set(listedIds(view))} capabilities={[CAPABILITY]}
          onClose={() => setDialog(false)} onConnected={(connected) => added(connected.resourceId, connected.name, connected.address)}
        />
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Harness />
  </StrictMode>,
);
