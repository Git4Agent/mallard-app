import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  ProjectBinding,
  ProjectContentEntry,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  PullReview,
  ThreadSyncComparison,
} from "../../src/types";
import ProjectFilesReviewPage, {
  searchProjectFileReviewCards,
  type ProjectFileReviewRow,
} from "../../src/components/project-sync/ProjectFilesReviewPage";
import PushResourceWorkspace, {
  PushApplyButton,
  includeRequiredProjectContentDirectories,
  projectFileSelectionAfterScan,
  pushBlockerRevealTarget,
  pushBudgetTotal,
  pushProjectFileWork,
  pushProjectFileCommitImpact,
  pushReviewBlockers,
  pushReviewDisabledReason,
  requiredProjectContentDirectoryIds,
  selectPushProjectFilesWithinBudget,
  updatePushSelection,
} from "../../src/components/project-sync/PushResourceWorkspace";
import RestorePlanView, { pullProjectFileCommitImpact } from "../../src/components/project-sync/RestorePlanView";
import { projectContentReviewRow } from "../../src/components/project-sync/model";
import SyncReviewTabs from "../../src/components/sync-review/SyncReviewTabs";
import { projectReviewTabs, syncReviewSteps } from "../../src/components/project-sync/reviewSteps";
import { applyPullReview } from "../../src/components/project-sync/pullReviewFlow";
import {
  buildPullReviewItems,
  includeRequiredPullProjectDirectories,
  pullWaitingFiles,
  recommendedPullSelection,
} from "../../src/components/project-sync/pullReviewModel";
import { i18n } from "../../src/i18n";

/* A scanned entry carries the bare portable descriptor, as the backend's
   `ProjectContentEntry` does; the review row is built from the entry. */
function descriptor(
  resourceId: string,
  type: "file" | "directory",
  path: string,
): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: { key: type === "file" ? "project_content_file" : "project_content_directory", version: 1 },
    provider: null,
    display_name: path.split("/").slice(-1)[0],
    provenance: { kind: "project_local", version: 1, fields: { relative_path: path } },
    apply_policy: { key: "explicit_review", version: 1 },
    codec_version: 1,
    attributes: {},
  };
}

