import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type {
  LocalProjectSummary,
  ProjectBinding,
  PullReview,
} from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem } from "../notice/Notice";
import ProjectActivityPage, { compactThreadTitle, type ReviewListState } from "./ProjectActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import ProjectFileCommitImpact, {
  type ProjectFileCommitImpactCounts,
} from "./ProjectFileCommitImpact";
import SyncReviewTabs from "../sync-review/SyncReviewTabs";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import NameSearchField from "../sync-review/NameSearchField";
import { nameMatchesSearch, type NameSearchResults } from "../sync-review/nameSearch";
import { type SyncReviewStep, historyStepLabel, projectReviewTabs, syncReviewSteps } from "./reviewSteps";
import { providerLabel, sessionNoun } from "./model";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { compareAgainOffered } from "./pullReviewFlow";
import type { PullApplyPhase, PullReviewSelection } from "./pullReviewFlow";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  fitPullRecommendation,
  itemKind,
  itemKindLabel,
  includeRequiredPullProjectDirectories,
  pullBudgetTotal,
  pullWaitingFiles,
  recommendedPullSelection,
  requiredPullProjectDirectoryIds,
  requiresApproval,
  restoreActionIds,
  type PullReviewItem,
} from "./pullReviewModel";
import { ReviewBudgetGauge, ReviewBudgetGlance, reviewBudgetSentence } from "../sync-review/ReviewBudgetMeter";
import { formatBytes } from "../../format";
import { useTranslation } from "../../i18n";

interface Props {
  projectName: string;
  /** A failed session action in the review's session list, or null to clear it;
      the shell draws it under the path bar. */
  onSessionActionError: (message: string | null) => void;

  project?: LocalProjectSummary;
  storageName?: string | null;
  plan: PullReview;
  binding: ProjectBinding;
  phase: PullApplyPhase;
  supportLoading: boolean;
  busy: boolean;
  error: string | null;
  /** Closes the failure box; the host clears `error`. */
  onDismissError: () => void;
  onApply: (selection: PullReviewSelection) => void;
  onBack: () => void;
  /** Rebuilds the plan against the storage's current head — the files
   *  page's Rescan. */
  onRefreshPlan?: () => void;
  /**
   * Mirrors the footer primary out to the host, so the storage-card Pull
   * button can apply the review — including the empty, acknowledge-only
   * apply — under exactly the footer's gating. Null once the review unmounts.
   */
  onPrimaryChange?: (primary: PullReviewPrimary | null) => void;
}

/** The review's primary action, as the host's Pull button sees it. */
export interface PullReviewPrimary {
  /** Why the apply must not run right now; null when it may. */
  disabledReason: string | null;
  /** The verb the button wears: "Pull N", or "Mark as reviewed" at zero,
   *  and "Loading sessions…" until the rows land. */
  label: string;
  /** The rows have not landed yet; the button shows the wait. */
  pending: boolean;
  /** Applies the review's current selection — the footer's own onClick. */
  apply: () => void;
}

export function pullSelectableResourceIds(items: PullReviewItem[]): Set<string> {
  return new Set(items.filter((item) => restoreActionIds(item).length > 0).map((item) => item.resourceId));
}

export function pullProjectFileCommitImpact(
  items: PullReviewItem[],
  selected: ReadonlySet<string>,
): ProjectFileCommitImpactCounts {
  const counts: ProjectFileCommitImpactCounts = {
    add: 0,
    replace: 0,
    deleteFile: 0,
    deleteDirectory: 0,
  };
  for (const item of items) {
    if (!selected.has(item.resourceId)) continue;
    switch (item.projectContentOperation) {
      case "add":
      case "create_directory":
        counts.add += 1;
        break;
      case "replace":
        counts.replace += 1;
        break;
      default:
        break;
    }
  }
  return counts;
}

/** The generic row's operation, in the files review's chip grammar. Rows
 * whose verbs live elsewhere (conversations, manual reviews) return null. */
function itemOperationChip(item: PullReviewItem): "replaceFile" | "addFile" | "mergeFile" | null {
  const action = item.restoreActions[0];
  if (!action) return null;
  if (action.kind.kind === "write_file") {
    return action.expected_target_sha256 ? "replaceFile" : "addFile";
  }
  if (action.kind.kind === "merge_file") return "mergeFile";
  return null;
}

