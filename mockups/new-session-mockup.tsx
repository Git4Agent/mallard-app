import { Fragment, StrictMode, useEffect, useId, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import SyncPathBar, { PathBarMenu, type PathBarCrumb, type PathBarMenuGroup } from "../src/components/sync-review/SyncPathBar";
import NameSearchField from "../src/components/sync-review/NameSearchField";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import ProjectViewSwitch from "../src/components/project-sync/ProjectViewSwitch";
import { BranchPicker, ProjectChatHistoryContent } from "../src/components/project-sync/ProjectActivityPage";
import { sessionNoun } from "../src/components/project-sync/model";
import type {
  GitCommitSummary,
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  SessionActionDescriptor,
  ThreadSyncComparison,
  ThreadSyncEntry,
  ThreadSyncState,
} from "../src/types";
import "../src/App.css";
import "./new-session-mockup.css";

/**
 * Mockup: start a new Codex or Claude session in the project you are on
 * (2026-10-07, docs/PLAN_NEW_SESSION.md).
 *
 * The page is the shipped `ProjectChatHistoryContent` on static data. The
 * search row is the real `NameSearchField`, drawn here so a direction can
 * close it with its control; the shipped row steps aside. Menus are the path
 * bar's own `PathBarMenu`.
 *
 * `npm run dev`, then /mockups/new-session-mockup.html
 * ?dir=all|today|a..f  ?kind=flat|git  ?agents=both|codex  ?theme=light|dark
 */

type Key = "a" | "b" | "c" | "d" | "e" | "f";
type Shown = "all" | "today" | Key;
type Kind = "flat" | "git";
type Agent = "codex" | "claude";
type Theme = "light" | "dark";

const KEYS: readonly Key[] = ["a", "b", "c", "d", "e", "f"];

function readKnobs() {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick<Shown>("dir", ["all", "today", ...KEYS], "all"),
    kind: pick<Kind>("kind", ["flat", "git"], "flat"),
    agents: pick("agents", ["both", "codex"] as const, "both"),
    theme: pick<Theme>("theme", ["light", "dark"], "light"),
  };
}

/* ——— references: a link line and the one reason against ———————————————— */

const REF = {
  zed: ["Zed", "https://zed.dev/docs/ai/agent-panel"],
  hig: ["HIG pull-down", "https://developer.apple.com/design/human-interface-guidelines/pull-down-buttons"],
  carbonMenu: ["Carbon menu button", "https://carbondesignsystem.com/components/menu-buttons/usage/"],
  carbonCombo: ["Carbon combo button", "https://carbondesignsystem.com/components/menu-buttons/usage/"],
  library: ["Mallard Library", "#"],
  vscode: ["VS Code terminal +▾", "https://code.visualstudio.com/docs/terminal/basics"],
  wt: ["Windows Terminal +▾", "https://learn.microsoft.com/en-us/windows/terminal/panes"],
  m3: ["Material split button", "https://developer.android.com/reference/kotlin/androidx/compose/material3/SplitButtonDefaults"],
  row: ["Mallard session row", "#"],
  ghProjects: ["GitHub Projects", "https://docs.github.com/en/issues/planning-and-tracking-with-projects/managing-items-in-your-project/adding-items-to-your-project"],
  ghDesktop: ["GitHub Desktop Repository menu", "https://docs.github.com/en/desktop/configuring-and-customizing-github-desktop/configuring-a-default-editor-in-github-desktop"],
  blankslate: ["Primer Blankslate", "https://primer.style/product/components/blankslate/guidelines/"],
  nng: ["NN/g empty states", "https://www.nngroup.com/articles/empty-state-interface-design/"],
  codexKeys: ["Codex app ⌘N", "https://learn.chatgpt.com/docs/reference/commands"],
  zedKeys: ["Zed ⌘N", "https://zed.dev/docs/ai/agent-panel"],
} as const satisfies Record<string, readonly [string, string]>;
type Ref = readonly [string, string];

interface Direction { key: "today" | Key; title: string; seen: Ref[]; against?: string; pick?: boolean }

