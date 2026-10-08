import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import ReviewPreparing from "../../src/components/sync-review/ReviewPreparing";
import { i18n } from "../../src/i18n";
import {
  PREPARING_CANCELLED,
  PREPARING_GATE_MS,
  createReviewPreparing,
  type PreparingTimers,
} from "../../src/components/sync-review/reviewPreparationLifecycle";

interface FakeTimer { run: () => void; ms: number; cleared: boolean }

function harness() {
  const log: string[] = [];
  const timers: FakeTimer[] = [];
  const fake: PreparingTimers = {
    set: (run, ms) => {
      const timer = { run, ms, cleared: false };
      timers.push(timer);
      return timer;
    },
    clear: (timer) => { (timer as FakeTimer).cleared = true; },
  };
  const tracker = createReviewPreparing({
    shown: () => log.push("shown"),
    writing: () => log.push("writing"),
    finished: () => log.push("finished"),
  }, fake);
  return { log, timers, tracker };
}

test("a prepare that settles inside the gate never shows its card", () => {
  const { log, timers, tracker } = harness();
  const preparing = tracker.begin();
  preparing.finish();
  assert.equal(timers[0].ms, PREPARING_GATE_MS);
  assert.equal(PREPARING_GATE_MS, 150);
  assert.equal(timers[0].cleared, true);
  assert.deepEqual(log, ["finished"]);
});

test("Cancel settles a prepare at once and drops the backend's late answer", async () => {
  const { log, timers, tracker } = harness();
  const preparing = tracker.begin();
  timers[0].run();
  assert.deepEqual(log, ["shown"]);

  let answer: (value: string) => void = () => {};
  const backend = new Promise<string>((resolve) => { answer = resolve; });
  const raced = preparing.race(backend);
  assert.equal(tracker.cancel(), true);
  assert.equal(await raced, PREPARING_CANCELLED);
  answer("late plan");
  await backend;
  preparing.finish();
  preparing.finish();
  assert.deepEqual(log, ["shown", "finished"]);
  assert.equal(tracker.cancel(), false);
});

test("a write withdraws Cancel for the rest of the prepare", async () => {
  const { log, tracker } = harness();
  const preparing = tracker.begin();
  preparing.writing();
  assert.equal(tracker.cancel(), false);
  assert.equal(await preparing.race(Promise.resolve("accepted")), "accepted");
  preparing.finish();
  assert.deepEqual(log, ["writing", "finished"]);
});

test("a prepare started from an open review shows its card at once, with no gate", () => {
  // Compare again, Rescan, and the Push review's Open Pull review unmount the
  // review now; waiting out the gate would drop to the page under it.
  const { log, timers, tracker } = harness();
  const preparing = tracker.begin({ immediate: true });
  assert.deepEqual(log, ["shown"]);
  assert.equal(timers.length, 0);
  preparing.finish();
  assert.deepEqual(log, ["shown", "finished"]);
});

test("a superseded prepare's gate, write and finish say nothing", () => {
  const { log, timers, tracker } = harness();
  const older = tracker.begin();
  const newer = tracker.begin();
  timers[0].run();
  older.writing();
  older.finish();
  assert.deepEqual(log, []);
  timers[1].run();
  newer.finish();
  assert.deepEqual(log, ["shown", "finished"]);
});

