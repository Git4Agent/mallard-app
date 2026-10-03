import { StrictMode, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "../src/components/Icons";
import { StorageDiscoveryPanel, candidateSelectable, type DiscoveryState } from "../src/components/project-sync/StorageDiscovery";
import {
  StorageEditor,
  fieldsSet,
  type SecretPresence,
  type StorageEditorForm,
  type StorageSetupMode,
} from "../src/components/project-sync/StorageSettings";
import type { StorageDiscoveryResult, StorageProviderDescriptor } from "../src/types";
import "../src/App.css";

/**
 * Live harness for the shipped Use-existing UI: the real StorageEditor and
 * StorageDiscoveryPanel, driven by a stand-in for the workspace's state and
 * a fake backend on timers. Verification only — delete after review.
 * `npm run dev`, then /mockups/storage-discovery-live.html (`?theme=dark`).
 */

const R2: StorageProviderDescriptor = {
  key: "r2",
  config_version: 1,
  display_name: "Cloudflare R2",
  description: "An R2 bucket reached through its S3 API.",
  default_name: "R2 storage",
  display_rank: 10,
  icon: "cloud",
  clear_mode: "unconditional",
  fields: [
    { field_id: "api_url", label: "S3 API URL", kind: "url", required: true, placeholder: "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent", help: "How to find the Cloudflare R2 S3 API URL" },
    { field_id: "bucket", label: "Bucket name", kind: "text", required: true, placeholder: "Example: codex-sync-backups", help: "How to create a Cloudflare R2 bucket" },
    { field_id: "account_id", label: "Account ID", kind: "text", required: true, placeholder: "Example: 023e105f4ecef8ad9ca31a8372d0c353", help: "Where to find your Cloudflare Account ID" },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true, placeholder: "Example: 4a2b…91ef", help: "How to create an R2 Access Key ID" },
  ],
  secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, placeholder: "Example: a1B2…x9Y0", help: "How to create an R2 Secret Access Key" }],
  managed_secret_id: null,
  setup_guide_url: "https://developers.cloudflare.com/r2/",
  manual_setup_hint: "Enter an existing R2 connection in the fields below.",
  setup_doors: {
    create_hint: "A new bucket in your Cloudflare account.",
    existing_hint: "A bucket Mallard already uses.",
    existing_help: {
      label: "Set up team access",
      url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/",
    },
  },
  discovery: {
    unit: "bucket",
    unit_plural: "buckets",
    place: "your Cloudflare account",
    find_label: "Find in my account",
    access_hint: "Uses your Wrangler sign-in. Read-only: lists the buckets and reads one marker file in each.",
    scanning_hint: "Looking through your Cloudflare account…",
    prepare_hint: "Cloudflare issues an access key for this bucket only.",
  },
};

const DROPBOX: StorageProviderDescriptor = {
  key: "dropbox",
  config_version: 1,
  display_name: "Dropbox",
  description: "A shared folder in your Dropbox, new or one you already use. Uploads up to 150 MiB per object.",
  default_name: "Dropbox storage",
  display_rank: 30,
  icon: "folder",
  clear_mode: "unconditional",
  fields: [
    { field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, placeholder: "Shared or team namespace ID", help: "Use the same namespace ID on each machine. This value cannot change after setup." },
  ],
  secrets: [
    { secret_id: "access_token", label: "Access token", required: true, placeholder: "User-linked Dropbox access token" },
    { secret_id: "refresh_token", label: "Refresh token", required: true, placeholder: "Dropbox refresh token" },
  ],
  managed_secret_id: "refresh_token",
  setup_guide_url: "https://www.dropbox.com/developers",
  manual_setup_hint: "Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.",
  setup_doors: {
    create_hint: "A new shared folder in your Dropbox.",
    existing_hint: "A shared folder Mallard already uses.",
  },
  discovery: {
    unit: "shared folder",
    unit_plural: "shared folders",
    place: "your Dropbox",
    find_label: "Find in my Dropbox",
    access_hint: "Read-only Dropbox access, approved in your browser: lists your shared folders and reads one marker file in each.",
    scanning_hint: "Approve access in your browser, then Mallard looks through your shared folders…",
    prepare_hint: "This is the connection Mallard keeps, renewed automatically.",
  },
};

