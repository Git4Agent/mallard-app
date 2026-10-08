import { useTranslation } from "../../i18n";
import Icon from "../Icons";
import type { CatalogScopeRow } from "./types";

export interface CatalogRemovalAction {
  label: string;
  busy: boolean;
  removing: boolean;
  onRemove: () => void;
}

export default function CatalogRemoveButton({ row, action }: { row: CatalogScopeRow; action?: CatalogRemovalAction }) {
  const { t } = useTranslation("library");
  if (!action || !row.local_present) return null;
  const reason = row.blocker ?? (!row.resource_id ? t("removal.unidentified") : null);
  return (
    <button type="button" className="btn btn-ghost sync-history-session-detail-action"
      title={reason ?? action.label} aria-label={action.label}
      disabled={action.busy || !!reason} onClick={action.onRemove}>
      <Icon name={action.removing ? "refresh" : "trash"} size={14} className={action.removing ? "icon-spin" : undefined} />
    </button>
  );
}
