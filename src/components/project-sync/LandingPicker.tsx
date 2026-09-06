import type { ReactNode } from "react";
import Icon, { type IconName } from "../Icons";

/**
 * The landing of a nav root — Projects, Catalog, Storages — when nothing in
 * it is selected: a heading, the count, the one verb that adds to the list,
 * and one box per entry, the whole box being the door.
 *
 * Every fact a card shows is registry-level — what the sidebar already holds.
 * The page never scans, counts sessions, or stamps; its worst case is a name
 * and a location, so opening it costs what opening the sidebar costs.
 */
export interface LandingCard {
  key: string;
  icon: IconName;
  title: string;
  /** The door's accessible name; the title when omitted. */
  ariaLabel?: string;
  /** Marks at the title's end — identity, never status. */
  marks?: ReactNode;
  /** The identifier line: a path, a bucket, a home. */
  location: string;
  /** One registry fact under the location, or a chip row. */
  detail?: ReactNode;
  /** A detail that says something is absent reads quieter. */
  detailMuted?: boolean;
  onOpen: () => void;
}

/**
 * With nothing listed, the page is the filled landing ghosted: one dashed
 * card per way Add can start — a kind, a provider, or the one verb — each
 * opening Add with that choice already made. A door names its choice and
 * wears the marks of the agents it covers; it carries no location, because
 * nothing has one yet.
 */
export interface LandingDoor {
  key: string;
  icon: IconName;
  label: string;
  /** "Add Skills", "Add Local folder storage" — the choice as a verb. */
  ariaLabel: string;
  marks?: ReactNode;
  onOpen: () => void;
}

/** One line under the ghost cards saying what the list is for, in icons and
 *  a few words — the only copy the empty page adds. */
export interface LandingCaption {
  icons: IconName[];
  text: string;
}

export default function LandingPicker({
  title,
  count,
  actionLabel,
  cards,
  doors,
  caption,
  busy = false,
  onAction,
}: {
  title: string;
  /** "N on this machine" — a fact about the list, in the identifier face. */
  count: string;
  actionLabel: string;
  cards: LandingCard[];
  doors: LandingDoor[];
  caption: LandingCaption;
  busy?: boolean;
  onAction: () => void;
}) {
  return (
    <section className="landing-picker" aria-labelledby="landing-picker-title">
      <div className="landing-picker-head">
        <h1 id="landing-picker-title" className="settings-section-title">{title}</h1>
        <span className="landing-picker-count">{count}</span>
        <button type="button" className="btn btn-primary" onClick={onAction} disabled={busy}>
          <Icon name="plus" size={13} />
          {actionLabel}
        </button>
      </div>
      {cards.length === 0 ? (
        <>
          <ul className="landing-picker-grid landing-picker-doors">
            {doors.map((door) => (
              <li key={door.key}>
                <button
                  type="button"
                  className="landing-picker-door"
                  aria-label={door.ariaLabel}
                  disabled={busy}
                  onClick={door.onOpen}
                >
                  <span className="landing-picker-door-title">
                    <Icon name={door.icon} size={17} />
                    <strong>{door.label}</strong>
                    <Icon name="plus" size={13} />
                  </span>
                  <span className="landing-picker-door-bar" aria-hidden="true" />
                  {door.marks
                    ? <span className="landing-picker-door-marks">{door.marks}</span>
                    : <span className="landing-picker-door-bar short" aria-hidden="true" />}
                </button>
              </li>
            ))}
          </ul>
          <p className="landing-picker-caption">
            {caption.icons.map((icon) => <Icon key={icon} name={icon} size={14} />)}
            <span>{caption.text}</span>
          </p>
        </>
      ) : (
        <ul className="landing-picker-grid">
          {cards.map((card) => (
            <li key={card.key}>
              <button
                type="button"
                className="landing-picker-card"
                aria-label={card.ariaLabel ?? card.title}
                disabled={busy}
                onClick={card.onOpen}
              >
                <span className="landing-picker-card-title">
                  <Icon name={card.icon} size={17} />
                  <strong>{card.title}</strong>
                  {card.marks && <span className="landing-picker-card-marks">{card.marks}</span>}
                </span>
                <span className="landing-picker-card-location">{card.location}</span>
                {card.detail && (
                  <span className={`landing-picker-card-detail${card.detailMuted ? " muted" : ""}`}>{card.detail}</span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
