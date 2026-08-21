import { useCallback, useEffect, useState, type ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ActivityLogPolicy, ActivityLogStats } from "../../types";
import Icon from "../Icons";
import { formatBytes } from "../../format";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import {
  setPreference,
  setThemeChoice,
  usePreference,
  useThemeChoice,
  type BooleanPreferenceName,
  type ThemeChoice,
} from "../../preferences";
import { projectSyncApi } from "./api";
import { errorMessage } from "./model";

/**
 * The app-wide Settings page: a primary-nav destination like the Storages
 * index, not a dialog. Five groups of rows — every row one global preference
 * or one log-maintenance verb. UI preferences write through preferences.ts;
 * the Logs group is a front end to the activity-log policy the Rust side
 * keeps in ~/.mallard/logs/_policy.json, applied on change (this page
 * absorbed LogManagerDialog, whose explicit Save existed only to keep its
 * cleanup button honest against a drafted select).
 */

interface Props {
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  /** The log drawer shows these events; cleanup changes what it should show. */
  onLogsChanged: () => void;
}

const RETENTION_OPTIONS = [7, 30, 90, 180, 365];
const SIZE_OPTIONS_MB = [25, 100, 250, 500, 1_000];

const THEME_CHOICES: Array<{ value: ThemeChoice; label: string }> = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

