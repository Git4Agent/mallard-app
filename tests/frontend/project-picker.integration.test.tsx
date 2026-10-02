import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ProjectPicker from "../../src/components/project-sync/ProjectPicker";
import { pickWelcomeRide, WELCOME_RIDE_MS, WELCOME_RIDES } from "../../src/components/project-sync/ProjectsWelcome";
import type { LocalProjectSummary, ProviderProfileSummary, StorageSummary } from "../../src/types";

const storages = [
  { storage_id: "storage-local", name: "Local storage 1", provider: "local", provider_label: "Local folder", icon: "drive", available: true, summary: [] },
  { storage_id: "storage-cloud", name: "Cloud storage 2", provider: "cloud-a", provider_label: "Cloud A", icon: "cloud", available: true, summary: [] },
] as unknown as StorageSummary[];

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
    <ProjectPicker projects={projects} profiles={profiles} storages={storages} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
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

test("with no projects the landing welcomes: a mallard ride, the heading, and the two ways Add starts", () => {
  const html = renderToStaticMarkup(
    <ProjectPicker projects={[]} profiles={profiles} storages={storages} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
  );
  assert.match(html, /<section class="projects-welcome" aria-labelledby="projects-welcome-title">/);
  assert.match(html, /<h1 id="projects-welcome-title">Add your first project<\/h1>/);
  // The art is decoration, and a first paint is always the still: the ride
  // starts on the client, and never under reduced motion.
  assert.match(html, /class="projects-welcome-art" aria-hidden="true"><img src="\/welcome\/mallard-(bicycle|rocket|submarine)-still\.svg" alt=""/);
  assert.doesNotMatch(html, /\.gif/);
  // One primary action and the cloud source beside it, nothing else.
  assert.match(html, /class="btn btn-primary"[^>]*>.*Add project…/);
  assert.match(html, /class="btn btn-ghost"[^>]*>.*data-icon="cloud".*From Mallard Cloud…/);
  assert.equal((html.match(/<button/g) ?? []).length, 2);
  // The filled page's head, doors and caption are not drawn.
  assert.doesNotMatch(html, /landing-picker|on this machine/);
});

test("the welcome never shows the same ride twice in a row, and one ride ends inside five seconds", () => {
  for (const previous of WELCOME_RIDES) {
    for (const roll of [0, 0.34, 0.5, 0.67, 0.999, 1]) {
      const ride = pickWelcomeRide(previous, () => roll);
      assert.ok(WELCOME_RIDES.includes(ride));
      assert.notEqual(ride, previous);
    }
  }
  // With no previous ride every one of the three can open the page.
  assert.deepEqual(
    new Set([0, 0.4, 0.8].map((roll) => pickWelcomeRide(null, () => roll))),
    new Set(WELCOME_RIDES),
  );
  // WCAG 2.2.2: moving content that stops within five seconds needs no pause control.
  assert.ok(WELCOME_RIDE_MS <= 5000);
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
