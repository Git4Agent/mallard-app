import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import Icon from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb, type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import { BranchPicker } from "./components/project-sync/ProjectActivityPage";
import "./App.css";
import "./RetroTheme.css";

/**
 * The shipped path bar, on the session-detail trail, at a width you can drag.
 * Not part of the app bundle: the build has one entry, and this file is
 * reachable only from the dev server.
 */

const SESSION_TITLE = "Add horror story document (Preserved 1 · Host 68f3a2c1)";
const PROJECT_NAME = "HorrorGame (myconfig)";
const STORAGE_NAME = "Local storage 1";

const noop = () => {};

const menu: PathBarMenuGroup[] = [
  { items: [{ id: "a", label: "Storage settings…", icon: "settings", onSelect: noop }] },
];

function crumbs(view: "session" | "activity"): PathBarCrumb[] {
  return [
    {
      id: "storage",
      label: STORAGE_NAME,
      icon: "drive",
      tone: "root",
      menuHeader: { label: STORAGE_NAME, detail: "/Users/hequ/storage", detailIsPath: true, icon: "drive" },
      menu,
      menuLabel: "Storage",
    },
    {
      id: "project",
      label: PROJECT_NAME,
      icon: "git-folder",
      onActivate: noop,
      menuHeader: { label: PROJECT_NAME, detail: "~/Desktop/HorrorGame", detailIsPath: true, icon: "git-folder" },
      menu,
      menuLabel: "Project",
    },
    view === "activity" ? {
      id: "section",
      label: "Git history",
      tone: "section",
      scope: (
        <BranchPicker
          branches={[
            { name: "main", is_current: false, available: true },
            { name: "feature/preserved-provenance", is_current: true, available: true },
          ]}
          selectedBranch="feature/preserved-provenance"
          disabled={false}
          onChange={noop}
        />
      ),
    } : {
      id: "session",
      label: SESSION_TITLE,
      title: `${SESSION_TITLE} · Claude Code · Active`,
      leading: (
        <span className="sync-path-leaf-mark active">
          <ProviderMark provider="claude" displayName="Claude Code" />
          <span className="sync-visually-hidden">Active</span>
        </span>
      ),
    },
  ];
}

const actions = (
  <>
    <span className="sync-path-attention warning" tabIndex={0} role="status">
      <Icon name="alert-triangle" size={13} />
      <b>3</b>
    </span>
    <button type="button" className="sync-path-icon-action" title="Refresh session">
      <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
    </button>
    <div className="sync-path-action-group" role="group" aria-label="Sync actions">
      <button type="button" className="sync-path-sync">
        <Icon name="download" size={14} />
        <span className="sync-path-sync-label">Pull</span>
        <span className="sync-path-count" aria-hidden="true">2</span>
      </button>
      <button type="button" className="sync-path-sync">
        <Icon name="upload" size={14} />
        <span className="sync-path-sync-label">Push</span>
        <span className="sync-path-count" aria-hidden="true">12</span>
      </button>
    </div>
  </>
);

const overflow: PathBarMenuGroup[] = [
  { items: [{ id: "about", label: "About Mallard", icon: "info", onSelect: noop }] },
];

function Specimen() {
  const [width, setWidth] = useState(588);
  const [sidebarHidden, setSidebarHidden] = useState(false);
  const [view, setView] = useState<"session" | "activity">("session");

  return (
    <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 16 }}>
      <div style={{ display: "flex", gap: 12, alignItems: "center", fontFamily: "system-ui", fontSize: 13 }}>
        <label>
          width{" "}
          <input
            type="range"
            min={420}
            max={1400}
            value={width}
            onChange={(event) => setWidth(Number(event.target.value))}
          />
        </label>
        <b id="width-readout">{width}px</b>
        <label>
          <input
            type="checkbox"
            checked={sidebarHidden}
            onChange={(event) => setSidebarHidden(event.target.checked)}
          />{" "}
          sidebar hidden
        </label>
        <label>
          <input
            type="checkbox"
            checked={view === "activity"}
            onChange={(event) => setView(event.target.checked ? "activity" : "session")}
          />{" "}
          activity view (section + branch chip)
        </label>
      </div>
      <div
        id="frame"
        className={`sync-app${sidebarHidden ? " sidebar-hidden" : ""}`}
        style={{ width, border: "1px solid #888", overflow: "hidden", resize: "horizontal" }}
      >
        <div className="sync-workspace">
          <SyncPathBar crumbs={crumbs(view)} overflow={overflow} overflowLabel="App actions" actions={actions} />
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode><Specimen /></StrictMode>,
);
