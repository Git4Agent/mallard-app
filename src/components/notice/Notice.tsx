import { Children, isValidElement, useEffect, useState, type ReactNode } from "react";
import Icon, { type IconName } from "../Icons";
import type { NoticeSeverity } from "../../types";
import { t, useTranslation } from "../../i18n";

/* One notice recipe for the whole app.
 *
 * Four published rules are built into the types rather than left to review:
 *
 *  - A zone shows ONE box, never a stack. `NoticeZone` takes every message a
 *    zone holds and renders a single box at the highest severity present.
 *    (GOV.UK notification banner: combine, never stack.)
 *  - The severity names itself in the text — "Error ·", "Warning ·", "Note ·",
 *    "Done ·" — so the meaning survives without the tint and without the
 *    glyph. (USWDS alert; WCAG 1.4.1.) Each rung also has its own glyph shape:
 *    error is the octagon, never warning's triangle.
 *  - The ARIA role is DERIVED from `timing`, never authored. Content that is
 *    already there when the view renders is not a status message, so it gets a
 *    named region instead of a live region. The union below makes `standing`
 *    unbuildable without a label. (WCAG 2.2 SC 4.1.3.)
 *  - Every message closes, through one control, `DismissButton`: last in the
 *    message, after any action, called "Dismiss". Closing hides the words,
 *    never the condition, so a closed message comes back when it changes or
 *    when its view is mounted again. (Fluent 2 message bar.) The one message
 *    that never closes is an `Acknowledgement`: the flow waits on its tick.
 */

/* The severity ladder itself lives in ../../types beside the rest of the
 * wire vocabulary — Rust sets it, so it is not the component's to define. */

/** Rung order. `danger` wins a zone; `success` yields to everything. */
const SEVERITY_RANK: Record<NoticeSeverity, number> = {
  danger: 3,
  warning: 2,
  info: 1,
  success: 0,
};

/** The rung's own word, in the interface language: "Error", "Warning",
 *  "Note", "Done". */
export function severityLabel(severity: NoticeSeverity): string {
  return t(`notice:severity.${severity}`);
}

export const SEVERITY_ICON: Record<NoticeSeverity, IconName> = {
  danger: "octagon-x",
  warning: "alert-triangle",
  info: "info",
  success: "check-circle",
};

/** The loudest rung in a set, or `null` when the set is empty. */
export function highestSeverity(severities: readonly NoticeSeverity[]): NoticeSeverity | null {
  let winner: NoticeSeverity | null = null;
  for (const severity of severities) {
    if (winner === null || SEVERITY_RANK[severity] > SEVERITY_RANK[winner]) winner = severity;
  }
  return winner;
}

/* `standing` is content the view renders with. It is not an event, so it must
 * not announce; it becomes a named region instead. `progress` and `blocking`
 * are things that HAPPEN — they announce politely and assertively.
 *
 * `inline` is the fourth case and the one to justify before reaching for: no
 * role at all, for a box whose meaning is already carried by something that
 * encloses it. One shape needs it: a marker that repeats per row would
 * otherwise mint one landmark per row — a file list with thirty conflicts
 * would hand a screen-reader user thirty regions called "Pull required", which
 * is worse than none. It is not standing content that needs finding; it is
 * read in place. A screen that IS the failure does not qualify: it replaces
 * the startup loader after mount, so the failure is an event, and its notice
 * is `blocking` so the change is spoken. */
export type NoticeTiming =
  | { timing: "standing"; label: string }
  | { timing: "progress" | "blocking" | "inline"; label?: undefined };

function liveRegion(timing: NoticeTiming) {
  if (timing.timing === "blocking") return { role: "alert" } as const;
  if (timing.timing === "progress") return { role: "status" } as const;
  if (timing.timing === "inline") return {} as const;
  return { role: "region", "aria-label": timing.label } as const;
}

export interface NoticeItem {
  key: string;
  severity: NoticeSeverity;
  /** A short lead, printed bold before the sentence. */
  title?: ReactNode;
  message: ReactNode;
  /** The one control this message offers, if any. The close control is not
   * one of these: the zone draws it after the action, for every message. */
  action?: ReactNode;
  /** Extra condition identity when the same text can describe a new
   * occurrence — an attempt counter, say, so a retry that fails the same way
   * is shown again rather than staying closed. */
  dismissalId?: string;
  /** The owner's half of closing, when the message must stay closed past this
   * view (a failure the owner holds in state). Closing never runs the action. */
  onDismiss?: () => void;
  /** A glyph that says more than the severity does — a thread's status mark,
   * say, tying a banner to the rows it speaks for. It stands in for the
   * severity icon, which is safe precisely because the rung is named in the
   * text and never rested on the glyph. */
  mark?: ReactNode;
}

function noticeText(node: ReactNode): string {
  return Children.toArray(node).map((child): string => {
    if (typeof child === "string" || typeof child === "number") return String(child);
    if (isValidElement<{ children?: ReactNode }>(child)) return noticeText(child.props.children);
    return "";
  }).join("");
}

