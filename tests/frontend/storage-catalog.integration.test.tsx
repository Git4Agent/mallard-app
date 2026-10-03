import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StorageCatalogPage from "../../src/components/storage-catalog/StorageCatalogPage";
import CatalogIndexPage from "../../src/components/storage-catalog/CatalogIndexPage";
import CatalogConnectDialog from "../../src/components/storage-catalog/CatalogConnectDialog";
import { storageCatalogApi } from "../../src/components/storage-catalog/api";
import {
  actionLabel,
  catalogAddChoices,
  catalogChoiceKey,
  catalogDefaultKind,
  catalogDefaultStorage,
  catalogDoors,
  catalogKindCardFoot,
  catalogKindCards,
  catalogNoun,
  catalogPresentation,
  catalogProviderList,
  catalogScopeLabel,
  catalogScopeViews,
  catalogPullRecommendation,
  catalogStorageHint,
  catalogStorageRepositoryViews,
  filterCatalogPullReviewItems,
  rowStatus,
  rowStatusLabel,
  unambiguousProfile,
} from "../../src/components/storage-catalog/model";
import { formatRelativeTime } from "../../src/clock";
import type {
  CatalogCapabilityDescriptor,
  CatalogCloudRepository,
  CatalogProfile,
  CatalogProfileHolding,
  CatalogPullAction,
  CatalogScopeDto,
  CatalogScopePullItem,
  CatalogScopeRow,
  CatalogScopeView,
} from "../../src/components/storage-catalog/types";

const commandNames = [
  "list_catalog_cloud_repositories", "list_catalog_cloud_shared", "list_catalog_cloud_starred", "list_catalog_cloud_public", "resolve_catalog_cloud_repository", "join_catalog_cloud_repository",
  "find_catalog_cloud_repository",
  "connect_catalog_cloud_repository", "create_catalog_cloud_repository", "get_catalog_cloud_status", "request_catalog_cloud_release", "discard_catalog_cloud_draft",
  "apply_catalog_resource_fork",
  "apply_catalog_scope_pull",
  "apply_catalog_scope_push",
  "create_catalog_scope",
  "delete_catalog_scope",
  "finish_catalog_unfinished_push",
  "get_catalog_resource_preview",
  "inspect_catalog_resource_profile",
  "link_catalog_scope_storage",
  "list_catalog_capabilities",
  "list_catalog_resource_profiles",
  "list_catalog_scope_resources",
  "list_catalog_scopes",
  "list_catalog_storage_repositories",
  "open_catalog_profile_in_app",
  "open_catalog_profile_in_terminal",
  "prepare_catalog_resource_fork",
  "prepare_catalog_scope_pull",
  "prepare_catalog_scope_push",
  "probe_catalog_resource_profile",
  "rename_catalog_resource_profile",
  "save_catalog_resource_profile",
  "unlink_catalog_scope_storage",
].sort();

/* The capabilities the backend registers today, in its display order:
   provider rank first, then registration. Every label, hint, and protocol
   here is the backend's word; the frontend only draws them. */
function skills(provider: string, providerLabel: string, targets: string[]): CatalogCapabilityDescriptor {
  return {
    provider,
    provider_label: providerLabel,
    resource_kind: "agent_skill",
    format: "agentskills.io.skill",
    format_version: 1,
    label: "Skills",
    singular_label: "Skill",
    presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
    capture: "directory_tree",
    apply: "automatic",
    targets,
  };
}
function plugins(provider: string, providerLabel: string): CatalogCapabilityDescriptor {
  return {
    provider,
    provider_label: providerLabel,
    resource_kind: "plugin_install_intent",
    format: "mallard.plugin-install-intent",
    format_version: 1,
    label: "Plugins",
    singular_label: "Plugin",
    presentation: { icon: "plugin", kind_label: "Plugins", format_label: "Plugin install intent" },
    capture: "provider_inventory",
    apply: "automatic",
    targets: ["user.plugins"],
  };
}
function pets(version: number): CatalogCapabilityDescriptor {
  return {
    provider: "codex",
    provider_label: "Codex",
    resource_kind: "desktop_pet",
    format: "openai.codex-pet",
    format_version: version,
    label: `Pets v${version}`,
    singular_label: `Pet v${version}`,
    presentation: { icon: version === 1 ? "pet" : `pet-v${version}`, kind_label: "Pets", format_label: `Codex pet v${version}` },
    capture: "directory_tree",
    apply: "automatic",
    targets: ["desktop.pets"],
  };
}
const CAPABILITIES: CatalogCapabilityDescriptor[] = [
  skills("codex", "Codex", ["provider-home.skills", "shared-user.skills"]),
  plugins("codex", "Codex"),
  pets(1),
  pets(2),
  skills("claude", "Claude", ["provider-home.skills"]),
  plugins("claude", "Claude"),
];
const [CODEX_SKILLS, CODEX_PLUGINS, CODEX_PETS_V1, CODEX_PETS_V2] = CAPABILITIES;

function profile(provider: string, providerLabel: string, displayName: string, home: string): CatalogProfile {
  return {
    profile_id: "catprofile-1",
    provider,
    provider_label: providerLabel,
    display_name: displayName,
    configured_home: home,
    revision: 1,
    created_at: 1,
    updated_at: 1,
  };
}
const CONF2 = profile("codex", "Codex", "conf2", "/Users/user/codex-conf2");

/** A row as `list_catalog_scopes` serves it for a registered capability. */
function scopeDto(capability: CatalogCapabilityDescriptor, owner: CatalogProfile, scopeId = "catscope-1"): CatalogScopeDto {
  return {
    scope: {
      scope_id: scopeId,
      provider: capability.provider,
      profile_id: owner.profile_id,
      resource_kind: capability.resource_kind,
      format: capability.format,
      format_version: capability.format_version,
      revision: 1,
      created_at: 1,
      updated_at: 1,
    },
    profile: owner,
    provider_label: capability.provider_label,
    kind_label: capability.presentation.kind_label,
    format_label: capability.presentation.format_label,
    label: capability.label,
    available: true,
  };
}

/** A row whose coordinate this build does not register: the backend keeps
 * the raw keys, labels what it can, and states why. */
const UNAVAILABLE_REASON =
  "Library coordinate codex/agent_skill/agentskills.io.skill@2 is not supported by this build";
const unavailableDto: CatalogScopeDto = {
  scope: {
    scope_id: "catscope-9",
    provider: "codex",
    profile_id: CONF2.profile_id,
    resource_kind: "agent_skill",
    format: "agentskills.io.skill",
    format_version: 2,
    revision: 1,
    created_at: 1,
    updated_at: 1,
  },
  profile: CONF2,
  provider_label: "Codex",
  kind_label: "agent_skill",
  format_label: "agentskills.io.skill@2",
  label: "codex/agent_skill/agentskills.io.skill@2",
  available: false,
  unavailable_reason: UNAVAILABLE_REASON,
};

const [scopeView, unavailableScope]: CatalogScopeView[] =
  catalogScopeViews([scopeDto(CODEX_SKILLS, CONF2), unavailableDto], CAPABILITIES);

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

