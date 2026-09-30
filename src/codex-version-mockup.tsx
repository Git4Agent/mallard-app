import { StrictMode, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import { NoticeZone, type NoticeItem } from "./components/notice/Notice";
import { ProjectChatHistoryContent } from "./components/project-sync/ProjectActivityPage";
import SessionDetailHeader from "./components/project-sync/session-detail/SessionDetailHeader";
import SyncPathBar, { PathBarBelow, type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import type {
  LocalProjectSummary,
  ProjectActivity,
  ProjectBinding,
  ProjectSessionDetail,
  ProjectSessionSummary,
  SessionActionDescriptor,
  SessionCount,
} from "./types";
import "./App.css";

/**
 * Mockup: what the user sees when a session's Open in Terminal or Open in
 * Codex refuses a Codex CLI older than 0.145.0, or cannot read its version.
 *
 * Each button checks the CLI it actually runs (owner, 2026-09-28):
 *   Open in Terminal  the CLI on the login-shell PATH
 *   Open in Codex     the CLI bundled in /Applications/ChatGPT.app
 *                     (Contents/Resources/codex-cli/bin/codex)
 *
 * Every frame is the shipped CSS and real components: SyncPathBar, the
 * Activity list (ProjectChatHistoryContent), SessionDetailHeader and
 * NoticeZone. Only the two actions are fake — each waits ~0.6 s for the
 * version probe, then fails with its CLI's chosen outcome.
 *
 *   A  as specified: the error lands in each page's own notice zone
 *   B  Activity hands row-action errors to the shell zone under the path bar
 *   C  copy only: a shorter wording, orthogonal to A/B
 *
 * `npm run dev`, then /codex-version-mockup.html
 * (`?button=terminal|app&cli=old|unknown|missing|ok&app=old|unknown|missing|ok&version=0.144.0|0.143.0|0.145.0-alpha.3&copy=spec|short&row=top|lower&theme=light|dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Button = "terminal" | "app";
type Outcome = "old" | "unknown" | "missing" | "ok";
type Version = "0.144.0" | "0.143.0" | "0.145.0-alpha.3";
type Copy = "spec" | "short";
type Row = "top" | "lower";
interface Knobs { button: Button; cli: Outcome; app: Outcome; version: Version; copy: Copy; row: Row; theme: "light" | "dark" }

const OUTCOMES: readonly Outcome[] = ["old", "unknown", "missing", "ok"];
const VERSIONS: readonly Version[] = ["0.144.0", "0.143.0", "0.145.0-alpha.3"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    button: pick("button", ["terminal", "app"] as const, "terminal"),
    cli: pick("cli", OUTCOMES, "old"),
    app: pick("app", OUTCOMES, "old"),
    version: pick("version", VERSIONS, "0.144.0"),
    copy: pick("copy", ["spec", "short"] as const, "spec"),
    row: pick("row", ["top", "lower"] as const, "lower"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const OUTCOME_LABEL: Record<Button, Record<Outcome, string>> = {
  terminal: { old: "Too old", unknown: "Unreadable", missing: "Not on PATH", ok: "0.145.0+" },
  app: { old: "Too old", unknown: "Unreadable", missing: "App not installed", ok: "0.145.0+" },
};

/* ——— the messages ——————————————————————————————————————————————— */

/** One plain string from Rust, exactly as the action's `Err` would carry it.
 *  Terminal's copy is the handoff's; the app's is proposed in its image. */
function message(button: Button, outcome: Outcome, copy: Copy, version: Version): string | null {
  if (outcome === "ok") return null;
  if (button === "terminal") {
    switch (outcome) {
      case "old":
        return copy === "spec"
          ? `Cannot open this session with Codex CLI ${version}. Mallard requires Codex CLI 0.145.0 or later for session resume. Update Codex CLI, then try again.`
          : `Codex CLI ${version} can't resume this session. Update Codex CLI to 0.145.0 or later, then try again.`;
      case "unknown":
        return copy === "spec"
          ? "Cannot determine the Codex CLI version. Check your Codex CLI installation, then try again."
          : "Mallard couldn't read the Codex CLI version. Check your Codex CLI installation, then try again.";
      case "missing":
        // Shipped today (actions.rs, macOS), unchanged by this work.
        return "Codex CLI is unavailable in the login shell; install it or use Open in Codex";
    }
  }
  switch (outcome) {
    case "old":
      return copy === "spec"
        ? `Cannot open this session in the Codex desktop app: its bundled Codex CLI is ${version}. Mallard requires Codex CLI 0.145.0 or later for session resume. Update the Codex desktop app, then try again.`
        : `The Codex desktop app's bundled Codex CLI ${version} can't resume this session. Update the Codex desktop app, then try again.`;
    case "unknown":
      return copy === "spec"
        ? "Cannot determine the Codex CLI version bundled with the Codex desktop app. Check your Codex desktop app installation, then try again."
        : "Mallard couldn't read the Codex desktop app's bundled CLI version. Check your Codex desktop app installation, then try again.";
    case "missing":
      // New: the bundle is checked before the probe, so this replaces
      // `open`'s own stderr ("Codex desktop app could not open the project: …").
      return "The Codex desktop app is not installed; install it or use Open in Terminal";
  }
}

/* ——— data ——————————————————————————————————————————————————————— */

const ROOT = "/Users/test/projects/mallard";
const PROJECT_NAME = "mallard";
const STORAGE = "Attic R2";

const project: LocalProjectSummary = {
  local_project_id: "project-mallard",
  bundle_id: "bundle-mallard",
  display_name: PROJECT_NAME,
  local_alias: null,
  repository_fingerprint: "fingerprint",
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex" },
  profile_names: ["Default Codex"],
  revision: 1,
};

const binding: ProjectBinding = {
  host_id: "00112233445566778899aabbccddeeff",
  replica_id: "replica",
  local_project_id: project.local_project_id,
  bundle_id: project.bundle_id,
  project_root: ROOT,
  canonical_project_root: ROOT,
  profile_ids: { codex: "profile-codex" },
  state: "active",
  revision: 1,
  updated_at: 2,
};

const OPEN_IN_APP: SessionActionDescriptor = {
  action_id: "codex.open-in-app",
  kind: "open_in_app",
  label: "Open in Codex",
  enabled: true,
  disabled_reason: null,
};
const OPEN_IN_TERMINAL: SessionActionDescriptor = {
  action_id: "codex.open-in-terminal",
  kind: "open_in_terminal",
  label: "Open in Terminal",
  enabled: true,
  disabled_reason: null,
};

const TITLES = [
  "Add Codex CLI version check",
  "Fix missing Codex turns in previews",
  "Map sessions onto the first-parent rail",
  "Tighten Push review loading caption",
  "Storage marks: tile option",
  "Back-to exit in empty reviews",
  "Session watcher removal",
  "Cursor file-state rule",
  "Claude sidecar pull",
  "Feishu drive provider",
];
const NOW = 1_790_000_000;

const sessions: ProjectSessionSummary[] = TITLES.map((title, index) => {
  const id = `019f7798-5437-7632-9dbc-${String(index).padStart(12, "0")}`;
  const started = NOW - index * 20_000;
  return {
    resource_id: `codex:conversation:${id}`,
    provider: "codex",
    provider_display_name: "Codex",
    provider_session_id: id,
    logical_thread_id: id,
    source: "local_only",
    access: "bound_ready",
    title,
    preserved: null,
    summary: null,
    started_at: started,
    ended_at: started + 2_400 + index * 300,
    transcript_path: `/Users/test/.codex/sessions/rollout-${id}.jsonl`,
    working_directory: ROOT,
    recorded_sha: null,
    is_active: false,
    user_round_count: 3 + index,
    agent_message_count: 6 + index * 2,
    tool_call_count: 11 + index * 3,
    total_input_tokens: 40_000 + index * 9_000,
    total_output_tokens: 6_000 + index * 800,
    total_tokens: 46_000 + index * 9_800,
    metrics_complete: true,
    actions: [OPEN_IN_APP, OPEN_IN_TERMINAL],
  };
});

/** Rows the scenarios aim at: the first, and one well below the fold. */
const TARGET: Record<Row, number> = { top: 0, lower: 7 };

const COMMITS = [[0, 1], [2, 3, 4], [5, 6], [7, 8, 9]];
const activity: ProjectActivity = {
  project_id: project.local_project_id,
  sessions,
  git: {
    selected_branch: "main",
    branches: [{ name: "main", is_current: true }],
    github_repository_url: null,
    commits: COMMITS.map((indexes, order) => ({
      sha: String(order + 1).repeat(40),
      short_sha: String(order + 1).repeat(7),
      committed_at: sessions[indexes[0]].ended_at + 60,
      subject: ["Add Codex CLI version check", "Fix missing Codex turns in Managed Cloud previews", "Prepare version 0.1.5", "Add Cloud project setup"][order],
      session_refs: indexes.map((index) => ({ resource_id: sessions[index].resource_id })),
    })),
    unique_session_count: sessions.length,
  },
  unmapped: [],
  providers: [{ provider: "codex", display_name: "Codex", access: "bound_ready" }],
  warnings: [],
  window_start: NOW - 400_000,
  window_end: NOW + 10_000,
  older: null,
  storage_sync: [],
};

const exact = (value: number): SessionCount => ({ value, quality: "exact" });
const detail: ProjectSessionDetail = {
  resource_id: sessions[TARGET.lower].resource_id,
  provider: "codex",
  provider_display_name: "Codex",
  provider_session_id: sessions[TARGET.lower].provider_session_id ?? "",
  title: sessions[TARGET.lower].title,
  summary: null,
  started_at: sessions[TARGET.lower].started_at,
  ended_at: sessions[TARGET.lower].ended_at,
  is_active: false,
  branch: "main",
  recorded_sha: "8".repeat(40),
  tokens: exact(114_600),
  event_counts: {
    prompts: exact(10),
    responses: exact(20),
    intermediate_steps: exact(4),
    checkpoints: exact(0),
    tool_calls: exact(32),
    file_edits: exact(6),
    shell_calls: exact(18),
    reads: exact(7),
    other_tools: exact(1),
    agent_calls: exact(0),
  },
  actions: [OPEN_IN_APP, OPEN_IN_TERMINAL],
  snapshot_id: "snapshot-1",
  warnings: [],
};

/* ——— the fake action ——————————————————————————————————————————— */

const PROBE_MS = 600;

interface Busy { resourceId: string; actionId: string }

/** Runs the Terminal action the way the pages do: busy while the backend
 *  probes, then either a returned error or nothing (Terminal opened). */
function useFakeAction(knobs: Knobs) {
  const [busy, setBusy] = useState<Busy | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [launched, setLaunched] = useState<string | null>(null);
  const timer = useRef<number | null>(null);
  const run = useCallback((resourceId: string, action: SessionActionDescriptor) => {
    if (!action.enabled) return;
    if (timer.current !== null) window.clearTimeout(timer.current);
    setError(null);
    setLaunched(null);
    setBusy({ resourceId, actionId: action.action_id });
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setBusy(null);
      const button: Button = action.kind === "open_in_terminal" ? "terminal" : "app";
      const failure = message(button, knobs[button === "terminal" ? "cli" : "app"], knobs.copy, knobs.version);
      setError(failure);
      setAttempt((value) => value + 1);
      setLaunched(failure === null ? action.label : null);
    }, PROBE_MS);
  }, [knobs.cli, knobs.app, knobs.copy, knobs.version]);
  useEffect(() => () => { if (timer.current !== null) window.clearTimeout(timer.current); }, []);
  return { busy, error, attempt, launched, run, clear: () => setError(null) };
}

