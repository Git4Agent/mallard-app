import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import { NoticeZone } from "./components/notice/Notice";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncReviewSelectionBar from "./components/sync-review/SyncReviewSelectionBar";
import ThreadStatusIcon, { type ThreadStatusIconName } from "./components/sync-review/ThreadStatusIcon";
import { PET_THUMBS } from "./catalog-pet-mockup-thumbs";
import "./App.css";
import "./RetroTheme.css";

/**
 * Specimen for the Catalog Push/Pull review. The shipped review reuses the
 * list's inventory card per row, and it has one hard defect plus a scale
 * problem this page demonstrates and proposes fixes for:
 *
 *  1. Geometry — `.catalog-card-ack` chips are auto-placed children of the
 *     card's grid. The base card template is `minmax(0,1fr) auto`; a card with
 *     TWO warnings drops the second chip into the `auto` actions track, which
 *     inflates to the chip's whole sentence — squeezing the title to "spee…"
 *     and colliding the chips. (The pet template got a placement pin; the base
 *     card never did. Same disease as the fork-form bug.)
 *  2. Scale — 37 rows × (provider mark + Reveal + ⓘ + full timestamp + boxed
 *     chips) is an inventory, not a review. No grouping, no order by decision
 *     weight, and the ack sentence buries its payload behind a constant
 *     24-character preamble.
 *
 * Three options below share the fixes and differ on one axis — how much the
 * row changes. Everything wears the shipped chrome (header, selection bar,
 * footer, badges, checkboxes) over the real stylesheets. Not part of the app
 * bundle — dev server only.
 *
 * DECIDED: Option A shipped (StorageCatalogPage + App.css). The reproduction
 * panel keeps the retired auto-placement through a scoped override; B and C
 * remain here as the declined alternatives.
 */

// ---------------------------------------------------------------------------
// Facts the panels draw — the screenshot's own resources.
// ---------------------------------------------------------------------------

const STORAGE = "Vault";

type MockWarningCode = "executable_content" | "secret_shaped";

interface MockWarning {
  code: MockWarningCode;
  paths: string;
}

interface MockSkill {
  id: string;
  name: string;
  files: string;
  size: string;
  stamp: string;
  date: string;
  relation: "new" | "changed";
  warnings: MockWarning[];
}

const SKILLS: MockSkill[] = [
  { id: "jupyter-notebook", name: "jupyter-notebook", files: "12", size: "30 KB", stamp: "Mar 31, 2026 at 17:24", date: "Mar 31, 2026", relation: "new", warnings: [] },
  { id: "sentry", name: "sentry", files: "6", size: "24 KB", stamp: "Mar 31, 2026 at 17:24", date: "Mar 31, 2026", relation: "new", warnings: [
    { code: "executable_content", paths: "scripts/sentry_api.py" },
    { code: "secret_shaped", paths: "scripts/sentry_api.py" },
  ] },
  { id: "yeet", name: "yeet", files: "5", size: "15 KB", stamp: "Mar 31, 2026 at 17:24", date: "Mar 31, 2026", relation: "new", warnings: [] },
  { id: "exascale-trace-download", name: "exascale-trace-download", files: "2", size: "8.2 KB", stamp: "Apr 30, 2026 at 12:12", date: "Apr 30, 2026", relation: "new", warnings: [] },
  { id: "brainstorming", name: "brainstorming", files: "9", size: "22 KB", stamp: "Mar 12, 2026 at 09:38", date: "Mar 12, 2026", relation: "new", warnings: [] },
  { id: "oci-ma-script-generator", name: "oci-ma-script-generator", files: "6", size: "46 KB", stamp: "Apr 30, 2026 at 12:12", date: "Apr 30, 2026", relation: "changed", warnings: [
    { code: "executable_content", paths: "auto-pin.sh, scripts/generate_ma_script.py" },
  ] },
  { id: "speech-synthesis", name: "speech-synthesis", files: "16", size: "48 KB", stamp: "Mar 31, 2026 at 17:24", date: "Mar 31, 2026", relation: "changed", warnings: [
    { code: "executable_content", paths: "scripts/text_to_speech.py" },
    { code: "secret_shaped", paths: "SKILL.md, references/cli.md, scripts/text_to_speech.py" },
  ] },
];

/** The lead derives from `warning.code` — the backend mints exactly two today
 * (`inventory.rs`) — and an unknown code falls back to the raw message, so no
 * warning ever loses its sentence to the map. */
const WARNING_LEAD: Record<MockWarningCode, string> = {
  executable_content: "Executable content",
  secret_shaped: "Credential-shaped content",
};

/** The backend's own sentence, verbatim shape, for the legacy panel. */
function backendMessage(warning: MockWarning): string {
  return warning.code === "executable_content"
    ? `contains executable content: ${warning.paths}`
    : `contains credential-shaped content: ${warning.paths}`;
}

function ackKey(skillId: string, warning: MockWarning): string {
  return `${skillId}:${warning.code}`;
}

const NEW_SKILLS = SKILLS.filter((skill) => skill.relation === "new");
const CHANGED_SKILLS = SKILLS.filter((skill) => skill.relation === "changed");
const RISKY_SKILLS = SKILLS.filter((skill) => skill.warnings.length > 0);
const CLEAN_SKILLS = SKILLS.filter((skill) => skill.warnings.length === 0);

