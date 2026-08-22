import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import AddCatalogScopeWorkspace from "./components/storage-catalog/AddCatalogScopeWorkspace";
import StorageCatalogPage from "./components/storage-catalog/StorageCatalogPage";
import type { StorageCatalogApi } from "./components/storage-catalog/api";
import type {
  CatalogPullItem,
  CatalogScopePullItem,
  CatalogScopePullPlan,
  CatalogScopePushPlan,
  CatalogScopeResources,
  CatalogScopeRow,
  CatalogScopeView,
} from "./components/storage-catalog/types";
import { PET_THUMBS } from "./catalog-pet-mockup-thumbs";
import "./App.css";
import "./RetroTheme.css";

/**
 * The real Catalog page against a stub backend, so the shipped component can
 * be seen and driven outside Tauri. Not part of the app bundle: the build has
 * one entry, and this file is reachable only from the dev server.
 */

const NOW = Math.floor(Date.now() / 1000);

const scope: CatalogScopeView = {
  scope: {
    scope_id: "catscope-1",
    provider: "codex",
    profile_id: "catprofile-1",
    resource_kind: "agent_skill",
    revision: 1,
    created_at: NOW,
    updated_at: NOW,
  },
  profile: {
    profile_id: "catprofile-1",
    provider_key: "codex",
    display_name: "Default Codex",
    configured_home: "/Users/hequ/.codex",
    revision: 1,
    created_at: NOW,
    updated_at: NOW,
  },
};

/** A stable sha-shaped stand-in per name, so the ⓘ popover carries a digest
 *  of realistic width without pretending to be anyone's data. */
function digest(name: string): string {
  let seed = 7;
  for (const ch of name) seed = (seed * 31 + ch.charCodeAt(0)) % 0xffff;
  return Array.from({ length: 16 }, (_, i) => (((seed + 1) * (i + 3) * 2_654_435_761) % 0xffff)
    .toString(16).padStart(4, "0")).join("");
}

function row(overrides: Partial<CatalogScopeRow> & { row_id: string; display_name: string }): CatalogScopeRow {
  return {
    resource_id: `resource-${overrides.row_id}`,
    scope_label: "Codex home",
    state: null,
    local_present: true,
    local_path: `/Users/hequ/.codex/skills/${overrides.display_name}`,
    local_modified_at: NOW - 86_400 * 3,
    local_files: 5,
    local_bytes: 22_100,
    remote_present: false,
    remote_updated_at: null,
    content_sha256: digest(overrides.display_name),
    warnings: [],
    blocker: null,
    ...overrides,
  };
}

const ROWS: CatalogScopeRow[] = [
  row({
    row_id: "triage", display_name: "triage", state: "in_sync",
    remote_present: true, remote_updated_at: NOW - 86_400 * 3,
    local_modified_at: NOW - 86_400 * 3,
    local_files: 6, local_bytes: 41_300,
  }),
  row({
    row_id: "changelog-writer", display_name: "changelog-writer", state: "local_changed",
    remote_present: true, remote_updated_at: NOW - 86_400 * 9, local_modified_at: NOW - 7_200,
    local_files: 3, local_bytes: 12_800,
    warnings: [{ code: "secret_shaped", message: "contains a secret-shaped value", digest: "digest-secret" }],
  }),
  row({
    row_id: "pdf-filler", display_name: "pdf-filler", state: "new",
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/pdf-filler",
    local_modified_at: NOW - 86_400 * 2,
    local_files: 11, local_bytes: 264_000,
  }),
  row({
    row_id: "release-notes", display_name: "release-notes", state: "storage_changed",
    remote_present: true, remote_updated_at: NOW - 3_600, local_modified_at: NOW - 86_400 * 5,
    local_files: 4, local_bytes: 18_400,
  }),
  row({
    row_id: "sql-review", display_name: "sql-review", state: "local_missing",
    local_present: false, local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null,
    remote_present: true, remote_updated_at: NOW - 86_400 * 8,
  }),
  row({
    row_id: "api-designer", display_name: "api-designer", state: "diverged",
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/api-designer",
    remote_present: true, remote_updated_at: NOW - 5_400, local_modified_at: NOW - 3_600,
    local_files: 9, local_bytes: 87_600,
    warnings: [{ code: "executable", message: "contains an executable file", digest: "digest-exec" }],
  }),
  row({
    row_id: "deploy-checklist", display_name: "deploy-checklist", state: "unreviewed_remote",
    remote_present: true, remote_updated_at: NOW - 86_400, local_modified_at: NOW - 86_400 * 6,
    local_files: 5, local_bytes: 22_100,
  }),
  row({
    row_id: "legacy-import", display_name: "legacy-import", state: "blocked",
    resource_id: null, scope_label: "Shared user", local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null, content_sha256: null,
    blocker: "the skill directory is a symlink, so its contents can't be captured safely",
  }),
];

