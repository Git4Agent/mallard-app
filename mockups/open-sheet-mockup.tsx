import { useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import AppLinkSheet, { type AppLinkSheetState } from "../src/components/app-link/AppLinkSheet";
import { invalidLink, linkVerdict, type LinkContext, type LinkStop, type LinkVerdict } from "../src/components/app-link/appLink";
import type { CatalogCapabilityDescriptor, CatalogScopeView } from "../src/components/storage-catalog/types";
import type { AppLinkAccount, AppLinkAnswer, AppLinkRepository } from "../src/types";
import "../src/App.css";
import "./open-sheet-mockup.css";

/**
 * Specimen: the Open sheet — what a `mallard://` link from the website
 * opens in the desktop (src/components/app-link/AppLinkSheet.tsx).
 *
 * Every frame is fed the same fourteen link states, each one the real
 * `linkVerdict` run on a stand-in answer, so the words are the shipped
 * words. *Before* is the shipped `AppLinkSheet` itself, mounted in place
 * (its `showModal()` is turned into `show()` so eight sheets can sit side by
 * side). A–G are built only from parts the app already ships (bands, ×,
 * account line, found rows, notices); they differ in structure only.
 * `npm run dev`, then /mockups/open-sheet-mockup.html
 * (`?view=compare|one&scene=<id>&dir=<id>&theme=light|dark`).
 */

/* A modal sheet takes the top layer and the viewport's centre; eight of
   them in one page must each stay in their own frame. */
HTMLDialogElement.prototype.showModal = function showInPlace(this: HTMLDialogElement) {
  this.show();
};

/* ——— link states, through the real verdict ————————————————————————— */

const HOST = "mallard-ai.com";
const PET = { provider: "codex", resource_kind: "desktop_pet", format: "openai.codex-pet", format_version: 2 };
const PETS = {
  ...PET, provider_label: "Codex", label: "Pets v2", singular_label: "Pet v2",
  presentation: { icon: "pet-v2", kind_label: "Pets v2", format_label: "Codex pet v2" }, capture: "directory_tree", apply: "automatic", targets: [],
} as unknown as CatalogCapabilityDescriptor;
const PETS_ROW = {
  scope: { scope_id: "scope-pets", profile_id: "profile-codex", ...PET, revision: 1, created_at: 0, updated_at: 0 },
  profile: { display_name: "Default Codex" }, label: "Pets v2",
} as unknown as CatalogScopeView;

const seen = (overrides: Partial<AppLinkRepository>): AppLinkRepository => ({
  repository_id: "repo_maya_portal", address: "maya/portal", repository_class: "git_project", bundle_id: "b".repeat(32),
  catalog: null, visibility: "public", role: null, ...overrides,
});
const PORTAL = seen({});
const API = seen({ repository_id: "repo_api_server", address: "Rika321/api-server", bundle_id: "a".repeat(32), visibility: "private", role: "owner" });
const LONG = seen({
  repository_id: "repo_long", address: "northwind-research/customer-billing-reconciliation", bundle_id: "d".repeat(32),
  visibility: "private", role: "writer",
});
const MOCHI = seen({ repository_id: "repo_maya_mochi", address: "maya/Mochi", repository_class: "catalog", bundle_id: null, catalog: { ...PET, resource_id: "mochi" } });
const DUCK = seen({ repository_id: "repo_duck", address: "Rika321/Duck", repository_class: "catalog", bundle_id: null, catalog: { ...PET, resource_id: "duck" }, role: "owner" });
const FABLE = seen({
  repository_id: "repo_fable", address: "Rika321/Fable", repository_class: "catalog", bundle_id: null,
  catalog: { ...PET, resource_id: "fable" }, role: "owner", visibility: "private",
});

const account = (sight: AppLinkAccount["sight"]): AppLinkAccount => ({
  storage_id: "storage-personal", storage_name: "Mallard Cloud", account_login: "Rika321", sight,
});
const found = (repository: AppLinkRepository) => account({ kind: "found", repository });
const answer = (address: string, accounts: AppLinkAccount[], host = HOST): AppLinkAnswer => ({
  host, address, accounts, can_set_up: host === HOST,
});

const CONTEXT: LinkContext = {
  links: [{ local_project_id: "project-api", storage_id: "storage-personal", bundle_id: "a".repeat(32), managed_cloud_repository_id: "repo_api_server" }],
  projects: [{ local_project_id: "project-api", display_name: "api-server", local_alias: null }],
  scopes: [PETS_ROW],
  capabilities: [PETS],
  places: {},
};
const NO_PETS_ROW: LinkContext = { ...CONTEXT, scopes: [] };

interface Scene { id: string; label: string; subject: string | null; state: AppLinkSheetState }
const answered = (verdict: LinkVerdict): AppLinkSheetState => ({ phase: "answered", verdict });

const SCENES: Scene[] = [
  { id: "looking", label: "Looking up", subject: "maya/portal", state: { phase: "looking" } },
  { id: "project-here", label: "Project · here", subject: API.address, state: answered(linkVerdict(answer(API.address, [found(API)]), CONTEXT)) },
  { id: "project-new", label: "Project · new", subject: PORTAL.address, state: answered(linkVerdict(answer(PORTAL.address, [found(PORTAL)]), CONTEXT)) },
  { id: "session-here", label: "Session · here", subject: API.address, state: answered(linkVerdict(answer(API.address, [found(API)]), CONTEXT, "row-session")) },
  { id: "session-new", label: "Session · new", subject: PORTAL.address, state: answered(linkVerdict(answer(PORTAL.address, [found(PORTAL)]), CONTEXT, "row-session")) },
  { id: "long", label: "Long address", subject: LONG.address, state: answered(linkVerdict(answer(LONG.address, [found(LONG)]), CONTEXT)) },
  { id: "item-new", label: "Pet · new", subject: MOCHI.address, state: answered(linkVerdict(answer(MOCHI.address, [found(MOCHI)]), CONTEXT)) },
  { id: "item-here", label: "Pet · in Catalog", subject: DUCK.address, state: answered(linkVerdict(answer(DUCK.address, [found(DUCK)]), CONTEXT)) },
  { id: "item-no-row", label: "Pet · no row", subject: FABLE.address, state: answered(linkVerdict(answer(FABLE.address, [found(FABLE)]), NO_PETS_ROW)) },
  { id: "sign-in", label: "Stop · sign in", subject: PORTAL.address, state: answered(linkVerdict(answer(PORTAL.address, []), CONTEXT)) },
  { id: "other-host", label: "Stop · other host", subject: PORTAL.address, state: answered(linkVerdict(answer(PORTAL.address, [], "staging.mallard-ai.com"), CONTEXT)) },
  { id: "unseen", label: "Stop · can’t see", subject: "acme/billing", state: answered(linkVerdict(answer("acme/billing", [account({ kind: "not_found" })]), CONTEXT)) },
  {
    id: "failed", label: "Stop · no answer", subject: PORTAL.address,
    state: answered(linkVerdict(answer(PORTAL.address, [account({ kind: "failed", message: "Mallard Cloud could not be reached. Check the network and try again." })]), CONTEXT)),
  },
  { id: "invalid", label: "Stop · bad link", subject: null, state: answered(invalidLink()) },
];

/* ——— one view model every direction draws from ———————————————————————— */

/* The verdict gives each thing as an icon and a word or two, the sentence
   kept for the tooltip; every direction draws from that. */

const STOP_VERB: Record<NonNullable<LinkStop["action"]>, string> = { set_up: "Set up Mallard Cloud", retry: "Try again" };

interface Fact { icon: IconName; word: string }
interface Here { present: boolean; icon: IconName; word: string; text: string }
type View =
  | { kind: "looking"; subject: string | null }
  | { kind: "landing"; address: string; mark: IconName; facts: Fact[]; here: Here; verb: string; task: string }
  | { kind: "stop"; subject: string | null; severity: LinkStop["severity"]; icon: IconName; title: string; message: string; verb: string | null };

function viewOf(scene: Scene): View {
  if (scene.state.phase === "looking") return { kind: "looking", subject: scene.subject };
  const verdict = scene.state.verdict;
  if (verdict.kind === "stop") {
    return {
      kind: "stop", subject: scene.subject, severity: verdict.severity, message: verdict.message,
      icon: verdict.icon, title: verdict.title, verb: verdict.action ? STOP_VERB[verdict.action] : null,
    };
  }
  const { here } = verdict.found;
  return {
    kind: "landing", address: verdict.found.address, mark: verdict.found.mark, facts: verdict.found.facts,
    here: { present: here.present, icon: here.place === "catalog" ? "catalog" : "computer", word: here.word, text: here.sentence },
    verb: verdict.verb, task: verdict.verb.replace(/…$/, ""),
  };
}

const subjectOf = (view: View) => (view.kind === "landing" ? view.address : view.subject ?? "Link");
/** A list-style sheet titles itself with the task; a stop names its subject there, the notice below says what went wrong. */
const taskOf = (view: View) => (view.kind === "landing" ? view.task : view.kind === "stop" ? view.subject ?? "Link" : "Open");

/* ——— shared parts, all shipped classes ———————————————————————————————— */

function CloseX() {
  return <button type="button" className="btn btn-ghost" aria-label="Close without opening the link"><Icon name="x" size={17} /></button>;
}

/** The dialogs' head band: an optional glyph, the display title, ×. */
function Head({ title, lead, line }: { title: ReactNode; lead?: ReactNode; line?: ReactNode }) {
  return (
    <header className="sync-modal-header">
      {lead}
      <div>
        <h1>{title}</h1>
        {line && <p className="os-line">{line}</p>}
      </div>
      <CloseX />
    </header>
  );
}

/** The dialogs' foot band: Cancel, then the verb on the trailing edge. */
function Foot({ view, cancel = "Cancel" }: { view: View; cancel?: string }) {
  const verb = view.kind === "landing" ? view.verb : view.kind === "stop" ? view.verb : null;
  const exit = verb || view.kind === "looking" ? cancel : "Close";
  return (
    <footer className="sync-modal-footer">
      <div>
        <button type="button" className="btn">{exit}</button>
        {view.kind === "looking"
          ? <button type="button" className="btn btn-primary" disabled>Open…</button>
          : verb && <button type="button" className="btn btn-primary">{verb}</button>}
      </div>
    </footer>
  );
}

/** A glyph in the app's 26 px icon square; a spinner while asking. */
function Glyph({ icon, tone = "", spin = false, size = "" }: { icon?: IconName; tone?: string; spin?: boolean; size?: string }) {
  return (
    <span className={`sync-storage-repository-icon os-glyph ${tone} ${size}`}>
      {spin ? <span className="status-loader" /> : icon && <Icon name={icon} size={size === "lg" ? 18 : 14} />}
    </span>
  );
}

/** Icon-and-word facts: Public, Owner, 1 session. */
function Facts({ view }: { view: View }) {
  if (view.kind === "looking") return <span className="os-facts" aria-hidden="true"><span className="os-skel" /><span className="os-skel short" /></span>;
  if (view.kind !== "landing") return null;
  return (
    <span className="os-facts">
      {view.facts.map((fact) => <span key={fact.word} className="storage-found-verdict"><Icon name={fact.icon} size={12} />{fact.word}</span>)}
    </span>
  );
}

/** Where it stands here, in a few words: green with a check when present. */
function HereWord({ view }: { view: View }) {
  if (view.kind === "looking") return <span className="os-skel" aria-hidden="true" />;
  if (view.kind !== "landing") return null;
  return (
    <span className={`os-here${view.here.present ? " present" : ""}`} title={view.here.text}>
      {view.here.present && <Icon name="check-circle" size={13} />}{view.here.word}
    </span>
  );
}

/** A stop: its glyph and its short title in the severity's notice tint. */
function StopLine({ view }: { view: Extract<View, { kind: "stop" }> }) {
  return (
    <div className={`notice ${view.severity} os-stop`} role={view.severity === "danger" ? "alert" : "status"} title={view.message}>
      <Icon name={view.icon} size={14} className="notice-mark" />
      <p className="notice-line"><strong className="notice-lead">{view.title}</strong></p>
    </div>
  );
}

/** One row: glyph, a name, a trailing word. */
function Row({ glyph, name, tail, muted = false }: { glyph: ReactNode; name: ReactNode; tail?: ReactNode; muted?: boolean }) {
  return (
    <div className={`storage-found-row with-verb os-row${muted ? " muted" : ""}`}>
      {glyph}
      <span className="storage-found-text"><span className="storage-found-name">{name}</span></span>
      {tail && <span className="storage-found-facts">{tail}</span>}
    </div>
  );
}

const subjectGlyph = (view: View) => view.kind === "landing" ? <Glyph icon={view.mark} /> : <Glyph spin />;
const placeGlyph = (view: View) => view.kind === "landing"
  ? <Glyph icon={view.here.icon} tone={view.here.present ? "ok" : ""} />
  : <Glyph icon="computer" />;

/* ——— A · Confirm ——————————————————————————————————————————————————————— */

function ConfirmSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-a">
      <Head title={view.kind === "stop" ? view.title : subjectOf(view)} lead={view.kind === "stop" ? <Glyph icon={view.icon} tone={view.severity} size="lg" /> : undefined} />
      {view.kind !== "stop" ? (
        <div className="sync-modal-body os-pairs">
          <Glyph icon="storage-mallard" /><Facts view={view} />
          {placeGlyph(view)}<HereWord view={view} />
        </div>
      ) : view.subject && <div className="sync-modal-body os-pairs"><Glyph icon="link" /><code className="os-code">{view.subject}</code></div>}
      <Foot view={view} />
    </dialog>
  );
}

