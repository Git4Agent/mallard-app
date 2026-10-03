import { StrictMode, useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import type { StorageSetupMode } from "../src/components/project-sync/StorageSettings";
import type { R2Account } from "../src/types";
import "../src/App.css";

/**
 * Round three of the R2 setup mockup: direction A1 (every step in view),
 * refined to the app's own grammar. One line per step; Change login and
 * Scan again are buttons; every browser step carries Open again + Copy link
 * + Cancel; the Use-existing list carries its verb on the row ("Use &
 * authorize") while the page footer keeps Finish setup, greyed until the key
 * is in; jurisdiction is a segmented control; Reset setup stands at the
 * lane's tail with the shipped semantics. Round four puts the bucket's name
 * in the Bucket line: proposed, editable, with Suggest another; a taken name
 * comes back to the line. `npm run dev`, then /mockups/r2-steps-mockup.html
 * (`?theme=dark`; `?play=…` drives it, see `usePlay`).
 */

/* ——— scenario knobs and data ——————————————————————————————— */

interface Knobs {
  accounts: "one" | "two";
  wrangler: "ok" | "missing";
  login: "in" | "out";
  consent: "ok" | "timeout";
  scan: "complete" | "partial" | "empty";
}

const T = { check: 900, browser: 2200, create: 1000, consent: 2400, timeout: 5200, scan: 1400, verify: 600 };

const ACCOUNTS: R2Account[] = [
  { id: "023e105f4ecef8ad9ca31a8372d0c353", name: "He Qu" },
  { id: "7f3c0a9e5b1d4c2f8e6a7b9c0d1e2f3a", name: "Acme Labs" },
];
const accountsFor = (knobs: Knobs) => knobs.accounts === "one" ? [ACCOUNTS[0]] : ACCOUNTS;
const shortId = (id: string) => `${id.slice(0, 8)}…${id.slice(-8)}`;

type Jurisdiction = "Default" | "EU" | "US" | "FedRAMP";
const JURISDICTIONS: Jurisdiction[] = ["Default", "EU", "US", "FedRAMP"];
type Marker = "storage" | "missing" | "unsupported" | "unreadable";
interface Bucket { name: string; jurisdiction: Jurisdiction; created: string; marker: Marker; detail?: string; connectedAs?: string }

const BUCKETS: Bucket[] = [
  { name: "mallard-otter", jurisdiction: "Default", created: "Aug 30", marker: "storage", detail: "layout 1", connectedAs: "R2 storage" },
  { name: "mallard-brave", jurisdiction: "Default", created: "Sep 2", marker: "storage", detail: "layout 1" },
  { name: "codex-sync-backups", jurisdiction: "EU", created: "Jul 14", marker: "storage", detail: "layout 1" },
  { name: "mallard-gently", jurisdiction: "Default", created: "Sep 8", marker: "unsupported", detail: "layout 2" },
  { name: "team-archive", jurisdiction: "Default", created: "Feb 1", marker: "unreadable", detail: "access denied" },
  { name: "assets-prod", jurisdiction: "Default", created: "Jan 12", marker: "missing" },
  { name: "www-static", jurisdiction: "Default", created: "Jan 12", marker: "missing" },
  { name: "photos-2024", jurisdiction: "EU", created: "Mar 3", marker: "missing" },
  { name: "backups-old", jurisdiction: "Default", created: "Nov 9", marker: "missing" },
  { name: "logs-archive", jurisdiction: "US", created: "Apr 21", marker: "missing" },
  { name: "media-cdn", jurisdiction: "EU", created: "Feb 17", marker: "missing" },
  { name: "artifacts", jurisdiction: "Default", created: "Aug 1", marker: "missing" },
];
/** What the fake backend proposes, in turn, for a new bucket. */
const PROPOSALS = ["mallard-fox", "mallard-otter", "mallard-calm", "mallard-eager"];
const nextProposal = (current: string) => PROPOSALS[(PROPOSALS.indexOf(current) + 1) % PROPOSALS.length];
/** What a name breaks of Cloudflare's rule (3–63 lowercase letters, numbers
 *  or hyphens, a letter or number at each end), or null when it keeps it. */
const bucketNameIssue = (name: string): string | null =>
  name.length < 3 || name.length > 63 ? "3–63 characters"
  : !/^[a-z0-9-]+$/.test(name) ? "lowercase letters, numbers or hyphens only"
  : name.startsWith("-") || name.endsWith("-") ? "starts and ends with a letter or number"
  : null;
const KEY_ID = "4a2b8c1d0e9f7a6b5c4d3e2f1a0b91ef";
const SECRET = "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0";

function bucketsFor(knobs: Knobs): Bucket[] {
  switch (knobs.scan) {
    case "complete": return BUCKETS.filter((row) => row.marker !== "unreadable");
    case "partial": return BUCKETS;
    case "empty": return BUCKETS.filter((row) => row.marker === "missing");
  }
}

const hostFor = (account: string, jurisdiction: Jurisdiction) => (
  `https://${account}${jurisdiction === "Default" ? "" : `.${jurisdiction.toLowerCase()}`}.r2.cloudflarestorage.com`
);

/** Timers a panel owns; Cancel and Reset clear them all. */
function useTimers() {
  const ids = useRef<number[]>([]);
  const clear = () => { ids.current.forEach((id) => window.clearTimeout(id)); ids.current = []; };
  const after = (ms: number, run: () => void) => { ids.current.push(window.setTimeout(run, ms)); };
  useEffect(() => clear, []);
  return { after, clear };
}

/** Seconds left on a browser step: the shipped budgets are 180 s for login
 *  and 120 s for consent. */
function useCountdown(active: boolean, seconds: number) {
  const [left, setLeft] = useState(seconds);
  useEffect(() => {
    if (!active) { setLeft(seconds); return undefined; }
    const started = Date.now();
    const id = window.setInterval(() => setLeft(Math.max(0, seconds - Math.floor((Date.now() - started) / 1000))), 250);
    return () => window.clearInterval(id);
  }, [active, seconds]);
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

/* ——— the hand-off ————————————————————————————————————— */

type Phase = "idle" | "check" | "login" | "account" | "bucket" | "find" | "pick" | "create" | "verify" | "authorize" | "review";
type FailCode = "wrangler_not_found" | "authorization_timeout" | "bucket_name_taken";

interface Handoff {
  phase: Phase;
  failed: { code: FailCode } | null;
  cancelled: boolean;
  loggedIn: boolean;
  account: R2Account | null;
  jurisdiction: Jurisdiction;
  /** The name in the Bucket line's field: the proposal until it is edited. */
  name: string;
  /** The last name the backend proposed. */
  proposal: string;
  bucketCreated: boolean;
  rows: Bucket[] | null;
  /** The existing bucket chosen from the list. */
  chosen: Bucket | null;
}

const PHASES: Phase[] = ["idle", "check", "login", "account", "bucket", "find", "pick", "create", "verify", "authorize", "review"];
const at = (phase: Phase) => PHASES.indexOf(phase);

function useHandoff(mode: StorageSetupMode, knobs: Knobs) {
  const { after, clear } = useTimers();
  const initial = (): Handoff => ({ phase: "idle", failed: null, cancelled: false, loggedIn: knobs.login === "in", account: null, jurisdiction: "Default", name: "", proposal: "", bucketCreated: false, rows: null, chosen: null });
  const [state, setState] = useState<Handoff>(initial);
  // Timer callbacks read the latest state here, never inside an updater
  // (StrictMode runs updaters twice, so no side effects belong there).
  const latest = useRef(state);
  latest.current = state;
  const patch = (next: Partial<Handoff>) => setState((current) => ({ ...current, ...next }));

  const authorize = () => {
    patch({ phase: "authorize", failed: null, cancelled: false });
    if (knobs.consent === "ok") after(T.consent, () => patch({ phase: "review" }));
    else after(T.timeout, () => patch({ failed: { code: "authorization_timeout" } }));
  };
  /* The account confirmed, a new bucket gets its proposed name at once: the
     proposal is local, only its availability waits for the create. */
  const afterAccount = (account: R2Account) => {
    patch({ account, phase: mode === "create" ? "bucket" : "find", name: PROPOSALS[0], proposal: PROPOSALS[0] });
    if (mode === "existing") after(T.scan, () => patch({ rows: bucketsFor(knobs), phase: "pick" }));
  };
  const afterLogin = () => {
    const accounts = accountsFor(knobs);
    accounts.length === 1 ? afterAccount(accounts[0]) : patch({ phase: "account" });
  };
  const login = () => {
    clear();
    patch({ phase: "login", loggedIn: false, failed: null, cancelled: false, account: null, rows: null, chosen: null });
    after(T.browser, () => { patch({ loggedIn: true }); afterLogin(); });
  };
  const start = () => {
    clear();
    patch({ phase: "check", failed: null, cancelled: false });
    after(T.check, () => {
      if (knobs.wrangler === "missing") return patch({ failed: { code: "wrangler_not_found" } });
      latest.current.loggedIn ? afterLogin() : login();
    });
  };
  const continueWith = (id: string) => afterAccount(accountsFor(knobs).find((account) => account.id === id)!);
  /* The account's own buckets are the truth: a name one of them already
     wears is taken, and the step comes back editable with the reason. */
  const createAndAuthorize = () => {
    clear();
    patch({ phase: "create", failed: null, cancelled: false });
    after(T.create, () => {
      if (BUCKETS.some((bucket) => bucket.name === latest.current.name)) return patch({ phase: "bucket", failed: { code: "bucket_name_taken" } });
      patch({ bucketCreated: true });
      authorize();
    });
  };
  /* Editing the name answers a taken one; the line returns to its work. */
  const setName = (name: string) => patch({ name, failed: latest.current.failed?.code === "bucket_name_taken" ? null : latest.current.failed });
  const suggestAnother = () => {
    const proposal = nextProposal(latest.current.proposal);
    patch({ name: proposal, proposal, failed: latest.current.failed?.code === "bucket_name_taken" ? null : latest.current.failed });
  };
  const useBucket = (bucket: Bucket) => {
    clear();
    patch({ chosen: bucket, phase: "verify", failed: null, cancelled: false });
    after(T.verify, authorize);
  };
  const cancel = () => { clear(); patch({ cancelled: true }); };
  const retry = () => {
    clear();
    const current = latest.current;
    if (current.phase === "authorize") mode === "create" && !current.bucketCreated ? createAndAuthorize() : authorize();
    else if (current.phase === "create") createAndAuthorize();
    else if (current.phase === "login") login();
    else if (current.phase === "find") afterAccount(current.account!);
    else start();
  };
  const scanAgain = () => { clear(); patch({ rows: null, chosen: null }); afterAccount(latest.current.account!); };
  const reset = () => { clear(); setState(initial()); };
  return { state, patch, start, login, continueWith, setName, suggestAnother, createAndAuthorize, useBucket, cancel, retry, scanAgain, reset };
}
type Flow = ReturnType<typeof useHandoff>;

const chosenBucket = (state: Handoff) => state.chosen?.name ?? state.name;
const chosenJurisdiction = (state: Handoff) => state.chosen?.jurisdiction ?? state.jurisdiction;

/* ——— card pieces drawn with the app's classes ————————————————— */

function Frame({ children, footer }: { children: ReactNode; footer: ReactNode }) {
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">{children}</div>
      <footer className="sync-storage-setup-footer">{footer}</footer>
    </div>
  );
}

/** The page footer never changes shape: Finish setup stays greyed until the
 *  key is in the fields, under Create new and Use existing alike. */
function FooterVerbs({ ready }: { ready: boolean }) {
  return (
    <>
      <span className="sync-storage-editor-progress" aria-live="polite">{ready ? "5 of 5 set" : "0 of 5 set"}</span>
      <div>
        <button type="button" className="btn btn-ghost">Discard draft…</button>
        <button type="button" className="btn btn-ghost">Cancel</button>
        <button type="button" className="btn" disabled={!ready}>Test connection</button>
        <button type="button" className="btn btn-primary" disabled={!ready}>Finish setup</button>
      </div>
    </>
  );
}

function CardHeader({ name }: { name: string }) {
  return (
    <header>
      <span className="sync-storage-icon"><Icon name="cloud" size={18} /></span>
      <input value={name} readOnly aria-label="Storage name" />
      <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
        {["Cloudflare R2", "Local folder", "Dropbox"].map((label, index) => (
          <button key={label} type="button" role="radio" aria-checked={index === 0} className={index === 0 ? "active" : undefined}>{label}</button>
        ))}
      </div>
    </header>
  );
}

function Doors({ mode, onChoose }: { mode: StorageSetupMode | null; onChoose: (mode: StorageSetupMode) => void }) {
  const door = (icon: IconName, key: StorageSetupMode, title: string, detail: string) => (
    <button type="button" className="storage-mode-door" role="radio" aria-checked={mode === key} onClick={() => onChoose(key)}>
      <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
      <span className="storage-mode-door-detail">{detail}</span>
    </button>
  );
  return (
    <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing storage">
      {door("plus", "create", "Create new", "A new bucket in your Cloudflare account, created with Wrangler or by hand.")}
      {door("search", "existing", "Use existing", "A bucket Mallard already uses, found in your account — or a connection you paste in.")}
    </div>
  );
}

function ManualRow({ label, hint, verb }: { label: string; hint: string; verb: ReactNode }) {
  return (
    <div className="storage-setup-route-row" role="note">
      <Icon name="info" size={13} />
      <span className="storage-setup-route-label">{label}</span>
      <span className="storage-setup-guide-copy">{hint}</span>
      {verb}
    </div>
  );
}

function Field({ label, value, placeholder, wide, secret }: { label: string; value?: string; placeholder?: string; wide?: boolean; secret?: boolean }) {
  return (
    <label className={wide ? "storage-provider-url-field" : undefined}>
      <span className="form-label-row"><span className="sync-field-name">{label}</span><button type="button" className="field-help" aria-label={label}>?</button></span>
      {secret ? (
        <span className="sync-secret-input">
          <input type="password" value={value ?? ""} placeholder={placeholder} readOnly autoComplete="new-password" />
          <button type="button" className="sync-secret-reveal">Show</button>
        </span>
      ) : <input type="text" value={value ?? ""} placeholder={placeholder} readOnly />}
    </label>
  );
}

function Fields({ filled }: { filled: { account: R2Account; bucket: string; jurisdiction: Jurisdiction } | null }) {
  return (
    <div className="sync-storage-fields storage-provider-fields" aria-label="Cloudflare R2 connection details">
      <Field label="S3 API URL" wide value={filled ? `${hostFor(filled.account.id, filled.jurisdiction)}/${filled.bucket}` : undefined} placeholder="Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent" />
      <Field label="Bucket name" value={filled?.bucket} placeholder="Example: codex-sync-backups" />
      <Field label="Account ID" value={filled?.account.id} placeholder="Example: 023e105f4ecef8ad9ca31a8372d0c353" />
      <Field label="Access Key ID" value={filled ? KEY_ID : undefined} placeholder="Example: 4a2b…91ef" />
      <Field label="Secret Access Key" secret value={filled ? SECRET : undefined} placeholder="Example: a1B2…x9Y0" />
    </div>
  );
}

const chip = (label: string, value: string) => <span key={label} className="sync-storage-repo-chip"><span>{label}</span><b>{value}</b></span>;

/** The app's closed-choice pill: the same control the card header uses for
 *  the storage type. Jurisdiction is a creation-time choice with four
 *  values, so it is a segment, not a select. */
function JurisdictionSegments({ value, onChange }: { value: Jurisdiction; onChange: (next: Jurisdiction) => void }) {
  return (
    <div className="sync-segmented r2-segmented" role="radiogroup" aria-label="Jurisdiction">
      {JURISDICTIONS.map((option) => (
        <button key={option} type="button" role="radio" aria-checked={value === option} className={value === option ? "active" : undefined} onClick={() => onChange(option)}>{option}</button>
      ))}
    </div>
  );
}

/** The app's select shell, as the setup page's profile row draws it. */
function AccountSelect({ value, onChange }: { value: string; onChange: (id: string) => void }) {
  return (
    <span className="sync-select-shell r2-select-shell">
      <select value={value} aria-label="Cloudflare account" onChange={(event) => onChange(event.target.value)}>
        <option value="">Choose an account</option>
        {ACCOUNTS.map((account) => <option key={account.id} value={account.id}>{account.name} · {shortId(account.id)}</option>)}
      </select>
    </span>
  );
}

const small = (label: ReactNode, onClick: () => void, tone: "plain" | "primary" | "ghost" = "plain", inert = false) => (
  <button type="button" className={`btn storage-setup-guide-button${tone === "primary" ? " btn-primary" : tone === "ghost" ? " btn-ghost" : ""}`} disabled={inert} onClick={onClick}>{label}</button>
);

/** What every browser step says and offers. The tab was opened by Mallard;
 *  Open again re-launches it; Copy link is for another browser profile;
 *  Cancel stops the wait and keeps everything else. */
const browserFact = (budget: string) => <>Opened in your browser · <span className="r2-countdown">{budget}</span></>;
const browserControls = (onCancel: () => void) => (
  <>
    {small(<><Icon name="external-link" size={12} /> Open again</>, () => undefined)}
    {small(<><Icon name="copy" size={12} /> Copy link</>, () => undefined)}
    {small("Cancel", onCancel, "ghost")}
  </>
);

/** The scan's rows, grouped as the shipped panel groups them, with the verb
 *  on the row: a Mallard storage that is not yet connected carries
 *  Use & authorize; a connected one names its storage and opens it. */
function BucketRows({ rows, onUse, partial, onCreateInstead }: {
  rows: Bucket[]; onUse: (bucket: Bucket) => void; partial: boolean; onCreateInstead: () => void;
}) {
  const [othersOpen, setOthersOpen] = useState(false);
  const verdict = (row: Bucket): { tone: string; icon: IconName; text: string } => ({
    storage: { tone: "ok", icon: "check-circle" as IconName, text: `Mallard storage · ${row.detail}` },
    missing: { tone: "mute", icon: "ban" as IconName, text: "No Mallard data" },
    unsupported: { tone: "warn", icon: "alert-triangle" as IconName, text: `Needs a newer Mallard · ${row.detail}` },
    unreadable: { tone: "bad", icon: "alert-triangle" as IconName, text: `Could not read the marker · ${row.detail}` },
  })[row.marker];
  const row = (bucket: Bucket) => {
    const v = verdict(bucket);
    return (
      <div key={bucket.name} className={`storage-found-row r2-found-row${bucket.marker === "missing" ? " muted" : ""}`}>
        <span className="sync-storage-repository-icon"><Icon name="cloud" size={14} /></span>
        <span className="storage-found-text">
          <span className="storage-found-name">{bucket.name}</span>
          <span className={`storage-found-verdict ${v.tone}`}><Icon name={v.icon} size={12} /> {v.text}</span>
        </span>
        <span className="storage-found-facts">{chip("Jurisdiction", bucket.jurisdiction)}{chip("Created", bucket.created)}</span>
        <span className="storage-found-verb">
          {bucket.connectedAs
            ? <><span className="storage-found-connected"><Icon name="check" size={12} /> Connected · {bucket.connectedAs}</span><button type="button" className="btn btn-ghost">Open</button></>
            : bucket.marker === "storage"
              ? <button type="button" className="btn btn-primary storage-setup-guide-button" onClick={() => onUse(bucket)} aria-label={`Use ${bucket.name} and authorize`}>Use &amp; authorize</button>
              : null}
        </span>
      </div>
    );
  };
  const group = (label: string, list: Bucket[], foldable = false) => list.length > 0 && (
    <>
      {foldable ? (
        <button type="button" className="storage-found-group-head" aria-expanded={othersOpen} onClick={() => setOthersOpen((open) => !open)}>
          <span>{label}</span><b>{list.length}</b><Icon name={othersOpen ? "chevron-up" : "chevron-down"} size={12} />
        </button>
      ) : <div className="storage-found-group-head"><span>{label}</span><b>{list.length}</b></div>}
      {(!foldable || othersOpen) && list.map(row)}
    </>
  );
  const found = rows.filter((r) => r.marker === "storage");
  return (
    <div className="r2-step-list">
      {partial && (
        /* The verb stays on the Buckets line; the box only says what the
           list cannot show, and the row under "Could not be checked" names
           the bucket and the reason. */
        <div className="notice warning storage-found-notice" role="alert">
          <Icon name="alert-triangle" size={13} className="notice-mark" />
          <div className="notice-body">
            <p className="notice-line"><strong className="notice-type">Warning ·</strong> <strong className="notice-lead">Scan incomplete.</strong> {rows.filter((r) => r.marker === "unreadable").length} bucket could not be checked; a Mallard storage there would not be listed.</p>
          </div>
        </div>
      )}
      <div className="storage-found-list" aria-label="Buckets found">
        {group("Mallard storages", found)}
        {group("Needs a newer Mallard", rows.filter((r) => r.marker === "unsupported"))}
        {group("Other buckets — no Mallard data", rows.filter((r) => r.marker === "missing"), true)}
        {group("Could not be checked", rows.filter((r) => r.marker === "unreadable"))}
        {found.length === 0 && (
          <div className="storage-found-empty">
            <Icon name="search" size={24} />
            <strong>No Mallard storage in your Cloudflare account.</strong>
            <span>{rows.length} buckets checked across Default, EU, US and FedRAMP. Nothing was changed.</span>
            <span className="storage-found-empty-verbs"><button type="button" className="btn btn-primary" onClick={onCreateInstead}>Create a new bucket instead</button></span>
          </div>
        )}
      </div>
    </div>
  );
}

/* ——— the steps ——————————————————————————————————————— */

type StepStatus = "todo" | "doing" | "done" | "failed" | "stopped";
interface StepView { key: string; title: string; status: StepStatus; fact?: ReactNode; controls?: ReactNode; body?: ReactNode }

function buildSteps(mode: StorageSetupMode, flow: Flow, ui: {
  accountPick: string; setAccountPick: (id: string) => void; loginBudget: string; consentBudget: string; onCreateInstead: () => void; partial: boolean;
}): StepView[] {
  const { state } = flow;
  const interrupted: StepStatus | null = state.failed ? "failed" : state.cancelled ? "stopped" : null;
  const statusOf = (...phases: Phase[]): StepStatus => {
    if (phases.includes(state.phase)) return interrupted ?? (state.phase === "review" ? "done" : "doing");
    return at(state.phase) > at(phases[phases.length - 1]) ? "done" : "todo";
  };
  const kept = mode === "create" && state.bucketCreated ? ` · ${state.name} kept` : "";
  const busy = !interrupted && ["check", "login", "find", "create", "verify", "authorize"].includes(state.phase);

  const wrangler: StepView = (() => {
    const status = statusOf("check");
    return {
      key: "wrangler", title: "Wrangler", status,
      fact: status === "done" ? <>4.125 · <code>/opt/homebrew/bin/wrangler</code></>
        : status === "failed" ? <>Not on PATH · <code>npm install --global wrangler@latest</code></>
        : "Checking the install…",
      controls: status === "failed" ? <>{small("Copy command", () => undefined)}{small("Copy agent message", () => undefined)}{small("Try again", flow.retry, "primary")}</> : undefined,
    };
  })();

  const signIn: StepView = (() => {
    const status = statusOf("login");
    return {
      key: "sign-in", title: "Sign in", status,
      fact: status === "doing" ? browserFact(ui.loginBudget)
        : status === "stopped" ? "Stopped"
        : status === "done" ? "Wrangler session on this Mac"
        : "Uses the Wrangler session; opens the browser if there is none",
      controls: status === "doing" ? browserControls(flow.cancel)
        : status === "stopped" ? small("Try again", flow.retry, "primary")
        : status === "done" && !busy && state.phase !== "review" ? small(<><Icon name="user" size={12} /> Change login</>, flow.login) : undefined,
    };
  })();

  const account: StepView = (() => {
    const status = statusOf("account");
    return {
      key: "account", title: "Account", status,
      fact: state.account ? <>{state.account.name} · <code>{shortId(state.account.id)}</code></>
        : status === "doing" ? "Two accounts in this session"
        : "From the session",
      controls: status === "doing" ? <><AccountSelect value={ui.accountPick} onChange={ui.setAccountPick} />{ui.accountPick ? small("Continue", () => flow.continueWith(ui.accountPick), "primary") : null}</> : undefined,
    };
  })();

  /* The name stands in the fact slot, proposed and editable, with Suggest
     another beside it; the note after it says whose the name is, spells the
     rule a wrong one breaks, or names the conflict a taken one met. The
     segments and Create & authorize stay live until a bucket exists. */
  const bucket: StepView = mode === "create" ? (() => {
    const status = statusOf("bucket", "create");
    const taken = state.failed?.code === "bucket_name_taken";
    const naming = state.phase === "bucket" && (!interrupted || taken);
    const issue = bucketNameIssue(state.name);
    const valid = issue === null;
    const nameField = (
      <span className="r2-name-field">
        <input
          className="r2-name-input" value={state.name} aria-label="Bucket name" aria-invalid={!valid} spellCheck={false}
          onChange={(event) => flow.setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && valid) flow.createAndAuthorize(); }}
        />
        {small(<><Icon name="refresh" size={12} /> Suggest another</>, flow.suggestAnother)}
      </span>
    );
    return {
      key: "bucket", title: "Bucket", status,
      fact: naming ? <>{nameField}<span>{taken ? "· already exists in this account · choose another name" : issue ? `· ${issue}` : state.name === state.proposal ? "· proposed by Mallard" : "· renamed"}</span></>
        : state.bucketCreated || status === "done" ? <><code>{state.name}</code> · {state.jurisdiction} · created</>
        : state.phase === "create" ? <>Creating <code>{state.name}</code>…</>
        : "A new bucket, named here",
      controls: naming
        ? <><JurisdictionSegments value={state.jurisdiction} onChange={(jurisdiction) => flow.patch({ jurisdiction })} />{small("Create & authorize", flow.createAndAuthorize, "primary", !valid)}</>
        : state.phase === "create" ? small("Cancel", flow.cancel, "ghost")
        : undefined,
    };
  })() : (() => {
    const status = statusOf("find", "pick");
    return {
      key: "buckets", title: "Buckets", status,
      fact: status === "stopped" ? "Stopped"
        : state.phase === "find" ? "Looking across Default, EU, US and FedRAMP · read-only"
        : state.rows && status === "doing" ? <>{state.rows.length} checked · read-only</>
        : status === "done" && state.chosen ? <><code>{state.chosen.name}</code> · {state.chosen.jurisdiction}</>
        : "Lists your buckets and reads one marker in each · read-only",
      controls: status === "stopped" ? small("Try again", flow.retry, "primary")
        : state.phase === "find" ? small("Cancel", flow.cancel, "ghost")
        : state.phase === "pick" ? small(<><Icon name="refresh" size={12} /> Scan again</>, flow.scanAgain) : undefined,
      body: state.rows && state.phase === "pick"
        ? <BucketRows rows={state.rows} onUse={flow.useBucket} partial={ui.partial} onCreateInstead={ui.onCreateInstead} />
        : undefined,
    };
  })();

  const key: StepView = (() => {
    const status = statusOf("verify", "authorize");
    return {
      key: "key", title: "Authorize key", status,
      fact: status === "failed" ? `Expired after 2 minutes${kept}`
        : status === "stopped" ? `Stopped${kept}`
        : state.phase === "verify" ? <>Checking <code>{chosenBucket(state)}</code> is unchanged…</>
        : state.phase === "authorize" ? browserFact(ui.consentBudget)
        : status === "done" ? "Issued · shown once, in the fields below"
        : "Object Read & Write · this bucket only",
      controls: status === "failed" || status === "stopped" ? small("Try again", flow.retry, "primary")
        : state.phase === "authorize" ? browserControls(flow.cancel) : undefined,
    };
  })();

  const review: StepView = { key: "review", title: "Review", status: state.phase === "review" ? "done" : "todo", fact: "Test connection, then Finish setup" };
  return [wrangler, signIn, account, bucket, key, review];
}

