import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import type { FeishuSetupFailure, FeishuSetupLogin, StorageProviderDescriptor, StorageSetupDraft } from "../../types";
import { writeClipboardText } from "../sync-review/clipboard";
import { projectSyncApi } from "./api";
import { openUrl } from "@tauri-apps/plugin-opener";
import Icon from "../Icons";
import { dropboxFolderNameIssue } from "./DropboxSetup";
import { Step, useCountdown, type StepStatus, type StepView, type StorageSetupPort } from "./StorageSetupStep";

/** Where the sign-in stands. Each phase belongs to one line of the lane;
 *  `review` is past the last one. */
type Phase = "idle" | "login" | "account" | "folder" | "prepare" | "review";
/** The door the lane was opened from: a new folder in My Space, an existing
 *  one by its pasted link, or a fresh sign-in for a saved storage. The
 *  backend's own mode is create or reconnect; the door only decides which
 *  folder field the create sends. */
export type FeishuLaneMode = "create" | "existing" | "reconnect";
interface State {
  phase: Phase;
  running: boolean;
  cancelling: boolean;
  error: string | null;
  diagnostics: FeishuDiagnostic[] | null;
  link: string | null;
  login: FeishuSetupLogin | null;
  /** The signed-in account, kept past review for the ready line. */
  account: FeishuSetupLogin["account"] | null;
  name: string;
  /** A pasted folder link or token. */
  link_: string;
}
const idle = (): State => ({ phase: "idle", running: false, cancelling: false, error: null, diagnostics: null, link: null, login: null, account: null, name: "", link_: "" });
type Api = Pick<typeof projectSyncApi, "authorizeFeishuSetup" | "proposeFeishuFolderName" | "prepareFeishuStorage" | "cancelFeishuSetup">;

