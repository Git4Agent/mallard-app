import { Fragment, StrictMode, useEffect, useLayoutEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import Icon from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import { NoticeZone } from "../src/components/notice/Notice";
import SessionRowIcon from "../src/components/project-sync/SessionRowIcon";
import { ProjectChatHistoryContent } from "../src/components/project-sync/ProjectActivityPage";
import SessionDetailPage from "../src/components/project-sync/session-detail/SessionDetailPage";
import { projectSyncApi } from "../src/components/project-sync/api";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import type {
  GitCommitSummary,
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionDetail,
  ProjectSessionSummary,
  ProjectSessionTimelinePage,
  SessionDetailWarning,
  SessionEventDetail,
  SessionMessageSettings,
  SessionTimelineEvent,
  ThreadSyncComparison,
  ThreadSyncDecision,
  ThreadSyncEntry,
  ThreadSyncState,
} from "../src/types";
import "../src/App.css";
import "./stored-session-mockup.css";

/**
 * Mockup: a session that is only in a storage opens its page like any other.
 *
 * Frame 0 is the activity list's "Only in rikaqu0223-arch" tile, before and
 * after: the title becomes the row's link and the chevron appears. The page
 * frames are the shipped SessionDetailPage on canned stored data, under the
 * real path bar, each with one way of saying the copy is the stored one:
 *
 *   A   status chip     the row's badge among the header facts (SHIPPED:
 *                       the owner picked A on 2026-10-07; A's frame is the
 *                       shipped page with its `source` prop, and C's chip is
 *                       the same)
 *   B   leaf mark       the row's glyph on the path bar's leaf crumb
 *   C   chip + Pull     A, plus a Pull button where Open in Codex would be
 *   D   banner          one info notice under the bar, with Pull
 *   E   note            the sentence inside the notes chip
 *   F   named Pull      a labelled "Pull from rikaqu0223-arch" button, no chip
 *
 * Every frame shares the bar: Pull counts this session (1), Push is off
 * ("No local session to push."), and there are no launch buttons.
 *
 *   /mockups/stored-session-mockup.html?dir=all|a|b|c|d|e|f&theme=light|dark
 */

/* ——— knobs ——————————————————————————————————————————————————————————— */

type Dir = "a" | "b" | "c" | "d" | "e" | "f";
type Shown = Dir | "all";
type Theme = "light" | "dark";

const DIRS: readonly Dir[] = ["a", "b", "c", "d", "e", "f"];

function readKnobs(): { dir: Shown; theme: Theme } {
  const query = new URLSearchParams(window.location.search);
  const dir = query.get("dir") as Shown | null;
  return {
    dir: dir && (dir === "all" || DIRS.includes(dir)) ? dir : "all",
    theme: query.get("theme") === "dark" ? "dark" : "light",
  };
}

const DIR_LABEL: Record<Dir, string> = {
  a: "A · Status chip (shipped)",
  b: "B · Leaf mark",
  c: "C · Chip + Pull",
  d: "D · Banner",
  e: "E · Note",
  f: "F · Named Pull",
};

const link = (href: string, label: string) => <a href={href} target="_blank" rel="noreferrer">{label}</a>;
const SRC = {
  onedrive: link("https://support.microsoft.com/en-us/office/save-disk-space-with-onedrive-files-on-demand-for-windows-0e6860d3-d9f3-4971-b321-7092438fb38e", "OneDrive Files On-Demand"),
  drive: link("https://support.google.com/drive/answer/2375012", "Google Drive streaming"),
  dropbox: link("https://help.dropbox.com/sync/make-files-online-only", "Dropbox online-only"),
  hig: link("https://developer.apple.com/design/human-interface-guidelines/icloud", "HIG iCloud"),
  vscode: link("https://code.visualstudio.com/blogs/2021/06/10/remote-repositories", "VS Code Remote Repositories"),
  primer: link("https://primer.style/product/components/banner/guidelines/", "Primer Banner"),
  carbon: link("https://carbondesignsystem.com/components/notification/usage/", "Carbon inline notification"),
  nng: link("https://www.nngroup.com/articles/indicators-validations-notifications/", "NN/g indicators"),
};

/** One "seen in" line and one against, per direction. */
const DIR_FROM: Record<Dir, { seen: ReactNode; against: string }> = {
  a: { seen: <>{SRC.onedrive}, {SRC.drive}: a status glyph on the item itself.</>, against: "Repeats the bar’s storage name" },
  b: { seen: <>{SRC.vscode}: a remote indicator in the frame, words on hover.</>, against: "Words only in the tooltip" },
  c: { seen: <>{SRC.dropbox}, {SRC.onedrive}: opening is downloading; {SRC.hig}: the person controls the download.</>, against: "Pull twice on one screen" },
  d: { seen: <>{SRC.primer}, {SRC.carbon}; {SRC.vscode}’s limited-functionality notice.</>, against: "A state said as a message" },
  e: { seen: <>{SRC.nng}: a persistent state is an indicator; the page’s own notes rule.</>, against: "The fact behind a click" },
  f: { seen: <>{SRC.vscode}’s “Continue Working On…”; {SRC.hig}’s download control.</>, against: "A verb standing in for a fact" },
};

/* ——— data ———————————————————————————————————————————————————————————— */

const STORAGE = "rikaqu0223-arch";
const PROJECT = "mallard-with-pelican";
const ROOT = "/Users/me/project/mallard-with-pelican";
const HOME = "/Users/me";
const NOW = 1_790_000_000;
const MIN = 60;
const HOUR = 3_600;
const DAY = 86_400;
const SENTENCE = `Only in ${STORAGE}. Pull to download it here.`;
const noop = () => undefined;
const wait = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms));

