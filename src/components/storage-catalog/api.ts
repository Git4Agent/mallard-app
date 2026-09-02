import { invoke } from "../../ipc";
import type {
  CatalogActionResult,
  CatalogCapability,
  CatalogProviderKey,
  CatalogResourceForkPlan,
  CatalogResourceKind,
  CatalogScopeLinkOutcome,
  CatalogScopePullOutcome,
  CatalogScopePullPlan,
  CatalogScopePushOutcome,
  CatalogScopePushPlan,
  CatalogScopeResources,
  CatalogScopeView,
  CatalogStorageRepository,
  LocalCatalogProfile,
} from "./types";

export const storageCatalogApi = {
  listCapabilities: () => invoke<CatalogCapability[]>("list_catalog_capabilities"),

  listProfiles: () =>
    invoke<LocalCatalogProfile[]>("list_catalog_resource_profiles"),

  /** Saving a home derives its name from the folder — setup never asks for
   * one. Re-saving an existing home returns that profile. */
  saveProfile: (provider: CatalogProviderKey, path: string) =>
    invoke<LocalCatalogProfile>("save_catalog_resource_profile", {
      provider,
      path,
    }),

  /** Renames the profile everywhere it appears — every Catalog row using this
   * home relabels at once. */
  renameProfile: (profileId: string, displayName: string, expectedRevision: number) =>
    invoke<LocalCatalogProfile>("rename_catalog_resource_profile", {
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

  listScopes: () => invoke<CatalogScopeView[]>("list_catalog_scopes"),

  /** Adding a row records a scope and nothing else — no inventory, no
   * provider CLI, no storage traffic. */
  createScope: (
    provider: CatalogProviderKey,
    profileId: string,
    resourceKind: CatalogResourceKind,
  ) =>
    invoke<CatalogScopeView>("create_catalog_scope", {
      provider,
      profileId,
      resourceKind,
    }),

  deleteScope: (scopeId: string, expectedRevision: number) =>
    invoke<boolean>("delete_catalog_scope", { scopeId, expectedRevision }),

  listScopeResources: (scopeId: string, storageId?: string | null) =>
    invoke<CatalogScopeResources>("list_catalog_scope_resources", {
      scopeId,
      storageId: storageId ?? null,
    }),

  /** The Catalog's kind repositories in one storage — derived ids, one head
   * read each, refusal rows for anything present but unreadable. Rendered by
   * the storage page beside the project repositories. */
  listStorageRepositories: (storageId: string) =>
    invoke<CatalogStorageRepository[]>("list_catalog_storage_repositories", {
      storageId,
    }),

  /** The preview for a stored-only pet row, fetched from the storage on
   * demand and cached by the atlas digest. Called lazily per card — never in
   * the list path — and a failure just leaves the glyph tile. */
  getPetPreview: (scopeId: string, storageId: string, resourceId: string) =>
    invoke<string>("get_catalog_pet_preview", { scopeId, storageId, resourceId }),

  /** The scope links to the storage once; every resource of this kind syncs
   * through that one pairing. Linking is configuration only. */
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
