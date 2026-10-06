import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import App from "../src/App";
import type { AppLinkAnswer, AppLinkArrival, AppLinkRepository } from "../src/types";

/**
 * Open in Mallard, live: the shipped shell on a stand-in backend, taking
 * links the way it takes them from the system — `take_app_link` at startup
 * and whenever `app-link` fires. `npm run dev`, then /mockups/deep-link-live.html
 * (`?accounts=one|two|none`, `?cold=<link>` to start the app from one).
 * Links: project-new, project-here, item-new, item-here, own-item, unseen,
 * other-host, invalid, and the listing's: listing, listing-pets,
 * listing-staging. Pages a link only opens (a project, a Catalog row)
 * call commands this backend does not answer; the bar and the rail show
 * where it went.
 */

const HOME = "/Users/you";
const HOST = "mallard-ai.com";
const ORIGIN = `https://${HOST}`;
const now = Math.floor(Date.now() / 1000);
const query = new URLSearchParams(window.location.search);
const accounts = query.get("accounts") ?? "one";

const cloud = (id: string, name: string, login: string) => ({
  storage_id: id, name, provider: "managed_cloud", config_version: 1, provider_label: "Mallard Cloud", icon: "cloud", available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: login, is_path: false },
    { field_id: "service_origin", label: "Service", value: ORIGIN, is_path: false },
  ],
  settings: { service_origin: ORIGIN, account_id: `user_${login}`, account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const PERSONAL = cloud("storage-personal", "Mallard Cloud", "Rika321");
const WORK = cloud("storage-work", "Mallard Cloud work", "rika-acme");
const STORAGES = accounts === "none" ? [] : accounts === "two" ? [PERSONAL, WORK] : [PERSONAL];

const registration = (id: string, name: string, bundle: string) => ({
  local_project_id: id, bundle_id: bundle, display_name: name, local_alias: null, repository_fingerprint: null,
  recipe: { schema_version: 1, revision: 1, entries: {} }, revision: 1, created_at: now, updated_at: now,
});
const PROJECTS = [registration("project-api", "api-server", "a".repeat(32)), registration("project-physics", "physics 225", "c".repeat(32))];
const LINKS = accounts === "none" ? [] : [{
  managed_cloud_repository_id: "repo_api_server", local_project_id: "project-api", storage_id: PERSONAL.storage_id, bundle_id: "a".repeat(32),
  project_content_preferences: {}, conversation_checkpoints: {}, pinned: false, created_at: now,
}];
const CONFIG = { schema: 1, revision: 3, storages: STORAGES, projects: PROJECTS, links: LINKS };

const PET = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const PETS = {
  ...PET, provider_label: "Codex", label: "Pets v2", singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" }, capture: "directory_tree", apply: "automatic", targets: [],
};
const CATALOG_PROFILE = { profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0 };
const PETS_SCOPE = {
  scope: { scope_id: "scope-pets", profile_id: CATALOG_PROFILE.profile_id, ...PET, revision: 1, created_at: 0, updated_at: 0 },
  profile: CATALOG_PROFILE, label: "Pets v2", available: true, provider_label: "Codex", kind_label: "Pets v2", format_label: "Codex pet v2",
};

const cloudProject = (overrides: Record<string, unknown>) => ({
  repository_id: "repo_maya_portal", owner_login: "maya", name: "portal", address: "maya/portal", website_url: `${ORIGIN}/maya/portal`,
  bundle_id: "b".repeat(32), repository_class: "git_project", visibility: "public", role: null, can_push: true,
  repository_fingerprint: "f".repeat(64), git_remote: "differs", github_url: "https://github.com/maya/portal", revision: 6,
  updated_at: now - 9 * 86_400, resources: 14, registration_only: false, description: "Customer portal, built with Codex.",
  star_count: 7, viewer_starred: true, ...overrides,
});
const cloudPet = {
  ...PET, provider_label: "Codex", kind_label: "Pets v2", format_label: "Codex pet v2",
  repository_id: "repo_maya_mochi", owner_login: "maya", name: "Mochi", address: "maya/Mochi", description: "A round rice cake that wobbles when you type.",
  star_count: 12, viewer_starred: true, website_url: `${ORIGIN}/maya/Mochi`, visibility: "public", role: null, can_push: false, revision: 5,
  updated_at: now - 6 * 86_400, registration_only: false, resource_id: "mochi", display_name: "Mochi", available: true,
};

/* ——— what the service says about each address, per account ———————————— */

const seen = (overrides: Partial<AppLinkRepository>): AppLinkRepository => ({
  repository_id: "repo_maya_portal", address: "maya/portal", repository_class: "git_project", bundle_id: "b".repeat(32),
  catalog: null, visibility: "public", role: null, ...overrides,
});
const SIGHTS: Record<string, { personal: AppLinkRepository | null; work: AppLinkRepository | null }> = {
  "maya/portal": { personal: seen({}), work: seen({}) },
  "rika321/api-server": {
    personal: seen({ repository_id: "repo_api_server", address: "Rika321/api-server", bundle_id: "a".repeat(32), visibility: "private", role: "owner" }),
    work: null,
  },
  "maya/mochi": {
    personal: seen({ repository_id: "repo_maya_mochi", address: "maya/Mochi", repository_class: "catalog", bundle_id: null, catalog: { ...PET, resource_id: "mochi" } }),
    work: null,
  },
  "rika321/duck": {
    personal: seen({ repository_id: "repo_duck", address: "Rika321/Duck", repository_class: "catalog", bundle_id: null, catalog: { ...PET, resource_id: "duck" }, role: "owner" }),
    work: null,
  },
  "rika321/fable": {
    personal: seen({ repository_id: "repo_fable", address: "Rika321/Fable", repository_class: "catalog", bundle_id: null, catalog: { ...PET, resource_id: "fable" }, role: "owner", visibility: "private" }),
    work: null,
  },
};

function answerFor(host: string, address: string): AppLinkAnswer {
  const onHost = host === HOST ? STORAGES : [];
  const sights = SIGHTS[address.toLowerCase()] ?? { personal: null, work: null };
  return {
    host,
    address,
    can_set_up: host === HOST,
    accounts: onHost.map((storage) => {
      const repository = storage === PERSONAL ? sights.personal : sights.work;
      return {
        storage_id: storage.storage_id,
        storage_name: storage.name,
        account_login: storage.settings.account_login,
        sight: repository ? { kind: "found", repository } : { kind: "not_found" },
      };
    }),
  };
}

const LINKS_TO_SEND: Record<string, AppLinkArrival> = {
  "project-new": { kind: "repository", host: HOST, address: "maya/portal" },
  "project-here": { kind: "repository", host: HOST, address: "Rika321/api-server" },
  "item-new": { kind: "repository", host: HOST, address: "maya/Mochi" },
  "item-here": { kind: "repository", host: HOST, address: "Rika321/Duck" },
  "own-item": { kind: "repository", host: HOST, address: "Rika321/Fable" },
  unseen: { kind: "repository", host: HOST, address: "acme/billing" },
  "other-host": { kind: "repository", host: "staging.mallard-ai.com", address: "maya/portal" },
  invalid: { kind: "invalid" },
  // Share yours: the website's listing, with and without a kind chip on.
  listing: { kind: "listing", host: HOST, listing_kind: null, can_set_up: true },
  "listing-pets": { kind: "listing", host: HOST, listing_kind: "desktop_pet", can_set_up: true },
  "listing-staging": { kind: "listing", host: "staging.mallard-ai.com", listing_kind: null, can_set_up: false },
};

/* ——— the stand-in backend ————————————————————————————————————————————— */

const log: string[] = [];
let render = () => {};
const note = (line: string) => {
  log.push(line);
  render();
};
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));
const addressOf = (text: string) => text.trim().replace(/^https?:\/\/[^/]+\//i, "").replace(/\/+$/, "").toLowerCase();

/** The link waiting to be taken, as the backend's inbox holds it. */
let waiting: AppLinkArrival | null = LINKS_TO_SEND[query.get("cold") ?? ""] ?? null;
const QUIET = new Set(["plugin:event|listen", "plugin:event|unlisten", "plugin:event|emit", "plugin:path|resolve_directory"]);

(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, string>;
  if (!QUIET.has(command)) note(command);
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "get_host_identity_status": return { host_id: "host-1", revision: 1, created_at: now, updated_at: now };
    case "get_project_sync_config": return CONFIG;
    case "list_local_projects": return PROJECTS;
    case "list_setup_drafts": return { drafts: [] };
    case "list_storage_setup_drafts": return { drafts: [], warnings: [] };
    case "list_catalog_scopes": return [PETS_SCOPE];
    case "list_catalog_capabilities": return [PETS];
    case "list_catalog_resource_profiles": return [CATALOG_PROFILE];
    case "list_project_repository_kinds": return { "project-api": true, "project-physics": false };
    case "list_agent_providers": return [
      { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
    ];
    case "list_storage_providers": return [];
    case "list_provider_profiles": return [{
      profile_id: "p-codex", provider: "codex", display_name: "~/.codex", path: `${HOME}/.codex`, canonical_path: `${HOME}/.codex`,
      revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
    }];
    case "list_project_bindings": return [];
    case "list_agent_folders": return { folders: [] };
    case "list_managed_cloud_repositories": return [];
    case "list_managed_cloud_starred":
    case "list_managed_cloud_public":
    case "list_catalog_cloud_starred":
    case "list_catalog_cloud_public": return { rows: [], next_cursor: null };
    case "list_catalog_cloud_shared": return [];
    case "probe_cloud_project_folder": return { folder: { kind: "new" }, can_clone: true };
    case "resolve_managed_cloud_repository": {
      await wait(300);
      const wanted = addressOf(args.address);
      return wanted === "maya/portal" ? { kind: "public", repository: cloudProject({}) } : { kind: "not_found", address: wanted };
    }
    case "resolve_catalog_cloud_repository": {
      await wait(300);
      const wanted = addressOf(args.address);
      return wanted === "maya/mochi" ? { kind: "found", repository: cloudPet } : { kind: "not_found", address: wanted };
    }
    case "take_app_link": {
      const link = waiting;
      waiting = null;
      return link;
    }
    case "resolve_app_link":
      await wait(500);
      return answerFor(args.host, args.address);
    case "find_catalog_cloud_repository":
      return args.repositoryId === "repo_duck" ? { scope_id: "scope-pets", storage_id: args.storageId, resource_id: "duck" } : null;
    default:
      return null;
  }
}, { shouldMockEvents: true });

