import { invoke } from "@tauri-apps/api/core";
import type {
  ActivityLogCleanupRequest,
  ActivityLogCleanupResult,
  ActivityLogPage,
  ActivityLogPolicy,
  ActivityLogQuery,
  ActivityLogStats,
  BundlePage,
  BundleReadiness,
  BundleRecipe,
  BundleSnapshotSummary,
  CodexAppRestartResult,
  CodexConversationPathAudit,
  CodexConversationPathRepairResult,
  ConnectProjectBundleRequest,
  ConnectR2StorageRequest,
  ConnectStorageResult,
  ConversationResolutionSelection,
  CreateSetupDraftResult,
  HostIdentityStatus,
  LocalProjectRegistration,
  OriginHostAlias,
  OriginHostLabel,
  ProjectActivity,
  ProjectBinding,
  ProjectDetail,
  ProjectContentInventory,
  ProjectOperationResult,
  ProjectOriginTokens,
  ProjectSessionDetail,
  ProjectSessionTimelinePage,
  ProjectSessionTimelineQuery,
  ProjectSetupDraft,
  ProjectStorageLink,
  ProviderDescriptor,
  ProviderKey,
  ProviderProfile,
  ProviderProfileProbe,
  ProviderProfileSummary,
  PushDestinationStatus,
  PushReviewContext,
  RegisterLocalProjectRequest,
  RegisterNewHostRequest,
  ResourceInventory,
  RestorePlan,
  RestoreResult,
  SaveProjectBindingRequest,
  SaveOriginHostAliasRequest,
  SaveProjectLinkRequest,
  SetupDraftInspection,
  SetupDraftList,
  SessionEventDetail,
  SyncConfig,
  ThreadSyncComparison,
  UpdateHostDisplayNameRequest,
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

/**
 * Current Tauri contract. Multi-word Rust parameters use Tauri's camel-case
 * JavaScript names, matching the existing command surface in App.tsx.
 */
export const projectSyncApi = {
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

  listAgentProviders: () =>
    invoke<ProviderDescriptor[]>("list_agent_providers"),

  listProviderProfiles: () =>
    invoke<ProviderProfileSummary[]>("list_provider_profiles"),

  probeProviderProfile: (provider: ProviderKey, path: string) =>
    invoke<ProviderProfileProbe>("probe_provider_profile", { provider, path }),

  createProviderProfile: (provider: ProviderKey, displayName: string, path: string) =>
    invoke<ProviderProfile>("create_provider_profile", { provider, displayName, path }),

  renameProviderProfile: (profileId: string, displayName: string, expectedRevision: number) =>
    invoke<ProviderProfile>("rename_provider_profile", { profileId, displayName, expectedRevision }),

  removeProviderProfile: (profileId: string, expectedRevision: number) =>
    invoke<boolean>("remove_provider_profile", { profileId, expectedRevision }),

  listProjects: () =>
    invoke<LocalProjectRegistration[]>("list_local_projects"),

  /** Heal interrupted setup and conversation work; run once per launch. */
  recoverStartupState: () =>
    invoke<void>("recover_startup_state"),

  listProjectRepositoryKinds: () =>
    invoke<Record<string, boolean>>("list_project_repository_kinds"),

  getProject: (localProjectId: string) =>
    invoke<ProjectDetail | null>("get_project", { localProjectId }),

  registerProject: (request: RegisterLocalProjectRequest) =>
    invoke<LocalProjectRegistration>("register_local_project", { request }),

  removeProject: (localProjectId: string) =>
    invoke<boolean>("remove_local_project", { localProjectId }),

  renameProject: (localProjectId: string, localAlias: string | null, expectedRevision: number) =>
    invoke<LocalProjectRegistration>("rename_local_project", { localProjectId, localAlias, expectedRevision }),

  getConfig: () =>
    invoke<SyncConfig>("get_project_sync_config"),

  saveConfig: (config: SyncConfig) =>
    invoke<SyncConfig>("save_project_sync_config", { config }),

  connectR2Storage: (request: ConnectR2StorageRequest) =>
    invoke<ConnectStorageResult>("connect_r2_storage", { request }),

  getMallardCliPath: () =>
    invoke<string>("get_mallard_cli_path"),

  getHostIdentityStatus: () =>
    invoke<HostIdentityStatus>("get_host_identity_status"),

  updateHostDisplayName: (request: UpdateHostDisplayNameRequest) =>
    invoke<HostIdentityStatus>("update_host_display_name", { request }),

  rotateHostIdentity: (request: RegisterNewHostRequest) =>
    invoke<HostIdentityStatus>("rotate_host_identity", { request }),

  getProjectOriginTokens: (localProjectId: string) =>
    invoke<ProjectOriginTokens>("get_project_origin_tokens", { localProjectId }),

  listOriginHostAliases: (bundleId?: string | null) =>
    invoke<OriginHostAlias[]>("list_origin_host_aliases", { bundleId: bundleId ?? null }),

  saveOriginHostAlias: (request: SaveOriginHostAliasRequest) =>
    invoke<OriginHostAlias>("save_origin_host_alias", { request }),

  resolveOriginHostLabel: (bundleId: string, originHostToken: string) =>
    invoke<OriginHostLabel>("resolve_origin_host_label", { bundleId, originHostToken }),

  listRemoteBundles: (storageId: string, cursor?: string | null) =>
    invoke<BundlePage>("list_remote_bundles", { storageId, cursor: cursor ?? null }),

  listRemoteBundleSnapshots: (storageId: string) =>
    invoke<BundleSnapshotSummary[]>("list_remote_bundle_snapshots", { storageId }),

  findRemoteBundleMatches: (storageId: string, repositoryFingerprint: string) =>
    invoke<BundleSnapshotSummary[]>("find_remote_bundle_matches", { storageId, repositoryFingerprint }),

  saveLink: (request: SaveProjectLinkRequest) =>
    invoke<ProjectStorageLink>("save_project_link", { request }),

  connectProjectToRemoteBundle: (request: ConnectProjectBundleRequest) =>
    invoke<ProjectDetail>("connect_project_to_remote_bundle", { request }),

  removeLink: (localProjectId: string, storageId: string) =>
    invoke<boolean>("remove_project_link", { localProjectId, storageId }),

  getInventory: (localProjectId: string) =>
    invoke<ResourceInventory>("get_bundle_inventory", { localProjectId }),

  preparePushReview: (localProjectId: string, storageId: string) =>
    invoke<PushReviewContext>("prepare_push_review", { localProjectId, storageId }),

  getPushDestinationStatus: (localProjectId: string, storageId: string) =>
    invoke<PushDestinationStatus>("get_push_destination_status", { localProjectId, storageId }),

  getSessionSyncComparison: (localProjectId: string, storageId: string) =>
    invoke<ThreadSyncComparison>("get_project_session_sync_comparison", { localProjectId, storageId }),

  inspectProjectFiles: (localProjectId: string, storageId: string) =>
    invoke<ProjectContentInventory>("inspect_project_files", { localProjectId, storageId }),

  pushBundle: (
    localProjectId: string,
    storageId: string,
    recipe: BundleRecipe,
    projectContentReviewToken?: string | null,
    projectContentRemovalIds: string[] = [],
    acknowledgedWarningDigests: string[] = [],
    conversationPushPlanId?: string | null,
    approvedConversationActionIds: string[] = [],
  ) => invoke<ProjectOperationResult>("push_bundle", {
    localProjectId,
    storageId,
    recipe,
    projectContentReviewToken: projectContentReviewToken ?? null,
    projectContentRemovalIds,
    acknowledgedWarningDigests,
    conversationPushPlanId: conversationPushPlanId ?? null,
    approvedConversationActionIds,
  }),

  getBinding: (localProjectId: string) =>
    invoke<ProjectBinding | null>("get_project_binding", { localProjectId }),

  auditCodexConversationPaths: (localProjectId: string) =>
    invoke<CodexConversationPathAudit>("audit_codex_conversation_paths", { localProjectId }),

  repairCodexConversationPaths: (localProjectId: string) =>
    invoke<CodexConversationPathRepairResult>("repair_codex_conversation_paths", { localProjectId }),

  listBindings: () =>
    invoke<ProjectBinding[]>("list_project_bindings"),

  saveBinding: (request: SaveProjectBindingRequest) =>
    invoke<ProjectBinding>("save_project_binding", { request }),

  planRestore: (storageId: string, bundleId: string, binding: ProjectBinding) =>
    invoke<RestorePlan>("plan_bundle_restore", { storageId, bundleId, binding }),

  getRestorePlan: (planId: string) =>
    invoke<RestorePlan>("get_restore_plan", { planId }),

  getPendingCodexDivergence: (localProjectId: string) =>
    invoke<RestoreResult | null>("get_pending_codex_divergence", { localProjectId }),

  applyRestore: (
    planId: string,
    approvedActionIds: string[],
    conversationResolutions: ConversationResolutionSelection[],
  ) => invoke<RestoreResult>("apply_bundle_restore", {
    planId,
    approvedActionIds,
    conversationResolutions,
  }),

  retryCodexDivergence: (operationId: string) =>
    invoke<RestoreResult>("retry_codex_divergence", { operationId }),

  forkCodexThread: (localProjectId: string, providerThreadId: string) =>
    invoke<RestoreResult>("fork_codex_thread", {
      localProjectId,
      providerThreadId,
    }),

  restartCodexAppAfterPull: (
    operationId: string,
    appKind: "codex" | "codex_plus_plus",
    activeChatsComplete: boolean,
  ) => invoke<CodexAppRestartResult>("restart_codex_app_after_pull", {
    request: {
      operation_id: operationId,
      app_kind: appKind,
      active_chats_complete: activeChatsComplete,
    },
  }),

  getReadiness: (storageId: string, bundleId: string, binding: ProjectBinding) =>
    invoke<BundleReadiness>("get_bundle_readiness", { storageId, bundleId, binding }),

  getRestoreReadiness: (restorePlanId: string) =>
    invoke<BundleReadiness>("get_restore_readiness", { restorePlanId }),

  listSetupDrafts: () =>
    invoke<SetupDraftList>("list_setup_drafts"),

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

  getProjectActivity: (
    localProjectId: string,
    storageId?: string | null,
    branch?: string | null,
    beforeTime?: number | null,
    windowDays = 30,
    forceRevalidate = false,
  ) => {
    const request = {
      localProjectId,
      storageId: storageId ?? null,
      branch: branch ?? null,
      beforeTime: beforeTime ?? null,
      windowDays,
      forceRevalidate,
    };
    return projectSyncSingleFlight(
      JSON.stringify(["get_project_activity", request]),
      () => invoke<ProjectActivity>("get_project_activity", request),
    );
  },

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
      anchorEventId: query.anchor_event_id ?? null,
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

  executeProjectSessionAction: (localProjectId: string, resourceId: string, actionId: string) =>
    invoke<void>("execute_project_session_action", { localProjectId, resourceId, actionId }),
};