/* ——— B · Found row ————————————————————————————————————————————————————— */

function FoundSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-b">
      <Head title={taskOf(view)} />
      <div className="sync-modal-body">
        {view.kind === "stop" ? <StopLine view={view} /> : (
          <div className="r2-step-list os-list">
            <div className="storage-found-list">
              <Row glyph={subjectGlyph(view)} name={subjectOf(view)} tail={<Facts view={view} />} />
              <Row glyph={placeGlyph(view)} name={<HereWord view={view} />} muted={view.kind !== "landing" || !view.here.present} />
            </div>
          </div>
        )}
      </div>
      <Foot view={view} />
    </dialog>
  );
}

/* ——— C · Subject header ————————————————————————————————————————————————— */

function SubjectSheet({ view }: { view: View }) {
  const lead = view.kind === "stop" ? <Glyph icon={view.icon} tone={view.severity} size="lg" />
    : view.kind === "landing" ? <Glyph icon={view.mark} size="lg" /> : <Glyph spin size="lg" />;
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-c">
      <Head lead={lead} title={view.kind === "stop" ? view.title : subjectOf(view)} line={view.kind === "stop" ? view.subject : <Facts view={view} />} />
      {view.kind !== "stop" && (
        <div className="sync-modal-body os-c-body">
          {placeGlyph(view)}<HereWord view={view} />
        </div>
      )}
      <Foot view={view} />
    </dialog>
  );
}

