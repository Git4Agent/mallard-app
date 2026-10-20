import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import SettingsPage, { cliHint, cliPlaceholder, cliSourceLabel, languageOptions } from "../../src/components/project-sync/SettingsPage";
import SyncPathBar from "../../src/components/sync-review/SyncPathBar";
import type { AppUpdates } from "../../src/components/AppUpdater";
import { i18n, OFFERED_LANGUAGES } from "../../src/i18n";
import { en } from "../../src/i18n/en";
import {
  getPreference,
  getThemeChoice,
  resolveTheme,
  setPreference,
  subscribePreferences,
} from "../../src/preferences";
import { clockOptions } from "../../src/clock";

const appActions = [{
  label: "Mallard",
  items: [
    { id: "settings", label: "Settings", icon: "settings" as const, onSelect: () => undefined },
    { id: "reload", label: "Reload app data", icon: "computer" as const, onSelect: () => undefined },
  ],
}];

/** The shell's updater with nothing to say: no release ready, no check run. */
const idleUpdates: AppUpdates = {
  readyVersion: null,
  checking: false,
  restarting: false,
  operationRunning: false,
  lastCheckedAt: null,
  receiptVersion: null,
  checkNow: () => undefined,
  restart: () => undefined,
  openReceipt: () => undefined,
};

function renderPage(actions = appActions): string {
  return renderToStaticMarkup(
    <SettingsPage
      appActions={actions}
      appUpdates={idleUpdates}
      onLogsChanged={() => undefined}
      onDone={() => undefined}
      doneTarget="mallard-app"
    />,
  );
}

/** The word printed beside each switch, in page order. */
function switchWords(html: string): string[] {
  return [...html.matchAll(
    /<label class="sync-settings-toggle"><input[^>]*role="switch"[^>]*><span class="sync-settings-state" aria-hidden="true">(On|Off)<\/span><\/label>/g,
  )].map((match) => match[1]);
}

