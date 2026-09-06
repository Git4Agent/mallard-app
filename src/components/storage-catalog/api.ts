import { invoke } from "../../ipc";
import type {
  CatalogActionResult,
  CatalogCapabilityDescriptor,
  CatalogCoordinate,
  CatalogProfile,
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

export const storageCatalogApi = {
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
  ) => invoke<CatalogScopePushOutcome>("apply_catalog_scope_push", {
    planId,
    selectedResourceIds,
    acknowledgedWarningDigests,
  }),

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
  ) => invoke<CatalogScopePullOutcome>("apply_catalog_scope_pull", {
    planId,
    choices,
    destructiveAcknowledged,
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