function formatOldest(timestamp?: number | null): string | null {
  if (!timestamp) return null;
  return new Date(timestamp).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function SettingsRow({
  label,
  children,
  control,
}: {
  label: string;
  children: ReactNode;
  control: ReactNode;
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

/** A boolean preference as a drawn switch; state lives in preferences.ts. */
function PreferenceSwitch({ name, label }: { name: BooleanPreferenceName; label: string }) {
  const value = usePreference(name);
  return (
    <input
      type="checkbox"
      role="switch"
      className="sync-settings-switch"
      checked={value}
      onChange={(event) => setPreference(name, event.target.checked)}
      aria-label={label}
    />
  );
}

export default function SettingsPage({ appActions, onLogsChanged }: Props) {
  const themeChoice = useThemeChoice();
  const [stats, setStats] = useState<ActivityLogStats | null>(null);
  const [logsBusy, setLogsBusy] = useState(false);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [statsLoading, setStatsLoading] = useState(true);
  const [logsNotice, setLogsNotice] = useState<string | null>(null);

  /* Retryable, because the card below renders plausible defaults when `stats`
     is null — 30 days, 100 MB — and a failed fetch used to leave those on
     screen looking like the real policy with no way to try again. */
  const loadStats = useCallback(async () => {
    setStatsLoading(true);
    setLogsError(null);
    try {
      setStats(await projectSyncApi.getActivityLogStats());
    } catch (reason) {
      setLogsError(errorMessage(reason));
    } finally {
      setStatsLoading(false);
    }
  }, []);

  useEffect(() => { void loadStats(); }, [loadStats]);

  /** Selects apply on change; the returned stats keep the readout current. */
  const applyPolicy = async (patch: Partial<ActivityLogPolicy>) => {
    if (!stats) return;
    setLogsBusy(true);
    setLogsError(null);
    setLogsNotice(null);
    try {
      setStats(await projectSyncApi.updateActivityLogPolicy({ ...stats.policy, ...patch }));
    } catch (reason) {
      setLogsError(errorMessage(reason));
    } finally {
      setLogsBusy(false);
    }
  };

  const cleanupLogs = async (deleteAll: boolean) => {
    if (!stats) return;
    const approved = await confirm(
      deleteAll
        ? "Delete all retained activity logs?\n\nMallard configuration, backups, local files, and cloud revisions will not be changed."
        : `Remove retained logs older than ${stats.policy.retention_days} days?\n\n${stats.file_count} log file${stats.file_count === 1 ? "" : "s"} (${formatBytes(stats.total_bytes)}) ${stats.file_count === 1 ? "is" : "are"} currently retained. Removed logs cannot be recovered.`,
      deleteAll
        ? { title: "Delete activity logs", kind: "warning", okLabel: "Delete logs", cancelLabel: "Cancel" }
        : { title: "Clean up logs", kind: "warning", okLabel: "Clean up", cancelLabel: "Cancel" },
    );
    if (!approved) return;
    setLogsBusy(true);
    setLogsError(null);
    setLogsNotice(null);
    try {
      const result = await projectSyncApi.cleanupActivityLogs(deleteAll ? { delete_all: true } : {});
      setStats(result.stats);
      setLogsNotice(
        deleteAll
          ? "Retained activity logs deleted"
          : result.removed_files
            ? `Removed ${result.removed_files} log file${result.removed_files === 1 ? "" : "s"} · ${formatBytes(result.reclaimed_bytes)} reclaimed`
            : "No old logs needed cleanup",
      );
      onLogsChanged();
    } catch (reason) {
      setLogsError(errorMessage(reason));
    } finally {
      setLogsBusy(false);
    }
  };

  const openLogsFolder = async () => {
    setLogsError(null);
    try {
      await openPath(await projectSyncApi.getActivityLogFolder());
    } catch (reason) {
      setLogsError(errorMessage(reason));
    }
  };

  const oldest = formatOldest(stats?.oldest_ts);

  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "settings", label: "Settings", icon: "settings", tone: "root" }]}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="Settings path"
      />
      <main className="sync-main sync-project-links-page sync-settings-page">
        <section className="profile-links-section" aria-label="Settings">

          <section className="sync-settings-group" aria-labelledby="settings-appearance">
            <h2 className="settings-section-title" id="settings-appearance">
              <Icon name="sun" size={12} /> Appearance
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Theme"
                control={(
                  <div className="sync-settings-segment" role="radiogroup" aria-label="Theme">
                    {THEME_CHOICES.map((choice) => (
                      <button
                        key={choice.value}
                        type="button"
                        role="radio"
                        aria-checked={themeChoice === choice.value}
                        className={themeChoice === choice.value ? "active" : undefined}
                        onClick={() => setThemeChoice(choice.value)}
                      >
                        {choice.label}
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
            <h2 className="settings-section-title" id="settings-privacy">
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
            <h2 className="settings-section-title" id="settings-sync">
              <Icon name="git-compare-arrows" size={12} /> Sync
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Open Logs on Push and Pull"
                control={<PreferenceSwitch name="openLogsOnSync" label="Open Logs on Push and Pull" />}
              >
                Pop the Logs drawer whenever a transfer starts, so pushes and pulls
                narrate themselves.
              </SettingsRow>
              <SettingsRow
                label="Refresh on window focus"
                control={<PreferenceSwitch name="refreshOnFocus" label="Refresh on window focus" />}
              >
                Coming back to Mallard after a few seconds away re-reads every project
                and storage.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-logs">
            <h2 className="settings-section-title" id="settings-logs">
              <Icon name="activity" size={12} /> Logs
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Keep events for"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={stats?.policy.retention_days ?? 30}
                      disabled={logsBusy || !stats}
                      onChange={(event) => void applyPolicy({ retention_days: Number(event.target.value) })}
                      aria-label="Keep log events for"
                    >
                      {stats && !RETENTION_OPTIONS.includes(stats.policy.retention_days) && (
                        <option value={stats.policy.retention_days}>{stats.policy.retention_days} days</option>
                      )}
                      {RETENTION_OPTIONS.map((days) => <option key={days} value={days}>{days} days</option>)}
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
                      value={stats ? Math.round(stats.policy.max_total_bytes / (1024 * 1024)) : 100}
                      disabled={logsBusy || !stats}
                      onChange={(event) => void applyPolicy({
                        max_total_bytes: Number(event.target.value) * 1024 * 1024,
                      })}
                      aria-label="Log size cap"
                    >
                      {stats && !SIZE_OPTIONS_MB.includes(Math.round(stats.policy.max_total_bytes / (1024 * 1024))) && (
                        <option value={Math.round(stats.policy.max_total_bytes / (1024 * 1024))}>
                          {formatBytes(stats.policy.max_total_bytes)}
                        </option>
                      )}
                      {SIZE_OPTIONS_MB.map((size) => <option key={size} value={size}>{size.toLocaleString()} MB</option>)}
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
                    <span
                      className="sync-settings-readout"
                      title={oldest ? `Oldest entry ${oldest}` : undefined}
                    >
                      {stats ? `${stats.file_count} file${stats.file_count === 1 ? "" : "s"} · ${formatBytes(stats.total_bytes)}` : "…"}
                    </span>
                    <button type="button" className="btn" onClick={() => void openLogsFolder()} disabled={logsBusy}>
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
                    <button type="button" className="btn" onClick={() => void cleanupLogs(false)} disabled={logsBusy || !stats}>
                      <Icon name="trash" size={12} /> Clean up now
                    </button>
                    <button
                      type="button"
                      className="btn btn-ghost danger"
                      onClick={() => void cleanupLogs(true)}
                      disabled={logsBusy || !stats || stats.file_count === 0}
                    >
                      Delete all
                    </button>
                  </>
                )}
              >
                Removes events older than the retention window. Sync state, backups,
                and cloud data are never touched.
              </SettingsRow>
            </div>
            {logsError && (
              <div className="sync-callout error" role="alert">
                <Icon name="alert-triangle" size={15} />
                <span>
                  {stats ? logsError : `Couldn’t read the log policy. ${logsError}`}
                </span>
                {!stats && (
                  <button type="button" className="btn" disabled={statsLoading} onClick={() => void loadStats()}>
                    <Icon name="refresh" size={14} className={statsLoading ? "icon-spin" : undefined} />
                    {statsLoading ? "Retrying…" : "Retry"}
                  </button>
                )}
              </div>
            )}
            {logsNotice && (
              <div className="sync-settings-note" role="status">
                <Icon name="check-circle" size={14} /> {logsNotice}
              </div>
            )}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-display">
            <h2 className="settings-section-title" id="settings-display">
              <Icon name="clock" size={12} /> Display
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="24-hour clock"
                control={<PreferenceSwitch name="clock24" label="24-hour clock" />}
              >
                One clock on every timestamp — session timelines, logs, and the
                catalog. Off follows the system locale.
              </SettingsRow>
            </div>
          </section>

        </section>
      </main>
    </>
  );
}
