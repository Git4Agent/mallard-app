import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import StorageCatalogPage from "./components/storage-catalog/StorageCatalogPage";
import type { StorageCatalogApi } from "./components/storage-catalog/api";
import type {
  CatalogResourcePullPlan,
  CatalogScopeResources,
  CatalogScopeRow,
  CatalogScopeView,
} from "./components/storage-catalog/types";
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
    linked: false,
    local_present: true,
    local_path: `/Users/hequ/.codex/skills/${overrides.display_name}`,
    local_modified_at: NOW - 86_400 * 3,
    local_files: 5,
    local_bytes: 22_100,
    remote_present: false,
    remote_revision: null,
    remote_updated_at: null,
    content_sha256: digest(overrides.display_name),
    registration_id: `registration-${overrides.row_id}`,
    registration_revision: 1,
    link_id: `link-${overrides.row_id}`,
    link_revision: 1,
    repository_id: `repository-${overrides.row_id}`,
    other_storage_names: [],
    warnings: [],
    blocker: null,
    ...overrides,
  };
}

const ROWS: CatalogScopeRow[] = [
  row({
    row_id: "triage", display_name: "triage", state: "in_sync",
    remote_present: true, remote_revision: 4, other_storage_names: ["Vault"],
    local_modified_at: NOW - 86_400 * 3,
    local_files: 6, local_bytes: 41_300,
  }),
  row({
    row_id: "changelog-writer", display_name: "changelog-writer", state: "local_changed",
    remote_present: true, remote_revision: 2, local_modified_at: NOW - 7_200,
    local_files: 3, local_bytes: 12_800,
    warnings: [{ code: "secret_shaped", message: "contains a secret-shaped value", digest: "digest-secret" }],
  }),
  row({
    row_id: "pdf-filler", display_name: "pdf-filler", state: "new", linked: false,
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/pdf-filler",
    link_id: null, registration_id: null, repository_id: null,
    local_modified_at: NOW - 86_400 * 2,
    local_files: 11, local_bytes: 264_000,
  }),
  row({
    row_id: "release-notes", display_name: "release-notes", state: "storage_changed",
    remote_present: true, remote_revision: 6, local_modified_at: NOW - 86_400 * 5,
    local_files: 4, local_bytes: 18_400,
  }),
  row({
    row_id: "sql-review", display_name: "sql-review", state: "local_missing",
    local_present: false, local_path: null, local_modified_at: null, linked: false,
    local_files: null, local_bytes: null,
    link_id: null, registration_id: null,
    remote_present: true, remote_revision: 3, remote_updated_at: NOW - 86_400 * 8,
  }),
  row({
    row_id: "api-designer", display_name: "api-designer", state: "diverged",
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/api-designer",
    remote_present: true, remote_revision: 5, local_modified_at: NOW - 3_600,
    local_files: 9, local_bytes: 87_600,
    warnings: [{ code: "executable", message: "contains an executable file", digest: "digest-exec" }],
  }),
  row({
    row_id: "deploy-checklist", display_name: "deploy-checklist", state: "unreviewed_remote",
    remote_present: true, remote_revision: 1, local_modified_at: NOW - 86_400 * 6,
    local_files: 5, local_bytes: 22_100,
  }),
  row({
    row_id: "legacy-import", display_name: "legacy-import", state: "blocked",
    scope_label: "Shared user", local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null, content_sha256: null,
    link_id: null, registration_id: null, repository_id: null,
    blocker: "the skill directory is a symlink, so its contents can't be captured safely",
  }),
];

function pullPlan(rowId: string, actions: CatalogResourcePullPlan["item"]["actions"], claims: string[] = []): CatalogResourcePullPlan {
  return {
    schema_version: 1,
    plan_id: `plan-${rowId}`,
    registration_id: `registration-${rowId}`,
    link_id: `link-${rowId}`,
    storage_id: "storage-local-1",
    repository_id: `repository-${rowId}`,
    item: {
      resource_id: `resource-${rowId}`,
      kind: "agent_skill",
      display_name: rowId,
      provider: "codex",
      relation: "storage_changed",
      disposition: { kind: "executable" },
      actions,
      default_action_id: actions[0]?.action_id ?? null,
    },
    conflicting_claims: claims.map((storage_id) => ({ storage_id, content_sha256: "a".repeat(64) })),
    created_at: NOW,
    expires_at: NOW + 900,
  };
}

const PULL_PLANS: Record<string, CatalogResourcePullPlan> = {
  "release-notes": pullPlan("release-notes", [
    { action_id: "a1", resource_id: "resource-release-notes", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a2", resource_id: "resource-release-notes", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "release-notes-local-copy", copy_resource_id: "resource-copy", copy_target_path: "/Users/hequ/.codex/skills/release-notes-local-copy", transformed_content_sha256: "c".repeat(64), original_content_sha256: "d".repeat(64), storage_content_sha256: "e".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a3", resource_id: "resource-release-notes", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "b".repeat(64) },
  ]),
  "sql-review": pullPlan("sql-review", [
    { action_id: "b1", resource_id: "resource-sql-review", kind: { kind: "install_skill" }, requires_destructive_approval: false, incoming_content_sha256: "f".repeat(64) },
  ]),
  "api-designer": pullPlan("api-designer", [
    { action_id: "c1", resource_id: "resource-api-designer", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "api-designer-local-copy", copy_resource_id: "resource-copy-2", copy_target_path: "/Users/hequ/.agents/skills/api-designer-local-copy", transformed_content_sha256: "1".repeat(64), original_content_sha256: "2".repeat(64), storage_content_sha256: "3".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "4".repeat(64) },
    { action_id: "c2", resource_id: "resource-api-designer", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "4".repeat(64) },
  ], ["storage-vault"]),
  "deploy-checklist": pullPlan("deploy-checklist", [
    { action_id: "d1", resource_id: "resource-deploy-checklist", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "5".repeat(64) },
  ]),
};

