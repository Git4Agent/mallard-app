import { t } from "../../i18n";
import type { CatalogNouns } from "./model";
import type { CatalogRemovalResult, CatalogResourceRemovalPlan } from "./types";

/** The dialog approves the backend's exact review. A changed page cancels it. */
export async function removeLocalCatalogResource(options: {
  nouns: CatalogNouns;
  prepare: () => Promise<CatalogResourceRemovalPlan>;
  confirm: (message: string, options: { title: string; kind: "warning"; okLabel: string; cancelLabel: string }) => Promise<boolean>;
  apply: (planId: string) => Promise<CatalogRemovalResult>;
  refresh: () => Promise<void>;
  isCurrent: () => boolean;
}): Promise<CatalogRemovalResult | null> {
  const plan = await options.prepare();
  if (!options.isCurrent()) return null;
  const title = options.nouns.say((noun) => (plan.uninstall
    ? t("library:removal.uninstallTitle", { noun })
    : t("library:removal.deleteTitle", { noun })), 1);
  const message = plan.uninstall
    ? t("library:removal.uninstallMessage", { name: plan.display_name })
    : t("library:removal.deleteMessage", { name: plan.display_name, count: plan.file_count });
  const okLabel = plan.uninstall ? t("library:removal.uninstall") : t("common:delete");
  if (!await options.confirm(message, { title, kind: "warning", okLabel, cancelLabel: t("common:cancel") }) || !options.isCurrent()) return null;
  try {
    return await options.apply(plan.plan_id);
  } finally {
    if (options.isCurrent()) await options.refresh();
  }
}
