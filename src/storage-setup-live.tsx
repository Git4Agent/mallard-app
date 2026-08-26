import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import { StorageEditor, newStorage, r2FieldsSet, storageConfigReady } from "./components/project-sync/StorageSettings";
import type { StorageConfig } from "./types";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shipped storage setup page — real SyncPathBar, real StorageEditor, real
 * App.css — with its commit bar under the form instead of in the trail. Not
 * part of the app bundle: the build has one entry, and this file is reachable
 * only from the dev server.
 */

const noop = () => {};

const appActions: PathBarMenuGroup[] = [
  { items: [{ id: "logs", label: "Show logs", icon: "terminal", onSelect: noop }] },
];

const crumbs: PathBarCrumb[] = [
  {
    id: "storages",
    label: "Storages",
    icon: "drive",
    tone: "root",
    title: "All storages",
    ariaLabel: "All storages",
    onActivate: noop,
  },
  { id: "storage", label: "New storage" },
];

function Harness() {
  const [dark, setDark] = useState(false);
  const [draft, setDraft] = useState<StorageConfig>(() => newStorage("local", 2));
  const [secretStored, setSecretStored] = useState(false);
  const completionTimer = useRef<number | null>(null);
  const ready = storageConfigReady(draft, secretStored);
  // The workspace's agent lifecycle, answered locally: copy flips the row to
  // listening, and a fake exact-draft load fills the same editable values that
  // the production watcher-triggered exact get adopts seconds later.
  const [agentPhase, setAgentPhase] = useState<"offer" | "launched" | "listening" | "ready">("offer");
  const [agentName, setAgentName] = useState("Claude Code");
  const [agentMenuOpen, setAgentMenuOpen] = useState(false);
  const [copyAgainFlashed, setCopyAgainFlashed] = useState(false);
  const cancelFakeCompletion = () => {
    if (completionTimer.current === null) return;
    window.clearTimeout(completionTimer.current);
    completionTimer.current = null;
  };
  useEffect(() => () => cancelFakeCompletion(), []);
  const resetDraft = (kind: "s3" | "local") => {
    cancelFakeCompletion();
    setDraft(newStorage(kind, 2));
    setSecretStored(false);
    setAgentPhase("offer");
  };
  const updateDraft = (next: StorageConfig) => {
    if (next.kind !== draft.kind) {
      cancelFakeCompletion();
      setSecretStored(false);
      setAgentPhase("offer");
    }
    setDraft(next);
  };
  const launchAgent = (name: string) => {
    cancelFakeCompletion();
    setAgentMenuOpen(false);
    setAgentName(name);
    setAgentPhase("launched");
    completionTimer.current = window.setTimeout(() => {
      completionTimer.current = null;
      setDraft((current) => ({
        ...current,
        bucket: "mallard-r2-storage-2",
        account_id: "0123456789abcdef0123456789abcdef",
        s3_endpoint: "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com",
        access_key_id: "prepared-access-key-id",
        secret_access_key: "prepared-secret-access-key",
      }));
      setSecretStored(true);
      setAgentPhase("ready");
    }, 2800);
  };
  const copyInstruction = () => {
    cancelFakeCompletion();
    setAgentMenuOpen(false);
    setAgentPhase("listening");
    completionTimer.current = window.setTimeout(() => {
      completionTimer.current = null;
      setDraft((current) => ({
        ...current,
        bucket: "mallard-r2-storage-2",
        account_id: "0123456789abcdef0123456789abcdef",
        s3_endpoint: "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com",
        access_key_id: "prepared-access-key-id",
        secret_access_key: "prepared-secret-access-key",
      }));
      setSecretStored(true);
      setAgentPhase("ready");
    }, 2800);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div
        style={{
          flex: "0 0 auto",
          display: "flex",
          alignItems: "center",
          gap: 14,
          padding: "8px 14px",
          borderBottom: "1px solid var(--border)",
          background: "var(--surface-subtle)",
          color: "var(--text-2)",
        }}
      >
        <strong style={{ color: "var(--text-0)" }}>Storage setup</strong>
        <div className="sync-segmented" role="radiogroup" aria-label="Storage kind">
          <button
            type="button"
            role="radio"
            aria-checked={draft.kind === "local"}
            className={draft.kind === "local" ? "active" : undefined}
            onClick={() => resetDraft("local")}
          >
            Local folder
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={draft.kind === "s3"}
            className={draft.kind === "s3" ? "active" : undefined}
            onClick={() => resetDraft("s3")}
          >
            Cloudflare R2
          </button>
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
          <Icon name="sun" size={13} /> {dark ? "Light" : "Dark"}
        </button>
      </div>

      <div className="app" style={{ flex: 1, minHeight: 0, height: "auto" }}>
        <div className="sync-workspace">
          <SyncPathBar
            crumbs={crumbs}
            overflow={appActions}
            overflowLabel="App actions"
            ariaLabel="New storage path"
          />
          <main className="sync-main sync-project-links-page sync-storage-settings-page">
            <section className="profile-links-section" aria-label="New storage">
              <div className="sync-storage-settings-below sync-storage-settings-dedicated settings-expanded">
                <div className="sync-storage-details-panel">
                  <StorageEditor
                    storage={draft}
                    disabled={false}
                    onChange={updateDraft}
                    secretStored={secretStored}
                    showNameField
                    showProgress={false}
                    agentSlot={agentPhase === "launched" ? (
                      <>
                        <span className="sync-r2-guide-wait" role="status">
                          <span className="status-loader" /> {agentName} is running the setup — finish Cloudflare sign-in when the browser opens…
                        </span>
                        <button type="button" className="btn-link" onClick={() => { cancelFakeCompletion(); setAgentPhase("offer"); }}>Cancel</button>
                      </>
                    ) : agentPhase === "listening" ? (
                      <>
                        <span className="sync-r2-guide-connected" role="status">
                          <Icon name="check-circle" size={13} /> Copied.
                        </span>
                        <span className="sync-r2-guide-wait" role="status">
                          <span className="status-loader" /> Listening for your agent…
                        </span>
                        <button
                          type="button"
                          className={copyAgainFlashed ? "btn sync-r2-guide-btn sync-r2-copied-flash" : "btn sync-r2-guide-btn"}
                          onClick={() => {
                            setCopyAgainFlashed(true);
                            window.setTimeout(() => setCopyAgainFlashed(false), 1100);
                          }}
                        >
                          <Icon name={copyAgainFlashed ? "check" : "copy"} size={13} /> {copyAgainFlashed ? "Copied" : "Copy again"}
                        </button>
                        <button type="button" className="btn-link" onClick={() => { cancelFakeCompletion(); setAgentPhase("offer"); }}>Cancel</button>
                      </>
                    ) : agentPhase === "ready" ? (
                      <span className="sync-r2-guide-connected" role="status">
                        <Icon name="check-circle" size={13} /> R2 settings filled in. Review or edit them, then finish setup yourself.
                      </span>
                    ) : (
                      <div className="sync-r2-split">
                        <button
                          type="button"
                          className="btn sync-r2-guide-btn sync-r2-agent-cta"
                          onClick={() => launchAgent("Claude Code")}
                        >
                          <Icon name="claude" size={13} /> Run with Claude Code
                        </button>
                        <button
                          type="button"
                          className="btn sync-r2-guide-btn sync-r2-agent-cta sync-r2-split-arrow"
                          aria-haspopup="menu"
                          aria-expanded={agentMenuOpen}
                          aria-label="More agent setup routes"
                          onClick={() => setAgentMenuOpen((open) => !open)}
                        >
                          <Icon name="chevron-down" size={11} />
                        </button>
                        {agentMenuOpen && (
                          <div className="sync-path-menu end" role="menu">
                            <button type="button" className="sync-path-menu-item" role="menuitem" onClick={() => launchAgent("Codex")}>
                              <Icon name="openai" size={13} />
                              <span className="sync-r2-menu-text">
                                Run with Codex
                                <small>Opens a terminal window with the instruction as its prompt</small>
                              </span>
                            </button>
                            <button type="button" className="sync-path-menu-item" role="menuitem" onClick={copyInstruction}>
                              <Icon name="copy" size={13} />
                              <span className="sync-r2-menu-text">
                                Copy agent instruction
                                <small>Paste into any other coding agent yourself</small>
                              </span>
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                  />
                </div>

                <footer className="sync-storage-setup-footer">
                  {draft.kind !== "local" && (
                    <span className="sync-storage-editor-progress" aria-live="polite">
                      {r2FieldsSet(draft, secretStored)} of 5 set
                    </span>
                  )}
                  <div>
                    <button type="button" className="btn btn-ghost" onClick={noop}>Cancel</button>
                    <button type="button" className="btn btn-primary" disabled={!ready} onClick={noop}>
                      Finish setup
                    </button>
                  </div>
                </footer>
              </div>
            </section>
          </main>
        </div>
      </div>
    </div>
  );
}

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
