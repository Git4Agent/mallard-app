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
import { ProjectFilesActivityView } from "../../src/components/project-sync/ProjectFilesActivityPage";

/* The Files face of a non-git project's activity page: read-only cards in
   the catalog grammar, the storage-only partition on top, and directory
   bands instead of a tree. */

function descriptor(resourceId: string, path: string): ProjectResourceDescriptor {
  return {
    resource_id: resourceId,
    kind: "project_content_file",
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

test("storage-only files lead the list under an Only-in-storage heading", () => {
  const markup = renderView(inventoryWith([
    entry("file-local", "cold-joke.md", "local_only"),
    entry("file-cloud", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
    entry("file-cloud-deep", "archive/interview.md", "storage_only", { local_present: false, storage_present: true }),
  ]));
  // The catalog page's own partition grammar: the shared section heading with
  // the storage name in the running face, and a "Project files" heading over
  // the local half while the partition exists. No counts, no phrase — the
  // rows' own badges carry "Pull to download it here".
  const cloudHeading = markup.indexOf('sync-history-storage-name">Attic R2');
  const localHeading = markup.indexOf("Project files</h2>");
  const cloudFile = markup.indexOf("press-kit.pdf");
  const cloudBand = markup.indexOf("archive/");
  const localFile = markup.indexOf("cold-joke.md");
  assert.match(markup, /sync-history-section-heading/);
  assert.ok(cloudHeading >= 0, "the storage-only partition is named after the storage");
  assert.ok(cloudFile > cloudHeading, "cloud files render inside the partition");
  assert.ok(cloudBand > cloudHeading && cloudBand < localFile, "a cloud-only folder bands inside the partition");
  assert.ok(localHeading > cloudFile && localHeading < localFile, "the local half is headed while the partition exists");
  assert.ok(localFile > cloudFile, "local files follow the partition");
  assert.doesNotMatch(markup, /sync-project-files-cloud-block|sync-project-files-cloud-heading|Pull to download<\/small>/);

  // Alone, the leaf crumb already names the page: no partition, no headings.
  const unpartitioned = renderView(inventoryWith([
    entry("file-local", "cold-joke.md", "local_only"),
  ]));
  assert.doesNotMatch(unpartitioned, /sync-history-section-heading|Project files<\/h2>/);
});

test("the activity list is read-only and directories band instead of nesting", () => {
  const markup = renderView(inventoryWith([
    entry("file-root", "cold-joke.md", "local_only"),
    {
      ...entry("dir-notes", "notes", "synced"),
      entry_type: "directory",
      descriptor: { ...descriptor("dir-notes", "notes"), kind: "project_content_directory" },
    },
    entry("file-notes-a", "notes/outline.md", "synced"),
    entry("file-notes-b", "notes/render.sh", "local_only", { warning_code: "executable", warning_digest: "b".repeat(64) }),
    entry("file-deep", "notes/drafts/chapter1.md", "local_only"),
  ]));
  assert.ok(!markup.includes("checkbox"), "no selection controls on the activity face");
  assert.ok(!markup.includes("Mark reviewed"), "no acknowledgement chips outside a review");
  assert.ok(markup.includes("notes/</strong>"), "a folder that holds files gets a band");
  assert.ok(markup.includes("notes/drafts/</strong>"), "a nested folder bands with its full path");
  assert.ok(markup.includes("2 files"), "the band counts the files directly inside it");
  assert.ok(markup.includes("Executable"), "warning files keep their chip");
  // The directory entry itself draws no card: only the band names it.
  assert.equal(markup.split("sync-history-thread-card").length - 1, 4, "one card per file, none per directory");
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
