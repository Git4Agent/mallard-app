import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { catalogScopeRemovalMessage } from "../../src/components/storage-catalog/model";

/* The Catalog refuses a row removal for two reasons. The service tests pin
   the backend messages; these tests pin their user-facing translations. */

const LINKS_STORAGES = "this row still links storages; unlink them first";
const ROW_CHANGED = "this Catalog row changed; refresh first";
const LABEL = "Pets v2 · Default Codex";

test("Catalog scope removal errors use actionable messages", () => {
  assert.equal(
    catalogScopeRemovalMessage(LABEL, new Error(LINKS_STORAGES)),
    "Unlink every storage from “Pets v2 · Default Codex”, then remove it again.",
  );
  assert.equal(
    catalogScopeRemovalMessage(LABEL, new Error(ROW_CHANGED)),
    "“Pets v2 · Default Codex” changed somewhere else. Refresh the Catalog, then try removing it again.",
  );
  assert.equal(
    catalogScopeRemovalMessage(LABEL, new Error("the catalog store is unreadable")),
    "the catalog store is unreadable",
  );
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

test("the shell error banner is a panel in the workspace stack", () => {
  // It renders as a sibling of the path bar and the page, so it has to carry
  // their width and frame. It used to inset itself 14px a side with its own
  // radius, which read as a foreign box sitting above two aligned ones.
  const css = readFileSync("src/App.css", "utf8");

  assert.match(css, /\.sync-backend-banner \{[^}]*margin: 0 0 10px;/);
  assert.doesNotMatch(css, /\.sync-backend-banner \{[^}]*margin: 10px 14px 0;/);
  assert.match(
    css,
    /\.sync-workspace > \.sync-backend-banner,\s*\n\s*\.sync-workspace > \.sync-main,/,
    "the banner must sit in the same frame group as the page it precedes",
  );
  assert.match(
    css,
    /\.sync-workspace > \.sync-backend-banner \{\s*\n\s*margin-bottom: var\(--retro-gutter\);/,
  );

  // Opaque, mixed onto paper. A translucent tint sits on the frame gutter
  // instead, which took the error text to 3.08:1 — under the 4.5 this palette
  // picks its ink against, and under the bar css-hygiene enforces elsewhere.
  assert.match(css, /background: color-mix\(in srgb, var\(--red\) 4%, var\(--surface-card\)\);/);
  assert.doesNotMatch(
    css,
    /\.sync-backend-banner\.sync-shell-error \{[^}]*background: var\(--danger-soft\)/,
  );
});