type ProviderId = "codex" | "claude";

const project: LocalProjectSummary = {
  local_project_id: "project-pelican",
  bundle_id: "bundle-pelican",
  display_name: PROJECT,
  local_alias: null,
  repository_fingerprint: null,
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  profile_names: ["Default Codex", "Default Claude"],
  revision: 1,
  is_git_repository: true,
};

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: "project-pelican",
  bundle_id: "bundle-pelican",
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

/** The stored Codex session every page frame opens. */
const STORED_ID = "codex:conversation:cloudthread-01k6zq2b7m8x4y9pelbell01";
const STORED_TITLE = "Add a bell to the pelican bicycle";
const ENDED = NOW - 8 * DAY;
const T0 = ENDED - 48 * MIN;
const COMMIT_SHA = `4f2a9c1${"0".repeat(33)}`;

const COMMITS = [
  { sha: COMMIT_SHA, subject: "Add the bicycle bell", at: T0 + 41 * MIN + 10 },
  { sha: `b81e03a${"0".repeat(33)}`, subject: "Fix the wing hinge at the frame", at: NOW - 9 * DAY },
] as const;

interface Seed {
  id: string;
  provider: ProviderId;
  title: string;
  state: ThreadSyncState;
  /** A commit index, or only in the storage. */
  where: number | "storage";
  endedAgo: number;
  minutes: number;
  prompts: number;
  replies: number;
  tools: number;
  tokens: number;
  cloudId?: string;
}

