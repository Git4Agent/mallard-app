import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import "../src/App.css";

/**
 * Mockup: the Storage band on the project setup page.
 *
 * What shipped when this was written was a heading, a warning triangle, and a
 * row of pills:
 *
 *     Storage                                              ⚠
 *     [ 🖥 Local only ]   [ ☁ myconfstorage2 ✓ ]
 *
 * Five things made that hard to answer. Points 2 and 3 were fixed on
 * 2026-09-04 — the slot is named "None yet" and the triangle no longer fires
 * for it — and the first panel below now shows that page. The rest stand:
 *
 *  1. The question is never asked. "Storage" is a noun, not a decision. Nothing
 *     on the page says what a storage does for this project, or what happens if
 *     you pick neither.
 *  2. FIXED. The safe answer was flagged as a problem: local-only still reports
 *     `state: "attention"`, so the heading grew a triangle reading "Needs
 *     review" — while the sentence behind it, "No storage linked; the project
 *     will not sync until one is added.", was never rendered, because
 *     ProjectSetupWorkspace prints a section message only when the state is
 *     `blocked`. The band now takes a badge only when it is blocked.
 *  3. FIXED. Two unlike things were drawn as siblings: "Local only" was a mode
 *     and "myconfstorage2" a name someone typed, side by side as if they
 *     answered the same question. The slot is now named "None yet", the way
 *     the projects landing card ("No storage linked") and the catalog crumb
 *     ("No storage") already name it, and `yet` carries point 5's later.
 *  4. The consequence is invisible until it happens. Choosing a storage
 *     silently unfolds a whole Repository band underneath — new repo, or match
 *     an existing one — which is a harder question than the one just answered.
 *  5. Nothing says "later", though later already works. `Link storage…` is a
 *     live verb on a project's menu (ProjectLinksWorkspace.tsx:2242), so this
 *     decision is fully reversible; setup just never mentions it, which makes
 *     the row read as a commitment.
 *
 * Five directions, ordered by how much they move. Each is the same band, with
 * its own state, rendered on the real App.css:
 *
 *   A · Say it in words   same pills; a question above them and the outcome
 *                         below. Copy-only, no new controls.
 *   B · Two doors         decide whether to sync, then which storage. The
 *                         option rows the Repository band already uses.
 *   C · Local by default  no question at all; a stated outcome and one quiet
 *                         "Link a storage…" that reveals the list on request.
 *   D · Ask nothing here  the band leaves setup entirely. Every project is
 *                         created local, and linking becomes a next step.
 *   E · One honest list   one radiogroup, homogeneous rows, and local named as
 *                         a real sentence so it is a true sibling.
 *
 * The switch at the top sets how many storages exist, because that is what the
 * directions handle differently — and the first-run case (none) is where the
 * shipped row is at its worst: a radiogroup with one option and a warning.
 *
 * Static data, shipped classes, real Icons. Dev server only:
 * `npm run dev`, then /setup-storage-mockup.html.
 */

/* ——— data ————————————————————————————————————————————————————————— */

interface Storage {
  id: string;
  name: string;
  icon: IconName;
  /** What the storage listing puts on a storage's second line. */
  detail: string;
  /** Paths keep the mono face; anything else is prose. */
  detailIsPath?: boolean;
}

const ALL_STORAGES: Storage[] = [
  { id: "s1", name: "myconfstorage2", icon: "cloud", detail: "Cloudflare R2 · conf-bucket" },
  { id: "s2", name: "Backup drive", icon: "drive", detail: "/Volumes/Backup/mallard", detailIsPath: true },
  { id: "s3", name: "Team bucket", icon: "cloud", detail: "Cloudflare R2 · team-shared" },
];

const PROJECT = "HorrorGame";

function storagesFor(count: number): Storage[] {
  return ALL_STORAGES.slice(0, count);
}

function nameOf(storages: Storage[], id: string | null): string {
  return storages.find((storage) => storage.id === id)?.name ?? "";
}

/* ——— shared parts ————————————————————————————————————————————————— */

/** The section shell every band on the setup page is built from. */
function Band({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading">
        <div>
          <strong>{title}</strong>
          {subtitle && <span>{subtitle}</span>}
        </div>
      </div>
      {children}
    </section>
  );
}

