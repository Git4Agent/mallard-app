import type { ManagedCloudLink, ManagedCloudPublicationStatus, ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import { ManagedCloudPanel } from "../src/components/managed-cloud/ManagedCloudChip";
import {
  canPublishVersion, cloudContributor, cloudFacts, cloudHeadline, cloudVerbs, publishVersionHold, type CloudHeadline,
} from "../src/components/managed-cloud/managedCloudLink";
import { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import { PathBarMenu, type PathBarMenuGroup, type PathBarMenuItem } from "../src/components/sync-review/SyncPathBar";

import "../src/App.css";
import "./delete-draft-mockup.css";

/**
 * Mockup: deleting the draft of a Mallard Cloud repository, on the app's
 * side (2026-10-06, docs/PLAN_DELETE_DRAFT.md). Today the Cloud panel's
 * Discard draft… row is the Owner's, on a public repository, while the draft
 * is past the latest version; a private repository's draft is discarded on
 * the website, and when there is nothing to remove neither side says so.
 *
 * Here: where the verb lives and how it asks (five directions, each with the
 * products it is seen in), the panel state by state (the shipped panel beside
 * the proposal), the three questions, and the notice after. The bar, the
 * chip, the panel, the menus, the sheet and the notices are the app's own
 * classes; "Today" draws the shipped `ManagedCloudPanel`. What is proposed is
 * `rule()` below and what sits under a `dd-` prefix.
 *
 * The mockups server (port 1437), then /mockups/delete-draft-mockup.html
 * (`?part=all|where|states|ask|after|refs&word=discard|delete&theme=light|dark&clean=1`).
 */

type Part = "all" | "where" | "states" | "ask" | "after" | "refs";
type WordKey = "discard" | "delete";
const PARTS: readonly Part[] = ["all", "where", "states", "ask", "after", "refs"];
const params = new URLSearchParams(window.location.search);
const pick = <T extends string>(key: string, options: readonly T[], fallback: T): T => {
  const value = params.get(key);
  return options.includes(value as T) ? (value as T) : fallback;
};
const noop = () => {};

/** The one word, decision 2: in the row, the question, the button and the notice. */
interface Word { key: WordKey; row: string; title: string; question: (name: string) => string; ok: string; done: string; verb: string }
const WORDS: Record<WordKey, Word> = {
  discard: { key: "discard", row: "Discard draft…", title: "Discard draft?", question: (name) => `Discard the draft of “${name}”?`, ok: "Discard", done: "Draft discarded", verb: "Discard" },
  delete: { key: "delete", row: "Delete draft…", title: "Delete draft?", question: (name) => `Delete the draft of “${name}”?`, ok: "Delete", done: "Draft deleted", verb: "Delete" },
};

/* ——— the repository, state by state ——— */

const ADDRESS = "rika/portal";
const baseLink: ManagedCloudLink = {
  repository_id: "repo_one", visibility: "private", can_pull: true, can_push: true,
  website_url: `https://mallard-ai.com/${ADDRESS}`, owner_login: "rika", name: "portal",
  repository_revision: 7, can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner",
  release: { kind: "private" },
};
/** Where a pushed draft stands: the latest version, the pushes past it, the public version, the version in review. */
const standing = (latest: number | null, ahead: number | null, shown: number | null = null, review: number | null = null): ManagedCloudPublicationStatus => ({
  draft: "pushed", latest_version: latest, next_version: (latest ?? 0) + 1, draft_ahead: ahead, public_version: shown, review_version: review,
});
const NOT_PUSHED: ManagedCloudPublicationStatus = { draft: "none", latest_version: null, next_version: 1, draft_ahead: null, public_version: null, review_version: null };
const ready = (publication: ManagedCloudPublicationStatus, of: Partial<ManagedCloudLink> = {}): ManagedCloudStatus => ({
  repository_id: "repo_one", state: "ready", message: null, link: { ...baseLink, ...of }, publication,
});

interface Scene { id: string; name: string; note: string; status: ManagedCloudStatus }
const SCENES: Scene[] = [
  { id: "private-none", name: "Private, nothing published", note: "Every repository before its first version. Today the draft is discarded on the website only; the row would remove every push.", status: ready(standing(null, null)) },
  { id: "private-ahead", name: "Private, the draft 3 pushes past version 2", note: "Today: website only. Proposed: the row, back to version 2.", status: ready(standing(2, 3)) },
  { id: "private-equal", name: "Private, the draft is version 3", note: "Nothing to remove. Today neither side says so; proposed: the row stays, off, and its title says why, as Publish does.", status: ready(standing(3, 0), { can_publish_version: false, can_discard_draft: false }) },
  { id: "private-admin", name: "Private, as an Admin", note: "The service accepts an Admin; today the app offers the row to the Owner alone (decision 3).", status: ready(standing(2, 3), { role: "admin" }) },
  { id: "public-ahead", name: "Public, the draft 1 push past version 3", note: "The one state with the row today. It does not change.", status: ready(standing(3, 1, 3), { visibility: "public", release: { kind: "snapshot_changed" } }) },
  { id: "public-equal", name: "Public, the draft is version 3, which is public", note: "Nothing to remove: off, with the reason.", status: ready(standing(3, 0, 3), { visibility: "public", can_request_release: false, can_publish_version: false, can_discard_draft: false, release: { kind: "public" } }) },
  { id: "review", name: "In review, the draft asked for", note: "The row stays on. The question says the open request is withdrawn; a request for a version would stay.", status: ready(standing(2, 3), { can_request_release: false, release: { kind: "waiting_owner", requested_by: "usr_2" } }) },
  { id: "not-pushed", name: "Nothing pushed yet", note: "Off: nothing pushed yet.", status: ready(NOT_PUSHED, { can_request_release: false, can_publish_version: false, can_discard_draft: false, release: { kind: "not_pushed" } }) },
  { id: "member", name: "Private, as a Member", note: "No row: the service refuses a Member.", status: ready(standing(2, 3), { role: "member" }) },
  { id: "contributor", name: "Public, signed in and not on the team", note: "No row, as today.", status: ready(standing(3, 1, 3), { visibility: "public", role: null, can_publish_version: false, can_discard_draft: false, release: { kind: "snapshot_changed" } }) },
];
const scene = (id: string) => SCENES.find((each) => each.id === id)!;

/** The proposal's rule (R1 and R3 of the plan), as the service's flag would
 *  answer it: the Owner and Admins see the row; it is on while there is
 *  something to remove, and off with the reason otherwise. */
function rule(status: ManagedCloudStatus) {
  const link = status.link!;
  const publication = status.publication!;
  const offered = link.role === "owner" || link.role === "admin";
  const kept = publication.latest_version == null ? null : `version ${publication.latest_version}`;
  const hold = publication.draft === "none" ? "Nothing pushed yet"
    : kept !== null && publication.draft_ahead === 0 ? `Nothing to remove: the draft is ${kept}`
      : null;
  const waiting = link.release.kind === "waiting_owner" || link.release.kind === "waiting_super_admin";
  return { offered, on: offered && hold === null, hold, kept, requested: waiting && publication.review_version === null };
}

/** What the dialog asks, in the proposal's words. */
function question(word: Word, status: ManagedCloudStatus) {
  const { kept, requested } = rule(status);
  const removes = kept
    ? `Removes every push after ${kept} from Mallard Cloud. The draft returns to ${kept}. Every version stays.`
    : "Removes every push from Mallard Cloud. There is no version to return to: the repository stays, empty.";
  return {
    title: word.title,
    message: `${word.question(ADDRESS)}\n\n${removes}${requested ? " The open release request is withdrawn." : ""}\n\nNothing is deleted on this computer. This cannot be undone.`,
    ok: word.ok,
  };
}

/* ——— the app's pieces ——— */

/** The chip's face, as `ManagedCloudChip` draws it. */
function Chip({ head, open }: { head: CloudHeadline; open?: boolean }) {
  return (
    <button type="button" className={`sync-path-attention sync-path-cloud-status ${head.tone}`} aria-haspopup="dialog" aria-expanded={open} aria-label={`Mallard Cloud: ${head.word}. ${head.line}`}>
      <Icon name={head.mark} size={16} />
      <b>{head.word}</b>
      <Icon name="chevron-down" size={11} className="sync-path-chevron" />
    </button>
  );
}

/** The path bar: the crumbs, the chip, Pull and Push. `menu` holds the project's menu open (B). */
function Bar({ head, open, menu }: { head: CloudHeadline; open?: boolean; menu?: PathBarMenuGroup[] }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="portal path">
        <li className="sync-path-item">
          <button type="button" className="sync-path-crumb root">
            <Icon name="storage-mallard" size={14} />
            <span>Mallard Cloud rika</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
        </li>
        <li className="sync-path-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <button type="button" className="sync-path-crumb" aria-haspopup="menu" aria-expanded={!!menu}>
            <Icon name="git-folder" size={14} />
            <span>portal</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
          {menu && (
            <PathBarMenu
              id="dd-project-menu"
              groups={menu}
              header={{ label: "portal", detail: "~/code/portal", detailIsPath: true, icon: "git-folder" }}
              label="Project"
              align="start"
              open
              registerRef={noop}
              onClose={noop}
            />
          )}
        </li>
        <li className="sync-path-item leaf-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <span className="sync-path-crumb section leaf" aria-current="page"><span>Sessions</span></span>
        </li>
      </ol>
      <div className="sync-path-actions">
        <Chip head={head} open={open} />
        <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
          <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
        </button>
        <div className="sync-path-action-group" role="group" aria-label="Sync actions">
          <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
          <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
        </div>
        <span className="sync-path-divider" aria-hidden="true" />
        <div className="sync-path-overflow">
          <button type="button" className="sync-path-more" title="App actions" aria-label="App actions"><Icon name="more" size={16} /></button>
        </div>
      </div>
    </header>
  );
}

/** The bar's end alone, for the pairs. */
function Actions({ head, open }: { head: CloudHeadline; open?: boolean }) {
  return (
    <div className="sync-path-actions">
      <Chip head={head} open={open} />
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
        <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
      </div>
    </div>
  );
}

interface Row { id: string; label: string; icon: IconName; danger?: boolean; disabled?: boolean; title?: string; copy?: boolean; fresh?: boolean }

/** The panel's rows under the proposal: the shipped verbs as `cloudVerbs`
 *  gives them, then the discard row by `rule()`. A row today's panel lacks is marked. */
function proposedRows(status: ManagedCloudStatus, word: Word): Row[] {
  const verbs = cloudVerbs(status);
  const r = rule(status);
  const rows: Row[] = [];
  if (verbs.publishVersion !== null) {
    rows.push({ id: "publish", label: `Publish version ${verbs.publishVersion}…`, icon: "git-commit", disabled: !canPublishVersion(status), title: publishVersionHold(status) ?? undefined });
  }
  if (verbs.requestRelease) rows.push({ id: "request", label: "Request public release", icon: "check-circle", copy: true });
  else if (verbs.viewRelease) rows.push({ id: "view-release", label: "View release request", icon: "external-link", copy: true });
  if (verbs.viewDraft) rows.push({ id: "draft", label: "View draft", icon: "external-link", copy: true });
  if (verbs.versions) rows.push({ id: "versions", label: "Versions", icon: "external-link", copy: true });
  if (verbs.viewPublic !== null) rows.push({ id: "public", label: "View public version", icon: "external-link", copy: true });
  if (r.offered) {
    rows.push({
      id: "discard", label: word.row, icon: "x", danger: true, disabled: !r.on, fresh: !verbs.discardDraft,
      title: r.hold ?? (r.kept ? `${word.row} · back to ${r.kept}` : `${word.row} · every push`),
    });
  }
  return rows;
}

/** The chip's panel under the proposal: `ManagedCloudPanel`'s markup, `proposedRows`' words.
 *  `tail` stands in for the danger row (A3's question, C's link). */
function Panel({ status, word, tail }: { status: ManagedCloudStatus; word: Word; tail?: ReactNode }) {
  const head = cloudHeadline(status);
  const link = status.link!;
  const rows = proposedRows(status, word);
  const plain = rows.filter((row) => !row.danger);
  const danger = rows.filter((row) => row.danger);
  const draw = (row: Row) => {
    const item = (
      <button key={row.copy ? undefined : row.id} type="button" className={`sync-path-menu-item${row.danger ? " danger" : ""}`} aria-disabled={row.disabled || undefined} title={row.title}>
        <Icon name={row.icon} size={14} />
        <span className="sync-path-menu-copy"><span>{row.label}</span></span>
        {row.fresh && <em className="dd-fresh">new</em>}
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
  const end = tail ?? danger.map(draw);
  const hasEnd = tail !== undefined || danger.length > 0;
  return (
    <div className="sync-path-menu dd-open">
      <div className="sync-cloud-panel">
        <div className={`sync-cloud-panel-head ${head.tone}`}>
          <Icon name={head.mark} size={24} />
          <span className="sync-path-menu-copy"><span>{head.word}</span><small>{ADDRESS}</small></span>
        </div>
        <p className="sync-cloud-panel-line">{head.line}</p>
        <dl className="sync-cloud-panel-facts">
          {cloudFacts(link, status.publication).map((fact) => <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>)}
        </dl>
        {cloudContributor(link) && <p className="sync-cloud-panel-line quiet">Not a collaborator: anyone signed in can push here, and the Owner decides what's released.</p>}
        {(plain.length > 0 || hasEnd) && (
          <div className="sync-path-menu-list sync-cloud-panel-verbs" role="group" aria-label="Mallard Cloud actions">
            <div className="sync-path-menu-separator" role="separator" />
            {plain.map(draw)}
            {hasEnd && plain.length > 0 && <div className="sync-path-menu-separator" role="separator" />}
            {end}
          </div>
        )}
      </div>
    </div>
  );
}

/** The shipped panel, held open, for the same repository today. */
function TodayPanel({ status }: { status: ManagedCloudStatus }) {
  return (
    <div className="sync-path-menu dd-open">
      <ManagedCloudPanel status={status} storageName="Mallard Cloud" busy={false} refreshing={false} close={noop} onOpenUrl={noop} onPublishVersion={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
    </div>
  );
}

/** The system's confirmation dialog, drawn: the app does not style it. */
function Alert({ word, status }: { word: Word; status: ManagedCloudStatus }) {
  const asked = question(word, status);
  return (
    <div className="dd-alert" role="img" aria-label="The system's confirmation dialog">
      <div className="dd-alert-mark">!</div>
      <h3>{asked.title}</h3>
      <p>{asked.message}</p>
      <div className="dd-alert-buttons"><span>Cancel</span><span className="default">{asked.ok}</span></div>
    </div>
  );
}

/** A2: a sheet of the app's own, in the Publish sheet's frame, with the facts as rows. */
function Sheet({ word, status }: { word: Word; status: ManagedCloudStatus }) {
  const r = rule(status);
  return (
    <dialog open className="sync-modal app-link-sheet dd-sheet" aria-label={word.title}>
      <header className="sync-modal-header">
        <div><h1>{word.title}</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close without discarding"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body app-link-lines">
        <span className="app-link-mark warning" aria-hidden="true"><Icon name="upload" size={14} /></span>
        <span className="app-link-here">{r.kept ? `3 pushes after ${r.kept} leave Mallard Cloud` : "Every push leaves Mallard Cloud"}</span>
        <span className="app-link-mark" aria-hidden="true"><Icon name="check-circle" size={14} /></span>
        <span className="app-link-here">{r.kept ? `The draft returns to ${r.kept}. Every version stays.` : "The repository stays, empty."}</span>
        <span className="app-link-mark" aria-hidden="true"><Icon name="computer" size={14} /></span>
        <span className="app-link-here">Nothing is deleted on this computer. This cannot be undone.</span>
      </div>
      <footer className="sync-modal-footer">
        <div>
          <button type="button" className="btn">Cancel</button>
          <button type="button" className="btn danger">{word.row.replace("…", "")}</button>
        </div>
      </footer>
    </dialog>
  );
}

/** A3: the row turned into the question, inside the panel's list. */
function Ask({ word, status }: { word: Word; status: ManagedCloudStatus }) {
  const r = rule(status);
  return (
    <div className="dd-ask" role="group" aria-label={word.title}>
      <span>{word.verb} the draft? {r.kept ? `Back to ${r.kept}.` : "Nothing is kept."}</span>
      <button type="button" className="btn danger">{word.ok}</button>
      <button type="button" className="btn btn-ghost">Cancel</button>
    </div>
  );
}

/** C: the row is a link to the website, like Versions. */
function LinkRow({ word }: { word: Word }) {
  return (
    <div className="sync-cloud-panel-row">
      <button type="button" className="sync-path-menu-item danger" title={`${word.row} on the website`}>
        <Icon name="external-link" size={14} />
        <span className="sync-path-menu-copy"><span>{word.row.replace("…", "")} on the website</span></span>
      </button>
      <button type="button" className="sync-cloud-panel-copy" aria-label="Copy the versions link"><Icon name="copy" size={13} /></button>
    </div>
  );
}

/** B: the project's menu, with the verb as one more danger pill beside Delete all local sessions. */
function projectMenu(word: Word, status: ManagedCloudStatus): PathBarMenuGroup[] {
  const r = rule(status);
  const discard: PathBarMenuItem = { id: "discard-draft", label: word.row, icon: "x", danger: true, title: r.kept ? `${word.row} · back to ${r.kept}` : `${word.row} · every push`, onSelect: noop };
  return [
    {
      items: [
        { id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop },
        { id: "agent", label: "Agent profile…", detail: "Codex · Claude", icon: "terminal", onSelect: noop },
        { id: "rename", label: "Rename project…", icon: "edit", onSelect: noop },
        { id: "delete-project-sessions", label: "Delete all local sessions…", icon: "trash", danger: true, onSelect: noop },
        discard,
      ],
    },
    {
      label: "Switch to",
      items: [
        { id: "horror", label: "HorrorGame", detail: "~/code/HorrorGame", detailIsPath: true, icon: "git-folder", onSelect: noop },
        { id: "notes", label: "field-notes", detail: "~/notes", detailIsPath: true, icon: "folder", onSelect: noop },
      ],
    },
    { items: [{ id: "add-project", label: "Add project…", icon: "plus", onSelect: noop }] },
  ];
}

/* ——— where the verb lives and how it asks ——— */

const HIG_ALERTS = "https://developer.apple.com/design/human-interface-guidelines/alerts";
const HIG_CONTEXT = "https://developer.apple.com/design/human-interface-guidelines/context-menus";
const HIG_SHEETS = "https://developer.apple.com/design/human-interface-guidelines/action-sheets";
const PRIMER = "https://primer.style/product/components/confirmation-dialog/guidelines";
const CARBON = "https://carbondesignsystem.com/components/modal/usage/";
const NNG = "https://www.nngroup.com/articles/confirmation-dialog/";
const WCAG = "https://www.w3.org/WAI/WCAG21/Understanding/error-prevention-legal-financial-data.html";
const GOVUK = "https://design-system.service.gov.uk/components/button/#warning-buttons";
const GH_RELEASES = "https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository";
const GHD_REMOVE = "https://docs.github.com/en/desktop/adding-and-cloning-repositories/adding-a-repository-from-your-local-computer-to-github-desktop";
const VERCEL = "https://vercel.com/docs/deployments/managing-deployments#delete-a-deployment";
const GMAIL = "https://support.google.com/mail/answer/8395";
const FIGMA = "https://help.figma.com/hc/en-us/articles/360038511833";
const MEDIUM = "https://help.medium.com/hc/en-us/articles/213904978";

interface Place { id: string; name: string; line: string; note: string; seen: Array<[string, string]>; against?: string; pick?: boolean; draw: (word: Word) => ReactNode }

const AHEAD = scene("private-ahead").status;

const PLACES: Place[] = [
  {
    id: "a1", name: "A1 · The panel's row, the system's dialog", line: "today's place and today's question, on every repository", pick: true,
    note: "The row every Cloud verb already has, now for the Owner and Admins on a private repository too, and off with its reason when there is nothing to remove. The system's dialog asks, as the app's fourteen other destructive verbs do: a title, the consequence, Cancel and the verb.",
    seen: [["Apple HIG, alerts: Cancel and a specific verb, for an uncommon act that cannot be undone", HIG_ALERTS], ["Vercel: a deployment's … menu, then Delete", VERCEL], ["GitHub Desktop: Remove repository…, then a dialog", GHD_REMOVE], ["Mallard: Delete all local sessions…", "#"]],
    draw: (word) => <><Bar head={cloudHeadline(AHEAD)} open /><div className="dd-below"><Panel status={AHEAD} word={word} /></div><div className="dd-scrim"><Alert word={word} status={AHEAD} /></div></>,
  },
  {
    id: "a2", name: "A2 · The panel's row, a sheet of the app's own", line: "the Publish sheet's frame, with the facts as rows",
    note: "Says what goes, what stays and what this computer keeps, each on its own line with a mark. It would be the app's first confirmation of its own, and it needs a danger button the desktop has only inside notices.",
    seen: [["Mallard: the Publish version sheet", "#"], ["GitHub: Delete this release, in a dialog", GH_RELEASES], ["Carbon: the danger modal", CARBON], ["Primer: ConfirmationDialog, a question for a title, a specific verb", PRIMER]],
    against: "Apple keeps an alert for a destructive moment and a sheet for a task that asks for input; this asks for nothing.",
    draw: (word) => <><Bar head={cloudHeadline(AHEAD)} /><div className="dd-stage"><Sheet word={word} status={AHEAD} /></div></>,
  },
  {
    id: "a3", name: "A3 · The question inside the panel", line: "a second click in place, as the website's Settings asks",
    note: "No dialog: the row turns into the question with two buttons. The panel closes when focus leaves it, so a slip closes the question too; and two verbs in a 320 px panel sit tight.",
    seen: [["Mallard's website: Settings asks in place", "#"], ["GOV.UK: a warning button as the second step", GOVUK], ["Gmail: Discard draft, no question at all", GMAIL]],
    against: "Apple, Primer and Carbon keep a destructive confirmation modal: a thing to read, not a row to miss. NN/g: a question that is easy to click through is not one.",
    draw: (word) => <><Bar head={cloudHeadline(AHEAD)} open /><div className="dd-below"><Panel status={AHEAD} word={word} tail={<Ask word={word} status={AHEAD} />} /></div></>,
  },
  {
    id: "b", name: "B · A pill in the project's menu", line: "beside Delete all local sessions…",
    note: "Where the verb lived on 2026-09-26, before the Cloud verbs moved to the chip's panel. Destructive items last and in red, as a context menu wants.",
    seen: [["Apple HIG, context menus: destructive items last, red", HIG_CONTEXT], ["Mallard: Delete all local sessions… is here", "#"]],
    against: "Two homes for the Cloud's verbs. The project's menu does not read the draft's state, so its pill cannot say “back to version 2” or go off with a reason.",
    draw: (word) => <Bar head={cloudHeadline(AHEAD)} menu={projectMenu(word, AHEAD)} />,
  },
  {
    id: "c", name: "C · On the website only", line: "the row opens the website's Versions tab",
    note: "No new act in the app: the row is a link, like Versions. The website asks and does it.",
    seen: [["GitHub Desktop: deleting the remote is GitHub's", GHD_REMOVE], ["Mallard today: an Admin discards on the website", "#"]],
    against: "The owner's 2026-09-26 words asked for both sides, and the app already has the row for the public case.",
    draw: (word) => <><Bar head={cloudHeadline(AHEAD)} open /><div className="dd-below"><Panel status={AHEAD} word={word} tail={<LinkRow word={word} />} /></div></>,
  },
];

function Seen({ seen }: { seen: Array<[string, string]> }) {
  return (
    <p className="dd-seen">Seen in: {seen.map(([label, href], index) => (
      <span key={label}>{index > 0 && "; "}{href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}</span>
    ))}</p>
  );
}

function Where({ word }: { word: Word }) {
  return (
    <section className="dd-part">
      <h2>Where the verb lives, and how it asks</h2>
      <p className="dd-lede">One state in all five: private, version 2 published, the draft three pushes on, the Owner signed in. Today this repository has no row at all.</p>
      <div className="dd-places">
        {PLACES.map((place) => (
          <article key={place.id} className="dd-place">
            <h3>{place.name}{place.pick && <em>Recommended</em>}<span>{place.line}</span></h3>
            <p className="dd-note">{place.note}</p>
            <Seen seen={place.seen} />
            {place.against && <p className="dd-against"><Icon name="alert-triangle" size={13} />{place.against}</p>}
            <div className={`dd-frame${place.id === "b" ? " tall" : ""}`}>{place.draw(word)}</div>
          </article>
        ))}
      </div>
    </section>
  );
}

/* ——— the panel, state by state ——— */

function States({ word }: { word: Word }) {
  return (
    <section className="dd-part">
      <h2>The panel, state by state</h2>
      <p className="dd-lede">Direction A1. Left of each pair: the shipped panel for the same repository today. Right: the proposal; a row today's panel lacks is marked. The chip and its headline do not change.</p>
      <div className="dd-states">
        {SCENES.map((each) => {
          const head = cloudHeadline(each.status);
          return (
            <article key={each.id} className="dd-state">
              <h3>{each.name}</h3>
              <p className="dd-note">{each.note}</p>
              <div className="dd-pair">
                <div>
                  <p className="dd-tag">Today</p>
                  <Actions head={head} open />
                  <TodayPanel status={each.status} />
                </div>
                <div>
                  <p className="dd-tag is-new">Proposed</p>
                  <Actions head={head} open />
                  <Panel status={each.status} word={word} />
                </div>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

/* ——— the questions ——— */

function Asks({ word }: { word: Word }) {
  const cases: Array<[string, string, ManagedCloudStatus]> = [
    ["A version to return to", "The pushes after version 2 go; version 2 and the ones before it stay, and so does every version.", scene("private-ahead").status],
    ["No version", "Every push goes. The repository stays, empty; Push creates a new draft.", scene("private-none").status],
    ["A request for the draft is open", "The request named the pushes that go, so it is withdrawn. A request for a version would stay.", scene("review").status],
  ];
  return (
    <section className="dd-part">
      <h2>What it asks</h2>
      <p className="dd-lede">The system's dialog, titled with the verb, with Cancel and the verb: no OK, no Yes. The middle paragraph says what goes and what stays; the last what this computer keeps.</p>
      <div className="dd-asks">
        {cases.map(([name, note, status]) => (
          <article key={name}>
            <h3>{name}</h3>
            <p className="dd-note">{note}</p>
            <div className="dd-scrim"><Alert word={word} status={status} /></div>
          </article>
        ))}
      </div>
    </section>
  );
}

/* ——— afterwards ——— */

function After({ word }: { word: Word }) {
  const kept: NoticeItem = { key: "kept", severity: "warning", title: word.done, message: "— Mallard Cloud holds version 2. Pull, then push again." };
  const none: NoticeItem = { key: "none", severity: "warning", title: word.done, message: "— Push creates a new draft." };
  const equal = scene("private-equal").status;
  const empty = scene("not-pushed").status;
  return (
    <section className="dd-part">
      <h2>Afterwards</h2>
      <p className="dd-lede">Nothing new: the standing notice this computer already shows when its bases are stale, under the bar, until a Pull or a Push replaces them. Only the title's word follows decision 2.</p>
      <div className="dd-states">
        <article className="dd-state">
          <h3>Back at version 2</h3>
          <p className="dd-note">On this computer and on every other that had pushed or pulled the draft. The chip reads Private; the panel says the draft is version 2.</p>
          <div className="dd-frame">
            <Bar head={cloudHeadline(equal)} />
            <div className="dd-main">
              <NoticeZone items={[kept]} subject="notes about this project activity" timing="standing" label="Notes about this project activity" />
              <p className="dd-hint">Sessions · 14</p>
            </div>
          </div>
        </article>
        <article className="dd-state">
          <h3>Nothing kept</h3>
          <p className="dd-note">A repository that had no version. The chip reads Not pushed.</p>
          <div className="dd-frame">
            <Bar head={cloudHeadline(empty)} />
            <div className="dd-main">
              <NoticeZone items={[none]} subject="notes about this project activity" timing="standing" label="Notes about this project activity" />
              <p className="dd-hint">Sessions · 14</p>
            </div>
          </div>
        </article>
      </div>
    </section>
  );
}

/* ——— the references ——— */

const REFS: Array<{ source: string; href: string; asks: string; here: string }> = [
  { source: "Apple HIG, alerts", href: HIG_ALERTS, asks: "An alert for an uncommon destructive act that cannot be undone; a direct title; a specific verb, never OK; always Cancel, on the leading side; destructive styling only when the person did not set out to destroy.", here: "A1: the system's dialog, Cancel then the verb. The button is not red: the person chose the row." },
  { source: "Apple HIG, action sheets", href: HIG_SHEETS, asks: "Mail, cancelling a draft: Delete Draft in the destructive style at the top, Save Draft, then Cancel at the bottom.", here: "The word Delete draft (decision 2b) is Apple's for this act." },
  { source: "Apple HIG, context menus", href: HIG_CONTEXT, asks: "Destructive items last, marked destructive, in red.", here: "The danger row is last in the panel, after a separator, in danger ink. B puts it last in the project's menu." },
  { source: "Primer, ConfirmationDialog", href: PRIMER, asks: "For deleting data or an irreversible act; the title a question under five words; the body names what is deleted and that it is permanent; imperative, specific buttons; the danger variant; never “Are you sure?”.", here: "The title “Discard draft?”; the body names the pushes, the version kept and what this computer keeps; A2's danger button." },
  { source: "Carbon, modal", href: CARBON, asks: "The danger modal confirms an act that would lose data if done by accident; the primary button becomes a danger button.", here: "A2." },
  { source: "NN/g, confirmation dialogs", href: NNG, asks: "Only before serious, irreversible acts; say what will happen; buttons that name the outcome; undo beats a question.", here: "The question says what goes and what stays. There is no undo: the pushes are deleted, so the question stays." },
  { source: "WCAG 2.1, 3.3.4 Error prevention", href: WCAG, asks: "A deletion of user data is reversible, checked, or confirmed.", here: "Confirmed, on both sides." },
  { source: "GOV.UK, warning buttons", href: GOVUK, asks: "Only for serious destructive acts; a first step in another style, the warning button as the final confirmation; never red alone.", here: "A3 and the website's in-place ask are that two-step. Every row and button names the act in words." },
  { source: "GitHub, releases", href: GH_RELEASES, asks: "A draft release is deleted from the trash icon beside it; a dialog asks with “Delete this release”.", here: "The verb at the item (the website's B); a dialog that repeats the verb (A2)." },
  { source: "Vercel, deployments", href: VERCEL, asks: "Delete sits in the deployment's … menu, last.", here: "A1: a danger row, last in the panel." },
  { source: "Gmail", href: GMAIL, asks: "Discard draft is a trash icon at the compose window's foot; no question.", here: "The word Discard (decision 2a). Gmail can afford no question: a draft is one person's; a Mallard draft is the team's." },
  { source: "Figma, files", href: FIGMA, asks: "Move to trash from a file's menu, then a button that repeats the act; restore from Trash.", here: "Reversible there; not here, so the question stays." },
  { source: "Medium, stories", href: MEDIUM, asks: "Delete story from the story's row menu, then confirm.", here: "The verb at the item." },
  { source: "Mallard, destructive verbs", href: "#", asks: "Fourteen confirms, all in the system's dialog (`confirm` of plugin-dialog); none in a dialog of the app's own.", here: "A1 keeps that. A2 would be the first of its kind." },
];

function Refs() {
  return (
    <section className="dd-part">
      <h2>References</h2>
      <p className="dd-lede">Each was read on 2026-10-06 from the system's or the product's own documentation. The product screens need an account and were not opened; Material 3's dialog page is script-rendered and could not be read.</p>
      <table className="dd-refs">
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

/* ——— the specimen ——— */

function Specimen() {
  const [theme, setTheme] = useState<"light" | "dark">(pick<"light" | "dark">("theme", ["light", "dark"], "light"));
  const [part, setPart] = useState<Part>(pick("part", PARTS, "all"));
  const [wordKey, setWordKey] = useState<WordKey>(pick<WordKey>("word", ["discard", "delete"], "discard"));
  const clean = params.has("clean");
  const word = WORDS[wordKey];
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    query.set("part", part);
    query.set("word", wordKey);
    query.set("theme", theme);
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [part, wordKey, theme]);
  const option = <T extends string>(value: T, current: T, set: (next: T) => void, label: string) => (
    <button key={value} type="button" className="dd-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const shows = (each: Part) => part === "all" || part === each;
  return (
    <main className="dd">
      {!clean && (
        <header className="dd-head">
          <h1>Deleting the draft, in the app</h1>
          <p>Today the Cloud panel's Discard draft… row is the Owner's, on a public repository, while the draft is past the latest version. Proposed: one rule from the service, the row on every repository for the Owner and Admins, off with its reason when there is nothing to remove. The website's half is mallardInternal's delete-draft-mockup.</p>
          <div className="dd-controls">
            <span>Part</span>
            {option("all", part, setPart, "All")}
            {option("where", part, setPart, "Where and how")}
            {option("states", part, setPart, "State by state")}
            {option("ask", part, setPart, "The question")}
            {option("after", part, setPart, "Afterwards")}
            {option("refs", part, setPart, "References")}
            <span>Word</span>
            {option("discard", wordKey, setWordKey, "Discard draft")}
            {option("delete", wordKey, setWordKey, "Delete draft")}
            <span>Theme</span>
            {option("light", theme, setTheme, "Light")}
            {option("dark", theme, setTheme, "Dark")}
          </div>
        </header>
      )}
      {shows("where") && <Where word={word} />}
      {shows("states") && <States word={word} />}
      {shows("ask") && <Asks word={word} />}
      {shows("after") && <After word={word} />}
      {shows("refs") && <Refs />}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