function entry(
  resourceId: string,
  type: "file" | "directory",
  path: string,
  undecided = true,
): ProjectContentEntry {
  return {
    descriptor: descriptor(resourceId, type, path),
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
    undecided,
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

const emptyDirectoryInventory: ProjectContentInventory = {
  ...inventory,
  entries: [
    entry("dir-assets", "directory", "素材"),
    entry("dir-previews", "directory", "素材/previews"),
    entry("dir-backgrounds", "directory", "素材/previews/backgrounds"),
    entry("file-notes", "file", "素材/notes.md"),
  ],
};

test("Python scripts have no executable warning or acknowledgement in Push and Pull", () => {
  const path = "姜维一计害三贤_180秒/新版配音/制作/make_neural_audio.py";
  const script = { ...entry("script", "file", path), mode: 0o755 };
  const scanned = { ...inventory, entries: [script] };
  const selected = new Set(["script"]);
  assert.equal(pushReviewDisabledReason({ selected, projectContentInventory: scanned, deletionApproved: true }), null);
  for (const mode of ["push", "pull"] as const) {
    const html = renderToStaticMarkup(
      <ProjectFilesReviewPage
        mode={mode} eligibility={scanned.eligibility}
        rows={[{ resourceId: "script", relativePath: path, entryType: "file", state: "local_only", mode: 0o755,
          localPresent: true, storagePresent: false,
          warningCode: script.warning_code, warningDigest: script.warning_digest }]}
        selectedIds={selected} onToggle={() => undefined} onReviewWarnings={() => undefined}
      />,
    );
    assert.match(html, /make_neural_audio\.py/);
    assert.match(html, /Python/);
    assert.doesNotMatch(html, /Executable|notice-chip warning|notice-ack|Mark reviewed/);
  }
});

test("clearing files removes stored choices and required ancestors while preserving other resources", () => {
  const scanned = {
    ...inventory,
    entries: inventory.entries.map((row) => ({ ...row, storage_present: true, selected_in_recipe: true, undecided: false })),
  };
  const selected = includeRequiredProjectContentDirectories(scanned, new Set(["file-a", "session", "tool"]));
  const next = new Set([...selected].filter((id) => !scanned.entries.some((row) => row.descriptor.resource_id === id)));
  assert.deepEqual(updatePushSelection(scanned, selected, next), new Set(["session", "tool"]));
  assert.equal(pushProjectFileWork(scanned, next, new Set(), true), null, "unselect is not storage deletion");
});

test("unselecting a file or folder removes unused ancestors but keeps other selected branches and empty directories", () => {
  const scanned = {
    ...inventory,
    entries: [
      ...inventory.entries,
      entry("dir-images", "directory", "docs/images"),
      entry("file-image", "file", "docs/images/cover.png"),
      entry("dir-empty", "directory", "empty"),
    ],
  };
  const selected = includeRequiredProjectContentDirectories(scanned, new Set(["file-a", "file-image", "dir-empty"]));
  const next = new Set(selected);
  next.delete("file-a");
  const withoutFile = updatePushSelection(scanned, selected, next);
  assert.deepEqual(withoutFile, new Set(["file-image", "dir-images", "dir-docs", "dir-empty"]));

  const withoutFolder = new Set(withoutFile);
  ["file-image", "dir-images"].forEach((id) => withoutFolder.delete(id));
  assert.deepEqual(updatePushSelection(scanned, withoutFile, withoutFolder), new Set(["dir-empty"]));
});

test("rescanning preserves unselected stored files and cleans ancestors of disappeared selections", () => {
  const scanned = {
    ...inventory,
    entries: inventory.entries.map((row) => ({ ...row, storage_present: true, selected_in_recipe: true })),
  };
  const defaults = projectFileSelectionAfterScan(null, scanned, new Set(["session"]), new Set());
  assert.deepEqual(defaults, new Set(["session", "dir-docs", "dir-specs", "file-a"]));
  assert.deepEqual(projectFileSelectionAfterScan(scanned, scanned, new Set(["session"]), new Set()), new Set(["session"]));
  const disappeared = { ...scanned, entries: scanned.entries.filter((row) => row.entry_type === "directory") };
  assert.deepEqual(projectFileSelectionAfterScan(scanned, disappeared, defaults, new Set()), new Set(["session"]));
  assert.deepEqual(projectFileSelectionAfterScan(null, scanned, new Set(), new Set(["file-a"])), new Set(["dir-docs", "dir-specs"]));
});

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

test("a ticked new file locks both directory ancestors and still reads New", () => {
  const selected = includeRequiredProjectContentDirectories(inventory, new Set(["file-a"]));
  const required = requiredProjectContentDirectoryIds(inventory, new Set(["file-a"]));
  assert.deepEqual([...required].sort(), ["dir-docs", "dir-specs"]);
  assert.deepEqual(
    [...includeRequiredProjectContentDirectories(inventory, new Set(["file-a"]))].sort(),
    ["dir-docs", "dir-specs", "file-a"],
  );

  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={inventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected}
      projectDefaults={selected}
      busy={false}
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
  // Being new is a fact of the file, not of its tick: the chip stays.
  assert.match(html, /class="sync-history-branch-state current"[^>]*>New</);
  // The directories draw no cards of their own: the folder's framed tile
  // speaks for them and the required-ancestor rule keeps them riding their
  // files. The tile head's control is the rows' own checkbox — here ticked,
  // since every file is selected — with the counter chip closing the strip;
  // the old check-circle/trash icon pair is gone.
  assert.match(html, /sync-list-tile-head/);
  assert.match(html, />docs\/specs\/</);
  assert.match(html, /aria-label="1 of 1 selected in docs\/specs"/);
  assert.match(html, /aria-label="Exclude every file in docs\/specs"[^>]*checked/);
  assert.doesNotMatch(html, /Select all files in|Clear the .* selection|sync-sync-review-icon-action[\s\S]{0,400}folder/);
  assert.equal(html.match(/catalog-card/g)?.length, 1, "only the file draws a card");
  assert.match(html, /aria-label="Include docs\/specs\/a\.md"[^>]*checked/);
  assert.match(html, /class="sync-name-search-count">1 file</);
  assert.match(html, /2 ignored · 1 blocked/);
  assert.doesNotMatch(html, /sync-project-file-selection-placeholder/);
  assert.doesNotMatch(html, /Exact entries selected|Scanning cannot prove/);
});

test("selecting a nested empty directory includes and locks its ancestors without selecting sibling files", () => {
  const selected = includeRequiredProjectContentDirectories(
    emptyDirectoryInventory,
    new Set(["dir-backgrounds"]),
  );
  assert.deepEqual([...selected].sort(), ["dir-assets", "dir-backgrounds", "dir-previews"]);
  assert.deepEqual(
    [...requiredProjectContentDirectoryIds(emptyDirectoryInventory, selected)].sort(),
    ["dir-assets", "dir-previews"],
    "the empty directory itself stays optional",
  );
});

test("Files Select includes every ancestor of an empty directory alongside sibling files", () => {
  const selected = selectPushProjectFilesWithinBudget(
    new Set(),
    new Set(emptyDirectoryInventory.entries.map((candidate) => candidate.descriptor.resource_id)),
    null,
    emptyDirectoryInventory,
  );
  assert.deepEqual(
    [...selected].sort(),
    ["dir-assets", "dir-backgrounds", "dir-previews", "file-notes"],
  );
});

test("the scan ticks no new file: it opens unticked and New, and the Sessions step counts it", () => {
  // What Select applies is not what opens ticked: the scan offers the new
  // file, and leaves the tick to the user.
  const offered = new Set(inventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const render = (
    activeStep: "history" | "project_files",
    selected: Set<string>,
    reviewed: ProjectContentInventory = inventory,
  ) => renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={reviewed.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected}
      projectDefaults={offered}
      busy={false}
      projectContentInventory={reviewed}
      projectContentScanned
      showProjectFiles
      activeStep={activeStep}
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  assert.equal(projectContentReviewRow(inventory.entries[2]).default_selected, false);

  const onFiles = render("project_files", new Set());
  assert.doesNotMatch(onFiles, /aria-label="Include docs\/specs\/a\.md"[^>]*checked/);
  assert.match(onFiles, /aria-label="0 of 1 selected in docs\/specs"/);
  assert.match(onFiles, /class="sync-history-branch-state current"[^>]*>New</);
  // Select is live: it is how the new files without a finding get their tick.
  const select = onFiles.match(/<button type="button" class="btn sync-sync-review-select"[^>]*>/)?.[0] ?? "";
  assert.doesNotMatch(select, /disabled/);

  // The file that waits is counted on the button that opens its list, and
  // Push stays live: nothing here needs the file to travel.
  // A box, not a ghost: as a ghost its count was easy to push past.
  const onSessions = render("history", new Set());
  assert.match(onSessions, /<button type="button" class="btn" title="1 new project file is not in this push">/);
  assert.match(onSessions, /<span class="sync-path-count">1 new<\/span>/);

  // Ticked, it no longer waits.
  const ticked = render("history", includeRequiredProjectContentDirectories(inventory, new Set(["file-a"])));
  assert.match(ticked, /title="Review project files next"/);
  assert.doesNotMatch(ticked, /sync-path-count">\d+ new/);

  // Left out at an earlier push, it is not counted and not New.
  const leftOut: ProjectContentInventory = {
    ...inventory,
    entries: inventory.entries.map((candidate) => ({ ...candidate, undecided: false })),
  };
  assert.match(render("history", new Set(), leftOut), /title="Review project files next"/);
  assert.doesNotMatch(render("project_files", new Set(), leftOut), />New</);
});

test("Files Select includes previously excluded and warned files without acknowledging warnings", () => {
  const flagged = entry("file-run", "file", "scripts/run.sh");
  flagged.warning_code = "possible_secret";
  flagged.warning_digest = "w".repeat(64);
  const leftOut = entry("file-l", "file", "notes/draft.md", false);
  const scanned: ProjectContentInventory = {
    ...inventory,
    excluded_resource_ids: [leftOut.descriptor.resource_id],
    entries: [
      ...inventory.entries,
      entry("dir-scripts", "directory", "scripts"),
      flagged,
      entry("dir-empty", "directory", "assets"),
      entry("dir-notes", "directory", "notes", false),
      leftOut,
    ],
  };
  const defaults = includeRequiredProjectContentDirectories(scanned, new Set(["file-a"]));
  const render = (selected: Set<string>, acknowledgedWarningDigests = new Set<string>()) => renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={scanned.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected} projectDefaults={defaults} busy={false}
      projectContentInventory={scanned} projectContentScanned showProjectFiles
      activeStep="project_files" acknowledgedWarningDigests={acknowledgedWarningDigests}
      onReviewProjectContentWarnings={() => undefined}
      onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined}
    />,
  );
  const initial = render(defaults);
  assert.match(initial, /title="Select all eligible project files \(8\)"/);
  assert.match(initial, /Select <span class="sync-path-count">8<\/span>/);
  assert.doesNotMatch(initial, /Unselect <span/);
  assert.doesNotMatch(initial, /aria-label="Include notes\/draft\.md"[^>]*checked/);

  const all = new Set(scanned.entries.map((candidate) => candidate.descriptor.resource_id));
  const selected = render(all);
  assert.match(selected, /Unselect <span class="sync-path-count">8<\/span>/);
  assert.match(selected, /aria-label="Include notes\/draft\.md"[^>]*checked/);
  assert.match(selected, /aria-label="Include scripts\/run\.sh"[^>]*checked/);
  assert.match(selected, /1 needs review/);
  assert.deepEqual(pushReviewBlockers(all, null, scanned).map((blocker) => blocker.state), ["warning_unacknowledged"]);
  assert.notEqual(pushReviewDisabledReason({ selected: all, projectContentInventory: scanned, deletionApproved: true }), null);

  const acknowledged = new Set([flagged.warning_digest]);
  assert.equal(pushReviewDisabledReason({
    selected: all, projectContentInventory: scanned, acknowledgedWarningDigests: acknowledged, deletionApproved: true,
  }), null);
  assert.doesNotMatch(render(all, acknowledged), /1 needs review/);
});

test("Files Select skips blocked, unselectable, held and removed files", () => {
  const blocked = { ...entry("blocked", "file", "blocked.md"), blocked_reason: "Cannot read file" };
  const held: ProjectContentEntry = { ...entry("held", "file", "held.md"), state: "storage_ahead", storage_present: true };
  const removed = entry("removed", "file", "removed.md");
  const unavailable = entry("unavailable", "file", "unavailable.md");
  const included = entry("included", "file", "included.md", false);
  const scanned = { ...inventory, entries: [blocked, held, removed, unavailable, included] };
  const resources = scanned.entries.map((candidate) => ({
    ...projectContentReviewRow(candidate),
    selectable: candidate !== unavailable,
  }));
  const render = (selected: Set<string>) => renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true} resources={resources}
      selected={selected} projectDefaults={new Set(resources.map((resource) => resource.resource_id))} busy={false}
      projectContentInventory={scanned} projectContentScanned showProjectFiles
      projectContentRemovals={new Set(["removed"])} activeStep="project_files"
      onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined}
    />,
  );
  assert.match(render(new Set()), /title="Select all eligible project files \(1\)"/);
  assert.match(render(new Set(["included"])), /Unselect <span class="sync-path-count">1<\/span>/);
});

