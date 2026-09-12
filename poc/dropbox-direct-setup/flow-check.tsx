// Browser regression fixture. Every API call is synthetic; no native/cloud access.
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { useDropboxSetup } from "../../src/components/project-sync/DropboxSetup";
import type { DropboxSetupMode, StorageSetupDraft } from "../../src/types";

function Check({ mode }: { mode: DropboxSetupMode }) {
  const calls = useRef({ saves: 0, scans: 0, prepares: 0, authorizedRevision: 0 });
  const [result, setResult] = useState("Running");
  const draft = useRef<StorageSetupDraft>({ draft_id: `draft-${mode}`, storage_id: `storage-${mode}`, name: "Fixture", provider: "dropbox", config_version: 1, provider_label: "Dropbox", available: true, settings: {}, secrets: [], edits_existing: mode === "reconnect", revision: 1, created_at: 1, updated_at: 1 });
  const assertRevision = (revision: number) => {
    if (revision !== calls.current.authorizedRevision) throw { code: "dropbox_draft_changed" };
  };
  const api = useRef({
    authorizeDropboxSetup: async (_id: string, revision: number) => {
      calls.current.authorizedRevision = revision;
      return { session_id: "session-fixture", account: { id: "dbid:fixture", name: "Test", email: "test@example.test" }, folder_name: null, creation_started: false };
    },
    proposeDropboxFolderName: async () => "Mallard Fixture",
    discoverDropboxStorage: async (_id: string, revision: number) => {
      assertRevision(revision); calls.current.scans += 1;
      return { scan_id: "scan-fixture", scope: "Fixture", status: "complete" as const, issues: [], candidates: [{ candidate_id: "0", name: "Folder", marker: "storage" as const, facts: [], cautions: [] }] };
    },
    prepareDropboxStorage: async (_id: string, revision: number) => {
      assertRevision(revision); calls.current.prepares += 1;
    },
    cancelDropboxSetup: async () => undefined,
  }).current;
  const flow = useDropboxSetup({ enabled: true, draftId: draft.current.draft_id, mode, api,
    port: { blocked: () => false, onRunStarted: () => undefined,
      beginRun: () => ({ isCurrent: () => true, persist: async () => {
        calls.current.saves += 1;
        draft.current = { ...draft.current, revision: draft.current.revision + 1 };
        return draft.current;
      }}),
      adoptPrepared: async () => true,
    },
  });
  const started = useRef(false);
  useEffect(() => {
    if (!started.current) { started.current = true; flow.start(); return; }
    if (flow.state.running) return;
    if (flow.state.error) { setResult(`FAIL: ${flow.state.error}`); return; }
    if (flow.state.phase === "account") flow.confirmAccount();
    if (flow.state.phase === "folder") flow.prepare();
    if (flow.state.phase === "pick") flow.prepare(flow.state.scan!.candidates[0]);
    if (flow.state.phase === "review") {
      const { saves, prepares, scans } = calls.current;
      setResult(saves === 1 && prepares === 1 && scans === Number(mode === "existing")
        ? "PASS: one saved revision from consent through review" : `FAIL: saves=${saves}, prepares=${prepares}, scans=${scans}`);
    }
  }, [flow.state]);
  return <p>{mode}: {result}</p>;
}
function CancelCheck() {
  const [result, setResult] = useState("Running");
  const release = useRef<(() => void) | null>(null);
  const calls = useRef({ authorizations: 0, cancellations: 0, adoptions: 0 });
  const draft = { draft_id: "cancel-fixture", storage_id: "fixture", name: "Fixture", provider: "dropbox", config_version: 1, provider_label: "Dropbox", available: true, settings: {}, secrets: [], edits_existing: false, revision: 2, created_at: 1, updated_at: 1 };
  const api = useRef({
    authorizeDropboxSetup: async () => {
      calls.current.authorizations += 1;
      await new Promise<void>(resolve => { release.current = resolve; });
      return { session_id: "late", account: { id: "dbid:fixture", name: "Test", email: "test@example.test" }, folder_name: null, creation_started: false };
    },
    proposeDropboxFolderName: async () => "Unused",
    discoverDropboxStorage: async () => { throw new Error("unexpected scan"); },
    prepareDropboxStorage: async () => { throw new Error("unexpected preparation"); },
    cancelDropboxSetup: async () => { calls.current.cancellations += 1; release.current?.(); },
  }).current;
  const flow = useDropboxSetup({ enabled:true, draftId:draft.draft_id,mode:"create",api,
    port: { blocked:()=>false,onRunStarted:()=>undefined,beginRun:()=>({isCurrent:()=>true,persist:async()=>draft}),
      adoptPrepared:async()=>{ calls.current.adoptions += 1; return true; } } });
  const started = useRef(false);
  useEffect(()=>{
    if (!started.current) {
      started.current=true; flow.start(); flow.start();
      setTimeout(()=>void flow.cancel(), 20);
      return;
    }
    if (!flow.state.running && flow.state.error === "dropbox_setup_cancelled") {
      setResult(calls.current.authorizations === 1 && calls.current.cancellations === 1 && calls.current.adoptions === 0 && flow.state.login === null
        ? "PASS: double click is single-flight; late consent is discarded" : "FAIL: cancelled consent was adopted");
    }
  },[flow.state]);
  return <p>cancel: {result}</p>;
}
function App() { return <><h1>Dropbox setup flow check</h1><p>Synthetic APIs only. No Dropbox consent or file changes.</p>{(["create", "existing", "reconnect"] as const).map(mode => <Check key={mode} mode={mode} />)}<CancelCheck /></>; }
createRoot(document.getElementById("root")!).render(<App />);
