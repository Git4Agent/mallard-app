import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ProjectPicker from "../../src/components/project-sync/ProjectPicker";
import { i18n } from "../../src/i18n";
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
    <ProjectPicker projects={projects} profiles={profiles} storages={storages} rememberedStorageByProject={{}} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
  );
  // Heading, count, and the one action.
  assert.match(html, /<h1 id="landing-picker-title" class="settings-section-title">Projects<\/h1>/);
  assert.match(html, /2 on this machine/);
  assert.match(html, /class="btn btn-primary"[^>]*>.*Add project…/);
  // One card per project; the alias is the label and the location is compacted.
  assert.equal((html.match(/class="landing-picker-card"/g) ?? []).length, 2);
  assert.match(html, /<strong>mallard-app \(Claude\)<\/strong>/);
  assert.match(html, /~\/Desktop\/project\/mallard-app/);
  assert.match(html, /aria-label="mallard-app \(Claude\), Git repository\. Storage: Local storage 1"/);
  assert.match(html, /aria-label="testProjectNonGit\. No storage linked"/);
  // Provider marks come from the binding's profile ids, not from a scan.
  assert.match(html, /sync-provider-mark claude/);
  assert.match(html, /sync-provider-mark codex/);
  // The linked storage is named beside its provider's mark; an unlinked
  // project says so, quietly, behind an empty dashed tile.
  assert.match(html, /class="landing-picker-storages-tiles"><svg[^>]*data-storage-provider="local"/);
  assert.match(html, /class="landing-picker-storages-name">Local storage 1</);
  assert.match(html, /landing-picker-card-detail muted"><span class="landing-picker-storages none"><svg[^>]*data-icon="link-off"[^>]*>.*No storage linked/);
  assert.doesNotMatch(html, /data-icon="storage"/);
  // Nothing on the page reads sessions, counts, or stamps.
  assert.doesNotMatch(html, /sessions|to push|Synced|\d{4} at \d{2}:\d{2}/);
  // And nothing of the empty shelf survives once a card exists.
  assert.doesNotMatch(html, /landing-picker-door|landing-picker-caption/);
});

test("a project card stacks its storages: the one it opens on by name, a count for the rest", () => {
  const stacked = [
    ...storages,
    { storage_id: "storage-r2", name: "R2 team", provider: "r2", provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [],
      connection: { state: "reconnect_required", code: "reconnect_required", message: "", observed_at: 0 } },
    { storage_id: "storage-gone", name: "Old share", provider: "webdav", provider_label: "WebDAV", icon: "drive", available: false, summary: [] },
  ] as unknown as StorageSummary[];
  const project = (linked_storage_ids: string[]): LocalProjectSummary[] => [
    { ...projects[0], linked_storage_ids },
  ];
  const render = (linked: string[], remembered: Record<string, string>) => renderToStaticMarkup(
    <ProjectPicker projects={project(linked)} profiles={profiles} storages={stacked} rememberedStorageByProject={remembered} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
  );

  // The remembered storage leads, the rest keep link order; one name, every mark.
  const html = render(["storage-local", "storage-cloud", "storage-r2"], { "project-a": "storage-cloud" });
  assert.equal((html.match(/class="landing-picker-storages-tiles">(?:<svg[^>]*>.*?<\/svg>)+<\/span>/g) ?? []).length, 1);
  assert.match(html, /class="landing-picker-storages-tiles"><svg[^>]*data-icon="cloud"[^>]*>.*?<\/svg><svg[^>]*data-storage-provider="local"[^>]*>.*?<\/svg><svg[^>]*data-storage-provider="r2"/);
  assert.match(html, /class="landing-picker-storages-name">Cloud storage 2<\/span><span class="landing-picker-storages-more">\+2<\/span>/);
  // A hidden storage's trouble is counted, never pinned to the name shown.
  assert.match(html, /\+2<\/span><span class="notice-chip warning">.*1 to reconnect<\/span>/);
  assert.match(html, /title="Cloud storage 2\nLocal storage 1\nR2 team"/);
  // The door's name carries every storage.
  assert.match(html, /aria-label="mallard-app \(Claude\), Git repository\. Storages: Cloud storage 2, opens here; Local storage 1; R2 team, Reconnect"/);

  // A remembered storage that is no longer linked falls back to the first link.
  assert.match(render(["storage-local", "storage-cloud"], { "project-a": "storage-r2" }), /class="landing-picker-storages-name">Local storage 1</);

  // The shown storage's own trouble is its word; unavailable outranks reconnect in the count.
  const troubled = render(["storage-gone", "storage-r2", "storage-gone"], {});
  assert.match(troubled, /class="landing-picker-storages-name">Old share<\/span><span class="notice-chip warning">.*Unavailable<\/span>/);
  assert.match(troubled, /\+2<\/span><span class="notice-chip warning">.*1 unavailable<\/span>/);
});

