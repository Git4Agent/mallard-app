import { storageIconName } from "../Icons";
import { useTranslation } from "../../i18n";
import { NoticeChip } from "../notice/Notice";
import type {
  ProjectStorageLink,
  StorageProviderDescriptor,
  StorageProviderKey,
  StorageSetupDraft,
  StorageSummary,
} from "../../types";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import LandingPicker, { type LandingCard, type LandingDoor } from "./LandingPicker";
import { storageDoors } from "./model";
import {
  findStorageProvider,
  storageConnectionIssue,
  storageSubtitle,
} from "./StorageSettings";
import { storageProviderName } from "./storageText";

/**
 * The page the storage trail is rooted at: every storage page begins its path
 * with a Storages crumb, and a crumb has to lead somewhere real. This is that
 * somewhere — one door per configured storage, each opening its page.
 *
 * The sidebar lists the same storages as a shortcut rail; this page is the
 * destination those shortcuts skip past, so it stays a plain index: no
 * per-card menus, no editors. Every verb but Add lives on the storage's own
 * page.
 */

interface Props {
  storages: StorageSummary[];
  links: ProjectStorageLink[];
  storageIssues: Record<string, string>;
  /** Persisted setup drafts — storages that were never created, and unsaved
      edits of existing ones. A row only reopens its draft; discard lives on
      the setup page, like every verb. */
  drafts: StorageSetupDraft[];
  /** Compiled provider descriptors: a draft carries no icon of its own, so
      its row borrows the provider's, and with nothing listed each provider
      is a door. */
  storageProviders: StorageProviderDescriptor[];
  busy: boolean;
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  /** Opens the storage's page. */
  onOpenStorage: (storageId: string) => void;
  onOpenStorageDraft: (draftId: string) => void;
  /** Add, with the provider already chosen when a door names one; without
      one, the provider picker asks. */
  onAddStorage: (provider?: StorageProviderKey) => void;
}

export default function StorageIndexPage({
  storages,
  links,
  storageIssues,
  drafts,
  storageProviders,
  busy,
  appActions,
  onOpenStorage,
  onOpenStorageDraft,
  onAddStorage,
}: Props) {
  const { t } = useTranslation("storage");
  const cards: LandingCard[] = [
    ...storages.map((storage): LandingCard => {
      const name = storage.name || t("shared.unnamed");
      const reconnect = storageConnectionIssue(storage);
      const issue = storageIssues[storage.storage_id] ?? null;
      const issueSentence = reconnect ?? issue;
      const linked = links.filter((link) => link.storage_id === storage.storage_id).length;
      return {
        key: storage.storage_id,
        icon: storageIconName(storage.provider, storage.icon),
        title: name,
        ariaLabel: [
          t("shared.openNamed", { name }),
          storage.available ? null : t("index.unavailableAdapter", { provider: storageProviderName(storage) }),
          issueSentence ? t("shared.storageIssue", { issue: issueSentence }) : null,
        ].filter(Boolean).join(t("shared.labelSeparator")),
        location: storageSubtitle(storage),
        /* The tally and the issue word are the card's two facts; the word is
           the second channel beyond colour (WCAG 1.4.1), and it stays one
           word because the issue itself is a sentence that would give way
           to nothing on a card — the accessible name carries it whole. */
        detail: (
          <span className="landing-picker-card-chips">
            <span className="sync-storage-repo-chip">
              <span>{t("index.projects")}</span>
              <b>{linked}</b>
            </span>
            {!storage.available && <NoticeChip severity="warning">{t("index.unavailable")}</NoticeChip>}
            {reconnect
              ? <NoticeChip severity="warning">{t("shared.reconnect")}</NoticeChip>
              : issue && <NoticeChip severity="warning">{t("index.issue")}</NoticeChip>}
          </span>
        ),
        onOpen: () => onOpenStorage(storage.storage_id),
      };
    }),
    // A persisted setup draft is content too: badged, tally-less, opening
    // its draft — and never leaking a secret (drafts carry presence only).
    ...drafts.map((draft): LandingCard => {
      const name = draft.name || t("shared.unnamed");
      return {
        key: draft.draft_id,
        icon: storageIconName(
          draft.provider,
          findStorageProvider(storageProviders, draft.provider, draft.config_version)?.icon,
        ),
        title: name,
        ariaLabel: t("index.draftLabel", { name }),
        location: draft.edits_existing
          ? t("index.draftReconnect", { provider: storageProviderName(draft) })
          : t("index.draftSetup"),
        detail: (
          <span className="landing-picker-card-chips">
            <span className="sync-draft-badge">{t("index.draft")}</span>
          </span>
        ),
        onOpen: () => onOpenStorageDraft(draft.draft_id),
      };
    }),
  ];
  const doors: LandingDoor[] = storageDoors(storageProviders).map((door) => ({
    key: door.key,
    icon: door.icon,
    label: door.label,
    ariaLabel: t("index.addProvider", { provider: door.label }),
    onOpen: () => onAddStorage(door.key),
  }));
  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "storages", label: t("shared.storages"), icon: "storage", tone: "root" }]}
        overflow={appActions}
        overflowLabel={t("shared.appActions")}
        ariaLabel={t("index.pathLabel")}
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        <LandingPicker
          title={t("shared.storages")}
          count={t("index.count", { count: cards.length })}
          actionLabel={t("shared.addStorageMore")}
          busy={busy}
          onAction={() => onAddStorage()}
          /* A build whose descriptors have not arrived still leaves one door:
             the picker itself. */
          doors={doors.length > 0 ? doors : [{
            key: "any",
            icon: "storage",
            label: t("shared.addStorageMore"),
            ariaLabel: t("shared.addStorage"),
            onOpen: () => onAddStorage(),
          }]}
          caption={{ icons: ["folder", "catalog"], text: t("index.caption") }}
          cards={cards}
        />
      </main>
    </>
  );
}
