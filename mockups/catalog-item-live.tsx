import { StrictMode, useEffect, useState, type CSSProperties } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import StorageCatalogPage, { type CatalogStorageOption } from "../src/components/storage-catalog/StorageCatalogPage";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogResourceDetail,
  CatalogScopeRow,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { MOSSY_ATLAS, MOSSY_TILE } from "./catalog-item-page-art";
import "../src/App.css";

/**
 * Live: the shipped Catalog item page — `CatalogItemPage` inside the real
 * `StorageCatalogPage` — on a stand-in backend, opened on one item. Dev
 * server only: `npm run dev`, then /mockups/catalog-item-live.html
 * (`?kind=skill|plugin|pet&storage=cloud|vault&width=580|778|1118&state=local|stored&theme=light|dark`).
 */

const params = new URLSearchParams(window.location.search);
function pick<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  const value = params.get(key);
  return value && (allowed as readonly string[]).includes(value) ? value as T : fallback;
}
type Kind = "skill" | "plugin" | "pet";
type StorageKey = "cloud" | "vault";
type Width = "580" | "778" | "1118";
const NOW = Math.floor(Date.now() / 1000);
const noop = () => {};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

const profileOf = (provider: "claude" | "codex"): CatalogProfile => ({
  profile_id: `profile-${provider}`, provider, provider_label: provider === "claude" ? "Claude" : "Codex",
  display_name: provider === "claude" ? "Claude" : "Codex", configured_home: `/Users/you/.${provider}`,
  revision: 1, created_at: NOW - 86_400, updated_at: NOW - 86_400,
});

const CAPABILITY: Record<Kind, CatalogCapabilityDescriptor> = {
  skill: {
    provider: "claude", provider_label: "Claude", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
    label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
    capture: "directory_tree", apply: "automatic", targets: ["user.skills"],
  },
  plugin: {
    provider: "claude", provider_label: "Claude", resource_kind: "plugin_install_intent", format: "mallard.plugin-install-intent", format_version: 1,
    label: "Plugins", singular_label: "Plugin", presentation: { icon: "plugin", kind_label: "Plugins", format_label: "Plugin install intent" },
    capture: "provider_inventory", apply: "automatic", targets: ["user.plugins"],
  },
  pet: {
    provider: "codex", provider_label: "Codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2,
    label: "Pets v2", singular_label: "Pet v2", presentation: { icon: "pet-v2", kind_label: "Pets", format_label: "Codex pet v2" },
    capture: "directory_tree", apply: "automatic", targets: ["desktop.pets"],
  },
};
const KINDS: Kind[] = ["skill", "plugin", "pet"];

const scopeOf = (kind: Kind): CatalogScopeView => {
  const capability = CAPABILITY[kind];
  const profile = profileOf(capability.provider as "claude" | "codex");
  return {
    scope: {
      scope_id: `scope-${kind}`, profile_id: profile.profile_id, provider: capability.provider, resource_kind: capability.resource_kind,
      format: capability.format, format_version: capability.format_version, revision: 1, created_at: NOW, updated_at: NOW,
    },
    profile, label: capability.label, available: true,
    provider_label: capability.provider_label, kind_label: capability.presentation.kind_label, format_label: capability.presentation.format_label,
    capability,
  };
};

const STORAGES: CatalogStorageOption[] = [
  { id: "storage-cloud", provider: "managed_cloud", name: "Mallard Cloud", provider_label: "Mallard Cloud", icon: "storage-mallard", detail: "maya · mallard-ai.com", detailIsPath: false, account_login: "maya" },
  { id: "storage-vault", provider: "local_folder", name: "Vault", provider_label: "Local folder", icon: "storage-local", detail: "~/Mallard Vault", detailIsPath: true },
];

