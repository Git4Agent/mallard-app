import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { StorageLockedForm, StorageReconnectBand, formFromDraft, secretPresenceOf } from "./components/project-sync/StorageSettings";
import ManagedCloudAddress from "./components/managed-cloud/ManagedCloudAddress";
import { isManagedCloudStorage, managedCloudAccount } from "./components/project-sync/RepositoryAddressPicker";
import type { StorageConnectionHealth, StorageProviderDescriptor, StorageSetupDraft, StorageSummary } from "./types";
import "./App.css";

/* A dev harness: the settled storage's page as the real components draw it,
   with a stub backend. Open /storage-locked-live.html on the Vite server. */

const R2: StorageProviderDescriptor = {
  key: "r2", config_version: 1, display_name: "Cloudflare R2", description: "", display_rank: 10, icon: "cloud",
  clear_mode: "unconditional", local_setup: "wrangler", setup_guide_url: "https://developers.cloudflare.com/r2/",
  fields: [
    { field_id: "api_url", label: "S3 API URL", kind: "url", required: true },
    { field_id: "bucket", label: "Bucket name", kind: "text", required: true },
    { field_id: "account_id", label: "Account ID", kind: "text", required: true },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true },
  ],
  secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, help: "Cloudflare shows this once, when the token is created. Mallard keeps it on this machine." }],
  managed_secret_id: null,
};
const DROPBOX: StorageProviderDescriptor = {
  key: "dropbox", config_version: 1, display_name: "Dropbox", description: "", display_rank: 30, icon: "cloud",
  clear_mode: "revision_checked", local_setup: "dropbox", setup_guide_url: "https://www.dropbox.com/developers",
  fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, help: "Use the same namespace ID on each machine. This value cannot change after setup." }],
  secrets: [
    { secret_id: "access_token", label: "Access token", required: true, help: "Short-lived; renewed automatically from the refresh token." },
    { secret_id: "refresh_token", label: "Refresh token", required: true, help: "Issued to this Mallard app and Dropbox account at sign-in." },
  ],
  managed_secret_id: "refresh_token",
};
const LOCAL: StorageProviderDescriptor = {
  key: "local", config_version: 1, display_name: "Local folder", description: "", display_rank: 20, icon: "drive",
  clear_mode: "revision_checked",
  fields: [{ field_id: "directory", label: "Location path", kind: "directory", required: true }],
  secrets: [], managed_secret_id: null,
};
const MALLARD: StorageProviderDescriptor = {
  key: "managed_cloud", config_version: 1, display_name: "Mallard Cloud", description: "", display_rank: 5, icon: "cloud",
  clear_mode: "unavailable", local_setup: "managed_cloud", fields: [],
  secrets: [{ secret_id: "account_token", label: "Sign-in token", required: true, help: "Issued at sign-in and valid 90 days. Reconnect to renew it." }],
  managed_secret_id: "account_token",
};
const mallardSummary = (connection: StorageConnectionHealth | null = null): StorageSummary => ({
  storage_id: connection ? "storage-mallard-refused" : "storage-mallard", name: "Mallard Cloud octocat", provider: "managed_cloud",
  config_version: 1, provider_label: "Mallard Cloud", icon: "cloud", available: true,
  summary: [
    { field_id: "account_login", label: "Account", value: "octocat", is_path: false },
    { field_id: "service_origin", label: "Service", value: "https://mallard-ai.com", is_path: false },
  ],
  settings: { service_origin: "https://mallard-ai.com", account_id: "user_1", account_login: "octocat" },
  secrets: [{ secret_id: "account_token", present: true }], connection,
});
const refusal: StorageConnectionHealth = {
  state: "reconnect_required", code: "r2_access_denied", observed_at: 1,
  message: "Cloudflare R2 rejected the stored access key (403 · InvalidAccessKeyId).",
};
const storages: { storage: StorageSummary; descriptor: StorageProviderDescriptor; verified?: number | null }[] = [
  { descriptor: MALLARD, verified: Date.now() - 120_000, storage: mallardSummary() },
  { descriptor: MALLARD, storage: mallardSummary({
    state: "token_expired", code: "managed_cloud_reconnect_required", observed_at: 1,
    message: "Mallard Cloud rejected the saved sign-in. Reconnect to sign in again.",
  }) },
  { descriptor: R2, verified: Date.now() - 180_000, storage: {
    storage_id: "storage-r2", name: "mallard-quick", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [],
    settings: { api_url: "https://023e105f4ecef8ad9ca31a8372d0c353.r2.cloudflarestorage.com", bucket: "mallard-quick", account_id: "023e105f4ecef8ad9ca31a8372d0c353", access_key_id: "4a2b9c1d7e8f0a3b5c6d7e8f9a0b1c2d" },
    secrets: [{ secret_id: "secret_access_key", present: true }],
  } },
  { descriptor: DROPBOX, storage: {
    storage_id: "storage-dbx", name: "Team Dropbox", provider: "dropbox", config_version: 1, provider_label: "Dropbox", icon: "cloud", available: true, summary: [],
    settings: { namespace_id: "ns:4823017" },
    secrets: [{ secret_id: "access_token", present: true }, { secret_id: "refresh_token", present: true }],
  } },
  { descriptor: R2, storage: {
    storage_id: "storage-r2-refused", name: "mallard-quick", provider: "r2", config_version: 1, provider_label: "Cloudflare R2", icon: "cloud", available: true, summary: [],
    settings: { api_url: "https://023e105f4ecef8ad9ca31a8372d0c353.r2.cloudflarestorage.com", bucket: "mallard-quick", account_id: "023e105f4ecef8ad9ca31a8372d0c353", access_key_id: "4a2b9c1d7e8f0a3b5c6d7e8f9a0b1c2d" },
    secrets: [{ secret_id: "secret_access_key", present: true }], connection: refusal,
  } },
  { descriptor: LOCAL, storage: {
    storage_id: "storage-local", name: "Backup disk", provider: "local", config_version: 1, provider_label: "Local folder", icon: "drive", available: true, summary: [],
    settings: { directory: "/Volumes/backup/mallard-storage" }, secrets: [],
  } },
];
const VALUES: Record<string, string> = {
  account_token: "desktop-session-demo-0123456789abcdef",
  secret_access_key: "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8s9T0u1V2w3X4",
  access_token: "sl.u.AFyQ3nR8kZt0b2LxPq9WvJm7Hc4YeTd1oSaG6NiUf5XrBwK2lM8pDzE0vC3hQ7yJ",
  refresh_token: "rt_9XkQ2mPz7LbT4vWn8HcJ3sYd6FgR1aE5uKoI0NxZ",
};
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function Card({ storage, descriptor, verified = null }: { storage: StorageSummary; descriptor: StorageProviderDescriptor; verified?: number | null }) {
  const [name, setName] = useState(storage.name);
  const [testing, setTesting] = useState(false);
  const [verifiedAt, setVerifiedAt] = useState<number | null>(verified);
  const [reconnecting, setReconnecting] = useState(false);
  const draft: StorageSetupDraft = {
    draft_id: `draft-${storage.storage_id}`, storage_id: storage.storage_id, name: storage.name, provider: storage.provider,
    config_version: 1, provider_label: storage.provider_label, available: true, settings: storage.settings,
    secrets: storage.secrets.map((secret) => ({ ...secret })), edits_existing: true, revision: 1, created_at: 1, updated_at: 1,
  };
  const [form, setForm] = useState(() => formFromDraft(descriptor, draft));
  return (
    <div className="sync-storage-details-panel" style={{ marginBottom: 18 }}>
      <StorageLockedForm
        storage={{ ...storage, name }}
        descriptor={descriptor}
        disabled={false}
        onRename={async (next) => { await wait(350); if (/taken/i.test(next)) return false; setName(next); return true; }}
        onRevealSecret={async (secretId) => { await wait(280); if (secretId === "refresh_token" && (window as { failReveal?: boolean }).failReveal) throw new Error("Couldn’t read the saved value"); return VALUES[secretId]; }}
        onTest={() => { setTesting(true); setVerifiedAt(null); void wait(900).then(() => { setTesting(false); setVerifiedAt(Date.now()); }); }}
        testing={testing}
        verifiedAt={verifiedAt}
        reconnect={{
          open: reconnecting,
          onOpen: () => setReconnecting(true),
          band: (
            <StorageReconnectBand
              descriptor={descriptor}
              form={form}
              secretPresence={secretPresenceOf(draft)}
              disabled={false}
              connection={storage.connection}
              onChange={setForm}
              setupSlot={<button type="button" className="btn storage-setup-guide-button storage-setup-agent-cta">Reconnect with Wrangler</button>}
              verbs={(
                <>
                  <button type="button" className="btn btn-ghost" onClick={() => setReconnecting(false)}>Cancel</button>
                  <button type="button" className="btn" disabled={!form.typedSecrets.secret_access_key}>Test connection</button>
                  <button type="button" className="btn btn-primary" disabled={!form.typedSecrets.secret_access_key}>Save credential</button>
                </>
              )}
            />
          ),
        }}
        lead={isManagedCloudStorage(storage) ? {
          band: <ManagedCloudAddress account={managedCloudAccount(storage)} onOpenUrl={(url) => window.open(url, "_blank")} />,
          facts: ["account_login", "service_origin"],
        } : undefined}
      />
    </div>
  );
}

function App() {
  return (
    <div style={{ padding: 20, maxWidth: 820 }}>
      {storages.map((entry) => <Card key={entry.storage.storage_id} {...entry} />)}
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<StrictMode><App /></StrictMode>);
