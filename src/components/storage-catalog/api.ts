import { invoke } from "@tauri-apps/api/core";
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

export type StorageCatalogApi = typeof storageCatalogApi;
