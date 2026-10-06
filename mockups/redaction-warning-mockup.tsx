import { StrictMode, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import Notice, { NoticeChip, NoticeZone, SEVERITY_ICON, type NoticeItem } from "../src/components/notice/Notice";
import { CredentialRedactionOffNote, PushApplyButton } from "../src/components/project-sync/PushResourceWorkspace";
import { projectReviewTabs } from "../src/components/project-sync/reviewSteps";
import { ReviewBudgetGauge, ReviewBudgetGlance } from "../src/components/sync-review/ReviewBudgetMeter";
import { PathBarPopover } from "../src/components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "../src/components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatBytes } from "../src/format";
import "../src/App.css";

/**
 * Mockup: a quieter way to say that credential redaction is off.
 *
 * Owner picked C on 2026-10-05 and it shipped: the C frame mounts the shipped
 * `CredentialRedactionOffNote`. The Before frame re-enacts the removed box
 * with a local copy of `CREDENTIAL_REDACTION_OFF_NOTICE`.
 *
 * Shipped, both Push reviews say it twice while Settings › Privacy ›
 * Credential redaction is off: the header's `.sync-settings-flag` reads
 * "Credential redaction: Off", and `CREDENTIAL_REDACTION_OFF_NOTICE` adds a
 * closable warning box at the top of the review scroll. The setting is off by
 * default (`app_settings.rs`), so the box is on every review for every user,
 * and closing it lasts only until the review opens again.
 *
 *   Before  what ships
 *   A       the header pill carries it: the warning chip, sentence in its title
 *   B       A as a button: the pill opens a small panel with the sentence and
 *           the way to the setting (the session page's notes chip)
 *   C       the neutral pill stays; one muted line by Push states the consequence
 *   D       the box becomes a strip flush under the tabs
 *   E       the review says only the neutral pill; the sentence lives at the switch
 *   F       the box once, then remembered closed until the setting changes
 *
 * `npm run dev`, then /mockups/redaction-warning-mockup.html
 * (`?scenario=off|held|failed|on&review=project|session&width=w970|w684|w414&theme=light|dark&only=before|a|…|f&open=1&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type ScenarioId = "off" | "held" | "failed" | "on";
type ReviewKind = "project" | "session";
type Dir = "before" | "a" | "b" | "c" | "d" | "e" | "f";
type Width = "w970" | "w684" | "w414";
interface Knobs {
  scenario: ScenarioId;
  review: ReviewKind;
  width: Width;
  theme: "light" | "dark";
  only: Dir | "all";
  /** B's panel starts open. */
  open: "0" | "1";
  shot: "on" | "off";
}

const SCENARIOS: readonly ScenarioId[] = ["off", "held", "failed", "on"];
const DIRS: readonly Dir[] = ["before", "a", "b", "c", "d", "e", "f"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scenario: pick("scenario", SCENARIOS, "off"),
    review: pick("review", ["project", "session"] as const, "project"),
    width: pick("width", ["w970", "w684", "w414"] as const, "w970"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    open: pick("open", ["0", "1"] as const, "0"),
    shot: pick("shot", ["on", "off"] as const, "off"),
  };
}

