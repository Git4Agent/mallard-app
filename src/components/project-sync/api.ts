import type { ManagedCloudDiscardedDraft, ManagedCloudLink, ManagedCloudListingPage, ManagedCloudPublicSort } from "../managed-cloud/types";
import { listen } from "@tauri-apps/api/event";
import { Channel, invoke } from "../../ipc";
import type {
  DropboxSetupMode,
  DropboxSetupLogin,
  DropboxSetupProgress,
  FeishuSetupLogin,
  FeishuSetupMode,
  FeishuSetupProgress,
  CloudProjectFolderProbe,
  CloudProjectFolderProgress,
  CloudProjectFolderRequest,
  CloudProjectFolderResult,
  ManagedCloudRepository,
  ManagedCloudRepositoryResolution,
  ManagedCloudSetupLogin,
  ManagedCloudSetupMode,
  ManagedCloudSetupProgress,
  R2Account,
  R2Jurisdiction,
  R2SetupProgress,
  ActivityLogCleanupRequest,
  AgentFolderList,
  AppLinkAnswer,
  AppLinkArrival,
  AppSettings,
  AppUpdateStatus,
  ActivityLogCleanupResult,
  ActivityLogPage,
  ActivityLogPolicy,
  ActivityLogQuery,
  ActivityLogStats,
  BundleReadiness,
  BundleRecipe,
  BundleSnapshotSummary,
  ConnectProjectBundleRequest,
  CreateSetupDraftResult,
  HostIdentityStatus,
  LocalProjectRegistration,
  LocalSessionDeleteResult,
  LocalStateSummary,
  LocalStateWipeResult,
  MaintenanceActionRequest,
  MaintenanceActionResult,
  ProjectActivity,
  ProjectActivityRefreshEvent,
  ProjectBinding,
  ProjectDetail,
  ProjectFolderProbe,
  ProjectGitChangedEvent,
  ProjectContentInventory,
  ProjectOperationResult,
  ProjectMarkdownImage,
  ProjectSessionDetail,
  ProjectSessionTimelinePage,
  ProjectSessionTimelineQuery,
  ProjectSessionsDeleteResult,
  ProjectSetupDraft,
  ProjectStorageLink,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfile,
  ProviderProfileProbe,
  ProviderProfileSummary,
  PushDestinationStatus,
  PushReviewContext,
  PullScope,
  PushScope,
  ResourceInventory,
  PullReview,
  RestoreResult,
  SaveProjectBindingRequest,
  SaveProjectLinkRequest,
  SetupDraftInspection,
  SetupDraftList,
  SessionEventDetail,
  SiblingBundleStatus,
  CommitStorageDraftResult,
  StorageDiscoveryResult,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSetupDraft,
  TestStorageResult,
  StorageSetupDraftList,
  TestStorageDraftResult,
  UpdateStorageSetupDraftRequest,
  StorageDataClearResult,
  SyncConfig,
} from "../../types";

export function createSingleFlight() {
  const requests = new Map<string, Promise<unknown>>();

  return function singleFlight<T>(key: string, start: () => Promise<T>): Promise<T> {
    const existing = requests.get(key);
    if (existing) return existing as Promise<T>;

    const request = start();
    requests.set(key, request);
    const clear = () => {
      if (requests.get(key) === request) requests.delete(key);
    };
    void request.then(clear, clear);
    return request;
  };
}

const projectSyncSingleFlight = createSingleFlight();

/* The backend watches one project's Git refs. A Push or Pull review can show
   the same project's Activity beside the workspace page, so each page holds
   the watch and only the last release stops it. */
const projectGitWatchHolds = new Map<string, number>();

export interface ActivityRefreshRequest {
  localProjectId: string;
  storageId: string;
  branch: string | null;
  windowDays: number;
  review: boolean;
  bindingRevision: number | null;
  refreshEpoch: number;
}

