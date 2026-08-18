import { invoke } from "@tauri-apps/api/core";
import type {
  CatalogActionResult,
  CatalogCapability,
  CatalogLinkInspection,
  CatalogProviderKey,
  CatalogResourceForkPlan,
  CatalogResourceKind,
  CatalogResourcePullOutcome,
  CatalogResourcePullPlan,
  CatalogResourcePushOutcome,
  CatalogResourcePushPlan,
  CatalogResourceStatus,
  CatalogResourceView,
  CatalogScopeLinkOutcome,
  CatalogScopeResources,
  CatalogScopeView,
  LinkCatalogScopeResourceRequest,
  LocalCatalogProfile,
} from "./types";

export const storageCatalogApi = {
  listCapabilities: () => invoke<CatalogCapability[]>("list_catalog_capabilities"),

  listProfiles: () =>
    invoke<LocalCatalogProfile[]>("list_catalog_resource_profiles"),

  saveProfile: (provider: CatalogProviderKey, displayName: string, path: string) =>
    invoke<LocalCatalogProfile>("save_catalog_resource_profile", {
      provider,
      displayName,
      path,
    }),

  deleteProfile: (profileId: string, expectedRevision: number) =>
    invoke<boolean>("delete_catalog_resource_profile", { profileId, expectedRevision }),

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

  linkScopeResource: (request: LinkCatalogScopeResourceRequest) =>
    invoke<CatalogScopeLinkOutcome>("link_catalog_scope_resource", { request }),

  /** The per-resource detail behind a row: its storage links and identity. */
  getResource: (registrationId: string) =>
    invoke<CatalogResourceView>("get_catalog_resource", { registrationId }),

  unlink: (registrationId: string, linkId: string, expectedRevision: number) =>
    invoke<boolean>("unlink_catalog_resource", {
      registrationId,
      linkId,
      expectedRevision,
    }),

  remove: (registrationId: string, expectedRevision: number) =>
    invoke<boolean>("remove_catalog_resource", { registrationId, expectedRevision }),

  inspectLinks: (registrationId: string) =>
    invoke<CatalogLinkInspection[]>("inspect_catalog_resource_links", {
      registrationId,
    }),

  preparePush: (linkId: string) =>
    invoke<CatalogResourcePushPlan>("prepare_catalog_resource_push", { linkId }),

  publish: (
    linkId: string,
    planId: string,
    include: boolean,
    acknowledgedWarningDigests: string[],
  ) => invoke<CatalogResourcePushOutcome>("publish_catalog_resource", {
    linkId,
    planId,
    include,
    acknowledgedWarningDigests,
  }),

  preparePull: (linkId: string, requestedCopyName?: string | null) =>
    invoke<CatalogResourcePullPlan>("prepare_catalog_resource_pull", {
      linkId,
      requestedCopyName: requestedCopyName ?? null,
    }),

  applyPull: (
    linkId: string,
    planId: string,
    approvedActionId: string | null,
    destructiveAcknowledged: boolean,
  ) => invoke<CatalogResourcePullOutcome>("apply_catalog_resource_pull", {
    linkId,
    planId,
    approvedActionId,
    destructiveAcknowledged,
  }),

  prepareFork: (registrationId: string, requestedName: string) =>
    invoke<CatalogResourceForkPlan>("prepare_catalog_resource_fork", {
      registrationId,
      requestedName,
    }),

  applyFork: (registrationId: string, planId: string) =>
    invoke<CatalogActionResult>("apply_catalog_resource_fork", {
      registrationId,
      planId,
    }),

  getStatus: (registrationId: string, linkId?: string | null) =>
    invoke<CatalogResourceStatus>("get_catalog_resource_status", {
      registrationId,
      linkId: linkId ?? null,
    }),
};

export type StorageCatalogApi = typeof storageCatalogApi;
