import { Fragment, StrictMode, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon, { type IconName } from "../src/components/Icons";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { SessionDetailMarkdown } from "../src/components/project-sync/session-detail/SessionDetailEvent";
import StorageCatalogPage, { type CatalogStorageOption } from "../src/components/storage-catalog/StorageCatalogPage";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import { catalogPresentation, catalogScopeLabel } from "../src/components/storage-catalog/model";
import type { CatalogCapabilityDescriptor, CatalogProfile, CatalogScopeRow, CatalogScopeView } from "../src/components/storage-catalog/types";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatFullDateTime } from "../src/clock";
import { formatBytes } from "../src/format";
import { MOSSY_ATLAS, MOSSY_TILE } from "./catalog-item-page-art";
import "../src/App.css";
import "./catalog-item-page-mockup.css";

/**
 * A Catalog item's page in the app, made like Mallard Cloud's
 * (docs/PLAN_CATALOG_ITEM_PAGE.md). One rule across every direction: each
 * fact is said once. The bar already says the name (leaf), kind and provider
 * (scope crumb), sync state (Push / Pull counts) and the repository (Cloud
 * chip), so the page repeats none of them.
 *
 * `npm run dev`, then /mockups/catalog-item-page-mockup.html
 * (`?kind=skill|plugin|pet&width=580|778|1118&open=today|a|b|c|d|e|f&theme=light|dark`).
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

/* ——— the machine ——— */

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
const STORAGES: CatalogStorageOption[] = [{
  id: "storage-cloud", provider: "managed_cloud", name: "Mallard Cloud", provider_label: "Mallard Cloud", icon: "storage-mallard",
  detail: `${LOGIN} · ${HOST}`, detailIsPath: false, account_login: LOGIN,
}];

/* ——— the items ——— */

const SKILL_MD = `# Release notes

Write release notes for the range the user names, or for everything since the newest tag.

## Steps

1. Find the range: \`git describe --tags --abbrev=0\` names the last tag.
2. Read \`git log --no-merges <tag>..HEAD\` and group the commits by area.
3. Draft one line per change in the house voice (see \`voice.md\`).
4. Put breaking changes first, under **Breaking**.

## Output

| Section | What goes in it |
| --- | --- |
| Breaking | Anything that needs action from users |
| Added | New features |
| Fixed | Bug fixes users would notice |
`;
const SKILL_FILES = [
  { path: "SKILL.md", size: 1_644, exec: false, text: null as string | null },
  { path: "voice.md", size: 3_120, exec: false, text: "# House voice\n\nShort sentences. Name what changed for the reader.\n\n- Lead with the verb: *Adds*, *Fixes*, *Removes*.\n- One change per line.\n" },
  { path: "templates/notes.md", size: 2_204, exec: false, text: "# Release {{version}}\n\n## Breaking\n\n## Added\n\n## Fixed\n" },
  { path: "scripts/range.sh", size: 2_444, exec: true, text: "#!/bin/sh\nset -eu\ntag=$(git describe --tags --abbrev=0 2>/dev/null || true)\n[ -n \"$tag\" ] && echo \"$tag\" || git rev-list --max-parents=0 HEAD | tail -n 1\n" },
];

const PLUGIN = {
  name: "lint-buddy", marketplace: "acme-tools", source: "acme-tools/claude-plugins", version: "1.4.0",
  displayName: "Lint Buddy", developer: "Acme Tools", category: "Code quality",
};

const ITEM: Record<KindKey, { id: string; name: string; folder: string; bytes: number | null; description: string }> = {
  skill: {
    id: "release-notes", name: "release-notes", folder: "~/.claude/skills/release-notes", bytes: 9_412,
    description: "Drafts release notes from the commits since the last tag, grouped by area, in the house voice.",
  },
  plugin: {
    id: "lint-buddy@acme-tools", name: "lint-buddy", folder: "~/.claude/plugins/cache/acme-tools/lint-buddy", bytes: null,
    description: "Runs your linters after every edit and explains each finding in plain words, with a fix you can accept.",
  },
  pet: {
    id: "mossy", name: "Mossy", folder: "~/.codex/pets/mossy", bytes: 6_034,
    description: "A mallard who naps on the terminal and paddles when a build is running.",
  },
};
const NEIGHBOURS: Record<KindKey, string[]> = {
  skill: ["commit-style", "pdf-tools", "release-notes", "sql-review"],
  plugin: ["code-review", "lint-buddy", "slack-notify"],
  pet: ["Biscuit", "Mossy", "Pip"],
};
const EDITED = NOW - 3_600 * 3;

