import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ProjectFileCommitImpact, { type ProjectFileCommitImpactCounts } from "../src/components/project-sync/ProjectFileCommitImpact";
import ProjectFilesReviewPage, { type ProjectFileReviewRow } from "../src/components/project-sync/ProjectFilesReviewPage";
import { PushApplyButton } from "../src/components/project-sync/PushResourceWorkspace";
import { projectReviewTabs } from "../src/components/project-sync/reviewSteps";
import { ReviewBudgetGauge, ReviewBudgetGlance, reviewBudgetSentence } from "../src/components/sync-review/ReviewBudgetMeter";
import SyncReviewSelectionBar from "../src/components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "../src/components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import { formatBytes } from "../src/format";
import "../src/App.css";

/**
 * Mockup: which project files a non-Git Push ticks by default.
 *
 * Owner picked A on 2026-10-02 and it shipped: the New chip is the real
 * card's, and the footer count wears the shipped rule. The Before frame is
 * a re-enactment of the lazy scan it replaced.
 *
 * Shipped, the folder is not walked until the Files tab is opened. A Push
 * from the Sessions step carries no files and no edits to files already in
 * storage. Opening the tab ticks every new file, flagged ones included.
 *
 *   Before  what ships
 *   A       changed files travel, new files wait for a tick
 *   B       new files travel unless flagged; Push waits until the list was seen
 *   C       A, plus a per-link switch that turns new files to travel
 *
 * `npm run dev`, then /mockups/push-file-defaults-mockup.html
 * (`?scenario=first|later|edits|big&open=history|project_files&width=w684|w970&theme=light|dark&only=before|a|b|c&shot=on`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type ScenarioId = "first" | "later" | "edits" | "big";
type Dir = "before" | "a" | "b" | "c";
type Width = "w684" | "w970";
type Step = "history" | "project_files";
interface Knobs { scenario: ScenarioId; open: Step; width: Width; theme: "light" | "dark"; only: Dir | "all"; shot: "on" | "off" }

const SCENARIOS: readonly ScenarioId[] = ["first", "later", "edits", "big"];
const DIRS: readonly Dir[] = ["before", "a", "b", "c"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scenario: pick("scenario", SCENARIOS, "first"),
    open: pick("open", ["history", "project_files"] as const, "history"),
    width: pick("width", ["w684", "w970"] as const, "w684"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
    shot: pick("shot", ["on", "off"] as const, "off"),
  };
}

const SCENARIO_LABEL: Record<ScenarioId, string> = {
  first: "First push (screenshot)",
  later: "Later push: 2 edited, 2 new",
  edits: "Later push: edits only",
  big: "First push: 560 MB of data",
};

const WIDTH_LABEL: Record<Width, string> = { w684: "684 px (1100 window)", w970: "970 px" };
const REVIEW_WIDTH: Record<Width, number> = { w684: 684, w970: 970 };

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (shipped)",
  a: "A · Changed files travel, new files wait",
  b: "B · New files travel, after one look",
  c: "C · A, with a switch",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>The folder is not walked until the Files tab is opened. Push from Sessions carries <b>no files</b>, and no edits to files already in storage, and this step never says so. Opening the tab ticks <b>every</b> new file, flagged ones included, so the review turns blocked. Looking at a tab changes what Push sends.</>,
  a: <>The scan runs with the review. A file already in storage is ticked when it changed here. A new file is never ticked for you: the step button counts the ones that wait, <b>Select</b> ticks those without a finding, and a flagged file needs its own tick. Push stays live on both steps. A file you saw and left unticked is remembered and not counted again.</>,
  b: <>The scan runs with the review and ticks every new file without a finding. Ticks on a step nobody opened would travel unseen, so while new files are ticked <b>Push waits until the Files step was shown once</b>, and the step button becomes the way forward. Files spend the 512 MB budget before sessions do; a large folder opens refused unless files get a fit rule too.</>,
  c: <>A&rsquo;s defaults, plus one switch on the Files step, saved with this storage link: <b>New files · Wait / Travel</b>. Switched to Travel, new files without a finding are ticked on every push and Push does not wait: the switch is the standing approval. Costs one persisted field and a second behaviour to test and explain.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const KB = 1024;
const MB = 1024 * KB;
const LIMIT = 512 * MB;
const RESERVED = 21 * KB;
const NOW = Math.floor(new Date("2026-10-02T22:45:00").getTime() / 1000);

interface MockSession { id: string; title: string; bytes: number; stamp: string }

interface MockFile {
  path: string;
  size: number;
  /** new: never pushed, never decided. changed: in storage, edited here.
   *  left: on this computer, unticked at an earlier push. */
  kind: "new" | "changed" | "left";
  warning?: "executable" | "possible_secret";
}

