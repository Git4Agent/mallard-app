import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type {
  LocalProjectSummary,
  ProjectBinding,
  PullReview,
} from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import ProjectActivityPage from "./ProjectActivityPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import ProjectFileCommitImpact, {
  type ProjectFileCommitImpactCounts,
} from "./ProjectFileCommitImpact";
import SyncReviewTabs from "../sync-review/SyncReviewTabs";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import { type SyncReviewStep, historyStepLabel, projectReviewTabs, syncReviewSteps } from "./reviewSteps";
import { providerLabel } from "./model";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { isIrrecoverableSyncError } from "./syncError";
import type { PullApplyPhase, PullReviewSelection } from "./pullReviewFlow";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  itemKindLabel,
  includeRequiredPullProjectDirectories,
  recommendedPullSelection,
  requiredPullProjectDirectoryIds,
  requiresApproval,
  restoreActionIds,
  type PullReviewItem,
} from "./pullReviewModel";

interface Props {
  projectName: string;
  project?: LocalProjectSummary;
  storageName?: string | null;
  plan: PullReview;
  binding: ProjectBinding;
  phase: PullApplyPhase;
  supportLoading: boolean;
  busy: boolean;
  error: string | null;
  onApply: (selection: PullReviewSelection) => void;
  onBack: () => void;
  /** Rebuilds the plan against the storage's current head — the files
   *  page's Rescan. */
  onRefreshPlan?: () => void;
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
function itemOperationChip(item: PullReviewItem): string | null {
  const action = item.restoreActions[0];
  if (!action) return null;
  if (action.kind.kind === "write_file") {
    return action.expected_target_sha256 ? "Replace file" : "Add file";
  }
  if (action.kind.kind === "merge_file") return "Merge file";
  return null;
}

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
        const status = activeRestore
          ? "Restoring"
          : item.conversationReviewRows[0]?.action === "keep_both" && checked
            ? "Keep both"
            : requiresApproval(item) && !checked
              ? "Needs approval"
              : checked ? null : "Not selected";
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
                  name="file"
                  size={14}
                />
              </span>
              <span className="sync-pull-item-copy">
                <strong>{item.title}</strong>
                <span>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</span>
              </span>
              {itemOperationChip(item) && (
                <span className="sync-history-branch-state history-only" title={item.detail}>
                  {itemOperationChip(item)}
                </span>
              )}
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
  storageName = "storage",
  plan,
  binding,
  phase,
  supportLoading,
  busy,
  error,
  onApply,
  onBack,
  onRefreshPlan,
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
  const [activeStep, setActiveStep] = useState<SyncReviewStep>("history");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
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

  const selection = useMemo(() => buildPullReviewSelection(items, selected), [items, selected]);
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
  const [reviewListReady, setReviewListReady] = useState(false);
  const primaryDisabledReason = !busy && activityHistoryInUse && !reviewListReady
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
  const recommendedIds = new Set(items.filter(recommendedPullSelection).map((item) => item.resourceId));
  // The history step hosts every non-file item: sessions in the list, agent
  // setup & tools in the details row at the bottom of the same panel.
  const stepItems = activeStep === "project_files"
    ? projectContentItems
    : [...conversationItems, ...otherItems];
  const pendingStepItems = stepItems.filter((item) => (
    restoreActionIds(item).length > 0
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
  if (projectContentItems.length > 0 && !projectContentEligible) {
    warningSteps.add("project_files");
  }
  const conversationIds = pullSelectableResourceIds(conversationItems);
  const storageReplacedGate = plan.remote_continuity?.state === "replaced";
  /* The banner zone that heads the review scroll. Both messages can be on
     screen at once, so they share one box at the loudest rung present. */
  const banners: NoticeItem[] = [];
  if (plan.conversation_review.some((row) => row.stored_without_credential_redaction)) {
    banners.push({
      key: "stored-without-redaction",
      severity: "warning",
      title: "Stored without credential redaction",
      message: "This review includes sessions that can contain API keys, tokens, and passwords.",
    });
  }
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
  if (error) {
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
  /* The replaced-history gate comes from the plan and appears on the first
     render — standing content, and a
     named region rather than a live one. The error is the opposite: the review
     always mounts with `error` null (the host clears it before it plans, and
     even the supporting-check failure lands after the plan is handed over), so
     an error that is present arrived after an action and stops the flow. One
     present error re-times the whole box. */
  const bannerTiming: NoticeTiming = error
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
            <h2 id="sync-pull-review-title">Pull review</h2>
            <span className="sync-pull-review-meta">Revision {plan.revision}</span>
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
          recommendedDisabled={pendingStepItems.length === 0 || stepMatchesRecommended}
          onRecommended={useRecommended}
          clearDisabled={selectedIn(stepItems) === 0}
          onClear={clearStep}
          close={{ ariaLabel: `Close pull review for ${projectName}`, onClose: onBack }}
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
              <NoticeZone items={banners} subject="notes about this Pull review" {...bannerTiming} />
            </div>
          )}
          {activeStep === "history" && (
            <>
              {conversationItems.length > 0 && project ? (
                <ProjectActivityPage
                  project={project}
                  binding={binding}
                  refreshEpoch={0}
                  activeStorageId={plan.storage_id}
                  activeStorageName={storageName}
                  selectionMode="pull"
                  selectedResourceIds={selected}
                  selectableResourceIds={conversationIds}
                  selectionDisabled={busy}
                  onToggleResource={toggle}
                  onDropSelection={(resourceIds) => setSelected((current) => {
                    // Show less took these out of view. A tick you can no
                    // longer see is one you cannot check before it applies.
                    const next = new Set(current);
                    resourceIds.forEach((resourceId) => next.delete(resourceId));
                    return next;
                  })}
                  onUnlinkStorage={onUnlinkStorage}
                  continuityActionBusy={busy}
                  onReviewListReadyChange={setReviewListReady}
                />
              ) : conversationItems.length > 0 ? (
                <ItemList
                  items={conversationItems}
                  selected={selected}
                  phase={phase}
                  disabled={busy}
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
                  <ItemList items={otherItems} selected={selected} phase={phase} disabled={busy} onToggle={toggle} />
                  {supportLoading && <div className="sync-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking readiness…</div>}
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
                reason: "Project-file eligibility was not recorded in this Pull plan.",
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
        <ProjectFileCommitImpact
          counts={projectFileImpact}
          deletionTarget="local"
          deletionApproved={false}
          disabled={busy}
          onDeletionApprovedChange={() => {}}
        />
        {busy && (
          /* Same contract as Push: no safe mid-write abort exists, so the
             sealed exits are named instead of leaving a cancel to hunt for.
             And as on Push it announces, because it appears only once `busy`
             flips — the one explanation for the button going dead under the
             reader's focus and Escape no longer closing the review. */
          <span role="status"><small>Pull cannot be cancelled while changes are being applied.</small></span>
        )}
        {showProjectFiles && (
          /* The step navigator, not a gate: Pull stays live on both steps —
             the same footer contract the Push review carries. */
          <button
            type="button"
            className="btn btn-ghost"
            disabled={busy}
            title={activeStep === "history" ? "Review project files next" : "Back to session selection"}
            onClick={() => setActiveStep(activeStep === "history" ? "project_files" : "history")}
          >
            {activeStep !== "history" && <Icon name="chevron-left" size={14} />}
            {activeStep === "history" ? "Project files" : historyStepLabel(showProjectFiles)}
            {activeStep === "history" && <Icon name="chevron-right" size={14} />}
          </button>
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
    </section>
  );
}