// React remounts can join a running refresh. Replay its latest stages so each
// caller receives local rows and the final result without a second remote read.
export function createActivityRefresh(
  start: (request: ActivityRefreshRequest, onEvent: (event: ProjectActivityRefreshEvent) => void) => Promise<void>,
) {
  const pending = new Map<string, {
    events: Map<ProjectActivityRefreshEvent["stage"], ProjectActivityRefreshEvent>;
    listeners: Set<(event: ProjectActivityRefreshEvent) => void>;
    promise: Promise<void>;
    request: ActivityRefreshRequest;
  }>();
  const refresh = (request: ActivityRefreshRequest, onEvent: (event: ProjectActivityRefreshEvent) => void) => {
    const key = JSON.stringify(request);
    const existing = pending.get(key);
    if (existing) {
      existing.listeners.add(onEvent);
      existing.events.forEach(onEvent);
      return existing.promise;
    }
    const events = new Map<ProjectActivityRefreshEvent["stage"], ProjectActivityRefreshEvent>();
    const listeners = new Set([onEvent]);
    const promise = Promise.resolve().then(() => start(request, (event) => {
      // An error replaces its result, and a recovered result replaces its error.
      if (event.stage === "activity") events.delete("activity_error");
      if (event.stage === "activity_error") events.delete("activity");
      if (event.stage === "comparison") events.delete("comparison_error");
      if (event.stage === "comparison_error") events.delete("comparison");
      events.set(event.stage, event);
      listeners.forEach((listener) => listener(event));
    })).finally(() => {
      if (pending.get(key)?.promise === promise) pending.delete(key);
    });
    pending.set(key, { events, listeners, promise, request });
    return promise;
  };
  return Object.assign(refresh, {
    invalidate(localProjectId: string, storageId: string | null) {
      for (const [key, entry] of pending) {
        if (entry.request.localProjectId === localProjectId && entry.request.storageId === storageId) {
          entry.listeners.clear();
          pending.delete(key);
        }
      }
    },
  });
}

const refreshProjectActivity = createActivityRefresh((request, onEvent) => {
  const onProgress = new Channel<ProjectActivityRefreshEvent>();
  onProgress.onmessage = onEvent;
  const { localProjectId, storageId, branch, windowDays, review } = request;
  return invoke<void>("refresh_project_activity", { localProjectId, storageId, branch, windowDays, review, onProgress });
});

/**
 * Current Tauri contract. Multi-word Rust parameters use Tauri's camel-case
 * JavaScript names, matching the existing command surface in App.tsx.
 */
function r2ProgressChannel(onProgress?: (progress: R2SetupProgress) => void): Channel<R2SetupProgress> {
  const channel = new Channel<R2SetupProgress>();
  if (onProgress) channel.onmessage = onProgress;
  return channel;
}

