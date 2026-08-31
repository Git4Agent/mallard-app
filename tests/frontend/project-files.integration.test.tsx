import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  ProjectBinding,
  ProjectContentEntry,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  RestorePlan,
} from "../../src/types";
import ProjectFilesReviewPage from "../../src/components/project-sync/ProjectFilesReviewPage";
import PushResourceWorkspace, {
  includeRequiredProjectContentDirectories,
  pushProjectFileCommitImpact,
  pushReviewBlockingCount,
  requiredProjectContentDirectoryIds,
} from "../../src/components/project-sync/PushResourceWorkspace";
import RestorePlanView, { pullProjectFileCommitImpact } from "../../src/components/project-sync/RestorePlanView";
import SyncReviewTabs from "../../src/components/sync-review/SyncReviewTabs";
import { projectReviewTabs, syncReviewSteps } from "../../src/components/project-sync/reviewSteps";
import { applyPullReview } from "../../src/components/project-sync/pullReviewFlow";
import {
  buildPullReviewItems,
  includeRequiredPullProjectDirectories,
  recommendedPullSelection,
} from "../../src/components/project-sync/pullReviewModel";

function descriptor(
  resourceId: string,
  kind: "project_content_file" | "project_content_directory",
  path: string,
): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind,
    provider: null,
    display_name: path.split("/").slice(-1)[0],
    provenance: { kind: "project_local", relative_path: path },
    apply_policy: "explicit_review",
    codec_version: 1,
    metadata: {},
    category: "project_files",
    logical_paths: [`project/${path}`],
    default_selected: false,
  };
}

function entry(
  resourceId: string,
  type: "file" | "directory",
  path: string,
  selectedAfterScan = true,
): ProjectContentEntry {
  return {
    descriptor: descriptor(
      resourceId,
      type === "file" ? "project_content_file" : "project_content_directory",
      path,
    ),
    entry_type: type,
    relative_path: path,
    logical_path: `project/${path}`,
    size: type === "file" ? 18 : null,
    mode: type === "file" ? 0o644 : 0o755,
    source_mtime: 1,
    state: "local_only",
    local_present: true,
    storage_present: false,
    base_present: false,
    local_digest: "a".repeat(64),
    storage_digest: null,
    base_digest: null,
    review_digest: "a".repeat(64),
    selected_in_recipe: false,
    newly_discovered: true,
    selected_after_scan: selectedAfterScan,
    blocked_reason: null,
    warning_code: null,
    warning_digest: null,
  };
}

const inventory: ProjectContentInventory = {
  local_project_id: "project-a",
  storage_id: "storage-a",
  project_root: "/tmp/project-a",
  eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  review_token: "review-token",
  storage_revision: null,
  preference_revision: 0,
  excluded_resource_ids: [],
  entries: [
    entry("dir-docs", "directory", "docs"),
    entry("dir-specs", "directory", "docs/specs"),
    entry("file-a", "file", "docs/specs/a.md"),
  ],
  ignored_count: 2,
  blocked_count: 1,
  warnings: [],
  scanned_at: 1,
};

const emptyCounts = { history: 0, project_files: 0 };

test("eligible reviews put Project files directly after Git & sessions with no Review step", () => {
  assert.deepEqual(syncReviewSteps(false), ["history"]);
  assert.deepEqual(syncReviewSteps(true), ["history", "project_files"]);

  const eligible = renderToStaticMarkup(
    <SyncReviewTabs
      tabs={projectReviewTabs(syncReviewSteps(true), emptyCounts)}
      activeId="project_files"
      onChange={() => undefined}
    />,
  );
  const history = eligible.indexOf(">Git &amp; sessions<");
  const projectFiles = eligible.indexOf(">Project files<");
  assert.ok(history >= 0 && history < projectFiles, "history renders before project files");
  assert.equal(
    (eligible.match(/role="tab"/g) ?? []).length,
    2,
    "the strip holds exactly the two remaining steps",
  );
  assert.doesNotMatch(eligible, />Review</);

  const git = renderToStaticMarkup(
    <SyncReviewTabs
      tabs={projectReviewTabs(syncReviewSteps(false), emptyCounts)}
      activeId="history"
      onChange={() => undefined}
    />,
  );
  assert.equal(git, "", "a single-step review renders no tab strip at all");
});

