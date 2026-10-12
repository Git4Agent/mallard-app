import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Icon from "../../src/components/Icons";
import ProjectSidebar from "../../src/components/project-sync/ProjectSidebar";
import StartupLoadingScreen from "../../src/components/project-sync/StartupLoadingScreen";
import SyncPathBar from "../../src/components/sync-review/SyncPathBar";
import {
  CODEX_DELETE_ERROR,
  codexDeleteCloseNotice,
  offersCloseCodex,
  localSessionDeleteError,
  orderStorages,
  repositoryFingerprintMatches,
  repositoryFingerprintsRequireAcknowledgement,
} from "../../src/components/project-sync/model";
import { i18n } from "../../src/i18n";
import { en } from "../../src/i18n/en";
import ProjectLinksWorkspace, {
  CatalogStorageRepositoryRow,
  projectPullActionLabel,
  projectPushActionLabel,
  projectSyncCountPhrase,
  publishVersionHolds,
  StorageRepositoryDetails,
  StorageRepositoryRow,
} from "../../src/components/project-sync/ProjectLinksWorkspace";
import { StorageDraftSaveQueue } from "../../src/components/project-sync/storageDraftSaveQueue";
import type { CatalogStorageRepositoryView } from "../../src/components/storage-catalog/types";
import { expandHomePath, profilePathHint } from "../../src/profilePath";
import { PROJECT_SYNC_SCHEMA, type ProjectSetupDraft, type StorageProviderDescriptor } from "../../src/types";
import {
  defaultSetupRepository,
  type ProfilePathProbeState,
} from "../../src/components/project-sync/ProjectSetupWorkspace";

test("setup defaults to one Git match and preserves an explicit repository choice", () => {
  const draft: ProjectSetupDraft = {
    schema: 1, draft_id: "draft", local_project_id: "project", new_bundle_id: "new-bundle",
    project_root: "/tmp/project", canonical_project_root: "/tmp/project", display_name: "project",
    repository_fingerprint: "git-fingerprint", profiles: {}, storage: { storage_id: "storage" },
    repository: { kind: "new" }, revision: 1, created_at: 1, updated_at: 1,
  };
  const match = {
    storage_id: "storage", bundle_id: "matched-bundle", display_name: "Matching repository",
    repository_fingerprint: "git-fingerprint",
  };
  const other = { ...match, bundle_id: "other-bundle", repository_fingerprint: "other-git" };
  const selected = defaultSetupRepository(draft, [other, match], false);
  assert.deepEqual(selected, { ...match, kind: "existing", mismatch_acknowledged: false });
  assert.equal(defaultSetupRepository(draft, [match], true), draft.repository, "keep an explicit New selection");
  const resumed = { ...draft, repository: selected };
  assert.equal(defaultSetupRepository(resumed, [other], false), selected, "keep a saved existing selection");
  assert.equal(defaultSetupRepository(draft, [], false), draft.repository);
  assert.equal(defaultSetupRepository(draft, [other], false), draft.repository);
  assert.equal(defaultSetupRepository(draft, [match, { ...match, bundle_id: "second-match" }], false), draft.repository);
  assert.equal(defaultSetupRepository(draft, [{ ...match, storage_id: "previous-storage" }], false), draft.repository);
  const nonGit = { ...draft, repository_fingerprint: null };
  assert.equal(defaultSetupRepository(nonGit, [{ ...match, repository_fingerprint: null }], false), draft.repository);
});

test("App mounts ProjectSync", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  assert.match(app, /<ProjectSync/);
});

test("the generic storage icon uses the soft-outline database", () => {
  for (const size of [14, 18, 24]) {
    const markup = renderToStaticMarkup(<Icon name="storage" size={size} className="storage-test" />);
    assert.match(markup, /data-icon="storage"/);
    assert.match(markup, /class="[^"]*storage-test[^"]*"/);
    assert.match(markup, new RegExp(`width="${size}"`));
    assert.match(markup, new RegExp(`height="${size}"`));
    assert.match(markup, /viewBox="0 0 24 24"/);
    assert.match(markup, /stroke="currentColor"/);
    assert.match(markup, /stroke-width="2"/);
    assert.match(markup, /stroke-linecap="round"/);
    assert.match(markup, /stroke-linejoin="round"/);
    assert.match(markup, /shape-rendering="geometricPrecision"/);
    assert.match(markup, /aria-hidden="true"/);
    assert.match(markup, /focusable="false"/);
    assert.match(markup, /tabler-icon-database/);
    assert.doesNotMatch(markup, /tabler-icon-server|data-storage-provider/);
  }
});

test("the current frontend contract is schema 1", () => {
  assert.equal(PROJECT_SYNC_SCHEMA, 1);
});

test("project setup finishes into Activity", () => {
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const completeSetup = workspace.slice(
    workspace.indexOf("  const completeSetup = async"),
    workspace.indexOf("\n  const saveStorageLink =", workspace.indexOf("  const completeSetup = async")),
  );

  assert.match(setupWorkspace, /onClick=\{\(\) => void finalize\(\)\}/);
  assert.match(completeSetup, /setActiveProjectId\(projectId\)/);
  assert.match(completeSetup, /await loadProjectData\(projectId, nextConfig, storageId, workflowEpoch\)/);
});

test("successful unlink and relink clear their stale storage notice", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const saveStart = workspace.indexOf("  const saveStorageLink = async");
  const saveEnd = workspace.indexOf("\n  const linkStorage = async", saveStart);
  const unlinkStart = workspace.indexOf("  const unlinkStorage = async");
  const unlinkEnd = workspace.indexOf("\n  const applyReview = async", unlinkStart);
  assert.ok(saveStart >= 0 && saveEnd > saveStart, "missing saveStorageLink");
  assert.ok(unlinkStart >= 0 && unlinkEnd > unlinkStart, "missing unlinkStorage");

  const clearsMatchingNotice = /setStorageNotice\(\(current\) => \(\s*current\?\.projectId === projectId && current\.storageId === storageId \? null : current\s*\)\)/;
  assert.match(workspace.slice(saveStart, saveEnd), clearsMatchingNotice);
  assert.match(workspace.slice(unlinkStart, unlinkEnd), clearsMatchingNotice);
});

test("Push requires the reviewed destination recipe", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const commands = readFileSync("src-tauri/src/project_sync/commands.rs", "utf8");
  const pushSignature = commands.match(/pub async fn push_bundle\([\s\S]*?\) -> Result<ProjectOperationResult, String>/)?.[0] ?? "";

  assert.match(pushSignature, /recipe: BundleRecipe/);
  assert.match(workspace, /recipeSelection\(savedRecipe \?\? nextInventory\.recipe\)/);
});

test("project setup accepts only the current canonical repository fingerprint", () => {
  assert.equal(repositoryFingerprintMatches("canonical", "canonical"), true);
  assert.equal(repositoryFingerprintMatches("legacy-dot-git", "canonical"), false);
  assert.equal(repositoryFingerprintMatches("different-repository", "canonical"), false);
  assert.equal(repositoryFingerprintMatches(null, "canonical"), false);
});

test("project setup acknowledges only conflicting repository fingerprints", () => {
  assert.equal(repositoryFingerprintsRequireAcknowledgement(null, null), false);
  assert.equal(repositoryFingerprintsRequireAcknowledgement(undefined, null), false);
  assert.equal(repositoryFingerprintsRequireAcknowledgement("canonical", "canonical"), false);
  assert.equal(repositoryFingerprintsRequireAcknowledgement("different", "canonical"), true);
  assert.equal(repositoryFingerprintsRequireAcknowledgement(null, "canonical"), true);
  assert.equal(repositoryFingerprintsRequireAcknowledgement("canonical", null), true);
});

test("the production Tauri handler exposes current sync and shared log commands", () => {
  const backend = readFileSync("src-tauri/src/lib.rs", "utf8");
  assert.match(backend, /project_sync::commands::get_project_sync_config/);
  assert.match(backend, /activity_log::query_activity_logs/);
});

test("the desktop shell supports a persistent sidebar toggle and keyboard shortcut", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const icons = readFileSync("src/components/Icons.tsx", "utf8");
  const css = readFileSync("src/App.css", "utf8");
  const tauriConfig = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));

  assert.match(workspace, /PROJECT_SIDEBAR_VISIBILITY_KEY/);
  assert.match(workspace, /aria-keyshortcuts="Meta\+B Control\+B"/);
  assert.match(workspace, /event\.metaKey/);
  assert.match(workspace, /event\.ctrlKey/);
  assert.match(workspace, /className="sync-sidebar-toggle"/);
  assert.match(workspace, /sidebarVisible \? "sidebar-collapse" : "sidebar-expand"/);
  assert.match(sidebar, /id="project-sidebar"/);
  assert.match(icons, /\| "sidebar-collapse"/);
  assert.match(icons, /\| "sidebar-expand"/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-sidebar/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar/);
  // One button: open, at the far end of the brand bar; hidden, leading the
  // path bar. Both rows share a top, below the macOS lights' strip.
  assert.match(css, /\.sync-sidebar-toggle \{[^}]*top: calc\(var\(--window-strip\) \+ 19px\);[^}]*left: calc\(var\(--sync-sidebar-width, 318px\) - 33px\);/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-sidebar-toggle \{[^}]*left: 18px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar \{[^}]*padding-left: 62px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar::before \{[^}]*left: 49px;/);
  assert.match(css, /\.sync-titlebar \{[^}]*flex: 0 0 auto;/);
  assert.match(css, /\.sync-titlebar \{[^}]*min-height: 3\.25rem;/);
  assert.match(css, /\.sync-sidebar-brand\s*\{[^}]*height: 48px;/s);
  // The 14px lights' top lands at y - 9 (AppKit keeps them 9px off the
  // bottom of a titlebar sized button + y), so y 26 puts them at 17-31: inside
  // the 30px strip both ways — 10-40 in the sidebar head, 8-38 above the path
  // bar while the sidebar is hidden. y 35 was the old 42px strip's, and left
  // them 2px over the hidden-sidebar path bar.
  assert.deepEqual(tauriConfig.app.windows[0].trafficLightPosition, { x: 26, y: 26 });
});

/* Only the macOS window draws controls over the page. Windows and Linux keep
   theirs in a native title bar, so a strip cleared for traffic lights there
   was an empty band with the toggle floating in it. */
