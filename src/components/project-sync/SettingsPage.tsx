import { useCallback, useEffect, useState, type ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ActivityLogPolicy, ActivityLogStats, AppSettings, LocalStateSummary } from "../../types";
import Icon from "../Icons";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { formatBytes } from "../../format";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import {
  clearStoredUiState,
  setPreference,
  setThemeChoice,
  usePreference,
  useThemeChoice,
  type BooleanPreferenceName,
  type ThemeChoice,
} from "../../preferences";
import { errorMessage } from "../../storageError";
import { projectSyncApi } from "./api";
import { LoadingBone } from "../LoadingScaffold";

/**
 * The app-wide Settings page: a primary-nav destination like the Storages
 * index, not a dialog. Seven groups of rows — every row one global preference
 * or one maintenance verb. UI preferences write through preferences.ts;
 * the Performance group fronts the tuning document the Rust side keeps in
 * ~/.mallard/settings.json, and the Logs group the activity-log policy in
 * ~/.mallard/logs/_policy.json, both applied on change (this page absorbed
 * LogManagerDialog, whose explicit Save existed only to keep its cleanup
 * button honest against a drafted select).
 */

interface Props {
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  /** The log drawer shows these events; cleanup changes what it should show. */
  onLogsChanged: () => void;
}

const RETENTION_OPTIONS = [7, 30, 90, 180, 365];
const SIZE_OPTIONS_MB = [25, 100, 250, 500, 1_000];
const THREAD_CAP_OPTIONS_MB = [25, 50, 100, 200, 400];
const PREVIEW_OPTIONS_KB = [256, 512, 1_024, 2_048];
const WORKER_OPTIONS = [1, 2, 4, 6, 8];
const TRANSFER_OPTIONS = [1, 2, 4, 6, 8, 12];

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
  const [tuning, setTuning] = useState<AppSettings | null>(null);
  const [tuningBusy, setTuningBusy] = useState(false);
  const [tuningError, setTuningError] = useState<string | null>(null);
  const [tuningSection, setTuningSection] = useState<"privacy" | "sync" | "performance">("privacy");
  const [tuningLoading, setTuningLoading] = useState(true);
  const [localSummary, setLocalSummary] = useState<LocalStateSummary | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);
  const [resetError, setResetError] = useState<string | null>(null);
  const [resetPhase, setResetPhase] = useState<"idle" | "working" | "reloading">("idle");
  /* Each group's box can hold initial-load content or an action result. The
     mount loads below are part of rendering this page, so a load that
     failed is content the view came up with — a named region, not an
     announcement (WCAG 2.2 SC 4.1.3). Retry, a select, a cleanup, Open folder
     and Reset are all things the user DID, and the same box has to announce
     once one of them speaks. After the first interaction, every message in
     that group answers an action. */
  const [logsFromAction, setLogsFromAction] = useState(false);
  const [tuningFromAction, setTuningFromAction] = useState(false);
  const [localDataFromAction, setLocalDataFromAction] = useState(false);

  /* Retryable because null stats render plausible defaults — 30 days and
     100 MB. A failed fetch must not leave those values without a retry. The
     retry passes `retried` so the refilled box announces the result. */
  const loadStats = useCallback(async (retried = false) => {
    setStatsLoading(true);
    setLogsError(null);
    if (retried) setLogsFromAction(true);
    try {
      setStats(await projectSyncApi.getActivityLogStats());
    } catch (reason) {
      setLogsError(errorMessage(reason));
    } finally {
      setStatsLoading(false);
    }
  }, []);

  useEffect(() => { void loadStats(); }, [loadStats]);

  /* Same retryable shape as loadStats: the selects render the defaults while
     `tuning` is null, and those must not pass for the stored values. */
  const loadTuning = useCallback(async (retried = false) => {
    setTuningLoading(true);
    setTuningError(null);
    if (retried) setTuningFromAction(true);
    try {
      setTuning(await projectSyncApi.getAppSettings());
    } catch (reason) {
      setTuningError(errorMessage(reason));
    } finally {
      setTuningLoading(false);
    }
  }, []);

  useEffect(() => { void loadTuning(); }, [loadTuning]);

  /* Same retryable shape as loadStats: the readout is a measurement, and a
     failed walk must not wedge the row it decorates. */
  const loadLocalSummary = useCallback(async (retried = false) => {
    setSummaryLoading(true);
    setResetError(null);
    if (retried) setLocalDataFromAction(true);
    try {
      setLocalSummary(await projectSyncApi.getLocalStateSummary());
    } catch (reason) {
      setResetError(errorMessage(reason));
    } finally {
      setSummaryLoading(false);
    }
  }, []);

  useEffect(() => { void loadLocalSummary(); }, [loadLocalSummary]);

  /** Selects apply on change; the returned stats keep the readout current. */
  const applyPolicy = async (patch: Partial<ActivityLogPolicy>) => {
    if (!stats) return;
    setLogsBusy(true);
    setLogsFromAction(true);
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

  /** Selects apply on change; the returned settings are the saved truth. */
  const applyTuning = async (patch: Partial<AppSettings>) => {
    if (!tuning) return;
    setTuningBusy(true);
    setTuningSection("credential_redaction_enabled" in patch ? "privacy"
      : "push_codex_children" in patch || "push_claude_children" in patch ? "sync" : "performance");
    setTuningFromAction(true);
    setTuningError(null);
    try {
      setTuning(await projectSyncApi.updateAppSettings({ ...tuning, ...patch }));
    } catch (reason) {
      setTuningError(errorMessage(reason));
    } finally {
      setTuningBusy(false);
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
    setLogsFromAction(true);
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
    setLogsFromAction(true);
    setLogsError(null);
    try {
      await openPath(await projectSyncApi.getActivityLogFolder());
    } catch (reason) {
      setLogsError(errorMessage(reason));
    }
  };

  /* Deliberately still offered when the summary failed to load: a user whose
     local state is too broken to measure is exactly who needs the reset. The
     confirm simply omits the size line then. */
  const resetMallard = async () => {
    const sizeLine = localSummary
      ? ` (${formatBytes(localSummary.total_bytes)} across ${localSummary.file_count} file${localSummary.file_count === 1 ? "" : "s"}, including ${formatBytes(localSummary.backups_bytes)} of backups)`
      : "";
    const approved = await confirm(
      `Permanently delete all of Mallard's local data and restart?\n\nEverything in ~/.mallard is deleted${sizeLine} — sync metadata, activity logs, and the only local backups of files Pull has replaced — and Mallard's settings return to their defaults. This cannot be undone.\n\nCloud revisions and your project files are not touched. Mallard will restart as a fresh install.`,
      { title: "Reset Mallard", kind: "warning", okLabel: "Reset Mallard", cancelLabel: "Cancel" },
    );
    if (!approved) return;
    setResetPhase("working");
    setLocalDataFromAction(true);
    setResetError(null);
    try {
      await projectSyncApi.resetLocalState();
    } catch (reason) {
      setResetError(errorMessage(reason));
      setResetPhase("idle");
      return;
    }
    // A reload, not a process restart: relaunching the binary under
    // `tauri dev` orphans the frontend from its dev server (the CLI kills
    // Vite when the original process exits) and lands on a white window.
    // The backend re-armed its watchers inside the command, everything else
    // there is per-call, and a reload re-boots the frontend the same way in
    // dev and in production. Sweep the webview-side state first; no
    // `finally` — the page stays locked until the reload tears it down.
    clearStoredUiState();
    setResetPhase("reloading");
    window.location.reload();
  };

  const oldest = formatOldest(stats?.oldest_ts);

  /* Each of the two groups below says at most two things — a failure and a
     receipt — and the two genuinely co-render: neither `loadStats` nor
     `openLogsFolder` clears the notice, so a failed folder open leaves the
     last cleanup's receipt standing beside it. One box per group, then, at the
     loudest rung present, instead of a red row stacked on a green one. */
  const logsNotices: NoticeItem[] = [];
  if (logsError) {
    logsNotices.push({
      key: "logs-error",
      severity: "danger",
      message: stats ? logsError : `Couldn’t read the log policy. ${logsError}`,
      action: stats ? undefined : (
        <button type="button" className="btn" disabled={statsLoading} onClick={() => void loadStats(true)}>
          <Icon name="refresh" size={14} className={statsLoading ? "icon-spin" : undefined} />
          {statsLoading ? "Retrying…" : "Retry"}
        </button>
      ),
    });
  }
  if (logsNotice) {
    logsNotices.push({ key: "logs-outcome", severity: "success", message: logsNotice });
  }
  /* Standing until the user touches the group: the only thing that can be in
     this box on first paint is the mount load's failure, and content the view
     renders with is not a status message. After a Retry, a select, a cleanup
     or an Open folder it is an answer — assertive while the failure stands,
     polite for a receipt. */
  const logsTiming: NoticeTiming = !logsFromAction
    ? { timing: "standing", label: "Problems with activity logs" }
    : logsError
      ? { timing: "blocking" }
      : { timing: "progress" };

  /* One thing to say: a load or apply failure. Applies post no receipt, like
     the Logs selects — the select showing the new value is the receipt. */
  const tuningNotices: NoticeItem[] = [];
  if (tuningError) {
    tuningNotices.push({
      key: "tuning-error",
      severity: "danger",
      message: tuning ? tuningError : `Couldn’t read the app settings. ${tuningError}`,
      action: tuning ? undefined : (
        <button type="button" className="btn" disabled={tuningLoading} onClick={() => void loadTuning(true)}>
          <Icon name="refresh" size={14} className={tuningLoading ? "icon-spin" : undefined} />
          {tuningLoading ? "Retrying…" : "Retry"}
        </button>
      ),
    });
  }
  /* Same reading as the Logs group: the mount load's failure is standing
     content; after a Retry or a select it is an answer. */
  const tuningTiming: NoticeTiming = !tuningFromAction
    ? { timing: "standing", label: "Problems with app settings" }
    : tuningError
      ? { timing: "blocking" }
      : { timing: "progress" };

  const localDataNotices: NoticeItem[] = [];
  if (resetError) {
    localDataNotices.push({
      key: "local-data-error",
      severity: "danger",
      message: resetError,
      action: localSummary ? undefined : (
        <button type="button" className="btn" disabled={summaryLoading} onClick={() => void loadLocalSummary(true)}>
          <Icon name="refresh" size={14} className={summaryLoading ? "icon-spin" : undefined} />
          {summaryLoading ? "Retrying…" : "Retry"}
        </button>
      ),
    });
  }
  if (resetPhase === "reloading") {
    localDataNotices.push({
      key: "local-data-reloading",
      severity: "success",
      message: "Local data deleted — starting fresh…",
    });
  }
  /* Same reading as the Logs group: the measurement walk runs on mount, so its
     failure is standing content, while a failed Reset — or a retried walk — is
     an event this box has to announce. */
  const localDataTiming: NoticeTiming = !localDataFromAction
    ? { timing: "standing", label: "Problems with local data" }
    : resetError
      ? { timing: "blocking" }
      : { timing: "progress" };

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
                control={(
                  <input
                    type="checkbox"
                    role="switch"
                    className="sync-settings-switch"
                    checked={tuning?.credential_redaction_enabled ?? true}
                    disabled={tuningBusy || !tuning}
                    onChange={(event) => void applyTuning({ credential_redaction_enabled: event.target.checked })}
                    aria-label="Credential redaction"
                  />
                )}
              >
                On replaces detected credentials in exported sessions and provider state.
                Off can export API keys, tokens, and passwords. Applies to this machine.
              </SettingsRow>
            </div>
            {tuningSection === "privacy" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-sync">
            <h2 className="settings-section-title" id="settings-sync">
              <Icon name="git-compare-arrows" size={12} /> Sync
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Push Codex children"
                control={
                  <input
                    type="checkbox"
                    role="switch"
                    className="sync-settings-switch"
                    checked={tuning?.push_codex_children ?? true}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(event) => void applyTuning({ push_codex_children: event.target.checked })}
                    aria-label="Push Codex children"
                  />
                }
              >
                Include child conversations in Push. Pull restores the files stored in the cloud.
              </SettingsRow>
              <SettingsRow
                label="Push Claude children"
                control={
                  <input
                    type="checkbox"
                    role="switch"
                    className="sync-settings-switch"
                    checked={tuning?.push_claude_children ?? true}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(event) => void applyTuning({ push_claude_children: event.target.checked })}
                    aria-label="Push Claude children"
                  />
                }
              >
                Include child conversations in Push. Pull restores the files stored in the cloud.
              </SettingsRow>
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
            {tuningSection === "sync" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-performance">
            <h2 className="settings-section-title" id="settings-performance">
              <Icon name="tool" size={12} /> Performance
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Thread size limit"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={tuning ? Math.round(tuning.conversation_file_limit_bytes / (1024 * 1024)) : 100}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({
                        conversation_file_limit_bytes: Number(event.target.value) * 1024 * 1024,
                      })}
                      aria-label="Thread size limit"
                    >
                      {tuning && !THREAD_CAP_OPTIONS_MB.includes(Math.round(tuning.conversation_file_limit_bytes / (1024 * 1024))) && (
                        <option value={Math.round(tuning.conversation_file_limit_bytes / (1024 * 1024))}>
                          {formatBytes(tuning.conversation_file_limit_bytes)}
                        </option>
                      )}
                      {THREAD_CAP_OPTIONS_MB.map((size) => <option key={size} value={size}>{size} MB</option>)}
                    </select>
                  </div>
                )}
              >
                The biggest session transcript Mallard will read. Threads over the
                cap are skipped by compare, Push, and Pull.
              </SettingsRow>
              <SettingsRow
                label="Preview size limit"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={tuning ? Math.round(tuning.timeline_preview_limit_bytes / 1024) : 1024}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({
                        timeline_preview_limit_bytes: Number(event.target.value) * 1024,
                      })}
                      aria-label="Preview size limit"
                    >
                      {tuning && !PREVIEW_OPTIONS_KB.includes(Math.round(tuning.timeline_preview_limit_bytes / 1024)) && (
                        <option value={Math.round(tuning.timeline_preview_limit_bytes / 1024)}>
                          {formatBytes(tuning.timeline_preview_limit_bytes)}
                        </option>
                      )}
                      {PREVIEW_OPTIONS_KB.map((size) => (
                        <option key={size} value={size}>
                          {size >= 1_024 ? `${size / 1_024} MB` : `${size} KB`}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
              >
                How much of one long message or tool call the session timeline
                shows. Longer content is trimmed in the view only; transcripts are
                never cut.
              </SettingsRow>
              <SettingsRow
                label="Comparison workers"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={tuning?.comparison_workers ?? 4}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ comparison_workers: Number(event.target.value) })}
                      aria-label="Comparison workers"
                    >
                      {tuning && !WORKER_OPTIONS.includes(tuning.comparison_workers) && (
                        <option value={tuning.comparison_workers}>
                          {tuning.comparison_workers} worker{tuning.comparison_workers === 1 ? "" : "s"}
                        </option>
                      )}
                      {WORKER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{count} worker{count === 1 ? "" : "s"}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                Worker threads that scan and compare local sessions. Capped by this
                machine's cores; the default suits most disks.
              </SettingsRow>
              <SettingsRow
                label="Cloud transfers"
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={tuning?.cloud_transfer_concurrency ?? 6}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ cloud_transfer_concurrency: Number(event.target.value) })}
                      aria-label="Parallel cloud transfers"
                    >
                      {tuning && !TRANSFER_OPTIONS.includes(tuning.cloud_transfer_concurrency) && (
                        <option value={tuning.cloud_transfer_concurrency}>
                          {tuning.cloud_transfer_concurrency} transfer{tuning.cloud_transfer_concurrency === 1 ? "" : "s"}
                        </option>
                      )}
                      {TRANSFER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{count} transfer{count === 1 ? "" : "s"}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                Objects moved in parallel during Push and Pull. Lower it on slow or
                metered connections.
              </SettingsRow>
            </div>
            {tuningSection === "performance" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
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
            <NoticeZone
              items={logsNotices}
              subject="notes about activity logs"
              {...logsTiming}
            />
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

          <section className="sync-settings-group" aria-labelledby="settings-local-data">
            <h2 className="settings-section-title" id="settings-local-data">
              <Icon name="drive" size={12} /> Local data
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Reset Mallard"
                control={(
                  <>
                    <span className="sync-settings-readout" title={localSummary?.root_path}>
                      {localSummary
                        ? `${localSummary.file_count} file${localSummary.file_count === 1 ? "" : "s"} · ${formatBytes(localSummary.total_bytes)}`
                        : summaryLoading
                          // A bone, not a plausible-looking placeholder: the
                          // value is being read, and only a failed read shows
                          // the honest "unknown" ellipsis.
                          ? <LoadingBone width={110} height={11} />
                          : "…"}
                    </span>
                    <button
                      type="button"
                      className="btn btn-ghost danger"
                      onClick={() => void resetMallard()}
                      disabled={resetPhase !== "idle"}
                    >
                      Reset Mallard
                    </button>
                  </>
                )}
              >
                Permanently deletes everything in <code>~/.mallard</code> — sync
                metadata, activity logs, and the backups of files Pull replaced —
                clears Mallard's saved preferences, then restarts as a fresh
                install. Cloud revisions and your project files are not touched.
              </SettingsRow>
            </div>
            <NoticeZone
              items={localDataNotices}
              subject="notes about local data"
              {...localDataTiming}
            />
          </section>

        </section>
      </main>
    </>
  );
}
