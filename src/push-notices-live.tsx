import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Notice, { NoticeZone } from "./components/notice/Notice";
import { clearedStorageNotice } from "./components/project-sync/clearedStorageNotice";
import { pushOutrankedNotice } from "./components/project-sync/syncOutcome";
import type { ScopeContinuity, ThreadSyncComparison } from "./types";
import "./App.css";

/* A dev harness: the three notices of DESIGN_CAS_FREE_STORAGE_REFACTOR.md
   section 13 and the standing notice of PLAN_CLEARED_STORAGE_PUSH_AGAIN.md
   section 5, drawn by the real components in the surfaces they already live
   in. Open /push-notices-live.html on the Vite server. */

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
        where="Push review error banner. Another computer pushed on the same version while this push ran: seen in the listing before the write, or selected by the listing after it. The push failed as a whole."
      >
        <Notice severity="danger" timing="blocking" className="sync-pull-error">
          The storage changed while pushing. Compare again.
        </Notice>
      </Specimen>
      <Specimen
        when="A confirmed push was outranked afterwards"
        where="Shell notice under the path bar, once, on the refresh that rolled the link back. The sessions show as not pushed again."
      >
        <Notice severity="warning" timing="progress">
          {pushOutrankedNotice("Attic R2")}
        </Notice>
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
        where="Push review error banner, when the storage cannot be reached after the record was sent."
      >
        <Notice severity="danger" timing="blocking" className="sync-pull-error">
          Couldn&apos;t confirm the push. Refresh to see whether it arrived.
        </Notice>
      </Specimen>
    </main>
  </StrictMode>,
);
