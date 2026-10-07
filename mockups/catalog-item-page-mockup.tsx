import { Fragment, StrictMode, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon, { type IconName } from "../src/components/Icons";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { SessionDetailMarkdown } from "../src/components/project-sync/session-detail/SessionDetailEvent";
import StorageCatalogPage, { type CatalogStorageOption } from "../src/components/storage-catalog/StorageCatalogPage";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import { catalogPresentation, catalogScopeLabel, rowStatus, rowStatusLabel } from "../src/components/storage-catalog/model";
import type {
  CatalogCapabilityDescriptor,
  CatalogLinkState,
  CatalogProfile,
  CatalogScopeRow,
  CatalogScopeView,
} from "../src/components/storage-catalog/types";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatFullDateTime } from "../src/clock";
import { formatBytes } from "../src/format";
import { MOSSY_ATLAS, MOSSY_TILE } from "./catalog-item-page-art";
import "../src/App.css";
import "./catalog-item-page-mockup.css";

/**
 * A Catalog item's page in the desktop app, made like its page on Mallard
 * Cloud (docs/PLAN_CATALOG_ITEM_PAGE.md).
 *
 * Today is the shipped page, live: the real `StorageCatalogPage` on a
 * stand-in backend, opened on one item of Mallard Cloud — the only storage
 * whose items have a page today. The proposals are drawn on the shipped bar
 * (`SyncPathBar`, `ManagedCloudChip`), the shipped tokens and classes, and
 * the shipped Markdown renderer (`SessionDetailMarkdown`); what they add sits
 * under a `ci-` prefix in catalog-item-page-mockup.css. The skill, plugin and
 * pet bodies are the website's (mallardInternal web/src/components:
 * CatalogItem.tsx, PluginPage.tsx, PetPage.tsx), redrawn here.
 *
 * `npm run dev`, then /mockups/catalog-item-page-mockup.html
 * (`?part=all|today|frames|bodies|copy|widths|decide|refs&kind=skill|plugin|pet&width=580|778|1118&theme=light|dark&clean=1`).
 */

const params = new URLSearchParams(window.location.search);
function pick<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  const value = params.get(key);
  return value && (allowed as readonly string[]).includes(value) ? value as T : fallback;
}
const noop = () => {};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
const NOW = Math.floor(Date.now() / 1000);
const LOGIN = "maya";
const HOST = "mallard-ai.com";

/* ——— the machine ———————————————————————————————————————————————— */

type KindKey = "skill" | "plugin" | "pet";
const KINDS: readonly KindKey[] = ["skill", "plugin", "pet"];

const CLAUDE: CatalogProfile = {
  profile_id: "profile-claude", provider: "claude", provider_label: "Claude", display_name: "Claude",
  configured_home: "/Users/you/.claude", revision: 1, created_at: NOW - 86_400 * 40, updated_at: NOW - 86_400 * 40,
};
const CODEX: CatalogProfile = {
  profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Codex",
  configured_home: "/Users/you/.codex", revision: 1, created_at: NOW - 86_400 * 40, updated_at: NOW - 86_400 * 40,
};

const CAPABILITY: Record<KindKey, CatalogCapabilityDescriptor> = {
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

const scopeOf = (kind: KindKey): CatalogScopeView => {
  const capability = CAPABILITY[kind];
  const profile = kind === "pet" ? CODEX : CLAUDE;
  return {
    scope: {
      scope_id: `scope-${kind}`, profile_id: profile.profile_id, provider: capability.provider, resource_kind: capability.resource_kind,
      format: capability.format, format_version: capability.format_version, revision: 1, created_at: NOW - 86_400 * 30, updated_at: NOW - 86_400 * 30,
    },
    profile, label: capability.label, available: true,
    provider_label: capability.provider_label, kind_label: capability.presentation.kind_label, format_label: capability.presentation.format_label,
    capability,
  };
};
const SCOPES = KINDS.map(scopeOf);
const CAPABILITIES = KINDS.map((kind) => CAPABILITY[kind]);

const STORAGES: CatalogStorageOption[] = [
  {
    id: "storage-cloud", provider: "managed_cloud", name: "Mallard Cloud", provider_label: "Mallard Cloud", icon: "storage-mallard",
    detail: `${LOGIN} · ${HOST}`, detailIsPath: false, account_login: LOGIN,
  },
  {
    id: "storage-vault", provider: "local_folder", name: "Vault", provider_label: "Local folder", icon: "storage-local",
    detail: "~/Mallard Vault", detailIsPath: true,
  },
];

/* ——— the items ——————————————————————————————————————————————————— */

const SKILL_MD = `---
name: release-notes
description: Drafts release notes from the commits since the last tag, grouped by area, in the house voice. Use when someone asks for a changelog, release notes, or what changed.
allowed-tools: Bash(git log:*), Bash(git describe:*), Read
---

# Release notes

Write release notes for the range the user names, or for everything since the newest tag.

## Steps

1. Find the range: \`git describe --tags --abbrev=0\` names the last tag.
2. Read \`git log --no-merges <tag>..HEAD\` and group the commits by the area in their subject.
3. Draft one line per change in the house voice (see \`voice.md\`).
4. Put breaking changes first, under **Breaking**.

## Output

| Section | What goes in it |
| --- | --- |
| Breaking | Anything that needs action from users |
| Added | New features |
| Fixed | Bug fixes users would notice |

Keep it under 300 words. Never invent a change that is not in the log.
`;

const SKILL_FILES: ReadonlyArray<{ path: string; size: number; mode: number }> = [
  { path: "SKILL.md", size: 1_644, mode: 0o644 },
  { path: "voice.md", size: 3_120, mode: 0o644 },
  { path: "templates/notes.md", size: 2_204, mode: 0o644 },
  { path: "scripts/range.sh", size: 2_444, mode: 0o755 },
];

const VOICE_MD = `# House voice

Short sentences. Name what changed for the reader, not what the code did.

- Lead with the verb: *Adds*, *Fixes*, *Removes*.
- One change per line; no "various improvements".
`;

const RANGE_SH = `#!/bin/sh
# The range release notes cover: the newest tag to HEAD.
set -eu
tag=$(git describe --tags --abbrev=0 2>/dev/null || true)
if [ -z "$tag" ]; then
  git rev-list --max-parents=0 HEAD | tail -n 1
else
  echo "$tag"
fi
`;

const PLUGIN = {
  id: "lint-buddy@acme-tools",
  name: "lint-buddy",
  marketplace: "acme-tools",
  source: "acme-tools/claude-plugins",
  listing: {
    display_name: "Lint Buddy",
    developer: "Acme Tools",
    category: "Code quality",
    description: "Runs your project's linters after every edit and explains each finding in plain words, with a fix you can accept.",
    links: [["Website", "https://example.com/lint-buddy"], ["Privacy", "https://example.com/privacy"], ["Terms", "https://example.com/terms"]] as Array<[string, string]>,
  },
  installed: "1.4.0",
};

/** The one item each kind's page opens on. */
const ITEM: Record<KindKey, { id: string; name: string; folder: string; files: number; bytes: number; description: string }> = {
  skill: {
    id: "release-notes", name: "release-notes", folder: "/Users/you/.claude/skills/release-notes", files: 4, bytes: 9_412,
    description: "Drafts release notes from the commits since the last tag, grouped by area, in the house voice. Use when someone asks for a changelog, release notes, or what changed.",
  },
  plugin: {
    id: PLUGIN.id, name: PLUGIN.name, folder: "/Users/you/.claude/plugins/cache/acme-tools/lint-buddy/1.4.0", files: 0, bytes: 0,
    description: PLUGIN.listing.description,
  },
  pet: {
    id: "mossy", name: "Mossy", folder: "/Users/you/.codex/pets/mossy", files: 2, bytes: 6_034,
    description: "A mallard who naps on the terminal and paddles when a build is running.",
  },
};

/** The rows the list shows around the item, for the inspector and sheet. */
const NEIGHBOURS: Record<KindKey, string[]> = {
  skill: ["commit-style", "pdf-tools", "release-notes", "sql-review"],
  plugin: ["code-review", "lint-buddy", "slack-notify"],
  pet: ["Biscuit", "Mossy", "Pip"],
};

const rowOf = (kind: KindKey, state: CatalogLinkState = "local_changed"): CatalogScopeRow => {
  const item = ITEM[kind];
  const local = state !== "local_missing";
  return {
    row_id: `row-${item.id}`, display_name: item.name, resource_id: item.id, state,
    local_present: local, local_path: local ? item.folder : null, local_modified_at: local ? NOW - 3_600 * 3 : null,
    local_files: kind === "plugin" || !local ? null : item.files, local_bytes: kind === "plugin" || !local ? null : item.bytes,
    remote_present: state !== "new", remote_updated_at: state !== "new" ? NOW - 86_400 * 2 : null, content_sha256: "a3f9".padEnd(64, "0"),
    preview_png: kind === "pet" ? MOSSY_TILE : null, description: kind === "pet" ? item.description : null, warnings: [],
  };
};

const statusOf = (kind: KindKey): ManagedCloudStatus => {
  const item = ITEM[kind];
  const repositoryId = `repo_${LOGIN}_${item.name}`;
  return {
    repository_id: repositoryId, state: "ready", message: null,
    link: {
      visibility: kind === "pet" ? "public" : "private", can_pull: true, repository_id: repositoryId,
      website_url: `https://${HOST}/${LOGIN}/${item.name}`, owner_login: LOGIN, name: item.name,
      release: kind === "pet" ? { kind: "public" } : { kind: "private" }, repository_revision: 4,
      can_request_release: kind !== "pet", can_publish_version: true, can_discard_draft: true, role: "owner", can_push: true,
    },
    publication: { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: 1, public_version: kind === "pet" ? 2 : null, review_version: null },
  };
};

/* ——— the stand-in backend, for Today ——————————————————————————————— */

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
const kindOfScope = (scopeId: string): KindKey => scopeId.replace("scope-", "") as KindKey;
storageCatalogApi.listScopeResources = async (scopeId, storageId) => {
  await wait(200);
  const kind = kindOfScope(scopeId);
  const scope = scopeOf(kind);
  const cloud = (storageId ?? "storage-cloud") === "storage-cloud";
  const item = ITEM[kind];
  return {
    provider_label: scope.provider_label, kind_label: scope.kind_label, format_label: scope.format_label,
    scope: scope.scope, profile: scope.profile,
    storage_id: storageId ?? "storage-cloud", storage_name: cloud ? "Mallard Cloud" : "Vault",
    link: { link_id: `link-${kind}`, revision: 3 }, storage_revision: cloud ? null : 7,
    rows: [rowOf(kind)], warnings: [], recovery: [],
    cloud_repositories: cloud ? { [item.id]: `repo_${LOGIN}_${item.name}` } : {},
    cloud_connections: cloud ? { [item.id]: `repo_${LOGIN}_${item.name}` } : {},
    cloud_status: null,
  };
};
storageCatalogApi.getPreview = async () => MOSSY_TILE;
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};
storageCatalogApi.listProfiles = async () => [CLAUDE, CODEX];
storageCatalogApi.listCapabilities = async () => CAPABILITIES;
storageCatalogApi.getCloudStatus = async (scopeId) => {
  await wait(250);
  return statusOf(kindOfScope(scopeId));
};