const STORAGES = [
  { id: "storage-local-1", name: "Local storage 1", kind: "local" as const, detail: "~/Library/Mallard/local-1", detailIsPath: true },
  { id: "storage-vault", name: "Vault", kind: "s3" as const, detail: "Bucket · mallard-vault", detailIsPath: false },
];

function stubApi(storageId: string | null): StorageCatalogApi {
  const page = (): CatalogScopeResources => ({
    scope: scope.scope,
    profile: scope.profile,
    storage_id: storageId,
    storage_name: STORAGES.find((storage) => storage.id === storageId)?.name ?? null,
    /* Without a storage the backend lists only what the home scan saw — no
       stored-only rows — and a blocked row keeps saying so. */
    rows: storageId ? ROWS : ROWS
      .filter((entry) => entry.local_present || entry.blocker)
      .map((entry) => ({ ...entry, state: entry.blocker ? entry.state : null, linked: false })),
    warnings: [],
  });
  const unsupported = () => Promise.reject(new Error("not wired in the live harness"));
  return {
    listScopeResources: async () => page(),
    preparePull: async (linkId: string) => {
      const plan = PULL_PLANS[linkId.replace(/^link-/, "")];
      if (!plan) throw new Error("no Pull plan for this row");
      return plan;
    },
    applyPull: async () => ({
      plan_id: "plan", remote_accepted: true,
      result: { resource_id: "r", display_name: "row", outcome: "updated" },
    }),
    preparePush: async (linkId: string) => ({
      schema_version: 1, plan_id: `push-${linkId}`, registration_id: "r", link_id: linkId,
      storage_id: storageId ?? "", repository_id: "repo",
      row: {
        resource_id: "r", kind: "agent_skill", display_name: "row", provider: "codex",
        relation: "local_changed", selectable: true, selected_by_default: true,
        held_for_pull: false, removal_candidate: false, warnings: [],
      },
      warning_digests: [], created_at: NOW, expires_at: NOW + 900,
    }),
    publish: async () => ({ plan_id: "p", published: true, revision: 7, message: "Published" }),
    linkScopeResource: async (request) => ({
      resource: await stubApi(storageId).getResource("registration-triage"),
      completion: "review_pull",
      link_id: `link-${(request.repository_id ?? request.local_resource_id ?? "").replace(/^(repository|resource)-/, "")}`,
    }),
    getResource: async () => ({
      registration: {
        registration_id: "registration-triage", provider: "codex", profile_id: "catprofile-1",
        resource_kind: "agent_skill", resource_id: "resource-triage",
        local_target: { kind: "skill_directory", scope: "codex_home", canonical_root: "/Users/hequ/.codex/skills", install_directory: "triage" },
        display_name: "triage", revision: 1, created_at: NOW, updated_at: NOW,
      },
      scope_label: "Codex home",
      links: [{
        link: {
          link_id: "link-triage", registration_id: "registration-triage", storage_id: "storage-local-1",
          repository_id: "repository-triage-0123456789abcdef", revision: 2, created_at: NOW, updated_at: NOW,
        },
        storage_name: "Local storage 1",
      }],
    }),
    getStatus: async () => ({ registration_id: "registration-triage", active_plan_ids: [], recovery_messages: [] }),
    inspectLinks: async () => [{
      registration_id: "registration-triage", link_id: "link-triage", storage_id: "storage-local-1",
      repository_id: "repository-triage", state: "in_sync", local_present: true, remote_present: true,
      remote_revision: 4, generated_at: NOW,
    }],
    unlink: async () => true,
    remove: async () => true,
    prepareFork: async () => unsupported() as never,
    applyFork: async () => unsupported() as never,
    listCapabilities: async () => [],
    listProfiles: async () => [scope.profile],
    saveProfile: async () => scope.profile,
    listScopes: async () => [scope],
    createScope: async () => scope,
    deleteScope: async () => true,
    deleteProfile: async () => true,
  };
}

function Harness() {
  const [storageId, setStorageId] = useState<string | null>("storage-local-1");
  const [dark, setDark] = useState(false);
  return (
    <div className="app" style={{ height: "100vh" }}>
      <div className="sync-workspace">
        <StorageCatalogPage
          scope={scope}
          scopes={[scope]}
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
            }],
          }]}
          onOpenScope={() => undefined}
          onOpenStorageSettings={() => undefined}
          onAddStorage={() => undefined}
          onAddScope={() => undefined}
          onRemoveScope={() => undefined}
          notify={(message) => console.info("notice:", message)}
          onScopeGone={() => undefined}
          api={stubApi(storageId)}
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
