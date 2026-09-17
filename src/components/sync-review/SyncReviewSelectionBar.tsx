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
  recommendedTitle,
  recommendedAriaLabel,
  recommendedDisabled,
  onRecommended,
  clearDisabled,
  onClear,
  searchActive = false,
  close,
}: {
  count: number;
  total: number;
  busy?: boolean;
  recommendedTitle: string;
  recommendedAriaLabel: string;
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
  return (
    <div className="sync-push-resource-actions">
      {/* The recommend button's own count is a property of the resource set,
          not of the selection, so it never moves when a row is ticked. Shown
          as a bare number beside a tick it read as "N selected"; the live
          count below is the one that answers that question. */}
      <span
        className="sync-sync-review-selected"
        style={{ "--review-count-digits": String(total).length } as CSSProperties}
        aria-live="polite"
        aria-label={`${count} of ${total} selected`}
        title={`${count} of ${total} selected`}
      >
        <b>{count}</b>/{total}
      </span>
      <button
        type="button"
        className="btn btn-ghost sync-sync-review-icon-action"
        onClick={onRecommended}
        disabled={busy || recommendedDisabled}
        title={recommendedTitle}
        aria-label={recommendedAriaLabel}
      >
        <Icon name="check-circle" size={15} />
      </button>
      <button
        type="button"
        className="btn btn-ghost sync-sync-review-icon-action"
        onClick={onClear}
        disabled={busy || clearDisabled}
        title={searchActive ? "Clear matching selection" : "Clear current selection"}
        aria-label={searchActive ? "Clear matching selection" : "Clear current selection"}
      >
        <Icon name="trash" size={15} />
      </button>
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