const SEEDS: Seed[] = [
  { id: "01", provider: "codex", title: "Create Pelican Bicycle SVG", state: "synced", where: 0, endedAgo: 2 * DAY, minutes: 28, prompts: 2, replies: 2, tools: 6, tokens: 385_000 },
  { id: "02", provider: "claude", title: "Pelican bicycle animation", state: "local_ahead", where: 0, endedAgo: 3 * DAY, minutes: 95, prompts: 3, replies: 4, tools: 34, tokens: 5_350_000 },
  { id: "03", provider: "codex", title: "Fix the pelican’s wing hinge", state: "synced", where: 1, endedAgo: 9 * DAY + 2 * HOUR, minutes: 39, prompts: 5, replies: 6, tools: 27, tokens: 1_480_000 },
  { id: "11", provider: "codex", title: STORED_TITLE, state: "storage_only", where: "storage", endedAgo: 8 * DAY, minutes: 48, prompts: 3, replies: 3, tools: 5, tokens: 1_310_000, cloudId: STORED_ID },
  { id: "12", provider: "claude", title: "Write the pelican’s flight notes", state: "storage_only", where: "storage", endedAgo: 3 * DAY + 5 * HOUR, minutes: 17, prompts: 2, replies: 3, tools: 5, tokens: 220_000, cloudId: "claude:conversation:cloudthread-01k6zq2b7m8x4y9pelnote02" },
];

function buildSession(seed: Seed): ProjectSessionSummary {
  const name = seed.provider === "codex" ? "Codex" : "Claude";
  const uuid = `019f8a10-${seed.id.repeat(2)}-7000-8000-0000000000${seed.id}`;
  const local = seed.where !== "storage";
  const ended = NOW - seed.endedAgo;
  const input = Math.round(seed.tokens * 0.82);
  return {
    resource_id: seed.cloudId ?? `${seed.provider}:conversation:${uuid}`,
    provider: seed.provider,
    provider_display_name: name,
    provider_session_id: local ? uuid : null,
    logical_thread_id: local ? uuid : seed.cloudId?.split(":").pop() ?? null,
    source: local ? (seed.state === "local_only" ? "local_only" : "local_and_remote") : "remote_only",
    access: "bound_ready",
    title: seed.title,
    preserved: null,
    summary: null,
    started_at: ended - seed.minutes * MIN,
    ended_at: ended,
    last_prompt_at: ended - 90,
    transcript_path: local ? `${HOME}/.${seed.provider}/sessions/${uuid}.jsonl` : null,
    working_directory: local ? ROOT : null,
    is_active: false,
    user_round_count: seed.prompts,
    agent_message_count: seed.replies,
    tool_call_count: seed.tools,
    total_input_tokens: input,
    total_output_tokens: seed.tokens - input,
    total_tokens: seed.tokens,
    metrics_complete: true,
    actions: local ? [
      { action_id: `${seed.provider}.open_in_app`, kind: "open_in_app", label: `Open in ${name}`, enabled: true, disabled_reason: null },
      { action_id: `${seed.provider}.open_in_terminal`, kind: "open_in_terminal", label: "Open in Terminal", enabled: true, disabled_reason: null },
    ] : [],
  };
}

function buildEntry(session: ProjectSessionSummary, state: ThreadSyncState): ThreadSyncEntry {
  const pushes = state === "local_only" || state === "local_ahead";
  const pulls = state === "storage_only" || state === "storage_ahead";
  const push: ThreadSyncDecision = { action: state === "local_only" ? "create_cloud" : pushes ? "publish" : "no_changes", selectable: pushes };
  const pull: ThreadSyncDecision = { action: pulls ? "create_local" : "no_changes", selectable: pulls };
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
    local_present: state !== "storage_only",
    storage_present: state !== "local_only",
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: session.provider_session_id,
    cloud_thread_id: session.source === "remote_only" ? session.logical_thread_id : null,
    pull_bytes: session.source === "remote_only" ? 212_000 : null,
    push_decision: push,
    pull_decision: pull,
  };
}

const PAIRS = SEEDS.map((seed) => ({ seed, session: buildSession(seed) }));
const SESSIONS = PAIRS.map((pair) => pair.session).sort((left, right) => right.ended_at - left.ended_at);

