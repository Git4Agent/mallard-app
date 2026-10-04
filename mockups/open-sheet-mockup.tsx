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
 * side). A–G are drawn here with the app's own box, pill and type tokens;
 * only their structure, placement and exits come from the references.
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

/* ——— shared parts ———————————————————————————————————————————————————— */

function Tile({ view, size }: { view: View; size: "md" | "lg" }) {
  const iconSize = size === "lg" ? 26 : 20;
  if (view.kind === "looking") return <span className={`os-tile ${size}`}><span className="status-loader" /></span>;
  if (view.kind === "stop") return <span className={`os-tile ${size} ${view.severity}`}><Icon name={STOP_ICON[view.severity]} size={iconSize} /></span>;
  return <span className={`os-tile ${size}`}><Icon name={view.mark} size={iconSize} /></span>;
}

function Facts({ view, source = false }: { view: Extract<View, { kind: "landing" }>; source?: boolean }) {
  return (
    <span className="os-facts">
      {source && <span className="os-chip"><Icon name="storage-mallard" size={12} /> Mallard Cloud</span>}
      <span className="os-chip ok"><Icon name="check-circle" size={12} /> {view.standing}</span>
      {view.sub && <span className="os-chip"><Icon name="message" size={12} /> {view.sub}</span>}
    </span>
  );
}

function HereLine({ here, className = "" }: { here: Here; className?: string }) {
  return (
    <p className={`os-here ${here.present ? "present" : "absent"} ${className}`}>
      <Icon name={here.place === "catalog" ? "catalog" : "computer"} size={14} />
      {here.present ? <span>{here.label}: <b>{here.value}</b></span> : <span>{here.text}</span>}
    </p>
  );
}

function Verbs({ view, cancel = "Cancel", wide = false }: { view: View; cancel?: string; wide?: boolean }) {
  const verb = view.kind === "landing" ? view.verb : view.kind === "stop" ? view.verb : null;
  const exit = verb || view.kind === "looking" ? cancel : "Close";
  const w = wide ? " os-wide" : "";
  return wide ? (
    <div className="os-verbs stacked">
      {verb && <button type="button" className={`btn btn-primary${w}`}>{verb}</button>}
      <button type="button" className={`btn${verb ? " btn-ghost" : ""}${w}`}>{exit}</button>
    </div>
  ) : (
    <div className="os-verbs">
      <button type="button" className="btn">{exit}</button>
      {verb && <button type="button" className="btn btn-primary">{verb}</button>}
    </div>
  );
}

function CloseX() {
  return (
    <button type="button" className="btn btn-ghost os-x" aria-label="Close without opening the link">
      <Icon name="x" size={17} />
    </button>
  );
}

/** The app's notice recipe, minus Dismiss: in a sheet the sheet's own exit closes it. */
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

/* The link spells its subject, so a sheet can title itself before the
   service answers; only the facts and the verb wait (Primer loading: the
   indicator stands where the content will land, without a jump). */
const subjectOf = (subject: string | null) => subject ?? "This link";
/** `chips`: the direction draws its chips in the body, so their row waits too. */
const Waiting = ({ chips = true }: { chips?: boolean }) => (
  <>
    {chips && <span className="os-facts" aria-hidden="true"><span className="os-chip os-skel" /><span className="os-chip os-skel wide" /></span>}
    <p className="os-wait"><span className="status-loader" /> Looking up on Mallard Cloud…</p>
  </>
);

/* ——— A · Alert ———————————————————————————————————————————————————————— */

function AlertSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal os-alert">
      <Tile view={view} size="lg" />
      {view.kind === "looking" && <><h1>{subjectOf(view.subject)}</h1><Waiting /></>}
      {view.kind === "stop" && <h1 className="os-stop-title">{view.message}</h1>}
      {view.kind === "landing" && (
        <>
          <h1>{view.address}</h1>
          <Facts view={view} source />
          <HereLine here={view.here} />
        </>
      )}
      <Verbs view={view} />
    </dialog>
  );
}

/* ——— B · Invite card ——————————————————————————————————————————————————— */

function InviteSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal os-invite">
      <p className="os-eyebrow"><Icon name="storage-mallard" size={13} /> From Mallard Cloud</p>
      <Tile view={view} size="lg" />
      {view.kind === "looking" && <><h1>{subjectOf(view.subject)}</h1><Waiting /></>}
      {view.kind === "stop" && <h1 className="os-stop-title">{view.message}</h1>}
      {view.kind === "landing" && (
        <>
          <h1>{view.address}</h1>
          <Facts view={view} />
          <HereLine here={view.here} className="banded" />
        </>
      )}
      <Verbs view={view} wide />
    </dialog>
  );
}

