import { useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import AppLinkSheet, { type AppLinkSheetState } from "../src/components/app-link/AppLinkSheet";
import { INVALID_LINK, linkVerdict, type LinkContext, type LinkStop, type LinkVerdict } from "../src/components/app-link/appLink";
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
  { id: "invalid", label: "Stop · bad link", subject: null, state: answered(INVALID_LINK) },
];

/* ——— one view model every direction draws from ———————————————————————— */

const STOP_VERB: Record<NonNullable<LinkStop["action"]>, string> = { set_up: "Set up Mallard Cloud", retry: "Try again" };
const STOP_ICON: Record<LinkStop["severity"], IconName> = { info: "storage-mallard", warning: "alert-triangle", danger: "octagon-x" };
const STOP_WORD: Record<LinkStop["severity"], string> = { info: "Note", warning: "Warning", danger: "Error" };

interface Here { present: boolean; label: string | null; value: string | null; text: string; place: "computer" | "catalog" }
type View =
  | { kind: "looking"; subject: string | null }
  | {
    kind: "landing"; address: string; mark: IconName; standing: string; sub: string | null;
    here: Here; verb: string; task: string;
  }
  | { kind: "stop"; subject: string | null; severity: LinkStop["severity"]; message: string; verb: string | null };

function hereOf(text: string, catalog: boolean): Here {
  const match = /^(On this computer|In your Catalog): (.+)$/.exec(text);
  const place = catalog ? "catalog" : "computer";
  return match ? { present: true, label: match[1], value: match[2], text, place } : { present: false, label: null, value: null, text, place };
}

function viewOf(scene: Scene): View {
  if (scene.state.phase === "looking") return { kind: "looking", subject: scene.subject };
  const verdict = scene.state.verdict;
  if (verdict.kind === "stop") {
    return { kind: "stop", subject: scene.subject, severity: verdict.severity, message: verdict.message, verb: verdict.action ? STOP_VERB[verdict.action] : null };
  }
  const catalog = verdict.destination.kind.startsWith("catalog") || verdict.destination.kind.startsWith("add_catalog");
  return {
    kind: "landing", address: verdict.found.address, mark: verdict.found.mark, standing: verdict.found.standing,
    sub: verdict.found.sub, here: hereOf(verdict.found.here, catalog), verb: verdict.verb, task: verdict.verb.replace(/…$/, ""),
  };
}

/* The link spells its subject, so a sheet can title itself before the
   service answers; only the facts and the verb wait. */
const subjectOf = (view: View) => (view.kind === "landing" ? view.address : view.subject ?? "This link");
/** A title for the task: the verb's own words on a landing. */
const taskOf = (view: View) => (view.kind === "landing" ? view.task : view.kind === "stop" ? "Can’t open this link" : "Open link");
const placeName = (here: Here) => (here.place === "catalog" ? "Your Catalog" : "This computer");
const placeIcon = (here: Here): IconName => (here.place === "catalog" ? "catalog" : "computer");

/* ——— shared parts, all shipped classes ———————————————————————————————— */

/** The dialogs' head band: eyebrow, display title, one line under it, ×. */
function Head({ eyebrow, title, line, lead }: { eyebrow?: ReactNode; title: ReactNode; line?: ReactNode; lead?: ReactNode }) {
  return (
    <header className="sync-modal-header">
      {lead}
      <div>
        {eyebrow && <span className="sync-eyebrow">{eyebrow}</span>}
        <h1>{title}</h1>
        {line && <p>{line}</p>}
      </div>
      <button type="button" className="btn btn-ghost" aria-label="Close without opening the link"><Icon name="x" size={17} /></button>
    </header>
  );
}

/** The dialogs' foot band: Cancel, then the verb on the trailing edge. */
function Foot({ view, cancel = "Cancel", note }: { view: View; cancel?: string; note?: ReactNode }) {
  const verb = view.kind === "landing" ? view.verb : view.kind === "stop" ? view.verb : null;
  const exit = verb || view.kind === "looking" ? cancel : "Close";
  return (
    <footer className="sync-modal-footer">
      {note}
      <div>
        <button type="button" className="btn">{exit}</button>
        {view.kind === "looking"
          ? <button type="button" className="btn btn-primary" disabled>Open…</button>
          : verb && <button type="button" className="btn btn-primary">{verb}</button>}
      </div>
    </footer>
  );
}