/** The closed messages still worth remembering: a closed message that is no
 * longer present has resolved, so it is forgotten, and if it occurs again it
 * is shown again. Returns `closed` itself when nothing was forgotten. */
export function forgetResolved(
  closed: ReadonlySet<string>,
  items: readonly NoticeItem[],
): ReadonlySet<string> {
  const present = new Set(items.map(noticeIdentity));
  const remaining = new Set([...closed].filter((id) => present.has(id)));
  return remaining.size === closed.size ? closed : remaining;
}

export function noticeIdentity(item: NoticeItem): string {
  return JSON.stringify([item.key, item.dismissalId, noticeText(item.title), noticeText(item.message)]);
}

function useVisibleNotices(items: readonly NoticeItem[]) {
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(() => new Set());
  const active = JSON.stringify(items.map(noticeIdentity));
  // A resolved message must be visible if it occurs again in this view.
  useEffect(() => {
    // Keyed on `active`, the items' identity: the array is new every render.
    setDismissed((previous) => forgetResolved(previous, items));
  }, [active]);
  return {
    visible: items.filter((item) => !dismissed.has(noticeIdentity(item))),
    dismiss(closing: readonly NoticeItem[]) {
      setDismissed((previous) => new Set([...previous, ...closing.map(noticeIdentity)]));
      for (const item of closing) item.onDismiss?.();
    },
  };
}

/** The one close control, for every message in the app. `what` completes the
 * accessible name: "Dismiss: <the message>". */
export function DismissButton({ what, onClick, className }: {
  what: string;
  onClick: () => void;
  className?: string;
}) {
  const { t } = useTranslation("notice");
  return (
    <button
      type="button"
      className={`sync-notice-dismiss${className ? ` ${className}` : ""}`}
      title={t("dismiss")}
      aria-label={t("dismissWhat", { what })}
      onClick={onClick}
    >
      <Icon name="x" size={12} />
    </button>
  );
}

/* A part that already ends as a sentence, in either language's stops. A
   full-width stop carries its own space; after any other the next part
   follows a space, as an English sentence does inside a Chinese line. */
const SENTENCE_END = /[.!?…:。！？：]$/;
const FULL_WIDTH_END = /[。！？：]$/;

/** The message as one spoken line for its close control's name: each part
 * ends as a sentence, so "Scan incomplete." is not read with a second stop.
 * A part that does not end as one takes the language's own stop. */
function itemText(item: NoticeItem): string {
  return [severityLabel(item.severity), noticeText(item.title), noticeText(item.message)]
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => (SENTENCE_END.test(part) ? part : t("notice:spoken.stop", { part })))
    .reduce((line, part) => (line ? `${line}${FULL_WIDTH_END.test(line) ? "" : " "}${part}` : part), "");
}

type NoticeShellProps = NoticeTiming & {
  severity: NoticeSeverity;
  className?: string;
  children: ReactNode;
  /** Trailing controls for the box as a whole. */
  action?: ReactNode;
  mark?: ReactNode;
};

function NoticeShell({ severity, className, children, action, mark, ...timing }: NoticeShellProps) {
  return (
    <div className={`notice ${severity}${className ? ` ${className}` : ""}`} {...liveRegion(timing)}>
      {mark ?? <Icon name={SEVERITY_ICON[severity]} size={13} className="notice-mark" />}
      <div className="notice-body">{children}</div>
      {action}
    </div>
  );
}

export type NoticeProps = NoticeTiming & {
  severity: NoticeSeverity;
  /** A short lead, printed bold after the type name. */
  title?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  mark?: ReactNode;
  className?: string;
  dismissalId?: string;
  onDismiss?: () => void;
};

/** One message. The type name leads the sentence. */
export default function Notice({ dismissalId, onDismiss, ...props }: NoticeProps) {
  const item: NoticeItem = {
    key: "notice",
    severity: props.severity,
    title: props.title,
    message: props.children,
    dismissalId,
    onDismiss,
  };
  const { visible, dismiss } = useVisibleNotices([item]);
  if (visible.length === 0) return null;
  return (
    <NoticeContent
      {...props}
      action={(
        <>
          {props.action}
          <DismissButton what={itemText(item)} onClick={() => dismiss([item])} />
        </>
      )}
    />
  );
}

function NoticeContent({ severity, title, children, action, mark, className, ...timing }: NoticeProps) {
  return (
    <NoticeShell severity={severity} className={className} action={action} mark={mark} {...timing}>
      <p className="notice-line">
        <strong className="notice-type">{severityLabel(severity)} ·</strong>{" "}
        {title ? <strong className="notice-lead">{title}</strong> : null}
        {title ? " " : null}
        {children}
      </p>
    </NoticeShell>
  );
}