/* ——— D · From → To ———————————————————————————————————————————————————— */

function End({ glyph, name, tail, absent = false }: { glyph: ReactNode; name: ReactNode; tail?: ReactNode; absent?: boolean }) {
  return (
    <div className={`os-end${absent ? " absent" : ""}`}>
      {glyph}
      <span className="os-end-name">{name}</span>
      {tail}
    </div>
  );
}

function RouteSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-d">
      <Head title={taskOf(view)} />
      <div className="sync-modal-body os-d-body">
        {view.kind === "stop" ? <StopLine view={view} /> : (
          <>
            <End glyph={<Glyph icon="storage-mallard" />} name={subjectOf(view)} tail={<Facts view={view} />} />
            <span className="os-d-arrow" aria-hidden="true"><Icon name="chevron-down" size={16} /></span>
            <End glyph={placeGlyph(view)} name={<HereWord view={view} />} absent={view.kind !== "landing" || !view.here.present} />
          </>
        )}
      </div>
      <Foot view={view} />
    </dialog>
  );
}

/* ——— E · Window sheet ———————————————————————————————————————————————————— */

function WindowSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-e">
      <Head
        lead={view.kind === "stop" ? <Glyph icon={view.icon} tone={view.severity} size="lg" /> : subjectGlyph(view)}
        title={view.kind === "stop" ? view.title : subjectOf(view)}
        line={view.kind === "stop" ? view.subject : <><Facts view={view} /><span className="os-sep" />{view.kind === "landing" && <Icon name={view.here.icon} size={12} />}<HereWord view={view} /></>}
      />
      <Foot view={view} />
    </dialog>
  );
}

