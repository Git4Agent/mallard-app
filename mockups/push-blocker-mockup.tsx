import { StrictMode, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ProjectFileCommitImpact, { type ProjectFileCommitImpactCounts } from "../src/components/project-sync/ProjectFileCommitImpact";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "../src/components/project-sync/ProjectFilesReviewPage";
import { PUSH_RUNNING_NOTE, PushApplyButton } from "../src/components/project-sync/PushResourceWorkspace";
import { projectReviewTabs } from "../src/components/project-sync/reviewSteps";
import { ReviewBudgetGauge, ReviewBudgetGlance, reviewBudgetSentence } from "../src/components/sync-review/ReviewBudgetMeter";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "../src/components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import "../src/App.css";

/**
 * Mockup: the Push review footer when a selected item blocks the push.
 *
 * Owner picked C on 2026-10-02 and it shipped: the C frame's pill wears the
 * shipped `.sync-review-blocker-pill`. The Before frame draws shipped leaf
 * components, so its counts now print compact like the app's.
 *
 * Shipped, the blocker sentence is a `<span role="status"><small>` that
 * inherits `flex: 1` (basis 0) from `.sync-inline-action-footer > span`. It
 * sits beside the file counts (`flex: 1 1 360px`) and the button cluster,
 * so it gets whatever the counts leave over: ~70px at a 970px review, five
 * lines tall. The running-push note uses the same span and the same crush.
 * Its copy also tells you to exclude a file that only needs "Mark reviewed".
 *
 *   Before  what ships
 *   A       the blocker and the running note join the over-limit sentence's
 *           row above the cluster, worded by cause, naming the item
 *   B       A, plus a Show link that opens the tab and lands on the row
 *   C       the blocker as a warning pill beside the gauge; pressing it is Show
 *
 * `npm run dev`, then /mockups/push-blocker-mockup.html
 * (`?scenario=open|other-tab|many|session|over|deletions|pushing&counts=all|nonzero&width=w970|w684|w414&theme=light|dark&only=before|a|b|c&shot=on|footers`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type ScenarioId = "open" | "other-tab" | "many" | "session" | "over" | "deletions" | "pushing";
type Dir = "before" | "a" | "b" | "c";
type Counts = "all" | "nonzero";
type Width = "w970" | "w684" | "w414";
type Step = "history" | "project_files";
interface Knobs { scenario: ScenarioId; counts: Counts; width: Width; theme: "light" | "dark"; only: Dir | "all"; shot: "on" | "footers" | "off" }

const SCENARIOS: readonly ScenarioId[] = ["open", "other-tab", "many", "session", "over", "deletions", "pushing"];
const DIRS: readonly Dir[] = ["before", "a", "b", "c"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scenario: pick("scenario", SCENARIOS, "open"),
    counts: pick("counts", ["all", "nonzero"] as const, "nonzero"),
    width: pick("width", ["w970", "w684", "w414"] as const, "w970"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    shot: pick("shot", ["on", "footers", "off"] as const, "off"),
  };
}

const SCENARIO_LABEL: Record<ScenarioId, string> = {
  open: "Opened blocked (screenshot)",
  "other-tab": "Blocker on the other tab",
  many: "Three flagged files",
  session: "Session can’t be captured",
  over: "Over limit + flagged file",
  deletions: "Flagged file + deletions",
  pushing: "Pushing",
};

const WIDTH_LABEL: Record<Width, string> = {
  w970: "970 px (screenshot)",
  w684: "684 px (1100 window)",
  w414: "414 px (800 window)",
};
const REVIEW_WIDTH: Record<Width, number> = { w970: 970, w684: 684, w414: 414 };

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (shipped)",
  a: "A · Join the refusal row",
  b: "B · Refusal row + Show",
  c: "C · Warning pill at the button",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>The sentence is a <code>flex: 1</code> span (basis 0) between the counts (<code>flex: 1 1 360px</code>) and the cluster, so it gets the leftover width. It is 10px grey <code>&lt;small&gt;</code>, and it says <i>can’t be captured — exclude it</i> for a file that only needs <b>Mark reviewed</b>. The Push button’s title says the same thing again, and the path bar’s Push uses that title too.</>,
  a: <>The over-limit sentence already has a row of its own above the cluster. The blocker and the running-push note move into that row: one sentence per line, right-aligned so it ends over the Push button, warning ink plus the row’s own triangle. The copy names the cause and the item (<i>run.sh is executable — mark it reviewed or exclude it.</i>), and the same string becomes Push’s title. The footer gains one line while blocked (about 23px), and at 970 and 684 it returns to 44px once you mark the file reviewed. Try it on the row. With the deletion approval or an over-limit sentence it can be taller than Before, by up to about 24px.</>,
  b: <>A, plus <b>Show</b> at the end of the sentence. It opens the tab that holds the first blocker and scrolls its card into view. This is the only direction that helps when the blocker sits on the tab you are <i>not</i> on: A relies on the tab’s warning dot. Real cost: cards need a <code>data-resource-id</code>, and a search that hides the card, or the 500-card render limit, would need the search set to its path.</>,
  c: <>No sentence row for the blocker. A warning pill sits in the cluster, in the gauge pill’s shape. With compact counts the footer stays one row at 970 and 684, and C is never taller than Before. Pressing it does B’s Show. The full sentence goes in the pill’s title and in a visually hidden status. The running note still needs a row, so it uses A’s.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;
const LIMIT = 512 * MB;
const NOW = Math.floor(new Date("2026-10-02T10:40:00").getTime() / 1000);

interface MockFile {
  path: string;
  size: number;
  warning?: "executable" | "possible_secret";
  removal?: boolean;
}

const BASE_FILES: MockFile[] = [
  { path: "README.txt", size: 2.1 * KB },
  { path: "notes/plan.md", size: 6.4 * KB },
  { path: "notes/todo.md", size: 1.8 * KB },
  { path: "assets/cover.png", size: 1.2 * MB },
  { path: "data/sample.csv", size: 310 * KB },
  { path: "data/schema.json", size: 4.2 * KB },
  { path: "scripts/run.sh", size: 1.1 * KB, warning: "executable" },
];

interface Scenario {
  id: ScenarioId;
  files: MockFile[];
  step: Step;
  /** A selected session the capture could not read. */
  lostSession: boolean;
  total: number;
  busy: boolean;
}

