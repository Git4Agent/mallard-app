import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import LogPanel from "../../src/components/LogPanel";

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
