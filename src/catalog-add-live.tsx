import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "./components/Icons";
import AddCatalogScopeWorkspace from "./components/storage-catalog/AddCatalogScopeWorkspace";
import type { CatalogStorageOption } from "./components/storage-catalog/StorageCatalogPage";
import type {
  CatalogCapabilityDescriptor,
  CatalogProfile,
  CatalogProfileHolding,
  CatalogScopeDto,
} from "./components/storage-catalog/types";
import "./App.css";

/**
 * Live: the shipped Add Catalog resource — New project's two steps, one row
 * per pass — on a stand-in backend. Real `AddCatalogScopeWorkspace`, real
 * steps, real App.css; `mockIPC` answers the Catalog's commands the way the
 * desktop app would, with a little latency so the loading states show.
 * Dev server only: `npm run dev`, then /catalog-add-live.html
 * (`?profiles=one|two&storages=none|one|many&catalog=empty|some&door=any|skill|pet-v1&theme=dark`).
 */

/* ——— the machine ———————————————————————————————————————————————— */

const HOME = "/Users/you";
const NOW = Math.floor(Date.now() / 1000);

function skills(provider: string, providerLabel: string, targets: string[]): CatalogCapabilityDescriptor {
  return {
    provider, provider_label: providerLabel, resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
    label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
    capture: "directory_tree", apply: "automatic", targets,
  };
}
function plugins(provider: string, providerLabel: string): CatalogCapabilityDescriptor {
  return {
    provider, provider_label: providerLabel, resource_kind: "plugin_install_intent", format: "mallard.plugin-install-intent", format_version: 1,
    label: "Plugins", singular_label: "Plugin", presentation: { icon: "plugin", kind_label: "Plugins", format_label: "Plugin install intent" },
    capture: "provider_inventory", apply: "automatic", targets: ["user.plugins"],
  };
}
function pets(version: number): CatalogCapabilityDescriptor {
  return {
    provider: "codex", provider_label: "Codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: version,
    label: `Pets v${version}`, singular_label: `Pet v${version}`,
    presentation: { icon: version === 1 ? "pet" : `pet-v${version}`, kind_label: "Pets", format_label: `Codex pet v${version}` },
    capture: "directory_tree", apply: "automatic", targets: ["desktop.pets"],
  };
}
const CAPABILITIES: CatalogCapabilityDescriptor[] = [
  skills("codex", "Codex", ["provider-home.skills", "shared-user.skills"]),
  plugins("codex", "Codex"),
  pets(1),
  pets(2),
  skills("claude", "Claude", ["provider-home.skills"]),
  plugins("claude", "Claude"),
];

function profileOf(id: string, provider: string, providerLabel: string, name: string, home: string): CatalogProfile {
  return { profile_id: id, provider, provider_label: providerLabel, display_name: name, configured_home: home, revision: 1, created_at: NOW - 86_400, updated_at: NOW - 86_400 };
}

/* What each home holds, per capability label: count and age in seconds. */
const HOLDS: Record<string, Record<string, [number | null, number]>> = {
  "codex-default": { Skills: [12, 7_200], Plugins: [3, 259_200], "Pets v1": [0, 0], "Pets v2": [2, 604_800] },
  "claude-default": { Skills: [4, 86_400], Plugins: [null, 0] },
  "codex-work": { Skills: [1, 604_800], Plugins: [0, 0], "Pets v1": [0, 0], "Pets v2": [0, 0] },
  "custom": { Skills: [2, 345_600], Plugins: [0, 0], "Pets v1": [0, 0], "Pets v2": [0, 0] },
};

/* ——— knobs ———————————————————————————————————————————————————————— */

type Knobs = { profiles: "one" | "two"; storages: "none" | "one" | "many"; catalog: "empty" | "some"; door: "any" | "skill" | "pet-v1"; theme: "light" | "dark" };
function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    profiles: pick("profiles", ["one", "two"] as const, "one"),
    storages: pick("storages", ["none", "one", "many"] as const, "one"),
    catalog: pick("catalog", ["empty", "some"] as const, "empty"),
    door: pick("door", ["any", "skill", "pet-v1"] as const, "any"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}