const DIRECTIONS: Direction[] = [
  { key: "today", title: "Today", seen: [] },
  { key: "a", title: "A · Menu button", pick: true, seen: [REF.zed, REF.hig, REF.carbonMenu, REF.library], against: "One agent: two items (HIG ≥3)" },
  { key: "b", title: "B · Split button", seen: [REF.vscode, REF.wt, REF.carbonCombo, REF.m3], against: "Default needs remembered state" },
  { key: "c", title: "C · Launch pairs", seen: [REF.row], against: "Four bare glyphs; Carbon caps three" },
  { key: "d", title: "D · Add row", seen: [REF.ghProjects], against: "A row that isn’t a session" },
  { key: "e", title: "E · Path bar +", seen: [REF.zed, REF.wt], against: "Full 580px bar: overlaps 39px" },
  { key: "f", title: "F · Project menu", seen: [REF.ghDesktop], against: "Hidden; three identical Terminal pills" },
];

function Seen({ seen, against }: { seen: readonly Ref[]; against?: string }) {
  if (seen.length === 0 && !against) return null;
  return (
    <p className="ns-seen">
      {seen.map(([label, href], index) => (
        <Fragment key={label}>
          {index > 0 && " · "}
          {href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}
        </Fragment>
      ))}
      {against && <span className="ns-against"><Icon name="alert-triangle" size={11} />{against}</span>}
    </p>
  );
}

/* ——— the owner's project, from the screenshot ——————————————————————— */

const PROJECT = "测试文件";
const ROOT = "/Users/me/Desktop/测试文件";
const STORAGE = "Mallard Cloud";
const NOW = Math.floor(Date.parse("2026-10-07T10:00:00") / 1000);
const AT = (iso: string) => Math.floor(Date.parse(iso) / 1000);

const AGENT: Record<Agent, { name: string; mark: IconName }> = {
  codex: { name: "Codex", mark: "openai" },
  claude: { name: "Claude", mark: "claude-mark" },
};

interface Seed {
  id: string;
  provider: Agent;
  title: string;
  state: ThreadSyncState;
  ended: number;
  minutes: number;
  prompts: number;
  tools: number;
  tokens: number;
  commit: number | null;
}

const SEEDS: Seed[] = [
  { id: "01", provider: "codex", title: "绘制鹈鹕射箭 SVG", state: "synced", ended: AT("2026-10-06T21:57:00"), minutes: 2, prompts: 4, tools: 9, tokens: 542_000, commit: 0 },
  { id: "02", provider: "claude", title: "Tune the bow curve and arrow arc", state: "local_ahead", ended: AT("2026-10-06T18:20:00"), minutes: 14, prompts: 6, tools: 21, tokens: 1_180_000, commit: null },
  { id: "03", provider: "codex", title: "Export the drawing as PNG", state: "local_only", ended: AT("2026-10-05T11:02:00"), minutes: 5, prompts: 2, tools: 4, tokens: 210_000, commit: 1 },
];

function sessionActions(provider: Agent): SessionActionDescriptor[] {
  const name = AGENT[provider].name;
  return [
    { action_id: `${provider}.open_in_app`, kind: "open_in_app", label: `Open in ${name}`, enabled: true, disabled_reason: null },
    { action_id: `${provider}.open_in_terminal`, kind: "open_in_terminal", label: "Open in Terminal", enabled: true, disabled_reason: null },
  ];
}

function buildSession(seed: Seed): ProjectSessionSummary {
  const name = AGENT[seed.provider].name;
  const uuid = `019f8a10-${seed.id.repeat(2)}-7000-8000-0000000000${seed.id}`;
  const input = Math.round(seed.tokens * 0.82);
  return {
    resource_id: `${seed.provider}:conversation:${uuid}`,
    provider: seed.provider,
    provider_display_name: name,
    provider_session_id: uuid,
    logical_thread_id: uuid,
    source: seed.state === "local_only" ? "local_only" : "local_and_remote",
    access: "bound_ready",
    title: seed.title,
    preserved: null,
    summary: null,
    started_at: seed.ended - seed.minutes * 60,
    ended_at: seed.ended,
    last_prompt_at: seed.ended - 30,
    transcript_path: `/Users/me/.${seed.provider}/sessions/${uuid}.jsonl`,
    working_directory: ROOT,
    is_active: false,
    user_round_count: seed.prompts,
    agent_message_count: seed.prompts,
    tool_call_count: seed.tools,
    total_input_tokens: input,
    total_output_tokens: seed.tokens - input,
    total_tokens: seed.tokens,
    metrics_complete: true,
    actions: sessionActions(seed.provider),
  };
}