/* ——— F · Current, cleaned ——————————————————————————————————————————————— */

function CleanedSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-f">
      <Head lead={<Glyph icon="storage-mallard" size="lg" />} title="Open link" />
      <div className="sync-modal-body">
        {view.kind === "stop" ? <StopLine view={view} /> : (
          <div className="r2-step-list os-list">
            <div className="storage-found-list">
              <Row glyph={subjectGlyph(view)} name={subjectOf(view)} tail={<Facts view={view} />} />
              <Row glyph={placeGlyph(view)} name={<HereWord view={view} />} muted={view.kind !== "landing" || !view.here.present} />
            </div>
          </div>
        )}
      </div>
      <Foot view={view} />
    </dialog>
  );
}

/* ——— G · Anchored card (behaviour change) ————————————————————————————————— */

function AnchoredCard({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-g" aria-modal="false">
      <Head
        lead={view.kind === "stop" ? <Glyph icon={view.icon} tone={view.severity} /> : subjectGlyph(view)}
        title={view.kind === "stop" ? view.title : subjectOf(view)}
        line={view.kind === "stop" ? view.subject : <>{view.kind === "landing" && <Icon name={view.here.icon} size={12} />}<HereWord view={view} /></>}
      />
      <Foot view={view} cancel="Not now" />
    </dialog>
  );
}

