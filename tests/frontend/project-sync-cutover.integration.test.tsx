import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StartupLoadingScreen from "../../src/components/project-sync/StartupLoadingScreen";
import { repositoryFingerprintMatches } from "../../src/components/project-sync/model";
import { PROJECT_SYNC_SCHEMA } from "../../src/types";
import {
  MIN_PANE_HEIGHT,
  catalogHeightRange,
  shareHolding,
  shareHoldingStorages,
} from "../../src/components/project-sync/ProjectSidebar";
import {
  expandHomePath,
  profilePathHint,
  type ProfilePathProbeState,
} from "../../src/components/project-sync/ProjectSetupWorkspace";

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

test("project setup always finishes into Activity without opening Pull review", () => {
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const completeSetup = workspace.slice(
    workspace.indexOf("  const completeSetup = async"),
    workspace.indexOf("\n  const saveStorageLink =", workspace.indexOf("  const completeSetup = async")),
  );

  assert.doesNotMatch(setupWorkspace, /SetupCompletion|Finish & review|finalize\("pull"\)/);
  assert.match(setupWorkspace, /onClick=\{\(\) => void finalize\(\)\}/);
  assert.match(completeSetup, /setActiveProjectId\(projectId\)/);
  assert.match(completeSetup, /await loadProjectData\(projectId, nextConfig, storageId, workflowEpoch\)/);
  assert.doesNotMatch(completeSetup, /planRestore|completion === "pull"|open Pull review/);
});

