import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import PushResourceWorkspace, {
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
  ConversationReviewAction,
  InventoryResource,
  ThreadSyncEntry,
  ThreadSyncComparison,
  ThreadSyncDecision,
} from "../../src/types";

function decision(
  action: ConversationReviewAction,
  _classification: string,
  selectable = false,
): ThreadSyncDecision {
  return {
    action,
    selectable,
  };
}

/* Inventory rows as the backend flattens them: the portable descriptor plus
   the presentation its registered contracts supply. */
function memoryResource(
  name: string,
  blockedReason: string | null = null,
): InventoryResource {
  return {
    resource_id: `codex:memory:${name}`,
    kind: { key: "provider_state", version: 1 },
    provider: "codex",
    provider_resource_type: "memory",
    display_name: name,
    provenance: { kind: "unknown", version: 1, fields: {} },
    apply_policy: { key: blockedReason ? "explicit_review" : "safe_file", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Provider state",
    apply_policy_label: blockedReason ? "Explicit review" : "Safe file",
    selectable: true,
    logical_paths: [`state/codex/memory/${name}/context.md`],
    default_selected: false,
    blocked_reason: blockedReason,
  };
}

function conversation(resourceId: string): InventoryResource {
  return {
    resource_id: resourceId,
    kind: { key: "conversation", version: 1 },
    provider: "codex",
    provider_resource_type: "session",
    display_name: resourceId,
    provenance: { kind: "conversation", version: 1, fields: {} },
    apply_policy: { key: "safe_file", version: 1 },
    codec_version: 1,
    attributes: {},
    category: "conversations",
    kind_label: "Conversation",
    apply_policy_label: "Safe file",
    selectable: true,
    default_selected: false,
  };
}

function threadEntryBase(
  threadId: string,
  resourceId: string,
  displayName: string,
): Pick<
  ThreadSyncEntry,
  | "provider"
  | "provider_display_name"
  | "access"
  | "thread_id"
  | "conversation_resource_id"
  | "resource_id"
  | "display_name"
  | "compared_against_storage"
  | "subagent_selection"
> {
  return {
    provider: "codex",
    provider_display_name: "Codex",
    access: "bound_ready",
    thread_id: threadId,
    conversation_resource_id: resourceId,
    resource_id: resourceId,
    display_name: displayName,
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
  };
}

const comparisonDefaults: Pick<
  ThreadSyncComparison,
  "scopes" | "provider_errors" | "remote_continuity"
> = {
  scopes: [],
  provider_errors: {},
  remote_continuity: { state: "in_sync", revision: 1 },
};

test("recommended Push selection includes only locally changed sessions", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const memory = memoryResource("frontend-memory");
  const comparison: ThreadSyncComparison = {
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        ...threadEntryBase("local-ahead", localAhead.resource_id, "Local ahead"),
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("publish", "local_outstanding", true),
        pull_decision: decision("report_push_eligible", "local_outstanding"),
      },
      {
        ...threadEntryBase("synced", synced.resource_id, "Synced"),
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_changes", "sync_current"),
        pull_decision: decision("no_changes", "sync_current"),
      },
      {
        ...threadEntryBase("storage-ahead", storageAhead.resource_id, "Storage ahead"),
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "cloud_outstanding"),
        pull_decision: decision("materialize_cloud", "cloud_outstanding", true),
      },
    ],
  };

  const recommended = recommendedPushSelection(
    new Set([synced.resource_id, storageAhead.resource_id, memory.resource_id]),
    [localAhead, synced, storageAhead, memory],
    comparison,
  );

  assert.deepEqual(
    [...recommended].sort(),
    [localAhead.resource_id, memory.resource_id].sort(),
  );

  const sourceRecipe: BundleRecipe = {
    schema_version: 1,
    revision: 3,
    entries: Object.fromEntries([synced, memory].map((resource) => [resource.resource_id, {
      resource_id: resource.resource_id,
      apply_policy: resource.apply_policy,
      required: false,
    }])),
  };
  const publishedRecipe = recipeWithSelection(
    sourceRecipe,
    [localAhead, synced, storageAhead, memory],
    recommended,
  );
  assert.deepEqual(
    Object.keys(publishedRecipe.entries).sort(),
    [localAhead.resource_id, memory.resource_id].sort(),
    "stored sessions outside the selection must not enter the capture recipe",
  );
});

