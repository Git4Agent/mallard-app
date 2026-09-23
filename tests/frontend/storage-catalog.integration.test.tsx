import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StorageCatalogPage from "../../src/components/storage-catalog/StorageCatalogPage";
import CatalogIndexPage from "../../src/components/storage-catalog/CatalogIndexPage";
import { storageCatalogApi } from "../../src/components/storage-catalog/api";
import {
  actionLabel,
  catalogAddChoices,
  catalogDoors,
  catalogNoun,
  catalogPresentation,
  catalogProviderList,
  catalogScopeLabel,
  catalogScopeViews,
  catalogStorageRepositoryViews,
  filterCatalogPullReviewItems,
  rowStatus,
  rowStatusLabel,
} from "../../src/components/storage-catalog/model";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogPullAction,
  CatalogScopeDto,
  CatalogScopePullItem,
  CatalogScopeRow,
  CatalogScopeView,
} from "../../src/components/storage-catalog/types";

const commandNames = [
  "apply_catalog_resource_fork",
  "apply_catalog_scope_pull",
  "apply_catalog_scope_push",
  "create_catalog_scope",
  "delete_catalog_scope",
  "finish_catalog_unfinished_push",
  "get_catalog_resource_preview",
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
    "listCapabilities", "listProfiles", "saveProfile", "renameProfile",
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
      ["plugin", { icon: "puzzle", artwork: false }],
      // The two pet formats are two rows, so they wear two marks; both hints
      // open the artwork column.
      ["pet", { icon: "paw", artwork: true }],
      ["pet-v2", { icon: "cat", artwork: true }],
    ],
  );
  assert.equal(catalogPresentation(CODEX_PETS_V1).icon, "paw");
  assert.equal(catalogPresentation(CODEX_PETS_V2).icon, "cat");
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
  assert.match(page, /catalog-pet-preview[\s\S]*?<Icon name="paw" size=\{16\} \/>/);
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
  const rows: CatalogScopeRow[] = cases.map((candidate) => ({
    row_id: `resource:${candidate.id}`,
    display_name: candidate.id,
    resource_id: candidate.id,
    state: candidate.state,
    local_present: candidate.localPresent,
    remote_present: true,
    warnings: [],
  }));
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
    filterCatalogPullReviewItems(items, rows).map((item) => item.item.resource_id),
    cases.filter((candidate) => candidate.visible).map((candidate) => candidate.id),
  );
});

