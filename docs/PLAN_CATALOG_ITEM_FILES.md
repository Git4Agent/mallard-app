# Opening a Skill's Files: B · Crumb, Desktop and Mallard Cloud

Status: the owner picked **B · Crumb** for both the desktop app and the
website. Both are implemented, uncommitted, as of October 8, 2026.
- **Desktop**: a live Tauri pass on real Codex and Claude homes is still open.
- **Mallard Cloud** (`web/src/skillFiles.ts`, `FileView.tsx`, `SkillFile.tsx`,
  `usePopover.ts`): the real components were checked on stub bytes, but a
  staging pass with live data is still open.
- **Both**: opening a file starts the page at its top.

Specimens:
- **Desktop**: `mockups/catalog-file-open-mockup.html?open=b` (mockups Vite,
  port 1437, or 1420). It holds every direction considered; B is the pick.
- **Cloud**: `mallardInternal/cloudflare/managed-cloud/web/catalog-file-mockup.html`
  (website Vite, port 5173). It shows B only: the item page and the file page,
  at 1200px and 375px.

## Problem (debugged)

The file rows under SKILL.md on a skill's page can't be opened, on either
product. Nothing is broken; opening a file was never built.
- **Desktop**:
  - `CatalogItemPage.tsx` `FileList` prints each path as `<code>` in a table
    cell, with no handler.
  - `CatalogResourceDetail::Document` carries only path, size and executable.
  - Only SKILL.md's bytes cross the IPC.
- **Cloud**: `CatalogItem.tsx` `FileTable` prints path, size and mode, all
  inert, though the bytes are one `loadObject(source, sha256)` away.

It affects every Codex and Claude skill (one `agentskills.io.skill` codec) on
every storage. Plugins and pets list no files.

## The design, on both products

The file is a page.

1. **Opening**: a row on the item page, or a relative link in SKILL.md or in
   an opened `.md` file, opens the file's page.
2. **The trail**: the file's base name becomes the last crumb, with the full
   path in its title. The item crumb before it is the way back.
3. **The crumb's menu switches files.**
   - Header: the full path.
   - A filter field when the skill has six files or more.
   - Then every other file: name, folder, size.
4. **The page body** is the file view.
   - Head: the file's facts, then the tools, right-aligned. The name is not
     repeated; the crumb says it.
   - Body, by the bytes:
     - **Numbered lines**, in their own horizontally scrolling box. A copy
       leaves the numbers out.
     - **Markdown**: Preview or Source. SKILL.md opens in Source, because its
       Preview is the item page.
     - **Not text**: a placeholder.
5. **What counts as text, on both products**: the bytes decode as strict
   UTF-8 and hold no NUL byte. The extension only picks Markdown rendering.
   This matters for files the Cloud's extension lists don't know: `.pyc` stays
   binary by its bytes, and extensionless scripts, `LICENSE` or `.tmpl` files
   read as text.
6. **Relative links**:
   - A link resolves against the linking file's own folder.
   - `..` is allowed, but never above the skill's root.
   - It opens only a file in the list; anything else stays text.
   - Absolute paths, schemes and `#` anchors keep today's behaviour.
7. **Never**: running a file, syntax colour, or a hidden-Unicode banner.
   Colour and the banner are possible later layers.