/** Pets v2 · Default Codex: the three real v2 pets wearing their real idle
 *  frames, a stored-only row whose preview arrives lazily from the storage,
 *  and a blocked one that keeps the glyph tile. */
const petScope: CatalogScopeView = {
  scope: {
    scope_id: "catscope-pets",
    provider: "codex",
    profile_id: "catprofile-1",
    resource_kind: "codex_pet_v2",
    revision: 1,
    created_at: NOW,
    updated_at: NOW,
  },
  profile: scope.profile,
};

const PET_ROWS: CatalogScopeRow[] = [
  row({
    row_id: "maple", display_name: "Maple", state: "in_sync",
    scope_label: "v2", local_path: "/Users/hequ/.codex/pets/maple",
    remote_present: true, remote_updated_at: NOW - 86_400 * 2,
    local_files: 1, local_bytes: 2_633_494,
    preview_png: PET_THUMBS.maple,
    pet_description: "A compact brown-and-white border collie pet for Codex.",
  }),
  row({
    row_id: "dahuang", display_name: "大黄", state: "local_changed",
    scope_label: "v2", local_path: "/Users/hequ/.codex/pets/dahuang",
    remote_present: true, remote_updated_at: NOW - 86_400 * 4, local_modified_at: NOW - 7_200,
    local_files: 1, local_bytes: 1_784_464,
    preview_png: PET_THUMBS.dahuang,
    pet_description: "一只性格乖巧、黄身白脸、身形修长的中华田园犬。",
  }),
  row({
    row_id: "patch", display_name: "Patch", state: "new",
    scope_label: "v2", local_path: "/Users/hequ/.codex/pets/patch",
    local_files: 1, local_bytes: 1_686_990,
    preview_png: PET_THUMBS.patch,
    pet_description: "A calm, observant green turtle with natural anatomy and patient, deliberate movement.",
  }),
  row({
    row_id: "biscuit", display_name: "Biscuit", state: "local_missing",
    scope_label: "v2", local_present: false, local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null, preview_png: null,
    remote_present: true, remote_updated_at: NOW - 86_400 * 4,
    pet_description: "A stored-only pet whose preview arrives from the storage.",
  }),
  row({
    row_id: "scout", display_name: "Scout", state: "blocked",
    resource_id: null, scope_label: "v2", local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null, content_sha256: null, preview_png: null,
    blocker: "the pet package is a symlink, so its contents can't be captured safely",
  }),
];

function pullItem(
  rowId: string,
  actions: CatalogPullItem["actions"],
  claims: string[] = [],
  kind: CatalogPullItem["kind"] = "agent_skill",
): CatalogScopePullItem {
  return {
    item: {
      resource_id: `resource-${rowId}`,
      kind,
      display_name: rowId,
      provider: "codex",
      relation: "storage_changed",
      disposition: { kind: "executable" },
      actions,
      default_action_id: actions[0]?.action_id ?? null,
    },
    conflicting_claims: claims.map((storage_id) => ({ storage_id, content_sha256: "a".repeat(64) })),
  };
}

const SKILL_PULL_ITEMS: CatalogScopePullItem[] = [
  pullItem("release-notes", [
    { action_id: "a1", resource_id: "resource-release-notes", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a2", resource_id: "resource-release-notes", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "release-notes-local-copy", copy_resource_id: "resource-copy", copy_target_path: "/Users/hequ/.codex/skills/release-notes-local-copy", transformed_content_sha256: "c".repeat(64), original_content_sha256: "d".repeat(64), storage_content_sha256: "e".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a3", resource_id: "resource-release-notes", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "b".repeat(64) },
  ]),
  pullItem("sql-review", [
    { action_id: "b1", resource_id: "resource-sql-review", kind: { kind: "install_skill" }, requires_destructive_approval: false, incoming_content_sha256: "f".repeat(64) },
  ]),
  pullItem("api-designer", [
    { action_id: "c1", resource_id: "resource-api-designer", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "api-designer-local-copy", copy_resource_id: "resource-copy-2", copy_target_path: "/Users/hequ/.agents/skills/api-designer-local-copy", transformed_content_sha256: "1".repeat(64), original_content_sha256: "2".repeat(64), storage_content_sha256: "3".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "4".repeat(64) },
    { action_id: "c2", resource_id: "resource-api-designer", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "4".repeat(64) },
  ], ["storage-vault"]),
  pullItem("deploy-checklist", [
    { action_id: "d1", resource_id: "resource-deploy-checklist", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "5".repeat(64) },
  ]),
  // Already-settled rows arrive in the plan and stay out of the review.
  {
    item: {
      resource_id: "resource-triage",
      kind: "agent_skill",
      display_name: "triage",
      provider: "codex",
      relation: "matching",
      disposition: { kind: "already_matching" },
      actions: [],
      default_action_id: null,
    },
    conflicting_claims: [],
  },
];