test("Files clear stays available when only unchanged hidden files are selected", () => {
  const stored: ProjectContentEntry = {
    ...entry("stored", "file", "unchanged.md", false),
    selected_in_recipe: true, storage_present: true, state: "synced",
  };
  const scanned = { ...inventory, entries: [stored, entry("new", "file", "new.md")] };
  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true} resources={scanned.entries.map((row) => projectContentReviewRow(row))}
      selected={new Set(["stored"])} projectDefaults={new Set()} busy={false}
      projectContentInventory={scanned} projectContentScanned projectFilesShown showProjectFiles activeStep="project_files"
      onToggle={() => undefined} onClose={() => undefined} onPush={() => undefined}
    />,
  );
  assert.doesNotMatch(html, /aria-label="Include unchanged\.md"/);
  const clear = html.match(/<button[^>]*aria-label="Clear current selection"[^>]*>/)?.[0];
  assert.ok(clear);
  assert.doesNotMatch(clear, /disabled/);
});

test("a push sends the scan's review token only when it has file work", () => {
  const stored = entry("file-s", "file", "notes/plan.md", false);
  stored.selected_in_recipe = true;
  stored.storage_present = true;
  stored.state = "synced";
  const fresh = entry("file-n", "file", "notes/ideas.md");
  const scanned = (...entries: ProjectContentEntry[]): ProjectContentInventory => ({ ...inventory, entries });
  const none = new Set<string>();

  // Sessions only: the stored file is as storage holds it, the new one waits
  // on a list nobody opened.
  assert.equal(pushProjectFileWork(scanned(stored, fresh), new Set(["file-s"]), none, false), null);
  // Unticking a stored file retains its published copy without capture.
  assert.equal(pushProjectFileWork(scanned(stored), none, none, false), null);
  // On a list the user saw, the new file left unticked is remembered; with
  // nothing new to remember, seeing the list is no work at all.
  assert.equal(pushProjectFileWork(scanned(stored, fresh), new Set(["file-s"]), none, true), "remember");
  assert.equal(pushProjectFileWork(scanned(stored), new Set(["file-s"]), none, true), null);
  assert.equal(pushProjectFileWork(scanned(stored), new Set(["file-s"]), new Set(["file-s"]), false), "send");
  // A new tick changes the recipe.
  assert.equal(pushProjectFileWork(scanned(stored, fresh), new Set(["file-s", "file-n"]), none, false), "send");
  // A stored file edited here travels without a visit to the Files step.
  const edited: ProjectContentEntry = { ...stored, state: "local_ahead" };
  assert.equal(pushProjectFileWork(scanned(edited), new Set(["file-s"]), none, false), "send");
  // A recipe file storage no longer holds: ticked it is pushed again, and
  // unticked the recipe loses it. Either way the backend has to review it.
  const cleared: ProjectContentEntry = { ...stored, storage_present: false, state: "local_only" };
  assert.equal(pushProjectFileWork(scanned(cleared), new Set(["file-s"]), none, false), "send");
  assert.equal(pushProjectFileWork(scanned(cleared), none, none, false), "send");
  // A stored file missing here is not captured: nothing to review.
  const missing: ProjectContentEntry = { ...stored, local_present: false, state: "missing" };
  assert.equal(pushProjectFileWork(scanned(missing), new Set(["file-s"]), none, false), null);
  // A Git-managed folder's files never travel.
  const gitManaged: ProjectContentInventory = { ...scanned(edited), eligibility: { state: "git_managed", reason: "Git manages files." } };
  assert.equal(pushProjectFileWork(gitManaged, new Set(["file-s"]), none, true), null);
});