/* ——— directions ——————————————————————————————————————————————————————— */

type Placement = "center" | "attached" | "anchored";
interface Direction {
  id: string;
  name: string;
  placement: Placement;
  seenIn: ReactNode;
  draw: (scene: Scene) => ReactNode;
  pick?: boolean;
  change?: string;
}

const DIRECTIONS: Direction[] = [
  {
    id: "before", name: "Shipped · A", placement: "center", seenIn: "The real AppLinkSheet, now A",
    draw: (scene) => <AppLinkSheet key={scene.id} subject={scene.subject} state={scene.state} onVerb={() => {}} onClose={() => {}} />,
  },
  { id: "a", name: "A · Confirm", placement: "center", pick: true, seenIn: "Two icon lines", draw: (scene) => <ConfirmSheet view={viewOf(scene)} /> },
  { id: "b", name: "B · Found row", placement: "center", seenIn: "Two rows in a list", draw: (scene) => <FoundSheet view={viewOf(scene)} /> },
  { id: "c", name: "C · Subject header", placement: "center", seenIn: "Subject in the head", draw: (scene) => <SubjectSheet view={viewOf(scene)} /> },
  { id: "d", name: "D · From → To", placement: "center", seenIn: "Cloud ↓ here", draw: (scene) => <RouteSheet view={viewOf(scene)} /> },
  { id: "e", name: "E · Window sheet", placement: "attached", seenIn: "Head and foot only", draw: (scene) => <WindowSheet view={viewOf(scene)} /> },
  { id: "f", name: "F · Current, cleaned", placement: "center", seenIn: "Shipped frame, trimmed", draw: (scene) => <CleanedSheet view={viewOf(scene)} /> },
  { id: "g", name: "G · Anchored card", placement: "anchored", change: "no backdrop", seenIn: "Corner popover", draw: (scene) => <AnchoredCard view={viewOf(scene)} /> },
];