const ITEM: Record<Kind, { id: string; name: string; folder: string; bytes: number | null }> = {
  skill: { id: "release-notes", name: "release-notes", folder: "/Users/you/.claude/skills/release-notes", bytes: 9_412 },
  plugin: { id: "lint-buddy@acme-tools", name: "lint-buddy@acme-tools", folder: "/Users/you/.claude/plugins/cache/acme-tools/lint-buddy/1.4.0", bytes: null },
  pet: { id: "mossy", name: "Mossy", folder: "/Users/you/.codex/pets/mossy", bytes: 6_034 },
};

const SKILL_MD = `---
name: release-notes
description: Drafts release notes from the commits since the last tag, grouped by area, in the house voice.
allowed-tools: Bash(git log:*), Read
---

# Release notes

Write release notes for the range the user names, or for everything since the newest tag.

## Steps

1. Find the range: \`git describe --tags --abbrev=0\` names the last tag.
2. Read \`git log --no-merges <tag>..HEAD\` and group the commits by area.
3. Draft one line per change in the house voice (see \`voice.md\`).

| Section | What goes in it |
| --- | --- |
| Breaking | Anything that needs action from users |
| Fixed | Bug fixes users would notice |

See the [Keep a Changelog](https://keepachangelog.com) guide.
`;

const detailOf = (kind: Kind, local: boolean): CatalogResourceDetail => {
  if (kind === "skill") {
    return { kind: "document", path: "SKILL.md", text: SKILL_MD, files: [
      { path: "SKILL.md", size: 1_644, executable: false },
      { path: "voice.md", size: 3_120, executable: false },
      { path: "templates/notes.md", size: 2_204, executable: false },
      { path: "scripts/range.sh", size: 2_444, executable: true },
    ] };
  }
  if (kind === "plugin") {
    return {
      kind: "install", plugin_id: "lint-buddy@acme-tools", marketplace: "acme-tools", location: "acme-tools/claude-plugins", git_ref: null,
      install: ["claude plugin marketplace add acme-tools/claude-plugins --scope user", "claude plugin install lint-buddy@acme-tools --scope user"],
      installed_version: local ? "1.4.0" : null,
      listing: local ? {
        display_name: "Lint Buddy", developer: "Acme Tools", category: "Code quality",
        description: "Runs your project's linters after every edit and explains each finding in plain words, with a fix you can accept.",
        website_url: "https://example.com/lint-buddy", privacy_url: "https://example.com/privacy", terms_url: "https://example.com/terms",
      } : null,
    };
  }
  return { kind: "sprite", atlas: MOSSY_ATLAS, columns: 8, rows: 11, description: "A mallard who naps on the terminal and paddles when a build is running." };
};

const rowOf = (kind: Kind, local: boolean): CatalogScopeRow => ({
  row_id: `row-${ITEM[kind].id}`, display_name: ITEM[kind].name, resource_id: ITEM[kind].id, state: local ? "local_changed" : "local_missing",
  local_present: local, local_path: local ? ITEM[kind].folder : null, local_modified_at: local ? NOW - 3_600 * 3 : null,
  local_files: local && kind !== "plugin" ? 4 : null, local_bytes: local ? ITEM[kind].bytes : null,
  remote_present: true, remote_updated_at: NOW - 86_400 * 2, content_sha256: "a3f9".padEnd(64, "0"),
  preview_png: kind === "pet" ? MOSSY_TILE : null, description: kind === "pet" ? "A mallard who naps." : null,
  warnings: local && kind === "skill" ? [{ code: "executable_content", message: "scripts/range.sh is executable.", digest: "d".repeat(64) }] : [],
});

const statusOf = (kind: Kind): ManagedCloudStatus => ({
  repository_id: `repo_${kind}`, state: "ready", message: null,
  link: {
    visibility: "private", can_pull: true, repository_id: `repo_${kind}`, website_url: `https://mallard-ai.com/maya/${ITEM[kind].name}`,
    owner_login: "maya", name: ITEM[kind].name, release: { kind: "private" }, repository_revision: 4,
    can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner", can_push: true,
  },
  publication: { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: 1, public_version: null, review_version: null },
});

/* ——— the stand-in backend ——— */

const local = pick("state", ["local", "stored"], "local") === "local";
/* `repo=new`: on Mallard Cloud, an item with no repository yet — the page
   shows the one its first Push creates above the item. */
