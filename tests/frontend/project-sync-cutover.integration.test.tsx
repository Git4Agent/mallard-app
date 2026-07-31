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
  assert.match(css, /\.sync-sidebar-toggle \{[^}]*top: 14px;[^}]*left: 87px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar \{[^}]*padding-left: 140px;/);
  assert.match(css, /\.sync-app\.sidebar-hidden \.sync-titlebar::before \{[^}]*left: 123px;/);
  assert.match(css, /\.sync-titlebar \{[^}]*height: 52px;[^}]*min-height: 52px;[^}]*flex: 0 0 52px;/);
  assert.match(css, /\.sync-sidebar-drag \{[^}]*height: 38px;/);
  assert.match(css, /\.sync-sidebar-brand \{[^}]*height: 52px;/);
  assert.deepEqual(tauriConfig.app.windows[0].trafficLightPosition, { x: 8, y: 29 });
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
