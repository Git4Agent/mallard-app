import { StrictMode, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import PageBack from "../src/components/sync-review/PageBack";
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
 * Mockup: the session page's back button, on a Catalog item's page.
 *
 * Every frame is the real `StorageCatalogPage` on a stand-in backend. 3 was
 * picked 2026-10-08 and shipped: its frame is the page as it ships. The
 * others hide the shipped button and seat the shared `PageBack` by a
 * mock-only portal.
 *
 *   Before  the crumbs are the only way back
 *   1       first on the page, above any card
 *   2       above the item, under any card (the session page's order)
 *   3       leads the facts line where the page has one (the session
 *           page's strip); its own row where it has none (shipped)
 *
 * /mockups/catalog-back-mockup.html
 * (`?kind=pet|skill&page=item|file&storage=cloud-new|cloud|vault&label=place|chevron&width=580|778|1118&theme=light|dark&only=before|1|2`).
 */

/* ——— knobs ——— */

type Kind = "skill" | "pet";
type Dir = "before" | "1" | "2" | "3";
type Width = "580" | "778" | "1118";
type StorageKnob = "cloud-new" | "cloud" | "vault";
interface Knobs {
  kind: Kind;
  page: "item" | "file";
  storage: StorageKnob;
  label: "place" | "chevron";
  width: Width;
  theme: "light" | "dark";
  only: Dir | "all";
}
const DIRS: readonly Dir[] = ["before", "1", "2", "3"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    kind: pick("kind", ["pet", "skill"] as const, "pet"),
    page: pick("page", ["item", "file"] as const, "item"),
    storage: pick("storage", ["cloud-new", "cloud", "vault"] as const, "cloud-new"),
    label: pick("label", ["place", "chevron"] as const, "place"),
    width: pick("width", ["580", "778", "1118"] as const, "778"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
  };
}

const DIR_TITLE: Record<Dir, string> = {
  before: "Before",
  "1": "1 · First on the page",
  "2": "2 · Under the cards",
  "3": "3 · Leads the facts line (shipped)",
};
/* One "seen in" and one against per direction, nothing else. */
const DIR_SEEN: Record<Dir, { seen: Array<[string, string]>; against: string | null }> = {
  before: { seen: [], against: "Crumbs read as places" },
  "1": {
    seen: [
      ["Primer PageHeader parent link", "https://primer.style/product/components/page-header/guidelines/"],
      ["Polaris Page", "https://shopify.dev/docs/api/app-home/polaris-web-components/layout-and-structure/page"],
    ],
    against: "Own row on narrow pages",
  },
  "2": {
    seen: [["Mallard session page (notice, then button)", "#"]],
    against: "Moves when a card appears",
  },
  "3": {
    seen: [["Mallard session page (button, then facts)", "#"]],
    against: "Moves when the rail opens",
  },
};

/* ——— the stand-in backend (from catalog-item-live) ——— */

const NOW = Math.floor(Date.now() / 1000);
const noop = () => {};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

const profileOf = (provider: "claude" | "codex"): CatalogProfile => ({
  profile_id: `profile-${provider}`, provider, provider_label: provider === "claude" ? "Claude" : "Codex",
  display_name: provider === "claude" ? "Default Claude" : "Default Codex", configured_home: `/Users/you/.${provider}`,
  revision: 1, created_at: NOW - 86_400, updated_at: NOW - 86_400,
});

const CAPABILITY: Record<Kind, CatalogCapabilityDescriptor> = {
  skill: {
    provider: "codex", provider_label: "Codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
    label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
    capture: "directory_tree", apply: "automatic", targets: ["user.skills"],
  },
  pet: {
    provider: "codex", provider_label: "Codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 1,
    label: "Pets v1", singular_label: "Pet v1", presentation: { icon: "pet", kind_label: "Pets", format_label: "Codex pet v1" },
    capture: "directory_tree", apply: "automatic", targets: ["desktop.pets"],
  },
};
const KINDS: Kind[] = ["pet", "skill"];

const scopeOf = (kind: Kind): CatalogScopeView => {
  const capability = CAPABILITY[kind];
  const profile = profileOf("codex");
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

const ITEM: Record<Kind, { id: string; name: string; folder: string; bytes: number }> = {
  skill: { id: "release-notes", name: "release-notes", folder: "/Users/you/.codex/skills/release-notes", bytes: 9_412 },
  pet: { id: "tracebit", name: "Tracebit", folder: "/Users/you/.codex/pets/tracebit", bytes: 65_536 },
};

const SKILL_MD = `---
name: release-notes
description: Drafts release notes from the commits since the last tag, grouped by area, in the house voice.
---

# Release notes

Write release notes for the range the user names, or for everything since the newest tag.

1. Find the range: \`git describe --tags --abbrev=0\` names the last tag.
2. Read \`git log --no-merges <tag>..HEAD\` and group the commits by area.
3. Draft one line per change in the [house voice](voice.md).
`;
const SKILL_FILES: Record<string, string> = {
  "SKILL.md": SKILL_MD,
  "voice.md": "# House voice\n\nShort sentences. Lead with the verb: *Adds*, *Fixes*, *Removes*.\n",
  "scripts/range.sh": "#!/bin/sh\nset -eu\ngit describe --tags --abbrev=0\n",
};

const detailOf = (kind: Kind): CatalogResourceDetail => kind === "skill"
  ? {
    kind: "document", path: "SKILL.md", text: SKILL_MD, files: Object.entries(SKILL_FILES).map(([path, text]) => ({
      path, size: new TextEncoder().encode(text).length, executable: path.endsWith(".sh"),
    })),
  }
  : { kind: "sprite", atlas: MOSSY_ATLAS, columns: 8, rows: 11, description: "A compact evidence-first debugging bot for OCI and Exascale work." };

const rowOf = (kind: Kind): CatalogScopeRow => ({
  row_id: `row-${ITEM[kind].id}`, display_name: ITEM[kind].name, resource_id: ITEM[kind].id, state: "local_changed",
  local_present: true, local_path: ITEM[kind].folder, local_modified_at: NOW - 3_600 * 3,
  local_files: 4, local_bytes: ITEM[kind].bytes,
  remote_present: true, remote_updated_at: NOW - 86_400 * 2, content_sha256: "a3f9".padEnd(64, "0"),
  preview_png: kind === "pet" ? MOSSY_TILE : null, description: null, warnings: [],
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

/** The knob the stand-in reads: whether a Cloud item has its repository yet. */
const backend = { connected: true };
const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
const kindOf = (scopeId: string) => scopeId.replace("scope-", "") as Kind;
storageCatalogApi.listScopeResources = async (scopeId, storageId) => {
  await wait(80);
  const kind = kindOf(scopeId);
  const scope = scopeOf(kind);
  const cloud = storageId === "storage-cloud";
  const saved = cloud && backend.connected ? { [ITEM[kind].id]: `repo_${kind}` } : {};
  return {
    provider_label: scope.provider_label, kind_label: scope.kind_label, format_label: scope.format_label,
    scope: scope.scope, profile: scope.profile, storage_id: storageId ?? "storage-vault", storage_name: cloud ? "Mallard Cloud" : "Vault",
    link: { link_id: `link-${kind}`, revision: 3 }, storage_revision: cloud ? null : 7,
    rows: [rowOf(kind)], warnings: [], recovery: [],
    cloud_repositories: saved, cloud_connections: saved, cloud_status: null,
  };
};
storageCatalogApi.getFile = async (_scopeId, _storageId, _resourceId, path) => {
  await wait(80);
  const text = SKILL_FILES[path];
  if (text === undefined) throw new Error("this file is not part of the resource");
  return { path, size: new TextEncoder().encode(text).length, executable: path.endsWith(".sh"), text, local_path: `${ITEM.skill.folder}/${path}` };
};
storageCatalogApi.getDetail = async (scopeId) => {
  await wait(120);
  return detailOf(kindOf(scopeId));
};
storageCatalogApi.getPreview = async () => MOSSY_TILE;
storageCatalogApi.getCloudStatus = async (scopeId) => statusOf(kindOf(scopeId));
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};

/* ——— the button, seated by direction ——————————————————————————————————
   The crumbs already walk back: the scope crumb from an item to the list,
   the item crumb from a file to the item. The button presses the same
   crumb, so every frame navigates for real. */

interface Target {
  voice: "app" | "name";
  label: string;
  crumb: string;
}

function readTarget(root: HTMLElement, scopeLabel: string): Target | null {
  const toItem = root.querySelector<HTMLElement>('[aria-label$=". Back to its page"]');
  if (toItem) {
    const name = toItem.getAttribute("aria-label")!.replace(/\. Back to its page$/, "");
    return { voice: "name", label: name, crumb: '[aria-label$=". Back to its page"]' };
  }
  if (root.querySelector('[aria-label$=". Back to the list"]')) {
    return { voice: "app", label: scopeLabel, crumb: '[aria-label$=". Back to the list"]' };
  }
  return null;
}

function Frame({ dir, knobs }: { dir: Dir; knobs: Knobs }) {
  const frameRef = useRef<HTMLDivElement>(null);
  const host = useMemo(() => {
    const node = document.createElement("div");
    node.className = "mk-back-host";
    return node;
  }, []);
  const [target, setTarget] = useState<Target | null>(null);
  const scope = scopeOf(knobs.kind);

  useLayoutEffect(() => {
    const root = frameRef.current;
    if (!root) return;
    let fileOpened = false;
    const sync = () => {
      // The file knob opens a skill's file once its list is on screen.
      if (knobs.page === "file" && knobs.kind === "skill" && !fileOpened) {
        const link = [...root.querySelectorAll<HTMLButtonElement>(".catalog-item-file")]
          .find((button) => button.textContent?.includes("voice.md"));
        if (link) {
          fileOpened = true;
          link.click();
        }
      }
      // 3 is the page as it ships; Before has no button; 1 and 2 seat one.
      const next = dir === "1" || dir === "2" ? readTarget(root, scope.label) : null;
      setTarget((current) => (current?.label === next?.label && current?.voice === next?.voice ? current : next));
      const parent = !next ? null
        : dir === "1"
          ? root.querySelector<HTMLElement>("main section.profile-links-section")
          : root.querySelector<HTMLElement>(".catalog-item-page");
      if (!parent) {
        if (host.parentElement) host.remove();
        return;
      }
      if (parent.firstChild !== host) parent.insertBefore(host, parent.firstChild);
    };
    const observer = new MutationObserver(sync);
    observer.observe(root, { childList: true, subtree: true });
    sync();
    return () => {
      observer.disconnect();
      host.remove();
    };
  }, [dir, host, knobs.page, knobs.kind, scope.label]);

  return (
    <div
      ref={frameRef}
      className={`mk-frame${dir === "3" ? "" : " mk-unshipped"}${knobs.label === "chevron" ? " mk-chevron" : ""}`}
      style={{ width: Number(knobs.width), "--live-pad": `${PAD[knobs.width]}px` } as CSSProperties}
    >
      <div className="mk-frame-column">
        <StorageCatalogPage
          scope={scope} scopes={KINDS.map(scopeOf)} storages={STORAGES} capabilities={KINDS.map((each) => CAPABILITY[each])}
          appActions={[]} initialStorageId={knobs.storage === "vault" ? "storage-vault" : "storage-cloud"} initialResourceId={ITEM[knobs.kind].id}
          onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
          onProfileRenamed={noop} onScopeGone={noop} notify={noop}
        />
      </div>
      {target && createPortal(
        <PageBack
          target={{
            voice: target.voice,
            label: target.label,
            onActivate: () => frameRef.current?.querySelector<HTMLElement>(target.crumb)?.click(),
          }}
        />,
        host,
      )}
    </div>
  );
}

/* ——— the page ——— */

/** The main column's side padding at each window: the frame is the bar and
 *  the page beside a sidebar. 778 is the 1100px default window. */
const PAD: Record<Width, number> = { "580": 28, "778": 38, "1118": 50 };

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    if (key === "kind" && value === "pet") next.page = "item";
    if (key === "page" && value === "file") next.kind = "skill";
    setKnobs(next);
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams({ ...next })}`);
  };
  useEffect(() => { document.documentElement.dataset.theme = knobs.theme; }, [knobs.theme]);
  backend.connected = knobs.storage !== "cloud-new";
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = knobs.only === "all" ? DIRS : [knobs.only];
  const remount = `${knobs.kind}-${knobs.page}-${knobs.storage}-${knobs.width}`;
  return (
    <div className="spec">
      <h1 className="spec-title">Catalog: a back button</h1>
      <div className="spec-controls">
        <div className="spec-group"><span>Item</span>{seg("kind", "pet", "Pet")}{seg("kind", "skill", "Skill")}</div>
        <div className="spec-group"><span>Page</span>{seg("page", "item", "Item")}{seg("page", "file", "Skill file")}</div>
        <div className="spec-group"><span>Storage</span>{seg("storage", "cloud-new", "Cloud, no repository yet")}{seg("storage", "cloud", "Cloud")}{seg("storage", "vault", "Folder")}</div>
        <div className="spec-group"><span>Label</span>{seg("label", "place", "‹ Place")}{seg("label", "chevron", "‹ only")}</div>
        <div className="spec-group"><span>Window</span>{seg("width", "580", "Minimum")}{seg("width", "778", "1100 px")}{seg("width", "1118", "1440 px")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <p className="spec-seen">
        Label · ‹ Place: <a href="#">Mallard session page</a> · against: repeats the crumb.
        {" "}‹ only: <a href="https://developer.apple.com/design/human-interface-guidelines/toolbars">HIG toolbars</a> (Back symbol, no word) · against: says not where.
      </p>
      <div className="spec-grid">
        {shown.map((dir) => (
          <div key={dir} data-dir={dir}>
            <h2 className="spec-col-title">{DIR_TITLE[dir]}</h2>
            <p className="spec-seen">
              {DIR_SEEN[dir].seen.map(([name, href], index) => (
                <span key={name}>{index > 0 && " · "}<a href={href}>{name}</a></span>
              ))}
              {DIR_SEEN[dir].against && <span className="spec-against">{DIR_SEEN[dir].seen.length > 0 && " · "}against: {DIR_SEEN[dir].against}</span>}
            </p>
            <Frame key={remount} dir={dir} knobs={knobs} />
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