const rowOf = (kind: KindKey): CatalogScopeRow => ({
  row_id: `row-${ITEM[kind].id}`, display_name: ITEM[kind].name, resource_id: ITEM[kind].id, state: "local_changed",
  local_present: true, local_path: ITEM[kind].folder, local_modified_at: EDITED,
  local_files: kind === "plugin" ? null : kind === "skill" ? 4 : 2, local_bytes: ITEM[kind].bytes,
  remote_present: true, remote_updated_at: NOW - 86_400 * 2, content_sha256: "a3f9".padEnd(64, "0"),
  preview_png: kind === "pet" ? MOSSY_TILE : null, description: kind === "pet" ? ITEM.pet.description : null, warnings: [],
});

const statusOf = (kind: KindKey): ManagedCloudStatus => {
  const repositoryId = `repo_${LOGIN}_${ITEM[kind].name}`;
  return {
    repository_id: repositoryId, state: "ready", message: null,
    link: {
      visibility: "private", can_pull: true, repository_id: repositoryId, website_url: `https://${HOST}/${LOGIN}/${ITEM[kind].name}`,
      owner_login: LOGIN, name: ITEM[kind].name, release: { kind: "private" }, repository_revision: 4,
      can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner", can_push: true,
    },
    publication: { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: 1, public_version: null, review_version: null },
  };
};

/* ——— the stand-in backend, for Today ——— */

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
const kindOfScope = (scopeId: string): KindKey => scopeId.replace("scope-", "") as KindKey;
storageCatalogApi.listScopeResources = async (scopeId) => {
  await wait(150);
  const kind = kindOfScope(scopeId);
  const scope = scopeOf(kind);
  const repository = `repo_${LOGIN}_${ITEM[kind].name}`;
  return {
    provider_label: scope.provider_label, kind_label: scope.kind_label, format_label: scope.format_label,
    scope: scope.scope, profile: scope.profile, storage_id: "storage-cloud", storage_name: "Mallard Cloud",
    link: { link_id: `link-${kind}`, revision: 3 }, storage_revision: null, rows: [rowOf(kind)], warnings: [], recovery: [],
    cloud_repositories: { [ITEM[kind].id]: repository }, cloud_connections: { [ITEM[kind].id]: repository }, cloud_status: null,
  };
};
storageCatalogApi.getPreview = async () => MOSSY_TILE;
/* Today is now the shipped page: it reads each kind's page as the backend serves it. */
storageCatalogApi.getDetail = async (scopeId) => {
  const kind = kindOfScope(scopeId);
  if (kind === "skill") return { kind: "document", path: "SKILL.md", text: SKILL_MD, files: SKILL_FILES.map(({ path, size, exec }) => ({ path, size, executable: exec })) };
  if (kind === "plugin") {
    return {
      kind: "install", plugin_id: `${PLUGIN.name}@${PLUGIN.marketplace}`, marketplace: PLUGIN.marketplace, location: PLUGIN.source, git_ref: null,
      install: [`claude plugin marketplace add ${PLUGIN.source} --scope user`, `claude plugin install ${PLUGIN.name}@${PLUGIN.marketplace} --scope user`],
      installed_version: PLUGIN.version,
      listing: { display_name: PLUGIN.displayName, developer: PLUGIN.developer, category: PLUGIN.category, description: ITEM.plugin.description, website_url: null, privacy_url: null, terms_url: null },
    };
  }
  return { kind: "sprite", atlas: MOSSY_ATLAS, columns: 8, rows: 11, description: ITEM.pet.description };
};
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};
storageCatalogApi.listProfiles = async () => [CLAUDE, CODEX];
storageCatalogApi.listCapabilities = async () => CAPABILITIES;
storageCatalogApi.getCloudStatus = async (scopeId) => {
  await wait(150);
  return statusOf(kindOfScope(scopeId));
};

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

