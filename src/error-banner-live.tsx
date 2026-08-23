import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import { NoticeZone } from "./components/notice/Notice";
import SyncPathBar, { type PathBarCrumb } from "./components/sync-review/SyncPathBar";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shell's error banner in the workspace stack it actually renders in:
 * banner, then path bar, then the page. The banner is a sibling of both, so
 * this is the only way to see whether it lines up with them.
 *
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const noop = () => {};

const crumbs: PathBarCrumb[] = [
  { id: "storages", label: "Storages", icon: "folder", ariaLabel: "All storages", onActivate: noop },
  { id: "scope", label: "Pets v1 · Default Codex", icon: "openai", onActivate: noop },
];

const RAW = "this row still owns registered resources; remove them first";
const WRITTEN = "Unlink every storage from “Pets v2 · Default Codex”, then remove each registration.";

function Harness() {
  const [dark, setDark] = useState(false);
  const [written, setWritten] = useState(true);
  document.documentElement.dataset.theme = dark ? "dark" : "light";

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div style={{ flex: "0 0 auto", display: "flex", gap: 12, padding: 10, fontFamily: "system-ui", fontSize: 13 }}>
        <label><input type="checkbox" checked={dark} onChange={() => setDark((v) => !v)} /> dark</label>
        <label><input type="checkbox" checked={written} onChange={() => setWritten((v) => !v)} /> written copy</label>
      </div>
      <div className="sync-app" style={{ flex: 1, minHeight: 0 }}>
        <div className="sync-workspace">
          <NoticeZone
            items={[{
              key: "shell-error",
              severity: "danger",
              message: written ? WRITTEN : RAW,
              action: (
                <>
                  <button type="button" className="btn">Open storage menu</button>
                  <button type="button" className="sync-notice-dismiss" title="Dismiss" aria-label="Dismiss error">
                    <Icon name="x" size={12} />
                  </button>
                </>
              ),
            }]}
            subject="messages from Mallard"
            className="sync-backend-banner"
            timing="blocking"
          />
          <SyncPathBar crumbs={crumbs} ariaLabel="Catalog path" />
          <main className="sync-main sync-project-links-page">
            <section className="profile-links-section" aria-label="Pets v2">
              <div className="sync-history-section-heading"><h2>Pets v2</h2></div>
              <div className="catalog-card">
                <div className="sync-history-thread-title"><strong>Maple</strong></div>
              </div>
            </section>
          </main>
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode><Harness /></StrictMode>,
);
