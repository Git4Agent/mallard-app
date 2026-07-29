import { useEffect, useMemo, useState } from "react";
import type {
  BundleFetchProgress,
  BundleRecipe,
  CapabilityStatusReport,
  LocalProjectSummary,
  ProjectBinding,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  PushDestinationStatus,
  ThreadSyncComparison,
} from "../../types";
import Icon from "../Icons";
import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import ResourceInventory from "./ResourceInventory";
import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
import SyncReviewTabs, {
  type SyncReviewStep,
  syncReviewSteps,
} from "./SyncReviewTabs";
import { categoryFor } from "./model";

const EMPTY_STATUSES = new Map<string, string>();
const STEP_LABELS: Record<SyncReviewStep, string> = {
  history: "Git & sessions",
  skills: "Skills",
  plugins: "Plugins",
  project_files: "Project files",
  review: "Review",
};
const STORAGE_BLOCKING_STATES = new Set(["storage_only", "storage_ahead", "diverged"]);
const PUSH_SELECTABLE_SYNC_STATES = new Set(["local_only", "local_ahead"]);

export interface PushReviewBlocker {
  kind: "storage" | "local";
  step: Exclude<SyncReviewStep, "review">;
  resourceId: string;
  state: string;
}

interface Props {
  resources: ProjectResourceDescriptor[];
  selected: Set<string>;
  projectDefaults: Set<string>;
  busy: boolean;
  error: string | null;
  project?: LocalProjectSummary;
  binding?: ProjectBinding | null;
  storageId?: string | null;
  storageName?: string | null;
  refreshEpoch?: number;
  threadComparison?: ThreadSyncComparison | null;
  capabilityReport?: CapabilityStatusReport | null;
  projectContentInventory?: ProjectContentInventory | null;
  projectContentScanned?: boolean;
  projectContentLoading?: boolean;
  projectContentRemovals?: ReadonlySet<string>;
  acknowledgedWarningDigests?: ReadonlySet<string>;
  destinationStatus?: PushDestinationStatus | null;
  /** Live upload progress while the push publishes; renders in the footer button. */
  fetchProgress?: BundleFetchProgress | null;
  showProjectFiles?: boolean;
  initialStep?: SyncReviewStep;
  activeStep?: SyncReviewStep;
  onStepChange?: (step: SyncReviewStep) => void;
  onToggle: (resourceId: string) => void;
  onSelectionChange?: (selected: Set<string>) => void;
  onScanProjectFiles?: () => void;
  onToggleProjectContentRemoval?: (resourceId: string) => void;
  onToggleProjectContentWarning?: (warningDigest: string) => void;
  onUseProjectDefaults: () => void;
  onClear: () => void;
  onClose: () => void;
  onPush: () => void;
  onPull?: () => void;
  onOpenStorageSettings?: () => void;
  onUnlinkStorage?: () => void;
}

function countSelected(items: ProjectResourceDescriptor[], selected: ReadonlySet<string>): number {
  return items.filter((item) => selected.has(item.resource_id)).length;
}

function groupForStep(step: SyncReviewStep, resources: ProjectResourceDescriptor[]): ProjectResourceDescriptor[] {
  if (step === "history") return resources.filter((resource) => categoryFor(resource) === "conversations");
  if (step === "skills") return resources.filter((resource) => categoryFor(resource) === "skills");
  if (step === "plugins") return resources.filter((resource) => categoryFor(resource) === "plugins");
  if (step === "project_files") return resources.filter((resource) => categoryFor(resource) === "project_files");
  return resources;
}

export function nextPushReviewStep(
  step: SyncReviewStep,
  steps: readonly SyncReviewStep[] = syncReviewSteps(false),
): SyncReviewStep {
  const stepIndex = steps.indexOf(step);
  return steps[Math.min(steps.length - 1, Math.max(0, stepIndex) + 1)];
}

function pushBlockerKind(
  state: string,
  storagePresent: boolean,
  selected: boolean,
): PushReviewBlocker["kind"] | null {
  if (STORAGE_BLOCKING_STATES.has(state)) return "storage";
  if (state === "unknown") {
    return storagePresent ? "storage" : selected ? "local" : null;
  }
  if (state === "unavailable") return selected ? "local" : null;
  if (state === "blocked") return selected ? "local" : null;
  return null;
}