const STORAGES = [
  { id: "storage-a", name: "Vault", provider_label: "Local folder", icon: "drive", detail: "~/Vault", detailIsPath: true },
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
  assert.match(empty, /aria-label="Add Plugins"[\s\S]*?data-icon="puzzle"/);
  assert.match(empty, /aria-label="Add Pets v1"[\s\S]*?data-icon="paw"/);
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
  assert.match(page, /const syncDisabled = busy \|\| !storageId \|\| loading \|\| unavailable;/);
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
    [["Skills", "lightbulb"], ["Plugins", "puzzle"], ["Pets v1", "paw"], ["Pets v2", "cat"]],
  );

  // The workspace draws exactly these choices, and Add records the one
  // descriptor the two choices name.
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  assert.match(add, /const choices = useMemo\(\(\) => catalogDoors\(capabilities\), \[capabilities\]\);/);
  assert.match(add, /role="radiogroup"[\s\S]*?choices\.map\(\(option\) => \{/);
  assert.match(add, /<Icon name=\{option\.icon\} size=\{24\} \/>/);
  assert.match(add, /<strong>\{option\.label\}<\/strong>/);
  assert.match(add, /catalogProviderList\(option\.providers\.map\(\(capability\) => capability\.provider_label\)\)/);
  assert.match(add, /const descriptor = choice\?\.providers\.find\(\(capability\) => capability\.provider === provider\) \?\? null;/);
  assert.match(add, /storageCatalogApi\.createScope\(descriptor, chosenProfileId\)/);

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
  // Acknowledgements are row-sized callouts inside the card that raised them:
  // the warning tint for a scan finding, the danger tint for an overwrite,
  // and the app's own checkbox rather than the platform's. The chip spans the
  // card's full row — auto-placement dropped a second chip into the grid's
  // auto-sized actions track and inflated it to the chip's whole sentence
  // (the sheared-chips screenshot); the pet template starts the span after
  // its preview column instead.
  assert.match(page, /catalog-card-ack\$\{danger \? " danger" : ""\}/);
  assert.match(
    readFileSync("src/App.css", "utf8"),
    /\.catalog-card-ack \{[^}]*grid-column: 1 \/ -1;/,
  );
  assert.match(
    readFileSync("src/App.css", "utf8"),
    /\.catalog-card-pet > \.catalog-card-ack \{[^}]*grid-column: 2 \/ -1;/,
  );
  // Payload first: the bold lead derives from warning.code with the raw
  // backend sentence as the fallback for unknown codes, and the ack verb is
  // the chip's trailing word — never the old constant preamble, whose text
  // the accessible name still carries in full.
  assert.match(page, /WARNING_LEADS/);
  assert.match(page, /warningParts\(warning\)/);
  assert.match(page, /aria-label=\{`Reviewed: \$\{warning\.message\}`\}|ariaLabel=\{`Reviewed: \$\{warning\.message\}`\}/);
  assert.match(page, /verbs=\{\["Mark reviewed", "Reviewed"\]\}/);
  assert.match(page, /verbs=\{\["Approve", "Approved"\]\}/);
  assert.doesNotMatch(page, /Reviewed this warning — |I understand this replaces/);
  // The count rides the verb at the point of commitment.
  assert.match(page, /Push \{pushSelected\.size\}/);
  assert.match(page, /Pull \{pullReviewSelectedCount\}/);
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
  assert.match(page, /counts = useMemo\(\(\) => rows\.reduce/);
  // Push and Pull create the one scope-storage pairing they need; the user
  // never links by hand, and each review consumes exactly one plan.
  assert.match(page, /ensureLink[\s\S]*storageCatalogApi\.linkScopeStorage/);
  assert.match(page, /await ensureLink\(\);[\s\S]*storageCatalogApi\.preparePush\(scope\.scope\.scope_id, storageId\)/);
  assert.match(page, /storageCatalogApi\.applyPush\(pushPlan\.plan_id, selected, \[\.\.\.pushAcks\]\)/);
  assert.match(page, /await ensureLink\(\);[\s\S]*storageCatalogApi\.preparePull\(scope\.scope\.scope_id, storageId, null\)/);
  assert.match(page, /storageCatalogApi\.applyPull\(pullPlan\.plan_id, choices, acknowledgements\)/);
  assert.match(page, /const reviewItems = filterCatalogPullReviewItems\(pullPlan\.items, rows\);[\s\S]*for \(const item of reviewItems\)/);
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

  assert.match(page, /const selectableRows = pushPlan\.rows\.filter\(\(row\) => row\.row\.selectable\);/);
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

test("Add is Kind then agent profile, and never picks a storage or resource", () => {
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  const order = ["Kind", "Agent profile"]
    .map((label) => add.indexOf(`<strong>${label}</strong>`));
  assert.ok(order.every((index) => index >= 0), "both Add bands render");
  assert.deepEqual(order, [...order].sort((left, right) => left - right));
  // Kind is a visible choice over the backend's capabilities, not a hidden
  // select and not a table of the frontend's own.
  assert.match(add, /role="radiogroup"[\s\S]*?choices\.map/);
  assert.doesNotMatch(add, /CATALOG_KINDS|Codex only|Codex and Claude/);
  assert.match(add, /storageCatalogApi\.createScope\(descriptor, chosenProfileId\)/);
  // No storage, no repository, no local resource: those belong to the row's
  // own page, chosen per resource link.
  assert.doesNotMatch(
    add,
    /storage_id|storageId|onAddStorage|repository_id|local_resource_id|listScopeResources|linkScopeResource/,
  );
  // It wears the project-setup language.
  assert.match(add, /sync-inline-new-project sync-setup-workspace/);
  assert.match(add, /sync-setup-section/);
  assert.match(add, /sync-setup-footer/);
  // Profile selection uses the same row as project setup: saved homes and a
  // Custom path… choice in one select, an inline path, and Browse.
  assert.match(add, /sync-agent-profile-row/);
  assert.match(add, /<option value="__pending">Custom path…<\/option>/);
  assert.match(add, /<Icon name="folder" size=\{13\} \/> Browse/);
  // Choosing a profile is an independent setup step. It remains usable before
  // Kind is chosen — every registered provider is offered — and picking a
  // Kind the provider is not registered for later clears the provider.
  assert.match(add, /choice \? choice\.providers : capabilities/);
  assert.match(add, /const stillSupported = next\.providers\.some\(\(capability\) => capability\.provider === provider\);/);
  assert.match(add, /aria-label="Provider"[\s\S]*?disabled=\{working\}/);
  assert.doesNotMatch(add, /disabled=\{working \|\| !kind\}/);
  assert.doesNotMatch(add, /Add profile|className="catalog-inline-form"/);
  assert.doesNotMatch(add, /Manage saved homes|removeProfile|storageCatalogApi\.deleteProfile/);
  assert.doesNotMatch(add, /probe_catalog_resource_profile|probeProviderProfile/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-setup-(grid|card|step)/);
});

test("the profile is named by its folder, and renamed from the row's page", () => {
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  // Setup never asks for a name: no name input, no separate commit step. A
  // pending path is saved as part of Add itself, then the scope lands on it.
  assert.doesNotMatch(add, /Profile name|Save home|catalog-profile-save-row/);
  // A display name reaches this page only as the label a provider mark draws
  // itself with — never as a field, a piece of state, or a value Add sends.
  assert.doesNotMatch(
    add.replace(/<ProviderMark[\s\S]*?\/>/g, ""),
    /displayName/,
    "displayName is only the provider mark's prop",
  );
  assert.match(add, /await storageCatalogApi\.saveProfile\(descriptor\.provider, expandedProfilePath\)/);
  assert.match(add, /profileId !== null \|\| customPathReady/);
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
  assert.match(types, /storage_identity_sha256: string;\s*head: LibraryHeadCoordinate;/);
  assert.match(types, /source_coordinate: CatalogCoordinate;/);
  assert.doesNotMatch(types, /resources: string\[\]|CatalogResourceFamily|kinds: /);
  // No closed union and no identity branch survives anywhere in the Library:
  // the backend's labels, hints, and protocols are the only vocabulary.
  for (const file of [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/storage-catalog/CatalogIndexPage.tsx",
    "src/components/storage-catalog/AddCatalogScopeWorkspace.tsx",
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
  assert.match(page, /Pull did not advance the comparison base/);
});

test("the Catalog remains independent from project sync", () => {
  const liveFiles = [
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/storage-catalog/AddCatalogScopeWorkspace.tsx",
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
