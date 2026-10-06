import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import AppLinkSheet from "../../src/components/app-link/AppLinkSheet";
import {
  catalogSeers,
  INVALID_LINK,
  linkVerdict,
  linkWaits,
  listingPage,
  listingVerdict,
  serviceHost,
  unanswered,
  type LinkContext,
  type LinkVerdict,
} from "../../src/components/app-link/appLink";
import NewProjectFolderStep from "../../src/components/project-sync/NewProjectFolderStep";
import RepositoryAddressPicker from "../../src/components/project-sync/RepositoryAddressPicker";
import AddCatalogScopeWorkspace from "../../src/components/storage-catalog/AddCatalogScopeWorkspace";
import type { CatalogCapabilityDescriptor } from "../../src/components/storage-catalog/types";
import type {
  AppLinkAccount,
  AppLinkAnswer,
  AppLinkRepository,
  AppLinkSight,
  ProviderDescriptor,
  StorageSummary,
} from "../../src/types";

/* Open in Mallard: a `mallard://` link from the website names a repository's
   page, or one session's of it. The backend reads it and asks each account
   on its host; this side joins the answer with what is on this computer,
   shows one sheet, and goes nowhere until the sheet's verb is pressed. A
   link never presses a verb: a session's opens its Pull review, and Pull
   there is the press that writes. */

const HOST = "mallard-ai.com";

const project = (overrides: Partial<AppLinkRepository> = {}): AppLinkRepository => ({
  repository_id: "repo_portal",
  address: "maya/portal",
  repository_class: "git_project",
  bundle_id: "b".repeat(32),
  catalog: null,
  visibility: "public",
  role: null,
  ...overrides,
});
const PET = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const item = (overrides: Partial<AppLinkRepository> = {}): AppLinkRepository => ({
  repository_id: "repo_mochi",
  address: "maya/Mochi",
  repository_class: "catalog",
  bundle_id: null,
  catalog: { ...PET, resource_id: "mochi" },
  visibility: "public",
  role: null,
  ...overrides,
});
const found = (repository: AppLinkRepository): AppLinkSight => ({ kind: "found", repository });
const account = (storageId: string, login: string, sight: AppLinkSight): AppLinkAccount => ({
  storage_id: storageId,
  storage_name: `Mallard Cloud ${login}`,
  account_login: login,
  sight,
});
const answer = (accounts: AppLinkAccount[], overrides: Partial<AppLinkAnswer> = {}): AppLinkAnswer => ({
  host: HOST,
  address: "maya/portal",
  accounts,
  can_set_up: true,
  ...overrides,
});

