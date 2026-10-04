import CatalogConnectDialog, { type CatalogConnectReason } from "./CatalogConnectDialog";
import ManagedCloudChip from "../managed-cloud/ManagedCloudChip";
import { cloudStatusNotice, cloudSyncBlocked, discardCloudDraft } from "../managed-cloud/managedCloudLink";
import { predictRepositoryAddress } from "../managed-cloud/repositoryAddress";
import { useItemCloudStatus } from "./itemCloudStatus";
import SyncOutcomeNotice from "../sync-review/SyncOutcomeNotice";
import type { SyncOutcomeView } from "../sync-review/outcome";
import { cloudPullOutcome, cloudPushOutcome, pushOutcome, pullOutcome, type CatalogOutcomeAction } from "./outcomes";
import ReviewPreparing from "../sync-review/ReviewPreparing";
import { createReviewPreparing, PREPARING_CANCELLED } from "../sync-review/reviewPreparationLifecycle";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import Icon, { storageIconName, type IconName } from "../Icons";
import { formatFullDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import { PUSH_FINISHED, finishPushOffered } from "../../unfinishedPush";
import Notice, { Acknowledgement, NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import LoadingScaffold from "../LoadingScaffold";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";
import { focusMenuEdge, moveMenuFocus } from "../sync-review/menuKeyboard";
import ProviderMark from "../sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import NameSearchField from "../sync-review/NameSearchField";
import { nameMatchesSearch, replaceSearchSelection } from "../sync-review/nameSearch";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { storageCatalogApi } from "./api";
import {
  actionLabel,
  catalogNoun,
  catalogPresentation,
  catalogPullRecommendation,
  catalogScopeIcon,
  catalogScopeLabel,
  filterCatalogPullReviewItems,
  rowPullable,
  rowPushable,
  rowStamp,
  rowStatus,
  rowStatusLabel,
} from "./model";
import type {
  CatalogCapabilityDescriptor,
  CatalogPullItem,
  CatalogPullSource,
  CatalogPushDestination,
  CatalogResourceWarning,
  CatalogScopePullOutcome,
  CatalogScopePullPlan,
  CatalogScopePushOutcome,
  CatalogScopePushPlan,
  CatalogScopeResources,
  CatalogScopeRow,
  CatalogScopeView,
} from "./types";

/**
 * One Catalog row's page, drawn the way a project's is: the storage chosen in
 * the path bar, one flat list of resources each wearing the status badge a
 * thread wears, and Push and Pull as page verbs with one review each — the
 * scope links to the storage once, and every review covers the whole set.
 *
 * The storage in the bar is the page's outer scope. Every resource of this
 * kind is listed against it, whether it is on this Mac, only in that storage,
 * or both; the badge says which. Linking is not a step the user takes — Push
 * and Pull create the one scope-storage pairing they need, which is what
 * makes this page read like the project one.
 */

export interface CatalogStorageOption {
  id: string;
  provider: string;
  name: string;
  /** The provider's own label — what a transport failure is blamed on. */
  provider_label: string;
  /** The provider's presentation hint, resolved to a mark at draw time. */
  icon: string;
  /** The one-line "where is it" the shell already computes for every storage
   * menu — the first path-shaped fact, else the first fact. */
  detail: string;
  detailIsPath: boolean;
  /** Mallard Cloud only: the signed-in account, which names new
   *  repositories (`{login}/{name}`). */
  account_login?: string | null;
}

type Mode = { kind: "list" } | { kind: "push" } | { kind: "pull" };

/** Mirrors the shell's notice tones without importing them: storage-catalog
 *  may not depend on project-sync. */
type CatalogNoticeTone = "ok" | "note" | "attention";


/** Client-side mirror of the backend's component-name rule: ASCII alphanumeric
 * start, then letters, digits, dots, dashes, underscores; no trailing dot. The
 * backend stays the authority — this only catches the obvious while typing. */
function forkNameRuleBroken(value: string): boolean {
  if (value === "") return false;
  return !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value) || value.endsWith(".");
}

const FORK_RULE_MESSAGE =
  "Letters and digits, plus dots, dashes and underscores after the first character — this becomes the copy’s folder name.";

/** The suggested fork name, seeded the way the backend seeds generated copy
 * names: from the directory name, valid by construction. Display names are
 * free text — "Mallard Duck-copy" is not a folder name — so a row that never
 * captured a path falls back to its display name squeezed into one. */
function forkSeed(row: CatalogScopeRow): string {
  const directory = row.local_path?.replace(/\/+$/, "").split("/").pop();
  if (directory) return `${directory}-copy`;
  const cleaned = row.display_name
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[^a-z0-9]+/, "")
    .replace(/-+$/, "");
  return cleaned ? `${cleaned}-copy` : "";
}

/** The acknowledgement's lead, keyed by the warning's code — the backend
 * mints `executable_content` and `secret_shaped` today (inventory.rs) — with
 * the prefix its message is known to carry. An unknown code, or a message
 * that stopped matching its prefix, keeps the whole backend sentence: the
 * backend stays the authority on what a warning says. */
const WARNING_LEADS: Record<string, { lead: string; prefix: string }> = {
  executable_content: { lead: "Executable content", prefix: "contains executable content: " },
  secret_shaped: { lead: "Credential-shaped content", prefix: "contains credential-shaped content: " },
};

function warningParts(warning: CatalogResourceWarning): { lead: string | null; body: string } {
  const known = WARNING_LEADS[warning.code];
  if (known && warning.message.startsWith(known.prefix)) {
    return { lead: known.lead, body: warning.message.slice(known.prefix.length) };
  }
  return { lead: null, body: warning.message };
}

/** The verb a local action reduces to, for the row's trailing readout. The
 * whole sentence stays in the choice list; the row gets the word. */
function actionVerb(label: string): string {
  if (label.startsWith("Keep local as")) return "Keep copy";
  if (label.startsWith("Overwrite")) return "Overwrite";
  if (label.startsWith("Install")) return "Install";
  return "Update";
}

/** The row's operation label, in the files review's chip grammar — "Install
 * skill", "Update pet · keeps local copy" — so every pull review names its
 * operations in one voice. The full backend sentence stays in the tooltip
 * and the choice list. */
function actionChipLabel(label: string, kindNoun: string): string {
  if (label.startsWith("Keep local as")) return `Update ${kindNoun} · keeps local copy`;
  if (label.startsWith("Overwrite")) return `Overwrite ${kindNoun}`;
  if (label.startsWith("Install")) return `Install ${kindNoun}`;
  return `Update ${kindNoun}`;
}

/** The project page's Storage card, as its outcome and its open review use
 *  it: the card frame, then the one storage block. A linked storage draws no
 *  heading; its name is the bar's storage crumb. Mallard Cloud only: any
 *  other storage keeps the Catalog's own strip and page. */
function StorageCard({ name, review, children }: { name: string; review?: boolean; children: ReactNode }) {
  return (
    <section className="project-storage-card project-storage-card-non-git" aria-label={`${name} storage`}>
      <div className="project-profile-storage-list">
        <div className={`storage-link-block selected${review ? " sync-review-open" : ""}`}>{children}</div>
      </div>
    </section>
  );
}

/** A Push row's repository on Mallard Cloud: where it goes, or the one this
 *  Push creates. */
function pushRepository(destination?: CatalogPushDestination | null) {
  if (!destination) return undefined;
  return {
    address: destination.address,
    detail: destination.create ? `This Push creates ${destination.address}, private to you.` : `Pushes to ${destination.address}`,
    creates: destination.create,
  };
}

/** A Pull row's repository on Mallard Cloud: what it reads, and which
 *  version: the draft for a collaborator, the public one for anyone else. */
function pullRepository(source?: CatalogPullSource | null) {
  if (!source) return undefined;
  return { address: source.address, detail: `Pulls ${source.reads} v${source.revision} from ${source.address}` };
}

/** The indicator's colour class, keyed by badge — the project's own pairing. */
const TONE: Record<string, string> = {
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

/** What the Pull review hangs off a row: the chosen action, and the way to
 * change it when the plan offers more than one. */
interface RowAction {
  chosen: string;
  options: Array<{ id: string; label: string }>;
  expanded: boolean;
  destructive: boolean;
  onToggle: () => void;
  onChoose: (actionId: string) => void;
}

function badge(row: CatalogScopeRow, storageName: string | null) {
  const status = rowStatus(row);
  const label = rowStatusLabel(row, storageName);
  return (
    <span
      className={`sync-thread-sync-indicator ${TONE[status] ?? "unknown"}`}
      data-tooltip={label}
      aria-label={label}
      tabIndex={0}
    >
      <ThreadStatusIcon status={status} size={18} />
    </span>
  );
}

/** One readout, drawn exactly as a session's is: a 12px glyph, a mono figure,
 * and the sentence in a tooltip rather than on the row. */
function Metric({ icon, value, detail, muted }: { icon: IconName; value: string; detail: string; muted?: boolean }) {
  return (
    <span
      className={`sync-history-thread-metric${muted ? " sync-history-partial" : ""}`}
      data-tooltip={detail}
      aria-label={detail}
      tabIndex={0}
    >
      <Icon name={icon} size={12} /><b>{value}</b>
    </span>
  );
}

/** The ⓘ, carrying what a session card's debug menu carries: the row's paths
 * and digests, each a click to copy. Same wrapper, same popover. */
function InfoMenu({ row, storageRevision, kindNoun }: { row: CatalogScopeRow; storageRevision: number | null; kindNoun: string }) {
  const title = `${kindNoun.charAt(0).toUpperCase()}${kindNoun.slice(1)} info`;
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const closeMenu = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) {
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    }
  }, []);
  const placement = useDebugPopoverPlacement(rootRef, panelRef, open, closeMenu);
  /* A dialog that closes when focus leaves it is unreachable by keyboard
     unless opening it also moves focus in. Once the panel has been placed,
     the first Copy button takes focus; Escape hands it back to the trigger. */
  const focusedForOpen = useRef(false);
  useEffect(() => {
    if (!open) {
      focusedForOpen.current = false;
      return;
    }
    if (!placement || focusedForOpen.current) return;
    focusedForOpen.current = true;
    const frame = window.requestAnimationFrame(() => {
      panelRef.current?.querySelector<HTMLElement>("button")?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [open, placement]);

  useEffect(() => {
    if (!open) return;
    // The panel is portalled onto body, so it is not inside rootRef. Both have
    // to count as "inside" or the first click on a Copy button dismisses it.
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      closeMenu();
    };
    const onFocusIn = (event: FocusEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      closeMenu();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.preventDefault();
      event.stopPropagation();
      closeMenu(true);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("focusin", onFocusIn);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("focusin", onFocusIn);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [closeMenu, open]);

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(null), 2_000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  const values = [
    {
      label: "Local path",
      value: row.local_path
        ?? (row.local_present ? "In this provider home — not captured" : "Not on this Mac"),
    },
    {
      label: "Stored copy",
      value: !row.remote_present
        ? "Not stored yet"
        : [
          storageRevision == null ? null : `Version ${storageRevision}`,
          row.remote_updated_at == null ? null : `updated ${formatFullDateTime(row.remote_updated_at)}`,
        ].filter(Boolean).join(" · ") || "In this storage",
    },
    {
      label: "Content digest",
      value: row.content_sha256 ? `sha256:${row.content_sha256}` : "Nothing captured",
    },
  ];

  return (
    <div ref={rootRef} className="sync-history-debug-menu">
      <button
        ref={triggerRef}
        type="button"
        className="btn btn-ghost sync-history-session-detail-action"
        title={title}
        aria-label={`${title} for ${row.display_name}`}
        aria-haspopup="dialog"
        aria-controls={menuId}
        aria-expanded={open}
        onClick={() => {
          setCopied(null);
          if (open) closeMenu();
          else setOpen(true);
        }}
      >
        <Icon name="info" size={13} />
      </button>
      {/* On body, not in the row. Nothing in the list can bound it there:
          not the row's card, not the commit rail's stacking context, and
          not .sync-main's scrollport. */}
      <TopLayer>
        <div
          id={menuId}
          ref={panelRef}
          className="sync-history-debug-popover"
          style={placement
            ? {
              top: `${placement.top}px`,
              left: `${placement.left}px`,
              width: `${placement.width}px`,
              maxHeight: `${placement.maxHeight}px`,
            }
            /* Measured on the first layout pass; kept out of the paint
               until then so it never shows at the window origin. */
            : { visibility: "hidden" }}
          role="dialog"
          aria-label={title}
          hidden={!open}
        >
          <header>
            <strong>{title}</strong>
          </header>
          <div className="sync-history-debug-values">
            {/* No aria-label: it would replace the row's content as the
                accessible name, and the value is the one thing worth hearing.
                Named from content instead, so the path or digest is announced
                and stays navigable with a reading cursor. */}
            {values.map((entry) => (
              <button
                key={entry.label}
                type="button"
                className="sync-history-debug-copy"
                title={`Copy ${entry.label.toLowerCase()}`}
                onClick={() => {
                  void writeClipboardText(entry.value)
                    .then(() => setCopied(entry.label))
                    .catch(() => setCopied("__failed__"));
                }}
              >
                <span>{entry.label}</span>
                <code>{entry.value}</code>
                <small className={copied === entry.label ? "copied" : undefined}>
                  {copied === entry.label
                    ? "Copied"
                    : copied === "__failed__" ? "Couldn’t copy" : "Copy"}
                </small>
              </button>
            ))}
          </div>
          <span className="sync-visually-hidden" role="status" aria-live="polite">
            {copied === "__failed__"
              ? "Could not copy."
              : copied ? `${copied} copied.` : ""}
          </span>
        </div>

      </TopLayer>
    </div>
  );
}

/** The fork drawer preselects its suggested name. Validation and backend
 * errors share the hint line. */
function ForkDrawer({
  displayName,
  seed,
  maxLength,
  busy,
  error,
  onEdited,
  onSubmit,
  onClose,
}: {
  displayName: string;
  seed: string;
  maxLength: number;
  busy: boolean;
  /** Inline refusal from prepare or apply. */
  error: string | null;
  /** Typing resumes: a stale refusal for the previous name clears. */
  onEdited: () => void;
  onSubmit: (name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(seed);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.select();
  }, []);
  const trimmed = name.trim();
  const broken = forkNameRuleBroken(trimmed);
  return (
    <form
      className="catalog-fork-drawer"
      onSubmit={(event) => {
        event.preventDefault();
        if (busy || broken || trimmed === "") return;
        onSubmit(trimmed);
      }}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || busy) return;
        /* This layer consumed Escape; nothing above it may also close. */
        event.stopPropagation();
        onClose();
      }}
    >
      <span className="catalog-fork-lead">
        Fork <strong>“{displayName}”</strong> as
      </span>
      <input
        ref={inputRef}
        value={name}
        maxLength={maxLength}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        aria-label={`Name for the fork of ${displayName}`}
        aria-invalid={broken || undefined}
        readOnly={busy}
        onChange={(event) => {
          setName(event.target.value);
          onEdited();
        }}
      />
      <button type="submit" className="btn btn-primary" disabled={busy || broken || trimmed === ""}>
        <Icon name="git-fork" size={13} /> Fork
      </button>
      <button type="button" className="btn btn-ghost" disabled={busy} onClick={onClose}>
        Cancel
      </button>
      {broken || error ? (
        <span className="catalog-fork-note danger" role="alert">
          <Icon name="alert-triangle" size={13} /> {broken ? FORK_RULE_MESSAGE : error}
        </span>
      ) : (
        <span className="catalog-fork-note">
          An independent copy beside the original, under this name — its own row, only on this
          computer until you push it.
        </span>
      )}
    </form>
  );
}