test("project setup configures the project without selecting sync contents", () => {
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const types = readFileSync("src/types.ts", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const backend = readFileSync("src-tauri/src/lib.rs", "utf8");

  assert.doesNotMatch(setupWorkspace, /Sync contents|ResourceInventory|selected_resource_ids/);
  assert.doesNotMatch(
    types,
    /selected_resource_ids|discovery_signature|discovery_profile_signature|selection_stale|profile_changed/,
  );
  assert.doesNotMatch(api, /discoverProject|invoke<ProjectDiscovery>\("discover_project"/);
  assert.doesNotMatch(backend, /commands::discover_project/);
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
  assert.doesNotMatch(pushSignature, /recipe: Option<BundleRecipe>/);
  assert.match(workspace, /recipeSelection\(savedRecipe \?\? nextInventory\.recipe\)/);
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
  // The bar holds a 3.25rem floor (52px at the default root) and grows with
  // its text instead of clipping it — height stays unpinned deliberately.
  assert.match(css, /\.sync-titlebar \{[^}]*min-height: 3\.25rem;[^}]*flex: 0 0 auto;/);
  // Read what the cascade actually delivers, not what App.css happens to say.
  // These two assertions passed for 38px/52px while RetroTheme.css — which
  // loads second and ties on specificity — rendered 42px/56px, so the gate
  // that exists to keep the macOS traffic lights centred in the brand row was
  // measuring a value that never reaches the screen. 56 / 2 = 28, which is
  // where trafficLightPosition.y sits.
  assert.equal(effectiveDeclaration(".sync-sidebar-drag", "height"), "42px");
  assert.equal(effectiveDeclaration(".sync-sidebar-brand", "height"), "56px");
  assert.deepEqual(tauriConfig.app.windows[0].trafficLightPosition, { x: 20, y: 29 });
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

  for (const file of ["src/App.css", "src/RetroTheme.css"]) {
    const wide = blocks(readFileSync(file, "utf8")).filter((block) => block.limit > 720);
    assert.ok(wide.length > 0, `${file} should still have wide breakpoints to check`);
    for (const block of wide) {
      for (const selector of [".sync-sidebar-brand", ".sync-brand-logo", ".sync-sidebar "]) {
        assert.ok(
          !block.body.includes(selector),
          `${file}: @media (max-width: ${block.limit}px) must not restyle ${selector.trim()}`,
        );
      }
      assert.doesNotMatch(
        block.body,
        /\.sync-sidebar\s*\{/,
        `${file}: @media (max-width: ${block.limit}px) must not resize the sidebar`,
      );
    }
  }
});

test("the path bar's trail is sized by the bar, never by the window", () => {
  const css = readFileSync("src/App.css", "utf8");
  // Declared once, so no later block can quietly re-cap the same box: a second
  // rule in a window-sized @media is how the viewport got back in last time.
  const rule = (selector: string) => {
    const parts = css.split(`\n${selector} {`);
    assert.equal(parts.length, 2, `${selector} must be declared in exactly one App.css rule`);
    return parts[1].slice(0, parts[1].indexOf("}"));
  };

  // The bar is the window less the sidebar, so a viewport share is a far
  // bigger share of the row than it reads as. At `max-width: 30vw` the session
  // title's floor came out wider than both ancestors had between them: the
  // storage crumb was squeezed out of its own box and its icon carried on
  // painting over the project's name.
  for (const selector of [".sync-path-crumb", ".sync-history-branch-trigger"]) {
    const declarations = rule(selector);
    assert.match(declarations, /max-width:[^;]*cqi/, `${selector} must cap against the bar`);
    assert.doesNotMatch(declarations, /max-width:[^;]*\dvw/, `${selector} must not cap against the window`);
  }

  // A crumb may give up its whole label; it may never be squeezed under the
  // icon and the chevron, which do not shrink and so carry on painting where
  // the box has already ended — over the crumb beside them.
  for (const selector of [".sync-path-item", ".sync-path-item:first-child", ".sync-path-crumb-link"]) {
    assert.match(
      rule(selector),
      /min-width: \d+(?:\.\d+)?rem;/,
      `${selector} must floor at its own chrome`,
    );
  }
  assert.match(css, /\n\.sync-path-crumb-toggle \{[^}]*flex: 0 0 auto;/);
});

test("the retro skin stays presentation-only and preserves the shell controls", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const startup = readFileSync("src/components/project-sync/StartupLoadingScreen.tsx", "utf8");
  const document = readFileSync("index.html", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");

  assert.match(app, /import "\.\/App\.css";\s*import "\.\/RetroTheme\.css";/);
  // One mark, everywhere: brand, product footer, startup screen, favicon.
  assert.equal(existsSync("public/mallard-logo-duck.svg"), true);
  for (const retired of [
    "public/mallard-logo-pixel.png",
    "public/mallard-logo-pixel-ring.png",
    "public/mallard-logo-pixel-duck.png",
  ]) {
    assert.equal(existsSync(retired), false, `${retired} must remain removed`);
  }
  const duck = readFileSync("public/mallard-logo-duck.svg", "utf8");
  // An existence check cannot tell a valid mark from a broken one. "--" is
  // illegal inside an XML comment and makes the whole file fail to parse, which
  // shows up as a broken-image icon in every mark at once while the suite stays
  // green — so the comment body is checked directly.
  for (const [, body] of duck.matchAll(/<!--([\s\S]*?)-->/g)) {
    assert.doesNotMatch(body, /--/, "an XML comment may not contain --");
  }
  assert.match(duck, /viewBox="0 0 20 20"/);
  assert.match(duck, /shape-rendering="crispEdges"/);
  // Loaded through <img>, so it has no CSS context and cannot use currentColor.
  assert.match(duck, /fill="#050913"/);
  assert.doesNotMatch(duck, /fill="currentColor"/);
  assert.equal(existsSync("public/fonts/DepartureMono-Regular.woff2"), true);
  assert.equal(existsSync("public/fonts/DepartureMono-OFL.txt"), true);
  assert.equal(existsSync("THIRD_PARTY_NOTICES.md"), true);
  assert.match(sidebar, /className="sync-brand-logo" src="\/mallard-logo-duck\.svg"/);
  assert.match(sidebar, /className="sync-sidebar-product-mark"[\s\S]*src="\/mallard-logo-duck\.svg"/);
  assert.match(startup, /src="\/mallard-logo-duck\.svg"/);
  assert.match(document, /rel="icon" type="image\/svg\+xml" href="\/mallard-logo-duck\.svg"/);
  assert.match(retroCss, /@font-face\s*\{[\s\S]*font-family: "Departure Mono"/);
  // Three faces, one rule each: pixel for chrome labels, sans for text that came
  // out of the user's data, mono for machine identifiers.
  assert.match(retroCss, /--font-sans: -apple-system, BlinkMacSystemFont/);
  assert.match(retroCss, /--font-mono: "SFMono-Regular", "SF Mono"/);
  assert.match(retroCss, /--font-pixel: "Departure Mono", var\(--font-mono\)/);
  assert.match(retroCss, /--font: var\(--font-sans\);/);
  // Root-relative scale (WCAG 1.4.4): every step resolves to the whole pixel
  // it was designed at when the root sits at the 16px default — 0.5625rem is
  // 9px, 1.625rem is 26px — and follows the user's text-size preference.
  assert.match(retroCss, /--type-3xs: 0\.5625rem;[\s\S]*--type-3xl: 1\.625rem;/);
  assert.match(retroCss, /--weight-regular: 400;[\s\S]*--weight-bold: 700;/);
  assert.match(retroCss, /html \{\s*font-size: 100%;\s*\}/);
  assert.match(retroCss, /:root\[data-theme="dark"\] \.sync-brand-logo,[\s\S]*:root\[data-theme="dark"\] \.sync-sidebar-product-mark img,[\s\S]*:root\[data-theme="dark"\] \.sync-startup-mark img\s*\{[^}]*filter: brightness\(0\) invert\(1\);/s);
  // A vector mark rasterises itself, so image-rendering governs nothing here.
  assert.doesNotMatch(retroCss, /image-rendering:/);
  // Every mark is a whole multiple of the 20-unit grid, or the cells land on
  // fractional device pixels and the silhouette goes ragged.
  assert.match(retroCss, /\.sync-brand-logo\s*\{[^}]*width: 40px;[^}]*height: 40px;/s);
  assert.match(retroCss, /\.sync-sidebar-product-mark img\s*\{[^}]*width: 20px;[^}]*height: 20px;/s);
  assert.match(retroCss, /\.sync-startup-mark img\s*\{[^}]*width: 40px;[^}]*height: 40px;/s);
  // The pixel face is declared exactly once, and that one rule also carries the
  // unsmoothed rasterisation. Two parallel selector lists used to drift apart;
  // counting the rules is what makes that impossible rather than merely visible.
  const pixelRules = [...retroCss.matchAll(/([^{}]+)\{([^}]*font-family: var\(--font-pixel\)[^}]*)\}/g)];
  assert.equal(pixelRules.length, 1, "the pixel face must be declared in exactly one rule");
  const [, pixelSelectors, pixelBody] = pixelRules[0];
  assert.match(pixelBody, /letter-spacing: 0;/);
  // Single-weight face plus font-synthesis: none — anything heavier would be a
  // declaration that quietly does nothing.
  assert.match(pixelBody, /font-weight: var\(--weight-regular\);/);
  assert.match(pixelBody, /font-synthesis: none;/);
  for (const selector of [
    ".btn",
    ".sync-sidebar-brand strong",
    ".sync-primary-nav button",
    ".sidebar-settings-sections .sidebar-section-title",
    ".sync-path-sync",
    ".sync-path-crumb.section",
    ".sync-history-section-heading h2",
    ".sync-history-branch-state",
    ".log-panel-title",
    ".log-type",
  ]) {
    assert.ok(pixelSelectors.includes(selector), `${selector} must take the pixel face`);
  }
  // A count and a branch name are data, so neither speaks in the app's voice.
  assert.ok(!pixelSelectors.includes(".sidebar-nav-badge"));
  assert.ok(!pixelSelectors.includes(".sync-history-branch-trigger"));
  // A storage name, a repo name and a session title are data; only the section
  // crumb is a closed word, so the bare crumb must not take the pixel face.
  assert.ok(!/\.sync-path-crumb(?![.\w-])/.test(pixelSelectors));
  assert.match(retroCss, /\.sync-path-crumb\s*\{[^}]*font-family: var\(--font-sans\)/s);
  assert.match(retroCss, /\.sync-path-menu-copy > small\s*\{[^}]*font-family: var\(--font-mono\)/s);
  // The old page-heading identity block is gone everywhere — the path bar is
  // the identity now, on the storage screens as well as the project ones.
  assert.doesNotMatch(retroCss, /sync-project-heading/);
  assert.match(retroCss, /\.sync-titlebar-context strong\s*\{[^}]*font-family: var\(--font-sans\)/s);
  // One surface across the bar: no second panel ending mid-titlebar.
  assert.match(retroCss, /\.sync-titlebar-context\s*\{[^}]*border: 0;[^}]*border-radius: 0;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sidebar-profile-main span\s*\{[^}]*font-family: var\(--font-sans\)/s);
  // A name out of the user's data reads in the clean face even when a pixel
  // label wraps it, and a byte readout keeps the identifier face inside a
  // pixel button. These two carve-outs are the pattern the rest copies.
  assert.match(retroCss, /\.sync-history-section-heading h2 \.sync-history-storage-name\s*\{[^}]*font-family: var\(--font-sans\)/s);
  assert.match(retroCss, /\.sync-btn-readout[^{]*\{[^}]*font-family: var\(--font-mono\)/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--bg-0: #fefbf6;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--bg-2: #faf3eb;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--surface-row-active: #f0e3d5;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--text-0: #1e1817;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--accent: #2a211d;/s);
  assert.match(retroCss, /:root\[data-theme="light"\]\s*\{[^}]*--retro-rule: #8f7f74;/s);
  assert.match(retroCss, /:root,\s*:root\[data-theme="dark"\]\s*\{[^}]*--border: #2e2925;[^}]*--border-mid: #443d36;[^}]*--border-strong: #7a7065;/s);
  assert.match(retroCss, /:root,\s*:root\[data-theme="dark"\]\s*\{[^}]*--retro-rule: #968a7c;[^}]*--retro-frame-edge: #968a7c;/s);
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
  assert.match(retroCss, /\.sync-sidebar-head,\s*\.sidebar-project-pane,\s*\.sidebar-storage-pane,\s*\.sidebar-catalog-pane,\s*\.sync-sidebar-product-mark,\s*\.sync-titlebar,\s*\.sync-workspace > \.sync-main,\s*\.sync-log-drawer\.open\s*\{[^}]*border: 1px solid var\(--retro-frame-edge\);[^}]*border-radius: var\(--retro-panel-radius\);/s);
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
  // Two elastic rows and a Catalog row that fits its contents up to a 40% cap
  // until the lower handle pins it, parted by two 3px handle tracks. The cap
  // matters because the row list is unbounded — one row per (provider home ×
  // kind) — and an uncapped content-sized track would squeeze Projects and
  // Storages onto their 96px floors.
  const sidebarSource = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  assert.match(sidebarSource, /gridTemplateRows: `\$\{elasticRows\(sectionLayout\.projectShare\)\} 3px \$\{sectionLayout\.catalogHeight === null \? "fit-content\(40%\)" : `\$\{sectionLayout\.catalogHeight\}px`\}`/);
  // Shares go out as fr factors scaled to 100, never as the fractions
  // themselves: a track whose flex factor is under 1 is only given that
  // fraction of the leftover space, so once a pane bottoms out on its floor the
  // rows stop filling the box and leave dead space under the last pane.
  assert.match(sidebarSource, /const projects = projectShare \* 100;/);
  assert.match(sidebarSource, /minmax\(\$\{MIN_PANE_HEIGHT\}px, \$\{projects\}fr\) 3px minmax\(\$\{MIN_PANE_HEIGHT\}px, \$\{100 - projects\}fr\)/);
  assert.match(retroCss, /\.sidebar-project-pane,\s*\.sidebar-storage-pane,\s*\.sidebar-catalog-pane\s*\{[^}]*background: var\(--bg-1\);/s);
  assert.doesNotMatch(retroCss, /\.sidebar-catalog-pane\s*\{[^}]*margin-top/s);
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
  // Smoothing is never disabled. Thresholding the edges rounds any partial
  // pixel coverage up to full ink, which thickens every stem and reads as a
  // permanently bold UI at 9-17px. One antialiased declaration, on body.
  assert.doesNotMatch(retroCss, /-webkit-font-smoothing: none;/);
  assert.equal(retroCss.match(/-webkit-font-smoothing: antialiased/g)?.length, 1);
  assert.match(retroCss, /:root\[data-theme="light"\]/);
  assert.match(retroCss, /:root\[data-theme="dark"\]/);
  assert.match(retroCss, /\.sync-titlebar-context/);
  assert.match(retroCss, /\.sync-history-commit-heading/);
  assert.match(retroCss, /\.sync-history-thread-card/);
  // The old continuous rail line is gone from BOTH sheets, not neutralised:
  // the theme's display:none existed only to kill the App.css original.
  assert.doesNotMatch(retroCss, /\.sync-history-commit-rail::before/);
  assert.doesNotMatch(readFileSync("src/App.css", "utf8"), /\.sync-history-commit-rail::before/);
  // Both rail segments yield the fold's 24px box by geometry — 12px short of
  // the anchor, resuming 12px past it. They must never go back to running
  // under the fold and relying on its background to hide them: `opacity` on a
  // disabled fold and the translucent hover wash each let the line straight
  // back through, so the spine broke at some folds, ran through others, and
  // grew a line on hover. `bottom: -anchor` closes the joint exactly — the
  // pseudo is laid out against the padding box, so the two card borders and
  // the 10px gap already span the 12px the fold needs.
  assert.match(retroCss, /\.sync-history-commit::before\s*\{[^}]*top: calc\(var\(--sync-history-row-anchor-y\) \+ 12px\);[^}]*bottom: calc\(-1 \* var\(--sync-history-row-anchor-y\)\);[^}]*left: calc\(var\(--sync-history-rail-x\) - var\(--sync-history-card-offset\)\);[^}]*width: 1px;[^}]*background: var\(--border-mid\);/s);
  assert.match(retroCss, /\.sync-history-uncommitted::after\s*\{[^}]*top: calc\(var\(--sync-history-row-anchor-y\) \+ 12px\);[^}]*bottom: calc\(-1 \* var\(--sync-history-row-anchor-y\)\);[^}]*left: calc\(var\(--sync-history-rail-x\) - var\(--sync-history-card-offset\)\);[^}]*background: repeating-linear-gradient\(/s);
  assert.match(retroCss, /\.sync-history-commit:last-child::before\s*\{[^}]*display: none;/s);
  // The Uncommitted group's own segment now reaches the first commit's fold,
  // so neither the first-child reach nor the only-child stub has any work.
  assert.doesNotMatch(retroCss, /\.sync-history-commit:(first|only)-child::before/);
  // The rail node became the fold button: one 24px control (WCAG 2.5.8) in
  // the rail column for every fold on the page, outside the card so neither
  // a review's checkbox nor an outcome banner can move it. The decorative
  // node, its connector, and the pixel-ring var it used are gone with it.
  assert.match(retroCss, /\.sync-history-spine-fold\s*\{[^}]*position: absolute;[^}]*top: calc\(var\(--sync-history-row-anchor-y\) - 12px\);[^}]*left: calc\(var\(--sync-history-rail-x\) - var\(--sync-history-card-offset\) - 12px\);[^}]*width: 24px;[^}]*height: 24px;[^}]*background: transparent;/s);
  assert.match(retroCss, /\.sync-history-spine-fold:disabled\s*\{[^}]*opacity: 0\.35;/s);
  assert.doesNotMatch(retroCss, /sync-history-commit-node|--sync-history-node-size|--sync-history-node-half|--retro-pixel-ring|sync-history-group-toggle/);
  // Storage-only rows and the Storage card live outside the timeline and need
  // the same row tokens; they are not inherited, so leaving either scope out
  // would lose the rail inset. The actions column shrink-wraps (auto): a fixed
  // 148px four-slot reservation left ~97px of blank between the ellipsized
  // title and the first of the two buttons the markup settled on.
  assert.match(retroCss, /\.sync-history-timeline,\s*\.sync-history-storage-only,\s*\.project-storage-card\s*\{[^}]*--sync-history-row-actions-column: auto;[^}]*--sync-history-commit-actions-column: 26px;[^}]*--sync-history-commit-date-column: 26ch;[^}]*--sync-history-row-icon-size: 18px;[^}]*--sync-history-row-anchor-y: 15px;[^}]*--sync-history-rail-x: 8px;[^}]*--sync-history-card-offset: 26px;/s);
  // The storage-only group takes the timeline's inset so its fold lands in the
  // rail column, and keeps the Uncommitted frame's 10px gap before the rail.
  assert.match(retroCss, /\.sync-history-storage-only\s*\{[^}]*margin: 12px 0 10px var\(--sync-history-card-offset\);/s);
  // Its heading count closes the bar at the card edge, like the stamps below.
  assert.match(retroCss, /\.sync-history-storage-only \.sync-history-heading-count\s*\{[^}]*margin-left: auto;/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)\s*\{[^{]*\.sync-history-timeline,\s*\.sync-history-storage-only\s*\{[^}]*--sync-history-row-anchor-y: 19px;/s);
  assert.doesNotMatch(retroCss, /sync-history-spine-mark/);
  assert.doesNotMatch(retroCss, /--sync-history-commit-thread-rail-x|--sync-history-commit-child-offset|--sync-history-commit-branch-length/);
  assert.doesNotMatch(retroCss, /\.sync-history-commit-heading:has\(\+ \.sync-history-thread-list\)::after/);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list::after\s*\{[^}]*display: none;/s);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list \.sync-thread-sync-indicator,[^{]*\{[^}]*background: transparent;[^}]*box-shadow: none;/s);
  // Titles are human text, so they read in the sans.
  assert.match(retroCss, /\.sync-history-commit-title > strong,[^}]*font-family: var\(--font-sans\);[^}]*font-size: var\(--type-sm\);[^}]*font-weight: var\(--weight-semibold\);[^}]*letter-spacing: 0;/s);
  // A date is scanned as a column, so it takes the identifier face and figures
  // that hold their width.
  assert.match(retroCss, /\.sync-history-commit-meta > time\s*\{[^}]*font-family: var\(--font-mono\);[^}]*font-size: var\(--type-xs\);[^}]*font-weight: var\(--weight-medium\);[^}]*font-variant-numeric: tabular-nums;[^}]*letter-spacing: 0;/s);
  // Only the link wears the chip — the border is the affordance — so the
  // chip rule is scoped to the link and a sha with no remote is bare mono.
  assert.match(retroCss, /\.sync-history-commit-link > code\s*\{[^}]*min-width: 7ch;[^}]*border: 1px solid var\(--border\);[^}]*font-family: var\(--font-mono\);[^}]*font-size: var\(--type-xs\);[^}]*font-weight: var\(--weight-medium\);[^}]*letter-spacing: 0;[^}]*text-align: center;/s);
  assert.doesNotMatch(retroCss, /\.sync-history-commit-heading code\s*\{/);
  assert.match(retroCss, /\.sync-history-commit-meta > code\s*\{[^}]*color: var\(--text-2\);[^}]*font-family: var\(--font-mono\);[^}]*font-size: var\(--type-xs\);/s);
  assert.match(retroCss, /\.sync-history-commit-link:hover code,\s*\.sync-history-commit-link:focus-visible code\s*\{[^}]*border-color: var\(--border-strong\);[^}]*color: var\(--text-0\);/s);
  // The subject outranks the session titles nested under it by one size step.
  // It used to be a weight step too, but 650 over 600 was never drawn apart.
  assert.match(retroCss, /\.sync-history-commit-title > strong\s*\{[^}]*font-size: var\(--type-md\);/s);
  // Pixel geometry only stays crisp at an exact 2:1 downscale of the 24px art.
  assert.match(retroCss, /\.sync-history-commit-icon > svg,[\s\S]*?\.sync-history-debug-trigger > svg\s*\{[^}]*width: 12px;[^}]*height: 12px;/s);
  // A translucent block is a smudge, not a pixel — metrics are held back with
  // colour so every block stays solid.
  assert.match(retroCss, /\.sync-history-thread-metric svg\s*\{[^}]*opacity: 1;[^}]*color: var\(--text-3\);/s);
  assert.match(retroCss, /\.sync-history-content\s*\{[^}]*container: sync-history-layout \/ inline-size;/s);
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)/);
  // The status legend was removed, so none of its styling should linger.
  assert.doesNotMatch(retroCss, /\.sync-history-status-legend/);
  // The list has no header to reflow: branch, counts and refresh are the path
  // bar's, and the bar carries its own narrow behaviour.
  assert.doesNotMatch(retroCss, /\.sync-history-header/);
  assert.doesNotMatch(retroCss, /\.sync-history-toolbar/);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card::before/);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card::after/);
  assert.match(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card\s*\{[^}]*border: 0;[^}]*border-radius: 0;/s);
  assert.doesNotMatch(retroCss, /\.sync-history-commit > \.sync-history-thread-list > \.sync-history-thread-card\s*\{[^}]*padding-left:/s);
  assert.match(retroCss, /\.sync-history-commit-heading,\s*\.sync-history-thread-card\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) var\(--sync-history-row-actions-column\);[^}]*grid-template-areas:[^}]*"title actions"[^}]*"meta meta";/s);
  assert.match(retroCss, /\.sync-history-thread-topline\s*\{[^}]*display: contents;/s);
  assert.match(retroCss, /\.sync-history-commit-meta,\s*\.sync-history-session-details\s*\{[^}]*padding-left: var\(--sync-history-row-text-offset\);/s);
  // Wide rows: the subject takes every spare pixel, and sha + date right-align
  // so the stamp closes the row at the card edge — one column with the
  // session and cloud stamps, no trailing actions track anywhere.
  assert.match(retroCss, /@container sync-history-layout \(min-width: 700px\)[\s\S]*?\.sync-history-commit-heading\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) auto;[^}]*grid-template-areas: "title meta";/s);
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

