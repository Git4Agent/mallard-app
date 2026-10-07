import type { CatalogRemovalResult, CatalogResourceRemovalPlan } from "./types";

/** The dialog approves the backend's exact review. A changed page cancels it. */
export async function removeLocalCatalogResource(options: {
  kindNoun: string;
  prepare: () => Promise<CatalogResourceRemovalPlan>;
  confirm: (message: string, options: { title: string; kind: "warning"; okLabel: string; cancelLabel: string }) => Promise<boolean>;
  apply: (planId: string) => Promise<CatalogRemovalResult>;
  refresh: () => Promise<void>;
  isCurrent: () => boolean;
}): Promise<CatalogRemovalResult | null> {
  const plan = await options.prepare();
  if (!options.isCurrent()) return null;
  const verb = plan.uninstall ? "Uninstall" : "Delete";
  const title = `${verb} local ${options.kindNoun}?`;
  const message = plan.uninstall
    ? `Uninstall “${plan.display_name}” from this provider home?\n\nThe provider will remove this user installation. Copies in storage are kept.`
    : `Permanently delete “${plan.display_name}” from this provider home?\n\nThis deletes its local folder and ${plan.file_count} file${plan.file_count === 1 ? "" : "s"}. Copies in storage are kept. This action cannot be undone.`;
  if (!await options.confirm(message, { title, kind: "warning", okLabel: verb, cancelLabel: "Cancel" }) || !options.isCurrent()) return null;
  try {
    return await options.apply(plan.plan_id);
  } finally {
    if (options.isCurrent()) await options.refresh();
  }
}