const ACTIVITY: ProjectActivity = {
  project_id: "project-pelican",
  sessions: SESSIONS,
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true }],
    commits: COMMITS.map((commit, index): GitCommitSummary => ({
      sha: commit.sha,
      short_sha: commit.sha.slice(0, 7),
      committed_at: commit.at,
      subject: commit.subject,
      session_refs: PAIRS.filter((pair) => pair.seed.where === index).map((pair) => ({ resource_id: pair.session.resource_id })),
    })),
    github_repository_url: `https://github.com/${STORAGE}/${PROJECT}`,
    unique_session_count: SESSIONS.length,
  },
  unmapped: PAIRS.filter((pair) => pair.seed.where === "storage").map((pair) => ({ resource_id: pair.session.resource_id, reason: "no_commit" })),
  providers: [
    { provider: "codex", display_name: "Codex", access: "bound_ready" },
    { provider: "claude", display_name: "Claude", access: "bound_ready" },
  ],
  warnings: [],
  window_start: NOW - 30 * DAY,
  window_end: NOW,
  older: { newest_at: NOW - 31 * DAY, commits: 4, sessions: 11 },
  storage_sync: [],
};

const COMPARISON: ThreadSyncComparison = {
  project_id: "project-pelican",
  storage_id: "storage-cloud",
  storage_name: STORAGE,
  compared_at: NOW,
  entries: PAIRS.map(({ seed, session }) => buildEntry(session, seed.state)),
  counts: { synced: 2, local: 1, storage: 2, diverged: 0, unavailable: 0, unknown: 0 },
  scopes: [],
  provider_errors: {},
  warnings: [],
  remote_continuity: { state: "in_sync", revision: 12 },
};

const CLOUD_STATUS: ManagedCloudStatus = {
  repository_id: `repo_${STORAGE}_${PROJECT}`, state: "ready", message: null,
  link: {
    visibility: "private", can_pull: true, repository_id: `repo_${STORAGE}_${PROJECT}`, website_url: `https://mallard.example/${STORAGE}/${PROJECT}`,
    owner_login: STORAGE, name: PROJECT, release: { kind: "private" }, repository_revision: 12,
    can_request_release: true, can_publish_version: true, can_discard_draft: false, role: "owner", can_push: true,
  },
  publication: { draft: "pushed", latest_version: 3, next_version: 4, draft_ahead: 0, public_version: null, review_version: null },
};

/* ——— the stored session, as the page reads it ———————————————————————— */

const SETTINGS: SessionMessageSettings = { model: "gpt-5.6-luna", effort: "high", source: "turn" };
const exact = (value: number) => ({ value, quality: "exact" as const });

function storedDetail(dir: Dir): ProjectSessionDetail {
  const warnings: SessionDetailWarning[] = dir === "e"
    ? [{ code: "stored_copy", message: `Read from ${STORAGE}, not from this computer. Pull to keep it here.`, severity: "info" }]
    : [];
  return {
    resource_id: `${STORED_ID}#${dir}`,
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: "019f8a10-1111-7000-8000-000000000011",
    title: STORED_TITLE,
    summary: null,
    started_at: T0,
    ended_at: ENDED,
    is_active: false,
    branch: "main",
    recorded_sha: `b81e03a${"0".repeat(33)}`,
    tokens: exact(1_310_000),
    event_counts: {
      prompts: exact(3), responses: exact(3), intermediate_steps: exact(1), checkpoints: exact(1), tool_calls: exact(5),
      file_edits: exact(2), shell_calls: exact(2), reads: exact(1), other_tools: exact(0), agent_calls: exact(1),
    },
    actions: [],
    github_repository_url: `https://github.com/${STORAGE}/${PROJECT}`,
    snapshot_id: "stored-snapshot-1",
    warnings,
    agent: null,
    settings: SETTINGS,
  };
}

