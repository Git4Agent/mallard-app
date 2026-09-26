import { useEffect, useRef, useState, type ReactNode } from "react";
import type { DropboxSetupLogin, DropboxSetupMode, StorageDiscoveryCandidate, StorageDiscoveryResult, StorageProviderDescriptor, StorageSetupDraft, StorageSummary } from "../../types";
import { writeClipboardText } from "../sync-review/clipboard";
import { projectSyncApi } from "./api";
import { openUrl } from "@tauri-apps/plugin-opener";
import Icon from "../Icons";
import { CandidateGroups, CandidateRow, candidateSelectable, DiscoveryEmpty, ScanIncompleteNotice } from "./StorageDiscovery";
import { Step, useCountdown, type StepStatus, type StepView, type StorageSetupPort } from "./StorageSetupStep";

type Phase = "idle" | "login" | "account" | "folder" | "find" | "pick" | "prepare" | "review";
interface State {
  phase: Phase;
  running: boolean;
  cancelling: boolean;
  error: string | null;
  link: string | null;
  login: DropboxSetupLogin | null;
  /** The signed-in account, kept past review for the ready line after the
   *  session itself is dropped. */
  account: DropboxSetupLogin["account"] | null;
  name: string;
  scan: StorageDiscoveryResult | null;
  chosen: StorageDiscoveryCandidate | null;
}
const idle = (): State => ({ phase: "idle", running: false, cancelling: false, error: null, link: null, login: null, account: null, name: "", scan: null, chosen: null });
type Api = Pick<typeof projectSyncApi, "authorizeDropboxSetup" | "proposeDropboxFolderName" | "discoverDropboxStorage" | "prepareDropboxStorage" | "cancelDropboxSetup">;

export function dropboxFolderNameIssue(name: string): string | null {
  return !name || new TextEncoder().encode(name).length > 255 || name.trim() !== name
    || name === "." || name === ".." || /[\x00-\x1f\x7f-\x9f/\\]/u.test(name)
    ? "one name, up to 255 bytes, no slashes or control characters" : null;
}

const errors: Record<string, string> = {
  dropbox_authorization_timeout: "Expired after 3 minutes",
  dropbox_consent_denied: "Dropbox access was not approved",
  dropbox_callback_unavailable: "The authorization port is in use · close the other setup attempt",
  dropbox_setup_cancelled: "Stopped",
  dropbox_new_setup_required: "This setup already has its folder · start a new setup for another account",
  dropbox_folder_name_invalid: "The folder name is not valid",
  dropbox_session_expired: "The sign-in expired · sign in again",
  dropbox_scan_expired: "Scan expired · sign in and scan again",
  dropbox_folder_changed: "The folder changed or could not be verified · scan again",
  dropbox_draft_changed: "The draft changed · sign in again for the current draft",
  dropbox_already_connected: "This folder is already connected · open its storage settings",
  dropbox_reconnect_required: "Dropbox access is no longer valid · reconnect to continue",
  dropbox_permission_denied: "Dropbox denied access · check the account and folder permissions",
  dropbox_setup_share_pending: "Dropbox is still preparing the folder · find it in your Dropbox once it appears, or choose another name",
  dropbox_cancel_failed: "Could not stop the setup",
};
function errorCode(reason: unknown): string {
  const value = reason && typeof reason === "object" && "code" in reason ? reason.code : "dropbox_setup_failed";
  return typeof value === "string" ? value : "dropbox_setup_failed";
}
/** The error that ends a run for good: no Try again on the line. */
const FINAL_CODES = new Set(["dropbox_new_setup_required"]);