function buildEntry(session: ProjectSessionSummary, state: ThreadSyncState): ThreadSyncEntry {
  const pushes = state === "local_only" || state === "local_ahead";
  return {
    provider: session.provider,
    provider_display_name: session.provider_display_name,
    access: "bound_ready",
    thread_id: session.provider_session_id ?? session.resource_id,
    conversation_resource_id: session.resource_id,
    resource_id: session.resource_id,
    display_name: session.title,
    state,
    status_detail: null,
    local_present: true,
    storage_present: state !== "local_only",
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: session.provider_session_id,
    push_decision: { action: state === "local_only" ? "create_cloud" : pushes ? "publish" : "no_changes", selectable: pushes },
    pull_decision: { action: "no_changes", selectable: false },
  };
}

function project(kind: Kind, agents: readonly Agent[]): LocalProjectSummary {
  return {
    local_project_id: "project-test",
    bundle_id: "bundle-test",
    display_name: PROJECT,
    local_alias: null,
    repository_fingerprint: null,
    project_root: ROOT,
    canonical_project_root: ROOT,
    profile_ids: Object.fromEntries(agents.map((agent) => [agent, `profile-${agent}`])),
    profile_names: agents.map((agent) => `Default ${AGENT[agent].name}`),
    revision: 1,
    is_git_repository: kind === "git",
  };
}

function binding(agents: readonly Agent[]): ProjectBinding {
  return {
    host_id: "00112233445566778899aabbccddeeff",
    replica_id: "replica",
    local_project_id: "project-test",
    bundle_id: "bundle-test",
    project_root: ROOT,
    canonical_project_root: ROOT,
    profile_ids: Object.fromEntries(agents.map((agent) => [agent, `profile-${agent}`])),
    state: "active",
    revision: 1,
    updated_at: 2,
  };
}

const COMMITS = [
  { sha: `4f2a9c1${"0".repeat(33)}`, subject: "Add the archer drawing", at: AT("2026-10-06T22:10:00") },
  { sha: `b81e03a${"0".repeat(33)}`, subject: "Start the pelican sketch", at: AT("2026-10-05T11:30:00") },
] as const;

function buildScene(kind: Kind, agents: readonly Agent[], empty: boolean, unreadable: Agent | null) {
  const seeds = empty ? [] : SEEDS.filter((seed) => agents.includes(seed.provider));
  const pairs = seeds.map((seed) => ({ seed, session: buildSession(seed) }));
  const sessions = pairs.map((pair) => pair.session);
  const commits: GitCommitSummary[] = COMMITS.map((commit, index) => ({
    sha: commit.sha,
    short_sha: commit.sha.slice(0, 7),
    committed_at: commit.at,
    subject: commit.subject,
    session_refs: pairs.filter((pair) => pair.seed.commit === index).map((pair) => ({ resource_id: pair.session.resource_id })),
  }));
  const activity: ProjectActivity = {
    project_id: "project-test",
    sessions,
    git: kind === "git" ? {
      selected_branch: "main",
      branches: [{ name: "main", is_current: true }],
      commits,
      github_repository_url: null,
      unique_session_count: sessions.length,
    } : null,
    unmapped: kind === "git"
      ? pairs.filter((pair) => pair.seed.commit === null).map((pair) => ({ resource_id: pair.session.resource_id, reason: "no_commit" }))
      : [],
    providers: agents.map((agent) => ({
      provider: agent,
      display_name: AGENT[agent].name,
      access: agent === unreadable ? "bound_unavailable" : "bound_ready",
    })),
    warnings: [],
    window_start: NOW - 30 * 86_400,
    window_end: NOW,
    older: null,
    storage_sync: [],
  };
  const comparison: ThreadSyncComparison = {
    project_id: "project-test",
    storage_id: "storage-cloud",
    storage_name: STORAGE,
    compared_at: NOW,
    entries: pairs.map(({ seed, session }) => buildEntry(session, seed.state)),
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync", revision: 4 },
  };
  return { activity, comparison };
}

