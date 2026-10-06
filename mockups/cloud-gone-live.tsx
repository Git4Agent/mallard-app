import type { ManagedCloudLink, ManagedCloudStatus } from "../src/components/managed-cloud/types";
import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { NoticeZone, type NoticeItem } from "../src/components/notice/Notice";
import ManagedCloudChip, { ManagedCloudPanel } from "../src/components/managed-cloud/ManagedCloudChip";
import { clearedStorageNotice } from "../src/components/project-sync/clearedStorageNotice";
import { cloudStatusNotice, cloudSyncBlocked, discardDraftQuestion } from "../src/components/managed-cloud/managedCloudLink";
import Icon from "../src/components/Icons";
import type { ScopeContinuity, ThreadSyncComparison } from "../src/types";
import "../src/App.css";

/* A dev harness for PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md: what the
   workspace says about a Mallard Cloud repository that is gone, being
   deleted, created again, or whose draft was discarded, and the discard verb
   in the Cloud chip's panel. Every sentence, every verb and both buttons' state
   come from the functions the workspace calls; the components are the
   shipped ones. Open /mockups/cloud-gone-live.html on the Vite server
   (`?theme=dark` for the dark palette). */

const link: ManagedCloudLink = {
  repository_id: "repo_one", visibility: "public", can_pull: true, can_push: true,
  website_url: "https://mallard-ai.com/rika/portal", owner_login: "rika", name: "portal",
  repository_revision: 7, can_request_release: true, can_publish_version: true, role: "owner",
  release: { kind: "snapshot_changed" },
};
/** Where a pushed draft stands: the latest version, the pushes past it, and
 *  the version the public reads. */
const standing = (latest: number | null, ahead: number | null, shown: number | null): NonNullable<ManagedCloudStatus["publication"]> => ({
  draft: "pushed", latest_version: latest, next_version: (latest ?? 0) + 1, draft_ahead: ahead, public_version: shown, review_version: null,
});
const ready = (overrides: Partial<ManagedCloudStatus> = {}, of: Partial<ManagedCloudLink> = {}): ManagedCloudStatus => ({
  repository_id: "repo_one", state: "ready", message: null,
  link: { ...link, ...of },
  publication: standing(2, 3, 2),
  ...overrides,
});
const failed = (state: ManagedCloudStatus["state"], message: string): ManagedCloudStatus => (
  { repository_id: "repo_one", state, link: null, message }
);

function comparison(
  status: ManagedCloudStatus,
  remote: ThreadSyncComparison["remote_continuity"],
  codex: ScopeContinuity,
): ThreadSyncComparison {
  return {
    project_id: "project-specimen", storage_id: "storage-specimen", storage_name: "Mallard Cloud",
    compared_at: 0, entries: [],
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [
      { scope: "project", access: "bound_ready", continuity: { state: "fresh" } },
      { scope: "provider:codex", provider: "codex", access: "bound_ready", continuity: codex },
    ],
    provider_errors: {}, warnings: [],
    remote_continuity: remote,
    managed_cloud_status: status,
  };
}

const chipActions = { onOpenUrl: () => {}, onPublishVersion: () => {}, onDiscardDraft: () => {}, onRetry: () => {}, onStorageSettings: () => {} };

/** The bar's two buttons and its Cloud chip, as the workspace decides them. */
function Actions({ status }: { status: ManagedCloudStatus }) {
  const blocked = cloudSyncBlocked(status);
  return (
    <div className="sync-path-actions" style={{ justifyContent: "flex-start", padding: "0 0 8px" }}>
      <ManagedCloudChip status={status} storageName="Mallard Cloud" busy={false} refreshing={false} {...chipActions} />
      <div className="sync-path-action-group" role="group" aria-label="Sync actions">
        <button type="button" className="sync-path-sync" disabled={blocked}><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
        <button type="button" className="sync-path-sync" disabled={blocked}><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span></button>
      </div>
    </div>
  );
}

/** The workspace's own note, with the one control the decision names. */
function WorkspaceNote({ status }: { status: ManagedCloudStatus }) {
  const note = cloudStatusNotice(status);
  if (!note) return null;
  const item: NoticeItem = {
    key: "managed-cloud-status", severity: "warning", title: note.title, message: note.message,
    action: note.action === null ? undefined : (
      <button type="button" className="btn btn-ghost">{note.action === "storage_settings" ? "Storage settings" : "Retry"}</button>
    ),
  };
  return <NoticeZone items={[item]} subject="notes about this workspace" timing="standing" label="Notes about this workspace" />;
}

function ActivityNote({ of }: { of: ThreadSyncComparison }) {
  const note = clearedStorageNotice(of, "Mallard Cloud");
  if (!note) return <p style={{ opacity: 0.7, fontSize: 13, margin: 0 }}>No note.</p>;
  return <NoticeZone items={[note]} subject="notes about this project activity" className="sync-history-notices" timing="standing" label="Notes about this project activity" />;
}

