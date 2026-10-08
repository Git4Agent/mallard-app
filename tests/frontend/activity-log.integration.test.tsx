import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import LogPanel, { nextFollowTail } from "../../src/components/LogPanel";
import ActivityLogDrawer from "../../src/components/project-sync/ActivityLogDrawer";
import { i18n } from "../../src/i18n";
import { en } from "../../src/i18n/en";

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

  // Gated, defaulting off: the preference enables the auto-open, and the
  // unread badge keeps counting either way.
  assert.match(pushHandler, /if \(getPreference\("openLogsOnSync"\)\) openActivity\(\);/);
  assert.match(pullHandler, /if \(getPreference\("openLogsOnSync"\)\) openActivity\(\);/);
  assert.match(syncSource, /scrollToBottomEpoch=\{logScrollToBottomEpoch\}/);
  assert.match(logSource, /followTailRef\.current = true/);
  assert.match(logSource, /body\.scrollTop = body\.scrollHeight/);
  assert.match(logSource, /new ResizeObserver/);
});

test("startup defers retained logs and opening the drawer loads them once", () => {
  const syncSource = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const drawerSource = readFileSync("src/components/project-sync/ActivityLogDrawer.tsx", "utf8");
  const backendSource = readFileSync("src-tauri/src/activity_log.rs", "utf8");
  const openActivity = syncSource.slice(
    syncSource.indexOf("const openActivity = () =>"),
    syncSource.indexOf("const startLogResize ="),
  );
  const queryCommand = backendSource.slice(
    backendSource.indexOf("pub async fn query_activity_logs"),
    backendSource.indexOf("pub async fn get_activity_log_stats"),
  );

  assert.match(
    drawerSource,
    /useEffect\(\(\) => \{\s*if \(open\) void loadActivityLogs\(\);\s*\}, \[loadActivityLogs, open, refreshEpoch\]\);/,
  );
  assert.doesNotMatch(openActivity, /loadActivityLogs/);
  assert.doesNotMatch(queryCommand, /store\.cleanup/);
  assert.match(queryCommand, /store\.query\(&query\)/);
});

