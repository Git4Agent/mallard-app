import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import { NoticeZone } from "./components/notice/Notice";
import { projectReviewTabs } from "./components/project-sync/reviewSteps";
import NameSearchField from "./components/sync-review/NameSearchField";
import { sessionNoun } from "./components/project-sync/model";
import { ReviewBudgetGauge, ReviewBudgetGlance } from "./components/sync-review/ReviewBudgetMeter";
import SyncPathBar, { type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import SyncReviewTabs from "./components/sync-review/SyncReviewTabs";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import "./App.css";

/**
 * Mockup: the "Back to …" buttons an empty review grew, redrawn in the
 * grammar the rest of the app already speaks.
 *
 * Three ship uncommitted, each a ghost + chevron-left in the review footer,
 * shown only when the review has nothing to commit:
 *   PushResourceWorkspace  "Back to Git history" / "Back to project"
 *   StorageCatalogPage     "Back to Library" (Push) and "Back to Library" (Pull)
 * Project Pull has none on purpose: its empty primary is a live
 * "Mark as reviewed", which already leaves.
 *
 *   Before  what ships (uncommitted)
 *   A       the exit takes the commit's slot while there is nothing to commit
 *   B       the exit sits in the "Nothing to push" row, beside its reason
 *   C       control: same slot, same ghost, label and glyph fixed only
 *
 * `npm run dev`, then /back-to-mockup.html
 * (`?scenario=git|git-held|nongit|nongit-files|lib-push|lib-push-held|lib-pull&dir=a|b|c&label=place|done&view=directions|context&width=wide|narrow&theme=light|dark`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type ScenarioId = "git" | "git-held" | "nongit" | "nongit-files" | "lib-push" | "lib-push-held" | "lib-pull";
type Dir = "before" | "a" | "b" | "c";
type LabelMode = "place" | "done";
type Width = "wide" | "narrow";
type View = "directions" | "context";
interface Knobs { scenario: ScenarioId; dir: Dir; label: LabelMode; width: Width; view: View; theme: "light" | "dark" }

const SCENARIOS: readonly ScenarioId[] = ["git", "git-held", "nongit", "nongit-files", "lib-push", "lib-push-held", "lib-pull"];
const DIRS: readonly Dir[] = ["before", "a", "b", "c"];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scenario: pick("scenario", SCENARIOS, "git"),
    dir: pick("dir", ["a", "b", "c"] as const, "a"),
    label: pick("label", ["place", "done"] as const, "place"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    view: pick("view", ["directions", "context"] as const, "directions"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const SCENARIO_LABEL: Record<ScenarioId, string> = {
  git: "Push · Git project",
  "git-held": "Push · Git, held for Pull",
  nongit: "Push · non-Git, Sessions tab",
  "nongit-files": "Push · non-Git, Files tab",
  "lib-push": "Catalog Push",
  "lib-push-held": "Catalog Push, held for Pull",
  "lib-pull": "Catalog Pull",
};

const DIR_LABEL: Record<Dir, string> = {
  before: "Before (uncommitted)",
  a: "A · The exit takes the commit’s slot",
  b: "B · The exit sits beside the reason",
  c: "C · Control: fix it in place",
};

const DIR_NOTE: Record<Dir, ReactNode> = {
  before: <>A ghost <b>‹ Back to …</b> added to the footer, right-pinned beside a disabled gold primary. It is the step navigator’s exact look (<code>btn btn-ghost</code> + <code>chevron-left</code>).</>,
  a: <>With nothing to commit, the footer’s one button leaves. It is a box <code>.btn</code> (not gold, since nothing is being committed; not ghost, since the ghost is the step navigator), in the apply button’s footprint, with no chevron. The dead primary is hidden, and so is the <b>‹ Sessions / Project files ›</b> step button: both tabs read 0 and the tab strip above already switches. The path bar keeps its disabled <b>Push</b> as the review’s verb. This follows Settings’ <b>Done</b>: nothing to commit, so the one button leaves.</>,
  b: <>The <b>Nothing to push</b> row gets the exit as its trailing action, the way <b>Try again</b> ends the history’s error row and <b>Back to Storages</b> sits in the empty storage picker. The footer is back to commit-only, the same as before the Back work, so the dead gold primary stays. In the held-for-Pull states this is B’s strongest case: the reason, <b>Open Pull review</b> and the exit sit together, and the footer doesn’t outshout the notice.</>,
  c: <>The smallest diff: the same slot and the same ghost, with the chevron dropped and the label corrected. This is the baseline the other two are measured against.</>,
};

/* ——— data ——————————————————————————————————————————————————————— */

const MB = 1024 * 1024;
const PROJECT = "echo-park-horror";
const STORAGE = "Attic R2";
const SCOPE = "Skills";
const PROFILE = "Codex";
const RESERVED = 0.4 * MB;

interface Scenario {
  id: ScenarioId;
  surface: "project" | "catalog";
  verb: "push" | "pull";
  git: boolean;
  step: "history" | "project_files";
  held: boolean;
}

function scenario(id: ScenarioId): Scenario {
  const catalog = id.startsWith("lib-");
  return {
    id,
    surface: catalog ? "catalog" : "project",
    verb: id === "lib-pull" ? "pull" : "push",
    git: id === "git" || id === "git-held",
    step: id === "nongit-files" ? "project_files" : "history",
    held: id.endsWith("-held"),
  };
}

/** The landing page, as the trail names it. */
function landing(s: Scenario): string {
  if (s.surface === "catalog") return SCOPE;
  return s.git ? "Git history" : PROJECT;
}

function exitLabel(dir: Dir, s: Scenario, mode: LabelMode): string {
  if (dir === "before") {
    if (s.surface === "catalog") return "Back to Library";
    return s.git ? "Back to Git history" : "Back to project";
  }
  return mode === "done" ? "Done" : `Back to ${landing(s)}`;
}

function exitTitle(s: Scenario): string {
  return `Close ${s.verb === "push" ? "Push" : "Pull"} review — back to ${landing(s)}`;
}

function reviewTitle(s: Scenario): string {
  return s.verb === "push" ? "Push review" : "Pull review";
}

/* ——— pieces ——————————————————————————————————————————————————————— */

function EmptyRow({ s, exit }: { s: Scenario; exit: ReactNode }) {
  const [strong, small] = s.surface === "catalog"
    ? s.verb === "push"
      ? ["Nothing to push", `This Mac already matches ${STORAGE}.`]
      : ["Nothing to pull", `These ${SCOPE.toLowerCase()} already match ${STORAGE}.`]
    : s.step === "project_files"
      ? ["No changes", null]
      : ["Nothing to push", "No local session changes are ready to push."];
  return (
    <div className={`sync-pull-empty${exit ? " mk-with-exit" : ""}`}>
      <Icon name="check-circle" size={16} />
      <span>
        <strong>{strong}</strong>
        {small && <small>{small}</small>}
      </span>
      {exit}
    </div>
  );
}

function HeldBanner({ s }: { s: Scenario }) {
  if (!s.held) return null;
  const openPull = <button type="button" className="btn btn-ghost">Open Pull review</button>;
  return (
    <div className="sync-review-banners">
      {s.surface === "project" ? (
        <NoticeZone
          timing="standing"
          label="Notes about this Push review"
          subject="notes about this Push review"
          items={[{
            key: "held",
            severity: "warning",
            mark: <ThreadStatusIcon status="divergent" size={16} />,
            title: "2 conversations held for Pull",
            message: "Storage has newer versions. Selected threads still push.",
            action: openPull,
          }]}
        />
      ) : (
        <NoticeZone
          timing="standing"
          label="Notes about this Push review"
          subject="notes about this Push review"
          items={[{
            key: "held",
            severity: "warning",
            title: `2 ${SCOPE.toLowerCase()} held for Pull`,
            message: `${STORAGE} has versions this Mac has not reviewed, so their local changes cannot push yet. Selected ${SCOPE.toLowerCase()} still push.`,
            action: openPull,
          }]}
        />
      )}
    </div>
  );
}

function Body({ s, rowExit }: { s: Scenario; rowExit: ReactNode }) {
  if (s.surface === "catalog") {
    return (
      <>
        <HeldBanner s={s} />
        <NameSearchField noun={() => SCOPE.toLowerCase()} query="" onChange={() => undefined} label={`Search ${SCOPE.toLowerCase()}`} matches={0} total={0} />
        <div className="sync-history-thread-list">
          <EmptyRow s={s} exit={rowExit} />
        </div>
      </>
    );
  }
  if (s.step === "project_files") {
    return (
      <section className="sync-project-files-page" aria-label="Project files">
        <div className="sync-project-files-search">
          <input type="search" placeholder="Search files…" aria-label="Search project file paths" disabled />
          <button type="button" className="btn btn-ghost"><Icon name="refresh" size={14} /> Rescan</button>
        </div>
        <EmptyRow s={s} exit={rowExit} />
      </section>
    );
  }
  return (
    <>
      <HeldBanner s={s} />
      <div className="sync-history-page sync-history-embedded sync-history-selection-review">
        <section className="sync-history-content" aria-label="Sessions">
          <NameSearchField noun={sessionNoun} query="" onChange={() => undefined} label="Search sessions" matches={0} total={0} />
          <EmptyRow s={s} exit={rowExit} />
        </section>
      </div>
    </>
  );
}

function Review({ dir, s, mode }: { dir: Dir; s: Scenario; mode: LabelMode }) {
  const project = s.surface === "project";
  const push = s.verb === "push";
  const budget = { total: RESERVED, limit: 512 * MB };
  const breakdown = "Sessions 0 B · Requirements 410 KB · Project files 0 B";
  const label = exitLabel(dir, s, mode);
  const title = exitTitle(s);

  /* The exits, one per direction. */
  const ghostExit = (
    <button type="button" className="btn btn-ghost" title={dir === "before" ? undefined : title}>
      {dir === "before" && <Icon name="chevron-left" size={14} />}
      {label}
    </button>
  );
  const boxExit = (
    <button type="button" className="btn mk-exit" title={title}>
      <span>{label}</span>
    </button>
  );
  const rowExit = dir === "b" ? (
    <button type="button" className="btn" title={title}>
      <span>{label}</span>
    </button>
  ) : null;

  /* The footer's step button walks between the review's tabs. In an empty
     review both tabs read 0 and the tab strip above already switches, so
     every proposed direction drops it: an empty footer holds one button. */
  const stepNavigator = project && !s.git && dir === "before" ? (
    <button type="button" className="btn btn-ghost" title={s.step === "history" ? "Review project files next" : "Back to session selection"}>
      {s.step !== "history" && <Icon name="chevron-left" size={14} />}
      {s.step === "history" ? "Project files" : "Sessions"}
      {s.step === "history" && <Icon name="chevron-right" size={14} />}
    </button>
  ) : null;
  const deadPrimary = (
    <button type="button" className="btn btn-primary sync-pull-apply-button" disabled title={`Nothing to ${s.verb}`}>
      <ThreadStatusIcon status={push ? "local-outstanding" : "cloud-outstanding"} size={16} />
      {project ? "Push" : `${push ? "Push" : "Pull"} 0`}
    </button>
  );
  /* Every proposed direction drops the meter from an empty review: with
     nothing selectable it only measures the reserved requirements of a push
     that cannot happen. Before draws it as it ships. */
  const meter = project && dir === "before";
  const gauge = meter ? <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} /> : null;

  let footer: ReactNode;
  if (dir === "a") {
    footer = <>{stepNavigator}{boxExit}</>;
  } else if (dir === "b") {
    footer = <>{stepNavigator}{gauge}{deadPrimary}</>;
  } else {
    footer = <>{ghostExit}{stepNavigator}{gauge}{deadPrimary}</>;
  }

  return (
    <section
      className={`sync-inline-action-review ${push ? "sync-inline-push-review" : "sync-inline-pull-review"} sync-sync-review-workspace`}
      aria-label={reviewTitle(s)}
    >
      <header className={`sync-inline-action-header${project ? " sync-push-resource-header" : ""} sync-sync-review-header`}>
        <div className="sync-sync-review-title">
          <h2>{reviewTitle(s)}</h2>
          {project && <span className="sync-settings-flag">Credential redaction: On</span>}
        </div>
        {meter && <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown} />}
        <SyncReviewSelectionBar
          count={0}
          total={0}
          recommendedCount={0}
          recommendedTitle="Use the recommended selection (0)"
          recommendedDisabled
          onRecommended={() => undefined}
          clearDisabled
          onClear={() => undefined}
          close={{ ariaLabel: `Close ${reviewTitle(s).toLowerCase()}`, onClose: () => undefined }}
        />
      </header>
      {project && !s.git && (
        <SyncReviewTabs
          tabs={projectReviewTabs(["history", "project_files"], { history: 0, project_files: 0 }, new Set(), "Sessions")}
          activeId={s.step}
          onChange={() => undefined}
        />
      )}
      <div className={`sync-inline-action-content${project ? " sync-push-resource-content" : ""} sync-sync-review-content`}>
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <Body s={s} rowExit={rowExit} />
        </div>
      </div>
      <footer className={`sync-inline-action-footer${project ? " sync-push-resource-footer" : ""} sync-sync-review-footer`}>
        {footer}
      </footer>
    </section>
  );
}