const LOCAL: StorageProviderDescriptor = {
  key: "local",
  config_version: 1,
  display_name: "Local folder",
  description: "A folder on this Mac or on a mounted drive.",
  display_rank: 20,
  icon: "drive",
  clear_mode: "revision_checked",
  fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true, placeholder: "e.g. /Volumes/backup/mallard-storage" }],
  secrets: [],
};

type Outcome = "complete" | "partial" | "empty" | "failed";

function scan(provider: StorageProviderDescriptor, outcome: Outcome): StorageDiscoveryResult {
  const r2 = provider.key === "r2";
  const rows: StorageDiscoveryResult["candidates"] = r2 ? [
    { candidate_id: "r2-1", name: "mallard-otter", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "Default" }, { label: "Created", value: "Aug 30" }], cautions: [], connected_storage_id: "storage-1" },
    { candidate_id: "r2-2", name: "mallard-brave", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "Default" }, { label: "Created", value: "Sep 2" }], cautions: [] },
    { candidate_id: "r2-3", name: "codex-sync-backups", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Jurisdiction", value: "EU" }, { label: "Created", value: "Jul 14" }], cautions: [] },
    { candidate_id: "r2-4", name: "mallard-gently", marker: "unsupported", marker_detail: "layout 2", facts: [{ label: "Jurisdiction", value: "Default" }, { label: "Created", value: "Sep 8" }], cautions: [] },
    { candidate_id: "r2-5", name: "team-archive", marker: "unreadable", marker_detail: "access denied", facts: [{ label: "Jurisdiction", value: "Default" }], cautions: [] },
    ...["assets-prod", "www-static", "photos-2024", "backups-old", "logs-archive", "tmp-uploads", "r2-test", "media-cdn", "artifacts"].map((name, index) => ({
      candidate_id: `r2-m${index}`, name, marker: "missing" as const, facts: [{ label: "Jurisdiction", value: index % 3 === 2 ? "EU" : "Default" }], cautions: [],
    })),
  ] : [
    { candidate_id: "dbx-1", name: "Mallard otter", identity: "ns 1234567890", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Access", value: "Owner" }], cautions: [] },
    { candidate_id: "dbx-2", name: "Mallard team notes", identity: "ns 2233445566", marker: "storage", marker_detail: "layout 1", facts: [{ label: "Access", value: "Viewer" }], cautions: ["Not in your Dropbox", "Viewer — Push will be refused"] },
    { candidate_id: "dbx-3", name: "Legal archive", identity: "ns 9988776655", marker: "unreadable", marker_detail: "not mounted", facts: [{ label: "Access", value: "Viewer" }], cautions: ["Not in your Dropbox"] },
    { candidate_id: "dbx-4", name: "Acme team space", identity: "ns 5544332211", marker: "missing", facts: [{ label: "Team", value: "space" }, { label: "Access", value: "Editor" }], cautions: [] },
    { candidate_id: "dbx-5", name: "Family photos", identity: "ns 3344556677", marker: "missing", facts: [{ label: "Access", value: "Owner" }], cautions: [] },
    { candidate_id: "dbx-6", name: "Design handoff", identity: "ns 4455667788", marker: "missing", facts: [{ label: "Access", value: "Editor" }], cautions: [] },
    { candidate_id: "dbx-7", name: "Podcast raw", identity: "ns 6677889900", marker: "missing", facts: [{ label: "Access", value: "Editor" }], cautions: ["Not in your Dropbox"] },
  ];
  const candidates = outcome === "empty"
    ? rows.filter((row) => row.marker === "missing")
    : outcome === "partial" ? rows : rows.filter((row) => row.marker !== "unreadable");
  const unreadable = candidates.filter((row) => row.marker === "unreadable");
  return {
    scan_id: "scan-1",
    scope: r2 ? "He Qu · 023e105f…72d0c353 · Default, EU and FedRAMP jurisdictions" : "quhe1996@gmail.com · shared folders and team space",
    status: outcome === "partial" ? "partial" : "complete",
    issues: unreadable.map((row) => `1 ${provider.discovery!.unit} could not be read (${row.name}: ${row.marker_detail}).`),
    candidates,
  };
}

