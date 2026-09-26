import { Step, useCountdown, type StepView, type StepStatus, type StorageSetupPort } from "./StorageSetupStep";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type {
  R2Account,
  R2Jurisdiction,
  R2SetupErrorCode,
  R2SetupProgress,
  StorageDiscoveryCandidate,
  StorageDiscoveryResult,
  StorageProviderDescriptor,
  StorageSetupDraft,
  StorageSummary,
} from "../../types";
import Icon from "../Icons";
import { writeClipboardText } from "../sync-review/clipboard";
import { projectSyncApi } from "./api";
import { openExternalUrl } from "./externalLink";
import {
  CandidateGroups,
  CandidateRow,
  candidateSelectable,
  DiscoveryEmpty,
  ScanIncompleteNotice,
} from "./StorageDiscovery";

/**
 * The Wrangler lane of an R2 draft: six steps in view, one line each, fed by
 * the packaged helper's progress as it happens. The hook owns the hand-off —
 * which step the helper is on, what it reported, how each step is retried
 * or stopped; the lane only draws it. The workspace keeps the draft: it
 * saves it before every run, seats what the helper prepared, and discards
 * it on Reset.
 */

export const WRANGLER_INSTALL_COMMAND = "npm install --global wrangler@latest";
export const WRANGLER_INSTALL_AGENT_MESSAGE =
  "Install Cloudflare Wrangler globally with `npm install --global wrangler@latest`. "
  + "Check `npm --version` first. If npm is missing, tell me that Node.js and npm are required. "
  + "After installation, verify `wrangler --version`, then stop. "
  + "I will return to Mallard to continue R2 setup.";

/** How the lane is used: a new bucket, one found in the account, or a new
 *  key for the bucket a storage already names. */
export type R2SetupMode = "create" | "existing" | "reconnect";

/** Where the hand-off stands. Each phase belongs to one step of the lane;
 *  `review` is past the last one. */
export type R2Phase =
  | "idle" | "check" | "login" | "account" | "bucket" | "find" | "pick" | "create" | "verify" | "authorize" | "review";

const PHASES: R2Phase[] = ["idle", "check", "login", "account", "bucket", "find", "pick", "create", "verify", "authorize", "review"];
const at = (phase: R2Phase) => PHASES.indexOf(phase);

export interface R2SetupState {
  phase: R2Phase;
  /** A helper run is in flight for the step at `phase`. */
  running: boolean;
  /** The step at `phase` failed with this code. */
  failed: R2SetupErrorCode | null;
  /** The step at `phase` was stopped by the user. */
  cancelled: boolean;
  cancelling: boolean;
  cancelError: string | null;
  /** The page the helper opened in the browser for the step at `phase`. */
  link: { stage: "login" | "bucket"; url: string } | null;
  wrangler: { version: string; path: string } | null;
  accounts: R2Account[];
  account: R2Account | null;
  jurisdiction: R2Jurisdiction;
  /** The name in the Bucket line's field: Mallard's proposal until it is edited. */
  name: string;
  /** The bucket the helper created, or found created, for this draft. */
  bucket: string | null;
  scan: StorageDiscoveryResult | null;
  /** The existing bucket taken from the scan. */
  chosen: StorageDiscoveryCandidate | null;
}

export function idleR2State(): R2SetupState {
  return {
    phase: "idle", running: false, failed: null, cancelled: false, cancelling: false, cancelError: null, link: null,
    wrangler: null, accounts: [], account: null, jurisdiction: "default", name: "", bucket: null, scan: null, chosen: null,
  };
}

/** What a name breaks of Cloudflare's rule (3–63 lowercase letters, numbers
 *  or hyphens, a letter or number at each end), or null when it keeps it.
 *  The mirror of `valid_bucket_name` in the R2 provider. */
export function bucketNameIssue(name: string): string | null {
  if (name.length < 3 || name.length > 63) return "3–63 characters";
  if (!/^[a-z0-9-]+$/.test(name)) return "lowercase letters, numbers or hyphens only";
  if (name.startsWith("-") || name.endsWith("-")) return "starts and ends with a letter or number";
  return null;
}

