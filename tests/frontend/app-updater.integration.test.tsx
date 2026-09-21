import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import {
  AUTO_CHECK_INTERVAL_MS,
  DEFER_UPDATE_MS,
  buildUpdateNotice,
  describeUpdateError,
  downloadPercentage,
  releaseNotePreview,
  releasePageUrl,
  shouldDeferUpdate,
  shouldRunScheduledCheck,
  UpdateProgressDialog,
  type UpdateNoticeHandlers,
  type UpdateSummary,
} from "../../src/components/AppUpdater";
import { NoticeZone } from "../../src/components/notice/Notice";

/* The updater is one item in the shell's single notice zone. These tests pin
 * the copy that matters — the version, the busy gate, the type name — and the
 * two rules the client side lives by: never restart over a tracked
 * operation, and never ask GitHub on a schedule the user switched off. */

const summary: UpdateSummary = {
  version: "0.2.0",
  currentVersion: "0.1.3",
  notes: "https://github.com/Git4Agent/mallard-app/releases/tag/v0.2.0",
  date: "2026-09-20T12:00:00Z",
};

const noop = () => undefined;
const handlers: UpdateNoticeHandlers = {
  onInstall: noop,
  onLater: noop,
  onRetry: noop,
  onDismiss: noop,
  onOpenReleaseNotes: noop,
};

function render(item: ReturnType<typeof buildUpdateNotice>): string {
  assert.ok(item, "the updater has something to say");
  return renderToStaticMarkup(
    <NoticeZone items={[item]} subject="messages from Mallard" timing="progress" />,
  );
}

test("an available update names the version and offers install, later, and the notes", () => {
  const html = render(buildUpdateNotice(
    { phase: "available", summary, error: null, completed: null, busy: false },
    handlers,
  ));
  assert.match(html, /Note ·/, "the rung is named in the text");
  assert.match(html, /Mallard 0\.2\.0 is available\./);
  assert.match(html, /You have 0\.1\.3/);
  assert.match(html, /<button[^>]*>Update and restart<\/button>/);
  assert.doesNotMatch(html, /<button[^>]*disabled=""[^>]*>Update and restart/);
  assert.match(html, /<button[^>]*>Later<\/button>/);
  assert.match(html, /title="https:\/\/github\.com\/Git4Agent\/mallard-app\/releases\/tag\/v0\.2\.0"[^>]*>Release notes/);
});

test("an available update waits until active sync work finishes", () => {
  const html = render(buildUpdateNotice(
    { phase: "available", summary, error: null, completed: null, busy: true },
    handlers,
  ));
  assert.match(html, /Finish the current operation before restarting\./);
  assert.match(html, /<button[^>]*disabled=""[^>]*>Update and restart<\/button>/);
});

test("a failed install keeps the version and offers a retry", () => {
  const html = render(buildUpdateNotice(
    { phase: "error", summary, error: "signature verification failed", completed: null, busy: false },
    handlers,
  ));
  assert.match(html, /Error ·/);
  assert.match(html, /Mallard 0\.2\.0 did not install\./);
  assert.match(html, /signature verification failed/);
  assert.match(html, /<button[^>]*>Try again<\/button>/);
  assert.doesNotMatch(html, /Update and restart/);
});

test("a manual check reports checking, up to date, or the failure", () => {
  const checking = render(buildUpdateNotice(
    { phase: "checking", summary: null, error: null, completed: null, busy: false },
    handlers,
  ));
  assert.match(checking, /Checking for updates\./);
  assert.doesNotMatch(checking, /<button/, "nothing to press while it looks");

  const latest = render(buildUpdateNotice(
    { phase: "latest", summary: null, error: null, completed: null, busy: false },
    handlers,
  ));
  assert.match(latest, /Done ·/);
  assert.match(latest, /Mallard is up to date\./);
  assert.match(latest, /aria-label="Dismiss update notice"/);

  const failed = render(buildUpdateNotice(
    { phase: "error", summary: null, error: "offline", completed: null, busy: false },
    handlers,
  ));
  assert.match(failed, /Could not check for updates\./);
  assert.match(failed, /offline/);
  assert.match(failed, /<button[^>]*>Try again<\/button>/);
});