export const projectSyncApi = {
  invalidateProjectActivity: refreshProjectActivity.invalidate,
  queryActivityLogs: (query: ActivityLogQuery) =>
    invoke<ActivityLogPage>("query_activity_logs", { query }),

  getActivityLogStats: () =>
    invoke<ActivityLogStats>("get_activity_log_stats"),

  updateActivityLogPolicy: (policy: ActivityLogPolicy) =>
    invoke<ActivityLogStats>("update_activity_log_policy", { policy }),

  cleanupActivityLogs: (request: ActivityLogCleanupRequest) =>
    invoke<ActivityLogCleanupResult>("cleanup_activity_logs", { request }),

  getActivityLogFolder: () =>
    invoke<string>("get_activity_log_folder"),

  getLocalStateSummary: () =>
    invoke<LocalStateSummary>("get_local_state_summary"),

  resetLocalState: () =>
    invoke<LocalStateWipeResult>("reset_local_state"),

  getAppSettings: () =>
    invoke<AppSettings>("get_app_settings"),

  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { settings }),

  prepareAppUpdate: () =>
    invoke<AppUpdateStatus>("prepare_app_update"),

  restartToUpdate: () =>
    invoke<void>("restart_to_update"),

  listAgentProviders: () =>
    invoke<ProviderDescriptor[]>("list_agent_providers"),

  listProviderProfiles: () =>
    invoke<ProviderProfileSummary[]>("list_provider_profiles"),

  probeProviderProfile: (provider: ProviderKey, path: string) =>
    invoke<ProviderProfileProbe>("probe_provider_profile", { provider, path }),

  createProviderProfile: (provider: ProviderKey, displayName: string, path: string) =>
    invoke<ProviderProfile>("create_provider_profile", { provider, displayName, path }),

  listProjects: () =>
    invoke<LocalProjectRegistration[]>("list_local_projects"),

  /** Heal interrupted setup and conversation work; run once per launch. */
  recoverStartupState: () =>
    invoke<void>("recover_startup_state"),

  listProjectRepositoryKinds: () =>
    invoke<Record<string, boolean>>("list_project_repository_kinds"),

  getProject: (localProjectId: string) =>
    invoke<ProjectDetail | null>("get_project", { localProjectId }),

  removeProject: (localProjectId: string) =>
    invoke<boolean>("remove_local_project", { localProjectId }),

  deleteLocalProjectSessions: (localProjectId: string, expectedProjectRevision: number) =>
    invoke<ProjectSessionsDeleteResult>("delete_local_project_sessions", {
      localProjectId,
      expectedProjectRevision,
    }),

  /** Finish that push: write the record an earlier push claimed and never
   *  wrote, once. */
  finishUnfinishedPush: (localProjectId: string, storageId: string) =>
    invoke<void>("finish_unfinished_push", { localProjectId, storageId }),

  clearStorageData: (storageId: string, expectedConfigRevision: number) =>
    invoke<StorageDataClearResult>("clear_storage_data", {
      storageId,
      expectedConfigRevision,
    }),

  renameProject: (localProjectId: string, localAlias: string | null, expectedRevision: number) =>
    invoke<LocalProjectRegistration>("rename_local_project", { localProjectId, localAlias, expectedRevision }),

  getConfig: () =>
    invoke<SyncConfig>("get_project_sync_config"),

  /** Every compiled storage provider's safe descriptor, in display order. */
  listStorageProviders: () =>
    invoke<StorageProviderDescriptor[]>("list_storage_providers"),

  renameStorage: (storageId: string, name: string, expectedConfigRevision: number) =>
    invoke<SyncConfig>("rename_storage", { storageId, name, expectedConfigRevision }),

  /** One saved secret, on Show or Copy. The backend logs the reveal. */
  revealStorageSecret: (storageId: string, secretId: string) =>
    invoke<string>("reveal_storage_secret", { storageId, secretId }),

  /** Probe a saved storage as configured, without a draft. */
  testStorage: (storageId: string) =>
    invoke<TestStorageResult>("test_storage", { storageId }),

  removeStorage: (storageId: string, expectedConfigRevision: number) =>
    invoke<SyncConfig>("remove_storage", { storageId, expectedConfigRevision }),

  /** The R2 hand-off through local Wrangler, one helper run per step. Every
   *  run reports progress on the channel as it happens. */
  getR2SetupAccounts: (draftId: string, expectedRevision: number, reauthenticate = false, onProgress?: (progress: R2SetupProgress) => void) =>
    invoke<R2Account[]>("get_r2_setup_accounts", { draftId, expectedRevision, reauthenticate, onProgress: r2ProgressChannel(onProgress) }),
  /** A readable name for the Bucket line to propose; free or not is only
   *  known at creation. */
  proposeR2BucketName: () => invoke<string>("propose_r2_bucket_name"),
  setupR2Storage: (draftId: string, expectedRevision: number, accountId: string, jurisdiction: R2Jurisdiction, bucket: string, onProgress?: (progress: R2SetupProgress) => void) =>
    invoke<void>("setup_r2_storage", { draftId, expectedRevision, accountId, jurisdiction, bucket, onProgress: r2ProgressChannel(onProgress) }),
  discoverR2Storage: (draftId: string, expectedRevision: number, accountId: string, onProgress?: (progress: R2SetupProgress) => void) =>
    invoke<StorageDiscoveryResult>("discover_r2_storage", { draftId, expectedRevision, accountId, onProgress: r2ProgressChannel(onProgress) }),
  prepareExistingR2Storage: (draftId: string, expectedRevision: number, scanId: string, candidateId: string, onProgress?: (progress: R2SetupProgress) => void) =>
    invoke<void>("prepare_existing_r2_storage", { draftId, expectedRevision, scanId, candidateId, onProgress: r2ProgressChannel(onProgress) }),
  cancelR2StorageSetup: (draftId: string, expectedRevision: number) =>
    invoke<void>("cancel_r2_storage_setup", { draftId, expectedRevision }),

  authorizeDropboxSetup: (draftId: string, expectedRevision: number, mode: DropboxSetupMode, switchAccount: boolean, onProgress: (progress: DropboxSetupProgress) => void) => {
    const channel = new Channel<DropboxSetupProgress>();
    channel.onmessage = onProgress;
    return invoke<DropboxSetupLogin>("authorize_dropbox_setup", { draftId, expectedRevision, mode, switchAccount, onProgress: channel });
  },
  proposeDropboxFolderName: () => invoke<string>("propose_dropbox_folder_name"),
  discoverDropboxStorage: (draftId: string, expectedRevision: number, sessionId: string) =>
    invoke<StorageDiscoveryResult>("discover_dropbox_storage", { draftId, expectedRevision, sessionId }),
  prepareDropboxStorage: (draftId: string, expectedRevision: number, sessionId: string, folderName: string | null, scanId: string | null, candidateId: string | null) =>
    invoke<void>("prepare_dropbox_storage", { draftId, expectedRevision, sessionId, folderName, scanId, candidateId }),
  cancelDropboxSetup: (draftId: string, expectedRevision: number) =>
    invoke<void>("cancel_dropbox_setup", { draftId, expectedRevision }),
  authorizeFeishuSetup: (draftId: string, expectedRevision: number, mode: FeishuSetupMode, onProgress: (progress: FeishuSetupProgress) => void) => {
    const channel = new Channel<FeishuSetupProgress>();
    channel.onmessage = onProgress;
    return invoke<FeishuSetupLogin>("authorize_feishu_setup", { draftId, expectedRevision, mode, onProgress: channel });
  },
  proposeFeishuFolderName: () => invoke<string>("propose_feishu_folder_name"),
  prepareFeishuStorage: (draftId: string, expectedRevision: number, sessionId: string, folderName: string | null, folderLink: string | null) =>
    invoke<void>("prepare_feishu_storage", { draftId, expectedRevision, sessionId, folderName, folderLink }),
  cancelFeishuSetup: (draftId: string, expectedRevision: number) =>
    invoke<void>("cancel_feishu_setup", { draftId, expectedRevision }),

  /** The Mallard Cloud sign-in: the link arrives on the channel, the
   *  callback lands on a loopback port, and the account and its token are
   *  written into the exact draft revision before the call returns. */
  signInManagedCloudStorage: (draftId: string, expectedRevision: number, mode: ManagedCloudSetupMode, onProgress: (progress: ManagedCloudSetupProgress) => void) => {
    const channel = new Channel<ManagedCloudSetupProgress>();
    channel.onmessage = onProgress;
    return invoke<ManagedCloudSetupLogin>("sign_in_managed_cloud_storage", { draftId, expectedRevision, mode, onProgress: channel });
  },
  cancelManagedCloudSetup: (draftId: string, expectedRevision: number) =>
    invoke<void>("cancel_managed_cloud_setup", { draftId, expectedRevision }),
  /** The website page and release state of a project on a Mallard Cloud
   *  storage; null when the storage is not Mallard Cloud or nothing was
   *  pushed yet. */
  requestManagedCloudRelease: (localProjectId: string, storageId: string, expectedRepositoryRevision: number) =>
    invoke<ManagedCloudLink>("request_managed_cloud_release", { localProjectId, storageId, expectedRepositoryRevision }),
  /** The Owner's discard of a draft over a public version: every push after
   *  the public version is removed from Mallard Cloud. One request; nothing
   *  on this computer is read, written or deleted. */
  discardManagedCloudDraft: (localProjectId: string, storageId: string, expectedRepositoryRevision: number) =>
    invoke<ManagedCloudDiscardedDraft>("discard_managed_cloud_draft", { localProjectId, storageId, expectedRepositoryRevision }),
  /** The project repositories the account belongs to on a Mallard Cloud
   *  storage, each compared with the local project's Git remote. */
  listManagedCloudRepositories: (storageId: string, localRepositoryFingerprint: string | null) =>
    invoke<ManagedCloudRepository[]>("list_managed_cloud_repositories", { storageId, localRepositoryFingerprint }),
  /** Starred: the project repositories the account starred on the
   *  website, one page. A row is picked by resolving its address. */
  listManagedCloudStarred: (storageId: string, cursor: string | null) =>
    invoke<ManagedCloudListingPage>("list_managed_cloud_starred", { storageId, cursor }),
  /** Public: the released projects, newest first or by stars, one page. */
  listManagedCloudPublic: (storageId: string, sort: ManagedCloudPublicSort, cursor: string | null) =>
    invoke<ManagedCloudListingPage>("list_managed_cloud_public", { storageId, sort, cursor }),
  /** One pasted address resolved against a Mallard Cloud storage. */
  resolveManagedCloudRepository: (storageId: string, address: string, localRepositoryFingerprint: string | null) =>
    invoke<ManagedCloudRepositoryResolution>("resolve_managed_cloud_repository", { storageId, address, localRepositoryFingerprint }),
  /** One invitation code entered on a Mallard Cloud storage: the account
   *  joins the private repository it belongs to, which comes back the way
   *  a resolved address does. */
  acceptManagedCloudInvitation: (storageId: string, code: string, localRepositoryFingerprint: string | null) =>
    invoke<ManagedCloudRepositoryResolution>("accept_managed_cloud_invitation", { storageId, code, localRepositoryFingerprint }),
  /** The `mallard://` link that is waiting, once: at startup and whenever
   *  `onAppLink` fires. Null when none is. */
  takeAppLink: () => invoke<AppLinkArrival | null>("take_app_link"),
  /** A link arrived; the handler takes it. */
  onAppLink: (handler: () => void) => listen("app-link", () => handler()),
  /** What each Mallard Cloud storage on the link's host sees at its
   *  address. Reads only. */
  resolveAppLink: (host: string, address: string) =>
    invoke<AppLinkAnswer>("resolve_app_link", { host, address }),
  /** Where a project got from Mallard Cloud may put its folder. Reads only. */
  probeCloudProjectFolder: (destination: string, repositoryFingerprint: string | null) =>
    invoke<CloudProjectFolderProbe>("probe_cloud_project_folder", { destination, repositoryFingerprint }),
  /** Makes that folder: a clone (gh, then git) when one works, else empty. */
  getCloudProjectFolder: (request: CloudProjectFolderRequest, onProgress: (progress: CloudProjectFolderProgress) => void) => {
    const channel = new Channel<CloudProjectFolderProgress>();
    channel.onmessage = onProgress;
    return invoke<CloudProjectFolderResult>("get_cloud_project_folder", { request, onProgress: channel });
  },
  cancelCloudProjectFolder: (operationId: string) =>
    invoke<void>("cancel_cloud_project_folder", { operationId }),

  getHostIdentityStatus: () =>
    invoke<HostIdentityStatus>("get_host_identity_status"),

  listRemoteBundleSnapshots: (storageId: string) =>
    invoke<BundleSnapshotSummary[]>("list_remote_bundle_snapshots", { storageId }),

  getSiblingBundleStatus: (localProjectId: string, storageId: string) =>
    invoke<SiblingBundleStatus>("get_sibling_bundle_status", { localProjectId, storageId }),

  saveLink: (request: SaveProjectLinkRequest) =>
    invoke<ProjectStorageLink>("save_project_link", { request }),

  connectProjectToRemoteBundle: (request: ConnectProjectBundleRequest) =>
    invoke<ProjectDetail>("connect_project_to_remote_bundle", { request }),

  removeLink: (localProjectId: string, storageId: string) =>
    invoke<boolean>("remove_project_link", { localProjectId, storageId }),

  getInventory: (localProjectId: string) =>
    invoke<ResourceInventory>("get_bundle_inventory", { localProjectId }),

  preparePushReview: (localProjectId: string, storageId: string, scope: PushScope) =>
    invoke<PushReviewContext>("prepare_push_review", { localProjectId, storageId, scope }),

  getPushDestinationStatus: (localProjectId: string, storageId: string) =>
    invoke<PushDestinationStatus>("get_push_destination_status", { localProjectId, storageId }),

  inspectProjectFiles: (localProjectId: string, storageId: string) => {
    const request = { localProjectId, storageId };
    return projectSyncSingleFlight(
      JSON.stringify(["inspect_project_files", request]),
      () => invoke<ProjectContentInventory>("inspect_project_files", request),
    );
  },

  pushBundle: (
    localProjectId: string,
    storageId: string,
    scope: PushScope,
    recipe: BundleRecipe,
    credentialRedactionEnabled: boolean,
    projectContentReviewToken?: string | null,
    /** The Files step was on screen with its scanned rows: a file left
     *  unticked there is remembered as left out. */
    projectFilesShown = false,
    projectContentRemovalIds: string[] = [],
    acknowledgedWarningDigests: string[] = [],
    subagentSelections: Record<string, import("../../types").SubagentSelection> = {},
    gitHistoryReviewToken: string | null = null,
  ) => invoke<ProjectOperationResult>("push_bundle", {
    localProjectId,
    storageId,
    scope,
    recipe,
    credentialRedactionEnabled,
    projectContentReviewToken: projectContentReviewToken ?? null,
    projectFilesShown,
    projectContentRemovalIds,
    acknowledgedWarningDigests,
    subagentSelections,
    gitHistoryReviewToken,
  }),

  /** Run one declared action. The backend refuses an action the provider
   *  does not declare and a confirming action the user has not confirmed. */
  executeProviderMaintenanceAction: (request: MaintenanceActionRequest) =>
    invoke<MaintenanceActionResult>("execute_provider_maintenance_action", { request }),

  listBindings: () =>
    invoke<ProjectBinding[]>("list_project_bindings"),

  saveBinding: (request: SaveProjectBindingRequest) =>
    invoke<ProjectBinding>("save_project_binding", { request }),

  planRestore: (storageId: string, bundleId: string, binding: ProjectBinding, scope: PullScope) =>
    invoke<PullReview>("plan_bundle_restore", { storageId, bundleId, binding, scope }),

  /** Where the session a stored row names is in a project: the id its page
   *  is opened by, or null when the project does not hold it. Reads only. */
  findProjectSession: (localProjectId: string, resourceId: string) =>
    invoke<string | null>("find_project_session", { localProjectId, resourceId }),

  applyRestore: (
    review: PullReview,
    approvedActionIds: string[],
  ) => invoke<RestoreResult>("apply_bundle_restore", {
    review,
    approvedActionIds,
  }),

  getRestoreReadiness: (localProjectId: string, storageId: string) =>
    invoke<BundleReadiness>("get_restore_readiness", { localProjectId, storageId }),

  listSetupDrafts: () =>
    invoke<SetupDraftList>("list_setup_drafts"),

  probeProjectFolder: (projectRoot: string) =>
    invoke<ProjectFolderProbe>("probe_project_folder", { projectRoot }),

  listAgentFolders: () =>
    invoke<AgentFolderList>("list_agent_folders"),

  createSetupDraft: (projectRoot: string) =>
    invoke<CreateSetupDraftResult>("create_setup_draft", { projectRoot }),

  getSetupDraft: (draftId: string) =>
    invoke<ProjectSetupDraft | null>("get_setup_draft", { draftId }),

  updateSetupDraft: (draft: ProjectSetupDraft) =>
    invoke<ProjectSetupDraft>("update_setup_draft", { draft }),

  discardSetupDraft: (draftId: string) =>
    invoke<boolean>("discard_setup_draft", { draftId }),

  inspectSetupDraft: (draftId: string) =>
    invoke<SetupDraftInspection>("inspect_setup_draft", { draftId }),

  finalizeProjectSetup: (draftId: string, expectedRevision: number) =>
    invoke<ProjectDetail>("finalize_project_setup", { draftId, expectedRevision }),

  listStorageSetupDrafts: () =>
    invoke<StorageSetupDraftList>("list_storage_setup_drafts"),

  createStorageSetupDraft: (provider: StorageProviderKey) =>
    invoke<StorageSetupDraft>("create_storage_setup_draft", { provider }),

  createStorageEditDraft: (storageId: string) =>
    invoke<StorageSetupDraft>("create_storage_edit_draft", { storageId }),

  getStorageSetupDraft: (draftId: string) =>
    invoke<StorageSetupDraft | null>("get_storage_setup_draft", { draftId }),

  updateStorageSetupDraft: (request: UpdateStorageSetupDraftRequest) =>
    invoke<StorageSetupDraft>("update_storage_setup_draft", { request }),

  discardStorageSetupDraft: (draftId: string) =>
    invoke<boolean>("discard_storage_setup_draft", { draftId }),

  testStorageSetupDraft: (draftId: string, expectedRevision: number) =>
    invoke<TestStorageDraftResult>("test_storage_setup_draft", { draftId, expectedRevision }),



  commitStorageSetupDraft: (
    draftId: string,
    expectedDraftRevision: number,
  ) => invoke<CommitStorageDraftResult>("commit_storage_setup_draft", {
    draftId,
    expectedDraftRevision,
  }),

  getProjectActivity: (
    localProjectId: string,
    storageId?: string | null,
    branch?: string | null,
    beforeTime?: number | null,
    windowDays = 30,
    review = false,
    fresh = false,
  ) => {
    const request = {
      localProjectId,
      storageId: storageId ?? null,
      branch: branch ?? null,
      beforeTime: beforeTime ?? null,
      windowDays,
      review,
    };
    // A reload for a ref change must not join a read that began before it.
    if (fresh) return invoke<ProjectActivity>("get_project_activity", request);
    return projectSyncSingleFlight(
      JSON.stringify(["get_project_activity", request]),
      () => invoke<ProjectActivity>("get_project_activity", request),
    );
  },

  refreshProjectActivity,

  /** Hold the Git watch on a project while a page shows its Activity; the
   *  returned function releases the hold. */
  holdProjectGitWatch: (localProjectId: string) => {
    const holds = projectGitWatchHolds.get(localProjectId) ?? 0;
    projectGitWatchHolds.set(localProjectId, holds + 1);
    if (holds === 0) void invoke<void>("watch_project_git", { localProjectId }).catch(() => undefined);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const remaining = (projectGitWatchHolds.get(localProjectId) ?? 1) - 1;
      if (remaining > 0) {
        projectGitWatchHolds.set(localProjectId, remaining);
        return;
      }
      projectGitWatchHolds.delete(localProjectId);
      void invoke<void>("unwatch_project_git", { localProjectId }).catch(() => undefined);
    };
  },

  onProjectGitChanged: (handler: (event: ProjectGitChangedEvent) => void) =>
    listen<ProjectGitChangedEvent>("project-git-changed", (event) => handler(event.payload)),

  getProjectSessionDetail: (localProjectId: string, resourceId: string) => {
    const request = { localProjectId, resourceId };
    return projectSyncSingleFlight(
      JSON.stringify(["get_project_session_detail", request]),
      () => invoke<ProjectSessionDetail>("get_project_session_detail", request),
    );
  },

  getProjectSessionTimelinePage: (
    localProjectId: string,
    resourceId: string,
    query: ProjectSessionTimelineQuery,
  ) => {
    const request = {
      localProjectId,
      resourceId,
      snapshotId: query.snapshot_id,
      cursor: query.cursor ?? null,
      limit: query.limit,
      includedEventKinds: query.included_event_kinds,
      includedToolCategories: query.included_tool_categories,
      turnId: query.turn_id ?? null,
    };
    return projectSyncSingleFlight(
      JSON.stringify(["get_project_session_timeline_page", request]),
      () => invoke<ProjectSessionTimelinePage>("get_project_session_timeline_page", request),
    );
  },

  getProjectSessionEventDetail: (
    localProjectId: string,
    resourceId: string,
    snapshotId: string,
    eventId: string,
  ) => {
    const request = { localProjectId, resourceId, snapshotId, eventId };
    return projectSyncSingleFlight(
      JSON.stringify(["get_project_session_event_detail", request]),
      () => invoke<SessionEventDetail>("get_project_session_event_detail", request),
    );
  },

  openProjectMarkdownPath: (localProjectId: string, relativePath: string) =>
    invoke<void>("open_project_markdown_path", { localProjectId, relativePath }),

  readProjectMarkdownImage: (localProjectId: string, relativePath: string) =>
    invoke<ProjectMarkdownImage>("read_project_markdown_image", { localProjectId, relativePath }),

  deleteLocalProjectSession: (localProjectId: string, resourceId: string) =>
    invoke<LocalSessionDeleteResult>("delete_local_project_session", { localProjectId, resourceId }),
  executeProjectSessionAction: (localProjectId: string, resourceId: string, actionId: string) =>
    invoke<void>("execute_project_session_action", { localProjectId, resourceId, actionId }),
};
