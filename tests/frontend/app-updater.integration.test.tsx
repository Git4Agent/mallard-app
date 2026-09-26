import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  AUTO_CHECK_INTERVAL_MS,
  UpdateCorner,
  launchReceipt,
  releasePageUrl,
  shouldRunScheduledCheck,
  type AppUpdates,
} from "../../src/components/AppUpdater";
import SettingsPage, { appVersionLine } from "../../src/components/project-sync/SettingsPage";

/* An update shows in one place, the sidebar's version mark, and never as a
 * failure. These tests pin the corner's states, the busy gate that keeps a
 * restart off a running operation, the receipt after an update, and the
 * Settings row that reports only what a check learned. */

const noop = () => undefined;
const idle: AppUpdates = {
  readyVersion: null,
  checking: false,
  restarting: false,
  operationRunning: false,
  lastCheckedAt: null,
  receiptVersion: null,
  checkNow: noop,
  restart: noop,
  openReceipt: noop,
};

const corner = (updates: Partial<AppUpdates>) => renderToStaticMarkup(<UpdateCorner updates={{ ...idle, ...updates }} />);

test("the corner is the plain version mark until a release is ready", () => {
  assert.equal(corner({}), "");
  // A check in flight, scheduled or manual, is not news.
  assert.equal(corner({ checking: true }), "");
});

test("a ready release is one Update button that names the version", () => {
  const html = corner({ readyVersion: "0.2.0" });
  assert.match(html, /^<button type="button" class="sync-update-pill"/);
  assert.match(html, /title="Restart Mallard to install 0\.2\.0"/);
  assert.match(html, /aria-label="Restart Mallard to install 0\.2\.0"/);
  assert.match(html, /<span>Update<\/span><\/button>$/);
  assert.doesNotMatch(html, /disabled/);
});

test("the restart waits for a running operation and says why", () => {
  const html = corner({ readyVersion: "0.2.0", operationRunning: true });
  assert.match(html, /disabled=""/);
  assert.match(html, /title="Mallard 0\.2\.0 is ready\. Finish the current operation, then restart to update\."/);
});

test("restarting is a status, not a button", () => {
  const html = corner({ readyVersion: "0.2.0", restarting: true });
  assert.match(html, /^<span class="sync-update-pill" role="status">/);
  assert.match(html, /class="status-loader"/);
  assert.match(html, /Restarting/);
  assert.doesNotMatch(html, /<button/);
});

test("the first launch after an update offers its notes once; a ready release wins", () => {
  const html = corner({ receiptVersion: "0.2.0" });
  assert.match(html, /class="sync-update-pill receipt"/);
  assert.match(html, /title="See what changed in Mallard 0\.2\.0"/);
  assert.match(html, /What’s new/);
  assert.match(corner({ receiptVersion: "0.2.0", readyVersion: "0.2.1" }), /<span>Update<\/span>/);

  assert.equal(launchReceipt(null, "0.2.0"), null, "a first launch has nothing to report");
  assert.equal(launchReceipt("0.2.0", "0.2.0"), null, "an ordinary launch");
  assert.equal(launchReceipt("0.1.9", "0.2.0"), "0.2.0");
  assert.equal(releasePageUrl("0.2.0"), "https://github.com/Git4Agent/mallard-app/releases/tag/v0.2.0");
});

test("a scheduled check honours the Settings switch and the interval", () => {
  const now = 1_700_000_000_000;
  assert.equal(shouldRunScheduledCheck(true, 0, now), true);
  assert.equal(shouldRunScheduledCheck(false, 0, now), false, "switched off: no request");
  assert.equal(shouldRunScheduledCheck(true, now - 1000, now), false, "just checked");
  assert.equal(shouldRunScheduledCheck(true, now - AUTO_CHECK_INTERVAL_MS, now), true);
});

test("Settings reports what a check learned and nothing about a failure", () => {
  assert.equal(appVersionLine("0.1.9", idle), "Mallard 0.1.9.");
  assert.equal(appVersionLine(null, idle), "Mallard.");
  assert.equal(
    appVersionLine("0.1.9", { readyVersion: null, lastCheckedAt: Date.now() }),
    "Mallard 0.1.9 is the newest release. Checked just now.",
  );
  assert.equal(
    appVersionLine("0.1.9", { readyVersion: "0.2.0", lastCheckedAt: Date.now() }),
    "Mallard 0.2.0 is ready to install. You have 0.1.9.",
  );

  const render = (updates: Partial<AppUpdates>) => renderToStaticMarkup(
    <SettingsPage
      appActions={[]}
      appUpdates={{ ...idle, ...updates }}
      onLogsChanged={noop}
      onDone={noop}
      doneTarget="Projects"
    />,
  );
  assert.match(render({ checking: true }), /<button[^>]*disabled=""[^>]*>Checking…<\/button>/);
  const ready = render({ readyVersion: "0.2.0" });
  assert.match(ready, /<button type="button" class="btn btn-primary"[^>]*>.*?Restart to update<\/button>/);
  assert.doesNotMatch(ready, /Check for updates/);
  assert.match(
    render({ readyVersion: "0.2.0", operationRunning: true }),
    /<button type="button" class="btn btn-primary" disabled="" title="Finish the current operation first">/,
  );
  assert.match(ready, /aria-label="Automatic updates"/);
});
