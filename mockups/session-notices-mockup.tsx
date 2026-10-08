import { StrictMode, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../src/components/notice/Notice";
import PageBack, { type PageBackTarget } from "../src/components/sync-review/PageBack";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../src/components/sync-review/SyncPathBar";
import type { NoticeSeverity } from "../src/types";
import "../src/App.css";

/**
 * Mockup: the messages on the session page.
 *
 * Today the page can draw two boxes around its fact strip:
 *   - above it, the session's Push status (`ProjectLinksWorkspace`'s
 *     "Session Push status" zone): "Note · This session is up to date.", or
 *     "Checking session…", or a gate such as "Storage has changes…";
 *   - below it, the page's own zone (`SessionDetailPage`): failed actions,
 *     a moved snapshot, and the transcript parser's caveats in its own words.
 *
 * Three kinds of thing share that one shape. A status (up to date, checking,
 * ready) is a label the Push button already carries. A gate (Pull first,
 * can't compare) is a reason Push is held. A caveat (an incomplete tool call,
 * a cut-short message) is a footnote on how the transcript was read. Only
 * events (a failed action, a moved snapshot) are messages.
 *
 * The Before frame mounts the shipped `NoticeZone` with the shipped props and
 * copy. The Proposed frame takes two axes — where the Push gate goes, and how
 * caveats show — and the owner's decision of 2026-10-03: "up to date" is not
 * shown at all.
 *
 * `npm run dev`, then /mockups/session-notices-mockup.html
 * (`?scene=screenshot|checking|running|pull|compare|many|event&status=zone|above|chip&caveats=line|box|chip|anchored&copy=new|today&width=wide|default|floor&page=session|agent&theme=light|dark&only=all|before|proposed&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Scene = "screenshot" | "checking" | "running" | "pull" | "compare" | "many" | "event";
type StatusPlace = "zone" | "above" | "chip";
type CaveatForm = "line" | "box" | "chip" | "anchored";
type Width = "wide" | "default" | "floor";
type Dir = "before" | "proposed";

interface Knobs {
  scene: Scene;
  status: StatusPlace;
  caveats: CaveatForm;
  copy: "new" | "today";
  width: Width;
  page: "session" | "agent";
  theme: "light" | "dark";
  only: Dir | "all";
  shot: "off" | "on";
}

const SCENE_IDS: readonly Scene[] = ["screenshot", "checking", "running", "pull", "compare", "many", "event"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scene: pick("scene", SCENE_IDS, "screenshot"),
    status: pick("status", ["zone", "above", "chip"] as const, "chip"),
    caveats: pick("caveats", ["line", "box", "chip", "anchored"] as const, "chip"),
    copy: pick("copy", ["new", "today"] as const, "new"),
    width: pick("width", ["wide", "default", "floor"] as const, "wide"),
    page: pick("page", ["session", "agent"] as const, "session"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", "before", "proposed"] as const, "all"),
    shot: pick("shot", ["off", "on"] as const, "off"),
  };
}

/* ——— the data the page is drawn from ———————————————————————————————— */

type PushState = "up_to_date" | "ready" | "checking" | "require_pull" | "compare_error";
type CaveatCode = "incomplete_tool_call" | "message_preview_truncated" | "message_phase_inferred" | "malformed_record";

interface SceneDef {
  label: string;
  push: PushState;
  caveats: CaveatCode[];
  active: boolean;
  snapshotChanged: boolean;
}

const SCENES: Record<Scene, SceneDef> = {
  screenshot: { label: "Screenshot", push: "up_to_date", caveats: ["incomplete_tool_call"], active: false, snapshotChanged: false },
  checking: { label: "Checking → ready", push: "checking", caveats: [], active: false, snapshotChanged: false },
  running: { label: "Running session", push: "ready", caveats: ["incomplete_tool_call"], active: true, snapshotChanged: false },
  pull: { label: "Pull first", push: "require_pull", caveats: ["message_phase_inferred"], active: false, snapshotChanged: false },
  compare: { label: "Can’t compare", push: "compare_error", caveats: [], active: false, snapshotChanged: false },
  many: {
    label: "Four caveats",
    push: "ready",
    caveats: ["incomplete_tool_call", "message_preview_truncated", "message_phase_inferred", "malformed_record"],
    active: false,
    snapshotChanged: false,
  },
  event: { label: "Snapshot moved", push: "up_to_date", caveats: ["incomplete_tool_call"], active: false, snapshotChanged: true },
};

const STORAGE = "Mallard Cloud team-space";
const PROJECT = "mallard-app";
const TITLE = "Run the e2e suites without xlarge";
const AGENT_TITLE = "Map the e2e suites and their sizes";
const COMPARE_ERROR = `Mallard couldn’t read ${STORAGE}: the request timed out.`;

/** `sessionPushIssue` as the workspace computes it today. */
function todayPushIssue(push: PushState): string | null {
  switch (push) {
    case "up_to_date": return "This session is up to date.";
    case "checking": return "Checking session…";
    case "require_pull": return "Storage has changes. Review Pull before pushing this session.";
    case "compare_error": return COMPARE_ERROR;
    case "ready": return null;
  }
}

interface CaveatDef {
  today: { severity: NoticeSeverity; text: string };
  severity: NoticeSeverity;
  text: ReactNode;
  /** The SHOW-bar control the caveat qualifies, for the anchored form. */
  anchor: "tools" | "events";
  /** Tool calls are off by default; this note offers to turn them on. */
  showsTools?: boolean;
}

const CAVEATS: Record<CaveatCode, CaveatDef> = {
  incomplete_tool_call: {
    today: { severity: "warning", text: "At least one Claude tool-use block has no durable matching result." },
    severity: "info",
    text: <>Tool calls marked <em>incomplete</em> never recorded a result. They were most likely interrupted.</>,
    anchor: "tools",
    showsTools: true,
  },
  message_preview_truncated: {
    today: {
      severity: "warning",
      text: "At least one Claude message exceeds the 1-MiB timeline display limit and is shown as a truncated preview.",
    },
    severity: "warning",
    text: <>Messages over 1 MiB are cut short here. The session file keeps the full text.</>,
    anchor: "events",
  },
  message_phase_inferred: {
    today: {
      severity: "info",
      text: "At least one Claude assistant message lacked a terminal stop reason, so its final-response placement was inferred.",
    },
    severity: "info",
    text: <>Some replies had no end marker, so Mallard guessed which are final replies and which are steps.</>,
    anchor: "events",
  },
  malformed_record: {
    today: { severity: "warning", text: "At least one Claude record was malformed and could not be indexed." },
    severity: "warning",
    text: <>Some lines of the session file couldn’t be read and are left out.</>,
    anchor: "events",
  },
};

const RUNG: Record<NoticeSeverity, string> = { danger: "Error", warning: "Warning", info: "Note", success: "Done" };
const RUNG_ICON: Record<NoticeSeverity, IconName> = {
  danger: "octagon-x",
  warning: "alert-triangle",
  info: "info",
  success: "check-circle",
};

const loudest = (rungs: NoticeSeverity[]): NoticeSeverity =>
  rungs.includes("danger") ? "danger" : rungs.includes("warning") ? "warning" : rungs.includes("info") ? "info" : "success";

const noop = () => undefined;

/* ——— the path bar, as the workspace mounts it on a session ————————————— */

const STORAGE_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "storage-settings", label: "Storage settings…", icon: "settings", onSelect: noop }] },
];
const PROJECT_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop }] },
];
const APP_MENU: PathBarMenuGroup[] = [
  { items: [{ id: "settings", label: "Settings…", icon: "settings", onSelect: noop }] },
];