function Step({ step }: { step: StepView }) {
  const mark = step.status === "done" ? <Icon name="check-circle" size={12} />
    : step.status === "doing" ? <span className="status-loader" />
    : step.status === "failed" ? <Icon name="alert-triangle" size={12} />
    : step.status === "stopped" ? <Icon name="ban" size={12} />
    : <span className="r2-step-dot" />;
  return (
    <li className="r2-step" data-status={step.status}>
      <span className="r2-step-mark">{mark}</span>
      <div className="r2-step-body">
        <span className="r2-step-title">{step.title}</span>
        {step.fact && <span className="r2-step-fact">{step.fact}</span>}
        {step.controls && <span className="r2-step-controls">{step.controls}</span>}
        {step.body}
      </div>
    </li>
  );
}

function ReadyLine({ state }: { state: Handoff }) {
  return (
    <span className="r2-ready" role="status">
      <span className="storage-setup-guide-connected"><Icon name="check-circle" size={13} /> Ready for review.</span>
      <span className="r2-facts">{chip("Bucket", chosenBucket(state))}{chip("Jurisdiction", chosenJurisdiction(state))}{chip("Account", state.account?.name ?? "")}{chip("Key", "this bucket only")}</span>
    </span>
  );
}

const RESET_TITLE = "Discards this setup draft and all of its progress, then opens a fresh one. A bucket already created in Cloudflare is kept.";