test("the card is the review's own frame: its caption, placeholders from the first frame, and Cancel", () => {
  const html = renderToStaticMarkup(
    <ReviewPreparing
      verb="pull"
      title="Pull review"
      caption="Reading the latest version from Attic R2"
      onCancel={() => undefined}
      cancelDisabled={false}
    />,
  );
  assert.match(html, /class="sync-inline-action-review sync-inline-pull-review sync-sync-review-workspace"/);
  assert.match(html, /<h2 id="sync-review-preparing-title">Pull review<\/h2>/);
  // The one caption, words only: no count, no bar.
  assert.match(html, /class="loading-scaffold sessions" role="region" aria-label="Reading the latest version from Attic R2"/);
  assert.match(html, /Reading the latest version from Attic R2…/);
  assert.doesNotMatch(html, /sync-btn-readout|\d+\/\d+ ·/);
  assert.equal(html.match(/class="loading-scaffold-row"/g)?.length, 3);
  // Cancel while it only reads; the primary wears the wait.
  assert.match(html, /aria-label="Cancel preparing the Pull review"[^>]*>|title="Cancel — nothing has changed"/);
  assert.doesNotMatch(html, /sync-inline-action-close" disabled=""/);
  assert.match(html, /<button type="button" class="btn btn-primary sync-pull-apply-button" disabled="" title="Preparing…">[\s\S]*?icon-spin[\s\S]*?Preparing…<\/button>/);

  const writing = renderToStaticMarkup(
    <ReviewPreparing
      verb="push"
      title="Push review"
      caption="Comparing this computer with Attic R2"
      onCancel={() => undefined}
      cancelDisabled
    />,
  );
  assert.match(writing, /sync-inline-push-review/);
  assert.match(writing, /class="btn btn-ghost sync-inline-action-close" disabled="" title="Changes are being written; this can&#x27;t be stopped now"/);

  // In Chinese the frame words its own controls; the host's title and
  // caption arrive already worded, and the storage name stays as given.
  void i18n.changeLanguage("zh-CN");
  try {
    const chinese = renderToStaticMarkup(
      <ReviewPreparing
        verb="pull"
        title="下载检查"
        caption="正在从 Attic R2 读取最新 Version"
        onCancel={() => undefined}
        cancelDisabled={false}
      />,
    );
    assert.match(chinese, /title="取消——没有任何更改" aria-label="取消准备下载检查"/);
    assert.match(chinese, /disabled="" title="正在准备…">[\s\S]*?正在准备…<\/button>/);
    assert.match(chinese, /正在从 Attic R2 读取最新 Version/);
  } finally {
    void i18n.changeLanguage("en");
  }
});

test("every prepare ends the card with what replaces it, and nothing celebrates a review opening", () => {
  const sync = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  // No shell receipt for a review that is on screen anyway.
  assert.doesNotMatch(sync, /Pull review ready for/);
  // Both prepares race their calls and close the card in their finally.
  assert.match(sync, /const preparing = beginPreparing\("push", projectId, storageId\);/);
  assert.match(sync, /const preparing = beginPreparing\("pull", projectId, storageId\);/);
  assert.match(sync, /preparing\.race\(projectSyncApi\.preparePushReview\(projectId, storageId, scope\)\)/);
  assert.equal(sync.match(/} finally \{\s*(?:\/\/[^\n]*\n\s*)*preparing\.finish\(\);/g)?.length, 3);
  // Pull's metadata accept is a write: Cancel is withdrawn before it.
  assert.match(sync, /preparing\?\.writing\(\);\s*await acceptPullMetadata/);
  // The card is an open review for both buttons, wearing the wait.
  assert.match(sync, /preparing: true,\s*continueDisabled: true,[\s\S]*?continueLabel: t\("preparing\.label"\),\s*continuePending: true,\s*onClose: cancelPreparing,/);
  assert.match(readFileSync("src/i18n/en/shell.ts", "utf8"), /preparing: \{[^}]*label: "Preparing…",/);

  const workspace = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  // While the card is up, busy means preparing — never publishing or applying.
  assert.match(workspace, /const workspacePushPublishing = workspacePushReviewOpen && busy && !workspacePreparingReview;/);
  assert.match(workspace, /const workspacePullApplying = workspacePullReviewOpen && busy && !workspacePreparingReview;/);
  // The prepare's progress is the card's words, not a count in the button.
  assert.doesNotMatch(workspace, /workspacePullReadout|phase === "plan"/);
});
