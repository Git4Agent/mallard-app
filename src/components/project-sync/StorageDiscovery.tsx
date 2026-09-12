import { useState, type ReactNode } from "react";
import type {
  StorageDiscoveryCandidate,
  StorageDiscoveryDescriptor,
  StorageDiscoveryResult,
  StorageProviderDescriptor,
  StorageSummary,
} from "../../types";
import Icon, { iconNameFromHint, type IconName } from "../Icons";

/**
 * The Use-existing half of a new storage draft: the look into the account,
 * and what it found. The workspace owns the scan, the choice, and the
 * prepare step; this file only draws them, from the descriptor's words and
 * the backend's result. Nothing here knows a provider by name.
 *
 * The rows, their groups, the empty scan, and the incomplete-scan notice
 * are shared with the R2 lane, which lists the same result inside its
 * Buckets step and puts the verb on the row.
 */

/** Where the look stands. `preparing` keeps the result so the list stays
 *  in view, inert, while the browser step runs for the chosen row. */
export type DiscoveryState =
  | { kind: "idle" }
  | { kind: "scanning" }
  | { kind: "result"; result: StorageDiscoveryResult }
  | { kind: "error"; message: string }
  | { kind: "preparing"; candidate: StorageDiscoveryCandidate; result: StorageDiscoveryResult };

/** A candidate the user may choose: a readable Mallard storage that no
 *  configured storage already points at. */
export function candidateSelectable(
  candidate: Pick<StorageDiscoveryCandidate, "marker" | "connected_storage_id">,
): boolean {
  return candidate.marker === "storage" && !candidate.connected_storage_id;
}

/** The marker's word for the row and the rung it reads at. The glyph and the
 *  sentence carry the meaning; the colour only seconds them. */
export function candidateVerdict(
  candidate: Pick<StorageDiscoveryCandidate, "marker" | "marker_detail">,
): { tone: "ok" | "mute" | "warn" | "bad"; icon: IconName; text: string } {
  const detail = candidate.marker_detail?.trim();
  const withDetail = (text: string) => (detail ? `${text} · ${detail}` : text);
  switch (candidate.marker) {
    case "storage": return { tone: "ok", icon: "check-circle", text: withDetail("Mallard storage") };
    case "missing": return { tone: "mute", icon: "ban", text: "No Mallard data" };
    case "unsupported": return { tone: "warn", icon: "alert-triangle", text: withDetail("Needs a newer Mallard") };
    case "invalid": return { tone: "bad", icon: "alert-triangle", text: withDetail("Marker is not valid") };
    case "unreadable": return { tone: "bad", icon: "alert-triangle", text: withDetail("Could not read the marker") };
  }
}

/** The tally under the scope line: storages first, then every other outcome
 *  that occurred — never a zero for one that did not. */
export function discoveryTally(result: Pick<StorageDiscoveryResult, "candidates">): string {
  const count = (marker: StorageDiscoveryCandidate["marker"]) => (
    result.candidates.filter((candidate) => candidate.marker === marker).length
  );
  const storages = count("storage");
  const parts = [
    `${storages} Mallard storage${storages === 1 ? "" : "s"}`,
    `${count("missing")} without Mallard data`,
  ];
  const unsupported = count("unsupported");
  if (unsupported > 0) parts.push(`${unsupported} unsupported`);
  const failed = count("invalid") + count("unreadable");
  if (failed > 0) parts.push(`${failed} could not be read`);
  return parts.join(" · ");
}

function GroupHead({ label, count, open, onToggle }: {
  label: string;
  count: number;
  open?: boolean;
  onToggle?: () => void;
}) {
  const inner = (
    <>
      <span>{label}</span>
      <b>{count}</b>
      {onToggle && <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />}
    </>
  );
  return onToggle
    ? <button type="button" className="storage-found-group-head" aria-expanded={open} onClick={onToggle}>{inner}</button>
    : <div className="storage-found-group-head">{inner}</div>;
}

/** One row per candidate: glyph, name over its verdict, facts, and the verb
 *  column. With `choice` the whole row is a radio's label, so the choice is
 *  one click anywhere on it and the page's footer acts on it; without it the
 *  row carries its own verb — a connected candidate names its storage and
 *  opens it either way. */
