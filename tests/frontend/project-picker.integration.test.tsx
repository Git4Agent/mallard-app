import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ProjectPicker from "../../src/components/project-sync/ProjectPicker";
import type { LocalProjectSummary, ProviderDescriptor, ProviderProfileSummary, StorageSummary } from "../../src/types";

const storages = [
  { storage_id: "storage-local", name: "Local storage 1", provider: "local", provider_label: "Local folder", icon: "drive", available: true, summary: [] },
  { storage_id: "storage-cloud", name: "Cloud storage 2", provider: "cloud-a", provider_label: "Cloud A", icon: "cloud", available: true, summary: [] },
] as unknown as StorageSummary[];

const profiles = [
  { profile_id: "profile-claude", provider: "claude", display_name: "Default Claude" },
  { profile_id: "profile-codex", provider: "codex", display_name: "Default Codex" },
] as unknown as ProviderProfileSummary[];

const providers = [
  { key: "codex", display_name: "Codex", display_rank: 0, profile_label: "Codex home", setup_default: true },
  { key: "claude", display_name: "Claude", display_rank: 1, profile_label: "Claude config", setup_default: true },
] as unknown as ProviderDescriptor[];

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
    <ProjectPicker projects={projects} profiles={profiles} providers={providers} storages={storages} onAddProject={() => {}} onSelectProject={() => {}} />,
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
  // And nothing of the empty shelf survives once a card exists.
  assert.doesNotMatch(html, /landing-picker-door|landing-picker-caption/);
});

test("with no projects the landing is the filled page ghosted: the head, one door, one line", () => {
  const html = renderToStaticMarkup(
    <ProjectPicker projects={[]} profiles={profiles} providers={providers} storages={storages} onAddProject={() => {}} onSelectProject={() => {}} />,
  );
  // The same head as the filled page, with the count at zero.
  assert.match(html, /<h1 id="landing-picker-title" class="settings-section-title">Projects<\/h1>/);
  assert.match(html, /0 on this machine/);
  assert.match(html, /class="btn btn-primary"[^>]*>.*Add project…/);
  // One door, because Add project opens the folder dialog straight away: a
  // dashed card in the card's anatomy, wearing every agent the build knows.
  assert.equal((html.match(/class="landing-picker-door"/g) ?? []).length, 1);
  assert.match(html, /class="landing-picker-door" aria-label="Add project: pick a folder"[\s\S]*?data-icon="folder"[\s\S]*?<strong>Pick a folder…<\/strong>[\s\S]*?data-icon="plus"/);
  assert.match(html, /landing-picker-door-bar"[\s\S]*?landing-picker-door-marks"[\s\S]*?sync-provider-mark codex[\s\S]*?sync-provider-mark claude/);
  assert.match(html, /landing-picker-caption"[\s\S]*?data-icon="cloud"[\s\S]*?Sessions &amp; memory travel with it/);
  // No mark, no "No projects yet", no explainer.
  assert.doesNotMatch(html, /mallard-logo-duck|No projects yet|landing-picker-empty/);
  assert.doesNotMatch(html, /Add a folder you run|sessions show up here|keeps them safe/);
  assert.doesNotMatch(html, /landing-picker-card"/);
});

test("every box in a landing row stands as tall as the row", () => {
  // The grid stretches each item to its row, but a box inside a block item
  // kept its content height: a card whose detail wrapped stood taller than its
  // neighbours. The item hands the row's height on; the boxes fill its width.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.landing-picker-grid > li \{\s*display: flex;\s*\}/);
  assert.match(css, /\.landing-picker-card \{\s*width: 100%;/);
  assert.match(css, /\.landing-picker-door \{\s*width: 100%;/);
});