/** One card per resource, wearing the activity card's grammar. Line one: the
 * badge, the name, the provider and terminal launches, Fork, and Reveal in
 * Finder. Line two: files and size, the rule, the ⓘ, and the full stamp.
 * Reviews put the checkbox where a session's review puts it — inside the
 * title line — and hang the Pull action pill after the name. */
function ResourceCard({
  row,
  storageName,
  storageRevision,
  provider,
  providerLabel,
  kindNoun,
  onOpenApp,
  onOpenTerminal,
  preview,
  onReveal,
  onFork,
  forkOpen,
  forkTitle,
  selectable,
  selected,
  onToggle,
  action,
  acks,
  footer,
  onOpen,
  repository,
}: {
  row: CatalogScopeRow;
  storageName: string | null;
  storageRevision: number | null;
  /** The profile's provider key, and the backend's label for it. */
  provider: string;
  providerLabel: string;
  /** The capability's noun, lowercased, for the operation chip: "skill",
   * "pet v1"… */
  kindNoun: string;
  /** The session card's launch pair, minus the session: the provider mark
   * opens the desktop app on this row's profile home, and Terminal opens the
   * provider CLI there — both at the default path. */
  onOpenApp: () => void;
  onOpenTerminal: () => void;
  /** The row's preview as a data URI. `null` draws the glyph tile — the
   * column never collapses and never shows a broken image — and `undefined`
   * (a capability without artwork) draws no tile at all. */
  preview?: string | null;
  onReveal: () => void;
  /** Given for a kind that can be forked at all. The row's own eligibility is
   *  decided here, so an ineligible row shows the control disabled rather than
   *  losing it — a kind that can never fork shows no button, since a control
   *  that is dead on every row of the page is noise rather than information. */
  /** Fork lives on the card: local rows of forkable kinds grow the button,
   * and it shows pressed while its drawer is open. */
  onFork?: () => void;
  forkOpen?: boolean;
  forkTitle?: string;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  action?: RowAction;
  /** Acknowledgement chips, part of the card the way the project files
   * review attaches "Reviewed this warning." to its own row. */
  acks?: ReactNode;
  /** The inline fork form, when this card's Fork is open. */
  footer?: ReactNode;
  /** On Mallard Cloud the name opens the item's page, the way a session's
   *  title opens its page on Activity. */
  onOpen?: () => void;
  /** A review's line-two readout on Mallard Cloud: the repository the row
   *  pushes to or pulls from, with the sentence in its tooltip. `creates`
   *  marks a repository the Push makes. */
  repository?: { address: string; detail: string; creates?: boolean };
}) {
  const stamp = rowStamp(row);
  /* The same test the fork action itself applies: there has to be a local copy
     to duplicate, and a resource id to name it by. */
  const canFork = row.local_present && !!row.resource_id;
  return (
    <article className={`sync-history-thread-card catalog-card${preview !== undefined ? " catalog-card-pet" : ""}`}>
      {preview !== undefined && (
        <span
          className="catalog-pet-preview"
          data-tooltip={row.description ?? undefined}
          tabIndex={row.description ? 0 : undefined}
          aria-hidden={row.description ? undefined : true}
        >
          {preview
            ? <img src={preview} alt={`${row.display_name} preview`} />
            : <Icon name="catalog-pet-v1" size={16} />}
        </span>
      )}
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${row.display_name}`}>
              <input
                type="checkbox"
                checked={!!selected}
                aria-label={`${selected ? "Exclude" : "Include"} ${row.display_name}`}
                onChange={() => onToggle?.()}
              />
            </label>
          )}
          {badge(row, storageName)}
          {onOpen ? (
            <button type="button" className="sync-history-session-title-button" title={`${row.display_name} · View ${kindNoun} details`} onClick={onOpen}>
              {row.display_name}
            </button>
          ) : <strong title={row.display_name}>{row.display_name}</strong>}
          {repository?.creates && (
            <span className="sync-history-branch-state current" title={repository.detail}>New repository</span>
          )}
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
                {actionChipLabel(action.chosen, kindNoun)}
                <Icon name={action.expanded ? "chevron-up" : "chevron-down"} size={10} />
              </button>
              {action.expanded && (
                /* The app's own popover, item markup and all: a checkable
                   list with the current choice ticked — and the keyboard
                   contract that `role="menu"` promises. */
                <div
                  className="sync-path-menu end"
                  role="menu"
                  aria-label={`Local action for ${row.display_name}`}
                  ref={(node) => focusMenuEdge(node, "first")}
                  onKeyDown={(event) => {
                    if (event.key === "Tab") {
                      action.onToggle();
                      return;
                    }
                    if (moveMenuFocus(event.currentTarget, event.key)) event.preventDefault();
                  }}
                >
                  <div className="sync-path-menu-group">
                    {action.options.map((option) => (
                      <button
                        key={option.id}
                        type="button"
                        role="menuitemcheckbox"
                        aria-checked={action.chosen === option.label}
                        className={`sync-path-menu-item${option.label.startsWith("Overwrite") ? " danger" : ""}`}
                        onClick={() => action.onChoose(option.id)}
                      >
                        <span className="sync-path-menu-copy">
                          <span>{actionVerb(option.label)}</span>
                          <small className="phrase">{option.label}</small>
                        </span>
                        {action.chosen === option.label && (
                          <Icon name="check" size={13} className="sync-path-menu-check" />
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </span>
          ) : (
            <span className="sync-history-branch-state history-only" title={action.chosen}>
              {actionVerb(action.chosen)}
            </span>
          ))}
        </div>
        {/* Line one's right-hand run, read across from a session's: the
            provider and Terminal launches on this row's profile home, Fork
            when the resource can be duplicated on this Mac, and Reveal in
            Finder. */}
        <div className="sync-history-thread-actions">
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            title={`Open in ${providerLabel}`}
            aria-label={`Open in ${providerLabel}`}
            onClick={onOpenApp}
          >
            <ProviderMark provider={provider} displayName={providerLabel} />
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            title="Open in Terminal"
            aria-label="Open in Terminal"
            onClick={onOpenTerminal}
          >
            <Icon name="terminal" size={14} />
          </button>
          {onFork && (
            <button
              type="button"
              className="btn btn-ghost sync-history-launch-action"
              /* Disabled, never dropped: a row with nothing local to copy kept
                 the same four controls as every other row, so the cluster does
                 not reflow and the eye keeps its column down the list. */
              disabled={!canFork}
              title={canFork ? forkTitle : `Not on this Mac, so there is nothing to fork`}
              aria-label={canFork ? forkTitle : `Cannot fork ${row.display_name}: not on this Mac`}
              aria-expanded={canFork ? !!forkOpen : undefined}
              onClick={onFork}
            >
              <Icon name="git-fork" size={14} />
            </button>
          )}
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!row.local_path}
            title={row.local_path ? `Reveal ${row.local_path} in Finder` : "Not on this Mac"}
            aria-label={`Reveal ${row.display_name} in Finder`}
            onClick={onReveal}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${row.display_name} details`}>
            <Metric
              icon="file"
              value={row.local_files == null ? "—" : String(row.local_files)}
              detail={row.local_files == null
                ? (row.local_present ? "On this Mac, but nothing could be captured" : "Not on this Mac, so nothing was counted")
                : `Files · ${row.local_files}`}
            />
            <Metric
              icon="drive"
              value={row.local_bytes == null ? "—" : formatBytes(row.local_bytes)}
              detail={row.local_bytes == null
                ? (row.local_present ? "On this Mac, but nothing could be captured" : "Not on this Mac, so nothing was measured")
                : `On disk · ${formatBytes(row.local_bytes)}`}
            />
            {repository && <Metric icon="cloud" value={repository.address} detail={repository.detail} />}
            {row.warnings.length > 0
              ? (
                <Metric
                  icon="alert-triangle"
                  value={String(row.warnings.length)}
                  muted
                  detail={row.warnings.map((warning) => warning.message).join("\n")}
                />
              )
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          {/* The rule parts the two readouts from the one button. */}
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <InfoMenu row={row} storageRevision={storageRevision} kindNoun={kindNoun} />
          </div>
          {/* The tooltip is the whole point of this stop — which time this is
              — so it has to be the accessible name too, or a screen reader
              hears a bare date and never learns whether it is the local
              modification or the storage update. */}
          <span
            className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
            data-tooltip={stamp?.detail}
            aria-label={stamp?.detail}
            tabIndex={0}
          >
            <time
              className={stamp == null ? "is-empty" : undefined}
              dateTime={stamp == null ? undefined : new Date(stamp.seconds * 1_000).toISOString()}
            >
              {stamp == null ? "—" : formatFullDateTime(stamp.seconds)}
            </time>
          </span>
        </div>
      </div>
      {acks}
      {footer}
    </article>
  );
}

