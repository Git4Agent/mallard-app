import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StartupLoadingScreen from "../../src/components/project-sync/StartupLoadingScreen";
import { repositoryFingerprintMatches } from "../../src/components/project-sync/model";
import { PROJECT_SYNC_SCHEMA } from "../../src/types";

test("ProjectSync is the only application interface", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  assert.match(app, /<ProjectSync/);
  assert.doesNotMatch(app, /ProjectSyncV\d/);
  assert.doesNotMatch(app, /LegacyApp|onOpenLegacy|mode.*legacy|SyncPanel|FilesWorkspace|FinishSetup/);

  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.doesNotMatch(`${sidebar}\n${workspace}`, /Legacy profiles|Open legacy|onOpenLegacy/);

  for (const removed of [
    "src/components/SyncPanel.tsx",
    "src/components/FilesWorkspace.tsx",
    "src/components/FileTree.tsx",
    "src/components/FilePreview.tsx",
    "src/components/FinishSetup.tsx",
  ]) {
    assert.equal(existsSync(removed), false, `${removed} must remain removed`);
  }
});

test("the current frontend contract is schema 1 without version-branded DTOs", () => {
  assert.equal(PROJECT_SYNC_SCHEMA, 1);
  const types = readFileSync("src/types.ts", "utf8");
  assert.doesNotMatch(types, /SyncConfigV\d|StorageConfigV\d/);
});

test("project setup does not offer an immediate push", () => {
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");

  assert.doesNotMatch(setupWorkspace, /Finish & push|finalize\("push"\)/);
});

test("project setup accepts only the current canonical repository fingerprint", () => {
  assert.equal(repositoryFingerprintMatches("canonical", "canonical"), true);
  assert.equal(repositoryFingerprintMatches("legacy-dot-git", "canonical"), false);
  assert.equal(repositoryFingerprintMatches("different-repository", "canonical"), false);
  assert.equal(repositoryFingerprintMatches(null, "canonical"), false);
});

test("the production Tauri handler exposes only current sync and shared log commands", () => {
  const backend = readFileSync("src-tauri/src/lib.rs", "utf8");
  assert.match(backend, /project_sync::commands::get_project_sync_config/);
  assert.doesNotMatch(backend, /project_sync_v\d/);
  assert.match(backend, /activity_log::query_activity_logs/);
  assert.doesNotMatch(
    backend,
    /list_config_dirs|read_file_content|write_file_content|get_sync_config,|save_sync_config,|sync_upload|sync_download|get_setup_readiness|setup_link/,
  );
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
  assert.match(css, /\.sync-sidebar-toggle \{[^}]*top: 14px;[^}]*left: 86px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar \{[^}]*padding-left: 140px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar::before \{[^}]*left: 123px;/);
  assert.match(css, /\.sync-titlebar \{[^}]*height: 52px;[^}]*min-height: 52px;[^}]*flex: 0 0 52px;/);
  assert.match(css, /\.sync-sidebar-drag \{[^}]*height: 38px;/);
  assert.match(css, /\.sync-sidebar-brand \{[^}]*height: 52px;/);
  assert.deepEqual(tauriConfig.app.windows[0].trafficLightPosition, { x: 20, y: 29 });
});

