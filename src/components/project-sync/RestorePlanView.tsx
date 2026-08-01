import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type {
  BundleReadiness,
  ConversationPlanAction,
  ConversationResolutionSelection,
  LocalProjectSummary,
  ProjectBinding,
  RestorePlan,
  RestoreResult,
} from "../../types";
import Icon from "../Icons";
import ProjectChatHistoryPage from "./ProjectChatHistoryPage";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "./ProjectFilesReviewPage";
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
  onVerifyCodexFork?: (operationId: string, providerThreadId: string | null) => void;
  onCancelCodexFork?: (operationId: string) => void;
  onRestartCodexApp?: (
    operationId: string,
    appKind: "codex" | "codex_plus_plus",
  ) => void;
  onRefresh: () => void;
  onBack: () => void;
  onUnlinkStorage?: () => void;
}

const STEP_LABELS: Record<SyncReviewStep, string> = {
  history: "Git & sessions",
  project_files: "Project files",
  review: "Review",
};

function phaseIndex(phase: PullApplyPhase): number {
  if (phase === "restoring") return 0;
  if (phase === "verifying") return 1;
  if (phase === "complete") return 2;
  return -1;
}

function PhaseProgress({
  phase,
  ready,
  restoreFailed,
}: {
  phase: PullApplyPhase;
  ready: boolean;
  restoreFailed: boolean;
}) {
  const activeIndex = phaseIndex(phase);
  const steps = [
    { label: "Restore project data", icon: "file" as const },
    { label: "Verify project readiness", icon: "refresh" as const },
  ];
  return (
    <section className="sync-pull-progress" aria-label="Pull progress" aria-live="polite">
      {steps.map((step, index) => {
        const current = activeIndex === index;
        const warning = phase === "complete" && (
          (index === 0 && restoreFailed) ||
          (index === 1 && !ready)
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

function nameStatusLabel(status: string | null | undefined): string {
  if (status === "applied") return "applied";
  if (status === "user_override") return "kept user override";
  if (status === "unchanged") return "unchanged";
  if (status === "deferred") return "deferred";
  return "failed";
}

function ResultDetails({
  restoreResult,
}: {
  restoreResult: RestoreResult | null;
}) {
  const results = [restoreResult].filter((result): result is RestoreResult => result !== null);
  if (results.length === 0) return null;
  return (
    <details className="sync-pull-result-details">
      <summary>Technical results</summary>
      {results.map((result, index) => (
        <div key={`${result.message}-${index}`}>
          <strong>{result.message}</strong>
          {"codex_divergences" in result && (result.codex_divergences ?? []).map((divergence) => (
            <section key={divergence.action_id} className="sync-divergence-result">
              <strong>
                {divergence.resolution === "preserve_local_with_native_fork"
                  ? "Both Codex branches preserved"
                  : "Cloud branch restored"}
              </strong>
              <dl>
                <div>
                  <dt>Original</dt>
                  <dd><code>{divergence.original_thread_id}</code> → <code>{divergence.original_provider_thread_id}</code></dd>
                </div>
                {divergence.preserved_thread_id && divergence.preserved_provider_thread_id && (
                  <div>
                    <dt>Preserved local</dt>
                    <dd><code>{divergence.preserved_thread_id}</code> → <code>{divergence.preserved_provider_thread_id}</code></dd>
                  </div>
                )}
                {divergence.executor && (
                  <div><dt>Fork method</dt><dd>{executorLabel(divergence.executor)}</dd></div>
                )}
                {divergence.desired_fork_name && (
                  <div>
                    <dt>Fork name</dt>
                    <dd>{divergence.desired_fork_name} · {nameStatusLabel(divergence.fork_name_status)}</dd>
                  </div>
                )}
                <div>
                  <dt>Original name</dt>
                  <dd>{divergence.desired_original_name ?? "Retained current title"} · {nameStatusLabel(divergence.original_name_status)}</dd>
                </div>
              </dl>
              {divergence.inventory_refresh_status === "deferred" && (
                <small>Codex names and conversation discovery are finishing in the background.</small>
              )}
              {divergence.inventory_refresh_status === "failed" && (
                <small>Codex conversation discovery refresh failed. The Pull result is safe; retry by reopening Codex or rechecking later.</small>
              )}
            </section>
          ))}
          {(result.failed_actions ?? []).map((failure) => (
            <span key={failure.action_id}><code>{failure.display_name ?? failure.action_id}</code>{failure.message}</span>
          ))}
        </div>
      ))}
    </details>
  );
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

function executorLabel(executor: string): string {
  if (executor === "app_server_stdio") return "app-server";
  if (executor === "native_pty") return "native PTY";
  return "manual";
}

function DivergenceResolutionCards({
  actions,
  selectedActionIds,
  resolutions,
  disabled,
  onChange,
}: {
  actions: ConversationPlanAction[];
  selectedActionIds: ReadonlySet<string>;
  resolutions: ReadonlyMap<string, ConversationResolutionSelection>;
  disabled: boolean;
  onChange: (selection: ConversationResolutionSelection) => void;
}) {
  if (actions.length === 0) return null;
  return (
    <div className="sync-divergence-resolutions">
      {actions.map((action) => {
        const plan = action.divergence;
        if (!plan) return null;
        const selection = resolutions.get(action.action_id) ?? {
          action_id: action.action_id,
          resolution: plan.recommended_resolution,
          destructive_approved: false,
        };
        const selected = selectedActionIds.has(action.action_id);
        const controlsDisabled = disabled || !selected;
        const conflictTitle = plan.pinned_local_title
          ?? plan.pinned_cloud_title
          ?? plan.source_provider_thread_id;
        const preserved = plan.local_branch_preserved;
        return (
          <fieldset key={action.action_id} className="sync-divergence-card" disabled={controlsDisabled}>
            <legend>Conversation conflict — {conflictTitle}</legend>
            <label className="sync-divergence-option">
              <input
                type="radio"
                name={`divergence-${action.action_id}`}
                checked={selection.resolution === "preserve_local_with_native_fork"}
                onChange={() => onChange({
                  action_id: action.action_id,
                  resolution: "preserve_local_with_native_fork",
                  destructive_approved: false,
                })}
              />
              <span>
                {preserved ? (
                  <>
                    <strong>Fork again</strong>
                    <small>Keep another copy of the local branch as a new Codex fork before restoring the cloud conversation.</small>
                  </>
                ) : (
                  <>
                    <strong>Keep both branches <em>Recommended</em></strong>
                    <small>Preserve the current local conversation as a Codex fork, then restore the cloud conversation into the original thread.</small>
                  </>
                )}
              </span>
            </label>
            <label className="sync-divergence-option">
              <input
                type="radio"
                name={`divergence-${action.action_id}`}
                checked={selection.resolution === "use_cloud_only"}
                onChange={() => onChange({
                  action_id: action.action_id,
                  resolution: "use_cloud_only",
                  destructive_approved: false,
                })}
              />
              <span>
                {preserved ? (
                  <>
                    <strong>Adopt cloud version <em>Recommended</em></strong>
                    <small>Your local branch is already preserved in a Codex fork. Restore the cloud conversation into the original thread; nothing is lost.</small>
                  </>
                ) : (
                  <>
                    <strong>Use cloud version only</strong>
                    <small>Replace the original local conversation. The divergent local branch will not remain as a Codex conversation.</small>
                  </>
                )}
              </span>
            </label>
            {selection.resolution === "use_cloud_only" && !preserved && (
              <label className="sync-divergence-confirm">
                <input
                  type="checkbox"
                  checked={selection.destructive_approved}
                  onChange={(event) => onChange({
                    ...selection,
                    destructive_approved: event.target.checked,
                  })}
                />
                I understand the divergent local branch will not remain available as a Codex conversation.
              </label>
            )}
            <dl className="sync-divergence-preview">
              <div><dt>Fork method</dt><dd>{plan.available_executors.map(executorLabel).join(" → ")}</dd></div>
              <div><dt>Fork name</dt><dd>{plan.desired_fork_name}</dd></div>
              <div><dt>Original</dt><dd>{plan.desired_original_name ?? "Retain current title"}</dd></div>
            </dl>
            {!selected && <p className="sync-divergence-unselected">Select this conversation to choose a resolution.</p>}
          </fieldset>
        );
      })}
    </div>
  );
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
        const conversationAction = item.conversationActions[0]?.action;
        const status: string | null = conversationAction === "no_op"
          ? "Already up to date"
          : conversationAction === "require_pull"
            ? "Pull required"
            : conversationAction === "blocked"
              ? "Can't sync"
              : conversationAction === "store_only" && !selectable
                ? "Backup only"
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
  onVerifyCodexFork,
  onCancelCodexFork,
  onRestartCodexApp,
  onRefresh,
  onBack,
  onUnlinkStorage,
}: Props) {
  const items = useMemo(() => buildPullReviewItems(plan), [plan]);
  const divergenceActions = useMemo(
    () => (plan.conversation_actions ?? []).filter((action) => action.action === "resolve_divergence"),
    [plan],
  );
  const [conversationResolutions, setConversationResolutions] = useState<
    Map<string, ConversationResolutionSelection>
  >(() => new Map(divergenceActions.map((action) => [
    action.action_id,
    {
      action_id: action.action_id,
      resolution: action.divergence?.recommended_resolution ?? "preserve_local_with_native_fork",
      destructive_approved: false,
    },
  ])));
  const [manualForkProviderThreadId, setManualForkProviderThreadId] = useState("");
  const [restartChatsComplete, setRestartChatsComplete] = useState(false);
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
  const projectItems = useMemo(() => items.filter((item) => item.category === "project_data"), [items]);
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
    setConversationResolutions((current) => new Map(divergenceActions.map((action) => [
      action.action_id,
      current.get(action.action_id) ?? {
        action_id: action.action_id,
        resolution: action.divergence?.recommended_resolution ?? "preserve_local_with_native_fork",
        destructive_approved: false,
      },
    ])));
  }, [divergenceActions]);

  useEffect(() => {
    setManualForkProviderThreadId("");
  }, [restoreResult?.codex_fork_checkpoints?.[0]?.operation_id]);

  useEffect(() => {
    setRestartChatsComplete(false);
  }, [restoreResult?.codex_app_refresh?.operation_id]);

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
      conversationResolutions,
    ),
    [items, selected, completedActionIds, completedResourceIds, conversationResolutions],
  );
  const selectedCount = selection.resourceIds.length;
  const selectedRestoreActionIds = new Set(selection.restoreActionIds);
  const preservedDivergenceActionIds = useMemo(() => new Set(
    divergenceActions
      .filter((action) => action.divergence?.local_branch_preserved)
      .map((action) => action.action_id),
  ), [divergenceActions]);
  const hasUnconfirmedCloudOnly = selection.conversationResolutions.some((resolution) => (
    resolution.resolution === "use_cloud_only"
    && !resolution.destructive_approved
    && !preservedDivergenceActionIds.has(resolution.action_id)
  ));
  // Manual-fork checkpoints are resolved from the Activity page; the Pull
  // review drives only the checkpoints its own plan created.
  const forkCheckpoints = (restoreResult?.codex_fork_checkpoints ?? []).filter(
    (checkpoint) => checkpoint.origin !== "manual",
  );
  const forkCheckpoint = forkCheckpoints[0] ?? null;
  const appRefresh = restoreResult?.codex_app_refresh ?? null;
  const forkCheckpointActive = Boolean(
    forkCheckpoint && restoreResult?.status !== "complete",
  );
  const pendingProjectContentDecisionCount = projectContentItems.filter((item) => (
    pendingIds(item, completedActionIds).length > 0 && !completedResourceIds.has(item.resourceId)
  )).length;
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
    if (phase !== "idle" || hasResult) setActiveStep("review");
  }, [hasResult, phase]);

  useEffect(() => {
    if (!reviewSteps.includes(activeStep)) setActiveStep("review");
  }, [activeStep, reviewSteps]);

  const failureCount = restoreResult?.failed_actions?.length ?? 0;
  const unchangedCount = manualItems.length;
  const finalReady = hasResult && !error && failureCount === 0 && readiness?.state === "ready";
  const reviewCanClose = !forkCheckpointActive && hasResult && (
    finalReady || (selectedCount === 0 && pendingProjectContentDecisionCount === 0)
  );
  const completedProjectItems = projectItems.filter((item) => completedResourceIds.has(item.resourceId));
  const failedProjectItems = projectItems.filter((item) => failedResourceIds.has(item.resourceId));
  const outcomeTitle = forkCheckpointActive
    ? forkCheckpoint?.status === "awaiting_manual_fork"
      ? "Waiting for a Codex fork"
      : "Fork recovery needs verification"
    : error && hasResult
    ? "Pull needs attention"
    : failureCount > 0
      ? `${failureCount} change${failureCount === 1 ? "" : "s"} need attention`
      : unchangedCount > 0
        ? `Project restored; ${unchangedCount} item${unchangedCount === 1 ? "" : "s"} left unchanged`
        : readiness?.state === "ready"
          ? "Pull complete"
          : "Project restored; setup needs attention";
  const buttonLabel = busy
    ? phase === "verifying"
      ? "Verifying readiness…"
      : "Restoring selected changes…"
    : forkCheckpointActive
      ? "Complete the fork checkpoint above"
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
  const recommendedIds = new Set(items.filter(defaultSelected).map((item) => item.resourceId));
  const stepItems = activeStep === "history"
    ? conversationItems
    : activeStep === "project_files"
      ? projectContentItems
      : items;
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
    if (activeStep === "review") return new Set();
    const next = new Set(current);
    for (const item of stepItems) next.delete(item.resourceId);
    return next;
  });
  const selectedIn = (chosen: PullReviewItem[]) => chosen.filter((item) => selected.has(item.resourceId)).length;
  const counts = {
    history: selectedIn(conversationItems),
    project_files: selectedIn(projectContentItems),
    review: selectedCount,
  };
  const warningSteps = new Set<SyncReviewStep>();
  for (const item of approvalItems) {
    if (itemKindLabel(item) === "Conversation") warningSteps.add("history");
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
  if (failureCount > 0) warningSteps.add("review");
  if (projectContentItems.length > 0 && !projectContentEligible) {
    warningSteps.add("project_files");
    warningSteps.add("review");
  }
  const conversationIds = pullSelectableResourceIds(
    conversationItems,
    completedActionIds,
    completedResourceIds,
  );
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
      className={embedded ? "sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" : "sync-main sync-pull-review-workspace sync-sync-review-workspace"}
      aria-labelledby="sync-pull-review-title"
    >
      <header className={embedded ? "sync-inline-action-header sync-inline-pull-header sync-sync-review-header" : "sync-pull-review-header sync-sync-review-header"}>
        {!embedded && (
          <button type="button" className="btn btn-ghost sync-pull-back" onClick={onBack} disabled={busy}>
            <Icon name="chevron-left" size={15} /> Back to {projectName}
          </button>
        )}
        <div className={embedded ? "sync-inline-action-heading" : "sync-pull-review-heading"}>
          <div className={embedded ? "sync-sync-review-title" : undefined}>
            {embedded
              ? <h2 id="sync-pull-review-title">Pull review</h2>
              : <h1 id="sync-pull-review-title">Restore {projectName}</h1>}
            {!embedded && (
                <p className="sync-pull-review-meta">
                  <span>{`Choose changes from ${storageName || "storage"} to apply to this machine.`}</span>
                </p>
              )}
          </div>
        </div>
        <div className="sync-push-resource-actions">
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action has-count"
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
            className="btn btn-ghost sync-sync-review-icon-action"
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
              className="btn btn-ghost sync-inline-action-close"
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
        className={embedded ? "sync-pull-review-content sync-inline-action-content sync-sync-review-content" : "sync-pull-review-content sync-sync-review-content"}
        role="tabpanel"
        aria-labelledby={`sync-review-${activeStep}-tab`}
      >
        <div
          className="sync-inline-action-scroll sync-sync-review-scroll"
        >
          {activeStep === "history" && (
            <DivergenceResolutionCards
              actions={divergenceActions}
              selectedActionIds={selectedRestoreActionIds}
              resolutions={conversationResolutions}
              disabled={busy || finalReady || forkCheckpointActive}
              onChange={(resolution) => setConversationResolutions((current) => {
                const next = new Map(current);
                next.set(resolution.action_id, resolution);
                return next;
              })}
            />
          )}
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
              selectionDisabled={busy || finalReady}
              onToggleResource={toggle}
              onUnlinkStorage={onUnlinkStorage}
              continuityActionBusy={busy || finalReady}
            />
          ) : activeStep === "history" && conversationItems.length > 0 ? (
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
          ) : activeStep === "history" ? (
            <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>No session changes</strong><small>Sessions already match this storage.</small></span></div>
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
            <div className="sync-sync-review-summary">
              {phase !== "idle" && (
                <PhaseProgress phase={phase} ready={finalReady} restoreFailed={failedProjectItems.length > 0} />
              )}
              {forkCheckpointActive && forkCheckpoint && (
                <section className="sync-fork-checkpoint" aria-live="polite">
                  <div>
                    <Icon name="git-branch" size={18} />
                    <span>
                      <strong>{forkCheckpoint.status === "awaiting_manual_fork" ? "Create the local fork in Codex" : "Verify the recovered Codex fork"}</strong>
                      {forkCheckpoints.length > 1 && (
                        <small>Fork checkpoint 1 of {forkCheckpoints.length} — the rest of this Pull is already applied.</small>
                      )}
                      <small>{forkCheckpoint.message}</small>
                      {forkCheckpoint.status === "awaiting_manual_fork" && (
                        <small>Finish any active turn first. Fork the source thread without sending a new message in the fork.</small>
                      )}
                    </span>
                  </div>
                  <dl>
                    <div><dt>Source thread</dt><dd><code>{forkCheckpoint.source_provider_thread_id}</code></dd></div>
                    <div><dt>Suggested name</dt><dd>{forkCheckpoint.suggested_fork_name}</dd></div>
                  </dl>
                  <label>
                    Forked thread ID <span>(optional; leave blank for a bounded scan)</span>
                    <input
                      type="text"
                      value={manualForkProviderThreadId}
                      disabled={busy}
                      onChange={(event) => setManualForkProviderThreadId(event.target.value)}
                      placeholder="019f…"
                    />
                  </label>
                  <div>
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={busy || !onVerifyCodexFork}
                      onClick={() => onVerifyCodexFork?.(
                        forkCheckpoint.operation_id,
                        manualForkProviderThreadId.trim() || null,
                      )}
                    >
                      Verify fork
                    </button>
                    <button
                      type="button"
                      className="btn btn-ghost"
                      disabled={busy || !onCancelCodexFork || forkCheckpoint.status !== "awaiting_manual_fork"}
                      onClick={() => onCancelCodexFork?.(forkCheckpoint.operation_id)}
                    >
                      Cancel
                    </button>
                  </div>
                </section>
              )}
              {!hasResult && hasUnconfirmedCloudOnly && (
                <div className="sync-callout warning" role="alert">
                  <Icon name="alert-triangle" size={15} />
                  Confirm that the divergent local Codex branch may be discarded before applying.
                </div>
              )}
              {hasResult && (
                <section className={`sync-pull-outcome${finalReady ? " success" : " warning"}`} aria-live="polite">
                  <Icon name={finalReady ? "check-circle" : "alert-triangle"} size={20} />
                  <div>
                    <h2>{outcomeTitle}</h2>
                    <ul>
                      {completedProjectItems.length > 0 && <li>{completedProjectItems.length} project data change{completedProjectItems.length === 1 ? "" : "s"} restored</li>}
                      {keptLocalConversationItems.length > 0 && <li>{keptLocalConversationItems.length} conversation{keptLocalConversationItems.length === 1 ? "" : "s"} kept local</li>}
                      {manualItems.length > 0 && <li>{manualItems.length} conflict{manualItems.length === 1 ? "" : "s"} left unchanged for manual resolution</li>}
                      {finalReady && <li>Project is ready</li>}
                    </ul>
                  </div>
                </section>
              )}
              {restoreResult?.status === "complete" && appRefresh && (
                <section className="sync-app-refresh" aria-live="polite">
                  <div>
                    <Icon name="refresh" size={18} />
                    <span>
                      <strong>Refresh {appRefresh.app_name}</strong>
                      <small>{appRefresh.message}</small>
                    </span>
                  </div>
                  {appRefresh.state === "restart_available" ? (
                    <>
                      <label>
                        <input
                          type="checkbox"
                          checked={restartChatsComplete}
                          disabled={busy}
                          onChange={(event) => setRestartChatsComplete(event.target.checked)}
                        />
                        My active Codex chats are complete.
                      </label>
                      <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={busy || !restartChatsComplete || !onRestartCodexApp}
                        onClick={() => onRestartCodexApp?.(
                          appRefresh.operation_id,
                          appRefresh.app_kind,
                        )}
                      >
                        Restart {appRefresh.app_name}
                      </button>
                    </>
                  ) : (
                    <small>Close and reopen the app manually. Pull is already complete.</small>
                  )}
                </section>
              )}

              <div className="sync-sync-review-summary-meta"><Icon name="download" size={14} /><span>{embedded ? `Generation ${plan.generation}` : `${storageName || "Storage"} · Generation ${plan.generation}`}</span></div>
              <button type="button" onClick={() => setActiveStep("history")}>
                <span><Icon name="git-branch" size={15} /><strong>Sessions</strong></span>
                <span>{counts.history} selected<Icon name="chevron-right" size={13} /></span>
              </button>
              {(otherItems.length > 0 || supportLoading) && (
                <details className="sync-sync-review-other">
                  <summary>
                    <span><Icon name="settings" size={15} /><strong>Agent setup & tools</strong></span>
                    <span>{selectedIn(otherItems)} selected<Icon name="chevron-right" size={13} /></span>
                  </summary>
                  <ItemList items={otherItems} selected={selected} completedActionIds={completedActionIds} completedResourceIds={completedResourceIds} failedResourceIds={failedResourceIds} phase={phase} disabled={busy || finalReady} onToggle={toggle} />
                  {supportLoading && <div className="sync-pull-support-loading"><Icon name="refresh" className="icon-spin" size={14} /> Checking readiness…</div>}
                </details>
              )}

              {!hasResult && plan.remote_continuity?.state === "replaced" && (
                <div className="sync-callout warning sync-sync-review-blocker">
                  <Icon name="alert-triangle" size={15} />
                  <span>
                    <strong>Storage history changed</strong>
                    {`The sync history in ${storageName || "this storage"} is different from the one Mallard last synced for this project (generation ${plan.remote_continuity.base_generation}). This usually means Mallard's sync data in that storage was deleted and created again. Applying this Pull adopts the replacement history. Mallard backs up and checks local content before changing it, and local-only items can be pushed afterward. If this change is unexpected, unlink the storage instead; unlinking does not delete files.`}
                  </span>
                  {onUnlinkStorage && (
                    <div className="sync-sync-review-recovery-actions">
                      <button type="button" className="btn btn-ghost danger" onClick={onUnlinkStorage} disabled={busy || finalReady}>
                        Unlink storage
                      </button>
                    </div>
                  )}
                </div>
              )}
              {!hasResult && keptLocalConversationItems.length > 0 && (
                <div className="sync-callout warning sync-sync-review-blocker">
                  <Icon name="alert-triangle" size={15} />
                  <span>
                    <strong>{keptLocalConversationItems.length === 1 ? "1 conversation differs" : `${keptLocalConversationItems.length} conversations differ`}</strong>
                    {`Apply will keep the local ${keptLocalConversationItems.length === 1 ? "version" : "versions"} and establish a comparison baseline for ${storageName || "storage"}. ${keptLocalConversationItems.length === 1 ? "It" : "They"} can be pushed later.`}
                  </span>
                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep("history")}>Review local</button>
                </div>
              )}
              {!hasResult && manualItems.length > 0 && (
                <div className="sync-callout warning sync-sync-review-blocker">
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
                <div className="sync-callout sync-sync-review-blocker">
                  <Icon name="help-circle" size={15} />
                  <span>
                    <strong>{hasResult ? `Optional change${approvalItems.length === 1 ? "" : "s"} skipped` : `Optional change${approvalItems.length === 1 ? "" : "s"} not selected`}</strong>
                    {hasResult
                      ? `${approvalItems.map((item) => item.title).join(", ")} ${approvalItems.length === 1 ? "was" : "were"} left unchanged.`
                      : `Apply will skip ${approvalItems.map((item) => item.title).join(", ")} and leave ${approvalItems.length === 1 ? "it" : "them"} unchanged on this machine.`}
                  </span>
                  <button type="button" className="btn btn-ghost" onClick={() => setActiveStep("review")}>Review optional</button>
                </div>
              )}

              {!supportLoading && items.length === 0 && (
                <div className="sync-pull-empty"><Icon name="check-circle" size={16} /><span><strong>Nothing to pull</strong><small>This project already matches the selected generation.</small></span></div>
              )}

              {(hasResult && ((readiness?.issues.length ?? 0) > 0 || !finalReady)) && (
                <section className="sync-pull-section" aria-labelledby="sync-pull-readiness">
                  <div className="sync-pull-section-heading">
                    <span className="sync-pull-section-icon"><Icon name="check-circle" size={16} /></span>
                    <div><h2 id="sync-pull-readiness">Remaining items</h2><p>{readiness?.issues.length ?? 0} item{readiness?.issues.length === 1 ? "" : "s"} still need attention after Pull.</p></div>
                    <button type="button" className="btn btn-ghost" onClick={onRefresh} disabled={busy}><Icon name="refresh" size={13} /> Recheck</button>
                  </div>
                  <div className="sync-readiness-list">
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

              {error && <div className="sync-callout error sync-pull-error" role="alert"><Icon name="alert-triangle" size={15} /> {error}</div>}
              <ResultDetails restoreResult={restoreResult} />
            </div>
          )}
        </div>
      </div>

      <footer className={embedded ? "sync-pull-apply-bar sync-inline-action-footer sync-sync-review-footer" : "sync-pull-apply-bar sync-sync-review-footer"}>
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
              className="btn btn-primary sync-pull-apply-button"
              disabled={busy || forkCheckpointActive || hasUnconfirmedCloudOnly || (!reviewCanClose
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
