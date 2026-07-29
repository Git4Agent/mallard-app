import assert from "node:assert/strict";
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
  requiredProjectContentDirectoryIds,
} from "../../src/components/project-sync/PushResourceWorkspace";
import RestorePlanView from "../../src/components/project-sync/RestorePlanView";
import SyncReviewTabs, { syncReviewSteps } from "../../src/components/project-sync/SyncReviewTabs";
import { applyPullReview } from "../../src/components/project-sync/pullReviewFlow";
import {
  buildPullReviewItems,
  defaultSelected,
  includeRequiredPullProjectDirectories,
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
    scope: "project",
    display_name: path.split("/").slice(-1)[0],
    provenance: { kind: "project_local", relative_path: path },
    apply_policy: "explicit_review",
    relative_cwd: null,
    codec_version: 1,
    metadata: {},
    category: "project_files",
    logical_paths: [`project/${path}`],
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
  storage_generation: null,
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

const emptyCounts = { history: 0, skills: 0, plugins: 0, project_files: 0, review: 0 };

test("eligible reviews put Project files directly between Plugins and Review", () => {
  const eligible = renderToStaticMarkup(
    <SyncReviewTabs
      activeStep="project_files"
      steps={syncReviewSteps(true)}
      counts={emptyCounts}
      onChange={() => undefined}
    />,
  );
  const plugins = eligible.indexOf(">Plugins<");
  const projectFiles = eligible.indexOf(">Project files<");
  const review = eligible.indexOf(">Review<");
  assert.ok(plugins >= 0 && plugins < projectFiles && projectFiles < review);

  const git = renderToStaticMarkup(
    <SyncReviewTabs
      activeStep="plugins"
      steps={syncReviewSteps(false)}
      counts={emptyCounts}
      onChange={() => undefined}
    />,
  );
  assert.doesNotMatch(git, />Project files</);
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
      initialStep="project_files"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  assert.match(html, /New · selected after scan/);
  assert.match(html, /required directory/);
  assert.match(html, /3 entr(?:y|ies)/);
  assert.match(html, /2 ignored · 1 blocked/);
  assert.doesNotMatch(html, /sync-project-file-selection-placeholder|type="checkbox"/);
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
      initialStep="review"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.match(html, />Project files</);
  assert.doesNotMatch(html, /<strong>Project files<\/strong>|Rescan|Search files/);
});

test("before Scan, Push uses a compact optional-files action", () => {
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
      initialStep="project_files"
      onToggle={() => undefined}
      onUseProjectDefaults={() => undefined}
      onClear={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
      onScanProjectFiles={() => undefined}
    />,
  );
  assert.match(html, /Files outside Git/);
  assert.match(html, /Scan files/);
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

  assert.match(html, /Project files/);
  assert.match(html, /Rescan/);
  assert.match(html, /No changes/);
  assert.doesNotMatch(html, /Search files|0 entries|0 selected|No eligible entries|Scanning cannot prove/);
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
  generation: 1,
  commit_id: "commit-a",
  manifest_sha256: "b".repeat(64),
  binding_revision: 0,
  created_at: 1,
  expires_at: 2,
  project_content_eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
  conversation_actions: [],
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
  const items = buildPullReviewItems(pullPlan, null);
  assert.ok(items.every((item) => item.category === "project_content"));
  assert.ok(items.every((item) => !defaultSelected(item)));
  const selected = includeRequiredPullProjectDirectories(items, new Set(["file-a"]));
  assert.deepEqual([...selected].sort(), ["dir-docs", "dir-specs", "file-a"]);

  const html = renderToStaticMarkup(
    <RestorePlanView
      projectName="Project B"
      profileLabel="Profile"
      plan={pullPlan}
      binding={binding}
      dependencyPlan={null}
      readiness={null}
      restoreResult={null}
      dependencyResult={null}
      phase="idle"
      supportLoading={false}
      completedActionIds={new Set()}
      completedResourceIds={new Set()}
      failedResourceIds={new Set()}
      busy={false}
      error={null}
      initialStep="review"
      onApply={() => undefined}
      onRefresh={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, /Plugins[\s\S]*Project files[\s\S]*Review/);
  assert.doesNotMatch(html, /<strong>Project files<\/strong>|0 apply · 3 keep local/);
  assert.match(html, />Keep project entries local</);
});

test("confirming keep-local still submits an empty approved set so Pull can advance its base", async () => {
  const calls: string[] = [];
  const result = await applyPullReview(
    {
      applyRestore: async (_planId, actionIds) => {
        calls.push(`restore:${actionIds.length}`);
        return { success: true, message: "Kept project files local", applied_action_ids: [], failed_actions: [] };
      },
      applyDependencies: async () => {
        throw new Error("not expected");
      },
      getRestoreReadiness: async () => {
        calls.push("verify");
        return { bundle_id: binding.bundle_id, state: "ready", issues: [] };
      },
    },
    pullPlan,
    null,
    { resourceIds: [], restoreActionIds: [], dependencyActionIds: [] },
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
