import { entryKind, isAddressShaped, isInvitationCodeShaped } from "../../src/components/managed-cloud/RepositoryLookupField";
import { listingRoleWord, matchesFilter } from "../../src/components/managed-cloud/CloudScopes";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { ManagedCloudRepository, ProjectSetupDraft, StorageSummary } from "../../src/types";
import BundleConnectionDialog from "../../src/components/project-sync/BundleConnectionDialog";
import { mergeSavedSetupDraft } from "../../src/components/project-sync/ProjectSetupWorkspace";
import RepositoryAddressPicker, {
  gitRemoteWord,
  inListing,
  managedCloudAccount,
  repositoryCautions,
  repositoryVerdict,
} from "../../src/components/project-sync/RepositoryAddressPicker";
import { deriveRepositoryName, nameRefusal, predictRepositoryAddress, type AddressPrediction } from "../../src/components/managed-cloud/repositoryAddress";
import { en } from "../../src/i18n/en";
import { i18n } from "../../src/i18n";
import ManagedCloudChip, { ManagedCloudPanel } from "../../src/components/managed-cloud/ManagedCloudChip";
import ManagedCloudAddress from "../../src/components/managed-cloud/ManagedCloudAddress";
import PublishVersionSheet from "../../src/components/managed-cloud/PublishVersionSheet";
import { discardDraftQuestion } from "../../src/components/managed-cloud/managedCloudLink";
import type { ManagedCloudLink, ManagedCloudStatus } from "../../src/components/managed-cloud/types";

/* Connecting a project to a Mallard Cloud repository by its website
   address, or by a private repository's invitation code. The picker never
   prints a bundle id; the rows are named by address; New repository says
   the address it will create; a public repository is connectable by anyone
   signed in; and the dialog opens for a Mallard Cloud storage whether or
   not the account holds anything yet. */

const cloud: StorageSummary = {
  storage_id: "storage-cloud",
  name: "Mallard Cloud",
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "Rika321", is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: { service_origin: "https://mallard-ai.com", github_user_id: "123", account_login: "Rika321" },
  secrets: [{ secret_id: "account_token", present: true }],
};

const CHECKOUT = "a".repeat(64);
const BUNDLE = "3f9a0c7e1b2d4a6f8c0e2b4d6f8a1c3e";

function repository(overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository {
  return {
    repository_id: "repo_1",
    owner_login: "Rika321",
    name: "HorrorGame",
    address: "Rika321/HorrorGame",
    display_name: overrides.name ?? "HorrorGame",
    website_url: "https://mallard-ai.com/Rika321/HorrorGame",
    bundle_id: BUNDLE,
    repository_class: "git_project",
    visibility: "private",
    role: "owner",
    can_push: true,
    repository_fingerprint: CHECKOUT,
    git_remote: "matches",
    github_url: null,
    revision: 4,
    updated_at: 1_787_298_240,
    resources: 3,
    registration_only: false,
    description: null,
    star_count: 0,
    viewer_starred: false,
    ...overrides,
  };
}

test("the New row predicts the address the service would create", () => {
  // The case table the service, the desktop and the website all assert.
  assert.equal(deriveRepositoryName("HorrorGame"), "HorrorGame");
  assert.equal(deriveRepositoryName("Horror Game"), "Horror-Game");
  assert.equal(deriveRepositoryName("  --weird__name.v2--  "), "--weird__name.v2--");
  assert.equal(deriveRepositoryName("My--Repo"), "My--Repo");
  assert.equal(deriveRepositoryName("大黄"), "大黄");
  assert.equal(deriveRepositoryName("  日本語  "), "日本語");
  assert.equal(deriveRepositoryName("Cafe\u0301"), "Caf\u00e9");
  assert.equal(deriveRepositoryName("大 黄 / 二"), "大-黄-二");
  assert.equal(deriveRepositoryName(".github"), ".github");
  assert.equal(deriveRepositoryName("x".repeat(150)), "x".repeat(100));
  assert.equal(deriveRepositoryName(`${"x".repeat(99)}-yyy`), "x".repeat(99));
  assert.equal(deriveRepositoryName("黄".repeat(150)), "黄".repeat(100));
  assert.equal(deriveRepositoryName("😀abc"), "abc");
  for (const nothing of ["🦆🦆", ".", "..", "///", "", "   "]) assert.equal(deriveRepositoryName(nothing), null, JSON.stringify(nothing));
  const addressOf = (prediction: AddressPrediction) => (prediction.kind === "address" ? prediction.address : prediction.kind);
  const mine = [repository({}), repository({ repository_id: "repo_2", name: "HorrorGame-2", address: "Rika321/HorrorGame-2" })];
  assert.deepEqual(predictRepositoryAddress("Rika321", "HorrorGame", mine), {
    kind: "address",
    address: "Rika321/HorrorGame-3",
    name: "HorrorGame-3",
    base: "HorrorGame",
    suffixed: true,
  });
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "horrorgame", mine)), "Rika321/horrorgame-3", "names compare case-insensitively");
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "Birding Log", mine)), "Rika321/Birding-Log");
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "My--Repo", [repository({ name: "My--Repo" })])), "Rika321/My--Repo-2");
  // Any script; a decomposed spelling of a held name is the same key; a title with nothing usable is a refusal, never `repository`.
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "大黄", mine)), "Rika321/大黄");
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "大黄", [repository({ name: "大黄", address: "Rika321/大黄" })])), "Rika321/大黄-2");
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "A\u0308rger", [repository({ name: "Ärger", address: "Rika321/Ärger" })])), "Rika321/Ärger-2");
  assert.deepEqual(predictRepositoryAddress("Rika321", "🦆🦆", mine), { kind: "refused" });
  assert.equal(nameRefusal("project"), "Name needs a letter or digit; rename the project first.");
  // Another owner's repository of the same name does not take the name.
  const theirs = [repository({ owner_login: "someone", address: "someone/HorrorGame" })];
  assert.equal(addressOf(predictRepositoryAddress("Rika321", "HorrorGame", theirs)), "Rika321/HorrorGame");
  assert.deepEqual(managedCloudAccount(cloud), { login: "Rika321", origin: "https://mallard-ai.com", host: "mallard-ai.com" });
});

