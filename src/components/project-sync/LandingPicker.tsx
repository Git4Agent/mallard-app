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

export default function LandingPicker({
  title,
  count,
  actionLabel,
  emptyTitle,
  cards,
  busy = false,
  onAction,
}: {
  title: string;
  /** "N on this machine" — a fact about the list, in the identifier face. */
  count: string;
  actionLabel: string;
  emptyTitle: string;
  cards: LandingCard[];
  busy?: boolean;
  onAction: () => void;
}) {
  const action = (
    <button type="button" className="btn btn-primary" onClick={onAction} disabled={busy}>
      <Icon name="plus" size={13} />
      {actionLabel}
    </button>
  );
  if (cards.length === 0) {
    return (
      <section className="landing-picker landing-picker-empty" aria-labelledby="landing-picker-title">
        <span className="landing-picker-mark" aria-hidden="true">
          <img src="/mallard-logo-duck.svg" alt="" />
        </span>
        <h1 id="landing-picker-title" className="settings-section-title">{emptyTitle}</h1>
        {action}
      </section>
    );
  }
  return (
    <section className="landing-picker" aria-labelledby="landing-picker-title">
      <div className="landing-picker-head">
        <h1 id="landing-picker-title" className="settings-section-title">{title}</h1>
        <span className="landing-picker-count">{count}</span>
        {action}
      </div>
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
    </section>
  );
}