function StepsCard({ knobs, mode, onChooseMode, onReset }: { knobs: Knobs; mode: StorageSetupMode; onChooseMode: (mode: StorageSetupMode) => void; onReset: () => void }) {
  const flow = useHandoff(mode, knobs);
  const { state } = flow;
  const [manualEntry, setManualEntry] = useState(false);
  const [accountPick, setAccountPick] = useState("");
  const consentBudget = useCountdown(state.phase === "authorize" && !state.failed && !state.cancelled, 120);
  const loginBudget = useCountdown(state.phase === "login" && !state.failed && !state.cancelled, 180);
  const started = state.phase !== "idle";
  const ready = state.phase === "review";
  const filled = ready && state.account ? { account: state.account, bucket: chosenBucket(state), jurisdiction: chosenJurisdiction(state) } : null;
  const steps = buildSteps(mode, flow, { accountPick, setAccountPick, loginBudget, consentBudget, onCreateInstead: () => onChooseMode("create"), partial: knobs.scan === "partial" });
  const resetVerb = started && (
    <button type="button" className="btn btn-ghost storage-setup-guide-button" title={RESET_TITLE} onClick={() => { flow.reset(); setAccountPick(""); onReset(); }}>
      <Icon name="refresh" size={12} /> Reset setup
    </button>
  );
  return (
    <Frame footer={<FooterVerbs ready={!!filled} />}>
      <article className="sync-storage-card">
        <CardHeader name={filled ? filled.bucket : "R2 storage 1"} />
        {!filled && <Doors mode={mode} onChoose={onChooseMode} />}
        <div className="storage-setup-route-row agent r2-lane" role="note">
          <Icon name="terminal" size={13} />
          <span className="storage-setup-route-label">Wrangler setup</span>
          {ready ? <><ReadyLine state={state} />{resetVerb}</> : started ? (
            <>
              <span className="storage-setup-guide-copy" />
              {resetVerb}
              <ol className="r2-steps">{steps.map((step) => <Step key={step.key} step={step} />)}</ol>
            </>
          ) : (
            <>
              <span className="storage-setup-guide-copy">{mode === "create" ? "Creates a bucket and issues its key. Sign-in and consent happen in your browser." : "Lists your buckets read-only, then issues a key for the one you pick."}</span>
              <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" onClick={flow.start}><Icon name="terminal" size={13} /> {mode === "create" ? "Set up with Wrangler" : "Find in my account"}</button>
            </>
          )}
        </div>
        {mode === "create" && !filled && <ManualRow label="Manual setup" hint="Enter an existing R2 connection in the fields below." verb={<button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>} />}
        {mode === "existing" && !filled && <ManualRow label="By hand" hint="Paste the connection values instead." verb={<button type="button" className="btn-link" onClick={() => setManualEntry((open) => !open)}>{manualEntry ? "Hide fields" : "Enter by hand"}</button>} />}
        {(filled || mode === "create" || manualEntry) && <Fields filled={filled} />}
      </article>
    </Frame>
  );
}

