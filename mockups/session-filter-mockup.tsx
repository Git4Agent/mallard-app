import { StrictMode, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../src/components/sync-review/ThreadStatusIcon";
import { nameMatchesSearch } from "../src/components/sync-review/nameSearch";
import ProjectViewSwitch from "../src/components/project-sync/ProjectViewSwitch";
import {
  ProjectChatHistoryContent,
  sessionStatusScope,
  sessionSyncPresentation,
} from "../src/components/project-sync/ProjectActivityPage";
import type { StatusScope } from "../src/components/sync-review/statusScopes";
import { sessionNoun } from "../src/components/project-sync/model";
import type {
  ConversationBlocker,
  GitCommitSummary,
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionSummary,
  SessionActionDescriptor,
  ThreadSyncComparison,
  ThreadSyncDecision,
  ThreadSyncEntry,
  ThreadSyncState,
} from "../src/types";
import "../src/App.css";

/**
 * Mockup: filter the activity list by a session's sync status.
 *
 * The list is the shipped `ProjectChatHistoryContent` on static data, so the
 * rows, the rail and the paging foot are the app's own.
 *
 *   A       a chip per status in the search row: glyph + count
 *   B       the same chips with the status named
 *   C       a scope bar under the search row: one scope at a time
 *   D       one Status button closing the row, with a check menu
 *   E       the session page's Show bar: all on, press to hide
 *
 * The owner picked C on 2026-10-03 and it ships: C's frames are the app's own
 * page with its own scope bar, preset through the list's `statusScope` prop.
 * In the other four the search row and its control are drawn here, the
 * shipped row and bar are hidden under them, and the mock narrows the data it
 * hands the page.
 *
 * `npm run dev`, then /mockups/session-filter-mockup.html
 * (`?dir=all|a|b|c|d|e&kind=git|flat&theme=light|dark`; `all` sets the five
 * side by side).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Dir = "a" | "b" | "c" | "d" | "e";
type Kind = "git" | "flat";
type Theme = "light" | "dark";
type Scene = "mixed" | "every" | "one" | "comparing" | "pushed";

const DIRS = ["a", "b", "c", "d", "e"] as const;

/** One direction in full, or all five heads side by side. */
type Shown = Dir | "all";

function readKnobs(): { dir: Shown; kind: Kind; theme: Theme } {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick<Shown>("dir", ["all", ...DIRS], "all"),
    kind: pick("kind", ["git", "flat"] as const, "git"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/** The owner's pick, 2026-10-03. Its frames are the shipped page. */
const SHIPPED: Dir = "c";

const DIR_LABEL: Record<Dir, string> = {
  a: "A · glyph chips",
  b: "B · named chips",
  c: "C · scope bar",
  d: "D · Status menu",
  e: "E · Show bar",
};

const link = (href: string, label: string) => <a href={href} target="_blank" rel="noreferrer">{label}</a>;

const SRC = {
  material: link("https://m3.material.io/components/chips/guidelines", "Material 3 filter chips"),
  hig: link("https://developer.apple.com/design/human-interface-guidelines/search-fields", "Apple HIG search fields"),
  primer: link("https://primer.style/product/scenario-patterns/filter/", "Primer filter pattern"),
  carbon: link("https://carbondesignsystem.com/patterns/filtering/", "Carbon filtering"),
  nng: link("https://www.nngroup.com/articles/applying-filters/", "NN/g on applying filters"),
  apg: link("https://www.w3.org/WAI/ARIA/apg/patterns/button/", "ARIA APG button"),
  gmail: link("https://workspaceupdates.googleblog.com/2020/02/gmail-search-chips-ga.html", "Gmail search chips"),
  jira: link("https://support.atlassian.com/jira-software-cloud/docs/show-or-hide-issues-on-your-board/", "Jira quick filters"),
  github: link("https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/filtering-and-searching-issues-and-pull-requests", "GitHub issue lists"),
  mail: link("https://support.apple.com/guide/mail/mlhl1f6cf15a/mac", "Mail’s Filter button"),
  linear: link("https://linear.app/docs/filters", "Linear filters"),
  devtools: link("https://developer.chrome.com/docs/devtools/console/reference", "Chrome DevTools Console"),
  vscode: link("https://code.visualstudio.com/docs/getstarted/userinterface", "VS Code Problems"),
};

const DIR_NOTES: Record<Dir, ReactNode[]> = {
  a: [
    <><b>One chip per status the loaded rows carry</b>, in the search row: the row’s own glyph and how many rows wear it. No new line of chrome unless all eight statuses meet a narrow pane.</>,
    <><b>Press to narrow.</b> Nothing pressed shows everything. Press one and the list keeps only those rows; press more to add statuses; press again, or ×, to release.</>,
    <>The status is named only in the tooltip, where the row badge names it. The standards below all put a word on a filter option.</>,
  ],
  b: [
    <><b>A’s chips with the status named.</b> The row doubles as the legend the row glyphs never had.</>,
    <><b>Press to narrow</b>, the same rule as A. The chips take one line under the field, inside the search row’s rule.</>,
    <>The chip form the references describe: a short word and a count on each option, several on at once, an edge that marks what is in force, and one control that clears it. Pick this over C if combining statuses matters.</>,
  ],
  c: [
    <><b>One scope at a time:</b> All, To push, To pull, Attention, Up to date. The words and groups are the path bar’s own: its two verbs and its attention chip.</>,
    <>A row has exactly one status, so the statuses split the list into parts, and a scope bar is the standard control for choosing one part. One press switches, All is the way back, and there is no separate clear control.</>,
    <>It cannot combine scopes: “everything except up to date” needs B. Costs a 36px strip whenever the list has two statuses.</>,
  ],
  d: [
    <><b>One Status button</b> closing the search row. Its menu lists each status by glyph, name and count, with a tick. The button names what is in force.</>,
    <><b>Tick to narrow</b>, the same rule as A and B. No space is taken until it is opened.</>,
    <>Every look and every change costs a click, and the counts are hidden until then.</>,
  ],
  e: [
    <><b>The session page’s Show bar</b>, as it ships, under the search row: every status on, with its name and count.</>,
    <><b>Press to hide.</b> One press hides “Up to date”. Seeing only one status takes a press for every other status.</>,
    <>Costs a 44px box whenever the list has two statuses.</>,
  ],
};

/** One line each, for the side-by-side view. */
const DIR_SUMMARY: Record<Dir, string> = {
  a: "press to narrow; several at once; names in the tooltip",
  b: "press to narrow; several at once; named",
  c: "one scope at a time; All is the way back",
  d: "tick to narrow, behind one button",
  e: "all on; press to hide",
};

const PUSH_COST: Record<Dir, string> = {
  a: "1 press",
  b: "1 press",
  c: "1 press",
  d: "2 presses",
  e: "4 presses",
};

const DIR_FROM: Record<Dir, ReactNode> = {
  a: <>Icon toggles with tallies: {SRC.vscode} (errors, warnings, infos), the {SRC.devtools} sidebar.</>,
  b: <>{SRC.material}, {SRC.gmail}, {SRC.jira}; counts and reset per {SRC.primer} and {SRC.carbon}.</>,
  c: <>{SRC.hig} (scope bar), the Open / Closed switch that heads {SRC.github}.</>,
  d: <>{SRC.primer} (a menu for a small set, selection shown on the trigger), {SRC.mail}, {SRC.linear}.</>,
  e: <>All-on level filters: {SRC.devtools} log levels, {SRC.vscode}; {SRC.carbon} allows either starting state.</>,
};

/** What the references ask of a list filter, and what the specimen does about it. */
const STANDARDS: { source: ReactNode; says: ReactNode; here: ReactNode }[] = [
  {
    source: SRC.material,
    says: <>Options are short words in a compact row; several can be on at once; a selected chip is marked. One set is either single- or multi-select, never both.</>,
    here: <>B. A is the same set without the words.</>,
  },
  {
    source: SRC.hig,
    says: <>A scope bar narrows a search among clearly defined categories. Open on the broad scope and let people narrow it.</>,
    here: <>C. Every direction opens on everything.</>,
  },
  {
    source: SRC.primer,
    says: <>Size the control to the task: a menu or a few exposed options for a small known set. Put it at the top of what it governs, show the selection on the trigger, pair each count with a label, never mark “active” by colour alone, give an empty result a way back, and announce the new count.</>,
    here: <>D’s button; counts beside names; the doubled ink edge; “Clear filter” in the empty box; the live “3 of 12 sessions”.</>,
  },
  {
    source: SRC.carbon,
    says: <>Apply at once when one category is filtered. While a filter is in force, show that it is and let it be cleared in one step. Start all-off or all-on by whether people mostly include or exclude.</>,
    here: <>No Apply step. × and “Clear filter”. A to D start all-off (include); E starts all-on (exclude).</>,
  },
  {
    source: SRC.nng,
    says: <>Instant filtering suits a list that answers within a second and a reader who is exploring; each option should say how many results it holds.</>,
    here: <>The list is already in memory. Every option carries its count.</>,
  },
  {
    source: SRC.apg,
    says: <>A toggle is a button with a pressed state, and its label does not change when it is pressed.</>,
    here: <>Chips are <code>aria-pressed</code> buttons named “To push: 3 sessions”; C is a radio group.</>,
  },
];

/* ——— the statuses ————————————————————————————————————————————————— */

type StatusKey = ThreadStatusIconName;

/** Work first, settled last, unjudged at the end. The order never changes,
 *  so a chip keeps its place as others come and go. */
const STATUS_ORDER: readonly StatusKey[] = [
  "local-outstanding", "cloud-outstanding", "divergent", "pending", "conflict", "unavailable", "synced", "unknown",
];

const STATUS_NAME: Record<StatusKey, string> = {
  "local-outstanding": "To push",
  "cloud-outstanding": "To pull",
  divergent: "Changed on both sides",
  pending: "Waiting",
  conflict: "Can’t match",
  unavailable: "Can’t sync",
  synced: "Up to date",
  unknown: "Not compared",
  untracked: "Not compared",
  checking: "Comparing",
};

/* ——— data ——————————————————————————————————————————————————————— */

const STORAGE = "myconfstorage";
const PROJECT = "HorrorGame";
const ROOT = "/Users/me/project/HorrorGame";
const HOME = "/Users/me";
const NOW = 1_790_000_000;
const HOUR = 3_600;
const DAY = 86_400;

type ProviderId = "codex" | "claude";

const project = (git: boolean): LocalProjectSummary => ({
  local_project_id: "project-horror",
  bundle_id: "bundle-horror",
  display_name: PROJECT,
  local_alias: null,
  repository_fingerprint: null,
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  profile_names: ["Default Codex", "Default Claude"],
  revision: 1,
  is_git_repository: git,
});

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: "project-horror",
  bundle_id: "bundle-horror",
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex", claude: "profile-claude" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

const COMMITS = [
  { sha: `4f2a9c1${"0".repeat(33)}`, subject: "Add horror story document", ago: 4 * DAY },
  { sha: `b81e03a${"0".repeat(33)}`, subject: "Fix door collision at the cellar stairs", ago: 9 * DAY },
  { sha: `0c77d2e${"0".repeat(33)}`, subject: "Bump engine to 4.3", ago: 12 * DAY },
] as const;

type Hold = Extract<ConversationBlocker, "busy" | "unknown_evidence">;

interface Seed {
  id: string;
  provider: ProviderId;
  title: string;
  /** Null draws a row the comparison has no entry for. */
  state: ThreadSyncState | null;
  hold?: Hold;
  /** A commit index, or one of the two unmapped groups. */
  where: number | "uncommitted" | "storage";
  endedAgo: number;
  minutes: number;
  prompts: number;
  replies: number;
  tools: number;
  tokens: number;
}

const SEEDS: Seed[] = [
  { id: "01", provider: "codex", title: "Create Pelican Bicycle SVG", state: "local_only", where: "uncommitted", endedAgo: 2 * HOUR, minutes: 28, prompts: 2, replies: 2, tools: 6, tokens: 385_000 },
  { id: "02", provider: "codex", title: "Create Pelican Bicycle SVG", state: "local_ahead", where: "uncommitted", endedAgo: 3 * HOUR, minutes: 45, prompts: 5, replies: 5, tools: 10, tokens: 416_000 },
  { id: "03", provider: "claude", title: "Pelican bicycle animation", state: "synced", where: "uncommitted", endedAgo: DAY, minutes: 95, prompts: 3, replies: 4, tools: 34, tokens: 5_350_000 },
  { id: "04", provider: "codex", title: "Improve game interaction and collision", state: "diverged", where: "uncommitted", endedAgo: 2 * DAY, minutes: 110, prompts: 4, replies: 4, tools: 78, tokens: 10_160_000 },
  { id: "05", provider: "codex", title: "Add horror story document", state: "synced", where: 0, endedAgo: 5 * DAY, minutes: 52, prompts: 6, replies: 7, tools: 41, tokens: 2_240_000 },
  { id: "06", provider: "claude", title: "Draft the basement chase scene", state: "local_ahead", where: 0, endedAgo: 6 * DAY, minutes: 74, prompts: 9, replies: 11, tools: 63, tokens: 3_900_000 },
  { id: "07", provider: "claude", title: "Balance lantern battery drain", state: "local_ahead", hold: "busy", where: 0, endedAgo: 7 * DAY, minutes: 31, prompts: 4, replies: 4, tools: 19, tokens: 1_120_000 },
  { id: "08", provider: "codex", title: "Fix door collision at the cellar stairs", state: "synced", where: 1, endedAgo: 10 * DAY, minutes: 39, prompts: 5, replies: 6, tools: 27, tokens: 1_480_000 },
  { id: "09", provider: "claude", title: "Tune ghost pathing through locked doors", state: "synced", where: 1, endedAgo: 11 * DAY, minutes: 126, prompts: 12, replies: 14, tools: 97, tokens: 6_020_000 },
  { id: "10", provider: "codex", title: "Menu flow and pause screen", state: "synced", where: 1, endedAgo: 11.5 * DAY, minutes: 22, prompts: 3, replies: 3, tools: 12, tokens: 640_000 },
  { id: "11", provider: "claude", title: "Record the ambient audio list", state: "storage_only", where: "storage", endedAgo: 3 * DAY, minutes: 17, prompts: 2, replies: 3, tools: 5, tokens: 220_000 },
  { id: "12", provider: "codex", title: "Sketch the attic map", state: "storage_only", where: "storage", endedAgo: 8 * DAY, minutes: 48, prompts: 6, replies: 6, tools: 22, tokens: 1_310_000 },
];

/** Each scene is the same twelve sessions with their verdicts changed. */
function sceneSeeds(scene: Scene): Seed[] {
  switch (scene) {
    case "every":
      return SEEDS.map((seed) => (
        seed.id === "03" ? { ...seed, state: "local_ahead", hold: "unknown_evidence" }
          : seed.id === "09" ? { ...seed, state: "unavailable" }
            : seed.id === "10" ? { ...seed, state: null }
              : seed
      ));
    case "one":
      return SEEDS.filter((seed) => seed.where !== "storage").map((seed) => ({ ...seed, state: "local_only", hold: undefined }));
    case "pushed":
      return SEEDS.map((seed) => (
        (seed.state === "local_only" || seed.state === "local_ahead") && !seed.hold ? { ...seed, state: "synced" } : seed
      ));
    default:
      return SEEDS;
  }
}

function sessionActions(provider: ProviderId, name: string): SessionActionDescriptor[] {
  return [
    { action_id: `${provider}.open_in_app`, kind: "open_in_app", label: `Open in ${name}`, enabled: true, disabled_reason: null },
    { action_id: `${provider}.open_in_terminal`, kind: "open_in_terminal", label: "Open in Terminal", enabled: true, disabled_reason: null },
  ];
}

function buildSession(seed: Seed): ProjectSessionSummary {
  const name = seed.provider === "codex" ? "Codex" : "Claude";
  const uuid = `019f8a10-${seed.id.repeat(2)}-7000-8000-0000000000${seed.id}`;
  const local = seed.where !== "storage";
  const ended = NOW - seed.endedAgo;
  const input = Math.round(seed.tokens * 0.82);
  return {
    resource_id: `${seed.provider}:conversation:${uuid}`,
    provider: seed.provider,
    provider_display_name: name,
    provider_session_id: uuid,
    logical_thread_id: uuid,
    source: !local ? "remote_only" : seed.state === "local_only" ? "local_only" : "local_and_remote",
    access: "bound_ready",
    title: seed.title,
    preserved: null,
    summary: null,
    started_at: ended - seed.minutes * 60,
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
    actions: local ? sessionActions(seed.provider, name) : [],
  };
}

function buildEntry(session: ProjectSessionSummary, state: ThreadSyncState, hold?: Hold): ThreadSyncEntry {
  const pushes = state === "local_only" || state === "local_ahead";
  const pulls = state === "storage_only" || state === "storage_ahead";
  const push: ThreadSyncDecision = hold
    ? { action: "blocked", blocker: hold, selectable: false }
    : state === "diverged"
      ? { action: "require_pull", selectable: false }
      : { action: state === "local_only" ? "create_cloud" : pushes ? "publish" : "no_changes", selectable: pushes };
  const pull: ThreadSyncDecision = state === "diverged"
    ? { action: "keep_both", selectable: true }
    : { action: pulls ? "create_local" : "no_changes", selectable: pulls };
  return {
    provider: session.provider,
    provider_display_name: session.provider_display_name,
    access: "bound_ready",
    thread_id: session.provider_session_id ?? session.resource_id,
    conversation_resource_id: session.resource_id,
    resource_id: session.resource_id,
    display_name: session.title,
    state,
    status_detail: state === "unavailable" ? "Its transcript holds a record Mallard can’t read." : null,
    local_present: state !== "storage_only",
    storage_present: state !== "local_only",
    compared_against_storage: true,
    subagent_selection: { wire_children: "on", keys: [], omitted: false, estimated_bytes: 0 },
    provider_thread_id: session.provider_session_id,
    push_decision: push,
    pull_decision: pull,
  };
}

interface SceneData {
  activity: ProjectActivity;
  comparison: ThreadSyncComparison | null;
  /** The status each loaded row's badge shows, resolved the way the row does. */
  statusByResource: Map<string, StatusKey>;
}

function buildScene(scene: Scene, kind: Kind): SceneData {
  const seeds = sceneSeeds(scene);
  const pairs = seeds.map((seed) => ({ seed, session: buildSession(seed) }));
  const sessions = pairs.map((pair) => pair.session).sort((left, right) => right.ended_at - left.ended_at);
  const entries = pairs.flatMap(({ seed, session }) => (seed.state ? [buildEntry(session, seed.state, seed.hold)] : []));
  const commits: GitCommitSummary[] = COMMITS.map((commit, index) => ({
    sha: commit.sha,
    short_sha: commit.sha.slice(0, 7),
    committed_at: NOW - commit.ago,
    subject: commit.subject,
    session_refs: pairs.filter((pair) => pair.seed.where === index).map((pair) => ({ resource_id: pair.session.resource_id })),
  }));
  const activity: ProjectActivity = {
    project_id: "project-horror",
    sessions,
    git: kind === "git" ? {
      selected_branch: "main",
      branches: [{ name: "main", is_current: true }],
      commits,
      github_repository_url: "https://github.com/example/HorrorGame",
      unique_session_count: sessions.length,
    } : null,
    unmapped: kind === "git"
      ? pairs.filter((pair) => typeof pair.seed.where === "string").map((pair) => ({ resource_id: pair.session.resource_id, reason: "no_commit" }))
      : [],
    providers: [
      { provider: "codex", display_name: "Codex", access: "bound_ready" },
      { provider: "claude", display_name: "Claude", access: "bound_ready" },
    ],
    warnings: [],
    window_start: NOW - 30 * DAY,
    window_end: NOW,
    older: { newest_at: NOW - 31 * DAY, commits: 6, sessions: 24 },
    storage_sync: [],
  };
  const comparison: ThreadSyncComparison | null = scene === "comparing" ? null : {
    project_id: "project-horror",
    storage_id: "storage-conf",
    storage_name: STORAGE,
    compared_at: NOW,
    entries,
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [],
    provider_errors: {},
    warnings: [],
    remote_continuity: { state: "in_sync", revision: 4 },
  };
  const entryByResource = new Map(entries.map((entry) => [entry.conversation_resource_id, entry]));
  const statusByResource = new Map<string, StatusKey>(sessions.map((session) => {
    const entry = entryByResource.get(session.resource_id);
    return [session.resource_id, entry ? sessionSyncPresentation(entry, STORAGE).status : comparison ? "unknown" : "checking"];
  }));
  return { activity, comparison, statusByResource };
}

/** The page's own narrowing, done ahead of it: rows outside `keep` go, and a
 *  commit left without a session goes with them, as a name search does. */
function narrowActivity(activity: ProjectActivity, keep: ReadonlySet<string>): ProjectActivity {
  return {
    ...activity,
    sessions: activity.sessions.filter((session) => keep.has(session.resource_id)),
    git: activity.git ? {
      ...activity.git,
      commits: activity.git.commits
        .map((commit) => ({ ...commit, session_refs: commit.session_refs.filter((reference) => keep.has(reference.resource_id)) }))
        .filter((commit) => commit.session_refs.length > 0),
    } : activity.git,
    unmapped: activity.unmapped.filter((reference) => keep.has(reference.resource_id)),
  };
}

/* ——— the proposed controls ———————————————————————————————————————— */

interface FilterProps {
  keys: readonly StatusKey[];
  counts: ReadonlyMap<StatusKey, number>;
  picked: ReadonlySet<StatusKey>;
  onToggle: (key: StatusKey) => void;
  onSet: (keys: readonly StatusKey[]) => void;
}

/** A, B · chips. Pressed narrows the list to that status; × releases all. */
function StatusChips({ keys, counts, picked, onToggle, onSet, named, tip }: FilterProps & { named: boolean; tip?: StatusKey }) {
  return (
    <div className={`sync-status-filter${named ? " named" : ""}${picked.size > 0 ? " narrowed" : ""}`} role="group" aria-label="Show only these statuses">
      {keys.map((key) => {
        const count = counts.get(key) ?? 0;
        return (
          <button key={key} type="button" className={`sync-status-chip${tip === key ? " mk-tip" : ""}`}
            aria-pressed={picked.has(key)} data-tooltip={named ? undefined : STATUS_NAME[key]}
            aria-label={`${STATUS_NAME[key]}: ${count} ${sessionNoun(count)}`} onClick={() => onToggle(key)}>
            <ThreadStatusIcon status={key} size={16} />
            {named && <span>{STATUS_NAME[key]}</span>}
            <b>{count}</b>
          </button>
        );
      })}
      {picked.size > 0 && (
        <button type="button" className="sync-name-search-clear" aria-label="Clear filter" title="Clear filter" onClick={() => onSet([])}>
          <Icon name="x" size={13} />
        </button>
      )}
    </div>
  );
}

/** D · one button and a check menu. Ticked narrows the list to that status. */
function StatusMenu({ keys, counts, picked, onToggle, onSet, startOpen = false }: FilterProps & { startOpen?: boolean }) {
  const [open, setOpen] = useState(startOpen);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open || startOpen) return undefined;
    const close = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open, startOpen]);
  const inForce = keys.filter((key) => picked.has(key));
  return (
    <div className="sync-status-menu" ref={root}>
      <button type="button" className="btn" aria-haspopup="menu" aria-expanded={open}
        aria-label={inForce.length ? `Status: ${inForce.map((key) => STATUS_NAME[key]).join(", ")}` : "Status: all"}
        onClick={() => setOpen(!open)}>
        {inForce.length > 0 && (
          <span className="sync-status-menu-glyphs">
            {inForce.map((key) => <ThreadStatusIcon key={key} status={key} size={15} />)}
          </span>
        )}
        {inForce.length === 0 ? "Status" : inForce.length === 1 ? STATUS_NAME[inForce[0]] : `${inForce.length} statuses`}
        <Icon name="chevron-down" size={12} />
      </button>
      {open && (
        <div className="sync-path-menu end" role="menu" aria-label="Show only these statuses">
          {keys.map((key) => (
            <button key={key} type="button" role="menuitemcheckbox" aria-checked={picked.has(key)}
              className="sync-path-menu-item" onClick={() => onToggle(key)}>
              <Icon name="check" size={14} className={`sync-path-menu-check${picked.has(key) ? "" : " off"}`} />
              <ThreadStatusIcon status={key} size={16} />
              <span className="sync-path-menu-copy"><span>{STATUS_NAME[key]}</span></span>
              <small>{counts.get(key) ?? 0}</small>
            </button>
          ))}
          {picked.size > 0 && (
            <>
              <div className="sync-path-menu-separator" />
              <button type="button" role="menuitem" className="sync-path-menu-item" onClick={() => onSet([])}>
                <Icon name="x" size={14} className="sync-path-menu-check" />
                <span className="sync-path-menu-copy"><span>Clear filter</span></span>
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** E · the session page's Show bar. `picked` holds what is turned off. */
function ShowBar({ keys, counts, picked, onToggle }: FilterProps) {
  return (
    <div className="session-detail-toolbar mk-show-bar">
      <span className="session-detail-toolbar-label" id="status-show-label">Show</span>
      <div className="session-detail-toolbar-toggles" role="group" aria-labelledby="status-show-label">
        {keys.map((key) => {
          const on = !picked.has(key);
          return (
            <button key={key} type="button" className={`session-detail-filter-toggle${on ? " is-on" : ""}`}
              aria-pressed={on} onClick={() => onToggle(key)}>
              <ThreadStatusIcon status={key} size={14} />
              <span className="session-detail-filter-name">{STATUS_NAME[key]}</span>
              <span className="session-detail-filter-count">{counts.get(key) ?? 0}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

/* ——— one frame: the path bar, the search row, the shipped page ————————— */

function Bar() {
  const crumbs: PathBarCrumb[] = [
    {
      id: "storage", label: STORAGE, icon: "storage-local", tone: "root", menuLabel: "Storage",
      menuHeader: { label: STORAGE, detail: "Local folder", icon: "storage-local" },
      menu: [{ items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] }],
    },
    {
      id: "project", label: PROJECT, icon: "git-folder", menuLabel: "Project",
      menuHeader: { label: PROJECT, detail: "~/project/HorrorGame", detailIsPath: true, icon: "git-folder" },
      menu: [{ items: [{ id: "project-settings", label: "Project setup…", icon: "settings", onSelect: () => undefined }] }],
    },
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      overflow={[{ items: [{ id: "app-settings", label: "Settings…", icon: "settings", onSelect: () => undefined }] }]}
      overflowLabel="App actions"
      actions={(
        <>
          <span className="sync-path-attention warning" tabIndex={0}
            data-tooltip="1 thread changed on both sides, 1 thread waiting to sync.">
            <Icon name="alert-triangle" size={13} />
            <b>2</b>
          </span>
          <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync">
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              <span className="sync-path-count" aria-hidden="true">2</span>
            </button>
            <button type="button" className="sync-path-sync">
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              <span className="sync-path-count" aria-hidden="true">9</span>
            </button>
          </div>
        </>
      )}
    />
  );
}

interface FrameProps {
  dir: Dir;
  kind: Kind;
  scene?: Scene;
  bar?: boolean;
  /** A, B, D: the statuses narrowed to. C: the statuses of the scope in
   *  force. E: the statuses turned off. */
  picks?: readonly StatusKey[];
  query?: string;
  menuOpen?: boolean;
  tip?: StatusKey;
  clip?: number;
}

/** A query no title holds. When the status filter leaves no row, it makes
 *  the shipped page draw nothing but its paging foot. */
const NO_ROW = "__no_row__";

function Frame({ dir, kind, scene = "mixed", bar = false, picks = [], query: startQuery = "", menuOpen = false, tip, clip }: FrameProps) {
  const data = useMemo(() => buildScene(scene, kind), [scene, kind]);
  const [query, setQuery] = useState(startQuery);
  const [picked, setPicked] = useState<ReadonlySet<StatusKey>>(() => new Set(picks));
  /* C is the shipped page: its own field, its own bar, the scope held here. */
  const [scope, setScope] = useState<StatusScope | null>(() => (picks.length ? sessionStatusScope(picks[0]) : null));
  const input = useRef<HTMLInputElement>(null);
  const proposed = dir !== SHIPPED;

  const counts = new Map<StatusKey, number>();
  for (const status of data.statusByResource.values()) counts.set(status, (counts.get(status) ?? 0) + 1);
  /* A status no loaded row carries has no control, unless it is in force:
     then it stays, at zero, so it can be released. */
  const keys = STATUS_ORDER.filter((key) => (counts.get(key) ?? 0) > 0 || picked.has(key));
  /* One status narrows nothing, and a list still being compared has no
     verdicts to narrow by. */
  const showControl = proposed && data.comparison !== null && (counts.size > 1 || picked.size > 0);
  const statusPasses = (status: StatusKey) => (
    dir === "e" ? !picked.has(status) : picked.size === 0 || picked.has(status)
  );
  const filtering = showControl && picked.size > 0;
  const keep = new Set(data.activity.sessions
    .filter((session) => !filtering || statusPasses(data.statusByResource.get(session.resource_id) ?? "unknown"))
    .map((session) => session.resource_id));
  const total = data.activity.sessions.length;
  const matches = data.activity.sessions.filter((session) => (
    keep.has(session.resource_id) && nameMatchesSearch(session.title, query)
  )).length;
  const narrowing = filtering || query.trim().length > 0;
  /* The status filter emptied the list, and releasing it would bring rows
     back: the box offers that. A name that matches nothing is the shipped
     box's own case. */
  const emptied = filtering && matches === 0
    && data.activity.sessions.some((session) => nameMatchesSearch(session.title, query));

  const toggle = (key: StatusKey) => setPicked((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  });
  const filter: FilterProps = { keys, counts, picked, onToggle: toggle, onSet: (next) => setPicked(new Set(next)) };

  const results = (
    <div className="sync-name-search-results" aria-live="polite" aria-atomic="true">
      <span className="sync-name-search-count">
        {narrowing
          ? <><b>{matches}</b><span> of {total} {sessionNoun(total)}</span></>
          : <>{total} {sessionNoun(total)}</>}
      </span>
    </div>
  );
  const field = (
    <div className="sync-name-search">
      <div className="sync-name-search-input">
        <Icon name="search" size={14} />
        <input ref={input} type="search" value={query} placeholder="Search loaded sessions…" aria-label="Search loaded sessions"
          onChange={(event) => setQuery(event.target.value)} />
        {query && (
          <button type="button" className="sync-name-search-clear" aria-label="Clear search" title="Clear search"
            onClick={() => { setQuery(""); input.current?.focus(); }}>
            <Icon name="x" size={13} />
          </button>
        )}
      </div>
      {/* A: the chips and the count travel together, beside the field while
          they fit and as one line under it when they don't. B: the named
          chips always take their own line, and the count keeps its place. */}
      {dir === "a" && showControl
        ? <div className="sync-name-search-tail"><StatusChips {...filter} named={false} tip={tip} />{results}</div>
        : results}
      {dir === "b" && showControl && <StatusChips {...filter} named />}
    </div>
  );
  const viewSwitch = kind === "flat" ? <ProjectViewSwitch view="sessions" onChange={() => undefined} /> : null;
  const menu = dir === "d" && showControl ? <StatusMenu {...filter} startOpen={menuOpen} /> : null;

  const page = (
    <ProjectChatHistoryContent
      project={project(kind === "git")}
      binding={binding}
      activity={proposed && filtering ? narrowActivity(data.activity, keep) : data.activity}
      loading={false}
      loadingMore={false}
      loadError={null}
      onRetryLoad={() => undefined}
      actionBusyResourceId={null}
      onLoadMore={() => undefined}
      onShowLess={() => undefined}
      canShowLess={false}
      onSessionAction={() => undefined}
      onDeleteSession={() => undefined}
      onOpenSessionDetail={() => undefined}
      comparison={data.comparison}
      comparisonLoading={data.comparison === null}
      activeStorageName={STORAGE}
      viewSwitch={viewSwitch ?? undefined}
      {...(proposed ? {
        searchQuery: emptied ? NO_ROW : query,
        onSearchQueryChange: () => undefined,
      } : {
        searchQuery: query,
        onSearchQueryChange: setQuery,
        statusScope: scope,
        onStatusScopeChange: setScope,
      })}
    />
  );

  const frameClass = [
    "spec-frame",
    proposed ? "mk-swap" : "",
    proposed && emptied ? "mk-emptied" : "",
  ].filter(Boolean).join(" ");

  return (
    <div className={frameClass}>
      <div className={clip ? "sync-app spec-clip" : "sync-app"} style={clip ? { ["--clip" as string]: `${clip}px` } : undefined}>
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          {bar && <Bar />}
          <div className="sync-project-tab-panel" role="tabpanel">
            {proposed && (viewSwitch || menu
              ? <div className="sync-list-head">{viewSwitch}{field}{menu}</div>
              : field)}
            {dir === "e" && showControl && <ShowBar {...filter} />}
            {proposed && emptied && (
              <div className="sync-history-state mk-empty">
                No matching loaded sessions.
                <button type="button" className="btn btn-ghost" onClick={() => setPicked(new Set())}>Clear filter</button>
              </div>
            )}
            {page}
          </div>
        </main>
      </div>
    </div>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const COMMON: readonly StatusKey[] = ["local-outstanding", "cloud-outstanding", "divergent", "pending", "synced"];
const ATTENTION: readonly StatusKey[] = ["divergent", "pending", "conflict", "unavailable"];

/** The picks that leave only `key` on screen, in each direction's own terms. */
const only = (dir: Dir, key: StatusKey): StatusKey[] => (dir === "e" ? COMMON.filter((other) => other !== key) : [key]);
/** The picks that leave everything but `key`; a scope bar has none. */
const allBut = (dir: Dir, key: StatusKey): StatusKey[] => (dir === "e" ? [key] : COMMON.filter((other) => other !== key));

interface Cell {
  title: string;
  note?: (dir: Dir) => string | null;
  props: (dir: Dir) => Omit<FrameProps, "dir" | "kind"> & { kind?: Kind };
  only?: readonly Dir[];
  not?: readonly Dir[];
  narrow?: boolean;
}

const presses = (count: number) => `${count} press${count === 1 ? "" : "es"}`;

const CELLS: Cell[] = [
  { title: "Opens with nothing narrowed", props: () => ({}) },
  {
    title: "Only what still needs a Push",
    note: (dir) => (dir === "d" ? "2 presses: open, tick" : PUSH_COST[dir]),
    props: (dir) => ({ picks: only(dir, "local-outstanding") }),
  },
  {
    title: "Everything except up to date",
    not: ["c"],
    note: (dir) => (dir === "d" ? "5 presses: open, four ticks" : presses(dir === "e" ? 1 : 4)),
    props: (dir) => ({ picks: allBut(dir, "synced") }),
  },
  { title: "What needs attention", only: ["c"], note: () => "the path bar’s chip, as a scope", props: () => ({ picks: ATTENTION }) },
  {
    title: "With a name search",
    note: () => "status and name both apply",
    props: (dir) => ({ picks: only(dir, "local-outstanding"), query: "pelican" }),
  },
  {
    title: "After a Push, the narrowed status has no rows left",
    note: (dir) => (dir === "e" ? "no toggle is left for it" : "it stays at 0, and the box offers the way back"),
    props: (dir) => ({ scene: "pushed", picks: only(dir, "local-outstanding"), clip: 330 }),
  },
  { title: "Tooltip", note: () => "the only place this direction names a status", only: ["a"], props: () => ({ tip: "divergent", clip: 150 }) },
  { title: "Menu open", only: ["d"], props: () => ({ picks: ["local-outstanding"], menuOpen: true, clip: 380 }) },
  { title: "Every status at once", note: () => "eight, the most there can be", props: () => ({ scene: "every" }) },
  { title: "Every row has the same status", note: () => "no control: it would narrow nothing", props: () => ({ scene: "one", clip: 190 }) },
  { title: "Still comparing", note: () => "no control until rows have a verdict", props: () => ({ scene: "comparing", clip: 190 }) },
  {
    title: "Non-Git project",
    note: () => "the view switch leads the same row",
    props: (dir) => ({ kind: "flat", picks: only(dir, "local-outstanding") }),
  },
  { title: "Narrowest pane (580px)", note: () => "every status", narrow: true, props: () => ({ scene: "every" }) },
];

function Page() {
  const initial = readKnobs();
  const [dir, setDir] = useState<Shown>(initial.dir);
  const [kind, setKind] = useState<Kind>(initial.kind);
  const [theme, setTheme] = useState<Theme>(initial.theme);
  document.documentElement.dataset.theme = theme;
  const seg = <T extends string>(value: T, current: T, set: (value: T) => void, label: string) => (
    <button key={value} type="button" className="spec-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  return (
    <div className="spec">
      <h1 className="spec-title">Filter sessions by status</h1>
      <p className="spec-sub">
        The activity list could be narrowed by name, not by the status glyph each row leads with. Five ways to add
        that, each taken from a published pattern, on the shipped page: <b>{PROJECT}</b> linked to <b>{STORAGE}</b>,
        twelve sessions loaded, twenty-four more behind “Load earlier”. “Compare all” sets the five side by side; a
        direction shows its whole page and every state. Every control works: press it, and type in the field.
      </p>
      <div className="spec-controls" role="group" aria-label="Specimen controls">
        <div className="spec-group"><span>Direction</span>
          {seg<Shown>("all", dir, setDir, "Compare all")}
          {DIRS.map((value) => seg<Shown>(value, dir, setDir, DIR_LABEL[value]))}
        </div>
        <div className="spec-group"><span>Project</span>
          {seg("git", kind, setKind, "Git")}{seg("flat", kind, setKind, "Non-Git")}
        </div>
        <div className="spec-group"><span>Theme</span>
          {seg("light", theme, setTheme, "Light")}{seg("dark", theme, setTheme, "Dark")}
        </div>
      </div>

      {dir === "all" ? (
        <>
          {DIRS.map((value) => (
            <section key={value} className="spec-compare">
              <h2 className="spec-h">
                {DIR_LABEL[value]}{value === SHIPPED && <em>picked, shipped</em>}
                <small>{DIR_SUMMARY[value]}</small>
              </h2>
              <div className="spec-cells">
                <div className="spec-cell">
                  <span>As the page opens</span>
                  <Frame key={`rest:${value}:${kind}`} dir={value} kind={kind} clip={236} />
                </div>
                <div className="spec-cell">
                  <span>Only what still needs a Push<small>{PUSH_COST[value]}</small></span>
                  <Frame key={`push:${value}:${kind}`} dir={value} kind={kind} clip={236} picks={only(value, "local-outstanding")} />
                </div>
              </div>
            </section>
          ))}
        </>
      ) : (
        <>
          <div className="spec-panel">
            <strong>{DIR_LABEL[dir]}{dir === SHIPPED ? " (picked 2026-10-03; these frames are the shipped page)" : ""}</strong>
            <ul>{DIR_NOTES[dir].map((note, index) => <li key={index}>{note}</li>)}</ul>
            {DIR_FROM[dir] && <p className="spec-from">Seen in: {DIR_FROM[dir]}</p>}
          </div>

          <h2 className="spec-h">Live<small>the whole page, with its path bar</small></h2>
          <div className="spec-live">
            <Frame key={`live:${dir}:${kind}`} dir={dir} kind={kind} bar />
          </div>

          <h2 className="spec-h">States<small>the head of the page in each; every control still works</small></h2>
          <div className="spec-cells">
            {CELLS.filter((cell) => (!cell.only || cell.only.includes(dir)) && !cell.not?.includes(dir)).map((cell) => {
              const { kind: cellKind, ...props } = cell.props(dir);
              const note = cell.note?.(dir);
              return (
                <div key={cell.title} className={`spec-cell${cell.narrow ? " narrow" : ""}`}>
                  <span>{cell.title}{note && <small>{note}</small>}</span>
                  <Frame key={`${cell.title}:${dir}:${kind}`} dir={dir} kind={cellKind ?? kind} clip={300} {...props} />
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* `?refs=off` drops the table from a captured sheet. */}
      <div className="spec-panel" hidden={new URLSearchParams(window.location.search).get("refs") === "off"}>
        <strong>What the references ask of a list filter</strong>
        <table className="spec-std">
          <thead><tr><th>Reference</th><th>What it asks</th><th>Here</th></tr></thead>
          <tbody>
            {STANDARDS.map((row, index) => (
              <tr key={index}><th>{row.source}</th><td>{row.says}</td><td>{row.here}</td></tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
