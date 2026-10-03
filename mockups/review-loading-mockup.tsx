import { StrictMode, createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import LoadingScaffold, { LoadingBone } from "../src/components/LoadingScaffold";
import { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import NameSearchField from "../src/components/sync-review/NameSearchField";
import { sessionNoun } from "../src/components/project-sync/model";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import { ReviewBudgetGauge, ReviewBudgetGlance, type ReviewBudget } from "../src/components/sync-review/ReviewBudgetMeter";
import SyncPathBar, { PathBarBelow } from "../src/components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "../src/components/sync-review/ThreadStatusIcon";
import ResourceInventory from "../src/components/project-sync/ResourceInventory";
import { pullRequiredCopy } from "../src/components/project-sync/PushResourceWorkspace";
import { formatBytes } from "../src/format";
import type { InventoryResource } from "../src/types";
import "../src/App.css";

/**
 * Mockup: the wait between clicking Push or Pull and a review you can act on.
 *
 * Today that wait happens in three places at once. First the path-bar button
 * spins "Preparing…" over an Activity page that doesn't change. Then the
 * review mounts before its list, with meters at 0 B, a count for rows that
 * aren't on screen, and a green "Done · ready" message. Last, Pull's list goes
 * blank while the comparison is still in flight. Each frame below is one
 * moment of that wait, drawn with the shipped leaf components on the shipped
 * CSS.
 *
 *   Before  what ships (Pull: 5 moments, Push: 3 + the gate)
 *   A       the storage card opens at the click; the review's own frame
 *           holds the stage line and Cancel, then becomes the review, or
 *           the card's notice, in place
 *   B       Activity stays; a progress notice under the path bar carries
 *           the stage and Cancel; the review appears only once it is whole
 *
 * Every stage named here is one Mallard can report today: Pull's plan fetch
 * sends objects and bytes, and the review list reports `activity` then
 * `comparison`. Push's prepare is one call, so it gets one stage line with an
 * indeterminate track, not a checklist.
 *
 * `npm run dev`, then /mockups/review-loading-mockup.html
 * (`?verb=pull|push&dir=before|a|b&view=strip|live&theme=light|dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type Verb = "pull" | "push";
type Dir = "before" | "a" | "b";
type View = "strip" | "live" | "compare";
/** How A's card says it is still preparing (round 2: the owner liked A, not
 *  the stage line). */
type Progress = "caption" | "quiet" | "header" | "line";
interface Knobs { verb: Verb; dir: Dir; view: View; progress: Progress; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    verb: pick("verb", ["pull", "push"] as const, "pull"),
    dir: pick("dir", ["before", "a", "b"] as const, "before"),
    view: pick("view", ["strip", "live", "compare"] as const, "strip"),
    progress: pick("progress", ["caption", "quiet", "header", "line"] as const, "caption"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

/* ——— data ——————————————————————————————————————————————————————— */

const MB = 1024 * 1024;
const LIMIT = 512 * MB;
const PROJECT = "echo-park-horror";
const STORAGE = "Attic R2";
const VERSION = 12;
const OBJECTS = 12;
const PLAN_BYTES = 9.8 * MB;

type Provider = "claude" | "codex";
const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude Code", codex: "Codex" };

interface MockSession {
  id: string;
  title: string;
  provider: Provider;
  bytes: number;
  when: string;
  messages: number;
  tools: number;
  tokens: string;
  length: string;
}

/** Only in Attic R2: what a Pull review lists. Nothing is ticked (owner rule). */
const PULL_ROWS: MockSession[] = [
  { id: "p1", title: "Tune the lantern flicker shader", provider: "claude", bytes: 42.3 * MB, when: "Sep 25, 22:14", messages: 164, tools: 131, tokens: "1.2M", length: "1h 52m" },
  { id: "p2", title: "Rework the basement chase AI", provider: "codex", bytes: 118 * MB, when: "Sep 24, 19:03", messages: 402, tools: 377, tokens: "3.4M", length: "5h 10m" },
  { id: "p3", title: "Fix save-slot corruption on quit", provider: "claude", bytes: 9.6 * MB, when: "Sep 23, 16:40", messages: 58, tools: 44, tokens: "380K", length: "41m" },
];

/** Only on this computer: what a Push review lists, defaults ticked. */
const PUSH_ROWS: MockSession[] = [
  { id: "q1", title: "Ghost pathing through locked doors", provider: "claude", bytes: 96 * MB, when: "Sep 26, 11:20", messages: 388, tools: 341, tokens: "3.0M", length: "5h 02m" },
  { id: "q2", title: "Menu flow and pause screen", provider: "codex", bytes: 58 * MB, when: "Sep 25, 17:02", messages: 190, tools: 152, tokens: "1.7M", length: "2h 48m" },
  { id: "q3", title: "Lantern battery balance", provider: "claude", bytes: 41 * MB, when: "Sep 25, 09:47", messages: 133, tools: 97, tokens: "1.1M", length: "1h 36m" },
  { id: "q4", title: "Crash on alt-tab in fullscreen", provider: "claude", bytes: 23 * MB, when: "Sep 24, 21:15", messages: 71, tools: 60, tokens: "620K", length: "58m" },
  { id: "q5", title: "Title card typography", provider: "codex", bytes: 12 * MB, when: "Sep 24, 13:30", messages: 40, tools: 29, tokens: "310K", length: "33m" },
];

/** Already the same on both sides: Activity's backdrop, never in a review. */
const SYNCED_ROWS: MockSession[] = [
  { id: "s1", title: "Add fog volume to the cellar", provider: "claude", bytes: 31 * MB, when: "Sep 22, 20:05", messages: 120, tools: 88, tokens: "900K", length: "1h 21m" },
  { id: "s2", title: "Audio occlusion pass", provider: "codex", bytes: 27 * MB, when: "Sep 21, 15:12", messages: 96, tools: 70, tokens: "760K", length: "1h 04m" },
];

const REQUIREMENT_BYTES = 3.2 * MB;
const REQUIREMENTS = [{
  resource_id: "tools:skill:level-lint",
  category: "tools",
  kind_label: "Skill",
  apply_policy_label: "Replace",
  display_name: "level-lint skill",
  selectable: true,
  default_selected: true,
}] as unknown as InventoryResource[];

const PUSH_TOTAL = PUSH_ROWS.reduce((sum, row) => sum + row.bytes, 0) + REQUIREMENT_BYTES;
const PULL_BUDGET: ReviewBudget = { total: 0, limit: LIMIT };
const PUSH_BUDGET: ReviewBudget = { total: PUSH_TOTAL, limit: LIMIT };
const PULL_BREAKDOWN = "Sessions 0 B · Setup & tools 0 B · Project files 0 B";
const PUSH_BREAKDOWN = `Sessions ${formatBytes(PUSH_TOTAL - REQUIREMENT_BYTES)} · Requirements ${formatBytes(REQUIREMENT_BYTES)} · Project files 0 B`;
/* The plan's pending rows: three sessions and one setup item. */
const PULL_PENDING = PULL_ROWS.length + 1;
const PUSH_PENDING = PUSH_ROWS.length + REQUIREMENTS.length;

const fetched = (index: number) => `${index}/${OBJECTS} · ${formatBytes((PLAN_BYTES * index) / OBJECTS)} of ${formatBytes(PLAN_BYTES)}`;
/** The shipped button readout, character for character. */
const buttonReadout = (index: number) => `${index}/${OBJECTS} · ${((PLAN_BYTES * index) / OBJECTS / 1_000_000).toFixed(1)} MB…`;

/* ——— one moment, described ——————————————————————————————————————— */

interface Prep {
  text: string;
  /** The few words the header variant has room for. */
  short: string;
  readout?: string;
  /** 0..1 for a determinate track; null draws the indeterminate sweep. */
  progress: number | null;
  steps: string[];
  step: number;
}

interface ReviewSpec {
  /** The header's meta: "Version 12" once the plan is read. */
  meta: string | null;
  /** Meter, count, ✓ and trash. Without them the header keeps only ×. */
  loaded: boolean;
  /** × reads Cancel: nothing has been read into the project yet. */
  cancel: boolean;
  /** The plan's standing warning; Pull knows it once the plan is read. */
  banner: boolean;
  body: "scaffold" | "blank" | "rows" | "prep";
  prep?: Prep;
  /** Placeholder rows: three until the plan or comparison gives the count. */
  bones?: number;
  /** Pull's Agent setup & tools row / Push's Requirements group. */
  tools: "none" | "bone" | "ready";
  footer: { gauge: boolean; label: string; enabled: boolean; title?: string; spin?: boolean };
}

interface Frame {
  page: "activity" | "review";
  bar: { label: string; readout?: string; spin?: boolean; active?: boolean; disabled?: boolean; count?: number };
  shell?: NoticeItem;
  card?: NoticeItem;
  review?: ReviewSpec;
}

interface Scene {
  id: string;
  title: string;
  note: ReactNode;
  /** How long the moment lasts in Live. */
  ms: number;
  /** An alternative ending: shown in the strip, skipped by Play. */
  ending?: boolean;
  frame: Frame;
}

const readyNotice: NoticeItem = {
  key: "ready",
  severity: "success",
  message: `Pull review ready for ${PROJECT} — nothing applied yet.`,
};

const autoAcceptNotice: NoticeItem = {
  key: "auto",
  severity: "success",
  message: `Storage reviewed for ${PROJECT} — files unchanged.`,
};

function failedNotice(verb: Verb): NoticeItem {
  return {
    key: "failed",
    severity: "danger",
    title: verb === "push" ? "Push couldn't start" : "Pull couldn't start",
    message: `${STORAGE} couldn't be reached. Check the connection, then try again.`,
    action: <div className="sync-sync-review-recovery-actions"><button type="button" className="btn btn-ghost">Retry</button></div>,
  };
}

function requiresPullNotice(): NoticeItem {
  const copy = pullRequiredCopy("remote_advanced", STORAGE, VERSION + 1, VERSION, 3);
  return {
    key: "requires-pull",
    severity: "info",
    title: copy.title,
    message: <>{copy.body}<small className="sync-sync-review-pull-detail">{copy.detail}</small></>,
    action: (
      <div className="sync-sync-review-recovery-actions">
        <button type="button" className="btn btn-ghost"><Icon name="download" size={13} />Open Pull review</button>
      </div>
    ),
  };
}

function progressNotice(verb: Verb, prep: Prep): NoticeItem {
  return {
    key: "preparing",
    severity: "info",
    title: verb === "pull" ? "Preparing Pull." : "Preparing Push.",
    message: (
      <>
        {prep.text}
        {prep.readout && <span className="mk-notice-readout">{prep.readout}</span>}
        <span className={`mk-prep-track mk-notice-track${verb === "push" ? " push" : ""}${prep.progress === null ? " indeterminate" : ""}`} aria-hidden="true">
          <i style={prep.progress === null ? undefined : { width: `${prep.progress * 100}%` }} />
        </span>
      </>
    ),
    action: <button type="button" className="btn">Cancel</button>,
  };
}

/* ——— the scenes ——————————————————————————————————————————————————— */

const PULL_STEPS = ["Read the version", "Load sessions", "Compare"];
const PUSH_STEPS = ["Compare with storage", "Load sessions"];

function scenes(verb: Verb, dir: Dir): Scene[] {
  if (verb === "pull") {
    const readoutAt = (index: number): Prep => ({
      text: `Reading the latest version from ${STORAGE}`,
      short: `Reading ${STORAGE} · ${index} of ${OBJECTS}`,
      readout: fetched(index),
      progress: index / OBJECTS,
      steps: PULL_STEPS,
      step: 0,
    });
    const loading: Prep = { text: "Loading sessions", short: "Loading sessions", progress: null, steps: PULL_STEPS, step: 1 };
    const comparing: Prep = { text: "Comparing with this computer", short: "Comparing", progress: null, steps: PULL_STEPS, step: 2 };

    if (dir === "before") {
      const mounted = (body: ReviewSpec["body"]): Frame => ({
        page: "review",
        bar: { label: "Mark as reviewed", active: true, disabled: true },
        shell: readyNotice,
        review: {
          meta: `Version ${VERSION}`, loaded: true, cancel: false, banner: true, body, tools: "ready",
          footer: { gauge: true, label: "Mark as reviewed", enabled: false, title: "Loading storage changes…" },
        },
      });
      return [
        {
          id: "click", title: "Click Pull", ms: 500,
          note: <>Only the button changes. The Activity list under it looks usable, but a busy lock has turned it off.</>,
          frame: { page: "activity", bar: { label: "Preparing…", spin: true, disabled: true, count: 3 } },
        },
        {
          id: "fetch", title: "Reading the version", ms: 2200,
          note: <>The download progress is squeezed into the button as <code>{buttonReadout(7)}</code>. Nothing says what is being read, and there is no way to stop it.</>,
          frame: { page: "activity", bar: { label: "", readout: buttonReadout(7), spin: true, disabled: true } },
        },
        {
          id: "mount", title: "The review mounts before its list", ms: 900,
          note: <><b>Done ·</b> says the review is ready, but the list is still loading. The meters show 0 B and the count says 0/4 for rows that aren’t on screen yet. The primary is disabled, and the reason is only in its tooltip.</>,
          frame: mounted("scaffold"),
        },
        {
          id: "blank", title: "Sessions loaded, comparison pending: blank", ms: 1400,
          note: <>Your screenshot. The loading placeholder is gated on the first read only. The review filter needs the comparison too, so the list renders empty between the two reads. The tools row moves up into the gap.</>,
          frame: mounted("blank"),
        },
        {
          id: "ready", title: "Ready", ms: 2600,
          note: <>The rows push the tools row down and the primary enables. The Done message is still showing.</>,
          frame: {
            ...mounted("rows"),
            bar: { label: "Mark as reviewed", active: true },
            review: { ...mounted("rows").review!, footer: { gauge: true, label: "Mark as reviewed", enabled: true } },
          },
        },
      ];
    }

    if (dir === "a") {
      const preparing = (prep: Prep, planRead: boolean, label: string): Frame => ({
        page: "review",
        bar: { label, spin: true, active: true, disabled: true },
        review: {
          meta: planRead ? `Version ${VERSION}` : null, loaded: false, cancel: true, banner: planRead,
          body: "prep", prep, bones: planRead ? PULL_ROWS.length : undefined, tools: planRead ? "bone" : "none",
          footer: { gauge: false, label, enabled: false, spin: true },
        },
      });
      return [
        {
          id: "open", title: "Click Pull: the card opens in place", ms: 600,
          note: <>If preparing is still running after 150 ms (the scaffold’s own gate), the storage card opens where the review will be. Anything faster goes straight to the review or one of the endings, so nothing flashes. <b>×</b> is <b>Cancel</b>: the result is thrown away and the page unlocks at once. The path-bar button and the footer show the same word.</>,
          frame: preparing(readoutAt(0), false, "Preparing…"),
        },
        {
          id: "fetch", title: "Reading the version", ms: 2200,
          note: <>The version downloads. The card doesn’t move; only the <b>Progress</b> choice above decides what it says in the meantime.</>,
          frame: preparing(readoutAt(7), false, "Preparing…"),
        },
        {
          id: "sessions", title: "Plan read: the version and its warning", ms: 900,
          note: <>Once the plan is read, the header can say <b>Version 12</b> and the redaction warning takes its place above the list. The warning is plan data, so it never arrives later and pushes the rows down.</>,
          frame: preparing(loading, true, "Loading sessions…"),
        },
        {
          id: "compare", title: "Comparing with this computer", ms: 1400,
          note: <>This is where today’s list goes blank. Here the placeholder rows stay until the comparison lands, and the footer says why it is waiting.</>,
          frame: preparing(comparing, true, "Loading sessions…"),
        },
        {
          id: "ready", title: "Ready, in place", ms: 2600,
          note: <>Rows replace the placeholders at the same height. The meters, the 0/4 count and ✓ appear only now, and the primary becomes <b>Mark as reviewed</b>. No Done message: the review on screen is the news.</>,
          frame: {
            page: "review",
            bar: { label: "Mark as reviewed", active: true },
            review: {
              meta: `Version ${VERSION}`, loaded: true, cancel: false, banner: true, body: "rows", tools: "ready",
              footer: { gauge: true, label: "Mark as reviewed", enabled: true },
            },
          },
        },
        {
          id: "auto", title: "Ending: nothing to review", ms: 0, ending: true,
          note: <>When the plan only needs a metadata accept, the card closes and the shipped message reports it. No review opened.</>,
          frame: { page: "activity", bar: { label: "Pull", count: 0 }, shell: autoAcceptNotice },
        },
        {
          id: "cleared", title: "Ending: nothing to pull", ms: 0, ending: true,
          note: <>A cleared storage this computer synced with before. The card becomes its shipped note; the activity list already says to push again.</>,
          frame: { page: "activity", bar: { label: "Pull", count: 0 }, card: { key: "nothing-to-pull", severity: "info", message: `There is nothing to pull from ${STORAGE}.` } },
        },
        {
          id: "failed", title: "Ending: couldn’t start", ms: 0, ending: true,
          note: <>A failed plan read turns the card into its shipped <b>Pull couldn’t start</b> notice with Retry, in the same slot. It never opens a review first.</>,
          frame: { page: "activity", bar: { label: "Pull", count: 3 }, card: failedNotice("pull") },
        },
      ];
    }

    const stay = (prep: Prep, label: string): Frame => ({
      page: "activity",
      bar: { label, spin: true, disabled: true },
      shell: progressNotice("pull", prep),
    });
    return [
      {
        id: "click", title: "Click Pull: a progress message under the bar", ms: 600,
        note: <>Activity stays on screen. The message zone under the path bar carries the stage and <b>Cancel</b>. Its × only hides the message; the Pull keeps preparing.</>,
        frame: stay(readoutAt(0), "Preparing…"),
      },
      {
        id: "fetch", title: "Reading the version", ms: 2200,
        note: <>The same objects and bytes as A, in a message rather than in the card.</>,
        frame: stay(readoutAt(7), "Preparing…"),
      },
      {
        id: "compare", title: "Loading and comparing, behind the page", ms: 2000,
        note: <>The review is mounted but hidden while it reads sessions and compares them, next to the visible Activity page. That means two activity loads at once and a new way of rendering the review. The Activity list stays locked the whole time.</>,
        frame: stay({ ...comparing, text: "Loading sessions and comparing with this computer" }, "Preparing…"),
      },
      {
        id: "ready", title: "Ready, all at once", ms: 2600,
        note: <>The review replaces Activity complete, with no intermediate states. No Done message.</>,
        frame: {
          page: "review",
          bar: { label: "Mark as reviewed", active: true },
          review: {
            meta: `Version ${VERSION}`, loaded: true, cancel: false, banner: true, body: "rows", tools: "ready",
            footer: { gauge: true, label: "Mark as reviewed", enabled: true },
          },
        },
      },
      {
        id: "failed", title: "Ending: couldn’t start", ms: 0, ending: true,
        note: <>The progress message gives way to the card’s shipped notice.</>,
        frame: { page: "activity", bar: { label: "Pull", count: 3 }, card: failedNotice("pull") },
      },
    ];
  }

  /* ——— Push ——— */
  const comparing: Prep = { text: `Comparing this computer with ${STORAGE}`, short: `Comparing with ${STORAGE}`, progress: null, steps: PUSH_STEPS, step: 0 };
  const loading: Prep = { text: "Loading sessions", short: "Loading sessions", progress: null, steps: PUSH_STEPS, step: 1 };
  const readyReview: ReviewSpec = {
    meta: null, loaded: true, cancel: false, banner: false, body: "rows", tools: "ready",
    footer: { gauge: true, label: "Push", enabled: true },
  };

  if (dir === "before") {
    return [
      {
        id: "click", title: "Click Push", ms: 4200,
        note: <>About 5 s of <b>Preparing…</b> with no readout at all. Discovery and the storage read are a single call, so the button has nothing to count.</>,
        frame: { page: "activity", bar: { label: "Preparing…", spin: true, disabled: true, count: 5 } },
      },
      {
        id: "mount", title: "The review mounts before its list", ms: 1100,
        note: <>The meter and 6/6 are real, because the defaults come from the comparison. But <b>Push is already enabled</b> over placeholder rows: it would publish sessions nobody has seen yet.</>,
        frame: {
          page: "review",
          bar: { label: "Push", active: true },
          review: { ...readyReview, body: "scaffold" },
        },
      },
      {
        id: "ready", title: "Ready", ms: 2600,
        note: <>Push never goes blank like Pull does, because it already has the comparison when the review opens.</>,
        frame: { page: "review", bar: { label: "Push", active: true }, review: readyReview },
      },
      {
        id: "gate", title: "Ending: storage has newer changes", ms: 0, ending: true,
        note: <>The shipped gate: the review never opens, and the storage card shows the reason with <b>Open Pull review</b>, after the same 5 s wait.</>,
        frame: { page: "activity", bar: { label: "Push", count: 5 }, card: requiresPullNotice() },
      },
    ];
  }

  if (dir === "a") {
    const preparing = (prep: Prep, known: boolean, label: string): Frame => ({
      page: "review",
      bar: { label, spin: true, active: true, disabled: true },
      review: {
        meta: null, loaded: false, cancel: true, banner: false, body: "prep", prep,
        bones: known ? PUSH_ROWS.length : undefined, tools: known ? "bone" : "none",
        footer: { gauge: false, label, enabled: false, spin: true },
      },
    });
    return [
      {
        id: "open", title: "Click Push: the card opens in place", ms: 4200,
        note: <>Push’s prepare is one backend call, so there is one step to name and never a checklist. <b>×</b> is <b>Cancel</b>.</>,
        frame: preparing(comparing, false, "Preparing…"),
      },
      {
        id: "sessions", title: "Compared: loading the list", ms: 1100,
        note: <>The defaults are already known here, but the meter and the count wait for the rows they measure. <b>Push waits too</b>, the same way Pull does.</>,
        frame: preparing(loading, true, "Loading sessions…"),
      },
      {
        id: "ready", title: "Ready, in place", ms: 2600,
        note: <>Rows, meter and count arrive together. Push enables once you can see what it will send.</>,
        frame: { page: "review", bar: { label: "Push", active: true }, review: readyReview },
      },
      {
        id: "gate", title: "Ending: storage has newer changes", ms: 0, ending: true,
        note: <>The same slot resolves into the storage card’s shipped gate notice. No review opens (gate-exit rule), and the wait before it looked like progress rather than a stall.</>,
        frame: { page: "activity", bar: { label: "Push", count: 5 }, card: requiresPullNotice() },
      },
      {
        id: "failed", title: "Ending: couldn’t start", ms: 0, ending: true,
        note: <>An unreachable storage turns the card into <b>Push couldn’t start</b> with Retry (or Reconnect for a refused credential).</>,
        frame: { page: "activity", bar: { label: "Push", count: 5 }, card: failedNotice("push") },
      },
    ];
  }

  const stay = (prep: Prep, label: string): Frame => ({
    page: "activity",
    bar: { label, spin: true, disabled: true },
    shell: progressNotice("push", prep),
  });
  return [
    {
      id: "click", title: "Click Push: a progress message under the bar", ms: 4200,
      note: <>One stage line and Cancel in the message zone. Activity stays on screen, locked.</>,
      frame: stay(comparing, "Preparing…"),
    },
    {
      id: "sessions", title: "Loading the list, behind the page", ms: 1100,
      note: <>The review is mounted hidden until its rows are in.</>,
      frame: stay(loading, "Preparing…"),
    },
    {
      id: "ready", title: "Ready, all at once", ms: 2600,
      note: <>Rows, meter, count and an enabled Push appear together.</>,
      frame: { page: "review", bar: { label: "Push", active: true }, review: readyReview },
    },
    {
      id: "gate", title: "Ending: storage has newer changes", ms: 0, ending: true,
      note: <>The message gives way to the card’s shipped gate notice.</>,
      frame: { page: "activity", bar: { label: "Push", count: 5 }, card: requiresPullNotice() },
    },
  ];
}

/* ——— rows, in the review's own markup ———————————————————————————— */

function SessionRow({ session, status, indicator, choice, selected, sizeLabel, chip }: {
  session: MockSession;
  status: ThreadStatusIconName;
  indicator: "local" | "storage" | "synced";
  choice: boolean;
  selected: boolean;
  sizeLabel: string | null;
  chip?: string;
}) {
  return (
    <article className={`sync-history-thread-card${choice ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}>
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {choice && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${session.title}`}>
              <input type="checkbox" checked={selected} readOnly aria-label={`${selected ? "Exclude" : "Include"} ${session.title}`} />
            </label>
          )}
          <span className={`sync-thread-sync-indicator ${indicator}`} tabIndex={0}>
            <ThreadStatusIcon status={status} size={18} />
          </span>
          <strong>{session.title}</strong>
        </div>
        <div className="sync-history-thread-actions">
          {chip && <span className="sync-history-branch-state history-only">{chip}</span>}
          <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label={`Open in ${PROVIDER_NAME[session.provider]}`}>
            <ProviderMark provider={session.provider} displayName={PROVIDER_NAME[session.provider]} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Session details">
            <span className="sync-history-thread-metric"><Icon name="message" size={12} /><b>{session.messages}</b></span>
            <span className="sync-history-thread-metric"><Icon name="tool" size={12} /><b>{session.tools}</b></span>
            <span className="sync-history-thread-metric"><Icon name="token" size={12} /><b>{session.tokens}</b></span>
            <span className="sync-history-thread-metric sync-history-thread-duration"><Icon name="clock" size={12} /><b>{session.length}</b></span>
            {sizeLabel && <span className="sync-history-thread-metric" data-tooltip={`${sizeLabel} · ${formatBytes(session.bytes)}`}><Icon name="drive" size={12} /><b>{formatBytes(session.bytes)}</b></span>}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <button type="button" className="btn btn-ghost sync-history-session-detail-action" aria-label="View session details">
              <Icon name="chevron-right" size={13} />
            </button>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end">
            <time>{session.when}</time>
          </span>
        </div>
      </div>
    </article>
  );
}

function StorageOnlyGroup({ rows, review }: { rows: MockSession[]; review: boolean }) {
  return (
    <section className="sync-history-storage-only" aria-labelledby="mk-storage-only">
      <div className="sync-history-section-heading">
        <h2 id="mk-storage-only"><Icon name="cloud" size={13} />Only in <span className="sync-history-storage-name">{STORAGE}</span></h2>
        <span className="sync-history-heading-count"><Icon name="message" size={12} />{rows.length}</span>
      </div>
      <div className="sync-history-thread-list flat">
        {rows.map((row) => (
          <SessionRow
            key={row.id}
            session={row}
            status="cloud-outstanding"
            indicator="storage"
            choice={review}
            selected={false}
            sizeLabel={review ? "Pull size" : null}
            chip={review ? "Add session" : undefined}
          />
        ))}
      </div>
    </section>
  );
}

/** The review list as it lands. */
function ReviewRows({ verb }: { verb: Verb }) {
  const [query, setQuery] = useState("");
  const rows = verb === "pull" ? PULL_ROWS : PUSH_ROWS;
  return (
    <div className="sync-history-page sync-history-embedded sync-history-selection-review">
      <section className="sync-history-content" aria-label="Sessions">
        <NameSearchField noun={sessionNoun} query={query} onChange={setQuery} label="Search sessions" matches={rows.length} total={rows.length} />
        {verb === "pull" ? <StorageOnlyGroup rows={PULL_ROWS} review /> : (
          <div className="sync-history-thread-list flat">
            {PUSH_ROWS.map((row) => (
              <SessionRow key={row.id} session={row} status="local-outstanding" indicator="local" choice selected sizeLabel="Push size" />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

const ProgressStyle = createContext<Progress>("caption");

const PROGRESS_LABEL: Record<Progress, string> = {
  caption: "Caption",
  quiet: "Placeholders only",
  header: "In the header",
  line: "Stage line (round 1)",
};

const PROGRESS_NOTE: Record<Progress, ReactNode> = {
  caption: <>The loading caption Activity and the Library already use, placed where the search field will land. Only its words change: <i>Reading the latest version from Attic R2…</i>, <i>Loading sessions…</i>, <i>Comparing with this computer…</i>. No numbers, no bar.</>,
  quiet: <>No words in the card, only placeholders. The footer button and the path-bar button already say <i>Preparing…</i>, then <i>Loading sessions…</i>, and that is all the wait says.</>,
  header: <>The words sit in the pinned header, where the meter will appear, and include Pull’s object count. The body is placeholders only.</>,
  line: <>Last round’s version, for comparison: a stage line in the search field’s slot, its underline as the progress track, and step dots.</>,
};

/** The stage line sits in the search field's slot, with the field's own
 *  geometry, and its underline is the progress track: when the list lands,
 *  the field takes the same 38px and the rule under it stops moving. Below
 *  it the row placeholders take their count from the plan (Pull) or the
 *  comparison (Push) once one is read, at the landed row's height. */
function PrepBones({ verb, prep, rows, grouped }: { verb: Verb; prep: Prep; rows: number; grouped: boolean }) {
  const progress = useContext(ProgressStyle);
  /* Grouped, the rows sit inside the storage-only box the way the landed
     rows do; alone, they are the scaffold's own boxed list. */
  const bones = (
    <div className={grouped ? "sync-history-thread-list flat mk-bones" : "loading-scaffold-list mk-bones"} aria-hidden="true">
      {Array.from({ length: rows }, (_, index) => [62, 44, 55, 48, 58][index % 5]).map((width, index) => (
        <div key={index} className="loading-scaffold-row">
          <div className="loading-scaffold-topline">
            <div className="loading-scaffold-title">
              <LoadingBone width={12} height={12} className="round quiet" />
              <LoadingBone width={`${width}%`} height={13} />
            </div>
            <div className="loading-scaffold-actions">
              <LoadingBone width={16} height={16} className="quiet" />
              <LoadingBone width={16} height={16} className="quiet" />
            </div>
          </div>
          <div className="loading-scaffold-chips">
            <LoadingBone width={54} height={9} className="pill quiet" />
            <LoadingBone width={38} height={9} className="pill quiet" />
            <LoadingBone width={46} height={9} className="pill quiet" />
          </div>
        </div>
      ))}
    </div>
  );
  return (
    <div className="sync-history-page sync-history-embedded sync-history-selection-review">
      <section className="sync-history-content" aria-label={prep.text}>
        {progress === "line" ? (
          <div className="sync-name-search mk-stage" role="status">
            <div className="sync-name-search-input">
              <span className="status-loader" />
              <b>{prep.text}…</b>
            </div>
            <div className="sync-name-search-results">
              {prep.readout && <span className="mk-prep-readout">{prep.readout}</span>}
              <span
                className="mk-stage-dots"
                role="img"
                aria-label={`Stage ${prep.step + 1} of ${prep.steps.length}`}
                title={prep.steps.map((step, index) => `${index < prep.step ? "✓" : index === prep.step ? "●" : "○"} ${step}`).join("   ")}
              >
                {prep.steps.map((step, index) => (
                  <i key={step} className={index < prep.step ? "done" : index === prep.step ? "now" : undefined} />
                ))}
              </span>
            </div>
            <span className={`mk-prep-track mk-stage-track${verb === "push" ? " push" : ""}${prep.progress === null ? " indeterminate" : ""}`} aria-hidden="true">
              <i style={prep.progress === null ? undefined : { width: `${prep.progress * 100}%` }} />
            </span>
          </div>
        ) : progress === "caption" ? (
          /* The shipped scaffold caption, word for word in its grammar
             ("Loading ⟨noun⟩…"), in the search field's slot. No numbers,
             no bar: only the words change from step to step. */
          <div className="sync-name-search mk-slot" role="status">
            <div className="loading-scaffold-caption"><span className="status-loader" /> {prep.text}…</div>
          </div>
        ) : (
          /* quiet and header: the field itself, in bone. The body says
             nothing; the buttons (and, for header, the header) do. */
          <div className="sync-name-search mk-slot" aria-hidden="true">
            <div className="sync-name-search-input">
              <LoadingBone width={14} height={14} className="round quiet" />
              <LoadingBone width={130} height={11} className="pill quiet" />
            </div>
            <div className="sync-name-search-results"><LoadingBone width={44} height={9} className="pill quiet" /></div>
          </div>
        )}
        {grouped ? (
          <section className="sync-history-storage-only" aria-hidden="true">
            <div className="sync-history-section-heading">
              <LoadingBone width={150} height={12} className="pill quiet" />
              <LoadingBone width={30} height={12} className="pill quiet" />
            </div>
            {bones}
          </section>
        ) : bones}
      </section>
    </div>
  );
}

/* ——— the review panel ——————————————————————————————————————————— */

function PullToolsRow({ state }: { state: ReviewSpec["tools"] }) {
  if (state === "none") return null;
  return (
    <details className="sync-sync-review-other">
      <summary>
        <span><Icon name="settings" size={15} /><strong>Agent setup &amp; tools</strong></span>
        {state === "bone"
          ? <span className="mk-tools-checking"><LoadingBone width={64} height={10} className="pill quiet" /><Icon name="chevron-right" size={13} /></span>
          : <span>0 selected<Icon name="chevron-right" size={13} /></span>}
      </summary>
    </details>
  );
}

function Review({ verb, spec }: { verb: Verb; spec: ReviewSpec }) {
  const pull = verb === "pull";
  const progress = useContext(ProgressStyle);
  const budget = pull ? PULL_BUDGET : PUSH_BUDGET;
  const breakdown = pull ? PULL_BREAKDOWN : PUSH_BREAKDOWN;
  const pending = pull ? PULL_PENDING : PUSH_PENDING;
  const banners: NoticeItem[] = spec.banner ? [{
    key: "stored-without-redaction",
    severity: "warning",
    title: "Stored without credential redaction",
    message: "This review includes sessions that can contain API keys, tokens, and passwords.",
  }] : [];
  return (
    <section
      className={`sync-inline-action-review ${pull ? "sync-inline-pull-review" : "sync-inline-push-review"} sync-sync-review-workspace`}
      aria-label={pull ? "Pull review" : "Push review"}
      aria-busy={!spec.loaded || undefined}
    >
      <header className={`sync-inline-action-header ${pull ? "sync-inline-pull-header" : "sync-push-resource-header"} sync-sync-review-header`}>
        <div className="sync-inline-action-heading">
          <div className="sync-sync-review-title">
            <h2>{pull ? "Pull review" : "Push review"}</h2>
            {pull
              ? spec.meta && <span className="sync-pull-review-meta">{spec.meta}</span>
              : <span className="sync-settings-flag">Credential redaction: On</span>}
          </div>
        </div>
        {spec.loaded && <ReviewBudgetGlance budget={budget} verb={verb} breakdown={breakdown} />}
        {!spec.loaded && progress === "header" && spec.prep && (
          /* Where the meter will land: the header is pinned, and the meter
             takes this exact slot when the list is in. */
          <span className="mk-head-status" role="status"><span className="status-loader" />{spec.prep.short}…</span>
        )}
        {spec.loaded ? (
          <SyncReviewSelectionBar
            count={pull ? 0 : pending}
            total={pending}
            recommendedCount={pull ? PULL_ROWS.length : PUSH_ROWS.length}
            recommendedTitle="Use recommended selection"
            recommendedDisabled={!pull}
            onRecommended={() => undefined}
            clearDisabled={pull}
            onClear={() => undefined}
            close={{ ariaLabel: `Close ${verb} review`, onClose: () => undefined }}
          />
        ) : (
          <div className="sync-push-resource-actions">
            <button
              type="button"
              className="btn btn-ghost sync-inline-action-close"
              title={spec.cancel ? "Cancel — nothing has changed" : undefined}
              aria-label={spec.cancel ? `Cancel preparing the ${verb === "pull" ? "Pull" : "Push"} review` : `Close ${verb} review`}
            >
              <Icon name="x" size={15} />
            </button>
          </div>
        )}
      </header>

      <div className={`${pull ? "sync-pull-review-content" : "sync-push-resource-content"} sync-inline-action-content sync-sync-review-content`}>
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {banners.length > 0 && (
            <div className="sync-review-banners">
              <NoticeZone items={banners} subject={`notes about this ${pull ? "Pull" : "Push"} review`} timing="standing" label={`Notes about this ${pull ? "Pull" : "Push"} review`} />
            </div>
          )}
          {spec.body === "rows" && <ReviewRows verb={verb} />}
          {spec.body === "prep" && spec.prep && <PrepBones verb={verb} prep={spec.prep} rows={spec.bones ?? 3} grouped={pull && spec.bones !== undefined} />}
          {spec.body === "scaffold" && (
            <div className="sync-history-page sync-history-embedded sync-history-selection-review">
              <section className="sync-history-content" aria-label="Sessions">
                <LoadingScaffold shape="rows" label="Loading cloud sessions" />
              </section>
            </div>
          )}
          {spec.body === "blank" && (
            <div className="sync-history-page sync-history-embedded sync-history-selection-review">
              <section className="sync-history-content" aria-label="Sessions" />
            </div>
          )}
          {pull ? <PullToolsRow state={spec.tools} /> : spec.tools === "ready" ? (
            <ResourceInventory resources={REQUIREMENTS} selected={new Set(REQUIREMENTS.map((item) => item.resource_id))} defaultCollapsed onToggle={() => undefined} />
          ) : spec.tools === "bone" ? (
            <LoadingBone width="100%" height={44} className="quiet" />
          ) : null}
        </div>
      </div>

      <footer className={`${pull ? "sync-pull-apply-bar " : "sync-push-resource-footer "}sync-inline-action-footer sync-sync-review-footer`}>
        {spec.footer.gauge && <ReviewBudgetGauge budget={budget} verb={verb} breakdown={breakdown} />}
        <button
          type="button"
          className="btn btn-primary sync-pull-apply-button"
          disabled={!spec.footer.enabled}
          title={spec.footer.title}
        >
          {spec.footer.spin
            ? <Icon name="refresh" size={16} className="icon-spin" />
            : <ThreadStatusIcon status={pull ? "cloud-outstanding" : "local-outstanding"} size={16} />}
          {spec.footer.label}
        </button>
      </footer>
    </section>
  );
}

/* ——— the workspace: path bar, message zone, page ————————————————— */

function PathActions({ verb, bar }: { verb: Verb; bar: Frame["bar"] }) {
  const button = (kind: Verb) => {
    const mine = kind === verb;
    const icon = kind === "pull" ? "download" : "upload";
    const idleCount = kind === "pull" ? 3 : 5;
    const label = mine ? bar.label : kind === "pull" ? "Pull" : "Push";
    const count = mine ? bar.count : idleCount;
    return (
      <button
        type="button"
        className={`sync-path-sync${mine && bar.active ? " active" : ""}`}
        disabled={mine ? bar.disabled : !!(bar.spin || bar.active)}
        aria-expanded={mine ? !!bar.active : false}
      >
        <Icon name={mine && bar.spin ? "refresh" : icon} size={14} className={mine && bar.spin ? "icon-spin" : undefined} />
        {mine && bar.readout
          ? <span className="sync-btn-readout">{bar.readout}</span>
          : <span className="sync-path-sync-label">{label}</span>}
        {count != null && count > 0 && !(mine && (bar.spin || bar.active)) && <span className="sync-path-count" aria-hidden="true">{count}</span>}
      </button>
    );
  };
  return (
    <>
      <button type="button" className="sync-path-icon-action" disabled={!!(bar.spin || bar.active)} aria-label="Refresh activity" title="Refresh activity">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        {button("pull")}
        {button("push")}
      </div>
    </>
  );
}

function ActivityBackdrop({ verb }: { verb: Verb }) {
  return (
    <div id="project-history-panel" className="sync-project-tab-panel">
      <div className="sync-history-page">
        <section className="sync-history-content" aria-label="Project activity">
          {verb === "pull" && <StorageOnlyGroup rows={PULL_ROWS} review={false} />}
          <div className="sync-history-thread-list flat">
            {verb === "push" && PUSH_ROWS.slice(0, 3).map((row) => (
              <SessionRow key={row.id} session={row} status="local-outstanding" indicator="local" choice={false} selected={false} sizeLabel={null} />
            ))}
            {SYNCED_ROWS.map((row) => (
              <SessionRow key={row.id} session={row} status="synced" indicator="synced" choice={false} selected={false} sizeLabel={null} />
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}

function Workspace({ verb, frame }: { verb: Verb; frame: Frame }) {
  const zone = (
    <NoticeZone
      items={frame.shell ? [frame.shell] : []}
      subject="messages from Mallard"
      className="sync-backend-banner"
      timing="progress"
    />
  );
  const reviewOpen = frame.page === "review" && !!frame.review;
  return (
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar
        crumbs={[
          { id: "projects", label: "Projects", tone: "root" },
          { id: "project", label: PROJECT, icon: "folder" },
          { id: "storage", label: STORAGE, icon: "storage-r2" },
          { id: "section", label: "Git history", tone: "section" },
        ]}
        actions={<PathActions verb={verb} bar={frame.bar} />}
        ariaLabel={`${PROJECT} path`}
      />
      <main className={`sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page${reviewOpen ? " sync-sync-review-page" : ""}`}>
        <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
          {(reviewOpen || frame.card) && (
            <div className="profile-links-list sync-combined-settings-panel">
              <article className="profile-link-card sync-project-link-card">
                <div className="profile-link-connections">
                  <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                    <div className="project-profile-storage-list">
                      <div className={`storage-link-block selected${reviewOpen ? " sync-review-open" : ""}`}>
                        {frame.card && (
                          <NoticeZone items={[frame.card]} subject="reasons this storage is held" className="sync-storage-notice notice-strip" timing="blocking" />
                        )}
                        {reviewOpen && (
                          <div className="sync-storage-inline-review" tabIndex={-1}>
                            <Review verb={verb} spec={frame.review!} />
                          </div>
                        )}
                      </div>
                    </div>
                  </section>
                </div>
              </article>
            </div>
          )}
        </section>
        {!reviewOpen && <ActivityBackdrop verb={verb} />}
      </main>
    </PathBarBelow.Provider>
  );
}

/* ——— the page ——————————————————————————————————————————————————— */

const DIR_LABEL: Record<Dir, string> = { before: "Before (shipped)", a: "A · open on click", b: "B · arrive whole" };

function Live({ verb, dir }: { verb: Verb; dir: Dir }) {
  const path = scenes(verb, dir).filter((scene) => !scene.ending);
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const timer = useRef<number | null>(null);
  useEffect(() => { setIndex(0); setPlaying(false); }, [verb, dir]);
  useEffect(() => {
    if (!playing) return;
    if (index >= path.length - 1) { setPlaying(false); return; }
    timer.current = window.setTimeout(() => setIndex((current) => current + 1), path[index].ms);
    return () => { if (timer.current) window.clearTimeout(timer.current); };
  }, [playing, index, path]);
  const scene = path[Math.min(index, path.length - 1)];
  const elapsed = path.slice(0, index).reduce((sum, item) => sum + item.ms, 0);
  return (
    <div>
      <div className="spec-live-bar">
        <button type="button" className="spec-seg" aria-pressed={playing} onClick={() => {
          if (index >= path.length - 1) setIndex(0);
          setPlaying(!playing);
        }}>{playing ? "Pause" : "▶ Play"}</button>
        <button type="button" className="spec-seg" disabled={index === 0} onClick={() => { setPlaying(false); setIndex(index - 1); }}>‹ Back</button>
        <button type="button" className="spec-seg" disabled={index >= path.length - 1} onClick={() => { setPlaying(false); setIndex(index + 1); }}>Step ›</button>
        <span className="spec-dots" aria-hidden="true">{path.map((item, dot) => <i key={item.id} className={dot <= index ? "on" : undefined} />)}</span>
        <span className="spec-clock">t = {(elapsed / 1000).toFixed(1)} s</span>
      </div>
      <h2 className="spec-col-title"><small>{index + 1}</small>{scene.title}</h2>
      <p className="spec-col-note">{scene.note}</p>
      <div className="spec-frame"><Workspace key={`${verb}-${dir}-${scene.id}`} verb={verb} frame={scene.frame} /></div>
    </div>
  );
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ verb: next.verb, dir: next.dir, view: next.view, progress: next.progress, theme: next.theme });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shown = scenes(knobs.verb, knobs.dir);
  /* Compare is A's view: one moment of the wait in every progress style. */
  const view: View = knobs.view === "compare" && knobs.dir !== "a" ? "strip" : knobs.view;
  const compareScene = shown.find((scene) => scene.id === (knobs.verb === "pull" ? "fetch" : "open")) ?? shown[0];
  return (
    <div className="spec">
      <h1 className="spec-title">Push and Pull: from the click to a review you can act on</h1>
      <p className="spec-sub">
        Each frame is one moment of the wait after you click Push or Pull, drawn with the shipped components on the shipped CSS.
        <b> Before</b> is what ships today. <b>A</b> opens the storage card at the click and fills it in place. <b>B</b> keeps
        Activity on screen and shows the review only once it is complete. The stages are only the ones Mallard can actually
        report today.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Verb</span>{seg("verb", "pull", "Pull")}{seg("verb", "push", "Push")}</div>
        <div className="spec-group"><span>Direction</span>{seg("dir", "before", DIR_LABEL.before)}{seg("dir", "a", DIR_LABEL.a)}{seg("dir", "b", DIR_LABEL.b)}</div>
        {knobs.dir === "a" && (
          <div className="spec-group">
            <span>Progress</span>
            {(["caption", "quiet", "header", "line"] as const).map((style) => seg("progress", style, PROGRESS_LABEL[style]))}
          </div>
        )}
        <div className="spec-group">
          <span>View</span>
          {seg("view", "strip", "Every moment")}{seg("view", "live", "Live")}
          {knobs.dir === "a" && seg("view", "compare", "Side by side")}
        </div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      {knobs.dir === "before" ? (
        <div className="spec-panel">
          <strong>What goes wrong today</strong>
          <ol>
            <li><b>A dead wait.</b> For several seconds (about 5 s for Push) the only sign is a spinning button. Pull’s byte count is squeezed into that button, and there is no Cancel.</li>
            <li><b>The review opens before its list.</b> Meters at 0 B and a 0/4 count describe rows that aren’t on screen yet, and a green <i>Done · ready</i> message stacks above it.</li>
            <li><b>Pull goes blank</b> (your screenshot). The loading placeholder covers only the first read, not the comparison.</li>
            <li><b>A greyed primary with no visible reason.</b> “Mark as reviewed” is disabled while loading, and the reason, <i>Loading storage changes…</i>, is only in a tooltip.</li>
            <li><b>Push is live over placeholder rows.</b> The defaults are ticked on rows the user hasn’t seen yet.</li>
            <li><b>Things move while loading.</b> The tools row jumps up into the blank, then back down when the rows land.</li>
          </ol>
        </div>
      ) : (
        <div className="spec-panel">
          <strong>{knobs.dir === "a" ? "A · the card opens in place" : "B · Activity stays, the review arrives whole"}</strong>
          <ul>
            {knobs.dir === "a" ? (
              <>
                <li>The wait happens where the result will appear: the storage card, which already hosts both the review and its gate notices.</li>
                <li>The card opens only if preparing takes longer than 150 ms. It then resolves in place into the <b>review</b>, the <b>gate notice</b>, <b>nothing to pull</b>, <b>couldn’t start</b>, or it closes with the shipped <i>files unchanged</i> message. A review never opens and then closes.</li>
                <li>Until the list lands there are no meters, no count and no ✓. The primary and the path-bar button show one shared label: <i>Preparing…</i>, then <i>Loading sessions…</i>. A hidden status line announces “ready” in place of the Done message.</li>
                <li>× is <b>Cancel</b> while preparing only reads. That needs a lock for this review that is separate from the app-wide <code>busy</code> (which today stays held until the backend call returns). Cancel is withdrawn once Pull’s auto-accept starts writing.</li>
                <li>Placeholder rows hold the list’s height the whole time, so the rows replace them without anything moving.</li>
                <li><b>Progress: {PROGRESS_LABEL[knobs.progress]}.</b> {PROGRESS_NOTE[knobs.progress]} <i>Side by side</i> shows all four at one moment.</li>
              </>
            ) : (
              <>
                <li>The stage and Cancel sit in the message zone under the path bar. The card and the page stay as they are until the review is complete.</li>
                <li>The review is mounted hidden and shown once its list is ready. No intermediate state is ever on screen.</li>
                <li>Cost: two activity loads run at once (the visible page and the hidden review), which needs a new way of rendering the review. Activity stays on screen but locked, and the progress is far from where the review will appear.</li>
              </>
            )}
          </ul>
        </div>
      )}

      {view === "compare" ? (
        <div className="spec-row">
          {(["caption", "quiet", "header", "line"] as const).map((style) => (
            <div key={style}>
              <h2 className="spec-col-title">{PROGRESS_LABEL[style]}{style === "line" && <em>round 1</em>}</h2>
              <p className="spec-col-note">{PROGRESS_NOTE[style]}</p>
              <ProgressStyle.Provider value={style}>
                <div className="spec-frame"><Workspace verb={knobs.verb} frame={compareScene.frame} /></div>
              </ProgressStyle.Provider>
            </div>
          ))}
        </div>
      ) : view === "live" ? (
        <ProgressStyle.Provider value={knobs.progress}>
          <Live verb={knobs.verb} dir={knobs.dir} />
        </ProgressStyle.Provider>
      ) : (
        <div className="spec-row">
          {shown.map((scene, index) => (
            <div key={`${knobs.verb}-${knobs.dir}-${scene.id}`}>
              <h2 className="spec-col-title">
                <small>{index + 1}</small>{scene.title}
                {scene.ending && <em>ending</em>}
              </h2>
              <p className="spec-col-note">{scene.note}</p>
              <ProgressStyle.Provider value={knobs.progress}>
                <div className="spec-frame"><Workspace verb={knobs.verb} frame={scene.frame} /></div>
              </ProgressStyle.Provider>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