/* ——— measurement ——————————————————————————————————————————————— */

type VerdictState = { tone: "seen" | "missed" | "quiet"; text: string } | null;

/** Where the error box sits against the part of the page the user can see. */
function measure(frame: HTMLElement, launched: string | null, attempt: number): VerdictState {
  if (attempt === 0) return null;
  if (launched) return { tone: "quiet", text: `${launched}: the version passes, so the launch goes ahead and the page does not change.` };
  const box = frame.querySelector<HTMLElement>(".notice");
  const main = frame.querySelector<HTMLElement>(".sync-main");
  if (!box || !main) return { tone: "quiet", text: "No error box rendered." };
  const boxRect = box.getBoundingClientRect();
  const view = main.contains(box) ? main.getBoundingClientRect() : frame.getBoundingClientRect();
  if (boxRect.bottom <= view.top) {
    return { tone: "missed", text: `Error box is ${Math.round(view.top - boxRect.bottom)} px above the visible area. Nothing in view says the action failed.` };
  }
  if (boxRect.top >= view.bottom) {
    return { tone: "missed", text: `Error box is ${Math.round(boxRect.top - view.bottom)} px below the visible area.` };
  }
  const where = main.contains(box) ? "at the top of the scrolled page" : "under the path bar, outside the scroll";
  return { tone: "seen", text: `Error box is on screen, ${where}.` };
}

