import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { NoticeZone } from "../src/components/notice/Notice";
import { clearedStorageNotice } from "../src/components/project-sync/clearedStorageNotice";
import SyncOutcomeNotice from "../src/components/sync-review/SyncOutcomeNotice";
import { REVIEW_EXPIRED, REVIEW_SUPERSEDED } from "../src/components/project-sync/pullReviewFlow";
import { PUSH_FINISHED, ownUnfinishedPushSentence, unfinishedPushSentence } from "../src/unfinishedPush";
import { pushFailureNotice } from "../src/components/project-sync/pushFailureNotice";
import type { SyncOutcome } from "../src/components/project-sync/syncOutcome";
import type { ScopeContinuity, ThreadSyncComparison } from "../src/types";
import "../src/App.css";

/* A dev harness: the notices of DESIGN_CAS_FREE_STORAGE_REFACTOR.md
   section 13, the standing notice of PLAN_CLEARED_STORAGE_PUSH_AGAIN.md
   section 5 in its three forms, and the Pull texts of
   PLAN_PULL_PINNED_VERSION.md section 5, drawn by the real components in
   the surfaces they already live in. Open /mockups/push-notices-live.html on the
   Vite server. */

const outcomeBase = {
  projectId: "project-specimen",
  storageId: "storage-specimen",
  tone: "warning" as const,
  defaultOpen: true,
  technical: [],
};

/** A Pull that stopped after it installed three sessions (D2 to D4). */
const stoppedPull: SyncOutcome = {
  ...outcomeBase,
  title: "Pull stopped",
  message: "Part of this version is missing from Attic R2. Compare again.",
  stats: [
    { icon: "download", value: "3", label: "restored" },
    { icon: "git-compare-arrows", value: "4", label: "not pulled" },
    { icon: "git-branch", label: "Version", value: "5", lead: true },
  ],
  rows: [
    { key: "a", title: "Refactor the parser", status: "synced" },
    { key: "b", title: "Write the changelog", status: "not_pulled" },
    { key: "c", title: "notes.md", status: "not_pulled" },
  ],
  actions: [{ kind: "open_pull", label: "Open Pull review" }],
};

/** Sessions that failed alone (D6 and T7): no reset is offered. */
const failedAlone: SyncOutcome = {
  ...outcomeBase,
  title: "2 changes need attention",
  stats: [
    { icon: "download", value: "1", label: "restored" },
    { icon: "octagon-x", value: "2", label: "failed" },
    { icon: "git-branch", label: "Version", value: "5", lead: true },
  ],
  rows: [
    { key: "a", title: "Refactor the parser", status: "synced" },
    {
      key: "d6",
      title: "An older session",
      status: "failed",
      reason: "This stored session cannot be read by this version of Mallard.",
    },
    {
      key: "t7",
      title: "Today's session",
      status: "failed",
      reason: "This conversation changed after the comparison. Compare again.",
    },
  ],
  actions: [{ kind: "open_pull", label: "Open Pull review" }],
};

/** A comparison of a storage another computer cleared: the agent's area
 *  reads `codex`, and the project's files were never stored. */
function clearedComparison(
  remote: ThreadSyncComparison["remote_continuity"],
  codex: ScopeContinuity,
): ThreadSyncComparison {
  return {
    project_id: "project-specimen",
    storage_id: "storage-specimen",
    storage_name: "Attic R2",
    compared_at: 0,
    entries: [],
    counts: { synced: 0, local: 0, storage: 0, diverged: 0, unavailable: 0, unknown: 0 },
    scopes: [
      { scope: "project", access: "bound_ready", continuity: { state: "fresh" } },
      { scope: "provider:codex", provider: "codex", access: "bound_ready", continuity: codex },
    ],
    provider_errors: {},
    warnings: [],
    remote_continuity: remote,
  };
}

/** The shell zone under the path bar, where a Push failure is drawn. */
function ShellBanner({ error }: { error: string }) {
  return (
    <NoticeZone
      items={[pushFailureNotice(error, "Attic R2", { busy: false, onFinishPush: () => undefined, onDismiss: () => undefined })]}
      subject="messages from Mallard"
      className="sync-backend-banner"
      timing="blocking"
    />
  );
}

/** The Pull review's failure, as the head of its banner zone draws it. */
function PullBanner({ error }: { error: string }) {
  return (
    <div className="sync-review-banners">
      <NoticeZone
        items={[{
          key: "pull-failed",
          severity: "danger",
          message: error,
          action: <button type="button" className="btn btn-ghost">Compare again</button>,
        }]}
        subject="notes about this Pull review"
        timing="blocking"
      />
    </div>
  );
}