/* ——— the frame: the shipped bar, the page under it ——— */

type Width = "580" | "778" | "1118";
const PAD: Record<Width, number> = { "580": 28, "778": 38, "1118": 50 };
const frameStyle = (width: Width) => ({ width: Number(width), "--ci-pad": `${PAD[width]}px` } as CSSProperties);

function Frame({ kind, width, leaf = true, children }: { kind: KindKey; width: Width; leaf?: boolean; children: ReactNode }) {
  return (
    <div className="ci-frame" style={frameStyle(width)}>
      <div className="ci-app">
        <Bar kind={kind} leaf={leaf} />
        <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">{children}</main>
      </div>
    </div>
  );
}

function Bar({ kind, leaf }: { kind: KindKey; leaf: boolean }) {
  const scope = scopeOf(kind);
  const glyph = catalogPresentation(scope.capability);
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root" },
    { id: "scope", label: catalogScopeLabel(scope), icon: glyph.icon, onActivate: leaf ? noop : undefined },
    ...(leaf ? [{
      id: "item", label: ITEM[kind].name,
      leading: kind === "pet" ? <img className="catalog-leaf-mark" src={MOSSY_TILE} alt="" /> : <Icon name={glyph.icon} size={14} />,
    }] : []),
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      actions={(
        <>
          {leaf && (
            <ManagedCloudChip id={`cloud-${kind}`} status={statusOf(kind)} storageName="Mallard Cloud" busy={false} refreshing={false}
              onOpenUrl={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
          )}
          <button type="button" className="sync-path-icon-action" aria-label="Refresh comparison">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync" disabled><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
            <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span><span className="sync-path-count" aria-hidden="true">1</span></button>
          </div>
        </>
      )}
    />
  );
}

/* ——— the page's own parts, each said once ——— */

/** The card's launch run: the provider app, Terminal, Fork, Reveal last. */
function Actions({ kind, labelled = false }: { kind: KindKey; labelled?: boolean }) {
  const profile = kind === "pet" ? CODEX : CLAUDE;
  const button = (icon: ReactNode, label: string) => (
    <button key={label} type="button" className={labelled ? "btn ci-act" : "btn btn-ghost sync-history-launch-action"} title={label} aria-label={label}>
      {icon}{labelled && <span>{label}</span>}
    </button>
  );
  return (
    <div className="ci-actions">
      {button(<ProviderMark provider={profile.provider} displayName={profile.provider_label} size={14} />, `Open in ${profile.provider_label}`)}
      {button(<Icon name="terminal" size={14} />, "Terminal")}
      {kind !== "plugin" && button(<Icon name="git-fork" size={14} />, "Fork")}
      {button(<Icon name="folder" size={14} />, "Reveal in Finder")}
    </div>
  );
}

/** What only this Mac knows: when it was edited, and its size. */
function facts(kind: KindKey): Array<[IconName, string, ReactNode]> {
  return [
    ["clock", "Edited", <time>{formatFullDateTime(EDITED)}</time>],
    ...(ITEM[kind].bytes == null ? [] : [["drive", "Size", formatBytes(ITEM[kind].bytes!)] as [IconName, string, ReactNode]]),
  ];
}

function FactLine({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-line">
      {facts(kind).map(([icon, label, value]) => <span key={label} className="ci-fact" title={label}><Icon name={icon} size={13} />{value}</span>)}
    </div>
  );
}

function FactList({ kind }: { kind: KindKey }) {
  return (
    <dl className="ci-facts">
      {facts(kind).map(([, label, value]) => <Fragment key={label}><dt>{label}</dt><dd>{value}</dd></Fragment>)}
    </dl>
  );
}

function Panel({ children }: { children: ReactNode }) {
  return <section className="ci-card ci-panel">{children}</section>;
}

/* ——— the bodies ——— */

function Markdown({ text }: { text: string }) {
  return <div className="session-detail-event-copy markdown-copy ci-md"><SessionDetailMarkdown text={text} /></div>;
}