export const R2_JURISDICTIONS: { key: R2Jurisdiction; label: string }[] = [
  { key: "default", label: "Default" },
  { key: "eu", label: "EU" },
  { key: "us", label: "US" },
  { key: "fedramp", label: "FedRAMP" },
];

/** What a failed step says, in the line's voice. */
const FAILURE_FACTS: Record<R2SetupErrorCode, string> = {
  wrangler_not_found: "Not on PATH",
  wrangler_runtime_unavailable: "Could not start · check the Node.js and Wrangler installation",
  wrangler_update_required: "Update needed · this Wrangler has no auth token --json",
  sign_in_required: "Sign-in did not complete",
  authorization_timeout: "Expired after 2 minutes",
  authorization_unavailable: "Could not finish · close the previous authorization page",
  account_ambiguous: "Choose an account",
  account_unavailable: "No longer available · check the login",
  draft_changed: "The draft changed · open it again",
  setup_busy: "Another storage operation is running · wait, then try again",
  creation_unknown: "Cloudflare did not confirm the bucket · check the account, then try again or use an existing one",
  permission_denied: "Cloudflare refused access · check the account permissions",
  r2_not_enabled: "R2 is not enabled for this account. Complete R2 setup in the Cloudflare dashboard, then try again.",
  authentication_failed: "Cloudflare could not verify the sign-in. Select Change login, then sign in again.",
  rate_limited: "Cloudflare received too many requests. Wait, then try again.",
  service_unavailable: "Could not reach Cloudflare or its service is temporarily unavailable. Check your connection, then try again.",
  request_rejected: "Cloudflare rejected the request. Check the account, bucket name, and jurisdiction.",
  identity_mismatch: "The account or bucket differs from this draft · reset to start over",
  bucket_name_taken: "Already exists in this account · choose another name",
  cancelled: "Stopped",
  helper_unavailable: "The Mallard setup helper is missing · restart Mallard",
  setup_timeout: "Timed out and stopped",
  setup_failed: "Could not finish",
};

export function r2SetupErrorCode(reason: unknown): R2SetupErrorCode {
  if (typeof reason === "object" && reason !== null && "code" in reason
    && typeof reason.code === "string" && Object.prototype.hasOwnProperty.call(FAILURE_FACTS, reason.code)) {
    return reason.code as R2SetupErrorCode;
  }
  return "setup_failed";
}

const DEPENDENCY_CODES = new Set<R2SetupErrorCode>([
  "wrangler_not_found", "wrangler_runtime_unavailable", "wrangler_update_required", "helper_unavailable",
]);
/** Only a fresh draft gets past this. */
const FINAL_CODES = new Set<R2SetupErrorCode>(["identity_mismatch"]);

/** The step a failure belongs to: the helper checks Wrangler and the session
 *  on every run, so those failures go to their own lines whatever ran. */
function failedPhase(code: R2SetupErrorCode, phase: R2Phase, mode: R2SetupMode): R2Phase {
  if (DEPENDENCY_CODES.has(code)) return "check";
  if (code === "sign_in_required") return "login";
  if (code === "account_ambiguous" || code === "account_unavailable") return "account";
  if (code === "bucket_name_taken") return "bucket";
  if (mode === "existing" && (code === "draft_changed" || code === "identity_mismatch") && (phase === "verify" || phase === "authorize")) return "find";
  return phase === "idle" ? "check" : phase;
}

type R2Api = Pick<typeof projectSyncApi,
  "getR2SetupAccounts" | "proposeR2BucketName" | "setupR2Storage" | "discoverR2Storage" | "prepareExistingR2Storage" | "cancelR2StorageSetup">;