/* ——— C · Subject header ————————————————————————————————————————————————— */

function SubjectSheet({ view }: { view: View }) {
  const title = view.kind === "landing" ? view.address : view.subject ?? "This link";
  return (
    <dialog open className="sync-modal os-subject">
      <header className="sync-modal-header">
        {view.kind === "landing"
          ? <span className="os-tile sm"><Icon name={view.mark} size={16} /></span>
          : <span className="os-tile sm"><Icon name="storage-mallard" size={16} /></span>}
        <div>
          <h1>{title}</h1>
          <p>
            {view.kind === "landing" ? <>Mallard Cloud · {view.standing}</> : "Mallard Cloud"}
          </p>
        </div>
        <CloseX />
      </header>
      <div className="sync-modal-body os-subject-body">
        {view.kind === "looking" && <Waiting chips={false} />}
        {view.kind === "stop" && <StopNotice view={view} />}
        {view.kind === "landing" && (
          <>
            {view.sub && <p className="os-line"><Icon name="message" size={14} /> {view.sub}</p>}
            <HereLine here={view.here} />
          </>
        )}
      </div>
      <footer className="sync-modal-footer"><Verbs view={view} /></footer>
    </dialog>
  );
}

/* ——— D · From → To ———————————————————————————————————————————————————— */

/* The far end's label already says where; its value says only what is
   there. Mock copy: the shipped `here` sentences each name the place. */
function routeValue(here: Here): string {
  if (here.present) return here.value!;
  const noRow = /^No (.+) row on this computer\.$/.exec(here.text);
  return noRow ? `No ${noRow[1]} row yet` : "Not here yet";
}
const routeNote = (here: Here) => here.text.match(/Set up the project, then use the link again\./)?.[0] ?? null;

function RouteSheet({ view }: { view: View }) {
  const title = view.kind === "landing" ? view.task : "Open from Mallard Cloud";
  return (
    <dialog open className="sync-modal os-route">
      <header className="sync-modal-header">
        <div><h1>{title}</h1></div>
        <CloseX />
      </header>
      <div className="sync-modal-body os-route-body">
        <div className={`os-end from ${view.kind === "stop" ? view.severity : ""}`}>
          <span className="os-tile md"><Icon name="storage-mallard" size={18} /></span>
          <span className="os-end-text">
            <small>Mallard Cloud</small>
            {view.kind === "landing" && (
              <>
                <b className="os-end-name"><Icon name={view.mark} size={14} /> {view.address}</b>
                <Facts view={view} />
              </>
            )}
            {view.kind === "looking" && <><b className="os-end-name">{subjectOf(view.subject)}</b><Waiting /></>}
            {view.kind === "stop" && (
              <>
                {view.subject && <b className="os-end-name">{view.subject}</b>}
                <span className={`os-end-verdict ${view.severity}`}><Icon name={STOP_ICON[view.severity]} size={13} /> {view.message}</span>
              </>
            )}
          </span>
        </div>
        {view.kind === "landing" && (
          <>
            <span className="os-route-arrow" aria-hidden="true"><Icon name="chevron-down" size={16} /></span>
            <div className={`os-end to ${view.here.present ? "present" : "absent"}`}>
              <span className="os-tile md"><Icon name={view.here.place === "catalog" ? "catalog" : "computer"} size={18} /></span>
              <span className="os-end-text">
                <small>{view.here.place === "catalog" ? "Your Catalog" : "This computer"}</small>
                <b className="os-end-name">{routeValue(view.here)}</b>
                {routeNote(view.here) && <span className="os-end-note">{routeNote(view.here)}</span>}
              </span>
            </div>
          </>
        )}
      </div>
      <footer className="sync-modal-footer"><Verbs view={view} /></footer>
    </dialog>
  );
}

/* ——— E · Window sheet ———————————————————————————————————————————————————— */

function WindowSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal os-window">
      <div className="os-window-body">
        <Tile view={view} size="md" />
        <div className="os-window-text">
          {view.kind === "looking" && <><h1>{subjectOf(view.subject)}</h1><Waiting /></>}
          {view.kind === "stop" && <h1 className="os-stop-title">{view.message}</h1>}
          {view.kind === "landing" && (
            <>
              <h1>{view.address}</h1>
              <Facts view={view} source />
              <HereLine here={view.here} />
            </>
          )}
        </div>
      </div>
      <Verbs view={view} />
    </dialog>
  );
}

/* ——— F · Current, cleaned ——————————————————————————————————————————————— */