function StoragePill({
  storage,
  active,
  onSelect,
}: {
  storage: Storage;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={`${storage.name} · ${storage.detail}`}
      title={`${storage.name} · ${storage.detail}`}
      className={`setup-pill${active ? " active" : ""}`}
      onClick={onSelect}
    >
      <Icon name={storage.icon} size={12} />
      <span className="setup-pill-label">{storage.name}</span>
      <span className="setup-pill-check" aria-hidden="true">
        {active && <Icon name="check" size={12} />}
      </span>
    </button>
  );
}

/** "Add a storage…" — a door out of the band, so it is dashed, never a peer
 *  of the answers beside it. */
function AddStoragePill({ label = "Add a storage…" }: { label?: string }) {
  return (
    <button type="button" className="setup-pill ghost">
      <Icon name="plus" size={12} />
      <span className="setup-pill-label">{label}</span>
    </button>
  );
}

/** One row of the option list the Repository band already draws. */
function OptionRow({
  active,
  icon,
  title,
  detail,
  detailIsPath,
  onSelect,
}: {
  active: boolean;
  icon: IconName;
  title: ReactNode;
  detail: ReactNode;
  detailIsPath?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      className={`sync-bundle-match-option${detailIsPath ? "" : " separate"}${active ? " active" : ""}`}
      onClick={onSelect}
    >
      <span className="sync-bundle-radio"><span /></span>
      <span className="setup-choice-mark" aria-hidden="true"><Icon name={icon} size={24} /></span>
      <span className="sync-bundle-match-copy">
        <strong>{title}</strong>
        <span>{detail}</span>
      </span>
    </button>
  );
}

/** The Repository band, stubbed: what appears the moment a storage is chosen.
 *  Present in every direction so the real cost of choosing stays visible. */
function RepositoryBand({ storageName }: { storageName: string }) {
  return (
    <Band title="Repository">
      <div className="sync-bundle-match-options" role="radiogroup" aria-label="Repo identity for this project">
        <OptionRow
          active
          icon="git-fork"
          title="New repository"
          detail={`Creates “${PROJECT}” in ${storageName} — machines joining later will find it there`}
          onSelect={() => undefined}
        />
      </div>
    </Band>
  );
}

/** The bands above Storage, so each direction is judged in its own page. */
function BandsAbove() {
  return (
    <>
      <Band title="Project">
        <div className="sync-field-grid sync-setup-project-fields">
          <label>
            <span>Name</span>
            <input value={PROJECT} readOnly />
          </label>
        </div>
      </Band>
      <Band title="Agent">
        <div className="setup-pill-row">
          <span className="setup-pill active" role="img" aria-label="Codex, on">
            <Icon name="openai" size={12} />
            <span className="setup-pill-label">Codex</span>
            <span className="setup-pill-check"><Icon name="check" size={12} /></span>
          </span>
        </div>
      </Band>
    </>
  );
}

/** The footer, whose label already changes with the answer. */
function Footer({ local }: { local: boolean }) {
  return (
    <footer className="mock-footer">
      <button type="button" className="btn btn-ghost">Discard</button>
      <button type="button" className="btn btn-primary">{local ? "Finish locally" : "Finish"}</button>
    </footer>
  );
}

function Page({ children, local }: { children: ReactNode; local: boolean }) {
  return (
    <div className="mock-page sync-setup-workspace">
      <div className="mock-body">
        <BandsAbove />
        {children}
      </div>
      <Footer local={local} />
    </div>
  );
}

/* ——— Today ———————————————————————————————————————————————————————— */

function Today({ count }: { count: number }) {
  const storages = storagesFor(count);
  const [chosen, setChosen] = useState<string | null>(null);
  return (
    <Page local={!chosen}>
      <Band title="Storage">
        <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
          <button
            type="button"
            role="radio"
            aria-checked={!chosen}
            className={`setup-pill${chosen ? "" : " active"}`}
            title="No storage linked yet — sessions stay on this computer"
            onClick={() => setChosen(null)}
          >
            <Icon name="computer" size={12} />
            <span className="setup-pill-label">None yet</span>
            <span className="setup-pill-check" aria-hidden="true">
              {!chosen && <Icon name="check" size={12} />}
            </span>
          </button>
          {storages.map((storage) => (
            <StoragePill
              key={storage.id}
              storage={storage}
              active={chosen === storage.id}
              onSelect={() => setChosen(storage.id)}
            />
          ))}
        </div>
      </Band>
      {chosen && <RepositoryBand storageName={nameOf(storages, chosen)} />}
    </Page>
  );
}

