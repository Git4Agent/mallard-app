import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type {
  BundleReadiness,
  LocalProjectSummary,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import ProjectActivityPage from "./ProjectActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import SyncReviewTabs from "../sync-review/SyncReviewTabs";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import { type SyncReviewStep, projectReviewTabs, syncReviewSteps } from "./reviewSteps";
import { providerLabel } from "./model";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { isIrrecoverableSyncError } from "./syncError";
import type { PullApplyPhase, PullReviewSelection } from "./pullReviewFlow";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  defaultSelected,
  itemKindLabel,
  includeRequiredPullProjectDirectories,
  pendingIds,
  requiredPullProjectDirectoryIds,
  keepsLocalConversation,
  requiresApproval,
  requiresManualResolution,
  restoreActionIds,
  type PullReviewItem,
} from "./pullReviewModel";

interface Props {
  projectName: string;
  project?: LocalProjectSummary;
  storageName?: string | null;
  profileLabel: string;
  plan: RestorePlan;
  binding: ProjectBinding;
  readiness: BundleReadiness | null;
  restoreResult: RestoreResult | null;
  phase: PullApplyPhase;
  supportLoading: boolean;
  completedActionIds: ReadonlySet<string>;
  completedResourceIds: ReadonlySet<string>;
  failedResourceIds: ReadonlySet<string>;
  busy: boolean;
  error: string | null;
  embedded?: boolean;
  initialStep?: SyncReviewStep;
  onApply: (selection: PullReviewSelection) => void;
  onBack: () => void;
  onUnlinkStorage?: () => void;
  onResetMetadata?: () => void;
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
  /** Applies the review's current selection — the footer's own onClick. */
  apply: () => void;
}

export function pullSelectableResourceIds(
  items: PullReviewItem[],
  completedActionIds: ReadonlySet<string>,
  completedResourceIds: ReadonlySet<string>,
): Set<string> {
  return new Set(items
    .filter((item) => (
      pendingIds(item, completedActionIds).length > 0
      && !completedResourceIds.has(item.resourceId)
      && !keepsLocalConversation(item)
      && !requiresManualResolution(item)
    ))
    .map((item) => item.resourceId));
}

