import { StrictMode, useEffect, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import "../src/App.css";

/**
 * Mockup: reusing an existing R2 bucket or Dropbox folder that Mallard
 * already writes to, beside creating a new one.
 *
 * Today the New-storage card has two lanes: Agent setup (creates a new
 * bucket / shared folder and fills the form) and Manual setup (paste a
 * connection you already hold). Nothing looks in the account. The POC in
 * poc/storage-discovery proved the look is cheap and read-only: list the
 * account's buckets or shared folders, read `.mallard/_storage.json` at
 * each root, keep missing / unsupported / unreadable apart, and identify a
 * storage by its destination (account + jurisdiction + bucket, or namespace
 * ID) — never by name.
 *
 * Four directions, ordered by how much they move, each an interactive
 * flow: `npm run dev`, then /mockups/storage-discovery-mockup.html (`?theme=dark`).
 * The harness switches provider (R2 / Dropbox) and scan outcome (complete /
 * partial / nothing found / can't start); every panel keeps its own step.
 */

/* ——— data ——————————————————————————————————————————— */

type Provider = "r2" | "dropbox";
type Outcome = "complete" | "partial" | "empty" | "failed";
type Marker = "storage" | "missing" | "unsupported" | "unreadable";
type Phase = "idle" | "scanning" | "results" | "preparing" | "prepared";

interface Candidate {
  id: string;
  name: string;
  marker: Marker;
  /** R2: the bucket's jurisdiction; shown as a chip because the scan
   *  covers every supported one and the row has to say which. */
  jurisdiction?: "Default" | "EU" | "FedRAMP";
  /** R2: bucket creation date from the list call. */
  created?: string;
  /** Dropbox: the caller's role on the shared folder. */
  role?: "Owner" | "Editor" | "Viewer";
  /** Dropbox: a listed share that is not mounted in the user's Dropbox. */
  unmounted?: boolean;
  /** Dropbox: the namespace ID — the identity, printed small. */
  namespace?: string;
  /** Dropbox: the team space root, not a shared folder. */
  teamRoot?: boolean;
  /** A saved connection already points at this destination. */
  connectedAs?: string;
  /** Why the marker could not be read. */
  problem?: string;
}

const R2_ROWS: Candidate[] = [
  { id: "r2-1", name: "mallard-otter", marker: "storage", jurisdiction: "Default", created: "Aug 30", connectedAs: "R2 storage" },
  { id: "r2-2", name: "mallard-brave", marker: "storage", jurisdiction: "Default", created: "Sep 2" },
  { id: "r2-3", name: "codex-sync-backups", marker: "storage", jurisdiction: "EU", created: "Jul 14" },
  { id: "r2-4", name: "mallard-fox", marker: "storage", jurisdiction: "EU", created: "Aug 3" },
  { id: "r2-5", name: "mallard-gently", marker: "unsupported", jurisdiction: "Default", created: "Sep 8" },
  { id: "r2-6", name: "team-archive", marker: "unreadable", jurisdiction: "Default", created: "Feb 1", problem: "access denied" },
  { id: "r2-7", name: "assets-prod", marker: "missing", jurisdiction: "Default", created: "Jan 12" },
  { id: "r2-8", name: "www-static", marker: "missing", jurisdiction: "Default", created: "Jan 12" },
  { id: "r2-9", name: "photos-2024", marker: "missing", jurisdiction: "EU", created: "Mar 3" },
  { id: "r2-10", name: "backups-old", marker: "missing", jurisdiction: "Default", created: "Nov 9" },
  { id: "r2-11", name: "logs-archive", marker: "missing", jurisdiction: "Default", created: "Apr 21" },
  { id: "r2-12", name: "tmp-uploads", marker: "missing", jurisdiction: "Default", created: "May 5" },
  { id: "r2-13", name: "r2-test", marker: "missing", jurisdiction: "Default", created: "Jun 30" },
  { id: "r2-14", name: "media-cdn", marker: "missing", jurisdiction: "EU", created: "Feb 17" },
  { id: "r2-15", name: "artifacts", marker: "missing", jurisdiction: "Default", created: "Aug 1" },
];

const DROPBOX_ROWS: Candidate[] = [
  { id: "dbx-1", name: "Mallard otter", marker: "storage", role: "Owner", namespace: "1234567890" },
  { id: "dbx-2", name: "Mallard team notes", marker: "storage", role: "Viewer", unmounted: true, namespace: "2233445566" },
  { id: "dbx-3", name: "Legal archive", marker: "unreadable", role: "Viewer", unmounted: true, namespace: "9988776655", problem: "not mounted" },
  { id: "dbx-4", name: "Acme team space", marker: "missing", role: "Editor", teamRoot: true, namespace: "5544332211" },
  { id: "dbx-5", name: "Family photos", marker: "missing", role: "Owner", namespace: "3344556677" },
  { id: "dbx-6", name: "Design handoff", marker: "missing", role: "Editor", namespace: "4455667788" },
  { id: "dbx-7", name: "Client X shared", marker: "missing", role: "Viewer", namespace: "5566778899" },
  { id: "dbx-8", name: "Podcast raw", marker: "missing", role: "Editor", unmounted: true, namespace: "6677889900" },
  { id: "dbx-9", name: "Contracts 2025", marker: "missing", role: "Owner", namespace: "7788990011" },
];

/** The rows a scan returns under each harness outcome. */
function rowsFor(provider: Provider, outcome: Outcome): Candidate[] {
  const all = provider === "r2" ? R2_ROWS : DROPBOX_ROWS;
  switch (outcome) {
    case "complete": return all.filter((row) => row.marker !== "unreadable");
    case "partial": return all;
    case "empty": return all.filter((row) => row.marker === "missing");
    case "failed": return [];
  }
}

interface ProviderCopy {
  display: string;
  icon: IconName;
  unit: string;
  units: string;
  /** Where the scan looks, in the user's words. */
  place: string;
  /** The scope line printed over the results. */
  scope: string;
  findVerb: string;
  /** One sentence on what the look uses and that it only reads. */
  auth: string;
  scanning: string;
  /** The wait after a row is chosen: both providers hand the credential
   *  step to the browser. */
  preparing: (name: string) => string;
  failedLead: string;
  failedBody: string;
  failedVerb: string;
  createDetail: string;
  existingDetail: string;
  agentDescription: string;
  manualHint: string;
  fieldCount: number;
}

const COPY: Record<Provider, ProviderCopy> = {
  r2: {
    display: "Cloudflare R2",
    icon: "cloud",
    unit: "bucket",
    units: "buckets",
    place: "your Cloudflare account",
    scope: "He Qu · 023e105f…72d0c353 · Default, EU and FedRAMP jurisdictions",
    findVerb: "Find in my account",
    auth: "Uses your Wrangler sign-in. Read-only: lists the buckets and reads one marker file in each.",
    scanning: "Looking through your Cloudflare account…",
    preparing: (name) => `Approve access to ${name} in your browser — Cloudflare issues an access key for this bucket only…`,
    failedLead: "Could not look in your Cloudflare account.",
    failedBody: "Wrangler is not signed in on this Mac. Sign in with wrangler login in a terminal, then try again.",
    failedVerb: "Open Terminal to sign in",
    createDetail: "A new bucket in your Cloudflare account, made by the agent or by hand.",
    existingDetail: "A bucket Mallard already uses, found in your account — or a connection you paste in.",
    agentDescription: "Fills these values for your review — you complete Cloudflare sign-in, MFA, and consent in the browser, and finish setup yourself.",
    manualHint: "Enter an existing R2 connection in the fields below.",
    fieldCount: 5,
  },
  dropbox: {
    display: "Dropbox",
    icon: "folder",
    unit: "folder",
    units: "folders",
    place: "your Dropbox",
    scope: "quhe1996@gmail.com · shared folders and team space",
    findVerb: "Find in my Dropbox",
    auth: "Read-only Dropbox access, approved in your browser: lists your shared folders and reads one marker file in each.",
    scanning: "Approve access in your browser, then Mallard looks through your shared folders…",
    preparing: (name) => `Approve access to ${name} in your browser — this is the connection Mallard keeps, renewed automatically…`,
    failedLead: "Could not look in your Dropbox.",
    failedBody: "Dropbox access was not approved — the browser window closed before consent.",
    failedVerb: "Try again",
    createDetail: "A new Mallard shared folder in your Dropbox, made by the agent.",
    existingDetail: "A shared folder Mallard already uses, found in your Dropbox — or a namespace you paste in.",
    agentDescription: "Approve Dropbox access in your browser. The agent creates a Mallard shared folder and fills this connection. Dropbox permission covers your account, beyond this folder.",
    manualHint: "Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.",
    fieldCount: 3,
  },
};

/* ——— the flow every direction walks ————————————————————— */

function useFlow(autoStart = false) {
  const [phase, setPhase] = useState<Phase>(autoStart ? "scanning" : "idle");
  const [selected, setSelected] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Candidate | null>(null);
  useEffect(() => {
    if (phase === "scanning") {
      const timer = setTimeout(() => setPhase("results"), 1500);
      return () => clearTimeout(timer);
    }
    if (phase === "preparing") {
      const timer = setTimeout(() => setPhase("prepared"), 1700);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [phase]);
  return {
    phase,
    selected,
    setSelected,
    chosen,
    start: () => { setSelected(null); setChosen(null); setPhase("scanning"); },
    use: (row: Candidate) => { setChosen(row); setPhase("preparing"); },
    cancel: () => { setChosen(null); setPhase("results"); },
    reset: () => { setSelected(null); setChosen(null); setPhase("idle"); },
  };
}

type Flow = ReturnType<typeof useFlow>;

/* ——— shared card parts ————————————————————————————————— */

function Card({ children }: { children: ReactNode }) {
  return <article className="sync-storage-card">{children}</article>;
}

function CardHeader({ provider, name }: { provider: Provider; name: string }) {
  return (
    <header>
      <span className="sync-storage-icon"><Icon name={COPY[provider].icon} size={18} /></span>
      <input value={name} readOnly aria-label="Storage name" placeholder="Storage name" />
      <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
        <button type="button" role="radio" aria-checked={provider === "r2"} className={provider === "r2" ? "active" : undefined}>Cloudflare R2</button>
        <button type="button" role="radio" aria-checked={false}>Local folder</button>
        <button type="button" role="radio" aria-checked={provider === "dropbox"} className={provider === "dropbox" ? "active" : undefined}>Dropbox</button>
      </div>
    </header>
  );
}

function AgentRouteRow({ provider }: { provider: Provider }) {
  return (
    <div className="storage-setup-route-row agent" role="note">
      <Icon name="terminal" size={13} />
      <span className="storage-setup-route-label">Agent setup</span>
      <span className="storage-setup-guide-copy">{COPY[provider].agentDescription}</span>
      <div className="storage-setup-agent-split">
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta">
          <ProviderMark provider="claude" displayName="Claude Code" size={13} /> Run with Claude Code
        </button>
        <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta storage-setup-agent-split-arrow" aria-haspopup="menu" aria-label="More agent setup routes">
          <Icon name="chevron-down" size={11} />
        </button>
      </div>
    </div>
  );
}

function ManualRouteRow({ provider, hint }: { provider: Provider; hint?: string }) {
  return (
    <div className="storage-setup-route-row" role="note">
      <Icon name="info" size={13} />
      <span className="storage-setup-route-label">Manual setup</span>
      <span className="storage-setup-guide-copy">{hint ?? COPY[provider].manualHint}</span>
      <button type="button" className="btn-link">Setup guide <Icon name="external-link" size={11} /></button>
    </div>
  );
}

function Field({ label, value, placeholder, wide, secret, present }: {
  label: string;
  value?: string;
  placeholder?: string;
  wide?: boolean;
  secret?: boolean;
  present?: boolean;
}) {
  const input = (
    <input
      type={secret ? "password" : "text"}
      value={value ?? ""}
      readOnly
      placeholder={present ? "Stored — type to replace" : placeholder}
      spellCheck={false}
    />
  );
  return (
    <label className={wide ? "storage-provider-url-field" : undefined}>
      <span className="form-label-row">
        <span className="sync-field-name">{label}</span>
        <button type="button" className="field-help" aria-label={label}>?</button>
      </span>
      {secret ? (
        <span className="sync-secret-input">
          {input}
          <button type="button" className="sync-secret-reveal">Show</button>
        </span>
      ) : input}
    </label>
  );
}

/** The provider's declared fields, empty or filled from a chosen destination. */
function Fields({ provider, chosen }: { provider: Provider; chosen?: Candidate | null }) {
  if (provider === "r2") {
    const host = chosen?.jurisdiction === "EU" ? "023e105f4ecef8ad9ca31a8372d0c353.eu.r2.cloudflarestorage.com" : "023e105f4ecef8ad9ca31a8372d0c353.r2.cloudflarestorage.com";
    return (
      <div className="sync-storage-fields storage-provider-fields" aria-label="Cloudflare R2 connection details">
        <Field label="S3 API URL" wide value={chosen ? `https://${host}/${chosen.name}` : undefined} placeholder="Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent" />
        <Field label="Bucket name" value={chosen?.name} placeholder="Example: codex-sync-backups" />
        <Field label="Account ID" value={chosen ? "023e105f4ecef8ad9ca31a8372d0c353" : undefined} placeholder="Example: 023e105f4ecef8ad9ca31a8372d0c353" />
        <Field label="Access Key ID" value={chosen ? "4a2b8c1d0e9f7a6b5c4d3e2f1a0b91ef" : undefined} placeholder="Example: 4a2b…91ef" />
        <Field label="Secret Access Key" secret present={!!chosen} placeholder="Example: a1B2…x9Y0" />
      </div>
    );
  }
  return (
    <div className="sync-storage-fields storage-provider-fields" aria-label="Dropbox connection details">
      <Field label="Namespace ID" value={chosen?.namespace} placeholder="Shared or team namespace ID" />
      <Field label="Access token" secret present={!!chosen} placeholder="User-linked Dropbox access token" />
      <Field label="Refresh token" secret present={!!chosen} placeholder="Dropbox refresh token" />
    </div>
  );
}

function SetupFooter({ provider, ready, primary, onPrimary, extra }: {
  provider: Provider;
  ready: boolean;
  primary?: string;
  onPrimary?: () => void;
  extra?: ReactNode;
}) {
  const total = COPY[provider].fieldCount;
  return (
    <footer className="sync-storage-setup-footer">
      <span className="sync-storage-editor-progress" aria-live="polite">{ready ? total : 0} of {total} set</span>
      <div>
        {extra}
        <button type="button" className="btn btn-ghost">Discard draft…</button>
        <button type="button" className="btn btn-ghost">Cancel</button>
        <button type="button" className="btn" disabled={!ready}>Test connection</button>
        <button type="button" className="btn btn-primary" disabled={!ready && !onPrimary} onClick={onPrimary}>{primary ?? "Finish setup"}</button>
      </div>
    </footer>
  );
}

/* ——— the scan and its rows ————————————————————————————— */

function verdictOf(row: Candidate): { tone: "ok" | "mute" | "warn" | "bad"; icon: IconName; text: string } {
  switch (row.marker) {
    case "storage": return { tone: "ok", icon: "check-circle", text: "Mallard storage · layout 1" };
    case "missing": return { tone: "mute", icon: "ban", text: "No Mallard data" };
    case "unsupported": return { tone: "warn", icon: "alert-triangle", text: "Newer Mallard layout (2) — update Mallard to connect" };
    case "unreadable": return { tone: "bad", icon: "alert-triangle", text: `Could not read the marker — ${row.problem}` };
  }
}

function RowFacts({ provider, row }: { provider: Provider; row: Candidate }) {
  if (provider === "r2") {
    return (
      <span className="storage-found-facts">
        <span className="sync-storage-repo-chip"><span>Jurisdiction</span><b>{row.jurisdiction}</b></span>
        <span className="sync-storage-repo-chip"><span>Created</span><b>{row.created}</b></span>
      </span>
    );
  }
  return (
    <span className="storage-found-facts">
      {row.teamRoot && <span className="sync-storage-repo-chip"><span>Team</span><b>space</b></span>}
      <span className="sync-storage-repo-chip"><span>Access</span><b>{row.role}</b></span>
      {row.unmounted && <span className="storage-found-caution"><Icon name="link-off" size={12} /> Not in your Dropbox</span>}
      {row.role === "Viewer" && row.marker === "storage" && <span className="storage-found-caution"><Icon name="alert-triangle" size={12} /> Viewer — Push will be refused</span>}
    </span>
  );
}

type Selection =
  | { mode: "use"; onUse: (row: Candidate) => void }
  | { mode: "radio"; selected: string | null; onSelect: (id: string) => void };

function CandidateRow({ provider, row, selection, quietVerb }: {
  provider: Provider;
  row: Candidate;
  selection: Selection;
  /** For a bucket without Mallard data: the quiet way to start one there. */
  quietVerb?: boolean;
}) {
  const verdict = verdictOf(row);
  const selectable = row.marker === "storage" && !row.connectedAs;
  const isSelected = selection.mode === "radio" && selection.selected === row.id;
  const classes = ["storage-found-row", row.marker === "missing" ? "muted" : "", isSelected ? "selected" : ""].filter(Boolean).join(" ");
  const choose = () => {
    if (!selectable || selection.mode !== "radio") return;
    selection.onSelect(row.id);
  };
  return (
    <div className={classes} onClick={choose} role={selection.mode === "radio" && selectable ? "radio" : undefined} aria-checked={selection.mode === "radio" && selectable ? isSelected : undefined}>
      {selection.mode === "radio" ? (
        <input type="radio" checked={isSelected} disabled={!selectable} onChange={choose} aria-label={`Use ${row.name}`} />
      ) : <span />}
      <span className="sync-storage-repository-icon"><Icon name={provider === "r2" ? "cloud" : row.teamRoot ? "user" : "folder"} size={14} /></span>
      <span className="storage-found-text">
        <span className="storage-found-name">
          {row.name}
          {row.namespace && <code>ns {row.namespace}</code>}
        </span>
        <span className={`storage-found-verdict ${verdict.tone}`}><Icon name={verdict.icon} size={12} /> {verdict.text}</span>
      </span>
      <RowFacts provider={provider} row={row} />
      <span className="storage-found-verb">
        {row.connectedAs ? (
          <>
            <span className="storage-found-connected"><Icon name="check" size={12} /> Connected · {row.connectedAs}</span>
            <button type="button" className="btn btn-ghost">Open</button>
          </>
        ) : selection.mode === "use" && selectable ? (
          <button type="button" className="btn" onClick={() => selection.onUse(row)}>Use</button>
        ) : selection.mode === "use" && row.marker === "missing" && quietVerb ? (
          <button type="button" className="btn btn-ghost" onClick={() => selection.onUse(row)}>Start a storage here</button>
        ) : null}
      </span>
    </div>
  );
}

function ScopeLine({ provider, rows, onRescan }: { provider: Provider; rows: Candidate[]; onRescan: () => void }) {
  const copy = COPY[provider];
  const storages = rows.filter((row) => row.marker === "storage").length;
  const missing = rows.filter((row) => row.marker === "missing").length;
  const unsupported = rows.filter((row) => row.marker === "unsupported").length;
  const unreadable = rows.filter((row) => row.marker === "unreadable").length;
  const tally = [
    `${storages} Mallard storage${storages === 1 ? "" : "s"}`,
    `${missing} without Mallard data`,
    unsupported ? `${unsupported} unsupported` : null,
    unreadable ? `${unreadable} could not be read` : null,
  ].filter(Boolean).join(" · ");
  return (
    <div className="storage-found-scope" role="status">
      <Icon name="search" size={13} />
      <span className="storage-found-scope-text">
        <span>{copy.scope} · {rows.length} {rows.length === 1 ? copy.unit : copy.units} checked, read-only.</span>
        <code>{tally}</code>
      </span>
      <button type="button" className="btn-link" onClick={onRescan}><Icon name="refresh" size={11} /> Scan again</button>
    </div>
  );
}

function GroupHead({ label, count, open, onToggle }: { label: string; count: number; open?: boolean; onToggle?: () => void }) {
  const inner = <><span>{label}</span><b>{count}</b>{onToggle && <Icon name={open ? "chevron-up" : "chevron-down"} size={12} />}</>;
  return onToggle
    ? <button type="button" className="storage-found-group-head" aria-expanded={open} onClick={onToggle}>{inner}</button>
    : <div className="storage-found-group-head">{inner}</div>;
}

/** The results, grouped by what the marker said: storages first (the
 *  unsupported one among them, refused in place), the rest folded, the
 *  unreadable ones named — never silently dropped. */
function CandidateList({ provider, rows, selection, quietVerb }: {
  provider: Provider;
  rows: Candidate[];
  selection: Selection;
  quietVerb?: boolean;
}) {
  const [othersOpen, setOthersOpen] = useState(false);
  const copy = COPY[provider];
  const storages = rows.filter((row) => row.marker === "storage");
  const unsupported = rows.filter((row) => row.marker === "unsupported");
  const others = rows.filter((row) => row.marker === "missing");
  const unreadable = rows.filter((row) => row.marker === "unreadable");
  return (
    <div className="storage-found-list">
      {storages.length > 0 && (
        <>
          <GroupHead label="Mallard storages" count={storages.length} />
          {storages.map((row) => <CandidateRow key={row.id} provider={provider} row={row} selection={selection} />)}
        </>
      )}
      {unsupported.length > 0 && (
        <>
          <GroupHead label="Mallard storages this version cannot open" count={unsupported.length} />
          {unsupported.map((row) => <CandidateRow key={row.id} provider={provider} row={row} selection={selection} />)}
        </>
      )}
      {others.length > 0 && (
        <>
          <GroupHead label={`Other ${copy.units} — no Mallard data`} count={others.length} open={othersOpen} onToggle={() => setOthersOpen((open) => !open)} />
          {othersOpen && others.map((row) => <CandidateRow key={row.id} provider={provider} row={row} selection={selection} quietVerb={quietVerb} />)}
        </>
      )}
      {unreadable.length > 0 && (
        <>
          <GroupHead label="Could not be checked" count={unreadable.length} />
          {unreadable.map((row) => <CandidateRow key={row.id} provider={provider} row={row} selection={selection} />)}
        </>
      )}
    </div>
  );
}

function PartialNotice({ provider, rows, onRescan }: { provider: Provider; rows: Candidate[]; onRescan: () => void }) {
  const unreadable = rows.filter((row) => row.marker === "unreadable");
  const copy = COPY[provider];
  return (
    <div className="notice warning storage-found-notice" role="alert">
      <Icon name="alert-triangle" size={13} className="notice-mark" />
      <div className="notice-body">
        <p className="notice-line">
          <strong className="notice-type">Warning ·</strong>{" "}
          <strong className="notice-lead">Scan incomplete.</strong>{" "}
          {unreadable.length} {unreadable.length === 1 ? copy.unit : copy.units} could not be read ({unreadable.map((row) => `${row.name}: ${row.problem}`).join("; ")}), so a Mallard storage there would not be listed.
        </p>
      </div>
      <button type="button" className="btn" onClick={onRescan}>Scan again</button>
    </div>
  );
}

function FailedNotice({ provider, onRetry }: { provider: Provider; onRetry: () => void }) {
  const copy = COPY[provider];
  return (
    <div className="notice danger storage-found-notice" role="alert">
      <Icon name="alert-triangle" size={13} className="notice-mark" />
      <div className="notice-body">
        <p className="notice-line">
          <strong className="notice-type">Error ·</strong>{" "}
          <strong className="notice-lead">{copy.failedLead}</strong>{" "}
          {copy.failedBody}
        </p>
      </div>
      <div className="storage-found-notice-verbs">
        <button type="button" className="btn btn-primary" onClick={onRetry}>{copy.failedVerb}</button>
        {provider === "r2" && <button type="button" className="btn" onClick={onRetry}>Try again</button>}
      </div>
    </div>
  );
}

function EmptyState({ provider, rows, onCreate, children }: { provider: Provider; rows: Candidate[]; onCreate?: () => void; children?: ReactNode }) {
  const copy = COPY[provider];
  return (
    <div className="storage-found-empty">
      <Icon name="search" size={24} />
      <strong>No Mallard storage in {copy.place}.</strong>
      <span>{rows.length} {copy.units} checked; none holds Mallard data. Nothing was changed.</span>
      {(onCreate || children) && (
        <span className="storage-found-empty-verbs">
          {onCreate && <button type="button" className="btn btn-primary" onClick={onCreate}>Create a new {copy.unit} instead</button>}
          {children}
        </span>
      )}
    </div>
  );
}

function PreparingRow({ provider, name, onCancel }: { provider: Provider; name: string; onCancel: () => void }) {
  return (
    <div className="storage-setup-guide" role="status">
      <span className="storage-setup-guide-wait"><span className="status-loader" /> {COPY[provider].preparing(name)}</span>
      <button type="button" className="btn-link" onClick={onCancel}>Cancel</button>
    </div>
  );
}

function PreparedRow({ provider, row }: { provider: Provider; row: Candidate }) {
  const copy = COPY[provider];
  const facts = provider === "r2"
    ? `${row.jurisdiction} jurisdiction · created ${row.created}`
    : `${row.role?.toLowerCase()} access`;
  return (
    <div className="storage-setup-guide" role="status">
      <Icon name="check-circle" size={13} />
      <span className="storage-setup-guide-connected">Using existing {copy.unit} <strong>{row.name}</strong> · {facts} · Mallard layout 1.</span>
      <span className="storage-setup-guide-copy">The connection below was issued for it. Review, test the connection, then finish setup.</span>
    </div>
  );
}

/** Scanning, then whichever result the harness outcome names. */
function ScanResults({ provider, outcome, flow, selection, quietVerb, onCreate, wrap }: {
  provider: Provider;
  outcome: Outcome;
  flow: Flow;
  selection: Selection;
  quietVerb?: boolean;
  onCreate?: () => void;
  /** The list is boxed differently in a card and on a page. */
  wrap?: (list: ReactNode) => ReactNode;
}) {
  const rows = rowsFor(provider, outcome);
  const box = wrap ?? ((list) => list);
  if (flow.phase === "scanning") {
    return (
      <div className="storage-setup-guide" role="status">
        <span className="storage-setup-guide-wait"><span className="status-loader" /> {COPY[provider].scanning}</span>
        <button type="button" className="btn-link" onClick={flow.reset}>Cancel</button>
      </div>
    );
  }
  if (flow.phase !== "results") return null;
  if (outcome === "failed") return <FailedNotice provider={provider} onRetry={flow.start} />;
  if (outcome === "empty") {
    return (
      <>
        <ScopeLine provider={provider} rows={rows} onRescan={flow.start} />
        {box(<CandidateList provider={provider} rows={rows} selection={selection} quietVerb={quietVerb} />)}
        <EmptyState provider={provider} rows={rows} onCreate={onCreate} />
      </>
    );
  }
  return (
    <>
      <ScopeLine provider={provider} rows={rows} onRescan={flow.start} />
      {outcome === "partial" && <PartialNotice provider={provider} rows={rows} onRescan={flow.start} />}
      {box(<CandidateList provider={provider} rows={rows} selection={selection} quietVerb={quietVerb} />)}
    </>
  );
}

function Reset({ onReset, children }: { onReset: () => void; children?: ReactNode }) {
  return (
    <p className="mock-aside">
      {children}
      <button type="button" className="mock-link" onClick={onReset}>reset</button>
    </p>
  );
}

const draftName = (provider: Provider) => provider === "r2" ? "R2 storage 1" : "Dropbox storage 1";

/* ——— Today ——————————————————————————————————————————— */

function Today({ provider }: { provider: Provider }) {
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <Card>
          <CardHeader provider={provider} name={draftName(provider)} />
          <AgentRouteRow provider={provider} />
          <ManualRouteRow provider={provider} />
          <Fields provider={provider} />
        </Card>
      </div>
      <SetupFooter provider={provider} ready={false} />
    </div>
  );
}

/* ——— A · A third lane ———————————————————————————————————— */

function DirectionA({ provider, outcome }: { provider: Provider; outcome: Outcome }) {
  const flow = useFlow();
  const copy = COPY[provider];
  const prepared = flow.phase === "prepared" && flow.chosen;
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <Card>
          <CardHeader provider={provider} name={prepared ? flow.chosen!.name : draftName(provider)} />
          {!prepared && <AgentRouteRow provider={provider} />}
          {prepared ? (
            <PreparedRow provider={provider} row={flow.chosen!} />
          ) : flow.phase === "preparing" && flow.chosen ? (
            <PreparingRow provider={provider} name={flow.chosen.name} onCancel={flow.cancel} />
          ) : (
            <div className="storage-setup-route-row" role="note">
              <Icon name="search" size={13} />
              <span className="storage-setup-route-label">Existing storage</span>
              <span className="storage-setup-guide-copy">
                Look in {copy.place} for {copy.units} Mallard already uses, and connect one. {copy.auth}
              </span>
              <button type="button" className="btn storage-setup-guide-button" disabled={flow.phase !== "idle"} onClick={flow.start}>
                <Icon name="search" size={13} /> {copy.findVerb}
              </button>
            </div>
          )}
          <ScanResults provider={provider} outcome={outcome} flow={flow} selection={{ mode: "use", onUse: flow.use }} quietVerb />
          {!prepared && <ManualRouteRow provider={provider} hint={`Or paste a connection you already hold into the fields below.`} />}
          <Fields provider={provider} chosen={prepared ? flow.chosen : null} />
        </Card>
      </div>
      <SetupFooter provider={provider} ready={!!prepared} />
      <Reset onReset={flow.reset}>
        Press {copy.findVerb}; the results slide in between the lanes and the fields, and Use fills the fields.
      </Reset>
    </div>
  );
}

/* ——— B · The question first ———————————————————————————————— */

type Mode = null | "create" | "existing";

function ModeDoor({ icon, title, detail, checked, onChoose }: { icon: IconName; title: string; detail: string; checked: boolean; onChoose: () => void }) {
  return (
    <button type="button" className="storage-mode-door" role="radio" aria-checked={checked} onClick={onChoose}>
      <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
      <span className="storage-mode-door-detail">{detail}</span>
    </button>
  );
}

function DirectionB({ provider, outcome }: { provider: Provider; outcome: Outcome }) {
  const flow = useFlow();
  const [mode, setMode] = useState<Mode>(null);
  const [manual, setManual] = useState(false);
  const copy = COPY[provider];
  const rows = rowsFor(provider, outcome);
  const prepared = flow.phase === "prepared" && flow.chosen;
  const selectedRow = rows.find((row) => row.id === flow.selected) ?? null;
  const choose = (next: Mode) => { setMode(next); setManual(false); flow.reset(); };
  const showResults = mode === "existing" && !prepared && flow.phase !== "preparing";
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <Card>
          <CardHeader provider={provider} name={prepared ? flow.chosen!.name : draftName(provider)} />
          {!prepared && (
            <div className="storage-mode-doors" role="radiogroup" aria-label="New or existing">
              <ModeDoor icon="plus" title="Create new" detail={copy.createDetail} checked={mode === "create"} onChoose={() => choose("create")} />
              <ModeDoor icon="search" title="Use existing" detail={copy.existingDetail} checked={mode === "existing"} onChoose={() => choose("existing")} />
            </div>
          )}
          {mode === "create" && (
            <>
              <AgentRouteRow provider={provider} />
              <ManualRouteRow provider={provider} hint="Or make the bucket yourself and paste its connection below." />
              <Fields provider={provider} />
            </>
          )}
          {showResults && (
            <>
              <div className="storage-setup-route-row" role="note">
                <Icon name="search" size={13} />
                <span className="storage-setup-route-label">Existing storage</span>
                <span className="storage-setup-guide-copy">{copy.auth}</span>
                {flow.phase === "idle" && (
                  <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta" onClick={flow.start}>
                    <Icon name="search" size={13} /> {copy.findVerb}
                  </button>
                )}
              </div>
              <ScanResults
                provider={provider}
                outcome={outcome}
                flow={flow}
                selection={{ mode: "radio", selected: flow.selected, onSelect: flow.setSelected }}
                onCreate={() => choose("create")}
              />
              <div className="storage-setup-route-row" role="note">
                <Icon name="info" size={13} />
                <span className="storage-setup-route-label">By hand</span>
                <span className="storage-setup-guide-copy">Already hold the connection values? Paste them instead.</span>
                <button type="button" className="btn-link" onClick={() => setManual((open) => !open)}>{manual ? "Hide fields" : "Enter by hand"}</button>
              </div>
              {manual && <Fields provider={provider} />}
            </>
          )}
          {mode === "existing" && flow.phase === "preparing" && flow.chosen && (
            <PreparingRow provider={provider} name={flow.chosen.name} onCancel={flow.cancel} />
          )}
          {prepared && (
            <>
              <PreparedRow provider={provider} row={flow.chosen!} />
              <Fields provider={provider} chosen={flow.chosen} />
            </>
          )}
        </Card>
      </div>
      {mode === "existing" && !prepared && !manual ? (
        <SetupFooter
          provider={provider}
          ready={false}
          primary={selectedRow ? `Use ${selectedRow.name}` : "Use selected"}
          onPrimary={selectedRow ? () => flow.use(selectedRow) : undefined}
        />
      ) : (
        <SetupFooter provider={provider} ready={!!prepared} />
      )}
      <Reset onReset={() => { setMode(null); setManual(false); flow.reset(); }}>
        Pick a door. Under Use existing, the page's primary is Use selected until a row is chosen; then it becomes Finish setup.
      </Reset>
    </div>
  );
}