/* ——— A · Say it in words —————————————————————————————————————————— */

function DirectionA({ count }: { count: number }) {
  const storages = storagesFor(count);
  const [chosen, setChosen] = useState<string | null>(null);
  return (
    <Page local={!chosen}>
      <Band
        title="Storage"
        subtitle="Where this project's sessions are copied, so your other machines can pull them."
      >
        <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
          <button
            type="button"
            role="radio"
            aria-checked={!chosen}
            className={`setup-pill${chosen ? "" : " active"}`}
            onClick={() => setChosen(null)}
          >
            <Icon name="computer" size={12} />
            <span className="setup-pill-label">None yet</span>
            <span className="setup-pill-check" aria-hidden="true">
              {!chosen && <Icon name="check" size={12} />}
            </span>
          </button>
          {storages.map((storage) => (
            <StoragePill
              key={storage.id}
              storage={storage}
              active={chosen === storage.id}
              onSelect={() => setChosen(storage.id)}
            />
          ))}
          <AddStoragePill />
        </div>
        {/* The outcome of the answer standing right now — the sentence the
            warning triangle was gesturing at and never printed. */}
        <p className="setup-outcome">
          <Icon name={chosen ? "cloud" : "computer"} size={13} />
          {chosen
            ? <>Sessions are copied to <strong>{nameOf(storages, chosen)}</strong>. Choose a repository below.</>
            : storages.length === 0
              ? <>Sessions stay on this Mac. Add a storage now, or link one later from the project's menu.</>
              : <>Sessions stay on this Mac. You can link a storage later from the project's menu.</>}
        </p>
      </Band>
      {chosen && <RepositoryBand storageName={nameOf(storages, chosen)} />}
    </Page>
  );
}

/* ——— B · Two doors ———————————————————————————————————————————————— */

function DirectionB({ count }: { count: number }) {
  const storages = storagesFor(count);
  const [sync, setSync] = useState(false);
  const [chosen, setChosen] = useState<string | null>(null);
  const active = sync ? chosen : null;
  return (
    <Page local={!active}>
      <Band title="Storage">
        <div className="sync-bundle-match-options" role="radiogroup" aria-label="Storage">
          <OptionRow
            active={!sync}
            icon="computer"
            title="Keep this project on this Mac"
            detail="Sessions stay here. You can link a storage any time."
            onSelect={() => { setSync(false); setChosen(null); }}
          />
          <OptionRow
            active={sync}
            icon="cloud"
            title="Copy sessions to a storage"
            detail={storages.length === 0
              ? "You have not added one yet — Mallard will help you add it."
              : "So your other machines can pull them."}
            onSelect={() => {
              setSync(true);
              setChosen((current) => current ?? storages[0]?.id ?? null);
            }}
          />
        </div>
        {/* The second question, asked only once the first is answered. */}
        {sync && (
          <div className="setup-nested">
            <span className="setup-nested-label">Storage</span>
            <div className="setup-pill-row" role="radiogroup" aria-label="Which storage">
              {storages.map((storage) => (
                <StoragePill
                  key={storage.id}
                  storage={storage}
                  active={chosen === storage.id}
                  onSelect={() => setChosen(storage.id)}
                />
              ))}
              <AddStoragePill label={storages.length === 0 ? "Add your first storage…" : "Add a storage…"} />
            </div>
          </div>
        )}
      </Band>
      {active && <RepositoryBand storageName={nameOf(storages, active)} />}
    </Page>
  );
}

/* ——— C · Local by default ————————————————————————————————————————— */

function DirectionC({ count }: { count: number }) {
  const storages = storagesFor(count);
  const [open, setOpen] = useState(false);
  const [chosen, setChosen] = useState<string | null>(null);
  return (
    <Page local={!chosen}>
      <Band title="Storage">
        {chosen ? (
          <p className="setup-outcome stated">
            <Icon name="cloud" size={13} />
            <span>Sessions are copied to <strong>{nameOf(storages, chosen)}</strong>.</span>
            <button type="button" className="btn btn-ghost" onClick={() => { setChosen(null); setOpen(false); }}>
              Keep it local
            </button>
          </p>
        ) : (
          <p className="setup-outcome stated">
            <Icon name="computer" size={13} />
            <span>Sessions stay on this Mac. Nothing is copied anywhere.</span>
            {!open && (
              <button type="button" className="btn btn-ghost" onClick={() => setOpen(true)}>
                <Icon name="link" size={12} /> Link a storage…
              </button>
            )}
          </p>
        )}
        {open && !chosen && (
          <div className="setup-nested">
            <div className="setup-pill-row" role="radiogroup" aria-label="Which storage">
              {storages.map((storage) => (
                <StoragePill
                  key={storage.id}
                  storage={storage}
                  active={false}
                  onSelect={() => setChosen(storage.id)}
                />
              ))}
              <AddStoragePill label={storages.length === 0 ? "Add your first storage…" : "Add a storage…"} />
              <button type="button" className="btn btn-ghost" onClick={() => setOpen(false)}>Cancel</button>
            </div>
          </div>
        )}
      </Band>
      {chosen && <RepositoryBand storageName={nameOf(storages, chosen)} />}
    </Page>
  );
}

/* ——— D · Ask nothing here ————————————————————————————————————————— */

function DirectionD() {
  const [finished, setFinished] = useState(false);
  if (!finished) {
    return (
      <div className="mock-page sync-setup-workspace">
        <div className="mock-body">
          <BandsAbove />
        </div>
        <footer className="mock-footer">
          <button type="button" className="btn btn-ghost">Discard</button>
          <button type="button" className="btn btn-primary" onClick={() => setFinished(true)}>Finish</button>
        </footer>
      </div>
    );
  }
  return (
    <div className="mock-page sync-setup-workspace">
      <div className="mock-body">
        {/* Where the question went: after the project exists, when it is one
            offer among the things you can now do — not a gate on finishing. */}
        <div className="setup-next-step">
          <span className="setup-next-mark" aria-hidden="true"><Icon name="check-circle" size={24} /></span>
          <div className="setup-next-copy">
            <strong>{PROJECT} is set up.</strong>
            <span>Its sessions are on this Mac. Link a storage to copy them somewhere your other machines can pull them from.</span>
          </div>
          <button type="button" className="btn btn-primary">
            <Icon name="link" size={12} /> Link a storage
          </button>
        </div>
        <button type="button" className="btn btn-ghost mock-reset" onClick={() => setFinished(false)}>
          ← Back to setup
        </button>
      </div>
    </div>
  );
}

/* ——— E · One honest list —————————————————————————————————————————— */

function DirectionE({ count }: { count: number }) {
  const storages = storagesFor(count);
  const [chosen, setChosen] = useState<string | null>(null);
  return (
    <Page local={!chosen}>
      <Band title="Storage" subtitle="Where should this project's sessions be saved?">
        <div className="sync-bundle-match-options" role="radiogroup" aria-label="Storage">
          <OptionRow
            active={!chosen}
            icon="computer"
            title="Only on this Mac"
            detail="Nothing is copied anywhere. You can link a storage later."
            onSelect={() => setChosen(null)}
          />
          {storages.map((storage) => (
            <OptionRow
              key={storage.id}
              active={chosen === storage.id}
              icon={storage.icon}
              title={storage.name}
              detail={storage.detail}
              detailIsPath={storage.detailIsPath}
              onSelect={() => setChosen(storage.id)}
            />
          ))}
        </div>
        <p className="setup-outcome">
          <Icon name="plus" size={13} />
          <button type="button" className="mock-link">
            {storages.length === 0 ? "Add your first storage…" : "Add a storage…"}
          </button>
        </p>
      </Band>
      {chosen && <RepositoryBand storageName={nameOf(storages, chosen)} />}
    </Page>
  );
}

/* ——— harness —————————————————————————————————————————————————————— */

function Panel({ title, cost, note, children }: {
  title: string;
  cost: string;
  note: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mockup-panel">
      <h3>{title} <span className="mockup-cost">{cost}</span></h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

const NOTES = {
  today: <>
    As it ships, <strong>including the rename that landed 2026-09-04</strong>: the empty slot
    is called "None yet" — the name the projects landing card and the catalog crumb already
    use for it — and the warning triangle no longer fires for it. It had marked the safe
    answer as a fault while refusing to say why: the page prints a section's message only
    when the state is <code>blocked</code>, so the <code>attention</code> rung showed a
    triangle whose sentence, <em>"No storage linked; the project will not sync until one is
    added."</em>, was never rendered. What is still open: no sentence says what either
    answer does, and picking a storage unfolds a Repository band you did not ask for.
  </>,
  a: <>
    Nothing moves. The heading takes the question as its subtitle, and a line under the row
    states the outcome of the answer standing right now. Half of this direction has shipped
    — the "None yet" rename and dropping the triangle — so what is left to judge here is
    the two sentences: the subtitle that says what a storage is for, and the outcome line
    that says where the sessions end up and that a storage can be linked later. Cheapest of
    the five, and the remaining half is copy only.
  </>,
  b: <>
    Two questions instead of one. First the decision — does this project sync at all — in
    the same option rows the Repository band below already uses, with the consequence
    written into each. Only the second door opens the list of storages. The mode and the
    names are never siblings, the "not yet" answer is a full sentence rather than the
    absence of one, and the storage list can grow without touching the decision. Costs one
    extra click for anyone who does want to sync, and the tallest band of the five.
  </>,
  c: <>
    The band stops asking. It states what will happen — sessions stay on this Mac — and
    offers one quiet verb. A reader who does not know what a storage is has nothing to
    answer and loses nothing; a reader who does clicks "Link a storage…" and gets the list.
    This is the same verb the project's own menu already carries after setup, so setup and
    the rest of the app say it the same way. The risk is discoverability: someone who came
    specifically to connect a storage has to notice a ghost button.
  </>,
  d: <>
    The band leaves setup altogether, and the Repository band goes with it. Setup asks only
    what it needs to create the project — folder, name, agent — and every project is born
    local. Linking becomes a next step on a project that already exists, where the offer
    can be made in full and where "not now" costs nothing. The most user-friendly of the
    five and the largest change: the two-machine user who came to connect a repository now
    does it in a second act, and the finalize path stops taking a storage at all.
  </>,
  e: <>
    One question, one list, homogeneous rows — the mode gets a real sentence, so it is a
    true sibling of the names rather than a pill that looks like one. Everything is visible
    at once with no extra click, and each storage can finally show what it is (its bucket,
    its path) instead of hiding it in a tooltip. Taller than the pill row, and it scales
    the way a list does: fine at three storages, long at ten.
  </>,
};

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [count, setCount] = useState(1);
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Storage, on the project setup page</h1>
            <p>
              The band asked for a storage with a bare noun, two unlike pills and a warning
              triangle on the safe answer. <strong>Landed 2026-09-04:</strong> the empty slot
              is now "None yet" and the triangle no longer fires for it, so the first panel
              shows today's real page. The five directions below remain on offer, ordered by
              how much they move: words only, two doors, no question at all, out of setup
              entirely, and one honest list. Each panel keeps its own state; choose a storage
              in any of them to see the Repository band it unfolds.
            </p>
            <div className="mockup-switch" role="radiogroup" aria-label="Storages configured">
              {[0, 1, 3].map((value) => (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={count === value}
                  className={`btn${count === value ? " btn-primary" : ""}`}
                  onClick={() => setCount(value)}
                >
                  {value === 0 ? "No storages yet" : value === 1 ? "1 storage" : "3 storages"}
                </button>
              ))}
            </div>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel title="Today" cost="ships now" note={NOTES.today}>
          <Today key={`today-${count}`} count={count} />
        </Panel>
        <Panel title="A · Say it in words" cost="copy only" note={NOTES.a}>
          <DirectionA key={`a-${count}`} count={count} />
        </Panel>
        <Panel title="B · Two doors" cost="one new step" note={NOTES.b}>
          <DirectionB key={`b-${count}`} count={count} />
        </Panel>
        <Panel title="C · Local by default" cost="band restated" note={NOTES.c}>
          <DirectionC key={`c-${count}`} count={count} />
        </Panel>
        <Panel title="D · Ask nothing here" cost="band removed" note={NOTES.d}>
          <DirectionD key={`d-${count}`} />
        </Panel>
        <Panel title="E · One honest list" cost="control swapped" note={NOTES.e}>
          <DirectionE key={`e-${count}`} count={count} />
        </Panel>
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
body { background: var(--app-shell); }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-switch { display: flex; gap: 8px; margin-top: 14px; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-cost { margin-left: 8px; padding: 2px 7px; border-radius: var(--r-pill); color: var(--text-2); background: var(--chip-bg); font-family: var(--font); font-size: var(--type-2xs); vertical-align: middle; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-panel > p code { font-family: var(--font-mono); font-size: var(--type-xs); }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-page { min-height: 0; overflow: visible; }
.mock-body { padding: 6px 18px 14px; }
.mock-footer { display: flex; justify-content: flex-end; gap: 7px; padding: 11px 18px; border-top: 1px solid var(--border); background: var(--surface-footer); }
.mock-reset { margin-top: 14px; }
.mock-link { padding: 0; border: 0; color: var(--text-2); background: none; font: inherit; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
`;

/* Everything a cutover would lift into App.css. Five small rules; every other
   class on this page is the app's own. */
const PROPOSED_CSS = `
/* The outcome of the answer standing right now: one line, the glyph of where
   the sessions end up, and the sentence the warning triangle never printed.
   Prose, not an error — it states a fact about a valid choice, so it takes the
   hint's quiet ink and none of the notice furniture. */
.setup-outcome {
  max-width: var(--sync-setup-column);
  margin: 1px 0 0;
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--text-2);
  font-size: var(--type-xs);
  line-height: var(--leading-snug);
}

.setup-outcome > svg { flex: 0 0 auto; color: var(--text-3); }
.setup-outcome strong { color: var(--text-1); font-weight: var(--weight-semibold); }

/* When the line IS the band — direction C — it carries the band's own verb at
   the end of it, so the row keeps a button's height rather than prose's. */
.setup-outcome.stated { min-height: 32px; font-size: var(--type-sm); }
.setup-outcome.stated > span { min-width: 0; flex: 1; }
.setup-outcome.stated > .btn { flex: 0 0 auto; }

/* A door out of the band, never a peer of the answers beside it: the dashed
   edge is the same one an empty drop target wears. */
.setup-pill.ghost {
  border-style: dashed;
  border-color: var(--border-strong);
  color: var(--text-2);
  background: transparent;
}

/* A second question the first one opened, indented under it so the two read as
   one band and not as two. */
.setup-nested {
  max-width: var(--sync-setup-column);
  margin-top: 3px;
  padding-left: 13px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  border-left: 2px solid var(--border-mid);
}

.setup-nested-label {
  color: var(--text-3);
  font-family: var(--font-display);
  font-size: var(--type-2xs);
  font-weight: var(--weight-semibold);
  letter-spacing: 0.12em;
  text-transform: uppercase;
}

/* Direction D only: the offer that replaces the band, made after the project
   exists. The card grammar the app already uses for a standalone offer. */
.setup-next-step {
  max-width: var(--sync-setup-column);
  margin-top: 14px;
  padding: 14px;
  display: flex;
  align-items: center;
  gap: 12px;
  border: var(--box-border);
  border-radius: var(--retro-panel-radius);
  background: var(--surface-row);
  box-shadow: var(--shadow-card);
}

.setup-next-mark { flex: 0 0 auto; display: grid; place-items: center; color: var(--text-2); }
.setup-next-copy { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 3px; }
.setup-next-copy > strong { color: var(--text-0); font-family: var(--font-display); font-size: var(--type-md); }
.setup-next-copy > span { color: var(--text-2); font-size: var(--type-sm); line-height: var(--leading-snug); }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
/* One root across hot reloads: re-running the module must reuse it, or the
   dev server's second pass warns and mounts a second tree. */
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