function leafMark(active: boolean) {
  return (
    <span className={`sync-path-leaf-mark${active ? " active" : ""}`}>
      <ProviderMark provider="claude" displayName="Claude" />
      {active && <span className="sync-visually-hidden">Active</span>}
    </span>
  );
}

function crumbsOf(knobs: Knobs, active: boolean): PathBarCrumb[] {
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: STORAGE, icon: "storage-mallard", tone: "root", menu: STORAGE_MENU, menuLabel: "Storage" },
    { id: "project", label: PROJECT, icon: "git-folder", onActivate: noop, menu: PROJECT_MENU, menuLabel: "Project" },
  ];
  if (knobs.page === "agent") {
    crumbs.push({ id: "session-parent", label: TITLE, title: `Back to ${TITLE}`, leading: leafMark(false), onActivate: noop });
    crumbs.push({ id: "session", label: AGENT_TITLE, leading: leafMark(false) });
  } else {
    crumbs.push({ id: "session", label: TITLE, leading: leafMark(active) });
  }
  return crumbs;
}

function PathActions({ push }: { push: PushState }) {
  const issue = todayPushIssue(push);
  return (
    <>
      <button type="button" className="sync-path-icon-action" title="Refresh session" aria-label="Refresh session">
        <span aria-hidden="true" className="icon-spin-frame">
          <Icon name="refresh" size={16} className={push === "checking" ? "icon-spin" : undefined} />
        </span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" title="Review and pull changes from this storage">
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
        </button>
        <button
          type="button"
          className="sync-path-sync"
          disabled={issue !== null}
          title={issue ?? "Review and push this session to this storage"}
        >
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {push === "ready" && <span className="sync-path-count" aria-hidden="true">1</span>}
        </button>
      </div>
    </>
  );
}

/* ——— the notes list, shared by the footnote, the chip and the anchors ——— */