/* ——— the page around it ——————————————————————————————————————————— */

function PathActions({ s }: { s: Scenario }) {
  const verbButton = (kind: "pull" | "push") => {
    const mine = kind === s.verb;
    return (
      <button type="button" className={`sync-path-sync${mine ? " active" : ""}`} disabled aria-expanded={mine}>
        <Icon name={kind === "pull" ? "download" : "upload"} size={14} />
        <span className="sync-path-sync-label">{kind === "pull" ? "Pull" : "Push"}</span>
        {mine && s.surface === "catalog" && <span className="sync-path-count" aria-hidden="true">0</span>}
      </button>
    );
  };
  return (
    <>
      <button type="button" className="sync-path-icon-action" aria-label="Refresh" title="Refresh">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        {verbButton("pull")}
        {verbButton("push")}
      </div>
    </>
  );
}

function crumbs(s: Scenario): PathBarCrumb[] {
  const storage: PathBarCrumb = { id: "storage", label: STORAGE, icon: "storage-r2", tone: "root" };
  if (s.surface === "catalog") {
    return [storage, { id: "scope", label: `${SCOPE} · ${PROFILE}`, icon: "lightbulb" }];
  }
  return [
    storage,
    { id: "project", label: PROJECT, icon: s.git ? "git-folder" : "folder" },
    ...(s.git ? [{ id: "section", label: "Git history", tone: "section" as const }] : []),
  ];
}

