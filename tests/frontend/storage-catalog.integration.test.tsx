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
  catalogMatchingRows,
  catalogNouns,
  catalogPresentation,
  catalogProviderList,
  catalogScopeLabel,
  catalogScopeViews,
  catalogPullRecommendation,
  catalogStatusScope,
  catalogStatusScopeCounts,
  catalogStorageHint,
  catalogStorageRepositoryViews,
  filterCatalogPullReviewItems,
  rowStatus,
  rowStatusLabel,
  unambiguousProfile,
} from "../../src/components/storage-catalog/model";
import { formatRelativeTime } from "../../src/clock";
import { i18n } from "../../src/i18n";
import { en } from "../../src/i18n/en";
import type { StatusScope } from "../../src/components/sync-review/statusScopes";
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
  "connect_catalog_cloud_repository", "create_catalog_cloud_repository", "get_catalog_cloud_status", "discard_catalog_cloud_draft",
  "apply_catalog_resource_fork",
  "prepare_catalog_resource_removal",
  "apply_catalog_resource_removal",
  "apply_catalog_scope_pull",
  "apply_catalog_scope_push",
  "create_catalog_scope",
  "delete_catalog_scope",
  "finish_catalog_unfinished_push",
  "get_catalog_resource_detail",
  "get_catalog_resource_file",
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

/** A capability's words, as a page builds them. */
const nounsOf = (capability: CatalogCapabilityDescriptor) =>
  catalogNouns(capability, { plural: capability.label, singular: capability.singular_label });

type CatalogTree = { readonly [key: string]: string | CatalogTree };
/** Every string of the Library's English catalog, for checks on copy that
 *  no longer lives in the source. */
