import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StorageCatalogPage from "../../src/components/storage-catalog/StorageCatalogPage";
import {
  actionLabel,
  catalogScopeLabel,
  linkStateLabel,
} from "../../src/components/storage-catalog/model";
import type {
  CatalogPullAction,
  CatalogScope,
  CatalogScopeView,
} from "../../src/components/storage-catalog/types";

const commandNames = [
  "apply_catalog_resource_fork",
  "apply_catalog_resource_pull",
  "create_catalog_scope",
  "delete_catalog_resource_profile",
  "delete_catalog_scope",
  "get_catalog_resource_status",
  "inspect_catalog_resource_link",
  "link_catalog_scope_resource",
  "list_catalog_capabilities",
  "list_catalog_resource_profiles",
  "list_catalog_scope_resources",
  "list_catalog_scopes",
  "prepare_catalog_resource_fork",
  "prepare_catalog_resource_pull",
  "prepare_catalog_resource_push",
  "publish_catalog_resource",
  "remove_catalog_resource",
  "save_catalog_resource_profile",
  "unlink_catalog_resource",
].sort();

test("the Catalog API exposes only the scope-rowed per-resource command surface", () => {
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
    "listCapabilities", "listProfiles", "saveProfile", "deleteProfile",
    "createScope", "listScopeResources", "linkScopeResource",
    "unlink", "remove", "inspectLink", "preparePush",
    "publish", "preparePull", "applyPull", "prepareFork", "applyFork", "getStatus",
  ]) {
    assert.match(callers, new RegExp(`api\\.${method}\\(`), `${method} has a production caller`);
  }
  // Listing and removing rows belong to the shell, because the sidebar owns
  // the menu.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  for (const method of ["listScopes", "deleteScope"]) {
    assert.match(shell, new RegExp(`storageCatalogApi\\.${method}\\(`), `${method} has a production caller`);
  }
});

test("a row label derives from the kind and the profile, covering every menu shape", () => {
  const scope = (resource_kind: CatalogScope["resource_kind"], provider: CatalogScope["provider"]): CatalogScope => ({
    scope_id: "catscope-1",
    provider,
    profile_id: "catprofile-1",
    resource_kind,
    revision: 1,
    created_at: 1,
    updated_at: 1,
  });
  const profile = (display_name: string) => ({ display_name });
  // The six rows the menu was designed around, plus the v2 sibling.
  assert.equal(catalogScopeLabel(scope("agent_skill", "codex"), profile("Codex home")), "Skills · Codex home");
  assert.equal(catalogScopeLabel(scope("agent_skill", "codex"), profile("conf2")), "Skills · conf2");
  assert.equal(catalogScopeLabel(scope("agent_skill", "claude"), profile("Claude home")), "Skills · Claude home");
  assert.equal(catalogScopeLabel(scope("codex_pet_v1", "codex"), profile("Codex home")), "Pets v1 · Codex home");
  assert.equal(catalogScopeLabel(scope("codex_pet_v1", "codex"), profile("conf2")), "Pets v1 · conf2");
  assert.equal(catalogScopeLabel(scope("plugin_install_intent", "codex"), profile("Codex home")), "Plugins · Codex home");
  assert.equal(catalogScopeLabel(scope("codex_pet_v2", "codex"), profile("Codex home")), "Pets v2 · Codex home");
  // A vanished profile degrades to the kind alone rather than crashing.
  assert.equal(catalogScopeLabel(scope("agent_skill", "codex"), null), "Skills");
});

test("the scope page opens on its derived label and loads that row only", () => {
  const view: CatalogScopeView = {
    scope: {
      scope_id: "catscope-1",
      provider: "codex",
      profile_id: "catprofile-1",
      resource_kind: "agent_skill",
      revision: 1,
      created_at: 1,
      updated_at: 1,
    },
    profile: {
      profile_id: "catprofile-1",
      provider_key: "codex",
      display_name: "conf2",
      configured_home: "/Users/user/codex-conf2",
      revision: 1,
      created_at: 1,
      updated_at: 1,
    },
  };
  const html = renderToStaticMarkup(
    <StorageCatalogPage
      scope={view}
      storages={[{ id: "storage-a", name: "Vault", kind: "local" }]}
      onScopeGone={() => undefined}
    />,
  );
  assert.match(html, /<h1>Skills · conf2<\/h1>/);
  assert.match(html, /Loading Skills · conf2/);
  assert.doesNotMatch(html, /(?:Library|Catalog) storage:|Choose another storage|Pair a .* profile/);
  // The permanent category surface is gone with the family model.
  assert.doesNotMatch(html, /Add Skill|Add Plugin|Add Pet\b/);
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /api\.listScopeResources\(scope\.scope\.scope_id/);
  assert.match(page, /missing locally/);
  assert.doesNotMatch(page, /CatalogCategory|CATALOG_CATEGORIES|family/);
});

test("Add is Kind then agent profile, and never picks a storage or resource", () => {
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  const order = ["Kind", "Agent profile"]
    .map((label) => add.indexOf(`<strong>${label}</strong>`));
  assert.ok(order.every((index) => index >= 0), "both Add bands render");
  assert.deepEqual(order, [...order].sort((left, right) => left - right));
  // Kind is a visible four-way choice, not a hidden select.
  assert.match(add, /role="radiogroup"[\s\S]*?CATALOG_KINDS\.map/);
  assert.match(add, /Codex only/);
  assert.match(add, /api\.createScope\(provider, profileId, kind\)/);
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
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-setup-(grid|card|step)/);
});

