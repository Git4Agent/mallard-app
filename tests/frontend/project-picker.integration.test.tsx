import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ProjectPicker from "../../src/components/project-sync/ProjectPicker";
import type { LocalProjectSummary, ProviderProfileSummary, StorageConfig } from "../../src/types";

const storages = [
  { id: "storage-local", name: "Local storage 1", kind: "local" },
  { id: "storage-r2", name: "R2 storage 2", kind: "s3" },
] as unknown as StorageConfig[];

const profiles = [
  { profile_id: "profile-claude", provider: "claude", display_name: "Default Claude" },
  { profile_id: "profile-codex", provider: "codex", display_name: "Default Codex" },
] as unknown as ProviderProfileSummary[];

const projects: LocalProjectSummary[] = [
  {
    local_project_id: "project-a",
    bundle_id: "bundle-a",
    display_name: "mallard-app",
    local_alias: "mallard-app (Claude)",
    revision: 3,
    project_root: "/Users/hequ/Desktop/project/mallard-app",
    profile_ids: { claude: "profile-claude", codex: "profile-codex" },
    linked_storage_ids: ["storage-local"],
    is_git_repository: true,
  },
  {
    local_project_id: "project-b",
    bundle_id: "bundle-b",
    display_name: "testProjectNonGit",
    revision: 1,
    project_root: "/Users/hequ/Desktop/project/testProjectNonGit",
    profile_ids: { codex: "profile-codex" },
    linked_storage_ids: [],
    is_git_repository: false,
  },
];

test("the projects landing lists every registered project as a door built from the registry alone", () => {
  const html = renderToStaticMarkup(
    <ProjectPicker projects={projects} profiles={profiles} storages={storages} onAddProject={() => {}} onSelectProject={() => {}} />,
  );
  // Heading, count, and the one action.
  assert.match(html, /<h1 id="landing-picker-title" class="settings-section-title">Projects<\/h1>/);
  assert.match(html, /2 on this machine/);
  assert.match(html, /class="btn btn-primary"[^>]*>.*Add project…/);
  // One card per project; the alias is the label and the location is compacted.
  assert.equal((html.match(/class="landing-picker-card"/g) ?? []).length, 2);
  assert.match(html, /<strong>mallard-app \(Claude\)<\/strong>/);
  assert.match(html, /~\/Desktop\/project\/mallard-app/);
  assert.match(html, /aria-label="mallard-app \(Claude\), Git repository"/);
  // Provider marks come from the binding's profile ids, not from a scan.
  assert.match(html, /sync-provider-mark claude/);
  assert.match(html, /sync-provider-mark codex/);
  // The linked storage is named; an unlinked project says so, quietly.
  assert.match(html, /Local storage 1/);
  assert.match(html, /landing-picker-card-detail muted"[^>]*>.*No storage linked/);
  // Nothing on the page reads sessions, counts, or stamps.
  assert.doesNotMatch(html, /sessions|to push|Synced|\d{4} at \d{2}:\d{2}/);
});

test("with no projects the landing shows the mark, the fact, and the one action — no explainer", () => {
  const html = renderToStaticMarkup(
    <ProjectPicker projects={[]} profiles={profiles} storages={storages} onAddProject={() => {}} onSelectProject={() => {}} />,
  );
  assert.match(html, /class="landing-picker landing-picker-empty"/);
  assert.match(html, /mallard-logo-duck\.svg/);
  assert.match(html, /No projects yet/);
  assert.match(html, /Add project…/);
  assert.doesNotMatch(html, /Add a folder you run|sessions show up here|keeps them safe/);
  assert.doesNotMatch(html, /landing-picker-card"/);
});

test("the three landings share one picker and the old history copy is gone", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /<ProjectPicker\s/);
  assert.match(workspace, /sync-landing-page/);
  assert.doesNotMatch(workspace, /Project history|Codex history|sync-git-info-page/);
  // Catalog and Storages render the same component; neither keeps a flat
  // index list or an explainer empty state of its own.
  for (const page of ["src/components/storage-catalog/CatalogIndexPage.tsx", "src/components/project-sync/StorageIndexPage.tsx"]) {
    const source = readFileSync(page, "utf8");
    assert.match(source, /<LandingPicker\s/);
    assert.doesNotMatch(source, /sync-storage-index-list|sync-storage-index-row|profile-links-empty|sync-path-sync-label/);
  }
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.landing-picker-card \{[^}]*border: var\(--box-border\);/);
  assert.match(css, /\.landing-picker-grid \{[^}]*repeat\(auto-fill, minmax\(280px, 1fr\)\)/);
});