test("the found row's words follow role and remote", () => {
  assert.deepEqual(repositoryVerdict(repository({})), { tone: "ok", icon: "check-circle", text: "Yours · Owner" });
  assert.equal(repositoryVerdict(repository({ role: "member", git_remote: "differs" })).text, "Collaborator · Member");
  assert.equal(repositoryVerdict(repository({ role: "member", git_remote: "differs" })).tone, "warn");
  assert.deepEqual(repositoryVerdict(repository({ role: null, visibility: "public" })), { tone: "ok", icon: "check-circle", text: "Public · anyone signed in can push" });
  assert.equal(repositoryVerdict(repository({ role: null, visibility: "public", git_remote: "differs" })).tone, "warn", "only the remote can warn");
  assert.equal(gitRemoteWord(repository({}), CHECKOUT), "this checkout's");
  assert.equal(gitRemoteWord(repository({ repository_class: "non_git_project", repository_fingerprint: null }), null), "none, like this folder");
  assert.equal(gitRemoteWord(repository({ git_remote: "differs" }), "b".repeat(64)), "differs");
  assert.equal(gitRemoteWord(repository({ git_remote: "unknown", repository_fingerprint: null }), CHECKOUT), "unverified");
  assert.deepEqual(repositoryCautions(repository({ role: null, git_remote: "differs" })), ["Different Git remote than this checkout"], "no read-only caution: a public repository takes pushes");
  assert.deepEqual(repositoryCautions(repository({})), []);
  assert.ok(isInvitationCodeShaped("abcde-fghjk-mnpqr-stvwx"));
  assert.ok(isInvitationCodeShaped(" ABCDE FGHJK MNPQR STVWX "));
  assert.ok(!isInvitationCodeShaped("ABCDE-FGHJK"));
  assert.ok(!isInvitationCodeShaped(""));
  // An address always has a slash, even one that is 20 characters long.
  assert.ok(!isInvitationCodeShaped("abcdefghij/klmnopqrs"));
  assert.ok(!isInvitationCodeShaped("mallard-ai.com/Rika321/HorrorGame"));
  assert.ok(!isInvitationCodeShaped("abcdefghij.klmnopqrs"));
  // The alphabet is the service's to refuse, so a mistyped code still joins
  // and gets the code's error rather than the address's.
  assert.ok(isInvitationCodeShaped("OOOOO-00000-IIIII-11111"));
  assert.ok(inListing({ bundle_id: BUNDLE }, [{ bundle_id: "a".repeat(32) }, { bundle_id: BUNDLE }]));
  assert.ok(!inListing({ bundle_id: BUNDLE }, [{ bundle_id: "a".repeat(32) }]));
  assert.ok(!inListing({ bundle_id: null }, [{ bundle_id: null }]), "a repository with no bundle is never a row's twin");
});