/** Scroll a frame's main so the target row sits a third of the way down. */
function scrollToRow(frame: HTMLElement, index: number) {
  const main = frame.querySelector<HTMLElement>(".sync-main");
  const rows = frame.querySelectorAll<HTMLElement>("article.sync-history-thread-card");
  frame.querySelectorAll(".mk-clicked").forEach((node) => node.classList.remove("mk-clicked"));
  const row = rows[index];
  if (!main || !row) return;
  row.classList.add("mk-clicked");
  main.scrollTop = index === 0
    ? 0
    : main.scrollTop + row.getBoundingClientRect().top - main.getBoundingClientRect().top - main.clientHeight / 3;
}

/* ——— frames ——————————————————————————————————————————————————————— */

function PathActions() {
  return (
    <>
      <button type="button" className="sync-path-icon-action" aria-label="Refresh" title="Refresh">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync">
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
        </button>
        <button type="button" className="sync-path-sync">
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
        </button>
      </div>
    </>
  );
}

const ACTIVITY_CRUMBS: PathBarCrumb[] = [
  { id: "storage", label: STORAGE, icon: "storage-r2", tone: "root" },
  { id: "project", label: PROJECT_NAME, icon: "git-folder" },
  { id: "section", label: "Git history", tone: "section" },
];

