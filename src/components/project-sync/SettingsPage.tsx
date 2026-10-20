import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import type { ParseKeys } from "i18next";
import { confirm, open as openFile } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type {
  ActivityLogPolicy,
  ActivityLogStats,
  AppSettings,
  CliSelectionSource,
  LocalStateSummary,
  ProviderCliStatus,
  ProviderDescriptor,
} from "../../types";
import Icon from "../Icons";
import SettingsSectionIcon from "./SettingsSectionIcon";
import { NoticeZone, type NoticeItem, type NoticeTiming } from "../notice/Notice";
import { clockOptions } from "../../clock";
import { formatBytes } from "../../format";
import SyncPathBar, { type PathBarMenuGroup } from "../sync-review/SyncPathBar";
import {
  clearStoredUiState,
  setLanguageChoice,
  setPreference,
  setThemeChoice,
  useLanguageChoice,
  usePreference,
  useThemeChoice,
  type BooleanPreferenceName,
  type LanguageChoice,
  type ThemeChoice,
  type UiLanguage,
} from "../../preferences";
import { LANGUAGE_NAMES, OFFERED_LANGUAGES, t, Trans, uiLanguage, uiLocale, useTranslation } from "../../i18n";
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

const THEME_CHOICES: Array<{ value: ThemeChoice; label: ParseKeys<"settings"> }> = [
  { value: "system", label: "appearance.system" },
  { value: "light", label: "appearance.light" },
  { value: "dark", label: "appearance.dark" },
];

export interface LanguageOption {
  value: LanguageChoice;
  label: string;
  /** The language the label is written in; System speaks the interface's. */
  lang?: UiLanguage;
}

/** The Language row's choices: System, then each offered language. */
export function languageOptions(offered: readonly UiLanguage[]): LanguageOption[] {
  return [
    { value: "system", label: t("settings:appearance.system") },
    ...offered.map((language) => ({ value: language, label: LANGUAGE_NAMES[language], lang: language })),
  ];
}

/* A new write to a store clears that store's failures, the way the section's
   error clears when its next apply starts. */
const TUNING_KEYS = [
  "credential_redaction_enabled",
  "update_check_enabled",
  "push_codex_children",
  "push_claude_children",
  "codex_cli_path",
  "claude_cli_path",
  "conversation_file_limit_bytes",
  "timeline_preview_limit_bytes",
  "comparison_workers",
  "cloud_transfer_concurrency",
] as const;
const POLICY_KEYS = ["retention_days", "max_total_bytes"] as const;

type TuningKey = (typeof TUNING_KEYS)[number];
type PolicyKey = (typeof POLICY_KEYS)[number];
type LocalKey = "theme" | "language" | BooleanPreferenceName;
type SettingKey = LocalKey | TuningKey | PolicyKey;

/** What the status region calls a setting once it saves: its row's label,
 *  worded when the setting settles. */
const SETTING_LABELS: Record<SettingKey, () => string> = {
  theme: () => t("settings:appearance.theme"),
  language: () => t("settings:appearance.language"),
  credential_redaction_enabled: () => t("settings:privacy.redaction"),
  update_check_enabled: () => t("settings:privacy.updates"),
  push_codex_children: () => t("settings:sync.codexChildren"),
  push_claude_children: () => t("settings:sync.claudeChildren"),
  codex_cli_path: () => t("settings:cli.label", { provider: "Codex" }),
  claude_cli_path: () => t("settings:cli.label", { provider: "Claude" }),
  openLogsOnSync: () => t("settings:sync.openLogs"),
  refreshOnFocus: () => t("settings:sync.refreshOnFocus"),
  conversation_file_limit_bytes: () => t("settings:performance.threadLimit"),
  timeline_preview_limit_bytes: () => t("settings:performance.previewLimit"),
  comparison_workers: () => t("settings:performance.workers"),
  cloud_transfer_concurrency: () => t("settings:performance.transfers"),
  retention_days: () => t("settings:logs.retention"),
  max_total_bytes: () => t("settings:logs.sizeCap"),
  clock24: () => t("settings:display.clock24"),
};

const RECEIPT_TEXT: Record<RowReceipt["kind"], ParseKeys<"common">> = {
  saving: "saving",
  saved: "saved",
  failed: "notSaved",
};

const rowId = (key: SettingKey) => `settings-row-${key}`;