/** The shipped page, live, opened on the kind's one item. */
function TodayPage({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-app">
      <StorageCatalogPage
        key={kind}
        scope={scopeOf(kind)} scopes={SCOPES} storages={STORAGES} capabilities={CAPABILITIES}
        appActions={[]} initialStorageId="storage-cloud" initialResourceId={ITEM[kind].id}
        onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
        onProfileRenamed={noop} onScopeGone={noop} notify={noop}
      />
    </div>
  );
}

/* ——— shared pieces of the proposal ——————————————————————————————— */

type Width = "580" | "778" | "1118";
const WIDTHS: Record<Width, { pad: number; window: string }> = {
  "580": { pad: 28, window: "the floor (an 800px window, or a wide sidebar)" },
  "778": { pad: 38, window: "a 1100px window, the default" },
  "1118": { pad: 50, window: "a 1440px window" },
};

/** The window's content column: the shipped bar, then the page under it. */
function Frame({ kind, width, leaf = true, height, children }: { kind: KindKey; width: Width; leaf?: boolean; height?: number; children: ReactNode }) {
  return (
    <div className="ci-frame" style={{ width: Number(width), height, "--ci-pad": `${WIDTHS[width].pad}px` } as CSSProperties}>
      <div className="ci-app">
        <Bar kind={kind} leaf={leaf} />
        <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
          {children}
        </main>
      </div>
    </div>
  );
}

function Bar({ kind, leaf }: { kind: KindKey; leaf: boolean }) {
  const scope = scopeOf(kind);
  const glyph = catalogPresentation(scope.capability);
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root", title: `Mallard Cloud · ${LOGIN} · ${HOST}` },
    { id: "scope", label: catalogScopeLabel(scope), icon: glyph.icon, onActivate: leaf ? noop : undefined, ariaLabel: leaf ? `${catalogScopeLabel(scope)}. Back to the list` : undefined },
    ...(leaf ? [{
      id: "item",
      label: ITEM[kind].name,
      leading: kind === "pet" ? <img className="catalog-leaf-mark" src={MOSSY_TILE} alt="" /> : <Icon name={glyph.icon} size={14} />,
    }] : []),
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      ariaLabel={`${catalogScopeLabel(scope)} path`}
      actions={(
        <>
          {leaf && (
            <ManagedCloudChip
              id={`cloud-${kind}`} status={statusOf(kind)} storageName="Mallard Cloud" busy={false} refreshing={false}
              onOpenUrl={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop}
            />
          )}
          <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync" disabled>
              <Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span>
            </button>
            <button type="button" className="sync-path-sync">
              <Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span>
              <span className="sync-path-count" aria-hidden="true">1</span>
            </button>
          </div>
        </>
      )}
    />
  );
}

const KIND_WORD: Record<KindKey, string> = { skill: "Skill", plugin: "Plugin", pet: "Desktop pet" };

/** The item's mark: the kind's glyph on its tile; a pet wears its idle frame. */
function KindTile({ kind, size = "lg" }: { kind: KindKey; size?: "lg" | "md" | "sm" }) {
  const glyph = catalogPresentation(CAPABILITY[kind]);
  return (
    <span className={`ci-tile ci-tile-${size} ci-tile-${kind}`} aria-hidden="true">
      {kind === "pet" ? <img src={MOSSY_TILE} alt="" /> : <Icon name={glyph.icon} size={size === "lg" ? 26 : size === "md" ? 20 : 14} />}
    </span>
  );
}

function StatusChip({ kind, state = "local_changed" }: { kind: KindKey; state?: CatalogLinkState }) {
  const row = rowOf(kind, state);
  return (
    <span className="ci-chip" title={rowStatusLabel(row, "Mallard Cloud")}>
      <ThreadStatusIcon status={rowStatus(row)} size={15} />
      {state === "in_sync" ? "Up to date" : state === "local_changed" ? "Changed here" : state === "storage_changed" ? "Changed in Mallard Cloud" : state === "diverged" ? "Changed on both" : state === "local_missing" ? "Not downloaded" : "Not pushed"}
    </span>
  );
}

function ProviderChip({ kind }: { kind: KindKey }) {
  const profile = kind === "pet" ? CODEX : CLAUDE;
  return (
    <span className="ci-chip">
      <ProviderMark provider={profile.provider} displayName={profile.provider_label} size={13} />
      {profile.provider_label}
    </span>
  );
}

/** The card's launch run, in the page's head: the provider app named, the
 *  rest as the card's own icon buttons. */
function LaunchActions({ kind, labelled = false }: { kind: KindKey; labelled?: boolean }) {
  const profile = kind === "pet" ? CODEX : CLAUDE;
  const forkable = kind !== "plugin";
  const ghost = (icon: IconName, label: string) => (
    <button type="button" className={labelled ? "btn ci-act" : "btn btn-ghost sync-history-launch-action"} title={label} aria-label={label}>
      <Icon name={icon} size={14} />{labelled && <span>{label}</span>}
    </button>
  );
  return (
    <div className="ci-actions">
      <button type="button" className="btn ci-act" title={`Open in ${profile.provider_label}`}>
        <ProviderMark provider={profile.provider} displayName={profile.provider_label} size={14} />
        <span>Open in {profile.provider_label}</span>
      </button>
      {ghost("terminal", "Open in Terminal")}
      {forkable && ghost("git-fork", `Fork ${ITEM[kind].name}`)}
      {ghost("folder", "Reveal in Finder")}
    </div>
  );
}

function ItemHeader({ kind, state = "local_changed", labelled = false }: { kind: KindKey; state?: CatalogLinkState; labelled?: boolean }) {
  return (
    <header className="ci-item-head">
      <KindTile kind={kind} />
      <div className="ci-item-title">
        <h1>{kind === "plugin" ? PLUGIN.listing.display_name : ITEM[kind].name}</h1>
        <div className="ci-item-sub">
          <StatusChip kind={kind} state={state} />
          <span className="ci-chip">{KIND_WORD[kind]}</span>
          <ProviderChip kind={kind} />
        </div>
      </div>
      <LaunchActions kind={kind} labelled={labelled} />
    </header>
  );
}

/** Which copy the body shows — the desktop's answer to the website's Draft ▾:
 *  this Mac's, or the storage's. Off when the two are the same. */
function CopySwitch({ state = "local_changed", shown = "mac", onShow }: { state?: CatalogLinkState; shown?: "mac" | "storage"; onShow?: (next: "mac" | "storage") => void }) {
  const same = state === "in_sync";
  const local = state !== "local_missing";
  const remote = state !== "new";
  const sentence = rowStatusLabel(rowOf("skill", state), "Mallard Cloud");
  return (
    <div className="ci-panel-head">
      {same || !local || !remote ? (
        <span className="ci-copy-one">
          <Icon name={!local ? "cloud" : "computer"} size={14} />
          <b>{!local ? "Mallard Cloud · draft" : same ? "This Mac = Mallard Cloud" : "This Mac"}</b>
        </span>
      ) : (
        <div className="sync-segmented ci-copy" role="tablist" aria-label="Which copy">
          <button type="button" role="tab" aria-selected={shown === "mac"} className={shown === "mac" ? "active" : undefined} onClick={() => onShow?.("mac")}>
            <Icon name="computer" size={13} /><span>This Mac</span>
          </button>
          <button type="button" role="tab" aria-selected={shown === "storage"} className={shown === "storage" ? "active" : undefined} onClick={() => onShow?.("storage")}>
            <Icon name="cloud" size={13} /><span>Mallard Cloud · draft</span>
          </button>
        </div>
      )}
      <span className="ci-panel-status">{sentence}</span>
    </div>
  );
}

