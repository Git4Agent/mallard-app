import type { SessionEventCounts } from "../../../types";
import {
  formatSessionCount,
  sessionCountQualityLabel,
  sessionDetailFilterToggles,
  sessionEventCoverage,
  type SessionDetailFilters,
  type SessionFilterKey,
} from "./sessionDetailModel";

interface Props {
  counts: SessionEventCounts;
  filters: SessionDetailFilters;
  loadedCount: number;
  onFiltersChange: (filters: SessionDetailFilters) => void;
}

/**
 * Filtering is a property of the list, so it sits over the list rather than in
 * a rail beside it. The rail it replaces cost a permanent 284px column to hold
 * four controls, three of which were empty on a typical session.
 */
export default function SessionDetailToolbar({
  counts,
  filters,
  loadedCount,
  onFiltersChange,
}: Props) {
  const toggles = sessionDetailFilterToggles(counts);
  const coverage = sessionEventCoverage(counts);
  const toggle = (key: SessionFilterKey) => {
    onFiltersChange({ ...filters, [key]: !filters[key] });
  };

  // One readout, and an honest one: how much of the whole record is on screen.
  // The page used to carry three counts that measured three different things.
  const readout = coverage.total == null
    ? `${loadedCount} shown`
    : `${loadedCount} of ${coverage.total}${coverage.approximate ? "+" : ""} events`;

  return (
    <div className="session-detail-toolbar">
      <span className="session-detail-toolbar-label" id="session-detail-filter-label">Show</span>
      <div className="session-detail-toolbar-toggles" role="group" aria-labelledby="session-detail-filter-label">
        {toggles.map(({ key, label, count }) => {
          const quality = sessionCountQualityLabel(count);
          const pressed = filters[key];
          return (
            <button
              key={key}
              type="button"
              className={`session-detail-filter-toggle${pressed ? " is-on" : ""}`}
              aria-pressed={pressed}
              title={count.explanation ?? (quality ? `${quality} ${formatSessionCount(count)}` : undefined)}
              onClick={() => toggle(key)}
            >
              <span className="session-detail-filter-box" aria-hidden="true" />
              <span className="session-detail-filter-name">{label}</span>
              <span className={`session-detail-filter-count quality-${count.quality}`}>
                {formatSessionCount(count)}
              </span>
            </button>
          );
        })}
      </div>
      <output className="session-detail-toolbar-readout">{readout}</output>
    </div>
  );
}