export function pushReviewBlockers(
  selected: ReadonlySet<string>,
  threadComparison?: ThreadSyncComparison | null,
  capabilityReport?: CapabilityStatusReport | null,
  projectContentInventory?: ProjectContentInventory | null,
  acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
): PushReviewBlocker[] {
  const blockers: PushReviewBlocker[] = [];
  for (const entry of threadComparison?.entries ?? []) {
    if (entry.cloud_thread_id) {
      continue;
    }
    const kind = pushBlockerKind(entry.state, entry.storage_present, selected.has(entry.resource_id));
    if (kind) blockers.push({ kind, step: "history", resourceId: entry.resource_id, state: entry.state });
  }
  for (const item of capabilityReport?.items ?? []) {
    const kind = pushBlockerKind(item.state, item.storage_present, selected.has(item.resource_id));
    if (kind) {
      blockers.push({
        kind,
        step: item.kind === "plugin" ? "plugins" : "skills",
        resourceId: item.resource_id,
        state: item.state,
      });
    }
  }
  for (const entry of projectContentInventory?.entries ?? []) {
    const selectedEntry = selected.has(entry.descriptor.resource_id);
    const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
    if (kind) {
      blockers.push({
        kind,
        step: "project_files",
        resourceId: entry.descriptor.resource_id,
        state: entry.state,
      });
      continue;
    }
    if (selectedEntry && entry.warning_digest && !acknowledgedWarningDigests.has(entry.warning_digest)) {
      blockers.push({
        kind: "local",
        step: "project_files",
        resourceId: entry.descriptor.resource_id,
        state: "warning_unacknowledged",
      });
    }
  }
  return blockers;
}

export function pushReviewBlockingCount(
  selected: ReadonlySet<string>,
  threadComparison?: ThreadSyncComparison | null,
  capabilityReport?: CapabilityStatusReport | null,
  projectContentInventory?: ProjectContentInventory | null,
  acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
): number {
  return pushReviewBlockers(
    selected,
    threadComparison,
    capabilityReport,
    projectContentInventory,
    acknowledgedWarningDigests,
  ).length;
}

export function pushSelectableResourceIds(
  resources: ProjectResourceDescriptor[],
  threadComparison?: ThreadSyncComparison | null,
  capabilityReport?: CapabilityStatusReport | null,
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const unselectableIds = new Set<string>();
  for (const entry of threadComparison?.entries ?? []) {
    if (!entry.push_decision.selectable) {
      unselectableIds.add(entry.resource_id);
    }
  }
  for (const item of capabilityReport?.items ?? []) {
    if (!PUSH_SELECTABLE_SYNC_STATES.has(item.state) || item.blocked_reason) {
      unselectableIds.add(item.resource_id);
    }
  }
  for (const entry of projectContentInventory?.entries ?? []) {
    if (entry.blocked_reason || STORAGE_BLOCKING_STATES.has(entry.state)) {
      unselectableIds.add(entry.descriptor.resource_id);
    }
  }
  return new Set(resources
    .filter((resource) => (
      !resource.blocked_reason
      && resource.apply_policy !== "never"
      && !unselectableIds.has(resource.resource_id)
    ))
    .map((resource) => resource.resource_id));
}

export function sanitizePushSelection(
  selected: ReadonlySet<string>,
  resources: ProjectResourceDescriptor[],
  threadComparison?: ThreadSyncComparison | null,
  capabilityReport?: CapabilityStatusReport | null,
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const selectableIds = pushSelectableResourceIds(
    resources,
    threadComparison,
    capabilityReport,
    projectContentInventory,
  );
  return new Set([...selected].filter((resourceId) => selectableIds.has(resourceId)));
}

