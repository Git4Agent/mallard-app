import { useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";
import PublishVersionSheet, { type PublishVersionFact } from "../src/components/managed-cloud/PublishVersionSheet";
import "../src/App.css";
import "./publish-sheet-mockup.css";

/**
 * Specimen: the Publish version sheet
 * (src/components/managed-cloud/PublishVersionSheet.tsx).
 *
 * *Before* is the shipped sheet itself, mounted in place. A–G keep the
 * shipped look — the `.sync-modal` frame, its head and foot bands, the
 * Baloo title, ink borders, pill buttons, the app's tokens — and change
 * only what the sheet holds, where, and in what words. Every frame gets the
 * same states; a typed name is typed into each field the way a keyboard
 * would (React sees the input event).
 * `npm run dev`, then /mockups/publish-sheet-mockup.html
 * (`?view=compare|one&scene=<id>&dir=<id>&theme=light|dark&focus=0|1`).
 */

/* Eight sheets on one page: each stays in its frame, and none takes focus
   (the page would jump to the last one). The Focus toggle draws the ring. */
HTMLDialogElement.prototype.showModal = function openInPlace(this: HTMLDialogElement) {
  this.setAttribute("open", "");
};
HTMLInputElement.prototype.focus = function noFocus() {};

/* ——— states ———————————————————————————————————————————————————————— */

interface Scene {
  id: string;
  label: string;
  version: number;
  holds: PublishVersionFact[];
  toPush: number;
  busy: boolean;
  typed?: string;
}

const SESSIONS_7: PublishVersionFact = { icon: "message", word: "7 sessions" };
const FULL: PublishVersionFact[] = [
  { icon: "message", word: "8 sessions" },
  { icon: "file", word: "15 files" },
  { icon: "upload", word: "3 pushes since version 2" },
];

const SCENES: Scene[] = [
  { id: "first", label: "Version 1 · Git project (the screenshot)", version: 1, holds: [SESSIONS_7], toPush: 0, busy: false },
  { id: "later", label: "Version 3 · files, pushes since 2", version: 3, holds: FULL, toPush: 0, busy: false },
  { id: "waiting", label: "2 sessions here not pushed", version: 3, holds: FULL, toPush: 2, busy: false },
  { id: "typed", label: "A long name typed", version: 1, holds: [SESSIONS_7], toPush: 0, busy: false, typed: "Landing page pass — GIF and SVG pair for the hero" },
  { id: "busy", label: "Publishing…", version: 1, holds: [SESSIONS_7], toPush: 0, busy: true, typed: "Landing page pass" },
];

const notPushed = (count: number) => (count === 1 ? "1 session here is not pushed, so not in it" : `${count} sessions here are not pushed, so not in it`);
const verbOf = (scene: Scene) => `Publish version ${scene.version}`;

/* ——— shared parts (shipped classes + the proposed pv- rules) ————————— */

function Facts({ holds, audience = true }: { holds: PublishVersionFact[]; audience?: boolean }) {
  return (
    <span className="pv-facts">
      {holds.map((fact) => <span key={fact.word} className="pv-fact"><Icon name={fact.icon} size={12} />{fact.word}</span>)}
      {audience && <span className="pv-fact pv-audience"><Icon name="lock" size={12} />Collaborators only</span>}
    </span>
  );
}

function Waiting({ count }: { count: number }) {
  if (count === 0) return null;
  return (
    <p className="pv-waiting">
      <ThreadStatusIcon status="local-outstanding" size={14} />
      <span>{notPushed(count)}</span>
    </p>
  );
}

function NameField({ value, onChange, busy, label = "Name" }: { value: string; onChange: (next: string) => void; busy: boolean; label?: string }) {
  return (
    <label className="pv-field">
      <span className="pv-label"><span>{label}</span><small>Optional</small></span>
      <input type="text" maxLength={80} spellCheck={false} value={value} disabled={busy} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

function Head({ title, closable = true, busy, children }: { title: string; closable?: boolean; busy: boolean; children?: ReactNode }) {
  return (
    <header className="sync-modal-header">
      <div>
        <h1>{title}</h1>
        {children}
      </div>
      {closable && (
        <button type="button" className="btn btn-ghost" disabled={busy} aria-label="Close without publishing">
          <Icon name="x" size={17} />
        </button>
      )}
    </header>
  );
}

function Foot({ scene, left }: { scene: Scene; left?: ReactNode }) {
  return (
    <footer className="sync-modal-footer">
      {left}
      <div>
        <button type="button" className="btn" disabled={scene.busy}>Cancel</button>
        <button type="button" className="btn btn-primary" disabled={scene.busy}>{scene.busy ? "Publishing…" : verbOf(scene)}</button>
      </div>
    </footer>
  );
}

/* ——— directions ———————————————————————————————————————————————————— */

/** A · Labeled field: one fact line (with who gets it), then a field with a real label. */
function LabeledSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-a" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <Facts holds={scene.holds} />
        <Waiting count={scene.toPush} />
        <NameField value={name} onChange={setName} busy={scene.busy} />
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

/** B · Version card: the row the version will be, filled in as the name is typed. */
function CardSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-b" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <div className="pv-card">
          <span className="pv-num" aria-hidden="true">{scene.version}</span>
          <span className="pv-card-text">
            <strong className={name.trim() ? undefined : "unnamed"}>{name.trim() || `Version ${scene.version}`}</strong>
            <Facts holds={scene.holds} />
          </span>
        </div>
        <Waiting count={scene.toPush} />
        <NameField value={name} onChange={setName} busy={scene.busy} />
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

/** C · Draft → Version: what is in Mallard Cloud now, and what it becomes. */
function RouteSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-c" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <div className="pv-end">
          <span className="pv-end-head"><Icon name="storage-mallard" size={14} /><b>Draft</b></span>
          <Facts holds={scene.holds} audience={false} />
        </div>
        <span className="pv-route-arrow" aria-hidden="true"><Icon name="chevron-down" size={16} /></span>
        <div className="pv-end becomes">
          <span className="pv-end-head"><b>Version {scene.version}</b><span className="pv-fact pv-audience"><Icon name="lock" size={12} />Collaborators only</span></span>
          <NameField value={name} onChange={setName} busy={scene.busy} />
        </div>
        <Waiting count={scene.toPush} />
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

/** D · Summary in the head: the facts are the title's line, the body is the field; Cancel is the one way out. */
function HeadSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-d" open>
      <Head title={verbOf(scene)} closable={false} busy={scene.busy}>
        <p className="pv-head-line"><Facts holds={scene.holds} /></p>
      </Head>
      <div className="sync-modal-body pv-body">
        <Waiting count={scene.toPush} />
        <NameField value={name} onChange={setName} busy={scene.busy} />
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

/** E · Field first, facts in the foot: the body asks one thing; the foot says what the verb takes. */
function FootSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-e" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <NameField value={name} onChange={setName} busy={scene.busy} />
        <Waiting count={scene.toPush} />
      </div>
      <Foot scene={scene} left={<span className="pv-foot-facts"><Facts holds={scene.holds} /></span>} />
    </dialog>
  );
}

/** F · Name and note: A, with the note the service already takes behind one button. */
function NoteSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  const [noteOpen, setNoteOpen] = useState(!!scene.typed);
  const [note, setNote] = useState(scene.typed ? "Hero GIF re-exported at 2×; SVG pair checked in Safari." : "");
  return (
    <dialog className="sync-modal pv-sheet pv-f" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <Facts holds={scene.holds} />
        <Waiting count={scene.toPush} />
        <NameField value={name} onChange={setName} busy={scene.busy} />
        {noteOpen ? (
          <label className="pv-field">
            <span className="pv-label"><span>Note</span><small>Optional</small></span>
            <textarea rows={3} maxLength={500} value={note} disabled={scene.busy} onChange={(event) => setNote(event.target.value)} />
          </label>
        ) : (
          <button type="button" className="btn-link pv-add-note" disabled={scene.busy} onClick={() => setNoteOpen(true)}>
            <Icon name="plus" size={12} />Add a note
          </button>
        )}
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

/** G · Anchored card: drops from the Cloud chip, no backdrop; same parts, popover measure. */
function AnchoredSheet({ scene }: { scene: Scene }) {
  const [name, setName] = useState("");
  return (
    <dialog className="sync-modal pv-sheet pv-g" open>
      <Head title={verbOf(scene)} busy={scene.busy} />
      <div className="sync-modal-body pv-body">
        <Facts holds={scene.holds} />
        <Waiting count={scene.toPush} />
        <NameField value={name} onChange={setName} busy={scene.busy} />
      </div>
      <Foot scene={scene} />
    </dialog>
  );
}

interface Ref { label: string; href: string }

interface Direction {
  id: string;
  name: string;
  placement: "center" | "anchored";
  /** What the direction does, in a few words. */
  gist: string;
  /** Where the form is seen: design systems and shipped products. */
  seenIn: Ref[];
  /** What argues against it. */
  against?: string;
  pick?: boolean;
  change?: string;
  draw: (scene: Scene) => ReactNode;
}

const REF = {
  carbonModal: { label: "Carbon Modal", href: "https://carbondesignsystem.com/components/modal/usage/" },
  carbonForms: { label: "Carbon Forms", href: "https://carbondesignsystem.com/patterns/forms-pattern/" },
  primerDialog: { label: "Primer Dialog", href: "https://primer.style/product/components/dialog/guidelines/" },
  primerForms: { label: "Primer Forms", href: "https://primer.style/product/ui-patterns/forms/" },
  atlassianForm: { label: "Atlassian Form", href: "https://atlassian.design/components/form/usage" },
  nngPlaceholder: { label: "NN/g placeholders", href: "https://www.nngroup.com/articles/form-design-placeholders/" },
  nngRequired: { label: "NN/g required fields", href: "https://www.nngroup.com/articles/required-fields/" },
  govukQuestions: { label: "GOV.UK question pages", href: "https://design-system.service.gov.uk/patterns/question-pages/" },
  apgDialog: { label: "ARIA APG dialog", href: "https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/examples/dialog/" },
  higSheets: { label: "HIG Sheets", href: "https://developer.apple.com/design/human-interface-guidelines/sheets" },
  higAlerts: { label: "HIG Alerts", href: "https://developer.apple.com/design/human-interface-guidelines/alerts" },
  figmaHistory: { label: "Figma · Save to version history", href: "https://help.figma.com/hc/en-us/articles/360038006754-View-a-file-s-version-history" },
  figmaLibrary: { label: "Figma · Publish a library", href: "https://help.figma.com/hc/en-us/articles/360025508373-Publish-a-library" },
  framerVersions: { label: "Framer · Staging and versions", href: "https://www.framer.com/help/articles/staging-and-versions/" },
  vercelPromote: { label: "Vercel · Promote to production", href: "https://vercel.com/docs/deployments/promoting-a-deployment" },
  vercelRollback: { label: "Vercel · Instant rollback", href: "https://vercel.com/docs/deployments/instant-rollback" },
  githubRelease: { label: "GitHub · Draft a new release", href: "https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository" },
  notionPublish: { label: "Notion · Publish", href: "https://www.notion.com/help/public-pages-and-web-publishing" },
} satisfies Record<string, Ref>;

const DIRECTIONS: Direction[] = [
  {
    id: "before", name: "Shipped", placement: "center", gist: "PublishVersionSheet today", seenIn: [],
    against: "The placeholder is the only label, and it goes once a name is typed. The tiles beside each line say nothing the line does not. Nothing says who gets the version.",
    draw: (scene) => (
      <PublishVersionSheet key={scene.id} version={scene.version} holds={scene.holds} sessionsToPush={scene.toPush} busy={scene.busy} onPublish={() => {}} onClose={() => {}} />
    ),
  },
  {
    id: "a", name: "A · Labeled field", placement: "center", pick: true, gist: "Fact line, then Name · Optional",
    seenIn: [REF.carbonModal, REF.primerForms, REF.figmaHistory],
    against: "With three facts, Collaborators only drops to a second line.",
    draw: (scene) => <LabeledSheet key={scene.id} scene={scene} />,
  },
  {
    id: "b", name: "B · Version card", placement: "center", gist: "The row it will be, filled in as you type",
    seenIn: [REF.figmaLibrary, REF.framerVersions, REF.vercelPromote],
    against: "The name shows twice while it is typed: in the card and in the field.",
    draw: (scene) => <CardSheet key={scene.id} scene={scene} />,
  },
  {
    id: "c", name: "C · Draft → Version", placement: "center", gist: "What is there, what it becomes",
    seenIn: [REF.vercelRollback, REF.vercelPromote],
    against: "The tallest sheet, with two boxes and an arrow for one optional field. Primer asks for forms in dialogs to stay simple.",
    draw: (scene) => <RouteSheet key={scene.id} scene={scene} />,
  },
  {
    id: "d", name: "D · Summary in the head", placement: "center", change: "no ×", gist: "Facts under the title, Cancel only",
    seenIn: [REF.higSheets, REF.higAlerts],
    against: "Primer Dialog requires a visible close button, and the shipped Open sheet keeps its ×.",
    draw: (scene) => <HeadSheet key={scene.id} scene={scene} />,
  },
  {
    id: "e", name: "E · Facts in the foot", placement: "center", gist: "Field first, facts beside the verb",
    seenIn: [REF.primerDialog],
    against: "No source puts facts in the foot. Primer's footer left holds secondary actions. Three facts wrap to three lines there.",
    draw: (scene) => <FootSheet key={scene.id} scene={scene} />,
  },
  {
    id: "f", name: "F · Name and note", placement: "center", change: "sends a note", gist: "A, plus Add a note",
    seenIn: [REF.figmaHistory, REF.githubRelease, REF.figmaLibrary],
    against: "The desktop command sends only a name today. A note changes the Tauri contract (the service already takes one, up to 500 characters).",
    draw: (scene) => <NoteSheet key={scene.id} scene={scene} />,
  },
  {
    id: "g", name: "G · Anchored card", placement: "anchored", change: "no backdrop", gist: "Drops from the Cloud chip",
    seenIn: [REF.notionPublish],
    against: "HIG Sheets puts a short task tied to the window in a modal sheet. Vercel and Figma publish from modals too.",
    draw: (scene) => <AnchoredSheet key={scene.id} scene={scene} />,
  },
];

/** What the references ask, and which sheet meets it: y yes, p partly, n no. */
const RULES: { rule: string; refs: Ref[]; marks: Record<string, "y" | "p" | "n">; note?: string }[] = [
  {
    rule: "A label that stays visible; a placeholder is never the label",
    refs: [REF.primerForms, REF.carbonForms, REF.atlassianForm, REF.nngPlaceholder],
    marks: { before: "n", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
  },
  {
    rule: "Mark the optional field (here the only field is optional)",
    refs: [REF.carbonForms, REF.govukQuestions, REF.nngRequired],
    marks: { before: "p", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
    note: "Shipped says it only in the placeholder.",
  },
  {
    rule: "Focus the first field when it opens",
    refs: [REF.carbonModal, REF.apgDialog],
    marks: { before: "y", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
    note: "Primer's default is the close button; Return in the field still publishes.",
  },
  {
    rule: "The title repeats the verb that opened it",
    refs: [REF.carbonModal, REF.vercelPromote],
    marks: { before: "y", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
  },
  {
    rule: "Say who gets it before the verb",
    refs: [REF.vercelRollback, REF.figmaLibrary, REF.notionPublish],
    marks: { before: "n", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
  },
  {
    rule: "Show what goes in",
    refs: [REF.figmaLibrary, REF.vercelRollback],
    marks: { before: "y", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "y" },
    note: "B and C show it as the thing being made.",
  },
  {
    rule: "A visible ×",
    refs: [REF.primerDialog, REF.carbonModal],
    marks: { before: "y", a: "y", b: "y", c: "y", d: "n", e: "y", f: "y", g: "y" },
    note: "HIG Sheets goes the other way: Cancel only.",
  },
  {
    rule: "Short, single column",
    refs: [REF.primerDialog, REF.carbonModal],
    marks: { before: "y", a: "y", b: "y", c: "p", d: "y", e: "y", f: "p", g: "y" },
  },
  {
    rule: "A short task tied to the window is a modal sheet",
    refs: [REF.higSheets],
    marks: { before: "y", a: "y", b: "y", c: "y", d: "y", e: "y", f: "y", g: "n" },
  },
  {
    rule: "A name plus a description",
    refs: [REF.figmaHistory, REF.githubRelease],
    marks: { before: "n", a: "n", b: "n", c: "n", d: "n", e: "n", f: "y", g: "n" },
    note: "Only if a note is wanted: it is a contract change.",
  },
];

const MARK = { y: "✓", p: "~", n: "✗" } as const;
const MARK_CLASS = { y: "yes", p: "part", n: "no" } as const;

function Links({ refs }: { refs: Ref[] }) {
  return <>{refs.map((ref, i) => <span key={ref.href}>{i > 0 && " · "}<a href={ref.href} target="_blank" rel="noreferrer">{ref.label}</a></span>)}</>;
}

/* ——— frames ——————————————————————————————————————————————————————————— */

function StandInApp() {
  return (
    <div className="pv-app" aria-hidden="true">
      <div className="pv-app-bar">
        <span className="pv-app-crumb">Projects</span>
        <span className="pv-app-crumb dim">api-server</span>
        <span className="pv-app-chip"><Icon name="cloud-lock" size={14} />Private<Icon name="chevron-down" size={12} /></span>
        <span className="pv-app-pill"><Icon name="download" size={13} />Pull</span>
        <span className="pv-app-pill"><Icon name="upload" size={13} />Push</span>
      </div>
      <div className="pv-app-side">
        {Array.from({ length: 6 }, (_, i) => <span key={i} className="pv-app-row" style={{ width: `${70 - i * 6}%` }} />)}
      </div>
      <div className="pv-app-main">
        {Array.from({ length: 4 }, (_, i) => <span key={i} className="pv-app-card" />)}
      </div>
    </div>
  );
}

/** Types the scene's name into the sheet's field the way a keyboard would. */
function typeInto(root: HTMLElement | null, text: string | undefined) {
  const input = root?.querySelector<HTMLInputElement>("dialog input[type=text]");
  if (!input || !text) return;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function Frame({ direction, scene, caption }: { direction: Direction; scene: Scene; caption: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [words, setWords] = useState<number | null>(null);
  useEffect(() => { typeInto(ref.current, scene.typed); }, [scene]);
  useEffect(() => {
    const dialog = ref.current?.querySelector("dialog");
    const next = dialog ? dialog.innerText.split(/\s+/).filter((word) => /\w/.test(word)).length : null;
    if (next !== words) setWords(next);
  });
  return (
    <figure className="pv-cell">
      <figcaption>{caption}{words !== null && <span className="pv-count">{words} words</span>}</figcaption>
      <div ref={ref} className={`pv-frame ${direction.placement}`}>
        <StandInApp />
        {direction.placement !== "anchored" && <div className="pv-scrim" />}
        {direction.draw(scene)}
      </div>
    </figure>
  );
}

/* ——— page ——————————————————————————————————————————————————————————— */

type ViewMode = "compare" | "one";
type Theme = "light" | "dark";
const query = new URLSearchParams(window.location.search);
const pick = <T extends string>(key: string, options: readonly T[], fallback: T): T => {
  const value = query.get(key);
  return options.find((option) => option === value) ?? fallback;
};

function Seg<T extends string>({ value, current, set, children }: { value: T; current: T; set: (value: T) => void; children: ReactNode }) {
  return <button type="button" className="spec-seg" aria-pressed={value === current} onClick={() => set(value)}>{children}</button>;
}

function Specimen() {
  const [mode, setMode] = useState<ViewMode>(pick("view", ["compare", "one"] as const, "compare"));
  const [sceneId, setScene] = useState(pick("scene", SCENES.map((s) => s.id), "first"));
  const [dirId, setDir] = useState(pick("dir", DIRECTIONS.map((d) => d.id), "a"));
  const [theme, setTheme] = useState<Theme>(pick("theme", ["light", "dark"] as const, "light"));
  const [focus, setFocus] = useState(pick("focus", ["1", "0"] as const, "1"));
  // Sheets only, no window behind them and no panels: one picture of all eight.
  const bare = query.get("bare") === "1";
  document.documentElement.dataset.theme = theme;
  const scene = SCENES.find((s) => s.id === sceneId)!;
  const direction = DIRECTIONS.find((d) => d.id === dirId)!;

  const nameOf = (d: Direction) => (
    <>
      <b>{d.name}</b>
      {d.pick && <em>recommended</em>}
      {d.change && <em className="change">{d.change}</em>}
      <small>{d.gist}</small>
    </>
  );

  return (
    <div className={`spec${focus === "1" ? " pv-focused" : ""}${bare ? " pv-bare" : ""}`}>
      <h1 className="spec-title">Publish version — directions</h1>
      <p className="spec-sub">
        The sheet <b>Publish version N…</b> opens from the Cloud chip. <b>Shipped</b> is the real sheet. A–G keep its look — frame, bands, title,
        buttons, tokens — and change only what it holds and where. A, B, C, E need no new data; D drops the ×; F sends a note; G drops the backdrop.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>View</span>
          <Seg value="compare" current={mode} set={setMode}>All directions, one state</Seg>
          <Seg value="one" current={mode} set={setMode}>One direction, all states</Seg>
        </div>
        <div className="spec-group"><span>Theme</span>
          <Seg value="light" current={theme} set={setTheme}>Light</Seg>
          <Seg value="dark" current={theme} set={setTheme}>Dark</Seg>
        </div>
        <div className="spec-group"><span>Field focus</span>
          <Seg value="1" current={focus} set={setFocus}>On (as it opens)</Seg>
          <Seg value="0" current={focus} set={setFocus}>Off</Seg>
        </div>
        {mode === "compare" ? (
          <div className="spec-group"><span>State</span>
            {SCENES.map((s) => <Seg key={s.id} value={s.id} current={sceneId} set={setScene}>{s.label}</Seg>)}
          </div>
        ) : (
          <div className="spec-group"><span>Direction</span>
            {DIRECTIONS.map((d) => <Seg key={d.id} value={d.id} current={dirId} set={setDir}>{d.name}</Seg>)}
          </div>
        )}
      </div>

      <div className="pv-grid">
        {mode === "compare"
          ? DIRECTIONS.map((d) => <Frame key={`${d.id}-${scene.id}`} direction={d} scene={scene} caption={nameOf(d)} />)
          : SCENES.map((s) => <Frame key={`${direction.id}-${s.id}`} direction={direction} scene={s} caption={<b>{s.label}</b>} />)}
      </div>

      <div className="spec-panel">
        <strong>Pick: A · Labeled field</strong>
        <p>
          One line says what goes in and who gets it (<i>Collaborators only</i>, the panel's own words). Under it, the field with a label that stays
          when a name is typed, marked <i>Optional</i>. The decorative tiles go. Same width, frame, ×, buttons and focus as today; no new data.
          B is the step up if the version should read as a thing being made. F is the owner's call: it adds the note the service already takes.
        </p>
      </div>

      <div className="spec-cols">
        <div className="spec-panel">
          <strong>Directions, where each is seen, and what argues against it</strong>
          <table className="spec-std">
            <thead><tr><th>Direction</th><th>Seen in</th><th>Against</th></tr></thead>
            <tbody>
              {DIRECTIONS.map((d) => (
                <tr key={d.id}>
                  <th>{d.name}</th>
                  <td>{d.seenIn.length > 0 ? <Links refs={d.seenIn} /> : <span className="part">—</span>}</td>
                  <td>{d.against}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="spec-panel">
          <strong>What the references ask</strong>
          <table className="spec-std">
            <thead><tr><th>Rule</th>{DIRECTIONS.map((d) => <th key={d.id} className="c">{d.id === "before" ? "Now" : d.id.toUpperCase()}</th>)}</tr></thead>
            <tbody>
              {RULES.map((r) => (
                <tr key={r.rule}>
                  <td>{r.rule}<br /><small className="part"><Links refs={r.refs} />{r.note && <> — {r.note}</>}</small></td>
                  {DIRECTIONS.map((d) => <td key={d.id} className={`c ${MARK_CLASS[r.marks[d.id]]}`}>{MARK[r.marks[d.id]]}</td>)}
                </tr>
              ))}
            </tbody>
          </table>
          <p className="spec-note">
            Not read: Material 3's site (it renders in JavaScript; its Android and web component docs were read instead, and say nothing about fields
            in dialogs); Google's current help no longer describes <i>Name current version</i>; Webflow (403); Atlassian's form-writing page (404);
            no NN/g article on first focus was found.
          </p>
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Specimen />);