/** The app-settings field that holds a provider's saved CLI path. */
const cliSettingKey = (provider: ProviderDescriptor): TuningKey =>
  `${provider.key}_cli_path` as TuningKey;

const CLI_SOURCE_LABELS: Record<CliSelectionSource, ParseKeys<"settings">> = {
  selected: "cli.custom",
  inherited_path: "cli.inheritedPath",
  shell_path: "cli.shellPath",
  standard_location: "cli.standardLocation",
};

/** The tag beside a CLI row's name: where the CLI it runs came from. */
export function cliSourceLabel(status: ProviderCliStatus | null | undefined): string | null {
  if (!status) return t("settings:cli.detecting");
  if (status.selected_path) return t("settings:cli.custom");
  return status.source ? t(CLI_SOURCE_LABELS[status.source], { ns: "settings" }) : null;
}

/** What an empty CLI path field says: the CLI automatic detection runs,
 *  or, with a path saved, what emptying the field does. */
export function cliPlaceholder(status: ProviderCliStatus | null | undefined, provider: string): string {
  if (!status) return t("settings:cli.detecting");
  if (status.selected_path) return t("settings:cli.emptyMeansAutomatic");
  if (status.launcher) return t("settings:cli.autoDetected", { path: status.launcher });
  return t("settings:cli.notFound", { provider });
}

/** The one line under a CLI path field, loudest fact first: an unsaved
 *  edit, then why nothing runs, then a failed test. A saved path always
 *  offers the way back to automatic. */
export function cliHint(
  status: ProviderCliStatus | null | undefined,
  test: CliTestResult | undefined,
  dirty: boolean,
): { tone: "bad" | null; text: string | null; useAutomatic: boolean } | null {
  if (dirty) return { tone: null, text: t("settings:cli.pressEnter"), useAutomatic: false };
  const useAutomatic = Boolean(status?.selected_path);
  if (status?.error) return { tone: "bad", text: status.error, useAutomatic };
  if (test?.kind === "failed") {
    return { tone: "bad", text: t("settings:cli.testFailed", { message: test.text }), useAutomatic };
  }
  return useAutomatic ? { tone: null, text: null, useAutomatic } : null;
}

export type CliTestResult = { kind: "version" | "failed"; text: string };