const RELATION_LABEL: Record<MockSkill["relation"], string> = {
  new: `Only on this computer. Push to save it in ${STORAGE}.`,
  changed: `Newer on this computer. Push to update ${STORAGE}.`,
};

// ---------------------------------------------------------------------------
// Shared shipped-class fragments
// ---------------------------------------------------------------------------

function Badge({ status, tone, label }: { status: ThreadStatusIconName; tone: string; label: string }) {
  return (
    <span className={`sync-thread-sync-indicator ${tone}`} data-tooltip={label} aria-label={label} tabIndex={0}>
      <ThreadStatusIcon status={status} size={18} />
    </span>
  );
}

function RowChoice({ name, checked, onToggle }: { name: string; checked: boolean; onToggle: () => void }) {
  return (
    <label className="sync-sync-row-choice" title={`${checked ? "Exclude" : "Include"} ${name}`}>
      <input
        type="checkbox"
        checked={checked}
        aria-label={`${checked ? "Exclude" : "Include"} ${name}`}
        onChange={onToggle}
      />
    </label>
  );
}

function MetricSpan({ icon, value, detail, className }: { icon: "file" | "drive" | "alert-triangle"; value: string; detail: string; className?: string }) {
  return (
    <span className={`sync-history-thread-metric${className ? ` ${className}` : ""}`} data-tooltip={detail} aria-label={detail} tabIndex={0}>
      <Icon name={icon} size={12} /><b>{value}</b>
    </span>
  );
}

function InfoButton({ name }: { name: string }) {
  return (
    <button
      type="button"
      className="btn btn-ghost sync-history-session-detail-action"
      title={`Paths and digests for ${name}`}
      aria-label={`Paths and digests for ${name}`}
    >
      <Icon name="info" size={13} />
    </button>
  );
}

/** The review panel's shipped chrome: title, the selection bar, the scroll,
 * and the footer. Panels differ only in what the scroll holds. */
function ReviewShell({
  title,
  verb,
  count,
  total,
  onAll,
  onNone,
  footerNote,
  footerDisabled,
  buttonStatus,
  children,
  className,
}: {
  title: string;
  verb: string;
  count: number;
  total: number;
  onAll: () => void;
  onNone: () => void;
  footerNote: ReactNode;
  footerDisabled: boolean;
  buttonStatus: ThreadStatusIconName;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`sync-inline-action-review sync-sync-review-workspace mock-review${className ? ` ${className}` : ""}`}>
      <header className="sync-inline-action-header sync-sync-review-header">
        <div className="sync-sync-review-title"><h2>{title}</h2></div>
        <SyncReviewSelectionBar
          count={count}
          total={total}
          recommendedTitle={`Use the recommended selection (${total})`}
          recommendedAriaLabel={`Use recommended selection (${total})`}
          recommendedDisabled={count === total}
          onRecommended={onAll}
          clearDisabled={count === 0}
          onClear={onNone}
          close={{ ariaLabel: `Close ${title}`, onClose: () => undefined }}
        />
      </header>
      <div className="sync-inline-action-content sync-sync-review-content">
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          {children}
        </div>
      </div>
      <footer className="sync-inline-action-footer sync-sync-review-footer">
        {footerNote}
        <button type="button" className="btn btn-primary sync-pull-apply-button" disabled={footerDisabled}>
          <ThreadStatusIcon status={buttonStatus} size={16} />
          {verb} {count}
        </button>
      </footer>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Panel 1 — the shipped card review, verbatim classes (the reproduction)
// ---------------------------------------------------------------------------

function LegacyCard({ skill, checked, onToggle, acks, onAck }: {
  skill: MockSkill;
  checked: boolean;
  onToggle: () => void;
  acks: Set<string>;
  onAck: (key: string) => void;
}) {
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <RowChoice name={skill.name} checked={checked} onToggle={onToggle} />
          <Badge status="local-outstanding" tone="local" label={RELATION_LABEL[skill.relation]} />
          <strong title={skill.name}>{skill.name}</strong>
        </div>
        <div className="sync-history-thread-actions">
          <span className="sync-history-launch-action sync-history-provider-inert" title="Codex skill">
            <ProviderMark provider="codex" displayName="Codex" />
          </span>
          <button type="button" className="btn btn-ghost sync-history-launch-action" title={`Reveal ${skill.name} in Finder`}>
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${skill.name} details`}>
            <MetricSpan icon="file" value={skill.files} detail={`Files · ${skill.files}`} />
            <MetricSpan icon="drive" value={skill.size} detail={`On disk · ${skill.size}`} />
            {skill.warnings.length > 0
              ? <MetricSpan icon="alert-triangle" value={String(skill.warnings.length)} className="sync-history-partial" detail={skill.warnings.map(backendMessage).join("\n")} />
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <div className="sync-history-debug-menu"><InfoButton name={skill.name} /></div>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Edited ${skill.stamp}`} tabIndex={0}>
            <time>{skill.stamp}</time>
          </span>
        </div>
      </div>
      {checked && skill.warnings.map((warning) => (
        <label key={warning.code} className="catalog-card-ack">
          <span className="sync-sync-row-choice">
            <input
              type="checkbox"
              checked={acks.has(ackKey(skill.id, warning))}
              aria-label={`Reviewed: ${backendMessage(warning)}`}
              onChange={() => onAck(ackKey(skill.id, warning))}
            />
          </span>
          <Icon name="alert-triangle" size={12} />
          <span>{`Reviewed this warning — ${backendMessage(warning)}.`}</span>
        </label>
      ))}
    </article>
  );
}