/* ——— the menu every menu direction opens ————————————————————————————— */

const noop = () => undefined;

/** One group per bound agent: its app, then Terminal. An agent whose profile
 *  can't be read keeps one inert row whose second line says why — the
 *  reason once, not on both rows. */
function newSessionGroups(agents: readonly Agent[], unreadable: Agent | null): PathBarMenuGroup[] {
  return agents.map((agent) => {
    const { name, mark } = AGENT[agent];
    if (agent === unreadable) {
      return {
        label: name,
        items: [{ id: `${agent}-off`, label: name, icon: mark, disabled: true, disabledReason: "Can’t read its profile · Agent profile…", onSelect: noop }],
      };
    }
    return {
      label: name,
      items: [
        { id: `${agent}-app`, label: `${name} app`, icon: mark, onSelect: noop },
        { id: `${agent}-terminal`, label: "Terminal", icon: "terminal", onSelect: noop },
      ],
    };
  });
}

interface MenuState { agents: readonly Agent[]; unreadable: Agent | null; open?: boolean; busy?: boolean }

/** B · the last choice one press away; the chevron holds them all. */
function NewSessionSplit({ agents, unreadable, open: startOpen = false, busy = false }: MenuState) {
  const [open, setOpen] = useState(startOpen);
  const id = useId();
  const first = agents.find((agent) => agent !== unreadable) ?? agents[0];
  return (
    <div className="ns-menu ns-split" role="group" aria-label="New session">
      <button type="button" className="btn" disabled={busy || first === unreadable} title={`New ${AGENT[first].name} session in the ${AGENT[first].name} app`}>
        {busy ? <Icon name="refresh" size={13} className="icon-spin" /> : <Icon name="plus" size={13} />}
        {AGENT[first].name} app
      </button>
      <button
        type="button"
        className="btn"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="More ways to start a session"
        disabled={busy}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => setOpen(!open)}
      >
        <Icon name="chevron-down" size={13} />
      </button>
      <PathBarMenu id={id} groups={newSessionGroups(agents, unreadable)}
        label="New session" align="end" open={open} registerRef={noop} onClose={() => setOpen(false)} />
    </div>
  );
}

/** C, D · the session row's own launch pair, once per bound agent. */
function LaunchPairs({ agents, unreadable, busy = false }: MenuState) {
  return (
    <>
      {agents.map((agent, index) => {
        const { name } = AGENT[agent];
        const off = agent === unreadable;
        const why = off ? ` — can’t read the ${name} profile` : "";
        return (
          <span key={agent} className="ns-pair">
            {index > 0 && <span className="ns-pair-rule" aria-hidden="true" />}
            <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={off}
              title={`New ${name} session in the ${name} app${why}`} aria-label={`New ${name} session in the ${name} app`}>
              {busy && index === 0 ? <Icon name="refresh" size={14} className="icon-spin" /> : <ProviderMark provider={agent} displayName={name} />}
            </button>
            <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={off}
              title={`New ${name} session in Terminal${why}`} aria-label={`New ${name} session in Terminal`}>
              <Icon name="terminal" size={14} />
            </button>
          </span>
        );
      })}
    </>
  );
}

/* ——— one frame: the path bar, the search row, the shipped page ———————————— */

/** `full` is the bar the 2026-10-02 width sweep measured: attention chip,
 *  Pull 12, Push 40. */