test("an explicit Push scan presents new entries selected and locks both directory ancestors", () => {
  const selected = new Set(inventory.entries
    .filter((candidate) => candidate.selected_after_scan)
    .map((candidate) => candidate.descriptor.resource_id));
  const required = requiredProjectContentDirectoryIds(inventory, new Set(["file-a"]));
  assert.deepEqual([...required].sort(), ["dir-docs", "dir-specs"]);
  assert.deepEqual(
    [...includeRequiredProjectContentDirectories(inventory, new Set(["file-a"]))].sort(),
    ["dir-docs", "dir-specs", "file-a"],
  );

  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={inventory.entries.map((candidate) => candidate.descriptor)}
      selected={selected}
      projectDefaults={selected}
      busy={false}
      error={null}
      projectContentInventory={inventory}
      projectContentScanned
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  // Push rows carry no state chip: the checked tick and the status mark say
  // everything the old "New · selected after scan" caption said.
  assert.doesNotMatch(html, /New · selected after scan|SELECTED AFTER SCAN/i);
  assert.match(html, /sync-thread-sync-indicator local/);
  // The directories draw no cards of their own: the full-path band speaks
  // for them and the required-ancestor rule keeps them riding their files.
  // The band's control is the rows' own checkbox — here ticked, since every
  // file is selected — with the counter chip at the band's edge; the old
  // check-circle/trash icon pair is gone.
  assert.match(html, /sync-project-files-group-heading/);
  assert.match(html, />docs\/specs\/</);
  assert.match(html, /aria-label="1 of 1 selected in docs\/specs"/);
  assert.match(html, /aria-label="Exclude every file in docs\/specs"[^>]*checked/);
  assert.doesNotMatch(html, /Select all files in|Clear the .* selection|sync-sync-review-icon-action[\s\S]{0,400}folder/);
  assert.equal(html.match(/catalog-card/g)?.length, 1, "only the file draws a card");
  assert.match(html, /aria-label="Include docs\/specs\/a\.md"[^>]*checked/);
  assert.match(html, /1 entry/);
  assert.match(html, /2 ignored · 1 blocked/);
  assert.doesNotMatch(html, /sync-project-file-selection-placeholder/);
  assert.doesNotMatch(html, /Exact entries selected|Scanning cannot prove/);
});

test("Project file controls stay on the dedicated page", () => {
  const selected = new Set(inventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={inventory.entries.map((candidate) => candidate.descriptor)}
      selected={selected}
      projectDefaults={selected}
      busy={false}
      error={null}
      projectContentInventory={inventory}
      projectContentScanned
      showProjectFiles
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, />Project files</);
  assert.doesNotMatch(html, /<strong>Project files<\/strong>|Rescan|Search files/);
});