const SCENARIO_LABEL: Record<ScenarioId, string> = {
  off: "Off (the screenshot)",
  held: "Off + sessions held for Pull",
  failed: "Off + comparison failed",
  on: "Redaction on",
};
const WIDTH_LABEL: Record<Width, string> = {
  w970: "970 px (screenshot)",
  w684: "684 px (1100 window)",
  w414: "414 px (800 window)",
};
const REVIEW_WIDTH: Record<Width, number> = { w970: 970, w684: 684, w414: 414 };

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (shipped)",
  a: "A · The pill carries it",
  b: "B · The pill opens a note",
  c: "C · A line by Push",
  d: "D · A strip under the tabs",
  e: "E · Said at the switch only",
  f: "F · Once, until the setting changes",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>The header pill says <i>Credential redaction: Off</i>, then a warning box says it again under the tabs, on every review. Its × lasts until the review is opened again. Next to a Pull hold or a failed comparison it becomes one line in a shared box, and that box&rsquo;s count goes up by one.</>,
  a: <>No box. While redaction is off, the header pill becomes the app&rsquo;s warning chip (<code>NoticeChip</code>: tint, glyph, two words). Chips are labels, so it has no ×. The sentence goes in its title and, for screen readers, in a visually hidden span. The face is short enough that <i>off</i> is never the word that gets cut; below 480 px the review keeps only the glyph. A title tooltip can&rsquo;t be reached by keyboard or touch, so a sighted keyboard user only sees the two words.</>,
  b: <>A, but the pill is a button: the session page&rsquo;s notes chip (outlined, warning ink, opens a <code>PathBarPopover</code> panel). The panel says what <i>off</i> means, as a fact rather than a warning, and offers <b>Open Privacy settings</b>. Keyboard and touch can reach the sentence, and the warning can be acted on. The pill becomes a control, so it gains <code>aria-expanded</code>. Below 480 px it keeps only the glyph, and its accessible name still says what it is.</>,
  c: <>The header pill stays as shipped, a neutral fact. The consequence moves to the commit: one muted line in the footer&rsquo;s note row, over the Push button, with no box and no ×. It costs the footer one line on every review while redaction is off.</>,
  d: <>Still a message, but flush under the tab bar: the <code>.notice-strip</code> tier (one line, top edge only, no corners). It keeps its × and comes back when the review reopens. It leaves the shared box, so a Pull hold and this strip become two separate elements.</>,
  e: <>The review keeps only the neutral pill. Settings › Privacy says the consequence once, under the switch, where the choice is made (second frame). It is the quietest option, and nothing on the review points back to Settings.</>,
  f: <>The shipped box, but its × is remembered (machine-local) until the setting changes, and from then on only the neutral pill shows. <b>This breaks the owner rule from 2026-09-24</b> that a closed message comes back when the screen reopens. Use the switch above the frame to see a later review.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const KB = 1024;
const MB = 1024 * KB;
const LIMIT = 512 * MB;
const RESERVED = 21 * KB;

interface MockSession { id: string; title: string; bytes: number; stamp: string }
const SESSIONS: MockSession[] = [
  { id: "s1", title: "Fix the login redirect after OAuth", bytes: 1.3 * MB, stamp: "Today 10:12" },
  { id: "s2", title: "Load the sample CSV into the preview", bytes: 420 * KB, stamp: "Today 09:40" },
  { id: "s3", title: "Tidy the README and the setup notes", bytes: 96 * KB, stamp: "Yesterday 18:03" },
  { id: "s4", title: "Port the retry helper to the new client", bytes: 2.2 * MB, stamp: "Yesterday 15:21" },
];

const REDACTION_SENTENCE = "Exported content can include API keys, tokens, and passwords.";
/** The box that shipped before C, kept here for the Before frame. */
const CREDENTIAL_REDACTION_OFF_NOTICE: NoticeItem = { key: "credential-redaction", severity: "warning", message: REDACTION_SENTENCE };
/* B's panel states the setting as a fact (Windows UX guidance), not as a warning. */
const PANEL_SENTENCE = "Sessions push as written, so they can include API keys, tokens, and passwords.";
const PILL_TITLE = `Credential redaction is off. ${REDACTION_SENTENCE} Change it in Settings › Privacy.`;

/** The other plan-derived banners the zone can hold, worded as shipped. */
function otherBanners(scenario: ScenarioId): NoticeItem[] {
  if (scenario === "held") {
    return [{
      key: "pull-holds",
      severity: "warning",
      mark: <ThreadStatusIcon status="divergent" size={16} />,
      title: "2 conversations held for Pull",
      message: "Storage has newer versions. Selected threads still push.",
      action: <button type="button" className="btn btn-ghost">Open Pull review</button>,
    }];
  }
  if (scenario === "failed") {
    return [{
      key: "comparison-failed",
      severity: "danger",
      title: "Conversations couldn't be compared, so nothing can be selected",
      message: "Storage returned an object Mallard cannot read.",
    }];
  }
  return [];
}

/* ——— the forms ———————————————————————————————————————————————————— */

/** Shipped: a stated fact in the header, both states. */
function NeutralFlag({ off }: { off: boolean }) {
  return <span className="sync-settings-flag">Credential redaction: {off ? "Off" : "On"}</span>;
}

/** A: the app's warning chip, a label with its sentence in the title. */
function WarningChip() {
  return (
    <>
      <span title={PILL_TITLE} className="mk-pill-wrap">
        <NoticeChip severity="warning" className="mk-redaction-chip"><span className="mk-pill-words">Redaction off</span></NoticeChip>
      </span>
      <span className="sync-visually-hidden">Warning: credential redaction is off. {REDACTION_SENTENCE}</span>
    </>
  );
}