test("a lookup's answer sets its chips under the name in every host", () => {
  // On one line the auto-sized chip column took its full width first: in
  // project setup the name shrank to a letter and the verdict wrapped a word
  // per line under the Visibility chip.
  for (const file of ["src/components/project-sync/RepositoryAddressPicker.tsx", "src/components/storage-catalog/CloudRepositoryChoice.tsx"]) {
    assert.match(readFileSync(file, "utf8"), /className=\{`storage-found-row with-verb repo-answer-row\$\{selected \? " selected" : ""\}`\}/, file);
  }
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.storage-found-row\.repo-answer-row \{ grid-template-columns: 26px minmax\(0, 1fr\) auto; row-gap: 6px; \}/);
  assert.match(css, /\.repo-answer-row > \.storage-found-facts \{ grid-row: 2; grid-column: 2 \/ -1;/);
  // The row's own layout, never one host's: setup was the host left out.
  assert.doesNotMatch(css, /\.(?:sync-bundle-connection-dialog|cloud-project-repository) \.storage-found-row/);
});

test("the picker names repositories by address and never by bundle id", () => {
  const rows = [
    repository({}),
    repository({ repository_id: "repo_2", owner_login: "rikaqu0223-arch", name: "tidewatch", address: "rikaqu0223-arch/tidewatch", role: "member", git_remote: "differs", repository_fingerprint: "b".repeat(64), bundle_id: "a81b3c5d7e9f1a2b4c6d8e0f2a4b6c8d" }),
    repository({ repository_id: "repo_3", name: "Birding-Log", address: "Rika321/Birding-Log", repository_class: "non_git_project", visibility: "public", git_remote: "differs", repository_fingerprint: null, bundle_id: "c02d4e6f8a1b3c5d7e9f0a2b4c6d8e1f", registration_only: true }),
  ];
  const html = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloud}
      projectName="HorrorGame"
      localFingerprint={CHECKOUT}
      listing={{ state: "ready", repositories: rows, refreshing: false }} onRefresh={() => {}}
      selection={{ kind: "new" }}
      rows="match-option"
      offerNew
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(html, /address or invitation code<\/span>/i, "the code has a place before any lookup");
  assert.match(html, /placeholder="Search, or paste a link, owner\/name or an invitation code"/, "words filter; a link, an address or a code is looked up");
  assert.equal(html.match(/<input/g)?.length, 1, "one field takes the address or the code");
  assert.match(html, /Look up/);
  assert.doesNotMatch(html, /Invitation code \(optional\)|Join repository/);
  assert.doesNotMatch(html, /Paste the page address/);
  assert.match(html, /role="tablist"[\s\S]*?data-scope="yours">[\s\S]*?Yours<small>3<\/small><\/button>/, "Yours heads the scopes, with its count");
  assert.match(html, /data-scope="starred"[\s\S]*?data-scope="public"/);
  assert.doesNotMatch(html, /read-only/i);
  // The display name leads the row; the address is the line beneath.
  assert.match(html, /<strong><span>HorrorGame<\/span><small class="sync-bundle-recommended">Git match<\/small>/);
  assert.match(html, /class="repo-row-address">Rika321\/HorrorGame<\/span>/);
  assert.match(html, /Private · Owner/);
  assert.match(html, /<strong><span>tidewatch<\/span>[\s\S]*?class="repo-row-address">rikaqu0223-arch\/tidewatch<\/span>/);
  assert.match(html, /Private · Member/);
  assert.match(html, /Public · Owner/);
  assert.match(html, /not pushed yet/);
  assert.match(html, /class="repo-new-address">Rika321\/HorrorGame-2<\/span>/);
  assert.match(html, /HorrorGame is already used\./);
  assert.doesNotMatch(html, new RegExp(BUNDLE), "no bundle id in the copy");
  assert.doesNotMatch(html, /a81b3c5d7e9f1a2b/);
  // Only the one true Git match wears the badge.
  assert.equal(html.match(/Git match/g)?.length, 1);

  const fresh = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloud}
      projectName="Horror Game"
      localFingerprint={null}
      listing={{ state: "ready", repositories: [], refreshing: false }} onRefresh={() => {}}
      selection={{ kind: "new" }}
      rows="choice"
      offerNew
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(fresh, /class="repo-new-address">Rika321\/Horror-Game<\/span>/);
  assert.match(fresh, /aria-checked="true"/, "the draft's own default, New repository, is the selected row");

  const acknowledging = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloud}
      projectName="HorrorGame"
      localFingerprint={CHECKOUT}
      listing={{ state: "ready", repositories: rows, refreshing: false }} onRefresh={() => {}}
      selection={{ kind: "existing", bundle_id: rows[1].bundle_id!, display_name: rows[1].address, repository_fingerprint: rows[1].repository_fingerprint, mismatch_acknowledged: false }}
      rows="choice"
      offerNew={false}
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(acknowledging, /pushed from a different Git remote than this checkout/);
  assert.doesNotMatch(acknowledging, /New repository/);

  // No rows: a heading and one quiet line, never an empty bordered list.
  const empty = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloud}
      projectName=""
      localFingerprint={null}
      listing={{ state: "ready", repositories: [], refreshing: false }} onRefresh={() => {}}
      selection={null}
      rows="match-option"
      offerNew={false}
      mode="clone"
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(empty, /data-scope="yours">[\s\S]*?Yours<\/button>/, "no count on an empty scope");
  assert.match(empty, /<strong>No repositories yet\.<\/strong>/);
  assert.match(empty, /Browse Public/, "the empty scope offers the other way in");
  assert.doesNotMatch(empty, /radiogroup/);
  assert.doesNotMatch(empty, /Paste the page address/, "clone mode says fewer words");
});

