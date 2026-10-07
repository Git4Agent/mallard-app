# A Catalog Item's Page, Like Mallard Cloud's

Status: proposal, October 6, 2026. Specimen:
`mockups/catalog-item-page-mockup.html` (served at
`http://localhost:1420/mockups/catalog-item-page-mockup.html`). Nothing in
`src/` or `src-tauri/` has changed.

## Problem

On Mallard Cloud's website a skill, plugin or pet has a real page: a head with
its tile and name, a body written for the kind (the skill's SKILL.md and files;
the plugin's store hero and install box; the pet playing on a desk), and a rail
of facts (`RepositoryPage.tsx`, `CatalogItem.tsx`, `PluginPage.tsx`,
`PetPage.tsx` in mallardInternal's `web/src`).

In the app, an item has a page only on Mallard Cloud
(`StorageCatalogPage.tsx`, `onOpen` is `managedCloud && resourceId`), and the
page is the item's list card, plus a "Repository · New · Change…" card before
its first Push (`itemPage`, ~line 2354). It says nothing the list does not:
no description, no SKILL.md, no files, no plugin words, no pet beyond its
96 × 104 idle tile. HEAD and the working tree agree here; this is not a
regression.

## Recommendation

**A · The website's page**, folding into **D · a strip of facts** under 880px
of page; **S1** for skills, the website's **store hero** for plugins (the
owner's Cloud pick), **T2** (the website's desk) for pets in layer 2. Every
storage gets the page.

- **Head** (in the page, under the bar): the kind tile (a pet's idle frame),
  the name (a plugin's listing name), and three chips: status, kind,
  provider. On the right, **Open in Claude/Codex** as the one named button,
  then Terminal, Fork (directory kinds) and Reveal in Finder as the card's own
  icon buttons. Push, Pull and the Cloud chip stay on the bar; nothing is added
  to the bar (its actions are at budget at 580px).
- **Body panel**: a head that says which copy the body shows — the app's
  answer to the website's Draft ▾ — then the kind's body.
- **Rail** (two columns from 880px of page, 280px wide): About (kind and
  provider chips, files, size; a plugin's installed version; a pet's format),
  This Mac (folder, edited, version, Reveal), the storage's card (on Mallard
  Cloud: repository, visibility, draft against versions, pushed, Open on
  website; before the first Push: the address it creates, New, Change…), and
  Identity (ID, coordinate, digest) behind a disclosure.
- **Under 880px** (every window at the default 1100px): the rail's facts
  become one run of chips under the head — files, size, edited, repository,
  version — and Identity sits under the body. No fact is dropped.
- **The description is said once, in the body**, which every width shows.

### What maps to what

| On Mallard Cloud | In the app | Why |
| --- | --- | --- |
| Trail: Repositories › owner › name › Item | The path bar: storage / Kind · Profile / item | The bar is already the trail |
| Kind tile, owner / name, visibility | Kind tile, name, status · kind · provider chips | Owner and visibility belong to the repository card |
| Star, Invite, Request public release | Not on the page; the Cloud chip's panel opens the website | Web verbs about the repository |
| Pull ▾ | Push and Pull on the bar, as today | Bar actions never shrink |
| Tabs: Item · Versions · Release · Members · Settings | No tab bar | The rest are the website's; one facet needs no tabs |
| Draft ▾ / Version N, Publish | This Mac \| storage switch; Publish stays in the Cloud panel | The app's question is which copy |
| Skill: description, SKILL.md, Files | The same | Same reader |
| Plugin: store hero, install box, facts | The same, plus "Installed in Claude on this Mac · 1.4.0" or "Pull installs it" | On this Mac, Pull is the install |
| Pet: desk with moments | The same desk (layer 2); the idle frame (layer 1) | The desk needs the atlas |
| About · People · Draft rail | About · This Mac · storage · Identity | People is the website's |

### Which copy

| Row state | Panel head |
| --- | --- |
| In sync | "This Mac = Mallard Cloud", no switch |
| Changed here / in storage / on both | Switch: This Mac (default) \| Mallard Cloud · draft |
| Only in the storage | "Mallard Cloud · draft", the stored copy |
| Not pushed | "This Mac" |

The sentence beside it is `rowStatusLabel`, as the row's badge says it.

## Directions considered

Six page frames are in the specimen (part *The page*), each at 580, 778 and
1118px (the floor, the 1100px default window with its 298px sidebar, a 1440px
window).

| | Direction | Seen in | Against |
| --- | --- | --- | --- |
| **A** | Head, body panel, facts rail; folds to D | Mallard Cloud; VS Code extension editor (Installation rail); Raycast Store (side column, no tabs); GitHub About; Primer PageLayout | VS Code hides its rail under 500px instead of moving it |
| B | Store hero, one column, Information at the end | Mac App Store; VS Code (actions under the description) | Primer and VS Code keep metadata in view; a long SKILL.md pushes it a screen away |
| C | Tabs: Overview · Files · Details | JetBrains (Additional Info tab); VS Code tabs; Obsidian directory | NN/g: most-needed content in the default view; a plugin or pet gets two tabs for one screen |
| D | Strip of facts, body full width | Mallard's session page chips; VS Code's header subtitle row | Loses the website's shape at wide windows |
| E | Inspector beside the list | JetBrains Settings › Plugins; Claude Code /plugin pane; HIG split views | ~460px for SKILL.md at 778px; no page to land on from a link |
| F | Sheet over the list | The app's own Add dialog | No product read first-hand does this; covers Push, Pull, the Cloud chip |

Bodies (part *The body*): skill S1 (the website's) / S2 (frontmatter as
fields) / S3 (file browser); plugin installed / not on this Mac / no listing;
pet T1 (idle frame, no new read) / T2 (the website's desk) / T3 (every moment
as a grid).

## Decisions for the owner

1. **Which storages give an item a page.** Every storage (recommended): the
   content is on this Mac whatever the storage. Or Mallard Cloud only (today's
   rule).
2. **How far the pet goes.** Layer 1: T1, no new read. Layer 2
   (recommended next): the atlas reaches the page; T2.
3. **Which copy.** This Mac first, the storage's copy on the switch
   (recommended; read on demand). Or this Mac only.
4. **The frame.** A (recommended), or one of B–F.

## Implementation sequence

Layered: each step ships a working page.

### Layer 1 — the page, this Mac's copy

1. **A neutral Markdown module.** `SessionDetailMarkdown` lives in
   `project-sync/session-detail/`, which storage-catalog may not import (the
   boundary test, `storage-catalog.integration.test.tsx` ~1253). Move its
   core (react-markdown + remark-gfm + math + Mermaid fences, no raw HTML) to
   `src/components/markdown/` and its `.session-detail-event-copy` rules to a
   neutral class both pages use. The session page keeps its project-image
   hooks as props. Clean cutover; no second renderer.
2. **One backend read: `get_catalog_resource_detail(scopeId, storageId?,
   resourceId, copy)`.** A codec method beside `preview`:
   `fn detail(&self, envelope, read_payload) -> Result<CatalogResourceDetail, String>`.
   - Skill (`agent_skill.rs`): SKILL.md's text, bounded by
     `MAX_SKILL_MD_BYTES` (256 KiB), and the file list (path, size, mode)
     from the envelope. The frontend splits the frontmatter, as the website.
   - Plugin (`plugin_intent.rs`): the intent's name, marketplace, source; and,
     for this Mac's copy, the installed listing and version. Reuse
     `plugin_listing.rs` (`listing_from_manifest`, `installed_listing`,
     `pub(super)` today; widen to `pub(crate)`) with a domain-level DTO rather
     than `managed_cloud::client::PluginListing`, since the page shows it
     without Mallard Cloud. **`plugin_listing.rs` is another session's
     uncommitted work (the Cloud plugin page's listing upload, October 6); it
     is not on `main` yet.** This step waits for it to land.
   - Pet (`codex_pet.rs`): layer 1 returns nothing new.
   Local copy: the capture the inventory already makes (`read_directory_tree`,
   no-follow, bounded). Register in `lib.rs`; DTOs in Rust and `types.ts`
   together; the call in `storageCatalogApi`.
3. **`CatalogItemPage.tsx`** in `storage-catalog/`: the head, the copy head
   (one label in layer 1), the fold (a container query at 880px), the rail
   cards, and the bodies (`SkillBody`, `PluginBody`, `PetBody`). Files, size,
   folder, times, warnings and the Cloud facts come from `CatalogScopeRow`,
   the Cloud status and the page, as today. Everything kind-specific comes
   from step 2's read: SKILL.md and the file list, and a plugin's listing and
   installed version (which About, the strip and the "Installed · 1.4.0" line
   show).
   `PluginBody`'s install lines port the website's `installWays`
   (`web/src/plugin.ts`) with its rule that shell-unsafe intent parts are
   never printed.
4. **`StorageCatalogPage.tsx`**: `itemPage` becomes `<CatalogItemPage>`; the
   Repository card moves into the rail's Mallard Cloud card. For decision 1,
   `itemRow` and `onOpen` drop their `managedCloud &&` gate; the Cloud status
   hook, the Cloud chip and the repository card keep theirs.
   The item's Push and Pull need no change on any storage: `openReview`
   prepares the scope's plan and ticks only this item (`remaining`), and the
   review lists only its row — the list's own Push with every other row
   unticked. On a shared repository (Vault, S3/R2, Dropbox) that Push
   publishes one new immutable snapshot of the scope in which only this item
   changed, as such a list Push does today.

### Layer 2 — the stored copy and the pet

5. **The stored copy.** `copy: "stored"` reads through the engine's verified
   `inspect` + `fetch_payload`, as `get_catalog_resource_preview` does; never
   in the list path; fetched only when the switch is pressed. The switch turns
   on for changed rows.
6. **The pet's desk.** The codec's `detail` returns the atlas as a data URI
   after the same digest, size (`MAX_PET_ATLAS_BYTES`) and pinned-dimension
   checks `pet_preview.rs` applies. Port the website's player
   (`web/src/pet.ts` geometry, `PetPage.tsx` desk) into
   `storage-catalog/petPlayer.ts` and `PetBody`; Pause, and paused under
   Reduce Motion.

## Safety and schema impact

- No schema or manifest change. Schema 1 throughout. Nothing new leaves the
  machine; the plugin listing read for the page is not sent anywhere.
- Every read is bounded and no-follow (the existing capture and
  provider-state readers). The stored copy is digest-verified before use.
- Markdown renders without raw HTML, through the shared `safeMarkdownUrl`
  transform. Links open through the opener, never in the web view.
- **New rule for the item page: images in SKILL.md are not fetched.** The
  session page loads safe https images directly; a skill can come from
  someone else through Mallard Cloud, and an embedded image would tell its
  host who opened the page. The neutral module takes the image policy as a
  prop; the item page passes the placeholder (`MarkdownImagePlaceholder`).
- Paths shown on the page are this Mac's own and stay in the frontend; they
  are never written into a manifest.
- Secret detection is unchanged; a row's warnings show in the rail.

## Tests

- Rust, beside each codec: a skill's detail returns SKILL.md and the file
  list; an oversized SKILL.md and a symlinked file fail closed; a plugin's
  detail carries the installed listing only for the local copy; a pet atlas
  with the wrong dimensions returns no atlas. A service test reads a stored
  copy through the verified fetch.
- Frontend (`storage-catalog.integration.test.tsx`): the page renders the
  head, chips and rail; the copy head per row state; the description once;
  the fold by computed style at 879 and 880px of page; the boundary test
  still passes with the neutral Markdown module.
- `npm run build`, `npm run test:frontend-integration`,
  `cargo test --manifest-path src-tauri/Cargo.toml --lib`.

## References

Read October 6, 2026; paraphrased. Primer PageLayout and PageHeader; Apple
HIG Toolbars, Tab views, Split views, Panels, Motion, Disclosure controls;
ARIA APG Tabs; NN/g Tabs used right, Progressive disclosure, Accordions on
desktop, Breadcrumbs; Carbon Tabs, Code snippet, Structured list; WCAG 2.2.2
and 2.3.3; VS Code's `extensionEditor.ts` and its CSS; JetBrains plugin help;
Claude Code plugin and skill docs; GitHub's repository page; the Mac App
Store; Raycast Store. Could not be read: Apple Inspectors (404), Primer
clipboard pages, Material 3 (read through its Android Tabs doc), npm, Docker,
Steam, LINE, Obsidian's in-app plugin view. None covers hiding a one-tab bar.
The specimen's References part links each one.
