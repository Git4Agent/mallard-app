import { useState, type ReactNode } from "react";
import type {
  StorageDiscoveryCandidate,
  StorageDiscoveryDescriptor,
  StorageDiscoveryFact,
  StorageDiscoveryIssue,
  StorageDiscoveryResult,
  StorageProviderDescriptor,
  StorageSummary,
} from "../../types";
import Icon, { storageIconName, type IconName } from "../Icons";
import Notice from "../notice/Notice";
import { t, uiLocale, useTranslation } from "../../i18n";
import { backendText } from "../../i18n/backend";
import { joinAnd } from "../../format";

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

/* The scan's sentences are the backend's English; each carries a code (and
   the values it was built from) that the interface translates by. */

/** The marker read's detail: "layout 1", "access denied". */
function markerDetail(
  candidate: Pick<StorageDiscoveryCandidate, "marker_detail" | "marker_detail_code" | "marker_layout_version">,
): string | undefined {
  const detail = candidate.marker_detail?.trim();
  if (!detail || !candidate.marker_detail_code) return detail;
  return backendText(`discovery.markerDetail.${candidate.marker_detail_code}`, detail, {
    version: candidate.marker_layout_version ?? "",
  });
}

/** A fact chip's label and value. A date is the backend's own in English
 *  and formatted for the interface's locale otherwise. */