test("the footer carries the step navigator beside Push, forward then back", () => {
  const selected = new Set(inventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const render = (activeStep: "history" | "project_files") => renderToStaticMarkup(
    <PushResourceWorkspace
      resources={inventory.entries.map((candidate) => candidate.descriptor)}
      selected={selected}
      projectDefaults={selected}
      busy={false}
      error={null}
      projectContentInventory={inventory}
      projectContentScanned
      showProjectFiles
      activeStep={activeStep}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  const onSessions = render("history");
  assert.match(onSessions, /title="Review project files next"/);
  assert.match(onSessions, />Push<\/button>/, "Push stays live beside the navigator");
  const onFiles = render("project_files");
  assert.match(onFiles, /title="Back to session selection"/);
  assert.match(onFiles, />Push<\/button>/);
});

test("warnings acknowledge through the catalog chip, and removal is the card's trash", () => {
  const warned = entry("file-w", "file", "notes/api_key.txt");
  warned.warning_code = "possible_secret";
  warned.warning_digest = "w".repeat(64);
  const stored = entry("file-s", "file", "notes/old_draft.md");
  stored.storage_present = true;
  stored.state = "synced";
  const rows = [warned, stored].map((candidate) => ({
    resourceId: candidate.descriptor.resource_id,
    relativePath: candidate.relative_path,
    entryType: candidate.entry_type as "file",
    state: candidate.state,
    size: candidate.size,
    mode: candidate.mode,
    sourceMtime: candidate.source_mtime,
    localPresent: candidate.local_present,
    storagePresent: candidate.storage_present,
    selectedAfterScan: candidate.selected_after_scan,
    warningCode: candidate.warning_code,
    warningDigest: candidate.warning_digest,
  }));
  const html = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={rows}
      selectedIds={new Set(["file-w", "file-s"])}
      removalIds={new Set()}
      onToggle={() => undefined}
      onToggleRemoval={() => undefined}
      onToggleWarning={() => undefined}
    />,
  );
  assert.match(html, /catalog-card-ack/);
  assert.match(html, /Credential-shaped content<\/strong> — notes\/api_key\.txt/);
  assert.match(html, />Mark reviewed</);
  assert.match(html, /aria-label="Remove notes\/old_draft\.md from storage"/);
  assert.match(html, /data-icon="trash"/);

  const removing = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={rows}
      selectedIds={new Set(["file-w"])}
      removalIds={new Set(["file-s"])}
      onToggle={() => undefined}
      onToggleRemoval={() => undefined}
      onToggleWarning={() => undefined}
    />,
  );
  assert.match(removing, /catalog-card removal/);
  assert.match(removing, />Removes from storage</);
  assert.match(removing, /aria-label="Keep notes\/old_draft\.md in storage"/);
});

test("project file commits count operations and gate selected deletions", () => {
  const storedFile = {
    ...entry("file-old", "file", "old.txt"),
    state: "storage_only" as const,
    local_present: false,
    storage_present: true,
  };
  const storedDirectory = {
    ...entry("dir-old", "directory", "old"),
    state: "storage_only" as const,
    local_present: false,
    storage_present: true,
  };
  const removalInventory = {
    ...inventory,
    entries: [storedFile, storedDirectory],
  };
  const removals = new Set(["file-old", "dir-old"]);
  assert.deepEqual(
    pushProjectFileCommitImpact(removalInventory, new Set(), removals),
    { add: 0, replace: 0, deleteFile: 1, deleteDirectory: 1 },
  );
  assert.equal(
    pushReviewBlockingCount(new Set(), undefined, removalInventory, new Set(), removals),
    0,
  );

  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={removalInventory.entries.map((candidate) => candidate.descriptor)}
      selected={new Set()}
      projectDefaults={new Set()}
      busy={false}
      error={null}
      projectContentInventory={removalInventory}
      projectContentScanned
      projectContentRemovals={removals}
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  assert.match(html, /<strong>0<\/strong> Add/);
  assert.match(html, /<strong>0<\/strong> Replace/);
  assert.match(html, /<strong>1<\/strong> Delete files/);
  assert.match(html, /<strong>1<\/strong> Delete folders/);
  assert.match(html, /Approve removal of 1 file and 1 folder from storage\./);
  assert.match(html, /title="Approve the selected project file deletions"/);

  const deletionPlan: RestorePlan = {
    ...pullPlan,
    actions: [
      ...pullPlan.actions,
      {
        action_id: "delete-old-file",
        resource_id: "delete-file",
        kind: {
          kind: "delete_project_file",
          logical_path: "project/old.txt",
          last_sha256: "d".repeat(64),
        },
        target_path: "/tmp/project-b/old.txt",
        requires_explicit_approval: true,
      },
      {
        action_id: "delete-old-directory",
        resource_id: "delete-directory",
        kind: { kind: "delete_project_directory", logical_path: "project/old" },
        target_path: "/tmp/project-b/old",
        requires_explicit_approval: true,
      },
    ],
  };
  const deletionItems = buildPullReviewItems(deletionPlan);
  assert.deepEqual(
    pullProjectFileCommitImpact(deletionItems, new Set(["delete-file", "delete-directory"])),
    { add: 0, replace: 0, deleteFile: 1, deleteDirectory: 1 },
  );

  const pullSource = readFileSync("src/components/project-sync/RestorePlanView.tsx", "utf8");
  assert.match(pullSource, /projectFileDeletionCount\(projectFileImpact\) > 0 && !deletionApproved/);
  assert.match(pullSource, /<ProjectFileCommitImpact[\s\S]*?deletionTarget="local"/);
});

