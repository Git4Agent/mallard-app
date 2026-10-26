import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import SettingsPage from "../../src/components/project-sync/SettingsPage";
import type { AppUpdates } from "../../src/components/AppUpdater";
import {
  EMPTY_RECEIPTS,
  FADE_MS,
  RECEIPT_MS,
  SLOW_MS,
  receiptsReducer,
  rowReceipt,
  statusLine,
  type ReceiptsAction,
  type ReceiptsState,
} from "../../src/components/project-sync/settingsReceipts";
import { getPreference, getThemeChoice, setPreference, setThemeChoice } from "../../src/preferences";
import { i18n, OFFERED_LANGUAGES } from "../../src/i18n";

const appActions = [{
  label: "Mallard",
  items: [{ id: "reload", label: "Reload app data", icon: "computer" as const, onSelect: () => undefined }],
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

function renderPage(doneTarget = "mallard-app"): string {
  return renderToStaticMarkup(
    <SettingsPage
      appActions={appActions}
      appUpdates={idleUpdates}
      onLogsChanged={() => undefined}
      onDone={() => undefined}
      doneTarget={doneTarget}
    />,
  );
}

const run = (...actions: ReceiptsAction[]): ReceiptsState => actions.reduce(receiptsReducer, EMPTY_RECEIPTS);

test("Settings saves as you go and says so: a status line and Done in the bar, no Save", () => {
  const html = renderPage();
  const bar = html.slice(html.indexOf("<header"), html.indexOf("</header>"));

  // The line states the rule until something changes — a result on a page
  // nobody touched would be a small lie. It is titlebar chrome, so it carries
  // the drag attribute like the rest of the bar.
  assert.match(
    bar,
    /<span class="sync-settings-status" data-state="rule" data-tauri-drag-region="true"><span data-tauri-drag-region="true">Changes save automatically<\/span><\/span>/,
  );

  // Done is a bar verb that only leaves, and its title names where it lands.
  assert.match(bar, /<button type="button" class="sync-path-sync" title="Close Settings — back to mallard-app">Done<\/button>/);
  assert.match(renderPage("Projects"), /title="Close Settings — back to Projects">Done<\/button>/);
  // Status, then Done, then the divider ahead of the app menu.
  assert.match(bar, /class="sync-settings-status"[\s\S]*>Done<\/button><span class="sync-path-divider"[\s\S]*aria-label="App actions"/);

  // Every row saved when it changed, so there is nothing for a Save to do.
  assert.doesNotMatch(html, />Save</);

  // One polite region speaks for the stamps, empty until a save, and no
  // stamp stands on first paint.
  assert.match(html, /<main[^>]*><p class="sync-visually-hidden" role="status"><\/p>/);
  assert.doesNotMatch(html, /sync-settings-receipt/);

  // Every setting row is an anchor the failure line can find; the verbs are not.
  const rows = [...html.matchAll(/id="settings-row-([A-Za-z_0-9]+)"/g)].map((match) => match[1]);
  assert.deepEqual(rows, [
    "theme",
    // Language stands under Theme once there is a second language to pick.
    ...(OFFERED_LANGUAGES.length > 1 ? ["language"] : []),
    "credential_redaction_enabled",
    "update_check_enabled",
    "push_codex_children",
    "push_claude_children",
    "openLogsOnSync",
    "refreshOnFocus",
    "project_file_limit_bytes",
    "timeline_preview_limit_bytes",
    "comparison_workers",
    "cloud_transfer_concurrency",
    "retention_days",
    "max_total_bytes",
    "clock24",
  ]);

  // Done is the canonical closer, so it lands on the open project's overview,
  // or on Projects when none is open.
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(
    shell,
    /onDone=\{\(\) => void closeWorkspaceSettings\(\)\}\s*doneTarget=\{activeSummary \? projectLabel\(activeSummary\) : t\("roots\.projects"\)\}/,
  );
  assert.match(readFileSync("src/i18n/en/shell.ts", "utf8"), /roots: \{\s*projects: "Projects",/);

  // The receipt words are closed vocabulary: the display face, from its one list.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.sync-settings-flag,\s*\.sync-settings-receipt,\s*\.sync-settings-status,/);
});

test("receipts: a stamp per change, one line for the page", () => {
  assert.deepEqual(statusLine(EMPTY_RECEIPTS), { tone: "rule", text: "Changes save automatically", at: null });

  // One save: its stamp, and the line counts the stamps you can see.
  let state = run({ type: "settled", key: "push_codex_children", ok: true, at: 1_000, seq: 1, announce: "Push Codex children saved" });
  assert.deepEqual(rowReceipt(state, "push_codex_children"), { kind: "saved", fading: false });
  assert.equal(statusLine(state).text, "Change saved");
  assert.deepEqual(state.announcement, { text: "Push Codex children saved", seq: 1 });

  // A second change inside the window: two stamps, two changes saved.
  state = receiptsReducer(state, { type: "settled", key: "clock24", ok: true, at: 1_700, seq: 2, announce: "24-hour clock saved" });
  assert.equal(statusLine(state).text, "2 changes saved");

  // The first stamp fades in place, then goes. An expiry belonging to another
  // save is ignored, so a re-saved row keeps its fresh stamp.
  state = receiptsReducer(state, { type: "fade", key: "push_codex_children", seq: 1 });
  assert.deepEqual(rowReceipt(state, "push_codex_children"), { kind: "saved", fading: true });
  state = receiptsReducer(state, { type: "expire", key: "clock24", seq: 1 });
  assert.ok(rowReceipt(state, "clock24"), "an expiry from another save must not take the stamp");
  state = receiptsReducer(state, { type: "expire", key: "push_codex_children", seq: 1 });
  assert.equal(rowReceipt(state, "push_codex_children"), null);
  assert.equal(statusLine(state).text, "Change saved");

  // With every stamp gone, the line settles on when the visit last saved.
  state = receiptsReducer(state, { type: "expire", key: "clock24", seq: 2 });
  assert.deepEqual(statusLine(state), { tone: "settled", text: "All changes saved", at: 1_700 });
});

test("receipts: a slow write says Saving…, and a failure stands until its store writes again", () => {
  const store = ["credential_redaction_enabled", "push_codex_children"];
  let state = run({ type: "begin", store }, { type: "slow", key: "push_codex_children" });
  assert.deepEqual(rowReceipt(state, "push_codex_children"), { kind: "saving" });
  assert.deepEqual(statusLine(state), { tone: "saving", text: "Saving…", at: null });

  // The write failed: the stamp and the line say so. Nothing is announced —
  // the section's alert already speaks — and the visit has saved nothing yet.
  state = receiptsReducer(state, { type: "settled", key: "push_codex_children", ok: false, at: 2_000, seq: 3, announce: null });
  assert.deepEqual(rowReceipt(state, "push_codex_children"), { kind: "failed" });
  assert.deepEqual(statusLine(state), { tone: "failed", text: "1 change not saved", at: null });
  assert.equal(state.announcement, null);
  assert.equal(state.savedAt, null);

  // A failure outranks a later save somewhere else…
  state = receiptsReducer(state, { type: "settled", key: "clock24", ok: true, at: 2_500, seq: 4, announce: "24-hour clock saved" });
  assert.equal(statusLine(state).tone, "failed");

  // …and the next write to its store clears it, the way the section's error clears.
  state = receiptsReducer(state, { type: "begin", store });
  assert.equal(rowReceipt(state, "push_codex_children"), null);
  assert.equal(statusLine(state).text, "Change saved");

  // The same sentence twice is still two announcements: its seq moves.
  const again = receiptsReducer(state, { type: "settled", key: "clock24", ok: true, at: 3_000, seq: 5, announce: "24-hour clock saved" });
  assert.equal(again.announcement?.text, state.announcement?.text);
  assert.notEqual(again.announcement?.seq, state.announcement?.seq);

  // The timings the page is built on: a normal write never shows Saving…,
  // and a stamp is gone within three seconds.
  assert.equal(SLOW_MS, 400);
  assert.ok(RECEIPT_MS + FADE_MS < 3_000);
});

test("the status line speaks the interface language, counting in Chinese without a plural", () => {
  const saved = run(
    { type: "settled", key: "clock24", ok: true, at: 1_000, seq: 1, announce: null },
    { type: "settled", key: "theme", ok: true, at: 1_100, seq: 2, announce: null },
  );
  const failed = run({ type: "settled", key: "clock24", ok: false, at: 1_000, seq: 1, announce: null });
  void i18n.changeLanguage("zh-CN");
  try {
    assert.equal(statusLine(EMPTY_RECEIPTS).text, "更改自动保存");
    assert.equal(statusLine(saved).text, "已保存 2 项更改");
    assert.equal(statusLine(failed).text, "1 项更改未保存");
    assert.equal(statusLine(run({ type: "begin", store: [] }, { type: "slow", key: "theme" })).text, "正在保存…");
    const html = renderPage("项目");
    assert.match(html, /title="关闭设置，返回项目">完成<\/button>/);
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.equal(statusLine(saved).text, "2 changes saved");
});

test("preference writes report whether they stuck, and a refused theme keeps the stored one", () => {
  // No storage here at all: nothing persists, and the setter says so.
  assert.equal(setPreference("refreshOnFocus", true), false);
  assert.equal(getPreference("refreshOnFocus"), false);

  const values = new Map<string, string>();
  let refuse = false;
  const localStorage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      if (refuse) throw new Error("QuotaExceededError");
      values.set(key, value);
    },
  };
  const scope = globalThis as { window?: unknown };
  scope.window = { localStorage };
  try {
    assert.equal(setPreference("refreshOnFocus", false), true);
    assert.equal(getPreference("refreshOnFocus"), false);
    assert.equal(setThemeChoice("light"), true);
    assert.equal(getThemeChoice(), "light");

    // A refused write leaves the previous value in force. The page reads it
    // back, so the control returns to it and its stamp says Not saved.
    refuse = true;
    assert.equal(setPreference("refreshOnFocus", true), false);
    assert.equal(getPreference("refreshOnFocus"), false);
    assert.equal(setThemeChoice("dark"), false);
    assert.equal(getThemeChoice(), "light");
  } finally {
    delete scope.window;
  }

  // The window repaints from what storage holds, never from a refused request.
  const source = readFileSync("src/preferences.ts", "utf8");
  assert.match(source, /applyThemeChoice\(saved \? choice : getThemeChoice\(\)\);/);
});
