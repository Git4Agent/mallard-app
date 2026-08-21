import { StrictMode, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import SyncPathBar, { type PathBarMenuGroup } from "./components/sync-review/SyncPathBar";
import "./App.css";
import "./RetroTheme.css";

/**
 * Proposed global Settings page, inside the real shell. Not part of the app
 * bundle: the build has one entry, and this file is reachable only from the
 * dev server.
 *
 * The sidebar is a static copy of ProjectSidebar's markup with one addition —
 * a Settings item above Logs — because the shipped component has no such item
 * yet. The path bar is the real SyncPathBar. The page body is the proposal:
 * five groups of rows, each row one global preference, controls drawn from the
 * app's own vocabulary (switch tokens, select shell, pixel-face chrome).
 *
 * Settings shown dark/light via the page's own Theme control; `?theme=light`
 * or `?theme=dark` picks the starting palette for headless screenshots.
 */

const noop = () => undefined;

type ThemeChoice = "system" | "light" | "dark";

function resolveTheme(choice: ThemeChoice): "light" | "dark" {
  if (choice === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return choice;
}

function applyChoice(choice: ThemeChoice) {
  document.documentElement.dataset.theme = resolveTheme(choice);
}

const bootTheme = ((): ThemeChoice => {
  const param = new URLSearchParams(window.location.search).get("theme");
  return param === "dark" || param === "light" || param === "system" ? param : "light";
})();
applyChoice(bootTheme);

/** The sidebar as ProjectSidebar renders it, plus the proposed Settings item. */
function StaticSidebar() {
  return (
    <aside id="project-sidebar" className="sync-sidebar">
      <div className="sync-sidebar-head">
        <div className="sync-sidebar-drag" />
        <div className="sync-sidebar-brand">
          <img className="sync-brand-logo" src="/mallard-logo-duck.svg" alt="" />
          <strong>Mallard</strong>
        </div>

        <nav className="sync-primary-nav" aria-label="Project sync navigation">
          <button type="button">
            <Icon name="folder" size={15} /> Projects
          </button>
          <button type="button">
            <Icon name="catalog" size={15} /> Catalog
          </button>
          <button type="button">
            <Icon name="drive" size={15} /> Storages
          </button>
          <button type="button" className="active">
            <Icon name="settings" size={15} /> Settings
          </button>
          <button type="button" aria-expanded={false}>
            <Icon name="activity" size={15} /> Logs
            <span className="sidebar-nav-badge sync-activity-badge">2</span>
          </button>
        </nav>
      </div>

      <div className="sidebar-section-divider sidebar-head-divider" role="separator" />

      <div
        className="sync-sidebar-sections sidebar-settings-sections"
        style={{ gridTemplateRows: "minmax(96px, 0.56fr) 3px minmax(96px, 0.44fr) 3px fit-content(40%)" }}
      >
        <section className="sidebar-split-pane sidebar-project-pane" aria-label="Projects">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Projects</span>
            </div>
            <div className="sidebar-heading-actions">
              <button type="button" className="sidebar-section-action sidebar-add-action" title="Add project">
                <Icon name="plus" size={16} />
              </button>
            </div>
          </div>
          <div className="sidebar-profile-list">
            <div className="sidebar-profile-item">
              <button type="button" className="sidebar-profile-main">
                <Icon name="git-folder" size={16} className="sync-project-git-icon" />
                <span>mallard-app</span>
              </button>
              <div className="sidebar-profile-actions">
                <button type="button" className="sidebar-profile-remove" title="Remove">
                  <Icon name="trash" size={13} />
                </button>
              </div>
            </div>
          </div>
        </section>

        <div className="sidebar-section-divider" role="separator" />

        <section className="sidebar-split-pane sidebar-storage-pane" aria-label="Storages">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Storages</span>
            </div>
            <button type="button" className="sidebar-section-action sidebar-add-action" title="Add storage">
              <Icon name="plus" size={16} />
            </button>
          </div>
          <div className="sidebar-links-list">
            <div className="sidebar-profile-item">
              <button type="button" className="sidebar-profile-main">
                <Icon name="drive" size={16} />
                <span>Local storage 1</span>
              </button>
              <div className="sidebar-profile-actions">
                <button type="button" className="sidebar-profile-remove" title="Remove">
                  <Icon name="trash" size={13} />
                </button>
              </div>
            </div>
          </div>
        </section>

        <div className="sidebar-section-divider" role="separator" />

        <section className="sidebar-split-pane sidebar-catalog-pane" aria-label="Catalog">
          <div className="sidebar-section-heading">
            <div className="sidebar-section-label">
              <span className="sidebar-section-title">Catalog</span>
            </div>
            <button type="button" className="sidebar-section-action sidebar-add-action" title="Add Catalog resource">
              <Icon name="plus" size={16} />
            </button>
          </div>
          <div className="sidebar-profile-list">
            <div className="sidebar-profile-item">
              <button type="button" className="sidebar-profile-main">
                <Icon name="message" size={16} />
                <span>Claude sessions</span>
              </button>
              <div className="sidebar-profile-actions">
                <button type="button" className="sidebar-profile-remove" title="Remove">
                  <Icon name="trash" size={13} />
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>

      <footer className="sync-sidebar-product-mark" aria-label="Mallard version 0.1.2">
        <img src="/mallard-logo-duck.svg" alt="" />
        <span>v0.1.2</span>
      </footer>
    </aside>
  );
}

function SettingsRow({
  label,
  children,
  control,
}: {
  label: string;
  children: React.ReactNode;
  control: React.ReactNode;
}) {
  return (
    <div className="sync-settings-row">
      <div className="sync-settings-copy">
        <strong>{label}</strong>
        <p>{children}</p>
      </div>
      <div className="sync-settings-control">{control}</div>
    </div>
  );
}

function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
}) {
  return (
    <input
      type="checkbox"
      role="switch"
      className="sync-settings-switch"
      checked={checked}
      onChange={(event) => onChange(event.target.checked)}
      aria-label={label}
    />
  );
}

