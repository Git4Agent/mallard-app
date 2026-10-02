import { useCallback, useEffect, useState, type ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type { ActivityLogPolicy, ActivityLogStats, AppSettings, LocalStateSummary } from "../../types";
import Icon from "../Icons";
import SettingsSectionIcon from "./SettingsSectionIcon";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { clockOptions } from "../../clock";
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
import { rowReceipt, statusLine, useSettingsReceipts, type RowReceipt, type StatusLine } from "./settingsReceipts";
import { AUTO_CHECK_INTERVAL_HOURS, useInstalledVersion, type AppUpdates } from "../AppUpdater";
import { formatRelativeTime } from "../../clock";

/**
 * The app-wide Settings page: a primary-nav destination like the Storages
 * index, not a dialog. Seven groups of rows — every row one global preference
 * or one maintenance verb. UI preferences write through preferences.ts;
 * the Performance group fronts the tuning document the Rust side keeps in
 * ~/.mallard/settings.json, and the Logs group the activity-log policy in
 * ~/.mallard/logs/_policy.json, both applied on change (this page absorbed
 * LogManagerDialog, whose explicit Save existed only to keep its cleanup
 * button honest against a drafted select).
 *
 * Applying on change is the platform rule for settings, so there is no Save.
 * What the page owes instead is proof: a receipt beside the control that
 * changed and one status line in the bar. The bar's Done only leaves —
 * nothing is left to save by then.
 */

interface Props {
  /** Theme and app-wide commands; the bar here is the window's only chrome. */
  appActions: PathBarMenuGroup[];
  /** The shell's updater: the manual check and the ready release. */
  appUpdates: AppUpdates;
  /** The log drawer shows these events; cleanup changes what it should show. */
  onLogsChanged: () => void;
  /** Leaves Settings. */
  onDone: () => void;
  /** Where Done lands — the open project, or the Projects landing. */
  doneTarget: string;
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

/* A new write to a store clears that store's failures, the way the section's
   error clears when its next apply starts. */
const TUNING_KEYS = [
  "credential_redaction_enabled",
  "update_check_enabled",
  "push_codex_children",
  "push_claude_children",
  "conversation_file_limit_bytes",
  "timeline_preview_limit_bytes",
  "comparison_workers",
  "cloud_transfer_concurrency",
] as const;
const POLICY_KEYS = ["retention_days", "max_total_bytes"] as const;

type TuningKey = (typeof TUNING_KEYS)[number];
type PolicyKey = (typeof POLICY_KEYS)[number];
type LocalKey = "theme" | BooleanPreferenceName;
type SettingKey = LocalKey | TuningKey | PolicyKey;

/** What the status region calls a setting once it saves: its row's label. */
const SETTING_LABELS: Record<SettingKey, string> = {
  theme: "Theme",
  credential_redaction_enabled: "Credential redaction",
  update_check_enabled: "Automatic updates",
  push_codex_children: "Push Codex subagents",
  push_claude_children: "Push Claude subagents",
  openLogsOnSync: "Open Logs on Push and Pull",
  refreshOnFocus: "Refresh on window focus",
  conversation_file_limit_bytes: "Thread size limit",
  timeline_preview_limit_bytes: "Preview size limit",
  comparison_workers: "Comparison workers",
  cloud_transfer_concurrency: "Cloud transfers",
  retention_days: "Keep events for",
  max_total_bytes: "Size cap",
  clock24: "24-hour clock",
};

/* Local preferences have no section error to explain a refused write, so the
   stamp carries the reason. */
const LOCAL_WRITE_REFUSED = "This machine refused to store the change, so the previous value stays.";

const RECEIPT_TEXT: Record<RowReceipt["kind"], string> = {
  saving: "Saving…",
  saved: "Saved",
  failed: "Not saved",
};

const rowId = (key: SettingKey) => `settings-row-${key}`;

function formatOldest(timestamp?: number | null): string | null {
  if (!timestamp) return null;
  return new Date(timestamp).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/**
 * The row's stamp. It stays one element from Saved through its dimmed last
 * beat, so the dim is a class change, not a remount. Hidden from assistive
 * technology: the page's status region speaks for every stamp.
 */
function SettingReceipt({ receipt, failureHint }: { receipt: RowReceipt | null; failureHint?: string }) {
  if (!receipt) return null;
  const fading = receipt.kind === "saved" && receipt.fading;
  return (
    <span
      className={`sync-settings-receipt ${receipt.kind}${fading ? " fading" : ""}`}
      title={receipt.kind === "failed" ? failureHint : undefined}
      aria-hidden="true"
    >
      <Icon
        name={receipt.kind === "saving" ? "refresh" : receipt.kind === "failed" ? "alert-triangle" : "check"}
        size={12}
        className={receipt.kind === "saving" ? "icon-spin" : undefined}
      />
      {RECEIPT_TEXT[receipt.kind]}
    </span>
  );
}

function SettingsRow({
  label,
  children,
  control,
  setting,
  receipt = null,
  failureHint,
}: {
  label: string;
  children: ReactNode;
  control: ReactNode;
  /** A row that holds a setting keeps a receipt slot and an anchor the status line can find. */
  setting?: SettingKey;
  receipt?: RowReceipt | null;
  failureHint?: string;
}) {
  return (
    <div className="sync-settings-row" id={setting ? rowId(setting) : undefined}>
      <div className="sync-settings-copy">
        <strong>{label}</strong>
        <p>{children}</p>
      </div>
      <div className="sync-settings-control">
        {setting && <SettingReceipt receipt={receipt} failureHint={failureHint} />}
        {control}
      </div>
    </div>
  );
}

/**
 * A boolean setting as a drawn switch with its state in words beside it. The
 * word is aria-hidden: role="switch" already announces On or Off.
 */
function SettingSwitch({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="sync-settings-toggle">
      <input
        type="checkbox"
        role="switch"
        className="sync-settings-switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        aria-label={label}
      />
      <span className="sync-settings-state" aria-hidden="true">{checked ? "On" : "Off"}</span>
    </label>
  );
}

/** What the App version row says. A failed check says nothing: the time
 * only moves when a check reached GitHub, so a stale time is the signal. */
export function appVersionLine(
  appVersion: string | null,
  updates: Pick<AppUpdates, "readyVersion" | "lastCheckedAt">,
): string {
  const installed = appVersion ? `Mallard ${appVersion}` : "Mallard";
  if (updates.readyVersion) {
    return `Mallard ${updates.readyVersion} is ready to install.${appVersion ? ` You have ${appVersion}.` : ""}`;
  }
  if (updates.lastCheckedAt) {
    return `${installed} is the newest release. Checked ${formatRelativeTime(updates.lastCheckedAt)}.`;
  }
  return `${installed}.`;
}

/** A boolean preference as a switch; state lives in preferences.ts. */
function PreferenceSwitch({
  name,
  label,
  onSaved,
}: {
  name: BooleanPreferenceName;
  label: string;
  onSaved: (saved: boolean) => void;
}) {
  const value = usePreference(name);
  return (
    <SettingSwitch
      label={label}
      checked={value}
      onChange={(checked) => onSaved(setPreference(name, checked))}
    />
  );
}

/**
 * The bar's status line: the rule, the latest receipt, then when the visit
 * last saved. It is titlebar chrome, so its static parts carry the drag
 * attribute like the rest of the bar. A failure turns it into a button that
 * finds the row.
 */
function SettingsStatus({ line, onFindFailure }: { line: StatusLine; onFindFailure: () => void }) {
  if (line.tone === "failed") {
    return (
      <span className="sync-settings-status" data-state="failed" data-tauri-drag-region>
        <button
          type="button"
          className="sync-settings-status-jump"
          onClick={onFindFailure}
          title="Show the setting that didn’t save"
        >
          <Icon name="alert-triangle" size={12} />
          <span>{line.text}</span>
        </button>
      </span>
    );
  }
  return (
    <span className="sync-settings-status" data-state={line.tone} data-tauri-drag-region>
      {line.tone === "saving" && <Icon name="refresh" size={12} className="icon-spin" />}
      {(line.tone === "saved" || line.tone === "settled") && <Icon name="check" size={12} />}
      <span data-tauri-drag-region>{line.text}</span>
      {/* The flex gap draws the space; this one is for anything that reads
          the line as text. */}
      {line.at !== null && " "}
      {line.at !== null && (
        <time dateTime={new Date(line.at).toISOString()} data-tauri-drag-region>
          {new Date(line.at).toLocaleTimeString([], clockOptions())}
        </time>
      )}
    </span>
  );
}

export default function SettingsPage({ appActions, appUpdates, onLogsChanged, onDone, doneTarget }: Props) {
  const themeChoice = useThemeChoice();
  const receipts = useSettingsReceipts();
  const { version: appVersion, supported: updatesSupported } = useInstalledVersion();
  const [stats, setStats] = useState<ActivityLogStats | null>(null);
  const [logsBusy, setLogsBusy] = useState(false);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [statsLoading, setStatsLoading] = useState(true);
  const [logsNotice, setLogsNotice] = useState<string | null>(null);
  const [pendingPolicy, setPendingPolicy] = useState<Partial<ActivityLogPolicy> | null>(null);
  const [tuning, setTuning] = useState<AppSettings | null>(null);
  const [tuningBusy, setTuningBusy] = useState(false);
  const [tuningError, setTuningError] = useState<string | null>(null);
  const [tuningSection, setTuningSection] = useState<"privacy" | "sync" | "performance">("privacy");
  const [tuningLoading, setTuningLoading] = useState(true);
  const [pendingTuning, setPendingTuning] = useState<Partial<AppSettings> | null>(null);
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

  /** Local preferences write synchronously; the receipt is the write's own answer. */
  const settleLocal = (key: LocalKey, saved: boolean) => {
    receipts.settle(key, saved, `${SETTING_LABELS[key]} ${saved ? "saved" : "not saved"}`);
  };

  /** Selects apply on change; the returned stats keep the readout current. */
  const applyPolicy = async (patch: Partial<ActivityLogPolicy>) => {
    if (!stats) return;
    const key = Object.keys(patch)[0] as PolicyKey;
    setLogsBusy(true);
    setLogsFromAction(true);
    setLogsError(null);
    setLogsNotice(null);
    setPendingPolicy(patch);
    receipts.begin(key, POLICY_KEYS);
    try {
      setStats(await projectSyncApi.updateActivityLogPolicy({ ...stats.policy, ...patch }));
      receipts.settle(key, true, `${SETTING_LABELS[key]} saved`);
    } catch (reason) {
      setLogsError(errorMessage(reason));
      // The group's alert announces the failure; the status region stays quiet.
      receipts.settle(key, false, null);
    } finally {
      setPendingPolicy(null);
      setLogsBusy(false);
    }
  };

  /** Selects apply on change; the returned settings are the saved truth. */
  const applyTuning = async (patch: Partial<AppSettings>) => {
    if (!tuning) return;
    const key = Object.keys(patch)[0] as TuningKey;
    setTuningBusy(true);
    setTuningSection("credential_redaction_enabled" in patch || "update_check_enabled" in patch ? "privacy"
      : "push_codex_children" in patch || "push_claude_children" in patch ? "sync" : "performance");
    setTuningFromAction(true);
    setTuningError(null);
    setPendingTuning(patch);
    receipts.begin(key, TUNING_KEYS);
    try {
      setTuning(await projectSyncApi.updateAppSettings({ ...tuning, ...patch }));
      receipts.settle(key, true, `${SETTING_LABELS[key]} saved`);
    } catch (reason) {
      setTuningError(errorMessage(reason));
      // The section's alert announces the failure; the status region stays quiet.
      receipts.settle(key, false, null);
    } finally {
      setPendingTuning(null);
      setTuningBusy(false);
    }
  };

  /** The red status line finds the row that did not save: scroll it into view
      and hand its control the focus when one can take it. */
  const findFailure = () => {
    const [key] = receipts.state.failed;
    const row = key ? document.getElementById(rowId(key as SettingKey)) : null;
    if (!row) return;
    row.scrollIntoView({ block: "center", behavior: "smooth" });
    row.querySelector<HTMLElement>("input:not(:disabled), select:not(:disabled), button:not(:disabled)")
      ?.focus({ preventScroll: true });
  };

  const cleanupLogs = async (deleteAll: boolean) => {
    if (!stats) return;
    const approved = await confirm(
      deleteAll
        ? "Delete all retained activity logs?\n\nMallard configuration, backups, local files, and cloud versions will not be changed."
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
      `Permanently delete all of Mallard's local data and restart?\n\nEverything in ~/.mallard is deleted${sizeLine} — sync metadata, activity logs, and the only local backups of files Pull has replaced — and Mallard's settings return to their defaults. This cannot be undone.\n\nCloud versions and your project files are not touched. Mallard will restart as a fresh install.`,
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
  // While a write runs, its control shows the value being written; a failure
  // drops the pending patch and the control returns to the stored value.
  const shownTuning = tuning && pendingTuning ? { ...tuning, ...pendingTuning } : tuning;
  const shownPolicy = stats ? { ...stats.policy, ...(pendingPolicy ?? {}) } : null;
  const receiptFor = (key: SettingKey) => rowReceipt(receipts.state, key);
  const announcement = receipts.state.announcement;

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

  /* One thing to say: a load or apply failure. An apply's receipt is the
     row's stamp and the bar's status line, not a box. */
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
        actions={(
          <>
            <SettingsStatus line={statusLine(receipts.state)} onFindFailure={findFailure} />
            <button
              type="button"
              className="sync-path-sync"
              onClick={onDone}
              title={`Close Settings — back to ${doneTarget}`}
            >
              Done
            </button>
          </>
        )}
        overflow={appActions}
        overflowLabel="App actions"
        ariaLabel="Settings path"
      />
      <main className="sync-main sync-project-links-page sync-settings-page">
        {/* One polite region speaks for every stamp. Keyed by the save, so the
            same sentence twice is still two announcements. */}
        <p className="sync-visually-hidden" role="status">
          {announcement && <span key={announcement.seq}>{announcement.text}</span>}
        </p>
        <section className="profile-links-section" aria-label="Settings">

          <section className="sync-settings-group" aria-labelledby="settings-appearance">
            <h2 className="settings-section-title" id="settings-appearance">
              <SettingsSectionIcon name="appearance" /> Appearance
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Theme"
                setting="theme"
                receipt={receiptFor("theme")}
                failureHint={LOCAL_WRITE_REFUSED}
                control={(
                  <div className="sync-settings-segment" role="radiogroup" aria-label="Theme">
                    {THEME_CHOICES.map((choice) => (
                      <button
                        key={choice.value}
                        type="button"
                        role="radio"
                        aria-checked={themeChoice === choice.value}
                        className={themeChoice === choice.value ? "active" : undefined}
                        onClick={() => {
                          if (themeChoice === choice.value) return;
                          settleLocal("theme", setThemeChoice(choice.value));
                        }}
                      >
                        {choice.label}
                      </button>
                    ))}
                  </div>
                )}
              >
                System uses your macOS theme.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-privacy">
            <h2 className="settings-section-title" id="settings-privacy">
              <SettingsSectionIcon name="privacy" /> Privacy
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Credential redaction"
                setting="credential_redaction_enabled"
                receipt={receiptFor("credential_redaction_enabled")}
                control={(
                  <SettingSwitch
                    label="Credential redaction"
                    checked={shownTuning?.credential_redaction_enabled ?? false}
                    disabled={tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ credential_redaction_enabled: checked })}
                  />
                )}
              >
                Hide detected credentials in pushed sessions. Changing it re-uploads synced
                sessions on the next Push. Earlier copies stay in storage history.
              </SettingsRow>
              <SettingsRow
                label="Automatic updates"
                setting="update_check_enabled"
                receipt={receiptFor("update_check_enabled")}
                control={(
                  <SettingSwitch
                    label="Automatic updates"
                    checked={shownTuning?.update_check_enabled ?? true}
                    disabled={tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ update_check_enabled: checked })}
                  />
                )}
              >
                Check GitHub at startup and every {AUTO_CHECK_INTERVAL_HOURS} hours. A new release
                downloads in the background and installs when you restart or quit Mallard.
                Sends nothing about you or your projects.
              </SettingsRow>
              <SettingsRow
                label="App version"
                control={appUpdates.readyVersion ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={appUpdates.operationRunning || appUpdates.restarting}
                    onClick={appUpdates.restart}
                    title={appUpdates.operationRunning ? "Finish the current operation first" : undefined}
                  >
                    <Icon name="refresh" size={14} />
                    {appUpdates.restarting ? "Restarting…" : "Restart to update"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    disabled={!updatesSupported || appUpdates.checking}
                    onClick={appUpdates.checkNow}
                    title={updatesSupported ? "Check GitHub for a newer Mallard release" : "Update checks are available in the desktop app"}
                  >
                    {appUpdates.checking ? "Checking…" : "Check for updates"}
                  </button>
                )}
              >
                {appVersionLine(appVersion, appUpdates)}
              </SettingsRow>
            </div>
            {tuningSection === "privacy" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-sync">
            <h2 className="settings-section-title" id="settings-sync">
              <SettingsSectionIcon name="sync" /> Sync
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Push Codex subagents"
                setting="push_codex_children"
                receipt={receiptFor("push_codex_children")}
                control={
                  <SettingSwitch
                    label="Push Codex subagents"
                    checked={shownTuning?.push_codex_children ?? false}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ push_codex_children: checked })}
                  />
                }
              >
                Include subagent conversations. Changing it re-uploads synced Codex sessions
                on the next Push.
              </SettingsRow>
              <SettingsRow
                label="Push Claude subagents"
                setting="push_claude_children"
                receipt={receiptFor("push_claude_children")}
                control={
                  <SettingSwitch
                    label="Push Claude subagents"
                    checked={shownTuning?.push_claude_children ?? false}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ push_claude_children: checked })}
                  />
                }
              >
                Include subagent conversations. Changing it re-uploads synced Claude sessions
                on the next Push.
              </SettingsRow>
              <SettingsRow
                label="Open Logs on Push and Pull"
                setting="openLogsOnSync"
                receipt={receiptFor("openLogsOnSync")}
                failureHint={LOCAL_WRITE_REFUSED}
                control={(
                  <PreferenceSwitch
                    name="openLogsOnSync"
                    label="Open Logs on Push and Pull"
                    onSaved={(saved) => settleLocal("openLogsOnSync", saved)}
                  />
                )}
              >
                Open Logs when Push or Pull starts.
              </SettingsRow>
              <SettingsRow
                label="Refresh on window focus"
                setting="refreshOnFocus"
                receipt={receiptFor("refreshOnFocus")}
                failureHint={LOCAL_WRITE_REFUSED}
                control={(
                  <PreferenceSwitch
                    name="refreshOnFocus"
                    label="Refresh on window focus"
                    onSaved={(saved) => settleLocal("refreshOnFocus", saved)}
                  />
                )}
              >
                Refresh projects and storage when you come back to Mallard.
              </SettingsRow>
            </div>
            {tuningSection === "sync" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-performance">
            <h2 className="settings-section-title" id="settings-performance">
              <SettingsSectionIcon name="performance" /> Performance
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Thread size limit"
                setting="conversation_file_limit_bytes"
                receipt={receiptFor("conversation_file_limit_bytes")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning ? Math.round(shownTuning.conversation_file_limit_bytes / (1024 * 1024)) : 100}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({
                        conversation_file_limit_bytes: Number(event.target.value) * 1024 * 1024,
                      })}
                      aria-label="Thread size limit"
                    >
                      {shownTuning && !THREAD_CAP_OPTIONS_MB.includes(Math.round(shownTuning.conversation_file_limit_bytes / (1024 * 1024))) && (
                        <option value={Math.round(shownTuning.conversation_file_limit_bytes / (1024 * 1024))}>
                          {formatBytes(shownTuning.conversation_file_limit_bytes)}
                        </option>
                      )}
                      {THREAD_CAP_OPTIONS_MB.map((size) => <option key={size} value={size}>{size} MB</option>)}
                    </select>
                  </div>
                )}
              >
                Skip transcripts above this limit in compare, Push, and Pull.
              </SettingsRow>
              <SettingsRow
                label="Preview size limit"
                setting="timeline_preview_limit_bytes"
                receipt={receiptFor("timeline_preview_limit_bytes")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning ? Math.round(shownTuning.timeline_preview_limit_bytes / 1024) : 1024}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({
                        timeline_preview_limit_bytes: Number(event.target.value) * 1024,
                      })}
                      aria-label="Preview size limit"
                    >
                      {shownTuning && !PREVIEW_OPTIONS_KB.includes(Math.round(shownTuning.timeline_preview_limit_bytes / 1024)) && (
                        <option value={Math.round(shownTuning.timeline_preview_limit_bytes / 1024)}>
                          {formatBytes(shownTuning.timeline_preview_limit_bytes)}
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
                Limit message and tool call previews. Full transcripts stay unchanged.
              </SettingsRow>
              <SettingsRow
                label="Comparison workers"
                setting="comparison_workers"
                receipt={receiptFor("comparison_workers")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning?.comparison_workers ?? 4}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ comparison_workers: Number(event.target.value) })}
                      aria-label="Comparison workers"
                    >
                      {shownTuning && !WORKER_OPTIONS.includes(shownTuning.comparison_workers) && (
                        <option value={shownTuning.comparison_workers}>
                          {shownTuning.comparison_workers} worker{shownTuning.comparison_workers === 1 ? "" : "s"}
                        </option>
                      )}
                      {WORKER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{count} worker{count === 1 ? "" : "s"}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                Parallel session comparisons, limited by CPU cores.
              </SettingsRow>
              <SettingsRow
                label="Cloud transfers"
                setting="cloud_transfer_concurrency"
                receipt={receiptFor("cloud_transfer_concurrency")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning?.cloud_transfer_concurrency ?? 6}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ cloud_transfer_concurrency: Number(event.target.value) })}
                      aria-label="Parallel cloud transfers"
                    >
                      {shownTuning && !TRANSFER_OPTIONS.includes(shownTuning.cloud_transfer_concurrency) && (
                        <option value={shownTuning.cloud_transfer_concurrency}>
                          {shownTuning.cloud_transfer_concurrency} transfer{shownTuning.cloud_transfer_concurrency === 1 ? "" : "s"}
                        </option>
                      )}
                      {TRANSFER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{count} transfer{count === 1 ? "" : "s"}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                Parallel Push and Pull transfers. Use fewer on slow or metered connections.
              </SettingsRow>
            </div>
            {tuningSection === "performance" && <NoticeZone items={tuningNotices} subject="notes about app settings" {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-logs">
            <h2 className="settings-section-title" id="settings-logs">
              <SettingsSectionIcon name="logs" /> Logs
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="Keep events for"
                setting="retention_days"
                receipt={receiptFor("retention_days")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownPolicy?.retention_days ?? 30}
                      disabled={logsBusy || !stats}
                      onChange={(event) => void applyPolicy({ retention_days: Number(event.target.value) })}
                      aria-label="Keep log events for"
                    >
                      {shownPolicy && !RETENTION_OPTIONS.includes(shownPolicy.retention_days) && (
                        <option value={shownPolicy.retention_days}>{shownPolicy.retention_days} days</option>
                      )}
                      {RETENTION_OPTIONS.map((days) => <option key={days} value={days}>{days} days</option>)}
                    </select>
                  </div>
                )}
              >
                Automatically delete older events.
              </SettingsRow>
              <SettingsRow
                label="Size cap"
                setting="max_total_bytes"
                receipt={receiptFor("max_total_bytes")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownPolicy ? Math.round(shownPolicy.max_total_bytes / (1024 * 1024)) : 100}
                      disabled={logsBusy || !stats}
                      onChange={(event) => void applyPolicy({
                        max_total_bytes: Number(event.target.value) * 1024 * 1024,
                      })}
                      aria-label="Log size cap"
                    >
                      {shownPolicy && !SIZE_OPTIONS_MB.includes(Math.round(shownPolicy.max_total_bytes / (1024 * 1024))) && (
                        <option value={Math.round(shownPolicy.max_total_bytes / (1024 * 1024))}>
                          {formatBytes(shownPolicy.max_total_bytes)}
                        </option>
                      )}
                      {SIZE_OPTIONS_MB.map((size) => <option key={size} value={size}>{size.toLocaleString()} MB</option>)}
                    </select>
                  </div>
                )}
              >
                Delete the oldest log files when this limit is exceeded.
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
                Delete expired log events. Keep sync state, backups, and cloud data.
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
              <SettingsSectionIcon name="display" /> Display
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label="24-hour clock"
                setting="clock24"
                receipt={receiptFor("clock24")}
                failureHint={LOCAL_WRITE_REFUSED}
                control={(
                  <PreferenceSwitch
                    name="clock24"
                    label="24-hour clock"
                    onSaved={(saved) => settleLocal("clock24", saved)}
                  />
                )}
              >
                Use 24-hour time throughout Mallard. Off uses the system locale.
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-local-data">
            <h2 className="settings-section-title" id="settings-local-data">
              <SettingsSectionIcon name="local-data" /> Local data
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
                Permanently delete <code>~/.mallard</code> (sync metadata, logs, Pull backups,
                preferences) and restart. Cloud versions and project files stay.
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