const PET_PULL_ITEMS: CatalogScopePullItem[] = [
  pullItem("biscuit", [
    { action_id: "p1", resource_id: "resource-biscuit", kind: { kind: "install_pet" }, requires_destructive_approval: false, incoming_content_sha256: "9".repeat(64) },
  ], [], "codex_pet_v2"),
];

const STORAGES = [
  { id: "storage-local-1", name: "Local storage 1", kind: "local" as const, detail: "~/Library/Mallard/local-1", detailIsPath: true },
  { id: "storage-vault", name: "Vault", kind: "s3" as const, detail: "Bucket · mallard-vault", detailIsPath: false },
];

function stubApi(storageId: string | null, view: "skills" | "pets"): StorageCatalogApi {
  const activeScope = view === "pets" ? petScope : scope;
  const activeRows = view === "pets" ? PET_ROWS : ROWS;
  const page = (): CatalogScopeResources => ({
    scope: activeScope.scope,
    profile: activeScope.profile,
    storage_id: storageId,
    storage_name: STORAGES.find((storage) => storage.id === storageId)?.name ?? null,
    link: storageId ? { link_id: `catlink-${view}`, revision: 3 } : null,
    storage_revision: storageId ? 6 : null,
    /* Without a storage the backend lists only what the home scan saw — no
       stored-only rows — and a blocked row keeps saying so. */
    rows: storageId ? activeRows : activeRows
      .filter((entry) => entry.local_present || entry.blocker)
      .map((entry) => ({ ...entry, state: entry.blocker ? entry.state : null })),
    warnings: [],
    recovery: [],
  });
  const pushPlan = (): CatalogScopePushPlan => ({
    schema_version: 1,
    plan_id: `push-${view}`,
    scope_id: activeScope.scope.scope_id,
    link_id: `catlink-${view}`,
    link_revision: 3,
    storage_id: storageId ?? "",
    repository_id: "library-kind",
    rows: activeRows
      .filter((entry) => entry.resource_id && (entry.local_present || entry.remote_present))
      .map((entry) => {
        const selectable = entry.state === "new" || entry.state === "local_changed";
        const held = entry.state === "diverged" || entry.state === "unreviewed_remote";
        return {
          row: {
            resource_id: entry.resource_id!,
            kind: activeScope.scope.resource_kind,
            display_name: entry.display_name,
            provider: "codex" as const,
            scope_label: entry.scope_label ?? null,
            relation: entry.state === "new"
              ? ("local_only" as const)
              : entry.state === "local_changed"
                ? ("local_changed" as const)
                : entry.state === "diverged"
                  ? ("changed_both" as const)
                  : ("matching" as const),
            selectable,
            selected_by_default: selectable,
            held_for_pull: held,
            removal_candidate: false,
            blocked_reason: held ? "Storage changed; review Pull before pushing" : null,
            warnings: entry.warnings,
            detail: selectable ? "Publish this resource to the linked local storage" : null,
          },
        };
      }),
    warning_digests: activeRows.flatMap((entry) => entry.warnings.map((warning) => warning.digest)),
    created_at: NOW,
    expires_at: NOW + 900,
  });
  const pullPlan = (): CatalogScopePullPlan => ({
    schema_version: 1,
    plan_id: `pull-${view}`,
    scope_id: activeScope.scope.scope_id,
    link_id: `catlink-${view}`,
    link_revision: 3,
    storage_id: storageId ?? "",
    repository_id: "library-kind",
    items: view === "pets" ? PET_PULL_ITEMS : SKILL_PULL_ITEMS,
    created_at: NOW,
    expires_at: NOW + 900,
  });
  return {
    listScopeResources: async () => page(),
    getPetPreview: async (_scopeId, _storageId, resourceId) => {
      // The stored-only pet, after a network-ish pause so the lazy fill is
      // visible; everything else has nothing stored to draw.
      if (resourceId !== "resource-biscuit") throw new Error("no stored preview");
      await new Promise((resolve) => setTimeout(resolve, 700));
      return PET_THUMBS.tracebill;
    },
    linkScopeStorage: async () => ({
      link_id: `catlink-${view}`,
      revision: 3,
      completion: "open",
    }),
    unlinkScopeStorage: async () => true,
    preparePush: async () => pushPlan(),
    applyPush: async (_planId, selectedResourceIds) => ({
      plan_id: `push-${view}`,
      published: selectedResourceIds.length > 0,
      revision: 7,
      message: selectedResourceIds.length > 0 ? "Published revision 7" : "Nothing was pushed",
    }),
    preparePull: async () => pullPlan(),
    applyPull: async (_planId, choices) => ({
      plan_id: `pull-${view}`,
      remote_accepted: true,
      results: Object.keys(choices).map((resourceId) => ({
        resource_id: resourceId,
        display_name: resourceId.replace(/^resource-/, ""),
        outcome: "updated" as const,
      })),
    }),
    prepareFork: async (scopeId, resourceId, requestedName) => ({
      schema_version: 1,
      plan_id: `fork-${resourceId}`,
      scope_id: scopeId,
      source_resource_id: resourceId,
      source_display_name: resourceId.replace(/^resource-/, ""),
      copy_name: requestedName,
      copy_resource_id: `resource-${requestedName}`,
      copy_target_path: `/Users/hequ/.codex/skills/${requestedName}`,
      created_at: NOW,
      expires_at: NOW + 900,
    }),
    applyFork: async (_scopeId, planId) => ({
      resource_id: planId.replace(/^fork-/, ""),
      display_name: planId.replace(/^fork-resource-/, ""),
      outcome: "forked",
      message: `Created ${planId.replace(/^fork-resource-/, "")}; local-only until you push it`,
    }),
    listCapabilities: async () => [
      {
        provider: "codex",
        kinds: ["agent_skill", "plugin_install_intent", "codex_pet_v1", "codex_pet_v2"],
      },
      { provider: "claude", kinds: ["agent_skill", "plugin_install_intent"] },
    ],
    listProfiles: async () => [scope.profile],
    /* Mirrors the backend: the name derives from the folder, nobody types one. */
    saveProfile: async (_provider, path) => ({
      ...scope.profile,
      profile_id: "catprofile-added",
      display_name: path.split("/").filter(Boolean).pop() ?? "profile",
      configured_home: path,
    }),
    renameProfile: async (_profileId, displayName) => ({
      ...scope.profile,
      display_name: displayName,
      revision: scope.profile.revision + 1,
    }),
    listScopes: async () => [scope],
    createScope: async () => scope,
    deleteScope: async () => true,
  };
}

