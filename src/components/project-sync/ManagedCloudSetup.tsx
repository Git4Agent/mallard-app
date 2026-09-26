import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ManagedCloudSetupLogin, ManagedCloudSetupMode, StorageSetupDraft, StorageSummary } from "../../types";
import { writeClipboardText } from "../sync-review/clipboard";
import { projectSyncApi } from "./api";
import { openUrl } from "@tauri-apps/plugin-opener";
import Icon from "../Icons";
import { Step, useCountdown, type StepStatus, type StepView, type StorageSetupPort } from "./StorageSetupStep";

/** Where the sign-in stands: one run signs in and saves the account, so
 *  the lane is either waiting on the browser or past its last line. */
type Phase = "idle" | "login" | "review";
interface State {
  phase: Phase;
  running: boolean;
  cancelling: boolean;
  error: string | null;
  link: string | null;
  /** The signed-in account: the one the draft now holds, or the one a
   *  failure is about (already connected, or not this storage's). */
  account: ManagedCloudSetupLogin["account"] | null;
  serviceOrigin: string | null;
}
const idle = (): State => ({ phase: "idle", running: false, cancelling: false, error: null, link: null, account: null, serviceOrigin: null });
type Api = Pick<typeof projectSyncApi, "signInManagedCloudStorage" | "cancelManagedCloudSetup">;

const errors: Record<string, string> = {
  managed_cloud_authorization_timeout: "Expired after 3 minutes",
  managed_cloud_consent_denied: "GitHub sign-in was not approved",
  managed_cloud_callback_unavailable: "No loopback port was free · close the other setup attempt",
  managed_cloud_callback_invalid: "The sign-in callback was not valid · sign in again",
  managed_cloud_setup_cancelled: "Stopped",
  managed_cloud_setup_busy: "Another storage operation is running · wait, then try again",
  managed_cloud_draft_changed: "The draft changed · sign in again for the current draft",
  managed_cloud_account_mismatch: "This storage belongs to another GitHub account · sign in with that account",
  storage_managed_account_exists: "This account is already connected · open its storage",
  managed_cloud_sign_in_rejected: "Mallard Cloud rejected the sign-in · try again",
  managed_cloud_permission_denied: "Mallard Cloud refused the account · it may be suspended",
  managed_cloud_unreachable: "Mallard Cloud could not be reached · check the network",
  managed_cloud_service_rejected: "Mallard Cloud rejected the request",
  managed_cloud_cancel_failed: "Could not stop the setup",
};
const ACCOUNT_CODES = new Set(["managed_cloud_account_mismatch", "storage_managed_account_exists"]);
function errorCode(reason: unknown): string {
  const value = reason && typeof reason === "object" && "code" in reason ? reason.code : "managed_cloud_setup_failed";
  return typeof value === "string" ? value : "managed_cloud_setup_failed";
}
/** The account a failure after sign-in is about, when the backend named it. */
function failedAccount(reason: unknown): ManagedCloudSetupLogin["account"] | null {
  const account = reason && typeof reason === "object" && "account" in reason ? reason.account : null;
  return account && typeof account === "object" && "id" in account && "login" in account
    && typeof account.id === "string" && typeof account.login === "string" ? { id: account.id, login: account.login } : null;
}

export function useManagedCloudSetup({ enabled, draftId, mode, port, api = projectSyncApi }: {
  enabled: boolean; draftId: string | null; mode: ManagedCloudSetupMode; port: StorageSetupPort; api?: Api;
}) {
  const [state, setState] = useState<State>(idle);
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
      if (saved) void api.cancelManagedCloudSetup(saved.draft_id, saved.revision).catch(() => {});
      target.current = null;
    };
  }, [enabled, draftId, mode, api]);

  /* One run: save the draft, sign in through the browser, and let the
     backend write the account and its token into that exact revision. */
  const start = () => {
    if (!enabled || running.current || portRef.current.blocked()) return;
    running.current = true;
    cancelled.current = false;
    const token = ++generation.current;
    const { persist, isCurrent } = portRef.current.beginRun();
    const live = () => generation.current === token && !cancelled.current && isCurrent();
    portRef.current.onRunStarted();
    patch({ phase: "login", running: true, cancelling: false, error: null, link: null, account: null, serviceOrigin: null });
    void (async () => {
      try {
        const saved = await persist();
        if (!saved || !live()) return;
        target.current = saved;
        const login = await api.signInManagedCloudStorage(saved.draft_id, saved.revision, mode, (progress) => {
          if (live()) patch({ link: progress.url });
        });
        if (!live()) return;
        patch({ account: login.account, serviceOrigin: login.service_origin, link: null });
        const adopted = await portRef.current.adoptPrepared(saved, live);
        if (adopted && live()) patch({ phase: "review" });
      } catch (reason) {
        if (generation.current === token) {
          patch({ error: cancelled.current ? "managed_cloud_setup_cancelled" : errorCode(reason), account: failedAccount(reason), link: null });
        }
      } finally {
        if (generation.current === token) {
          running.current = false;
          patch({ running: false, cancelling: false, link: null, ...(cancelled.current ? { error: "managed_cloud_setup_cancelled" } : {}) });
        }
      }
    })();
  };

  const cancel = async () => {
    cancelled.current = true;
    patch({ cancelling: true, link: null });
    const saved = target.current;
    try {
      if (saved) await api.cancelManagedCloudSetup(saved.draft_id, saved.revision);
    } catch {
      patch({ error: "managed_cloud_cancel_failed", cancelling: false });
    }
  };
  const dismiss = () => {
    if (running.current) return;
    generation.current += 1;
    const saved = target.current;
    if (saved) void api.cancelManagedCloudSetup(saved.draft_id, saved.revision).catch(() => {});
    target.current = null;
    setState(idle());
  };
  return { state, isRunning: () => running.current, start, cancel, dismiss };
}