export interface R2SetupFlow {
  state: R2SetupState;
  /** Synchronous: a helper run is in flight. */
  isRunning: () => boolean;
  start: () => void;
  changeLogin: () => void;
  continueWith: (accountId: string) => void;
  setJurisdiction: (jurisdiction: R2Jurisdiction) => void;
  /** The field's text; editing answers a name that was taken. */
  setName: (name: string) => void;
  /** Another proposal in the field. */
  suggestAnother: () => void;
  createAndAuthorize: () => void;
  useBucket: (candidate: StorageDiscoveryCandidate) => void;
  cancel: () => Promise<void>;
  retry: () => void;
  scanAgain: () => void;
  /** Back to the start without touching the draft: a field was edited, or
   *  the page moved on. A run in flight keeps its place. */
  dismiss: () => void;
}

type Task = (saved: StorageSetupDraft, live: () => boolean) => Promise<Partial<R2SetupState> | null>;

export function useR2Setup({ enabled, draftId, mode, port, api = projectSyncApi }: {
  enabled: boolean;
  /** A different draft starts from nothing. */
  draftId: string | null;
  mode: R2SetupMode;
  port: StorageSetupPort;
  api?: R2Api;
}): R2SetupFlow {
  const [state, setState] = useState<R2SetupState>(idleR2State);
  const stateRef = useRef(state);
  stateRef.current = state;
  const portRef = useRef(port);
  portRef.current = port;
  const phaseRef = useRef<R2Phase>("idle");
  const runningRef = useRef(false);
  const cancelRef = useRef(false);
  const targetRef = useRef<StorageSetupDraft | null>(null);
  useEffect(() => {
    if (!runningRef.current) {
      phaseRef.current = "idle";
      setState(idleR2State());
    }
  }, [draftId]);
  const patch = (next: Partial<R2SetupState>) => {
    if (next.phase) phaseRef.current = next.phase;
    setState((current) => ({ ...current, ...next }));
  };

  /* Progress arrives on the run's channel: the page a step needs visited,
     the Wrangler it found, the bucket it made. No page is opened here: the
     lane hands the link over and the user picks the browser or profile it
     goes into. */
  const progressFor = (live: () => boolean) => (progress: R2SetupProgress) => {
    if (!live()) return;
    if (progress.status === "wrangler_ready") patch({ wrangler: { version: progress.version, path: progress.path } });
    else if (progress.status === "bucket_ready") patch({ bucket: progress.bucket });
    else patch({ link: { stage: progress.stage, url: progress.url }, phase: progress.stage === "login" ? "login" : "authorize" });
  };

  const run = async (begin: Partial<R2SetupState>, task: Task) => {
    const current = portRef.current;
    if (!enabled || runningRef.current || current.blocked()) return;
    runningRef.current = true;
    cancelRef.current = false;
    targetRef.current = null;
    const { isCurrent, persist } = current.beginRun();
    current.onRunStarted();
    patch({ ...begin, running: true, failed: null, cancelled: false, cancelling: false, cancelError: null, link: null });
    const live = () => isCurrent() && !cancelRef.current;
    try {
      const saved = await persist();
      if (!saved || !isCurrent()) return;
      if (cancelRef.current) {
        patch({ cancelled: true, cancelling: false });
        return;
      }
      targetRef.current = saved;
      const next = await task(saved, live);
      if (cancelRef.current) {
        if (isCurrent()) patch({ cancelled: true, cancelling: false });
        return;
      }
      if (next) patch(next);
    } catch (reason) {
      if (!isCurrent()) return;
      const code = r2SetupErrorCode(reason);
      if (code === "cancelled") patch({ cancelled: true, cancelling: false });
      else patch({ failed: code, cancelling: false, phase: failedPhase(code, phaseRef.current, mode) });
    } finally {
      runningRef.current = false;
      targetRef.current = null;
      patch({ running: false });
    }
  };

  const scanTask = (account: R2Account): Task => async (saved, live) => {
    patch({ phase: "find", account, scan: null, chosen: null, link: null });
    const scan = await api.discoverR2Storage(saved.draft_id, saved.revision, account.id, progressFor(live));
    return { phase: "pick", scan, link: null };
  };
  /* The account is confirmed before anything is looked at or made, however
     many the session holds: this is where a login is switched. */
  const accountsTask = (login: boolean): Task => async (saved, live) => {
    const accounts = await api.getR2SetupAccounts(saved.draft_id, saved.revision, login, progressFor(live));
    return { phase: "account", accounts, account: accounts.length === 1 ? accounts[0] : null, link: null };
  };
  const adoptTask = (request: (saved: StorageSetupDraft, live: () => boolean) => Promise<void>): Task => async (saved, live) => {
    await request(saved, live);
    if (!live()) return null;
    const adopted = await portRef.current.adoptPrepared(saved, live);
    return adopted ? { phase: "review", link: null } : null;
  };

  const start = () => void run(
    { phase: "check", wrangler: null, accounts: [], account: null, scan: null, chosen: null },
    accountsTask(false),
  );
  const changeLogin = () => void run(
    { phase: "login", accounts: [], account: null, scan: null, chosen: null },
    accountsTask(true),
  );
  const scan = (account: R2Account) => void run({ phase: "find", account, scan: null, chosen: null }, scanTask(account));
  /* The name is proposed here, offline; whether it is free is only known at
     the create. A proposal that cannot be fetched fails the Bucket step. */
  const propose = async () => {
    try {
      const name = await api.proposeR2BucketName();
      if (phaseRef.current === "bucket") patch({ name, failed: null });
    } catch (reason) {
      if (phaseRef.current === "bucket") patch({ failed: r2SetupErrorCode(reason) });
    }
  };
  const continueWith = (accountId: string) => {
    const account = stateRef.current.accounts.find((candidate) => candidate.id === accountId);
    if (!account) return;
    if (mode === "existing") return scan(account);
    patch({ phase: "bucket", account, name: "" });
    if (mode === "create") void propose();
  };
  const setName = (name: string) => patch(stateRef.current.failed === "bucket_name_taken" ? { name, failed: null } : { name });
  const createAndAuthorize = () => {
    const { account, jurisdiction, bucket, name } = stateRef.current;
    // A bucket already made keeps its name; until then the field's is asked for.
    const requested = bucket ?? name.trim();
    if (!account || (mode === "create" && bucketNameIssue(requested))) return;
    void run({ phase: "create" }, adoptTask((saved, live) => (
      api.setupR2Storage(saved.draft_id, saved.revision, account.id, jurisdiction, requested, progressFor(live))
    )));
  };
  const useBucket = (candidate: StorageDiscoveryCandidate) => {
    const { scan: result } = stateRef.current;
    if (!result || !candidateSelectable(candidate)) return;
    void run({ phase: "verify", chosen: candidate }, adoptTask((saved, live) => (
      api.prepareExistingR2Storage(saved.draft_id, saved.revision, result.scan_id, candidate.candidate_id, progressFor(live))
    )));
  };
  const cancel = async () => {
    if (!runningRef.current || cancelRef.current) return;
    cancelRef.current = true;
    patch({ cancelling: true, cancelError: null });
    const target = targetRef.current;
    // Still saving: the run stops before it launches the helper.
    if (!target) return;
    try {
      await api.cancelR2StorageSetup(target.draft_id, target.revision);
    } catch {
      if (runningRef.current && targetRef.current === target) {
        cancelRef.current = false;
        patch({ cancelling: false, cancelError: "Could not cancel. Select Cancel to try again." });
      }
    }
  };
  const retry = () => {
    const current = stateRef.current;
    switch (current.phase) {
      case "check":
      case "login":
      case "account":
        return start();
      case "find":
        return current.account ? scan(current.account) : start();
      case "bucket":
        return void propose();
      case "create":
      case "verify":
      case "authorize":
        if (mode === "existing") {
          if (current.chosen) useBucket(current.chosen);
          return;
        }
        return createAndAuthorize();
      default:
        return;
    }
  };
  const scanAgain = () => {
    const { account } = stateRef.current;
    if (account) scan(account);
  };
  const dismiss = () => {
    if (runningRef.current) return;
    phaseRef.current = "idle";
    setState(idleR2State());
  };
  return {
    state,
    isRunning: () => runningRef.current,
    start, changeLogin, continueWith, setJurisdiction: (jurisdiction) => patch({ jurisdiction }),
    setName, suggestAnother: () => void propose(),
    createAndAuthorize, useBucket, cancel, retry, scanAgain, dismiss,
  };
}

