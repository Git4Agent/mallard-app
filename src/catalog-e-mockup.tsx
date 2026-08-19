import { StrictMode, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import {
  actionLabel,
  rowPullable,
  rowPushable,
  rowStamp,
  rowStatus,
  rowStatusLabel,
} from "./components/storage-catalog/model";
import type { CatalogPullAction, CatalogScopeRow } from "./components/storage-catalog/types";
import "./App.css";
import "./RetroTheme.css";

/**
 * Option E · without the chevron, applied to the whole Catalog page: the list,
 * the no-storage state, the Push review, the Pull review, and the empty
 * states. Drawn with the shipped stylesheet and the shipped components — the
 * path bar, the selection bar, the status glyph — and the review shells are
 * the project's own, class for class, because the Catalog reviews already
 * wear them. Not part of the app bundle: the build has one entry, and this
 * file is reachable only from the dev server.
 *
 * Two figures on every card are not on the row DTO yet: files and size on
 * disk. The backend already computes both during capture, and it already
 * computes the content digest the ⓘ popover shows, then drops all three —
 * the same gap `local_modified_at` had before it was added.
 */

const STORAGE_NAME = "Local storage 1";

/** Fixed local times, so the stamps read the way a real column does — a list
 *  of identical minutes is exactly what makes a mock look mocked. */
const at = (month: number, day: number, hour: number, minute: number) =>
  Math.floor(new Date(2026, month - 1, day, hour, minute).getTime() / 1_000);

/** The two freshly-edited rows are pinned to the clock instead, so they read
 *  "1h ago" whenever the page is opened rather than drifting into the future. */
const ago = (seconds: number) => Math.floor(Date.now() / 1_000) - seconds;

/** The indicator's colour class, keyed by badge — copied from the page so the
 *  specimens wear exactly the colours the app does. */
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

function row(overrides: Partial<CatalogScopeRow> & { row_id: string; display_name: string }): CatalogScopeRow {
  return {
    resource_id: `resource-${overrides.row_id}`,
    scope_label: "Codex home",
    state: null,
    linked: true,
    local_present: true,
    local_path: `/Users/hequ/.codex/skills/${overrides.display_name}`,
    local_modified_at: at(8, 15, 9, 12),
    remote_present: false,
    remote_revision: null,
    remote_updated_at: null,
    registration_id: `registration-${overrides.row_id}`,
    registration_revision: 1,
    link_id: `link-${overrides.row_id}`,
    link_revision: 1,
    repository_id: `repository-${overrides.row_id}`,
    other_storage_names: [],
    warnings: [],
    blocker: null,
    ...overrides,
  };
}

/** Every state a row can be in, so no mode gets to look good on the easy
 *  half of the list. The same cast the live harness uses. */
const ROWS: CatalogScopeRow[] = [
  row({
    row_id: "triage", display_name: "triage", state: "in_sync",
    remote_present: true, remote_revision: 4, other_storage_names: ["Vault"],
    local_modified_at: at(8, 15, 9, 12),
  }),
  row({
    row_id: "changelog-writer", display_name: "changelog-writer", state: "local_changed",
    remote_present: true, remote_revision: 2, local_modified_at: ago(5_400),
    warnings: [{ code: "secret_shaped", message: "contains a secret-shaped value", digest: "digest-secret" }],
  }),
  row({
    row_id: "pdf-filler", display_name: "pdf-filler", state: "new", linked: false,
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/pdf-filler",
    link_id: null, registration_id: null, repository_id: null,
    local_modified_at: at(8, 16, 16, 3),
  }),
  row({
    row_id: "release-notes", display_name: "release-notes", state: "storage_changed",
    remote_present: true, remote_revision: 6, local_modified_at: at(8, 13, 8, 27),
    remote_updated_at: at(8, 17, 22, 6),
  }),
  row({
    row_id: "sql-review", display_name: "sql-review", state: "local_missing",
    local_present: false, local_path: null, local_modified_at: null, linked: false,
    link_id: null, registration_id: null,
    remote_present: true, remote_revision: 3, remote_updated_at: at(8, 10, 14, 39),
  }),
  row({
    row_id: "api-designer", display_name: "api-designer", state: "diverged",
    scope_label: "Shared user", local_path: "/Users/hequ/.agents/skills/api-designer",
    remote_present: true, remote_revision: 5, local_modified_at: ago(3_600),
    warnings: [{ code: "executable", message: "contains an executable file", digest: "digest-exec" }],
  }),
  row({
    row_id: "deploy-checklist", display_name: "deploy-checklist", state: "unreviewed_remote",
    remote_present: true, remote_revision: 1, local_modified_at: at(8, 12, 19, 8),
  }),
  row({
    row_id: "legacy-import", display_name: "legacy-import", state: "blocked",
    scope_label: "Shared user", local_path: null, local_modified_at: null,
    link_id: null, registration_id: null, repository_id: null,
    blocker: "the skill directory is a symlink, so its contents can't be captured safely",
  }),
];

/** Twenty rows, because the list's real job is to be read down a column of
 *  forty and every option looks fine at three. */
const LONG_ROWS: CatalogScopeRow[] = [
  ...ROWS,
  ...["commit-message", "docstrings", "e2e-runner", "figma-export", "gh-triage", "i18n-audit",
    "lint-fixer", "mermaid", "openapi-diff", "perf-budget", "rust-clippy", "schema-drift"]
    .map((name, index) => row({
      row_id: name,
      display_name: name,
      state: (["in_sync", "in_sync", "local_changed", "in_sync", "storage_changed", "in_sync"] as const)[index % 6],
      remote_present: true,
      remote_revision: (index % 7) + 1,
      local_modified_at: at(8, 14 - index, 7 + (index % 11), (index * 7) % 60),
      scope_label: index % 5 === 0 ? "Shared user" : "Codex home",
      local_path: index % 5 === 0
        ? `/Users/hequ/.agents/skills/${name}`
        : `/Users/hequ/.codex/skills/${name}`,
    })),
];

/* ── the facts a card carries ────────────────────────────────────────────── */

const FACTS: Record<string, { files: number; bytes: number }> = {
  triage: { files: 6, bytes: 41_300 },
  "changelog-writer": { files: 3, bytes: 12_800 },
  "pdf-filler": { files: 11, bytes: 264_000 },
  "release-notes": { files: 4, bytes: 18_400 },
  "sql-review": { files: 0, bytes: 0 },
  "api-designer": { files: 9, bytes: 87_600 },
  "deploy-checklist": { files: 5, bytes: 22_100 },
  "legacy-import": { files: 0, bytes: 0 },
};

/** Deterministic stand-ins for the generated rows, so the long list carries
 *  figures of realistic width without pretending to be anyone's data. */
function facts(candidate: CatalogScopeRow) {
  const known = FACTS[candidate.row_id];
  if (known) return known;
  const seed = candidate.display_name.length;
  return { files: (seed % 9) + 2, bytes: 6_400 * ((seed % 13) + 2) };
}

const bytes = (value: number) => value >= 1_000_000
  ? `${(value / 1_048_576).toFixed(1)} MB`
  : `${Math.max(1, Math.round(value / 1024))} KB`;

/** The activity page's own stamp, to the character: "Aug 17, 2026 at 9:11 PM". */
function fullStamp(seconds: number): string {
  const when = new Date(seconds * 1_000);
  const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" }).format(when);
  const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(when);
  return `${date} at ${clock}`;
}

const stampSeconds = (candidate: CatalogScopeRow): number | null =>
  (candidate.local_present ? candidate.local_modified_at : null) ?? candidate.remote_updated_at ?? null;

const home = (path: string) => path.replace("/Users/hequ", "~");

/** A stable sha-shaped stand-in per name. The real digest is computed on
 *  every scan already — `row_for` derives the state from it — it is just not
 *  on the DTO. */
function digest(name: string): string {
  let seed = 7;
  for (const ch of name) seed = (seed * 31 + ch.charCodeAt(0)) % 0xffff;
  return Array.from({ length: 16 }, (_, i) => (((seed + 1) * (i + 3) * 2_654_435_761) % 0xffff)
    .toString(16).padStart(4, "0")).join("");
}

function badge(candidate: CatalogScopeRow, storageName: string | null) {
  const status = rowStatus(candidate);
  const label = [
    rowStatusLabel(candidate, storageName),
    candidate.other_storage_names.length > 0 && `Also linked to ${candidate.other_storage_names.join(", ")}.`,
  ].filter(Boolean).join(" ");
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
 *  and the sentence in a tooltip rather than on the row. */
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

/* ── the one clickable left on line two ──────────────────────────────────── */

/** The ⓘ, carrying what the session card's debug menu carries: the row's
 *  paths and digests, each a click to copy. Same wrapper, same popover. */
function InfoMenu({ candidate }: { candidate: CatalogScopeRow }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
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
      value: candidate.local_path
        ?? (candidate.local_present ? "In this provider home — not captured" : "Not on this Mac"),
    },
    { label: "Storage repository", value: candidate.repository_id ?? "Not stored yet" },
    {
      label: "Content digest",
      value: !candidate.blocker && (candidate.local_present || candidate.remote_present)
        ? `sha256:${digest(candidate.display_name)}`
        : "Nothing captured",
    },
  ];

  return (
    <div ref={rootRef} className="sync-history-debug-menu">
      <button
        type="button"
        className="btn btn-ghost sync-history-session-detail-action"
        title="Paths and digests"
        aria-label={`Paths and digests for ${candidate.display_name}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => { setCopied(null); setOpen(!open); }}
      >
        <Icon name="info" size={13} />
      </button>
      <div className="sync-history-debug-popover" role="dialog" aria-label="Paths and digests" hidden={!open}>
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
                void navigator.clipboard?.writeText(entry.value)
                  .then(() => setCopied(entry.label))
                  .catch(() => setCopied(null));
              }}
            >
              <span>{entry.label}</span>
              <code>{entry.value}</code>
              <small className={copied === entry.label ? "copied" : undefined}>
                {copied === entry.label ? "Copied" : "Copy"}
              </small>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ── the card ────────────────────────────────────────────────────────────── */

/** The verb a local action reduces to, for the row's pill. The whole sentence
 *  stays in the choice list; the row gets the word. Copied from the page. */
function actionVerb(label: string): string {
  if (label.startsWith("Keep local as")) return "Keep copy";
  if (label.startsWith("Overwrite")) return "Overwrite";
  if (label.startsWith("Install")) return "Install";
  return "Update";
}

/** What the Pull review hangs off a card: the chosen action, and the way to
 *  change it when the plan offers more than one. Copied from the page. */
interface RowAction {
  chosen: string;
  options: Array<{ id: string; label: string }>;
  expanded: boolean;
  destructive: boolean;
  onToggle: () => void;
  onChoose: (actionId: string) => void;
}

/**
 * Option E · without the chevron. Line one: badge, the name as the door, the
 * provider mark and Reveal in Finder. Line two: files and size, the rule, the
 * ⓘ, and the full stamp. Reviews put the checkbox where a session's review
 * puts it and hang the Pull action pill after the name.
 */
function ECard({ candidate, storageName, nameOpens, selectable, selected, onToggle, action, acks }: {
  candidate: CatalogScopeRow;
  storageName: string | null;
  nameOpens?: boolean;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  action?: RowAction;
  /** Acknowledgement lines, part of the card the way the project files
   *  review attaches "Reviewed this warning." to its own row. */
  acks?: ReactNode;
}) {
  const detail = facts(candidate);
  const seconds = stampSeconds(candidate);
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          {selectable && (
            <label className="sync-sync-row-choice" title={`${selected ? "Exclude" : "Include"} ${candidate.display_name}`}>
              <input
                type="checkbox"
                checked={!!selected}
                aria-label={`${selected ? "Exclude" : "Include"} ${candidate.display_name}`}
                onChange={() => onToggle?.()}
              />
            </label>
          )}
          {badge(candidate, storageName)}
          {nameOpens && candidate.registration_id ? (
            <button type="button" className="catalog-card-name" title="Storage links, fork, unlink">
              {candidate.display_name}
            </button>
          ) : (
            <strong title={candidate.display_name}>{candidate.display_name}</strong>
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
                {actionVerb(action.chosen)}
                <Icon name={action.expanded ? "chevron-up" : "chevron-down"} size={10} />
              </button>
              {action.expanded && (
                <div className="sync-path-menu end" role="menu" aria-label={`Local action for ${candidate.display_name}`}>
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
        {/* Line one's right-hand pair, read across from a session's: the
            provider mark that says whose home this is, and the one verb that
            opens the thing somewhere else — Finder rather than a terminal. */}
        <div className="sync-history-thread-actions">
          <span className="sync-history-launch-action sync-history-provider-inert" title="Codex skill">
            <ProviderMark provider="codex" displayName="Codex" />
          </span>
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!candidate.local_path}
            title={candidate.local_path ? `Reveal ${home(candidate.local_path)} in Finder` : "Not on this Mac"}
            aria-label={`Reveal ${candidate.display_name} in Finder`}
          >
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label="Skill details">
            <Metric
              icon="file"
              value={detail.files ? String(detail.files) : "—"}
              detail={detail.files ? `Files · ${detail.files}` : "Not on this Mac, so nothing was counted"}
            />
            <Metric
              icon="drive"
              value={detail.bytes ? bytes(detail.bytes) : "—"}
              detail={detail.bytes ? `On disk · ${bytes(detail.bytes)}` : "Not on this Mac, so nothing was measured"}
            />
            {candidate.warnings.length > 0
              ? <Metric
                  icon="alert-triangle"
                  value={String(candidate.warnings.length)}
                  muted
                  detail={candidate.warnings.map((warning) => warning.message).join("\n")}
                />
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          {/* The rule parts the two readouts from the one button. */}
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <InfoMenu candidate={candidate} />
          </div>
          <span
            className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end"
            data-tooltip={rowStamp(candidate)?.detail}
            tabIndex={0}
          >
            <time>{seconds == null ? "—" : fullStamp(seconds)}</time>
          </span>
        </div>
      </div>
      {acks}
    </article>
  );
}

/* ── the bar over every mode ─────────────────────────────────────────────── */

function tallies(rows: CatalogScopeRow[]) {
  return rows.reduce((totals, current) => {
    const status = rowStatus(current);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 });
}

function attentionSentence(rows: CatalogScopeRow[]) {
  return ([
    ["divergent", "changed on both sides"],
    ["pending", "waiting to sync"],
    ["unavailable", "can't be synced"],
  ] as Array<[string, string]>)
    .map(([status, phrase]) => {
      const count = rows.filter((current) => rowStatus(current) === status).length;
      return count === 0 ? null : `${count} ${count === 1 ? "skill" : "skills"} ${phrase}`;
    })
    .filter((phrase): phrase is string => phrase !== null)
    .join(", ");
}

/** The page's own path bar: storage crumb, scope crumb, and the three
 *  right-hand controls, with the active verb marked the way the page marks
 *  it. Menus are real; their items are inert in a mock. */
function CatalogBar({ rows, storageChosen, active, overflow }: {
  rows: CatalogScopeRow[];
  storageChosen: boolean;
  active?: "push" | "pull";
  overflow: PathBarMenuGroup[];
}) {
  const counts = tallies(rows);
  const syncDisabledReason = storageChosen
    ? undefined
    : "Add a storage to these skills before pushing or pulling";
  const crumbs: PathBarCrumb[] = [
    storageChosen ? {
      id: "storage",
      label: STORAGE_NAME,
      icon: "drive",
      tone: "root",
      title: `${STORAGE_NAME} · ~/Library/Mallard/local-1`,
      ariaLabel: `${STORAGE_NAME}. Storage menu`,
      menuLabel: "Storage",
      menuHeader: { label: STORAGE_NAME, detail: "~/Library/Mallard/local-1", detailIsPath: true, icon: "drive" },
      menu: [
        { items: [{ id: "settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] },
        {
          label: "Switch to",
          items: [{ id: "vault", label: "Vault", detail: "Bucket · mallard-vault", icon: "cloud", onSelect: () => undefined }],
        },
        { items: [{ id: "add-storage", label: "Add storage…", icon: "plus", onSelect: () => undefined }] },
      ],
    } : {
      id: "storage",
      label: "No storage",
      icon: "drive",
      tone: "root",
      warning: true,
      ariaLabel: "No storage. Storage menu",
      menuLabel: "Storage",
      menuHeader: { label: "No storage linked", icon: "drive" },
      menuBanner: "Add a storage to compare these skills and turn on Push and Pull.",
      menu: [
        { items: [{ id: "add-storage", label: "Add storage…", icon: "plus", onSelect: () => undefined }] },
      ],
    },
    {
      id: "scope",
      label: "Skills · Default Codex",
      icon: "file",
      title: "Skills · Default Codex\n/Users/hequ/.codex",
      ariaLabel: "Skills · Default Codex. Catalog row menu",
      menuLabel: "Catalog row",
      menuHeader: { label: "Skills · Default Codex", detail: "/Users/hequ/.codex", detailIsPath: true, icon: "file" },
      menu: [
        {
          items: [
            { id: "rescan", label: "Rescan this provider home", icon: "refresh", onSelect: () => undefined },
            {
              id: "remove",
              label: "Remove this Catalog row…",
              icon: "trash",
              danger: true,
              title: "Stop listing this home's resources; nothing on disk or in storage is deleted",
              onSelect: () => undefined,
            },
          ],
        },
        { items: [{ id: "add-scope", label: "Add Catalog resource…", icon: "plus", onSelect: () => undefined }] },
      ],
    },
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      overflow={overflow}
      overflowLabel="App actions"
      ariaLabel="Skills · Default Codex path"
      actions={(
        <>
          {counts.attention > 0 && (
            <span className="sync-path-attention warning" tabIndex={0} role="status" data-tooltip={attentionSentence(rows)}>
              <Icon name="alert-triangle" size={13} /><b>{counts.attention}</b>
            </span>
          )}
          <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button
              type="button"
              className={`sync-path-sync${active === "pull" ? " active" : ""}`}
              aria-expanded={active === "pull"}
              disabled={!storageChosen}
              title={syncDisabledReason}
            >
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              {counts.pull > 0 && active !== "pull" && (
                <span className="sync-path-count" aria-hidden="true">{counts.pull}</span>
              )}
            </button>
            <button
              type="button"
              className={`sync-path-sync${active === "push" ? " active" : ""}`}
              aria-expanded={active === "push"}
              disabled={!storageChosen}
              title={syncDisabledReason}
            >
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              {counts.push > 0 && active !== "push" && (
                <span className="sync-path-count" aria-hidden="true">{counts.push}</span>
              )}
            </button>
          </div>
        </>
      )}
    />
  );
}

/* ── the modes ───────────────────────────────────────────────────────────── */

function ListStage({ rows, overflow }: { rows: CatalogScopeRow[]; overflow: PathBarMenuGroup[] }) {
  return (
    <div className="sync-workspace">
      <CatalogBar rows={rows} storageChosen overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Skills">
          <div className="sync-history-section-heading">
            <h2><Icon name="file" size={14} />Skills</h2>
            <span className="sync-history-heading-count"><Icon name="file" size={12} />{rows.length}</span>
          </div>
          <div className="sync-history-thread-list">
            {rows.map((candidate) => (
              <ECard key={candidate.row_id} candidate={candidate} storageName={STORAGE_NAME} nameOpens />
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}

function NoStorageStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  const rows = ROWS
    .filter((candidate) => candidate.local_present || candidate.blocker)
    .map((candidate) => ({
      ...candidate,
      state: candidate.blocker ? candidate.state : null,
      linked: false,
    }));
  return (
    <div className="sync-workspace">
      <CatalogBar rows={rows} storageChosen={false} overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Skills">
          <div className="sync-history-section-heading">
            <h2><Icon name="file" size={14} />Skills</h2>
            <span className="sync-history-heading-count"><Icon name="file" size={12} />{rows.length}</span>
          </div>
          <div className="sync-history-thread-list">
            {rows.map((candidate) => (
              <ECard key={candidate.row_id} candidate={candidate} storageName={null} nameOpens />
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}

function PushStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  const pushRows = ROWS.filter(rowPushable);
  const held = ROWS.filter((candidate) => candidate.local_present
    && (candidate.state === "diverged" || candidate.state === "unreviewed_remote"));
  const [selected, setSelected] = useState<Set<string>>(() => new Set(pushRows.map((candidate) => candidate.row_id)));
  const [acks, setAcks] = useState<Set<string>>(new Set());
  const warnings = pushRows
    .filter((candidate) => selected.has(candidate.row_id))
    .flatMap((candidate) => candidate.warnings);
  const unacknowledged = warnings.some((warning) => !acks.has(warning.digest));
  const toggle = (rowId: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(rowId)) next.delete(rowId);
    else next.add(rowId);
    return next;
  });
  return (
    <div className="sync-workspace">
      <CatalogBar rows={ROWS} storageChosen active="push" overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        <section className="sync-inline-action-review sync-inline-push-review sync-sync-review-workspace" aria-labelledby="mock-push-title">
          <header className="sync-inline-action-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="mock-push-title">Push review</h2></div>
            <SyncReviewSelectionBar
              count={selected.size}
              total={pushRows.length}
              busy={false}
              recommendedTitle={`Use the recommended selection (${pushRows.length})`}
              recommendedAriaLabel={`Use recommended selection (${pushRows.length})`}
              recommendedDisabled={selected.size === pushRows.length}
              onRecommended={() => setSelected(new Set(pushRows.map((candidate) => candidate.row_id)))}
              clearDisabled={selected.size === 0}
              onClear={() => setSelected(new Set())}
              close={{ ariaLabel: "Close Push review", onClose: () => undefined }}
            />
          </header>
          <div className="sync-inline-action-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              {held.length > 0 && (
                <div className="sync-review-banners">
                  <div className="sync-callout sync-sync-review-pull-holds" role="status">
                    <ThreadStatusIcon status="divergent" size={16} />
                    <span>
                      <strong>{`${held.length} skills held for Pull`}</strong>
                      {`${STORAGE_NAME} has newer versions of them, so their local changes cannot push yet. Selected skills still push.`}
                    </span>
                    <button type="button" className="btn btn-ghost">Open Pull review</button>
                  </div>
                </div>
              )}
              <div className="sync-history-thread-list">
                {pushRows.map((candidate) => (
                  <ECard
                    key={candidate.row_id}
                    candidate={candidate}
                    storageName={STORAGE_NAME}
                    selectable
                    selected={selected.has(candidate.row_id)}
                    onToggle={() => toggle(candidate.row_id)}
                    acks={selected.has(candidate.row_id) && candidate.warnings.map((warning) => (
                      <label key={warning.digest} className="catalog-card-ack">
                        <span className="sync-sync-row-choice">
                          <input
                            type="checkbox"
                            checked={acks.has(warning.digest)}
                            aria-label={`Reviewed: ${warning.message}`}
                            onChange={(event) => setAcks((current) => {
                              const next = new Set(current);
                              if (event.target.checked) next.add(warning.digest);
                              else next.delete(warning.digest);
                              return next;
                            })}
                          />
                        </span>
                        <Icon name="alert-triangle" size={12} />
                        <span>{`Reviewed this warning — ${warning.message}.`}</span>
                      </label>
                    ))}
                  />
                ))}
              </div>
            </div>
          </div>
          <footer className="sync-inline-action-footer sync-sync-review-footer">
            {unacknowledged && <span role="status"><small>Acknowledge the warnings above before pushing.</small></span>}
            <button
              type="button"
              className="btn btn-primary sync-pull-apply-button"
              disabled={selected.size === 0 || unacknowledged}
            >
              <ThreadStatusIcon status="local-outstanding" size={16} />
              Push
            </button>
          </footer>
        </section>
      </main>
    </div>
  );
}

/* The Pull plans the backend would return, one per pullable fixture row, with
   the same action vocabulary the live harness stubs. */
const PULL_ACTIONS: Record<string, CatalogPullAction[]> = {
  "release-notes": [
    { action_id: "a1", resource_id: "resource-release-notes", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a2", resource_id: "resource-release-notes", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "release-notes-local-copy", copy_resource_id: "resource-copy", copy_target_path: "/Users/hequ/.codex/skills/release-notes-local-copy", transformed_content_sha256: "c".repeat(64), original_content_sha256: "d".repeat(64), storage_content_sha256: "e".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
    { action_id: "a3", resource_id: "resource-release-notes", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "b".repeat(64) },
  ],
  "sql-review": [
    { action_id: "b1", resource_id: "resource-sql-review", kind: { kind: "install_skill" }, requires_destructive_approval: false, incoming_content_sha256: "f".repeat(64) },
  ],
  "api-designer": [
    { action_id: "c1", resource_id: "resource-api-designer", kind: { kind: "preserve_skill_as_copy_and_overwrite", copy: { copy_name: "api-designer-local-copy", copy_resource_id: "resource-copy-2", copy_target_path: "/Users/hequ/.agents/skills/api-designer-local-copy", transformed_content_sha256: "1".repeat(64), original_content_sha256: "2".repeat(64), storage_content_sha256: "3".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "4".repeat(64) },
    { action_id: "c2", resource_id: "resource-api-designer", kind: { kind: "overwrite_skill_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "4".repeat(64) },
  ],
  "deploy-checklist": [
    { action_id: "d1", resource_id: "resource-deploy-checklist", kind: { kind: "update_skill" }, requires_destructive_approval: false, incoming_content_sha256: "5".repeat(64) },
  ],
};

function PullStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  const entries = ROWS.filter(rowPullable);
  const [selected, setSelected] = useState<Set<string>>(() => new Set(entries.map((candidate) => candidate.row_id)));
  const [choice, setChoice] = useState<Record<string, string>>(() => Object.fromEntries(
    entries.map((candidate) => [candidate.row_id, PULL_ACTIONS[candidate.row_id][0].action_id]),
  ));
  const [acks, setAcks] = useState<Set<string>>(new Set());
  const [openChoices, setOpenChoices] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  /* Outside-click and Escape dismiss the action menu, the same contract the
     path bar's own menus keep. */
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

  const chosenAction = (rowId: string): CatalogPullAction | undefined =>
    PULL_ACTIONS[rowId].find((candidate) => candidate.action_id === choice[rowId]);
  const needsAck = (rowId: string) => chosenAction(rowId)?.requires_destructive_approval ?? false;
  const blocked = entries.some((candidate) => selected.has(candidate.row_id)
    && needsAck(candidate.row_id) && !acks.has(candidate.row_id));
  const toggle = (rowId: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(rowId)) next.delete(rowId);
    else next.add(rowId);
    return next;
  });

  return (
    <div className="sync-workspace">
      <CatalogBar rows={ROWS} storageChosen active="pull" overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="mock-pull-title">
          <header className="sync-inline-action-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="mock-pull-title">Pull review</h2></div>
            <SyncReviewSelectionBar
              count={selected.size}
              total={entries.length}
              busy={false}
              recommendedTitle={`Use the recommended local actions (${entries.length})`}
              recommendedAriaLabel={`Use recommended selection (${entries.length})`}
              recommendedDisabled={false}
              onRecommended={() => {
                setSelected(new Set(entries.map((candidate) => candidate.row_id)));
                setChoice(Object.fromEntries(entries.map((candidate) => [candidate.row_id, PULL_ACTIONS[candidate.row_id][0].action_id])));
                setAcks(new Set());
              }}
              clearDisabled={selected.size === 0}
              onClear={() => setSelected(new Set())}
              close={{ ariaLabel: "Close Pull review", onClose: () => undefined }}
            />
          </header>
          <div className="sync-inline-action-content sync-sync-review-content">
            <div className="sync-inline-action-scroll sync-sync-review-scroll">
              <div className="sync-review-banners">
                <div className="sync-callout warning" role="status">
                  <Icon name="alert-triangle" size={16} />
                  <span>
                    <strong>api-designer is claimed by Vault</strong>
                    A different version of this local target is pinned there. Preserve a copy, acknowledge overwrite, or Skip.
                  </span>
                </div>
              </div>
              <div className="sync-history-thread-list" ref={listRef}>
                {entries.map((candidate) => {
                  const included = selected.has(candidate.row_id);
                  const expanded = openChoices === candidate.row_id;
                  const action = chosenAction(candidate.row_id);
                  const chosen = action ? actionLabel(action) : "Skip";
                  return (
                      <ECard
                        key={candidate.row_id}
                        candidate={candidate}
                        storageName={STORAGE_NAME}
                        selectable
                        selected={included}
                        onToggle={() => toggle(candidate.row_id)}
                        action={included ? {
                          chosen,
                          options: PULL_ACTIONS[candidate.row_id].map((option) => ({
                            id: option.action_id,
                            label: actionLabel(option),
                          })),
                          expanded,
                          destructive: needsAck(candidate.row_id),
                          onToggle: () => setOpenChoices(expanded ? null : candidate.row_id),
                          onChoose: (actionId) => {
                            setChoice((current) => ({ ...current, [candidate.row_id]: actionId }));
                            setAcks((current) => {
                              const next = new Set(current);
                              next.delete(candidate.row_id);
                              return next;
                            });
                            setOpenChoices(null);
                          },
                        } : undefined}
                        acks={included && needsAck(candidate.row_id) && (
                          /* Part of the card it belongs to, not the footer:
                             the footer says why the button is off, this says
                             which row turned it off. */
                          <label className="catalog-card-ack danger">
                            <span className="sync-sync-row-choice">
                              <input
                                type="checkbox"
                                checked={acks.has(candidate.row_id)}
                                aria-label={`Approve overwriting ${candidate.display_name}`}
                                onChange={(event) => setAcks((current) => {
                                  const next = new Set(current);
                                  if (event.target.checked) next.add(candidate.row_id);
                                  else next.delete(candidate.row_id);
                                  return next;
                                })}
                              />
                            </span>
                            <Icon name="alert-triangle" size={12} />
                            <span>{`I understand this replaces the current ${candidate.display_name} on this Mac.`}</span>
                          </label>
                        )}
                      />
                  );
                })}
              </div>
            </div>
          </div>
          <footer className="sync-inline-action-footer sync-sync-review-footer">
            {blocked && <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>}
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
    </div>
  );
}

function StatesStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  return (
    <div className="sync-workspace">
      <CatalogBar rows={[]} storageChosen overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <div className="sync-history-state">Loading Skills · Default Codex…</div>
        <div className="sync-history-state">No skill on this Mac, and none stored yet.</div>
        <div className="sync-pull-empty">
          <Icon name="check-circle" size={16} />
          <span>
            <strong>Nothing to push</strong>
            <small>{`This Mac already matches ${STORAGE_NAME}.`}</small>
          </span>
        </div>
        <div className="sync-pull-empty">
          <Icon name="check-circle" size={16} />
          <span>
            <strong>Nothing to pull</strong>
            <small>{`These skills already match ${STORAGE_NAME}.`}</small>
          </span>
        </div>
      </main>
    </div>
  );
}

/* ── the page around them ────────────────────────────────────────────────── */

function Panel({ title, standard, note, children }: {
  title: string;
  standard: string;
  note: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p className="mockup-standard">{standard}</p>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(false);
  const [long, setLong] = useState(false);
  const rows = long ? LONG_ROWS : ROWS;
  const toggleTheme = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
  };
  const overflow: PathBarMenuGroup[] = [{
    items: [
      { id: "theme", label: dark ? "Light theme" : "Dark theme", icon: dark ? "sun" : "moon", onSelect: toggleTheme },
    ],
  }];
  return (
    <div className="app">
      <style>{MOCK_CSS}</style>
      <style>{SHELL_CSS}</style>
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Catalog · Option E, the whole page</h1>
            <p>
              The card you chose — files and size, the rule, one <code>ⓘ</code>, the full stamp,
              and the name as the door — carried through every mode the page has. The review
              shells are the project&rsquo;s own Push and Pull reviews, class for class: the same
              header and selection bar, the banner zone above the list, the checkbox inside the
              title line, one primary in the footer, and no tab strip because one kind of
              resource is not a choice of steps. Everything is interactive: tick rows, open the
              <code> ⓘ</code>, change a Pull action, and watch the footer gate. Files, size and
              the digest are computed on every scan today and dropped before the DTO — the same
              gap <code>local_modified_at</code> had before it was added.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn" onClick={() => setLong(!long)}>
              <Icon name="activity" size={13} /> {long ? "8 rows" : "20 rows"}
            </button>
            <button type="button" className="btn" onClick={toggleTheme}>
              <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
            </button>
          </div>
        </header>

        <Panel
          title="The list"
          standard="Main mode, a storage in the bar"
          note={<>The chosen card, under the page&rsquo;s real bar. Five of the eight names are
            doors — <code>pdf-filler</code>, <code>sql-review</code> and <code>legacy-import</code>
            have no registration, so no page behind them, and stay plain text. The <code>ⓘ</code>
            opens the card&rsquo;s paths and digests, each a click to copy. <code>sql-review</code>
            is only in storage and <code>legacy-import</code> is blocked, so their line two reads
            as two em-dashes and their Reveal button is off.</>}
        >
          <ListStage rows={rows} overflow={overflow} />
        </Panel>

        <Panel
          title="No storage chosen"
          standard="The same list with nothing to compare against"
          note={<>Every badge goes hollow — nothing can be claimed without a comparison — and
            Push and Pull are off with the reason in their tooltip. The cards keep their local
            facts: files, size, the stamp, the paths behind the <code>ⓘ</code>. Rows that exist
            only in a storage are gone, because no storage is telling us about them.</>}
        >
          <NoStorageStage overflow={overflow} />
        </Panel>

        <Panel
          title="Push review"
          standard="The project's Push review, carrying these cards"
          note={<>The project review shell: title and selection bar in the header, the held-for-
            Pull banner above the list — derived from the plan, never from the selection — and
            one primary in the footer. The card takes the checkbox where a session&rsquo;s review
            puts it, inside the title line ahead of the badge. <code>changelog-writer</code>
            carries a warning, so its acknowledgement sits inside its own card as a row-sized
            callout — the banner zone's warning tint, the app's own checkbox — and holds the
            Push button until ticked — untick it to watch the footer gate.</>}
        >
          <PushStage overflow={overflow} />
        </Panel>

        <Panel
          title="Pull review"
          standard="The project's Pull review, carrying these cards"
          note={<>Same shell, opposite direction. Each included card states its local action as
            a pill after the name — the word, with the whole sentence in the menu behind it.
            <code> release-notes</code> offers three actions; switch <code>api-designer</code> to
            Overwrite and its acknowledgement joins the card as a danger-tinted callout and holds
            the Pull button.
            The claimed-by banner sits in the banner zone, above the list, where the project
            reviews put plan-derived facts.</>}
        >
          <PullStage overflow={overflow} />
        </Panel>

        <Panel
          title="The states"
          standard="Loading, empty, and nothing-to-move"
          note={<>The list&rsquo;s loading and empty lines are the page&rsquo;s own. The two
            review empties adopt the project&rsquo;s <code>Nothing to pull</code> treatment —
            a check glyph and a sentence — where the page today prints a bare line; matching the
            project reviews means matching their quiet states too.</>}
        >
          <StatesStage overflow={overflow} />
        </Panel>
      </div>
    </div>
  );
}

/* ── proposed CSS ────────────────────────────────────────────────────────── */

const MOCK_CSS = `
/* Line one's name behaves like a session's title: it takes the slack so the
   provider mark and the launch verb stay pinned right. */
.catalog-card .sync-history-thread-title > strong {
  flex: 0 1 auto;
}

/* The name as the door. Same treatment the shipped list's row name has: no
   colour of its own, an underline only under the pointer. */
.catalog-card-name {
  min-width: 0;
  flex: 0 1 auto;
  overflow: hidden;
  padding: 0;
  border: 0;
  background: none;
  color: var(--text-0);
  font: inherit;
  font-size: var(--type-md);
  font-weight: var(--weight-semibold);
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}

.catalog-card-name:hover { text-decoration: underline; }

/* The acknowledgement is a callout, row-sized: the tint and 9px radius the
   banner zone wears — warning for a scan finding, danger for an overwrite —
   and the app's own monochrome checkbox, the one the title line already
   uses, instead of the platform's. It hugs its sentence at line two's x, so
   it reads as furniture of the card rather than a form under it. */
.catalog-card-ack {
  width: fit-content;
  min-height: 28px;
  display: flex;
  align-items: center;
  gap: 7px;
  margin-left: 26px;
  padding: 2px 11px 2px 2px;
  border: 1px solid color-mix(in srgb, var(--yellow) 22%, transparent);
  border-radius: 9px;
  background: var(--warning-soft);
  color: var(--warning-text);
  font-size: var(--type-2xs);
  cursor: pointer;
}

.catalog-card-ack.danger {
  border-color: color-mix(in srgb, var(--red) 24%, transparent);
  background: var(--danger-soft);
  color: var(--danger-text);
}

.catalog-card-ack > .sync-sync-row-choice {
  width: 24px;
  min-width: 24px;
  min-height: 24px;
}

.catalog-card-ack > svg { flex: 0 0 auto; }
`;

const SHELL_CSS = `
/* The app is a fixed shell — html, body, #root and .app are all height-locked
   and overflow: hidden, because in Tauri only the inner panes scroll. A
   mockup is a document, so it has to hand the wheel back to the page. */
html, body, #root { height: auto; min-height: 100%; overflow: visible; }
.app { height: auto; min-height: 100vh; overflow: visible; }
.mockup-stage .sync-workspace { height: auto; }
.mockup-stage .sync-main { height: auto; overflow: visible; }

/* width:100% is load-bearing — .app is a column flex container, and an auto
   cross-axis margin without it lets the shell shrink to its content. */
.mockup-shell { width: 100%; max-width: 1320px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 78ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-controls { display: inline-flex; gap: 8px; flex: 0 0 auto; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > .mockup-standard { margin: 0 0 8px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); letter-spacing: .04em; text-transform: uppercase; }
.mockup-panel > p { max-width: 84ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }

/* overflow stays visible so the ⓘ popover and the action menu can hang past
   the stage edge; the bar's own corners round with the frame. */
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--bg-0); overflow: visible; }
.mockup-stage > .sync-workspace > .sync-pathbar { border-radius: 6px 6px 0 0; }
.mockup-stage .sync-main { border-radius: 0 0 6px 6px; }
body { background: var(--app-shell); }
`;

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