test("the retro skin stays presentation-only and preserves the shell controls", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const startup = readFileSync("src/components/project-sync/StartupLoadingScreen.tsx", "utf8");
  const document = readFileSync("index.html", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");

  assert.match(app, /import "\.\/App\.css";\s*import "\.\/RetroTheme\.css";/);
  assert.equal(existsSync("public/mallard-logo-pixel.png"), true);
  assert.equal(existsSync("public/mallard-logo-pixel-ring.png"), true);
  assert.equal(existsSync("public/mallard-logo-pixel-duck.png"), true);
  assert.equal(existsSync("public/fonts/PixelifySans-Variable.ttf"), true);
  assert.equal(existsSync("public/fonts/PixelifySans-OFL.txt"), true);
  assert.equal(existsSync("THIRD_PARTY_NOTICES.md"), true);
  assert.match(sidebar, /className="sync-brand-logo" src="\/mallard-logo-pixel-ring\.png"/);
  assert.match(sidebar, /className="sync-sidebar-product-mark"[\s\S]*src="\/mallard-logo-pixel-duck\.png"/);
  assert.match(startup, /src="\/mallard-logo-pixel\.png"/);
  assert.match(document, /href="\/mallard-logo-pixel\.png"/);
  assert.match(retroCss, /@font-face\s*\{[\s\S]*font-family: "Pixelify Sans"/);
  assert.match(retroCss, /--font-pixel: "Pixelify Sans", var\(--font\)/);
  assert.match(retroCss, /:root\[data-theme="dark"\] \.sync-brand-logo,[\s\S]*:root\[data-theme="dark"\] \.sync-sidebar-product-mark img,[\s\S]*:root\[data-theme="dark"\] \.sync-startup-mark img\s*\{[^}]*filter: brightness\(0\) invert\(1\);/s);
  assert.match(retroCss, /\.sync-brand-logo,\s*\.sync-sidebar-product-mark img\s*\{[^}]*image-rendering: pixelated;/s);
  assert.match(retroCss, /\.sync-startup-mark img\s*\{[^}]*image-rendering: auto;/s);
  assert.match(retroCss, /\.sync-sidebar-brand strong\s*\{[\s\S]*font-family: var\(--font-pixel\)/);
  assert.match(retroCss, /\.btn\s*\{[^}]*font-family: var\(--font-pixel\)/s);
  assert.match(retroCss, /\.sync-project-heading-name h1\s*\{[^}]*font-family: var\(--font\)/s);
  assert.match(retroCss, /\.sync-project-heading-meta\s*\{[^}]*font: 400 13px\/18px var\(--font\);[^}]*font-kerning: none;[^}]*font-synthesis: none;[^}]*font-variant-ligatures: none;[^}]*letter-spacing: 0;[^}]*overflow: hidden;/s);
  assert.match(retroCss, /\.sync-project-heading-meta-item\s*\{[^}]*font: 400 13px\/18px var\(--font\);[^}]*font-kerning: none;[^}]*font-synthesis: none;[^}]*font-variant-ligatures: none;[^}]*letter-spacing: 0;/s);
  assert.match(retroCss, /\.sync-project-heading-meta-item > span\s*\{[^}]*display: block;[^}]*font: 400 13px\/18px var\(--font\);[^}]*font-kerning: none;[^}]*font-synthesis: none;[^}]*font-variant-ligatures: none;[^}]*letter-spacing: 0;/s);
  assert.match(retroCss, /\.sync-project-heading-meta-item \+ \.sync-project-heading-meta-item::before\s*\{[^}]*content: "";[^}]*width: 1px;[^}]*height: 14px;[^}]*background: var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-titlebar-context strong\s*\{[^}]*font-family: var\(--font\)/s);
  // One surface across the bar: no second panel ending mid-titlebar.
  assert.match(retroCss, /\.sync-titlebar-context\s*\{[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sidebar-profile-main span\s*\{[^}]*font-family: var\(--font\)/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--bg-0: #fefbf6;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--bg-2: #faf3eb;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--surface-row-active: #f0e3d5;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--text-0: #1e1817;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--accent: #2a211d;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--retro-rule: #8f7f74;/s);
  assert.match(retroCss, /:root,\s*:root\[data-theme="dark"\]\s*\{[^}]*--border: #292929;[^}]*--border-mid: #3d3d3d;[^}]*--border-strong: #707070;/s);
  assert.match(retroCss, /:root,\s*:root\[data-theme="dark"\]\s*\{[^}]*--retro-rule: #8a8a8a;[^}]*--retro-frame-edge: #8a8a8a;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--retro-scroll-thumb: #d3d4d7;[^}]*--retro-scroll-thumb-highlight: #dcdddf;[^}]*--retro-scroll-thumb-shadow: rgba\(61, 50, 44, 0\.18\);/s);
  assert.match(retroCss, /::-webkit-scrollbar\s*\{[^}]*width: 6px;[^}]*height: 6px;/s);
  assert.match(retroCss, /::-webkit-scrollbar-track\s*\{[^}]*background: transparent;[^}]*box-shadow: none;/s);
  assert.match(retroCss, /::-webkit-scrollbar-thumb\s*\{[^}]*min-height: 28px;[^}]*min-width: 28px;[^}]*border: 0;[^}]*border-radius: 3px;[^}]*background: var\(--retro-scroll-thumb\);/s);
  assert.match(retroCss, /::-webkit-scrollbar-thumb:vertical\s*\{[^}]*inset -1px 0 0 var\(--retro-scroll-thumb-highlight\),[^}]*0 1px 2px var\(--retro-scroll-thumb-shadow\);/s);
  assert.match(retroCss, /::-webkit-scrollbar-thumb:horizontal\s*\{[^}]*inset 0 -1px 0 var\(--retro-scroll-thumb-highlight\),[^}]*0 1px 2px var\(--retro-scroll-thumb-shadow\);/s);
  assert.match(retroCss, /::-webkit-scrollbar-corner\s*\{[^}]*background: transparent;[^}]*box-shadow: none;/s);
  // The shell frame is a mosaic of rounded tiles floating on one gutter, so a
  // horizontal separator meets a vertical one in rounded corners — never in a
  // square T-joint — and the outer padding reads as the same gutter.
  assert.match(retroCss, /:root,\s*:root\[data-theme="dark"\]\s*\{[^}]*--retro-gutter: 3px;[^}]*--retro-panel-radius: 10px;[^}]*--retro-panel-radius-inner: 9px;/s);
  assert.match(retroCss, /\.sync-app\s*\{[^}]*padding: var\(--retro-gutter\);[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: var\(--retro-frame-gutter\);[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.sync-sidebar\s*\{[^}]*overflow: visible;[^}]*gap: var\(--retro-gutter\);[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: var\(--retro-frame-gutter\);[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.sync-workspace\s*\{[^}]*overflow: visible;[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: var\(--retro-frame-gutter\);[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.sync-sidebar-head,\s*\.sidebar-project-pane,\s*\.sidebar-storage-pane,\s*\.sync-sidebar-product-mark,\s*\.sync-titlebar,\s*\.sync-workspace > \.sync-main,\s*\.sync-log-drawer\.open\s*\{[^}]*border: 1px solid var\(--retro-frame-edge\);[^}]*border-radius: var\(--retro-panel-radius\);/s);
  assert.match(sidebar, /className="sync-sidebar-head"/);
  assert.doesNotMatch(retroCss, /\.sync-(?:sidebar|workspace)::after/);
  assert.match(retroCss, /\.sync-sidebar-resizer\s*\{[^}]*width: 7px;[^}]*margin-right: -2px;[^}]*margin-left: -2px;[^}]*flex: 0 0 7px;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-sidebar-resizer::before\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-sidebar-resizer::after\s*\{[^}]*inset: var\(--retro-panel-radius\) auto var\(--retro-panel-radius\) 2px;[^}]*width: var\(--retro-gutter\);[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-primary-nav\s*\{[^}]*position: relative;[^}]*border-bottom: 0;/s);
  assert.match(retroCss, /\.sync-primary-nav::after\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-titlebar\s*\{[^}]*margin-bottom: var\(--retro-gutter\);/s);
  assert.doesNotMatch(retroCss, /\.sync-titlebar\s*\{[^}]*border-bottom: 1px solid var\(--retro-rule\);/s);
  assert.match(retroCss, /\.sync-sidebar-sections\s*\{[^}]*overflow: visible;[^}]*background: transparent;/s);
  assert.match(readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8"), /gridTemplateRows: `minmax\(96px, \$\{projectShare\}fr\) 3px minmax\(96px, \$\{1 - projectShare\}fr\)`/);
  assert.match(retroCss, /\.sidebar-project-pane\s*\{[^}]*background: var\(--bg-1\);/s);
  assert.match(retroCss, /\.sidebar-storage-pane\s*\{[^}]*background: var\(--bg-1\);/s);
  assert.match(retroCss, /\.sidebar-section-divider\s*\{[^}]*height: 9px;[^}]*margin: -3px 0;[^}]*align-self: center;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sidebar-section-divider::before\s*\{[^}]*display: none;/s);
  // The handles only tint the gutter they straddle, and stop short of the tile
  // corners so the rounded junction keeps its shape while dragging.
  assert.match(retroCss, /\.sidebar-section-divider::after\s*\{[^}]*top: 3px;[^}]*right: var\(--retro-panel-radius\);[^}]*left: var\(--retro-panel-radius\);[^}]*height: var\(--retro-gutter\);[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sidebar-section-divider:hover::after,[\s\S]*?\.sidebar-settings-sections\.resizing \.sidebar-section-divider::after\s*\{[^}]*background: var\(--border-strong\);/s);
  assert.match(workspace, /className=\{`sync-workspace\$\{activityOpen \? " logs-open" : ""\}\$\{resizingLog \? " resizing-log" : ""\}`\}/);
  assert.doesNotMatch(retroCss, /\.sync-workspace\.logs-open > \.sync-main/);
  // A closed drawer collapses its gutter with its height, so nothing is left
  // hanging under the workspace tile.
  assert.match(retroCss, /\.sync-log-drawer\.open\s*\{[^}]*overflow: visible;[^}]*margin-top: var\(--retro-gutter\);[^}]*padding-top: 0;/s);
  assert.match(retroCss, /\.sync-log-drawer \.log-resizer\s*\{[^}]*top: calc\(-1px - var\(--retro-gutter\) - 1px\);[^}]*height: 9px;/s);
  assert.match(retroCss, /\.sync-log-drawer \.log-resizer::before\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-log-drawer \.log-resizer::after\s*\{[^}]*top: 0;[^}]*right: var\(--retro-panel-radius\);[^}]*left: var\(--retro-panel-radius\);[^}]*height: var\(--retro-gutter\);[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-log-drawer\.open::before,\s*\.sync-log-drawer\.open::after\s*\{[^}]*display: none;/s);
  // The drawer tile paints its own rounded background; only the scrolling body
  // needs to clip, so the type filter can still open upward out of the panel.
  assert.match(retroCss, /\.sync-log-drawer \.log-panel\s*\{[^}]*overflow: visible;[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-log-drawer \.log-panel-body\s*\{[^}]*border-radius: 0 0 var\(--retro-panel-radius-inner\) var\(--retro-panel-radius-inner\);/s);
  assert.match(retroCss, /\.sync-log-drawer \.log-panel-header\s*\{[^}]*padding-right: 16px;[^}]*padding-left: 16px;/s);
  assert.doesNotMatch(retroCss, /body\s*\{[^}]*font-family: var\(--font-pixel\)/s);
  // Pixel type needs whole-pixel glyph origins: no fractional tracking, and no
  // geometricPrecision (which tells the rasteriser to skip grid-fitting).
  assert.match(retroCss, /body\s*\{[^}]*letter-spacing: 0;[^}]*text-rendering: auto;/s);
  assert.doesNotMatch(retroCss, /letter-spacing: 0\.006em/);
  assert.doesNotMatch(retroCss, /text-rendering: geometricPrecision/);
  // The pixel face is drawn as hard blocks, so it rasterises unsmoothed.
  assert.match(retroCss, /\.btn,\s*\.sync-sidebar-brand strong,[\s\S]*?\.sync-history-section-heading h2\s*\{\s*-webkit-font-smoothing: none;\s*\}/s);
  assert.match(retroCss, /:root\[data-theme="light"\]/);
  assert.match(retroCss, /:root\[data-theme="dark"\]/);
  assert.match(retroCss, /\.sync-titlebar-context/);
  assert.match(retroCss, /\.sync-history-commit-heading/);
  assert.match(retroCss, /\.sync-history-thread-card/);
  assert.match(retroCss, /\.sync-history-timeline\.has-uncommitted::before\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-timeline\.has-uncommitted::after\s*\{[^}]*width: var\(--sync-history-node-size\);[^}]*height: var\(--sync-history-node-size\);[^}]*box-shadow: inset 0 0 0 3px var\(--text-2\);/s);
  assert.match(retroCss, /\.sync-history-commit-rail::before\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-commit::before\s*\{[^}]*top: var\(--sync-history-row-anchor-y\);[^}]*bottom: calc\(-10px - var\(--sync-history-row-anchor-y\)\);[^}]*left: calc\(var\(--sync-history-rail-x\) - var\(--sync-history-card-offset\)\);[^}]*width: 1px;[^}]*background: var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-uncommitted::before\s*\{[^}]*top: var\(--sync-history-row-anchor-y\);[^}]*width: calc\(var\(--sync-history-card-offset\) - var\(--sync-history-rail-x\)\);[^}]*background: repeating-linear-gradient\(/s);
  assert.match(retroCss, /\.sync-history-uncommitted::after\s*\{[^}]*bottom: -10px;[^}]*left: calc\(var\(--sync-history-rail-x\) - var\(--sync-history-card-offset\)\);[^}]*background: repeating-linear-gradient\(/s);
  assert.match(retroCss, /\.sync-history-commit-node\s*\{[^}]*width: var\(--sync-history-node-size\);[^}]*height: var\(--sync-history-node-size\);/s);
  assert.match(retroCss, /\.sync-history-commit-node::after\s*\{[^}]*top: calc\(var\(--sync-history-node-half\) - 1px\);[^}]*left: calc\(var\(--sync-history-node-size\) - 1px\);[^}]*width: calc\(\s*var\(--sync-history-card-offset\) - var\(--sync-history-rail-x\) -\s*var\(--sync-history-node-half\)\s*\);[^}]*background: var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-commit:last-child::before\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-timeline\.has-uncommitted \.sync-history-commit:only-child::before\s*\{[^}]*top: 0;[^}]*bottom: auto;[^}]*height: var\(--sync-history-row-anchor-y\);[^}]*display: block;/s);
  // Storage-only rows live outside the timeline and need the same row tokens.
  assert.match(retroCss, /\.sync-history-timeline,\s*\.sync-history-storage-only\s*\{[^}]*--sync-history-row-actions-column: 80px;[^}]*--sync-history-commit-actions-column: 26px;[^}]*--sync-history-commit-date-column: 26ch;[^}]*--sync-history-row-icon-size: 18px;[^}]*--sync-history-row-anchor-y: 15px;[^}]*--sync-history-rail-x: 8px;[^}]*--sync-history-card-offset: 26px;[^}]*--sync-history-node-size: 12px;[^}]*--sync-history-node-half: 6px;/s);
  assert.doesNotMatch(retroCss, /--sync-history-commit-thread-rail-x|--sync-history-commit-child-offset|--sync-history-commit-branch-length/);
  assert.doesNotMatch(retroCss, /\.sync-history-commit-heading:has\(\+ \.sync-history-thread-list\)::after/);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list::after\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list \.sync-thread-sync-indicator,[^{]*\{[^}]*background: transparent;[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.sync-history-commit-title > strong,[^}]*font: 600 12\.5px\/1\.35 var\(--font\);[^}]*letter-spacing: 0;/s);
  assert.match(retroCss, /\.sync-history-commit-meta > time\s*\{[^}]*font: 500 11\.5px\/1\.35 var\(--font\);[^}]*letter-spacing: 0;/s);
  // The sha reads as an identifier chip, not as more prose.
  assert.match(retroCss, /\.sync-history-commit-heading code\s*\{[^}]*min-width: 7ch;[^}]*border: 1px solid var\(--border\);[^}]*font: 500 11\.5px\/1\.35 var\(--font-mono\);[^}]*letter-spacing: 0;[^}]*text-align: center;/s);
  assert.match(retroCss, /\.sync-history-commit-link:hover code,\s*\.sync-history-commit-link:focus-visible code\s*\{[^}]*border-color: var\(--border-strong\);[^}]*color: var\(--text-0\);/s);
  // The subject outranks the session titles nested under it.
  assert.match(retroCss, /\.sync-history-commit-title > strong\s*\{[^}]*font: 650 13px\/1\.35 var\(--font\);/s);
  // Pixel geometry only stays crisp at an exact 2:1 downscale of the 24px art.
  assert.match(retroCss, /\.sync-history-commit-icon > svg,[\s\S]*?\.sync-history-chat-toggle > svg\s*\{[^}]*width: 12px;[^}]*height: 12px;/s);
  // A translucent block is a smudge, not a pixel — metrics are held back with
  // colour so every block stays solid.
  assert.match(retroCss, /\.sync-history-thread-metric svg\s*\{[^}]*opacity: 1;[^}]*color: var\(--text-3\);/s);
  // Rail nodes are stepped pixel rings, not antialiased circles.
  assert.match(retroCss, /--retro-pixel-ring: polygon\(/s);
  assert.match(retroCss, /\.sync-history-commit-node\s*\{[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-history-commit-node::before\s*\{[^}]*box-shadow: inset 0 0 0 3px var\(--text-2\);[^}]*clip-path: var\(--retro-pixel-ring\);/s);
  assert.match(retroCss, /\.sync-history-timeline\.has-uncommitted::after\s*\{[^}]*border-radius: 0;[^}]*clip-path: var\(--retro-pixel-ring\);/s);
  assert.match(retroCss, /\.sync-history-content\s*\{[^}]*container: sync-history-layout \/ inline-size;/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)/);
  // The status legend was removed, so none of its styling should linger.
  assert.doesNotMatch(retroCss, /\.sync-history-status-legend/);
  assert.match(retroCss, /@container sync-history-layout \(max-width: 760px\)\s*\{[^}]*\.sync-history-header\s*\{[^}]*flex-direction: column;/s);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card::before/);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card::after/);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card\s*\{[^}]*border: 0;[^}]*border-radius: 0;/s);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card\s*\{[^}]*padding-left:/s);
  assert.match(retroCss, /\.sync-history-commit-heading,\s*\.sync-history-thread-card\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) var\(--sync-history-row-actions-column\);[^}]*grid-template-areas:[^}]*"title actions"[^}]*"meta meta";/s);
  assert.match(retroCss, /\.sync-history-thread-topline\s*\{[^}]*display: contents;/s);
  assert.match(retroCss, /\.sync-history-commit-meta,\s*\.sync-history-session-details\s*\{[^}]*padding-left: var\(--sync-history-row-text-offset\);/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)\s*\{[^}]*\.sync-history-timeline\s*\{[^}]*--sync-history-row-anchor-y: 19px;/s);
  // Wide rows: the subject takes every spare pixel, and date + sha right-align
  // into a fixed gutter so they stack into columns down the timeline.
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-commit-heading,[^{]*\{[^}]*grid-template-columns: minmax\(0, 1fr\) auto var\(--sync-history-commit-actions-column\);[^}]*grid-template-areas: "title meta actions";/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-commit-meta\s*\{[^}]*justify-content: flex-end;/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-commit-meta > time\s*\{[^}]*min-width: var\(--sync-history-commit-date-column\);/s);
  assert.doesNotMatch(retroCss, /--sync-history-row-title-column|--sync-history-row-meta-column/);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-thread-card\s*\{[^}]*min-height: 52px;[^}]*grid-template-columns: minmax\(0, 1fr\) var\(--sync-history-row-actions-column\);[^}]*grid-template-areas:\s*"title actions"\s*"meta meta";[^}]*row-gap: 2px;[^}]*padding: 5px 12px 6px;/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-session-details\s*\{[^}]*padding-left: var\(--sync-history-row-text-offset\);[^}]*gap: 0;/s);
  assert.match(retroCss, /\.sync-history-uncommitted\s*\{[^}]*border: 1px solid var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-uncommitted \.sync-history-section-heading\s*\{[^}]*border-bottom: 1px solid var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-uncommitted \.sync-history-thread-list\.flat\s*\{[^}]*border: 0;/s);
  assert.match(retroCss, /\.sync-history-uncommitted \.sync-history-thread-card \+ \.sync-history-thread-card\s*\{[^}]*border-top: 1px solid var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-commit\s*\{[^}]*margin: 0 0 10px var\(--sync-history-card-offset\);[^}]*border: 1px solid var\(--border-mid\);[^}]*border-radius: 4px;[^}]*background: var\(--surface-row\);/s);
  assert.match(retroCss, /\.sync-history-commit::after\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list\s*\{[^}]*margin: 0;/s);
  assert.match(retroCss, /\.sync-log-drawer,\s*\.log-drawer\s*\{[^}]*margin-top: 0;[^}]*border: 0;[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.log-filter-control select\s*\{[^}]*box-shadow: none;/s);
  assert.match(retroCss, /\.sync-modal/);
  assert.doesNotMatch(retroCss, /!important/);

  assert.match(workspace, /aria-label=\{`Switch to \$\{theme === "dark" \? "light" : "dark"\} theme`\}/);
  assert.match(workspace, /<Icon name=\{theme === "dark" \? "moon" : "sun"\} size=\{18\} \/>/);
  assert.doesNotMatch(workspace, /\{theme === "dark" \? "Dark" : "Light"\} theme/);
  assert.match(retroCss, /\.sync-theme-button\s*\{[^}]*width: 34px;[^}]*height: 32px;[^}]*justify-content: center;[^}]*padding: 0;/s);
  assert.match(workspace, /aria-label="App actions"[\s\S]*?<Icon name="more" size=\{16\} \/>/);
});