test("the meter and the refusal count only the files a push would capture", () => {
  const stored = entry("file-s", "file", "notes/plan.md", false);
  stored.selected_in_recipe = true;
  stored.storage_present = true;
  stored.state = "synced";
  stored.size = 4_000;
  const fresh = entry("file-n", "file", "notes/ideas.md");
  fresh.size = 1_000;
  const scanned: ProjectContentInventory = { ...inventory, entries: [stored, fresh] };
  const compared = { capture_budget: { limit_bytes: 10_000, reserved_bytes: 0 }, entries: [] } as unknown as ThreadSyncComparison;
  const files = (selected: string[], filesShown = false) => (
    pushBudgetTotal(new Set(selected), compared, scanned, { filesShown })?.projectFiles
  );

  // The scan ticks the stored file. Storage holds it as it is, so a push
  // without file work captures nothing and the meter counts nothing.
  assert.equal(files(["file-s"]), 0);
  // With file work the backend captures every selected file, changed or not.
  assert.equal(files(["file-s", "file-n"]), 5_000);
  assert.equal(files(["file-s"], true), 4_000);
  assert.equal(files(["file-n"]), 1_000);
  assert.equal(files([], true), 0);

  // A selection of files storage already holds sends nothing.
  const refusal = (selected: string[], projectFilesShown = false) => pushReviewDisabledReason({
    selected: new Set(selected),
    projectContentInventory: scanned,
    deletionApproved: true,
    projectFilesShown,
  });
  assert.equal(refusal(["file-s"]), "Nothing is selected");
  // Remembering a file left out is not a push of its own.
  assert.equal(refusal(["file-s"], true), "Nothing is selected");
  assert.equal(refusal(["file-s", "file-n"]), null);
  assert.equal(refusal(["file-s", "session-a"]), null);
});

test("the folder walk is the review's own wait: Push wears it, and a failed walk says so", () => {
  const button = (filesPending: boolean) => renderToStaticMarkup(
    <PushApplyButton busy={false} filesPending={filesPending} disabledReason={filesPending ? "Project files are still being scanned" : null} onPush={() => undefined} />,
  );
  assert.match(button(true), /icon-spin[\s\S]*Scanning files…<\/button>/);
  assert.match(button(true), /disabled=""[^>]*title="Project files are still being scanned"/);
  assert.match(button(false), />Push<\/button>/);

  const page = (loading: boolean) => renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={[]}
      selectedIds={new Set()}
      scanned={false}
      loading={loading}
      onScan={() => undefined}
      onToggle={() => undefined}
    />,
  );
  assert.match(page(false), /Project files were not scanned\./);
  assert.match(page(false), /Rescan<\/button>/);
  assert.doesNotMatch(page(true), /were not scanned|No changes/);

  // The walk starts with the review, in one place: the Files step no longer
  // launches it, and the review stays usable while it runs.
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const open = sync.slice(sync.indexOf("const openPushChooser = async"), sync.indexOf("const scanProjectFiles = async"));
  assert.match(open, /if \(scope\.kind === "project" && !isGitRepository\) \{\s*void scanProjectFiles\(projectId, storageId, new Set\(\)\);/);
  assert.doesNotMatch(sync, /busy \|\| projectContentLoading/);
  // The scan's token rides only a push that has file work.
  const publish = sync.slice(sync.indexOf("const publishPendingPush = async"), sync.indexOf("const saveRemap = async"));
  assert.match(publish, /pushProjectFileWork\([\s\S]*\) !== null;/);
  assert.match(publish, /reviewsFiles \? filesInventory\.review_token \?\? null : null,\s*reviewsFiles && pendingPush\.projectFilesShown,/);
  const filesPage = readFileSync("src/components/project-sync/ProjectFilesReviewPage.tsx", "utf8");
  assert.doesNotMatch(filesPage, /autoScanRequested|onScan\(\);/);
});

