import type { BundleSnapshotSummary, SiblingBundleStatus } from "../../types";
import Notice from "../notice/Notice";
import { useTranslation } from "../../i18n";

interface Props {
  status: SiblingBundleStatus;
  busy: boolean;
  onConnect: (matches: BundleSnapshotSummary[]) => void;
}

/**
 * Another repo in the linked storage carries this project's repository.
 * Connecting to it makes pushes and pulls from every machine converge. The
 * `none` state renders nothing so callers can pass status through unfiltered.
 */
export default function SiblingRepoNotice({ status, busy, onConnect }: Props) {
  const { t } = useTranslation("reviews");
  if (status.state === "none") return null;
  const sibling = status.state === "sibling_found"
    ? status.matches[0] ?? null
    : status.survivor;
  if (!sibling) return null;
  const matches = status.state === "sibling_found" ? status.matches : [status.survivor];
  const body = status.state === "registration_twin"
    ? t("sibling.twin")
    : sibling.registration_only
      ? t("sibling.registrationOnly")
      : t("sibling.carries");
  /* `warning`, not `info`: every branch of the body says the reader is seeing
   * less than the whole truth — a repo they cannot see carries updates, or two
   * repos are racing for the same repository — and every branch asks for the
   * same act to fix it. Nothing is broken, so it is not `danger`.
   *
   * `standing`, not an event: the status is one of the answers the project
   * load returns, so the banner is on screen the moment the storage block
   * first paints. Content the view renders with is not a status message
   * (WCAG 2.2 SC 4.1.3) and must not use a live-region role. */
  return (
    <Notice
      severity="warning"
      timing="standing"
      label={t("sibling.notes")}
      className="sync-storage-notice sync-sibling-repo-notice notice-strip"
      title={t("sibling.title", { name: sibling.display_name })}
      action={(
        <button
          type="button"
          className="btn btn-small btn-primary"
          disabled={busy}
          onClick={() => onConnect(matches)}
        >
          {t("sibling.connect")}
        </button>
      )}
    >
      {body}
    </Notice>
  );
}