const connected = pick("repo", ["saved", "new"], "saved") === "saved";
const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
const kindOf = (scopeId: string) => scopeId.replace("scope-", "") as Kind;
storageCatalogApi.listScopeResources = async (scopeId, storageId) => {
  await wait(120);
  const kind = kindOf(scopeId);
  const scope = scopeOf(kind);
  const cloud = storageId === "storage-cloud";
  return {
    provider_label: scope.provider_label, kind_label: scope.kind_label, format_label: scope.format_label,
    scope: scope.scope, profile: scope.profile, storage_id: storageId ?? "storage-vault", storage_name: cloud ? "Mallard Cloud" : "Vault",
    link: { link_id: `link-${kind}`, revision: 3 }, storage_revision: cloud ? null : 7,
    rows: [rowOf(kind, local)], warnings: [], recovery: [],
    cloud_repositories: cloud && connected ? { [ITEM[kind].id]: `repo_${kind}` } : {},
    cloud_connections: cloud && connected ? { [ITEM[kind].id]: `repo_${kind}` } : {},
    cloud_status: null,
  };
};
storageCatalogApi.getDetail = async (scopeId) => {
  await wait(250);
  return detailOf(kindOf(scopeId), local);
};
storageCatalogApi.getPreview = async () => MOSSY_TILE;
storageCatalogApi.getCloudStatus = async (scopeId) => statusOf(kindOf(scopeId));
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};

/* ——— the harness ——— */

const PAD: Record<Width, number> = { "580": 28, "778": 38, "1118": 50 };

function Harness() {
  const [kind, setKind] = useState<Kind>(pick("kind", KINDS, "skill"));
  const [storage, setStorage] = useState<StorageKey>(pick<StorageKey>("storage", ["cloud", "vault"], "cloud"));
  const [width, setWidth] = useState<Width>(pick<Width>("width", ["580", "778", "1118"], "1118"));
  const [theme, setTheme] = useState(pick("theme", ["light", "dark"], "dark"));
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams({ kind, storage, width, theme, state: local ? "local" : "stored", repo: connected ? "saved" : "new" });
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [kind, storage, width, theme]);
  const seg = <T extends string>(value: T, current: T, set: (next: T) => void) => (
    <button key={value} type="button" className={value === current ? "btn btn-primary" : "btn"} onClick={() => set(value)}>{value}</button>
  );
  return (
    <div style={{ display: "grid", gap: 12, padding: 16 }}>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
        {KINDS.map((each) => seg(each, kind, setKind))}
        {(["cloud", "vault"] as StorageKey[]).map((each) => seg(each, storage, setStorage))}
        {(["580", "778", "1118"] as Width[]).map((each) => seg(each, width, setWidth))}
        {(["light", "dark"] as const).map((each) => seg(each, theme, setTheme))}
      </div>
      <div
        className="catalog-item-live-frame"
        style={{ width: Number(width), height: 760, display: "flex", overflow: "hidden", border: "1px dashed var(--border-strong)", borderRadius: 12, background: "var(--retro-frame-gutter)", "--live-pad": `${PAD[width]}px` } as CSSProperties}
      >
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0, height: "100%" }}>
          <StorageCatalogPage
            key={`${kind}-${storage}`}
            scope={scopeOf(kind)} scopes={KINDS.map(scopeOf)} storages={STORAGES} capabilities={KINDS.map((each) => CAPABILITY[each])}
            appActions={[]} initialStorageId={`storage-${storage}`} initialResourceId={ITEM[kind].id}
            onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
            onProfileRenamed={noop} onScopeGone={noop} notify={noop}
          />
        </div>
      </div>
      <style>{`html, body { height: 100%; overflow: hidden !important; } body { display: block !important; background: var(--bg-0) !important; } #root { height: 100%; overflow-y: auto !important; } .catalog-item-live-frame .sync-main { overflow-y: auto; padding-inline: var(--live-pad); }`}</style>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Harness />
  </StrictMode>,
);
