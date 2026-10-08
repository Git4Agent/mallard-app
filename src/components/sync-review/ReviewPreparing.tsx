import { useEffect } from "react";
import Icon from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import { useTranslation } from "../../i18n";

/**
 * The review's own frame while the review is still being prepared: the
 * storage card opens here, where the review will be, and turns into it (or
 * into the card's notice) in place. It draws the review's header, the list's
 * `sessions` scaffold under one caption, and the footer's primary, with the
 * review's own classes so the swap to the mounted review keeps its geometry.
 *
 * Nothing is known about the rows yet, so it draws the default placeholders.
 * While preparing only reads, × is Cancel; once the host starts a write, it
 * withdraws Cancel. Presentational: the host owns every word and callback.
 */
export default function ReviewPreparing({
  verb,
  title,
  caption,
  onCancel,
  cancelDisabled,
}: {
  /** Picks the review's own classes, so the frame matches it. */
  verb: "push" | "pull";
  title: string;
  /** What is being waited on, without the ellipsis. */
  caption: string;
  onCancel: () => void;
  cancelDisabled: boolean;
}) {
  const { t } = useTranslation("syncReview");
  useEffect(() => {
    const cancelOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !cancelDisabled) onCancel();
    };
    window.addEventListener("keydown", cancelOnEscape);
    return () => window.removeEventListener("keydown", cancelOnEscape);
  }, [cancelDisabled, onCancel]);
  const pull = verb === "pull";
  return (
    <section
      className={`sync-inline-action-review ${pull ? "sync-inline-pull-review" : "sync-inline-push-review"} sync-sync-review-workspace`}
      aria-labelledby="sync-review-preparing-title"
      aria-busy="true"
    >
      <header className={`sync-inline-action-header ${pull ? "sync-inline-pull-header" : "sync-push-resource-header"} sync-sync-review-header`}>
        <div className="sync-inline-action-heading">
          <div className="sync-sync-review-title">
            <h2 id="sync-review-preparing-title">{title}</h2>
          </div>
        </div>
        <div className="sync-push-resource-actions">
          <button
            type="button"
            className="btn btn-ghost sync-inline-action-close"
            onClick={onCancel}
            disabled={cancelDisabled}
            title={cancelDisabled ? t("preparing.writing") : t("preparing.cancel")}
            aria-label={t("preparing.cancelLabel", { title })}
          >
            <Icon name="x" size={15} />
          </button>
        </div>
      </header>

      <div className={`${pull ? "sync-pull-review-content" : "sync-push-resource-content"} sync-inline-action-content sync-sync-review-content`}>
        <div className="sync-inline-action-scroll sync-sync-review-scroll">
          <div className="sync-history-page sync-history-embedded sync-history-selection-review">
            <section className="sync-history-content" aria-label={title}>
              <LoadingScaffold shape="sessions" label={caption} />
            </section>
          </div>
        </div>
      </div>

      <footer className={`${pull ? "sync-pull-apply-bar " : "sync-push-resource-footer "}sync-inline-action-footer sync-sync-review-footer`}>
        <button type="button" className="btn btn-primary sync-pull-apply-button" disabled title={t("preparing.label")}>
          <Icon name="refresh" size={16} className="icon-spin" />
          {t("preparing.label")}
        </button>
      </footer>
    </section>
  );
}
