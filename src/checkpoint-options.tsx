import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import "./App.css";
import "./RetroTheme.css";

/**
 * Layout candidates for the checkpoint row, against the shipped skin. Not part
 * of the app bundle: the build has one entry, and this file is reachable only
 * from the dev server.
 *
 * The row carries five facts and nothing else — short sha, subject, branch,
 * commit clock, and the fact that Mallard matched the commit itself rather than
 * reading it from the transcript. What differs between candidates is how much
 * furniture those five facts are given, and where the last one goes.
 */

const SHA = "9f75e4c4f87d";
const FULL_SHA = "9f75e4c4f87d5fde447d36c3e172d820b744e1db";
const SUBJECT = "Enhance UI and GameLoop";
const LONG_SUBJECT =
  "Rework the creature silhouette pass so the human and the shape share one rig";
const CLOCK = "00:57";
const DATETIME = "2026-08-01T00:57:07";
const WHY = "Not recorded by the transcript. Mallard matched the first commit on main at or after the session ended.";

type Variant = "shipped" | "a" | "b" | "c" | "d";

function Node() {
  return (
    <span className="session-detail-event-node" role="img" aria-label={`Checkpoint · ${CLOCK}`} title={`Checkpoint · ${CLOCK}`}>
      <Icon name="git-commit" size={12} />
    </span>
  );
}

function Sha({ linked = false }: { linked?: boolean }) {
  if (!linked) {
    return <code className="session-detail-checkpoint-sha" title={FULL_SHA}>{SHA}</code>;
  }
  return (
    <a
      className="session-detail-checkpoint-link"
      href={`https://github.com/Git4Agent/HorrorGame/commit/${FULL_SHA}`}
      target="_blank"
      rel="noopener noreferrer"
      title={`Open ${FULL_SHA} on GitHub`}
      onClick={(event) => event.preventDefault()}
    >
      <Icon name="external-link" size={12} />
      <code className="session-detail-checkpoint-sha">{SHA}</code>
    </a>
  );
}

function Branch() {
  return <span className="session-detail-checkpoint-branch">main</span>;
}

function Clock() {
  return <time dateTime={DATETIME}>{CLOCK}</time>;
}

function CheckpointRow({ variant, subject, flagged, linked = false }: {
  variant: Variant;
  subject: string;
  /** false renders the same layout for a commit Mallard did not have to guess
   *  at — the state the row sits in once the flag is gone. */
  flagged: boolean;
  /** true renders the sha as the off-app link a configured remote earns. */
  linked?: boolean;
}) {
  const body = () => {
    switch (variant) {
      // Shipped: the flag is a chip on its own line, above a card that holds
      // the facts. Three stacked boxes for one commit.
      case "shipped":
        return (
          <>
            {flagged && (
              <div className="session-detail-event-flags">
                <span className="session-detail-quality quality-inferred">Inferred</span>
              </div>
            )}
            <div className="session-detail-checkpoint-card">
              <Sha />
              <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
              <span className="session-detail-checkpoint-end"><Branch /><Clock /></span>
            </div>
          </>
        );
      // A: the same card, with the flag folded into the end cluster it belongs
      // beside. One line, one box, no other change.
      case "a":
        return (
          <div className="session-detail-checkpoint-card">
            <Sha linked={linked} />
            <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
            <span className="session-detail-checkpoint-end">
              {flagged && <span className="session-detail-quality quality-inferred" title={WHY}>Inferred</span>}
              <Branch />
              <Clock />
            </span>
          </div>
        );
      // B: not a card at all. A commit is a milestone the conversation passes,
      // so it rides the rail as a marker and lets the hairline carry the width.
      case "b":
        return (
          <div className="opt-b-marker">
            <Sha />
            <span className="opt-b-subject" title={subject}>{subject}</span>
            <span className="opt-b-rule" aria-hidden="true" />
            <span className="opt-b-end">
              <Branch />
              <Clock />
              {flagged && <span className="opt-b-flag" title={WHY}>inferred</span>}
            </span>
          </div>
        );
      // C: the page's own card grammar — content line, then the quiet meta line
      // prompts already use. The flag becomes a word among the other metadata.
      case "c":
        return (
          <div className="session-detail-checkpoint-card opt-c-card">
            <div className="opt-c-line">
              <Sha />
              <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
            </div>
            <div className="session-detail-card-meta">
              <Branch />
              <Clock />
              {flagged && <span className="opt-c-flag" title={WHY}>inferred from Git</span>}
            </div>
          </div>
        );
      // D: no flag chip. The card says what the relation actually is and wears
      // a dashed edge, so nothing has to be decoded from a colour.
      case "d":
        return (
          <div className={`session-detail-checkpoint-card${flagged ? " opt-d-card" : ""}`} title={flagged ? WHY : undefined}>
            {flagged && <span className="opt-d-lead">Likely commit</span>}
            <Sha />
            <span className="session-detail-checkpoint-subject" title={subject}>{subject}</span>
            <span className="session-detail-checkpoint-end"><Branch /><Clock /></span>
          </div>
        );
    }
  };
  return (
    <article className={`session-detail-event event-checkpoint opt-${variant}`}>
      <Node />
      <div className="session-detail-event-body">{body()}</div>
    </article>
  );
}

