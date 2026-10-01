import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import NameSearchField from "./components/sync-review/NameSearchField";
import ProviderMark from "./components/sync-review/ProviderMark";
import { ReviewBudgetGauge, ReviewBudgetGlance } from "./components/sync-review/ReviewBudgetMeter";
import SyncPathBar, { type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import "./App.css";

/**
 * Mockup, round 3 (owner picked "B · on the timeline"): five ways to mark the
 * commits a Push adds, all on the review's session rail. No Git row, no
 * sentences; words live in tooltips.
 *
 *   Before  what ships (uncommitted)
 *   B1      ↑ chip on each new commit
 *   B2      new commits take the sessions' "only on this computer" icon
 *   B3      a storage tag on the commit storage already has (git log --decorate)
 *   B4      a divider between new and stored commits
 *   B5      the spine turns the push colour along the new stretch
 *
 * `npm run dev`, then /git-history-push-mockup.html
 * (`?scenario=uptodate|gitonly|both|branch|rewritten&view=directions|context&dir=b1..b5&width=wide|narrow&theme=light|dark&shot=before|b1..b5`).
 */

/* ——— knobs ———————————————————————————————————————————————————————— */

type ScenarioId = "uptodate" | "gitonly" | "both" | "branch" | "rewritten";
type Variant = "b1" | "b2" | "b3" | "b4" | "b5";
type Dir = "before" | Variant;
type Width = "wide" | "narrow";
type View = "directions" | "context";
interface Knobs { scenario: ScenarioId; dir: Variant; width: Width; view: View; theme: "light" | "dark" }

const SCENARIOS: readonly ScenarioId[] = ["uptodate", "gitonly", "both", "branch", "rewritten"];
const VARIANTS: readonly Variant[] = ["b1", "b2", "b3", "b4", "b5"];
const DIRS: readonly Dir[] = ["before", ...VARIANTS];

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    scenario: pick("scenario", SCENARIOS, "both"),
    dir: pick("dir", VARIANTS, "b1"),
    width: pick("width", ["wide", "narrow"] as const, "wide"),
    view: pick("view", ["directions", "context"] as const, "directions"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const SCENARIO_LABEL: Record<ScenarioId, string> = {
  uptodate: "Up to date",
  gitonly: "New commits",
  both: "New commits + sessions",
  branch: "Other branch",
  rewritten: "Rewritten",
};

const DIR_LABEL: Record<Dir, string> = {
  before: "Before",
  b1: "B1 · ↑ chip",
  b2: "B2 · Push icon",
  b3: "B3 · Storage tag",
  b4: "B4 · Divider",
  b5: "B5 · Push spine",
};

const DIR_NOTE: Record<Dir, string> = {
  before: "Ships today.",
  b1: "A ↑ chip on each new commit.",
  b2: "New commits wear the sessions’ upload icon.",
  b3: "A tag where storage is. Above it is new.",
  b4: "A rule between new and stored.",
  b5: "The rail turns blue along new commits.",
};

/* ——— data ——————————————————————————————————————————————————————— */

const MB = 1024 * 1024;
const PROJECT = "mallard-with-pelican";
const STORAGE = "Attic R2";
const RESERVED = 0.4 * MB;

interface Commit { sha: string; subject: string; when: string }

const C1: Commit = { sha: "1787e59c4531a0f7d2b6e3c19a84f5d07b2c6e11", subject: "Add mallard carrying pelican artwork", when: "Sep 29, 21:14" };
const C2: Commit = { sha: "53414b33bb13e8a2c47f9d061b5e2a3c8d7f4e90", subject: "Merge pull request #1 from FuyaoLi2017/codex/fuyao-test", when: "Sep 29, 18:02" };
const C3: Commit = { sha: "3d91603aab13c5e7f80b2d4a6c9e1f3b5d7a9c02", subject: "Add mallard and pelican story and artwork", when: "Sep 28, 23:40" };
/** The pushed version of C1 before `git commit --amend`. */
const AMENDED: Commit = { sha: "e41b9d07c2a85f3e19b0d4c6a7e8f2b1c3d5e7f9", subject: "Add mallard carrying pelican artwork", when: "Sep 29, 20:55" };

type Provider = "claude" | "codex";
const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude Code", codex: "Codex" };

interface MockSession {
  id: string;
  title: string;
  provider: Provider;
  commit: string;
  bytes: number;
  when: string;
  messages: number;
  tools: number;
  tokens: string;
  length: string;
}

const S1: MockSession = { id: "s1", title: "Draw the mallard carrying the pelican", provider: "claude", commit: C1.sha, bytes: 3.2 * MB, when: "Sep 29, 20:41", messages: 64, tools: 51, tokens: "412K", length: "48m" };
const S2: MockSession = { id: "s2", title: "Write the mallard and pelican story", provider: "codex", commit: C3.sha, bytes: 1.1 * MB, when: "Sep 28, 22:05", messages: 22, tools: 9, tokens: "96K", length: "17m" };

type RefState = "uptodate" | "ahead" | "branch" | "rewritten";

interface Scenario {
  id: ScenarioId;
  branch: string;
  commits: Commit[];
  stored: { branch: string; head: string };
  /** Leading commits storage does not have. */
  newCount: number;
  state: RefState;
  sessions: MockSession[];
}

function scenario(id: ScenarioId): Scenario {
  const commits = [C1, C2, C3];
  switch (id) {
    case "uptodate":
      return { id, branch: "main", commits, stored: { branch: "main", head: C1.sha }, newCount: 0, state: "uptodate", sessions: [] };
    case "gitonly":
      return { id, branch: "main", commits, stored: { branch: "main", head: C3.sha }, newCount: 2, state: "ahead", sessions: [] };
    case "both":
      return { id, branch: "main", commits, stored: { branch: "main", head: C3.sha }, newCount: 2, state: "ahead", sessions: [S1, S2] };
    case "branch":
      return { id, branch: "pelican-art", commits, stored: { branch: "main", head: C1.sha }, newCount: 0, state: "branch", sessions: [] };
    case "rewritten":
      return { id, branch: "main", commits, stored: { branch: "main", head: AMENDED.sha }, newCount: 1, state: "rewritten", sessions: [] };
  }
}

const changed = (s: Scenario) => s.state !== "uptodate";
const short = (sha: string) => sha.slice(0, 7);

function newTitle(s: Scenario): string {
  return s.state === "rewritten"
    ? `Replaces ${short(s.stored.head)} in ${STORAGE} (history rewritten)`
    : `New since the last Push to ${STORAGE}`;
}

function tagTitle(s: Scenario): string {
  if (s.state === "branch") return `${STORAGE} has ${s.stored.branch}. This Push sends ${s.branch}.`;
  if (s.state === "rewritten") return `${STORAGE} has ${short(s.stored.head)}, which is no longer on ${s.branch}`;
  return `${STORAGE} has history up to here`;
}

/* ——— the storage marks (B3 tag, B4 divider) ——————————————————————— */

function StorageTag({ s }: { s: Scenario }) {
  const warn = s.state === "rewritten";
  return (
    <span className={`sync-history-branch-state mk-tag${warn ? " history-only" : ""}`} title={tagTitle(s)}>
      <Icon name="storage-r2" size={11} />
      {STORAGE}
      {s.state === "branch" && <span className="mk-tag-branch">{s.stored.branch}</span>}
    </span>
  );
}

function Divider({ s }: { s: Scenario }) {
  return (
    <li className={`mk-divider${s.state === "rewritten" ? " warn" : ""}`} aria-label={tagTitle(s)}>
      <StorageTag s={s} />
    </li>
  );
}

/* ——— the session rail ——————————————————————————————————————————— */

function SessionRow({ session }: { session: MockSession }) {
  return (
    <article className="sync-history-thread-card sync-sync-selectable-row selected">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <label className="sync-sync-row-choice" title={`Exclude ${session.title}`}>
            <input type="checkbox" defaultChecked aria-label={`Exclude ${session.title}`} />
          </label>
          <span className="sync-thread-sync-indicator local" aria-label="Only on this computer" tabIndex={0}>
            <ThreadStatusIcon status="local-outstanding" size={18} />
          </span>
          <strong>{session.title}</strong>
        </div>
        <div className="sync-history-thread-actions">
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

interface RailItem { commit: Commit; outgoing: boolean; anchor: boolean; ghost: boolean }

/** What the rail draws. Shipped: commits with sessions. Every variant adds
 *  the outgoing commits; B3/B4 also draw the commit storage stops at (or, if
 *  history was rewritten, the replaced commit as a ghost). */
function railItems(s: Scenario, variant: Variant | null): RailItem[] {
  const anchored = variant === "b3" || variant === "b4";
  const items: RailItem[] = [];
  s.commits.forEach((commit, index) => {
    const outgoing = variant != null && index < s.newCount;
    if (anchored && s.state === "rewritten" && index === s.newCount) {
      items.push({ commit: AMENDED, outgoing: false, anchor: true, ghost: true });
    }
    const anchor = anchored && s.state !== "rewritten" && index === s.newCount;
    const sessions = s.sessions.some((session) => session.commit === commit.sha);
    if (sessions || outgoing || anchor) items.push({ commit, outgoing, anchor, ghost: false });
  });
  return items;
}

function Rail({ s, variant }: { s: Scenario; variant: Variant | null }) {
  const items = railItems(s, variant);
  if (items.length === 0) return null;
  const warn = s.state === "rewritten";
  return (
    <div className="sync-history-timeline">
      <section className="sync-history-commit-section" aria-label={`First-parent commits on ${s.branch}`}>
        <ol className="sync-history-commit-rail">
          {items.map(({ commit, outgoing, anchor, ghost }) => {
            const rows = ghost ? [] : s.sessions.filter((session) => session.commit === commit.sha);
            const spine = variant === "b5" && outgoing;
            const liClass = [
              "sync-history-commit",
              ghost ? "mk-ghost" : "",
              spine ? `mk-spine${warn ? " warn" : ""}` : "",
            ].filter(Boolean).join(" ");
            const icon = variant === "b2" && outgoing
              ? <ThreadStatusIcon status={warn ? "divergent" : "local-outstanding"} size={16} />
              : <Icon name="git-commit" size={14} />;
            return [
              variant === "b4" && anchor ? <Divider key={`divider-${commit.sha}`} s={s} /> : null,
              <li key={commit.sha} className={liClass} title={outgoing ? newTitle(s) : ghost ? tagTitle(s) : undefined}>
                {rows.length > 0 ? (
                  <button type="button" className="sync-history-spine-fold" aria-label={`Collapse commit ${short(commit.sha)}`} aria-expanded>
                    <Icon name="chevron-down" size={12} />
                  </button>
                ) : (
                  <span className="sync-history-spine-fold inert" aria-hidden="true"><Icon name="chevron-right" size={12} /></span>
                )}
                <div className="sync-history-commit-heading">
                  <span className="sync-history-commit-title">
                    <span className={`sync-history-commit-icon${variant === "b2" && outgoing ? " mk-push-icon" : ""}`} aria-hidden="true">{icon}</span>
                    <strong title={commit.subject}>{commit.subject}</strong>
                  </span>
                  <span className="sync-history-commit-meta">
                    {variant === "b1" && outgoing && (
                      <span className={`sync-history-branch-state mk-chip${warn ? " history-only" : ""}`}>↑</span>
                    )}
                    {variant === "b3" && anchor && <StorageTag s={s} />}
                    <code title={commit.sha}>{short(commit.sha)}</code>
                    <time>{commit.when}</time>
                  </span>
                </div>
                {rows.length > 0 && (
                  <div className="sync-history-thread-list">
                    {rows.map((session) => <SessionRow key={session.id} session={session} />)}
                  </div>
                )}
              </li>,
            ];
          })}
        </ol>
      </section>
    </div>
  );
}

function EmptyRow({ s, dir }: { s: Scenario; dir: Dir }) {
  if (s.sessions.length > 0) return null;
  if (dir === "before") {
    return (
      <div className="sync-pull-empty">
        <Icon name="check-circle" size={16} />
        <span><strong>Nothing to push</strong><small>No local session changes are ready to push.</small></span>
      </div>
    );
  }
  return (
    <div className="sync-pull-empty mk-empty">
      <Icon name="check-circle" size={16} />
      <span><strong>{changed(s) ? "No session changes" : "Nothing to push"}</strong></span>
    </div>
  );
}

/** Before: the shipped band, verbatim. */
function ShippedBlock({ s }: { s: Scenario }) {
  return (
    <details className="sync-push-git-history" open>
      <summary>Git history: {s.branch} · {s.commits.length.toLocaleString()} commits{changed(s) ? " · changed" : ""}</summary>
      <p>This Push includes the Git history shown here. Sessions are placed at the closest commit at or after their last prompt.</p>
      <ol>
        {s.commits.slice(0, 20).map((commit) => (
          <li key={commit.sha}><code>{commit.sha.slice(0, 12)}</code> {commit.subject || "Untitled commit"}</li>
        ))}
      </ol>
    </details>
  );
}

function Review({ dir, s }: { dir: Dir; s: Scenario }) {
  const variant = dir === "before" ? null : dir;
  const sessionBytes = s.sessions.reduce((sum, session) => sum + session.bytes, 0);
  const budget = { total: RESERVED + sessionBytes, limit: 512 * MB };
  const breakdown = `Sessions ${(sessionBytes / MB).toFixed(1)} MB · Requirements 410 KB`;
  const empty = s.sessions.length === 0 && !changed(s);
  /* Nothing to search, so no search field — proposed directions only. */
  const search = dir === "before" || s.sessions.length > 0;

  return (
    <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-label="Push review">
      <header className="sync-inline-action-header sync-push-resource-header sync-sync-review-header">
        <div className="sync-sync-review-title">
          <h2>Push review</h2>
          <span className="sync-settings-flag">Credential redaction: Off</span>
        </div>
        {!empty && <ReviewBudgetGlance budget={budget} verb="push" breakdown={breakdown} />}
        <SyncReviewSelectionBar
          count={s.sessions.length}
          total={s.sessions.length}
          recommendedCount={s.sessions.length}
          recommendedTitle="Use the recommended session selection"
          recommendedDisabled
          onRecommended={() => undefined}
          clearDisabled={s.sessions.length === 0}
          onClear={() => undefined}
          close={{ ariaLabel: "Close push review", onClose: () => undefined }}
        />
      </header>

      {dir === "before" && <ShippedBlock s={s} />}

      <div className="sync-inline-action-content sync-push-resource-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section className="sync-history-content" aria-label="Sessions">
              {search && <NameSearchField query="" onChange={() => undefined} label="Search sessions" matches={s.sessions.length} total={s.sessions.length} />}
              <EmptyRow s={s} dir={dir} />
              <Rail s={s} variant={variant} />
            </section>
          </div>
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer">
        {empty ? (
          <button type="button" className="btn sync-review-exit" title="Close Push review — back to Git history">
            <span>Back to Git history</span>
          </button>
        ) : (
          <>
            <ReviewBudgetGauge budget={budget} verb="push" breakdown={breakdown} />
            <button type="button" className="btn btn-primary sync-pull-apply-button">
              <ThreadStatusIcon status="local-outstanding" size={16} />
              Push
            </button>
          </>
        )}
      </footer>
    </section>
  );
}

/* ——— the page around it ——————————————————————————————————————————— */

function InApp({ dir, s }: { dir: Dir; s: Scenario }) {
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: STORAGE, icon: "storage-r2", tone: "root" },
    { id: "project", label: PROJECT, icon: "git-folder" },
    { id: "section", label: "Git history", tone: "section" },
  ];
  const actions = (
    <>
      <button type="button" className="sync-path-icon-action" aria-label="Refresh">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" disabled><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
        <button type="button" className="sync-path-sync active" disabled aria-expanded><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
      </div>
    </>
  );
  return (
    <>
      <SyncPathBar crumbs={crumbs} actions={actions} ariaLabel="Path" />
      <main className="sync-main sync-project-links-page sync-project-workspace-page sync-project-combined-page sync-sync-review-page">
        <section className="profile-links-section sync-project-overview" aria-label={PROJECT}>
          <div className="profile-links-list sync-combined-settings-panel">
            <article className="profile-link-card sync-project-link-card">
              <div className="profile-link-connections">
                <section className="project-storage-card" aria-label={`${PROJECT} storage`}>
                  <div className="project-profile-storage-list">
                    <div className="storage-link-block selected sync-review-open">
                      <div className="sync-storage-inline-review" tabIndex={-1}><Review dir={dir} s={s} /></div>
                    </div>
                  </div>
                </section>
              </div>
            </article>
          </div>
        </section>
      </main>
    </>
  );
}

/* ——— the specimen ————————————————————————————————————————————————— */

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
  const frame = (dir: Dir): ReactNode => {
    const width = knobs.view === "context" ? PANE_WIDTH[knobs.width] : REVIEW_WIDTH[knobs.width];
    return (
      <div key={dir} style={{ width }}>
        <h2 className="spec-col-title">{DIR_LABEL[dir]}<span>{DIR_NOTE[dir]}</span></h2>
        <div className={frameClass} style={{ width }}>
          {knobs.view === "context" ? <InApp dir={dir} s={s} /> : <Review dir={dir} s={s} />}
        </div>
      </div>
    );
  };

  /* `?shot=<dir>`: one titled frame and nothing else, for capture. */
  const shot = new URLSearchParams(window.location.search).get("shot") as Dir | null;
  if (shot && DIRS.includes(shot)) return <div className="spec spec-shot">{frame(shot)}</div>;

  return (
    <div className="spec">
      <h1 className="spec-title">Push review · new commits on the timeline</h1>
      <div className="spec-controls">
        <div className="spec-group">{SCENARIOS.map((id) => seg("scenario", id, SCENARIO_LABEL[id]))}</div>
        <div className="spec-group">{seg("view", "directions", "Side by side")}{seg("view", "context", "In the app")}</div>
        {knobs.view === "context" && <div className="spec-group">{VARIANTS.map((variant) => seg("dir", variant, variant.toUpperCase()))}</div>}
        <div className="spec-group">{seg("width", "wide", "Wide")}{seg("width", "narrow", "Narrow")}</div>
        <div className="spec-group">{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <div className="spec-grid">
        {knobs.view === "directions" ? DIRS.map(frame) : (["before", knobs.dir] as const).map(frame)}
      </div>
      <p className="spec-foot">
        Hover a mark for its sentence. B1, B2 follow <a href="https://github.com/desktop/desktop/issues/5873" target="_blank" rel="noreferrer">GitHub Desktop’s ↑</a>;
        B3 <a href="https://git-scm.com/docs/git-log#Documentation/git-log.txt---decorateshortfullautono" target="_blank" rel="noreferrer">git log --decorate</a>’s <code>origin/main</code>;
        B4 the “new since” divider in chat and PR timelines; B5 <a href="https://code.visualstudio.com/docs/sourcecontrol/repos-remotes" target="_blank" rel="noreferrer">VS Code’s graph</a>.
      </p>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