interface Shown {
  code: CaveatCode;
  severity: NoticeSeverity;
  text: ReactNode;
}

function NoteLine({ note, toolsOn, onShowTools }: { note: Shown; toolsOn: boolean; onShowTools: () => void }) {
  return (
    <p className={`mk-note ${note.severity}`}>
      <Icon name={RUNG_ICON[note.severity]} size={12} />
      <span>
        <b>{RUNG[note.severity]} ·</b> {note.text}
        {CAVEATS[note.code].showsTools && !toolsOn && (
          <button type="button" className="mk-note-action" onClick={onShowTools}>Show tool calls</button>
        )}
      </span>
    </p>
  );
}

/** A top-layer list under its trigger. The app would seat it through the
 *  existing `TopLayer`; the specimen uses a native popover. */
function NotesPopover({
  id,
  anchor,
  title,
  notes,
  toolsOn,
  onShowTools,
}: {
  id: string;
  anchor: RefObject<HTMLElement | null>;
  title: string;
  notes: Shown[];
  toolsOn: boolean;
  onShowTools: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const pop = ref.current;
    if (!pop) return;
    const place = (event: Event) => {
      if ((event as ToggleEvent).newState !== "open" || !anchor.current) return;
      const rect = anchor.current.getBoundingClientRect();
      pop.style.top = `${Math.round(rect.bottom + 6)}px`;
      pop.style.left = `${Math.round(Math.max(16, Math.min(rect.left, window.innerWidth - 436)))}px`;
    };
    const close = () => { if (pop.matches(":popover-open")) pop.hidePopover(); };
    const scroller = document.getElementById("root");
    pop.addEventListener("beforetoggle", place);
    scroller?.addEventListener("scroll", close, { passive: true });
    return () => {
      pop.removeEventListener("beforetoggle", place);
      scroller?.removeEventListener("scroll", close);
    };
  }, [anchor]);
  return (
    <div ref={ref} id={id} popover="auto" className="mk-pop" role="dialog" aria-label={title}>
      <p className="mk-pop-title">{title}</p>
      {notes.map((note) => (
        <NoteLine key={note.code} note={note} toolsOn={toolsOn} onShowTools={() => { onShowTools(); ref.current?.hidePopover(); }} />
      ))}
    </div>
  );
}

function NotesChip({ dir, notes, toolsOn, onShowTools }: { dir: Dir; notes: Shown[]; toolsOn: boolean; onShowTools: () => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const rung = loudest(notes.map((note) => note.severity));
  const id = `mk-pop-chip-${dir}`;
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className={`session-detail-chip mk-notes-chip ${rung}`}
        popoverTarget={id}
        aria-label={`${notes.length} ${notes.length === 1 ? "note" : "notes"} on this transcript`}
      >
        <Icon name={RUNG_ICON[rung]} size={12} />
        {notes.length} {notes.length === 1 ? "note" : "notes"}
      </button>
      <NotesPopover id={id} anchor={anchor} title="Notes on this transcript" notes={notes} toolsOn={toolsOn} onShowTools={onShowTools} />
    </>
  );
}

function AnchorMark({ id, notes, toolsOn, onShowTools }: { id: string; notes: Shown[]; toolsOn: boolean; onShowTools: () => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const rung = loudest(notes.map((note) => note.severity));
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className={`mk-anchor ${rung}`}
        popoverTarget={id}
        aria-label={`${notes.length} ${notes.length === 1 ? "note" : "notes"}`}
        title={`${notes.length} ${notes.length === 1 ? "note" : "notes"}`}
      >
        <Icon name={RUNG_ICON[rung]} size={12} />
      </button>
      <NotesPopover id={id} anchor={anchor} title="Notes on this list" notes={notes} toolsOn={toolsOn} onShowTools={onShowTools} />
    </>
  );
}

function Footnote({ notes, toolsOn, onShowTools }: { notes: Shown[]; toolsOn: boolean; onShowTools: () => void }) {
  if (notes.length === 0) return null;
  if (notes.length === 1) {
    return (
      <aside className="mk-notes" aria-label="Notes on this transcript">
        <NoteLine note={notes[0]} toolsOn={toolsOn} onShowTools={onShowTools} />
      </aside>
    );
  }
  const rung = loudest(notes.map((note) => note.severity));
  const warnings = notes.filter((note) => note.severity === "warning").length;
  return (
    <aside className="mk-notes" aria-label="Notes on this transcript">
      <details>
        <summary className={rung}>
          <Icon name={RUNG_ICON[rung]} size={12} />
          <span><b>{notes.length} notes</b> on how Mallard read this transcript{warnings > 0 ? ` · ${warnings} ${warnings === 1 ? "warning" : "warnings"}` : ""}</span>
          <Icon name="chevron-right" size={12} className="mk-fold" />
        </summary>
        <ul>
          {notes.map((note) => (
            <li key={note.code}><NoteLine note={note} toolsOn={toolsOn} onShowTools={onShowTools} /></li>
          ))}
        </ul>
      </details>
    </aside>
  );
}