test("Project file controls stay on the dedicated page", () => {
  const selected = new Set(inventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={inventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected}
      projectDefaults={selected}
      busy={false}
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
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={inventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected}
      projectDefaults={selected}
      busy={false}
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
    undecided: candidate.undecided,
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
      onReviewWarnings={() => undefined}
    />,
  );
  // The warning is an acknowledgement: the push waits on its tick, so it is
  // the one message with no close control.
  const ack = html.match(/<label class="notice warning notice-ack">[\s\S]*?<\/label>/)?.[0] ?? "";
  assert.match(ack, /type="checkbox"/);
  assert.doesNotMatch(ack, /sync-notice-dismiss/);
  // The row chip names the finding in the same words as Files activity.
  assert.match(html, /class="notice-chip warning"><svg[\s\S]*?<\/svg>Possible secret<\/span>/);
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
      onReviewWarnings={() => undefined}
    />,
  );
  assert.match(removing, /catalog-card removal/);
  assert.match(removing, />Removes from storage</);
  assert.match(removing, /aria-label="Keep notes\/old_draft\.md in storage"/);
});

test("the Files step's own acknowledgement reviews every listed selected flagged file at once", () => {
  const flagged = (id: string, path: string, code: string) => {
    const candidate = entry(id, "file", path);
    candidate.warning_code = code;
    candidate.warning_digest = id.padEnd(64, "0");
    return candidate;
  };
  const rows = [
    flagged("file-a", "art.py", "possible_secret"),
    flagged("file-b", "film/make_audio.py", "possible_secret"),
    flagged("file-c", "film/voices.toml", "possible_secret"),
    flagged("file-d", "film/unticked.sh", "possible_secret"),
    entry("file-e", "file", "notes/plain.md"),
  ].map((candidate) => ({
    resourceId: candidate.descriptor.resource_id,
    relativePath: candidate.relative_path,
    entryType: candidate.entry_type as "file",
    state: candidate.state,
    localPresent: candidate.local_present,
    storagePresent: candidate.storage_present,
    warningCode: candidate.warning_code,
    warningDigest: candidate.warning_digest,
  }));
  const render = (acknowledged: string[], selected = ["file-a", "file-b", "file-c", "file-e"]) => renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
      rows={rows}
      selectedIds={new Set(selected)}
      acknowledgedWarningDigests={new Set(acknowledged)}
      onToggle={() => undefined}
      onReviewWarnings={() => undefined}
    />,
  );
  const strip = (html: string) => html.match(
    /<label class="notice warning notice-ack sync-project-files-review-all[^"]*">[\s\S]*?<\/label>/,
  )?.[0] ?? "";

  // Unticked: the count and the findings say what the one tick reviews. The
  // flagged file left unticked is not among them, and the rows keep their
  // own boxes under it.
  const none = render([]);
  assert.match(strip(none), /3 selected files flagged<\/strong> — 3 credential-shaped/);
  assert.match(strip(none), /aria-label="Reviewed: 3 selected files flagged — 3 credential-shaped"/);
  assert.match(strip(none), />Mark all reviewed</);
  assert.doesNotMatch(strip(none), /aria-checked|checked=""/);
  assert.equal((none.match(/<label class="notice warning notice-ack/g) ?? []).length, 5);
  assert.ok(none.indexOf("sync-project-files-review-all") < none.indexOf("sync-list-tile"), "heads the list");

  // Some ticked: the box is mixed and still offers the rest.
  const some = strip(render(["file-a".padEnd(64, "0")]));
  assert.match(some, /aria-checked="mixed"/);
  assert.doesNotMatch(some, /checked=""/);
  assert.match(some, />Mark all reviewed</);

  // All ticked: done, in the success tint.
  const all = strip(render(["file-a", "file-b", "file-c"].map((id) => id.padEnd(64, "0"))));
  assert.match(all, /^<label class="notice warning notice-ack sync-project-files-review-all done">/);
  assert.match(all, /checked=""/);
  assert.match(all, />All reviewed</);

  // Nothing flagged is selected: no box to tick.
  assert.equal(strip(render([], ["file-e"])), "");
});

test("Push review lists local-outstanding and unresolved local project files", () => {
  const cases = [
    { id: "local-only", state: "local_only" as const, local: true, storage: false, visible: true },
    { id: "local-ahead", state: "local_ahead" as const, local: true, storage: true, visible: true },
    { id: "synced", state: "synced" as const, local: true, storage: true, visible: false },
    { id: "storage-only", state: "storage_only" as const, local: false, storage: true, visible: false },
    { id: "storage-ahead", state: "storage_ahead" as const, local: true, storage: true, visible: false },
    { id: "diverged", state: "diverged" as const, local: true, storage: true, visible: false },
    { id: "missing", state: "missing" as const, local: false, storage: true, visible: false },
    { id: "blocked", state: "blocked" as const, local: true, storage: false, visible: false },
    { id: "unknown", state: "unknown" as const, local: true, storage: false, visible: true },
    { id: "unknown-stored", state: "unknown" as const, local: true, storage: true, visible: false },
  ];
  const mixedInventory: ProjectContentInventory = {
    ...inventory,
    entries: cases.map((candidate) => ({
      ...entry(`file-${candidate.id}`, "file", `doc/${candidate.id}.md`),
      state: candidate.state,
      local_present: candidate.local,
      storage_present: candidate.storage,
      blocked_reason: candidate.state === "blocked" ? "Unreadable file." : null,
    })),
  };
  const selected = new Set(mixedInventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={mixedInventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={selected}
      projectDefaults={selected}
      busy={false}
      projectContentInventory={mixedInventory}
      projectContentScanned
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );

  assert.equal((html.match(/class="sync-thread-sync-indicator local"/g) ?? []).length, 2);
  assert.equal((html.match(/class="sync-thread-sync-indicator unknown"/g) ?? []).length, 1);
  assert.match(html, /aria-label="3 of 3 selected"/);
  assert.match(html, />3 files</);
  assert.doesNotMatch(html, /aria-label="Remove .* from storage"/);
  const list = html.slice(html.indexOf('<section class="sync-project-files-page"'), html.indexOf('<footer class="sync-inline-action-footer'));
  for (const candidate of cases) {
    const pattern = new RegExp(`doc/${candidate.id}\\.md`);
    if (candidate.visible) assert.match(list, pattern);
    else assert.doesNotMatch(list, pattern);
  }
  // The unresolved local file stops the push, and its own tick is the way out.
  assert.match(list, /<input type="checkbox" aria-label="Include doc\/unknown\.md" checked=""\/>/);
  assert.match(html, />4 project files require Pull</);
});

test("Excluding an unresolved local file and its folder clears the Push refusal", () => {
  const unresolved = (resourceId: string, type: "file" | "directory", path: string) => ({
    ...entry(resourceId, type, path, false),
    state: "unknown" as const,
    selected_in_recipe: true,
    newly_discovered: false,
  });
  const unresolvedInventory: ProjectContentInventory = {
    ...inventory,
    entries: [
      unresolved("dir-doc", "directory", "doc"),
      unresolved("file-plan", "file", "doc/plan.md"),
      entry("file-notes", "file", "notes.md"),
    ],
  };
  const selected = new Set(["dir-doc", "file-plan", "file-notes"]);
  const refusal = pushReviewDisabledReason({
    selected,
    projectContentInventory: unresolvedInventory,
    deletionApproved: true,
  });
  assert.equal(refusal, "2 selected items can’t be captured — exclude them to push.");

  // The pill passes over the folder the file requires and opens the file,
  // then opens the folder once the file is out.
  const listed = new Set(unresolvedInventory.entries.map((candidate) => candidate.descriptor.resource_id));
  const revealTarget = (picked: Set<string>) => pushBlockerRevealTarget(
    pushReviewBlockers(picked, undefined, unresolvedInventory),
    new Set(),
    listed,
    requiredProjectContentDirectoryIds(unresolvedInventory, picked),
  )?.resourceId;
  assert.equal(revealTarget(selected), "file-plan");
  assert.equal(revealTarget(new Set(["dir-doc", "file-notes"])), "dir-doc");

  const render = (picked: Set<string>) => renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={unresolvedInventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={picked}
      projectDefaults={selected}
      busy={false}
      projectContentInventory={unresolvedInventory}
      projectContentScanned
      showProjectFiles
      activeStep="project_files"
      onToggle={() => undefined}
      onClose={() => undefined}
      onPush={() => undefined}
    />,
  );
  // The unknown folder draws a card beside its file. While the file is
  // selected the folder is required; once the file is unticked, the folder's
  // card carries a tick of its own.
  const bothSelected = render(selected);
  assert.match(bothSelected, /data-resource-id="dir-doc"/);
  assert.match(bothSelected, />Required folder</);
  assert.doesNotMatch(bothSelected, /aria-label="Include doc"/);
  const fileExcluded = render(new Set(["dir-doc", "file-notes"]));
  assert.match(fileExcluded, /<input type="checkbox" aria-label="Include doc\/plan\.md"\/>/);
  assert.match(fileExcluded, /<input type="checkbox" aria-label="Include doc" checked=""\/>/);

  assert.equal(
    pushReviewDisabledReason({
      selected: new Set(["file-notes"]),
      projectContentInventory: unresolvedInventory,
      deletionApproved: true,
    }),
    null,
  );
});

test("Push gates storage deletions and Pull omits conflict deletions", () => {
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
    pushReviewBlockers(new Set(), undefined, removalInventory, new Set(), removals).length,
    0,
  );

  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={removalInventory.entries.map((candidate) => projectContentReviewRow(candidate))}
      selected={new Set()}
      projectDefaults={new Set()}
      busy={false}
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
  // Only the operations the commit performs: no zero counts.
  assert.doesNotMatch(html, /<strong>0<\/strong>/);
  assert.match(html, /class="sync-project-file-commit-impact with-approval"/);
  assert.match(html, /<strong>1<\/strong> Delete files/);
  assert.match(html, /<strong>1<\/strong> Delete folders/);
  assert.match(html, /Approve removal of 1 file and 1 folder from storage\./);
  assert.match(html, /title="Approve the selected project file deletions"/);

  const deletionPlan: PullReview = {
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
    deletionItems.map((item) => item.resourceId),
    ["dir-docs", "dir-specs", "file-a"],
  );
  assert.deepEqual(
    pullProjectFileCommitImpact(deletionItems, new Set(["delete-file", "delete-directory"])),
    { add: 0, replace: 0, deleteFile: 0, deleteDirectory: 0 },
  );
});

test("file review details reuse the project copyable popover", () => {
  const html = renderToStaticMarkup(
    <ProjectFilesReviewPage
      mode="push"
      projectRoot="/tmp/project-a"
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
  assert.match(html, /aria-label="Show file info for doc\/test-design\.md"/);
  assert.match(html, /aria-haspopup="dialog"/);
  assert.match(html, /<strong>File info<\/strong>/);
  assert.match(html, /<code>\/tmp\/project-a\/doc\/test-design\.md<\/code>/);
  assert.match(html, /<span>Modified<\/span>/);
  assert.match(html, />Copy all<\/button>/);
  assert.doesNotMatch(html, /sync-project-file-details/);
});

test("opening the files step scans by itself — no interstitial, just the scaffold", () => {
  const html = renderToStaticMarkup(
    <PushResourceWorkspace onSessionActionError={() => undefined}
      credentialRedactionEnabled={true}
      resources={[]}
      selected={new Set()}
      projectDefaults={new Set()}
      busy={false}
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
  assert.match(html, /Rescan/, "the search row's retry survives a failed first scan");
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
  assert.match(html, /No changes/);
  // The shared search row stays (disabled — nothing to filter), counts in the
  // shared noun, and the step's rescan closes it: the review has no path bar.
  assert.match(html, /class="sync-name-search"/);
  assert.match(html, /placeholder="Search files…"[^>]*aria-label="Search files"[^>]*disabled/);
  assert.match(html, /class="sync-name-search-count">0 files</);
  assert.match(html, /class="sync-name-search-results"[\s\S]*Rescan<\/button><\/div>/);
  assert.doesNotMatch(html, /sync-project-files-search|entries|0 selected|No eligible entries|Scanning cannot prove|<h3/);
});

test("the file review's search matches path words and counts the selections it hides", () => {
  const row = (resourceId: string, relativePath: string): ProjectFileReviewRow => ({
    resourceId,
    relativePath,
    entryType: "file",
    state: "local_only",
    localPresent: true,
    storagePresent: false,
  });
  const cards = [
    row("a", "docs/specs/Plan.md"),
    row("b", "docs/readme.md"),
    row("c", "src/plan-notes.txt"),
  ];
  const selected = new Set(["a", "b"]);
  const isSelected = (card: ProjectFileReviewRow) => selected.has(card.resourceId);

  const all = searchProjectFileReviewCards(cards, "  ", isSelected);
  assert.deepEqual(all.matchingCards.map((card) => card.resourceId), ["a", "b", "c"]);
  assert.equal(all.hiddenSelected, 0);

  // Every word must occur somewhere in the path, case-insensitively — the
  // same rule the Files activity page and the session reviews search by.
  const plan = searchProjectFileReviewCards(cards, "PLAN", isSelected);
  assert.deepEqual(plan.matchingCards.map((card) => card.resourceId), ["a", "c"]);
  assert.equal(plan.hiddenSelected, 1, "the selected readme is hidden");

  const specs = searchProjectFileReviewCards(cards, "docs plan", isSelected);
  assert.deepEqual(specs.matchingCards.map((card) => card.resourceId), ["a"]);
  assert.equal(specs.hiddenSelected, 1);

  const none = searchProjectFileReviewCards(cards, "missing", isSelected);
  assert.deepEqual(none.matchingCards, []);
  assert.equal(none.hiddenSelected, 2);
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

const pullPlan: PullReview = {
  apply_limit_bytes: 512 * 1024 * 1024,
  scope: { kind: "project" },
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

test("Pull review omits project content that already matches storage", () => {
  const digest = "c".repeat(64);
  const noOpPlan: PullReview = {
    ...pullPlan,
    plan_id: "plan-project-file-noop",
    actions: [
      {
        action_id: "ensure-same",
        resource_id: "directory-same",
        kind: {
          kind: "ensure_project_directory",
          logical_path: "project/doc",
          mode: 0o755,
          source_mtime: 1,
        },
        target_path: "/tmp/project-b/doc",
        expected_target_mode: 0o755,
        requires_explicit_approval: true,
      },
      {
        action_id: "write-same",
        resource_id: "file-same",
        kind: {
          kind: "write_project_file",
          logical_path: "project/ghost_story.docx",
          mode: 0o644,
          source_mtime: 1,
        },
        target_path: "/tmp/project-b/ghost_story.docx",
        source_sha256: digest,
        materialized_sha256: digest,
        expected_target_sha256: digest,
        expected_target_mode: 0o644,
        requires_explicit_approval: true,
      },
      {
        action_id: "write-new",
        resource_id: "file-new",
        kind: {
          kind: "write_project_file",
          logical_path: "project/cold-joke.md",
          mode: 0o644,
          source_mtime: 1,
        },
        target_path: "/tmp/project-b/cold-joke.md",
        source_sha256: "d".repeat(64),
        materialized_sha256: "d".repeat(64),
        requires_explicit_approval: true,
      },
    ],
  };

  const items = buildPullReviewItems(noOpPlan);
  assert.deepEqual(items.map((item) => item.resourceId), ["file-new"]);
  assert.equal(items[0]?.projectContentOperation, "add");
});

test("Pull counts the files that wait unticked: adds as new, replaces as changed, never a folder", () => {
  const replace: PullReview["actions"][number] = {
    action_id: "write-b",
    resource_id: "file-b",
    kind: { kind: "write_project_file", logical_path: "project/b.md", mode: 0o644, source_mtime: 1 },
    target_path: "/tmp/project-b/b.md",
    source_sha256: "e".repeat(64),
    expected_target_sha256: "f".repeat(64),
    expected_target_mode: 0o644,
    requires_explicit_approval: true,
  };
  const plan: PullReview = { ...pullPlan, actions: [...pullPlan.actions, replace] };
  const items = buildPullReviewItems(plan);
  assert.deepEqual(pullWaitingFiles(items, new Set()), { added: 1, changed: 1 });
  assert.deepEqual(
    pullWaitingFiles(items, includeRequiredPullProjectDirectories(items, new Set(["file-a"]))),
    { added: 0, changed: 1 },
  );
  assert.deepEqual(pullWaitingFiles(items, new Set(["file-a", "file-b"])), { added: 0, changed: 0 });

  const render = (reviewed: PullReview) => renderToStaticMarkup(
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="Project B"
      plan={reviewed}
      binding={binding}
      phase="idle"
      supportLoading={false}
      busy={false}
      error={null}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
      onRefreshPlan={() => undefined}
    />,
  );
  // Both kinds wait: a chip each, one sentence for the button.
  const html = render(plan);
  assert.match(html, /title="2 project files are not in this pull"/);
  assert.match(html, /<span class="sync-path-count">1 new<\/span><span class="sync-path-count">1 changed<\/span>/);

  // A folder that cannot sync has no rows to tick, so nothing waits.
  const ineligible = render({ ...plan, project_content_eligibility: { state: "git_managed", reason: "Inside a Git work tree." } });
  assert.match(ineligible, /title="Review project files next"/);
  assert.doesNotMatch(ineligible, /not in this pull|sync-path-count">\d+ (new|changed)/);
});

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
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="Project B"
      plan={pullPlan}
      binding={binding}
      phase="idle"
      supportLoading={false}
      busy={false}
      error={null}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
      onRefreshPlan={() => undefined}
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
  // The footer carries the step navigator, a box like Push's, counting the
  // file that waits unticked (its folders are not counted).
  assert.match(html, /<button type="button" class="btn" title="1 project file is not in this pull">/);
  assert.match(html, /<span class="sync-path-count">1 new<\/span>/);
  assert.doesNotMatch(html, /changed<\/span>/);

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
  const genericPlan: PullReview = {
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
    <RestorePlanView onSessionActionError={() => undefined}
      projectName="Project B"
      plan={genericPlan}
      binding={binding}
      phase="idle"
      supportLoading={false}
      busy={false}
      error={null}
      onDismissError={() => undefined}
      onApply={() => undefined}
      onBack={() => undefined}
    />,
  );
  assert.match(html, />Add file</);
  assert.match(html, />Replace file</);
});

test("a diverged pull row carries and names its preserved copy", () => {
  const preservedPlan: PullReview = {
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
        source_size: 4_096,
        requires_explicit_approval: true,
      },
    ],
  };
  const items = buildPullReviewItems(preservedPlan);
  assert.equal(items.length, 1);
  assert.equal(items[0].projectContentOperation, "replace");
  assert.equal(items[0].projectContentPreservedCopy, "draft-local-copy.md");
  assert.equal(items[0].projectContentSize, 4_096, "the storage-side size rides the plan into the row");

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
          message: "Kept project files local",
          plan_id: pullPlan.plan_id,
          applied_action_ids: [],
          failed_actions: [],
          preserved_threads: [],
          after_pull_actions: [],
          not_pulled_action_ids: [],
          readiness: { bundle_id: binding.bundle_id, state: "ready", issues: [] },
          readiness_error: null,
        };
      },
    },
    pullPlan,
    { resourceIds: [], restoreActionIds: [] },
    () => undefined,
  );
  assert.deepEqual(calls, ["restore:0"]);
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

/* The Files review in Chinese: the review's own words translate; paths and
   the backend's blocked reason stay exactly as given. */
test("the Files review speaks Chinese around the user's paths", () => {
  const pullRows: ProjectFileReviewRow[] = [
    {
      resourceId: "file-notes",
      relativePath: "notes/outline.md",
      entryType: "file",
      state: "diverged",
      size: 18,
      sourceMtime: 1,
      localPresent: true,
      storagePresent: true,
      operation: "replace",
      preservedCopyPath: "notes/outline (local).md",
    },
    {
      resourceId: "file-readme",
      relativePath: "README.md",
      entryType: "file",
      state: "storage_only",
      size: 18,
      localPresent: false,
      storagePresent: true,
      operation: "add",
    },
  ];
  void i18n.changeLanguage("zh-CN");
  try {
    const pull = renderToStaticMarkup(
      <ProjectFilesReviewPage
        mode="pull"
        projectRoot="/tmp/project-a"
        eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
        rows={pullRows}
        selectedIds={new Set(["file-notes"])}
        onScan={() => undefined}
        onToggle={() => undefined}
      />,
    );
    assert.match(pull, /aria-label="项目文件"/);
    assert.match(pull, /重新扫描<\/button>/);
    assert.match(pull, />替换文件 · 保留本地副本</);
    assert.match(pull, />添加文件</);
    assert.match(pull, /aria-label="本机和存储中都有更改。应用时会先将本地文件保留为 notes\/outline \(local\)\.md，再写入存储中的内容。"/);
    assert.match(pull, /title="排除“notes\/outline\.md”"/);
    assert.match(pull, /aria-label="应用“README\.md”"/);
    assert.match(pull, /aria-label="应用项目根目录中的所有文件"/);
    assert.match(pull, /aria-label="项目根目录中已选 0\/1"/);
    assert.match(pull, /aria-label="排除 notes 中的所有文件"/);
    assert.match(pull, /aria-label="notes 中已选 1\/1"/);
    assert.match(pull, />项目根目录</);
    assert.match(pull, /data-tooltip="存储中 · 18 B"/);

    const blocked = renderToStaticMarkup(
      <ProjectFilesReviewPage
        mode="push"
        eligibility={{ state: "eligible", reason: "Not inside a Git work tree." }}
        rows={[{
          resourceId: "file-key",
          relativePath: "keys/id_rsa",
          entryType: "blocked",
          state: "blocked",
          localPresent: true,
          storagePresent: false,
          blockedReason: "Symlinks are not synced.",
        }]}
        selectedIds={new Set()}
        onToggle={() => undefined}
      />,
    );
    assert.match(blocked, /aria-label="无法同步：Symlinks are not synced\."/);
    assert.match(blocked, />已阻止</);
  } finally {
    void i18n.changeLanguage("en");
  }
});
