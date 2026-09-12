import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import SyncPathBar, { PathBarBelow } from "../../src/components/sync-review/SyncPathBar";
import { NoticeZone } from "../../src/components/notice/Notice";
import StorageIndexPage from "../../src/components/project-sync/StorageIndexPage";
import SettingsPage from "../../src/components/project-sync/SettingsPage";

/* The shell's notice zone — backend failures, action errors, the "Done ·"
 * receipts — is one box for the whole workspace, and it sits UNDER the top bar
 * on every screen. The bar is the window's title bar (traffic lights and all),
 * so nothing goes above it, and the trail is read before the message: the
 * order Material, Fluent, Polaris and GOV.UK all place a page banner in.
 *
 * Screens that draw their own path bar cannot be interleaved by the shell, so
 * the zone reaches them through `PathBarBelow` and the bar renders it right
 * after its own row. The shell's own titlebar keeps the zone as its next
 * sibling. Either way the DOM order is bar, zone, page — the same order the
 * eye and the tab sequence follow. */

const zone = (
  <NoticeZone
    items={[{ key: "receipt", severity: "success", message: "backup removed — files kept on disk." }]}
    subject="messages from Mallard"
    className="sync-backend-banner"
    timing="progress"
  />
);

const BAR = 'class="sync-pathbar sync-titlebar"';
const ZONE = 'class="notice success sync-backend-banner"';
const PAGE = '<main class="sync-main';

function assertOrder(html: string, ...needles: string[]) {
  const positions = needles.map((needle) => {
    const at = html.indexOf(needle);
    assert.notEqual(at, -1, `expected ${needle} in the markup`);
    assert.equal(html.indexOf(needle, at + 1), -1, `${needle} must appear once`);
    return at;
  });
  for (let index = 1; index < positions.length; index += 1) {
    assert.ok(positions[index - 1] < positions[index], `${needles[index - 1]} must precede ${needles[index]}`);
  }
}

test("the path bar renders the shell zone right after its own row", () => {
  const crumbs = [{ id: "storages", label: "Storages" }];
  const html = renderToStaticMarkup(
    <PathBarBelow.Provider value={zone}>
      <SyncPathBar crumbs={crumbs} />
    </PathBarBelow.Provider>,
  );
  assertOrder(html, BAR, "</header>", ZONE);

  // Nothing handed down, nothing drawn: the bar stands alone.
  const bare = renderToStaticMarkup(<SyncPathBar crumbs={crumbs} />);
  assert.doesNotMatch(bare, /sync-backend-banner/);
});

test("a path-bar screen flattens to bar, zone, page in the workspace column", () => {
  const storages = renderToStaticMarkup(
    <PathBarBelow.Provider value={zone}>
      <StorageIndexPage
        storages={[]}
        links={[] as never}
        storageIssues={{}}
        drafts={[]}
        storageProviders={[]}
        busy={false}
        appActions={[]}
        onOpenStorage={() => undefined}
        onOpenStorageDraft={() => undefined}
        onAddStorage={() => undefined}
      />
    </PathBarBelow.Provider>,
  );
  assertOrder(storages, BAR, ZONE, PAGE);

  const settings = renderToStaticMarkup(
    <PathBarBelow.Provider value={zone}>
      <SettingsPage appActions={[]} onLogsChanged={() => undefined} onDone={() => undefined} doneTarget="Projects" />
    </PathBarBelow.Provider>,
  );
  assertOrder(settings, BAR, ZONE, PAGE);
});

test("the shell keeps its own titlebar first and hands the zone to path-bar screens", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const titlebar = shell.indexOf('<header className="sync-titlebar" data-tauri-drag-region>');
  const underTitlebar = shell.indexOf("{shellNoticeZone}");
  const handedDown = shell.indexOf("<PathBarBelow.Provider value={showsPathBar ? shellNoticeZone : null}>");
  assert.ok(titlebar > -1, "the shell draws a titlebar for screens without a path bar");
  assert.ok(underTitlebar > titlebar, "the zone follows the shell's own titlebar");
  assert.ok(handedDown > underTitlebar, "path-bar screens receive the zone through the bar");

  // One zone, built once, drawn in exactly one of the two places.
  assert.equal((shell.match(/\{shellNoticeZone\}/g) ?? []).length, 1);
  assert.equal((shell.match(/className="sync-backend-banner"/g) ?? []).length, 1);
});

test("the shell zone stacks under the bar so the bar's menus open over it", () => {
  const css = readFileSync("src/App.css", "utf8");
  const zIndex = (selector: string) => {
    const block = css.match(new RegExp(`\\n${selector.replace(/\./g, "\\.")} \\{([^}]*)\\}`))?.[1];
    assert.ok(block, `${selector} has a rule`);
    const value = block.match(/z-index: (\d+);/)?.[1];
    assert.ok(value, `${selector} sets z-index`);
    return Number(value);
  };
  const banner = zIndex(".sync-backend-banner");
  assert.ok(banner < zIndex(".sync-pathbar"), "the banner paints under the path bar");
  assert.ok(banner < zIndex(".sync-titlebar"), "the banner paints under the titlebar");
});