test("file review details reuse the project copyable popover", () => {
  const html = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={[{
        resourceId: "file-design",
        relativePath: "doc/test-design.md",
        entryType: "file",
        state: "synced",
        size: 18,
        mode: 0o644,
        sourceMtime: 1,
        localPresent: true,
        storagePresent: true,
      }]}
      selectedIds={new Set(["file-design"])}
      onToggle={() => undefined}
    />,
  );
  assert.match(html, /aria-label="Show details for doc\/test-design\.md"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /<strong>File details<\/strong>/);
  assert.match(html, /<span>Modified<\/span>/);
  assert.match(html, />Copy all<\/button>/);
  assert.doesNotMatch(html, /sync-project-file-details/);
});

test("opening the files step scans by itself — no interstitial, just the scaffold", () => {
  const html = renderToStaticMarkup(
    <PushResourceWorkspace
      resources={[]}
      selected={new Set()}
      projectDefaults={new Set()}
      busy={false}
      error={null}
      projectContentInventory={null}
      projectContentScanned={false}
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onScanProjectFiles={() => undefined}
    />,
  );
  // The step is the opt-in: entering it starts the scan, so there is no
  // "Files outside Git" page and no Scan button to press first. The scaffold
  // itself sits behind LoadingScaffold's anti-flicker gate, which a static
  // render never passes — what this render CAN prove is that the interstitial
  // is gone and nothing claims "No changes" before the scan has spoken.
  assert.doesNotMatch(html, /Files outside Git|Scan files/);
  assert.doesNotMatch(html, /No changes/);
  assert.match(html, /Rescan/, "the bar's retry survives a failed first scan");
  assert.doesNotMatch(html, />Optional|Nothing is uploaded|New eligible files|Review them before continuing/);
  assert.doesNotMatch(html, /a\.md/);
});

test("an empty file review keeps only the action and state", () => {
  const html = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={[]}
      selectedIds={new Set()}
      onScan={() => undefined}
      onToggle={() => undefined}
    />,
  );

  assert.match(html, /aria-label="Project files"/, "the section keeps its name without a heading row");
  assert.match(html, /Rescan/);
  assert.match(html, /No changes/);
  // The one-line bar keeps its search box (disabled — nothing to filter) and
  // hides the tally rather than printing "0 entries".
  assert.match(html, /placeholder="Search files…"[^>]*disabled/);
  assert.doesNotMatch(html, /0 entries|0 selected|No eligible entries|Scanning cannot prove|<h3/);
});

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica-b",
  local_project_id: "project-b",
  bundle_id: "11111111111111111111111111111111",
  project_root: "/tmp/project-b",
  canonical_project_root: "/tmp/project-b",
  profile_ids: {},
  state: "active",
  revision: 0,
  updated_at: 1,
};

const pullPlan: RestorePlan = {
  schema_version: 1,
  plan_id: "plan-project-files",
  storage_id: "storage-a",
  bundle_id: binding.bundle_id,
  host_id: binding.host_id,
  replica_id: binding.replica_id,
  expected_host_identity_revision: 0,
  revision: 1,
  commit_id: "commit-a",
  manifest_sha256: "b".repeat(64),
  binding_revision: 0,
  created_at: 1,
  expires_at: 2,
  project_content_eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  conversation_actions: [],
  conversation_review: [],
  actions: [
    {
      action_id: "ensure-docs",
      resource_id: "dir-docs",
      kind: { kind: "ensure_project_directory", logical_path: "project/docs", mode: 0o755, source_mtime: 1 },
      target_path: "/tmp/project-b/docs",
      requires_explicit_approval: true,
    },
    {
      action_id: "ensure-specs",
      resource_id: "dir-specs",
      kind: { kind: "ensure_project_directory", logical_path: "project/docs/specs", mode: 0o755, source_mtime: 1 },
      target_path: "/tmp/project-b/docs/specs",
      requires_explicit_approval: true,
    },
    {
      action_id: "write-a",
      resource_id: "file-a",
      kind: { kind: "write_project_file", logical_path: "project/docs/specs/a.md", mode: 0o644, source_mtime: 1 },
      target_path: "/tmp/project-b/docs/specs/a.md",
      source_sha256: "c".repeat(64),
      requires_explicit_approval: true,
    },
  ],
};