/* ——— C · Look first, then ask ———————————————————————————————— */

function DirectionC({ provider, outcome }: { provider: Provider; outcome: Outcome }) {
  const flow = useFlow(true);
  const [dismissed, setDismissed] = useState(false);
  const copy = COPY[provider];
  const rows = rowsFor(provider, outcome);
  const prepared = flow.phase === "prepared" && flow.chosen;
  const storages = rows.filter((row) => row.marker === "storage");
  const unreadable = rows.filter((row) => row.marker === "unreadable");
  const band = flow.phase === "results" && !dismissed;
  return (
    <div className="mock-body">
      <div className="sync-storage-details-panel">
        <Card>
          <CardHeader provider={provider} name={prepared ? flow.chosen!.name : draftName(provider)} />
          {flow.phase === "scanning" && (
            <div className="storage-setup-guide" role="status">
              <span className="storage-setup-guide-wait"><span className="status-loader" /> Looking for Mallard storage in {copy.place} first…</span>
            </div>
          )}
          {band && outcome === "failed" && (
            <div className="storage-setup-guide" role="note">
              <Icon name="info" size={13} />
              <span className="storage-setup-guide-copy">
                {provider === "r2" ? "Not signed in to Cloudflare on this Mac, so Mallard could not look for existing storage first." : "Dropbox access was not approved, so Mallard could not look for existing storage first."}
              </span>
              <button type="button" className="btn-link" onClick={flow.start}>{provider === "r2" ? "Sign in and look" : "Try again"}</button>
            </div>
          )}
          {band && outcome === "empty" && (
            <div className="storage-setup-guide" role="note">
              <Icon name="check-circle" size={13} />
              <span className="storage-setup-guide-copy">No Mallard storage in {copy.place} yet — {rows.length} {copy.units} checked. Set up a new one below.</span>
              <button type="button" className="btn-link" onClick={() => setDismissed(true)}>Dismiss</button>
            </div>
          )}
          {band && (outcome === "complete" || outcome === "partial") && (
            <div className={`notice ${outcome === "partial" ? "warning" : "info"} storage-found-notice storage-found-band`} role="status">
              <Icon name={outcome === "partial" ? "alert-triangle" : "info"} size={13} className="notice-mark" />
              <div className="notice-body">
                <p className="notice-line">
                  <strong className="notice-type">{outcome === "partial" ? "Warning ·" : "Note ·"}</strong>{" "}
                  <strong className="notice-lead">{storages.length} Mallard storage{storages.length === 1 ? "" : "s"} found in {copy.place}.</strong>{" "}
                  Use one, or set up a new {copy.unit} below.
                  {outcome === "partial" && <> {unreadable.length} {unreadable.length === 1 ? copy.unit : copy.units} could not be read ({unreadable.map((row) => row.name).join(", ")}), so this list may be short.</>}
                </p>
                <div className="storage-found-list">
                  {storages.map((row) => <CandidateRow key={row.id} provider={provider} row={row} selection={{ mode: "use", onUse: flow.use }} />)}
                </div>
              </div>
              <button type="button" className="btn btn-ghost" onClick={() => setDismissed(true)}>Dismiss</button>
            </div>
          )}
          {flow.phase === "preparing" && flow.chosen && <PreparingRow provider={provider} name={flow.chosen.name} onCancel={flow.cancel} />}
          {prepared ? (
            <PreparedRow provider={provider} row={flow.chosen!} />
          ) : flow.phase !== "preparing" && (
            <>
              <AgentRouteRow provider={provider} />
              <ManualRouteRow provider={provider} />
            </>
          )}
          <Fields provider={provider} chosen={prepared ? flow.chosen : null} />
        </Card>
      </div>
      <SetupFooter provider={provider} ready={!!prepared} />
      <Reset onReset={() => { setDismissed(false); flow.start(); }}>
        The look starts when the card opens; nothing was pressed. Dismiss leaves today's card.
      </Reset>
    </div>
  );
}