/* ——— the session page —————————————————————————————————————————————— */

interface PageModel {
  dir: Dir;
  knobs: Knobs;
  scene: SceneDef;
  push: PushState;
  toolsOn: boolean;
  onShowTools: () => void;
  onToggleTools: () => void;
}

/** The gate the proposal keeps: a reason Push is held that the user clears. */
function gateItem(push: PushState, copy: Knobs["copy"]): NoticeItem | null {
  if (push === "require_pull") {
    return {
      key: "session-push-gate",
      severity: "info",
      message: copy === "new"
        ? "Storage has changes. Pull them before pushing this session."
        : "Storage has changes. Review Pull before pushing this session.",
      action: (
        <button type="button" className="btn btn-ghost">
          <Icon name="download" size={13} />
          Open Pull review
        </button>
      ),
    };
  }
  if (push === "compare_error") {
    return {
      key: "session-push-gate",
      severity: "danger",
      title: copy === "new" ? "Couldn’t compare this session." : undefined,
      message: COMPARE_ERROR,
    };
  }
  return null;
}

function snapshotItem(): NoticeItem {
  return {
    key: "snapshot-changed",
    severity: "warning",
    title: "This session changed.",
    message: "Refresh before continuing so pages are not mixed across snapshots.",
    action: <button type="button" className="btn btn-ghost">Refresh</button>,
  };
}

function StatusChip({ push }: { push: PushState }) {
  if (push !== "require_pull" && push !== "compare_error") return null;
  const pull = push === "require_pull";
  const sentence = pull ? "Storage has changes. Pull them before pushing this session." : COMPARE_ERROR;
  return (
    <span className={`session-detail-chip mk-status-chip ${pull ? "info" : "danger"}`} title={sentence}>
      <Icon name={pull ? "download" : "octagon-x"} size={12} />
      {pull ? "Pull first" : "Can’t compare"}
      <span className="sync-visually-hidden"> {sentence}</span>
    </span>
  );
}