test("only macOS keeps a strip clear for the traffic lights", () => {
  const css = readFileSync("src/App.css", "utf8");
  const main = readFileSync("src/main.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  assert.match(main, /applyPlatform\(\);[\s\S]*createRoot/);
  assert.match(css, /:root \{[^}]*--window-strip: 0px;/);
  assert.match(css, /:root\[data-platform="macos"\] \{\s*--window-strip: 30px;\s*\}/);
  assert.match(css, /\.sync-sidebar-drag,\s*\.sync-window-strip \{[^}]*height: var\(--window-strip\);/);
  // Hidden, the sidebar head no longer holds the lights' strip, so the
  // workspace opens with it.
  assert.match(workspace, /sidebarVisible \? null : <div className="sync-window-strip" data-tauri-drag-region \/>/);
});

// The sidebar's width is --sync-sidebar-width, owned by the drag handle, so
// window width does not move the rail at all above the 720px icon-rail step.
// Anything inside the rail that keys off window width therefore restyles itself
// while the rail it sits in stays put — which is exactly the bug where the
// wordmark dropped 26px -> 23px and its padding 22px -> 14px as the window
// crossed 860px. Only the 720px collapse, where the rail really does become an
// icon strip, may touch these.
test("the sidebar rail is sized by its own width, never by the window", () => {
  const blocks = (css: string) => {
    const found: { limit: number; body: string }[] = [];
    const opener = /@media\s*\(max-width:\s*(\d+)px\)\s*\{/g;
    for (let hit = opener.exec(css); hit; hit = opener.exec(css)) {
      let depth = 1;
      let index = hit.index + hit[0].length;
      const start = index;
      while (index < css.length && depth > 0) {
        if (css[index] === "{") depth += 1;
        if (css[index] === "}") depth -= 1;
        index += 1;
      }
      found.push({ limit: Number(hit[1]), body: css.slice(start, index - 1) });
    }
    return found;
  };

  const wide = blocks(readFileSync("src/App.css", "utf8")).filter((block) => block.limit > 720);
  assert.ok(wide.length > 0, "App.css should still have wide breakpoints to check");
  for (const block of wide) {
    for (const selector of [".sync-sidebar-brand", ".sync-brand-logo", ".sync-sidebar "]) {
      assert.ok(
        !block.body.includes(selector),
        `@media (max-width: ${block.limit}px) must not restyle ${selector.trim()}`,
      );
    }
    assert.doesNotMatch(
      block.body,
      /\.sync-sidebar\s*\{/,
      `@media (max-width: ${block.limit}px) must not resize the sidebar`,
    );
  }
});

test("the path bar's trail is sized by the bar, never by the window", () => {
  const css = readFileSync("src/App.css", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  // The bar is the window less the sidebar, so a viewport share is a far
  // bigger share of the row than it reads as. At `max-width: 30vw` the session
  // title's floor came out wider than both ancestors had between them: the
  // storage crumb was squeezed out of its own box and its icon carried on
  // painting over the project's name.
  for (const selector of [".sync-path-crumb", ".sync-history-branch-trigger"]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(
      css,
      new RegExp(`\\n${escaped} \\{[^}]*max-width:[^;]*cqi`),
      `${selector} must cap against the bar`,
    );
    assert.doesNotMatch(
      css,
      new RegExp(`\\n${escaped} \\{[^}]*max-width:[^;]*\\dvw`),
      `${selector} must not cap against the window`,
    );
  }

  // A crumb may give up its whole label; it may never be squeezed under the
  // icon and the chevron, which do not shrink and so carry on painting where
  // the box has already ended — over the crumb beside them.
  for (const selector of [".sync-path-item", ".sync-path-item:first-child", ".sync-path-crumb-link"]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(
      css,
      new RegExp(`\\n${escaped} \\{[^}]*min-width: \\d+(?:\\.\\d+)?rem;`),
      `${selector} must floor at its own chrome`,
    );
  }
  assert.match(css, /\n\.sync-path-crumb-toggle \{[^}]*flex: 0 0 auto;/);
  assert.doesNotMatch(
    css,
    /\.sync-path-sync-label\s*\{[^}]*display:\s*none/,
    "compact layouts must keep Push and Pull visible",
  );
  assert.match(shell, /window\.innerWidth - PROJECT_WINDOW_FRAME_WIDTH - MIN_PROJECT_WORKSPACE_WIDTH/);
  assert.match(css, /\.sync-pathbar \.sync-history-branch-name,[\s\S]*?display: none;/);
});

// The ladder is laid out down to a 580px bar. The window frame sits between the
// window and the bar: an 8px gutter each side and the 8px resize seam. Counted
// as bar, it let an 800px window, or a sidebar dragged to its limit in any
// window, put the bar at 556px, and the Git trail ran under the attention chip.
test("the path bar never goes under its 580px floor", () => {
  const css = readFileSync("src/App.css", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
    app: { windows: { minWidth: number }[] };
  };
  const constant = (name: string) => Number(shell.match(new RegExp(`const ${name} = (\\d+);`))?.[1]);

  assert.match(css, /--retro-gutter-x: 8px;/);
  assert.match(css, /\n\.sync-app \{[^}]*padding: var\(--retro-gutter\) var\(--retro-gutter-x\);/);
  assert.match(css, /\n\.sync-sidebar-resizer \{[^}]*width: 12px;[^}]*margin-right: -2px;[^}]*margin-left: -2px;/);
  const frame = 2 * 8 + (12 - 2 - 2);
  assert.equal(constant("PROJECT_WINDOW_FRAME_WIDTH"), frame);

  const floor = constant("MIN_PROJECT_WORKSPACE_WIDTH");
  assert.equal(floor, 580);
  assert.equal(
    config.app.windows[0].minWidth - constant("MIN_PROJECT_SIDEBAR_WIDTH") - frame,
    floor,
    "the smallest window must still give the bar its floor",
  );
});

// The bar does not clip and its action cluster runs the bar's full height, so
// the right inset alone keeps a control's box out of the rounded corners. At
// 10px, pressing Push, Pull or ⋯ in the live WebKit view repainted that box
// with the bar's background and left both right corners squared off.
test("nothing in the path bar reaches into its rounded corners", () => {
  const css = readFileSync("src/App.css", "utf8");
  const bar = css.match(/\n\.sync-titlebar \{[^}]*background: var\(--bg-2\);\n\}/)?.[0] ?? "";
  assert.match(bar, /padding-right: var\(--retro-panel-radius\);/, "the right inset must clear the corner");
  // The radius covers the 16px inner curve plus 2px. A control that hangs
  // further past its box than that needs a wider inset.
  assert.match(css, /--shadow-control: 2px 2px 0 /);
  for (const selector of [".sync-path-sync", ".sync-path-more"]) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(css, new RegExp(`\\n${escaped}:active:not\\(:disabled\\) \\{[^}]*transform: translate\\(2px, 2px\\);`));
  }
});