/** The Cloud chip's panel, held open: the verbs the status offers. */
function Menu({ status }: { status: ManagedCloudStatus }) {
  return (
    <div className="sync-path-menu" style={{ position: "static", width: 320 }}>
      <ManagedCloudPanel status={status} storageName="Mallard Cloud" busy={false} refreshing={false} close={() => {}} {...chipActions} />
    </div>
  );
}

function Specimen({ when, where, children }: { when: string; where: string; children: ReactNode }) {
  return (
    <section style={{ display: "grid", gap: 8 }}>
      <header>
        <strong>{when}</strong>
        <div style={{ opacity: 0.7, fontSize: 13 }}>{where}</div>
      </header>
      {children}
    </section>
  );
}

const gone = failed("gone", "Deleted, or this account lost access. Push creates it again.");
const deleting = failed("deleting", "Push creates it again.");
const refused = failed("inaccessible", "Mallard Cloud refused this account. It may be suspended.");
const createdAgain = ready({ publication: standing(null, null, null) }, { repository_id: "repo_two", visibility: "private", release: { kind: "private" } });
const backAtPublic = ready({ publication: standing(2, 0, 2) }, { release: { kind: "public" }, can_request_release: false, can_publish_version: false });
const emptied = ready({ publication: { ...standing(null, null, null), draft: "none" } }, { visibility: "private", release: { kind: "not_pushed" }, can_request_release: false, can_publish_version: false });
const question = discardDraftQuestion(link, ready().publication);

document.documentElement.dataset.theme = new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light";
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <main style={{ maxWidth: 860, margin: "32px auto", padding: "0 24px 60px", display: "grid", gap: 32 }}>
      <Specimen when="Being deleted" where="The repository was removed on the website and the service is still erasing it. The workspace's note has no control; Pull and Push are open.">
        <Actions status={deleting} />
        <WorkspaceNote status={deleting} />
      </Specimen>
      <Specimen when="Repository gone" where="Removed, or this account lost access: the service answers both alike.">
        <Actions status={gone} />
        <WorkspaceNote status={gone} />
        <ActivityNote of={comparison(gone, { state: "remote_missing", base_revision: 2 }, { state: "fresh" })} />
      </Specimen>
      <Specimen when="Any other answer" where="Here a suspended account. It never reads as empty: Pull and Push are closed, and Retry compares again.">
        <Actions status={refused} />
        <WorkspaceNote status={refused} />
      </Specimen>
      <Specimen when="Created again" where="Another computer of the account pushed the repository again. It is read in place of the one the link saved, until a Pull or a Push saves it.">
        <Actions status={createdAgain} />
        <ActivityNote of={comparison(createdAgain, { state: "replaced", base_revision: 2, remote_revision: 1 }, { state: "unreviewed", remote_revision: 1 })} />
      </Specimen>
      <Specimen when="Draft discarded, back at the latest version" where="On every computer that had pushed or pulled the draft.">
        <Actions status={backAtPublic} />
        <ActivityNote of={comparison(backAtPublic, { state: "replaced", base_revision: 5, remote_revision: 2 }, { state: "unreviewed", remote_revision: 2 })} />
      </Specimen>
      <Specimen when="Draft discarded, and another computer has pushed since" where="">
        <Actions status={ready()} />
        <ActivityNote of={comparison(ready(), { state: "replaced", base_revision: 5, remote_revision: 3 }, { state: "unreviewed", remote_revision: 3 })} />
      </Specimen>
      <Specimen when="Draft discarded on a private repository" where="Discarded on the website: nothing is left to compare with.">
        <Actions status={emptied} />
        <ActivityNote of={comparison(emptied, { state: "remote_missing", base_revision: 2 }, { state: "fresh" })} />
      </Specimen>
      <Specimen when="The Owner's Cloud panel, draft past the public version" where="Discard draft… is the last row, in danger ink.">
        <Menu status={ready()} />
      </Specimen>
      <Specimen when="An Admin's Cloud panel, the same repository" where="No discard: an Admin discards on the website.">
        <Menu status={ready({}, { role: "admin" })} />
      </Specimen>
      <Specimen when="The Cloud panel while another repository is read in place of the link's" where="Publish and the discard name the repository by the link, so they wait.">
        <Menu status={ready({}, { repository_id: "repo_two" })} />
      </Specimen>
      <Specimen when="What Discard draft… asks" where="In the system's dialog: Cancel, Discard.">
        <pre style={{ margin: 0, padding: 12, whiteSpace: "pre-wrap", border: "2px solid var(--border-mid)", borderRadius: 10, font: "inherit" }}>
          <strong>{question.title}</strong>{"\n\n"}{question.message}{"\n\n"}[{question.cancelLabel}] [{question.okLabel}]
        </pre>
      </Specimen>
    </main>
  </StrictMode>,
);
