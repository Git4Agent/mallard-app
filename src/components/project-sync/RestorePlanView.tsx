import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type {
  BundleReadiness,
  DependencyPlan,
  DependencyResult,
  LocalProjectSummary,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../types";
import Icon from "../Icons";
import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
import SkillsPluginStatusPage from "./SkillsPluginStatusPage";
import SyncReviewTabs, {
  type SyncReviewStep,
  syncReviewSteps,
} from "./SyncReviewTabs";
import { providerLabel } from "./model";
import type { PullApplyPhase, PullReviewSelection } from "./pullReviewFlow";
import {
  buildPullReviewItems,
  buildPullReviewSelection,
  defaultSelected,
  dependencyActionIds,
  isPinnedNoopConversation,
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
  dependencyPlan: DependencyPlan | null;
  readiness: BundleReadiness | null;
  restoreResult: RestoreResult | null;
  dependencyResult: DependencyResult | null;
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
  onRefresh: () => void;
  onBack: () => void;
}

const STEP_LABELS: Record<SyncReviewStep, string> = {
  history: "Git & sessions",
  skills: "Skills",
  plugins: "Plugins",
  project_files: "Project files",
  review: "Review",
};

function phaseIndex(phase: PullApplyPhase): number {
  if (phase === "restoring") return 0;
  if (phase === "installing") return 1;
  if (phase === "verifying") return 2;
  if (phase === "complete") return 3;
  return -1;
}

function PhaseProgress({
  phase,
  ready,
  restoreFailed,
  installFailed,
}: {
  phase: PullApplyPhase;
  ready: boolean;
  restoreFailed: boolean;
  installFailed: boolean;
}) {
  const activeIndex = phaseIndex(phase);
  const steps = [
    { label: "Restore project data and custom skills", icon: "file" as const },
    { label: "Install selected plugins and tools", icon: "download" as const },
    { label: "Verify project readiness", icon: "refresh" as const },
  ];
  return (
    <section className="v3-pull-progress" aria-label="Pull progress" aria-live="polite">
      {steps.map((step, index) => {
        const current = activeIndex === index;
        const warning = phase === "complete" && (
          (index === 0 && restoreFailed) ||
          (index === 1 && installFailed) ||
          (index === 2 && !ready)
        );
        const complete = !warning && (activeIndex > index || phase === "complete");
        return (
          <div key={step.label} className={`${current ? "active" : ""}${complete ? " complete" : ""}${warning ? " warning" : ""}`}>
            <span>
              <Icon
                name={warning ? "alert-triangle" : complete ? "check-circle" : current ? "refresh" : step.icon}
                size={16}
                className={current ? "icon-spin" : undefined}
              />
            </span>
            <strong>{step.label}</strong>
            <small>{warning ? "Needs attention" : complete ? "Complete" : current ? "In progress" : "Waiting"}</small>
          </div>
        );
      })}
    </section>
  );
}

function ResultDetails({
  restoreResult,
  dependencyResult,
}: {
  restoreResult: RestoreResult | null;
  dependencyResult: DependencyResult | null;
}) {
  const results = [restoreResult, dependencyResult].filter((result): result is RestoreResult | DependencyResult => result !== null);
  if (results.length === 0) return null;
  return (
    <details className="v3-pull-result-details">
      <summary>Technical results</summary>
      {results.map((result, index) => (
        <div key={`${result.message}-${index}`}>
          <strong>{result.message}</strong>
          {(result.failed_actions ?? []).map((failure) => (
            <span key={failure.action_id}><code>{failure.display_name ?? failure.action_id}</code>{failure.message}</span>
          ))}
        </div>
      ))}
    </details>
  );
}

function ItemList({
  items,
  selected,
  completedActionIds,
  completedResourceIds,
  failedResourceIds,
  phase,
  supportLoading,
  disabled,
  onToggle,
}: {
  items: PullReviewItem[];
  selected: ReadonlySet<string>;
  completedActionIds: ReadonlySet<string>;
  completedResourceIds: ReadonlySet<string>;
  failedResourceIds: ReadonlySet<string>;
  phase: PullApplyPhase;
  supportLoading: boolean;
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
    <div className="v3-pull-items" role="list">
      {items.map((item) => {
        const pending = pendingIds(item, completedActionIds);
        const completed = completedResourceIds.has(item.resourceId);
        const failed = failedResourceIds.has(item.resourceId);
        const waitingForInstaller =
          item.category === "global_tool" &&
          item.toolKind !== "custom_skill" &&
          pending.length === 0 &&
          !completed &&
          supportLoading;
        const keepLocalConversation = keepsLocalConversation(item);
        const manualResolution = requiresManualResolution(item);
        const blocked = keepLocalConversation || manualResolution || pending.length === 0 || completed || waitingForInstaller;
        const checked = selected.has(item.resourceId) && !completed;
        const expanded = expandedItems.has(item.resourceId);
        const activeRestore = phase === "restoring" && checked && restoreActionIds(item).some((id) => !completedActionIds.has(id));
        const activeInstall = phase === "installing" && checked && dependencyActionIds(item).some((id) => !completedActionIds.has(id));
        const status: string | null = keepLocalConversation
          ? "Keep local"
          : manualResolution
          ? "Resolve manually"
          : item.category === "global_tool"
          ? completed
            ? "Installed"
            : failed
              ? "Failed"
              : activeRestore || activeInstall
                ? "Installing"
                : checked || waitingForInstaller
                  ? null
                  : "Needs approval"
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
            className={`v3-pull-item${checked ? " selected" : ""}${completed ? " completed" : ""}${failed ? " failed" : ""}`}
            role="listitem"
          >
            <label className="v3-pull-item-select">
              <input
                type="checkbox"
                aria-label={`Select ${item.title}`}
                checked={checked}
                disabled={disabled || blocked}
                onChange={() => onToggle(item.resourceId)}
              />
            </label>
            <button
              type="button"
              className="v3-pull-item-main"
              onClick={() => toggleDetails(item.resourceId)}
              aria-expanded={expanded}
              aria-label={`${expanded ? "Hide" : "Show"} details for ${item.title}`}
            >
              <span className="v3-pull-item-toggle">
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={12} />
              </span>
              <span className="v3-pull-item-icon">
                <Icon
                  name={failed || manualResolution || keepLocalConversation ? "alert-triangle" : completed ? "check-circle" : item.toolKind === "plugin" ? "download" : item.toolKind === "custom_skill" ? "folder" : "file"}
                  size={14}
                />
              </span>
              <span className="v3-pull-item-copy">
                <strong>{item.title}</strong>
                <span>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</span>
              </span>
              {status && <span className={`v3-pull-item-status ${status.toLowerCase().replace(/\s+/g, "-")}`}>{status}</span>}
            </button>
            {expanded && (
              <div className="v3-pull-item-detail">
                <p>{item.detail}</p>
                <div className="v3-pull-item-detail-grid">
                  <span>Type</span><code>{itemKindLabel(item)}{item.provider ? ` · ${providerLabel(item.provider)}` : ""}</code>
                  <span>Resource</span><code>{item.resourceId}</code>
                  {item.restoreActions.map((action) => (
                    <Fragment key={action.action_id}><span>Restore</span><code>{action.action_id}</code></Fragment>
                  ))}
                  {item.dependencyActions.map((action) => (
                    <Fragment key={action.action_id}><span>Installer</span><code>{action.action_id}</code></Fragment>
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
  dependencyPlan,
  readiness,
  restoreResult,
  dependencyResult,
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
  onRefresh,
  onBack,
}: Props) {
  const items = useMemo(() => buildPullReviewItems(plan, dependencyPlan), [plan, dependencyPlan]);
  const conversationItems = useMemo(
    () => items.filter((item) => itemKindLabel(item) === "Conversation"),
    [items],
  );
  const upToDateConversationItems = useMemo(
    () => conversationItems.filter(isPinnedNoopConversation),
    [conversationItems],
  );
  const upToDateConversationIds = useMemo(
    () => new Set(upToDateConversationItems.map((item) => item.resourceId)),
    [upToDateConversationItems],
  );
  const internalResourceIds = useMemo(() => {
    const resourceIds = new Set(upToDateConversationIds);
    if (items.some((item) => item.resourceId === "codex:session-index")) {
      resourceIds.add("codex:session-index");
    }
    return resourceIds;
  }, [items, upToDateConversationIds]);
  const visibleItems = useMemo(
    () => items.filter((item) => !internalResourceIds.has(item.resourceId)),
    [items, internalResourceIds],
  );
  const visibleConversationItems = useMemo(
    () => conversationItems.filter((item) => !upToDateConversationIds.has(item.resourceId)),
    [conversationItems, upToDateConversationIds],
  );
  const conversationReviewIds = useMemo(
    () => new Set(conversationItems.map((item) => item.resourceId)),
    [conversationItems],
  );
  const skillItems = useMemo(() => items.filter((item) => item.toolKind === "custom_skill"), [items]);
  const pluginItems = useMemo(() => items.filter((item) => item.toolKind === "plugin"), [items]);
  const projectContentItems = useMemo(
    () => items.filter((item) => item.category === "project_content"),
    [items],
  );
  const otherItems = useMemo(() => visibleItems.filter((item) => (
    itemKindLabel(item) !== "Conversation"
    && item.toolKind !== "custom_skill"
    && item.toolKind !== "plugin"
    && item.category !== "project_content"
  )), [visibleItems]);
  const projectItems = useMemo(() => visibleItems.filter((item) => item.category === "project_data"), [visibleItems]);
  const globalItems = useMemo(() => visibleItems.filter((item) => item.category === "global_tool"), [visibleItems]);
  const showProjectFiles = plan.project_content_eligibility?.state === "eligible" || projectContentItems.length > 0;
  const reviewSteps = useMemo(() => syncReviewSteps(showProjectFiles), [showProjectFiles]);
  const knownItems = useRef(new Set<string>());
  const [activeStep, setActiveStep] = useState<SyncReviewStep>(() => (
    syncReviewSteps(showProjectFiles).includes(initialStep) ? initialStep : "review"
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
    () => buildPullReviewSelection(items, selected, completedActionIds, completedResourceIds),
    [items, selected, completedActionIds, completedResourceIds],
  );
  const selectedCount = selection.resourceIds.filter((resourceId) => !internalResourceIds.has(resourceId)).length;
  const pendingProjectContentDecisionCount = projectContentItems.filter((item) => (
    pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
  )).length;
  const hasResult = phase === "complete" || restoreResult !== null || dependencyResult !== null;
  const approvalItems = visibleItems.filter((item) => (
    item.category !== "project_content"
    && requiresApproval(item)
    && !selected.has(item.resourceId)
    && !completedResourceIds.has(item.resourceId)
  ));
  const manualItems = visibleItems.filter((item) => (
    requiresManualResolution(item) && !completedResourceIds.has(item.resourceId)
  ));
  const keptLocalConversationItems = visibleItems.filter((item) => (
    keepsLocalConversation(item) && !completedResourceIds.has(item.resourceId)
  ));

  useEffect(() => {
    if (phase !== "idle" || hasResult) setActiveStep("review");
  }, [hasResult, phase]);

  useEffect(() => {
    if (!reviewSteps.includes(activeStep)) setActiveStep("review");
  }, [activeStep, reviewSteps]);

  const failureCount = (restoreResult?.failed_actions?.length ?? 0) + (dependencyResult?.failed_actions?.length ?? 0);
  const skippedTools = globalItems.filter((item) => (
    !selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId)
  )).length;
  const unchangedCount = skippedTools + manualItems.length;
  const finalReady = hasResult && !error && failureCount === 0 && skippedTools === 0 && readiness?.state === "ready";
  const reviewCanClose = (hasResult && (
    finalReady || (selectedCount === 0 && pendingProjectContentDecisionCount === 0)
  )) || (!hasResult
    && !supportLoading
    && visibleItems.length === 0
    && pendingProjectContentDecisionCount === 0
    && keptLocalConversationItems.length === 0);
  const completedProjectItems = projectItems.filter((item) => completedResourceIds.has(item.resourceId));
  const completedSkills = globalItems.filter((item) => (
    item.toolKind === "custom_skill" && completedResourceIds.has(item.resourceId)
  ));
  const completedPlugins = globalItems.filter((item) => (
    item.toolKind === "plugin" && completedResourceIds.has(item.resourceId)
  ));
  const failedProjectItems = projectItems.filter((item) => failedResourceIds.has(item.resourceId));
  const failedTools = globalItems.filter((item) => failedResourceIds.has(item.resourceId));
  const outcomeTitle = error && hasResult
    ? "Pull needs attention"
    : failureCount > 0
      ? `${failureCount} change${failureCount === 1 ? "" : "s"} need attention`
      : unchangedCount > 0
        ? `Project restored; ${unchangedCount} item${unchangedCount === 1 ? "" : "s"} left unchanged`
        : readiness?.state === "ready"
          ? "Pull complete"
          : "Project restored; setup needs attention";
  const buttonLabel = busy
    ? phase === "installing"
      ? "Installing selected tools…"
      : phase === "verifying"
        ? "Verifying readiness…"
        : "Restoring selected changes…"
    : hasResult && failedTools.length > 0 && selectedCount === failedResourceIds.size
      ? `Retry failed installation${failedTools.length === 1 ? "" : "s"}`
      : reviewCanClose
        ? "Done"
        : hasResult && selectedCount > 0
          ? "Apply remaining changes"
          : hasResult
            ? "Review complete"
        : selectedCount === 0 && pendingProjectContentDecisionCount > 0
          ? `Keep project entr${pendingProjectContentDecisionCount === 1 ? "y" : "ies"} local`
          : selectedCount === 0 && keptLocalConversationItems.length > 0
            ? `Keep conversation${keptLocalConversationItems.length === 1 ? "" : "s"} local`
          : "Apply changes";

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
  const dependencyMessages = (dependencyPlan?.blockers?.length ?? 0) + (dependencyPlan?.warnings?.length ?? 0);
  const recommendedIds = new Set(items.filter(defaultSelected).map((item) => item.resourceId));
  const stepItems = activeStep === "history"
    ? visibleConversationItems
    : activeStep === "skills"
      ? skillItems
      : activeStep === "plugins"
        ? pluginItems
        : activeStep === "project_files"
          ? projectContentItems
        : visibleItems;
  const pendingStepItems = stepItems.filter((item) => (
    pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
  ));
  const pendingRecommendedCount = pendingStepItems.filter((item) => recommendedIds.has(item.resourceId)).length;
  const stepMatchesRecommended = pendingStepItems.every((item) => (
    selected.has(item.resourceId) === recommendedIds.has(item.resourceId)
  ));
  const useRecommended = () => setSelected((current) => {
    if (activeStep === "review") return includeRequiredPullProjectDirectories(items, new Set(recommendedIds));
    const next = new Set(current);
    for (const item of stepItems) {
      next.delete(item.resourceId);
      if (recommendedIds.has(item.resourceId)) next.add(item.resourceId);
    }
    return includeRequiredPullProjectDirectories(items, next);
  });
  const clearStep = () => setSelected((current) => {
    if (activeStep === "review") return new Set(internalResourceIds);
    const next = new Set(current);
    for (const item of stepItems) next.delete(item.resourceId);
    return next;
  });
  const selectedIn = (chosen: PullReviewItem[]) => chosen.filter((item) => selected.has(item.resourceId)).length;
  const counts = {
    history: selectedIn(visibleConversationItems),
    skills: selectedIn(skillItems),
    plugins: selectedIn(pluginItems),
    project_files: selectedIn(projectContentItems),
    review: selectedCount,
  };
  const warningSteps = new Set<SyncReviewStep>();
  for (const item of approvalItems) {
    if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
    else if (item.toolKind === "custom_skill") warningSteps.add("skills");
    else if (item.toolKind === "plugin") warningSteps.add("plugins");
    warningSteps.add("review");
  }
  for (const item of manualItems) {
    if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
    warningSteps.add("review");
  }
  if (keptLocalConversationItems.length > 0) {
    warningSteps.add("history");
    warningSteps.add("review");
  }
  if (dependencyMessages > 0 || failureCount > 0) warningSteps.add("review");
  if (projectContentItems.length > 0 && !projectContentEligible) {
    warningSteps.add("project_files");
    warningSteps.add("review");
  }
  const selectableIds = (chosen: PullReviewItem[]) => new Set(chosen
    .filter((item) => pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId))
    .map((item) => item.resourceId));
  const conversationIds = selectableIds(conversationItems);
  const skillIds = selectableIds(skillItems);
  const pluginIds = selectableIds(pluginItems);
  const stepIndex = reviewSteps.indexOf(activeStep);
  const goBack = () => setActiveStep(reviewSteps[Math.max(0, stepIndex - 1)]);
  const goNext = () => setActiveStep(reviewSteps[Math.min(reviewSteps.length - 1, stepIndex + 1)]);
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
      className={embedded ? "v3-inline-action-review v3-inline-pull-review v3-sync-review-workspace" : "v3-main v3-pull-review-workspace v3-sync-review-workspace"}
      aria-labelledby="v3-pull-review-title"
    >
      <header className={embedded ? "v3-inline-action-header v3-inline-pull-header v3-sync-review-header" : "v3-pull-review-header v3-sync-review-header"}>
        {!embedded && (
          <button type="button" className="btn btn-ghost v3-pull-back" onClick={onBack} disabled={busy}>
            <Icon name="chevron-left" size={15} /> Back to {projectName}
          </button>
        )}
        <div className={embedded ? "v3-inline-action-heading" : "v3-pull-review-heading"}>
          <div className={embedded ? "v3-sync-review-title" : undefined}>
            {embedded
              ? <h2 id="v3-pull-review-title">Pull review</h2>
              : <h1 id="v3-pull-review-title">Restore {projectName}</h1>}
            {!embedded && (
                <p className="v3-pull-review-meta">
                  <span>{`Choose changes from ${storageName || "storage"} to apply to this machine.`}</span>
                </p>
              )}
          </div>
        </div>
        <div className="v3-push-resource-actions">
          <button
            type="button"
            className="btn btn-ghost v3-sync-review-icon-action has-count"
            onClick={useRecommended}
            disabled={busy || finalReady || pendingStepItems.length === 0 || stepMatchesRecommended}
            title="Restore the recommended safe selection"
            aria-label={`Use recommended selection (${pendingRecommendedCount})`}
          >
            <Icon name="check-circle" size={15} />
            <span aria-hidden="true">{pendingRecommendedCount}</span>
          </button>
          <button
            type="button"
            className="btn btn-ghost v3-sync-review-icon-action"
            onClick={clearStep}
            disabled={busy || finalReady || selectedIn(stepItems) === 0}
            title="Clear current selection"
            aria-label="Clear current selection"
          >
            <Icon name="trash" size={15} />
          </button>
          {embedded && (
            <button
              type="button"
              className="btn btn-ghost v3-inline-action-close"
              onClick={onBack}
              disabled={busy}
              aria-label={`Close pull review for ${projectName}`}
            >
              <Icon name="x" size={15} />
            </button>
          )}
        </div>
      </header>

      <SyncReviewTabs
        activeStep={activeStep}
        counts={counts}
        steps={reviewSteps}
        warningSteps={warningSteps}
        disabled={busy}
        onChange={setActiveStep}
      />

      <div
        id={`sync-review-${activeStep}-panel`}
        className={embedded ? "v3-pull-review-content v3-inline-action-content v3-sync-review-content" : "v3-pull-review-content v3-sync-review-content"}
        role="tabpanel"
        aria-labelledby={`sync-review-${activeStep}-tab`}
      >
        <div
          className="v3-inline-action-scroll v3-sync-review-scroll"
        >
          {activeStep === "history" && conversationItems.length > 0 && project && binding.profile_ids.codex ? (
            <ProjectChatHistoryPage
              embedded
              project={project}
              binding={binding}
              refreshEpoch={0}
              activeStorageId={plan.storage_id}
              activeStorageName={storageName}
              selectionMode="pull"
              selectedResourceIds={selected}
              selectableResourceIds={conversationIds}
              reviewResourceIds={conversationReviewIds}
              upToDateResourceIds={upToDateConversationIds}
              selectionDisabled={busy || finalReady}
              onToggleResource={toggle}
            />
          ) : activeStep === "history" && visibleConversationItems.length > 0 ? (
            <ItemList
              items={visibleConversationItems}
              selected={selected}
              completedActionIds={completedActionIds}
              completedResourceIds={completedResourceIds}
              failedResourceIds={failedResourceIds}
              phase={phase}
              supportLoading={supportLoading}
              disabled={busy || finalReady}
              onToggle={toggle}
            />
          ) : activeStep === "history" ? (
            <div className="v3-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No session changes</strong><small>Sessions already match this storage.</small></span></div>
          ) : null}

          {activeStep === "skills" && skillItems.length > 0 && project ? (
            <SkillsPluginStatusPage
              view="skills"
              project={project}
              binding={binding}
              refreshEpoch={0}
              activeStorageId={plan.storage_id}
              activeStorageName={storageName}
              onOpenProjectSettings={() => undefined}
              selectionMode="pull"
              selectedResourceIds={selected}
              selectableResourceIds={skillIds}
              selectionDisabled={busy || finalReady}
              onToggleResource={toggle}
            />
          ) : activeStep === "skills" && skillItems.length > 0 ? (
            <ItemList items={skillItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
          ) : activeStep === "skills" ? (
            <div className="v3-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No skill changes</strong><small>Skills already match this storage.</small></span></div>
          ) : null}

          {activeStep === "plugins" && pluginItems.length > 0 && project ? (
            <SkillsPluginStatusPage
              view="plugins"
              project={project}
              binding={binding}
              refreshEpoch={0}
              activeStorageId={plan.storage_id}
              activeStorageName={storageName}
              onOpenProjectSettings={() => undefined}
              selectionMode="pull"
              selectedResourceIds={selected}
              selectableResourceIds={pluginIds}
              selectionDisabled={busy || finalReady}
              onToggleResource={toggle}
            />
          ) : activeStep === "plugins" && pluginItems.length > 0 ? (
            <ItemList items={pluginItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
          ) : activeStep === "plugins" ? (
            <div className="v3-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No plugin changes</strong><small>Plugins already match this storage.</small></span></div>
          ) : null}

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

          {activeStep === "review" && (
            <div className="v3-sync-review-summary">
              {phase !== "idle" && (
                <PhaseProgress phase={phase} ready={finalReady} restoreFailed={failedProjectItems.length > 0} installFailed={failedTools.length > 0} />
              )}
              {hasResult && (
                <section className={`v3-pull-outcome${finalReady ? " success" : " warning"}`} aria-live="polite">
                  <Icon name={finalReady ? "check-circle" : "alert-triangle"} size={20} />
                  <div>
                    <h2>{outcomeTitle}</h2>
                    <ul>
                      {completedProjectItems.length > 0 && <li>{completedProjectItems.length} project data change{completedProjectItems.length === 1 ? "" : "s"} restored</li>}
                      {completedSkills.length > 0 && <li>{completedSkills.length} custom skill{completedSkills.length === 1 ? "" : "s"} restored</li>}
                      {completedPlugins.length > 0 && <li>{completedPlugins.map((item) => item.title).join(", ")} installed</li>}
                      {skippedTools > 0 && <li>{globalItems.filter((item) => !selected.has(item.resourceId) && !completedResourceIds.has(item.resourceId)).map((item) => item.title).join(", ")} left unchanged</li>}
                      {keptLocalConversationItems.length > 0 && <li>{keptLocalConversationItems.length} conversation{keptLocalConversationItems.length === 1 ? "" : "s"} kept local</li>}
                      {manualItems.length > 0 && <li>{manualItems.length} conflict{manualItems.length === 1 ? "" : "s"} left unchanged for manual resolution</li>}
                      {finalReady && <li>Project is ready</li>}
                    </ul>
                  </div>
                </section>
              )}

              <div className="v3-sync-review-summary-meta"><Icon name="download" size={14} /><span>{embedded ? `Generation ${plan.source_version.generation}` : `${storageName || "Storage"} · Generation ${plan.source_version.generation}`}</span></div>
              <button type="button" onClick={() => setActiveStep("history")}>
                <span><Icon name="git-branch" size={15} /><strong>Sessions</strong></span>
                <span>{counts.history} selected<Icon name="chevron-right" size={13} /></span>
              </button>
              <button type="button" onClick={() => setActiveStep("skills")}>
                <span><Icon name="folder" size={15} /><strong>Skills</strong></span>
                <span>{counts.skills} selected<Icon name="chevron-right" size={13} /></span>
              </button>
              <button type="button" onClick={() => setActiveStep("plugins")}>
                <span><Icon name="link" size={15} /><strong>Plugins</strong></span>
                <span>{counts.plugins} selected<Icon name="chevron-right" size={13} /></span>
              </button>
              {(otherItems.length > 0 || supportLoading) && (
                <details className="v3-sync-review-other">
                  <summary>
                    <span><Icon name="settings" size={15} /><strong>Agent setup & tools</strong></span>
                    <span>{selectedIn(otherItems)} selected<Icon name="chevron-right" size={13} /></span>
                  </summary>
                  <ItemList items={otherItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} supportLoading={supportLoading} disabled={busy || finalReady} onToggle={toggle} />
                  {supportLoading && <div className="v3-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking installers…</div>}
                </details>
              )}

              {(dependencyPlan?.blockers ?? []).map((blocker) => <div key={blocker} className="v3-callout error"><Icon name="alert-triangle" size={15} /> {blocker}</div>)}
              {(dependencyPlan?.warnings ?? []).map((warning) => <div key={warning} className="v3-callout"><Icon name="alert-triangle" size={15} /> {warning}</div>)}
              {!hasResult && keptLocalConversationItems.length > 0 && (
                <div className="v3-callout warning v3-sync-review-blocker">
                  <Icon name="alert-triangle" size={15} />
                  <span>
                    <strong>{keptLocalConversationItems.length === 1 ? "1 conversation differs" : `${keptLocalConversationItems.length} conversations differ`}</strong>
                    {`Apply will keep the local ${keptLocalConversationItems.length === 1 ? "version" : "versions"} and establish a comparison baseline for ${storageName || "storage"}. ${keptLocalConversationItems.length === 1 ? "It" : "They"} can be pushed later.`}
                  </span>
                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep("history")}>Review local</button>
                </div>
              )}
              {!hasResult && manualItems.length > 0 && (
                <div className="v3-callout warning v3-sync-review-blocker">
                  <Icon name="alert-triangle" size={15} />
                  <span>
                    <strong>{manualItems.length === 1 && itemKindLabel(manualItems[0]) === "Conversation" ? "1 conflicted conversation will stay unchanged" : `${manualItems.length} changes need manual resolution`}</strong>
                    {selectedCount > 0
                      ? `Apply will still restore ${selectedCount} selected change${selectedCount === 1 ? "" : "s"}. Resolve ${manualItems.length === 1 ? "the conflicted item" : "the conflicted items"} separately.`
                      : `Resolve ${manualItems.length === 1 ? "this item" : "these items"} before it can be restored.`}
                  </span>
                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep("history")}>Review conflict</button>
                </div>
              )}
              {!hasResult && approvalItems.length > 0 && (
                <div className="v3-callout v3-sync-review-blocker">
                  <Icon name="help-circle" size={15} />
                  <span>
                    <strong>{hasResult ? `Optional change${approvalItems.length === 1 ? "" : "s"} skipped` : `Optional change${approvalItems.length === 1 ? "" : "s"} not selected`}</strong>
                    {hasResult
                      ? `${approvalItems.map((item) => item.title).join(", ")} ${approvalItems.length === 1 ? "was" : "were"} left unchanged.`
                      : `Apply will skip ${approvalItems.map((item) => item.title).join(", ")} and leave ${approvalItems.length === 1 ? "it" : "them"} unchanged on this machine.`}
                  </span>
                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep(approvalItems[0]?.toolKind === "plugin" ? "plugins" : approvalItems[0]?.toolKind === "custom_skill" ? "skills" : "review")}>Review optional</button>
                </div>
              )}

              {!supportLoading && items.length === 0 && (
                <div className="v3-pull-empty"><Icon name="check-circle" size={16} /><span><strong>Nothing to pull</strong><small>This project already matches the selected generation.</small></span></div>
              )}

              {(hasResult && ((readiness?.issues.length ?? 0) > 0 || !finalReady)) && (
                <section className="v3-pull-section" aria-labelledby="v3-pull-readiness">
                  <div className="v3-pull-section-heading">
                    <span className="v3-pull-section-icon"><Icon name="check-circle" size={16} /></span>
                    <div><h2 id="v3-pull-readiness">Remaining items</h2><p>{readiness?.issues.length ?? 0} item{readiness?.issues.length === 1 ? "" : "s"} still need attention after Pull.</p></div>
                    <button type="button" className="btn btn-ghost" onClick={onRefresh} disabled={busy}><Icon name="refresh" size={13} /> Recheck</button>
                  </div>
                  <div className="v3-readiness-list">
                    {(readiness?.issues ?? []).map((issue) => (
                      <div key={issue.issue_id} className={`severity-${issue.severity ?? "info"}`}>
                        <Icon name={issue.severity === "error" ? "alert-triangle" : "check-circle"} size={14} />
                        <span><strong>{issue.title}</strong><small>{issue.detail}</small></span>
                        {issue.provider && <em>{providerLabel(issue.provider)}</em>}
                      </div>
                    ))}
                    {(readiness?.issues.length ?? 0) === 0 && (
                      <div className="severity-info"><Icon name="refresh" size={14} /><span><strong>Readiness is not confirmed</strong><small>Recheck after resolving the message above or selecting remaining setup items.</small></span></div>
                    )}
                  </div>
                </section>
              )}

              {error && <div className="v3-callout error v3-pull-error" role="alert"><Icon name="alert-triangle" size={15} /> {error}</div>}
              <ResultDetails restoreResult={restoreResult} dependencyResult={dependencyResult} />
            </div>
          )}
        </div>
      </div>

      <footer className={embedded ? "v3-pull-apply-bar v3-inline-action-footer v3-sync-review-footer" : "v3-pull-apply-bar v3-sync-review-footer"}>
        {(hasResult || !embedded) && (
          <span>
            {hasResult && <strong>{outcomeTitle}</strong>}
            {!embedded && <small>{hasResult ? "Review the result above or return to the project." : "Recommended safe changes are selected by default."}</small>}
          </span>
        )}
        {stepIndex > 0 && activeStep !== "review" && <button type="button" className="btn btn-ghost" onClick={goBack} disabled={busy}>Back</button>}
        {activeStep !== "review" ? (
          <button type="button" className="btn btn-primary" onClick={goNext} disabled={busy}>
            Next: {STEP_LABELS[reviewSteps[Math.min(stepIndex + 1, reviewSteps.length - 1)]]}
            <Icon name="chevron-right" size={14} />
          </button>
        ) : (
          <>
            {stepIndex > 0 && <button type="button" className="btn btn-ghost" onClick={goBack} disabled={busy}>Back</button>}
            <button
              type="button"
              className="btn btn-primary v3-pull-apply-button"
              disabled={busy || (!reviewCanClose
                && selectedCount === 0
                && pendingProjectContentDecisionCount === 0
                && keptLocalConversationItems.length === 0)}
              onClick={reviewCanClose ? onBack : () => onApply(selection)}
            >
              {busy && <Icon name="refresh" size={16} className="icon-spin" />}
              {buttonLabel}
            </button>
          </>
        )}
      </footer>
    </ReviewContainer>
  );
}
