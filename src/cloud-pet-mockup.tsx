import { StrictMode, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { formatFullDateTime } from "./clock";
import { formatBytes } from "./format";
import Icon, { type IconName } from "./components/Icons";
import ManagedCloudChip, { ManagedCloudPanel } from "./components/managed-cloud/ManagedCloudChip";
import RepositoryLookupField, { isInvitationCodeShaped } from "./components/managed-cloud/RepositoryLookupField";
import { cloudStatusNotice, cloudSyncBlocked } from "./components/managed-cloud/managedCloudLink";
import type { ManagedCloudLink, ManagedCloudStatus } from "./components/managed-cloud/types";
import Notice, { Acknowledgement, FieldMessage, NoticeZone, type NoticeItem } from "./components/notice/Notice";
import ProjectSidebar from "./components/project-sync/ProjectSidebar";
import { ModeDoor } from "./components/project-sync/StorageSettings";
import { buildPullOutcome, buildPushOutcome, projectOutcomeAction, type SyncOutcome } from "./components/project-sync/syncOutcome";
import type { PullReviewItem } from "./components/project-sync/pullReviewModel";
import { predictRepositoryAddress } from "./components/managed-cloud/repositoryAddress";
import { formatRelativeTime } from "./clock";
import StorageCatalogPage, { type CatalogStorageOption } from "./components/storage-catalog/StorageCatalogPage";
import AddCatalogScopeWorkspace, { type CatalogCloudLanding } from "./components/storage-catalog/AddCatalogScopeWorkspace";
import { storageCatalogApi } from "./components/storage-catalog/api";
import { rowStamp, rowStatus, rowStatusLabel } from "./components/storage-catalog/model";
import type {
  CatalogActionOutcome,
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogLinkState,
  CatalogPullAction,
  CatalogScopeRow,
  CatalogScopeView,
} from "./components/storage-catalog/types";
import NameSearchField from "./components/sync-review/NameSearchField";
import ReviewPreparing from "./components/sync-review/ReviewPreparing";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncOutcomeNotice from "./components/sync-review/SyncOutcomeNotice";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import TopLayer from "./components/sync-review/TopLayer";
import { focusMenuEdge, moveMenuFocus } from "./components/sync-review/menuKeyboard";
import type { OutcomeAction, SyncOutcomeRow, SyncOutcomeStat, SyncOutcomeView } from "./components/sync-review/outcome";
import type { PullReview, RestoreResult, ThreadSyncComparison } from "./types";
import "./App.css";

/**
 * Specimen for docs/PLAN_CATALOG_CLOUD_LIKE_PROJECTS.md: pets and skills on
 * Mallard Cloud, the way a project is.
 *
 * Scene 0 is today: the shipped Catalog row page, live, against a stand-in
 * for the backend it calls (per-row chips, Connect Cloud repository, the
 * inline Cloud repositories panel, Push and Pull all work; the sidebar
 * switches it between Pets v2 and Skills). The proposed scenes are drawn
 * from the same shipped parts and classes — the Catalog card, the session
 * title button, the project page's Storage card holding the connect dialog's
 * New repository row, the review readouts and action chips, the Cloud chip
 * and panel, the project's connect dialog — so nothing on them is a new
 * style. Every chip word,
 * panel verb and workspace note comes from managedCloudLink.ts; every badge
 * sentence from the Catalog's model.ts.
 *
 * `npm run dev`, then /cloud-pet-mockup.html (`?scene=today|compare|list|pet|
 * connect|connect-states|connected|connected-existing|push|pushed|states|
 * batch|pull-pets|pulled-pets|skills|pull-skills|pulled-skills|add|add-door|
 * add-states|added|several|other`, `?theme=dark`).
 *
 * Only Mallard Cloud changes. The storage crumb switches to Vault, a local
 * folder, whose Catalog row keeps today's page: the `other` scene.
 *
 * In the dialog these answer: maya/Duck (public), an invite code — any 20
 * letters, e.g. ABCD-EFGH-JKLM-NPQR-STUV — (joins maya/Duck, or what Add
 * brings in), maya/Mochi and maya/changelog-bot (public), maya/pdf-tools (a
 * public skill), Rika321/Owl (another pet), Rika321/portal (a project),
 * Rika321/Duck-classic (Pets v1), a github.com address,
 * staging.mallard-ai.com/… (another service). Anything else is not found.
 */

/* ——— data ————————————————————————————————————————————————————————— */

const LOGIN = "Rika321";
const HOST = "mallard-ai.com";
const STORAGE_NAME = "Mallard Cloud";
const VAULT_NAME = "Vault";
const HOME = "/Users/you/.codex";
const now = Math.floor(Date.now() / 1000);
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
const capitalize = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);

const art = (body: string) => `data:image/svg+xml;utf8,${encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 52">${body}</svg>`,
)}`;
const ART = {
  duck: art(`<ellipse cx="22" cy="35" rx="15" ry="11" fill="#f4c542"/><circle cx="28" cy="20" r="9" fill="#f4c542"/><path d="M36 19l9 2.5-9 3z" fill="#ec8a23"/><circle cx="30" cy="18" r="1.8" fill="#2a2a2a"/><path d="M12 34q8 7 16 0" stroke="#d6a52a" stroke-width="2" fill="none"/>`),
  pip: art(`<path d="M12 18l4-9 6 7z M36 18l-4-9-6 7z" fill="#e98a3c"/><ellipse cx="24" cy="31" rx="15" ry="14" fill="#f09a4a"/><circle cx="19" cy="28" r="2" fill="#2a2a2a"/><circle cx="29" cy="28" r="2" fill="#2a2a2a"/><path d="M22 33q2 2 4 0" stroke="#7a3e12" stroke-width="1.6" fill="none"/>`),
  mossy: art(`<ellipse cx="24" cy="34" rx="17" ry="12" fill="#5fae5a"/><circle cx="16" cy="22" r="6" fill="#5fae5a"/><circle cx="32" cy="22" r="6" fill="#5fae5a"/><circle cx="16" cy="22" r="2.6" fill="#fff"/><circle cx="32" cy="22" r="2.6" fill="#fff"/><circle cx="16.6" cy="22.4" r="1.4" fill="#2a2a2a"/><circle cx="32.6" cy="22.4" r="1.4" fill="#2a2a2a"/><path d="M17 36q7 5 14 0" stroke="#2f6e2c" stroke-width="1.8" fill="none"/>`),
  fable: art(`<ellipse cx="24" cy="30" rx="15" ry="16" fill="#8a6345"/><circle cx="18" cy="25" r="5.5" fill="#f3e6cf"/><circle cx="30" cy="25" r="5.5" fill="#f3e6cf"/><circle cx="18" cy="25" r="2.4" fill="#2a2a2a"/><circle cx="30" cy="25" r="2.4" fill="#2a2a2a"/><path d="M22 30l2 3 2-3z" fill="#e5a23a"/><path d="M11 15l5 5 M37 15l-5 5" stroke="#8a6345" stroke-width="3"/>`),
  biscuit: art(`<ellipse cx="11" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="37" cy="28" rx="5" ry="10" fill="#7a4a24"/><ellipse cx="24" cy="31" rx="14" ry="13" fill="#d9a066"/><ellipse cx="24" cy="38" rx="7" ry="5" fill="#f3e1c4"/><circle cx="19" cy="27" r="2" fill="#2a2a2a"/><circle cx="29" cy="27" r="2" fill="#2a2a2a"/><ellipse cx="24" cy="34.5" rx="2.6" ry="1.8" fill="#2a2a2a"/>`),
  mochi: art(`<ellipse cx="24" cy="32" rx="17" ry="14" fill="#f6c6d4"/><circle cx="18" cy="30" r="1.8" fill="#2a2a2a"/><circle cx="30" cy="30" r="1.8" fill="#2a2a2a"/><ellipse cx="14" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><ellipse cx="34" cy="34" rx="3" ry="1.6" fill="#ee8fab"/><path d="M22 34q2 1.6 4 0" stroke="#a24d68" stroke-width="1.4" fill="none"/>`),
};

/** The two Catalog rows this specimen walks: one with artwork, one without. */
type KindKey = "pets" | "skills";
interface KindInfo {
  scopeId: string;
  label: string;
  singular: string;
  noun: string;
  plural: string;
  /** What a refusal calls it: "… is a project, not a pet." */
  word: string;
  icon: IconName;
  artwork: boolean;
}
const KINDS: Record<KindKey, KindInfo> = {
  pets: { scopeId: "scope-pets-v2", label: "Pets v2", singular: "Pet v2", noun: "pet v2", plural: "pets v2", word: "pet", icon: "catalog-pet-v2", artwork: true },
  skills: { scopeId: "scope-skills", label: "Skills", singular: "Skill", noun: "skill", plural: "skills", word: "skill", icon: "lightbulb", artwork: false },
};
const rowLabel = (kind: KindKey) => `${KINDS[kind].label} · Codex home`;

type ItemId =
  | "duck" | "pip" | "mossy" | "fable" | "mochi" | "biscuit"
  | "deploy-checklist" | "pdf-tools" | "sql-review" | "release-notes" | "commit-style" | "changelog-bot" | "lint-rules"
  /* The copy a keep-local Pull leaves of sql-review: a new skill on this Mac. */
  | "sql-review-2";

interface Item {
  id: ItemId;
  kind: KindKey;
  name: string;
  description: string;
  files: number;
  bytes: number;
  edited: number;
  sha: string;
  art: string | null;
  /** On this Mac before anything is pulled. */
  local: boolean;
}
const item = (id: ItemId, kind: KindKey, name: string, description: string, files: number, bytes: number, edited: number, sha: string, local: boolean): Item => ({
  id, kind, name, description, files, bytes, edited, sha, local, art: kind === "pets" ? ART[id as keyof typeof ART] : null,
});
const ITEMS: Record<ItemId, Item> = {
  duck: item("duck", "pets", "Duck", "A yellow duck that waddles along the dock.", 2, 151_552, now - 2 * 3600, "3f9a0c7e1b2d4a6f8c0e2b4d6f8a1c3e", true),
  pip: item("pip", "pets", "Pip", "An orange kitten that naps on the menu bar.", 2, 98_304, now - 3 * 86400, "a81b3c5d7e9f1a2b4c6d8e0f2a4b6c8d", true),
  mossy: item("mossy", "pets", "Mossy", "A frog that hops when a build finishes.", 3, 212_992, now - 40 * 60, "c02d4e6f8a1b3c5d7e9f0a2b4c6d8e1f", true),
  fable: item("fable", "pets", "Fable", "An owl that watches long runs.", 2, 131_072, now - 9 * 86400, "d13e5f7a9b1c3d5e7f9a1b3c5d7e9f1a", false),
  biscuit: item("biscuit", "pets", "Biscuit", "A beagle that fetches finished builds.", 2, 104_448, now - 5 * 86400, "0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d", false),
  mochi: item("mochi", "pets", "Mochi", "A round rice cake that wobbles when you type.", 2, 87_040, now - 6 * 86400, "e24f6a8b0c2d4e6f8a0b2c4d6e8f0a2b", false),
  "deploy-checklist": item("deploy-checklist", "skills", "deploy-checklist", "Walks a release through staging and production.", 3, 18_432, now - 4 * 86400, "f35a7b9c1d3e5f7a9b1c3d5e7f9a1b3c", false),
  "pdf-tools": item("pdf-tools", "skills", "pdf-tools", "Fills, merges and splits PDFs.", 6, 41_984, now - 2 * 86400, "a46b8c0d2e4f6a8b0c2d4e6f8a0b2c4d", true),
  "sql-review": item("sql-review", "skills", "sql-review", "Reviews migrations for locks and data loss.", 4, 22_528, now - 5 * 3600, "b57c9d1e3f5a7b9c1d3e5f7a9b1c3d5e", true),
  "release-notes": item("release-notes", "skills", "release-notes", "Drafts release notes from merged pull requests.", 2, 9_216, now - 12 * 86400, "c68d0e2f4a6b8c0d2e4f6a8b0c2d4e6f", true),
  "commit-style": item("commit-style", "skills", "commit-style", "The house style for commit messages.", 1, 3_072, now - 20 * 86400, "d79e1f3a5b7c9d1e3f5a7b9c1d3e5f7a", true),
  "changelog-bot": item("changelog-bot", "skills", "changelog-bot", "Keeps CHANGELOG.md in step with tags.", 3, 12_288, now - 3 * 86400, "e80f2a4b6c8d0e2f4a6b8c0d2e4f6a8b", false),
  "lint-rules": item("lint-rules", "skills", "lint-rules", "Fixes lint findings the team’s way.", 2, 6_144, now - 2 * 86400, "1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e", false),
  "sql-review-2": item("sql-review-2", "skills", "sql-review-2", "Reviews migrations for locks and data loss.", 4, 22_528, now - 5 * 3600, "f91a3b5c7d9e1f3a5b7c9d1e3f5a7b9c", true),
};
const folderOf = (id: ItemId) => `${HOME}/${ITEMS[id].kind === "pets" ? "pets" : "skills"}/${id}`;
const copyName = (id: ItemId) => (ITEMS[id].kind === "pets" ? `${ITEMS[id].name} 2` : `${ITEMS[id].name}-2`);

/** Where Duck's repository stands as the click-through moves it. */
type DuckRepository = "none" | "created" | "pushed" | "maya" | "kai";

/** The row's storage. Only Mallard Cloud changes; any other storage keeps
 *  today's page, here a local folder. */
type StorageKind = "cloud" | "vault";

type PullChoice = "install" | "update" | "keep" | "overwrite";

interface View {
  storage: StorageKind;
  kind: KindKey;
  page: "today" | "today-add" | "compare" | "list" | "item" | "push-item" | "push-batch" | "pull-item" | "pull-batch" | "states" | "connect-states" | "add-resource" | "add-states";
  item: ItemId;
  duck: DuckRepository;
  dialog: DialogReason | null;
  outcome: "push" | "pull" | null;
  /** Items a Push or Pull in this walk brought level with the storage. */
  pushed: ItemId[];
  pulled: ItemId[];
  /** What the last Push sent and which repositories it made; what the last
   *  Pull chose per item. The outcome cards read these. */
  lastPush: ItemId[];
  lastPushCreated: ItemId[];
  pullChoices: Partial<Record<ItemId, PullChoice>>;
  note: string | null;
  /** Items brought in with Add from Mallard Cloud. */
  added: ItemId[];
  several: boolean;
  /** Add Catalog resource's open door. */
  door: "local" | "cloud";
}
type DialogReason = "connect" | "several" | "add";
type Go = (next: Partial<View>) => void;

const START: View = {
  storage: "cloud", kind: "pets", page: "list", item: "duck", duck: "none", dialog: null, outcome: null,
  pushed: [], pulled: [], lastPush: [], lastPushCreated: [], pullChoices: {}, note: null, added: [], several: false, door: "local",
};

const storageName = (view: View) => (view.storage === "cloud" ? STORAGE_NAME : VAULT_NAME);

/* ——— repository statuses, in the service's words ———————————————————— */

const link = (owner: string, name: string, of: Partial<ManagedCloudLink> = {}): ManagedCloudLink => ({
  repository_id: `repo_${owner}_${name}`.toLowerCase(), visibility: "private", can_pull: true, can_push: true,
  website_url: `https://${HOST}/${owner}/${name}`, owner_login: owner, name,
  repository_revision: 4, can_request_release: true, role: "owner", release: { kind: "private" }, ...of,
});
const ready = (of: ManagedCloudLink, publication: ManagedCloudStatus["publication"], saved = of.repository_id): ManagedCloudStatus => ({
  repository_id: saved, state: "ready", link: of, publication, message: null,
});
const failed = (state: ManagedCloudStatus["state"], message: string, repositoryId = "repo_rika321_duck"): ManagedCloudStatus => ({
  repository_id: repositoryId, state, link: null, message,
});
const publicRead = { visibility: "public", role: null, release: { kind: "public" }, can_request_release: false } as const;

const STATUS = {
  notPushed: ready(link(LOGIN, "Duck", { release: { kind: "not_pushed" }, can_request_release: false }), { state: "no_draft", private_revision: null, public_revision: null }),
  duckPrivate: ready(link(LOGIN, "Duck"), { state: "not_public", private_revision: 1, public_revision: null }),
  duckMaya: ready(link("maya", "Duck", publicRead), { state: "matches_published", private_revision: 6, public_revision: 6 }),
  duckKai: ready(link("kai", "Duck", { role: "member", can_request_release: false }), { state: "not_public", private_revision: 4, public_revision: null }),
  pip: ready(link(LOGIN, "Pip"), { state: "not_public", private_revision: 3, public_revision: null }),
  mossy: ready(link(LOGIN, "Mossy", { visibility: "public", release: { kind: "snapshot_changed" } }), { state: "unpublished_draft", private_revision: 4, public_revision: 2 }),
  fable: ready(link(LOGIN, "Fable"), { state: "not_public", private_revision: 1, public_revision: null }),
  mochi: ready(link("maya", "Mochi", publicRead), { state: "matches_published", private_revision: 5, public_revision: 5 }),
  biscuit: ready(link("kai", "Biscuit", { role: "member", can_request_release: false }), { state: "not_public", private_revision: 2, public_revision: null }),
  lintRules: ready(link("kai", "lint-rules", { role: "member", can_request_release: false }), { state: "not_public", private_revision: 3, public_revision: null }),
  deploy: ready(link(LOGIN, "deploy-checklist"), { state: "not_public", private_revision: 2, public_revision: null }),
  pdf: ready(link(LOGIN, "pdf-tools"), { state: "not_public", private_revision: 5, public_revision: null }),
  pdfMaya: ready(link("maya", "pdf-tools", publicRead), { state: "matches_published", private_revision: 9, public_revision: 9 }),
  sql: ready(link(LOGIN, "sql-review", { visibility: "public", release: { kind: "snapshot_changed" } }), { state: "unpublished_draft", private_revision: 4, public_revision: 3 }),
  release: failed("inaccessible", "Mallard Cloud refused this account for maya/release-notes.", "repo_maya_release-notes"),
  commit: ready(link(LOGIN, "commit-style"), { state: "not_public", private_revision: 1, public_revision: null }),
  changelog: ready(link("maya", "changelog-bot", publicRead), { state: "matches_published", private_revision: 2, public_revision: 2 }),
  inReview: ready(link(LOGIN, "Duck", { release: { kind: "waiting_owner", requested_by: "user_2" }, can_request_release: false }), { state: "not_public", private_revision: 2, public_revision: null }),
  published: ready(link(LOGIN, "Duck", { visibility: "public", release: { kind: "public" }, can_request_release: false }), { state: "matches_published", private_revision: 2, public_revision: 2 }),
  /* Learned from the account's listing, not yet saved by a Push or Pull:
     the status names no saved repository, so the verbs that name it wait. */
  learned: ready(link(LOGIN, "Fable"), { state: "not_public", private_revision: 1, public_revision: null }, ""),
  gone: failed("gone", "Deleted, or this account lost access. Push creates it again."),
  deleting: failed("deleting", "Push creates it again."),
  signIn: failed("sign_in_required", "Signed out of Mallard Cloud. Sign in again from Storage settings."),
  refused: failed("inaccessible", "Mallard Cloud refused this account. It may be suspended."),
};