const knobs = readKnobs();

const profiles: CatalogProfile[] = [
  profileOf("codex-default", "codex", "Codex", "Default Codex", `${HOME}/.codex`),
  ...(knobs.profiles === "two" ? [profileOf("codex-work", "codex", "Codex", "Work Codex", `${HOME}/work/.codex`)] : []),
  profileOf("claude-default", "claude", "Claude", "Default Claude", `${HOME}/.claude`),
];

const storage = (id: string, provider: string, name: string, provider_label: string, detail: string): CatalogStorageOption => ({
  id, provider, name, provider_label, icon: provider === "local" ? "drive" : "cloud", detail, detailIsPath: provider === "local",
  account_login: provider === "managed_cloud" ? "kai" : null,
});
const ALL_STORAGES = [
  storage("cloud", "managed_cloud", "Mallard Cloud", "Mallard Cloud", "kai"),
  storage("vault", "r2", "Vault", "Cloudflare R2", "team · vault"),
  storage("drive", "local", "Backup drive", "Local folder", "/Volumes/Backup/mallard"),
];
const storages = knobs.storages === "none" ? [] : knobs.storages === "one" ? ALL_STORAGES.slice(1, 2) : ALL_STORAGES;

function scopeDto(capability: CatalogCapabilityDescriptor, profile: CatalogProfile, scopeId: string): CatalogScopeDto {
  return {
    scope: { scope_id: scopeId, provider: capability.provider, profile_id: profile.profile_id, resource_kind: capability.resource_kind, format: capability.format, format_version: capability.format_version, revision: 1, created_at: NOW, updated_at: NOW },
    profile, label: capability.label, provider_label: capability.provider_label, kind_label: capability.presentation.kind_label, format_label: capability.presentation.format_label, available: true,
  };
}
const existing: CatalogScopeDto[] = knobs.catalog === "some" ? [scopeDto(CAPABILITIES[0], profiles[0], "scope-skills-codex")] : [];
const DOOR_KEYS: Record<Knobs["door"], string | null> = { any: null, skill: "agent_skill/agentskills.io.skill@1", "pet-v1": "desktop_pet/openai.codex-pet@1" };

/* ——— the stand-in backend ————————————————————————————————————————— */

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
let nextId = 1;
(window as unknown as { isTauri: boolean }).isTauri = true;
mockIPC(async (command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  switch (command) {
    case "plugin:path|resolve_directory": return HOME;
    case "plugin:dialog|open": return `${HOME}/agents/.codex`;
    case "list_catalog_resource_profiles": await wait(200); return profiles;
    case "list_catalog_capabilities": await wait(120); return CAPABILITIES;
    case "list_catalog_cloud_shared": await wait(200); return [];
    case "probe_catalog_resource_profile": {
      await wait(250);
      const path = String(args.path);
      const known = profiles.find((profile) => profile.configured_home === path);
      if (known || path === `${HOME}/agents/.codex` || path === `${HOME}/work/.claude`) {
        return { canonical_path: path, readable: true, existing_profile_id: known?.profile_id ?? null, suggested_name: known?.display_name ?? path.split("/").pop() };
      }
      throw new Error(`inspect catalog profile '${path}': No such file or directory`);
    }
    case "save_catalog_resource_profile": {
      await wait(200);
      const path = String(args.path);
      const known = profiles.find((profile) => profile.configured_home === path);
      if (known) return known;
      const saved = profileOf(`custom-${nextId++}`, String(args.provider), String(args.provider) === "codex" ? "Codex" : "Claude", path.split("/").pop() ?? "profile", path);
      profiles.push(saved);
      return saved;
    }
    case "rename_catalog_resource_profile": {
      await wait(120);
      const profile = profiles.find((candidate) => candidate.profile_id === args.profileId)!;
      profile.display_name = String(args.displayName);
      profile.revision += 1;
      return profile;
    }
    case "inspect_catalog_resource_profile": {
      await wait(900);
      const profile = profiles.find((candidate) => candidate.profile_id === args.profileId)!;
      const holds = HOLDS[profile.profile_id] ?? HOLDS.custom;
      const holdings: CatalogProfileHolding[] = CAPABILITIES.filter((capability) => capability.provider === profile.provider).map((capability) => {
        const [count, age] = holds[capability.label] ?? [0, 0];
        return {
          capability, count, newest_modified_at: count ? NOW - age : null, roots: capability.targets.map((target) => target.split(".")[0]),
          scope_id: existing.find((scope) => scope.scope.profile_id === profile.profile_id && scope.scope.resource_kind === capability.resource_kind && scope.scope.format_version === capability.format_version)?.scope.scope_id ?? null,
        };
      });
      return { profile, last_used_storage_id: knobs.catalog === "some" && storages.some((candidate) => candidate.id === "vault") ? "vault" : null, holdings };
    }
    case "create_catalog_scope": {
      await wait(250);
      const profile = profiles.find((candidate) => candidate.profile_id === args.profileId)!;
      const capability = CAPABILITIES.find((candidate) => (
        candidate.provider === args.provider && candidate.resource_kind === args.resourceKind && candidate.format === args.format && candidate.format_version === args.formatVersion
      ))!;
      if (existing.some((scope) => scope.scope.profile_id === profile.profile_id && scope.scope.resource_kind === capability.resource_kind && scope.scope.format_version === capability.format_version)) {
        throw new Error("this Catalog row already exists");
      }
      const created = scopeDto(capability, profile, `scope-${nextId++}`);
      existing.push(created);
      return created;
    }
    case "link_catalog_scope_storage": await wait(200); return { link_id: `link-${nextId++}`, revision: 1, completion: "open" };
    default: throw new Error(`unmocked command ${command}`);
  }
});