/** The connect dialog's account line: where the link was answered. */
function Source({ host = HOST }: { host?: string }) {
  return (
    <div className="sync-bundle-context">
      <Icon name="storage-mallard" size={17} />
      <span><strong>Mallard Cloud · Rika321</strong><small>{host}</small></span>
    </div>
  );
}

/** The pickers' found row: the subject's glyph, name over its standing. */
function FoundRow({ view }: { view: View }) {
  if (view.kind === "looking") {
    return (
      <div className="storage-found-row with-verb">
        <span className="sync-storage-repository-icon"><span className="status-loader" /></span>
        <span className="storage-found-text">
          <span className="storage-found-name">{subjectOf(view)}</span>
          <span className="storage-found-verdict">Looking up on Mallard Cloud…</span>
        </span>
      </div>
    );
  }
  if (view.kind !== "landing") return null;
  return (
    <div className="storage-found-row with-verb">
      <span className="sync-storage-repository-icon"><Icon name={view.mark} size={14} /></span>
      <span className="storage-found-text">
        <span className="storage-found-name">{view.address}</span>
        <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {view.standing}</span>
        {view.sub && <span className="storage-found-verdict"><Icon name="message" size={12} /> {view.sub}</span>}
      </span>
    </div>
  );
}

/** The same row, for the far end: what is here, at the row's own size. */
function HereRow({ here }: { here: Here }) {
  return (
    <div className={`storage-found-row with-verb${here.present ? "" : " muted"}`}>
      <span className={`sync-storage-repository-icon os-place${here.present ? " present" : ""}`}><Icon name={placeIcon(here)} size={14} /></span>
      <span className="storage-found-text">
        <span className="storage-found-name">{here.present ? here.value : here.text}</span>
        <span className="storage-found-verdict">{placeName(here)}</span>
      </span>
      {here.present && <span className="storage-found-connected"><Icon name="check" size={12} /> Here</span>}
    </div>
  );
}

/** The app's notice recipe; a stop is a notice, never an empty list. */
function StopNotice({ view }: { view: Extract<View, { kind: "stop" }> }) {
  return (
    <div className={`notice ${view.severity} os-notice`} role={view.severity === "danger" ? "alert" : "status"}>
      <Icon name={STOP_ICON[view.severity]} size={13} className="notice-mark" />
      <div className="notice-body">
        <p className="notice-line"><strong className="notice-type">{STOP_WORD[view.severity]} ·</strong> {view.message}</p>
      </div>
    </div>
  );
}

/** One line of what is here, under a head band, as the dialogs' sub-line. */
function hereLine(view: View): ReactNode {
  if (view.kind === "looking") return <><span className="status-loader os-inline-loader" /> Looking up on Mallard Cloud…</>;
  if (view.kind === "stop") return view.subject ? <code>{view.subject}</code> : "The link could not be read.";
  return view.here.present ? <>{view.here.label}: <b>{view.here.value}</b></> : view.here.text;
}

/* ——— A · Confirm ——————————————————————————————————————————————————————— */

function ConfirmSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-a">
      <Head eyebrow="Open link" title={view.kind === "stop" ? taskOf(view) : subjectOf(view)} />
      <div className="sync-modal-body os-a-body">
        <Source />
        {view.kind === "stop" ? <StopNotice view={view} /> : (
          <dl className="os-facts">
            <dt>Mallard Cloud</dt>
            <dd>{view.kind === "landing"
              ? <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {view.standing}{view.sub && <> · {view.sub}</>}</span>
              : <span className="storage-found-verdict"><span className="status-loader" /> Looking up…</span>}
            </dd>
            <dt>{view.kind === "landing" ? placeName(view.here) : "This computer"}</dt>
            <dd className={view.kind === "landing" && view.here.present ? "present" : ""}>
              {view.kind === "landing" ? (view.here.present ? view.here.value : view.here.text) : "—"}
            </dd>
          </dl>
        )}
      </div>
      <Foot view={view} />
    </dialog>
  );
}

/* ——— B · Found row (the connect dialog's sibling) ————————————————————————— */

function FoundSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-b">
      <Head title={taskOf(view)} line={`A link from ${HOST} names this ${view.kind === "landing" && view.here.place === "catalog" ? "item" : "repository"}.`} />
      <div className="sync-modal-body">
        <Source />
        {view.kind === "stop" ? <StopNotice view={view} /> : (
          <div className="r2-step-list os-list">
            <div className="storage-found-list" aria-label="Repository at this address">
              <FoundRow view={view} />
              {view.kind === "landing" && <HereRow here={view.here} />}
              {view.kind === "looking" && <div className="storage-found-row with-verb muted os-skel-row" aria-hidden="true" />}
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
  const lead = (
    <span className="sync-storage-repository-icon os-lead">
      {view.kind === "looking" ? <span className="status-loader" />
        : <Icon name={view.kind === "landing" ? view.mark : STOP_ICON[view.severity]} size={16} />}
    </span>
  );
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-c">
      <Head
        lead={lead}
        title={subjectOf(view)}
        line={view.kind === "landing" ? <>Mallard Cloud · {view.standing}{view.sub && <> · {view.sub}</>}</> : "Mallard Cloud"}
      />
      <div className="sync-modal-body os-c-body">
        {view.kind === "stop" && <StopNotice view={view} />}
        {view.kind === "looking" && <small className="sync-setup-inline-status" role="status"><span className="status-loader" /> Looking up on Mallard Cloud…</small>}
        {view.kind === "landing" && (
          <div className={`notice ${view.here.present ? "success" : "info"} os-notice`} role="status">
            <Icon name={placeIcon(view.here)} size={13} className="notice-mark" />
            <div className="notice-body">
              <p className="notice-line">
                <strong className="notice-type">{placeName(view.here)} ·</strong>{" "}
                {view.here.present ? view.here.value : view.here.text}
              </p>
            </div>
          </div>
        )}
      </div>
      <Foot view={view} />
    </dialog>
  );
}

/* ——— D · From → To ———————————————————————————————————————————————————— */

/** A group box as the project page draws "Only in Mallard Cloud". */
function Group({ icon, title, absent, children }: { icon: IconName; title: ReactNode; absent?: boolean; children: ReactNode }) {
  return (
    <section className={`os-group${absent ? " absent" : ""}`}>
      <div className="sync-history-section-heading os-group-head"><h2><Icon name={icon} size={13} />{title}</h2></div>
      {children}
    </section>
  );
}

function RouteSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-d">
      <Head title={taskOf(view)} line={view.kind === "landing" ? `From Mallard Cloud to ${view.here.place === "catalog" ? "your Catalog" : "this computer"}.` : undefined} />
      <div className="sync-modal-body os-d-body">
        <Group icon="storage-mallard" title={<>On <span className="sync-history-storage-name">Mallard Cloud</span></>}>
          {view.kind === "stop" ? <div className="os-group-notice"><StopNotice view={view} /></div> : <FoundRow view={view} />}
        </Group>
        {view.kind !== "stop" && (
          <>
            <span className="os-d-arrow" aria-hidden="true"><Icon name="chevron-down" size={16} /></span>
            <Group
              icon={view.kind === "landing" ? placeIcon(view.here) : "computer"}
              title={view.kind === "landing" ? placeName(view.here) : "This computer"}
              absent={view.kind !== "landing" || !view.here.present}
            >
              {view.kind === "landing" ? <HereRow here={view.here} /> : <div className="storage-found-row with-verb muted os-skel-row" aria-hidden="true" />}
            </Group>
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
      <Head eyebrow="From Mallard Cloud" title={view.kind === "stop" ? taskOf(view) : subjectOf(view)} line={view.kind === "stop" ? undefined : hereLine(view)} />
      {view.kind === "stop" && <div className="sync-modal-body"><StopNotice view={view} /></div>}
      <Foot view={view} note={view.kind === "landing" ? <span className="storage-found-verdict ok"><Icon name="check-circle" size={12} /> {view.standing}{view.sub && <> · {view.sub}</>}</span> : undefined} />
    </dialog>
  );
}

/* ——— F · Current, cleaned ——————————————————————————————————————————————— */

function CleanedSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal sync-bundle-connection-dialog os-f">
      <Head title="Open from Mallard Cloud" />
      <div className="sync-modal-body">
        {view.kind === "stop" ? <StopNotice view={view} /> : (
          <div className="r2-step-list os-list">
            <div className="storage-found-list" aria-label="Repository at this address">
              <FoundRow view={view} />
              {view.kind === "landing" && <HereRow here={view.here} />}
              {view.kind === "looking" && <div className="storage-found-row with-verb muted os-skel-row" aria-hidden="true" />}
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
      <Head eyebrow="From Mallard Cloud" title={view.kind === "stop" ? taskOf(view) : subjectOf(view)} />
      <div className="sync-modal-body os-g-body">
        {view.kind === "stop" ? <StopNotice view={view} /> : (
          <div className="r2-step-list os-list">
            <div className="storage-found-list">
              {view.kind === "landing" ? <HereRow here={view.here} /> : <FoundRow view={view} />}
            </div>
          </div>
        )}
      </div>
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

const a = (href: string, label: string) => <a href={href} target="_blank" rel="noreferrer">{label}</a>;

const DIRECTIONS: Direction[] = [
  {
    id: "before", name: "Before · shipped", placement: "center",
    seenIn: <>The real <code>AppLinkSheet</code>, mounted</>,
    draw: (scene) => <AppLinkSheet key={scene.id} subject={scene.subject} state={scene.state} onVerb={() => {}} onClose={() => {}} />,
  },
  {
    id: "a", name: "A · Confirm", placement: "center", pick: true,
    seenIn: <>Short confirm in the dialog bands · {a("https://developers.raycast.com/information/lifecycle/deeplinks", "Raycast")}, {a("https://developer.apple.com/design/human-interface-guidelines/alerts", "HIG Alerts")}</>,
    draw: (scene) => <ConfirmSheet view={viewOf(scene)} />,
  },
  {
    id: "b", name: "B · Found row", placement: "center",
    seenIn: <>The Connect dialog’s account line and list, one row each end · {a("https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop", "GitHub Desktop Clone")}</>,
    draw: (scene) => <FoundSheet view={viewOf(scene)} />,
  },
  {
    id: "c", name: "C · Subject header", placement: "center",
    seenIn: <>{a("https://primer.style/product/components/dialog/guidelines/", "Primer Dialog")}, {a("https://carbondesignsystem.com/components/modal/usage/", "Carbon Modal")}; the head band holds the subject</>,
    draw: (scene) => <SubjectSheet view={viewOf(scene)} />,
  },
  {
    id: "d", name: "D · From → To", placement: "center",
    seenIn: <>The project page’s “Only in Mallard Cloud” group box, twice · {a("https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop", "GitHub Desktop")}</>,
    draw: (scene) => <RouteSheet view={viewOf(scene)} />,
  },
  {
    id: "e", name: "E · Window sheet", placement: "attached",
    seenIn: <>{a("https://developer.apple.com/design/human-interface-guidelines/sheets", "HIG Sheets (macOS)")}; bands only, hung from the bar</>,
    draw: (scene) => <WindowSheet view={viewOf(scene)} />,
  },
  {
    id: "f", name: "F · Current, cleaned", placement: "center",
    seenIn: <>The shipped sheet: narrower, the here line moved into the list, a stop as a notice</>,
    draw: (scene) => <CleanedSheet view={viewOf(scene)} />,
  },
  {
    id: "g", name: "G · Anchored card", placement: "anchored", change: "behaviour change",
    seenIn: <>{a("https://www.nngroup.com/articles/modal-nonmodal-dialog/", "NN/g non-modal")}; the same bands at popover size, no backdrop</>,
    draw: (scene) => <AnchoredCard view={viewOf(scene)} />,
  },
];

/* ——— the references, as rules ———————————————————————————————————————— */

type MarkValue = "yes" | "part" | "no" | string;
interface Rule { rule: string; from: string; marks: Record<string, MarkValue> }
const every = (value: MarkValue, overrides: Record<string, MarkValue> = {}): Record<string, MarkValue> => ({
  before: value, a: value, b: value, c: value, d: value, e: value, f: value, g: value, ...overrides,
});

const RULES: Rule[] = [
  { rule: "Built from the app’s dialog parts (bands, ×, account line, rows, notices)", from: "Mallard’s Connect, Setup and Library dialogs", marks: every("yes") },
  { rule: "Title names the subject or the task", from: "HIG Modality · Primer · Carbon · Fluent 2 · NN/g", marks: every("yes", { before: "no", f: "no" }) },
  { rule: "Narrow enough for one short message", from: "Primer ≤ 480 · Material ≤ 560", marks: { before: "680", a: "440", b: "520", c: "500", d: "500", e: "480", f: "520", g: "380" } },
  { rule: "One level of box: no frame inside the sheet", from: "Primer Dialog · HIG Modality", marks: every("yes", { before: "no", b: "part", d: "no", f: "part", g: "part" }) },
  { rule: "What is here, at the subject’s size", from: "NN/g confirmation · Primer ConfirmationDialog", marks: every("yes", { before: "no" }) },
  { rule: "Says where it came from", from: "Primer Dialog (deep links disorient) · Carbon label", marks: every("yes") },
  { rule: "Exits", from: "The app: × in the head band and Cancel in the foot", marks: every("× + Cancel", { g: "× + Not now" }) },
  { rule: "Primary on the trailing edge", from: "HIG · Material · Primer · Carbon · Atlassian · the app", marks: every("yes") },
  { rule: "A stop reads as a stop (the app’s notice)", from: "Primer degraded · Carbon inline notification", marks: every("yes", { before: "no" }) },
  { rule: "Looking up keeps the layout", from: "Primer loading", marks: every("yes", { before: "no" }) },
  { rule: "Where it sits", from: "HIG Sheets · NN/g modal cost", marks: { before: "centre", a: "centre", b: "centre", c: "centre", d: "centre", e: "under the bar", f: "centre", g: "corner, no backdrop" } },
];

function Mark({ value }: { value: MarkValue }) {
  if (value === "yes") return <td className="c yes">✓</td>;
  if (value === "no") return <td className="c no">✗</td>;
  if (value === "part") return <td className="c part">partly</td>;
  return <td className="c part">{value}</td>;
}

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

function Frame({ direction, scene, caption }: { direction: Direction; scene: Scene; caption: ReactNode }) {
  return (
    <figure className="os-cell">
      <figcaption>{caption}</figcaption>
      <div className={`os-frame ${direction.placement}`}>
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
        What a <code>mallard://</code> link from the website opens. <b>Before</b> is the shipped sheet, mounted for real.
        A–G are now built only from parts the app already ships: the dialogs’ head and foot bands (display title, round ×, Cancel and the
        yellow verb), the Connect dialog’s account line, the pickers’ found rows and uppercase chips, the project page’s group box, and the
        app’s notices. They differ in structure, not in look. Every frame gets the same link state through the real <code>linkVerdict</code>.
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
        <strong>What the references ask, and how each direction meets it</strong>
        <table className="spec-std">
          <thead>
            <tr><th>Rule</th><th>From</th>{DIRECTIONS.map((d) => <th key={d.id} className="c">{d.id === "before" ? "Before" : d.id.toUpperCase()}</th>)}</tr>
          </thead>
          <tbody>
            {RULES.map((rule) => (
              <tr key={rule.rule}>
                <td>{rule.rule}</td>
                <td className="from">{rule.from}</td>
                {DIRECTIONS.map((d) => <Mark key={d.id} value={rule.marks[d.id]} />)}
              </tr>
            ))}
          </tbody>
        </table>
        <p className="spec-note">
          Where the references disagreed, the app’s own dialogs now decide: every direction keeps the <b>×</b> in the head band beside Cancel,
          as Connect, Setup and the Library do; the verb stays on the <b>trailing edge</b> on every platform; titles carry <b>no full stop</b>,
          as every other dialog title in the app. The shipped verbs and their ellipses are unchanged.
        </p>
      </div>

      <div className="spec-panel">
        <strong>Recommendation: A · Confirm</strong>
        <p>
          A is the smallest sheet that still looks like every other Mallard dialog: the subject as the display title, the Connect dialog’s
          account line to say where the answer came from, two labelled lines (what Mallard Cloud has, what is here) at the same size, and the
          usual foot band. A stop keeps the bands and puts the app’s notice where the two lines were.
        </p>
        <ul>
          <li><b>B</b> and <b>F</b> keep the pickers’ list box, so they sit closest to the Connect dialog; the box holds two rows, the subject and its place here.</li>
          <li><b>C</b> moves the subject into the head band, with its glyph, and leaves the body to one notice.</li>
          <li><b>D</b> draws both ends as the project page’s group boxes; worth its height on Set up and Add, where the far end does not exist yet.</li>
          <li><b>E</b> is bands only, hung from the bar; <b>G</b> is the same bands at popover size with no backdrop (behaviour change).</li>
        </ul>
        <p className="spec-note">
          Whatever is picked, two fixes ride along: focus opens on the verb, not on ×; and the sheet opens titled with the link’s subject, with a
          placeholder where the answer will land, so it does not grow when the answer arrives.
        </p>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Specimen />);