test("the connect dialog for a Mallard Cloud storage takes an address, empty account or not", () => {
  const html = renderToStaticMarkup(
    <BundleConnectionDialog
      projectName="HorrorGame"
      currentBundleId="b751caac4627e21d5809e0cf925a7c1f"
      projectFingerprint={CHECKOUT}
      storage={{ ...cloud, name: "Mallard Cloud Rika321" }}
      matches={[]}
      reason="link"
      busy={false}
      error={null}
      onCancel={() => undefined}
      onUseExisting={() => undefined}
      onKeepCurrent={() => undefined}
    />,
  );
  assert.match(html, /id="sync-bundle-connection-title">Connect HorrorGame<\/h1>/);
  assert.doesNotMatch(html, /Repo identity|Paste a repository address|Connecting keeps/);
  assert.match(html, /<strong>Mallard Cloud · Rika321<\/strong>/);
  assert.doesNotMatch(html, /Mallard Cloud Rika321 · Rika321/);
  assert.match(html, /address or invitation code/i);
  assert.match(html, /Loading…/);
  assert.doesNotMatch(html, /Paste an address or a code/);
  assert.doesNotMatch(html, /Current local repo/, "no bundle id in the context line");
  assert.doesNotMatch(html, /Create separate repo/);
  assert.match(html, />Connect<\/button>/);
  assert.doesNotMatch(html, /Finish connection|Check connection/);

  const missing = renderToStaticMarkup(
    <BundleConnectionDialog
      projectName="HorrorGame"
      currentBundleId="b751caac4627e21d5809e0cf925a7c1f"
      projectFingerprint={CHECKOUT}
      storage={cloud}
      matches={[]}
      reason="missing"
      busy={false}
      error={null}
      onCancel={() => undefined}
      onUseExisting={() => undefined}
      onKeepCurrent={() => undefined}
    />,
  );
  assert.match(missing, /Choose a repository/);
  assert.match(missing, /Review separate Push/);
});

test("linking a Mallard Cloud storage always opens the address dialog, and setup never picks a row by itself", () => {
  const projectSync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const link = projectSync.slice(projectSync.indexOf("const linkStorage = async"));
  assert.ok(
    link.indexOf("isManagedCloudStorage(") < link.indexOf("listRemoteBundleSnapshots("),
    "the managed storage is routed to the dialog before any listing",
  );
  assert.match(link, /matches: \[\], reason: "link"/);
  assert.match(projectSync, /<BundleConnectionDialog\s+projectName=\{pendingConnectionProject\.display_name\}/,
    "the preview uses the same persisted name as creation, even when the project has a local alias");

  const setup = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  assert.doesNotMatch(setup, /repoAutoSelected/);
  assert.match(setup, /<RepositoryAddressPicker/);

  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.match(api, /resolve_managed_cloud_repository/);
  assert.match(api, /list_managed_cloud_repositories/);
  assert.match(api, /accept_managed_cloud_invitation/);

  // A public repository the account does not belong to is a contributor's, not read-only.
  const card = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const cloud = readFileSync("src/components/managed-cloud/managedCloudLink.ts", "utf8");
  assert.match(cloud, /link\.role === null && link\.visibility === "public" && link\.can_push/);
  assert.match(cloud, /link\.role === null \? t\("cloud:role\.contributor"\)/);
  assert.equal(en.cloud.role.contributor, "Contributor");
  assert.doesNotMatch(card, /Read-only/);
  assert.doesNotMatch(readFileSync("src/i18n/en/projectPage.ts", "utf8"), /read-only/i);
  assert.doesNotMatch(cloud, /Read-only/);
  // The chip's and panel's words live in the cloud catalog now.
  assert.doesNotMatch(readFileSync("src/i18n/en/cloud.ts", "utf8"), /read-only/i);
  const dialog = readFileSync("src/components/project-sync/BundleConnectionDialog.tsx", "utf8");
  assert.doesNotMatch(dialog, /read-only/i);
  // The dialog's words live in the reviews catalog now.
  assert.doesNotMatch(readFileSync("src/i18n/en/reviews.ts", "utf8"), /read-only/i);
});