/* ——— the facts ——— */

const exact = (seconds: number) => formatFullDateTime(seconds);
const tilde = (path: string) => path.replace(/^\/Users\/you/, "~");

function About({ kind }: { kind: KindKey }) {
  return (
    <section className="ci-card ci-side-card">
      <h3>About</h3>
      {/* The description is said once, in the body, which every width
          shows; the rail folds away below 880px. */}
      <div className="ci-chips">
        <span className="ci-chip">{KIND_WORD[kind]}</span>
        <ProviderChip kind={kind} />
      </div>
      <ul className="ci-stats">
        {kind === "plugin" ? (
          <li><Icon name="check-circle" size={13} /><b>{PLUGIN.installed}</b> installed</li>
        ) : (
          <>
            <li><Icon name="file" size={13} /><b>{ITEM[kind].files}</b> files</li>
            <li><Icon name="drive" size={13} /><b>{formatBytes(ITEM[kind].bytes)}</b> on disk</li>
          </>
        )}
        {kind === "pet" && <li><Icon name="catalog-pet-v2" size={13} />Format <b>v2</b> · 11 rows</li>}
      </ul>
    </section>
  );
}

function ThisMac({ kind }: { kind: KindKey }) {
  return (
    <section className="ci-card ci-side-card">
      <h3><Icon name="computer" size={15} />This Mac</h3>
      <dl className="ci-facts">
        <dt>Folder</dt>
        <dd>
          <code className="ci-path" title={ITEM[kind].folder}>{tilde(ITEM[kind].folder)}</code>
        </dd>
        <dt>Edited</dt>
        <dd><time>{exact(NOW - 3_600 * 3)}</time></dd>
        {kind === "plugin" && (<><dt>Version</dt><dd>{PLUGIN.installed}</dd></>)}
      </dl>
      <button type="button" className="btn ci-act ci-side-more"><Icon name="folder" size={13} /><span>Reveal in Finder</span></button>
    </section>
  );
}

function InStorage({ kind }: { kind: KindKey }) {
  const status = statusOf(kind);
  return (
    <section className="ci-card ci-side-card">
      <h3><Icon name="storage-mallard" size={15} />Mallard Cloud</h3>
      <dl className="ci-facts">
        <dt>Repository</dt>
        <dd><code>{LOGIN}/{ITEM[kind].name}</code><span className={`ci-pill ${status.link!.visibility}`}>{status.link!.visibility}</span></dd>
        <dt>Draft</dt>
        <dd>1 push past version 2</dd>
        <dt>Pushed</dt>
        <dd><time>{exact(NOW - 86_400 * 2)}</time></dd>
      </dl>
      <a className="ci-side-more ci-link" href={status.link!.website_url} onClick={(event) => event.preventDefault()}>
        Open on {HOST}<Icon name="external-link" size={11} />
      </a>
    </section>
  );
}

function Identity({ kind }: { kind: KindKey }) {
  const capability = CAPABILITY[kind];
  return (
    <details className="ci-identity">
      <summary><Icon name="chevron-right" size={11} />Identity</summary>
      <dl className="ci-facts">
        <dt>ID</dt>
        <dd><code>{ITEM[kind].id}</code></dd>
        <dt>Coordinate</dt>
        <dd>{KIND_WORD[kind]} · {capability.format} v{capability.format_version}</dd>
        <dt>Digest</dt>
        <dd><code title={"a3f9".padEnd(64, "0")}>a3f9000000000000</code></dd>
      </dl>
    </details>
  );
}

function Rail({ kind }: { kind: KindKey }) {
  return (
    <aside className="ci-side">
      <About kind={kind} />
      <ThisMac kind={kind} />
      <InStorage kind={kind} />
      <Identity kind={kind} />
    </aside>
  );
}

/** One run of fact chips under the head, with Identity behind a disclosure —
 *  the session page's chip strip, made for an item. */
function FactStrip({ kind }: { kind: KindKey }) {
  const status = statusOf(kind);
  const chip = (icon: IconName, text: ReactNode, title: string) => (
    <span className="ci-fact" title={title}><Icon name={icon} size={13} />{text}</span>
  );
  return (
    <div className="ci-strip">
      {kind === "plugin"
        ? chip("check-circle", <>Installed · <b>{PLUGIN.installed}</b></>, `Installed in Claude on this Mac, version ${PLUGIN.installed}`)
        : <>
          {chip("file", <b>{ITEM[kind].files}</b>, `${ITEM[kind].files} files`)}
          {chip("drive", <b>{formatBytes(ITEM[kind].bytes)}</b>, "On disk")}
        </>}
      {chip("clock", <time>{exact(NOW - 3_600 * 3)}</time>, "Edited on this Mac")}
      {chip("storage-mallard", <><code>{LOGIN}/{ITEM[kind].name}</code><span className={`ci-pill ${status.link!.visibility}`}>{status.link!.visibility}</span></>, "Its repository on Mallard Cloud")}
      {chip("cloud", <>Version 2 <span className="ci-muted">+1 push</span></>, "The draft is 1 push past version 2")}
    </div>
  );
}

/* ——— the skill ——— */

/** The website's frontmatter split (CatalogItem.tsx), as it reads SKILL.md. */
function splitFrontmatter(text: string): { fields: Record<string, string>; body: string } {
  const lines = text.split(/\r?\n/);
  const close = lines[0] === "---" ? lines.findIndex((line, index) => index > 0 && line.trimEnd() === "---") : -1;
  if (close === -1) return { fields: {}, body: text };
  const fields: Record<string, string> = {};
  for (let index = 1; index < close; index++) {
    const match = /^([A-Za-z0-9_-]+):(.*)$/.exec(lines[index]!);
    if (match && match[2]!.trim()) fields[match[1]!] = match[2]!.trim();
  }
  return { fields, body: lines.slice(close + 1).join("\n") };
}
const SKILL = splitFrontmatter(SKILL_MD);

function Markdown({ text }: { text: string }) {
  return <div className="session-detail-event-copy ci-md"><SessionDetailMarkdown text={text} /></div>;
}