let ordinal = 0;
const at = (seconds: number) => T0 + seconds;
const prompt = (id: string, seconds: number, text: string, turn: string, calls: number): SessionTimelineEvent => ({
  event_id: id, ordinal: ++ordinal, timestamp: at(seconds), kind: "prompt", quality: "exact", turn_id: turn, text, settings: SETTINGS,
  turn_tool_summary: { turn_id: turn, tool_calls: exact(calls) },
});
const reply = (id: string, seconds: number, text: string, turn: string): SessionTimelineEvent => ({
  event_id: id, ordinal: ++ordinal, timestamp: at(seconds), kind: "response", quality: "exact", turn_id: turn, text, settings: SETTINGS,
});
const step = (id: string, seconds: number, text: string, turn: string): SessionTimelineEvent => ({
  event_id: id, ordinal: ++ordinal, timestamp: at(seconds), kind: "intermediate_step", quality: "exact", turn_id: turn, text,
});
const tool = (
  id: string, seconds: number, turn: string, native: string, category: "file_edit" | "shell" | "read", summary: string,
  argument: string, result: string, paths: string[],
): SessionTimelineEvent => ({
  event_id: id, ordinal: ++ordinal, timestamp: at(seconds), kind: "tool_call", quality: "exact", turn_id: turn,
  tool: {
    provider_call_id: id, native_name: native, category, status: "completed", summary,
    argument_preview: argument, argument_truncated: false, result_preview: result, result_truncated: false, affected_paths: paths,
  },
});

const EVENTS: SessionTimelineEvent[] = [
  prompt("p1", 0, "Add a bell to the pelican’s bicycle. It should ring when the brake lever is pressed.", "t1", 3),
  step("s1", 40, "Reading the bicycle SVG and the brake handler before changing either.", "t1"),
  tool("c1", 55, "t1", "read_file", "read", "Read src/pelican/bicycle.svg", "src/pelican/bicycle.svg", "1,204 lines", ["src/pelican/bicycle.svg"]),
  tool("c2", 3 * MIN, "t1", "apply_patch", "file_edit", "Edit src/pelican/bicycle.svg", "*** Update File: src/pelican/bicycle.svg\n+<g id=\"bell\" …", "Patched 1 file", ["src/pelican/bicycle.svg"]),
  tool("c3", 5 * MIN, "t1", "apply_patch", "file_edit", "Edit src/pelican/brake.ts", "*** Update File: src/pelican/brake.ts\n+  ringBell();", "Patched 1 file", ["src/pelican/brake.ts"]),
  reply("r1", 7 * MIN, "Added a `<g id=\"bell\">` group beside the handlebar in `src/pelican/bicycle.svg`, and `ringBell()` in the brake handler. The bell rings once per press.", "t1"),
  prompt("p2", 14 * MIN, "Make it ring twice on a hard brake.", "t2", 1),
  tool("c4", 16 * MIN, "t2", "shell", "shell", "npm test", "npm test", "42 passed, 0 failed", []),
  {
    event_id: "g1", ordinal: ++ordinal, timestamp: at(19 * MIN), kind: "agent_call", quality: "exact", turn_id: "t2",
    agent: {
      provider_agent_id: "agent-bell-review", label: "Review the bell for accessibility", task: "Check the new bell group has a title and the sound has a visual cue.",
      role: null, model: null, mode: "spawn", status: "completed", started_at: at(19 * MIN), ended_at: at(22 * MIN), tool_calls: 4, tokens: 61_000,
      brief: "Check the new bell group has a title and the sound has a visual cue.", brief_truncated: false,
      report: "Added `<title>Bell</title>` to the group and suggested a 120 ms flash on ring.", report_truncated: false,
      transcript_resource_id: null,
    },
  },
  reply("r2", 24 * MIN, "A hard brake (lever past 70%) now rings twice, 180 ms apart. The accessibility review added a title to the bell group.", "t2"),
  prompt("p3", 40 * MIN, "Commit it.", "t3", 1),
  tool("c5", 41 * MIN, "t3", "shell", "shell", "git commit -am 'Add the bicycle bell'", "git commit -am 'Add the bicycle bell'", "[main 4f2a9c1] Add the bicycle bell\n 2 files changed, 31 insertions(+)", []),
  {
    event_id: "k1", ordinal: ++ordinal, timestamp: COMMITS[0].at, kind: "checkpoint", quality: "exact", turn_id: null,
    checkpoint: { sha: COMMIT_SHA, short_sha: COMMIT_SHA.slice(0, 7), subject: COMMITS[0].subject, committed_at: COMMITS[0].at, branch: "main" },
  },
  reply("r3", 42 * MIN, "Committed as `4f2a9c1` on `main`.", "t3"),
];