export function recommendedPushSelection(
  selected: ReadonlySet<string>,
  resources: ProjectResourceDescriptor[],
  threadComparison?: ThreadSyncComparison | null,
  capabilityReport?: CapabilityStatusReport | null,
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const selectableIds = pushSelectableResourceIds(
    resources,
    threadComparison,
    capabilityReport,
    projectContentInventory,
  );
  const recommended = new Set([...selected].filter((resourceId) => selectableIds.has(resourceId)));
  if (!threadComparison) return recommended;

  const comparisonByResourceId = new Map(threadComparison.entries.map((entry) => [entry.resource_id, entry]));
  const conversations = resources.filter((resource) => categoryFor(resource) === "conversations");
  const sessionIndex = conversations.find((resource) => resource.resource_id === "codex:session-index") ?? null;

  for (const resource of conversations) {
    if (resource.resource_id === sessionIndex?.resource_id) continue;
    const comparison = comparisonByResourceId.get(resource.resource_id);
    if (!comparison) continue;
    if (
      comparison.push_decision.selectable
      && comparison.push_decision.selected_by_default
      && selectableIds.has(resource.resource_id)
    ) {
      recommended.add(resource.resource_id);
    } else {
      recommended.delete(resource.resource_id);
    }
  }

  if (sessionIndex) {
    const hasSelectedSession = conversations.some((resource) => (
      resource.resource_id !== sessionIndex.resource_id && recommended.has(resource.resource_id)
    ));
    if (hasSelectedSession && selectableIds.has(sessionIndex.resource_id)) {
      recommended.add(sessionIndex.resource_id);
    } else {
      recommended.delete(sessionIndex.resource_id);
    }
  }

  return recommended;
}

export function preserveSyncedPushRecipeEntries(
  recipe: BundleRecipe,
  sourceRecipe: BundleRecipe,
  threadComparison?: ThreadSyncComparison | null,
): BundleRecipe {
  const syncedResourceIds = new Set((threadComparison?.entries ?? [])
    .filter((entry) => entry.state === "synced")
    .map((entry) => entry.resource_id));
  if (syncedResourceIds.size === 0) return recipe;

  const entries = { ...recipe.entries };
  for (const resourceId of [...syncedResourceIds, "codex:session-index"]) {
    const sourceEntry = sourceRecipe.entries[resourceId];
    if (sourceEntry) entries[resourceId] = sourceEntry;
  }
  return { ...recipe, entries };
}

export function requiredProjectContentDirectoryIds(
  inventory: ProjectContentInventory | null | undefined,
  selected: ReadonlySet<string>,
): Set<string> {
  const directoriesByPath = new Map((inventory?.entries ?? [])
    .filter((entry) => entry.entry_type === "directory")
    .map((entry) => [entry.relative_path, entry.descriptor.resource_id]));
  const required = new Set<string>();
  for (const entry of inventory?.entries ?? []) {
    if (!selected.has(entry.descriptor.resource_id) || entry.entry_type !== "file") continue;
    const parts = entry.relative_path.split("/");
    for (let end = 1; end < parts.length; end += 1) {
      const resourceId = directoriesByPath.get(parts.slice(0, end).join("/"));
      if (resourceId) required.add(resourceId);
    }
  }
  return required;
}

export function includeRequiredProjectContentDirectories(
  inventory: ProjectContentInventory | null | undefined,
  selected: ReadonlySet<string>,
): Set<string> {
  const next = new Set(selected);
  requiredProjectContentDirectoryIds(inventory, next).forEach((resourceId) => next.add(resourceId));
  return next;
}

