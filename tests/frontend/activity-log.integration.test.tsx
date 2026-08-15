import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import LogPanel, { nextFollowTail } from "../../src/components/LogPanel";

test("log bar keeps the multi-type filter and Manage logs in the header", () => {
  const html = renderToStaticMarkup(
    <LogPanel
      lines={[{
        schema: 1,
        id: "event-one",
        ts: 1,
        level: "success",
        type: "push",
        event: "push.completed",
        message: "Push complete",
      }]}
      typeFilters={["push", "pull"]}
      levelFilter="success"
      onTypeFiltersChange={() => undefined}
      onLevelFilterChange={() => undefined}
      onSearchChange={() => undefined}
      onManage={() => undefined}
      onClose={() => undefined}
    />,
  );

  const headerEnd = html.indexOf("log-panel-body");
  assert.ok(headerEnd > 0);
  const header = html.slice(0, headerEnd);
  assert.match(header, /Filter log by types: 2 selected/);
  assert.match(header, />Select all</);
  assert.match(header, />Deselect all</);
  assert.equal(header.match(/type="checkbox"/g)?.length, 7);
  assert.equal(header.match(/checked=""/g)?.length, 2);
  assert.match(header, /Filter log by level/);
  assert.match(header, /Search log/);
  assert.match(header, />Manage logs</);
  assert.doesNotMatch(header, />clear</);
  assert.match(html, /log-type-push/);
  assert.match(html, /Push complete/);
});

test("log exposes retained-history pagination", () => {
  const html = renderToStaticMarkup(
    <LogPanel
      lines={[]}
      hasOlder
      loadingOlder={false}
      onLoadOlder={() => undefined}
      onClose={() => undefined}
    />,
  );

  assert.match(html, />Load older logs</);
});

test("fork completion log levels render green for success and red for failure", () => {
  const html = renderToStaticMarkup(
    <LogPanel
      lines={[
        {
          schema: 1,
          id: "event-fork-success",
          ts: 2,
          level: "success",
          type: "pull",
          event: "fork.completed",
          message: "Created the Codex fork 'Fork title'",
        },
        {
          schema: 1,
          id: "event-fork-failure",
          ts: 1,
          level: "error",
          type: "pull",
          event: "fork.failed",
          message: "Mallard did not acknowledge the Codex fork",
        },
      ]}
      onClose={() => undefined}
    />,
  );
  assert.match(html, /log-line log-success/);
  assert.match(html, /Created the Codex fork/);
  assert.match(html, /log-line log-error/);
  assert.match(html, /Mallard did not acknowledge the Codex fork/);

  // Log messages are prose, so they take the 4.5:1 text variants of the
  // signal hues rather than the raw glyph inks.
  const css = readFileSync("src/App.css", "utf8");
  assert.match(css, /\.log-success \.log-msg \{ color: var\(--success-text\); \}/);
  assert.match(css, /\.log-error \.log-msg \{ color: var\(--danger-text\); \}/);
});

test("Push and Pull open the activity log at its live tail", () => {
  const logSource = readFileSync("src/components/LogPanel.tsx", "utf8");
  const syncSource = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const pushHandler = syncSource.slice(
    syncSource.indexOf("const publishProject = async"),
    syncSource.indexOf("const publishPendingPush = async"),
  );
  const pullHandler = syncSource.slice(
    syncSource.indexOf("const applyReview = async"),
    syncSource.indexOf("const refreshRestore = async"),
  );

  assert.match(pushHandler, /openActivity\(\)/);
  assert.match(pullHandler, /openActivity\(\)/);
  assert.match(syncSource, /scrollToBottomEpoch=\{logScrollToBottomEpoch\}/);
  assert.match(logSource, /followTailRef\.current = true/);
  assert.match(logSource, /body\.scrollTop = body\.scrollHeight/);
  assert.match(logSource, /new ResizeObserver/);
});

test("the log tail is not dropped by the scroll events its own pinning queues", () => {
  // The last pin landed at 1000; the list has grown by two lines since.
  const pinnedScrollTop = 1000;
  const grown = { scrollHeight: 1235, clientHeight: 193, pinnedScrollTop };

  // The pin's own scroll event arrives late and reports a two-line gap. The
  // viewport never moved, so the tail stays engaged.
  assert.equal(nextFollowTail(true, { ...grown, scrollTop: 1000 }), true);
  // The reader scrolls away from the pinned offset: following stops.
  assert.equal(nextFollowTail(true, { ...grown, scrollTop: 700 }), false);
  // ...and resumes once they scroll back down to the newest line.
  assert.equal(nextFollowTail(false, { ...grown, scrollTop: 1042 }), true);
  // A reader parked mid-list is not dragged back by a stale pinned offset.
  assert.equal(
    nextFollowTail(false, { scrollHeight: 2000, clientHeight: 193, pinnedScrollTop, scrollTop: 1000 }),
    false,
  );
});
