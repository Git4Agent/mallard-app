import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
  preserveSyncedPushRecipeEntries,
  pullRequiredCopy,
  pushReviewBlockers,
  pushReviewPullHolds,
  recommendedPushSelection,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import ResourceInventory from "../../src/components/project-sync/ResourceInventory";
import { recipeWithSelection } from "../../src/components/project-sync/model";
import type {
  BundleRecipe,
  ConversationPlanActionType,
  ConversationReasonCode,
  ProjectResourceDescriptor,
  ThreadSyncComparison,
  ThreadSyncDecision,
} from "../../src/types";

function decision(
  action: ConversationPlanActionType,
  reasonCode: ConversationReasonCode,
  selectable = false,
): ThreadSyncDecision {
  return {
    action,
    reason_code: reasonCode,
    selectable,
    selected_by_default: selectable,
    requires_explicit_approval: false,
  };
}

function memoryResource(
  name: string,
  blockedReason: string | null = null,
): ProjectResourceDescriptor {
  return {
    resource_id: `codex:memory:${name}`,
    kind: "project_memory",
    provider: "codex",
    scope: "provider_state",
    display_name: name,
    provenance: { kind: "unknown" },
    apply_policy: blockedReason ? "explicit_review" : "safe_file",
    codec_version: 1,
    metadata: {},
    category: "conversations",
    logical_paths: [`state/codex/memory/${name}/context.md`],
    default_selected: false,
    blocked_reason: blockedReason,
  };
}

function conversation(resourceId: string): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: "conversation",
    provider: "codex",
    scope: "provider_state",
    display_name: resourceId,
    provenance: { kind: "conversation" },
    apply_policy: "safe_file",
    codec_version: 1,
    metadata: {},
    category: "conversations",
  };
}

test("recommended Push selection includes only locally changed sessions", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const sessionIndex = conversation("codex:session-index");
  const memory = memoryResource("frontend-memory");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "local-ahead",
        resource_id: localAhead.resource_id,
        display_name: "Local ahead",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
        pull_decision: decision("keep_local", "cloud_prefix_of_local"),
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_op", "identical"),
        pull_decision: decision("no_op", "identical"),
      },
      {
        thread_id: "storage-ahead",
        resource_id: storageAhead.resource_id,
        display_name: "Storage ahead",
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "local_prefix_of_cloud"),
        pull_decision: decision("fast_forward_local", "local_prefix_of_cloud", true),
      },
    ],
  };

  const recommended = recommendedPushSelection(
    new Set([synced.resource_id, storageAhead.resource_id, sessionIndex.resource_id, memory.resource_id]),
    [localAhead, synced, storageAhead, sessionIndex, memory],
    comparison,
  );

  assert.deepEqual(
    [...recommended].sort(),
    [localAhead.resource_id, sessionIndex.resource_id, memory.resource_id].sort(),
  );

  const sourceRecipe: BundleRecipe = {
    schema_version: 1,
    revision: 3,
    entries: Object.fromEntries([synced, sessionIndex, memory].map((resource) => [resource.resource_id, {
      resource_id: resource.resource_id,
      apply_policy: resource.apply_policy,
      required: false,
    }])),
  };
  const selectedRecipe: BundleRecipe = {
    ...sourceRecipe,
    entries: { [memory.resource_id]: sourceRecipe.entries[memory.resource_id] },
  };
  const publishedRecipe = preserveSyncedPushRecipeEntries(selectedRecipe, sourceRecipe, comparison);
  assert.deepEqual(
    Object.keys(publishedRecipe.entries).sort(),
    [synced.resource_id, sessionIndex.resource_id, memory.resource_id].sort(),
    "unchecked synced sessions stay stored without appearing in the recommended action set",
  );
});

test("non-uploadable sessions cannot remain in the active Push selection", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "local-ahead",
        resource_id: localAhead.resource_id,
        display_name: "Local ahead",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
        pull_decision: decision("keep_local", "cloud_prefix_of_local"),
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_op", "identical"),
        pull_decision: decision("no_op", "identical"),
      },
      {
        thread_id: "storage-ahead",
        resource_id: storageAhead.resource_id,
        display_name: "Storage ahead",
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "local_prefix_of_cloud"),
        pull_decision: decision("fast_forward_local", "local_prefix_of_cloud", true),
      },
    ],
  };

  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([localAhead.resource_id, synced.resource_id, storageAhead.resource_id]),
      [localAhead, synced, storageAhead],
      comparison,
    )],
    [localAhead.resource_id],
  );
});