function SessionPage({ dir, knobs, scene, push, toolsOn, onShowTools, onToggleTools }: PageModel) {
  const before = dir === "before";
  const agent = knobs.page === "agent";
  const back: PageBackTarget = agent
    ? { voice: "name", label: TITLE, onActivate: noop }
    : { voice: "app", label: "Git history", onActivate: noop };

  /* What the transcript caveats say, in the frame's words. The proposal drops
     the incomplete-call note while the session runs: the last call is in
     flight, and its row already says so. */
  const notes: Shown[] = scene.caveats
    .filter((code) => before || !(code === "incomplete_tool_call" && scene.active))
    .map((code) => {
      const def = CAVEATS[code];
      return before || knobs.copy === "today"
        ? { code, severity: def.today.severity, text: def.today.text }
        : { code, severity: def.severity, text: def.text };
    });

  /* The page's one zone. */
  const zone: NoticeItem[] = [];
  if (scene.snapshotChanged) zone.push(snapshotItem());
  if (!before && knobs.status === "zone") {
    const gate = gateItem(push, knobs.copy);
    if (gate) zone.push(gate);
  }
  if (before || knobs.caveats === "box") {
    for (const note of notes) zone.push({ key: note.code, severity: note.severity, message: note.text });
  }
  const timing: NoticeTiming = scene.snapshotChanged
    ? { timing: "blocking" }
    : { timing: "standing", label: "Notes about this session" };

  const toolNotes = notes.filter((note) => CAVEATS[note.code].anchor === "tools");
  const eventNotes = notes.filter((note) => CAVEATS[note.code].anchor === "events");
  const anchored = !before && knobs.caveats === "anchored";

  const toggle = (label: string, count: string, on: boolean, quality: string, onClick?: () => void) => (
    <button type="button" className={`session-detail-filter-toggle${on ? " is-on" : ""}`} aria-pressed={on} onClick={onClick}>
      <span className="session-detail-filter-box" aria-hidden="true" />
      <span className="session-detail-filter-name">{label}</span>
      <span className={`session-detail-filter-count quality-${quality}`}>{count}</span>
    </button>
  );

  const shownEvents = scene.active ? (toolsOn ? 3 : 1) : (toolsOn ? 4 : 2);

  return (
    <section className="session-detail-page" aria-label={agent ? AGENT_TITLE : TITLE}>
      <header className="session-detail-header">
        <div className="session-detail-strip">
          <div className="session-detail-lead">
            <PageBack target={back} />
            <div className="session-detail-chips" aria-label="Session facts">
              <span className="session-detail-chip"><Icon name="git-branch" size={12} /><code>HEAD</code></span>
              <span className="session-detail-chip"><time>Oct 1, 2026 · 12:10</time></span>
              <span className="session-detail-chip">
                <Icon name={scene.active ? "activity" : "clock"} size={12} /><b>{scene.active ? "running" : agent ? "6m" : "2h 48m"}</b>
              </span>
              <span className="session-detail-chip"><code>{agent ? "312K" : "4.80M"}</code> tokens</span>
              {agent && <span className="session-detail-chip"><Icon name="git-fork" size={12} /><code>explorer</code></span>}
              {agent && <span className="session-detail-chip"><code>claude-sonnet-5-5</code></span>}
              {agent && <span className="session-detail-chip">background</span>}
              {!before && knobs.caveats === "chip" && notes.length > 0 && (
                <NotesChip dir={dir} notes={notes} toolsOn={toolsOn} onShowTools={onShowTools} />
              )}
            </div>
          </div>
          <div className="session-detail-header-actions" aria-label="Session actions">
            {!before && knobs.status === "chip" && <StatusChip push={push} />}
            <button type="button" className="btn btn-ghost session-detail-launch" aria-label="Open in Claude" title="Open in Claude">
              <ProviderMark provider="claude" displayName="Claude" size={12} />
            </button>
            <button type="button" className="btn btn-ghost session-detail-launch" aria-label="Open in Terminal" title="Open in Terminal">
              <Icon name="terminal" size={12} />
            </button>
          </div>
        </div>
      </header>

      <NoticeZone
        items={zone}
        subject="notes about this session"
        className={before ? "session-detail-notices" : "mk-zone"}
        {...timing}
      />

      {!before && knobs.caveats === "line" && <Footnote notes={notes} toolsOn={toolsOn} onShowTools={onShowTools} />}

      <div className="session-detail-layout">
        <div className="session-detail-toolbar">
          <span className="session-detail-toolbar-label">Show</span>
          <div className="session-detail-toolbar-toggles" role="group" aria-label="Show">
            {toggle("Prompts", "9", true, "exact")}
            {toggle("Replies", "12", true, "exact")}
            {toggle("Steps", "41", false, "exact")}
            {toggle("Tool calls", "31+", toolsOn, "lower_bound", onToggleTools)}
            {anchored && toolNotes.length > 0 && (
              <AnchorMark id={`mk-pop-tools-${dir}`} notes={toolNotes} toolsOn={toolsOn} onShowTools={onShowTools} />
            )}
          </div>
          <output className={`session-detail-toolbar-readout${anchored && eventNotes.length > 0 ? " mk-has-anchor" : ""}`}>
            {shownEvents} of 93+ events
            {anchored && eventNotes.length > 0 && (
              <AnchorMark id={`mk-pop-events-${dir}`} notes={eventNotes} toolsOn={toolsOn} onShowTools={onShowTools} />
            )}
          </output>
        </div>
        <div className="session-detail-days">
          <div className="session-detail-day">
            <h2 className="session-detail-day-divider"><span>Oct 1, 2026</span></h2>
            <ol className="session-detail-event-list">
              <li>
                <article className="session-detail-event event-prompt">
                  <span className="session-detail-event-node" role="img" aria-label="You · 12:10"><Icon name="user" size={12} /></span>
                  <div className="session-detail-event-body">
                    <div className="session-detail-card session-detail-card-prompt">
                      <div className="session-detail-card-head"><time>12:10</time><span>{scene.active ? "running" : "2h 48m"}</span><span>31 calls</span></div>
                      <div className="session-detail-event-copy">
                        <p>{agent
                          ? "List every e2e suite under tests/ with its size class, and say which ones the xlarge tag covers."
                          : "help me run all the e2e tests exclude the xlarge one"}</p>
                      </div>
                    </div>
                  </div>
                </article>
              </li>
              {toolsOn && (
                <li>
                  <article className="session-detail-event event-tool-call tool-completed">
                    <span className="session-detail-event-tick" aria-hidden="true" />
                    <div className="session-detail-event-body">
                      <details className="session-detail-tool-call status-completed">
                        <summary>
                          <span className="session-detail-tool-name cat-read">Read</span>
                          <span className="session-detail-tool-summary">tests/e2e/README.md</span>
                          <span className="session-detail-tool-chevron" aria-hidden="true"><Icon name="chevron-right" size={12} /></span>
                        </summary>
                      </details>
                    </div>
                  </article>
                </li>
              )}
              {toolsOn && (
                <li>
                  <article className="session-detail-event event-tool-call tool-incomplete">
                    <span className="session-detail-event-tick" aria-hidden="true" />
                    <div className="session-detail-event-body">
                      <div className="session-detail-event-flags">
                        <span className="session-detail-quality quality-lower_bound">Partial</span>
                      </div>
                      <details className="session-detail-tool-call status-incomplete">
                        <summary>
                          <span className="session-detail-tool-name cat-shell">Bash</span>
                          <span className="session-detail-tool-summary">npm run test:e2e -- --exclude xlarge</span>
                          <span className="session-detail-tool-status status-incomplete">incomplete</span>
                          <span className="session-detail-tool-chevron" aria-hidden="true"><Icon name="chevron-right" size={12} /></span>
                        </summary>
                      </details>
                    </div>
                  </article>
                </li>
              )}
              {!scene.active && (
                <li>
                  <article className="session-detail-event event-response">
                    <span className="session-detail-event-node" role="img" aria-label="Claude · 14:58">
                      <ProviderMark provider="claude" displayName="Claude" size={12} />
                    </span>
                    <div className="session-detail-event-body">
                      <div className="session-detail-card session-detail-card-reply">
                        <div className="session-detail-card-head"><time>14:58</time></div>
                        <div className="session-detail-event-copy">
                          <p>The run was stopped before the suite finished. Every suite up to <code>sync-large</code> passed; the remaining three did not start.</p>
                        </div>
                      </div>
                    </div>
                  </article>
                </li>
              )}
            </ol>
          </div>
        </div>
      </div>
    </section>
  );
}

