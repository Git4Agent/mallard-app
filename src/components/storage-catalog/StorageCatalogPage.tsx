import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import Icon, { type IconName } from "../Icons";
import { formatFullDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { errorMessage, storageAccessErrorMessage } from "../../storageError";
import Notice, { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import LoadingScaffold from "../LoadingScaffold";
import { writeClipboardText } from "../sync-review/clipboard";
import TopLayer from "../sync-review/TopLayer";
import { useDebugPopoverPlacement } from "../sync-review/debugPopoverPlacement";
import { focusMenuEdge, moveMenuFocus } from "../sync-review/menuKeyboard";
import ProviderMark from "../sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import SyncReviewSelectionBar from "../sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "../sync-review/ThreadStatusIcon";
import { storageCatalogApi } from "./api";
import {
  actionLabel,
  catalogKindMeta,
  providerLabel,
  catalogScopeLabel,
  rowPullable,
  rowPushable,
  rowStamp,
  rowStatus,
  rowStatusLabel,
} from "./model";
import type {
  CatalogProviderKey,
  CatalogPullItem,
  CatalogResourceKind,
  CatalogResourceWarning,
  CatalogScopePullPlan,
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
  name: string;
  kind: "local" | "s3";
  /** The one-line "where is it" the shell already computes for every storage
   * menu — a folder for local, a bucket for S3. */
  detail: string;
  detailIsPath: boolean;
}

type Mode = { kind: "list" } | { kind: "push" } | { kind: "pull" };

/** Mirrors the shell's notice tones without importing them: storage-catalog
 *  may not depend on project-sync. */
type CatalogNoticeTone = "ok" | "attention";

/** "Codex pet v1", "Plugin intent", "Codex home skill" — the phrase the fork
 * button carries so the copy names exactly what it duplicates. */
function resourceKindPhrase(kind: CatalogResourceKind, scopeLabel?: string | null): string {
  if (kind === "codex_pet_v1") return "Codex pet v1";
  if (kind === "codex_pet_v2") return "Codex pet v2";
  if (kind === "plugin_install_intent") return "Plugin intent";
  return scopeLabel ? `${scopeLabel} skill` : "Skill";
}

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

/** The acknowledgement chip's lead, keyed by the warning's code — the backend
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
function InfoMenu({ row, storageRevision }: { row: CatalogScopeRow; storageRevision: number | null }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const placement = useDebugPopoverPlacement(rootRef, panelRef, open, () => setOpen(false));

  useEffect(() => {
    if (!open) return;
    // The panel is portalled onto body, so it is not inside rootRef. Both have
    // to count as "inside" or the first click on a Copy button dismisses it.
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      /* This layer consumed Escape; the Push/Pull review listens on window
         and must not also close and discard the whole review. */
      event.stopPropagation();
      setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

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
          storageRevision == null ? null : `Revision ${storageRevision}`,
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
        type="button"
        className="btn btn-ghost sync-history-session-detail-action"
        title="Paths and digests"
        aria-label={`Paths and digests for ${row.display_name}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => { setCopied(null); setOpen(!open); }}
      >
        <Icon name="info" size={13} />
      </button>
      {/* On body, not in the row. Nothing in the list can bound it there:
          not the row's card, not the commit rail's stacking context, and
          not .sync-main's scrollport. */}
      <TopLayer>
        <div
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
          aria-label="Paths and digests"
          hidden={!open}
        >
          <header>
            <strong>Paths and digests</strong>
            <span>Click a value to copy it.</span>
          </header>
          <div className="sync-history-debug-values">
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

/** The acknowledgement chip, payload-first: the bold lead names the finding,
 * the body carries the flagged paths, and the ack verb is the chip's trailing
 * word — livening once ticked — rather than a constant preamble in front of
 * every sentence. The chip spans the card's full row; the accessible name
 * still reads the backend's whole sentence. */
function AckChip({ danger, lead, body, checked, ariaLabel, verbs, onToggle }: {
  danger?: boolean;
  lead: string | null;
  body: string;
  checked: boolean;
  ariaLabel: string;
  /** The trailing word, unticked then ticked: Mark reviewed → Reviewed for a
   * scan finding, Approve → Approved for an overwrite. */
  verbs: [string, string];
  onToggle: (checked: boolean) => void;
}) {
  return (
    <label className={`catalog-card-ack${danger ? " danger" : ""}`}>
      <span className="sync-sync-row-choice">
        <input type="checkbox" checked={checked} aria-label={ariaLabel} onChange={(event) => onToggle(event.target.checked)} />
      </span>
      <Icon name="alert-triangle" size={12} />
      <span className="catalog-card-ack-copy">
        {lead === null ? body : <><strong>{lead}</strong> — {body}</>}
      </span>
      <small className={checked ? "done" : undefined}>{checked ? verbs[1] : verbs[0]}</small>
    </label>
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
}: {
  row: CatalogScopeRow;
  storageName: string | null;
  storageRevision: number | null;
  provider: CatalogProviderKey;
  /** The session card's launch pair, minus the session: the provider mark
   * opens the desktop app on this row's profile home, and Terminal opens the
   * provider CLI there — both at the default path. */
  onOpenApp: () => void;
  onOpenTerminal: () => void;
  /** The pet's idle frame as a data URI. `null` draws the kind-glyph tile —
   * the column never collapses and never shows a broken image — and
   * `undefined` (non-pet scopes) draws no tile at all. */
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
          data-tooltip={row.pet_description ?? undefined}
          tabIndex={row.pet_description ? 0 : undefined}
          aria-hidden={row.pet_description ? undefined : true}
        >
          {preview
            ? <img src={preview} alt={`${row.display_name} preview`} />
            : <Icon name="paw" size={16} />}
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
          <strong title={row.display_name}>{row.display_name}</strong>
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
            <span className="catalog-row-action" title={action.chosen}>{actionVerb(action.chosen)}</span>
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
            title={`Open in ${providerLabel(provider)}`}
            aria-label={`Open in ${providerLabel(provider)}`}
            onClick={onOpenApp}
          >
            <ProviderMark provider={provider} displayName={providerLabel(provider)} />
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
            <InfoMenu row={row} storageRevision={storageRevision} />
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
  appActions,
  onOpenScope,
  onOpenStorageSettings,
  onAddStorage,
  onAddScope,
  onRemoveScope,
  onProfileRenamed,
  openStorageMenuRequestId,
  onScopeGone,
  notify,
}: {
  scope: CatalogScopeView;
  /** The other Catalog rows, for this row's Switch-to menu. */
  scopes: CatalogScopeView[];
  storages: CatalogStorageOption[];
  /** Theme and app-wide commands: this bar is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onOpenScope: (scopeId: string) => void;
  onOpenStorageSettings: (storageId: string) => void;
  onAddStorage: () => void;
  onAddScope: () => void;
  onRemoveScope: () => void;
  /** Reload the scope list after a profile rename so shell labels update. */
  onProfileRenamed: () => void;
  /** Opens the storage crumb menu for recovery actions initiated by the shell. */
  openStorageMenuRequestId?: number | null;
  /** The row disappeared underneath the page (deleted elsewhere). */
  onScopeGone: () => void;
  /** Outcomes go to the shell's one notification surface, the way every other
   * screen reports them — not to a block that pushes the list down. The tone
   * union is declared locally: this module may not import from project-sync. */
  notify: (message: string, tone?: CatalogNoticeTone) => void;
}) {
  const label = catalogScopeLabel(scope.scope, scope.profile);
  const kind = catalogKindMeta(scope.scope.resource_kind);
  /* Pets are the one kind that carries artwork: their rows grow a preview
     tile, filled inline for local atlases and lazily for stored-only rows. */
  const petKind = scope.scope.resource_kind === "codex_pet_v1"
    || scope.scope.resource_kind === "codex_pet_v2";
  const forkableKind = scope.scope.resource_kind !== "plugin_install_intent";
  const [page, setPage] = useState<CatalogScopeResources | null>(null);
  const [chosenStorageId, setChosenStorageId] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
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
     storage's kind, which is all the translation needs. */
  const storageFailureMessage = (reason: unknown): string => storageAccessErrorMessage(
    storages.find((candidate) => candidate.id === (chosenStorageId ?? storageId)) ?? null,
    reason,
  );

  const onScopeGoneRef = useRef(onScopeGone);
  onScopeGoneRef.current = onScopeGone;

  const load = useCallback(async () => {
    const epoch = ++epochRef.current;
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
  }, [chosenStorageId, scope.scope.scope_id]);

  useEffect(() => {
    void load();
    return () => {
      epochRef.current += 1;
    };
  }, [load]);

  useEffect(() => {
    setMode({ kind: "list" });
    setChosenStorageId(null);
    setForkFor(null);
  }, [scope.scope.scope_id]);

  useEffect(() => {
    previewAttempts.current = new Set();
    setRemotePreviews({});
  }, [scope.scope.scope_id, chosenStorageId]);

  const rows = useMemo(() => page?.rows ?? [], [page]);
  const storageName = page?.storage_name ?? null;
  const storageId = page?.storage_id ?? null;
  const pageLink = page?.link ?? null;
  const storageRevision = page?.storage_revision ?? null;
  const rowByResource = useMemo(() => {
    const map = new Map<string, CatalogScopeRow>();
    for (const row of rows) {
      if (row.resource_id) map.set(row.resource_id, row);
    }
    return map;
  }, [rows]);

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
  const pushRows = useMemo(() => rows.filter(rowPushable), [rows]);
  const pullRows = useMemo(() => rows.filter(rowPullable), [rows]);

  /* The bar's three numbers are read off the same rows the list draws — the
     project derives its own header tally the same way. */
  const counts = useMemo(() => rows.reduce((totals, row) => {
    const status = rowStatus(row);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 }), [rows]);

  const attentionDetail = useMemo(() => ([
    ["divergent", "changed on both sides"],
    ["pending", "waiting to sync"],
    ["unavailable", "can't be synced"],
  ] as Array<[string, string]>)
    .map(([status, phrase]) => {
      const count = rows.filter((row) => rowStatus(row) === status).length;
      const noun = count === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
      return count === 0 ? null : `${count} ${noun} ${phrase}`;
    })
    .filter((phrase): phrase is string => phrase !== null)
    .join(", "), [kind, rows]);

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

  /* Stored-only pet rows ask for their preview after the list is on screen:
     one storage round-trip per row, kept out of the list call on purpose —
     this Mac may be offline, and a page must never wait on its decoration.
     A failure keeps the glyph tile, silently. */
  useEffect(() => {
    if (!petKind || !storageId) return;
    const scopeId = scope.scope.scope_id;
    const targets = rows.filter((row) => !row.local_present && row.remote_present
      && row.resource_id && !row.preview_png && !previewAttempts.current.has(row.row_id));
    if (targets.length === 0) return;
    let cancelled = false;
    void (async () => {
      for (const row of targets) {
        previewAttempts.current.add(row.row_id);
        try {
          const preview = await storageCatalogApi.getPetPreview(scopeId, storageId, row.resource_id!);
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
  }, [petKind, rows, scope.scope.scope_id, storageId]);

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
    if (!storageId) throw new Error("choose a storage first");
    await storageCatalogApi.linkScopeStorage(scope.scope.scope_id, storageId);
  }, [page, scope.scope.scope_id, storageId]);

  const openPush = async () => {
    if (!storageId) return;
    setBusy(true);
    setError(null);
    setFailures([]);
    try {
      await ensureLink();
      const plan = await storageCatalogApi.preparePush(scope.scope.scope_id, storageId);
      setPushPlan(plan);
      setPushSelected(new Set(plan.rows
        .filter((row) => row.row.selected_by_default)
        .map((row) => row.row.resource_id)));
      setPushAcks(new Set());
      setMode({ kind: "push" });
    } catch (reason) {
      setError(storageFailureMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const runPush = async () => {
    if (!pushPlan) return;
    const selected = [...pushSelected];
    setBusy(true);
    setError(null);
    setFailures([]);
    setProgress(`Pushing ${selected.length} ${selected.length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase()}…`);
    try {
      const outcome = await storageCatalogApi.applyPush(pushPlan.plan_id, selected, [...pushAcks]);
      setMode({ kind: "list" });
      setPushPlan(null);
      await load();
      const noun = selected.length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
      notify(outcome.published
        ? `Pushed ${selected.length} ${noun} to ${storageName ?? "storage"}.`
        : outcome.message);
    } catch (reason) {
      setMode({ kind: "list" });
      setPushPlan(null);
      await load();
      notify(`Push failed — nothing was published.`, "attention");
      setFailures([storageFailureMessage(reason)]);
    } finally {
      setProgress(null);
      setBusy(false);
    }
  };

  /** Opening the Pull review pins one plan over every stored resource. The
   * scope pairs with the storage first when it never has — the same "link for
   * review" step this page has always taken, configuration only, no provider
   * content changed. */
  const openPull = async () => {
    if (!storageId) return;
    setBusy(true);
    setError(null);
    setFailures([]);
    try {
      await ensureLink();
      const plan = await storageCatalogApi.preparePull(scope.scope.scope_id, storageId, null);
      setPullPlan(plan);
      setPullSelected(new Set(plan.items
        .filter((item) => item.item.default_action_id)
        .map((item) => item.item.resource_id)));
      setPullChoice(Object.fromEntries(plan.items
        .filter((item) => item.item.default_action_id)
        .map((item) => [item.item.resource_id, item.item.default_action_id as string])));
      setPullAcks(new Set());
      setMode({ kind: "pull" });
    } catch (reason) {
      setError(storageFailureMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const runPull = async () => {
    if (!pullPlan) return;
    const choices: Record<string, string> = {};
    for (const resourceId of pullSelected) {
      const actionId = pullChoice[resourceId];
      if (actionId) choices[resourceId] = actionId;
    }
    setBusy(true);
    setError(null);
    setFailures([]);
    setProgress(`Pulling ${Object.keys(choices).length} ${Object.keys(choices).length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase()}…`);
    try {
      const outcome = await storageCatalogApi.applyPull(pullPlan.plan_id, choices, [...pullAcks]);
      setMode({ kind: "list" });
      setPullPlan(null);
      await load();
      const failed = outcome.results
        .filter((result) => result.outcome === "failed" || result.outcome === "partial")
        .map((result) => `${result.display_name}: ${result.message ?? result.outcome}`);
      const applied = outcome.results
        .filter((result) => result.outcome === "installed"
          || result.outcome === "updated"
          || result.outcome === "preserved_and_updated").length;
      const noun = applied === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase();
      if (failed.length > 0) {
        notify(
          `Pulled ${applied} of ${Object.keys(choices).length} ${kind.plural.toLowerCase()} — ${failed.length} failed.`,
          "attention",
        );
        setFailures(failed);
        return;
      }
      notify(outcome.remote_accepted
        ? `Pulled ${applied} ${noun} from ${storageName ?? "storage"}.`
        : `Pulled ${applied} ${noun}. Pull did not advance the comparison base.`,
        outcome.remote_accepted ? "ok" : "attention");
    } catch (reason) {
      setMode({ kind: "list" });
      setPullPlan(null);
      await load();
      notify(`Pull failed — nothing was changed.`, "attention");
      setFailures([storageFailureMessage(reason)]);
    } finally {
      setProgress(null);
      setBusy(false);
    }
  };

  const unlinkStorage = async () => {
    if (!storageId || !pageLink) return;
    const approved = await confirm(
      `Unlink “${storageName}” from ${label}?\n\nPush and Pull stop for these ${kind.plural.toLowerCase()}. Content already stored there and local content are left untouched, and you can link again later.`,
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
      icon: storage?.kind === "s3" ? "cloud" : "drive",
      tone: "root",
      warning: !storageId,
      title: storage ? `${storage.name || "Storage"} · ${storage.detail}` : undefined,
      ariaLabel: `${storage?.name ?? "No storage"}. Storage menu`,
      menuLabel: "Storage",
      menuHeader: {
        label: storage?.name || "No storage linked",
        detail: storage?.detail,
        detailIsPath: storage?.detailIsPath,
        icon: storage?.kind === "s3" ? "cloud" : "drive",
      },
      menuBanner: storageId
        ? undefined
        : `Add a storage to compare these ${kind.plural.toLowerCase()} and turn on Push and Pull.`,
      menu: [
        {
          items: [
            {
              id: "settings",
              label: "Storage settings…",
              icon: "settings",
              disabled: !storageId || busy,
              onSelect: () => storageId && onOpenStorageSettings(storageId),
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
              icon: candidate.kind === "s3" ? ("cloud" as const) : ("drive" as const),
              disabled: busy,
              onSelect: () => {
                setChosenStorageId(candidate.id);
                setMode({ kind: "list" });
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
      icon: kind.icon,
      title: `${label}\n${scope.profile.configured_home}`,
      ariaLabel: `${label}. Catalog row menu`,
      menuLabel: "Catalog row",
      menuHeader: {
        label,
        detail: scope.profile.configured_home,
        detailIsPath: true,
        icon: kind.icon,
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
              label: catalogScopeLabel(candidate.scope, candidate.profile),
              detail: candidate.profile.configured_home,
              detailIsPath: true,
              icon: catalogKindMeta(candidate.scope.resource_kind).icon,
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
  ];

  const syncDisabled = busy || !storageId || loading;
  const syncDisabledReason = storageId
    ? undefined
    : `Add a storage to these ${kind.plural.toLowerCase()} before pushing or pulling`;
  const pushReviewWarnings = pushPlan?.rows
    .filter((row) => pushSelected.has(row.row.resource_id))
    .flatMap((row) => row.row.warnings) ?? [];
  const pushReviewUnacknowledged = pushReviewWarnings
    .some((warning) => !pushAcks.has(warning.digest));
  const pushReviewApplyDisabled = busy
    || pushSelected.size === 0
    || pushReviewUnacknowledged;
  const pushReviewDisabledReason = pushSelected.size === 0
    ? `Select at least one ${kind.label.toLowerCase()} to push`
    : pushReviewUnacknowledged
      ? "Acknowledge the selected warnings before pushing"
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
          {counts.attention > 0 && (
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
              className={`sync-path-sync${mode.kind === "pull" ? " active" : ""}`}
              aria-expanded={mode.kind === "pull"}
              disabled={syncDisabled || pullRows.length === 0}
              title={syncDisabledReason}
              onClick={() => void openPull()}
            >
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              {counts.pull > 0 && mode.kind !== "pull" && (
                <span className="sync-path-count" aria-hidden="true">{counts.pull}</span>
              )}
            </button>
            <button
              type="button"
              className={`sync-path-sync${mode.kind === "push" ? " active" : ""}`}
              aria-expanded={mode.kind === "push"}
              disabled={mode.kind === "push"
                ? pushReviewApplyDisabled
                : syncDisabled || pushRows.length === 0}
              title={mode.kind === "push" ? pushReviewDisabledReason : syncDisabledReason}
              onClick={() => void (mode.kind === "push" ? runPush() : openPush())}
            >
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              {mode.kind === "push"
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

  /* Keep work in flight and failures on screen. One zone takes the loudest
     severity; per-item failures keep one line each. Successes use the shell
     notice. */
  const pageNotices: NoticeItem[] = [];
  if (progress) {
    pageNotices.push({ key: "progress", severity: "info", message: progress });
  }
  if (error) {
    pageNotices.push({ key: "error", severity: "danger", message: error });
  }
  for (const failure of failures) {
    pageNotices.push({ key: `failure:${failure}`, severity: "danger", message: failure });
  }
  /* Nothing in this zone is standing content: the progress line is written
     while a Push or Pull runs, and both failure surfaces are written after
     one. Even the load failure arrives after the page has already drawn its
     "Loading…" state, so it is an event too — an assertive one, because the
     thing the user asked for did not happen. Work in flight only advises. */
  const noticeTiming: NoticeTiming = error || failures.length > 0
    ? { timing: "blocking" }
    : { timing: "progress" };
  const notices = (
    <NoticeZone items={pageNotices} subject="problems with this sync" {...noticeTiming} />
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

  // ---- Push review -------------------------------------------------------

  if (mode.kind === "push" && pushPlan) {
    const selectableRows = pushPlan.rows.filter((row) => row.row.selectable);
    const visibleRows = pushPlan.rows.filter((row) => (
      row.row.selectable || row.row.relation === "matching"
    ));
    const heldRows = pushPlan.rows.filter((row) => row.row.held_for_pull);
    return (
      <>
        {bar}
        <main className="sync-main sync-project-links-page sync-sync-review-page">
          <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="catalog-push-title">
            <header className="sync-inline-action-header sync-sync-review-header">
              <div className="sync-sync-review-title"><h2 id="catalog-push-title">Push review</h2></div>
              <SyncReviewSelectionBar
                count={pushSelected.size}
                total={selectableRows.length}
                busy={busy}
                recommendedTitle={`Use the recommended selection (${selectableRows.length})`}
                recommendedAriaLabel={`Use recommended selection (${selectableRows.length})`}
                recommendedDisabled={pushSelected.size === selectableRows.length}
                onRecommended={() => setPushSelected(new Set(selectableRows.map((row) => row.row.resource_id)))}
                clearDisabled={pushSelected.size === 0}
                onClear={() => setPushSelected(new Set())}
                close={{ ariaLabel: "Close Push review", onClose: () => setMode({ kind: "list" }) }}
              />
            </header>
            <div className="sync-inline-action-content sync-sync-review-content">
              <div className="sync-inline-action-scroll sync-sync-review-scroll">
                {notices}
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
                      title={`${heldRows.length} ${heldRows.length === 1 ? kind.label.toLowerCase() : kind.plural.toLowerCase()} held for Pull`}
                      action={(
                        <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => void openPull()}>
                          Open Pull review
                        </button>
                      )}
                    >
                      {`${storageName ?? "Storage"} has versions this Mac has not reviewed, so their local changes cannot push yet. Selected ${kind.plural.toLowerCase()} still push.`}
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
                        onOpenApp={openProfileApp}
                        onOpenTerminal={openProfileTerminal}
                        preview={petKind ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
                        onReveal={() => reveal(row)}
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
                            <AckChip
                              key={warning.digest}
                              lead={parts.lead}
                              body={parts.body}
                              checked={pushAcks.has(warning.digest)}
                              ariaLabel={`Reviewed: ${warning.message}`}
                              verbs={["Mark reviewed", "Reviewed"]}
                              onToggle={(checked) => setPushAcks((current) => {
                                const next = new Set(current);
                                if (checked) next.add(warning.digest);
                                else next.delete(warning.digest);
                                return next;
                              })}
                            />
                          );
                        })}
                      />
                    );
                  })}
                  {selectableRows.length === 0 && (
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
              {/* No live region: this reads off the plan's warnings and the
                  current selection, and it is on screen beside the button it
                  explains — the button's own disabled state is what a reader
                  arrives at. */}
              {pushReviewUnacknowledged && <span><small>Acknowledge the warnings above before pushing.</small></span>}
              {/* The count rides the verb — the number at the point of
                  commitment, the way a Git client's commit button carries it. */}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={pushReviewApplyDisabled}
                onClick={() => void runPush()}
              >
                <ThreadStatusIcon status="local-outstanding" size={16} />
                Push {pushSelected.size}
              </button>
            </footer>
          </section>
        </main>
      </>
    );
  }

  // ---- Pull review -------------------------------------------------------

  if (mode.kind === "pull" && pullPlan) {
    /* Rows already settled — in sync, or deliberately kept local — stay out
       of the review; the review is the work, not the inventory. */
    const reviewItems = pullPlan.items.filter((item) => item.item.disposition.kind !== "already_matching"
      && item.item.disposition.kind !== "kept_local");
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
    const blocked = reviewItems.some((item) => pullSelected.has(item.item.resource_id)
      && needsAck(item) && !pullAcks.has(item.item.resource_id));
    return (
      <>
        {bar}
        <main className="sync-main sync-project-links-page sync-sync-review-page">
          <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="catalog-pull-title">
            <header className="sync-inline-action-header sync-sync-review-header">
              <div className="sync-sync-review-title"><h2 id="catalog-pull-title">Pull review</h2></div>
              <SyncReviewSelectionBar
                count={pullSelected.size}
                total={reviewItems.length}
                busy={busy}
                recommendedTitle={`Use the recommended local actions (${reviewItems.length})`}
                recommendedAriaLabel={`Use recommended selection (${reviewItems.length})`}
                recommendedDisabled={busy}
                onRecommended={() => {
                  setPullSelected(new Set(pullPlan.items
                    .filter((item) => item.item.default_action_id)
                    .map((item) => item.item.resource_id)));
                  setPullChoice(Object.fromEntries(pullPlan.items
                    .filter((item) => item.item.default_action_id)
                    .map((item) => [item.item.resource_id, item.item.default_action_id as string])));
                  setPullAcks(new Set());
                }}
                clearDisabled={pullSelected.size === 0}
                onClear={() => setPullSelected(new Set())}
                close={{ ariaLabel: "Close Pull review", onClose: () => setMode({ kind: "list" }) }}
              />
            </header>
            <div className="sync-inline-action-content sync-sync-review-content">
              <div className="sync-inline-action-scroll sync-sync-review-scroll">
                {notices}
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
                      subject={`${kind.plural.toLowerCase()} claimed elsewhere`}
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
                  {reviewItems.map((item) => {
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
                        onOpenApp={openProfileApp}
                        onOpenTerminal={openProfileTerminal}
                        preview={petKind ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
                        onReveal={() => reveal(row)}
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
                          <AckChip
                            danger
                            lead="Overwrites this Mac’s copy"
                            body={`replaces the current ${item.item.display_name} here with the stored version`}
                            checked={pullAcks.has(resourceId)}
                            ariaLabel={`Approve overwriting ${item.item.display_name}`}
                            verbs={["Approve", "Approved"]}
                            onToggle={(checked) => setPullAcks((current) => {
                              const next = new Set(current);
                              if (checked) next.add(resourceId);
                              else next.delete(resourceId);
                              return next;
                            })}
                          />
                        )}
                      />
                    );
                  })}
                  {reviewItems.length === 0 && !busy && (
                    <div className="sync-pull-empty">
                      <Icon name="check-circle" size={16} />
                      <span>
                        <strong>Nothing to pull</strong>
                        <small>{`These ${kind.plural.toLowerCase()} already match ${storageName ?? "storage"}.`}</small>
                      </span>
                    </div>
                  )}
                </div>
              </div>
            </div>
            <footer className="sync-inline-action-footer sync-sync-review-footer">
              {/* It announces: choosing Overwrite from a row's menu is what
                  makes this appear, and it appears at the same moment the Pull
                  button goes disabled. A polite region does not announce what
                  is already present, so the case where the plan opens blocked
                  costs nothing. */}
              {blocked && <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>}
              <button
                type="button"
                className="btn btn-primary sync-pull-apply-button"
                disabled={busy || pullSelected.size === 0 || blocked}
                onClick={() => void runPull()}
              >
                <ThreadStatusIcon status="cloud-outstanding" size={16} />
                Pull {pullSelected.size}
              </button>
            </footer>
          </section>
        </main>
      </>
    );
  }

  // ---- the list ----------------------------------------------------------

  return (
    <>
      {bar}
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
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
        {loading && !page ? (
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
        ) : rows.length === 0 ? (
          /* Its own container: wrapping one sentence in a list card drew a
             second border around it. Nor does it name the home or the
             storage — the crumb bar above is already saying both. */
          <div className="sync-history-state">
            {`No ${kind.label.toLowerCase()} on this Mac, and none stored yet.`}
          </div>
        ) : (
          <section className="profile-links-section" aria-label={label}>
            {/* The project's own section heading: what the list is, in the
                same shape at the same weight. No count — every row already
                wears the same provider mark, so a mark-plus-number chip here
                read as one more row rather than a summary. */}
            <div className="sync-history-section-heading">
              <h2><Icon name={kind.icon} size={14} />{kind.plural}</h2>
            </div>
            <div className="sync-history-thread-list">
              {rows.map((row) => {
                const forkable = forkableKind && row.local_present && !!row.resource_id;
                const forkOpen = forkable && forkFor === row.resource_id;
                const kindPhrase = resourceKindPhrase(scope.scope.resource_kind, row.scope_label);
                return (
                  <ResourceCard
                    key={row.row_id}
                    row={row}
                    storageName={storageName}
                    storageRevision={storageRevision}
                    provider={scope.scope.provider}
                    onOpenApp={openProfileApp}
                    onOpenTerminal={openProfileTerminal}
                    preview={petKind ? row.preview_png ?? remotePreviews[row.row_id] ?? null : undefined}
                    onReveal={() => reveal(row)}
                    onFork={forkableKind ? () => {
                      setForkError(null);
                      setForkFor(forkOpen ? null : row.resource_id!);
                    } : undefined}
                    forkOpen={forkOpen}
                    forkTitle={`Fork ${row.display_name} — duplicate this ${kindPhrase} under a new name`}
                    footer={forkOpen && (
                      <ForkDrawer
                        displayName={row.display_name}
                        seed={forkSeed(row)}
                        maxLength={petKind ? 64 : 128}
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
              })}
            </div>
          </section>
        )}
      </main>
    </>
  );
}