/** B: the session page's notes chip, opening a panel. */
function WarningPopover({ startOpen }: { startOpen: boolean }) {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!startOpen) return;
    const timer = window.setTimeout(() => ref.current?.querySelector<HTMLButtonElement>("button")?.click(), 120);
    return () => window.clearTimeout(timer);
  }, [startOpen]);
  return (
    <span ref={ref} className="mk-pill-wrap">
      <PathBarPopover
        id="redaction"
        label="Credential redaction"
        className="session-detail-chip session-detail-notes-chip warning mk-redaction-button"
        ariaLabel="Credential redaction is off"
        tooltip="Credential redaction is off"
        face={<><Icon name={SEVERITY_ICON.warning} size={12} /><span className="mk-pill-words">Redaction off</span></>}
      >
        {(close) => (
          <div className="session-detail-notes mk-redaction-panel">
            <p className="session-detail-notes-title">Credential redaction is off</p>
            <p className="session-detail-note warning">
              <Icon name={SEVERITY_ICON.warning} size={12} />
              <span>{PANEL_SENTENCE}</span>
            </p>
            <p className="mk-panel-foot">
              <button type="button" className="btn-link" onClick={() => close(true)}>Open Privacy settings</button>
            </p>
          </div>
        )}
      </PathBarPopover>
      <span className="sync-visually-hidden">Credential redaction is off. {PANEL_SENTENCE}</span>
    </span>
  );
}

/** The header's redaction mark, per direction. */
function HeaderMark({ dir, off, startOpen }: { dir: Dir; off: boolean; startOpen: boolean }) {
  if (!off) return <NeutralFlag off={false} />;
  if (dir === "a") return <WarningChip />;
  if (dir === "b") return <WarningPopover startOpen={startOpen} />;
  return <NeutralFlag off />;
}

/** Settings › Privacy, as E would word the row. */
function SettingsRowSpecimen({ dir }: { dir: Dir }) {
  return (
    <div className="sync-settings-card mk-settings-card">
      <div className="sync-settings-row">
        <div className="sync-settings-copy">
          <strong>Credential redaction</strong>
          <p>
            Hide detected credentials in pushed sessions.
            {dir === "e" && <> <span className="mk-settings-consequence">While off, pushed sessions can include API keys, tokens, and passwords.</span></>}
            {" "}Changing it re-uploads synced sessions on the next Push. Earlier copies stay in storage history.
          </p>
        </div>
        <div className="sync-settings-control">
          <label className="sync-settings-toggle">
            <input type="checkbox" role="switch" className="sync-settings-switch" checked={false} readOnly aria-label="Credential redaction" />
            <span className="sync-settings-state" aria-hidden="true">Off</span>
          </label>
        </div>
      </div>
    </div>
  );
}

/* ——— one review ————————————————————————————————————————————————— */

function SessionsStandIn({ sessions, selected, onToggle }: {
  sessions: MockSession[];
  selected: ReadonlySet<string>;
  onToggle: (id: string) => void;
}) {
  return (
    <div className="mk-session-list" aria-label="Sessions (stand-in)">
      {sessions.map((session) => (
        <article key={session.id} className="sync-history-thread-card catalog-card" data-row>
          <div className="sync-history-thread-topline">
            <div className="sync-history-thread-title">
              <label className="sync-sync-row-choice">
                <input type="checkbox" checked={selected.has(session.id)} onChange={() => onToggle(session.id)} aria-label={`Include ${session.title}`} />
              </label>
              <span className="sync-thread-sync-indicator local">
                <ThreadStatusIcon status="local-outstanding" size={18} />
              </span>
              <strong>{session.title}</strong>
            </div>
          </div>
          <div className="sync-history-session-details">
            <div className="sync-history-session-summary">
              <div className="sync-history-thread-meta">
                <span className="sync-history-thread-metric"><b>{formatBytes(session.bytes)}</b></span>
              </div>
              <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end">{session.stamp}</span>
            </div>
          </div>
        </article>
      ))}
    </div>
  );
}