// ---------------------------------------------------------------------------
// Option A — cards kept, chips repaired
// ---------------------------------------------------------------------------

/** The repaired chip: full-row placement (the `.opt-a` rule), payload first —
 * bold lead from `warning.code`, then the paths — and the ack verb as a fixed
 * trailing word that lights when ticked. Accessible name unchanged. */
function AckChip({ lead, paths, message, danger, checked, onToggle }: {
  lead: string;
  paths: string;
  message: string;
  danger?: boolean;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className={`catalog-card-ack${danger ? " danger" : ""}`}>
      <span className="sync-sync-row-choice">
        <input type="checkbox" checked={checked} aria-label={`Reviewed: ${message}`} onChange={onToggle} />
      </span>
      <Icon name="alert-triangle" size={12} />
      <span className="catalog-card-ack-copy"><strong>{lead}</strong> — {paths}</span>
      <small className={checked ? "done" : undefined}>{checked ? "Reviewed" : "Mark reviewed"}</small>
    </label>
  );
}

function OptionACard({ skill, checked, onToggle, acks, onAck }: {
  skill: MockSkill;
  checked: boolean;
  onToggle: () => void;
  acks: Set<string>;
  onAck: (key: string) => void;
}) {
  return (
    <article className="sync-history-thread-card catalog-card">
      <div className="sync-history-thread-topline">
        <div className="sync-history-thread-title">
          <RowChoice name={skill.name} checked={checked} onToggle={onToggle} />
          <Badge status="local-outstanding" tone="local" label={RELATION_LABEL[skill.relation]} />
          <strong title={skill.name}>{skill.name}</strong>
        </div>
        <div className="sync-history-thread-actions">
          <span className="sync-history-launch-action sync-history-provider-inert" title="Codex skill">
            <ProviderMark provider="codex" displayName="Codex" />
          </span>
          <button type="button" className="btn btn-ghost sync-history-launch-action" title={`Reveal ${skill.name} in Finder`}>
            <Icon name="folder" size={14} />
          </button>
        </div>
      </div>
      <div className="sync-history-session-details">
        <div className="sync-history-session-summary">
          <div className="sync-history-thread-meta" aria-label={`${skill.name} details`}>
            <MetricSpan icon="file" value={skill.files} detail={`Files · ${skill.files}`} />
            <MetricSpan icon="drive" value={skill.size} detail={`On disk · ${skill.size}`} />
            {skill.warnings.length > 0
              ? <MetricSpan icon="alert-triangle" value={String(skill.warnings.length)} className="sync-history-partial" detail={skill.warnings.map(backendMessage).join("\n")} />
              : <span className="sync-history-thread-metric" aria-hidden="true" />}
          </div>
          <span className="sync-history-actions-divider" aria-hidden="true" />
          <div className="sync-history-row-clickables">
            <div className="sync-history-debug-menu"><InfoButton name={skill.name} /></div>
          </div>
          <span className="sync-history-thread-metric sync-history-thread-time sync-history-thread-time-end" data-tooltip={`Edited ${skill.stamp}`} tabIndex={0}>
            <time>{skill.stamp}</time>
          </span>
        </div>
      </div>
      {checked && skill.warnings.map((warning) => (
        <AckChip
          key={warning.code}
          lead={WARNING_LEAD[warning.code]}
          paths={warning.paths}
          message={backendMessage(warning)}
          checked={acks.has(ackKey(skill.id, warning))}
          onToggle={() => onAck(ackKey(skill.id, warning))}
        />
      ))}
    </article>
  );
}

// ---------------------------------------------------------------------------
// Options B and C — the review manifest: dense grouped rows
// ---------------------------------------------------------------------------

function GroupHeading({ label, count, onAll, onNone }: {
  label: string;
  count: number;
  onAll?: () => void;
  onNone?: () => void;
}) {
  return (
    <div className="catalog-review-heading">
      <h4>{label}</h4>
      <span className="count">{count}</span>
      <span className="space" />
      {onAll && (
        <>
          <button type="button" className="catalog-review-heading-action" onClick={onAll}>All</button>
          <button type="button" className="catalog-review-heading-action" onClick={onNone}>None</button>
        </>
      )}
    </div>
  );
}

/** The quiet ack line the dense rows use instead of a boxed chip: indented
 * under its row, the project-files review's own register. */