/* ——— what the page reads for each item ——————————————————————————————— */

function itemState(id: ItemId, view: View): CatalogLinkState {
  const cloud = view.storage === "cloud";
  if (view.pulled.includes(id) || view.pushed.includes(id)) return "in_sync";
  switch (id) {
    case "duck":
      if (cloud && view.duck === "pushed") return "in_sync";
      if (cloud && (view.duck === "maya" || view.duck === "kai")) return "unreviewed_remote";
      return "new";
    case "pip":
      return cloud && view.several ? "blocked" : "storage_changed";
    case "mossy":
      return "local_changed";
    case "fable":
    case "mochi":
    case "biscuit":
    case "deploy-checklist":
    case "changelog-bot":
    case "lint-rules":
      return "local_missing";
    case "pdf-tools":
      return "storage_changed";
    case "sql-review":
      return "diverged";
    case "release-notes":
      return cloud ? "blocked" : "in_sync";
    case "commit-style":
      return "in_sync";
    case "sql-review-2":
      return "new";
  }
}

function rowOf(id: ItemId, view: View): CatalogScopeRow {
  const it = ITEMS[id];
  const local = it.local || view.pulled.includes(id);
  const state = itemState(id, view);
  return {
    row_id: `resource:${id}`,
    display_name: it.name,
    resource_id: id,
    state,
    local_present: local,
    local_path: local ? folderOf(id) : null,
    local_modified_at: local ? it.edited : null,
    local_files: local ? it.files : null,
    local_bytes: local ? it.bytes : null,
    remote_present: state !== "new",
    remote_updated_at: state === "new" ? null : it.edited - 600,
    content_sha256: it.sha,
    preview_png: it.art,
    description: it.description,
    warnings: [],
    blocker: state !== "blocked" ? null
      : id === "pip" ? "In 2 repositories you can see. Choose one on its page."
        : "Mallard Cloud refused this account for maya/release-notes.",
  };
}

/** The repository the page resolved for an item: saved, or learned. */
function statusOf(id: ItemId, view: View): ManagedCloudStatus | null {
  switch (id) {
    case "duck":
      return view.duck === "none" ? null
        : view.duck === "created" ? STATUS.notPushed
          : view.duck === "maya" ? STATUS.duckMaya : view.duck === "kai" ? STATUS.duckKai : STATUS.duckPrivate;
    case "pip": return view.several ? null : STATUS.pip;
    case "mossy": return STATUS.mossy;
    case "fable": return STATUS.fable;
    case "mochi": return STATUS.mochi;
    case "biscuit": return STATUS.biscuit;
    case "lint-rules": return STATUS.lintRules;
    case "deploy-checklist": return STATUS.deploy;
    case "pdf-tools": return STATUS.pdf;
    case "sql-review": return STATUS.sql;
    case "release-notes": return STATUS.release;
    case "commit-style": return STATUS.commit;
    case "changelog-bot": return STATUS.changelog;
    case "sql-review-2": return null;
  }
}

const PET_LIST: ItemId[] = ["fable", "duck", "pip", "mossy"];
const SKILL_LIST: ItemId[] = ["deploy-checklist", "pdf-tools", "sql-review", "release-notes", "commit-style"];
function listedItems(view: View): ItemId[] {
  const base = view.kind === "pets" ? PET_LIST : SKILL_LIST;
  /* A keep-local Pull leaves the local copy as a new item of its own. */
  const kept: ItemId[] = view.kind === "skills" && view.pulled.includes("sql-review") && view.pullChoices["sql-review"] === "keep" ? ["sql-review-2"] : [];
  return [...view.added.filter((id) => ITEMS[id].kind === view.kind), ...base, ...kept];
}

/** One readout on line two: the cloud glyph, the address, the sentence in its
 *  tooltip. Mallard Cloud only: elsewhere the row is the repository. */
interface Readout {
  address: string;
  tooltip: string;
  created?: boolean;
}
function pushDestination(id: ItemId, view: View): Readout | undefined {
  if (view.storage !== "cloud") return undefined;
  const of = statusOf(id, view)?.link;
  if (of) return { address: `${of.owner_login}/${of.name}`, tooltip: `Pushes to ${of.owner_login}/${of.name}` };
  const address = predictRepositoryAddress(LOGIN, ITEMS[id].name, []).address;
  return { address, tooltip: `This Push creates ${address}, private to you and your collaborators.`, created: true };
}
function pullSource(id: ItemId, view: View): Readout | undefined {
  if (view.storage !== "cloud") return undefined;
  const status = statusOf(id, view);
  if (!status?.link) return undefined;
  const of = status.link;
  const address = `${of.owner_login}/${of.name}`;
  /* What a Pull reads: the draft for a collaborator, the public version for
     a repository this account only reads. */
  const version = of.role === null ? `public v${status.publication?.public_revision}` : `draft v${status.publication?.private_revision}`;
  return { address, tooltip: `Pulls ${version} from ${address}` };
}

/* The Pull plan's local actions, in the protocol's own sentences. */
interface PullOption {
  id: PullChoice;
  label: string;
  destructive: boolean;
}
function pullOptions(id: ItemId, view: View): PullOption[] {
  switch (itemState(id, view)) {
    case "local_missing":
      return [{ id: "install", label: "Install stored content", destructive: false }];
    case "storage_changed":
      return [{ id: "update", label: "Update from storage", destructive: false }];
    case "diverged":
    case "unreviewed_remote":
      return [
        { id: "keep", label: `Keep local as ${copyName(id)} and overwrite`, destructive: false },
        { id: "overwrite", label: "Overwrite local content", destructive: true },
      ];
    default:
      return [];
  }
}

const pushIds = (view: View) => listedItems(view).filter((id) => {
  const row = rowOf(id, view);
  return row.local_present && (row.state === "new" || row.state === "local_changed");
});
const pullIds = (view: View) => listedItems(view).filter((id) => pullOptions(id, view).length > 0);

/* The shipped review's chip grammar (StorageCatalogPage's actionVerb and
   actionChipLabel), restated: the row gets the word, the menu the sentence. */
function actionVerb(label: string): string {
  if (label.startsWith("Keep local as")) return "Keep copy";
  if (label.startsWith("Overwrite")) return "Overwrite";
  if (label.startsWith("Install")) return "Install";
  return "Update";
}
function actionChipLabel(label: string, noun: string): string {
  if (label.startsWith("Keep local as")) return `Update ${noun} · keeps local copy`;
  if (label.startsWith("Overwrite")) return `Overwrite ${noun}`;
  if (label.startsWith("Install")) return `Install ${noun}`;
  return `Update ${noun}`;
}

/* ——— outcomes ——————————————————————————————————————————————————————— */

/* On Mallard Cloud the card is the project's, as syncOutcome.ts builds it:
   the count names what moved ("2 pets v2", "restored"), one repository's
   version leads, every row carries its identifier and its time on line two
   (a session's thread id; here the item's repository address), a kept copy
   reads "kept independently", and a success folds itself. Any other storage
   keeps today's Catalog card. */
const pushedAddress = (id: ItemId, view: View) => (view.lastPushCreated.includes(id)
  ? predictRepositoryAddress(LOGIN, ITEMS[id].name, []).address
  : pushDestination(id, view)?.address);

function pushOutcomeView(view: View, single: boolean): SyncOutcomeView {
  const ids = view.lastPush;
  if (view.storage !== "cloud") {
    return {
      tone: "success", title: "Push complete",
      stats: [{ icon: "upload", value: String(ids.length), label: "completed" }, { icon: "git-branch", label: "Version", value: "8", lead: true }],
      rows: ids.map((id) => ({ key: id, title: ITEMS[id].name, status: "synced" })), actions: [], defaultOpen: false, technical: [],
    };
  }
  const kind = KINDS[view.kind];
  const stats: SyncOutcomeStat[] = [{ icon: "upload", value: String(ids.length), label: ids.length === 1 ? kind.noun : kind.plural }];
  if (single) {
    const before = ids[0] === "duck" ? 0 : statusOf(ids[0], view)?.publication?.private_revision ?? 0;
    stats.push({ icon: "git-branch", label: "Version", value: String(before + 1), lead: true });
  }
  return {
    tone: "success", title: "Push complete", stats,
    rows: ids.map((id) => ({ key: id, title: ITEMS[id].name, status: "synced", threadId: pushedAddress(id, view), updatedAt: now })),
    actions: [], defaultOpen: false, technical: [],
  };
}

function pullOutcomeView(view: View, single: boolean): SyncOutcomeView {
  const ids = Object.keys(view.pullChoices) as ItemId[];
  if (view.storage !== "cloud") {
    return {
      tone: "success", title: "Pull complete",
      stats: [{ icon: "download", value: String(ids.length), label: "applied" }],
      rows: ids.map((id) => ({
        key: id, title: ITEMS[id].name, status: view.pullChoices[id] === "keep" ? "preserved" : "synced",
        reason: view.pullChoices[id] === "keep" ? `Kept local as ${copyName(id)}` : undefined,
      })),
      actions: [], defaultOpen: false, technical: [],
    };
  }
  const kind = KINDS[view.kind];
  const restored = ids.filter((id) => view.pullChoices[id] !== "keep");
  const kept = ids.filter((id) => view.pullChoices[id] === "keep");
  const stats: SyncOutcomeStat[] = [
    { icon: "download", value: String(restored.length), label: "restored" },
    ...(kept.length > 0 ? [{ icon: "git-fork" as const, value: String(kept.length), label: "kept independently" }] : []),
  ];
  if (single) {
    const status = statusOf(ids[0], view);
    const version = status?.link?.role === null ? status.publication?.public_revision : status?.publication?.private_revision;
    stats.push({ icon: "git-branch", label: "Version", value: String(version ?? 1), lead: true });
  }
  const rows: SyncOutcomeRow[] = [
    /* As buildPullOutcome's rows: the identifier on line two, no time. */
    ...restored.map((id) => ({ key: id, title: ITEMS[id].name, status: "synced" as const, threadId: pullSource(id, view)?.address })),
    ...kept.map((id) => ({
      key: `${id}:kept`, title: copyName(id), status: "preserved" as const, threadId: copyName(id),
      reason: `The stored ${ITEMS[id].name} was restored, and the local copy was kept as an independent ${kind.word}.`,
    })),
  ];
  return { tone: "success", title: "Pull complete", stats, rows, actions: [], defaultOpen: false, technical: [] };
}

/** The project page's Storage card as its outcome and its open review use it:
 *  the card frame, then the one storage block. A linked storage draws no
 *  heading; its name is the bar's storage crumb. */
function StorageCard({ review, children }: { review?: boolean; children: ReactNode }) {
  return (
    <section className="project-storage-card project-storage-card-non-git" aria-label={`${STORAGE_NAME} storage`}>
      <div className="project-profile-storage-list">
        <div className={`storage-link-block selected${review ? " sync-review-open" : ""}`}>{children}</div>
      </div>
    </section>
  );
}

/** Where a finished Push or Pull lands: on Mallard Cloud, the Storage card,
 *  as the project's does; elsewhere today's bare strip. */
function OutcomeStrip({ view, outcome, onDismiss }: { view: View; outcome: SyncOutcomeView | null; onDismiss: () => void }) {
  if (!outcome) return null;
  const notice = <SyncOutcomeNotice outcome={outcome} onDismiss={onDismiss} />;
  return view.storage === "cloud" ? <StorageCard>{notice}</StorageCard> : notice;
}

/* ——— the shell ————————————————————————————————————————————————————— */

