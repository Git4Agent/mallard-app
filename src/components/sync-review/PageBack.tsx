import Icon from "../Icons";
import { useTranslation } from "../../i18n";

/**
 * Where a page's back button lands: one level up the trail, named as the
 * trail names it. `app` is a word the app owns — a section like Git history,
 * a Catalog row like Pets v1 — and speaks in the display face; `name` is the
 * user's own data — a session's title, an item's name — and keeps the sans
 * face its crumb uses.
 */
export interface PageBackTarget {
  voice: "app" | "name";
  label: string;
  onActivate: () => void;
  disabled?: boolean;
}

/**
 * A page's own way back. The crumb before the leaf walks the same way, but a
 * crumb reads as a place, not as a control; this leads the page's first row
 * in every state the page has — opening, failed and loaded — so the way out
 * never waits on the load.
 */
export default function PageBack({ target }: { target: PageBackTarget }) {
  const { t } = useTranslation("syncReview");
  const name = t("back", { label: target.label });
  return (
    <button
      type="button"
      className={`btn sync-page-back${target.voice === "name" ? " named" : ""}`}
      disabled={target.disabled}
      title={name}
      aria-label={name}
      onClick={target.onActivate}
    >
      <Icon name="chevron-left" size={13} />
      <span>{target.label}</span>
    </button>
  );
}