test("Pull project content starts as keep-local and selecting a nested file requires its folders", () => {
  const items = buildPullReviewItems(pullPlan);
  assert.ok(items.every((item) => item.category === "project_content"));
  // Nothing starts selected — but the header's Recommended now knows the
  // files step's safe set: adds and new directories, never a replace or a
  // delete. (Every project-content action requires explicit approval by
  // backend policy, so the gate is the operation; pressing Recommended is
  // the review.) This absorbed the toolbar's "Select safe additions".
  assert.deepEqual(
    items.filter(recommendedPullSelection).map((item) => item.resourceId).sort(),
    ["dir-docs", "dir-specs", "file-a"],
  );
  const selected = includeRequiredPullProjectDirectories(items, new Set(["file-a"]));
  assert.deepEqual([...selected].sort(), ["dir-docs", "dir-specs", "file-a"]);
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="Project B"
      plan={pullPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );

  assert.equal((html.match(/role="tab"/g) ?? []).length, 2);
  const history = html.indexOf(">Sessions<");
  const projectFiles = html.indexOf(">Project files<");
  assert.ok(history >= 0 && history < projectFiles, "history stays the first step");
  assert.doesNotMatch(html, />Review</);
  assert.match(html, /aria-selected="true"[^>]*aria-label="Sessions, 0 selected"/);
  assert.match(html, /id="sync-review-history-panel"/);
  assert.match(html, />No session changes</);
  assert.doesNotMatch(html, /sync-project-files-page/, "the file review stays on its own step");
  // The third smart-select is gone for good: the header's Recommended is the
  // safe selection now, and the toolbar is a search bar again.
  assert.doesNotMatch(html, /Select safe additions/);
  assert.match(
    html,
    /class="btn btn-primary sync-pull-apply-button">/,
    "undecided file entries keep Pull available so keep-local can advance the base",
  );
  assert.match(html, /title="Review project files next"/, "the footer carries the step navigator");

  const filesHtml = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="pull"
      eligibility={pullPlan.project_content_eligibility!}
      rows={[
        {
          resourceId: "dir-docs",
          relativePath: "docs",
          entryType: "directory",
          state: "create_directory",
          mode: 0o755,
          sourceMtime: 1,
          localPresent: false,
          storagePresent: true,
          operation: "create_directory",
        },
        {
          resourceId: "dir-specs",
          relativePath: "docs/specs",
          entryType: "directory",
          state: "create_directory",
          mode: 0o755,
          sourceMtime: 1,
          localPresent: false,
          storagePresent: true,
          operation: "create_directory",
        },
        {
          resourceId: "file-a",
          relativePath: "docs/specs/a.md",
          entryType: "file",
          state: "add",
          mode: 0o644,
          sourceMtime: 1,
          localPresent: false,
          storagePresent: true,
          operation: "add",
        },
      ]}
      selectedIds={new Set()}
      onToggle={() => undefined}
    />,
  );
  // The folders draw no cards: the full-path band and its subtree cluster
  // speak for them, and approving the file rides the required-ancestor rule.
  assert.match(filesHtml, />docs\/specs\/</);
  assert.match(filesHtml, /aria-label="0 of 1 selected in docs\/specs"/, "every entry starts as keep-local");
  assert.equal(filesHtml.match(/catalog-card/g)?.length, 1, "only the file draws a card");
  assert.match(filesHtml, /aria-label="Apply docs\/specs\/a\.md"/);
  assert.match(filesHtml, />Add file</);
});