interface Scenario { id: ScenarioId; sessions: MockSession[]; files: MockFile[] }

const SCREENSHOT_SESSIONS: MockSession[] = [
  { id: "session:release", title: "Locate mallard-release folder", bytes: 1.4 * MB, stamp: "Oct 2, 2026 at 22:41" },
  { id: "session:feishu", title: "Locate and move Feishu env", bytes: 415 * KB, stamp: "Oct 2, 2026 at 22:34" },
];

function scenario(id: ScenarioId): Scenario {
  if (id === "later") {
    return {
      id,
      sessions: [{ id: "session:notes", title: "Draft the release notes", bytes: 620 * KB, stamp: "Oct 4, 2026 at 09:12" }],
      files: [
        { path: "notes/plan.md", size: 7.0 * KB, kind: "changed" },
        { path: "data/sample.csv", size: 33 * KB, kind: "changed" },
        { path: "notes/ideas.md", size: 3.2 * KB, kind: "new" },
        { path: "assets/logo.png", size: 22 * KB, kind: "new" },
        { path: "scripts/run.sh", size: 1.1 * KB, kind: "left", warning: "executable" },
        { path: ".env", size: 0.4 * KB, kind: "left", warning: "possible_secret" },
      ],
    };
  }
  if (id === "edits") {
    return {
      id,
      sessions: [{ id: "session:notes", title: "Draft the release notes", bytes: 620 * KB, stamp: "Oct 4, 2026 at 09:12" }],
      files: [{ path: "notes/plan.md", size: 7.0 * KB, kind: "changed" }],
    };
  }
  if (id === "big") {
    return {
      id,
      sessions: SCREENSHOT_SESSIONS,
      files: [
        { path: "README.md", size: 2.1 * KB, kind: "new" },
        { path: "notes/plan.md", size: 6.4 * KB, kind: "new" },
        { path: "data/out/summary.csv", size: 310 * KB, kind: "new" },
        ...Array.from({ length: 40 }, (_, index): MockFile => ({
          path: `data/raw/part-${String(index + 1).padStart(2, "0")}.parquet`,
          size: 14 * MB,
          kind: "new",
        })),
      ],
    };
  }
  return {
    id,
    sessions: SCREENSHOT_SESSIONS,
    files: [
      { path: "README.md", size: 2.1 * KB, kind: "new" },
      { path: ".env", size: 0.4 * KB, kind: "new", warning: "possible_secret" },
      { path: "notes/plan.md", size: 6.4 * KB, kind: "new" },
      { path: "notes/todo.md", size: 1.8 * KB, kind: "new" },
      { path: "assets/cover.png", size: 48 * KB, kind: "new" },
      { path: "data/sample.csv", size: 31 * KB, kind: "new" },
      { path: "scripts/run.sh", size: 1.1 * KB, kind: "new", warning: "executable" },
    ],
  };
}

const fileId = (file: MockFile) => `project_file:${file.path}`;
const digestOf = (file: MockFile) => `digest:${file.path}`;

/** Before draws the shipped row, which had no New chip. */
function toRow(file: MockFile, proposed: boolean): ProjectFileReviewRow {
  return {
    resourceId: fileId(file),
    relativePath: file.path,
    entryType: "file",
    state: file.kind === "changed" ? "local_ahead" : "local_only",
    size: file.size,
    mode: file.warning === "executable" ? 0o755 : 0o644,
    sourceMtime: NOW - 3600,
    localPresent: true,
    storagePresent: file.kind === "changed",
    undecided: proposed && file.kind === "new",
    warningCode: file.warning ?? null,
    warningDigest: file.warning ? digestOf(file) : null,
  };
}