/* ——— the frame ———————————————————————————————————————————————————— */

function Live() {
  const [landed, setLanded] = useState<{ label: string; storage: string | null; linkError: string | null } | null>(null);
  const [epoch, setEpoch] = useState(0);
  return (
    <div className="sync-app" style={{ ["--sync-sidebar-width" as string]: "0px" }}>
      <div className="sync-workspace">
        <header className="sync-titlebar">
          <div className="sync-titlebar-context">
            <Icon name="catalog" size={16} />
            <strong>Add Catalog resource</strong>
            <span>live · {knobs.profiles === "two" ? "two Codex profiles" : "one profile each"} · {storages.length} storage{storages.length === 1 ? "" : "s"}{knobs.catalog === "some" ? " · Skills · Default Codex added" : ""}{knobs.door !== "any" ? ` · from the ${knobs.door === "skill" ? "Skills" : "Pets v1"} door` : ""}</span>
          </div>
        </header>
        {landed ? (
          <main className="sync-main sync-project-links-page sync-landing-page">
            <div className="sync-folder-choices-state" role="status">
              <Icon name="check-circle" size={14} />
              {landed.label} added to the Catalog{landed.storage ? ` · on ${landed.storage}` : " · no storage yet"}{landed.linkError ? ` · link failed: ${landed.linkError}` : ""}.
              <button type="button" className="btn" onClick={() => { setLanded(null); setEpoch((value) => value + 1); }}>Add another</button>
            </div>
          </main>
        ) : (
          <AddCatalogScopeWorkspace
            key={epoch}
            workflowEpoch={epoch}
            isWorkflowCurrent={(value) => value === epoch}
            busy={false}
            initialChoiceKey={DOOR_KEYS[knobs.door]}
            scopes={existing}
            storages={storages}
            cloudAccounts={storages.filter((candidate) => candidate.provider === "managed_cloud").map((candidate) => ({ id: candidate.id, name: candidate.name, account_login: candidate.account_login ?? "" }))}
            onSetUpCloud={() => undefined}
            onClose={() => setLanded({ label: "Nothing", storage: null, linkError: null })}
            onCreated={(scope, storageId, linkError) => setLanded({
              label: `${scope.label} · ${scope.profile.display_name}`,
              storage: storages.find((candidate) => candidate.id === storageId)?.name ?? null,
              linkError,
            })}
            onAdded={() => undefined}
          />
        )}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = knobs.theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Live /></StrictMode>);