test("the notice toast squares off and reports its own tone", () => {
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");

  // Squared to the app's corner, and on the whole-pixel scale: 8px/10.5px were
  // the last of App.css's pill styling showing through the retro skin.
  assert.match(retroCss, /\.sync-notice\s*\{[^}]*border-radius: 4px;[^}]*font-size: var\(--type-xs\);/s);
  assert.match(retroCss, /\.sync-notice-dismiss\s*\{[^}]*border-radius: 3px;/s);
  // The rail is positioned, not a stretched flex item: stretch spans only the
  // content box, which left it short of both edges by the toast's padding.
  assert.match(retroCss, /\.sync-notice::before\s*\{[^}]*position: absolute;[^}]*inset: 0 auto 0 0;[^}]*width: 3px;[^}]*background: var\(--green\);/s);
  assert.match(retroCss, /\.sync-notice\s*\{[^}]*overflow: hidden;/s);
  assert.match(retroCss, /\.sync-notice\.is-attention::before\s*\{[^}]*background: var\(--yellow\);/s);

  // A green tick used to announce every notice, including the ones that report
  // a stop. Tone drives the glyph, so those two cannot drift apart again.
  assert.match(workspace, /type SyncNoticeTone = "ok" \| "attention"/);
  assert.match(workspace, /notice\.tone === "attention" \? "alert-triangle" : "check-circle"/);
  assert.match(workspace, /notice\.tone === "attention" \? " is-attention" : ""/);
  // An unconfirmed binding is a stop, not a success.
  assert.match(workspace, /before pulling\.",\s*"attention"\)/);
  // Every other call site stays a one-argument confirmation.
  assert.match(workspace, /const notify = \(message: string, tone: SyncNoticeTone = "ok"\) => setNotice\(\{ message, tone \}\)/);
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
    /const workspaceTitle = activeDraftSummary\?\.display_name\s*\?\? \(addingCatalogScope \? "Add Catalog resource" : null\)\s*\?\? \(activeSummary \? null : "Projects"\);/,
  );
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
  assert.match(linksWorkspace, /const workspaceCanSync = workspaceBindingActive && workspaceProfilesReadable/);
  assert.match(linksWorkspace, /onConfigureBinding\(workspaceProject\.local_project_id\)/);
  assert.match(setupWorkspace, /mountedRef\.current && isWorkflowCurrent\(workflowEpoch\)/);
  assert.match(setupWorkspace, /if \(!workflowIsActive\(\)\) return;\s*onFinalized\(detail, workflowEpoch\)/);
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