const EVENT_DETAILS: Record<string, SessionEventDetail> = Object.fromEntries(EVENTS.filter((event) => event.tool).map((event) => [
  event.event_id,
  {
    event_id: event.event_id,
    arguments: event.tool?.argument_preview ?? null, arguments_truncated: false,
    result: event.tool?.result_preview ?? null, result_truncated: false,
    affected_paths: event.tool?.affected_paths ?? [],
  },
]));

const dirOf = (resourceId: string): Dir => (resourceId.split("#")[1] as Dir | undefined) ?? "a";

projectSyncApi.getProjectSessionDetail = (async (_projectId: string, resourceId: string) => {
  await wait(120);
  return storedDetail(dirOf(resourceId));
}) as typeof projectSyncApi.getProjectSessionDetail;
projectSyncApi.getProjectSessionTimelinePage = (async (_projectId: string, resourceId: string, page: { snapshot_id: string; included_event_kinds: string[] }) => {
  await wait(60);
  const kinds = new Set(page.included_event_kinds);
  const result: ProjectSessionTimelinePage = {
    resource_id: resourceId, snapshot_id: page.snapshot_id,
    events: EVENTS.filter((event) => kinds.has(event.kind)), next_cursor: null, warnings: [],
  };
  return result;
}) as unknown as typeof projectSyncApi.getProjectSessionTimelinePage;
projectSyncApi.getProjectSessionEventDetail = (async (_projectId: string, _resourceId: string, _snapshotId: string, eventId: string) => {
  await wait(60);
  const detail = EVENT_DETAILS[eventId];
  if (!detail) throw new Error("no such event");
  return detail;
}) as typeof projectSyncApi.getProjectSessionEventDetail;

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}

/* ——— slots: a place in the shipped markup for a proposed element ——————— */

/** A `display: contents` span placed in the first `selector` match under
 *  `root`, once it exists. The proposed element is portaled into it, so the
 *  shipped component's tree is left alone. */
function useSlot(root: HTMLElement | null, selector: string | null, where: "prepend" | "append"): HTMLElement | null {
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  useEffect(() => {
    if (!root || !selector) return undefined;
    let made: HTMLElement | null = null;
    const timer = window.setInterval(() => {
      const host = root.querySelector<HTMLElement>(selector);
      if (!host) return;
      window.clearInterval(timer);
      made = document.createElement("span");
      made.className = "ss-slot";
      host[where](made);
      setSlot(made);
    }, 40);
    return () => {
      window.clearInterval(timer);
      made?.remove();
      setSlot(null);
    };
  }, [root, selector, where]);
  return slot;
}

/* ——— frame 0: the row ————————————————————————————————————————————————— */

/** After: every stored row's title is the row's link and the chevron is
 *  there, with the shipped classes. The shipped `<strong>` is hidden. */
