import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  ActivityLogLevel,
  ActivityLogType,
  AppTheme,
  BundleFetchProgress,
  BundleRecipe,
  BundleReadiness,
  BundleSnapshotSummary,
  CodexConversationPathAudit,
  HostIdentityStatus,
  LocalProjectRegistration,
  LocalProjectSummary,
  LogLine,
  ProjectBinding,
  ProjectContentInventory,
  ProjectDetail,
  ProjectStorageLink,
  ProjectSyncConfigChangedEvent,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfile,
  ProviderProfileSummary,
  ResourceInventory as ResourceInventoryModel,
  RestorePlan,
  RestoreResult,
  SetupDraftSummary,
  SiblingBundleStatus,
  StorageConfig,
  StorageSetupDraft,
  StorageSetupDraftsChangedEvent,
  SyncConfig,
  ThreadSyncComparison,
} from "../../types";
import { PROJECT_SYNC_SCHEMA } from "../../types";
import Icon from "../Icons";
import Notice, { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import LogPanel, { ACTIVITY_LOG_TYPES } from "../LogPanel";
import BundleConnectionDialog from "./BundleConnectionDialog";
import SettingsPage from "./SettingsPage";
import { getPreference } from "../../preferences";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import ProjectBindingEditor, {
  type BindingProfileSelection,
  type ProjectBindingDraft,
  type ProjectBindingSubmission,
} from "./ProjectBindingEditor";
import ProjectLinksWorkspace, {
  storageSubtitle,
  storageSubtitleIsPath,
  type StorageNotice,
} from "./ProjectLinksWorkspace";
import StorageIndexPage from "./StorageIndexPage";
import type { PathBarMenuGroup } from "../sync-review/SyncPathBar";
import PushResourceWorkspace, {
  includeRequiredProjectContentDirectories,
  preserveSyncedPushRecipeEntries,
  pullRequiredCopy,
  pushReviewBlockingCount,
  recommendedPushSelection,
  sanitizePushSelection,
} from "./PushResourceWorkspace";
import ProjectSetupWorkspace from "./ProjectSetupWorkspace";
import ProjectSidebar from "./ProjectSidebar";
import type {
  ActiveProjectSubview,
  OpenSessionDetailRequest,
} from "./session-detail/sessionDetailModel";
import StorageCatalogPage, {
  type CatalogStorageOption,
} from "../storage-catalog/StorageCatalogPage";
import CatalogIndexPage from "../storage-catalog/CatalogIndexPage";
import AddCatalogScopeWorkspace from "../storage-catalog/AddCatalogScopeWorkspace";
import { storageCatalogApi } from "../storage-catalog/api";
import type { CatalogScopeView } from "../storage-catalog/types";
import {
  catalogScopeLabel,
  catalogScopeRemovalMessage,
  catalogScopeRemovalRequiresCleanup,
} from "../storage-catalog/model";
import RestorePlanView, { type PullReviewPrimary } from "./RestorePlanView";
import StartupLoadingScreen, { type StartupStage } from "./StartupLoadingScreen";
import { type SyncReviewStep } from "./reviewSteps";
import { buildPullReviewItems } from "./pullReviewModel";
import { buildPullOutcome, buildPushOutcome, type SyncOutcome } from "./syncOutcome";
import { projectSyncApi } from "./api";
import { r2S3ApiUrl } from "./StorageSettings";
import {
  applyPullReview,
  beginPullReview,
  hasPendingRestoreMutations,
  loadPullReviewSupport,
  type PullApplyPhase,
  type PullReviewSelection,
} from "./pullReviewFlow";
import {
  inventoryResources,
  projectLabel,
  recipeSelection,
  recipeWithSelection,
  type RenameResult,
} from "./model";

interface Props {
  theme: AppTheme;
  onThemeChange: (theme: AppTheme) => void;
}

const EMPTY_CONFIG: SyncConfig = {
  schema: PROJECT_SYNC_SCHEMA,
  revision: 0,
  storages: [],
  projects: [],
  links: [],
};

function sameStorageConfig(left: SyncConfig["storages"][number], right: SyncConfig["storages"][number]): boolean {
  return left.id === right.id
    && left.name === right.name
    && left.kind === right.kind
    && left.bucket === right.bucket
    && left.access_key_id === right.access_key_id
    && left.secret_access_key === right.secret_access_key
    && left.account_id === right.account_id
    && left.s3_endpoint === right.s3_endpoint
    && left.region === right.region
    && left.local_dir === right.local_dir
    && left.supports_conditional_writes === right.supports_conditional_writes;
}

function sameR2Connection(left: SyncConfig["storages"][number], right: SyncConfig["storages"][number]): boolean {
  return left.kind === "s3"
    && right.kind === "s3"
    && left.bucket === right.bucket
    && left.access_key_id === right.access_key_id
    && left.secret_access_key === right.secret_access_key
    && left.account_id === right.account_id
    && left.s3_endpoint === right.s3_endpoint
    && left.region === right.region;
}

export function applyStorageDraft(
  storages: SyncConfig["storages"],
  draft: SyncConfig["storages"][number],
  expectedBaseline: SyncConfig["storages"][number] | null,
): SyncConfig["storages"] {
  const existing = storages.find((candidate) => candidate.id === draft.id) ?? null;
  if (expectedBaseline) {
    if (
      expectedBaseline.id !== draft.id
      || !existing
      || !sameStorageConfig(existing, expectedBaseline)
    ) {
      throw new Error(
        "This storage changed outside the editor. Reload its current configuration before saving.",
      );
    }
  } else if (existing) {
    throw new Error(
      "A storage with this identifier was added outside the editor. Reload before saving.",
    );
  }
  return existing
    ? storages.map((candidate) => candidate.id === draft.id ? draft : candidate)
    : [...storages, draft];
}

export function shouldReloadConfigRevision(
  currentRevision: number,
  eventRevision: number,
  hasValidSnapshot: boolean,
): boolean {
  return !hasValidSnapshot || eventRevision > currentRevision;
}

const PROJECT_SIDEBAR_WIDTH_KEY = "mallard.project-sidebar-width";
const PROJECT_SIDEBAR_VISIBILITY_KEY = "mallard.project-sidebar-visible";
const ACTIVE_STORAGE_BY_PROJECT_KEY = "mallard.active-storage-by-project";
const DEFAULT_PROJECT_SIDEBAR_WIDTH = 298;
const MIN_PROJECT_SIDEBAR_WIDTH = 220;
const MAX_PROJECT_SIDEBAR_WIDTH = 560;
const MIN_PROJECT_WORKSPACE_WIDTH = 580;
const FOCUS_RECONCILE_MIN_AWAY_MS = 5000;

function clampSidebarWidth(width: number, maxWidth = MAX_PROJECT_SIDEBAR_WIDTH): number {
  return Math.min(maxWidth, Math.max(MIN_PROJECT_SIDEBAR_WIDTH, width));
}

function availableProjectSidebarWidth(): number {
  return Math.max(
    MIN_PROJECT_SIDEBAR_WIDTH,
    Math.min(MAX_PROJECT_SIDEBAR_WIDTH, window.innerWidth - MIN_PROJECT_WORKSPACE_WIDTH),
  );
}

function storedSidebarWidth(): number {
  try {
    const value = Number.parseInt(window.localStorage.getItem(PROJECT_SIDEBAR_WIDTH_KEY) ?? "", 10);
    return Number.isFinite(value)
      ? clampSidebarWidth(value, availableProjectSidebarWidth())
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

/** Toast outcome. `ok` confirms completion; `attention` reports a stop that
 *  needs action. Hard failures use `error`. */
type SyncNoticeTone = "ok" | "attention";

interface SyncNotice {
  message: string;
  tone: SyncNoticeTone;
}

interface PendingBundleConnection {
  projectId: string;
  storageId: string;
  matches: BundleSnapshotSummary[];
  reason: "link" | "missing" | "sibling";
}

/** Sibling detection result for one project × storage pair. */
interface SiblingBundleNotice {
  projectId: string;
  storageId: string;
  status: SiblingBundleStatus;
}

interface PendingPush {
  projectId: string;
  storageId: string;
  projectName: string;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  storage: StorageConfig;
  inventory: ResourceInventoryModel;
  savedRecipe: BundleRecipe | null;
  projectDefaults: Set<string>;
  threadComparison?: ThreadSyncComparison | null;
  projectContentInventory: ProjectContentInventory | null;
  projectContentScanned: boolean;
  showProjectFiles: boolean;
  conversationPushPlanId: string | null;
  conversationActionIds: Record<string, string>;
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

function defaultProfileIds(): Record<ProviderKey, string> {
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

export default function ProjectSync({ theme, onThemeChange }: Props) {
  const [config, setConfig] = useState<SyncConfig>(EMPTY_CONFIG);
  const [registrations, setRegistrations] = useState<LocalProjectRegistration[]>([]);
  const [repositoryKinds, setRepositoryKinds] = useState<Record<string, boolean>>({});
  const [bindings, setBindings] = useState<ProjectBinding[]>([]);
  const [profiles, setProfiles] = useState<ProviderProfileSummary[]>([]);
  const [agentProviders, setAgentProviders] = useState<ProviderDescriptor[]>([]);
  const [conversationPathAudits, setConversationPathAudits] = useState<Record<string, CodexConversationPathAudit>>({});
  const [conversationPathAuditErrors, setConversationPathAuditErrors] = useState<Record<string, string>>({});
  const [conversationPathAuditLoading, setConversationPathAuditLoading] = useState(true);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [activeProjectSubview, setActiveProjectSubview] = useState<ActiveProjectSubview>({ kind: "overview" });
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [inventory, setInventory] = useState<ResourceInventoryModel | null>(null);
  const [readiness, setReadiness] = useState<BundleReadiness | null>(null);
  const [activeStorageByProject, setActiveStorageByProject] = useState<Record<string, string>>(storedActiveStorages);
  const [activeStorageSettingsId, setActiveStorageSettingsId] = useState<string | null>(null);
  // The page the storage trail is rooted at: every storage page starts its
  // path with a Storages crumb, and this is where that crumb goes.
  const [storageIndexOpen, setStorageIndexOpen] = useState(false);
  // The Catalog menu starts empty and grows one row per (provider profile ×
  // resource kind) the user adds. Each registered resource owns its own
  // storage links; there is no storage-wide Catalog selection.
  const [catalogScopes, setCatalogScopes] = useState<CatalogScopeView[]>([]);
  const [catalogScopeId, setCatalogScopeId] = useState<string | null>(null);
  const [addingCatalogScope, setAddingCatalogScope] = useState(false);
  const [catalogIndexOpen, setCatalogIndexOpen] = useState(false);
  const [storageIssues, setStorageIssues] = useState<Record<string, string>>({});
  const [storageEditorRequest, setStorageEditorRequest] = useState<
    | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
    | { mode: "create"; storageKind: "local" | "s3"; requestId: number }
    | { mode: "resume-draft"; draft: StorageSetupDraft; requestId: number }
    | { mode: "close"; requestId: number }
    | null
  >(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  busyRef.current = busy;
  const [error, setError] = useState<string | null>(null);
  const [catalogRemovalErrorTarget, setCatalogRemovalErrorTarget] = useState<{
    label: string;
    message: string;
    scopeId: string;
  } | null>(null);
  const [catalogStorageMenuRequest, setCatalogStorageMenuRequest] = useState<{
    requestId: number;
    scopeId: string;
  } | null>(null);
  const [backendError, setBackendError] = useState<string | null>(null);
  const [notice, setNotice] = useState<SyncNotice | null>(null);
  const notify = (message: string, tone: SyncNoticeTone = "ok") => setNotice({ message, tone });
  const [hostIdentityStatus, setHostIdentityStatus] = useState<HostIdentityStatus | null>(null);
  const [hostIdentityShellReady, setHostIdentityShellReady] = useState(false);
  const [hostIdentityError, setHostIdentityError] = useState<string | null>(null);
  const [startupStage, setStartupStage] = useState<StartupStage>("installation");
  const [pendingBundleConnection, setPendingBundleConnection] = useState<PendingBundleConnection | null>(null);
  const [siblingNotice, setSiblingNotice] = useState<SiblingBundleNotice | null>(null);
  const [pendingPush, setPendingPush] = useState<PendingPush | null>(null);
  const [pushSelected, setPushSelected] = useState<Set<string>>(new Set());
  const [pushProjectContentRemovals, setPushProjectContentRemovals] = useState<Set<string>>(new Set());
  const [pushWarningAcknowledgements, setPushWarningAcknowledgements] = useState<Set<string>>(new Set());
  const [projectContentLoading, setProjectContentLoading] = useState(false);
  const [pushError, setPushError] = useState<string | null>(null);
  const [pushReviewStep, setPushReviewStep] = useState<SyncReviewStep>("history");
  // A gate the push (or a pull plan) cannot get past never opens a review;
  // it lands as a banner on the storage card with the way forward on it.
  const [storageNotice, setStorageNotice] = useState<StorageNotice | null>(null);
  // What produced the current Pull outcome, so Recheck can rebuild the banner
  // against fresh readiness instead of replanning the whole Pull.
  const pullOutcomeInputs = useRef<Parameters<typeof buildPullOutcome>[0] | null>(null);
  const [logLines, setLogLines] = useState<LogLine[]>([]);
  const [logTypeFilters, setLogTypeFilters] = useState<ActivityLogType[]>(() => [...ACTIVITY_LOG_TYPES]);
  const [logLevelFilter, setLogLevelFilter] = useState<ActivityLogLevel | "all">("all");
  const [logSearchInput, setLogSearchInput] = useState("");
  const [logSearch, setLogSearch] = useState("");
  const [logCursor, setLogCursor] = useState<string | null>(null);
  const [logLoading, setLogLoading] = useState(true);
  const [logLoadingOlder, setLogLoadingOlder] = useState(false);
  const [logLoadError, setLogLoadError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
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
  const logDrawerRef = useRef<HTMLDivElement>(null);
  const logFiltersRef = useRef({ types: logTypeFilters, level: logLevelFilter, search: logSearch });
  const logRequestRef = useRef(0);
  const sidebarResizeRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null);

  const [setupDrafts, setSetupDrafts] = useState<SetupDraftSummary[]>([]);
  const [storageSetupDrafts, setStorageSetupDrafts] = useState<StorageSetupDraft[]>([]);
  const [setupDraftId, setSetupDraftId] = useState<string | null>(null);
  const [editingBinding, setEditingBinding] = useState<ProjectBindingDraft | null>(null);

  const [restorePlan, setRestorePlan] = useState<RestorePlan | null>(null);
  const [fetchProgress, setFetchProgress] = useState<BundleFetchProgress | null>(null);
  const [restoreBinding, setRestoreBinding] = useState<ProjectBinding | null>(null);
  const [restoreProjectName, setRestoreProjectName] = useState("project");
  const [restoreResult, setRestoreResult] = useState<RestoreResult | null>(null);
  const [historyRefreshEpoch, setHistoryRefreshEpoch] = useState(0);
  const [storageRefreshEpoch, setStorageRefreshEpoch] = useState(0);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [pullApplyPhase, setPullApplyPhase] = useState<PullApplyPhase>("idle");
  // The open Pull review's footer primary, mirrored out so the storage card's
  // Pull button runs the same apply under the same gating.
  const [pullReviewPrimary, setPullReviewPrimary] = useState<PullReviewPrimary | null>(null);
  const [reviewSupportLoading, setReviewSupportLoading] = useState(false);
  const [completedPullActionIds, setCompletedPullActionIds] = useState<Set<string>>(new Set());
  const [completedPullResourceIds, setCompletedPullResourceIds] = useState<Set<string>>(new Set());
  const [failedPullResourceIds, setFailedPullResourceIds] = useState<Set<string>>(new Set());
  const restoreRequest = useRef(0);
  const shellRequest = useRef(0);
  /* A project switch supersedes the load in flight. The workflow epoch alone
     cannot see it — it only moves when the host identity changes — so a slow
     project's detail and inventory could land on the currently selected one. */
  const projectRequest = useRef(0);
  const hostWorkflowEpoch = useRef(0);
  const configRef = useRef<SyncConfig>(EMPTY_CONFIG);
  const configSnapshotValidRef = useRef(false);
  const configEventRequest = useRef(0);
  const storageSetupDraftRequest = useRef(0);
  const readyHostIdentity = useRef<{ hostId: string; revision: number } | null>(null);

  const adoptConfigSnapshot = useCallback((saved: SyncConfig) => {
    if (
      configSnapshotValidRef.current
      && saved.revision < configRef.current.revision
    ) return;
    configRef.current = saved;
    configSnapshotValidRef.current = true;
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
        links: saved.links.filter((link) => (
          link.local_project_id === current.project.local_project_id
        )),
      };
    });
    setActiveStorageByProject((current) => Object.fromEntries(Object.entries(current).filter(
      ([localProjectId, storageId]) => saved.links.some((link) => (
        link.local_project_id === localProjectId && link.storage_id === storageId
      )),
    )));
  }, []);

  const closeProjectSubview = useCallback(() => {
    setActiveProjectSubview({ kind: "overview" });
  }, []);

  const openSessionDetail = useCallback((
    projectId: string,
    request: OpenSessionDetailRequest,
  ) => {
    setActiveProjectSubview({
      kind: "session_detail",
      localProjectId: projectId,
      resourceId: request.resourceId,
      occurrenceKey: request.occurrenceKey,
    });
  }, []);

  const toggleSidebar = useCallback(() => {
    sidebarResizeRef.current = null;
    setResizingSidebar(false);
    setSidebarVisible((current) => !current);
  }, []);

  const updateStorageIssue = useCallback((storageId: string, issue: string | null) => {
    if (issue) setNotice(null);
    setStorageIssues((current) => {
      if (issue) {
        return current[storageId] === issue ? current : { ...current, [storageId]: issue };
      }
      if (!(storageId in current)) return current;
      const next = { ...current };
      delete next[storageId];
      return next;
    });
  }, []);

  useEffect(() => {
    const storageIds = new Set(config.storages.map((storage) => storage.id));
    setStorageIssues((current) => {
      const next = Object.fromEntries(
        Object.entries(current).filter(([storageId]) => storageIds.has(storageId)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [config.storages]);

  useEffect(() => {
    configRef.current = config;
  }, [config]);

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

      setSidebarWidth((current) => clampSidebarWidth(current, availableProjectSidebarWidth()));
    };
    window.addEventListener("resize", fitSidebarToWindow);
    return () => window.removeEventListener("resize", fitSidebarToWindow);
  }, []);

  useEffect(() => {
    // The drag handler clamps logHeight against the window, but only while
    // dragging — shrinking the window afterwards could hold a drawer taller
    // than the space above it and starve the main pane to nothing.
    const fitLogToWindow = () => {
      setLogHeight((current) => Math.min(current, Math.max(140, window.innerHeight - 120)));
    };
    window.addEventListener("resize", fitLogToWindow);
    return () => window.removeEventListener("resize", fitLogToWindow);
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
    if (activityOpen) void loadActivityLogs();
  }, [activityOpen, loadActivityLogs]);

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

  useEffect(() => {
    // Live transfer progress for Pull plan/apply and Push uploads. The final
    // per-bundle event (index == total) clears the readout so the button
    // returns to its static busy label while the remaining work finishes.
    const unlisten = listen<BundleFetchProgress>("project-sync-fetch-progress", (event) => {
      setFetchProgress(event.payload.index >= event.payload.total_objects ? null : event.payload);
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisten = listen<ProjectSyncConfigChangedEvent>(
      "project-sync-config-changed",
      (event) => {
        if (!shouldReloadConfigRevision(
          configRef.current.revision,
          event.payload.revision,
          configSnapshotValidRef.current,
        )) return;
        const requestId = ++configEventRequest.current;
        void projectSyncApi.getConfig().then((nextConfig) => {
          if (
            disposed
            || requestId !== configEventRequest.current
            || nextConfig.schema !== PROJECT_SYNC_SCHEMA
            || !shouldReloadConfigRevision(
              configRef.current.revision,
              nextConfig.revision,
              configSnapshotValidRef.current,
            )
          ) return;
          adoptConfigSnapshot(nextConfig);
        }).catch(() => {
          // The native watcher validates before emitting. A transient read
          // failure leaves the current, already validated snapshot visible.
        });
      },
    );
    return () => {
      disposed = true;
      configEventRequest.current += 1;
      void unlisten.then((dispose) => dispose());
    };
  }, [adoptConfigSnapshot]);

  const openActivity = () => {
    activityOpenRef.current = true;
    setActivityOpen(true);
    setUnreadLogs(0);
    setLogScrollToBottomEpoch((epoch) => epoch + 1);
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
      is_git_repository: repositoryKinds[registration.local_project_id],
    };
  }), [activeProjectId, bindings, config.links, profiles, registrations, repositoryKinds, resources]);

  const activeSummary = projects.find((candidate) => candidate.local_project_id === activeProjectId) ?? null;
  const activeDraftSummary = setupDraftId
    ? setupDrafts.find((candidate) => candidate.draft_id === setupDraftId) ?? null
    : null;
  const activeCatalogScope = catalogScopeId && !setupDraftId && !addingCatalogScope && !catalogIndexOpen
    ? catalogScopes.find((candidate) => candidate.scope.scope_id === catalogScopeId) ?? null
    : null;
  useEffect(() => {
    if (catalogStorageMenuRequest?.scopeId !== activeCatalogScope?.scope.scope_id) return;
    setCatalogStorageMenuRequest(null);
  }, [activeCatalogScope?.scope.scope_id, catalogStorageMenuRequest]);
  const catalogStorages: CatalogStorageOption[] = config.storages.map((storage) => ({
    id: storage.id,
    name: storage.name,
    kind: storage.kind,
    detail: storageSubtitle(storage),
    detailIsPath: storageSubtitleIsPath(storage),
  }));
  // The project workspace names itself in its path bar, so the titlebar stays
  // silent there and speaks only for the screens that carry no trail.
  const workspaceTitle = activeDraftSummary?.display_name
    ?? (addingCatalogScope ? "Add Catalog resource" : null)
    ?? (activeSummary ? null : "Projects");
  const workspaceIcon = activeDraftSummary
    ? "folder"
    : addingCatalogScope
      ? "catalog"
      : "folder";
  // Project and storage workspaces use a path bar as the titlebar. The storage
  // page reports its state through activeStorageSettingsId after the editor
  // effect confirms it.
  const showsPathBar = !activeDraftSummary
    && !addingCatalogScope
    && !setupDraftId
    && (!!activeSummary
      || !!activeCatalogScope
      || catalogIndexOpen
      || storageIndexOpen
      || settingsOpen
      || activeStorageSettingsId != null);
  const pendingConnectionProject = pendingBundleConnection
    ? registrations.find((candidate) => candidate.local_project_id === pendingBundleConnection.projectId) ?? null
    : null;
  const pendingConnectionStorage = pendingBundleConnection
    ? config.storages.find((candidate) => candidate.id === pendingBundleConnection.storageId) ?? null
    : null;
  const projectLinks: ProjectStorageLink[] = detail?.project.local_project_id === activeProjectId
    ? detail.links
    : config.links.filter((link) => link.local_project_id === activeProjectId);
  /* The remembered storage counts only while it is linked to this project, so
     Push, Pull, and the comparison list always use the same destination.
     `loadProjectData` validates the same condition. */
  const rememberedStorageId = activeProjectId ? activeStorageByProject[activeProjectId] : undefined;
  const activeStorageId = activeProjectId
    ? (rememberedStorageId && projectLinks.some((link) => link.storage_id === rememberedStorageId)
      ? rememberedStorageId
      : projectLinks[0]?.storage_id ?? null)
    : null;
  useEffect(() => {
    setActiveProjectSubview((current) => {
      if (current.kind !== "session_detail") return current;
      const leftProjectWorkspace = current.localProjectId !== activeProjectId
        || setupDraftId != null
        || addingCatalogScope
        || catalogIndexOpen
        || catalogScopeId != null
        || activeStorageSettingsId != null
        || pendingPush != null
        || restorePlan != null;
      return leftProjectWorkspace ? { kind: "overview" } : current;
    });
  }, [
    activeProjectId,
    activeStorageId,
    activeStorageSettingsId,
    addingCatalogScope,
    catalogIndexOpen,
    catalogScopeId,
    pendingPush,
    restorePlan,
    setupDraftId,
  ]);
  const clearHostScopedWorkflows = useCallback(() => {
    hostWorkflowEpoch.current += 1;
    restoreRequest.current += 1;
    setPendingBundleConnection(null);
    setPendingPush(null);
    setPushSelected(new Set());
    setPushProjectContentRemovals(new Set());
    setPushWarningAcknowledgements(new Set());
    setProjectContentLoading(false);
    setPushError(null);
    setPushReviewStep("history");
    setStorageNotice(null);
    setEditingBinding(null);
    setRestorePlan(null);
    setRestoreBinding(null);
    setRestoreResult(null);
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setSetupDrafts([]);
    setStorageSetupDrafts([]);
    setCatalogScopes([]);
    setActiveProjectSubview({ kind: "overview" });
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
    configRef.current = EMPTY_CONFIG;
    configSnapshotValidRef.current = false;
    setConfig(EMPTY_CONFIG);
    setError(null);
    setBackendError(null);
    setNotice(null);
    setBusy(false);
  }, []);
  const loadConversationPathAudits = async (
    nextBindings: ProjectBinding[],
    shellRequestId?: number,
    workflowEpoch?: number,
  ) => {
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || (workflowEpoch !== undefined && hostWorkflowEpoch.current !== workflowEpoch)
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
      || (workflowEpoch !== undefined && hostWorkflowEpoch.current !== workflowEpoch)
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
    nextConfig: SyncConfig;
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
    setConversationPathAuditLoading(true);
    setStartupStage("installation");
    const failures: string[] = [];
    let nextProjects: LocalProjectRegistration[] = [];
    let nextConfig: SyncConfig = EMPTY_CONFIG;
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
      setStartupStage("settings");
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
      if (
        configSnapshotValidRef.current
        && nextConfig.revision < configRef.current.revision
      ) {
        nextConfig = configRef.current;
      } else {
        configRef.current = nextConfig;
        configSnapshotValidRef.current = true;
        setConfig(nextConfig);
      }
    } catch (reason) {
      failures.push(`Configuration: ${errorMessage(reason)}`);
      if (configSnapshotValidRef.current) {
        nextConfig = configRef.current;
      } else {
        configRef.current = EMPTY_CONFIG;
        configSnapshotValidRef.current = false;
        setConfig(EMPTY_CONFIG);
      }
    }
    setStartupStage("repair");
    try {
      // Heals interrupted finalizations before any project data is rendered.
      // It owns a stage because rescanning provider history after an
      // interrupted sync can take a while, and it must not read as a freeze.
      await projectSyncApi.recoverStartupState();
    } catch (reason) {
      failures.push(`Recovery: ${errorMessage(reason)}`);
    }
    if (shellRequest.current !== requestId) return unavailableResult();
    setStartupStage("projects");
    // Recovery can change every document below. Start independent reads only
    // after it completes, then adopt their results in two UI phases.
    const projectReads = Promise.allSettled([
      projectSyncApi.listProjects(),
      refreshSetupDrafts(requestId),
      refreshStorageSetupDrafts(requestId),
      refreshCatalogScopes(requestId),
      projectSyncApi.listProjectRepositoryKinds(),
    ] as const);
    const providerReads = Promise.allSettled([
      projectSyncApi.listAgentProviders(),
      projectSyncApi.listProviderProfiles(),
      projectSyncApi.listBindings(),
    ] as const);
    const [projectsResult, , , , repositoryKindsResult] = await projectReads;
    if (shellRequest.current !== requestId) return unavailableResult();
    if (projectsResult.status === "fulfilled") {
      nextProjects = projectsResult.value;
      setRegistrations(nextProjects);
    } else {
      failures.push(`Projects: ${errorMessage(projectsResult.reason)}`);
      setRegistrations([]);
    }
    if (repositoryKindsResult.status === "fulfilled") {
      setRepositoryKinds(repositoryKindsResult.value);
    } else {
      failures.push(`Repository types: ${errorMessage(repositoryKindsResult.reason)}`);
      setRepositoryKinds({});
    }
    setStartupStage("providers");
    const [agentProvidersResult, profilesResult, bindingsResult] = await providerReads;
    if (shellRequest.current !== requestId) return unavailableResult();
    if (agentProvidersResult.status === "fulfilled") {
      setAgentProviders(agentProvidersResult.value);
    } else {
      failures.push(`Agent providers: ${errorMessage(agentProvidersResult.reason)}`);
      setAgentProviders([]);
    }
    if (profilesResult.status === "fulfilled") {
      nextProfiles = profilesResult.value;
      setProfiles(nextProfiles);
    } else {
      failures.push(`Profiles: ${errorMessage(profilesResult.reason)}`);
      setProfiles([]);
    }
    if (bindingsResult.status === "fulfilled") {
      nextBindings = bindingsResult.value;
      setBindings(nextBindings);
    } else {
      failures.push(`Bindings: ${errorMessage(bindingsResult.reason)}`);
      setBindings([]);
    }
    setStartupStage("workspace");
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
    void loadConversationPathAudits(nextBindings, requestId);
    return { nextProjects, nextConfig, nextBindings, nextProfiles, hostIdentityReady: true };
  };

  /* Storage transports fail in AWS SDK prose ("S3 conditional put …: dispatch
     failure: io error"). The storage page already translates that into the
     sentence that names the fix; Push and Pull showed the raw string for the
     same root cause. One resolution point, so both surfaces agree. */
  const storageFailureMessage = (storageId: string, reason: unknown): string => storageAccessErrorMessage(
    configRef.current.storages.find((candidate) => candidate.id === storageId) ?? null,
    reason,
  );

  /* A running Push or Pull writes the bundle and the local journal together;
     killing the process midway is what the review footers warn cannot be
     undone, and the red close button did exactly that without asking.

     The listener is registered only while something is in flight. That keeps
     the ordinary close on its default path — `onCloseRequested` destroys the
     window itself when the handler allows the close, so a listener that is
     always mounted puts every close through this code. */
  useEffect(() => {
    if (!busy) return;
    let disposed = false;
    const pending = getCurrentWindow().onCloseRequested(async (event) => {
      if (!busyRef.current) return;
      event.preventDefault();
      const leave = await confirm(
        "A sync is still writing. Quitting now can leave this project's synced state half-written, and it cannot be undone.\n\nWait for it to finish, or quit anyway?",
        { title: "Sync in progress", kind: "warning", okLabel: "Quit anyway", cancelLabel: "Keep syncing" },
      );
      if (leave) await getCurrentWindow().destroy();
    });
    return () => {
      disposed = true;
      void pending.then((unlisten) => { if (disposed) unlisten(); });
    };
  }, [busy]);

  const loadProjectData = async (
    projectId: string,
    cfg = config,
    preferredStorage?: string | null,
    workflowEpoch?: number,
  ) => {
    const requestedWorkflowEpoch = workflowEpoch ?? hostWorkflowEpoch.current;
    if (hostWorkflowEpoch.current !== requestedWorkflowEpoch) return;
    const requestId = ++projectRequest.current;
    const superseded = () => hostWorkflowEpoch.current !== requestedWorkflowEpoch
      || projectRequest.current !== requestId;
    setLoading(true);
    setError(null);
    try {
      const nextDetail = await projectSyncApi.getProject(projectId);
      if (superseded()) return;
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
          if (superseded()) return;
        } catch (reason) {
          if (superseded()) return;
          setError(errorMessage(reason));
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
      // Project selection does not probe remote readiness. Pull populates
      // `readiness` from the reviewed plan when it needs it.
      if (storageId && nextDetail.binding?.state === "active") {
        setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
      }
      setReadiness(null);

      // Sibling detection (advisory): while this link's bundle has no
      // published content, the storage may hold this repository in another
      // repo. Failures stay silent — the banner is a hint, not evidence.
      let nextSibling: SiblingBundleNotice | null = null;
      if (storageId) {
        try {
          const status = await projectSyncApi.getSiblingBundleStatus(projectId, storageId);
          if (superseded()) return;
          if (status.state !== "none") {
            nextSibling = { projectId, storageId, status };
          }
        } catch {
          if (superseded()) return;
        }
      }
      setSiblingNotice(nextSibling);
    } catch (reason) {
      if (superseded()) return;
      setDetail(null);
      setInventory(null);
      setReadiness(null);
      setSiblingNotice(null);
      setError(errorMessage(reason));
    } finally {
      // A superseded load must not clear the spinner the live one is holding.
      if (!superseded()) setLoading(false);
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
  }, []); // ProjectSync is the project-sync lifetime boundary.

  const refresh = async (workflowEpoch?: number) => {
    if (
      workflowEpoch !== undefined
      && hostWorkflowEpoch.current !== workflowEpoch
    ) return;
    const current = activeProjectId;
    const { nextProjects, nextConfig, hostIdentityReady } = await loadShell();
    if (
      !hostIdentityReady
      || (workflowEpoch !== undefined && hostWorkflowEpoch.current !== workflowEpoch)
    ) return;
    const continuationEpoch = workflowEpoch ?? hostWorkflowEpoch.current;
    const next = current && nextProjects.some((candidate) => candidate.local_project_id === current)
      ? current
      : nextProjects[0]?.local_project_id ?? null;
    setActiveProjectId(next);
    if (next) await loadProjectData(next, nextConfig, undefined, continuationEpoch);
    else {
      setDetail(null);
      setInventory(null);
    }
    if (hostWorkflowEpoch.current !== continuationEpoch) return;
    setHistoryRefreshEpoch((epoch) => epoch + 1);
  };

  // Quietly reconciles state commonly changed outside Mallard (git init, external
  // renames) after the window regains focus. Deliberately narrower than refresh():
  // no audits, profiles, readiness probes, history reset, or loading indicator.
  const reconcileExternalChanges = async () => {
    if (loading || busy || !hostIdentityShellReady) return;
    const shellRequestId = shellRequest.current;
    const workflowEpoch = hostWorkflowEpoch.current;
    const stale = () => (
      shellRequest.current !== shellRequestId
      || hostWorkflowEpoch.current !== workflowEpoch
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
        await refresh(workflowEpoch);
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
      if (blurredAt !== null && Date.now() - blurredAt >= FOCUS_RECONCILE_MIN_AWAY_MS
        && getPreference("refreshOnFocus")) {
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
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      setAppMenuOpen(false);
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

  const selectProject = async (projectId: string, preferredStorage?: string | null) => {
    closeProjectSubview();
    setActiveProjectId(projectId);
    await loadProjectData(projectId, config, preferredStorage);
  };

  const selectStorage = (projectId: string, storageId: string) => {
    if (!config.links.some((link) => (
      link.local_project_id === projectId && link.storage_id === storageId
    ))) return;
    closeProjectSubview();
    setActiveProjectId(projectId);
    setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
  };

  /** Probe a provider path and reuse the profile it already belongs to, or
   *  register one under the probe's suggested name. Refreshes the profile
   *  list; refusals throw into the caller's error envelope. */
  const ensureProfileAtPath = async (
    provider: ProviderKey,
    path: string,
    workflowEpoch: number,
  ): Promise<ProviderProfile | null> => {
    const probe = await projectSyncApi.probeProviderProfile(provider, path.trim());
    if (hostWorkflowEpoch.current !== workflowEpoch) return null;
    const currentProfiles = await projectSyncApi.listProviderProfiles();
    if (hostWorkflowEpoch.current !== workflowEpoch) return null;
    const existing = probe.existing_profile_id
      ? currentProfiles.find((profile) => profile.profile_id === probe.existing_profile_id) ?? null
      : null;
    const profile = existing ?? await projectSyncApi.createProviderProfile(
      provider,
      probe.suggested_name,
      path.trim(),
    );
    if (hostWorkflowEpoch.current !== workflowEpoch) return null;
    const nextProfiles = existing ? currentProfiles : await projectSyncApi.listProviderProfiles();
    if (hostWorkflowEpoch.current !== workflowEpoch) return null;
    setProfiles(nextProfiles);
    return profile;
  };

  const refreshSetupDrafts = async (shellRequestId?: number, workflowEpoch?: number) => {
    const requestedWorkflowEpoch = workflowEpoch ?? hostWorkflowEpoch.current;
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || hostWorkflowEpoch.current !== requestedWorkflowEpoch
    ) return;
    try {
      const listed = await projectSyncApi.listSetupDrafts();
      if (
        (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
        || hostWorkflowEpoch.current !== requestedWorkflowEpoch
      ) return;
      setSetupDrafts(listed.drafts);
    } catch {
      // Drafts are a convenience surface; the projects list stays usable.
    }
  };

  const refreshStorageSetupDrafts = async (shellRequestId?: number, workflowEpoch?: number) => {
    const requestedWorkflowEpoch = workflowEpoch ?? hostWorkflowEpoch.current;
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || hostWorkflowEpoch.current !== requestedWorkflowEpoch
    ) return;
    const requestId = ++storageSetupDraftRequest.current;
    try {
      const listed = await projectSyncApi.listStorageSetupDrafts();
      if (
        requestId !== storageSetupDraftRequest.current
        || (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
        || hostWorkflowEpoch.current !== requestedWorkflowEpoch
      ) return;
      setStorageSetupDrafts(listed.drafts);
    } catch {
      // Drafts are a convenience surface; the storages index stays usable.
    }
  };

  useEffect(() => {
    const unlisten = listen<StorageSetupDraftsChangedEvent>(
      "project-sync-storage-setup-drafts-changed",
      () => {
        // The native event is intentionally value-free. Reload through the
        // command that redacts `secret_access_key` and exposes only
        // `secret_present` to the webview.
        void refreshStorageSetupDrafts();
      },
    );
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const closeWorkspaceSettings = () => {
    closeProjectSubview();
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    // The Catalog page counts as a workspace overlay: navigating to a
    // project or draft leaves it the same way it leaves storage settings.
    setCatalogScopeId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setStorageIndexOpen(false);
    setSettingsOpen(false);
    setStorageEditorRequest((current) => ({
      mode: "close",
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  /** One route into a storage page, shared by the sidebar, the Storages
      index, and the banners that deep-link a specific storage's settings. */
  const openStorageWorkspace = (storageId: string, settingsExpanded: boolean) => {
    closeProjectSubview();
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setStorageIndexOpen(false);
    setSettingsOpen(false);
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    setStorageEditorRequest((current) => ({
      mode: "open",
      storageId,
      settingsExpanded,
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  const openStorageIndex = () => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    setStorageIndexOpen(true);
  };

  const openProjectsIndex = () => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    projectRequest.current += 1;
    setLoading(false);
    setActiveProjectId(null);
    setDetail(null);
    setInventory(null);
    setReadiness(null);
    setSiblingNotice(null);
  };

  const openCatalogIndex = () => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    setCatalogIndexOpen(true);
  };

  const openSettings = () => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    setSettingsOpen(true);
  };

  const beginAddProject = async () => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const picked = await open({ directory: true, multiple: false });
    if (
      hostWorkflowEpoch.current !== workflowEpoch
      || typeof picked !== "string"
      || !picked
    ) return;
    setBusy(true);
    setError(null);
    try {
      const created = await projectSyncApi.createSetupDraft(picked);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      closeWorkspaceSettings();
      setSetupDraftId(created.draft.draft_id);
      if (created.resumed) {
        notify(`Resumed setup draft for ${created.draft.display_name}.`);
      }
      await refreshSetupDrafts(undefined, workflowEpoch);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const refreshCatalogScopes = async (shellRequestId?: number, workflowEpoch?: number) => {
    const requestedWorkflowEpoch = workflowEpoch ?? hostWorkflowEpoch.current;
    if (
      (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
      || hostWorkflowEpoch.current !== requestedWorkflowEpoch
    ) return;
    try {
      const listed = await storageCatalogApi.listScopes();
      if (
        (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
        || hostWorkflowEpoch.current !== requestedWorkflowEpoch
      ) return;
      setCatalogScopes(listed);
    } catch {
      // The menu is a convenience surface; the rest of the shell stays usable.
    }
  };

  /** Opens the Add-row workspace: Kind, then provider home, and nothing else. */
  const beginAddCatalogScope = () => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    setCatalogIndexOpen(false);
    setAddingCatalogScope(true);
  };

  const openCatalogScope = (scopeId: string) => {
    setError(null);
    closeWorkspaceSettings();
    setSetupDraftId(null);
    setCatalogIndexOpen(false);
    setCatalogScopeId(scopeId);
  };

  const completeAddCatalogScope = async (scope: CatalogScopeView, workflowEpoch: number) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(scope.scope.scope_id);
    notify(`${catalogScopeLabel(scope.scope, scope.profile)} added to the Catalog.`);
    await refreshCatalogScopes(undefined, workflowEpoch);
  };

  const removeCatalogScope = async (scopeId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const scope = catalogScopes.find((candidate) => candidate.scope.scope_id === scopeId);
    if (!scope) {
      await refreshCatalogScopes(undefined, workflowEpoch);
      return;
    }
    const label = catalogScopeLabel(scope.scope, scope.profile);
    const approved = await confirm(
      `Remove the “${label}” row from the Catalog?\n\nOnly the menu row is removed. No provider or storage content is touched.`,
      { title: "Remove Catalog row" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    setCatalogRemovalErrorTarget(null);
    try {
      await storageCatalogApi.deleteScope(scopeId, scope.scope.revision);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      if (catalogScopeId === scopeId) {
        setCatalogScopeId(null);
        setCatalogIndexOpen(true);
      }
      await refreshCatalogScopes(undefined, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify(`${label} removed — no provider or storage content touched.`);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) {
        const message = catalogScopeRemovalMessage(label, reason);
        setError(message);
        setCatalogRemovalErrorTarget(catalogScopeRemovalRequiresCleanup(reason)
          ? { label, message, scopeId }
          : null);
      }
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
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
    const workflowEpoch = hostWorkflowEpoch.current;
    const summary = setupDrafts.find((candidate) => candidate.draft_id === draftId);
    const approved = await confirm(
      `Discard the setup draft for “${summary?.display_name ?? "this project"}”?\n\nOnly the draft is removed. No project files are touched.`,
      { title: "Discard draft" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    try {
      await projectSyncApi.discardSetupDraft(draftId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      if (setupDraftId === draftId) setSetupDraftId(null);
      await refreshSetupDrafts(undefined, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify("Draft discarded — no files touched.");
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  /** Discard a storage setup draft from the rail. If that draft's form is
      open (the workspace reports its storage id as the active editor), the
      editor closes too — its state describes a draft that no longer exists. */
  const discardStorageSetupDraft = async (draftId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const draft = storageSetupDrafts.find((candidate) => candidate.draft_id === draftId);
    const discardImpact = draft?.storage.kind === "s3"
      ? "This removes only Mallard’s local setup draft. It does not delete any Cloudflare bucket or access key that may have been created during preparation."
      : draft?.storage.kind === "local"
        ? "This removes only Mallard’s local setup draft. The selected folder and its contents are not changed."
        : "This removes only Mallard’s local setup draft; external storage resources are not changed.";
    const approved = await confirm(
      `Discard the storage draft for “${draft?.storage.name || "(unnamed)"}”?\n\n${discardImpact}`,
      { title: "Discard storage draft" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    try {
      await projectSyncApi.discardStorageSetupDraft(draftId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      if (draft && activeStorageSettingsId === draft.storage.id) {
        setStorageEditorRequest((current) => ({
          mode: "close",
          requestId: (current?.requestId ?? 0) + 1,
        }));
      }
      await refreshStorageSetupDrafts(undefined, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify("Storage draft discarded.");
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const completeSetup = async (
    detail: ProjectDetail,
    workflowEpoch: number,
  ) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setSetupDraftId(null);
    const projectId = detail.project.local_project_id;
    const storageId = detail.links[0]?.storage_id ?? null;
    notify(`${projectLabel(detail.project)} is set up${storageId ? "" : " — link storage when ready"}.`);
    const { nextConfig, hostIdentityReady } = await loadShell();
    if (!hostIdentityReady || hostWorkflowEpoch.current !== workflowEpoch) return;
    await refreshSetupDrafts(undefined, workflowEpoch);
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setActiveProjectId(projectId);
    if (detail.binding) setBindings((current) => upsertBinding(current, detail.binding as ProjectBinding));
    await loadProjectData(projectId, nextConfig, storageId, workflowEpoch);
  };

  const saveStorageLink = async (
    projectId: string,
    storageId: string,
    workflowEpoch = hostWorkflowEpoch.current,
  ) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    await projectSyncApi.saveLink({
      local_project_id: projectId,
      storage_id: storageId,
      pinned: true,
    });
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setStorageNotice((current) => (
      current?.projectId === projectId && current.storageId === storageId ? null : current
    ));
    const { nextConfig, hostIdentityReady } = await loadShell();
    if (!hostIdentityReady || hostWorkflowEpoch.current !== workflowEpoch) return;
    setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
    setActiveProjectId(projectId);
    await loadProjectData(projectId, nextConfig, storageId, workflowEpoch);
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    notify("Storage linked.");
  };

  const linkStorage = async (projectId: string, storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      const project = registrations.find((candidate) => candidate.local_project_id === projectId);
      if (!project) throw new Error("The selected local project no longer exists.");
      const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      bundles.sort((left, right) => {
        const leftMatches = !!project.repository_fingerprint
          && left.repository_fingerprint === project.repository_fingerprint;
        const rightMatches = !!project.repository_fingerprint
          && right.repository_fingerprint === project.repository_fingerprint;
        if (leftMatches !== rightMatches) return leftMatches ? -1 : 1;
        return (right.updated_at ?? 0) - (left.updated_at ?? 0);
      });
      if (bundles.some((bundle) => bundle.bundle_id === project.bundle_id) || bundles.length === 0) {
        await saveStorageLink(projectId, storageId, workflowEpoch);
        return;
      }
      setPendingBundleConnection({ projectId, storageId, matches: bundles, reason: "link" });
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const repairConversationPaths = async (projectId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const audit = conversationPathAudits[projectId];
    if (!audit?.can_repair) return;
    const registration = registrations.find((candidate) => candidate.local_project_id === projectId);
    const projectName = registration ? projectLabel(registration) : "this project";
    const approved = await confirm(
      `Repair ${audit.issues.length} Codex conversation path${audit.issues.length === 1 ? "" : "s"} for “${projectName}”?\n\nOnly conversations explicitly assigned to this project by Codex Desktop will be changed. Mallard will back up every affected rollout first, then update structural cwd fields without changing messages or history.\n\nClose the affected Codex tasks before continuing so they are not written at the same time.`,
      { title: "Repair conversation paths" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;

    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.repairCodexConversationPaths(projectId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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
        workflowEpoch,
      );
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify(
        `Repaired ${result.repaired_thread_ids.length} conversation path${result.repaired_thread_ids.length === 1 ? "" : "s"} — originals backed up.`,
      );
    } catch (reason) {
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      setError(errorMessage(reason));
      try {
        const refreshed = await projectSyncApi.auditCodexConversationPaths(projectId);
        if (hostWorkflowEpoch.current !== workflowEpoch) return;
        setConversationPathAudits((current) => ({ ...current, [projectId]: refreshed }));
      } catch {
        // Keep the prior gate visible when the follow-up audit also fails.
      }
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const openPushChooser = async (
    projectId: string,
    storageId: string,
    workflowEpoch = hostWorkflowEpoch.current,
  ) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    if (restorePlan) closePullReview();
    setPushReviewStep("history");
    setPushProjectContentRemovals(new Set());
    setPushWarningAcknowledgements(new Set());
    setProjectContentLoading(false);
    setStorageNotice(null);
    setBusy(true);
    setError(null);
    setPushError(null);
    try {
      // One backend call runs a single discovery pass and a single remote
      // snapshot for every review section.
      const context = await projectSyncApi.preparePushReview(projectId, storageId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      const nextDetail = context.project;
      const nextInventory = context.inventory;
      const storage = config.storages.find((candidate) => candidate.id === storageId);
      if (!storage) throw new Error("The selected storage no longer exists.");
      const link = nextDetail.links.find((candidate) => candidate.storage_id === storageId);
      if (!link) throw new Error("This project is no longer linked to the selected storage.");

      // A destination the push cannot get past never opens the review — the
      // gate lands as a banner on the storage card instead.
      const destination = context.destination_status;
      if (
        destination.state === "requires_pull"
        || destination.state === "remote_history_missing"
        || destination.state === "remote_history_replaced"
      ) {
        const changedConversations = context.thread_comparison
          ? context.thread_comparison.counts.storage + context.thread_comparison.counts.diverged
          : 0;
        setStorageNotice(destination.state === "requires_pull"
          ? {
            projectId,
            storageId,
            kind: "requires_pull",
            copy: pullRequiredCopy(
              destination.reason,
              storage.name,
              destination.remote_revision,
              destination.base_revision,
              changedConversations,
            ),
          }
          : {
            projectId,
            storageId,
            kind: destination.state === "remote_history_missing" ? "remote_missing" : "remote_replaced",
            baseRevision: destination.base_revision ?? null,
          });
        setActiveProjectId(projectId);
        setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
        return;
      }

      const savedRecipe = link.recipe ?? null;
      const nextResources = inventoryResources(nextInventory);
      // Degraded sections such as offline storage or capture failures open
      // without a comparison.
      const nextThreadComparison = context.thread_comparison ?? undefined;
      const isGitRepository = repositoryKinds[projectId] ?? false;
      const lockedProjectContentInventory =
        context.project_files && context.project_files.entries.length > 0
          ? context.project_files
          : null;
      const projectDefaults = recommendedPushSelection(
        recipeSelection(savedRecipe ?? nextInventory.recipe),
        nextResources,
        nextThreadComparison,
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
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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
        projectContentInventory: lockedProjectContentInventory,
        projectContentScanned: lockedProjectContentInventory !== null,
        showProjectFiles: !isGitRepository || lockedProjectContentInventory !== null,
        conversationPushPlanId: context.conversation_push_plan_id ?? null,
        conversationActionIds: context.conversation_action_ids ?? {},
      });
      setPushSelected(new Set(projectDefaults));
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) {
        setStorageNotice({
          projectId,
          storageId,
          kind: "action_failed",
          action: "push",
          message: storageFailureMessage(storageId, reason),
        });
      }
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const refreshRepositoryKinds = async (workflowEpoch?: number) => {
    if (
      workflowEpoch !== undefined
      && hostWorkflowEpoch.current !== workflowEpoch
    ) return;
    try {
      const nextRepositoryKinds = await projectSyncApi.listProjectRepositoryKinds();
      if (
        workflowEpoch !== undefined
        && hostWorkflowEpoch.current !== workflowEpoch
      ) return;
      setRepositoryKinds(nextRepositoryKinds);
    } catch (reason) {
      // Keep the previous classification. Emptying it on a transient failure
      // silently reclassified every Git project as non-Git, which is what the
      // Push review reads to decide whether project files may travel.
      if (
        workflowEpoch === undefined
        || hostWorkflowEpoch.current === workflowEpoch
      ) setError(errorMessage(reason));
    }
  };

  const scanPendingProjectFiles = async () => {
    const pending = pendingPush;
    if (!pending || projectContentLoading) return;
    const workflowEpoch = hostWorkflowEpoch.current;
    setProjectContentLoading(true);
    setPushError(null);
    try {
      const inspected = await projectSyncApi.inspectProjectFiles(pending.projectId, pending.storageId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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
      if (hostWorkflowEpoch.current === workflowEpoch) setPushError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setProjectContentLoading(false);
    }
  };

  // Narrow post-push reconcile, modeled on reconcileExternalChanges: the push
  // already succeeded and reported, so the UI unblocks immediately while this
  // refreshes config/link/project state in the background. Inventory refresh
  // is deliberately deferred to the next dialog open or full refresh().
  const refreshAfterPush = async (
    projectId: string,
    workflowEpoch: number,
  ) => {
    const stale = () => hostWorkflowEpoch.current !== workflowEpoch;
    try {
      const [nextConfig, nextDetail] = await Promise.all([
        projectSyncApi.getConfig(),
        projectSyncApi.getProject(projectId),
      ]);
      if (stale()) return;
      if (nextConfig.schema === PROJECT_SYNC_SCHEMA) {
        if (
          !configSnapshotValidRef.current
          || nextConfig.revision >= configRef.current.revision
        ) {
          configRef.current = nextConfig;
          configSnapshotValidRef.current = true;
          setConfig(nextConfig);
        }
      }
      if (!nextDetail) return;
      setDetail(nextDetail);
      setRegistrations((current) => upsertProject(current, nextDetail.project));
      if (nextDetail.binding) {
        setBindings((current) => upsertBinding(current, nextDetail.binding as ProjectBinding));
      }
    } catch {
      // Background reconcile stays silent; the push result was already shown.
    }
  };

  const publishProject = async (
    projectId: string,
    storageId: string,
    recipe: BundleRecipe,
    projectContentReviewToken: string | null = null,
    projectContentRemovalIds: string[] = [],
    acknowledgedWarningDigests: string[] = [],
    conversationPushPlanId: string | null = null,
    approvedConversationActionIds: string[] = [],
  ) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    if (getPreference("openLogsOnSync")) openActivity();
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
        conversationPushPlanId,
        approvedConversationActionIds,
      );
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      // The push is finished, so the review closes and the result lands on the
      // storage card — one banner that folds itself when nothing needs a
      // decision, instead of a toast that takes the detail with it.
      setStorageNotice({
        projectId,
        storageId,
        kind: "outcome",
        outcome: buildPushOutcome({
          projectId,
          storageId,
          selectedResourceIds: safePushSelection,
          threadComparison: pendingPush?.threadComparison,
          result,
        }),
      });
      setHistoryRefreshEpoch((epoch) => epoch + 1);
      setPendingPush(null);
      setPushProjectContentRemovals(new Set());
      setPushWarningAcknowledgements(new Set());
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
      void refreshAfterPush(projectId, workflowEpoch);
    } catch (reason) {
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      const message = storageFailureMessage(storageId, reason);
      if (pendingPush) {
        try {
          const destinationStatus = await projectSyncApi.getPushDestinationStatus(
            projectId,
            storageId,
          );
          if (hostWorkflowEpoch.current !== workflowEpoch) return;
          if (
            destinationStatus.state === "remote_history_missing"
            || destinationStatus.state === "remote_history_replaced"
            || destinationStatus.state === "requires_pull"
          ) {
            // The destination moved out from under the push mid-flight; the
            // review can no longer finish its job, so it closes and the gate
            // explains itself on the storage card.
            setPendingPush(null);
            setPushError(null);
            setPushProjectContentRemovals(new Set());
            setPushWarningAcknowledgements(new Set());
            const changedConversations = pendingPush.threadComparison
              ? pendingPush.threadComparison.counts.storage + pendingPush.threadComparison.counts.diverged
              : 0;
            setStorageNotice(destinationStatus.state === "requires_pull"
              ? {
                projectId,
                storageId,
                kind: "requires_pull",
                copy: pullRequiredCopy(
                  destinationStatus.reason,
                  pendingPush.storage.name,
                  destinationStatus.remote_revision,
                  destinationStatus.base_revision,
                  changedConversations,
                ),
              }
              : {
                projectId,
                storageId,
                kind: destinationStatus.state === "remote_history_missing" ? "remote_missing" : "remote_replaced",
                baseRevision: destinationStatus.base_revision ?? null,
              });
            return;
          }
        } catch {
          // Preserve the original Push error when the lightweight recheck also fails.
        }
        setPushError(message);
      } else {
        setError(message);
      }
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
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
      pendingPush.conversationPushPlanId,
      [...safeSelection]
        .map((resourceId) => pendingPush.conversationActionIds[resourceId])
        .filter((actionId): actionId is string => Boolean(actionId)),
    );
  };

  const saveRemap = async (submission: ProjectBindingSubmission) => {
    if (!activeProjectId) return;
    const workflowEpoch = hostWorkflowEpoch.current;
    const projectId = activeProjectId;
    setBusy(true);
    setError(null);
    try {
      // Typed custom paths become registered profiles first; the binding
      // command itself only accepts profile ids.
      const profileIds: Record<ProviderKey, string> = {};
      for (const [provider, selection] of Object.entries(submission.profiles) as Array<[ProviderKey, BindingProfileSelection]>) {
        if (selection.kind === "existing") {
          profileIds[provider] = selection.profile_id;
          continue;
        }
        const profile = await ensureProfileAtPath(provider, selection.path, workflowEpoch);
        if (!profile) return;
        profileIds[provider] = profile.profile_id;
      }
      const saved = await projectSyncApi.saveBinding({
        local_project_id: projectId,
        project_root: submission.project_root,
        profile_ids: profileIds,
        expected_revision: submission.expected_revision ?? binding?.revision ?? null,
      });
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      setEditingBinding(null);
      const nextBindings = upsertBinding(bindings, saved);
      setBindings(nextBindings);
      setDetail((current) => current ? { ...current, binding: saved } : current);
      await refreshRepositoryKinds(workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify("Machine binding updated — cloud identity unchanged.");
      await loadConversationPathAudits(nextBindings, undefined, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      await loadProjectData(projectId, config, activeStorageId, workflowEpoch);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  /* Return rename errors to the inline form so unrelated shell failures cannot
     appear as rename failures. A superseded workflow reports no outcome and
     preserves the form. */
  const renameProject = async (projectId: string, alias: string | null): Promise<RenameResult> => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    try {
      const registration = registrations.find((candidate) => candidate.local_project_id === projectId);
      if (!registration) throw new Error("Project not found.");
      const updated = await projectSyncApi.renameProject(projectId, alias, registration.revision);
      if (hostWorkflowEpoch.current !== workflowEpoch) return { ok: false, message: null };
      setRegistrations((current) => current.map((candidate) => (
        candidate.local_project_id === projectId ? updated : candidate
      )));
      if (activeProjectId === projectId) {
        setDetail((current) => current ? { ...current, project: updated } : current);
      }
      notify(alias
        ? `Shown as “${alias}” on this computer.`
        : `Name reset to “${updated.display_name}”.`);
      return { ok: true };
    } catch (reason) {
      if (hostWorkflowEpoch.current !== workflowEpoch) return { ok: false, message: null };
      return { ok: false, message: errorMessage(reason) };
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
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
    setFetchProgress(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(true);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
    setRestorePlan(null);
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
      // Nothing to review means the storage only moved its revision: one
      // empty apply acknowledges it right here, no review to click through.
      // A replaced history is never adopted silently — that plan still opens.
      if (
        !hasPendingRestoreMutations(review.restorePlan)
        && review.restorePlan.remote_continuity?.state !== "replaced"
      ) {
        const finalization = await projectSyncApi.applyRestore(review.restorePlan.plan_id, []);
        if (restoreRequest.current !== requestId) return null;
        if (
          !finalization.success
          || finalization.status !== "complete"
          || finalization.reviewed_base_advanced === false
        ) {
          throw new Error("Nothing to pull, but the reviewed baseline could not be advanced.");
        }
        setReviewSupportLoading(false);
        setRestoreBinding(null);
        notify(`${displayName} is up to date (revision ${review.restorePlan.revision}).`);
        return null;
      }
      setRestorePlan(review.restorePlan);
      notify(`Pull review ready for ${displayName} — nothing applied yet.`);

      // Supporting context fills in asynchronously after the workspace is usable.
      // Its lifecycle is request-scoped so closing or replanning cannot apply
      // stale results to another review.
      void review.support
        .then((support) => {
          if (restoreRequest.current !== requestId) return;
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
      const message = storageFailureMessage(storageId, reason);
      // A superseded plan reports nothing: the review it belonged to is gone,
      // and its caller would otherwise paint a failure over the live one.
      if (restoreRequest.current !== requestId) return null;
      setReviewSupportLoading(false);
      setRestoreError(message);
      return message;
    } finally {
      if (restoreRequest.current === requestId) setBusy(false);
    }
  };

  /**
   * The one follow-up a finished sync can still owe. Restart and retry talk to
   * the backend directly because the review is closed by the time this banner
   * exists. Anything unresolved reopens Pull and replans from scratch.
   */
  const runOutcomeAction = async (outcome: SyncOutcome) => {
    const action = outcome.action;
    if (!action) return;
    if (action.kind === "reset_metadata") {
      await resetProjectSyncMetadata(outcome.projectId);
      return;
    }
    if (action.kind === "open_pull") {
      setStorageNotice(null);
      void beginProjectRestore(outcome.projectId, outcome.storageId);
      return;
    }
    if (action.kind === "recheck") {
      // Nothing to re-apply: ask the backend again and rebuild the same banner
      // from the same result, so a fixed profile simply clears it.
      const inputs = pullOutcomeInputs.current;
      if (!inputs) return;
      setBusy(true);
      try {
        const readiness = await projectSyncApi.getRestoreReadiness(action.planId);
        setReadiness(readiness);
        const next = buildPullOutcome({ ...inputs, readiness });
        if (next.tone === "success" && !next.action) {
          setStorageNotice(null);
          notify("Setup resolved — project ready.");
        } else {
          setStorageNotice({
            projectId: outcome.projectId,
            storageId: outcome.storageId,
            kind: "outcome",
            outcome: next,
          });
        }
      } catch (reason) {
        setStorageNotice({
          projectId: outcome.projectId,
          storageId: outcome.storageId,
          kind: "action_failed",
          action: "pull",
          message: errorMessage(reason),
        });
      } finally {
        setBusy(false);
      }
      return;
    }
    // The backend refuses to restart unless the caller vouches that active
    // chats are complete — so actually ask, instead of vouching unasked.
    const chatsComplete = await confirm(
      `${action.label} now?\n\nAny chats it is still running are interrupted mid-turn. Continue only once active chats are complete.`,
      { title: action.label },
    );
    if (!chatsComplete) return;
    setBusy(true);
    try {
      const result = await projectSyncApi.restartCodexAppAfterPull(
        action.operationId,
        action.appKind,
        chatsComplete,
      );
      setHistoryRefreshEpoch((epoch) => epoch + 1);
      if (result.success) {
        setStorageNotice(null);
        notify(result.message);
      } else {
        setStorageNotice({
          projectId: outcome.projectId,
          storageId: outcome.storageId,
          kind: "action_failed",
          action: "pull",
          message: result.message,
        });
      }
    } catch (reason) {
      setStorageNotice({
        projectId: outcome.projectId,
        storageId: outcome.storageId,
        kind: "action_failed",
        action: "pull",
        message: errorMessage(reason),
      });
    } finally {
      setBusy(false);
    }
  };

  const beginProjectRestore = async (projectId: string, storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setPendingPush(null);
    setPushError(null);
    setStorageNotice(null);
    setError(null);
    try {
      const nextDetail = await projectSyncApi.getProject(projectId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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
          // The binding is missing or unconfirmed here, so the checkout and
          // profiles are still this machine's to choose.
          locked: false,
          expected_revision: nextDetail.binding?.revision ?? null,
        });
        notify(nextDetail.binding
          ? "Confirm project folder and profile before pulling."
          : "Choose project folder and profile before pulling.",
          "attention");
        return;
      }
      const failure = await planRestore(
        storageId,
        nextDetail.project.bundle_id,
        nextDetail.binding,
        projectLabel(nextDetail.project),
      );
      if (!failure) return;
      if (failure.includes("does not exist")) {
        const bundles = await projectSyncApi.listRemoteBundleSnapshots(storageId);
        if (hostWorkflowEpoch.current !== workflowEpoch) return;
        if (bundles.length > 0) {
          bundles.sort((left, right) => {
            const leftMatches = !!nextDetail.project.repository_fingerprint
              && left.repository_fingerprint === nextDetail.project.repository_fingerprint;
            const rightMatches = !!nextDetail.project.repository_fingerprint
              && right.repository_fingerprint === nextDetail.project.repository_fingerprint;
            if (leftMatches !== rightMatches) return leftMatches ? -1 : 1;
            return (right.updated_at ?? 0) - (left.updated_at ?? 0);
          });
          // The chooser is the feedback: it names the repos this storage does
          // hold and lets one be adopted.
          setPendingBundleConnection({ projectId, storageId, matches: bundles, reason: "missing" });
          return;
        }
      }
      // Planning failed and there was nothing to offer instead. Say so: the
      // review never opens, so this notice is the only thing the user sees.
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      setStorageNotice({
        projectId,
        storageId,
        kind: "action_failed",
        action: "pull",
        message: failure,
      });
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) {
        setStorageNotice({
          projectId,
          storageId,
          kind: "action_failed",
          action: "pull",
          message: errorMessage(reason),
        });
      }
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
    const workflowEpoch = hostWorkflowEpoch.current;
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
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      setPendingBundleConnection(null);
      const { nextConfig, hostIdentityReady } = await loadShell();
      if (!hostIdentityReady || hostWorkflowEpoch.current !== workflowEpoch) return;
      setActiveProjectId(projectId);
      setActiveStorageByProject((current) => ({
        ...current,
        [projectId]: storageId,
      }));
      await loadProjectData(projectId, nextConfig, storageId, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      if (!connected.binding) {
        notify("Repo connected — set folder and profile before Pull.");
        return;
      }
      await planRestore(
        storageId,
        bundleId,
        connected.binding,
        projectLabel(connected.project),
      );
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const keepPendingBundle = async () => {
    if (!pendingBundleConnection) return;
    const workflowEpoch = hostWorkflowEpoch.current;
    const pending = pendingBundleConnection;
    setBusy(true);
    setError(null);
    try {
      if (pending.reason === "link") {
        await saveStorageLink(pending.projectId, pending.storageId, workflowEpoch);
        if (hostWorkflowEpoch.current !== workflowEpoch) return;
      } else if (pending.reason === "sibling") {
        // A dismissed sibling offer changes nothing: the link keeps its own
        // repo, and the banner returns on the next refresh while the split
        // still exists.
        setPendingBundleConnection(null);
        return;
      } else {
        setPendingBundleConnection(null);
        notify("Kept local-only identity — Push publishes it separately.");
        await openPushChooser(pending.projectId, pending.storageId, workflowEpoch);
        return;
      }
      setPendingBundleConnection(null);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const removeProject = async (projectId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const summary = projects.find((candidate) => candidate.local_project_id === projectId);
    const approved = await confirm(
      `Remove “${summary ? projectLabel(summary) : "this project"}” from Mallard?\n\nThe checkout and provider files stay on disk. Only this app's project registration, links, and active binding are removed.`,
      { title: "Remove project" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    try {
      await projectSyncApi.removeProject(projectId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      await refresh(workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify(`${summary ? projectLabel(summary) : "Project"} removed — files kept on disk.`);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const closePullReview = () => {
    restoreRequest.current += 1;
    setRestorePlan(null);
    setRestoreBinding(null);
    setRestoreResult(null);
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
    setCompletedPullActionIds(new Set());
    setCompletedPullResourceIds(new Set());
    setFailedPullResourceIds(new Set());
  };

  async function resetProjectSyncMetadata(projectId: string): Promise<void> {
    const workflowEpoch = hostWorkflowEpoch.current;
    const summary = projects.find((candidate) => candidate.local_project_id === projectId);
    if (!summary) {
      setError("The selected local project no longer exists.");
      return;
    }
    const approved = await confirm(
      `Reset local sync state for “${projectLabel(summary)}”?\n\nMallard will forget this project's reviewed sync bases, conversation mappings, recovery records, Pull results, and saved Push/Pull plans. Project files, provider sessions, project setup, recipes, storage configuration, and cloud data will not be deleted.`,
      { title: "Reset local sync state" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;

    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.resetProjectSyncMetadata(
        projectId,
        summary.revision,
      );
      if (hostWorkflowEpoch.current !== workflowEpoch) return;

      closePullReview();
      pullOutcomeInputs.current = null;
      setPendingPush(null);
      setPushError(null);
      setStorageNotice(null);
      setActiveProjectId(projectId);
      setActiveProjectSubview({ kind: "overview" });
      await refresh(workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;

      const removed = result.scope_bases_removed
        + result.conversation_checkpoints_removed
        + result.conversation_bindings_removed
        + result.materializations_removed
        + result.journal_operations_removed
        + result.restore_plans_removed
        + result.push_plans_removed;
      if (result.unreadable_plan_files_retained > 0) {
        notify(
          `Local sync state reset (${removed} records removed). ${result.unreadable_plan_files_retained} unreadable saved plan file${result.unreadable_plan_files_retained === 1 ? " was" : "s were"} retained for inspection.`,
          "attention",
        );
      } else {
        notify(`Local sync state reset — ${removed} derived record${removed === 1 ? "" : "s"} removed; files and cloud data kept.`);
      }
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  }

  // App commands in the path-bar overflow: Settings and a full reload.
  const appActions: PathBarMenuGroup[] = [{
    label: "Mallard",
    items: [
      {
        id: "settings",
        label: "Settings",
        icon: "settings" as const,
        onSelect: openSettings,
      },
      {
        id: "reload",
        label: "Reload app data",
        detail: "Every project and storage, from disk",
        // Not `refresh`: the bar's refresh button is one project's activity,
        // and two identical glyphs in one row is the confusion this redesign
        // set out to remove. This one reloads what the machine knows.
        icon: "computer" as const,
        disabled: loading || busy,
        onSelect: () => { void refresh(); },
      },
    ],
  }];

  // Window-level controls: the theme toggle and the app menu. They belong to
  // the window, not to any screen, so the titlebar carries them on the screens
  // that have one; path-bar screens reach the same scope through the
  // overflow's Mallard group, and theme lives on the Settings page there.
  const windowActions = (
    <div className="sync-titlebar-actions">
      <button
        type="button"
        className="sync-theme-button"
        onClick={() => onThemeChange(theme === "dark" ? "light" : "dark")}
        title={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
        aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
      >
        <Icon name={theme === "dark" ? "moon" : "sun"} size={18} />
      </button>
      <div ref={appMenuRef} className={`storage-link-overflow${appMenuOpen ? " open" : ""}`}>
        <button
          type="button"
          className="storage-link-more"
          aria-haspopup="menu"
          aria-expanded={appMenuOpen}
          aria-controls="sync-app-actions-menu"
          title="App actions"
          aria-label="App actions"
          onClick={() => setAppMenuOpen((current) => !current)}
        >
          <Icon name="more" size={16} />
        </button>
        <div
          id="sync-app-actions-menu"
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
  );

  // One way into a project, shared by the sidebar and the path bar's repo crumb.
  const openProject = (projectId: string) => {
    setSetupDraftId(null);
    closeWorkspaceSettings();
    if (restorePlan) closePullReview();
    void selectProject(projectId);
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
      locked: projectBinding?.state === "active",
      expected_revision: projectBinding?.revision ?? null,
    });
  };

  const unlinkStorage = async (
    projectId: string,
    storageId: string,
    reason: "standard" | "remote_history_missing" = "standard",
  ) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const project = projects.find((candidate) => candidate.local_project_id === projectId);
    const storage = config.storages.find((candidate) => candidate.id === storageId);
    if (!project || !storage) return;

    const projectName = projectLabel(project);
    const storageName = storage.name || "Storage";
    const approved = await confirm(
      reason === "remote_history_missing"
        ? `Unlink “${storageName}” from “${projectName}”?\n\nMallard will clear this project’s reviewed sync base. No project files or cloud objects will be deleted. After unlinking, link this storage again and review a new Push.`
        : `Unlink “${storageName}” from “${projectName}”?\n\nPush and Pull will stop for this pairing. Files and repositories already stored there will not be deleted, and you can link the storage again later.`,
      { title: "Unlink storage" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;

    setBusy(true);
    setError(null);
    try {
      const removed = await projectSyncApi.removeLink(projectId, storageId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      setStorageNotice((current) => (
        current?.projectId === projectId && current.storageId === storageId ? null : current
      ));

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
      if (!hostIdentityReady || hostWorkflowEpoch.current !== workflowEpoch) return;
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
      await loadProjectData(projectId, nextConfig, nextStorageId, workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      notify(removed
        ? reason === "remote_history_missing"
          ? `${storageName} unlinked — files kept; link again for a fresh history.`
          : `${storageName} unlinked from ${projectName} — files kept.`
        : `${storageName} was already unlinked.`);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const applyReview = async (selection: PullReviewSelection) => {
    if (!restorePlan || !restoreBinding) return;
    const requestId = restoreRequest.current;
    const workflowEpoch = hostWorkflowEpoch.current;
    const currentRestorePlan = restorePlan;
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
    if (getPreference("openLogsOnSync")) openActivity();
    setBusy(true);
    setRestoreError(null);
    try {
      const result = await applyPullReview(
        projectSyncApi,
        currentRestorePlan,
        selection,
        (phase) => {
          if (restoreRequest.current === requestId) setPullApplyPhase(phase);
        },
      );
      if (restoreRequest.current !== requestId) return;
      if (result.restoreResult) {
        setRestoreResult(result.restoreResult);
      }
      if (result.restoreResult?.success) {
        setHistoryRefreshEpoch((epoch) => epoch + 1);
      }
      if (result.readiness) setReadiness(result.readiness);
      if (result.error) setRestoreError(result.error);

      const resourceByAction = new Map<string, string>();
      for (const action of currentRestorePlan.actions) {
        resourceByAction.set(action.action_id, action.resource_id);
      }
      for (const action of currentRestorePlan.conversation_actions ?? []) {
        resourceByAction.set(
          action.action_id,
          action.source_resource_id
            ?? `${action.provider}:conversation:${action.thread_id ?? action.cloud_thread_id}`,
        );
      }
      const appliedActionIds = new Set(result.restoreResult?.applied_action_ids ?? []);
      const failedActionIds = new Set(
        (result.restoreResult?.failed_actions ?? []).map((failure) => failure.action_id),
      );
      const interruptedActionIds = result.failedPhase === "restoring"
        ? selection.restoreActionIds
        : [];
      for (const actionId of interruptedActionIds) {
        if (!appliedActionIds.has(actionId)) failedActionIds.add(actionId);
      }
      const failedResources = new Set(
        [...failedActionIds]
          .map((actionId) => resourceByAction.get(actionId))
          .filter((resourceId): resourceId is string => Boolean(resourceId)),
      );
      const selectedActions = selection.restoreActionIds;
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

      // The Pull is finished — successfully or not — so the review closes and
      // the result lands on the storage card. Anything left over reopens the
      // review, which replans from scratch, so there is nothing to keep alive
      // here.
      if (result.restoreResult) {
        pullOutcomeInputs.current = {
          projectId: restoreBinding.local_project_id,
          plan: currentRestorePlan,
          result: result.restoreResult,
          readiness: result.readiness ?? readiness,
          items: buildPullReviewItems(currentRestorePlan),
          completedResourceIds: new Set([...completedPullResourceIds, ...successfulResources]),
          error: result.error,
        };
        setStorageNotice({
          projectId: restoreBinding.local_project_id,
          storageId: currentRestorePlan.storage_id,
          kind: "outcome",
          outcome: buildPullOutcome(pullOutcomeInputs.current),
        });
        closePullReview();
        if (activeProjectId) {
          await loadProjectData(activeProjectId, config, activeStorageId, workflowEpoch);
        }
        return;
      }

      // Refresh the plan after a partial pass so remaining setup is
      // presented against one current revision.
      const approvedThisPass = new Set(selection.restoreActionIds);
      const hasSkippedActions = currentRestorePlan.actions
        .map((action) => action.action_id)
        .some((actionId) => (
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
        setReadiness(support.readiness);
        if (support.errors.length > 0) {
          setRestoreError(`Changes were applied, but remaining setup could not be prepared: ${support.errors.join(" ")}`);
        }
      }
      if (activeProjectId) {
        await loadProjectData(activeProjectId, config, activeStorageId, workflowEpoch);
      }
    } catch (reason) {
      if (restoreRequest.current === requestId) {
        setPullApplyPhase("complete");
        setRestoreError(errorMessage(reason));
      }
    } finally {
      // Guarded by workflow epoch, not the restore request: completing
      // the apply closes the review, and closePullReview advances the request
      // counter — a request-id guard here would skip this line on every
      // successful Pull and leave the whole app disabled.
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const saveStorageConfig = async (
    update: (current: SyncConfig) => SyncConfig,
    successNotice = "Project storage settings saved.",
    workflowEpoch = hostWorkflowEpoch.current,
  ): Promise<boolean> => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return false;
    setBusy(true);
    setError(null);
    try {
      // Focused project operations can advance the global config revision.
      // Rebase this storage-only edit on the latest document instead of a
      // potentially stale React snapshot.
      const current = await projectSyncApi.getConfig();
      if (hostWorkflowEpoch.current !== workflowEpoch) return false;
      const saved = await projectSyncApi.saveConfig(update(current));
      if (hostWorkflowEpoch.current !== workflowEpoch) return false;
      adoptConfigSnapshot(saved);
      notify(successNotice);
      return true;
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const saveInlineStorage = async (
    storage: SyncConfig["storages"][number],
    expectedBaseline: SyncConfig["storages"][number] | null,
    setupDraftId?: string | null,
    setupDraftRevision?: number | null,
    setupDraftStorageId?: string | null,
  ) => {
    const needsVerifiedR2Save = storage.kind === "s3"
      && (!expectedBaseline || !sameR2Connection(storage, expectedBaseline));
    if (needsVerifiedR2Save) {
      const apiUrl = r2S3ApiUrl(storage.account_id, storage.bucket, storage.s3_endpoint);
      if (!apiUrl) {
        setError("Enter a valid Cloudflare R2 S3 API URL before connecting.");
        return false;
      }
      const workflowEpoch = hostWorkflowEpoch.current;
      setBusy(true);
      setError(null);
      try {
        const current = await projectSyncApi.getConfig();
        if (hostWorkflowEpoch.current !== workflowEpoch) return false;
        const currentStorage = current.storages.find((candidate) => candidate.id === storage.id) ?? null;
        if (expectedBaseline) {
          if (!currentStorage || !sameStorageConfig(currentStorage, expectedBaseline)) {
            throw new Error("Storage settings changed outside this editor. Reload them before saving.");
          }
        } else if (currentStorage) {
          throw new Error("A storage with this identity already exists. Start a new storage draft.");
        }

        const hasAnyDraftPin = setupDraftId != null
          || setupDraftRevision != null
          || setupDraftStorageId != null;
        const usesPinnedDraft = !expectedBaseline
          && !!setupDraftId
          && setupDraftRevision != null
          && !!setupDraftStorageId;
        if (
          hasAnyDraftPin
          && !usesPinnedDraft
        ) {
          throw new Error("The reviewed R2 draft could not be pinned. Review the form and try Finish setup again.");
        }
        if (!expectedBaseline && !usesPinnedDraft && storage.secret_access_key.length === 0) {
          throw new Error("Enter a Secret Access Key or save a complete R2 draft before finishing setup.");
        }
        const result = await projectSyncApi.upsertR2Storage({
          storage_id: expectedBaseline ? storage.id : null,
          // A blank secret may be resolved only from the exact revision and
          // inert identity that the form persisted immediately before this
          // user action. An explicitly typed secret does not reference the
          // backend-held draft at all.
          draft_id: usesPinnedDraft ? setupDraftId : null,
          draft_revision: usesPinnedDraft ? setupDraftRevision : null,
          draft_storage_id: usesPinnedDraft ? setupDraftStorageId : null,
          name: storage.name,
          api_url: apiUrl,
          access_key_id: storage.access_key_id,
          secret_access_key: usesPinnedDraft ? "" : storage.secret_access_key,
          expected_config_revision: current.revision,
        });
        if (hostWorkflowEpoch.current !== workflowEpoch) return false;
        const saved = await projectSyncApi.getConfig();
        if (
          saved.schema !== PROJECT_SYNC_SCHEMA
          || saved.revision < result.config_revision
          || !saved.storages.some((candidate) => candidate.id === result.storage_id)
        ) {
          throw new Error("R2 was verified, but its committed configuration could not be refreshed.");
        }
        if (hostWorkflowEpoch.current !== workflowEpoch) return false;
        adoptConfigSnapshot(saved);
        notify(`${expectedBaseline ? "Updated" : "Connected"} ${storage.name || "Cloudflare R2"}.`);
        await refreshStorageSetupDrafts(undefined, workflowEpoch);
        return true;
      } catch (reason) {
        if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
        return false;
      } finally {
        if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
      }
    }
    const saved = await saveStorageConfig((current) => {
      return {
        ...current,
        storages: applyStorageDraft(current.storages, storage, expectedBaseline),
      };
    });
    if (saved && setupDraftId) {
      // The local-kind path has no backend draft consumer, so the frontend
      // retires the draft after a successful create.
      try {
        await projectSyncApi.discardStorageSetupDraft(setupDraftId);
      } catch {
        // A leftover draft row is harmless; the user can discard it.
      }
      await refreshStorageSetupDrafts();
    }
    return saved;
  };

  /** Commit-free probe of the New-storage form against R2. Failures surface
      exactly where save failures do; success is the caller's verdict to
      render. */
  const testInlineStorage = async (
    storage: SyncConfig["storages"][number],
    setupDraftId?: string | null,
    setupDraftRevision?: number | null,
    setupDraftStorageId?: string | null,
  ) => {
    const apiUrl = r2S3ApiUrl(storage.account_id, storage.bucket, storage.s3_endpoint);
    if (!apiUrl) {
      setError("Enter a valid Cloudflare R2 S3 API URL before testing.");
      return false;
    }
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      const current = await projectSyncApi.getConfig();
      if (hostWorkflowEpoch.current !== workflowEpoch) return false;
      const hasAnyDraftPin = setupDraftId != null
        || setupDraftRevision != null
        || setupDraftStorageId != null;
      const usesPinnedDraft = !!setupDraftId
        && setupDraftRevision != null
        && !!setupDraftStorageId;
      if (
        hasAnyDraftPin
        && !usesPinnedDraft
      ) {
        throw new Error("The reviewed R2 draft could not be pinned. Review the form and try Test connection again.");
      }
      if (!usesPinnedDraft && storage.secret_access_key.length === 0) {
        throw new Error("Enter a Secret Access Key or save a complete R2 draft before testing the connection.");
      }
      const result = await projectSyncApi.testR2Storage({
        storage_id: null,
        draft_id: usesPinnedDraft ? setupDraftId : null,
        draft_revision: usesPinnedDraft ? setupDraftRevision : null,
        draft_storage_id: usesPinnedDraft ? setupDraftStorageId : null,
        name: storage.name,
        api_url: apiUrl,
        access_key_id: storage.access_key_id,
        secret_access_key: usesPinnedDraft ? "" : storage.secret_access_key,
        expected_config_revision: current.revision,
      });
      return hostWorkflowEpoch.current === workflowEpoch && result.verified;
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const clearStorageData = async (storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const storage = configRef.current.storages.find((candidate) => candidate.id === storageId);
    if (!storage) {
      setError("The selected storage no longer exists.");
      return;
    }
    const storageName = storage.name || "Storage";
    const approved = await confirm(
      `Permanently clear all Mallard data from “${storageName}”?\n\nThis deletes every synced project repository and Library resource in this storage. It does not delete unrelated bucket/folder contents, local project files, provider sessions, or the storage connection. This cannot be undone.`,
      { title: "Clear storage data" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;

    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.clearStorageData(
        storageId,
        configRef.current.revision,
      );
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      closePullReview();
      pullOutcomeInputs.current = null;
      setPendingPush(null);
      setPushError(null);
      await refresh(workflowEpoch);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      // The storage page is very likely open behind the confirm dialog, still
      // listing the repositories this call just deleted. Reload it.
      setStorageRefreshEpoch((current) => current + 1);
      const deleted = result.repository_objects_deleted + result.library_objects_deleted;
      const retained = result.unreadable_plan_files_retained;
      notify(
        retained > 0
          ? `${storageName} cleared — ${deleted} object${deleted === 1 ? "" : "s"} deleted. ${retained} unreadable saved plan file${retained === 1 ? " was" : "s were"} retained for inspection.`
          : `${storageName} cleared — ${deleted} Mallard object${deleted === 1 ? "" : "s"} deleted; storage settings and unrelated data kept.`,
        retained > 0 ? "attention" : "ok",
      );
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const removeStorage = async (storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
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
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    const removed = await saveStorageConfig((current) => {
      if (current.links.some((link) => link.storage_id === storageId)) {
        throw new Error(`${storageName} is linked to a project. Unlink it before removing the storage.`);
      }
      return {
        ...current,
        storages: current.storages.filter((candidate) => candidate.id !== storageId),
      };
    }, `${storageName} removed. Stored files were not deleted.`, workflowEpoch);
    if (removed && hostWorkflowEpoch.current === workflowEpoch) {
      setStorageEditorRequest((current) => ({
        mode: "close",
        requestId: (current?.requestId ?? 0) + 1,
      }));
      // Removing the storage whose page you are on leaves nothing to stand
      // on; the trail's root is the natural place to land.
      if (activeStorageSettingsId === storageId) setStorageIndexOpen(true);
    }
  };

  const openNewStorageConfiguration = () => {
    closeProjectSubview();
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setStorageIndexOpen(false);
    setSettingsOpen(false);
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    setStorageEditorRequest((current) => ({
      mode: "create",
      storageKind: "local",
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  /** Reopen a persisted storage setup draft in the New-storage form. The
      draft is fetched here so the workspace's request handler stays
      synchronous, like every other editor request. */
  const openStorageSetupDraft = async (draftId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setError(null);
    let draft: StorageSetupDraft | null = null;
    try {
      draft = await projectSyncApi.getStorageSetupDraft(draftId);
    } catch (reason) {
      setError(errorMessage(reason));
      return;
    }
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    if (!draft) {
      setError("This storage draft no longer exists.");
      await refreshStorageSetupDrafts(undefined, workflowEpoch);
      return;
    }
    closeProjectSubview();
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setStorageIndexOpen(false);
    setSettingsOpen(false);
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    const loaded = draft;
    setStorageEditorRequest((current) => ({
      mode: "resume-draft",
      draft: loaded,
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  const safePushSelection = pendingPush
    ? sanitizePushSelection(
      pushSelected,
      pushResources(pendingPush),
      pendingPush.threadComparison,
      pendingPush.projectContentInventory,
    )
    : pushSelected;
  const pendingPushBlockingCount = pendingPush
    ? pushReviewBlockingCount(
      safePushSelection,
      pendingPush.threadComparison,
      pendingPush.projectContentInventory,
      pushWarningAcknowledgements,
      pushProjectContentRemovals,
    )
    : 0;
  const inlineStorageReview = pendingPush ? {
    kind: "push" as const,
    projectId: pendingPush.projectId,
    storageId: pendingPush.storageId,
    onContinue: () => {
      void publishPendingPush();
    },
    continueDisabled: (safePushSelection.size === 0 && pushProjectContentRemovals.size === 0)
      || pendingPushBlockingCount > 0
      || projectContentLoading,
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
        projectContentInventory={pendingPush.projectContentInventory}
        projectContentScanned={pendingPush.projectContentScanned}
        projectContentLoading={projectContentLoading}
        projectContentRemovals={pushProjectContentRemovals}
        acknowledgedWarningDigests={pushWarningAcknowledgements}
        fetchProgress={fetchProgress}
        showProjectFiles={pendingPush.showProjectFiles}
        activeStep={pushReviewStep}
        onStepChange={setPushReviewStep}
        onToggle={(resourceId) => setPushSelected((current) => {
          const next = new Set(current);
          if (next.has(resourceId)) next.delete(resourceId);
          else next.add(resourceId);
          return next;
        })}
        onSelectionChange={setPushSelected}
        onScanProjectFiles={() => void scanPendingProjectFiles()}
        onToggleProjectContentWarning={(warningDigest) => setPushWarningAcknowledgements((current) => {
          const next = new Set(current);
          if (next.has(warningDigest)) next.delete(warningDigest);
          else next.add(warningDigest);
          return next;
        })}
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
        onUnlinkStorage={() => {
          void unlinkStorage(
            pendingPush.projectId,
            pendingPush.storageId,
            "remote_history_missing",
          );
        }}
        onResetMetadata={() => void resetProjectSyncMetadata(pendingPush.projectId)}
      />
    ),
  } : restorePlan && restoreBinding ? {
    kind: "pull" as const,
    projectId: restoreBinding.local_project_id,
    storageId: restorePlan.storage_id,
    // The card's Pull is the review's footer primary: same apply, same
    // gating. An empty selection still applies — it advances the reviewed
    // baseline, which is the whole point of an acknowledge-only Pull.
    onContinue: () => {
      pullReviewPrimary?.apply();
    },
    continueDisabled: !pullReviewPrimary || pullReviewPrimary.disabledReason !== null,
    continueTitle: pullReviewPrimary?.disabledReason ?? null,
    onClose: closePullReview,
    content: (
      <RestorePlanView
        project={projects.find((candidate) => candidate.local_project_id === restoreBinding.local_project_id) ?? undefined}
        storageName={config.storages.find((storage) => storage.id === restorePlan.storage_id)?.name ?? null}
        projectName={restoreProjectName}
        plan={restorePlan}
        binding={restoreBinding}
        readiness={readiness}
        restoreResult={restoreResult}
        phase={pullApplyPhase}
        supportLoading={reviewSupportLoading}
        completedActionIds={completedPullActionIds}
        completedResourceIds={completedPullResourceIds}
        failedResourceIds={failedPullResourceIds}
        busy={busy}
        error={restoreError}
        onApply={(selection) => void applyReview(selection)}
        onBack={closePullReview}
        onRefreshPlan={() => void planRestore(
          restorePlan.storage_id,
          restoreBinding.bundle_id,
          restoreBinding,
          restoreProjectName,
        )}
        onUnlinkStorage={() => {
          void unlinkStorage(
            restoreBinding.local_project_id,
            restorePlan.storage_id,
            "remote_history_missing",
          );
        }}
        onResetMetadata={() => void resetProjectSyncMetadata(restoreBinding.local_project_id)}
        onPrimaryChange={setPullReviewPrimary}
      />
    ),
  } : null;

  if (!hostIdentityStatus && hostIdentityError) {
    return (
      <main className="sync-host-recovery" aria-labelledby="sync-host-recovery-title">
        {/* No role here. This whole screen IS the failure — heading, prose,
            callout and Retry — and it is what the shell renders instead of the
            app, so it is already there when the view paints. Wrapping a screen
            in role="alert" made a screen reader read the entire page as one
            interruption; the heading and the labelled <main> name it. */}
        <section className="sync-empty-state">
          <div className="sync-empty-illustration" aria-hidden="true">
            <Icon name="alert-triangle" size={24} />
          </div>
          <h1 id="sync-host-recovery-title">Host identity unavailable</h1>
          <p>Mallard could not verify this installation, so sync remains paused.</p>
          {/* `inline`, not `standing`: this box is one paragraph of a screen
              that is already named by its heading and its labelled <main>, so a
              region around it would be a landmark inside a landmark saying the
              same thing. */}
          <Notice severity="danger" timing="inline">{hostIdentityError}</Notice>
          <button type="button" className="btn btn-primary" onClick={() => void refresh()} disabled={loading}>
            {loading ? "Checking…" : "Retry"}
          </button>
        </section>
      </main>
    );
  }

  if (!hostIdentityShellReady) {
    return <StartupLoadingScreen stage={startupStage} />;
  }

  /* One shell notice zone renders one box at the loudest severity present. */
  const shellNotices: NoticeItem[] = [];
  if (backendError) {
    shellNotices.push({
      key: "backend-unavailable",
      severity: "danger",
      title: "Project-sync backend unavailable.",
      message: backendError,
      action: <button type="button" className="btn" onClick={() => void refresh()}>Retry</button>,
    });
  }
  if (error) {
    shellNotices.push({
      key: "shell-error",
      severity: "danger",
      message: error,
      /* This sits outside every early return so failures remain visible on
         every screen, including failures from destructive actions. */
      action: (
        <>
          {catalogRemovalErrorTarget?.message === error && (
            <button
              type="button"
              className="btn"
              onClick={() => {
                setCatalogStorageMenuRequest((current) => ({
                  requestId: (current?.requestId ?? 0) + 1,
                  scopeId: catalogRemovalErrorTarget.scopeId,
                }));
                setCatalogRemovalErrorTarget(null);
                openCatalogScope(catalogRemovalErrorTarget.scopeId);
              }}
              title={`Open ${catalogRemovalErrorTarget.label}`}
            >
              Open storage menu
            </button>
          )}
          <button
            type="button"
            className="sync-notice-dismiss"
            onClick={() => {
              setError(null);
              setCatalogRemovalErrorTarget(null);
            }}
            title="Dismiss"
            aria-label="Dismiss error"
          >
            <Icon name="x" size={12} />
          </button>
        </>
      ),
    });
  }
  if (notice) {
    shellNotices.push({
      key: "shell-notice",
      /* "attention" requires user action; "ok" is a successful receipt. */
      severity: notice.tone === "attention" ? "warning" : "success",
      message: notice.message,
      action: (
        <button type="button" className="sync-notice-dismiss" onClick={() => setNotice(null)}
          title="Dismiss" aria-label="Dismiss notification">
          <Icon name="x" size={12} />
        </button>
      ),
    });
  }
  /* Every rung of this zone is an event. `error` is a handler reporting that
     what the user just did did not happen. `backendError` looks like standing
     content because the startup load writes it, but the same `refresh()` runs
     behind "Reload app data" and the path-bar overflow, so a backend that
     falls over on an explicit refresh would go unannounced if this zone were
     a region — and the shell paints the startup screen until the load
     settles, so even the startup case arrives as a change. A notice is
     advisory. The loudest present event re-times the whole box. */
  const shellNoticeTiming: NoticeTiming = error || backendError
    ? { timing: "blocking" }
    : { timing: "progress" };

  return (
    <div
      className={`sync-app${sidebarVisible ? "" : " sidebar-hidden"}${resizingSidebar ? " resizing-sidebar" : ""}`}
      style={{ "--sync-sidebar-width": `${sidebarWidth}px` } as CSSProperties}
    >
      <button
        type="button"
        className="sync-sidebar-toggle"
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
        storageDrafts={storageSetupDrafts}
        onOpenStorageDraft={(draftId) => void openStorageSetupDraft(draftId)}
        onDiscardStorageDraft={(draftId) => void discardStorageSetupDraft(draftId)}
        storageUsage={Object.fromEntries(config.storages.map((storage) => [
          storage.id,
          config.links.filter((link) => link.storage_id === storage.id).length,
        ]))}
        storageIssues={storageIssues}
        activeProjectId={activeProjectId}
        activeStorageId={activeStorageSettingsId}
        catalogScopes={catalogScopes}
        activeCatalogScopeId={activeCatalogScope?.scope.scope_id ?? null}
        addingCatalogScope={addingCatalogScope}
        catalogIndexOpen={catalogIndexOpen}
        storageIndexOpen={storageIndexOpen}
        settingsOpen={settingsOpen}
        loading={loading}
        busy={busy || !!inlineStorageReview}
        activityOpen={activityOpen}
        unreadLogs={unreadLogs}
        onOpenProjects={openProjectsIndex}
        onSelectProject={openProject}
        onRemoveProject={(id) => void removeProject(id)}
        onToggleActivity={() => {
          const next = !activityOpen;
          activityOpenRef.current = next;
          if (next) setUnreadLogs(0);
          setActivityOpen(next);
          if (next) {
            window.requestAnimationFrame(() => logDrawerRef.current?.focus());
          }
        }}
        onAddProject={() => void beginAddProject()}
        onOpenStorage={openStorageWorkspace}
        onRemoveStorage={(storageId) => void removeStorage(storageId)}
        onAddStorage={openNewStorageConfiguration}
        onOpenCatalogIndex={openCatalogIndex}
        onOpenCatalogScope={openCatalogScope}
        onAddCatalogScope={beginAddCatalogScope}
        onRemoveCatalogScope={(scopeId) => void removeCatalogScope(scopeId)}
        onOpenStorageIndex={openStorageIndex}
        onOpenSettings={openSettings}
      />

      <div
        className="sync-sidebar-resizer"
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

      <div className={`sync-workspace${activityOpen ? " logs-open" : ""}${resizingLog ? " resizing-log" : ""}`}>
        {/* The project workspace merges the window chrome into its path bar, so
            there is one row at the top instead of a named strip above an empty
            one. Every other screen still gets a titlebar of its own. */}
        {showsPathBar ? null : (
        <header className="sync-titlebar" data-tauri-drag-region>
          <div className="sync-titlebar-context" data-tauri-drag-region title={workspaceTitle ?? undefined}>
            {workspaceTitle && (
              <>
                <Icon name={workspaceIcon} size={16} />
                <strong>{workspaceTitle}</strong>
              </>
            )}
            {activeDraftSummary && <span>Draft setup</span>}
          </div>
          {windowActions}
        </header>
        )}

        {/* Keeps `sync-backend-banner` for the shell's own geometry only: the
            zone is a sibling of the path bar and the page, and that class is
            what gives it their width, their frame and the 10px below it. */}
        <NoticeZone
          items={shellNotices}
          subject="messages from Mallard"
          className="sync-backend-banner"
          {...shellNoticeTiming}
        />

        {addingCatalogScope ? (
          <AddCatalogScopeWorkspace
            workflowEpoch={hostWorkflowEpoch.current}
            isWorkflowCurrent={(epoch) => hostWorkflowEpoch.current === epoch}
            busy={busy}
            onClose={openCatalogIndex}
            onCreated={(scope, epoch) => void completeAddCatalogScope(scope, epoch)}
          />
        ) : activeCatalogScope ? (
          <StorageCatalogPage
            key={activeCatalogScope.scope.scope_id}
            scope={activeCatalogScope}
            scopes={catalogScopes}
            storages={catalogStorages}
            appActions={appActions}
            onOpenScope={openCatalogScope}
            onOpenStorageSettings={(storageId) => openStorageWorkspace(storageId, true)}
            onAddStorage={openNewStorageConfiguration}
            onAddScope={() => beginAddCatalogScope()}
            onRemoveScope={() => void removeCatalogScope(activeCatalogScope.scope.scope_id)}
            onProfileRenamed={() => void refreshCatalogScopes()}
            openStorageMenuRequestId={catalogStorageMenuRequest?.scopeId === activeCatalogScope.scope.scope_id
              ? catalogStorageMenuRequest.requestId
              : null}
            notify={(message) => notify(message)}
            onScopeGone={() => {
              setCatalogScopeId(null);
              setCatalogIndexOpen(true);
              void refreshCatalogScopes();
            }}
          />
        ) : catalogIndexOpen ? (
          <CatalogIndexPage
            scopes={catalogScopes}
            busy={busy}
            appActions={appActions}
            onOpenScope={openCatalogScope}
            onAddScope={beginAddCatalogScope}
          />
        ) : settingsOpen ? (
          <SettingsPage
            appActions={appActions}
            onLogsChanged={() => void loadActivityLogs()}
          />
        ) : storageIndexOpen ? (
          <StorageIndexPage
            storages={config.storages}
            links={config.links}
            storageIssues={storageIssues}
            drafts={storageSetupDrafts}
            busy={busy}
            appActions={appActions}
            onOpenStorage={(storageId) => openStorageWorkspace(storageId, false)}
            onOpenStorageDraft={(draftId) => void openStorageSetupDraft(draftId)}
            onAddStorage={openNewStorageConfiguration}
          />
        ) : (
        <ProjectLinksWorkspace
            projects={projects}
            activeProjectId={activeProjectId}
            activeStorageId={activeStorageId}
            activeProjectSubview={activeProjectSubview}
            bindings={bindings}
            profiles={profiles}
            providers={agentProviders}
            storages={config.storages}
            storageSetupDrafts={storageSetupDrafts}
            links={config.links}
            loading={loading}
            busy={busy}
            error={error}
            conversationPathAudits={conversationPathAudits}
            conversationPathAuditErrors={conversationPathAuditErrors}
            conversationPathAuditLoading={conversationPathAuditLoading}
            fetchProgress={fetchProgress}
            onSelectStorage={(projectId, storageId) => selectStorage(projectId, storageId)}
            onOpenSessionDetail={openSessionDetail}
            onCloseSessionDetail={closeProjectSubview}
            onLinkStorage={(projectId, storageId) => linkStorage(projectId, storageId)}
            onUnlinkStorage={(projectId, storageId) => unlinkStorage(projectId, storageId)}
            onPush={(projectId, storageId) => openPushChooser(projectId, storageId)}
            onPull={(projectId, storageId) => beginProjectRestore(projectId, storageId)}
            siblingNotice={siblingNotice}
            onConnectSibling={(projectId, storageId, matches) => {
              setPendingBundleConnection({ projectId, storageId, matches, reason: "sibling" });
            }}
            onConfigureBinding={configureProjectBinding}
            onRepairConversationPaths={(projectId) => repairConversationPaths(projectId)}
            onRenameProject={(projectId, alias) => renameProject(projectId, alias)}
            onResetProjectMetadata={(projectId) => resetProjectSyncMetadata(projectId)}
            onRefresh={() => void refresh()}
            onAddProject={() => void beginAddProject()}
            onSelectProject={openProject}
            appActions={appActions}
            onAddStorage={openNewStorageConfiguration}
            onOpenStorage={openStorageWorkspace}
            onOpenStorageIndex={openStorageIndex}
            onClearStorage={(storageId) => void clearStorageData(storageId)}
            onRemoveStorage={(storageId) => void removeStorage(storageId)}
            onSaveStorage={saveInlineStorage}
            onTestStorage={testInlineStorage}
            onStorageDraftsChanged={() => void refreshStorageSetupDrafts()}
            storageEditorRequest={storageEditorRequest}
            onStorageEditorRequestHandled={() => setStorageEditorRequest(null)}
            onStorageEditorChange={setActiveStorageSettingsId}
            onStorageIssueChange={updateStorageIssue}
            historyRefreshEpoch={historyRefreshEpoch}
            storageRefreshEpoch={storageRefreshEpoch}
            inlineStorageReview={inlineStorageReview}
            storageNotice={storageNotice}
            onStorageNoticeAction={(outcome) => void runOutcomeAction(outcome)}
            onDismissStorageNotice={() => setStorageNotice(null)}
            newProjectSetup={setupDraftId ? (
              <ProjectSetupWorkspace
                key={setupDraftId}
                draftId={setupDraftId}
                workflowEpoch={hostWorkflowEpoch.current}
                isWorkflowCurrent={(workflowEpoch) => (
                  hostWorkflowEpoch.current === workflowEpoch
                )}
                providers={agentProviders}
                profiles={profiles}
                projects={projects}
                storages={config.storages}
                busy={busy}
                onClose={() => void closeSetupDraft()}
                onDiscard={(draftId) => void discardSetupDraft(draftId)}
                onAddStorage={openNewStorageConfiguration}
                onFinalized={(detail, workflowEpoch) => (
                  void completeSetup(detail, workflowEpoch)
                )}
              />
            ) : null}
          />
        )}

        <div
          ref={logDrawerRef}
          id="activity-log-drawer"
          className={`log-drawer sync-log-drawer${activityOpen ? " open" : ""}`}
          style={{ height: activityOpen ? logHeight : undefined }}
          role="region"
          aria-label="Logs"
          tabIndex={-1}
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
          {activityOpen && (
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
              onManage={openSettings}
              onClose={() => {
                activityOpenRef.current = false;
                setActivityOpen(false);
                window.requestAnimationFrame(() => {
                  document.getElementById("activity-log-trigger")?.focus();
                });
              }}
            />
          )}
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
          binding={editingBinding}
          providers={agentProviders}
          profiles={profiles}
          busy={busy}
          error={error}
          actionLabel="Save project setup"
          onCancel={() => setEditingBinding(null)}
          onSubmit={(next) => void saveRemap(next)}
        />
      )}

    </div>
  );
}