const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? "" : "s"}`;

/* ——— the industry line ————————————————————————————————————————————— */

type Verdict = "yes" | "no" | "ask";
interface IndustryRow { tool: string; known: [Verdict, string]; fresh: [Verdict, string]; net: string; href: string }
interface IndustryGroup { title: string; rows: IndustryRow[] }

/* Checked against vendor docs or source on 2026-10-02. Claims the docs do
   not state outright are marked as such. */
const INDUSTRY: IndustryGroup[] = [
  {
    title: "Review, then commit (the commit stays local and can be amended)",
    rows: [
      { tool: "Git CLI", known: ["ask", "only with commit -a"], fresh: ["no", "never without git add"], net: ".gitignore, commit --dry-run", href: "https://git-scm.com/docs/git-commit" },
      { tool: "GitHub Desktop", known: ["yes", "ticked"], fresh: ["yes", "ticked"], net: ".gitignore; checks for files over 100 MB. An open request asks to stop ticking untracked files.", href: "https://docs.github.com/en/desktop/making-changes-in-a-branch/committing-and-reviewing-changes-to-your-project-in-github-desktop" },
      { tool: "VS Code", known: ["ask", "asks: stage all?"], fresh: ["ask", "in the same prompt"], net: "Nothing staged means a Yes / Always / Never prompt; smart commit is off by default.", href: "https://github.com/microsoft/vscode/blob/main/extensions/git/package.json" },
      { tool: "JetBrains IDEs", known: ["yes", "whole changelist"], fresh: ["no", "separate Unversioned node"], net: "New files are ticked to add them. That they start unticked is implied by the docs, not stated.", href: "https://www.jetbrains.com/help/idea/commit-and-push-changes.html" },
      { tool: "Sublime Merge, Sourcetree, GitKraken", known: ["no", "unstaged"], fresh: ["no", "unstaged"], net: "One Stage all control. Sublime Merge lists new files in their own Untracked section.", href: "https://www.sublimemerge.com/docs/getting_started" },
    ],
  },
  {
    title: "Sync or back up to a place only you read (no review step)",
    rows: [
      { tool: "Dropbox, Syncthing", known: ["yes", "all"], fresh: ["yes", "all"], net: "Opt out per file: an ignore attribute, .stignore.", href: "https://help.dropbox.com/sync/ignored-files" },
      { tool: "Time Machine, Backblaze, Arq, restic", known: ["yes", "all"], fresh: ["yes", "all"], net: "Exclusion lists. Backblaze ships default exclusions.", href: "https://help.backblaze.com/hc/en-us/articles/217666298-Exclusions-Settings-Mac" },
    ],
  },
  {
    title: "Publish to an audience (cannot be taken back)",
    rows: [
      { tool: "npm publish", known: ["yes", "all"], fresh: ["yes", "all"], net: "A files allowlist, .npmignore, a never-include list, npm pack --dry-run. npm’s own blog calls the allowlist the safest method; leaked tokens in public tarballs are documented.", href: "https://blog.npmjs.org/post/165769683050/publishing-what-you-mean-to-publish" },
      { tool: "Docker build, Vercel CLI, gcloud", known: ["yes", "all"], fresh: ["yes", "all"], net: ".dockerignore; Vercel skips .env.local and friends by itself; gcloud generates a .gcloudignore and can list what it would upload.", href: "https://vercel.com/docs/deployments/vercel-ignore" },
      { tool: "cargo package", known: ["yes", "all"], fresh: ["ask", "refuses a dirty tree"], net: "Outside Git it skips every dotfile. Uncommitted files need --allow-dirty. cargo package --list previews.", href: "https://doc.rust-lang.org/cargo/commands/cargo-package.html" },
      { tool: "GitHub push protection", known: ["ask", "secrets blocked"], fresh: ["ask", "secrets blocked"], net: "On by default for public repositories; a bypass needs a reason. Files over 100 MiB are refused.", href: "https://docs.github.com/en/code-security/secret-scanning/introduction/about-push-protection" },
    ],
  },
];

const GUIDANCE: { source: string; says: string; href: string }[] = [
  { source: "Microsoft, Windows checkbox guidelines", says: "A checkbox starts in the safest, most private state. Convenience decides only when safety is not in play.", href: "https://learn.microsoft.com/en-us/windows/win32/uxguide/ctrl-check-boxes" },
  { source: "Nielsen Norman Group", says: "People keep the defaults they are given, so the default is the behaviour most of them will get.", href: "https://www.nngroup.com/articles/the-power-of-defaults/" },
  { source: "GDPR Art. 25(2)", says: "By default, personal data must not reach an open-ended audience without the person acting.", href: "https://gdpr-info.eu/art-25-gdpr/" },
  { source: "Saltzer & Schroeder, fail-safe defaults", says: "Base access on permission, not exclusion: the default is no access.", href: "https://web.mit.edu/Saltzer/www/publications/protection/Basic.html" },
];

const VERDICT_CLASS: Record<Verdict, string> = { yes: "spec-yes", no: "spec-no", ask: "spec-ask" };

/* ——— pieces ——————————————————————————————————————————————————————— */

function SessionsStandIn({ sessions, selected, onToggle }: {
  sessions: MockSession[];
  selected: ReadonlySet<string>;
  onToggle: (id: string) => void;
}) {
  return (
    <div className="mk-session-list" aria-label="Sessions (stand-in)">
      {sessions.map((session) => (
        <article key={session.id} className="sync-history-thread-card catalog-card">
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

function Review({ dir, s, open }: { dir: Dir; s: Scenario; open: Step }) {
  const fresh = s.files.filter((file) => file.kind === "new");
  const freshSafe = fresh.filter((file) => !file.warning);
  const changed = s.files.filter((file) => file.kind === "changed");

  /* What each direction ticks without being asked. */
  const defaults = (travel: boolean): string[] => {
    if (dir === "before") return [...fresh, ...changed].map(fileId);
    if (dir === "b" || (dir === "c" && travel)) return [...freshSafe, ...changed].map(fileId);
    return changed.map(fileId);
  };
  /* What Select ticks: shipped, the defaults; proposed, every file without a
     finding that nobody left out. */
  const recommended = new Set((dir === "before" ? [...fresh, ...changed] : [...freshSafe, ...changed]).map(fileId));

  const scannedAtOpen = dir !== "before" || open === "project_files";
  const [scanned, setScanned] = useState(scannedAtOpen);
  const [seen, setSeen] = useState(open === "project_files");
  const [travel, setTravel] = useState(false);
  const [step, setStep] = useState<Step>(open);
  const [selected, setSelected] = useState<Set<string>>(() => new Set([
    ...s.sessions.map((session) => session.id),
    ...(scannedAtOpen ? defaults(false) : []),
  ]));
  const [acknowledged, setAcknowledged] = useState<Set<string>>(() => new Set());

  const goTo = (next: Step) => {
    setStep(next);
    if (next !== "project_files") return;
    setSeen(true);
    if (!scanned) {
      // Shipped: opening the step is the scan, and the scan ticks its finds.
      setScanned(true);
      setSelected((current) => new Set([...current, ...defaults(false)]));
    }
  };
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const switchTravel = (on: boolean) => {
    setTravel(on);
    setSelected((current) => {
      const next = new Set(current);
      freshSafe.forEach((file) => (on ? next.add(fileId(file)) : next.delete(fileId(file))));
      return next;
    });
  };

  const files = scanned ? s.files : [];
  const tickedFiles = files.filter((file) => selected.has(fileId(file)));
  const tickedSessions = s.sessions.filter((session) => selected.has(session.id));
  const waiting = fresh.filter((file) => !selected.has(fileId(file)));
  const blockers = tickedFiles.filter((file) => file.warning && !acknowledged.has(digestOf(file)));
  const budget = {
    total: RESERVED
      + tickedSessions.reduce((sum, session) => sum + session.bytes, 0)
      + tickedFiles.reduce((sum, file) => sum + file.size, 0),
    limit: LIMIT,
  };
  const over = budget.total > budget.limit;
  const overSentence = over ? reviewBudgetSentence(budget, "push") : null;
  const breakdown = `Sessions ${formatBytes(tickedSessions.reduce((sum, session) => sum + session.bytes, 0))} · Requirements ${formatBytes(RESERVED)} · Project files ${formatBytes(tickedFiles.reduce((sum, file) => sum + file.size, 0))}`;
  const impact: ProjectFileCommitImpactCounts = {
    add: tickedFiles.filter((file) => file.kind !== "changed").length,
    replace: tickedFiles.filter((file) => file.kind === "changed").length,
    deleteFile: 0,
    deleteDirectory: 0,
  };

  /* B: ticks nobody has seen do not travel. */
  const unseenNew = dir === "b" && !seen ? tickedFiles.filter((file) => file.kind === "new").length : 0;
  const blockerSentence = blockers.length === 1
    ? `${blockers[0].path.split("/").pop()} ${blockers[0].warning === "possible_secret" ? "may hold a secret" : "is executable"} — mark it reviewed or exclude it.`
    : blockers.length > 1 ? `${blockers.length} files need review — mark them reviewed or exclude them.` : null;
  const disabledReason = overSentence
    ?? blockerSentence
    ?? (unseenNew > 0 ? `Review the ${plural(unseenNew, "new project file")} first` : null)
    ?? (tickedSessions.length + tickedFiles.length === 0 ? "Nothing is selected" : null);

  const onFiles = step === "project_files";
  const stepRows = onFiles ? files.map(fileId) : s.sessions.map((session) => session.id);
  const stepRecommended = onFiles ? recommended : new Set(s.sessions.map((session) => session.id));
  const matchesRecommended = stepRows.every((id) => selected.has(id) === stepRecommended.has(id));
  /* A · C: the files that wait, counted on the button that opens them. */
  const news = (dir === "a" || (dir === "c" && !travel)) && !onFiles ? waiting.length : 0;

  return (
    <>
      <section
        className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace"
        aria-label="Push review"
      >
        <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
          <div className="sync-sync-review-title">
            <h2>Push review</h2>
            <span className="sync-settings-flag">Credential redaction: Off</span>
          </div>
          <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown} />
          <SyncReviewSelectionBar
            count={stepRows.filter((id) => selected.has(id)).length}
            total={stepRows.length}
            recommendedCount={stepRecommended.size}
            recommendedTitle={onFiles ? "Select the project files without a finding" : "Use the recommended session selection"}
            recommendedDisabled={stepRecommended.size === 0 || matchesRecommended}
            onRecommended={() => setSelected((current) => {
              const next = new Set(current);
              stepRows.forEach((id) => (stepRecommended.has(id) ? next.add(id) : next.delete(id)));
              return next;
            })}
            clearDisabled={!stepRows.some((id) => selected.has(id))}
            onClear={() => setSelected((current) => {
              const next = new Set(current);
              // Shipped: clearing the files step keeps files already in storage.
              const keep = new Set(onFiles ? changed.map(fileId) : []);
              stepRows.forEach((id) => { if (!keep.has(id)) next.delete(id); });
              return next;
            })}
            close={{ ariaLabel: "Close push review", onClose: () => undefined }}
          />
        </header>
        <SyncReviewTabs
          tabs={projectReviewTabs(
            ["history", "project_files"],
            { history: tickedSessions.length, project_files: tickedFiles.length },
            new Set<Step>(blockers.length > 0 ? ["project_files"] : []),
            "Sessions",
          )}
          activeId={step}
          onChange={(id) => goTo(id as Step)}
        />
        <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            {!onFiles ? (
              <div className="sync-history-page sync-history-embedded sync-history-selection-review">
                <SessionsStandIn sessions={s.sessions} selected={selected} onToggle={toggle} />
              </div>
            ) : (
              <>
                {dir === "c" && fresh.length > 0 && (
                  <div className="mk-newfiles-rule">
                    <span>New files</span>
                    <div className="sync-segmented" role="group" aria-label="New files in this folder">
                      <button type="button" className={travel ? "" : "active"} aria-pressed={!travel} onClick={() => switchTravel(false)}>Wait for a tick</button>
                      <button type="button" className={travel ? "active" : ""} aria-pressed={travel} onClick={() => switchTravel(true)}>Travel with each push</button>
                    </div>
                  </div>
                )}
                <ProjectFilesReviewPage
                  mode="push"
                  eligibility={{ state: "eligible", reason: "This project folder is not inside a Git work tree." }}
                  rows={files.map((file) => toRow(file, dir !== "before"))}
                  selectedIds={selected}
                  acknowledgedWarningDigests={acknowledged}
                  scanned
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
              </>
            )}
          </div>
        </div>
        <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
          {overSentence && <p className="sync-review-footer-note danger">{overSentence}</p>}
          <ProjectFileCommitImpact
            counts={impact}
            deletionTarget="storage"
            deletionApproved={false}
            disabled={false}
            onDeletionApprovedChange={() => undefined}
          />
          <button
            type="button"
            className={unseenNew > 0 ? "btn" : "btn btn-ghost"}
            title={onFiles ? "Back to session selection" : news > 0 ? `${plural(news, "new file")} not in this push — review project files` : "Review project files next"}
            onClick={() => goTo(onFiles ? "history" : "project_files")}
          >
            {onFiles && <Icon name="chevron-left" size={14} />}
            {onFiles ? "Sessions" : unseenNew > 0 ? `Review ${plural(unseenNew, "file")}` : "Project files"}
            {news > 0 && <span className="sync-path-count">{news} new</span>}
            {!onFiles && <Icon name="chevron-right" size={14} />}
          </button>
          {blockers.length > 0 && (
            <button type="button" className="sync-review-blocker-pill" title={blockerSentence ?? undefined} onClick={() => goTo("project_files")}>
              <Icon name="alert-triangle" size={13} />
              {`${blockers.length} need${blockers.length === 1 ? "s" : ""} review`}
            </button>
          )}
          <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} />
          <PushApplyButton busy={false} disabledReason={disabledReason} onPush={() => undefined} />
        </footer>
      </section>
      <p className="spec-measure">
        Push now carries <b>{plural(tickedSessions.length, "session")}</b>
        {scanned
          ? <> + <b>{plural(tickedFiles.length, "file")}</b>{tickedFiles.length > 0 && ` (${formatBytes(tickedFiles.reduce((sum, file) => sum + file.size, 0))})`} · stay here: <b>{s.files.length - tickedFiles.length}</b></>
          : <> · <i>folder not scanned: no file travels{changed.length > 0 ? `, and ${plural(changed.length, "edit")} to synced files ${changed.length === 1 ? "stays" : "stay"} here` : ""}</i></>}
        {disabledReason && <> · Push refuses: <i>{disabledReason}</i></>}
      </p>
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
    <div className={`spec${knobs.shot === "off" ? "" : " spec-shot"}`}>
      <h1 className="spec-title">Non-Git Push: which project files are ticked for you</h1>
      <p className="spec-sub">
        Drawn on the shipped CSS with the real header, tabs, file cards, counts, gauge and Push button. The Sessions list is a
        stand-in. Every frame is live: switch tabs, tick rows, press <b>Select</b>. The line under each frame says what Push
        would send right now.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Scenario</span>{SCENARIOS.map((id) => seg("scenario", id, SCENARIO_LABEL[id]))}</div>
        <div className="spec-group"><span>Opens on</span>{seg("open", "history", "Sessions")}{seg("open", "project_files", "Project files")}</div>
        <div className="spec-group"><span>Review</span>{(["w684", "w970"] as const).map((w) => seg("width", w, WIDTH_LABEL[w]))}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panels">
        <div className="spec-panel">
          <strong>What ships today</strong>
          <ol>
            <li><b>The scan is lazy.</b> <code>prepare_push_review</code> walks project files only for Git-managed projects. A non-Git folder is walked when the Files tab is opened (<code>scanPendingProjectFiles</code>).</li>
            <li><b>Push from Sessions sends no files.</b> Without a review token the push does not walk the folder (<code>include_project_content = false</code>), so an edit made here to a file already in storage is not captured. Nothing on the Sessions step says so.</li>
            <li><b>Opening the tab ticks every new file.</b> <code>selected_after_scan</code> = new, not blocked, not left out earlier. Findings do not opt out, so <code>.env</code> and <code>run.sh</code> are ticked and Push turns blocked.</li>
            <li><b>Unticked is remembered.</b> A local file left unticked at a push goes to <code>excluded_resource_ids</code> and is not ticked again.</li>
            <li><b>The walk reads every file</b> up to 16 MiB to hash it and look for secrets; larger files are blocked. <code>node_modules</code>, <code>target</code>, <code>dist</code>, <code>build</code>, VCS folders and <code>.gitignore</code> / <code>.mallardignore</code> matches are skipped.</li>
          </ol>
        </div>

        <div className="spec-panel">
          <strong>The default, file by file</strong>
          <table className="spec-table">
            <thead><tr><th /><th>Today</th><th>A</th><th>B</th><th>C</th></tr></thead>
            <tbody>
              <tr><th>Changed here, already in storage</th><td>ticked once the tab is opened; else the edit stays here</td><td className="pick">ticked</td><td>ticked</td><td>ticked</td></tr>
              <tr><th>New, no finding</th><td>ticked once the tab is opened</td><td className="pick">unticked, counted on the step button</td><td>ticked</td><td>unticked; ticked with the switch on</td></tr>
              <tr><th>New, executable or credential-shaped</th><td>ticked; Push blocked until reviewed</td><td className="pick">unticked</td><td>unticked</td><td>unticked</td></tr>
              <tr><th>Left out at an earlier push</th><td>unticked</td><td className="pick">unticked, not counted</td><td>unticked</td><td>unticked</td></tr>
              <tr><th>Push from the Sessions step</th><td>no files, no edits</td><td className="pick">edits travel; new files wait</td><td>waits until Files was shown</td><td>as the switch says</td></tr>
            </tbody>
          </table>
        </div>
      </div>

      <div className="spec-panel">
        <strong>Where other tools draw the line</strong>
        <p>
          Every tool sends a changed file it has sent before. They split on files they have never sent, and the split follows
          two things: whether you look at a list first, and whether what leaves can be taken back. Mallard&rsquo;s Push is
          reviewed on a second tab, is published immutably, and can go to a public Mallard Cloud project, which puts it in the
          third group, where include-everything is the documented source of leaks.
        </p>
        <table className="spec-table">
          <thead><tr><th>Tool</th><th>Changed, sent before</th><th>New files</th><th>Safety net</th></tr></thead>
          {INDUSTRY.map((group) => (
            <tbody key={group.title}>
              <tr><td colSpan={4} className="spec-table-group">{group.title}</td></tr>
              {group.rows.map((row) => (
                <tr key={row.tool}>
                  <th><a href={row.href} target="_blank" rel="noreferrer">{row.tool}</a></th>
                  <td><span className={VERDICT_CLASS[row.known[0]]}>{row.known[1]}</span></td>
                  <td><span className={VERDICT_CLASS[row.fresh[0]]}>{row.fresh[1]}</span></td>
                  <td>{row.net}</td>
                </tr>
              ))}
            </tbody>
          ))}
        </table>
        <ul>
          {GUIDANCE.map((item) => (
            <li key={item.source}><a href={item.href} target="_blank" rel="noreferrer"><b>{item.source}</b></a>: {item.says}</li>
          ))}
        </ul>
      </div>

      <div className="spec-stack" style={{ gridTemplateColumns: `repeat(auto-fit, ${width}px)` }}>
        {dirs.map((dir) => (
          <div key={dir} style={{ width }}>
            <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "a" && <em>recommended</em>}</h2>
            <p className="spec-col-note">{DIR_NOTE[dir]}</p>
            <div className="spec-frame">
              <Review key={`${knobs.scenario}-${dir}-${knobs.open}`} dir={dir} s={s} open={knobs.open} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