function StoredRowLinks({ root }: { root: HTMLElement | null }) {
  const [targets, setTargets] = useState<{ title: HTMLElement; chevron: HTMLElement; name: string }[]>([]);
  useLayoutEffect(() => {
    if (!root) return undefined;
    const made: { title: HTMLElement; chevron: HTMLElement; name: string; strong: HTMLElement }[] = [];
    root.querySelectorAll<HTMLElement>(".sync-history-thread-card").forEach((card) => {
      if (!card.querySelector(".sync-thread-sync-indicator.storage")) return;
      const strong = card.querySelector<HTMLElement>(".sync-history-thread-title > strong");
      const clickables = card.querySelector<HTMLElement>(".sync-history-row-clickables");
      if (!strong || !clickables) return;
      strong.hidden = true;
      const title = document.createElement("span");
      title.className = "ss-slot";
      strong.after(title);
      const chevron = document.createElement("span");
      chevron.className = "ss-slot";
      clickables.append(chevron);
      made.push({ title, chevron, name: strong.textContent ?? "", strong });
    });
    setTargets(made);
    return () => {
      made.forEach(({ title, chevron, strong }) => {
        title.remove();
        chevron.remove();
        strong.hidden = false;
      });
      setTargets([]);
    };
  }, [root]);
  return (
    <>
      {targets.map(({ title, chevron, name }) => (
        <Fragment key={name}>
          {createPortal(
            <button type="button" className="sync-history-session-title-button" title="View session details">{name}</button>,
            title,
          )}
          {createPortal(
            <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="View session details" title="View session details">
              <SessionRowIcon name="chevron-right" />
            </button>,
            chevron,
          )}
        </Fragment>
      ))}
    </>
  );
}

function RowFrame({ after }: { after: boolean }) {
  const [root, setRoot] = useState<HTMLElement | null>(null);
  return (
    <div className="ss-cell">
      <span>{after ? "After" : "Before"}<small>{after ? "the title is the row’s link; the chevron opens it" : "a stored row’s title is plain text"}</small></span>
      <div className={`ss-frame ss-row-frame${after ? " ss-row-after" : ""}`} ref={setRoot}>
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <ProjectChatHistoryContent
            project={project}
            binding={binding}
            activity={ACTIVITY}
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
            comparison={COMPARISON}
            activeStorageName={STORAGE}
            searchQuery=""
            onSearchQueryChange={noop}
          />
        </main>
        {after && <StoredRowLinks root={root} />}
      </div>
    </div>
  );
}

/* ——— the page frames ———————————————————————————————————————————————— */

function Bar({ dir }: { dir: Dir }) {
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: STORAGE, icon: "storage-mallard", tone: "root", title: `Mallard Cloud · ${STORAGE}` },
    { id: "project", label: PROJECT, icon: "git-folder", title: ROOT, onActivate: noop },
    {
      id: "session",
      label: STORED_TITLE,
      title: dir === "b" ? `${STORED_TITLE} · Codex\n${SENTENCE}` : `${STORED_TITLE} · Codex`,
      // B: the row's glyph is a second leaf mark after the provider's; the
      // shipped mark is a one-cell grid, so a second child would stack.
      leading: (
        <>
          <span className="sync-path-leaf-mark"><ProviderMark provider="codex" displayName="Codex" /></span>
          {dir === "b" && (
            <span className="sync-path-leaf-mark ss-leaf-status">
              <ThreadStatusIcon status="cloud-outstanding" size={14} aria-label={SENTENCE} />
            </span>
          )}
        </>
      ),
    },
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      actions={(
        <>
          <ManagedCloudChip id={`cloud-${dir}`} status={CLOUD_STATUS} storageName={STORAGE} busy={false} refreshing={false}
            onOpenUrl={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
          <button type="button" className="sync-path-icon-action" aria-label="Refresh session">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync" title={`Pull this session from ${STORAGE}`} aria-label={`Pull 1 session from ${STORAGE}`}>
              <Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span><span className="sync-path-count" aria-hidden="true">1</span>
            </button>
            <button type="button" className="sync-path-sync" disabled title="No local session to push." aria-label={`Push to ${STORAGE}`}>
              <Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span>
            </button>
          </div>
        </>
      )}
    />
  );
}