const DETAIL_CRUMBS: PathBarCrumb[] = [
  ...ACTIVITY_CRUMBS,
  { id: "session", label: detail.title },
];

function Verdict({ frameRef, launched, attempt, watch }: {
  frameRef: RefObject<HTMLDivElement | null>;
  launched: string | null;
  attempt: number;
  watch: unknown;
}) {
  const [verdict, setVerdict] = useState<VerdictState>(null);
  useLayoutEffect(() => {
    const frame = frameRef.current;
    if (!frame) return;
    const update = () => setVerdict(measure(frame, launched, attempt));
    update();
    const main = frame.querySelector(".sync-main");
    main?.addEventListener("scroll", update);
    return () => main?.removeEventListener("scroll", update);
  }, [frameRef, launched, attempt, watch]);
  if (!verdict) return <div className="spec-verdict quiet">Click a Terminal button in any row to run the action.</div>;
  return <div className={`spec-verdict ${verdict.tone}`}>{verdict.text}</div>;
}

function ActivityFrame({ direction, knobs }: { direction: "a" | "b"; knobs: Knobs }) {
  const frameRef = useRef<HTMLDivElement>(null);
  const action = useFakeAction(knobs);
  const { run } = action;

  /* Every knob change replays the click on the target row. */
  useEffect(() => {
    const frame = frameRef.current;
    if (!frame) return;
    scrollToRow(frame, TARGET[knobs.row]);
    run(sessions[TARGET[knobs.row]].resource_id, knobs.button === "terminal" ? OPEN_IN_TERMINAL : OPEN_IN_APP);
  }, [knobs.row, knobs.button, run]);

  const shellItems: NoticeItem[] = direction === "b" && action.error
    ? [{
        key: "session-action-failed",
        severity: "danger",
        message: action.error,
        dismissalId: String(action.attempt),
        onDismiss: action.clear,
      }]
    : [];
  const shellZone = (
    <NoticeZone
      items={shellItems}
      subject="messages from Mallard"
      className="sync-backend-banner"
      timing="blocking"
    />
  );

  return (
    <>
      <div className="spec-frame" ref={frameRef} style={{ width: 782 }}>
        <PathBarBelow.Provider value={shellZone}>
          <SyncPathBar crumbs={ACTIVITY_CRUMBS} actions={<PathActions />} ariaLabel={`${PROJECT_NAME} path`} />
        </PathBarBelow.Provider>
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
          <section className="profile-links-section sync-project-overview" aria-label={PROJECT_NAME}>
            <div id="project-history-panel" className="sync-project-tab-panel">
              <ProjectChatHistoryContent
                project={project}
                binding={binding}
                activity={activity}
                loading={false}
                loadingMore={false}
                actionError={direction === "a" ? action.error : null}
                loadError={null}
                onRetryLoad={() => undefined}
                actionBusyResourceId={action.busy?.resourceId ?? null}
                actionBusyActionId={action.busy?.actionId ?? null}
                onLoadMore={() => undefined}
                onShowLess={() => undefined}
                canShowLess={false}
                onSessionAction={(session, descriptor) => {
                  const frame = frameRef.current;
                  if (frame) {
                    frame.querySelectorAll(".mk-clicked").forEach((node) => node.classList.remove("mk-clicked"));
                    const index = sessions.findIndex((candidate) => candidate.resource_id === session.resource_id);
                    frame.querySelectorAll("article.sync-history-thread-card")[index]?.classList.add("mk-clicked");
                  }
                  run(session.resource_id, descriptor);
                }}
              />
            </div>
          </section>
        </main>
      </div>
      <Verdict frameRef={frameRef} launched={action.launched} attempt={action.attempt} watch={action.error} />
    </>
  );
}