test("non-uploadable sessions cannot remain in the active Push selection", () => {
  const localAhead = conversation("codex:session:local-ahead");
  const synced = conversation("codex:session:synced");
  const storageAhead = conversation("codex:session:storage-ahead");
  const comparison: ThreadSyncComparison = {
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 1, local: 1, storage: 1, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        ...threadEntryBase("local-ahead", localAhead.resource_id, "Local ahead"),
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("publish", "local_outstanding", true),
        pull_decision: decision("report_push_eligible", "local_outstanding"),
      },
      {
        ...threadEntryBase("synced", synced.resource_id, "Synced"),
        state: "synced",
        local_present: true,
        storage_present: true,
        push_decision: decision("no_changes", "sync_current"),
        pull_decision: decision("no_changes", "sync_current"),
      },
      {
        ...threadEntryBase("storage-ahead", storageAhead.resource_id, "Storage ahead"),
        state: "storage_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("require_pull", "cloud_outstanding"),
        pull_decision: decision("materialize_cloud", "cloud_outstanding", true),
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
    ...threadEntryBase("provider-diverged", resource.resource_id, "Diverged"),
    state: "diverged" as const,
    local_present: true,
    storage_present: true,
    push_decision: decision("require_pull", "divergent"),
    pull_decision: decision("keep_both", "diverged", true),
  };
  const comparison: ThreadSyncComparison = {
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 0, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [{
      ...entry,
      cloud_thread_id: "thread-diverged",
      compared_against_storage: true,
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
      onToggle={() => undefined}
    />,
  );

  assert.equal((html.match(/sync-resource-row blocked/g) ?? []).length, 2);
  assert.doesNotMatch(html, /sync-resource-row-selectable|sync-selection-placeholder/);
  assert.doesNotMatch(html, /type="checkbox"/);
});

test("a group states its own name once, and counts its blocked rows rather than assuming them", () => {
  const requirement = (
    name: string,
    blockedReason: string | null = null,
  ): InventoryResource => ({
    ...memoryResource(name, blockedReason),
    resource_id: `codex:requirement:${name}`,
    kind: { key: "requirement", version: 1 },
    kind_label: "Requirement",
    category: "tools",
  });
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[requirement("missing", "Resource is unavailable at the mapped paths"), requirement("present")]}
      selected={new Set()}
      defaultCollapsed
      onToggle={() => undefined}
    />,
  );

  // One header for one list. The push step used to wrap this in a <details>
  // that repeated the word, the chevron and a count of its own.
  assert.equal((html.match(/>Requirements</g) ?? []).length, 1);
  assert.equal((html.match(/sync-resource-group-chevron/g) ?? []).length, 1);
  // Counted, not assumed: two rows, one blocked. The wrapper called both
  // unavailable while the row below it still offered a checkbox.
  assert.match(html, /sync-resource-group-unavailable[^>]*title="1 of 2 unavailable on this machine"/);
  assert.match(html, /1 unavailable/);
  assert.match(html, /sync-resource-group-count">0\/2</);
  // Folded on request, so the header is all a secondary group costs.
  assert.doesNotMatch(html, /sync-resource-list/);

  // A group with nothing blocked shows no tally at all.
  const clean = renderToStaticMarkup(
    <ResourceInventory
      resources={[requirement("present")]}
      selected={new Set()}
      onToggle={() => undefined}
    />,
  );
  assert.doesNotMatch(clean, /sync-resource-group-unavailable/);
  assert.match(clean, /sync-resource-list/);
});

test("a resource named by its own id states that id once, and says why once", () => {
  // The shape the service returns for a recipe entry that no longer resolves
  // locally: the id is the only available name.
  const ghostId = "codex:session:687bc2c4-9612-313d-a3d2-6d29d1b003a0";
  const ghost: InventoryResource = {
    ...memoryResource("ghost", "Selected for sync, but unavailable at the mapped paths on this machine"),
    resource_id: ghostId,
    display_name: ghostId,
    kind: { key: "requirement", version: 1 },
    kind_label: "Requirement",
    category: "tools",
    description: null,
    logical_paths: [],
  };
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[ghost]}
      selected={new Set()}
      onToggle={() => undefined}
    />,
  );

  // The collapsed row prints the id once, as its title.
  assert.equal((html.match(new RegExp(ghostId, "g")) ?? []).length, 1);
  // The hidden `provider · kind` line is gone with its display:none rule, so
  // the name is the only thing in the copy slot.
  assert.match(html, /sync-resource-copy"><strong>codex:session:[^<]*<\/strong><\/span>/);

  // The fold is internal state, so static rendering cannot open it — pin the
  // guard at the source instead, and that the caption survives for a resource
  // whose name is not its id.
  const inventory = readFileSync("src/components/project-sync/ResourceInventory.tsx", "utf8");
  assert.match(
    inventory,
    /\{resource\.resource_id !== resource\.display_name && \(\s*<><dt>Resource ID<\/dt><dd><code>\{resource\.resource_id\}<\/code><\/dd><\/>\s*\)\}/,
  );
  assert.match(inventory, /<dt>Kind<\/dt><dd>\{resource\.kind_label\}<\/dd>/);
  assert.match(inventory, /<dt>Apply<\/dt><dd>\{resource\.apply_policy_label\}<\/dd>/);
  assert.doesNotMatch(inventory, /providerLabel|kindLabel\(|applyPolicyLabel\(/);
});

test("an unknown kind coordinate renders by its backend label and stays unselectable", () => {
  // A resource whose contracts this build has never heard of arrives already
  // decided by the backend: filed under tools, labelled by its raw coordinate,
  // and not selectable. The row shows that label and offers no checkbox — the
  // frontend compares no key of its own to find out.
  const unknown: InventoryResource = {
    ...memoryResource("future"),
    resource_id: "codex:future:thing",
    display_name: "Future thing",
    kind: { key: "future_kind", version: 3 },
    kind_label: "future_kind@3",
    apply_policy: { key: "future_policy", version: 2 },
    apply_policy_label: "future_policy@2",
    category: "tools",
    selectable: false,
  };
  const html = renderToStaticMarkup(
    <ResourceInventory
      resources={[unknown]}
      selected={new Set([unknown.resource_id])}
      onToggle={() => undefined}
    />,
  );

  assert.match(html, /Future thing/);
  assert.doesNotMatch(html, /type="checkbox"|sync-resource-row-selectable/);
  // The fold is internal state, so static markup cannot open it: pin at the
  // source that its Kind and Apply readouts are the DTO's labels, verbatim.
  const row = readFileSync("src/components/project-sync/ResourceInventory.tsx", "utf8");
  assert.match(row, /<dt>Kind<\/dt><dd>\{resource\.kind_label\}<\/dd>\s*<dt>Apply<\/dt><dd>\{resource\.apply_policy_label\}<\/dd>/);
  // The recipe rejects it too, for the same backend-supplied reason.
  const recipe = recipeWithSelection(
    { schema_version: 1, revision: 1, entries: {} },
    [unknown, memoryResource("present")],
    new Set([unknown.resource_id, "codex:memory:present"]),
  );
  assert.deepEqual(Object.keys(recipe.entries), ["codex:memory:present"]);
  assert.deepEqual(recipe.entries["codex:memory:present"].apply_policy, { key: "safe_file", version: 1 });

  // The source keeps no vocabulary of kinds or policies to fall back on.
  const model = readFileSync("src/components/project-sync/model.ts", "utf8");
  const inventory = readFileSync("src/components/project-sync/ResourceInventory.tsx", "utf8");
  for (const source of [model, inventory]) {
    assert.doesNotMatch(source, /\bKIND_LABELS\b|CATEGORY_BY_KIND|APPLY_POLICY_LABELS|kind\.key|apply_policy\.key|"requirement"|"never"/);
  }
});

test("the push step renders the requirements group without a second disclosure", () => {
  const push = readFileSync("src/components/project-sync/PushResourceWorkspace.tsx", "utf8");
  const pull = readFileSync("src/components/project-sync/RestorePlanView.tsx", "utf8");

  assert.doesNotMatch(push, /sync-sync-review-other/);
  assert.match(push, /<ResourceInventory\s+resources=\{other\}[\s\S]*?defaultCollapsed/);
  // The pull side keeps its wrapper: ItemList has no header of its own, so
  // there the <details> is the only thing naming the group.
  assert.match(pull, /<details className="sync-sync-review-other">/);
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
  const resource = { ...memoryResource("review"), category: "tools" as const };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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

test("a Push error names itself and offers no local-state reset", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
      resources={[resource]}
      selected={new Set([resource.resource_id])}
      projectDefaults={new Set([resource.resource_id])}
      busy={false}
      error="conversation Push action 'action-a' is stale at LineageDigest"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, /stale at LineageDigest/);
  assert.doesNotMatch(html, /Reset local sync state/);
});

test("a failed comparison names the error and offers no reset", () => {
  const resource = memoryResource("review");
  const render = (threadComparisonError: string) => renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
      resources={[resource]}
      selected={new Set()}
      projectDefaults={new Set()}
      busy={false}
      error={null}
      threadComparisonError={threadComparisonError}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  const offline = render("S3 get 'bundles/head.json': dispatch failure: io error");
  assert.match(offline, /Conversations couldn&#x27;t be compared/);
  assert.doesNotMatch(offline, /Reset local sync state/);
  // Contradictory metadata is not repaired by forgetting sync bases either:
  // no Pull or Push surface offers the reset (PLAN_PULL_PINNED_VERSION.md,
  // section 4.4).
  const stale = render("conversation checkpoint is stale at LineageDigest");
  assert.match(stale, /stale at LineageDigest/);
  assert.doesNotMatch(stale, /Reset local sync state/);
});

test("two steps render two plain tabs — never a Review tab", () => {
  const resource = memoryResource("review");
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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
  // A review that shows the files step is a non-git project, so the first
  // tab holds sessions alone — there is no Git history to co-bill it for.
  assert.match(html, />Sessions</);
  assert.match(html, />Project files</);
  assert.doesNotMatch(html, />Review</);
  assert.doesNotMatch(html, /Next: |Back: /, "tabs are navigation, not a wizard");
  assert.match(html, />Push<\/button>/);
  const primary = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[\s\S]*?<\/button>/)?.[0] ?? "";
  assert.match(primary, /data-thread-status="local-outstanding"/);
  assert.doesNotMatch(primary, /data-icon="upload"/);
});

test("blocked destinations gate before review and publish failures recheck the destination", () => {
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const openPushChooser = source.slice(
    source.indexOf("const openPushChooser = async"),
    source.indexOf("const scanPendingProjectFiles = async"),
  );
  const destinationGate = openPushChooser.indexOf('destination.state === "requires_pull"');
  const pendingPushCreation = openPushChooser.indexOf("setPendingPush({");
  assert.ok(destinationGate >= 0 && destinationGate < pendingPushCreation);
  assert.match(openPushChooser.slice(destinationGate, pendingPushCreation), /setStorageNotice\([\s\S]*return;/);
  // An unreachable storage stops at the card too, as the action-failed
  // notice whose actions are Retry, Reconnect, or a metadata-gated Reset —
  // never a review with 0/0 selectable and an unconditional Reset.
  const unavailableGate = openPushChooser.indexOf('destination.state === "unavailable"');
  assert.ok(unavailableGate >= 0 && unavailableGate < destinationGate);
  assert.match(
    openPushChooser.slice(unavailableGate, destinationGate),
    /kind: "action_failed",\s*action: "push",\s*message: storageFailureMessage\(storageId, destination\.message\)[\s\S]*return;/,
  );

  const publishProject = source.slice(
    source.indexOf("const publishProject = async"),
    source.indexOf("const publishPendingPush = async"),
  );
  assert.match(
    publishProject,
    /catch \(reason\)[\s\S]*getPushDestinationStatus\([\s\S]*destinationStatus\.state === "requires_pull"[\s\S]*setPendingPush\(null\)/,
  );
  // A cleared or recreated storage is no Push state of its own: the gate
  // answers ready or requires_pull, and nothing asks for an unlink.
  assert.doesNotMatch(source, /remote_history_missing|remote_history_replaced/);
});

test("the pull-required sentence counts the conversations a Pull would bring", () => {
  // storage (2) + diverged (1); the sentence names conversations because that
  // is all the comparison counted. ProjectSync feeds this straight into the
  // storage-card notice. It must not threaten overwrites: a push carries
  // newer cloud threads forward and holds them per thread, so the sentence
  // points at the review and what it unlocks instead.
  assert.match(
    pullRequiredCopy("remote_advanced", "R2 storage 1", 4, 3, 3).body,
    /^3 conversations are newer in R2 storage 1\. Review them in Pull — they stay held afterwards while your other sessions push\.$/,
  );
});

test("an unreviewed storage and a changed binding each get their own plain sentence", () => {
  const unreviewed = pullRequiredCopy("remote_without_base", "R2 storage 1", 6, undefined);
  assert.equal(unreviewed.title, "Storage review required");
  assert.equal(unreviewed.body, "Review the existing data in R2 storage 1 before pushing from this connection.");
  assert.equal(unreviewed.detail, "Version 6 · review required");

  const rebound = pullRequiredCopy("binding_changed", null, 6, 5);
  assert.equal(rebound.title, "Project changed since last sync");
  assert.match(rebound.body, /Review this storage first\./);

  // One conversation reads as one, not as "1 conversations are" — and the
  // pronouns follow it through the whole sentence.
  assert.match(
    pullRequiredCopy("remote_advanced", "R2 storage 1", 4, 3, 1).body,
    /^1 conversation is newer in R2 storage 1\. Review it in Pull — it stays held afterwards/,
  );
});

test("held threads and storage blockers share one banner with one way forward", () => {
  const held = conversation("codex:session:held");
  const pushable = conversation("codex:session:pushable");
  const comparison: ThreadSyncComparison = {
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 1, storage: 0, diverged: 1, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: [
      {
        ...threadEntryBase("held", held.resource_id, "Held"),
        state: "diverged",
        local_present: true,
        storage_present: true,
        cloud_thread_id: "thread-held",
        compared_against_storage: true,
        push_decision: decision("require_pull", "divergent"),
        pull_decision: decision("keep_both", "diverged", true),
      },
      {
        ...threadEntryBase("pushable", pushable.resource_id, "Pushable"),
        state: "local_ahead",
        local_present: true,
        storage_present: true,
        push_decision: decision("publish", "local_outstanding", true),
        pull_decision: decision("report_push_eligible", "local_outstanding"),
      },
    ],
  };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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
  const bannerZone = (markup: string) => markup.match(/<div class="sync-review-banners">[\s\S]*?<\/div><\/div>/)?.[0];

  // One box, structurally: the zone renders a single `.notice` however many
  // messages it holds, so "one banner, not two" is no longer a thing a call
  // site can get wrong.
  assert.equal(html.match(/class="notice /g)?.length, 1, "one banner, not two");
  assert.match(html, />1 conversation held for Pull</);
  assert.match(html, /Storage has newer versions\. Selected threads still push\./);
  assert.equal(html.match(/Open Pull review/g)?.length, 1);
  // Plan-derived content is on screen the moment the review paints, so it is a
  // named region and not a status message (WCAG 2.2 SC 4.1.3).
  assert.match(html, /class="notice warning" role="region" aria-label="Notes about this Push review"/);
  assert.doesNotMatch(html, /role="status"|role="alert"/);
  assert.match(bannerZone(html) ?? "", /data-thread-status="divergent"/);
  assert.doesNotMatch(bannerZone(html) ?? "", /data-icon="git-compare-arrows"/);
  assert.doesNotMatch(html, />Requirements</);
  assert.doesNotMatch(html, /codex:conversation:/);
  // The held thread keeps its cloud version and Push stays available for the rest.
  const pushButton = html.match(/<button type="button" class="btn btn-primary sync-pull-apply-button"[^>]*>/)?.[0];
  assert.doesNotMatch(pushButton ?? "", /disabled=""/);

  // The banner is plan-derived: deselecting every row leaves it identical, so
  // ticking a checkbox can never reflow the list underneath it.
  const deselected = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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
  assert.ok(bannerZone(html), "the held banner renders");
  assert.equal(bannerZone(deselected), bannerZone(html), "the banner zone does not change with selection");
});

test("state changes do not transition, so a tick cannot flicker its neighbours", () => {
  // A transitioning element is promoted to its own compositing layer for the
  // duration, and unsmoothed crisp-edged icons re-rasterised against that
  // layer's origin flip whole pixels — the screen appears to flicker with zero
  // geometry change. Ticking one checkbox used to animate four things at once.
  const css = readFileSync("src/App.css", "utf8");
  const blanket = css.match(/\.sync-app \*,\s*\n\.sync-app \*::before,\s*\n\.sync-app \*::after \{[^}]*\}/)?.[0] ?? "";
  assert.match(blanket, /transition: none/, "state transitions must stay disabled app-wide");

  // Match the checkbox selectors' specificity so the blanket rule also wins
  // for their border, fill, and tick.
  const checkbox = css.match(/\.sync-sync-row-choice input,[\s\S]*?\{[^}]*\}/g) ?? [];
  const nulled = checkbox.find((rule) => /transition: none/.test(rule)) ?? "";
  assert.match(nulled, /\.sync-sync-row-choice input::before/, "the tick's transform must not animate");
  assert.match(nulled, /\.sync-sync-review-workspace \.sync-pull-item-select input/);

  // Animations are deliberately untouched: the spinners still run.
  assert.doesNotMatch(blanket, /animation/, "dropping animations would stop the spinners");
});

test("a row's status icon is keyed off its state, not off the selection", () => {
  // Review and Activity share the same status presentation and pixel badge.
  // The selection-dependent phrase may change, but the status glyph does not.
  const source = readFileSync("src/components/project-sync/ProjectActivityPage.tsx", "utf8");
  assert.doesNotMatch(source, /REVIEW_STATE_ICONS|SyncDecisionBadge/);
  assert.match(source, /function ReviewSyncIndicator/);
  assert.match(source, /const presentation = sessionSyncPresentation\(entry, storageName\)/);
  assert.match(source, /<ThreadStatusIcon status=\{presentation\.status\} size=\{18\} \/>/);
});

test("the header count reserves its widest slot so ticking a row moves no icon", () => {
  // tabular-nums equalises digit widths but not digit counts: 9 → 10 grew the
  // string by a character and pushed the header's icon buttons 6px sideways.
  const rows = Array.from({ length: 13 }, (_, i) => conversation(`codex:session:n${i}`));
  const comparison: ThreadSyncComparison = {
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 13, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    warnings: [],
    entries: rows.map((row, i) => ({
      ...threadEntryBase(`n${i}`, row.resource_id, `Conversation ${i}`),
      state: "local_ahead" as const,
      local_present: true,
      storage_present: true,
      push_decision: decision("publish", "local_outstanding", true),
      pull_decision: decision("report_push_eligible", "local_outstanding"),
    })),
  };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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

  const css = readFileSync("src/App.css", "utf8");
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
    ...comparisonDefaults,
    project_id: "project-a",
    storage_id: "storage-a",
    storage_name: "Local storage 1",
    compared_at: 1,
    counts: { synced: 0, local: 1, storage: 0, diverged: 0, unavailable: 1, unknown: 0 },
    warnings: [],
    entries: [
      {
        ...threadEntryBase("broken", broken.resource_id, "Broken"),
        compared_against_storage: false,
        state: "unavailable",
        local_present: true,
        storage_present: false,
        push_decision: decision("create_cloud", "missing_cloud", true),
        pull_decision: decision("report_local_only", "missing_cloud"),
      },
    ],
  };
  const render = (selected: Set<string>) => renderToStaticMarkup(
    <PushResourceWorkspace
      credentialRedactionEnabled={true}
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


test("search can expose matching resources in a collapsed inventory group", () => {
  const resource = conversation("codex:session:search-match");
  const html = renderToStaticMarkup(
    <ResourceInventory resources={[resource]} selected={new Set()} defaultCollapsed forceExpanded onToggle={() => undefined} />,
  );
  assert.match(html, /class="sync-resource-list"/);
  assert.match(html, /aria-expanded="true"/);
  assert.match(html, /aria-label="Include /);
});
