import { StrictMode, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import StorageCatalogPage, { type CatalogStorageOption } from "../src/components/storage-catalog/StorageCatalogPage";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import type {
  CatalogCapabilityDescriptor,
  CatalogLinkState,
  CatalogProfile,
  CatalogScopeRow,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import type { StatusScope } from "../src/components/sync-review/statusScopes";
import "../src/App.css";

/**
 * Library status filter — the shipped `StorageCatalogPage` (L1 strip) on a
 * stand-in backend. Presets are typed and clicked into the real page.
 * `?theme=light|dark&only=vault|cloud|empty|narrow`
 */

const params = new URLSearchParams(window.location.search);
const only = params.get("only");
const NOW = Math.floor(Date.now() / 1000);
const noop = () => {};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

/* ——— stand-in data ——— */

const profile: CatalogProfile = {
  profile_id: "profile-claude", provider: "claude", provider_label: "Claude", display_name: "Claude",
  configured_home: "/Users/you/.claude", revision: 1, created_at: NOW - 86_400, updated_at: NOW - 86_400,
};
const capability: CatalogCapabilityDescriptor = {
  provider: "claude", provider_label: "Claude", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
  label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
  capture: "directory_tree", apply: "automatic", targets: ["user.skills"],
};
const scope: CatalogScopeView = {
  scope: {
    scope_id: "scope-skill", profile_id: profile.profile_id, provider: "claude", resource_kind: capability.resource_kind,
    format: capability.format, format_version: 1, revision: 1, created_at: NOW, updated_at: NOW,
  },
  profile, label: "Skills", available: true, provider_label: "Claude", kind_label: "Skills", format_label: "Agent Skill", capability,
};
const STORAGES: CatalogStorageOption[] = [
  { id: "storage-vault", provider: "local_folder", name: "Vault", provider_label: "Local folder", icon: "storage-local", detail: "~/Mallard Vault", detailIsPath: true },
  { id: "storage-cloud", provider: "managed_cloud", name: "Mallard Cloud", provider_label: "Mallard Cloud", icon: "storage-mallard", detail: "maya · mallard-ai.com", detailIsPath: false, account_login: "maya" },
];

/* [name, state, on this Mac, in the storage] — every state the Library draws. */
const SPEC: [string, CatalogLinkState, boolean, boolean][] = [
  ["release-notes", "local_changed", true, true],
  ["pr-review", "new", true, false],
  ["changelog-voice", "remote_missing", true, false],
  ["sql-helper", "storage_changed", true, true],
  ["figma-to-code", "local_missing", false, true],
  ["design-tokens", "local_missing", false, true],
  ["ghost-notes", "local_missing", false, false],
  ["api-docs", "diverged", true, true],
  ["onboarding", "unreviewed_remote", true, true],
  ["secrets-scan", "blocked", true, true],
  ["commit-message", "in_sync", true, true],
  ["test-writer", "in_sync", true, true],
  ["lint-fix", "in_sync", true, true],
];
const ROWS: CatalogScopeRow[] = SPEC.map(([name, state, local, remote], index) => ({
  row_id: `row-${name}`, display_name: name, resource_id: name, state,
  local_present: local, local_path: local ? `/Users/you/.claude/skills/${name}` : null,
  local_modified_at: local ? NOW - 3_600 * (index + 2) : null, local_files: local ? 3 : null, local_bytes: local ? 4_200 + index * 310 : null,
  remote_present: remote, remote_updated_at: remote ? NOW - 86_400 * (index % 4 + 1) : null,
  content_sha256: `${index}`.padEnd(64, "0"), preview_png: null, description: null, warnings: [],
  blocker: state === "blocked" ? "Two folders claim this skill’s name." : null,
}));

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
storageCatalogApi.listScopeResources = async (_scopeId, storageId) => {
  await wait(80);
  const cloud = storageId === "storage-cloud";
  const repos = cloud ? Object.fromEntries(ROWS.filter((row) => row.remote_present).map((row) => [row.resource_id!, `repo_${row.resource_id}`])) : {};
  return {
    provider_label: "Claude", kind_label: "Skills", format_label: "Agent Skill",
    scope: scope.scope, profile, storage_id: storageId ?? "storage-vault", storage_name: cloud ? "Mallard Cloud" : "Vault",
    link: { link_id: "link-skill", revision: 3 }, storage_revision: cloud ? null : 7,
    rows: ROWS, warnings: [], recovery: [], cloud_repositories: repos, cloud_connections: repos, cloud_status: null,
  };
};
storageCatalogApi.getDetail = async () => ({
  kind: "document", path: "SKILL.md", text: "# A skill\n\nWhat it does.\n", files: [{ path: "SKILL.md", size: 26, executable: false }],
});
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};

/* ——— presets: the harness clicks the shipped strip, as a reader would ——— */

const SCOPE_LABEL: Record<StatusScope, string> = { push: "To push", pull: "To pull", attention: "Attention", synced: "Up to date", unknown: "Not compared" };

/** Once the list has loaded, type the search and press the scope's radio. */
function usePreset(frame: HTMLElement | null, scope: StatusScope | null, query: string) {
  useEffect(() => {
    if (!frame || (scope === null && !query)) return;
    const apply = () => {
      const strip = frame.querySelector<HTMLElement>(".sync-status-scopes");
      const input = frame.querySelector<HTMLInputElement>(".profile-links-section .sync-name-search input");
      if (!strip || !input) return false;
      if (query) {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, query);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (scope) {
        [...strip.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
          .find((radio) => radio.getAttribute("aria-label")?.startsWith(`${SCOPE_LABEL[scope]},`))?.click();
      }
      return true;
    };
    if (apply()) return;
    const observer = new MutationObserver(() => { if (apply()) observer.disconnect(); });
    observer.observe(frame, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [frame, scope, query]);
}

/* ——— frames ——— */

const PAD: Record<number, number> = { 580: 28, 778: 38, 1118: 50 };

function Frame({ cloud = false, width = 1118, height = 760, scope: preset = null, query = "" }: {
  cloud?: boolean;
  width?: number;
  height?: number;
  scope?: StatusScope | null;
  query?: string;
}) {
  const [frame, setFrame] = useState<HTMLElement | null>(null);
  usePreset(frame, preset, query);
  return (
    <div ref={setFrame} className="cf-frame" style={{ width, height, "--live-pad": `${PAD[width] ?? 50}px` } as CSSProperties}>
      <div className="cf-frame-inner">
        <StorageCatalogPage
          scope={scope} scopes={[scope]} storages={STORAGES} capabilities={[capability]}
          appActions={[]} initialStorageId={cloud ? "storage-cloud" : "storage-vault"}
          onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
          onProfileRenamed={noop} onScopeGone={noop} notify={noop}
        />
      </div>
    </div>
  );
}

function Part({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  if (only && only !== id) return null;
  return (
    <section className="cf-part" data-part={id}>
      <header><h2>{title}</h2></header>
      {children}
    </section>
  );
}

function Specimen() {
  const [theme, setTheme] = useState(params.get("theme") === "light" ? "light" : "dark");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  const switched = useRef(false);
  useEffect(() => {
    if (!switched.current) { switched.current = true; return; }
    params.set("theme", theme);
    window.history.replaceState(null, "", `?${params.toString()}`);
  }, [theme]);
  return (
    <div className="cf-page">
      {!only && (
        <div className="cf-top">
          <strong>Library status filter</strong>
          {(["light", "dark"] as const).map((each) => (
            <button key={each} type="button" className={each === theme ? "btn btn-primary" : "btn"} onClick={() => setTheme(each)}>{each}</button>
          ))}
        </div>
      )}
      <Part id="vault" title="Vault · All"><Frame /></Part>
      <Part id="cloud" title="Mallard Cloud · To pull"><Frame cloud scope="pull" /></Part>
      <Part id="empty" title="Attention + “release” · Clear filter"><Frame width={778} height={300} scope="attention" query="release" /></Part>
      <Part id="narrow" title="580 · To push"><Frame width={580} height={520} scope="push" /></Part>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