export default function StorageCatalogPage({
  scope,
  scopes,
  storages,
  capabilities,
  appActions,
  onOpenScope,
  onOpenStorageDetails,
  onAddStorage,
  onAddScope,
  onRemoveScope,
  onProfileRenamed,
  openStorageMenuRequestId,
  initialStorageId,
  initialResourceId,
  initialPull = false,
  onScopeGone,
  notify,
}: {
  scope: CatalogScopeView;
  /** The other Catalog rows, for this row's Switch-to menu. */
  scopes: CatalogScopeView[];
  storages: CatalogStorageOption[];
  /** Every registered capability, for the words a chooser says about
   *  another kind's repository. */
  capabilities: readonly CatalogCapabilityDescriptor[];
  /** Theme and app-wide commands: this bar is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  onOpenStorageDetails: (storageId: string) => void;
  onAddStorage: () => void;
  onAddScope: () => void;
  onRemoveScope: () => void;
  /** Reload the scope list after a profile rename so shell labels update. */
  onProfileRenamed: () => void;
  /** Opens the storage crumb menu for recovery actions initiated by the shell. */
  openStorageMenuRequestId?: number | null;
  initialStorageId?: string;
  /** Open on one item's page (Mallard Cloud), as Add lands. */
  initialResourceId?: string;
  /** A link named that item: its page opens with its Pull review, as its
   *  Pull button opens it. Pull there is still the press that writes. */
  initialPull?: boolean;
  /** The row disappeared underneath the page (deleted elsewhere). */
  onScopeGone: () => void;
  /** Notices for profile and storage changes go to the shell. Sync outcomes
   * remain on this page until dismissed or another review is opened. */
  notify: (message: string, tone?: CatalogNoticeTone) => void;
}) {
  const label = catalogScopeLabel(scope);
  const glyph = catalogPresentation(scope.capability);
  const plural = scope.label.toLowerCase();
  const singular = scope.capability?.singular_label ?? scope.label;
  const noun = useCallback(
    (count: number) => catalogNoun(scope.label, singular, count),
    [scope.label, singular],
  );
  const artwork = glyph.artwork;
  const forkable = scope.capability?.capture === "directory_tree";
  const unavailable = !scope.available;
  const unavailableReason = scope.unavailable_reason
    ?? "This row's coordinate is not supported by this build.";
  const [page, setPage] = useState<CatalogScopeResources | null>(null);
  const [chosenStorageId, setChosenStorageId] = useState<string | null>(initialStorageId ?? null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [searchQuery, setSearchQuery] = useState("");
  const searchActive = searchQuery.trim().length > 0;
  useEffect(() => { setSearchQuery(""); }, [scope.scope.scope_id, chosenStorageId, mode.kind]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<SyncOutcomeView<CatalogOutcomeAction> | null>(null);
  /* On Mallard Cloud each item is its own repository, and has its own page,
     as a project has: the item it shows, null on the list. A review opened
     there holds that item alone. */
  const [itemId, setItemId] = useState<string | null>(initialResourceId ?? null);
  const [connectReason, setConnectReason] = useState<CatalogConnectReason | null>(null);
  const [preparing, setPreparing] = useState<"push" | "pull" | null>(null);
  const preparingKind = useRef<"push" | "pull">("push");
  const [reviewPreparing] = useState(() => createReviewPreparing({
    shown: () => setPreparing(preparingKind.current), writing: () => {}, finished: () => setPreparing(null),
  }));
  const operationEpoch = useRef(0);
  const applyInFlight = useRef(false);
  const prepareInFlight = useRef(false);
  useEffect(() => () => { operationEpoch.current += 1; reviewPreparing.cancel(); }, [scope.scope.scope_id, chosenStorageId, reviewPreparing]);
  /* A running Push or Pull: the sentence a storage's notice zone shows, and
     the counts Mallard Cloud's bar reads out, as a project's does. */
  const [progress, setProgress] = useState<{ text: string; done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  /* Per-item failures are a list. Joining them with newlines into the single
     error string put them in a flex callout that collapses whitespace, so five
     failures rendered as one run-on sentence. */
  const [failures, setFailures] = useState<string[]>([]);

  /* Renaming the profile — setup derived its name from the folder, and this
     is the one place a name is chosen. It opens from the scope crumb's menu
     as a row under the trail, the shape the project page uses, and relabels
     every Catalog row on this home at once. */
  const [renamingProfile, setRenamingProfile] = useState(false);
  const [profileNameDraft, setProfileNameDraft] = useState("");
  const [renameError, setRenameError] = useState<string | null>(null);

  /* The reviews hold the one plan the backend pinned; selections and
     acknowledgements are keyed by resource id, the plan's own key. */
  const [pushPlan, setPushPlan] = useState<CatalogScopePushPlan | null>(null);
  const [pushSelected, setPushSelected] = useState<Set<string>>(new Set());
  const [pushAcks, setPushAcks] = useState<Set<string>>(new Set());

  const [pullPlan, setPullPlan] = useState<CatalogScopePullPlan | null>(null);
  const [pullSelected, setPullSelected] = useState<Set<string>>(new Set());
  const [pullChoice, setPullChoice] = useState<Record<string, string>>({});
  const [pullAcks, setPullAcks] = useState<Set<string>>(new Set());
  const [openChoices, setOpenChoices] = useState<string | null>(null);
  const pullListRef = useRef<HTMLDivElement | null>(null);

  /* Previews for stored-only pet rows, keyed by row id. Each row asks once
   * per page (the ref) so an offline storage is not hammered on every render;
   * the backend caches the derived picture by digest across everything. */
  const [remotePreviews, setRemotePreviews] = useState<Record<string, string>>({});
  const previewAttempts = useRef<Set<string>>(new Set());

  /* Fork lives on the card: the id of the resource whose drawer is open, and
     the refusal that drawer is showing. The name draft belongs to the drawer
     itself — it reseeds from the row each time one opens. */
  const [forkFor, setForkFor] = useState<string | null>(null);
  const [forkError, setForkError] = useState<string | null>(null);
  const epochRef = useRef(0);

  /* `onScopeGone` arrives as a fresh arrow on every parent render. Reading it
     through a ref keeps the loader's identity tied to what it actually reads —
     the scope and the chosen storage — so an unrelated re-render of the shell
     (a splitter drag, a log keystroke, a sync event) no longer re-runs it.
     Every real refresh is already explicit: each mutation below awaits load(),
     and the path bar has its own Refresh. */
  /* The same translation the project page applies: a transport failure names
     what to check instead of quoting the AWS SDK. `storages` carries each
     storage's provider label, which is all the translation needs. */
  const storageFailureMessage = (reason: unknown): string => storageAccessErrorMessage(
    storages.find((candidate) => candidate.id === (chosenStorageId ?? storageId)) ?? null,
    reason,
  );

  const onScopeGoneRef = useRef(onScopeGone);
  onScopeGoneRef.current = onScopeGone;

  const load = useCallback(async () => {
    const epoch = ++epochRef.current;
    if (!scope.available) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    setFailures([]);
    try {
      const next = await storageCatalogApi.listScopeResources(scope.scope.scope_id, chosenStorageId);
      if (epoch !== epochRef.current) return;
      setPage(next);
    } catch (reason) {
      if (epoch !== epochRef.current) return;
      if (errorMessage(reason).startsWith("unknown catalog scope")) {
        onScopeGoneRef.current();
        return;
      }
      setError(storageFailureMessage(reason));
    } finally {
      if (epoch === epochRef.current) setLoading(false);
    }
  }, [chosenStorageId, scope.available, scope.scope.scope_id]);

  useEffect(() => {
    void load();
    return () => {
      epochRef.current += 1;
    };
  }, [load]);

  useEffect(() => {
    setMode({ kind: "list" });
    setChosenStorageId(initialStorageId ?? null);
    setItemId(initialResourceId ?? null);
    setConnectReason(null);
    setForkFor(null);
  }, [scope.scope.scope_id]);

  useEffect(() => {
    previewAttempts.current = new Set();
    setRemotePreviews({});
  }, [scope.scope.scope_id, chosenStorageId]);

  const rows = useMemo(() => page?.rows ?? [], [page]);
  const storageId = chosenStorageId ?? page?.storage_id ?? null;
  const currentStorage = storages.find((storage) => storage.id === storageId);
  const storageName = currentStorage?.name ?? page?.storage_name ?? null;
  const managedCloud = currentStorage?.provider === "managed_cloud";
  const pageLink = page?.link ?? null;
  const storageRevision = managedCloud ? null : page?.storage_revision ?? null;
  const rowByResource = useMemo(() => {
    const map = new Map<string, CatalogScopeRow>();
    for (const row of rows) {
      if (row.resource_id) map.set(row.resource_id, row);
    }
    return map;
  }, [rows]);
  /* An item's page, on Mallard Cloud only: its row, the repository the page
     knows for it (saved, or learned from the account's listing), and the
     account that names a new one. */
  const itemRow = managedCloud && itemId ? rowByResource.get(itemId) ?? null : null;
  const itemRepository = itemRow && itemId ? page?.cloud_repositories[itemId] ?? null : null;
  const itemSaved = !!itemId && !!page?.cloud_connections[itemId];
  const accountLogin = currentStorage?.account_login ?? "";
  /* The page's one Mallard Cloud request was refused for the account's
     sake: only this Mac's side is listed, and nothing is compared. */
  const pageCloud = managedCloud ? page?.cloud_status ?? null : null;
  const { status: itemStatus, retry: retryItemStatus } = useItemCloudStatus({
    scopeId: scope.scope.scope_id,
    storageId: itemRow ? storageId : null,
    resourceId: itemRow ? itemId : null,
    repositoryId: itemRepository,
    readKey: page,
  });
  /* An item that left the page (switched storage, removed) closes its page. */
  useEffect(() => {
    if (itemId && page && !loading && !rowByResource.has(itemId)) setItemId(null);
  }, [itemId, loading, page, rowByResource]);
  const pullReviewItems = useMemo(
    () => filterCatalogPullReviewItems(pullPlan?.items ?? [])
      .filter((item) => !itemRow || item.item.resource_id === itemRow.resource_id),
    [itemRow, pullPlan],
  );
  const pullReviewSelectedCount = pullReviewItems
    .filter((item) => pullSelected.has(item.item.resource_id)).length;

  /* The loaded page carries the profile at its current revision; the prop
     stands in before the first load. Rename pins the fresh revision. */
  const profile = page?.profile ?? scope.profile;
  const profileFolderName =
    profile.configured_home.split(/[\\/]/).filter(Boolean).pop() ?? "";
  const profileNameChanged = profileNameDraft.trim().length > 0
    && profileNameDraft.trim() !== profile.display_name;

  const saveProfileName = async () => {
    if (!profileNameChanged) return;
    setBusy(true);
    setRenameError(null);
    try {
      await storageCatalogApi.renameProfile(profile.profile_id, profileNameDraft.trim(), profile.revision);
      setRenamingProfile(false);
      onProfileRenamed();
      await load();
    } catch (reason) {
      setRenameError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };
  /* The bar counts what the page shows: the list's rows, or the one item. */
  const barRows = useMemo(() => (itemRow ? [itemRow] : rows), [itemRow, rows]);
  const pushRows = useMemo(() => barRows.filter(rowPushable), [barRows]);
  const pullRows = useMemo(() => barRows.filter(rowPullable), [barRows]);

  /* The bar's three numbers are read off the same rows the list draws — the
     project derives its own header tally the same way. */
  const counts = useMemo(() => barRows.reduce((totals, row) => {
    const status = rowStatus(row);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 }), [barRows]);

  const attentionDetail = useMemo(() => ([
    ["divergent", "changed on both sides"],
    ["pending", "waiting to sync"],
    ["unavailable", "can't be synced"],
  ] as Array<[string, string]>)
    .map(([status, phrase]) => {
      const count = rows.filter((row) => rowStatus(row) === status).length;
      return count === 0 ? null : `${count} ${noun(count)} ${phrase}`;
    })
    .filter((phrase): phrase is string => phrase !== null)
    .join(", "), [noun, rows]);

  /* Outside-click and Escape dismiss the action menu, the same contract the
     path bar's own menus keep. */
  useEffect(() => {
    if (!openChoices) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && pullListRef.current?.contains(target)) return;
      setOpenChoices(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      setOpenChoices(null);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [openChoices]);

  /* Stored-only rows with artwork ask for their preview after the list is on screen:
     one storage round-trip per row, kept out of the list call on purpose —
     this Mac may be offline, and a page must never wait on its decoration.
     A failure keeps the glyph tile, silently. */
  useEffect(() => {
    if (!artwork || !storageId) return;
    const scopeId = scope.scope.scope_id;
    const targets = rows.filter((row) => !row.local_present && row.remote_present
      && row.resource_id && !row.preview_png && !previewAttempts.current.has(row.row_id));
    if (targets.length === 0) return;
    let cancelled = false;
    void (async () => {
      for (const row of targets) {
        previewAttempts.current.add(row.row_id);
        try {
          const preview = await storageCatalogApi.getPreview(scopeId, storageId, row.resource_id!);
          if (cancelled) return;
          setRemotePreviews((current) => ({ ...current, [row.row_id]: preview }));
        } catch {
          // Decoration: offline or a missing payload keeps the glyph.
        }
        if (cancelled) return;
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [artwork, rows, scope.scope.scope_id, storageId]);

  /** The card's launch verbs. Failures land on the page's own error
   * surface, the same place every other verb reports. */
  const reveal = useCallback((row: CatalogScopeRow) => {
    if (!row.local_path) return;
    void revealItemInDir(row.local_path).catch((reason) => setError(errorMessage(reason)));
  }, []);

  /** The session card's launch pair, profile-scoped: open the provider
   * desktop app or the provider CLI on this row's configured home, at the
   * default path — no project and no session to name. */
  const openProfileApp = useCallback(() => {
    void storageCatalogApi.openProfileApp(profile.profile_id)
      .catch((reason) => setError(errorMessage(reason)));
  }, [profile.profile_id]);
  const openProfileTerminal = useCallback(() => {
    void storageCatalogApi.openProfileTerminal(profile.profile_id)
      .catch((reason) => setError(errorMessage(reason)));
  }, [profile.profile_id]);

  /** Push and Pull need the scope's one pairing with this storage. Linking is
   * configuration only, no provider content changed, so creating it as part
   * of the verb the user pressed is safe — and it is why nothing on this page
   * asks the user to link by hand. */
  const ensureLink = useCallback(async (): Promise<void> => {
    if (page?.link) return;
    if (!scope.available) throw new Error(unavailableReason);
    if (!storageId) throw new Error("choose a storage first");
    await storageCatalogApi.linkScopeStorage(scope.scope.scope_id, storageId);
  }, [page, scope.available, scope.scope.scope_id, storageId, unavailableReason]);

  const openReview = async (verb: "push" | "pull", remaining?: string[]) => {
    if (!storageId || prepareInFlight.current || applyInFlight.current) return;
    prepareInFlight.current = true;
    const epoch = operationEpoch.current;
    setBusy(true); setError(null); setFailures([]); setOutcome(null);
    // Preparing supersedes the previous backend plan. Cancel or failure must
    // return to the list, never leave an obsolete Apply button available.
    setMode({ kind: "list" }); setPushPlan(null); setPullPlan(null);
    let handle: ReturnType<typeof reviewPreparing.begin> | null = null;
    try {
      await ensureLink();
      if (epoch !== operationEpoch.current) return;
      preparingKind.current = verb;
      handle = reviewPreparing.begin();
      if (verb === "push") {
        const plan = await handle.race(storageCatalogApi.preparePush(scope.scope.scope_id, storageId));
        if (plan === PREPARING_CANCELLED || epoch !== operationEpoch.current) return;
        setPushPlan(plan);
        setPushSelected(new Set(plan.rows.filter(({ row }) => row.selected_by_default && (!remaining?.length || remaining.includes(row.resource_id))).map(({ row }) => row.resource_id)));
        setPushAcks(new Set()); setMode({ kind: "push" });
      } else {
        const plan = await handle.race(storageCatalogApi.preparePull(scope.scope.scope_id, storageId));
        if (plan === PREPARING_CANCELLED || epoch !== operationEpoch.current) return;
        setPullPlan(plan);
        const items = filterCatalogPullReviewItems(plan.items);
        setPullSelected(new Set(items.filter(({ item }) => item.default_action_id && (!remaining?.length || remaining.includes(item.resource_id))).map(({ item }) => item.resource_id)));
        setPullChoice(Object.fromEntries(items.filter(({ item }) => item.default_action_id).map(({ item }) => [item.resource_id, item.default_action_id!])));
        setPullAcks(new Set()); setMode({ kind: "pull" });
      }
    } catch (reason) { if (epoch === operationEpoch.current) setError(storageFailureMessage(reason)); }
    finally { handle?.finish(); prepareInFlight.current = false; if (epoch === operationEpoch.current) setBusy(false); }
  };
  /* On an item's page the review holds that item alone. */
  const openPush = () => openReview("push", itemRow?.resource_id ? [itemRow.resource_id] : undefined);
  const openPull = () => openReview("pull", itemRow?.resource_id ? [itemRow.resource_id] : undefined);

  /** The release and the discard name the item's saved repository. One the
   *  page learned from the account's listing is saved first, as the item's
   *  next Push or Pull would save it. */
  const savedItemRepository = async (): Promise<string> => {
    if (!itemId || !itemRepository || !storageId) throw new Error("This item has no repository yet.");
    if (!itemSaved) {
      const link = await storageCatalogApi.linkScopeStorage(scope.scope.scope_id, storageId);
      await storageCatalogApi.connectCloudRepository(scope.scope.scope_id, storageId, itemId, itemRepository, link.revision);
    }
    return itemRepository;
  };
  const requestRelease = async () => {
    const link = itemStatus?.link;
    if (!link || !itemId || !storageId || busy) return;
    setBusy(true);
    setError(null);
    try {
      const repositoryId = await savedItemRepository();
      await storageCatalogApi.requestCloudRelease(scope.scope.scope_id, storageId, itemId, repositoryId, link.repository_revision);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
    await load();
  };
  const discardDraft = async () => {
    if (!itemId || !storageId || busy) return;
    setBusy(true);
    try {
      await discardCloudDraft(itemStatus, {
        confirm,
        discard: async (expected) => storageCatalogApi.discardCloudDraft(
          scope.scope.scope_id, storageId, itemId, await savedItemRepository(), expected,
        ),
        report: setError,
        refresh: () => void load(),
      });
    } finally {
      setBusy(false);
    }
  };

  /* A finished Push or Pull in the words its storage's page uses. On
     Mallard Cloud they are the project's: the count names what moved, one
     repository's version leads, and each row names its repository. */
  const pushResult = (plan: CatalogScopePushPlan, result: CatalogScopePushOutcome) => (managedCloud
    ? cloudPushOutcome(result, {
      noun,
      addressOf: (resourceId) => plan.destinations?.[resourceId]?.address,
      at: Math.floor(Date.now() / 1_000),
    })
    : pushOutcome(result));
  const pullResult = (plan: CatalogScopePullPlan, choices: Record<string, string>, result: CatalogScopePullOutcome) => {
    if (!managedCloud) return pullOutcome(result);
    const pulled = Object.keys(choices);
    return cloudPullOutcome(result, {
      noun,
      addressOf: (resourceId) => plan.sources?.[resourceId]?.address,
      at: Math.floor(Date.now() / 1_000),
      keptAs: (resourceId) => {
        const action = plan.items.find(({ item }) => item.resource_id === resourceId)?.item.actions
          .find((candidate) => candidate.action_id === choices[resourceId]);
        return action?.kind.kind === "preserve_as_copy_and_overwrite" ? action.kind.copy.copy_name : null;
      },
      version: pulled.length === 1 ? plan.sources?.[pulled[0]]?.revision ?? plan.item_heads?.[pulled[0]]?.revision ?? null : null,
    });
  };

  const runPush = async () => {
    if (!pushPlan || applyInFlight.current) return;
    applyInFlight.current = true;
    const epoch = operationEpoch.current;
    setBusy(true); setError(null); setFailures([]);
    setProgress({ text: `Pushing ${pushSelected.size} ${noun(pushSelected.size)}…`, done: 0, total: pushSelected.size });
    try {
      const result = await storageCatalogApi.applyPush(pushPlan.plan_id, [...pushSelected], [...pushAcks], (event) => {
        if (epoch === operationEpoch.current && event.plan_id === pushPlan.plan_id) {
          setProgress({ text: `${event.message} (${event.completed}/${event.total})`, done: event.completed, total: event.total });
        }
      });
      if (epoch !== operationEpoch.current) return;
      setMode({ kind: "list" }); setPushPlan(null);
      await load();
      if (epoch !== operationEpoch.current) return;
      setOutcome(pushResult(pushPlan, result));
      if (result.stopped && finishPushOffered(result.stopped, storageName)) setFailures([result.stopped]);
    } catch (reason) {
      if (epoch !== operationEpoch.current) return;
      setMode({ kind: "list" }); setPushPlan(null);
      await load();
      if (epoch === operationEpoch.current) setError(`${storageFailureMessage(reason)} Review again before retrying.`);
    } finally {
      applyInFlight.current = false;
      if (epoch === operationEpoch.current) { setProgress(null); setBusy(false); }
    }
  };

  /* Finish that push: the record an earlier push claimed and never wrote is
     written once, and the page reports. Nothing runs this by itself, and
     nothing local moves: the user reviews again afterwards. */
  const finishUnfinishedPush = async () => {
    if (!storageId) return;
    setBusy(true);
    setError(null);
    try {
      await storageCatalogApi.finishUnfinishedPush(scope.scope.scope_id, storageId);
      setFailures([]);
      notify(PUSH_FINISHED);
    } catch (reason) {
      setFailures([storageFailureMessage(reason)]);
    } finally {
      setBusy(false);
    }
  };

  /** Opening the Pull review pins one plan over every stored resource. The
   * scope pairs with the storage first when it never has — the same "link for
   * review" step this page has always taken, configuration only, no provider
   * content changed. */
  const runPull = async () => {
    if (!pullPlan || applyInFlight.current) return;
    applyInFlight.current = true;
    const epoch = operationEpoch.current;
    const choices: Record<string, string> = {};
    for (const { item } of filterCatalogPullReviewItems(pullPlan.items)) {
      if (pullSelected.has(item.resource_id) && pullChoice[item.resource_id]) choices[item.resource_id] = pullChoice[item.resource_id];
    }
    setBusy(true); setError(null); setFailures([]);
    setProgress({ text: `Pulling ${Object.keys(choices).length} ${noun(Object.keys(choices).length)}…`, done: 0, total: Object.keys(choices).length });
    try {
      const result = await storageCatalogApi.applyPull(pullPlan.plan_id, choices, [...pullAcks].filter((id) => choices[id] != null), (event) => {
        if (epoch === operationEpoch.current && event.plan_id === pullPlan.plan_id) {
          setProgress({ text: `${event.message} (${event.completed}/${event.total})`, done: event.completed, total: event.total });
        }
      });
      if (epoch !== operationEpoch.current) return;
      setMode({ kind: "list" }); setPullPlan(null);
      await load();
      if (epoch === operationEpoch.current) setOutcome(pullResult(pullPlan, choices, result));
    } catch (reason) {
      if (epoch !== operationEpoch.current) return;
      setMode({ kind: "list" }); setPullPlan(null);
      await load();
      if (epoch === operationEpoch.current) setError(`${storageFailureMessage(reason)} Compare again to check completed changes.`);
    } finally {
      applyInFlight.current = false;
      if (epoch === operationEpoch.current) { setProgress(null); setBusy(false); }
    }
  };

  const unlinkStorage = async () => {
    if (!storageId || !pageLink) return;
    const approved = await confirm(
      `Unlink “${storageName}” from ${label}?\n\nPush and Pull stop for these ${plural}. Content already stored there and local content are left untouched, and you can link again later.`,
      { title: "Unlink storage" },
    );
    if (!approved) return;
    setBusy(true);
    setError(null);
    setFailures([]);
    try {
      await storageCatalogApi.unlinkScopeStorage(pageLink.link_id, pageLink.revision);
      notify("Storage link removed. Local and stored content were left untouched.");
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
    await load();
  };

  /* A refusal lands in the drawer that asked — never in the page banner. */
  const runFork = async (row: CatalogScopeRow, name: string) => {
    if (!row.resource_id) return;
    setBusy(true);
    setForkError(null);
    try {
      const plan = await storageCatalogApi.prepareFork(scope.scope.scope_id, row.resource_id, name.trim());
      const result = await storageCatalogApi.applyFork(scope.scope.scope_id, plan.plan_id);
      setForkFor(null);
      notify(result.message ?? `Created ${result.display_name}`);
      await load();
    } catch (reason) {
      setForkError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  // ---- the bar -----------------------------------------------------------

  /* The project workspace's trail exactly: the storage it syncs with, then
     the thing being synced. No Storages crumb — that belongs to the storage
     screens, which are rooted at their index; a workspace starts at its own
     outer scope. */
  const storage = storages.find((candidate) => candidate.id === storageId) ?? null;
  const crumbs: PathBarCrumb[] = [
    {
      id: "storage",
      label: storage?.name || (storageId ? "(unnamed)" : "No storage"),
      icon: storageIconName(storage?.provider, storage?.icon),
      tone: "root",
      warning: !storageId,
      title: storage ? `${storage.name || "Storage"} · ${storage.detail}` : undefined,
      ariaLabel: `${storage?.name ?? "No storage"}. Storage menu`,
      menuLabel: "Storage",
      menuHeader: {
        label: storage?.name || "No storage linked",
        detail: storage?.detail,
        detailIsPath: storage?.detailIsPath,
        icon: storageIconName(storage?.provider, storage?.icon),
      },
      /* A row that can never push or pull is not invited to add a storage. */
      menuBanner: storageId || unavailable
        ? undefined
        : `Add a storage to compare these ${plural} and turn on Push and Pull.`,
      menu: [
        {
          items: [
            {
              id: "details",
              label: "Storage details…",
              icon: "settings",
              disabled: !storageId || busy,
              onSelect: () => storageId && onOpenStorageDetails(storageId),
            },
            {
              id: "unlink",
              label: `Unlink from ${label}…`,
              icon: "link-off",
              danger: true,
              disabled: !storageId || busy || !pageLink,
              disabledReason: storageId ? "This row is not linked to it yet" : undefined,
              title: "Stop syncing this pairing; content already in storage is kept",
              onSelect: () => void unlinkStorage(),
            },
          ],
        },
        ...(storages.filter((candidate) => candidate.id !== storageId).length > 0 ? [{
          label: "Switch to",
          items: storages
            .filter((candidate) => candidate.id !== storageId)
            .map((candidate) => ({
              id: candidate.id,
              label: candidate.name || "(unnamed)",
              detail: candidate.detail,
              detailIsPath: candidate.detailIsPath,
              icon: storageIconName(candidate.provider, candidate.icon),
              disabled: busy,
              onSelect: () => {
                setChosenStorageId(candidate.id);
                setMode({ kind: "list" });
                setItemId(null);
              },
            })),
        }] : []),
        {
          items: [{
            id: "add-storage",
            label: "Add storage…",
            icon: "plus",
            disabled: busy,
            onSelect: onAddStorage,
          }],
        },
      ],
    },
    {
      id: "scope",
      label,
      icon: glyph.icon,
      title: `${label}\n${scope.profile.configured_home}`,
      ariaLabel: itemRow ? `${label}. Back to the list` : `${label}. Catalog row menu`,
      /* On an item's page the row is the way back, and keeps its menu. */
      onActivate: itemRow ? () => {
        setMode({ kind: "list" });
        setItemId(null);
      } : undefined,
      menuLabel: "Catalog row",
      menuHeader: {
        label,
        detail: scope.profile.configured_home,
        detailIsPath: true,
        icon: glyph.icon,
      },
      menu: [
        {
          items: [
            {
              id: "rescan",
              label: "Rescan this provider home",
              icon: "refresh",
              disabled: busy || loading,
              onSelect: () => void load(),
            },
            {
              id: "rename",
              label: "Rename profile…",
              icon: "edit",
              disabled: busy,
              title: `Renames “${profile.display_name}” for every Catalog row that uses this home`,
              onSelect: () => {
                setMode({ kind: "list" });
                setProfileNameDraft(profile.display_name);
                setRenameError(null);
                setRenamingProfile(true);
              },
            },
            {
              id: "remove",
              label: "Remove this Catalog row…",
              icon: "trash",
              danger: true,
              disabled: busy,
              title: "Stop listing this home's resources; nothing on disk or in storage is deleted",
              onSelect: onRemoveScope,
            },
          ],
        },
        ...(scopes.filter((candidate) => candidate.scope.scope_id !== scope.scope.scope_id).length > 0 ? [{
          label: "Switch to",
          items: scopes
            .filter((candidate) => candidate.scope.scope_id !== scope.scope.scope_id)
            .map((candidate) => ({
              id: candidate.scope.scope_id,
              label: catalogScopeLabel(candidate),
              detail: candidate.profile.configured_home,
              detailIsPath: true,
              icon: catalogScopeIcon(candidate),
              disabled: busy,
              onSelect: () => onOpenScope(candidate.scope.scope_id),
            })),
        }] : []),
        {
          items: [{
            id: "add-scope",
            label: "Add Catalog resource…",
            icon: "plus",
            disabled: busy,
            onSelect: onAddScope,
          }],
        },
      ],
    },
    ...(itemRow ? [{
      id: "item",
      label: itemRow.display_name,
      /* The item's own preview at a leaf's size, where a session leaf wears
         its provider's mark. */
      leading: artwork && (itemRow.preview_png ?? remotePreviews[itemRow.row_id])
        ? <img className="catalog-leaf-mark" src={itemRow.preview_png ?? remotePreviews[itemRow.row_id]} alt="" />
        : <Icon name={glyph.icon} size={14} />,
    }] : []),
  ];

  /* Mallard Cloud closes Pull and Push as it does for a project: the account
     refused, the item's repository not ready (a gone one reads as empty, and
     Push creates it again), or an item waiting for its repository to be
     chosen. */
  const cloudClosedReason = pageCloud
    ? pageCloud.message ?? "Mallard Cloud refused this account."
    : itemRow?.state === "blocked"
      ? itemRow.blocker ?? "Choose this item's repository first."
      : itemRow && cloudSyncBlocked(itemStatus)
        ? itemStatus?.message ?? "Mallard Cloud isn't ready for this item."
        : null;
  /* Mallard Cloud's bar speaks as a project's does: Preparing… while its
     review is read, the review's own verb while it is open (Push, Pull N),
     then Pushing… or Pulling…, and a count only while its review is shut. */
  const cloudRunning = (verb: "push" | "pull") => mode.kind === verb && progress !== null;
  const cloudSpinning = (verb: "push" | "pull") => managedCloud && (preparing === verb || cloudRunning(verb));
  const cloudVerb = (verb: "push" | "pull") => {
    if (cloudRunning(verb) && progress && progress.total > 1) {
      return <span className="sync-btn-readout">{`${progress.done}/${progress.total}…`}</span>;
    }
    const word = verb === "push" ? "Push" : "Pull";
    const label = preparing === verb
      ? "Preparing…"
      : cloudRunning(verb)
        ? `${word === "Push" ? "Pushing" : "Pulling"}…`
        : verb === "pull" && mode.kind === "pull" ? `Pull ${pullReviewSelectedCount}` : word;
    return <span className="sync-path-sync-label">{label}</span>;
  };
  const cloudCount = (verb: "push" | "pull") => {
    const count = verb === "push" ? counts.push : counts.pull;
    return mode.kind !== verb && preparing !== verb && count > 0
      ? <span className="sync-path-count" aria-hidden="true">{count}</span>
      : null;
  };
  /* On Mallard Cloud an open review holds the bar, as a project's does: the
     other verb waits until it closes. */
  const syncDisabled = busy || !storageId || loading || unavailable || cloudClosedReason !== null
    || (managedCloud && mode.kind !== "list");
  const syncDisabledReason = unavailable
    ? unavailableReason
    : cloudClosedReason
      ?? (storageId ? undefined : `Add a storage to these ${plural} before pushing or pulling`);
  /* The landing of a link that named this item, once: when its row has
     loaded, the review its Pull button opens; with nothing to pull, one line.
     A page whose sync is closed says why itself. */
  const landingPull = useRef(initialPull);
  useEffect(() => {
    if (!landingPull.current || !itemRow || loading || busy || mode.kind !== "list") return;
    landingPull.current = false;
    if (syncDisabled) return;
    if (pullRows.length > 0) void openPull();
    else notify("Already here and up to date.", "note");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [itemRow, loading, busy, mode.kind]);
  const pushReviewWarnings = pushPlan?.rows
    .filter((row) => pushSelected.has(row.row.resource_id))
    .flatMap((row) => row.row.warnings) ?? [];
  const pushReviewUnacknowledged = pushReviewWarnings
    .some((warning) => !pushAcks.has(warning.digest));
  const pushReviewApplyDisabled = busy
    || pushSelected.size === 0
    || pushReviewUnacknowledged;
  const pushReviewDisabledReason = pushSelected.size === 0
    ? `Select at least one ${noun(1)} to push`
    : pushReviewUnacknowledged
      ? "Acknowledge the selected warnings before pushing"
      : undefined;
  const pullReviewBlocked = pullReviewItems.some((item) => {
    const resourceId = item.item.resource_id;
    const actionId = pullChoice[resourceId];
    const action = item.item.actions.find((candidate) => candidate.action_id === actionId);
    return pullSelected.has(resourceId)
      && action?.requires_destructive_approval
      && !pullAcks.has(resourceId);
  }) ?? false;
  const pullReviewApplyDisabled = busy
    || pullReviewSelectedCount === 0
    || pullReviewBlocked;
  const pullReviewDisabledReason = pullReviewSelectedCount === 0
    ? `Select at least one ${noun(1)} to pull`
    : pullReviewBlocked
      ? "Acknowledge the selected overwrite before pulling"
      : undefined;
  const bar = (
    <SyncPathBar
      crumbs={crumbs}
      overflow={appActions}
      overflowLabel="App actions"
      openMenuRequest={openStorageMenuRequestId == null
        ? undefined
        : { menuId: "storage", requestId: openStorageMenuRequestId }}
      ariaLabel={`${label} path`}
      actions={(
        <>
          {/* An item's repository lives in its bar, as a project's does. */}
          {itemRow && itemRepository && (
            <ManagedCloudChip
              id={`cloud-${itemRow.row_id}`}
              status={itemStatus}
              storageName={storageName ?? "Mallard Cloud"}
              busy={busy}
              refreshing={!itemStatus}
              onOpenUrl={(url) => void openUrl(url).catch((reason) => setError(errorMessage(reason)))}
              onRequestRelease={() => void requestRelease()}
              onDiscardDraft={() => void discardDraft()}
              onRetry={retryItemStatus}
              onStorageSettings={() => { if (storageId) onOpenStorageDetails(storageId); }}
            />
          )}
          {!itemRow && counts.attention > 0 && (
            /* Not role="status": this is a standing count, not an event, and
               a live region re-announces it on every unrelated re-render. The
               label carries what the number means. */
            <span
              className="sync-path-attention warning"
              tabIndex={0}
              data-tooltip={attentionDetail}
              aria-label={`${counts.attention} needing attention. ${attentionDetail}`}
            >
              <Icon name="alert-triangle" size={13} /><b>{counts.attention}</b>
            </span>
          )}
          <button
            type="button"
            className="sync-path-icon-action"
            title="Refresh comparison"
            aria-label="Refresh comparison"
            disabled={busy || loading}
            onClick={() => void load()}
          >
            <span aria-hidden="true" className={`icon-spin-frame${loading ? " spinning" : ""}`}>
              <Icon name="refresh" size={16} />
            </span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button
              type="button"
              className={`sync-path-sync${mode.kind === "pull" || (managedCloud && preparing === "pull") ? " active" : ""}`}
              aria-expanded={mode.kind === "pull"}
              disabled={mode.kind === "pull"
                ? pullReviewApplyDisabled
                : syncDisabled || pullRows.length === 0}
              title={mode.kind === "pull" ? pullReviewDisabledReason : syncDisabledReason}
              onClick={() => void (mode.kind === "pull" ? runPull() : openPull())}
            >
              <Icon name={cloudSpinning("pull") ? "refresh" : "download"} size={14} className={cloudSpinning("pull") ? "icon-spin" : undefined} />
              {managedCloud ? cloudVerb("pull") : <span className="sync-path-sync-label">Pull</span>}
              {managedCloud
                ? cloudCount("pull")
                : mode.kind === "pull"
                  ? <span className="sync-path-count" aria-hidden="true">{pullReviewSelectedCount}</span>
                  : counts.pull > 0 && (
                    <span className="sync-path-count" aria-hidden="true">{counts.pull}</span>
                  )}
            </button>
            <button
              type="button"
              className={`sync-path-sync${mode.kind === "push" || (managedCloud && preparing === "push") ? " active" : ""}`}
              aria-expanded={mode.kind === "push"}
              disabled={mode.kind === "push"
                ? pushReviewApplyDisabled
                : syncDisabled || pushRows.length === 0}
              title={mode.kind === "push" ? pushReviewDisabledReason : syncDisabledReason}
              onClick={() => void (mode.kind === "push" ? runPush() : openPush())}
            >
              <Icon name={cloudSpinning("push") ? "refresh" : "upload"} size={14} className={cloudSpinning("push") ? "icon-spin" : undefined} />
              {managedCloud ? cloudVerb("push") : <span className="sync-path-sync-label">Push</span>}
              {managedCloud
                ? cloudCount("push")
                : mode.kind === "push"
                  ? <span className="sync-path-count" aria-hidden="true">{pushSelected.size}</span>
                  : counts.push > 0 && (
                    <span className="sync-path-count" aria-hidden="true">{counts.push}</span>
                  )}
            </button>
          </div>
        </>
      )}
    />
  );

  /* Keep work in flight and failures over an existing page on screen. An
     initial load failure uses the Retry panel below and is not repeated here.
     One zone takes the loudest severity; per-item failures keep one line each.
     Successes use the shell notice. */
  const pageNotices: NoticeItem[] = [];
  /* On Mallard Cloud the running verb reads out on its button, as a
     project's does; elsewhere the progress line stays in the zone. */
  if (progress && !managedCloud) {
    pageNotices.push({ key: "progress", severity: "info", message: progress.text });
  }
  if (error && page) {
    pageNotices.push({ key: "error", severity: "danger", message: error,
      action: mode.kind !== "list" ? <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void openReview(mode.kind as "push" | "pull")}>Compare again</button> : undefined,
    });
  }
  for (const failure of failures) {
    pageNotices.push({
      key: `failure:${failure}`,
      severity: "danger",
      message: failure,
      /* A push onto the selected version that did not finish offers the one
         verb that finishes it. */
      action: finishPushOffered(failure, storageName)
        ? (
          <button type="button" className="btn btn-ghost" onClick={() => void finishUnfinishedPush()} disabled={busy}>
            <Icon name="upload" size={13} />
            Finish that push
          </button>
        )
        : undefined,
    });
  }
  /* Nothing in this zone is standing content: the progress line is written
     while a Push or Pull runs, and both failure surfaces are written after
     one. A failure blocks because the requested operation did not happen;
     work in flight only advises. */
  const noticeTiming: NoticeTiming = (error && page) || failures.length > 0
    ? { timing: "blocking" }
    : { timing: "progress" };
  /* Mallard Cloud's notes, the project's words: what the account or the
     item's repository allows, each with its one control. They describe what
     the page read, so they stand. */
  const cloudNotes: NoticeItem[] = [];
  const cloudNote = (key: string, status: Parameters<typeof cloudStatusNotice>[0], retry: () => void) => {
    const note = cloudStatusNotice(status);
    if (!note) return;
    cloudNotes.push({
      key,
      severity: "warning",
      title: note.title,
      message: note.message,
      action: note.action === null ? undefined : (
        <button
          type="button"
          className="btn btn-ghost"
          disabled={busy}
          onClick={() => (note.action === "storage_settings" ? storageId && onOpenStorageDetails(storageId) : retry())}
        >
          {note.action === "storage_settings" ? "Storage settings" : "Retry"}
        </button>
      ),
    });
  };
  cloudNote("account", pageCloud, () => void load());
  if (itemRow && !pageCloud) cloudNote("repository", itemStatus, retryItemStatus);
  if (itemRow?.state === "blocked" && itemRow.resource_id) {
    const resourceId = itemRow.resource_id;
    cloudNotes.push({
      key: "several",
      severity: "warning",
      title: `${itemRow.display_name} is in more than one repository you can see.`,
      message: "Choose the one this Mac uses. Pull and Push wait until then.",
      action: (
        <button
          type="button"
          className="btn btn-ghost"
          disabled={busy || !storageId}
          onClick={() => setConnectReason({ kind: "several", resourceId, name: itemRow.display_name })}
        >
          Choose repository
        </button>
      ),
    });
  }
  const notices = (
    <>
      <NoticeZone items={pageNotices} subject="problems with this sync" {...noticeTiming} />
      <NoticeZone
        items={cloudNotes}
        timing="standing"
        label={`Notes about ${itemRow ? itemRow.display_name : "Mallard Cloud"}`}
        subject="Mallard Cloud notes"
      />
    </>
  );

  /* Review cards read the same list rows the page drew; a plan row whose page
     row vanished mid-review still renders from the plan's own facts. */
  const rowForItem = (item: CatalogPullItem): CatalogScopeRow => rowByResource.get(item.resource_id) ?? {
    row_id: `resource:${item.resource_id}`,
    display_name: item.display_name,
    resource_id: item.resource_id,
    scope_label: item.scope_label ?? null,
    state: null,
    local_present: false,
    remote_present: true,
    warnings: [],
  };

  /* On Mallard Cloud a review opens where a project's does: in the Storage
     card under the bar, Preparing… first. Any other storage keeps the
     Catalog's own review page. */
  const reviewFrame = (content: ReactNode) => (
    <>
      {bar}
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        {managedCloud ? (
          <section className="profile-links-section" aria-label={itemRow?.display_name ?? label}>
            <StorageCard name={storageName ?? "Mallard Cloud"} review>
              <div className="sync-storage-inline-review" tabIndex={-1}>{content}</div>
            </StorageCard>
          </section>
        ) : content}
      </main>
    </>
  );

  if (preparing) {
    const storageWord = storageName ?? "Mallard Cloud";
    return reviewFrame(
      <ReviewPreparing
        verb={preparing}
        title={`${preparing === "push" ? "Push" : "Pull"} review`}
        caption={!managedCloud
          ? `Comparing ${plural}`
          : preparing === "pull"
            ? `Reading the latest version from ${storageWord}`
            : `Comparing this computer with ${storageWord}`}
        onCancel={() => reviewPreparing.cancel()}
        cancelDisabled={false}
      />,
    );
  }

  // ---- Push review -------------------------------------------------------

  if (mode.kind === "push" && pushPlan) {
    /* Opened from an item's page, the review holds that item alone. */
    const planRows = pushPlan.rows.filter((row) => !itemRow || row.row.resource_id === itemRow.resource_id);
    const selectableRows = planRows.filter((row) => row.row.selectable);
    const visibleRows = selectableRows.filter((row) => nameMatchesSearch(
      rowByResource.get(row.row.resource_id)?.display_name ?? row.row.display_name, searchQuery,
    ));
    const matchingIds = visibleRows.map((row) => row.row.resource_id);
    const matchingSelected = matchingIds.filter((id) => pushSelected.has(id)).length;
    const heldRows = planRows.filter((row) => row.row.held_for_pull);
    const pushing = cloudRunning("push");
    return reviewFrame(
      <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="catalog-push-title">
        <header className="sync-inline-action-header sync-sync-review-header">
          <div className="sync-sync-review-title"><h2 id="catalog-push-title">Push review</h2></div>
          <SyncReviewSelectionBar
            count={pushSelected.size}
            total={selectableRows.length}
            busy={busy}
            recommendedCount={visibleRows.length}
            recommendedTitle={`Use the recommended selection${searchActive ? " for matching names" : ""} (${visibleRows.length})`}
            recommendedDisabled={matchingSelected === visibleRows.length}
            onRecommended={() => setPushSelected((current) => replaceSearchSelection(current, matchingIds, new Set(matchingIds)))}
            searchActive={searchActive}
            clearDisabled={matchingSelected === 0}
            onClear={() => setPushSelected((current) => replaceSearchSelection(current, matchingIds))}
            close={{ ariaLabel: "Close Push review", onClose: () => setMode({ kind: "list" }) }}
          />
        </header>
        <div className="sync-inline-action-content sync-sync-review-content">
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            {notices}
            <NameSearchField query={searchQuery} onChange={setSearchQuery} label={`Search ${plural}`}
              matches={visibleRows.length} total={selectableRows.length} noun={noun}
              hiddenSelected={pushSelected.size - matchingSelected} disabled={busy} />
            {searchActive && visibleRows.length === 0 && <div className="sync-history-state">No matching {plural}.</div>}
            {heldRows.length > 0 && (
              <div className="sync-review-banners">
                {/* The held rows are the pinned plan's own facts — on
                    screen the moment the review opens — so this is a named
                    region, not something that announces itself. It takes
                    the warning rung rather than the note one: those local
                    changes cannot push at all until the reader goes
                    through Pull, which is action the box is asking for. */}
                <Notice
                  severity="warning"
                  timing="standing"
                  label="Notes about this Push review"
                  title={`${heldRows.length} ${noun(heldRows.length)} held for Pull`}
                  action={(
                    <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void openPull()}>
                      Open Pull review
                    </button>
                  )}
                >
                  {`${storageName ?? "Storage"} has versions this Mac has not reviewed, so their local changes cannot push yet. Selected ${plural} still push.`}
                </Notice>
              </div>
            )}
            <div className="sync-history-thread-list">
              {visibleRows.map((planRow) => {
                const resourceId = planRow.row.resource_id;
                const row = rowByResource.get(resourceId) ?? {
                  row_id: `resource:${resourceId}`,
                  display_name: planRow.row.display_name,
                  resource_id: resourceId,
                  scope_label: planRow.row.scope_label ?? null,
                  state: null,
                  local_present: true,
                  remote_present: planRow.row.storage_content_sha256 != null,
                  warnings: planRow.row.warnings,
                };
                return (
                  <ResourceCard
                    key={resourceId}
                    row={row}
                    storageName={storageName}
                    storageRevision={storageRevision}
                    provider={scope.scope.provider}
                    providerLabel={profile.provider_label}
                    kindNoun={noun(1)}
                    onOpenApp={openProfileApp}
                    onOpenTerminal={openProfileTerminal}
                    preview={artwork ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
                    onReveal={() => reveal(row)}
                    repository={pushRepository(pushPlan.destinations?.[resourceId])}
                    selectable={planRow.row.selectable}
                    selected={planRow.row.selectable && pushSelected.has(resourceId)}
                    onToggle={planRow.row.selectable ? () => setPushSelected((current) => {
                      const next = new Set(current);
                      if (next.has(resourceId)) next.delete(resourceId);
                      else next.add(resourceId);
                      return next;
                    }) : undefined}
                    acks={planRow.row.selectable && pushSelected.has(resourceId) && planRow.row.warnings.map((warning) => {
                      /* Inside the card that raised it, as a row-sized
                         callout: a warning collected at the foot of the
                         list makes you count rows to find out who it is
                         about. */
                      const parts = warningParts(warning);
                      return (
                        <Acknowledgement
                          key={warning.digest}
                          title={parts.lead ?? undefined}
                          checked={pushAcks.has(warning.digest)}
                          ariaLabel={`Reviewed: ${warning.message}`}
                          verbs={["Mark reviewed", "Reviewed"]}
                          onChange={(checked) => setPushAcks((current) => {
                            const next = new Set(current);
                            if (checked) next.add(warning.digest);
                            else next.delete(warning.digest);
                            return next;
                          })}
                        >
                          {parts.lead === null ? parts.body : `— ${parts.body}`}
                        </Acknowledgement>
                      );
                    })}
                  />
                );
              })}
              {!searchActive && selectableRows.length === 0 && (
                <div className="sync-pull-empty">
                  <Icon name="check-circle" size={16} />
                  <span>
                    <strong>Nothing to push</strong>
                    <small>{`This Mac already matches ${storageName ?? "storage"}.`}</small>
                  </span>
                </div>
              )}
            </div>
          </div>
        </div>
        <footer className="sync-inline-action-footer sync-sync-review-footer">
          {selectableRows.length === 0 ? (
            /* Nothing can push, so the commit's slot holds the way out —
               the project Push review's rule. It names the page it lands
               on, the one headed by this scope's label. */
            <button
              type="button"
              className="btn sync-review-exit"
              disabled={busy}
              title={`Close Push review — back to ${itemRow?.display_name ?? scope.label}`}
              onClick={() => setMode({ kind: "list" })}
            >
              <span>{`Back to ${itemRow?.display_name ?? scope.label}`}</span>
            </button>
          ) : (
            <>
              {/* No live region: this reads off the plan's warnings and the
                  current selection, and it is on screen beside the button it
                  explains — the button's own disabled state is what a reader
                  arrives at. */}
              {pushReviewUnacknowledged && <span><small>Acknowledge the warnings above before pushing.</small></span>}
              {/* On Mallard Cloud, the project's contract: a running push
                  seals every exit, and says so. */}
              {pushing && <span role="status"><small>A running push finishes or fails whole — it can’t be cancelled midway.</small></span>}
              {/* The count rides the verb — the number at the point of
                  commitment, the way a Git client's commit button carries it.
                  On Mallard Cloud the verb is the project's: Push, with the
                  count in the selection. */}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={pushReviewApplyDisabled}
                onClick={() => void runPush()}
              >
                {pushing
                  ? <Icon name="refresh" size={16} className="icon-spin" />
                  : <ThreadStatusIcon status="local-outstanding" size={16} />}
                {managedCloud ? (pushing ? "Pushing…" : "Push") : `Push ${pushSelected.size}`}
              </button>
            </>
          )}
        </footer>
      </section>,
    );
  }

  // ---- Pull review -------------------------------------------------------

  if (mode.kind === "pull" && pullPlan) {
    const reviewItems = pullReviewItems;
    const visibleItems = reviewItems.filter((item) => nameMatchesSearch(rowForItem(item.item).display_name, searchQuery));
    const matchingIds = visibleItems.map((item) => item.item.resource_id);
    const matchingSelected = matchingIds.filter((id) => pullSelected.has(id)).length;
    const recommendation = catalogPullRecommendation(visibleItems, pullSelected, pullChoice);
    const chosenLabel = (item: (typeof reviewItems)[number]) => {
      const actionId = pullChoice[item.item.resource_id];
      const action = item.item.actions.find((candidate) => candidate.action_id === actionId);
      return action ? actionLabel(action) : "Skip";
    };
    const needsAck = (item: (typeof reviewItems)[number]) => {
      const actionId = pullChoice[item.item.resource_id];
      return item.item.actions
        .find((candidate) => candidate.action_id === actionId)?.requires_destructive_approval ?? false;
    };
    const claimed = reviewItems.filter((item) => item.conflicting_claims.length > 0);
    const blocked = pullReviewBlocked;
    const pulling = cloudRunning("pull");
    /* One repository has one version to name, as a project's header does; a
       batch reads several, and each row's readout names its own. */
    const pullVersion = managedCloud && itemRow?.resource_id
      ? pullPlan.sources?.[itemRow.resource_id]?.revision ?? pullPlan.item_heads?.[itemRow.resource_id]?.revision ?? null
      : null;
    return reviewFrame(
      <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="catalog-pull-title">
        <header className="sync-inline-action-header sync-sync-review-header">
          <div className="sync-sync-review-title">
            <h2 id="catalog-pull-title">Pull review</h2>
            {pullVersion != null && <span className="sync-pull-review-meta">Version {pullVersion}</span>}
          </div>
          <SyncReviewSelectionBar
            count={pullReviewSelectedCount}
            total={reviewItems.length}
            busy={busy}
            recommendedCount={recommendation.items.length}
            recommendedTitle={`Use the recommended local actions${searchActive ? " for matching names" : ""} (${recommendation.items.length})`}
            recommendedDisabled={recommendation.matches}
            onRecommended={() => {
              setPullSelected((current) => replaceSearchSelection(current, matchingIds,
                new Set(recommendation.items.map((item) => item.item.resource_id))));
              setPullChoice((current) => {
                const next = { ...current };
                matchingIds.forEach((id) => { delete next[id]; });
                recommendation.items.forEach((item) => { next[item.item.resource_id] = item.item.default_action_id!; });
                return next;
              });
              setPullAcks((current) => replaceSearchSelection(current, matchingIds));
            }}
            searchActive={searchActive}
            clearDisabled={matchingSelected === 0}
            onClear={() => setPullSelected((current) => replaceSearchSelection(current, matchingIds))}
            close={{ ariaLabel: "Close Pull review", onClose: () => setMode({ kind: "list" }) }}
          />
        </header>
        <div className="sync-inline-action-content sync-sync-review-content">
          <div className="sync-inline-action-scroll sync-sync-review-scroll">
            {notices}
            <NameSearchField query={searchQuery} onChange={setSearchQuery} label={`Search ${plural}`}
              matches={visibleItems.length} total={reviewItems.length} noun={noun}
              hiddenSelected={pullReviewSelectedCount - matchingSelected} disabled={busy} />
            {searchActive && visibleItems.length === 0 && <div className="sync-history-state">No matching {plural}.</div>}
            {/* One banner zone above the list, the way the project
                reviews place plan-derived facts — never a zone per row. */}
            {claimed.length > 0 && (
              <div className="sync-review-banners">
                {/* Every claim came in with the plan this review pinned,
                    so the zone is a named region and never announces. One
                    box holds all claims. */}
                <NoticeZone
                  timing="standing"
                  label="Notes about this Pull review"
                  subject={`${plural} claimed elsewhere`}
                  items={claimed.map((item) => ({
                    key: item.item.resource_id,
                    severity: "warning" as const,
                    title: `${item.item.display_name} is claimed by ${item.conflicting_claims
                      .map((claim) => storages.find((storage) => storage.id === claim.storage_id)?.name ?? claim.storage_id)
                      .join(", ")}`,
                    message:
                      "A different version of this local target is pinned there. Preserve a copy, acknowledge overwrite, or Skip.",
                  }))}
                />
              </div>
            )}
            <div className="sync-history-thread-list" ref={pullListRef}>
              {visibleItems.map((item) => {
                const resourceId = item.item.resource_id;
                const row = rowForItem(item.item);
                const included = pullSelected.has(resourceId);
                const expanded = openChoices === resourceId;
                const chosen = chosenLabel(item);
                const executable = item.item.disposition.kind === "executable";
                return (
                  <ResourceCard
                    key={resourceId}
                    row={row}
                    storageName={storageName}
                    storageRevision={storageRevision}
                    provider={scope.scope.provider}
                    providerLabel={profile.provider_label}
                    kindNoun={noun(1)}
                    onOpenApp={openProfileApp}
                    onOpenTerminal={openProfileTerminal}
                    preview={artwork ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
                    onReveal={() => reveal(row)}
                    repository={managedCloud ? pullRepository(pullPlan.sources?.[resourceId]) : undefined}
                    selectable={executable}
                    selected={included}
                    onToggle={() => setPullSelected((current) => {
                      const next = new Set(current);
                      if (next.has(resourceId)) next.delete(resourceId);
                      else next.add(resourceId);
                      return next;
                    })}
                    action={executable && included ? {
                      chosen,
                      options: item.item.actions.map((action) => ({
                        id: action.action_id,
                        label: actionLabel(action),
                      })),
                      expanded,
                      destructive: needsAck(item),
                      onToggle: () => setOpenChoices(expanded ? null : resourceId),
                      onChoose: (actionId) => {
                        setPullChoice((current) => ({ ...current, [resourceId]: actionId }));
                        setPullAcks((current) => {
                          const next = new Set(current);
                          next.delete(resourceId);
                          return next;
                        });
                        setOpenChoices(null);
                      },
                    } : undefined}
                    acks={included && needsAck(item) && (
                      /* Part of the card it belongs to, not the footer:
                         the footer says why the button is off, this says
                         which row turned it off. */
                      /* A Warning, not an Error: nothing has failed; the
                         verb carries the risk. */
                      <Acknowledgement
                        title="Overwrites this Mac’s copy"
                        checked={pullAcks.has(resourceId)}
                        ariaLabel={`Approve overwriting ${item.item.display_name}`}
                        verbs={["Approve overwrite", "Approved"]}
                        onChange={(checked) => setPullAcks((current) => {
                          const next = new Set(current);
                          if (checked) next.add(resourceId);
                          else next.delete(resourceId);
                          return next;
                        })}
                      >
                        {`— replaces the current ${item.item.display_name} here with the stored version`}
                      </Acknowledgement>
                    )}
                  />
                );
              })}
              {!searchActive && reviewItems.length === 0 && !busy && (
                <div className="sync-pull-empty">
                  <Icon name="check-circle" size={16} />
                  <span>
                    <strong>Nothing to pull</strong>
                    <small>{`These ${plural} already match ${storageName ?? "storage"}.`}</small>
                  </span>
                </div>
              )}
            </div>
          </div>
        </div>
        <footer className="sync-inline-action-footer sync-sync-review-footer">
          {reviewItems.length === 0 ? (
            /* Nothing to pull: the commit's slot holds the way out, as on
               Push. */
            <button
              type="button"
              className="btn sync-review-exit"
              disabled={busy}
              title={`Close Pull review — back to ${itemRow?.display_name ?? scope.label}`}
              onClick={() => setMode({ kind: "list" })}
            >
              <span>{`Back to ${itemRow?.display_name ?? scope.label}`}</span>
            </button>
          ) : (
            <>
              {/* It announces: choosing Overwrite from a row's menu is what
                  makes this appear, and it appears at the same moment the Pull
                  button goes disabled. A polite region does not announce what
                  is already present, so the case where the plan opens blocked
                  costs nothing. */}
              {blocked && <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>}
              {pulling && <span role="status"><small>Pull cannot be cancelled while changes are being applied.</small></span>}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={pullReviewApplyDisabled}
                onClick={() => void runPull()}
              >
                {pulling
                  ? <Icon name="refresh" size={16} className="icon-spin" />
                  : <ThreadStatusIcon status="cloud-outstanding" size={16} />}
                {pulling ? "Pulling…" : `Pull ${pullReviewSelectedCount}`}
              </button>
            </>
          )}
        </footer>
      </section>,
    );
  }

  // ---- the list ----------------------------------------------------------

  /* Stored-only rows lead the page, the way the activity page leads with its
     storage-only sessions: what is not on this Mac is the one thing the list
     cannot say row by row, so it gets said by position. A row that is nowhere
     at all (a link whose content vanished on both sides) stays in the kind's
     own section — "not in the storage" would be the wrong heading for it. */
  const matchingRows = rows.filter((row) => nameMatchesSearch(row.display_name, searchQuery));
  const cloudRows = matchingRows.filter((row) => !row.local_present && row.remote_present);
  const localRows = matchingRows.filter((row) => row.local_present || !row.remote_present);

  const renderRow = (row: CatalogScopeRow) => {
    const rowForkable = forkable && row.local_present && !!row.resource_id;
    const forkOpen = rowForkable && forkFor === row.resource_id;
    const resourceId = row.resource_id;
    return (
      <ResourceCard
        key={row.row_id}
        /* On Mallard Cloud each item is its own repository: its name opens
           its page. The page's own card is where you already are. */
        onOpen={managedCloud && resourceId && !itemRow ? () => {
          setOutcome(null);
          setItemId(resourceId);
        } : undefined}
        row={row}
        storageName={storageName}
        storageRevision={storageRevision}
        provider={scope.scope.provider}
        providerLabel={profile.provider_label}
        kindNoun={noun(1)}
        onOpenApp={openProfileApp}
        onOpenTerminal={openProfileTerminal}
        preview={artwork ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
        onReveal={() => reveal(row)}
        onFork={forkable ? () => {
          setForkError(null);
          setForkFor(forkOpen ? null : row.resource_id!);
        } : undefined}
        forkOpen={forkOpen}
        forkTitle={`Fork ${row.display_name} — duplicate this ${noun(1)} under a new name`}
        footer={forkOpen && (
          <ForkDrawer
            displayName={row.display_name}
            seed={forkSeed(row)}
            maxLength={128}
            busy={busy}
            error={forkError}
            onEdited={() => setForkError(null)}
            onSubmit={(name) => void runFork(row, name)}
            onClose={() => {
              setForkError(null);
              setForkFor(null);
            }}
          />
        )}
      />
    );
  };

  /* The section heading a row sits under: what is only in the storage, or
     the kind's own. On Mallard Cloud the storage's mark names the storage,
     and the count closes the heading, as a project's "Only in" does. */
  const cloudHeading = (count: number) => (
    <div className="sync-history-section-heading">
      {managedCloud ? (
        <>
          <h2 title={`Only in ${storageName ?? "Mallard Cloud"}`}><Icon name="storage-mallard" size={14} />Not downloaded</h2>
          <span
            className="sync-history-heading-count"
            title={`${count} ${noun(count)} only in ${storageName ?? "Mallard Cloud"}`}
            aria-label={`${count} ${noun(count)} only in ${storageName ?? "Mallard Cloud"}`}
          >
            <Icon name={glyph.icon} size={12} />{count}
          </span>
        </>
      ) : (
        <h2>
          <Icon name="cloud" size={14} />
          Only in <span className="sync-history-storage-name">{storageName ?? "storage"}</span>
        </h2>
      )}
    </div>
  );
  const kindHeading = (
    <div className="sync-history-section-heading">
      <h2><Icon name={glyph.icon} size={14} />{scope.label}</h2>
    </div>
  );

  /* A finished Push or Pull. On Mallard Cloud it lands on the Storage card,
     as a project's does, at the head of the list or the item's page; any
     other storage keeps the Catalog's strip over the page. */
  const outcomeNotice = outcome && (
    <SyncOutcomeNotice outcome={outcome} busy={busy} onDismiss={() => setOutcome(null)} onAction={(action) => void openReview(action.kind, action.resourceIds)} />
  );
  const outcomeCard = managedCloud && outcomeNotice
    ? <StorageCard name={storageName ?? "Mallard Cloud"}>{outcomeNotice}</StorageCard>
    : null;

  /* An item's page: while it has no repository, the one its first Push
     creates, with Change… to choose another; then its own card, under the
     heading the list puts it under. Once it has a repository, that lives in
     the bar's Cloud chip and is fixed. */
  const itemPage = itemRow && (() => {
    const resourceId = itemRow.resource_id!;
    const name = itemRow.display_name;
    const prediction = predictRepositoryAddress(accountLogin, name, []);
    const address = accountLogin ? prediction.address : prediction.name;
    return (
      <section className="profile-links-section" aria-label={name}>
        {outcomeCard}
        {!itemRepository && itemRow.state !== "blocked" && !pageCloud && (
          <>
            <div className="sync-history-section-heading">
              <h2><Icon name="cloud" size={14} />Repository</h2>
            </div>
            <div className="sync-history-thread-list">
              <article className="sync-history-thread-card catalog-card" aria-label={`${name} repository`}>
                <div className="sync-history-thread-topline">
                  <div className="sync-history-thread-title">
                    <span
                      className="sync-thread-sync-indicator untracked"
                      data-tooltip={`Not in ${storageName ?? "Mallard Cloud"} yet`}
                      aria-label={`Not in ${storageName ?? "Mallard Cloud"} yet`}
                      tabIndex={0}
                    >
                      <Icon name="cloud" size={18} />
                    </span>
                    <strong title={address}>{address}</strong>
                    <span className="sync-history-branch-state current" title="Your first Push creates it">New</span>
                    <span className="sync-history-branch-state current" title="Only you and your collaborators can see it">Private</span>
                  </div>
                  <div className="sync-history-thread-actions">
                    <button
                      type="button"
                      className="btn"
                      aria-haspopup="dialog"
                      aria-label="Change repository"
                      disabled={busy || !storageId}
                      onClick={() => setConnectReason({ kind: "connect", resourceId, name })}
                    >
                      <Icon name="link" size={13} />
                      Change…
                    </button>
                  </div>
                </div>
              </article>
            </div>
          </>
        )}
        {!itemRow.local_present && itemRow.remote_present ? cloudHeading(1) : kindHeading}
        <div className="sync-history-thread-list">{renderRow(itemRow)}</div>
      </section>
    );
  })();

  const dialog = connectReason && storageId && (
    <CatalogConnectDialog
      key={connectReason.kind === "add" ? "add" : `${connectReason.kind}:${connectReason.resourceId}`}
      reason={connectReason}
      scopeId={scope.scope.scope_id}
      storageId={storageId}
      accountLogin={accountLogin}
      coordinate={scope.scope}
      noun={noun(1)}
      rowLabel={label}
      listed={new Set(rows.flatMap((row) => (row.resource_id ? [row.resource_id] : [])))}
      capabilities={capabilities}
      onClose={() => setConnectReason(null)}
      onConnected={(connected) => {
        setConnectReason(null);
        notify(connectReason.kind === "add"
          ? `${connected.name} added from ${connected.address}.`
          : `${connected.name} connected to ${connected.address}.`);
        setOutcome(null);
        setItemId(connected.resourceId);
        void load();
      }}
    />
  );

  /* Add from Mallard Cloud: someone else's item, shared with you or by
     address or invitation code. The list's own verb, beside the list it adds
     to, with the sidebar's + for adding a row. Mallard Cloud only. */
  const addFromCloud = managedCloud && (
    <button
      type="button"
      className="btn"
      aria-haspopup="dialog"
      title={`Someone else’s ${noun(1)}: shared with you, or by address or invitation code`}
      disabled={busy || loading || mode.kind !== "list" || !storageId || !!pageCloud}
      onClick={() => setConnectReason({ kind: "add" })}
    >
      <Icon name="plus" size={13} />
      Add from Mallard Cloud…
    </button>
  );

  return (
    <>
      {bar}
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        {dialog}
        {managedCloud ? rows.length === 0 && !itemPage && outcomeCard : outcomeNotice}

        {renamingProfile && (
          <form
            className="sync-path-rename"
            onSubmit={(event) => {
              event.preventDefault();
              void saveProfileName();
            }}
          >
            <label htmlFor="catalog-profile-rename-input">Profile name</label>
            <input
              id="catalog-profile-rename-input"
              value={profileNameDraft}
              onChange={(event) => setProfileNameDraft(event.target.value)}
              aria-label="Profile name"
              disabled={busy}
              autoFocus
            />
            <button
              type="submit"
              className="btn btn-primary"
              disabled={busy || !profileNameChanged}
            >
              Save
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy}
              onClick={() => {
                setProfileNameDraft(profile.display_name);
                setRenamingProfile(false);
                setRenameError(null);
              }}
            >
              Cancel
            </button>
            {profileFolderName && profileFolderName !== profileNameDraft.trim() && (
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => setProfileNameDraft(profileFolderName)}
              >
                Use folder name
              </button>
            )}
            {renameError && (
              <div className="sync-path-rename-error" role="alert">
                <Icon name="alert-triangle" size={13} /> {renameError}
              </div>
            )}
          </form>
        )}
        {notices}
        {/* Recovery messages arrive with the page, so they are standing
            content in one named box. */}
        <NoticeZone
          timing="standing"
          label="Notes about this Catalog row"
          subject="notes about this Catalog row"
          items={(page?.recovery ?? []).map((message) => ({
            key: message,
            severity: "warning" as const,
            message,
          }))}
        />
        {unavailable ? (
          /* The backend's reason, standing: nothing was asked of the home or
             the storage, and nothing on the bar can be pressed. */
          <Notice
            severity="danger"
            timing="standing"
            label="Why this Catalog row is unavailable"
            title={`${label} can’t be used in this build.`}
          >
            {unavailableReason}
          </Notice>
        ) : loading && !page ? (
          <LoadingScaffold shape="rows" label={`Loading ${label}`} />
        ) : error && !page ? (
          /* "None stored yet" is a fact about the storage. When the request
             that would have listed them failed, we know no such thing — and
             saying it anyway tells the user their content is gone.

             The failure is what happened to the load the page just ran, so it
             blocks; the empty-state panel is for what the page found, not for
             what it could not ask. */
          <Notice
            severity="danger"
            timing="blocking"
            title={`Couldn’t load ${label}.`}
            action={(
              <button type="button" className="btn" onClick={() => void load()}>
                <Icon name="refresh" size={14} /> Retry
              </button>
            )}
          >
            {error}
          </Notice>
        ) : itemPage ? (
          itemPage
        ) : rows.length === 0 ? (
          /* Its own container: wrapping one sentence in a list card drew a
             second border around it. Nor does it name the home or the
             storage — the crumb bar above is already saying both. */
          <div className="sync-history-state">
            {`No ${noun(1)} on this Mac, and none stored yet.`}
            {addFromCloud}
          </div>
        ) : (
          <section className="profile-links-section" aria-label={label}>
            {outcomeCard}
            {/* On Mallard Cloud the head gains Add from Mallard Cloud…
                beside the search, outside its rule, and holds the count
                back. Any other storage keeps today's head. */}
            <NameSearchField query={searchQuery} onChange={setSearchQuery} label={`Search ${plural}`}
              matches={matchingRows.length} total={rows.length} noun={noun} disabled={busy}
              count={!managedCloud} trailing={addFromCloud} />
            {searchActive && matchingRows.length === 0 && <div className="sync-history-state">No matching {plural}.</div>}
            {/* Two plain sections, one list broken in two: the same heading
                shape at the same weight, the same cards at the same rhythm.
                No counts on either heading — every row already wears the same
                provider mark, so a mark-plus-number chip here read as one more
                row rather than a summary. */}
            {cloudRows.length > 0 && (
              <>
                {cloudHeading(cloudRows.length)}
                <div className="sync-history-thread-list">
                  {cloudRows.map(renderRow)}
                </div>
              </>
            )}
            {localRows.length > 0 && (
              <>
                {kindHeading}
                <div className="sync-history-thread-list">
                  {localRows.map(renderRow)}
                </div>
              </>
            )}
          </section>
        )}
      </main>
    </>
  );
}