| | Desktop | Mallard Cloud |
| --- | --- | --- |
| Where the open file lives | `itemFile` state beside `itemId` | the address, `?file=<path>` on the Item tab |
| Trail | `SyncPathBar` leaf crumb. Its switcher (`menuHeader` + a labelled group, search from six) ships today, as on the storage page's leaf | `Trail`'s last crumb gains an optional menu, the one new piece, on the version pop-up's box (`.ver-menu`) |
| Facts | executable glyph, `N lines · size` | size, sha256 chip, mode (the gallery's `file-facts`) |
| Tools | Preview / Source, Copy, Reveal in Finder | Preview / Source, Download |
| Text shown up to | 2 MiB | 512 KiB |
| Not text | "Binary file · not shown" | "No preview for this kind of file. Download it to open it." (shipped words) |
| Bytes from | one new command | the existing objects route, by digest |

**Why the text caps differ.** 2 MiB is the per-file capture cap, and the
desktop has no Download to fall back on. The website keeps its gallery's
512 KiB, with Download beyond it.

## Desktop

### Backend: one command, no schema or manifest change

```
get_catalog_resource_file(scope_id, storage_id?, resource_id, path)
  -> CatalogResourceFile { path, size, executable, text: Option<String>, local_path: Option<String> }
```

- **Resolution**: one helper, shared with `get_catalog_resource_detail`.
  - This Mac's copy comes from the inventory capture's payloads; no storage
    is asked.
  - Otherwise, `stored_scope_resource` and `engine.fetch_payload` are used,
    verified by digest.
- **Lookup**: an exact match of `path` in `envelope.files`. The path is
  never joined onto a directory to read it. A path not listed is refused.
- **Read**: `read_detail_payload(file, MAX_DETAIL_FILE_BYTES, …)`, with a new
  2 MiB constant in `codec.rs`. The read checks size and SHA-256 whatever
  the source.
- **`text`**: `None` unless the bytes are strict UTF-8 with no NUL byte.
- **`local_path`**: `Path::join` of the row's folder and the envelope's
  validated relative path. It is filled only for this Mac's copy, and it is
  used only by Reveal.
- **Kept out**: no disk cache, no logging of contents, no new codec method.
- **Wiring**:
  - Register the command in `lib.rs`.
  - Add the DTO in Rust and `storage-catalog/types.ts` together.
  - Add `getFile` to `storageCatalogApi`.
  - Extend the pinned command list and the `storageCatalogApi` method list in
    `storage-catalog.integration.test.tsx`.
- **Measured**: `inventory_local` re-captures the whole profile home on every
  call, as the page read already does. On a fixture home of 101 skills × 10
  files (~4 MB), one open took ~125 ms median in a debug build, an upper
  bound for release. That is under the 150 ms line, so there is no
  single-resource capture.

### Frontend

- **`StorageCatalogPage`**:
  - Add `itemFile: string | null`, cleared when the item, storage or scope
    changes.
  - The item crumb gets `onActivate` (back).
  - The file crumb gets `menuHeader` (full path) and one labelled group,
    *Files*, holding the other files.
- **`CatalogItemPage`**: `FileList` rows become buttons (`onOpenFile`). With
  a file open, the page is the file view alone, with no rail.
- **`CatalogFileView.tsx`** (new):
  - The head and the three bodies.
  - Its data comes from `useCatalogItemFile(scope, storage, row, path)`,
    keyed like `useCatalogItemDetail` plus the path. A Pull or Push that
    changes the content reads the file again.
  - A path gone after a refresh shows the standing "Couldn't read" notice.
- **`MarkdownText`**:
  - New optional `resolveLink(href) => path | null`.
  - While it is given, `urlTransform` keeps an href it resolves, so `../x`
    survives `safeMarkdownUrl`.
  - The `a` override opens that path.
  - The resolver is a pure helper in `storage-catalog/` that knows the file
    list.
- **CSS**: a `.catalog-file-*` block beside `.catalog-item-*` in `App.css`.
  The Preview / Source switch is the shipped `.sync-segmented`.
- **No provider branches** (the Catalog test forbids them).

### Tests

- **Rust, beside the helper**:
  - Read by exact path from the local capture.
  - Read from the stored copy, with a tampered payload refused.
  - Refused paths: unknown, `../x`, absolute, and a different case.
  - UTF-8 with a NUL byte, and invalid UTF-8, give `text: None`.
  - Larger than the cap is refused.
  - `local_path` is absent for a stored read.
- **Frontend**:
  - Rows are buttons.
  - The file crumb and its menu render.
  - The item crumb goes back.
  - Numbered lines show.
  - Preview / Source, with SKILL.md opening in Source.
  - The binary placeholder shows.
  - Reveal is off for a stored-only copy.
  - The resolver: `references/a.md` → `../scripts/x.py` opens; `../../x` and
    an unlisted file stay text.

## Mallard Cloud (`mallardInternal/cloudflare/managed-cloud/web`)

No API, D1 or Worker change. `CatalogEnvelope.files[]` already carries
`path`, `sha256`, `size` and `mode`. The objects route already serves a
payload by digest to collaborators, and to public readers for the
publication they read.

- **Address**:
  - `?file=<path>` on the Item tab. Opening pushes a history entry, so Back
    returns to the item, and the address can be shared.
  - Every file address is built from the current params, as `setPlace`
    already does: set or delete `file` only, and keep `version`. This covers
    rows, menu entries, SKILL.md links and the Item crumb back. Otherwise a
    reader on Version 3 who opens a file would land on the draft and read
    different bytes.
  - A shared link without `version` shows that reader's default version
    (the draft for whoever may read it, the public version for anyone else), as
    GitHub's default branch does.
  - `setParam` already drops `file` when the tab or version changes. Update
    its comment, which still says an open file belongs to the Files tab.
  - A `file` not in the shown version gets a standing notice with a link
    back to the item.
- **Rows and links**:
  - `SkillPage` rows become `<Link>`s to `?file=…`, so they can be
    middle-clicked and shared.
  - `Markdown` gains an optional `fileHref(href) => string | null`, the
    website's twin of the desktop's `resolveLink`. `urlTransform` keeps an
    href it resolves, and `a` renders a router `Link`.
- **Trail**: `Repositories / owner / name / Item` becomes `… / Item` (now a
  link) `/ <base name>`.
  - `Crumb` gains `menu?: { header: string; items: Array<{ label; detail; to }> }`.
    Each `to` comes from the address builder, so it keeps `version`.
  - The menu reuses `.ver-menu`. Under 640px it hangs from the trail's left
    edge at full width.
  - `VersionSwitcher`'s click-outside, Esc and focus-return effect moves into
    a small `usePopover` hook that both use.
- **File view**: the gallery `FileViewer`'s body becomes one shared
  component.
  - It holds the bar with facts and tools, `CodeLines`, `file-doc` Markdown,
    `file-none`, `ShaChip` and `download`.
  - The gallery's dialog and the skill's file page both render it.
  - Text is decided by the bytes, so the Files tab's viewer changes too: an
    extensionless text file now previews there. Gallery cards still fetch
    only by extension, so a thumbnail never loads a binary to learn it is one.
- **The project Files tab stays a dialog** (its Quick Look), a deliberate
  difference. A gallery of pictures and documents steps through files; a
  skill is read one file at a time, as a page.
- **Tests**: the website's tests are node-only (`web/test/*.test.ts`, run by
  `npm run test:web`), with no DOM harness, and this plan does not add one.
  The pure parts go in one web module, `web/src/skillFiles.ts`, tested from
  `web/test/skill_files.test.ts`:
  - The address builder: it sets `file`, keeps `version`, and drops `file`
    on the way back.
  - The relative-link resolver.
  - Text by bytes, where a NUL byte or invalid UTF-8 means not text.
  - The menu's filter.

  The rendered trail, menu and missing-`file` notice are checked on staging.

