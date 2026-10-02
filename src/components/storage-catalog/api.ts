import type { ManagedCloudStatus, ManagedCloudDiscardedDraft } from "../managed-cloud/types";
import type { CatalogSyncProgress, CatalogCloudRepository, CatalogCloudResolution } from "./types";
import { Channel, invoke } from "../../ipc";
import type {
  CatalogActionResult,
  CatalogCapabilityDescriptor,
  CatalogCoordinate,
  CatalogProfile,
  CatalogProfileInspection,
  CatalogProfileProbe,
  CatalogResourceForkPlan,
  CatalogScopeDto,
  CatalogScopeLinkOutcome,
  CatalogScopePullOutcome,
  CatalogScopePullPlan,
  CatalogScopePushOutcome,
  CatalogScopePushPlan,
  CatalogScopeResources,
  CatalogStorageRepository,
} from "./types";

function progressChannel(receive: (progress: CatalogSyncProgress) => void) {
  const channel = new Channel<CatalogSyncProgress>();
  channel.onmessage = receive;
  return channel;
}

export const storageCatalogApi = {
  /** The repositories this account belongs to that hold one item. */
  listCloudRepositories: (scopeId: string, storageId: string, resourceId: string) =>
    invoke<CatalogCloudRepository[]>("list_catalog_cloud_repositories", { scopeId, storageId, resourceId }),
  /** Someone else's item repositories this account belongs to that no row
   *  has saved, of every coordinate this build opens. */
  listCloudShared: (storageId: string) =>
    invoke<CatalogCloudRepository[]>("list_catalog_cloud_shared", { storageId }),
  resolveCloudRepository: (storageId: string, address: string) =>
    invoke<CatalogCloudResolution>("resolve_catalog_cloud_repository", { storageId, address }),
  joinCloudRepository: (storageId: string, code: string) =>
    invoke<CatalogCloudResolution>("join_catalog_cloud_repository", { storageId, code }),
  /** Saves an item's repository; an item that has one keeps it. Nothing is
   *  pulled. */
  connectCloudRepository: (scopeId: string, storageId: string, resourceId: string, repositoryId: string, expectedLinkRevision: number) =>
    invoke<void>("connect_catalog_cloud_repository", { scopeId, storageId, resourceId, repositoryId, expectedLinkRevision }),
  /** Creates a local item's private repository now and saves it. Safe to
   *  repeat: an existing one comes back. */
  createCloudRepository: (scopeId: string, storageId: string, resourceId: string, expectedLinkRevision: number) =>
    invoke<CatalogCloudRepository>("create_catalog_cloud_repository", { scopeId, storageId, resourceId, expectedLinkRevision }),
  getCloudStatus: (scopeId: string, storageId: string, resourceId: string, repositoryId: string) =>
    invoke<ManagedCloudStatus>("get_catalog_cloud_status", { scopeId, storageId, resourceId, repositoryId }),
  requestCloudRelease: (scopeId: string, storageId: string, resourceId: string, repositoryId: string, expectedRevision: number) =>
    invoke<void>("request_catalog_cloud_release", { scopeId, storageId, resourceId, repositoryId, expectedRevision }),
  discardCloudDraft: (scopeId: string, storageId: string, resourceId: string, repositoryId: string, expectedRevision: number) =>
    invoke<ManagedCloudDiscardedDraft>("discard_catalog_cloud_draft", { scopeId, storageId, resourceId, repositoryId, expectedRevision }),
  /** Every capability this build registers, in the backend's display order.
   * The frontend joins scopes and repositories to these by coordinate. */
  listCapabilities: () =>
    invoke<CatalogCapabilityDescriptor[]>("list_catalog_capabilities"),

  listProfiles: () => invoke<CatalogProfile[]>("list_catalog_resource_profiles"),

  /** Saving a home derives its name from the folder — setup never asks for
   * one. Re-saving an existing home returns that profile. */
  saveProfile: (provider: string, path: string) =>
    invoke<CatalogProfile>("save_catalog_resource_profile", {
      provider,
      path,
    }),

  /** What a typed home would become — the canonical folder, whether it can
   * be read, the profile it already is, the name Set up would give it —
   * answered without writing. */
  probeProfile: (provider: string, path: string) =>
    invoke<CatalogProfileProbe>("probe_catalog_resource_profile", { provider, path }),

  /** One saved home for its setup page: what it holds per capability, the
   * rows that already exist, and the storage a Catalog row used last. */
  inspectProfile: (profileId: string) =>
    invoke<CatalogProfileInspection>("inspect_catalog_resource_profile", { profileId }),

  /** Renames the profile everywhere it appears — every Catalog row using this
   * home relabels at once. */
  renameProfile: (profileId: string, displayName: string, expectedRevision: number) =>
    invoke<CatalogProfile>("rename_catalog_resource_profile", {
      profileId,
      displayName,
      expectedRevision,
    }),

  /** The session card's launch pair, profile-scoped: open the provider
   * desktop app or Terminal on the profile's configured home, at the default
   * path — no project and no session. */
  openProfileApp: (profileId: string) =>
    invoke<void>("open_catalog_profile_in_app", { profileId }),

  openProfileTerminal: (profileId: string) =>
    invoke<void>("open_catalog_profile_in_terminal", { profileId }),

  listScopes: () => invoke<CatalogScopeDto[]>("list_catalog_scopes"),

  /** Adding a row records a scope at one coordinate and nothing else — no
   * inventory, no provider CLI, no storage traffic. */
  createScope: (coordinate: CatalogCoordinate, profileId: string) =>
    invoke<CatalogScopeDto>("create_catalog_scope", {
      provider: coordinate.provider,
      profileId,
      resourceKind: coordinate.resource_kind,
      format: coordinate.format,
      formatVersion: coordinate.format_version,
    }),

  deleteScope: (scopeId: string, expectedRevision: number) =>
    invoke<boolean>("delete_catalog_scope", { scopeId, expectedRevision }),

  listScopeResources: (scopeId: string, storageId?: string | null) =>
    invoke<CatalogScopeResources>("list_catalog_scope_resources", {
      scopeId,
      storageId: storageId ?? null,
    }),

  /** The Catalog's repositories in one storage — derived ids, one head read
   * each, refusal rows for anything present but unreadable. Rendered by the
   * storage page beside the project repositories. */
  listStorageRepositories: (storageId: string) =>
    invoke<CatalogStorageRepository[]>("list_catalog_storage_repositories", {
      storageId,
    }),

  /** The preview for a stored-only row, fetched from the storage on demand
   * and cached by digest; the codec of the row's format draws it. Called
   * lazily per card — never in the list path — and a failure just leaves the
   * glyph tile. */
  getPreview: (scopeId: string, storageId: string, resourceId: string) =>
    invoke<string>("get_catalog_resource_preview", { scopeId, storageId, resourceId }),

  /** The scope links to the storage once; every resource of this coordinate
   * syncs through that one pairing. Linking is configuration only. */
  linkScopeStorage: (scopeId: string, storageId: string) =>
    invoke<CatalogScopeLinkOutcome>("link_catalog_scope_storage", {
      scopeId,
      storageId,
    }),

  unlinkScopeStorage: (linkId: string, expectedRevision: number) =>
    invoke<boolean>("unlink_catalog_scope_storage", { linkId, expectedRevision }),

  preparePush: (scopeId: string, storageId: string) =>
    invoke<CatalogScopePushPlan>("prepare_catalog_scope_push", {
      scopeId,
      storageId,
    }),

  applyPush: (
    planId: string,
    selectedResourceIds: string[],
    acknowledgedWarningDigests: string[],
    onProgress: (progress: CatalogSyncProgress) => void,
  ) => invoke<CatalogScopePushOutcome>("apply_catalog_scope_push", {
    planId,
    selectedResourceIds,
    acknowledgedWarningDigests,
    onProgress: progressChannel(onProgress),
  }),

  /** Finish that push: write the record an earlier push claimed and never
   *  wrote, once. */
  finishUnfinishedPush: (scopeId: string, storageId: string) =>
    invoke<void>("finish_catalog_unfinished_push", { scopeId, storageId }),

  preparePull: (
    scopeId: string,
    storageId: string,
    requestedCopyNames?: Record<string, string> | null,
  ) =>
    invoke<CatalogScopePullPlan>("prepare_catalog_scope_pull", {
      scopeId,
      storageId,
      requestedCopyNames: requestedCopyNames ?? null,
    }),

  applyPull: (
    planId: string,
    choices: Record<string, string>,
    destructiveAcknowledged: string[],
    onProgress: (progress: CatalogSyncProgress) => void,
  ) => invoke<CatalogScopePullOutcome>("apply_catalog_scope_pull", {
    planId,
    choices,
    destructiveAcknowledged,
    onProgress: progressChannel(onProgress),
  }),

  prepareFork: (scopeId: string, resourceId: string, requestedName: string) =>
    invoke<CatalogResourceForkPlan>("prepare_catalog_resource_fork", {
      scopeId,
      resourceId,
      requestedName,
    }),

  applyFork: (scopeId: string, planId: string) =>
    invoke<CatalogActionResult>("apply_catalog_resource_fork", {
      scopeId,
      planId,
    }),
};
