import { useEffect, useRef, useState } from "react";
import Icon, { type IconName } from "./Icons";
import SyncPathBar, { type PathBarCrumb } from "./project-sync/SyncPathBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "./project-sync/ThreadStatusIcon";
import SyncReviewSelectionBar from "./sync-review/SyncReviewSelectionBar";

/**
 * Mockup only — not wired into the app.
 *
 * The Catalog scope page redrawn against the project workflow: one storage
 * chosen in the path bar, one flat list of resources each carrying the same
 * status badge a thread carries, and Push/Pull as page verbs with a review,
 * instead of a per-resource drill-down with a storage select on every row.
 *
 * Every row state below maps to a real `CatalogLinkState` the backend already
 * derives (`resource_planning.rs::derive_link_state`), so nothing here is a
 * badge with no data behind it.
 */

type Specimen = "list" | "push" | "pull" | "unlinked" | "empty";

const STORAGE = "Local storage 1";

/** The indicator's colour class, keyed by badge — the project's own pairing. */
const TONE: Record<ThreadStatusIconName, string> = {
  synced: "synced",
  "local-outstanding": "local",
  "cloud-outstanding": "storage",
  divergent: "diverged",
  pending: "pending",
  conflict: "conflict",
  unavailable: "unavailable",
  untracked: "untracked",
  checking: "checking",
  unknown: "unknown",
};

interface Row {
  id: string;
  name: string;
  /** `CatalogLinkState`, spelled as the backend spells it. */
  state: string;
  status: ThreadStatusIconName;
  /** The badge's sentence — what the project puts in the icon tooltip. */
  label: string;
  /** Where the resource lives, as one glyph: the profile's own home, the
   *  machine-wide shared root, or nowhere local yet. */
  where: "home" | "shared" | "storage";
  path: string;
  revision: string;
  stamp: string;
  /** The stamp's exact reading, for its tooltip. */
  stampDetail: string;
  /** Storages this resource is linked to besides the one in the path bar. */
  alsoLinkedTo?: string[];
  /** Push review: can this row be included, and is it on by default? */
  pushable?: boolean;
  pushDefault?: boolean;
  pushNote?: string;
  /** Pull review: the local action Pull would take. */
  pullAction?: string;
  pullAlternatives?: string[];
}