function formatOldest(timestamp?: number | null): string | null {
  if (!timestamp) return null;
  return new Date(timestamp).toLocaleDateString(uiLocale(), {
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
  const { t } = useTranslation("common");
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
      {t(RECEIPT_TEXT[receipt.kind])}
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
  const { t } = useTranslation("settings");
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
      <span className="sync-settings-state" aria-hidden="true">{checked ? t("switch.on") : t("switch.off")}</span>
    </label>
  );
}

/**
 * One provider's CLI as a path field: empty means automatic, and the
 * placeholder names what automatic detection runs. Like the project folder
 * field it never saves as it is typed — every prefix of a path is an
 * absolute path too — so Enter saves the field and Browse saves its pick.
 */
function CliPathRow({
  provider,
  setting,
  savedPath,
  status,
  test,
  receipt,
  disabled,
  testDisabled,
  onSave,
  onBrowse,
  onTest,
}: {
  provider: ProviderDescriptor;
  setting: SettingKey;
  /** The saved path as settings hold it; null is automatic. */
  savedPath: string | null;
  status: ProviderCliStatus | null | undefined;
  test: CliTestResult | undefined;
  receipt: RowReceipt | null;
  disabled: boolean;
  testDisabled: boolean;
  onSave: (path: string | null) => void;
  onBrowse: () => void;
  onTest: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const inputId = useId();
  const hintId = `${inputId}-hint`;
  const [draft, setDraft] = useState(savedPath ?? "");
  // A save from Browse or Use automatic replaces the text; a refused save
  // leaves the saved path, and so the typed text, as it was.
  useEffect(() => setDraft(savedPath ?? ""), [savedPath]);
  const typed = draft.trim();
  const dirty = typed !== (savedPath ?? "");
  const source = cliSourceLabel(status);
  const hint = cliHint(status, test, dirty);
  const name = provider.display_name;
  return (
    <div className="sync-settings-row settings-cli-row" id={rowId(setting)}>
      <div className="settings-cli-head">
        <strong>{t("cli.label", { provider: name })}</strong>
        {source && <span className={`settings-cli-source${status?.selected_path ? " custom" : ""}`}>{source}</span>}
        {test?.kind === "version" && (
          <span className="settings-cli-version" title={t("common:version", { version: test.text })}>
            <Icon name="check" size={11} />
            {test.text}
          </span>
        )}
      </div>
      <div className="sync-settings-control">
        <SettingReceipt receipt={receipt} />
        {/* Test runs the saved CLI, so it waits while the field says otherwise. */}
        <button type="button" className="btn" disabled={testDisabled || dirty} onClick={onTest} title={t("cli.testTitle")}>
          {t("cli.test")}
        </button>
      </div>
      <div className="sync-project-folder-field settings-cli-field">
        <label htmlFor={inputId} className="sync-visually-hidden">{t("cli.pathLabel", { provider: name })}</label>
        <div className="sync-profile-select-row">
          <input
            id={inputId}
            value={draft}
            placeholder={cliPlaceholder(status, name)}
            // Read-only rather than disabled while a save runs, so Enter
            // keeps the focus in the field.
            readOnly={disabled}
            spellCheck={false}
            autoCapitalize="none"
            autoCorrect="off"
            autoComplete="off"
            aria-invalid={hint?.tone === "bad" || undefined}
            aria-describedby={hint ? hintId : undefined}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              if (dirty && !disabled) onSave(typed || null);
            }}
          />
          <button type="button" className="btn" disabled={disabled} onClick={onBrowse} title={t("cli.browseTitle", { provider: name })}>
            <Icon name="folder" size={13} /> {t("cli.browse")}
          </button>
        </div>
        {hint && (
          /* Polite: it changes as the field is typed in. */
          <p id={hintId} className={`sync-setup-path-hint${hint.tone ? ` ${hint.tone}` : ""}`} aria-live="polite">
            {hint.text}
            {hint.useAutomatic && (
              <button type="button" className="btn-link" disabled={disabled} onClick={() => onSave(null)}>
                {t("cli.useAutomatic")}
              </button>
            )}
          </p>
        )}
      </div>
    </div>
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
    return appVersion
      ? t("settings:version.readyHave", { ready: updates.readyVersion, installed: appVersion })
      : t("settings:version.ready", { ready: updates.readyVersion });
  }
  if (updates.lastCheckedAt) {
    return t("settings:version.newest", { installed, when: formatRelativeTime(updates.lastCheckedAt) });
  }
  return t("settings:version.installed", { installed });
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
  const { t } = useTranslation("settings");
  if (line.tone === "failed") {
    return (
      <span className="sync-settings-status" data-state="failed" data-tauri-drag-region>
        <button
          type="button"
          className="sync-settings-status-jump"
          onClick={onFindFailure}
          title={t("status.findFailure")}
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
          {new Date(line.at).toLocaleTimeString(uiLocale(), clockOptions())}
        </time>
      )}
    </span>
  );
}