function Direction({ knobs }: { knobs: Knobs }) {
  const [mode, setMode] = useState<StorageSetupMode | null>(null);
  const [draft, setDraft] = useState(1);
  return mode === null ? (
    <Frame footer={<FooterVerbs ready={false} />}>
      <article className="sync-storage-card"><CardHeader name={`R2 storage ${draft}`} /><Doors mode={mode} onChoose={setMode} /></article>
    </Frame>
  ) : (
    /* Reset opens a fresh draft: a new key remounts the card past the doors. */
    <StepsCard key={`${mode}-${draft}`} knobs={knobs} mode={mode} onChooseMode={setMode} onReset={() => { setMode(null); setDraft((n) => n + 1); }} />
  );
}

/* ——— the page ———————————————————————————————————————— */

function Toggle<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: [T, string][]; onChange: (next: T) => void }) {
  return (
    <div className="mock-toggle">
      <span className="mock-aside">{label}</span>
      <div className="sync-segmented" role="radiogroup" aria-label={label}>
        {options.map(([key, text]) => (
          <button key={key} type="button" role="radio" aria-checked={value === key} className={value === key ? "active" : undefined} onClick={() => onChange(key)}>{text}</button>
        ))}
      </div>
    </div>
  );
}

const CHANGES: [string, string][] = [
  ["Buttons, not links", "Change login and Scan again are the lane's small buttons (the agent-CTA size), with their marks. Cancel on a browser line is the same button in the ghost tone."],
  ["The verb on the row", "Under Use existing every Mallard storage that is not yet connected carries Use & authorize in its verb column, where a connected one shows Connected · name and Open. One click; the list folds into the Buckets line once a bucket is taken."],
  ["The footer never moves", "Finish setup and Test connection stay greyed until the key is in the fields, under Create new and Use existing alike. The page keeps one primary."],
  ["Jurisdiction as a segment", "The card header's own pill control (Default · EU · US · FedRAMP) on the Bucket line, instead of a native select."],
  ["App recipes only", "Account chooser in the setup page's select shell; the partial-scan warning in the notice box; solid hairlines between steps; the list's icon tile, chips and group heads unchanged. The only new rules are the step grid and the countdown."],
  ["Reset setup", "Ghost button at the lane's tail from the first step on, including the ready line. Shipped semantics: this draft is discarded, a fresh one opens on the doors, a created bucket is kept."],
  ["The name in view (round four)", "The Bucket line proposes a name in an editable field, with Suggest another beside it; the note after the field says whose the name is, spells the rule a wrong one breaks, or reports a taken one. A taken name (here: any the account already lists, such as team-archive) creates nothing and comes back to this line; the segments stay live until a bucket exists."],
];

