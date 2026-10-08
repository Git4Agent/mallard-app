# Filter the Library by status

- Status: the owner picked **L1** on 2026-10-07. Implemented the same day,
  uncommitted. *As built* at the end records what landed. Open: a pass in the
  running Tauri app.
- Scope: the Library list (`StorageCatalogPage`, list view). The item page and
  the Push/Pull reviews get no control.
- Visual contract: `mockups/catalog-filter-mockup.html`
  (`?theme=light|dark&only=vault|cloud|empty|narrow`). Every frame is the
  shipped `StorageCatalogPage` on a stand-in backend; presets are typed and
  clicked into the real page.
- Screens (gitignored): directions in
  `output/catalog-status-filter-mock-2026-10-07/`, shipped in
  `output/catalog-status-filter-2026-10-07/`.

## Why the Library has no filter

Deliberate, not a regression. `PLAN_SESSION_STATUS_FILTER.md` and
`PLAN_FILES_STATUS_FILTER.md` both list the Library under *Not in this
change*, and `StatusScopeBar` was never wired into `StorageCatalogPage`. The
list only has the name search.

## Directions

| | Form | Seen in | Against |
|---|---|---|---|
| **L1 (recommended)** | The Sessions/Files strip under the search | this app's Sessions and Files; Mail/Finder scope bars; GitHub Open/Closed | one more row |
| L2 | One *Status ▾* button and a radio menu in the search row | Mail's Filter, Linear, Primer filter pattern | hides counts; unlike Sessions, Files |
| L3 | L1, and the bar's attention chip selects *Attention* | VS Code status bar → Problems; Xcode issue counts | Sessions' chip doesn't, yet |

References for the strip form: the table in `PLAN_SESSION_STATUS_FILTER.md`.

## The mapping

One `catalogStatusScope(row)` beside `rowStatus` in `model.ts`, read by both
the strip and the bar's Push / Pull / attention counts. It sorts by the badge
the row draws, so the numbers the bar shows today do not change:

| Badge (`rowStatus`) | States | Scope |
|---|---|---|
| local-outstanding | `new`, `local_changed`, `remote_missing` | To push |
| cloud-outstanding | `storage_changed`, `local_missing` | To pull |
| divergent, unavailable | `diverged`, `unreviewed_remote`, `blocked` | Attention |
| synced | `in_sync` | Up to date |
| untracked | no state | Not compared |

Rows have no state only when no storage is chosen or the storage refused the
account, and then every row has none — so *Not compared* never shares the
strip with another scope, and the strip never shows there.

Noted while mapping, not confirmed reachable: a `remote_missing` row's badge
says "Gone from storage. Push to put it back.", but the Push plan holds it
(`selectable` is only `new` / `local_changed`) and Pull keeps the local copy,
while the bar counts it under Push. The state needs an accepted base, a local
copy, and a stored snapshot that continues the accepted history without the
item; a missing Mallard Cloud item stream or a replaced history drops the base
instead (the row reads `new`). No user path to it was traced. The strip
follows the badge either way.

## L1 implementation

1. `catalogStatusScope` in `model.ts`; the page's `counts` reduce through it.
2. `statusScope` state beside `searchQuery`, reset by the same effect (kind,
   storage, review mode). It survives opening an item and coming back, as the
   search does.
3. `matchingRows` filters by scope and name; the "Only in" / "Not downloaded"
   split stays, and its heading count is the narrowed count.
4. Render `<StatusScopeBar label="Show {plural} by status" …>` right after the
   list's `NameSearchField` (pass `narrowed`). The Sessions rules: drawn with
   two or more scopes or while one is chosen; a chosen scope stays at 0;
   *Clear filter* only when releasing the scope brings rows back.
5. CSS: none. Measured in the specimen: the strip sits flush under the field
   (0px) and takes over its rule on both heads (Vault and Mallard Cloud's
   `.sync-list-head` with *Add from Mallard Cloud…*), and the first heading
   keeps the list's existing 18px.

No DTO, command, or Rust change.

## Tests

In `tests/frontend/storage-catalog.integration.test.tsx` (which also forbids provider branches in the Library frontend): no strip with one scope or no
storage; each scope's count equals the rows it draws; strip push/pull/attention
equal the bar's counts; scope and name both apply and an emptied section
leaves with its heading; a chosen scope with no rows stays at 0 with *Clear
filter* only when clearing brings rows back. Handoff: `npm run build`,
`npm run test:frontend-integration`.

## As built (2026-10-07)

| Step | Landed in |
|---|---|
| One mapping | `catalogStatusScope`, `catalogStatusScopeCounts`, `catalogMatchingRows` beside `rowStatus` in `storage-catalog/model.ts`; the page's `counts` reduce through `catalogStatusScope` |
| State | `chosenStatusScope` beside `searchQuery`, cleared by the same effect |
| Narrowing | `matchingRows = catalogMatchingRows(rows, searchQuery, statusScope)`; the two sections split it as before |
| The strip | `StatusScopeBar` after the list's `NameSearchField` (`narrowed` passed), the Sessions/Files show rule, *Clear filter* via `scopeEmptiedList` with focus handed to the field |
| CSS | none |
| Tests | four in `storage-catalog.integration.test.tsx`: the mapping (and one-scope when uncompared), counts equal drawn rows and the bar reads the same scopes, scope + name narrowing, the page wiring |

Differences from the plan: the narrowing is a pure helper in `model.ts`
rather than inline, because the suite renders the page as static markup and
never reaches a loaded list.

Verified on the shipped page in the specimen (Chromium): Vault and Mallard
Cloud heads, the strip flush under the field (0px) with 18px to the first
heading, one row of radios at 580px; strip counts equal the bar's (Push 3,
Pull 4, attention 3); *Clear filter* releases the scope, keeps the query and
focuses the field; the scope survives an item's page and back; arrows move
the scope. `npm run build` passes; `npm run test:frontend-integration` passes
619 of 619.