test("the titlebar marks active Git projects with the Git folder icon", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");

  assert.match(
    workspace,
    /const workspaceIcon = !activeDraftSummary && activeSummary\?\.is_git_repository === true\s*\? "git-folder"\s*: "folder";/,
  );
  assert.match(workspace, /<Icon name=\{workspaceIcon\} size=\{16\} \/>/);
});

test("the shell verifies host identity and relies on backend auto-registration", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const startupScreen = readFileSync("src/components/project-sync/StartupLoadingScreen.tsx", "utf8");
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const linksWorkspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  assert.match(workspace, /projectSyncApi\.getHostIdentityStatus\(\)/);
  assert.match(workspace, /!hostIdentityStatus && hostIdentityError/);
  assert.match(workspace, /Host identity unavailable/);
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
    /const loadProjectData = async \([\s\S]*const requestedWorkflowGeneration = workflowGeneration \?\? hostWorkflowGeneration\.current;[\s\S]*getProject\(projectId\);[\s\S]*hostWorkflowGeneration\.current !== requestedWorkflowGeneration/,
  );
  assert.match(
    workspace,
    /const completeSetup = async \([\s\S]*workflowGeneration: number,[\s\S]*hostWorkflowGeneration\.current !== workflowGeneration[\s\S]*await loadShell\(\);[\s\S]*!hostIdentityReady \|\| hostWorkflowGeneration\.current !== workflowGeneration/,
  );
  assert.match(
    workspace,
    /const connectStorageBundle = async[\s\S]*const workflowGeneration = hostWorkflowGeneration\.current;[\s\S]*await loadShell\(\);[\s\S]*!hostIdentityReady \|\| hostWorkflowGeneration\.current !== workflowGeneration/,
  );
  // A mid-load identity change (backend auto-registration) restarts the
  // shell load instead of surfacing stale host-scoped state.
  assert.match(
    workspace,
    /verifiedHostIdentity\.host_id !== nextHostIdentity\.host_id[\s\S]*return loadShell\(\);/,
  );
  assert.match(workspace, /if \(!hostIdentityShellReady\) \{/);
  assert.match(workspace, /<StartupLoadingScreen stage=\{startupStage\} \/>/);
  assert.match(startupScreen, /Checking installation…/);
  assert.match(workspace, /nextDetail\.binding\.state !== "active"/);
  assert.match(workspace, /expected_revision: nextDetail\.binding\?\.revision \?\? null/);
  assert.match(workspace, /nextDetail\.binding\?\.state === "active"/);
  // The blocking recovery gate is gone: the backend registers a fresh host
  // automatically, so the shell never asks the user to register.
  assert.doesNotMatch(workspace, /origin_key_available/);
  assert.doesNotMatch(workspace, /registerAsNewHost/);
  assert.doesNotMatch(workspace, /"Register as new host"/);
  assert.ok(
    workspace.indexOf("if (!hostIdentityShellReady)") < workspace.indexOf("className={`sync-app"),
    "the ordinary shell must remain behind the completed identity refresh gate",
  );
  assert.match(linksWorkspace, /const bindingActive = projectBinding\?\.state === "active"/);
  assert.match(linksWorkspace, /const canSync = bindingActive && profilesReadable/);
  assert.match(linksWorkspace, /onConfigureBinding\(project\.local_project_id\)/);
  assert.match(setupWorkspace, /mountedRef\.current && isWorkflowCurrent\(workflowGeneration\)/);
  assert.match(setupWorkspace, /if \(!workflowIsActive\(\)\) return;\s*onFinalized\(detail, completion, workflowGeneration\)/);
  assert.doesNotMatch(workspace, /Host settings/);
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

test("host-scoped workflows keep their originating generation across async continuations", () => {
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
    const capture = body.indexOf("const workflowGeneration = hostWorkflowGeneration.current;");
    const awaited = body.indexOf(awaitExpression);
    assert.ok(capture >= 0, "workflow must capture the host generation");
    assert.ok(awaited > capture, `generation must be captured before ${awaitExpression}`);
  };

  const addProject = section(workspace, "beginAddProject", "openSetupDraft");
  capturesBefore(addProject, "await open(");
  assert.match(addProject, /await projectSyncApi\.createSetupDraft\(picked\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return;/);
  assert.match(addProject, /refreshSetupDrafts\(undefined, workflowGeneration\)/);

  const chooseProfile = section(workspace, "chooseAndCreateProfile", "refreshSetupDrafts");
  capturesBefore(chooseProfile, "await open(");
  assert.match(chooseProfile, /createProfileAtPath\(provider, picked, workflowGeneration\)/);

  const discardDraft = section(workspace, "discardSetupDraft", "completeSetup");
  capturesBefore(discardDraft, "await confirm(");
  assert.match(discardDraft, /await projectSyncApi\.discardSetupDraft\(draftId\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return;/);

  const linkStorage = section(workspace, "linkStorage", "repairConversationPaths");
  capturesBefore(linkStorage, "await projectSyncApi.listRemoteBundleSnapshots");
  assert.match(linkStorage, /saveStorageLink\(projectId, storageId, workflowGeneration\)/);
  assert.match(linkStorage, /hostWorkflowGeneration\.current === workflowGeneration\) setBusy\(false\)/);

  const repairPaths = section(workspace, "repairConversationPaths", "openPushChooser");
  capturesBefore(repairPaths, "await confirm(");
  assert.match(repairPaths, /await projectSyncApi\.repairCodexConversationPaths\(projectId\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return;/);
  assert.match(repairPaths, /activeStorageByProject\[projectId\],[\s\S]*workflowGeneration,[\s\S]*\);/);

  const renameProject = section(workspace, "renameProject", "planRestore");
  capturesBefore(renameProject, "await projectSyncApi.renameProject");
  assert.match(renameProject, /hostWorkflowGeneration\.current !== workflowGeneration\) return false;/);

  const keepBundle = section(workspace, "keepPendingBundle", "removeProject");
  capturesBefore(keepBundle, "await saveStorageLink");
  assert.match(keepBundle, /openPushChooser\(pending\.projectId, pending\.storageId, workflowGeneration\)/);

  const removeProject = section(workspace, "removeProject", "closePullReview");
  capturesBefore(removeProject, "await confirm(");
  assert.match(removeProject, /await refresh\(workflowGeneration\)/);

  const unlinkStorage = section(workspace, "unlinkStorage", "applyReview");
  capturesBefore(unlinkStorage, "await confirm(");
  assert.match(unlinkStorage, /await projectSyncApi\.removeLink\(projectId, storageId\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return;/);
  assert.match(unlinkStorage, /loadProjectData\(projectId, nextConfig, nextStorageId, workflowGeneration\)/);

  const saveStorage = section(workspace, "saveStorageConfig", "saveInlineStorage");
  assert.match(saveStorage, /workflowGeneration = hostWorkflowGeneration\.current/);
  assert.match(saveStorage, /await projectSyncApi\.getConfig\(\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return false;/);
  assert.match(saveStorage, /await projectSyncApi\.saveConfig\(update\(current\)\);\s*if \(hostWorkflowGeneration\.current !== workflowGeneration\) return false;/);

  const removeStorage = section(workspace, "removeStorage", "openNewStorageConfiguration");
  capturesBefore(removeStorage, "await confirm(");
  assert.match(removeStorage, /Stored files were not deleted\.`, workflowGeneration\)/);

  assert.match(workspace, /workflowGeneration=\{hostWorkflowGeneration\.current\}/);
  assert.match(workspace, /isWorkflowCurrent=\{\(workflowGeneration\) => \([\s\S]*hostWorkflowGeneration\.current === workflowGeneration/);
  const finalize = section(setupWorkspace, "finalize", null);
  assert.match(finalize, /const detail = await projectSyncApi\.finalizeProjectSetup\(draftId, expectedRevision\);\s*if \(!workflowIsActive\(\)\) return;\s*onFinalized\(detail, completion, workflowGeneration\)/);
  assert.match(setupWorkspace, /mountedRef\.current = false;\s*inspectRequest\.current \+= 1;/);
});

test("refocus reconciles external changes narrowly and the full reload lives in the titlebar menu", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");

  // The sidebar no longer carries an always-visible refresh icon; full reload is a
  // labeled recovery action in the titlebar overflow menu.
  assert.doesNotMatch(sidebar, /onRefresh|Refresh projects/);
  assert.match(workspace, /Reload app data/);
  assert.match(workspace, /className="sync-titlebar-actions"/);

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
  assert.doesNotMatch(reconcile, /auditCodexConversationPaths|listProviderProfiles|listBindings|getReadiness|getInventory|setLoading/);
  assert.match(reconcile, /shellRequest\.current !== shellRequestId/);
  assert.match(reconcile, /hostWorkflowGeneration\.current !== workflowGeneration/);
  assert.match(reconcile, /await refresh\(workflowGeneration\)/);
  assert.match(workspace, /Date\.now\(\) - blurredAt >= FOCUS_RECONCILE_MIN_AWAY_MS/);
});