/** The folder token a pasted Drive folder link or bare token names. */
export function feishuFolderToken(value: string): string {
  const trimmed = value.trim();
  return /^https?:\/\//u.test(trimmed) ? trimmed.split("?")[0].split("#")[0].split("/").pop() ?? "" : trimmed;
}
/** A pasted Drive folder link, or a bare folder token. */
export function feishuFolderLinkIssue(value: string): string | null {
  const trimmed = value.trim();
  const token = feishuFolderToken(trimmed);
  if (/^https?:\/\//u.test(trimmed) && !/^https:\/\/[^/]+\.(feishu\.cn|larksuite\.com)\//u.test(trimmed)) return "a Feishu Drive folder link";
  return /^[A-Za-z0-9_-]{1,256}$/u.test(token) ? null : "a folder link or its token";
}

const errors: Record<string, string> = {
  feishu_authorization_timeout: "Expired after 3 minutes",
  feishu_consent_denied: "Feishu access was not approved",
  feishu_callback_unavailable: "The sign-in port is in use · close the other setup attempt",
  feishu_callback_invalid: "The sign-in callback was not valid · sign in again",
  feishu_setup_cancelled: "Stopped",
  feishu_setup_busy: "Another storage operation is running · wait, then try again",
  feishu_app_unavailable: "This build carries no Feishu app · a release build carries it; in development set MALLARD_FEISHU_APP_ID and MALLARD_FEISHU_APP_SECRET",
  feishu_app_rejected: "Feishu refused Mallard's app · update Mallard, then try again",
  feishu_scope_mismatch: "Feishu granted different permissions · ask your admin to approve Mallard's app as published",
  feishu_permission_denied: "Feishu denied access · ask your admin to install Mallard's app for you",
  feishu_reconnect_required: "Feishu rejected the sign-in · sign in again",
  feishu_request_failed: "Feishu did not answer · try again",
  feishu_response_invalid: "Feishu returned an unexpected answer · try again",
  feishu_session_expired: "The sign-in expired · sign in again",
  feishu_draft_changed: "The draft changed · sign in again for the current draft",
  feishu_folder_name_invalid: "The folder name is not valid",
  feishu_folder_link_invalid: "That is not a Feishu Drive folder link or token",
  feishu_already_connected: "This folder is already connected · open its storage settings",
  feishu_setup_identity_mismatch: "The folder or app does not match this setup",
  feishu_connection_not_saved: "Mallard could not save the sign-in on this computer",
  feishu_cancel_failed: "Could not stop the setup",
};
function errorCode(reason: unknown): string {
  const value = reason && typeof reason === "object" && "code" in reason ? reason.code : "feishu_setup_failed";
  return typeof value === "string" ? value : "feishu_setup_failed";
}
/** One bounded diagnostic field: a fixed label and an allow-listed value. */
export interface FeishuDiagnostic { label: string; value: string }
/** Display only the bounded diagnostic fields, never a response body or message. */
export function feishuSetupDiagnostics(reason: unknown): FeishuDiagnostic[] | null {
  if (!reason || typeof reason !== "object") return null;
  const failure = reason as Partial<FeishuSetupFailure>;
  const evidence = failure.evidence;
  const fields: FeishuDiagnostic[] = [];
  const endpoints = ["oauth/v3/token", "authen/v1/user_info", "drive/v1/files", "drive/v1/files/create_folder"];
  if (evidence && typeof evidence === "object" && endpoints.includes(evidence.endpoint)) {
    fields.push({ label: "Endpoint", value: evidence.endpoint });
    if (Number.isInteger(evidence.http_status) && evidence.http_status! >= 100 && evidence.http_status! <= 599) fields.push({ label: "HTTP status", value: String(evidence.http_status) });
    if (Number.isSafeInteger(failure.provider_code) && failure.provider_code! > 0) fields.push({ label: "Feishu code", value: String(failure.provider_code) });
    const tags = ["invalid_request", "invalid_client", "invalid_grant", "unauthorized_client", "unsupported_grant_type", "invalid_scope", "access_denied", "server_error", "temporarily_unavailable", "20073"];
    if (typeof evidence.error_tag === "string" && tags.includes(evidence.error_tag)) fields.push({ label: "OAuth error", value: evidence.error_tag });
    if (typeof evidence.request_id === "string" && /^[A-Za-z0-9-]{1,64}$/u.test(evidence.request_id)) fields.push({ label: "Request ID", value: evidence.request_id });
  }
  const pkce = failure.pkce;
  if (pkce && typeof pkce === "object") {
    for (const [label, value] of [
      ["PKCE challenge present", pkce.challenge_present],
      ["PKCE method S256", pkce.method_s256],
      ["PKCE verifier matches challenge", pkce.verifier_matches_challenge],
      ["Callback state verified", pkce.callback_state_verified],
    ] as const) {
      if (typeof value === "boolean") fields.push({ label, value: value ? "Yes" : "No" });
    }
    for (const [label, value] of [
      ["PKCE verifier length", pkce.verifier_length],
      ["PKCE challenge length", pkce.challenge_length],
    ] as const) {
      if (Number.isSafeInteger(value) && value >= 0 && value <= 8192) fields.push({ label, value: String(value) });
    }
  }
  return fields.length ? fields : null;
}
/** The clipboard form: one `Label: value` line per field. */
export function feishuDiagnosticsText(fields: FeishuDiagnostic[]): string {
  return fields.map(({ label, value }) => `${label}: ${value}`).join("\n");
}
/** Errors the Folder line owns. */
const FOLDER_CODES = new Set(["feishu_folder_name_invalid", "feishu_folder_link_invalid", "feishu_already_connected"]);

export function useFeishuSetup({ enabled, draftId, mode, port, api = projectSyncApi }: {
  enabled: boolean; draftId: string | null; mode: FeishuLaneMode; port: StorageSetupPort; api?: Api;
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
      if (saved) void api.cancelFeishuSetup(saved.draft_id, saved.revision).catch(() => {});
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
    patch({ phase, running: true, cancelling: false, error: null, diagnostics: null, link: null });
    try {
      // The sign-in pins this revision. Later steps must not save it again.
      const saved = phase === "login" ? await persist() : target.current;
      if (!saved || !live()) return;
      target.current = saved;
      const next = await task(saved, live);
      if (next && generation.current === token && !cancelled.current) patch(next);
    } catch (reason) {
      if (generation.current === token) patch({ error: cancelled.current ? "feishu_setup_cancelled" : errorCode(reason), diagnostics: cancelled.current ? null : feishuSetupDiagnostics(reason), link: null });
    } finally {
      if (generation.current === token) {
        running.current = false;
        patch({ running: false, cancelling: false, link: null, ...(cancelled.current ? { error: "feishu_setup_cancelled" } : {}) });
      }
    }
  };

  const start = () => {
    void run("login", async (saved, live) => {
      patch({ login: null, account: null });
      const login = await api.authorizeFeishuSetup(saved.draft_id, saved.revision, mode === "reconnect" ? "reconnect" : "create", (progress) => {
        if (live()) patch({ link: progress.url });
      });
      return live() ? { phase: "account", login, account: login.account, link: null } : null;
    });
  };

  const confirmAccount = () => {
    const login = current.current.login;
    if (!login) return;
    if (mode === "reconnect") return prepare();
    void run("folder", async (_saved, live) => {
      if (mode === "existing") return live() ? { phase: "folder" } : null;
      const kept = current.current.name;
      const name = kept || await api.proposeFeishuFolderName();
      return live() ? { phase: "folder", name } : null;
    });
  };

  const prepare = () => {
    const { login, name, link_ } = current.current;
    if (!login) return;
    if (mode === "create" && dropboxFolderNameIssue(name) || mode === "existing" && feishuFolderLinkIssue(link_)) return;
    void run("prepare", async (saved, live) => {
      await api.prepareFeishuStorage(saved.draft_id, saved.revision, login.session_id,
        mode === "create" ? name : null,
        mode === "existing" ? link_.trim() : null);
      if (!live()) return null;
      const adopted = await portRef.current.adoptPrepared(saved, live);
      return adopted ? { phase: "review", login: null } : null;
    });
  };

  const cancel = async () => {
    cancelled.current = true;
    patch({ cancelling: true, link: null });
    const saved = target.current;
    try {
      if (saved) await api.cancelFeishuSetup(saved.draft_id, saved.revision);
    } catch {
      patch({ error: "feishu_cancel_failed", diagnostics: null, cancelling: false });
    }
  };
  const dismiss = () => {
    if (running.current) return;
    generation.current += 1;
    const saved = target.current;
    if (saved) void api.cancelFeishuSetup(saved.draft_id, saved.revision).catch(() => {});
    target.current = null;
    setState(idle());
  };
  const suggest = () => void run("folder", async (_saved, live) => {
    const name = await api.proposeFeishuFolderName();
    return live() ? { name, error: null } : null;
  });
  return { state, isRunning: () => running.current, start, confirmAccount, prepare, cancel, dismiss, suggest,
    setName: (name: string) => patch({ name, error: null, phase: "folder" }),
    setLink: (link_: string) => patch({ link_, error: null, phase: "folder" }),
  };
}

const PHASES: Phase[] = ["idle", "login", "account", "folder", "prepare", "review"];
const chip = (label: string, value: string) => (
  <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);
const RESET_TITLE = "Discards this setup draft and all of its progress, then opens a fresh one. A folder already created in Feishu is kept.";

/** The Feishu lane in the R2 and Dropbox lanes' grammar: Sign in (the link
 *  copied or opened, never printed), Account, Folder (the Create-new door's
 *  proposed name, or the Use-existing door's pasted link), Access (the
 *  sign-in this computer keeps), Review. What this lane alone must say, in
 *  as few words as hold it: consent covers every Drive folder the user can
 *  reach, data never passes through Mallard, an admin installs Mallard's
 *  app (the guide's label), and the sign-in lasts a year on this computer. */
export function FeishuSetupLane({ flow, mode, descriptor, disabled, onReset }: {
  flow: ReturnType<typeof useFeishuSetup>; mode: FeishuLaneMode; descriptor: StorageProviderDescriptor;
  disabled: boolean;
  /** Reset setup: this draft goes, a fresh one opens. */
  onReset: () => void;
}) {
  const { state } = flow;
  const [copyNotice, setCopyNotice] = useState<{ text: string; url?: string } | null>(null);
  useEffect(() => { setCopyNotice(null); }, [state.link, state.phase, state.error, state.diagnostics]);
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
  const copyDiagnostics = async () => {
    if (!state.diagnostics) return;
    try {
      await writeClipboardText(feishuDiagnosticsText(state.diagnostics));
      setCopyNotice({ text: "Diagnostics copied." });
    } catch {
      setCopyNotice({ text: "Could not copy. Select and copy the diagnostics." });
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
      {copyNotice.url && <input aria-label="Feishu sign-in link" readOnly value={copyNotice.url} />}
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
  const guideButton = descriptor.setup_guide_url
    ? <button type="button" className="btn-link" onClick={() => void open(descriptor.setup_guide_url!)}>Admin guide <Icon name="external-link" size={11} /></button>
    : null;

  if (state.phase === "idle") {
    return (
      <>
        <span className="storage-setup-guide-copy">
          {mode === "reconnect" ? "Renews the sign-in for this storage’s folder. Covers all your Drive folders." : "Covers all your Drive folders. Data never passes through Mallard."}
          {" "}{guideButton}
        </span>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={() => flow.start()}>
          <Icon name="link" size={13} />{" "}
          {mode === "reconnect" ? "Reconnect Feishu" : "Connect Feishu"}
        </button>
      </>
    );
  }
  if (state.phase === "review") {
    const folder = mode === "create" ? state.name : mode === "existing" ? feishuFolderToken(state.link_) : null;
    return (
      <>
        <span className="r2-ready" role="status">
          <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
          <span className="r2-facts">
            {folder && chip("Folder", folder)}
            {state.account && chip("Account", state.account.name)}
            {chip("Access", "Drive-wide · 1 year")}
          </span>
        </span>
        {resetButton}
      </>
    );
  }

  const index = PHASES.indexOf(state.phase);
  /* Which line a failure is said on: the step the run was in, except that
     a refused folder is the Folder line's. */
  const failedKey = !state.error ? null
    : FOLDER_CODES.has(state.error) ? "folder"
    : state.phase === "login" ? "login"
    : state.phase === "account" ? "account"
    : state.phase === "folder" ? "folder"
    : "access";
  const statusOf = (key: string, first: number, last = first): StepStatus =>
    index > last ? "done" : index < first ? "todo" : failedKey === key ? "failed" : "doing";
  const reason = () => errors[state.error ?? ""] ?? "Feishu setup could not finish";
  const settled = !state.running;
  const retryButton = button("Try again", () => flow.start(), "primary");
  const changeLoginButton = button(<><Icon name="user" size={12} /> Change login</>, () => flow.start());

  const signIn: StepView = (() => {
    const status = statusOf("login", 1);
    return {
      key: "login", title: "Sign in", status,
      fact: status === "failed" ? reason()
        : status === "doing" ? (state.link ? <>Approve in your browser · <span className="r2-countdown">{budget}</span></> : "Starting sign-in…")
        : status === "done" ? "Approved · all Drive folders"
        : "In your browser",
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}{guideButton}</>
        : status === "doing" ? (state.link ? browserControls(state.link) : cancelButton)
        : settled && status === "done" && state.phase !== "account" ? changeLoginButton
        : undefined,
    };
  })();

  const account: StepView = (() => {
    const status = statusOf("account", 2);
    return {
      key: "account", title: "Account", status,
      fact: status === "failed" ? reason()
        : state.account ? state.account.name
        : "Confirmed before anything changes",
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}</>
        : status === "doing" ? <>{button("Continue", flow.confirmAccount, "primary")}{changeLoginButton}</>
        : undefined,
    };
  })();

  const folder: StepView = (() => {
    const status = mode === "reconnect" ? (index > 3 ? "done" : "todo") : statusOf("folder", 3, 3);
    const naming = mode !== "reconnect" && (state.phase === "folder" || failedKey === "folder");
    const creating = mode === "create";
    const nameIssue = dropboxFolderNameIssue(state.name);
    const linkIssue = feishuFolderLinkIssue(state.link_);
    const issue = creating ? nameIssue : linkIssue;
    const field = creating ? (
      <span className="r2-name-field">
        <input
          className="r2-name-input"
          value={state.name}
          aria-label="Feishu folder name"
          aria-invalid={nameIssue !== null}
          spellCheck={false}
          disabled={disabled}
          onChange={(event) => flow.setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && nameIssue === null) flow.prepare(); }}
        />
        {button(<><Icon name="refresh" size={12} /> Suggest</>, flow.suggest)}
      </span>
    ) : (
      <span className="r2-name-field">
        <input
          className="r2-name-input"
          value={state.link_}
          aria-label="Feishu folder link"
          aria-invalid={state.link_.length > 0 && linkIssue !== null}
          placeholder="Paste a folder link"
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          disabled={disabled}
          onChange={(event) => flow.setLink(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && linkIssue === null) flow.prepare(); }}
        />
      </span>
    );
    /* Only a refusal or a broken value is said beside the field; an empty
       link field speaks through its placeholder. */
    const note = failedKey === "folder" ? `· ${reason()}`
      : issue && (creating || state.link_.length > 0) ? `· ${issue}`
      : null;
    const token = feishuFolderToken(state.link_);
    return {
      key: "folder", title: "Folder", status: naming ? (failedKey === "folder" ? "failed" : "doing") : status,
      fact: naming ? <>{field}{note && <span>{note}</span>}</>
        : mode === "reconnect" ? "This storage’s folder"
        : state.phase === "prepare" ? (creating ? <>Creating <code>{state.name}</code>…</> : <>Checking <code>{token}</code>…</>)
        : status === "done" ? (creating ? <><code>{state.name}</code> · created</> : <code>{token}</code>)
        : creating ? "New, in My Space" : "A shared folder’s link",
      controls: naming ? button(creating ? "Create & connect" : "Use & connect", () => flow.prepare(), "primary", disabled || issue !== null)
        : mode !== "reconnect" && state.phase === "prepare" ? cancelButton
        : undefined,
    };
  })();

  const access: StepView = (() => {
    const status = statusOf("access", 4);
    return {
      key: "access", title: "Access", status,
      fact: status === "failed" ? reason()
        : status === "doing" ? (mode === "reconnect" ? "Renewing the sign-in…" : "Saving the sign-in…")
        : "Kept on this computer · 1 year",
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
      <ol className="r2-steps">{[signIn, account, folder, access, review].map((step) => <Step key={step.key} step={{ ...step, body: <>{step.body}{step.status === "failed" && state.diagnostics && (
        <details className="feishu-setup-diagnostics">
          <summary><Icon name="chevron-right" size={12} /> Diagnostics</summary>
          <div className="feishu-setup-diagnostics-sheet">
            <dl>{state.diagnostics.map(({ label, value }) => <Fragment key={label}><dt>{label}</dt><dd>{value}</dd></Fragment>)}</dl>
            <button type="button" className="feishu-setup-diagnostics-copy" aria-label="Copy diagnostics" disabled={disabled} onClick={() => void copyDiagnostics()}>
              <Icon name="copy" size={12} /> Copy
            </button>
          </div>
          {copyStatus}
        </details>
      )}</> }} />)}</ol>
    </>
  );
}