function Specimen({ when, where, children }: { when: string; where: string; children: React.ReactNode }) {
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

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <main style={{ maxWidth: 860, margin: "32px auto", padding: "0 24px", display: "grid", gap: 32 }}>
      <Specimen
        when="RemoteChanged"
        where="The shell banner under the path bar, while the Push review is open. Another computer pushed on the same version while this push ran: seen in the listing before the write, or selected by the listing after it. The push failed as a whole."
      >
        <ShellBanner error="The storage changed while pushing. Compare again." />
      </Specimen>
      <Specimen
        when="A confirmed push was displaced afterwards"
        where="Standing note in the project activity notice zone, also above the Push review list, while the comparison reads the last push displaced. Nothing is rolled back; the areas it wrote read as never reviewed here, so a Pull comes first. It ends by itself after the next Push."
      >
        <NoticeZone
          items={[clearedStorageNotice(
            clearedComparison(
              { state: "displaced", base_revision: 3, remote_revision: 3, saved_by: "push" },
              { state: "unreviewed", remote_revision: 3 },
            ),
            "Attic R2",
          )!]}
          subject="notes about this project activity"
          timing="standing"
          label="Notes about this project activity"
        />
      </Specimen>
      <Specimen
        when="A pulled version was displaced afterwards"
        where="The same zone, when this computer did not publish the displaced version: it pulled it."
      >
        <NoticeZone
          items={[clearedStorageNotice(
            clearedComparison(
              { state: "displaced", base_revision: 3, remote_revision: 3, saved_by: "pull" },
              { state: "unreviewed", remote_revision: 3 },
            ),
            "Attic R2",
          )!]}
          subject="notes about this project activity"
          timing="standing"
          label="Notes about this project activity"
        />
      </Specimen>
      <Specimen
        when="Another computer cleared the storage, and Push is open"
        where="Standing note in the project activity notice zone, also above the Push review list. Nobody pushed since the clear, or only into areas this computer does not write. It ends by itself after the Push."
      >
        <NoticeZone
          items={[clearedStorageNotice(
            clearedComparison({ state: "remote_missing", base_revision: 7 }, { state: "fresh" }),
            "Attic R2",
          )!]}
          subject="notes about this project activity"
          timing="standing"
          label="Notes about this project activity"
        />
      </Specimen>
      <Specimen
        when="Another computer cleared the storage and pushed again"
        where="The same zone. An area this computer writes holds content it never reviewed, so a Pull comes first. The Pull review itself shows no note: it is an ordinary first Pull."
      >
        <NoticeZone
          items={[clearedStorageNotice(
            clearedComparison(
              { state: "replaced", base_revision: 7, remote_revision: 2 },
              { state: "unreviewed", remote_revision: 2 },
            ),
            "Attic R2",
          )!]}
          subject="notes about this project activity"
          timing="standing"
          label="Notes about this project activity"
        />
      </Specimen>
      <Specimen
        when="OutcomeUnknown"
        where="The shell banner under the path bar, when the storage cannot be read after a claim or a record was sent, so the one check that would settle it cannot be made."
      >
        <ShellBanner error="Couldn't confirm the push. Refresh to see whether it arrived." />
      </Specimen>
      <Specimen
        when="PushUnfinished"
        where="The shell banner under the path bar, with Finish that push: a push onto this version claimed it and never wrote its record, another computer's or this computer's own for other content. The action writes that record once and lists, and the banner then reads Finished in the done tone, or the RemoteChanged or OutcomeUnknown sentence. The Library page has the same sentences and action."
      >
        <ShellBanner error={unfinishedPushSentence("Attic R2")} />
        <ShellBanner error={ownUnfinishedPushSentence("Attic R2")} />
        <ShellBanner error={PUSH_FINISHED} />
      </Specimen>
      <Specimen
        when="A Pull refused after its review"
        where="The head of the Pull review's banner zone, with Compare again: the pinned version lost a race, the storage was cleared, or this computer synced past the review from another window (T3 to T5, T11); or the review is older than a day (T6)."
      >
        <PullBanner error={REVIEW_SUPERSEDED} />
        <PullBanner error={REVIEW_EXPIRED} />
      </Specimen>
      <Specimen
        when="A Pull that stopped"
        where="The Pull result on the storage card: the storage could not be read after the download. What was installed stays, every row not reached reads not pulled, and no line says that Push is blocked."
      >
        <SyncOutcomeNotice outcome={stoppedPull} onDismiss={() => undefined} />
      </Specimen>
      <Specimen
        when="Nothing to pull"
        where="The storage card's notice, in the note tone and with no button: a cleared storage this computer synced with before. The standing note says to push again."
      >
        <NoticeZone
          items={[{ key: "nothing-to-pull", severity: "info", message: "There is nothing to pull from Attic R2." }]}
          subject="reasons this storage is held"
          timing="blocking"
        />
      </Specimen>
      <Specimen
        when="Sessions that failed alone"
        where="Rows of the Pull result: a stored session this version of Mallard cannot read (D6), and a session that changed here after the comparison (T7). Neither offers a reset."
      >
        <SyncOutcomeNotice outcome={failedAlone} onDismiss={() => undefined} />
      </Specimen>
    </main>
  </StrictMode>,
);