export default function SettingsPage({ appActions, appUpdates, onLogsChanged, onDone, doneTarget }: Props) {
  const { t } = useTranslation(["settings", "common"]);
  const themeChoice = useThemeChoice();
  const languageChoice = useLanguageChoice();
  /* Local preferences have no section error to explain a refused write, so
     the stamp carries the reason. */
  const localWriteRefused = t("receipt.refused");
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
  const [tuningSection, setTuningSection] = useState<"privacy" | "sync" | "cli" | "performance">("privacy");
  /* The CLI rows: one per provider that declares a command-line tool, each
     with what discovery resolves right now and the last Test's answer. The
     status may wait on one shell probe per app process; the row says
     Detecting… meanwhile. */
  const [cliProviders, setCliProviders] = useState<ProviderDescriptor[] | null>(null);
  const [cliStatus, setCliStatus] = useState<Record<string, ProviderCliStatus | null>>({});
  const [cliTests, setCliTests] = useState<Record<string, CliTestResult>>({});
  const [cliBusy, setCliBusy] = useState(false);
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

  const inspectCli = useCallback(async (provider: ProviderDescriptor) => {
    setCliStatus((current) => ({ ...current, [provider.key]: null }));
    try {
      const status = await projectSyncApi.inspectProviderCli(provider.key);
      setCliStatus((current) => ({ ...current, [provider.key]: status }));
    } catch (reason) {
      setCliStatus((current) => ({
        ...current,
        [provider.key]: {
          provider: provider.key,
          selected_path: null,
          launcher: null,
          executable: null,
          source: null,
          error: errorMessage(reason),
        },
      }));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const providers = (await projectSyncApi.listAgentProviders()).filter((provider) => provider.has_cli);
        if (cancelled) return;
        setCliProviders(providers);
        for (const provider of providers) void inspectCli(provider);
      } catch (reason) {
        if (!cancelled) {
          setCliProviders([]);
          setTuningError(errorMessage(reason));
        }
      }
    })();
    return () => { cancelled = true; };
  }, [inspectCli]);

  /** Settings stores the path as typed or picked, null for automatic;
      discovery re-reads it on the next action. */
  const saveCliPath = async (provider: ProviderDescriptor, path: string | null) => {
    await applyTuning({ [cliSettingKey(provider)]: path });
    setCliTests((current) => { const next = { ...current }; delete next[provider.key]; return next; });
    void inspectCli(provider);
  };

  const chooseCli = async (provider: ProviderDescriptor) => {
    const status = cliStatus[provider.key];
    const picked = await openFile({
      multiple: false,
      directory: false,
      title: t("cli.pickerTitle", { provider: provider.display_name }),
      // Opens beside the CLI in use: the panel hides dot-folders such as ~/.nvm.
      defaultPath: status?.selected_path ?? status?.launcher ?? undefined,
    });
    if (typeof picked !== "string") return;
    await saveCliPath(provider, picked);
  };

  /** Only `--version`, against throwaway homes; the version shows only after this. */
  const testCli = async (provider: ProviderDescriptor) => {
    setCliBusy(true);
    try {
      const version = await projectSyncApi.testProviderCli(provider.key);
      setCliTests((current) => ({ ...current, [provider.key]: { kind: "version", text: version } }));
    } catch (reason) {
      setCliTests((current) => ({ ...current, [provider.key]: { kind: "failed", text: errorMessage(reason) } }));
    } finally {
      setCliBusy(false);
    }
  };

  const refreshCliDetection = async () => {
    setCliBusy(true);
    setTuningSection("cli");
    setTuningFromAction(true);
    setTuningError(null);
    try {
      await projectSyncApi.refreshCliDetection();
      setCliTests({});
      for (const provider of cliProviders ?? []) void inspectCli(provider);
    } catch (reason) {
      setTuningError(errorMessage(reason));
    } finally {
      setCliBusy(false);
    }
  };

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
    const label = SETTING_LABELS[key]();
    // A language change takes effect before this handler's next render.
    const values = { label, lng: uiLanguage() };
    receipts.settle(key, saved, saved ? t("receipt.saved", values) : t("receipt.notSaved", values));
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
      receipts.settle(key, true, t("receipt.saved", { label: SETTING_LABELS[key]() }));
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
      : "push_codex_children" in patch || "push_claude_children" in patch ? "sync"
        : "codex_cli_path" in patch || "claude_cli_path" in patch ? "cli" : "performance");
    setTuningFromAction(true);
    setTuningError(null);
    setPendingTuning(patch);
    receipts.begin(key, TUNING_KEYS);
    try {
      setTuning(await projectSyncApi.updateAppSettings({ ...tuning, ...patch }));
      receipts.settle(key, true, t("receipt.saved", { label: SETTING_LABELS[key]() }));
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
        ? t("logs.deleteAllMessage")
        : t("logs.cleanUpMessage", {
          count: stats.file_count,
          days: stats.policy.retention_days,
          size: formatBytes(stats.total_bytes),
        }),
      deleteAll
        ? { title: t("logs.deleteAllTitle"), kind: "warning", okLabel: t("logs.deleteAllOk"), cancelLabel: t("common:cancel") }
        : { title: t("logs.cleanUpTitle"), kind: "warning", okLabel: t("logs.cleanUpOk"), cancelLabel: t("common:cancel") },
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
          ? t("logs.deleted")
          : result.removed_files
            ? t("logs.removed", { count: result.removed_files, size: formatBytes(result.reclaimed_bytes) })
            : t("logs.nothingRemoved"),
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
    const message = localSummary
      ? t("reset.messageSized", {
        count: localSummary.file_count,
        total: formatBytes(localSummary.total_bytes),
        backups: formatBytes(localSummary.backups_bytes),
      })
      : t("reset.message");
    const approved = await confirm(
      message,
      { title: t("reset.title"), kind: "warning", okLabel: t("reset.ok"), cancelLabel: t("common:cancel") },
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
      message: stats ? logsError : t("logs.readFailed", { error: logsError }),
      action: stats ? undefined : (
        <button type="button" className="btn" disabled={statsLoading} onClick={() => void loadStats(true)}>
          <Icon name="refresh" size={14} className={statsLoading ? "icon-spin" : undefined} />
          {statsLoading ? t("retrying") : t("retry")}
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
    ? { timing: "standing", label: t("logs.problems") }
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
      message: tuning ? tuningError : t("tuning.readFailed", { error: tuningError }),
      action: tuning ? undefined : (
        <button type="button" className="btn" disabled={tuningLoading} onClick={() => void loadTuning(true)}>
          <Icon name="refresh" size={14} className={tuningLoading ? "icon-spin" : undefined} />
          {tuningLoading ? t("retrying") : t("retry")}
        </button>
      ),
    });
  }
  /* Same reading as the Logs group: the mount load's failure is standing
     content; after a Retry or a select it is an answer. */
  const tuningTiming: NoticeTiming = !tuningFromAction
    ? { timing: "standing", label: t("tuning.problems") }
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
          {summaryLoading ? t("retrying") : t("retry")}
        </button>
      ),
    });
  }
  if (resetPhase === "reloading") {
    localDataNotices.push({
      key: "local-data-reloading",
      severity: "success",
      message: t("reset.deleted"),
    });
  }
  /* Same reading as the Logs group: the measurement walk runs on mount, so its
     failure is standing content, while a failed Reset — or a retried walk — is
     an event this box has to announce. */
  const localDataTiming: NoticeTiming = !localDataFromAction
    ? { timing: "standing", label: t("reset.problems") }
    : resetError
      ? { timing: "blocking" }
      : { timing: "progress" };

  return (
    <>
      <SyncPathBar
        crumbs={[{ id: "settings", label: t("common:settings"), icon: "settings", tone: "root" }]}
        actions={(
          <>
            <SettingsStatus line={statusLine(receipts.state)} onFindFailure={findFailure} />
            <button
              type="button"
              className="sync-path-sync"
              onClick={onDone}
              title={t("page.doneTitle", { target: doneTarget })}
            >
              {t("page.done")}
            </button>
          </>
        )}
        overflow={appActions}
        overflowLabel={t("page.appActions")}
        ariaLabel={t("page.path")}
      />
      <main className="sync-main sync-project-links-page sync-settings-page">
        {/* One polite region speaks for every stamp. Keyed by the save, so the
            same sentence twice is still two announcements. */}
        <p className="sync-visually-hidden" role="status">
          {announcement && <span key={announcement.seq}>{announcement.text}</span>}
        </p>
        <section className="profile-links-section" aria-label={t("common:settings")}>

          <section className="sync-settings-group" aria-labelledby="settings-appearance">
            <h2 className="settings-section-title" id="settings-appearance">
              <SettingsSectionIcon name="appearance" /> {t("sections.appearance")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("appearance.theme")}
                setting="theme"
                receipt={receiptFor("theme")}
                failureHint={localWriteRefused}
                control={(
                  <div className="sync-settings-segment" role="radiogroup" aria-label={t("appearance.theme")}>
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
                        {t(choice.label)}
                      </button>
                    ))}
                  </div>
                )}
              >
                {t("appearance.themeHint")}
              </SettingsRow>
              {/* A choice only once there is more than one language to make it
                  between; until then the row would offer English or English. */}
              {OFFERED_LANGUAGES.length > 1 && (
                <SettingsRow
                  label={t("appearance.language")}
                  setting="language"
                  receipt={receiptFor("language")}
                  failureHint={localWriteRefused}
                  control={(
                    <div className="sync-settings-segment" role="radiogroup" aria-label={t("appearance.language")}>
                      {languageOptions(OFFERED_LANGUAGES).map((choice) => (
                        <button
                          key={choice.value}
                          type="button"
                          role="radio"
                          lang={choice.lang}
                          aria-checked={languageChoice === choice.value}
                          className={languageChoice === choice.value ? "active" : undefined}
                          onClick={() => {
                            if (languageChoice === choice.value) return;
                            settleLocal("language", setLanguageChoice(choice.value));
                          }}
                        >
                          {choice.label}
                        </button>
                      ))}
                    </div>
                  )}
                >
                  {t("appearance.languageHint")}
                </SettingsRow>
              )}
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-privacy">
            <h2 className="settings-section-title" id="settings-privacy">
              <SettingsSectionIcon name="privacy" /> {t("sections.privacy")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("privacy.redaction")}
                setting="credential_redaction_enabled"
                receipt={receiptFor("credential_redaction_enabled")}
                control={(
                  <SettingSwitch
                    label={t("privacy.redaction")}
                    checked={shownTuning?.credential_redaction_enabled ?? false}
                    disabled={tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ credential_redaction_enabled: checked })}
                  />
                )}
              >
                {t("privacy.redactionHint")}
              </SettingsRow>
              <SettingsRow
                label={t("privacy.updates")}
                setting="update_check_enabled"
                receipt={receiptFor("update_check_enabled")}
                control={(
                  <SettingSwitch
                    label={t("privacy.updates")}
                    checked={shownTuning?.update_check_enabled ?? true}
                    disabled={tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ update_check_enabled: checked })}
                  />
                )}
              >
                {t("privacy.updatesHint", { hours: AUTO_CHECK_INTERVAL_HOURS })}
              </SettingsRow>
              <SettingsRow
                label={t("version.label")}
                control={appUpdates.readyVersion ? (
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={appUpdates.operationRunning || appUpdates.restarting}
                    onClick={appUpdates.restart}
                    title={appUpdates.operationRunning ? t("version.busy") : undefined}
                  >
                    <Icon name="refresh" size={14} />
                    {appUpdates.restarting ? t("version.restarting") : t("version.restart")}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    disabled={!updatesSupported || appUpdates.checking}
                    onClick={appUpdates.checkNow}
                    title={updatesSupported ? t("version.checkTitle") : t("version.desktopOnly")}
                  >
                    {appUpdates.checking ? t("version.checking") : t("version.check")}
                  </button>
                )}
              >
                {appVersionLine(appVersion, appUpdates)}
              </SettingsRow>
            </div>
            {tuningSection === "privacy" && <NoticeZone items={tuningNotices} subject={t("tuning.subject")} {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-sync">
            <h2 className="settings-section-title" id="settings-sync">
              <SettingsSectionIcon name="sync" /> {t("sections.sync")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("sync.codexChildren")}
                setting="push_codex_children"
                receipt={receiptFor("push_codex_children")}
                control={
                  <SettingSwitch
                    label={t("sync.codexChildren")}
                    checked={shownTuning?.push_codex_children ?? false}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ push_codex_children: checked })}
                  />
                }
              >
                {t("sync.codexChildrenHint")}
              </SettingsRow>
              <SettingsRow
                label={t("sync.claudeChildren")}
                setting="push_claude_children"
                receipt={receiptFor("push_claude_children")}
                control={
                  <SettingSwitch
                    label={t("sync.claudeChildren")}
                    checked={shownTuning?.push_claude_children ?? false}
                    disabled={tuningLoading || tuningBusy || !tuning}
                    onChange={(checked) => void applyTuning({ push_claude_children: checked })}
                  />
                }
              >
                {t("sync.claudeChildrenHint")}
              </SettingsRow>
              <SettingsRow
                label={t("sync.openLogs")}
                setting="openLogsOnSync"
                receipt={receiptFor("openLogsOnSync")}
                failureHint={localWriteRefused}
                control={(
                  <PreferenceSwitch
                    name="openLogsOnSync"
                    label={t("sync.openLogs")}
                    onSaved={(saved) => settleLocal("openLogsOnSync", saved)}
                  />
                )}
              >
                {t("sync.openLogsHint")}
              </SettingsRow>
              <SettingsRow
                label={t("sync.refreshOnFocus")}
                setting="refreshOnFocus"
                receipt={receiptFor("refreshOnFocus")}
                failureHint={localWriteRefused}
                control={(
                  <PreferenceSwitch
                    name="refreshOnFocus"
                    label={t("sync.refreshOnFocus")}
                    onSaved={(saved) => settleLocal("refreshOnFocus", saved)}
                  />
                )}
              >
                {t("sync.refreshOnFocusHint")}
              </SettingsRow>
            </div>
            {tuningSection === "sync" && <NoticeZone items={tuningNotices} subject={t("tuning.subject")} {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-cli">
            <h2 className="settings-section-title" id="settings-cli">
              <SettingsSectionIcon name="cli" /> {t("sections.cli")}
            </h2>
            <div className="sync-settings-card">
              {(cliProviders ?? []).map((provider) => {
                const key = cliSettingKey(provider);
                const status = cliStatus[provider.key];
                // The saved value, not the pending one: a refused save must
                // leave the typed text in the field.
                const saved = tuning?.[key];
                return (
                  <CliPathRow
                    key={provider.key}
                    provider={provider}
                    setting={key}
                    savedPath={typeof saved === "string" ? saved : null}
                    status={status}
                    test={cliTests[provider.key]}
                    receipt={receiptFor(key)}
                    disabled={tuningBusy || !tuning}
                    testDisabled={cliBusy || !status?.executable}
                    onSave={(path) => void saveCliPath(provider, path)}
                    onBrowse={() => void chooseCli(provider)}
                    onTest={() => void testCli(provider)}
                  />
                );
              })}
              <SettingsRow
                label={t("cli.detection")}
                control={(
                  <button
                    type="button"
                    className="btn"
                    disabled={cliBusy}
                    onClick={() => void refreshCliDetection()}
                  >
                    <Icon name="refresh" size={14} />
                    {t("cli.refresh")}
                  </button>
                )}
              >
                {t("cli.detectionHint")}
              </SettingsRow>
            </div>
            {tuningSection === "cli" && <NoticeZone items={tuningNotices} subject={t("tuning.subject")} {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-performance">
            <h2 className="settings-section-title" id="settings-performance">
              <SettingsSectionIcon name="performance" /> {t("sections.performance")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("performance.threadLimit")}
                setting="conversation_file_limit_bytes"
                receipt={receiptFor("conversation_file_limit_bytes")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning ? Math.round(shownTuning.conversation_file_limit_bytes / (1024 * 1024)) : 200}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({
                        conversation_file_limit_bytes: Number(event.target.value) * 1024 * 1024,
                      })}
                      aria-label={t("performance.threadLimit")}
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
                {t("performance.threadLimitHint")}
              </SettingsRow>
              <SettingsRow
                label={t("performance.previewLimit")}
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
                      aria-label={t("performance.previewLimit")}
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
                {t("performance.previewLimitHint")}
              </SettingsRow>
              <SettingsRow
                label={t("performance.workers")}
                setting="comparison_workers"
                receipt={receiptFor("comparison_workers")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning?.comparison_workers ?? 4}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ comparison_workers: Number(event.target.value) })}
                      aria-label={t("performance.workers")}
                    >
                      {shownTuning && !WORKER_OPTIONS.includes(shownTuning.comparison_workers) && (
                        <option value={shownTuning.comparison_workers}>
                          {t("performance.workerCount", { count: shownTuning.comparison_workers })}
                        </option>
                      )}
                      {WORKER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{t("performance.workerCount", { count })}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                {t("performance.workersHint")}
              </SettingsRow>
              <SettingsRow
                label={t("performance.transfers")}
                setting="cloud_transfer_concurrency"
                receipt={receiptFor("cloud_transfer_concurrency")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownTuning?.cloud_transfer_concurrency ?? 6}
                      disabled={tuningBusy || !tuning}
                      onChange={(event) => void applyTuning({ cloud_transfer_concurrency: Number(event.target.value) })}
                      aria-label={t("performance.transfersLabel")}
                    >
                      {shownTuning && !TRANSFER_OPTIONS.includes(shownTuning.cloud_transfer_concurrency) && (
                        <option value={shownTuning.cloud_transfer_concurrency}>
                          {t("performance.transferCount", { count: shownTuning.cloud_transfer_concurrency })}
                        </option>
                      )}
                      {TRANSFER_OPTIONS.map((count) => (
                        <option key={count} value={count}>{t("performance.transferCount", { count })}</option>
                      ))}
                    </select>
                  </div>
                )}
              >
                {t("performance.transfersHint")}
              </SettingsRow>
            </div>
            {tuningSection === "performance" && <NoticeZone items={tuningNotices} subject={t("tuning.subject")} {...tuningTiming} />}
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-logs">
            <h2 className="settings-section-title" id="settings-logs">
              <SettingsSectionIcon name="logs" /> {t("sections.logs")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("logs.retention")}
                setting="retention_days"
                receipt={receiptFor("retention_days")}
                control={(
                  <div className="sync-select-shell">
                    <select
                      value={shownPolicy?.retention_days ?? 30}
                      disabled={logsBusy || !stats}
                      onChange={(event) => void applyPolicy({ retention_days: Number(event.target.value) })}
                      aria-label={t("logs.retentionLabel")}
                    >
                      {shownPolicy && !RETENTION_OPTIONS.includes(shownPolicy.retention_days) && (
                        <option value={shownPolicy.retention_days}>{t("logs.days", { count: shownPolicy.retention_days })}</option>
                      )}
                      {RETENTION_OPTIONS.map((days) => <option key={days} value={days}>{t("logs.days", { count: days })}</option>)}
                    </select>
                  </div>
                )}
              >
                {t("logs.retentionHint")}
              </SettingsRow>
              <SettingsRow
                label={t("logs.sizeCap")}
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
                      aria-label={t("logs.sizeCapLabel")}
                    >
                      {shownPolicy && !SIZE_OPTIONS_MB.includes(Math.round(shownPolicy.max_total_bytes / (1024 * 1024))) && (
                        <option value={Math.round(shownPolicy.max_total_bytes / (1024 * 1024))}>
                          {formatBytes(shownPolicy.max_total_bytes)}
                        </option>
                      )}
                      {SIZE_OPTIONS_MB.map((size) => <option key={size} value={size}>{size.toLocaleString(uiLocale())} MB</option>)}
                    </select>
                  </div>
                )}
              >
                {t("logs.sizeCapHint")}
              </SettingsRow>
              <SettingsRow
                label={t("logs.files")}
                control={(
                  <>
                    <span
                      className="sync-settings-readout"
                      title={oldest ? t("logs.oldest", { date: oldest }) : undefined}
                    >
                      {stats ? t("logs.readout", { count: stats.file_count, size: formatBytes(stats.total_bytes) }) : "…"}
                    </span>
                    <button type="button" className="btn" onClick={() => void openLogsFolder()} disabled={logsBusy}>
                      <Icon name="folder" size={12} /> {t("logs.openFolder")}
                    </button>
                  </>
                )}
              >
                <Trans t={t} i18nKey="logs.filesHint" components={{ code: <code /> }} />
              </SettingsRow>
              <SettingsRow
                label={t("logs.cleanUp")}
                control={(
                  <>
                    <button type="button" className="btn" onClick={() => void cleanupLogs(false)} disabled={logsBusy || !stats}>
                      <Icon name="trash" size={12} /> {t("logs.cleanUpNow")}
                    </button>
                    <button
                      type="button"
                      className="btn btn-ghost danger"
                      onClick={() => void cleanupLogs(true)}
                      disabled={logsBusy || !stats || stats.file_count === 0}
                    >
                      {t("logs.deleteAll")}
                    </button>
                  </>
                )}
              >
                {t("logs.cleanUpHint")}
              </SettingsRow>
            </div>
            <NoticeZone
              items={logsNotices}
              subject={t("logs.subject")}
              {...logsTiming}
            />
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-display">
            <h2 className="settings-section-title" id="settings-display">
              <SettingsSectionIcon name="display" /> {t("sections.display")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("display.clock24")}
                setting="clock24"
                receipt={receiptFor("clock24")}
                failureHint={localWriteRefused}
                control={(
                  <PreferenceSwitch
                    name="clock24"
                    label={t("display.clock24")}
                    onSaved={(saved) => settleLocal("clock24", saved)}
                  />
                )}
              >
                {t("display.clock24Hint")}
              </SettingsRow>
            </div>
          </section>

          <section className="sync-settings-group" aria-labelledby="settings-local-data">
            <h2 className="settings-section-title" id="settings-local-data">
              <SettingsSectionIcon name="local-data" /> {t("sections.localData")}
            </h2>
            <div className="sync-settings-card">
              <SettingsRow
                label={t("reset.label")}
                control={(
                  <>
                    <span className="sync-settings-readout" title={localSummary?.root_path}>
                      {localSummary
                        ? t("logs.readout", { count: localSummary.file_count, size: formatBytes(localSummary.total_bytes) })
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
                      {t("reset.label")}
                    </button>
                  </>
                )}
              >
                <Trans t={t} i18nKey="reset.hint" components={{ code: <code /> }} />
              </SettingsRow>
            </div>
            <NoticeZone
              items={localDataNotices}
              subject={t("reset.subject")}
              {...localDataTiming}
            />
          </section>

        </section>
      </main>
    </>
  );
}
