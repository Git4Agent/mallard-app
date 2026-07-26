import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
  nextPushReviewStep,
  preserveSyncedPushRecipeEntries,
  pushSelectableResourceIds,
  recommendedPushSelection,
  sanitizePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import ResourceInventory from "../../src/components/project-sync/ResourceInventory";
import { recipeWithSelection } from "../../src/components/project-sync/model";
import type { BundleRecipe, ProjectResourceDescriptor } from "../../src/types";
import type { ThreadSyncComparison } from "../../src/types";

function standaloneSkill(
  effectiveName: string,
  installDirectory: string,
  blockedReason: string | null = null,
): ProjectResourceDescriptor {
  return {
    resource_id: `codex:standalone-skill:${effectiveName}`,
    kind: "standalone_skill",
    provider: "codex",
    scope: "provider_state",
    display_name: effectiveName,
    provenance: {
      kind: "standalone_snapshot",
      stable_key: `custom-skill:v1:codex:${effectiveName}`,
    },
    apply_policy: blockedReason ? "explicit_review" : "safe_file",
    codec_version: 1,
    metadata: {
      effective_name: effectiveName,
      install_dir_name: installDirectory,
      provider_adapter_version: "2",
    },
    category: "skills",
    logical_paths: [`state/codex/skills/${installDirectory}/SKILL.md`],
    default_selected: false,
    blocked_reason: blockedReason,
    install_behavior: "install on restore",
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
  const skill = standaloneSkill("frontend-skill", "frontend-skill");
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
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
      },
      {
        thread_id: "storage-ahead",
        resource_id: storageAhead.resource_id,
        display_name: "Storage ahead",
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
      },
    ],
  };

  const recommended = recommendedPushSelection(
    new Set([synced.resource_id, storageAhead.resource_id, sessionIndex.resource_id, skill.resource_id]),
    [localAhead, synced, storageAhead, sessionIndex, skill],
    comparison,
  );

  assert.deepEqual(
    [...recommended].sort(),
    [localAhead.resource_id, sessionIndex.resource_id, skill.resource_id].sort(),
  );
  assert.equal(
    pushSelectableResourceIds(
      [localAhead, synced, storageAhead, sessionIndex, skill],
      comparison,
    ).has(synced.resource_id),
    false,
    "an up-to-date session is informational, not a Push action",
  );
  assert.deepEqual(
    [...sanitizePushSelection(
      new Set([synced.resource_id, sessionIndex.resource_id]),
      [localAhead, synced, storageAhead, sessionIndex, skill],
      comparison,
    )],
    [],
    "the internal session index cannot keep an otherwise empty Push selected",
  );

  const sourceRecipe: BundleRecipe = {
    schema_version: 1,
    revision: 3,
    entries: Object.fromEntries([synced, sessionIndex, skill].map((resource) => [resource.resource_id, {
      resource_id: resource.resource_id,
      apply_policy: resource.apply_policy,
      required: false,
    }])),
  };
  const selectedRecipe: BundleRecipe = {
    ...sourceRecipe,
    entries: { [skill.resource_id]: sourceRecipe.entries[skill.resource_id] },
  };
  const publishedRecipe = preserveSyncedPushRecipeEntries(selectedRecipe, sourceRecipe, comparison);
  assert.deepEqual(
    Object.keys(publishedRecipe.entries).sort(),
    [synced.resource_id, sessionIndex.resource_id, skill.resource_id].sort(),
    "unchecked synced sessions stay stored without appearing in the recommended action set",
  );
});

function inputTagFor(html: string, accessibleName: string): string {
  const marker = `aria-label="${accessibleName}"`;
  const markerIndex = html.indexOf(marker);
  assert.notEqual(markerIndex, -1, `missing input '${accessibleName}'`);
  const start = html.lastIndexOf("<input", markerIndex);
  const end = html.indexOf(">", markerIndex);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  return html.slice(start, end + 1);
}

test("a declared skill name may differ from its selectable install folder", () => {
  const resource = standaloneSkill(
    "get-real-hardware-rh-service",
    "capture-lsservice-detail",
  );
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[resource]}
      selected={new Set()}
      statuses={new Map()}
      onToggle={() => undefined}
    />,
  );

  assert.match(html, /get-real-hardware-rh-service/);
  assert.match(html, /folder capture-lsservice-detail/);
  assert.doesNotMatch(html, /contradicts directory/);
  assert.doesNotMatch(
    inputTagFor(html, "Include get-real-hardware-rh-service"),
    /\bdisabled\b/,
    "a valid declared-name/folder mismatch must stay selectable",
  );
});

test("duplicate effective-name claims remain unselectable", () => {
  const reason = "effective skill name 'review' is declared by multiple directories";
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[
        standaloneSkill("review", "review-one", reason),
        {
          ...standaloneSkill("review", "review-two", reason),
          resource_id: "codex:standalone-skill:review-two-blocked",
        },
      ]}
      selected={new Set()}
      statuses={new Map()}
      onToggle={() => undefined}
    />,
  );

  assert.match(html, /folder review-one/);
  assert.match(html, /folder review-two/);
  assert.equal((html.match(/v3-resource-row blocked/g) ?? []).length, 2);
  const disabledInputs = html.match(/<input[^>]*disabled=""[^>]*>/g) ?? [];
  assert.equal(disabledInputs.length, 2);
});

