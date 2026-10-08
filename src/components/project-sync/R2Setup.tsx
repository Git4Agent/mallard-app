import { Step, useCountdown, type StepView, type StepStatus, type StorageSetupPort } from "./StorageSetupStep";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ParseKeys } from "i18next";
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
import { t as translate, Trans, useTranslation } from "../../i18n";
import { writeClipboardText } from "../sync-review/clipboard";
import { projectSyncApi } from "./api";
import { openExternalUrl } from "./externalLink";
import {
  CandidateGroups,
  CandidateRow,
  candidateSelectable,
  DiscoveryEmpty,
  discoveryFact,
  discoveryScope,
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

type StorageKey = ParseKeys<"storage">;

// i18n-ignore: a shell command, not copy
export const WRANGLER_INSTALL_COMMAND = "npm install --global wrangler@latest";
/* The instruction the user pastes into a coding agent, not interface copy:
   it stays one English prompt whatever the interface language. */
export const WRANGLER_INSTALL_AGENT_MESSAGE =
  // i18n-ignore: a prompt for the agent
  "Install Cloudflare Wrangler globally with `npm install --global wrangler@latest`. "
  // i18n-ignore: a prompt for the agent
  + "Check `npm --version` first. If npm is missing, tell me that Node.js and npm are required. "
  // i18n-ignore: a prompt for the agent
  + "After installation, verify `wrangler --version`, then stop. "
  // i18n-ignore: a prompt for the agent
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
  /** The last Cancel could not reach the helper; Cancel may be tried again. */
  cancelFailed: boolean;
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
    phase: "idle", running: false, failed: null, cancelled: false, cancelling: false, cancelFailed: false, link: null,
    wrangler: null, accounts: [], account: null, jurisdiction: "default", name: "", bucket: null, scan: null, chosen: null,
  };
}

/** Which part of Cloudflare's rule (3–63 lowercase letters, numbers or
 *  hyphens, a letter or number at each end) a name breaks, or null when it
 *  keeps it. The mirror of `valid_bucket_name` in the R2 provider; the lane
 *  words it with `bucketNameIssue`. */
function bucketNameRule(name: string): "length" | "charset" | "ends" | null {
  if (name.length < 3 || name.length > 63) return "length";
  if (!/^[a-z0-9-]+$/.test(name)) return "charset";
  if (name.startsWith("-") || name.endsWith("-")) return "ends";
  return null;
}

export const R2_JURISDICTIONS = [
  { key: "default", labelKey: "r2.jurisdiction.default" },
  { key: "eu", labelKey: "r2.jurisdiction.eu" },
  { key: "us", labelKey: "r2.jurisdiction.us" },
  { key: "fedramp", labelKey: "r2.jurisdiction.fedramp" },
] as const satisfies readonly { key: R2Jurisdiction; labelKey: StorageKey }[];

