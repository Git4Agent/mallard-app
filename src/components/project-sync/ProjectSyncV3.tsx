import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import type {
  ActivityLogLevel,
  ActivityLogType,
  AppTheme,
  BundleRecipe,
  BundleReadiness,
  BundleSnapshotSummary,
  CapabilityStatusReport,
  CodexConversationPathAudit,
  DependencyPlan,
  DependencyResult,
  HostIdentityStatus,
  LocalProjectRegistration,
  LocalProjectSummary,
  LogLine,
  ProjectBinding,
  ProjectContentInventory,
  ProjectDetail,
  ProjectProvider,
  ProjectStorageLink,
  ProviderProfile,
  ProviderProfileSummary,
  ResourceInventory as ResourceInventoryModel,
  RestorePlan,
  RestoreResult,
  SetupDraftSummary,
  StorageConfigV3,
  SyncConfigV3,
  ThreadSyncComparison,
} from "../../types";
import { PROJECT_SYNC_SCHEMA } from "../../types";
import Icon from "../Icons";
import LogPanel, { ACTIVITY_LOG_TYPES } from "../LogPanel";
import BundleConnectionDialog from "./BundleConnectionDialog";
import LogManagerDialog from "./LogManagerDialog";
import ProjectBindingEditor, { type ProjectBindingDraft } from "./ProjectBindingEditor";
import ProjectLinksWorkspace from "./ProjectLinksWorkspace";
import PushResourceWorkspace, {
  includeRequiredProjectContentDirectories,
  nextPushReviewStep,
  preserveSyncedPushRecipeEntries,
  pushReviewBlockingCount,
  recommendedPushSelection,
  sanitizePushSelection,
} from "./PushResourceWorkspace";
import ProjectSetupWorkspace, { type SetupCompletion } from "./ProjectSetupWorkspace";
import ProjectSidebar from "./ProjectSidebar";
import RestorePlanView from "./RestorePlanView";
import { syncReviewSteps, type SyncReviewStep } from "./SyncReviewTabs";
import { projectSyncApi } from "./api";
import {
  applyPullReview,
  beginPullReview,
  loadPullReviewSupport,
  type PullApplyPhase,
  type PullReviewSelection,
} from "./pullReviewFlow";
import {
  errorMessage,
  inventoryResources,
  projectLabel,
  recipeSelection,
  recipeWithSelection,
} from "./model";

interface Props {
  theme: AppTheme;
  onThemeChange: (theme: AppTheme) => void;
}

const EMPTY_CONFIG: SyncConfigV3 = {
  schema: PROJECT_SYNC_SCHEMA,
  revision: 0,
  storages: [],
  projects: [],
  links: [],
};

const PROJECT_SIDEBAR_WIDTH_KEY = "agent-sync.project-sidebar-width";
const PROJECT_SIDEBAR_VISIBILITY_KEY = "mallard.project-sidebar-visible";
const ACTIVE_STORAGE_BY_PROJECT_KEY = "mallard.active-storage-by-project";
const DEFAULT_PROJECT_SIDEBAR_WIDTH = 318;
const MIN_PROJECT_SIDEBAR_WIDTH = 220;
const MAX_PROJECT_SIDEBAR_WIDTH = 560;
const FOCUS_RECONCILE_MIN_AWAY_MS = 5000;

function clampSidebarWidth(width: number, maxWidth = MAX_PROJECT_SIDEBAR_WIDTH): number {
  return Math.min(maxWidth, Math.max(MIN_PROJECT_SIDEBAR_WIDTH, width));
}

function availableProjectSidebarWidth(): number {
  return Math.max(
    MIN_PROJECT_SIDEBAR_WIDTH,
    Math.min(MAX_PROJECT_SIDEBAR_WIDTH, window.innerWidth - 420),
  );
}

function storedSidebarWidth(): number {
  try {
    const value = Number.parseInt(window.localStorage.getItem(PROJECT_SIDEBAR_WIDTH_KEY) ?? "", 10);
    return Number.isFinite(value)
      ? clampSidebarWidth(value, window.innerWidth > 720 ? availableProjectSidebarWidth() : MAX_PROJECT_SIDEBAR_WIDTH)
      : DEFAULT_PROJECT_SIDEBAR_WIDTH;
  } catch {
    return DEFAULT_PROJECT_SIDEBAR_WIDTH;
  }
}

function storedSidebarVisibility(): boolean {
  if (typeof window === "undefined") return true;
  try {
    return window.localStorage.getItem(PROJECT_SIDEBAR_VISIBILITY_KEY) !== "hidden";
  } catch {
    return true;
  }
}

function sidebarShortcutLabel(): string {
  if (typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.platform)) return "⌘B";
  return "Ctrl+B";
}

function storedActiveStorages(): Record<string, string> {
  try {
    const value = JSON.parse(window.localStorage.getItem(ACTIVE_STORAGE_BY_PROJECT_KEY) ?? "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter((entry): entry is [string, string] => (
      typeof entry[0] === "string" && typeof entry[1] === "string"
    )));
  } catch {
    return {};
  }
}

interface PendingBundleConnection {
  projectId: string;
  storageId: string;
  matches: BundleSnapshotSummary[];
  reason: "link" | "missing";
}

interface PendingPush {
  projectId: string;
  storageId: string;
  projectName: string;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  storage: StorageConfigV3;
  inventory: ResourceInventoryModel;
  savedRecipe: BundleRecipe | null;
  projectDefaults: Set<string>;
  threadComparison?: ThreadSyncComparison | null;
  capabilityReport?: CapabilityStatusReport | null;
  projectContentInventory: ProjectContentInventory | null;
  projectContentScanned: boolean;
  showProjectFiles: boolean;
}

function pushResources(pending: PendingPush): ReturnType<typeof inventoryResources> {
  const resources = inventoryResources(pending.inventory);
  const byId = new Map(resources.map((resource) => [resource.resource_id, resource]));
  for (const entry of pending.projectContentInventory?.entries ?? []) {
    byId.set(entry.descriptor.resource_id, {
      ...entry.descriptor,
      category: "project_files",
      logical_paths: [entry.logical_path],
      default_selected: entry.selected_in_recipe || entry.selected_after_scan,
      selected_by_default: entry.selected_in_recipe || entry.selected_after_scan,
      blocked_reason: entry.blocked_reason,
      description: entry.relative_path,
    });
  }
  return [...byId.values()];
}

function upsertProject(
  projects: LocalProjectRegistration[],
  project: LocalProjectRegistration,
): LocalProjectRegistration[] {
  const found = projects.some((candidate) => candidate.local_project_id === project.local_project_id);
  return found
    ? projects.map((candidate) => candidate.local_project_id === project.local_project_id ? project : candidate)
    : [...projects, project];
}

function upsertBinding(bindings: ProjectBinding[], binding: ProjectBinding): ProjectBinding[] {
  const found = bindings.some((candidate) => candidate.local_project_id === binding.local_project_id);
  return found
    ? bindings.map((candidate) => candidate.local_project_id === binding.local_project_id ? binding : candidate)
    : [...bindings, binding];
}

function defaultProfileIds(): Partial<Record<ProjectProvider, string>> {
  return {};
}

function logMatchesFilters(
  line: LogLine,
  types: readonly ActivityLogType[],
  level: ActivityLogLevel | "all",
  search = "",
): boolean {
  const needle = search.trim().toLocaleLowerCase();
  return types.includes(line.type ?? "system")
    && (level === "all" || line.level === level)
    && (!needle
      || line.message.toLocaleLowerCase().includes(needle)
      || !!line.event?.toLocaleLowerCase().includes(needle));
}

function mergeLogLines(...groups: LogLine[][]): LogLine[] {
  const entries = new Map<string, LogLine>();
  for (const line of groups.flat()) {
    const key = line.id ?? `${line.ts}:${line.level}:${line.type ?? "system"}:${line.message}`;
    entries.set(key, line);
  }
  return [...entries.values()].sort((left, right) => (
    left.ts - right.ts || (left.id ?? "").localeCompare(right.id ?? "")
  ));
}