test("the Settings page is a path-bar screen of grouped preference rows", () => {
  const html = renderPage();

  // Path bar: one root crumb naming the page, the app menu in the overflow.
  assert.match(html, /class="sync-path-crumb root leaf"/);
  assert.match(html, /aria-label="Settings path"/);
  assert.match(html, />Settings<\/span>/);
  assert.match(html, /aria-label="App actions"/);

  // Seven groups, in reading order, each heading on the shared chrome class.
  const headings = [...html.matchAll(/class="settings-section-title"[^>]*>.*?<\/svg>([^<]+)</g)]
    .map((match) => match[1].trim());
  assert.deepEqual(headings, ["Appearance", "Privacy", "Sync", "Agent CLIs", "Performance", "Logs", "Display", "Local data"]);

  // Theme: a radiogroup of the three choices; absence of a stored key means
  // System, so it is the checked one under the test's bare environment.
  assert.match(html, /role="radiogroup" aria-label="Theme"/);
  const checked = html.match(/<button[^>]*role="radio"[^>]*aria-checked="true"[^>]*>([A-Za-z]+)</);
  assert.equal(checked?.[1], "System");
  assert.match(html, />Light</);
  assert.match(html, />Dark</);

  // Privacy and Sync default Off, including while saved settings load.
  const switches = html.match(/role="switch"[^>]*class="sync-settings-switch"[^>]*checked=""/g) ?? [];
  assert.equal(switches.length, 2);
  assert.match(html, /<input[^>]*role="switch"[^>]*disabled=""[^>]*aria-label="Automatic updates"[^>]*checked=""/);
  // The manual check sits beside its switch and is inert outside the desktop app.
  assert.match(html, /<button[^>]*disabled=""[^>]*>Check for updates<\/button>/);
  for (const provider of ["Codex", "Claude"]) {
    assert.match(html, new RegExp(`<input[^>]*role="switch"[^>]*disabled=""[^>]*aria-label="Push ${provider} subagents"\\/>`));
  }
  // Each switch says its state in a word after it, inside the label so the
  // word is part of the click target, and aria-hidden because the switch
  // already announces itself.
  assert.deepEqual(switchWords(html), ["Off", "On", "Off", "Off", "Off", "Off", "On"]);

  // Redaction is disabled until saved settings load.
  assert.doesNotMatch(html, /Always on/);
  assert.doesNotMatch(html, /redact\.v1|mallard\.redact/);
  assert.match(html, /<input[^>]*role="switch"[^>]*disabled=""[^>]*aria-label="Credential redaction"\/>/);

  // The Logs group carries the absorbed log-policy surface: both selects on
  // the app's select shell, the folder verb, and both cleanup verbs.
  assert.match(html, /aria-label="Keep log events for"/);
  assert.match(html, /aria-label="Log size cap"/);
  assert.equal((html.match(/class="sync-select-shell"/g) ?? []).length, 6);
  assert.match(html, /> Open folder</);
  assert.match(html, /> Clean up now</);
  assert.match(html, />Delete all</);
  assert.match(html, /~\/.mallard\/logs/);

  // Stats have not loaded in a static render; the selects sit disabled on
  // their defaults rather than inventing values.
  assert.match(html, /<option value="30" selected="">30 days<\/option>/);
  assert.match(html, /<option value="100" selected="">100 MB<\/option>/);

  // Agent CLIs: the provider rows arrive with the provider list, so a static
  // render holds only the detection row — and never a provider process.
  assert.match(html, /aria-labelledby="settings-cli"/);
  assert.match(html, /<button[^>]*>.*?Refresh detection<\/button>/);
  assert.match(html, /then the PATH your shell sets/);
  assert.match(html, /On Windows a changed PATH needs a Mallard restart\./);
  assert.doesNotMatch(html, /Detecting…/);

  // The Performance group: four tuning selects on the same shell. The thread
  // cap defaults to 200 MiB, while the log size cap above stays at 100 MiB.
  assert.match(html, /aria-label="Thread size limit"[\s\S]{0,600}<option value="200" selected="">200 MB<\/option>/);
  assert.match(html, /<option value="400">400 MB<\/option>/);
  assert.match(html, /aria-label="Preview size limit"/);
  assert.match(html, /<option value="1024" selected="">1 MB<\/option>/);
  assert.match(html, /<option value="4" selected="">4 workers<\/option>/);
  assert.match(html, /aria-label="Parallel cloud transfers"/);
  assert.match(html, /<option value="6" selected="">6 transfers<\/option>/);

  // Local data: the reset row is a danger verb with a size readout. In a
  // static render the summary has not loaded — the readout shows an ellipsis
  // and the button is deliberately NOT disabled: a user whose state is too
  // broken to measure is exactly who needs the reset.
  assert.match(html, /class="btn btn-ghost danger"[^>]*>Reset Mallard</);
  assert.doesNotMatch(html, /<button[^>]*class="btn btn-ghost danger"[^>]*disabled[^>]*>Reset Mallard</);
  assert.match(html, /class="sync-settings-readout"[^>]*>…</);
  // The row copy names the irreversible parts: backups and preferences.
  assert.match(html, /Pull backups/);
  assert.match(html, /preferences\) and restart/);
});

test("a switch set off says Off, not the default", () => {
  // The word follows the stored value: with the 24-hour clock stored off, its
  // row reads Off and its input is unchecked.
  const store = new Map([["mallard.clock-24h", "off"]]);
  const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      localStorage: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, value); },
      },
    },
  });
  try {
    const html = renderPage();
    assert.deepEqual(switchWords(html), ["Off", "On", "Off", "Off", "Off", "Off", "Off"]);
    assert.match(html, /<input[^>]*role="switch"[^>]*aria-label="24-hour clock"\/>/);
    for (const name of ["openLogsOnSync", "refreshOnFocus"] as const) {
      assert.equal(getPreference(name), false);
      assert.equal(setPreference(name, true), true);
      assert.equal(getPreference(name), true);
    }
    assert.deepEqual(switchWords(renderPage()), ["Off", "On", "Off", "Off", "On", "On", "Off"]);
    for (const name of ["openLogsOnSync", "refreshOnFocus"] as const) {
      assert.equal(setPreference(name, false), true);
      assert.equal(getPreference(name), false);
    }
  } finally {
    if (previous) Object.defineProperty(globalThis, "window", previous);
    else delete (globalThis as { window?: unknown }).window;
  }
});