const PETS: CatalogCapabilityDescriptor = {
  ...PET,
  provider_label: "Codex",
  label: "Pets v2",
  singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" },
  capture: "directory_tree",
  apply: "automatic",
  targets: [],
};
const PETS_ROW = {
  scope: { scope_id: "scope-pets", profile_id: "profile-codex", ...PET, revision: 1, created_at: 0, updated_at: 0 },
  label: "Pets v2",
  profile: { profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex", configured_home: "~/.codex", revision: 1, created_at: 0, updated_at: 0 },
};
const context = (overrides: Partial<LinkContext> = {}): LinkContext => ({
  links: [],
  projects: [],
  scopes: [],
  capabilities: [PETS],
  places: {},
  ...overrides,
});

function landing(verdict: LinkVerdict) {
  assert.equal(verdict.kind, "landing");
  if (verdict.kind !== "landing") throw new Error("unreachable");
  return verdict;
}
function stop(verdict: LinkVerdict) {
  assert.equal(verdict.kind, "stop");
  if (verdict.kind !== "stop") throw new Error("unreachable");
  return verdict;
}

test("a project that is not here lands on New project, the one account that sees it ticked", () => {
  const one = landing(linkVerdict(answer([account("s-personal", "Rika321", found(project()))]), context()));
  assert.deepEqual(one.destination, { kind: "new_project", storageId: "s-personal", address: "maya/portal" });
  assert.equal(one.verb, "Set up project…");
  assert.deepEqual(one.found, {
    address: "maya/portal",
    mark: "git-folder",
    facts: [{ icon: "cloud", word: "Public" }],
    here: { place: "computer", present: false, word: "Not here", sentence: "Not on this computer." },
  });

  // Two accounts on the host, one of which sees it: that one is ticked.
  const oneOfTwo = landing(linkVerdict(answer([
    account("s-personal", "Rika321", { kind: "not_found" }),
    account("s-work", "rika-acme", found(project({ role: "member", visibility: "private" }))),
  ]), context()));
  assert.deepEqual(oneOfTwo.destination, { kind: "new_project", storageId: "s-work", address: "maya/portal" });
  assert.deepEqual(oneOfTwo.found.facts, [{ icon: "lock", word: "Private" }, { icon: "user", word: "Member" }]);

  // Both see it: none is ticked, the page's own rule for several accounts.
  const both = landing(linkVerdict(answer([
    account("s-personal", "Rika321", found(project())),
    account("s-work", "rika-acme", found(project())),
  ]), context()));
  assert.deepEqual(both.destination, { kind: "new_project", storageId: null, address: "maya/portal" });

  // A project without Git wears the folder mark.
  assert.equal(
    landing(linkVerdict(answer([account("s", "Rika321", found(project({ repository_class: "non_git_project" })))]), context())).found.mark,
    "folder",
  );
});

test("a project connected here opens, by the saved repository or by its bundle", () => {
  const projects = [
    { local_project_id: "p-other", display_name: "other", local_alias: null },
    { local_project_id: "p-portal", display_name: "portal", local_alias: "Portal (work)" },
    { local_project_id: "p-portal-2", display_name: "portal-2", local_alias: null },
  ];
  const seen = answer([account("s-personal", "Rika321", found(project({ role: "owner", visibility: "private" })))]);

  const byPin = landing(linkVerdict(seen, context({
    projects,
    links: [
      { local_project_id: "p-portal-2", storage_id: "s-personal", bundle_id: "x".repeat(32), managed_cloud_repository_id: "repo_portal" },
      { local_project_id: "p-portal", storage_id: "s-personal", bundle_id: "y".repeat(32), managed_cloud_repository_id: "repo_portal" },
    ],
  })));
  assert.deepEqual(byPin.destination, { kind: "project", projectId: "p-portal", storageId: "s-personal" }, "the first in the project list");
  assert.equal(byPin.verb, "Open project");
  assert.deepEqual(byPin.found.here, { place: "computer", present: true, word: "Portal (work)", sentence: "On this computer: Portal (work)" });
  assert.deepEqual(byPin.found.facts, [{ icon: "lock", word: "Private" }, { icon: "user", word: "Owner" }]);

  const byBundle = landing(linkVerdict(seen, context({
    projects,
    links: [{ local_project_id: "p-other", storage_id: "s-personal", bundle_id: "b".repeat(32), managed_cloud_repository_id: null }],
  })));
  assert.deepEqual(byBundle.destination, { kind: "project", projectId: "p-other", storageId: "s-personal" });

  // The same repository on a storage that is not on the link's host is another service's.
  const elsewhere = landing(linkVerdict(seen, context({
    projects,
    links: [{ local_project_id: "p-portal", storage_id: "s-staging", bundle_id: "b".repeat(32), managed_cloud_repository_id: "repo_portal" }],
  })));
  assert.equal(elsewhere.destination.kind, "new_project");

  // Connected through an account that can no longer see it: still this computer's project.
  const throughAnother = landing(linkVerdict(answer([
    account("s-personal", "Rika321", { kind: "not_found" }),
    account("s-work", "rika-acme", found(project())),
  ]), context({
    projects,
    links: [{ local_project_id: "p-portal", storage_id: "s-personal", bundle_id: "b".repeat(32), managed_cloud_repository_id: "repo_portal" }],
  })));
  assert.deepEqual(throughAnother.destination, { kind: "project", projectId: "p-portal", storageId: "s-personal" });
});

test("a session's link opens that session's Pull review in the project that holds its repository", () => {
  const SESSION = "codex:conversation:cloudthread-0a1b2c3d";
  const projects = [{ local_project_id: "p-portal", display_name: "portal", local_alias: null }];
  const links = [{ local_project_id: "p-portal", storage_id: "s-personal", bundle_id: "b".repeat(32), managed_cloud_repository_id: "repo_portal" }];
  const seen = answer([account("s-personal", "Rika321", found(project({ role: "owner", visibility: "private" })))]);

  // The project is here: the sheet names the repository and that the link
  // names one session of it. No storage is read for the session's own name.
  const here = landing(linkVerdict(seen, context({ projects, links }), SESSION));
  assert.equal(here.verb, "Pull session…");
  assert.deepEqual(here.destination, { kind: "pull_session", projectId: "p-portal", storageId: "s-personal", resourceId: SESSION });
  assert.deepEqual(here.found.facts, [{ icon: "lock", word: "Private" }, { icon: "user", word: "Owner" }, { icon: "message", word: "1 session" }]);
  assert.equal(here.found.here.word, "portal");
  assert.equal(here.found.here.present, true);

  // The same repository without a session is the repository link it was.
  const repository = landing(linkVerdict(seen, context({ projects, links })));
  assert.equal(repository.verb, "Open project");
  assert.equal(repository.found.facts.some((fact) => fact.icon === "message"), false);

  // The project is not here: setup, as a repository link; the session is
  // not carried through it, and the sheet says to use the link again.
  const away = landing(linkVerdict(seen, context(), SESSION));
  assert.equal(away.verb, "Set up project…");
  assert.deepEqual(away.destination, { kind: "new_project", storageId: "s-personal", address: "maya/portal" });
  assert.equal(away.found.facts.at(-1)?.word, "1 session");
  assert.deepEqual(away.found.here, {
    place: "computer", present: false, word: "Not here", sentence: "Not on this computer. Set up the project, then use the link again.",
  });

  // The stops are the repository link's.
  assert.equal(stop(linkVerdict(answer([]), context(), SESSION)).action, "set_up");
  assert.equal(stop(linkVerdict(answer([account("s", "guest", { kind: "not_found" })]), context(), SESSION)).message, "guest can’t see maya/portal.");
  // A Catalog item has no session pages: the website writes no such link.
  assert.equal(linkVerdict(answer([account("s-personal", "Rika321", found(item()))]), context({ scopes: [PETS_ROW] }), SESSION), INVALID_LINK);

  // The sheet's facts gain the session, and its verb is the review's opener.
  const sheet = renderToStaticMarkup(
    <AppLinkSheet subject="maya/portal" state={{ phase: "answered", verdict: here }} onVerb={() => undefined} onClose={() => undefined} />,
  );
  assert.match(sheet, /<span class="app-link-fact">.*?Owner<\/span><span class="app-link-fact">.*?1 session<\/span>/s);
  assert.match(sheet, /<span class="app-link-here present" title="On this computer: portal">.*?portal<\/span>/s);
  assert.match(sheet, /<button[^>]*btn-primary[^>]*>Pull session…<\/button>/);
  assert.doesNotMatch(sheet, /cloudthread/, "the session's id is never shown");
});

test("a Catalog item's link pulls it where it is saved, or lands on the page that adds it", () => {
  const seenPet = (role: string | null = null) => answer(
    [account("s-personal", "Rika321", found(item({ role })))],
    { address: "maya/Mochi" },
  );
  assert.deepEqual(catalogSeers(seenPet()), [{ storageId: "s-personal", repositoryId: "repo_mochi" }]);
  assert.deepEqual(catalogSeers(answer([account("s", "Rika321", found(project()))])), [], "a project asks the Catalog nothing");

  // Saved by a row: the item's page there.
  const saved = landing(linkVerdict(seenPet(), context({
    scopes: [PETS_ROW],
    places: { "s-personal": { scope_id: "scope-pets", storage_id: "s-personal", resource_id: "mochi-local" } },
  })));
  assert.deepEqual(saved.destination, { kind: "catalog_item", scopeId: "scope-pets", storageId: "s-personal", resourceId: "mochi-local" });
  // The item's page is its repository's, so its link pulls it: the verb
  // opens the page on the review its Pull button opens.
  assert.equal(saved.verb, "Pull pet v2…");
  assert.deepEqual(saved.found.facts, [{ icon: "cloud", word: "Public" }]);
  assert.deepEqual(saved.found.here, { place: "catalog", present: true, word: "Pets v2 · Default Codex", sentence: "In your Catalog: Pets v2 · Default Codex" });
  assert.equal(saved.found.mark, "catalog-pet-v2");

  // Someone else's, not saved: Add Catalog resource › From Mallard Cloud.
  const add = landing(linkVerdict(seenPet(), context({ scopes: [PETS_ROW], places: { "s-personal": null } })));
  assert.deepEqual(add.destination, { kind: "add_catalog_item", storageId: "s-personal", address: "maya/Mochi" });
  assert.equal(add.verb, "Add to Catalog…");
  assert.deepEqual(add.found.here, { place: "catalog", present: false, word: "Not in Catalog", sentence: "Not in your Catalog." });

  // The account's own, not saved: every row of its kind lists it, and Add
  // from Mallard Cloud is for someone else's.
  const ownListed = landing(linkVerdict(seenPet("owner"), context({ scopes: [PETS_ROW] })));
  assert.deepEqual(ownListed.destination, { kind: "catalog_item", scopeId: "scope-pets", storageId: "s-personal", resourceId: "mochi" });
  assert.equal(ownListed.verb, "Pull pet v2…");
  // No row of its kind: Add makes one, and the row then opens on the item.
  const ownNoRow = landing(linkVerdict(seenPet("owner"), context()));
  assert.deepEqual(ownNoRow.destination, { kind: "add_catalog_row", choiceKey: "desktop_pet/openai.codex-pet@2", storageId: "s-personal", resourceId: "mochi" });
  assert.equal(ownNoRow.verb, "Add to Catalog…");
  assert.deepEqual(ownNoRow.found.here, { place: "catalog", present: false, word: "No Pets v2 row", sentence: "No Pets v2 row on this computer." });

  // A kind this build does not register goes nowhere.
  const unknown = stop(linkVerdict(seenPet(), context({ capabilities: [] })));
  assert.equal(unknown.message, "maya/Mochi is a kind this app can’t open.");
  assert.equal(unknown.title, "Can’t open this kind");
  assert.equal(unknown.action, null);
});

test("a link that cannot land says why, and offers a verb only where there is a way forward", () => {
  // No account on the host. This build's service: setup. Any other host: nothing.
  const signIn = stop(linkVerdict(answer([]), context()));
  assert.deepEqual(signIn, {
    kind: "stop", severity: "info", title: "Sign in first", icon: "user", message: "Sign in to Mallard Cloud to open maya/portal.", action: "set_up",
  });
  const otherHost = stop(linkVerdict(answer([], { host: "staging.mallard-ai.com", can_set_up: false }), context()));
  assert.deepEqual(otherHost, {
    kind: "stop", severity: "warning", title: "No account on staging.mallard-ai.com", icon: "user",
    message: "No account here for staging.mallard-ai.com.", action: null,
  });

  // Nobody can see it: the picker's words for one account.
  const unseen = stop(linkVerdict(answer([account("s", "Rika321", { kind: "not_found" })]), context()));
  assert.equal(unseen.title, "No access");
  assert.equal(unseen.message, "Rika321 can’t see maya/portal.");
  assert.equal(
    stop(linkVerdict(answer([
      account("s", "Rika321", { kind: "not_found" }),
      account("t", "rika-acme", { kind: "not_found" }),
    ]), context())).message,
    "No account here can see maya/portal.",
  );

  // An account that could not be asked may be the one that sees it: say so, and ask again.
  const failed = stop(linkVerdict(answer([
    account("s", "Rika321", { kind: "not_found" }),
    account("t", "rika-acme", { kind: "failed", message: "Mallard Cloud rejected the saved sign-in. Reconnect Mallard Cloud in storage settings." }),
  ]), context()));
  assert.deepEqual(failed, unanswered("Mallard Cloud rejected the saved sign-in. Reconnect Mallard Cloud in storage settings."));
  assert.equal(failed.severity, "danger");
  assert.equal(failed.action, "retry");
  assert.equal(failed.title, "Mallard Cloud didn’t answer");
  // …but not when another account does see it.
  assert.equal(linkVerdict(answer([
    account("s", "Rika321", { kind: "failed", message: "unreachable" }),
    account("t", "rika-acme", found(project())),
  ]), context()).kind, "landing");

  assert.deepEqual(stop(linkVerdict(answer([account("s", "Rika321", found(project({ bundle_id: null })))]), context())), INVALID_LINK);
  assert.equal(INVALID_LINK.message, "Mallard can’t open that link.");
  assert.equal(INVALID_LINK.title, "Broken link");
});

test("the listing's link goes straight to Projects, or to the Catalog for a Catalog kind", () => {
  const listing = (listing_kind: Parameters<typeof listingPage>[0]["listing_kind"], overrides: { host?: string; can_set_up?: boolean } = {}) => ({
    kind: "listing" as const, host: HOST, listing_kind, can_set_up: true, ...overrides,
  });
  // Share yours: nothing to ask the service and nothing to choose between, so no sheet.
  assert.deepEqual(listingVerdict(listing(null), [HOST]), { kind: "direct", destination: { kind: "projects" } });
  assert.deepEqual(listingVerdict(listing("project"), [HOST]), { kind: "direct", destination: { kind: "projects" } });
  for (const kind of ["agent_skill", "plugin_install_intent", "desktop_pet"] as const) {
    assert.deepEqual(listingVerdict(listing(kind), ["localhost:8787", HOST]), { kind: "direct", destination: { kind: "catalog" } }, kind);
    assert.equal(listingPage(listing(kind)), "Catalog");
  }
  assert.equal(listingPage(listing(null)), "Projects");
  assert.equal(listingPage(listing("project")), "Projects");

  // No storage signs in to the link's host: the sheet, with setup only for this build's service.
  assert.deepEqual(listingVerdict(listing("desktop_pet"), []), {
    kind: "stop", severity: "info", title: "Sign in first", icon: "user", message: "Sign in to Mallard Cloud to share.", action: "set_up",
  });
  assert.deepEqual(listingVerdict(listing(null, { host: "staging.mallard-ai.com", can_set_up: false }), [HOST]), {
    kind: "stop", severity: "warning", title: "No account on staging.mallard-ai.com", icon: "user",
    message: "No account here for staging.mallard-ai.com.", action: null,
  });

  // A storage's host, spelled as the backend spells a link's.
  assert.equal(serviceHost("https://mallard-ai.com"), "mallard-ai.com");
  assert.equal(serviceHost("https://WWW.Mallard-AI.com/"), "mallard-ai.com");
  assert.equal(serviceHost("http://localhost:8787"), "localhost:8787");
  assert.equal(serviceHost("https://mallard-ai.com:443"), "mallard-ai.com", "a default port is no port");
  assert.equal(serviceHost("not an origin"), null);
  assert.equal(serviceHost(""), null);
});

test("a link waits only while the shell is in the middle of something", () => {
  const free = { busy: false, reviewOpen: false, dialogOpen: false };
  assert.equal(linkWaits(free), false);
  assert.equal(linkWaits({ ...free, busy: true }), true);
  assert.equal(linkWaits({ ...free, reviewOpen: true }), true);
  assert.equal(linkWaits({ ...free, dialogOpen: true }), true);
});

test("the sheet titles itself with what the link names, says the rest in two short lines, and offers one verb", () => {
  const sheet = (state: Parameters<typeof AppLinkSheet>[0]["state"], subject: string | null = "maya/portal") => renderToStaticMarkup(
    <AppLinkSheet subject={subject} state={state} onVerb={() => undefined} onClose={() => undefined} />,
  );
  const buttons = (html: string) => [...html.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map((match) => match[1].replace(/<[^>]+>/g, ""));

  // Looking up: titled with the link's subject at once; the lines hold their place.
  const looking = sheet({ phase: "looking" });
  assert.match(looking, /class="sync-modal app-link-sheet"/);
  assert.match(looking, /<h1[^>]*>maya\/portal<\/h1>/);
  assert.match(looking, /Looking up maya\/portal…/);
  assert.equal((looking.match(/class="app-link-placeholder/g) ?? []).length, 3, "where the answer lands");
  assert.deepEqual(buttons(looking), ["", "Cancel", "Open…"]);
  assert.match(looking, /<button[^>]*btn-primary[^>]*disabled=""[^>]*>Open…<\/button>/, "nothing to press until the service answers");

  const verdict = linkVerdict(answer([account("s-personal", "Rika321", found(project()))]), context());
  const answered = sheet({ phase: "answered", verdict });
  assert.match(answered, /<h1[^>]*>maya\/portal<\/h1>/);
  assert.match(answered, /<span class="app-link-fact">.*?Public<\/span>/s);
  assert.match(answered, /<span class="app-link-here" title="Not on this computer\.">Not here<\/span>/);
  assert.deepEqual(buttons(answered), ["", "Cancel", "Set up project…"]);
  assert.equal((answered.match(/btn btn-primary/g) ?? []).length, 1, "one verb");

  // A stop is the title, its sentence the description; the subject stays in view.
  const signIn = sheet({ phase: "answered", verdict: linkVerdict(answer([]), context()) });
  assert.match(signIn, /<h1[^>]*title="Sign in to Mallard Cloud to open maya\/portal\."[^>]*>Sign in first<\/h1>/);
  assert.match(signIn, /aria-describedby="[^"]+"/);
  assert.match(signIn, /class="sync-visually-hidden">Sign in to Mallard Cloud to open maya\/portal\.<\/span>/);
  assert.match(signIn, /<code class="app-link-subject">maya\/portal<\/code>/);
  assert.deepEqual(buttons(signIn), ["", "Cancel", "Set up Mallard Cloud"]);

  const refused = sheet({ phase: "answered", verdict: INVALID_LINK }, null);
  assert.match(refused, /<h1[^>]*>Broken link<\/h1>/);
  assert.doesNotMatch(refused, /app-link-subject/, "a link that cannot be read names nothing");
  assert.deepEqual(buttons(refused), ["", "Close"], "a link that cannot be read offers nothing");
  assert.doesNotMatch(refused, /mallard:\/\//, "none of the link's own text is shown");

  const retry = sheet({ phase: "answered", verdict: unanswered("Couldn’t reach Mallard Cloud.") });
  assert.match(retry, /<h1[^>]*role="alert"[^>]*>Mallard Cloud didn’t answer<\/h1>/);
  assert.deepEqual(buttons(retry), ["", "Cancel", "Try again"]);

  // Return presses the verb: focus moves to it when it appears, never to ×.
  const source = readFileSync("src/components/app-link/AppLinkSheet.tsx", "utf8");
  assert.match(source, /useEffect\(\(\) => \{ \(verbRef\.current \?\? exitRef\.current\)\?\.focus\(\); \}, \[verb\]\);/);
});

/* ——— the landing pages take the address ——————————————————————————————— */

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
    { field_id: "service_origin", label: "Service", value: `https://${HOST}`, is_path: false },
  ],
  settings: { service_origin: `https://${HOST}`, github_user_id: login === "octocat" ? "123" : "456", account_login: login },
  secrets: [{ secret_id: "account_token", present: true }],
});
const PROVIDERS = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
] as ProviderDescriptor[];

function newProject(storages: StorageSummary[], initialStorageId: string | null, initialAddress: string | null) {
  return renderToStaticMarkup(
    <NewProjectFolderStep
      workflowEpoch={1}
      isWorkflowCurrent={() => true}
      projects={[]}
      providers={PROVIDERS}
      profiles={[]}
      storages={storages}
      busy={false}
      initialSource="cloud"
      initialStorageId={initialStorageId}
      initialAddress={initialAddress}
      onCancel={() => undefined}
      onChoose={() => undefined}
      onOpenProject={() => undefined}
      onAddCloudAccount={() => undefined}
      onCloudProjectDone={() => undefined}
    />,
  );
}

test("New project opened for a repository starts with its address in the field", () => {
  const personal = cloudStorage("s-personal", "Rika321");
  const work = cloudStorage("s-work", "rika-acme");

  const one = newProject([personal], "s-personal", "maya/portal");
  assert.match(one, /<input[^>]*value="maya\/portal"/, "the field holds the address");
  assert.match(one, /role="radio" aria-checked="true"[^>]*aria-label="Mallard Cloud Rika321 · Mallard Cloud"/);

  // Several accounts see it: none ticked, and the hint names what waits on the pick.
  const several = newProject([personal, work], null, "maya/portal");
  assert.doesNotMatch(several, /aria-checked="true"[^>]*class="setup-pill/);
  assert.match(several, /<small class="sync-setup-hint">Choose an account for maya\/portal\.<\/small>/);
  assert.doesNotMatch(several, /<input[^>]*value="maya\/portal"/, "no Repository band until an account is chosen");

  // Without a link the page is as it was.
  assert.match(newProject([personal, work], null, null), /<small class="sync-setup-hint">Choose an account first\.<\/small>/);
  assert.match(newProject([personal], "s-personal", null), /<input[^>]*value=""/);
});

test("the address is looked up once by the pages' own lookups, which only select", () => {
  const picker = readFileSync("src/components/project-sync/RepositoryAddressPicker.tsx", "utf8");
  assert.match(picker, /const \[entry, setEntry\] = useState\(initialAddress \?\? ""\);/);
  assert.match(picker, /useEffect\(\(\) => \{\s*if \(initialAddress\) void runLookupFor\(initialAddress\);\s*\}, \[\]\);/);
  const chooser = readFileSync("src/components/storage-catalog/CloudRepositoryChoice.tsx", "utf8");
  assert.match(chooser, /const \[entry, setEntry\] = useState\(initialAddress \?\? ""\);/);
  assert.match(chooser, /useEffect\(\(\) => \{\s*if \(initialAddress\) void submit\(initialAddress\);\s*\}, \[\]\);/);

  // The static picker shows the address before any answer.
  const picked = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloudStorage("s-personal", "Rika321")}
      projectName=""
      localFingerprint={null}
      listing={{ state: "ready", repositories: [] }}
      selection={null}
      rows="match-option"
      offerNew={false}
      mode="clone"
      disabled={false}
      initialAddress="maya/portal"
      onSelect={() => undefined}
    />,
  );
  assert.match(picked, /<input[^>]*value="maya\/portal"/);
});

test("Add Catalog resource opened for a repository starts on From Mallard Cloud, on its account", () => {
  const accounts = [
    { id: "s-personal", name: "Mallard Cloud", account_login: "Rika321" },
    { id: "s-work", name: "Mallard Cloud work", account_login: "rika-acme" },
  ];
  const page = (initialCloud: { accountId: string; address: string } | null) => renderToStaticMarkup(
    <AddCatalogScopeWorkspace
      workflowEpoch={1}
      isWorkflowCurrent={() => true}
      busy={false}
      initialChoiceKey={null}
      scopes={[]}
      storages={[]}
      cloudAccounts={accounts}
      initialCloud={initialCloud}
      onSetUpCloud={() => undefined}
      onClose={() => undefined}
      onCreated={() => undefined}
      onAdded={() => undefined}
    />,
  );
  const opened = page({ accountId: "s-work", address: "maya/Mochi" });
  assert.match(opened, /role="radio" aria-checked="true"[^>]*>.*?From Mallard Cloud/s);
  assert.match(opened, /class="setup-pill active"[^>]*>.*?Mallard Cloud work · rika-acme/s);
  assert.match(opened, /<input[^>]*value="maya\/Mochi"/);
  assert.match(opened, /<button[^>]*btn-primary[^>]*disabled=""[^>]*>.*?Add<\/button>/s, "Add waits for the lookup and the person");

  // Without a link it opens on On this computer, as before.
  assert.match(page(null), /role="radio" aria-checked="true"[^>]*>.*?On this computer/s);
});

/* ——— what a link is allowed to do ————————————————————————————————————— */

test("a link reads and navigates; every write stays behind a verb the person presses", () => {
  // The hook: four reads, no write.
  const hook = readFileSync("src/components/app-link/useAppLink.ts", "utf8");
  const calls = [...hook.matchAll(/\b(projectSyncApi|storageCatalogApi)\.(\w+)/g)].map((match) => `${match[1]}.${match[2]}`);
  assert.deepEqual([...new Set(calls)].sort(), [
    "projectSyncApi.onAppLink",
    "projectSyncApi.resolveAppLink",
    "projectSyncApi.takeAppLink",
    "storageCatalogApi.findCloudRepository",
  ]);
  const model = readFileSync("src/components/app-link/appLink.ts", "utf8");
  assert.doesNotMatch(model, /\binvoke\b|projectSyncApi|storageCatalogApi/, "the model decides and calls nothing");

  // The shell: the sheet's verb runs the routes a sidebar click runs, and nothing that writes.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const follow = shell.slice(shell.indexOf("const goToLinkDestination = async"), shell.indexOf("if (!hostIdentityStatus && hostIdentityError)"));
  assert.ok(follow.length > 0 && follow.length < 3_000 && follow.includes("const followAppLink = async"), "the two functions were found");
  const called = [...new Set([...follow.matchAll(/\b(?:await |void )?([A-Za-z.]+)\(/g)].map((match) => match[1]))].sort();
  assert.deepEqual(called, [
    "appLink.drop",
    "appLink.retry",
    "appLink.setAside",
    "beginAddCatalogScope",
    "beginAddProject",
    "beginSessionRestore",
    "closeWorkspaceSettings",
    "createStorageDraft",
    "goToLinkDestination",
    "openCatalogIndex",
    "openProject",
    "openProjectsIndex",
    "setCatalogLandingItem",
    "setCatalogPreferredStorage",
    "setCatalogScopeId",
    "setError",
    "setSetupView",
  ]);
  // The listing's link goes on without a sheet, through the same routes and no others.
  assert.match(shell, /go: \(destination\) => void goToLinkDestination\(destination\),/);
  assert.match(hook, /const verdict = listingVerdict\(link, contextRef\.current\(\)\.cloudHosts\);\s*if \(verdict\.kind === "direct"\) \{\s*setHeld\(null\);\s*goRef\.current\(verdict\.destination\);\s*return true;\s*\}/);
  assert.match(hook, /if \(heldRef\.current\?\.phase !== "open" && waitsRef\.current\(\)\) \{\s*setHeld\(\{ link, phase: "waiting"/, "it waits like any link");
  assert.match(shell, /cloudHosts: configRef\.current\.storages\s*\.filter\(isManagedCloudStorage\)/, "the saved config, so a storage setup just added counts");
  assert.doesNotMatch(follow, /projectSyncApi|storageCatalogApi/, "no backend call follows a link");
  // The one route a session's link adds is the shipped Pull path with a
  // scope: it opens a review, and Pull there is the press that writes.
  assert.match(follow, /case "pull_session":[\s\S]{0,120}if \(!\(await openProject\(to\.projectId, to\.storageId\)\)\) return;\s*await beginSessionRestore\(to\.projectId, to\.storageId, to\.resourceId\);/);
  assert.match(shell, /const beginSessionRestore = \(projectId: string, storageId: string, resourceId: string\) => \(\s*beginProjectRestore\(projectId, storageId, \{ kind: "session", resource_id: resourceId \}\)\s*\);/);
  assert.match(shell, /const request = \{ storageId, bundleId, binding: targetBinding, scope \};/);
  // A session's link is answered as its repository's: the service is not
  // asked about the session, and the hook reads nothing more for it.
  assert.match(hook, /held\.link\.kind === "repository" \|\| held\.link\.kind === "session"/);
  assert.match(hook, /verdict = linkVerdict\(answer, \{ \.\.\.contextRef\.current\(\), places \}, session\);/);
  // A Catalog item's link lands on its page with the review its Pull button
  // opens; from Add, only when a link opened Add.
  assert.match(follow, /setCatalogLandingItem\(\{ scopeId: to\.scopeId, resourceId: to\.resourceId, pull: true \}\);/);
  assert.match(shell, /const pull = catalogAddPullRef\.current !== null;\s*catalogAddPullRef\.current = null;/);
  assert.match(shell, /initialPull=\{catalogLandingItem\?\.scopeId === activeCatalogScope\.scope\.scope_id && catalogLandingItem\.pull === true\}/);
  const catalogPage = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(catalogPage, /if \(pullRows\.length > 0\) void openPull\(\);\s*else notify\("Already here and up to date\.", "note"\);/);

  // The sheet opens by itself only when the shell is free; otherwise the link waits under the bar.
  assert.match(shell, /waits: \(\) => linkWaits\(\{\s*busy: busyRef\.current,\s*reviewOpen: Boolean\(pendingPush \|\| restorePlan \|\| preparingReview\),\s*dialogOpen: document\.querySelector\("dialog\[open\]:not\(\.app-link-sheet\)"\) !== null,\s*\}\)/);
  assert.match(shell, /heldLink\?\.phase === "waiting"/);
  assert.match(shell, /message: `\$\{link\.kind === "listing" \? listingPage\(link\) : link\.address\} is waiting\.`/);
  assert.match(shell, /disabled=\{busy\}[\s\S]{0,160}onClick=\{appLink\.open\}/, "Open waits for a running operation");
  // One sheet per arrival, and a sheet lets go of its own link only: a close
  // that lands late must not take a newer link with it.
  assert.match(shell, /heldLink\?\.phase === "open" && \([\s\S]{0,220}<AppLinkSheet\s+key=\{heldLink\.id\}/);
  assert.match(shell, /onClose=\{\(\) => appLink\.drop\(heldLink\.id\)\}/);
  assert.match(hook, /drop: \(id\?: number\) => setHeld\(\(current\) => \(current && \(id === undefined \|\| current\.id === id\) \? null : current\)\)/);
  assert.match(hook, /answered\?\.round === held\.round \? \{ phase: "answered", verdict: answered\.verdict \} : \{ phase: "looking" \}/, "an answer shows only for the asking it came from");
  // A new Mallard Cloud account lets through the link that was set aside
  // for one, when its storage opens: the listing's goes on to its own page.
  assert.match(shell, /if \(!result\.replaced && isManagedCloudStorage\(committed\) && appLink\.awaitsAccount\(\)\) \{\s*appLinkLandingRef\.current = result\.storage_id;\s*\}/);
  assert.match(shell, /appLinkLandingRef\.current = null;\s*if \(linkLanding === storageId && appLink\.resume\(\)\) return;/);

  // Both commands and the scheme are registered; the webview holds no deep-link permission.
  const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
  assert.match(lib, /project_sync::app_link::take_app_link,\s*project_sync::app_link::resolve_app_link,/);
  assert.match(lib, /project_sync::commands::find_project_session,/, "where a pulled session's page is: a read");
  assert.match(lib, /storage_catalog::resource_commands::find_catalog_cloud_repository,/);
  assert.match(lib, /\.plugin\(tauri_plugin_deep_link::init\(\)\)/);
  const capability = readFileSync("src-tauri/capabilities/default.json", "utf8");
  assert.doesNotMatch(capability, /deep-link/, "the link is read in Rust; the window is handed only what was read");
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  assert.deepEqual(config.plugins["deep-link"], { desktop: { schemes: ["mallard"] } });
});