function SettingsPage() {
  const [themeChoice, setThemeChoice] = useState<ThemeChoice>(bootTheme);
  const [openLogs, setOpenLogs] = useState(true);
  const [focusRefresh, setFocusRefresh] = useState(true);
  const [clock24, setClock24] = useState(true);
  const [retention, setRetention] = useState("30");
  const [sizeCap, setSizeCap] = useState("100");

  useEffect(() => {
    applyChoice(themeChoice);
    if (themeChoice !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const follow = () => applyChoice("system");
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, [themeChoice]);

  const appActions: PathBarMenuGroup[] = [{
    label: "Mallard",
    items: [
      { id: "settings", label: "Settings", icon: "settings", onSelect: noop },
      {
        id: "reload",
        label: "Reload app data",
        detail: "Every project and storage, from disk",
        icon: "computer",
        onSelect: noop,
      },
    ],
  }];

  return (
    <div className="sync-workspace">
      <SyncPathBar
        crumbs={[{ id: "settings", label: "Settings", icon: "settings", tone: "root" }]}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="Settings path"
      />
      <main className="sync-main sync-project-links-page sync-settings-page">
        <section className="profile-links-section" aria-label="Settings">

          <section className="sync-settings-group" aria-labelledby="settings-appearance">
            <h2 id="settings-appearance">
              <Icon name="sun" size={12} /> Appearance
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Theme"
                control={(
                  <div className="sync-settings-segment" role="radiogroup" aria-label="Theme">
                    {(["system", "light", "dark"] as const).map((choice) => (
                      <button
                        key={choice}
                        type="button"
                        role="radio"
                        aria-checked={themeChoice === choice}
                        className={themeChoice === choice ? "active" : undefined}
                        onClick={() => setThemeChoice(choice)}
                      >
                        {choice === "system" ? "System" : choice === "light" ? "Light" : "Dark"}
                      </button>
                    ))}
                  </div>
                )}
              >
                System follows macOS. Light and dark share one warm palette.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-privacy">
            <h2 id="settings-privacy">
              <Icon name="lock" size={12} /> Privacy
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Credential redaction"
                control={<span className="sync-settings-flag">Always on</span>}
              >
                API keys, tokens, and passwords in captured conversations are replaced
                with typed placeholders before anything leaves this machine.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-sync">
            <h2 id="settings-sync">
              <Icon name="git-compare-arrows" size={12} /> Sync
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Open Logs on Push and Pull"
                control={<Switch checked={openLogs} onChange={setOpenLogs} label="Open Logs on Push and Pull" />}
              >
                Pop the Logs drawer whenever a transfer starts, so pushes and pulls
                narrate themselves.
              </SettingsRow>
              <SettingsRow
                label="Refresh on window focus"
                control={<Switch checked={focusRefresh} onChange={setFocusRefresh} label="Refresh on window focus" />}
              >
                Coming back to Mallard after a few seconds away re-reads every project
                and storage.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-logs">
            <h2 id="settings-logs">
              <Icon name="activity" size={12} /> Logs
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Keep events for"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={retention}
                      onChange={(event) => setRetention(event.target.value)}
                      aria-label="Keep log events for"
                    >
                      <option value="7">7 days</option>
                      <option value="30">30 days</option>
                      <option value="90">90 days</option>
                      <option value="180">180 days</option>
                      <option value="365">365 days</option>
                    </select>
                  </div>
                )}
              >
                Events older than this are pruned automatically.
              </SettingsRow>
              <SettingsRow
                label="Size cap"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={sizeCap}
                      onChange={(event) => setSizeCap(event.target.value)}
                      aria-label="Log size cap"
                    >
                      <option value="25">25 MB</option>
                      <option value="100">100 MB</option>
                      <option value="250">250 MB</option>
                      <option value="500">500 MB</option>
                      <option value="1000">1 GB</option>
                    </select>
                  </div>
                )}
              >
                Oldest files go first when the log folder outgrows the cap.
              </SettingsRow>
              <SettingsRow
                label="Log files"
                control={(
                  <>
                    <span className="sync-settings-readout">38 files · 12.6 MB</span>
                    <button type="button" className="btn">
                      <Icon name="folder" size={12} /> Open folder
                    </button>
                  </>
                )}
              >
                Logs stay on this machine in <code>~/.mallard/logs</code>.
              </SettingsRow>
              <SettingsRow
                label="Clean up"
                control={(
                  <>
                    <button type="button" className="btn">
                      <Icon name="trash" size={12} /> Clean up now
                    </button>
                    <button type="button" className="btn btn-ghost danger">Delete all</button>
                  </>
                )}
              >
                Removes events older than the retention window. Sync state, backups,
                and cloud data are never touched.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-display">
            <h2 id="settings-display">
              <Icon name="clock" size={12} /> Display
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="24-hour clock"
                control={<Switch checked={clock24} onChange={setClock24} label="24-hour clock" />}
              >
                One clock on every timestamp — session timelines, logs, and the
                catalog. Off follows the system locale.
              </SettingsRow>
            </div>
          </section>

        </section>
      </main>
    </div>
  );
}

function Harness() {
  return (
    <div className="sync-app" style={{ "--sync-sidebar-width": "300px" } as React.CSSProperties}>
      <StaticSidebar />
      <div className="sync-sidebar-resizer" role="separator" aria-orientation="vertical" />
      <SettingsPage />
    </div>
  );
}

const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);