function Bar({ dir, kind, menu, full = false }: { dir: "today" | Key; kind: Kind; menu: MenuState; full?: boolean }) {
  const [barOpen, setBarOpen] = useState(dir === "e" && !!menu.open);
  const verbs = dir === "f"
    ? menu.agents.flatMap((agent) => {
      const { name, mark } = AGENT[agent];
      return [
        { id: `${agent}-app`, label: `New ${name} session`, detail: `${name} app`, icon: mark, onSelect: noop },
        { id: `${agent}-terminal`, label: `New ${name} session`, detail: "Terminal", icon: "terminal" as const, onSelect: noop },
      ];
    })
    : [];
  const crumbs: PathBarCrumb[] = [
    {
      id: "storage", label: STORAGE, icon: "storage-mallard", tone: "root", menuLabel: "Storage",
      menuHeader: { label: STORAGE, detail: "Mallard Cloud", icon: "storage-mallard" },
      menu: [{ items: [{ id: "storage-settings", label: "Storage details…", icon: "settings", onSelect: noop }] }],
    },
    {
      id: "project", label: PROJECT, icon: kind === "git" ? "git-folder" : "folder", menuLabel: "Project",
      scope: kind === "git" ? (
        <>
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <BranchPicker branches={[{ name: "main", is_current: true }]} selectedBranch="main" disabled={false} onChange={noop} />
        </>
      ) : undefined,
      menuHeader: { label: PROJECT, detail: "~/Desktop/测试文件", detailIsPath: true, icon: "folder" },
      menu: [{
        items: [
          { id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop },
          ...verbs,
          { id: "agent", label: "Agent profile…", detail: menu.agents.map((agent) => `Default ${AGENT[agent].name}`).join(" · "), icon: "terminal", onSelect: noop },
          { id: "rename", label: "Rename project…", icon: "edit", onSelect: noop },
          { id: "delete", label: "Delete all local sessions…", icon: "trash", danger: true, onSelect: noop },
        ],
      }],
    },
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      openMenuRequest={dir === "f" && menu.open ? { menuId: "project", requestId: 1 } : undefined}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: noop }] }]}
      overflowLabel="App actions"
      actions={(
        <>
          {dir === "e" && (
            <div className="sync-path-overflow ns-bar-new">
              <button type="button" className="sync-path-icon-action" title="New session" aria-label="New session"
                aria-haspopup="menu" aria-expanded={barOpen} disabled={menu.busy}
                onPointerDown={(event) => event.preventDefault()} onClick={() => setBarOpen(!barOpen)}>
                {menu.busy ? <Icon name="refresh" size={16} className="icon-spin" /> : <Icon name="plus" size={16} />}
              </button>
              <PathBarMenu id="ns-bar-menu" groups={newSessionGroups(menu.agents, menu.unreadable)} label="New session"
                align="end" open={barOpen} registerRef={noop} onClose={() => setBarOpen(false)} />
            </div>
          )}
          {full && (
            <span className="sync-path-attention warning" tabIndex={0} data-tooltip="1 thread changed on both sides, 1 thread waiting to sync.">
              <Icon name="alert-triangle" size={13} />
              <b>2</b>
            </span>
          )}
          <button type="button" className="sync-path-icon-action" title="Mallard Cloud: up to date" aria-label="Mallard Cloud: up to date">
            <Icon name="cloud-check" size={16} />
          </button>
          <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync">
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              {full && <span className="sync-path-count" aria-hidden="true">12</span>}
            </button>
            <button type="button" className="sync-path-sync">
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              <span className="sync-path-count" aria-hidden="true">{full ? 40 : 2}</span>
            </button>
          </div>
        </>
      )}
    />
  );
}

interface FrameProps {
  dir: "today" | Key;
  kind: Kind;
  agents: readonly Agent[];
  open?: boolean;
  busy?: boolean;
  unreadable?: Agent | null;
  empty?: boolean;
  clip?: number;
  width?: number;
  full?: boolean;
}