const libraryCopy = (function flatten(tree: CatalogTree): string[] {
  return Object.values(tree).flatMap((value) => (typeof value === "string" ? [value] : flatten(value)));
})(en.library).join("\n");

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
    readFileSync("src/components/storage-catalog/CatalogItemPage.tsx", "utf8"),
    readFileSync("src/components/storage-catalog/CatalogFileView.tsx", "utf8"),
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
    "createScope", "listScopeResources", "getPreview", "getDetail", "getFile", "linkScopeStorage",
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
  const noun = (capability: CatalogCapabilityDescriptor, count: number) => nounsOf(capability).say((word) => word, count);
  assert.equal(noun(CODEX_SKILLS, 1), "skill");
  assert.equal(noun(CODEX_SKILLS, 3), "skills");
  assert.equal(noun(CODEX_PLUGINS, 0), "plugins");
  assert.equal(noun(CODEX_PETS_V1, 1), "pet v1");
  assert.equal(noun(CODEX_PETS_V2, 2), "pets v2");
  // In English every registered kind's noun is its backend label, lowercased,
  // in the count's number; a shared count line prints the same word.
  for (const capability of CAPABILITIES) {
    assert.equal(noun(capability, 1), capability.singular_label.toLowerCase());
    assert.equal(noun(capability, 4), capability.label.toLowerCase());
    assert.equal(nounsOf(capability).counted(4), capability.label.toLowerCase());
  }
  // A kind this build has no words for is named by its own labels.
  const policies = catalogNouns({ presentation: { icon: "policy", kind_label: "Policies", format_label: "Policy" } }, { plural: "Policies", singular: "Policy" });
  assert.equal(policies.say((word) => word, 1), "policy");
  assert.equal(policies.say((word) => word), "policies", "no count is the plural");
  // A sentence that opens on the noun capitalizes it.
  assert.equal(nounsOf(CODEX_PETS_V1).lead((word) => `${word} info`), "Pet v1 info");
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
  assert.match(setup, /role="radiogroup" aria-label=\{t\("setup\.kind"\)\}[\s\S]*?cards\.map\(\(candidate\) => \{/);
  assert.equal(en.library.setup.kind, "Resource kind");
  assert.match(setup, /className=\{`setup-choice-card\$\{active \? " active" : ""\}`\}/);
  assert.match(setup, /<Icon name=\{catalogPresentation\(candidate\.capability\)\.icon\} size=\{24\} \/>/);
  assert.match(setup, /\{capabilityLabel\(candidate\.capability\)\}/);
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
    /i18nKey="library:list\.onlyInHeading"[\s\S]*?components=\{\{ name: <span className="sync-history-storage-name">\{storageName \?\? t\("status\.anyStorage"\)\}<\/span> \}\}[\s\S]*<h2><Icon name=\{glyph\.icon\} size=\{14\} \/>\{catalogScopeKindLabel\(scope\)\}<\/h2>/,
    "the cloud section renders above the capability's own section",
  );
  assert.equal(en.library.list.onlyInHeading, "Only in <name/>");
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
  assert.match(page, /<LoadingScaffold shape="rows" label=\{t\("page\.loading", \{ label \}\)\} \/>/);
  assert.equal(en.library.page.loading, "Loading {{label}}");
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
  assert.match(page, /t\("info\.copyFailedStatus"\)/);
  assert.equal(en.library.info.copyFailedStatus, "Could not copy.");
  // Reveal in Finder stays off when nothing is local.
  assert.match(page, /revealItemInDir/);
  assert.match(page, /disabled=\{!row\.local_path\}/);
  // The provider mark is a launch button now — the session card's pair,
  // profile-scoped: the mark opens the desktop app on this profile's home,
  // and the terminal button opens the provider CLI there. Both name the
  // action with the backend's provider label, carry no inert span, and
  // report failures on the page surface.
  assert.doesNotMatch(page, /sync-history-provider-inert/);
  assert.match(page, /title=\{t\("card\.openIn", \{ provider: providerLabel \}\)\}[\s\S]{0,200}?<ProviderMark provider=\{provider\} displayName=\{providerLabel\}/);
  assert.equal(en.library.card.openIn, "Open in {{provider}}");
  assert.match(page, /providerLabel=\{profile\.provider_label\}/);
  assert.match(page, /aria-label=\{t\("card\.openInTerminal"\)\}[\s\S]{0,120}?<Icon name="terminal" size=\{14\}/);
  assert.equal(en.library.card.openInTerminal, "Open in Terminal");
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
  assert.match(page, /ariaLabel=\{t\("push\.reviewedLabel", \{ message: warning\.message \}\)\}/);
  assert.equal(en.library.push.reviewedLabel, "Reviewed: {{message}}");
  assert.match(page, /verbs=\{\[t\("push\.markReviewed"\), t\("push\.reviewed"\)\]\}/);
  assert.deepEqual([en.library.push.markReviewed, en.library.push.reviewed], ["Mark reviewed", "Reviewed"]);
  assert.match(page, /verbs=\{\[t\("pull\.approve"\), t\("pull\.approved"\)\]\}/);
  assert.deepEqual([en.library.pull.approve, en.library.pull.approved], ["Approve overwrite", "Approved"]);
  for (const source of [page, libraryCopy]) assert.doesNotMatch(source, /Reviewed this warning — |I understand this replaces/);
  // The count rides the verb at the point of commitment; on Mallard Cloud
  // Push says only Push, as a project's does.
  assert.match(page, /t\("verb\.pushCount", \{ count: pushSelected\.size \}\)/);
  assert.match(page, /t\("verb\.pullCount", \{ count: pullReviewSelectedCount \}\)/);
  assert.deepEqual([en.library.verb.pushCount, en.library.verb.pullCount], ["Push {{count}}", "Pull {{count}}"]);
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
    const key = verb === "Push" ? "push" : "pull";
    assert.match(exitBranch, new RegExp(`title=\\{t\\("${key}\\.exitTitle", \\{ name: itemRow\\?\\.display_name \\?\\? catalogScopeKindLabel\\(scope\\) \\}\\)\\}`));
    assert.equal(en.library[key].exitTitle, `Close ${verb} review — back to {{name}}`);
    assert.match(exitBranch, /<span>\{t\("review\.backTo", \{ name: itemRow\?\.display_name \?\? catalogScopeKindLabel\(scope\) \}\)\}<\/span>/);
    assert.match(exitBranch, /onClick=\{\(\) => setMode\(\{ kind: "list" \}\)\}/);
    assert.doesNotMatch(exitBranch, /btn-primary|btn-ghost|chevron-left/);
    assert.match(commitBranch, new RegExp(`t\\("verb\\.${key}Count"`));
  }
  assert.equal(en.library.review.backTo, "Back to {{name}}");
  // "Library" is not a name the app shows; the sidebar and index say Catalog.
  assert.doesNotMatch(page, /Back to Library/);
  assert.doesNotMatch(libraryCopy, /\bLibrary\b/);
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
  assert.match(page, /sync-path-sync[\s\S]*\{t\("common:pull"\)\}<\/span>[\s\S]*counts\.pull/);
  assert.match(page, /sync-path-sync[\s\S]*\{t\("common:push"\)\}<\/span>[\s\S]*counts\.push/);
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
  assert.match(page, /actionChipLabel\(action\.chosen\.kind, nouns\)/);
  // The verb, chip and danger mark follow the plan's action kind, never its
  // English label read back.
  assert.match(page, /option\.kind === "overwrite_with_storage" \? " danger" : ""/);
  assert.equal(en.library.chip.keepCopy, "Update {{noun}} · keeps local copy");
  assert.match(page, /nouns=\{nouns\}/);
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
  // And the popover keeps the keyboard contract that role implies; the
  // placement hook focuses its first option once placed.
  assert.match(page, /moveMenuFocus\(event\.currentTarget, event\.key\)/);
  // The list is on body, placed against the window: the review's scroll and
  // the storage card both clip, and a list drawn in the row ran off their
  // bottom edge.
  assert.match(page, /useDebugPopoverPlacement\(shellRef, menuRef, expanded, onToggle, false\)/);
  assert.match(page, /<TopLayer>\s*<div\s+ref=\{menuRef\}\s+className="sync-path-menu end"/);
  // Outside is neither the pill nor the list, which is no longer in the row.
  assert.match(page, /shellRef\.current\?\.contains\(target\) \|\| menuRef\.current\?\.contains\(target\)/);
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
  assert.match(add, /title=\{t\("add\.local"\)\}[\s\S]*title=\{t\("add\.cloud"\)\}/);
  assert.deepEqual([en.library.add.local, en.library.add.cloud], ["On this computer", "From Mallard Cloud"]);
  assert.match(add, /className="storage-mode-doors" role="radiogroup"/);
  // From Mallard Cloud is New project's bands: the account, the repository
  // (its kind comes with it), then the row it lands in.
  const cloud = ["bar.storage", "add.repository", "bar.row"].map((key) => add.indexOf(`<strong>{t("${key}")}</strong>`));
  assert.ok(cloud.every((index) => index >= 0));
  assert.deepEqual(cloud, [...cloud].sort((left, right) => left - right));
  assert.deepEqual([en.library.bar.storage, en.library.add.repository, en.library.bar.row], ["Storage", "Repository", "Catalog row"]);
  assert.match(add, /<strong>\{t\("add\.noAccount"\)\}<\/strong>/);
  assert.equal(en.library.add.noAccount, "No Mallard Cloud account yet.");
  assert.match(add, /\{t\("add\.fixed"\)\}/);
  assert.equal(en.library.add.fixed, "Can’t be changed later.");
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
  assert.match(home, /<option value="">\{t\("home\.chooseProfile"\)\}<\/option>/);
  assert.match(home, /<option value="__pending">\{t\("home\.customPath"\)\}<\/option>/);
  assert.match(home, /<Icon name="folder" size=\{13\} \/> \{t\("home\.browse"\)\}/);
  assert.deepEqual([en.library.home.chooseProfile, en.library.home.customPath, en.library.home.browse], ["Choose profile", "Custom path…", "Browse"]);
  assert.match(home, /from "\.\.\/\.\.\/profilePath"/);
  assert.match(home, /profilePathHint\(probe, selection\.path, homePath, \{ display_name: label \}, profiles\)/);
  assert.match(home, /storageCatalogApi\.probeProfile\(provider, path\)/);
  // The project's preselection rule, verbatim: one profile for the provider
  // is chosen, none or several wait on "Choose profile" and the project's
  // field message.
  assert.match(home, /const only = unambiguousProfile\(profiles, key\);/);
  assert.match(home, /t\("home\.chooseFor", \{ provider: label \}\)/);
  assert.equal(en.library.home.chooseFor, "Choose a profile for {{provider}}.");
  // The button names the step it opens, from the saved name or the one the
  // probe would give a typed folder.
  assert.match(home, /homeName \? t\("home\.setUpNamed", \{ name: homeName \}\) : t\("home\.setUp"\)/);
  assert.deepEqual([en.library.home.setUpNamed, en.library.home.setUp], ["Set up {{name}}", "Set up"]);
  assert.match(home, /resolvedProbe\?\.suggested_name/);

  // Step two is the project's setup page for that home: Home, one kind as
  // the shipped cards, Storage as New project's pill row, Finish.
  const bands = ["setup.home", "setup.resource", "bar.storage"].map((key) => setup.indexOf(`<strong>{t("${key}")}</strong>`));
  assert.deepEqual([en.library.setup.home, en.library.setup.resource], ["Home", "Resource"]);
  assert.ok(bands.every((index) => index >= 0), "all three bands render");
  assert.deepEqual(bands, [...bands].sort((left, right) => left - right));
  assert.match(setup, /storageCatalogApi\.inspectProfile\(profile\.profile_id\)/);
  assert.match(setup, /catalogDefaultKind\(cards, initialChoiceKey\)/);
  assert.match(setup, /catalogDefaultStorage\(storages, inspection\?\.last_used_storage_id\)/);
  assert.match(setup, /label=\{t\("setup\.noneYet"\)\}/);
  assert.equal(en.library.setup.noneYet, "None yet");
  assert.match(setup, /storageIconName\(candidate\.provider, candidate\.icon\)/);
  assert.match(setup, /chosenStorage \? t\("setup\.finish"\) : t\("setup\.finishLocally"\)/);
  assert.deepEqual([en.library.setup.finish, en.library.setup.finishLocally], ["Finish", "Finish locally"]);
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
  for (const source of [add, home, setup, libraryCopy]) {
    assert.doesNotMatch(source, /CATALOG_KINDS|Codex only|Codex and Claude/);
  }
  assert.match(add, /sync-inline-new-project sync-setup-workspace/);
  assert.match(setup, /sync-inline-new-project sync-setup-workspace/);
  assert.match(home, /sync-setup-section/);
  assert.match(home, /sync-setup-footer/);
  assert.match(setup, /sync-setup-footer/);
  assert.doesNotMatch(add + home + setup, /Add profile|className="catalog-inline-form"|Manage saved homes|removeProfile|storageCatalogApi\.deleteProfile/);
  assert.doesNotMatch(libraryCopy, /Add profile|Manage saved homes/);
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
  assert.doesNotMatch(home, /Profile name|Save home|catalog-profile-save-row|t\("rename\./);
  assert.doesNotMatch(libraryCopy, /Save home/);
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
  assert.match(page, /label: t\("bar\.rename"\)/);
  assert.equal(en.library.bar.rename, "Rename profile…");
  assert.match(page, /title: t\("bar\.renameTitle", \{ name: profile\.display_name \}\)/);
  assert.match(en.library.bar.renameTitle, /for every Catalog row that uses this home/);
  assert.match(page, /className="sync-path-rename"/);
  assert.match(page, /storageCatalogApi\.renameProfile\(profile\.profile_id, profileNameDraft\.trim\(\), profile\.revision\)/);
  assert.match(page, /\{t\("rename\.useFolder"\)\}/);
  assert.equal(en.library.rename.useFolder, "Use folder name");
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
  assert.match(page, /label: t\("bar\.unlinkFrom", \{ label \}\)/);
  assert.equal(en.library.bar.unlinkFrom, "Unlink from {{label}}…");
  assert.match(page, /storageCatalogApi\.unlinkScopeStorage\(pageLink\.link_id, pageLink\.revision\)/);
  assert.match(page, /t\("unlink\.message", \{/);
  assert.match(en.library.unlink.message, /Push and Pull stop for these \{\{noun\}\}/);
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
  assert.match(page, /title=\{canFork \? forkTitle : t\("card\.nothingToFork"\)\}/);
  assert.equal(en.library.card.nothingToFork, "Not on this Mac, so there is nothing to fork");
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
    "src/components/storage-catalog/CatalogItemPage.tsx",
    "src/components/storage-catalog/CatalogPetDesk.tsx",
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
  assert.match(page, /t\("pull\.claimedBy", \{[\s\S]*t\("pull\.claimedMessage"\)/);
  assert.match(`${en.library.pull.claimedBy} ${en.library.pull.claimedMessage}`, /claimed by[\s\S]*Preserve a copy, acknowledge overwrite, or Skip/);
  assert.match(readFileSync("src/components/storage-catalog/outcomes.ts", "utf8"), /t\("library:outcome\.changesKept"\)/);
  assert.match(en.library.outcome.changesKept, /Completed changes were kept/);
});

test("the Catalog remains independent from project sync", () => {
  const liveFiles = [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/storage-catalog/AddCatalogScopeWorkspace.tsx",
    "src/components/storage-catalog/CatalogHomeStep.tsx",
    "src/components/storage-catalog/CatalogScopeSetupStep.tsx",
    "src/components/storage-catalog/setupFurniture.tsx",
    "src/components/storage-catalog/CatalogItemPage.tsx",
    "src/components/storage-catalog/CatalogPetDesk.tsx",
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
  assert.match(page, /notify\(pushFinished\(\)\)/);
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
  for (const file of ["managed-cloud/ManagedCloudChip.tsx", "managed-cloud/managedCloudLink.ts", "managed-cloud/RepositoryLookupField.tsx", "sync-review/SyncOutcomeNotice.tsx", "sync-review/reviewPreparationLifecycle.ts", "sync-review/PageBack.tsx"]) {
    const source = readFileSync(`src/components/${file}`, "utf8");
    assert.doesNotMatch(source, /project-sync|storage-catalog|from ["'].*ipc["']/);
  }
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  for (const source of [page, readFileSync("src/components/storage-catalog/outcomes.ts", "utf8"), libraryCopy]) {
    assert.doesNotMatch(source, /nothing was published|nothing was changed/);
  }
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
      pushPlan: plan, pushSelected: new Set(["duck"]), pushAcks: new Set(), setProgress: noop, nouns: nounsOf(CODEX_PETS_V1),
      t: i18n.getFixedT(null, ["library", "common"]),
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
  const item = { kind: "item" as const, coordinate: pet, nouns: nounsOf(CODEX_PETS_V2), resourceId: "duck", name: "Duck", offerNew: true };
  const add = { kind: "add" as const, coordinate: pet, nouns: nounsOf(CODEX_PETS_V2), rowLabel: "Pets v2 · Codex home", listed: new Set(["duck"]) };
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
      nouns={nounsOf(CODEX_PETS_V2)} rowLabel="Pets v2 · Codex home" listed={new Set()} capabilities={CAPABILITIES}
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
  assert.match(dialog, /t\("add\.fixed"\)/);
  assert.match(dialog, /storageCatalogApi\.createCloudRepository\(/);
  assert.match(dialog, /storageCatalogApi\.connectCloudRepository\(/);
  assert.doesNotMatch(dialog, /preparePull|applyPull|openPull|Connect and review|Disconnect/);
  assert.doesNotMatch(libraryCopy, /Connect and review|Disconnect/);
});

test("every storage's item has its own page, and on Mallard Cloud its repository lives in its bar", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The list carries no Cloud chip and no per-row connect; its names open
  // the item's page on every storage.
  assert.doesNotMatch(page, /CatalogCloudItem|CatalogCloudConnections|Connect Cloud repository|>Cloud repositories</);
  assert.doesNotMatch(libraryCopy, /Connect Cloud repository|^Cloud repositories$/m);
  assert.match(page, /onOpen=\{resourceId && !itemRow \?/);
  assert.match(page, /const itemRow = itemId \? rowByResource\.get\(itemId\) \?\? null : null;/);
  // What only Mallard Cloud has stays Mallard Cloud's: the first Push's
  // repository card, and the several-repositories note.
  assert.match(page, /\{managedCloud && !itemRepository && itemRow\.state !== "blocked" && !pageCloud && \(/);
  assert.match(page, /if \(managedCloud && itemRow\?\.state === "blocked" && itemRow\.resource_id\) \{/);
  assert.match(page, /className="sync-history-session-title-button" title=\{nouns\.say\(\(noun\) => t\("card\.viewDetails", \{ name: row\.display_name, noun \}\), 1\)\}/);
  assert.equal(en.library.card.viewDetails, "{{name}} · View {{noun}} details");
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
  assert.doesNotMatch(page, /sync-path-icon-action"\s+title=(?:"Add from Mallard Cloud…"|\{t\("cloud\.addFrom"\)\})/);
  assert.equal((page.match(/t\("cloud\.addFrom"\)/g) ?? []).length, 1, "Add from Mallard Cloud… is one button, the list head's");
  assert.match(page, /const addFromCloud = managedCloud && \(\s*<button\s+type="button"\s+className="btn"\s+aria-haspopup="dialog"/);
  assert.match(page, /<NameSearchField[^/]*count=\{!managedCloud\} trailing=\{addFromCloud\} \/>/);
  assert.match(page, /t\("list\.empty", \{ noun \}\), 1\)\}\s*\{addFromCloud\}/);
  assert.match(en.library.list.empty, /none stored yet\.$/);
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
  assert.match(page, /t\("review\.readingLatest", \{ storage: storageWord \}\)/);
  assert.match(page, /t\("review\.comparingWith", \{ storage: storageWord \}\)/);
  assert.deepEqual(
    [en.library.review.readingLatest, en.library.review.comparingWith],
    ["Reading the latest version from {{storage}}", "Comparing this computer with {{storage}}"],
  );
  assert.match(page, /const outcomeCard = managedCloud && outcomeNotice/);
  assert.match(page, /managedCloud \? \(pushing \? t\("verb\.pushing"\) : t\("common:push"\)\) : t\("verb\.pushCount", \{ count: pushSelected\.size \}\)/);
  assert.match(page, /t\("push\.cantCancel"\)/);
  assert.equal(en.library.push.cantCancel, "A running push finishes or fails whole — it can’t be cancelled midway.");
  assert.match(page, /t\("pull\.cantCancel"\)/);
  assert.equal(en.library.pull.cantCancel, "Pull cannot be cancelled while changes are being applied.");
  assert.match(page, /\? cloudPushOutcome\(result, \{/);
  const api = readFileSync("src/components/storage-catalog/api.ts", "utf8");
  assert.doesNotMatch(api, /disconnect/);
  const chip = readFileSync("src/components/managed-cloud/ManagedCloudChip.tsx", "utf8");
  assert.doesNotMatch(chip, /Change repository…|Disconnect repository|onReconnect|onDisconnect/);
});

test("a Mallard Cloud outcome speaks the project's words: what moved, its repository, one version", async () => {
  const { cloudPushOutcome, cloudPullOutcome, pushOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const addresses: Record<string, string> = { duck: "Rika321/Duck", pip: "kai/Pip", mossy: "Rika321/Mossy" };
  const words = { nouns: nounsOf(CODEX_PETS_V2), addressOf: (id: string) => addresses[id], at: 100 };
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

/* ——— an item's page ——— */

function itemRow(overrides: Partial<CatalogScopeRow> = {}): CatalogScopeRow {
  return {
    row_id: "row-release-notes",
    display_name: "release-notes",
    resource_id: "release-notes",
    state: "local_changed",
    local_present: true,
    local_path: "/Users/you/.claude/skills/release-notes",
    local_modified_at: 1_790_000_000,
    local_files: 2,
    local_bytes: 4_096,
    remote_present: true,
    remote_updated_at: 1_789_000_000,
    content_sha256: "a".repeat(64),
    preview_png: null,
    description: null,
    warnings: [],
    ...overrides,
  };
}

const itemActions = {
  provider: "claude",
  providerLabel: "Claude",
  onOpenApp: () => {},
  onOpenTerminal: () => {},
  onFork: () => {},
  forkTitle: "Fork release-notes",
  onReveal: () => {},
};

const itemFiles = { open: null, onOpen: () => {}, onReveal: () => {} };

/* From an item, the way back is its list, named as its row is: the app's own word. */
const itemBack = { voice: "app" as const, label: "Skills", onActivate: () => {} };

test("an item's page holds its body and what only this Mac knows, each said once", async () => {
  const { default: CatalogItemPage } = await import("../../src/components/storage-catalog/CatalogItemPage");
  const text = [
    "---",
    "name: release-notes",
    "description: Drafts release notes from the commits since the last tag.",
    "allowed-tools: Bash(git log:*)",
    "---",
    "# Release notes",
    "",
    "![diagram](https://example.com/diagram.png)",
    "",
    "<script>alert(1)</script>",
  ].join("\n");
  const html = renderToStaticMarkup(
    <CatalogItemPage
      row={itemRow()}
      back={itemBack}
      detail={{ status: "ready", detail: { kind: "document", path: "SKILL.md", text, files: [
        { path: "SKILL.md", size: 1_644, executable: false },
        { path: "scripts/range.sh", size: 2_444, executable: true },
      ] } }}
      actions={itemActions}
      files={itemFiles}
      onOpenExternal={() => {}}
    />,
  );
  // The body: the frontmatter's description, SKILL.md without its
  // frontmatter, and every file, executables marked.
  assert.match(html, /<p class="catalog-item-lede">Drafts release notes from the commits since the last tag\.<\/p>/);
  assert.match(html, /<h1>Release notes<\/h1>/);
  assert.doesNotMatch(html, /allowed-tools|<script/);
  assert.match(html, /<button type="button" class="catalog-item-file">.*?<code>scripts\/range\.sh<\/code><\/button><span class="catalog-item-exec" title="Executable"/);
  // An image in a skill is never fetched.
  assert.match(html, /markdown-image-placeholder/);
  assert.doesNotMatch(html, /<img/);
  // The bar names the item, its kind, provider and sync state; the page
  // repeats none of them. Its way back names the row it returns to — the one
  // crumb the page says again, as a session's page names its section.
  assert.doesNotMatch(html, />release-notes</);
  assert.doesNotMatch(html, /sync-history-branch-state|thread-status-icon|>Skill<|>Claude</);
  // This Mac's facts, then the launch run with Reveal in Finder last, once in
  // the rail and once in the line it folds into.
  assert.match(html, /<dt>Edited<\/dt><dd><time/);
  assert.match(html, /<dt>Size<\/dt><dd>4\.0 KB<\/dd>/);
  const runs = [...html.matchAll(/<div class="sync-history-thread-actions catalog-item-actions">(.*?)<\/div>/g)];
  assert.equal(runs.length, 2);
  for (const [, run] of runs) {
    const labels = [...run.matchAll(/aria-label="([^"]+)"/g)].map((match) => match[1]);
    assert.deepEqual(labels, ["Open in Claude", "Open in Terminal", "Fork release-notes", "Reveal release-notes in Finder"]);
  }
  assert.doesNotMatch(html, /\/Users\/you\/\.claude\/skills\/release-notes<\//);
});

// The session page's way back, on a Catalog item: the first item in the page's
// facts line, so it leads the page at every width, and one level up the trail
// as the crumbs walk it.
test("an item's page leads its facts line with the way back to its list", async () => {
  const { default: CatalogItemPage } = await import("../../src/components/storage-catalog/CatalogItemPage");
  const html = renderToStaticMarkup(
    <CatalogItemPage
      row={itemRow({ display_name: "Tracebit", resource_id: "tracebit" })}
      back={{ ...itemBack, label: "Pets v1" }}
      detail={{ status: "loading" }}
      actions={itemActions}
      files={itemFiles}
      onOpenExternal={() => {}}
    />,
  );
  // Before the body has loaded, and ahead of the facts: the way out never
  // waits on the read.
  assert.match(html, /^<div class="catalog-item-page"><div class="catalog-item-line"><button type="button" class="btn sync-page-back" title="Back to Pets v1" aria-label="Back to Pets v1"><svg[^>]*data-icon="chevron-left"[\s\S]*?<\/svg><span>Pets v1<\/span><\/button><div class="catalog-item-facts-line">/);
  assert.doesNotMatch(html, /sync-page-back[^>]*disabled/);
  assert.equal(html.match(/sync-page-back/g)?.length, 1);

  // The page and the crumbs share one way up: an open file returns to its
  // item, named as the user named it; the item returns to its row's list.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /const closeItemFile = \(\) => setItemFile\(null\);\s*const closeItem = \(\) => \{\s*setMode\(\{ kind: "list" \}\);\s*setItemId\(null\);\s*\};/);
  assert.match(page, /onActivate: itemRow \? closeItem : undefined,/);
  assert.match(page, /onActivate: openFilePath \? closeItemFile : undefined,/);
  assert.match(page, /back=\{openFilePath\s*\? \{ voice: "name", label: name, onActivate: closeItemFile \}\s*: \{ voice: "app", label: catalogScopeKindLabel\(scope\), onActivate: closeItem \}\}/);
});

test("a plugin's page is the store hero with this Mac's install state and the adapter's lines", async () => {
  const { default: CatalogItemPage } = await import("../../src/components/storage-catalog/CatalogItemPage");
  const plugin = (overrides: Partial<Extract<import("../../src/components/storage-catalog/types").CatalogResourceDetail, { kind: "install" }>> = {}) => ({
    kind: "install" as const,
    plugin_id: "lint-buddy@acme-tools",
    marketplace: "acme-tools",
    location: "acme-tools/claude-plugins",
    git_ref: null,
    install: ["claude plugin marketplace add acme-tools/claude-plugins --scope user", "claude plugin install lint-buddy@acme-tools --scope user"],
    installed_version: "1.4.0",
    listing: {
      display_name: "Lint Buddy", developer: "Acme Tools", description: "Runs your linters.", category: "Code quality",
      website_url: "https://example.com/lint-buddy", privacy_url: null, terms_url: null,
    },
    ...overrides,
  });
  const render = (row: CatalogScopeRow, detail: ReturnType<typeof plugin>) => renderToStaticMarkup(
    <CatalogItemPage row={row} back={{ ...itemBack, label: "Plugins" }} detail={{ status: "ready", detail }} actions={{ ...itemActions, onFork: undefined }} files={itemFiles} onOpenExternal={() => {}} />,
  );
  const installed = render(itemRow({ display_name: "lint-buddy@acme-tools", local_files: null, local_bytes: null }), plugin());
  assert.match(installed, /<b>Lint Buddy<\/b> by Acme Tools · Code quality/);
  assert.match(installed, /Installed · 1\.4\.0/);
  assert.match(installed, /<span class="catalog-item-prompt" aria-hidden="true">\$ <\/span>claude plugin install lint-buddy@acme-tools --scope user/);
  assert.match(installed, /aria-label="Copy claude plugin marketplace add acme-tools\/claude-plugins --scope user &amp;&amp; claude plugin install/);
  assert.match(installed, /acme-tools\/claude-plugins<svg/);
  assert.match(installed, />Website<svg/);
  // Stored only: Pull is the install. No printable lines: no box.
  const stored = render(
    itemRow({ local_present: false, local_path: null, local_modified_at: null, local_files: null, local_bytes: null }),
    plugin({ installed_version: null, listing: null, install: [] }),
  );
  assert.match(stored, /Pull installs it/);
  assert.doesNotMatch(stored, /catalog-item-install|Installed ·/);
  assert.match(stored, /<dt>Stored<\/dt>/);
  // Not on this Mac: nothing to fork or reveal.
  assert.match(stored, /disabled="" title="Not on this Mac"/);
});

test("a pet's page plays its atlas on the desk, with Pause, and says what it is once", async () => {
  const { default: CatalogItemPage } = await import("../../src/components/storage-catalog/CatalogItemPage");
  const html = renderToStaticMarkup(
    <CatalogItemPage
      row={itemRow({ display_name: "Mossy", resource_id: "mossy" })}
      back={{ ...itemBack, label: "Pets v1" }}
      detail={{ status: "ready", detail: { kind: "sprite", atlas: "data:image/webp;base64,AAAA", columns: 8, rows: 11, description: "A mallard who naps." } }}
      actions={{ ...itemActions, provider: "codex", providerLabel: "Codex", forkTitle: "Fork Mossy" }}
      files={itemFiles}
      onOpenExternal={() => {}}
    />,
  );
  assert.match(html, /class="catalog-pet-desk"/);
  for (const moment of ["Idle", "Working", "Needs you", "Review", "Failed"]) {
    assert.match(html, new RegExp(`class="catalog-pet-moment"[^>]*>${moment}<`));
  }
  assert.match(html, />Pause</);
  assert.equal(html.match(/A mallard who naps\./g)?.length, 1);
  // The player: Reduce Motion starts it paused, and a v2 atlas turns to look.
  const desk = readFileSync("src/components/storage-catalog/CatalogPetDesk.tsx", "utf8");
  assert.match(desk, /const playing = !\(paused \?\? reduced\);/);
  assert.match(desk, /const looks = rows >= 11;/);
});

test("off Mallard Cloud, a blocked item's page says why it can't be synced", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /\{!managedCloud && itemRow\.state === "blocked" && \(\s*<Notice severity="warning" timing="standing" label=\{t\("item\.whyBlocked", \{ name \}\)\} title=\{t\("item\.blocked", \{ name \}\)\}>\s*\{itemRow\.blocker \?\? /);
  assert.deepEqual([en.library.item.whyBlocked, en.library.item.blocked], ["Why {{name}} can't be synced", "{{name}} can't be synced."]);
});

test("SKILL.md's frontmatter splits the way Mallard Cloud's skill page splits it", async () => {
  const { splitSkillFrontmatter } = await import("../../src/components/storage-catalog/skillFiles");
  const split = splitSkillFrontmatter("---\nname: 'it''s'\ndescription: >\n  Two\n  lines.\nmetadata:\n  nested: skipped\n---\nBody\n");
  assert.deepEqual(split.fields, { name: "it's", description: "Two lines." });
  assert.equal(split.body, "Body\n");
  assert.deepEqual(splitSkillFrontmatter("No frontmatter."), { fields: {}, body: "No frontmatter." });
});

test("an item's rail sits beside its body from 880px of page, and joins the way back's line under that", () => {
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.catalog-item-page \{[^}]*container: catalog-item \/ inline-size;/);
  assert.match(css, /\.catalog-item-rail \{[^}]*display: none;/);
  // Whatever stands above the item (the first Push's repository card, an
  // outcome, a blocked note) keeps a heading's space from it; nothing above,
  // no space.
  assert.match(css, /\.profile-links-section > \* \+ \.catalog-item-page \{\s*margin-top: 18px;\s*\}/);
  assert.match(css, /@container catalog-item \(min-width: 880px\) \{\s*\.catalog-item-line > \.catalog-item-facts-line,\s*\.catalog-item-line > \.catalog-item-actions \{\s*display: none;\s*\}\s*\.catalog-item-hub \{\s*grid-template-columns: minmax\(0, 1fr\) 260px;\s*\}\s*\.catalog-item-rail \{\s*display: flex;\s*\}/);
  assert.doesNotMatch(css, /\.catalog-item-line \{\s*display: none;/);
});

test("the app's one Markdown renderer belongs to neither feature", () => {
  const renderer = readFileSync("src/components/markdown/MarkdownText.tsx", "utf8");
  assert.match(renderer, /skipHtml/);
  // Every destination passes `safeMarkdownUrl`, except an href the page
  // itself resolves to one of its own files.
  assert.match(renderer, /\(url: string\) => \(fileLink\?\.resolve\(url\) \? url : safeMarkdownUrl\(resolveUrl \? resolveUrl\(url\) : url\)\)/);
  assert.match(renderer, /urlTransform=\{urlTransform\}/);
  // An image is a chip, never fetched; without alt text it names its kind.
  assert.match(renderer, /img: \(\{ alt \}\) => <MarkdownImagePlaceholder label=\{alt\} \/>/);
  assert.match(renderer, /const label = given \|\| word;/);
  for (const file of ["src/components/markdown/MarkdownText.tsx", "src/components/markdown/markdownUrl.ts", "src/components/markdown/remarkLatexMath.ts"]) {
    assert.doesNotMatch(readFileSync(file, "utf8"), /project-sync|storage-catalog/, file);
  }
  assert.match(readFileSync("src/components/project-sync/session-detail/SessionDetailEvent.tsx", "utf8"), /from "\.\.\/\.\.\/markdown\/MarkdownText"/);
  assert.match(readFileSync("src/components/storage-catalog/CatalogItemPage.tsx", "utf8"), /from "\.\.\/markdown\/MarkdownText"/);
});

test("a pet of any script predicts its own address, and a title with nothing usable is a refusal", async () => {
  const { nameRefusal, predictRepositoryAddress } = await import("../../src/components/managed-cloud/repositoryAddress");
  assert.deepEqual(predictRepositoryAddress("Rika321", "大黄", []), { kind: "address", address: "Rika321/大黄", name: "大黄", base: "大黄", suffixed: false });
  assert.deepEqual(predictRepositoryAddress("Rika321", "大黄", [{ owner_login: "Rika321", name: "大黄" }]), { kind: "address", address: "Rika321/大黄-2", name: "大黄-2", base: "大黄", suffixed: true });
  assert.deepEqual(predictRepositoryAddress("Rika321", "🦆🦆", []), { kind: "refused" });
  assert.equal(nameRefusal("item"), "Name needs a letter or digit; rename the item first.");
  // The item page's repository card and the chooser's New row print the prediction, or that one sentence.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /prediction\.kind === "address" \? \(accountLogin \? prediction\.address : prediction\.name\) : nameRefusal\("item"\)/);
  const chooser = readFileSync("src/components/storage-catalog/CloudRepositoryChoice.tsx", "utf8");
  assert.match(chooser, /<span className="repo-new-refusal">\{nameRefusal\("item"\)\}<\/span>/);
});

test("local removal button follows local presence, safety and busy state", async () => {
  const { default: CatalogRemoveButton } = await import("../../src/components/storage-catalog/CatalogRemoveButton");
  const row = { local_present: true, resource_id: "one", blocker: null } as CatalogScopeRow;
  const action = { label: "Delete local pet", busy: false, removing: false, onRemove() {} };
  assert.match(renderToStaticMarkup(<CatalogRemoveButton row={row} action={action} />), /aria-label="Delete local pet"/);
  assert.equal(renderToStaticMarkup(<CatalogRemoveButton row={{ ...row, local_present: false }} action={action} />), "");
  assert.equal(renderToStaticMarkup(<CatalogRemoveButton row={row} />), "");
  assert.match(renderToStaticMarkup(<CatalogRemoveButton row={{ ...row, blocker: "Unsafe folder" }} action={action} />), /disabled/);
  assert.match(renderToStaticMarkup(<CatalogRemoveButton row={row} action={{ ...action, busy: true, removing: true }} />), /icon-spin/);
});

test("removal cancellation and page changes never apply", async () => {
  const { removeLocalCatalogResource } = await import("../../src/components/storage-catalog/removalFlow");
  const plan = { plan_id: "review", scope_id: "scope", resource_id: "item", display_name: "Duck", uninstall: false, file_count: 3, created_at: 1, expires_at: 2 };
  for (const approved of [false, true]) {
    let current = true;
    let applied = false;
    const result = await removeLocalCatalogResource({
      nouns: nounsOf(CODEX_PETS_V1), prepare: async () => plan,
      confirm: async (message, options) => {
        assert.match(message, /3 files/);
        assert.equal(options.okLabel, "Delete");
        if (approved) current = false;
        return approved;
      },
      apply: async () => { applied = true; throw new Error("must not apply"); },
      refresh: async () => { throw new Error("must not refresh"); }, isCurrent: () => current,
    });
    assert.equal(result, null);
    assert.equal(applied, false);
  }
});

test("plugin removal uses uninstall copy and refreshes after success or failure", async () => {
  const { removeLocalCatalogResource } = await import("../../src/components/storage-catalog/removalFlow");
  for (const fail of [false, true]) {
    let refreshed = 0;
    const work = removeLocalCatalogResource({
      nouns: nounsOf(CODEX_PLUGINS),
      prepare: async () => ({ plan_id: "review", scope_id: "scope", resource_id: "plugin", display_name: "Plugin", uninstall: true, file_count: 0, created_at: 1, expires_at: 2 }),
      confirm: async (message, options) => { assert.match(message, /user installation/); assert.equal(options.okLabel, "Uninstall"); return true; },
      apply: async (id) => { assert.equal(id, "review"); if (fail) throw new Error("provider failed"); return { removed: true, message: "Removed" }; },
      refresh: async () => { refreshed++; }, isCurrent: () => true,
    });
    if (fail) await assert.rejects(work, /provider failed/);
    else assert.equal((await work)?.removed, true);
    assert.equal(refreshed, 1);
  }
});

/* ——— the list's status scopes ——— */

const SCOPED_ROWS: CatalogScopeRow[] = ([
  ["release-notes", "local_changed", true, true],
  ["pr-review", "new", true, false],
  ["changelog-voice", "remote_missing", true, false],
  ["sql-helper", "storage_changed", true, true],
  ["figma-to-code", "local_missing", false, true],
  ["ghost-notes", "local_missing", false, false],
  ["api-docs", "diverged", true, true],
  ["onboarding", "unreviewed_remote", true, true],
  ["secrets-scan", "blocked", true, true],
  ["commit-message", "in_sync", true, true],
] as const).map(([name, state, local, remote]) => itemRow({
  row_id: `row-${name}`, display_name: name, resource_id: name, state, local_present: local, remote_present: remote,
}));

test("a Library row's status scope is the badge it wears", () => {
  const expected: Record<string, StatusScope> = {
    "release-notes": "push",
    "pr-review": "push",
    "changelog-voice": "push",
    "sql-helper": "pull",
    "figma-to-code": "pull",
    "ghost-notes": "pull",
    "api-docs": "attention",
    "onboarding": "attention",
    "secrets-scan": "attention",
    "commit-message": "synced",
  };
  for (const row of SCOPED_ROWS) {
    assert.equal(catalogStatusScope(row), expected[row.display_name], row.display_name);
  }
  // No storage chosen: every row is uncompared, so the strip has one scope
  // and is not drawn.
  const uncompared = SCOPED_ROWS.map((row) => ({ ...row, state: null }));
  assert.deepEqual([...catalogStatusScopeCounts(uncompared)], [["unknown", SCOPED_ROWS.length]]);
});

test("each Library status scope counts the rows it draws, as the bar does", () => {
  const counts = catalogStatusScopeCounts(SCOPED_ROWS);
  assert.deepEqual(Object.fromEntries(counts), { push: 3, pull: 3, attention: 3, synced: 1 });
  for (const [scope, count] of counts) {
    assert.equal(catalogMatchingRows(SCOPED_ROWS, "", scope).length, count, scope);
  }
  assert.equal(catalogMatchingRows(SCOPED_ROWS, "", null).length, SCOPED_ROWS.length);
  // The bar's Push, Pull and attention numbers read the same scopes.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(
    page,
    /const scope = catalogStatusScope\(row\);\s*if \(scope === "push" \|\| scope === "pull" \|\| scope === "attention"\) totals\[scope\] \+= 1;/,
  );
});

test("a status scope and the name search narrow the Library list together", () => {
  const names = (rows: CatalogScopeRow[]) => rows.map((row) => row.display_name);
  assert.deepEqual(names(catalogMatchingRows(SCOPED_ROWS, "re", "push")), ["release-notes", "pr-review"]);
  assert.deepEqual(names(catalogMatchingRows(SCOPED_ROWS, "notes", "pull")), ["ghost-notes"]);
  // A scope can empty the list while the name alone still matches: that is
  // the one case the empty box offers Clear filter.
  assert.deepEqual(catalogMatchingRows(SCOPED_ROWS, "release", "attention"), []);
  assert.equal(catalogMatchingRows(SCOPED_ROWS, "release", null).length, 1);
});

test("the Library list wears the project faces' status strip", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Under the list's own search, which counts "3 of 13" while a scope holds.
  assert.match(
    page,
    /narrowed=\{statusScope !== null\} count=\{!managedCloud\} trailing=\{addFromCloud\} \/>\s*\{scopeBarShown && \([\s\S]*?<StatusScopeBar\s+label=\{nouns\.say\(\(noun\) => t\("list\.showByStatus", \{ noun \}\)\)\}/,
  );
  // The list only: no strip in a review or on an item's page.
  assert.equal(page.match(/<StatusScopeBar/g)?.length, 1);
  // Drawn with two scopes, or while one is chosen; the chosen one stays at 0.
  assert.match(page, /const scopeBarShown = rows\.length > 0 && \(scopeCounts\.size > 1 \|\| chosenStatusScope !== null\);/);
  assert.match(page, /\(scopeCounts\.get\(each\.id\) \?\? 0\) > 0 \|\| each\.id === statusScope/);
  assert.match(page, /const matchingRows = catalogMatchingRows\(rows, searchQuery, statusScope\);/);
  // Clear filter only when releasing the scope brings rows back.
  assert.match(
    page,
    /const scopeEmptiedList = statusScope !== null && matchingRows\.length === 0\s*&& rows\.some\(\(row\) => nameMatchesSearch\(row\.display_name, searchQuery\)\);/,
  );
  assert.match(page, /\{scopeEmptiedList && \(\s*<button type="button" className="btn btn-ghost" onClick=\{clearStatusScope\}>\{t\("list\.clearFilter"\)\}<\/button>/);
  assert.deepEqual([en.library.list.showByStatus, en.library.list.clearFilter], ["Show {{noun}} by status", "Clear filter"]);
  // The scope lives and resets with the search.
  assert.match(
    page,
    /setSearchQuery\(""\);\s*setChosenStatusScope\(null\);\s*\}, \[scope\.scope\.scope_id, chosenStorageId, mode\.kind\]\);/,
  );
});

/* ——— a skill's files, each opened as its own page ——— */

const skillFile = (overrides: Partial<import("../../src/components/storage-catalog/types").CatalogResourceFile> = {}) => ({
  path: "scripts/range.sh",
  size: 31,
  executable: true,
  text: "#!/bin/sh\nset -eu\necho range\n",
  local_path: "/Users/you/.claude/skills/release-notes/scripts/range.sh",
  ...overrides,
});

const openFilePage = async (file: ReturnType<typeof skillFile>, row = itemRow()) => {
  const { default: CatalogItemPage } = await import("../../src/components/storage-catalog/CatalogItemPage");
  return renderToStaticMarkup(
    <CatalogItemPage
      row={row}
      back={{ voice: "name", label: row.display_name, onActivate: () => {} }}
      detail={{ status: "ready", detail: { kind: "document", path: "SKILL.md", text: "# Release notes\n", files: [
        { path: "SKILL.md", size: 16, executable: false },
        { path: "references/voice.md", size: 40, executable: false },
        { path: "scripts/range.sh", size: 31, executable: true },
      ] } }}
      actions={itemActions}
      files={{ ...itemFiles, open: { path: file.path, state: { status: "ready", file } } }}
      onOpenExternal={() => {}}
    />,
  );
};

test("an open file is the page: numbered lines, Copy and Reveal, and nothing of the item's", async () => {
  const html = await openFilePage(skillFile());
  assert.equal(html.match(/<span class="catalog-file-line">/g)?.length, 3);
  assert.match(html, /style="--catalog-file-gutter:2ch"/);
  assert.match(html, /<span class="catalog-item-exec" title="Executable — shown, never run" aria-label="Executable">/);
  assert.match(html, /<span>3 lines · 31 B<\/span>/);
  assert.match(html, /aria-label="Copy the contents of scripts\/range\.sh"/);
  assert.match(html, /title="Reveal \/Users\/you\/\.claude\/skills\/release-notes\/scripts\/range\.sh in Finder"/);
  // The bar's last crumb names the file, and the file page is the file alone
  // under its way back to the item.
  assert.match(html, /^<div class="catalog-item-page"><div class="catalog-item-line"><button type="button" class="btn sync-page-back named" title="Back to release-notes" aria-label="Back to release-notes"><svg[^>]*data-icon="chevron-left"[\s\S]*?<\/svg><span>release-notes<\/span><\/button><\/div><section class="catalog-item-panel"/);
  assert.doesNotMatch(html, /catalog-item-rail|catalog-item-facts-line|catalog-item-files|>scripts\/range\.sh</);
  // The storage's copy has nothing on this Mac to reveal.
  const stored = await openFilePage(skillFile({ local_path: null }));
  assert.match(stored, /disabled="" title="Not on this Mac" aria-label="Reveal scripts\/range\.sh in Finder"/);
});

test("a file that is not text says so, and Markdown opens in Preview except SKILL.md", async () => {
  const binary = await openFilePage(skillFile({ path: "scripts/__pycache__/range.pyc", executable: false, size: 2_048, text: null }));
  assert.match(binary, /Binary file · not shown/);
  assert.doesNotMatch(binary, /catalog-file-code|Copy the contents/);
  assert.match(binary, /<span>2\.0 KB<\/span>/);

  const notes = await openFilePage(skillFile({ path: "references/voice.md", executable: false, text: "---\nname: voice\n---\n# Voice\n\nSee [the script](../scripts/range.sh).\n" }));
  assert.match(notes, /aria-pressed="true" class="active">Preview</);
  assert.match(notes, /<h1>Voice<\/h1>/);
  assert.doesNotMatch(notes, /name: voice/);
  // A relative link resolves against the open file's own folder.
  assert.match(notes, /<a href="scripts\/range\.sh">the script<\/a>/);

  const skillMd = await openFilePage(skillFile({ path: "SKILL.md", executable: false, text: "---\nname: release-notes\n---\n# Release notes\n" }));
  assert.match(skillMd, /aria-pressed="true" class="active">Source</);
  assert.match(skillMd, /<span class="catalog-file-line">name: release-notes\n<\/span>/);
});

test("a skill's relative links name its own files and stay inside it", async () => {
  const { resolveSkillLink } = await import("../../src/components/storage-catalog/skillFiles");
  const listed = new Set(["SKILL.md", "references/a.md", "scripts/x.py"]);
  // The same vectors as Mallard Cloud's skill page.
  assert.equal(resolveSkillLink("SKILL.md", "references/a.md", listed), "references/a.md");
  assert.equal(resolveSkillLink("SKILL.md", "./references/a.md", listed), "references/a.md");
  assert.equal(resolveSkillLink("references/a.md", "../scripts/x.py", listed), "scripts/x.py");
  assert.equal(resolveSkillLink("references/a.md", "../../x", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "missing.md", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "https://e.com/x", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "#top", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "/etc/passwd", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "references%2Fa.md", listed), "references/a.md");
  assert.equal(resolveSkillLink("SKILL.md", "references/a.md?raw=1", listed), null);
  assert.equal(resolveSkillLink("SKILL.md", "%E0%A4%A", listed), null);
});

test("the item's rows and SKILL.md's links open its files; without a file link Markdown keeps them text", async () => {
  const { default: MarkdownText } = await import("../../src/components/markdown/MarkdownText");
  const text = "Read [the notes](references/a.md) and [this](../outside.md).";
  const plain = renderToStaticMarkup(<MarkdownText text={text} />);
  assert.doesNotMatch(plain, /<a /);
  const listed = new Set(["references/a.md"]);
  const { resolveSkillLink } = await import("../../src/components/storage-catalog/skillFiles");
  const linked = renderToStaticMarkup(
    <MarkdownText text={text} fileLink={{ resolve: (href) => resolveSkillLink("SKILL.md", href, listed), open: () => {} }} />,
  );
  assert.match(linked, /<a href="references\/a\.md">the notes<\/a>/);
  assert.match(linked, /<span>this<\/span>/);
  // Neither link carries react-markdown's syntax tree as an attribute.
  const external = renderToStaticMarkup(<MarkdownText text="[x](https://example.com)" onOpenExternal={() => {}} />);
  assert.equal(external, '<p><a href="https://example.com/" rel="noopener noreferrer">x</a></p>');
  assert.doesNotMatch(linked, /node=/);

  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The open file belongs to the scope, storage and item it was opened on.
  assert.match(page, /const itemFileOwner = `\$\{scope\.scope\.scope_id\}\\u0000\$\{storageId \?\? ""\}\\u0000\$\{itemId \?\? ""\}`;/);
  assert.match(page, /if \(itemFileFor !== itemFileOwner\) \{\s*setItemFileFor\(itemFileOwner\);\s*setItemFile\(null\);\s*\}/);
  assert.match(page, /const openFilePath = itemFileFor === itemFileOwner \? itemFile : null;/);
  // The item crumb is a way back; the file is the leaf, its menu the other files.
  assert.match(page, /onActivate: openFilePath \? closeItemFile : undefined,/);
  assert.match(page, /id: "file",\s*label: baseName\(openFilePath\),\s*title: openFilePath,/);
  assert.match(page, /items: itemFiles\.filter\(\(file\) => file\.path !== openFilePath\)/);
});

/* ——— in Chinese ——— */

/** `t` for a key the test builds while walking the catalog. */
const anyKey = i18n.t.bind(i18n) as unknown as (key: string, options?: Record<string, unknown>) => string;

/** Renders with the interface in Chinese, then puts English back. */
function inChinese<T>(fn: () => T): T {
  void i18n.changeLanguage("zh-CN");
  try {
    return fn();
  } finally {
    void i18n.changeLanguage("en");
  }
}

const HAN = "\\p{Script=Han}";
/** Han against a Latin letter or digit with no space between: the one
 *  spacing fault a noun arriving in a sentence could make. */
const UNSPACED = new RegExp(`${HAN}[A-Za-z0-9]|[A-Za-z0-9]${HAN}`, "u");

test("in Chinese a kind's noun is Skill, 插件 or 宠物, spaced from Han only when it is Latin", async () => {
  const { zhCN } = await import("../../src/i18n/zh-CN");
  const catalog = zhCN.library as CatalogTree;
  const sentences = (function walk(tree: CatalogTree, prefix: string): Array<[string, string]> {
    return Object.entries(tree).flatMap(([key, value]) => (typeof value === "string" ? [[`${prefix}${key}`, value] as [string, string]] : walk(value, `${prefix}${key}.`)));
  })(catalog, "");
  // The catalog writes {{noun}} straight against the Han beside it; the
  // space a Latin noun needs is put in when it arrives.
  for (const [key, text] of sentences) {
    assert.doesNotMatch(text, new RegExp(`${HAN} \\{\\{(noun|wanted)\\}\\}|\\{\\{(noun|wanted)\\}\\} ${HAN}`, "u"), key);
  }
  inChinese(() => {
    /* Every other value is Han, so only the noun's own edges can set
       Latin against Han. */
    const said = (capability: CatalogCapabilityDescriptor, key: string, count = 3) => nounsOf(capability).say((noun) => {
      const text = sentences.find(([name]) => name === key)?.[1] ?? "";
      const values = Object.fromEntries([...text.matchAll(/\{\{(\w+)\}\}/g)].map(([, name]) => [name, "甲"]));
      return anyKey(`library:${key}`, { ...values, noun, count });
    }, count);
    assert.equal(said(CODEX_SKILLS, "chip.install", 1), "安装 Skill");
    assert.equal(said(CODEX_PLUGINS, "chip.install", 1), "安装插件");
    assert.equal(said(CODEX_PETS_V1, "removal.deleteLabel", 1), "删除本地宠物 v1");
    assert.equal(said(CODEX_PETS_V2, "outcome.needAttention"), "3 个宠物 v2 需要处理");
    assert.equal(said(CODEX_SKILLS, "kindCard.count", 9), "9 个 Skill");
    assert.equal(said(CODEX_SKILLS, "review.search"), "搜索 Skill");
    assert.equal(said(CODEX_SKILLS, "chip.keepCopy", 1), "更新 Skill · 保留本地副本");
    assert.equal(nounsOf(CODEX_SKILLS).lead((noun) => i18n.t("library:info.label", { noun, name: "release-notes" })), "release-notes 的 Skill 信息");
    assert.equal(nounsOf(CODEX_SKILLS).counted(9), "个 Skill");
    assert.equal(nounsOf(CODEX_PLUGINS).counted(2), "个插件");
    // Every sentence holding a noun reads right for every kind, Latin or Han.
    for (const capability of [CODEX_SKILLS, CODEX_PLUGINS, CODEX_PETS_V1, CODEX_PETS_V2]) {
      for (const [key, text] of sentences) {
        if (!text.includes("{{noun}}")) continue;
        const out = said(capability, key);
        assert.doesNotMatch(out, UNSPACED, `${capability.label} ${key}: ${out}`);
        assert.doesNotMatch(out, / {2}|^ | $|[\uE000\uE001]/, `${capability.label} ${key}: ${out}`);
      }
    }
  });
});

test("in Chinese a scope page speaks Chinese around the backend's labels and reasons", () => {
  const unavailable = inChinese(() => renderScopePage(unavailableScope));
  // The backend's reason and the row's raw label stay as they came.
  assert.match(unavailable, new RegExp(escapeRegExp(UNAVAILABLE_REASON)));
  assert.match(unavailable, /当前构建无法使用 codex\/agent_skill\/agentskills\.io\.skill@2 · conf2。/);
  assert.match(unavailable, /aria-label="此资源库行为何不可用"/);
  assert.equal(
    unavailable.match(new RegExp(`<button[^>]*class="sync-path-sync"[^>]*disabled=""[^>]*title="${escapeRegExp(UNAVAILABLE_REASON)}"`, "g"))?.length,
    2,
    "Pull and Push carry the backend's reason, in English",
  );
  const page = inChinese(() => renderScopePage());
  assert.match(page, /<span class="sync-path-sync-label">下载<\/span>/);
  assert.match(page, /<span class="sync-path-sync-label">上传<\/span>/);
  assert.match(page, /aria-label="刷新比较"/);
  assert.match(page, /aria-label="无存储。存储菜单"/);
  // The capability's label comes from Rust, keyed by its presentation hint:
  // Skill stays English and singular in Chinese; the profile's name stays.
  assert.match(page, /aria-label="Skill · conf2 路径"/);
  assert.match(page, /aria-label="Skill · conf2。资源库行菜单"/);
  assert.doesNotMatch(page, /技能|推送|拉取/);
  // English is back afterwards.
  assert.match(renderScopePage(), /<span class="sync-path-sync-label">Pull<\/span>/);

  const index = inChinese(() => renderToStaticMarkup(
    <CatalogIndexPage scopes={[]} capabilities={CAPABILITIES} busy={false} appActions={[]} onOpenScope={() => undefined} onAddScope={() => undefined} />,
  ));
  assert.match(index, /本机 0 个/);
  assert.match(index, /添加资源…/);
  assert.match(index, /通过你的存储共享/);
  assert.match(index, /aria-label="添加 Skill"/);
});

test("in Chinese a Pull review row's chooser keeps names and the backend's words", async () => {
  const { ResourceCard } = await import("../../src/components/storage-catalog/StorageCatalogPage");
  const pullAction = (kind: CatalogPullAction["kind"]["kind"], id: string): CatalogPullAction => ({
    action_id: id,
    resource_id: "release-notes",
    kind: kind === "preserve_as_copy_and_overwrite"
      ? { kind, copy: { copy_name: "release-notes-local", copy_resource_id: "r", copy_target_path: "/t", transformed_content_sha256: "e", original_content_sha256: "f", storage_content_sha256: "1", reusable_witness: false } }
      : { kind } as CatalogPullAction["kind"],
    requires_destructive_approval: kind === "overwrite_with_storage",
    expected_target_state: { state: "absent" },
    incoming_content_sha256: "c",
  });
  const actions = [pullAction("preserve_as_copy_and_overwrite", "keep"), pullAction("overwrite_with_storage", "over"), pullAction("install", "inst"), pullAction("update", "upd")];
  const card = (capability: CatalogCapabilityDescriptor, chosen: number) => {
    const options = actions.map((candidate) => ({ id: candidate.action_id, label: actionLabel(candidate), kind: candidate.kind.kind }));
    return renderToStaticMarkup(
      <ResourceCard
        row={itemRow({ state: "storage_changed", blocker: "Folder is a symlink", warnings: [{ code: "secret_shaped", digest: "d1", message: "contains credential-shaped content: scripts/run.sh" }] })}
        storageName="Vault"
        storageRevision={null}
        provider="codex"
        providerLabel="Codex"
        nouns={nounsOf(capability)}
        onOpenApp={() => undefined}
        onOpenTerminal={() => undefined}
        onReveal={() => undefined}
        removal={{ label: "删除本地 Skill", busy: false, removing: false, onRemove: () => undefined }}
        selectable
        selected
        action={{ chosen: options[chosen]!, options, expanded: true, destructive: chosen === 1, onToggle: () => undefined, onChoose: () => undefined }}
      />,
    );
  };
  const skill = inChinese(() => card(CODEX_SKILLS, 0));
  // The chip, its menu and every option, in Chinese; the row and the copy
  // keep the names they were given.
  assert.match(skill, /<strong title="release-notes">release-notes<\/strong>/);
  assert.match(skill, /title="将本地保留为 release-notes-local 并覆盖——选择其他本地操作">更新 Skill · 保留本地副本<svg/);
  assert.match(skill, /role="menu" aria-label="release-notes 的本地操作"/);
  for (const [verb, phrase] of [["保留副本", "将本地保留为 release-notes-local 并覆盖"], ["覆盖", "覆盖本地内容"], ["安装", "安装存储中的内容"], ["更新", "从存储更新"]]) {
    assert.match(skill, new RegExp(`<span>${verb}</span><small class="phrase">${phrase}</small>`));
  }
  assert.match(skill, /class="sync-path-menu-item danger"[^>]*><span class="sync-path-menu-copy"><span>覆盖<\/span>/);
  assert.match(skill, /aria-label="排除 release-notes"/);
  assert.match(skill, /data-tooltip="Vault 中较新。下载以更新本机。"/);
  // The backend's refusal and warning stay in its words.
  assert.match(skill, /title="Folder is a symlink"/);
  assert.match(skill, /data-tooltip="contains credential-shaped content: scripts\/run\.sh"/);
  assert.doesNotMatch(skill, /技能|推送|拉取/);
  // A Han noun sets against the Han beside it.
  assert.match(inChinese(() => card(CODEX_PLUGINS, 2)), />安装插件<svg/);
  assert.match(inChinese(() => card(CODEX_PETS_V1, 1)), />覆盖宠物 v1<svg/);
  // And English is what it was.
  assert.match(card(CODEX_SKILLS, 0), />Update skill · keeps local copy<svg/);
});

test("in Chinese the fork drawer keeps the name and the backend's refusal as given", async () => {
  const { ForkDrawer } = await import("../../src/components/storage-catalog/StorageCatalogPage");
  const drawer = (seed: string, error: string | null) => renderToStaticMarkup(
    <ForkDrawer displayName="大黄" seed={seed} maxLength={128} busy={false} error={error}
      onEdited={() => undefined} onSubmit={() => undefined} onClose={() => undefined} />,
  );
  const refusal = "invalid skill name: '潮汐笔记'";
  const refused = inChinese(() => drawer("big-yellow-copy", refusal));
  assert.match(refused, /<span class="catalog-fork-lead">将<strong>“大黄”<\/strong>复刻为<\/span>/);
  assert.match(refused, /aria-label="大黄复刻的名称"/);
  assert.match(refused, /<\/svg> 复刻<\/button>/);
  assert.match(refused, />取消<\/button>/);
  assert.match(refused, new RegExp(`role="alert"><svg[\\s\\S]*?</svg> ${escapeRegExp(refusal.replace(/'/g, "&#x27;"))}</span>`));
  const broken = inChinese(() => drawer("潮汐-copy", null));
  assert.match(broken, /以字母或数字开头，之后可用字母、数字、点、短横线和下划线——这会成为副本的文件夹名。/);
  assert.match(broken, /value="潮汐-copy"/);
  assert.match(inChinese(() => drawer("big-yellow-copy", null)), /在原件旁以此名称建立的独立副本——单独成行，上传之前只在本机。/);
  assert.match(drawer("big-yellow-copy", null), /<span class="catalog-fork-lead">Fork <strong>“大黄”<\/strong> as<\/span>/);
  // A name is text in either language, never markup the sentence's tags parse.
  const markup = "a<i>b</i> & <b>c</b> {{x}} <0>z</0>";
  const escaped = "a&lt;i&gt;b&lt;/i&gt; &amp; &lt;b&gt;c&lt;/b&gt; {{x}} &lt;0&gt;z&lt;/0&gt;";
  const named = (displayName: string) => renderToStaticMarkup(
    <ForkDrawer displayName={displayName} seed="x" maxLength={128} busy={false} error={null}
      onEdited={() => undefined} onSubmit={() => undefined} onClose={() => undefined} />,
  );
  assert.match(named(markup), new RegExp(`<span class="catalog-fork-lead">Fork <strong>“${escapeRegExp(escaped)}”</strong> as</span>`));
  assert.match(inChinese(() => named(markup)), new RegExp(`<span class="catalog-fork-lead">将<strong>“${escapeRegExp(escaped)}”</strong>复刻为</span>`));
  // A card's Fork names the kind in Chinese, the item as it is.
  assert.equal(
    inChinese(() => nounsOf(CODEX_SKILLS).say((noun) => i18n.t("library:fork.title", { name: "release-notes", noun }), 1)),
    "复刻 release-notes——以新名称复制此 Skill",
  );
});

test("in Chinese an outcome counts in 个 and keeps each row's message", async () => {
  const { cloudPullOutcome, pushOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const pulled = inChinese(() => cloudPullOutcome({ plan_id: "review", remote_accepted: true, results: [
    { resource_id: "pip", display_name: "Pip", outcome: "failed", message: "Provider apply stopped" },
    { resource_id: "duck", display_name: "Duck", outcome: "skipped", message: "Not on this computer" },
  ] }, { nouns: nounsOf(CODEX_SKILLS), addressOf: () => null, at: 1, keptAs: () => null, version: null }));
  assert.equal(pulled.title, "2 个 Skill 需要处理");
  assert.deepEqual(pulled.stats.map((stat) => stat.label), ["已还原", "失败", "已跳过"]);
  assert.deepEqual(pulled.rows.map((row) => [row.title, row.reason]), [["Pip", "Provider apply stopped"], ["Duck", "Not on this computer"]]);
  const pushed = inChinese(() => pushOutcome({ plan_id: "review", published: true, revision: 4, stopped: "Second item refused", message: "", results: [
    { resource_id: "a", display_name: "Duck", state: "published", message: null },
    { resource_id: "b", display_name: "Seal", state: "failed", message: "No access" },
  ] }));
  assert.equal(pushed.title, "已上传 1 / 2 项");
  assert.deepEqual(pushed.technical, [{ label: "上传已停止", value: "Second item refused" }]);
  assert.deepEqual(pushed.stats.map((stat) => stat.label), ["已完成", "Version"]);
});

test("in Chinese the backend's capability labels read by their presentation hint; Skill stays English and singular", () => {
  const withProfile = (capability: CatalogCapabilityDescriptor, name: string) => ({
    label: capability.label,
    capability,
    profile: { display_name: name },
  });
  assert.deepEqual(
    inChinese(() => [CODEX_SKILLS, CODEX_PLUGINS, CODEX_PETS_V1, CODEX_PETS_V2].map((capability) => catalogScopeLabel(withProfile(capability, "Codex home")))),
    ["Skill · Codex home", "插件 · Codex home", "宠物 v1 · Codex home", "宠物 v2 · Codex home"],
  );
  // English is the backend's label; a row with no capability keeps its raw label.
  assert.equal(catalogScopeLabel(withProfile(CODEX_PLUGINS, "Codex home")), "Plugins · Codex home");
  assert.equal(inChinese(() => catalogScopeLabel(unavailableScope)), "codex/agent_skill/agentskills.io.skill@2 · conf2");
  // The Add chooser and the index doors, one per shape.
  assert.deepEqual(inChinese(() => catalogAddChoices(CAPABILITIES).map((choice) => choice.label)), ["Skill", "插件", "宠物 v1", "宠物 v2"]);
  assert.deepEqual(catalogAddChoices(CAPABILITIES).map((choice) => choice.label), ["Skills", "Plugins", "Pets v1", "Pets v2"]);
  const index = inChinese(() => renderToStaticMarkup(
    <CatalogIndexPage scopes={[scopeView]} capabilities={CAPABILITIES} busy={false} appActions={[]} onOpenScope={() => undefined} onAddScope={() => undefined} />,
  ));
  assert.match(index, /aria-label="打开 Skill · conf2"/);
  assert.doesNotMatch(index, /Skills/);
  // A kind card whose count is not in yet shows its format, in Chinese.
  const card = catalogKindCards(CAPABILITIES, { profile_id: "catprofile-1", provider: "codex" }, [], null)
    .find((candidate) => candidate.capability === CODEX_PLUGINS)!;
  assert.equal(inChinese(() => catalogKindCardFoot(card)), "插件安装意向");
  assert.equal(catalogKindCardFoot(card), "Plugin install intent");
  // The scope page's crumb, and nowhere the backend's English plural.
  const page = inChinese(() => renderScopePage());
  assert.match(page, /aria-label="Skill · conf2 路径"/);
  assert.doesNotMatch(page, /Skills/);
});

/* D7: the Library's progress lines and fixed result messages are the
   backend's English with a stage or message code and the item's name. */
test("Library progress and fixed results keep their English and are worded by code in Chinese", async () => {
  const { catalogActionMessage, catalogProgressText, catalogRemovalMessage, pullOutcome } = await import("../../src/components/storage-catalog/outcomes");
  const pushing = { plan_id: "catplan-1", completed: 1, total: 3, message: "Pushing tidal-notes…", stage: "pushing" as const, name: "tidal-notes" };
  const finished = { plan_id: "catplan-1", completed: 3, total: 3, message: "Push finished", stage: "push_finished" as const };
  const skipped = {
    resource_id: "skill:a", display_name: "tidal-notes", outcome: "skipped" as const,
    message: "Skipped; local content was left unchanged", message_code: "skipped" as const,
  };
  const kept = {
    resource_id: "skill:b", display_name: "潮汐笔记", outcome: "preserved_and_updated" as const,
    message: "Kept your version as 'tidal-notes-local'", message_code: "kept_as" as const, message_name: "tidal-notes-local",
  };
  const failedRow = { resource_id: "skill:c", display_name: "harbor", outcome: "failed" as const, message: "payload digest mismatch" };
  const forked = {
    resource_id: "skill:d", display_name: "tidal-notes-2", outcome: "forked" as const,
    message: "Forked from 'tidal-notes'; local-only until you push it", message_code: "forked" as const, message_name: "tidal-notes",
  };
  const removed = { removed: true, message: "tidal-notes removed from this provider home. Copies in storage are kept.", display_name: "tidal-notes" };
  const notRemoved = { removed: false, message: "Removal did not complete. Some local changes may have been made. busy", display_name: "tidal-notes" };
  const reasons = () => pullOutcome({ plan_id: "catplan-2", remote_accepted: false, results: [skipped, kept, failedRow] }).rows.map((row) => row.reason);

  // English is the backend's own text.
  assert.equal(catalogProgressText(pushing), "Pushing tidal-notes… (1/3)");
  assert.equal(catalogProgressText(finished), "Push finished (3/3)");
  assert.deepEqual(reasons(), [skipped.message, kept.message, failedRow.message]);
  assert.equal(catalogActionMessage(forked), forked.message);
  assert.equal(catalogRemovalMessage(removed), removed.message);

  inChinese(() => {
    assert.equal(catalogProgressText(pushing), "正在上传 tidal-notes… (1/3)");
    assert.equal(catalogProgressText(finished), "上传完成 (3/3)");
    // An error stays as the backend wrote it; names stay as given.
    assert.deepEqual(reasons(), ["已跳过，本地内容保持不变", "你本地的内容已另存为“tidal-notes-local”", "payload digest mismatch"]);
    assert.equal(catalogActionMessage(forked), "已从“tidal-notes”复刻；上传之前仅保存在本地");
    assert.equal(catalogRemovalMessage(removed), "已从此代理主目录移除 tidal-notes。存储中的副本会保留。");
    assert.equal(catalogRemovalMessage(notRemoved), notRemoved.message);
  });
});