function DetailFrame({ knobs }: { knobs: Knobs }) {
  const frameRef = useRef<HTMLDivElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const action = useFakeAction(knobs);
  const { run } = action;
  useEffect(() => {
    run(detail.resource_id, knobs.button === "terminal" ? OPEN_IN_TERMINAL : OPEN_IN_APP);
  }, [knobs.button, run]);

  const notices: NoticeItem[] = action.error
    ? [{ key: "action-failed", severity: "danger", message: action.error }]
    : [];

  return (
    <>
      <div className="spec-frame" ref={frameRef} style={{ width: 782 }}>
        <SyncPathBar crumbs={DETAIL_CRUMBS} actions={<PathActions />} ariaLabel="Session path" />
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page" style={{ height: 300 }}>
          <section className="session-detail-page" aria-labelledby="session-detail-heading">
            <SessionDetailHeader
              detail={detail}
              headingRef={headingRef}
              actionBusyId={action.busy?.actionId ?? null}
              onAction={(descriptor) => run(detail.resource_id, descriptor)}
            />
            <NoticeZone
              items={notices}
              subject="notes about this session"
              className="session-detail-notices"
              {...(notices.length > 0 ? { timing: "blocking" as const } : { timing: "standing" as const, label: "Notes about this session" })}
            />
            <div className="mk-transcript" aria-hidden="true"><div /><div /><div /><div /></div>
          </section>
        </main>
      </div>
      <Verdict frameRef={frameRef} launched={action.launched} attempt={action.attempt} watch={action.error} />
    </>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

const COLUMN: Record<"a" | "b", { title: string; note: ReactNode; badge?: string }> = {
  a: {
    title: "A · As specified: the page’s own notice zone",
    badge: "no frontend change",
    note: <>The error rides the existing <code>action-failed</code> item at the top of the Activity list. This is what the handoff describes, and it needs no frontend change. The catch: the zone scrolls with the list, and nothing scrolls it into view. A click on a lower row fails off-screen, for either button. The same is true today of every row-action error, including <i>Codex CLI is unavailable</i>.</>,
  },
  b: {
    title: "B · Activity: the error goes under the path bar",
    badge: "broader than this task",
    note: <>Activity hands row-action errors to the shell zone under the path bar. That zone sits outside the scrolling <code>main</code>, so it is always on screen. This is where Push failures already moved (2026-09-24), with the same <b>×</b>. Costs: every <code>actionError</code> on Activity moves, including delete failures. A callback runs from <code>ProjectActivityPage</code> through <code>ProjectLinksWorkspace</code> to <code>ProjectSync</code>. The error needs its own clear rule (the next action, or leaving this project’s Activity), or it follows the user to Settings.</>,
  },
};

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams({ ...next })}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string, disabled = false) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} disabled={disabled} onClick={() => set(key, value)}>{label}</button>
  );
  /* Remount every frame on a knob change so each replays its click cleanly. */
  const replay = JSON.stringify(knobs);
  const shown: Outcome = knobs.button === "terminal" ? knobs.cli : knobs.app;
  const copyRows: Array<{ button: Button; outcome: Outcome; label: string; note?: string }> = [
    { button: "terminal", outcome: "old", label: `Too old (${knobs.version})` },
    { button: "terminal", outcome: "unknown", label: "Unreadable" },
    { button: "terminal", outcome: "missing", label: "Not on PATH", note: "unchanged (today)" },
    { button: "app", outcome: "old", label: `Too old (${knobs.version})` },
    { button: "app", outcome: "unknown", label: "Unreadable" },
    { button: "app", outcome: "missing", label: "App not installed", note: "new, replaces open’s stderr" },
  ];

  return (
    <div className="spec">
      <h1 className="spec-title">Open in Terminal and Open in Codex: refusing a Codex CLI older than 0.145.0</h1>
      <p className="spec-sub">
        Each button checks the CLI it actually runs. <b>Open in Terminal</b> checks the CLI on the login-shell PATH. <b>Open in
        Codex</b> checks the CLI bundled inside <code>/Applications/ChatGPT.app</code> (<code>Contents/Resources/codex-cli/bin/codex</code>;
        0.158.0-alpha.2.1 on this Mac, beside 0.156.0 on PATH). An old CLI on one side leaves the other button alone. Every frame
        uses the shipped CSS and real components; only the two actions are fake. Each spins for ~0.6 s (the version probe) and
        then fails with its CLI’s outcome below. A knob change replays a click on the dashed row, and any button in a frame can be
        clicked again. The line under each frame measures whether the error box is on screen.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Clicked</span>{seg("button", "terminal", "Open in Terminal")}{seg("button", "app", "Open in Codex")}</div>
        <div className="spec-group"><span>PATH CLI</span>{OUTCOMES.map((outcome) => seg("cli", outcome, OUTCOME_LABEL.terminal[outcome]))}</div>
        <div className="spec-group"><span>App’s CLI</span>{OUTCOMES.map((outcome) => seg("app", outcome, OUTCOME_LABEL.app[outcome]))}</div>
        <div className="spec-group"><span>Detected</span>{VERSIONS.map((version) => seg("version", version, version, knobs.cli !== "old" && knobs.app !== "old"))}</div>
        <div className="spec-group"><span>Copy</span>{seg("copy", "spec", "As specified")}{seg("copy", "short", "C · Shorter")}</div>
        <div className="spec-group"><span>Clicked row</span>{seg("row", "top", "Top row")}{seg("row", "lower", "Row 8 (scrolled)")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-grid">
        {(["a", "b"] as const).map((direction) => (
          <div key={direction} style={{ width: 782 }}>
            <h2 className="spec-col-title">{COLUMN[direction].title}{COLUMN[direction].badge && <em>{COLUMN[direction].badge}</em>}</h2>
            <p className="spec-col-note">{COLUMN[direction].note}</p>
            <ActivityFrame key={replay} direction={direction} knobs={knobs} />
          </div>
        ))}
      </div>

      <div className="spec-grid">
        <div style={{ width: 782 }}>
          <h2 className="spec-col-title">Session Detail: same in A and B</h2>
          <p className="spec-col-note">Both buttons sit in the header strip right above the page’s notice zone, and that strip is not sticky. So whoever clicks one is looking at the spot where the error lands. No change is needed in either direction.</p>
          <DetailFrame key={replay} knobs={knobs} />
        </div>
        <div style={{ width: 782 }}>
          <h2 className="spec-col-title">Copy: as specified vs C · shorter</h2>
          <p className="spec-col-note">One plain string from Rust. The notice adds the <b>Error ·</b> lead and the <b>×</b>. Terminal’s wording is the handoff’s. The app’s is <b>proposed</b>: its remedy is updating the Codex desktop app, not the CLI. Tinted row = the error shown.</p>
          <div className="spec-panel">
            <table className="spec-copy">
              <thead><tr><th /><th>As specified</th><th>C · Shorter</th></tr></thead>
              <tbody>
                {copyRows.map((row) => (
                  <tr key={`${row.button}:${row.outcome}`} className={knobs.button === row.button && shown === row.outcome ? "on" : undefined}>
                    <th>{row.button === "terminal" ? "Terminal" : "Codex app"}<br />{row.label}</th>
                    <td>{message(row.button, row.outcome, "spec", knobs.version)}{row.note && <><br /><i>{row.note}</i></>}</td>
                    <td>{row.outcome === "missing" ? <i>same</i> : message(row.button, row.outcome, "short", knobs.version)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="spec-panel" style={{ marginTop: 12 }}>
            <strong>Held fixed in every choice</strong>
            <ul>
              <li>The check runs only on the click. No probe per row, no cached version, and no disabled or annotated menu item.</li>
              <li>Only session resume is checked. The Library’s fresh Codex terminal and fresh Codex app launches are unchanged.</li>
              <li>A rejected click never reaches <code>before_launch</code>, the Terminal launcher or <code>open</code>, and never touches session files. The probe runs with a throwaway <code>CODEX_HOME</code>, because <code>codex --version</code> writes <code>tmp/arg0/…</code> into whichever home it is given.</li>
              <li><code>0.145.0-alpha.3</code> is refused, because a prerelease sorts below its release. <code>0.145.0</code>, <code>0.147.0</code>, <code>0.156.0</code> and the bundled <code>0.158.0-alpha.2.1</code> pass.</li>
              <li>Neither “too old” message points at the other button. Each button checks its own CLI, so the other one may fail as well.</li>
              <li>Windows: Terminal gets the check. The app check waits on an owner decision, because the bundled CLI’s location inside the Windows app is unverified.</li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