test("the restart receipt shows once and links the notes; idle says nothing", () => {
  const done = render(buildUpdateNotice(
    { phase: "idle", summary: null, error: null, completed: summary, busy: false },
    handlers,
  ));
  assert.match(done, /Mallard 0\.2\.0 installed\./);
  assert.match(done, /Release notes/);

  assert.equal(
    buildUpdateNotice({ phase: "idle", summary: null, error: null, completed: null, busy: false }, handlers),
    null,
  );
});

test("the download sheet is a native modal that cannot be cancelled", () => {
  const html = renderToStaticMarkup(
    <UpdateProgressDialog phase="downloading" downloaded={512} total={2048} version="0.2.0" />,
  );
  assert.match(html, /<dialog[^>]*class="sync-modal app-update-dialog"/);
  assert.match(html, /Downloading Mallard 0\.2\.0/);
  assert.match(html, /<progress[^>]*value="25"/);
  assert.match(html, /25% downloaded/);

  const installing = renderToStaticMarkup(
    <UpdateProgressDialog phase="installing" downloaded={2048} total={2048} version="0.2.0" />,
  );
  assert.match(installing, /Installing update/);
  assert.doesNotMatch(installing, /value="/, "an indeterminate bar while the app is replaced");
});

test("Later hides that version, and only that version, for a day", () => {
  const now = 1_700_000_000_000;
  const deferred = { version: "0.2.0", until: now + DEFER_UPDATE_MS };
  assert.equal(shouldDeferUpdate(deferred, "0.2.0", now), true);
  assert.equal(shouldDeferUpdate(deferred, "0.2.0", now + DEFER_UPDATE_MS), false);
  assert.equal(shouldDeferUpdate(deferred, "0.2.1", now), false);
  assert.equal(shouldDeferUpdate(null, "0.2.0", now), false);
});

test("a scheduled check honours the Settings switch and the interval; a manual check ignores both", () => {
  const now = 1_700_000_000_000;
  assert.equal(shouldRunScheduledCheck(true, 0, now), true);
  assert.equal(shouldRunScheduledCheck(false, 0, now), false, "switched off: no request");
  assert.equal(shouldRunScheduledCheck(true, now - 1000, now), false, "just checked");
  assert.equal(shouldRunScheduledCheck(true, now - AUTO_CHECK_INTERVAL_MS, now), true);

  const source = readFileSync("src/components/AppUpdater.tsx", "utf8");
  // The manual path never consults the switch: the gate lives under `!interactive`.
  assert.match(source, /if \(!interactive\) \{[\s\S]*?shouldRunScheduledCheck\(enabled, lastCheckRef\.current\)/);
  assert.match(source, /interactive \? "privacy"|checkForUpdates\(true\)/);
});

test("the shell contributes the updater's item to its one notice zone and gates on busy", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  assert.match(shell, /useAppUpdates\(\{\s*busy,\s*autoCheckEnabled: readAutoUpdateCheckEnabled,\s*\}\)/);
  assert.match(shell, /settings\.update_check_enabled/);
  assert.match(shell, /if \(appUpdates\.notice\) shellNotices\.push\(appUpdates\.notice\);/);
  assert.match(shell, /\{appUpdates\.progress && <UpdateProgressDialog \{\.\.\.appUpdates\.progress\} \/>\}/);
  assert.doesNotMatch(shell, /app-update-notice/, "no banner family of its own");

  const settings = readFileSync("src/components/project-sync/SettingsPage.tsx", "utf8");
  assert.match(settings, /setting="update_check_enabled"/);
  assert.match(settings, /onClick=\{\(\) => requestAppUpdateCheck\(\)\}/);
});

test("helpers: percentages clamp, previews skip bare links, errors always read", () => {
  assert.equal(downloadPercentage(50, 200), 25);
  assert.equal(downloadPercentage(400, 200), 100);
  assert.equal(downloadPercentage(10, 0), null);
  assert.equal(releaseNotePreview("https://github.com/Git4Agent/mallard-app/releases/tag/v0.2.0"), null);
  assert.equal(releaseNotePreview("## Fixes\n\n- Safer restores"), "Fixes");
  assert.equal(releaseNotePreview("   "), null);
  assert.equal(releasePageUrl("0.2.0"), "https://github.com/Git4Agent/mallard-app/releases/tag/v0.2.0");
  assert.equal(describeUpdateError(new Error(" boom ")), "boom");
  assert.equal(describeUpdateError({ code: 7 }), '{"code":7}');
  assert.match(describeUpdateError(undefined), /could not complete the update/);
});