const ROWS: Row[] = [
  {
    id: "triage",
    name: "triage",
    state: "in_sync",
    status: "synced",
    label: `Up to date here and in ${STORAGE}.`,
    where: "home",
    path: "~/.codex/skills/triage",
    revision: "rev 7",
    stamp: "Aug 14",
    stampDetail: "Last synced Aug 14, 2026 · 6:02 PM",
    alsoLinkedTo: ["Vault"],
  },
  {
    id: "changelog-writer",
    name: "changelog-writer",
    state: "local_changed",
    status: "local-outstanding",
    label: `Newer on this computer. Push to update ${STORAGE}.`,
    where: "home",
    path: "~/.codex/skills/changelog-writer",
    revision: "rev 3",
    stamp: "2h ago",
    stampDetail: "Edited Aug 17, 2026 · 8:41 AM",
    pushable: true,
    pushDefault: true,
  },
  {
    id: "pdf-filler",
    name: "pdf-filler",
    state: "new",
    status: "local-outstanding",
    label: `Only on this computer. Push to save it in ${STORAGE}.`,
    where: "shared",
    path: "~/.agents/skills/pdf-filler",
    revision: "never pushed",
    stamp: "Aug 15",
    stampDetail: "Created Aug 15, 2026 · 11:20 AM",
    pushable: true,
    pushDefault: true,
  },
  {
    id: "release-notes",
    name: "release-notes",
    state: "storage_changed",
    status: "cloud-outstanding",
    label: `Newer in ${STORAGE}. Pull to update this computer.`,
    where: "home",
    path: "~/.codex/skills/release-notes",
    revision: "rev 9 stored",
    stamp: "Aug 16",
    stampDetail: "Stored Aug 16, 2026 · 9:14 AM",
    pullAction: "Update from storage",
    pullAlternatives: ["Keep local as release-notes-local and overwrite", "Overwrite local content"],
  },
  {
    id: "sql-review",
    name: "sql-review",
    state: "local_missing",
    status: "cloud-outstanding",
    label: `Only in ${STORAGE}. Pull to download it here.`,
    where: "storage",
    path: "would install to ~/.codex/skills/sql-review",
    revision: "rev 2 stored",
    stamp: "Aug 9",
    stampDetail: "Stored Aug 9, 2026 · 4:38 PM",
    pullAction: "Install stored content",
  },
  {
    id: "api-designer",
    name: "api-designer",
    state: "diverged",
    status: "divergent",
    label: `Changed here and in ${STORAGE}. Review before syncing.`,
    where: "shared",
    path: "~/.agents/skills/api-designer",
    revision: "rev 5",
    stamp: "1h ago",
    stampDetail: "Edited Aug 17, 2026 · 9:52 AM",
    pushable: true,
    pushDefault: false,
    pushNote: "Storage changed too — pull or acknowledge before this one pushes.",
    pullAction: "Keep local as api-designer-local and overwrite",
    pullAlternatives: ["Overwrite local content"],
  },
  {
    id: "deploy-checklist",
    name: "deploy-checklist",
    state: "unreviewed_remote",
    status: "pending",
    label: `Held until you Pull: ${STORAGE} has a version this computer has never reviewed.`,
    where: "home",
    path: "~/.codex/skills/deploy-checklist",
    revision: "rev 4 stored",
    stamp: "Aug 12",
    stampDetail: "Stored Aug 12, 2026 · 2:07 PM",
    pullAction: "Update from storage",
  },
  {
    id: "screenshot",
    name: "screenshot",
    state: "diverged",
    status: "conflict",
    label: "A different version of this directory is claimed by Vault. Preserve a copy, acknowledge overwrite, or skip.",
    where: "home",
    path: "~/.codex/skills/screenshot",
    revision: "claimed by Vault",
    stamp: "Aug 11",
    stampDetail: "Edited Aug 11, 2026 · 10:15 AM",
  },
  {
    id: "legacy-import",
    name: "legacy-import",
    state: "blocked",
    status: "unavailable",
    label: "Can't be synced: the skill directory is a symlink, so its contents can't be captured safely.",
    where: "shared",
    path: "~/.agents/skills/legacy-import",
    revision: "symlink",
    stamp: "Aug 3",
    stampDetail: "Edited Aug 3, 2026 · 7:44 PM",
  },
];

/* Name-ascending, the default every comparable list ships with (Finder, Drive,
   Dropbox, the VS Code extensions view). Ordering by sync state would put the
   answer in two places at once and move a row out from under the cursor the
   moment its state changed. */
const LISTED = [...ROWS].sort((left, right) => left.name.localeCompare(right.name));
const PUSH_ROWS = LISTED.filter((row) => row.pushable);
const PULL_ROWS = LISTED.filter((row) => row.pullAction);

/** Where the resource lives, said in one glyph. The label is the whole
 *  sentence, so nothing has to be printed beside it. */
const LOCATION: Record<Row["where"], { icon: IconName; label: (path: string) => string }> = {
  home: { icon: "folder", label: (path) => `In this Codex home · ${path}` },
  // User scope rather than config scope — the same distinction Git draws with
  // --global vs --local, and VS Code with User vs Workspace settings. Editing
  // it here changes it for every Codex config on the Mac.
  shared: { icon: "user", label: (path) => `User-wide · shared by every Codex config on this Mac · ${path}` },
  storage: { icon: "cloud", label: (path) => `Not on this Mac — ${path}` },
};

/** The verb a local action reduces to, for the row's trailing readout. The
 *  sentence stays in the choice list; the row gets the word — the same trade
 *  the project's Pull review makes with "Keep local" / "Keep both". */
function actionVerb(action: string): string {
  if (action.startsWith("Keep local as")) return "Keep copy";
  if (action.startsWith("Overwrite")) return "Overwrite";
  if (action.startsWith("Install")) return "Install";
  return "Update";
}

/** What the Pull review hangs off a row: the chosen action, and the way to
 *  change it when the plan offers more than one. */