test("closed Logs remove their controls and explicit open manages focus", () => {
  const syncSource = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const drawerSource = readFileSync("src/components/project-sync/ActivityLogDrawer.tsx", "utf8");
  const sidebarSource = readFileSync("src/components/project-sync/ProjectSidebar.tsx", "utf8");

  assert.match(sidebarSource, /id="activity-log-trigger"/);
  assert.match(sidebarSource, /aria-controls="activity-log-drawer"/);
  assert.match(sidebarSource, /aria-expanded=\{activityOpen\}/);
  assert.match(drawerSource, /id="activity-log-drawer"[\s\S]*?role="region"[\s\S]*?aria-label=\{t\("drawer\.label"\)\}[\s\S]*?tabIndex=\{-1\}/);
  assert.equal(en.logs.drawer.label, "Logs");
  assert.match(drawerSource, /\{open && \(\s*<LogPanel/);
  assert.match(syncSource, /requestAnimationFrame\(\(\) => logDrawerRef\.current\?\.focus\(\)\)/);
  assert.match(syncSource, /document\.getElementById\("activity-log-trigger"\)\?\.focus\(\)/);
});

test("live log lines update only the drawer and publish at most once per frame", () => {
  const syncSource = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const drawerSource = readFileSync("src/components/project-sync/ActivityLogDrawer.tsx", "utf8");
  const listener = syncSource.slice(
    syncSource.indexOf('listen<LogLine>("sync-log"'),
    syncSource.indexOf('listen<BundleFetchProgress>("project-sync-fetch-progress"'),
  );

  assert.doesNotMatch(syncSource, /const \[logLines, setLogLines\]/);
  assert.match(syncSource, /const liveLogLinesRef = useRef<LogLine\[\]>\(\[\]\);/);
  assert.doesNotMatch(listener, /setLogLines/);
  assert.match(drawerSource, /const pendingLinesRef = useRef<LogLine\[\]>\(\[\]\);/);
  assert.match(drawerSource, /flushFrameRef\.current = window\.requestAnimationFrame/);
  assert.match(drawerSource, /if \(openRef\.current && logMatchesFilters/);
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

test("the Logs drawer in Chinese: its chrome translated, every line as the backend wrote it", () => {
  const lines = [
    { schema: 1, id: "one", ts: Date.UTC(2026, 9, 7, 13, 5, 9), level: "success" as const, type: "push" as const, event: "push.completed", message: "Pushed 3 sessions to Attic R2" },
    { schema: 1, id: "two", ts: Date.UTC(2026, 9, 7, 13, 6, 0), level: "error" as const, type: "configuration" as const, event: "config.failed", message: "Could not read settings.json" },
  ];
  const panel = () => renderToStaticMarkup(
    <LogPanel
      lines={lines}
      typeFilters={["push", "pull"]}
      levelFilter="all"
      error="Activity log store is unavailable"
      onTypeFiltersChange={() => undefined}
      onLevelFilterChange={() => undefined}
      onSearchChange={() => undefined}
      onManage={() => undefined}
      onClose={() => undefined}
    />,
  );
  const drawer = () => renderToStaticMarkup(
    <ActivityLogDrawer
      open={false}
      height={240}
      refreshEpoch={0}
      scrollToBottomEpoch={0}
      drawerRef={() => undefined}
      onStartResize={() => undefined}
      onResetHeight={() => undefined}
      onManage={() => undefined}
      onClose={() => undefined}
      readLiveLines={() => []}
      subscribeToLiveLines={() => () => undefined}
    />,
  );

  // English first: the type tag is the type's own lowercase word.
  const english = panel();
  assert.match(english, /<span class="log-type log-type-push">push<\/span>/);
  assert.match(english, /<span class="log-type log-type-configuration">configuration<\/span>/);
  assert.match(english, /<span class="sync-visually-hidden">Success<\/span>/);
  assert.match(english, /placeholder="Search logs"/);

  void i18n.changeLanguage("zh-CN");
  try {
    const html = panel();
    assert.match(html, /<span class="log-panel-title">日志<\/span>/);
    assert.match(html, /aria-label="按类型筛选日志：已选 2 个"/);
    assert.match(html, /<span class="log-type-filter-key">类型<\/span>/);
    assert.match(html, />全选</);
    assert.match(html, />全不选</);
    assert.match(html, /<span>上传<\/span>/);
    assert.match(html, /<span>配置<\/span>/);
    assert.match(html, /aria-label="按级别筛选日志"/);
    assert.match(html, /<option value="all" selected="">全部<\/option>/);
    assert.match(html, /<option value="error">错误<\/option>/);
    // Search matches the English lines, and its placeholder says so.
    assert.match(html, /placeholder="搜索日志（仅匹配英文）" aria-label="搜索日志"/);
    assert.match(html, />管理日志</);
    assert.match(html, /title="关闭" aria-label="关闭日志"/);
    // Each line keeps the backend's words; the tag and the hidden level are ours.
    assert.match(html, /<span class="log-type log-type-push">上传<\/span><span class="log-msg">Pushed 3 sessions to Attic R2<\/span>/);
    assert.match(html, /<span class="log-type log-type-configuration">配置<\/span><span class="log-msg">Could not read settings\.json<\/span>/);
    assert.match(html, /<span class="sync-visually-hidden">成功<\/span>/);
    // A failed read: our lead-in, the backend's sentence as it came.
    assert.match(html, /无法载入保留的日志：Activity log store is unavailable/);
    assert.match(html, /aria-label="关闭：错误。无法载入保留的日志：Activity log store is unavailable。"/);
    assert.match(drawer(), /role="region" aria-label="日志"/);
    assert.match(renderToStaticMarkup(<LogPanel lines={[]} onClose={() => undefined} />), />没有符合筛选条件的记录。</);
  } finally {
    void i18n.changeLanguage("en");
  }
  assert.match(drawer(), /role="region" aria-label="Logs"/);
});
