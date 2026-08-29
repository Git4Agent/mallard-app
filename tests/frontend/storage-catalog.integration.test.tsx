import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StorageCatalogPage from "../../src/components/storage-catalog/StorageCatalogPage";
import {
  CATALOG_KINDS,
  actionLabel,
  catalogScopeLabel,
} from "../../src/components/storage-catalog/model";
import type {
  CatalogPullAction,
  CatalogScope,
  CatalogScopeView,
} from "../../src/components/storage-catalog/types";

const commandNames = [
  "apply_catalog_resource_fork",
  "apply_catalog_scope_pull",
  "apply_catalog_scope_push",
  "create_catalog_scope",
  "delete_catalog_scope",
  "get_catalog_pet_preview",
  "link_catalog_scope_storage",
  "list_catalog_capabilities",
  "list_catalog_resource_profiles",
  "list_catalog_scope_resources",
  "list_catalog_scopes",
  "list_catalog_storage_repositories",
  "prepare_catalog_resource_fork",
  "prepare_catalog_scope_pull",
  "prepare_catalog_scope_push",
  "rename_catalog_resource_profile",
  "save_catalog_resource_profile",
  "unlink_catalog_scope_storage",
].sort();

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
    "createScope", "listScopeResources", "getPetPreview", "linkScopeStorage",
    "unlinkScopeStorage", "preparePush", "applyPush", "preparePull", "applyPull",
    "prepareFork", "applyFork",
  ]) {
    assert.match(callers, new RegExp(`storageCatalogApi\\.${method}\\(`), `${method} has a production caller`);
  }
  // Listing and removing rows belong to the shell, because the sidebar owns
  // the menu.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  for (const method of ["listScopes", "deleteScope"]) {
    assert.match(shell, new RegExp(`storageCatalogApi\\.${method}\\(`), `${method} has a production caller`);
  }
  // The storage page's repository listing is the one storage-level read: its
  // caller is the storage page, not a Catalog screen.
  const storagePage = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(storagePage, /storageCatalogApi\.listStorageRepositories\(/);
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

test("every kind wears its own mark rather than borrowing one", () => {
  // All three used to borrow, and all three borrowed badly: `file` said where a
  // skill was stored rather than what it was, `tool` is the cog a tool call
  // already spends, and `openai` names the *provider* — which ProviderMark
  // paints on the same row, so as a kind glyph it said nothing at all.
  assert.deepEqual(
    CATALOG_KINDS.map((kind) => [kind.kind, kind.icon]),
    [
      ["agent_skill", "lightbulb"],
      ["plugin_install_intent", "puzzle"],
      // One glyph for both pet formats: the rows differ by label, not by kind.
      ["codex_pet_v1", "paw"],
      ["codex_pet_v2", "paw"],
    ],
  );
  for (const borrowed of ["file", "tool", "openai"]) {
    assert.ok(
      !CATALOG_KINDS.some((kind) => kind.icon === borrowed),
      `no kind may fall back to \`${borrowed}\``,
    );
  }

  // The pet card's tile is the kind glyph when a row has no sprite yet, so the
  // column never collapses and never repeats the provider mark beside it.
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  assert.match(page, /catalog-pet-preview[\s\S]*?<Icon name="paw" size=\{16\} \/>/);
  assert.doesNotMatch(page, /<Icon name="openai"/);
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
      onScopeGone={() => undefined}
      onProfileRenamed={() => undefined}
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
  // Reveal in Finder is the card's one launch verb, off when nothing is
  // local; the provider mark beside it is inert.
  assert.match(page, /revealItemInDir/);
  assert.match(page, /disabled=\{!row\.local_path\}/);
  assert.match(page, /sync-history-provider-inert/);
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
    readFileSync("src/RetroTheme.css", "utf8"),
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
  assert.match(page, /Pull \{pullSelected\.size\}/);
});

test("pet rows wear the preview tile, and stored-only previews arrive lazily", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  // Pets are the one kind with artwork: their cards open a preview column —
  // image when a preview exists, the kind glyph when nothing can be drawn —
  // and non-pet scopes never grow the column at all.
  assert.match(page, /catalog-card-pet/);
  assert.match(page, /catalog-pet-preview/);
  assert.match(page, /petKind \? row\.preview_png \?\? remotePreviews\[row\.row_id\] \?\? null : undefined/);
  // Stored-only rows ask after render, once per page, and a failure keeps
  // the glyph — the list call itself never waits on a storage round-trip.
  assert.match(page, /storageCatalogApi\.getPetPreview\(scopeId, storageId, row\.resource_id!\)/);
  assert.match(page, /previewAttempts/);
  assert.doesNotMatch(page, /getPetPreview[\s\S]{0,400}listScopeResources\(/);
  // The tile letterboxes in both stylesheets: the tile itself in App.css,
  // the card's template extension beside the template it extends.
  assert.match(readFileSync("src/App.css", "utf8"), /\.catalog-pet-preview \{/);
  assert.match(
    readFileSync("src/RetroTheme.css", "utf8"),
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
  assert.match(page, /storageCatalogApi\.applyPull\(pullPlan\.plan_id, choices, \[\.\.\.pullAcks\]\)/);
  // The reviews are the project's own review chrome.
  assert.match(page, /SyncReviewSelectionBar/);
  assert.match(page, /sync-inline-push-review sync-sync-review-workspace/);
  assert.match(page, /sync-inline-pull-review sync-sync-review-workspace/);
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

test("Push review keeps matching resources visible but outside its selection", () => {
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");

  assert.match(page, /const selectableRows = pushPlan\.rows\.filter\(\(row\) => row\.row\.selectable\);/);
  assert.match(
    page,
    /const visibleRows = pushPlan\.rows\.filter\(\(row\) => \(\s*row\.row\.selectable \|\| row\.row\.relation === "matching"\s*\)\);/,
  );
  assert.match(page, /total=\{selectableRows\.length\}/);
  assert.match(page, /onRecommended=\{\(\) => setPushSelected\(new Set\(selectableRows\.map/);
  assert.match(page, /\{visibleRows\.map\(\(planRow\) => \{/);
  assert.match(page, /selectable=\{planRow\.row\.selectable\}/);
  assert.match(page, /onToggle=\{planRow\.row\.selectable \? \(\) => setPushSelected/);
  assert.match(page, /acks=\{planRow\.row\.selectable && pushSelected\.has\(resourceId\)/);
  assert.match(page, /\{selectableRows\.length === 0 && \(/);
  assert.match(page, /\{rows\.map\(\(row\) => \{/);
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
  assert.match(add, /storageCatalogApi\.createScope\(provider, chosenProfileId, kind\)/);
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
  assert.doesNotMatch(add, /Manage saved homes|removeProfile|storageCatalogApi\.deleteProfile/);
  assert.doesNotMatch(add, /probe_catalog_resource_profile|probeProviderProfile/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /catalog-setup-(grid|card|step)/);
});

test("the profile is named by its folder, and renamed from the row's page", () => {
  const add = readFileSync("src/components/storage-catalog/AddCatalogScopeWorkspace.tsx", "utf8");
  // Setup never asks for a name: no name input, no separate commit step. A
  // pending path is saved as part of Add itself, then the scope lands on it.
  assert.doesNotMatch(add, /Profile name|Save home|displayName|catalog-profile-save-row/);
  assert.match(add, /await storageCatalogApi\.saveProfile\(provider, expandedProfilePath\)/);
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
  const types = readFileSync("src/components/storage-catalog/types.ts", "utf8");
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
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