interface RowAction {
  chosen: string;
  options: string[];
  expanded: boolean;
  destructive: boolean;
  onToggle: () => void;
  /** Picks an option and closes the menu. */
  onChoose: (option: string) => void;
}

/** One line: what state it is in, what it is called, where it lives, when it
 *  last moved. Everything else — revisions, hashes, other storage links — is a
 *  tooltip or the detail page behind the chevron. */
function ResourceCard({
  row,
  selectable,
  selected,
  onToggle,
  action,
}: {
  row: Row;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  action?: RowAction;
}) {
  const location = LOCATION[row.where];
  const badgeLabel = row.alsoLinkedTo
    ? `${row.label} Also linked to ${row.alsoLinkedTo.join(", ")}.`
    : row.label;
  return (
    <article
      className={`catalog-row${selectable ? " sync-sync-selectable-row" : ""}${selected ? " selected" : ""}`}
    >
      {selectable && (
        <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${row.name}`}>
          <input
            type="checkbox"
            checked={!!selected}
            aria-label={`${selected ? "Exclude" : "Include"} ${row.name}`}
            onChange={() => onToggle?.()}
          />
        </label>
      )}
      <span
        className={`sync-thread-sync-indicator ${TONE[row.status]}`}
        data-tooltip={badgeLabel}
        aria-label={badgeLabel}
        tabIndex={0}
      >
        <ThreadStatusIcon status={row.status} size={18} />
      </span>
      <button type="button" className="catalog-row-name" title={`${row.name} · open`}>
        {row.name}
      </button>
      {action && (action.options.length > 1 ? (
        <span className="catalog-row-action-shell">
          <button
            type="button"
            className={`catalog-row-action is-choice${action.destructive ? " destructive" : ""}`}
            aria-expanded={action.expanded}
            aria-haspopup="menu"
            title={`${action.chosen} — choose a different local action`}
            onClick={action.onToggle}
          >
            {actionVerb(action.chosen)}
            <Icon name={action.expanded ? "chevron-up" : "chevron-down"} size={10} />
          </button>
          {action.expanded && (
            /* The app's own popover, item markup and all: a checkable list with
               the current choice ticked, which is what every file client from
               Fork to GitHub Desktop puts behind "use mine / use theirs". */
            <div className="sync-path-menu end" role="menu" aria-label={`Local action for ${row.name}`}>
              <div className="sync-path-menu-group">
                {action.options.map((option) => (
                  <button
                    key={option}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={action.chosen === option}
                    className={`sync-path-menu-item${option.startsWith("Overwrite") ? " danger" : ""}`}
                    onClick={() => action.onChoose(option)}
                  >
                    <span className="sync-path-menu-copy">
                      <span>{actionVerb(option)}</span>
                      <small className="phrase">{option}</small>
                    </span>
                    {action.chosen === option && (
                      <Icon name="check" size={13} className="sync-path-menu-check" />
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}
        </span>
      ) : (
        <span className="catalog-row-action" title={action.chosen}>{actionVerb(action.chosen)}</span>
      ))}
      <span
        className="catalog-row-place"
        data-tooltip={location.label(row.path)}
        aria-label={location.label(row.path)}
        tabIndex={0}
      >
        <Icon name={location.icon} size={13} />
      </span>
      <span className="catalog-row-stamp" data-tooltip={row.stampDetail} aria-label={row.stampDetail} tabIndex={0}>
        {row.stamp}
      </span>
      <button
        type="button"
        className="btn btn-ghost catalog-row-open"
        title="Storage links, fork, unlink"
        aria-label={`Open ${row.name}`}
      >
        <Icon name="chevron-right" size={14} />
      </button>
    </article>
  );
}

function pathCrumbs(leaf: string): PathBarCrumb[] {
  return [
    {
      id: "storages",
      label: "Storages",
      icon: "drive",
      tone: "section",
      onActivate: () => {},
    },
    {
      id: "storage",
      label: STORAGE,
      icon: "cloud",
      tone: "root",
      menuLabel: "Switch storage",
      menuHeader: { label: STORAGE, detail: "~/Library/Mallard/local-1", detailIsPath: true, icon: "cloud" },
      menu: [
        {
          items: [
            { id: "settings", label: "Storage settings", icon: "settings", onSelect: () => {} },
            { id: "unlink", label: "Unlink from these skills", icon: "link-off", danger: true, onSelect: () => {} },
          ],
        },
        {
          label: "Switch to",
          items: [
            { id: "vault", label: "Vault", detail: "s3 · mallard-vault", onSelect: () => {} },
            { id: "team", label: "Team bucket", detail: "s3 · mallard-team", onSelect: () => {} },
          ],
        },
      ],
    },
    {
      id: "scope",
      label: leaf,
      icon: "catalog",
      menuLabel: "Catalog row actions",
      menuHeader: { label: leaf, detail: "~/.codex", detailIsPath: true, icon: "catalog" },
      menu: [
        {
          items: [
            { id: "rescan", label: "Rescan this Codex home", icon: "refresh", onSelect: () => {} },
            { id: "remove", label: "Remove this Catalog row", icon: "trash", danger: true, onSelect: () => {} },
          ],
        },
        {
          label: "Switch to",
          items: [
            { id: "skills-default", label: "Skills · conf2", onSelect: () => {} },
            { id: "pets-v1", label: "Pets v1 · conf2", onSelect: () => {} },
            { id: "plugins", label: "Plugins · Default Codex", onSelect: () => {} },
          ],
        },
      ],
    },
  ];
}

function FactStrip({ storage = STORAGE as string | null }: { storage?: string | null }) {
  return (
    <section className="sync-history-project-context" aria-label="Catalog row information">
      <span className="sync-history-context-item" data-tooltip="Provider home: ~/.codex" tabIndex={0}>
        <Icon name="folder" size={14} /><span>~/.codex</span>
      </span>
      <span className="sync-history-context-item" title="Profile: Default Codex (Codex)">
        <Icon name="openai" size={14} /><span className="sync-history-context-name">Default Codex</span>
      </span>
      {storage ? (
        <span className="sync-history-context-item sync-history-storage-item" title={`Storage: ${storage}`}>
          <Icon name="cloud" size={14} /><strong>{storage}</strong>
          <span className="recorded" data-tooltip="Last pull: Aug 16, 2026 · 9:14 AM" tabIndex={0}>
            <Icon name="download" size={13} />
          </span>
          <span className="recorded" data-tooltip="Last push: Aug 14, 2026 · 6:02 PM" tabIndex={0}>
            <Icon name="upload" size={13} />
          </span>
        </span>
      ) : (
        <span className="sync-history-context-item muted" title="No storage linked">
          <Icon name="cloud" size={14} /><span>No storage</span>
        </span>
      )}
    </section>
  );
}

function SectionHeading({ count }: { count: number }) {
  return (
    <div className="catalog-list-heading">
      <Icon name="file" size={14} />
      <h2>Skills</h2>
      <span className="catalog-list-count">{count}</span>
    </div>
  );
}

function PathActions({
  pull,
  push,
  attention,
  reviewing,
  disabled,
  disabledReason,
}: {
  pull: number;
  push: number;
  attention: number;
  reviewing?: "push" | "pull";
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <>
      {attention > 0 && (
        <span
          className="sync-path-attention warning"
          tabIndex={0}
          role="status"
          data-tooltip="1 skill changed on both sides, 1 skill waiting to sync, 1 skill Mallard could not match, 1 skill that can't be synced"
        >
          <Icon name="alert-triangle" size={13} /><b>{attention}</b>
        </span>
      )}
      <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
        <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
      </button>
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button
          type="button"
          className={`sync-path-sync${reviewing === "pull" ? " active" : ""}`}
          aria-expanded={reviewing === "pull"}
          disabled={disabled}
          title={disabledReason}
        >
          <Icon name="download" size={14} />
          <span className="sync-path-sync-label">Pull</span>
          {pull > 0 && reviewing !== "pull" && <span className="sync-path-count" aria-hidden="true">{pull}</span>}
        </button>
        <button
          type="button"
          className={`sync-path-sync${reviewing === "push" ? " active" : ""}`}
          aria-expanded={reviewing === "push"}
          disabled={disabled}
          title={disabledReason}
        >
          <Icon name="upload" size={14} />
          <span className="sync-path-sync-label">Push</span>
          {push > 0 && reviewing !== "push" && <span className="sync-path-count" aria-hidden="true">{push}</span>}
        </button>
      </div>
    </>
  );
}

/* ---------------------------------------------------------------- specimens */

function ListSpecimen() {
  return (
    <>
      <SyncPathBar
        crumbs={pathCrumbs("Skills · Default Codex")}
        actions={<PathActions pull={3} push={3} attention={4} />}
        ariaLabel="Skills · Default Codex path"
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Skills · Default Codex">
          <FactStrip />
          <section className="catalog-list-section" aria-label="Skills">
            <SectionHeading count={LISTED.length} />
            <div className="catalog-row-list">
              {LISTED.map((row) => <ResourceCard key={row.id} row={row} />)}
            </div>
          </section>
        </section>
      </main>
    </>
  );
}

function PushSpecimen() {
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(PUSH_ROWS.filter((row) => row.pushDefault).map((row) => row.id)),
  );
  const [acknowledged, setAcknowledged] = useState(false);
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const recommended = PUSH_ROWS.filter((row) => row.pushDefault).map((row) => row.id);
  return (
    <>
      <SyncPathBar
        crumbs={pathCrumbs("Skills · Default Codex")}
        actions={<PathActions pull={3} push={3} attention={4} reviewing="push" />}
        ariaLabel="Skills · Default Codex path"
      />
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="catalog-push-title">
          <header className="sync-inline-action-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="catalog-push-title">Push review</h2></div>
            <SyncReviewSelectionBar
              count={selected.size}
              total={PUSH_ROWS.length}
              recommendedTitle={`Use the recommended skill selection (${recommended.length})`}
              recommendedAriaLabel={`Use recommended selection (${recommended.length})`}
              recommendedDisabled={false}
              onRecommended={() => setSelected(new Set(recommended))}
              clearDisabled={selected.size === 0}
              onClear={() => setSelected(new Set())}
              close={{ ariaLabel: "Close push review", onClose: () => {} }}
            />
          </header>
          <div className="sync-inline-action-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              <div className="sync-review-banners">
                <div className="sync-callout sync-sync-review-pull-holds" role="status">
                  <ThreadStatusIcon status="divergent" size={16} />
                  <span>
                    <strong>3 skills held for Pull</strong>
                    Storage has newer versions. Selected skills still push.
                  </span>
                  <button type="button" className="btn btn-ghost">Open Pull review</button>
                </div>
              </div>
              <div className="catalog-row-list">
                {PUSH_ROWS.map((row) => (
                  <ResourceCard
                    key={row.id}
                    row={row}
                    selectable
                    selected={selected.has(row.id)}
                    onToggle={() => toggle(row.id)}
                  />
                ))}
              </div>
              {selected.has("api-designer") && (
                <label className="catalog-review-option warning">
                  <input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.target.checked)} />
                  <span>api-designer changed in storage too — I understand pushing replaces the stored version.</span>
                </label>
              )}
            </div>
          </div>
          <footer className="sync-inline-action-footer sync-sync-review-footer">
            <button
              type="button"
              className="btn btn-primary sync-pull-apply-button"
              disabled={selected.size === 0 || (selected.has("api-designer") && !acknowledged)}
            >
              <ThreadStatusIcon status="local-outstanding" size={16} />
              Push
            </button>
          </footer>
        </section>
      </main>
    </>
  );
}

function PullSpecimen() {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(["release-notes", "sql-review", "deploy-checklist"]));
  const [choice, setChoice] = useState<Record<string, string>>({});
  const [openChoices, setOpenChoices] = useState<string | null>(null);
  const [acknowledged, setAcknowledged] = useState<Set<string>>(new Set());
  const listRef = useRef<HTMLDivElement | null>(null);

  /* Outside-click and Escape dismiss the menu, the same contract the path
     bar's own menus keep. A popover that only closes by hitting its opener
     again is the one way this pattern goes wrong. */
  useEffect(() => {
    if (!openChoices) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && listRef.current?.contains(target)) return;
      setOpenChoices(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpenChoices(null);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [openChoices]);
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });
  const chosenFor = (row: Row) => choice[row.id] ?? row.pullAction!;
  /* `requires_destructive_approval` is a property of the action, not the row:
     replacing local content needs a tick, preserving a copy first does not. */
  const isDestructive = (action: string) => action.startsWith("Overwrite");
  const blocked = PULL_ROWS.some((row) => selected.has(row.id)
    && isDestructive(chosenFor(row))
    && !acknowledged.has(row.id));
  return (
    <>
      <SyncPathBar
        crumbs={pathCrumbs("Skills · Default Codex")}
        actions={<PathActions pull={3} push={3} attention={4} reviewing="pull" />}
        ariaLabel="Skills · Default Codex path"
      />
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="catalog-pull-title">
          <header className="sync-inline-action-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="catalog-pull-title">Pull review</h2></div>
            <SyncReviewSelectionBar
              count={selected.size}
              total={PULL_ROWS.length}
              recommendedTitle="Use the recommended local actions (3)"
              recommendedAriaLabel="Use recommended selection (3)"
              recommendedDisabled={false}
              onRecommended={() => setSelected(new Set(["release-notes", "sql-review", "deploy-checklist"]))}
              clearDisabled={selected.size === 0}
              onClear={() => setSelected(new Set())}
              close={{ ariaLabel: "Close pull review", onClose: () => {} }}
            />
          </header>
          <div className="sync-inline-action-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              <div className="sync-review-banners">
                <div className="sync-callout warning" role="status">
                  <Icon name="alert-triangle" size={16} />
                  <span>
                    <strong>api-designer is claimed by Vault</strong>
                    A different version of this directory is pinned there. Preserve a copy, acknowledge overwrite, or leave it out.
                  </span>
                </div>
              </div>
              <div className="catalog-row-list" ref={listRef}>
                {PULL_ROWS.map((row) => {
                  const options = [row.pullAction!, ...(row.pullAlternatives ?? [])];
                  const chosen = chosenFor(row);
                  const included = selected.has(row.id);
                  const expanded = openChoices === row.id;
                  const needsAck = included && isDestructive(chosen);
                  return (
                    <div key={row.id} className="catalog-pull-row">
                      <ResourceCard
                        row={row}
                        selectable
                        selected={included}
                        onToggle={() => toggle(row.id)}
                        action={included ? {
                          chosen,
                          options,
                          expanded,
                          destructive: isDestructive(chosen),
                          onToggle: () => setOpenChoices(expanded ? null : row.id),
                          onChoose: (option) => {
                            setChoice((current) => ({ ...current, [row.id]: option }));
                            setAcknowledged((current) => {
                              const next = new Set(current);
                              next.delete(row.id);
                              return next;
                            });
                            setOpenChoices(null);
                          },
                        } : undefined}
                      />
                      {needsAck && (
                        /* Sits under the row it belongs to, not in the footer:
                           the footer says why the button is off, this says
                           which row turned it off. */
                        <label className="catalog-row-ack">
                          <input
                            type="checkbox"
                            checked={acknowledged.has(row.id)}
                            onChange={(event) => setAcknowledged((current) => {
                              const next = new Set(current);
                              if (event.target.checked) next.add(row.id);
                              else next.delete(row.id);
                              return next;
                            })}
                          />
                          <span>{`I understand this replaces the current ${row.name} on this Mac.`}</span>
                        </label>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
          <footer className="sync-inline-action-footer sync-sync-review-footer">
            {blocked && (
              <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>
            )}
            <button
              type="button"
              className="btn btn-primary sync-pull-apply-button"
              disabled={selected.size === 0 || blocked}
            >
              <ThreadStatusIcon status="cloud-outstanding" size={16} />
              Pull
            </button>
          </footer>
        </section>
      </main>
    </>
  );
}

function UnlinkedSpecimen() {
  const rows = LISTED.slice(0, 4).map((row) => ({
    ...row,
    status: "untracked" as ThreadStatusIconName,
    state: "untracked",
    label: "Only on this Mac. Link a storage to compare and sync it.",
    revision: "not linked",
    alsoLinkedTo: undefined,
  }));
  return (
    <>
      <SyncPathBar
        crumbs={[
          { id: "storages", label: "Storages", icon: "drive", tone: "section", onActivate: () => {} },
          {
            id: "storage",
            label: "No storage",
            icon: "cloud",
            tone: "root",
            warning: true,
            menuLabel: "Choose storage",
            menuHeader: { label: "No storage linked", icon: "cloud" },
            menuBanner: "Link a storage to compare these skills and turn on Push and Pull.",
            menu: [{ label: "Link to", items: [
              { id: "local", label: STORAGE, onSelect: () => {} },
              { id: "vault", label: "Vault", detail: "s3 · mallard-vault", onSelect: () => {} },
            ] }],
          },
          ...pathCrumbs("Skills · Default Codex").slice(2),
        ]}
        actions={(
          <PathActions
            pull={0}
            push={0}
            attention={0}
            disabled
            disabledReason="Link a storage to these skills before pushing or pulling"
          />
        )}
        ariaLabel="Skills · Default Codex path"
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Skills · Default Codex">
          <FactStrip storage={null} />
          <section className="catalog-list-section" aria-label="Skills">
            <SectionHeading count={rows.length} />
            <div className="catalog-row-list">
              {rows.map((row) => <ResourceCard key={row.id} row={row} />)}
            </div>
          </section>
        </section>
      </main>
    </>
  );
}

function EmptySpecimen() {
  return (
    <>
      <SyncPathBar
        crumbs={pathCrumbs("Pets v1 · Default Codex")}
        actions={<PathActions pull={0} push={0} attention={0} />}
        ariaLabel="Pets v1 · Default Codex path"
      />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Pets v1 · Default Codex">
          <FactStrip />
          <div className="sync-history-state">
            No Codex pet v1 exists under ~/.codex, and {STORAGE} has none stored.
          </div>
        </section>
      </main>
    </>
  );
}

const SPECIMENS: Array<{ id: Specimen; label: string; note: string }> = [
  { id: "list", label: "List", note: "One storage, one list, every badge state the backend can derive." },
  { id: "push", label: "Push review", note: "The same list in selection mode — the project's own Push review." },
  { id: "pull", label: "Pull review", note: "Selection, plus the exact local action per skill behind the app's own popover." },
  { id: "unlinked", label: "No storage", note: "Nothing to compare against yet; every row is untracked." },
  { id: "empty", label: "Empty row", note: "A Catalog row with nothing on either side." },
];

function initialSpecimen(): Specimen {
  const hash = typeof window === "undefined" ? "" : window.location.hash.replace("#", "");
  return SPECIMENS.some((entry) => entry.id === hash) ? (hash as Specimen) : "list";
}

export default function CatalogRedesignMockup() {
  const [specimen, setSpecimenState] = useState<Specimen>(initialSpecimen);
  const setSpecimen = (next: Specimen) => {
    window.location.hash = next;
    setSpecimenState(next);
  };
  const [dark, setDark] = useState(false);
  const active = SPECIMENS.find((entry) => entry.id === specimen)!;

  return (
    <div className="catalog-mockup">
      <header className="catalog-mockup-bar">
        <strong>Catalog page redesign</strong>
        <nav>
          {SPECIMENS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={entry.id === specimen ? "active" : undefined}
              onClick={() => setSpecimen(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </nav>
        <button
          type="button"
          className="catalog-mockup-theme"
          onClick={() => {
            const next = !dark;
            setDark(next);
            document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
          }}
        >
          {dark ? "Light" : "Dark"}
        </button>
      </header>
      <p className="catalog-mockup-note">{active.note}</p>
      <div className="catalog-mockup-frame">
        <div className="app">
          <div className="sync-workspace">
            {specimen === "list" && <ListSpecimen />}
            {specimen === "push" && <PushSpecimen />}
            {specimen === "pull" && <PullSpecimen />}
            {specimen === "unlinked" && <UnlinkedSpecimen />}
            {specimen === "empty" && <EmptySpecimen />}
          </div>
        </div>
      </div>
    </div>
  );
}