function Harness() {
  const [storageId, setStorageId] = useState<string | null>("storage-local-1");
  const [view, setView] = useState<"skills" | "pets" | "add">("skills");
  const [dark, setDark] = useState(false);
  if (view === "add") {
    return (
      <div className="app" style={{ height: "100vh" }}>
        <div className="sync-workspace">
          <AddCatalogScopeWorkspace
            workflowEpoch={0}
            isWorkflowCurrent={() => true}
            busy={false}
            onClose={() => setView("skills")}
            onCreated={() => setView("pets")}
            api={stubApi(storageId, "pets")}
          />
        </div>
      </div>
    );
  }
  return (
    <div className="app" style={{ height: "100vh" }}>
      <div className="sync-workspace">
        <StorageCatalogPage
          scope={view === "pets" ? petScope : scope}
          scopes={[scope, petScope]}
          storages={STORAGES}
          appActions={[{
            items: [{
              id: "theme",
              label: dark ? "Light theme" : "Dark theme",
              icon: "sun",
              onSelect: () => {
                const next = !dark;
                setDark(next);
                document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
              },
            }, {
              id: "storage",
              label: storageId ? "Forget the storage" : "Use Local storage 1",
              icon: "cloud",
              onSelect: () => setStorageId(storageId ? null : "storage-local-1"),
            }, {
              id: "view",
              label: view === "pets" ? "View Skills" : "View Pets v2",
              icon: "openai",
              onSelect: () => setView(view === "pets" ? "skills" : "pets"),
            }, {
              id: "add",
              label: "View Add resource",
              icon: "plus",
              onSelect: () => setView("add"),
            }],
          }]}
          onOpenScope={(scopeId) => setView(scopeId === "catscope-pets" ? "pets" : "skills")}
          onOpenStorageSettings={() => undefined}
          onAddStorage={() => undefined}
          onAddScope={() => undefined}
          onRemoveScope={() => undefined}
          onProfileRenamed={() => undefined}
          notify={(message) => console.info("notice:", message)}
          onScopeGone={() => undefined}
          api={stubApi(storageId, view)}
        />
      </div>
    </div>
  );
}

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