// Bare at 14px, the provider artwork sank into the honey current row. The bar
// now draws it as the rail does: on the provider's pale tile in the ink box.
test("the path bar stands storage marks on the rail's tile", () => {
  const css = readFileSync("src/App.css", "utf8");
  const tile = css.match(
    /\n:is\(\.sync-path-crumb, \.sync-path-menu-current, \.sync-path-menu-item\) > svg\[data-storage-provider\] \{[^}]*\}/,
  )?.[0] ?? "";
  assert.match(tile, /border: var\(--box-border\);/);
  assert.match(tile, /background: var\(--storage-icon-tile\);/);
  // Scoped to the bar: the settings plate and the landing cards pad their own
  // artwork, and an unscoped rule would tile them twice.
  assert.doesNotMatch(css, /\nsvg\[data-storage-provider\] \{/);
  // Menu rows keep their height and the icon-only crumb stays inside its
  // item's floor.
  assert.match(
    css,
    /\n:is\(\.sync-path-menu-current, \.sync-path-menu-item\) > svg\[data-storage-provider\] \{[^}]*margin-block: -2px;/,
  );
  assert.match(css, /\n\.sync-path-crumb:has\(> svg\[data-storage-provider\]\) \{\s*padding-left: 3px;/);

  // The selectors reach the mark only as a direct child of the crumb, the
  // current row and a switch target, which is where the bar renders it.
  const markup = renderToStaticMarkup(
    <SyncPathBar
      crumbs={[{
        id: "storage",
        label: "Mallard Cloud",
        icon: "storage-mallard",
        tone: "root",
        menuHeader: { label: "Mallard Cloud", icon: "storage-mallard" },
        menu: [{
          label: "Switch to",
          items: [{ id: "local", label: "my-storage", icon: "storage-local", onSelect: () => {} }],
        }],
      }, { id: "project", label: "mallard-app", icon: "git-folder" }]}
    />,
  );
  assert.match(markup, /class="sync-path-crumb root"[^>]*><svg[^>]*data-storage-provider="mallard"/);
  assert.match(markup, /class="sync-path-menu-current"[^>]*><svg[^>]*data-storage-provider="mallard"/);
  assert.match(markup, /class="sync-path-menu-item target"[^>]*><svg[^>]*data-storage-provider="local"/);
});

test("the sidebar names its roots, rows and actions in the interface language; names and paths stay as given", () => {
  const noop = () => undefined;
  const project = (overrides: object) => ({
    local_project_id: "p-git", bundle_id: "b", display_name: "repo-one", local_alias: null, revision: 1,
    project_root: "/Users/me/code/repo-one", profile_ids: { codex: "profile-codex" }, linked_storage_ids: [], is_git_repository: true,
    ...overrides,
  });
  const props = {
    projects: [
      project({}),
      project({ local_project_id: "p-plain", display_name: "plain", local_alias: "我的笔记", profile_ids: {}, is_git_repository: false }),
    ],
    drafts: [{ draft_id: "d1", display_name: "draft repo", project_root: "/tmp/draft", updated_at: 1, revision: 1, status: "attention", last_error: "disk full" }],
    activeDraftId: null,
    storages: [
      { storage_id: "s-attic", name: "Attic", provider: "local", provider_label: "Local folder", icon: "drive", available: true, summary: [] },
      { storage_id: "s-old", name: "", provider: "webdav", provider_label: "WebDAV", icon: "drive", available: false, summary: [] },
    ],
    storageDrafts: [{ draft_id: "sd", storage_id: "s-new", name: "", provider: "r2", config_version: 1 }],
    storageProviders: [], storageUsage: { "s-attic": 2 }, activeProjectId: "p-git", activeStorageId: null,
    catalogScopes: [], activeCatalogScopeId: null, addingProject: false, addingCatalogScope: false, addingStorage: false,
    catalogIndexOpen: false, storageIndexOpen: false, settingsOpen: false, loading: false, busy: false, activityOpen: false, unreadLogs: 3,
    onOpenProjects: noop, onSelectProject: noop, onRemoveProject: noop, onSelectDraft: noop, onDiscardDraft: noop, onToggleActivity: noop,
    onAddProject: noop, onOpenStorage: noop, onOpenStorageDraft: noop, onDiscardStorageDraft: noop, onRemoveStorage: noop, onAddStorage: noop,
    onOpenCatalogIndex: noop, onOpenCatalogScope: noop, onAddCatalogScope: noop, onRemoveCatalogScope: noop, onOpenStorageIndex: noop, onOpenSettings: noop,
  } as unknown as Parameters<typeof ProjectSidebar>[0];
  const render = (overrides: Partial<Parameters<typeof ProjectSidebar>[0]> = {}) => renderToStaticMarkup(<ProjectSidebar {...props} {...overrides} />);

  const english = render();
  assert.match(english, /aria-label="Projects" title="Projects"/);
  assert.match(english, /aria-label="Logs, 3 unread"/);
  assert.match(english, /<span class="sidebar-section-title">Projects<\/span>/);
  assert.match(english, /aria-label="repo-one, Git repository"/);
  assert.match(english, /aria-label="我的笔记\. Project issue: No agent profile is assigned"/);
  assert.match(english, /title="\/tmp\/draft — last attempt failed: disk full"/);
  assert.match(english, /aria-label="Mallard version \d+\.\d+\.\d+"/);

  void i18n.changeLanguage("zh-CN");
  try {
    const html = render();
    assert.match(html, /<nav class="sidebar-root-tabs" aria-label="项目同步导航">/);
    for (const name of ["项目", "资源库", "存储", "设置"]) assert.match(html, new RegExp(`aria-label="${name}" title="${name}"`));
    assert.match(html, /aria-label="日志，3 条未读" title="日志"/);
    assert.match(html, /<span class="sidebar-section-title">项目<\/span>/);
    assert.match(html, /aria-label="添加项目"/);
    // The user's names and paths, and the backend's error, are never translated.
    assert.match(html, /aria-label="repo-one，Git 仓库"/);
    assert.match(html, /<span>我的笔记<\/span>/);
    assert.match(html, /aria-label="我的笔记。项目问题：未分配代理配置"/);
    assert.match(html, /title="Git 仓库\n\/Users\/me\/code\/repo-one"/);
    assert.match(html, /title="仓库：plain\n\/Users\/me\/code\/repo-one"/);
    assert.match(html, /title="\/tmp\/draft — 上次尝试失败：disk full"/);
    assert.match(html, /<span class="sync-draft-badge attention">.*?草稿<\/span>/);
    assert.match(html, /aria-label="从 Mallard 中移除 repo-one"/);
    assert.match(html, /<strong>Mallard<\/strong>/);
    assert.match(html, /aria-label="Mallard v\d+\.\d+\.\d+"/);
    assert.doesNotMatch(html, /Project sync navigation|Add project|Git repository|resumable|Remove /);

    const storages = render({ storageIndexOpen: true });
    assert.match(storages, /<span class="sidebar-section-title">存储<\/span>/);
    assert.match(storages, /<span>Attic<\/span>/);
    assert.match(storages, /title="Attic 关联了 2 个项目；请先取消关联再移除"/);
    assert.match(storages, /<span>（未命名）<\/span>/);
    assert.match(storages, /aria-label="打开存储。存储问题：当前构建的 Mallard 没有 WebDAV 适配器"/);
    assert.match(storages, /aria-label="继续设置 （未命名） — 可继续的存储草稿"/);
    assert.match(storages, /aria-label="添加存储"/);

    const loading = render({ loading: true, projects: [], drafts: [] });
    assert.match(loading, /<div class="sidebar-msg">正在载入项目…<\/div>/);
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.equal(render(), english, "back in English, the sidebar is the English one again");
});

test("top-level navigation opens stable index pages", () => {
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  // The strip names each root once, in rail order, and each tab carries the
  // destination in its own accessible name because it draws no text.
  assert.match(sidebar, /aria-label=\{t\("roots\.projects"\)\}[\s\S]*?aria-label=\{t\("roots\.catalog"\)\}[\s\S]*?aria-label=\{t\("roots\.storages"\)\}/);
  assert.deepEqual([en.shell.roots.projects, en.shell.roots.catalog, en.shell.roots.storages], ["Projects", "Catalog", "Storages"]);
  assert.match(sidebar, /onClick=\{onOpenProjects\}/);
  assert.match(sidebar, /onClick=\{onOpenCatalogIndex\}/);
  assert.match(sidebar, /onClick=\{onOpenStorageIndex\}/);
  assert.doesNotMatch(sidebar, /primaryStorageId|onOpenCatalogMenu/);

  assert.match(shell, /const openProjectsIndex = async \(\) => \{[\s\S]*?setActiveProjectId\(null\);/);
  assert.match(shell, /const openCatalogIndex = async \(\) => \{[\s\S]*?setCatalogIndexOpen\(true\);/);
  assert.match(shell, /catalogIndexOpen \? \(\s*<CatalogIndexPage/);
  assert.match(shell, /storageIndexOpen \? \(\s*<StorageIndexPage/);
  assert.doesNotMatch(shell, /const openCatalogMenu/);

  assert.match(workspace, /className="sync-storage-index-list" aria-label=\{t\("links\.projectsLabel"\)\}/);
  assert.equal(en.projectPage.links.projectsLabel, "Projects");
  assert.match(workspace, /onClick=\{\(\) => onSelectProject\(project\.local_project_id\)\}/);
});

test("App imports one complete global stylesheet", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  const css = readFileSync("src/App.css", "utf8");
  const imports = [...app.matchAll(/import "([^"]+\.css)";/g)].map((match) => match[1]);

  assert.deepEqual(imports, ["./App.css"]);
  assert.doesNotMatch(css, /@import|@layer/);
  assert.match(css, /@font-face\s*\{[\s\S]*font-family: "Baloo 2"/);
});

test("the shell notice reports its own tone from the shared severity ladder", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const css = readFileSync("src/App.css", "utf8");

  // The shell uses the shared notice recipe and keeps only its dismiss control.
  assert.doesNotMatch(css, /\.sync-notice[\s.,:{]/);
  assert.match(css, /\.sync-notice-dismiss\s*\{[^}]*border-radius: 3px;/s);

  // A green tick used to announce every notice, including the ones that report
  // a stop. The tone maps onto the app-wide rung, so those two cannot drift
  // apart again — and the rung decides the glyph, the tint AND the word the
  // box prints before its message.
  assert.match(workspace, /type SyncNoticeTone = "ok" \| "note" \| "attention"/);
  assert.match(workspace, /severity: notice\.tone === "attention" \? "warning" : notice\.tone === "note" \? "info" : "success"/);
  // An unconfirmed binding is a stop, not a success.
  assert.match(workspace, /notify\(t\(nextDetail\.binding \? "notices\.confirmBinding" : "notices\.chooseBinding"\), "attention"\)/);
  assert.match(en.shell.notices.confirmBinding, /before pulling\.$/);
  assert.match(en.shell.notices.chooseBinding, /before pulling\.$/);
  // Every other call site stays a one-argument confirmation.
  assert.match(workspace, /const notify = \(message: string, tone: SyncNoticeTone = "ok"\) => setNotice\(\{ message, tone \}\)/);

  // The shell slot fills opaquely at every rung. A translucent tint here sits
  // on the frame gutter rather than on paper, which is what took the ink below
  // 4.5:1 when the banner last carried one.
  for (const rung of ["danger", "warning", "info", "success"]) {
    assert.match(css, new RegExp(`\\.notice\\.sync-backend-banner\\.${rung} \\{ background: color-mix\\(in srgb, var\\(--\\w+\\) \\d+%, var\\(--surface-card\\)\\); \\}`));
  }
});

test("notice copy stays one line: no two-sentence reassurance", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const notices = [...workspace.matchAll(/\bnotify\(\s*([\s\S]*?)\);/g)].map((m) => m[1]);
  assert.ok(notices.length >= 15, `expected the notice corpus, found ${notices.length}`);
  for (const body of notices) {
    for (const literal of body.matchAll(/[`"]([^`"]{12,})[`"]/g)) {
      const text = literal[1];
      if (!/[a-z]/.test(text)) continue;
      // A toast is a confirmation, not documentation. One sentence; a dash
      // carries the caveat. Trailing "." is the only period allowed.
      assert.ok(
        !/\.\s+[A-Z]/.test(text),
        `notice copy runs to a second sentence: "${text}"`,
      );
    }
  }
});

test("the titlebar stays silent for a project and names the screens without a trail", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  // The project workspace names itself in the path bar, so the titlebar would
  // only repeat it — and so does the Catalog row's page, which is why it is
  // not named here either. Every screen without a trail still says what it is.
  assert.match(
    workspace,
    /const onProjectsLanding = !activeDraftSummary && !choosingProjectFolder && !addingCatalogScope && !activeSummary;\s*const workspaceTitle = activeDraftSummary\?\.display_name\s*\?\? \(choosingProjectFolder \? t\("pages\.newProject"\) : null\)\s*\?\? \(addingCatalogScope \? t\("pages\.addCatalog"\) : null\)\s*\?\? \(onProjectsLanding \? t\("roots\.projects"\) : null\);/,
  );
  assert.deepEqual([en.shell.pages.newProject, en.shell.pages.addCatalog, en.shell.roots.projects], ["New project", "Add Catalog resource", "Projects"]);
  assert.match(workspace, /\{workspaceTitle && \(/);
  assert.match(workspace, /<Icon name=\{workspaceIcon\} size=\{16\} \/>/);
});

test("the shell verifies host identity and relies on backend auto-registration", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const startupScreen = readFileSync("src/components/project-sync/StartupLoadingScreen.tsx", "utf8");
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const linksWorkspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(workspace, /projectSyncApi\.getHostIdentityStatus\(\)/);
  assert.match(workspace, /!hostIdentityStatus && hostIdentityError/);
  assert.match(workspace, /<h1 id="sync-host-recovery-title">\{t\("load\.hostTitle"\)\}<\/h1>/);
  assert.equal(en.shell.load.hostTitle, "Host identity unavailable");
  assert.match(workspace, /if \(!hostIdentityReady\) return/);
  const shellRefreshes = [...workspace.matchAll(/const \{([^}]*)\} = await loadShell\(\);/g)];
  assert.ok(shellRefreshes.length > 0);
  assert.ok(
    shellRefreshes.every((match) => match[1].includes("hostIdentityReady")),
    "every shell refresh continuation must stop when host identity verification fails",
  );
  assert.match(workspace, /const \[hostIdentityShellReady, setHostIdentityShellReady\] = useState\(false\)/);
  assert.match(workspace, /const clearHostScopedWorkflows = useCallback\(\(\) => \{[\s\S]*setPendingPush\(null\)/);
  assert.match(workspace, /const clearHostScopedWorkflows = useCallback\(\(\) => \{[\s\S]*setEditingBinding\(null\)/);
  assert.match(workspace, /const clearHostScopedWorkflows = useCallback\(\(\) => \{[\s\S]*setRestorePlan\(null\)/);
  assert.match(
    workspace,
    /const loadProjectData = async \([\s\S]*const requestedWorkflowEpoch = workflowEpoch \?\? hostWorkflowEpoch\.current;[\s\S]*getProject\(projectId\);[\s\S]*hostWorkflowEpoch\.current !== requestedWorkflowEpoch/,
  );
  assert.match(
    workspace,
    /const completeSetup = async \([\s\S]*workflowEpoch: number,[\s\S]*hostWorkflowEpoch\.current !== workflowEpoch[\s\S]*await loadShell\(\);[\s\S]*!hostIdentityReady \|\| hostWorkflowEpoch\.current !== workflowEpoch/,
  );
  assert.match(
    workspace,
    /const connectStorageBundle = async[\s\S]*const workflowEpoch = hostWorkflowEpoch\.current;[\s\S]*await loadShell\(\);[\s\S]*!hostIdentityReady \|\| hostWorkflowEpoch\.current !== workflowEpoch/,
  );
  // A mid-load identity change (backend auto-registration) restarts the
  // shell load instead of surfacing stale host-scoped state.
  assert.match(
    workspace,
    /verifiedHostIdentity\.host_id !== nextHostIdentity\.host_id[\s\S]*return loadShell\(\);/,
  );
  assert.match(workspace, /if \(!hostIdentityShellReady\) \{/);
  assert.match(workspace, /<StartupLoadingScreen stage=\{startupStage\} \/>/);
  assert.match(startupScreen, /const status = t\(`startup\.\$\{currentStage\.id\}`\);/);
  assert.match(renderToStaticMarkup(<StartupLoadingScreen stage="installation" />), /role="status"[^>]*>Checking installation…<\/p>/);
  assert.match(workspace, /nextDetail\.binding\.state !== "active"/);
  assert.match(workspace, /expected_revision: nextDetail\.binding\?\.revision \?\? null/);
  assert.match(workspace, /nextDetail\.binding\?\.state === "active"/);
  // The blocking recovery gate is gone: the backend registers a fresh host
  // automatically, so the shell never asks the user to register.
  assert.doesNotMatch(workspace, /origin_key_available/);
  assert.doesNotMatch(workspace, /registerAsNewHost/);
  assert.doesNotMatch(workspace, /"Register as new host"/);
  assert.doesNotMatch(JSON.stringify(en.shell), /Register as new host|Host settings/, "nor in the shell's catalog");
  assert.ok(
    workspace.indexOf("if (!hostIdentityShellReady)") < workspace.indexOf("className={`sync-app"),
    "the ordinary shell must remain behind the completed identity refresh gate",
  );
  assert.match(linksWorkspace, /const bindingActive = projectBinding\?\.state === "active"/);
  assert.match(linksWorkspace, /const workspaceCanSync = workspaceBindingActive && workspaceProfilesReadable/);
  assert.match(linksWorkspace, /onConfigureBinding\(workspaceProject\.local_project_id\)/);
  assert.match(setupWorkspace, /mountedRef\.current && isWorkflowCurrent\(workflowEpoch\)/);
  assert.match(setupWorkspace, /if \(!workflowIsActive\(\)\) return;\s*onFinalized\(detail, workflowEpoch\)/);
  assert.doesNotMatch(workspace, /Host settings/);
});

test("startup overlaps independent reads", () => {
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const loadShell = source.slice(
    source.indexOf("  const loadShell = async"),
    source.indexOf("\n  const storageFailureMessage"),
  );
  const configRead = loadShell.indexOf("await projectSyncApi.getConfig()");
  const recovery = loadShell.indexOf("await projectSyncApi.recoverStartupState()");
  const projectReads = loadShell.indexOf("const projectReads = Promise.allSettled([");
  const providerReads = loadShell.indexOf("const providerReads = Promise.allSettled([");
  const firstBatchWait = loadShell.indexOf("await projectReads");
  const providerBatchWait = loadShell.indexOf("await providerReads");
  const verificationRead = loadShell.lastIndexOf(
    "await projectSyncApi.getHostIdentityStatus()",
  );
  const shellReady = loadShell.indexOf("setHostIdentityShellReady(true)");

  assert.ok(configRead >= 0 && configRead < recovery);
  assert.ok(recovery < projectReads);
  assert.ok(projectReads < providerReads && providerReads < firstBatchWait);
  assert.ok(firstBatchWait < providerBatchWait && providerBatchWait < verificationRead);

  for (const request of [
    "projectSyncApi.listProjects()",
    "refreshSetupDrafts(requestId)",
    "refreshStorageSetupDrafts(requestId)",
    "refreshCatalogScopes(requestId)",
    "projectSyncApi.listProjectRepositoryKinds()",
  ]) {
    const index = loadShell.indexOf(request, projectReads);
    assert.ok(index > projectReads && index < providerReads, `${request} must start in projectReads`);
  }
  for (const request of [
    "projectSyncApi.listAgentProviders()",
    "projectSyncApi.listProviderProfiles()",
    "projectSyncApi.listBindings()",
  ]) {
    const index = loadShell.indexOf(request, providerReads);
    assert.ok(index > providerReads && index < firstBatchWait, `${request} must start in providerReads`);
  }

  assert.ok(shellReady >= 0);
  assert.doesNotMatch(loadShell, /PreSync|preSync/);
});

test("in Chinese the startup screen says each stage in Chinese and keeps the product's name", () => {
  void i18n.changeLanguage("zh-CN");
  try {
    const html = renderToStaticMarkup(<StartupLoadingScreen stage="repair" />);
    assert.match(html, /<h1 id="sync-startup-title">Mallard<\/h1>/);
    assert.match(html, /aria-label="Mallard 启动进度"/);
    assert.match(html, /aria-valuetext="正在完成中断的工作…"/);
    assert.match(html, /role="status"[^>]*>正在完成中断的工作…<\/p>/);
  } finally {
    void i18n.changeLanguage("en");
  }
});

test("the startup screen communicates minimal, staged loading progress", () => {
  const html = renderToStaticMarkup(<StartupLoadingScreen stage="projects" />);
  const css = readFileSync("src/App.css", "utf8");

  assert.match(html, /<h1 id="sync-startup-title">Mallard<\/h1>/);
  assert.match(html, /Finding projects…/);
  assert.doesNotMatch(html, /Preparing your workspace/);
  assert.match(html, /role="progressbar"/);
  assert.match(html, /aria-valuenow="4"/);
  assert.match(html, /aria-valuemax="6"/);
  assert.match(css, /\.sync-startup-screen \{/);
  assert.match(css, /@keyframes sync-startup-pulse/);
  assert.match(css, /@keyframes sync-startup-progress-glint/);
});

test("host-scoped workflows keep their originating epoch across async continuations", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const section = (source: string, name: string, nextName: string | null) => {
    const start = source.indexOf(`  const ${name} =`);
    const end = nextName
      ? source.indexOf(`\n  const ${nextName} =`, start + 1)
      : source.indexOf("\n\n  if (!draft)", start + 1);
    assert.ok(start >= 0, `missing ${name}`);
    assert.ok(end > start, `missing end of ${name}`);
    return source.slice(start, end);
  };
  const capturesBefore = (body: string, awaitExpression: string) => {
    const capture = body.indexOf("const workflowEpoch = hostWorkflowEpoch.current;");
    const awaited = body.indexOf(awaitExpression);
    assert.ok(capture >= 0, "workflow must capture the host workflow epoch");
    assert.ok(awaited > capture, `epoch must be captured before ${awaitExpression}`);
  };

  // Add project writes nothing: it opens setup at New project's doors (the
  // Folder field, or From Mallard Cloud on an account). The draft is born
  // when a step chooses a folder, inside its own epoch.
  const addProject = section(workspace, "beginAddProject", "chooseProjectFolder");
  assert.match(addProject, /setSetupView\(\{ kind: "folder", \.\.\.open \}\)/);
  assert.doesNotMatch(addProject, /createSetupDraft|await open\(/);
  const chooseFolder = section(workspace, "chooseProjectFolder", "refreshCatalogScopes");
  capturesBefore(chooseFolder, "await projectSyncApi.createSetupDraft(");
  assert.match(chooseFolder, /await projectSyncApi\.createSetupDraft\(projectRoot\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);
  assert.match(chooseFolder, /current\?\.kind === "folder" \? \{ kind: "draft", draftId: created\.draft\.draft_id \} : current/);
  assert.match(chooseFolder, /refreshSetupDrafts\(undefined, workflowEpoch\)/);

  // ensureProfileAtPath runs inside a caller's envelope: the epoch arrives as
  // a parameter, and every await re-checks it before touching state.
  const ensureProfile = section(workspace, "ensureProfileAtPath", "refreshSetupDrafts");
  assert.match(ensureProfile, /workflowEpoch: number,/);
  assert.match(ensureProfile, /await projectSyncApi\.probeProviderProfile\(provider, path\.trim\(\)\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return null;/);
  assert.match(ensureProfile, /if \(hostWorkflowEpoch\.current !== workflowEpoch\) return null;\s*setProfiles\(nextProfiles\);/);

  const discardDraft = section(workspace, "discardSetupDraft", "completeSetup");
  capturesBefore(discardDraft, "await confirm(");
  assert.match(discardDraft, /await projectSyncApi\.discardSetupDraft\(draftId\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);

  const linkStorage = section(workspace, "linkStorage", "openPushChooser");
  capturesBefore(linkStorage, "await projectSyncApi.listRemoteBundleSnapshots");
  assert.match(linkStorage, /saveStorageLink\(projectId, storageId, workflowEpoch\)/);
  assert.match(linkStorage, /hostWorkflowEpoch\.current === workflowEpoch\) setBusy\(false\)/);

  const renameProject = section(workspace, "renameProject", "planRestore");
  capturesBefore(renameProject, "await projectSyncApi.renameProject");
  // The guard still bails on a superseded workflow; it now reports that
  // distinctly from a real failure so the rename form stays quiet.
  assert.match(renameProject, /hostWorkflowEpoch\.current !== workflowEpoch\) return \{ ok: false, message: null \};/);

  const keepBundle = section(workspace, "keepPendingBundle", "removeProject");
  capturesBefore(keepBundle, "await saveStorageLink");
  assert.match(keepBundle, /openPushChooser\(pending\.projectId, pending\.storageId, \{ kind: "project" \}, undefined, workflowEpoch\)/);

  const removeProject = section(workspace, "removeProject", "closePullReview");
  capturesBefore(removeProject, "await confirm(");
  assert.match(removeProject, /await refresh\(workflowEpoch\)/);

  const unlinkStorage = section(workspace, "unlinkStorage", "applyReview");
  capturesBefore(unlinkStorage, "await confirm(");
  assert.match(unlinkStorage, /await projectSyncApi\.removeLink\(projectId, storageId\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);
  assert.match(unlinkStorage, /loadProjectData\(projectId, nextConfig, nextStorageId, workflowEpoch\)/);

  // Storage writes are focused commands. A draft commit re-reads the config
  // it changed, and a rename adopts the config the backend returns.
  const commitStorage = section(workspace, "commitStorageDraft", "renameStorage");
  assert.match(commitStorage, /const workflowEpoch = hostWorkflowEpoch\.current;/);
  assert.match(commitStorage, /await projectSyncApi\.getConfig\(\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return null;/);
  const renameStorage = section(workspace, "renameStorage", "clearStorageData");
  assert.match(renameStorage, /await projectSyncApi\.renameStorage\(storageId, name, configRef\.current\.revision\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return false;/);

  const clearStorage = section(workspace, "clearStorageData", "removeStorage");
  capturesBefore(clearStorage, "await confirm(");
  assert.match(clearStorage, /await projectSyncApi\.clearStorageData\(/);
  assert.match(clearStorage, /await confirm\(t\("confirm\.clearStorage", \{ name: storageName \}\)/);
  assert.match(en.shell.confirm.clearStorage, /^Permanently clear all Mallard data/);

  const removeStorage = section(workspace, "removeStorage", "openNewStorageConfiguration");
  capturesBefore(removeStorage, "await confirm(");
  assert.match(removeStorage, /await projectSyncApi\.removeStorage\(storageId, configRef\.current\.revision\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);

  assert.match(workspace, /workflowEpoch=\{hostWorkflowEpoch\.current\}/);
  assert.match(workspace, /isWorkflowCurrent=\{\(workflowEpoch\) => \([\s\S]*hostWorkflowEpoch\.current === workflowEpoch/);
  const finalize = section(setupWorkspace, "finalize", null);
  assert.match(finalize, /const detail = await projectSyncApi\.finalizeProjectSetup\(draftId, expectedRevision\);\s*if \(!workflowIsActive\(\)\) return;\s*onFinalized\(detail, workflowEpoch\)/);
  assert.match(setupWorkspace, /mountedRef\.current = false;\s*inspectRequest\.current \+= 1;/);
});

// One home per scope: each crumb's menu is a header naming its object, that
// object's verbs, then Switch-to commands. The old shape — a radio switcher on
// the crumb and the verbs in the far-right ⋯ — is gone, not aliased.
test("path bar menus keep one home per scope", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const bar = readFileSync("src/components/sync-review/SyncPathBar.tsx", "utf8");

  // Storage crumb: header block, settings + unlink beside their noun, and the
  // link row that stays put at zero candidates with its reason spelled out.
  assert.match(workspace, /menuHeader: \{\s*label: workspaceStorage\.name \|\| t\("fallback\.unnamed"\),/);
  assert.equal(en.projectPage.fallback.unnamed, "(unnamed)");
  assert.match(workspace, /label: t\("storageMenu\.details"\)/);
  assert.equal(en.projectPage.storageMenu.details, "Storage details…");
  assert.match(workspace, /label: t\("storageMenu\.unlink", \{ project: projectLabel\(workspaceProject\) \}\)/);
  assert.equal(en.projectPage.storageMenu.unlink, "Unlink from {{project}}…");
  assert.match(workspace, /icon: "link-off" as const/);
  assert.match(workspace, /disabledReason: workspaceAvailableStorages\.length === 0/);
  // Link storage… opens the chooser outright; the old toggle belonged to an
  // in-card band that could be left open behind you.
  assert.match(workspace, /onSelect: \(\) => setLinkingProjectId\(workspaceProject\.local_project_id\)/);
  // The crumb's Link storage… opens the chooser dialog; the in-card band that
  // used to answer it is gone, not aliased.
  assert.match(workspace, /<StorageLinkDialog[\s\S]*onCancel=\{\(\) => setLinkingProjectId\(null\)\}/);
  assert.doesNotMatch(workspace, /storage-link-picker/);
  assert.doesNotMatch(workspace, /checked: storage\.id === workspaceStorage\.id/);
  assert.doesNotMatch(JSON.stringify(en.projectPage), /"Unlink storage"/);

  // The workspace ⋯ holds app scope alone: Settings, the theme switch, and
  // the full reload. The checked item exposes the current theme state.
  assert.match(workspace, /const pathOverflow: PathBarMenuGroup\[\] = workspaceProject \? appActions : \[\];/);
  assert.match(sync, /label: t\("common:settings"\),\s*icon: "settings" as const,\s*onSelect: openSettings,/);
  assert.match(sync, /label: t\("window\.darkTheme"\),\s*icon: "moon" as const,\s*checked: theme === "dark",/);
  assert.equal(en.shell.window.darkTheme, "Dark theme");

  // The bar renders no radio items anywhere, and inert rows stay focusable.
  assert.doesNotMatch(bar, /menuitemradio/);
  assert.match(bar, /role=\{checkable \? "menuitemcheckbox" : "menuitem"\}/);
  assert.match(bar, /aria-disabled=\{inert \|\| undefined\}/);
});

test("refocus reconciles external changes narrowly and the full reload lives in the titlebar menu", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");

  // The sidebar no longer carries an always-visible refresh icon; full reload is a
  // labeled recovery action in the titlebar overflow menu.
  assert.doesNotMatch(sidebar, /onRefresh|Refresh projects/);
  assert.doesNotMatch(JSON.stringify(en.shell.sidebar), /Refresh projects/);
  assert.match(workspace, /label: t\("window\.reload"\),\s*detail: t\("window\.reloadDetail"\),/);
  assert.equal(en.shell.window.reload, "Reload app data");
  assert.match(workspace, /className="sync-titlebar-actions"/);
  // The Projects landing and New project reach the theme through the app
  // menu's Dark theme item only.
  assert.match(workspace, /onProjectsLanding \|\| choosingProjectFolder \? null : \(\s*<button[\s\S]*?className="sync-theme-button"/);
  assert.match(workspace, /<PathBarMenu\s+id="sync-app-actions-menu"\s+groups=\{appActions\}/);
  // Its items sit at tabIndex -1, so opening must move focus in and closing
  // must hand it back — a no-op registerRef left the menu unreachable by key.
  assert.match(workspace, /registerRef=\{\(node\) => \{ appMenuPanelRef\.current = node; \}\}/);
  assert.match(workspace, /focusMenuEdge\(appMenuPanelRef\.current, "first"\)/);
  assert.match(workspace, /onClose=\{closeAppMenu\}/);
  assert.doesNotMatch(workspace, /registerRef=\{\(\) => undefined\}/);

  // Refocus reconcile only fires after the window was away, and stays narrower than
  // refresh(): project list, repository kinds, and active detail — no audits, profiles,
  // readiness probes, inventory, or loading indicator.
  const start = workspace.indexOf("const reconcileExternalChanges = async");
  const end = workspace.indexOf("\n  const selectProject =", start);
  assert.ok(start >= 0 && end > start, "missing reconcileExternalChanges");
  const reconcile = workspace.slice(start, end);
  assert.match(reconcile, /if \(loading \|\| busy \|\| !hostIdentityShellReady\) return;/);
  assert.match(reconcile, /listProjects\(\)/);
  assert.match(reconcile, /listProjectRepositoryKinds\(\)/);
  assert.match(reconcile, /getProject\(activeProjectId\)/);
  assert.doesNotMatch(reconcile, /executeProviderMaintenanceAction|listProviderProfiles|listBindings|getReadiness|getInventory|setLoading/);
  assert.match(reconcile, /shellRequest\.current !== shellRequestId/);
  assert.match(reconcile, /hostWorkflowEpoch\.current !== workflowEpoch/);
  assert.match(reconcile, /await refresh\(workflowEpoch\)/);
  assert.match(workspace, /Date\.now\(\) - blurredAt >= FOCUS_RECONCILE_MIN_AWAY_MS/);
});

/* The sidebar is one fixed head over one elastic list, so what it demands of a
   window is a constant rather than a sum of pane floors. Three tiles parted by
   the frame gutter, and only the middle one grows — which is what has to hold
   for the 800x560 floor in tauri.conf.json to fit at all. It replaced a
   three-pane split that asked for 694px there and got 544. */
test("the sidebar fits the smallest window it can be opened at", () => {
  const css = readFileSync("src/App.css", "utf8");
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
    app: { windows: { minHeight: number }[] };
  };

  // No handle and no stored height: the same fixed rows at every window size.
  assert.match(css, /\.sync-sidebar-head \{[^}]*flex: 0 0 auto;/);
  assert.match(css, /\.sync-sidebar-brand \{[^}]*height: 48px;[^}]*flex: 0 0 48px;/);
  assert.match(css, /\.sidebar-root-tab \{[^}]*height: 40px;/);
  assert.match(css, /\.sidebar-root-tabs \{[^}]*padding: 0 8px 6px;/);
  // The rail takes what is left and its own list scrolls, so the window's
  // height reaches the rows instead of a tray wrapped around them.
  assert.match(css, /\.sync-sidebar-rail \{[^}]*min-height: 0;[^}]*flex: 1;/);
  assert.match(css, /\.sidebar-rail-list \{[^}]*flex: 1;[^}]*overflow-y: auto;/);

  const GUTTER = 8;  // --retro-gutter: the frame, and the seam between tiles
  // The taller head is macOS's: 30 lights strip + 48 brand bar + 46 tab
  // strip + 4 border. Elsewhere the strip is 0 and the list gains 30px.
  const HEAD = 128;
  const MARK = 44;   // .sync-sidebar-product-mark min-height
  const minHeight = config.app.windows[0].minHeight;
  assert.equal(minHeight, 560);
  const list = minHeight - (2 * GUTTER + HEAD + 2 * GUTTER + MARK);
  assert.equal(list, 356, "the rail's height at the window floor");

  // Nothing is drag-sized or remembered between launches any more: the panes,
  // their floors, the handles between them, and the keys that stored the split.
  assert.doesNotMatch(css, /sidebar-split-pane|sidebar-section-divider|sidebar-settings-sections/);
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  assert.doesNotMatch(sidebar, /projectShare|storageHeight|collapsedSections|MIN_PANE_HEIGHT/);
  assert.doesNotMatch(sidebar, /mallard\.sidebar-section/);
});

test("custom profile paths resolve this user's home", () => {
  assert.equal(expandHomePath("~", "/Users/ada"), "/Users/ada");
  assert.equal(expandHomePath("~/work/codex-home", "/Users/ada"), "/Users/ada/work/codex-home");
  assert.equal(expandHomePath("~/work", "/Users/ada/"), "/Users/ada/work", "a trailing slash must not double up");
  assert.equal(expandHomePath("  ~/work  ", "/Users/ada"), "/Users/ada/work");
  assert.equal(expandHomePath("/opt/codex", "/Users/ada"), "/opt/codex", "absolute paths pass through");
  assert.equal(expandHomePath("~other/work", "/Users/ada"), "~other/work", "only this user's home expands");
  assert.equal(expandHomePath("~/work", null), "~/work", "unresolved home leaves the text alone");
});

test("the custom path field reports what the finish-time probe would say", () => {
  const codex = { display_name: "Codex", default_profile_directory: ".codex" };
  const profiles = [{ profile_id: "profile-1", display_name: "Default Codex" }];
  const home = "/Users/ada";
  const probe = (over: Record<string, unknown> = {}) => ({
    status: "ok" as const,
    probe: {
      provider: "codex",
      requested_path: "/Users/ada/work",
      resolved_path: "/Users/ada/work/.codex",
      canonical_path: "/Users/ada/work/.codex",
      suggested_name: "work",
      readable: true,
      writable: true,
      detected_child: false,
      existing_profile_id: null,
      ...over,
    },
  } as ProfilePathProbeState);

  // Nothing typed yet: guidance, not a verdict.
  const empty = profilePathHint(undefined, "", home, codex, profiles);
  assert.equal(empty.tone, "mute");
  assert.match(empty.message, /Type or paste a path, or Browse/);

  // Mid-type relative text is not an error — it is not a path yet.
  assert.equal(profilePathHint(undefined, "work/", home, codex, profiles).tone, "mute");
  assert.equal(profilePathHint({ status: "checking" }, "~/work", home, codex, profiles).tone, "mute");

  // A miss while typing reads as a sentence, never an errno.
  const missing = profilePathHint(
    { status: "error", message: "inspect provider profile '/Users/ada/wo': No such file or directory (os error 2)" },
    "~/wo", home, codex, profiles,
  );
  assert.deepEqual({ tone: missing.tone, message: missing.message }, { tone: "bad", message: "No folder at this path yet." });

  // Every other refusal is already written for people: keep the sentence,
  // drop the internal call prefix.
  const symlink = profilePathHint(
    { status: "error", message: "provider profile '/Users/ada/link' cannot be a symlink; choose the resolved directory" },
    "~/link", home, codex, profiles,
  );
  assert.equal(symlink.tone, "bad");
  assert.match(symlink.message, /cannot be a symlink; choose the resolved directory/);
  assert.doesNotMatch(symlink.message, /^inspect provider profile/);

  // A path that is already a profile reuses it rather than making a twin.
  const known = profilePathHint(probe({ existing_profile_id: "profile-1" }), "~/.codex", home, codex, profiles);
  assert.equal(known.tone, "ok");
  assert.match(known.message, /Already set up as “Default Codex”/);

  // The probe descends into the provider's own directory; say so, since the
  // resolved path is not what was typed.
  const descended = profilePathHint(probe({ detected_child: true }), "~/work", home, codex, profiles);
  assert.deepEqual(
    { tone: descended.tone, path: descended.path },
    { tone: "ok", path: "/Users/ada/work/.codex" },
  );
  assert.match(descended.message, /Found \.codex inside/);

  // Unreadable is a warning with the path, not a hard refusal: the folder is
  // real, its permissions are the problem.
  const unreadable = profilePathHint(probe({ readable: false }), "~/work", home, codex, profiles);
  assert.deepEqual(
    { tone: unreadable.tone, path: unreadable.path },
    { tone: "warn", path: "/Users/ada/work/.codex" },
  );

  const plain = profilePathHint(probe(), "~/work", home, codex, profiles);
  assert.deepEqual({ tone: plain.tone, path: plain.path }, { tone: "ok", path: "/Users/ada/work/.codex" });

  // The line is styled by tone and carries the path in the mono face.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-setup-path-hint \{[^}]*grid-column: 2 \/ -1;/s);
  assert.match(css, /\.sync-setup-path-hint\.bad \{ color: var\(--danger-text\); \}/);
  assert.match(css, /\.sync-setup-path-hint code \{[^}]*font-family: var\(--font-mono\);/s);
});

test("checkboxes and radios take the app's accent, not the OS default", () => {
  // Unset, these fall through to the system accent — macOS blue on a sepia
  // page. One rule covers every one of them.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(
    css,
    /input\[type="checkbox"\],\s*input\[type="radio"\] \{\s*accent-color: var\(--accent\);\s*\}/,
  );
  assert.doesNotMatch(
    css,
    /\.log-type-filter-option input \{[^}]*accent-color/s,
    "the global rule replaced the one-off",
  );
});

test("only the log folder is opened with openPath; everything else reveals", () => {
  // `opener:allow-open-path` is scoped to $HOME/.mallard/logs in
  // src-tauri/capabilities/default.json, so open_path on any other path is
  // refused by the capability layer. That refusal is a rejected promise, which
  // is invisible unless it is caught — a menu item that silently does nothing.
  // `reveal_item_in_dir` carries no scope check and is what the menus use.
  const files = [
    "src/components/project-sync/ProjectLinksWorkspace.tsx",
    "src/components/project-sync/ProjectSync.tsx",
    "src/components/storage-catalog/StorageCatalogPage.tsx",
    "src/components/project-sync/ProjectSetupWorkspace.tsx",
  ];
  for (const file of files) {
    assert.doesNotMatch(
      readFileSync(file, "utf8"),
      /\bopenPath\b/,
      `${file} must reveal rather than open: openPath is out of capability scope here`,
    );
  }

  // The one in-scope caller keeps it, and still handles the rejection.
  const settings = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  assert.match(settings, /await openPath\(await projectSyncApi\.getActivityLogFolder\(\)\)/);
  assert.match(settings, /catch \(reason\) \{\s*setLogsError\(errorMessage\(reason\)\)/);

  // Every reveal reports its failure instead of dropping it.
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(workspace, /void revealItemInDir\(path\)\.catch\(/);
  assert.doesNotMatch(workspace, /void revealItemInDir\([^)]*\);(?!\s*\/\/)/);
});

test("the empty storage slot is named, not marked as a fault", () => {
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const band = setupWorkspace.slice(
    setupWorkspace.indexOf("<div><strong>{t(\"sections.storage\")}</strong></div>"),
    setupWorkspace.indexOf("{storageSection?.message && storageSection.state === \"blocked\""),
  );
  assert.ok(band.length > 0, "missing the Storage band");
  assert.equal(en.projectSetup.sections.storage, "Storage");

  // "Local only" was a mode standing among proper nouns, so the row read as
  // two answers to two different questions. The slot is named the way the
  // rest of the app names it — the projects landing card says "No storage
  // linked", the catalog crumb "No storage" — and `yet` carries the fact that
  // Link storage… is on the project's own menu afterwards.
  assert.match(band, /<span className="setup-pill-label">\{t\("setup\.noneYet"\)\}<\/span>/);
  assert.equal(en.projectSetup.setup.noneYet, "None yet");
  assert.doesNotMatch(setupWorkspace, /Local only/);
  assert.doesNotMatch(JSON.stringify(en.projectSetup), /Local only/);

  // The glyph still says where the sessions live, which is why the label does
  // not have to.
  assert.match(band, /<Icon name="computer" size=\{12\} \/>\s*<span className="setup-pill-label">\{t\("setup\.noneYet"\)\}/);

  // A section message prints only when the state is `blocked`, so an
  // `attention` badge here could never say what it wanted — and `attention`
  // on this section means only "no storage linked", which the row states as
  // its own answer. A badge on the safe choice is a fault marker with no
  // fault behind it.
  assert.match(band, /storageSection\?\.state === "blocked" \? sectionBadge\(storageSection\) : null/);
});


test("Connect completes without preparing or applying Pull", async () => {
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const connectStart = source.indexOf("  const connectStorageBundle = async (");
  const connectEnd = source.indexOf("\n  const keepPendingBundle = async", connectStart);
  assert.ok(connectStart >= 0 && connectEnd > connectStart);
  const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
  const handler = transformSync(source.slice(connectStart, connectEnd), { loader: "ts" }).code;

  for (const scenario of ["active", "no-binding", "inactive-binding", "connect-failed", "refresh-failed", "host-changed"] as const) {
    const project = { local_project_id: "project-fixture", bundle_id: "bundle-fixture" };
    const binding = scenario === "no-binding" ? null : {
      ...project, state: scenario === "inactive-binding" ? "unconfirmed" : "active",
    };
    const calls: string[] = [];
    const errors: string[] = [];
    const epoch = { current: 1 };
    const config = { links: [] };
    let busy = false;
    let closed = false;
    const inFlight = { current: false };
    const unexpectedPull = () => assert.fail("Link must not prepare or apply Pull");
    const context = {
      registrations: [project], connectionInFlight: inFlight, hostWorkflowEpoch: epoch,
      setBusy: (value: boolean) => { busy = value; },
      setError: (error: string | null) => { if (error) errors.push(error); },
      setPendingBundleConnection: (value: unknown) => { assert.equal(value, null); closed = true; calls.push("close"); },
      setActiveProjectId: () => undefined, setActiveStorageByProject: () => undefined,
      loadShell: async () => ({ nextConfig: config, hostIdentityReady: true }),
      loadProjectData: async () => {
        calls.push("refresh");
        if (scenario === "refresh-failed") throw new Error("Refresh failed");
        if (scenario === "host-changed") epoch.current += 1;
      },
      notify: (message: string) => { assert.equal(message, "Storage linked."); calls.push("notify"); },
      t: i18n.getFixedT(null, ["shell", "common"]),
      errorMessage: (reason: unknown) => String(reason),
      planRestore: unexpectedPull,
      projectSyncApi: {
        connectProjectToRemoteBundle: async (request: unknown) => {
          calls.push("connect");
          assert.equal(busy, true);
          assert.deepEqual(request, {
            local_project_id: project.local_project_id, storage_id: "storage-fixture", bundle_id: project.bundle_id,
            expected_bundle_id: project.bundle_id, pinned: true, allow_repository_mismatch: false,
          });
          if (scenario === "connect-failed") throw new Error("Connection failed");
          return { project, binding };
        },
        planRestore: unexpectedPull,
        getRestoreReadiness: unexpectedPull,
        applyRestore: unexpectedPull,
      },
    };
    const run = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handler}; return connectStorageBundle("project-fixture", "storage-fixture", "bundle-fixture");`);
    await run(context);
    assert.equal(inFlight.current, false);
    if (scenario !== "host-changed") assert.equal(busy, false);
    if (scenario === "active" || scenario === "no-binding" || scenario === "inactive-binding") {
      assert.deepEqual(calls, ["connect", "refresh", "close", "notify"]);
      assert.equal(closed, true);
      assert.deepEqual(errors, []);
    } else if (scenario === "host-changed") {
      assert.equal(closed, false);
      assert.deepEqual(calls, ["connect", "refresh"]);
      assert.deepEqual(errors, []);
    } else {
      assert.equal(closed, false, "keep failures visible in the chooser");
      assert.deepEqual(calls, scenario === "connect-failed" ? ["connect"] : ["connect", "refresh"]);
      assert.deepEqual(errors, [scenario === "connect-failed" ? "Error: Connection failed" : "Error: Refresh failed"]);
    }
  }
});

test("link reloads skip sibling metadata checks and later project loads check again", async () => {
  const source = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const section = (start: string, end: string) => {
    const startAt = source.indexOf(start);
    const endAt = source.indexOf(end, startAt);
    assert.ok(startAt >= 0 && endAt > startAt);
    return source.slice(startAt, endAt);
  };
  const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
  const handlers = transformSync([
    section("  const loadProjectData = async (", "\n  useEffect(() => {"),
    section("  const saveStorageLink = async (", "\n  const linkStorage = async"),
    section("  const connectStorageBundle = async (", "\n  const keepPendingBundle = async"),
  ].join("\n"), { loader: "ts" }).code;

  for (const operation of ["saveStorageLink", "connectStorageBundle"] as const) {
    const projectId = "project-fixture";
    const storageId = "storage-fixture";
    const bundleId = "bundle-fixture";
    const project = { local_project_id: projectId, bundle_id: bundleId };
    const config = { links: [{ local_project_id: projectId, storage_id: storageId }] };
    const detail = { project, binding: { ...project, state: "active" }, links: config.links };
    const inventory = { resources: [] };
    const sibling = { state: "sibling_found", matches: [] };
    const calls: string[] = [];
    const errors: string[] = [];
    let siblingNotice: unknown = { projectId, storageId, status: sibling };
    let shownInventory: unknown = null;
    let loading = false;
    let busy = false;
    const context = {
      config, registrations: [project], activeStorageByProject: {},
      hostWorkflowEpoch: { current: 1 }, projectRequest: { current: 0 }, connectionInFlight: { current: false },
      setLoading: (value: boolean) => { loading = value; },
      setBusy: (value: boolean) => { busy = value; },
      setError: (error: string | null) => { if (error) errors.push(error); },
      setDetail: () => undefined, setRegistrations: () => undefined, setBindings: () => undefined,
      setActiveProjectId: () => undefined, setActiveStorageByProject: () => undefined,
      setStorageNotice: () => undefined, setPendingBundleConnection: () => undefined,
      setInventory: (value: unknown) => { shownInventory = value; },
      setSiblingNotice: (value: unknown) => { siblingNotice = value; },
      loadShell: async () => ({ nextConfig: config, hostIdentityReady: true }),
      notify: (message: string) => assert.equal(message, "Storage linked."),
      t: i18n.getFixedT(null, ["shell", "common"]),
      errorMessage: (reason: unknown) => String(reason),
      planRestore: () => assert.fail("Link must not prepare Pull"),
      projectSyncApi: {
        saveLink: async () => { calls.push("save"); },
        connectProjectToRemoteBundle: async () => { calls.push("connect"); return detail; },
        getProject: async () => { calls.push("project"); return detail; },
        getInventory: async () => { calls.push("inventory"); return inventory; },
        getSiblingBundleStatus: async () => { calls.push("siblings"); return sibling; },
      },
    };
    const link = operation === "saveStorageLink"
      ? 'saveStorageLink("project-fixture", "storage-fixture")'
      : 'connectStorageBundle("project-fixture", "storage-fixture", "bundle-fixture")';
    const run = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${handlers}; return {
      link: () => ${link},
      reload: () => loadProjectData("project-fixture"),
    };`)(context);
    await run.link();
    const saved = operation === "saveStorageLink" ? "save" : "connect";
    assert.deepEqual(calls, [saved, "project", "inventory"], operation);
    assert.equal(siblingNotice, null, "clear the old advisory notice after linking");
    assert.equal(shownInventory, inventory, "local inventory still loads");
    assert.equal(loading, false);
    assert.equal(busy, false);
    assert.deepEqual(errors, []);

    await run.reload();
    assert.deepEqual(calls, [saved, "project", "inventory", "project", "inventory", "siblings"]);
    assert.deepEqual(siblingNotice, { projectId, storageId, status: sibling });
    assert.equal(loading, false);
    assert.deepEqual(errors, []);
  }
});

test("the connection flow has no separate Finish step or persisted schema change", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const dialog = readFileSync("src/components/project-sync/BundleConnectionDialog.tsx", "utf8");
  assert.doesNotMatch(workspace + dialog, /ConnectionCheckBody|continueConnection|retryConnectionCheck|Finish connection|requireReview/);
  assert.doesNotMatch(JSON.stringify(en.shell), /Finish connection/, "nor in the shell's catalog");
  assert.match(dialog, /t\("connect\.connect"\)/);
  assert.match(readFileSync("src/i18n/en/reviews.ts", "utf8"), /\n {4}connect: "Connect",\n/);
  assert.match(workspace, /await connectStorageBundle\(projectId, storageId, project\.bundle_id\)/, "same-bundle local storage links use the same connection flow");
  assert.equal(PROJECT_SYNC_SCHEMA, 1);
});

test("only a Codex deletion rejected with -32600 offers Close Codex", () => {
  assert.equal(localSessionDeleteError("codex", { provider_error: CODEX_DELETE_ERROR, metadata_error: null }), codexDeleteCloseNotice());
  assert.equal(codexDeleteCloseNotice(), "Delete failed. Close Codex, then try Delete again.");
  // The notice's Close Codex button follows it in either language.
  assert.equal(offersCloseCodex(`Deleted 2 of 3 sessions. ${codexDeleteCloseNotice()}`), true);
  assert.equal(offersCloseCodex("删除失败。请关闭 Codex，然后再点一次“删除”。"), true);
  assert.equal(offersCloseCodex("Session files: disk full"), false);
  assert.equal(localSessionDeleteError("claude", { provider_error: CODEX_DELETE_ERROR, metadata_error: null }), `Session files: ${CODEX_DELETE_ERROR}`);
  assert.equal(localSessionDeleteError("codex", { provider_error: "Codex provider request failed (code -32601).", metadata_error: "Disk full" }),
    "Session files: Codex provider request failed (code -32601). Mallard binding: Disk full");
  assert.equal(localSessionDeleteError("codex", { provider_error: null, metadata_error: null }), null);
});

test("project session deletion requires warning approval and replaces metadata-only reset", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const start = workspace.indexOf("  async function deleteLocalProjectSessions(");
  assert.ok(start >= 0);
  const action = workspace.slice(start, workspace.indexOf("  // App commands", start));
  const approval = action.indexOf("if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;");
  const request = action.indexOf("await projectSyncApi.deleteLocalProjectSessions(");
  assert.ok(action.indexOf("await confirm(") < approval && approval < request);
  assert.match(action, /kind: "warning", okLabel: t\("confirm\.deleteSessionsOk"\), cancelLabel: t\("common:cancel"\)/);
  assert.equal(en.shell.confirm.deleteSessionsOk, "Delete all");
  assert.match(en.shell.confirm.deleteSessions, /Project files and cloud copies are kept\. This cannot be undone\.$/);
  assert.match(action, /result\.provider_errors/);
  assert.match(action, /result\.metadata_error/);
  assert.ok(action.indexOf("await refresh(workflowEpoch)") < action.indexOf("setError(["));
  const menu = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  assert.match(menu, /label: t\("projectMenu\.deleteSessions"\)/);
  assert.equal(en.projectPage.projectMenu.deleteSessions, "Delete all local sessions…");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const commands = readFileSync("src-tauri/src/lib.rs", "utf8");
  assert.match(api, /invoke<ProjectSessionsDeleteResult>\("delete_local_project_sessions"/);
  assert.match(commands, /commands::delete_local_project_sessions,/);
  assert.doesNotMatch(api + commands, /reset_project_sync_metadata/);
});

test("setup pages list storages Mallard Cloud first, then the other clouds, then local, and open on the first", () => {
  // The registry's display rank is the preference; the name decides among
  // equals; a provider this build does not register sorts last.
  const providers = [
    { key: "local", display_rank: 30 },
    { key: "r2", display_rank: 10 },
    { key: "managed_cloud", display_rank: 5 },
    { key: "dropbox", display_rank: 20 },
  ] as StorageProviderDescriptor[];
  const storages = [
    { storage_id: "b", name: "Backup drive", provider: "local" },
    { storage_id: "v", name: "Vault", provider: "r2" },
    { storage_id: "c", name: "Mallard Cloud", provider: "managed_cloud" },
    { storage_id: "d", name: "Drop", provider: "dropbox" },
    { storage_id: "a", name: "Archive", provider: "local" },
    { storage_id: "x", name: "Mystery", provider: "unknown" },
  ];
  assert.deepEqual(orderStorages(storages, providers).map((storage) => storage.storage_id), ["c", "v", "d", "a", "b", "x"]);
  assert.deepEqual(storages.map((storage) => storage.storage_id), ["b", "v", "c", "d", "a", "x"], "the input is not reordered in place");

  // The shell orders once, and every setup page lists what it is given: the
  // draft's setup page, New project (its Cloud door included), and Add
  // Catalog resource.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /const setupStorages = useMemo\(\s*\(\) => orderStorages\(config\.storages, storageProviders\),/);
  assert.equal((shell.match(/storages=\{setupStorages\}/g) ?? []).length, 2);
  assert.match(shell, /const catalogStorages: CatalogStorageOption\[\] = setupStorages\.map\(/);

  // A new draft opens on the first by the same rule, seeded where the
  // profiles are seeded; "None yet" stays one click away.
  const rust = readFileSync("src-tauri/src/project_sync/service.rs", "utf8");
  assert.match(
    rust,
    /let descriptors = storage_provider_descriptors\(\);[\s\S]*?\.min_by\(\|left, right\| \{[\s\S]*?\.cmp\(&right\.0\)[\s\S]*?then_with\(\|\| left\.1\.name\.cmp\(&right\.1\.name\)\)/,
  );
  assert.doesNotMatch(rust, /config\.storages\.len\(\) == 1/);
});

/** The project page with a Git project on an R2 storage whose Push just
 *  failed: the path bar, both crumb menus (rendered closed, so present in the
 *  markup), Pull and Push, and the storage card's notice. */
function renderProjectPage(): string {
  const noop = () => undefined;
  const project = (overrides: object) => ({
    local_project_id: "p-git", bundle_id: "b", display_name: "repo-one", local_alias: null, revision: 1,
    project_root: "/Users/me/code/repo-one", profile_ids: { codex: "profile-codex" }, linked_storage_ids: ["s-attic"],
    is_git_repository: true, ...overrides,
  });
  const props = {
    projects: [
      project({}),
      project({ local_project_id: "p-notes", display_name: "潮汐笔记", project_root: "/Users/me/notes", is_git_repository: false }),
    ],
    activeProjectId: "p-git", activeStorageId: "s-attic", rememberedStorageByProject: {},
    activeProjectSubview: { kind: "overview" },
    bindings: [{
      host_id: "h", replica_id: "r", local_project_id: "p-git", bundle_id: "b", project_root: "/Users/me/code/repo-one",
      canonical_project_root: "/Users/me/code/repo-one", profile_ids: { codex: "profile-codex" }, state: "active", revision: 1, updated_at: 1,
    }],
    profiles: [{
      profile_id: "profile-codex", provider: "codex", display_name: "Work Codex", path: "/Users/me/.codex", canonical_path: "/Users/me/.codex",
      revision: 1, created_at: 1, updated_at: 1, available: true, readable: true, writable: true, used_by_projects: ["p-git"],
    }],
    storageProviders: [],
    storages: [
      { storage_id: "s-attic", name: "Attic", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [], settings: {}, secrets: [] },
      { storage_id: "s-drive", name: "Backup drive", provider: "local", config_version: 1, provider_label: "Local folder", icon: "drive", available: true, summary: [], settings: {}, secrets: [] },
    ],
    storageSetupDrafts: [],
    links: [{
      local_project_id: "p-git", storage_id: "s-attic", bundle_id: "b", project_content_preferences: {},
      conversation_checkpoints: {}, pinned: false, created_at: 1,
    }],
    loading: false, busy: false,
    onSelectStorage: noop, onOpenSessionDetail: noop, onSessionActionError: noop, onNotify: noop, onCloseSessionDetail: noop,
    onLinkStorage: noop, onUnlinkStorage: noop, onPush: noop, onPull: noop, onConfigureBinding: noop,
    onRenameProject: () => ({ ok: true, message: null }), onDeleteProjectSessions: noop, onRefresh: noop, onAddProject: noop,
    onAddCloudProject: noop, onSelectProject: noop, appActions: [], onAddStorage: noop, onCreateStorageDraft: async () => undefined,
    onOpenStorage: noop, onOpenStorageIndex: noop, onClearStorage: noop, onRemoveStorage: noop, onRenameStorage: async () => true,
    onCommitStorageDraft: async () => null, onTestStorageDraft: async () => true, onTestStorage: async () => true,
    storageDraftSaveQueue: new StorageDraftSaveQueue(),
    storageNotice: {
      projectId: "p-git", storageId: "s-attic", kind: "action_failed", action: "push",
      scope: { kind: "project" }, message: "HTTP 503: upstream unavailable",
    },
  } as unknown as Parameters<typeof ProjectLinksWorkspace>[0];
  return renderToStaticMarkup(<ProjectLinksWorkspace {...props} />);
}

test("the project page's menus and storage card speak Chinese; names, paths and the backend's sentence stay as given", () => {
  const english = renderProjectPage();
  assert.match(english, /aria-label="Attic\. Storage menu"/);
  assert.match(english, /aria-label="Unlink from repo-one…"/);
  assert.match(english, /aria-label="repo-one\. Project menu"/);
  assert.match(english, /aria-label="Rename project…"/);
  assert.match(english, /title="\/Users\/me\/code\/repo-one\nCodex agent home: \/Users\/me\/\.codex"/);
  assert.match(english, /aria-label="Pull from Attic"/);
  assert.match(english, /aria-label="Push to Attic"/);
  assert.match(english, /Push couldn&#x27;t start/);

  void i18n.changeLanguage("zh-CN");
  try {
    const html = renderProjectPage();
    assert.match(html, /<ol class="sync-path" aria-label="repo-one 路径"/);
    // The storage crumb and its menu.
    assert.match(html, /title="Attic · Cloudflare R2" aria-label="Attic。存储菜单"/);
    assert.match(html, /role="menu" aria-label="存储"/);
    assert.match(html, /aria-label="存储详情…"/);
    assert.match(html, /aria-label="取消与 repo-one 的关联…"/);
    assert.match(html, /title="停止同步此关联；存储中已有的文件会保留"/);
    assert.match(html, /关联存储…/);
    assert.match(html, /添加存储…/);
    // The project crumb and its menu; the other project's name is its own.
    assert.match(html, /aria-label="repo-one。项目菜单"/);
    assert.match(html, /title="\/Users\/me\/code\/repo-one\nCodex 代理主目录：\/Users\/me\/\.codex"/);
    assert.match(html, /role="menu" aria-label="项目"/);
    for (const label of ["在访达中显示", "代理配置…", "重命名项目…", "删除所有本地会话…", "切换到"]) {
      assert.match(html, new RegExp(`aria-label="${label}"`));
    }
    assert.match(html, /title="删除此项目在本机的会话和子代理，并清除其会话绑定和本地同步状态"/);
    assert.match(html, /添加项目…/);
    assert.match(html, /潮汐笔记/);
    assert.match(html, /Work Codex/);
    // Pull and Push.
    assert.match(html, /role="group" aria-label="同步操作"/);
    assert.match(html, /title="先检查下载内容，再应用" aria-label="从 Attic 下载"/);
    assert.match(html, /title="选择要上传到此存储的内容" aria-label="上传到 Attic"/);
    assert.match(html, /<span class="sync-path-sync-label">下载<\/span>/);
    assert.match(html, /<span class="sync-path-sync-label">上传<\/span>/);
    // The storage card: Chinese frame, the backend's sentence unchanged.
    assert.match(html, /aria-label="repo-one 的存储"/);
    assert.match(html, /上传未能开始/);
    assert.match(html, /HTTP 503: upstream unavailable/);
    assert.match(html, />重试<\/button>/);
    assert.doesNotMatch(html, /Storage menu|Project menu|Storage details|Unlink from|Rename project|Agent profile…|Delete all local sessions|Reveal in Finder|Sync actions|Pull from|Push to|couldn&#x27;t start|>Retry</);
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.equal(renderProjectPage(), english, "back in English, the page is the English one again");
});

test("a storage's repository rows and the project page's phrases speak Chinese; repository and backend names stay as given", () => {
  const bundle = {
    bundle_id: "bundle-1", display_name: "mallard-app", revision: 3, resource_count: 9,
    updated_at: Date.now() - 29 * 60 * 1000, providers: ["codex", "claude"], repository_fingerprint: "3f9c1a77",
  } as unknown as Parameters<typeof StorageRepositoryRow>[0]["bundle"];
  const repo = {
    repository_id: "library-1", provider: "codex", provider_label: "Codex", kind_label: "Skills",
    revision: null, updated_at: null, resource_names: [], error: "library tag does not match", capability: null,
  } as unknown as CatalogStorageRepositoryView;
  const englishRow = renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />);
  assert.match(englishRow, /aria-label="Show details for mallard-app"/);

  void i18n.changeLanguage("zh-CN");
  try {
    const row = renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />);
    assert.match(row, /<strong class="sync-storage-repository-name">mallard-app<\/strong>/);
    assert.match(row, /<span>Version<\/span><b>3<\/b>/);
    assert.match(row, /<span>更新时间<\/span><b>29分钟前<\/b>/);
    assert.match(row, /aria-label="显示 mallard-app 的详情" title="显示 mallard-app 的详情"/);
    const unnamed = renderToStaticMarkup(<StorageRepositoryRow bundle={{ ...bundle, display_name: "" }} />);
    assert.match(unnamed, /<strong class="sync-storage-repository-name">未命名仓库<\/strong>/);
    assert.match(unnamed, /aria-label="显示未命名仓库的详情"/);

    const details = renderToStaticMarkup(<StorageRepositoryDetails bundle={bundle} />);
    for (const term of ["仓库 ID", "更新时间", "资源", "代理", "指纹"]) assert.match(details, new RegExp(`<dt>${term}</dt>`));
    assert.match(details, /<dd>Codex · Claude<\/dd>/);

    // The kind and agent labels arrive from the backend and stay as they are.
    const unreadable = renderToStaticMarkup(<CatalogStorageRepositoryRow repo={repo} />);
    assert.match(unreadable, /Skills · Codex/);
    assert.match(unreadable, /sync-storage-repo-unreadable[^>]*>无法读取</);
    assert.match(unreadable, /aria-label="显示 Skills · Codex 的详情"/);

    assert.equal(projectSyncCountPhrase(2, 0), "2 个较新的会话");
    assert.equal(projectSyncCountPhrase(0, 1), "1 个已更改的文件");
    assert.equal(projectPushActionLabel({ preparing: false, publishing: true }), "正在上传…");
    assert.equal(projectPullActionLabel({ preparing: true, applying: false }), "正在准备…");
    assert.equal(projectPullActionLabel({ preparing: false, applying: false }), "下载");
    const comparison = { entries: [{ storage_present: true }, { storage_present: true }] } as unknown as Parameters<typeof publishVersionHolds>[0];
    const publication = { latest_version: 3, draft_ahead: 2 } as unknown as Parameters<typeof publishVersionHolds>[2];
    assert.deepEqual(publishVersionHolds(comparison, null, publication).map((fact) => fact.word), ["2 个会话", "Version 3 之后有 2 次上传"]);
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.equal(renderToStaticMarkup(<StorageRepositoryRow bundle={bundle} />), englishRow);
});

test("the app menu switches language beside the theme, naming the other language in itself", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const menu = workspace.slice(workspace.indexOf("const appActions: PathBarMenuGroup[]"), workspace.indexOf("const windowActions"));
  // After Dark theme, before Reload: the other offered language, named in
  // itself with its own `lang`, picked outright (stepping out of System).
  assert.ok(menu.indexOf('id: "theme"') < menu.indexOf('id: "language"') && menu.indexOf('id: "language"') < menu.indexOf('id: "reload"'));
  assert.match(menu, /OFFERED_LANGUAGES\.filter\(\(language\) => language !== uiLanguage\(\)\)/);
  assert.match(menu, /label: LANGUAGE_NAMES\[language\],\s*lang: language,\s*icon: "language" as const,/);
  assert.match(menu, /onSelect: \(\) => \{ setLanguageChoice\(language\); \}/);
  assert.match(readFileSync("src/components/sync-review/SyncPathBar.tsx", "utf8"), /<span lang=\{item\.lang\}>\{item\.label\}<\/span>/);
  assert.equal(en.shell.window.switchLanguage, "Switch the interface to {{language}}");
});