test("conversation divergence holds only its thread instead of blocking Push", () => {
  const resource = conversation("codex:session:diverged");
  const entry = {
    thread_id: "provider-diverged",
    resource_id: resource.resource_id,
    display_name: "Diverged",
    state: "diverged" as const,
    local_present: true,
    storage_present: true,
    push_decision: decision("require_pull", "diverged"),
    pull_decision: decision("resolve_divergence", "diverged", true),
  };
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 0, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [{
      ...entry,
      cloud_thread_id: "thread-diverged",
    }],
  };

  assert.deepEqual(
    pushReviewBlockers(new Set([resource.resource_id]), comparison),
    [],
    "a divergence never disables the whole Push; its row is simply not pushable",
  );
  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([resource.resource_id]),
      [resource],
      comparison,
    )],
    [],
    "the divergent row itself stays unselectable until a Pull resolves it",
  );
  assert.deepEqual(
    pushReviewPullHolds(comparison).map((held) => held.resource_id),
    [resource.resource_id],
    "the held thread is surfaced for the Pull-first affordance",
  );
});

test("blocked resources remain unselectable in the inventory", () => {
  const reason = "memory directory is unreadable";
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[
        memoryResource("review-one", reason),
        memoryResource("review-two", reason),
      ]}
      selected={new Set()}
      statuses={new Map()}
      onToggle={() => undefined}
    />,
  );

  assert.equal((html.match(/sync-resource-row blocked/g) ?? []).length, 2);
  assert.doesNotMatch(html, /sync-resource-row-selectable|sync-selection-placeholder/);
  assert.doesNotMatch(html, /type="checkbox"/);
});

test("blocked resources cannot enter a published recipe", () => {
  const allowed = memoryResource("frontend-memory");
  const blocked = memoryResource("broken-memory", "context.md is unreadable");
  const recipe = recipeWithSelection(
    { schema_version: 1, revision: 3, entries: {} },
    [allowed, blocked],
    new Set([allowed.resource_id, blocked.resource_id]),
  );

  assert.deepEqual(Object.keys(recipe.entries), [allowed.resource_id]);
});

test("the push review is one screen: header actions, list, Push in the footer", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, />Push review</);
  assert.match(html, /sync-sync-review-title/);
  assert.doesNotMatch(html, /Choose what to include/);
  assert.match(html, /aria-label="Use recommended selection \(1\)"/);
  assert.match(html, /aria-label="Clear current selection"/);
  assert.match(html, /aria-label="Close push review"/);
  const recommendedAction = html.indexOf('aria-label="Use recommended selection (1)"');
  const clearAction = html.indexOf('aria-label="Clear current selection"');
  const closeAction = html.indexOf('aria-label="Close push review"');
  assert.ok(recommendedAction < clearAction && clearAction < closeAction);
  // The Review step is gone and a single step renders no tab strip at all.
  assert.doesNotMatch(html, /role="tablist"/);
  assert.doesNotMatch(html, />Review</);
  assert.doesNotMatch(html, /Next: |Back: /);
  assert.match(html, />Push<\/button>/);
  const footer = html.slice(html.indexOf('<footer class="sync-inline-action-footer sync-push-resource-footer'));
  assert.doesNotMatch(footer, /<small>/, "footers are buttons only");
  assert.doesNotMatch(footer, /\d+ included|Push \d+ resources?/);
  assert.doesNotMatch(html, /Remote history is missing/);
});

test("two steps render two plain tabs — never a Review tab", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      showProjectFiles
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, /role="tablist"/);
  assert.equal(html.match(/role="tab"/g)?.length, 2);
  assert.match(html, />Git &amp; sessions</);
  assert.match(html, />Project files</);
  assert.doesNotMatch(html, />Review</);
  assert.doesNotMatch(html, /Next: |Back: /, "tabs are navigation, not a wizard");
  assert.match(html, />Push<\/button>/);
});

test("destination gates never render inside the push review", () => {
  const resource = memoryResource("review");
  const destinations = [
    { state: "remote_history_missing", base_generation: 3 },
    {
      state: "remote_history_replaced",
      base_generation: 1,
      remote_generation: 2,
      base_manifest_sha256: "a".repeat(64),
      remote_manifest_sha256: "b".repeat(64),
    },
    { state: "requires_pull", reason: "remote_advanced", remote_generation: 4, base_generation: 3 },
  ];
  for (const destinationStatus of destinations) {
    const html = renderToStaticMarkup(
      <PushResourceWorkspace
        resources={[resource]}
        selected={new Set([resource.resource_id])}
        projectDefaults={new Set([resource.resource_id])}
        busy={false}
        error={null}
        storageName="R2 storage 1"
        destinationStatus={destinationStatus as never}
        onToggle={() => undefined}
        onClose={() => undefined}
        onPush={() => undefined}
        onPull={() => undefined}
        onUnlinkStorage={() => undefined}
      />,
    );

    // These states close the review before it opens — ProjectSync lands them
    // as a storage-card notice instead. The workspace keeps only a defensive
    // disable with the reason on the button.
    assert.doesNotMatch(html, /Remote history is missing|Storage history changed|Storage has newer changes/);
    assert.doesNotMatch(html, /Open Pull review/);
    const pushButton = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[^>]*>/)?.[0];
    assert.match(pushButton ?? "", /disabled=""/);
    assert.match(pushButton ?? "", /title="Resolve the storage notice before pushing"/);
  }
});