/* ——— D · Its own door and page ———————————————————————————————— */

function DirectionD({ provider, outcome }: { provider: Provider; outcome: Outcome }) {
  const flow = useFlow();
  const [entered, setEntered] = useState(false);
  const copy = COPY[provider];
  const rows = rowsFor(provider, outcome);
  const prepared = flow.phase === "prepared" && flow.chosen;
  const selectedRow = rows.find((row) => row.id === flow.selected) ?? null;
  return (
    <div className="mock-body">
      <section className="landing-picker mock-landing" aria-label="Storages landing">
        <div className="landing-picker-head">
          <h1 className="settings-section-title">Storages</h1>
          <span className="landing-picker-count">1 on this machine</span>
          <button type="button" className="btn" onClick={() => setEntered(true)}><Icon name="search" size={13} /> Use existing…</button>
          <button type="button" className="btn btn-primary mock-no-auto"><Icon name="plus" size={13} /> Add storage…</button>
        </div>
        <ul className="landing-picker-grid landing-picker-doors">
          {[["cloud", "Cloudflare R2"], ["drive", "Local folder"], ["folder", "Dropbox"]].map(([icon, label]) => (
            <li key={label}>
              <button type="button" className="landing-picker-door">
                <span className="landing-picker-door-title"><Icon name={icon as IconName} size={17} /><strong>{label}</strong><Icon name="plus" size={13} /></span>
                <span className="landing-picker-door-bar" aria-hidden="true" />
                <span className="landing-picker-door-bar short" aria-hidden="true" />
              </button>
            </li>
          ))}
          <li>
            <button type="button" className="landing-picker-door storage-find-door" onClick={() => setEntered(true)}>
              <span className="landing-picker-door-title"><Icon name="search" size={17} /><strong>Use existing storage</strong><Icon name="chevron-right" size={13} /></span>
              <span className="landing-picker-door-bar" aria-hidden="true" />
              <span className="landing-picker-door-bar short" aria-hidden="true" />
            </button>
          </li>
        </ul>
        <p className="landing-picker-caption"><Icon name="folder" size={14} /><Icon name="catalog" size={14} /><span>Projects & Catalog sync here</span></p>
      </section>

      {entered && !prepared && (
        <div className="storage-find-page">
          <div className="mock-crumbs"><Icon name="storage" size={13} /> Storages <Icon name="chevron-right" size={11} /> <strong>Use existing storage</strong></div>
          <div className="storage-find-scope">
            <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
              <button type="button" role="radio" aria-checked={provider === "r2"} className={provider === "r2" ? "active" : undefined}>Cloudflare R2</button>
              <button type="button" role="radio" aria-checked={provider === "dropbox"} className={provider === "dropbox" ? "active" : undefined}>Dropbox</button>
              <button type="button" role="radio" aria-checked={false}>Local folder</button>
            </div>
            {provider === "r2" && (
              <label>Account
                <select defaultValue="a">
                  <option value="a">He Qu · 023e105f…72d0c353</option>
                  <option value="b">Acme Inc · 7f1c22a0…9d4e0b11</option>
                </select>
              </label>
            )}
            <span className="storage-find-scope-note">{copy.auth}</span>
            <button type="button" className="btn btn-primary" disabled={flow.phase !== "idle"} onClick={flow.start}><Icon name="search" size={13} /> {provider === "r2" ? "Scan account" : "Sign in and scan"}</button>
          </div>
          {flow.phase === "idle" && (
            <div className="storage-find-list">
              <div className="storage-found-empty">
                <Icon name="search" size={24} />
                <strong>Nothing scanned yet.</strong>
                <span>The scan reads one marker file per {copy.unit} and changes nothing. A Mallard storage is recognised by that marker, whatever the {copy.unit} is named.</span>
              </div>
            </div>
          )}
          {flow.phase === "preparing" && flow.chosen && (
            <div className="storage-find-list"><PreparingRow provider={provider} name={flow.chosen.name} onCancel={flow.cancel} /></div>
          )}
          <ScanResults
            provider={provider}
            outcome={outcome}
            flow={flow}
            selection={{ mode: "radio", selected: flow.selected, onSelect: flow.setSelected }}
            wrap={(list) => <div className="storage-find-list">{list}</div>}
          />
          {flow.phase === "results" && outcome !== "failed" && (
            <footer className="sync-storage-setup-footer">
              <span>{selectedRow ? `Continue prepares a New storage draft for ${selectedRow.name}; nothing connects until you finish setup there.` : "Choose one Mallard storage to continue."}</span>
              <div>
                <button type="button" className="btn btn-ghost" onClick={() => { setEntered(false); flow.reset(); }}>Cancel</button>
                <button type="button" className="btn btn-primary" disabled={!selectedRow} onClick={() => selectedRow && flow.use(selectedRow)}>
                  {selectedRow ? `Continue with ${selectedRow.name}` : "Continue"} <Icon name="chevron-right" size={12} />
                </button>
              </div>
            </footer>
          )}
        </div>
      )}

      {prepared && (
        <>
          <div className="mock-crumbs"><Icon name="storage" size={13} /> Storages <Icon name="chevron-right" size={11} /> <strong>New storage</strong></div>
          <div className="sync-storage-details-panel">
            <Card>
              <CardHeader provider={provider} name={flow.chosen!.name} />
              <PreparedRow provider={provider} row={flow.chosen!} />
              <Fields provider={provider} chosen={flow.chosen} />
            </Card>
          </div>
          <SetupFooter provider={provider} ready />
        </>
      )}
      <Reset onReset={() => { setEntered(false); flow.reset(); }}>
        Use existing… (or the dashed door) opens its own page; Continue hands the chosen destination to the New-storage page you already know.
      </Reset>
    </div>
  );
}