function CleanedSheet({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal os-cleaned">
      <header className="sync-modal-header">
        <div><h1>Open from Mallard Cloud</h1></div>
        <CloseX />
      </header>
      <div className="sync-modal-body os-cleaned-body">
        {view.kind === "looking" && (
          <div className="os-found">
            <span className="sync-storage-repository-icon"><span className="status-loader" /></span>
            <span className="os-found-text"><b>{subjectOf(view.subject)}</b><Waiting /></span>
          </div>
        )}
        {view.kind === "stop" && <StopNotice view={view} />}
        {view.kind === "landing" && (
          <div className="os-found">
            <span className="sync-storage-repository-icon"><Icon name={view.mark} size={14} /></span>
            <span className="os-found-text">
              <b>{view.address}</b>
              <Facts view={view} />
              <HereLine here={view.here} />
            </span>
          </div>
        )}
      </div>
      <footer className="sync-modal-footer"><Verbs view={view} /></footer>
    </dialog>
  );
}

/* ——— G · Anchored card (behaviour change) ————————————————————————————————— */

function AnchoredCard({ view }: { view: View }) {
  return (
    <dialog open className="sync-modal os-anchored" aria-modal="false">
      <div className="os-anchored-head">
        <span><Icon name="storage-mallard" size={13} /> From Mallard Cloud</span>
        <CloseX />
      </div>
      <div className="os-anchored-body">
        <Tile view={view} size="md" />
        <div className="os-window-text">
          {view.kind === "looking" && <><h1>{subjectOf(view.subject)}</h1><Waiting /></>}
          {view.kind === "stop" && <h1 className="os-stop-title">{view.message}</h1>}
          {view.kind === "landing" && (
            <>
              <h1>{view.address}</h1>
              <Facts view={view} />
              <HereLine here={view.here} />
            </>
          )}
        </div>
      </div>
      <Verbs view={view} cancel="Not now" />
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
    id: "a", name: "A · Alert", placement: "center", pick: true,
    seenIn: <>{a("https://developers.raycast.com/information/lifecycle/deeplinks", "Raycast deeplink confirm")}, {a("https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/extensions/browser/extensionUrlHandler.ts", "VS Code URI confirm")}, {a("https://developer.apple.com/design/human-interface-guidelines/alerts", "HIG Alerts")}</>,
    draw: (scene) => <AlertSheet view={viewOf(scene)} />,
  },
  {
    id: "b", name: "B · Invite card", placement: "center",
    seenIn: <>{a("https://discord.com/invite/discord-developers", "Discord invite")}, {a("https://developers.raycast.com/information/lifecycle/deeplinks", "Raycast (stacked verbs)")}</>,
    draw: (scene) => <InviteSheet view={viewOf(scene)} />,
  },
  {
    id: "c", name: "C · Subject header", placement: "center",
    seenIn: <>{a("https://primer.style/product/components/dialog/guidelines/", "Primer Dialog")}, {a("https://carbondesignsystem.com/components/modal/usage/", "Carbon Modal")}; the frame (title band, ×, Cancel + verb) as in {a("https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop", "GitHub Desktop’s Clone")}</>,
    draw: (scene) => <SubjectSheet view={viewOf(scene)} />,
  },
  {
    id: "d", name: "D · From → To", placement: "center",
    seenIn: <>{a("https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop", "GitHub Desktop · Clone from link")}, {a("https://source.chromium.org/chromium/chromium/src/+/main:chrome/app/generated_resources.grd", "Chrome “Open app?”")}</>,
    draw: (scene) => <RouteSheet view={viewOf(scene)} />,
  },
  {
    id: "e", name: "E · Window sheet", placement: "attached",
    seenIn: <>{a("https://developer.apple.com/design/human-interface-guidelines/sheets", "HIG Sheets (macOS)")} · no shipped product checked</>,
    draw: (scene) => <WindowSheet view={viewOf(scene)} />,
  },
  {
    id: "f", name: "F · Current, cleaned", placement: "center",
    seenIn: <>Mallard’s own found row (New project › From Mallard Cloud)</>,
    draw: (scene) => <CleanedSheet view={viewOf(scene)} />,
  },
  {
    id: "g", name: "G · Anchored card", placement: "anchored", change: "behaviour change",
    seenIn: <>{a("https://www.nngroup.com/articles/modal-nonmodal-dialog/", "NN/g non-modal")}, {a("https://primer.style/product/components/dialog/guidelines/", "Primer (deep links)")} · no shipped product checked</>,
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
  { rule: "Title names the subject or the task", from: "HIG Modality · Primer · Carbon · Fluent 2 · NN/g", marks: every("yes", { before: "no", f: "no" }) },
  { rule: "Narrow enough for one short message", from: "Primer ≤ 480 · Material ≤ 560", marks: { before: "680", a: "400", b: "380", c: "520", d: "520", e: "460", f: "560", g: "360" } },
  { rule: "One level of box: no frame inside the sheet", from: "Primer Dialog · HIG Modality", marks: every("yes", { before: "no", d: "part" }) },
  { rule: "What is here, at reading size, beside the subject", from: "NN/g confirmation · Primer ConfirmationDialog", marks: every("yes", { before: "no" }) },
  { rule: "Says where it came from", from: "Primer Dialog (deep links disorient) · Carbon label", marks: every("yes", { a: "landings only", e: "landings only" }) },
  { rule: "Exits", from: "HIG · Material · Fluent 2 · NN/g: one; Primer · Carbon · Atlassian: ×", marks: { before: "× + Cancel", a: "Cancel", b: "Cancel", c: "× + Cancel", d: "× + Cancel", e: "Cancel", f: "× + Cancel", g: "× + Not now" } },
  { rule: "Primary on the trailing edge (top when stacked)", from: "HIG · Material · Primer · Carbon · Atlassian", marks: every("yes") },
  { rule: "A stop reads as a stop, its way forward in place", from: "Primer degraded · Carbon inline notification · HIG Alerts", marks: every("yes", { before: "part", g: "no" }) },
  {
    rule: "Looking up keeps the layout (px the sheet grows when the answer lands)", from: "Primer loading",
    marks: { before: "+63–77", a: "0 · session +26–43", b: "+56", c: "0 · session +27", d: "+68", e: "0 · session +17", f: "0", g: "0 · session +16" },
  },
  { rule: "Where it sits", from: "HIG Sheets · NN/g modal cost · Carbon non-modal", marks: { before: "centre", a: "centre", b: "centre", c: "centre", d: "centre", e: "under the bar", f: "centre", g: "corner, no backdrop" } },
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
        What a <code>mallard://</code> link from the website opens. <b>Before</b> is the shipped sheet, mounted for real;
        A–G use the app’s own box, pill and type. Every frame gets the same link state, run through the real <code>linkVerdict</code>,
        so the words are the shipped words except where a direction rearranges them. The frames are crops of the 1100×720 window.
      </p>


      <div className="spec-cols">
        <div className="spec-panel">
          <strong>What the shipped sheet does, against the references</strong>
          <ol>
            <li><b>The title never changes</b> (“Open from Mallard Cloud”), while the button under it does: Open project, Set up project…, Pull session…, Add to Catalog…. HIG (modality), Primer, Carbon and Fluent 2 want a title that names the task or the subject; Carbon and Atlassian want it to match the button.</li>
            <li><b>680 px for one row and one line.</b> Primer’s large dialog is 480, Material’s widest basic dialog 560; Primer asks whether anything from 640 up should be a page.</li>
            <li><b>A box inside the box.</b> The pickers’ list frame, built to hold many rows, holds one. Primer asks for a single simple column; HIG warns against an app inside the app.</li>
            <li><b>The deciding fact is the smallest text.</b> “Not on this computer.” is an 11 px caption outside the frame; the 12 px name sits inside it. NN/g and Primer: a confirmation names the specifics it acts on.</li>
            <li><b>A stop looks like an empty list:</b> a muted glyph centred in a bordered box. Primer’s degraded-content pattern allows a muted centred message; the frame around it is what makes it read as “nothing here”.</li>
            <li><b>Two exits that do the same thing</b> (× and Cancel). NN/g: worth it only when they differ. HIG, Material and Windows draw no × on a short dialog; Primer, Carbon and Atlassian always do.</li>
            <li><b>Focus opens on ×</b> (measured in the live harness), so Return does nothing. Carbon puts it on the primary when there are no fields; the ARIA pattern on the most-used button.</li>
            <li><b>Looking up is one small line</b>, and the sheet then grows from 181 px to 244–258 px. Primer: the indicator stands where the content will land, without a jump.</li>
          </ol>
        </div>
        <div className="spec-panel">
          <strong>Products that answer a link from outside (seen first-hand)</strong>
          <ul>
            <li>{a("https://source.chromium.org/chromium/chromium/src/+/main:chrome/app/generated_resources.grd", "Chrome")} asks first: “Open <i>app</i>?”, “<i>site</i> wants to open this application”, a checkbox to always allow, Open + Cancel. <b>Mallard’s sheet is the second ask</b>, so it should tell what was found and what the verb will do, not ask “Open?” again.</li>
            <li>{a("https://developers.raycast.com/information/lifecycle/deeplinks", "Raycast")} (docs screenshot): alert, the command’s icon top-left, title “Request to run <i>command</i>”, a line saying it came from outside, three full-width stacked buttons, no ×.</li>
            <li>{a("https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/extensions/browser/extensionUrlHandler.ts", "VS Code")} (source): confirm titled “Allow ‘<i>extension</i>’ extension to open this URI?”, the URI as detail, “do not ask again” checkbox, Open.</li>
            <li>{a("https://docs.github.com/en/desktop/adding-and-cloning-repositories/cloning-a-repository-from-github-to-github-desktop", "GitHub Desktop")} (docs screenshot): the link opens the task’s own dialog, “Clone a Repository”, with the URL filled in and a local path to choose; × and Cancel, Clone on the right.</li>
            <li>{a("https://discord.com/invite/discord-developers", "Discord invite")}: a centred card, a small line “You’ve been invited to join”, the server’s icon, its name as the largest text, member counts, one full-width button.</li>
          </ul>
          <p className="spec-note">
            Could not see: Apple Shortcuts’ add sheet (guide page redirects), Postman’s Run-in-Postman import dialog (no screenshot in the docs),
            Raycast’s manual page (404; the developer docs were used). Not read in the standards: Windows guidance on ellipses, Atlassian’s and
            ContentDialog’s pixel widths, the Fluent 2 live demo (its button order comes from story code only), Apple’s image-only multi-step sheet examples.
          </p>
        </div>
      </div>

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
          Where the references disagree: <b>×</b> (Primer, Carbon, Atlassian always draw one; HIG, Material and Windows don’t on a short dialog; Fluent 2 only in place of Cancel);
          <b> button order</b> (Windows puts the action on the left; everyone else on the right, and the app already does); <b>title punctuation</b> (HIG ends a sentence title with a period, Fluent 2 never does).
          Every direction keeps the shipped verbs, whose ellipses already follow HIG: “…” when the verb opens more to fill in, none on Open project.
        </p>
      </div>

      <div className="spec-panel">
        <strong>Recommendation: A · Alert</strong>
        <p>
          A takes the compact short-ask form most systems share: a specific title, a line or two, two verbs (HIG alert, Material basic dialog,
          Primer ConfirmationDialog at 320–480 px, Carbon’s small modal). Raycast and VS Code answer an outside link with this form. The shipped words already
          fit it: subject, standing, what is here, one verb. A drops the bands, the inner frame and the duplicate ×. It puts the deciding line at reading size,
          makes a stop’s message the title in a tinted tile (Carbon’s “title as message”), and is 400 px wide.
        </p>
        <p>
          <b>Where A breaks with the references:</b> Primer, Carbon and Atlassian require a × (A keeps Cancel and Esc, as HIG, Material and Windows do).
          Primer, Carbon and ContentDialog put no icon in the head; HIG and Material allow one. A stop’s message as a title can run to two lines, which is HIG’s limit.
          Its stops don’t say “Mallard Cloud”; the tile and the words carry them.
        </p>
        <ul>
          <li><b>B</b> is the same content centred, with the verb full-width: friendlier, louder, and the furthest from the app’s other dialogs.</li>
          <li><b>C</b> is the pick if this sheet should stay in the app’s dialog family (bands and ×), as Primer, Carbon and Atlassian draw it.</li>
          <li><b>D</b> earns its space only on Set up / Add, where the far end does not exist yet; on Open project it is two boxes for one fact.</li>
          <li><b>E</b> is the macOS document sheet; it fits a Mac app, but the Windows build would have to draw the same thing.</li>
          <li><b>G</b> changes behaviour: no backdrop, the link no longer has to wait while a review or a dialog is open. HIG says never to show a warning in a popover, so the stops argue against it.</li>
        </ul>
        <p className="spec-note">
          Whatever is picked, two fixes ride along. Focus opens on the verb, not on × (Carbon, ARIA). The sheet opens titled with the link’s subject while the
          service is asked, so it barely moves when the answer lands (Primer loading).
        </p>
        <p className="spec-note">
          <b>The references that argue against any sheet on a link:</b> Primer warns that a dialog opened from a link disorients; Carbon wants dialogs
          opened by the person; Polaris never opens one by itself; HIG avoids alerts at launch, which is what a cold start from a link is.
          <b> For it:</b> the person pressed Open in Mallard and then the browser’s own “Open Mallard?”, so the sheet continues something they started,
          which NN/g counts as a fair modal; and every direction says where it came from. Only G avoids the objection, and it changes behaviour.
        </p>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Specimen />);