const capability = (label: string, singular: string, kind: string, format: string, version: number, icon: string): CatalogCapabilityDescriptor => ({
  provider: "codex", resource_kind: kind, format, format_version: version, provider_label: "Codex", label, singular_label: singular,
  presentation: { icon, kind_label: label, format_label: format }, capture: "directory_tree", apply: "automatic", targets: [],
});
const PROFILE = { profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Codex home", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0 };
const scopeView = (id: string, cap: CatalogCapabilityDescriptor): CatalogScopeView => ({
  scope: { scope_id: id, profile_id: PROFILE.profile_id, provider: cap.provider, resource_kind: cap.resource_kind, format: cap.format, format_version: cap.format_version, revision: 1, created_at: 0, updated_at: 0 },
  profile: PROFILE, label: cap.label, available: true, provider_label: "Codex", kind_label: cap.label, format_label: cap.format, capability: cap,
});
const SCOPES = [
  scopeView("scope-pets-v2", capability("Pets v2", "Pet v2", "desktop_pet", "openai.codex-pet", 2, "pet-v2")),
  scopeView("scope-pets-v1", capability("Pets v1", "Pet v1", "desktop_pet", "openai.codex-pet", 1, "pet")),
  scopeView("scope-skills", capability("Skills", "Skill", "agent_skill", "agentskills.io.skill", 1, "skill")),
];
const scopeFor = (kind: KindKey) => SCOPES.find((scope) => scope.scope.scope_id === KINDS[kind].scopeId)!;
const noop = () => {};

function Shell({ kind, adding, onOpenScope, onAddScope, children }: {
  kind: KindKey;
  adding: boolean;
  onOpenScope: (scopeId: string) => void;
  onAddScope: () => void;
  children: ReactNode;
}) {
  return (
    <div className="sync-app" style={{ "--sync-sidebar-width": "248px" } as CSSProperties}>
      <ProjectSidebar
        projects={[]} drafts={[]} activeDraftId={null} storages={[]} storageDrafts={[]} storageProviders={[]} storageUsage={{}}
        activeProjectId={null} activeStorageId={null} catalogScopes={SCOPES} activeCatalogScopeId={adding ? null : KINDS[kind].scopeId}
        addingProject={false} addingCatalogScope={adding} addingStorage={false} catalogIndexOpen={false} storageIndexOpen={false} settingsOpen={false}
        loading={false} busy={false} activityOpen={false} unreadLogs={0}
        onOpenProjects={noop} onSelectProject={noop} onRemoveProject={noop} onSelectDraft={noop} onDiscardDraft={noop}
        onToggleActivity={noop} onAddProject={noop} onOpenStorage={noop} onOpenStorageDraft={noop} onDiscardStorageDraft={noop}
        onRemoveStorage={noop} onAddStorage={noop} onOpenCatalogIndex={noop} onOpenCatalogScope={onOpenScope} onAddCatalogScope={onAddScope}
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
    <PathBarBelow.Provider value={<NoticeZone items={items} subject="messages from Mallard" className="sync-backend-banner" timing="progress" />}>
      {children}
    </PathBarBelow.Provider>
  );
}

/* ——— today: the shipped page, on a stand-in backend ————————————————————— */

const TODAY_STORAGES: CatalogStorageOption[] = [
  { id: "storage-cloud", provider: "managed_cloud", name: STORAGE_NAME, provider_label: "Mallard Cloud", icon: "cloud", detail: `${LOGIN} · ${HOST}`, detailIsPath: false, account_login: LOGIN },
  { id: "storage-vault", provider: "local", name: VAULT_NAME, provider_label: "Local folder", icon: "folder", detail: "~/Mallard Vault", detailIsPath: true },
];
const COORDINATES = {
  pets: { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2, provider_label: "Codex", kind_label: "Pets", format_label: "Codex pet v2" },
  skills: { provider: "codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, provider_label: "Codex", kind_label: "Skills", format_label: "Agent Skill" },
};
const kindOfScope = (scopeId: string): KindKey => (scopeId === KINDS.skills.scopeId ? "skills" : "pets");
/** The live page's stand-in: each item's repository status. */
const TODAY_STATUS: Partial<Record<ItemId, ManagedCloudStatus>> = {
  duck: STATUS.duckPrivate, pip: STATUS.pip, mossy: STATUS.mossy, fable: STATUS.fable, biscuit: STATUS.biscuit, mochi: STATUS.mochi,
  "deploy-checklist": STATUS.deploy, "pdf-tools": STATUS.pdf, "sql-review": STATUS.sql, "release-notes": STATUS.release, "commit-style": STATUS.commit,
  "lint-rules": STATUS.lintRules, "changelog-bot": STATUS.changelog,
};
/** A status as the chooser's row: what the service's summary carries. */
const repositoryOf = (status: ManagedCloudStatus, id: ItemId, role?: string | null): CatalogCloudRepository => {
  const of = status.link!;
  const kind = ITEMS[id].kind;
  return {
    ...COORDINATES[kind], repository_id: of.repository_id, owner_login: of.owner_login, name: of.name,
    address: `${of.owner_login}/${of.name}`, website_url: of.website_url, visibility: of.visibility, description: null, star_count: 0, viewer_starred: false,
    role: role === undefined ? of.role : role, can_push: of.can_push,
    revision: status.publication?.private_revision ?? status.publication?.public_revision ?? 0,
    updated_at: ITEMS[id].edited - 600, registration_only: of.release.kind === "not_pushed",
    resource_id: id, display_name: ITEMS[id].name, available: true,
  };
};
const repositoryStatus = (id: ItemId, repositoryId: string): ManagedCloudStatus | null => {
  const known = [STATUS.duckMaya, STATUS.duckKai, STATUS.pdfMaya, STATUS.mochi, STATUS.changelog, STATUS.biscuit, STATUS.lintRules]
    .find((status) => status.link?.repository_id === repositoryId);
  if (known) return known;
  if (id === "release-notes") {
    return repositoryId.startsWith("repo_maya")
      ? ready(link("maya", "release-notes", { role: "member", can_request_release: false }), { state: "not_public", private_revision: 5, public_revision: null })
      : ready(link(LOGIN, "release-notes"), { state: "not_public", private_revision: 2, public_revision: null });
  }
  if (today.created.has(id)) return { ...STATUS.notPushed, repository_id: repositoryId, link: { ...STATUS.notPushed.link!, repository_id: repositoryId, name: ITEMS[id].name } };
  return TODAY_STATUS[id] ?? null;
};

/** What the stand-in backend remembers between calls, reset on entering the scene. */
const today = {
  revision: 3,
  connected: { pets: {}, skills: {} } as Record<KindKey, Record<string, string>>,
  created: new Set<string>(),
  pushed: new Set<string>(),
  pulled: new Set<string>(),
  pushStorage: "storage-cloud",
  kind: "pets" as KindKey,
};
function resetToday() {
  today.revision = 3;
  today.connected = {
    pets: { pip: "repo_rika321_pip", mossy: "repo_rika321_mossy" },
    skills: { "pdf-tools": "repo_rika321_pdf-tools", "sql-review": "repo_rika321_sql-review", "commit-style": "repo_rika321_commit-style" },
  };
  today.created = new Set();
  today.pushed = new Set();
  today.pulled = new Set();
}
resetToday();

/** The items a row lists: its own, and those Add brought in. */
function todayIds(kind: KindKey): ItemId[] {
  const added = (Object.keys(today.connected[kind]) as ItemId[]).filter((id) => ITEMS[id] && !(kind === "pets" ? PET_LIST : SKILL_LIST).includes(id));
  return [...(kind === "pets" ? PET_LIST : SKILL_LIST), ...added];
}

function todayRows(kind: KindKey, cloud: boolean): CatalogScopeRow[] {
  const base: View = { ...START, kind, storage: "vault" };
  return todayIds(kind).map((id) => {
    const row = rowOf(id, base);
    if (today.pushed.has(id)) return { ...row, state: "in_sync", remote_present: true, remote_updated_at: now };
    if (today.pulled.has(id)) {
      return { ...row, state: "in_sync", local_present: true, local_path: folderOf(id), local_modified_at: now, local_files: ITEMS[id].files, local_bytes: ITEMS[id].bytes };
    }
    if (cloud && id === "release-notes" && !today.connected[kind][id]) {
      return { ...row, state: "blocked", blocker: "In 2 repositories you can see. Choose one on its page." };
    }
    if (!ITEMS[id].local && !(kind === "pets" ? PET_LIST : SKILL_LIST).includes(id)) {
      return { ...row, state: "local_missing", local_present: false, local_path: null, local_files: null, local_bytes: null, remote_present: true };
    }
    return row;
  });
}

const actionFor = (id: ItemId, choice: PullChoice): CatalogPullAction => ({
  action_id: `${choice}-${id}`,
  resource_id: id,
  kind: choice === "install" ? { kind: "install" }
    : choice === "update" ? { kind: "update" }
      : choice === "overwrite" ? { kind: "overwrite_with_storage" }
        : {
          kind: "preserve_as_copy_and_overwrite",
          copy: {
            copy_name: copyName(id), copy_resource_id: `${id}-2`, copy_target_path: `${folderOf(id)}-2`, transformed_content_sha256: "3".repeat(64),
            original_content_sha256: "4".repeat(64), storage_content_sha256: "5".repeat(64), reusable_witness: false,
          },
        },
  requires_destructive_approval: choice === "overwrite",
  expected_target_state: choice === "install" ? { state: "absent" } : { state: "sha256", sha256: ITEMS[id].sha.padEnd(64, "0") },
  incoming_content_sha256: "2".repeat(64),
});
const OUTCOME_OF: Record<PullChoice, CatalogActionOutcome> = { install: "installed", update: "updated", keep: "preserved_and_updated", overwrite: "updated" };

/* The page calls nothing else on these paths; the plugins (dialog, opener)
   answer null through the IPC mock, so a confirm reads as Cancel. */
const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
const learnedFor = (kind: KindKey): Record<string, string> => (kind === "pets"
  ? { pip: "repo_rika321_pip", mossy: "repo_rika321_mossy", fable: "repo_rika321_fable" }
  : { "deploy-checklist": "repo_rika321_deploy-checklist", "pdf-tools": "repo_rika321_pdf-tools", "sql-review": "repo_rika321_sql-review", "commit-style": "repo_rika321_commit-style" });
storageCatalogApi.listScopeResources = async (scopeId, storageId) => {
  await wait(250);
  const kind = kindOfScope(scopeId);
  today.kind = kind;
  const storage = storageId ?? "storage-cloud";
  const cloud = storage === "storage-cloud";
  const known = { ...learnedFor(kind), ...today.connected[kind] };
  return {
    ...COORDINATES[kind],
    scope: scopeFor(kind).scope, profile: PROFILE,
    storage_id: storage, storage_name: cloud ? STORAGE_NAME : VAULT_NAME,
    link: { link_id: `link-${kind}-${storage}`, revision: today.revision },
    storage_revision: cloud ? null : 7,
    rows: todayRows(kind, cloud), warnings: [], recovery: [],
    cloud_repositories: cloud ? known : {},
    cloud_connections: cloud ? { ...today.connected[kind] } : {},
    cloud_status: null,
  };
};
storageCatalogApi.getPreview = async (_scopeId, _storageId, resourceId) => ITEMS[resourceId as ItemId]?.art ?? ART.duck;
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};
storageCatalogApi.getCloudStatus = async (_scopeId, _storageId, resourceId, repositoryId) => {
  await wait(350);
  return repositoryStatus(resourceId as ItemId, repositoryId) ?? STATUS.gone;
};
/* The repositories that hold an item: release-notes is in two. */
storageCatalogApi.listCloudRepositories = async (_scopeId, _storageId, resourceId) => {
  await wait(300);
  if (resourceId === "release-notes") {
    return [
      repositoryOf(ready(link(LOGIN, "release-notes"), { state: "not_public", private_revision: 2, public_revision: null }), "release-notes"),
      repositoryOf(ready(link("maya", "release-notes", { role: "member", can_request_release: false }), { state: "not_public", private_revision: 5, public_revision: null }), "release-notes"),
    ];
  }
  return resourceId === "duck" ? [repositoryOf(STATUS.duckKai, "duck")] : [];
};
storageCatalogApi.listCloudShared = async () => {
  await wait(300);
  return [repositoryOf(STATUS.biscuit, "biscuit"), repositoryOf(STATUS.lintRules, "lint-rules")]
    .filter((row) => !today.connected.pets[row.resource_id] && !today.connected.skills[row.resource_id]);
};
storageCatalogApi.resolveCloudRepository = async (_storageId, address) => {
  await wait(400);
  const text = address.trim().replace(/^https?:\/\//i, "");
  if (/^(www\.)?github\.com\//i.test(text)) return { kind: "github" };
  if (/^staging\.mallard-ai\.com\//i.test(text)) return { kind: "wrong_service", host: "staging.mallard-ai.com", expected: HOST };
  const path = text.replace(/^mallard-ai\.com\//i, "").toLowerCase();
  if (!/^[a-z0-9._-]+\/[a-z0-9._-]+$/.test(path)) return { kind: "invalid" };
  if (path === `${LOGIN.toLowerCase()}/portal`) return { kind: "not_catalog", address: `${LOGIN}/portal` };
  const found: Array<[ManagedCloudStatus, ItemId]> = [
    [STATUS.duckMaya, "duck"], [STATUS.duckKai, "duck"], [STATUS.mochi, "mochi"], [STATUS.pdfMaya, "pdf-tools"], [STATUS.changelog, "changelog-bot"],
    [STATUS.fable, "fable"], [STATUS.mossy, "mossy"], [STATUS.pip, "pip"], [STATUS.deploy, "deploy-checklist"], [STATUS.pdf, "pdf-tools"],
    [STATUS.sql, "sql-review"], [STATUS.commit, "commit-style"], [STATUS.biscuit, "biscuit"], [STATUS.lintRules, "lint-rules"],
  ];
  const hit = found.find(([status]) => `${status.link!.owner_login}/${status.link!.name}`.toLowerCase() === path);
  return hit ? { kind: "found", repository: repositoryOf(hit[0], hit[1]) } : { kind: "not_found", address: path };
};
/* An invite code joins maya/Duck on a pets row, maya's changelog-bot on a
   skills row. */
storageCatalogApi.joinCloudRepository = async () => {
  await wait(500);
  return today.kind === "pets"
    ? { kind: "found", repository: repositoryOf(STATUS.duckMaya, "duck", "member") }
    : { kind: "found", repository: repositoryOf(STATUS.changelog, "changelog-bot", "member") };
};
storageCatalogApi.linkScopeStorage = async (scopeId, storageId) => ({ link_id: `link-${kindOfScope(scopeId)}-${storageId}`, revision: today.revision, completion: "open" });
storageCatalogApi.connectCloudRepository = async (scopeId, _storageId, resourceId, repositoryId) => {
  await wait(300);
  today.connected[kindOfScope(scopeId)][resourceId] = repositoryId;
  today.revision += 1;
};
storageCatalogApi.createCloudRepository = async (scopeId, _storageId, resourceId) => {
  await wait(400);
  const id = resourceId as ItemId;
  const repositoryId = `repo_rika321_${resourceId}_new`;
  today.created.add(id);
  today.connected[kindOfScope(scopeId)][resourceId] = repositoryId;
  today.revision += 1;
  return repositoryOf(repositoryStatus(id, repositoryId)!, id);
};
storageCatalogApi.requestCloudRelease = async () => { await wait(300); };
storageCatalogApi.discardCloudDraft = async (_scopeId, _storageId, _resourceId, repositoryId) => {
  await wait(300);
  return { repository_id: repositoryId, removal_id: "removal_1", state: "done", kept_revision: 2 };
};
storageCatalogApi.preparePush = async (scopeId, storageId) => {
  await wait(900);
  today.pushStorage = storageId;
  const kind = kindOfScope(scopeId);
  const cloud = storageId === "storage-cloud";
  const rows = todayRows(kind, cloud).filter((row) => row.local_present && (row.state === "new" || row.state === "local_changed"));
  const known: Record<string, string> = { ...learnedFor(kind), ...today.connected[kind] };
  return {
    plan_id: `push-${Date.now()}`, scope_id: scopeId, link_id: `link-${kind}-${storageId}`, link_revision: today.revision,
    storage_id: storageId, repository_id: `library-${kind}`, storage_identity_sha256: "0".repeat(64),
    destinations: cloud ? Object.fromEntries(rows.map((row) => {
      const id = row.resource_id as ItemId;
      const status = known[id] ? repositoryStatus(id, known[id]) : null;
      return [id, status?.link
        ? { repository_id: known[id], address: `${status.link.owner_login}/${status.link.name}`, create: false }
        : { repository_id: null, address: predictRepositoryAddress(LOGIN, row.display_name, []).address, create: true }];
    })) : undefined,
    rows: rows.map((row) => ({ row: {
      ...COORDINATES[kind], resource_id: row.resource_id!, display_name: row.display_name,
      relation: row.state === "new" ? "local_only" as const : "local_changed" as const,
      selectable: true, selected_by_default: true, held_for_pull: false,
      local_content_sha256: row.content_sha256, storage_content_sha256: row.state === "new" ? null : "1".repeat(64), warnings: [],
    } })),
    warning_digests: [], created_at: now, expires_at: now + 600,
  };
};
storageCatalogApi.applyPush = async (planId, selected, _acknowledged, onProgress) => {
  const cloud = today.pushStorage === "storage-cloud";
  for (const [index, id] of selected.entries()) {
    onProgress({ plan_id: planId, completed: index, total: selected.length, message: cloud ? `Pushing ${ITEMS[id as ItemId].name}…` : "Publishing reviewed items…" });
    await wait(450);
    today.pushed.add(id);
    today.created.delete(id);
    const kind = ITEMS[id as ItemId].kind;
    if (cloud) today.connected[kind][id] = today.connected[kind][id] ?? `repo_rika321_${id}`;
  }
  return {
    plan_id: planId, published: true, stopped: null, message: "", revision: cloud ? null : 8,
    results: selected.map((id) => ({
      resource_id: id, display_name: ITEMS[id as ItemId].name, state: "published" as const, message: null,
      revision: cloud ? (TODAY_STATUS[id as ItemId]?.publication?.private_revision ?? 0) + 1 : null,
    })),
  };
};
storageCatalogApi.preparePull = async (scopeId, storageId) => {
  await wait(900);
  const kind = kindOfScope(scopeId);
  const items = todayRows(kind, storageId === "storage-cloud").flatMap((row) => {
    const id = row.resource_id as ItemId;
    const choices: PullChoice[] = row.state === "local_missing" ? ["install"]
      : row.state === "storage_changed" ? ["update"]
        : row.state === "diverged" ? ["keep", "overwrite"] : [];
    if (choices.length === 0) return [];
    return [{
      item: {
        ...COORDINATES[kind], resource_id: id, display_name: row.display_name,
        relation: row.state === "local_missing" ? "storage_only" as const : row.state === "storage_changed" ? "storage_changed" as const : "changed_both" as const,
        disposition: { kind: "executable" as const },
        actions: choices.map((choice) => actionFor(id, choice)),
        default_action_id: `${choices[0]}-${id}`,
      },
      conflicting_claims: [],
    }];
  });
  const known: Record<string, string> = { ...learnedFor(kind), ...today.connected[kind] };
  return {
    plan_id: `pull-${Date.now()}`, scope_id: scopeId, link_id: `link-${kind}-${storageId}`, link_revision: today.revision,
    storage_id: storageId, repository_id: `library-${kind}`, storage_identity_sha256: "0".repeat(64),
    sources: storageId === "storage-cloud" ? Object.fromEntries(items.flatMap(({ item }) => {
      const status = known[item.resource_id] ? repositoryStatus(item.resource_id as ItemId, known[item.resource_id]) : null;
      if (!status?.link) return [];
      const reads = status.link.role === null ? "public" as const : "draft" as const;
      return [[item.resource_id, {
        repository_id: status.link.repository_id, address: `${status.link.owner_login}/${status.link.name}`, reads,
        revision: (reads === "public" ? status.publication?.public_revision : status.publication?.private_revision) ?? 1,
      }]];
    })) : undefined,
    items, created_at: now, expires_at: now + 600,
  };
};
storageCatalogApi.applyPull = async (planId, choices, _acknowledged, onProgress) => {
  const ids = Object.keys(choices);
  for (const [index, id] of ids.entries()) {
    onProgress({ plan_id: planId, completed: index, total: ids.length, message: `Pulling ${ITEMS[id as ItemId].name}…` });
    await wait(450);
    today.pulled.add(id);
  }
  return {
    plan_id: planId, remote_accepted: true, stopped: null,
    results: ids.map((id) => ({
      resource_id: id, action_id: choices[id], display_name: ITEMS[id as ItemId].name,
      outcome: OUTCOME_OF[choices[id].split("-")[0] as PullChoice] ?? "updated", message: null,
    })),
  };
};

const LIVE_CAPABILITIES = SCOPES.flatMap((scope) => (scope.capability ? [scope.capability] : []));
storageCatalogApi.listProfiles = async () => [PROFILE];
storageCatalogApi.listCapabilities = async () => LIVE_CAPABILITIES;

/** The shipped Catalog row page, live on the stand-in backend. */
function TodayPage({ kind, landing }: { kind: KindKey; landing?: CatalogCloudLanding | null }) {
  const [note, setNote] = useState<string | null>(landing?.note ?? null);
  return (
    <ShellNote note={note} onDismiss={() => setNote(null)}>
      <StorageCatalogPage
        scope={scopeFor(kind)} scopes={SCOPES} storages={TODAY_STORAGES} capabilities={LIVE_CAPABILITIES}
        initialResourceId={landing?.resourceId}
        appActions={[]} initialStorageId="storage-cloud"
        onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
        onProfileRenamed={noop} onScopeGone={noop} notify={(message) => setNote(message)}
      />
    </ShellNote>
  );
}

/* ——— the proposed bar ——————————————————————————————————————————————————— */

const STORAGES = {
  cloud: { name: STORAGE_NAME, detail: `${LOGIN} · ${HOST}`, icon: "storage-mallard", path: false },
  vault: { name: VAULT_NAME, detail: "~/Mallard Vault", icon: "storage-local", path: true },
} as const;

/** The storage crumb, with today's Switch to: the way to see that another
 *  storage keeps today's page. */
function storageCrumb(view: View, go: Go): PathBarCrumb {
  const storage = STORAGES[view.storage];
  const other: StorageKind = view.storage === "cloud" ? "vault" : "cloud";
  return {
    id: "storage", label: storage.name, icon: storage.icon, tone: "root",
    title: `${storage.name} · ${storage.detail}`, ariaLabel: `${storage.name}. Storage menu`, menuLabel: "Storage",
    menuHeader: { label: storage.name, detail: storage.detail, detailIsPath: storage.path, icon: storage.icon },
    menu: [
      { items: [
        { id: "details", label: "Storage details…", icon: "settings", onSelect: noop },
        { id: "unlink", label: `Unlink from ${rowLabel(view.kind)}…`, icon: "link-off", danger: true, onSelect: noop },
      ] },
      { label: "Switch to", items: [{
        id: other, label: STORAGES[other].name, detail: STORAGES[other].detail, detailIsPath: STORAGES[other].path, icon: STORAGES[other].icon,
        onSelect: () => go({ ...START, storage: other, kind: view.kind }),
      }] },
      { items: [{ id: "add-storage", label: "Add storage…", icon: "plus", onSelect: noop }] },
    ],
  };
}

/** The Catalog row crumb: today's menu. On Mallard Cloud its last group also
 *  holds Add from Mallard Cloud…, beside Add Catalog resource…. On an item's
 *  page its label is the way back to the list. */
function rowCrumb(view: View, go: Go, back?: () => void): PathBarCrumb {
  const label = rowLabel(view.kind);
  const icon = KINDS[view.kind].icon;
  return {
    id: "scope", label, icon, title: `${label}\n~/.codex`,
    ariaLabel: back ? `${label}. Back to the list` : `${label}. Catalog row menu`, menuLabel: "Catalog row",
    menuHeader: { label, detail: "~/.codex", detailIsPath: true, icon },
    onActivate: back,
    menu: [
      { items: [
        { id: "rescan", label: "Rescan this provider home", icon: "refresh", onSelect: noop },
        { id: "rename", label: "Rename profile…", icon: "edit", onSelect: noop },
        { id: "remove", label: "Remove this Catalog row…", icon: "trash", danger: true, onSelect: noop },
      ] },
      { items: [
        { id: "add-scope", label: "Add Catalog resource…", icon: "plus", onSelect: () => go({ page: "add-resource", door: "local" }) },
      ] },
    ],
  };
}

function itemCrumb(id: ItemId): PathBarCrumb {
  const it = ITEMS[id];
  return {
    id: "item", label: it.name,
    leading: it.art ? <img className="pet-leaf-mark" src={it.art} alt="" /> : <Icon name={KINDS[it.kind].icon} size={14} />,
  };
}

function SyncButtons({ pull, push, pullDisabled, pushDisabled, active, onPull, onPush }: {
  pull: number; push: number; pullDisabled: boolean; pushDisabled: boolean; active?: "push" | "pull" | null;
  onPull: () => void; onPush: () => void;
}) {
  return (
    <div className="sync-path-action-group" role="group" aria-label="Sync actions">
      <button type="button" className={`sync-path-sync${active === "pull" ? " active" : ""}`} aria-expanded={active === "pull"} disabled={pullDisabled} onClick={onPull}>
        <Icon name="download" size={14} />
        <span className="sync-path-sync-label">Pull</span>
        {pull > 0 && !pullDisabled && <span className="sync-path-count" aria-hidden="true">{pull}</span>}
      </button>
      <button type="button" className={`sync-path-sync${active === "push" ? " active" : ""}`} aria-expanded={active === "push"} disabled={pushDisabled} onClick={onPush}>
        <Icon name="upload" size={14} />
        <span className="sync-path-sync-label">Push</span>
        {push > 0 && !pushDisabled && <span className="sync-path-count" aria-hidden="true">{push}</span>}
      </button>
    </div>
  );
}

const Refresh = () => (
  <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
    <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
  </button>
);

/* ——— the Catalog card, as shipped ——————————————————————————————————————— */

const TONE: Record<string, string> = {
  synced: "synced", "local-outstanding": "local", "cloud-outstanding": "storage", divergent: "diverged", unavailable: "unavailable", untracked: "untracked",
};

function Badge({ row, storage }: { row: CatalogScopeRow; storage: string }) {
  const status = rowStatus(row);
  const label = rowStatusLabel(row, storage);
  return (
    <span className={`sync-thread-sync-indicator ${TONE[status] ?? "unknown"}`} data-tooltip={label} aria-label={label} tabIndex={0}>
      <ThreadStatusIcon status={status} size={18} />
    </span>
  );
}

function Metrics({ row, singular, readout }: { row: CatalogScopeRow; singular: string; readout?: Readout }) {
  const stamp = rowStamp(row);
  return (
    <div className="sync-history-session-details">
      <div className="sync-history-session-summary">
        <div className="sync-history-thread-meta">
          <span className="sync-history-thread-metric" data-tooltip={row.local_files == null ? "Not on this Mac, so nothing was counted" : `Files · ${row.local_files}`}>
            <Icon name="file" size={12} /><b>{row.local_files == null ? "—" : row.local_files}</b>
          </span>
          <span className="sync-history-thread-metric" data-tooltip={row.local_bytes == null ? "Not on this Mac, so nothing was measured" : `On disk · ${formatBytes(row.local_bytes)}`}>
            <Icon name="drive" size={12} /><b>{row.local_bytes == null ? "—" : formatBytes(row.local_bytes)}</b>
          </span>
          {readout ? (
            <span className="sync-history-thread-metric" data-tooltip={readout.tooltip}>
              <Icon name="cloud" size={12} /><b>{readout.address}</b>
            </span>
          ) : <span className="sync-history-thread-metric" aria-hidden="true" />}
        </div>
        <span className="sync-history-actions-divider" aria-hidden="true" />
        <div className="sync-history-row-clickables">
          <button type="button" className="btn btn-ghost sync-history-launch-action" title={`${singular} info`} aria-label={`${singular} info`}><Icon name="info" size={13} /></button>
        </div>
        <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={stamp?.detail}>
          <time>{stamp ? formatFullDateTime(stamp.seconds) : "—"}</time>
        </span>
      </div>
    </div>
  );
}

function Launches({ row, noun }: { row: CatalogScopeRow; noun: string }) {
  return (
    <div className="sync-history-thread-actions">
      <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Codex" aria-label="Open in Codex">
        <ProviderMark provider="codex" displayName="Codex" />
      </button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Terminal" aria-label="Open in Terminal"><Icon name="terminal" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!row.local_present} title={`Fork ${row.display_name} — duplicate this ${noun} under a new name`} aria-label={`Fork ${row.display_name}`}><Icon name="git-fork" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!row.local_path} title={row.local_path ? `Reveal ${row.local_path} in Finder` : "Not on this Mac"} aria-label={`Reveal ${row.display_name} in Finder`}><Icon name="folder" size={14} /></button>
    </div>
  );
}

const MENU_GAP = 6;
const MENU_EDGE = 8;

/** Where a row's action menu goes once it leaves the card for the page layer.
 *  The Storage card clips to its radius (overflow: hidden), so a menu drawn in
 *  place is cut by the card's edge and lies on the footer; the project's own
 *  in-card popovers go to the page layer for the same reason (TopLayer with
 *  useDebugPopoverPlacement). The side follows that hook's rule — below unless
 *  below cannot hold it and above holds more — with the review's footer as the
 *  floor, so the menu never sits on the footer's verb; it keeps its own width
 *  and starts at the chip's left edge, so it never covers the row's name.
 *  Scrolling or resizing dismisses it, as it does a row popover. */
function useChipMenuPlacement(anchorRef: RefObject<HTMLElement | null>, panelRef: RefObject<HTMLElement | null>, open: boolean, onDismiss: () => void) {
  const [placement, setPlacement] = useState<CSSProperties | null>(null);
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;
  useLayoutEffect(() => {
    if (!open) {
      setPlacement(null);
      return;
    }
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    const chip = anchor.getBoundingClientRect();
    const size = panel.getBoundingClientRect();
    const footer = anchor.closest(".sync-inline-action-review")?.querySelector(":scope > footer")?.getBoundingClientRect();
    const floor = Math.min(window.innerHeight, footer?.top ?? window.innerHeight);
    const roomAboveFooter = floor - chip.bottom - MENU_GAP - MENU_EDGE;
    const roomBelow = window.innerHeight - chip.bottom - MENU_GAP - MENU_EDGE;
    const roomAbove = chip.top - MENU_GAP - MENU_EDGE;
    const height = Math.ceil(size.height);
    const up = height > roomAboveFooter && roomAbove > roomAboveFooter;
    const maxHeight = Math.max(0, Math.min(height, up ? roomAbove : roomBelow));
    setPlacement({
      position: "fixed",
      zIndex: 100,
      right: "auto",
      top: up ? chip.top - MENU_GAP - maxHeight : chip.bottom + MENU_GAP,
      left: Math.max(MENU_EDGE, Math.min(chip.left, window.innerWidth - size.width - MENU_EDGE)),
      maxHeight,
    });
  }, [anchorRef, panelRef, open]);
  useEffect(() => {
    if (!open) return;
    const close = () => dismiss.current();
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [open]);
  return placement;
}

/** The Pull review's local action on a row, as shipped: one action is a
 *  readout; several are a chip whose menu lists the plan's sentences, with
 *  the shipped keyboard contract. The menu is drawn on the page layer. */
function ActionChip({ name, noun, options, chosen, expanded, onToggle, onChoose }: {
  name: string;
  noun: string;
  options: PullOption[];
  chosen: PullOption;
  expanded: boolean;
  onToggle: () => void;
  onChoose: (id: PullChoice) => void;
}) {
  const shellRef = useRef<HTMLSpanElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const toggle = useRef(onToggle);
  toggle.current = onToggle;
  const placement = useChipMenuPlacement(shellRef, panelRef, expanded, () => toggle.current());
  useEffect(() => {
    if (expanded && placement) focusMenuEdge(panelRef.current, "first");
  }, [expanded, placement]);
  useEffect(() => {
    if (!expanded) return;
    const outside = (event: MouseEvent) => {
      if (event.target instanceof Node && !shellRef.current?.contains(event.target) && !panelRef.current?.contains(event.target)) toggle.current();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the review must not also close. */
      event.stopPropagation();
      toggle.current();
      triggerRef.current?.focus();
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [expanded]);
  if (options.length < 2) {
    return <span className="sync-history-branch-state history-only" title={chosen.label}>{actionVerb(chosen.label)}</span>;
  }
  return (
    <span ref={shellRef} className="catalog-row-action-shell">
      <button ref={triggerRef} type="button" className={`catalog-row-action is-choice${chosen.destructive ? " destructive" : ""}`} aria-expanded={expanded} aria-haspopup="menu"
        title={`${chosen.label} — choose a different local action`} onClick={onToggle}>
        {actionChipLabel(chosen.label, noun)}
        <Icon name={expanded ? "chevron-up" : "chevron-down"} size={10} />
      </button>
      {expanded && (
        <TopLayer>
          <div ref={panelRef} className="sync-path-menu" role="menu" aria-label={`Local action for ${name}`}
            style={placement ?? { position: "fixed", top: 0, left: 0, visibility: "hidden" }}
            onKeyDown={(event) => {
              /* Tab leaves from the chip, so focus walks on from the row
                 and not from the end of the page the menu is drawn on. */
              if (event.key === "Tab") {
                onToggle();
                triggerRef.current?.focus();
                return;
              }
              if (moveMenuFocus(event.currentTarget, event.key)) event.preventDefault();
            }}>
            <div className="sync-path-menu-group">
              {options.map((option) => (
                <button key={option.id} type="button" role="menuitemcheckbox" aria-checked={option.id === chosen.id}
                  className={`sync-path-menu-item${option.destructive ? " danger" : ""}`} onClick={() => {
                    onChoose(option.id);
                    triggerRef.current?.focus();
                  }}>
                  <span className="sync-path-menu-copy">
                    <span>{actionVerb(option.label)}</span>
                    <small className="phrase">{option.label}</small>
                  </span>
                  {option.id === chosen.id && <Icon name="check" size={13} className="sync-path-menu-check" />}
                </button>
              ))}
            </div>
          </div>
        </TopLayer>
      )}
    </span>
  );
}

/** The Catalog row card. On Mallard Cloud the name opens the item's page,
 *  the way a session's title opens its page on Activity. */
function ItemCard({ row, kind, storage, onOpen, selectable, selected, onToggle, readout, action, acks }: {
  row: CatalogScopeRow;
  kind: KindKey;
  storage: string;
  onOpen?: () => void;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  readout?: Readout;
  action?: ReactNode;
  acks?: ReactNode;
}) {
  const info = KINDS[kind];
  return (
    <article className={`sync-history-thread-card catalog-card${info.artwork ? " catalog-card-pet" : ""}`}>
      {info.artwork && (
        <span className="catalog-pet-preview" data-tooltip={row.description ?? undefined} tabIndex={row.description ? 0 : undefined}>
          {row.preview_png ? <img src={row.preview_png} alt={`${row.display_name} preview`} /> : <Icon name="catalog-pet-v1" size={16} />}
        </span>
      )}
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${row.display_name}`}>
              <input type="checkbox" checked={!!selected} aria-label={`${selected ? "Exclude" : "Include"} ${row.display_name}`} onChange={() => onToggle?.()} />
            </label>
          )}
          <Badge row={row} storage={storage} />
          {onOpen ? (
            <button type="button" className="sync-history-session-title-button" title={`${row.display_name} · View ${info.word} details`} onClick={onOpen}>
              {row.display_name}
            </button>
          ) : <strong title={row.display_name}>{row.display_name}</strong>}
          {readout?.created && <span className="sync-history-branch-state current" title={readout.tooltip}>New repository</span>}
          {action}
        </div>
        <Launches row={row} noun={info.noun} />
      </div>
      <Metrics row={row} singular={info.singular} readout={readout} />
      {acks}
    </article>
  );
}

/** The list's two plain sections, as shipped: what is only in the storage
 *  first, then the kind's own. */
function ItemSections({ rows, kind, storage, open }: { rows: CatalogScopeRow[]; kind: KindKey; storage: string; open?: (id: ItemId) => () => void }) {
  const cloudRows = rows.filter((row) => !row.local_present && row.remote_present);
  const localRows = rows.filter((row) => row.local_present || !row.remote_present);
  const card = (row: CatalogScopeRow) => <ItemCard key={row.row_id} row={row} kind={kind} storage={storage} onOpen={open?.(row.resource_id as ItemId)} />;
  return (
    <>
      {cloudRows.length > 0 && (
        <>
          {/* On Mallard Cloud the storage's own mark names it, so the words
              shrink to one phrase in one face; the count is the project's
              heading count. Any other storage keeps today's heading. */}
          <div className="sync-history-section-heading">
            {storage === STORAGE_NAME ? (
              <>
                <h2 title={`Only in ${storage}`}><Icon name="storage-mallard" size={14} />Not downloaded</h2>
                <span className="sync-history-heading-count" title={`${cloudRows.length} ${cloudRows.length === 1 ? KINDS[kind].noun : KINDS[kind].plural} only in ${storage}`}
                  aria-label={`${cloudRows.length} ${cloudRows.length === 1 ? KINDS[kind].noun : KINDS[kind].plural} only in ${storage}`}>
                  <Icon name={KINDS[kind].icon} size={12} />{cloudRows.length}
                </span>
              </>
            ) : (
              <h2><Icon name="cloud" size={14} />Only in <span className="sync-history-storage-name">{storage}</span></h2>
            )}
          </div>
          <div className="sync-history-thread-list">{cloudRows.map(card)}</div>
        </>
      )}
      {localRows.length > 0 && (
        <>
          <div className="sync-history-section-heading">
            <h2><Icon name={KINDS[kind].icon} size={14} />{KINDS[kind].label}</h2>
          </div>
          <div className="sync-history-thread-list">{localRows.map(card)}</div>
        </>
      )}
    </>
  );
}

/* ——— the proposed Catalog row page (the list) ———————————————————————————— */

function ListPage({ view, go }: { view: View; go: Go }) {
  const [query, setQuery] = useState("");
  const kind = KINDS[view.kind];
  const ids = listedItems(view);
  const rows = ids.map((id) => rowOf(id, view)).filter((row) => row.display_name.toLowerCase().includes(query.trim().toLowerCase()));
  /* The bar's numbers, read the way the shipped page reads them: Pull counts
     what is newer in the storage, and two-sided or held rows count toward
     attention, though the Pull review still lists the two-sided ones. */
  const counts = ids.map((id) => rowOf(id, view)).reduce((totals, row) => {
    const status = rowStatus(row);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 });
  const nounOf = (count: number) => (count === 1 ? kind.noun : kind.plural);
  const attentionDetail = ([["divergent", "changed on both sides"], ["unavailable", "can't be synced"]] as Array<[string, string]>)
    .map(([status, phrase]) => {
      const count = ids.filter((id) => rowStatus(rowOf(id, view)) === status).length;
      return count === 0 ? null : `${count} ${nounOf(count)} ${phrase}`;
    })
    .filter((phrase): phrase is string => phrase !== null)
    .join(", ");
  /* Only a Mallard Cloud row opens an item's page; any other storage keeps
     today's row page, name and all. */
  const cloud = view.storage === "cloud";
  const open = cloud ? (id: ItemId) => () => go({ page: "item", item: id, outcome: null, note: null }) : undefined;
  const outcome = view.outcome === "push" ? pushOutcomeView(view, false) : view.outcome === "pull" ? pullOutcomeView(view, false) : null;
  return (
    <ShellNote note={view.note} onDismiss={() => go({ note: null })}>
      <SyncPathBar
        crumbs={[storageCrumb(view, go), rowCrumb(view, go)]}
        ariaLabel={`${rowLabel(view.kind)} path`}
        actions={(
          <>
            {counts.attention > 0 && (
              <span className="sync-path-attention warning" tabIndex={0} data-tooltip={attentionDetail} aria-label={`${counts.attention} needing attention. ${attentionDetail}`}>
                <Icon name="alert-triangle" size={13} /><b>{counts.attention}</b>
              </span>
            )}
            {/* Add from Mallard Cloud, where today's Cloud repositories button
                sits: the bar's own round action, as refresh is, with the
                sidebar's + for adding. */}
            {cloud && (
              <button type="button" className="sync-path-icon-action" title="Add from Mallard Cloud…" aria-label="Add from Mallard Cloud" aria-haspopup="dialog"
                onClick={() => go({ dialog: "add" })}>
                <Icon name="plus" size={16} />
              </button>
            )}
            <Refresh />
            <SyncButtons pull={counts.pull} push={counts.push} pullDisabled={pullIds(view).length === 0} pushDisabled={pushIds(view).length === 0}
              onPull={() => go({ page: "pull-batch", outcome: null })} onPush={() => go({ page: "push-batch", outcome: null })} />
          </>
        )}
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        {!cloud && <OutcomeStrip view={view} outcome={outcome} onDismiss={() => go({ outcome: null })} />}
        <section className="profile-links-section" aria-label={rowLabel(view.kind)}>
          {cloud && <OutcomeStrip view={view} outcome={outcome} onDismiss={() => go({ outcome: null })} />}
          <NameSearchField noun={(count: number) => (count === 1 ? kind.noun : kind.plural)} query={query} onChange={setQuery} label={`Search ${kind.plural}`} matches={rows.length} total={ids.length} />
          <ItemSections rows={rows} kind={view.kind} storage={storageName(view)} open={open} />
        </section>
      </main>
    </ShellNote>
  );
}

/* ——— the proposed item page ———————————————————————————————————————————— */

/** The chip's verbs. No Change repository…: a connected item keeps its
 *  repository. */
const CHIP_ACTIONS = {
  onOpenUrl: noop,
  onRequestRelease: noop,
  onDiscardDraft: noop,
  onRetry: noop,
  onStorageSettings: noop,
};

/** An item's repository while it has none: one row, drawn like the item's
 *  card below it — the address the first Push creates, New and Private, and
 *  Change…, which opens the connect dialog. Once the item has a repository
 *  the section steps aside and the repository lives in the bar's Cloud chip,
 *  as a linked project's does. */
function RepositoryCard({ it, onChoose }: { it: Item; onChoose: () => void }) {
  const prediction = predictRepositoryAddress(LOGIN, it.name, []);
  return (
    <>
      <div className="sync-history-section-heading">
        <h2><Icon name="cloud" size={14} />Repository</h2>
      </div>
      <div className="sync-history-thread-list">
        <article className="sync-history-thread-card catalog-card" aria-label={`${it.name} repository`}>
          <div className="sync-history-thread-topline">
            <div className="sync-history-thread-title">
              <span className="sync-thread-sync-indicator untracked" data-tooltip={`Not in ${STORAGE_NAME} yet`} aria-label={`Not in ${STORAGE_NAME} yet`} tabIndex={0}>
                <Icon name="cloud" size={18} />
              </span>
              <strong title={prediction.address}>{prediction.address}</strong>
              <span className="sync-history-branch-state current" title="Your first Push creates it">New</span>
              <span className="sync-history-branch-state current" title="Only you can see it">Private</span>
            </div>
            <div className="sync-history-thread-actions">
              <button type="button" className="btn" aria-haspopup="dialog" aria-label="Change repository" onClick={onChoose}>
                <Icon name="link" size={13} />
                Change…
              </button>
            </div>
          </div>
        </article>
      </div>
    </>
  );
}

/** One item, the way a project is one repository: the project's bar (Cloud
 *  chip, refresh, Pull and Push for this item), its notes under the bar, its
 *  Storage card while there is no repository, then the item's own card from
 *  the list. */
function ItemPage({ view, go }: { view: View; go: Go }) {
  const id = view.item;
  const row = rowOf(id, view);
  const status = statusOf(id, view);
  const blocked = cloudSyncBlocked(status) || row.state === "blocked";
  const pushable = pushIds(view).includes(id);
  const pullable = pullOptions(id, view).length > 0;
  const back = () => go({ page: "list", outcome: null, note: null });
  const outcome = view.outcome === "push" ? pushOutcomeView(view, true) : view.outcome === "pull" ? pullOutcomeView(view, true) : null;

  const notes: NoticeItem[] = [];
  const note = cloudStatusNotice(status);
  if (note) {
    notes.push({
      key: "managed-cloud-status", severity: "warning", title: note.title, message: note.message,
      action: note.action === null ? undefined : (
        <button type="button" className="btn btn-ghost">{note.action === "storage_settings" ? "Storage settings" : "Retry"}</button>
      ),
    });
  }
  if (row.state === "blocked" && id === "pip") {
    notes.push({
      key: "several", severity: "warning", title: `${row.display_name} is in 2 repositories you can see.`,
      message: "Choose the one this Mac uses. Pull and Push wait until then.",
      action: <button type="button" className="btn btn-ghost" onClick={() => go({ dialog: "several" })}>Choose repository</button>,
    });
  }

  return (
    <ShellNote note={view.note} onDismiss={() => go({ note: null })}>
      <SyncPathBar
        crumbs={[storageCrumb(view, go), rowCrumb(view, go, back), itemCrumb(id)]}
        ariaLabel={`${row.display_name} path`}
        actions={(
          <>
            {status && (
              <ManagedCloudChip id={`cloud-${id}`} status={status} storageName={STORAGE_NAME} busy={false} refreshing={false} {...CHIP_ACTIONS} />
            )}
            <Refresh />
            <SyncButtons pull={pullable ? 1 : 0} push={pushable ? 1 : 0} pullDisabled={blocked || !pullable} pushDisabled={blocked || !pushable}
              onPull={() => go({ page: "pull-item", outcome: null, note: null })} onPush={() => go({ page: "push-item", outcome: null, note: null })} />
          </>
        )}
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <NoticeZone items={notes} subject={`notes about this ${KINDS[view.kind].word}`} timing="standing" label={`Notes about this ${KINDS[view.kind].word}`} />
        <section className="profile-links-section" aria-label={row.display_name}>
          <OutcomeStrip view={view} outcome={outcome} onDismiss={() => go({ outcome: null })} />
          {!status && row.state !== "blocked" && <RepositoryCard it={ITEMS[id]} onChoose={() => go({ dialog: "connect" })} />}
          <ItemSections rows={[row]} kind={view.kind} storage={STORAGE_NAME} />
        </section>
      </main>
    </ShellNote>
  );
}

/* ——— Push and Pull reviews ——————————————————————————————————————————————— */

/* On Mallard Cloud a review runs the way the project's does
   (ProjectLinksWorkspace, PushResourceWorkspace, RestorePlanView): it opens
   inside the Storage card, first as the Preparing frame; the bar's button
   reads Preparing…, then the review's verb, then Pushing… or Pulling…, and
   carries no count while the review is open; the footer's verb is Push, or
   Pull N, and a running one says it can't be cancelled. On any other storage
   today's Catalog review stays as it is. */
const PREPARING_MS = 700;
const RUNNING_MS = 900;
type ReviewPhase = "preparing" | "open" | "running";

function useReviewPhase(cloud: boolean): [ReviewPhase, (finish: () => void) => void] {
  const [phase, setPhase] = useState<ReviewPhase>(cloud ? "preparing" : "open");
  useEffect(() => {
    if (phase !== "preparing") return;
    const timer = window.setTimeout(() => setPhase("open"), PREPARING_MS);
    return () => window.clearTimeout(timer);
  }, [phase]);
  const run = (finish: () => void) => {
    if (!cloud) {
      finish();
      return;
    }
    setPhase("running");
    window.setTimeout(finish, RUNNING_MS);
  };
  return [phase, run];
}

function ReviewBar({ view, go, single, ids, active, label, spinning, disabled, count, onRun }: {
  view: View; go: Go; single: boolean; ids: ItemId[]; active: "push" | "pull";
  /** Mallard Cloud: the project bar's face for an open review. */
  label: string;
  spinning: boolean;
  disabled: boolean;
  /** Any other storage: today's bar, the selection's count on the verb. */
  count: number;
  onRun: () => void;
}) {
  const cloud = view.storage === "cloud";
  const chip = single ? statusOf(ids[0], view) : null;
  const button = (kind: "pull" | "push") => {
    const open = kind === active;
    return (
      <button type="button" className={`sync-path-sync${open ? " active" : ""}`} aria-expanded={open} disabled={!open || disabled} onClick={open ? onRun : undefined}>
        <Icon name={open && spinning ? "refresh" : kind === "pull" ? "download" : "upload"} size={14} className={open && spinning ? "icon-spin" : undefined} />
        <span className="sync-path-sync-label">{open ? label : kind === "pull" ? "Pull" : "Push"}</span>
      </button>
    );
  };
  return (
    <SyncPathBar
      crumbs={single
        ? [storageCrumb(view, go), rowCrumb(view, go, () => go({ page: "list" })), itemCrumb(ids[0])]
        : [storageCrumb(view, go), rowCrumb(view, go)]}
      ariaLabel={`${active === "push" ? "Push" : "Pull"} review path`}
      actions={(
        <>
          {chip && <ManagedCloudChip id="cloud-review" status={chip} storageName={STORAGE_NAME} busy refreshing={false} {...CHIP_ACTIONS} />}
          <Refresh />
          {cloud ? (
            <div className="sync-path-action-group" role="group" aria-label="Sync actions">{button("pull")}{button("push")}</div>
          ) : (
            <SyncButtons pull={active === "pull" ? count : 0} push={active === "push" ? count : 0}
              pullDisabled={active !== "pull" || disabled} pushDisabled={active !== "push" || disabled}
              active={active} onPull={onRun} onPush={onRun} />
          )}
        </>
      )}
    />
  );
}

/** The page a review sits on: inside the Storage card on Mallard Cloud, as
 *  the project's does; on its own elsewhere, as today's Catalog review does. */
function ReviewPage({ view, preparing, verb, onCancel, children }: {
  view: View; preparing: boolean; verb: "push" | "pull"; onCancel: () => void; children: ReactNode;
}) {
  if (view.storage !== "cloud") return <main className="sync-main sync-project-links-page sync-sync-review-page">{children}</main>;
  return (
    <main className="sync-main sync-project-links-page sync-sync-review-page">
      <section className="profile-links-section" aria-label={rowLabel(view.kind)}>
        <StorageCard review>
          <div className="sync-storage-inline-review" tabIndex={-1}>
            {preparing ? (
              <ReviewPreparing verb={verb} title={verb === "push" ? "Push review" : "Pull review"}
                caption={verb === "push" ? `Comparing this computer with ${STORAGE_NAME}` : `Reading the latest version from ${STORAGE_NAME}`}
                onCancel={onCancel} cancelDisabled={false} />
            ) : children}
          </div>
        </StorageCard>
      </section>
    </main>
  );
}

function PushReview({ view, go, ids, single }: { view: View; go: Go; ids: ItemId[]; single: boolean }) {
  const [selected, setSelected] = useState(() => new Set(ids));
  const cloud = view.storage === "cloud";
  const [phase, start] = useReviewPhase(cloud);
  const running = phase === "running";
  const back = () => go({ page: single ? "item" : "list" });
  const run = () => start(() => {
    const chosen = ids.filter((id) => selected.has(id));
    go({
      page: single ? "item" : "list", outcome: "push", pushed: [...view.pushed, ...chosen], lastPush: chosen,
      lastPushCreated: chosen.filter((id) => pushDestination(id, view)?.created),
      duck: cloud && chosen.includes("duck") ? "pushed" : view.duck, note: null,
    });
  });
  const label = phase === "preparing" ? "Preparing…" : running ? "Pushing…" : "Push";
  return (
    <>
      <ReviewBar view={view} go={go} single={single} ids={ids} active="push" label={label} spinning={phase !== "open"}
        disabled={phase !== "open" || selected.size === 0} count={selected.size} onRun={run} />
      <ReviewPage view={view} preparing={phase === "preparing"} verb="push" onCancel={back}>
        <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="mock-push-title">
          <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="mock-push-title">Push review</h2></div>
            <SyncReviewSelectionBar
              count={selected.size} total={ids.length} busy={running}
              recommendedCount={ids.length} recommendedTitle={`Use the recommended selection (${ids.length})`}
              recommendedDisabled={selected.size === ids.length} onRecommended={() => setSelected(new Set(ids))}
              clearDisabled={selected.size === 0} onClear={() => setSelected(new Set())}
              close={{ ariaLabel: cloud ? "Close push review" : "Close Push review", onClose: back }}
            />
          </header>
          <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              <div className="sync-history-thread-list">
                {ids.map((id) => (
                  <ItemCard key={id} row={rowOf(id, view)} kind={ITEMS[id].kind} storage={storageName(view)} selectable selected={selected.has(id)}
                    readout={pushDestination(id, view)}
                    onToggle={() => setSelected((current) => {
                      const next = new Set(current);
                      if (next.has(id)) next.delete(id);
                      else next.add(id);
                      return next;
                    })} />
                ))}
              </div>
            </div>
          </div>
          <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
            {running && <span role="status"><small>A running push finishes or fails whole — it can’t be cancelled midway.</small></span>}
            <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={running || selected.size === 0} onClick={run}>
              {running ? <Icon name="refresh" size={16} className="icon-spin" /> : <ThreadStatusIcon status="local-outstanding" size={16} />}
              {cloud ? (running ? "Pushing…" : "Push") : `Push ${selected.size}`}
            </button>
          </footer>
        </section>
      </ReviewPage>
    </>
  );
}

/** Today's Pull review rows: each row's local action as a chip, an overwrite
 *  approved on its own card. On Mallard Cloud each row also names the
 *  repository it reads, an item whose repository refuses the account is held
 *  in the banner above the list, as Push holds rows for Pull, and the
 *  project's header, footer and bar carry it. */
function PullReview({ view, go, ids, single }: { view: View; go: Go; ids: ItemId[]; single: boolean }) {
  const cloud = view.storage === "cloud";
  const kind = KINDS[view.kind];
  const optionsOf = (id: ItemId) => pullOptions(id, view);
  const defaults = () => Object.fromEntries(ids.map((id) => [id, optionsOf(id)[0].id])) as Partial<Record<ItemId, PullChoice>>;
  const [selected, setSelected] = useState(() => new Set(ids));
  const [choice, setChoice] = useState(defaults);
  const [acks, setAcks] = useState(() => new Set<ItemId>());
  const [expanded, setExpanded] = useState<ItemId | null>(null);
  const [phase, start] = useReviewPhase(cloud);
  const running = phase === "running";
  const chosenOf = (id: ItemId) => optionsOf(id).find((option) => option.id === choice[id]) ?? optionsOf(id)[0];
  const pending = ids.some((id) => selected.has(id) && chosenOf(id).destructive && !acks.has(id));
  const held = !single && cloud ? listedItems(view).filter((id) => id !== "pip" && itemState(id, view) === "blocked") : [];
  const recommended = ids.every((id) => selected.has(id) && choice[id] === optionsOf(id)[0].id);
  /* One repository has one version to name, as the project's header does;
     a batch reads several, and each row's readout names its own. */
  const source = single ? statusOf(ids[0], view) : null;
  const version = source?.link?.role === null ? source.publication?.public_revision : source?.publication?.private_revision;
  const back = () => go({ page: single ? "item" : "list" });
  const run = () => start(() => {
    const chosen = ids.filter((id) => selected.has(id));
    go({
      page: single ? "item" : "list", outcome: "pull", pulled: [...view.pulled, ...chosen],
      pullChoices: Object.fromEntries(chosen.map((id) => [id, chosenOf(id).id])) as Partial<Record<ItemId, PullChoice>>, note: null,
    });
  });
  const verb = `Pull ${selected.size}`;
  const label = phase === "preparing" ? "Preparing…" : running ? "Pulling…" : verb;
  return (
    <>
      <ReviewBar view={view} go={go} single={single} ids={ids} active="pull" label={label} spinning={phase !== "open"}
        disabled={phase !== "open" || selected.size === 0 || pending} count={selected.size} onRun={run} />
      <ReviewPage view={view} preparing={phase === "preparing"} verb="pull" onCancel={back}>
        <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="mock-pull-title">
          <header className="sync-inline-action-header sync-inline-pull-header sync-sync-review-header">
            <div className="sync-inline-action-heading">
              <div className="sync-sync-review-title">
                <h2 id="mock-pull-title">Pull review</h2>
                {cloud && single && version != null && <span className="sync-pull-review-meta">Version {version}</span>}
              </div>
            </div>
            <SyncReviewSelectionBar
              count={selected.size} total={ids.length} busy={running}
              recommendedCount={ids.length} recommendedTitle={`Use the recommended local actions (${ids.length})`}
              recommendedDisabled={recommended}
              onRecommended={() => { setSelected(new Set(ids)); setChoice(defaults()); setAcks(new Set()); }}
              clearDisabled={selected.size === 0} onClear={() => setSelected(new Set())}
              close={{ ariaLabel: cloud ? `Close pull review for ${rowLabel(view.kind)}` : "Close Pull review", onClose: back }}
            />
          </header>
          <div className="sync-pull-review-content sync-inline-action-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              {held.length > 0 && (
                <div className="sync-review-banners">
                  <Notice severity="warning" timing="standing" label="Notes about this Pull review"
                    title={`${held.length} ${held.length === 1 ? kind.noun : kind.plural} can't be pulled`}>
                    {`Mallard Cloud refused this account for ${held.map((id) => pullSourceAddress(id)).join(", ")}. The other ${kind.plural} still pull.`}
                  </Notice>
                </div>
              )}
              <div className="sync-history-thread-list">
                {ids.map((id) => {
                  const row = rowOf(id, view);
                  const options = optionsOf(id);
                  const chosen = chosenOf(id);
                  const included = selected.has(id);
                  return (
                    <ItemCard key={id} row={row} kind={ITEMS[id].kind} storage={storageName(view)} selectable selected={included}
                      readout={pullSource(id, view)}
                      onToggle={() => setSelected((current) => {
                        const next = new Set(current);
                        if (next.has(id)) next.delete(id);
                        else next.add(id);
                        return next;
                      })}
                      action={included ? (
                        <ActionChip name={row.display_name} noun={kind.noun} options={options} chosen={chosen} expanded={expanded === id}
                          onToggle={() => setExpanded(expanded === id ? null : id)}
                          onChoose={(next) => {
                            setChoice((current) => ({ ...current, [id]: next }));
                            setAcks((current) => {
                              const after = new Set(current);
                              after.delete(id);
                              return after;
                            });
                            setExpanded(null);
                          }} />
                      ) : undefined}
                      acks={included && chosen.destructive ? (
                        <Acknowledgement title="Overwrites this Mac’s copy" checked={acks.has(id)} ariaLabel={`Approve overwriting ${row.display_name}`}
                          verbs={["Approve overwrite", "Approved"]}
                          onChange={(checked) => setAcks((current) => {
                            const after = new Set(current);
                            if (checked) after.add(id);
                            else after.delete(id);
                            return after;
                          })}>
                          {`— replaces the current ${row.display_name} here with the stored version`}
                        </Acknowledgement>
                      ) : undefined}
                    />
                  );
                })}
              </div>
            </div>
          </div>
          <footer className="sync-pull-apply-bar sync-inline-action-footer sync-sync-review-footer">
            {pending && <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>}
            {running && <span role="status"><small>Pull cannot be cancelled while changes are being applied.</small></span>}
            <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={running || selected.size === 0 || pending} onClick={run}>
              {running ? <Icon name="refresh" size={16} className="icon-spin" /> : <ThreadStatusIcon status="cloud-outstanding" size={16} />}
              {running ? "Pulling…" : verb}
            </button>
          </footer>
        </section>
      </ReviewPage>
    </>
  );
}

/** A held item's address, whatever its status could read. */
const pullSourceAddress = (id: ItemId) => (id === "release-notes" ? "maya/release-notes" : ITEMS[id].name);

/* ——— the connect dialog: the project's, with a Catalog item in it ————————————— */

type Coordinate = "pets-v2" | "pets-v1" | "skills" | "project";
interface CloudRow {
  address: string;
  owner: string;
  name: string;
  item: string;
  coordinate: Coordinate;
  visibility: "private" | "public";
  role: "owner" | "admin" | "member" | null;
  version: number;
  updated: number;
  /** Created by Create and connect, nothing pushed yet. */
  registration?: boolean;
}
const cloudRow = (address: string, of: Partial<CloudRow> = {}): CloudRow => {
  const [owner, name] = address.split("/");
  return { address, owner, name, item: name, coordinate: "pets-v2", visibility: "private", role: "owner", version: 3, updated: now - 2 * 3600, ...of };
};
/** What the service answers by address. */
const DIRECTORY: CloudRow[] = [
  cloudRow("maya/Duck", { visibility: "public", role: null, version: 6, updated: now - 26 * 3600 }),
  cloudRow("kai/Duck", { role: "member", version: 4, updated: now - 3 * 3600 }),
  cloudRow("maya/Mochi", { visibility: "public", role: null, version: 5, updated: now - 6 * 86400 }),
  cloudRow(`${LOGIN}/Owl`, { item: "Owl" }),
  cloudRow(`${LOGIN}/portal`, { coordinate: "project" }),
  cloudRow(`${LOGIN}/Duck-classic`, { item: "Duck", coordinate: "pets-v1" }),
  cloudRow("maya/pdf-tools", { coordinate: "skills", visibility: "public", role: null, version: 9, updated: now - 30 * 3600 }),
  cloudRow("maya/changelog-bot", { coordinate: "skills", visibility: "public", role: null, version: 2, updated: now - 3 * 86400 }),
  /* Shared with this account: kai invited it to both. */
  cloudRow("kai/Biscuit", { role: "member", version: 2, updated: now - 2 * 86400 }),
  cloudRow("kai/lint-rules", { coordinate: "skills", role: "member", version: 3, updated: now - 86400 }),
];
/** What an invite code joins on Add, for each kind: someone else's item. */
const ADDED: Record<KindKey, ItemId> = { pets: "mochi", skills: "changelog-bot" };

/** A repository's kind, as a row of this specimen; Pets v1 has none here. */
const kindOf = (coordinate: Coordinate): KindKey | null => (coordinate === "pets-v2" ? "pets" : coordinate === "skills" ? "skills" : null);

/** Who chooses: an item connecting (connect, several), the row page's Add
 *  (this row's kind), or Add Catalog resource's From Mallard Cloud door (any
 *  kind: the repository brings its own). */
type ChooseReason = DialogReason | "door";

/** What a list offers. Connect: the repositories holding the item that this
 *  account belongs to — Duck's is kai/Duck; one it owned would already be
 *  its repository. Add: shared with you — someone else's repository this
 *  account belongs to, whose item no row here has yet. Your own are listed
 *  on the row page already. */
function listingFor(reason: ChooseReason, subject: ItemId, view: View): CloudRow[] {
  if (reason === "several") {
    return [cloudRow(`${LOGIN}/Pip`, { version: 3, updated: now - 3 * 86400 }), cloudRow("maya/Pip", { role: "member", version: 7, updated: now - 5 * 3600 })];
  }
  if (reason === "connect") return subject === "duck" ? DIRECTORY.filter((row) => row.address === "kai/Duck") : [];
  return DIRECTORY.filter((row) => {
    const kind = kindOf(row.coordinate);
    if (!kind || row.owner === LOGIN || row.role === null) return false;
    if (reason === "add" && kind !== view.kind) return false;
    return !listedItems({ ...view, kind }).some((id) => ITEMS[id].name === row.item);
  });
}

type Lookup =
  | { kind: "idle" }
  | { kind: "resolving"; address: string }
  | { kind: "found"; repository: CloudRow; joined: boolean }
  | { kind: "error"; message: ReactNode }
  | { kind: "not_found"; address: string };

/** A row's standing in one or two words: its role, and Public when anyone
 *  signed in can push to it. */
const roleWord = (row: CloudRow) => (row.role === null ? "Public · Contributor" : `${row.visibility === "public" ? "Public · " : ""}${capitalize(row.role)}`);

/** One state of a chooser, frozen, for the states boards. */
interface ConnectPreset {
  listing?: "loading" | "ready" | { error: string };
  /** The rows the listing answered, in place of the reason's own. */
  rows?: CloudRow[];
  /** The account already owns a repository with the item's name. */
  taken?: boolean;
  entry?: string;
  lookup?: Lookup;
  joining?: boolean;
  joinError?: string;
  selection?: "new" | string | null;
  working?: boolean;
  failure?: string;
  /** The door: no Mallard Cloud account set up yet. */
  noAccount?: boolean;
  /** The door: no Catalog row of the repository's kind yet. */
  newRow?: boolean;
}

const LISTING_MS = 600;

/** The repository choice the dialog and the From Mallard Cloud door share:
 *  the address or invite-code field, what it answered, and the list, read
 *  when the chooser opens as the project's picker reads it. */
function useRepositoryChoice(reason: ChooseReason, view: View, preset: ConnectPreset | undefined, working: boolean) {
  const frozen = preset !== undefined;
  const adding = reason === "add" || reason === "door";
  const kind = KINDS[view.kind];
  const mine: Coordinate = view.kind === "pets" ? "pets-v2" : "skills";
  const subject: ItemId = reason === "several" ? "pip" : adding ? ADDED[view.kind] : view.item;
  const subjectName = ITEMS[subject].name;
  const listing = preset?.rows ?? listingFor(reason, subject, view);
  const ownsOne = listing.some((row) => row.role === "owner");
  const offerNew = !adding && !ownsOne;
  const [listState, setListState] = useState<"loading" | "ready" | { error: string }>(preset?.listing ?? "loading");
  useEffect(() => {
    if (frozen || listState !== "loading") return;
    const timer = window.setTimeout(() => setListState("ready"), LISTING_MS);
    return () => window.clearTimeout(timer);
  }, [frozen, listState]);
  const [entry, setEntry] = useState(preset?.entry ?? "");
  const [lookup, setLookup] = useState<Lookup>(preset?.lookup ?? { kind: "idle" });
  const [joining, setJoining] = useState(preset?.joining ?? false);
  const [joinError, setJoinError] = useState<string | null>(preset?.joinError ?? null);
  const [selection, setSelection] = useState<"new" | string | null>(preset?.selection !== undefined ? preset.selection : offerNew ? "new" : null);
  const owned = listing.map((row) => ({ owner_login: row.owner, name: row.name }));
  const prediction = predictRepositoryAddress(LOGIN, subjectName, preset?.taken ? [...owned, { owner_login: LOGIN, name: subjectName }] : owned);

  const answer = (typed: string): Lookup => {
    const text = typed.trim().replace(/^https?:\/\//i, "");
    if (/^(www\.)?github\.com\//i.test(text)) return { kind: "error", message: "That’s a GitHub address." };
    if (/^staging\.mallard-ai\.com\//i.test(text)) return { kind: "error", message: "That’s staging.mallard-ai.com, not this account." };
    const path = text.replace(/^mallard-ai\.com\//i, "");
    if (!/^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(path)) return { kind: "error", message: "Type owner/name, or paste an invite code." };
    const found = DIRECTORY.find((row) => row.address.toLowerCase() === path.toLowerCase());
    if (!found) return { kind: "not_found", address: path };
    if (found.coordinate === "project") return { kind: "error", message: `${found.address} is a project, not a ${reason === "door" ? "Catalog resource" : kind.word}.` };
    const into = kindOf(found.coordinate);
    if (reason === "door") {
      if (!into) return { kind: "error", message: `${found.address} is a Pets v1 repository.` };
    } else {
      if (found.coordinate === "pets-v1" && mine === "pets-v2") return { kind: "error", message: `${found.address} is a Pets v1 repository.` };
      if (found.coordinate !== mine) return { kind: "error", message: `${found.address} is a ${found.coordinate === "skills" ? "skill" : "pet"}, not a ${kind.word}.` };
    }
    if (adding && found.owner === LOGIN) return { kind: "error", message: `${found.address} is yours — already listed.` };
    if (adding) {
      const row = into ?? view.kind;
      if (listedItems({ ...view, kind: row }).some((id) => ITEMS[id].name === found.item)) return { kind: "error", message: `${rowLabel(row)} already has ${found.item}.` };
      return { kind: "found", repository: found, joined: false };
    }
    if (found.item !== subjectName) return { kind: "error", message: `${found.address} holds ${found.item}, not ${subjectName}.` };
    return { kind: "found", repository: found, joined: false };
  };

  const submit = () => {
    const typed = entry.trim();
    if (!typed) return;
    setJoinError(null);
    if (isInvitationCodeShaped(typed)) {
      setJoining(true);
      window.setTimeout(() => {
        const base = DIRECTORY.find((row) => row.coordinate === mine && row.item === subjectName && row.owner !== LOGIN && row.role === null) ?? cloudRow(`maya/${subjectName}`, { coordinate: mine });
        const repository = { ...base, role: "member" as const };
        setJoining(false);
        setEntry(repository.address);
        setLookup({ kind: "found", repository, joined: true });
        setSelection(repository.address);
      }, 500);
      return;
    }
    setLookup({ kind: "resolving", address: typed });
    window.setTimeout(() => {
      const next = answer(typed);
      setLookup(next);
      if (next.kind === "found") setSelection(next.repository.address);
      else if (selection !== "new") setSelection(null);
    }, 450);
  };

  const rows = lookup.kind === "found" && lookup.joined && !adding ? [lookup.repository, ...listing] : listing;
  const inRows = lookup.kind === "found" && listing.some((row) => row.address === lookup.repository.address);
  const chosen = selection && selection !== "new"
    ? rows.find((row) => row.address === selection) ?? (lookup.kind === "found" ? lookup.repository : null)
    : null;
  const ready = listState === "ready";
  const kindLabel = (row: CloudRow) => KINDS[kindOf(row.coordinate) ?? view.kind];

  const field = (
    <RepositoryLookupField value={entry} onChange={(value) => { setEntry(value); setJoinError(null); }} onSubmit={submit}
      disabled={working} looking={lookup.kind === "resolving"} joining={joining} invalid={lookup.kind === "error" || joinError !== null}
      error={joinError ? <FieldMessage blocking>{joinError}</FieldMessage> : undefined} />
  );

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
          <span>Check the spelling, or paste an invite code.</span>
          {offerNew && (
            <span className="storage-found-empty-verbs">
              <button type="button" className="btn btn-primary" onClick={() => { setLookup({ kind: "idle" }); setSelection("new"); }}>Use a new repository</button>
            </span>
          )}
        </div>
      </div>
    );
  } else if (lookup.kind === "found" && (inRows || lookup.joined) && !adding) {
    found = <small className="repo-answer-line" role="status"><Icon name="check-circle" size={13} /> {lookup.joined ? `Joined ${lookup.repository.address}. Selected below.` : "Selected below."}</small>;
  } else if (lookup.kind === "found") {
    const repository = lookup.repository;
    const selected = selection === repository.address;
    const its = kindLabel(repository);
    found = (
      <div className="r2-step-list">
        <div className="storage-found-list" aria-label="Repository at this address">
          <div className={`storage-found-row with-verb${selected ? " selected" : ""}`}>
            <span className="sync-storage-repository-icon"><Icon name={its.icon} size={14} /></span>
            <span className="storage-found-text">
              <span className="storage-found-name">{repository.address}</span>
              <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {roleWord(lookup.joined ? { ...repository, role: "member" } : repository)} · v{repository.version}</span>
            </span>
            <span className="storage-found-verb">
              {selected
                ? <span className="storage-found-connected"><Icon name="check" size={12} /> Selected</span>
                : <button type="button" className="btn btn-primary" onClick={() => setSelection(repository.address)}>Use this</button>}
            </span>
          </div>
        </div>
      </div>
    );
  }

  const list = (
    <>
      {adding && <div className="repo-list-head">Shared with you{ready && rows.length > 0 && <small>{rows.length}</small>}</div>}
      {listState === "loading" ? (
        <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Loading repositories…</small>
      ) : typeof listState === "object" ? (
        <Notice severity="danger" timing="blocking">{listState.error}</Notice>
      ) : !offerNew && rows.length === 0 ? (
        <p className="repo-list-empty">{adding ? "Nothing shared yet." : "No repositories yet."}</p>
      ) : (
        reason === "door" ? (
          /* The door's rows are New project's: square radios and a mark, in
             the setup page's skin; the mark is the repository's kind. */
          <div className="sync-bundle-match-options repo-rows" role="radiogroup" aria-label="Shared with you">
            {rows.map((row) => (
              <button key={row.address} type="button" role="radio" aria-checked={selection === row.address}
                className={`sync-bundle-match-option${selection === row.address ? " active" : ""}`}
                onClick={() => { setSelection(row.address); setEntry(row.address); setLookup({ kind: "idle" }); }} disabled={working}>
                <span className="sync-bundle-radio"><span /></span>
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={kindLabel(row).icon} size={24} /></span>
                <span className="sync-bundle-match-copy">
                  <strong>{row.address}</strong>
                  <span>{kindLabel(row).singular} · {roleWord(row)}</span>
                </span>
                <span className="sync-bundle-match-meta">{formatRelativeTime(row.updated)}</span>
              </button>
            ))}
          </div>
        ) : (
        <div className="sync-bundle-choice-list repo-rows" role="radiogroup" aria-label={adding ? "Shared with you" : `Repositories that hold ${subjectName}`}>
          {offerNew && (
            <button type="button" role="radio" aria-checked={selection === "new"} className={`sync-bundle-choice separate${selection === "new" ? " active" : ""}`}
              onClick={() => { setSelection("new"); setLookup({ kind: "idle" }); }} disabled={working}>
              <span className="sync-bundle-radio"><span /></span>
              <span className="sync-bundle-choice-copy">
                <strong>{prediction.address}<small className="sync-bundle-recommended">New</small></strong>
                {prediction.suffixed && <span>{prediction.base} is taken.</span>}
              </span>
              <span className="sync-bundle-choice-meta"><small>Private</small></span>
            </button>
          )}
          {rows.map((row) => (
            <button key={row.address} type="button" role="radio" aria-checked={selection === row.address}
              className={`sync-bundle-choice${selection === row.address ? " active" : ""}`}
              onClick={() => { setSelection(row.address); setEntry(row.address); setLookup({ kind: "idle" }); }} disabled={working}>
              <span className="sync-bundle-radio"><span /></span>
              <span className="sync-bundle-choice-copy">
                <strong>{row.address}</strong>
                <span>{roleWord(row)}</span>
              </span>
              <span className="sync-bundle-choice-meta">
                {!row.registration && <strong>v{row.version}</strong>}
                <small>{row.registration ? "not pushed yet" : formatRelativeTime(row.updated)}</small>
              </span>
            </button>
          ))}
        </div>
        )
      )}
    </>
  );

  return { adding, kind, subjectName, selection, chosen, ready, field, found, list };
}

/** A connected item keeps its repository, and an added one its own, so the
 *  footer says so before the choice is made, whichever it is. */
function chooserNote(selection: string | null, ready: boolean): string | null {
  return selection && (selection !== "new" || ready) ? "Can’t be changed later." : null;
}

function ConnectDialog({ reason, view, preset, onCancel, onConnected }: {
  reason: DialogReason;
  view: View;
  /** Drawn in place and frozen in one state, for the states boards. */
  preset?: ConnectPreset;
  onCancel: () => void;
  onConnected: (choice: { kind: "new" } | { kind: "existing"; repository: CloudRow }) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const frozen = preset !== undefined;
  useEffect(() => { if (!frozen) dialog.current?.showModal(); }, [frozen]);
  const [working, setWorking] = useState(preset?.working ?? false);
  const { adding, subjectName, selection, chosen, ready, field, found, list } = useRepositoryChoice(reason, view, preset, working);
  const leanNote = chooserNote(selection, ready);
  const failure = preset?.failure ?? null;
  const close = () => { if (!frozen) dialog.current?.close(); };
  const verb = adding ? "Add" : "Connect";
  const title = reason === "add" ? "Add from Mallard Cloud" : reason === "several" ? "Choose a repository" : `Connect ${subjectName}`;
  const lead = reason === "several" ? `${subjectName} is in 2 repositories. Pick one.` : null;
  const note = leanNote;

  const confirm = () => {
    if (!selection) return;
    setWorking(true);
    window.setTimeout(() => {
      setWorking(false);
      onConnected(selection === "new" ? { kind: "new" } : { kind: "existing", repository: chosen ?? cloudRow(selection) });
    }, 500);
  };

  return (
    <dialog ref={dialog} open={frozen || undefined} className={`sync-modal sync-bundle-connection-dialog mock-lean${frozen ? " mock-frozen-dialog" : ""}`} aria-labelledby={titleId}
      onCancel={(event) => { if (working) event.preventDefault(); }} onClose={onCancel}>
      <header className="sync-modal-header">
        <div>
          <h1 id={titleId}>{title}</h1>
          {lead && <p>{lead}</p>}
        </div>
        <button type="button" className="btn btn-ghost" onClick={close} disabled={working} aria-label="Close repo chooser">
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body repo-connect">
        {field}
        {found}
        {list}
        {failure && <Notice severity="danger" timing="blocking">{failure}</Notice>}
      </div>
      <footer className="sync-modal-footer">
        {note ? <span>{note}</span> : null}
        <div>
          <button type="button" className="btn" onClick={close} disabled={working}>Cancel</button>
          <button type="button" className="btn btn-primary" disabled={working || !selection || (selection === "new" && !ready)} onClick={confirm}>
            {working ? (adding ? "Adding…" : "Connecting…") : verb}
          </button>
        </div>
      </footer>
    </dialog>
  );
}

/* ——— Add Catalog resource, with New project's two doors ———————————————— */

const LOCAL_KINDS: Array<{ icon: IconName; label: string; providers: Array<[string, string]> }> = [
  { icon: "catalog-pet-v2", label: "Pets v2", providers: [["codex", "Codex"]] },
  { icon: "catalog-pet-v1", label: "Pets v1", providers: [["codex", "Codex"]] },
  { icon: "lightbulb", label: "Skills", providers: [["codex", "Codex"], ["claude", "Claude"]] },
  { icon: "catalog-plugin", label: "Plugins", providers: [["codex", "Codex"], ["claude", "Claude"]] },
];

/** The sidebar's Catalog + opens Add Catalog resource; it gains New
 *  project's two doors. On this computer is today's page: a kind, then an
 *  agent profile. From Mallard Cloud is New project's door for a Catalog
 *  item: the account, the repository — shared with you, or by address or
 *  invite code, its kind coming with it — then the Catalog row it lands in.
 *  Add opens the item's page with Pull 1; nothing is pulled on its own. */
function AddResourcePage({ view, go, preset, onAdded }: {
  view: View;
  go: Go;
  /** Frozen in one state, for the states board. */
  preset?: ConnectPreset;
  onAdded: (repository: CloudRow) => void;
}) {
  const titleId = useId();
  const frozen = preset !== undefined;
  const cloud = view.door === "cloud";
  const [working, setWorking] = useState(preset?.working ?? false);
  const { chosen, field, found, list } = useRepositoryChoice("door", view, preset, working);
  const into = chosen ? kindOf(chosen.coordinate) : null;
  const noAccount = preset?.noAccount ?? false;
  const failure = preset?.failure ?? null;
  const door = (next: View["door"]) => () => { if (!frozen) go({ door: next }); };
  const cancel = () => { if (!frozen) go({ page: "list", door: "local" }); };
  const add = () => {
    if (!chosen) return;
    setWorking(true);
    window.setTimeout(() => {
      setWorking(false);
      onAdded(chosen);
    }, 500);
  };
  const pill = (icon: IconName, label: string) => (
    <button type="button" role="radio" aria-checked className="setup-pill active">
      <Icon name={icon} size={12} />
      <span className="setup-pill-label">{label}</span>
      <span className="setup-pill-check" aria-hidden="true"><Icon name="check" size={12} /></span>
    </button>
  );

  const local = (
    <>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>Kind</strong></div></div>
        <div className="setup-choice-grid" role="radiogroup" aria-label="Resource kind">
          {LOCAL_KINDS.map((option) => (
            <button key={option.label} type="button" role="radio" aria-checked={false} className="setup-choice-card" onClick={noop}>
              <span className="setup-choice-card-head">
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={option.icon} size={24} /></span>
                <strong>{option.label}</strong>
                <span className="setup-choice-card-check" aria-hidden="true" />
              </span>
              <span className="setup-choice-card-foot">
                {option.providers.map(([key, name]) => <ProviderMark key={key} provider={key} displayName={name} size={12} />)}
                {option.providers.length === 1 ? `${option.providers[0][1]} only` : option.providers.map(([, name]) => name).join(" and ")}
              </span>
            </button>
          ))}
        </div>
      </section>
      <section className="sync-setup-section">
        <div className="sync-card-heading"><div><strong>Agent profile</strong></div></div>
        <small className="sync-setup-hint">Choose a kind first.</small>
      </section>
    </>
  );

  const fromCloud = (
    <>
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
          <div className="setup-pill-row" role="radiogroup" aria-label="Storage">{pill("storage-mallard", `${STORAGE_NAME} · ${LOGIN}`)}</div>
        )}
      </section>
      {!noAccount && (
        <section className="sync-setup-section repo-connect cloud-project-repository mock-lean">
          <div className="sync-card-heading"><div><strong>Repository</strong></div></div>
          {field}
          {found}
          {list}
        </section>
      )}
      {!noAccount && (
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Catalog row</strong></div></div>
          {!into ? (
            <small className="sync-setup-hint">Choose a repository first.</small>
          ) : preset?.newRow ? (
            <>
              <div className="setup-pill-row" role="radiogroup" aria-label="Provider">
                <button type="button" role="radio" aria-checked className="setup-pill active">
                  <ProviderMark provider="codex" displayName="Codex" size={12} />
                  <span className="setup-pill-label">Codex</span>
                  <span className="setup-pill-check" aria-hidden="true"><Icon name="check" size={12} /></span>
                </button>
              </div>
              <small className="sync-setup-hint">Add creates {rowLabel(into)}.</small>
            </>
          ) : (
            <div className="setup-pill-row" role="radiogroup" aria-label="Catalog row">{pill(KINDS[into].icon, rowLabel(into))}</div>
          )}
        </section>
      )}
      {failure && <Notice severity="danger" timing="blocking">{failure}</Notice>}
    </>
  );

  const content = (
    <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-labelledby={titleId}>
      <header className="sync-modal-header">
        <div><h1 id={titleId}>Add Catalog resource</h1></div>
        <button type="button" className="btn btn-ghost" onClick={cancel} disabled={working} aria-label="Close without adding a resource">
          <Icon name="x" size={17} />
        </button>
      </header>
      <div className="sync-modal-body">
        <div className="storage-mode-doors" role="radiogroup" aria-label="Where the resource comes from">
          <ModeDoor icon="computer" title="On this computer" detail="A kind and an agent home." checked={!cloud} disabled={working} onChoose={door("local")} />
          <ModeDoor icon="storage-mallard" title="From Mallard Cloud" detail="A repository shared with you." checked={cloud} disabled={working} onChoose={door("cloud")} />
        </div>
        {cloud ? fromCloud : local}
      </div>
      <footer className="sync-modal-footer sync-setup-footer">
        {cloud && chosen && <span className="sync-setup-save-state">Can’t be changed later.</span>}
        <div>
          <button type="button" className="btn btn-ghost" onClick={cancel} disabled={working}>Cancel</button>
          <button type="button" className="btn btn-primary" disabled={working || !cloud || !chosen || noAccount} onClick={add}>
            {working ? "Adding…" : "Add"}
          </button>
        </div>
      </footer>
    </section>
  );
  if (frozen) return content;
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page">
      <section className="profile-links-section">{content}</section>
    </main>
  );
}

/* ——— the choosers, state by state ——————————————————————————————————————— */

const MAYA_DUCK = DIRECTORY.find((row) => row.address === "maya/Duck")!;
const MAYA_MOCHI = DIRECTORY.find((row) => row.address === "maya/Mochi")!;
const CODE = "ABCD-EFGH-JKLM-NPQR-STUV";
const EXPIRED = "That code expired or was used. Ask for a new one.";

/* Each specimen is the live chooser, frozen. Unless a state says otherwise
   the list has loaded. */
const CONNECT_STATES: Array<{ when: string; preset: ConnectPreset }> = [
  { when: "Loading repositories", preset: { listing: "loading" } },
  { when: "None holds Duck yet", preset: { rows: [] } },
  { when: "Pick one of yours", preset: { selection: "kai/Duck" } },
  { when: "Name taken", preset: { rows: [], taken: true } },
  { when: "Looking up", preset: { entry: "maya/Duck", lookup: { kind: "resolving", address: "maya/Duck" } } },
  { when: "Found by address", preset: { entry: "maya/Duck", lookup: { kind: "found", repository: MAYA_DUCK, joined: false }, selection: "maya/Duck" } },
  { when: "Holds another pet", preset: { entry: `${LOGIN}/Owl`, lookup: { kind: "error", message: `${LOGIN}/Owl holds Owl, not Duck.` } } },
  { when: "Nothing there", preset: { entry: "maya/Ducky", lookup: { kind: "not_found", address: "maya/Ducky" }, selection: null } },
  { when: "Joining with a code", preset: { entry: CODE, joining: true } },
  { when: "Joined", preset: { entry: "maya/Duck", lookup: { kind: "found", repository: { ...MAYA_DUCK, role: "member" }, joined: true }, selection: "maya/Duck" } },
  { when: "Code didn’t work", preset: { entry: CODE, joinError: EXPIRED } },
  { when: "Can’t load", preset: { listing: { error: "Couldn’t reach Mallard Cloud." }, selection: null } },
  { when: "Connecting", preset: { selection: "kai/Duck", working: true } },
  { when: "Couldn’t connect", preset: { selection: "kai/Duck", failure: "Mallard Cloud didn’t answer. Try again." } },
];

const ADD_STATES: Array<{ when: string; preset: ConnectPreset }> = [
  { when: "Loading", preset: { listing: "loading" } },
  { when: "Shared with you", preset: { selection: "kai/Biscuit" } },
  { when: "Nothing shared yet", preset: { rows: [] } },
  { when: "Found by address", preset: { entry: "maya/Mochi", lookup: { kind: "found", repository: MAYA_MOCHI, joined: false }, selection: "maya/Mochi" } },
  { when: "Already in this row", preset: { entry: "maya/Duck", lookup: { kind: "error", message: `${rowLabel("pets")} already has Duck.` } } },
  { when: "Yours", preset: { entry: `${LOGIN}/Owl`, lookup: { kind: "error", message: `${LOGIN}/Owl is yours — already listed.` } } },
  { when: "Another kind", preset: { entry: "maya/pdf-tools", lookup: { kind: "error", message: "maya/pdf-tools is a skill, not a pet." } } },
  { when: "Nothing there", preset: { entry: "maya/Mochie", lookup: { kind: "not_found", address: "maya/Mochie" } } },
  { when: "Joined with a code", preset: { entry: "maya/Mochi", lookup: { kind: "found", repository: { ...MAYA_MOCHI, role: "member" }, joined: true }, selection: "maya/Mochi" } },
  { when: "Code didn’t work", preset: { entry: CODE, joinError: EXPIRED } },
  { when: "Adding", preset: { selection: "kai/Biscuit", working: true } },
  { when: "Couldn’t add", preset: { selection: "kai/Biscuit", failure: "Mallard Cloud didn’t answer. Try again." } },
];

const DOOR_STATES: Array<{ when: string; preset: ConnectPreset }> = [
  { when: "Add Catalog resource · no Mallard Cloud account", preset: { noAccount: true } },
  { when: "Add Catalog resource · no row of that kind yet", preset: { selection: "kai/Biscuit", newRow: true } },
];

function ChooserStatesBoard({ view, reason }: { view: View; reason: "connect" | "add" }) {
  const states = reason === "connect" ? CONNECT_STATES : ADD_STATES;
  const at: View = { ...view, page: "item", item: "duck" };
  return (
    <div className="connect-states">
      {states.map((state) => (
        <section key={state.when} className="connect-state">
          <header><strong>{state.when}</strong></header>
          <ConnectDialog reason={reason} view={at} preset={{ listing: "ready", ...state.preset }} onCancel={noop} onConnected={noop} />
        </section>
      ))}
      {reason === "add" && DOOR_STATES.map((state) => (
        <section key={state.when} className="connect-state">
          <header><strong>{state.when}</strong></header>
          <div className="mock-frozen-page">
            <AddResourcePage view={{ ...view, door: "cloud" }} go={noop} preset={{ listing: "ready", ...state.preset }} onAdded={noop} />
          </div>
        </section>
      ))}
    </div>
  );
}

/* ——— repository states, on an item page's bar —————————————————————————— */

function StateBar({ status }: { status: ManagedCloudStatus }) {
  const blocked = cloudSyncBlocked(status);
  return (
    <div className="sync-path-actions pet-state-bar">
      <ManagedCloudChip status={status} storageName={STORAGE_NAME} busy={false} refreshing={false} onOpenUrl={noop} onRequestRelease={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" disabled={blocked}><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
        <button type="button" className="sync-path-sync" disabled={blocked}><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
      </div>
    </div>
  );
}

function StateNote({ status }: { status: ManagedCloudStatus }) {
  const note = cloudStatusNotice(status);
  if (!note) return <p className="pet-state-quiet">No note: the chip says it all.</p>;
  const item: NoticeItem = {
    key: "managed-cloud-status", severity: "warning", title: note.title, message: note.message,
    action: note.action === null ? undefined : <button type="button" className="btn btn-ghost">{note.action === "storage_settings" ? "Storage settings" : "Retry"}</button>,
  };
  return <NoticeZone items={[item]} subject="notes about this pet" timing="standing" label="Notes about this pet" />;
}

function StatesBoard() {
  const specimens: Array<{ when: string; where: string; status: ManagedCloudStatus }> = [
    { when: "Not pushed", where: "Create and connect made the repository; nothing is in it yet.", status: STATUS.notPushed },
    { when: "Private", where: "Pushed. Collaborators only.", status: STATUS.duckPrivate },
    { when: "In review", where: "A release request waits for the Owner.", status: STATUS.inReview },
    { when: "Published", where: "The draft is the public version.", status: STATUS.published },
    { when: "Draft ahead of public", where: "The Owner can discard back to the public version.", status: STATUS.mossy },
    { when: "Gone", where: "Removed on the website, or access lost. Pull and Push stay open; Push creates it again.", status: STATUS.gone },
    { when: "Being deleted", where: "The service is still erasing it.", status: STATUS.deleting },
    { when: "Sign in", where: "Pull and Push close; the note sends you to Storage settings.", status: STATUS.signIn },
    { when: "No access", where: "Pull and Push close; Retry compares again.", status: STATUS.refused },
  ];
  return (
    <div className="pet-states">
      {specimens.map((specimen) => (
        <section key={specimen.when} className="pet-state">
          <header><strong>{specimen.when}</strong><span>{specimen.where}</span></header>
          <StateBar status={specimen.status} />
          <StateNote status={specimen.status} />
        </section>
      ))}
      <section className="pet-state">
        <header><strong>The Owner’s panel, draft ahead of public</strong><span>The project’s verbs. No Change repository…, no Disconnect: a connected pet keeps its repository.</span></header>
        <div className="sync-path-menu pet-held-panel">
          <ManagedCloudPanel status={STATUS.mossy} storageName={STORAGE_NAME} busy={false} refreshing={false} close={noop}
            onOpenUrl={noop} onRequestRelease={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
        </div>
      </section>
      <section className="pet-state">
        <header><strong>Learned from the listing, not saved yet</strong><span>Release and discard name the saved binding, so they wait until a Push or Pull saves it, as they wait for a project whose pin moved.</span></header>
        <div className="sync-path-menu pet-held-panel">
          <ManagedCloudPanel status={STATUS.learned} storageName={STORAGE_NAME} busy={false} refreshing={false} close={noop}
            onOpenUrl={noop} onRequestRelease={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
        </div>
      </section>
    </div>
  );
}

/* ——— the project's own cards, beside the proposed ones ——————————————————— */

/* The project's cards come from its own builders (syncOutcome.ts) over a
   three-session comparison; only the fields those builders read are given. */
const PROJECT_THREADS = [
  { id: "res-1", title: "Add provider-neutral session detail", thread: "019a2f4e-7c1e-7b8a-9d2f-3e6a1c0b5d41", minutes: 12 },
  { id: "res-2", title: "Fix bundled Codex CLI path detection", thread: "019a2f51-0a3b-7c4d-8e5f-6a7b8c9d0e12", minutes: 47 },
  { id: "res-3", title: "Session visuals and Git history", thread: "019a2f63-4b5c-7d6e-9f80-1a2b3c4d5e6f", minutes: 95 },
];
const projectComparison = (heldLast: boolean) => ({
  entries: PROJECT_THREADS.map((thread, index) => {
    const held = heldLast && index === PROJECT_THREADS.length - 1;
    return {
      resource_id: thread.id, display_name: thread.title, thread_id: thread.thread, provider_display_name: "Codex",
      state: held ? "storage" : "local", local_updated_at: now - thread.minutes * 60, storage_updated_at: null,
      push_decision: { action: held ? "require_pull" : "publish", selectable: !held },
    };
  }),
}) as unknown as ThreadSyncComparison;
const PUSH_RESULT = { success: true, message: "", revision: 12, results: [] };
const projectPush = (heldLast: boolean): SyncOutcome => buildPushOutcome({
  projectId: "project", storageId: "storage-cloud", selectedResourceIds: new Set(PROJECT_THREADS.map((thread) => thread.id)),
  threadComparison: projectComparison(heldLast), result: PUSH_RESULT,
});
const projectPull = (keptLast: boolean): SyncOutcome => {
  const pulled = PROJECT_THREADS.slice(0, 2);
  return buildPullOutcome({
    projectId: "project",
    plan: {
      storage_id: "storage-cloud", revision: 12, actions: [],
      conversation_review: pulled.map((thread, index) => ({ resource_id: thread.id, thread_id: thread.thread, action_id: `action-${index}` })),
    } as unknown as PullReview,
    result: {
      success: true, message: "", readiness: null, readiness_error: null, plan_id: "plan-12", reviewed_base_advanced: true,
      applied_action_ids: pulled.map((_, index) => `action-${index}`), failed_actions: [], after_pull_actions: [], stopped: null, not_pulled_action_ids: [],
      preserved_threads: keptLast ? [{
        action_id: "action-1", original_thread_id: pulled[1].thread,
        preserved_thread_id: "019a2f77-2c3d-7e4f-8a90-b1c2d3e4f5a6", preserved_display_title: `${pulled[1].title} (local)`,
      }] : [],
    } as unknown as RestoreResult,
    readiness: null,
    items: pulled.map((thread) => ({ resourceId: thread.id, threadTitle: thread.title, title: thread.title })) as unknown as PullReviewItem[],
    completedResourceIds: new Set(pulled.map((thread) => thread.id)),
  });
};

/* A pet held for Pull, as buildPushOutcome reads a held session: warning
   tone, the count of what did push, the held row's reason, Open Pull review. */
const PETS_PUSH_HELD: SyncOutcomeView = {
  tone: "warning", title: "Pushed 1 of 2 pets v2",
  stats: [{ icon: "upload", value: "1", label: "pushed" }, { icon: "git-compare-arrows", value: "1", label: "held for Pull" }],
  rows: [
    { key: "mossy", title: "Mossy", status: "synced", threadId: `${LOGIN}/Mossy`, updatedAt: now },
    { key: "pip", title: "Pip", status: "held", reason: "Mallard Cloud has a newer Pip this Mac hasn’t reviewed. Pull it before pushing.", threadId: `${LOGIN}/Pip`, updatedAt: now - 3 * 3600 },
  ],
  actions: [{ kind: "open_pull", label: "Open Pull review" }],
  defaultOpen: true,
  technical: [],
};
const openPullPresentation = (action: OutcomeAction, index: number) => ({ key: `${action.kind}:${index}`, icon: "download" as const, available: true });

const COMPARISONS: Array<{ label: string; note: string; project: SyncOutcome; proposed: SyncOutcomeView[] }> = [
  {
    label: "Push complete",
    note: "Folded on the Storage card. The count names what moved — sessions there, pets here — and one repository’s version leads. Expand ⌄: each row carries its identifier and its time on line two.",
    project: projectPush(false),
    proposed: [
      pushOutcomeView({ ...START, page: "item", duck: "pushed", lastPush: ["duck"] }, true),
      pushOutcomeView({ ...START, lastPush: ["duck", "mossy"], lastPushCreated: ["duck"] }, false),
    ],
  },
  {
    label: "Pull complete",
    note: "Restored, and the version for one repository; a batch reads several repositories, so its rows name them instead.",
    project: projectPull(false),
    proposed: [
      pullOutcomeView({ ...START, page: "item", item: "mochi", added: ["mochi"], pullChoices: { mochi: "install" } }, true),
      pullOutcomeView({ ...START, pullChoices: { fable: "install", pip: "update" } }, false),
    ],
  },
  {
    label: "Pull that kept the local copy",
    note: "Kept independently, with the reason in the row’s ⓘ: a session there, a skill here.",
    project: projectPull(true),
    proposed: [pullOutcomeView({ ...START, kind: "skills", pullChoices: { "deploy-checklist": "install", "pdf-tools": "update", "sql-review": "keep" } }, false)],
  },
  {
    label: "Push with an item held for Pull",
    note: "Warning tone, the held row open with its reason, and Open Pull review.",
    project: projectPush(true),
    proposed: [PETS_PUSH_HELD],
  },
];

function CompareBoard() {
  return (
    <div className="mock-compare">
      <div className="mock-compare-columns">
        <strong>Project · shipped, from syncOutcome.ts</strong>
        <strong>Pets and skills on Mallard Cloud · proposed</strong>
      </div>
      {COMPARISONS.map((row) => (
        <section key={row.label} className="mock-compare-row">
          <header><strong>{row.label}</strong><span>{row.note}</span></header>
          <div className="mock-compare-cells">
            <div>
              <StorageCard><SyncOutcomeNotice outcome={row.project} onDismiss={noop} onAction={noop} presentAction={projectOutcomeAction} /></StorageCard>
            </div>
            <div>
              {row.proposed.map((outcome, index) => (
                <StorageCard key={index}><SyncOutcomeNotice outcome={outcome} onDismiss={noop} onAction={noop} presentAction={openPullPresentation} /></StorageCard>
              ))}
            </div>
          </div>
        </section>
      ))}
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
}
const SKILLS_START: View = { ...START, kind: "skills" };
const SCENES: Scene[] = [
  {
    id: "today", title: "Live", view: { ...START, page: "today" },
    shows: "The implemented page, live on a stand-in backend. A name opens its page; Change… connects (try maya/Duck); the bar’s + adds kai/Biscuit; Skills’ release-notes is in two repositories; the sidebar’s + opens Add Catalog resource with its From Mallard Cloud door.",
    mirrors: "—",
    today: "This is the build.",
  },
  {
    id: "compare", title: "Notifications side by side", view: { ...START, page: "compare" },
    shows: "The completion notification, double-checked: the project’s own cards, built by its buildPushOutcome and buildPullOutcome, beside the proposed pet and skill cards — each on the Storage card, in the same words, folded the same way.",
    mirrors: "syncOutcome.ts and the project page’s Storage card.",
    today: "The Catalog’s card says “N completed” or “N applied”, has no identifier or time on its rows, and sits bare at the top of the page.",
  },
  {
    id: "list", title: "Catalog row", view: START,
    shows: "On Mallard Cloud the list keeps today’s cards without the Cloud slot. A pet’s name opens its page, as a session’s title does on Activity. The bar’s + adds from Mallard Cloud, where Cloud repositories is today.",
    mirrors: "A project’s sessions list: row badges, then batch Pull and Push in the bar.",
    today: "A Cloud chip or “Connect Cloud repository” on every row, one status request each, and “Cloud repositories” in the bar.",
  },
  {
    id: "pet", title: "Duck, no repository", view: { ...START, page: "item" },
    shows: "Duck’s page. Repository: Rika321/Duck, New, Private. Push creates it; Change… picks another (scene 4). Once connected, it’s fixed.",
    mirrors: "A project’s Storage card before a storage is linked.",
    today: "The first Push creates Rika321/Duck without ever naming it.",
  },
  {
    id: "connect", title: "Connect Duck", view: { ...START, page: "item", dialog: "connect" },
    shows: "The project’s dialog. Pick New repository or kai/Duck, or type an address (maya/Duck) or an invite code. The footer says it can’t be changed later. Every state: scene 5.",
    mirrors: "Connect {project} (BundleConnectionDialog with RepositoryAddressPicker).",
    today: "The inline “Cloud repositories” panel: bare rows, then “Connect and review” opens Pull.",
  },
  {
    id: "connect-states", title: "Connect states", view: { ...START, page: "connect-states" },
    shows: "Every state of the dialog, frozen side by side.",
    mirrors: "The project’s picker: Loading repositories…, field errors, Joining…, the danger notice, Connecting….",
    today: "A list and a button; errors as one line.",
  },
  {
    id: "connected", title: "Created and connected", view: { ...START, page: "item", duck: "created", note: `Duck connected to ${LOGIN}/Duck.` },
    shows: "Create and connect made Rika321/Duck, empty. The card goes; the chip reads Not pushed. Its panel has no Change repository…: Duck keeps it. Push is next.",
    mirrors: "A project’s Link: the registration creates the repository, and its storage moves to the crumb.",
    today: "No such step.",
  },
  {
    id: "connected-existing", title: "Connected to kai/Duck", view: { ...START, page: "item", duck: "kai", note: "Duck connected to kai/Duck." },
    shows: "Picked from the list. The card goes; the chip reads Private (Draft v4); Pull 1 reviews it. Nothing is pulled on its own, and there is no Change repository….",
    mirrors: "A project connected to an existing repository: its state in the Cloud chip, Pull to review what is there.",
    today: "Connect and review opens the Pull review at once.",
  },
  {
    id: "push", title: "Push review", view: { ...START, page: "push-item", duck: "created" },
    shows: "The project’s Push review, inside the Storage card: Preparing… first (“Comparing this computer with Mallard Cloud”), then the review, holding Duck alone; line two names where it goes. The footer’s verb is Push — the count stays in the selection — and a running push says it can’t be cancelled midway.",
    mirrors: "PushResourceWorkspace in the project’s Storage card, and the bar’s Preparing… / Push / Pushing….",
    today: "Push only from the row page, and the review never says where an item goes.",
  },
  {
    id: "pushed", title: "Pushed", view: { ...START, page: "item", duck: "pushed", outcome: "push", lastPush: ["duck"] },
    shows: "The result lands on the Storage card, folded: Done · Push complete · 1 pet v2 · Version 1, as a project’s reads 3 sessions · Version 12. The chip’s panel holds Request public release and View draft; no Change repository…, no Disconnect.",
    mirrors: "The project’s outcome on its Storage card, and the Cloud chip in its bar.",
    today: "Push complete · 1 completed, bare at the top of the page; the chip sits on the row, with Change and an unconfirmed Disconnect.",
  },
  {
    id: "states", title: "Repository states", view: { ...START, page: "states" },
    shows: "The project’s chip words and workspace notes on a pet’s bar. Pull and Push close as cloudSyncBlocked says.",
    mirrors: "cloud-gone-live.html, word for word.",
    today: "These states live only inside each row chip’s panel; Pull and Push never close.",
  },
  {
    id: "batch", title: "Batch Push", view: { ...START, page: "push-batch" },
    shows: "Many new pets make one review, not many dialogs: each row names its repository on line two, and a new one reads New repository. The result is Push complete · 2 pets v2, its rows naming their repositories.",
    mirrors: "The New repository row, in bulk.",
    today: "The same review, with no destinations.",
  },
  {
    id: "pull-pets", title: "Batch Pull · pets", view: { ...START, page: "pull-batch" },
    shows: "The project’s Pull review, inside the Storage card after Preparing… (“Reading the latest version from Mallard Cloud”): each row keeps today’s local action (Install, Update), and line two names the repository it reads, with the version in its tooltip. Pull N at the foot and on the bar; a running pull says it can’t be cancelled.",
    mirrors: "RestorePlanView in the project’s Storage card — here every pet is its own repository, so every row says which, and the header’s Version shows for one pet.",
    today: "The same review, with no sources.",
  },
  {
    id: "pulled-pets", title: "Pulled · pets", view: { ...START, outcome: "pull", pulled: ["fable", "pip"], pullChoices: { fable: "install", pip: "update" } },
    shows: "Done · Pull complete · 2 restored, folded on the Storage card; expand it and each row names its repository and time. Fable is on this Mac now, and Pip matches.",
    mirrors: "The project’s Pull outcome: “2 restored · Version 12”, on its Storage card.",
    today: "Pull complete · 2 applied, bare at the top of the page, with no sources.",
  },
  {
    id: "skills", title: "Skills row", view: SKILLS_START,
    shows: "The same list for skills, with no artwork. The bar counts as today: Pull 2 for the two newer in Mallard Cloud, and ⚠ 2 for sql-review (changed on both sides) and release-notes, held because Mallard Cloud refused this account for maya/release-notes. Its badge says so, and its page carries the project’s Repository unavailable note.",
    mirrors: "A project whose repository refuses the account: the note, and Pull closed.",
    today: "A No access chip on that row and nothing else; Pull still offers the rest without a word.",
  },
  {
    id: "pull-skills", title: "Batch Pull · skills", view: { ...SKILLS_START, page: "pull-batch" },
    shows: "Three skills pull. sql-review changed here too, so it keeps the local copy as sql-review-2 by default; its chip’s menu offers Overwrite, which asks for approval on the card. The held skill is named above the list.",
    mirrors: "The held-for-Pull banner of today’s Push review, and today’s keep-a-copy and overwrite actions.",
    today: "The held skill is not mentioned, and the rows name no source.",
  },
  {
    id: "pulled-skills", title: "Pulled · skills", view: { ...SKILLS_START, outcome: "pull", pulled: ["deploy-checklist", "pdf-tools", "sql-review"], pullChoices: { "deploy-checklist": "install", "pdf-tools": "update", "sql-review": "keep" } },
    shows: "Pull complete · 2 restored · 1 kept independently, as a project reads a session it kept. Each row names its repository on line two; the local copy stays as sql-review-2, a new skill on this Mac, whose page shows New repository Rika321/sql-review-2 with Choose repository…, and the bar offers Push 1.",
    mirrors: "The project’s Pull outcome, kept-independently row and all.",
    today: "Pull complete · 3 applied, with no sources.",
  },
  {
    id: "add", title: "Add from Mallard Cloud", view: { ...START, dialog: "add" },
    shows: "Start 1, the row page: the bar’s + (Add from Mallard Cloud), beside refresh, where Cloud repositories is today. Pick kai/Biscuit, shared with you, or paste an address (maya/Mochi) or invite code.",
    mirrors: "New project › From Mallard Cloud, for this row’s kind.",
    today: "Cloud repositories opens an inline panel of bare rows.",
  },
  {
    id: "add-door", title: "Add Catalog resource", view: { ...START, page: "add-resource", door: "cloud" },
    shows: "Start 2, the sidebar’s Catalog +: New project’s two doors. From Mallard Cloud: Storage, Repository (its kind comes with it), Catalog row, Add.",
    mirrors: "New project › On this computer | From Mallard Cloud.",
    today: "One way in: Kind, then Agent profile.",
  },
  {
    id: "add-states", title: "Add states", view: { ...START, page: "add-states" },
    shows: "Every state of Add, frozen: the dialog, then the door’s own two.",
    mirrors: "The project’s picker and its From Mallard Cloud door.",
    today: "A list and a button.",
  },
  {
    id: "added", title: "Added", view: { ...START, page: "item", item: "biscuit", added: ["biscuit"], note: "Biscuit added from kai/Biscuit." },
    shows: "Biscuit opens on its page: Only in Mallard Cloud, Pull 1. Nothing is pulled on its own, and its repository is fixed.",
    mirrors: "From Mallard Cloud opens a project on Activity; nothing is pulled.",
    today: "“Connect and review” opens the Pull review.",
  },
  {
    id: "several", title: "Two repositories", view: { ...START, page: "item", item: "pip", several: true },
    shows: "No more page failure: the row is blocked, and its page asks which repository to use.",
    mirrors: "Choose a repository (the project dialog’s missing reason).",
    today: "The whole page fails: “Several Cloud repositories contain this item.”",
  },
  {
    id: "other", title: "Other storages", view: { ...START, storage: "vault" },
    shows: "Unchanged. A Catalog row on a local folder, R2, Dropbox or Feishu storage is one repository with one version: plain names, batch Pull and Push, no item page, no Cloud chip, no sources or destinations, no held banner. Push ends at Version 8, as today. Switch storages from the storage crumb; the sidebar switches to Skills.",
    mirrors: "Today’s Catalog row page, as shipped.",
    today: "The same.",
  },
];

/** Whether two views show the same step; the shell note does not count. */
const sameView = (a: View, b: View) => a.storage === b.storage && a.kind === b.kind && a.page === b.page && a.item === b.item
  && a.duck === b.duck && a.dialog === b.dialog && a.outcome === b.outcome && a.several === b.several && a.door === b.door
  && a.pushed.join() === b.pushed.join() && a.pulled.join() === b.pulled.join() && a.added.join() === b.added.join();

function Harness() {
  const params = new URLSearchParams(window.location.search);
  const initial = Math.max(0, SCENES.findIndex((scene) => scene.id === (params.get("scene") ?? "today")));
  const [sceneIndex, setSceneIndex] = useState(initial);
  const [view, setView] = useState<View>(SCENES[initial].view);
  /* Entering Today starts its stand-in backend over. */
  const [todayRun, setTodayRun] = useState(0);
  /* The live Add Catalog resource lands on the item's page in the live row. */
  const [landing, setLanding] = useState<CatalogCloudLanding | null>(null);
  const [theme, setTheme] = useState(params.get("theme") === "dark" ? "dark" : "light");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
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
    if (SCENES[index].view.page === "today") {
      resetToday();
      setTodayRun((run) => run + 1);
    }
    setView(SCENES[index].view);
  };
  const go: Go = (next) => setView((current) => ({ ...current, ...next }));
  /* The sidebar's Pets v2 and Skills rows switch the Catalog row, today's
     page included. */
  const openScope = (scopeId: string) => {
    const kind = scopeId === KINDS.skills.scopeId ? "skills" : scopeId === KINDS.pets.scopeId ? "pets" : null;
    if (!kind) return;
    if (view.page === "today" || view.page === "today-add") {
      setLanding(null);
      setView({ ...view, page: "today", kind });
    } else setView({ ...START, storage: view.storage, kind });
  };

  /* Add, from the bar or the door: the item lands on its page in its row. */
  const added = (repository: CloudRow) => {
    const kind = kindOf(repository.coordinate) ?? view.kind;
    const id = (Object.keys(ITEMS) as ItemId[]).find((key) => ITEMS[key].kind === kind && ITEMS[key].name === repository.item) ?? ADDED[kind];
    setView({ ...START, kind, page: "item", item: id, added: [id], note: `${ITEMS[id].name} added from ${repository.address}.` });
  };
  const connected = (choice: { kind: "new" } | { kind: "existing"; repository: CloudRow }) => {
    const reason = view.dialog;
    if (reason === "add") {
      if (choice.kind === "existing") added(choice.repository);
      return;
    }
    if (reason === "several") {
      const address = choice.kind === "existing" ? choice.repository.address : `${LOGIN}/Pip`;
      setView({ ...view, dialog: null, several: false, note: `Pip connected to ${address}.` });
      return;
    }
    const name = ITEMS[view.item].name;
    if (choice.kind === "new") {
      setView({ ...view, dialog: null, duck: view.item === "duck" ? "created" : view.duck, note: `${name} connected to ${LOGIN}/${name}.` });
      return;
    }
    const mine = choice.repository.owner === LOGIN;
    const other: DuckRepository = choice.repository.owner === "kai" ? "kai" : "maya";
    setView({ ...view, dialog: null, duck: view.item === "duck" && !mine ? other : view.duck, outcome: null, note: `${name} connected to ${choice.repository.address}.` });
  };

  let page: ReactNode;
  switch (view.page) {
    case "today": page = <TodayPage key={`${todayRun}:${view.kind}:${landing?.resourceId ?? ""}`} kind={view.kind} landing={landing} />; break;
    case "today-add":
      page = (
        <AddCatalogScopeWorkspace workflowEpoch={0} isWorkflowCurrent={() => true} busy={false} initialChoiceKey={null}
          scopes={SCOPES} storages={TODAY_STORAGES} cloudAccounts={[{ id: "storage-cloud", name: STORAGE_NAME, account_login: LOGIN }]}
          onSetUpCloud={noop} onClose={() => go({ page: "today" })} onCreated={noop}
          onAdded={(added) => {
            setLanding(added);
            setView({ ...view, page: "today", kind: kindOfScope(added.scope.scope.scope_id) });
          }} />
      );
      break;
    case "list": page = <ListPage key={`${view.kind}:${view.storage}`} view={view} go={go} />; break;
    case "item": page = <ItemPage view={view} go={go} />; break;
    case "push-item": page = <PushReview key={`push:${view.item}`} view={view} go={go} ids={[view.item]} single />; break;
    case "push-batch": page = <PushReview key={`push:${view.kind}:${view.storage}`} view={view} go={go} ids={pushIds(view)} single={false} />; break;
    case "pull-item": page = <PullReview key={`pull:${view.item}`} view={view} go={go} ids={[view.item]} single />; break;
    case "pull-batch": page = <PullReview key={`pull:${view.kind}:${view.storage}`} view={view} go={go} ids={pullIds(view)} single={false} />; break;
    case "add-resource": page = <AddResourcePage view={view} go={go} onAdded={added} />; break;
    case "states":
    case "connect-states":
    case "add-states":
    case "compare": page = null; break;
  }

  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Pets and skills on Mallard Cloud, the way projects are</strong>
        <span className="harness-scope">Only Mallard Cloud changes; other storages keep today’s flow ({SCENES.length - 1}).</span>
        <nav className="harness-scenes" aria-label="Scenes">
          {SCENES.map((candidate, index) => (
            <button key={candidate.id} type="button"
              className={[index === sceneIndex ? "active" : "", candidate.id === "today" ? "today" : ""].filter(Boolean).join(" ") || undefined}
              aria-current={index === sceneIndex ? "step" : undefined} onClick={() => jump(index)}>
              <b>{index}</b> {candidate.title}
            </button>
          ))}
        </nav>
        <label><input type="checkbox" checked={theme === "dark"} onChange={(event) => setTheme(event.target.checked ? "dark" : "light")} /> Dark</label>
      </div>
      <div className="harness-notes">
        <p><b>{sceneIndex} · {scene.title}.</b> {scene.shows}</p>
        {scene.id !== "today" && <p><span>Mirrors</span>{scene.mirrors}</p>}
        {scene.id !== "today" && <p><span>Today</span>{scene.today}</p>}
      </div>
      {view.page === "states" ? <StatesBoard /> : view.page === "compare" ? <CompareBoard /> : view.page === "connect-states" ? <ChooserStatesBoard view={view} reason="connect" /> : view.page === "add-states" ? <ChooserStatesBoard view={view} reason="add" /> : (
        <div className="harness-frame">
          <Shell kind={view.kind} adding={view.page === "add-resource" || view.page === "today-add"} onOpenScope={openScope}
            onAddScope={() => (view.page === "today" || view.page === "today-add"
              ? go({ page: "today-add" })
              : go({ page: "add-resource", door: "local", dialog: null, outcome: null, note: null }))}>{page}</Shell>
        </div>
      )}
      {view.dialog && (
        <ConnectDialog key={`${view.dialog}:${view.item}:${view.duck}`} reason={view.dialog} view={view} onCancel={() => go({ dialog: null })} onConnected={connected} />
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Harness />
  </StrictMode>,
);