function scenario(id: ScenarioId): Scenario {
  let files = BASE_FILES;
  if (id === "many") {
    files = [
      ...BASE_FILES,
      { path: "scripts/build.sh", size: 2.4 * KB, warning: "executable" },
      { path: ".env.local", size: 0.4 * KB, warning: "possible_secret" },
    ];
  }
  if (id === "session" || id === "pushing") files = BASE_FILES.map((file) => ({ ...file, warning: undefined }));
  if (id === "deletions") {
    files = [
      ...BASE_FILES,
      { path: "notes/old-plan.md", size: 5.2 * KB, removal: true },
      { path: "data/legacy.csv", size: 88 * KB, removal: true },
    ];
  }
  return {
    id,
    files,
    step: id === "other-tab" || id === "session" ? "history" : "project_files",
    lostSession: id === "session",
    total: id === "over" ? 2.6 * GB : 1.9 * MB,
    busy: id === "pushing",
  };
}

const fileId = (path: string) => `project_file:${path}`;
const SESSION_LOST_ID = "session:fix-login";
const SESSION_LOST_TITLE = "Fix login redirect";
const SESSIONS = [
  { id: "session:readme", title: "Tidy the README", size: "420 KB" },
  { id: SESSION_LOST_ID, title: SESSION_LOST_TITLE, size: "—" },
  { id: "session:csv", title: "Load the sample CSV", size: "1.1 MB" },
];

function toRow(file: MockFile): ProjectFileReviewRow {
  return {
    resourceId: fileId(file.path),
    relativePath: file.path,
    entryType: "file",
    state: file.removal ? "missing" : "local_only",
    size: file.size,
    mode: file.warning === "executable" ? 0o755 : 0o644,
    sourceMtime: NOW - 3600,
    localPresent: !file.removal,
    storagePresent: !!file.removal,
    undecided: !file.removal,
    warningCode: file.warning ?? null,
    warningDigest: file.warning ? `digest:${file.path}` : null,
  };
}

/* ——— the blocker, and the one sentence both Push buttons would read —— */

interface Blocker {
  kind: "warning" | "lost";
  step: Step;
  id: string;
  name: string;
  warning?: MockFile["warning"];
}

