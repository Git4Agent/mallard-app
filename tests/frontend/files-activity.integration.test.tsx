import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  ProjectContentEntry,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  ResourceSyncState,
} from "../../src/types";
import {
  ProjectFilesActivityView,
  buildProjectFilesActivityProjection,
} from "../../src/components/project-sync/ProjectFilesActivityPage";
import { PROJECT_FILE_CARD_RENDER_LIMIT } from "../../src/components/project-sync/model";

/* The Files face of a non-git project's activity page: read-only cards in
   the catalog grammar, the storage-only partition on top, and directory
   bands instead of a tree. */

/* A scanned entry carries the bare portable descriptor, as the backend's
   `ProjectContentEntry` does; the Files page reads the entry, not a label. */
function descriptor(resourceId: string, path: string): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: { key: "project_content_file", version: 1 },
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
  path: string,
  state: ResourceSyncState,
  overrides: Partial<ProjectContentEntry> = {},
): ProjectContentEntry {
  return {
    descriptor: descriptor(resourceId, path),
    entry_type: "file",
    relative_path: path,
    logical_path: `project/${path}`,
    size: 18,
    mode: 0o644,
    source_mtime: 1,
    state,
    local_present: true,
    storage_present: state === "synced" || state === "storage_ahead" || state === "storage_only" || state === "diverged",
    base_present: false,
    local_digest: "a".repeat(64),
    storage_digest: null,
    base_digest: null,
    review_digest: "a".repeat(64),
    selected_in_recipe: false,
    newly_discovered: true,
    selected_after_scan: false,
    blocked_reason: null,
    warning_code: null,
    warning_digest: null,
    ...overrides,
  };
}

function inventoryWith(entries: ProjectContentEntry[], overrides: Partial<ProjectContentInventory> = {}): ProjectContentInventory {
  return {
    local_project_id: "project-a",
    storage_id: "storage-a",
    project_root: "/tmp/project-a",
    eligibility: { state: "eligible", reason: "Not inside a Git work tree." },
    review_token: null,
    storage_revision: null,
    preference_revision: 0,
    excluded_resource_ids: [],
    entries,
    ignored_count: 1,
    blocked_count: 1,
    warnings: ["Ignore file '.mallardignore' exceeds the metadata limit"],
    scanned_at: 1,
    ...overrides,
  };
}

function renderView(inventory: ProjectContentInventory | null, loading = false, error: string | null = null) {
  return renderToStaticMarkup(
    <ProjectFilesActivityView
      inventory={inventory}
      storageName="Attic R2"
      loading={loading}
      error={error}
      onRescan={() => undefined}
      onOpenFile={() => undefined}
      onRevealPath={() => undefined}
    />,
  );
}

test("file inspections share concurrent native work", () => {
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const inspectProjectFiles = api.slice(
    api.indexOf("inspectProjectFiles:"),
    api.indexOf("pushBundle:", api.indexOf("inspectProjectFiles:")),
  );

  assert.match(inspectProjectFiles, /projectSyncSingleFlight/);
  assert.match(inspectProjectFiles, /\["inspect_project_files", request\]/);
});