test("the storage stack draws its marks on the rail's tile, scoped to the stack", () => {
  const css = readFileSync("src/App.css", "utf8");
  const tile = css.match(/\n\.landing-picker-storages-tiles > svg \{[^}]*\}/)?.[0] ?? "";
  assert.match(tile, /border: var\(--box-border\);/);
  assert.match(css, /\n\.landing-picker-storages-tiles > svg\[data-storage-provider\] \{\s*background: var\(--storage-icon-tile\);/);
  assert.match(css, /\n\.landing-picker-storages-tiles > svg \+ svg \{\s*margin-left: -8px;/);
  // One line whatever the count: the name gives way, never the row.
  assert.match(css, /\n\.landing-picker-storages-name \{[^}]*text-overflow: ellipsis;[^}]*white-space: nowrap;/);
});

test("with no projects the landing welcomes: a mallard ride, the heading, and the two ways Add starts", () => {
  const html = renderToStaticMarkup(
    <ProjectPicker projects={[]} profiles={profiles} storages={storages} rememberedStorageByProject={{}} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
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

test("the welcome plate clips its ride to the frame's rounded corners", () => {
  // The ride GIFs are opaque with square, near-white corners, and only the
  // still clips itself: while a ride played, its corners poked past the
  // plate's radius, plain on the dark sepia. The plate clips its images; its
  // shadow and dark ring paint outside the box, so the clip leaves them.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /(?:^|\n)\.projects-welcome-art \{[^}]*border-radius: 2\.2% \/ 4\.6%;[^}]*overflow: hidden;/);
  assert.match(css, /:root\[data-theme="dark"\] \.projects-welcome-art \{\s*box-shadow: 0 0 0 2px var\(--ink-line\), var\(--shadow-card\);/);
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

test("in Chinese the landing and the welcome speak Chinese; names, paths and storages stay as given", () => {
  const stacked = [
    ...storages,
    { storage_id: "storage-r2", name: "R2 team", provider: "r2", provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [],
      connection: { state: "reconnect_required", code: "reconnect_required", message: "", observed_at: 0 } },
    { storage_id: "storage-gone", name: "旧共享", provider: "webdav", provider_label: "WebDAV", icon: "drive", available: false, summary: [] },
  ] as unknown as StorageSummary[];
  const picker = (list: LocalProjectSummary[], remembered: Record<string, string> = {}) => renderToStaticMarkup(
    <ProjectPicker projects={list} profiles={profiles} storages={stacked} rememberedStorageByProject={remembered} onAddProject={() => {}} onAddCloudProject={() => {}} onSelectProject={() => {}} />,
  );
  void i18n.changeLanguage("zh-CN");
  try {
    const html = picker(projects);
    assert.match(html, /<h1 id="landing-picker-title" class="settings-section-title">项目<\/h1>/);
    assert.match(html, /本机 2 个/);
    assert.match(html, /class="btn btn-primary"[^>]*>.*添加项目…/);
    assert.match(html, /<strong>mallard-app \(Claude\)<\/strong>/, "the alias is the user's");
    assert.match(html, /~\/Desktop\/project\/mallard-app/);
    assert.match(html, /aria-label="mallard-app \(Claude\)，Git 仓库。存储：Local storage 1"/);
    assert.match(html, /aria-label="testProjectNonGit。未关联存储"/);
    assert.match(html, /landing-picker-storages none"><svg[^>]*>.*未关联存储/);
    assert.doesNotMatch(html, /on this machine|No storage linked|Git repository|Add project/);

    const stack = picker([{ ...projects[0], linked_storage_ids: ["storage-local", "storage-cloud", "storage-r2", "storage-gone"] }], { "project-a": "storage-cloud" });
    assert.match(stack, /aria-label="mallard-app \(Claude\)，Git 仓库。存储：Cloud storage 2，默认打开；Local storage 1；R2 team，需重新连接；旧共享，不可用"/);
    assert.match(stack, /\+3<\/span><span class="notice-chip warning">.*1 个不可用<\/span>/);
    const reconnect = picker([{ ...projects[0], linked_storage_ids: ["storage-local", "storage-r2"] }]);
    assert.match(reconnect, /<span class="notice-chip warning">.*1 个需重新连接<\/span>/);

    const welcome = picker([]);
    assert.match(welcome, /<h1 id="projects-welcome-title">添加你的第一个项目<\/h1>/);
    assert.match(welcome, /class="btn btn-primary"[^>]*>.*添加项目…/);
    assert.match(welcome, /class="btn btn-ghost"[^>]*>.*data-icon="cloud".*从 Mallard Cloud 添加…/);
  } finally {
    void i18n.changeLanguage("en");
  }
  // Back in English, the same render is the English one.
  assert.match(picker(projects), /2 on this machine/);
});