/** The row above, so each candidate is judged against what it follows. */
function ResponseRow() {
  return (
    <article className="session-detail-event event-response">
      <span className="session-detail-event-node" role="img" aria-label="Claude · 05:13" title="Claude · 05:13">
        <ProviderMark provider="claude" displayName="Claude" size={12} />
      </span>
      <div className="session-detail-event-body">
        <div className="session-detail-event-copy session-detail-response-prose">
          <p>The loop now ticks on a fixed timestep and the creature rig shares one skeleton with the human, so the silhouette pass stops double-drawing.</p>
        </div>
      </div>
    </article>
  );
}

function Excerpt({ variant, subject, flagged, gap }: {
  variant: Variant;
  subject: string;
  flagged: boolean;
  gap: string;
}) {
  return (
    <div className="session-detail-page specimen-frame">
      <div className="session-detail-timeline">
        <div className="session-detail-day">
          <h2 className="session-detail-day-divider"><span>Aug 1, 2026</span></h2>
          <ol className="session-detail-event-list">
            <li><ResponseRow /></li>
            <li>
              <div className="session-detail-event-gap"><span>{gap}</span></div>
              <CheckpointRow variant={variant} subject={subject} flagged={flagged} />
            </li>
          </ol>
        </div>
      </div>
    </div>
  );
}

const CANDIDATES: { id: Variant; name: string; blurb: string }[] = [
  { id: "shipped", name: "Before", blurb: "62px — flag on its own line, above the card" },
  { id: "a", name: "A · One line — shipped", blurb: "36px — flag folded into the end cluster; nothing else moved" },
  { id: "b", name: "B · Rail marker", blurb: "28px — no card; the commit rides a hairline across the column" },
  { id: "c", name: "C · Commit block", blurb: "66px — content line + the meta line prompts already use" },
  { id: "d", name: "D · Stated relation", blurb: "36px — no chip: words and a dashed edge carry the uncertainty" },
];