export default function PushResourceWorkspace({
  resources,
  selected,
  projectDefaults,
  busy,
  error,
  project,
  binding = null,
  storageId = null,
  storageName = "storage",
  refreshEpoch = 0,
  threadComparison,
  capabilityReport,
  projectContentInventory = null,
  projectContentScanned = false,
  projectContentLoading = false,
  projectContentRemovals = new Set(),
  acknowledgedWarningDigests = new Set(),
  destinationStatus,
  fetchProgress = null,
  showProjectFiles = false,
  initialStep = "history",
  activeStep: controlledActiveStep,
  onStepChange,
  onToggle,
  onSelectionChange,
  onScanProjectFiles,
  onToggleProjectContentRemoval,
  onToggleProjectContentWarning,
  onUseProjectDefaults,
  onClear,
  onClose,
  onPush,
  onPull,
  onOpenStorageSettings,
  onUnlinkStorage,
}: Props) {
  const steps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const [uncontrolledActiveStep, setUncontrolledActiveStep] = useState<SyncReviewStep>(initialStep);
  const requestedActiveStep = controlledActiveStep ?? uncontrolledActiveStep;
  const activeStep = steps.includes(requestedActiveStep) ? requestedActiveStep : "review";
  const selectStep = (step: SyncReviewStep) => {
    if (controlledActiveStep === undefined) setUncontrolledActiveStep(step);
    onStepChange?.(step);
  };

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, onClose]);

  const conversations = useMemo(
    () => resources.filter((resource) => categoryFor(resource) === "conversations"),
    [resources],
  );
  const sessionIndex = conversations.find((resource) => resource.resource_id === "codex:session-index") ?? null;
  const visibleConversations = conversations.filter((resource) => resource.resource_id !== sessionIndex?.resource_id);
  const skills = useMemo(() => resources.filter((resource) => categoryFor(resource) === "skills"), [resources]);
  const plugins = useMemo(() => resources.filter((resource) => categoryFor(resource) === "plugins"), [resources]);
  const projectFiles = useMemo(
    () => resources.filter((resource) => categoryFor(resource) === "project_files"),
    [resources],
  );
  const other = useMemo(
    () => resources.filter((resource) => categoryFor(resource) === "tools"),
    [resources],
  );
  const selectableIds = useMemo(
    () => pushSelectableResourceIds(resources, threadComparison, capabilityReport, projectContentInventory),
    [capabilityReport, projectContentInventory, resources, threadComparison],
  );
  const effectiveSelected = useMemo(
    () => new Set([...selected].filter((resourceId) => selectableIds.has(resourceId))),
    [selectableIds, selected],
  );
  const safeProjectDefaults = useMemo(
    () => new Set([...projectDefaults].filter((resourceId) => selectableIds.has(resourceId))),
    [projectDefaults, selectableIds],
  );

  const counts = {
    history: countSelected(visibleConversations, effectiveSelected),
    skills: countSelected(skills, effectiveSelected),
    plugins: countSelected(plugins, effectiveSelected),
    project_files: countSelected(projectFiles, effectiveSelected),
    review: effectiveSelected.size,
  };
  const blockers = pushReviewBlockers(
    effectiveSelected,
    threadComparison,
    capabilityReport,
    projectContentInventory,
    acknowledgedWarningDigests,
  );
  const blockingCount = blockers.length;
  const missingDestination = destinationStatus?.state === "remote_history_missing"
    ? destinationStatus
    : null;
  const remoteHistoryMissing = missingDestination !== null;
  const storageBlockingCount = blockers.filter((blocker) => blocker.kind === "storage").length;
  const localBlockingCount = blockingCount - storageBlockingCount;
  const warningSteps = new Set<SyncReviewStep>();
  blockers.forEach((blocker) => warningSteps.add(blocker.step));
  if (blockingCount > 0) warningSteps.add("review");
  const blockerTitle = storageBlockingCount > 0
    ? localBlockingCount > 0 ? "Review before pushing" : "Pull before pushing"
    : "Resolve selected resources";
  const blockerDetail = [
    storageBlockingCount > 0
      ? `${storageBlockingCount} storage change${storageBlockingCount === 1 ? "" : "s"} need${storageBlockingCount === 1 ? "s" : ""} review.`
      : null,
    localBlockingCount > 0
      ? `${localBlockingCount} selected local resource${localBlockingCount === 1 ? "" : "s"} cannot be captured. Exclude ${localBlockingCount === 1 ? "it" : "them"} or fix the local issue.`
      : null,
  ].filter((message): message is string => Boolean(message)).join(" ");
  const blockerFooter = [
    storageBlockingCount > 0
      ? `${storageBlockingCount} storage change${storageBlockingCount === 1 ? "" : "s"} need${storageBlockingCount === 1 ? "s" : ""} review`
      : null,
    localBlockingCount > 0
      ? `${localBlockingCount} selected resource${localBlockingCount === 1 ? "" : "s"} need${localBlockingCount === 1 ? "s" : ""} attention`
      : null,
  ].filter((message): message is string => Boolean(message)).join(" · ");

  const commitSelection = (next: Set<string>) => {
    const safeNext = includeRequiredProjectContentDirectories(
      projectContentInventory,
      new Set([...next].filter((resourceId) => selectableIds.has(resourceId))),
    );
    if (onSelectionChange) {
      onSelectionChange(safeNext);
      return;
    }
    const ids = new Set([...selected, ...safeNext]);
    ids.forEach((resourceId) => {
      if (selected.has(resourceId) !== safeNext.has(resourceId)) onToggle(resourceId);
    });
  };
  const toggleResource = (resourceId: string) => {
    if (!selectableIds.has(resourceId)) return;
    const next = new Set(effectiveSelected);
    const requiredProjectDirectories = requiredProjectContentDirectoryIds(projectContentInventory, next);
    if (requiredProjectDirectories.has(resourceId)) return;
    if (next.has(resourceId)) next.delete(resourceId);
    else next.add(resourceId);
    if (conversations.some((resource) => resource.resource_id === resourceId) && sessionIndex) {
      const selectedSessionCount = visibleConversations.filter((resource) => next.has(resource.resource_id)).length;
      if (selectedSessionCount > 0) next.add(sessionIndex.resource_id);
      else next.delete(sessionIndex.resource_id);
    }
    commitSelection(next);
  };
  const useRecommendedForStep = () => {
    if (activeStep === "review") {
      if (safeProjectDefaults.size === projectDefaults.size) onUseProjectDefaults();
      else commitSelection(new Set(safeProjectDefaults));
      return;
    }
    const group = groupForStep(activeStep, resources);
    const next = new Set(effectiveSelected);
    group.forEach((resource) => next.delete(resource.resource_id));
    group.forEach((resource) => {
      if (safeProjectDefaults.has(resource.resource_id)) next.add(resource.resource_id);
    });
    commitSelection(next);
  };
  const clearStep = () => {
    if (activeStep === "review") {
      onClear();
      return;
    }
    const group = groupForStep(activeStep, resources);
    const next = new Set(effectiveSelected);
    group.forEach((resource) => {
      const projectEntry = projectContentInventory?.entries.find((entry) => (
        entry.descriptor.resource_id === resource.resource_id
      ));
      if (activeStep !== "project_files" || !projectEntry?.storage_present) {
        next.delete(resource.resource_id);
      }
    });
    commitSelection(next);
  };
  const stepIndex = steps.indexOf(activeStep);
  const previousStep = steps[Math.max(0, stepIndex - 1)];
  const nextStep = nextPushReviewStep(activeStep, steps);
  const goBack = () => selectStep(previousStep);
  const goNext = () => selectStep(nextStep);
  const selectedCount = effectiveSelected.size;
  const selectableConversationIds = new Set(visibleConversations
    .filter((resource) => selectableIds.has(resource.resource_id))
    .map((resource) => resource.resource_id));
  const selectableSkillIds = new Set(skills
    .filter((resource) => selectableIds.has(resource.resource_id))
    .map((resource) => resource.resource_id));
  const selectablePluginIds = new Set(plugins
    .filter((resource) => selectableIds.has(resource.resource_id))
    .map((resource) => resource.resource_id));
  const requiredProjectDirectories = requiredProjectContentDirectoryIds(
    projectContentInventory,
    effectiveSelected,
  );
  const projectFileRows: ProjectFileReviewRow[] = (projectContentInventory?.entries ?? []).map((entry) => ({
    resourceId: entry.descriptor.resource_id,
    relativePath: entry.relative_path,
    entryType: entry.entry_type,
    state: entry.state,
    size: entry.size,
    mode: entry.mode,
    sourceMtime: entry.source_mtime,
    localPresent: entry.local_present,
    storagePresent: entry.storage_present,
    newlyDiscovered: entry.newly_discovered,
    selectedAfterScan: entry.selected_after_scan,
    blockedReason: entry.blocked_reason,
    warningCode: entry.warning_code,
    warningDigest: entry.warning_digest,
  }));
  const currentGroup = groupForStep(activeStep, resources);
  const currentDefaults = currentGroup.filter((resource) => safeProjectDefaults.has(resource.resource_id));
  const currentMatchesDefaults = activeStep === "review"
    ? effectiveSelected.size === safeProjectDefaults.size
      && [...effectiveSelected].every((resourceId) => safeProjectDefaults.has(resourceId))
    : currentGroup.every((resource) => (
      effectiveSelected.has(resource.resource_id) === safeProjectDefaults.has(resource.resource_id)
    ));
  const recommendedCount = activeStep === "review" ? safeProjectDefaults.size : currentDefaults.length;

  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="sync-push-resource-title">
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2 id="sync-push-resource-title">Push review</h2>
        </div>
        <div className="sync-push-resource-actions">
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action has-count"
            onClick={useRecommendedForStep}
            disabled={busy || safeProjectDefaults.size === 0 || currentMatchesDefaults}
            title={`Use the recommended ${activeStep === "review" ? "selection" : `${activeStep} selection`}`}
            aria-label={`Use recommended selection (${recommendedCount})`}
          >
            <Icon name="check-circle" size={15} />
            <span aria-hidden="true">{recommendedCount}</span>
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action"
            onClick={clearStep}
            disabled={busy || countSelected(currentGroup, effectiveSelected) === 0}
            title="Clear current selection"
            aria-label="Clear current selection"
          >
            <Icon name="trash" size={15} />
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-inline-action-close"
            onClick={onClose}
            disabled={busy}
            aria-label="Close push review"
          >
            <Icon name="x" size={15} />
          </button>
        </div>
      </header>

      <SyncReviewTabs
        activeStep={activeStep}
        counts={counts}
        steps={steps}
        warningSteps={warningSteps}
        disabled={busy}
        onChange={selectStep}
      />

      <div
        id={`sync-review-${activeStep}-panel`}
        className="sync-inline-action-content sync-push-resource-content sync-sync-review-content"
        role="tabpanel"
        aria-labelledby={`sync-review-${activeStep}-tab`}
      >
        <div
          className="sync-inline-action-scroll sync-sync-review-scroll"
        >
          {activeStep === "history" && project && binding?.profile_ids?.codex ? (
            <ProjectChatHistoryPage
              embedded
              project={project}
              binding={binding}
              refreshEpoch={refreshEpoch}
              activeStorageId={storageId}
              activeStorageName={storageName}
              selectionMode="push"
              selectedResourceIds={effectiveSelected}
              selectableResourceIds={selectableConversationIds}
              selectionDisabled={busy}
              onToggleResource={toggleResource}
              comparisonOverride={threadComparison}
            />
          ) : activeStep === "history" ? (
            <ResourceInventory resources={conversations} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableConversationIds} disabled={busy} onToggle={toggleResource} />
          ) : null}

          {(activeStep === "skills" || activeStep === "plugins") && project ? (
            <SkillsPluginStatusPage
              view={activeStep}
              project={project}
              binding={binding}
              refreshEpoch={refreshEpoch}
              activeStorageId={storageId}
              activeStorageName={storageName}
              onOpenProjectSettings={() => undefined}
              selectionMode="push"
              selectedResourceIds={effectiveSelected}
              selectableResourceIds={activeStep === "skills" ? selectableSkillIds : selectablePluginIds}
              selectionDisabled={busy}
              onToggleResource={toggleResource}
              reportOverride={capabilityReport}
            />
          ) : activeStep === "skills" ? (
            <ResourceInventory resources={skills} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableSkillIds} disabled={busy} onToggle={toggleResource} />
          ) : activeStep === "plugins" ? (
            <ResourceInventory resources={plugins} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectablePluginIds} disabled={busy} onToggle={toggleResource} />
          ) : null}

          {activeStep === "project_files" && (
            <ProjectFilesReviewPage
              mode="push"
              eligibility={projectContentInventory?.eligibility ?? {
                state: project?.is_git_repository ? "git_managed" : "eligible",
                reason: project?.is_git_repository
                  ? "Git manages files in this project folder."
                  : "This project folder is not inside a Git work tree.",
              }}
              rows={projectFileRows}
              selectedIds={effectiveSelected}
              requiredIds={requiredProjectDirectories}
              removalIds={projectContentRemovals}
              acknowledgedWarningDigests={acknowledgedWarningDigests}
              scanned={projectContentScanned}
              loading={projectContentLoading}
              ignoredCount={projectContentInventory?.ignored_count}
              blockedCount={projectContentInventory?.blocked_count}
              warnings={projectContentInventory?.warnings}
              disabled={busy}
              onScan={onScanProjectFiles}
              onToggle={toggleResource}
              onBulkToggle={(resourceIds, shouldSelect) => {
                const next = new Set(effectiveSelected);
                resourceIds.forEach((resourceId) => {
                  if (shouldSelect) next.add(resourceId);
                  else if (!requiredProjectDirectories.has(resourceId)) next.delete(resourceId);
                });
                commitSelection(next);
              }}
              onToggleRemoval={onToggleProjectContentRemoval}
              onToggleWarning={onToggleProjectContentWarning}
              onReviewPull={onPull}
            />
          )}

          {activeStep === "review" && (
            <div className="sync-sync-review-summary">
              {missingDestination && (
                <div className="sync-callout sync-sync-review-blocker sync-sync-review-remote-missing" role="alert">
                  <Icon name="alert-triangle" size={15} />
                  <span>
                    <strong>Remote history is missing</strong>
                    Mallard previously synced generation {missingDestination.base_generation} to {storageName || "this storage"}, but that repository is not present at the configured location. Mallard cannot determine why. If this is expected, unlink and link the storage again before pushing.
                  </span>
                  <div className="sync-sync-review-recovery-actions">
                    {onOpenStorageSettings && (
                      <button type="button" className="btn btn-ghost" onClick={onOpenStorageSettings} disabled={busy}>
                        Storage settings
                      </button>
                    )}
                    {onUnlinkStorage && (
                      <button type="button" className="btn btn-ghost danger" onClick={onUnlinkStorage} disabled={busy}>
                        Unlink storage…
                      </button>
                    )}
                  </div>
                </div>
              )}
              <button type="button" onClick={() => selectStep("history")}>
                <span><Icon name="git-branch" size={15} /><strong>Sessions</strong></span>
                <span>{counts.history} included<Icon name="chevron-right" size={13} /></span>
              </button>
              <button type="button" onClick={() => selectStep("skills")}>
                <span><Icon name="folder" size={15} /><strong>Skills</strong></span>
                <span>{counts.skills} included<Icon name="chevron-right" size={13} /></span>
              </button>
              <button type="button" onClick={() => selectStep("plugins")}>
                <span><Icon name="link" size={15} /><strong>Plugins</strong></span>
                <span>{counts.plugins} included<Icon name="chevron-right" size={13} /></span>
              </button>
              {other.length > 0 && (
                <details className="sync-sync-review-other">
                  <summary>
                    <span><Icon name="settings" size={15} /><strong>Requirements</strong></span>
                    <span>{other.length} unavailable<Icon name="chevron-right" size={13} /></span>
                  </summary>
                  <ResourceInventory resources={other} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableIds} disabled={busy} onToggle={toggleResource} />
                </details>
              )}
              {sessionIndex && effectiveSelected.has(sessionIndex.resource_id) && (
                <p className="sync-sync-review-derived"><Icon name="check-circle" size={13} />The project session index is included automatically.</p>
              )}
              {blockingCount > 0 && (
                <div className="sync-callout sync-sync-review-blocker" role="alert">
                  <Icon name="alert-triangle" size={15} />
                  <span><strong>{blockerTitle}</strong>{blockerDetail}</span>
                  {storageBlockingCount > 0 && onPull && <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>Review Pull</button>}
                </div>
              )}
            </div>
          )}
        </div>

        {error && <div className="sync-callout error sync-pull-error"><Icon name="alert-triangle" size={15} /> {error}</div>}
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {blockingCount > 0 && <span><small>{blockerFooter}</small></span>}
        {stepIndex > 0 && (
          <button type="button" className="btn btn-secondary sync-sync-review-back" onClick={goBack} disabled={busy}>
            <Icon name="chevron-left" size={14} />
            Back: {STEP_LABELS[previousStep]}
          </button>
        )}
        {activeStep !== "review" ? (
          <button type="button" className="btn btn-primary" onClick={goNext} disabled={busy}>
            Next: {STEP_LABELS[nextStep]}
            <Icon name="chevron-right" size={14} />
          </button>
        ) : (
          <button type="button" className="btn btn-primary sync-pull-apply-button" onClick={onPush} disabled={busy || remoteHistoryMissing || (selectedCount === 0 && projectContentRemovals.size === 0) || blockingCount > 0}>
            <Icon name={busy ? "refresh" : "upload"} size={16} className={busy ? "icon-spin" : undefined} />
            {busy
              ? (fetchProgress && fetchProgress.phase === "push"
                ? `${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`
                : "Pushing…")
              : "Push"}
          </button>
        )}
      </footer>
    </section>
  );
}