function Frame({ dir, kind, agents, open = false, busy = false, unreadable = null, empty = false, clip, width, full = false }: FrameProps) {
  const { activity, comparison } = buildScene(kind, agents, empty, unreadable);
  const [query, setQuery] = useState("");
  const menu: MenuState = { agents, unreadable, open, busy };
  const viewSwitch = kind === "flat" ? <ProjectViewSwitch view="sessions" onChange={noop} /> : undefined;
  /* A shipped (2026-10-07): its frames are the app's own page, with the
     real menu; an "open" frame presses it once mounted. */
  const shipped = dir === "a";
  const swap = dir !== "today" && !shipped;
  const frameRef = useRef<HTMLDivElement>(null);
  /* StrictMode runs the effect twice; the menu must be pressed once. */
  const pressed = useRef(false);
  useEffect(() => {
    if (!shipped || !open || pressed.current) return;
    pressed.current = true;
    frameRef.current?.querySelector<HTMLButtonElement>(".sync-new-session > button")?.click();
  }, [shipped, open]);
  const total = activity.sessions.length;
  const matches = activity.sessions.filter((session) => session.title.toLowerCase().includes(query.trim().toLowerCase())).length;

  const trailing = dir === "b" ? <NewSessionSplit {...menu} />
        : dir === "c" ? (
          <div className="ns-pairs" role="group" aria-label="New session">
            <span className="ns-pairs-cap"><Icon name="plus" size={13} />New</span>
            <LaunchPairs {...menu} />
          </div>
        )
          : undefined;

  const head = swap && (
    <NameSearchField query={query} onChange={setQuery} label="Search loaded sessions" matches={matches} total={total}
      noun={sessionNoun} leading={viewSwitch} trailing={trailing} />
  );

  const addRow = dir === "d" && (
    <div className="sync-history-thread-list flat ns-add-list">
      <div className="ns-add-row">
        <span className="ns-add-title"><Icon name="plus" size={14} />New session</span>
        <span className="ns-add-actions"><LaunchPairs {...menu} /></span>
      </div>
    </div>
  );

  const page = (
    <ProjectChatHistoryContent
      project={project(kind, agents)}
      binding={binding(agents)}
      activity={activity}
      loading={false}
      loadingMore={false}
      loadError={null}
      onRetryLoad={noop}
      actionBusyResourceId={null}
      onLoadMore={noop}
      onShowLess={noop}
      canShowLess={false}
      onSessionAction={noop}
      onDeleteSession={noop}
      onOpenSessionDetail={noop}
      comparison={comparison}
      activeStorageName={STORAGE}
      storageId="storage-cloud"
      viewSwitch={viewSwitch}
      searchQuery={query}
      onSearchQueryChange={setQuery}
      onStartSession={shipped ? noop : undefined}
      startSessionBusy={shipped && busy}
    />
  );

  return (
    <div ref={frameRef} className={`spec-frame${swap ? " ns-swap" : ""}`} style={width ? { width } : undefined}>
      <div className={clip ? "sync-app spec-clip" : "sync-app"} style={clip ? { ["--clip" as string]: `${clip}px` } : undefined}>
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <Bar dir={dir} kind={kind} menu={menu} full={full} />
          <div className="sync-project-tab-panel" role="tabpanel">
            {head}
            {addRow}
            {page}
          </div>
        </main>
      </div>
    </div>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const LIVE = new URLSearchParams(window.location.search).get("live");

interface Cell { title: string; props: Omit<FrameProps, "dir" | "kind" | "agents"> & { agents?: readonly Agent[] }; only?: readonly Key[]; not?: readonly Key[] }

const MENUS: readonly Key[] = ["a", "b", "e", "f"];

const CELLS: Cell[] = [
  { title: "At rest", props: {} },
  { title: "Open", only: MENUS, props: { open: true, clip: 360 } },
  { title: "Codex only", props: { agents: ["codex"], open: true, clip: 330 } },
  { title: "Claude profile unreadable", props: { unreadable: "claude", open: true, clip: 360 } },
  { title: "Opening", props: { busy: true } },
  { title: "Empty project", only: ["a"], props: { empty: true } },
  { title: "580px bar", props: { width: 618 } },
  { title: "580px bar, full", only: ["e"], props: { width: 618, full: true } },
];

function Page() {
  const initial = readKnobs();
  const [dir, setDir] = useState<Shown>(initial.dir);
  const [kind, setKind] = useState<Kind>(initial.kind);
  const [agentsKnob, setAgentsKnob] = useState<"both" | "codex">(initial.agents);
  const [theme, setTheme] = useState<Theme>(initial.theme);
  document.documentElement.dataset.theme = theme;
  const agents: readonly Agent[] = agentsKnob === "both" ? ["codex", "claude"] : ["codex"];
  const seg = <T extends string>(value: T, current: T, set: (value: T) => void, label: string) => (
    <button key={value} type="button" className="spec-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const shown = DIRECTIONS.find((direction) => direction.key === dir);
  /* F's open menu takes focus, as the bar's own request does, and that
     scrolls the page; the specimen opens at its head. */
  useEffect(() => {
    const timer = window.setTimeout(() => { document.getElementById("root")!.scrollTop = 0; }, 200);
    return () => window.clearTimeout(timer);
  }, [dir, kind, agentsKnob]);
  return (
    <div className="spec">
      <h1 className="spec-title">New session in this project</h1>
      <div className="spec-controls" role="group" aria-label="Specimen controls">
        <div className="spec-group">
          {seg<Shown>("all", dir, setDir, "Compare")}
          {DIRECTIONS.map((direction) => seg<Shown>(direction.key, dir, setDir, direction.title))}
        </div>
        <div className="spec-group">{seg("flat", kind, setKind, "Non-Git")}{seg("git", kind, setKind, "Git")}</div>
        <div className="spec-group">{seg("both", agentsKnob, setAgentsKnob, "Codex + Claude")}{seg("codex", agentsKnob, setAgentsKnob, "Codex only")}</div>
        <div className="spec-group">{seg("light", theme, setTheme, "Light")}{seg("dark", theme, setTheme, "Dark")}</div>
      </div>

      {dir === "all" ? (
        <>
          <div className="spec-cells">
            {DIRECTIONS.map((direction) => (
              <section key={direction.key} className={`spec-cell${direction.pick ? " pick" : ""}`}>
                <h2 className="spec-h">{direction.title}{direction.pick && <em>picked, shipped</em>}</h2>
                <Seen seen={direction.seen} against={direction.against} />
                <Frame key={`${direction.key}:${kind}:${agentsKnob}`} dir={direction.key} kind={kind} agents={agents}
                  open={MENUS.includes(direction.key as Key)} clip={360} />
              </section>
            ))}
          </div>
          <section className="spec-with">
            <h2 className="spec-h">With A · empty project</h2>
            <Seen seen={[REF.blankslate, REF.nng]} />
            <Frame key={`blank:${kind}:${agentsKnob}`} dir="a" kind="flat" agents={agents} empty clip={260} width={720} />
            <Seen seen={[REF.codexKeys, REF.zedKeys]} against="⌘N opens the same menu" />
          </section>
        </>
      ) : shown && (
        <>
          <h2 className="spec-h">{shown.title}{shown.pick && <em>picked, shipped</em>}</h2>
          <Seen seen={shown.seen} against={shown.against} />
          {/* `?live=open|unreadable|empty` opens or empties the live frame
              alone: the shipped menu closes when another takes focus, so the
              cells below cannot all stand open at once. */}
          <div className="spec-live"><Frame key={`live:${dir}:${kind}:${agentsKnob}`} dir={shown.key} kind={kind} agents={agents}
            open={LIVE === "open" || LIVE === "unreadable"} unreadable={LIVE === "unreadable" ? "claude" : null} empty={LIVE === "empty"} /></div>
          {dir !== "today" && !LIVE && (
            <div className="spec-cells">
              {CELLS.filter((cell) => (!cell.only || cell.only.includes(dir as Key)) && !cell.not?.includes(dir as Key)).map((cell) => {
                const { agents: cellAgents, open, ...props } = cell.props;
                return (
                  <section key={cell.title} className="spec-cell">
                    <h3 className="spec-h3">{cell.title}</h3>
                    <Frame key={`${cell.title}:${dir}:${kind}:${agentsKnob}`} dir={dir as Key} kind={kind}
                      agents={cellAgents ?? agents} open={open && MENUS.includes(dir as Key)} clip={props.clip ?? 260} {...props} />
                  </section>
                );
              })}
            </div>
          )}
        </>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