const FAILURE: Record<string, string> = {
  r2: "Wrangler is not signed in on this Mac. Sign in with wrangler login in a terminal, then try again.",
  dropbox: "Dropbox access was not approved — the browser window closed before consent.",
};

function preparedForm(provider: StorageProviderDescriptor, name: string): { form: StorageEditorForm; presence: SecretPresence } {
  if (provider.key === "r2") {
    return {
      form: {
        name,
        settings: {
          api_url: `https://023e105f4ecef8ad9ca31a8372d0c353.r2.cloudflarestorage.com/${name}`,
          bucket: name,
          account_id: "023e105f4ecef8ad9ca31a8372d0c353",
          access_key_id: "4a2b8c1d0e9f7a6b5c4d3e2f1a0b91ef",
        },
        typedSecrets: { secret_access_key: "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0" },
        persistedSecrets: { secret_access_key: "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0" },
      },
      presence: { secret_access_key: true },
    };
  }
  return {
    form: {
      name,
      settings: { namespace_id: "1234567890" },
      typedSecrets: { access_token: "sl.token", refresh_token: "rt.token" },
      persistedSecrets: { access_token: "sl.token", refresh_token: "rt.token" },
    },
    presence: { access_token: true, refresh_token: true },
  };
}

/** The workspace's storage page, reduced to the state this feature reads. */
function Page({ provider, outcome }: { provider: StorageProviderDescriptor; outcome: Outcome }) {
  const discovery = provider.discovery!;
  const [form, setForm] = useState<StorageEditorForm>({ name: `${provider.default_name} draft 1`, settings: {}, typedSecrets: {} });
  const [presence, setPresence] = useState<SecretPresence>({});
  const [mode, setMode] = useState<StorageSetupMode | null>(null);
  const [manualEntry, setManualEntry] = useState(false);
  const [state, setState] = useState<DiscoveryState>({ kind: "idle" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [ready, setReady] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (state.kind === "scanning") {
      const timer = setTimeout(() => {
        setState(outcome === "failed" ? { kind: "error", message: FAILURE[provider.key] } : { kind: "result", result: scan(provider, outcome) });
      }, 1500);
      return () => clearTimeout(timer);
    }
    if (state.kind === "preparing") {
      const timer = setTimeout(() => {
        const prepared = preparedForm(provider, state.candidate.name);
        setForm(prepared.form);
        setPresence(prepared.presence);
        setMode(null);
        setManualEntry(false);
        setSelectedId(null);
        setState({ kind: "idle" });
        setReady(`Using existing ${discovery.unit} ${state.candidate.name}. Review the connection, optionally test it, then select Finish setup.`);
        setRunning(false);
      }, 1700);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [state, provider, outcome, discovery.unit]);

  const progress = fieldsSet(provider, form, presence);
  const asking = !!provider.discovery && (mode !== null || progress.set === 0);
  const usingExistingVerbs = asking && mode === "existing" && !manualEntry;
  const selected = state.kind === "result"
    ? state.result.candidates.find((candidate) => candidate.candidate_id === selectedId && candidateSelectable(candidate)) ?? null
    : null;
  const choose = (next: StorageSetupMode) => {
    if (next === mode) return;
    setMode(next);
    setManualEntry(false);
    setState({ kind: "idle" });
    setSelectedId(null);
  };
  const setupSlot = ready ? (
    <span className="storage-setup-guide-connected" role="status"><Icon name="check-circle" size={13} /> {ready}</span>
  ) : (
    <div className="storage-setup-agent-split">
      <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta">Run with Claude Code</button>
      <button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta storage-setup-agent-split-arrow" aria-label="More agent setup routes"><Icon name="chevron-down" size={11} /></button>
    </div>
  );

  return (
    <>
      <div className="sync-storage-details-panel">
        <StorageEditor
          descriptor={provider}
          providers={[R2, LOCAL, DROPBOX]}
          form={form}
          secretPresence={presence}
          disabled={running}
          onChange={(next) => { setForm(next); setReady(null); }}
          onSwitchProvider={() => undefined}
          setupMode={asking ? { mode, onChoose: choose } : undefined}
          manualEntry={manualEntry}
          discoverySlot={asking && mode === "existing" ? (
            <StorageDiscoveryPanel
              descriptor={provider}
              discovery={discovery}
              state={state}
              selectedId={selectedId}
              storages={[{ storage_id: "storage-1", name: "R2 storage", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [], settings: {}, secrets: [] }]}
              manualEntry={manualEntry}
              disabled={running}
              onScan={() => { setSelectedId(null); setState({ kind: "scanning" }); }}
              onCancelScan={() => setState({ kind: "idle" })}
              onSelect={setSelectedId}
              onOpenStorage={() => undefined}
              onToggleManual={() => setManualEntry((open) => !open)}
              onCreateInstead={() => choose("create")}
            />
          ) : undefined}
          setupSlot={setupSlot}
        />
      </div>
      <footer className="sync-storage-setup-footer">
        <span className="sync-storage-editor-progress" aria-live="polite">{progress.set} of {progress.total} set</span>
        <div>
          <button type="button" className="btn btn-ghost">Discard draft…</button>
          <button type="button" className="btn btn-ghost">Cancel</button>
          <button type="button" className="btn" disabled={running || progress.set < progress.total}>Test connection</button>
          {usingExistingVerbs ? (
            <button
              type="button"
              className="btn btn-primary"
              disabled={running || !selected}
              onClick={() => {
                if (!selected || state.kind !== "result") return;
                setRunning(true);
                setState({ kind: "preparing", candidate: selected, result: state.result });
              }}
            >
              {running ? "Preparing…" : selected ? `Use ${selected.name}` : "Use selected"}
            </button>
          ) : (
            <button type="button" className="btn btn-primary" disabled={running || progress.set < progress.total}>Finish setup</button>
          )}
        </div>
      </footer>
    </>
  );
}

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [provider, setProvider] = useState<"r2" | "dropbox">("r2");
  const [outcome, setOutcome] = useState<Outcome>("complete");
  const descriptor = provider === "r2" ? R2 : DROPBOX;
  const segmented = <T extends string>(value: T, options: [T, string][], onChange: (next: T) => void) => (
    <div className="sync-segmented" role="radiogroup">
      {options.map(([key, text]) => (
        <button key={key} type="button" role="radio" aria-checked={value === key} className={value === key ? "active" : undefined} onClick={() => onChange(key)}>{text}</button>
      ))}
    </div>
  );
  return (
    <div className="app live-root">
      <div className="live-shell">
        <header className="live-head">
          <h1>Use existing storage — shipped components</h1>
          {segmented(provider, [["r2", "Cloudflare R2"], ["dropbox", "Dropbox"]], setProvider)}
          {segmented(outcome, [["complete", "Complete"], ["partial", "Partial"], ["empty", "Nothing found"], ["failed", "Can't start"]], setOutcome)}
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
        <main className="sync-main sync-project-links-page sync-storage-settings-page live-main">
          <section className="profile-links-section" aria-label="New storage">
            <Page key={`${provider}-${outcome}`} provider={descriptor} outcome={outcome} />
          </section>
        </main>
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
body { background: var(--app-shell); }
.app.live-root { height: auto; min-height: 100vh; overflow: visible; }
.live-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 24px 24px 80px; }
.live-head { display: flex; align-items: center; gap: 16px; margin-bottom: 18px; }
.live-head h1 { margin: 0 auto 0 0; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.live-main { display: block; height: auto; overflow: visible; }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