const chip = (label: string, value: string) => (
  <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);
const RESET_TITLE = "Discards this setup draft and all of its progress, then opens a fresh one. Nothing on Mallard Cloud changes.";

/** The Mallard Cloud lane in the R2 lane's grammar: Sign in (the link
 *  copied or opened, never printed), Account (whose session the draft
 *  keeps), Review. What this lane alone must say: what collaborators can
 *  read, and that a release makes a snapshot public. */
export function ManagedCloudSetupLane({ flow, mode, storages, disabled, onOpenStorage, onReset }: {
  flow: ReturnType<typeof useManagedCloudSetup>; mode: ManagedCloudSetupMode;
  storages: readonly StorageSummary[]; disabled: boolean; onOpenStorage: (id: string) => void;
  /** Reset setup: this draft goes, a fresh one opens. */
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
      {copyNotice.url && <input aria-label="Mallard Cloud sign-in link" readOnly value={copyNotice.url} />}
    </>
  );
  /* The browser step is the user's turn, in whichever browser or profile
     they choose: the link is theirs to copy; opening it here is the shortcut. */
  const browserControls = (url: string) => (
    <>
      {button(<><Icon name="copy" size={12} /> Copy sign-in link</>, () => void copy(url), "primary", false)}
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
          {mode === "create"
            ? "Signs in with GitHub in a browser you choose and keeps a 90-day session to Mallard Cloud. Pushes stay private to you and the collaborators you invite; a snapshot becomes public only when you request a release on the website."
            : "Renews this storage’s session with the same GitHub account. Sign-in happens in a browser you choose."}
        </span>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={() => flow.start()}>
          <Icon name="link" size={13} />{" "}
          {mode === "create" ? "Sign in with GitHub" : "Reconnect Mallard Cloud"}
        </button>
      </>
    );
  }
  if (state.phase === "review") {
    return (
      <>
        <span className="r2-ready" role="status">
          <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
          <span className="r2-facts">
            {state.account && chip("Account", state.account.login)}
            {state.serviceOrigin && chip("Service", state.serviceOrigin.replace(/^https?:\/\//, ""))}
            {chip("Access", "90-day session · private pushes")}
          </span>
        </span>
        {resetButton}
      </>
    );
  }

  /* One run covers both lines: a failure about the account is the Account
     line's to explain, after a sign-in that did happen; any other failure
     is the Sign in line's. */
  const accountFailed = state.error !== null && ACCOUNT_CODES.has(state.error);
  const reason = () => errors[state.error ?? ""] ?? "Mallard Cloud setup could not finish";
  const retryButton = button("Try again", () => flow.start(), "primary");
  const changeLoginButton = button(<><Icon name="user" size={12} /> Change login</>, () => flow.start());
  const existing = state.error === "storage_managed_account_exists" && state.account
    ? storages.find((storage) => storage.provider === "managed_cloud" && storage.settings.account_id === state.account?.id) ?? null
    : null;

  const signIn: StepView = (() => {
    const status: StepStatus = accountFailed ? "done" : state.error ? "failed" : "doing";
    return {
      key: "login", title: "Sign in", status,
      fact: status === "failed" ? reason()
        : status === "doing" ? (state.link ? <>Sign in with GitHub in a browser on this computer · <span className="r2-countdown">{budget}</span></> : "Starting sign-in…")
        : "Signed in with GitHub",
      controls: status === "failed" ? retryButton
        : status === "doing" ? (state.link ? browserControls(state.link) : cancelButton)
        : undefined,
    };
  })();

  const account: StepView = (() => {
    const status: StepStatus = accountFailed ? "failed" : "todo";
    return {
      key: "account", title: "Account", status,
      fact: status === "failed" ? reason()
        : mode === "reconnect" ? "This storage’s account · a renewed 90-day session"
        : "Whichever GitHub account signs in · a 90-day session, private to you and your collaborators",
      controls: status === "failed"
        ? <>{existing ? button(`Open ${existing.name}`, () => onOpenStorage(existing.storage_id), "primary") : retryButton}{changeLoginButton}</>
        : undefined,
    };
  })();

  const review: StepView = {
    key: "review", title: "Review", status: "todo",
    fact: mode === "reconnect" ? "Test connection, then Save storage" : "Test connection, then Finish setup",
  };

  return (
    <>
      {resetButton}
      <ol className="r2-steps">{[signIn, account, review].map((step) => <Step key={step.key} step={step} />)}</ol>
    </>
  );
}
