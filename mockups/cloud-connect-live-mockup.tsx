import { StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { ManagedCloudRepository, ManagedCloudRepositoryResolution, StorageSummary } from "../src/types";
import Icon, { type IconName } from "../src/components/Icons";
import BundleConnectionDialog from "../src/components/project-sync/BundleConnectionDialog";
import RepositoryAddressPicker, {
  useManagedCloudRepositories,
  type RepositorySelection,
} from "../src/components/project-sync/RepositoryAddressPicker";
import { projectSyncApi } from "../src/components/project-sync/api";
import "../src/App.css";

/**
 * The shipped connect-by-address parts, live: the real `BundleConnectionDialog`
 * for a Mallard Cloud storage and the real `RepositoryAddressPicker` inside
 * the setup page's Repository band, against a stand-in for the two backend
 * reads. `npm run dev`, then /mockups/cloud-connect-live-mockup.html (`?theme=dark`,
 * `?view=dialog|setup`). Addresses that resolve: Rika321/HorrorGame (yours),
 * rikaqu0223-arch/tidewatch (member, other remote), rikaqu0223-arch/ledger-api
 * (public), anything on staging.mallard-ai.com (wrong service), a github.com
 * address, Rika321/skills-pack (catalog), anything else (not found).
 */

const CHECKOUT = "a".repeat(64);

const STORAGE: StorageSummary = {
  storage_id: "storage-cloud",
  name: "Mallard Cloud",
  provider: "managed_cloud",
  config_version: 1,
  provider_label: "Mallard Cloud",
  icon: "cloud",
  available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "Rika321", is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: { service_origin: "https://mallard-ai.com", account_id: "user_1", account_login: "Rika321" },
  secrets: [{ secret_id: "account_token", present: true }],
};

const now = Math.floor(Date.now() / 1000);
const repo = (overrides: Partial<ManagedCloudRepository>): ManagedCloudRepository => ({
  repository_id: "repo_1", description: null, star_count: 0, viewer_starred: false,
  owner_login: "Rika321",
  name: "HorrorGame",
  address: "Rika321/HorrorGame",
  website_url: "https://mallard-ai.com/Rika321/HorrorGame",
  bundle_id: "3f9a0c7e1b2d4a6f8c0e2b4d6f8a1c3e",
  repository_class: "git_project",
  visibility: "private",
  role: "owner",
  can_push: true,
  repository_fingerprint: CHECKOUT,
  git_remote: "matches",
  github_url: null,
  revision: 4,
  updated_at: now - 2 * 3600,
  resources: 3,
  registration_only: false,
  ...overrides,
});
const MINE: ManagedCloudRepository[] = [
  repo({}),
  repo({ repository_id: "repo_2", owner_login: "rikaqu0223-arch", name: "tidewatch", address: "rikaqu0223-arch/tidewatch", website_url: "https://mallard-ai.com/rikaqu0223-arch/tidewatch", bundle_id: "a81b3c5d7e9f1a2b4c6d8e0f2a4b6c8d", role: "member", repository_fingerprint: "b".repeat(64), git_remote: "differs", revision: 9, resources: 2, updated_at: now - 26 * 3600 }),
  repo({ repository_id: "repo_3", name: "Birding-Log", address: "Rika321/Birding-Log", website_url: "https://mallard-ai.com/Rika321/Birding-Log", bundle_id: "c02d4e6f8a1b3c5d7e9f0a2b4c6d8e1f", repository_class: "non_git_project", visibility: "public", repository_fingerprint: null, git_remote: "differs", revision: 2, resources: 2, updated_at: now - 3 * 86400 }),
];
const PUBLIC = repo({ repository_id: "repo_9", owner_login: "rikaqu0223-arch", name: "ledger-api", address: "rikaqu0223-arch/ledger-api", website_url: "https://mallard-ai.com/rikaqu0223-arch/ledger-api", bundle_id: "d13e5f7a9b1c3d5e7f9a1b3c5d7e9f1a", visibility: "public", role: null, repository_fingerprint: "e".repeat(64), git_remote: "differs", revision: 3, resources: 1, updated_at: now - 7 * 86400 });
const CATALOG = repo({ repository_id: "repo_5", name: "skills-pack", address: "Rika321/skills-pack", website_url: "https://mallard-ai.com/Rika321/skills-pack", bundle_id: null, repository_class: "catalog", repository_fingerprint: null, git_remote: "unknown" });

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

/** A stand-in for `resolve_managed_cloud_repository`, following its grammar. */
function resolve(input: string): ManagedCloudRepositoryResolution {
  let text = input.trim().split(/[?#]/)[0];
  let host: string | null = null;
  const scheme = text.match(/^([a-z]+):\/\/(.*)$/i);
  if (scheme) {
    if (!/^https?$/i.test(scheme[1])) return { kind: "invalid" };
    const slash = scheme[2].indexOf("/");
    host = (slash < 0 ? scheme[2] : scheme[2].slice(0, slash)).toLowerCase();
    text = slash < 0 ? "" : scheme[2].slice(slash + 1);
  } else {
    const first = text.split("/")[0];
    if (first.includes(".") || first.includes(":") || first.toLowerCase() === "localhost") {
      host = first.toLowerCase();
      text = text.slice(first.length + 1);
    }
  }
  if (host) {
    host = host.replace(/^www\./, "");
    if (host === "github.com") return { kind: "github" };
    if (host !== "mallard-ai.com") return { kind: "wrong_service", host, expected: "mallard-ai.com" };
  }
  const [owner, name] = text.split("/").filter(Boolean);
  if (!owner || !name || !/^[A-Za-z0-9_-]{1,80}$/.test(owner) || !/^[A-Za-z0-9._-]{1,100}$/.test(name)) return { kind: "invalid" };
  const address = `${owner}/${name}`.toLowerCase();
  const known = [...MINE, PUBLIC, CATALOG].find((candidate) => candidate.address.toLowerCase() === address);
  if (!known) return { kind: "not_found", address: `${owner}/${name}` };
  if (known.repository_class === "catalog") return { kind: "not_a_project", repository: known };
  return known.role === null ? { kind: "public", repository: known } : { kind: "collaborator", repository: known };
}

projectSyncApi.listManagedCloudRepositories = async () => { await wait(500); return MINE; };
projectSyncApi.resolveManagedCloudRepository = async (_storageId, address) => { await wait(700); return resolve(address); };

function Log({ lines }: { lines: string[] }) {
  return (
    <div className="mock-log" aria-live="polite">
      {lines.length === 0 ? <span className="mock-aside">Backend calls appear here.</span> : lines.map((line, index) => <code key={index}>{line}</code>)}
    </div>
  );
}

function DialogView({ log }: { log: (line: string) => void }) {
  const [instance, setInstance] = useState(0);
  const [busy, setBusy] = useState(false);
  return (
    <BundleConnectionDialog
      key={instance}
      projectName="HorrorGame"
      currentBundleId="7c1e0a5b9d3f4e2a8b6c1d0e9f8a7b6c"
      projectFingerprint={CHECKOUT}
      storage={STORAGE}
      matches={[]}
      reason="link"
      busy={busy}
      error={null}
      onCancel={() => { log("dialog closed"); setInstance((n) => n + 1); }}
      onUseExisting={(choice) => {
        log(`connect_project_to_remote_bundle bundle=${choice.bundleId.slice(0, 8)}… allow_repository_mismatch=${choice.allowRepositoryMismatch}`);
        setBusy(true);
        void wait(900).then(() => { setBusy(false); setInstance((n) => n + 1); });
      }}
      onKeepCurrent={() => {
        log("save_project_link (creates Rika321/HorrorGame-2 from the project name)");
        setBusy(true);
        void wait(900).then(() => { setBusy(false); setInstance((n) => n + 1); });
      }}
    />
  );
}

function SetupView({ log }: { log: (line: string) => void }) {
  const [selection, setSelection] = useState<RepositorySelection>({ kind: "new" });
  const [name, setName] = useState("HorrorGame");
  const listing = useManagedCloudRepositories(STORAGE.storage_id, CHECKOUT);
  const pill = (icon: IconName, label: string, active: boolean) => (
    <button type="button" role="radio" aria-checked={active} className={`setup-pill${active ? " active" : ""}`} title={label}>
      <Icon name={icon} size={12} />
      <span className="setup-pill-label">{label}</span>
      <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
    </button>
  );
  const blocked = selection.kind === "existing" && (selection.repository_fingerprint ?? null) !== CHECKOUT && !selection.mismatch_acknowledged;
  return (
    <section className="sync-inline-new-project sync-setup-workspace" aria-labelledby="setup-title">
      <header className="sync-modal-header">
        <div><h1 id="setup-title">Set up {name || "project"}</h1></div>
        <button type="button" className="btn btn-ghost" aria-label="Close setup; the draft is kept"><Icon name="x" size={17} /></button>
      </header>
      <div className="sync-modal-body">
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Project</strong></div></div>
          <div className="sync-field-grid sync-setup-project-fields">
            <label>
              <span>Name</span>
              <input value={name} disabled={selection.kind === "existing"} onChange={(event) => setName(event.target.value)} />
            </label>
            <label>
              <span>Folder</span>
              <div className="sync-profile-select-row">
                <input value="~/Desktop/project/HorrorGame" readOnly />
                <button type="button" className="btn"><Icon name="folder" size={12} /> Change…</button>
              </div>
            </label>
          </div>
          {selection.kind === "existing" && <small className="sync-setup-hint">The project name follows the connected remote repo.</small>}
        </section>
        <section className="sync-setup-section">
          <div className="sync-card-heading"><div><strong>Storage</strong></div></div>
          <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
            {pill("computer", "None yet", false)}
            {pill("cloud", STORAGE.name, true)}
            {pill("drive", "Team R2", false)}
          </div>
        </section>
        <section className="sync-setup-section repo-connect">
          <div className="sync-card-heading">
            <div><strong>Repository</strong></div>
            {blocked && <span className="sync-setup-state blocked" aria-label="Action required" title="Action required"><Icon name="alert-triangle" size={14} /></span>}
          </div>
          {blocked && (
            <div className="sync-setup-field-error"><Icon name="alert-triangle" size={13} /> The remote repo is not verified to match this folder's Git remote; acknowledge this to continue.</div>
          )}
          <RepositoryAddressPicker
            storage={STORAGE}
            projectName={name || "project"}
            localFingerprint={CHECKOUT}
            listing={listing}
            selection={selection}
            rows="match-option"
            offerNew
            disabled={false}
            onSelect={(next) => {
              setSelection(next ?? { kind: "new" });
              log(!next || next.kind === "new" ? "draft.repository = new" : `draft.repository = existing ${next.display_name} (acknowledged=${next.mismatch_acknowledged})`);
            }}
          />
        </section>
      </div>
      <footer className="sync-modal-footer sync-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost">Discard</button>
          <button type="button" className="btn btn-primary" disabled={blocked || listing.state === "loading"} onClick={() => log("finalize_project_setup")}>Finish</button>
        </div>
      </footer>
    </section>
  );
}

const PARAMS = new URLSearchParams(location.search);

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [view, setView] = useState<"dialog" | "setup">(() => (PARAMS.get("view") === "setup" ? "setup" : "dialog"));
  const [lines, setLines] = useState<string[]>([]);
  const log = (line: string) => setLines((current) => [...current.slice(-5), line]);
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Mallard Cloud · connect by address, live</h1>
            <p>
              The shipped components against a stand-in backend. Try <code>https://mallard-ai.com/Rika321/HorrorGame</code>,{" "}
              <code>rikaqu0223-arch/ledger-api</code> (public), <code>rikaqu0223-arch/tidewatch</code> (other remote),{" "}
              <code>https://staging.mallard-ai.com/Rika321/HorrorGame</code>, a github.com address, or a name that does not exist.
            </p>
          </div>
          <button type="button" className="btn btn-ghost" onClick={() => { const next = !dark; setDark(next); document.documentElement.setAttribute("data-theme", next ? "dark" : "light"); }}>
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>
        <div className="mock-controls">
          <div className="mock-toggle">
            <span className="mock-aside">View</span>
            <div className="sync-segmented" role="radiogroup" aria-label="View">
              <button type="button" role="radio" aria-checked={view === "dialog"} className={view === "dialog" ? "active" : undefined} onClick={() => setView("dialog")}>Connect dialog</button>
              <button type="button" role="radio" aria-checked={view === "setup"} className={view === "setup" ? "active" : undefined} onClick={() => setView("setup")}>Setup band</button>
            </div>
          </div>
          <Log lines={lines} />
        </div>
        <section className="mockup-panel">
          <div className="mockup-stage">
            <div className="mock-body">
              {view === "dialog" ? <DialogView log={log} /> : <SetupView log={log} />}
            </div>
          </div>
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
.mockup-head p { max-width: 90ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-head code { font-size: var(--type-sm); }
.mockup-panel { margin-top: 18px; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mock-body { display: flex; flex-direction: column; gap: 14px; padding: 16px 18px 18px; min-height: 420px; }
.mock-aside { margin: 0; color: var(--text-3); font-size: var(--type-xs); line-height: 1.5; }
.mock-controls { position: sticky; top: 0; z-index: 4; display: flex; flex-wrap: wrap; align-items: center; gap: 10px 22px; margin: 18px 0 0; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-card); box-shadow: var(--shadow-card); }
.mock-toggle { display: flex; align-items: center; gap: 10px; }
.mock-log { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.mock-log code { color: var(--text-1); font-size: var(--type-2xs); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
/* The modal dialog is laid out in flow inside the stage so the page keeps scrolling. */
.sync-bundle-connection-dialog { position: static; max-height: none; margin: 0 auto; }
.sync-bundle-connection-dialog::backdrop { background: transparent; }
`;

document.documentElement.setAttribute("data-theme", PARAMS.get("theme") === "dark" ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS;
document.head.appendChild(style);
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
