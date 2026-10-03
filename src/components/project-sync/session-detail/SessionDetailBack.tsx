import Icon from "../../Icons";

/**
 * Where the session page's back button lands: one level up the trail, named
 * as the trail names it. From a session that is the project's page — a
 * `section`, the app's own word — and from an agent's page it is the session
 * that delegated to it, whose title is the user's data.
 */
export interface SessionBackTarget {
  kind: "section" | "session";
  label: string;
  onActivate: () => void;
  disabled?: boolean;
}

/**
 * The page's own way back. The project crumb walks the same way, but a crumb
 * reads as a place, not as a control; this leads the page in every state it
 * has — opening, failed and loaded — so the way out never waits on the load.
 */
export default function SessionDetailBack({ target }: { target: SessionBackTarget }) {
  return (
    <button
      type="button"
      className={`btn session-detail-back${target.kind === "session" ? " to-session" : ""}`}
      disabled={target.disabled}
      title={`Back to ${target.label}`}
      aria-label={`Back to ${target.label}`}
      onClick={target.onActivate}
    >
      <Icon name="chevron-left" size={13} />
      <span>{target.label}</span>
    </button>
  );
}
