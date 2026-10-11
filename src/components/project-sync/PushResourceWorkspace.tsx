import { useEffect, useMemo, useRef, useState } from "react";
import type {
  BundleFetchProgress,
  GitHistoryOutgoing,
  GitHistorySnapshot,
  LocalProjectSummary,
  ProjectBinding,
  ProjectContentEntry,
  ProjectContentInventory,
  InventoryResource,
  ThreadSyncComparison,
  ThreadSyncEntry,
} from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import ProjectActivityPage, { type PushGitHistory, type ReviewListState } from "./ProjectActivityPage";
import ProjectFilesReviewPage, {
  projectFileReviewCards,
  type ProjectFileReveal,
  type ProjectFileReviewRow,
} from "./ProjectFilesReviewPage";
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
import { STORAGE_BLOCKING_STATES, projectLabel, sessionNoun } from "./model";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { isPushReviewDecision } from "../sync-review/pullReviewStatus";
import { ReviewBudgetGauge, ReviewBudgetGlance, reviewBudgetSentence, type ReviewBudget } from "../sync-review/ReviewBudgetMeter";
import { formatBytes } from "../../format";
import { t, useTranslation } from "../../i18n";

export interface PushReviewBlocker {
  kind: "storage" | "local";
  step: SyncReviewStep;
  resourceId: string;
  state: string;
}

/** A selected file whose scanner finding has not been marked reviewed. */
const WARNING_UNACKNOWLEDGED = "warning_unacknowledged";

interface Props {
  credentialRedactionEnabled: boolean;
  /** A failed session action in the review's session list, or null to clear it;
      the shell draws it under the path bar. */
  onSessionActionError: (message: string | null) => void;