function Review({ dir, scenario, review, startOpen, later }: {
  dir: Dir;
  scenario: ScenarioId;
  review: ReviewKind;
  startOpen: boolean;
  /** F: a later review, after the box was closed once. */
  later: boolean;
}) {
  const off = scenario !== "on";
  const sessions = review === "session" ? SESSIONS.slice(0, 1) : SESSIONS;
  const [selected, setSelected] = useState<ReadonlySet<string>>(() => new Set(sessions.map((session) => session.id)));
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const ticked = sessions.filter((session) => selected.has(session.id));
  const budget = { total: RESERVED + ticked.reduce((sum, session) => sum + session.bytes, 0), limit: LIMIT };
  const breakdown = `Sessions ${formatBytes(budget.total - RESERVED)} · Requirements ${formatBytes(RESERVED)} · Project files 0 B`;

  /* The zone: the redaction item only where this direction keeps it boxed. */
  const boxed = off && (dir === "before" || (dir === "f" && !later));
  const banners: NoticeItem[] = [
    ...(review === "project" ? otherBanners(scenario) : []),
    ...(boxed ? [CREDENTIAL_REDACTION_OFF_NOTICE] : []),
  ];
  const strip = off && dir === "d";
  const footerLine = off && dir === "c";

  return (
    <section
      className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace"
      aria-label="Push review"
    >
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <HeaderMark dir={dir} off={off} startOpen={startOpen} />
        </div>
        {review === "project" ? (
          <>
            <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown} />
            <SyncReviewSelectionBar
              count={ticked.length}
              total={sessions.length}
              recommendedCount={sessions.length}
              recommendedTitle="Use the recommended session selection"
              recommendedDisabled={ticked.length === sessions.length}
              onRecommended={() => setSelected(new Set(sessions.map((session) => session.id)))}
              clearDisabled={ticked.length === 0}
              onClear={() => setSelected(new Set())}
              close={{ ariaLabel: "Close push review", onClose: () => undefined }}
            />
          </>
        ) : (
          <div className="sync-push-resource-actions">
            <button type="button" className="btn btn-ghost sync-inline-action-close" aria-label="Close push review">
              <Icon name="x" size={15} />
            </button>
          </div>
        )}
      </header>
      {review === "project" && (
        <SyncReviewTabs
          tabs={projectReviewTabs(["history", "project_files"], { history: ticked.length, project_files: 0 }, new Set(), "Sessions")}
          activeId="history"
          onChange={() => undefined}
        />
      )}
      {strip && <StripNotice />}
      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {banners.length > 0 && (
            <div className="sync-review-banners">
              <NoticeZone
                items={banners}
                subject="notes about this Push review"
                timing="standing"
                label="Notes about this Push review"
              />
            </div>
          )}
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <SessionsStandIn sessions={sessions} selected={selected} onToggle={toggle} />
          </div>
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {footerLine && <CredentialRedactionOffNote />}
        {review === "project" && (
          <button type="button" className="btn btn-ghost" title="Review project files next">
            Project files
            <Icon name="chevron-right" size={14} />
          </button>
        )}
        <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} />
        <PushApplyButton busy={false} disabledReason={ticked.length === 0 ? "Nothing is selected" : null} onPush={() => undefined} />
      </footer>
    </section>
  );
}

/** D: the shipped strip tier, flush under the tab bar, with the shipped ×. */
function StripNotice() {
  return (
    <Notice severity="warning" timing="standing" label="Notes about this Push review" className="notice-strip mk-redaction-strip">
      {REDACTION_SENTENCE}
    </Notice>
  );
}

/* ——— measurement ——————————————————————————————————————————————————— */

type Rgba = [number, number, number, number];
function parseColor(value: string): Rgba | null {
  const match = value.match(/rgba?\(([^)]+)\)/);
  if (!match) return null;
  const parts = match[1].split(/[\s,/]+/).filter(Boolean).map(Number);
  return [parts[0], parts[1], parts[2], parts.length > 3 ? parts[3] : 1];
}
function over(top: Rgba, bottom: Rgba): Rgba {
  const a = top[3] + bottom[3] * (1 - top[3]);
  if (a === 0) return [0, 0, 0, 0];
  const mix = (i: number) => (top[i] * top[3] + bottom[i] * bottom[3] * (1 - top[3])) / a;
  return [mix(0), mix(1), mix(2), a];
}
/** The opaque colour an element is painted on: its own fill laid over every
 * ancestor's, down to the first opaque one. */