function InApp({ dir, s, mode }: { dir: Dir; s: Scenario; mode: LabelMode }) {
  const review = <Review dir={dir} s={s} mode={mode} />;
  return (
    <>
      <SyncPathBar crumbs={crumbs(s)} actions={<PathActions s={s} />} ariaLabel="Path" />
      {s.surface === "catalog" ? (
        <main className="sync-main sync-project-links-page sync-sync-review-page">{review}</main>
      ) : (
        <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
          <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
            <div className="profile-links-list sync-combined-settings-panel">
              <article className="profile-link-card sync-project-link-card">
                <div className="profile-link-connections">
                  <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                    <div className="project-profile-storage-list">
                      <div className="storage-link-block selected sync-review-open">
                        <div className="sync-storage-inline-review" tabIndex={-1}>{review}</div>
                      </div>
                    </div>
                  </section>
                </div>
              </article>
            </div>
          </section>
        </main>
      )}
    </>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

/** The main pane beside a 318px sidebar at the 1100px default window and the
 *  800px minimum, and the review inside it (measured for the Select mockup). */
const PANE_WIDTH: Record<Width, number> = { wide: 782, narrow: 482 };
const REVIEW_WIDTH: Record<Width, number> = { wide: 684, narrow: 414 };

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
  const frameClass = `spec-frame${knobs.width === "narrow" ? " narrow" : ""}`;
  return (
    <div className="spec">
      <h1 className="spec-title">Empty review: the “Back to …” exit, in the app’s own grammar</h1>
      <p className="spec-sub">
        Drawn on the shipped CSS with the real path bar, header, Select pill, tabs, meter, notices and search field. Only the
        exit is new (mock CSS in the page’s <code>&lt;style&gt;</code>). Every frame is a review with <b>nothing to commit</b>,
        the only state where the button appears.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Review</span>{SCENARIOS.map((id) => seg("scenario", id, SCENARIO_LABEL[id]))}</div>
        <div className="spec-group"><span>View</span>{seg("view", "directions", "All directions")}{seg("view", "context", "In the app")}</div>
        {knobs.view === "context" && (
          <div className="spec-group"><span>Direction</span>{DIRS.filter((dir) => dir !== "before").map((dir) => seg("dir", dir, DIR_LABEL[dir].split(" · ")[0]))}</div>
        )}
        <div className="spec-group"><span>Label</span>{seg("label", "place", "Back to ‹page›")}{seg("label", "done", "Done")}</div>
        <div className="spec-group"><span>Pane</span>{seg("width", "wide", "1100 px window")}{seg("width", "narrow", "800 px window")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <div className="spec-panel">
        <strong>Why the shipped button clashes</strong>
        <ol>
          <li><b>It wears the step navigator’s exact look.</b> Both are <code>btn btn-ghost</code> with a <code>chevron-left</code> in the same footer, so an exit reads as a step. A non-Git empty Push shows both (<code>showProjectFiles</code> is always true there). On the Files tab that’s two left chevrons side by side: <i>‹ Back to project ‹ Sessions</i>.</li>
          <li><b>Two of its labels name places the screen never shows.</b> The UI calls the Library <b>Catalog</b> (sidebar, index crumb), and the page you land on is headed <b>{SCOPE}</b>, with the crumb <b>{SCOPE} · {PROFILE}</b>. A non-Git review lands on the project, whose crumb reads <b>{PROJECT}</b>, not “project”.</li>
          <li><b>The loudest control is dead.</b> The only other button in an empty footer is the disabled gold primary (<i>Push</i>, <i>Push 0</i>, <i>Pull 0</i>), 180px wide. On project Push a 410 KB “requirements only” meter, in both the header and the footer, still measures a push that can’t happen. <b>Every proposed direction drops both meters from an empty review.</b></li>
          <li><b>The reason and the way out are a panel apart.</b> <i>Nothing to push</i> sits at the top of the list. The exit is pinned to the far right of the footer.</li>
          <li><b>The gate fires early on non-Git.</b> A non-Git Push opens with <code>projectContentScanned: false</code>, and <code>emptyReview</code> doesn’t wait for the scan. So the button can appear (disabled, since <code>busy</code> includes the scan) and then vanish when files land. Every direction adds the scan to the gate.</li>
          <li>Out of scope but visible here: Catalog Push held for Pull says <i>2 skills held for Pull</i> <b>and</b> <i>This Mac already matches {STORAGE}</i>. The empty row’s copy ignores held rows.</li>
        </ol>
      </div>

      {knobs.view === "directions" ? (
        <div className="spec-grid">
          {DIRS.map((dir) => (
            <div key={dir} style={{ width: REVIEW_WIDTH[knobs.width] }}>
              <h2 className="spec-col-title">{DIR_LABEL[dir]}{dir === "a" && <em>recommended</em>}</h2>
              <p className="spec-col-note">{DIR_NOTE[dir]}</p>
              <div className={frameClass} style={{ width: REVIEW_WIDTH[knobs.width] }}>
                <Review dir={dir} s={s} mode={knobs.label} />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="spec-grid">
          {(["before", knobs.dir] as const).map((dir) => (
            <div key={dir} style={{ width: PANE_WIDTH[knobs.width] }}>
              <h2 className="spec-col-title">{DIR_LABEL[dir]}</h2>
              <p className="spec-col-note">{DIR_NOTE[dir]}</p>
              <div className={frameClass} style={{ width: PANE_WIDTH[knobs.width] }}>
                <InApp dir={dir} s={s} mode={knobs.label} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