export function discoveryFact(fact: StorageDiscoveryFact): { label: string; value: string } {
  if (!fact.key) return fact;
  const label = backendText(`discovery.fact.${fact.key}`, fact.label);
  const locale = uiLocale();
  if (fact.key === "created") {
    // Cloudflare stamps creation in UTC, which is the day the backend prints.
    const value = fact.at != null && locale
      ? new Intl.DateTimeFormat(locale, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" })
        .format(fact.at * 1000)
      : fact.value;
    return { label, value };
  }
  return {
    label,
    value: fact.value_code ? backendText(`discovery.${fact.key}.${fact.value_code}`, fact.value) : fact.value,
  };
}

/** Where the scan looked: the jurisdictions it read, or the shares. */
export function discoveryScope(
  result: Pick<StorageDiscoveryResult, "scope" | "scope_code" | "jurisdictions">,
): string {
  if (result.scope_code === "jurisdictions" && result.jurisdictions?.length && uiLocale()) {
    return joinAnd(result.jurisdictions.map((code) => backendText(`discovery.jurisdiction.${code}`, code)));
  }
  return result.scope_code ? backendText(`discovery.scope.${result.scope_code}`, result.scope) : result.scope;
}

/** The descriptor's discovery words in the interface's language. */
export function discoveryWords(provider: string, discovery: StorageDiscoveryDescriptor): StorageDiscoveryDescriptor {
  const word = (field: keyof StorageDiscoveryDescriptor) => backendText(`discovery.${provider}.${field}`, discovery[field]);
  return {
    unit: word("unit"),
    unit_plural: word("unit_plural"),
    place: word("place"),
    find_label: word("find_label"),
    access_hint: word("access_hint"),
    scanning_hint: word("scanning_hint"),
    prepare_hint: word("prepare_hint"),
  };
}

/** The marker's word for the row and the rung it reads at. The glyph and the
 *  sentence carry the meaning; the colour only seconds them. */
export function candidateVerdict(
  candidate: Pick<StorageDiscoveryCandidate, "marker" | "marker_detail" | "marker_detail_code" | "marker_layout_version">,
): { tone: "ok" | "mute" | "warn" | "bad"; icon: IconName; text: string } {
  const detail = markerDetail(candidate);
  const withDetail = (text: string) => (detail ? `${text} · ${detail}` : text);
  switch (candidate.marker) {
    case "storage": return { tone: "ok", icon: "check-circle", text: withDetail(t("storage:discovery.verdict.storage")) };
    case "missing": return { tone: "mute", icon: "ban", text: t("storage:discovery.verdict.missing") };
    case "unsupported": return { tone: "warn", icon: "alert-triangle", text: withDetail(t("storage:discovery.verdict.unsupported")) };
    case "invalid": return { tone: "bad", icon: "alert-triangle", text: withDetail(t("storage:discovery.verdict.invalid")) };
    case "unreadable": return { tone: "bad", icon: "alert-triangle", text: withDetail(t("storage:discovery.verdict.unreadable")) };
  }
}

/** The tally under the scope line: storages first, then every other outcome
 *  that occurred — never a zero for one that did not. */
export function discoveryTally(result: Pick<StorageDiscoveryResult, "candidates">): string {
  const count = (marker: StorageDiscoveryCandidate["marker"]) => (
    result.candidates.filter((candidate) => candidate.marker === marker).length
  );
  const parts: string[] = [
    t("storage:discovery.tally.storages", { count: count("storage") }),
    t("storage:discovery.tally.missing", { count: count("missing") }),
  ];
  const unsupported = count("unsupported");
  if (unsupported > 0) parts.push(t("storage:discovery.tally.unsupported", { count: unsupported }));
  const failed = count("invalid") + count("unreadable");
  if (failed > 0) parts.push(t("storage:discovery.tally.failed", { count: failed }));
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
  descriptor: Pick<StorageProviderDescriptor, "key" | "icon">;
  candidate: StorageDiscoveryCandidate;
  disabled: boolean;
  /** The configured storage already pointing here, by name. */
  connectedName: string | null;
  choice?: { selected: boolean; onSelect: (candidateId: string) => void };
  /** The row's own verb, for a candidate that can be taken. */
  verb?: ReactNode;
  onOpenStorage: (storageId: string) => void;
}) {
  const { t } = useTranslation(["storage", "common"]);
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
          aria-label={t("discovery.use", { name: candidate.name })}
        />
      )}
      <span className="sync-storage-repository-icon"><Icon name={storageIconName(descriptor.key, descriptor.icon)} size={14} /></span>
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
        {candidate.facts.map((fact) => {
          const { label, value } = discoveryFact(fact);
          return <span key={fact.label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>;
        })}
        {candidate.cautions.map((caution, index) => {
          const code = candidate.caution_codes?.[index];
          return (
            <span key={caution} className="storage-found-caution">
              <Icon name="alert-triangle" size={12} /> {code ? backendText(`discovery.caution.${code}`, caution) : caution}
            </span>
          );
        })}
      </span>
      <span className="storage-found-verb">
        {candidate.connected_storage_id ? (
          <>
            <span className="storage-found-connected"><Icon name="check" size={12} /> {t("discovery.connected", { name: connectedName ?? t("discovery.anotherStorage") })}</span>
            <button
              type="button"
              className="btn btn-ghost"
              disabled={disabled}
              onClick={() => onOpenStorage(candidate.connected_storage_id!)}
            >
              {t("common:open")}
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
  const { t } = useTranslation("storage");
  const [othersOpen, setOthersOpen] = useState(false);
  const found = result.candidates.filter((candidate) => candidate.marker === "storage");
  const newer = result.candidates.filter((candidate) => candidate.marker === "unsupported");
  const others = result.candidates.filter((candidate) => candidate.marker === "missing");
  const failed = result.candidates.filter((candidate) => candidate.marker === "invalid" || candidate.marker === "unreadable");
  if (result.candidates.length === 0) return null;
  return (
    <div className="storage-found-list" role={choice ? "radiogroup" : undefined} aria-label={t("discovery.found", { units: unitPlural })}>
      {found.length > 0 && (
        <>
          <GroupHead label={t("discovery.group.storages")} count={found.length} />
          {found.map(row)}
        </>
      )}
      {newer.length > 0 && (
        <>
          <GroupHead label={t("discovery.group.newer")} count={newer.length} />
          {newer.map(row)}
        </>
      )}
      {others.length > 0 && (
        <>
          <GroupHead
            label={t("discovery.group.others", { units: unitPlural })}
            count={others.length}
            open={othersOpen}
            onToggle={() => setOthersOpen((open) => !open)}
          />
          {othersOpen && others.map(row)}
        </>
      )}
      {failed.length > 0 && (
        <>
          <GroupHead label={t("discovery.group.failed")} count={failed.length} />
          {failed.map(row)}
        </>
      )}
    </div>
  );
}

/** A short list is not an empty one: the box says what the scan skipped so
 *  a storage that would have been listed is not taken for absent. The verb
 *  that scans again lives on the line above it, not here. */
export function ScanIncompleteNotice({ issues, codes }: {
  issues: string[];
  /** Each issue's code and count, in `issues` order. */
  codes?: StorageDiscoveryIssue[];
}) {
  const { t } = useTranslation("storage");
  const sentences = issues.map((issue, index) => {
    const code = codes?.[index];
    return code ? backendText(`discovery.issue.${code.code}`, issue, { count: code.count ?? "" }) : issue;
  });
  // English sentences are spaced; Chinese ones end in 。 and run on.
  const gap = uiLocale() ? "" : " ";
  return (
    <Notice severity="warning" timing="blocking" className="storage-found-notice" title={t("discovery.scanIncomplete")}>
      {[...sentences, t("discovery.scanIncompleteTail")].join(gap)}
    </Notice>
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
  const { t } = useTranslation("storage");
  return (
    <div className="storage-found-empty">
      <Icon name="search" size={24} />
      <strong>{t("discovery.emptyTitle", { place })}</strong>
      <span>{sentence}</span>
      <span className="storage-found-empty-verbs">
        <button type="button" className="btn btn-primary" onClick={onCreateInstead} disabled={disabled}>
          {t("discovery.createInstead", { unit })}
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
  const { t } = useTranslation("storage");
  return (
    <div className="storage-setup-route-row" role="note">
      <Icon name="info" size={13} />
      <span className="storage-setup-route-label">{t("discovery.byHand")}</span>
      <span className="storage-setup-guide-copy">{t("discovery.byHandHint")}</span>
      <button
        type="button"
        className="btn storage-manual-toggle"
        aria-expanded={manualEntry}
        aria-label={manualEntry ? t("shared.hideFields") : t("shared.enterByHand")}
        title={manualEntry ? t("shared.hideFields") : t("shared.enterByHand")}
        onClick={onToggle}
        disabled={disabled}
      >
        <Icon name={manualEntry ? "chevron-up" : "chevron-down"} size={12} />
      </button>
    </div>
  );
}

function DiscoveryResults({ descriptor, discovery, result, selectedId, storages, disabled, onSelect, onScan, onOpenStorage, onCreateInstead }: {
  descriptor: Pick<StorageProviderDescriptor, "key" | "icon">;
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
  const { t } = useTranslation("storage");
  const words = discoveryWords(descriptor.key, discovery);
  const checked = result.candidates.length;
  const nouns = { count: checked, unit: words.unit, units: words.unit_plural };
  const found = result.candidates.filter((candidate) => candidate.marker === "storage").length;
  return (
    <>
      <div className="storage-found-scope" role="status">
        <Icon name="search" size={13} />
        <span className="storage-found-scope-text">
          <span>{t("discovery.checked", { ...nouns, scope: discoveryScope(result) })}</span>
          <code>{discoveryTally(result)}</code>
        </span>
        <button type="button" className="btn-link" onClick={onScan} disabled={disabled}>
          <Icon name="refresh" size={11} /> {t("shared.scanAgain")}
        </button>
      </div>
      {result.status === "partial" && <ScanIncompleteNotice issues={result.issues} codes={result.issue_codes} />}
      <CandidateGroups
        result={result}
        unitPlural={words.unit_plural}
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
          place={words.place}
          unit={words.unit}
          sentence={t("discovery.noneFound", nouns)}
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
  descriptor: Pick<StorageProviderDescriptor, "key" | "icon">;
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
  const { t } = useTranslation(["storage", "common"]);
  const words = discoveryWords(descriptor.key, discovery);
  return (
    <>
      {state.kind === "scanning" ? (
        <div className="storage-setup-guide" role="status">
          <span className="storage-setup-guide-wait"><span className="status-loader" /> {words.scanning_hint}</span>
          <button type="button" className="btn-link" onClick={onCancelScan}>{t("common:cancel")}</button>
        </div>
      ) : state.kind === "preparing" ? (
        <div className="storage-setup-guide" role="status">
          <span className="storage-setup-guide-wait">
            <span className="status-loader" /> {t("discovery.approveAccess", { name: state.candidate.name })} {words.prepare_hint}
          </span>
        </div>
      ) : (
        <div className="storage-setup-route-row" role="note">
          <Icon name="search" size={13} />
          <span className="storage-setup-route-label">{t("discovery.existing")}</span>
          <span className="storage-setup-guide-copy">{words.access_hint}</span>
          {state.kind === "idle" && (
            <button
              type="button"
              className="btn storage-setup-guide-button storage-setup-agent-cta"
              onClick={onScan}
              disabled={disabled}
            >
              <Icon name="search" size={13} /> {words.find_label}
            </button>
          )}
        </div>
      )}
      {state.kind === "error" && (
        <div className="notice danger storage-found-notice" role="alert">
          <Icon name="alert-triangle" size={13} className="notice-mark" />
          <div className="notice-body">
            <p className="notice-line">
              <strong className="notice-type">{t("shared.errorType")}</strong>{" "}
              <strong className="notice-lead">{t("discovery.lookFailed", { place: words.place })}</strong>{" "}
              {state.message}
            </p>
          </div>
          <button type="button" className="btn btn-primary" onClick={onScan} disabled={disabled}>{t("common:tryAgain")}</button>
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