/* ——— frames ———————————————————————————————————————————————————————— */

const STATUS_NOTE: Record<StatusPlace, ReactNode> = {
  zone: <><b>Push status:</b> up to date, checking and ready draw nothing — Push’s own state, tooltip and count say them. A gate the user must clear (Pull first, can’t compare) joins the page’s one box.</>,
  above: <><b>Push status:</b> up to date, checking and ready draw nothing. A gate keeps today’s place, its own box above the strip.</>,
  chip: <><b>Push status:</b> up to date, checking and ready draw nothing. A gate is a chip at the head of the strip’s verbs, under Push; the sentence is its tooltip.</>,
};

const CAVEAT_NOTE: Record<CaveatForm, ReactNode> = {
  line: <><b>Caveats:</b> a footnote above SHOW — no frame, no ×, the rung named in its ink. Several fold under one line. <i>Owner call: today they close.</i></>,
  box: <><b>Caveats:</b> stay messages in the box, in plain words; the incomplete-call note drops to Note and hides while the session runs.</>,
  chip: <><b>Caveats:</b> a chip at the end of the facts; the list opens from it. Quietest; the words are a click away. <i>Owner call: a label, no ×.</i></>,
  anchored: <><b>Caveats:</b> a mark on what they qualify — tool-call notes on Tool calls, the rest on the event count. <i>Owner call: a label, no ×.</i></>,
};

/** The main pane beside a 318px sidebar: the screenshot's, an 1100px window's, and the floor. */
function paneWidth(width: Width): number {
  return width === "wide" ? 990 : width === "default" ? 782 : 580;
}

function Frame({ dir, knobs, push }: { dir: Dir; knobs: Knobs; push: PushState }) {
  const scene = SCENES[knobs.scene];
  const [toolsOn, setToolsOn] = useState(false);
  useEffect(() => { setToolsOn(false); }, [knobs.scene]);
  const before = dir === "before";
  const issue = todayPushIssue(push);
  const aboveItems: NoticeItem[] = before
    ? issue ? [{ key: "session-push", severity: "info", message: issue }] : []
    : knobs.status === "above" ? [gateItem(push, knobs.copy)].filter((item): item is NoticeItem => item !== null) : [];
  return (
    <div className="spec-frame">
      <SyncPathBar
        crumbs={crumbsOf(knobs, scene.active)}
        actions={<PathActions push={push} />}
        overflow={APP_MENU}
        overflowLabel="App actions"
        ariaLabel={`${PROJECT} path`}
      />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page">
        {aboveItems.length > 0 && (
          <NoticeZone
            items={aboveItems}
            subject="session Push status"
            timing="standing"
            label="Session Push status"
          />
        )}
        <SessionPage
          dir={dir}
          knobs={knobs}
          scene={scene}
          push={push}
          toolsOn={toolsOn}
          onShowTools={() => setToolsOn(true)}
          onToggleTools={() => setToolsOn((on) => !on)}
        />
      </main>
    </div>
  );
}

/* ——— measuring ——————————————————————————————————————————————————————— */

interface Measure { gap: number; boxes: number }

function measureAll(): Record<string, Measure> {
  const out: Record<string, Measure> = {};
  document.querySelectorAll<HTMLElement>("[data-dir]").forEach((column) => {
    const bar = column.querySelector<HTMLElement>("header.sync-pathbar");
    const toolbar = column.querySelector<HTMLElement>(".session-detail-toolbar");
    if (!bar || !toolbar || !column.dataset.dir) return;
    out[column.dataset.dir] = {
      gap: Math.round(toolbar.getBoundingClientRect().top - bar.getBoundingClientRect().bottom),
      boxes: column.querySelectorAll(".spec-frame .notice").length,
    };
  });
  return out;
}