test("a saved folder name reaches the preview without replacing newer name or folder edits", () => {
  const submitted: ProjectSetupDraft = {
    schema: 1, draft_id: "draft-name", local_project_id: "project-name", new_bundle_id: BUNDLE,
    project_root: "/tmp/new-folder", canonical_project_root: "/tmp/old-folder",
    display_name: "old-folder", repository_fingerprint: null, profiles: {}, storage: null,
    repository: { kind: "new" }, revision: 1, created_at: 1, updated_at: 1,
  };
  const saved = { ...submitted, canonical_project_root: "/tmp/new-folder", display_name: "My--Repo", repository_fingerprint: CHECKOUT, revision: 2, updated_at: 2 };
  assert.equal(mergeSavedSetupDraft(submitted, submitted, saved).display_name, "My--Repo");
  assert.equal(mergeSavedSetupDraft(submitted, submitted, saved).repository_fingerprint, CHECKOUT);
  const editedName = mergeSavedSetupDraft({ ...submitted, display_name: "Explicit name" }, submitted, saved);
  assert.equal(editedName.display_name, "Explicit name");
  assert.equal(editedName.revision, 2);
  const editedFolder = mergeSavedSetupDraft({ ...submitted, project_root: "/tmp/next-folder" }, submitted, saved);
  assert.equal(editedFolder.display_name, submitted.display_name);
  assert.equal(editedFolder.canonical_project_root, submitted.canonical_project_root);
  assert.equal(editedFolder.repository_fingerprint, null);
  assert.equal(editedFolder.revision, 2);
});

