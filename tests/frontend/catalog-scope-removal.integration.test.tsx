import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { catalogScopeRemovalMessage } from "../../src/components/storage-catalog/model";

/* The Catalog refuses a row removal for two reasons and words both for a log.
   The frontend rewrites them, which means matching on backend prose — a
   contract no compiler defends. These gates are what defends it: the strings
   below are copied verbatim from resource_commands.rs, and the last test reads
   that file to prove they are still the ones it produces. */

const LINKS_STORAGES = "this row still links storages; unlink them first";
const ROW_CHANGED = "this Catalog row changed; refresh first";
const LABEL = "Pets v2 · Default Codex";

test("a row that still links storages gives the ordered cleanup", () => {
  const message = catalogScopeRemovalMessage(LABEL, new Error(LINKS_STORAGES));

  assert.equal(
    message,
    "Unlink every storage from “Pets v2 · Default Codex”, then remove it again.",
  );
  // The backend's wording is replaced by the actual UI action.
  assert.doesNotMatch(message, /\brow still links\b/);
  assert.match(message, /Unlink every storage/);
});

test("a stale row names the row and the one action that clears it", () => {
  const message = catalogScopeRemovalMessage(LABEL, new Error(ROW_CHANGED));

  assert.equal(
    message,
    "“Pets v2 · Default Codex” changed somewhere else. Refresh the Catalog, then try removing it again.",
  );
});

test("anything else passes through rather than being guessed at", () => {
  const message = catalogScopeRemovalMessage(LABEL, new Error("the catalog store is unreadable"));
  assert.equal(message, "the catalog store is unreadable");
});

test("a failed removal opens the affected row's storage menu", () => {
  const shell = readFileSync("src/components/project-sync/ProjectSync.tsx", "utf8");
  const page = readFileSync("src/components/storage-catalog/StorageCatalogPage.tsx", "utf8");
  const pathBar = readFileSync("src/components/sync-review/SyncPathBar.tsx", "utf8");
  assert.match(shell, /catalogRemovalErrorTarget\?\.message === error/);
  assert.match(shell, /openCatalogScope\(catalogRemovalErrorTarget\.scopeId\)/);
  assert.match(shell, />\s*Open storage menu\s*<\/button>/);
  assert.match(page, /\{ menuId: "storage", requestId: openStorageMenuRequestId \}/);
  assert.match(pathBar, /setOpenMenu\(openMenuRequest\.menuId\)/);
});

test("both refusals are still worded the way the translation expects", () => {
  // If a backend cleanup rewords either string, the rewrite silently stops
  // matching and the raw prose reaches the user again. Fail here instead.
  const commands = readFileSync("src-tauri/src/storage_catalog/resource_commands.rs", "utf8");
  assert.ok(
    commands.includes(`"${LINKS_STORAGES}"`),
    "resource_commands.rs no longer produces the string catalogScopeRemovalMessage matches on",
  );
  assert.ok(
    commands.includes(`"${ROW_CHANGED}"`),
    "resource_commands.rs no longer produces the stale-row string",
  );
});

test("the shell error banner is a panel in the workspace stack", () => {
  // It renders as a sibling of the path bar and the page, so it has to carry
  // their width and frame. It used to inset itself 14px a side with its own
  // radius, which read as a foreign box sitting above two aligned ones.
  const appCss = readFileSync("src/App.css", "utf8");
  const retroCss = readFileSync("src/RetroTheme.css", "utf8");

  assert.match(appCss, /\.sync-backend-banner \{[^}]*margin: 0 0 10px;/);
  assert.doesNotMatch(appCss, /\.sync-backend-banner \{[^}]*margin: 10px 14px 0;/);
  assert.match(
    retroCss,
    /\.sync-workspace > \.sync-backend-banner,\s*\n\s*\.sync-workspace > \.sync-main,/,
    "the banner must sit in the same frame group as the page it precedes",
  );
  assert.match(
    retroCss,
    /\.sync-workspace > \.sync-backend-banner \{\s*\n\s*margin-bottom: var\(--retro-gutter\);/,
  );

  // Opaque, mixed onto paper. A translucent tint sits on the frame gutter
  // instead, which took the error text to 3.08:1 — under the 4.5 this palette
  // picks its ink against, and under the bar css-hygiene enforces elsewhere.
  assert.match(appCss, /background: color-mix\(in srgb, var\(--red\) 4%, var\(--surface-card\)\);/);
  assert.doesNotMatch(
    appCss,
    /\.sync-backend-banner\.sync-shell-error \{[^}]*background: var\(--danger-soft\)/,
  );
});