/** What a failed step says, in the line's voice: a catalog key per code. */
const FAILURE_FACTS: Record<R2SetupErrorCode, StorageKey> = {
  wrangler_not_found: "r2Failure.wranglerNotFound",
  wrangler_runtime_unavailable: "r2Failure.wranglerRuntimeUnavailable",
  wrangler_update_required: "r2Failure.wranglerUpdateRequired",
  sign_in_required: "r2Failure.signInRequired",
  authorization_timeout: "r2Failure.authorizationTimeout",
  authorization_unavailable: "r2Failure.authorizationUnavailable",
  account_ambiguous: "r2Failure.accountAmbiguous",
  account_unavailable: "r2Failure.accountUnavailable",
  draft_changed: "r2Failure.draftChanged",
  setup_busy: "laneError.busy",
  creation_unknown: "r2Failure.creationUnknown",
  permission_denied: "r2Failure.permissionDenied",
  r2_not_enabled: "r2Failure.r2NotEnabled",
  authentication_failed: "r2Failure.authenticationFailed",
  rate_limited: "r2Failure.rateLimited",
  service_unavailable: "r2Failure.serviceUnavailable",
  request_rejected: "r2Failure.requestRejected",
  identity_mismatch: "r2Failure.identityMismatch",
  bucket_name_taken: "r2Failure.bucketNameTaken",
  cancelled: "lane.stopped",
  helper_unavailable: "r2Failure.helperUnavailable",
  setup_timeout: "r2Failure.setupTimeout",
  setup_failed: "r2Failure.setupFailed",
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
    patch({ ...begin, running: true, failed: null, cancelled: false, cancelling: false, cancelFailed: false, link: null });
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
      else patch({
        failed: code, cancelling: false, phase: failedPhase(code, phaseRef.current, mode),
        // A duplicate reopens naming; the next request must follow that field,
        // even if an earlier attempt created a bucket before authorization failed.
        ...(code === "bucket_name_taken" && mode === "create" ? { bucket: null } : {}),
      });
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
    if (!account || (mode === "create" && bucketNameRule(requested))) return;
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
    patch({ cancelling: true, cancelFailed: false });
    const target = targetRef.current;
    // Still saving: the run stops before it launches the helper.
    if (!target) return;
    try {
      await api.cancelR2StorageSetup(target.draft_id, target.revision);
    } catch {
      if (runningRef.current && targetRef.current === target) {
        cancelRef.current = false;
        patch({ cancelling: false, cancelFailed: true });
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

/** What a name breaks of Cloudflare's rule, in the Bucket line's words, or
 *  null when it keeps it. */
export function bucketNameIssue(name: string): string | null {
  const rule = bucketNameRule(name);
  return rule ? translate(`storage:r2.bucketIssue.${rule}`) : null;
}

const LOGIN_BUDGET_SECONDS = 180;
const CONSENT_BUDGET_SECONDS = 120;

const shortId = (id: string) => `${id.slice(0, 8)}…${id.slice(-8)}`;
const jurisdictionLabel = (key: R2Jurisdiction) => {
  const option = R2_JURISDICTIONS.find((candidate) => candidate.key === key);
  return option ? translate(`storage:${option.labelKey}`) : key;
};
/** The jurisdiction the scan reported for a found bucket. */
const chosenJurisdiction = (candidate: StorageDiscoveryCandidate | null) => {
  const fact = candidate?.facts.find((candidate) => candidate.key === "jurisdiction");
  return fact && discoveryFact(fact).value;
};

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
  const { t } = useTranslation(["storage", "common"]);
  const { state } = flow;
  const [accountPick, setAccountPick] = useState("");
  const [copyNotice, setCopyNotice] = useState<{ key: StorageKey; url?: string } | null>(null);
  useEffect(() => { setCopyNotice(null); }, [state.link?.url, state.phase, state.failed]);
  const waiting = !state.failed && !state.cancelled;
  const loginBudget = useCountdown(state.phase === "login" && waiting ? state.link?.url ?? null : null, LOGIN_BUDGET_SECONDS);
  const consentBudget = useCountdown(state.phase === "authorize" && waiting ? state.link?.url ?? null : null, CONSENT_BUDGET_SECONDS);

  const copy = async (text: string, copied: StorageKey, url?: string) => {
    try {
      await writeClipboardText(text);
      setCopyNotice({ key: copied });
    } catch {
      setCopyNotice({ key: "r2.copyTextFailed", url });
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
  const cancelButton = button(state.cancelling ? t("lane.cancelling") : t("common:cancel"), () => void flow.cancel(), "ghost", state.cancelling);
  const copyStatus = copyNotice && (
    <>
      <span role="status">{t(copyNotice.key)}</span>
      {copyNotice.url && <input aria-label={t("r2.authorizationLink")} readOnly value={copyNotice.url} />}
    </>
  );
  /* A browser step is the user's turn, in whichever browser or profile they
     choose: the link is theirs to copy; opening it here is the shortcut. */
  const browserFact = (stage: "login" | "bucket", budget: string) => (
    <>{stage === "login" ? t("r2.signInCloudflare") : t("lane.approveInBrowser")} · <span className="r2-countdown">{budget}</span></>
  );
  const browserControls = (url: string) => (
    <>
      {button(<><Icon name="copy" size={12} /> {t("lane.copyLink")}</>, () => void copy(url, "lane.linkCopied", url), "primary", false)}
      {button(<><Icon name="external-link" size={12} /> {t("lane.openBrowser")}</>, () => void openExternalUrl(url), "plain", false)}
      {cancelButton}
      {copyStatus}
    </>
  );
  const retryButton = state.failed && FINAL_CODES.has(state.failed) ? null : button(t("common:tryAgain"), flow.retry, "primary");

  const interrupted: StepStatus | null = state.failed ? "failed" : state.cancelled ? "stopped" : null;
  const statusOf = (...phases: R2Phase[]): StepStatus => {
    if (phases.includes(state.phase)) return interrupted ?? (state.phase === "review" ? "done" : "doing");
    return at(state.phase) > at(phases[phases.length - 1]) ? "done" : "todo";
  };
  const failure = () => state.failed ? t(FAILURE_FACTS[state.failed]) : t("lane.stopped");
  const kept = mode !== "existing" && state.bucket ? ` · ${t("r2.bucketKept", { bucket: state.bucket })}` : "";
  const settled = !state.running && state.phase !== "review";
  /* The account line carries Change login while the account is being
     confirmed; the sign-in line carries it once the lane has moved on. */
  const confirming = state.phase === "account" && !interrupted;
  const changeLoginButton = button(<><Icon name="user" size={12} /> {t("lane.changeLogin")}</>, flow.changeLogin);

  const wrangler: StepView = (() => {
    const status = statusOf("check");
    const dependency = state.failed === "wrangler_not_found" || state.failed === "wrangler_update_required";
    return {
      key: "wrangler", title: t("r2.wrangler"), status,
      fact: status === "failed" ? (dependency ? <>{failure()} · <code>{WRANGLER_INSTALL_COMMAND}</code></> : failure())
        : status === "stopped" ? t("lane.stopped")
        : status === "doing" ? t("lane.checking")
        : state.wrangler ? <>{state.wrangler.version} · <code>{state.wrangler.path}</code></>
        : status === "done" ? t("r2.checked")
        : t("r2.checksInstall"),
      controls: status === "failed" ? (
        <>
          {dependency && button(t("r2.copyCommand"), () => void copy(WRANGLER_INSTALL_COMMAND, "r2.commandCopied"))}
          {dependency && button(t("r2.copyAgentMessage"), () => void copy(WRANGLER_INSTALL_AGENT_MESSAGE, "r2.agentMessageCopied"))}
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
      key: "sign-in", title: t("lane.signIn"), status,
      fact: status === "failed" ? failure()
        : status === "stopped" ? t("lane.stopped")
        : status === "doing" ? (state.link ? browserFact("login", loginBudget) : t("lane.startingSignIn"))
        : status === "done" ? t("r2.wranglerSession")
        : t("r2.wranglerOrBrowser"),
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
      key: "account", title: t("lane.account"), status,
      fact: status === "failed" ? failure()
        : state.account ? <>{state.account.name || state.account.id} · <code>{shortId(state.account.id)}</code></>
        : status === "doing" ? t("r2.accountsInSession", { count: state.accounts.length })
        : t("lane.confirmedFirst"),
      controls: status === "failed" ? <>{retryButton}{changeLoginButton}</>
        : status === "doing" ? (
          <>
            {!state.account && (
              <span className="sync-select-shell r2-select-shell">
                <select value={pickable ? accountPick : ""} aria-label={t("r2.cloudflareAccount")} disabled={disabled} onChange={(event) => setAccountPick(event.target.value)}>
                  <option value="">{t("r2.chooseAccount")}</option>
                  {state.accounts.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>{candidate.name || candidate.id} · {shortId(candidate.id)}</option>
                  ))}
                </select>
              </span>
            )}
            {button(t("lane.continue"), () => flow.continueWith(chosenId), "primary", disabled || !chosenId)}
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
    const scanAgainButton = button(<><Icon name="refresh" size={12} /> {t("shared.scanAgain")}</>, flow.scanAgain);
    return {
      key: "buckets", title: t("r2.buckets"), status,
      fact: status === "failed" ? (state.failed === "draft_changed" ? t("r2.scanExpired")
          : state.failed === "identity_mismatch" ? t("r2.bucketGone")
          : failure())
        : status === "stopped" ? t("lane.stopped")
        : state.phase === "find" ? t("r2.scanning")
        : result && status === "doing" ? t("lane.checkedReadOnly", { count: result.candidates.length })
        : status === "done" && state.chosen ? <><code>{state.chosen.name}</code> · {chosenJurisdiction(state.chosen) ?? t("r2.jurisdiction.default")}</>
        : t("r2.yourBuckets"),
      controls: status === "failed" ? (expired ? scanAgainButton : retryButton)
        : status === "stopped" ? retryButton
        : state.phase === "find" ? cancelButton
        : state.phase === "pick" ? scanAgainButton
        : undefined,
      body: result && state.phase === "pick" && !interrupted ? (
        <div className="r2-step-list">
          {result.status === "partial" && <ScanIncompleteNotice issues={result.issues} codes={result.issue_codes} />}
          <CandidateGroups
            result={result}
            unitPlural={t("r2.units")}
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
                    aria-label={t("r2.useAuthorizeLabel", { name: candidate.name })}
                  >
                    {t("r2.useAuthorize")}
                  </button>
                ) : undefined}
                onOpenStorage={onOpenStorage}
              />
            )}
          />
          {found === 0 && (
            <DiscoveryEmpty
              place={t("r2.place")}
              unit={t("r2.unit")}
              sentence={t("r2.noneFound", { count: result.candidates.length, scope: discoveryScope(result) })}
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
          aria-label={t("r2.bucketName")}
          aria-invalid={issue !== null}
          spellCheck={false}
          disabled={disabled}
          onChange={(event) => flow.setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && issue === null) flow.createAndAuthorize(); }}
        />
        {button(<><Icon name="refresh" size={12} /> {t("lane.suggest")}</>, flow.suggestAnother)}
      </span>
    );
    const nameNote = taken ? `· ${t("r2.nameTaken")}`
      : issue ? `· ${issue}`
      : null;
    const namedFact = named && <><code>{named}</code> · {reconnect ? t("r2.kept") : `${jurisdictionLabel(state.jurisdiction)} · ${t("lane.created")}`}</>;
    return {
      key: "bucket", title: t("r2.bucket"), status,
      fact: naming ? <span className="r2-name-with-note">{nameField}{nameNote && <span>{nameNote}</span>}</span>
        : status === "failed" ? `${failure()}${kept}`
        : status === "stopped" ? `${t("lane.stopped")}${kept}`
        : state.phase === "create" ? (namedFact || (reconnect ? t("r2.checkingBucket") : <Trans t={t} i18nKey="storage:lane.creating" components={{ name: <code>{state.name}</code> }} />))
        : status === "done" ? (namedFact || t("r2.created"))
        : reconnect ? <><code>{bucketName}</code> · {t("r2.thisBucket")}</>
        : t("r2.newBucket"),
      controls: naming ? (
        <>
          <div className="sync-segmented r2-segmented" role="radiogroup" aria-label={t("r2.jurisdictionLabel")}>
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
                {t(option.labelKey)}
              </button>
            ))}
          </div>
          {button(t("r2.createAuthorize"), flow.createAndAuthorize, "primary", disabled || issue !== null)}
        </>
      ) : status === "failed" || status === "stopped" ? retryButton
        : state.phase === "create" ? cancelButton
        : status === "doing" ? button(t("r2.authorize"), flow.createAndAuthorize, "primary")
        : undefined,
    };
  })();

  const key: StepView = (() => {
    const status = statusOf("verify", "authorize");
    return {
      key: "key", title: t("r2.authorizeKey"), status,
      fact: status === "failed" ? `${failure()}${kept}`
        : status === "stopped" ? `${t("lane.stopped")}${kept}`
        : state.phase === "verify" ? <Trans t={t} i18nKey="storage:r2.checkingUnchanged" components={{ name: <code>{state.chosen?.name}</code> }} />
        : state.phase === "authorize" ? (state.link ? browserFact("bucket", consentBudget) : t("r2.requestingConsent"))
        : status === "done" ? t("r2.issued")
        : t("r2.keyScope"),
      controls: status === "failed" || status === "stopped" ? retryButton
        : state.phase === "verify" ? cancelButton
        : state.phase === "authorize" ? (state.link ? browserControls(state.link.url) : cancelButton)
        : undefined,
    };
  })();

  const review: StepView = {
    key: "review", title: t("lane.review"), status: state.phase === "review" ? "done" : "todo",
    fact: mode === "reconnect" ? t("lane.reviewReconnect") : t("lane.reviewCreate"),
  };

  const steps = [wrangler, signIn, account, bucket, key, review];
  const resetButton = (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" disabled={disabled} title={t("r2.resetTitle")} onClick={onReset}>
      <Icon name="refresh" size={12} /> {t("lane.reset")}
    </button>
  );

  if (state.phase === "idle") {
    return (
      <>
        <span className="storage-setup-guide-copy">
          {mode === "create" ? null
            : mode === "existing" ? t("r2.existingIntro")
            : t("r2.reconnectIntro")}
        </span>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" disabled={disabled} onClick={flow.start}>
          <Icon name="terminal" size={13} />{" "}
          {mode === "create" ? t("r2.setUp") : mode === "existing" ? t("r2.find") : t("r2.reconnect")}
        </button>
      </>
    );
  }
  if (state.phase === "review") {
    const bucketFact = state.chosen?.name ?? state.bucket ?? bucketName ?? "";
    const jurisdictionFact = chosenJurisdiction(state.chosen) ?? jurisdictionLabel(state.jurisdiction);
    return (
      <>
        <span className="r2-ready" role="status">
          <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> {t("lane.ready")}</span>
          <span className="r2-facts">
            {chip(t("r2.bucket"), bucketFact)}
            {chip(t("r2.jurisdictionLabel"), jurisdictionFact)}
            {chip(t("lane.account"), state.account?.name || state.account?.id || "")}
            {chip(t("r2.key"), t("r2.thisBucketOnly"))}
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
      {state.cancelFailed && <span className="storage-setup-guide-error" role="alert">{t("r2.cancelFailed")}</span>}
    </>
  );
}