(window as unknown as { mkMeasure: typeof measureAll }).mkMeasure = measureAll;

/* ——— the page ——————————————————————————————————————————————————— */

const PRESETS: { id: string; label: string; status: StatusPlace; caveats: CaveatForm; rec?: boolean }[] = [
  { id: "rec", label: "Box for gates, footnote for caveats", status: "zone", caveats: "line" },
  { id: "quiet", label: "Chips only (picked)", status: "chip", caveats: "chip", rec: true },
  { id: "place", label: "In place", status: "zone", caveats: "anchored" },
  { id: "boxes", label: "Boxes, new words", status: "zone", caveats: "box" },
];

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [phase, setPhase] = useState<"checking" | "ready">("checking");
  const [play, setPlay] = useState(true);
  const [measures, setMeasures] = useState<Record<string, Measure>>({});
  const [jump, setJump] = useState<Record<string, Partial<Record<"checking" | "ready", number>>>>({});

  const set = (patch: Partial<Knobs>) => {
    const next = { ...knobs, ...patch };
    setKnobs(next);
    const query = new URLSearchParams({ ...next });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string, rec = false) => (
    <button
      key={String(value)}
      type="button"
      className={`spec-seg${rec ? " rec" : ""}`}
      aria-pressed={knobs[key] === value}
      onClick={() => set({ [key]: value } as Partial<Knobs>)}
    >{label}</button>
  );

  const checkingScene = knobs.scene === "checking";
  const push: PushState = checkingScene ? (phase === "checking" ? "checking" : "ready") : SCENES[knobs.scene].push;

  useEffect(() => {
    if (!checkingScene || !play) return;
    const timer = window.setInterval(() => setPhase((current) => (current === "checking" ? "ready" : "checking")), 1800);
    return () => window.clearInterval(timer);
  }, [checkingScene, play]);

  useEffect(() => {
    let live = true;
    void document.fonts.ready.then(() => window.setTimeout(() => {
      if (!live) return;
      const next = measureAll();
      setMeasures(next);
      if (checkingScene) {
        setJump((current) => {
          const out = { ...current };
          for (const [dir, measure] of Object.entries(next)) out[dir] = { ...out[dir], [phase]: measure.gap };
          return out;
        });
      }
    }, 60));
    return () => { live = false; };
  }, [knobs, phase, checkingScene]);

  const shown: Dir[] = knobs.only === "all" ? ["before", "proposed"] : [knobs.only];
  const width = paneWidth(knobs.width);
  const presetActive = (preset: (typeof PRESETS)[number]) => knobs.status === preset.status && knobs.caveats === preset.caveats;

  return (
    <div className="spec">
      {knobs.shot === "off" && (
        <>
          <h1 className="spec-title">Messages on the session page</h1>
          <p className="spec-sub">
            Both frames are the main pane at the chosen width, drawn with the shipped path bar, session page classes,
            and <code>NoticeZone</code> on the shipped CSS. Only classes prefixed <code>mk-</code> are new. The figure
            under each frame is measured: the distance from the path bar to the SHOW bar, and the boxes drawn.
            Owner decisions 2026-10-03: <i>up to date</i> is not shown; <b>Chips only</b> picked and implemented
            (docs/PLAN_SESSION_DETAIL_NOTICES.md, live check at /mockups/session-notices-live.html).
          </p>
          <div className="spec-controls">
            <div className="spec-row">
              <div className="spec-group"><span>Scene</span>{SCENE_IDS.map((id) => seg("scene", id, SCENES[id].label))}</div>
              {checkingScene && (
                <div className="spec-group">
                  <span>Check</span>
                  <button type="button" className="spec-seg" aria-pressed={play} onClick={() => setPlay(!play)}>{play ? "Playing" : "Paused"}</button>
                  <button type="button" className="spec-seg" aria-pressed={phase === "checking"} onClick={() => { setPlay(false); setPhase("checking"); }}>Checking</button>
                  <button type="button" className="spec-seg" aria-pressed={phase === "ready"} onClick={() => { setPlay(false); setPhase("ready"); }}>Ready</button>
                </div>
              )}
            </div>
            <div className="spec-row">
              <div className="spec-group">
                <span>Preset</span>
                {PRESETS.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    className={`spec-seg${preset.rec ? " rec" : ""}`}
                    aria-pressed={presetActive(preset)}
                    onClick={() => set({ status: preset.status, caveats: preset.caveats })}
                  >{preset.label}</button>
                ))}
              </div>
            </div>
            <div className="spec-row">
              <div className="spec-group"><span>Push gate</span>{seg("status", "zone", "In the page’s box")}{seg("status", "above", "Own box above")}{seg("status", "chip", "Chip under Push", true)}</div>
              <div className="spec-group"><span>Caveats</span>{seg("caveats", "line", "Footnote")}{seg("caveats", "box", "Box")}{seg("caveats", "chip", "Notes chip", true)}{seg("caveats", "anchored", "On SHOW")}</div>
              <div className="spec-group"><span>Words</span>{seg("copy", "new", "Plain", true)}{seg("copy", "today", "Today’s")}</div>
            </div>
            <div className="spec-row">
              <div className="spec-group"><span>Page</span>{seg("page", "session", "Session")}{seg("page", "agent", "Agent’s page")}</div>
              <div className="spec-group"><span>Pane</span>{seg("width", "wide", "Screenshot")}{seg("width", "default", "1100 px window")}{seg("width", "floor", "Minimum")}</div>
              <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
            </div>
          </div>
        </>
      )}

      <div className="spec-grid">
        {shown.map((dir) => {
          const measure = measures[dir];
          const moves = jump[dir];
          const delta = checkingScene && moves?.checking !== undefined && moves.ready !== undefined
            ? Math.abs(moves.checking - moves.ready)
            : null;
          return (
            <div key={dir} data-dir={dir} style={{ width }}>
              <h2 className="spec-col-title">{dir === "before" ? "Before" : "Proposed"}</h2>
              <p className="spec-col-note">
                {dir === "before"
                  ? <>Two boxes around the strip. The Note above is the session’s Push status; the Warning below is the transcript parser’s caveat in its own words. Both close, and both come back on the next open.</>
                  : <>{STATUS_NOTE[knobs.status]} {CAVEAT_NOTE[knobs.caveats]}</>}
              </p>
              <Frame dir={dir} knobs={knobs} push={push} />
              {measure && (
                <p className={`spec-measure${delta ? " over" : ""}`}>
                  {width} px pane · SHOW bar {measure.gap} px below the path bar · {measure.boxes} {measure.boxes === 1 ? "box" : "boxes"}
                  {delta !== null && ` · moves ${delta} px when the check lands`}
                </p>
              )}
            </div>
          );
        })}
      </div>

      {knobs.shot === "off" && (
        <section className="spec-copy">
          <h2 className="spec-col-title">The words</h2>
          <p className="spec-col-note">
            Caveats are written in Rust (<code>providers/claude/history.rs</code> and <code>providers/codex/history.rs</code>),
            so the plain words are a backend change. The incomplete-call rung moves from Warning to Note: nothing is
            lost — the transcript itself holds no result.
          </p>
          <table>
            <thead><tr><th>Code</th><th>Today</th><th>Proposed</th></tr></thead>
            <tbody>
              <tr>
                <td>push: no_changes</td>
                <td className="old"><span className="spec-rung info">Note</span> This session is up to date.</td>
                <td><span className="spec-rung hidden">—</span> Not shown. Push is disabled and its tooltip says it.</td>
              </tr>
              <tr>
                <td>push: checking</td>
                <td className="old"><span className="spec-rung info">Note</span> Checking session…</td>
                <td><span className="spec-rung hidden">—</span> Not shown. The refresh mark spins; Push’s tooltip says it.</td>
              </tr>
              <tr>
                <td>push: ready</td>
                <td className="old"><span className="spec-rung hidden">—</span> Nothing (the box above unmounts).</td>
                <td><span className="spec-rung hidden">—</span> Nothing. Push wears its count, 1.</td>
              </tr>
              <tr>
                <td>push: require_pull</td>
                <td className="old"><span className="spec-rung info">Note</span> Storage has changes. Review Pull before pushing this session.</td>
                <td><span className="spec-rung info">Note</span> Storage has changes. Pull them before pushing this session. <i>[Open Pull review]</i></td>
              </tr>
              <tr>
                <td>push: comparison error</td>
                <td className="old"><span className="spec-rung info">Note</span> {COMPARE_ERROR}</td>
                <td><span className="spec-rung danger">Error</span> <b>Couldn’t compare this session.</b> {COMPARE_ERROR}</td>
              </tr>
              {(Object.keys(CAVEATS) as CaveatCode[]).map((code) => (
                <tr key={code}>
                  <td>{code}</td>
                  <td className="old"><span className={`spec-rung ${CAVEATS[code].today.severity}`}>{RUNG[CAVEATS[code].today.severity]}</span> {CAVEATS[code].today.text}</td>
                  <td>
                    <span className={`spec-rung ${CAVEATS[code].severity}`}>{RUNG[CAVEATS[code].severity]}</span> {CAVEATS[code].text}
                    {code === "incomplete_tool_call" && <i> Hidden while the session runs. [Show tool calls] when they are off.</i>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