/* ——— harness ————————————————————————————————————————— */

function Panel({ title, cost, note, children }: { title: string; cost: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title} <span className="mockup-cost">{cost}</span></h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Toggle<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: [T, string][]; onChange: (next: T) => void }) {
  return (
    <div className="mock-toggle">
      <span className="storage-setup-route-label">{label}</span>
      <div className="sync-segmented" role="radiogroup" aria-label={label}>
        {options.map(([key, text]) => (
          <button key={key} type="button" role="radio" aria-checked={value === key} className={value === key ? "active" : undefined} onClick={() => onChange(key)}>{text}</button>
        ))}
      </div>
    </div>
  );
}

const NOTES = {
  today: <>
    As it ships. A new storage draft opens on its provider with two lanes: <strong>Agent setup</strong>,
    which runs the CLI through a coding agent to make a <em>new</em> bucket or shared folder
    (<code>mallard-otter</code>, <code>Mallard otter</code>) and fill the form; and <strong>Manual
    setup</strong>, which takes a connection you already hold. Reusing a storage that another
    machine set up means finding the bucket in Cloudflare's dashboard, minting an access key by
    hand, and pasting four values. Nothing in the app looks in the account, although the POC
    showed the look is one list call plus one 4 KiB read per bucket, all read-only.
  </>,
  a: <>
    Nothing moves; one lane is added between the two that exist. <strong>Existing storage</strong> says
    where it will look and what it uses, and <strong>Find in my account</strong> runs the scan in place.
    The results slide in under the lanes — Mallard storages first, the buckets without Mallard
    data folded, anything unreadable named rather than dropped — and <strong>Use</strong> on a row
    opens the browser for that bucket's access key (the same broker the agent route uses), then
    fills the fields below and swaps the lanes for a green "Using existing bucket…" row. A bucket
    already connected shows its storage name and an Open door instead of Use. This is the
    Cloudflare-dashboard shape (a picker on the resource, create beside it) folded into the card
    the user already knows. Cost: one route row, one typed scan command per provider, one
    "authorize this destination" command. Downside: the card gets long when all three lanes and
    fifteen rows are visible at once.
  </>,
  b: <>
    The question first. Under the header the card asks <strong>Create new</strong> or <strong>Use
    existing</strong> as two doors, the way Firebase's <code>init</code>, Obsidian Sync, and GitHub
    Desktop ask before showing any form. Create new is today's card. Use existing is the scan —
    rows are radios, the page's primary reads <strong>Use selected</strong> until one is chosen —
    with manual entry demoted to a "by hand" line under the list, because pasting a connection is
    also using an existing bucket. Once a row is used, the doors give way to the prepared card
    with Finish setup. Cost: the two doors, a mode on the draft, the same two commands as A. It
    makes the manual route one click further for people who have their values ready.
  </>,
  c: <>
    Look first, then ask, the way Vercel's CLI says "Found project — link to it?". The card
    opens as it does today, but the scan starts at once in the background, and when it
    finds Mallard storages a band above the lanes lists them with <strong>Use</strong> on each and
    a Dismiss. Nothing found becomes one quiet line; not signed in becomes an offer to sign in and
    look. The common second-machine case costs no clicks at all, and the create route is never
    hidden. Cost: the same commands as A, plus a scan that runs without being asked — up to 120 s
    of network and, for Dropbox, a browser consent nobody requested — so it needs the
    provider sign-in to be there already, and for Dropbox it only makes sense after consent.
  </>,
  d: <>
    Its own door and its own page, the way Amplify separates <code>add storage</code> from{" "}
    <code>import storage</code>. The Storages landing gets <strong>Use existing…</strong> beside Add
    storage, and the empty landing a dashed door for it — the second-machine moment is exactly
    when the landing is empty. The page owns the scope: provider, the account when Wrangler
    reports more than one, and the read-only sentence; the results get the full width, and{" "}
    <strong>Continue</strong> hands the chosen destination to the New-storage page you already
    know, prepared and waiting for Test connection and Finish setup. Cost: a landing verb, a
    door, a page with a path-bar crumb, the same commands. The most room and the clearest
    separation, at the price of one more screen and a verb on the landing head that has held
    only Add.
  </>,
};