  gitHistory?: GitHistorySnapshot | null;
  gitHistoryChanged?: boolean;
  gitHistoryOutgoing?: GitHistoryOutgoing | null;
  resources: InventoryResource[];
  selected: Set<string>;
  projectDefaults: Set<string>;
  busy: boolean;
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
  /** The Files step was on screen with its scanned rows: a new file left
   *  unticked from then on is one the push remembers as left out. */
  projectFilesShown?: boolean;
  projectContentRemovals?: ReadonlySet<string>;
  acknowledgedWarningDigests?: ReadonlySet<string>;
  /** Owned above the panel: the path bar's Push publishes this same review,
   *  so its refusal has to see the acknowledgement too. */
  deletionApproved?: boolean;
  onDeletionApprovedChange?: (approved: boolean) => void;
  /** Owned above the panel for the same reason: the path bar's Push waits
   *  for the session list exactly as the footer's does. */
  listState?: ReviewListState;
  onListStateChange?: (state: ReviewListState) => void;
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

/** The files the Push review lists: the ones this computer would publish, and
 *  an unresolved one only this computer holds. A selected `unknown` file stops
 *  the push and excluding it is the only way past, so its row has to be on
 *  screen to untick. One that storage also holds waits on a Pull whatever the
 *  selection, and the banner speaks for it. */
function isPushReviewProjectContent(entry: ProjectContentEntry): boolean {
  if (!entry.local_present) return false;
  if (entry.state === "unknown") return !entry.storage_present;
  return entry.state === "local_only" || entry.state === "local_ahead";
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
  // A storage without a name is "this storage", inside each sentence.
  const storage = storageName?.trim() || null;
  // The three fields stack as one banner: title, one sentence, and the lineage
  // figures abbreviated to what support actually reads off a screenshot. The
  // footer button says what to do, so the sentence only has to say what is
  // there and what the review unlocks.
  const detail = baseRevision != null
    ? t("reviews:pullRequired.detailReviewed", { remote: remoteRevision, base: baseRevision })
    : t("reviews:pullRequired.detailRequired", { remote: remoteRevision });
  if (reason === "remote_advanced") {
    const count = changedConversationCount;
    return {
      title: t("reviews:pullRequired.advancedTitle"),
      body: count > 0
        ? (storage
          ? t("reviews:pullRequired.advancedConversations", { count, storage })
          : t("reviews:pullRequired.advancedConversationsUnnamed", { count }))
        : (storage
          ? t("reviews:pullRequired.advanced", { storage })
          : t("reviews:pullRequired.advancedUnnamed")),
      detail,
    };
  }
  if (reason === "remote_without_base") {
    return {
      title: t("reviews:pullRequired.withoutBaseTitle"),
      body: storage
        ? t("reviews:pullRequired.withoutBase", { storage })
        : t("reviews:pullRequired.withoutBaseUnnamed"),
      detail,
    };
  }
  return {
    title: t("reviews:pullRequired.bindingChangedTitle"),
    body: storage
      ? t("reviews:pullRequired.bindingChanged", { storage })
      : t("reviews:pullRequired.bindingChangedUnnamed"),
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
  projectFilesShown = false,
): PushReviewBlocker[] {
  const blockers: PushReviewBlocker[] = [];
  const capturesProjectFiles = !!projectContentInventory
    && pushProjectFileWork(projectContentInventory, selected, projectContentRemovals, projectFilesShown) !== null;
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
    if (capturesProjectFiles && selectedEntry && entry.local_present
      && entry.warning_digest && !acknowledgedWarningDigests.has(entry.warning_digest)) {
      blockers.push({
        kind: "local",
        step: "project_files",
        resourceId: entry.descriptor.resource_id,
        state: WARNING_UNACKNOWLEDGED,
      });
    }
  }
  return blockers;
}

/** The sentence about one flagged file, by the scanner's finding: by its
 *  file name, or as "a selected file" when the scan no longer lists it. */
function projectFileWarningSentence(code: string | null | undefined, name: string | null): string {
  if (code === "executable") {
    return name === null ? t("reviews:blockers.executableUnnamed") : t("reviews:blockers.executable", { name });
  }
  if (code === "possible_secret") {
    return name === null ? t("reviews:blockers.secretUnnamed") : t("reviews:blockers.secret", { name });
  }
  return name === null ? t("reviews:blockers.warningUnnamed") : t("reviews:blockers.warning", { name });
}

/** One blocking item by the name its row shows: a file by its file name, a
 *  session by its title; null when the review no longer lists it. */
function blockerName(
  blocker: PushReviewBlocker,
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): string | null {
  if (blocker.step === "history") {
    return threadComparison?.entries.find((candidate) => candidate.resource_id === blocker.resourceId)?.display_name ?? null;
  }
  const entry = projectContentInventory?.entries.find((candidate) => candidate.descriptor.resource_id === blocker.resourceId);
  return entry ? entry.relative_path.split("/").pop()! : null;
}

/** The sentence about one item the capture cannot read. */
function unreadableSentence(
  blocker: PushReviewBlocker,
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): string {
  const name = blockerName(blocker, threadComparison, projectContentInventory);
  if (blocker.step === "history") {
    return name === null ? t("reviews:blockers.unreadableUnnamedSession") : t("reviews:blockers.unreadableSession", { name });
  }
  return name === null ? t("reviews:blockers.unreadableUnnamedFile") : t("reviews:blockers.unreadable", { name });
}

/**
 * Why the selected items stop the push, worded by the fix. A scanner finding
 * is cleared by Mark reviewed on its card; an item the capture cannot read
 * only by excluding it. One item is named, several are counted. The footer's
 * pill, its status line and both Push buttons' titles read this one sentence,
 * so they cannot disagree. Storage holds are not here: they are the banner's.
 */
export function pushBlockerSentence(
  blockers: readonly PushReviewBlocker[],
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
): string | null {
  const local = blockers.filter((blocker) => blocker.kind !== "storage");
  const flagged = local.filter((blocker) => blocker.state === WARNING_UNACKNOWLEDGED);
  const unreadable = local.filter((blocker) => blocker.state !== WARNING_UNACKNOWLEDGED);
  let flaggedSentence: string | null = null;
  if (flagged.length === 1) {
    const entry = projectContentInventory?.entries.find((candidate) => (
      candidate.descriptor.resource_id === flagged[0].resourceId
    ));
    flaggedSentence = projectFileWarningSentence(entry?.warning_code, blockerName(flagged[0], threadComparison, projectContentInventory));
  } else if (flagged.length > 1) {
    flaggedSentence = t("reviews:blockers.flaggedMany", { count: flagged.length });
  }
  let captureSentence: string | null = null;
  if (unreadable.length === 1) {
    captureSentence = unreadableSentence(unreadable[0], threadComparison, projectContentInventory);
  } else if (unreadable.length > 1) {
    captureSentence = t("reviews:blockers.unreadableMany", { count: unreadable.length });
  }
  if (flaggedSentence && captureSentence) {
    return t("reviews:blockers.both", { flagged: flaggedSentence, unreadable: captureSentence });
  }
  return flaggedSentence ?? captureSentence;
}

/** The blocking row the footer's pill opens. A blocker whose row the review
 *  lists wins over one it does not, and a folder its selected files require
 *  waits behind them: its card has no tick until they are excluded. */
export function pushBlockerRevealTarget(
  blockers: readonly PushReviewBlocker[],
  listedSessionIds: ReadonlySet<string>,
  listedProjectFileIds: ReadonlySet<string>,
  requiredProjectDirectoryIds: ReadonlySet<string>,
): PushReviewBlocker | null {
  const local = blockers.filter((blocker) => blocker.kind !== "storage");
  return local.find((blocker) => (
    blocker.step === "history"
      ? listedSessionIds.has(blocker.resourceId)
      : listedProjectFileIds.has(blocker.resourceId) && !requiredProjectDirectoryIds.has(blocker.resourceId)
  )) ?? local[0] ?? null;
}

/** The pill's short form of the same blockers: a count and what it needs. */
export function pushBlockerPillLabel(blockers: readonly PushReviewBlocker[]): string | null {
  const local = blockers.filter((blocker) => blocker.kind !== "storage");
  if (local.length === 0) return null;
  const flagged = local.filter((blocker) => blocker.state === WARNING_UNACKNOWLEDGED).length;
  if (flagged === local.length) return t("reviews:blockers.needsReview", { count: flagged });
  if (flagged === 0) return t("reviews:blockers.unavailable", { count: local.length });
  return t("reviews:blockers.needAttention", { count: local.length });
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
  const room = fixedBudget ? fixedBudget.limit - fixedBudget.total : Infinity;
  let remainingBytes = room;
  for (const entry of conversations) {
    if (!selectableIds.has(entry.resource_id)) continue;
    const bytes = entry.capture_bytes ?? 0;
    if (bytes > remainingBytes) {
      // A strict time prefix, not a first fit: the walk ends at the first
      // session that does not fit, so the ticked rows are one block at the
      // top of the list with nothing older slipped in under them. The one
      // exception is a session that could not fit an empty push — skipped,
      // or the default would stall at zero behind a row the meter refuses
      // anyway.
      if (bytes > room) continue;
      break;
    }
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

/** What Select applies on the Files step: what the recipe holds, and every
 *  new entry the scan found nothing against. A new folder is offered by
 *  itself only when it is empty; one that holds entries rides the
 *  required-ancestor rule of whatever is ticked inside it, so a folder whose
 *  only file is flagged does not travel empty. */
export function projectFileSelectOffer(inventory: ProjectContentInventory): Set<string> {
  const holdsEntries = new Set<string>();
  for (const entry of inventory.entries) {
    const parts = entry.relative_path.split("/");
    for (let end = 1; end < parts.length; end += 1) holdsEntries.add(parts.slice(0, end).join("/"));
  }
  return new Set(inventory.entries
    .filter((entry) => entry.selected_in_recipe || (
      entry.undecided
      && !entry.warning_code
      && (entry.entry_type !== "directory" || !holdsEntries.has(entry.relative_path))
    ))
    .map((entry) => entry.descriptor.resource_id));
}

/** What a Push has to do with the project folder.
 *
 *  `send`: it changes what storage or the recipe holds — a removal, a tick
 *  that changes the recipe, or a selected file that differs here from the
 *  stored one. `remember`: nothing to send, but a new file was left unticked
 *  on a list the user saw, and the push records it as left out. Null: no
 *  file work.
 *
 *  Only a push with file work sends the scan's review token. Without it the
 *  backend keeps the stored files as they are and does not walk the folder,
 *  so a file an agent is still writing cannot refuse a push of sessions; with
 *  it the backend captures every selected file, changed or not. */
export function pushProjectFileWork(
  inventory: ProjectContentInventory,
  selected: ReadonlySet<string>,
  removals: ReadonlySet<string>,
  filesShown: boolean,
): "send" | "remember" | null {
  if (inventory.eligibility.state !== "eligible") return null;
  if (removals.size > 0) return "send";
  let remembers = false;
  for (const entry of inventory.entries) {
    const picked = selected.has(entry.descriptor.resource_id);
    // The backend re-adds a recipe file storage still holds, so leaving one
    // unticked changes nothing; every other difference changes the recipe.
    if (picked !== entry.selected_in_recipe && !(entry.selected_in_recipe && entry.storage_present)) return "send";
    if (picked && entry.local_present && entry.state !== "synced") return "send";
    if (filesShown && entry.undecided && !picked) remembers = true;
  }
  return remembers ? "remember" : null;
}

/** The file decisions a push carries beside its selection. */
export interface PushFileReview {
  removals?: ReadonlySet<string>;
  /** The Files step was on screen with its scanned rows. */
  filesShown?: boolean;
}

/** What the current selection would put into one push: the selected sessions
 *  by their capture bytes, the requirements the push always carries, and the
 *  selected project files by size — every one of them when the push has file
 *  work, since the backend then captures them all, and none when it has not.
 *  Null when the review carries no budget (a review prepared without a
 *  comparison, or a Pull). */
export function pushBudgetTotal(
  selected: ReadonlySet<string>,
  threadComparison?: ThreadSyncComparison | null,
  projectContentInventory?: ProjectContentInventory | null,
  { removals = new Set<string>(), filesShown = false }: PushFileReview = {},
): { sessions: number; reserved: number; projectFiles: number; total: number; limit: number } | null {
  const budget = threadComparison?.capture_budget;
  if (!budget) return null;
  const sessions = (threadComparison?.entries ?? [])
    .filter((entry) => entry.local_present && selected.has(entry.resource_id))
    .reduce((sum, entry) => sum + (entry.capture_bytes ?? 0), 0);
  const capturesFiles = !!projectContentInventory
    && pushProjectFileWork(projectContentInventory, selected, removals, filesShown) !== null;
  const projectFiles = capturesFiles
    ? projectContentInventory.entries
      .filter((entry) => entry.local_present && selected.has(entry.descriptor.resource_id))
      .reduce((sum, entry) => sum + (entry.size ?? 0), 0)
    : 0;
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
  projectFilesShown = false,
  listState = "ready",
  gitHistoryChanged = false,
  sessionScope = false,
}: {
  /** Sanitized: `sanitizePushSelection`, which is what the push carries. */
  selected: ReadonlySet<string>;
  threadComparison?: ThreadSyncComparison | null;
  projectContentInventory?: ProjectContentInventory | null;
  acknowledgedWarningDigests?: ReadonlySet<string>;
  projectContentRemovals?: ReadonlySet<string>;
  deletionApproved: boolean;
  projectContentLoading?: boolean;
  /** The Files step was on screen with its scanned rows. */
  projectFilesShown?: boolean;
  /** The session list's state: a Push waits until its rows are on screen,
   *  so nothing ticked by default travels unseen. */
  listState?: ReviewListState;
  gitHistoryChanged?: boolean;
  /** One session's own review: over the limit, it has nothing to exclude. */
  sessionScope?: boolean;
}): string | null {
  /* First: until the rows are on screen, or while the scan is running,
     every judgement below is provisional, including the size of the push. */
  if (listState === "loading") return t("reviews:review.loadingSessions");
  // The list itself carries the error and its Try again; the button only points at it.
  if (listState === "failed") return t("projects:review.listFailed");
  if (projectContentLoading) return t("reviews:push.filesScanning");
  const budget = pushBudgetTotal(selected, threadComparison, projectContentInventory, {
    removals: projectContentRemovals,
    filesShown: projectFilesShown,
  });
  if (budget && budget.total > budget.limit) {
    return sessionScope ? sessionOverLimitSentence(budget) : reviewBudgetSentence(budget, "push");
  }
  const blockers = pushReviewBlockers(
    selected,
    threadComparison,
    projectContentInventory,
    acknowledgedWarningDigests,
    projectContentRemovals,
    projectFilesShown,
  );
  const blockerSentence = pushBlockerSentence(blockers, threadComparison, projectContentInventory);
  if (blockerSentence) return blockerSentence;
  if (blockers.length > 0) return t("reviews:push.resolveHeld");
  const impact = pushProjectFileCommitImpact(
    projectContentInventory,
    selected,
    projectContentRemovals,
  );
  if (projectFileDeletionCount(impact) > 0 && !deletionApproved) {
    return t("reviews:push.approveDeletions");
  }
  /* A scan ticks the files the recipe already holds, so a selection can be
     full of files storage has as they are. Those send nothing. */
  const fileIds = new Set((projectContentInventory?.entries ?? []).map((entry) => entry.descriptor.resource_id));
  const sendsFiles = !!projectContentInventory
    && pushProjectFileWork(projectContentInventory, selected, projectContentRemovals, projectFilesShown) === "send";
  if (
    ![...selected].some((resourceId) => !fileIds.has(resourceId))
    && !sendsFiles
    && projectContentRemovals.size === 0
    && !gitHistoryChanged
  ) return t("reviews:review.nothingSelected");
  return null;
}

/** The meter's refusal without its way out: a session's own review cannot
 *  exclude anything, so the sentence stops at the overshoot. */
function sessionOverLimitSentence(budget: ReviewBudget): string {
  return t("reviews:push.overLimit", { over: formatBytes(budget.total - budget.limit), limit: formatBytes(budget.limit) });
}

type PushBudget = NonNullable<ReturnType<typeof pushBudgetTotal>>;

/** The meter's title on both instances: the sources the total is made of.
 *  The meter itself is the shared review primitive; only this wording is
 *  Push's, because only Push carries requirements. */
function pushBudgetBreakdown(total: PushBudget): string {
  return t("reviews:push.breakdown", {
    sessions: formatBytes(total.sessions),
    requirements: formatBytes(total.reserved),
    projectFiles: formatBytes(total.projectFiles),
  });
}

/** The commit of both reviews that push. It wears the wait while the rows
 *  load and the folder is walked, and the transfer's own readout while it
 *  runs. */
export function PushApplyButton({ busy, listPending = false, filesPending = false, fetchProgress = null, disabledReason, onPush }: {
  busy: boolean;
  listPending?: boolean;
  /** The project folder is still being walked. */
  filesPending?: boolean;
  fetchProgress?: BundleFetchProgress | null;
  /** `pushReviewDisabledReason`, which is also the button's title. */
  disabledReason: string | null;
  onPush: () => void;
}) {
  const { t } = useTranslation(["reviews", "common"]);
  return (
    <button
      type="button"
      className="btn btn-primary sync-pull-apply-button"
      onClick={onPush}
      disabled={busy || disabledReason !== null}
      title={disabledReason ?? undefined}
    >
      {busy || listPending || filesPending
        ? <Icon name="refresh" size={16} className="icon-spin" />
        : <ThreadStatusIcon status="local-outstanding" size={16} />}
      {busy
        ? (fetchProgress && fetchProgress.phase === "push"
          ? <span className="sync-btn-readout">{`${fetchProgress.index}/${fetchProgress.total_objects} · ${(fetchProgress.fetched_bytes / 1_000_000).toFixed(1)} MB…`}</span>
          : t("push.pushing"))
        : listPending ? t("review.loadingSessions") : filesPending ? t("push.scanningFiles") : t("common:push")}
    </button>
  );
}

export default function PushResourceWorkspace({
  credentialRedactionEnabled,
  onSessionActionError,
  gitHistory,
  gitHistoryChanged = false,
  gitHistoryOutgoing = null,
  resources,
  selected,
  projectDefaults,
  busy,
  threadComparisonError = null,
  project,
  binding = null,
  storageId = null,
  storageName: storageNameProp,
  refreshEpoch = 0,
  threadComparison,
  projectContentInventory = null,
  projectContentScanned = false,
  projectContentLoading = false,
  projectFilesShown = false,
  projectContentRemovals = new Set(),
  acknowledgedWarningDigests = new Set(),
  deletionApproved = false,
  onDeletionApprovedChange,
  listState = "ready",
  onListStateChange,
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
}: Props) {
  const { t } = useTranslation("reviews");
  // An absent name reads as the word; a null one stays null.
  const storageName = storageNameProp === undefined ? t("review.defaultStorage") : storageNameProp;
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<NameSearchResults | null>(null);
  useEffect(() => { setSearchQuery(""); setSearchResults(null); }, [project?.local_project_id, storageId]);
  const steps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const [uncontrolledActiveStep, setUncontrolledActiveStep] = useState<SyncReviewStep>("history");
  const requestedActiveStep = controlledActiveStep ?? uncontrolledActiveStep;
  const activeStep = steps.includes(requestedActiveStep) ? requestedActiveStep : "history";
  /* A card the blocker pill asked the files page to open. Switching steps by
     hand drops it, so coming back to the files tab does not open it again. */
  const [projectFileReveal, setProjectFileReveal] = useState<ProjectFileReveal | null>(null);
  const selectStep = (step: SyncReviewStep) => {
    if (controlledActiveStep === undefined) setUncontrolledActiveStep(step);
    setProjectFileReveal(null);
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
  /* The Git history travels on the session rail itself: the commits storage
     lacks are the snapshot's leading ones, and the rail marks them ↑. */
  const pushGitHistory = useMemo<PushGitHistory | undefined>(() => gitHistory ? {
    outgoing: new Set(gitHistory.commits.slice(0, gitHistoryOutgoing?.new_commits ?? 0).map((commit) => commit.sha)),
    rewritten: gitHistoryOutgoing?.rewritten ?? false,
    changed: gitHistoryChanged,
  } : undefined, [gitHistory, gitHistoryOutgoing, gitHistoryChanged]);
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
  const selectableIds = useMemo(
    () => pushSelectableResourceIds(resources, threadComparison, projectContentInventory),
    [projectContentInventory, resources, threadComparison],
  );
  const effectiveSelected = useMemo(
    () => new Set([...selected].filter((resourceId) => selectableIds.has(resourceId))),
    [selectableIds, selected],
  );
  const capturesProjectFiles = !!projectContentInventory
    && pushProjectFileWork(projectContentInventory, effectiveSelected, projectContentRemovals, projectFilesShown) !== null;
  const visibleProjectContentEntries = useMemo(
    () => (projectContentInventory?.entries ?? []).filter((entry) => (
      isPushReviewProjectContent(entry)
      // File work captures selected unchanged files too. Keep their warning
      // controls visible, including after the user marks them reviewed.
      || (capturesProjectFiles && entry.local_present && entry.state === "synced"
        && entry.warning_digest && effectiveSelected.has(entry.descriptor.resource_id)
        && !projectContentRemovals.has(entry.descriptor.resource_id))
    )),
    [capturesProjectFiles, effectiveSelected, projectContentInventory, projectContentRemovals],
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
    projectFilesShown,
  );
  const pullHolds = pushReviewPullHolds(threadComparison);
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
      title: t("push.heldConversations", { count: heldConversationCount }),
      message: t("push.heldConversationsDetail"),
      action: onPull
        ? (
          <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>{t("review.openPull")}</button>
        )
        : undefined,
    });
  }
  if (heldProjectFileCount > 0) {
    banners.push({
      key: "project-file-pull-holds",
      severity: "warning",
      mark: <ThreadStatusIcon status="cloud-outstanding" size={16} />,
      title: t("push.heldFiles", { count: heldProjectFileCount }),
      message: t("push.heldFilesDetail"),
      action: onPull
        ? (
          <button type="button" className="btn btn-ghost" onClick={onPull} disabled={busy}>{t("review.openPull")}</button>
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
  if (threadComparisonError) {
    banners.push({
      key: "comparison-failed",
      severity: "danger",
      title: t("push.comparisonFailed"),
      message: threadComparisonError,
    });
  }
  /* Plan-derived is not the same as present-at-open. Project-file holds come
     from an inventory that can be null until the scan starts, so the zone can
     appear after the review opens. Standing only if it was there at open. */
  const bannersAtOpen = useRef<boolean | null>(null);
  if (bannersAtOpen.current === null) bannersAtOpen.current = banners.length > 0;
  const bannerTiming: NoticeTiming = bannersAtOpen.current
    ? { timing: "standing", label: t("push.notes") }
    : { timing: "progress" };
  // Selection-driven, so it reports in the footer rather than the banner zone
  // above the list — see the banner comment.
  const blockerSentence = pushBlockerSentence(blockers, threadComparison, projectContentInventory);
  const blockerPillLabel = pushBlockerPillLabel(blockers);
  /* The pill opens the first blocking row: its tab, and on the files step the
     card itself, which the page scrolls to and hands focus. */
  const revealFirstBlocker = () => {
    const target = pushBlockerRevealTarget(
      blockers,
      visibleConversationResourceIds,
      visibleProjectFileResourceIds,
      requiredProjectContentDirectoryIds(projectContentInventory, effectiveSelected),
    );
    if (!target) return;
    selectStep(target.step);
    if (target.step === "project_files") {
      setProjectFileReveal({ resourceId: target.resourceId });
    }
  };
  // Every eligible session remains selectable. Only the aggregate byte
  // ceiling can refuse Apply.
  const budgetTotal = pushBudgetTotal(effectiveSelected, threadComparison, projectContentInventory, {
    removals: projectContentRemovals,
    filesShown: projectFilesShown,
  });
  const overLimitSentence = budgetTotal && budgetTotal.total > budgetTotal.limit
    ? reviewBudgetSentence(budgetTotal, "push")
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
    projectFilesShown,
    listState,
    gitHistoryChanged,
  });
  /* The meters and the count measure rows; until the rows land they would
     measure a default selection nobody has seen. */
  const listPending = listState === "loading";
  /* Nothing to push, so the footer's one button leaves instead. Not while a
     file scan runs: its files may still land, and the footer is the loading
     one until they do. A scan that failed does not hold it back — it lands
     nothing, so a review waiting on it would never offer the way out. */
  const emptyReview = listState === "ready"
    && !projectContentLoading
    && visibleResources.length === 0
    && projectContentRemovals.size === 0
    && !gitHistoryChanged;
  // The page the review closes onto, as the path bar names it.
  const exitTarget = project?.is_git_repository ? t("push.exitGitHistory") : project ? projectLabel(project) : null;
  /* The review says it is ready once, when its rows land — after mount, so a
     review ready from its first render still changes the region. */
  const [readyAnnounced, setReadyAnnounced] = useState(false);
  useEffect(() => {
    if (listState === "ready") setReadyAnnounced(true);
  }, [listState]);
  // The placeholders take the count of sessions the list will land.
  const reviewScaffold = useMemo(
    () => ({ count: visibleConversations.length, storageOnly: 0 }),
    [visibleConversations.length],
  );

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
    undecided: entry.undecided,
    blockedReason: entry.blocked_reason,
    warningCode: entry.warning_code,
    warningDigest: entry.warning_digest,
  }));
  /* The files that wait: new to this storage, never left out, and not
     ticked. A Push leaves them here, so the Sessions step counts them on the
     button that opens their list. */
  const waitingProjectFiles = projectFileReviewCards(projectFileRows)
    .filter((row) => row.undecided && !effectiveSelected.has(row.resourceId))
    .length;
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
          <h2 id="sync-push-resource-title">{t("push.title")}</h2>
          <span className="sync-settings-flag">{credentialRedactionEnabled ? t("push.redactionOn") : t("push.redactionOff")}</span>
          <span className="sync-visually-hidden" role="status">
            {readyAnnounced ? t("push.ready") : ""}
          </span>
        </div>
        {/* An empty review draws no meter: it would measure only the
            reserved requirements of a push that cannot happen. */}
        {budgetTotal && !listPending && !emptyReview && (
          // i18n-ignore: the meter's mode key, not copy
          <ReviewBudgetGlance budget={budgetTotal} verb="push" breakdown={pushBudgetBreakdown(budgetTotal)} />
        )}
        <SyncReviewSelectionBar
          count={countSelected(currentGroup, effectiveSelected)}
          total={currentGroup.length}
          busy={busy}
          listPending={listPending}
          recommendedTitle={searchActive
            ? t("push.recommendMatching", { count: recommendedCount })
            : activeStep === "history"
              ? t("push.recommendSessions", { count: recommendedCount })
              : t("push.recommendFiles", { count: recommendedCount })}
          recommendedCount={recommendedCount}
          recommendedDisabled={currentDefaults.length === 0 || currentMatchesDefaults}
          onRecommended={useRecommendedForStep}
          searchActive={searchActive}
          clearDisabled={countSelected(searchGroup, effectiveSelected) === 0}
          onClear={clearStep}
          close={{ ariaLabel: t("push.close"), onClose }}
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
                subject={t("push.notesSubject")}
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
                  pushGitHistory={pushGitHistory}
                  searchQuery={searchQuery}
                  onSearchQueryChange={setSearchQuery}
                  onSearchResultsChange={setSearchResults}
                  selectedResourceIds={effectiveSelected}
                  selectableResourceIds={selectableConversationIds}
                  selectionDisabled={busy}
                  onToggleResource={toggleResource}
                  onActionError={onSessionActionError}
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
                  onReviewListStateChange={onListStateChange}
                  reviewScaffold={reviewScaffold}
                />
              ) : (
                <>
                  <NameSearchField query={searchQuery} onChange={setSearchQuery} label={t("review.searchSessions")}
                    matches={matchingConversations.length} total={visibleConversations.length} noun={sessionNoun}
                    hiddenSelected={countSelected(visibleConversations, effectiveSelected) - countSelected(matchingConversations, effectiveSelected)} />
                  {searchActive && matchingConversations.length === 0 && <div className="sync-history-state">{t("review.noMatches")}</div>}
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
                  ? t("push.gitManaged")
                  : t("push.notGit"),
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
              reveal={projectFileReveal}
            />
          )}

        </div>
      </div>

      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {overLimitSentence && (
          /* The refusal takes a row of its own above the button cluster, where
             the gauge that caused it and the button it stops both sit. It does
             not announce: the gauge beside it is the status region, and its
             own text already changed to the overshoot. */
          <p className="sync-review-footer-note danger">{overLimitSentence}</p>
        )}
        <ProjectFileCommitImpact
          counts={projectFileImpact}
          deletionTarget="storage"
          deletionApproved={deletionApproved}
          disabled={busy}
          onDeletionApprovedChange={(approved) => onDeletionApprovedChange?.(approved)}
        />
        {/* Selection-driven: ticking a row that carries an unreviewed finding
            is what makes the blocker pill appear, at the same moment Push goes
            disabled — so its sentence announces here, or the button's refusal
            has no explanation. The region stays mounted, so the change is what
            it reads; a blocker present at first render is not announced. */}
        <span className="sync-visually-hidden" role="status">{blockerSentence ?? ""}</span>
        {emptyReview ? (
          /* Nothing to commit, so the commit's slot holds the way out — the
             way Settings' Done leaves a page with nothing to save. A box, not
             the gold (nothing is committed); it stands where the step
             navigator would, which has nothing to walk between here: both
             tabs read 0 and the strip above already switches. The path
             bar's Push stays, disabled, as the review's verb. */
          <button
            type="button"
            className="btn sync-review-exit"
            disabled={busy}
            title={exitTarget === null ? t("push.exitTitleUnnamed") : t("push.exitTitle", { target: exitTarget })}
            onClick={onClose}
          >
            <span>{exitTarget === null ? t("push.exitUnnamed") : t("push.exit", { target: exitTarget })}</span>
          </button>
        ) : (
          <>
            {showProjectFiles && (
              /* The step navigator, not a gate: Push stays live on both steps.
                 It reads forward on the sessions step and back on the files one,
                 so the footer is always "keep reviewing, or publish now". A
                 box, not a ghost: as a ghost its count of waiting files was
                 easy to push past. */
              <button
                type="button"
                className="btn"
                disabled={busy}
                title={activeStep !== "history"
                  ? t("review.backToSessions")
                  : waitingProjectFiles > 0
                    ? t("push.waitingFiles", { count: waitingProjectFiles })
                    : t("review.filesNext")}
                onClick={() => selectStep(activeStep === "history" ? "project_files" : "history")}
              >
                {activeStep !== "history" && <Icon name="chevron-left" size={14} />}
                {activeStep === "history" ? t("review.projectFiles") : historyStepLabel(showProjectFiles)}
                {activeStep === "history" && waitingProjectFiles > 0 && (
                  <span className="sync-path-count">{t("push.newCount", { count: waitingProjectFiles })}</span>
                )}
                {activeStep === "history" && <Icon name="chevron-right" size={14} />}
              </button>
            )}
            {blockerPillLabel && (
              /* What stops Push, beside the gauge and the button it disables:
                 a count and what it needs. The title carries the sentence,
                 and pressing it opens the first blocking row. */
              <button
                type="button"
                className="sync-review-blocker-pill"
                disabled={busy}
                title={blockerSentence ?? undefined}
                onClick={revealFirstBlocker}
              >
                <Icon name="alert-triangle" size={13} />
                {blockerPillLabel}
              </button>
            )}
            {budgetTotal && !listPending && (
              // i18n-ignore: the meter's mode key, not copy
              <ReviewBudgetGauge budget={budgetTotal} verb="push" breakdown={pushBudgetBreakdown(budgetTotal)} />
            )}
            <PushApplyButton
              busy={busy}
              listPending={listPending}
              filesPending={projectContentLoading}
              fetchProgress={fetchProgress}
              disabledReason={pushDisabledReason}
              onPush={onPush}
            />
          </>
        )}
      </footer>
    </section>
  );
}