function ItemList({
  items,
  selected,
  completedActionIds,
  completedResourceIds,
  failedResourceIds,
  phase,
  disabled,
  onToggle,
}: {
  items: PullReviewItem[];
  selected: ReadonlySet<string>;
  completedActionIds: ReadonlySet<string>;
  completedResourceIds: ReadonlySet<string>;
  failedResourceIds: ReadonlySet<string>;
  phase: PullApplyPhase;
  disabled: boolean;
  onToggle: (resourceId: string) => void;
}) {
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
        const pending = pendingIds(item, completedActionIds);
        const completed = completedResourceIds.has(item.resourceId);
        const failed = failedResourceIds.has(item.resourceId);
        const keepLocalConversation = keepsLocalConversation(item);
        const manualResolution = requiresManualResolution(item);
        const blocked = keepLocalConversation || manualResolution || pending.length === 0 || completed;
        const selectable = !blocked;
        const checked = selected.has(item.resourceId) && !completed;
        const expanded = expandedItems.has(item.resourceId);
        const activeRestore = phase === "restoring" && checked && restoreActionIds(item).some((id) => !completedActionIds.has(id));
        const conversationAction = item.conversationReviewRows[0]?.action
          ?? item.conversationActions[0]?.action;
        const status: string | null = conversationAction === "adopt_cloud_metadata"
          ? checked ? "Mark synced" : "Not selected"
          : conversationAction === "blocked"
              ? "Can't sync"
              : conversationAction === "keep_both" && checked
                ? "Keep both"
                : keepLocalConversation
          ? "Keep local"
          : manualResolution
          ? "Resolve manually"
          : completed
            ? "Restored"
            : failed
              ? "Failed"
              : activeRestore
                ? "Restoring"
                : requiresApproval(item) && !checked
                  ? "Needs approval"
                  : checked
                    ? null
                    : "Not selected";
        return (
          <div
            key={item.resourceId}
            className={`sync-pull-item${selectable ? " sync-pull-item-selectable" : ""}${checked ? " selected" : ""}${completed ? " completed" : ""}${failed ? " failed" : ""}`}
            role="listitem"
          >
            {selectable && (
              <label className="sync-pull-item-select">
                <input
                  type="checkbox"
                  aria-label={`Select ${item.title}`}
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
              aria-label={`${expanded ? "Hide" : "Show"} details for ${item.title}`}
            >
              <span className="sync-pull-item-toggle">
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
              </span>
              <span className="sync-pull-item-icon">
                <Icon
                  name={failed || manualResolution || keepLocalConversation ? "alert-triangle" : completed ? "check-circle" : "file"}
                  size={14}
                />
              </span>
              <span className="sync-pull-item-copy">
                <strong>{item.title}</strong>
                <span>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</span>
              </span>
              {status && <span className={`sync-pull-item-status ${status.toLowerCase().replace(/\s+/g, "-")}`}>{status}</span>}
            </button>
            {expanded && (
              <div className="sync-pull-item-detail">
                <p>{item.detail}</p>
                <div className="sync-pull-item-detail-grid">
                  <span>Type</span><code>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</code>
                  <span>Resource</span><code>{item.resourceId}</code>
                  {item.restoreActions.map((action) => (
                    <Fragment key={action.action_id}><span>Restore</span><code>{action.action_id}</code></Fragment>
                  ))}
                  {item.conversationActions.map((action) => (
                    <Fragment key={action.action_id}><span>Conversation</span><code>{action.action}</code></Fragment>
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
  project,
  profileLabel,
  storageName = "storage",
  plan,
  binding,
  readiness,
  restoreResult,
  phase,
  supportLoading,
  completedActionIds,
  completedResourceIds,
  failedResourceIds,
  busy,
  error,
  embedded = false,
  initialStep = "history",
  onApply,
  onBack,
  onUnlinkStorage,
  onResetMetadata,
  onPrimaryChange,
}: Props) {
  const items = useMemo(() => buildPullReviewItems(plan), [plan]);
  const conversationItems = useMemo(
    () => items.filter((item) => itemKindLabel(item) === "Conversation"),
    [items],
  );
  const projectContentItems = useMemo(
    () => items.filter((item) => item.category === "project_content"),
    [items],
  );
  const otherItems = useMemo(() => items.filter((item) => (
    itemKindLabel(item) !== "Conversation"
    && item.category !== "project_content"
  )), [items]);
  const showProjectFiles = plan.project_content_eligibility?.state === "eligible" || projectContentItems.length > 0;
  const reviewSteps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const knownItems = useRef(new Set<string>());
  const [activeStep, setActiveStep] = useState<SyncReviewStep>(() => (
    syncReviewSteps(showProjectFiles).includes(initialStep) ? initialStep : "history"
  ));
  const [selected, setSelected] = useState<Set<string>>(() => new Set(
    items.filter(defaultSelected).map((item) => item.resourceId),
  ));

  useEffect(() => {
    const currentIds = new Set(items.map((item) => item.resourceId));
    setSelected((current) => {
      const next = new Set([...current].filter((resourceId) => (
        currentIds.has(resourceId) && !completedResourceIds.has(resourceId)
      )));
      for (const item of items) {
        if (!knownItems.current.has(item.resourceId) && defaultSelected(item)) next.add(item.resourceId);
        if (failedResourceIds.has(item.resourceId)) next.add(item.resourceId);
      }
      return includeRequiredPullProjectDirectories(items, next);
    });
    knownItems.current = currentIds;
  }, [items, completedResourceIds, failedResourceIds]);

  const selection = useMemo(
    () => buildPullReviewSelection(
      items,
      selected,
      completedActionIds,
      completedResourceIds,
    ),
    [items, selected, completedActionIds, completedResourceIds],
  );
  const hasResult = phase === "complete" || restoreResult !== null;
  const approvalItems = items.filter((item) => (
    item.category !== "project_content"
    && requiresApproval(item)
    && !selected.has(item.resourceId)
    && !completedResourceIds.has(item.resourceId)
  ));
  const manualItems = items.filter((item) => (
    requiresManualResolution(item) && !completedResourceIds.has(item.resourceId)
  ));
  const keptLocalConversationItems = items.filter((item) => (
    keepsLocalConversation(item) && !completedResourceIds.has(item.resourceId)
  ));

  useEffect(() => {
    if (!reviewSteps.includes(activeStep)) setActiveStep("history");
  }, [activeStep, reviewSteps]);

  // The embedded Activity review renders the storage rows ("Only in …") from
  // its own async fetches; until it reports ready, the primary must not apply
  // a selection the user has not seen. The ItemList fallback is synchronous.
  const embeddedHistoryInUse = conversationItems.length > 0 && Boolean(project);
  const [reviewListReady, setReviewListReady] = useState(false);
  const failureCount = restoreResult?.failed_actions?.length ?? 0;
  const finalReady = hasResult && !error && failureCount === 0 && readiness?.state === "ready";
  // The review closes the moment a result exists — the outcome is reported on
  // the storage card — so the footer only ever offers the Pull itself. An
  // empty selection is still a real Pull: the apply writes receipts and
  // advances the reviewed baseline, which is what unblocks Push when the
  // storage is ahead and the user wants none of its changes.
  const primaryDisabledReason = busy
    ? null
    : embeddedHistoryInUse && !reviewListReady
      ? "Loading storage changes…"
      : null;

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
      apply: () => onApplyRef.current(selectionRef.current),
    });
  }, [primaryDisabledReason, onPrimaryChange]);
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
  const recommendedIds = new Set(items.filter(defaultSelected).map((item) => item.resourceId));
  // The history step hosts every non-file item: sessions in the list, agent
  // setup & tools in the details row at the bottom of the same panel.
  const stepItems = activeStep === "project_files"
    ? projectContentItems
    : [...conversationItems, ...otherItems];
  const pendingStepItems = stepItems.filter((item) => (
    pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
  ));
  const pendingRecommendedCount = pendingStepItems.filter((item) => recommendedIds.has(item.resourceId)).length;
  const stepMatchesRecommended = pendingStepItems.every((item) => (
    selected.has(item.resourceId) === recommendedIds.has(item.resourceId)
  ));
  const useRecommended = () => setSelected((current) => {
    const next = new Set(current);
    for (const item of stepItems) {
      next.delete(item.resourceId);
      if (recommendedIds.has(item.resourceId)) next.add(item.resourceId);
    }
    return includeRequiredPullProjectDirectories(items, next);
  });
  const clearStep = () => setSelected((current) => {
    const next = new Set(current);
    for (const item of stepItems) next.delete(item.resourceId);
    return next;
  });
  const selectedIn = (chosen: PullReviewItem[]) => chosen.filter((item) => selected.has(item.resourceId)).length;
  const counts = {
    history: selectedIn(conversationItems) + selectedIn(otherItems),
    project_files: selectedIn(projectContentItems),
  };
  const warningSteps = new Set<SyncReviewStep>();
  for (const item of approvalItems) {
    if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
  }
  for (const item of manualItems) {
    if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
  }
  if (keptLocalConversationItems.length > 0) warningSteps.add("history");
  if (projectContentItems.length > 0 && !projectContentEligible) {
    warningSteps.add("project_files");
  }
  const conversationIds = pullSelectableResourceIds(
    conversationItems,
    completedActionIds,
    completedResourceIds,
  );
  const storageReplacedGate = !hasResult && plan.remote_continuity?.state === "replaced";
  /* The banner zone that heads the review scroll. Both messages can be on
     screen at once, so they share one box at the loudest rung present. */
  const banners: NoticeItem[] = [];
  if (storageReplacedGate) {
    banners.push({
      key: "storage-replaced",
      severity: "warning",
      title: "Storage history changed",
      message: `${storageName || "This storage"} was reset. Pulling adopts its new history; local content is backed up first.`,
      action: onUnlinkStorage
        ? (
          <button type="button" className="btn btn-ghost danger" onClick={onUnlinkStorage} disabled={busy}>
            Unlink storage…
          </button>
        )
        : undefined,
    });
  }
  if (error && !hasResult) {
    banners.push({
      key: "pull-failed",
      severity: "danger",
      message: error,
      action: isIrrecoverableSyncError(error) && onResetMetadata
        ? (
          <button type="button" className="btn btn-ghost danger" onClick={onResetMetadata} disabled={busy}>
            <Icon name="trash" size={13} />
            Reset local sync state
          </button>
        )
        : undefined,
    });
  }
  /* The replaced-history gate is read straight off the plan this view was
     handed, so it is painted on the first render — standing content, and a
     named region rather than a live one. The error is the opposite: the review
     always mounts with `error` null (the host clears it before it plans, and
     even the supporting-check failure lands after the plan is handed over), so
     an error that is present arrived after an action and stops the flow. One
     present error re-times the whole box. */
  const bannerTiming: NoticeTiming = error && !hasResult
    ? { timing: "blocking" }
    : { timing: "standing", label: "Notes about this Pull review" };
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
        storagePresent: item.projectContentOperation !== "delete_file" && item.projectContentOperation !== "delete_directory",
        operation: item.projectContentOperation ?? undefined,
      };
    });
  const ReviewContainer = embedded ? "section" : "main";

  return (
    <ReviewContainer
      className={embedded ? "sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" : "sync-main sync-pull-review-workspace sync-sync-review-workspace"}
      aria-labelledby="sync-pull-review-title"
    >
      <header className={embedded ? "sync-inline-action-header sync-inline-pull-header sync-sync-review-header" : "sync-pull-review-header sync-sync-review-header"}>
        {!embedded && (
          <button type="button" className="btn btn-ghost sync-pull-back" onClick={onBack} disabled={busy}>
            <Icon name="chevron-left" size={15} /> Back to <span className="sync-btn-name">{projectName}</span>
          </button>
        )}
        <div className={embedded ? "sync-inline-action-heading" : "sync-pull-review-heading"}>
          <div className={embedded ? "sync-sync-review-title" : undefined}>
            {embedded
              ? (
                <>
                  <h2 id="sync-pull-review-title">Pull review</h2>
                  <span className="sync-pull-review-meta">Revision {plan.revision}</span>
                </>
              )
              : <h1 id="sync-pull-review-title">Restore {projectName}</h1>}
            {!embedded && (
                <p className="sync-pull-review-meta">
                  <span>{`Choose changes from ${storageName || "storage"} to apply to this machine.`}</span>
                  {/* Naming the write destination belongs on the screen that
                      approves the write. The prop was already required and
                      passed; only the line that read it was removed. */}
                  {profileLabel && <span>{`Installs into ${profileLabel}.`}</span>}
                </p>
              )}
          </div>
        </div>
        {/* The recommend button's own count is a property of the plan, not of
            the selection, so it never moves when a row is ticked. */}
        <SyncReviewSelectionBar
          count={selectedIn(pendingStepItems)}
          total={pendingStepItems.length}
          busy={busy}
          recommendedTitle={`Restore the recommended safe selection (${pendingRecommendedCount})`}
          recommendedAriaLabel={`Use recommended selection (${pendingRecommendedCount})`}
          recommendedDisabled={finalReady || pendingStepItems.length === 0 || stepMatchesRecommended}
          onRecommended={useRecommended}
          clearDisabled={finalReady || selectedIn(stepItems) === 0}
          onClear={clearStep}
          close={embedded
            ? { ariaLabel: `Close pull review for ${projectName}`, onClose: onBack }
            : undefined}
        />
      </header>

      <SyncReviewTabs
        tabs={projectReviewTabs(reviewSteps, counts, warningSteps)}
        activeId={activeStep}
        disabled={busy}
        onChange={(step) => setActiveStep(step as SyncReviewStep)}
      />

      <div
        id={`sync-review-${activeStep}-panel`}
        className={embedded ? "sync-pull-review-content sync-inline-action-content sync-sync-review-content" : "sync-pull-review-content sync-sync-review-content"}
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
              <NoticeZone items={banners} subject="notes about this Pull review" {...bannerTiming} />
            </div>
          )}
          {activeStep === "history" && (
            <>
              {conversationItems.length > 0 && project ? (
                <ProjectActivityPage
                  embedded
                  project={project}
                  binding={binding}
                  refreshEpoch={0}
                  activeStorageId={plan.storage_id}
                  activeStorageName={storageName}
                  selectionMode="pull"
                  selectedResourceIds={selected}
                  selectableResourceIds={conversationIds}
                  selectionDisabled={busy || finalReady}
                  onToggleResource={toggle}
                  onUnlinkStorage={onUnlinkStorage}
                  continuityActionBusy={busy || finalReady}
                  onReviewListReadyChange={setReviewListReady}
                />
              ) : conversationItems.length > 0 ? (
                <ItemList
                  items={conversationItems}
                  selected={selected}
                  completedActionIds={completedActionIds}
                  completedResourceIds={completedResourceIds}
                  failedResourceIds={failedResourceIds}
                  phase={phase}
                  disabled={busy || finalReady}
                  onToggle={toggle}
                />
              ) : items.length === 0 && !supportLoading ? (
                <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>Nothing to pull</strong><small>This project already matches the selected revision.</small></span></div>
              ) : (
                <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No session changes</strong><small>Sessions already match this storage.</small></span></div>
              )}
              {(otherItems.length > 0 || supportLoading) && (
                <details className="sync-sync-review-other">
                  <summary>
                    <span><Icon name="settings" size={15} /><strong>Agent setup &amp; tools</strong></span>
                    <span>{selectedIn(otherItems)} selected<Icon name="chevron-right" size={13} /></span>
                  </summary>
                  <ItemList items={otherItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} disabled={busy || finalReady} onToggle={toggle} />
                  {supportLoading && <div className="sync-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking readiness…</div>}
                </details>
              )}
            </>
          )}

          {activeStep === "project_files" && (
            <ProjectFilesReviewPage
              mode="pull"
              eligibility={plan.project_content_eligibility ?? {
                state: "unknown",
                reason: "Project-file eligibility was not recorded in this Pull plan.",
              }}
              rows={projectContentRows}
              selectedIds={selected}
              requiredIds={requiredProjectDirectories}
              disabled={busy || finalReady}
              onSelectSafe={() => setSelected((current) => includeRequiredPullProjectDirectories(
                items,
                new Set([
                  ...current,
                  ...projectContentItems
                    .filter((item) => item.projectContentOperation === "add" || item.projectContentOperation === "create_directory")
                    .map((item) => item.resourceId),
                ]),
              ))}
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

      <footer className={embedded ? "sync-pull-apply-bar sync-inline-action-footer sync-sync-review-footer" : "sync-pull-apply-bar sync-sync-review-footer"}>
        {busy && (
          /* Same contract as Push: no safe mid-write abort exists, so the
             sealed exits are named instead of leaving a cancel to hunt for.
             And as on Push it announces, because it appears only once `busy`
             flips — the one explanation for the button going dead under the
             reader's focus and Escape no longer closing the review. */
          <span role="status"><small>A running pull finishes or fails whole — it can’t be cancelled midway.</small></span>
        )}
        <button
          type="button"
          className="btn btn-primary sync-pull-apply-button"
          disabled={busy || primaryDisabledReason !== null}
          title={primaryDisabledReason ?? undefined}
          onClick={() => onApply(selection)}
        >
          {busy
            ? <Icon name="refresh" size={16} className="icon-spin" />
            : <ThreadStatusIcon status="cloud-outstanding" size={16} />}
          {busy ? "Pulling…" : "Pull"}
        </button>
      </footer>
    </ReviewContainer>
  );
}