export function CandidateRow({ descriptor, candidate, disabled, connectedName, choice, verb, onOpenStorage }: {
  descriptor: Pick<StorageProviderDescriptor, "icon">;
  candidate: StorageDiscoveryCandidate;
  disabled: boolean;
  /** The configured storage already pointing here, by name. */
  connectedName: string | null;
  choice?: { selected: boolean; onSelect: (candidateId: string) => void };
  /** The row's own verb, for a candidate that can be taken. */
  verb?: ReactNode;
  onOpenStorage: (storageId: string) => void;
}) {
  const verdict = candidateVerdict(candidate);
  const classes = [
    "storage-found-row",
    candidate.marker === "missing" ? "muted" : "",
    choice ? (choice.selected ? "selected" : "") : "with-verb",
  ].filter(Boolean).join(" ");
  const inner = (
    <>
      {choice && (
        /* A row that cannot be chosen carries a disabled radio, which also
           keeps its Open button from selecting anything. */
        <input
          type="radio"
          name="storage-found-candidate"
          checked={choice.selected}
          disabled={disabled || !candidateSelectable(candidate)}
          onChange={() => choice.onSelect(candidate.candidate_id)}
          aria-label={`Use ${candidate.name}`}
        />
      )}
      <span className="sync-storage-repository-icon"><Icon name={iconNameFromHint(descriptor.icon, "drive")} size={14} /></span>
      <span className="storage-found-text">
        <span className="storage-found-name">
          {candidate.name}
          {candidate.identity && <code>{candidate.identity}</code>}
        </span>
        <span className={`storage-found-verdict ${verdict.tone}`}>
          <Icon name={verdict.icon} size={12} /> {verdict.text}
        </span>
      </span>
      <span className="storage-found-facts">
        {candidate.facts.map((fact) => (
          <span key={fact.label} className="sync-storage-repo-chip"><span>{fact.label}</span><b>{fact.value}</b></span>
        ))}
        {candidate.cautions.map((caution) => (
          <span key={caution} className="storage-found-caution"><Icon name="alert-triangle" size={12} /> {caution}</span>
        ))}
      </span>
      <span className="storage-found-verb">
        {candidate.connected_storage_id ? (
          <>
            <span className="storage-found-connected"><Icon name="check" size={12} /> Connected · {connectedName ?? "another storage"}</span>
            <button
              type="button"
              className="btn btn-ghost"
              disabled={disabled}
              onClick={() => onOpenStorage(candidate.connected_storage_id!)}
            >
              Open
            </button>
          </>
        ) : verb}
      </span>
    </>
  );
  return choice
    ? <label className={classes}>{inner}</label>
    : <div className={classes}>{inner}</div>;
}

/** The results, grouped by what the marker said: storages first, the ones a
 *  newer Mallard wrote next, the destinations without Mallard data folded,
 *  and every failed read named rather than dropped. Nothing when the scan
 *  listed nothing at all. */
export function CandidateGroups({ result, unitPlural, choice, row }: {
  result: Pick<StorageDiscoveryResult, "candidates">;
  unitPlural: string;
  /** The rows are radios of one group. */
  choice: boolean;
  row: (candidate: StorageDiscoveryCandidate) => ReactNode;
}) {
  const [othersOpen, setOthersOpen] = useState(false);
  const found = result.candidates.filter((candidate) => candidate.marker === "storage");
  const newer = result.candidates.filter((candidate) => candidate.marker === "unsupported");
  const others = result.candidates.filter((candidate) => candidate.marker === "missing");
  const failed = result.candidates.filter((candidate) => candidate.marker === "invalid" || candidate.marker === "unreadable");
  if (result.candidates.length === 0) return null;
  return (
    <div className="storage-found-list" role={choice ? "radiogroup" : undefined} aria-label={`${unitPlural} found`}>
      {found.length > 0 && (
        <>
          <GroupHead label="Mallard storages" count={found.length} />
          {found.map(row)}
        </>
      )}
      {newer.length > 0 && (
        <>
          <GroupHead label="Needs a newer Mallard" count={newer.length} />
          {newer.map(row)}
        </>
      )}
      {others.length > 0 && (
        <>
          <GroupHead
            label={`Other ${unitPlural} — no Mallard data`}
            count={others.length}
            open={othersOpen}
            onToggle={() => setOthersOpen((open) => !open)}
          />
          {othersOpen && others.map(row)}
        </>
      )}
      {failed.length > 0 && (
        <>
          <GroupHead label="Could not be checked" count={failed.length} />
          {failed.map(row)}
        </>
      )}
    </div>
  );
}

