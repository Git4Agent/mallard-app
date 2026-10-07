import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { ManagedCloudListingRow } from "../src/components/managed-cloud/types";
import NewProjectFolderStep from "../src/components/project-sync/NewProjectFolderStep";
import { projectSyncApi } from "../src/components/project-sync/api";
import type {
  CloudProjectFolder,
  ManagedCloudRepository,
  ManagedCloudRepositoryResolution,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderProfileSummary,
  StorageSummary,
} from "../src/types";
import "../src/App.css";

/**
 * The shipped New project › From Mallard Cloud, live, against a stand-in
 * for the backend it calls. `npm run dev` (or `tauri dev`), then
 * /mockups/cloud-project-live.html (`?accounts=one|several|none&git=yes|no&clone=works|fails&theme=dark`).
 * Addresses that resolve: maya/portal (GitHub), maya/billing-svc (not on
 * GitHub), maya/field-notes (no Git). Folders: …/taken holds files with
 * no Git remote, …/fork holds a checkout of another remote, …/empty is
 * empty, …/checkout is already the clone, …/project is already a project,
 * anything under a missing parent is refused, anything else is new.
 */

const HOME = "/Users/you";
const query = new URLSearchParams(window.location.search);
const knob = (key: string, fallback: string) => query.get(key) ?? fallback;
document.documentElement.dataset.theme = knob("theme", "light");