export default function ProjectSyncV3({ theme, onThemeChange }: Props) {
  const [config, setConfig] = useState<SyncConfigV3>(EMPTY_CONFIG);
  const [registrations, setRegistrations] = useState<LocalProjectRegistration[]>([]);
  const [repositoryKinds, setRepositoryKinds] = useState<Record<string, boolean>>({});
  const [bindings, setBindings] = useState<ProjectBinding[]>([]);
  const [profiles, setProfiles] = useState<ProviderProfileSummary[]>([]);
  const [conversationPathAudits, setConversationPathAudits] = useState<Record<string, CodexConversationPathAudit>>({});
  const [conversationPathAuditErrors, setConversationPathAuditErrors] = useState<Record<string, string>>({});
  const [conversationPathAuditLoading, setConversationPathAuditLoading] = useState(true);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [inventory, setInventory] = useState<ResourceInventoryModel | null>(null);
  const [readiness, setReadiness] = useState<BundleReadiness | null>(null);
  const [activeStorageByProject, setActiveStorageByProject] = useState<Record<string, string>>(storedActiveStorages);
  const [activeStorageSettingsId, setActiveStorageSettingsId] = useState<string | null>(null);
  const [storageEditorRequest, setStorageEditorRequest] = useState<
    | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
    | { mode: "create"; storageKind: "local" | "s3"; requestId: number }
    | { mode: "close"; requestId: number }
    | null
  >(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [backendError, setBackendError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [hostIdentityStatus, setHostIdentityStatus] = useState<HostIdentityStatus | null>(null);
  const [hostIdentityShellReady, setHostIdentityShellReady] = useState(false);
  const [hostIdentityBusy, setHostIdentityBusy] = useState(false);
  const [hostIdentityError, setHostIdentityError] = useState<string | null>(null);
  const [pendingBundleConnection, setPendingBundleConnection] = useState<PendingBundleConnection | null>(null);
  const [pendingPush, setPendingPush] = useState<PendingPush | null>(null);
  const [pushSelected, setPushSelected] = useState<Set<string>>(new Set());
  const [pushProjectContentRemovals, setPushProjectContentRemovals] = useState<Set<string>>(new Set());
  const [pushWarningAcknowledgements, setPushWarningAcknowledgements] = useState<Set<string>>(new Set());
  const [projectContentLoading, setProjectContentLoading] = useState(false);
  const [pushError, setPushError] = useState<string | null>(null);
  const [pushReviewStep, setPushReviewStep] = useState<SyncReviewStep>("history");
  const [logLines, setLogLines] = useState<LogLine[]>([]);
  const [logTypeFilters, setLogTypeFilters] = useState<ActivityLogType[]>(() => [...ACTIVITY_LOG_TYPES]);
  const [logLevelFilter, setLogLevelFilter] = useState<ActivityLogLevel | "all">("all");
  const [logSearchInput, setLogSearchInput] = useState("");
  const [logSearch, setLogSearch] = useState("");
  const [logCursor, setLogCursor] = useState<string | null>(null);
  const [logLoading, setLogLoading] = useState(true);
  const [logLoadingOlder, setLogLoadingOlder] = useState(false);
  const [logLoadError, setLogLoadError] = useState<string | null>(null);
  const [logManagerOpen, setLogManagerOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [logScrollToBottomEpoch, setLogScrollToBottomEpoch] = useState(0);
  const [logHeight, setLogHeight] = useState(240);
  const [resizingLog, setResizingLog] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(storedSidebarWidth);
  const [sidebarVisible, setSidebarVisible] = useState(storedSidebarVisibility);
  const [resizingSidebar, setResizingSidebar] = useState(false);
  const [unreadLogs, setUnreadLogs] = useState(0);
  const [appMenuOpen, setAppMenuOpen] = useState(false);
  const appMenuRef = useRef<HTMLDivElement>(null);
  const activityOpenRef = useRef(false);
  const logFiltersRef = useRef({ types: logTypeFilters, level: logLevelFilter, search: logSearch });
  const logRequestRef = useRef(0);
  const sidebarResizeRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null);

  const [setupDrafts, setSetupDrafts] = useState<SetupDraftSummary[]>([]);
  const [setupDraftId, setSetupDraftId] = useState<string | null>(null);
  const [editingBinding, setEditingBinding] = useState<ProjectBindingDraft | null>(null);

  const [restorePlan, setRestorePlan] = useState<RestorePlan | null>(null);
  const [restoreBinding, setRestoreBinding] = useState<ProjectBinding | null>(null);
  const [restoreProjectName, setRestoreProjectName] = useState("project");
  const [dependencyPlan, setDependencyPlan] = useState<DependencyPlan | null>(null);
  const [restoreResult, setRestoreResult] = useState<RestoreResult | null>(null);
  const [historyRefreshEpoch, setHistoryRefreshEpoch] = useState(0);
  const [projectTabResetEpoch, setProjectTabResetEpoch] = useState(0);
  const [dependencyResult, setDependencyResult] = useState<DependencyResult | null>(null);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [pullApplyPhase, setPullApplyPhase] = useState<PullApplyPhase>("idle");
  const [reviewSupportLoading, setReviewSupportLoading] = useState(false);
  const [completedPullActionIds, setCompletedPullActionIds] = useState<Set<string>>(new Set());
  const [completedPullResourceIds, setCompletedPullResourceIds] = useState<Set<string>>(new Set());
  const [failedPullResourceIds, setFailedPullResourceIds] = useState<Set<string>>(new Set());
  const restoreRequest = useRef(0);
  const shellRequest = useRef(0);
  const hostWorkflowGeneration = useRef(0);
  const readyHostIdentity = useRef<{ hostId: string; revision: number } | null>(null);

  const toggleSidebar = useCallback(() => {
    sidebarResizeRef.current = null;
    setResizingSidebar(false);
    setSidebarVisible((current) => !current);
  }, []);

  useEffect(() => {
    activityOpenRef.current = activityOpen;
  }, [activityOpen]);

  useEffect(() => {
    logFiltersRef.current = { types: logTypeFilters, level: logLevelFilter, search: logSearch };
  }, [logLevelFilter, logSearch, logTypeFilters]);

  useEffect(() => {
    const timer = window.setTimeout(() => setLogSearch(logSearchInput), 180);
    return () => window.clearTimeout(timer);
  }, [logSearchInput]);

  useEffect(() => {
    try {
      window.localStorage.setItem(PROJECT_SIDEBAR_WIDTH_KEY, String(sidebarWidth));
    } catch {
      // Persistence is a convenience; resizing still works if storage is unavailable.
    }
  }, [sidebarWidth]);

  useEffect(() => {
    try {
      window.localStorage.setItem(PROJECT_SIDEBAR_VISIBILITY_KEY, sidebarVisible ? "visible" : "hidden");
    } catch {
      // The sidebar can still be toggled for this session when storage is unavailable.
    }
  }, [sidebarVisible]);

  useEffect(() => {
    const handleSidebarShortcut = (event: KeyboardEvent) => {
      if (
        event.repeat
        || event.altKey
        || event.shiftKey
        || (!event.metaKey && !event.ctrlKey)
        || event.key.toLocaleLowerCase() !== "b"
      ) return;
      event.preventDefault();
      toggleSidebar();
    };
    window.addEventListener("keydown", handleSidebarShortcut);
    return () => window.removeEventListener("keydown", handleSidebarShortcut);
  }, [toggleSidebar]);

  useEffect(() => {
    try {
      window.localStorage.setItem(ACTIVE_STORAGE_BY_PROJECT_KEY, JSON.stringify(activeStorageByProject));
    } catch {
      // The selected comparison target remains available for this session.
    }
  }, [activeStorageByProject]);

  useEffect(() => {
    const fitSidebarToWindow = () => {
      if (window.innerWidth <= 720) return;
      setSidebarWidth((current) => clampSidebarWidth(current, availableProjectSidebarWidth()));
    };
    window.addEventListener("resize", fitSidebarToWindow);
    return () => window.removeEventListener("resize", fitSidebarToWindow);
  }, []);

  const loadActivityLogs = useCallback(async (cursor: string | null = null) => {
    const requestId = ++logRequestRef.current;
    const startedAt = Date.now();
    const loadingOlder = cursor !== null;
    if (loadingOlder) {
      setLogLoadingOlder(true);
    } else {
      setLogLoading(true);
      setLogLines([]);
    }
    setLogLoadError(null);
    if (logTypeFilters.length === 0) {
      setLogCursor(null);
      setLogLines([]);
      setLogLoading(false);
      setLogLoadingOlder(false);
      return;
    }
    try {
      const page = await projectSyncApi.queryActivityLogs({
        types: logTypeFilters,
        levels: logLevelFilter === "all" ? [] : [logLevelFilter],
        search: logSearch.trim() || null,
        cursor,
        limit: 500,
      });
      if (requestId !== logRequestRef.current) return;
      setLogCursor(page.next_cursor ?? null);
      setLogLines((current) => {
        const next = loadingOlder
          ? mergeLogLines(page.entries, current)
          : mergeLogLines(
            page.entries,
            current.filter((line) => line.ts >= startedAt
              && logMatchesFilters(line, logTypeFilters, logLevelFilter, logSearch)),
          );
        return next.slice(-2_000);
      });
    } catch (reason) {
      if (requestId === logRequestRef.current) setLogLoadError(errorMessage(reason));
    } finally {
      if (requestId === logRequestRef.current) {
        setLogLoading(false);
        setLogLoadingOlder(false);
      }
    }
  }, [logLevelFilter, logSearch, logTypeFilters]);

  useEffect(() => {
    void loadActivityLogs();
  }, [loadActivityLogs]);

  useEffect(() => {
    const unlisten = listen<LogLine>("sync-log", (event) => {
      const filters = logFiltersRef.current;
      if (logMatchesFilters(event.payload, filters.types, filters.level, filters.search)) {
        setLogLines((current) => mergeLogLines(current, [event.payload]).slice(-2_000));
      }
      if (!activityOpenRef.current) setUnreadLogs((current) => Math.min(99, current + 1));
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const openActivity = () => {
    activityOpenRef.current = true;
    setActivityOpen(true);
    setUnreadLogs(0);
    setLogScrollToBottomEpoch((epoch) => epoch + 1);
    void loadActivityLogs();
  };

  const startLogResize = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !activityOpen) return;
    event.preventDefault();

    const startY = event.clientY;
    const startHeight = logHeight;
    const maxHeight = Math.max(140, Math.min(520, window.innerHeight - 120));
    setResizingLog(true);

    const handleMouseMove = (moveEvent: MouseEvent) => {
      const nextHeight = startHeight + startY - moveEvent.clientY;
      setLogHeight(Math.min(maxHeight, Math.max(140, nextHeight)));
    };
    const handleMouseUp = () => {
      setResizingLog(false);
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
  };

  const startSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    sidebarResizeRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: sidebarWidth,
    };
    setResizingSidebar(true);
  };

  const continueSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const resize = sidebarResizeRef.current;
    if (!resize || resize.pointerId !== event.pointerId) return;
    setSidebarWidth(clampSidebarWidth(
      resize.startWidth + event.clientX - resize.startX,
      availableProjectSidebarWidth(),
    ));
  };

  const finishSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (sidebarResizeRef.current?.pointerId !== event.pointerId) return;
    sidebarResizeRef.current = null;
    setResizingSidebar(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const resizeSidebarWithKeyboard = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    let nextWidth: number | null = null;
    if (event.key === "ArrowLeft") nextWidth = sidebarWidth - 16;
    if (event.key === "ArrowRight") nextWidth = sidebarWidth + 16;
    if (event.key === "Home") nextWidth = MIN_PROJECT_SIDEBAR_WIDTH;
    if (event.key === "End") nextWidth = availableProjectSidebarWidth();
    if (nextWidth === null) return;
    event.preventDefault();
    setSidebarWidth(clampSidebarWidth(nextWidth, availableProjectSidebarWidth()));
  };

  const binding = detail?.binding ?? null;
  const resources = inventoryResources(inventory);
  const projects = useMemo<LocalProjectSummary[]>(() => registrations.map((registration) => {
    const localBinding = bindings.find((candidate) => (
      candidate.local_project_id === registration.local_project_id && candidate.state === "active"
    ));
    const links = config.links.filter((link) => link.local_project_id === registration.local_project_id);
    const active = registration.local_project_id === activeProjectId;
    const activeResources = active ? resources : [];
    return {
      local_project_id: registration.local_project_id,
      bundle_id: registration.bundle_id,
      display_name: registration.display_name,
      local_alias: registration.local_alias,
      revision: registration.revision,
      repository_fingerprint: registration.repository_fingerprint,
      project_root: localBinding?.project_root ?? null,
      canonical_project_root: localBinding?.canonical_project_root ?? null,
      profile_ids: localBinding?.profile_ids ?? {},
      profile_names: [...new Set(Object.values(localBinding?.profile_ids ?? {})
        .map((profileId) => profiles.find((profile) => profile.profile_id === profileId)?.display_name)
        .filter((name): name is string => Boolean(name)))],
      providers: active
        ? [...new Set(activeResources.flatMap((resource) => resource.provider ? [resource.provider] : []))]
        : undefined,
      resource_count: active ? activeResources.length : undefined,
      selected_resource_count: Object.keys(registration.recipe.entries).length,
      linked_storage_ids: links.map((link) => link.storage_id),
      readiness_state: active ? readiness?.state : undefined,
      is_git_repository: repositoryKinds[registration.local_project_id],
    };
  }), [activeProjectId, bindings, config.links, profiles, readiness?.state, registrations, repositoryKinds, resources]);

  const activeSummary = projects.find((candidate) => candidate.local_project_id === activeProjectId) ?? null;
  const activeDraftSummary = setupDraftId
    ? setupDrafts.find((candidate) => candidate.draft_id === setupDraftId) ?? null
    : null;
  const workspaceTitle = activeDraftSummary?.display_name
    ?? (activeSummary ? projectLabel(activeSummary) : "Projects");
  const workspaceIcon = !activeDraftSummary && activeSummary?.is_git_repository === true
    ? "git-folder"
    : "folder";
  const restoreProfileLabel = restoreBinding
    ? [...new Set(Object.values(restoreBinding.profile_ids)
      .map((profileId) => profiles.find((profile) => profile.profile_id === profileId)?.display_name)
      .filter((name): name is string => Boolean(name)))]
      .join(" + ") || "the assigned provider profile"
    : "the assigned provider profile";
  const pendingConnectionProject = pendingBundleConnection
    ? registrations.find((candidate) => candidate.local_project_id === pendingBundleConnection.projectId) ?? null
    : null;
  const pendingConnectionStorage = pendingBundleConnection
    ? config.storages.find((candidate) => candidate.id === pendingBundleConnection.storageId) ?? null
    : null;
  const projectLinks: ProjectStorageLink[] = detail?.project.local_project_id === activeProjectId
    ? detail.links
    : config.links.filter((link) => link.local_project_id === activeProjectId);
  const activeStorageId = activeProjectId
    ? activeStorageByProject[activeProjectId] ?? projectLinks[0]?.storage_id ?? null
    : null;
  const clearHostScopedWorkflows = useCallback(() => {
    hostWorkflowGeneration.current += 1;
    restoreRequest.current += 1;
    setPendingBundleConnection(null);
    setPendingPush(null);
    setPushSelected(new Set());
    setPushProjectContentRemovals(new Set());
    setPushWarningAcknowledgements(new Set());
    setProjectContentLoading(false);
    setPushError(null);
    setPushReviewStep("history");
    setEditingBinding(null);
    setRestorePlan(null);
    setRestoreBinding(null);
    setDependencyPlan(null);
    setRestoreResult(null);
    setDependencyResult(null);
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
    setSetupDraftId(null);
    setSetupDrafts([]);
    setActiveProjectId(null);
    setDetail(null);
    setInventory(null);
    setReadiness(null);
    setRegistrations([]);
    setRepositoryKinds({});
    setBindings([]);
    setProfiles([]);
    setConversationPathAudits({});
    setConversationPathAuditErrors({});
    setConversationPathAuditLoading(false);
    setConfig(EMPTY_CONFIG);
    setError(null);
    setBackendError(null);
    setNotice(null);
    setBusy(false);
  }, []);
  const loadConversationPathAudits = async (
    nextBindings: ProjectBinding[],
    shellRequestId?: number,
    workflowGeneration?: number,
  ) => {
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || (workflowGeneration !== undefined && hostWorkflowGeneration.current !== workflowGeneration)
    ) return;
    setConversationPathAuditLoading(true);
    const activeBindings = nextBindings.filter((candidate) => candidate.state === "active");
    const results = await Promise.all(activeBindings.map(async (candidate) => {
      try {
        const audit = await projectSyncApi.auditCodexConversationPaths(candidate.local_project_id);
        return { projectId: candidate.local_project_id, audit, error: null };
      } catch (reason) {
        return {
          projectId: candidate.local_project_id,
          audit: null,
          error: errorMessage(reason),
        };
      }
    }));
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || (workflowGeneration !== undefined && hostWorkflowGeneration.current !== workflowGeneration)
    ) return;
    const nextAudits: Record<string, CodexConversationPathAudit> = {};
    const nextErrors: Record<string, string> = {};
    for (const result of results) {
      if (result.audit) nextAudits[result.projectId] = result.audit;
      if (result.error) nextErrors[result.projectId] = result.error;
    }
    setConversationPathAudits(nextAudits);
    setConversationPathAuditErrors(nextErrors);
    setConversationPathAuditLoading(false);
  };
  const loadShell = async (): Promise<{
    nextProjects: LocalProjectRegistration[];
    nextConfig: SyncConfigV3;
    nextBindings: ProjectBinding[];
    nextProfiles: ProviderProfileSummary[];
    hostIdentityReady: boolean;
  }> => {
    const requestId = ++shellRequest.current;
    const unavailableResult = () => ({
      nextProjects: registrations,
      nextConfig: config,
      nextBindings: bindings,
      nextProfiles: profiles,
      hostIdentityReady: false,
    });
    setLoading(true);
    const failures: string[] = [];
    let nextProjects: LocalProjectRegistration[] = [];
    let nextConfig: SyncConfigV3 = EMPTY_CONFIG;
    let nextBindings: ProjectBinding[] = [];
    let nextProfiles: ProviderProfileSummary[] = [];
    let verifiedHostIdentity: HostIdentityStatus | null = null;
    try {
      const nextHostIdentity = await projectSyncApi.getHostIdentityStatus();
      if (shellRequest.current !== requestId) return unavailableResult();
      verifiedHostIdentity = nextHostIdentity;
      const previous = readyHostIdentity.current;
      if (previous && (
        previous.hostId !== nextHostIdentity.host_id
        || previous.revision !== nextHostIdentity.revision
      )) {
        setHostIdentityShellReady(false);
        clearHostScopedWorkflows();
      }
      setHostIdentityStatus(nextHostIdentity);
      setHostIdentityError(null);
      if (!nextHostIdentity.origin_key_available) {
        setHostIdentityShellReady(false);
        clearHostScopedWorkflows();
        setLoading(false);
        return unavailableResult();
      }
    } catch (reason) {
      if (shellRequest.current !== requestId) return unavailableResult();
      setHostIdentityStatus(null);
      setHostIdentityError(errorMessage(reason));
      setHostIdentityShellReady(false);
      clearHostScopedWorkflows();
      setLoading(false);
      return unavailableResult();
    }
    try {
      nextConfig = await projectSyncApi.getConfig();
      if (shellRequest.current !== requestId) return unavailableResult();
      if (nextConfig.schema !== PROJECT_SYNC_SCHEMA) {
        throw new Error(
          `Expected project sync schema ${PROJECT_SYNC_SCHEMA}, received schema ${nextConfig.schema}`,
        );
      }
      setConfig(nextConfig);
    } catch (reason) {
      failures.push(`Configuration: ${errorMessage(reason)}`);
      setConfig(EMPTY_CONFIG);
    }
    try {
      // Listing projects also completes any interrupted setup finalization,
      // so drafts are refreshed afterwards to reflect consumed drafts.
      nextProjects = await projectSyncApi.listProjects();
      if (shellRequest.current !== requestId) return unavailableResult();
      setRegistrations(nextProjects);
    } catch (reason) {
      failures.push(`Projects: ${errorMessage(reason)}`);
      setRegistrations([]);
    }
    await refreshSetupDrafts(requestId);
    if (shellRequest.current !== requestId) return unavailableResult();
    try {
      const nextRepositoryKinds = await projectSyncApi.listProjectRepositoryKinds();
      if (shellRequest.current !== requestId) return unavailableResult();
      setRepositoryKinds(nextRepositoryKinds);
    } catch (reason) {
      failures.push(`Repository types: ${errorMessage(reason)}`);
      setRepositoryKinds({});
    }
    try {
      nextProfiles = await projectSyncApi.listProviderProfiles();
      if (shellRequest.current !== requestId) return unavailableResult();
      setProfiles(nextProfiles);
    } catch (reason) {
      failures.push(`Profiles: ${errorMessage(reason)}`);
      setProfiles([]);
    }
    try {
      nextBindings = await projectSyncApi.listBindings();
      if (shellRequest.current !== requestId) return unavailableResult();
      setBindings(nextBindings);
    } catch (reason) {
      failures.push(`Bindings: ${errorMessage(reason)}`);
      setBindings([]);
    }
    await loadConversationPathAudits(nextBindings, requestId);
    if (shellRequest.current !== requestId) return unavailableResult();
    setBackendError(failures.length > 0 ? failures.join(" · ") : null);
    let nextHostIdentity: HostIdentityStatus;
    try {
      nextHostIdentity = await projectSyncApi.getHostIdentityStatus();
    } catch (reason) {
      if (shellRequest.current !== requestId) return unavailableResult();
      setHostIdentityStatus(null);
      setHostIdentityError(errorMessage(reason));
      setHostIdentityShellReady(false);
      clearHostScopedWorkflows();
      setLoading(false);
      return unavailableResult();
    }
    if (shellRequest.current !== requestId) return unavailableResult();
    if (!nextHostIdentity.origin_key_available) {
      setHostIdentityStatus(nextHostIdentity);
      setHostIdentityShellReady(false);
      clearHostScopedWorkflows();
      setLoading(false);
      return unavailableResult();
    }
    if (
      !verifiedHostIdentity
      || verifiedHostIdentity.host_id !== nextHostIdentity.host_id
      || verifiedHostIdentity.revision !== nextHostIdentity.revision
    ) {
      setHostIdentityStatus(nextHostIdentity);
      setHostIdentityShellReady(false);
      clearHostScopedWorkflows();
      return loadShell();
    }
    setHostIdentityStatus(nextHostIdentity);
    readyHostIdentity.current = {
      hostId: nextHostIdentity.host_id,
      revision: nextHostIdentity.revision,
    };
    setHostIdentityShellReady(true);
    setLoading(false);
    return { nextProjects, nextConfig, nextBindings, nextProfiles, hostIdentityReady: true };
  };

  const loadProjectData = async (
    projectId: string,
    cfg = config,
    preferredStorage?: string | null,
    workflowGeneration?: number,
  ) => {
    const requestedWorkflowGeneration = workflowGeneration ?? hostWorkflowGeneration.current;
    if (hostWorkflowGeneration.current !== requestedWorkflowGeneration) return;
    setLoading(true);
    setError(null);
    try {
      const nextDetail = await projectSyncApi.getProject(projectId);
      if (hostWorkflowGeneration.current !== requestedWorkflowGeneration) return;
      if (!nextDetail) throw new Error("The selected local project no longer exists.");
      setDetail(nextDetail);
      setRegistrations((current) => upsertProject(current, nextDetail.project));
      if (nextDetail.binding) {
        setBindings((current) => upsertBinding(current, nextDetail.binding as ProjectBinding));
      }

      let nextInventory: ResourceInventoryModel | null = null;
      if (nextDetail.binding?.state === "active") {
        try {
          nextInventory = await projectSyncApi.getInventory(projectId);
          if (hostWorkflowGeneration.current !== requestedWorkflowGeneration) return;
        } catch (reason) {
          if (hostWorkflowGeneration.current === requestedWorkflowGeneration) {
            setError(errorMessage(reason));
          }
        }
      }
      setInventory(nextInventory);

      const links = nextDetail.links.length > 0
        ? nextDetail.links
        : cfg.links.filter((link) => link.local_project_id === projectId);
      const remembered = activeStorageByProject[projectId];
      const storageId = preferredStorage && links.some((link) => link.storage_id === preferredStorage)
        ? preferredStorage
        : remembered && links.some((link) => link.storage_id === remembered)
          ? remembered
          : links[0]?.storage_id ?? null;
      if (storageId && nextDetail.binding?.state === "active") {
        setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
        try {
          const nextReadiness = await projectSyncApi.getReadiness(
            storageId,
            nextDetail.project.bundle_id,
            nextDetail.binding,
          );
          if (hostWorkflowGeneration.current !== requestedWorkflowGeneration) return;
          setReadiness(nextReadiness);
        } catch {
          if (hostWorkflowGeneration.current === requestedWorkflowGeneration) {
            setReadiness(null);
          }
        }
      } else {
        setReadiness(null);
      }
    } catch (reason) {
      if (hostWorkflowGeneration.current !== requestedWorkflowGeneration) return;
      setDetail(null);
      setInventory(null);
      setReadiness(null);
      setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === requestedWorkflowGeneration) setLoading(false);
    }
  };

  useEffect(() => {
    void (async () => {
      const { nextProjects, nextConfig, hostIdentityReady } = await loadShell();
      if (!hostIdentityReady) return;
      const first = nextProjects[0]?.local_project_id ?? null;
      setActiveProjectId(first);
      if (first) await loadProjectData(first, nextConfig);
    })();
  }, []); // ProjectSyncV3 is the schema-3 lifetime boundary.

  const refresh = async (workflowGeneration?: number) => {
    if (
      workflowGeneration !== undefined
      && hostWorkflowGeneration.current !== workflowGeneration
    ) return;
    const current = activeProjectId;
    const { nextProjects, nextConfig, hostIdentityReady } = await loadShell();
    if (
      !hostIdentityReady
      || (workflowGeneration !== undefined && hostWorkflowGeneration.current !== workflowGeneration)
    ) return;
    const continuationGeneration = workflowGeneration ?? hostWorkflowGeneration.current;
    const next = current && nextProjects.some((candidate) => candidate.local_project_id === current)
      ? current
      : nextProjects[0]?.local_project_id ?? null;
    setActiveProjectId(next);
    if (next) await loadProjectData(next, nextConfig, undefined, continuationGeneration);
    else {
      setDetail(null);
      setInventory(null);
    }
    if (hostWorkflowGeneration.current !== continuationGeneration) return;
    setHistoryRefreshEpoch((epoch) => epoch + 1);
  };

  // Quietly reconciles state commonly changed outside Mallard (git init, external
  // renames) after the window regains focus. Deliberately narrower than refresh():
  // no audits, profiles, readiness probes, history reset, or loading indicator.
  const reconcileExternalChanges = async () => {
    if (loading || busy || !hostIdentityShellReady) return;
    const shellRequestId = shellRequest.current;
    const workflowGeneration = hostWorkflowGeneration.current;
    const stale = () => (
      shellRequest.current !== shellRequestId
      || hostWorkflowGeneration.current !== workflowGeneration
    );
    try {
      const [nextProjects, nextRepositoryKinds] = await Promise.all([
        projectSyncApi.listProjects(),
        projectSyncApi.listProjectRepositoryKinds(),
      ]);
      if (stale()) return;
      setRegistrations(nextProjects);
      setRepositoryKinds(nextRepositoryKinds);
      if (!activeProjectId) return;
      if (!nextProjects.some((candidate) => candidate.local_project_id === activeProjectId)) {
        // The active project vanished externally; the full refresh reselects safely.
        await refresh(workflowGeneration);
        return;
      }
      const nextDetail = await projectSyncApi.getProject(activeProjectId);
      if (stale() || !nextDetail) return;
      setDetail(nextDetail);
      if (nextDetail.binding) {
        setBindings((current) => upsertBinding(current, nextDetail.binding as ProjectBinding));
      }
    } catch {
      // Background reconcile stays silent; explicit actions still surface their errors.
    }
  };
  const reconcileExternalChangesRef = useRef(reconcileExternalChanges);
  reconcileExternalChangesRef.current = reconcileExternalChanges;
  const windowBlurredAt = useRef<number | null>(null);
  useEffect(() => {
    const handleBlur = () => { windowBlurredAt.current = Date.now(); };
    const handleFocus = () => {
      const blurredAt = windowBlurredAt.current;
      windowBlurredAt.current = null;
      if (blurredAt !== null && Date.now() - blurredAt >= FOCUS_RECONCILE_MIN_AWAY_MS) {
        void reconcileExternalChangesRef.current();
      }
    };
    window.addEventListener("blur", handleBlur);
    window.addEventListener("focus", handleFocus);
    return () => {
      window.removeEventListener("blur", handleBlur);
      window.removeEventListener("focus", handleFocus);
    };
  }, []);

  useEffect(() => {
    if (!appMenuOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !appMenuRef.current?.contains(event.target)) setAppMenuOpen(false);
    };
    const handleFocusIn = (event: FocusEvent) => {
      if (event.target instanceof Node && !appMenuRef.current?.contains(event.target)) setAppMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setAppMenuOpen(false);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("focusin", handleFocusIn);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("focusin", handleFocusIn);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [appMenuOpen]);

  const registerNewHost = async () => {
    if (!hostIdentityStatus) return;
    shellRequest.current += 1;
    setHostIdentityShellReady(false);
    clearHostScopedWorkflows();
    setHostIdentityBusy(true);
    setHostIdentityError(null);
    try {
      await projectSyncApi.registerAsNewHost({
        expected_host_id: hostIdentityStatus.host_id,
        expected_revision: hostIdentityStatus.revision,
        local_display_name: hostIdentityStatus.local_display_name ?? null,
      });
      await refresh();
      setNotice("Registered as a new host. Existing project bindings must be revalidated before sync resumes.");
    } catch (reason) {
      setHostIdentityError(errorMessage(reason));
      try {
        const refreshed = await projectSyncApi.getHostIdentityStatus();
        if (refreshed.origin_key_available) {
          setHostIdentityError(null);
          await refresh();
        } else {
          setHostIdentityStatus(refreshed);
        }
      } catch {
        // Preserve the actionable registration error when status refresh also
        // fails; the recovery surface still offers a full shell retry.
      }
    } finally {
      setHostIdentityBusy(false);
    }
  };

  const selectProject = async (projectId: string, preferredStorage?: string | null) => {
    setActiveProjectId(projectId);
    await loadProjectData(projectId, config, preferredStorage);
  };

  const selectStorage = (projectId: string, storageId: string) => {
    if (!config.links.some((link) => (
      link.local_project_id === projectId && link.storage_id === storageId
    ))) return;
    setActiveProjectId(projectId);
    setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
  };

  const createProfileAtPath = async (
    provider: ProjectProvider,
    path: string,
    workflowGeneration = hostWorkflowGeneration.current,
  ): Promise<ProviderProfile | null> => {
    if (!path.trim() || hostWorkflowGeneration.current !== workflowGeneration) return null;
    setBusy(true);
    setError(null);
    try {
      const probe = await projectSyncApi.probeProviderProfile(provider, path.trim());
      if (hostWorkflowGeneration.current !== workflowGeneration) return null;
      const currentProfiles = await projectSyncApi.listProviderProfiles();
      if (hostWorkflowGeneration.current !== workflowGeneration) return null;
      const existing = probe.existing_profile_id
        ? currentProfiles.find((profile) => profile.profile_id === probe.existing_profile_id) ?? null
        : null;
      const profile = existing ?? await projectSyncApi.createProviderProfile(
        provider,
        probe.suggested_name,
        path.trim(),
      );
      if (hostWorkflowGeneration.current !== workflowGeneration) return null;
      const nextProfiles = existing ? currentProfiles : await projectSyncApi.listProviderProfiles();
      if (hostWorkflowGeneration.current !== workflowGeneration) return null;
      setProfiles(nextProfiles);
      return profile;
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
      return null;
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const chooseAndCreateProfile = async (provider: ProjectProvider): Promise<ProviderProfile | null> => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const picked = await open({ directory: true, multiple: false });
    if (
      hostWorkflowGeneration.current !== workflowGeneration
      || typeof picked !== "string"
      || !picked
    ) return null;
    return createProfileAtPath(provider, picked, workflowGeneration);
  };

  const refreshSetupDrafts = async (shellRequestId?: number, workflowGeneration?: number) => {
    const requestedWorkflowGeneration = workflowGeneration ?? hostWorkflowGeneration.current;
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || hostWorkflowGeneration.current !== requestedWorkflowGeneration
    ) return;
    try {
      const listed = await projectSyncApi.listSetupDrafts();
      if (
        (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
        || hostWorkflowGeneration.current !== requestedWorkflowGeneration
      ) return;
      setSetupDrafts(listed.drafts);
    } catch {
      // Drafts are a convenience surface; the projects list stays usable.
    }
  };

  const closeWorkspaceSettings = () => {
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    setStorageEditorRequest((current) => ({
      mode: "close",
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  const beginAddProject = async () => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const picked = await open({ directory: true, multiple: false });
    if (
      hostWorkflowGeneration.current !== workflowGeneration
      || typeof picked !== "string"
      || !picked
    ) return;
    setBusy(true);
    setError(null);
    try {
      const created = await projectSyncApi.createSetupDraft(picked);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      closeWorkspaceSettings();
      setSetupDraftId(created.draft.draft_id);
      if (created.resumed) {
        setNotice(`Resumed the saved setup draft for ${created.draft.display_name}.`);
      }
      await refreshSetupDrafts(undefined, workflowGeneration);
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const openSetupDraft = (draftId: string) => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId((current) => current === draftId ? null : draftId);
  };

  const closeSetupDraft = async () => {
    setSetupDraftId(null);
    await refreshSetupDrafts();
  };

  const discardSetupDraft = async (draftId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const summary = setupDrafts.find((candidate) => candidate.draft_id === draftId);
    const approved = await confirm(
      `Discard the setup draft for “${summary?.display_name ?? "this project"}”?\n\nOnly the draft is removed. No project files are touched.`,
      { title: "Discard draft" },
    );
    if (!approved || hostWorkflowGeneration.current !== workflowGeneration) return;
    setBusy(true);
    setError(null);
    try {
      await projectSyncApi.discardSetupDraft(draftId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      if (setupDraftId === draftId) setSetupDraftId(null);
      await refreshSetupDrafts(undefined, workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice("Setup draft discarded. Project files were not touched.");
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const completeSetup = async (
    detail: ProjectDetail,
    completion: SetupCompletion,
    workflowGeneration: number,
  ) => {
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    setSetupDraftId(null);
    const projectId = detail.project.local_project_id;
    const storageId = detail.links[0]?.storage_id ?? null;
    setNotice(completion === "pull"
      ? `${projectLabel(detail.project)} connected. Review Pull before files are applied.`
      : `${projectLabel(detail.project)} is set up${storageId ? "" : " — link storage when ready"}.`);
    const { nextConfig, hostIdentityReady } = await loadShell();
    if (!hostIdentityReady || hostWorkflowGeneration.current !== workflowGeneration) return;
    await refreshSetupDrafts(undefined, workflowGeneration);
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    setActiveProjectId(projectId);
    if (detail.binding) setBindings((current) => upsertBinding(current, detail.binding as ProjectBinding));
    await loadProjectData(projectId, nextConfig, storageId, workflowGeneration);
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    if (completion === "pull" && storageId && detail.binding) {
      await planRestore(storageId, detail.project.bundle_id, detail.binding, projectLabel(detail.project));
    } else if (completion === "push" && storageId) {
      await publishProject(projectId, storageId, detail.project.recipe);
    }
  };

  const saveStorageLink = async (
    projectId: string,
    storageId: string,
    workflowGeneration = hostWorkflowGeneration.current,
  ) => {
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    await projectSyncApi.saveLink({
      local_project_id: projectId,
      storage_id: storageId,
      pinned: true,
    });
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    const { nextConfig, hostIdentityReady } = await loadShell();
    if (!hostIdentityReady || hostWorkflowGeneration.current !== workflowGeneration) return;
    setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
    setActiveProjectId(projectId);
    await loadProjectData(projectId, nextConfig, storageId, workflowGeneration);
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    setNotice("Storage linked to this project repo.");
  };

  const linkStorage = async (projectId: string, storageId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    setBusy(true);
    setError(null);
    try {
      const project = registrations.find((candidate) => candidate.local_project_id === projectId);
      if (!project) throw new Error("The selected local project no longer exists.");
      const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      bundles.sort((left, right) => {
        const leftMatches = !!project.repository_fingerprint
          && left.repository_fingerprint === project.repository_fingerprint;
        const rightMatches = !!project.repository_fingerprint
          && right.repository_fingerprint === project.repository_fingerprint;
        if (leftMatches !== rightMatches) return leftMatches ? -1 : 1;
        return (right.updated_at ?? 0) - (left.updated_at ?? 0);
      });
      if (bundles.some((bundle) => bundle.bundle_id === project.bundle_id) || bundles.length === 0) {
        await saveStorageLink(projectId, storageId, workflowGeneration);
        return;
      }
      setPendingBundleConnection({ projectId, storageId, matches: bundles, reason: "link" });
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const repairConversationPaths = async (projectId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const audit = conversationPathAudits[projectId];
    if (!audit?.can_repair) return;
    const registration = registrations.find((candidate) => candidate.local_project_id === projectId);
    const projectName = registration ? projectLabel(registration) : "this project";
    const approved = await confirm(
      `Repair ${audit.issues.length} Codex conversation path${audit.issues.length === 1 ? "" : "s"} for “${projectName}”?\n\nOnly conversations explicitly assigned to this project by Codex Desktop will be changed. Mallard will back up every affected rollout first, then update structural cwd fields without changing messages or history.\n\nClose the affected Codex tasks before continuing so they are not written at the same time.`,
      { title: "Repair conversation paths" },
    );
    if (!approved || hostWorkflowGeneration.current !== workflowGeneration) return;

    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.repairCodexConversationPaths(projectId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setConversationPathAudits((current) => ({
        ...current,
        [projectId]: result.audit,
      }));
      setConversationPathAuditErrors((current) => {
        const next = { ...current };
        delete next[projectId];
        return next;
      });
      setActiveProjectId(projectId);
      await loadProjectData(
        projectId,
        config,
        activeStorageByProject[projectId],
        workflowGeneration,
      );
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice(
        `Repaired ${result.repaired_thread_ids.length} Codex conversation path${result.repaired_thread_ids.length === 1 ? "" : "s"}. Original rollouts were backed up.`,
      );
    } catch (reason) {
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setError(errorMessage(reason));
      try {
        const refreshed = await projectSyncApi.auditCodexConversationPaths(projectId);
        if (hostWorkflowGeneration.current !== workflowGeneration) return;
        setConversationPathAudits((current) => ({ ...current, [projectId]: refreshed }));
      } catch {
        // Keep the prior gate visible when the follow-up audit also fails.
      }
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const openPushChooser = async (
    projectId: string,
    storageId: string,
    workflowGeneration = hostWorkflowGeneration.current,
  ) => {
    if (hostWorkflowGeneration.current !== workflowGeneration) return;
    if (restorePlan) closePullReview();
    setPushReviewStep("history");
    setPushProjectContentRemovals(new Set());
    setPushWarningAcknowledgements(new Set());
    setProjectContentLoading(false);
    setBusy(true);
    setError(null);
    setPushError(null);
    try {
      const [nextInventory, nextDetail] = await Promise.all([
        projectSyncApi.getInventory(projectId),
        projectSyncApi.getProject(projectId),
      ]);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      if (!nextDetail) throw new Error("The selected local project no longer exists.");
      const storage = config.storages.find((candidate) => candidate.id === storageId);
      if (!storage) throw new Error("The selected storage no longer exists.");
      const link = nextDetail.links.find((candidate) => candidate.storage_id === storageId);
      if (!link) throw new Error("This project is no longer linked to the selected storage.");

      const savedRecipe = link.recipe ?? null;
      const [threadComparisonResult, capabilityReportResult] = await Promise.allSettled([
        nextDetail.binding?.profile_ids.codex
          ? projectSyncApi.getThreadSyncComparison(projectId, storageId)
          : Promise.resolve(null),
        nextDetail.binding
          ? projectSyncApi.getCapabilityStatus(projectId, storageId)
          : Promise.resolve(null),
      ]);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      const nextResources = inventoryResources(nextInventory);
      const nextThreadComparison = threadComparisonResult.status === "fulfilled"
        ? threadComparisonResult.value
        : undefined;
      const nextCapabilityReport = capabilityReportResult.status === "fulfilled"
        ? capabilityReportResult.value
        : undefined;
      const isGitRepository = repositoryKinds[projectId] ?? false;
      let lockedProjectContentInventory: ProjectContentInventory | null = null;
      if (isGitRepository && nextDetail.binding) {
        try {
          const inspected = await projectSyncApi.inspectProjectFiles(projectId, storageId);
          if (hostWorkflowGeneration.current !== workflowGeneration) return;
          if (inspected.entries.length > 0) lockedProjectContentInventory = inspected;
        } catch {
          // A Git project without stored generic content keeps the normal four-step review.
        }
      }
      const projectDefaults = recommendedPushSelection(
        recipeSelection(nextInventory.recipe),
        nextResources,
        nextThreadComparison,
        nextCapabilityReport,
      );
      const projectSummary: LocalProjectSummary = {
        local_project_id: nextDetail.project.local_project_id,
        bundle_id: nextDetail.project.bundle_id,
        display_name: nextDetail.project.display_name,
        local_alias: nextDetail.project.local_alias,
        revision: nextDetail.project.revision,
        repository_fingerprint: nextDetail.project.repository_fingerprint,
        project_root: nextDetail.binding?.project_root ?? null,
        canonical_project_root: nextDetail.binding?.canonical_project_root ?? null,
        profile_ids: nextDetail.binding?.profile_ids ?? {},
        profile_names: [...new Set(Object.values(nextDetail.binding?.profile_ids ?? {})
          .map((profileId) => profiles.find((profile) => profile.profile_id === profileId)?.display_name)
          .filter((name): name is string => Boolean(name)))],
        providers: [...new Set(nextResources.flatMap((resource) => resource.provider ? [resource.provider] : []))],
        resource_count: nextResources.length,
        selected_resource_count: Object.keys(nextDetail.project.recipe.entries).length,
        linked_storage_ids: nextDetail.links.map((candidate) => candidate.storage_id),
        is_git_repository: isGitRepository,
      };
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setPendingPush({
        projectId,
        storageId,
        projectName: projectLabel(nextDetail.project),
        project: projectSummary,
        binding: nextDetail.binding ?? null,
        storage,
        inventory: nextInventory,
        savedRecipe,
        projectDefaults,
        threadComparison: nextThreadComparison,
        capabilityReport: nextCapabilityReport,
        projectContentInventory: lockedProjectContentInventory,
        projectContentScanned: lockedProjectContentInventory !== null,
        showProjectFiles: !isGitRepository || lockedProjectContentInventory !== null,
      });
      setPushSelected(new Set(projectDefaults));
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const refreshRepositoryKinds = async (workflowGeneration?: number) => {
    if (
      workflowGeneration !== undefined
      && hostWorkflowGeneration.current !== workflowGeneration
    ) return;
    try {
      const nextRepositoryKinds = await projectSyncApi.listProjectRepositoryKinds();
      if (
        workflowGeneration !== undefined
        && hostWorkflowGeneration.current !== workflowGeneration
      ) return;
      setRepositoryKinds(nextRepositoryKinds);
    } catch {
      if (
        workflowGeneration === undefined
        || hostWorkflowGeneration.current === workflowGeneration
      ) setRepositoryKinds({});
    }
  };

  const scanPendingProjectFiles = async () => {
    const pending = pendingPush;
    if (!pending || projectContentLoading) return;
    const workflowGeneration = hostWorkflowGeneration.current;
    setProjectContentLoading(true);
    setPushError(null);
    try {
      const inspected = await projectSyncApi.inspectProjectFiles(pending.projectId, pending.storageId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setPendingPush((current) => {
        if (!current || current.projectId !== pending.projectId || current.storageId !== pending.storageId) {
          return current;
        }
        const previousGenericIds = new Set((current.projectContentInventory?.entries ?? [])
          .map((entry) => entry.descriptor.resource_id));
        const inspectedIds = new Set(inspected.entries.map((entry) => entry.descriptor.resource_id));
        setPushSelected((currentSelection) => {
          const next = new Set(currentSelection);
          for (const resourceId of previousGenericIds) {
            if (!inspectedIds.has(resourceId)) next.delete(resourceId);
          }
          for (const entry of inspected.entries) {
            const resourceId = entry.descriptor.resource_id;
            const wasSelected = currentSelection.has(resourceId);
            const shouldSelect = !pushProjectContentRemovals.has(resourceId) && (
              wasSelected || entry.selected_in_recipe || entry.selected_after_scan
            );
            if (shouldSelect) next.add(resourceId);
            else next.delete(resourceId);
          }
          return includeRequiredProjectContentDirectories(inspected, next);
        });
        setPushWarningAcknowledgements((currentAcknowledgements) => {
          const currentWarningDigests = new Set(inspected.entries
            .map((entry) => entry.warning_digest)
            .filter((digest): digest is string => Boolean(digest)));
          return new Set([...currentAcknowledgements].filter((digest) => currentWarningDigests.has(digest)));
        });
        const nextDefaults = new Set(current.projectDefaults);
        inspected.entries.forEach((entry) => {
          if (entry.selected_in_recipe || entry.selected_after_scan) {
            nextDefaults.add(entry.descriptor.resource_id);
          }
        });
        return {
          ...current,
          projectContentInventory: inspected,
          projectContentScanned: true,
          showProjectFiles: true,
          projectDefaults: includeRequiredProjectContentDirectories(inspected, nextDefaults),
        };
      });
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setPushError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setProjectContentLoading(false);
    }
  };

  const publishProject = async (
    projectId: string,
    storageId: string,
    recipe: BundleRecipe,
    projectContentReviewToken: string | null = null,
    projectContentRemovalIds: string[] = [],
    acknowledgedWarningDigests: string[] = [],
  ) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    openActivity();
    setBusy(true);
    setError(null);
    setPushError(null);
    try {
      const result = await projectSyncApi.pushBundle(
        projectId,
        storageId,
        recipe,
        projectContentReviewToken,
        projectContentRemovalIds,
        acknowledgedWarningDigests,
      );
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice(result.message);
      setHistoryRefreshEpoch((epoch) => epoch + 1);
      setProjectTabResetEpoch((epoch) => epoch + 1);
      setPendingPush(null);
      setPushProjectContentRemovals(new Set());
      setPushWarningAcknowledgements(new Set());
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
      const { nextConfig, hostIdentityReady } = await loadShell();
      if (!hostIdentityReady || hostWorkflowGeneration.current !== workflowGeneration) return;
      await loadProjectData(projectId, nextConfig, storageId, workflowGeneration);
    } catch (reason) {
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      const message = errorMessage(reason);
      if (pendingPush) setPushError(message);
      else setError(message);
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const publishPendingPush = async () => {
    if (!pendingPush) return;
    const resources = pushResources(pendingPush);
    const baseRecipe = pendingPush.savedRecipe ?? {
      ...pendingPush.inventory.recipe,
      revision: 0,
      entries: {},
    };
    const safeSelection = sanitizePushSelection(
      pushSelected,
      resources,
      pendingPush.threadComparison,
      pendingPush.capabilityReport,
      pendingPush.projectContentInventory,
    );
    const preservationRecipe: BundleRecipe = {
      ...pendingPush.inventory.recipe,
      entries: {
        ...pendingPush.inventory.recipe.entries,
        ...(pendingPush.savedRecipe?.entries ?? {}),
      },
    };
    const recipe = preserveSyncedPushRecipeEntries(
      recipeWithSelection(baseRecipe, resources, safeSelection),
      preservationRecipe,
      pendingPush.threadComparison,
    );
    for (const resourceId of pushProjectContentRemovals) delete recipe.entries[resourceId];
    await publishProject(
      pendingPush.projectId,
      pendingPush.storageId,
      recipe,
      pendingPush.projectContentInventory?.eligibility.state === "eligible"
        ? pendingPush.projectContentInventory.review_token ?? null
        : null,
      [...pushProjectContentRemovals],
      [...pushWarningAcknowledgements],
    );
  };

  const saveRemap = async (nextBinding: ProjectBindingDraft) => {
    if (!activeProjectId) return;
    const workflowGeneration = hostWorkflowGeneration.current;
    const projectId = activeProjectId;
    setBusy(true);
    setError(null);
    try {
      const saved = await projectSyncApi.saveBinding({
        local_project_id: projectId,
        project_root: nextBinding.project_root,
        profile_ids: nextBinding.profile_ids,
        expected_revision: nextBinding.expected_revision ?? binding?.revision ?? null,
      });
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setEditingBinding(null);
      const nextBindings = upsertBinding(bindings, saved);
      setBindings(nextBindings);
      setDetail((current) => current ? { ...current, binding: saved } : current);
      await refreshRepositoryKinds(workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice("Machine binding updated. Cloud identity and logical paths were unchanged.");
      await loadConversationPathAudits(nextBindings, undefined, workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      await loadProjectData(projectId, config, activeStorageId, workflowGeneration);
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const renameProject = async (projectId: string, alias: string | null): Promise<boolean> => {
    const workflowGeneration = hostWorkflowGeneration.current;
    setBusy(true);
    setError(null);
    try {
      const registration = registrations.find((candidate) => candidate.local_project_id === projectId);
      if (!registration) throw new Error("Project not found.");
      const updated = await projectSyncApi.renameProject(projectId, alias, registration.revision);
      if (hostWorkflowGeneration.current !== workflowGeneration) return false;
      setRegistrations((current) => current.map((candidate) => (
        candidate.local_project_id === projectId ? updated : candidate
      )));
      if (activeProjectId === projectId) {
        setDetail((current) => current ? { ...current, project: updated } : current);
      }
      setNotice(alias
        ? `Shown as “${alias}” on this computer.`
        : `Name reset to “${updated.display_name}”.`);
      return true;
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const planRestore = async (
    storageId: string,
    bundleId: string,
    targetBinding: ProjectBinding,
    displayName: string,
  ) => {
    const requestId = ++restoreRequest.current;
    // Pull review owns the workspace. Keep the activity drawer from covering
    // its approval controls; logs remain available from the sidebar.
    activityOpenRef.current = false;
    setActivityOpen(false);
    setBusy(true);
    setRestoreError(null);
    setRestoreResult(null);
    setDependencyResult(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(true);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
    setRestorePlan(null);
    setDependencyPlan(null);
    setReadiness(null);
    setRestoreBinding(targetBinding);
    setRestoreProjectName(displayName);
    try {
      // The project-data review is the primary Pull surface. Render it as soon as it
      // is ready; optional dependency/readiness checks must never hide a valid
      // restore plan or its Apply button.
      const request = { storageId, bundleId, binding: targetBinding };
      const review = await beginPullReview(projectSyncApi, request);
      if (restoreRequest.current !== requestId) return null;
      setRestorePlan(review.restorePlan);
      setNotice(`Pull review ready for ${displayName}. Nothing has been applied yet.`);

      // Supporting context fills in asynchronously after the workspace is usable.
      // Its lifecycle is request-scoped so closing or replanning cannot apply
      // stale results to another review.
      void review.support
        .then((support) => {
          if (restoreRequest.current !== requestId) return;
          setDependencyPlan(support.dependencyPlan);
          setReadiness(support.readiness);
          if (support.errors.length > 0) {
            setRestoreError(`The project changes are ready to apply. ${support.errors.join(" ")}`);
          }
        })
        .catch((reason) => {
          if (restoreRequest.current === requestId) {
            setRestoreError(`The project changes are ready to apply. Supporting checks: ${errorMessage(reason)}`);
          }
        })
        .finally(() => {
          if (restoreRequest.current === requestId) setReviewSupportLoading(false);
        });
      return null;
    } catch (reason) {
      const message = errorMessage(reason);
      if (restoreRequest.current === requestId) {
        setReviewSupportLoading(false);
        setRestoreError(message);
        setError(message);
      }
      return message;
    } finally {
      if (restoreRequest.current === requestId) setBusy(false);
    }
  };

  const beginProjectRestore = async (projectId: string, storageId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    setPendingPush(null);
    setPushError(null);
    setError(null);
    try {
      const nextDetail = await projectSyncApi.getProject(projectId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      if (!nextDetail) throw new Error("The selected local project no longer exists.");
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
      setDetail(nextDetail);
      if (!nextDetail.binding || nextDetail.binding.state !== "active") {
        setEditingBinding({
          local_project_id: nextDetail.project.local_project_id,
          bundle_id: nextDetail.project.bundle_id,
          project_root: nextDetail.binding?.project_root ?? "",
          profile_ids: nextDetail.binding?.profile_ids ?? defaultProfileIds(),
          expected_revision: nextDetail.binding?.revision ?? null,
        });
        setNotice(nextDetail.binding
          ? "Confirm this machine's project folder and provider profile before pulling."
          : "Choose this machine's project folder and provider profile before pulling.");
        return;
      }
      const failure = await planRestore(
        storageId,
        nextDetail.project.bundle_id,
        nextDetail.binding,
        projectLabel(nextDetail.project),
      );
      if (failure?.includes("does not exist")) {
        const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
        if (hostWorkflowGeneration.current !== workflowGeneration) return;
        if (bundles.length > 0) {
          bundles.sort((left, right) => {
            const leftMatches = !!nextDetail.project.repository_fingerprint
              && left.repository_fingerprint === nextDetail.project.repository_fingerprint;
            const rightMatches = !!nextDetail.project.repository_fingerprint
              && right.repository_fingerprint === nextDetail.project.repository_fingerprint;
            if (leftMatches !== rightMatches) return leftMatches ? -1 : 1;
            return (right.updated_at ?? 0) - (left.updated_at ?? 0);
          });
          setPendingBundleConnection({ projectId, storageId, matches: bundles, reason: "missing" });
        }
      }
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    }
  };

  const connectPendingBundle = async (match: BundleSnapshotSummary) => {
    if (!pendingBundleConnection) return;
    const pending = pendingBundleConnection;
    const project = registrations.find((candidate) => candidate.local_project_id === pending.projectId);
    const allowRepositoryMismatch = !!project?.repository_fingerprint
      && project.repository_fingerprint !== match.repository_fingerprint;
    await connectStorageBundle(
      pending.projectId,
      pending.storageId,
      match.bundle_id,
      allowRepositoryMismatch,
    );
  };

  const connectStorageBundle = async (
    projectId: string,
    storageId: string,
    bundleId: string,
    allowRepositoryMismatch = false,
  ) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const project = registrations.find((candidate) => (
      candidate.local_project_id === projectId
    ));
    if (!project) {
      setError("The selected local project no longer exists.");
      return;
    }
    if (project.bundle_id === bundleId) return;
    setBusy(true);
    setError(null);
    try {
      const connected = await projectSyncApi.connectProjectToRemoteBundle({
        local_project_id: projectId,
        storage_id: storageId,
        bundle_id: bundleId,
        expected_bundle_id: project.bundle_id,
        pinned: true,
        allow_repository_mismatch: allowRepositoryMismatch,
      });
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setPendingBundleConnection(null);
      const { nextConfig, hostIdentityReady } = await loadShell();
      if (!hostIdentityReady || hostWorkflowGeneration.current !== workflowGeneration) return;
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({
        ...current,
        [projectId]: storageId,
      }));
      await loadProjectData(projectId, nextConfig, storageId, workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      if (!connected.binding) {
        setNotice("Remote repo connected. Configure the project folder and provider profile before Pull.");
        return;
      }
      await planRestore(
        storageId,
        bundleId,
        connected.binding,
        projectLabel(connected.project),
      );
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const keepPendingBundle = async () => {
    if (!pendingBundleConnection) return;
    const workflowGeneration = hostWorkflowGeneration.current;
    const pending = pendingBundleConnection;
    setBusy(true);
    setError(null);
    try {
      if (pending.reason === "link") {
        await saveStorageLink(pending.projectId, pending.storageId, workflowGeneration);
        if (hostWorkflowGeneration.current !== workflowGeneration) return;
      } else {
        setPendingBundleConnection(null);
        setNotice("Kept the local-only repo identity. Review Push to publish it as a separate remote project.");
        await openPushChooser(pending.projectId, pending.storageId, workflowGeneration);
        return;
      }
      setPendingBundleConnection(null);
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const removeProject = async (projectId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const summary = projects.find((candidate) => candidate.local_project_id === projectId);
    const approved = await confirm(
      `Remove “${summary ? projectLabel(summary) : "this project"}” from Mallard?\n\nThe checkout and provider files stay on disk. Only this app's project registration, links, and active binding are removed.`,
      { title: "Remove project" },
    );
    if (!approved || hostWorkflowGeneration.current !== workflowGeneration) return;
    setBusy(true);
    setError(null);
    try {
      await projectSyncApi.removeProject(projectId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      await refresh(workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice(`${summary ? projectLabel(summary) : "Project"} removed from Mallard. Files were not deleted.`);
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const closePullReview = () => {
    restoreRequest.current += 1;
    setRestorePlan(null);
    setRestoreBinding(null);
    setDependencyPlan(null);
    setRestoreResult(null);
    setDependencyResult(null);
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
  };

  const configureProjectBinding = (projectId: string) => {
    const project = registrations.find((candidate) => candidate.local_project_id === projectId);
    if (!project) return;
    const projectBinding = bindings.find((candidate) => candidate.local_project_id === projectId) ?? null;
    setPendingPush(null);
    setPushError(null);
    closePullReview();
    setError(null);
    setActiveProjectId(projectId);
    setEditingBinding({
      local_project_id: projectId,
      bundle_id: project.bundle_id,
      project_root: projectBinding?.project_root ?? "",
      profile_ids: projectBinding?.profile_ids ?? defaultProfileIds(),
      expected_revision: projectBinding?.revision ?? null,
    });
  };

  const unlinkStorage = async (projectId: string, storageId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const project = projects.find((candidate) => candidate.local_project_id === projectId);
    const storage = config.storages.find((candidate) => candidate.id === storageId);
    if (!project || !storage) return;

    const projectName = projectLabel(project);
    const storageName = storage.name || "Storage";
    const approved = await confirm(
      `Unlink “${storageName}” from “${projectName}”?\n\nPush and Pull will stop for this pairing. Files and repositories already stored there will not be deleted, and you can link the storage again later.`,
      { title: "Unlink storage" },
    );
    if (!approved || hostWorkflowGeneration.current !== workflowGeneration) return;

    setBusy(true);
    setError(null);
    try {
      const removed = await projectSyncApi.removeLink(projectId, storageId);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;

      if (pendingPush?.projectId === projectId && pendingPush.storageId === storageId) {
        setPendingPush(null);
        setPushError(null);
      }
      if (restoreBinding?.local_project_id === projectId && restorePlan?.storage_id === storageId) {
        closePullReview();
      }
      if (pendingBundleConnection?.projectId === projectId && pendingBundleConnection.storageId === storageId) {
        setPendingBundleConnection(null);
      }

      const { nextConfig, hostIdentityReady } = await loadShell();
      if (!hostIdentityReady || hostWorkflowGeneration.current !== workflowGeneration) return;
      const remainingLinks = nextConfig.links.filter((link) => link.local_project_id === projectId);
      const rememberedStorageId = activeStorageByProject[projectId];
      const nextStorageId = rememberedStorageId
        && rememberedStorageId !== storageId
        && remainingLinks.some((link) => link.storage_id === rememberedStorageId)
        ? rememberedStorageId
        : remainingLinks[0]?.storage_id ?? null;

      setActiveStorageByProject((current) => {
        const next = { ...current };
        if (nextStorageId) next[projectId] = nextStorageId;
        else delete next[projectId];
        return next;
      });
      setActiveProjectId(projectId);
      await loadProjectData(projectId, nextConfig, nextStorageId, workflowGeneration);
      if (hostWorkflowGeneration.current !== workflowGeneration) return;
      setNotice(removed
        ? `${storageName} unlinked from ${projectName}. Stored files were not deleted.`
        : `${storageName} was already unlinked from ${projectName}.`);
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const applyReview = async (selection: PullReviewSelection) => {
    if (!restorePlan || !restoreBinding) return;
    const requestId = restoreRequest.current;
    const workflowGeneration = hostWorkflowGeneration.current;
    const currentRestorePlan = restorePlan;
    const currentDependencyPlan = dependencyPlan;
    // The review-ready notice describes the pre-Apply state. Clear it before
    // execution so a completed Pull can never still claim that nothing was
    // applied.
    setNotice(null);
    setFailedPullResourceIds((current) => {
      const next = new Set(current);
      for (const resourceId of selection.resourceIds) next.delete(resourceId);
      return next;
    });
    if (selection.restoreActionIds.length > 0) setRestoreResult(null);
    if (selection.dependencyActionIds.length > 0) setDependencyResult(null);
    openActivity();
    setBusy(true);
    setRestoreError(null);
    try {
      const result = await applyPullReview(
        projectSyncApi,
        currentRestorePlan,
        currentDependencyPlan,
        selection,
        (phase) => {
          if (restoreRequest.current === requestId) setPullApplyPhase(phase);
        },
      );
      if (restoreRequest.current !== requestId) return;
      const pullMutationCompleted = (
        result.restoreResult !== null || result.dependencyResult !== null
      ) && (
        result.restoreResult?.success ?? true
      ) && (
        result.dependencyResult?.success ?? true
      );
      if (result.success || (!result.error && pullMutationCompleted)) {
        setProjectTabResetEpoch((epoch) => epoch + 1);
      }
      if (result.restoreResult) {
        setRestoreResult(result.restoreResult);
      }
      if (result.dependencyResult) setDependencyResult(result.dependencyResult);
      if (result.restoreResult?.success
        || (result.dependencyResult?.applied_action_ids?.length ?? 0) > 0) {
        setHistoryRefreshEpoch((epoch) => epoch + 1);
      }
      if (result.readiness) setReadiness(result.readiness);
      if (result.error) setRestoreError(result.error);

      const resourceByAction = new Map<string, string>();
      for (const action of currentRestorePlan.actions) {
        resourceByAction.set(action.action_id, action.resource_id);
      }
      for (const action of currentDependencyPlan?.actions ?? []) {
        resourceByAction.set(action.action_id, action.resource_id);
      }
      const appliedActionIds = new Set([
        ...(result.restoreResult?.applied_action_ids ?? []),
        ...(result.dependencyResult?.applied_action_ids ?? []),
      ]);
      const failedActionIds = new Set([
        ...(result.restoreResult?.failed_actions ?? []).map((failure) => failure.action_id),
        ...(result.dependencyResult?.failed_actions ?? []).map((failure) => failure.action_id),
      ]);
      const interruptedActionIds = result.failedPhase === "restoring"
        ? selection.restoreActionIds
        : result.failedPhase === "installing"
          ? selection.dependencyActionIds
          : [];
      for (const actionId of interruptedActionIds) {
        if (!appliedActionIds.has(actionId)) failedActionIds.add(actionId);
      }
      if (appliedActionIds.size > 0 && !result.error && failedActionIds.size === 0) {
        const appliedLabel = `${appliedActionIds.size} approved change${appliedActionIds.size === 1 ? "" : "s"}`;
        setNotice(result.readiness?.state === "ready"
          ? `Pull finished for ${restoreProjectName}: applied ${appliedLabel}.`
          : `Pull finished for ${restoreProjectName}: applied ${appliedLabel}. Optional setup remains.`);
      }
      const failedResources = new Set(
        [...failedActionIds]
          .map((actionId) => resourceByAction.get(actionId))
          .filter((resourceId): resourceId is string => Boolean(resourceId)),
      );
      const selectedActions = [...selection.restoreActionIds, ...selection.dependencyActionIds];
      const successfulResources = new Set(selection.resourceIds.filter((resourceId) => {
        const actionIds = selectedActions.filter((actionId) => resourceByAction.get(actionId) === resourceId);
        return actionIds.length > 0 && actionIds.every((actionId) => appliedActionIds.has(actionId));
      }));
      setCompletedPullActionIds((current) => new Set([...current, ...appliedActionIds]));
      setCompletedPullResourceIds((current) => {
        const next = new Set(current);
        for (const resourceId of successfulResources) next.add(resourceId);
        for (const resourceId of failedResources) next.delete(resourceId);
        return next;
      });
      setFailedPullResourceIds((current) => {
        const next = new Set(current);
        for (const resourceId of selection.resourceIds) next.delete(resourceId);
        for (const resourceId of failedResources) next.add(resourceId);
        return next;
      });

      // Restore plans remain single-use. Refresh both aligned plans after a
      // partial pass so remaining setup is presented against one current
      // generation; dependency plans themselves also support direct retry.
      const approvedThisPass = new Set([
        ...selection.restoreActionIds,
        ...selection.dependencyActionIds,
      ]);
      const hasSkippedActions = [
        ...currentRestorePlan.actions.map((action) => action.action_id),
        ...(currentDependencyPlan?.actions ?? []).map((action) => action.action_id),
      ].some((actionId) => (
        !completedPullActionIds.has(actionId) && !approvedThisPass.has(actionId)
      ));
      if (!result.success || hasSkippedActions) {
        const nextRestore = await projectSyncApi.planRestore(
          currentRestorePlan.storage_id,
          currentRestorePlan.bundle_id,
          restoreBinding,
        );
        if (restoreRequest.current !== requestId) return;
        setRestorePlan(nextRestore);
        const support = await loadPullReviewSupport(projectSyncApi, nextRestore);
        if (restoreRequest.current !== requestId) return;
        setDependencyPlan(support.dependencyPlan);
        setReadiness(support.readiness);
        if (support.errors.length > 0) {
          setRestoreError(`Changes were applied, but remaining setup could not be prepared: ${support.errors.join(" ")}`);
        }
      }
      if (activeProjectId) {
        await loadProjectData(activeProjectId, config, activeStorageId, workflowGeneration);
      }
    } catch (reason) {
      if (restoreRequest.current === requestId) {
        setPullApplyPhase("complete");
        setRestoreError(errorMessage(reason));
      }
    } finally {
      if (restoreRequest.current === requestId) setBusy(false);
    }
  };

  const refreshRestore = async () => {
    if (!restorePlan || !restoreBinding) return;
    const requestId = restoreRequest.current;
    const currentPlan = restorePlan;
    setBusy(true);
    setRestoreError(null);
    try {
      const nextRestore = await projectSyncApi.planRestore(
        currentPlan.storage_id,
        currentPlan.bundle_id,
        restoreBinding,
      );
      if (restoreRequest.current !== requestId) return;
      const support = await loadPullReviewSupport(projectSyncApi, nextRestore);
      if (restoreRequest.current !== requestId) return;
      if (nextRestore.generation !== currentPlan.generation || nextRestore.manifest_sha256 !== currentPlan.manifest_sha256) {
        setRestoreResult(null);
        setDependencyResult(null);
        setPullApplyPhase("idle");
        setCompletedPullActionIds(new Set());
        setCompletedPullResourceIds(new Set());
        setFailedPullResourceIds(new Set());
      }
      setRestorePlan(nextRestore);
      setDependencyPlan(support.dependencyPlan);
      setReadiness(support.readiness);
      if (support.errors.length > 0) setRestoreError(support.errors.join(" "));
    } catch (reason) {
      if (restoreRequest.current === requestId) setRestoreError(errorMessage(reason));
    } finally {
      if (restoreRequest.current === requestId) setBusy(false);
    }
  };

  const saveStorageConfig = async (
    update: (current: SyncConfigV3) => SyncConfigV3,
    successNotice = "Project storage settings saved.",
    workflowGeneration = hostWorkflowGeneration.current,
  ): Promise<boolean> => {
    if (hostWorkflowGeneration.current !== workflowGeneration) return false;
    setBusy(true);
    setError(null);
    try {
      // Focused project operations can advance the global config revision.
      // Rebase this storage-only edit on the latest document instead of the
      // potentially stale React snapshot used to render the editor.
      const current = await projectSyncApi.getConfig();
      if (hostWorkflowGeneration.current !== workflowGeneration) return false;
      const saved = await projectSyncApi.saveConfig(update(current));
      if (hostWorkflowGeneration.current !== workflowGeneration) return false;
      setConfig(saved);
      setRegistrations(saved.projects);
      setDetail((current) => {
        if (!current) return current;
        const savedProject = saved.projects.find((candidate) => (
          candidate.local_project_id === current.project.local_project_id
        ));
        return {
          ...current,
          project: savedProject ?? current.project,
          links: saved.links.filter((link) => link.local_project_id === current.project.local_project_id),
        };
      });
      setActiveStorageByProject((current) => Object.fromEntries(Object.entries(current).filter(
        ([localProjectId, storageId]) => saved.links.some((link) => (
          link.local_project_id === localProjectId && link.storage_id === storageId
        )),
      )));
      setNotice(successNotice);
      return true;
    } catch (reason) {
      if (hostWorkflowGeneration.current === workflowGeneration) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowGeneration.current === workflowGeneration) setBusy(false);
    }
  };

  const saveInlineStorage = async (storage: SyncConfigV3["storages"][number]) => {
    return saveStorageConfig((current) => {
      const exists = current.storages.some((candidate) => candidate.id === storage.id);
      return {
        ...current,
        storages: exists
          ? current.storages.map((candidate) => candidate.id === storage.id ? storage : candidate)
          : [...current.storages, storage],
      };
    });
  };

  const removeStorage = async (storageId: string) => {
    const workflowGeneration = hostWorkflowGeneration.current;
    const storage = config.storages.find((candidate) => candidate.id === storageId);
    if (!storage) return;
    const storageName = storage.name || "Storage";
    const linkedProjects = config.links.filter((link) => link.storage_id === storageId).length;
    if (linkedProjects > 0) {
      setError(`${storageName} is linked to ${linkedProjects} project${linkedProjects === 1 ? "" : "s"}. Unlink it before removing the storage.`);
      return;
    }
    const approved = await confirm(
      `Remove “${storageName}” from Mallard?\n\nFiles already written to the storage location will not be deleted.`,
      { title: "Remove storage" },
    );
    if (!approved || hostWorkflowGeneration.current !== workflowGeneration) return;
    const removed = await saveStorageConfig((current) => {
      if (current.links.some((link) => link.storage_id === storageId)) {
        throw new Error(`${storageName} is linked to a project. Unlink it before removing the storage.`);
      }
      return {
        ...current,
        storages: current.storages.filter((candidate) => candidate.id !== storageId),
      };
    }, `${storageName} removed. Stored files were not deleted.`, workflowGeneration);
    if (removed && hostWorkflowGeneration.current === workflowGeneration) {
      setStorageEditorRequest((current) => ({
        mode: "close",
        requestId: (current?.requestId ?? 0) + 1,
      }));
    }
  };

  const openNewStorageConfiguration = () => {
    setSetupDraftId(null);
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    setStorageEditorRequest((current) => ({
      mode: "create",
      storageKind: "local",
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  const safePushSelection = pendingPush
    ? sanitizePushSelection(
      pushSelected,
      pushResources(pendingPush),
      pendingPush.threadComparison,
      pendingPush.capabilityReport,
      pendingPush.projectContentInventory,
    )
    : pushSelected;
  const pendingPushBlockingCount = pendingPush
    ? pushReviewBlockingCount(
      safePushSelection,
      pendingPush.threadComparison,
      pendingPush.capabilityReport,
      pendingPush.projectContentInventory,
      pushWarningAcknowledgements,
    )
    : 0;
  const pendingPushSteps = syncReviewSteps(pendingPush?.showProjectFiles ?? false);
  const inlineStorageReview = pendingPush ? {
    kind: "push" as const,
    projectId: pendingPush.projectId,
    storageId: pendingPush.storageId,
    onContinue: () => {
      if (pushReviewStep === "review") {
        void publishPendingPush();
        return;
      }
      setPushReviewStep(nextPushReviewStep(pushReviewStep, pendingPushSteps));
    },
    continueDisabled: pushReviewStep === "review"
      && ((safePushSelection.size === 0 && pushProjectContentRemovals.size === 0)
        || pendingPushBlockingCount > 0
        || projectContentLoading),
    onClose: () => {
      if (busy) return;
      setPendingPush(null);
      setPushError(null);
      setPushProjectContentRemovals(new Set());
      setPushWarningAcknowledgements(new Set());
    },
    content: (
      <PushResourceWorkspace
        resources={pushResources(pendingPush)}
        selected={pushSelected}
        projectDefaults={pendingPush.projectDefaults}
        busy={busy || projectContentLoading}
        error={pushError}
        project={pendingPush.project}
        binding={pendingPush.binding}
        storageId={pendingPush.storageId}
        storageName={pendingPush.storage.name}
        refreshEpoch={historyRefreshEpoch}
        threadComparison={pendingPush.threadComparison}
        capabilityReport={pendingPush.capabilityReport}
        projectContentInventory={pendingPush.projectContentInventory}
        projectContentScanned={pendingPush.projectContentScanned}
        projectContentLoading={projectContentLoading}
        projectContentRemovals={pushProjectContentRemovals}
        acknowledgedWarningDigests={pushWarningAcknowledgements}
        showProjectFiles={pendingPush.showProjectFiles}
        activeStep={pushReviewStep}
        onStepChange={setPushReviewStep}
        onToggle={(resourceId) => setPushSelected((current) => {
          const next = new Set(current);
          if (next.has(resourceId)) next.delete(resourceId);
          else next.add(resourceId);
          return next;
        })}
        onUseProjectDefaults={() => setPushSelected(new Set(pendingPush.projectDefaults))}
        onSelectionChange={setPushSelected}
        onScanProjectFiles={() => void scanPendingProjectFiles()}
        onToggleProjectContentRemoval={(resourceId) => {
          setPushProjectContentRemovals((current) => {
            const next = new Set(current);
            if (next.has(resourceId)) {
              next.delete(resourceId);
              const entry = pendingPush.projectContentInventory?.entries.find((candidate) => (
                candidate.descriptor.resource_id === resourceId
              ));
              if (entry?.selected_in_recipe) {
                setPushSelected((selection) => new Set(selection).add(resourceId));
              }
            } else {
              next.add(resourceId);
              setPushSelected((selection) => {
                const selected = new Set(selection);
                selected.delete(resourceId);
                return selected;
              });
            }
            return next;
          });
        }}
        onToggleProjectContentWarning={(warningDigest) => setPushWarningAcknowledgements((current) => {
          const next = new Set(current);
          if (next.has(warningDigest)) next.delete(warningDigest);
          else next.add(warningDigest);
          return next;
        })}
        onClear={() => setPushSelected(new Set((pendingPush.projectContentInventory?.entries ?? [])
          .filter((entry) => entry.storage_present && !pushProjectContentRemovals.has(entry.descriptor.resource_id))
          .map((entry) => entry.descriptor.resource_id)))}
        onClose={() => {
          if (busy) return;
          setPendingPush(null);
          setPushError(null);
          setPushProjectContentRemovals(new Set());
          setPushWarningAcknowledgements(new Set());
        }}
        onPush={() => void publishPendingPush()}
        onPull={() => {
          setPendingPush(null);
          setPushError(null);
          setPushProjectContentRemovals(new Set());
          setPushWarningAcknowledgements(new Set());
          void beginProjectRestore(pendingPush.projectId, pendingPush.storageId);
        }}
      />
    ),
  } : restorePlan && restoreBinding ? {
    kind: "pull" as const,
    projectId: restoreBinding.local_project_id,
    storageId: restorePlan.storage_id,
    onClose: closePullReview,
    content: (
      <RestorePlanView
        embedded
        project={projects.find((candidate) => candidate.local_project_id === restoreBinding.local_project_id) ?? undefined}
        storageName={config.storages.find((storage) => storage.id === restorePlan.storage_id)?.name ?? null}
        projectName={restoreProjectName}
        profileLabel={restoreProfileLabel}
        plan={restorePlan}
        binding={restoreBinding}
        dependencyPlan={dependencyPlan}
        readiness={readiness}
        restoreResult={restoreResult}
        dependencyResult={dependencyResult}
        phase={pullApplyPhase}
        supportLoading={reviewSupportLoading}
        completedActionIds={completedPullActionIds}
        completedResourceIds={completedPullResourceIds}
        failedResourceIds={failedPullResourceIds}
        busy={busy}
        error={restoreError}
        onApply={(selection) => void applyReview(selection)}
        onRefresh={() => void refreshRestore()}
        onBack={closePullReview}
      />
    ),
  } : null;

  if (!hostIdentityStatus && hostIdentityError) {
    return (
      <main className="v3-host-recovery" aria-labelledby="v3-host-recovery-title">
        <section className="v3-empty-state" role="alert">
          <div className="v3-empty-illustration" aria-hidden="true">
            <Icon name="alert-triangle" size={24} />
          </div>
          <h1 id="v3-host-recovery-title">Host identity unavailable</h1>
          <p>Mallard could not verify this installation, so sync remains paused.</p>
          <div className="v3-callout error">
            <Icon name="alert-triangle" size={15} />
            <span>{hostIdentityError}</span>
          </div>
          <button type="button" className="btn btn-primary" onClick={() => void refresh()} disabled={loading}>
            {loading ? "Checking…" : "Retry"}
          </button>
        </section>
      </main>
    );
  }

  if (hostIdentityStatus && !hostIdentityStatus.origin_key_available) {
    return (
      <main className="v3-host-recovery" aria-labelledby="v3-host-recovery-title">
        <section className="v3-empty-state" role="alertdialog" aria-modal="true">
          <div className="v3-empty-illustration" aria-hidden="true">
            <Icon name="alert-triangle" size={24} />
          </div>
          <span className="v3-eyebrow">Host identity recovery</span>
          <h1 id="v3-host-recovery-title">Register this installation as a new host</h1>
          <p>
            Mallard found local state whose protected origin key is unavailable. This usually means
            the state was copied from another machine or OS account, so this installation cannot
            safely represent the original host.
          </p>
          <p>
            Registering creates a new private identity and fresh project replica IDs. Existing
            bindings are detached until revalidated. Provider threads, project files, reviewed
            history, and cloud content are not renamed or deleted.
          </p>
          {hostIdentityError && (
            <div className="v3-callout error" role="alert">
              <Icon name="alert-triangle" size={15} />
              <span>{hostIdentityError}</span>
            </div>
          )}
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => void registerNewHost()}
            disabled={hostIdentityBusy}
            autoFocus
          >
            {hostIdentityBusy ? "Registering…" : "Register as new host"}
          </button>
        </section>
      </main>
    );
  }

  if (!hostIdentityShellReady) {
    return (
      <main className="v3-host-recovery" aria-labelledby="v3-host-check-title">
        <section className="v3-empty-state" role="status" aria-live="polite">
          <div className="v3-empty-illustration" aria-hidden="true">
            <Icon name="refresh" size={24} className="icon-spin" />
          </div>
          <h1 id="v3-host-check-title">Checking this installation…</h1>
        </section>
      </main>
    );
  }

  return (
    <div
      className={`v3-app${sidebarVisible ? "" : " sidebar-hidden"}${resizingSidebar ? " resizing-sidebar" : ""}`}
      style={{ "--v3-sidebar-width": `${sidebarWidth}px` } as CSSProperties}
    >
      <button
        type="button"
        className="v3-sidebar-toggle"
        onClick={toggleSidebar}
        title={`Toggle sidebar ${sidebarShortcutLabel()}`}
        aria-label={sidebarVisible ? "Hide sidebar" : "Show sidebar"}
        aria-controls="project-sidebar"
        aria-expanded={sidebarVisible}
        aria-keyshortcuts="Meta+B Control+B"
      >
        <Icon name={sidebarVisible ? "sidebar-collapse" : "sidebar-expand"} size={15} />
      </button>

      <ProjectSidebar
        projects={projects}
        drafts={setupDrafts}
        activeDraftId={setupDraftId}
        onSelectDraft={(draftId) => openSetupDraft(draftId)}
        onDiscardDraft={(draftId) => void discardSetupDraft(draftId)}
        storages={config.storages}
        storageUsage={Object.fromEntries(config.storages.map((storage) => [
          storage.id,
          config.links.filter((link) => link.storage_id === storage.id).length,
        ]))}
        activeProjectId={activeProjectId}
        activeStorageId={activeStorageSettingsId}
        loading={loading}
        busy={busy || !!inlineStorageReview}
        activityOpen={activityOpen}
        unreadLogs={unreadLogs}
        onSelectProject={(id) => {
          setSetupDraftId(null);
          closeWorkspaceSettings();
          if (restorePlan) closePullReview();
          void selectProject(id);
        }}
        onRemoveProject={(id) => void removeProject(id)}
        onToggleActivity={() => {
          const next = !activityOpen;
          activityOpenRef.current = next;
          if (next) setUnreadLogs(0);
          setActivityOpen(next);
        }}
        onAddProject={() => void beginAddProject()}
        onOpenStorage={(storageId, settingsExpanded) => {
          setSetupDraftId(null);
          setPendingPush(null);
          setPushError(null);
          if (restorePlan) closePullReview();
          setStorageEditorRequest((current) => ({
            mode: "open",
            storageId,
            settingsExpanded,
            requestId: (current?.requestId ?? 0) + 1,
          }));
        }}
        onRemoveStorage={(storageId) => void removeStorage(storageId)}
        onAddStorage={openNewStorageConfiguration}
      />

      <div
        className="v3-sidebar-resizer"
        role="separator"
        aria-label="Resize sidebar"
        aria-orientation="vertical"
        aria-valuemin={MIN_PROJECT_SIDEBAR_WIDTH}
        aria-valuemax={availableProjectSidebarWidth()}
        aria-valuenow={sidebarWidth}
        tabIndex={0}
        onPointerDown={startSidebarResize}
        onPointerMove={continueSidebarResize}
        onPointerUp={finishSidebarResize}
        onPointerCancel={finishSidebarResize}
        onKeyDown={resizeSidebarWithKeyboard}
        onDoubleClick={() => setSidebarWidth(DEFAULT_PROJECT_SIDEBAR_WIDTH)}
      />

      <div className={`v3-workspace${resizingLog ? " resizing-log" : ""}`}>
        <header className="v3-titlebar" data-tauri-drag-region>
          <div className="v3-titlebar-context" data-tauri-drag-region title={workspaceTitle}>
            <Icon name={workspaceIcon} size={16} />
            <strong>{workspaceTitle}</strong>
            {activeDraftSummary && <span>Draft setup</span>}
          </div>
          <div className="v3-titlebar-actions">
            <button
              type="button"
              className="v3-theme-button"
              onClick={() => onThemeChange(theme === "dark" ? "light" : "dark")}
            >
              {theme === "dark" ? "Light" : "Dark"} theme
            </button>
            <div ref={appMenuRef} className={`storage-link-overflow${appMenuOpen ? " open" : ""}`}>
              <button
                type="button"
                className="storage-link-more"
                aria-haspopup="menu"
                aria-expanded={appMenuOpen}
                aria-controls="v3-app-actions-menu"
                title="App actions"
                aria-label="App actions"
                onClick={() => setAppMenuOpen((current) => !current)}
              >
                <Icon name="more" size={16} />
              </button>
              <div
                id="v3-app-actions-menu"
                className="storage-link-actions-menu"
                role="menu"
                aria-label="App actions"
                hidden={!appMenuOpen}
              >
                <button
                  type="button"
                  role="menuitem"
                  className="storage-link-actions-menu-item"
                  disabled={loading || busy}
                  title="Reload all Mallard data from disk and storage; project files are not modified"
                  onClick={() => {
                    setAppMenuOpen(false);
                    void refresh();
                  }}
                >
                  <Icon name="refresh" size={14} className={loading ? "icon-spin" : undefined} />
                  <span>Reload app data</span>
                </button>
              </div>
            </div>
          </div>
        </header>

        {backendError && (
          <div className="v3-backend-banner">
            <Icon name="alert-triangle" size={15} />
            <span><strong>Project-sync backend unavailable.</strong> {backendError}</span>
            <button type="button" className="btn" onClick={() => void refresh()}>Retry</button>
          </div>
        )}
        {notice && (
          <div className="v3-notice" role="status" aria-live="polite">
            <Icon name="check-circle" size={14} />
            <span>{notice}</span>
            <button type="button" className="v3-notice-dismiss" onClick={() => setNotice(null)}
              title="Dismiss" aria-label="Dismiss notification">
              <Icon name="x" size={12} />
            </button>
          </div>
        )}

        <ProjectLinksWorkspace
            projects={projects}
            activeProjectId={activeProjectId}
            activeStorageId={activeStorageId}
            bindings={bindings}
            profiles={profiles}
            storages={config.storages}
            links={config.links}
            loading={loading}
            busy={busy}
            error={error}
            conversationPathAudits={conversationPathAudits}
            conversationPathAuditErrors={conversationPathAuditErrors}
            conversationPathAuditLoading={conversationPathAuditLoading}
            onSelectStorage={(projectId, storageId) => selectStorage(projectId, storageId)}
            onLinkStorage={(projectId, storageId) => linkStorage(projectId, storageId)}
            onUnlinkStorage={(projectId, storageId) => unlinkStorage(projectId, storageId)}
            onPush={(projectId, storageId) => openPushChooser(projectId, storageId)}
            onPull={(projectId, storageId) => beginProjectRestore(projectId, storageId)}
            onConfigureBinding={configureProjectBinding}
            onRepairConversationPaths={(projectId) => repairConversationPaths(projectId)}
            onRenameProject={(projectId, alias) => renameProject(projectId, alias)}
            onRefresh={() => void refresh()}
            onAddProject={() => void beginAddProject()}
            onOpenStorageSettings={openNewStorageConfiguration}
            onSaveStorage={saveInlineStorage}
            storageEditorRequest={storageEditorRequest}
            onStorageEditorRequestHandled={() => setStorageEditorRequest(null)}
            onStorageEditorChange={setActiveStorageSettingsId}
            historyRefreshEpoch={historyRefreshEpoch}
            projectTabResetEpoch={projectTabResetEpoch}
            inlineStorageReview={inlineStorageReview}
            newProjectSetup={setupDraftId ? (
              <ProjectSetupWorkspace
                key={setupDraftId}
                draftId={setupDraftId}
                workflowGeneration={hostWorkflowGeneration.current}
                isWorkflowCurrent={(workflowGeneration) => (
                  hostWorkflowGeneration.current === workflowGeneration
                )}
                profiles={profiles}
                projects={projects}
                storages={config.storages}
                busy={busy}
                onClose={() => void closeSetupDraft()}
                onDiscard={(draftId) => void discardSetupDraft(draftId)}
                onAddStorage={openNewStorageConfiguration}
                onFinalized={(detail, completion, workflowGeneration) => (
                  void completeSetup(detail, completion, workflowGeneration)
                )}
              />
            ) : null}
          />

        <div
          className={`log-drawer v3-log-drawer${activityOpen ? " open" : ""}`}
          style={{ height: activityOpen ? logHeight : undefined }}
        >
          {activityOpen && (
            <div
              className="log-resizer"
              role="separator"
              aria-label="Resize log"
              aria-orientation="horizontal"
              onMouseDown={startLogResize}
              onDoubleClick={() => setLogHeight(240)}
            />
          )}
          <LogPanel
            lines={logLines}
            typeFilters={logTypeFilters}
            levelFilter={logLevelFilter}
            search={logSearchInput}
            loading={logLoading}
            loadingOlder={logLoadingOlder}
            hasOlder={!!logCursor}
            error={logLoadError}
            scrollToBottomEpoch={logScrollToBottomEpoch}
            onTypeFiltersChange={setLogTypeFilters}
            onLevelFilterChange={setLogLevelFilter}
            onSearchChange={setLogSearchInput}
            onLoadOlder={() => logCursor && void loadActivityLogs(logCursor)}
            onManage={() => setLogManagerOpen(true)}
            onClose={() => {
              activityOpenRef.current = false;
              setActivityOpen(false);
            }}
          />
        </div>
      </div>

      {pendingBundleConnection && pendingConnectionProject && pendingConnectionStorage && (
        <BundleConnectionDialog
          projectName={projectLabel(pendingConnectionProject)}
          currentBundleId={pendingConnectionProject.bundle_id}
          projectFingerprint={pendingConnectionProject.repository_fingerprint}
          storage={pendingConnectionStorage}
          matches={pendingBundleConnection.matches}
          reason={pendingBundleConnection.reason}
          busy={busy}
          error={error}
          onCancel={() => setPendingBundleConnection(null)}
          onUseExisting={(match) => void connectPendingBundle(match)}
          onKeepCurrent={() => void keepPendingBundle()}
        />
      )}

      {editingBinding && (
        <ProjectBindingEditor
          title={`Project setup for ${activeSummary ? projectLabel(activeSummary) : "project"}`}
          description="Choose this machine's checkout and one agent profile: Codex or Claude. Existing files and provider state are never moved or deleted."
          binding={editingBinding}
          profiles={profiles}
          busy={busy}
          error={error}
          actionLabel="Save project setup"
          onCancel={() => setEditingBinding(null)}
          onAddProfile={chooseAndCreateProfile}
          onSubmit={(next) => void saveRemap(next)}
        />
      )}

      {logManagerOpen && (
        <LogManagerDialog
          onClose={() => setLogManagerOpen(false)}
          onLogsChanged={() => void loadActivityLogs()}
        />
      )}

    </div>
  );
}