function blockersOf(s: Scenario, selected: ReadonlySet<string>, acknowledged: ReadonlySet<string>): Blocker[] {
  const list: Blocker[] = [];
  if (s.lostSession && selected.has(SESSION_LOST_ID)) {
    list.push({ kind: "lost", step: "history", id: SESSION_LOST_ID, name: SESSION_LOST_TITLE });
  }
  for (const file of s.files) {
    const id = fileId(file.path);
    if (file.warning && selected.has(id) && !acknowledged.has(`digest:${file.path}`)) {
      list.push({ kind: "warning", step: "project_files", id, name: file.path.split("/").pop()!, warning: file.warning });
    }
  }
  return list;
}

/** Proposed: one helper, worded by cause. It feeds the footer line and the
 *  disabled reason, which is also the title of both Push buttons. */
function blockerSentence(list: readonly Blocker[]): string | null {
  const flagged = list.filter((blocker) => blocker.kind === "warning");
  const lost = list.filter((blocker) => blocker.kind === "lost");
  const parts: string[] = [];
  if (flagged.length === 1) {
    const [only] = flagged;
    parts.push(`${only.name} ${only.warning === "possible_secret" ? "may hold a secret" : "is executable"} — mark it reviewed or exclude it.`);
  } else if (flagged.length > 1) {
    parts.push(`${flagged.length} files need review — mark them reviewed or exclude them.`);
  }
  if (lost.length === 1) parts.push(`“${lost[0].name}” can’t be captured — exclude it to push.`);
  else if (lost.length > 1) parts.push(`${lost.length} sessions can’t be captured — exclude them to push.`);
  return parts.length > 0 ? parts.join(" ") : null;
}

function pillLabel(list: readonly Blocker[]): string {
  const flagged = list.filter((blocker) => blocker.kind === "warning").length;
  if (flagged === list.length) return `${flagged} need${flagged === 1 ? "s" : ""} review`;
  if (flagged === 0) return `${list.length} unavailable`;
  return `${list.length} need attention`;
}

function showLabel(list: readonly Blocker[]): string {
  const first = list[0];
  if (first.kind === "lost") return "Show session";
  return list.filter((blocker) => blocker.kind === "warning").length > 1 ? "Show files" : "Show file";
}

/* ——— pieces ——————————————————————————————————————————————————————— */

function SessionsStandIn({ selected, onToggle }: { selected: ReadonlySet<string>; onToggle: (id: string) => void }) {
  return (
    <div className="mk-session-list" aria-label="Sessions (stand-in)">
      {SESSIONS.map((session) => {
        const lost = session.id === SESSION_LOST_ID;
        return (
          <article key={session.id} className="sync-history-thread-card catalog-card" data-mk-id={session.id}>
            <div className="sync-history-thread-topline">
              <div className="sync-history-thread-title">
                <label className="sync-sync-row-choice">
                  <input type="checkbox" checked={selected.has(session.id)} onChange={() => onToggle(session.id)} aria-label={`Include ${session.title}`} />
                </label>
                <span className={`sync-thread-sync-indicator ${lost ? "unavailable" : "local"}`}>
                  <ThreadStatusIcon status={lost ? "unavailable" : "local-outstanding"} size={18} />
                </span>
                <strong>{session.title}</strong>
                {lost && <span className="sync-history-branch-state history-only">Unavailable</span>}
              </div>
            </div>
            <div className="sync-history-session-details">
              <div className="sync-history-session-summary">
                <div className="sync-history-thread-meta">
                  <span className="sync-history-thread-metric"><b>{session.size}</b></span>
                </div>
              </div>
            </div>
          </article>
        );
      })}
    </div>
  );
}