function FileTable() {
  return (
    <table className="ci-table">
      <tbody>
        {SKILL_FILES.map((file) => (
          <tr key={file.path}>
            <td><Icon name="file" size={13} /><code>{file.path}</code>{file.exec && <span className="ci-exec" title="Executable"><Icon name="terminal" size={12} /></span>}</td>
            <td>{formatBytes(file.size)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

type SkillKey = "s1" | "s2";
function SkillBody({ variant = "s1", lede = true, files = true }: { variant?: SkillKey; lede?: boolean; files?: boolean }) {
  const [open, setOpen] = useState("SKILL.md");
  const file = SKILL_FILES.find((each) => each.path === open)!;
  return (
    <div className="ci-skill">
      {lede && <p className="ci-lede">{ITEM.skill.description}</p>}
      {variant === "s2" ? (
        <div className="ci-browser">
          <ul className="ci-tree">
            {SKILL_FILES.map((each) => (
              <li key={each.path}>
                <button type="button" aria-current={open === each.path ? "true" : undefined} onClick={() => setOpen(each.path)}>
                  <Icon name="file" size={13} /><span>{each.path}</span>
                </button>
              </li>
            ))}
          </ul>
          <div className="ci-viewer">
            {file.text == null ? <Markdown text={SKILL_MD} /> : file.path.endsWith(".md") ? <Markdown text={file.text} /> : <pre className="ci-code"><code>{file.text}</code></pre>}
          </div>
        </div>
      ) : (
        <>
          <div className="ci-inner"><Markdown text={SKILL_MD} /></div>
          {files && <FileTable />}
        </>
      )}
    </div>
  );
}

const INSTALL = [
  { key: "app", label: "Claude Code", prompt: ">", lines: [`/plugin install ${PLUGIN.name} --marketplace ${PLUGIN.source}`] },
  { key: "terminal", label: "Terminal", prompt: "$", lines: [`claude plugin marketplace add ${PLUGIN.source}`, `claude plugin install ${PLUGIN.name}@${PLUGIN.marketplace}`] },
];

function PluginBody({ intro = true, installed = true }: { intro?: boolean; installed?: boolean }) {
  const [way, setWay] = useState("app");
  const [copied, setCopied] = useState(false);
  const shown = INSTALL.find((each) => each.key === way)!;
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <div className="ci-plugin">
      {intro && <p className="ci-by"><b>{PLUGIN.displayName}</b> by {PLUGIN.developer} · {PLUGIN.category}</p>}
      {intro && <p className="ci-lede">{ITEM.plugin.description}</p>}
      <p className={`ci-here${installed ? "" : " is-absent"}`}>
        <Icon name={installed ? "check-circle" : "download"} size={14} />
        {installed ? `Installed · ${PLUGIN.version}` : "Pull installs it"}
      </p>
      <div className="ci-install">
        <div className="ci-install-head">
          <div className="sync-segmented" role="tablist" aria-label="Install with">
            {INSTALL.map((each) => (
              <button key={each.key} type="button" role="tab" aria-selected={each.key === way} className={each.key === way ? "active" : undefined} onClick={() => setWay(each.key)}>{each.label}</button>
            ))}
          </div>
          <button type="button" className="btn ci-copy-btn" onClick={() => setCopied(true)}><Icon name={copied ? "check" : "copy"} size={13} />{copied ? "Copied" : "Copy"}</button>
        </div>
        <pre className="ci-lines"><code>{shown.lines.map((line) => <span key={line}><span className="ci-prompt">{shown.prompt} </span>{line}{"\n"}</span>)}</code></pre>
      </div>
      <p className="ci-links">
        <a href="#" onClick={(event) => event.preventDefault()}><Icon name="link" size={12} />{PLUGIN.source}<Icon name="external-link" size={10} /></a>
        {["Website", "Privacy", "Terms"].map((label) => <a key={label} href="#" onClick={(event) => event.preventDefault()}>{label}<Icon name="external-link" size={10} /></a>)}
      </p>
    </div>
  );
}

/* The website's pet player (web/src/pet.ts, PetPage.tsx), redrawn. */
const CELL_W = 192;
const CELL_H = 208;
type Moment = "idle" | "running" | "waiting" | "review" | "failed";
const STATES: Record<Moment | "right" | "left", { row: number; durations: number[] }> = {
  idle: { row: 0, durations: [280, 110, 110, 140, 140, 320] },
  right: { row: 1, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  left: { row: 2, durations: [120, 120, 120, 120, 120, 120, 120, 220] },
  failed: { row: 5, durations: [140, 140, 140, 140, 140, 140, 140, 240] },
  waiting: { row: 6, durations: [150, 150, 150, 150, 150, 260] },
  running: { row: 7, durations: [120, 120, 120, 120, 120, 220] },
  review: { row: 8, durations: [150, 150, 150, 150, 150, 280] },
};
const MOMENTS: ReadonlyArray<[Moment, string]> = [["idle", "Idle"], ["running", "Working"], ["waiting", "Needs you"], ["review", "Review"], ["failed", "Failed"]];

function Sprite({ state, playing, scale = 1 }: { state: keyof typeof STATES; playing: boolean; scale?: number }) {
  const { row, durations } = STATES[state];
  const [at, setAt] = useState({ state, frame: 0 });
  const frame = at.state === state ? at.frame % durations.length : 0;
  useEffect(() => {
    if (!playing) return;
    const timer = window.setTimeout(() => setAt({ state, frame: (frame + 1) % durations.length }), durations[frame]);
    return () => window.clearTimeout(timer);
  }, [state, frame, playing, durations]);
  return (
    <span className="ci-sprite" aria-hidden="true" style={{
      width: CELL_W * scale, height: CELL_H * scale, backgroundImage: `url(${MOSSY_ATLAS})`,
      backgroundSize: `${8 * CELL_W * scale}px ${11 * CELL_H * scale}px`,
      backgroundPosition: `${-frame * CELL_W * scale}px ${-row * CELL_H * scale}px`,
    }} />
  );
}

function CodexWindow({ moment }: { moment: Moment }) {
  return (
    <div className="ci-win" aria-hidden="true">
      <div className="ci-win-bar"><i /><i /><i /></div>
      <div className="ci-win-body">
        <span className="ci-win-line" style={{ width: "58%" }} />
        <span className="ci-win-line" style={{ width: "74%" }} />
        {moment === "running" && <span className="ci-win-dots"><i /><i /><i /></span>}
        {moment === "waiting" && <span className="ci-win-ask"><span className="ci-win-btns"><b>Allow</b><b>Deny</b></span></span>}
        {moment === "review" && <span className="ci-win-diff"><i className="is-del" /><i className="is-add" /></span>}
        {moment === "failed" && <span className="ci-win-fail"><b>!</b></span>}
      </div>
    </div>
  );
}

function Desk({ moment, playing }: { moment: Moment; playing: boolean }) {
  const desk = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [x, setX] = useState<number | null>(null);
  const [run, setRun] = useState<"left" | "right" | null>(null);
  const stop = useRef<number | undefined>(undefined);
  useEffect(() => {
    const element = desk.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const half = CELL_W / 2;
  const center = Math.min(Math.max(x ?? width * 0.78, half), Math.max(half, width - half));
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const right = event.key === "ArrowRight";
    setX(center + (right ? 18 : -18));
    setRun(right ? "right" : "left");
    window.clearTimeout(stop.current);
    stop.current = window.setTimeout(() => setRun(null), 220);
  };
  return (
    <div ref={desk} className="ci-desk">
      <CodexWindow moment={moment} />
      {width > 0 && (
        <button type="button" className="ci-desk-pet" style={{ left: center - half }} aria-label="Mossy. Arrow keys make it run." onKeyDown={onKeyDown}>
          <Sprite state={run ?? moment} playing={playing || run !== null} />
        </button>
      )}
    </div>
  );
}

type PetKey = "t1" | "t2" | "t3";
function PetBody({ variant = "t2", lede = true }: { variant?: PetKey; lede?: boolean }) {
  const [moment, setMoment] = useState<Moment>("idle");
  const [playing, setPlaying] = useState(!window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  return (
    <div className="ci-pet">
      {variant === "t1" && <div className="ci-pet-still"><img src={MOSSY_TILE} alt="Mossy" width={192} height={208} /></div>}
      {variant === "t2" && (
        <>
          <Desk moment={moment} playing={playing} />
          <div className="ci-pet-tools">
            <div className="ci-moments" role="group" aria-label="Codex moments">
              {MOMENTS.map(([state, label]) => (
                <button key={state} type="button" className="ci-moment" aria-pressed={moment === state} onClick={() => { setMoment(state); setPlaying(true); }}>{label}</button>
              ))}
            </div>
            <button type="button" className="btn btn-ghost ci-act" onClick={() => setPlaying((value) => !value)}>{playing ? "Pause" : "Play"}</button>
          </div>
        </>
      )}
      {variant === "t3" && (
        <div className="ci-grid">
          {MOMENTS.map(([state, label]) => (
            <button key={state} type="button" className="ci-cell" aria-pressed={moment === state} onClick={() => setMoment(state)}>
              <Sprite state={state} playing={moment === state && playing} scale={0.42} />
              <span>{label}</span>
            </button>
          ))}
        </div>
      )}
      {lede && <p className="ci-lede">{ITEM.pet.description}</p>}
    </div>
  );
}

function Body({ kind, lede = true, files = true, pet = "t2" }: { kind: KindKey; lede?: boolean; files?: boolean; pet?: PetKey }) {
  if (kind === "skill") return <SkillBody lede={lede} files={files} />;
  if (kind === "plugin") return <PluginBody intro={lede} />;
  return <PetBody variant={pet} lede={lede} />;
}

/* ——— the six pages ——— */

type Key = "a" | "b" | "c" | "d" | "e";

/** A · The website's: the body panel with a rail beside it. Under 880px of
 *  page the rail becomes one line above the panel. */
function PageA({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-page ci-a">
      <div className="ci-strip ci-a-fold"><FactLine kind={kind} /><Actions kind={kind} /></div>
      <div className="ci-hub">
        <Panel><Body kind={kind} /></Panel>
        <aside className="ci-card ci-rail"><FactList kind={kind} /><Actions kind={kind} /></aside>
      </div>
    </div>
  );
}

/** B · A store's: a hero, the body, the facts at the end. */
function PageB({ kind }: { kind: KindKey }) {
  return (
    <div className="ci-page">
      <header className="ci-hero">
        {kind === "plugin" && <p className="ci-by"><b>{PLUGIN.displayName}</b> by {PLUGIN.developer} · {PLUGIN.category}</p>}
        <p className="ci-lede">{ITEM[kind].description}</p>
        <Actions kind={kind} labelled />
      </header>
      <Panel><Body kind={kind} lede={false} /></Panel>
      <section className="ci-info"><FactList kind={kind} /></section>
    </div>
  );
}

/** C · Tabs: Overview, Files, Info. */
function PageC({ kind }: { kind: KindKey }) {
  const tabs = kind === "skill" ? ["Overview", "Files", "Info"] : ["Overview", "Info"];
  const [tab, setTab] = useState("Overview");
  return (
    <div className="ci-page">
      <div className="ci-tabs-row">
        <div className="sync-project-tabs ci-tabs" role="tablist">
          {tabs.map((label) => <button key={label} type="button" role="tab" aria-selected={tab === label} className={tab === label ? "active" : undefined} onClick={() => setTab(label)}>{label}</button>)}
        </div>
        <Actions kind={kind} />
      </div>
      {tab === "Overview" && <Panel><Body kind={kind} files={false} /></Panel>}
      {tab === "Files" && <Panel><FileTable /></Panel>}
      {tab === "Info" && <Panel><FactList kind={kind} /></Panel>}
    </div>
  );
}

/** D · The list stays; the chosen item opens beside it. */
function PageD({ kind }: { kind: KindKey }) {
  const glyph = catalogPresentation(CAPABILITY[kind]).icon;
  return (
    <div className="ci-page ci-inspector">
      <ul className="ci-list">
        {NEIGHBOURS[kind].map((name) => (
          <li key={name} className={name === ITEM[kind].name ? "is-on" : undefined}>
            <ThreadStatusIcon status={name === ITEM[kind].name ? "local-outstanding" : "synced"} size={15} />
            <Icon name={glyph} size={13} /><span>{name}</span>
          </li>
        ))}
      </ul>
      <aside className="ci-card ci-side">
        <Actions kind={kind} />
        <Body kind={kind} files={false} pet="t3" />
        <FactList kind={kind} />
      </aside>
    </div>
  );
}

/** E · A sheet over the list. */
function PageE({ kind }: { kind: KindKey }) {
  const glyph = catalogPresentation(CAPABILITY[kind]).icon;
  return (
    <div className="ci-page ci-sheet-page">
      <div className="ci-behind" aria-hidden="true">
        {NEIGHBOURS[kind].map((name) => <div key={name} className="ci-card ci-ghost"><Icon name={glyph} size={13} />{name}</div>)}
      </div>
      <div className="ci-scrim">
        <div className="ci-card ci-sheet" role="dialog" aria-label={ITEM[kind].name}>
          <div className="ci-side-head">
            <strong className="ci-sheet-name">{ITEM[kind].name}</strong>
            <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Close"><Icon name="x" size={14} /></button>
          </div>
          <Body kind={kind} files={false} pet="t3" />
          <div className="ci-strip"><FactLine kind={kind} /><Actions kind={kind} /></div>
        </div>
      </div>
    </div>
  );
}

const PAGES: Record<Key, (props: { kind: KindKey }) => ReactNode> = { a: PageA, b: PageB, c: PageC, d: PageD, e: PageE };

interface Direction { key: "today" | Key; title: string; seen: Array<[string, string]>; against?: string; pick?: boolean }

/** Title, sources, the one reason against — nothing more. */
const VSCODE = "https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/extensions/browser/extensionEditor.ts";
const JETBRAINS = "https://www.jetbrains.com/help/idea/managing-plugins.html";
const DIRECTIONS: Direction[] = [
  { key: "today", title: "Today", seen: [] },
  { key: "a", title: "A · Rail", pick: true, seen: [["Mallard Cloud", "https://mallard-ai.com"], ["VS Code", VSCODE], ["Raycast", "https://www.raycast.com/raycast/github"], ["Primer", "https://primer.style/product/components/page-layout/"]], against: "Rail from ~1300px windows; a line below" },
  { key: "b", title: "B · Hero", seen: [["App Store", "https://apps.apple.com/us/app/xcode/id497799835?mt=12"], ["VS Code", VSCODE]], against: "Facts a screen away" },
  { key: "c", title: "C · Tabs", seen: [["JetBrains", JETBRAINS], ["VS Code", VSCODE]], against: "NN/g: key content in tabs" },
  { key: "d", title: "D · Inspector", seen: [["JetBrains", JETBRAINS], ["HIG", "https://developer.apple.com/design/human-interface-guidelines/split-views"]], against: "Narrow; no link target" },
  { key: "e", title: "E · Sheet", seen: [["Mallard Add dialog", "#"]], against: "Covers Push / Pull" },
];

function Seen({ seen, against }: { seen: Array<[string, string]>; against?: string }) {
  if (seen.length === 0 && !against) return null;
  return (
    <p className="ci-seen">
      {seen.map(([label, href], index) => <Fragment key={label}>{index > 0 && " · "}{href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}</Fragment>)}
      {against && <span className="ci-against"><Icon name="alert-triangle" size={11} />{against}</span>}
    </p>
  );
}

function BodyCase({ title, pick: picked, seen, against, children }: { title: string; pick?: boolean; seen: Array<[string, string]>; against?: string; children: ReactNode }) {
  return (
    <article className="ci-case">
      <h3>{title}{picked && <em>Pick</em>}</h3>
      <Seen seen={seen} against={against} />
      <Panel>{children}</Panel>
    </article>
  );
}

/* ——— the specimen ——— */

type Open = Direction["key"];
type Theme = "light" | "dark";
const OPENS: readonly Open[] = ["today", "a", "b", "c", "d", "e"];

function Specimen() {
  const [theme, setTheme] = useState<Theme>(pick<Theme>("theme", ["light", "dark"], "dark"));
  const [kind, setKind] = useState<KindKey>(pick("kind", KINDS, "skill"));
  const [width, setWidth] = useState<Width>(pick<Width>("width", ["580", "778", "1118"], "778"));
  const [open, setOpen] = useState<Open | null>(params.has("open") ? pick<Open>("open", OPENS, "a") : null);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams({ kind, width, theme });
    if (open) query.set("open", open);
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [kind, width, theme, open]);
  const option = <T extends string>(value: T, current: T, set: (next: T) => void, label: string) => (
    <button key={value} type="button" className="ci-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const page = (key: Open) => {
    if (key === "today") return <div className="ci-frame" style={frameStyle(width)}><TodayPage kind={kind} /></div>;
    const Page = PAGES[key];
    return <Frame kind={kind} width={width} leaf={key !== "d" && key !== "e"}><Page kind={kind} /></Frame>;
  };
  const shown = open ? DIRECTIONS.find((each) => each.key === open)! : null;
  return (
    <main className="ci">
      <div className="ci-controls">
        {option("skill", kind, setKind, "Skill")}
        {option("plugin", kind, setKind, "Plugin")}
        {option("pet", kind, setKind, "Pet")}
        <i />
        {option("580", width, setWidth, "580")}
        {option("778", width, setWidth, "778")}
        {option("1118", width, setWidth, "1118")}
        <i />
        {option("light", theme, setTheme, "Light")}
        {option("dark", theme, setTheme, "Dark")}
      </div>
      {shown ? (
        <section className="ci-open">
          <h2>
            <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="All" onClick={() => setOpen(null)}><Icon name="chevron-left" size={14} /></button>
            {shown.title}{shown.pick && <em>Pick</em>}
          </h2>
          <Seen seen={shown.seen} against={shown.against} />
          {page(shown.key)}
        </section>
      ) : (
        <>
          <div className="ci-compare">
            {DIRECTIONS.map((direction) => (
              <article key={direction.key} className="ci-cell-page">
                <h3><button type="button" onClick={() => setOpen(direction.key)}>{direction.title}</button>{direction.pick && <em>Pick</em>}</h3>
                <Seen seen={direction.seen} against={direction.against} />
                <div className="ci-zoom" onClick={() => setOpen(direction.key)}>{page(direction.key)}</div>
              </article>
            ))}
          </div>
          <div className="ci-cases">
            {kind === "skill" && (
              <>
                <BodyCase title="S1 · Stacked" pick seen={[["Mallard Cloud", "https://mallard-ai.com"], ["GitHub", "https://github.com/microsoft/vscode"]]}><SkillBody /></BodyCase>
                <BodyCase title="S2 · Files beside" seen={[["GitHub file view", "https://github.com/microsoft/vscode"]]} against="Reads every file"><SkillBody variant="s2" /></BodyCase>
              </>
            )}
            {kind === "plugin" && (
              <>
                <BodyCase title="Installed" seen={[["Mallard Cloud", "https://mallard-ai.com"], ["Claude Code", "https://code.claude.com/docs/en/discover-plugins"]]}><PluginBody /></BodyCase>
                <BodyCase title="Not on this Mac" seen={[["Carbon code snippet", "https://carbondesignsystem.com/components/code-snippet/usage/"]]}><PluginBody installed={false} /></BodyCase>
              </>
            )}
            {kind === "pet" && (
              <>
                <BodyCase title="T1 · Still" seen={[]} against="No motion; upscaled"><PetBody variant="t1" /></BodyCase>
                <BodyCase title="T2 · Desk" pick seen={[["Mallard Cloud", "https://mallard-ai.com"], ["WCAG 2.2.2", "https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html"]]}><PetBody /></BodyCase>
                <BodyCase title="T3 · Grid" seen={[]} against="No precedent read"><PetBody variant="t3" /></BodyCase>
              </>
            )}
          </div>
        </>
      )}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