/* ——— frames ——————————————————————————————————————————————————————————— */

function StandInApp() {
  return (
    <div className="os-app" aria-hidden="true">
      <div className="os-app-bar">
        <span className="os-app-crumb">Projects</span>
        <span className="os-app-crumb dim">api-server</span>
        <span className="os-app-pill" />
        <span className="os-app-pill primary" />
      </div>
      <div className="os-app-side">
        {Array.from({ length: 6 }, (_, i) => <span key={i} className="os-app-row" style={{ width: `${70 - i * 6}%` }} />)}
      </div>
      <div className="os-app-main">
        {Array.from({ length: 4 }, (_, i) => <span key={i} className="os-app-card" />)}
      </div>
    </div>
  );
}

/** The words a reader meets in a sheet, the glyphs left out. */
function useWordCount() {
  const ref = useRef<HTMLDivElement>(null);
  const [words, setWords] = useState<number | null>(null);
  useEffect(() => {
    const dialog = ref.current?.querySelector("dialog");
    setWords(dialog ? dialog.innerText.split(/\s+/).filter((word) => /\w/.test(word)).length : null);
  });
  return { ref, words };
}

function Frame({ direction, scene, caption }: { direction: Direction; scene: Scene; caption: ReactNode }) {
  const { ref, words } = useWordCount();
  return (
    <figure className="os-cell">
      <figcaption>{caption}{words !== null && <span className="os-count">{words} words</span>}</figcaption>
      <div ref={ref} className={`os-frame ${direction.placement}`}>
        <StandInApp />
        {direction.placement !== "anchored" && <div className="os-scrim" />}
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
  const [sceneId, setScene] = useState(pick("scene", SCENES.map((s) => s.id), "session-new"));
  const [dirId, setDir] = useState(pick("dir", DIRECTIONS.map((d) => d.id), "a"));
  const [theme, setTheme] = useState<Theme>(pick("theme", ["light", "dark"] as const, "light"));
  document.documentElement.dataset.theme = theme;
  const scene = SCENES.find((s) => s.id === sceneId)!;
  const direction = DIRECTIONS.find((d) => d.id === dirId)!;

  const nameOf = (d: Direction) => (
    <>
      <b>{d.name}</b>
      {d.pick && <em>recommended</em>}
      {d.change && <em className="change">{d.change}</em>}
      <small>{d.seenIn}</small>
    </>
  );

  return (
    <div className="spec">
      <h1 className="spec-title">Open sheet — directions</h1>
      <p className="spec-sub">
        What a <code>mallard://</code> link opens. <b>Shipped</b> is the real sheet, now built as A; A–G say the same in icons and a few words, in the
        app’s own dialog parts. Hover a short line for the full sentence.
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

      <div className="os-grid">
        {mode === "compare"
          ? DIRECTIONS.map((d) => <Frame key={`${d.id}-${scene.id}`} direction={d} scene={scene} caption={nameOf(d)} />)
          : SCENES.map((s) => <Frame key={`${direction.id}-${s.id}`} direction={direction} scene={s} caption={<b>{s.label}</b>} />)}
      </div>

      <div className="spec-panel">
        <strong>Pick: A · Confirm</strong>
        <p>Subject as the title, two icon lines (cloud, here), the usual Cancel and verb. Whichever is built should open with focus on the verb.</p>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Specimen />);