/**
 * The value a selector's property actually resolves to across both
 * stylesheets. App.css loads first and RetroTheme.css second (src/App.tsx), so
 * on a specificity tie the later file wins — and asserting against App.css
 * alone silently pins a dead declaration.
 */
function effectiveDeclaration(selector: string, property: string): string | null {
  const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
  let value: string | null = null;
  for (const file of ["src/App.css", "src/RetroTheme.css"]) {
    for (const block of strip(readFileSync(file, "utf8")).matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const prelude = block[1].replace(/\s+/g, " ").trim();
      if (prelude.startsWith("@")) continue;
      if (!prelude.split(",").some((s) => s.trim() === selector)) continue;
      for (const decl of block[2].split(";")) {
        const colon = decl.indexOf(":");
        if (colon < 0) continue;
        if (decl.slice(0, colon).trim() !== property) continue;
        value = decl.slice(colon + 1).trim();
      }
    }
  }
  return value;
}

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

  const addProject = section(workspace, "beginAddProject", "openSetupDraft");
  capturesBefore(addProject, "await open(");
  assert.match(addProject, /await projectSyncApi\.createSetupDraft\(picked\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);
  assert.match(addProject, /refreshSetupDrafts\(undefined, workflowEpoch\)/);

  // ensureProfileAtPath runs inside a caller's envelope: the epoch arrives as
  // a parameter, and every await re-checks it before touching state.
  const ensureProfile = section(workspace, "ensureProfileAtPath", "refreshSetupDrafts");
  assert.match(ensureProfile, /workflowEpoch: number,/);
  assert.match(ensureProfile, /await projectSyncApi\.probeProviderProfile\(provider, path\.trim\(\)\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return null;/);
  assert.match(ensureProfile, /if \(hostWorkflowEpoch\.current !== workflowEpoch\) return null;\s*setProfiles\(nextProfiles\);/);

  const discardDraft = section(workspace, "discardSetupDraft", "completeSetup");
  capturesBefore(discardDraft, "await confirm(");
  assert.match(discardDraft, /await projectSyncApi\.discardSetupDraft\(draftId\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);

  const linkStorage = section(workspace, "linkStorage", "repairConversationPaths");
  capturesBefore(linkStorage, "await projectSyncApi.listRemoteBundleSnapshots");
  assert.match(linkStorage, /saveStorageLink\(projectId, storageId, workflowEpoch\)/);
  assert.match(linkStorage, /hostWorkflowEpoch\.current === workflowEpoch\) setBusy\(false\)/);

  const repairPaths = section(workspace, "repairConversationPaths", "openPushChooser");
  capturesBefore(repairPaths, "await confirm(");
  assert.match(repairPaths, /await projectSyncApi\.repairCodexConversationPaths\(projectId\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);
  assert.match(repairPaths, /activeStorageByProject\[projectId\],[\s\S]*workflowEpoch,[\s\S]*\);/);

  const renameProject = section(workspace, "renameProject", "planRestore");
  capturesBefore(renameProject, "await projectSyncApi.renameProject");
  // The guard still bails on a superseded workflow; it now reports that
  // distinctly from a real failure so the rename form stays quiet.
  assert.match(renameProject, /hostWorkflowEpoch\.current !== workflowEpoch\) return \{ ok: false, message: null \};/);

  const keepBundle = section(workspace, "keepPendingBundle", "removeProject");
  capturesBefore(keepBundle, "await saveStorageLink");
  assert.match(keepBundle, /openPushChooser\(pending\.projectId, pending\.storageId, workflowEpoch\)/);

  const removeProject = section(workspace, "removeProject", "closePullReview");
  capturesBefore(removeProject, "await confirm(");
  assert.match(removeProject, /await refresh\(workflowEpoch\)/);

  const unlinkStorage = section(workspace, "unlinkStorage", "applyReview");
  capturesBefore(unlinkStorage, "await confirm(");
  assert.match(unlinkStorage, /await projectSyncApi\.removeLink\(projectId, storageId\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return;/);
  assert.match(unlinkStorage, /loadProjectData\(projectId, nextConfig, nextStorageId, workflowEpoch\)/);

  const saveStorage = section(workspace, "saveStorageConfig", "saveInlineStorage");
  assert.match(saveStorage, /workflowEpoch = hostWorkflowEpoch\.current/);
  assert.match(saveStorage, /await projectSyncApi\.getConfig\(\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return false;/);
  assert.match(saveStorage, /await projectSyncApi\.saveConfig\(update\(current\)\);\s*if \(hostWorkflowEpoch\.current !== workflowEpoch\) return false;/);

  const clearStorage = section(workspace, "clearStorageData", "removeStorage");
  capturesBefore(clearStorage, "await confirm(");
  assert.match(clearStorage, /await projectSyncApi\.clearStorageData\(/);
  assert.match(clearStorage, /Permanently clear all Mallard data/);

  const removeStorage = section(workspace, "removeStorage", "openNewStorageConfiguration");
  capturesBefore(removeStorage, "await confirm(");
  assert.match(removeStorage, /Stored files were not deleted\.`, workflowEpoch\)/);

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
  assert.match(workspace, /menuHeader: \{\s*label: workspaceStorage\.name \|\| "\(unnamed\)",/);
  assert.match(workspace, /label: "Storage settings…"/);
  assert.match(workspace, /label: `Unlink from \$\{projectLabel\(workspaceProject\)\}…`/);
  assert.match(workspace, /icon: "link-off" as const/);
  assert.match(workspace, /disabledReason: workspaceAvailableStorages\.length === 0/);
  assert.doesNotMatch(workspace, /checked: storage\.id === workspaceStorage\.id/);
  assert.doesNotMatch(workspace, /label: "Unlink storage"/);

  // The workspace ⋯ holds app scope alone: the way into Settings and the full
  // reload. The theme checked-item moved onto the Settings page's Appearance
  // row — the menu holds commands, not state.
  assert.match(workspace, /const pathOverflow: PathBarMenuGroup\[\] = workspaceProject \? appActions : \[\];/);
  assert.match(sync, /label: "Settings",\s*icon: "settings" as const,\s*onSelect: openSettings,/);
  assert.doesNotMatch(sync, /label: "Dark theme"/);

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
  assert.match(reconcile, /hostWorkflowEpoch\.current !== workflowEpoch/);
  assert.match(reconcile, /await refresh\(workflowEpoch\)/);
  assert.match(workspace, /Date\.now\(\) - blurredAt >= FOCUS_RECONCILE_MIN_AWAY_MS/);
});

// Each handle must move the one gutter it borders. The panes below the head and
// above Catalog are sized by share, so any change to the box they divide slides
// their gutters unless the share is re-solved against the new box.
test("every sidebar handle moves its own gutter and no other", () => {
  const BOX = 700;
  const CATALOG = 175;
  const elastic = BOX - 6 - CATALOG;
  const projects = 210;
  const storages = elastic - projects;

  // Head handle: the box shrinks by what the head takes, and Storages must come
  // out of it unchanged — Projects, the pane it borders, absorbs the whole move.
  const shrunkBox = BOX - 60;
  const heldShare = shareHoldingStorages(storages, shrunkBox, CATALOG);
  const shrunkElastic = shrunkBox - 6 - CATALOG;
  assert.ok(Math.abs((1 - heldShare) * shrunkElastic - storages) < 0.5, "Storages holds its pixels");
  assert.ok(Math.abs(heldShare * shrunkElastic - (projects - 60)) < 0.5, "Projects absorbs the change");

  // Catalog handle: the elastic space shrinks as Catalog grows, and Projects is
  // the pane that must not move.
  const grown = CATALOG + 40;
  const share = shareHolding(projects, BOX, grown);
  const grownElastic = BOX - 6 - grown;
  assert.ok(Math.abs(share * grownElastic - projects) < 0.5, "Projects holds its pixels");
  assert.ok(Math.abs((1 - share) * grownElastic - (storages - 40)) < 0.5, "Storages absorbs the change");
});

test("the Catalog pane floors at the same height as the panes above it", () => {
  // It used to floor at its own content height, so the bottom handle refused to
  // move while the two above travelled down to MIN_PANE_HEIGHT.
  const [min, max] = catalogHeightRange(700, 210);
  assert.equal(min, MIN_PANE_HEIGHT);
  assert.equal(max, 700 - 6 - 210 - MIN_PANE_HEIGHT);
  // The list scrolls unconditionally — the unbounded row list needs it past
  // the 40% cap as much as under a pinned height — so the old is-pinned
  // overflow switch stays deleted.
  const css = readFileSync("src/App.css", "utf8");
  assert.doesNotMatch(css, /\.sidebar-catalog-pane\.is-pinned/);
  assert.match(
    css,
    /\.sidebar-split-pane > \.sidebar-profile-list,\s*\.sidebar-split-pane > \.sidebar-links-list\s*\{[^}]*overflow-y: scroll;/s,
  );
});

test("a custom profile path can be typed, and ~ resolves before the probe", () => {
  // The backend takes absolute clean paths only, so the tilde has to be gone
  // by the time a path is handed to it.
  assert.equal(expandHomePath("~", "/Users/ada"), "/Users/ada");
  assert.equal(expandHomePath("~/work/codex-home", "/Users/ada"), "/Users/ada/work/codex-home");
  assert.equal(expandHomePath("~/work", "/Users/ada/"), "/Users/ada/work", "a trailing slash must not double up");
  assert.equal(expandHomePath("  ~/work  ", "/Users/ada"), "/Users/ada/work");
  assert.equal(expandHomePath("/opt/codex", "/Users/ada"), "/opt/codex", "absolute paths pass through");
  assert.equal(expandHomePath("~other/work", "/Users/ada"), "~other/work", "only this user's home expands");
  assert.equal(expandHomePath("~/work", null), "~/work", "unresolved home leaves the text alone");

  // Choosing "Custom path…" must open the field rather than the folder
  // dialog: a pasted path, or one on an unmounted volume, cannot come back
  // from a picker. Browse stays as the other way in.
  const setupWorkspace = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  const pendingBranch = setupWorkspace.slice(
    setupWorkspace.indexOf('if (next === "__pending")'),
    setupWorkspace.indexOf('<option value="">Choose profile'),
  );
  assert.ok(pendingBranch.length > 0, "the __pending branch must be found");
  assert.match(pendingBranch, /kind: "pending", path: "", display_name: ""/);
  assert.doesNotMatch(pendingBranch, /chooseProfileFolder/);
  assert.match(setupWorkspace, /className="btn" onClick=\{\(\) => void chooseProfileFolder\(provider\)\}/);
});

test("the binding dialog wears the setup page's language", () => {
  const editor = readFileSync("src/components/project-sync/ProjectBindingEditor.tsx", "utf8");

  // The body is the setup page's bands, and the profile row is the setup
  // page's row: profiles and a Custom path… choice in one select, an inline
  // path field, Browse, and the judged-as-typed hint.
  assert.match(editor, /sync-setup-section/);
  assert.match(editor, /<option value="__pending">Custom path…<\/option>/);
  assert.match(editor, /<Icon name="folder" size=\{13\} \/> Browse/);
  assert.match(editor, /sync-setup-path-hint/);
  assert.match(editor, /useProfilePathProbes|profilePathHint/);

  // An active binding renders as a fact sheet: read-only checkout, each
  // agent's profile shown as its name and path, and one Done button. Only an
  // unbound or unconfirmed project still gets the editor.
  assert.match(editor, /readOnly title=\{projectRoot\}/);
  assert.match(editor, /binding\.locked \? \(/);
  assert.match(editor, /sync-provider-name/);
  assert.match(editor, /<code>\{compactProjectPath\(profile\.path\)\}<\/code>/);
  assert.match(editor, /Profile missing on this machine/);
  assert.match(editor, />Done<\/button>/);
  assert.doesNotMatch(editor, /onChange=\{\(event\) => setProjectRoot/);

  // The old dialog's furniture stays deleted: the banner, the folder card,
  // the header paragraph, and the parent-driven Add flow.
  assert.doesNotMatch(editor, /sync-binding-safety|sync-folder-field|onAddProfile/);
  assert.doesNotMatch(editor, /\{description\}/);

  // Typed paths ride the submission as pending selections; ProjectSync
  // registers them before the binding command, which takes ids only.
  assert.match(editor, /kind: "pending", path: expandHomePath\(selection\.path, homePath\)/);
  const workspace = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(workspace, /ensureProfileAtPath\(provider, selection\.path, workflowEpoch\)/);
  assert.match(workspace, /locked: projectBinding\?\.state === "active"/);
  assert.doesNotMatch(workspace, /chooseAndCreateProfile|onAddProfile/);

  // The shared row can never outgrow its column, and hands the path field its
  // own line when the column gets too tight to share.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-agent-profile-row\.has-custom-path \{\s*grid-template-columns: minmax\(140px, 220px\) minmax\(0, 1fr\) auto;\s*\}/);
  assert.match(css, /\.sync-agent-profile-list \{ container-type: inline-size; \}/);
  assert.match(css, /@container \(max-width: 480px\)/);
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