/** A short list is not an empty one: the box says what the scan skipped so
 *  a storage that would have been listed is not taken for absent. The verb
 *  that scans again lives on the line above it, not here. */
export function ScanIncompleteNotice({ issues }: { issues: string[] }) {
  return (
    <div className="notice warning storage-found-notice" role="alert">
      <Icon name="alert-triangle" size={13} className="notice-mark" />
      <div className="notice-body">
        <p className="notice-line">
          <strong className="notice-type">Warning ·</strong>{" "}
          <strong className="notice-lead">Scan incomplete.</strong>{" "}
          {issues.join(" ")} A Mallard storage there would not be listed.
        </p>
      </div>
    </div>
  );
}

/** Nothing found: the mark, the fact, the way on. */
export function DiscoveryEmpty({ place, unit, sentence, disabled, onCreateInstead }: {
  place: string;
  unit: string;
  /** What was checked, and that nothing was changed. */
  sentence: string;
  disabled: boolean;
  /** The empty scan's way on: back to the Create-new door. */
  onCreateInstead: () => void;
}) {
  return (
    <div className="storage-found-empty">
      <Icon name="search" size={24} />
      <strong>No Mallard storage in {place}.</strong>
      <span>{sentence}</span>
      <span className="storage-found-empty-verbs">
        <button type="button" className="btn btn-primary" onClick={onCreateInstead} disabled={disabled}>
          Create a new {unit} instead
        </button>
      </span>
    </div>
  );
}

/** Pasting a connection is also using an existing destination, so the
 *  fields live under the Use-existing door too — one line, and they unfold. */
export function ManualEntryRow({ manualEntry, disabled, onToggle }: {
  manualEntry: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="storage-setup-route-row" role="note">
      <Icon name="info" size={13} />
      <span className="storage-setup-route-label">By hand</span>
      <span className="storage-setup-guide-copy">Already hold the connection values? Paste them instead.</span>
      <button
        type="button"
        className="btn storage-manual-toggle"
        aria-expanded={manualEntry}
        aria-label={manualEntry ? "Hide fields" : "Enter by hand"}
        title={manualEntry ? "Hide fields" : "Enter by hand"}
        onClick={onToggle}
        disabled={disabled}
      >
        <Icon name={manualEntry ? "chevron-up" : "chevron-down"} size={12} />
      </button>
    </div>
  );
}

function DiscoveryResults({ descriptor, discovery, result, selectedId, storages, disabled, onSelect, onScan, onOpenStorage, onCreateInstead }: {
  descriptor: Pick<StorageProviderDescriptor, "icon">;
  discovery: StorageDiscoveryDescriptor;
  result: StorageDiscoveryResult;
  selectedId: string | null;
  storages: readonly StorageSummary[];
  disabled: boolean;
  onSelect: (candidateId: string) => void;
  onScan: () => void;
  onOpenStorage: (storageId: string) => void;
  onCreateInstead: () => void;
}) {
  const checked = result.candidates.length;
  const found = result.candidates.filter((candidate) => candidate.marker === "storage").length;
  return (
    <>
      <div className="storage-found-scope" role="status">
        <Icon name="search" size={13} />
        <span className="storage-found-scope-text">
          <span>{result.scope} · {checked} {checked === 1 ? discovery.unit : discovery.unit_plural} checked, read-only.</span>
          <code>{discoveryTally(result)}</code>
        </span>
        <button type="button" className="btn-link" onClick={onScan} disabled={disabled}>
          <Icon name="refresh" size={11} /> Scan again
        </button>
      </div>
      {result.status === "partial" && <ScanIncompleteNotice issues={result.issues} />}
      <CandidateGroups
        result={result}
        unitPlural={discovery.unit_plural}
        choice
        row={(candidate) => (
          <CandidateRow
            key={candidate.candidate_id}
            descriptor={descriptor}
            candidate={candidate}
            disabled={disabled}
            connectedName={candidate.connected_storage_id
              ? storages.find((storage) => storage.storage_id === candidate.connected_storage_id)?.name ?? null
              : null}
            choice={{ selected: candidate.candidate_id === selectedId, onSelect }}
            onOpenStorage={onOpenStorage}
          />
        )}
      />
      {found === 0 && (
        <DiscoveryEmpty
          place={discovery.place}
          unit={discovery.unit}
          sentence={`${checked} ${checked === 1 ? discovery.unit : discovery.unit_plural} checked; none holds Mallard data. Nothing was changed.`}
          disabled={disabled}
          onCreateInstead={onCreateInstead}
        />
      )}
    </>
  );
}

