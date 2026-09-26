import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { FeishuSetupLane, feishuSetupDiagnostics, type FeishuLaneMode, type useFeishuSetup } from "./components/project-sync/FeishuSetup";
import type { StorageProviderDescriptor } from "./types";
import "./App.css";

/**
 * The Feishu setup lane as it ships, one frozen scene per panel, drawn with
 * the real `FeishuSetupLane` so the specimen and the app cannot drift.
 * `npm run dev`, then /feishu-lane-mockup.html (`?theme=dark`,
 * `?scene=idle|browser|account|naming|existing|refused|connected|ready|reconnect`).
 */

type Flow = ReturnType<typeof useFeishuSetup>;
type Scene = { id: string; title: string; mode: FeishuLaneMode; state: Partial<Flow["state"]> };

const DESCRIPTOR: StorageProviderDescriptor = {
  key: "feishu", config_version: 1, display_name: "Feishu", description: "A folder in your Feishu Drive",
  display_rank: 25, icon: "cloud", clear_mode: "unavailable", default_name: "Feishu storage",
  fields: [{ field_id: "folder_token", label: "Folder token", kind: "text", required: true }],
  secrets: [],
  local_setup: "feishu",
  setup_guide_url: "https://example.test/feishu-app-guide",
};
const LOGIN = { session_id: "session-1", account: { id: "ou_fixture", name: "He Qu" } };
const LINK = "https://accounts.feishu.cn/open-apis/authen/v1/authorize?client_id=cli_a1b2c3d4e5&response_type=code&redirect_uri=http%3A%2F%2F127.0.0.1%3A53683%2Ffeishu%2Fcallback&state=x9f2";

const SCENES: Scene[] = [
  { id: "idle", title: "Before start", mode: "create", state: { phase: "idle" } },
  { id: "browser", title: "Browser consent", mode: "create", state: { phase: "login", running: true, link: LINK } },
  { id: "account", title: "Confirm account", mode: "create", state: { phase: "account", login: LOGIN, account: LOGIN.account } },
  { id: "naming", title: "Folder · new", mode: "create", state: { phase: "folder", login: LOGIN, account: LOGIN.account, name: "Mallard fox" } },
  { id: "existing", title: "Folder · existing link", mode: "existing", state: { phase: "folder", login: LOGIN, account: LOGIN.account, link_: "https://example.feishu.cn/drive/folder/fldcnAbC123" } },
  { id: "refused", title: "Mallard’s app was refused", mode: "create", state: { phase: "login", error: "feishu_app_rejected" } },
  { id: "diagnostics", title: "Sign-in diagnostics (fixture)", mode: "create", state: { phase: "login", error: "feishu_reconnect_required", diagnostics: feishuSetupDiagnostics({ provider_code: 20049, evidence: { endpoint: "oauth/v3/token", http_status: 400, error_tag: "invalid_grant", request_id: "req-fixture" } }) } },
  { id: "connected", title: "Folder already connected", mode: "existing", state: { phase: "prepare", error: "feishu_already_connected", login: LOGIN, account: LOGIN.account, link_: "fldcnAbC123" } },
  { id: "ready", title: "Ready for review", mode: "create", state: { phase: "review", account: LOGIN.account, name: "Mallard fox" } },
  { id: "reconnect", title: "Reconnect · renewing", mode: "reconnect", state: { phase: "prepare", running: true, login: LOGIN, account: LOGIN.account } },
];

const noop = () => undefined;
function flow(state: Partial<Flow["state"]>): Flow {
  return {
    state: { phase: "idle", running: false, cancelling: false, error: null, diagnostics: null, link: null, login: null, account: null, name: "", link_: "", ...state },
    isRunning: () => false, start: noop, confirmAccount: noop, prepare: noop, cancel: async () => undefined, dismiss: noop, suggest: noop,
    setName: noop, setLink: noop,
  };
}

function Specimen() {
  const query = new URLSearchParams(window.location.search);
  const only = query.get("scene");
  const scenes = only ? SCENES.filter((scene) => scene.id === only) : SCENES;
  return (
    <main className="sync-main sync-storage-settings-page" style={{ padding: 24, display: "grid", gap: 24 }}>
      {scenes.map((scene) => (
        <section key={scene.id} className="sync-storage-card" aria-label={scene.title}>
          <h2 style={{ margin: "0 0 8px", fontSize: "var(--type-sm)" }}>{scene.title}</h2>
          <div className="storage-setup-route-row agent r2-lane" role="note">
            <FeishuSetupLane flow={flow(scene.state)} mode={scene.mode} descriptor={DESCRIPTOR} disabled={scene.state.running ?? false} onReset={noop} />
          </div>
        </section>
      ))}
    </main>
  );
}

const theme = new URLSearchParams(window.location.search).get("theme");
if (theme === "dark") document.documentElement.dataset.theme = "dark";
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);