function AckLine({ lead, body, message, danger, checked, onToggle }: {
  lead: string;
  body: string;
  message: string;
  danger?: boolean;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <label className={`catalog-review-ack${danger ? " danger" : ""}`}>
      <span className="sync-sync-row-choice">
        <input type="checkbox" checked={checked} aria-label={`${danger ? "Approved" : "Reviewed"}: ${message}`} onChange={onToggle} />
      </span>
      <Icon name="alert-triangle" size={12} />
      <span className="mock-ack-copy"><strong>{lead}</strong> — {body}</span>
      <small className={checked ? "done" : undefined}>
        {danger ? (checked ? "Approved" : "Approve") : (checked ? "Reviewed" : "Mark reviewed")}
      </small>
    </label>
  );
}

function ManifestRow({ name, status, tone, statusLabel, thumb, pill, warningCount, warningDetail, files, size, date, dateDetail, checked, onToggle, children }: {
  name: string;
  status: ThreadStatusIconName;
  tone: string;
  statusLabel: string;
  thumb?: string;
  pill?: ReactNode;
  warningCount?: number;
  warningDetail?: string;
  files: string;
  size: string;
  date: string;
  dateDetail: string;
  checked: boolean;
  onToggle: () => void;
  children?: ReactNode;
}) {
  return (
    <div className="catalog-review-item">
      <div className="catalog-review-row">
        <RowChoice name={name} checked={checked} onToggle={onToggle} />
        <Badge status={status} tone={tone} label={statusLabel} />
        {thumb && <span className="catalog-review-thumb"><img src={thumb} alt="" /></span>}
        <strong title={name}>{name}</strong>
        {pill}
        <span className="catalog-review-facts">
          {warningCount ? (
            <MetricSpan icon="alert-triangle" value={String(warningCount)} className="warn" detail={warningDetail ?? ""} />
          ) : null}
          <MetricSpan icon="file" value={files} detail={files === "—" ? "Not on this Mac, so nothing was counted" : `Files · ${files}`} />
          <MetricSpan icon="drive" value={size} detail={size === "—" ? "Not on this Mac, so nothing was measured" : `On disk · ${size}`} />
          <InfoButton name={name} />
          <span className="sync-history-thread-metric sync-history-thread-time" data-tooltip={dateDetail} tabIndex={0}>
            <time>{date}</time>
          </span>
        </span>
      </div>
      {children}
    </div>
  );
}

function SkillManifestRow({ skill, checked, onToggle, acks, onAck }: {
  skill: MockSkill;
  checked: boolean;
  onToggle: () => void;
  acks: Set<string>;
  onAck: (key: string) => void;
}) {
  return (
    <ManifestRow
      name={skill.name}
      status="local-outstanding"
      tone="local"
      statusLabel={RELATION_LABEL[skill.relation]}
      warningCount={skill.warnings.length || undefined}
      warningDetail={skill.warnings.map(backendMessage).join("\n")}
      files={skill.files}
      size={skill.size}
      date={skill.date}
      dateDetail={`Edited ${skill.stamp}`}
      checked={checked}
      onToggle={onToggle}
    >
      {checked && skill.warnings.map((warning) => (
        <AckLine
          key={warning.code}
          lead={WARNING_LEAD[warning.code]}
          body={warning.paths}
          message={backendMessage(warning)}
          checked={acks.has(ackKey(skill.id, warning))}
          onToggle={() => onAck(ackKey(skill.id, warning))}
        />
      ))}
    </ManifestRow>
  );
}

// ---------------------------------------------------------------------------
// Panel state: each panel keeps its own selection and acknowledgements
// ---------------------------------------------------------------------------

function usePushState(list: MockSkill[] = SKILLS) {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(list.map((skill) => skill.id)));
  const [acks, setAcks] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  const ack = (key: string) => setAcks((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  });
  const setMany = (ids: string[], on: boolean) => setSelected((current) => {
    const next = new Set(current);
    ids.forEach((id) => (on ? next.add(id) : next.delete(id)));
    return next;
  });
  const pendingAcks = list
    .filter((skill) => selected.has(skill.id))
    .flatMap((skill) => skill.warnings.map((warning) => ackKey(skill.id, warning)))
    .filter((key) => !acks.has(key));
  return { selected, acks, toggle, ack, setMany, setSelected, pendingAcks };
}

function pushFooterNote(pending: number, empty: boolean): ReactNode {
  if (pending > 0) return <span><small>Acknowledge the warnings above before pushing.</small></span>;
  if (empty) return <span><small>Nothing is selected.</small></span>;
  return <span />;
}

// ---------------------------------------------------------------------------
// The panels
// ---------------------------------------------------------------------------

const LEGACY_SKILLS = SKILLS.filter((skill) => skill.warnings.length > 0);

function PanelLegacy() {
  const state = usePushState(LEGACY_SKILLS);
  return (
    <ReviewShell
      title="Push review"
      verb="Push"
      count={state.selected.size}
      total={LEGACY_SKILLS.length}
      onAll={() => state.setSelected(new Set(LEGACY_SKILLS.map((skill) => skill.id)))}
      onNone={() => state.setSelected(new Set())}
      footerNote={pushFooterNote(state.pendingAcks.length, state.selected.size === 0)}
      footerDisabled={state.selected.size === 0 || state.pendingAcks.length > 0}
      buttonStatus="local-outstanding"
    >
      <div className="sync-history-thread-list legacy-repro">
        {LEGACY_SKILLS.map((skill) => (
          <LegacyCard
            key={skill.id}
            skill={skill}
            checked={state.selected.has(skill.id)}
            onToggle={() => state.toggle(skill.id)}
            acks={state.acks}
            onAck={state.ack}
          />
        ))}
      </div>
    </ReviewShell>
  );
}