/** The counts knob: the shipped component prints all four, zeros included. */
function Impact({ counts, mode, deletionApproved, onApprove }: {
  counts: ProjectFileCommitImpactCounts;
  mode: Counts;
  deletionApproved: boolean;
  onApprove: (approved: boolean) => void;
}) {
  if (mode === "all") {
    return (
      <ProjectFileCommitImpact
        counts={counts}
        deletionTarget="storage"
        deletionApproved={deletionApproved}
        disabled={false}
        onDeletionApprovedChange={onApprove}
      />
    );
  }
  const deletions = counts.deleteFile + counts.deleteDirectory;
  const parts: [number, string][] = [
    [counts.add, "Add"],
    [counts.replace, "Replace"],
    [counts.deleteFile, "Delete files"],
    [counts.deleteDirectory, "Delete folders"],
  ];
  return (
    <div className="sync-project-file-commit-impact mk-compact" aria-label="Selected project file impact">
      <div className="sync-project-file-impact-counts">
        {parts.filter(([n]) => n > 0).map(([n, label]) => <span key={label}><strong>{n}</strong> {label}</span>)}
      </div>
      {deletions > 0 && (
        <label className="sync-project-file-deletion-ack">
          <span className="sync-sync-row-choice">
            <input type="checkbox" checked={deletionApproved} onChange={(event) => onApprove(event.target.checked)} aria-label="Approve the selected project file deletions" />
          </span>
          <span>{`Approve removal of ${counts.deleteFile} file${counts.deleteFile === 1 ? "" : "s"} and ${counts.deleteDirectory} folder${counts.deleteDirectory === 1 ? "" : "s"} from storage.`}</span>
        </label>
      )}
    </div>
  );
}