test("rows carry the description and the stars, and the one field knows words from an address or a code", () => {
  const rows = [
    repository({ description: "Customer portal.", star_count: 3, viewer_starred: true }),
    repository({ repository_id: "repo_2", name: "notes", address: "Rika321/notes", display_name: "notes", bundle_id: "a".repeat(32), repository_class: "non_git_project", git_remote: "differs", repository_fingerprint: null, description: null, star_count: 0 }),
    repository({ repository_id: "repo_3", name: "大黄", address: "Rika321/大黄", display_name: "大黄", bundle_id: "c".repeat(32), repository_class: "non_git_project", git_remote: "differs", repository_fingerprint: null, description: "黄色的大狗。", star_count: 0 }),
    repository({ repository_id: "repo_4", name: "Horror-Game", address: "Rika321/Horror-Game", display_name: "Horror Game", bundle_id: "d".repeat(32), repository_class: "non_git_project", git_remote: "differs", repository_fingerprint: null, description: null, star_count: 0 }),
  ];
  const html = renderToStaticMarkup(
    <RepositoryAddressPicker
      storage={cloud}
      projectName="HorrorGame"
      localFingerprint={CHECKOUT}
      listing={{ state: "ready", repositories: rows, refreshing: false }} onRefresh={() => {}}
      selection={null}
      rows="choice"
      offerNew={false}
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  // The display name leads; the address and the description are the second line.
  assert.match(html, /class="sync-bundle-choice repo-row starred"[\s\S]*?<strong><span>HorrorGame<\/span>[\s\S]*?class="repo-row-blurb"><span class="repo-row-address">Rika321\/HorrorGame<\/span><i class="repo-row-sep" aria-hidden="true">·<\/i>Customer portal\.<\/span>/, "the address and the description are the second line");
  assert.match(html, /data-icon="star-filled"[^>]*>[\s\S]*?<\/svg> 3 · /, "the account's own star is filled, with the count");
  assert.match(html, /<strong><span>notes<\/span>[\s\S]*?class="repo-row-blurb"><span class="repo-row-address">Rika321\/notes<\/span><\/span>/, "no description: the address alone");
  // A name of any script prints as the service spells it; a title that differs from its address shows both.
  assert.match(html, /<strong><span>大黄<\/span>[\s\S]*?class="repo-row-blurb"><span class="repo-row-address">Rika321\/大黄<\/span><i class="repo-row-sep" aria-hidden="true">·<\/i>黄色的大狗。<\/span>/);
  assert.match(html, /<strong><span>Horror Game<\/span>[\s\S]*?class="repo-row-address">Rika321\/Horror-Game<\/span>/);
  assert.doesNotMatch(html, /Rika321\/notes[\s\S]*?data-icon="star"/, "no stars, no star");
  assert.match(html, /<strong>Private · Owner<\/strong>/, "the role stays in the meta column");

  assert.equal(entryKind(""), "idle");
  assert.equal(entryKind("bis"), "filter");
  assert.equal(entryKind("maya/Mochi"), "address");
  assert.equal(entryKind("https://mallard-ai.com/zoe/Lantern"), "address");
  assert.equal(entryKind("mallard-ai.com/zoe/Lantern/sessions/1"), "address");
  assert.equal(entryKind("mallard-waffle-ivy-2026"), "code");
  assert.ok(isAddressShaped("Rika321/HorrorGame") && !isAddressShaped("horror game"));
  // A name of any script, raw or percent-encoded, is an address; a bare word or a name with a space is a filter.
  assert.ok(isAddressShaped("Rika321/大黄") && isAddressShaped("Rika321/%E5%A4%A7%E9%BB%84") && isAddressShaped("mallard-ai.com/Rika321/大黄/"));
  assert.ok(isAddressShaped("https://mallard-ai.com/Rika321/%E5%A4%A7%E9%BB%84/sessions/x?commit=1"));
  assert.ok(!isAddressShaped("大黄") && !isAddressShaped("Rika321/大 黄") && !isAddressShaped("大黄/HorrorGame"));
  assert.equal(entryKind("Rika321/大黄"), "address");
  assert.ok(matchesFilter("beagle", "kai/Biscuit", "A beagle that fetches finished builds."));
  assert.ok(matchesFilter("BIS", "kai/Biscuit", null) && !matchesFilter("tofu", "kai/Biscuit", null));
  // The display name counts too, and the comparison is by key: NFC and case.
  assert.ok(matchesFilter("大", "Rika321/大黄", null) && matchesFilter("horror game", "Rika321/Horror-Game", null, "Horror Game"));
  assert.ok(matchesFilter("CAFÉ", "x/Cafe\u0301", null) && !matchesFilter("duck", "Rika321/大黄", null, "大黄"));
  assert.equal(listingRoleWord({ role: null, visibility: "public" }), "Public");
  assert.equal(listingRoleWord({ role: "member", visibility: "private" }), "Member");
  assert.equal(listingRoleWord({ role: "member", visibility: "public" }), "Public · Member");
});

/* Chinese: the Cloud chip in two states, its panel, the address band and
   Publish version. Version stays English; addresses, logins and the
   service's own sentences stay as given. */
test("in Chinese the Cloud chip, its panel and Publish version speak Chinese, and what the service said stays as given", () => {
  const link: ManagedCloudLink = {
    repository_id: "repo_one", visibility: "private", can_pull: true, can_push: true,
    website_url: "https://mallard-ai.com/rika/portal", owner_login: "rika", name: "portal",
    repository_revision: 7, can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner",
    release: { kind: "private" },
  };
  const ready: ManagedCloudStatus = {
    repository_id: "repo_one", state: "ready", message: null, link,
    publication: { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: 3, public_version: null, review_version: null },
  };
  const signedOut: ManagedCloudStatus = { repository_id: "repo_one", state: "sign_in_required", link: null, message: "Mallard Cloud rejected the saved sign-in." };
  const actions = { onOpenUrl: () => {}, onPublishVersion: () => {}, onDiscardDraft: () => {}, onRetry: () => {}, onStorageSettings: () => {} };
  void i18n.changeLanguage("zh-CN");
  try {
    const chip = renderToStaticMarkup(<ManagedCloudChip status={ready} storageName="Mallard Cloud Rika" busy={false} refreshing={false} {...actions} />);
    assert.match(chip, /<b>私有<\/b>/);
    assert.match(chip, /aria-label="Mallard Cloud：私有。草稿比 Version 2 多 3 次上传。"/);
    // Signed out: the chip's word is Chinese, the service's sentence is not.
    const out = renderToStaticMarkup(<ManagedCloudChip status={signedOut} storageName="Mallard Cloud Rika" busy={false} refreshing={false} {...actions} />);
    assert.match(out, /<b>登录<\/b>/);
    assert.match(out, /aria-label="Mallard Cloud：登录。Mallard Cloud rejected the saved sign-in\."/);

    const panel = renderToStaticMarkup(<ManagedCloudPanel status={ready} storageName="Mallard Cloud Rika" busy={false} refreshing={false} close={() => {}} {...actions} />);
    assert.match(panel, /<span>私有<\/span><small>rika\/portal<\/small>/);
    assert.match(panel, /<dt>草稿<\/dt><dd>多 3 次上传<\/dd>[^]*?<dt>Version<\/dt><dd>2<\/dd>[^]*?<dt>公开<\/dt><dd>私有<\/dd>[^]*?<dt>你<\/dt><dd>所有者<\/dd>/);
    assert.match(panel, /发布 Version 3…[^]*?申请公开发布[^]*?查看草稿[^]*?Version 列表[^]*?丢弃草稿…/);
    assert.match(panel, /aria-label="复制草稿链接" title="复制草稿链接\nhttps:\/\/mallard-ai\.com\/rika\/portal\?version=draft"/);
    assert.match(panel, /title="丢弃草稿… · 回到 Version 2"/);
    assert.match(panel, /role="group" aria-label="Mallard Cloud 操作"/);
    const outPanel = renderToStaticMarkup(<ManagedCloudPanel status={signedOut} storageName="Mallard Cloud Rika" busy={false} refreshing={false} close={() => {}} {...actions} />);
    assert.match(outPanel, /登录[^]*?Mallard Cloud Rika[^]*?Mallard Cloud rejected the saved sign-in\.[^]*?存储设置/);

    // Discard draft asks in Chinese around the address as it is.
    const question = discardDraftQuestion(link, ready.publication);
    assert.equal(question.title, "丢弃草稿？");
    assert.match(question.message, /^丢弃“rika\/portal”的草稿？\n\n从 Mallard Cloud 删除 Version 2 之后的每一次上传。/);
    assert.equal(question.kept, "Version 2");
    assert.deepEqual([question.okLabel, question.cancelLabel], ["丢弃", "取消"]);

    const sheet = renderToStaticMarkup(
      <PublishVersionSheet version={3} holds={[{ icon: "message", word: "8 个会话" }]} sessionsToPush={2} busy={false} onPublish={() => {}} onClose={() => {}} />,
    );
    assert.match(sheet, /<h1 id="[^"]+">发布 Version 3<\/h1>/);
    assert.match(sheet, /aria-label="关闭，不发布"/);
    assert.match(sheet, /<span class="app-link-here">本机有 2 个会话尚未上传，不会包含在内<\/span>/);
    assert.match(sheet, /placeholder="名称（可选）"/);
    assert.match(sheet, /<button type="button" class="btn">取消<\/button><button type="button" class="btn btn-primary">发布 Version 3<\/button>/);
    assert.doesNotMatch(sheet, /版本/);

    const band = renderToStaticMarkup(
      <ManagedCloudAddress account={{ login: "octocat", origin: "https://mallard-ai.com", host: "mallard-ai.com" }} onOpenUrl={() => {}} />,
    );
    assert.match(band, /已登录：<b>octocat<\/b>/);
    assert.match(band, /title="在浏览器中打开 https:\/\/mallard-ai\.com"[^]*?打开网站<\/button>/);

    // The picker's field and tabs; the rows' addresses and roles as words.
    const picker = renderToStaticMarkup(
      <RepositoryAddressPicker storage={cloud} projectName="HorrorGame" localFingerprint={CHECKOUT}
        listing={{ state: "ready", repositories: [repository({})], refreshing: false }} onRefresh={() => {}} selection={{ kind: "new" }}
        rows="match-option" offerNew disabled={false} onSelect={() => undefined} />,
    );
    assert.match(picker, /placeholder="搜索，或粘贴链接、owner\/name 或邀请码"/);
    assert.match(picker, /查找<\/button>/);
    assert.match(picker, /role="tablist" aria-label="查找范围"[^]*?你的<small>1<\/small>[^]*?已加星标[^]*?公开/);
    assert.match(picker, /<strong>新仓库<\/strong>[^]*?class="repo-new-address">Rika321\/HorrorGame-2<\/span><span>HorrorGame 已被使用。<\/span>/);
    assert.match(picker, /私有 · 所有者/);
    assert.equal(listingRoleWord({ role: "member", visibility: "public" }), "公开 · 成员");
    assert.equal(nameRefusal("project"), "名称需要包含字母或数字；请先重命名项目。");
  } finally {
    void i18n.changeLanguage("en");
  }
});

test("the scope strip ends in Refresh, and the rows stay while it runs", () => {
  const rows = [repository({}), repository({ repository_id: "repo_2", name: "tidewatch", address: "Rika321/tidewatch", bundle_id: "a81b3c5d7e9f1a2b4c6d8e0f2a4b6c8d" })];
  const picker = (refreshing: boolean) => renderToStaticMarkup(
    <RepositoryAddressPicker storage={cloud} projectName="HorrorGame" localFingerprint={CHECKOUT}
      listing={{ state: "ready", repositories: rows, refreshing }} onRefresh={() => {}}
      selection={null} rows="choice" offerNew={false} disabled={false} onSelect={() => undefined} />,
  );
  const rest = picker(false);
  assert.match(rest, /role="tablist"[\s\S]*?<button type="button" class="btn btn-ghost repo-scope-refresh" aria-label="Refresh" title="Refresh"><svg/, "an icon square at the strip's end, named by its tooltip");
  assert.doesNotMatch(rest, /repo-scope-refresh"[^>]*aria-busy/);
  assert.match(rest, /<span class="sync-visually-hidden" role="status"><\/span>/, "nothing is announced before a refresh");
  const busy = picker(true);
  assert.match(busy, /repo-scope-refresh" aria-label="Refreshing…" title="Refreshing…" aria-busy="true" disabled=""><svg[^>]*class="[^"]*icon-spin"/, "pending: disabled, spinning");
  assert.match(busy, /Yours<small>2<\/small>/, "the count stays");
  assert.equal(busy.match(/class="sync-bundle-choice repo-row/g)?.length, 2, "the rows stay on screen while the reload runs");
  assert.doesNotMatch(busy, /status-loader/, "no loader replaces them");
  assert.match(busy, /role="status">Refreshing…<\/span>/);

  // One Refresh reads every scope again, keeping what is held: the hook
  // tells a refresh from a new key, and every host wires both halves.
  const scopes = readFileSync("src/components/managed-cloud/CloudScopes.tsx", "utf8");
  assert.match(scopes, /sameKey && current\.state === "ready" \? \{ \.\.\.current, refreshing: true \} : \{ state: "loading" \}/);
  assert.match(scopes, /sameKey && held\.state === "ready"/, "Public's first page is read again only when Public holds rows");
  assert.match(scopes, /\}, \[key, generation\]\);/);
  const pickerSource = readFileSync("src/components/project-sync/RepositoryAddressPicker.tsx", "utf8");
  assert.match(pickerSource, /sameSource && current\.state === "ready" \? \{ \.\.\.current, refreshing: true \} : \{ state: "loading" \}/);
  assert.match(pickerSource, /onRefresh=\{\(\) => \{ onRefresh\(\); listings\.refresh\(\); \}\}/);
  for (const host of ["BundleConnectionDialog", "ProjectSetupWorkspace", "CloudProjectSource"]) {
    const source = readFileSync(`src/components/project-sync/${host}.tsx`, "utf8");
    assert.match(source, /onRefresh=\{(refresh|managed\.refresh)\}/, `${host} passes the account listing's refresh`);
  }
  const chooser = readFileSync("src/components/storage-catalog/CloudRepositoryChoice.tsx", "utf8");
  assert.match(chooser, /onRefresh=\{\(\) => \{ setGeneration\(\(held\) => held \+ 1\); listings\.refresh\(\); \}\}/, "the Library's chooser refreshes the same way");
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.repo-scope-tabs > \.repo-scope-refresh \{ margin-left: auto; margin-bottom: 7px; width: 28px; min-height: 28px; padding: 0; \}/);
  assert.match(css, /\.repo-scope-tabs > \.repo-scope-sort \+ \.repo-scope-refresh \{ margin-left: 8px; \}/);
  assert.match(css, /\.repo-scope-refresh:active:not\(:disabled\),/, "icon-only: no press displacement");
});