function paintedBackground(element: Element): Rgba {
  const stack: Rgba[] = [];
  for (let node: Element | null = element; node; node = node.parentElement) {
    const fill = parseColor(getComputedStyle(node).backgroundColor);
    if (fill && fill[3] > 0) {
      stack.push(fill);
      if (fill[3] >= 1) break;
    }
  }
  return stack.reduceRight<Rgba>((bottom, top) => over(top, bottom), [255, 255, 255, 1]);
}
function luminance([r, g, b]: Rgba) {
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}
function contrast(element: Element): number {
  const ink = parseColor(getComputedStyle(element).color) ?? [0, 0, 0, 1];
  const paper = paintedBackground(element);
  const [hi, lo] = [luminance(over(ink, paper)), luminance(paper)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

interface Reading { rowTop: number | null; pill: string | null; cut: boolean; ink: number | null; footer: number }
function readFrame(frame: HTMLElement): Reading {
  const section = frame.querySelector("section");
  const row = frame.querySelector("[data-row]");
  const pill = frame.querySelector<HTMLElement>(".sync-sync-review-title > :not(h2):not(.sync-visually-hidden)");
  const face = pill?.matches(".mk-pill-wrap") ? pill.querySelector<HTMLElement>(".notice-chip, button") : pill;
  /* The zone counts only while the redaction sentence is in it. */
  const boxed = [...frame.querySelectorAll(".sync-review-banners .notice")].find((box) => box.textContent?.includes("API keys"));
  const warningInk = frame.querySelector(".mk-redaction-chip, .mk-redaction-button, .redaction-off, .mk-redaction-strip .notice-line") ?? boxed ?? null;
  const top = section?.getBoundingClientRect().top ?? 0;
  const title = frame.querySelector(".sync-sync-review-title");
  const faceRect = face?.getBoundingClientRect();
  const titleRect = title?.getBoundingClientRect();
  return {
    rowTop: row ? Math.round(row.getBoundingClientRect().top - top) : null,
    pill: face ? (face.querySelector(".mk-pill-words")?.getClientRects().length === 0 ? "glyph only" : face.textContent) : null,
    cut: !!face && (face.scrollWidth > face.clientWidth + 1 || (!!faceRect && !!titleRect && faceRect.right > titleRect.right + 1)),
    ink: warningInk ? Math.round(contrast(warningInk) * 100) / 100 : null,
    footer: Math.round(frame.querySelector("footer")?.getBoundingClientRect().height ?? 0),
  };
}

function Measure({ frameRef, deps }: { frameRef: React.RefObject<HTMLDivElement | null>; deps: unknown[] }) {
  const [reading, setReading] = useState<Reading | null>(null);
  useLayoutEffect(() => {
    let cancelled = false;
    const run = () => {
      if (!cancelled && frameRef.current) setReading(readFrame(frameRef.current));
    };
    void document.fonts.ready.then(() => window.setTimeout(run, 60));
    const observer = new ResizeObserver(run);
    if (frameRef.current) observer.observe(frameRef.current);
    return () => {
      cancelled = true;
      observer.disconnect();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  if (!reading) return null;
  return (
    <p className="spec-measure">
      First row at <b>{reading.rowTop ?? "—"} px</b> from the top
      {" · "}footer <b>{reading.footer} px</b>
      {reading.pill && <> · pill <b>{reading.pill === "glyph only" ? "glyph only" : reading.cut ? `“${reading.pill}” cut` : `“${reading.pill}” whole`}</b></>}
      {reading.ink !== null && <> · warning ink <b>{reading.ink.toFixed(2)}:1</b></>}
    </p>
  );
}

/* ——— references ——————————————————————————————————————————————————— */

interface Reference { label: string; href: string }
const R = {
  carbonStatus: { label: "Carbon status indicators", href: "https://carbondesignsystem.com/patterns/status-indicator-pattern/" },
  carbonNotification: { label: "Carbon notification", href: "https://carbondesignsystem.com/components/notification/usage/" },
  vscodeTrust: { label: "VS Code Restricted Mode", href: "https://code.visualstudio.com/docs/editing/workspaces/workspace-trust" },
  vscodeNotifications: { label: "VS Code notifications", href: "https://code.visualstudio.com/api/ux-guidelines/notifications" },
  chromeNotSecure: { label: "Chrome “Not secure” chip", href: "https://support.google.com/chrome/answer/95617" },
  chromeSafeBrowsing: { label: "Chrome “No protection (not recommended)”", href: "https://support.google.com/chrome/answer/99020" },
  atlassianLozenge: { label: "Atlassian Lozenge", href: "https://atlassian.design/components/lozenge/usage" },
  atlassianInline: { label: "Atlassian Inline message", href: "https://atlassian.design/components/inline-message/usage" },
  atlassianSection: { label: "Atlassian Section message", href: "https://atlassian.design/components/section-message/usage" },
  atlassianBanner: { label: "Atlassian Banner", href: "https://atlassian.design/components/banner/usage" },
  macPrivacy: { label: "macOS Control Center privacy indicator", href: "https://support.apple.com/en-nz/guide/mac-help/mchl50f94f8f" },
  primerInline: { label: "Primer InlineMessage", href: "https://primer.style/product/components/inline-message" },
  primerBanner: { label: "Primer Banner", href: "https://primer.style/product/components/banner/guidelines/" },
  primerMessaging: { label: "Primer messaging pattern", href: "https://primer.style/product/ui-patterns/notification-messaging/" },
  gmailExternal: { label: "Gmail external-recipient warning", href: "https://knowledge.workspace.google.com/admin/gmail/advanced/control-gmail-external-recipient-warnings" },
  driveShare: { label: "Google Drive “Anyone with the link”", href: "https://support.google.com/drive/answer/2494822" },
  fluentMessageBar: { label: "Fluent 2 MessageBar", href: "https://fluent2.microsoft.design/components/web/react/core/messagebar/usage" },
  fluentBadge: { label: "Fluent 2 Badge", href: "https://fluent2.microsoft.design/components/web/react/core/badge/usage" },
  fileVault: { label: "macOS turning off FileVault", href: "https://support.apple.com/guide/mac-help/turn-off-filevault-on-mac-mchlp2560/mac" },
  githubVisibility: { label: "GitHub repository visibility", href: "https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/managing-repository-settings/setting-repository-visibility" },
  appleAlerts: { label: "Apple HIG Alerts", href: "https://developer.apple.com/design/human-interface-guidelines/alerts" },
  windowsWarnings: { label: "Windows UX: warning messages", href: "https://learn.microsoft.com/en-us/windows/win32/uxguide/mess-warn" },
  windowsNotifications: { label: "Windows UX: notifications", href: "https://learn.microsoft.com/en-us/windows/win32/uxguide/mess-notif" },
  nngIndicators: { label: "NN/g indicators vs notifications", href: "https://www.nngroup.com/articles/indicators-validations-notifications/" },
  nngTooltips: { label: "NN/g tooltip guidelines", href: "https://www.nngroup.com/articles/tooltip-guidelines/" },
  nngBlindness: { label: "NN/g banner blindness", href: "https://www.nngroup.com/articles/banner-blindness-old-and-new-findings/" },
  neat: { label: "NEAT (Reeder et al., Microsoft)", href: "https://shostack.org/files/papers/ReederEtAl_NEATatMicrosoft.pdf" },
  vance2017: { label: "Vance et al., CHI 2017", href: "https://scholarsarchive.byu.edu/facpub/9293" },
  anderson2016: { label: "Anderson, Vance, Kirwan et al., JMIS 2016", href: "https://scholarsarchive.byu.edu/facpub/1955" },
  egelman: { label: "Egelman, Cranor, Hong 2008", href: "https://css.csail.mit.edu/6.566/2018/readings/youve-been-warned.pdf" },
  spool: { label: "Spool: do users change their settings?", href: "https://archive.uie.com/brainsparks/2011/09/14/do-users-change-their-settings/" },
} satisfies Record<string, Reference>;

const SEEN_IN: Record<Exclude<Dir, "before">, Reference[]> = {
  a: [R.carbonStatus, R.vscodeTrust, R.chromeNotSecure, R.atlassianLozenge],
  b: [R.atlassianInline, R.chromeNotSecure, R.vscodeTrust, R.macPrivacy],
  c: [R.primerInline, R.gmailExternal, R.driveShare],
  d: [R.carbonNotification, R.atlassianSection, R.fluentMessageBar],
  e: [R.chromeSafeBrowsing, R.fileVault, R.githubVisibility],
  f: [R.vscodeTrust, R.primerBanner, R.vscodeNotifications],
};

/** Who argues against each form. */
const AGAINST: Record<Dir, { refs: Reference[]; says: string }> = {
  before: { refs: [R.primerBanner, R.appleAlerts, R.windowsWarnings, R.vance2017], says: "Banners sparingly; an informative, non-actionable interruption is resented; warn once per condition; attention to a repeated warning drops within a working week." },
  a: { refs: [R.nngTooltips, R.egelman], says: "Never put needed information only in a tooltip (no keyboard, no touch). Passive indicators are often missed, though that study measured an attack at the moment of decision, not a standing setting." },
  b: { refs: [R.windowsWarnings], says: "Information people need should not hide behind progressive disclosure. Here the face still states the condition; only the consequence is one press away." },
  c: { refs: [], says: "No direct counter-source found. My inference: a line that repeats on every review habituates like the box, only more slowly." },
  d: { refs: [R.nngBlindness, R.anderson2016], says: "Coloured, boxed content at the top is what people learn to skip; the same warning every time habituates." },
  e: { refs: [R.spool, R.windowsNotifications], says: "Fewer than 5% of people change a default, so people on the default never meet the sentence; don't assume a notification is seen." },
  f: { refs: [R.windowsWarnings, R.fluentMessageBar], says: "A warning that seems to need “Don’t show again” should be reconsidered; Fluent brings a dismissed warning back next session. It also breaks the owner rule of 2026-09-24." },
};

type Mark = "yes" | "no" | "part";
const ASKS: { ask: string; refs: Reference[]; marks: Record<Dir, Mark> }[] = [
  {
    ask: "An ongoing condition is an indicator, not a message",
    refs: [R.carbonStatus, R.nngIndicators, R.windowsNotifications],
    marks: { before: "no", a: "yes", b: "yes", c: "part", d: "no", e: "yes", f: "part" },
  },
  {
    ask: "Don’t repeat the same warning on every visit",
    refs: [R.windowsWarnings, R.vance2017, R.primerBanner],
    marks: { before: "no", a: "yes", b: "yes", c: "no", d: "no", e: "yes", f: "yes" },
  },
  {
    ask: "State a configured setting as a fact",
    refs: [R.windowsWarnings],
    marks: { before: "no", a: "yes", b: "yes", c: "yes", d: "no", e: "yes", f: "no" },
  },
  {
    ask: "The consequence is reachable without hover",
    refs: [R.nngTooltips],
    marks: { before: "yes", a: "no", b: "yes", c: "yes", d: "yes", e: "no", f: "part" },
  },
  {
    ask: "Offer the way to change it",
    refs: [R.neat, R.windowsWarnings],
    marks: { before: "no", a: "no", b: "yes", c: "no", d: "no", e: "no", f: "no" },
  },
  {
    ask: "Colour + symbol + words",
    refs: [R.carbonStatus],
    marks: { before: "yes", a: "yes", b: "yes", c: "part", d: "yes", e: "no", f: "yes" },
  },
  {
    ask: "Fits the app’s rules (chips never close; messages close and come back)",
    refs: [],
    marks: { before: "yes", a: "yes", b: "yes", c: "yes", d: "yes", e: "yes", f: "no" },
  },
];
const MARK_TEXT: Record<Mark, string> = { yes: "✓", no: "✗", part: "~" };

function RefLinks({ refs }: { refs: Reference[] }) {
  return <>{refs.map((ref, index) => (
    <span key={ref.href}>{index > 0 && " · "}<a href={ref.href} target="_blank" rel="noreferrer">{ref.label}</a></span>
  ))}</>;
}

function References() {
  return (
    <div className="spec-panel">
      <strong>What the references ask, and how each direction meets it</strong>
      <table className="spec-table">
        <thead>
          <tr><th>Ask</th>{DIRS.map((dir) => <th key={dir} className={dir === "b" ? "pick" : undefined}>{dir === "before" ? "Before" : dir.toUpperCase()}</th>)}<th>Sources</th></tr>
        </thead>
        <tbody>
          {ASKS.map((row) => (
            <tr key={row.ask}>
              <th>{row.ask}</th>
              {DIRS.map((dir) => <td key={dir} className={`spec-mark ${row.marks[dir]}${dir === "b" ? " pick" : ""}`}>{MARK_TEXT[row.marks[dir]]}</td>)}
              <td className="spec-sources">{row.refs.length > 0 ? <RefLinks refs={row.refs} /> : "owner, 2026-09-24 / 09-25"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <ul>
        <li><b>Off is a default, not a choice.</b> Most people never change a default (<RefLinks refs={[R.spool]} />), so for nearly everyone this is a state they did not pick. That argues for a calm fact, not a warning, and for keeping the way to Settings one press away.</li>
        <li><b>Not mocked:</b> warning only when it matters, as Gmail does for external recipients (<RefLinks refs={[R.gmailExternal]} />): the warning would appear only when the selected sessions hold detected credentials. That needs a detector pass while the review loads, which is a backend change. Turning redaction on by default would also leave this pill speaking only for people who chose Off.</li>
        <li><b>Could not load:</b> Material banners (m2 renders blank, m3 returned 404) and Polaris Banner (seen only in search snippets). The GitHub “Public” label and the line by GitHub&rsquo;s Create button are not documented, so they are not cited.</li>
      </ul>
    </div>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

function Frame({ dir, knobs }: { dir: Dir; knobs: Knobs }) {
  const ref = useRef<HTMLDivElement>(null);
  const [later, setLater] = useState(false);
  const width = REVIEW_WIDTH[knobs.width];
  const seen = dir === "before" ? [] : SEEN_IN[dir];
  return (
    <div style={{ width }}>
      <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "c" && <em>picked · shipped</em>}</h2>
      <p className="spec-col-note">{DIR_NOTE[dir]}</p>
      {seen.length > 0 && (
        <p className="spec-seen">Seen in: <RefLinks refs={seen} /></p>
      )}
      <p className="spec-seen spec-against">Against: {AGAINST[dir].refs.length > 0 && <><RefLinks refs={AGAINST[dir].refs} /> — </>}{AGAINST[dir].says}</p>
      {dir === "f" && knobs.scenario !== "on" && (
        <div className="spec-group spec-inframe">
          <span>Shows</span>
          <button type="button" className="spec-seg" aria-pressed={!later} onClick={() => setLater(false)}>First review after turning it off</button>
          <button type="button" className="spec-seg" aria-pressed={later} onClick={() => setLater(true)}>Every later review</button>
        </div>
      )}
      <div ref={ref} className="spec-frame">
        <Review
          key={`${knobs.scenario}-${knobs.review}-${dir}-${later}`}
          dir={dir}
          scenario={knobs.scenario}
          review={knobs.review}
          startOpen={knobs.open === "1"}
          later={later}
        />
      </div>
      <Measure frameRef={ref} deps={[knobs.scenario, knobs.review, knobs.width, knobs.theme, later]} />
      {dir === "e" && knobs.scenario !== "on" && (
        <>
          <p className="spec-col-note mk-settings-caption">Settings › Privacy, worded for E:</p>
          <div className="spec-frame mk-settings-frame"><SettingsRowSpecimen dir="e" /></div>
        </>
      )}
    </div>
  );
}

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams({ ...next })}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const dirs = knobs.only === "all" ? DIRS : [knobs.only];
  const width = REVIEW_WIDTH[knobs.width];
  return (
    <div className={`spec${knobs.shot === "on" ? " spec-shot" : ""}`}>
      <h1 className="spec-title">Push review: a quieter way to say that credential redaction is off</h1>
      <p className="spec-sub">
        Built on the shipped CSS, with the real header pill, selection bar, tabs, notice zone, gauge and Push button. The
        session list is a stand-in. Every frame is live: tick rows, close messages, and press B&rsquo;s pill. The line under
        each frame measures how far the warning pushes the first row down and whether the pill survives the width.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Scenario</span>{SCENARIOS.map((id) => seg("scenario", id, SCENARIO_LABEL[id]))}</div>
        <div className="spec-group"><span>Review</span>{seg("review", "project", "Project Push")}{seg("review", "session", "Session Push")}</div>
        <div className="spec-group"><span>Width</span>{(["w970", "w684", "w414"] as const).map((w) => seg("width", w, WIDTH_LABEL[w]))}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
        <div className="spec-group"><span>Show</span>{seg("only", "all", "All")}{DIRS.map((dir) => seg("only", dir, dir === "before" ? "Before" : dir.toUpperCase()))}</div>
      </div>

      <div className="spec-panel">
        <strong>What shipped before C</strong>
        <ol>
          <li><b>Off is the default.</b> <code>credential_redaction_enabled: false</code> in <code>app_settings.rs</code>, so every user sees the box on every Push review, for a choice most of them never made.</li>
          <li><b>It is said twice.</b> The header pill already reads <i>Credential redaction: Off</i> (<code>.sync-settings-flag</code>, both reviews), and <code>CREDENTIAL_REDACTION_OFF_NOTICE</code> repeats it as a warning box at the top of the scroll.</li>
          <li><b>Closing does not last.</b> Every message closes (owner, 2026-09-24), and a closed one comes back when the review reopens. The × hides the box for one review only.</li>
          <li><b>It shares the zone.</b> With a Pull hold or a failed comparison, the box turns into a counted list (<i>2 notes about this Push review</i>). The redaction line then makes the real gate look like one note among several.</li>
          <li><b>The pill gives way first.</b> <code>.sync-sync-review-title &gt; .sync-settings-flag</code> ellipsizes from the end, so at narrow widths <i>Off</i> is the first word lost.</li>
        </ol>
      </div>

      <References />

      <div className="spec-stack" style={{ gridTemplateColumns: `repeat(auto-fit, ${width}px)` }}>
        {dirs.map((dir) => <Frame key={`${dir}-${knobs.width}`} dir={dir} knobs={knobs} />)}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
