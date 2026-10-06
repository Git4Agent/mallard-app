import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import SyncPathBar, { PathBarBelow } from "../../src/components/sync-review/SyncPathBar";
import type { NoticeZoneProps } from "../../src/components/notice/Notice";
import { connectionDetail, connectionSuspends } from "../../src/components/project-sync/StorageSettings";
import StorageIndexPage from "../../src/components/project-sync/StorageIndexPage";
import SettingsPage from "../../src/components/project-sync/SettingsPage";
import type { AppUpdates } from "../../src/components/AppUpdater";

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

const zone: NoticeZoneProps = {
  items: [{ key: "receipt", severity: "success", message: "backup removed — files kept on disk." }],
  subject: "messages from Mallard",
  className: "sync-backend-banner",
  timing: "progress",
};

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
      <SettingsPage appActions={[]} appUpdates={idleUpdates} onLogsChanged={() => undefined} onDone={() => undefined} doneTarget="Projects" />
    </PathBarBelow.Provider>,
  );
  assertOrder(settings, BAR, ZONE, PAGE);
});

test("the shell keeps its own titlebar first and hands the zone to path-bar screens", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const titlebar = shell.indexOf('<header className="sync-titlebar" data-tauri-drag-region>');
  const underTitlebar = shell.indexOf("<NoticeZone {...shellNoticeZone} />");
  const handedDown = shell.indexOf("<PathBarBelow.Provider value={showsPathBar ? shellNoticeZone : null}>");
  assert.ok(titlebar > -1, "the shell draws a titlebar for screens without a path bar");
  assert.ok(underTitlebar > titlebar, "the zone follows the shell's own titlebar");
  assert.ok(handedDown > underTitlebar, "path-bar screens receive the zone through the bar");

  // One zone, built once, drawn in exactly one of the two places.
  assert.equal((shell.match(/<NoticeZone \{\.\.\.shellNoticeZone\} \/>/g) ?? []).length, 1);
  assert.equal((shell.match(/className: "sync-backend-banner"/g) ?? []).length, 1);
});

// Render the production storage notice selection and its actual path bar.
// Commands and provider homes are not used by this fixture.
function storageBanner(options: {
  bundleError?: string;
  catalogRepoError?: string;
  storageDraftActionError?: string;
  refused?: boolean;
  bundleLoading?: boolean;
}, shell: NoticeZoneProps = { ...zone, items: [] }) {
  const source = readFileSync("src/components/project-sync/ProjectLinksWorkspace.tsx", "utf8");
  const start = source.indexOf("    const storageNotices: NoticeItem[] = [];");
  const end = source.indexOf("    /* On an existing storage", start);
  const barStart = source.indexOf("      <SyncPathBar", end);
  const barEnd = source.indexOf("      />", barStart) + "      />".length;
  assert.ok(start >= 0 && end > start && barStart > end && barEnd > barStart);
  const pageEnd = source.indexOf("    const workspaceNotices", barEnd);
  assert.doesNotMatch(source.slice(barEnd, pageEnd), /items=\{storageNotices\}/, "no second storage error box in page content");
  const context = {
    React: { createElement }, SyncPathBar, connectionDetail, connectionSuspends,
    storageDraftActionError: options.storageDraftActionError ?? null,
    storageSession: null, descriptor: null, providerLabelForPage: "Mallard Cloud",
    savedEditedStorage: { connection: options.refused ? {
      state: "reconnect_required", message: "Reconnect Mallard Cloud.", evidence: null,
    } : null },
    creatingStorage: false, bundleLoading: options.bundleLoading ?? false,
    bundleError: options.bundleError ?? null, catalogRepoError: options.catalogRepoError ?? null,
    revealError: null, pageStorageId: "storage-test",
    storagePathCrumbs: [{ id: "storage-test", label: "Test storage" }],
    storagePathOverflow: [], busy: false, storagePathActions: null, storageName: "Test storage",
  };
  const { transformSync } = createRequire(`${process.cwd()}/package.json`)("esbuild") as typeof import("esbuild");
  const code = transformSync(`${source.slice(start, end)}\nreturn (${source.slice(barStart, barEnd)});`, { loader: "tsx" }).code;
  const render = new Function("context", `const { ${Object.keys(context).join(", ")} } = context; ${code}`);
  return renderToStaticMarkup(
    <PathBarBelow.Provider value={shell}>
      {render(context)}
      <main className="sync-main">Storage settings</main>
    </PathBarBelow.Provider>,
  );
}

test("storage repository and catalog failures reach the shared banner before page content", () => {
  for (const options of [
    { bundleError: "Mallard Cloud rejected the saved sign-in. HTTP 401." },
    { catalogRepoError: "Catalog repositories could not be read." },
  ]) {
    const html = storageBanner(options);
    assertOrder(html, BAR, 'class="notice danger sync-backend-banner"', PAGE);
    assert.match(html, /role="region" aria-label="Messages from Mallard"/);
    const message = options.bundleError ?? options.catalogRepoError!;
    assert.ok(html.includes(message));
    assert.equal((html.match(/class="sync-notice-dismiss"/g) ?? []).length, 1);
  }
});

test("storage and shell failures share one banner with blocking announcements", () => {
  const html = storageBanner({ bundleError: "Storage read failed." }, {
    ...zone, timing: "blocking",
    items: [{ key: "shell-error", severity: "danger", message: "Rename failed." }],
  });
  assertOrder(html, BAR, 'class="notice danger sync-backend-banner"', PAGE);
  assert.match(html, /role="alert"/);
  assert.match(html, /Storage read failed\./);
  assert.match(html, /Rename failed\./);
  assert.match(html, /aria-label="Dismiss: all 2 messages from Mallard"/);
  assert.match(html, /aria-label="Dismiss: Error\. Storage read failed\."/);
  assert.match(html, /aria-label="Dismiss: Error\. Rename failed\."/);
  const actionFailure = storageBanner({ storageDraftActionError: "Draft save failed." }, zone);
  assert.match(actionFailure, /role="alert"/);
  assert.match(actionFailure, /Draft save failed\./);
  assert.match(actionFailure, /backup removed/);
});

test("the shared storage banner preserves refusal precedence and clears after a successful read", () => {
  const refused = storageBanner({ refused: true, bundleError: "Duplicate listing failure." });
  assert.match(refused, /Reconnect Mallard Cloud\./);
  assert.doesNotMatch(refused, /Duplicate listing failure/);
  assert.doesNotMatch(storageBanner({ bundleError: "Previous failure.", bundleLoading: true }), /sync-backend-banner|Previous failure/);
  assert.doesNotMatch(storageBanner({}), /sync-backend-banner/);
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