const PARAMS = new URLSearchParams(location.search);
function initialKnobs(): Knobs {
  const pick = <K extends keyof Knobs>(key: K, allowed: Knobs[K][], fallback: Knobs[K]): Knobs[K] => {
    const value = PARAMS.get(key) as Knobs[K] | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    accounts: pick("accounts", ["one", "two"], "one"),
    wrangler: pick("wrangler", ["ok", "missing"], "ok"),
    login: pick("login", ["in", "out"], "in"),
    consent: pick("consent", ["ok", "timeout"], "ok"),
    scan: pick("scan", ["complete", "partial", "empty"], "complete"),
  };
}
/** `?play=Create new,Set up with Wrangler,~1600,use:mallard-brave` drives the
 *  panel after mount (`~ms` waits), for headless captures; `type:team-archive`
 *  types into the bucket-name field. */
function usePlay() {
  useEffect(() => {
    const script = PARAMS.get("play");
    if (!script) return undefined;
    const ids: number[] = [];
    let delay = 500;
    for (const step of script.split(",").map((part) => part.trim()).filter(Boolean)) {
      if (step.startsWith("~")) { delay += Number(step.slice(1)) || 0; continue; }
      ids.push(window.setTimeout(() => {
        const root = document.querySelector(".mockup-stage");
        if (!root) return;
        if (step.startsWith("use:")) {
          const row = [...root.querySelectorAll(".storage-found-row")].find((element) => element.textContent?.includes(step.slice(4)));
          row?.querySelector<HTMLButtonElement>(".btn-primary")?.click();
          return;
        }
        if (step.startsWith("type:")) {
          const field = root.querySelector<HTMLInputElement>(".r2-name-input");
          if (!field) return;
          // React listens to the native setter's change, not to `.value =`.
          Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, step.slice(5));
          field.dispatchEvent(new Event("input", { bubbles: true }));
          return;
        }
        [...root.querySelectorAll("button")].find((button) => button.textContent?.trim().startsWith(step))?.click();
      }, delay));
      delay += 250;
    }
    return () => ids.forEach((id) => window.clearTimeout(id));
  }, []);
}

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [knobs, setKnobs] = useState<Knobs>(initialKnobs);
  usePlay();
  const set = <K extends keyof Knobs>(key: K) => (value: Knobs[K]) => setKnobs((current) => ({ ...current, [key]: value }));
  const bare = PARAMS.get("only") !== null;
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Steps in the lane · A1, refined</h1>
            <p>
              Every step in view, in the app's own grammar: buttons where there were links, the verb on the
              bucket row, the footer's Finish setup greyed until the key is in, jurisdiction as a segment,
              and Reset setup at the lane's tail. Walks Create new and Use existing on a fake backend; the
              knobs change what it meets.
            </p>
          </div>
          <button type="button" className="btn btn-ghost" onClick={() => { const next = !dark; setDark(next); document.documentElement.setAttribute("data-theme", next ? "dark" : "light"); }}>
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <div className="mock-controls">
          <Toggle label="Accounts" value={knobs.accounts} options={[["one", "One"], ["two", "Two"]]} onChange={set("accounts")} />
          <Toggle label="Wrangler" value={knobs.wrangler} options={[["ok", "Installed"], ["missing", "Missing"]]} onChange={set("wrangler")} />
          <Toggle label="Sign-in" value={knobs.login} options={[["in", "Signed in"], ["out", "Signed out"]]} onChange={set("login")} />
          <Toggle label="Consent" value={knobs.consent} options={[["ok", "Completes"], ["timeout", "Times out"]]} onChange={set("consent")} />
          <Toggle label="Scan" value={knobs.scan} options={[["complete", "Complete"], ["partial", "Partial"], ["empty", "Nothing found"]]} onChange={set("scan")} />
        </div>

        <section className="mockup-panel">
          <h3>A1 · Every step in view <span className="mockup-cost">round three</span></h3>
          <div className="mockup-stage"><Direction key={JSON.stringify(knobs)} knobs={knobs} /></div>
        </section>

        {!bare && <section className="mockup-panel">
          <h3>What changed since round two</h3>
          <dl className="mock-decisions">{CHANGES.map(([term, detail]) => <div key={term}><dt>{term}</dt><dd>{detail}</dd></div>)}</dl>
        </section>}
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
body { background: var(--app-shell); }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-panel { margin-top: 30px; padding-top: 18px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 12px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-cost { margin-left: 8px; padding: 2px 7px; border-radius: var(--r-pill); color: var(--text-2); background: var(--chip-bg); font-family: var(--font-mono); font-size: var(--type-2xs); }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; gap: 10px 22px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-decisions { display: grid; gap: 10px; margin: 8px 0 0; }
.mock-decisions > div { display: grid; grid-template-columns: 200px minmax(0, 1fr); gap: 14px; }
.mock-decisions dt { color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-semibold); }
.mock-decisions dd { margin: 0; max-width: 88ch; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
`;

/* Everything a cutover would lift into App.css. Every other class on this
   page is the app's own. */
const PROPOSED_CSS = `
/* The lane, unfolded: one list under the label, in the lane's tint. Each
   step is one line — glyph, title, fact, and the controls that step owns —
   parted by the hairline every row in the app uses. A completed step keeps
   its fact in plain text; only the glyph and a failed line's colour say
   what state a step is in. */
.r2-lane > .storage-setup-guide-copy:empty { display: none; }
.r2-lane > .btn-ghost { margin-left: auto; }
.r2-steps { flex: 1 0 100%; list-style: none; margin: 2px 0 0; padding: 0; display: grid; }
.r2-step { display: grid; grid-template-columns: 18px minmax(0, 1fr); column-gap: 8px; align-items: start; padding: 6px 0; color: var(--text-2); font-size: var(--type-xs); border-top: 1px solid color-mix(in srgb, var(--border) 70%, transparent); }
.r2-step-mark { display: grid; place-items: center; width: 18px; height: 18px; color: var(--text-3); }
.r2-step[data-status="done"] .r2-step-mark { color: var(--success-text); }
.r2-step[data-status="failed"] .r2-step-mark { color: var(--danger-text); }
.r2-step-dot { width: 9px; height: 9px; border: 1.5px solid var(--border-strong); border-radius: 50%; }
.r2-step-body { min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; line-height: 18px; }
.r2-step-title { color: var(--text-1); font-weight: var(--weight-semibold); }
.r2-step[data-status="todo"] .r2-step-title { color: var(--text-3); font-weight: var(--weight-medium); }
.r2-step[data-status="doing"] .r2-step-title { color: var(--text-0); }
.r2-step[data-status="failed"] .r2-step-title, .r2-step[data-status="failed"] .r2-step-fact { color: var(--danger-text); }
.r2-step[data-status="stopped"] .r2-step-fact { color: var(--text-3); }
.r2-step-fact { color: var(--text-2); }
.r2-step[data-status="todo"] .r2-step-fact { color: var(--text-3); }
.r2-step-fact code, .storage-setup-guide-copy code { font-family: var(--font-mono); font-size: var(--type-2xs); }
.r2-step-controls { display: inline-flex; flex-wrap: wrap; align-items: center; gap: 6px; margin-left: auto; }
.r2-countdown { color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); font-variant-numeric: tabular-nums; }
/* The card header's segment, at the line's height. */
.r2-segmented > button { height: 22px; padding: 0 8px; }
/* The setup page's select shell, sized to the line rather than a form row. */
.r2-select-shell { flex: 0 1 260px; }
.r2-select-shell > select { height: 26px; padding: 0 28px 0 8px; border: 1px solid var(--control-border); border-radius: 7px; color: var(--text-0); background: var(--surface-row); font: inherit; font-size: var(--type-xs); }
/* The list a step opens, boxed under it at the fact's inset; rows keep the
   shipped grid minus the radio column, and carry their verb instead. */
.r2-step-list { flex: 1 0 100%; margin-top: 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.r2-step-list > .storage-found-list { border-bottom: 0; }
.r2-step-list > .notice { margin: 8px 10px; }
.storage-found-row.r2-found-row { grid-template-columns: 26px minmax(0, 1fr) auto auto; cursor: default; }
.storage-found-row.r2-found-row .storage-found-verb { min-width: 0; }
/* The lane, done: the ready sentence and its facts in chips. */
.r2-ready { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; flex: 1 1 300px; }
.r2-facts { display: inline-flex; flex-wrap: wrap; gap: 6px; }
.r2-facts > .sync-storage-repo-chip { font-size: var(--type-2xs); }
`;

const START_DARK = PARAMS.get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
