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
  AppTheme,
  BundleFetchProgress,
  BundleRecipe,
  BundleSnapshotSummary,
  HostIdentityStatus,
  InventoryResource,
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
  PullReview,
  SetupDraftSummary,
  SiblingBundleStatus,
  CommitStorageDraftResult,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSetupDraft,
  StorageConnectionChangedEvent,
  StorageSetupDraftsChangedEvent,
  StorageSummary,
  SyncConfig,
  ThreadSyncComparison,
} from "../../types";
import { PROJECT_SYNC_SCHEMA } from "../../types";
import Icon from "../Icons";
import Notice, { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import BundleConnectionDialog from "./BundleConnectionDialog";
import ActivityLogDrawer from "./ActivityLogDrawer";
import SettingsPage from "./SettingsPage";
import { getPreference } from "../../preferences";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import ProjectBindingEditor, {
  type BindingProfileSelection,
  type ProjectBindingDraft,
  type ProjectBindingSubmission,
} from "./ProjectBindingEditor";
import ProjectLinksWorkspace, { type StorageNotice } from "./ProjectLinksWorkspace";
import { StorageDraftSaveQueue } from "./storageDraftSaveQueue";
import ResizeGrip from "./ResizeGrip";
import StorageIndexPage from "./StorageIndexPage";
import { storageSubtitle, storageSubtitleIsPath } from "./StorageSettings";
import StorageProviderPicker from "./StorageProviderPicker";
import { PathBarBelow, PathBarMenu, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import PushResourceWorkspace, {
  includeRequiredProjectContentDirectories,
  pullRequiredCopy,
  pushReviewDisabledReason,
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
import type { CatalogCapabilityDescriptor, CatalogScopeDto, CatalogScopeView } from "../storage-catalog/types";
import {
  catalogScopeLabel,
  catalogScopeRemovalMessage,
  catalogScopeRemovalRequiresCleanup,
  catalogScopeViews,
} from "../storage-catalog/model";
import RestorePlanView, { type PullReviewPrimary } from "./RestorePlanView";
import StartupLoadingScreen, { type StartupStage } from "./StartupLoadingScreen";
import { type SyncReviewStep } from "./reviewSteps";
import { buildPullReviewItems, pullActionResourceIds } from "./pullReviewModel";
import {
  buildPullOutcome,
  buildPushOutcome,
  type SyncOutcome,
  type SyncOutcomeAction,
} from "./syncOutcome";
import { projectSyncApi } from "./api";
import {
  applyPullReview,
  beginPullReview,
  hasPendingRestoreMutations,
  type PullApplyPhase,
  type PullReviewSelection,
} from "./pullReviewFlow";
import {
  inventoryResources,
  maintenanceActionFor,
  projectContentReviewRow,
  projectLabel,
  recipeSelection,
  recipeWithSelection,
  type RenameResult,
} from "./model";

interface Props {
  theme: AppTheme;
  onThemeChange: (theme: AppTheme) => void;
}

const MAX_LIVE_LOG_LINES = 2_000;

const EMPTY_CONFIG: SyncConfig = {
  schema: PROJECT_SYNC_SCHEMA,
  revision: 0,
  storages: [],
  projects: [],
  links: [],
};

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
  credentialRedactionEnabled: boolean;
  projectId: string;
  storageId: string;
  projectName: string;
  project: LocalProjectSummary;
  binding: ProjectBinding | null;
  storage: StorageSummary;
  inventory: ResourceInventoryModel;
  savedRecipe: BundleRecipe | null;
  projectDefaults: Set<string>;
  threadComparison?: ThreadSyncComparison | null;
  /* Why the comparison is absent when it is: without it no conversation is
     selectable, and the review must say so rather than open empty. */
  threadComparisonError: string | null;
  projectContentInventory: ProjectContentInventory | null;
  projectContentScanned: boolean;
  showProjectFiles: boolean;
}

function pushResources(pending: PendingPush): InventoryResource[] {
  const resources = inventoryResources(pending.inventory);
  const byId = new Map(resources.map((resource) => [resource.resource_id, resource]));
  for (const entry of pending.projectContentInventory?.entries ?? []) {
    const resourceId = entry.descriptor.resource_id;
    byId.set(resourceId, projectContentReviewRow(entry, byId.get(resourceId)));
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

export default function ProjectSync({ theme, onThemeChange }: Props) {
  const [config, setConfig] = useState<SyncConfig>(EMPTY_CONFIG);
  const [registrations, setRegistrations] = useState<LocalProjectRegistration[]>([]);
  const [repositoryKinds, setRepositoryKinds] = useState<Record<string, boolean>>({});
  const [bindings, setBindings] = useState<ProjectBinding[]>([]);
  const [profiles, setProfiles] = useState<ProviderProfileSummary[]>([]);
  const [agentProviders, setAgentProviders] = useState<ProviderDescriptor[]>([]);
  const [storageProviders, setStorageProviders] = useState<StorageProviderDescriptor[]>([]);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [activeProjectSubview, setActiveProjectSubview] = useState<ActiveProjectSubview>({ kind: "overview" });
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [inventory, setInventory] = useState<ResourceInventoryModel | null>(null);
  const [activeStorageByProject, setActiveStorageByProject] = useState<Record<string, string>>(storedActiveStorages);
  const [activeStorageSettingsId, setActiveStorageSettingsId] = useState<string | null>(null);
  // The page the storage trail is rooted at: every storage page starts its
  // path with a Storages crumb, and this is where that crumb goes.
  const [storageIndexOpen, setStorageIndexOpen] = useState(false);
  const [storageProviderPickerOpen, setStorageProviderPickerOpen] = useState(false);
  // Add storage starts with a provider choice; the picker is its own screen,
  // rooted at the Storages crumb like every storage page.
  // The Catalog menu starts empty and grows one row per (provider profile ×
  // resource kind) the user adds. Each registered resource owns its own
  // storage links; there is no storage-wide Catalog selection.
  const [catalogScopes, setCatalogScopes] = useState<CatalogScopeView[]>([]);
  /* The registry behind the rows, kept for the doors an empty Catalog draws. */
  const [catalogCapabilities, setCatalogCapabilities] = useState<CatalogCapabilityDescriptor[]>([]);
  /* The Kind a door chose on the way into Add; null when the verb was bare. */
  const [catalogAddChoiceKey, setCatalogAddChoiceKey] = useState<string | null>(null);
  const [catalogScopeId, setCatalogScopeId] = useState<string | null>(null);
  const [addingCatalogScope, setAddingCatalogScope] = useState(false);
  const [catalogIndexOpen, setCatalogIndexOpen] = useState(false);
  const [storageIssues, setStorageIssues] = useState<Record<string, string>>({});
  const [storageEditorRequest, setStorageEditorRequest] = useState<
    | { mode: "open"; storageId: string; settingsExpanded: boolean; requestId: number }
    | { mode: "resume-draft"; draft: StorageSetupDraft; requestId: number }
    | { mode: "close"; requestId: number }
    | null
  >(null);
  const storageDraftSaveQueue = useRef(new StorageDraftSaveQueue()).current;
  const storageNavigationGuardRef = useRef<(() => Promise<boolean>) | null>(null);
  const setStorageNavigationGuard = useCallback((guard: (() => Promise<boolean>) | null) => {
    storageNavigationGuardRef.current = guard;
  }, []);
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
  /* Beside the selection, not inside the review panel: the path bar's Push
     publishes the same review, so its refusal has to see this too. */
  const [pushDeletionApproved, setPushDeletionApproved] = useState(false);
  const [projectContentLoading, setProjectContentLoading] = useState(false);
  const [pushError, setPushError] = useState<string | null>(null);
  const [pushReviewStep, setPushReviewStep] = useState<SyncReviewStep>("history");
  // A gate the push (or a pull plan) cannot get past never opens a review;
  // it lands as a banner on the storage card with the way forward on it.
  const [storageNotice, setStorageNotice] = useState<StorageNotice | null>(null);
  // What produced the current Pull outcome, so Recheck can rebuild the banner
  // against fresh readiness instead of replanning the whole Pull.
  const pullOutcomeInputs = useRef<Parameters<typeof buildPullOutcome>[0] | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [logScrollToBottomEpoch, setLogScrollToBottomEpoch] = useState(0);
  const [logRefreshEpoch, setLogRefreshEpoch] = useState(0);
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
  const liveLogLinesRef = useRef<LogLine[]>([]);
  const liveLogSubscribersRef = useRef(new Set<(line: LogLine) => void>());
  const sidebarResizeRef = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null);

  const [setupDrafts, setSetupDrafts] = useState<SetupDraftSummary[]>([]);
  const [storageSetupDrafts, setStorageSetupDrafts] = useState<StorageSetupDraft[]>([]);
  const [setupDraftId, setSetupDraftId] = useState<string | null>(null);
  const [editingBinding, setEditingBinding] = useState<ProjectBindingDraft | null>(null);

  const [restorePlan, setRestorePlan] = useState<PullReview | null>(null);
  const [fetchProgress, setFetchProgress] = useState<BundleFetchProgress | null>(null);
  const [restoreBinding, setRestoreBinding] = useState<ProjectBinding | null>(null);
  const [restoreProjectName, setRestoreProjectName] = useState("project");
  const [historyRefreshEpoch, setHistoryRefreshEpoch] = useState(0);
  const [storageRefreshEpoch, setStorageRefreshEpoch] = useState(0);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [pullApplyPhase, setPullApplyPhase] = useState<PullApplyPhase>("idle");
  // The open Pull review's footer primary, mirrored out so the storage card's
  // Pull button runs the same apply under the same gating.
  const [pullReviewPrimary, setPullReviewPrimary] = useState<PullReviewPrimary | null>(null);
  const [reviewSupportLoading, setReviewSupportLoading] = useState(false);
  const pullApplyInFlight = useRef(false);
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
    const storageIds = new Set(config.storages.map((storage) => storage.storage_id));
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

  const readLiveLogLines = useCallback((): readonly LogLine[] => liveLogLinesRef.current, []);

  const subscribeToLiveLogLines = useCallback((subscriber: (line: LogLine) => void) => {
    liveLogSubscribersRef.current.add(subscriber);
    return () => liveLogSubscribersRef.current.delete(subscriber);
  }, []);

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

  useEffect(() => {
    const unlisten = listen<LogLine>("sync-log", (event) => {
      const retained = liveLogLinesRef.current;
      retained.push(event.payload);
      if (retained.length > MAX_LIVE_LOG_LINES) {
        retained.splice(0, retained.length - MAX_LIVE_LOG_LINES);
      }
      for (const subscriber of liveLogSubscribersRef.current) subscriber(event.payload);
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
    id: storage.storage_id,
    name: storage.name,
    provider_label: storage.provider_label,
    icon: storage.icon,
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
      || storageProviderPickerOpen
      || settingsOpen
      || activeStorageSettingsId != null);
  const pendingConnectionProject = pendingBundleConnection
    ? registrations.find((candidate) => candidate.local_project_id === pendingBundleConnection.projectId) ?? null
    : null;
  const pendingConnectionStorage = pendingBundleConnection
    ? config.storages.find((candidate) => candidate.storage_id === pendingBundleConnection.storageId) ?? null
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
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
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
    setRegistrations([]);
    setRepositoryKinds({});
    setBindings([]);
    setProfiles([]);
    configRef.current = EMPTY_CONFIG;
    configSnapshotValidRef.current = false;
    setConfig(EMPTY_CONFIG);
    setError(null);
    setBackendError(null);
    setNotice(null);
    setBusy(false);
  }, []);
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
    setStartupStage("installation");
    const failures: string[] = [];
    let nextProjects: LocalProjectRegistration[] = [];
    let nextConfig: SyncConfig = EMPTY_CONFIG;
    let nextBindings: ProjectBinding[] = [];
    let nextProfiles: ProviderProfileSummary[] = [];
    let nextAgentProviders: ProviderDescriptor[] = [];
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
      projectSyncApi.listStorageProviders(),
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
    const [agentProvidersResult, storageProvidersResult, profilesResult, bindingsResult] = await providerReads;
    if (shellRequest.current !== requestId) return unavailableResult();
    if (agentProvidersResult.status === "fulfilled") {
      nextAgentProviders = agentProvidersResult.value;
      setAgentProviders(nextAgentProviders);
    } else {
      failures.push(`Agent providers: ${errorMessage(agentProvidersResult.reason)}`);
      setAgentProviders([]);
    }
    if (storageProvidersResult.status === "fulfilled") {
      setStorageProviders(storageProvidersResult.value);
    } else {
      failures.push(`Storage providers: ${errorMessage(storageProvidersResult.reason)}`);
      setStorageProviders([]);
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
    return { nextProjects, nextConfig, nextBindings, nextProfiles, hostIdentityReady: true };
  };

  /* Storage transports fail in AWS SDK prose ("S3 conditional put …: dispatch
     failure: io error"). The storage page already translates that into the
     sentence that names the fix; Push and Pull showed the raw string for the
     same root cause. One resolution point, so both surfaces agree. */
  const storageFailureMessage = (storageId: string, reason: unknown): string => storageAccessErrorMessage(
    configRef.current.storages.find((candidate) => candidate.storage_id === storageId) ?? null,
    reason,
  );

  /* A running Push or Pull can write remote and local state. Killing the
     process midway can leave a partial result that Mallard does not retry.

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
      // `readiness` from the current review when it needs it.
      if (storageId && nextDetail.binding?.state === "active") {
        setActiveStorageByProject((current) => ({ ...current, [projectId]: storageId }));
      }

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
        // command that reports secret presence only; no value ever reaches
        // the webview.
        void refreshStorageSetupDrafts();
      },
    );
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  /* Runtime connection health rides the storage listing. The config file
     and its revision are unchanged when a credential is refused, so this
     re-read skips the revision gate the config-changed listener applies. */
  useEffect(() => {
    let disposed = false;
    const unlisten = listen<StorageConnectionChangedEvent>(
      "project-sync-storage-connection-changed",
      () => {
        const requestId = ++configEventRequest.current;
        void projectSyncApi.getConfig().then((nextConfig) => {
          if (
            disposed
            || requestId !== configEventRequest.current
            || nextConfig.schema !== PROJECT_SYNC_SCHEMA
          ) return;
          adoptConfigSnapshot(nextConfig);
        }).catch(() => {
          // The listing already shown stays; the next read tells the rest.
        });
      },
    );
    return () => {
      disposed = true;
      void unlisten.then((dispose) => dispose());
    };
  }, [adoptConfigSnapshot]);

  const saveStorageDraftBeforeNavigation = async () => (
    storageNavigationGuardRef.current ? storageNavigationGuardRef.current() : true
  );

  const closeWorkspaceSettings = async (): Promise<boolean> => {
    if (!(await saveStorageDraftBeforeNavigation())) return false;
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
    setStorageProviderPickerOpen(false);
    setSettingsOpen(false);
    setStorageEditorRequest((current) => ({
      mode: "close",
      requestId: (current?.requestId ?? 0) + 1,
    }));
    return true;
  };

  /** One route into a storage page, shared by the sidebar, the Storages
      index, and the banners that deep-link a specific storage's settings. */
  const openStorageWorkspace = async (storageId: string, settingsExpanded: boolean) => {
    if (!(await saveStorageDraftBeforeNavigation())) return;
    closeProjectSubview();
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setStorageIndexOpen(false);
    setStorageProviderPickerOpen(false);
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

  const openStorageIndex = async () => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    setStorageIndexOpen(true);
    setStorageProviderPickerOpen(false);
  };

  const openProjectsIndex = async () => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    projectRequest.current += 1;
    setLoading(false);
    setActiveProjectId(null);
    setDetail(null);
    setInventory(null);
    setSiblingNotice(null);
  };

  const openCatalogIndex = async () => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    setCatalogIndexOpen(true);
  };

  const openSettings = async () => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
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
    if (!(await closeWorkspaceSettings())) return;
    setBusy(true);
    setError(null);
    try {
      const created = await projectSyncApi.createSetupDraft(picked);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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
      // The rows and the capabilities they register, joined by coordinate:
      // the descriptor is where a row's glyph and artwork come from.
      const [listed, capabilities] = await Promise.all([
        storageCatalogApi.listScopes(),
        storageCatalogApi.listCapabilities(),
      ]);
      if (
        (shellRequestId !== undefined && shellRequest.current !== shellRequestId)
        || hostWorkflowEpoch.current !== requestedWorkflowEpoch
      ) return;
      setCatalogScopes(catalogScopeViews(listed, capabilities));
      setCatalogCapabilities(capabilities);
    } catch {
      // The menu is a convenience surface; the rest of the shell stays usable.
    }
  };

  /** Opens the Add-row workspace: Kind, then provider home, and nothing else.
      A door arrives with Kind already chosen. */
  const beginAddCatalogScope = async (choiceKey?: string) => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    setCatalogIndexOpen(false);
    setCatalogAddChoiceKey(choiceKey ?? null);
    setAddingCatalogScope(true);
  };

  const openCatalogScope = async (scopeId: string) => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    setCatalogIndexOpen(false);
    setCatalogScopeId(scopeId);
  };

  const completeAddCatalogScope = async (scope: CatalogScopeDto, workflowEpoch: number) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(scope.scope.scope_id);
    notify(`${catalogScopeLabel(scope)} added to the Catalog.`);
    await refreshCatalogScopes(undefined, workflowEpoch);
  };

  const removeCatalogScope = async (scopeId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const scope = catalogScopes.find((candidate) => candidate.scope.scope_id === scopeId);
    if (!scope) {
      await refreshCatalogScopes(undefined, workflowEpoch);
      return;
    }
    const label = catalogScopeLabel(scope);
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

  const openSetupDraft = async (draftId: string) => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
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

  /** Discard a storage setup draft from the rail. The workspace watches the
      draft list: a form open on that draft drops it the moment the list no
      longer carries it. */
  const discardStorageSetupDraft = async (draftId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const draft = storageSetupDrafts.find((candidate) => candidate.draft_id === draftId);
    const approved = await confirm(
      `Discard the storage draft for “${draft?.name || "(unnamed)"}”?\n\nThis removes only Mallard’s local setup draft; external storage resources are not changed.`,
      { title: "Discard storage draft" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    try {
      await storageDraftSaveQueue.enqueue(() => projectSyncApi.discardStorageSetupDraft(draftId));
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
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

  const openPushChooser = async (
    projectId: string,
    storageId: string,
    workflowEpoch = hostWorkflowEpoch.current,
  ) => {
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    setBusy(true);
    setError(null);
    setPushError(null);
    try {
      if (restorePlan) closePullReview();
      setPushReviewStep("history");
      setPushProjectContentRemovals(new Set());
      setPushWarningAcknowledgements(new Set());
      setProjectContentLoading(false);
      setStorageNotice(null);
      // One backend call runs a single discovery pass and a single remote
      // snapshot for every review section.
      const context = await projectSyncApi.preparePushReview(projectId, storageId);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      const nextDetail = context.project;
      const nextInventory = context.inventory;
      const storage = config.storages.find((candidate) => candidate.storage_id === storageId);
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
        credentialRedactionEnabled: context.credential_redaction_enabled,
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
        threadComparisonError: context.thread_comparison_error ?? null,
        projectContentInventory: lockedProjectContentInventory,
        projectContentScanned: lockedProjectContentInventory !== null,
        showProjectFiles: !isGitRepository || lockedProjectContentInventory !== null,
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
  ) => {
    if (!pendingPush) return;
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
        pendingPush.credentialRedactionEnabled,
        projectContentReviewToken,
        projectContentRemovalIds,
        acknowledgedWarningDigests,
        Object.fromEntries((pendingPush?.threadComparison?.entries ?? []).map((entry) => [entry.resource_id, entry.subagent_selection])),
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
    // Capture only the visible selection. The backend retains stored conversations.
    const recipe = recipeWithSelection(baseRecipe, resources, safeSelection);
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
    setFetchProgress(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(true);
    setRestorePlan(null);
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
        const finalization = await projectSyncApi.applyRestore(review.restorePlan, []);
        if (restoreRequest.current !== requestId) return null;
        if (
          !finalization.success
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

  /** Finished-sync actions run after the review closes. Provider maintenance
   *  offers keep their backend order and execute independently. */
  const runOutcomeAction = async (outcome: SyncOutcome, action: SyncOutcomeAction) => {
    if (!outcome.actions.includes(action)) return;
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
        const readiness = await projectSyncApi.getRestoreReadiness(inputs.projectId, inputs.plan.storage_id);
        const next = buildPullOutcome({ ...inputs, readiness, error: null });
        pullOutcomeInputs.current = { ...inputs, readiness, error: null };
        if (next.tone === "success" && next.actions.length === 0) {
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
    if (!action.offer.available) return;
    const descriptor = maintenanceActionFor(
      agentProviders,
      action.offer.provider,
      action.offer.action_id,
    );
    if (!descriptor) {
      notify(`“${action.label}” is no longer declared by this provider.`, "attention");
      return;
    }
    const confirmed = descriptor.requires_confirmation
      ? await confirm(
        `${action.label}?\n\n${action.title}\n\n${descriptor.description}`,
        { title: action.label },
      )
      : false;
    if (descriptor.requires_confirmation && !confirmed) return;
    setBusy(true);
    try {
      const result = await projectSyncApi.executeProviderMaintenanceAction({
        provider: action.offer.provider,
        action_id: action.offer.action_id,
        arguments: action.offer.arguments,
        confirmed,
      });
      setHistoryRefreshEpoch((epoch) => epoch + 1);
      if (result.ready) {
        const remainingActions = outcome.actions.filter((candidate) => candidate !== action);
        setStorageNotice((current) => {
          if (current?.kind !== "outcome" || current.outcome !== outcome) return current;
          if (remainingActions.length === 0) return null;
          return {
            ...current,
            outcome: { ...outcome, actions: remainingActions },
          };
        });
        notify(result.summary);
      } else {
        notify(result.summary, "attention");
      }
    } catch (reason) {
      notify(errorMessage(reason), "attention");
    } finally {
      setBusy(false);
    }
  };

  const beginProjectRestore = async (projectId: string, storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      setPendingPush(null);
      setPushError(null);
      setStorageNotice(null);
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
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
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
    setRestoreError(null);
    setPullApplyPhase("idle");
    setReviewSupportLoading(false);
  };

  async function resetProjectSyncMetadata(projectId: string): Promise<void> {
    const workflowEpoch = hostWorkflowEpoch.current;
    const summary = projects.find((candidate) => candidate.local_project_id === projectId);
    if (!summary) {
      setError("The selected local project no longer exists.");
      return;
    }
    const approved = await confirm(
      `Reset local sync state for “${projectLabel(summary)}”?\n\nMallard will remove all local session bindings for this project, including restored-session path mappings. It will also forget reviewed sync bases and Pull results. Project files, provider sessions, project setup, recipes, storage configuration, and cloud data will not be deleted.`,
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
        + result.materializations_removed;
      notify(`Local sync state reset — ${removed} derived record${removed === 1 ? "" : "s"} removed; files and cloud data kept.`);
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
        id: "theme",
        label: "Dark theme",
        icon: "moon" as const,
        checked: theme === "dark",
        onSelect: () => onThemeChange(theme === "dark" ? "light" : "dark"),
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
      {workspaceTitle === "Projects" ? null : (
        <button
          type="button"
          className="sync-theme-button"
          onClick={() => onThemeChange(theme === "dark" ? "light" : "dark")}
          title={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
          aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
        >
          <Icon name={theme === "dark" ? "moon" : "sun"} size={18} />
        </button>
      )}
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
        <PathBarMenu
          id="sync-app-actions-menu"
          groups={appActions}
          label="App actions"
          align="end"
          open={appMenuOpen}
          registerRef={() => undefined}
          onClose={() => setAppMenuOpen(false)}
        />
      </div>
    </div>
  );

  // One way into a project, shared by the sidebar and the path bar's repo crumb.
  const openProject = async (projectId: string) => {
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    if (restorePlan) closePullReview();
    await selectProject(projectId);
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
    const storage = config.storages.find((candidate) => candidate.storage_id === storageId);
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
    if (!restorePlan || !restoreBinding || pullApplyInFlight.current) return;
    pullApplyInFlight.current = true;
    const requestId = ++restoreRequest.current;
    const workflowEpoch = hostWorkflowEpoch.current;
    const currentRestorePlan = restorePlan;
    if (getPreference("openLogsOnSync")) openActivity();
    setBusy(true);
    setReviewSupportLoading(false);
    setRestoreError(null);
    setNotice(null);
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
      if (!result.restoreResult) {
        setRestoreError(result.error);
        return;
      }
      if (result.restoreResult.applied_action_ids.length > 0) {
        setHistoryRefreshEpoch((epoch) => epoch + 1);
      }
      const resourceByAction = pullActionResourceIds(currentRestorePlan);
      const appliedActionIds = new Set(result.restoreResult.applied_action_ids);
      const successfulResources = new Set(selection.resourceIds.filter((resourceId) => {
        const actionIds = selection.restoreActionIds.filter((id) => resourceByAction.get(id) === resourceId);
        return actionIds.length > 0 && actionIds.every((id) => appliedActionIds.has(id));
      }));
      pullOutcomeInputs.current = {
        projectId: restoreBinding.local_project_id,
        plan: currentRestorePlan,
        result: result.restoreResult,
        readiness: result.readiness,
        items: buildPullReviewItems(currentRestorePlan),
        completedResourceIds: successfulResources,
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
    } catch (reason) {
      if (restoreRequest.current === requestId) setRestoreError(errorMessage(reason));
    } finally {
      pullApplyInFlight.current = false;
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  /** Verify a draft against its provider without committing anything. The
      failure lands in the shell error zone like every other refusal; success
      is the caller's verdict to render. */
  const testStorageDraft = async (draftId: string, expectedRevision: number): Promise<boolean> => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.testStorageSetupDraft(draftId, expectedRevision);
      return hostWorkflowEpoch.current === workflowEpoch && result.verified;
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const commitStorageDraft = async (
    draftId: string,
    expectedDraftRevision: number,
  ): Promise<CommitStorageDraftResult | null> => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.commitStorageSetupDraft(
        draftId,
        expectedDraftRevision,
      );
      if (hostWorkflowEpoch.current !== workflowEpoch) return null;
      const saved = await projectSyncApi.getConfig();
      if (hostWorkflowEpoch.current !== workflowEpoch) return null;
      if (
        saved.schema !== PROJECT_SYNC_SCHEMA
        || saved.revision < result.config_revision
        || !saved.storages.some((candidate) => candidate.storage_id === result.storage_id)
      ) {
        throw new Error("The storage was verified and saved, but its configuration could not be refreshed.");
      }
      adoptConfigSnapshot(saved);
      const storageName = saved.storages
        .find((candidate) => candidate.storage_id === result.storage_id)?.name || "Storage";
      notify(`${result.replaced ? "Updated" : "Connected"} ${storageName}.`);
      await refreshStorageSetupDrafts(undefined, workflowEpoch);
      return hostWorkflowEpoch.current === workflowEpoch ? result : null;
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
      return null;
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  /** Rename a settled storage in place. Only connection settings go through
      a draft; the name is one focused, revision-checked write. */
  const renameStorage = async (storageId: string, name: string): Promise<boolean> => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setBusy(true);
    setError(null);
    try {
      const saved = await projectSyncApi.renameStorage(storageId, name, configRef.current.revision);
      if (hostWorkflowEpoch.current !== workflowEpoch) return false;
      adoptConfigSnapshot(saved);
      notify(`Storage renamed to “${name}”.`);
      return true;
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
      return false;
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const clearStorageData = async (storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const storage = configRef.current.storages.find((candidate) => candidate.storage_id === storageId);
    if (!storage) {
      setError("The selected storage no longer exists.");
      return;
    }
    const expectedConfigRevision = configRef.current.revision;
    const storageName = storage.name || "Storage";
    const approved = await confirm(
      `Permanently clear all Mallard data from “${storageName}”?\n\nThis deletes every synced project repository and Library resource in this storage. It does not delete unrelated bucket/folder contents, local project files, provider sessions, or the storage connection. This cannot be undone.\n\nStop sync on all machines before you continue. By choosing “Sync stopped — clear”, you confirm that sync has stopped on all machines.`,
      { title: "Clear storage data", okLabel: "Sync stopped — clear", cancelLabel: "Cancel" },
    );
    if (!approved || hostWorkflowEpoch.current !== workflowEpoch) return;

    setBusy(true);
    setError(null);
    try {
      const result = await projectSyncApi.clearStorageData(
        storageId,
        expectedConfigRevision,
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
      notify(
        `${storageName} cleared — ${deleted} Mallard object${deleted === 1 ? "" : "s"} deleted; storage settings and unrelated data kept.`,
        "ok",
      );
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  const removeStorage = async (storageId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    const storage = config.storages.find((candidate) => candidate.storage_id === storageId);
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
    setBusy(true);
    setError(null);
    try {
      const saved = await projectSyncApi.removeStorage(storageId, configRef.current.revision);
      if (hostWorkflowEpoch.current !== workflowEpoch) return;
      adoptConfigSnapshot(saved);
      notify(`${storageName} removed — stored files were not deleted.`);
      setStorageEditorRequest((current) => ({
        mode: "close",
        requestId: (current?.requestId ?? 0) + 1,
      }));
      // Removing the storage whose page you are on leaves nothing to stand
      // on; the trail's root is the natural place to land.
      if (activeStorageSettingsId === storageId) setStorageIndexOpen(true);
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
  };

  /** Add storage starts with a provider choice. The picker is its own
      screen, rooted at the Storages crumb like every storage page; the
      card's type switch can still change the choice afterwards. */
  const openNewStorageConfiguration = async () => {
    setError(null);
    if (!(await closeWorkspaceSettings())) return;
    setSetupDraftId(null);
    setStorageProviderPickerOpen(true);
  };

  /** A door names its provider, so its draft is created straight away; the
      bare verb goes through the picker. */
  const beginAddStorage = (provider?: StorageProviderKey) => {
    if (provider) void createStorageDraft(provider);
    else openNewStorageConfiguration();
  };

  /** Open a storage draft in the New-storage form (or, for an edit draft,
      on its storage's page). Every draft comes through this one door. */
  const openStorageDraftForm = (draft: StorageSetupDraft) => {
    closeProjectSubview();
    setSetupDraftId(null);
    setAddingCatalogScope(false);
    setCatalogIndexOpen(false);
    setCatalogScopeId(null);
    setStorageIndexOpen(false);
    setStorageProviderPickerOpen(false);
    setSettingsOpen(false);
    setPendingPush(null);
    setPushError(null);
    if (restorePlan) closePullReview();
    setStorageEditorRequest((current) => ({
      mode: "resume-draft",
      draft,
      requestId: (current?.requestId ?? 0) + 1,
    }));
  };

  /** The chosen provider mints the draft; the backend binds it to that
      provider for good, and the form opens on it. */
  const createStorageDraft = async (provider: StorageProviderKey, name?: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    if (!(await saveStorageDraftBeforeNavigation())) return;
    setBusy(true);
    setError(null);
    let draft: StorageSetupDraft | null = null;
    try {
      draft = await storageDraftSaveQueue.enqueue(
        () => projectSyncApi.createStorageSetupDraft(provider),
      );
      // A name typed before a type switch carries over to the new draft.
      if (name?.trim()) {
        const created = draft;
        draft = await storageDraftSaveQueue.enqueue(() => projectSyncApi.updateStorageSetupDraft({
          draft_id: created.draft_id,
          revision: created.revision,
          name,
          settings: created.settings,
          secrets: {},
        }));
      }
    } catch (reason) {
      if (hostWorkflowEpoch.current === workflowEpoch) setError(errorMessage(reason));
    } finally {
      if (hostWorkflowEpoch.current === workflowEpoch) setBusy(false);
    }
    if (!draft || hostWorkflowEpoch.current !== workflowEpoch) return;
    await refreshStorageSetupDrafts(undefined, workflowEpoch);
    if (hostWorkflowEpoch.current !== workflowEpoch) return;
    openStorageDraftForm(draft);
  };

  /** Reopen a persisted storage setup draft. The draft is fetched here so
      the workspace's request handler stays synchronous, like every other
      editor request. */
  const openStorageSetupDraft = async (draftId: string) => {
    const workflowEpoch = hostWorkflowEpoch.current;
    setError(null);
    if (!(await saveStorageDraftBeforeNavigation())) return;
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
    openStorageDraftForm(draft);
  };

  const safePushSelection = pendingPush
    ? sanitizePushSelection(
      pushSelected,
      pushResources(pendingPush),
      pendingPush.threadComparison,
      pendingPush.projectContentInventory,
    )
    : pushSelected;
  /* One rule for both buttons that publish this review — the panel's own
     footer Push and the path bar's, which continues the open review rather
     than opening one. They were two rules once, and the bar's knew nothing
     about the capture budget or the deletion acknowledgement, so a review the
     footer refused could still be pushed from above it. */
  const pendingPushDisabledReason = pendingPush
    ? pushReviewDisabledReason({
      selected: safePushSelection,
      threadComparison: pendingPush.threadComparison,
      projectContentInventory: pendingPush.projectContentInventory,
      acknowledgedWarningDigests: pushWarningAcknowledgements,
      projectContentRemovals: pushProjectContentRemovals,
      deletionApproved: pushDeletionApproved,
      projectContentLoading,
    })
    : null;
  const inlineStorageReview = pendingPush ? {
    kind: "push" as const,
    projectId: pendingPush.projectId,
    storageId: pendingPush.storageId,
    onContinue: () => {
      void publishPendingPush();
    },
    continueDisabled: pendingPushDisabledReason !== null,
    /* Pull already said why its button was withheld; Push said nothing, so a
       refusal in the bar had no explanation anywhere near it. */
    continueTitle: pendingPushDisabledReason,
    onClose: () => {
      if (busy) return;
      setPendingPush(null);
      setPushError(null);
      setPushProjectContentRemovals(new Set());
      setPushWarningAcknowledgements(new Set());
    },
    content: (
      <PushResourceWorkspace
        credentialRedactionEnabled={pendingPush.credentialRedactionEnabled}
        resources={pushResources(pendingPush)}
        selected={pushSelected}
        projectDefaults={pendingPush.projectDefaults}
        busy={busy || projectContentLoading}
        error={pushError}
        threadComparisonError={pendingPush.threadComparisonError}
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
        deletionApproved={pushDeletionApproved}
        onDeletionApprovedChange={setPushDeletionApproved}
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
        storageName={config.storages.find((storage) => storage.storage_id === restorePlan.storage_id)?.name ?? null}
        projectName={restoreProjectName}
        plan={restorePlan}
        binding={restoreBinding}
        phase={pullApplyPhase}
        supportLoading={reviewSupportLoading}
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
        {/* No role on the screen. Wrapping the whole page in role="alert" made
            a screen reader read heading, prose, callout and Retry as one
            interruption; the heading and the labelled <main> name it. The
            announcement rides the notice below instead. */}
        <section className="sync-empty-state">
          <div className="sync-empty-illustration" aria-hidden="true">
            <Icon name="alert-triangle" size={24} />
          </div>
          <h1 id="sync-host-recovery-title">Host identity unavailable</h1>
          <p>Mallard could not verify this installation, so sync remains paused.</p>
          {/* `blocking`, not `inline`: this screen replaces the startup loader
              after mount, so the failure is an event the page did not render
              with. Without a live region a screen-reader user is never told
              the app failed to start and has to find the changed heading by
              hand. The alert is scoped to the one sentence saying what went
              wrong, not to the page. */}
          <Notice severity="danger" timing="blocking">{hostIdentityError}</Notice>
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
  /* A modal makes the shell inert. Keep its action failure inside the dialog
     and suppress the inaccessible copy behind it. */
  const dialogOwnsError = Boolean(
    editingBinding
    || (pendingBundleConnection && pendingConnectionProject && pendingConnectionStorage),
  );
  const shellError = dialogOwnsError ? null : error;
  if (shellError) {
    shellNotices.push({
      key: "shell-error",
      severity: "danger",
      message: shellError,
      /* Shell-owned failures stay visible on every non-modal screen,
         including failures from destructive actions. */
      action: (
        <>
          {catalogRemovalErrorTarget?.message === shellError && (
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
  /* Every rung of this zone is an event. `shellError` reports that what the
     user just did did not happen. `backendError` looks like standing
     content because the startup load writes it, but the same `refresh()` runs
     behind "Reload app data" and the path-bar overflow, so a backend that
     falls over on an explicit refresh would go unannounced if this zone were
     a region — and the shell paints the startup screen until the load
     settles, so even the startup case arrives as a change. A notice is
     advisory. The loudest present event re-times the whole box. */
  const shellNoticeTiming: NoticeTiming = shellError || backendError
    ? { timing: "blocking" }
    : { timing: "progress" };
  /* Keeps `sync-backend-banner` for the shell's own geometry only: the zone is
     a sibling of the top bar and the page, and that class is what gives it
     their width, their frame and the gutter below it. It sits UNDER the top
     bar on every screen — the bar is the window's title bar, and the trail is
     read before the message — so a screen that draws its own path bar has to
     place it: it reaches those screens through `PathBarBelow`. */
  const shellNoticeZone = (
    <NoticeZone
      items={shellNotices}
      subject="messages from Mallard"
      className="sync-backend-banner"
      {...shellNoticeTiming}
    />
  );

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
        storageProviders={storageProviders}
        storageUsage={Object.fromEntries(config.storages.map((storage) => [
          storage.storage_id,
          config.links.filter((link) => link.storage_id === storage.storage_id).length,
        ]))}
        storageIssues={storageIssues}
        activeProjectId={activeProjectId}
        activeStorageId={activeStorageSettingsId}
        catalogScopes={catalogScopes}
        catalogCapabilities={catalogCapabilities}
        activeCatalogScopeId={activeCatalogScope?.scope.scope_id ?? null}
        addingCatalogScope={addingCatalogScope}
        catalogIndexOpen={catalogIndexOpen}
        storageIndexOpen={storageIndexOpen || storageProviderPickerOpen}
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
        onAddStorage={beginAddStorage}
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
        // A pointer drags the seam without focusing it, so no focus ring is
        // left behind; the keyboard still reaches it through its tab stop.
        onMouseDown={(event) => event.preventDefault()}
      >
        <ResizeGrip />
      </div>

      <div className={`sync-workspace${activityOpen ? " logs-open" : ""}${resizingLog ? " resizing-log" : ""}`}>
        {/* The project workspace merges the window chrome into its path bar, so
            there is one row at the top instead of a named strip above an empty
            one. Every other screen still gets a titlebar of its own. */}
        {showsPathBar ? null : (
        <>
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
        {shellNoticeZone}
        </>
        )}

        {/* Screens that draw their own path bar place the zone themselves,
            right under the bar, through the context the bar reads. */}
        <PathBarBelow.Provider value={showsPathBar ? shellNoticeZone : null}>
        {addingCatalogScope ? (
          <AddCatalogScopeWorkspace
            key={catalogAddChoiceKey ?? "any"}
            workflowEpoch={hostWorkflowEpoch.current}
            isWorkflowCurrent={(epoch) => hostWorkflowEpoch.current === epoch}
            busy={busy}
            initialChoiceKey={catalogAddChoiceKey}
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
            capabilities={catalogCapabilities}
            busy={busy}
            appActions={appActions}
            onOpenScope={openCatalogScope}
            onAddScope={beginAddCatalogScope}
          />
        ) : settingsOpen ? (
          <SettingsPage
            appActions={appActions}
            onLogsChanged={() => setLogRefreshEpoch((epoch) => epoch + 1)}
            onDone={() => void closeWorkspaceSettings()}
            doneTarget={activeSummary ? projectLabel(activeSummary) : "Projects"}
          />
        ) : storageProviderPickerOpen ? (
          <StorageProviderPicker
            providers={storageProviders}
            busy={busy}
            appActions={appActions}
            onChoose={(provider) => void createStorageDraft(provider)}
            onCancel={openStorageIndex}
          />
        ) : storageIndexOpen ? (
          <StorageIndexPage
            storages={config.storages}
            links={config.links}
            storageIssues={storageIssues}
            drafts={storageSetupDrafts}
            storageProviders={storageProviders}
            busy={busy}
            appActions={appActions}
            onOpenStorage={(storageId, settingsExpanded) => void openStorageWorkspace(storageId, settingsExpanded)}
            onOpenStorageDraft={(draftId) => void openStorageSetupDraft(draftId)}
            onAddStorage={beginAddStorage}
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
            storageProviders={storageProviders}
            storages={config.storages}
            storageSetupDrafts={storageSetupDrafts}
            links={config.links}
            loading={loading}
            busy={busy}
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
            onRenameProject={(projectId, alias) => renameProject(projectId, alias)}
            onResetProjectMetadata={(projectId) => resetProjectSyncMetadata(projectId)}
            onRefresh={() => void refresh()}
            onAddProject={() => void beginAddProject()}
            onSelectProject={openProject}
            appActions={appActions}
            onAddStorage={openNewStorageConfiguration}
            onCreateStorageDraft={createStorageDraft}
            onOpenStorage={openStorageWorkspace}
            onOpenStorageIndex={openStorageIndex}
            onClearStorage={(storageId) => void clearStorageData(storageId)}
            onRemoveStorage={(storageId) => void removeStorage(storageId)}
            onRenameStorage={renameStorage}
            onCommitStorageDraft={commitStorageDraft}
            onTestStorageDraft={testStorageDraft}
            onStorageDraftsChanged={() => void refreshStorageSetupDrafts()}
            storageDraftSaveQueue={storageDraftSaveQueue}
            storageEditorRequest={storageEditorRequest}
            onStorageEditorRequestHandled={() => setStorageEditorRequest(null)}
            onStorageNavigationGuardChange={setStorageNavigationGuard}
            onStorageEditorChange={setActiveStorageSettingsId}
            onStorageIssueChange={updateStorageIssue}
            historyRefreshEpoch={historyRefreshEpoch}
            storageRefreshEpoch={storageRefreshEpoch}
            inlineStorageReview={inlineStorageReview}
            storageNotice={storageNotice}
            onStorageNoticeAction={(outcome, action) => void runOutcomeAction(outcome, action)}
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
                onFinalized={(detail, workflowEpoch) => (
                  void completeSetup(detail, workflowEpoch)
                )}
              />
            ) : null}
          />
        )}
        </PathBarBelow.Provider>

        <ActivityLogDrawer
          open={activityOpen}
          height={logHeight}
          refreshEpoch={logRefreshEpoch}
          scrollToBottomEpoch={logScrollToBottomEpoch}
          drawerRef={logDrawerRef}
          onStartResize={startLogResize}
          onResetHeight={() => setLogHeight(240)}
          onManage={openSettings}
          onClose={() => {
            activityOpenRef.current = false;
            setActivityOpen(false);
            window.requestAnimationFrame(() => {
              document.getElementById("activity-log-trigger")?.focus();
            });
          }}
          readLiveLines={readLiveLogLines}
          subscribeToLiveLines={subscribeToLiveLogLines}
        />
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
