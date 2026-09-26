import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import SettingsPage from "../../src/components/project-sync/SettingsPage";
import type { AppUpdates } from "../../src/components/AppUpdater";
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

function renderPage(): string {
  return renderToStaticMarkup(
    <SettingsPage
      appActions={appActions}
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
  assert.deepEqual(headings, ["Appearance", "Privacy", "Sync", "Performance", "Logs", "Display", "Local data"]);

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

  // The Performance group: four tuning selects on the same shell, each
  // defaulting to what was hardcoded before the knobs existed. The thread
  // cap shares its "100 MB" face with the log size cap, so its default is
  // pinned through its own select; the other three are unambiguous.
  assert.match(html, /aria-label="Thread size limit"[\s\S]{0,600}<option value="100" selected="">100 MB<\/option>/);
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

test("Reset Mallard wipes ~/.mallard behind a warning confirm and restarts", () => {
  const page = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  const api = readFileSync("src/components/project-sync/api.ts", "utf8");
  const reset = readFileSync("src-tauri/src/app_reset.rs", "utf8");
  const lib = readFileSync("src-tauri/src/lib.rs", "utf8");

  // One warning confirm whose copy owns the irreversibility; the cutover is
  // a webview reload after the wipe resolves, with the localStorage sweep in
  // between. Never a process restart: relaunching the raw binary under
  // `tauri dev` orphans the frontend from its dev server (the CLI kills Vite
  // when the original process exits) and lands on a white window.
  assert.match(page, /This cannot be undone\./);
  assert.match(page, /kind: "warning", okLabel: "Reset Mallard"/);
  assert.match(page, /await projectSyncApi\.resetLocalState\(\);[\s\S]{0,900}clearStoredUiState\(\);[\s\S]{0,200}setResetPhase\("reloading"\);[\s\S]{0,200}window\.location\.reload\(\);/);

  assert.match(api, /invoke<LocalStateSummary>\("get_local_state_summary"\)/);
  assert.match(api, /invoke<LocalStateWipeResult>\("reset_local_state"\)/);
  assert.doesNotMatch(api, /restart_app/);

  // Rust side: the wipe holds the catalog-state guard, re-arms the only two
  // long-lived holders of pre-wipe state (the process survives the reset),
  // never relaunches the process, and logs no persisted success entry (that
  // would recreate ~/.mallard/logs the reset just deleted).
  assert.match(reset, /acquire_state_guard\(\)/);
  assert.match(reset, /wipe_local_state\(\)/);
  assert.match(reset, /session_watch_service\(\)\.reset_all_caches\(\);/);
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
  // that persists, installs, then drops the caches that baked the old
  // limits in — the watcher hears no filesystem event for a settings change.
  assert.match(document, /pub const APP_SETTINGS_SCHEMA: u32 = 1;/);
  assert.match(document, /#\[serde\(default = "default_conversation_file_limit_bytes"\)\]/);
  assert.match(document, /#\[serde\(default = "default_timeline_preview_limit_bytes"\)\]/);
  assert.match(commands, /save_app_settings\(repository, settings\)\?;\s*install_app_settings\(settings\);/);
  assert.match(commands, /reset_all_caches_and_purge_disk\(repository\.root\(\)\)/);
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
  assert.match(source, /Hide detected credentials in pushed sessions\. Changing it re-uploads synced\s*sessions on the next Push\. Earlier copies stay in storage history\./);
  assert.match(source, /Changing it re-uploads synced Codex sessions\s*on the next Push\./);
  assert.match(source, /Changing it re-uploads synced Claude sessions\s*on the next Push\./);
});
