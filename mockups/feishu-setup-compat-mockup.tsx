import { StrictMode, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { R2SetupLane, idleR2State, type R2SetupFlow, type R2SetupState } from "../src/components/project-sync/R2Setup";
import { DropboxSetupLane, type useDropboxSetup } from "../src/components/project-sync/DropboxSetup";
import { FeishuSetupLane, type FeishuLaneMode, type useFeishuSetup } from "../src/components/project-sync/FeishuSetup";
import { StorageEditor, type StorageEditorForm, type StorageSetupMode } from "../src/components/project-sync/StorageSettings";
import type { StorageProviderDescriptor } from "../src/types";
import "../src/App.css";

/**
 * The New-storage page of R2, Dropbox and Feishu side by side, each drawn
 * with the real `StorageEditor` and the real lane on a frozen scene, so the
 * three can be compared as they ship. `npm run dev`, then
 * /mockups/feishu-setup-compat-mockup.html (`?theme=dark`,
 * `?scene=open|start|browser|folder|existingStart|existing|ready|existingReady`,
 * `?provider=r2|dropbox|feishu`).
 */

type Scene = "open" | "start" | "browser" | "folder" | "existingStart" | "existing" | "ready" | "existingReady";
const SCENES: [Scene, string][] = [
  ["open", "1 · A fresh draft"], ["start", "2 · Create new, before start"], ["browser", "3 · Browser sign-in"],
  ["folder", "4 · Naming a new folder or bucket"], ["existingStart", "5 · Use existing, before start"], ["existing", "6 · Use existing"],
  ["ready", "7 · Ready for review · new"], ["existingReady", "8 · Ready for review · existing"],
];

/* ——— descriptors, as the backend sends them ——————————————————— */

const GUIDE = "https://example.test/feishu-app-guide";
const R2: StorageProviderDescriptor = {
  key: "r2", config_version: 1, display_name: "Cloudflare R2", description: "An R2 bucket reached through its S3 API.",
  default_name: "R2 storage", display_rank: 10, icon: "cloud", clear_mode: "unconditional",
  fields: [
    { field_id: "api_url", label: "S3 API URL", kind: "url", required: true, placeholder: "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent" },
    { field_id: "bucket", label: "Bucket name", kind: "text", required: true, placeholder: "Example: codex-sync-backups" },
    { field_id: "account_id", label: "Account ID", kind: "text", required: true, placeholder: "Example: 023e105f4ecef8ad9ca31a8372d0c353" },
    { field_id: "access_key_id", label: "Access Key ID", kind: "text", required: true, placeholder: "Example: 4a2b…91ef" },
  ],
  secrets: [{ secret_id: "secret_access_key", label: "Secret Access Key", required: true, placeholder: "Example: a1B2…x9Y0" }],
  managed_secret_id: null,
  local_setup: "wrangler",
  setup_guide_url: "https://developers.cloudflare.com/r2/",
  manual_setup_hint: "Enter an existing R2 connection in the fields below.",
  setup_doors: {
    create_hint: "A new bucket in your Cloudflare account.",
    existing_hint: "A bucket Mallard already uses.",
    existing_help: { label: "Set up team access", url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/" },
  },
  discovery: { unit: "bucket", unit_plural: "buckets", place: "your Cloudflare account", find_label: "Find in my account", access_hint: "", scanning_hint: "", prepare_hint: "" },
};
const DROPBOX: StorageProviderDescriptor = {
  key: "dropbox", config_version: 1, display_name: "Dropbox",
  description: "A shared folder in your Dropbox, new or one you already use. Uploads up to 150 MiB per object.",
  default_name: "Dropbox storage", display_rank: 30, icon: "folder", clear_mode: "unconditional",
  fields: [{ field_id: "namespace_id", label: "Namespace ID", kind: "text", required: true, placeholder: "Shared or team namespace ID" }],
  secrets: [
    { secret_id: "access_token", label: "Access token", required: true, placeholder: "User-linked Dropbox access token" },
    { secret_id: "refresh_token", label: "Refresh token", required: true, placeholder: "Dropbox refresh token" },
  ],
  managed_secret_id: "refresh_token",
  local_setup: "dropbox",
  setup_guide_url: "https://www.dropbox.com/developers",
  manual_setup_hint: "Enter an existing shared or team namespace and a user-linked token. Mallard stores data in .mallard within that namespace.",
  setup_doors: { create_hint: "A new shared folder in your Dropbox.", existing_hint: "A shared folder Mallard already uses.", existing_help: null },
  discovery: { unit: "shared folder", unit_plural: "shared folders", place: "your Dropbox", find_label: "Find in my Dropbox", access_hint: "", scanning_hint: "", prepare_hint: "" },
};
const FEISHU: StorageProviderDescriptor = {
  key: "feishu", config_version: 1, display_name: "Feishu",
  description: "A folder in your Feishu Drive, through Mallard's Feishu app. Uploads up to 100 MiB per object.",
  default_name: "Feishu storage", display_rank: 25, icon: "cloud", clear_mode: "unavailable",
  fields: [{ field_id: "folder_token", label: "Folder token", kind: "text", required: true, placeholder: "The Drive folder's token",
    help: "The Drive folder this storage lives in. Fixed after setup; share it in Feishu with each user who connects.", help_url: GUIDE }],
  secrets: [],
  managed_secret_id: null,
  local_setup: "feishu",
  setup_guide_url: GUIDE,
  manual_setup_hint: null,
  setup_doors: { create_hint: "A new folder in your My Space.", existing_hint: "A folder shared with you in Feishu.", existing_help: null },
  discovery: null,
};
const PROVIDERS = [R2, FEISHU, DROPBOX];

/* ——— frozen flows ————————————————————————————————————————————— */

const noop = () => undefined;
const LINK = "https://accounts.example.test/authorize?client_id=cli_a1b2&state=x9f2";
const FOLDER_LINK = "https://example.feishu.cn/drive/folder/fldcnAbC123xyz";
const isExisting = (scene: Scene) => scene === "existingStart" || scene === "existing" || scene === "existingReady";
const isReady = (scene: Scene) => scene === "ready" || scene === "existingReady";
const doorOf = (scene: Scene): StorageSetupMode | null => scene === "open" ? null : isExisting(scene) ? "existing" : "create";

function r2Flow(scene: Scene): R2SetupFlow {
  const account = { id: "023e105f4ecef8ad9ca31a8372d0c353", name: "He Qu" };
  const wrangler = { version: "4.38.0", path: "/usr/local/bin/wrangler" };
  const byScene: Record<Scene, Partial<R2SetupState>> = {
    open: {}, start: {}, existingStart: {},
    browser: { phase: "login", running: true, link: { stage: "login", url: LINK }, wrangler },
    folder: { phase: "bucket", wrangler, accounts: [account], account, name: "mallard-fox" },
    existing: { phase: "find", running: true, wrangler, accounts: [account], account },
    ready: { phase: "review", account, bucket: "mallard-fox", name: "mallard-fox" },
    existingReady: { phase: "review", account, bucket: "mallard-fox", name: "mallard-fox" },
  };
  return {
    state: { ...idleR2State(), ...byScene[scene] }, isRunning: () => false, start: noop, changeLogin: noop, continueWith: noop, setJurisdiction: noop,
    setName: noop, suggestAnother: noop, createAndAuthorize: noop, useBucket: noop, cancel: async () => undefined, retry: noop, scanAgain: noop, dismiss: noop,
  };
}

type DropboxFlow = ReturnType<typeof useDropboxSetup>;
function dropboxFlow(scene: Scene): DropboxFlow {
  const account = { id: "dbid:1", name: "He Qu", email: "he@example.test" };
  const login = { session_id: "s1", account };
  const byScene: Record<Scene, Partial<DropboxFlow["state"]>> = {
    open: {}, start: {}, existingStart: {},
    browser: { phase: "login", running: true, link: LINK },
    folder: { phase: "folder", login, account, name: "Mallard fox" },
    existing: { phase: "find", running: true, login, account },
    ready: { phase: "review", account, name: "Mallard fox" },
    existingReady: { phase: "review", account, name: "Mallard fox" },
  };
  return {
    state: { phase: "idle", running: false, cancelling: false, error: null, link: null, login: null, account: null, name: "", scan: null, chosen: null, ...byScene[scene] },
    isRunning: () => false, start: noop, confirmAccount: noop, scan: noop, prepare: noop, cancel: async () => undefined, dismiss: noop, suggest: noop, setName: noop,
  };
}

type FeishuFlow = ReturnType<typeof useFeishuSetup>;
function feishuFlow(scene: Scene): FeishuFlow {
  const account = { id: "ou_fixture", name: "He Qu" };
  const login = { session_id: "s1", account };
  const byScene: Record<Scene, Partial<FeishuFlow["state"]>> = {
    open: {}, start: {}, existingStart: {},
    browser: { phase: "login", running: true, link: LINK },
    folder: { phase: "folder", login, account, name: "Mallard fox" },
    existing: { phase: "folder", login, account, link_: FOLDER_LINK },
    ready: { phase: "review", account, name: "Mallard fox" },
    existingReady: { phase: "review", account, link_: FOLDER_LINK },
  };
  return {
    state: { phase: "idle", running: false, cancelling: false, error: null, diagnostics: null, link: null, login: null, account: null, name: "", link_: "", ...byScene[scene] },
    isRunning: () => false, start: noop, confirmAccount: noop, prepare: noop, cancel: async () => undefined, dismiss: noop, suggest: noop,
    setName: noop, setLink: noop,
  };
}

/* ——— the page ————————————————————————————————————————————————— */

function settingsFor(descriptor: StorageProviderDescriptor, scene: Scene): Record<string, string> {
  if (!isReady(scene)) return {};
  if (descriptor.key === "r2") return { api_url: "https://023e105f4ecef8ad9ca31a8372d0c353.r2.cloudflarestorage.com", bucket: "mallard-fox", account_id: "023e105f4ecef8ad9ca31a8372d0c353", access_key_id: "4a2b8c1d91ef" };
  if (descriptor.key === "dropbox") return { namespace_id: "1234567890" };
  return { folder_token: "fldcnAbC123xyz" };
}

function Page({ descriptor, scene }: { descriptor: StorageProviderDescriptor; scene: Scene }) {
  const [form, setForm] = useState<StorageEditorForm>({
    name: isReady(scene) ? (descriptor.key === "r2" ? "R2 mallard-fox" : "Mallard fox") : descriptor.default_name ?? "",
    settings: settingsFor(descriptor, scene),
    typedSecrets: {},
  });
  const secrets = Object.fromEntries(descriptor.secrets.map((secret) => [secret.secret_id, isReady(scene)]));
  const door = doorOf(scene);
  const mode: FeishuLaneMode = door === "existing" ? "existing" : "create";
  const setupMode = descriptor.setup_doors && !isReady(scene) ? { mode: door, onChoose: noop } : undefined;
  const lane: ReactNode = descriptor.key === "r2"
    ? <R2SetupLane mode={mode} flow={r2Flow(scene)} disabled={false} descriptor={descriptor} storages={[]} onOpenStorage={noop} onCreateInstead={noop} onReset={noop} />
    : descriptor.key === "dropbox"
      ? <DropboxSetupLane flow={dropboxFlow(scene)} mode={mode} disabled={false} descriptor={descriptor} storages={[]} onOpenStorage={noop} onCreateInstead={noop} onNewSetup={noop} onReset={noop} />
      : <FeishuSetupLane flow={feishuFlow(scene)} mode={mode} disabled={false} descriptor={descriptor} onReset={noop} />;
  return (
    <div className="sync-storage-settings-below sync-storage-settings-dedicated">
      <div className="sync-storage-details-panel">
        <StorageEditor
          descriptor={descriptor}
          providers={PROVIDERS}
          form={form}
          secretPresence={secrets}
          disabled={false}
          onChange={setForm}
          onSwitchProvider={noop}
          setupMode={setupMode}
          setupDone={isReady(scene)}
          setupSlot={lane}
        />
      </div>
      <footer className="sync-storage-setup-footer">
        <div>
          <button type="button" className="btn btn-ghost">Discard draft…</button>
          <button type="button" className="btn btn-ghost">Close</button>
          <button type="button" className="btn">Test connection</button>
          <button type="button" className="btn btn-primary">Finish setup</button>
        </div>
      </footer>
    </div>
  );
}

function Specimen() {
  const query = new URLSearchParams(window.location.search);
  const only = query.get("scene");
  const provider = query.get("provider");
  const scenes = only ? SCENES.filter(([id]) => id === only) : SCENES;
  const providers = [R2, DROPBOX, FEISHU].filter((descriptor) => !provider || descriptor.key === provider);
  return (
    <div className="compat-specimen">
      {scenes.map(([id, title]) => (
        <section key={id} className="compat-scene">
          <h2 className="compat-scene-title">{title}</h2>
          <div className="compat-row">
            {providers.map((descriptor) => (
              <div key={descriptor.key} className="compat-col">
                <h3 className="compat-col-title">{descriptor.display_name}</h3>
                <main className="sync-main sync-project-links-page sync-storage-settings-page">
                  <section className="profile-links-section">
                    <Page descriptor={descriptor} scene={id} />
                  </section>
                </main>
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

const theme = new URLSearchParams(window.location.search).get("theme");
document.documentElement.dataset.theme = theme === "dark" ? "dark" : "light";
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);