function Review({ dir, s, counts }: { dir: Dir; s: Scenario; counts: Counts }) {
  const allIds = [
    ...SESSIONS.map((session) => session.id),
    ...s.files.filter((file) => !file.removal).map((file) => fileId(file.path)),
  ];
  const [selected, setSelected] = useState<Set<string>>(() => new Set(allIds));
  const [acknowledged, setAcknowledged] = useState<Set<string>>(
    () => new Set(s.busy ? s.files.filter((file) => file.warning).map((file) => `digest:${file.path}`) : []),
  );
  const [deletionApproved, setDeletionApproved] = useState(false);
  const [step, setStep] = useState<Step>(s.step);
  const [flashId, setFlashId] = useState<string | null>(null);
  const frameRef = useRef<HTMLElement>(null);
  const footerRef = useRef<HTMLElement>(null);
  const [footerHeight, setFooterHeight] = useState(0);

  useLayoutEffect(() => {
    const footer = footerRef.current;
    if (!footer) return;
    const observer = new ResizeObserver(() => setFooterHeight(Math.round(footer.getBoundingClientRect().height)));
    observer.observe(footer);
    return () => observer.disconnect();
  }, []);

  /* Show: once the tab has rendered, land on the named card. */
  useEffect(() => {
    if (!flashId) return;
    const root = frameRef.current;
    const path = flashId.startsWith("project_file:") ? flashId.slice("project_file:".length) : null;
    const card = path
      ? root?.querySelector(`strong[title="${CSS.escape(path)}"]`)?.closest("article")
      : root?.querySelector(`[data-mk-id="${CSS.escape(flashId)}"]`);
    if (!card) return;
    card.scrollIntoView({ block: "center", behavior: "smooth" });
    card.classList.add("mk-flash");
    const timer = window.setTimeout(() => { card.classList.remove("mk-flash"); setFlashId(null); }, 1400);
    return () => window.clearTimeout(timer);
  }, [flashId, step]);

  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  const blockers = blockersOf(s, selected, acknowledged);
  const sentence = blockerSentence(blockers);
  const budget = { total: s.total, limit: LIMIT };
  const over = budget.total > budget.limit;
  const overSentence = over ? reviewBudgetSentence(budget, "push") : null;
  const breakdown = `Sessions 1.5 MB · Requirements 410 KB · Project files ${over ? "2.6 GB" : "1.5 MB"}`;

  const impact: ProjectFileCommitImpactCounts = {
    add: s.files.filter((file) => !file.removal && selected.has(fileId(file.path))).length,
    replace: 0,
    deleteFile: s.files.filter((file) => file.removal).length,
    deleteDirectory: 0,
  };
  const deletionsPending = impact.deleteFile > 0 && !deletionApproved;

  const before = dir === "before";
  const disabledReason = over
    ? overSentence
    : blockers.length > 0
      ? (before ? "Exclude the items that can't be captured" : sentence)
      : deletionsPending ? "Approve the selected project file deletions" : null;
  const beforeSentence = blockers.length > 0
    ? `${blockers.length} selected item${blockers.length === 1 ? "" : "s"} can't be captured — exclude ${blockers.length === 1 ? "it" : "them"} to push.`
    : null;

  const show = () => {
    if (blockers.length === 0) return;
    setStep(blockers[0].step);
    setFlashId(blockers[0].id);
  };

  const fileCount = impact.add;
  const sessionCount = SESSIONS.filter((session) => selected.has(session.id)).length;
  const warningSteps = new Set<Step>(blockers.map((blocker) => blocker.step));
  const stepNavigator = (
    <button
      type="button"
      className="btn btn-ghost"
      disabled={s.busy}
      title={step === "history" ? "Review project files next" : "Back to session selection"}
      onClick={() => setStep(step === "history" ? "project_files" : "history")}
    >
      {step !== "history" && <Icon name="chevron-left" size={14} />}
      {step === "history" ? "Project files" : "Sessions"}
      {step === "history" && <Icon name="chevron-right" size={14} />}
    </button>
  );
  const gauge = <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} />;
  const push = <PushApplyButton busy={s.busy} disabledReason={disabledReason} onPush={() => undefined} />;

  let footer: ReactNode;
  if (before) {
    footer = (
      <>
        {overSentence && <p className="sync-review-budget-sentence">{overSentence}</p>}
        <ProjectFileCommitImpact
          counts={impact}
          deletionTarget="storage"
          deletionApproved={deletionApproved}
          disabled={s.busy}
          onDeletionApprovedChange={setDeletionApproved}
        />
        {beforeSentence && <span role="status"><small>{beforeSentence}</small></span>}
        {s.busy && <span role="status"><small>{PUSH_RUNNING_NOTE}</small></span>}
        {stepNavigator}
        {gauge}
        {push}
      </>
    );
  } else {
    const pill = dir === "c" && sentence;
    footer = (
      <>
        {overSentence && (
          <p className="mk-note" style={{ color: "var(--danger-text)" }}>
            <span className="mk-note-text">{overSentence}</span>
          </p>
        )}
        {sentence && !pill && (
          <p className="mk-note warning" role="status">
            <Icon name="alert-triangle" size={12} aria-hidden="true" />
            <span className="mk-note-text">{sentence}</span>
            {dir === "b" && <button type="button" className="btn-link" onClick={show}>{showLabel(blockers)}</button>}
          </p>
        )}
        {pill && <span className="sync-visually-hidden" role="status">{sentence}</span>}
        {s.busy && <p className="mk-note" role="status"><span className="mk-note-text">{PUSH_RUNNING_NOTE}</span></p>}
        <Impact counts={impact} mode={counts} deletionApproved={deletionApproved} onApprove={setDeletionApproved} />
        {stepNavigator}
        {pill && (
          <button type="button" className="sync-review-blocker-pill" title={sentence} onClick={show}>
            <Icon name="alert-triangle" size={13} aria-hidden="true" />
            {pillLabel(blockers)}
          </button>
        )}
        {gauge}
        {push}
      </>
    );
  }

  return (
    <>
      <section
        ref={frameRef}
        className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace"
        aria-label="Push review"
      >
        <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
          <div className="sync-sync-review-title">
            <h2>Push review</h2>
            <span className="sync-settings-flag">Credential redaction: On</span>
          </div>
          <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown} />
          <SyncReviewSelectionBar
            count={step === "history" ? sessionCount : fileCount}
            total={step === "history" ? SESSIONS.length : s.files.filter((file) => !file.removal).length}
            busy={s.busy}
            recommendedCount={step === "history" ? SESSIONS.length : s.files.length}
            recommendedTitle="Use the recommended selection"
            recommendedDisabled
            onRecommended={() => undefined}
            clearDisabled={false}
            onClear={() => undefined}
            close={{ ariaLabel: "Close push review", onClose: () => undefined }}
          />
        </header>
        <SyncReviewTabs
          tabs={projectReviewTabs(["history", "project_files"], { history: sessionCount, project_files: fileCount }, warningSteps, "Sessions")}
          activeId={step}
          disabled={s.busy}
          onChange={(id) => setStep(id as Step)}
        />
        <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            {step === "history" ? (
              <div className="sync-history-page sync-history-embedded sync-history-selection-review">
                <SessionsStandIn selected={selected} onToggle={toggle} />
              </div>
            ) : (
              <ProjectFilesReviewPage
                mode="push"
                eligibility={{ state: "eligible", reason: "This project folder is not inside a Git work tree." }}
                rows={s.files.map(toRow)}
                selectedIds={selected}
                removalIds={new Set(s.files.filter((file) => file.removal).map((file) => fileId(file.path)))}
                acknowledgedWarningDigests={acknowledged}
                scanned
                disabled={s.busy}
                onToggle={toggle}
                onBulkToggle={(ids, on) => setSelected((current) => {
                  const next = new Set(current);
                  ids.forEach((id) => (on ? next.add(id) : next.delete(id)));
                  return next;
                })}
                onToggleWarning={(digest) => setAcknowledged((current) => {
                  const next = new Set(current);
                  if (next.has(digest)) next.delete(digest); else next.add(digest);
                  return next;
                })}
              />
            )}
          </div>
        </div>
        <footer ref={footerRef} className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
          {footer}
        </footer>
      </section>
      <p className="spec-measure">footer <b>{footerHeight}px</b> · Push title: <b>{disabledReason ?? "—"}</b></p>
    </>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

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
  const s = scenario(knobs.scenario);
  const width = REVIEW_WIDTH[knobs.width];
  const dirs = knobs.only === "all" ? DIRS : [knobs.only];
  return (
    <div className={`spec${knobs.shot === "off" ? "" : " spec-shot"}${knobs.shot === "footers" ? " spec-footers" : ""}`}>
      <h1 className="spec-title">Push footer: a blocked selection, without the five-line column</h1>
      <p className="spec-sub">
        Drawn on the shipped CSS with the real header, tabs, file cards, counts, gauge and Push button. Only the note row and
        the pill are new (mock CSS in the page’s <code>&lt;style&gt;</code>). The cards are live: tick, untick, and
        <b> Mark reviewed</b> all change the footer, and the measurement under each frame updates.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Scenario</span>{SCENARIOS.map((id) => seg("scenario", id, SCENARIO_LABEL[id]))}</div>
        <div className="spec-group"><span>Review</span>{(["w970", "w684", "w414"] as const).map((w) => seg("width", w, WIDTH_LABEL[w]))}</div>
        <div className="spec-group"><span>Counts (A–C)</span>{seg("counts", "all", "All four (shipped)")}{seg("counts", "nonzero", "Compact")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel">
        <strong>What’s wrong in the screenshot</strong>
        <ol>
          <li><b>The sentence is crushed into a column.</b> It is <code>&lt;span role="status"&gt;&lt;small&gt;</code>, and <code>.sync-inline-action-footer &gt; span</code> gives it <code>flex: 1</code> (basis 0). The counts block next to it is <code>flex: 1 1 360px</code> with <code>margin-right: auto</code>. At 970px the leftover is about 140px, split between the two, so the sentence gets about 70px and five lines. At 684 the counts take their own row. At 414 the column is about 120px.</li>
          <li><b>The running-push note has the same bug.</b> <code>PUSH_RUNNING_NOTE</code> uses the same span (see <i>Pushing</i>).</li>
          <li><b>The copy points at the wrong fix.</b> “Can’t be captured — exclude it” covers two causes. One is a session or file the capture really can’t read (<code>unavailable</code>, <code>blocked</code>, <code>unknown</code>). The other is a selected file with an unacknowledged scanner warning (<i>Executable</i> or <i>Possible secret</i>). The second is fixed by <b>Mark reviewed</b> on its card, and the sentence never says so. With 7 files to add on the Files tab, the second is the likely cause of the screenshot, but that is an inference.</li>
          <li><b>The review can open already blocked.</b> In <code>service.rs</code>, <code>selected_after_scan</code> = newly discovered, not blocked, not excluded. Scanner warnings don’t opt out, so a new <code>run.sh</code> is ticked by default and the review opens with Push dead.</li>
          <li><b>Two strings, and they disagree.</b> The footer says <i>N selected items can’t be captured…</i>. <code>pushReviewDisabledReason</code> says <i>Exclude the items that can’t be captured</i>, and that is also the title of the path bar’s Push. Whichever direction you pick, the plan replaces both with one helper.</li>
          <li>Smaller: the zeros (<i>0 Replace · 0 Delete files · 0 Delete folders</i>) take about 250px of the footer and say nothing. The <i>Counts</i> knob drops them in A–C.</li>
        </ol>
      </div>

      <div className="spec-stack">
        {dirs.map((dir) => (
          <div key={dir}>
            <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "a" && <em>recommended</em>}</h2>
            <p className="spec-col-note">{DIR_NOTE[dir]}</p>
            <div className="spec-frame" style={{ width }}>
              <Review key={`${knobs.scenario}-${dir}-${knobs.counts}`} dir={dir} s={s} counts={knobs.counts} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
