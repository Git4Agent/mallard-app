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
  "get_catalog_resource",
  "get_catalog_resource_status",
  "inspect_catalog_resource_links",
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
    "listCapabilities", "listProfiles", "saveProfile",
    "createScope", "listScopeResources", "linkScopeResource", "getResource",
    "unlink", "remove", "inspectLinks", "preparePush",
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

const scopeView: CatalogScopeView = {
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

function renderScopePage() {
  return renderToStaticMarkup(
    <StorageCatalogPage
      scope={scopeView}
      scopes={[scopeView]}
      storages={[{ id: "storage-a", name: "Vault", kind: "local", detail: "~/Vault", detailIsPath: true }]}
      appActions={[]}
      onOpenScope={() => undefined}
      onOpenStorageSettings={() => undefined}
      onAddStorage={() => undefined}
      onAddScope={() => undefined}
      onRemoveScope={() => undefined}
      notify={() => undefined}
    />,
  );
}

test("the scope page wears the project workspace's trail and list", () => {
  const html = renderScopePage();
  // The project workspace's own two crumbs — the storage it syncs with, then
  // the thing being synced. No Storages crumb: that roots the storage
  // screens, not a workspace. The page never names itself twice, so there is
  // no <h1> over the list.
  assert.doesNotMatch(html, /Storages/);
  assert.match(html, /Skills · conf2/);
  assert.doesNotMatch(html, /<h1>Skills · conf2<\/h1>/);
  assert.match(html, /Loading Skills · conf2…/);
  assert.doesNotMatch(html, /(?:Library|Catalog) storage:|Choose another storage|Pair a .* profile/);
  assert.doesNotMatch(html, /Add Skill|Add Plugin|Add Pet\b/);
  // Browsing a storage from a select is gone: the storage is the page's outer
  // scope, chosen in the bar.
  assert.doesNotMatch(html, /Browse stored resources|<select/);
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /api\.listScopeResources\(scope\.scope\.scope_id, chosenStorageId\)/);
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
  // The card is the activity card, and the name is its one door — the chevron
  // column went with the row grammar that carried it.
  assert.match(page, /sync-history-thread-card catalog-card/);
  assert.match(page, /className="catalog-card-name"/);
  assert.doesNotMatch(
    page,
    /chevron-right|catalog-row-name|catalog-row-place|catalog-row-stamp|catalog-row-open|catalog-review-row|catalog-row-list/,
  );
  // Line two states what the scan measured — files, bytes on disk — never a
  // recount of its own.
  assert.match(page, /row\.local_files/);
  assert.match(page, /row\.local_bytes/);
  // The ⓘ carries the row's paths and digests in the session card's own
  // popover, each a click to copy.
  assert.match(page, /sync-history-debug-popover/);
  assert.match(page, /sha256:\$\{row\.content_sha256\}/);
  // Reveal in Finder is the card's one launch verb, off when nothing is
  // local; the provider mark beside it is inert.
  assert.match(page, /revealItemInDir/);
  assert.match(page, /disabled=\{!row\.local_path\}/);
  assert.match(page, /sync-history-provider-inert/);
  // Acknowledgements are row-sized callouts inside the card that raised them:
  // the warning tint for a scan finding, the danger tint for an overwrite,
  // and the app's own checkbox rather than the platform's.
  assert.match(page, /className="catalog-card-ack"/);
  assert.match(page, /className="catalog-card-ack danger"/);
  assert.match(page, /Reviewed this warning — /);
});

test("Push and Pull are page verbs with one review each, and link on the way", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // The bar owns both verbs, with the count the list itself produced.
  assert.match(page, /sync-path-sync[\s\S]*Pull<\/span>[\s\S]*counts\.pull/);
  assert.match(page, /sync-path-sync[\s\S]*Push<\/span>[\s\S]*counts\.push/);
  assert.match(page, /counts = useMemo\(\(\) => rows\.reduce/);
  // Push and Pull create the pairing they need; the user never links by hand.
  assert.match(page, /ensureLink[\s\S]*api\.linkScopeResource/);
  assert.match(page, /const linkId = await ensureLink\(row\);[\s\S]*api\.preparePush\(linkId\)/);
  assert.match(page, /api\.publish\(linkId, plan\.plan_id, true/);
  assert.match(page, /const linkId = await ensureLink\(row\);[\s\S]*api\.preparePull\(linkId, null\)/);
  assert.match(page, /api\.applyPull\(\s*entry\.linkId,\s*entry\.plan\.plan_id/);
  // The reviews are the project's own review chrome.
  assert.match(page, /SyncReviewSelectionBar/);
  assert.match(page, /sync-inline-push-review sync-sync-review-workspace/);
  assert.match(page, /sync-inline-pull-review sync-sync-review-workspace/);
  // A local action is a readout plus the app's own popover, never a select.
  assert.match(page, /sync-path-menu end" role="menu"/);
  assert.doesNotMatch(page, /<select/);
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
  // Profile selection uses the same row as project setup: saved homes and a
  // Custom path… choice in one select, an inline path, and Browse.
  assert.match(add, /sync-agent-profile-row/);
  assert.match(add, /<option value="__pending">Custom path…<\/option>/);
  assert.match(add, /<Icon name="folder" size=\{13\} \/> Browse/);
  // Choosing a profile is an independent setup step. It remains usable before
  // Kind is chosen; picking an incompatible Kind later clears the provider.
  assert.match(add, /kind === null \|\| capability\.kinds\.includes\(kind\)/);
  assert.match(add, /aria-label="Provider"[\s\S]*?disabled=\{working\}/);
  assert.doesNotMatch(add, /disabled=\{working \|\| !kind\}/);
  assert.doesNotMatch(add, /Add profile|className="catalog-inline-form"/);
  assert.doesNotMatch(add, /Manage saved homes|removeProfile|api\.deleteProfile/);
  assert.doesNotMatch(add, /probe_catalog_resource_profile|probeProviderProfile/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-setup-(grid|card|step)/);
});

test("linking is a side effect of the verb, and never syncs content", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Link changes configuration only; the comment on the one place that calls
  // it says so, because nothing in the UI asks the user to do it any more.
  assert.match(page, /configuration only, no provider content changed/);
  // A local-only row links by resource; a stored-only row adopts the
  // repository. Both decided from the row, not from a picker.
  assert.match(page, /local_resource_id: row\.repository_id \? null : row\.resource_id/);
  assert.match(page, /repository_id: row\.repository_id \?\? null/);
  // A row that is already paired never links again.
  assert.match(page, /if \(row\.link_id\) return row\.link_id;/);
});

test("Refresh, Unlink and Fork stay scoped to one resource behind the row", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /api\.inspectLinks\(registrationId\)/);
  assert.match(page, /inspectionEpochRef/);
  assert.doesNotMatch(page, /forceRevalidate|force_revalidate/);
  assert.match(page, /api\.prepareFork\(resource\.registration\.registration_id/);
  assert.match(page, /api\.unlink\(resource\.registration\.registration_id, linkId, revision\)/);
  // Unlinking the whole pairing is the storage crumb's verb, worded the way
  // the project's is, and it names the count before it runs.
  assert.match(page, /label: `Unlink from \$\{label\}…`/);
  assert.match(page, /Push and Pull stop for all \$\{linked\.length\} paired/);
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