test("generic pull rows wear the same operation chips", () => {
  const genericPlan: RestorePlan = {
    ...pullPlan,
    plan_id: "plan-generic",
    project_content_eligibility: undefined,
    actions: [
      {
        action_id: "write-state",
        resource_id: "state-config",
        kind: { kind: "write_file", logical_path: "codex/state/config.json" },
        target_path: "/tmp/codex/config.json",
        source_sha256: "c".repeat(64),
        requires_explicit_approval: false,
      },
      {
        action_id: "replace-state",
        resource_id: "state-notes",
        kind: { kind: "write_file", logical_path: "codex/state/notes.md" },
        target_path: "/tmp/codex/notes.md",
        source_sha256: "c".repeat(64),
        expected_target_sha256: "d".repeat(64),
        requires_explicit_approval: true,
      },
    ],
  };
  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="Project B"
      plan={genericPlan}
      binding={binding}
      readiness={null}
      restoreResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, />Add file</);
  assert.match(html, />Replace file</);
});

test("a diverged pull row carries and names its preserved copy", () => {
  const preservedPlan: RestorePlan = {
    ...pullPlan,
    plan_id: "plan-preserve",
    actions: [
      {
        action_id: "write-diverged",
        resource_id: "file-d",
        kind: {
          kind: "write_project_file",
          logical_path: "project/draft.md",
          mode: 0o644,
          source_mtime: 1,
          preserve: {
            copy_logical_path: "project/draft-local-copy.md",
            local_content_sha256: "d".repeat(64),
            storage_content_sha256: "e".repeat(64),
            reusable_witness: false,
          },
        },
        target_path: "/tmp/project-b/draft.md",
        source_sha256: "e".repeat(64),
        expected_target_sha256: "d".repeat(64),
        requires_explicit_approval: true,
      },
    ],
  };
  const items = buildPullReviewItems(preservedPlan);
  assert.equal(items.length, 1);
  assert.equal(items[0].projectContentOperation, "replace");
  assert.equal(items[0].projectContentPreservedCopy, "draft-local-copy.md");

  const filesHtml = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="pull"
      eligibility={pullPlan.project_content_eligibility!}
      rows={[
        {
          resourceId: "file-d",
          relativePath: "draft.md",
          entryType: "file",
          state: "replace",
          mode: 0o644,
          sourceMtime: 1,
          localPresent: true,
          storagePresent: true,
          operation: "replace",
          preservedCopyPath: "draft-local-copy.md",
        },
      ]}
      selectedIds={new Set(["file-d"])}
      onToggle={() => undefined}
    />,
  );
  assert.ok(
    filesHtml.includes("Replace file · keeps local copy"),
    "the row names the non-lossy replace",
  );
});

test("confirming keep-local still submits an empty approved set so Pull can advance its base", async () => {
  const calls: string[] = [];
  const result = await applyPullReview(
    {
      applyRestore: async (_planId, actionIds) => {
        calls.push(`restore:${actionIds.length}`);
        return {
          success: true,
          status: "complete",
          message: "Kept project files local",
          plan_id: pullPlan.plan_id,
          applied_action_ids: [],
          failed_actions: [],
          divergences: [],
          preserved_threads: [],
        };
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "ready", issues: [] };
      },
    },
    pullPlan,
    { resourceIds: [], restoreActionIds: [] },
    () => undefined,
  );
  assert.deepEqual(calls, ["restore:0", "verify"]);
  assert.equal(result.success, true);
});

test("stored project files stay visible but locked when the binding is Git-managed", () => {
  const html = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="pull"
      eligibility={{ state: "git_managed", reason: "Git manages this folder.", detected_root: "/tmp/repo" }}
      rows={[{
        resourceId: "file-a",
        relativePath: "a.md",
        entryType: "file",
        state: "add",
        localPresent: false,
        storagePresent: true,
        operation: "add",
      }]}
      selectedIds={new Set()}
      onToggle={() => undefined}
    />,
  );
  assert.match(html, /Project files are locked/);
  assert.match(html, /Git manages this folder/);
  assert.match(html, /a\.md/);
  assert.doesNotMatch(html, /aria-label="Apply a\.md"/);
  assert.doesNotMatch(html, /sync-project-file-selection-placeholder|type="checkbox"/);
});