function PanelOptionA() {
  const state = usePushState();
  return (
    <ReviewShell
      title="Push review"
      verb="Push"
      count={state.selected.size}
      total={SKILLS.length}
      onAll={() => state.setSelected(new Set(SKILLS.map((skill) => skill.id)))}
      onNone={() => state.setSelected(new Set())}
      footerNote={pushFooterNote(state.pendingAcks.length, state.selected.size === 0)}
      footerDisabled={state.selected.size === 0 || state.pendingAcks.length > 0}
      buttonStatus="local-outstanding"
    >
      <div className="sync-history-thread-list">
        {SKILLS.map((skill) => (
          <OptionACard
            key={skill.id}
            skill={skill}
            checked={state.selected.has(skill.id)}
            onToggle={() => state.toggle(skill.id)}
            acks={state.acks}
            onAck={state.ack}
          />
        ))}
      </div>
    </ReviewShell>
  );
}

function PanelOptionB() {
  const state = usePushState();
  return (
    <ReviewShell
      title="Push review"
      verb="Push"
      count={state.selected.size}
      total={SKILLS.length}
      onAll={() => state.setSelected(new Set(SKILLS.map((skill) => skill.id)))}
      onNone={() => state.setSelected(new Set())}
      footerNote={pushFooterNote(state.pendingAcks.length, state.selected.size === 0)}
      footerDisabled={state.selected.size === 0 || state.pendingAcks.length > 0}
      buttonStatus="local-outstanding"
    >
      <div className="catalog-review-list">
        <div className="catalog-review-group">
          <GroupHeading
            label={`New to ${STORAGE}`}
            count={NEW_SKILLS.length}
            onAll={() => state.setMany(NEW_SKILLS.map((skill) => skill.id), true)}
            onNone={() => state.setMany(NEW_SKILLS.map((skill) => skill.id), false)}
          />
          {NEW_SKILLS.map((skill) => (
            <SkillManifestRow key={skill.id} skill={skill} checked={state.selected.has(skill.id)} onToggle={() => state.toggle(skill.id)} acks={state.acks} onAck={state.ack} />
          ))}
        </div>
        <div className="catalog-review-group">
          <GroupHeading
            label="Changed"
            count={CHANGED_SKILLS.length}
            onAll={() => state.setMany(CHANGED_SKILLS.map((skill) => skill.id), true)}
            onNone={() => state.setMany(CHANGED_SKILLS.map((skill) => skill.id), false)}
          />
          {CHANGED_SKILLS.map((skill) => (
            <SkillManifestRow key={skill.id} skill={skill} checked={state.selected.has(skill.id)} onToggle={() => state.toggle(skill.id)} acks={state.acks} onAck={state.ack} />
          ))}
        </div>
      </div>
    </ReviewShell>
  );
}

function PetRows() {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(["maple", "dahuang"]));
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  return (
    <div className="catalog-review-list">
      <div className="catalog-review-group">
        <GroupHeading label={`New to ${STORAGE}`} count={2} />
        <ManifestRow
          name="Maple"
          status="local-outstanding"
          tone="local"
          statusLabel={RELATION_LABEL.new}
          thumb={PET_THUMBS.maple}
          files="1"
          size="2.5 MB"
          date="Jul 11, 2026"
          dateDetail="Edited Jul 11, 2026 at 00:55"
          checked={selected.has("maple")}
          onToggle={() => toggle("maple")}
        />
        <ManifestRow
          name="大黄"
          status="local-outstanding"
          tone="local"
          statusLabel={RELATION_LABEL.new}
          thumb={PET_THUMBS.dahuang}
          files="1"
          size="1.8 MB"
          date="Jun 2, 2026"
          dateDetail="Edited Jun 2, 2026 at 18:20"
          checked={selected.has("dahuang")}
          onToggle={() => toggle("dahuang")}
        />
      </div>
    </div>
  );
}