test("blocked resources cannot enter a published recipe", () => {
  const allowed = standaloneSkill("frontend-skill", "frontend-skill");
  const blocked = standaloneSkill("broken-skill", "broken-skill", "SKILL.md is unreadable");
  const recipe = recipeWithSelection(
    { schema_version: 1, revision: 3, entries: {} },
    [allowed, blocked],
    new Set([allowed.resource_id, blocked.resource_id]),
  );

  assert.deepEqual(Object.keys(recipe.entries), [allowed.resource_id]);
});

test("the push chooser shows one concise selection summary", () => {
  const resource = standaloneSkill("review", "review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, />Push review</);
  assert.match(html, /v3-sync-review-title/);
  assert.doesNotMatch(html, /Choose what to include/);
  assert.match(html, /aria-label="Use recommended selection \(1\)"/);
  assert.match(html, /aria-label="Clear current selection"/);
  assert.match(html, /aria-label="Close push review"/);
  const recommendedAction = html.indexOf('aria-label="Use recommended selection (1)"');
  const clearAction = html.indexOf('aria-label="Clear current selection"');
  const closeAction = html.indexOf('aria-label="Close push review"');
  assert.ok(recommendedAction < clearAction && clearAction < closeAction);
  assert.match(html, />Review</);
  assert.match(html, /Back: Plugins/);
  assert.match(html, /btn btn-secondary v3-sync-review-back/);
  assert.match(html, />Push<\/button>/);
  const footer = html.slice(html.indexOf('<footer class="v3-inline-action-footer v3-push-resource-footer'));
  assert.doesNotMatch(footer, /\d+ included|Push \d+ resources?/);
  assert.doesNotMatch(html, /Choose resources to push/);
  assert.doesNotMatch(html, /Selections are saved after Push succeeds/);
  assert.doesNotMatch(html, /last selection|selection will be remembered|resource selected/i);
});

test("Push review counts only outstanding sessions and keeps synced sessions informational", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const sessionIndex = conversation("codex:session-index");
  const resources = [localAhead, synced, sessionIndex];
  const comparison: ThreadSyncComparison = {
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "R2 storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        thread_id: "local-ahead",
        resource_id: localAhead.resource_id,
        display_name: "Local ahead",
        state: "local_ahead",
        local_present: true,
        storage_present: true,
      },
      {
        thread_id: "synced",
        resource_id: synced.resource_id,
        display_name: "Synced",
        state: "synced",
        local_present: true,
        storage_present: true,
      },
    ],
  };
  const mixedHtml = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={resources}
      selected={new Set(resources.map((resource) => resource.resource_id))}
      projectDefaults={new Set(resources.map((resource) => resource.resource_id))}
      busy={false}
      error={null}
      initialStep="review"
      threadComparison={comparison}
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(mixedHtml, /aria-label="Git &amp; sessions, 1 selected"/);
  assert.match(mixedHtml, /aria-label="Use recommended selection \(1\)"/);
  assert.match(mixedHtml, />1 included/);
  assert.doesNotMatch(mixedHtml, /project session index|codex:session-index/i);

  const syncedOnlyHtml = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[synced, sessionIndex]}
      selected={new Set([synced.resource_id, sessionIndex.resource_id])}
      projectDefaults={new Set([synced.resource_id, sessionIndex.resource_id])}
      busy={false}
      error={null}
      initialStep="review"
      threadComparison={{
        ...comparison,
        counts: { ...comparison.counts, local: 0 },
        entries: comparison.entries.filter((entry) => entry.state === "synced"),
      }}
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  const pushButton = syncedOnlyHtml.match(
    /<button[^>]*v3-pull-apply-button[^>]*>.*?Push<\/button>/s,
  )?.[0] ?? "";

  assert.match(syncedOnlyHtml, /aria-label="Git &amp; sessions, 0 selected"/);
  assert.match(syncedOnlyHtml, /aria-label="Use recommended selection \(0\)"/);
  assert.match(syncedOnlyHtml, />0 included/);
  assert.match(pushButton, /disabled=""/);
  assert.doesNotMatch(syncedOnlyHtml, /project session index|codex:session-index/i);
});

test("the push chooser provides contextual back and next actions", () => {
  const resource = standaloneSkill("review", "review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error={null}
      initialStep="skills"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, /Back: Git &amp; sessions/);
  assert.match(html, /Next: Plugins/);
  assert.equal(nextPushReviewStep("history"), "skills");
  assert.equal(nextPushReviewStep("skills"), "plugins");
  assert.equal(nextPushReviewStep("plugins"), "review");
  assert.equal(nextPushReviewStep("review"), "review");
});
