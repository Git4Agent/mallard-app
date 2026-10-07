import type { ManagedCloudLink, ManagedCloudPublicationStatus, ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import { ManagedCloudPanel } from "../src/components/managed-cloud/ManagedCloudChip";
import { cloudHeadline, type CloudTone } from "../src/components/managed-cloud/managedCloudLink";
import { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import ThreadStatusIcon from "../src/components/sync-review/ThreadStatusIcon";

import "../src/App.css";
import "./cloud-versions-mockup.css";

/**
 * Mockup: versions of a Mallard Cloud repository, on the app's side
 * (2026-10-05). The owner picked model 1 and the word "Version": Push moves
 * the draft; Publish version freezes it as Version N; a public release names
 * a version. The website's half is mallardInternal's
 * web/versions-mockup.html, where he picked B + A.
 *
 * Here: where Publish version lives in the app (five places, each with the
 * products it is seen in), the Cloud chip and its panel state by state, and
 * the sheet Publish asks in. Today's frames are the shipped
 * `ManagedCloudPanel` and `cloudHeadline`; the bar's buttons, the panel, the
 * Push footer, the notice and the sheet are the app's own classes. What is
 * proposed is the `SCENES` table below (candidate words for
 * managedCloudLink.ts) and what sits under a `cv-` prefix.
 *
 * The mockups server, then /mockups/cloud-versions-mockup.html
 * (`?part=all|where|chip|sheet|refs&group=all|private|public&theme=light|dark&clean=1`).
 */

type Part = "all" | "where" | "chip" | "sheet" | "refs";
const PARTS: readonly Part[] = ["all", "where", "chip", "sheet", "refs"];
const params = new URLSearchParams(window.location.search);
const pick = <T extends string>(key: string, options: readonly T[], fallback: T): T => {
  const value = params.get(key);
  return options.includes(value as T) ? (value as T) : fallback;
};

/** `?group=private|public` draws half of the chip's states, for a screenshot that fits. */
const GROUP = pick<"all" | "private" | "public">("group", ["all", "private", "public"], "all");

/* ——— today's status, for the frames that show the shipped words ———
   The shipped status carries versions since the proposal below was built, so
   each "Today" frame now draws the shipped panel for its scene's state. */

const ADDRESS = "rikaqu0223-arch/mallardNonGit";
const baseLink: ManagedCloudLink = {
  repository_id: "repo_one", visibility: "private", can_pull: true, can_push: true,
  website_url: `https://mallard-ai.com/${ADDRESS}`, owner_login: "rikaqu0223-arch", name: "mallardNonGit",
  repository_revision: 7, can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner",
  release: { kind: "private" },
};
/** Where a pushed draft stands: the latest version, the pushes past it, the public version, the version in review. */
const standing = (latest: number | null, ahead: number | null, shown: number | null = null, review: number | null = null): ManagedCloudPublicationStatus => ({
  draft: "pushed", latest_version: latest, next_version: (latest ?? 0) + 1, draft_ahead: ahead, public_version: shown, review_version: review,
});
const today = (publication: ManagedCloudPublicationStatus, of: Partial<ManagedCloudLink> = {}): ManagedCloudStatus => ({
  repository_id: "repo_one", state: "ready", message: null, link: { ...baseLink, ...of }, publication,
});
const noop = () => {};
const chipActions = { onOpenUrl: noop, onPublishVersion: noop, onDiscardDraft: noop, onRetry: noop, onStorageSettings: noop };

/* ——— the proposal: one row of this table per state ——— */

interface Head { word: string; line: string; mark: IconName; tone: CloudTone }
interface Row { id: string; label: string; icon: IconName; copy?: boolean; danger?: boolean; disabled?: boolean; fresh?: boolean; title?: string }
interface Scene {
  id: string;
  name: string;
  note: string;
  today: ManagedCloudStatus;
  head: Head;
  facts: Array<{ label: string; value: string }>;
  rows: Row[];
  standing?: string;
}

const publish = (n: number, off = false): Row => ({
  id: "publish", label: `Publish version ${n}…`, icon: "git-commit", fresh: true, disabled: off,
  title: off ? `Nothing new since version ${n - 1}` : undefined,
});
const request: Row = { id: "request", label: "Request public release", icon: "check-circle", copy: true };
const viewRequest: Row = { id: "view-request", label: "View release request", icon: "external-link", copy: true };
const viewDraft: Row = { id: "draft", label: "View draft", icon: "external-link", copy: true };
const versions: Row = { id: "versions", label: "Versions", icon: "external-link", copy: true, fresh: true };
const viewPublic: Row = { id: "public", label: "View public version", icon: "external-link", copy: true };
const discard = (n: number): Row => ({ id: "discard", label: "Discard draft…", icon: "x", danger: true, title: `Discard draft… · back to version ${n}` });
const facts = (draft: string, version: string, shown: string, you = "Owner") => [
  { label: "Draft", value: draft }, { label: "Version", value: version }, { label: "Public", value: shown }, { label: "You", value: you },
];

const SCENES: Scene[] = [
  {
    id: "private-none", name: "Private, nothing published", note: "A repository as it is today.",
    today: today(standing(null, null)),
    head: { word: "Private", line: "Draft only. No version yet.", mark: "cloud-lock", tone: "plain" },
    facts: facts("Not published", "—", "Private"),
    rows: [publish(1), request, viewDraft],
  },
  {
    id: "private-ahead", name: "Private, the draft three pushes on", note: "The chip does not move: private is the fact that matters at a glance. The panel says where the draft stands.",
    today: today(standing(2, 3)),
    head: { word: "Private", line: "The draft is 3 pushes past version 2.", mark: "cloud-lock", tone: "plain" },
    facts: facts("3 pushes on", "2", "Private"),
    rows: [publish(3), request, viewDraft, versions],
  },
  {
    id: "private-equal", name: "Private, the draft is version 3", note: "Just after Publish. It is off until a push changes the draft.",
    today: today(standing(3, 0), { can_publish_version: false, can_discard_draft: false }),
    head: { word: "Private", line: "Version 3. Collaborators only.", mark: "cloud-lock", tone: "plain" },
    facts: facts("Same as 3", "3", "Private"),
    rows: [publish(4, true), request, viewDraft, versions],
  },
  {
    id: "review", name: "In review", note: "As today, naming the version. Pushing on changes the draft, not what is reviewed.",
    today: today(standing(3, 0, null, 3), { role: "member", can_request_release: false, can_publish_version: false, can_discard_draft: false, release: { kind: "waiting_owner", requested_by: "usr_1" } }),
    head: { word: "In review", line: "Version 3 is waiting for the Owner.", mark: "cloud-clock", tone: "info" },
    facts: facts("Same as 3", "3", "Private", "Member"),
    rows: [publish(4, true), viewRequest, viewDraft, versions],
  },
  {
    id: "public-equal", name: "Public, the draft is the public version", note: "Today's Published v3. “Published” now means frozen, so the chip says Public.",
    today: today(standing(3, 0, 3), { visibility: "public", can_request_release: false, can_publish_version: false, can_discard_draft: false, release: { kind: "public" } }),
    head: { word: "Public v3", line: "Version 3 is public. The draft is the same.", mark: "cloud-check", tone: "success" },
    facts: facts("Same as 3", "3", "3"),
    rows: [publish(4, true), viewDraft, versions, viewPublic],
  },
  {
    id: "public-draft", name: "Public, the draft one push on", note: "Today's Draft v8. The draft carries no number of its own any more.",
    today: today(standing(3, 1, 3), { visibility: "public", release: { kind: "snapshot_changed" } }),
    head: { word: "Draft", line: "1 push past version 3, which is public.", mark: "cloud-dot", tone: "plain" },
    facts: facts("1 push on", "3", "3"),
    rows: [publish(4), request, viewDraft, versions, viewPublic, discard(3)],
  },
  {
    id: "public-version", name: "Public, version 4 published and not public", note: "A state today cannot say: a frozen copy newer than what the public reads.",
    today: today(standing(4, 0, 3), { visibility: "public", can_publish_version: false, can_discard_draft: false, release: { kind: "snapshot_changed" } }),
    head: { word: "Version 4", line: "Not public. Public is version 3.", mark: "cloud-dot", tone: "plain" },
    facts: facts("Same as 4", "4", "3"),
    rows: [publish(5, true), request, viewDraft, versions, viewPublic],
  },
  {
    id: "contributor", name: "Public, signed in and not on the team", note: "Today's rule: pushes as a contributor, reads the draft. Publish and Discard are the team's.",
    today: today(standing(3, 1, 3), { visibility: "public", role: null, can_publish_version: false, can_discard_draft: false, release: { kind: "snapshot_changed" } }),
    head: { word: "Draft", line: "1 push past version 3, which is public.", mark: "cloud-dot", tone: "plain" },
    facts: facts("1 push on", "3", "3", "Contributor"),
    rows: [request, viewDraft, versions, viewPublic],
    standing: "Not a collaborator: anyone signed in can push here, and the Owner decides what's released.",
  },
];
const scene = (id: string) => SCENES.find((each) => each.id === id)!;

/* ——— the app's pieces ——— */

/** The chip's face, as `ManagedCloudChip` draws it. */
function Chip({ head }: { head: Head }) {
  return (
    <button type="button" className={`sync-path-attention sync-path-cloud-status ${head.tone}`} aria-label={`Mallard Cloud: ${head.word}. ${head.line}`}>
      <Icon name={head.mark} size={16} />
      <b>{head.word}</b>
      <Icon name="chevron-down" size={11} className="sync-path-chevron" />
    </button>
  );
}

/** The end of the path bar: the Cloud chip, Pull and Push. */
function Bar({ head, pushCount, extra }: { head: Head; pushCount?: number; extra?: ReactNode }) {
  return (
    <div className="sync-path-actions cv-bar">
      <Chip head={head} />
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
        <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span>{pushCount != null && <span className="sync-path-count">{pushCount}</span>}</button>
        {extra}
      </div>
    </div>
  );
}

/** The chip's panel in the proposal: `ManagedCloudPanel`'s markup, the table's words. */
function Panel({ of, without }: { of: Scene; without?: string }) {
  const rows = of.rows.filter((row) => row.id !== without);
  const plain = rows.filter((row) => !row.danger);
  const danger = rows.filter((row) => row.danger);
  const draw = (row: Row) => {
    const item = (
      <button key={row.copy ? undefined : row.id} type="button" className={`sync-path-menu-item${row.danger ? " danger" : ""}`} aria-disabled={row.disabled || undefined} title={row.title}>
        <Icon name={row.icon} size={14} />
        <span className="sync-path-menu-copy"><span>{row.label}</span></span>
        {row.fresh && <em className="cv-fresh">new</em>}
      </button>
    );
    if (!row.copy) return item;
    return (
      <div key={row.id} className="sync-cloud-panel-row">
        {item}
        <button type="button" className="sync-cloud-panel-copy" aria-label={`Copy the link of ${row.label}`}><Icon name="copy" size={13} /></button>
      </div>
    );
  };
  return (
    <div className="sync-path-menu cv-open">
      <div className="sync-cloud-panel">
        <div className={`sync-cloud-panel-head ${of.head.tone}`}>
          <Icon name={of.head.mark} size={24} />
          <span className="sync-path-menu-copy"><span>{of.head.word}</span><small>{ADDRESS}</small></span>
        </div>
        <p className="sync-cloud-panel-line">{of.head.line}</p>
        <dl className="sync-cloud-panel-facts">
          {of.facts.map((fact) => <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>)}
        </dl>
        {of.standing && <p className="sync-cloud-panel-line quiet">{of.standing}</p>}
        <div className="sync-path-menu-list sync-cloud-panel-verbs" role="group" aria-label="Mallard Cloud actions">
          <div className="sync-path-menu-separator" role="separator" />
          {plain.map(draw)}
          {danger.length > 0 && <div className="sync-path-menu-separator" role="separator" />}
          {danger.map(draw)}
        </div>
      </div>
    </div>
  );
}

/** The shipped panel, held open, for the same repository today. */
function TodayPanel({ status }: { status: ManagedCloudStatus }) {
  return (
    <div className="sync-path-menu cv-open">
      <ManagedCloudPanel status={status} storageName="Mallard Cloud" busy={false} refreshing={false} close={noop} {...chipActions} />
    </div>
  );
}

/** The sheet Publish asks in: the link sheet's frame, one optional field. */
function PublishSheet({ pending }: { pending: number }) {
  return (
    <dialog open className="sync-modal app-link-sheet cv-sheet" aria-label="Publish version 3">
      <header className="sync-modal-header">
        <div><h1>Publish version 3</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close without publishing"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body app-link-lines">
        <span className="app-link-mark" aria-hidden="true"><Icon name="storage-mallard" size={14} /></span>
        <span className="app-link-facts">
          <span className="app-link-fact"><Icon name="message" size={12} />8 sessions</span>
          <span className="app-link-fact"><Icon name="file" size={12} />15 files</span>
          <span className="app-link-fact"><Icon name="upload" size={12} />3 pushes since version 2</span>
        </span>
        {pending > 0 && (
          <>
            <span className="app-link-mark warning" aria-hidden="true"><ThreadStatusIcon status="local-outstanding" size={16} /></span>
            <span className="app-link-here">{pending} sessions here are not pushed, so not in it</span>
          </>
        )}
        <span className="app-link-mark" aria-hidden="true"><Icon name="edit" size={14} /></span>
        <label className="cv-name"><span className="sync-visually-hidden">Name, optional</span><input type="text" placeholder="Name (optional)" defaultValue={pending > 0 ? "" : "GIF and SVG pair"} spellCheck={false} /></label>
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn btn-primary">Publish version 3</button>
        </div>
      </footer>
    </dialog>
  );
}

/* ——— where Publish version lives ——— */

interface Place { id: string; name: string; line: string; note: string; seen: Array<[string, string]>; against?: string; pick?: boolean; draw: () => ReactNode }

const pushed: NoticeItem = {
  key: "pushed", severity: "success", title: "Pushed",
  message: "3 sessions to Mallard Cloud.",
  action: <button type="button" className="btn btn-ghost">Publish version 3…</button>,
};

const PLACES: Place[] = [
  {
    id: "a", name: "A · In the Cloud panel", line: "a row in the chip's panel", pick: true,
    note: "Where every Cloud verb already lives. The bar does not change, and the row is one press from any page of the project.",
    seen: [["Mallard today: Request public release, View draft, Discard draft", "#"], ["AWS Lambda, Publish new version", "https://docs.aws.amazon.com/lambda/latest/dg/configuration-versions.html"], ["Figma, Save to version history", "https://help.figma.com/hc/en-us/articles/360038006754-View-a-file-s-version-history"]],
    draw: () => <><Bar head={scene("private-ahead").head} /><Panel of={scene("private-ahead")} /></>,
  },
  {
    id: "b", name: "B · With the Push", line: "a tick in the Push review's footer",
    note: "One step for whoever pushes and means it as a version. It can only freeze what this Push sends, and a Push that fails half-way has to say what happened to the version.",
    seen: [["Google Tag Manager, Publish and Create Version", "https://support.google.com/tagmanager/answer/6107163?hl=en"], ["IntelliJ, Commit and Push", "https://www.jetbrains.com/help/idea/commit-and-push-changes.html"]],
    draw: () => (
      <footer className="sync-inline-action-footer sync-push-resource-footer sync-sync-review-footer cv-footer">
        <label className="cv-also"><input type="checkbox" defaultChecked readOnly />Publish as version 3</label>
        <button type="button" className="btn btn-primary sync-pull-apply-button"><ThreadStatusIcon status="local-outstanding" size={16} />Push <span className="sync-path-count">3</span></button>
      </footer>
    ),
  },
  {
    id: "c", name: "C · After the Push", line: "the Pushed notice offers it",
    note: "Asked at the moment the draft changed. The notice closes, so the offer is gone when it does; it needs A behind it.",
    seen: [["GitHub Desktop, Preview Pull Request after a push", "https://docs.github.com/en/desktop/working-with-your-remote-repository-on-github-or-github-enterprise/creating-an-issue-or-pull-request-from-github-desktop"]],
    draw: () => <><Bar head={scene("private-ahead").head} /><NoticeZone items={[pushed]} subject="notes about this workspace" timing="standing" label="Notes about this workspace" /></>,
  },
  {
    id: "d", name: "D · On the bar", line: "a third button beside Pull and Push",
    note: "Always in view.",
    seen: [["Kaggle, Save Version beside the editor", "https://www.kaggle.com/product-feedback/139884"]],
    against: "The bar has 7 px to spare at its 580 px floor (measured 2026-09-27), and Apple keeps a toolbar to frequent commands with one primary. Publish is occasional.",
    draw: () => <Bar head={scene("private-ahead").head} extra={<button type="button" className="sync-path-sync"><Icon name="git-commit" size={14} /><span className="sync-path-sync-label">Publish</span></button>} />,
  },
  {
    id: "e", name: "E · On the website only", line: "the panel links to Versions; Publish is there",
    note: "No new act in the app. The panel still says where the draft stands and opens the Versions tab.",
    seen: [["GitHub Desktop, Create Pull Request opens GitHub", "https://docs.github.com/en/desktop/working-with-your-remote-repository-on-github-or-github-enterprise/creating-an-issue-or-pull-request-from-github-desktop"], ["Mallard today: the release review, Members and an Admin's discard", "#"]],
    against: "Only the app knows which sessions here are not pushed. The website cannot say “2 sessions are not in it”.",
    draw: () => <><Bar head={scene("private-ahead").head} /><Panel of={scene("private-ahead")} without="publish" /></>,
  },
];

function Seen({ seen }: { seen: Array<[string, string]> }) {
  return (
    <p className="cv-seen">Seen in: {seen.map(([label, href], index) => (
      <span key={label}>{index > 0 && "; "}{href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}</span>
    ))}</p>
  );
}

function Where() {
  return (
    <section className="cv-part">
      <h2>Where Publish version lives in the app</h2>
      <p className="cv-lede">One state in all five: private, version 2 published, the draft three pushes on.</p>
      <div className="cv-places">
        {PLACES.map((place) => (
          <article key={place.id} className="cv-place">
            <h3>{place.name}{place.pick && <em>Recommended</em>}<span>{place.line}</span></h3>
            <p className="cv-note">{place.note}</p>
            <Seen seen={place.seen} />
            {place.against && <p className="cv-against"><Icon name="alert-triangle" size={13} />{place.against}</p>}
            <div className="cv-stage">{place.draw()}</div>
          </article>
        ))}
      </div>
    </section>
  );
}

/* ——— the chip and its panel, state by state ——— */

function Chips() {
  return (
    <section className="cv-part">
      <h2>The Cloud chip and its panel, state by state</h2>
      <p className="cv-lede">Direction A. Left of each pair: the shipped panel for the same repository today. Right: the proposal, new rows marked.</p>
      <div className="cv-states">
        {SCENES.filter((each) => GROUP === "all" || GROUP === (each.id.startsWith("public") || each.id === "contributor" ? "public" : "private")).map((each) => {
          const before = cloudHeadline(each.today);
          return (
            <article key={each.id} className="cv-state">
              <h3>{each.name}</h3>
              <p className="cv-note">{each.note}</p>
              <div className="cv-pair">
                <div>
                  <p className="cv-tag">Today</p>
                  <Bar head={before} />
                  <TodayPanel status={each.today} />
                </div>
                <div>
                  <p className="cv-tag is-new">Proposed</p>
                  <Bar head={each.head} />
                  <Panel of={each} />
                </div>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function Sheets() {
  return (
    <section className="cv-part">
      <h2>What Publish asks</h2>
      <p className="cv-lede">A sheet, not the system's alert: it takes a name. It says what the version holds, and what on this computer is left out of it.</p>
      <div className="cv-sheets">
        <article>
          <h3>Everything here is pushed</h3>
          <div className="cv-stage is-sheet"><PublishSheet pending={0} /></div>
        </article>
        <article>
          <h3>Two sessions here are waiting to be pushed</h3>
          <p className="cv-note">The bar's Push says 2. Publish freezes the draft in Mallard Cloud, so those two are not in version 3; the sheet says so and does not stop it.</p>
          <div className="cv-stage is-sheet"><Bar head={scene("private-ahead").head} pushCount={2} /><PublishSheet pending={2} /></div>
        </article>
      </div>
      <p className="cv-lede">Nothing else in the app changes. A session's status stays this computer against the draft; Pull brings the draft.</p>
    </section>
  );
}

const REFS: Array<{ source: string; href: string; asks: string; here: string }> = [
  { source: "Apple HIG, toolbars", href: "https://developer.apple.com/design/human-interface-guidelines/toolbars", asks: "A toolbar holds the commands people use most, with one primary action; the rest go to menus.", here: "Against D. Publish is occasional, so it goes to the chip's panel (A)." },
  { source: "Apple HIG, sheets and alerts", href: "https://developer.apple.com/design/human-interface-guidelines/sheets", asks: "A sheet for a scoped task that asks for something before returning; an alert for critical or destructive moments, and not for routine acts.", here: "Publish asks in a sheet with Cancel and one verb. The system alert stays for Discard draft." },
  { source: "AWS Lambda, versions", href: "https://docs.aws.amazon.com/lambda/latest/dg/configuration-versions.html", asks: "Publish new version from the function's own menu, with an optional description; none while nothing changed.", here: "A; the sheet's optional name; the row is off while the draft is the latest version." },
  { source: "Google Tag Manager, Submit", href: "https://support.google.com/tagmanager/answer/6107163?hl=en", asks: "One dialog offers Create Version alone, or Publish and Create Version, with a name and description.", here: "B is that combined step, at the Push. The website's Release tab uses the same idea for “publish, then release”." },
  { source: "IntelliJ, Commit and Push", href: "https://www.jetbrains.com/help/idea/commit-and-push-changes.html", asks: "A second act beside Commit that does both, then shows what will be pushed.", here: "B: a second act beside Push." },
  { source: "GitHub Desktop, pull requests", href: "https://docs.github.com/en/desktop/working-with-your-remote-repository-on-github-or-github-enterprise/creating-an-issue-or-pull-request-from-github-desktop", asks: "After a push the app offers Preview Pull Request; Create Pull Request goes straight to the website.", here: "C offers the next step after a Push. E hands the act to the website." },
  { source: "GitHub Desktop, tags", href: "https://docs.github.com/en/desktop/managing-commits/managing-tags-in-github-desktop", asks: "Create Tag… is in a commit's context menu, asks for a name in a dialog, and the tag is shown on the commit.", here: "A names a point of the history from a menu, in a small dialog. The app has no push list to hang it on, so the chip's panel is the menu." },
  { source: "Figma, version history", href: "https://help.figma.com/hc/en-us/articles/360038006754-View-a-file-s-version-history", asks: "Save to version history is a menu command; it asks for a title and a description.", here: "A, and the sheet's name." },
  { source: "Mallard, the path bar's budget", href: "#", asks: "Actions never shrink; 7 px spare at a 580 px bar with Pull 12 and Push 40 (measured 2026-09-27).", here: "Against D. The chip's word stays within its 17 characters: Private, Draft, Version 4, Public v3, In review." },
];

function Refs() {
  return (
    <section className="cv-part">
      <h2>References</h2>
      <p className="cv-lede">Each was read on 2026-10-05 from the product's or the system's own documentation. No desktop app was opened to check a layout first-hand.</p>
      <table className="cv-refs">
        <thead><tr><th>Reference</th><th>It asks or shows</th><th>In the specimen</th></tr></thead>
        <tbody>
          {REFS.map((ref) => (
            <tr key={ref.source}>
              <td>{ref.href === "#" ? ref.source : <a href={ref.href} target="_blank" rel="noreferrer">{ref.source}</a>}</td>
              <td>{ref.asks}</td>
              <td>{ref.here}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function Specimen() {
  const [theme, setTheme] = useState<"light" | "dark">(pick<"light" | "dark">("theme", ["light", "dark"], "light"));
  const [part, setPart] = useState<Part>(pick("part", PARTS, "all"));
  const clean = params.has("clean");
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    query.set("part", part);
    query.set("theme", theme);
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [part, theme]);
  const option = <T extends string>(value: T, current: T, set: (next: T) => void, label: string) => (
    <button key={value} type="button" className="cv-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const shows = (each: Part) => part === "all" || part === each;
  return (
    <main className="cv">
      {!clean && (
        <header className="cv-head">
          <h1>Versions in the app</h1>
          <p>Model 1 on the desktop: Push moves the draft, Publish version freezes it, a public release names a version. The website's half is B + A.</p>
          <div className="cv-controls">
            <span>Part</span>
            {option("all", part, setPart, "All")}
            {option("where", part, setPart, "Where Publish lives")}
            {option("chip", part, setPart, "Chip and panel")}
            {option("sheet", part, setPart, "The sheet")}
            {option("refs", part, setPart, "References")}
            <span>Theme</span>
            {option("light", theme, setTheme, "Light")}
            {option("dark", theme, setTheme, "Dark")}
          </div>
        </header>
      )}
      {shows("where") && <Where />}
      {shows("chip") && <Chips />}
      {shows("sheet") && <Sheets />}
      {shows("refs") && <Refs />}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
