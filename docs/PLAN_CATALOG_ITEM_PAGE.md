# A Catalog Item's Page, Like Mallard Cloud's

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: implemented October 6, 2026 (owner picked A, every storage, the desk
for pets). Live Tauri pass against real provider homes not yet run. Specimen:
`mockups/catalog-item-page-mockup.html`. Nothing in `src/` or `src-tauri/`
has changed.

## Problem

On Mallard Cloud a skill, plugin or pet has a page written for its kind. In
the app an item has a page only on Mallard Cloud, and that page is its list
card (`StorageCatalogPage.tsx`, `itemPage`).

## Rule

Each fact is said once. The bar already says the name (leaf), kind and
provider (scope crumb), sync state (Push / Pull counts) and the repository
(Cloud chip). The page repeats none of them: no title row, no kind tile, no
status or provider chips, no repository card.

## Recommendation: A · Rail

- **Panel**: the kind's body, this Mac's copy. No copy switch.
- **Rail** (260px, from 880px of page): Edited, Size, then the icon actions,
  right-aligned: Open in Claude/Codex, Terminal, Fork, Reveal in Finder.
- **Under 880px** (any window under ~1300px with the default sidebar): the
  rail becomes one line of chips above the panel.

Bodies, from the website:

| Kind | Body |
| --- | --- |
| Skill | Description, SKILL.md rendered, file list (S1) |
| Plugin | Name by developer · category, description, Installed · version (or Pull installs it), install box, Source and links |
| Pet | The desk with moments and Pause (T2), description |

Other directions in the specimen: B · Hero (App Store), C · Tabs (JetBrains),
D · Inspector (JetBrains, HIG split views), E · Sheet.

## Decisions

A · Rail; every storage; the desk for pets.

## What changes

- **No schema, manifest or Mallard Cloud API change.** SKILL.md, the file
  list, the pet atlas and its description are already in each item's
  payload; a plugin's words come from its installed `plugin.json`.
- **One Tauri command**, `get_catalog_resource_detail(scopeId, storageId,
  resourceId)`: this Mac's capture when present, else the stored copy through
  the verified fetch `get_catalog_resource_preview` uses — so stored-only items
  on every storage get the page. One codec method, `detail()`, beside
  `preview()`.
- A plugin that is not installed here shows its `name@marketplace` and
  install box only; its developer and description need the installed
  manifest.
- Before an item's first Push to Mallard Cloud, today's "New repository ·
  Change…" card stays above the panel; it is the only place to choose a
  different repository.

## Implementation

### Layer 1

1. Move the Markdown renderer out of `project-sync/session-detail/` to
   `src/components/markdown/` (the module-boundary test forbids
   storage-catalog importing project-sync). Images in SKILL.md render as
   placeholders, never fetched.
2. Codec `detail()` beside `preview()` and
   `get_catalog_resource_detail(scopeId, resourceId)` (this Mac's copy):
   SKILL.md (≤ `MAX_SKILL_MD_BYTES`) and the file list; a plugin's intent,
   listing and installed version (reusing `plugin_listing.rs`, committed in
   7235c4b, widened to `pub(crate)`; the version is the inventory's
   `observed_version`). Register in
   `lib.rs`; DTOs in Rust and `types.ts` together.
3. `CatalogItemPage.tsx` replaces `itemPage`. `itemRow` and `onOpen` drop
   their `managedCloud` gate; the Cloud status hook and chip keep theirs. Per-item Push and Pull already
   work on every storage (`openReview` ticks one row).

### Layer 2

4. The pet atlas, in the same `detail()` read, through the same digest, size and pinned-dimension checks as
   `pet_preview.rs`; port the website's player; Pause, paused under Reduce
   Motion.

## Safety

No schema or manifest change. Reads stay bounded and no-follow. Markdown has no raw HTML; links open externally.

## Tests

Rust: each codec's `detail` (bounds, symlinks, wrong atlas dimensions).
Frontend: the page per kind, the fold by computed style at 879/880px, the boundary test. `npm run build`,
`npm run test:frontend-integration`, `cargo test --manifest-path
src-tauri/Cargo.toml --lib`.

## Sources

Primer PageLayout; Apple HIG Toolbars, Split views, Motion; NN/g Tabs used
right; Carbon Code snippet; WCAG 2.2.2; VS Code `extensionEditor.ts`;
JetBrains plugins; Raycast Store; Mac App Store; Claude Code plugin docs.

## What was built

- Backend: codec `detail()` beside `preview()` (skill: `SKILL.md` ≤ 256 KiB
  and the file list; plugin: the intent; pet: the atlas, digest- and
  dimension-checked, as a data URI); `get_catalog_resource_detail` reads this
  Mac's capture first and asks no storage, else the storage's verified copy
  (`stored_scope_resource`, shared with the stored-only preview). A plugin's
  install lines are its adapter's own `plan_native_apply` for an empty home
  behind the provider CLI, printed only when every word is shell-plain; an
  installed plugin adds `observed_version` and its `plugin.json` listing.
- Frontend: `src/components/markdown/` (`MarkdownText`, `markdownUrl`,
  `remarkLatexMath`) moved out of the session page; `.markdown-copy` carries
  the shared typography. `CatalogItemPage` (body panel, rail from 880px of
  page, one line under it) and `CatalogPetDesk` (Mallard Cloud's player) in
  `storage-catalog/`; `StorageCatalogPage` opens an item on every storage and
  keeps the repository card and several-repositories note Mallard Cloud's.
- The provider-specific install syntax lives in the backend adapter, not the
  frontend: the Catalog's no-provider-branch test stands.
- Live harness: `mockups/catalog-item-live.html`.

## Verification

`cargo test --manifest-path src-tauri/Cargo.toml --lib` (549 passed),
`npm run build`, `npm run test:frontend-integration` (601 passed). Clippy on
the app crate reports nothing in the changed code. It fails on code committed
since it was last green (`mallard-domain` `nonminimal_bool` from 0c6813e,
2026-09-09; `apply_push_with_repositories` at 9 arguments since 8577306,
2026-10-02; about fifty more, mostly `needless_borrow` in
`project_sync/service.rs`).