test("linking happens on the row's page and never syncs content", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Link changes configuration only; the copy on the page says so.
  assert.match(page, /Link changes configuration only/);
  assert.match(page, /configuration only, no provider content changed/);
  // A discovered local resource links with a storage chosen inline; a stored
  // resource links for review and routes into Pull.
  assert.match(page, /local_resource_id: "localResourceId" in selection/);
  assert.match(page, /repository_id: "repositoryId" in selection/);
  assert.match(page, /review_pull[\s\S]*openPull\(result\.link_id\)/);
});

test("Push Pull Refresh and Unlink are scoped to one resource link; Fork is resource-scoped", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /api\.inspectLink\(linkId, force\)/);
  assert.match(page, /api\.preparePush\(linkId\)/);
  assert.match(page, /api\.publish\(linkId, plan\.plan_id/);
  assert.match(page, /api\.preparePull\(linkId/);
  assert.match(page, /api\.applyPull\([\s\S]*linkId,[\s\S]*plan\.plan_id/);
  assert.match(page, /api\.prepareFork\(resource\.registration\.registration_id/);
  assert.match(page, /api\.unlink\(resource\.registration\.registration_id, linkId, revision\)/);
  assert.match(page, /SyncReviewSelectionBar/);
  assert.doesNotMatch(page, /forkCodexThread|fork_codex_thread|storage-wide|bulk sync/i);
});

test("provider and pet identities stay explicit in the frontend contract", () => {
  const types = readFileSync("src/components/storage-catalog/types.ts", "utf8");
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  const model = readFileSync("src/components/storage-catalog/model.ts", "utf8");
  assert.match(types, /CatalogProviderKey = "codex" \| "claude"/);
  assert.match(types, /"codex_pet_v1"[\s\S]*"codex_pet_v2"/);
  assert.match(page, /Codex pet v1/);
  assert.match(page, /Codex pet v2/);
  // Pet v1 and v2 are two separate addable rows, never collapsed to "pets".
  assert.match(model, /kind: "codex_pet_v1", label: "Pet v1"/);
  assert.match(model, /kind: "codex_pet_v2", label: "Pet v2"/);
  // Capabilities are kind-typed; there is no plural-string matching left.
  assert.match(types, /kinds: CatalogResourceKind\[\]/);
  assert.doesNotMatch(types, /resources: string\[\]|CatalogResourceFamily/);
});

test("Pull labels preserve copies for skills and pets while plugins only overwrite", () => {
  const action = (kind: CatalogPullAction["kind"]): CatalogPullAction => ({
    action_id: `action-${"a".repeat(24)}`,
    resource_id: `resource-${"b".repeat(64)}`,
    kind,
    requires_destructive_approval: false,
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
    actionLabel(action({ kind: "preserve_skill_as_copy_and_overwrite", copy })),
    "Keep local as review-local-copy and overwrite",
  );
  assert.equal(
    actionLabel(action({ kind: "preserve_pet_as_copy_and_overwrite", copy })),
    "Keep local as review-local-copy and overwrite",
  );
  assert.equal(
    actionLabel(action({ kind: "overwrite_plugin_with_storage", plan: {} })),
    "Overwrite local content",
  );
  assert.equal(linkStateLabel("diverged"), "Diverged");

  const types = readFileSync("src/components/storage-catalog/types.ts", "utf8");
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(types, /conflicting_claims: CatalogClaimPin\[\]/);
  assert.match(page, /claimed by[\s\S]*Preserve a copy, acknowledge overwrite, or Skip/);
  assert.match(page, /Pull did not advance the comparison base/);
});

test("the Catalog remains independent and removed draft-era UI stays deleted", () => {
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
  for (const removed of [
    "CatalogSetupWorkspace.tsx",
    "StorageCatalogPullReview.tsx",
    "StorageCatalogPushReview.tsx",
    "StorageLibraryOverview.tsx",
    "StoragePetList.tsx",
    "StoragePluginList.tsx",
    "StorageSkillList.tsx",
  ]) {
    assert.equal(existsSync(`src/components/storage-catalog/${removed}`), false, `${removed} stays deleted`);
  }
  const projectTypes = readFileSync("src/types.ts", "utf8");
  const projectApi = readFileSync("src/components/project-sync/api.ts", "utf8");
  assert.doesNotMatch(projectTypes, /plugin_install_intent|codex_pet_v1|catalog_resource/i);
  assert.doesNotMatch(projectApi, /catalog/i);
});

test("only global provider skill roots are discoverable", () => {
  const profiles = readFileSync("src-tauri/src/storage_catalog/profiles.rs", "utf8");
  assert.match(profiles, /canonical_home\.join\("skills"\)/);
  assert.match(profiles, /os_user_home\.join\("\.agents"\)\.join\("skills"\)/);
  assert.doesNotMatch(profiles, /project_root|checkout_root|working_directory|\.skill\/|\.agent\/ssd/);
});

test("the shared skill root belongs to the default Codex home only", () => {
  const profiles = readFileSync("src-tauri/src/storage_catalog/profiles.rs", "utf8");
  // The shared-root push is gated on the default `~/.codex` home, and the
  // directory name comes from the provider descriptor rather than a string.
  // Tests may spell out the path; production code must not.
  const production = profiles.split("#[cfg(test)]")[0];
  assert.match(production, /is_default_codex_home\(&profile\.canonical_home, os_user_home\)/);
  assert.match(production, /default_profile_directory/);
  assert.doesNotMatch(production, /"\.codex"/);
});
