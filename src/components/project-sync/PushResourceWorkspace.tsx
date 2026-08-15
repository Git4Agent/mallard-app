import { useEffect, useMemo, useState } from "react";
import type {
  BundleFetchProgress,
  BundleRecipe,
  LocalProjectSummary,
  ProjectBinding,
  ProjectContentInventory,
  ProjectResourceDescriptor,
  PushDestinationStatus,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon from "../Icons";
import ProjectActivityPage from "./ProjectActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import ResourceInventory from "./ResourceInventory";
import SyncReviewTabs from "../sync-review/SyncReviewTabs";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import { type SyncReviewStep, projectReviewTabs, syncReviewSteps } from "./reviewSteps";
import { categoryFor } from "./model";
import ThreadStatusIcon from "./ThreadStatusIcon";
import { isIrrecoverableSyncError } from "./syncError";

const EMPTY_STATUSES = new Map<string, string>();
const STORAGE_BLOCKING_STATES = new Set(["storage_only", "storage_ahead", "diverged"]);

export interface PushReviewBlocker {
  kind: "storage" | "local";
  step: SyncReviewStep;
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
  onClose: () => void;
  onPush: () => void;
  onPull?: () => void;
  onUnlinkStorage?: () => void;
  onResetMetadata?: () => void;
}

function countSelected(items: ProjectResourceDescriptor[], selected: ReadonlySet<string>): number {
  return items.filter((item) => selected.has(item.resource_id)).length;
}

function groupForStep(step: SyncReviewStep, resources: ProjectResourceDescriptor[]): ProjectResourceDescriptor[] {
  // The history step hosts every non-file resource: sessions in the list and
  // the tools/requirements rows at the bottom of the same panel.
  if (step === "project_files") return resources.filter((resource) => categoryFor(resource) === "project_files");
  return resources.filter((resource) => categoryFor(resource) !== "project_files");
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

export interface PullRequiredCopy {
  title: string;
  body: string;
  /** Lineage numbers, kept for support and out of the leading sentence. */
  detail: string;
}

/**
 * Customer-facing copy for a destination that has to be pulled before it can
 * accept a push. The reason codes are lineage vocabulary — revisions, bases,
 * replicas — and none of it means anything to the person holding a blocked
 * Push. The callout says which storage moved, what a Pull would show, and why
 * pushing first is unsafe; the numbers stay on the detail line so support can
 * still read them off a screenshot.
 */
export function pullRequiredCopy(
  reason: "remote_without_base" | "binding_changed" | "remote_advanced",
  storageName: string | null | undefined,
  remoteRevision: number,
  baseRevision: number | null | undefined,
  changedConversationCount = 0,
): PullRequiredCopy {
  const storage = storageName?.trim() || "this storage";
  // The three fields stack as one banner: title, one sentence, and the lineage
  // figures abbreviated to what support actually reads off a screenshot. The
  // footer button says what to do, so the sentence only has to say what is
  // there and what pushing past it would cost.
  const detail = `Revision ${remoteRevision} · ${baseRevision != null ? `reviewed ${baseRevision}` : "never reviewed"}`;
  if (reason === "remote_advanced") {
    return {
      title: "Storage has newer changes",
      body: changedConversationCount > 0
        ? `${changedConversationCount} conversation${changedConversationCount === 1 ? " is" : "s are"} newer in ${storage}. Pushing could overwrite them.`
        : `${storage} changed after this computer last synced. Pushing could overwrite it.`,
      detail,
    };
  }
  if (reason === "remote_without_base") {
    return {
      title: "Storage has unreviewed data",
      body: `${storage} holds data never reviewed on this computer.`,
      detail,
    };
  }
  return {
    title: "Project changed since last sync",
    body: `The project folder or Codex profile is different now. Review ${storage} first.`,
    detail,
  };
}

/**
 * Conversation threads that cannot push until a Pull resolves them
 * (diverged or behind storage). They hold back only themselves: the Push
 * button stays enabled and their cloud versions are retained untouched.
 */
export function pushReviewPullHolds(
  threadComparison?: ThreadSyncComparison | null,
): ThreadSyncEntry[] {
  return (threadComparison?.entries ?? []).filter((entry) => (
    entry.push_decision?.action === "require_pull"
    || entry.push_decision?.action === "require_pull_metadata"
  ));
}

export function pushReviewBlockers(
  selected: ReadonlySet<string>,
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
  acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
): PushReviewBlocker[] {
  const blockers: PushReviewBlocker[] = [];
  for (const entry of threadComparison?.entries ?? []) {
    // Thread rows never disable the whole Push: a diverged or held thread
    // is unselectable and keeps its cloud version, while every other
    // selected thread still publishes.
    if (entry.cloud_thread_id) {
      continue;
    }
    const selectedEntry = selected.has(entry.resource_id);
    const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
    if (kind) blockers.push({ kind, step: "history", resourceId: entry.resource_id, state: entry.state });
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
  projectContentInventory?: ProjectContentInventory | null,
  acknowledgedWarningDigests: ReadonlySet<string> = new Set(),
): number {
  return pushReviewBlockers(
    selected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
  ).length;
}

export function pushSelectableResourceIds(
  resources: ProjectResourceDescriptor[],
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const unselectableIds = new Set<string>();
  for (const entry of threadComparison?.entries ?? []) {
    if (!entry.push_decision.selectable) {
      unselectableIds.add(entry.resource_id);
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
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const selectableIds = pushSelectableResourceIds(
    resources,
    threadComparison,
    projectContentInventory,
  );
  return new Set([...selected].filter((resourceId) => selectableIds.has(resourceId)));
}

export function recommendedPushSelection(
  selected: ReadonlySet<string>,
  resources: ProjectResourceDescriptor[],
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): Set<string> {
  const selectableIds = pushSelectableResourceIds(
    resources,
    threadComparison,
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
  onClose,
  onPush,
  onPull,
  onUnlinkStorage,
  onResetMetadata,
}: Props) {
  const steps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const [uncontrolledActiveStep, setUncontrolledActiveStep] = useState<SyncReviewStep>(initialStep);
  const requestedActiveStep = controlledActiveStep ?? uncontrolledActiveStep;
  const activeStep = steps.includes(requestedActiveStep) ? requestedActiveStep : "history";
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
  const projectFiles = useMemo(
    () => resources.filter((resource) => categoryFor(resource) === "project_files"),
    [resources],
  );
  const other = useMemo(
    () => resources.filter((resource) => categoryFor(resource) === "tools"),
    [resources],
  );
  const selectableIds = useMemo(
    () => pushSelectableResourceIds(resources, threadComparison, projectContentInventory),
    [projectContentInventory, resources, threadComparison],
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
    history: countSelected(visibleConversations, effectiveSelected) + countSelected(other, effectiveSelected),
    project_files: countSelected(projectFiles, effectiveSelected),
  };
  const blockers = pushReviewBlockers(
    effectiveSelected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
  );
  const blockingCount = blockers.length;
  const pullHolds = pushReviewPullHolds(threadComparison);
  // Gates on the destination close this workspace before it opens (they exit
  // to the storage-card notice); the flags stay only as a defensive disable.
  const remoteHistoryMissing = destinationStatus?.state === "remote_history_missing";
  const remoteHistoryReplaced = destinationStatus?.state === "remote_history_replaced";
  const remotePullRequired = destinationStatus?.state === "requires_pull";
  const storageBlockingCount = blockers.filter((blocker) => blocker.kind === "storage").length;
  const localBlockingCount = blockingCount - storageBlockingCount;
  const warningSteps = new Set<SyncReviewStep>();
  blockers.forEach((blocker) => warningSteps.add(blocker.step));
  // One banner covers everything a Pull would clear plus anything locally
  // stuck: held threads and storage blockers describe the same situation, so
  // they share a count instead of stacking two callouts that say Pull twice.
  const heldConversationCount = new Set([
    ...pullHolds.map((entry) => entry.resource_id),
    ...blockers.filter((blocker) => blocker.kind === "storage").map((blocker) => blocker.resourceId),
  ]).size;
  // Selection-driven, so it reports in the footer rather than the banner zone
  // above the list — see the banner comment.
  const localBlockerSentence = localBlockingCount > 0
    ? `${localBlockingCount} selected item${localBlockingCount === 1 ? "" : "s"} can't be captured — exclude ${localBlockingCount === 1 ? "it" : "them"} to push.`
    : null;
  const pushDisabledReason = remoteHistoryMissing || remoteHistoryReplaced || remotePullRequired
    ? "Resolve the storage notice before pushing"
    : localBlockingCount > 0
      ? "Exclude the items that can't be captured"
      : blockingCount > 0
        ? "Resolve the held items above before pushing"
        : effectiveSelected.size === 0 && projectContentRemovals.size === 0
          ? "Nothing is selected"
          : null;

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
    const group = groupForStep(activeStep, resources);
    const next = new Set(effectiveSelected);
    group.forEach((resource) => next.delete(resource.resource_id));
    group.forEach((resource) => {
      if (safeProjectDefaults.has(resource.resource_id)) next.add(resource.resource_id);
    });
    commitSelection(next);
  };
  const clearStep = () => {
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
  const selectableConversationIds = new Set(visibleConversations
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
  const currentMatchesDefaults = currentGroup.every((resource) => (
    effectiveSelected.has(resource.resource_id) === safeProjectDefaults.has(resource.resource_id)
  ));
  const recommendedCount = currentDefaults.length;

  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="sync-push-resource-title">
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2 id="sync-push-resource-title">Push review</h2>
        </div>
        <SyncReviewSelectionBar
          count={countSelected(currentGroup, effectiveSelected)}
          total={currentGroup.length}
          busy={busy}
          recommendedTitle={`Use the recommended ${activeStep === "history" ? "session" : "project file"} selection (${recommendedCount})`}
          recommendedAriaLabel={`Use recommended selection (${recommendedCount})`}
          recommendedDisabled={safeProjectDefaults.size === 0 || currentMatchesDefaults}
          onRecommended={useRecommendedForStep}
          clearDisabled={countSelected(currentGroup, effectiveSelected) === 0}
          onClear={clearStep}
          close={{ ariaLabel: "Close push review", onClose }}
        />
      </header>

      <SyncReviewTabs
        tabs={projectReviewTabs(steps, counts, warningSteps)}
        activeId={activeStep}
        disabled={busy}
        onChange={(step) => selectStep(step as SyncReviewStep)}
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
          {heldConversationCount > 0 && (
            /* Held threads and storage blockers describe one situation, so one
               banner speaks for both. Everything here is derived from the plan,
               never from the selection: this zone sits above the list, so a
               banner that appeared on a checkbox click would shove the row out
               from under the cursor. Selection-driven problems report in the
               footer, beside the button they block. */
            <div className="sync-review-banners">
              <div className="sync-callout sync-sync-review-pull-holds" role="status">
                <ThreadStatusIcon status="divergent" size={16} />
                <span>
                  <strong>
                    {`${heldConversationCount} conversation${heldConversationCount === 1 ? "" : "s"} held for Pull`}
                  </strong>
                  Storage has newer versions. Selected threads still push.
                </span>
                {onPull && (
                  <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>Open Pull review</button>
                )}
              </div>
            </div>
          )}
          {activeStep === "history" && (
            <>
              {project && binding ? (
                <ProjectActivityPage
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
                  onReviewPull={onPull}
                  onUnlinkStorage={onUnlinkStorage}
                  continuityActionBusy={busy}
                  comparisonOverride={threadComparison}
                />
              ) : (
                <ResourceInventory resources={conversations} selected={effectiveSelected} statuses={EMPTY_STATUSES} selectableResourceIds={selectableConversationIds} disabled={busy} onToggle={toggleResource} />
              )}
              {/* No disclosure wrapper here: `other` is exactly the `tools`
                  category, so ResourceInventory renders one group whose own
                  header is already the word Requirements, a count, Include all
                  and a fold. Wrapping it printed that header twice — two
                  chevrons and two disagreeing counts for one list. The pull
                  side keeps its wrapper because ItemList has no header at all. */}
              {other.length > 0 && (
                <ResourceInventory
                  resources={other}
                  selected={effectiveSelected}
                  statuses={EMPTY_STATUSES}
                  selectableResourceIds={selectableIds}
                  disabled={busy}
                  defaultCollapsed
                  onToggle={toggleResource}
                />
              )}
            </>
          )}

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

        </div>

        {error && (
          <div className="sync-callout error sync-pull-error" role="alert">
            <Icon name="alert-triangle" size={15} />
            <span>{error}</span>
            {isIrrecoverableSyncError(error) && onResetMetadata && (
              <button type="button" className="btn btn-ghost danger" onClick={onResetMetadata} disabled={busy}>
                <Icon name="trash" size={13} />
                Reset local sync state
              </button>
            )}
          </div>
        )}
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {localBlockerSentence && (
          <span role="status"><small>{localBlockerSentence}</small></span>
        )}
        {busy && (
          /* Every exit is sealed while the transfer runs, on purpose: the
             backend has no safe mid-write abort. Saying so beats leaving the
             user hunting for a cancel that does not exist. */
          <span role="status"><small>A running push finishes or fails whole — it can’t be cancelled midway.</small></span>
        )}
        <button
          type="button"
          className="btn btn-primary sync-pull-apply-button"
          onClick={onPush}
          disabled={busy || pushDisabledReason !== null}
          title={pushDisabledReason ?? undefined}
        >
          {busy
            ? <Icon name="refresh" size={16} className="icon-spin" />
            : <ThreadStatusIcon status="local-outstanding" size={16} />}
          {busy
            ? (fetchProgress && fetchProgress.phase === "push"
              ? <span className="sync-btn-readout">{`${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`}</span>
              : "Pushing…")
            : "Push"}
        </button>
      </footer>
    </section>
  );
}
