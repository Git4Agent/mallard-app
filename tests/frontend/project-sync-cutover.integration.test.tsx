import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import StartupLoadingScreen from "../../src/components/project-sync/StartupLoadingScreen";
import {
  repositoryFingerprintMatches,
  repositoryFingerprintsRequireAcknowledgement,
} from "../../src/components/project-sync/model";
import { expandHomePath } from "../../src/profilePath";
import { PROJECT_SYNC_SCHEMA } from "../../src/types";
import {
  profilePathHint,
  type ProfilePathProbeState,
} from "../../src/components/project-sync/ProjectSetupWorkspace";

test("App mounts ProjectSync", () => {
  const app = readFileSync("src/App.tsx", "utf8");
  assert.match(app, /<ProjectSync/);
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
  assert.match(css, /\.sync-sidebar-toggle \{[^}]*top: 20px;[^}]*left: 92px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar \{[^}]*padding-left: 146px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar::before \{[^}]*left: 129px;/);
  assert.match(css, /\.sync-titlebar \{[^}]*flex: 0 0 auto;/);
  assert.match(css, /\.sync-titlebar \{[^}]*min-height: 3\.25rem;/);
  assert.match(css, /\.sync-sidebar-drag\s*\{[^}]*height: 42px;/s);
  assert.match(css, /\.sync-sidebar-brand\s*\{[^}]*height: 56px;/s);
  assert.deepEqual(tauriConfig.app.windows[0].trafficLightPosition, { x: 26, y: 35 });
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
  assert.match(shell, /const MIN_PROJECT_WORKSPACE_WIDTH = 580;/);
  assert.match(shell, /window\.innerWidth - MIN_PROJECT_WORKSPACE_WIDTH/);
  assert.match(css, /\.sync-pathbar \.sync-history-branch-name,[\s\S]*?display: none;/);
});

test("top-level navigation opens stable index pages", () => {
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");

  // The strip names each root once, in rail order, and each tab carries the
  // destination in its own accessible name because it draws no text.
  assert.match(sidebar, /aria-label="Projects"[\s\S]*?aria-label="Catalog"[\s\S]*?aria-label="Storages"/);
  assert.match(sidebar, /onClick=\{onOpenProjects\}/);
  assert.match(sidebar, /onClick=\{onOpenCatalogIndex\}/);
  assert.match(sidebar, /onClick=\{onOpenStorageIndex\}/);
  assert.doesNotMatch(sidebar, /primaryStorageId|onOpenCatalogMenu/);

  assert.match(shell, /const openProjectsIndex = async \(\) => \{[\s\S]*?setActiveProjectId\(null\);/);
  assert.match(shell, /const openCatalogIndex = async \(\) => \{[\s\S]*?setCatalogIndexOpen\(true\);/);
  assert.match(shell, /catalogIndexOpen \? \(\s*<CatalogIndexPage/);
  assert.match(shell, /storageIndexOpen \? \(\s*<StorageIndexPage/);
  assert.doesNotMatch(shell, /const openCatalogMenu/);

  assert.match(workspace, /className="sync-storage-index-list" aria-label="Projects"/);
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
  assert.match(workspace, /type SyncNoticeTone = "ok" \| "attention"/);
  assert.match(workspace, /severity: notice\.tone === "attention" \? "warning" : "success"/);
  // An unconfirmed binding is a stop, not a success.
  assert.match(workspace, /before pulling\.",\s*"attention"\)/);
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
    /const workspaceTitle = activeDraftSummary\?\.display_name\s*\?\? \(choosingProjectFolder \? "New project" : null\)\s*\?\? \(addingCatalogScope \? "Add Catalog resource" : null\)\s*\?\? \(activeSummary \? null : "Projects"\);/,
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

  // Add project writes nothing: it opens setup at the Folder field. The draft
  // is born when that step chooses a folder, inside its own epoch.
  const addProject = section(workspace, "beginAddProject", "chooseProjectFolder");
  assert.match(addProject, /setSetupView\(\{ kind: "folder" \}\)/);
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
  assert.match(keepBundle, /openPushChooser\(pending\.projectId, pending\.storageId, workflowEpoch\)/);

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
  assert.match(clearStorage, /Permanently clear all Mallard data/);

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
  assert.match(workspace, /menuHeader: \{\s*label: workspaceStorage\.name \|\| "\(unnamed\)",/);
  assert.match(workspace, /label: "Storage details…"/);
  assert.match(workspace, /label: `Unlink from \$\{projectLabel\(workspaceProject\)\}…`/);
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
  assert.doesNotMatch(workspace, /label: "Unlink storage"/);

  // The workspace ⋯ holds app scope alone: Settings, the theme switch, and
  // the full reload. The checked item exposes the current theme state.
  assert.match(workspace, /const pathOverflow: PathBarMenuGroup\[\] = workspaceProject \? appActions : \[\];/);
  assert.match(sync, /label: "Settings",\s*icon: "settings" as const,\s*onSelect: openSettings,/);
  assert.match(sync, /label: "Dark theme",\s*icon: "moon" as const,\s*checked: theme === "dark",/);

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
  assert.match(workspace, /workspaceTitle === "Projects" \? null : \(\s*<button[\s\S]*?className="sync-theme-button"/);
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

  // No handle and no stored height: the same three rows at every window size.
  assert.match(css, /\.sync-sidebar-head \{[^}]*height: 148px;[^}]*flex: 0 0 148px;/);
  // The rail takes what is left and its own list scrolls, so the window's
  // height reaches the rows instead of a tray wrapped around them.
  assert.match(css, /\.sync-sidebar-rail \{[^}]*min-height: 0;[^}]*flex: 1;/);
  assert.match(css, /\.sidebar-rail-list \{[^}]*flex: 1;[^}]*overflow-y: auto;/);

  const GUTTER = 8;  // --retro-gutter: the frame, and the seam between tiles
  const HEAD = 148;  // 42 drag strip + 56 wordmark + 46 tab strip + 4 border
  const MARK = 44;   // .sync-sidebar-product-mark min-height
  const minHeight = config.app.windows[0].minHeight;
  assert.equal(minHeight, 560);
  const list = minHeight - (2 * GUTTER + HEAD + 2 * GUTTER + MARK);
  assert.equal(list, 336, "the rail's height at the window floor");

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
    setupWorkspace.indexOf("<div><strong>Storage</strong></div>"),
    setupWorkspace.indexOf("{storageSection?.message && storageSection.state === \"blocked\""),
  );
  assert.ok(band.length > 0, "missing the Storage band");

  // "Local only" was a mode standing among proper nouns, so the row read as
  // two answers to two different questions. The slot is named the way the
  // rest of the app names it — the projects landing card says "No storage
  // linked", the catalog crumb "No storage" — and `yet` carries the fact that
  // Link storage… is on the project's own menu afterwards.
  assert.match(band, /<span className="setup-pill-label">None yet<\/span>/);
  assert.doesNotMatch(setupWorkspace, /Local only/);

  // The glyph still says where the sessions live, which is why the label does
  // not have to.
  assert.match(band, /<Icon name="computer" size=\{12\} \/>\s*<span className="setup-pill-label">None yet/);

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
  assert.match(dialog, /"Connect"/);
  assert.match(workspace, /await connectStorageBundle\(projectId, storageId, project\.bundle_id\)/, "same-bundle local storage links use the same connection flow");
  assert.equal(PROJECT_SYNC_SCHEMA, 1);
});