const STANDARDS: [string, string, string][] = [
  ["Firebase CLI · init", "Use an existing project · Create a new project · Don't set up", "A three-way choice before any form; the existing branch lists what the account holds. → B"],
  ["Obsidian Sync", "Create new remote vault · Pick remote vault and connect", "Two verbs on the same panel; connecting an existing vault lists the remote vaults. → B"],
  ["GitHub Desktop", "Create a new repository · Add an existing repository · Clone a repository", "Distinct entries, each with its own dialog. → D"],
  ["AWS Amplify", "amplify add storage vs amplify import storage", "Import is its own command that searches the account and picks a bucket. → D"],
  ["Cloudflare dashboard · bindings", "R2 bucket ▾ select, Create bucket beside it", "The picker sits on the resource field; create is the escape hatch. → A"],
  ["Vercel CLI", "Found project 'x'. Link to it? (y/N)", "Looks first, offers the match, falls back to Link to existing / create. → C"],
];

const DECISIONS = [
  ["Where the scan runs", "The POC handoff proposes typed Tauri commands (in-app Rust discovery reusing the Wrangler token and a read-only Dropbox grant). Creation runs through the CLI + agent instead. If discovery has to take the agent route too, the scanning row becomes today's \"Listening for your agent…\" and the list is unchanged."],
  ["Dropbox asks twice", "The POC's read-only look needs sharing.read, which the storage grant lacks; connecting then needs the storage grant for that namespace. Either two browser consents (as drawn), or one consent up front that carries both — simpler for the user, but a write-capable token before anything is chosen."],
  ["Buckets without Mallard data", "Drawn folded, with a quiet \"Start a storage here\" (A only). The manual route already accepts any bucket, so this hides nothing new; it can also be cut, leaving the fold as information only."],
  ["Jurisdictions", "Drawn as one scan across Default, EU and FedRAMP with a chip per row, as Cloudflare's own list does; US is not listed because the adapter does not support it. A per-scan jurisdiction select is the smaller backend change."],
  ["The word", "\"Use existing\" matches the bundle dialog's verb and Firebase; Obsidian says Connect, Amplify says Import. Rows say Use; the storage connects at Finish setup, as today."],
];

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [provider, setProvider] = useState<Provider>("r2");
  const [outcome, setOutcome] = useState<Outcome>("complete");
  const key = `${provider}-${outcome}`;
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Create a new bucket, or use one Mallard already has</h1>
            <p>
              A New-storage draft can make a new R2 bucket or Dropbox folder through the agent, or
              take a connection pasted by hand. It cannot look in the account for storage that
              another machine already set up, although the POC showed that look is read-only and
              cheap. Four directions below, ordered by how much they move: a third lane; the
              question first; look first, then ask; a door and a page of its own. Every panel walks
              the whole flow — scan, pick, browser step, prepared card — and keeps its own state.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <div className="mock-controls">
          <Toggle label="Provider" value={provider} options={[["r2", "Cloudflare R2"], ["dropbox", "Dropbox"]]} onChange={setProvider} />
          <Toggle label="Scan outcome" value={outcome} options={[["complete", "Complete"], ["partial", "Partial"], ["empty", "Nothing found"], ["failed", "Can't start"]]} onChange={setOutcome} />
        </div>

        <section className="mockup-panel">
          <h3>What the neighbours do <span className="mockup-cost">the patterns the directions borrow</span></h3>
          <table className="mock-standards">
            <thead><tr><th>Product</th><th>Its words</th><th>Shape, and which direction takes it</th></tr></thead>
            <tbody>
              {STANDARDS.map(([product, words, shape]) => (
                <tr key={product}><td>{product}</td><td><code>{words}</code></td><td>{shape}</td></tr>
              ))}
            </tbody>
          </table>
        </section>

        <Panel title="Today" cost="ships now" note={NOTES.today}><Today key={key} provider={provider} /></Panel>
        <Panel title="A · A third lane" cost="one row, two commands" note={NOTES.a}><DirectionA key={key} provider={provider} outcome={outcome} /></Panel>
        <Panel title="B · The question first" cost="two doors, a mode, two commands" note={NOTES.b}><DirectionB key={key} provider={provider} outcome={outcome} /></Panel>
        <Panel title="C · Look first, then ask" cost="an unasked scan, two commands" note={NOTES.c}><DirectionC key={key} provider={provider} outcome={outcome} /></Panel>
        <Panel title="D · A door and a page" cost="a landing verb, a page, two commands" note={NOTES.d}><DirectionD key={key} provider={provider} outcome={outcome} /></Panel>

        <section className="mockup-panel">
          <h3>Open decisions <span className="mockup-cost">the same in every direction</span></h3>
          <dl className="mock-decisions">
            {DECISIONS.map(([term, detail]) => (
              <div key={term}><dt>{term}</dt><dd>{detail}</dd></div>
            ))}
          </dl>
        </section>
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
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-cost { margin-left: 8px; padding: 2px 7px; border-radius: var(--r-pill); color: var(--text-2); background: var(--chip-bg); font-family: var(--font-mono); font-size: var(--type-2xs); }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-panel > p code { font-family: var(--font-mono); font-size: var(--type-xs); }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-link { margin-left: 6px; padding: 0; border: 0; color: var(--text-2); background: none; font: inherit; text-decoration: underline; text-underline-offset: 2px; cursor: pointer; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; gap: 10px 28px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-standards { width: 100%; border-collapse: collapse; margin: 8px 0 0; color: var(--text-2); font-size: var(--type-xs); line-height: 1.45; }
.mock-standards th { padding: 6px 10px 6px 0; text-align: left; color: var(--text-3); font-size: var(--type-3xs); font-weight: var(--weight-semibold); letter-spacing: 1px; text-transform: uppercase; border-bottom: 1px solid var(--border); }
.mock-standards td { padding: 7px 10px 7px 0; vertical-align: top; border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent); }
.mock-standards td:first-child { color: var(--text-0); white-space: nowrap; }
.mock-standards code { font-family: var(--font-mono); font-size: var(--type-2xs); }
.mock-decisions { display: grid; gap: 10px; margin: 8px 0 0; }
.mock-decisions > div { display: grid; grid-template-columns: 200px minmax(0, 1fr); gap: 14px; }
.mock-decisions dt { color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-semibold); }
.mock-decisions dd { margin: 0; max-width: 88ch; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
.mock-landing { padding-bottom: 6px; border-bottom: 1px dashed var(--border); }
.mock-crumbs { display: flex; align-items: center; gap: 6px; color: var(--text-2); font-family: var(--font-display); font-size: var(--type-xs); }
.mock-crumbs strong { color: var(--text-0); }
.mock-body .sync-storage-setup-footer > span:first-child { color: var(--text-3); font-size: var(--type-xs); }
`;

/* Everything a cutover would lift into App.css. Every other class on this
   page is the app's own. */
const PROPOSED_CSS = `
/* The scan's scope line: who was looked at, how many, and the tally, in the
   guide row's voice, with Scan again at the tail. */
.storage-found-scope { padding: 8px 14px; display: flex; align-items: center; flex-wrap: wrap; gap: 4px 9px; border-bottom: 1px solid var(--border); color: var(--text-2); background: var(--surface-subtle); font-size: var(--type-xs); }
.storage-found-scope > svg { flex: none; }
.storage-found-scope-text { min-width: 0; flex: 1 1 300px; display: flex; flex-direction: column; gap: 2px; }
.storage-found-scope-text code { color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); }
.storage-found-scope > .btn-link { min-height: 24px; padding: 2px 0; flex: none; display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }

/* Group heads in the field-name voice; a foldable one is a button. */
.storage-found-group-head { width: 100%; padding: 6px 14px; display: flex; align-items: center; gap: 8px; border: 0; border-bottom: 1px solid var(--border); color: var(--text-3); background: var(--surface-subtle); font: inherit; font-size: var(--type-3xs); font-weight: var(--weight-semibold); letter-spacing: 1px; text-transform: uppercase; text-align: left; }
.storage-found-group-head > b { color: var(--text-2); font-family: var(--font-mono); font-size: var(--type-2xs); }
.storage-found-group-head > svg { margin-left: auto; }
button.storage-found-group-head { cursor: pointer; }
button.storage-found-group-head:hover { color: var(--text-1); background: var(--bg-hover); }

/* One row per candidate: choice, glyph, name over its verdict, facts, verb.
   The verdict is the marker's word, coloured by rung and never only by
   colour — the glyph and the sentence carry it. */
.storage-found-row { min-height: 50px; padding: 7px 14px; display: grid; grid-template-columns: auto 26px minmax(0, 1fr) auto auto; align-items: center; gap: 10px; border-bottom: 1px solid var(--border); background: var(--surface-row); }
.storage-found-row:last-child { border-bottom: 0; }
.storage-found-row[role="radio"] { cursor: pointer; }
.storage-found-row[role="radio"]:hover { background: var(--surface-row-hover); }
.storage-found-row.selected { background: var(--surface-row-active); }
.storage-found-row > input[type="radio"] { margin: 0; }
.storage-found-row.muted .storage-found-name { color: var(--text-2); font-weight: var(--weight-medium); }
.storage-found-text { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.storage-found-name { min-width: 0; overflow: hidden; color: var(--text-0); font-size: var(--type-sm); font-weight: var(--weight-semibold); text-overflow: ellipsis; white-space: nowrap; }
.storage-found-name > code { margin-left: 7px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); font-weight: var(--weight-regular); }
.storage-found-verdict { display: inline-flex; align-items: center; gap: 5px; color: var(--text-3); font-size: var(--type-2xs); }
.storage-found-verdict > svg { flex: none; }
.storage-found-verdict.ok { color: var(--success-text); }
.storage-found-verdict.warn { color: var(--warning-text); }
.storage-found-verdict.bad { color: var(--danger-text); }
.storage-found-facts { display: flex; align-items: center; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
.storage-found-caution { display: inline-flex; align-items: center; gap: 4px; color: var(--warning-text); font-size: var(--type-2xs); font-weight: var(--weight-medium); white-space: nowrap; }
.storage-found-verb { min-width: 58px; display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
.storage-found-connected { display: inline-flex; align-items: center; gap: 5px; padding: 3px 8px; border: 1px solid color-mix(in srgb, var(--green) 45%, var(--border)); border-radius: var(--r-pill); color: var(--success-text); background: var(--success-soft); font-size: var(--type-2xs); font-weight: var(--weight-semibold); white-space: nowrap; }

/* Nothing found: the mark, the fact, the way on. */
.storage-found-empty { padding: 20px 16px; display: flex; flex-direction: column; align-items: center; gap: 6px; color: var(--text-2); font-size: var(--type-xs); text-align: center; }
.storage-found-empty > svg { color: var(--text-3); margin-bottom: 4px; }
.storage-found-empty > strong { color: var(--text-0); font-size: var(--type-sm); }
.storage-found-empty-verbs { margin-top: 6px; display: flex; gap: 6px; }

/* A scan notice seated in the card at the field inset; its verbs on a row of
   their own, like the folder-collision band. */
.sync-storage-card > .storage-found-notice, .storage-find-page > .storage-found-notice { margin: 10px 14px; flex-wrap: wrap; }
.storage-find-page > .storage-found-notice { margin: 0; }
.storage-found-notice-verbs { flex: 1 0 100%; display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
/* C: the found band carries rows inside the notice body. */
.storage-found-band .storage-found-list { margin-top: 6px; border: 1px solid var(--border); border-radius: 6px; overflow: hidden; }

/* B: the question, as two doors in the card. Dashed until chosen, then the
   active-row fill and the strong edge — selection in the app's own grammar. */
.storage-mode-doors { padding: 12px 14px; display: grid; grid-template-columns: 1fr 1fr; gap: 10px; border-bottom: 1px solid var(--border); }
.storage-mode-door { min-width: 0; display: flex; flex-direction: column; gap: 5px; padding: 10px 12px; border: 2px dashed var(--border-mid); border-radius: var(--retro-panel-radius); color: var(--text-2); background: transparent; font: inherit; text-align: left; cursor: pointer; }
.storage-mode-door:hover { color: var(--text-0); background: var(--bg-hover); }
.storage-mode-door[aria-checked="true"] { border-style: solid; border-color: var(--border-strong); color: var(--text-0); background: var(--surface-row-active); }
.storage-mode-door-title { display: flex; align-items: center; gap: 8px; }
.storage-mode-door-title > strong { font-family: var(--font-display); font-size: var(--type-md); font-weight: var(--weight-bold); }
.storage-mode-door-detail { font-size: var(--type-xs); line-height: 1.45; }

/* D: the page — scope band, boxed list, the same footer as setup. */
.storage-find-page { display: flex; flex-direction: column; gap: 12px; }
.storage-find-scope { padding: 10px 14px; display: flex; align-items: center; flex-wrap: wrap; gap: 8px 14px; border: var(--box-border); border-radius: var(--retro-panel-radius); color: var(--text-2); background: var(--surface-card); box-shadow: var(--shadow-card); font-size: var(--type-xs); }
.storage-find-scope > label { display: inline-flex; align-items: center; gap: 7px; }
.storage-find-scope > span { display: inline-flex; align-items: center; gap: 6px; }
.storage-find-scope select { height: 28px; padding: 0 26px 0 8px; border: 1px solid var(--control-border); border-radius: 7px; color: var(--text-0); background: var(--surface-row); font: inherit; font-size: var(--type-xs); }
.storage-find-scope-note { flex: 1 1 260px; color: var(--text-3); }
.storage-find-scope > .btn { margin-left: auto; }
.storage-find-list { border: var(--box-border); border-radius: var(--retro-panel-radius); background: var(--surface-card); box-shadow: var(--shadow-card); overflow: hidden; }
.storage-find-list > .storage-setup-guide { border-bottom: 0; }
.storage-find-door .landing-picker-door-title > strong { color: var(--accent); }
/* The landing head's second verb sits beside Add: only the first button in
   the cluster takes the auto margin. */
.landing-picker-head > .btn + .btn { margin-left: 0; }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
