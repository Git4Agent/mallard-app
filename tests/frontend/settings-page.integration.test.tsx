import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import SettingsPage from "../../src/components/project-sync/SettingsPage";
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

function renderPage(): string {
  return renderToStaticMarkup(
    <SettingsPage appActions={appActions} onLogsChanged={() => undefined} />,
  );
}

test("the Settings page is a path-bar screen of grouped preference rows", () => {
  const html = renderPage();

  // Path bar: one root crumb naming the page, the app menu in the overflow.
  assert.match(html, /class="sync-path-crumb root leaf"/);
  assert.match(html, /aria-label="Settings path"/);
  assert.match(html, />Settings<\/span>/);
  assert.match(html, /aria-label="App actions"/);

  // Five groups, in reading order, each heading on the shared chrome class.
  const headings = [...html.matchAll(/class="settings-section-title"[^>]*>.*?<\/svg>\s*([A-Za-z-]+)</g)]
    .map((match) => match[1]);
  assert.deepEqual(headings, ["Appearance", "Privacy", "Sync", "Logs", "Display"]);

  // Theme: a radiogroup of the three choices; absence of a stored key means
  // System, so it is the checked one under the test's bare environment.
  assert.match(html, /role="radiogroup" aria-label="Theme"/);
  const checked = html.match(/<button[^>]*role="radio"[^>]*aria-checked="true"[^>]*>([A-Za-z]+)</);
  assert.equal(checked?.[1], "System");
  assert.match(html, />Light</);
  assert.match(html, />Dark</);

  // The three boolean preferences render as drawn switches, all on by
  // default — each default preserves what was hardcoded before.
  const switches = html.match(/role="switch"[^>]*class="sync-settings-switch"[^>]*checked=""/g) ?? [];
  assert.equal(switches.length, 3);

  // Redaction is a stated fact, not a control — and the copy names the
  // guarantee without surfacing the ruleset identifier.
  assert.match(html, /class="sync-settings-flag">Always on</);
  assert.doesNotMatch(html, /redact\.v1|mallard\.redact/);
  assert.doesNotMatch(html, /<input[^>]*aria-label="Credential redaction"/);

  // The Logs group carries the absorbed log-policy surface: both selects on
  // the app's select shell, the folder verb, and both cleanup verbs.
  assert.match(html, /aria-label="Keep log events for"/);
  assert.match(html, /aria-label="Log size cap"/);
  assert.equal((html.match(/class="sync-select-shell"/g) ?? []).length, 2);
  assert.match(html, /> Open folder</);
  assert.match(html, /> Clean up now</);
  assert.match(html, />Delete all</);
  assert.match(html, /~\/.mallard\/logs/);

  // Stats have not loaded in a static render; the selects sit disabled on
  // their defaults rather than inventing values.
  assert.match(html, /<option value="30" selected="">30 days<\/option>/);
  assert.match(html, /<option value="100" selected="">100 MB<\/option>/);
});

test("preferences default on, live under the mallard.* keys, and notify", () => {
  // No storage in this environment: every read lands on its default.
  assert.equal(getPreference("openLogsOnSync"), true);
  assert.equal(getPreference("refreshOnFocus"), true);
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
  // The legacy theme store is gone, module and key alike — the old key may
  // be named in prose, but no code path reads or writes it.
  assert.doesNotMatch(source, /(getItem|setItem)\("agent-sync-theme"/);
  assert.equal(existsSync("src/theme.ts"), false);
});

test("one clock decision serves every timestamp", () => {
  // Default 24-hour: the named h23 cycle with a padded hour, so fixed-width
  // columns stay fixed; `hour12: false` is not a synonym and stays banned.
  assert.deepEqual(clockOptions(), { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  assert.deepEqual(clockOptions(true), {
    hour: "2-digit", minute: "2-digit", hourCycle: "h23", second: "2-digit",
  });

  // Every production formatter that prints an hour goes through clockOptions;
  // no site keeps a private hour/minute pair.
  const sites = [
    "src/components/LogPanel.tsx",
    "src/components/sync-review/SyncOutcomeNotice.tsx",
    "src/components/project-sync/ProjectActivityPage.tsx",
    "src/components/project-sync/ProjectLinksWorkspace.tsx",
    "src/components/project-sync/session-detail/sessionDetailModel.ts",
    "src/components/storage-catalog/model.ts",
    "src/components/storage-catalog/StorageCatalogPage.tsx",
  ];
  for (const site of sites) {
    const text = readFileSync(site, "utf8");
    assert.match(text, /clockOptions\(/, `${site} must draw its clock from clockOptions`);
    assert.doesNotMatch(text, /hour12|hourCycle/, `${site} must not carry its own hour cycle`);
    assert.doesNotMatch(text, /hour: "(2-digit|numeric)"/, `${site} must not carry its own hour field`);
  }
  // The log drawer subscribes: it can be visible beside the Settings page
  // while the preference flips, and its stamps must follow immediately.
  assert.match(readFileSync("src/components/LogPanel.tsx", "utf8"), /usePreference\("clock24"\)/);
});

test("Settings is the fifth destination and absorbed the log manager", () => {
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const sidebar = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");

  // A workspace view like the Storages index: opened from the sidebar,
  // closed by every sibling opener, carrying the path bar.
  assert.match(sidebar, /<Icon name="settings" size=\{15\} \/> Settings/);
  assert.match(sync, /const \[settingsOpen, setSettingsOpen\] = useState\(false\);/);
  assert.match(sync, /\|\| settingsOpen\n/);
  assert.match(sync, /settingsOpen \? \(\s*<SettingsPage/);
  assert.match(sync, /onOpenSettings=\{openSettings\}/);

  // The focus reconcile honors its preference.
  assert.match(sync, /FOCUS_RECONCILE_MIN_AWAY_MS\n\s*&& getPreference\("refreshOnFocus"\)/);

  // The dialog is gone — file, mount, and the drawer's manage affordance now
  // navigates to the page instead of opening a modal.
  assert.equal(existsSync("src/components/project-sync/LogManagerDialog.tsx"), false);
  assert.doesNotMatch(sync, /LogManagerDialog|logManagerOpen/);
  assert.match(sync, /onManage=\{openSettings\}/);

  // Its CSS went with it, and the page's own vocabulary took its place.
  const appCss = readFileSync("src/App.css", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");
  assert.doesNotMatch(appCss, /sync-log-manager|sync-log-policy-fields|sync-log-storage-summary|sync-log-cleanup-actions/);
  assert.doesNotMatch(retroCss, /sync-log-manager|sync-log-policy-fields|sync-log-storage-summary|sync-log-cleanup-actions/);

  // The switch finally spends the tokens both palettes always carried, and
  // the two new chrome vocabularies speak pixel through the single shared
  // rule (whose one-rule invariant the cutover suite enforces).
  assert.match(appCss, /\.sync-settings-switch \{[^}]*background: var\(--switch-off\);/s);
  assert.match(appCss, /\.sync-settings-switch::before \{[^}]*background: var\(--switch-thumb\);/s);
  assert.match(retroCss, /\.sync-settings-flag,\n/);
  assert.match(retroCss, /\.sync-settings-segment > button,\n/);
});