test("the pull-required sentence counts the conversations a Pull would bring", () => {
  // storage (2) + diverged (1); the sentence names conversations because that
  // is all the comparison counted. ProjectSync feeds this straight into the
  // storage-card notice.
  assert.match(
    pullRequiredCopy("remote_advanced", "R2 storage 1", 4, 3, 3).body,
    /^3 conversations are newer in R2 storage 1\. Pushing could overwrite them\.$/,
  );
});

test("an unreviewed storage and a changed binding each get their own plain sentence", () => {
  const unreviewed = pullRequiredCopy("remote_without_base", "R2 storage 1", 6, undefined);
  assert.equal(unreviewed.title, "Storage has unreviewed data");
  assert.equal(unreviewed.body, "R2 storage 1 holds data never reviewed on this computer.");
  assert.equal(unreviewed.detail, "Generation 6 · never reviewed");

  const rebound = pullRequiredCopy("binding_changed", null, 6, 5);
  assert.equal(rebound.title, "Project changed since last sync");
  assert.match(rebound.body, /Review this storage first\./);

  // One conversation reads as one, not as "1 conversations are".
  assert.match(
    pullRequiredCopy("remote_advanced", "R2 storage 1", 4, 3, 1).body,
    /^1 conversation is newer in R2 storage 1\./,
  );
});

test("held threads and storage blockers share one banner with one way forward", () => {
  const held = conversation("codex:session:held");
  const pushable = conversation("codex:session:pushable");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 1, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "held",
        resource_id: held.resource_id,
        display_name: "Held",
        state: "diverged",
        local_present: true,
        storage_present: true,
        cloud_thread_id: "thread-held",
        push_decision: decision("require_pull", "diverged"),
        pull_decision: decision("resolve_divergence", "diverged", true),
      },
      {
        thread_id: "pushable",
        resource_id: pushable.resource_id,
        display_name: "Pushable",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
        pull_decision: decision("keep_local", "cloud_prefix_of_local"),
      },
    ],
  };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[held, pushable]}
      selected={new Set([pushable.resource_id])}
      projectDefaults={new Set([pushable.resource_id])}
      busy={false}
      error={null}
      storageName="Local storage 1"
      threadComparison={comparison}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );

  assert.equal(html.match(/sync-sync-review-pull-holds/g)?.length, 1, "one banner, not two");
  assert.match(html, />1 conversation held for Pull</);
  assert.match(html, /Storage has newer versions\. Selected threads still push\./);
  assert.equal(html.match(/Open Pull review/g)?.length, 1);
  assert.match(html, /class="sync-callout sync-sync-review-pull-holds" role="status"/);
  // The held thread keeps its cloud version and Push stays available for the rest.
  const pushButton = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[^>]*>/)?.[0];
  assert.doesNotMatch(pushButton ?? "", /disabled=""/);

  // The banner is plan-derived: deselecting every row leaves it identical, so
  // ticking a checkbox can never reflow the list underneath it.
  const deselected = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[held, pushable]}
      selected={new Set()}
      projectDefaults={new Set([pushable.resource_id])}
      busy={false}
      error={null}
      storageName="Local storage 1"
      threadComparison={comparison}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );
  const bannerZone = (markup: string) => markup.match(/<div class="sync-review-banners">[\s\S]*?<\/div><\/div>/)?.[0];
  assert.ok(bannerZone(html), "the held banner renders");
  assert.equal(bannerZone(deselected), bannerZone(html), "the banner zone does not change with selection");
});

test("the pixel skin transitions nothing, so a tick cannot flicker its neighbours", () => {
  // A transitioning element is promoted to its own compositing layer for the
  // duration, and unsmoothed crisp-edged icons re-rasterised against that
  // layer's origin flip whole pixels — the screen appears to flicker with zero
  // geometry change. Ticking one checkbox used to animate four things at once.
  const retro = readFileSync("src/RetroTheme.css", "utf8");
  const blanket = retro.match(/\.sync-app \*,\s*\n\.sync-app \*::before,\s*\n\.sync-app \*::after \{[^}]*\}/)?.[0] ?? "";
  assert.match(blanket, /transition: none/, "the skin drops transitions app-wide");

  // The base sheet styles the row checkbox with a class *and* a type, which
  // outranks the one-class blanket rule, so the skin repeats those selectors.
  const checkbox = retro.match(/\.sync-sync-row-choice input,[\s\S]*?\{[^}]*\}/g) ?? [];
  const nulled = checkbox.find((rule) => /transition: none/.test(rule)) ?? "";
  assert.match(nulled, /\.sync-sync-row-choice input::before/, "the tick's transform must not animate");
  assert.match(nulled, /\.sync-sync-review-workspace \.sync-pull-item-select input/);

  // Animations are deliberately untouched: the spinners still run.
  assert.doesNotMatch(blanket, /animation/, "dropping animations would stop the spinners");
});

