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
import type { StatusScope } from "../../src/components/sync-review/statusScopes";
import {
  ProjectFilesActivityView,
  buildProjectFilesActivityProjection,
  fileStatusScope,
  projectFileSyncCounts,
} from "../../src/components/project-sync/ProjectFilesActivityPage";
import { PROJECT_FILE_CARD_RENDER_LIMIT } from "../../src/components/project-sync/model";
import { i18n } from "../../src/i18n";

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
    undecided: false,
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
    file_limit_bytes: 512 * 1024 * 1024,
    warnings: ["Ignore file '.mallardignore' exceeds the metadata limit"],
    scanned_at: 1,
    ...overrides,
  };
}

function renderView(
  inventory: ProjectContentInventory | null,
  loading = false,
  error: string | null = null,
  statusScope?: StatusScope | null,
) {
  return renderToStaticMarkup(
    <ProjectFilesActivityView
      inventory={inventory}
      storageName="Attic R2"
      loading={loading}
      error={error}
      onOpenFile={() => undefined}
      onRevealPath={() => undefined}
      statusScope={statusScope}
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

test("a scan's Pull and Push tallies follow the verbs its rows name", () => {
  const counts = projectFileSyncCounts(inventoryWith([
    // Push sends what is only here or newer here.
    entry("only-here", "draft.md", "local_only"),
    entry("newer-here", "notes/outline.md", "local_ahead"),
    // Pull brings what is only in storage or newer there — a removal in
    // storage included — and what was deleted here while storage kept it.
    entry("only-there", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
    entry("newer-there", "score.mp3", "storage_ahead"),
    entry("removed-there", "retired.md", "storage_ahead", { storage_present: false, base_present: true }),
    entry("gone-here", "doc/test-design.md", "missing", { local_present: false, storage_present: true, base_present: true }),
    // Neither verb claims a file waiting on the reader's choice, a settled
    // one, an unknown one, or one that cannot travel — even a new one.
    entry("both-sides", "both.md", "diverged"),
    entry("same", "same.md", "synced"),
    entry("unknown", "unknown.md", "unknown"),
    entry("blocked", "poster.psd", "blocked", {
      entry_type: "blocked",
      blocked_reason: "File exceeds the 100 MiB project-content limit",
      storage_present: false,
    }),
    entry("blocked-new", "render.mov", "local_only", { blocked_reason: "File exceeds the 100 MiB project-content limit" }),
    // Directory rows are never drawn, so they are never counted.
    {
      ...entry("dir-assets", "assets", "storage_only", { local_present: false, storage_present: true }),
      entry_type: "directory",
      descriptor: { ...descriptor("dir-assets", "assets"), kind: { key: "project_content_directory", version: 1 } },
    },
  ]));
  assert.deepEqual(counts, { pull: 4, push: 2 });

  // No scan yet, or a folder whose files cannot travel, claims nothing.
  assert.deepEqual(projectFileSyncCounts(null), { pull: 0, push: 0 });
  assert.deepEqual(
    projectFileSyncCounts(inventoryWith([entry("file-a", "a.md", "local_only")], {
      eligibility: { state: "git_managed", reason: "Git manages files in this project folder." },
    })),
    { pull: 0, push: 0 },
  );
});

test("the workspace holds the one file scan the Files face and the path bar read", () => {
  const files = readFileSync("src/components/project-sync/ProjectFilesActivityPage.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  // The face is a pure view of the scan; it neither scans nor reports a
  // header of its own.
  assert.doesNotMatch(files, /export default function|onHeaderModelChange|FilesActivityHeaderModel/);
  assert.equal((files.match(/projectSyncApi\.inspectProjectFiles\(/g) ?? []).length, 1);
  assert.match(files, /export function useProjectFilesScan\(/);
  assert.equal((workspace.match(/useProjectFilesScan\(/g) ?? []).length, 1);
  assert.match(workspace, /<ProjectFilesActivityView\s+inventory=\{projectFilesScan\.inventory\}/);
  // A Git project has no Files face to scan for, and a review owns the
  // project and the host lock while it is open; the scan reruns on its close.
  assert.match(
    workspace,
    /useProjectFilesScan\(\s*activeProject && !activeProject\.is_git_repository && !newProjectSetup && !inlineStorageReview/,
  );
  // A result is read only for the pair it was scanned for, so one project's
  // files never count on another's bar while its own scan is in flight.
  assert.match(files, /const current = scanned\?\.key === key \? scanned : null;/);
  assert.match(files, /loading: key !== null && scanningKey === key,/);
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
    new RegExp(`First ${PROJECT_FILE_CARD_RENDER_LIMIT} of ${files.length} files shown\\. Search or choose a status to narrow the list\\.`),
  );

  const completeProjection = buildProjectFilesActivityProjection(files, "", null);
  const completeVisibleCount = completeProjection.cloudEntries.length
    + completeProjection.localGroups.reduce((sum, group) => sum + group.entries.length, 0);
  assert.equal(completeProjection.matchingCount, files.length);
  assert.equal(completeVisibleCount, PROJECT_FILE_CARD_RENDER_LIMIT);
  assert.ok(completeProjection.cloudEntries.length > 0, "the bounded result retains cloud rows");
  assert.ok(completeProjection.localGroups.length > 0, "the bounded result retains local groups");

  const narrowedProjection = buildProjectFilesActivityProjection(files, "needle-after-limit", null);
  const narrowedEntries = [
    ...narrowedProjection.cloudEntries,
    ...narrowedProjection.localGroups.flatMap((group) => group.entries),
  ];
  assert.equal(narrowedProjection.matchingCount, 1, "search runs before the render cap");
  assert.equal(narrowedEntries[0]?.relative_path, "deep/needle-after-limit.md");

  // A status scope runs before the cap too: To pull reaches the needle, the
  // last file of the inventory, and To push leaves it out.
  const pullProjection = buildProjectFilesActivityProjection(files, "", "pull");
  const pullEntries = [
    ...pullProjection.cloudEntries,
    ...pullProjection.localGroups.flatMap((group) => group.entries),
  ];
  assert.equal(pullProjection.matchingCount, Math.ceil(files.length / 2));
  assert.ok(pullEntries.some((candidate) => candidate.relative_path === "deep/needle-after-limit.md"));
  assert.equal(buildProjectFilesActivityProjection(files, "needle-after-limit", "push").matchingCount, 0);

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
  const cloudTile = markup.indexOf("sync-list-tile cloud");
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
  assert.doesNotMatch(unpartitioned, /sync-list-tile cloud|sync-history-section-heading/);
  assert.match(unpartitioned, />Project root</);
});

test("the Files face wears the Sessions face's chrome", () => {
  const markup = renderView(inventoryWith([
    entry("file-root", "cold-joke.md", "local_only", { source_mtime: 1_752_800_000 }),
    entry("file-notes", "notes/outline.md", "synced"),
  ]));

  // One search row: the Sessions face's field, its count naming what it counts.
  assert.match(markup, /<div class="sync-name-search">/);
  assert.match(markup, /aria-label="Search files"/);
  assert.match(markup, /class="sync-name-search-count">2 files</);
  assert.doesNotMatch(markup, /sync-project-files-search|entries|Rescan/);
  // One refresh: the path bar's glyph already rescans this face.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /onRefresh: projectFilesScan\.rescan,/);
  assert.doesNotMatch(workspace, /onRescan=/);

  // One info glyph and one readout family: the session rows' own square ⓘ,
  // and Kind and Size in the same 14px row glyphs as messages and tokens.
  assert.match(markup, /sync-history-debug-trigger[^>]*>[^<]*<svg[^>]*data-icon="info" class="session-row-icon"/);
  assert.match(markup, /<svg[^>]*data-icon="file" class="session-row-icon"/);
  assert.match(markup, /<svg[^>]*data-icon="size" class="session-row-icon"/);
  assert.doesNotMatch(markup, /data-icon="drive"/);

  // One stamp: the Sessions face's formatter, closing the row.
  assert.match(markup, /<time dateTime="2025-07-18T[^"]+">[^<]+ at [^<]+<\/time>/);

  // A non-Git project's root is a plain folder, and a file shows in Finder.
  assert.doesNotMatch(markup, /data-icon="git-folder"/);
  assert.match(markup, /title="Show cold-joke\.md in Finder"/);
  assert.doesNotMatch(markup, /Reveal/);
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
    entry("file-notes-b", "notes/render.sh", "local_only", { warning_code: "possible_secret", warning_digest: "b".repeat(64) }),
    entry("file-deep", "notes/drafts/chapter1.md", "local_only"),
  ]));
  assert.ok(!markup.includes("checkbox"), "no selection controls on the activity face");
  assert.ok(!markup.includes("Mark reviewed"), "no acknowledgement chips outside a review");
  // The head's label is pixel-face chrome; the path inside it is the user's
  // data and rides the sans carve-out span, like the storage name beside it.
  assert.ok(markup.includes("<span>notes/</span></strong>"), "a folder that holds files heads its own tile");
  assert.ok(markup.includes("<span>notes/drafts/</span></strong>"), "a nested folder tiles with its full path");
  assert.ok(markup.includes("2 files"), "the tile head counts the files directly inside it");
  assert.ok(markup.includes("Possible secret"), "warning files keep their chip");
  // The directory entry itself draws no card: only its tile head names it.
  assert.equal(markup.split("sync-history-thread-card").length - 1, 4, "one two-row card per file, none per directory");
  assert.match(markup, /Kind · Markdown|>Markdown</, "the readout line carries the kind chip");
});

test("file details reuse the project copyable popover", () => {
  const markup = renderView(inventoryWith([
    entry("file-design", "doc/test-design.md", "synced"),
  ]));
  assert.match(markup, /aria-label="Show file info for doc\/test-design\.md"/);
  assert.match(markup, /aria-haspopup="dialog"/);
  const controlledPanel = markup.match(/aria-controls="([^"]+)"/)?.[1];
  assert.ok(controlledPanel);
  assert.match(markup, new RegExp(`id="${controlledPanel}"`));
  assert.match(markup, /<strong>File info<\/strong>/);
  assert.doesNotMatch(markup, /Click a value to copy it\./);
  assert.match(markup, /<code>\/tmp\/project-a\/doc\/test-design\.md<\/code>/);
  assert.match(markup, />Copy all<\/button>/);
  assert.doesNotMatch(markup, /sync-project-file-details/);
});

test("blocked and storage-only rows say why they cannot act", () => {
  const markup = renderView(inventoryWith([
    entry("file-blocked", "poster.psd", "blocked", {
      entry_type: "blocked",
      blocked_reason: "File exceeds the 100 MiB project-content limit",
      storage_present: false,
    }),
    entry("file-cloud", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
  ]));
  assert.ok(markup.includes("Blocked"), "a blocked row carries its chip");
  assert.ok(markup.includes("File exceeds the 100 MiB project-content limit"), "the reason travels with the row");
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

test("the project's view switch leads the Files face in every state", () => {
  const viewSwitch = <div className="sync-project-view-tabs" role="tablist" aria-label="Project view" />;
  const render = (inventory: ProjectContentInventory | null, loading = false, error: string | null = null) => (
    renderToStaticMarkup(
      <ProjectFilesActivityView
        inventory={inventory}
        storageName="Attic R2"
        loading={loading}
        error={error}
        viewSwitch={viewSwitch}
      />,
    )
  );
  const lead = /^<section class="sync-project-files-page" aria-label="Project files">/;

  // With files to search, the pill rides the search row, ahead of the field
  // and outside its rule.
  const listed = render(inventoryWith([entry("file-a", "a.md", "local_only")]));
  assert.match(listed, new RegExp(`${lead.source}<div class="sync-list-head"><div class="sync-project-view-tabs"[^>]*></div><div class="sync-name-search">`));

  // Every other state still opens with it, on a row of its own, so the switch
  // never moves between the faces or while a scan lands.
  for (const [label, html] of [
    ["scanning", render(null, true)],
    ["unlinked", render(null)],
    ["failed", render(null, false, "scan failed: folder unreadable")],
    ["locked", render(inventoryWith([], { eligibility: { state: "git_managed", reason: "Git manages files in this project folder." } }))],
    ["empty", render(inventoryWith([]))],
  ] as const) {
    assert.match(html, new RegExp(`${lead.source}<div class="sync-project-view-row"><div class="sync-project-view-tabs"`), label);
  }

  // The review renders the same page with no switch.
  assert.doesNotMatch(renderView(inventoryWith([entry("file-a", "a.md", "local_only")])), /sync-project-view/);
});

/* One of each scope's files, in three folders and the cloud partition. */
function scopedInventory(): ProjectContentInventory {
  return inventoryWith([
    entry("only-here", "draft.md", "local_only"),
    entry("newer-here", "notes/outline.md", "local_ahead"),
    entry("only-there", "press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
    entry("gone-here", "doc/test-design.md", "missing", { local_present: false, storage_present: true, base_present: true }),
    entry("newer-there", "notes/score.mp3", "storage_ahead"),
    entry("both-sides", "notes/both.md", "diverged"),
    entry("blocked", "poster.psd", "blocked", {
      entry_type: "blocked",
      blocked_reason: "File exceeds the 100 MiB project-content limit",
      storage_present: false,
    }),
    entry("same", "same.md", "synced"),
  ]);
}

const scopeStrip = (html: string) => html.match(/<div class="sync-project-tabs sync-status-scopes"[\s\S]*?<\/div>/)?.[0] ?? "";
const scopeRadios = (html: string) => [...scopeStrip(html).matchAll(/role="radio" aria-checked="(true|false)" aria-label="([^"]+)"/g)]
  .map(([, checked, label]) => (checked === "true" ? `${label} ✓` : label));
const showsFile = (html: string, path: string) => (
  html.includes(`title="Open ${path}"`) || html.includes(`<strong title="${path}">`)
);

test("the Files face carries the Sessions face's scope bar, counted from the file badges", () => {
  const inventory = scopedInventory();
  const markup = renderView(inventory);

  // The strip follows the search row, named for what it narrows, with the
  // Sessions face's words and a count on every scope.
  assert.match(scopeStrip(markup), /role="radiogroup" aria-label="Show files by status"/);
  assert.ok(markup.indexOf("sync-name-search") < markup.indexOf("sync-status-scopes"));
  assert.ok(markup.indexOf("sync-status-scopes") < markup.indexOf("sync-history-thread-card"));
  assert.deepEqual(scopeRadios(markup), [
    "All, 8 files ✓",
    "To push, 2 files",
    "To pull, 3 files",
    "Attention, 2 files",
    "Up to date, 1 file",
  ]);

  // The path bar's file tallies read the same mapping, so the strip's two
  // verbs equal its numbers — a deleted-here file counted under Pull.
  const counts = projectFileSyncCounts(inventory);
  const scopeCount = (scope: StatusScope) => inventory.entries.filter((candidate) => fileStatusScope(candidate) === scope).length;
  assert.deepEqual(counts, { pull: scopeCount("pull"), push: scopeCount("push") });
  assert.equal(fileStatusScope(inventory.entries.find((candidate) => candidate.state === "missing")!), "pull");
  assert.equal(fileStatusScope(inventory.entries.find((candidate) => candidate.state === "blocked")!), "attention");

  // One status across every file narrows nothing, so there is no strip.
  const settled = renderView(inventoryWith([entry("a", "a.md", "synced"), entry("b", "notes/b.md", "synced")]));
  assert.doesNotMatch(settled, /sync-status-scopes/);
});

test("a status scope narrows the files, keeps their folders, and reads like a search", () => {
  const pull = renderView(scopedInventory(), false, null, "pull");
  assert.ok(scopeRadios(pull).includes("To pull, 3 files ✓"));
  for (const path of ["press-kit.pdf", "doc/test-design.md", "notes/score.mp3"]) {
    assert.ok(showsFile(pull, path), `${path} is a file to pull`);
  }
  for (const path of ["draft.md", "notes/outline.md", "notes/both.md", "poster.psd", "same.md"]) {
    assert.ok(!showsFile(pull, path), `${path} is not a file to pull`);
  }
  // The cloud partition still leads; a folder with nothing to pull leaves.
  assert.match(pull, /sync-list-tile cloud/);
  assert.ok(pull.includes("<span>notes/</span></strong>"));
  assert.doesNotMatch(pull, />Project root</);
  // The count reads as it does under a search.
  assert.match(pull, /class="sync-name-search-count"><b>3<\/b><span> of 8 files<\/span>/);

  const attention = renderView(scopedInventory(), false, null, "attention");
  assert.ok(showsFile(attention, "notes/both.md"));
  assert.ok(showsFile(attention, "poster.psd"));
  assert.ok(!showsFile(attention, "doc/test-design.md"), "a deleted-here file waits on Pull, not on a person");
});

test("a chosen scope with no files stays at 0 and offers Clear filter", () => {
  const settledPush = inventoryWith([
    entry("a", "a.md", "synced"),
    entry("b", "notes/b.md", "storage_only", { local_present: false, storage_present: true }),
  ]);
  const emptied = renderView(settledPush, false, null, "push");
  assert.ok(scopeRadios(emptied).includes("To push, 0 files ✓"));
  assert.match(emptied, /No matching files\.<button type="button" class="btn btn-ghost">Clear filter<\/button>/);
  assert.doesNotMatch(emptied, /sync-history-thread-card/);

  // Nothing to scope, no strip: a scope chosen before the files went away is
  // not applied to an empty folder.
  const empty = renderView(inventoryWith([]), false, null, "push");
  assert.doesNotMatch(empty, /sync-status-scopes|Clear filter/);
  assert.match(empty, /No files/);

  // The strip sits flush under the field whose rule it takes over; the
  // page's own gap spaces it from the list.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-project-files-page > \.sync-name-search,\n\.sync-project-files-page > \.sync-list-head \{ margin-bottom: 0; \}/);
  assert.match(css, /\.sync-project-files-page > \.sync-status-scopes\.sync-project-tabs \{ margin: -12px 0 0; \}/);
});

/* The Files face in Chinese: the face's own words translate; paths, the
   storage's name and the scanner's warning text stay exactly as given. */
test("the Files face speaks Chinese around the user's paths and storage name", () => {
  void i18n.changeLanguage("zh-CN");
  try {
    const markup = renderView(inventoryWith([
      entry("file-root", "cold-joke.md", "local_only"),
      entry("file-notes", "notes/outline.md", "synced"),
      entry("file-cloud", "archive/press-kit.pdf", "storage_only", { local_present: false, storage_present: true }),
    ]));
    assert.match(markup, /<section class="sync-project-files-page" aria-label="项目文件">/);
    assert.match(markup, /aria-label="搜索文件"/);
    assert.match(markup, /class="sync-name-search-count">3 个文件</);
    assert.match(markup, /aria-label="按状态显示文件"/);
    // The partitions: the storage-only tile names the storage, the local
    // half by folder, each folder's facts counted in Chinese.
    assert.match(markup, /仅在 <span class="sync-history-storage-name">Attic R2<\/span> 中/);
    assert.match(markup, />项目根目录</);
    assert.match(markup, /<span>notes\/<\/span>/);
    assert.match(markup, /1 个文件 · 18 B/);
    // Rows: their status against the storage by name, their readouts and actions.
    assert.match(markup, /aria-label="仅在本机。上传后保存到 Attic R2。"/);
    assert.match(markup, /aria-label="本机与 Attic R2 均为最新。"/);
    assert.match(markup, /aria-label="仅在 Attic R2 中。下载后才会出现在本机。"/);
    assert.match(markup, /title="打开 cold-joke\.md"/);
    assert.match(markup, /aria-label="在访达中显示 cold-joke\.md"/);
    assert.match(markup, /aria-label="无法显示 archive\/press-kit\.pdf：不在这台 Mac 上"/);
    assert.match(markup, /data-tooltip="磁盘上 · 18 B"/);
    assert.match(markup, /data-tooltip="Attic R2 中 · 18 B"/);
    assert.match(markup, /aria-label="显示 cold-joke\.md 的文件信息"/);
    // The scan's summary is the face's; the warning itself is the scanner's.
    assert.match(markup, /1 个已忽略 · 1 个已阻止 · 1 个扫描警告/);
    assert.match(markup, /<p>Ignore file &#x27;\.mallardignore&#x27; exceeds the metadata limit<\/p>/);
    assert.doesNotMatch(markup, /Project root|Search files|Only in|Up to date|scan warning/);

    // The empty and unlinked states.
    assert.match(renderView(inventoryWith([])), /<strong>没有文件<\/strong><small>此文件夹中还没有要同步的内容。<\/small>/);
    assert.match(renderView(null), /关联一个存储，即可扫描此文件夹并比较其中的文件。/);
  } finally {
    void i18n.changeLanguage("en");
  }
});