function PanelOptionC() {
  const state = usePushState();
  const riskyAckTotal = RISKY_SKILLS
    .filter((skill) => state.selected.has(skill.id))
    .reduce((sum, skill) => sum + skill.warnings.length, 0);
  const riskyAcked = riskyAckTotal - state.pendingAcks.length;
  return (
    <ReviewShell
      title="Push review"
      verb="Push"
      count={state.selected.size}
      total={SKILLS.length}
      onAll={() => state.setSelected(new Set(SKILLS.map((skill) => skill.id)))}
      onNone={() => state.setSelected(new Set())}
      footerNote={riskyAckTotal > 0
        ? <span><small>{`${riskyAcked} of ${riskyAckTotal} warnings reviewed.`}</small></span>
        : <span />}
      footerDisabled={state.selected.size === 0 || state.pendingAcks.length > 0}
      buttonStatus="local-outstanding"
    >
      <div className="catalog-review-list">
        <div className="catalog-review-group">
          <GroupHeading
            label="Needs your review"
            count={RISKY_SKILLS.length}
            onAll={() => state.setMany(RISKY_SKILLS.map((skill) => skill.id), true)}
            onNone={() => state.setMany(RISKY_SKILLS.map((skill) => skill.id), false)}
          />
          {RISKY_SKILLS.map((skill) => (
            <SkillManifestRow key={skill.id} skill={skill} checked={state.selected.has(skill.id)} onToggle={() => state.toggle(skill.id)} acks={state.acks} onAck={state.ack} />
          ))}
        </div>
      </div>
      <details className="sync-sync-review-other">
        <summary>
          <span>
            <Icon name="check-circle" size={14} />
            <strong>Ready to push · {CLEAN_SKILLS.length}</strong>
            <small className="mock-fold-note">no findings, included</small>
          </span>
          <Icon name="chevron-right" size={13} />
        </summary>
        <div className="catalog-review-list">
          <div className="catalog-review-group">
            {CLEAN_SKILLS.map((skill) => (
              <SkillManifestRow key={skill.id} skill={skill} checked={state.selected.has(skill.id)} onToggle={() => state.toggle(skill.id)} acks={state.acks} onAck={state.ack} />
            ))}
          </div>
        </div>
      </details>
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// The Pull review, in the manifest grammar
// ---------------------------------------------------------------------------

function PanelPull() {
  const ALL_PULL = ["release-notes", "deploy-checklist", "sql-review", "api-designer", "speech-synthesis"];
  const [selected, setSelected] = useState<Set<string>>(() => new Set(ALL_PULL));
  const [approved, setApproved] = useState(false);
  const toggle = (id: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });
  const blocked = selected.has("speech-synthesis") && !approved;
  return (
    <ReviewShell
      title="Pull review"
      verb="Pull"
      count={selected.size}
      total={ALL_PULL.length}
      onAll={() => setSelected(new Set(ALL_PULL))}
      onNone={() => setSelected(new Set())}
      footerNote={blocked
        ? <span role="status"><small>Acknowledge the overwrite above before pulling.</small></span>
        : <span />}
      footerDisabled={selected.size === 0 || blocked}
      buttonStatus="cloud-outstanding"
      className="sync-inline-pull-review"
    >
      <div className="sync-review-banners">
        <NoticeZone
          timing="standing"
          label="Notes about this Pull review"
          subject="skills claimed elsewhere"
          items={[{
            key: "api-designer",
            severity: "warning",
            title: "api-designer is claimed by Attic",
            message: "A different version of this local target is pinned there. Preserve a copy, acknowledge overwrite, or Skip.",
          }]}
        />
      </div>
      <div className="catalog-review-list">
        <div className="catalog-review-group">
          <GroupHeading label={`Changed in ${STORAGE}`} count={2} />
          <ManifestRow
            name="release-notes"
            status="cloud-outstanding"
            tone="storage"
            statusLabel={`Newer in ${STORAGE}. Pull to update this computer.`}
            pill={<span className="catalog-row-action" title="Update from storage">Update</span>}
            files="3"
            size="9.1 KB"
            date="Apr 2, 2026"
            dateDetail="Edited Apr 2, 2026 at 08:15"
            checked={selected.has("release-notes")}
            onToggle={() => toggle("release-notes")}
          />
          <ManifestRow
            name="deploy-checklist"
            status="pending"
            tone="pending"
            statusLabel={`Held until you Pull: ${STORAGE} has a version this computer has never reviewed.`}
            pill={<span className="catalog-row-action" title="Update from storage">Update</span>}
            files="4"
            size="11 KB"
            date="Mar 28, 2026"
            dateDetail="Edited Mar 28, 2026 at 19:02"
            checked={selected.has("deploy-checklist")}
            onToggle={() => toggle("deploy-checklist")}
          />
        </div>
        <div className="catalog-review-group">
          <GroupHeading label={`Only in ${STORAGE}`} count={1} />
          <ManifestRow
            name="sql-review"
            status="cloud-outstanding"
            tone="storage"
            statusLabel={`Only in ${STORAGE}. Pull to download it here.`}
            pill={<span className="catalog-row-action" title="Install stored content">Install</span>}
            files="—"
            size="—"
            date="Feb 14, 2026"
            dateDetail="Stored Feb 14, 2026 at 10:44"
            checked={selected.has("sql-review")}
            onToggle={() => toggle("sql-review")}
          />
        </div>
        <div className="catalog-review-group">
          <GroupHeading label="Changed on both sides" count={2} />
          <ManifestRow
            name="api-designer"
            status="divergent"
            tone="diverged"
            statusLabel={`Changed here and in ${STORAGE}. Review before syncing.`}
            pill={(
              <span className="catalog-row-action-shell">
                <button type="button" className="catalog-row-action is-choice" aria-expanded={false} aria-haspopup="menu" title="Keep local as api-designer-copy and overwrite — choose a different local action">
                  Keep copy
                  <Icon name="chevron-down" size={10} />
                </button>
              </span>
            )}
            files="4"
            size="21.4 KB"
            date="Aug 19, 2026"
            dateDetail="Edited Aug 19, 2026 at 14:03"
            checked={selected.has("api-designer")}
            onToggle={() => toggle("api-designer")}
          />
          <ManifestRow
            name="speech-synthesis"
            status="divergent"
            tone="diverged"
            statusLabel={`Changed here and in ${STORAGE}. Review before syncing.`}
            pill={(
              <span className="catalog-row-action-shell">
                <button type="button" className="catalog-row-action is-choice destructive" aria-expanded={false} aria-haspopup="menu" title="Overwrite local content — choose a different local action">
                  Overwrite
                  <Icon name="chevron-down" size={10} />
                </button>
              </span>
            )}
            files="16"
            size="48 KB"
            date="Mar 31, 2026"
            dateDetail="Edited Mar 31, 2026 at 17:24"
            checked={selected.has("speech-synthesis")}
            onToggle={() => toggle("speech-synthesis")}
          >
            {selected.has("speech-synthesis") && (
              <AckLine
                lead="Overwrites this Mac’s copy"
                body="replaces the current speech-synthesis here with the stored version"
                message="I understand this replaces the current speech-synthesis on this Mac."
                danger
                checked={approved}
                onToggle={() => setApproved((current) => !current)}
              />
            )}
          </ManifestRow>
        </div>
      </div>
    </ReviewShell>
  );
}

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage tight">{children}</div>
    </section>
  );
}