/** A row's status chip: its words, and the class it has always worn. */
type ItemStatus = "restoring" | "keepBoth" | "needsApproval" | "notSelected";
const ITEM_STATUS_CLASS: Record<ItemStatus, string> = {
  restoring: "restoring",
  keepBoth: "keep-both",
  needsApproval: "needs-approval",
  notSelected: "not-selected",
};

function ItemList({
  items,
  selected,
  phase,
  disabled,
  onToggle,
}: {
  items: PullReviewItem[];
  selected: ReadonlySet<string>;
  phase: PullApplyPhase;
  disabled: boolean;
  onToggle: (resourceId: string) => void;
}) {
  const { t } = useTranslation("reviews");
  const [expandedItems, setExpandedItems] = useState<Set<string>>(() => new Set());
  const toggleDetails = (resourceId: string) => setExpandedItems((current) => {
    const next = new Set(current);
    if (next.has(resourceId)) next.delete(resourceId);
    else next.add(resourceId);
    return next;
  });

  return (
    <div className="sync-pull-items" role="list">
      {items.map((item) => {
        const selectable = restoreActionIds(item).length > 0;
        const checked = selected.has(item.resourceId);
        const expanded = expandedItems.has(item.resourceId);
        const activeRestore = phase === "restoring" && checked;
        const status: ItemStatus | null = activeRestore
          ? "restoring"
          : item.conversationReviewRows[0]?.action === "keep_both" && checked
            ? "keepBoth"
            : requiresApproval(item) && !checked
              ? "needsApproval"
              : checked ? null : "notSelected";
        const chip = itemOperationChip(item);
        return (
          <div
            key={item.resourceId}
            className={`sync-pull-item${selectable ? " sync-pull-item-selectable" : ""}${checked ? " selected" : ""}`}
            role="listitem"
          >
            {selectable && (
              <label className="sync-pull-item-select">
                <input
                  type="checkbox"
                  aria-label={t("item.select", { title: item.title })}
                  checked={checked}
                  disabled={disabled}
                  onChange={() => onToggle(item.resourceId)}
                />
              </label>
            )}
            <button
              type="button"
              className="sync-pull-item-main"
              onClick={() => toggleDetails(item.resourceId)}
              aria-expanded={expanded}
              aria-label={expanded ? t("item.hideDetails", { title: item.title }) : t("item.showDetails", { title: item.title })}
            >
              <span className="sync-pull-item-toggle">
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
              </span>
              <span className="sync-pull-item-icon">
                <Icon
                  name="file"
                  size={14}
                />
              </span>
              <span className="sync-pull-item-copy">
                <strong>{item.title}</strong>
                <span>
                  {itemKindLabel(item)}
                  {item.provider ? ` · ${providerLabel(item.provider)}` : ""}
                  {item.pullBytes != null ? ` · ${formatBytes(item.pullBytes)}` : ""}
                </span>
              </span>
              {chip && (
                <span className="sync-history-branch-state history-only" title={item.detail}>
                  {t(`item.${chip}`)}
                </span>
              )}
              {status && <span className={`sync-pull-item-status ${ITEM_STATUS_CLASS[status]}`}>{t(`item.${status}`)}</span>}
            </button>
            {expanded && (
              <div className="sync-pull-item-detail">
                <p>{item.detail}</p>
                <div className="sync-pull-item-detail-grid">
                  <span>{t("item.type")}</span><code>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</code>
                  <span>{t("item.resource")}</span><code>{item.resourceId}</code>
                  {item.restoreActions.map((action) => (
                    <Fragment key={action.action_id}><span>{t("item.restore")}</span><code>{action.action_id}</code></Fragment>
                  ))}
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export default function RestorePlanView({
  projectName,
  onSessionActionError,
  project,
  storageName: storageNameProp,
  plan,
  binding,
  phase,
  supportLoading,
  busy,
  error,
  onDismissError,
  onApply,
  onBack,
  onRefreshPlan,
  onPrimaryChange,
}: Props) {
  const { t, i18n } = useTranslation(["reviews", "common", "projects"]);
  // An absent name reads as the word; a null one stays null.
  const storageName = storageNameProp === undefined ? t("review.defaultStorage") : storageNameProp;
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<NameSearchResults | null>(null);
  useEffect(() => { setSearchQuery(""); setSearchResults(null); }, [plan.plan_id]);
  // The items carry their words, so a language switch rebuilds them.
  const items = useMemo(() => buildPullReviewItems(plan), [plan, i18n.language]);
  const conversationItems = useMemo(
    () => items.filter((item) => itemKind(item) === "conversation"),
    [items],
  );
  const projectContentItems = useMemo(
    () => items.filter((item) => item.category === "project_content"),
    [items],
  );
  const otherItems = useMemo(() => items.filter((item) => (
    itemKind(item) !== "conversation"
    && item.category !== "project_content"
  )), [items]);
  // A session's own Pull: the one row, ticked. Its plan holds no file
  // eligibility, so it has one step and the step strip is not drawn.
  const sessionScope = plan.scope.kind === "session";
  const showProjectFiles = plan.project_content_eligibility?.state === "eligible" || projectContentItems.length > 0;
  const reviewSteps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const [activeStep, setActiveStep] = useState<SyncReviewStep>("history");
  // A project's review opens with nothing ticked. A session's opens on the
  // row its link named, ticked; Pull is still the press that writes.
  const [selected, setSelected] = useState<Set<string>>(() => (
    sessionScope ? pullSelectableResourceIds(items) : new Set()
  ));
  const projectFileImpact = useMemo(
    () => pullProjectFileCommitImpact(projectContentItems, selected),
    [projectContentItems, selected],
  );
  useEffect(() => {
    const currentIds = new Set(items.map((item) => item.resourceId));
    setSelected((current) => {
      const next = new Set([...current].filter((resourceId) => currentIds.has(resourceId)));
      return includeRequiredPullProjectDirectories(items, next);
    });
  }, [items]);
  // Compare again hands the same review a new plan: its row is ticked anew.
  useEffect(() => {
    if (!sessionScope) return;
    setSelected(pullSelectableResourceIds(items));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan.plan_id]);
  const reviewResourceIds = useMemo(
    () => (sessionScope ? new Set(plan.conversation_review.map((row) => row.resource_id)) : undefined),
    [plan.conversation_review, sessionScope],
  );
  const sessionTitle = sessionScope && conversationItems[0]
    ? compactThreadTitle(conversationItems[0].threadTitle ?? "").trim() || t("review.untitledSession")
    : null;

  const selection = useMemo(() => buildPullReviewSelection(items, selected), [items, selected]);
  // What the selection would install against the limit the review carries.
  // Nothing is reserved: a Pull carries nothing it does not show as a row.
  const budget = useMemo(
    () => pullBudgetTotal(items, selected, plan.apply_limit_bytes),
    [items, plan.apply_limit_bytes, selected],
  );
  const budgetBreakdown = t("pull.breakdown", {
    sessions: formatBytes(budget.sessions),
    other: formatBytes(budget.other),
    projectFiles: formatBytes(budget.projectFiles),
  });
  const overLimitSentence = budget.total > budget.limit ? reviewBudgetSentence(budget, "pull") : null;
  const approvalItems = items.filter((item) => (
    item.category !== "project_content" && requiresApproval(item) && !selected.has(item.resourceId)
  ));

  useEffect(() => {
    if (!reviewSteps.includes(activeStep)) setActiveStep("history");
  }, [activeStep, reviewSteps]);

  // The Activity review renders the storage rows ("Only in …") from
  // its own async fetches; until it reports ready, the primary must not apply
  // a selection the user has not seen. The ItemList fallback is synchronous.
  const activityHistoryInUse = conversationItems.length > 0 && Boolean(project);
  const [listState, setListState] = useState<ReviewListState>("loading");
  const listPending = activityHistoryInUse && listState === "loading";
  const listFailed = activityHistoryInUse && listState === "failed";
  // What the placeholders draw while the list loads: the plan already knows
  // how many sessions will land, and which of them exist only in storage.
  const reviewScaffold = useMemo(() => ({
    count: conversationItems.length,
    storageOnly: conversationItems.filter((item) => (
      item.conversationReviewRows.some((row) => row.action === "create_local")
    )).length,
  }), [conversationItems]);
  // Loading first: while the list is not on screen every judgement below it
  // is provisional. Then a list that failed, then the limit, with the
  // meter's own sentence as the reason — the same rules the path bar's Pull
  // reads through the primary.
  const selectedPendingCount = items.filter((item) => (
    selected.has(item.resourceId) && restoreActionIds(item).length > 0
  )).length;
  // A session's Pull reviews one row of the storage: an empty apply would
  // mark nothing as reviewed, so with the row unticked there is nothing to do.
  const primaryDisabledReason = !busy && listPending
    ? t("review.loadingSessions")
    : !busy && listFailed
      ? t("projects:review.listFailed")
      : overLimitSentence ?? (sessionScope && selectedPendingCount === 0 ? t("review.nothingSelected") : null);
  // A project's empty apply still runs — it marks the storage as reviewed
  // and unblocks Push — so the verb must say so instead of promising a Pull
  // that restores nothing. With rows ticked the button carries their count.
  // Until the rows land the verb is the wait itself, on both buttons.
  const primaryLabel = listPending
    ? t("review.loadingSessions")
    : selectedPendingCount > 0
      ? t("pull.pullCount", { count: selectedPendingCount })
      : sessionScope ? t("common:pull") : t("pull.markReviewed");
  const primaryBusyLabel = selectedPendingCount > 0 ? t("pull.pulling") : t("pull.markingReviewed");
  /* The green "ready" message is gone from the shell, so the review says it
     itself, once, when its rows land. Set after mount so a review that is
     ready from its first render still changes the region and is heard. */
  const [readyAnnounced, setReadyAnnounced] = useState(false);
  useEffect(() => {
    if (!listPending && !listFailed) setReadyAnnounced(true);
  }, [listPending, listFailed]);

  // The storage-card Pull button is the same control as the footer primary,
  // so it must run the same apply under the same gating — including the
  // empty, acknowledge-only apply that only advances the reviewed baseline.
  // The refs keep the reported closure current without re-reporting on every
  // checkbox tick.
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const onApplyRef = useRef(onApply);
  onApplyRef.current = onApply;
  useEffect(() => {
    onPrimaryChange?.({
      disabledReason: primaryDisabledReason,
      label: primaryLabel,
      pending: listPending,
      apply: () => onApplyRef.current(selectionRef.current),
    });
  }, [primaryDisabledReason, primaryLabel, listPending, onPrimaryChange]);
  useEffect(() => () => onPrimaryChange?.(null), [onPrimaryChange]);

  const requiredProjectDirectories = requiredPullProjectDirectoryIds(items, selected);
  const projectContentEligible = plan.project_content_eligibility?.state === "eligible";
  const toggle = (resourceId: string) => setSelected((current) => {
    const item = items.find((candidate) => candidate.resourceId === resourceId);
    if (item?.category === "project_content") {
      if (!projectContentEligible || requiredPullProjectDirectoryIds(items, current).has(resourceId)) {
        return current;
      }
    }
    const next = new Set(current);
    if (next.has(resourceId)) next.delete(resourceId);
    else next.add(resourceId);
    return includeRequiredPullProjectDirectories(items, next);
  });
  // The history step hosts every non-file item: sessions in the list, agent
  // setup & tools in the details row at the bottom of the same panel.
  const stepItems = activeStep === "project_files"
    ? projectContentItems
    : [...conversationItems, ...otherItems];
  const searchActive = activeStep === "history" && searchQuery.trim().length > 0;
  const matchingConversations = conversationItems.filter((item) => nameMatchesSearch(item.threadTitle || t("pull.untitledThread"), searchQuery));
  const matchingIds = activityHistoryInUse
    ? (searchResults?.query === searchQuery ? searchResults.resourceIds : new Set<string>())
    : new Set(matchingConversations.map((item) => item.resourceId));
  const searchStepItems = searchActive
    ? conversationItems.filter((item) => matchingIds.has(item.resourceId))
    : stepItems;
  // The sessions step's ✓ is the newest safe sessions that fit this pull,
  // charging whatever the files step has ticked; the files step's ✓ stays
  // every safe addition, and the meter refuses if they do not fit.
  const recommendedIds = activeStep === "history"
    ? fitPullRecommendation(items, searchStepItems, selected, plan.apply_limit_bytes)
    : new Set(items.filter(recommendedPullSelection).map((item) => item.resourceId));
  const pendingSearchItems = searchStepItems.filter((item) => restoreActionIds(item).length > 0);
  const pendingStepItems = stepItems.filter((item) => (
    restoreActionIds(item).length > 0
  ));
  const pendingRecommendedCount = pendingSearchItems.filter((item) => recommendedIds.has(item.resourceId)).length;
  const stepMatchesRecommended = pendingSearchItems.every((item) => (
    selected.has(item.resourceId) === recommendedIds.has(item.resourceId)
  ));
  const useRecommended = () => setSelected((current) => {
    const next = new Set(current);
    for (const item of searchStepItems) {
      next.delete(item.resourceId);
      if (recommendedIds.has(item.resourceId)) next.add(item.resourceId);
    }
    return includeRequiredPullProjectDirectories(items, next);
  });
  const clearStep = () => setSelected((current) => {
    const next = new Set(current);
    for (const item of searchStepItems) next.delete(item.resourceId);
    return next;
  });
  const selectedIn = (chosen: PullReviewItem[]) => chosen.filter((item) => selected.has(item.resourceId)).length;
  const counts = {
    history: selectedIn(conversationItems) + selectedIn(otherItems),
    project_files: selectedIn(projectContentItems),
  };
  const warningSteps = new Set<SyncReviewStep>();
  for (const item of approvalItems) {
    if (itemKind(item) === "conversation") warningSteps.add("history");
  }
  if (projectContentItems.length > 0 && !projectContentEligible) {
    warningSteps.add("project_files");
  }
  const conversationIds = pullSelectableResourceIds(conversationItems);
  /* The banner zone that heads the review scroll holds the Pull failure. A
     Pull of a storage that was cleared and pushed again is an ordinary first
     Pull, so nothing here speaks about the storage's history. */
  const banners: NoticeItem[] = [];
  if (error) {
    banners.push({
      key: "pull-failed",
      severity: "danger",
      message: error,
      /* A review the storage moved past, or one older than a day, is
         rebuilt in place: the same control as Rescan on the files step. */
      action: compareAgainOffered(error) && onRefreshPlan ? (
        <button type="button" className="btn btn-ghost" onClick={onRefreshPlan} disabled={busy}>
          <Icon name="refresh" size={13} />
          {t("pull.compareAgain")}
        </button>
      ) : undefined,
      /* The zone draws the close control; closing clears the host's copy,
         and the review stays open under it. */
      onDismiss: onDismissError,
    });
  }
  /* The files that wait, counted on the step button that opens their list.
     A folder that cannot sync has no rows to tick, so nothing waits. */
  const waitingFiles = projectContentEligible
    ? pullWaitingFiles(projectContentItems, selected)
    : { added: 0, changed: 0 };
  const waitingFileCount = waitingFiles.added + waitingFiles.changed;
  const projectContentRows: ProjectFileReviewRow[] = projectContentItems
    .filter((item) => item.projectContentPath && item.projectContentEntryType)
    .map((item) => {
      const action = item.restoreActions[0];
      return {
        resourceId: item.resourceId,
        relativePath: item.projectContentPath!,
        entryType: item.projectContentEntryType!,
        state: item.projectContentOperation ?? "needs_review",
        mode: action && "mode" in action.kind ? action.kind.mode : action?.expected_target_mode,
        sourceMtime: action && "source_mtime" in action.kind ? action.kind.source_mtime : null,
        localPresent: Boolean(action?.expected_target_sha256 || action?.expected_target_mode != null),
        size: item.projectContentSize,
        storagePresent: true,
        operation: item.projectContentOperation ?? undefined,
        preservedCopyPath: item.projectContentPreservedCopy ?? undefined,
      };
    });
  return (
    <section
      className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace"
      aria-labelledby="sync-pull-review-title"
    >
      <header className="sync-inline-action-header sync-inline-pull-header sync-sync-review-header">
        <div className="sync-inline-action-heading">
          <div className="sync-sync-review-title">
            <h2 id="sync-pull-review-title">{sessionTitle ? t("pull.sessionTitle", { title: sessionTitle }) : t("pull.title")}</h2>
            <span className="sync-pull-review-meta">{t("common:version", { version: plan.revision })}</span>
          </div>
          <span className="sync-visually-hidden" role="status">
            {readyAnnounced ? t("pull.ready") : ""}
          </span>
        </div>
        {/* The meters and the count measure rows; until the rows land they
            would measure nothing anyone can see. The recommend button's own
            count is a property of the plan, not of the selection, so it never
            moves when a row is ticked. */}
        {/* i18n-ignore: the meter's mode key, not copy */}
        {!listPending && <ReviewBudgetGlance budget={budget} verb="pull" breakdown={budgetBreakdown} />}
        <SyncReviewSelectionBar
          count={selectedIn(pendingStepItems)}
          total={pendingStepItems.length}
          busy={busy}
          listPending={listPending}
          recommendedTitle={activeStep === "history"
            ? (searchActive
              ? t("pull.recommendNewestMatching", { count: pendingRecommendedCount })
              : t("pull.recommendNewest", { count: pendingRecommendedCount }))
            : t("pull.recommendSafe", { count: pendingRecommendedCount })}
          recommendedCount={pendingRecommendedCount}
          recommendedDisabled={pendingSearchItems.length === 0 || stepMatchesRecommended}
          onRecommended={useRecommended}
          searchActive={searchActive}
          clearDisabled={selectedIn(searchStepItems) === 0}
          onClear={clearStep}
          close={{ ariaLabel: t("pull.close", { project: projectName }), onClose: onBack }}
        />
      </header>

      <SyncReviewTabs
        tabs={projectReviewTabs(reviewSteps, counts, warningSteps, historyStepLabel(showProjectFiles))}
        activeId={activeStep}
        disabled={busy}
        onChange={(step) => setActiveStep(step as SyncReviewStep)}
      />

      <div
        id={`sync-review-${activeStep}-panel`}
        className="sync-pull-review-content sync-inline-action-content sync-sync-review-content"
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
              {/* The review always mounts with `error` null (the host clears it
                  before it plans, and even the supporting-check failure lands
                  after the plan is handed over), so an error that is present
                  arrived after an action and stops the flow. */}
              <NoticeZone items={banners} subject={t("pull.notesSubject")} timing="blocking" />
            </div>
          )}
          {activeStep === "history" && (
            <>
              {!activityHistoryInUse && conversationItems.length > 0 && (
                <>
                  <NameSearchField query={searchQuery} onChange={setSearchQuery} label={t("review.searchSessions")}
                    matches={matchingConversations.length} total={conversationItems.length} noun={sessionNoun}
                    hiddenSelected={selectedIn(conversationItems) - selectedIn(matchingConversations)} />
                  {searchActive && matchingConversations.length === 0 && <div className="sync-history-state">{t("review.noMatches")}</div>}
                </>
              )}
              {conversationItems.length > 0 && project ? (
                <ProjectActivityPage
                  project={project}
                  binding={binding}
                  refreshEpoch={0}
                  activeStorageId={plan.storage_id}
                  activeStorageName={storageName}
                  selectionMode="pull"
                  searchQuery={searchQuery}
                  onSearchQueryChange={setSearchQuery}
                  onSearchResultsChange={setSearchResults}
                  selectedResourceIds={selected}
                  selectableResourceIds={conversationIds}
                  reviewResourceIds={reviewResourceIds}
                  selectionDisabled={busy}
                  onToggleResource={toggle}
                  onActionError={onSessionActionError}
                  onDropSelection={(resourceIds) => setSelected((current) => {
                    // Show less took these out of view. A tick you can no
                    // longer see is one you cannot check before it applies.
                    const next = new Set(current);
                    resourceIds.forEach((resourceId) => next.delete(resourceId));
                    return next;
                  })}
                  continuityActionBusy={busy}
                  onReviewListStateChange={setListState}
                  reviewScaffold={reviewScaffold}
                />
              ) : conversationItems.length > 0 ? (
                <ItemList
                  items={matchingConversations}
                  selected={selected}
                  phase={phase}
                  disabled={busy}
                  onToggle={toggle}
                />
              ) : items.length === 0 && !supportLoading ? (
                <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>{t("pull.nothingToPull")}</strong><small>{t("pull.nothingToPullDetail")}</small></span></div>
              ) : (
                <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>{t("pull.noSessionChanges")}</strong><small>{t("pull.noSessionChangesDetail")}</small></span></div>
              )}
              {!sessionScope && (otherItems.length > 0 || supportLoading) && (
                <details className="sync-sync-review-other">
                  <summary>
                    <span><Icon name="settings" size={15} /><strong>{t("pull.setupAndTools")}</strong></span>
                    {/* The count waits on the readiness checks, in the line
                        that is on screen, not inside a closed fold. */}
                    <span>{supportLoading ? t("pull.checking") : t("pull.selectedCount", { count: selectedIn(otherItems) })}<Icon name="chevron-right" size={13} /></span>
                  </summary>
                  <ItemList items={otherItems} selected={selected} phase={phase} disabled={busy} onToggle={toggle} />
                </details>
              )}
            </>
          )}

          {activeStep === "project_files" && (
            <ProjectFilesReviewPage
              mode="pull"
              projectRoot={binding.canonical_project_root || binding.project_root}
              onScan={onRefreshPlan}
              eligibility={plan.project_content_eligibility ?? {
                state: "unknown",
                reason: t("pull.eligibilityUnrecorded"),
              }}
              rows={projectContentRows}
              selectedIds={selected}
              requiredIds={requiredProjectDirectories}
              disabled={busy}
              onToggle={toggle}
              onBulkToggle={(resourceIds, shouldSelect) => setSelected((current) => {
                if (!projectContentEligible) return current;
                const next = new Set(current);
                resourceIds.forEach((resourceId) => {
                  if (shouldSelect) next.add(resourceId);
                  else if (!requiredPullProjectDirectoryIds(items, next).has(resourceId)) next.delete(resourceId);
                });
                return includeRequiredPullProjectDirectories(items, next);
              })}
            />
          )}

        </div>
      </div>

      <footer className="sync-pull-apply-bar sync-inline-action-footer sync-sync-review-footer">
        {overLimitSentence && (
          /* The refusal takes a row of its own above the button cluster, where
             the gauge that caused it and the button it stops both sit. It does
             not announce: the gauge beside it is the status region, and its
             own text already changed to the overshoot. */
          <p className="sync-review-footer-note danger">{overLimitSentence}</p>
        )}
        <ProjectFileCommitImpact
          counts={projectFileImpact}
          deletionTarget="local"
          deletionApproved={false}
          disabled={busy}
          onDeletionApprovedChange={() => {}}
        />
        {showProjectFiles && (
          /* The step navigator, not a gate: Pull stays live on both steps —
             the same footer contract, box and waiting count the Push review
             carries. */
          <button
            type="button"
            className="btn"
            disabled={busy}
            title={activeStep !== "history"
              ? t("review.backToSessions")
              : waitingFileCount > 0
                ? t("pull.waitingFiles", { count: waitingFileCount })
                : t("review.filesNext")}
            onClick={() => setActiveStep(activeStep === "history" ? "project_files" : "history")}
          >
            {activeStep !== "history" && <Icon name="chevron-left" size={14} />}
            {activeStep === "history" ? t("review.projectFiles") : historyStepLabel(showProjectFiles)}
            {activeStep === "history" && waitingFiles.added > 0 && (
              <span className="sync-path-count">{t("pull.newCount", { count: waitingFiles.added })}</span>
            )}
            {activeStep === "history" && waitingFiles.changed > 0 && (
              <span className="sync-path-count">{t("pull.changedCount", { count: waitingFiles.changed })}</span>
            )}
            {activeStep === "history" && <Icon name="chevron-right" size={14} />}
          </button>
        )}
        {/* i18n-ignore: the meter's mode key, not copy */}
        {!listPending && <ReviewBudgetGauge budget={budget} verb="pull" breakdown={budgetBreakdown} />}
        <button
          type="button"
          className="btn btn-primary sync-pull-apply-button"
          disabled={busy || primaryDisabledReason !== null}
          title={primaryDisabledReason ?? undefined}
          onClick={() => onApply(selection)}
        >
          {busy || listPending
            ? <Icon name="refresh" size={16} className="icon-spin" />
            : <ThreadStatusIcon status="cloud-outstanding" size={16} />}
          {busy ? primaryBusyLabel : primaryLabel}
        </button>
      </footer>
    </section>
  );
}