test("the Catalog API exposes only the scope-level command surface", () => {
  const source = readFileSync("src/components/storage-catalog/api.ts", "utf8");
  const callers = [
    readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8"),
    readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8"),
    readFileSync("src/components/storage-catalog/CatalogHomeStep.tsx", "utf8"),
    readFileSync("src/components/storage-catalog/CatalogScopeSetupStep.tsx", "utf8"),
  ].join("\n");
  const invoked = [...source.matchAll(/invoke(?:<[^>]+>)?\("([^"]+)"/g)]
    .map((match) => match[1])
    .sort();
  assert.deepEqual(invoked, commandNames);
  assert.doesNotMatch(
    source,
    /storage_catalog_binding|prepare_storage_catalog|publish_storage_catalog|apply_storage_catalog|catalog_profiles|setup_draft/,
  );
  for (const method of [
    "listCapabilities", "listProfiles", "saveProfile", "renameProfile", "probeProfile", "inspectProfile",
    "createScope", "listScopeResources", "getPreview", "linkScopeStorage",
    "unlinkScopeStorage", "openProfileApp", "openProfileTerminal",
    "preparePush", "applyPush", "finishUnfinishedPush", "preparePull", "applyPull",
    "prepareFork", "applyFork",
  ]) {
    assert.match(callers, new RegExp(`storageCatalogApi\\.${method}\\(`), `${method} has a production caller`);
  }
  // Listing and removing rows belong to the shell, because the sidebar owns
  // the menu — and the shell joins the rows to the capabilities they
  // register, since the glyph lives on the descriptor.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  for (const method of ["listScopes", "listCapabilities", "deleteScope"]) {
    assert.match(shell, new RegExp(`storageCatalogApi\\.${method}\\(`), `${method} has a production caller`);
  }
  assert.match(shell, /setCatalogScopes\(catalogScopeViews\(listed, capabilities\)\)/);
  // The storage page's repository listing is the one storage-level read: its
  // caller is the storage page, not a Catalog screen, and it makes the same
  // join.
  const storagePage = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(storagePage, /storageCatalogApi\.listStorageRepositories\(/);
  assert.match(storagePage, /catalogStorageRepositoryViews\(repositories, capabilities\)/);
});

test("a row label derives from the capability's label and the profile, covering every menu shape", () => {
  const named = (display_name: string) => ({ display_name });
  // The six rows the menu was designed around, plus the v2 sibling.
  assert.equal(catalogScopeLabel({ label: "Skills", profile: named("Codex home") }), "Skills · Codex home");
  assert.equal(catalogScopeLabel({ label: "Skills", profile: named("conf2") }), "Skills · conf2");
  assert.equal(catalogScopeLabel({ label: "Skills", profile: named("Claude home") }), "Skills · Claude home");
  assert.equal(catalogScopeLabel({ label: "Pets v1", profile: named("Codex home") }), "Pets v1 · Codex home");
  assert.equal(catalogScopeLabel({ label: "Pets v1", profile: named("conf2") }), "Pets v1 · conf2");
  assert.equal(catalogScopeLabel({ label: "Plugins", profile: named("Codex home") }), "Plugins · Codex home");
  assert.equal(catalogScopeLabel({ label: "Pets v2", profile: named("Codex home") }), "Pets v2 · Codex home");
  // Through the join: a registered row carries its descriptor, and an
  // unregistered one names itself by its raw coordinate and carries none.
  assert.equal(catalogScopeLabel(scopeView), "Skills · conf2");
  assert.equal(scopeView.capability, CODEX_SKILLS);
  assert.equal(catalogScopeLabel(unavailableScope), "codex/agent_skill/agentskills.io.skill@2 · conf2");
  assert.equal(unavailableScope.capability, null);
  // The join is by the whole coordinate: a version apart is a different
  // capability, and the storage page's repositories join the same way.
  const petRows = catalogScopeViews(
    [scopeDto(CODEX_PETS_V1, CONF2, "s1"), scopeDto(CODEX_PETS_V2, CONF2, "s2")],
    CAPABILITIES,
  );
  assert.deepEqual(petRows.map((row) => row.capability?.label), ["Pets v1", "Pets v2"]);
  const [repo] = catalogStorageRepositoryViews([{
    repository_id: "library-3a91f04c77d2b8e6a5c31f9d04b7e28c",
    provider: "claude",
    resource_kind: "agent_skill",
    format: "agentskills.io.skill",
    format_version: 1,
    provider_label: "Claude",
    kind_label: "Skills",
    format_label: "Agent Skill",
    revision: 3,
    updated_at: 1,
    resource_names: [],
    error: null,
  }], CAPABILITIES);
  assert.equal(repo.capability?.provider, "claude");
});

test("the copy's nouns come from the capability's label", () => {
  assert.equal(catalogNoun("Skills", "Skill", 1), "skill");
  assert.equal(catalogNoun("Skills", "Skill", 3), "skills");
  assert.equal(catalogNoun("Plugins", "Plugin", 0), "plugins");
  assert.equal(catalogNoun("Pets v1", "Pet v1", 1), "pet v1");
  assert.equal(catalogNoun("Pets v2", "Pet v2", 2), "pets v2");
  assert.equal(catalogNoun("Policies", "Policy", 1), "policy");
});

test("every presentation hint wears its own mark rather than borrowing one", () => {
  // All three used to borrow, and all three borrowed badly: `file` said where a
  // skill was stored rather than what it was, `tool` is the cog a tool call
  // already spends, and `openai` names the *provider* — which ProviderMark
  // paints on the same row, so as a kind glyph it said nothing at all.
  const hint = (icon: string) => ({ presentation: { icon, kind_label: "", format_label: "" } });
  assert.deepEqual(
    ["skill", "plugin", "pet", "pet-v2"].map((icon) => [icon, catalogPresentation(hint(icon))]),
    [
      ["skill", { icon: "lightbulb", artwork: false }],
      ["plugin", { icon: "catalog-plugin", artwork: false }],
      // The two pet formats are two rows, so they wear two marks; both hints
      // open the artwork column.
      ["pet", { icon: "catalog-pet-v1", artwork: true }],
      ["pet-v2", { icon: "catalog-pet-v2", artwork: true }],
    ],
  );
  assert.equal(catalogPresentation(CODEX_PETS_V1).icon, "catalog-pet-v1");
  assert.equal(catalogPresentation(CODEX_PETS_V2).icon, "catalog-pet-v2");
  // A hint this build has never heard of — a capability added later — draws
  // the Catalog glyph and opens no artwork; so does a row with no capability.
  assert.deepEqual(catalogPresentation(hint("note")), { icon: "catalog", artwork: false });
  assert.deepEqual(catalogPresentation(null), { icon: "catalog", artwork: false });
  for (const borrowed of ["file", "tool", "openai"]) {
    assert.ok(
      !["skill", "plugin", "pet", "pet-v2", "note"].some((icon) => catalogPresentation(hint(icon)).icon === borrowed),
      `no hint may fall back to \`${borrowed}\``,
    );
  }

  // The card's tile is the glyph when a row has no preview yet, so the column
  // never collapses and never repeats the provider mark beside it.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /catalog-pet-preview[\s\S]*?<Icon name="catalog-pet-v1" size=\{16\} \/>/);
  assert.doesNotMatch(page, /<Icon name="openai"/);
});

test("first-contact Catalog rows use the same storage and conflict states as sessions", () => {
  const row = (values: Partial<CatalogScopeRow>): CatalogScopeRow => ({
    row_id: "resource:review",
    display_name: "Review",
    local_present: false,
    remote_present: true,
    warnings: [],
    ...values,
  });

  const storageOnly = row({ state: "local_missing" });
  assert.equal(rowStatus(storageOnly), "cloud-outstanding");
  assert.equal(
    rowStatusLabel(storageOnly, "Vault"),
    "Only in Vault. Pull to download it here.",
  );

  const noCommonBase = row({
    state: "unreviewed_remote",
    local_present: true,
  });
  assert.equal(rowStatus(noCommonBase), "divergent");
  assert.equal(
    rowStatusLabel(noCommonBase, "Vault"),
    "Changed here and in Vault. Review before syncing.",
  );
});

test("Catalog Pull review keeps only cloud-outstanding and divergent rows", () => {
  const cases: Array<{
    id: string;
    capability: CatalogCapabilityDescriptor;
    state: NonNullable<CatalogScopeRow["state"]>;
    relation: CatalogScopePullItem["item"]["relation"];
    localPresent: boolean;
    visible: boolean;
  }> = [
    { id: "skill-cloud", capability: CODEX_SKILLS, state: "storage_changed", relation: "storage_changed", localPresent: true, visible: true },
    { id: "plugin-cloud", capability: CODEX_PLUGINS, state: "local_missing", relation: "storage_only", localPresent: false, visible: true },
    { id: "pet-v1-diverged", capability: CODEX_PETS_V1, state: "diverged", relation: "changed_both", localPresent: true, visible: true },
    { id: "pet-v2-diverged", capability: CODEX_PETS_V2, state: "unreviewed_remote", relation: "storage_only", localPresent: true, visible: true },
    { id: "matching", capability: CODEX_SKILLS, state: "in_sync", relation: "matching", localPresent: true, visible: false },
    { id: "local", capability: CODEX_PLUGINS, state: "local_changed", relation: "local_changed", localPresent: true, visible: false },
    { id: "blocked", capability: CODEX_PETS_V1, state: "blocked", relation: "blocked", localPresent: true, visible: false },
  ];
  const items: CatalogScopePullItem[] = cases.map((candidate) => ({
    item: {
      resource_id: candidate.id,
      provider: candidate.capability.provider,
      resource_kind: candidate.capability.resource_kind,
      format: candidate.capability.format,
      format_version: candidate.capability.format_version,
      provider_label: candidate.capability.provider_label,
      kind_label: candidate.capability.presentation.kind_label,
      format_label: candidate.capability.presentation.format_label,
      display_name: candidate.id,
      relation: candidate.relation,
      disposition: { kind: "executable" },
      actions: [],
    },
    conflicting_claims: [],
  }));

  assert.deepEqual(
    filterCatalogPullReviewItems(items).map((item) => item.item.resource_id),
    cases.filter((candidate) => candidate.visible).map((candidate) => candidate.id),
  );
});

test("Library Pull's Select counts and restores only default actions", () => {
  // The header's Select used to count every visible item while ticking only
  // those with a default action, and stayed live once they were ticked.
  const pullItem = (id: string, defaultAction: string | null): CatalogScopePullItem => ({
    item: {
      resource_id: id,
      provider: CODEX_SKILLS.provider,
      resource_kind: CODEX_SKILLS.resource_kind,
      format: CODEX_SKILLS.format,
      format_version: CODEX_SKILLS.format_version,
      provider_label: CODEX_SKILLS.provider_label,
      kind_label: CODEX_SKILLS.presentation.kind_label,
      format_label: CODEX_SKILLS.presentation.format_label,
      display_name: id,
      relation: "storage_changed",
      disposition: { kind: "executable" },
      actions: [],
      default_action_id: defaultAction,
    },
    conflicting_claims: [],
  });
  const items = [pullItem("update", "update-action"), pullItem("install", "install-action"), pullItem("choose", null)];
  const defaults = new Set(["update", "install"]);
  const choices = { update: "update-action", install: "install-action" };

  const opened = catalogPullRecommendation(items, defaults, choices);
  assert.deepEqual(opened.items.map((item) => item.item.resource_id), ["update", "install"]);
  assert.equal(opened.matches, true, "the defaults the review opens with are already the recommendation");

  // Unticking a default, ticking the item without one, or choosing another
  // action for a ticked default each leaves something for Select to restore.
  assert.equal(catalogPullRecommendation(items, new Set(["update"]), choices).matches, false);
  assert.equal(catalogPullRecommendation(items, new Set([...defaults, "choose"]), choices).matches, false);
  assert.equal(catalogPullRecommendation(items, defaults, { ...choices, update: "overwrite-action" }).matches, false);
  // A choice left behind on an unticked item is not a selection.
  assert.equal(catalogPullRecommendation(items, defaults, { ...choices, choose: "overwrite-action" }).matches, true);
  // With no defaults there is nothing to select.
  assert.deepEqual(catalogPullRecommendation([pullItem("choose", null)], new Set(), {}).items, []);
});

const STORAGES = [
  { id: "storage-a", name: "Vault", provider: "local", provider_label: "Local folder", icon: "drive", detail: "~/Vault", detailIsPath: true },
];

test("the Catalog index is a stable root with empty and populated states", () => {
  const opened: Array<string | undefined> = [];
  const empty = renderToStaticMarkup(
    <CatalogIndexPage
      scopes={[]}
      capabilities={CAPABILITIES}
      busy={false}
      appActions={[]}
      onOpenScope={() => undefined}
      onAddScope={(choiceKey) => opened.push(choiceKey)}
    />,
  );
  assert.match(empty, /aria-label="Catalog path"/);
  assert.match(empty, />Catalog</);
  // Empty, the page is the filled landing ghosted: the head with its count
  // and verb, then one dashed door per shape the registry offers — the
  // chooser's choices in the chooser's order, wearing the shape's glyph and
  // the marks of the providers registered for it — and one line of purpose.
  assert.match(empty, /landing-picker-head[\s\S]*?0 on this machine[\s\S]*?class="btn btn-primary"[^>]*>[\s\S]*?Add resource…/);
  assert.doesNotMatch(empty, /No Catalog resources yet|Add Catalog resource…|mallard-logo-duck/);
  const doors = [...empty.matchAll(/class="landing-picker-door" aria-label="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(doors, ["Add Skills", "Add Plugins", "Add Pets v1", "Add Pets v2"]);
  assert.match(empty, /aria-label="Add Skills"[\s\S]*?data-icon="lightbulb"[\s\S]*?<strong>Skills<\/strong>[\s\S]*?data-icon="plus"[\s\S]*?sync-provider-mark codex[\s\S]*?sync-provider-mark claude/);
  assert.match(empty, /aria-label="Add Plugins"[\s\S]*?data-icon="catalog-plugin"/);
  assert.match(empty, /aria-label="Add Pets v1"[\s\S]*?data-icon="catalog-pet-v1"/);
  assert.match(empty, /landing-picker-caption"[\s\S]*?data-icon="cloud"[\s\S]*?Shared through your storages/);
  assert.doesNotMatch(empty, /landing-picker-card"/);

  // Before the registry answers, the one door is the chooser itself.
  const unanswered = renderToStaticMarkup(
    <CatalogIndexPage
      scopes={[]}
      capabilities={[]}
      busy={false}
      appActions={[]}
      onOpenScope={() => undefined}
      onAddScope={() => undefined}
    />,
  );
  assert.equal((unanswered.match(/class="landing-picker-door"/g) ?? []).length, 1);
  assert.match(unanswered, /class="landing-picker-door" aria-label="Add resource"[\s\S]*?data-icon="catalog"/);

  const populated = renderToStaticMarkup(
    <CatalogIndexPage
      scopes={[scopeView]}
      capabilities={CAPABILITIES}
      busy={false}
      appActions={[]}
      onOpenScope={() => undefined}
      onAddScope={() => undefined}
    />,
  );
  assert.match(populated, /landing-picker-card"/);
  assert.doesNotMatch(populated, /landing-picker-door|landing-picker-caption/);
  // The door is named by the scope label; the card itself shows the
  // capability's label, the profile, and the profile's home as its location.
  assert.match(populated, /aria-label="Open Skills · conf2"/);
  assert.match(populated, /<strong>Skills<\/strong>/);
  assert.match(populated, /data-icon="lightbulb"/);
  assert.match(populated, /landing-picker-card-location">~\/codex-conf2</);
  assert.doesNotMatch(populated, /sync-storage-index-row/);
});

function renderScopePage(scope: CatalogScopeView = scopeView) {
  return renderToStaticMarkup(
    <StorageCatalogPage
      scope={scope}
      scopes={[scopeView, unavailableScope]}
      storages={STORAGES}
      capabilities={CAPABILITIES}
      appActions={[]}
      onOpenScope={() => undefined}
      onOpenStorageDetails={() => undefined}
      onAddStorage={() => undefined}
      onAddScope={() => undefined}
      onRemoveScope={() => undefined}
      onScopeGone={() => undefined}
      onProfileRenamed={() => undefined}
      notify={() => undefined}
    />,
  );
}

test("an unavailable row states the backend's reason and offers no link, push, or pull", () => {
  const html = renderScopePage(unavailableScope);
  // The reason is the backend's sentence, standing on the page, and the row
  // still names itself by its raw coordinate in the trail.
  assert.match(html, new RegExp(escapeRegExp(UNAVAILABLE_REASON)));
  assert.match(html, /codex\/agent_skill\/agentskills\.io\.skill@2 · conf2 can’t be used in this build\./);
  assert.match(html, /aria-label="Why this Catalog row is unavailable"/);
  // Both verbs are off, and each carries the reason as its title.
  const verbs = html.match(
    new RegExp(`<button[^>]*class="sync-path-sync"[^>]*disabled=""[^>]*title="${escapeRegExp(UNAVAILABLE_REASON)}"`, "g"),
  );
  assert.equal(verbs?.length, 2, "Pull and Push are disabled with the reason");
  // No list, no empty-state sentence: nothing was asked of the home.
  assert.doesNotMatch(html, /sync-history-state|sync-history-section-heading|catalog-card/);
  // A row this build does not register wears the Catalog glyph, not a
  // borrowed one.
  assert.match(html, /data-icon="catalog"/);

  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The page never scans the home or links the storage for such a row: the
  // loader returns before the resource read, and the one place a link is
  // made refuses first.
  assert.match(page, /if \(!scope\.available\) \{\s*setLoading\(false\);\s*return;\s*\}[\s\S]*?storageCatalogApi\.listScopeResources\(/);
  assert.match(page, /if \(!scope\.available\) throw new Error\(unavailableReason\);[\s\S]*?storageCatalogApi\.linkScopeStorage\(/);
  assert.match(page, /const syncDisabled = busy \|\| !storageId \|\| loading \|\| unavailable \|\| cloudClosedReason !== null\s*\|\| \(managedCloud && mode\.kind !== "list"\);/);
  assert.match(page, /const syncDisabledReason = unavailable\s*\? unavailableReason/);

  // The index card says the same thing where its profile fact goes.
  const index = renderToStaticMarkup(
    <CatalogIndexPage
      scopes={[unavailableScope]}
      capabilities={CAPABILITIES}
      busy={false}
      appActions={[]}
      onOpenScope={() => undefined}
      onAddScope={() => undefined}
    />,
  );
  assert.match(index, new RegExp(`conf2 — ${escapeRegExp(UNAVAILABLE_REASON)}`));
});

test("the Add chooser lists the backend's capabilities in its order and records the chosen coordinate", async () => {
  // Descriptors group by portable shape — kind, format, version — in the
  // order the backend first lists each, with the providers registered for it
  // in the backend's order. That is the four-way Kind choice the page has
  // always drawn, read off the registry instead of a table.
  const choices = catalogAddChoices(CAPABILITIES);
  assert.deepEqual(
    choices.map((choice) => [choice.label, choice.providers.map((capability) => capability.provider)]),
    [
      ["Skills", ["codex", "claude"]],
      ["Plugins", ["codex", "claude"]],
      ["Pets v1", ["codex"]],
      ["Pets v2", ["codex"]],
    ],
  );
  assert.equal(catalogProviderList(["Codex", "Claude"]), "Codex and Claude");
  assert.equal(catalogProviderList(["Codex"]), "Codex only");
  assert.equal(catalogProviderList(["Codex", "Claude", "Other"]), "Codex, Claude and Other");
  // A capability this build registers later — a new provider, kind, format —
  // lists itself with no frontend change.
  const note: CatalogCapabilityDescriptor = {
    provider: "testprov",
    provider_label: "Test provider",
    resource_kind: "test_note",
    format: "mallard.test-note",
    format_version: 1,
    label: "Notes",
    singular_label: "Note",
    presentation: { icon: "note", kind_label: "Notes", format_label: "Test note" },
    capture: "directory_tree",
    apply: "manual",
    targets: ["provider-home.notes"],
  };
  assert.deepEqual(
    catalogAddChoices([...CAPABILITIES, note]).map((choice) => choice.label),
    ["Skills", "Plugins", "Pets v1", "Pets v2", "Notes"],
  );

  // Add offers that same list as the Catalog landing's doors — the choice
  // plus the glyph of the shape it names — so the door you came through is
  // still on screen as the choice it made.
  assert.deepEqual(
    catalogDoors(CAPABILITIES).map((door) => [door.label, door.icon]),
    [["Skills", "lightbulb"], ["Plugins", "catalog-plugin"], ["Pets v1", "catalog-pet-v1"], ["Pets v2", "catalog-pet-v2"]],
  );

  // The setup page draws the same shapes as cards — the capabilities the
  // home's provider registers, in the registry's order — and Finish records
  // the one card chosen, on the one home chosen a step earlier.
  const codexHome = { profile_id: "catprofile-1", provider: "codex" };
  assert.deepEqual(
    catalogKindCards(CAPABILITIES, codexHome, [], null).map((card) => [card.capability.label, card.key]),
    [
      ["Skills", catalogChoiceKey(CODEX_SKILLS)],
      ["Plugins", catalogChoiceKey(CODEX_PLUGINS)],
      ["Pets v1", catalogChoiceKey(CODEX_PETS_V1)],
      ["Pets v2", catalogChoiceKey(CODEX_PETS_V2)],
    ],
  );
  assert.deepEqual(
    catalogKindCards(CAPABILITIES, { profile_id: "catprofile-3", provider: "claude" }, [], null).map((card) => card.capability.label),
    ["Skills", "Plugins"],
  );
  const setup = readFileSync("src/components/storage-catalog/CatalogScopeSetupStep.tsx", "utf8");
  assert.match(setup, /const cards = catalogKindCards\(capabilities, profile, scopes, inspection\?\.holdings \?\? null\);/);
  assert.match(setup, /role="radiogroup" aria-label="Resource kind"[\s\S]*?cards\.map\(\(candidate\) => \{/);
  assert.match(setup, /className=\{`setup-choice-card\$\{active \? " active" : ""\}`\}/);
  assert.match(setup, /<Icon name=\{catalogPresentation\(candidate\.capability\)\.icon\} size=\{24\} \/>/);
  assert.match(setup, /\{candidate\.capability\.label\}/);
  assert.match(setup, /\{catalogKindCardFoot\(candidate\)\}/);
  assert.match(setup, /storageCatalogApi\.createScope\(card\.capability, profile\.profile_id\)/);

  // And the call sends the four coordinate fields under Tauri's camelCase
  // keys — captured at the bridge, where the desktop app would receive it.
  const calls: Array<{ command: string; args: unknown }> = [];
  const host = globalThis as { isTauri?: boolean; window?: unknown };
  const previous = { isTauri: host.isTauri, window: host.window };
  host.isTauri = true;
  host.window = {
    __TAURI_INTERNALS__: {
      invoke: async (command: string, args: unknown) => {
        calls.push({ command, args });
        return {};
      },
    },
  };
  try {
    await storageCatalogApi.createScope(CODEX_PETS_V2, "catprofile-1");
  } finally {
    host.isTauri = previous.isTauri;
    host.window = previous.window;
  }
  assert.deepEqual(calls, [{
    command: "create_catalog_scope",
    args: {
      provider: "codex",
      profileId: "catprofile-1",
      resourceKind: "desktop_pet",
      format: "openai.codex-pet",
      formatVersion: 2,
    },
  }]);
});

test("stored-only rows lead the scope page in their own plain section", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // A1: the partition and nothing else. Stored-only rows lift into a section
  // headed "Only in <storage>" above the capability's own list — the same
  // heading shape, the same cards, no counts, no fold, no new CSS. A row
  // missing on both sides stays in the capability's section: "not in the
  // storage" would be the wrong heading for it.
  assert.match(page, /const cloudRows = matchingRows\.filter\(\(row\) => !row\.local_present && row\.remote_present\)/);
  assert.match(page, /const localRows = matchingRows\.filter\(\(row\) => row\.local_present \|\| !row\.remote_present\)/);
  assert.match(
    page,
    /Only in <span className="sync-history-storage-name">\{storageName \?\? "storage"\}<\/span>[\s\S]*<h2><Icon name=\{glyph\.icon\} size=\{14\} \/>\{scope\.label\}<\/h2>/,
    "the cloud section renders above the capability's own section",
  );
  // Both sections draw the same rows through one renderer, so the two halves
  // cannot drift apart card by card.
  assert.match(page, /\{cloudRows\.map\(renderRow\)\}/);
  assert.match(page, /\{localRows\.map\(renderRow\)\}/);
});

test("the scope page wears the project workspace's trail and list", () => {
  const html = renderScopePage();
  // The project workspace's own two crumbs — the storage it syncs with, then
  // the thing being synced. No Storages crumb: that roots the storage
  // screens, not a workspace. The page never names itself twice, so there is
  // no <h1> over the list.
  assert.doesNotMatch(html, /Storages/);
  assert.match(html, /Skills · conf2/);
  assert.doesNotMatch(html, /<h1>Skills · conf2<\/h1>/);
  // The first load is the rows scaffold, named with the same wording the
  // old line carried. Its 150ms gate means a static render shows nothing —
  // that blank IS the recipe's flash-free warm path, so the named state is
  // pinned at the source instead of in this markup.
  assert.doesNotMatch(html, /sync-history-state/);
  assert.doesNotMatch(html, /(?:Library|Catalog) storage:|Choose another storage|Pair a .* profile/);
  assert.doesNotMatch(html, /Add Skill|Add Plugin|Add Pet\b/);
  // Browsing a storage from a select is gone: the storage is the page's outer
  // scope, chosen in the bar.
  assert.doesNotMatch(html, /Browse stored resources|<select/);
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // `rows`, not a card grid: the landed list is the same thread-card family
  // Activity lands, and the scaffold must draw the page's own destination.
  assert.match(page, /<LoadingScaffold shape="rows" label=\{`Loading \$\{label\}`\} \/>/);
  // An initial load error belongs only to the Retry panel below the notices;
  // later action errors remain in the page notice zone.
  assert.match(page, /if \(error && page\) \{\s*pageNotices\.push/);
  assert.match(page, /\) : error && !page \? \(/);
  const scaffold = readFileSync("src/components/LoadingScaffold.tsx", "utf8");
  // The recipe's contract: a quiet named region, bones hidden from
  // assistive tech, the caption naming the noun, and the 150ms gate.
  assert.match(scaffold, /role="region"/);
  assert.match(scaffold, /aria-hidden="true"/);
  assert.match(scaffold, /\{label\}…/);
  assert.match(scaffold, /setPastGate\(true\), 150\)/);
  assert.match(page, /storageCatalogApi\.listScopeResources\(scope\.scope\.scope_id, chosenStorageId\)/);
  assert.doesNotMatch(page, /CatalogCategory|CATALOG_CATEGORIES|family/);
});

test("every row is the activity card: badge, name as the door, facts, one ⓘ", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  const model = readFileSync("src/components/storage-catalog/model.ts", "utf8");
  // The badge is the project's own indicator, so the two pages answer the
  // same question with the same glyph.
  assert.match(page, /ThreadStatusIcon status=\{status\} size=\{18\}/);
  assert.match(page, /className=\{`sync-thread-sync-indicator \$\{TONE\[status\] \?\? "unknown"\}`\}/);
  // Every badge maps to a state the backend derives; none is decorative.
  for (const state of [
    "in_sync", "new", "local_changed", "remote_missing",
    "storage_changed", "local_missing", "diverged", "unreviewed_remote", "blocked",
  ]) {
    assert.match(model, new RegExp(`case "${state}":`), `${state} has a badge`);
  }
  // The card is the activity card. The name is plain text — the per-resource
  // detail page it used to open went with the per-resource links.
  assert.match(page, /sync-history-thread-card catalog-card/);
  assert.doesNotMatch(
    page,
    /catalog-card-name|chevron-right|catalog-row-name|catalog-row-place|catalog-row-stamp|catalog-row-open|catalog-review-row|catalog-row-list/,
  );
  assert.doesNotMatch(page, /kind: "detail"|catalog-link-card|inspectLinks|registration_id/);
  // Line two states what the scan measured — files, bytes on disk — never a
  // recount of its own.
  assert.match(page, /row\.local_files/);
  assert.match(page, /row\.local_bytes/);
  // The ⓘ carries the row's paths and digests in the session card's own
  // popover, each a click to copy.
  assert.match(page, /sync-history-debug-popover/);
  assert.match(page, /sha256:\$\{row\.content_sha256\}/);
  assert.match(page, /aria-controls=\{menuId\}/);
  assert.match(page, /closeMenu\(true\)/);
  assert.match(page, /document\.addEventListener\("focusin", onFocusIn\)/);
  assert.match(page, /role="status" aria-live="polite"/);
  assert.match(page, /Could not copy\./);
  // Reveal in Finder stays off when nothing is local.
  assert.match(page, /revealItemInDir/);
  assert.match(page, /disabled=\{!row\.local_path\}/);
  // The provider mark is a launch button now — the session card's pair,
  // profile-scoped: the mark opens the desktop app on this profile's home,
  // and the terminal button opens the provider CLI there. Both name the
  // action with the backend's provider label, carry no inert span, and
  // report failures on the page surface.
  assert.doesNotMatch(page, /sync-history-provider-inert/);
  assert.match(page, /title=\{`Open in \$\{providerLabel\}`\}[\s\S]{0,200}?<ProviderMark provider=\{provider\} displayName=\{providerLabel\}/);
  assert.match(page, /providerLabel=\{profile\.provider_label\}/);
  assert.match(page, /aria-label="Open in Terminal"[\s\S]{0,120}?<Icon name="terminal" size=\{14\}/);
  assert.match(page, /storageCatalogApi\.openProfileApp\(profile\.profile_id\)/);
  assert.match(page, /storageCatalogApi\.openProfileTerminal\(profile\.profile_id\)/);
  // Acknowledgements are the shared Acknowledgement inside the card that
  // raised them, both at the Warning rung — an overwrite has not failed, so
  // it is not an Error; its verb carries the risk. The box spans the card's
  // full row — auto-placement dropped a second one into the grid's
  // auto-sized actions track and inflated it to the whole sentence (the
  // sheared-chips screenshot); the pet template starts the span after its
  // preview column instead.
  assert.equal(page.match(/<Acknowledgement\b/g)?.length, 2);
  assert.doesNotMatch(page, /AckChip|catalog-card-ack|danger\s*\n\s*lead=/);
  assert.match(
    readFileSync("src/App.css", "utf8"),
    /\.catalog-card > \.notice-ack \{[^}]*grid-column: 1 \/ -1;/,
  );
  assert.match(
    readFileSync("src/App.css", "utf8"),
    /\.catalog-card-pet > \.notice-ack \{[^}]*grid-column: 2 \/ -1;/,
  );
  // Payload first: the bold lead derives from warning.code with the raw
  // backend sentence as the fallback for unknown codes, and the ack verb is
  // the chip's trailing word — never the old constant preamble, whose text
  // the accessible name still carries in full.
  assert.match(page, /WARNING_LEADS/);
  assert.match(page, /warningParts\(warning\)/);
  assert.match(page, /aria-label=\{`Reviewed: \$\{warning\.message\}`\}|ariaLabel=\{`Reviewed: \$\{warning\.message\}`\}/);
  assert.match(page, /verbs=\{\["Mark reviewed", "Reviewed"\]\}/);
  assert.match(page, /verbs=\{\["Approve overwrite", "Approved"\]\}/);
  assert.doesNotMatch(page, /Reviewed this warning — |I understand this replaces/);
  // The count rides the verb at the point of commitment; on Mallard Cloud
  // Push says only Push, as a project's does.
  assert.match(page, /`Push \$\{pushSelected\.size\}`/);
  assert.match(page, /`Pull \$\{pullReviewSelectedCount\}`/);
});

test("an empty Catalog review's footer holds only the way back to the page it closes onto", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The exit takes the commit's slot: the footer's only button, a box rather
  // than the gold or a ghost, named for the page it lands on: the item's, or
  // the one headed by the scope's label.
  for (const [empty, verb] of [["selectableRows", "Push"], ["reviewItems", "Pull"]] as const) {
    const footer = page.match(new RegExp(
      `<footer className="sync-inline-action-footer sync-sync-review-footer">\\s*\\{${empty}\\.length === 0 \\? \\([\\s\\S]*?</footer>`,
    ))?.[0];
    assert.ok(footer, `${verb} footer branches on an empty review`);
    const [exitBranch, commitBranch] = footer.split(") : (");
    assert.match(exitBranch, /className="btn sync-review-exit"/);
    assert.match(exitBranch, new RegExp(`title=\\{\`Close ${verb} review — back to \\$\\{itemRow\\?\\.display_name \\?\\? scope\\.label\\}\`\\}`));
    assert.match(exitBranch, /<span>\{`Back to \$\{itemRow\?\.display_name \?\? scope\.label\}`\}<\/span>/);
    assert.match(exitBranch, /onClick=\{\(\) => setMode\(\{ kind: "list" \}\)\}/);
    assert.doesNotMatch(exitBranch, /btn-primary|btn-ghost|chevron-left/);
    assert.match(commitBranch, new RegExp(`${verb} \\$\\{`));
  }
  // "Library" is not a name the app shows; the sidebar and index say Catalog.
  assert.doesNotMatch(page, /Back to Library/);
});

test("rows of a capability with artwork wear the preview tile, and stored-only previews arrive lazily", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Artwork is a presentation fact of the capability, read off its hint:
  // those cards open a preview column — image when a preview exists, the
  // glyph when nothing can be drawn — and no other card grows the column.
  assert.match(page, /const glyph = catalogPresentation\(scope\.capability\);/);
  assert.match(page, /const artwork = glyph\.artwork;/);
  assert.match(page, /catalog-card-pet/);
  assert.match(page, /catalog-pet-preview/);
  assert.match(page, /artwork \? row\.preview_png \?\? remotePreviews\[row\.row_id\] \?\? null : undefined/);
  // The tile's sentence is the codec's own description of the row.
  assert.match(page, /data-tooltip=\{row\.description \?\? undefined\}/);
  // Stored-only rows ask after render, once per page, and a failure keeps
  // the glyph — the list call itself never waits on a storage round-trip.
  assert.match(page, /if \(!artwork \|\| !storageId\) return;/);
  assert.match(page, /storageCatalogApi\.getPreview\(scopeId, storageId, row\.resource_id!\)/);
  assert.match(page, /previewAttempts/);
  assert.doesNotMatch(page, /getPreview[\s\S]{0,400}listScopeResources\(/);
  // The tile and the card template share one stylesheet.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.catalog-pet-preview \{/);
  assert.match(
    css,
    /\.catalog-card-pet \{[^}]*"preview title actions"[^}]*"preview meta meta"/,
  );
});

test("Push and Pull are page verbs with one review each, and link on the way", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The bar owns both verbs, with the count the list itself produced.
  assert.match(page, /sync-path-sync[\s\S]*Pull<\/span>[\s\S]*counts\.pull/);
  assert.match(page, /sync-path-sync[\s\S]*Push<\/span>[\s\S]*counts\.push/);
  assert.match(page, /counts = useMemo\(\(\) => barRows\.reduce/);
  // Push and Pull create the one scope-storage pairing they need; the user
  // never links by hand, and each review consumes exactly one plan.
  assert.match(page, /ensureLink[\s\S]*storageCatalogApi\.linkScopeStorage/);
  assert.match(page, /await ensureLink\(\);[\s\S]*storageCatalogApi\.preparePush\(scope\.scope\.scope_id, storageId\)/);
  assert.match(page, /storageCatalogApi\.applyPush\(pushPlan\.plan_id, \[\.\.\.pushSelected\], \[\.\.\.pushAcks\],/);
  assert.match(page, /await ensureLink\(\);[\s\S]*storageCatalogApi\.preparePull\(scope\.scope\.scope_id, storageId\)/);
  assert.match(page, /storageCatalogApi\.applyPull\(pullPlan\.plan_id, choices, \[\.\.\.pullAcks\]/);
  assert.match(page, /for \(const \{ item \} of filterCatalogPullReviewItems\(pullPlan\.items\)\)/);
  // The reviews are the project's own review chrome.
  assert.match(page, /SyncReviewSelectionBar/);
  assert.match(page, /sync-inline-push-review sync-sync-review-workspace/);
  assert.match(page, /sync-inline-pull-review sync-sync-review-workspace/);
  // Pull operations speak the files review's chip grammar — "Install skill",
  // "Update pet v1 · keeps local copy" — one voice across every pull review,
  // with the noun read off the capability's label.
  assert.match(page, /actionChipLabel\(action\.chosen, kindNoun\)/);
  assert.match(page, /keeps local copy/);
  assert.match(page, /kindNoun=\{noun\(1\)\}/);
  // The scroll owns the gutter under the header. A catalog review's first
  // child is a progress notice, a banner or row one, and only the banner ever
  // carried a top margin — and only by collapsing out of the scroll — so a
  // review that opened without one stood its first card's border on the
  // header's own hairline.
  const reviewCss = readFileSync("src/App.css", "utf8");
  assert.match(
    reviewCss,
    /\.sync-sync-review-scroll:has\(> \.sync-history-thread-list\) \{[^}]*padding-top: 8px;/,
  );
  assert.match(
    reviewCss,
    /\.sync-sync-review-scroll:has\(> \.sync-history-thread-list\) > \.sync-review-banners \{[^}]*margin-top: 0;/,
  );
  // A local action is a readout plus the app's own popover, never a select.
  assert.match(page, /className="sync-path-menu end"\s*\n\s*role="menu"/);
  assert.doesNotMatch(page, /<select/);
  // And the popover keeps the keyboard contract that role implies.
  assert.match(page, /moveMenuFocus\(event\.currentTarget, event\.key\)/);
  assert.match(page, /focusMenuEdge\(node, "first"\)/);
});

test("the active path-bar Push commits the reviewed selection without rebuilding it", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");

  assert.match(
    page,
    /onClick=\{\(\) => void \(mode\.kind === "push" \? runPush\(\) : openPush\(\)\)\}/,
  );
  assert.match(
    page,
    /disabled=\{mode\.kind === "push"\s*\? pushReviewApplyDisabled\s*:\s*syncDisabled \|\| pushRows\.length === 0\}/,
  );
  assert.match(
    page,
    /const pushReviewApplyDisabled = busy\s*\|\| pushSelected\.size === 0\s*\|\| pushReviewUnacknowledged;/,
  );
  assert.match(page, /disabled=\{pushReviewApplyDisabled\}\s*onClick=\{\(\) => void runPush\(\)\}/);
  assert.match(page, /mode\.kind === "push"[\s\S]*?sync-path-count[\s\S]*?pushSelected\.size/);
});

test("the active path-bar Pull commits only the visible reviewed selection", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");

  assert.match(
    page,
    /onClick=\{\(\) => void \(mode\.kind === "pull" \? runPull\(\) : openPull\(\)\)\}/,
  );
  assert.match(
    page,
    /disabled=\{mode\.kind === "pull"\s*\? pullReviewApplyDisabled\s*:\s*syncDisabled \|\| pullRows\.length === 0\}/,
  );
  assert.match(
    page,
    /const pullReviewApplyDisabled = busy\s*\|\| pullReviewSelectedCount === 0\s*\|\| pullReviewBlocked;/,
  );
  assert.match(page, /disabled=\{pullReviewApplyDisabled\}\s*onClick=\{\(\) => void runPull\(\)\}/);
  assert.match(page, /mode\.kind === "pull"[\s\S]*?sync-path-count[\s\S]*?pullReviewSelectedCount/);
});

test("Push review shows only local-outstanding rows, whatever their capability", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");

  assert.match(page, /const selectableRows = planRows\.filter\(\(row\) => row\.row\.selectable\);/);
  assert.match(page, /const visibleRows = selectableRows\.filter/);
  assert.doesNotMatch(page, /row\.row\.selectable \|\| row\.row\.relation === "matching"/);
  assert.match(page, /total=\{selectableRows\.length\}/);
  assert.match(page, /onRecommended=\{\(\) => setPushSelected\(\(current\) => replaceSearchSelection\(current, matchingIds/);
  assert.match(page, /\{visibleRows\.map\(\(planRow\) => \{/);
  assert.match(page, /selectable=\{planRow\.row\.selectable\}/);
  assert.match(page, /onToggle=\{planRow\.row\.selectable \? \(\) => setPushSelected/);
  assert.match(page, /acks=\{planRow\.row\.selectable && pushSelected\.has\(resourceId\)/);
  assert.match(page, /\{!searchActive && selectableRows\.length === 0 && \(/);
  // The list page still draws every row — now through the shared renderer
  // that feeds both of its sections.
  assert.match(page, /\{cloudRows\.map\(renderRow\)\}[\s\S]*\{localRows\.map\(renderRow\)\}/);
});

test("Add is New project's two steps: the agent home as the project's Agent band, then that home's setup page", () => {
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  const home = readFileSync("src/components/storage-catalog/CatalogHomeStep.tsx", "utf8");
  const setup = readFileSync("src/components/storage-catalog/CatalogScopeSetupStep.tsx", "utf8");

  // New project's two doors, at the top of step one.
  assert.match(add, /title="On this computer"[\s\S]*title="From Mallard Cloud"/);
  assert.match(add, /className="storage-mode-doors" role="radiogroup"/);
  // From Mallard Cloud is New project's bands: the account, the repository
  // (its kind comes with it), then the row it lands in.
  const cloud = ["Storage", "Repository", "Catalog row"].map((label) => add.indexOf(`<strong>${label}</strong>`));
  assert.ok(cloud.every((index) => index >= 0));
  assert.deepEqual(cloud, [...cloud].sort((left, right) => left - right));
  assert.match(add, /No Mallard Cloud account yet\./);
  assert.match(add, /Can’t be changed later\./);
  assert.match(add, /storageCatalogApi\.connectCloudRepository\(/);
  assert.doesNotMatch(add + home + setup, /preparePull|applyPull|preparePush|applyPush/);

  // Step one is the project's Agent band: one card per registered provider,
  // the mark and name on the toggle line, and under the chosen card the
  // project's profile row — saved profiles and Custom path… in one select,
  // the inline path, Browse, and the hint the project writes. One row per
  // pass, so the toggle is a radio.
  assert.match(home, /className="sync-provider-cards" role="radiogroup"/);
  assert.match(home, /className="sync-provider-toggle setup-marked-toggle"/);
  assert.match(home, /type="radio"\s+name="catalog-home-provider"/);
  assert.match(home, /sync-agent-profile-row/);
  assert.match(home, /<option value="">Choose profile<\/option>/);
  assert.match(home, /<option value="__pending">Custom path…<\/option>/);
  assert.match(home, /<Icon name="folder" size=\{13\} \/> Browse/);
  assert.match(home, /from "\.\.\/\.\.\/profilePath"/);
  assert.match(home, /profilePathHint\(probe, selection\.path, homePath, \{ display_name: label \}, profiles\)/);
  assert.match(home, /storageCatalogApi\.probeProfile\(provider, path\)/);
  // The project's preselection rule, verbatim: one profile for the provider
  // is chosen, none or several wait on "Choose profile" and the project's
  // field message.
  assert.match(home, /const only = unambiguousProfile\(profiles, key\);/);
  assert.match(home, /Choose a profile for \{label\}\./);
  // The button names the step it opens, from the saved name or the one the
  // probe would give a typed folder.
  assert.match(home, /homeName \? `Set up \$\{homeName\}` : "Set up"/);
  assert.match(home, /resolvedProbe\?\.suggested_name/);

  // Step two is the project's setup page for that home: Home, one kind as
  // the shipped cards, Storage as New project's pill row, Finish.
  const bands = ["Home", "Resource", "Storage"].map((label) => setup.indexOf(`<strong>${label}</strong>`));
  assert.ok(bands.every((index) => index >= 0), "all three bands render");
  assert.deepEqual(bands, [...bands].sort((left, right) => left - right));
  assert.match(setup, /storageCatalogApi\.inspectProfile\(profile\.profile_id\)/);
  assert.match(setup, /catalogDefaultKind\(cards, initialChoiceKey\)/);
  assert.match(setup, /catalogDefaultStorage\(storages, inspection\?\.last_used_storage_id\)/);
  assert.match(setup, /label="None yet"/);
  assert.match(setup, /storageIconName\(candidate\.provider, candidate\.icon\)/);
  assert.match(setup, /chosenStorage \? "Finish" : "Finish locally"/);
  // Finish makes exactly one row and links it once; a link that fails after
  // the row was made leaves the row standing and says so.
  assert.equal((setup.match(/storageCatalogApi\.createScope\(/g) ?? []).length, 1);
  assert.equal((setup.match(/storageCatalogApi\.linkScopeStorage\(/g) ?? []).length, 1);
  assert.match(setup, /onFinished\(created, null, errorMessage\(reason\)\)/);
  assert.match(setup, /if \(!storage\) \{\s*onFinished\(created, null, null\);/);
  // The shell lands on the row with the storage it was linked to.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /if \(storageId\) setCatalogPreferredStorage\(\(current\) => \(\{ \.\.\.current, \[scope\.scope\.scope_id\]: storageId \}\)\);/);
  assert.match(shell, /storages=\{catalogStorages\}/);

  // Both steps wear the project-setup language, and the Kind is never a
  // table of the frontend's own.
  for (const source of [add, home, setup]) {
    assert.doesNotMatch(source, /CATALOG_KINDS|Codex only|Codex and Claude/);
  }
  assert.match(add, /sync-inline-new-project sync-setup-workspace/);
  assert.match(setup, /sync-inline-new-project sync-setup-workspace/);
  assert.match(home, /sync-setup-section/);
  assert.match(home, /sync-setup-footer/);
  assert.match(setup, /sync-setup-footer/);
  assert.doesNotMatch(add + home + setup, /Add profile|className="catalog-inline-form"|Manage saved homes|removeProfile|storageCatalogApi\.deleteProfile/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-setup-(grid|card|step)/);
});

test("Add's setup page chooses by the project's rules: one unambiguous profile, the found kind, the preferred storage", () => {
  const codex = profile("codex", "Codex", "Default Codex", "/Users/ada/.codex");
  const work = { ...codex, profile_id: "catprofile-2", display_name: "Work Codex", configured_home: "/Users/ada/work/.codex" };
  const claude = { ...profile("claude", "Claude", "Default Claude", "/Users/ada/.claude"), profile_id: "catprofile-3" };
  assert.equal(unambiguousProfile([codex, claude], "codex"), codex);
  assert.equal(unambiguousProfile([codex, work, claude], "codex"), null, "two profiles: no guess");
  assert.equal(unambiguousProfile([codex], "claude"), null);

  const holding = (capability: CatalogCapabilityDescriptor, count: number | null, scope_id: string | null = null): CatalogProfileHolding => ({
    capability, count, newest_modified_at: count ? 1_700_000_000 : null, roots: ["Codex home"], scope_id,
  });
  const found = [holding(CODEX_SKILLS, 12), holding(CODEX_PLUGINS, 3), holding(CODEX_PETS_V1, 0), holding(CODEX_PETS_V2, 2)];
  const cards = catalogKindCards(CAPABILITIES, codex, [], found);
  assert.equal(catalogDefaultKind(cards, null), catalogChoiceKey(CODEX_SKILLS), "the first kind with something in it");
  assert.equal(catalogDefaultKind(cards, catalogChoiceKey(CODEX_PETS_V1)), catalogChoiceKey(CODEX_PETS_V1), "a Kind door is the answer outright");
  assert.equal(catalogKindCardFoot(cards[0]), `12 skills · ${formatRelativeTime(1_700_000_000)}`);
  assert.equal(catalogKindCardFoot(cards[2]), "Nothing here yet");
  assert.equal(catalogKindCardFoot(catalogKindCards(CAPABILITIES, codex, [], null)[0]), "Agent Skill", "the format label until the count arrives");
  // A kind this home already has a row for is Added, never the default —
  // from the inspection, or from the rows the shell already lists.
  const added = catalogKindCards(CAPABILITIES, codex, [], [holding(CODEX_SKILLS, 12, "scope-1"), ...found.slice(1)]);
  assert.equal(added[0].existingScopeId, "scope-1");
  assert.equal(catalogDefaultKind(added, catalogChoiceKey(CODEX_SKILLS)), catalogChoiceKey(CODEX_PLUGINS));
  const listed = catalogKindCards(CAPABILITIES, codex, [{ scope: { ...CODEX_SKILLS, scope_id: "scope-9", profile_id: codex.profile_id, revision: 1, created_at: 1, updated_at: 1 } }], null);
  assert.equal(listed[0].existingScopeId, "scope-9");
  // Nothing found and no door: nothing guessed.
  assert.equal(catalogDefaultKind(catalogKindCards(CAPABILITIES, codex, [], null), null), null);

  // Storage: the one a row used last, else the first as the shell orders
  // them — Mallard Cloud, the other clouds, local — else none; and why.
  const storages = [{ id: "cloud", provider: "managed_cloud" }, { id: "vault", provider: "r2" }, { id: "drive", provider: "local" }];
  assert.equal(catalogDefaultStorage(storages, null), "cloud");
  assert.equal(catalogDefaultStorage(storages, "vault"), "vault");
  assert.equal(catalogDefaultStorage(storages, "gone"), "cloud");
  assert.equal(catalogDefaultStorage([], null), null);
  assert.equal(catalogStorageHint(storages, "cloud", null), "Mallard Cloud first.");
  assert.equal(catalogStorageHint(storages, "vault", "vault"), "Used last by the Catalog.");
  assert.equal(catalogStorageHint(storages.slice(1, 2), "vault", null), "Your only storage.");
  assert.equal(catalogStorageHint(storages, "drive", null), null);
  assert.equal(catalogStorageHint([], null, null), "No storage yet.");
});

test("the profile is named by its folder, and renamed from the row's page", () => {
  const home = readFileSync("src/components/storage-catalog/CatalogHomeStep.tsx", "utf8");
  // Step one never asks for a name: a typed path is saved as part of Set up
  // itself, named by its folder, and the home's setup page opens on it.
  assert.doesNotMatch(home, /Profile name|Save home|catalog-profile-save-row/);
  // A display name reaches step one only as the label a provider mark draws
  // itself with — never as a field, a piece of state, or a value it sends.
  assert.doesNotMatch(
    home.replace(/<ProviderMark[\s\S]*?\/>/g, ""),
    /displayName/,
    "displayName is only the provider mark's prop",
  );
  assert.match(home, /const saved = await storageCatalogApi\.saveProfile\(provider, expandedPath\);[\s\S]*?onChosen\(saved\)/);
  assert.match(home, /const ready = existing !== null \|\| resolvedProbe !== null;/);
  // Step two's Name is the project's: the home renamed through the command
  // the row's page uses, pinned to the profile's revision.
  const setup = readFileSync("src/components/storage-catalog/CatalogScopeSetupStep.tsx", "utf8");
  assert.match(setup, /storageCatalogApi\.renameProfile\(profile\.profile_id, name\.trim\(\), profile\.revision\)/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-profile-save-row/);

  // Renaming lives on the row's page: the scope crumb's menu opens the same
  // rename row the project page uses, pinned to the profile's revision, with
  // the derived folder name one click away.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /label: "Rename profile…"/);
  assert.match(page, /for every Catalog row that uses this home/);
  assert.match(page, /className="sync-path-rename"/);
  assert.match(page, /storageCatalogApi\.renameProfile\(profile\.profile_id, profileNameDraft\.trim\(\), profile\.revision\)/);
  assert.match(page, /Use folder name/);
  // A successful rename relabels the shell too — sidebar and Switch-to read
  // the scope list.
  assert.match(page, /onProfileRenamed\(\)/);
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /onProfileRenamed=\{\(\) => void refreshCatalogScopes\(\)\}/);
});

test("linking is a side effect of the verb, and never syncs content", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Link changes configuration only; the comment on the one place that calls
  // it says so, because nothing in the UI asks the user to do it any more.
  assert.match(page, /configuration only, no provider content changed/);
  // The scope links to the storage once — never per resource.
  assert.match(page, /storageCatalogApi\.linkScopeStorage\(scope\.scope\.scope_id, storageId\)/);
  // A page that is already paired never links again.
  assert.match(page, /if \(page\?\.link\) return;/);
});

test("Unlink is the storage crumb's verb and Fork lives on the card", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Unlinking the one pairing is the storage crumb's verb, worded the way
  // the project's is, pinned to the link's own revision.
  assert.match(page, /label: `Unlink from \$\{label\}…`/);
  assert.match(page, /storageCatalogApi\.unlinkScopeStorage\(pageLink\.link_id, pageLink\.revision\)/);
  assert.match(page, /Push and Pull stop for these/);
  // Fork is the card's own verb: a git-fork button toggling the card's
  // drawer, keyed by the scope and the resource — no registration anywhere.
  assert.match(page, /storageCatalogApi\.prepareFork\(scope\.scope\.scope_id, row\.resource_id, name\.trim\(\)\)/);
  assert.match(page, /storageCatalogApi\.applyFork\(scope\.scope\.scope_id, plan\.plan_id\)/);
  assert.match(page, /className="catalog-fork-drawer"/);
  assert.match(page, /seed=\{forkSeed\(row\)\}/);
  assert.match(page, /forkNameRuleBroken/);
  // A refusal stays with the drawer that initiated the action.
  assert.match(page, /setForkError\(errorMessage\(reason\)\)/);
  // A row with nothing local to copy keeps the button and loses only its
  // action, the way Reveal beside it already does — dropping it reflowed the
  // cluster and cost the list its column of controls.
  assert.match(page, /const canFork = row\.local_present && !!row\.resource_id;/);
  assert.match(page, /disabled=\{!canFork\}/);
  assert.match(page, /title=\{canFork \? forkTitle : `Not on this Mac, so there is nothing to fork`\}/);
  // The capture protocol decides whether the control exists at all: a fork
  // copies a directory, so a capability inventoried by its provider shows no
  // button rather than one dead on every row.
  assert.match(page, /const forkable = scope\.capability\?\.capture === "directory_tree";/);
  assert.match(page, /onFork=\{forkable \? \(\) => \{/);
  // The readouts hold a column so the rule, the info button and the timestamp
  // after them start at the same x on every row.
  const appCss = readFileSync("src/App.css", "utf8");
  assert.match(appCss, /\.catalog-card \.sync-history-thread-meta\s*\{[^}]*min-width: 7\.5rem;/s);
  // The stamp cells already end on the same pixel; the glyphs inside them did
  // not. An em dash fills its whole cell while a digit stops short of its own,
  // so the placeholder is inset by that bearing and the ink edges read as one.
  assert.match(page, /className=\{stamp == null \? "is-empty" : undefined\}/);
  assert.match(appCss, /\.sync-history-thread-time time\.is-empty\s*\{[^}]*padding-right: 0\.085em;/s);
});

test("provider, kind, and format are open keys with backend labels in the frontend contract", () => {
  const types = readFileSync("src/components/storage-catalog/types.ts", "utf8");
  // Coordinates are four open axes, and every DTO that names one carries the
  // backend's labels beside the raw keys.
  assert.match(types, /export interface CatalogCoordinate \{\s*provider: string;\s*resource_kind: string;\s*format: string;\s*format_version: number;\s*\}/);
  assert.match(types, /export interface CatalogCoordinateLabels \{\s*provider_label: string;\s*kind_label: string;\s*format_label: string;\s*\}/);
  assert.match(types, /export interface CatalogCapabilityDescriptor extends CatalogCoordinate/);
  assert.match(types, /presentation: CatalogPresentation;/);
  assert.match(types, /capture: CatalogCaptureSupport;/);
  assert.match(types, /export interface CatalogScope extends CatalogCoordinate/);
  assert.match(types, /export interface CatalogPushRow extends CatalogCoordinate, CatalogCoordinateLabels/);
  assert.match(types, /export interface CatalogPullItem extends CatalogCoordinate, CatalogCoordinateLabels/);
  assert.match(types, /export interface CatalogStorageRepository extends CatalogCoordinate, CatalogCoordinateLabels/);
  assert.match(types, /available: boolean;\s*unavailable_reason\?: string;/);
  assert.match(types, /expected_target_state: CatalogTargetExpectation;/);
  // The Pull review pins the coordinate repository's version, or, on a
  // storage that holds one item per repository, every item's own.
  assert.match(types, /head\?: LibraryHeadCoordinate;\s*item_heads\?: Record<string, LibraryHeadCoordinate>;/);
  assert.match(types, /observed_items\?: Record<string, LibraryHeadCoordinate>;/);
  assert.match(types, /source_coordinate: CatalogCoordinate;/);
  assert.doesNotMatch(types, /resources: string\[\]|CatalogResourceFamily|kinds: /);
  // No closed union and no identity branch survives anywhere in the Library:
  // the backend's labels, hints, and protocols are the only vocabulary.
  for (const file of [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/storage-catalog/CatalogIndexPage.tsx",
    "src/components/storage-catalog/AddCatalogScopeWorkspace.tsx",
    "src/components/storage-catalog/CatalogHomeStep.tsx",
    "src/components/storage-catalog/CatalogScopeSetupStep.tsx",
    "src/components/storage-catalog/setupFurniture.tsx",
    "src/components/storage-catalog/api.ts",
    "src/components/storage-catalog/model.ts",
    "src/components/storage-catalog/types.ts",
  ]) {
    assert.doesNotMatch(
      readFileSync(file, "utf8"),
      /=== "(codex|claude|agent_skill|plugin_install_intent|desktop_pet|codex_pet_v1|codex_pet_v2)"|Catalog(Provider|Resource)(Key|Kind)\b|CATALOG_KINDS|catalogKindMeta|petKind|pet_description/,
      file,
    );
  }
});

test("Pull labels are the protocol's four outcomes, the same for every format", () => {
  const action = (kind: CatalogPullAction["kind"]): CatalogPullAction => ({
    action_id: `action-${"a".repeat(24)}`,
    resource_id: `resource-${"b".repeat(64)}`,
    kind,
    requires_destructive_approval: false,
    expected_target_state: { state: "absent" },
    incoming_content_sha256: "c".repeat(64),
  });
  const copy = {
    copy_name: "review-local-copy",
    copy_resource_id: `resource-${"d".repeat(64)}`,
    copy_target_path: "/tmp/review-local-copy",
    transformed_content_sha256: "e".repeat(64),
    original_content_sha256: "f".repeat(64),
    storage_content_sha256: "1".repeat(64),
    reusable_witness: false,
  };
  assert.equal(
    actionLabel(action({ kind: "preserve_as_copy_and_overwrite", copy })),
    "Keep local as review-local-copy and overwrite",
  );
  assert.equal(actionLabel(action({ kind: "overwrite_with_storage" })), "Overwrite local content");
  assert.equal(actionLabel(action({ kind: "install" })), "Install stored content");
  assert.equal(actionLabel(action({ kind: "update" })), "Update from storage");
  const types = readFileSync("src/components/storage-catalog/types.ts", "utf8");
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.doesNotMatch(types, /install_skill|install_plugin|install_pet|overwrite_plugin_with_storage/);
  assert.match(types, /conflicting_claims: CatalogClaimPin\[\]/);
  assert.match(page, /claimed by[\s\S]*Preserve a copy, acknowledge overwrite, or Skip/);
  assert.match(readFileSync("src/components/storage-catalog/outcomes.ts", "utf8"), /Completed changes were kept/);
});

test("the Catalog remains independent from project sync", () => {
  const liveFiles = [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/storage-catalog/AddCatalogScopeWorkspace.tsx",
    "src/components/storage-catalog/CatalogHomeStep.tsx",
    "src/components/storage-catalog/CatalogScopeSetupStep.tsx",
    "src/components/storage-catalog/setupFurniture.tsx",
    "src/components/storage-catalog/api.ts",
    "src/components/storage-catalog/model.ts",
    "src/components/storage-catalog/types.ts",
  ];
  for (const file of liveFiles) {
    const source = readFileSync(file, "utf8");
    assert.doesNotMatch(source, /components\/project-sync|from "\.\.\/\.\.\/types"|from "\.\.\/types"/);
  }
  const projectTypes = readFileSync("src/types.ts", "utf8");
  const projectApi = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.doesNotMatch(projectTypes, /plugin_install_intent|codex_pet_v1|catalog_resource/i);
  assert.doesNotMatch(projectApi, /catalog/i);
});

/* Target roots are resolved by each provider's adapter from the selected
   profile home; the common profile resolution never discovers one. Tests may
   spell out paths; the production halves of both modules are what is pinned. */
const rustProduction = (file: string) => readFileSync(file, "utf8").split("#[cfg(test)]")[0];

test("only global provider skill roots are discoverable", () => {
  const adapters = rustProduction("src-tauri/src/storage_catalog/adapters.rs");
  const profiles = rustProduction("src-tauri/src/storage_catalog/profiles.rs");
  // Each provider's home skill root, and the one shared user root.
  assert.match(adapters, /"provider-home\.skills",\s*&canonical_home\.join\("skills"\)/);
  assert.match(adapters, /"shared-user\.skills",\s*&os_user_home\.join\("\.agents"\)\.join\("skills"\)/);
  // Never a project, a checkout, a working directory, or a per-project
  // skill folder — neither where roots are resolved nor where profiles are.
  for (const source of [adapters, profiles]) {
    assert.doesNotMatch(source, /project_root|checkout_root|working_directory|\.skill\/|\.agent\/ssd/);
  }
});

test("the shared skill root belongs to the default Codex home only", () => {
  const adapters = rustProduction("src-tauri/src/storage_catalog/adapters.rs");
  const profiles = rustProduction("src-tauri/src/storage_catalog/profiles.rs");
  // The Codex adapter offers the shared root only when the profile is the
  // default home, and the default directory's name is read from the
  // provider descriptor before the adapter's own fallback.
  assert.match(
    adapters,
    /\.default_profile_directory\s*\.unwrap_or_else[\s\S]{0,80}?if is_default_home\(canonical_home, os_user_home, &default_directory\) \{\s*targets\.push\(resolved_target\(\s*"shared-user\.skills"/,
  );
  // The Claude adapter exposes only its own home root and plugin home.
  assert.match(
    adapters,
    /Claude exposes only its own user-global skills root and plugin home[\s\S]*?fn resolve_targets[\s\S]*?"provider-home\.skills"[\s\S]*?"user\.plugins"[\s\S]*?\]\)\s*\}/,
  );
  const claudeTargets = adapters.slice(adapters.indexOf("Claude exposes only its own user-global skills root"));
  assert.doesNotMatch(claudeTargets.slice(0, claudeTargets.indexOf("fn launch_home_app")), /shared-user\.skills/);
  // Common profile resolution names no provider directory: that knowledge
  // stays in the provider descriptor.
  assert.match(profiles, /default_profile_directory/);
  assert.doesNotMatch(profiles, /"\.codex"|"\.claude"/);
});


test("search bulk selection retains hidden choices and does not select ineligible matches", async () => {
  const { replaceSearchSelection } = await import("../../src/components/sync-review/nameSearch");
  const selected = new Set(["hidden", "matching-old"]);
  const matchingIds = ["matching-old", "matching-safe", "matching-blocked"];
  const recommended = new Set(["matching-safe", "outside-safe"]);
  const next = replaceSearchSelection(selected, matchingIds, recommended);
  assert.deepEqual([...next].sort(), ["hidden", "matching-safe"]);
  assert.deepEqual([...selected].sort(), ["hidden", "matching-old"]);
  assert.deepEqual([...replaceSearchSelection(next, matchingIds)], ["hidden"]);
  assert.deepEqual([...replaceSearchSelection(next, [])].sort(), [...next].sort());
});

test("the Library page offers Finish that push on the unfinished-push sentence only", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The failure zone's one control, keyed on the backend's sentence for this
  // storage; the action goes through the API and reports on the same page.
  assert.match(page, /action: finishPushOffered\(failure, storageName\)/);
  assert.match(page, /Finish that push/);
  assert.match(page, /storageCatalogApi\.finishUnfinishedPush\(scope\.scope\.scope_id, storageId\)/);
  assert.match(page, /notify\(PUSH_FINISHED\)/);
});


test("Catalog Push reports completed, failed and unattempted items without claiming rollback", async () => {
  const { pushOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const result = pushOutcome({ plan_id: "review", published: true, revision: null, stopped: "Second item refused", message: "1 of 3 completed", results: [
    { resource_id: "a", display_name: "Duck", state: "published", message: null },
    { resource_id: "b", display_name: "Seal", state: "failed", message: "No access" },
    { resource_id: "c", display_name: "Fox", state: "not_attempted", message: null },
  ] });
  assert.equal(result.title, "Pushed 1 of 3 items");
  assert.deepEqual(result.rows.map((row) => row.status), ["synced", "failed", "not_attempted"]);
  assert.deepEqual(result.actions[0].resourceIds, ["b", "c"]);
  const { default: Outcome } = await import("../../src/components/sync-review/SyncOutcomeNotice");
  const html = renderToStaticMarkup(<Outcome outcome={result} onDismiss={() => {}} />);
  assert.match(html, /Duck/); assert.match(html, /Not attempted/);
  assert.doesNotMatch(html, /nothing was published|Not pulled/);
});

test("Catalog Pull keeps preserved, skipped and failed results distinct", async () => {
  const { pullOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const result = pullOutcome({ plan_id: "review", remote_accepted: false, results: [
    { resource_id: "a", display_name: "Duck", outcome: "preserved_and_updated" },
    { resource_id: "b", display_name: "Seal", outcome: "skipped" },
    { resource_id: "c", display_name: "Fox", outcome: "partial", message: "Provider apply stopped" },
  ] });
  assert.deepEqual(result.rows.map((row) => row.status), ["preserved", "skipped", "failed"]);
  assert.deepEqual(result.actions[0].resourceIds, ["b", "c"]);
});

test("Cloud repository and review components are shared without feature imports", () => {
  for (const file of ["managed-cloud/ManagedCloudChip.tsx", "managed-cloud/managedCloudLink.ts", "managed-cloud/RepositoryLookupField.tsx", "sync-review/SyncOutcomeNotice.tsx", "sync-review/reviewPreparationLifecycle.ts"]) {
    const source = readFileSync(`src/components/${file}`, "utf8");
    assert.doesNotMatch(source, /project-sync|storage-catalog|from ["'].*ipc["']/);
  }
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.doesNotMatch(page, /nothing was published|nothing was changed/);
  assert.match(page, /applyInFlight.current/);
  assert.match(page, /PREPARING_CANCELLED/);
});

for (const verb of ["push", "pull"] as const) {
  test(`Catalog ${verb} cancel, failure and retry release busy state without automatic request loops`, async () => {
    const { createRequire } = await import("node:module");
    const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
    const { createReviewPreparing, PREPARING_CANCELLED } = await import("../../src/components/sync-review/reviewPreparationLifecycle");
    const source = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
    const slice = (start: string, end: string) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
    const handlers = transformSync([
      slice("  const openReview = async", "  /* On an item's page the review holds"),
      slice("  const runPush = async", "  /* Finish that push:"),
      slice("  const runPull = async", "  const unlinkStorage ="),
    ].join("\n"), { loader: "ts" }).code;
    const pending: { resolve: (value: unknown) => void; reject: (error: unknown) => void }[] = [];
    const deferred = () => new Promise<unknown>((resolve, reject) => pending.push({ resolve, reject }));
    const noop = () => {};
    let busy = false;
    let shown: unknown = null;
    let mode: unknown = { kind: verb };
    let error: unknown = null;
    let refreshes = 0;
    const reviewPreparing = createReviewPreparing({ shown: noop, writing: noop, finished: noop }, { set: () => 0, clear: noop });
    const plan = {
      plan_id: "fresh", rows: [{ row: { resource_id: "duck", selected_by_default: true } }],
      items: [{ item: { resource_id: "duck", relation: "storage_only", default_action_id: "install" } }],
    };
    const context = {
      storageId: "cloud", scope: { scope: { scope_id: "pets" } }, prepareInFlight: { current: false }, applyInFlight: { current: false },
      operationEpoch: { current: 0 }, preparingKind: { current: "push" }, reviewPreparing, PREPARING_CANCELLED,
      setBusy: (value: boolean) => { busy = value; }, setError: (value: unknown) => { error = value; },
      setPushPlan: (value: unknown) => { shown = value; }, setFailures: noop, setOutcome: noop, ensureLink: async () => {},
      setPushSelected: noop, setPushAcks: noop, setMode: (value: unknown) => { mode = value; },
      setPullPlan: (value: unknown) => { shown = value; }, setPullSelected: noop, setPullChoice: noop, setPullAcks: noop,
      filterCatalogPullReviewItems, storageCatalogApi: { preparePush: deferred, applyPush: deferred, preparePull: deferred, applyPull: deferred }, storageFailureMessage: String,
      pushPlan: plan, pushSelected: new Set(["duck"]), pushAcks: new Set(), setProgress: noop, noun: () => "pets",
      pullPlan: plan, pullSelected: new Set(["duck"]), pullChoice: { duck: "install" }, pullAcks: new Set(), pullResult: noop,
      load: async () => { refreshes += 1; }, pushResult: noop, finishPushOffered: () => false, storageName: "Cloud",
    };
    const flow = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handlers}; return { openReview, apply: context.verb === "push" ? runPush : runPull };`)({ ...context, verb }) as {
      openReview: (verb: "push" | "pull") => Promise<void>; apply: () => Promise<void>;
    };
    const cancelled = flow.openReview(verb);
    await Promise.resolve();
    await flow.openReview(verb);
    assert.equal(pending.length, 1, "double clicks cannot start duplicate preparations");
    reviewPreparing.cancel();
    await cancelled;
    assert.equal(busy, false);
    assert.equal(context.prepareInFlight.current, false);
    assert.deepEqual(mode, { kind: "list" }, "cancelling a replacement review leaves no obsolete Apply control");
    assert.equal(shown, null);
    const retry = flow.openReview(verb);
    await Promise.resolve();
    pending[1].resolve(plan);
    await retry;
    pending[0].resolve({ ...plan, plan_id: "late" });
    await Promise.resolve();
    assert.equal(shown, plan, "a cancelled request cannot replace the current review");
    assert.equal(pending.length, 2, "settling a read never triggers another read");
    const applying = flow.apply();
    await flow.apply();
    assert.equal(pending.length, 3, "apply is single flight");
    pending[2].reject(new Error("Connection lost"));
    await applying;
    assert.equal(busy, false);
    assert.equal(context.applyInFlight.current, false);
    assert.equal(shown, null, "a failed apply removes the stale plan before another attempt");
    assert.equal(refreshes, 1);
    assert.match(String(error), verb === "push" ? /Review again before retrying/ : /Compare again to check completed changes/);
    assert.equal(pending.length, 3, "a failed write is never retried automatically");
    const failedReview = flow.openReview(verb);
    await Promise.resolve();
    pending[3].reject(new Error("Account unavailable"));
    await failedReview;
    assert.equal(busy, false);
    assert.equal(context.prepareInFlight.current, false);
    assert.equal(shown, null);
    assert.deepEqual(mode, { kind: "list" });
    assert.equal(pending.length, 4, "a failed comparison waits for an explicit retry");
  });
}

function cloudRepository(overrides: Partial<CatalogCloudRepository> = {}): CatalogCloudRepository {
  const owner = overrides.owner_login ?? "kai";
  const name = overrides.name ?? "Duck";
  return {
    provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2,
    provider_label: "Codex", kind_label: "Pets", format_label: "Codex pet v2",
    repository_id: `repo_${owner}_${name}`, owner_login: owner, name, address: `${owner}/${name}`,
    website_url: `https://mallard-ai.com/${owner}/${name}`, visibility: "private", role: "member", can_push: true,
    revision: 4, updated_at: 1, registration_only: false, resource_id: "duck", display_name: "Duck", available: true,
    description: null, star_count: 0, viewer_starred: false,
    ...overrides,
  };
}

test("a pet's chooser names every misfit in one sentence, and the backend checks again on Connect", async () => {
  const { cloudRepositoryMisfit, cloudRoleWord } = await import("../../src/components/storage-catalog/CloudRepositoryChoice");
  const pet = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
  const item = { kind: "item" as const, coordinate: pet, noun: "pet v2", resourceId: "duck", name: "Duck", offerNew: true };
  const add = { kind: "add" as const, coordinate: pet, noun: "pet v2", rowLabel: "Pets v2 · Codex home", listed: new Set(["duck"]) };
  const skill = cloudRepository({ owner_login: "maya", name: "pdf-tools", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1, kind_label: "Skills" });
  const classic = cloudRepository({ owner_login: "Rika321", name: "Duck-classic", format_version: 1, format_label: "Codex pet v1" });
  const owl = cloudRepository({ owner_login: "Rika321", name: "Owl", resource_id: "owl", display_name: "Owl", role: "owner" });
  assert.equal(cloudRepositoryMisfit(skill, item, CAPABILITIES), "maya/pdf-tools is a skill, not a pet v2.");
  assert.equal(cloudRepositoryMisfit(classic, item, CAPABILITIES), "Rika321/Duck-classic is a Pets v1 repository.");
  assert.equal(cloudRepositoryMisfit(owl, item, CAPABILITIES), "Rika321/Owl holds Owl, not Duck.");
  assert.equal(cloudRepositoryMisfit(cloudRepository(), item, CAPABILITIES), null);
  // Add brings in someone else's item the row has not got.
  assert.equal(cloudRepositoryMisfit(owl, add, CAPABILITIES), "Rika321/Owl is yours — already listed.");
  assert.equal(cloudRepositoryMisfit(cloudRepository(), add, CAPABILITIES), "Pets v2 · Codex home already has Duck.");
  assert.equal(cloudRepositoryMisfit(cloudRepository({ resource_id: "biscuit", display_name: "Biscuit" }), add, CAPABILITIES), null);
  assert.equal(cloudRepositoryMisfit(skill, { kind: "door" }, CAPABILITIES), null, "the door takes any kind");
  assert.match(cloudRepositoryMisfit({ ...skill, available: false }, { kind: "door" }, CAPABILITIES) ?? "", /can't open/);
  assert.equal(cloudRoleWord({ role: null, visibility: "public" }), "Public · Contributor");
  assert.equal(cloudRoleWord({ role: "member", visibility: "private" }), "Member");
  assert.equal(cloudRoleWord({ role: "owner", visibility: "public" }), "Public · Owner");
});

test("a closed chooser drops its late answer, and a lookup never asks twice", async () => {
  const { createRequire } = await import("node:module");
  const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
  const source = readFileSync("src/components/storage-catalog/CloudRepositoryChoice.tsx", "utf8");
  const slice = (start: string, end: string) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
  const handlers = transformSync(slice("  const submit = async", "  const choose ="), { loader: "ts" }).code;
  let resolve!: (value: unknown) => void;
  let selection: unknown = "new";
  let calls = 0;
  const noop = () => {};
  const context = {
    entry: "maya/Duck", working: false, joining: false, lookup: { kind: "idle" }, selection: "new",
    sequence: { current: 0 }, storageId: "cloud",
    setJoinError: noop, setJoining: noop, setLookup: noop, setEntry: noop,
    setSelection: (value: unknown) => { selection = value; },
    setTab: noop, setResolvingRow: noop, scopeHolding: () => null,
    isInvitationCodeShaped: () => false, errorMessage: String,
    answer: (resolution: { kind: string; repository?: unknown }) => resolution.kind === "found"
      ? { kind: "found", repository: resolution.repository, joined: false }
      : { kind: "error", message: "no" },
    storageCatalogApi: { resolveCloudRepository: () => { calls += 1; return new Promise((done) => { resolve = done; }); } },
  };
  const flow = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handlers}; return { submit };`)(context) as {
    submit: () => Promise<void>;
  };
  const pending = flow.submit();
  context.sequence.current += 1; // The dialog closed: its cleanup moves the number on.
  resolve({ kind: "found", repository: cloudRepository() });
  await pending;
  assert.equal(selection, "new", "a closed chooser's answer selects nothing");
  assert.equal(calls, 1, "nothing asks again on its own");
});

test("the connect dialog is the project's, with fewer words, and opens no review", () => {
  const html = renderToStaticMarkup(
    <CatalogConnectDialog
      reason={{ kind: "connect", resourceId: "duck", name: "Duck" }}
      scopeId="pets" storageId="cloud" accountLogin="Rika321"
      coordinate={{ provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 }}
      noun="pet v2" rowLabel="Pets v2 · Codex home" listed={new Set()} capabilities={CAPABILITIES}
      onClose={() => undefined} onConnected={() => undefined}
    />,
  );
  assert.match(html, /<h1[^>]*>Connect Duck<\/h1>/);
  assert.match(html, /class="sync-modal sync-bundle-connection-dialog"/);
  assert.match(html, /<span class="sync-visually-hidden">Search, address or invitation code<\/span>/, "the placeholder says it; the label is for screen readers");
  assert.match(html, /Loading repositories…/);
  assert.doesNotMatch(html, /sync-bundle-context|repo-list-head/, "no context row, no list heading");
  assert.doesNotMatch(html, /Can’t be changed later/, "said once the choice can be made");
  const dialog = readFileSync("src/components/storage-catalog/CatalogConnectDialog.tsx", "utf8");
  assert.match(dialog, /Can’t be changed later\./);
  assert.match(dialog, /storageCatalogApi\.createCloudRepository\(/);
  assert.match(dialog, /storageCatalogApi\.connectCloudRepository\(/);
  assert.doesNotMatch(dialog, /preparePull|applyPull|openPull|Connect and review|Disconnect/);
});

test("on Mallard Cloud an item has its own page and its repository lives in its bar", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The list carries no Cloud chip and no per-row connect; its names open
  // the item's page on Mallard Cloud only.
  assert.doesNotMatch(page, /CatalogCloudItem|CatalogCloudConnections|Connect Cloud repository|>Cloud repositories</);
  assert.match(page, /onOpen=\{managedCloud && resourceId && !itemRow \?/);
  assert.match(page, /className="sync-history-session-title-button" title=\{`\$\{row\.display_name\} · View \$\{kindNoun\} details`\}/);
  // One chip, in the bar, on an item's page; read once per load.
  assert.equal(page.match(/<ManagedCloudChip/g)?.length, 1);
  assert.match(page, /\{itemRow && itemRepository && \(\s*<ManagedCloudChip/);
  assert.match(page, /readKey: page,/);
  // The project's notes, and Pull and Push close as they do for a project.
  assert.match(page, /cloudStatusNotice\(status\)/);
  assert.match(page, /itemRow && cloudSyncBlocked\(itemStatus\)/);
  // A connected item keeps its repository: no Change or Disconnect verbs.
  assert.doesNotMatch(page, /onReconnect|onDisconnect|disconnectCloudRepository/);
  // Add from Mallard Cloud is the list head's verb beside the search, on
  // Mallard Cloud's list only: the head is a row, the search keeps its own
  // rule with no count, and the bar holds no round +. An empty row offers
  // the same verb in its empty state.
  assert.doesNotMatch(page, /sync-path-icon-action"\s+title="Add from Mallard Cloud…"/);
  assert.match(page, /const addFromCloud = managedCloud && \(\s*<button\s+type="button"\s+className="btn"\s+aria-haspopup="dialog"/);
  assert.match(page, /<NameSearchField[^/]*count=\{!managedCloud\} trailing=\{addFromCloud\} \/>/);
  assert.match(page, /none stored yet\.`\}\s*\{addFromCloud\}/);
  const field = readFileSync("src/components/sync-review/NameSearchField.tsx", "utf8");
  assert.match(field, /\{count \? \(/);
  assert.match(field, /className="sync-visually-hidden" role="status"/);
  assert.match(field, /<div className="sync-list-head">\s*\{leading\}\s*\{field\}\s*\{trailing\}/);
  const headCss = readFileSync("src/App.css", "utf8");
  assert.match(headCss, /\.sync-list-head\s*\{[^}]*display:\s*flex;/);
  assert.match(headCss, /\.sync-list-head > \.sync-name-search\s*\{[^}]*margin-bottom:\s*0;/);
  // Reviews opened on an item's page hold that item alone.
  assert.match(page, /openReview\("push", itemRow\?\.resource_id \? \[itemRow\.resource_id\] : undefined\)/);
  assert.match(page, /const planRows = pushPlan\.rows\.filter\(\(row\) => !itemRow \|\| row\.row\.resource_id === itemRow\.resource_id\);/);
  assert.match(page, /repository=\{pushRepository\(pushPlan\.destinations\?\.\[resourceId\]\)\}/);
  assert.match(page, /repository=\{managedCloud \? pullRepository\(pullPlan\.sources\?\.\[resourceId\]\) : undefined\}/);
  // A review, its Preparing… and a finished Push or Pull sit on the Storage
  // card, as a project's do; any other storage keeps the Catalog's page.
  assert.match(page, /<StorageCard name=\{storageName \?\? "Mallard Cloud"\} review>\s*<div className="sync-storage-inline-review" tabIndex=\{-1\}>/);
  assert.match(page, /Reading the latest version from \$\{storageWord\}/);
  assert.match(page, /Comparing this computer with \$\{storageWord\}/);
  assert.match(page, /const outcomeCard = managedCloud && outcomeNotice/);
  assert.match(page, /managedCloud \? \(pushing \? "Pushing…" : "Push"\) : `Push \$\{pushSelected\.size\}`/);
  assert.match(page, /A running push finishes or fails whole — it can’t be cancelled midway\./);
  assert.match(page, /Pull cannot be cancelled while changes are being applied\./);
  assert.match(page, /\? cloudPushOutcome\(result, \{/);
  const api = readFileSync("src/components/storage-catalog/api.ts", "utf8");
  assert.doesNotMatch(api, /disconnect/);
  const chip = readFileSync("src/components/managed-cloud/ManagedCloudChip.tsx", "utf8");
  assert.doesNotMatch(chip, /Change repository…|Disconnect repository|onReconnect|onDisconnect/);
});

test("a Mallard Cloud outcome speaks the project's words: what moved, its repository, one version", async () => {
  const { cloudPushOutcome, cloudPullOutcome, pushOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const noun = (count: number) => (count === 1 ? "pet v2" : "pets v2");
  const addresses: Record<string, string> = { duck: "Rika321/Duck", pip: "kai/Pip", mossy: "Rika321/Mossy" };
  const words = { noun, addressOf: (id: string) => addresses[id], at: 100 };
  const single = cloudPushOutcome({ plan_id: "review", published: true, revision: null, stopped: null, message: "", results: [
    { resource_id: "duck", display_name: "Duck", state: "published", message: null, revision: 3 },
  ] }, words);
  assert.equal(single.title, "Push complete");
  assert.deepEqual(single.stats, [
    { icon: "upload", value: "1", label: "pet v2" },
    { icon: "git-branch", label: "Version", value: "3", lead: true },
  ]);
  assert.deepEqual(single.rows[0], { key: "duck", title: "Duck", status: "synced", reason: null, threadId: "Rika321/Duck", updatedAt: 100 });
  const batch = cloudPushOutcome({ plan_id: "review", published: true, revision: null, stopped: "Pip refused", message: "", results: [
    { resource_id: "duck", display_name: "Duck", state: "published", message: null, revision: 3 },
    { resource_id: "pip", display_name: "Pip", state: "failed", message: "No access", revision: null },
  ] }, words);
  assert.equal(batch.title, "Pushed 1 of 2 pets v2");
  assert.deepEqual(batch.stats.map((stat) => stat.label), ["pushed", "not pushed"], "a batch has no one version");
  assert.deepEqual(batch.actions[0].resourceIds, ["pip"]);
  const pulled = cloudPullOutcome({ plan_id: "review", remote_accepted: true, results: [
    { resource_id: "pip", display_name: "Pip", outcome: "updated" },
    { resource_id: "mossy", display_name: "Mossy", outcome: "preserved_and_updated" },
  ] }, { ...words, keptAs: (id) => (id === "mossy" ? "Mossy 2" : null), version: null });
  assert.equal(pulled.title, "Pull complete");
  assert.deepEqual(pulled.stats.map((stat) => `${stat.value} ${stat.label}`), ["1 restored", "1 kept independently"]);
  assert.deepEqual(pulled.rows.map((row) => [row.title, row.status, row.threadId]), [["Pip", "synced", "kai/Pip"], ["Mossy 2", "preserved", "Mossy 2"]]);
  const one = cloudPullOutcome({ plan_id: "review", remote_accepted: true, results: [
    { resource_id: "pip", display_name: "Pip", outcome: "installed" },
  ] }, { ...words, keptAs: () => null, version: 7 });
  assert.equal(one.stats.find((stat) => stat.lead)?.value, "7");
  // Any other storage keeps the Catalog's own card and its one version.
  const folder = pushOutcome({ plan_id: "review", published: true, revision: 8, stopped: null, message: "", results: [
    { resource_id: "duck", display_name: "Duck", state: "published", message: null },
  ] });
  assert.deepEqual(folder.stats.map((stat) => stat.label), ["completed", "Version"]);
});