test("a row's status icon is keyed off its state, not off the selection", () => {
  // The icon sits beside the checkbox: keying it off the selection-dependent
  // decision phrase swapped the glyph under the very cursor that ticked it.
  const source = readFileSync("src/components/project-sync/ProjectChatHistoryPage.tsx", "utf8");
  const map = source.match(/const REVIEW_STATE_ICONS[\s\S]*?\};/)?.[0] ?? "";
  assert.match(map, /local_ahead: "upload"/, "the map is keyed by entry state");
  assert.match(map, /storage_ahead: "download"/);
  assert.doesNotMatch(map, /"Not selected"|"Update cloud"/, "no selection-dependent phrases key the icon");
  // The phrase still reaches the user, in the tooltip and the label, where it
  // costs no layout.
  assert.match(source, /REVIEW_STATE_ICONS\[state \?\? "unknown"\]/);
  assert.doesNotMatch(source, /REVIEW_STATE_ICONS\[label\]/, "the label must not pick the icon");
});

test("the header count reserves its widest slot so ticking a row moves no icon", () => {
  // tabular-nums equalises digit widths but not digit counts: 9 → 10 grew the
  // string by a character and pushed the header's icon buttons 6px sideways.
  const rows = Array.from({ length: 13 }, (_, i) => conversation(`codex:session:n${i}`));
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 13, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: rows.map((row, i) => ({
      thread_id: `n${i}`,
      resource_id: row.resource_id,
      display_name: `Conversation ${i}`,
      state: "local_ahead" as const,
      local_present: true,
      storage_present: true,
      push_decision: decision("fast_forward_cloud", "cloud_prefix_of_local", true),
      pull_decision: decision("keep_local", "cloud_prefix_of_local"),
    })),
  };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={rows}
      selected={new Set(rows.slice(0, 9).map((row) => row.resource_id))}
      projectDefaults={new Set()}
      busy={false}
      error={null}
      storageName="Local storage 1"
      threadComparison={comparison}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );
  // The numerator sits in its own slot, sized from the total's digit count.
  assert.match(html, /class="sync-sync-review-selected" style="--review-count-digits:2"/);
  assert.match(html, /<b>9<\/b>\/13/);

  const css = readFileSync("src/RetroTheme.css", "utf8");
  const slot = css.match(/\.sync-sync-review-selected > b \{[^}]+\}/)?.[0] ?? "";
  assert.match(slot, /min-width: calc\(var\(--review-count-digits, 1\) \* 1ch\)/);
  assert.match(slot, /display: inline-block/, "an inline element cannot hold a width");
  assert.match(slot, /text-align: right/);
});

test("a selected item that cannot be captured reports in the footer, not above the list", () => {
  // The banner zone sits above the list inside the scroll: a banner that
  // appeared on a checkbox click would shove the clicked row out from under
  // the cursor (measured at 255px before this fix).
  const broken = conversation("codex:session:broken");
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 1, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "broken",
        resource_id: broken.resource_id,
        display_name: "Broken",
        state: "unavailable",
        local_present: true,
        storage_present: false,
        push_decision: decision("create_cloud", "local_only", true),
        pull_decision: decision("no_op", "local_only"),
      },
    ],
  };
  const render = (selected: Set<string>) => renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[broken]}
      selected={selected}
      projectDefaults={new Set()}
      busy={false}
      error={null}
      storageName="Local storage 1"
      threadComparison={comparison}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onPull={() => undefined}
    />,
  );

  const unselected = render(new Set());
  const selected = render(new Set([broken.resource_id]));
  assert.doesNotMatch(unselected, /sync-review-banners/);
  assert.doesNotMatch(selected, /sync-review-banners/, "selection must not add a banner above the list");
  const CANNOT_CAPTURE = /1 selected item can&#x27;t be captured — exclude it to push\./;
  assert.doesNotMatch(unselected, CANNOT_CAPTURE);
  assert.match(selected, CANNOT_CAPTURE);
  // The footer is outside the scroll, so its note cannot move the rows.
  const footer = selected.match(/<footer class="sync-inline-action-footer[^"]*">[\s\S]*?<\/footer>/)?.[0] ?? "";
  assert.match(footer, CANNOT_CAPTURE);
  assert.match(footer, /disabled=""/, "Push stays blocked while the item is selected");
});
