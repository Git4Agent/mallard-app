import type { ReactNode } from "react";
import { useTranslation } from "../../i18n";
import Icon, { type IconName } from "../Icons";

/** The parts Add Catalog resource's steps share with New project, drawn in
 *  the shipped classes: a door, a pill, and the band badge. */

/** New project's door, in its own classes. */
export function SourceDoor({ icon, title, detail, checked, disabled, onChoose }: {
  icon: IconName;
  title: string;
  detail: string;
  checked: boolean;
  disabled: boolean;
  onChoose: () => void;
}) {
  return (
    <div className="storage-mode-option">
      <button type="button" className="storage-mode-door" role="radio" aria-checked={checked} disabled={disabled} onClick={onChoose}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
    </div>
  );
}

export function SetupPill({ active, disabled, mark, label, title, onClick }: {
  active: boolean;
  disabled: boolean;
  mark: ReactNode;
  label: string;
  title?: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={title}
      title={title}
      className={`setup-pill${active ? " active" : ""}`}
      disabled={disabled}
      onClick={onClick}
    >
      {mark}
      <span className="setup-pill-label">{label}</span>
      <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
    </button>
  );
}

/** The setup page's "Needs review" mark on a band still waiting for an
 *  answer. */
export function attentionBadge(show: boolean) {
  return show ? <AttentionBadge /> : null;
}

function AttentionBadge() {
  const { t } = useTranslation("library");
  return (
    <span className="sync-setup-state attention" aria-label={t("setup.needsReview")} title={t("setup.needsReview")}>
      <Icon name="alert-triangle" size={14} />
    </span>
  );
}