export function useDropboxSetup({ enabled, draftId, mode, port, api = projectSyncApi }: {
  enabled: boolean; draftId: string | null; mode: DropboxSetupMode; port: StorageSetupPort; api?: Api;
}) {
  const [state, setState] = useState<State>(idle);
  const current = useRef(state);
  current.current = state;
  const portRef = useRef(port);
  portRef.current = port;
  const running = useRef(false);
  const cancelled = useRef(false);
  const generation = useRef(0);
  const target = useRef<StorageSetupDraft | null>(null);
  const patch = (update: Partial<State>) => setState((state) => ({ ...state, ...update }));

  useEffect(() => {
    setState(idle());
    return () => {
      generation.current += 1;
      cancelled.current = true;
      running.current = false;
      const saved = target.current;
      if (saved) void api.cancelDropboxSetup(saved.draft_id, saved.revision).catch(() => {});
      target.current = null;
    };
  }, [enabled, draftId, mode, api]);

  const run = async (phase: Phase, task: (saved: StorageSetupDraft, live: () => boolean) => Promise<Partial<State> | null>) => {
    if (!enabled || running.current || portRef.current.blocked()) return;
    running.current = true;
    cancelled.current = false;
    const token = ++generation.current;
    const { persist, isCurrent } = portRef.current.beginRun();
    const live = () => generation.current === token && !cancelled.current && isCurrent();
    portRef.current.onRunStarted();
    patch({ phase, running: true, cancelling: false, error: null, link: null });
    try {
      // Consent pins this revision. Later steps must not save it again.
      const saved = phase === "login" ? await persist() : target.current;
      if (!saved || !live()) return;
      target.current = saved;
      const next = await task(saved, live);
      // Adoption reseats the draft, so its own guards decide whether it succeeded.
      if (next && generation.current === token && !cancelled.current) patch(next);
    } catch (reason) {
      if (generation.current === token) patch({ error: cancelled.current ? "dropbox_setup_cancelled" : errorCode(reason), link: null });
    } finally {
      if (generation.current === token) {
        running.current = false;
        patch({ running: false, cancelling: false, link: null, ...(cancelled.current ? { error: "dropbox_setup_cancelled" } : {}) });
      }
    }
  };

  const start = (switchAccount = false) => {
    void run("login", async (saved, live) => {
      patch({ login: null, account: null, scan: null, chosen: null });
      const login = await api.authorizeDropboxSetup(saved.draft_id, saved.revision, mode, switchAccount, (progress) => {
        if (live()) patch({ link: progress.url });
      });
      return live() ? { phase: "account", login, account: login.account, link: null } : null;
    });
  };

  const confirmAccount = () => {
    const login = current.current.login;
    if (!login) return;
    if (mode === "existing") return scan();
    if (mode === "reconnect") return prepare();
    void run("folder", async (_saved, live) => {
      const kept = current.current.name;
      const name = kept || await api.proposeDropboxFolderName();
      return live() ? { phase: "folder", name } : null;
    });
  };

  const scan = () => {
    const login = current.current.login;
    if (!login) return;
    void run("find", async (saved, live) => {
      patch({ scan: null, chosen: null });
      const scan = await api.discoverDropboxStorage(saved.draft_id, saved.revision, login.session_id);
      return live() ? { phase: "pick", scan } : null;
    });
  };

  const prepare = (candidate?: StorageDiscoveryCandidate) => {
    const { login, scan, name } = current.current;
    if (!login || mode === "create" && dropboxFolderNameIssue(name)
      || mode === "existing" && (!candidate || !scan || !candidateSelectable(candidate))) return;
    void run("prepare", async (saved, live) => {
      patch({ chosen: candidate ?? null });
      await api.prepareDropboxStorage(saved.draft_id, saved.revision, login.session_id,
        mode === "create" ? name : null, candidate ? scan!.scan_id : null, candidate?.candidate_id ?? null);
      if (!live()) return null;
      const adopted = await portRef.current.adoptPrepared(saved, live);
      return adopted ? { phase: "review", login: null, scan: null } : null;
    });
  };

  const cancel = async () => {
    cancelled.current = true;
    patch({ cancelling: true, link: null });
    const saved = target.current;
    try {
      if (saved) await api.cancelDropboxSetup(saved.draft_id, saved.revision);
    } catch {
      patch({ error: "dropbox_cancel_failed", cancelling: false });
    }
  };
  const dismiss = () => {
    if (running.current) return;
    generation.current += 1;
    const saved = target.current;
    if (saved) void api.cancelDropboxSetup(saved.draft_id, saved.revision).catch(() => {});
    target.current = null;
    setState(idle());
  };
  const suggest = () => void run("folder", async (_saved, live) => {
    const name = await api.proposeDropboxFolderName();
    return live() ? { name, error: null } : null;
  });
  return { state, isRunning: () => running.current, start, confirmAccount, scan, prepare, cancel, dismiss, suggest,
    setName: (name: string) => patch({ name, error: null, phase: "folder" }),
  };
}

