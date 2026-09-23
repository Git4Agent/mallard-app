import { useEffect, useMemo, useRef, useState } from "react";
import type {
  BundleFetchProgress,
  LocalProjectSummary,
  ProjectBinding,
  ProjectContentEntry,
  ProjectContentInventory,
  InventoryResource,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon from "../Icons";
import Notice, { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import ProjectActivityPage from "./ProjectActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import ProjectFileCommitImpact, {
  projectFileDeletionCount,
  type ProjectFileCommitImpactCounts,
} from "./ProjectFileCommitImpact";
import ResourceInventory from "./ResourceInventory";
import SyncReviewTabs from "../sync-review/SyncReviewTabs";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import NameSearchField from "../sync-review/NameSearchField";
import { nameMatchesSearch, type NameSearchResults } from "../sync-review/nameSearch";
import { type SyncReviewStep, historyStepLabel, projectReviewTabs, syncReviewSteps } from "./reviewSteps";
import { STORAGE_BLOCKING_STATES } from "./model";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { isPushReviewDecision } from "../sync-review/pullReviewStatus";
import { formatBytes } from "../../format";
import { PUSH_FINISHED, finishPushOffered } from "../../unfinishedPush";

export interface PushReviewBlocker {
  kind: "storage" | "local";
  step: SyncReviewStep;
  resourceId: string;
  state: string;
}

interface Props {
  credentialRedactionEnabled: boolean;
  resources: InventoryResource[];
  selected: Set<string>;
  projectDefaults: Set<string>;
  busy: boolean;
  error: string | null;
  /* The backend opened the review without a conversation comparison. */
  threadComparisonError?: string | null;
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
  /** Owned above the panel: the path bar's Push publishes this same review,
   *  so its refusal has to see the acknowledgement too. */
  deletionApproved?: boolean;
  onDeletionApprovedChange?: (approved: boolean) => void;
  /** Live upload progress while the push publishes; renders in the footer button. */
  fetchProgress?: BundleFetchProgress | null;
  showProjectFiles?: boolean;
  activeStep?: SyncReviewStep;
  onStepChange?: (step: SyncReviewStep) => void;
  onToggle: (resourceId: string) => void;
  onSelectionChange?: (selected: Set<string>) => void;
  onScanProjectFiles?: () => void;
  onToggleProjectContentWarning?: (warningDigest: string) => void;
  onClose: () => void;
  onPush: () => void;
  onPull?: () => void;
  /** Finish that push: the record an earlier push claimed and never wrote is
   *  written once. Offered only while the error is that sentence. */
  onFinishPush?: () => void;
}

function countSelected(items: InventoryResource[], selected: ReadonlySet<string>): number {
  return items.filter((item) => selected.has(item.resource_id)).length;
}

function groupForStep(step: SyncReviewStep, resources: InventoryResource[]): InventoryResource[] {
  // The history step hosts every non-file resource: sessions in the list and
  // the tools/requirements rows at the bottom of the same panel.
  if (step === "project_files") return resources.filter((resource) => resource.category === "project_files");
  return resources.filter((resource) => resource.category !== "project_files");
}

function isLocalOutstandingProjectContent(entry: ProjectContentEntry): boolean {
  return entry.local_present && (entry.state === "local_only" || entry.state === "local_ahead");
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
 * Push. The callout says which storage moved, what a Pull would show, and
 * what the review unlocks: newer conversations are never overwritten by a
 * push (they are carried forward and held per thread), so the sentence
 * points at the review instead of threatening data loss. The numbers stay on
 * the detail line so support can still read them off a screenshot.
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
  // there and what the review unlocks.
  const detail = `Version ${remoteRevision} · ${baseRevision != null ? `reviewed ${baseRevision}` : "review required"}`;
  if (reason === "remote_advanced") {
    const one = changedConversationCount === 1;
    return {
      title: "Storage has newer changes",
      body: changedConversationCount > 0
        ? `${changedConversationCount} conversation${one ? " is" : "s are"} newer in ${storage}. Review ${one ? "it" : "them"} in Pull — ${one ? "it stays" : "they stay"} held afterwards while your other sessions push.`
        : `${storage} changed after this computer last synced. Review it in Pull before pushing.`,
      detail,
    };
  }
  if (reason === "remote_without_base") {
    return {
      title: "Storage review required",
      body: `Review the existing data in ${storage} before pushing from this connection.`,
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
  projectContentRemovals: ReadonlySet<string> = new Set(),
): PushReviewBlocker[] {
  const blockers: PushReviewBlocker[] = [];
  for (const entry of threadComparison?.entries ?? []) {
    // Thread rows never disable the whole Push: a diverged or held thread
    // is unselectable and keeps its cloud version, while every other
    // selected thread still publishes.
    if (entry.compared_against_storage) {
      continue;
    }
    const selectedEntry = selected.has(entry.resource_id);
    const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
    if (kind) blockers.push({ kind, step: "history", resourceId: entry.resource_id, state: entry.state });
  }
  for (const entry of projectContentInventory?.entries ?? []) {
    const resourceId = entry.descriptor.resource_id;
    if (projectContentRemovals.has(resourceId)) continue;
    const selectedEntry = selected.has(resourceId);
    const kind = pushBlockerKind(entry.state, entry.storage_present, selectedEntry);
    if (kind) {
      blockers.push({
        kind,
        step: "project_files",
        resourceId,
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

export function pushSelectableResourceIds(
  resources: InventoryResource[],
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
      && resource.selectable
      && !unselectableIds.has(resource.resource_id)
    ))
    .map((resource) => resource.resource_id));
}

export function sanitizePushSelection(
  selected: ReadonlySet<string>,
  resources: InventoryResource[],
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
  resources: InventoryResource[],
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

  const conversationIds = new Set(resources
    .filter((resource) => resource.category === "conversations")
    .map((resource) => resource.resource_id));
  const conversations = threadComparison.entries
    .filter((entry) => conversationIds.has(entry.resource_id))
    .sort((left, right) => (
      (right.local_updated_at ?? 0) - (left.local_updated_at ?? 0)
      || (left.provider < right.provider ? -1 : left.provider > right.provider ? 1 : 0)
      || (left.resource_id < right.resource_id ? -1 : left.resource_id > right.resource_id ? 1 : 0)
    ));

  // Rebuild session defaults so a saved selection cannot exceed the budget.
  // Reserve the same fixed content that the review meter counts.
  for (const entry of conversations) recommended.delete(entry.resource_id);
  const fixedBudget = pushBudgetTotal(recommended, threadComparison, projectContentInventory);
  let remainingBytes = fixedBudget ? fixedBudget.limit - fixedBudget.total : Infinity;
  for (const entry of conversations) {
    if (!selectableIds.has(entry.resource_id)) continue;
    const bytes = entry.capture_bytes ?? 0;
    if (bytes > remainingBytes) continue;
    recommended.add(entry.resource_id);
    remainingBytes -= bytes;
  }

  return recommended;
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

export function pushProjectFileCommitImpact(
  inventory: ProjectContentInventory | null | undefined,
  selected: ReadonlySet<string>,
  removals: ReadonlySet<string>,
): ProjectFileCommitImpactCounts {
  const counts: ProjectFileCommitImpactCounts = {
    add: 0,
    replace: 0,
    deleteFile: 0,
    deleteDirectory: 0,
  };
  for (const entry of inventory?.entries ?? []) {
    const resourceId = entry.descriptor.resource_id;
    if (removals.has(resourceId)) {
      if (entry.entry_type === "directory") counts.deleteDirectory += 1;
      else counts.deleteFile += 1;
      continue;
    }
    if (!selected.has(resourceId) || entry.entry_type === "blocked") continue;
    if (!entry.storage_present) {
      counts.add += 1;
    } else if (entry.entry_type === "file" && entry.state !== "synced") {
      counts.replace += 1;
    }
  }
  return counts;
}

/** What the current selection would put into one push: the selected sessions
 *  by their capture bytes, the requirements the push always carries, and the
 *  selected project files by size. Null when the review carries no budget
 *  (a review prepared without a comparison, or a Pull). */
export function pushBudgetTotal(
  selected: ReadonlySet<string>,
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): { sessions: number; reserved: number; projectFiles: number; total: number; limit: number } | null {
  const budget = threadComparison?.capture_budget;
  if (!budget) return null;
  const sessions = (threadComparison?.entries ?? [])
    .filter((entry) => entry.local_present && selected.has(entry.resource_id))
    .reduce((sum, entry) => sum + (entry.capture_bytes ?? 0), 0);
  const projectFiles = (projectContentInventory?.entries ?? [])
    .filter((entry) => entry.local_present && selected.has(entry.descriptor.resource_id))
    .reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  const total = sessions + budget.reserved_bytes + projectFiles;
  return { sessions, reserved: budget.reserved_bytes, projectFiles, total, limit: budget.limit_bytes };
}

/** Why Push must refuse, or null when it may proceed.
 *
 *  Both buttons that publish a review read this one rule. They had a rule
 *  each before, and only the footer's was complete: the path bar's Push
 *  publishes the open review rather than opening one, and its own condition
 *  knew nothing about the capture budget or the project-file deletion
 *  acknowledgement — so a review the footer refused, over the limit or with
 *  unapproved deletions, could still be pushed from the bar above it. The
 *  string is both buttons' title, so the refusal is worded once too. */
export function pushReviewDisabledReason({
  selected,
  threadComparison = null,
  projectContentInventory = null,
  acknowledgedWarningDigests = new Set<string>(),
  projectContentRemovals = new Set<string>(),
  deletionApproved,
  projectContentLoading = false,
}: {
  /** Sanitized: `sanitizePushSelection`, which is what the push carries. */
  selected: ReadonlySet<string>;
  threadComparison?: ThreadSyncComparison | null;
  projectContentInventory?: ProjectContentInventory | null;
  acknowledgedWarningDigests?: ReadonlySet<string>;
  projectContentRemovals?: ReadonlySet<string>;
  deletionApproved: boolean;
  projectContentLoading?: boolean;
}): string | null {
  /* First: while the scan is running every judgement below it is provisional,
     including the size of the push. */
  if (projectContentLoading) return "Project files are still being scanned";
  const budget = pushBudgetTotal(selected, threadComparison, projectContentInventory);
  if (budget && budget.total > budget.limit) return pushBudgetSentence(budget);
  const blockers = pushReviewBlockers(
    selected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
    projectContentRemovals,
  );
  if (blockers.some((blocker) => blocker.kind !== "storage")) {
    return "Exclude the items that can't be captured";
  }
  if (blockers.length > 0) return "Resolve the held items above before pushing";
  const impact = pushProjectFileCommitImpact(
    projectContentInventory,
    selected,
    projectContentRemovals,
  );
  if (projectFileDeletionCount(impact) > 0 && !deletionApproved) {
    return "Approve the selected project file deletions";
  }
  if (selected.size === 0 && projectContentRemovals.size === 0) return "Nothing is selected";
  return null;
}

type PushBudget = NonNullable<ReturnType<typeof pushBudgetTotal>>;

/** What the meter says in one line: the size while it fits, the overshoot and
 *  the way out past the limit. The over sentence is also the Push button's
 *  title, so the refusal and its reason are worded once. */
function pushBudgetSentence(total: PushBudget): string {
  return total.total > total.limit
    ? `${formatBytes(total.total - total.limit)} over the ${formatBytes(total.limit)} push limit — exclude sessions or files to push.`
    : `This push · ${formatBytes(total.total)} of ${formatBytes(total.limit)}`;
}

function pushBudgetBreakdown(total: PushBudget): string {
  return `Sessions ${formatBytes(total.sessions)} · Requirements ${formatBytes(total.reserved)} · Project files ${formatBytes(total.projectFiles)}`;
}

/** `424 MB / 512 MB` while it fits, the overshoot alone once it does not —
 *  `short` drops the trailing "over 512 MB" where the sentence beside it has
 *  already named the limit. Carries the meter semantics for both instances. */
function PushBudgetFigure({ total, short = false, label }: {
  total: PushBudget;
  short?: boolean;
  label?: string;
}) {
  const over = total.total - total.limit;
  return (
    <span
      className={`sync-push-budget-figure${over > 0 ? " over" : ""}`}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={total.limit}
      aria-valuenow={Math.min(total.total, total.limit)}
      aria-valuetext={pushBudgetSentence(total)}
    >
      {over > 0
        ? <><b>+{formatBytes(over)}</b>{short ? "" : ` over ${formatBytes(total.limit)}`}</>
        : <><b>{formatBytes(total.total)}</b> / {formatBytes(total.limit)}</>}
    </span>
  );
}

/** The bar under the header's figure. Scaled to max(total, limit): while the
 *  push fits, the track's right end IS the limit; past it the limit becomes a
 *  tick the fill visibly runs beyond, so the overshoot reads as a distance and
 *  not only as a number.
 *
 *  One fill, not a segment per source. Sessions, requirements and project
 *  files were drawn in three tones first, and the palette cannot carry them at
 *  6px: --text-3 against --thread-status-local measures 1.05:1 on cream, so
 *  the requirements segment simply disappeared into the sessions one. The
 *  breakdown stays where it has always been — the title on both instances.
 *  Decoration: the figure beside it is the meter. */
function PushBudgetTrack({ total }: { total: PushBudget }) {
  const scale = Math.max(total.total, total.limit);
  const over = total.total > total.limit;
  return (
    <span className={`sync-push-budget-track${over ? " over" : ""}`} aria-hidden="true">
      <i style={{ width: `${(total.total / scale) * 100}%` }} />
      {over && <b style={{ left: `${(total.limit / scale) * 100}%` }} />}
    </span>
  );
}

/** The header's glance: the same measurement as the footer gauge, in the one
 *  other zone of the review that does not scroll away, so what this push
 *  carries is on screen at the top of a long list as well as at the button.
 *  It is not a live region — the footer's is, and two would read every tick
 *  out twice. */
function PushBudgetGlance({ total }: { total: PushBudget }) {
  return (
    <div className="sync-push-budget-glance" title={pushBudgetBreakdown(total)}>
      <PushBudgetTrack total={total} />
      <PushBudgetFigure total={total} short label="Push size" />
    </div>
  );
}

/** The footer gauge: a pill the height of the button that fills left to right
 *  behind its own figure, set flush against Push so the quantity and the act
 *  that spends it read as one object. Live with the selection, which is why it
 *  sits beside the button and not in the banner zone above the list. */
function PushBudgetGauge({ total }: { total: PushBudget }) {
  const over = total.total > total.limit;
  return (
    <div
      className={`sync-push-budget-gauge${over ? " over" : ""}`}
      role="status"
      title={pushBudgetBreakdown(total)}
    >
      {/* Past the limit there is nothing left to fill, and painting the danger
          ink as a wash under its own figure took that figure to 3.4:1 on the
          light theme. The plate and the ring carry the state instead; how far
          over it went is the header track's job. */}
      {!over && (
        <span
          className="sync-push-budget-fill"
          style={{ width: `${(total.total / total.limit) * 100}%` }}
          aria-hidden="true"
        />
      )}
      <PushBudgetFigure total={total} short />
    </div>
  );
}

export default function PushResourceWorkspace({
  credentialRedactionEnabled,
  resources,
  selected,
  projectDefaults,
  busy,
  error,
  threadComparisonError = null,
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
  deletionApproved = false,
  onDeletionApprovedChange,
  fetchProgress = null,
  showProjectFiles = false,
  activeStep: controlledActiveStep,
  onStepChange,
  onToggle,
  onSelectionChange,
  onScanProjectFiles,
  onToggleProjectContentWarning,
  onClose,
  onPush,
  onPull,
  onFinishPush,
}: Props) {
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<NameSearchResults | null>(null);
  useEffect(() => { setSearchQuery(""); setSearchResults(null); }, [project?.local_project_id, storageId]);
  const steps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const [uncontrolledActiveStep, setUncontrolledActiveStep] = useState<SyncReviewStep>("history");
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
    () => resources.filter((resource) => resource.category === "conversations"),
    [resources],
  );
  const projectFiles = useMemo(
    () => resources.filter((resource) => resource.category === "project_files"),
    [resources],
  );
  const other = useMemo(
    () => resources.filter((resource) => resource.category === "tools"),
    [resources],
  );
  const visibleConversationResourceIds = useMemo(
    () => new Set((threadComparison?.entries ?? [])
      .filter((entry) => (
        entry.local_present
        && isPushReviewDecision(entry.push_decision)
      ))
      .map((entry) => entry.resource_id)),
    [threadComparison],
  );
  const visibleConversations = useMemo(
    () => conversations.filter((resource) => visibleConversationResourceIds.has(resource.resource_id)),
    [conversations, visibleConversationResourceIds],
  );
  const visibleProjectContentEntries = useMemo(
    () => (projectContentInventory?.entries ?? []).filter(isLocalOutstandingProjectContent),
    [projectContentInventory],
  );
  const visibleProjectFileResourceIds = useMemo(
    () => new Set(visibleProjectContentEntries.map((entry) => entry.descriptor.resource_id)),
    [visibleProjectContentEntries],
  );
  const visibleProjectFiles = useMemo(
    () => projectFiles.filter((resource) => visibleProjectFileResourceIds.has(resource.resource_id)),
    [projectFiles, visibleProjectFileResourceIds],
  );
  const visibleResources = useMemo(
    () => [...visibleConversations, ...other, ...visibleProjectFiles],
    [other, visibleConversations, visibleProjectFiles],
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
  const projectFileImpact = useMemo(
    () => pushProjectFileCommitImpact(
      projectContentInventory,
      effectiveSelected,
      projectContentRemovals,
    ),
    [effectiveSelected, projectContentInventory, projectContentRemovals],
  );

  const counts = {
    history: countSelected(visibleConversations, effectiveSelected) + countSelected(other, effectiveSelected),
    project_files: countSelected(visibleProjectFiles, effectiveSelected),
  };
  const blockers = pushReviewBlockers(
    effectiveSelected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
    projectContentRemovals,
  );
  const blockingCount = blockers.length;
  const pullHolds = pushReviewPullHolds(threadComparison);
  const storageBlockingCount = blockers.filter((blocker) => blocker.kind === "storage").length;
  const localBlockingCount = blockingCount - storageBlockingCount;
  const warningSteps = new Set<SyncReviewStep>();
  blockers.forEach((blocker) => warningSteps.add(blocker.step));
  const heldConversationCount = new Set([
    ...pullHolds.map((entry) => entry.resource_id),
    ...blockers
      .filter((blocker) => blocker.kind === "storage" && blocker.step === "history")
      .map((blocker) => blocker.resourceId),
  ]).size;
  const heldProjectFileCount = new Set(blockers
    .filter((blocker) => blocker.kind === "storage" && blocker.step === "project_files")
    .map((blocker) => blocker.resourceId)).size;
  /* The banner zone that heads the review scroll. Everything in it is derived
     from the plan, never from the selection: this zone sits above the list, so
     a banner that appeared on a checkbox click would shove the row out from
     under the cursor. Selection-driven problems report in the footer, beside
     the button they block. Being plan-derived is also what makes the zone
     standing — it is already on screen when the review opens, so it is a named
     region and not a live one. */
  const banners: NoticeItem[] = [];
  if (heldConversationCount > 0) {
    banners.push({
      key: "pull-holds",
      // Nothing is broken here — the selected threads still publish — but the
      // push is not the whole truth while these are held back, and the Pull
      // this line offers is the action that clears them. Information that may
      // require action is a warning, not a note.
      severity: "warning",
      // The rows' own divergent mark, not the generic warning triangle: this
      // banner speaks for the threads carrying that status, and the mark is
      // what ties it to them. The rung is still named in the text.
      mark: <ThreadStatusIcon status="divergent" size={16} />,
      title: `${heldConversationCount} conversation${heldConversationCount === 1 ? "" : "s"} held for Pull`,
      message: "Storage has newer versions. Selected threads still push.",
      action: onPull
        ? (
          <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>Open Pull review</button>
        )
        : undefined,
    });
  }
  if (heldProjectFileCount > 0) {
    banners.push({
      key: "project-file-pull-holds",
      severity: "warning",
      mark: <ThreadStatusIcon status="cloud-outstanding" size={16} />,
      title: `${heldProjectFileCount} project file${heldProjectFileCount === 1 ? "" : "s"} ${heldProjectFileCount === 1 ? "requires" : "require"} Pull`,
      message: "Storage has newer file changes. Review them before pushing local project files.",
      action: onPull
        ? (
          <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>Open Pull review</button>
        )
        : undefined,
    });
  }
  /* No comparison means no row can be selected: every session shows its
     fallback status and the counts read 0/0. That is a broken review, not an
     empty one, so it says why at the top. Reset is offered only when the
     error is recognised stale sync metadata; an unreachable storage never
     reaches this review (it stops at the storage card), and any other
     failure must not tempt a repair that forgets sync bases. */
  if (!credentialRedactionEnabled) {
    banners.push({
      key: "credential-redaction",
      severity: "warning",
      message: "Exported content can include API keys, tokens, and passwords.",
    });
  }
  if (threadComparisonError) {
    banners.push({
      key: "comparison-failed",
      severity: "danger",
      title: "Conversations couldn't be compared, so nothing can be selected",
      message: threadComparisonError,
    });
  }
  /* Plan-derived is not the same as present-at-open. Project-file holds come
     from an inventory that can be null until the scan starts, so the zone can
     appear after the review opens. Standing only if it was there at open. */
  const bannersAtOpen = useRef<boolean | null>(null);
  if (bannersAtOpen.current === null) bannersAtOpen.current = banners.length > 0;
  const bannerTiming: NoticeTiming = bannersAtOpen.current
    ? { timing: "standing", label: "Notes about this Push review" }
    : { timing: "progress" };
  // Selection-driven, so it reports in the footer rather than the banner zone
  // above the list — see the banner comment.
  const localBlockerSentence = localBlockingCount > 0
    ? `${localBlockingCount} selected item${localBlockingCount === 1 ? "" : "s"} can't be captured — exclude ${localBlockingCount === 1 ? "it" : "them"} to push.`
    : null;
  // Every eligible session remains selectable. Only the aggregate byte
  // ceiling can refuse Apply.
  const budgetTotal = pushBudgetTotal(effectiveSelected, threadComparison, projectContentInventory);
  const overLimitSentence = budgetTotal && budgetTotal.total > budgetTotal.limit
    ? pushBudgetSentence(budgetTotal)
    : null;
  /* Not recomputed here: the path bar's Push publishes this same review, and
     the two conditions diverged for as long as each owned its own. */
  const pushDisabledReason = pushReviewDisabledReason({
    selected: effectiveSelected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
    projectContentRemovals,
    deletionApproved,
    projectContentLoading,
  });

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
    commitSelection(next);
  };
  const useRecommendedForStep = () => {
    const group = searchGroup;
    const next = new Set(effectiveSelected);
    group.forEach((resource) => next.delete(resource.resource_id));
    group.forEach((resource) => {
      if (safeProjectDefaults.has(resource.resource_id)) next.add(resource.resource_id);
    });
    commitSelection(next);
  };
  const clearStep = () => {
    const group = searchGroup;
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
  const projectFileRows: ProjectFileReviewRow[] = visibleProjectContentEntries.map((entry) => ({
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
  const currentGroup = groupForStep(activeStep, visibleResources);
  const searchActive = activeStep === "history" && searchQuery.trim().length > 0;
  const matchingConversations = visibleConversations.filter((resource) => nameMatchesSearch(resource.display_name, searchQuery));
  const matchingIds = project && binding
    ? (searchResults?.query === searchQuery ? searchResults.resourceIds : new Set<string>())
    : new Set(matchingConversations.map((resource) => resource.resource_id));
  const searchGroup = searchActive
    ? currentGroup.filter((resource) => resource.category === "conversations" && matchingIds.has(resource.resource_id))
    : currentGroup;
  const currentDefaults = searchGroup.filter((resource) => safeProjectDefaults.has(resource.resource_id));
  const currentMatchesDefaults = searchGroup.every((resource) => (
    effectiveSelected.has(resource.resource_id) === safeProjectDefaults.has(resource.resource_id)
  ));
  const recommendedCount = currentDefaults.length;

  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="sync-push-resource-title">
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2 id="sync-push-resource-title">Push review</h2>
          <span className="sync-settings-flag">Credential redaction: {credentialRedactionEnabled ? "On" : "Off"}</span>
        </div>
        {budgetTotal && <PushBudgetGlance total={budgetTotal} />}
        <SyncReviewSelectionBar
          count={countSelected(currentGroup, effectiveSelected)}
          total={currentGroup.length}
          busy={busy}
          recommendedTitle={`Use the recommended ${searchActive ? "matching session" : activeStep === "history" ? "session" : "project file"} selection (${recommendedCount})`}
          recommendedAriaLabel={`Use recommended selection${searchActive ? " for matching sessions" : ""} (${recommendedCount})`}
          recommendedDisabled={currentDefaults.length === 0 || currentMatchesDefaults}
          onRecommended={useRecommendedForStep}
          searchActive={searchActive}
          clearDisabled={countSelected(searchGroup, effectiveSelected) === 0}
          onClear={clearStep}
          close={{ ariaLabel: "Close push review", onClose }}
        />
      </header>

      <SyncReviewTabs
        tabs={projectReviewTabs(steps, counts, warningSteps, historyStepLabel(showProjectFiles))}
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
          {banners.length > 0 && (
            /* The wrapper stays: it owns the zone's spacing and resets the
               box's own margin, which is the only thing the notice cannot do
               for itself. */
            <div className="sync-review-banners">
              <NoticeZone
                items={banners}
                subject="notes about this Push review"
                {...bannerTiming}
              />
            </div>
          )}
          {activeStep === "history" && (
            <>
              {project && binding ? (
                <ProjectActivityPage
                  project={project}
                  binding={binding}
                  refreshEpoch={refreshEpoch}
                  activeStorageId={storageId}
                  activeStorageName={storageName}
                  selectionMode="push"
                  searchQuery={searchQuery}
                  onSearchQueryChange={setSearchQuery}
                  onSearchResultsChange={setSearchResults}
                  selectedResourceIds={effectiveSelected}
                  selectableResourceIds={selectableConversationIds}
                  selectionDisabled={busy}
                  onToggleResource={toggleResource}
                  onDropSelection={(resourceIds) => {
                    // Show less took these out of view. A tick you can no
                    // longer see is one you cannot check before it travels.
                    const next = new Set(effectiveSelected);
                    resourceIds.forEach((resourceId) => next.delete(resourceId));
                    commitSelection(next);
                  }}
                  onReviewPull={onPull}
                  continuityActionBusy={busy}
                  comparisonOverride={threadComparison}
                />
              ) : (
                <>
                  <NameSearchField query={searchQuery} onChange={setSearchQuery} label="Search sessions"
                    matches={matchingConversations.length} total={visibleConversations.length}
                    hiddenSelected={countSelected(visibleConversations, effectiveSelected) - countSelected(matchingConversations, effectiveSelected)} />
                  {searchActive && matchingConversations.length === 0 && <div className="sync-history-state">No matching sessions.</div>}
                  {(!searchActive || matchingConversations.length > 0) && (
                    <ResourceInventory resources={matchingConversations} selected={effectiveSelected} selectableResourceIds={selectableConversationIds} disabled={busy} forceExpanded={searchActive} onToggle={toggleResource} />
                  )}
                </>
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
              projectRoot={projectContentInventory?.project_root}
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
              onToggleWarning={onToggleProjectContentWarning}
              onReviewPull={onPull}
            />
          )}

        </div>

        {error && (
          /* The push the user just started failed. That is an event — the
             review rendered without it — and it stops the flow, so the box
             announces assertively. A push onto this version that did not
             finish offers the one verb that finishes it, and once it did the
             same box reads so in the done tone: the box prints its severity
             word, so a finished push is never an "Error". */
          <Notice
            severity={error === PUSH_FINISHED ? "success" : "danger"}
            timing="blocking"
            className="sync-pull-error"
            action={finishPushOffered(error, storageName) && onFinishPush
              ? (
                <button type="button" className="btn btn-ghost" onClick={onFinishPush} disabled={busy}>
                  <Icon name="upload" size={13} />
                  Finish that push
                </button>
              )
              : undefined}
          >
            {error}
          </Notice>
        )}
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {overLimitSentence && (
          /* The refusal takes a row of its own above the button cluster, where
             the gauge that caused it and the button it stops both sit. It does
             not announce: the gauge beside it is the status region, and its
             own text already changed to the overshoot. */
          <p className="sync-push-budget-sentence">{overLimitSentence}</p>
        )}
        <ProjectFileCommitImpact
          counts={projectFileImpact}
          deletionTarget="storage"
          deletionApproved={deletionApproved}
          disabled={busy}
          onDeletionApprovedChange={(approved) => onDeletionApprovedChange?.(approved)}
        />
        {localBlockerSentence && (
          /* Selection-driven: ticking a row that carries an uncaptured warning
             is what makes this appear, and it appears at the same moment the
             Push button silently goes disabled — so it announces, or the
             button's refusal has no explanation. Harmless in the case where
             the recommended selection already holds one, because a polite
             region does not announce what is present at first render. */
          <span role="status"><small>{localBlockerSentence}</small></span>
        )}
        {busy && (
          /* Every exit is sealed while the transfer runs, on purpose: the
             backend has no safe mid-write abort. Saying so beats leaving the
             user hunting for a cancel that does not exist. The sentence states
             a standing contract rather than reporting an event — the event is
             the push, which the button beside it is already narrating. It
             still announces: it appears only once `busy` flips, and it is the
             only thing that explains why Escape has stopped closing the review
             and the button under the reader's focus has gone dead. */
          <span role="status"><small>A running push finishes or fails whole — it can’t be cancelled midway.</small></span>
        )}
        {showProjectFiles && (
          /* The step navigator, not a gate: Push stays live on both steps.
             It reads forward on the sessions step and back on the files one,
             so the footer is always "keep reviewing, or publish now". */
          <button
            type="button"
            className="btn btn-ghost"
            disabled={busy}
            title={activeStep === "history" ? "Review project files next" : "Back to session selection"}
            onClick={() => selectStep(activeStep === "history" ? "project_files" : "history")}
          >
            {activeStep !== "history" && <Icon name="chevron-left" size={14} />}
            {activeStep === "history" ? "Project files" : historyStepLabel(showProjectFiles)}
            {activeStep === "history" && <Icon name="chevron-right" size={14} />}
          </button>
        )}
        {budgetTotal && <PushBudgetGauge total={budgetTotal} />}
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
