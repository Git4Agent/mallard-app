import { isInvitationCodeShaped } from "../../src/components/managed-cloud/RepositoryLookupField";
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
import { normalizeRepositoryName, predictRepositoryAddress } from "../../src/components/managed-cloud/repositoryAddress";

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
  settings: { service_origin: "https://mallard-ai.com", account_id: "user_1", account_login: "Rika321" },
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
    ...overrides,
  };
}

test("the New row predicts the address the service would create", () => {
  assert.equal(normalizeRepositoryName("Horror Game"), "Horror-Game");
  assert.equal(normalizeRepositoryName("  --weird__name.v2--  "), "--weird__name.v2--");
  assert.equal(normalizeRepositoryName("My--Repo"), "My--Repo");
  assert.equal(normalizeRepositoryName("///"), "repository");
  const mine = [repository({}), repository({ repository_id: "repo_2", name: "HorrorGame-2", address: "Rika321/HorrorGame-2" })];
  assert.deepEqual(predictRepositoryAddress("Rika321", "HorrorGame", mine), {
    address: "Rika321/HorrorGame-3",
    name: "HorrorGame-3",
    base: "HorrorGame",
    suffixed: true,
  });
  assert.equal(predictRepositoryAddress("Rika321", "horrorgame", mine).address, "Rika321/horrorgame-3", "names compare case-insensitively");
  assert.equal(predictRepositoryAddress("Rika321", "Birding Log", mine).address, "Rika321/Birding-Log");
  assert.equal(predictRepositoryAddress("Rika321", "My--Repo", [repository({ name: "My--Repo" })]).address, "Rika321/My--Repo-2");
  // Another owner's repository of the same name does not take the name.
  const theirs = [repository({ owner_login: "someone", address: "someone/HorrorGame" })];
  assert.equal(predictRepositoryAddress("Rika321", "HorrorGame", theirs).suffixed, false);
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
      listing={{ state: "ready", repositories: rows }}
      selection={{ kind: "new" }}
      rows="match-option"
      offerNew
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(html, /Address or invite code<\/span>/, "the code has a place before any lookup");
  assert.match(html, /placeholder="owner\/name or invite code"/);
  assert.equal(html.match(/<input/g)?.length, 1, "one field takes the address or the code");
  assert.match(html, /Look up/);
  assert.doesNotMatch(html, /Invitation code \(optional\)|Join repository/);
  assert.doesNotMatch(html, /Paste the page address/);
  assert.match(html, /class="repo-list-head">Repositories<small>3<\/small><\/div>/);
  assert.doesNotMatch(html, /read-only/i);
  assert.match(html, /Rika321\/HorrorGame<small class="sync-bundle-recommended">Git match<\/small>/);
  assert.match(html, /Private · Owner/);
  assert.match(html, /rikaqu0223-arch\/tidewatch/);
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
      listing={{ state: "ready", repositories: [] }}
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
      listing={{ state: "ready", repositories: rows }}
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
      listing={{ state: "ready", repositories: [] }}
      selection={null}
      rows="match-option"
      offerNew={false}
      mode="clone"
      disabled={false}
      onSelect={() => undefined}
    />,
  );
  assert.match(empty, /class="repo-list-head">Repositories<\/div>/);
  assert.match(empty, /<p class="repo-list-empty">No repositories yet\.<\/p>/);
  assert.doesNotMatch(empty, /radiogroup|sync-bundle-match-options/);
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
  assert.match(html, /Address or invite code/);
  assert.match(html, /Loading repositories/);
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
  assert.match(cloud, /"Contributor"/);
  assert.doesNotMatch(card, /Read-only/);
  assert.doesNotMatch(cloud, /Read-only/);
  const dialog = readFileSync("src/components/project-sync/BundleConnectionDialog.tsx", "utf8");
  assert.doesNotMatch(dialog, /read-only/i);
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
