import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { renderToStaticMarkup } from "react-dom/server";
import { projectSyncApi } from "../../src/components/project-sync/api";
import CloudProjectSource, {
  browsedPath,
  clonesFrom,
  cloneFailureText,
  connectExistingFolder,
  defaultAgents,
  defaultProjectParent,
  destinationVerdict,
  joinFolder,
  mismatchAcknowledged,
  needsTick,
  type ConnectChoices,
} from "../../src/components/project-sync/CloudProjectSource";
import NewProjectFolderStep from "../../src/components/project-sync/NewProjectFolderStep";
import ProjectFolderField from "../../src/components/project-sync/ProjectFolderField";
import { compactFolderPath, readFolderInput } from "../../src/components/project-sync/projectFolder";
import RepositoryAddressPicker, {
  cloneSourceWord,
  isManagedCloudStorage,
  roleWord,
} from "../../src/components/project-sync/RepositoryAddressPicker";
import type {
  CloudProjectFolder,
  LocalProjectSummary,
  ManagedCloudRepository,
  ProjectDetail,
  ProjectSetupDraft,
  ProviderDescriptor,
  ProviderProfileSummary,
  StorageSummary,
} from "../../src/types";

/* New project › From Mallard Cloud: the account first, the repository by
   its address, the folder it gets, the agents, and one verb that gets the
   folder and connects it. A GitHub repository is cloned; anything else, or a
   clone that fails, is an empty folder. A folder this computer already has
   is connected as it is. Nothing is pulled. */

const HOME = "/home/you";
const compact = (path: string) => compactFolderPath(path, HOME);