test("the activity list searches the full inventory but renders at most 500 file cards", () => {
  const files = Array.from(
    { length: PROJECT_FILE_CARD_RENDER_LIMIT + 7 },
    (_, index) => {
      const cloudOnly = index % 2 === 0;
      const path = index === PROJECT_FILE_CARD_RENDER_LIMIT + 6
        ? "deep/needle-after-limit.md"
        : `folder-${index % 12}/file-${String(index).padStart(4, "0")}.md`;
      return entry(
        `file-${index}`,
        path,
        cloudOnly ? "storage_only" : "local_only",
        cloudOnly ? { local_present: false, storage_present: true } : {},
      );
    },
  );
  const directories = Array.from({ length: 20 }, (_, index) => ({
    ...entry(`directory-${index}`, `folder-${index}`, "synced"),
    entry_type: "directory" as const,
    descriptor: {
      ...descriptor(`directory-${index}`, `folder-${index}`),
      kind: { key: "project_content_directory", version: 1 },
    },
  }));

  const markup = renderView(inventoryWith([...directories, ...files]));
  assert.equal(
    markup.split("sync-history-thread-card").length - 1,
    PROJECT_FILE_CARD_RENDER_LIMIT,
    "cloud and local groups share one file-card budget",
  );
  assert.match(markup, /sync-project-file-limit/);
  assert.match(
    markup,
    new RegExp(`First ${PROJECT_FILE_CARD_RENDER_LIMIT} of ${files.length} files shown\\. Search to narrow the list\\.`),
  );

  const completeProjection = buildProjectFilesActivityProjection(files, "");
  const completeVisibleCount = completeProjection.cloudEntries.length
    + completeProjection.localGroups.reduce((sum, group) => sum + group.entries.length, 0);
  assert.equal(completeProjection.matchingCount, files.length);
  assert.equal(completeVisibleCount, PROJECT_FILE_CARD_RENDER_LIMIT);
  assert.ok(completeProjection.cloudEntries.length > 0, "the bounded result retains cloud rows");
  assert.ok(completeProjection.localGroups.length > 0, "the bounded result retains local groups");

  const narrowedProjection = buildProjectFilesActivityProjection(files, "needle-after-limit");
  const narrowedEntries = [
    ...narrowedProjection.cloudEntries,
    ...narrowedProjection.localGroups.flatMap((group) => group.entries),
  ];
  assert.equal(narrowedProjection.matchingCount, 1, "search runs before the render cap");
  assert.equal(narrowedEntries[0]?.relative_path, "deep/needle-after-limit.md");

  const source = readFileSync("src/components/project-sync/ProjectFilesActivityPage.tsx", "utf8");
  assert.match(source, /useDeferredValue\(search\)/, "large-inventory search uses deferred input");
});