/* ——— the lane ——————————————————————————————————————— */

const RESET_TITLE = "Discards the entire local setup draft and all setup progress. Any bucket already created in Cloudflare and any connected storage are kept.";
const LOGIN_BUDGET_SECONDS = 180;
const CONSENT_BUDGET_SECONDS = 120;

const shortId = (id: string) => `${id.slice(0, 8)}…${id.slice(-8)}`;
const jurisdictionLabel = (key: R2Jurisdiction) => R2_JURISDICTIONS.find((option) => option.key === key)?.label ?? key;

const chip = (label: string, value: string) => (
  <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>
);

export function R2SetupLane({ mode, flow, disabled, descriptor, storages, bucketName, onOpenStorage, onCreateInstead, onReset }: {
  mode: R2SetupMode;
  flow: R2SetupFlow;
  /** The page is busy — the lane's run included. A run keeps its own Cancel
   *  and browser verbs live. */
  disabled: boolean;
  descriptor: Pick<StorageProviderDescriptor, "key" | "icon">;
  /** Every configured storage, to name the one a found bucket is already
   *  connected as. */
  storages: readonly StorageSummary[];
  /** The bucket the storage already names, for the reconnect lane. */
  bucketName?: string;
  onOpenStorage: (storageId: string) => void;
  onCreateInstead: () => void;
  onReset: () => void;
}) {
  const { state } = flow;
  const [accountPick, setAccountPick] = useState("");
  const [copyNotice, setCopyNotice] = useState<{ text: string; url?: string } | null>(null);
  useEffect(() => { setCopyNotice(null); }, [state.link?.url, state.phase, state.failed]);
  const waiting = !state.failed && !state.cancelled;
  const loginBudget = useCountdown(state.phase === "login" && waiting ? state.link?.url ?? null : null, LOGIN_BUDGET_SECONDS);
  const consentBudget = useCountdown(state.phase === "authorize" && waiting ? state.link?.url ?? null : null, CONSENT_BUDGET_SECONDS);

  const copy = async (text: string, label: string, url?: string) => {
    try {
      await writeClipboardText(text);
      setCopyNotice({ text: `${label} copied.` });
    } catch {
      setCopyNotice({ text: "Could not copy. Select and copy the text.", url });
    }
  };
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
  const copyStatus = copyNotice && (
    <>
      <span role="status">{copyNotice.text}</span>
      {copyNotice.url && <input aria-label="Authorization link" readOnly value={copyNotice.url} />}
    </>
  );
  /* A browser step is the user's turn, in whichever browser or profile they
     choose: the link is theirs to copy; opening it here is the shortcut. */
  const browserFact = (stage: "login" | "bucket", budget: string) => (
    <>{stage === "login" ? "Sign in to Cloudflare in your browser" : "Approve in your browser"} · <span className="r2-countdown">{budget}</span></>
  );
  const browserControls = (url: string) => (
    <>
      {button(<><Icon name="copy" size={12} /> Copy link</>, () => void copy(url, "Link", url), "primary", false)}
      {button(<><Icon name="external-link" size={12} /> Open browser</>, () => void openExternalUrl(url), "plain", false)}
      {cancelButton}
      {copyStatus}
    </>
  );
  const retryButton = state.failed && FINAL_CODES.has(state.failed) ? null : button("Try again", flow.retry, "primary");

  const interrupted: StepStatus | null = state.failed ? "failed" : state.cancelled ? "stopped" : null;
  const statusOf = (...phases: R2Phase[]): StepStatus => {
    if (phases.includes(state.phase)) return interrupted ?? (state.phase === "review" ? "done" : "doing");
    return at(state.phase) > at(phases[phases.length - 1]) ? "done" : "todo";
  };
  const failure = (fallback?: string) => state.failed ? FAILURE_FACTS[state.failed] : fallback ?? "Stopped";
  const kept = mode !== "existing" && state.bucket ? ` · ${state.bucket} kept` : "";
  const settled = !state.running && state.phase !== "review";
  /* The account line carries Change login while the account is being
     confirmed; the sign-in line carries it once the lane has moved on. */
  const confirming = state.phase === "account" && !interrupted;
  const changeLoginButton = button(<><Icon name="user" size={12} /> Change login</>, flow.changeLogin);

  const wrangler: StepView = (() => {
    const status = statusOf("check");
    const dependency = state.failed === "wrangler_not_found" || state.failed === "wrangler_update_required";
    return {
      key: "wrangler", title: "Wrangler", status,
      fact: status === "failed" ? (dependency ? <>{failure()} · <code>{WRANGLER_INSTALL_COMMAND}</code></> : failure())
        : status === "stopped" ? "Stopped"
        : status === "doing" ? "Checking…"
        : state.wrangler ? <>{state.wrangler.version} · <code>{state.wrangler.path}</code></>
        : status === "done" ? "Checked"
        : "Checks the install",
      controls: status === "failed" ? (
        <>
          {dependency && button("Copy command", () => void copy(WRANGLER_INSTALL_COMMAND, "Command"))}
          {dependency && button("Copy agent message", () => void copy(WRANGLER_INSTALL_AGENT_MESSAGE, "Agent message"))}
          {retryButton}
          {copyStatus}
        </>
      ) : status === "stopped" ? retryButton
        : status === "doing" ? cancelButton
        : undefined,
    };
  })();

  const signIn: StepView = (() => {
    const status = statusOf("login");
    return {
      key: "sign-in", title: "Sign in", status,
      fact: status === "failed" ? failure()
        : status === "stopped" ? "Stopped"
        : status === "doing" ? (state.link ? browserFact("login", loginBudget) : "Starting sign-in…")
        : status === "done" ? "Wrangler session"
        : "Wrangler session, or your browser",
      controls: status === "failed" || status === "stopped" ? retryButton
        : status === "doing" ? (state.link ? browserControls(state.link.url) : cancelButton)
        : status === "done" && settled && !confirming ? changeLoginButton
        : undefined,
    };
  })();

  const account: StepView = (() => {
    const status = statusOf("account");
    const pickable = state.accounts.some((candidate) => candidate.id === accountPick);
    const chosenId = state.account?.id ?? (pickable ? accountPick : "");
    return {
      key: "account", title: "Account", status,
      fact: status === "failed" ? failure()
        : state.account ? <>{state.account.name || state.account.id} · <code>{shortId(state.account.id)}</code></>
        : status === "doing" ? `${state.accounts.length} accounts in this session`
        : "Confirmed before anything changes",
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}</>
        : status === "doing" ? (
          <>
            {!state.account && (
              <span className="sync-select-shell r2-select-shell">
                <select value={pickable ? accountPick : ""} aria-label="Cloudflare account" disabled={disabled} onChange={(event) => setAccountPick(event.target.value)}>
                  <option value="">Choose an account</option>
                  {state.accounts.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>{candidate.name || candidate.id} · {shortId(candidate.id)}</option>
                  ))}
                </select>
              </span>
            )}
            {button("Continue", () => flow.continueWith(chosenId), "primary", disabled || !chosenId)}
            {changeLoginButton}
          </>
        ) : undefined,
    };
  })();

  const bucket: StepView = mode === "existing" ? (() => {
    const status = statusOf("find", "pick");
    const result = state.scan;
    const found = result ? result.candidates.filter((candidate) => candidate.marker === "storage").length : 0;
    const expired = state.failed === "draft_changed" || state.failed === "identity_mismatch";
    const scanAgainButton = button(<><Icon name="refresh" size={12} /> Scan again</>, flow.scanAgain);
    return {
      key: "buckets", title: "Buckets", status,
      fact: status === "failed" ? (state.failed === "draft_changed" ? "Scan expired"
          : state.failed === "identity_mismatch" ? "The bucket changed or is no longer available"
          : failure())
        : status === "stopped" ? "Stopped"
        : state.phase === "find" ? "Scanning all jurisdictions · read-only"
        : result && status === "doing" ? <>{result.candidates.length} checked · read-only</>
        : status === "done" && state.chosen ? <><code>{state.chosen.name}</code> · {state.chosen.facts.find((fact) => fact.label === "Jurisdiction")?.value ?? "Default"}</>
        : "Your buckets · read-only",
      controls: status === "failed" ? (expired ? scanAgainButton : retryButton)
        : status === "stopped" ? retryButton
        : state.phase === "find" ? cancelButton
        : state.phase === "pick" ? scanAgainButton
        : undefined,
      body: result && state.phase === "pick" && !interrupted ? (
        <div className="r2-step-list">
          {result.status === "partial" && <ScanIncompleteNotice issues={result.issues} />}
          <CandidateGroups
            result={result}
            unitPlural="buckets"
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
                verb={candidateSelectable(candidate) ? (
                  <button
                    type="button"
                    className="btn btn-primary storage-setup-guide-button"
                    disabled={disabled}
                    onClick={() => flow.useBucket(candidate)}
                    aria-label={`Use ${candidate.name} and authorize`}
                  >
                    Use &amp; authorize
                  </button>
                ) : undefined}
                onOpenStorage={onOpenStorage}
              />
            )}
          />
          {found === 0 && (
            <DiscoveryEmpty
              place="your Cloudflare account"
              unit="bucket"
              sentence={`${result.candidates.length} bucket${result.candidates.length === 1 ? "" : "s"} checked across ${result.scope}. Nothing was changed.`}
              disabled={disabled}
              onCreateInstead={onCreateInstead}
            />
          )}
        </div>
      ) : undefined,
    };
  })() : (() => {
    const status = statusOf("bucket", "create");
    const named = state.bucket ?? (mode === "reconnect" ? bucketName ?? null : null);
    const reconnect = mode === "reconnect";
    const taken = state.failed === "bucket_name_taken";
    /* Until a bucket exists the name is the user's: proposed in the field,
       editable, and back in the field with the reason when it was taken.
       The note after the field spells the rule a wrong one breaks, or
       names the conflict; the segments stay live. */
    const naming = !reconnect && state.phase === "bucket" && (!interrupted || taken);
    const issue = bucketNameIssue(state.name);
    const nameField = (
      <span className="r2-name-field">
        <input
          className="r2-name-input"
          value={state.name}
          aria-label="Bucket name"
          aria-invalid={issue !== null}
          spellCheck={false}
          disabled={disabled}
          onChange={(event) => flow.setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && issue === null) flow.createAndAuthorize(); }}
        />
        {button(<><Icon name="refresh" size={12} /> Suggest</>, flow.suggestAnother)}
      </span>
    );
    const nameNote = taken ? "· already exists in this account · choose another name"
      : issue ? `· ${issue}`
      : null;
    return {
      key: "bucket", title: "Bucket", status,
      fact: naming ? <>{nameField}{nameNote && <span>{nameNote}</span>}</>
        : status === "failed" ? `${failure()}${kept}`
        : status === "stopped" ? `Stopped${kept}`
        : state.phase === "create" ? (named ? <><code>{named}</code> · {reconnect ? "kept" : `${jurisdictionLabel(state.jurisdiction)} · created`}</> : reconnect ? "Checking the bucket…" : <>Creating <code>{state.name}</code>…</>)
        : status === "done" ? (named ? <><code>{named}</code> · {reconnect ? "kept" : `${jurisdictionLabel(state.jurisdiction)} · created`}</> : "Created")
        : reconnect ? <><code>{bucketName}</code> · this storage’s bucket</>
        : "New bucket",
      controls: naming ? (
        <>
          <div className="sync-segmented r2-segmented" role="radiogroup" aria-label="Jurisdiction">
            {R2_JURISDICTIONS.map((option) => (
              <button
                key={option.key}
                type="button"
                role="radio"
                aria-checked={state.jurisdiction === option.key}
                className={state.jurisdiction === option.key ? "active" : undefined}
                disabled={disabled}
                onClick={() => flow.setJurisdiction(option.key)}
              >
                {option.label}
              </button>
            ))}
          </div>
          {button("Create & authorize", flow.createAndAuthorize, "primary", disabled || issue !== null)}
        </>
      ) : status === "failed" || status === "stopped" ? retryButton
        : state.phase === "create" ? cancelButton
        : status === "doing" ? button("Authorize", flow.createAndAuthorize, "primary")
        : undefined,
    };
  })();

  const key: StepView = (() => {
    const status = statusOf("verify", "authorize");
    return {
      key: "key", title: "Authorize key", status,
      fact: status === "failed" ? `${failure()}${kept}`
        : status === "stopped" ? `Stopped${kept}`
        : state.phase === "verify" ? <>Checking <code>{state.chosen?.name}</code> is unchanged…</>
        : state.phase === "authorize" ? (state.link ? browserFact("bucket", consentBudget) : "Requesting consent…")
        : status === "done" ? "Issued · shown once, in the fields below"
        : "Object Read & Write · this bucket only",
      controls: status === "failed" || status === "stopped" ? retryButton
        : state.phase === "verify" ? cancelButton
        : state.phase === "authorize" ? (state.link ? browserControls(state.link.url) : cancelButton)
        : undefined,
    };
  })();

  const review: StepView = {
    key: "review", title: "Review", status: state.phase === "review" ? "done" : "todo",
    fact: mode === "reconnect" ? "Test, then Save credential" : "Test, then Finish setup",
  };

  const steps = [wrangler, signIn, account, bucket, key, review];
  const resetButton = (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" disabled={disabled} title={RESET_TITLE} onClick={onReset}>
      <Icon name="refresh" size={12} /> Reset setup
    </button>
  );

  if (state.phase === "idle") {
    return (
      <>
        <span className="storage-setup-guide-copy">
          {mode === "create" ? "Creates a bucket and a key for it only."
            : mode === "existing" ? "Lists your buckets, read-only, then issues a key."
            : "Issues a new key for this bucket."}
        </span>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={flow.start}>
          <Icon name="terminal" size={13} />{" "}
          {mode === "create" ? "Set up with Wrangler" : mode === "existing" ? "Find in my account" : "Reconnect with Wrangler"}
        </button>
      </>
    );
  }
  if (state.phase === "review") {
    const bucketFact = state.chosen?.name ?? state.bucket ?? bucketName ?? "";
    const jurisdictionFact = state.chosen?.facts.find((fact) => fact.label === "Jurisdiction")?.value ?? jurisdictionLabel(state.jurisdiction);
    return (
      <>
        <span className="r2-ready" role="status">
          <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
          <span className="r2-facts">
            {chip("Bucket", bucketFact)}
            {chip("Jurisdiction", jurisdictionFact)}
            {chip("Account", state.account?.name || state.account?.id || "")}
            {chip("Key", "this bucket only")}
          </span>
        </span>
        {resetButton}
      </>
    );
  }
  return (
    <>
      {resetButton}
      <ol className="r2-steps">{steps.map((step) => <Step key={step.key} step={step} />)}</ol>
      {state.cancelError && <span className="storage-setup-guide-error" role="alert">{state.cancelError}</span>}
    </>
  );
}