const cloud = (id: string, login: string): StorageSummary => ({
  storage_id: id,
  name: `Mallard Cloud ${login}`,
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: login, is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: { service_origin: "https://mallard-ai.com", account_id: `user_${login}`, account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const STORAGES: StorageSummary[] = knob("accounts", "several") === "none"
  ? []
  : knob("accounts", "several") === "one"
    ? [cloud("storage-personal", "Rika321")]
    : [cloud("storage-personal", "Rika321"), cloud("storage-work", "rika-acme")];

const now = Math.floor(Date.now() / 1000);
const repo = (overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository => ({
  repository_id: "repo_portal", owner_login: "maya", name: "portal", address: "maya/portal", display_name: overrides.name ?? "portal", description: null, star_count: 0, viewer_starred: false,
  website_url: "https://mallard-ai.com/maya/portal", bundle_id: "b".repeat(32), repository_class: "git_project",
  visibility: "private", role: "member", can_push: true, repository_fingerprint: "f".repeat(64), git_remote: "differs",
  github_url: "https://github.com/acme/portal", revision: 12, updated_at: now - 7_200, resources: 9, registration_only: false,
  ...overrides,
});
const REPOS = [
  repo({}),
  repo({ repository_id: "repo_billing", name: "billing-svc", address: "maya/billing-svc", website_url: "https://mallard-ai.com/maya/billing-svc", bundle_id: "c".repeat(32), github_url: null, repository_fingerprint: "e".repeat(64) }),
  repo({ repository_id: "repo_notes", name: "field-notes", address: "maya/field-notes", website_url: "https://mallard-ai.com/maya/field-notes", bundle_id: "d".repeat(32), repository_class: "non_git_project", github_url: null, repository_fingerprint: null }),
];

const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, has_cli: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const profile = (profileId: string, provider: string, dir: string): ProviderProfileSummary => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const PROFILES = [profile("p-codex", "codex", ".codex"), profile("p-claude", "claude", ".claude")];

const log: string[] = [];
let render = () => {};
const note = (line: string) => {
  log.push(line);
  render();
};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC((command) => (command === "plugin:path|resolve_directory" ? HOME : null));
}

projectSyncApi.listManagedCloudRepositories = async (storageId) => {
  await wait(300);
  return storageId === "storage-work" ? REPOS : [];
};
const listed = (overrides: Partial<ManagedCloudListingRow>): ManagedCloudListingRow => ({
  repository_id: "repo_agent_kit", owner_login: "lin", name: "agent-kit", address: "lin/agent-kit", display_name: overrides.name ?? "agent-kit",
  website_url: "https://mallard-ai.com/lin/agent-kit", repository_class: "git_project", catalog: null, visibility: "public",
  role: null, github_url: "https://github.com/lin/agent-kit", description: "Sessions from building a small agent harness.",
  star_count: 48, viewer_starred: true, revision: 22, updated_at: now - 2 * 86_400, ...overrides,
});
const STARRED = [listed({}), listed({ repository_id: "repo_maya_portal", owner_login: "maya", name: "portal", address: "maya/portal", website_url: "https://mallard-ai.com/maya/portal", github_url: "https://github.com/maya/portal", description: "A portal, a different one.", star_count: 7, revision: 6, updated_at: now - 9 * 86_400 })];
const PUBLIC = [
  listed({ repository_id: "repo_dotfiles", owner_login: "ada", name: "dotfiles", address: "ada/dotfiles", website_url: "https://mallard-ai.com/ada/dotfiles", github_url: "https://github.com/ada/dotfiles", description: "How Ada sets up a Mac.", star_count: 15, viewer_starred: false, revision: 30, updated_at: now - 86_400 }),
  ...STARRED,
  listed({ repository_id: "repo_handbook", owner_login: "leo", name: "handbook", address: "leo/handbook", website_url: "https://mallard-ai.com/leo/handbook", repository_class: "non_git_project", github_url: null, description: "Team handbook, folder only.", star_count: 3, viewer_starred: false, revision: 2, updated_at: now - 4 * 86_400 }),
];
projectSyncApi.listManagedCloudStarred = async () => { await wait(300); return { rows: STARRED, next_cursor: null }; };
projectSyncApi.listManagedCloudPublic = async (_storageId, sort) => {
  await wait(400);
  const rows = sort === "stars" ? [...PUBLIC].sort((a, b) => b.star_count - a.star_count) : PUBLIC;
  return { rows, next_cursor: null };
};
projectSyncApi.resolveManagedCloudRepository = async (storageId, address): Promise<ManagedCloudRepositoryResolution> => {
  note(`resolve_managed_cloud_repository ${storageId} ${address}`);
  await wait(400);
  const name = address.replace(/^https?:\/\/[^/]+\//, "").replace(/^mallard-ai\.com\//, "").toLowerCase();
  const found = REPOS.find((candidate) => candidate.address === name);
  if (!found || storageId !== "storage-work") return { kind: "not_found", address: name };
  return { kind: "collaborator", repository: found };
};
projectSyncApi.probeCloudProjectFolder = async (destination) => {
  const folder: CloudProjectFolder = destination.includes("/nowhere/")
    ? { kind: "refused", message: "Its parent folder doesn’t exist." }
    : destination.endsWith("/taken") ? { kind: "occupied", suggestion: `${destination}-2`, local_fingerprint: null, git_remote: null }
    : destination.endsWith("/fork")
      ? { kind: "occupied", suggestion: `${destination}-2`, local_fingerprint: "e".repeat(64), git_remote: "github.com/maya/portal" }
    : destination.endsWith("/project") ? { kind: "project", project_id: "project-api" }
    : destination.endsWith("/empty") ? { kind: "empty" }
    : destination.endsWith("/checkout") ? { kind: "checkout" }
    : { kind: "new" };
  return { folder, can_clone: knob("git", "yes") === "yes" };
};
let cancelled = false;
projectSyncApi.cancelCloudProjectFolder = async (operationId) => {
  note(`cancel_cloud_project_folder ${operationId}`);
  cancelled = true;
};
projectSyncApi.getCloudProjectFolder = async (request, onProgress) => {
  note(`get_cloud_project_folder ${request.destination} github_url=${request.github_url ?? "none"}`);
  cancelled = false;
  const canClone = knob("git", "yes") === "yes";
  if (request.is_git_project && !canClone) throw new Error("Install Git to create this Git project.");
  if (request.github_url && canClone) {
    for (const percent of [null, 12, 48, 81]) {
      if (cancelled) throw new Error("Stopped.");
      onProgress({ stage: "cloning", percent });
      await wait(450);
    }
    if (knob("clone", "works") === "works") return { kind: "cloned", canonical_path: request.destination };
  }
  onProgress({ stage: "creating" });
  await wait(300);
  return request.is_git_project
    ? { kind: "initialized", canonical_path: request.destination, failure: {
      reason: request.github_url ? "refused" : "missing_source",
      command: request.github_url ? `git -C ${request.destination} pull origin HEAD` : null,
    } }
    : { kind: "created", canonical_path: request.destination };
};
let draft: ProjectSetupDraft | null = null;
projectSyncApi.createSetupDraft = async (projectRoot) => {
  note(`create_setup_draft ${projectRoot}`);
  draft = {
    schema: 1, draft_id: "draft-1", local_project_id: "project-1", new_bundle_id: "a".repeat(32), project_root: projectRoot,
    canonical_project_root: projectRoot, display_name: projectRoot.split("/").pop() ?? "project",
    repository_fingerprint: null, profiles: {}, storage: null, repository: { kind: "new" }, revision: 0, created_at: now, updated_at: now,
  };
  return { draft, resumed: false };
};
projectSyncApi.updateSetupDraft = async (next) => {
  note(`update_setup_draft storage=${next.storage?.storage_id} repository=${next.repository.kind === "existing" ? next.repository.display_name : "new"} ack=${next.repository.kind === "existing" ? next.repository.mismatch_acknowledged : "-"} profiles=${Object.keys(next.profiles).join(",")}`);
  draft = { ...next, revision: next.revision + 1 };
  return draft;
};
projectSyncApi.finalizeProjectSetup = async (draftId, revision) => {
  note(`finalize_project_setup ${draftId} r${revision}`);
  await wait(300);
  return {
    project: {
      local_project_id: "project-1", bundle_id: "b".repeat(32), display_name: draft?.display_name ?? "project",
      recipe: {} as never, revision: 1, created_at: now, updated_at: now,
    },
    links: [], binding: null, materializations: [],
  };
};

function Harness() {
  const [, setTick] = useState(0);
  render = () => setTick((tick) => tick + 1);
  const [key, setKey] = useState(0);
  return (
    <div className="harness">
      <div className="harness-controls">
        <strong>Stand-in backend</strong>
        <span>accounts={knob("accounts", "several")} · git={knob("git", "yes")} · clone={knob("clone", "works")}</span>
        <button type="button" className="btn" onClick={() => { log.length = 0; setKey((value) => value + 1); }}>Reset</button>
      </div>
      <div className="harness-frame">
        <main className="sync-main sync-project-links-page sync-project-setup-page">
          <section className="profile-links-section sync-project-overview">
            <NewProjectFolderStep
              key={key}
              workflowEpoch={1}
              isWorkflowCurrent={() => true}
              projects={[{ local_project_id: "p", bundle_id: "x", display_name: "api-server", revision: 1, project_root: `${HOME}/code/api-server` }]}
              providers={PROVIDERS}
              profiles={PROFILES}
              storages={STORAGES}
              busy={false}
              initialSource="cloud"
              onCancel={() => note("cancel (page closed)")}
              onChoose={() => undefined}
              onOpenProject={(projectId) => note(`open project ${projectId}`)}
              onAddCloudAccount={() => note("add Mallard Cloud account (opens storage setup)")}
              onCloudProjectDone={(detail, fallback) => note(`done: ${detail.project.display_name}${fallback ? ` · fallback ${fallback.failure.reason}: ${fallback.failure.command}` : ""}`)}
            />
          </section>
        </main>
      </div>
      <div className="harness-log" aria-live="polite">{log.length ? log.join("\n") : "Backend calls appear here."}</div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<StrictMode><Harness /></StrictMode>);
