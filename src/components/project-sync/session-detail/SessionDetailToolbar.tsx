import type { SessionCount, SessionEventCounts } from "../../../types";
import { useTranslation } from "../../../i18n";
import { backendText } from "../../../i18n/backend";
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

/** Why a count is not exact: the backend's English, translated by its code. */
export function sessionCountExplanation(count: SessionCount): string | undefined {
  const english = count.explanation ?? undefined;
  return english && count.explanation_code
    ? backendText(`sessionCount.${count.explanation_code}`, english)
    : english;
}

/** Filtering is a property of the list, so the controls sit above the list. */
export default function SessionDetailToolbar({
  counts,
  filters,
  loadedCount,
  onFiltersChange,
}: Props) {
  const { t } = useTranslation("session");
  const toggles = sessionDetailFilterToggles(counts);
  const coverage = sessionEventCoverage(counts);
  const toggle = (key: SessionFilterKey) => {
    onFiltersChange({ ...filters, [key]: !filters[key] });
  };

  // One readout states how much of the whole record is on screen.
  const readout = coverage.total == null
    ? t("toolbar.shown", { loaded: loadedCount })
    : t("toolbar.ofEvents", { loaded: loadedCount, total: `${coverage.total}${coverage.approximate ? "+" : ""}` });

  return (
    <div className="session-detail-toolbar">
      <span className="session-detail-toolbar-label" id="session-detail-filter-label">{t("toolbar.show")}</span>
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
              title={sessionCountExplanation(count) ?? (quality ? t("toolbar.countTitle", { quality, value: formatSessionCount(count) }) : undefined)}
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