test("sync preferences default off, live under the mallard.* keys, and notify", () => {
  // No storage in this environment: every read lands on its default.
  assert.equal(getPreference("openLogsOnSync"), false);
  assert.equal(getPreference("refreshOnFocus"), false);
  assert.equal(getPreference("clock24"), true);
  assert.equal(getThemeChoice(), "system");
  // Without matchMedia, System resolves to the app's historical dark.
  assert.equal(resolveTheme("system"), "dark");

  let notified = 0;
  const unsubscribe = subscribePreferences(() => { notified += 1; });
  setPreference("clock24", false);
  assert.equal(notified, 1);
  unsubscribe();

  const source = readFileSync("src/preferences.ts", "utf8");
  assert.match(source, /"mallard\.theme"/);
  assert.match(source, /"mallard\.open-logs-on-sync"/);
  assert.match(source, /"mallard\.refresh-on-focus"/);
  assert.match(source, /"mallard\.clock-24h"/);
  // Factory reset is a prefix sweep, not an enumerated key list — future
  // mallard.* keys must be covered without anyone remembering this function.
  assert.match(source, /export function clearStoredUiState\(/);
  assert.match(source, /startsWith\("mallard\."\)/);
  assert.match(source, /removeItem/);
});

test("clock options use a padded 24-hour cycle", () => {
  // Default 24-hour: the named h23 cycle with a padded hour, so fixed-width
  // columns stay fixed; `hour12: false` is not a synonym and stays banned.
  assert.deepEqual(clockOptions(), { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  assert.deepEqual(clockOptions(true), {
    hour: "2-digit", minute: "2-digit", hourCycle: "h23", second: "2-digit",
  });
});

test("Reset Mallard wipes ~/.mallard behind a warning confirm and reloads the webview", () => {
  const page = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const reset = readFileSync("src-tauri/src/app_reset.rs", "utf8");
  const lib = readFileSync("src-tauri/src/lib.rs", "utf8");

  // One warning confirm whose copy owns the irreversibility; the cutover is
  // a webview reload after the wipe resolves, with the localStorage sweep in
  // between. Never a process restart: relaunching the raw binary under
  // `tauri dev` orphans the frontend from its dev server (the CLI kills Vite
  // when the original process exits) and lands on a white window.
  assert.match(en.settings.reset.message, /This cannot be undone\./);
  assert.match(en.settings.reset.messageSized_other, /This cannot be undone\./);
  assert.match(page, /kind: "warning", okLabel: t\("reset\.ok"\)/);
  assert.equal(en.settings.reset.ok, "Reset Mallard");
  assert.match(page, /await projectSyncApi\.resetLocalState\(\);[\s\S]{0,900}clearStoredUiState\(\);[\s\S]{0,200}setResetPhase\("reloading"\);[\s\S]{0,200}window\.location\.reload\(\);/);

  assert.match(api, /invoke<LocalStateSummary>\("get_local_state_summary"\)/);
  assert.match(api, /invoke<LocalStateWipeResult>\("reset_local_state"\)/);
  assert.doesNotMatch(api, /restart_app/);

  // The local core guards the wipe. The Tauri command delegates to
  // it, re-arms live state, and logs no persisted success entry that would
  // recreate ~/.mallard/logs. The process survives the reset.
  assert.match(reset, /reset_local_state_with_repositories\(&repository, &catalog\)/);
  assert.match(reset, /acquire_state_guard\(\)\?;[\s\S]*?repository\.wipe_local_state\(\)\?/);
  assert.match(reset, /plan_store::discard_all\(catalog\.root\(\)\)/);
  assert.match(reset, /observation_cache\(\)\.reset\(\);/);
  assert.match(reset, /ConfigWatcher::start\(app\.clone\(\)\)/);
  assert.doesNotMatch(reset, /request_restart|restart_app/);
  assert.doesNotMatch(reset, /local_state_reset_(completed|succeeded)/);
  assert.match(lib, /app_reset::get_local_state_summary,\s*app_reset::reset_local_state,/);
  assert.doesNotMatch(lib, /restart_app/);
});

test("Performance settings persist through the backend tuning document", () => {
  const page = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const commands = readFileSync("src-tauri/src/app_settings.rs", "utf8");
  const document = readFileSync("crates/mallard-infrastructure/src/app_settings.rs", "utf8");
  const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
  const reset = readFileSync("src-tauri/src/app_reset.rs", "utf8");

  // Selects apply on change through one patch-merging handler, like the log
  // policy's, and state keeps the backend's saved read-back.
  assert.match(page, /const applyTuning = async \(patch: Partial<AppSettings>\) => \{/);
  assert.match(page, /projectSyncApi\.updateAppSettings\(\{ \.\.\.tuning, \.\.\.patch \}\)/);

  assert.match(api, /invoke<AppSettings>\("get_app_settings"\)/);
  assert.match(api, /invoke<AppSettings>\("update_app_settings", \{ settings \}\)/);

  // Rust side: one schema-1 document whose every field carries a serde
  // default (a partial file parses), hard-reject validation, and an update
  // that persists, installs, then drops the observations that baked the old
  // limits in — a settings change touches no session file, so their stamps
  // would all still hold.
  assert.match(document, /pub const APP_SETTINGS_SCHEMA: u32 = 1;/);
  assert.match(document, /#\[serde\(default = "default_conversation_file_limit_bytes"\)\]/);
  assert.match(document, /#\[serde\(default = "default_timeline_preview_limit_bytes"\)\]/);
  assert.match(commands, /update_app_settings_core\(&repository, &settings, \|\| \{/);
  assert.match(commands, /save_app_settings\(repository, settings\)\?;\s*install_app_settings\(settings\);/);
  assert.match(commands, /observation_cache\(\)\.reset_and_purge_disk\(repository\.root\(\)\)/);
  assert.match(lib, /app_settings::get_app_settings,\s*app_settings::update_app_settings,/);

  // Startup makes the stored values live before the first command; the
  // factory reset returns the installed atomics to defaults right after the
  // wipe deletes settings.json, since the process survives the reset.
  assert.match(lib, /install_app_settings\(/);
  assert.match(reset, /install_app_settings\(\s*&mallard_infrastructure::app_settings::AppSettings::default\(\)/);
});

test("every export-shaping setting says that changing it re-uploads synced sessions, and Off says what stays behind", () => {
  // H05: the switch is easy, the consequence is not — the next Push republishes
  // every synced session, and revisions are immutable, so copies pushed with
  // redaction off remain readable in the storage's history.
  const source = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  assert.match(en.settings.privacy.redactionHint, /^Hide detected credentials in pushed sessions\. Changing it re-uploads synced sessions on the next Push\. Earlier copies stay in storage history\.$/);
  assert.match(en.settings.sync.codexChildrenHint, /Changing it re-uploads synced Codex sessions on the next Push\.$/);
  assert.match(en.settings.sync.claudeChildrenHint, /Changing it re-uploads synced Claude sessions on the next Push\.$/);
  // Each sentence is the hint of its own row.
  for (const key of ["privacy.redactionHint", "sync.codexChildrenHint", "sync.claudeChildrenHint"]) {
    assert.match(source, new RegExp(`\\{t\\("${key.replace(".", "\\.")}"\\)\\}`), key);
  }
});

test("each agent CLI row is a path field: typed or browsed, saved on Enter as app settings, tested only with --version", () => {
  const page = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const types = readFileSync("src/types.ts", "utf8");
  const commands = readFileSync("src-tauri/src/project_sync/provider_cli.rs", "utf8");
  const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
  const document = readFileSync("crates/mallard-infrastructure/src/app_settings.rs", "utf8");
  const discovery = readFileSync("crates/mallard-infrastructure/src/cli_discovery.rs", "utf8");

  // One row per provider that declares a CLI, never a fixed provider list;
  // the path is one more app-settings field saved through the same handler,
  // typed, browsed, or cleared back to automatic (null).
  assert.match(page, /\.filter\(\(provider\) => provider\.has_cli\)/);
  assert.match(page, /applyTuning\(\{ \[cliSettingKey\(provider\)\]: path \}\)/);
  assert.match(page, /await saveCliPath\(provider, picked\);/);
  assert.match(page, /onClick=\{\(\) => onSave\(null\)\}/);
  // Browse opens beside the CLI in use: the macOS panel hides ~/.nvm.
  assert.match(page, /openFile\(\{\s*multiple: false,\s*directory: false,[\s\S]*?defaultPath: status\?\.selected_path \?\? status\?\.launcher \?\? undefined,/);
  // The field never saves as it is typed or on blur — `/Users` is absolute
  // and would save — only on Enter, trimmed, empty meaning automatic.
  const row = page.slice(page.indexOf("function CliPathRow("), page.indexOf("export default function SettingsPage("));
  assert.ok(row.length > 0);
  assert.doesNotMatch(row, /onBlur/);
  assert.match(row, /if \(event\.key !== "Enter"\) return;/);
  assert.match(row, /if \(dirty && !disabled\) onSave\(typed \|\| null\);/);
  assert.match(row, /readOnly=\{disabled\}/);
  // Test resolves the saved CLI, never the field's unsaved text.
  assert.match(row, /disabled=\{testDisabled \|\| dirty\}/);
  // The field follows the saved value, never the pending one: a refused save
  // must leave the typed text in place.
  assert.match(page, /const saved = tuning\?\.\[key\];/);
  assert.match(page, /savedPath=\{typeof saved === "string" \? saved : null\}/);
  assert.match(types, /codex_cli_path: string \| null;/);
  assert.match(types, /claude_cli_path: string \| null;/);
  assert.match(types, /has_cli: boolean;/);
  assert.doesNotMatch(types, /ProviderCliLauncher|cli_launcher/);

  // The tag says where the CLI came from; the empty field's placeholder
  // names what automatic detection runs; one hint line, loudest first.
  const found = {
    provider: "codex", selected_path: null, launcher: "/opt/homebrew/bin/codex",
    executable: "/opt/homebrew/Cellar/codex/0.156.0/bin/codex", source: "inherited_path" as const, error: null,
  };
  const saved = { ...found, selected_path: "/Users/dev/bin/codex", launcher: "/Users/dev/bin/codex", source: "selected" as const };
  const gone = { ...saved, launcher: null, executable: null, source: null, error: "Selected Codex executable is unavailable. Choose it again in Settings." };
  const missing = { ...found, launcher: null, executable: null, source: null, error: "Codex CLI not found. Install Codex, or choose its executable in Settings." };

  assert.equal(cliSourceLabel(null), "Detecting…");
  assert.equal(cliSourceLabel(found), "Mallard's PATH");
  assert.equal(cliSourceLabel({ ...found, source: "shell_path" }), "shell PATH");
  assert.equal(cliSourceLabel({ ...found, source: "standard_location" }), "install folder");
  assert.equal(cliSourceLabel(saved), "Custom");
  assert.equal(cliSourceLabel(gone), "Custom");
  assert.equal(cliSourceLabel(missing), null);

  assert.equal(cliPlaceholder(null, "Codex"), "Detecting…");
  assert.equal(cliPlaceholder(found, "Codex"), "Auto-detected: /opt/homebrew/bin/codex");
  assert.equal(cliPlaceholder(missing, "Codex"), "Not found. Type the path to Codex.");
  assert.equal(cliPlaceholder(saved, "Codex"), "Leave empty to detect automatically");

  assert.equal(cliHint(found, undefined, false), null);
  assert.deepEqual(cliHint(found, undefined, true), { tone: null, text: "Press Enter to save.", useAutomatic: false });
  assert.deepEqual(cliHint(gone, undefined, true), { tone: null, text: "Press Enter to save.", useAutomatic: false });
  assert.deepEqual(cliHint(saved, undefined, false), { tone: null, text: null, useAutomatic: true });
  assert.deepEqual(cliHint(gone, undefined, false), { tone: "bad", text: gone.error, useAutomatic: true });
  assert.deepEqual(cliHint(missing, undefined, false), { tone: "bad", text: missing.error, useAutomatic: false });
  assert.deepEqual(
    cliHint(saved, { kind: "failed", text: "Codex did not report a version." }, false),
    { tone: "bad", text: "Test failed: Codex did not report a version.", useAutomatic: true },
  );
  assert.equal(cliHint(found, { kind: "version", text: "0.156.0" }, false), null);

  assert.match(api, /invoke<ProviderCliStatus>\("inspect_provider_cli", \{ provider \}\)/);
  assert.match(api, /invoke<string>\("test_provider_cli", \{ provider \}\)/);
  assert.match(api, /invoke<void>\("refresh_cli_detection"\)/);
  assert.match(lib, /project_sync::provider_cli::inspect_provider_cli,\s*project_sync::provider_cli::test_provider_cli,\s*project_sync::provider_cli::refresh_cli_detection,/);

  // Rust side: the saved path is validated as one absolute file path, the
  // document is bounded on read as well as write, and a path change is not
  // a tuning change (it must purge no cache). Test runs --version alone.
  assert.match(document, /pub codex_cli_path: Option<String>,/);
  assert.match(document, /pub claude_cli_path: Option<String>,/);
  assert.match(document, /const MAX_SETTINGS_BYTES: u64 = 16 \* 1024;/);
  assert.match(document, /metadata\.len\(\) > MAX_SETTINGS_BYTES/);
  assert.match(document, /fn validate_cli_path\(label: &str, path: &str\)/);
  assert.match(document, /Tuning values only: a CLI path or an export setting never purges a cache\./);
  assert.match(commands, /cli\.resolve\(settings\.cli_path\(provider\)\)\?;\s*cli\.probe_version\(&resolved\)/);
  assert.match(discovery, /\.arg\("--version"\)\s*\.env_clear\(\)/);
  assert.match(discovery, /\.args\(\["-i", "-l", "-c", &script\]\)/);
  assert.match(discovery, /SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs\(10\)/);
  assert.match(discovery, /SHELL_PROBE_OUTPUT_LIMIT: usize = 64 \* 1024/);
});

/** Runs `render` with the interface in Chinese, then puts English back. */
function inChinese<T>(render: () => T): T {
  void i18n.changeLanguage("zh-CN");
  try {
    return render();
  } finally {
    void i18n.changeLanguage("en");
  }
}

/** Every Latin word a Chinese page shows — text and the names a reader or a
 *  screen reader meets — after the words that stay English by decision. */
function latinLeft(html: string, kept: readonly string[]): string[] {
  const named = [...html.matchAll(/(?:title|aria-label|placeholder|data-tooltip)="([^"]*)"/g)].map((match) => match[1]);
  let text = [html.replace(/<[^>]+>/g, " "), ...named].join(" ");
  for (const word of kept) text = text.split(word).join(" ");
  return text.match(/[A-Za-z]{2,}/g) ?? [];
}

/* Product and system names, units, paths, the done target the test hands in,
   and English as the Language row names it — in its own language. */
const KEPT_LATIN = ["~/.mallard/logs", "~/.mallard", "--version", "mallard-app", "Mallard", "GitHub", "Codex", "Claude", "CLI", "PATH", "Shell", "macOS", "Windows", "CPU", "Version", "MB", "KB", "English"];

test("the Settings page in Chinese: every row, the path bar and the switches, nothing left in English", () => {
  const html = inChinese(() => renderPage([{
    label: "Mallard",
    items: [{ id: "reload", label: "重新载入应用数据", icon: "computer" as const, onSelect: () => undefined }],
  }]));

  // The bar: its root crumb, its name, Done and where Done lands, the app menu.
  assert.match(html, /aria-label="设置路径"/);
  assert.match(html, />设置<\/span>/);
  assert.match(html, /<button type="button" class="sync-path-sync" title="关闭设置，返回 mallard-app">完成<\/button>/);
  assert.match(html, /aria-label="应用操作"/);
  assert.match(html, /<span data-tauri-drag-region="true">更改自动保存<\/span>/);

  const headings = [...html.matchAll(/class="settings-section-title"[^>]*>.*?<\/svg>([^<]+)</g)].map((match) => match[1].trim());
  assert.deepEqual(headings, ["外观", "隐私", "同步", "代理 CLI", "性能", "日志", "显示", "本地数据"]);

  // Appearance: the theme's three choices, System first and checked.
  assert.match(html, /role="radiogroup" aria-label="主题"/);
  assert.match(html, /<button type="button" role="radio" aria-checked="true" class="active">跟随系统<\/button>/);
  assert.match(html, />浅色</);
  assert.match(html, />深色</);
  assert.match(html, /“跟随系统”使用 macOS 的外观设置。/);

  // Privacy: redaction is 密钥隐藏, its switch named so, the hint names the scope.
  assert.match(html, /<strong>密钥隐藏<\/strong>/);
  assert.match(html, /<input[^>]*role="switch"[^>]*disabled=""[^>]*aria-label="密钥隐藏"\/>/);
  assert.match(html, /隐藏检测到的密钥、令牌和密码/);
  assert.match(html, /启动时及每隔 6小时检查一次 GitHub。/);
  assert.match(html, /<button[^>]*disabled=""[^>]*>检查更新<\/button>/);

  // Each switch's word, in page order; the selects and their counts.
  assert.deepEqual(
    [...html.matchAll(/<span class="sync-settings-state" aria-hidden="true">([^<]+)<\/span>/g)].map((match) => match[1]),
    ["关", "开", "关", "关", "关", "关", "开"],
  );
  assert.match(html, /<option value="30" selected="">30天<\/option>/);
  assert.match(html, /<option value="4" selected="">4 个<\/option>/);
  assert.match(html, /日志保存在本机的 <code>~\/\.mallard\/logs<\/code> 中。/);
  assert.match(html, /永久删除 <code>~\/\.mallard<\/code>（同步元数据、日志、下载备份和偏好设置）并重启。/);
  assert.match(html, /class="btn btn-ghost danger"[^>]*>重置 Mallard</);

  assert.deepEqual(latinLeft(html, KEPT_LATIN), []);

  // English comes back whole once the language does.
  assert.match(renderPage(), /<strong>Credential redaction<\/strong>/);
});

test("the Language row offers System and each offered language in its own name", () => {
  // The choices, for the build as it is and for the build with Chinese on.
  assert.deepEqual(languageOptions(["en"]), [
    { value: "system", label: "System" },
    { value: "en", label: "English", lang: "en" },
  ]);
  assert.deepEqual(languageOptions(["en", "zh-CN"]), [
    { value: "system", label: "System" },
    { value: "en", label: "English", lang: "en" },
    { value: "zh-CN", label: "简体中文", lang: "zh-CN" },
  ]);
  // System speaks the interface's language; each language keeps its own name.
  assert.deepEqual(inChinese(() => languageOptions(["en", "zh-CN"])).map((option) => option.label), ["跟随系统", "English", "简体中文"]);

  // The row stands under Theme only while there is a choice to make.
  const html = renderPage();
  const language = /<div class="sync-settings-row" id="settings-row-language">[\s\S]*?role="radiogroup" aria-label="Language"/;
  if (OFFERED_LANGUAGES.length > 1) {
    assert.match(html, language);
    assert.match(html, /id="settings-row-theme"[\s\S]*?id="settings-row-language"[\s\S]*?System uses your macOS language\./);
    assert.match(html, /<button type="button" role="radio" lang="zh-CN"[^>]*>简体中文<\/button>/);
    assert.match(html, /<button type="button" role="radio" lang="en"[^>]*>English<\/button>/);
  } else {
    assert.doesNotMatch(html, language);
    assert.doesNotMatch(html, /settings-row-language|System uses your macOS language/);
  }
  assert.equal(en.settings.appearance.language, "Language");
  assert.equal(en.settings.appearance.languageHint, "System uses your macOS language.");
});

test("the path bar in Chinese: its menus, the switcher's filter and the banner's count", () => {
  const storages = Array.from({ length: 6 }, (_, index) => ({ id: `s${index}`, label: `Attic ${index}`, onSelect: () => undefined }));
  // The caller words the menu's noun; the bar words everything around it.
  const render = (menuLabel: string) => renderToStaticMarkup(
    <SyncPathBar
      crumbs={[
        {
          id: "storage",
          label: "Attic R2",
          tone: "root",
          menuLabel,
          menuHeader: { label: "Attic R2" },
          menu: [{ label: "Attic", items: storages }],
        },
        { id: "project", label: "HorrorGame", onActivate: () => undefined, menu: [{ items: [{ id: "a", label: "Rename", onSelect: () => undefined }] }] },
      ]}
      overflow={[{ items: [{ id: "reload", label: "Reload", onSelect: () => undefined }] }]}
      notices={{
        items: [
          { key: "a", severity: "warning", message: "Unavailable for Attic R2." },
          { key: "b", severity: "info", message: "Comparing." },
        ],
        timing: "standing",
        label: "Notes",
      }}
    />,
  );
  const english = render("Storage");
  assert.match(english, /aria-label="Workspace path"/);
  assert.match(english, /placeholder="Switch storage…" aria-label="Filter storage list"/);
  assert.match(english, /title="More actions" aria-label="More actions"/);
  assert.match(english, /title="Switch" aria-label="Switch"/);
  assert.match(english, /<strong class="notice-lead">2 messages from Mallard<\/strong>/);
  assert.match(english, /aria-label="Dismiss: all 2 messages from Mallard"/);

  const html = inChinese(() => render("存储"));
  assert.match(html, /aria-label="工作区路径"/);
  assert.match(html, /placeholder="切换存储…" aria-label="筛选存储列表"/);
  assert.match(html, /title="更多操作" aria-label="更多操作"/);
  assert.match(html, /title="切换" aria-label="切换"/);
  // The zone counts in its subject's measure word and names its rung.
  assert.match(html, /<strong class="notice-type">警告 ·<\/strong> <strong class="notice-lead">2 条来自 Mallard 的消息<\/strong>/);
  assert.match(html, /aria-label="关闭：全部 2 条来自 Mallard 的消息"/);
  // What the caller handed in stays as given: names and the backend's sentence.
  assert.match(html, />Attic R2</);
  assert.match(html, />HorrorGame</);
  assert.match(html, /Unavailable for Attic R2\./);
  assert.match(html, /aria-label="关闭：警告。Unavailable for Attic R2\."/);
});
