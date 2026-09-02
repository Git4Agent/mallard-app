import type { ReactNode } from "react";
import Icon, { type IconName } from "../Icons";
import type { NoticeSeverity } from "../../types";

/* One notice recipe for the whole app.
 *
 * Three published rules are built into the types rather than left to review:
 *
 *  - A zone shows ONE box, never a stack. `NoticeZone` takes every message a
 *    zone holds and renders a single box at the highest severity present.
 *    (GOV.UK notification banner: combine, never stack.)
 *  - The severity names itself in the text — "Error ·", "Warning ·", "Note ·",
 *    "Done ·" — so the meaning survives without the tint and without the
 *    glyph. (USWDS alert; WCAG 1.4.1.)
 *  - The ARIA role is DERIVED from `timing`, never authored. Content that is
 *    already there when the view renders is not a status message, so it gets a
 *    named region instead of a live region. The union below makes `standing`
 *    unbuildable without a label. (WCAG 2.2 SC 4.1.3.)
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

const SEVERITY_LABEL: Record<NoticeSeverity, string> = {
  danger: "Error",
  warning: "Warning",
  info: "Note",
  success: "Done",
};

const SEVERITY_ICON: Record<NoticeSeverity, IconName> = {
  danger: "alert-triangle",
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
  /** The one control this message offers, if any. */
  action?: ReactNode;
  /** A glyph that says more than the severity does — a thread's status mark,
   * say, tying a banner to the rows it speaks for. It stands in for the
   * severity icon, which is safe precisely because the rung is named in the
   * text and never rested on the glyph. */
  mark?: ReactNode;
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
};

/** One message. The type name leads the sentence. */
export default function Notice({ severity, title, children, action, mark, className, ...timing }: NoticeProps) {
  return (
    <NoticeShell severity={severity} className={className} action={action} mark={mark} {...timing}>
      <p className="notice-line">
        <strong className="notice-type">{SEVERITY_LABEL[severity]} ·</strong>{" "}
        {title ? <strong className="notice-lead">{title}</strong> : null}
        {title ? " " : null}
        {children}
      </p>
    </NoticeShell>
  );
}

export type NoticeZoneProps = NoticeTiming & {
  items: readonly NoticeItem[];
  /** Plural noun for the count line, e.g. "notes about this record". */
  subject: string;
  className?: string;
};

/** Every message a zone holds, in one box at the loudest rung present. */
export function NoticeZone({ items, subject, className, ...timing }: NoticeZoneProps) {
  const severity = highestSeverity(items.map((item) => item.severity));
  if (severity === null) return null;

  if (items.length === 1) {
    const only = items[0];
    return (
      <Notice
        severity={only.severity}
        title={only.title}
        action={only.action}
        mark={only.mark}
        className={className}
        {...timing}
      >
        {only.message}
      </Notice>
    );
  }

  /* The box wears one tint, so in a mixed zone the ink would say "error" over a
   * note about how the cache was built. Each message names its own rung
   * instead; a zone where every message agrees needs no such repetition,
   * because the title already said it once. */
  const mixed = items.some((item) => item.severity !== severity);

  return (
    <NoticeShell severity={severity} className={className} {...timing}>
      <p className="notice-line">
        <strong className="notice-type">{SEVERITY_LABEL[severity]} ·</strong>{" "}
        <strong className="notice-lead">{`${items.length} ${subject}`}</strong>
      </p>
      <ul className="notice-list">
        {items.map((item) => (
          <li key={item.key}>
            {/* One box means one leading glyph, so an item's own mark rides its
                own line instead. */}
            {item.mark}
            {mixed ? <strong className="notice-type">{SEVERITY_LABEL[item.severity]} ·</strong> : null}
            {mixed ? " " : null}
            {item.title ? <strong className="notice-lead">{item.title}</strong> : null}
            {item.title ? " " : null}
            {item.message}
            {item.action}
          </li>
        ))}
      </ul>
    </NoticeShell>
  );
}