function Specimen() {
  const [theme, setTheme] = useState<"dark" | "light">(
    new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light",
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  return (
    <div className="specimen-page">
      <style>{CANDIDATE_CSS}</style>
      <header className="specimen-head">
        <div>
          <h1>The checkpoint row</h1>
          <p>
            Five facts: sha, subject, branch, clock, and that Mallard matched the
            commit rather than reading it from the transcript. The shipped row
            spends three stacked boxes on them and puts the least important fact
            in the loudest colour, on a line of its own.
          </p>
        </div>
        <div className="specimen-controls">
          <label>
            <input type="checkbox" checked={theme === "dark"} onChange={(event) => setTheme(event.target.checked ? "dark" : "light")} />
            {" "}dark
          </label>
        </div>
      </header>

      <section>
        <h2>1 · Inferred beside certain</h2>
        <p className="specimen-lead">
          Left: the checkpoint Mallard guessed — no sha in the transcript, so the
          first commit on the branch 20 hours after the session ended. Right: the
          same layout for a commit that needed no guess. Today every checkpoint
          renders as the left column; the right is what each layout settles into
          once the flag is gone, and shows how far the row moves when it is.
        </p>
        <div className="specimen-pair-head">
          <span><b>Inferred</b> — matched from Git, ±20h</span>
          <span><b>Certain</b> — the commit is known</span>
        </div>
        <div className="specimen-rows">
          {CANDIDATES.map((candidate) => (
            <div className="specimen-row" key={candidate.id}>
              <div className="specimen-row-label"><b>{candidate.name}</b><span>{candidate.blurb}</span></div>
              <div className="specimen-pair">
                <Excerpt variant={candidate.id} subject={SUBJECT} flagged gap="+20h" />
                <Excerpt variant={candidate.id} subject={SUBJECT} flagged={false} gap="+2m" />
              </div>
            </div>
          ))}
        </div>
      </section>

      <section>
        <h2>2 · The sha, linked and bare</h2>
        <p className="specimen-lead">
          A GitHub remote makes the sha an off-app target, marked by the glyph
          the activity list already uses. Without one the chip stays exactly as
          it is — no box promising a page that does not exist.
        </p>
        <div className="specimen-pair-head">
          <span><b>Remote configured</b> — opens the commit on GitHub</span>
          <span><b>No remote</b> — full sha on hover, nothing to open</span>
        </div>
        <div className="specimen-pair">
          <div className="specimen-frame">
            <div className="session-detail-page">
              <div className="session-detail-timeline">
                <ol className="session-detail-event-list">
                  <li><CheckpointRow variant="a" subject={SUBJECT} flagged linked /></li>
                </ol>
              </div>
            </div>
          </div>
          <div className="specimen-frame">
            <div className="session-detail-page">
              <div className="session-detail-timeline">
                <ol className="session-detail-event-list">
                  <li><CheckpointRow variant="a" subject={SUBJECT} flagged /></li>
                </ol>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section>
        <h2>3 · Long subject, narrow column</h2>
        <p className="specimen-lead">
          A subject that cannot fit, in a 440px column. The card had no
          <code>min-width: 0</code>, so it grew past its column and the ellipsis
          it asks for could never fire; that one-line fix shipped with A, and
          every row here now truncates while the sha, branch and clock survive.
        </p>
        <div className="specimen-narrow">
          {CANDIDATES.map((candidate) => (
            <div className="specimen-row" key={candidate.id}>
              <div className="specimen-row-label"><b>{candidate.name}</b></div>
              <div className="specimen-frame">
                <div className="session-detail-page">
                  <div className="session-detail-timeline">
                    <ol className="session-detail-event-list">
                      <li><CheckpointRow variant={candidate.id} subject={LONG_SUBJECT} flagged /></li>
                    </ol>
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

/* Candidate-only rules. Rendered inside the tree so they land after Vite's
   injected App.css and win ties without !important. */
const CANDIDATE_CSS = `
/* The card carries this in App.css now; the marker is specimen-only, so it
   still needs its own. */
.opt-b-marker {
  min-width: 0;
}

.opt-b-marker {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 28px;
}
.opt-b-subject {
  min-width: 0;
  flex: 0 1 auto;
  overflow: hidden;
  color: var(--text-0);
  font-size: var(--type-md);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.opt-b-rule {
  flex: 1 1 16px;
  min-width: 12px;
  height: 1px;
  background: var(--border);
}
.opt-b-end {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.opt-b-end time {
  color: var(--text-2);
  font-size: var(--type-xs);
  font-variant-numeric: tabular-nums;
}
.opt-b-flag {
  color: var(--purple);
  font-size: var(--type-3xs);
}

.opt-c-card {
  display: block;
  min-height: 0;
  padding: 0;
}
.opt-c-line {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px 7px;
}
.opt-c-flag { color: var(--purple); }

.opt-d-card { border-style: dashed; }
.opt-d-lead {
  flex: 0 0 auto;
  color: var(--purple);
  font-family: var(--font-pixel, inherit);
  font-size: var(--type-3xs);
  letter-spacing: 0.5px;
  text-transform: uppercase;
}
`;

createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