/** What the system does when a link is opened: the backend keeps it and says so. */
async function send(name: string) {
  waiting = LINKS_TO_SEND[name];
  note(`— link: ${name}`);
  await emit("app-link");
}

function Harness() {
  const [, setTick] = useState(0);
  render = () => setTick((tick) => tick + 1);
  const busyRef = useRef<HTMLDialogElement>(null);
  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Open a link</strong>
        {Object.keys(LINKS_TO_SEND).map((name) => (
          <button key={name} type="button" data-link={name} onClick={() => void send(name)}>{name}</button>
        ))}
        <span>accounts={accounts}</span>
        <button type="button" data-busy onClick={() => busyRef.current?.showModal()}>Open a dialog first…</button>
        <button type="button" onClick={() => { log.length = 0; render(); }}>Clear log</button>
      </div>
      <div className="harness-frame"><App /></div>
      <pre className="harness-log" aria-live="polite">{log.length ? log.join("\n") : "Backend calls appear here."}</pre>
      <dialog ref={busyRef} className="harness-busy">
        A dialog of the app’s is open. A link that arrives now waits under the bar.
        <div>
          <button type="button" className="btn" data-link-in-dialog onClick={() => void send("project-new")}>Send project-new</button>{" "}
          <button type="button" className="btn" data-listing-in-dialog onClick={() => void send("listing-pets")}>Send listing-pets</button>{" "}
          <button type="button" className="btn" onClick={() => busyRef.current?.close()}>Close</button>
        </div>
      </dialog>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Harness />);