export type NoticeZoneProps = NoticeTiming & {
  items: readonly NoticeItem[];
  /** What the count line counts, after its number: "notes about this
   *  record"; in Chinese it opens on its measure word, "条关于…的提示". */
  subject: string;
  className?: string;
};

/** Every message a zone holds, in one box at the loudest rung present. */
export function NoticeZone({ items: allItems, subject, className, ...timing }: NoticeZoneProps) {
  const { t } = useTranslation("notice");
  const { visible: items, dismiss } = useVisibleNotices(allItems);
  const severity = highestSeverity(items.map((item) => item.severity));
  if (severity === null) return null;

  if (items.length === 1) {
    const only = items[0];
    return (
      <NoticeContent
        severity={only.severity}
        title={only.title}
        action={(
          <>
            {only.action}
            <DismissButton what={itemText(only)} onClick={() => dismiss([only])} />
          </>
        )}
        mark={only.mark}
        className={className}
        {...timing}
      >
        {only.message}
      </NoticeContent>
    );
  }

  /* The box wears one tint, so in a mixed zone the ink would say "error" over a
   * note about how the cache was built. Each message names its own rung
   * instead; a zone where every message agrees needs no such repetition,
   * because the title already said it once. */
  const mixed = items.some((item) => item.severity !== severity);

  return (
    <NoticeShell
      severity={severity}
      className={className}
      action={<DismissButton what={t("zone.all", { count: items.length, subject })} onClick={() => dismiss(items)} />}
      {...timing}
    >
      <p className="notice-line">
        <strong className="notice-type">{severityLabel(severity)} ·</strong>{" "}
        <strong className="notice-lead">{t("zone.count", { count: items.length, subject })}</strong>
      </p>
      <ul className="notice-list">
        {items.map((item) => (
          <li key={item.key}>
            {/* One box means one leading glyph, so an item's own mark rides its
                own line instead. */}
            {item.mark}
            {mixed ? <strong className="notice-type">{severityLabel(item.severity)} ·</strong> : null}
            {mixed ? " " : null}
            {item.title ? <strong className="notice-lead">{item.title}</strong> : null}
            {item.title ? " " : null}
            {item.message}
            {item.action}
            <DismissButton what={itemText(item)} onClick={() => dismiss([item])} />
          </li>
        ))}
      </ul>
    </NoticeShell>
  );
}

/** A warning the flow waits on: the box is the checkbox's label, and it never
 * closes — ticking it is the only way past it. `verbs` is the trailing word,
 * unticked then ticked ("Mark reviewed" → "Reviewed"). */
export function Acknowledgement({ checked, onChange, disabled, title, children, verbs, ariaLabel, className }: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  title?: ReactNode;
  children: ReactNode;
  verbs?: readonly [string, string];
  /** The whole sentence when the box shows a shortened one. */
  ariaLabel?: string;
  className?: string;
}) {
  return (
    <label className={`notice warning notice-ack${className ? ` ${className}` : ""}`}>
      <span className="sync-sync-row-choice notice-ack-choice">
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          aria-label={ariaLabel}
          onChange={(event) => onChange(event.target.checked)}
        />
      </span>
      <Icon name={SEVERITY_ICON.warning} size={13} className="notice-mark" />
      <span className="notice-body">
        <span className="notice-line">
          <strong className="notice-type">{severityLabel("warning")} ·</strong>{" "}
          {title ? <strong className="notice-lead">{title}</strong> : null}
          {title ? " " : null}
          {children}
        </span>
      </span>
      {verbs && <small className={`notice-ack-verb${checked ? " done" : ""}`}>{checked ? verbs[1] : verbs[0]}</small>}
    </label>
  );
}

/** The message under an invalid field: no box, the error's glyph and ink, a
 * spoken "Error:" before it, and the same close control as every other
 * message. Closing hides the sentence, never the field's invalid state; a new
 * sentence shows again. */
export function FieldMessage({ id, blocking, children }: {
  /** The id the field's `aria-describedby` points at. */
  id?: string;
  /** Announce it (`role="alert"`) when it arrives after an action. */
  blocking?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation("notice");
  const text = noticeText(children);
  const [closed, setClosed] = useState<string | null>(null);
  if (closed === text) return null;
  return (
    <div id={id} className="notice-field" role={blocking ? "alert" : undefined}>
      <Icon name={SEVERITY_ICON.danger} size={13} />
      <span><span className="sync-visually-hidden">{t("field.spoken")}</span>{children}</span>
      <DismissButton what={t("field.dismiss", { text })} onClick={() => setClosed(text)} />
    </div>
  );
}

/** A row's or a bar's status in one word or a count: a pill in the rung's
 * tint. The word names the state ("Reconnect", "Possible secret"), not the
 * rung. */
export function NoticeChip({ severity, children, className }: {
  severity: NoticeSeverity;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span className={`notice-chip ${severity}${className ? ` ${className}` : ""}`}>
      <Icon name={SEVERITY_ICON[severity]} size={12} aria-hidden="true" />
      {children}
    </span>
  );
}
