import Icon from "../Icons";
import type { ProjectStorageLink, StorageConfig, StorageSetupDraft } from "../../types";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import LandingPicker, { type LandingCard } from "./LandingPicker";
import { storageSubtitle } from "./ProjectLinksWorkspace";

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
  storages: StorageConfig[];
  links: ProjectStorageLink[];
  storageIssues: Record<string, string>;
  /** Persisted setup drafts — storages that were never created. A row only
      reopens its draft; discard lives on the setup page, like every verb. */
  drafts: StorageSetupDraft[];
  busy: boolean;
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  onOpenStorage: (storageId: string) => void;
  onOpenStorageDraft: (draftId: string) => void;
  onAddStorage: () => void;
}

export default function StorageIndexPage({
  storages,
  links,
  storageIssues,
  drafts,
  busy,
  appActions,
  onOpenStorage,
  onOpenStorageDraft,
  onAddStorage,
}: Props) {
  const cards: LandingCard[] = [
    ...storages.map((storage): LandingCard => {
      const name = storage.name || "(unnamed)";
      const issue = storageIssues[storage.id] ?? null;
      const linked = links.filter((link) => link.storage_id === storage.id).length;
      return {
        key: storage.id,
        icon: storage.kind === "local" ? "drive" : "cloud",
        title: name,
        ariaLabel: issue ? `Open ${name}. Storage issue: ${issue}` : `Open ${name}`,
        location: storageSubtitle(storage),
        /* The tally and the issue word are the card's two facts; the word is
           the second channel beyond colour (WCAG 1.4.1), and it stays one
           word because the issue itself is a sentence that would give way
           to nothing on a card — the accessible name carries it whole. */
        detail: (
          <span className="landing-picker-card-chips">
            <span className="sync-storage-repo-chip">
              <span>Projects</span>
              <b>{linked}</b>
            </span>
            {issue && (
              <span className="sync-storage-index-warning">
                <Icon name="alert-triangle" size={12} aria-hidden="true" />
                <span>Issue</span>
              </span>
            )}
          </span>
        ),
        onOpen: () => onOpenStorage(storage.id),
      };
    }),
    // A persisted setup draft is content too: badged, tally-less, opening
    // its draft — and never leaking a secret (the view arrives redacted).
    ...drafts.map((draft): LandingCard => {
      const name = draft.storage.name || "(unnamed)";
      return {
        key: draft.draft_id,
        icon: draft.storage.kind === "local" ? "drive" : "cloud",
        title: name,
        ariaLabel: `Continue setting up ${name} — resumable storage draft`,
        location: "Setup not finished — not connected to Mallard yet",
        detail: (
          <span className="landing-picker-card-chips">
            <span className="sync-draft-badge">Draft</span>
          </span>
        ),
        onOpen: () => onOpenStorageDraft(draft.draft_id),
      };
    }),
  ];
  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "storages", label: "Storages", icon: "drive", tone: "root" }]}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="Storages path"
      />
      <main className="sync-main sync-project-links-page sync-landing-page">
        <LandingPicker
          title="Storages"
          count={cards.length === 1 ? "1 on this machine" : `${cards.length} on this machine`}
          actionLabel="Add storage…"
          emptyTitle="No storages yet"
          busy={busy}
          onAction={onAddStorage}
          cards={cards}
        />
      </main>
    </>
  );
}