## Order

1. **Desktop backend.** Measure `inventory_local` first, then add the command
   and its Rust tests.
2. **Desktop frontend**, then the live Tauri pass on a real Codex and Claude
   skill. Before editing `StorageCatalogPage.tsx` (the crumbs and the
   `itemPage` block), message the status-filter session, which holds that
   file's list section uncommitted.
3. **Cloud**: shared file view, address, Trail menu, links and tests, then
   staging.

## Out of scope

- **`__pycache__/*.pyc` rows**: they are real captured files. The skill
  capture is the exact tree, so filtering them would change digests and
  identity.
- **Later layers**: syntax colour (a highlighter dependency on both sides),
  and a banner for hidden or bidirectional Unicode (as Gitea and GitHub show).
- **Moving between files**: no ←/→ stepping on either product; the crumb
  menu covers it.

## References

These are the sources read for the direction (paraphrased):
- **The file as a page, named by the last crumb**:
  [GitHub blob](https://github.com/anthropics/skills/blob/main/skills/pdf/SKILL.md),
  [Hugging Face](https://huggingface.co/openai-community/gpt2/blob/main/config.json).
- **The last crumb is the current page**:
  [NN/g breadcrumbs](https://www.nngroup.com/articles/breadcrumbs/),
  [HIG path controls](https://developer.apple.com/design/human-interface-guidelines/path-controls).
- **Folding from a split into this page when narrow**: Android list-detail.
- **Preview / Source and binary placeholders**: GitHub, GitLab, Gitea and VS
  Code.

Material, npm's Code tab, Sourcegraph and Apple's QLPreviewPanel docs could
not be read and are not cited.
