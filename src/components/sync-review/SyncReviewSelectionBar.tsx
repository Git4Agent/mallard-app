import type { CSSProperties } from "react";
import Icon from "../Icons";

/**
 * The shared selected/total count, recommended-selection, and clear controls
 * of a review header, with the optional close affordance the embedded flows
 * append to the same cluster. The bar is purely presentational: features own
 * every count, title, and callback.
 */
export default function SyncReviewSelectionBar({
  count,
  total,
  busy = false,
  recommendedCount,
  recommendedTitle,
  recommendedDisabled,
  onRecommended,
  clearDisabled,
  onClear,
  searchActive = false,
  listPending = false,
  close,
}: {
  count: number;
  total: number;
  busy?: boolean;
  /** The rows have not landed: a count and the controls that change a
   *  selection would speak for rows nobody can see, so only close stays. */
  listPending?: boolean;
  /** How many rows the recommend action would tick: what it does, not how
   *  many are ticked now. */
  recommendedCount: number;
  /** The sentence behind the count, worded by the feature that knows it. */
  recommendedTitle: string;
  recommendedDisabled: boolean;
  onRecommended: () => void;
  clearDisabled: boolean;
  onClear: () => void;
  searchActive?: boolean;
  close?: {
    ariaLabel: string;
    onClose: () => void;
  };
}) {
  const unselect = recommendedDisabled && recommendedCount > 0 && !clearDisabled;
  const clearTitle = searchActive ? "Clear matching selection" : "Clear current selection";

  return (
    <div className="sync-push-resource-actions">
      {!listPending && (
        <>
          <span
            className="sync-sync-review-selected"
            style={{ "--review-count-digits": String(total).length } as CSSProperties}
            aria-live="polite"
            aria-label={`${count} of ${total} selected`}
            title={`${count} of ${total} selected`}
          >
            <b>{count}</b>/{total}
          </span>
          {/* The active recommendation can be cleared with the same button.
              Feature callbacks keep the action scoped to this step or search. */}
          <button
            type="button"
            className="btn sync-sync-review-select"
            onClick={unselect ? onClear : onRecommended}
            disabled={busy || (!unselect && (recommendedDisabled || recommendedCount === 0))}
            title={unselect ? clearTitle : recommendedTitle}
          >
            <Icon name="check-circle" size={14} />
            {/* The space keeps the accessible name "Select 3", not "Select3";
                a flex item's trailing space draws nothing. */}
            {unselect ? "Unselect " : "Select "}
            <span className="sync-path-count">{recommendedCount}</span>
          </button>
          <button
            type="button"
            className="btn btn-ghost sync-sync-review-icon-action"
            onClick={onClear}
            disabled={busy || clearDisabled}
            title={clearTitle}
            aria-label={clearTitle}
          >
            <Icon name="trash" size={15} />
          </button>
        </>
      )}
      {close && (
        <button
          type="button"
          className="btn btn-ghost sync-inline-action-close"
          onClick={close.onClose}
          disabled={busy}
          aria-label={close.ariaLabel}
        >
          <Icon name="x" size={15} />
        </button>
      )}
    </div>
  );
}
