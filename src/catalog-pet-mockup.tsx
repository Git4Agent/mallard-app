import { StrictMode, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon from "./components/sync-review/ThreadStatusIcon";
import { actionLabel, rowStamp, rowStatus, rowStatusLabel } from "./components/storage-catalog/model";
import type { CatalogPullAction, CatalogScopeRow } from "./components/storage-catalog/types";
import { PET_THUMBS } from "./catalog-pet-mockup-thumbs";
import "./App.css";
import "./RetroTheme.css";

/**
 * Pet previews on the Catalog page. The shipped Option E card, with one
 * addition for pet scopes: the pet's own idle frame leading the card, the way
 * every two-line list with artwork leads with it — iOS table cells, Finder,
 * Slack file rows. The previews here are the real first cells of the real
 * spritesheets in ~/.codex/pets, cropped 192x208 and scaled to 2x the 44x48
 * box — exactly the artifact the backend would derive once per content digest
 * and cache. Not part of the app bundle: the build has one entry, and this
 * file is reachable only from the dev server.
 */

const STORAGE_NAME = "Local storage 1";

const at = (month: number, day: number, hour: number, minute: number) =>
  Math.floor(new Date(2026, month - 1, day, hour, minute).getTime() / 1_000);

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

/** The description each real pet.json carries; the preview's tooltip. */
const ABOUT: Record<string, string> = {
  maple: "A compact brown-and-white border collie pet for Codex.",
  dahuang: "一只性格乖巧、黄身白脸、身形修长的中华田园犬。",
  patch: "A calm, observant green turtle with natural anatomy and patient, deliberate movement.",
  tracebill: "A calm, sharp-eyed debugging duck built for fast, evidence-backed OCI and Exascale work.",
  tracebit: "A compact evidence-first debugging bot for OCI and Exascale work.",
};

function row(overrides: Partial<CatalogScopeRow> & { row_id: string; display_name: string }): CatalogScopeRow {
  return {
    resource_id: `resource-${overrides.row_id}`,
    scope_label: "v2",
    state: null,
    linked: true,
    local_present: true,
    local_path: `/Users/hequ/.codex/pets/${overrides.row_id}`,
    local_modified_at: at(8, 15, 9, 12),
    local_files: 1,
    local_bytes: 1_784_464,
    remote_present: false,
    remote_revision: null,
    remote_updated_at: null,
    content_sha256: "8c1e".repeat(16),
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

/** Pets v2 · Default Codex — the three real v2 pets plus the two states a
 *  preview cannot come from disk for: stored-only, and blocked. */
const V2_ROWS: CatalogScopeRow[] = [
  row({
    row_id: "maple", display_name: "Maple", state: "in_sync",
    remote_present: true, remote_revision: 3, local_bytes: 2_633_494,
  }),
  row({
    row_id: "dahuang", display_name: "大黄", state: "local_changed",
    remote_present: true, remote_revision: 5, local_modified_at: at(8, 18, 8, 41),
  }),
  row({
    row_id: "patch", display_name: "Patch", state: "new", linked: false,
    link_id: null, repository_id: null, local_bytes: 1_686_990,
    local_modified_at: at(8, 16, 19, 25),
  }),
  row({
    row_id: "biscuit", display_name: "Biscuit", state: "local_missing",
    local_present: false, local_path: null, local_modified_at: null, linked: false,
    local_files: null, local_bytes: null,
    link_id: null, registration_id: null,
    remote_present: true, remote_revision: 2, remote_updated_at: at(8, 11, 15, 2),
  }),
  row({
    row_id: "scout", display_name: "Scout", state: "blocked",
    local_path: null, local_modified_at: null,
    local_files: null, local_bytes: null, content_sha256: null,
    link_id: null, registration_id: null, repository_id: null,
    blocker: "the pet package is a symlink, so its contents can't be captured safely",
  }),
];

/** Pets v1 · Default Codex — the two real v1 ducks, in the Pull review. */
const V1_ROWS: CatalogScopeRow[] = [
  row({
    row_id: "tracebill", display_name: "Tracebill", state: "storage_changed",
    scope_label: "v1", local_path: "/Users/hequ/.codex/pets/tracebill",
    remote_present: true, remote_revision: 7, local_bytes: 2_037_914,
    remote_updated_at: at(8, 17, 22, 6),
  }),
  row({
    row_id: "tracebit", display_name: "Tracebit", state: "diverged",
    scope_label: "v1", local_path: "/Users/hequ/.codex/pets/tracebit",
    remote_present: true, remote_revision: 4, local_bytes: 66_396,
    local_modified_at: at(8, 18, 7, 58),
  }),
];

const PULL_ACTIONS: Record<string, CatalogPullAction[]> = {
  tracebill: [
    { action_id: "a1", resource_id: "resource-tracebill", kind: { kind: "update_pet" }, requires_destructive_approval: false, incoming_content_sha256: "b".repeat(64) },
  ],
  tracebit: [
    { action_id: "c1", resource_id: "resource-tracebit", kind: { kind: "preserve_pet_as_copy_and_overwrite", copy: { copy_name: "tracebit-local-copy", copy_resource_id: "resource-copy", copy_target_path: "/Users/hequ/.codex/pets/tracebit-local-copy", transformed_content_sha256: "1".repeat(64), original_content_sha256: "2".repeat(64), storage_content_sha256: "3".repeat(64), reusable_witness: false } }, requires_destructive_approval: false, incoming_content_sha256: "4".repeat(64) },
    { action_id: "c2", resource_id: "resource-tracebit", kind: { kind: "overwrite_pet_with_storage" }, requires_destructive_approval: true, incoming_content_sha256: "4".repeat(64) },
  ],
};

/* ── the pieces the shipped card already has ─────────────────────────────── */

const formatBytes = (value: number) => value >= 1_000_000
  ? `${(value / 1_048_576).toFixed(1)} MB`
  : `${value === 0 ? 0 : Math.max(1, Math.round(value / 1024))} KB`;

function fullStamp(seconds: number): string {
  const when = new Date(seconds * 1_000);
  const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" }).format(when);
  const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(when);
  return `${date} at ${clock}`;
}

const stampSeconds = (candidate: CatalogScopeRow): number | null =>
  (candidate.local_present ? candidate.local_modified_at : null) ?? candidate.remote_updated_at ?? null;

function badge(candidate: CatalogScopeRow, storageName: string | null) {
  const status = rowStatus(candidate);
  const label = rowStatusLabel(candidate, storageName);
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

function Metric({ icon, value, detail }: { icon: IconName; value: string; detail: string }) {
  return (
    <span className="sync-history-thread-metric" data-tooltip={detail} aria-label={detail} tabIndex={0}>
      <Icon name={icon} size={12} /><b>{value}</b>
    </span>
  );
}

function InfoMenu({ candidate }: { candidate: CatalogScopeRow }) {
  const [open, setOpen] = useState(false);
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
  const values = [
    {
      label: "Local path",
      value: candidate.local_path
        ?? (candidate.local_present ? "In this provider home — not captured" : "Not on this Mac"),
    },
    { label: "Storage repository", value: candidate.repository_id ?? "Not stored yet" },
    {
      label: "Content digest",
      value: candidate.content_sha256 ? `sha256:${candidate.content_sha256}` : "Nothing captured",
    },
  ];
  return (
    <div ref={rootRef} className="sync-history-debug-menu">
      <button
        type="button"
        className="btn btn-ghost sync-history-session-detail-action"
        title="Paths and digests"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
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
            <button key={entry.label} type="button" className="sync-history-debug-copy">
              <span>{entry.label}</span>
              <code>{entry.value}</code>
              <small>Copy</small>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function actionVerb(label: string): string {
  if (label.startsWith("Keep local as")) return "Keep copy";
  if (label.startsWith("Overwrite")) return "Overwrite";
  if (label.startsWith("Install")) return "Install";
  return "Update";
}

interface RowAction {
  chosen: string;
  options: Array<{ id: string; label: string }>;
  expanded: boolean;
  destructive: boolean;
  onToggle: () => void;
  onChoose: (actionId: string) => void;
}

/* ── the one addition: the preview ───────────────────────────────────────── */

/** The pet's idle frame in a fixed 44x48 tile, letterboxed on the card's own
 *  paper. A row with nothing local to draw gets the kind glyph instead —
 *  never a broken image, and never a missing column. */
function Preview({ candidate }: { candidate: CatalogScopeRow }) {
  const thumb = candidate.local_present && !candidate.blocker ? PET_THUMBS[candidate.row_id] : undefined;
  const about = ABOUT[candidate.row_id];
  return (
    <span className="catalog-pet-preview" data-tooltip={about} aria-hidden={about ? undefined : true} tabIndex={about ? 0 : undefined}>
      {thumb
        ? <img src={thumb} alt={`${candidate.display_name} preview`} />
        : <Icon name="openai" size={16} />}
    </span>
  );
}

/** The shipped Option E card with the preview joined into its own grid: the
 *  retro card is `"title actions" / "meta meta"`, and the pet variant opens a
 *  leading column spanning both rows — the iOS/Finder two-line-row grammar,
 *  said in the card's existing template. */
function PetCard({ candidate, storageName, nameOpens, selectable, selected, onToggle, action, acks }: {
  candidate: CatalogScopeRow;
  storageName: string | null;
  nameOpens?: boolean;
  selectable?: boolean;
  selected?: boolean;
  onToggle?: () => void;
  action?: RowAction;
  acks?: ReactNode;
}) {
  const seconds = stampSeconds(candidate);
  return (
    <article className="sync-history-thread-card catalog-card catalog-card-pet">
      <Preview candidate={candidate} />
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
          <div className="sync-history-thread-actions">
            <span className="sync-history-launch-action sync-history-provider-inert" title="Codex pet">
              <ProviderMark provider="codex" displayName="Codex" />
            </span>
            <button
              type="button"
              className="btn btn-ghost sync-history-launch-action"
              disabled={!candidate.local_path}
              title={candidate.local_path ? `Reveal ${candidate.local_path} in Finder` : "Not on this Mac"}
              aria-label={`Reveal ${candidate.display_name} in Finder`}
            >
              <Icon name="folder" size={14} />
            </button>
          </div>
        </div>
        <div className="sync-history-session-details">
          <div className="sync-history-session-summary">
            <div className="sync-history-thread-meta" aria-label={`${candidate.display_name} details`}>
              <Metric
                icon="file"
                value={candidate.local_files == null ? "—" : String(candidate.local_files)}
                detail={candidate.local_files == null
                  ? (candidate.local_present ? "On this Mac, but nothing could be captured" : "Not on this Mac, so nothing was counted")
                  : `Files · ${candidate.local_files}`}
              />
              <Metric
                icon="drive"
                value={candidate.local_bytes == null ? "—" : formatBytes(candidate.local_bytes)}
                detail={candidate.local_bytes == null
                  ? (candidate.local_present ? "On this Mac, but nothing could be captured" : "Not on this Mac, so nothing was measured")
                  : `On disk · ${formatBytes(candidate.local_bytes)}`}
              />
              <span className="sync-history-thread-metric" aria-hidden="true" />
            </div>
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

/* ── the bar ─────────────────────────────────────────────────────────────── */

function tallies(rows: CatalogScopeRow[]) {
  return rows.reduce((totals, current) => {
    const status = rowStatus(current);
    if (status === "local-outstanding") totals.push += 1;
    else if (status === "cloud-outstanding") totals.pull += 1;
    else if (status !== "synced" && status !== "untracked") totals.attention += 1;
    return totals;
  }, { push: 0, pull: 0, attention: 0 });
}

function PetBar({ rows, scopeLabel, active, overflow }: {
  rows: CatalogScopeRow[];
  scopeLabel: string;
  active?: "push" | "pull";
  overflow: PathBarMenuGroup[];
}) {
  const counts = tallies(rows);
  const crumbs: PathBarCrumb[] = [
    {
      id: "storage",
      label: STORAGE_NAME,
      icon: "drive",
      tone: "root",
      title: `${STORAGE_NAME} · ~/Library/Mallard/local-1`,
      ariaLabel: `${STORAGE_NAME}. Storage menu`,
      menuLabel: "Storage",
      menuHeader: { label: STORAGE_NAME, detail: "~/Library/Mallard/local-1", detailIsPath: true, icon: "drive" },
      menu: [{ items: [{ id: "settings", label: "Storage settings…", icon: "settings", onSelect: () => undefined }] }],
    },
    {
      id: "scope",
      label: scopeLabel,
      icon: "openai",
      title: `${scopeLabel}\n/Users/hequ/.codex`,
      ariaLabel: `${scopeLabel}. Catalog row menu`,
      menuLabel: "Catalog row",
      menuHeader: { label: scopeLabel, detail: "/Users/hequ/.codex", detailIsPath: true, icon: "openai" },
      menu: [{ items: [{ id: "rescan", label: "Rescan this provider home", icon: "refresh", onSelect: () => undefined }] }],
    },
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      overflow={overflow}
      overflowLabel="App actions"
      ariaLabel={`${scopeLabel} path`}
      actions={(
        <>
          {counts.attention > 0 && (
            <span className="sync-path-attention warning" tabIndex={0} role="status" data-tooltip="1 pet can't be synced">
              <Icon name="alert-triangle" size={13} /><b>{counts.attention}</b>
            </span>
          )}
          <button type="button" className="sync-path-icon-action" title="Refresh comparison" aria-label="Refresh comparison">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className={`sync-path-sync${active === "pull" ? " active" : ""}`} aria-expanded={active === "pull"}>
              <Icon name="download" size={14} />
              <span className="sync-path-sync-label">Pull</span>
              {counts.pull > 0 && active !== "pull" && <span className="sync-path-count" aria-hidden="true">{counts.pull}</span>}
            </button>
            <button type="button" className={`sync-path-sync${active === "push" ? " active" : ""}`} aria-expanded={active === "push"}>
              <Icon name="upload" size={14} />
              <span className="sync-path-sync-label">Push</span>
              {counts.push > 0 && active !== "push" && <span className="sync-path-count" aria-hidden="true">{counts.push}</span>}
            </button>
          </div>
        </>
      )}
    />
  );
}

/* ── the modes ───────────────────────────────────────────────────────────── */

function ListStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  return (
    <div className="sync-workspace">
      <PetBar rows={V2_ROWS} scopeLabel="Pets v2 · Default Codex" overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
        <section className="profile-links-section" aria-label="Pets v2">
          <div className="sync-history-section-heading">
            <h2><Icon name="openai" size={14} />Pets v2</h2>
            <span className="sync-history-heading-count"><Icon name="openai" size={12} />{V2_ROWS.length}</span>
          </div>
          <div className="sync-history-thread-list">
            {V2_ROWS.map((candidate) => (
              <PetCard key={candidate.row_id} candidate={candidate} storageName={STORAGE_NAME} nameOpens />
            ))}
          </div>
        </section>
      </main>
    </div>
  );
}

function PullStage({ overflow }: { overflow: PathBarMenuGroup[] }) {
  const entries = V1_ROWS;
  const [selected, setSelected] = useState<Set<string>>(() => new Set(entries.map((candidate) => candidate.row_id)));
  const [choice, setChoice] = useState<Record<string, string>>(() => Object.fromEntries(
    entries.map((candidate) => [candidate.row_id, PULL_ACTIONS[candidate.row_id][0].action_id]),
  ));
  const [acks, setAcks] = useState<Set<string>>(new Set());
  const [openChoices, setOpenChoices] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

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
      <PetBar rows={V1_ROWS} scopeLabel="Pets v1 · Default Codex" active="pull" overflow={overflow} />
      <main className="sync-main sync-project-links-page sync-sync-review-page">
        <section className="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace" aria-labelledby="pet-pull-title">
          <header className="sync-inline-action-header sync-sync-review-header">
            <div className="sync-sync-review-title"><h2 id="pet-pull-title">Pull review</h2></div>
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
              <div className="sync-history-thread-list" ref={listRef}>
                {entries.map((candidate) => {
                  const included = selected.has(candidate.row_id);
                  const expanded = openChoices === candidate.row_id;
                  const action = chosenAction(candidate.row_id);
                  const chosen = action ? actionLabel(action) : "Skip";
                  return (
                    <PetCard
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
            <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={selected.size === 0 || blocked}>
              <ThreadStatusIcon status="cloud-outstanding" size={16} />
              Pull
            </button>
          </footer>
        </section>
      </main>
    </div>
  );
}

/** The preview's anatomy: where the picture comes from, how it scales, and
 *  what stands in when there is nothing to draw. */
function AnatomyStage() {
  return (
    <div className="pet-anatomy">
      <figure>
        <figcaption>The source: cell (0,0) of the atlas — the idle frame.
          8 columns of 192×208; 9 rows in v1, 11 in v2. Cropped once per
          content digest, scaled to 96×104 (2× the box), cached.</figcaption>
        <div className="pet-anatomy-strip">
          {["tracebill", "maple", "patch", "dahuang", "tracebit"].map((pet) => (
            <span key={pet} className="catalog-pet-preview"><img src={PET_THUMBS[pet]} alt={pet} /></span>
          ))}
        </div>
      </figure>
      <figure>
        <figcaption>Smooth downscale, not <code>image-rendering: pixelated</code>:
          these atlases are painterly, and the pixelated hint (right) is only
          for true pixel art.</figcaption>
        <div className="pet-anatomy-strip">
          <span className="catalog-pet-preview"><img src={PET_THUMBS.tracebill} alt="smooth" /></span>
          <span className="catalog-pet-preview"><img src={PET_THUMBS.tracebill} alt="pixelated" style={{ imageRendering: "pixelated" }} /></span>
        </div>
      </figure>
      <figure>
        <figcaption>Nothing to draw, still a column: stored-only and blocked
          rows keep the tile and wear the kind glyph — never a broken image,
          never a shifted name.</figcaption>
        <div className="pet-anatomy-strip">
          <span className="catalog-pet-preview"><Icon name="openai" size={16} /></span>
        </div>
      </figure>
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
            <h1>Catalog · pet previews</h1>
            <p>
              The shipped Option E card, with one addition on pet scope pages: the pet&rsquo;s own
              idle frame leading the card. The grammar is the industry&rsquo;s two-line-row-with-
              artwork standard — iOS table cells, Finder, Slack file rows — where the picture
              spans both lines on the left and everything else stays put. Every preview below is
              the real first cell of the real spritesheet in <code>~/.codex/pets</code>, cropped
              192×208 and scaled to 2× the 44×48 tile: the exact artifact the backend would
              derive once per content digest and cache. Skills pages are untouched — a scope
              page is one kind, so the column never appears where there is nothing to draw.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn" onClick={toggleTheme}>
              <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
            </button>
          </div>
        </header>

        <Panel
          title="The list"
          standard="Pets v2 · Default Codex, a storage in the bar"
          note={<>Maple, 大黄 and Patch wear their real idle frames. <code>Biscuit</code> is only
            in storage and <code>Scout</code> is blocked, so both wear the kind glyph in the same
            tile — the column holds its width, the badge still carries the state, and line two
            still reads two em-dashes. Pets report one file, so line two&rsquo;s figures are the
            atlas: <code>1 · 2.5 MB</code>.</>}
        >
          <ListStage overflow={overflow} />
        </Panel>

        <Panel
          title="Pull review"
          standard="Pets v1 · Default Codex, the same card under review"
          note={<>The checkbox, the action pill and the acknowledgement chip all coexist with the
            preview: switch <code>Tracebit</code> to Overwrite and the danger chip joins the card
            under the same left edge, while the duck stays where it was. Seeing which pet an
            overwrite replaces is the point of putting the picture on the row.</>}
        >
          <PullStage overflow={overflow} />
        </Panel>

        <Panel
          title="The preview itself"
          standard="Derivation, scaling, and the empty tile"
          note={<>What the backend caches and why the tile never breaks. One decoded atlas is
            1.7–2.6&nbsp;MB; the derived cell is 5–20&nbsp;KB — a thousandfold saving that makes
            the list DTO carry previews without weighing anything.</>}
        >
          <AnatomyStage />
        </Panel>
      </div>
    </div>
  );
}

/* ── proposed CSS ────────────────────────────────────────────────────────── */

const MOCK_CSS = `
/* The pet card opens a leading column in the retro card's own template —
   "title actions" over "meta meta" becomes the same two rows with a preview
   spanning both. Everything already placed by area stays where it was.
   (Ships in RetroTheme.css, beside the template it extends.) */
.catalog-card-pet {
  grid-template-columns: auto minmax(0, 1fr) var(--sync-history-row-actions-column);
  grid-template-areas:
    "preview title actions"
    "preview meta meta";
}

/* An acknowledgement chip is auto-placed, so it must not fall under the
   preview column. */
.catalog-card-pet > .catalog-card-ack {
  grid-column: 2 / -1;
}

/* A fixed 44x48 tile (the atlas cell's 192:208, to the pixel grid), the
   artwork letterboxed inside it, on the card's own paper. The empty tile
   wears the kind glyph so the column never collapses. */
.catalog-pet-preview {
  grid-area: preview;
  width: 44px;
  height: 48px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-2);
  color: var(--text-3);
}

.catalog-pet-preview > img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

/* Outside a card — the anatomy strips — the tile stands alone. */
.pet-anatomy .catalog-pet-preview {
  grid-area: auto;
}
`;

const SHELL_CSS = `
html, body, #root { height: auto; min-height: 100%; overflow: visible; }
.app { height: auto; min-height: 100vh; overflow: visible; }
.mockup-stage .sync-workspace { height: auto; }
.mockup-stage .sync-main { height: auto; overflow: visible; }

.mockup-shell { width: 100%; max-width: 1320px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 78ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-controls { display: inline-flex; gap: 8px; flex: 0 0 auto; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > .mockup-standard { margin: 0 0 8px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); letter-spacing: .04em; text-transform: uppercase; }
.mockup-panel > p { max-width: 84ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }

.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--bg-0); overflow: visible; }
.mockup-stage > .sync-workspace > .sync-pathbar { border-radius: 6px 6px 0 0; }
.mockup-stage .sync-main { border-radius: 0 0 6px 6px; }
body { background: var(--app-shell); }

.pet-anatomy { display: grid; gap: 18px; padding: 16px; }
.pet-anatomy figure { margin: 0; display: grid; gap: 8px; }
.pet-anatomy figcaption { max-width: 76ch; color: var(--text-2); font-size: var(--type-xs); line-height: 1.5; }
.pet-anatomy-strip { display: flex; align-items: center; gap: 10px; }
`;

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