const cloudStorage = (id: string, login: string): StorageSummary => ({
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
  settings: { service_origin: "https://mallard-ai.com", github_user_id: login === "octocat" ? "123" : "456", account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const personal = cloudStorage("storage-personal", "Rika321");
const work = cloudStorage("storage-work", "rika-acme");
const r2 = { ...cloudStorage("storage-r2", "r2"), provider: "r2", name: "Team R2", provider_label: "Cloudflare R2" } as StorageSummary;

const repository = (overrides: Partial<ManagedCloudRepository> = {}): ManagedCloudRepository => ({
  repository_id: "repo_portal",
  owner_login: "maya",
  name: "portal",
  address: "maya/portal",
  website_url: "https://mallard-ai.com/maya/portal",
  bundle_id: "b".repeat(32),
  repository_class: "git_project",
  visibility: "private",
  role: "member",
  can_push: true,
  repository_fingerprint: "f".repeat(64),
  git_remote: "differs",
  github_url: "https://github.com/acme/portal",
  revision: 12,
  updated_at: 1_787_298_240,
  resources: 9,
  registration_only: false,
  description: null, star_count: 0, viewer_starred: false,
  ...overrides,
});

const providers = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, has_cli: true, maintenance_actions: [] },
] as ProviderDescriptor[];
const profile = (profileId: string, provider: string, dir: string): ProviderProfileSummary => ({
  profile_id: profileId, provider, display_name: `~/${dir}`, path: `${HOME}/${dir}`, canonical_path: `${HOME}/${dir}`,
  revision: 1, created_at: 0, updated_at: 0, available: true, readable: true, writable: true, used_by_projects: [], error: null,
});
const profiles = [profile("p-codex", "codex", ".codex"), profile("p-claude", "claude", ".claude")];

test("cloud setup prefers each resolved default home among several profiles, regardless of name or order", () => {
  const defaults = profiles.map((item) => ({
    ...item,
    display_name: "Renamed profile",
    canonical_path: `/resolved/${item.provider}`,
  }));
  const custom = providers.map((item) => profile(`custom-${item.key}`, item.key, `${item.key}-custom`));
  const paths = { codex: "/resolved/codex", claude: "/resolved/claude" };
  for (const candidates of [[...custom, ...defaults], [...defaults, ...custom]]) {
    assert.deepEqual(defaultAgents(providers, candidates, paths, { enabled: {}, chosen: {} }), {
      enabled: { codex: true, claude: true },
      chosen: { codex: "p-codex", claude: "p-claude" },
    });
  }
});

test("cloud setup excludes unusable defaults and only selects an unambiguous remaining profile", () => {
  const paths = { codex: `${HOME}/.codex`, claude: `${HOME}/.claude` };
  const unavailable = profiles.map((item) => ({ ...item, available: false }));
  const unreadable = profiles.map((item) => ({ ...item, readable: false }));
  const custom = providers.map((item) => profile(`custom-${item.key}`, item.key, `${item.key}-custom`));
  for (const unusable of [unavailable, unreadable]) {
    assert.deepEqual(defaultAgents(providers, unusable, paths, { enabled: {}, chosen: {} }), {
      enabled: { codex: false, claude: false }, chosen: { codex: "", claude: "" },
    });
    assert.deepEqual(defaultAgents(providers, [...unusable, ...custom], paths, { enabled: {}, chosen: {} }).chosen, {
      codex: "custom-codex", claude: "custom-claude",
    });
  }
  assert.deepEqual(defaultAgents(providers, [...profiles, ...custom], {}, { enabled: {}, chosen: {} }).chosen, {
    codex: "", claude: "",
  });
});

test("cloud setup preserves manual selections, cleared selections, and disabled agents when defaults resolve", () => {
  const custom = providers.map((item) => profile(`custom-${item.key}`, item.key, `${item.key}-custom`));
  const choices = { enabled: { claude: false }, chosen: { codex: "custom-codex", claude: "" } };
  const resolvedPaths: Record<string, string>[] = [{}, { codex: `${HOME}/.codex`, claude: `${HOME}/.claude` }];
  for (const paths of resolvedPaths) {
    assert.deepEqual(defaultAgents(providers, [...profiles, ...custom], paths, choices), {
      enabled: { codex: true, claude: false }, chosen: choices.chosen,
    });
  }
});

function renderSource(storages: StorageSummary[], initialStorageId: string | null = null) {
  return renderToStaticMarkup(
    <CloudProjectSource
      doors={<div data-doors="" />}
      storages={storages}
      initialStorageId={initialStorageId}
      providers={providers}
      profiles={profiles}
      projects={[]}
      busy={false}
      workflowEpoch={1}
      isWorkflowCurrent={() => true}
      onCancel={() => undefined}
      onAddAccount={() => undefined}
      onOpenProject={() => undefined}
      onDone={() => undefined}
    />,
  );
}

const occupied = (overrides: Partial<Extract<CloudProjectFolder, { kind: "occupied" }>> = {}): CloudProjectFolder => ({
  kind: "occupied",
  suggestion: `${HOME}/code/portal-2`,
  local_fingerprint: null,
  git_remote: null,
  ...overrides,
});

test("the account comes first: the only one is checked, several wait for a pick, none offers set up", () => {
  const one = renderSource([personal, r2]);
  assert.match(one, /<strong>Storage<\/strong>/);
  assert.match(one, /class="setup-pill active"[^>]*>[\s\S]*?Mallard Cloud Rika321/);
  assert.doesNotMatch(one, /Team R2/, "only Mallard Cloud accounts are offered");
  assert.match(one, /<strong>Repository<\/strong>/);
  assert.match(one, /address or invitation code/i);
  // No repository yet: no Project or Agent band, and nothing to do.
  assert.doesNotMatch(one, /<strong>Project<\/strong>/);
  assert.match(one, /<button type="button" class="btn btn-primary" disabled="">Connect<\/button>/);

  const several = renderSource([personal, work]);
  assert.doesNotMatch(several, /setup-pill active/);
  assert.match(several, /Choose an account first\./);
  assert.doesNotMatch(several, /address or invitation code/i, "the Repository band waits for the account");

  const landed = renderSource([personal, work], "storage-work");
  assert.match(landed, /class="setup-pill active"[^>]*>[\s\S]*?Mallard Cloud rika-acme/);

  const none = renderSource([r2]);
  assert.match(none, /No Mallard Cloud account yet\./);
  assert.match(none, /Set up Mallard Cloud/);
});

test("the picker in clone mode compares no remote, offers no new repository, and says less", () => {
  const rows = [repository(), repository({ repository_id: "repo_notes", name: "notes", address: "Rika321/notes", repository_class: "non_git_project", role: "owner", github_url: null, repository_fingerprint: null })];
  const html = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={personal}
      projectName=""
      localFingerprint={null}
      listing={{ state: "ready", repositories: rows }}
      selection={null}
      rows="match-option"
      offerNew={false}
      mode="clone"
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.doesNotMatch(html, /New repository/);
  assert.doesNotMatch(html, /Git match|Git remote|different Git remote/);
  assert.doesNotMatch(html, /Paste the page address|No repository address is needed/);
  // One field for an address or a code; the verb is Look up until a code is typed.
  assert.match(html, /address or invitation code[\s\S]*?Look up<\/button>/i);
  // The scopes are tabs, Yours first; the field says a link is taken.
  assert.match(html, /role="tablist"[\s\S]*?Yours<small>2<\/small>[\s\S]*?Starred[\s\S]*?Public/);
  assert.match(html, /placeholder="Search, or paste a link, owner\/name or an invitation code"/);
  assert.doesNotMatch(html, /Join<\/button>/);
  assert.match(html, /maya\/portal[\s\S]*?Member/);
  assert.match(html, /Rika321\/notes[\s\S]*?Owner/);

  assert.equal(cloneSourceWord(repository()), "github.com/acme/portal");
  assert.equal(cloneSourceWord(repository({ github_url: null })), "GitHub URL missing");
  assert.equal(cloneSourceWord(repository({ repository_class: "non_git_project", github_url: null })), null);
  assert.equal(roleWord(repository({ role: null })), "Public");
  assert.equal(roleWord(repository({ role: "admin" })), "Admin");
});

test("the Folder line says where the project lands and how, in few words", () => {
  const path = `${HOME}/code/portal`;
  const input = readFolderInput("~/code/portal", HOME);
  const key = `${path}\n${"f".repeat(64)}\n0`;
  const verdict = (folder: CloudProjectFolder, canClone = true, repo = repository()) => destinationVerdict(
    input,
    { status: "done", key, probe: { folder, can_clone: canClone } },
    key,
    repo,
    compact,
    "new",
  );
  for (const kind of ["new", "empty"] as const) {
    assert.deepEqual(verdict({ kind }), { tone: "ok", message: "Clones github.com/acme/portal; keeps an empty Git repository if cloning fails." });
    assert.deepEqual(verdict({ kind }, false), { tone: "bad", message: "Install Git to create this Git project." });
    assert.deepEqual(verdict({ kind }, true, repository({ github_url: null })), { tone: "warn", message: "Creates an empty Git repository. The repository URL is missing." });
    assert.equal(verdict({ kind }, false, repository({ github_url: null }))?.tone, "bad");
  }
  assert.deepEqual(
    verdict({ kind: "new" }, true, repository({ repository_class: "non_git_project", github_url: null, repository_fingerprint: null })),
    { tone: "ok", message: "New, empty folder." },
  );
  assert.deepEqual(verdict({ kind: "checkout" }), { tone: "ok", message: "Already its checkout. Nothing to clone." });
  // A folder that holds files is two ways on: as it is, or a new one beside it.
  assert.deepEqual(verdict(occupied()), {
    tone: "warn",
    message: "Holds files.",
    actions: [
      { label: "Use it as it is", run: "use-as-is" },
      { label: "Use ~/code/portal-2", value: "~/code/portal-2" },
    ],
  });
  assert.deepEqual(verdict(occupied({ suggestion: "" })), {
    tone: "warn",
    message: "Holds files.",
    actions: [{ label: "Use it as it is", run: "use-as-is" }],
  });
  assert.deepEqual(verdict({ kind: "project", project_id: "project-portal" }), {
    tone: "bad",
    message: "Already a project here.",
    actions: [{ label: "Open it", run: "open-project" }],
  });
  assert.deepEqual(verdict({ kind: "refused", message: "That’s a file." }), { tone: "bad", message: "That’s a file." });
  // A stale answer, or one still coming, says nothing.
  assert.equal(destinationVerdict(input, { status: "checking", key }, key, repository(), compact, "new"), null);
  assert.equal(destinationVerdict(input, { status: "done", key: "other", probe: { folder: { kind: "new" }, can_clone: true } }, key, repository(), compact, "new"), null);
  assert.deepEqual(destinationVerdict(readFolderInput("portal", HOME), { status: "idle" }, null, repository(), compact, "new"), { tone: "bad", message: "Enter a full path." });

  assert.equal(clonesFrom(repository(), true), "github.com/acme/portal");
  assert.equal(clonesFrom(repository(), false), null);
  assert.equal(clonesFrom(repository({ github_url: null }), true), null);
  assert.equal(cloneFailureText("refused"), "GitHub said no.");
  assert.equal(cloneFailureText("missing_source"), "The repository URL is missing.");
});

test("an existing folder is used as it is, and says when setup's tick is needed", () => {
  const path = `${HOME}/code/portal`;
  const input = readFolderInput("~/code/portal", HOME);
  const key = `${path}\n${"f".repeat(64)}\n0`;
  const plain = repository({ name: "field-notes", repository_class: "non_git_project", github_url: null, repository_fingerprint: null });
  const verdict = (folder: CloudProjectFolder, repo = repository()) => destinationVerdict(
    input,
    { status: "done", key, probe: { folder, can_clone: true } },
    key,
    repo,
    compact,
    "existing",
  );
  assert.deepEqual(verdict({ kind: "checkout" }), { tone: "ok", message: "Already its checkout. Nothing to clone." });
  // Nothing is cloned into an empty folder, and nothing is made.
  assert.deepEqual(verdict({ kind: "empty" }), { tone: "ok", message: "Empty. Nothing is cloned." });
  assert.deepEqual(verdict({ kind: "new" }), { tone: "bad", message: "No folder here." });
  assert.deepEqual(verdict(occupied(), plain), { tone: "ok", message: "Used as it is." });
  assert.deepEqual(verdict(occupied()), { tone: "warn", message: "No Git remote to check." });
  assert.deepEqual(
    verdict(occupied({ local_fingerprint: "e".repeat(64), git_remote: "github.com/maya/portal" })),
    { tone: "warn", message: "Git remote: github.com/maya/portal." },
  );
  assert.deepEqual(
    verdict(occupied({ local_fingerprint: "e".repeat(64), git_remote: "github.com/maya/field-notes" }), plain),
    { tone: "warn", message: "A Git checkout; field-notes has no Git." },
  );
  assert.deepEqual(verdict({ kind: "project", project_id: "project-portal" }), {
    tone: "bad",
    message: "Already a project here.",
    actions: [{ label: "Open it", run: "open-project" }],
  });

  // The tick is setup's rule, and only for a folder that holds files.
  assert.equal(needsTick("existing", occupied(), repository()), true);
  assert.equal(needsTick("existing", occupied({ local_fingerprint: "f".repeat(64) }), repository()), false);
  assert.equal(needsTick("existing", occupied(), plain), false);
  assert.equal(needsTick("existing", { kind: "empty" }, repository()), false);
  assert.equal(needsTick("new", occupied(), repository()), false);
  // An empty folder has nothing to verify; files are acknowledged only by the tick.
  assert.equal(mismatchAcknowledged("empty", false), true);
  assert.equal(mismatchAcknowledged("occupied", false), false);
  assert.equal(mismatchAcknowledged("occupied", true), true);
  assert.equal(mismatchAcknowledged("checkout", true), false);

  // Browse picks the parent for a new folder and the folder itself otherwise.
  assert.equal(browsedPath("new", `${HOME}/Desktop/project`, "HorrorGame"), `${HOME}/Desktop/project/HorrorGame`);
  assert.equal(browsedPath("existing", `${HOME}/Desktop/project/demo`, "HorrorGame"), `${HOME}/Desktop/project/demo`);
});

test("connecting an existing folder asks again, never clones, and sends the person's tick", async () => {
  const calls: string[] = [];
  let sent: ProjectSetupDraft | null = null;
  let answer: CloudProjectFolder = occupied();
  const draftFor = (root: string): ProjectSetupDraft => ({
    schema: 1, draft_id: "draft-1", local_project_id: "project-1", new_bundle_id: "a".repeat(32), project_root: root,
    canonical_project_root: root, display_name: "demo", repository_fingerprint: null, profiles: {}, storage: null,
    repository: { kind: "new" }, revision: 0, created_at: 0, updated_at: 0,
  });
  const detail = { project: { local_project_id: "project-1", display_name: "portal" }, links: [], binding: null, materializations: [] } as unknown as ProjectDetail;
  const api = projectSyncApi as unknown as Record<string, unknown>;
  const original = { ...api };
  Object.assign(api, {
    probeCloudProjectFolder: async () => {
      calls.push("probe");
      return { folder: answer, can_clone: true };
    },
    getCloudProjectFolder: async () => {
      calls.push("get");
      throw new Error("an existing folder is never cloned or made");
    },
    createSetupDraft: async (root: string) => {
      calls.push(`draft ${root}`);
      return { draft: draftFor(root), resumed: false };
    },
    updateSetupDraft: async (draft: ProjectSetupDraft) => {
      calls.push("update");
      sent = draft;
      return { ...draft, revision: draft.revision + 1 };
    },
    finalizeProjectSetup: async () => {
      calls.push("finalize");
      return detail;
    },
  });
  const path = `${HOME}/Desktop/project/demo`;
  const choices = (repo: ManagedCloudRepository): ConnectChoices => ({
    storageId: "storage-work",
    bundleId: "b".repeat(32),
    repository: repo,
    profiles: { codex: { kind: "existing", profile_id: "p-codex" } },
  });
  const acknowledged = () => {
    const repository = (sent as ProjectSetupDraft | null)?.repository;
    return repository?.kind === "existing" ? repository.mismatch_acknowledged : undefined;
  };
  const plain = repository({ repository_class: "non_git_project", github_url: null, repository_fingerprint: null });
  try {
    // No Git either side: nothing to verify, nothing acknowledged.
    assert.equal(await connectExistingFolder(path, occupied(), false, choices(plain)), detail);
    assert.deepEqual(calls, ["probe", `draft ${path}`, "update", "finalize"], "the typed path, and never the folder step");
    assert.equal(acknowledged(), false);

    // A Git repository and a folder with no remote: the tick, or nothing.
    calls.length = 0;
    sent = null;
    assert.equal(await connectExistingFolder(path, occupied(), false, choices(repository())), null);
    assert.deepEqual(calls, ["probe"], "an unticked folder that can't be verified is not connected");
    assert.equal(await connectExistingFolder(path, occupied(), true, choices(repository())), detail);
    assert.equal(acknowledged(), true);

    // An empty folder has nothing to verify; its checkout matches.
    answer = { kind: "empty" };
    assert.equal(await connectExistingFolder(path, { kind: "empty" }, false, choices(repository())), detail);
    assert.equal(acknowledged(), true);
    answer = { kind: "checkout" };
    assert.equal(await connectExistingFolder(path, { kind: "checkout" }, false, choices(repository())), detail);
    assert.equal(acknowledged(), false);

    // The folder changed since the line was drawn: nothing is written.
    calls.length = 0;
    answer = occupied();
    assert.equal(await connectExistingFolder(path, { kind: "empty" }, false, choices(repository())), null);
    assert.deepEqual(calls, ["probe"]);
    // Or it now needs a tick it was never given.
    calls.length = 0;
    answer = occupied({ local_fingerprint: "e".repeat(64), git_remote: "github.com/maya/notes" });
    assert.equal(await connectExistingFolder(path, occupied(), false, choices(plain)), null);
    assert.deepEqual(calls, ["probe"]);
    assert.ok(!calls.includes("get"));
  } finally {
    Object.assign(api, original);
  }
});

test("the Folder field takes a switch in its label's place and several ways on", () => {
  const html = renderToStaticMarkup(
    <ProjectFolderField
      value="~/code/portal"
      verdict={{
        tone: "warn",
        message: "Holds files.",
        actions: [
          { label: "Use it as it is", run: "use-as-is" },
          { label: "Use ~/code/portal-2", value: "~/code/portal-2" },
        ],
      }}
      disabled={false}
      heading={<div data-switch="" />}
      placeholder="~/code/my-app"
      onChange={() => undefined}
      onEnter={() => undefined}
      onBrowsed={() => undefined}
    />,
  );
  assert.match(html, /<label[^>]*class="sync-visually-hidden"[^>]*>Folder<\/label><div data-switch="">/);
  assert.match(
    html,
    /Holds files\.<button[^>]*class="btn-link"[^>]*>Use it as it is<\/button><span aria-hidden="true">·<\/span><button[^>]*class="btn-link"[^>]*>Use ~\/code\/portal-2<\/button>/,
  );
});

test("a new project's folder goes beside the ones already here, named for the repository", () => {
  const at = (root: string) => ({ project_root: root }) as LocalProjectSummary;
  assert.equal(defaultProjectParent([], HOME), HOME);
  assert.equal(
    defaultProjectParent([at(`${HOME}/code/a`), at(`${HOME}/code/b`), at(`${HOME}/notes/c`)], HOME),
    `${HOME}/code`,
  );
  assert.equal(joinFolder(`${HOME}/code`, "portal"), `${HOME}/code/portal`);
  assert.equal(joinFolder(`${HOME}/code/`, "portal"), `${HOME}/code/portal`);
  assert.equal(joinFolder("C:\\work", "portal"), "C:\\work\\portal");
});

test("New project opens on From Mallard Cloud when asked, on the account it names", () => {
  const html = renderToStaticMarkup(
    <NewProjectFolderStep
      workflowEpoch={1}
      isWorkflowCurrent={() => true}
      projects={[]}
      providers={providers}
      profiles={profiles}
      storages={[personal, work]}
      busy={false}
      initialSource="cloud"
      initialStorageId="storage-work"
      onCancel={() => undefined}
      onChoose={() => undefined}
      onOpenProject={() => undefined}
      onAddCloudAccount={() => undefined}
      onCloudProjectDone={() => undefined}
    />,
  );
  assert.match(html, /aria-checked="true"[^>]*>[\s\S]*?<strong>From Mallard Cloud<\/strong>/);
  assert.doesNotMatch(html, /<strong>Folder<\/strong>/, "the folder door's band is not drawn");
  assert.match(html, /class="setup-pill active"[^>]*>[\s\S]*?Mallard Cloud rika-acme/);
});

for (const scenario of [
  "Storages", "New project", "another storage", "reconnect", "reset", "R2", "Catalog", "Catalog reset",
  // A `mallard://` link set aside while Mallard Cloud is signed in to.
  "a waiting listing link", "a waiting repository link", "a waiting link, reconnect",
] as const) {
  test(`storage setup returns to its source: ${scenario}`, async () => {
    const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
    const body = (start: string, end: string) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
    const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
    const handlers = transformSync([
      body("  const createStorageDraft = async", "  /** Reopen a persisted storage setup draft"),
      body("  const commitStorageDraft = async", "  /** Rename a settled storage"),
      body("  const openStorageWorkspace = async", "  const openStorageIndex = async"),
    ].join("\n"), { loader: "ts" }).code;
    const noop = () => {};
    const destinations: unknown[] = [];
    const errors: unknown[] = [];
    let draftStorage = scenario === "R2" ? r2 : personal;
    const committed = scenario === "another storage" || scenario === "reset" || scenario === "Catalog reset" ? work : scenario === "R2" ? r2 : personal;
    const cloudProjectSetupStorageRef = { current: null as string | null };
    const waitingLink = scenario === "a waiting repository link" ? "repository" : scenario.startsWith("a waiting") ? "listing" : null;
    let linksResumed = 0;
    const context = {
      appLink: {
        awaitsAccount: () => waitingLink !== null,
        // The listing's link goes straight to its page; a repository's opens its sheet.
        resume: () => { linksResumed += 1; return waitingLink === "listing"; },
      },
      appLinkLandingRef: { current: null as string | null },
      hostWorkflowEpoch: { current: 1 }, cloudProjectSetupStorageRef,
      cloudLandingRef: { current: null as string | null },
      catalogStorageSetupRef: { current: null as { scopeId: string; draftId: string | null } | null },
      catalogStorageLandingRef: { current: null as { scopeId: string; storageId: string } | null },
      setCatalogPreferredStorage: noop,
      openCatalogScope: (scopeId: string) => { destinations.push({ scopeId }); },
      PROJECT_SYNC_SCHEMA: 1, isManagedCloudStorage, restorePlan: null,
      saveStorageDraftBeforeNavigation: async () => true,
      storageDraftSaveQueue: { enqueue: (action: () => Promise<unknown>) => action() },
      projectSyncApi: {
        createStorageSetupDraft: async () => ({ draft_id: `draft-${draftStorage.storage_id}`, storage_id: draftStorage.storage_id }),
        commitStorageSetupDraft: async () => ({ storage_id: committed.storage_id, config_revision: 7, replaced: scenario.endsWith("reconnect") }),
        getConfig: async () => ({ schema: 1, revision: 7, storages: [personal, work, r2] }),
      },
      refreshStorageSetupDrafts: async () => {}, openStorageDraftForm: noop,
      adoptConfigSnapshot: noop, notify: noop, setBusy: noop,
      setError: (reason: unknown) => { if (reason) errors.push(reason); },
      errorMessage: (reason: unknown) => String(reason),
      beginAddProject: async (options: unknown) => { destinations.push(options); },
      closeProjectSubview: noop, setSetupView: noop, setAddingCatalogScope: noop,
      setCatalogIndexOpen: noop, setCatalogScopeId: noop, setStorageIndexOpen: noop,
      setStorageProviderPickerOpen: noop, setSettingsOpen: noop, setPendingPush: noop,
      setPushError: noop, closePullReview: noop,
      setStorageEditorRequest: (update: (previous: null) => unknown) => { destinations.push(update(null)); },
    };
    const create = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handlers}; return { createStorageDraft, commitStorageDraft, openStorageWorkspace };`);
    const flow = create(context) as {
      createStorageDraft: (provider: string, name?: string, returnToNewProject?: boolean, keepCatalogReturn?: boolean) => Promise<void>;
      commitStorageDraft: (draftId: string, revision: number) => Promise<unknown>;
      openStorageWorkspace: (storageId: string) => Promise<void>;
    };
    if (scenario.startsWith("Catalog")) context.catalogStorageSetupRef.current = { scopeId: "pets", draftId: null };
    if (scenario === "Storages" || scenario.startsWith("Catalog") || waitingLink) await flow.createStorageDraft("managed_cloud");
    else await flow.createStorageDraft(draftStorage.provider, undefined, true);
    if (scenario === "reset" || scenario === "Catalog reset") {
      const activeStorageSettingsId = draftStorage.storage_id;
      draftStorage = work;
      await flow.createStorageDraft("managed_cloud", undefined,
        cloudProjectSetupStorageRef.current !== null && cloudProjectSetupStorageRef.current === activeStorageSettingsId,
        scenario === "Catalog reset");
    }
    await flow.commitStorageDraft(`draft-${committed.storage_id}`, 1);
    await flow.openStorageWorkspace(committed.storage_id);
    assert.deepEqual(errors, []);
    const returnsToProject = scenario === "New project" || scenario === "reset";
    // The listing's link went on to its own page: this storage's does not open.
    assert.deepEqual(destinations, scenario === "a waiting listing link" ? [] : [scenario.startsWith("Catalog") ? { scopeId: "pets" } : returnsToProject
      ? { source: "cloud", storageId: committed.storage_id }
      : { mode: "open", storageId: committed.storage_id, requestId: 1 }]);
    // The redirect is consumed. Opening this storage again opens its own page.
    destinations.length = 0;
    await flow.openStorageWorkspace(committed.storage_id);
    assert.deepEqual(destinations, [{ mode: "open", storageId: committed.storage_id, requestId: 1 }]);
    // A link set aside for a Mallard Cloud account is let through once, when
    // the new account's storage opens: not on a reconnect, and never when no
    // link is waiting.
    assert.equal(linksResumed, scenario === "a waiting listing link" || scenario === "a waiting repository link" ? 1 : 0);
  });
}
