import { StrictMode, useState } from "react";
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
  const ready = storageConfigReady(draft, false);
  // The workspace's agent lifecycle, answered locally: copy flips the row to
  // listening, and a fake CLI finish lands the adoption note seconds later.
  const [agentPhase, setAgentPhase] = useState<"offer" | "listening" | "connected">("offer");
  const copyInstruction = () => {
    setAgentPhase("listening");
    window.setTimeout(() => setAgentPhase("connected"), 2800);
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
            onClick={() => setDraft(newStorage("local", 2))}
          >
            Local folder
          </button>
          <button
            type="button"
            role="radio"
            aria-checked={draft.kind === "s3"}
            className={draft.kind === "s3" ? "active" : undefined}
            onClick={() => setDraft(newStorage("s3", 2))}
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
                    onChange={setDraft}
                    secretStored={false}
                    showNameField
                    showProgress={false}
                    agentSlot={agentPhase === "listening" ? (
                      <span className="sync-r2-guide-wait" role="status">
                        <span className="status-loader" /> Paste into your agent — listening…
                      </span>
                    ) : agentPhase === "connected" ? (
                      <span className="sync-r2-guide-connected" role="status">
                        <Icon name="check-circle" size={13} /> Connected R2 storage 2. Settings filled in automatically.
                      </span>
                    ) : (
                      <button type="button" className="btn sync-r2-guide-btn sync-r2-agent-cta" onClick={copyInstruction}>
                        <Icon name="terminal" size={13} /> Copy agent instruction
                      </button>
                    )}
                  />
                </div>

                <footer className="sync-storage-setup-footer">
                  {draft.kind !== "local" && (
                    <span className="sync-storage-editor-progress" aria-live="polite">
                      {r2FieldsSet(draft, false)} of 5 set
                    </span>
                  )}
                  <div>
                    <button type="button" className="btn btn-ghost" onClick={noop}>Cancel</button>
                    <button type="button" className="btn btn-primary" disabled={!ready} onClick={noop}>
                      Create storage
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
