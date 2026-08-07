# Excerpts: frontend (src/)

Skill/plugin sync UI removed. `SkillsPluginStatusPage.tsx` was removed whole and is archived
verbatim alongside this file. This is the removal diff for the surviving frontend files, taken
against commit a9e428e.

```diff
diff --git a/src/components/project-sync/ProjectLinksWorkspace.tsx b/src/components/project-sync/ProjectLinksWorkspace.tsx
index 74242e2..06a15c0 100644
--- a/src/components/project-sync/ProjectLinksWorkspace.tsx
+++ b/src/components/project-sync/ProjectLinksWorkspace.tsx
@@ -3,7 +3,6 @@ import {
   useMemo,
   useRef,
   useState,
-  type KeyboardEvent as ReactKeyboardEvent,
   type ReactNode,
 } from "react";
 import type {
@@ -19,7 +18,6 @@ import type {
 } from "../../types";
 import Icon from "../Icons";
 import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
-import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
 import ConversationPathRepairNotice from "./ConversationPathRepairNotice";
 import { newStorage, storageConfigReady, StorageEditor } from "./StorageSettings";
 import { projectSyncApi } from "./api";
@@ -34,7 +32,6 @@ import {
 } from "./model";

 type LinkKey = { projectId: string; storageId: string };
-export type ProjectWorkspaceTab = "history" | "skills" | "plugins";
 export type StorageReviewKind = "pull" | "push";
 type StorageEditorRequest =
   | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
@@ -83,7 +80,6 @@ interface Props {
   onStorageIssueChange?: (storageId: string, issue: string | null) => void;
   newProjectSetup?: ReactNode;
   historyRefreshEpoch?: number;
-  projectTabResetEpoch?: number;
   inlineStorageReview?: InlineStorageReview | null;
 }

@@ -93,93 +89,6 @@ function storageSubtitle(storage: StorageConfig): string {
   return storage.s3_endpoint ? "S3 endpoint configured" : "S3 storage not configured";
 }

-export function ProjectWorkspaceTabs({
-  activeTab,
-  isGitRepository,
-  onChange,
-}: {
-  activeTab: ProjectWorkspaceTab;
-  isGitRepository: boolean;
-  onChange: (tab: ProjectWorkspaceTab) => void;
-}) {
-  const tabOrder: ProjectWorkspaceTab[] = ["history", "skills", "plugins"];
-  const selectFromKeyboard = (
-    event: ReactKeyboardEvent<HTMLButtonElement>,
-    tab: ProjectWorkspaceTab,
-  ) => {
-    event.preventDefault();
-    const tabList = event.currentTarget.parentElement;
-    onChange(tab);
-    window.requestAnimationFrame(() => {
-      tabList
-        ?.querySelector<HTMLButtonElement>(`[data-project-tab="${tab}"]`)
-        ?.focus();
-    });
-  };
-  const handleKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
-    const currentTab = event.currentTarget.dataset.projectTab as ProjectWorkspaceTab;
-    const currentIndex = tabOrder.indexOf(currentTab);
-    if (event.key === "Home") return selectFromKeyboard(event, tabOrder[0]);
-    if (event.key === "End") return selectFromKeyboard(event, tabOrder[tabOrder.length - 1]);
-    if (event.key === "ArrowLeft") {
-      return selectFromKeyboard(event, tabOrder[(currentIndex - 1 + tabOrder.length) % tabOrder.length]);
-    }
-    if (event.key === "ArrowRight") {
-      selectFromKeyboard(event, tabOrder[(currentIndex + 1) % tabOrder.length]);
-    }
-  };
-
-  return (
-    <div className="sync-project-tabs" role="tablist" aria-label="Project information">
-      <button
-        type="button"
-        id="project-history-tab"
-        data-project-tab="history"
-        role="tab"
-        aria-selected={activeTab === "history"}
-        aria-controls="project-history-panel"
-        tabIndex={activeTab === "history" ? 0 : -1}
-        className={activeTab === "history" ? "active" : undefined}
-        onClick={() => onChange("history")}
-        onKeyDown={handleKeyDown}
-      >
-        <Icon name={isGitRepository ? "git-branch" : "openai"} size={14} />
-        {isGitRepository ? "Git & sessions" : "Sessions"}
-      </button>
-      <button
-        type="button"
-        id="project-skills-tab"
-        data-project-tab="skills"
-        role="tab"
-        aria-selected={activeTab === "skills"}
-        aria-controls="project-skills-panel"
-        tabIndex={activeTab === "skills" ? 0 : -1}
-        className={activeTab === "skills" ? "active" : undefined}
-        onClick={() => onChange("skills")}
-        onKeyDown={handleKeyDown}
-      >
-        <Icon name="folder" size={14} />
-        Skills
-      </button>
-      <button
-        type="button"
-        id="project-plugins-tab"
-        data-project-tab="plugins"
-        role="tab"
-        aria-selected={activeTab === "plugins"}
-        aria-controls="project-plugins-panel"
-        tabIndex={activeTab === "plugins" ? 0 : -1}
-        className={activeTab === "plugins" ? "active" : undefined}
-        onClick={() => onChange("plugins")}
-        onKeyDown={handleKeyDown}
-      >
-        <Icon name="link" size={14} />
-        Plugins
-      </button>
-    </div>
-  );
-}
-
 export function StorageRepositoryRow({ bundle }: { bundle: RemoteBundleSummary }) {
   const name = bundle.display_name || "Unnamed repository";
   return (
@@ -295,7 +204,6 @@ export default function ProjectLinksWorkspace({
   onStorageIssueChange,
   newProjectSetup,
   historyRefreshEpoch = 0,
-  projectTabResetEpoch = 0,
   inlineStorageReview,
 }: Props) {
   const [linkingProjectId, setLinkingProjectId] = useState<string | null>(null);
@@ -310,7 +218,6 @@ export default function ProjectLinksWorkspace({
   const [bundleError, setBundleError] = useState<string | null>(null);
   const [renamingProjectId, setRenamingProjectId] = useState<string | null>(null);
   const [projectAliasDraft, setProjectAliasDraft] = useState("");
-  const [activeProjectTab, setActiveProjectTab] = useState<ProjectWorkspaceTab>("history");
   const [storagePickerProjectId, setStoragePickerProjectId] = useState<string | null>(null);
   const [storageActionsProjectId, setStorageActionsProjectId] = useState<string | null>(null);
   const bundleRequestRef = useRef(0);
@@ -344,10 +251,6 @@ export default function ProjectLinksWorkspace({
     setStorageActionsProjectId(null);
   }, [activeProjectId, activeStorageId]);

-  useEffect(() => {
-    setActiveProjectTab("history");
-  }, [projectTabResetEpoch]);
-
   useEffect(() => {
     if (!inlineStorageReview) return;
     setStoragePickerProjectId(null);
@@ -1514,20 +1417,7 @@ export default function ProjectLinksWorkspace({
         ) : null)}

         {activeProject && !newProjectSetup && !inlineStorageReview && (
-          <ProjectWorkspaceTabs
-            activeTab={activeProjectTab}
-            isGitRepository={!!activeProject.is_git_repository}
-            onChange={setActiveProjectTab}
-          />
-        )}
-
-        {activeProject && !newProjectSetup && !inlineStorageReview && activeProjectTab === "history" && (
-          <div
-            id="project-history-panel"
-            className="sync-project-tab-panel"
-            role="tabpanel"
-            aria-labelledby="project-history-tab"
-          >
+          <div id="project-history-panel" className="sync-project-tab-panel">
             <ProjectChatHistoryPage
               embedded
               project={activeProject}
@@ -1538,32 +1428,6 @@ export default function ProjectLinksWorkspace({
             />
           </div>
         )}
-
-        {activeProject && !newProjectSetup && !inlineStorageReview && (
-          activeProjectTab === "skills" || activeProjectTab === "plugins"
-        ) && (
-          <div
-            id={`project-${activeProjectTab}-panel`}
-            className="sync-project-tab-panel"
-            role="tabpanel"
-            aria-labelledby={`project-${activeProjectTab}-tab`}
-          >
-            <SkillsPluginStatusPage
-              view={activeProjectTab}
-              project={activeProject}
-              binding={activeBindingByProject.get(activeProject.local_project_id) ?? null}
-              refreshEpoch={historyRefreshEpoch}
-              activeStorageId={activeStorageId}
-              activeStorageName={storages.find((storage) => storage.id === activeStorageId)?.name ?? null}
-              onOpenProjectSettings={() => {
-                document.getElementById("project-configuration-panel")?.scrollIntoView({
-                  block: "start",
-                  behavior: "smooth",
-                });
-              }}
-            />
-          </div>
-        )}
       </section>

     </main>
diff --git a/src/components/project-sync/ProjectSync.tsx b/src/components/project-sync/ProjectSync.tsx
index 0649184..f8b33e3 100644
--- a/src/components/project-sync/ProjectSync.tsx
+++ b/src/components/project-sync/ProjectSync.tsx
@@ -15,10 +15,7 @@ import type {
   BundleRecipe,
   BundleReadiness,
   BundleSnapshotSummary,
-  CapabilityStatusReport,
   CodexConversationPathAudit,
-  DependencyPlan,
-  DependencyResult,
   HostIdentityStatus,
   LocalProjectRegistration,
   LocalProjectSummary,
@@ -164,7 +161,6 @@ interface PendingPush {
   savedRecipe: BundleRecipe | null;
   projectDefaults: Set<string>;
   threadComparison?: ThreadSyncComparison | null;
-  capabilityReport?: CapabilityStatusReport | null;
   projectContentInventory: ProjectContentInventory | null;
   projectContentScanned: boolean;
   showProjectFiles: boolean;
@@ -307,11 +303,8 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
   const [fetchProgress, setFetchProgress] = useState<BundleFetchProgress | null>(null);
   const [restoreBinding, setRestoreBinding] = useState<ProjectBinding | null>(null);
   const [restoreProjectName, setRestoreProjectName] = useState("project");
-  const [dependencyPlan, setDependencyPlan] = useState<DependencyPlan | null>(null);
   const [restoreResult, setRestoreResult] = useState<RestoreResult | null>(null);
   const [historyRefreshEpoch, setHistoryRefreshEpoch] = useState(0);
-  const [projectTabResetEpoch, setProjectTabResetEpoch] = useState(0);
-  const [dependencyResult, setDependencyResult] = useState<DependencyResult | null>(null);
   const [restoreError, setRestoreError] = useState<string | null>(null);
   const [pullApplyPhase, setPullApplyPhase] = useState<PullApplyPhase>("idle");
   const [reviewSupportLoading, setReviewSupportLoading] = useState(false);
@@ -637,9 +630,7 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     setEditingBinding(null);
     setRestorePlan(null);
     setRestoreBinding(null);
-    setDependencyPlan(null);
     setRestoreResult(null);
-    setDependencyResult(null);
     setRestoreError(null);
     setPullApplyPhase("idle");
     setReviewSupportLoading(false);
@@ -1333,7 +1324,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       // Degraded sections (storage offline, capture failures) open the dialog
       // without their comparison, exactly like the old per-command flow.
       const nextThreadComparison = context.thread_comparison ?? undefined;
-      const nextCapabilityReport = context.capability_report ?? undefined;
       const isGitRepository = repositoryKinds[projectId] ?? false;
       const lockedProjectContentInventory =
         context.project_files && context.project_files.entries.length > 0
@@ -1343,7 +1333,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         recipeSelection(nextInventory.recipe),
         nextResources,
         nextThreadComparison,
-        nextCapabilityReport,
       );
       const projectSummary: LocalProjectSummary = {
         local_project_id: nextDetail.project.local_project_id,
@@ -1377,7 +1366,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         savedRecipe,
         projectDefaults,
         threadComparison: nextThreadComparison,
-        capabilityReport: nextCapabilityReport,
         projectContentInventory: lockedProjectContentInventory,
         projectContentScanned: lockedProjectContentInventory !== null,
         showProjectFiles: !isGitRepository || lockedProjectContentInventory !== null,
@@ -1538,7 +1526,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       if (hostWorkflowEpoch.current !== workflowEpoch) return;
       setNotice(result.message);
       setHistoryRefreshEpoch((epoch) => epoch + 1);
-      setProjectTabResetEpoch((epoch) => epoch + 1);
       setPendingPush(null);
       setPushProjectContentRemovals(new Set());
       setPushWarningAcknowledgements(new Set());
@@ -1589,7 +1576,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       pushSelected,
       resources,
       pendingPush.threadComparison,
-      pendingPush.capabilityReport,
       pendingPush.projectContentInventory,
     );
     const preservationRecipe: BundleRecipe = {
@@ -1693,7 +1679,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     setBusy(true);
     setRestoreError(null);
     setRestoreResult(null);
-    setDependencyResult(null);
     setFetchProgress(null);
     setPullApplyPhase("idle");
     setReviewSupportLoading(true);
@@ -1701,7 +1686,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     setCompletedPullResourceIds(new Set());
     setFailedPullResourceIds(new Set());
     setRestorePlan(null);
-    setDependencyPlan(null);
     setReadiness(null);
     setRestoreBinding(targetBinding);
     setRestoreProjectName(displayName);
@@ -1750,7 +1734,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       void review.support
         .then((support) => {
           if (restoreRequest.current !== requestId) return;
-          setDependencyPlan(support.dependencyPlan);
           setReadiness(support.readiness);
           if (support.errors.length > 0) {
             setRestoreError(`The project changes are ready to apply. ${support.errors.join(" ")}`);
@@ -1948,9 +1931,7 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     restoreRequest.current += 1;
     setRestorePlan(null);
     setRestoreBinding(null);
-    setDependencyPlan(null);
     setRestoreResult(null);
-    setDependencyResult(null);
     setRestoreError(null);
     setPullApplyPhase("idle");
     setReviewSupportLoading(false);
@@ -2050,7 +2031,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     const requestId = restoreRequest.current;
     const workflowEpoch = hostWorkflowEpoch.current;
     const currentRestorePlan = restorePlan;
-    const currentDependencyPlan = dependencyPlan;
     // The review-ready notice describes the pre-Apply state. Clear it before
     // execution so a completed Pull can never still claim that nothing was
     // applied.
@@ -2061,7 +2041,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       return next;
     });
     if (selection.restoreActionIds.length > 0) setRestoreResult(null);
-    if (selection.dependencyActionIds.length > 0) setDependencyResult(null);
     openActivity();
     setBusy(true);
     setRestoreError(null);
@@ -2069,29 +2048,16 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       const result = await applyPullReview(
         projectSyncApi,
         currentRestorePlan,
-        currentDependencyPlan,
         selection,
         (phase) => {
           if (restoreRequest.current === requestId) setPullApplyPhase(phase);
         },
       );
       if (restoreRequest.current !== requestId) return;
-      const pullMutationCompleted = (
-        result.restoreResult !== null || result.dependencyResult !== null
-      ) && (
-        result.restoreResult?.success ?? true
-      ) && (
-        result.dependencyResult?.success ?? true
-      );
-      if (result.success || (!result.error && pullMutationCompleted)) {
-        setProjectTabResetEpoch((epoch) => epoch + 1);
-      }
       if (result.restoreResult) {
         setRestoreResult(result.restoreResult);
       }
-      if (result.dependencyResult) setDependencyResult(result.dependencyResult);
-      if (result.restoreResult?.success
-        || (result.dependencyResult?.applied_action_ids?.length ?? 0) > 0) {
+      if (result.restoreResult?.success) {
         setHistoryRefreshEpoch((epoch) => epoch + 1);
       }
       if (result.readiness) setReadiness(result.readiness);
@@ -2107,22 +2073,13 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
           action.source_resource_id ?? `${action.provider}:conversation:${action.thread_id}`,
         );
       }
-      for (const action of currentDependencyPlan?.actions ?? []) {
-        resourceByAction.set(action.action_id, action.resource_id);
-      }
-      const appliedActionIds = new Set([
-        ...(result.restoreResult?.applied_action_ids ?? []),
-        ...(result.dependencyResult?.applied_action_ids ?? []),
-      ]);
-      const failedActionIds = new Set([
-        ...(result.restoreResult?.failed_actions ?? []).map((failure) => failure.action_id),
-        ...(result.dependencyResult?.failed_actions ?? []).map((failure) => failure.action_id),
-      ]);
+      const appliedActionIds = new Set(result.restoreResult?.applied_action_ids ?? []);
+      const failedActionIds = new Set(
+        (result.restoreResult?.failed_actions ?? []).map((failure) => failure.action_id),
+      );
       const interruptedActionIds = result.failedPhase === "restoring"
         ? selection.restoreActionIds
-        : result.failedPhase === "installing"
-          ? selection.dependencyActionIds
-          : [];
+        : [];
       for (const actionId of interruptedActionIds) {
         if (!appliedActionIds.has(actionId)) failedActionIds.add(actionId);
       }
@@ -2137,7 +2094,7 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
           .map((actionId) => resourceByAction.get(actionId))
           .filter((resourceId): resourceId is string => Boolean(resourceId)),
       );
-      const selectedActions = [...selection.restoreActionIds, ...selection.dependencyActionIds];
+      const selectedActions = selection.restoreActionIds;
       const successfulResources = new Set(selection.resourceIds.filter((resourceId) => {
         const actionIds = selectedActions.filter((actionId) => resourceByAction.get(actionId) === resourceId);
         return actionIds.length > 0 && actionIds.every((actionId) => appliedActionIds.has(actionId));
@@ -2161,18 +2118,14 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         return;
       }

-      // Refresh both aligned plans after a partial pass so remaining setup is
+      // Refresh the plan after a partial pass so remaining setup is
       // presented against one current revision.
-      const approvedThisPass = new Set([
-        ...selection.restoreActionIds,
-        ...selection.dependencyActionIds,
-      ]);
-      const hasSkippedActions = [
-        ...currentRestorePlan.actions.map((action) => action.action_id),
-        ...(currentDependencyPlan?.actions ?? []).map((action) => action.action_id),
-      ].some((actionId) => (
-        !completedPullActionIds.has(actionId) && !approvedThisPass.has(actionId)
-      ));
+      const approvedThisPass = new Set(selection.restoreActionIds);
+      const hasSkippedActions = currentRestorePlan.actions
+        .map((action) => action.action_id)
+        .some((actionId) => (
+          !completedPullActionIds.has(actionId) && !approvedThisPass.has(actionId)
+        ));
       if (!result.success || hasSkippedActions) {
         const nextRestore = await projectSyncApi.planRestore(
           currentRestorePlan.storage_id,
@@ -2183,7 +2136,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         setRestorePlan(nextRestore);
         const support = await loadPullReviewSupport(projectSyncApi, nextRestore);
         if (restoreRequest.current !== requestId) return;
-        setDependencyPlan(support.dependencyPlan);
         setReadiness(support.readiness);
         if (support.errors.length > 0) {
           setRestoreError(`Changes were applied, but remaining setup could not be prepared: ${support.errors.join(" ")}`);
@@ -2232,7 +2184,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       }
       const support = await loadPullReviewSupport(projectSyncApi, nextRestore);
       if (restoreRequest.current !== requestId) return;
-      setDependencyPlan(support.dependencyPlan);
       setReadiness(support.readiness);
       if (support.errors.length > 0) {
         setRestoreError(`${resolvedNote}. Remaining checks: ${support.errors.join(" ")}`);
@@ -2282,7 +2233,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         });
       }
       setHistoryRefreshEpoch((epoch) => epoch + 1);
-      setProjectTabResetEpoch((epoch) => epoch + 1);
       const remaining = result.codex_fork_checkpoints.length;
       if (remaining > 0) {
         setNotice(`Codex fork verified. ${remaining} more fork checkpoint${remaining === 1 ? "" : "s"} to resolve.`);
@@ -2372,14 +2322,12 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       if (restoreRequest.current !== requestId) return;
       if (nextRestore.revision !== currentPlan.revision || nextRestore.manifest_sha256 !== currentPlan.manifest_sha256) {
         setRestoreResult(null);
-        setDependencyResult(null);
         setPullApplyPhase("idle");
         setCompletedPullActionIds(new Set());
         setCompletedPullResourceIds(new Set());
         setFailedPullResourceIds(new Set());
       }
       setRestorePlan(nextRestore);
-      setDependencyPlan(support.dependencyPlan);
       setReadiness(support.readiness);
       if (support.errors.length > 0) setRestoreError(support.errors.join(" "));
     } catch (reason) {
@@ -2494,7 +2442,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
       pushSelected,
       pushResources(pendingPush),
       pendingPush.threadComparison,
-      pendingPush.capabilityReport,
       pendingPush.projectContentInventory,
     )
     : pushSelected;
@@ -2502,7 +2449,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
     ? pushReviewBlockingCount(
       safePushSelection,
       pendingPush.threadComparison,
-      pendingPush.capabilityReport,
       pendingPush.projectContentInventory,
       pushWarningAcknowledgements,
     )
@@ -2546,7 +2492,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         storageName={pendingPush.storage.name}
         refreshEpoch={historyRefreshEpoch}
         threadComparison={pendingPush.threadComparison}
-        capabilityReport={pendingPush.capabilityReport}
         projectContentInventory={pendingPush.projectContentInventory}
         projectContentScanned={pendingPush.projectContentScanned}
         projectContentLoading={projectContentLoading}
@@ -2647,10 +2592,8 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
         profileLabel={restoreProfileLabel}
         plan={restorePlan}
         binding={restoreBinding}
-        dependencyPlan={dependencyPlan}
         readiness={readiness}
         restoreResult={restoreResult}
-        dependencyResult={dependencyResult}
         phase={pullApplyPhase}
         supportLoading={reviewSupportLoading}
         completedActionIds={completedPullActionIds}
@@ -2886,7 +2829,6 @@ export default function ProjectSync({ theme, onThemeChange }: Props) {
             onStorageEditorChange={setActiveStorageSettingsId}
             onStorageIssueChange={updateStorageIssue}
             historyRefreshEpoch={historyRefreshEpoch}
-            projectTabResetEpoch={projectTabResetEpoch}
             inlineStorageReview={inlineStorageReview}
             newProjectSetup={setupDraftId ? (
               <ProjectSetupWorkspace
diff --git a/src/components/project-sync/PushResourceWorkspace.tsx b/src/components/project-sync/PushResourceWorkspace.tsx
index d2f9b11..59c18d8 100644
--- a/src/components/project-sync/PushResourceWorkspace.tsx
+++ b/src/components/project-sync/PushResourceWorkspace.tsx
@@ -2,7 +2,6 @@ import { useEffect, useMemo, useState } from "react";
 import type {
   BundleFetchProgress,
   BundleRecipe,
-  CapabilityStatusReport,
   LocalProjectSummary,
   ProjectBinding,
   ProjectContentInventory,
@@ -15,7 +14,6 @@ import Icon from "../Icons";
 import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
 import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
 import ResourceInventory from "./ResourceInventory";
-import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
 import SyncReviewTabs, {
   type SyncReviewStep,
   syncReviewSteps,
@@ -25,13 +23,10 @@ import { categoryFor } from "./model";
 const EMPTY_STATUSES = new Map<string, string>();
 const STEP_LABELS: Record<SyncReviewStep, string> = {
   history: "Git & sessions",
-  skills: "Skills",
-  plugins: "Plugins",
   project_files: "Project files",
   review: "Review",
 };
 const STORAGE_BLOCKING_STATES = new Set(["storage_only", "storage_ahead", "diverged"]);
-const PUSH_SELECTABLE_SYNC_STATES = new Set(["local_only", "local_ahead"]);

 export interface PushReviewBlocker {
   kind: "storage" | "local";
@@ -52,7 +47,6 @@ interface Props {
   storageName?: string | null;
   refreshEpoch?: number;
   threadComparison?: ThreadSyncComparison | null;
-  capabilityReport?: CapabilityStatusReport | null;
   projectContentInventory?: ProjectContentInventory | null;
   projectContentScanned?: boolean;
   projectContentLoading?: boolean;
@@ -85,8 +79,6 @@ function countSelected(items: ProjectResourceDescriptor[], selected: ReadonlySet

 function groupForStep(step: SyncReviewStep, resources: ProjectResourceDescriptor[]): ProjectResourceDescriptor[] {
   if (step === "history") return resources.filter((resource) => categoryFor(resource) === "conversations");
-  if (step === "skills") return resources.filter((resource) => categoryFor(resource) === "skills");
-  if (step === "plugins") return resources.filter((resource) => categoryFor(resource) === "plugins");
   if (step === "project_files") return resources.filter((resource) => categoryFor(resource) === "project_files");
   return resources;
 }
@@ -129,7 +121,6 @@ export function pushReviewPullHolds(
 export function pushReviewBlockers(
   selected: ReadonlySet<string>,
   threadComparison?: ThreadSyncComparison | null,
-  capabilityReport?: CapabilityStatusReport | null,
   projectContentInventory?: ProjectContentInventory | null,
   acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
 ): PushReviewBlocker[] {
@@ -145,17 +136,6 @@ export function pushReviewBlockers(
     const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
     if (kind) blockers.push({ kind, step: "history", resourceId: entry.resource_id, state: entry.state });
   }
-  for (const item of capabilityReport?.items ?? []) {
-    const kind = pushBlockerKind(item.state, item.storage_present, selected.has(item.resource_id));
-    if (kind) {
-      blockers.push({
-        kind,
-        step: item.kind === "plugin" ? "plugins" : "skills",
-        resourceId: item.resource_id,
-        state: item.state,
-      });
-    }
-  }
   for (const entry of projectContentInventory?.entries ?? []) {
     const selectedEntry = selected.has(entry.descriptor.resource_id);
     const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
@@ -183,14 +163,12 @@ export function pushReviewBlockers(
 export function pushReviewBlockingCount(
   selected: ReadonlySet<string>,
   threadComparison?: ThreadSyncComparison | null,
-  capabilityReport?: CapabilityStatusReport | null,
   projectContentInventory?: ProjectContentInventory | null,
   acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
 ): number {
   return pushReviewBlockers(
     selected,
     threadComparison,
-    capabilityReport,
     projectContentInventory,
     acknowledgedWarningDigests,
   ).length;
@@ -199,7 +177,6 @@ export function pushReviewBlockingCount(
 export function pushSelectableResourceIds(
   resources: ProjectResourceDescriptor[],
   threadComparison?: ThreadSyncComparison | null,
-  capabilityReport?: CapabilityStatusReport | null,
   projectContentInventory?: ProjectContentInventory | null,
 ): Set<string> {
   const unselectableIds = new Set<string>();
@@ -208,11 +185,6 @@ export function pushSelectableResourceIds(
       unselectableIds.add(entry.resource_id);
     }
   }
-  for (const item of capabilityReport?.items ?? []) {
-    if (!PUSH_SELECTABLE_SYNC_STATES.has(item.state) || item.blocked_reason) {
-      unselectableIds.add(item.resource_id);
-    }
-  }
   for (const entry of projectContentInventory?.entries ?? []) {
     if (entry.blocked_reason || STORAGE_BLOCKING_STATES.has(entry.state)) {
       unselectableIds.add(entry.descriptor.resource_id);
@@ -231,13 +203,11 @@ export function sanitizePushSelection(
   selected: ReadonlySet<string>,
   resources: ProjectResourceDescriptor[],
   threadComparison?: ThreadSyncComparison | null,
-  capabilityReport?: CapabilityStatusReport | null,
   projectContentInventory?: ProjectContentInventory | null,
 ): Set<string> {
   const selectableIds = pushSelectableResourceIds(
     resources,
     threadComparison,
-    capabilityReport,
     projectContentInventory,
   );
   return new Set([...selected].filter((resourceId) => selectableIds.has(resourceId)));
@@ -247,13 +217,11 @@ export function recommendedPushSelection(
   selected: ReadonlySet<string>,
   resources: ProjectResourceDescriptor[],
   threadComparison?: ThreadSyncComparison | null,
-  capabilityReport?: CapabilityStatusReport | null,
   projectContentInventory?: ProjectContentInventory | null,
 ): Set<string> {
   const selectableIds = pushSelectableResourceIds(
     resources,
     threadComparison,
-    capabilityReport,
     projectContentInventory,
   );
   const recommended = new Set([...selected].filter((resourceId) => selectableIds.has(resourceId)));
@@ -350,7 +318,6 @@ export default function PushResourceWorkspace({
   storageName = "storage",
   refreshEpoch = 0,
   threadComparison,
-  capabilityReport,
   projectContentInventory = null,
   projectContentScanned = false,
   projectContentLoading = false,
@@ -398,8 +365,6 @@ export default function PushResourceWorkspace({
   );
   const sessionIndex = conversations.find((resource) => resource.resource_id === "codex:session-index") ?? null;
   const visibleConversations = conversations.filter((resource) => resource.resource_id !== sessionIndex?.resource_id);
-  const skills = useMemo(() => resources.filter((resource) => categoryFor(resource) === "skills"), [resources]);
-  const plugins = useMemo(() => resources.filter((resource) => categoryFor(resource) === "plugins"), [resources]);
   const projectFiles = useMemo(
     () => resources.filter((resource) => categoryFor(resource) === "project_files"),
     [resources],
@@ -409,8 +374,8 @@ export default function PushResourceWorkspace({
     [resources],
   );
   const selectableIds = useMemo(
-    () => pushSelectableResourceIds(resources, threadComparison, capabilityReport, projectContentInventory),
-    [capabilityReport, projectContentInventory, resources, threadComparison],
+    () => pushSelectableResourceIds(resources, threadComparison, projectContentInventory),
+    [projectContentInventory, resources, threadComparison],
   );
   const effectiveSelected = useMemo(
     () => new Set([...selected].filter((resourceId) => selectableIds.has(resourceId))),
@@ -423,15 +388,12 @@ export default function PushResourceWorkspace({

   const counts = {
     history: countSelected(visibleConversations, effectiveSelected),
-    skills: countSelected(skills, effectiveSelected),
-    plugins: countSelected(plugins, effectiveSelected),
     project_files: countSelected(projectFiles, effectiveSelected),
     review: effectiveSelected.size,
   };
   const blockers = pushReviewBlockers(
     effectiveSelected,
     threadComparison,
-    capabilityReport,
     projectContentInventory,
     acknowledgedWarningDigests,
   );
@@ -534,12 +496,6 @@ export default function PushResourceWorkspace({
   const selectableConversationIds = new Set(visibleConversations
     .filter((resource) => selectableIds.has(resource.resource_id))
     .map((resource) => resource.resource_id));
-  const selectableSkillIds = new Set(skills
-    .filter((resource) => selectableIds.has(resource.resource_id))
-    .map((resource) => resource.resource_id));
-  const selectablePluginIds = new Set(plugins
-    .filter((resource) => selectableIds.has(resource.resource_id))
-    .map((resource) => resource.resource_id));
   const requiredProjectDirectories = requiredProjectContentDirectoryIds(
     projectContentInventory,
     effectiveSelected,
@@ -648,28 +604,6 @@ export default function PushResourceWorkspace({
             <ResourceInventory resources={conversations} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableConversationIds} disabled={busy} onToggle={toggleResource} />
           ) : null}

-          {(activeStep === "skills" || activeStep === "plugins") && project ? (
-            <SkillsPluginStatusPage
-              view={activeStep}
-              project={project}
-              binding={binding}
-              refreshEpoch={refreshEpoch}
-              activeStorageId={storageId}
-              activeStorageName={storageName}
-              onOpenProjectSettings={() => undefined}
-              selectionMode="push"
-              selectedResourceIds={effectiveSelected}
-              selectableResourceIds={activeStep === "skills" ? selectableSkillIds : selectablePluginIds}
-              selectionDisabled={busy}
-              onToggleResource={toggleResource}
-              reportOverride={capabilityReport}
-            />
-          ) : activeStep === "skills" ? (
-            <ResourceInventory resources={skills} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableSkillIds} disabled={busy} onToggle={toggleResource} />
-          ) : activeStep === "plugins" ? (
-            <ResourceInventory resources={plugins} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectablePluginIds} disabled={busy} onToggle={toggleResource} />
-          ) : null}
-
           {activeStep === "project_files" && (
             <ProjectFilesReviewPage
               mode="push"
@@ -733,14 +667,6 @@ export default function PushResourceWorkspace({
                 <span><Icon name="git-branch" size={15} /><strong>Sessions</strong></span>
                 <span>{counts.history} included<Icon name="chevron-right" size={13} /></span>
               </button>
-              <button type="button" onClick={() => selectStep("skills")}>
-                <span><Icon name="folder" size={15} /><strong>Skills</strong></span>
-                <span>{counts.skills} included<Icon name="chevron-right" size={13} /></span>
-              </button>
-              <button type="button" onClick={() => selectStep("plugins")}>
-                <span><Icon name="link" size={15} /><strong>Plugins</strong></span>
-                <span>{counts.plugins} included<Icon name="chevron-right" size={13} /></span>
-              </button>
               {other.length > 0 && (
                 <details className="sync-sync-review-other">
                   <summary>
diff --git a/src/components/project-sync/ResourceInventory.tsx b/src/components/project-sync/ResourceInventory.tsx
index e612c88..5a917be 100644
--- a/src/components/project-sync/ResourceInventory.tsx
+++ b/src/components/project-sync/ResourceInventory.tsx
@@ -12,10 +12,8 @@ interface Props {
   onToggle: (resourceId: string) => void;
 }

-const CATEGORY_ICON: Record<ProjectResourceCategory, "activity" | "settings" | "folder" | "link"> = {
+const CATEGORY_ICON: Record<ProjectResourceCategory, "activity" | "settings" | "folder"> = {
   conversations: "activity",
-  skills: "folder",
-  plugins: "link",
   project_files: "folder",
   tools: "settings",
 };
@@ -25,20 +23,7 @@ function statusLabel(status?: string): string {
   return status.replace(/_/g, " ");
 }

-function providedSkills(resource: ProjectResourceDescriptor): string[] {
-  const raw = resource.metadata?.plugin_provided_skills_json;
-  if (!raw) return [];
-  try {
-    const parsed = JSON.parse(raw);
-    return Array.isArray(parsed) ? parsed.filter((name) => typeof name === "string") : [];
-  } catch {
-    return [];
-  }
-}
-
-function resourceIcon(resource: ProjectResourceDescriptor): "activity" | "file" | "folder" | "link" | "settings" {
-  if (resource.kind === "standalone_skill") return "folder";
-  if (resource.kind === "plugin") return "link";
+function resourceIcon(resource: ProjectResourceDescriptor): "activity" | "file" | "settings" {
   if (resource.kind === "requirement") return "settings";
   if (resource.kind.includes("conversation")) return "activity";
   return "file";
@@ -61,11 +46,6 @@ function ResourceRow({
 }) {
   const [expanded, setExpanded] = useState(false);
   const blocked = !!resource.blocked_reason;
-  const needsInstall = resource.kind.includes("plugin") || resource.kind.includes("skill");
-  const installDirectory = resource.metadata?.install_dir_name;
-  const hasDistinctInstallDirectory = resource.kind === "standalone_skill"
-    && !!installDirectory
-    && installDirectory.toLocaleLowerCase() !== resource.display_name.toLocaleLowerCase();

   return (
     <div className={`sync-resource-row${selectable ? " sync-resource-row-selectable" : ""}${checked ? " selected" : ""}${blocked ? " blocked" : ""}${expanded ? " expanded" : ""}`}>
@@ -95,22 +75,10 @@ function ResourceRow({
         <span className="sync-resource-copy">
           <strong>{resource.display_name}</strong>
           <span>
-            {providerLabel(resource.provider)} · {resource.kind === "standalone_skill"
-              ? "global custom skill"
-              : resource.metadata?.plugin_origin
-                ? `global plugin (${resource.metadata.plugin_origin})`
-                : resource.scope.replace(/_/g, " ")}
-            {resource.provided_by ? ` · provided by ${resource.provided_by}` : ""}
-            {hasDistinctInstallDirectory ? ` · folder ${installDirectory}` : ""}
-            {resource.metadata?.plugin_observed_version
-              ? ` · v${resource.metadata.plugin_observed_version} observed`
-              : ""}
+            {providerLabel(resource.provider)} · {resource.scope.replace(/_/g, " ")}
           </span>
         </span>
         <span className="sync-resource-meta">
-          {needsInstall && checked && (
-            <span className="sync-resource-install">{resource.install_behavior ?? "install on restore"}</span>
-          )}
           {status && <span className={`sync-resource-status status-${status}`}>{statusLabel(status)}</span>}
         </span>
       </button>
@@ -126,18 +94,11 @@ function ResourceRow({
           <div className="sync-resource-detail-grid">
             <span>Resource ID</span><code>{resource.resource_id}</code>
             <span>Kind</span><code>{resource.kind}</code>
-            {hasDistinctInstallDirectory && <><span>Install folder</span><code>{installDirectory}</code></>}
             {resource.apply_policy && <><span>Apply</span><code>{resource.apply_policy}</code></>}
           </div>
           {resource.blocked_reason && (
             <div className="sync-resource-blocker"><Icon name="alert-triangle" size={14} /> {resource.blocked_reason}</div>
           )}
-          {providedSkills(resource).length > 0 && (
-            <div>
-              <span>Skills provided by this plugin (not separately selectable)</span>
-              <code>{providedSkills(resource).join("\n")}</code>
-            </div>
-          )}
         </div>
       )}
     </div>
diff --git a/src/components/project-sync/RestorePlanView.tsx b/src/components/project-sync/RestorePlanView.tsx
index 8e8a2cf..72ab1c4 100644
--- a/src/components/project-sync/RestorePlanView.tsx
+++ b/src/components/project-sync/RestorePlanView.tsx
@@ -3,8 +3,6 @@ import type {
   BundleReadiness,
   ConversationPlanAction,
   ConversationResolutionSelection,
-  DependencyPlan,
-  DependencyResult,
   LocalProjectSummary,
   ProjectBinding,
   RestorePlan,
@@ -13,7 +11,6 @@ import type {
 import Icon from "../Icons";
 import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
 import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
-import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
 import SyncReviewTabs, {
   type SyncReviewStep,
   syncReviewSteps,
@@ -24,7 +21,6 @@ import {
   buildPullReviewItems,
   buildPullReviewSelection,
   defaultSelected,
-  dependencyActionIds,
   itemKindLabel,
   includeRequiredPullProjectDirectories,
   pendingIds,
@@ -43,10 +39,8 @@ interface Props {
   profileLabel: string;
   plan: RestorePlan;
   binding: ProjectBinding;
-  dependencyPlan: DependencyPlan | null;
   readiness: BundleReadiness | null;
   restoreResult: RestoreResult | null;
-  dependencyResult: DependencyResult | null;
   phase: PullApplyPhase;
   supportLoading: boolean;
   completedActionIds: ReadonlySet<string>;
@@ -69,17 +63,14 @@ interface Props {

 const STEP_LABELS: Record<SyncReviewStep, string> = {
   history: "Git & sessions",
-  skills: "Skills",
-  plugins: "Plugins",
   project_files: "Project files",
   review: "Review",
 };

 function phaseIndex(phase: PullApplyPhase): number {
   if (phase === "restoring") return 0;
-  if (phase === "installing") return 1;
-  if (phase === "verifying") return 2;
-  if (phase === "complete") return 3;
+  if (phase === "verifying") return 1;
+  if (phase === "complete") return 2;
   return -1;
 }

@@ -87,17 +78,14 @@ function PhaseProgress({
   phase,
   ready,
   restoreFailed,
-  installFailed,
 }: {
   phase: PullApplyPhase;
   ready: boolean;
   restoreFailed: boolean;
-  installFailed: boolean;
 }) {
   const activeIndex = phaseIndex(phase);
   const steps = [
-    { label: "Restore project data and custom skills", icon: "file" as const },
-    { label: "Install selected plugins and tools", icon: "download" as const },
+    { label: "Restore project data", icon: "file" as const },
     { label: "Verify project readiness", icon: "refresh" as const },
   ];
   return (
@@ -106,8 +94,7 @@ function PhaseProgress({
         const current = activeIndex === index;
         const warning = phase === "complete" && (
           (index === 0 && restoreFailed) ||
-          (index === 1 && installFailed) ||
-          (index === 2 && !ready)
+          (index === 1 && !ready)
         );
         const complete = !warning && (activeIndex > index || phase === "complete");
         return (
@@ -138,12 +125,10 @@ function nameStatusLabel(status: string | null | undefined): string {

 function ResultDetails({
   restoreResult,
-  dependencyResult,
 }: {
   restoreResult: RestoreResult | null;
-  dependencyResult: DependencyResult | null;
 }) {
-  const results = [restoreResult, dependencyResult].filter((result): result is RestoreResult | DependencyResult => result !== null);
+  const results = [restoreResult].filter((result): result is RestoreResult => result !== null);
   if (results.length === 0) return null;
   return (
     <details className="sync-pull-result-details">
@@ -318,7 +303,6 @@ function ItemList({
   completedResourceIds,
   failedResourceIds,
   phase,
-  supportLoading,
   disabled,
   onToggle,
 }: {
@@ -328,7 +312,6 @@ function ItemList({
   completedResourceIds: ReadonlySet<string>;
   failedResourceIds: ReadonlySet<string>;
   phase: PullApplyPhase;
-  supportLoading: boolean;
   disabled: boolean;
   onToggle: (resourceId: string) => void;
 }) {
@@ -346,20 +329,13 @@ function ItemList({
         const pending = pendingIds(item, completedActionIds);
         const completed = completedResourceIds.has(item.resourceId);
         const failed = failedResourceIds.has(item.resourceId);
-        const waitingForInstaller =
-          item.category === "global_tool" &&
-          item.toolKind !== "custom_skill" &&
-          pending.length === 0 &&
-          !completed &&
-          supportLoading;
         const keepLocalConversation = keepsLocalConversation(item);
         const manualResolution = requiresManualResolution(item);
-        const blocked = keepLocalConversation || manualResolution || pending.length === 0 || completed || waitingForInstaller;
+        const blocked = keepLocalConversation || manualResolution || pending.length === 0 || completed;
         const selectable = !blocked;
         const checked = selected.has(item.resourceId) && !completed;
         const expanded = expandedItems.has(item.resourceId);
         const activeRestore = phase === "restoring" && checked && restoreActionIds(item).some((id) => !completedActionIds.has(id));
-        const activeInstall = phase === "installing" && checked && dependencyActionIds(item).some((id) => !completedActionIds.has(id));
         const conversationAction = item.conversationActions[0]?.action;
         const status: string | null = conversationAction === "no_op"
           ? "Already up to date"
@@ -373,16 +349,6 @@ function ItemList({
           ? "Keep local"
           : manualResolution
           ? "Resolve manually"
-          : item.category === "global_tool"
-          ? completed
-            ? "Installed"
-            : failed
-              ? "Failed"
-              : activeRestore || activeInstall
-                ? "Installing"
-                : checked || waitingForInstaller
-                  ? null
-                  : "Needs approval"
           : completed
             ? "Restored"
             : failed
@@ -423,7 +389,7 @@ function ItemList({
               </span>
               <span className="sync-pull-item-icon">
                 <Icon
-                  name={failed || manualResolution || keepLocalConversation ? "alert-triangle" : completed ? "check-circle" : item.toolKind === "plugin" ? "download" : item.toolKind === "custom_skill" ? "folder" : "file"}
+                  name={failed || manualResolution || keepLocalConversation ? "alert-triangle" : completed ? "check-circle" : "file"}
                   size={14}
                 />
               </span>
@@ -445,9 +411,6 @@ function ItemList({
                   {item.conversationActions.map((action) => (
                     <Fragment key={action.action_id}><span>Conversation</span><code>{action.action}</code></Fragment>
                   ))}
-                  {item.dependencyActions.map((action) => (
-                    <Fragment key={action.action_id}><span>Installer</span><code>{action.action_id}</code></Fragment>
-                  ))}
                 </div>
               </div>
             )}
@@ -464,10 +427,8 @@ export default function RestorePlanView({
   storageName = "storage",
   plan,
   binding,
-  dependencyPlan,
   readiness,
   restoreResult,
-  dependencyResult,
   phase,
   supportLoading,
   completedActionIds,
@@ -484,7 +445,7 @@ export default function RestorePlanView({
   onRefresh,
   onBack,
 }: Props) {
-  const items = useMemo(() => buildPullReviewItems(plan, dependencyPlan), [plan, dependencyPlan]);
+  const items = useMemo(() => buildPullReviewItems(plan), [plan]);
   const divergenceActions = useMemo(
     () => (plan.conversation_actions ?? []).filter((action) => action.action === "resolve_divergence"),
     [plan],
@@ -505,20 +466,15 @@ export default function RestorePlanView({
     () => items.filter((item) => itemKindLabel(item) === "Conversation"),
     [items],
   );
-  const skillItems = useMemo(() => items.filter((item) => item.toolKind === "custom_skill"), [items]);
-  const pluginItems = useMemo(() => items.filter((item) => item.toolKind === "plugin"), [items]);
   const projectContentItems = useMemo(
     () => items.filter((item) => item.category === "project_content"),
     [items],
   );
   const otherItems = useMemo(() => items.filter((item) => (
     itemKindLabel(item) !== "Conversation"
-    && item.toolKind !== "custom_skill"
-    && item.toolKind !== "plugin"
     && item.category !== "project_content"
   )), [items]);
   const projectItems = useMemo(() => items.filter((item) => item.category === "project_data"), [items]);
-  const globalItems = useMemo(() => items.filter((item) => item.category === "global_tool"), [items]);
   const showProjectFiles = plan.project_content_eligibility?.state === "eligible" || projectContentItems.length > 0;
   const reviewSteps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
   const knownItems = useRef(new Set<string>());
@@ -587,7 +543,7 @@ export default function RestorePlanView({
   const pendingProjectContentDecisionCount = projectContentItems.filter((item) => (
     pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
   )).length;
-  const hasResult = phase === "complete" || restoreResult !== null || dependencyResult !== null;
+  const hasResult = phase === "complete" || restoreResult !== null;
   const approvalItems = items.filter((item) => (
     item.category !== "project_content"
     && requiresApproval(item)
@@ -609,24 +565,14 @@ export default function RestorePlanView({
     if (!reviewSteps.includes(activeStep)) setActiveStep("review");
   }, [activeStep, reviewSteps]);

-  const failureCount = (restoreResult?.failed_actions?.length ?? 0) + (dependencyResult?.failed_actions?.length ?? 0);
-  const skippedTools = globalItems.filter((item) => (
-    !selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId)
-  )).length;
-  const unchangedCount = skippedTools + manualItems.length;
-  const finalReady = hasResult && !error && failureCount === 0 && skippedTools === 0 && readiness?.state === "ready";
+  const failureCount = restoreResult?.failed_actions?.length ?? 0;
+  const unchangedCount = manualItems.length;
+  const finalReady = hasResult && !error && failureCount === 0 && readiness?.state === "ready";
   const reviewCanClose = !forkCheckpointActive && hasResult && (
     finalReady || (selectedCount === 0 && pendingProjectContentDecisionCount === 0)
   );
   const completedProjectItems = projectItems.filter((item) => completedResourceIds.has(item.resourceId));
-  const completedSkills = globalItems.filter((item) => (
-    item.toolKind === "custom_skill" && completedResourceIds.has(item.resourceId)
-  ));
-  const completedPlugins = globalItems.filter((item) => (
-    item.toolKind === "plugin" && completedResourceIds.has(item.resourceId)
-  ));
   const failedProjectItems = projectItems.filter((item) => failedResourceIds.has(item.resourceId));
-  const failedTools = globalItems.filter((item) => failedResourceIds.has(item.resourceId));
   const outcomeTitle = forkCheckpointActive
     ? forkCheckpoint?.status === "awaiting_manual_fork"
       ? "Waiting for a Codex fork"
@@ -641,15 +587,11 @@ export default function RestorePlanView({
           ? "Pull complete"
           : "Project restored; setup needs attention";
   const buttonLabel = busy
-    ? phase === "installing"
-      ? "Installing selected tools…"
-      : phase === "verifying"
-        ? "Verifying readiness…"
-        : "Restoring selected changes…"
+    ? phase === "verifying"
+      ? "Verifying readiness…"
+      : "Restoring selected changes…"
     : forkCheckpointActive
       ? "Complete the fork checkpoint above"
-    : hasResult && failedTools.length > 0 && selectedCount === failedResourceIds.size
-      ? `Retry failed installation${failedTools.length === 1 ? "" : "s"}`
       : reviewCanClose
         ? "Done"
         : hasResult && selectedCount > 0
@@ -676,17 +618,12 @@ export default function RestorePlanView({
     else next.add(resourceId);
     return includeRequiredPullProjectDirectories(items, next);
   });
-  const dependencyMessages = (dependencyPlan?.blockers?.length ?? 0) + (dependencyPlan?.warnings?.length ?? 0);
   const recommendedIds = new Set(items.filter(defaultSelected).map((item) => item.resourceId));
   const stepItems = activeStep === "history"
     ? conversationItems
-    : activeStep === "skills"
-      ? skillItems
-      : activeStep === "plugins"
-        ? pluginItems
-        : activeStep === "project_files"
-          ? projectContentItems
-        : items;
+    : activeStep === "project_files"
+      ? projectContentItems
+      : items;
   const pendingStepItems = stepItems.filter((item) => (
     pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
   ));
@@ -712,16 +649,12 @@ export default function RestorePlanView({
   const selectedIn = (chosen: PullReviewItem[]) => chosen.filter((item) => selected.has(item.resourceId)).length;
   const counts = {
     history: selectedIn(conversationItems),
-    skills: selectedIn(skillItems),
-    plugins: selectedIn(pluginItems),
     project_files: selectedIn(projectContentItems),
     review: selectedCount,
   };
   const warningSteps = new Set<SyncReviewStep>();
   for (const item of approvalItems) {
     if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
-    else if (item.toolKind === "custom_skill") warningSteps.add("skills");
-    else if (item.toolKind === "plugin") warningSteps.add("plugins");
     warningSteps.add("review");
   }
   for (const item of manualItems) {
@@ -732,7 +665,7 @@ export default function RestorePlanView({
     warningSteps.add("history");
     warningSteps.add("review");
   }
-  if (dependencyMessages > 0 || failureCount > 0) warningSteps.add("review");
+  if (failureCount > 0) warningSteps.add("review");
   if (projectContentItems.length > 0 && !projectContentEligible) {
     warningSteps.add("project_files");
     warningSteps.add("review");
@@ -742,8 +675,6 @@ export default function RestorePlanView({
     completedActionIds,
     completedResourceIds,
   );
-  const skillIds = pullSelectableResourceIds(skillItems, completedActionIds, completedResourceIds);
-  const pluginIds = pullSelectableResourceIds(pluginItems, completedActionIds, completedResourceIds);
   const stepIndex = reviewSteps.indexOf(activeStep);
   const goBack = () => setActiveStep(reviewSteps[Math.max(0, stepIndex - 1)]);
   const goNext = () => setActiveStep(reviewSteps[Math.min(reviewSteps.length - 1, stepIndex + 1)]);
@@ -877,7 +808,6 @@ export default function RestorePlanView({
               completedResourceIds={completedResourceIds}
               failedResourceIds={failedResourceIds}
               phase={phase}
-              supportLoading={supportLoading}
               disabled={busy || finalReady}
               onToggle={toggle}
             />
@@ -885,48 +815,6 @@ export default function RestorePlanView({
             <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No session changes</strong><small>Sessions already match this storage.</small></span></div>
           ) : null}

-          {activeStep === "skills" && skillItems.length > 0 && project ? (
-            <SkillsPluginStatusPage
-              view="skills"
-              project={project}
-              binding={binding}
-              refreshEpoch={0}
-              activeStorageId={plan.storage_id}
-              activeStorageName={storageName}
-              onOpenProjectSettings={() => undefined}
-              selectionMode="pull"
-              selectedResourceIds={selected}
-              selectableResourceIds={skillIds}
-              selectionDisabled={busy || finalReady}
-              onToggleResource={toggle}
-            />
-          ) : activeStep === "skills" && skillItems.length > 0 ? (
-            <ItemList items={skillItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
-          ) : activeStep === "skills" ? (
-            <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No skill changes</strong><small>Skills already match this storage.</small></span></div>
-          ) : null}
-
-          {activeStep === "plugins" && pluginItems.length > 0 && project ? (
-            <SkillsPluginStatusPage
-              view="plugins"
-              project={project}
-              binding={binding}
-              refreshEpoch={0}
-              activeStorageId={plan.storage_id}
-              activeStorageName={storageName}
-              onOpenProjectSettings={() => undefined}
-              selectionMode="pull"
-              selectedResourceIds={selected}
-              selectableResourceIds={pluginIds}
-              selectionDisabled={busy || finalReady}
-              onToggleResource={toggle}
-            />
-          ) : activeStep === "plugins" && pluginItems.length > 0 ? (
-            <ItemList items={pluginItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
-          ) : activeStep === "plugins" ? (
-            <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No plugin changes</strong><small>Plugins already match this storage.</small></span></div>
-          ) : null}
-
           {activeStep === "project_files" && (
             <ProjectFilesReviewPage
               mode="pull"
@@ -963,7 +851,7 @@ export default function RestorePlanView({
           {activeStep === "review" && (
             <div className="sync-sync-review-summary">
               {phase !== "idle" && (
-                <PhaseProgress phase={phase} ready={finalReady} restoreFailed={failedProjectItems.length > 0} installFailed={failedTools.length > 0} />
+                <PhaseProgress phase={phase} ready={finalReady} restoreFailed={failedProjectItems.length > 0} />
               )}
               {forkCheckpointActive && forkCheckpoint && (
                 <section className="sync-fork-checkpoint" aria-live="polite">
@@ -1030,9 +918,6 @@ export default function RestorePlanView({
                     <h2>{outcomeTitle}</h2>
                     <ul>
                       {completedProjectItems.length > 0 && <li>{completedProjectItems.length} project data change{completedProjectItems.length === 1 ? "" : "s"} restored</li>}
-                      {completedSkills.length > 0 && <li>{completedSkills.length} custom skill{completedSkills.length === 1 ? "" : "s"} restored</li>}
-                      {completedPlugins.length > 0 && <li>{completedPlugins.map((item) => item.title).join(", ")} installed</li>}
-                      {skippedTools > 0 && <li>{globalItems.filter((item) => !selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId)).map((item) => item.title).join(", ")} left unchanged</li>}
                       {keptLocalConversationItems.length > 0 && <li>{keptLocalConversationItems.length} conversation{keptLocalConversationItems.length === 1 ? "" : "s"} kept local</li>}
                       {manualItems.length > 0 && <li>{manualItems.length} conflict{manualItems.length === 1 ? "" : "s"} left unchanged for manual resolution</li>}
                       {finalReady && <li>Project is ready</li>}
@@ -1083,27 +968,17 @@ export default function RestorePlanView({
                 <span><Icon name="git-branch" size={15} /><strong>Sessions</strong></span>
                 <span>{counts.history} selected<Icon name="chevron-right" size={13} /></span>
               </button>
-              <button type="button" onClick={() => setActiveStep("skills")}>
-                <span><Icon name="folder" size={15} /><strong>Skills</strong></span>
-                <span>{counts.skills} selected<Icon name="chevron-right" size={13} /></span>
-              </button>
-              <button type="button" onClick={() => setActiveStep("plugins")}>
-                <span><Icon name="link" size={15} /><strong>Plugins</strong></span>
-                <span>{counts.plugins} selected<Icon name="chevron-right" size={13} /></span>
-              </button>
               {(otherItems.length > 0 || supportLoading) && (
                 <details className="sync-sync-review-other">
                   <summary>
                     <span><Icon name="settings" size={15} /><strong>Agent setup & tools</strong></span>
                     <span>{selectedIn(otherItems)} selected<Icon name="chevron-right" size={13} /></span>
                   </summary>
-                  <ItemList items={otherItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
-                  {supportLoading && <div className="sync-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking installers…</div>}
+                  <ItemList items={otherItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} disabled={busy || finalReady} onToggle={toggle} />
+                  {supportLoading && <div className="sync-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking readiness…</div>}
                 </details>
               )}

-              {(dependencyPlan?.blockers ?? []).map((blocker) => <div key={blocker} className="sync-callout error"><Icon name="alert-triangle" size={15} /> {blocker}</div>)}
-              {(dependencyPlan?.warnings ?? []).map((warning) => <div key={warning} className="sync-callout"><Icon name="alert-triangle" size={15} /> {warning}</div>)}
               {!hasResult && keptLocalConversationItems.length > 0 && (
                 <div className="sync-callout warning sync-sync-review-blocker">
                   <Icon name="alert-triangle" size={15} />
@@ -1135,7 +1010,7 @@ export default function RestorePlanView({
                       ? `${approvalItems.map((item) => item.title).join(", ")} ${approvalItems.length === 1 ? "was" : "were"} left unchanged.`
                       : `Apply will skip ${approvalItems.map((item) => item.title).join(", ")} and leave ${approvalItems.length === 1 ? "it" : "them"} unchanged on this machine.`}
                   </span>
-                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep(approvalItems[0]?.toolKind === "plugin" ? "plugins" : approvalItems[0]?.toolKind === "custom_skill" ? "skills" : "review")}>Review optional</button>
+                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep("review")}>Review optional</button>
                 </div>
               )}

@@ -1166,7 +1041,7 @@ export default function RestorePlanView({
               )}

               {error && <div className="sync-callout error sync-pull-error" role="alert"><Icon name="alert-triangle" size={15} /> {error}</div>}
-              <ResultDetails restoreResult={restoreResult} dependencyResult={dependencyResult} />
+              <ResultDetails restoreResult={restoreResult} />
             </div>
           )}
         </div>
diff --git a/src/components/project-sync/SyncReviewTabs.tsx b/src/components/project-sync/SyncReviewTabs.tsx
index c890c45..2aece2b 100644
--- a/src/components/project-sync/SyncReviewTabs.tsx
+++ b/src/components/project-sync/SyncReviewTabs.tsx
@@ -1,14 +1,14 @@
 import type { KeyboardEvent as ReactKeyboardEvent } from "react";
 import Icon from "../Icons";

-export type SyncReviewStep = "history" | "skills" | "plugins" | "project_files" | "review";
+export type SyncReviewStep = "history" | "project_files" | "review";

 export type SyncReviewTabCounts = Record<SyncReviewStep, number>;

 export function syncReviewSteps(includeProjectFiles: boolean): SyncReviewStep[] {
   return includeProjectFiles
-    ? ["history", "skills", "plugins", "project_files", "review"]
-    : ["history", "skills", "plugins", "review"];
+    ? ["history", "project_files", "review"]
+    : ["history", "review"];
 }

 interface Props {
@@ -23,11 +23,9 @@ interface Props {
 const STEP_DEFINITIONS: Record<SyncReviewStep, {
   id: SyncReviewStep;
   label: string;
-  icon: "git-branch" | "folder" | "link" | "file" | "check-circle";
+  icon: "git-branch" | "file" | "check-circle";
 }> = {
   history: { id: "history", label: "Git & sessions", icon: "git-branch" },
-  skills: { id: "skills", label: "Skills", icon: "folder" },
-  plugins: { id: "plugins", label: "Plugins", icon: "link" },
   project_files: { id: "project_files", label: "Project files", icon: "file" },
   review: { id: "review", label: "Review", icon: "check-circle" },
 };
diff --git a/src/components/project-sync/api.ts b/src/components/project-sync/api.ts
index 139f7dc..dcb1a5f 100644
--- a/src/components/project-sync/api.ts
+++ b/src/components/project-sync/api.ts
@@ -10,7 +10,6 @@ import type {
   BundleReadiness,
   BundleRecipe,
   BundleSnapshotSummary,
-  CapabilityStatusReport,
   CodexAppRestartResult,
   CodexConversationPathAudit,
   CodexConversationPathRepairResult,
@@ -18,8 +17,6 @@ import type {
   ConversationResolutionSelection,
   CodexThreadDetailsPage,
   CreateSetupDraftResult,
-  DependencyPlan,
-  DependencyResult,
   HostIdentityStatus,
   LocalProjectRegistration,
   OriginHostAlias,
@@ -187,12 +184,6 @@ export const projectSyncApi = {
   getPushDestinationStatus: (localProjectId: string, storageId: string) =>
     invoke<PushDestinationStatus>("get_push_destination_status", { localProjectId, storageId }),

-  getCapabilityStatus: (localProjectId: string, storageId?: string | null) =>
-    invoke<CapabilityStatusReport>("get_project_capability_status", {
-      localProjectId,
-      storageId: storageId ?? null,
-    }),
-
   getThreadSyncComparison: (localProjectId: string, storageId: string) =>
     invoke<ThreadSyncComparison>("get_project_thread_sync_comparison", { localProjectId, storageId }),

@@ -274,12 +265,6 @@ export const projectSyncApi = {
     },
   }),

-  planDependencies: (restorePlanId: string) =>
-    invoke<DependencyPlan>("plan_dependencies", { restorePlanId }),
-
-  applyDependencies: (planId: string, actionIds: string[]) =>
-    invoke<DependencyResult>("apply_dependency_actions", { planId, actionIds }),
-
   getReadiness: (storageId: string, bundleId: string, binding: ProjectBinding) =>
     invoke<BundleReadiness>("get_bundle_readiness", { storageId, bundleId, binding }),

diff --git a/src/components/project-sync/model.ts b/src/components/project-sync/model.ts
index 23bc4cb..767f2a2 100644
--- a/src/components/project-sync/model.ts
+++ b/src/components/project-sync/model.ts
@@ -28,8 +28,6 @@ export const RESOURCE_GROUPS: Array<{
   description: string;
 }> = [
   { id: "conversations", label: "Conversations", description: "Project-scoped tasks and sessions" },
-  { id: "skills", label: "Skills", description: "Selected standalone skills from the provider home" },
-  { id: "plugins", label: "Plugins", description: "Portable install intent; payloads and caches never sync" },
   { id: "project_files", label: "Project files", description: "Exact files and folders selected from a non-Git project" },
   { id: "tools", label: "Requirements", description: "Environment requirements this backup expects on each machine" },
 ];
@@ -46,16 +44,12 @@ const CATEGORY_BY_KIND: Record<string, ProjectResourceCategory> = {
   project_memory: "conversations",
   project_content_file: "project_files",
   project_content_directory: "project_files",
-  skill: "skills",
-  standalone_skill: "skills",
-  plugin_skill: "skills",
-  plugin: "plugins",
   requirement: "tools",
 };

 export function categoryFor(resource: ProjectResourceDescriptor): ProjectResourceCategory {
   const explicit = resource.category;
-  if (explicit === "conversations" || explicit === "skills" || explicit === "plugins" || explicit === "project_files" || explicit === "tools") {
+  if (explicit === "conversations" || explicit === "project_files" || explicit === "tools") {
     return explicit;
   }
   return CATEGORY_BY_KIND[resource.kind.toLowerCase()] ?? "tools";
diff --git a/src/components/project-sync/pullReviewFlow.ts b/src/components/project-sync/pullReviewFlow.ts
index 913b83d..fc54d9f 100644
--- a/src/components/project-sync/pullReviewFlow.ts
+++ b/src/components/project-sync/pullReviewFlow.ts
@@ -1,8 +1,6 @@
 import type {
   BundleReadiness,
   ConversationResolutionSelection,
-  DependencyPlan,
-  DependencyResult,
   ProjectBinding,
   RestorePlan,
   RestoreResult,
@@ -11,7 +9,6 @@ import { errorMessage } from "./model";

 export interface PullReviewApi {
   planRestore: (storageId: string, bundleId: string, binding: ProjectBinding) => Promise<RestorePlan>;
-  planDependencies: (restorePlanId: string) => Promise<DependencyPlan>;
   getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
 }

@@ -22,7 +19,6 @@ export interface PullReviewRequest {
 }

 export interface PullReviewSupport {
-  dependencyPlan: DependencyPlan | null;
   readiness: BundleReadiness | null;
   errors: string[];
 }
@@ -32,12 +28,11 @@ export interface StartedPullReview {
   support: Promise<PullReviewSupport>;
 }

-export type PullApplyPhase = "idle" | "restoring" | "installing" | "verifying" | "complete";
+export type PullApplyPhase = "idle" | "restoring" | "verifying" | "complete";

 export interface PullReviewSelection {
   resourceIds: string[];
   restoreActionIds: string[];
-  dependencyActionIds: string[];
   conversationResolutions: ConversationResolutionSelection[];
 }

@@ -47,17 +42,15 @@ export interface PullReviewApplyApi {
     actionIds: string[],
     conversationResolutions: ConversationResolutionSelection[],
   ) => Promise<RestoreResult>;
-  applyDependencies: (planId: string, actionIds: string[]) => Promise<DependencyResult>;
   getRestoreReadiness: (restorePlanId: string) => Promise<BundleReadiness>;
 }

 export interface PullReviewApplyResult {
   restoreResult: RestoreResult | null;
-  dependencyResult: DependencyResult | null;
   readiness: BundleReadiness | null;
   success: boolean;
   error: string | null;
-  failedPhase: "restoring" | "installing" | "verifying" | null;
+  failedPhase: "restoring" | "verifying" | null;
 }

 function hasKeepLocalConversationDecision(plan: RestorePlan): boolean {
@@ -87,13 +80,12 @@ export function hasPendingRestoreMutations(plan: RestorePlan): boolean {
 }

 /**
- * The two backend plans are immutable approval surfaces. Refuse to combine
- * them unless every pin identifies the same remote snapshot and local
- * binding; otherwise a single Apply click could approve two different states.
+ * The restore plan is an immutable approval surface. Refuse a selection whose
+ * ids no longer belong to it; otherwise a single Apply click could approve a
+ * state the user never reviewed.
  */
 export function assertPullPlansAligned(
   restorePlan: RestorePlan,
-  dependencyPlan: DependencyPlan | null,
   selection: PullReviewSelection,
 ): void {
   const restoreIds = new Set([
@@ -123,41 +115,17 @@ export function assertPullPlansAligned(
   if (resolutionIds.size !== selectedDivergences.size) {
     throw new Error("Choose one resolution for every selected conversation conflict.");
   }
-  if (!dependencyPlan && selection.dependencyActionIds.length > 0) {
-    throw new Error("The selected tools do not have an installation plan yet.");
-  }
-  if (dependencyPlan) {
-    const sameSnapshot =
-      dependencyPlan.storage_id === restorePlan.storage_id &&
-      dependencyPlan.bundle_id === restorePlan.bundle_id &&
-      dependencyPlan.host_id === restorePlan.host_id &&
-      dependencyPlan.replica_id === restorePlan.replica_id &&
-      dependencyPlan.expected_host_identity_revision === restorePlan.expected_host_identity_revision &&
-      dependencyPlan.revision === restorePlan.revision &&
-      dependencyPlan.commit_id === restorePlan.commit_id &&
-      dependencyPlan.manifest_sha256 === restorePlan.manifest_sha256 &&
-      dependencyPlan.binding_revision === restorePlan.binding_revision;
-    if (!sameSnapshot) {
-      throw new Error("The project and tool plans describe different bundle revisions. Refresh the Pull review.");
-    }
-    const dependencyIds = new Set(dependencyPlan.actions.map((action) => action.action_id));
-    if (selection.dependencyActionIds.some((actionId) => !dependencyIds.has(actionId))) {
-      throw new Error("The selected tool installations no longer belong to this Pull review.");
-    }
-  }
 }

 /** Execute the one-button Pull workflow in the order presented by the UI. */
 export async function applyPullReview(
   api: PullReviewApplyApi,
   restorePlan: RestorePlan,
-  dependencyPlan: DependencyPlan | null,
   selection: PullReviewSelection,
   onPhase: (phase: PullApplyPhase) => void,
 ): Promise<PullReviewApplyResult> {
-  assertPullPlansAligned(restorePlan, dependencyPlan, selection);
+  assertPullPlansAligned(restorePlan, selection);
   let restoreResult: RestoreResult | null = null;
-  let dependencyResult: DependencyResult | null = null;
   let readiness: BundleReadiness | null = null;
   let executionError: string | null = null;
   let failedPhase: PullReviewApplyResult["failedPhase"] = null;
@@ -183,19 +151,6 @@ export async function applyPullReview(
     }
   }

-  onPhase("installing");
-  if (!executionError && selection.dependencyActionIds.length > 0 && dependencyPlan) {
-    try {
-      dependencyResult = await api.applyDependencies(
-        dependencyPlan.plan_id,
-        selection.dependencyActionIds,
-      );
-    } catch (reason) {
-      executionError = `Tool installation failed: ${errorMessage(reason)}`;
-      failedPhase = "installing";
-    }
-  }
-
   onPhase("verifying");
   try {
     readiness = await api.getRestoreReadiness(restorePlan.plan_id);
@@ -208,12 +163,10 @@ export async function applyPullReview(
     executionError === null &&
     (restoreResult?.success ?? true) &&
     (restoreResult ? restoreResult.status === "complete" : true) &&
-    (dependencyResult?.success ?? true) &&
     readiness?.state === "ready";
   onPhase("complete");
   return {
     restoreResult,
-    dependencyResult,
     readiness,
     success,
     error: executionError,
@@ -238,9 +191,9 @@ export async function beginPullReview(

 /**
  * Load only the mutation review. This is deliberately separate from optional
- * supporting checks so a dependency/readiness error cannot hide a valid plan
- * or its Apply button. Planning fetches the bundle itself — no separate
- * prefetch, the payload must only be downloaded once.
+ * supporting checks so a readiness error cannot hide a valid plan or its
+ * Apply button. Planning fetches the bundle itself — no separate prefetch,
+ * the payload must only be downloaded once.
  */
 export async function preparePullReview(
   api: PullReviewApi,
@@ -254,20 +207,9 @@ export async function loadPullReviewSupport(
   api: PullReviewApi,
   restorePlan: RestorePlan,
 ): Promise<PullReviewSupport> {
-  const [dependencies, readiness] = await Promise.allSettled([
-    api.planDependencies(restorePlan.plan_id),
-    api.getRestoreReadiness(restorePlan.plan_id),
-  ]);
-  const errors: string[] = [];
-  if (dependencies.status === "rejected") {
-    errors.push(`Dependency checks: ${errorMessage(dependencies.reason)}`);
-  }
-  if (readiness.status === "rejected") {
-    errors.push(`Readiness checks: ${errorMessage(readiness.reason)}`);
+  try {
+    return { readiness: await api.getRestoreReadiness(restorePlan.plan_id), errors: [] };
+  } catch (reason) {
+    return { readiness: null, errors: [`Readiness checks: ${errorMessage(reason)}`] };
   }
-  return {
-    dependencyPlan: dependencies.status === "fulfilled" ? dependencies.value : null,
-    readiness: readiness.status === "fulfilled" ? readiness.value : null,
-    errors,
-  };
 }
diff --git a/src/components/project-sync/pullReviewModel.ts b/src/components/project-sync/pullReviewModel.ts
index 10ee2a2..dd261f2 100644
--- a/src/components/project-sync/pullReviewModel.ts
+++ b/src/components/project-sync/pullReviewModel.ts
@@ -1,16 +1,13 @@
 import type {
   ConversationPlanAction,
   ConversationResolutionSelection,
-  DependencyAction,
-  DependencyPlan,
   ProjectProvider,
   RestoreAction,
   RestorePlan,
 } from "../../types";
 import type { PullReviewSelection } from "./pullReviewFlow";

-type PullReviewCategory = "project_data" | "project_content" | "global_tool";
-type GlobalToolKind = "plugin" | "custom_skill" | "setup_item";
+type PullReviewCategory = "project_data" | "project_content";

 export interface PullReviewItem {
   resourceId: string;
@@ -18,10 +15,8 @@ export interface PullReviewItem {
   title: string;
   detail: string;
   provider: ProjectProvider | null;
-  toolKind: GlobalToolKind | null;
   restoreActions: RestoreAction[];
   conversationActions: ConversationPlanAction[];
-  dependencyActions: DependencyAction[];
   projectContentPath: string | null;
   projectContentEntryType: "file" | "directory" | null;
   projectContentOperation: "add" | "replace" | "create_directory" | "delete_file" | "delete_directory" | null;
@@ -46,31 +41,18 @@ function restoreActionCopy(action: RestoreAction): {
   detail: string;
   provider: ProjectProvider | null;
   category: PullReviewCategory;
-  toolKind: GlobalToolKind | null;
   projectContentPath?: string;
   projectContentEntryType?: "file" | "directory";
   projectContentOperation?: PullReviewItem["projectContentOperation"];
 } {
   const kind = action.kind.kind;
   const provider = "provider" in action.kind ? action.kind.provider : null;
-  if (kind === "install_custom_skill" || kind === "overwrite_custom_skill") {
-    return {
-      title: action.kind.skill_name,
-      detail: kind === "overwrite_custom_skill"
-        ? "Replaces the existing custom skill after creating a local backup."
-        : "Restores this custom skill into the assigned provider profile.",
-      provider,
-      category: "global_tool",
-      toolKind: "custom_skill",
-    };
-  }
   if (kind === "materialize_conversation") {
     return {
       title: "Conversation",
       detail: action.kind.logical_path,
       provider,
       category: "project_data",
-      toolKind: null,
     };
   }
   if (kind === "write_project_file") {
@@ -80,7 +62,6 @@ function restoreActionCopy(action: RestoreAction): {
       detail: projectPath,
       provider: null,
       category: "project_content",
-      toolKind: null,
       projectContentPath: projectPath,
       projectContentEntryType: "file",
       projectContentOperation: action.expected_target_sha256 ? "replace" : "add",
@@ -93,7 +74,6 @@ function restoreActionCopy(action: RestoreAction): {
       detail: projectPath,
       provider: null,
       category: "project_content",
-      toolKind: null,
       projectContentPath: projectPath,
       projectContentEntryType: "directory",
       projectContentOperation: "create_directory",
@@ -107,7 +87,6 @@ function restoreActionCopy(action: RestoreAction): {
       detail: projectPath,
       provider: null,
       category: "project_content",
-      toolKind: null,
       projectContentPath: projectPath,
       projectContentEntryType: isDirectory ? "directory" : "file",
       projectContentOperation: isDirectory ? "delete_directory" : "delete_file",
@@ -119,7 +98,6 @@ function restoreActionCopy(action: RestoreAction): {
       detail: action.kind.logical_path,
       provider,
       category: "project_data",
-      toolKind: null,
     };
   }
   if (kind === "manual") {
@@ -133,7 +111,6 @@ function restoreActionCopy(action: RestoreAction): {
       detail: action.kind.message,
       provider: conversationProvider,
       category: "project_data",
-      toolKind: null,
     };
   }
   return {
@@ -141,23 +118,11 @@ function restoreActionCopy(action: RestoreAction): {
     detail: action.target_path ?? kind,
     provider,
     category: "project_data",
-    toolKind: null,
   };
 }

-function dependencyToolKind(action: DependencyAction): GlobalToolKind {
-  if (action.kind === "install_codex_plugin" || action.kind === "install_claude_plugin") {
-    return "plugin";
-  }
-  if (action.kind === "install_standalone_skill") return "custom_skill";
-  return "setup_item";
-}
-
-/** Merge the two backend plans into one logical row per resource ID. */
-export function buildPullReviewItems(
-  plan: RestorePlan,
-  dependencyPlan: DependencyPlan | null,
-): PullReviewItem[] {
+/** Merge the plan's file and conversation actions into one logical row per resource ID. */
+export function buildPullReviewItems(plan: RestorePlan): PullReviewItem[] {
   const items = new Map<string, PullReviewItem>();
   const ensure = (
     resourceId: string,
@@ -165,12 +130,9 @@ export function buildPullReviewItems(
     title: string,
     detail: string,
     provider: ProjectProvider | null,
-    toolKind: GlobalToolKind | null,
   ) => {
     const existing = items.get(resourceId);
     if (existing) {
-      if (category === "global_tool") existing.category = "global_tool";
-      if (toolKind) existing.toolKind = toolKind;
       if (provider) existing.provider = provider;
       return existing;
     }
@@ -180,10 +142,8 @@ export function buildPullReviewItems(
       title,
       detail,
       provider,
-      toolKind,
       restoreActions: [],
       conversationActions: [],
-      dependencyActions: [],
       projectContentPath: null,
       projectContentEntryType: null,
       projectContentOperation: null,
@@ -200,16 +160,11 @@ export function buildPullReviewItems(
       copy.title,
       copy.detail,
       copy.provider,
-      copy.toolKind,
     );
     item.restoreActions.push(action);
     if (copy.projectContentPath) item.projectContentPath = copy.projectContentPath;
     if (copy.projectContentEntryType) item.projectContentEntryType = copy.projectContentEntryType;
     if (copy.projectContentOperation) item.projectContentOperation = copy.projectContentOperation;
-    if (copy.category === "global_tool") {
-      item.title = copy.title;
-      item.detail = copy.detail;
-    }
   }

   for (const action of plan.conversation_actions ?? []) {
@@ -236,37 +191,15 @@ export function buildPullReviewItems(
       "Conversation",
       detail,
       action.provider,
-      null,
     );
     item.conversationActions.push(action);
     item.detail = detail;
   }

-  for (const action of dependencyPlan?.actions ?? []) {
-    const toolKind = dependencyToolKind(action);
-    const item = ensure(
-      action.resource_id,
-      "global_tool",
-      action.display_name,
-      toolKind === "plugin"
-        ? "Installed through the provider's native plugin manager."
-        : toolKind === "custom_skill"
-          ? "Installed into the assigned provider profile."
-          : "Checked on this machine during setup.",
-      action.provider ?? null,
-      toolKind,
-    );
-    item.category = "global_tool";
-    item.title = action.display_name;
-    item.toolKind = toolKind;
-    item.dependencyActions.push(action);
-  }
-
   return [...items.values()].sort((left, right) => {
     const categoryRank: Record<PullReviewCategory, number> = {
       project_data: 0,
       project_content: 1,
-      global_tool: 2,
     };
     if (left.category !== right.category) {
       return categoryRank[left.category] - categoryRank[right.category];
@@ -313,21 +246,14 @@ export function restoreActionIds(item: PullReviewItem): string[] {
   return [...fileActions, ...conversationActions];
 }

-export function dependencyActionIds(item: PullReviewItem): string[] {
-  return item.dependencyActions.map((action) => action.action_id);
-}
-
-function actionableIds(item: PullReviewItem): string[] {
-  return [...restoreActionIds(item), ...dependencyActionIds(item)];
-}
-
 export function pendingIds(item: PullReviewItem, completedActionIds: ReadonlySet<string>): string[] {
-  return actionableIds(item).filter((actionId) => !completedActionIds.has(actionId));
+  return restoreActionIds(item).filter((actionId) => !completedActionIds.has(actionId));
 }

 export function requiresApproval(item: PullReviewItem): boolean {
-  const restore = item.restoreActions.filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind));
-  return [...restore, ...item.dependencyActions].some((action) => action.requires_explicit_approval)
+  return item.restoreActions
+    .filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind))
+    .some((action) => action.requires_explicit_approval)
     || item.conversationActions.some((action) => conversationActionRequiresApproval(action));
 }

@@ -351,10 +277,7 @@ export function requiresManualResolution(item: PullReviewItem): boolean {

 export function defaultSelected(item: PullReviewItem): boolean {
   if (item.category === "project_content") return false;
-  const actions = [
-    ...item.restoreActions.filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind)),
-    ...item.dependencyActions,
-  ];
+  const actions = item.restoreActions.filter((action) => !NON_DIRECT_RESTORE_KINDS.has(action.kind.kind));
   const selectedConversationActions = item.conversationActions.filter(conversationActionSelectable);
   return (actions.length > 0 || selectedConversationActions.length > 0)
     && actions.every((action) => !action.requires_explicit_approval)
@@ -362,9 +285,6 @@ export function defaultSelected(item: PullReviewItem): boolean {
 }

 export function itemKindLabel(item: PullReviewItem): string {
-  if (item.toolKind === "plugin") return "Plugin";
-  if (item.toolKind === "custom_skill") return "Custom skill";
-  if (item.toolKind === "setup_item") return "Setup item";
   if (
     item.conversationActions.length > 0
     || /^(codex|claude):session:/.test(item.resourceId)
@@ -407,7 +327,6 @@ export function buildPullReviewSelection(
       .filter((item) => pendingIds(item, completedActionIds).length > 0)
       .map((item) => item.resourceId),
     restoreActionIds: chosen.flatMap((item) => restoreActionIds(item).filter((id) => !completedActionIds.has(id))),
-    dependencyActionIds: chosen.flatMap((item) => dependencyActionIds(item).filter((id) => !completedActionIds.has(id))),
     conversationResolutions: chosenConversationActions
       .filter((action) => action.action === "resolve_divergence")
       .map((action) => conversationResolutions.get(action.action_id) ?? {
diff --git a/src/types.ts b/src/types.ts
index a803718..69753cc 100644
--- a/src/types.ts
+++ b/src/types.ts
@@ -79,8 +79,6 @@ export type ProjectProvider = "codex" | "claude";

 export type ProjectResourceCategory =
   | "conversations"
-  | "skills"
-  | "plugins"
   | "project_files"
   | "tools";

@@ -102,12 +100,10 @@ export type ProjectResourceKind =
   | "project_content_file"
   | "project_content_directory"
   | "project_memory"
-  | "standalone_skill"
-  | "plugin"
   | "requirement";

-export type ProjectResourceScope = "project" | "provider_state" | "dependency" | "requirement";
-export type ProjectApplyPolicy = "safe_file" | "merge" | "explicit_install" | "explicit_review" | "manual_only" | "never";
+export type ProjectResourceScope = "project" | "provider_state" | "requirement";
+export type ProjectApplyPolicy = "safe_file" | "merge" | "explicit_review" | "manual_only" | "never";

 export interface ProjectResourceDescriptor {
   resource_id: string;
@@ -127,8 +123,6 @@ export interface ProjectResourceDescriptor {
   default_selected?: boolean;
   selected_by_default?: boolean;
   blocked_reason?: string | null;
-  provided_by?: string | null;
-  install_behavior?: string | null;
 }

 export interface RecipeEntry {
@@ -530,7 +524,7 @@ export interface ResourceStatusReport {
   warnings?: string[];
 }

-export type CapabilityStatusState =
+export type ResourceSyncState =
   | "synced"
   | "local_only"
   | "local_ahead"
@@ -541,44 +535,6 @@ export type CapabilityStatusState =
   | "unknown"
   | "not_compared";

-export interface CapabilityProfileContext {
-  provider: ProjectProvider;
-  profile_id: string;
-  display_name: string;
-  path: string;
-  shared_project_count: number;
-}
-
-export interface CapabilityStatusItem extends ProjectResourceDescriptor {
-  category: "skills" | "plugins" | string;
-  state: CapabilityStatusState | string;
-  local_present: boolean;
-  storage_present: boolean;
-  selected_in_recipe: boolean;
-  blocked_reason?: string | null;
-  logical_paths: string[];
-  local_digest?: string | null;
-  storage_digest?: string | null;
-  local_version?: string | null;
-  storage_version?: string | null;
-  enabled?: boolean | null;
-  provided_skills: string[];
-  message?: string | null;
-}
-
-export interface CapabilityStatusReport {
-  project_id: string;
-  project_name: string;
-  profiles: CapabilityProfileContext[];
-  storage_id?: string | null;
-  storage_name?: string | null;
-  revision?: number | null;
-  base_revision?: number | null;
-  compared_at: number;
-  items: CapabilityStatusItem[];
-  warnings: string[];
-}
-
 export type ProjectFileSyncEligibilityState = "eligible" | "git_managed" | "unknown";

 export interface ProjectFileSyncEligibility {
@@ -597,7 +553,7 @@ export interface ProjectContentEntry {
   size?: number | null;
   mode?: number | null;
   source_mtime?: number | null;
-  state: CapabilityStatusState | string;
+  state: ResourceSyncState | string;
   local_present: boolean;
   storage_present: boolean;
   base_present: boolean;
@@ -697,8 +653,6 @@ export interface PushReviewContext {
   destination_status: PushDestinationStatus;
   thread_comparison: ThreadSyncComparison | null;
   thread_comparison_error: string | null;
-  capability_report: CapabilityStatusReport | null;
-  capability_report_error: string | null;
   project_files: ProjectContentInventory | null;
   project_files_error: string | null;
   conversation_push_plan_id?: string | null;
@@ -725,8 +679,6 @@ export type RestoreActionKind =
   | { kind: "delete_project_directory"; logical_path: string }
   | { kind: "merge_file"; logical_path: string }
   | { kind: "materialize_conversation"; provider: ProjectProvider; logical_path: string }
-  | { kind: "install_custom_skill"; provider: ProjectProvider; skill_name: string }
-  | { kind: "overwrite_custom_skill"; provider: ProjectProvider; skill_name: string }
   | { kind: "manual"; message: string };

 export interface RestoreAction {
@@ -900,42 +852,6 @@ export interface CodexAppRestartResult {
   message: string;
 }

-export interface DependencyPlan {
-  schema_version: 1;
-  plan_id: string;
-  storage_id: string;
-  bundle_id: string;
-  host_id: string;
-  replica_id: string;
-  expected_host_identity_revision: number;
-  revision: number;
-  commit_id: string;
-  manifest_sha256: string;
-  binding_revision: number;
-  created_at: number;
-  expires_at: number;
-  actions: DependencyAction[];
-  blockers?: string[];
-  warnings?: string[];
-}
-
-export interface DependencyAction {
-  action_id: string;
-  resource_id: string;
-  kind: "install_codex_plugin" | "install_claude_plugin" | "install_standalone_skill" | "check_binary" | "check_environment" | "manual";
-  display_name: string;
-  provider?: ProjectProvider | null;
-  argv: string[];
-  requires_explicit_approval: boolean;
-}
-
-export interface DependencyResult {
-  success: boolean;
-  message: string;
-  applied_action_ids?: string[];
-  failed_actions?: Array<{ action_id: string; display_name?: string | null; message: string }>;
-}
-
 export interface BundleReadinessIssue {
   issue_id: string;
   category: ProjectResourceCategory | string;
```