function FileTable({ onOpen, open }: { onOpen?: (path: string) => void; open?: string }) {
  return (
    <table className="ci-table">
      <thead><tr><th>Path</th><th>Size</th><th>Mode</th></tr></thead>
      <tbody>
        {SKILL_FILES.map((file) => (
          <tr key={file.path} className={open === file.path ? "is-open" : undefined}>
            <td>{onOpen ? <button type="button" className="ci-file-link" onClick={() => onOpen(file.path)}><Icon name="file" size={13} /><code>{file.path}</code></button> : <code>{file.path}</code>}</td>
            <td>{formatBytes(file.size)}</td>
            <td><code className={file.mode === 0o755 ? "ci-exec" : undefined} title={file.mode === 0o755 ? "Executable" : undefined}>{file.mode === 0o755 ? "0755" : "0644"}</code></td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export type SkillBodyKey = "s1" | "s2" | "s3";

/** S1 the website's; S2 the frontmatter as fields; S3 the files beside what they say. */
function SkillBody({ variant = "s1", files = true }: { variant?: SkillBodyKey; files?: boolean }) {
  const [open, setOpen] = useState("SKILL.md");
  if (variant === "s3") {
    const text = open === "SKILL.md" ? null : open === "voice.md" ? VOICE_MD : open === "scripts/range.sh" ? RANGE_SH : "# Release {{version}}\n\n## Breaking\n\n## Added\n\n## Fixed\n";
    return (
      <div className="ci-skill">
        <p className="ci-lede">{SKILL.fields.description}</p>
        <div className="ci-browser">
          <ul className="ci-tree" aria-label="Files">
            {SKILL_FILES.map((file) => (
              <li key={file.path}>
                <button type="button" aria-current={open === file.path ? "true" : undefined} onClick={() => setOpen(file.path)}>
                  <Icon name="file" size={13} /><span>{file.path}</span>
                </button>
              </li>
            ))}
          </ul>
          <div className="ci-viewer">
            <div className="ci-viewer-head"><code>{open}</code><span>{formatBytes(SKILL_FILES.find((file) => file.path === open)!.size)}</span></div>
            {open === "SKILL.md" ? <Markdown text={SKILL.body} />
              : open.endsWith(".md") ? <Markdown text={text!} />
                : <pre className="ci-code"><code>{text}</code></pre>}
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="ci-skill">
      {variant === "s1" && <p className="ci-lede">{SKILL.fields.description}</p>}
      {variant === "s2" && (
        <dl className="ci-frontmatter" aria-label="SKILL.md frontmatter">
          {Object.entries(SKILL.fields).map(([key, value]) => (
            <Fragment key={key}><dt><code>{key}</code></dt><dd>{value}</dd></Fragment>
          ))}
        </dl>
      )}
      <div className="ci-inner"><Markdown text={SKILL.body} /></div>
      {files && (
        <>
          <h3 className="ci-sub">Files</h3>
          <FileTable />
        </>
      )}
    </div>
  );
}

/* ——— the plugin ——— */

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <button type="button" className="btn ci-copy-btn" aria-label={`Copy ${text}`} onClick={() => setCopied(true)}>
      <Icon name={copied ? "check" : "copy"} size={13} />{copied ? "Copied" : "Copy"}
    </button>
  );
}

const INSTALL_WAYS = [
  { key: "app", label: "Claude Code", prompt: ">", lines: [`/plugin install ${PLUGIN.name} --marketplace ${PLUGIN.source}`] },
  { key: "terminal", label: "Terminal", prompt: "$", lines: [`claude plugin marketplace add ${PLUGIN.source}`, `claude plugin install ${PLUGIN.name}@${PLUGIN.marketplace}`] },
];

function Install() {
  const [key, setKey] = useState("app");
  const shown = INSTALL_WAYS.find((way) => way.key === key)!;
  return (
    <div className="ci-install">
      <div className="ci-install-head">
        <div className="sync-segmented" role="tablist" aria-label="Install with">
          {INSTALL_WAYS.map((way) => (
            <button key={way.key} type="button" role="tab" aria-selected={way.key === key} className={way.key === key ? "active" : undefined} onClick={() => setKey(way.key)}>{way.label}</button>
          ))}
        </div>
        <CopyButton text={shown.lines.join("\n")} />
      </div>
      <pre className="ci-lines"><code>
        {shown.lines.map((line) => <span key={line}><span className="ci-prompt" aria-hidden="true">{shown.prompt} </span>{line}{"\n"}</span>)}
      </code></pre>
    </div>
  );
}

type PluginState = "installed" | "absent" | "no-listing";

/** The website's store hero (PluginPage.tsx, the owner's "C · Store hero + C2 ·
 *  Tabs"), with the one line only this Mac can say: whether it is installed. */
function PluginBody({ state = "installed", named = false, hero = true }: { state?: PluginState; named?: boolean; hero?: boolean }) {
  const listing = state === "no-listing" ? null : PLUGIN.listing;
  return (
    <div className="ci-plugin">
      {/* The page's head already names it; the name line is the website's,
          where the head names the repository instead. */}
      {named && (
        <p className="ci-plugin-name">
          <b>{listing?.display_name ?? PLUGIN.name}</b>
          {!listing && <span>@{PLUGIN.marketplace}</span>}
        </p>
      )}
      {hero && listing && <p className="ci-plugin-by">by <b>{listing.developer}</b> · {listing.category}</p>}
      {hero && listing && <p className="ci-lede">{listing.description}</p>}
      <p className={`ci-here${state === "absent" ? " is-absent" : ""}`}>
        {state === "absent" ? (
          <><Icon name="cloud" size={14} />Not installed on this Mac. <b>Pull</b> installs it in Claude.</>
        ) : (
          <><Icon name="check-circle" size={14} />Installed in Claude on this Mac · version {PLUGIN.installed}</>
        )}
      </p>
      <Install />
      <dl className="ci-facts ci-plugin-facts">
        <dt>ID</dt>
        <dd><code>{PLUGIN.id}</code></dd>
        <dt>Works with</dt>
        <dd><ProviderChip kind="plugin" /></dd>
        <dt>Source</dt>
        <dd><a className="ci-link" href="#" onClick={(event) => event.preventDefault()}><Icon name="link" size={13} />{PLUGIN.source}<Icon name="external-link" size={10} /></a></dd>
        {listing && (
          <>
            <dt>Links</dt>
            <dd className="ci-links">{listing.links.map(([label, href]) => <a key={label} className="ci-link" href={href} onClick={(event) => event.preventDefault()}>{label}<Icon name="external-link" size={10} /></a>)}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

/* ——— the pet ——— */

/* The website's player (web/src/pet.ts + PetPage.tsx), redrawn: cells of
   192 × 208, one row per state; v2 adds two look rows. */
const CELL_W = 192;
const CELL_H = 208;
type Moment = "idle" | "running" | "waiting" | "review" | "failed";
const STATES: Record<Moment | "running-right" | "running-left", { row: number; durations: number[] }> = {
  idle: { row: 0, durations: [280, 110, 110, 140, 140, 320] },
  "running-right": { row: 1, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  "running-left": { row: 2, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  failed: { row: 5, durations: [140, 140, 140, 140, 140, 140, 140, 240] },
  waiting: { row: 6, durations: [150, 150, 150, 150, 150, 260] },
  running: { row: 7, durations: [120, 120, 120, 120, 120, 220] },
  review: { row: 8, durations: [150, 150, 150, 150, 150, 280] },
};
const MOMENTS: ReadonlyArray<{ state: Moment; label: string }> = [
  { state: "idle", label: "Idle" },
  { state: "running", label: "Working" },
  { state: "waiting", label: "Needs you" },
  { state: "review", label: "Review" },
  { state: "failed", label: "Failed" },
];

function useFrame(key: keyof typeof STATES, playing: boolean): number {
  const state = STATES[key];
  const [at, setAt] = useState({ key, frame: 0 });
  const frame = at.key === key ? at.frame % state.durations.length : 0;
  useEffect(() => {
    if (!playing) return;
    const timer = window.setTimeout(() => setAt({ key, frame: (frame + 1) % state.durations.length }), state.durations[frame]);
    return () => window.clearTimeout(timer);
  }, [key, frame, playing, state.durations]);
  return frame;
}

function Sprite({ state, playing, scale = 1 }: { state: keyof typeof STATES; playing: boolean; scale?: number }) {
  const frame = useFrame(state, playing);
  return (
    <span
      className="ci-sprite"
      aria-hidden="true"
      style={{
        width: CELL_W * scale, height: CELL_H * scale,
        backgroundImage: `url(${MOSSY_ATLAS})`, backgroundSize: `${8 * CELL_W * scale}px ${11 * CELL_H * scale}px`,
        backgroundPosition: `${-frame * CELL_W * scale}px ${-STATES[state].row * CELL_H * scale}px`,
      }}
    />
  );
}

function CodexWindow({ moment }: { moment: Moment }) {
  return (
    <div className="ci-win" aria-hidden="true">
      <div className="ci-win-bar"><i /><i /><i /></div>
      <div className="ci-win-body">
        <span className="ci-win-line" style={{ width: "58%" }} />
        <span className="ci-win-line" style={{ width: "74%" }} />
        <span className="ci-win-line" style={{ width: "36%" }} />
        {moment === "running" && <span className="ci-win-dots"><i /><i /><i /></span>}
        {moment === "waiting" && <span className="ci-win-ask"><span className="ci-win-line" style={{ width: "64%" }} /><span className="ci-win-btns"><b>Allow</b><b>Deny</b></span></span>}
        {moment === "review" && <span className="ci-win-diff"><i className="is-del" /><i className="is-add" /><i className="is-add" /></span>}
        {moment === "failed" && <span className="ci-win-fail"><b>!</b><span className="ci-win-line" style={{ width: "70%" }} /></span>}
      </div>
    </div>
  );
}

/** The website's desk: the pet beside a Codex window; drag or arrow keys run it. */
function Desk({ moment, playing, height = 300 }: { moment: Moment; playing: boolean; height?: number }) {
  const desk = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [x, setX] = useState<number | null>(null);
  const [run, setRun] = useState<"left" | "right" | null>(null);
  const grab = useRef<{ last: number } | null>(null);
  const stop = useRef<number | undefined>(undefined);
  useEffect(() => {
    const element = desk.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const half = CELL_W / 2;
  const clamp = (value: number) => Math.min(Math.max(value, half), Math.max(half, width - half));
  const center = clamp(x ?? width * 0.78);
  const runTo = (next: number, direction: "left" | "right") => {
    setX(clamp(next));
    setRun(direction);
    window.clearTimeout(stop.current);
    stop.current = window.setTimeout(() => setRun(null), 220);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const right = event.key === "ArrowRight";
    runTo(center + (right ? 18 : -18), right ? "right" : "left");
  };
  return (
    <div
      ref={desk}
      className="ci-desk"
      style={{ height }}
      onPointerMove={(event: PointerEvent<HTMLDivElement>) => {
        if (!grab.current) return;
        const delta = event.clientX - grab.current.last;
        if (Math.abs(delta) < 1) return;
        grab.current.last = event.clientX;
        runTo(center + delta, delta > 0 ? "right" : "left");
      }}
      onPointerUp={() => { grab.current = null; }}
      onPointerLeave={() => { grab.current = null; }}
    >
      <CodexWindow moment={moment} />
      {width > 0 && (
        <button
          type="button"
          className="ci-desk-pet"
          style={{ left: center - half }}
          aria-label="Mossy. Drag it, or press the left and right arrow keys, to make it run."
          onPointerDown={(event) => { event.currentTarget.setPointerCapture(event.pointerId); grab.current = { last: event.clientX }; }}
          onKeyDown={onKeyDown}
        >
          <Sprite state={run === "right" ? "running-right" : run === "left" ? "running-left" : moment} playing={playing || run !== null} />
        </button>
      )}
    </div>
  );
}

function MomentButtons({ moment, onMoment }: { moment: Moment; onMoment: (next: Moment) => void }) {
  return (
    <div className="ci-moments" role="group" aria-label="Codex moments">
      {MOMENTS.map((entry) => (
        <button key={entry.state} type="button" className="ci-moment" aria-pressed={moment === entry.state} onClick={() => onMoment(entry.state)}>
          {entry.label}
        </button>
      ))}
    </div>
  );
}

export type PetBodyKey = "t1" | "t2" | "t3";

/** T1 today's idle frame, bigger; T2 the website's desk; T3 every moment at once. */
function PetBody({ variant = "t2" }: { variant?: PetBodyKey }) {
  const [moment, setMoment] = useState<Moment>("idle");
  const [playing, setPlaying] = useState(!window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (variant === "t1") {
    return (
      <div className="ci-pet ci-pet-still">
        <div className="ci-pet-stage">
          <img src={MOSSY_TILE} alt="Mossy, idle" width={192} height={208} />
        </div>
        <p className="ci-lede">{ITEM.pet.description}</p>
        <p className="ci-note">The idle frame the list already draws (96 × 104), at twice its size. Nothing new is read; the pet's other rows stay unseen until it is open in Codex.</p>
      </div>
    );
  }
  if (variant === "t3") {
    return (
      <div className="ci-pet">
        <div className="ci-sheet-grid" role="list" aria-label="Mossy in each Codex moment">
          {MOMENTS.map((entry) => (
            <button key={entry.state} type="button" role="listitem" className="ci-sheet-cell" aria-pressed={moment === entry.state} onClick={() => setMoment(entry.state)}>
              <Sprite state={entry.state} playing={moment === entry.state && playing} scale={0.5} />
              <span>{entry.label}</span>
            </button>
          ))}
        </div>
        <div className="ci-pet-tools">
          <span className="ci-muted">Press a moment to play it; the rest hold their first frame.</span>
          <button type="button" className="btn btn-ghost ci-act" onClick={() => setPlaying((value) => !value)}>{playing ? "Pause" : "Play"}</button>
        </div>
        <p className="ci-lede">{ITEM.pet.description}</p>
      </div>
    );
  }
  return (
    <div className="ci-pet">
      <Desk moment={moment} playing={playing} />
      <div className="ci-pet-tools">
        <MomentButtons moment={moment} onMoment={(next) => { setMoment(next); setPlaying(true); }} />
        <button type="button" className="btn btn-ghost ci-act" aria-pressed={!playing} onClick={() => setPlaying((value) => !value)}>{playing ? "Pause" : "Play"}</button>
      </div>
      <p className="ci-lede">{ITEM.pet.description}</p>
    </div>
  );
}

/** The kind's body, as each direction draws it. */
function Body({ kind, skill = "s1", pet = "t2", plugin = "installed", files = true }: { kind: KindKey; skill?: SkillBodyKey; pet?: PetBodyKey; plugin?: PluginState; files?: boolean }) {
  if (kind === "skill") return <SkillBody variant={skill} files={files} />;
  if (kind === "plugin") return <PluginBody state={plugin} />;
  return <PetBody variant={pet} />;
}

/* ——— the page frames ——————————————————————————————————————————————— */

type FrameKey = "a" | "b" | "c" | "d" | "e" | "f";

/** A · The website's page: head, the body in a panel with the copy switch on
 *  top, the facts in a rail. Under 880px of page the rail folds into D's strip. */
function PageA({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-page ci-page-a">
      <ItemHeader kind={kind} />
      <div className="ci-strip-fold"><FactStrip kind={kind} /></div>
      <div className="ci-hub">
        <section className="ci-card ci-panel">
          <CopySwitch />
          <div className="ci-panel-body"><Body kind={kind} /></div>
        </section>
        <Rail kind={kind} />
      </div>
    </div>
  );
}

/** B · A store's product page: a hero with the actions under the words, the
 *  body, then Information at the end. One column at every width. */
function PageB({ kind }: { kind: KindKey }) {
  const status = statusOf(kind);
  const facts: Array<[string, ReactNode]> = [
    ...(kind === "plugin" ? [["Installed", PLUGIN.installed] as [string, ReactNode]] : [["Files", String(ITEM[kind].files)] as [string, ReactNode], ["On disk", formatBytes(ITEM[kind].bytes)] as [string, ReactNode]]),
    ["Edited", exact(NOW - 3_600 * 3)],
    ["Folder", <code className="ci-path">{tilde(ITEM[kind].folder)}</code>],
    ["Repository", <><code>{LOGIN}/{ITEM[kind].name}</code><span className={`ci-pill ${status.link!.visibility}`}>{status.link!.visibility}</span></>],
    ["Draft", "1 push past version 2"],
    ["ID", <code>{ITEM[kind].id}</code>],
  ];
  return (
    <div className="ci-page ci-page-b">
      <header className="ci-hero">
        <KindTile kind={kind} />
        <div className="ci-hero-words">
          <h1>{kind === "plugin" ? PLUGIN.listing.display_name : ITEM[kind].name}</h1>
          <p className="ci-hero-by">{kind === "plugin" && <>by <b>{PLUGIN.listing.developer}</b> · {PLUGIN.listing.category} · </>}{KIND_WORD[kind]} · {kind === "pet" ? "Codex" : "Claude"} · <StatusChip kind={kind} /></p>
          <p className="ci-lede">{ITEM[kind].description}</p>
          <LaunchActions kind={kind} labelled />
        </div>
      </header>
      <section className="ci-card ci-panel">
        <CopySwitch />
        <div className="ci-panel-body">
          {kind === "skill" ? <SkillBody variant="s2" files={false} /> : kind === "plugin" ? <PluginBody hero={false} /> : <PetBody />}
        </div>
      </section>
      {kind === "skill" && (
        <section className="ci-card ci-panel">
          <div className="ci-panel-head"><b>Files</b><span className="ci-panel-status">4 files · {formatBytes(ITEM.skill.bytes)}</span></div>
          <div className="ci-panel-body"><FileTable /></div>
        </section>
      )}
      <section className="ci-info">
        <h2>Information</h2>
        <dl className="ci-info-grid">
          {facts.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}
        </dl>
      </section>
    </div>
  );
}

/** C · Tabs per facet: Overview, Files (a skill's), Details. No rail; the
 *  facts are a tab away. */
function PageC({ kind }: { kind: KindKey }) {
  const tabs = kind === "skill" ? ["Overview", "Files", "Details"] : ["Overview", "Details"];
  const [tab, setTab] = useState("Overview");
  return (
    <div className="ci-page ci-page-c">
      <ItemHeader kind={kind} />
      <div className="sync-project-tabs ci-tabs" role="tablist" aria-label={`${ITEM[kind].name} sections`}>
        {tabs.map((label) => (
          <button key={label} type="button" role="tab" aria-selected={tab === label} className={tab === label ? "active" : undefined} onClick={() => setTab(label)}>
            {label}{label === "Files" && <small className="ci-count">4</small>}
          </button>
        ))}
      </div>
      {tab === "Overview" && (
        <section className="ci-card ci-panel">
          <CopySwitch />
          <div className="ci-panel-body"><Body kind={kind} files={false} /></div>
        </section>
      )}
      {tab === "Files" && <section className="ci-card ci-panel"><div className="ci-panel-body"><FileTable /></div></section>}
      {tab === "Details" && (
        <div className="ci-details-grid">
          <About kind={kind} />
          <ThisMac kind={kind} />
          <InStorage kind={kind} />
          <section className="ci-card ci-side-card"><Identity kind={kind} /></section>
        </div>
      )}
    </div>
  );
}

/** D · One strip of facts under the head, Identity behind a disclosure, the
 *  body at the page's full width. The session page's grammar. */
function PageD({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-page ci-page-d">
      <ItemHeader kind={kind} />
      <FactStrip kind={kind} />
      <section className="ci-card ci-panel">
        <CopySwitch />
        <div className="ci-panel-body"><Body kind={kind} /></div>
      </section>
      <Identity kind={kind} />
    </div>
  );
}

/** E · An inspector: the list stays, and the chosen row's page opens beside it. */
function PageE({ kind }: { kind: KindKey }) {
  const glyph = catalogPresentation(CAPABILITY[kind]);
  return (
    <div className="ci-page ci-page-e">
      <ul className="ci-mini-list" aria-label={CAPABILITY[kind].label}>
        {NEIGHBOURS[kind].map((name) => {
          const on = name === ITEM[kind].name;
          return (
            <li key={name} className={on ? "is-on" : undefined}>
              <ThreadStatusIcon status={on ? "local-outstanding" : "synced"} size={15} />
              {kind === "pet" && on ? <img src={MOSSY_TILE} alt="" className="ci-mini-art" /> : <Icon name={glyph.icon} size={13} />}
              <span>{name}</span>
            </li>
          );
        })}
      </ul>
      <aside className="ci-card ci-inspector" aria-label={`${ITEM[kind].name} inspector`}>
        <div className="ci-inspector-head">
          <KindTile kind={kind} size="md" />
          <div className="ci-item-title">
            <h1>{kind === "plugin" ? PLUGIN.listing.display_name : ITEM[kind].name}</h1>
            <div className="ci-item-sub"><StatusChip kind={kind} /></div>
          </div>
        </div>
        <LaunchActions kind={kind} />
        <div className="ci-inspector-body">
          {kind === "skill" ? <SkillBody variant="s1" files={false} /> : kind === "plugin" ? <PluginBody /> : <PetBody variant="t3" />}
        </div>
        <Identity kind={kind} />
      </aside>
    </div>
  );
}

/** F · A sheet over the list, the way a store opens an item in place. */
function PageF({ kind }: { kind: KindKey }) {
  const glyph = catalogPresentation(CAPABILITY[kind]);
  return (
    <div className="ci-page ci-page-f">
      <div className="ci-behind" aria-hidden="true">
        {NEIGHBOURS[kind].map((name) => (
          <div key={name} className="ci-card ci-ghost-row"><Icon name={glyph.icon} size={13} /><span>{name}</span></div>
        ))}
      </div>
      <div className="ci-scrim">
        <div className="ci-card ci-sheet" role="dialog" aria-label={ITEM[kind].name}>
          <div className="ci-sheet-head">
            <KindTile kind={kind} size="md" />
            <div className="ci-item-title">
              <h1>{kind === "plugin" ? PLUGIN.listing.display_name : ITEM[kind].name}</h1>
              <div className="ci-item-sub"><StatusChip kind={kind} /><ProviderChip kind={kind} /></div>
            </div>
            <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Close"><Icon name="x" size={14} /></button>
          </div>
          <div className="ci-sheet-body">
            {kind === "skill" ? <SkillBody variant="s1" files={false} /> : kind === "plugin" ? <PluginBody /> : <PetBody variant="t3" />}
          </div>
          <div className="ci-sheet-foot">
            <LaunchActions kind={kind} />
            <span className="ci-muted">Push and Pull stay on the bar, behind the sheet.</span>
          </div>
        </div>
      </div>
    </div>
  );
}

const PAGES: Record<FrameKey, (props: { kind: KindKey }) => ReactNode> = { a: PageA, b: PageB, c: PageC, d: PageD, e: PageE, f: PageF };

interface Direction {
  key: FrameKey;
  title: string;
  sub: string;
  seen: Array<[string, string]>;
  against?: string;
  pick?: boolean;
}

const DIRECTIONS: Direction[] = [
  {
    key: "a", title: "A · The website's page", pick: true,
    sub: "Head with the item's tile, name and chips; the body in a panel whose head says which copy it shows; About, This Mac, Mallard Cloud and Identity in a rail. Under 880px of page the rail folds into D's strip, so the default window reads as D.",
    seen: [["Mallard Cloud, item page", "https://mallard-ai.com"], ["VS Code, extension editor (Installation rail)", "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts"], ["Raycast Store (side column, no tabs)", "https://www.raycast.com/raycast/github"], ["GitHub, repository About", "https://github.com/microsoft/vscode"], ["Primer, PageLayout pane", "https://primer.style/product/components/page-layout/"]],
    against: "VS Code hides its rail under 500px rather than moving it; A moves it into the strip, as Primer PageLayout allows, so no fact is lost.",
  },
  {
    key: "b", title: "B · A store's product page",
    sub: "A hero with the actions under the words, the body, then Information at the end. One column at every width, so nothing folds.",
    seen: [["Mac App Store, Information at the end", "https://apps.apple.com/us/app/xcode/id497799835?mt=12"], ["VS Code, actions under the description", "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts"]],
    against: "Primer PageLayout and VS Code keep metadata in view beside the content; a long SKILL.md pushes Information a screen away.",
  },
  {
    key: "c", title: "C · Tabs per facet",
    sub: "Overview, Files (a skill's), Details. No rail; the facts are one tab away, and the body has the page's full width.",
    seen: [["JetBrains, Overview / What's New / Reviews / Additional Info", "https://www.jetbrains.com/help/idea/managing-plugins.html"], ["VS Code, Details / Features / Changelog (shown only with content)", "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts"], ["Obsidian, community directory", "https://community.obsidian.md/"]],
    against: "NN/g Tabs used right: what people need most belongs in the default view; a plugin or pet gets two tabs for one screen of content. VS Code and Obsidian keep a rail beside their tabs.",
  },
  {
    key: "d", title: "D · A strip of facts",
    sub: "One run of chips under the head — files, size, edited, repository, version — Identity behind a disclosure, the body at full width. What A becomes in a narrow window.",
    seen: [["Mallard, the session page's fact chips", "#"], ["VS Code, the header's subtitle row (publisher, installs, rating)", "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts"], ["Apple HIG, disclosure controls", "https://developer.apple.com/design/human-interface-guidelines/disclosure-controls"]],
  },
  {
    key: "e", title: "E · An inspector beside the list",
    sub: "The list stays; the chosen row opens beside it, compact. No page of its own.",
    seen: [["JetBrains, Settings › Plugins (list beside details)", "https://www.jetbrains.com/help/idea/managing-plugins.html"], ["Claude Code, /plugin details pane", "https://code.claude.com/docs/en/discover-plugins"], ["Apple HIG, panels and split views", "https://developer.apple.com/design/human-interface-guidelines/split-views"]],
    against: "At 778px the list gets ~200px and SKILL.md ~460px; unlike the website's page, and the item cannot have a page of its own to land on from a link.",
  },
  {
    key: "f", title: "F · A sheet over the list",
    sub: "The item opens in place over the list, the way a plugin browser shows one; closing returns to the same scroll.",
    seen: [["Mallard, the Add from Mallard Cloud dialog (the app's own sheet)", "#"]],
    against: "No product read first-hand opens an item's page as a sheet (Obsidian's in-app browser and Docker could not be read). Apple prefers a pane for details; the sheet covers Push, Pull and the Cloud chip, and cannot be linked to.",
  },
];

/* ——— the parts ——————————————————————————————————————————————————— */

type Part = "all" | "compare" | "today" | "frames" | "bodies" | "copy" | "widths" | "decide" | "refs";
const PARTS: readonly Part[] = ["all", "compare", "today", "frames", "bodies", "copy", "widths", "decide", "refs"];

/** The six side by side, at half size, in the default window — the view to
 *  choose from; each one full size is under The page. */
function Compare({ kind, width }: { kind: KindKey; width: Width }) {
  return (
    <section className="ci-part">
      <h2>Side by side</h2>
      <p className="ci-lede">All six at {width}px — {WIDTHS[width].window} — at half size. A becomes D below 880px of page, which at the default sidebar is any window under about 1300px; switch the width to 1118 to see A's rail.</p>
      <div className="ci-compare">
        {DIRECTIONS.map((direction) => {
          const Page = PAGES[direction.key];
          return (
            <a key={direction.key} className="ci-compare-cell" href={`#direction-${direction.key}`}>
              <h3>{direction.title}{direction.pick && <em>Recommended</em>}</h3>
              <div className="ci-compare-zoom">
                <Frame kind={kind} width={width} leaf={direction.key !== "e" && direction.key !== "f"} height={700}>
                  <Page kind={kind} />
                </Frame>
              </div>
            </a>
          );
        })}
      </div>
    </section>
  );
}

function Today({ kind }: { kind: KindKey }) {
  return (
    <section className="ci-part">
      <h2>Today</h2>
      <p className="ci-lede">The shipped page, live, at the default window. An item has a page only on Mallard Cloud; it holds the row's own card and nothing more — no description, no SKILL.md, no pet beyond its idle tile, no plugin words. Push, Pull and the Cloud chip sit on the bar.</p>
      <div className="ci-frame" style={{ width: 778, "--ci-pad": "38px" } as CSSProperties}>
        <TodayPage kind={kind} />
      </div>
    </section>
  );
}

function Frames({ kind, width }: { kind: KindKey; width: Width }) {
  return (
    <section className="ci-part">
      <h2>The page — six directions</h2>
      <p className="ci-lede">Each frame is the window's content column at {width}px — {WIDTHS[width].window} — with the shipped bar on top. The bodies inside are the website's; what changes is how the page holds them. Change the width above to see each fold.</p>
      <div className="ci-directions">
        {DIRECTIONS.map((direction) => {
          const Page = PAGES[direction.key];
          return (
            <article key={direction.key} className="ci-direction" id={`direction-${direction.key}`}>
              <h3>{direction.title}{direction.pick && <em>Recommended</em>}</h3>
              <p className="ci-note">{direction.sub}</p>
              <p className="ci-seen">Seen in: {direction.seen.map(([label, href], index) => (
                <Fragment key={label}>{index > 0 && " · "}{href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}</Fragment>
              ))}</p>
              {direction.against && <p className="ci-against"><Icon name="alert-triangle" size={12} />{direction.against}</p>}
              <Frame kind={kind} width={width} leaf={direction.key !== "e" && direction.key !== "f"}>
                <Page kind={kind} />
              </Frame>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function BodyCase({ title, sub, seen, against, children }: { title: string; sub: string; seen: string; against?: string; children: ReactNode }) {
  return (
    <article className="ci-body-case">
      <h3>{title}</h3>
      <p className="ci-note">{sub}</p>
      <p className="ci-seen">Seen in: {seen}</p>
      {against && <p className="ci-against"><Icon name="alert-triangle" size={12} />{against}</p>}
      <div className="ci-body-frame ci-card ci-panel">
        {children}
      </div>
    </article>
  );
}

function Bodies({ kind }: { kind: KindKey }) {
  return (
    <section className="ci-part">
      <h2>The {kind === "skill" ? "skill's" : kind === "plugin" ? "plugin's" : "pet's"} body</h2>
      {kind === "skill" && (
        <div className="ci-body-cases">
          <BodyCase title="S1 · The website's" sub="The description, SKILL.md rendered without its frontmatter, then the files. Recommended: the same page on both sides." seen="Mallard Cloud's skill page; npm and GitHub's README-then-files.">
            <CopySwitch /><div className="ci-panel-body"><SkillBody variant="s1" /></div>
          </BodyCase>
          <BodyCase title="S2 · The frontmatter as fields" sub="Every frontmatter key as a row — name, description, allowed-tools — so what Claude reads to decide when to use the skill is visible." seen="GitHub's rendering of YAML front matter as a table; Claude Code's skills docs (frontmatter fields)." against="Repeats the description, and a long allowed-tools list crowds the top.">
            <CopySwitch /><div className="ci-panel-body"><SkillBody variant="s2" /></div>
          </BodyCase>
          <BodyCase title="S3 · Files beside what they say" sub="The files on the left, the chosen one on the right: SKILL.md rendered, a script as code. Reads every file, not just SKILL.md." seen="GitHub's file view; VS Code's explorer; Mallard Cloud's own Files tab." against="Needs a bounded read of every file, and the tree takes ~180px a narrow window cannot spare.">
            <CopySwitch /><div className="ci-panel-body"><SkillBody variant="s3" /></div>
          </BodyCase>
        </div>
      )}
      {kind === "plugin" && (
        <div className="ci-body-cases">
          <BodyCase title="Installed here" sub="The website's store hero, as the owner picked it for Mallard Cloud, plus the one line only this Mac can say." seen="Mallard Cloud's plugin page; Claude Code's /plugin details (installed state).">
            <div className="ci-panel-body"><PluginBody state="installed" named /></div>
          </BodyCase>
          <BodyCase title="Not on this Mac" sub="Stored only: the line says Pull installs it; the copy box is for doing it by hand." seen="App Store (Get on a product you do not have).">
            <div className="ci-panel-body"><PluginBody state="absent" named /></div>
          </BodyCase>
          <BodyCase title="No listing" sub="A plugin whose manifest says nothing Mallard can show: the intent's own name@marketplace, as the website does." seen="Mallard Cloud's plugin page, before its listing answers.">
            <div className="ci-panel-body"><PluginBody state="no-listing" named /></div>
          </BodyCase>
        </div>
      )}
      {kind === "pet" && (
        <div className="ci-body-cases">
          <BodyCase title="T1 · The idle frame, bigger" sub="Layer 1: no new read. The list's preview at twice its size, with the description." seen="the Catalog card's own tile; Mallard Cloud's pet tile in its header." against="The tile is 96 × 104; at 192 × 208 it is upscaled. Says nothing about how the pet moves.">
            <div className="ci-panel-body"><PetBody variant="t1" /></div>
          </BodyCase>
          <BodyCase title="T2 · The website's desk" sub="Layer 2: the atlas reaches the page. The pet beside a Codex window, moments as buttons, drag or arrow keys to run; Pause holds it (WCAG 2.2.2), reduced motion starts paused. Recommended." seen="Mallard Cloud's pet page; GitHub's play/pause on animated images; macOS Screen Saver, which previews only when asked.">
            <div className="ci-panel-body"><PetBody variant="t2" /></div>
          </BodyCase>
          <BodyCase title="T3 · Every moment at once" sub="Layer 2, smaller: one cell per moment, still on its first frame; pressing a cell plays it. Fits an inspector or a narrow window." seen="no product read first-hand (LINE's sticker page served a stub; Telegram was not tried)." against="Five half-size sprites read smaller than the desk, and lose the Codex window that says what each moment means.">
            <div className="ci-panel-body"><PetBody variant="t3" /></div>
          </BodyCase>
        </div>
      )}
    </section>
  );
}

const COPY_CASES: Array<{ state: CatalogLinkState; title: string; shows: string }> = [
  { state: "in_sync", title: "The same on both", shows: "One label, no switch: there is one copy to show." },
  { state: "local_changed", title: "Changed here", shows: "The switch, on This Mac. Mallard Cloud · draft shows what a Pull would bring back." },
  { state: "storage_changed", title: "Changed in Mallard Cloud", shows: "The switch, on This Mac; the draft side shows what Pull would bring." },
  { state: "diverged", title: "Changed on both", shows: "The switch: the page is where you read both before you choose Pull's Keep or Overwrite." },
  { state: "local_missing", title: "Not downloaded", shows: "One label, the stored copy: read before Pull installs it." },
  { state: "new", title: "Not pushed", shows: "One label, this Mac: nothing stored to compare." },
];

function CopyStates() {
  const [shown, setShown] = useState<Record<string, "mac" | "storage">>({});
  return (
    <section className="ci-part">
      <h2>Which copy — the website's Draft ▾, made for this Mac</h2>
      <p className="ci-lede">The website asks which version you read. The app's question is which copy: this Mac's, or the storage's. Versions stay in the Cloud chip's panel, where Publish and Discard already live.</p>
      <div className="ci-copy-cases">
        {COPY_CASES.map((entry) => (
          <article key={entry.state} className="ci-copy-case">
            <h3>{entry.title}</h3>
            <div className="ci-card ci-panel">
              <CopySwitch state={entry.state} shown={shown[entry.state] ?? "mac"} onShow={(next) => setShown((current) => ({ ...current, [entry.state]: next }))} />
            </div>
            <p className="ci-note">{entry.shows}</p>
          </article>
        ))}
      </div>
    </section>
  );
}

function Widths({ kind }: { kind: KindKey }) {
  return (
    <section className="ci-part">
      <h2>A at the three widths</h2>
      <p className="ci-lede">The rail needs 880px of page beside the body; below that it folds into the strip. Measured against the shipped floor: the path bar's 580px, the 1100px default window with its 298px sidebar, and a 1440px window.</p>
      <div className="ci-width-row">
        {(Object.keys(WIDTHS) as Width[]).map((width) => (
          <article key={width} className="ci-direction">
            <h3>{width}px <span className="ci-muted">— {WIDTHS[width].window}</span></h3>
            <Frame kind={kind} width={width}><PageA kind={kind} /></Frame>
          </article>
        ))}
      </div>
    </section>
  );
}

const MAP: Array<{ cloud: string; app: string; why: string }> = [
  { cloud: "Trail: Repositories › owner › name › Item", app: "The path bar: Mallard Cloud / Skills · Claude / release-notes", why: "The bar already is the trail; the leaf is the item." },
  { cloud: "Kind tile, owner / name, visibility pill", app: "Kind tile (a pet's idle frame), the name, status · kind · provider chips", why: "Owner and visibility describe the repository: they go to the Mallard Cloud card and the Cloud chip." },
  { cloud: "Star, Invite, Request public release", app: "Not on the app's page; the Cloud chip's panel opens the website", why: "Web verbs about the repository; the panel already carries Open on website, Publish, Discard." },
  { cloud: "Pull ▾ (Add to my Catalog, Download .zip)", app: "Push and Pull on the bar, as today", why: "The bar's actions never shrink and are already at budget at 580px; nothing is added to them." },
  { cloud: "Tabs: Item · Versions · Release · Members · Settings", app: "No tab bar (one facet), or C's Overview · Files · Details", why: "Versions, Release, Members, Settings are the website's; the website hides a single tab too." },
  { cloud: "Draft ▾ / Version N switcher, Publish version N", app: "This Mac | Mallard Cloud · draft switch; Publish stays in the Cloud panel", why: "The app's question is which copy, not which version." },
  { cloud: "Skill: description, SKILL.md, Files table", app: "The same body", why: "Same reader, same order. Needs a bounded read of SKILL.md and the file list." },
  { cloud: "Plugin: store hero + install box + facts", app: "The same body, plus one line: installed here, or Pull installs it", why: "On this Mac, Pull is the install; the copy box is for doing it by hand." },
  { cloud: "Pet: desk with moments", app: "The same desk (layer 2), or the idle frame (layer 1)", why: "The desk needs the atlas on the page and a port of the player." },
  { cloud: "About · People · Draft rail", app: "About · This Mac · Mallard Cloud · Identity", why: "People is the website's; This Mac (folder, edited, version) is only the app's." },
  { cloud: "(the app today) Repository card: New · Private · Change…", app: "The Mallard Cloud card: the address the first Push creates, New, Change…", why: "Today's only extra card moves into the rail it belongs to; the blocked note (in two repositories) stays above the page." },
  { cloud: "(another storage) —", app: "The storage's card: stored copy, updated, the same or not", why: "A Vault or R2 item gets the same page, without the Cloud chip." },
];

function Decide() {
  return (
    <section className="ci-part">
      <h2>What maps to what, and what is yours to decide</h2>
      <table className="ci-refs">
        <thead><tr><th>On Mallard Cloud</th><th>In the app</th><th>Why</th></tr></thead>
        <tbody>{MAP.map((row) => <tr key={row.cloud}><td>{row.cloud}</td><td>{row.app}</td><td>{row.why}</td></tr>)}</tbody>
      </table>
      <div className="ci-questions">
        <article className="ci-card ci-side-card">
          <h3>1 · Which storages give an item a page</h3>
          <p className="ci-note"><b>Every storage (recommended).</b> SKILL.md, a plugin's words and a pet's art are on this Mac whatever the storage; a Vault or R2 item gets the page without the Mallard Cloud card.</p>
          <p className="ci-note"><b>Mallard Cloud only.</b> Today's rule: the name opens a page only where each item is its own repository.</p>
        </article>
        <article className="ci-card ci-side-card">
          <h3>2 · How far the pet goes</h3>
          <p className="ci-note"><b>Layer 1</b> ships T1 with no new read. <b>Layer 2 (recommended next)</b> sends the verified atlas to the page and ports the website's player: T2 on the page, T3 where space is short.</p>
        </article>
        <article className="ci-card ci-side-card">
          <h3>3 · Which copy</h3>
          <p className="ci-note"><b>This Mac first, Mallard Cloud on the switch (recommended).</b> The stored copy's SKILL.md is read only when asked, through the same verified fetch the pet preview uses.</p>
          <p className="ci-note"><b>This Mac only.</b> No switch; a stored-only item shows only its facts until pulled.</p>
        </article>
      </div>
    </section>
  );
}

const REFS: Array<{ source: string; href: string; asks: string; here: string }> = [
  { source: "Primer, PageLayout", href: "https://primer.style/product/components/page-layout/", asks: "Secondary information (metadata, details) in a pane beside the content; the pane may move or hide per breakpoint.", here: "A's rail; it folds into the strip below 880px." },
  { source: "Primer, PageHeader", href: "https://primer.style/product/components/page-header/guidelines/", asks: "Title with a leading visual and trailing status; actions trailing; parent link in a narrow context bar.", here: "The head: tile, name, chips; the bar's scope crumb is the parent link." },
  { source: "Apple HIG, Toolbars", href: "https://developer.apple.com/design/human-interface-guidelines/toolbars", asks: "At most one prominent action; don't put one action in several places.", here: "Push and Pull stay on the bar only; the head has one named button (Open in Claude)." },
  { source: "Apple HIG, Tab views · NN/g, Tabs used right", href: "https://www.nngroup.com/articles/tabs-used-right/", asks: "Tabs for few, parallel groups; the most-used content in the default tab; not when people need several at once.", here: "A and D have no tabs; C is the tabbed one, and says what it hides." },
  { source: "ARIA APG, Tabs", href: "https://www.w3.org/WAI/ARIA/apg/patterns/tabs/", asks: "tablist / tab / tabpanel, arrow keys between tabs, automatic activation when panels are instant.", here: "The copy switch and C's tabs." },
  { source: "Carbon, Code snippet", href: "https://carbondesignsystem.com/components/code-snippet/usage/", asks: "Read-only command with its own Copy button; a confirmation after copy; focus stays on the button.", here: "The plugin's install box: one Copy, Copied for 1.6s." },
  { source: "Carbon, Structured list · MDN <dl>", href: "https://carbondesignsystem.com/components/structured-list/usage/", asks: "Key/value facts as a simple list; name–value groups in a description list.", here: "Every facts block is a <dl>." },
  { source: "WCAG 2.2.2 Pause, Stop, Hide · HIG Motion", href: "https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html", asks: "Looping motion over 5s next to other content needs a pause; honour Reduce Motion.", here: "The desk and the sheet have Pause; reduced motion starts them paused." },
  { source: "NN/g, Progressive disclosure · HIG Disclosure controls", href: "https://www.nngroup.com/articles/progressive-disclosure/", asks: "Rarely needed detail one labelled step away, hidden by default.", here: "Identity (ID, coordinate, digest) behind a disclosure, as on the website." },
  { source: "NN/g, Breadcrumbs", href: "https://www.nngroup.com/articles/breadcrumbs/", asks: "Hierarchy, current page last and not a link.", here: "The bar's leaf is the item; the scope crumb goes back." },
  { source: "VS Code, extension editor", href: "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts", asks: "Header (icon, name, publisher, description, actions); README in the main column; a rail of Installation / Marketplace facts, hidden under 500px.", here: "A, with This Mac as VS Code's Installation and Mallard Cloud as its Marketplace." },
  { source: "Raycast Store", href: "https://www.raycast.com/raycast/github", asks: "Install in the header; commands, README and version history in the main column; categories, platforms, contributors in a side column; no tabs.", here: "A: no tabs, a rail." },
  { source: "GitHub, repository page", href: "https://github.com/microsoft/vscode", asks: "Star and Fork in the header, underline tabs, an About rail beside the README.", here: "A's rail; the website's own page is this shape." },
  { source: "JetBrains, Settings › Plugins", href: "https://www.jetbrains.com/help/idea/managing-plugins.html", asks: "The list beside the details; Install or Update beside the name; facts in an Additional Info tab.", here: "E (list beside details) and C (facts in a tab)." },
  { source: "Mac App Store", href: "https://apps.apple.com/us/app/xcode/id497799835?mt=12", asks: "One column, no tabs; Information (Seller, Size, Category, Compatibility) at the end.", here: "B." },
  { source: "Claude Code, plugins and skills", href: "https://code.claude.com/docs/en/discover-plugins", asks: "A plugin is name@marketplace, installed in-session or from the shell; a skill is SKILL.md (frontmatter + instructions) plus files.", here: "The plugin's two install ways; the skill's description from frontmatter, the body rendered." },
];

function Refs() {
  return (
    <section className="ci-part">
      <h2>References</h2>
      <p className="ci-lede">Read on 2026-10-06, paraphrased. VS Code was read from its source (extensionEditor.ts and its CSS). Could not be read, so not cited: Apple's Inspectors page (404), Primer's clipboard pages, Material 3's site (read through its Android Tabs doc instead), npm (bot check), Docker docs and Hub, Steam, LINE (a stub), Obsidian's in-app plugin view; Telegram was not tried. None of the sources covers hiding a tab bar that has one tab.</p>
      <table className="ci-refs">
        <thead><tr><th>Reference</th><th>It asks or shows</th><th>In the specimen</th></tr></thead>
        <tbody>
          {REFS.map((ref) => (
            <tr key={ref.source}>
              <td><a href={ref.href} target="_blank" rel="noreferrer">{ref.source}</a></td>
              <td>{ref.asks}</td>
              <td>{ref.here}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

/* ——— the specimen ——— */

type Theme = "light" | "dark";

function Specimen() {
  const [theme, setTheme] = useState<Theme>(pick<Theme>("theme", ["light", "dark"], "dark"));
  const [kind, setKind] = useState<KindKey>(pick("kind", KINDS, "skill"));
  const [part, setPart] = useState<Part>(pick("part", PARTS, "all"));
  const [width, setWidth] = useState<Width>(pick<Width>("width", ["580", "778", "1118"], "778"));
  const clean = params.has("clean");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    query.set("part", part);
    query.set("kind", kind);
    query.set("width", width);
    query.set("theme", theme);
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [part, kind, width, theme]);
  const option = <T extends string>(value: T, current: T, set: (next: T) => void, label: string) => (
    <button key={value} type="button" className="ci-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const shows = (each: Part) => part === "all" || part === each;
  return (
    <main className="ci">
      {!clean && (
        <header className="ci-head">
          <h1>A Catalog item's page, like Mallard Cloud's</h1>
          <p>Today a skill, plugin or pet has a page only on Mallard Cloud, and the page is its list card. Proposed: the website's page — its skill, plugin and pet bodies — inside the app's bar, with the facts only this Mac knows.</p>
          <div className="ci-controls">
            <span>Part</span>
            {option("all", part, setPart, "All")}
            {option("compare", part, setPart, "Side by side")}
            {option("today", part, setPart, "Today")}
            {option("frames", part, setPart, "The page")}
            {option("bodies", part, setPart, "The body")}
            {option("copy", part, setPart, "Which copy")}
            {option("widths", part, setPart, "Widths")}
            {option("decide", part, setPart, "Decisions")}
            {option("refs", part, setPart, "References")}
            <span>Item</span>
            {option("skill", kind, setKind, "Skill")}
            {option("plugin", kind, setKind, "Plugin")}
            {option("pet", kind, setKind, "Pet")}
            <span>Width</span>
            {option("580", width, setWidth, "580")}
            {option("778", width, setWidth, "778")}
            {option("1118", width, setWidth, "1118")}
            <span>Theme</span>
            {option("light", theme, setTheme, "Light")}
            {option("dark", theme, setTheme, "Dark")}
          </div>
        </header>
      )}
      {shows("compare") && <Compare kind={kind} width={width} />}
      {shows("today") && <Today kind={kind} />}
      {shows("frames") && <Frames kind={kind} width={width} />}
      {shows("bodies") && <Bodies kind={kind} />}
      {shows("copy") && <CopyStates />}
      {shows("widths") && <Widths kind={kind} />}
      {shows("decide") && <Decide />}
      {shows("refs") && <Refs />}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