test("storage-only files lead the list under an Only-in-storage heading", () => {
  const markup = renderView(inventoryWith([
    entry("file-local", "cold-joke.md", "local_only"),
    entry("file-cloud", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
    entry("file-cloud-deep", "archive/interview.md", "storage_only", { local_present: false, storage_present: true }),
  ]));
  // The ledger grammar: the storage-only partition is one cloud-tinted tile
  // leading the page, its head naming the storage in the running face — no
  // counts, no phrase; the rows' own badges carry "Pull to download it here".
  // Nested cloud files keep their directory as a quiet prefix inside it, and
  // the local half follows as framed tiles headed "Project root" and by path.
  const cloudTile = markup.indexOf("sync-project-file-tile cloud");
  const cloudName = markup.indexOf('sync-history-storage-name">Attic R2');
  const cloudFile = markup.indexOf("press-kit.pdf");
  const cloudPrefix = markup.indexOf("archive/");
  const rootTile = markup.indexOf("Project root");
  const localFile = markup.indexOf("cold-joke.md");
  assert.ok(cloudTile >= 0, "the storage-only partition is a cloud-tinted tile");
  assert.ok(cloudName > cloudTile, "the tile head names the storage");
  assert.ok(cloudFile > cloudName, "cloud files render inside the tile");
  assert.ok(cloudPrefix > cloudName && cloudPrefix < rootTile, "a nested cloud file keeps its path prefix inside the tile");
  assert.ok(rootTile > cloudFile, "the local half follows as its own tile");
  assert.ok(localFile > rootTile, "local files render inside the Project root tile");
  assert.match(markup, /class="sync-history-session-title-button" title="Open cold-joke\.md"/);
  assert.doesNotMatch(markup, /title="Open press-kit\.pdf"/);
  assert.doesNotMatch(markup, /sync-history-section-heading|Project files<\/h2>|Pull to download<\/small>/);

  // Alone, the local tiles stand without any cloud partition.
  const unpartitioned = renderView(inventoryWith([
    entry("file-local", "cold-joke.md", "local_only"),
  ]));
  assert.doesNotMatch(unpartitioned, /sync-project-file-tile cloud|sync-history-section-heading/);
  assert.match(unpartitioned, />Project root</);
  assert.match(unpartitioned, / Rescan</, "the bar carries its own rescan beside the tally");
});

test("missing local files remain actionable storage copies", () => {
  const markup = renderView(inventoryWith([
    entry("file-missing", "doc/test-design.md", "missing", {
      local_present: false,
      storage_present: true,
      base_present: true,
      local_digest: null,
      storage_digest: "b".repeat(64),
      base_digest: "b".repeat(64),
    }),
  ]));

  assert.match(markup, /sync-history-storage-name">Attic R2/);
  assert.match(markup, /data-thread-status="cloud-outstanding"/);
  assert.match(
    markup,
    /Not in this folder\. It remains in Attic R2\. Pull to download it here, or remove it during Push\./,
  );
  assert.doesNotMatch(markup, /data-thread-status="unavailable"/);
});

test("the activity list is read-only and directories band instead of nesting", () => {
  const markup = renderView(inventoryWith([
    entry("file-root", "cold-joke.md", "local_only"),
    {
      ...entry("dir-notes", "notes", "synced"),
      entry_type: "directory",
      descriptor: { ...descriptor("dir-notes", "notes"), kind: { key: "project_content_directory", version: 1 } },
    },
    entry("file-notes-a", "notes/outline.md", "synced"),
    entry("file-notes-b", "notes/render.sh", "local_only", { warning_code: "executable", warning_digest: "b".repeat(64) }),
    entry("file-deep", "notes/drafts/chapter1.md", "local_only"),
  ]));
  assert.ok(!markup.includes("checkbox"), "no selection controls on the activity face");
  assert.ok(!markup.includes("Mark reviewed"), "no acknowledgement chips outside a review");
  // The head's label is pixel-face chrome; the path inside it is the user's
  // data and rides the sans carve-out span, like the storage name beside it.
  assert.ok(markup.includes("<span>notes/</span></strong>"), "a folder that holds files heads its own tile");
  assert.ok(markup.includes("<span>notes/drafts/</span></strong>"), "a nested folder tiles with its full path");
  assert.ok(markup.includes("2 files"), "the tile head counts the files directly inside it");
  assert.ok(markup.includes("Executable"), "warning files keep their chip");
  // The directory entry itself draws no card: only its tile head names it.
  assert.equal(markup.split("sync-history-thread-card").length - 1, 4, "one two-row card per file, none per directory");
  assert.match(markup, /Kind · Markdown|>Markdown</, "the readout line carries the kind chip");
});

test("file details reuse the project copyable popover", () => {
  const markup = renderView(inventoryWith([
    entry("file-design", "doc/test-design.md", "synced"),
  ]));
  assert.match(markup, /aria-label="Show details for doc\/test-design\.md"/);
  assert.match(markup, /aria-haspopup="dialog"/);
  const controlledPanel = markup.match(/aria-controls="([^"]+)"/)?.[1];
  assert.ok(controlledPanel);
  assert.match(markup, new RegExp(`id="${controlledPanel}"`));
  assert.match(markup, /<strong>File details<\/strong>/);
  assert.match(markup, /Click a value to copy it\./);
  assert.match(markup, /<code>\/tmp\/project-a\/doc\/test-design\.md<\/code>/);
  assert.match(markup, />Copy all<\/button>/);
  assert.doesNotMatch(markup, /sync-project-file-details/);
});

test("blocked and storage-only rows say why they cannot act", () => {
  const markup = renderView(inventoryWith([
    entry("file-blocked", "poster.psd", "blocked", {
      entry_type: "blocked",
      blocked_reason: "File exceeds the 16 MiB project-content limit",
      storage_present: false,
    }),
    entry("file-cloud", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
  ]));
  assert.ok(markup.includes("Blocked"), "a blocked row carries its chip");
  assert.ok(markup.includes("File exceeds the 16 MiB project-content limit"), "the reason travels with the row");
  assert.ok(markup.includes("Not on this Mac"), "a cloud-only row's reveal is disabled with the reason");
  assert.ok(markup.includes("Only in Attic R2. Pull to download it here."), "the cloud row states its status against the storage by name");
});

test("ineligible scans, missing storage, and scan problems each speak", () => {
  const locked = renderView(inventoryWith([], {
    eligibility: { state: "git_managed", reason: "Git manages files in this project folder." },
  }));
  assert.ok(locked.includes("Project files are locked."), "a git-managed folder shows the locked notice");

  const unlinked = renderView(null, false);
  assert.ok(unlinked.includes("Link a storage"), "no storage means no comparison, said plainly");

  const failed = renderView(null, false, "scan failed: folder unreadable");
  assert.ok(failed.includes("scan failed: folder unreadable"), "a failed scan interrupts with the error");

  const summary = renderView(inventoryWith([entry("file-a", "a.md", "local_only")]));
  assert.ok(summary.includes("1 ignored · 1 blocked · 1 scan warning"), "the scan summary keeps its counts");
});
