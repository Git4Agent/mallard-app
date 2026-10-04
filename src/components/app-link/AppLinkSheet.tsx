import { useEffect, useId, useRef } from "react";
import Icon, { type IconName } from "../Icons";
import type { LinkStop, LinkVerdict } from "./appLink";

/**
 * The sheet every `mallard://` link opens. Its title is what the link names;
 * under it, two lines in a glyph and a word or two each: what Mallard Cloud
 * says about it, and where it stands on this computer. A link that goes
 * nowhere titles the sheet with why. One verb; nothing moves until it is
 * pressed, and closing the sheet drops the link.
 */

export type AppLinkSheetState =
  | { phase: "looking" }
  | { phase: "answered"; verdict: LinkVerdict };

const STOP_VERB: Record<NonNullable<LinkStop["action"]>, string> = {
  set_up: "Set up Mallard Cloud",
  retry: "Try again",
};

function Mark({ icon, tone, large = false }: { icon: IconName | null; tone?: string; large?: boolean }) {
  return (
    <span className={`app-link-mark${large ? " large" : ""}${tone ? ` ${tone}` : ""}`} aria-hidden="true">
      {icon ? <Icon name={icon} size={large ? 18 : 14} /> : <span className="status-loader" />}
    </span>
  );
}

export default function AppLinkSheet({ subject, state, onVerb, onClose }: {
  /** `owner/name` as the link spelled it; null for a link that could not be read. */
  subject: string | null;
  state: AppLinkSheetState;
  /** The verb was pressed: go where the verdict says. */
  onVerb: (verdict: LinkVerdict) => void;
  onClose: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const verbRef = useRef<HTMLButtonElement>(null);
  const exitRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const reasonId = useId();
  useEffect(() => { dialogRef.current?.showModal(); }, []);
  /* `close()` hands focus back; the link is let go here rather than on the
     dialog's own close event, which also covers Esc and may arrive late. */
  const close = () => {
    dialogRef.current?.close();
    onClose();
  };

  const verdict = state.phase === "answered" ? state.verdict : null;
  const verb = !verdict ? null : verdict.kind === "landing" ? verdict.verb : verdict.action ? STOP_VERB[verdict.action] : null;
  // Return presses the verb once there is one; until then, the exit.
  useEffect(() => { (verbRef.current ?? exitRef.current)?.focus(); }, [verb]);

  const stop = verdict?.kind === "stop" ? verdict : null;
  const landing = verdict?.kind === "landing" ? verdict : null;
  const title = stop ? stop.title : landing ? landing.found.address : subject ?? "Link";

  return (
    <dialog
      ref={dialogRef}
      className="sync-modal app-link-sheet"
      aria-labelledby={titleId}
      aria-describedby={stop ? reasonId : undefined}
      onClose={onClose}
    >
      <header className="sync-modal-header">
        {stop && <Mark icon={stop.icon} tone={stop.severity} large />}
        <div>
          <h1 id={titleId} title={stop?.message} role={stop ? (stop.severity === "danger" ? "alert" : "status") : undefined}>{title}</h1>
          {stop && <span id={reasonId} className="sync-visually-hidden">{stop.message}</span>}
        </div>
        <button type="button" className="btn btn-ghost" onClick={close} aria-label="Close without opening the link">
          <Icon name="x" size={17} />
        </button>
      </header>
      {stop ? subject && (
        <div className="sync-modal-body app-link-lines">
          <Mark icon="link" />
          <code className="app-link-subject">{subject}</code>
        </div>
      ) : (
        <div className="sync-modal-body app-link-lines" role="status">
          <Mark icon="storage-mallard" />
          {landing ? (
            <span className="app-link-facts">
              {landing.found.facts.map((fact) => (
                <span key={fact.word} className="app-link-fact"><Icon name={fact.icon} size={12} />{fact.word}</span>
              ))}
            </span>
          ) : (
            <span className="app-link-facts">
              <span className="sync-visually-hidden">Looking up {subject ?? "the link"}…</span>
              <span className="app-link-placeholder" aria-hidden="true" /><span className="app-link-placeholder short" aria-hidden="true" />
            </span>
          )}
          <Mark icon={landing ? (landing.found.here.place === "catalog" ? "catalog" : "computer") : "computer"} tone={landing?.found.here.present ? "ok" : undefined} />
          {landing ? (
            <span className={`app-link-here${landing.found.here.present ? " present" : ""}`} title={landing.found.here.sentence}>
              {landing.found.here.present && <Icon name="check-circle" size={13} />}{landing.found.here.word}
            </span>
          ) : (
            <span className="app-link-placeholder" aria-hidden="true" />
          )}
        </div>
      )}
      <footer className="sync-modal-footer">
        <div>
          <button ref={exitRef} type="button" className="btn" onClick={close}>{verb || !verdict ? "Cancel" : "Close"}</button>
          {verdict && verb ? (
            <button ref={verbRef} type="button" className="btn btn-primary" onClick={() => onVerb(verdict)}>{verb}</button>
          ) : !verdict && (
            <button type="button" className="btn btn-primary" disabled>Open…</button>
          )}
        </div>
      </footer>
    </dialog>
  );
}