function Harness() {
  const [dark, setDark] = useState(
    () => document.documentElement.getAttribute("data-theme") === "dark",
  );
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Push and Pull review — three options</h1>
            <p>
              The shipped review reuses the list’s inventory card per row. Its warning chips are
              auto-placed grid children: on a card with two warnings the second chip falls into
              the template’s auto-sized actions column, inflates it to the chip’s whole sentence,
              truncates the title and collides the chips — the screenshot. And at 37 rows the
              card itself is the wrong grain: provider mark ×37, Reveal, full timestamps, boxed
              chips, no grouping. Every option below fixes the chips; they differ in how far the
              row goes toward a review manifest — the register Git clients and file-sync reviews
              settled on: dense status-first rows, grouped, with the count on the commit button.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel
          title="What this fixed — the retired review, reproduced"
          note={<>
            The pre-repair review, shipped classes with the retired auto-placement restored by
            a scoped override (App.css pins the span now) — only the three rows that carry
            warnings. On <code>sentry</code> and <code>speech-synthesis</code> the second chip
            auto-places into the card grid’s <code>auto</code> actions column and inflates it;
            the title track collapses and the chips shear.
          </>}
        >
          <PanelLegacy />
        </Panel>

        <Panel
          title="Option A — cards kept, chips repaired (SHIPPED)"
          note={<>
            The smallest change: the review stays the list’s card, and the chips get the fork
            drawer’s cure — an explicit <code>grid-column: 1 / -1</code>, one full row each,
            never in the actions track. Chip copy turns payload-first: bold lead derived
            from <code>warning.code</code>, then the flagged paths, with the ack verb as the
            chip’s trailing word instead of a 24-character preamble before every sentence.
            The Push button carries the count. Familiar, zero new vocabulary — but 37 rows
            still stand this tall, ungrouped, with a provider mark and a Reveal on every one.
          </>}
        >
          <PanelOptionA />
        </Panel>

        <Panel
          title="Option B — the review manifest"
          note={<>
            The review stops borrowing the inventory card and gets its own grain: one 36px row —
            checkbox, status badge, name, then right-packed facts (warnings, files, size, ⓘ,
            date) — grouped by what the push does (<em>New to Vault</em> / <em>Changed</em>),
            each group with All · None. Provider mark, Reveal and Fork stay on the list page;
            the ⓘ keeps paths and digests one click away. Warning rows keep a ⚠ count even when
            excluded, and grow quiet indented ack lines while included — the project-files
            review’s register, boxes only where the page banner speaks. Seven rows here; the
            same frame holds 37 without changing shape.
          </>}
        >
          <PanelOptionB />
          <div className="mockup-substage">
            <p>Pet scopes keep their artwork — the tile shrinks to the line:</p>
            <PetRows />
          </div>
        </Panel>

        <Panel
          title="Option C — decisions first"
          note={<>
            Option B’s rows, partitioned by decision weight instead of by relation: everything
            carrying a finding stands at the top under <em>Needs your review</em>, ack lines
            open; the clean bulk folds into one summary row — the app’s own review fold — that
            expands to the same manifest. The footer counts the gate down. A 37-row push becomes
            the three decisions it actually contains. The trade: rows leave list order, and the
            clean majority is a sentence until expanded.
          </>}
        >
          <PanelOptionC />
        </Panel>

        <Panel
          title="Pull, in the same grammar"
          note={<>
            Whichever option wins, Pull wears it too — shown here on B’s rows. The local-action
            pill keeps its shipped anatomy: a readout when the plan offers one action, the ghost
            chevron button and its existing menu when there is a real choice, red while the
            choice is destructive — and the overwrite acknowledgement is the same ack line in
            its danger register. Claims stay one banner above the list, exactly as shipped.
          </>}
        >
          <PanelPull />
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles: the harness shell, then the proposed vocabulary. Everything below
// `PROPOSED_CSS` is the change under review; shipped classes are untouched so
// the reproduction panel keeps its bug. Option A's repair is scoped to `.opt-a`.
// ---------------------------------------------------------------------------

const SHELL_CSS = `
/* The app chain is height-locked; a mockup is a document and must scroll. */
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }

.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 82ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); }
.mockup-stage.tight { padding: 0; overflow: hidden; }
.mockup-substage { padding: 12px 16px 16px; border-top: 1px solid var(--border); }
.mockup-substage > p { margin: 0 0 8px; color: var(--text-2); font-size: var(--type-xs); }
body { background: var(--app-shell); }

/* The app page owns scrolling, not the panel; in the harness neither should. */
.mock-review .sync-sync-review-scroll { max-height: none; overflow: visible; scrollbar-gutter: auto; }
`;

const PROPOSED_CSS = `
/* ── Option A SHIPPED ────────────────────────────────────────────────────────
   The chip's span, payload-first copy and trailing ack verb live in App.css
   now (.catalog-card-ack / .catalog-card-ack-copy), which this harness
   imports. The reproduction panel restores the retired auto-placement so it
   keeps showing what was replaced. */
.legacy-repro .catalog-card-ack { grid-column: auto; }

/* The declined manifest options keep their own copy styles. */
.mock-ack-copy { min-width: 0; }
.mock-ack-copy strong { font-weight: var(--weight-semibold); color: inherit; }
.catalog-review-ack > small {
  flex: 0 0 auto;
  margin-left: 4px;
  color: var(--text-2);
  font-size: var(--type-2xs);
  font-weight: var(--weight-semibold);
  white-space: nowrap;
}
.catalog-review-ack > small.done { color: inherit; }

/* ── The review manifest (Options B and C, and Pull) ──────────────────────────
   One bordered list; groups separated by the stronger hairline, rows by the
   lighter one — the commit card's own hierarchy. */
.catalog-review-list {
  min-width: 0;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--surface-row);
}

.catalog-review-heading {
  min-height: 31px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 2px 12px;
  border-bottom: 1px solid var(--border);
}

.catalog-review-heading > h4 {
  margin: 0;
  color: var(--text-1);
  font-size: var(--type-xs);
  font-weight: var(--weight-semibold);
}

.catalog-review-heading > .count {
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--type-2xs);
  font-weight: var(--weight-semibold);
}

.catalog-review-heading > .space { flex: 1; }

.catalog-review-heading-action {
  padding: 2px 5px;
  border: 0;
  border-radius: 4px;
  color: var(--text-3);
  background: transparent;
  font: inherit;
  font-size: var(--type-2xs);
  font-weight: var(--weight-medium);
  cursor: pointer;
}

.catalog-review-heading-action:hover {
  color: var(--text-0);
  background: var(--button-bg-hover);
}

.catalog-review-group + .catalog-review-group { border-top: 1px solid var(--border-mid); }
.catalog-review-item + .catalog-review-item { border-top: 1px solid var(--border); }

.catalog-review-row {
  min-height: 36px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 12px;
}

.catalog-review-row > strong {
  min-width: 0;
  overflow: hidden;
  color: var(--text-0);
  font-size: var(--type-sm);
  font-weight: var(--weight-semibold);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.catalog-review-facts {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 10px;
  margin-left: auto;
}

.catalog-review-facts .warn,
.catalog-review-facts .warn > b { color: var(--warning-text); }

.catalog-review-facts > .sync-history-thread-time > time {
  display: inline-block;
  min-width: 84px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* The name outranks the date. The facts cluster never shrinks, so in a narrow
   panel the name would truncate first; below the workspace's own container
   threshold the date steps out instead — its exact stamp stays in the ⓘ. */
@container sync-review-layout (max-width: 860px) {
  .catalog-review-facts > .sync-history-thread-time { display: none; }
}

/* The pet tile, shrunk to the line — same border, paper and letterboxing as
   the card's 44px tile. */
.catalog-review-thumb {
  width: 26px;
  height: 28px;
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 5px;
  background: var(--bg-2);
  color: var(--text-3);
}

.catalog-review-thumb > img { width: 100%; height: 100%; object-fit: contain; }

/* The quiet ack line: indented under its row into the badge column, tint-free
   — the severity lives in the ink and the glyph, the way the project-files
   review writes its per-row markers. */
.catalog-review-ack {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 28px;
  padding: 1px 12px 3px 44px;
  color: var(--warning-text);
  font-size: var(--type-2xs);
  cursor: pointer;
}

.catalog-review-ack.danger { color: var(--danger-text); }
.catalog-review-ack > svg { flex: 0 0 auto; }
.catalog-review-ack > .sync-sync-row-choice { width: 24px; min-width: 24px; min-height: 24px; }
.catalog-review-item > .catalog-review-ack:last-child { padding-bottom: 6px; }

/* Option C's fold holds a manifest of its own. */
.sync-sync-review-other > .catalog-review-list { margin: 4px 12px 12px; }
.mock-fold-note { color: var(--text-3); font-size: var(--type-2xs); }
`;

/* ?theme=dark so a headless capture can take the dark panels without clicking. */
const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