const PHASES: Phase[] = ["idle", "login", "account", "folder", "find", "pick", "prepare", "review"];
const chip = (label: string, value: string) => (
  <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);
const RESET_TITLE = "Discards this setup draft and all of its progress, then opens a fresh one. A folder already created in Dropbox is kept.";

/** The Dropbox lane in the R2 lane's grammar: one line per step with its
 *  glyph, fact and the controls that step owns; a failure said on its own
 *  line; the browser step's link copied or opened, never printed; Reset
 *  setup at the head; the ready sentence with its facts at review. What
 *  Dropbox alone must say — consent covers the whole account, not the
 *  folder — is said before start, on the Sign in line, and in the ready
 *  line's Access chip. */
export function DropboxSetupLane({ flow, mode, descriptor, storages, disabled, onOpenStorage, onCreateInstead, onNewSetup, onReset }: {
  flow: ReturnType<typeof useDropboxSetup>; mode: DropboxSetupMode; descriptor: StorageProviderDescriptor;
  storages: readonly StorageSummary[]; disabled: boolean; onOpenStorage: (id: string) => void;
  onCreateInstead: () => void;
  /** A fresh draft for another account, keeping this one and its folder. */
  onNewSetup: () => void;
  /** Reset setup: this draft goes, a fresh one opens on the doors. */
  onReset: () => void;
}) {
  const { state } = flow;
  const [copyNotice, setCopyNotice] = useState<{ text: string; url?: string } | null>(null);
  useEffect(() => { setCopyNotice(null); }, [state.link, state.phase, state.error]);
  const budget = useCountdown(state.link, 180);
  const button = (label: ReactNode, onClick: () => void, tone: "plain" | "primary" | "ghost" = "plain", inert = disabled) => (
    <button
      type="button"
      className={`btn storage-setup-guide-button${tone === "primary" ? " btn-primary" : tone === "ghost" ? " btn-ghost" : ""}`}
      disabled={inert}
      onClick={onClick}
    >
      {label}
    </button>
  );
  /* Cancel belongs to the run, so it ignores the page's lock. */
  const cancelButton = button(state.cancelling ? "Cancelling…" : "Cancel", () => void flow.cancel(), "ghost", state.cancelling);
  const copy = async (url: string) => {
    try {
      await writeClipboardText(url);
      setCopyNotice({ text: "Link copied." });
    } catch {
      setCopyNotice({ text: "Could not copy. Select and copy the link.", url });
    }
  };
  const open = async (url: string) => {
    try {
      await openUrl(url);
    } catch {
      setCopyNotice({ text: "Could not open a browser. Copy the link into a browser on this computer.", url });
    }
  };
  const copyStatus = copyNotice && (
    <>
      <span role="status">{copyNotice.text}</span>
      {copyNotice.url && <input aria-label="Dropbox authorization link" readOnly value={copyNotice.url} />}
    </>
  );
  /* The browser step is the user's turn, in whichever browser or profile
     they choose: the link is theirs to copy; opening it here is the shortcut. */
  const browserControls = (url: string) => (
    <>
      {button(<><Icon name="copy" size={12} /> Copy link</>, () => void copy(url), "primary", false)}
      {button(<><Icon name="external-link" size={12} /> Open browser</>, () => void open(url), "plain", false)}
      {cancelButton}
      {copyStatus}
    </>
  );
  const resetButton = (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" disabled={disabled} title={RESET_TITLE} onClick={onReset}>
      <Icon name="refresh" size={12} /> Reset setup
    </button>
  );

  if (state.phase === "idle") {
    return (
      <>
        <span className="storage-setup-guide-copy">
          {mode === "create" ? "Covers your whole Dropbox account."
            : mode === "existing" ? "Lists your shared folders, read-only. Covers your whole Dropbox account."
            : "Renews access to this storage’s folder. Covers your whole Dropbox account."}
        </span>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={() => flow.start()}>
          <Icon name="link" size={13} />{" "}
          {mode === "create" ? "Connect Dropbox" : mode === "existing" ? "Find in my Dropbox" : "Reconnect Dropbox"}
        </button>
      </>
    );
  }
  if (state.phase === "review") {
    const folder = state.chosen?.name ?? (mode === "create" ? state.name : null);
    return (
      <>
        <span className="r2-ready" role="status">
          <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
          <span className="r2-facts">
            {folder && chip("Folder", folder)}
            {state.chosen?.identity && chip("Namespace", state.chosen.identity)}
            {state.account && chip("Account", state.account.name || state.account.email)}
            {chip("Access", "account-wide · renews")}
          </span>
        </span>
        {resetButton}
      </>
    );
  }

  const index = PHASES.indexOf(state.phase);
  const taken = state.error === "dropbox_folder_name_taken";
  /* Which line a failure is said on: the step the run was in, except that a
     taken name is the Folder line's to fix and a refused account switch is
     the Sign in line's to explain. */
  const failedKey = !state.error || taken ? null
    : state.error === "dropbox_new_setup_required" ? "login"
    : state.phase === "login" ? "login"
    : state.phase === "account" ? "account"
    : state.phase === "prepare" && mode !== "create" ? "access"
    : "folder";
  const statusOf = (key: string, first: number, last = first): StepStatus =>
    index > last ? "done" : index < first ? "todo" : failedKey === key ? "failed" : "doing";
  const reason = () => errors[state.error ?? ""] ?? "Dropbox setup could not finish";
  const settled = !state.running;
  const retryButton = state.error && FINAL_CODES.has(state.error) ? null : button("Try again", () => flow.start(), "primary");
  const newSetupButton = mode === "create" && state.error === "dropbox_new_setup_required"
    ? button("New setup for another account", onNewSetup) : null;
  /* A draft that already holds its folder is fixed to it, so the way to
     another account is a new setup, offered where Change login was. */
  const changeLoginButton = newSetupButton ?? button(<><Icon name="user" size={12} /> Change login</>, () => flow.start(true));

  const signIn: StepView = (() => {
    const status = statusOf("login", 1);
    return {
      key: "login", title: "Sign in", status,
      fact: status === "failed" ? reason()
        : status === "doing" ? (state.link ? <>Approve in your browser · <span className="r2-countdown">{budget}</span></> : "Starting sign-in…")
        : "Approved · whole account",
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}</>
        : status === "doing" ? (state.link ? browserControls(state.link) : cancelButton)
        : settled && state.phase !== "account" ? changeLoginButton
        : undefined,
    };
  })();

  const account: StepView = (() => {
    const status = statusOf("account", 2);
    return {
      key: "account", title: "Account", status,
      fact: status === "failed" ? reason()
        : state.account ? <>{state.account.name || state.account.email} · <code>{state.account.email}</code></>
        : "Confirmed before anything changes",
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}</>
        : status === "doing" ? <>{button("Continue", flow.confirmAccount, "primary")}{changeLoginButton}</>
        : undefined,
    };
  })();

  const folder: StepView = mode === "existing" ? (() => {
    const status = statusOf("folder", 4, 5);
    const result = state.scan;
    const found = result ? result.candidates.filter(candidateSelectable).length : 0;
    /* A changed folder wants a fresh scan of the same session; an expired
       scan wants the sign-in again, which Try again is. */
    const expired = state.error === "dropbox_folder_changed";
    const scanAgainButton = button(<><Icon name="refresh" size={12} /> Scan again</>, flow.scan);
    return {
      key: "folders", title: "Folders", status,
      fact: status === "failed" ? reason()
        : state.phase === "find" ? "Scanning · read-only"
        : result && status === "doing" ? <>{result.candidates.length} checked · read-only</>
        : status === "done" && state.chosen ? <><code>{state.chosen.name}</code>{state.chosen.identity && <> · {state.chosen.identity}</>}</>
        : "Your shared folders · read-only",
      controls: status === "failed" ? (expired ? scanAgainButton : retryButton)
        : state.phase === "find" ? cancelButton
        : state.phase === "pick" ? scanAgainButton
        : undefined,
      body: result && state.phase === "pick" && !state.error ? (
        <div className="r2-step-list">
          {result.status === "partial" && <ScanIncompleteNotice issues={result.issues} />}
          <CandidateGroups
            result={result}
            unitPlural="shared folders"
            choice={false}
            row={(candidate) => (
              <CandidateRow
                key={candidate.candidate_id}
                descriptor={descriptor}
                candidate={candidate}
                disabled={disabled}
                connectedName={candidate.connected_storage_id
                  ? storages.find((storage) => storage.storage_id === candidate.connected_storage_id)?.name ?? null
                  : null}
                verb={candidateSelectable(candidate)
                  ? button("Use & connect", () => flow.prepare(candidate), "primary")
                  : undefined}
                onOpenStorage={onOpenStorage}
              />
            )}
          />
          {found === 0 && (
            <DiscoveryEmpty
              place="your Dropbox"
              unit="shared folder"
              sentence={`${result.candidates.length} shared folder${result.candidates.length === 1 ? "" : "s"} checked. Nothing was changed.`}
              disabled={disabled}
              onCreateInstead={onCreateInstead}
            />
          )}
        </div>
      ) : undefined,
    };
  })() : (() => {
    /* Creating the folder is this line's work, so it runs through prepare. */
    const status = mode === "reconnect" ? statusOf("folder", 3, 5) : statusOf("folder", 3, 6);
    /* Until a folder exists the name is the user's: proposed in the field,
       editable, and back in the field with the reason when it was taken. */
    const naming = mode === "create" && (state.phase === "folder" || taken);
    const issue = dropboxFolderNameIssue(state.name);
    const nameField = (
      <span className="r2-name-field">
        <input
          className="r2-name-input"
          value={state.name}
          aria-label="Dropbox folder name"
          aria-invalid={issue !== null}
          spellCheck={false}
          disabled={disabled}
          onChange={(event) => flow.setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && issue === null) flow.prepare(); }}
        />
        {button(<><Icon name="refresh" size={12} /> Suggest</>, flow.suggest)}
      </span>
    );
    /* Only a taken or broken name is said beside the field. */
    const nameNote = taken ? "· already exists in your Dropbox · choose another name"
      : issue ? `· ${issue}`
      : null;
    return {
      key: "folder", title: "Folder", status: naming ? "doing" : status,
      fact: naming ? <>{nameField}{nameNote && <span>{nameNote}</span>}</>
        : status === "failed" ? reason()
        : mode === "reconnect" ? "This storage’s folder"
        : state.phase === "prepare" ? <>Creating <code>{state.name}</code>…</>
        : status === "done" ? <><code>{state.name}</code> · created</>
        : "New shared folder",
      controls: naming ? button("Create & connect", () => flow.prepare(), "primary", disabled || issue !== null)
        : status === "failed" ? retryButton
        : mode === "create" && state.phase === "prepare" ? cancelButton
        : undefined,
    };
  })();

  const access: StepView = (() => {
    const status = mode === "create" ? (index > 6 ? "done" : "todo") : statusOf("access", 6);
    return {
      key: "access", title: "Access", status,
      fact: status === "failed" ? reason()
        : status === "doing" ? (mode === "reconnect" ? "Renewing access…" : <>Connecting <code>{state.chosen?.name}</code>…</>)
        : status === "done" ? "Saved · renews automatically"
        : "Renews automatically · whole account",
      controls: status === "failed" ? retryButton : status === "doing" ? cancelButton : undefined,
    };
  })();

  const review: StepView = {
    key: "review", title: "Review", status: "todo",
    fact: mode === "reconnect" ? "Test, then Save credential" : "Test, then Finish setup",
  };

  return (
    <>
      {resetButton}
      <ol className="r2-steps">{[signIn, account, folder, access, review].map((step) => <Step key={step.key} step={step} />)}</ol>
    </>
  );
}