function PageFrame({ dir, head }: { dir: Dir; head: boolean }) {
  const [root, setRoot] = useState<HTMLElement | null>(null);
  const actionSlot = useSlot(root, dir === "c" || dir === "f" ? ".session-detail-header-actions" : null, "append");
  return (
    <div className={`ss-frame ss-page-frame${head ? " ss-head" : ""}`} ref={setRoot}>
      <div className="ss-app">
        <Bar dir={dir} />
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          {dir === "d" && (
            <NoticeZone
              items={[{
                key: "stored",
                severity: "info",
                title: "Not on this computer.",
                message: `Pull it from ${STORAGE} to open it in Codex.`,
                action: <button type="button" className="btn btn-ghost"><Icon name="download" size={12} />Pull</button>,
              }]}
              subject="notes about this session"
              className="session-detail-notices ss-banner"
              timing="blocking"
            />
          )}
          <SessionDetailPage
            localProjectId="project-pelican"
            projectRoot={ROOT}
            resourceId={`${STORED_ID}#${dir}`}
            occurrenceKey={`storage-only:${STORED_ID}`}
            source={dir === "a" || dir === "c"
              ? { kind: "stored", storageId: "storage-cloud", storageName: STORAGE }
              : { kind: "local" }}
            back={{ voice: "app", label: "Git history", onActivate: noop }}
            onHeaderChange={noop}
            onOpenSession={noop}
            pushGate={null}
          />
        </main>
      </div>
      {actionSlot && dir === "c" && createPortal(
        <button type="button" className="btn btn-ghost session-detail-launch" title={`Pull this session from ${STORAGE}`} aria-label={`Pull this session from ${STORAGE}`}>
          <Icon name="download" size={12} />
        </button>,
        actionSlot,
      )}
      {actionSlot && dir === "f" && createPortal(
        <button type="button" className="btn btn-ghost ss-named-pull" title={SENTENCE}>
          <Icon name="download" size={12} />Pull from {STORAGE}
        </button>,
        actionSlot,
      )}
    </div>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

function Seg<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: [T, string][]; onChange: (next: T) => void }) {
  return (
    <div className="ss-group" role="group" aria-label={label}>
      <span>{label}</span>
      {options.map(([key, text]) => (
        <button key={key} type="button" className="ss-seg" aria-pressed={value === key} onClick={() => onChange(key)}>{text}</button>
      ))}
    </div>
  );
}

function Specimen() {
  const [knobs, setKnobs] = useState(readKnobs);
  useEffect(() => {
    document.documentElement.dataset.theme = knobs.theme;
    window.history.replaceState(null, "", `?${new URLSearchParams({ dir: knobs.dir, theme: knobs.theme })}`);
  }, [knobs]);
  const shown: readonly Dir[] = knobs.dir === "all" ? DIRS : [knobs.dir];
  return (
    <div className="ss-spec">
      <h1 className="ss-title">A stored session’s page</h1>
      <p className="ss-sub">
        A row under <em>Only in {STORAGE}</em> opens like any other row. The page is the shipped one; it reads the stored transcript,
        so Pull counts this session and Push is off. Six ways to say the copy is the stored one, each with a “seen in” line and one against.
      </p>
      <div className="ss-controls">
        <Seg label="View" value={knobs.dir} options={[["all", "Compare all"], ...DIRS.map((dir): [Shown, string] => [dir, DIR_LABEL[dir]])]}
          onChange={(dir) => setKnobs({ ...knobs, dir })} />
        <Seg label="Theme" value={knobs.theme} options={[["light", "Light"], ["dark", "Dark"]]} onChange={(theme) => setKnobs({ ...knobs, theme })} />
      </div>

      <h2 className="ss-h">The row<small>the activity list’s stored tile</small></h2>
      <div className="ss-cells">
        <RowFrame after={false} />
        <RowFrame after />
      </div>

      {shown.map((dir) => (
        <Fragment key={dir}>
          <h2 className="ss-h">
            {DIR_LABEL[dir]}
            {dir === "a" && <em>shipped</em>}
            <small>against: {DIR_FROM[dir].against}</small>
          </h2>
          <p className="ss-from">Seen in: {DIR_FROM[dir].seen}</p>
          <PageFrame dir={dir} head={knobs.dir === "all"} />
        </Fragment>
      ))}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