export function StorageDiscoveryPanel({
  descriptor,
  discovery,
  state,
  selectedId,
  storages,
  manualEntry,
  disabled,
  onScan,
  onCancelScan,
  onSelect,
  onOpenStorage,
  onToggleManual,
  onCreateInstead,
}: {
  descriptor: Pick<StorageProviderDescriptor, "icon">;
  /** The descriptor's discovery block, already known to be present. */
  discovery: StorageDiscoveryDescriptor;
  state: DiscoveryState;
  selectedId: string | null;
  /** Every configured storage, to name the one a candidate is already
   *  connected as. */
  storages: readonly StorageSummary[];
  /** The fields are shown under the panel for a connection pasted by hand. */
  manualEntry: boolean;
  disabled: boolean;
  onScan: () => void;
  onCancelScan: () => void;
  onSelect: (candidateId: string) => void;
  onOpenStorage: (storageId: string) => void;
  onToggleManual: () => void;
  /** The empty scan's way on: back to the Create-new door. */
  onCreateInstead: () => void;
}) {
  return (
    <>
      {state.kind === "scanning" ? (
        <div className="storage-setup-guide" role="status">
          <span className="storage-setup-guide-wait"><span className="status-loader" /> {discovery.scanning_hint}</span>
          <button type="button" className="btn-link" onClick={onCancelScan}>Cancel</button>
        </div>
      ) : state.kind === "preparing" ? (
        <div className="storage-setup-guide" role="status">
          <span className="storage-setup-guide-wait">
            <span className="status-loader" /> Approve access to {state.candidate.name} in your browser. {discovery.prepare_hint}
          </span>
        </div>
      ) : (
        <div className="storage-setup-route-row" role="note">
          <Icon name="search" size={13} />
          <span className="storage-setup-route-label">Existing storage</span>
          <span className="storage-setup-guide-copy">{discovery.access_hint}</span>
          {state.kind === "idle" && (
            <button
              type="button"
              className="btn storage-setup-guide-button storage-setup-agent-cta"
              onClick={onScan}
              disabled={disabled}
            >
              <Icon name="search" size={13} /> {discovery.find_label}
            </button>
          )}
        </div>
      )}
      {state.kind === "error" && (
        <div className="notice danger storage-found-notice" role="alert">
          <Icon name="alert-triangle" size={13} className="notice-mark" />
          <div className="notice-body">
            <p className="notice-line">
              <strong className="notice-type">Error ·</strong>{" "}
              <strong className="notice-lead">Could not look in {discovery.place}.</strong>{" "}
              {state.message}
            </p>
          </div>
          <button type="button" className="btn btn-primary" onClick={onScan} disabled={disabled}>Try again</button>
        </div>
      )}
      {(state.kind === "result" || state.kind === "preparing") && (
        <DiscoveryResults
          descriptor={descriptor}
          discovery={discovery}
          result={state.result}
          selectedId={selectedId}
          storages={storages}
          disabled={disabled || state.kind === "preparing"}
          onSelect={onSelect}
          onScan={onScan}
          onOpenStorage={onOpenStorage}
          onCreateInstead={onCreateInstead}
        />
      )}
      {state.kind !== "preparing" && (
        <ManualEntryRow manualEntry={manualEntry} disabled={disabled} onToggle={onToggleManual} />
      )}
    </>
  );
}
