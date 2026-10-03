# Plan: Add Catalog resource, the way New project is

Proposal, 2026-10-02. Specimen: `mockups/catalog-add-home-mockup.html` (`npm run dev`,
then `/mockups/catalog-add-home-mockup.html`; `?stock=some|none&profiles=one|two&
storages=none|one|many&catalog=empty|some&door=any|skill|pet-v1&
resource=cards|rows|pills&theme=dark`). Earlier specimen, kept for the
record: `mockups/catalog-add-step-mockup.html` (kind first against home first).

**Status (2026-10-02): implemented**, steps 1–5, with the four items under
*To confirm* taken as written (owner, 2026-10-02: "Sounds good, go and
implement"). The shipped parts: `orderStorages` in `project-sync/model.ts`
and the draft's seeded storage in `project_sync/service.rs` (step 1);
`probe_catalog_resource_profile` and `inspect_catalog_resource_profile` in
`storage_catalog/service.rs` (step 2); `CatalogHomeStep.tsx`,
`CatalogScopeSetupStep.tsx` and `setupFurniture.tsx` under
`AddCatalogScopeWorkspace.tsx`, with `profilePathHint` moved to
`src/profilePath.ts` for both setup pages (steps 3–4); the README paragraph
(step 5). The specimen stays as the record of the decisions.

## Why

Add Catalog resource asks two questions and answers neither. Kind is four
cards with none chosen, Agent profile two pills with none chosen, each under
a warning triangle; Add makes one row and lands on it with "No storage" in
its crumb. The backend already knows most of the answers: it registers
"Default Codex" and "Default Claude" the first time profiles are listed, the
same way project setup does, and every machine with `~/.codex` and
`~/.claude` wants the same handful of rows. A Catalog that covers both homes
is six passes through a page that remembers nothing.

New project reads before it asks. The setup page switches on every agent the
registry marks a setup default, chooses its profile exactly when this
machine offers one unambiguous profile, leads Storage with the safe answer,
and says on the button what it will do. The rules this plan leans on are
the usual ones: pre-fill the most common value and never the first in the
list (NN/g on defaults); a wizard is for infrequent setup, its button names
the outcome, and last time's answers are next time's defaults (NN/g on
wizards); the common path first, the uncommon one behind Custom path…
(progressive disclosure).

## Decided (owner, 2026-10-02)

1. **Two steps, New project's shape.** Step one picks the agent home; step
   two sets it up. Home first, not kind first.
2. **One row per pass.** Every list is a radio. Finish makes exactly one
   Catalog row — one kind on one home — as Add does today. A second home or
   a second kind is a second pass.
3. **Step one is the project's Agent band**, lifted whole: one card per
   provider with its mark, name and profile label on the toggle line; the
   chosen card opens the project's profile row — saved profiles and Custom
   path… in one select, the inline path, Browse, the hint naming what the
   path resolved to. The toggle is a radio rather than a checkbox; the card
   not in use is only its name. The project's preselection rule applies
   verbatim: a provider's profile is preselected exactly when this machine
   offers one profile for it; with none, or several, the select reads
   "Choose profile" and the band says "Choose a profile for Codex."
4. **Step two is the kind as the shipped Kind cards**, with what the home
   holds on the foot line ("12 skills · 2 hours ago", or "Nothing here
   yet"), then Storage, then Finish. Rows and pills were drawn and not
   picked.
5. **Storage order is Mallard Cloud, then the other clouds (R2, Dropbox,
   Feishu), then local folders — on project setup and Catalog setup alike.**
   The backend already ranks the providers this way
   (`StorageProviderDescriptor.display_rank`: 5, 10, 20, 25, 30); the setup
   pages never used it. The default follows the same order when nothing
   was used before.
6. **Little copy.** No "holds…" sentences, no explanation under an inactive
   card. One short line under Storage only when it states why the default
   is the default ("Your only storage.", "Used last by the Catalog.",
   "Mallard Cloud first.").

## What the user sees

### 1. Step one — Add Catalog resource

The page keeps its title, its close button, and New project's two doors.
From Mallard Cloud is unchanged (the account, the repository, then the row
it lands in). On this computer is one band, **Agent**:

- One `sync-provider-card` per registered provider, in the registry's
  order. The card that starts chosen is the provider whose profile was
  active most recently; the first provider when this machine has none.
- The chosen card holds the profile row: the select lists the provider's
  saved profiles and Custom path…, with the one unambiguous profile
  preselected; Browse opens the folder dialog; the hint under the row reads
  "Codex profile · /Users/you/.codex" for a saved home, and for a typed path
  the project's hints verbatim — the two typing states, "Already set up as
  'Work Codex' — setup will reuse it.", "This folder can't be read.", "No
  folder at this path yet.", "Codex profile · <canonical path>".
- Nothing chosen: the attention badge on the band and the project's field
  message, "Choose a profile for Codex."
- The button names the step it opens: **Set up Default Codex**. For a typed
  path it saves the profile first — named by its folder, as
  `save_catalog_resource_profile` already does; re-picking a saved home
  returns that profile — and reads **Set up .codex**. A saved profile with
  no row is harmless and is listed next time, exactly as the project's
  profiles outlive its drafts.

### 2. Step two — Set up Default Codex

Three bands and the footer, the project's setup page in the Catalog's
nouns:

- **Home**: Name, editable (a change is `rename_catalog_resource_profile`
  on Finish), and Folder, read-only. Changing the home is going back, not
  browsing again; the specimen's Browse… on this field is dropped.
- **Resource**: the shipped `setup-choice-card` grid, one card per
  capability the provider registers, in the registry's order. The foot line
  carries the count and the newest stamp, or "Nothing here yet"; the roots
  the count came from are the card's tooltip. A kind this home already has
  a row for is greyed with an **Added** chip. Preselected: the kind a Kind
  door named, else the first card with something in it and no row; nothing
  found and no door: no guess, the badge and "Choose a kind." Counts arrive
  after the page opens; until they do the foot line is the format label.
- **Storage**: New project's pill row. "None yet" leads, as it ships; the
  storages follow in preference order. Preselected: the storage a Catalog
  row used last (the newest link across rows), else the first by
  preference, else None yet. Under the row, the one short line above, or
  nothing.
- **Footer**: the row Finish makes, named — "Skills · Default Codex" — then
  Cancel and **Finish** (**Finish locally** with None yet).

Finish creates the row (`create_catalog_scope`), links the chosen storage
(`link_catalog_scope_storage`; linking is configuration only, nothing is
pushed or pulled), and lands on the row's page with that storage in its
crumb and the shell notice "Skills · Default Codex added to the Catalog."

### 3. Doors

The sidebar's and landing's Kind doors keep opening Add; home first means
step one still shows, and the door's kind is preselected in step two. The
Catalog row menu's "Add Catalog resource…" and the rail's + open step one
with nothing named.

### 4. When something fails

- The profile save on Set up fails: step one stays, the error under the
  band, the button live.
- `create_catalog_scope` refuses ("this Catalog row already exists"): step
  two stays, the Resource card greys to Added, the error names it.
- The link after creation fails: the row exists; the page lands on it with
  the error, and its storage menu is the retry. Finish never undoes a
  creation.
- The counts read fails: the cards keep the format label on the foot line;
  the band's error says counts are unavailable; choosing still works.

### 5. What goes away

Today's single-page Kind cards over the Agent profile band in Add, the
provider pills before the select, the two attention triangles on an empty
page, and the "On this computer picks no storage" rule: Storage moves into
Add, the way it sits on the project's setup page.

## Backend (`src-tauri/src/storage_catalog/`)

Two reads, no writes beyond the commands that exist. Catalog-owned, Tauri
commands in `resource_commands.rs`, logic in `service.rs` beside
`list_catalog_resource_profiles_with_repository`; no `project_sync` import.

- `probe_catalog_resource_profile(provider, path) -> CatalogProfileProbeDto
  { canonical_path, readable, existing_profile_id, suggested_name }`. The
  project's probe shape for the Catalog's hints. Validation is
  `validate_new_profile_home` (absolute, clean, canonical, no symlinked
  ancestor, outside the Mallard root); `readable` is one metadata read;
  `existing_profile_id` matches the canonical home against saved profiles;
  `suggested_name` is `derived_profile_name`. Never scans a project folder.
- `inspect_catalog_resource_profile(profile_id) ->
  CatalogProfileInspectionDto { profile, last_used_storage_id, holdings:
  [{ coordinate + labels, count, newest_modified_at, roots: [summary],
  scope_id }] }`. One entry per capability the profile's provider
  registers. The count is `inventory_local` for that capability over the
  resolved context — the same read the row page does on open, bounded as
  it is today, run per capability — reduced to a count, the newest stamp,
  and the target summaries; no payload, path list, or digest crosses IPC.
  `scope_id` joins the row that already exists. `last_used_storage_id` is
  the storage of the newest `CatalogScopeLink` by `updated_at`, or null.
  If the read proves slow on large homes, a count-only classification
  (readdir plus the codec's capture of one entry at a time, no files kept)
  is the optimization; not before.

No schema change. Profiles, scopes, and links keep schema 1 and their
fields; portable manifests are untouched. Storage ordering needs no
backend change.

## Frontend

- `src/components/storage-catalog/AddCatalogScopeWorkspace.tsx` becomes the
  host of two stages, keyed as New project keys its step and its draft
  page: `CatalogHomeStep.tsx` (step one) and `CatalogScopeSetupStep.tsx`
  (step two), both in `storage-catalog/`.
- The profile row is a Catalog-owned `CatalogProfileRow.tsx` in the
  shipped classes (`sync-provider-card`, `sync-provider-toggle
  setup-marked-toggle`, `sync-agent-profile-row`, `sync-select-shell`,
  `sync-setup-path-hint`); the Catalog may not import project-sync
  components, and the integration suite checks that. The pure hint
  builder, `profilePathHint`, moves from `ProjectSetupWorkspace.tsx` to
  `src/profilePath.ts` beside `expandHomePath`, typed structurally (a probe
  with `canonical_path`, `readable`, `existing_profile_id`; a descriptor
  with `display_name`), and both pages import it from there. The debounced
  probe hook stays per page, each calling its own API.
- `storage-catalog/types.ts` gains `CatalogProfileProbe` and
  `CatalogProfileInspection`; `storage-catalog/api.ts` gains `probeProfile`
  and `inspectProfile`.
- Storage order is one helper, `orderStorages(storages, providers)` in
  `src/components/project-sync/model.ts`: provider rank from the served
  `StorageProviderDescriptor.display_rank`, then name. `ProjectSync.tsx`
  applies it once to `config.storages` before the list reaches
  `ProjectSetupWorkspace`, `CloudProjectSource`, `NewProjectFolderStep`,
  and `catalogStorages`; the pages render the order they are given.
- Project setup's default: a draft with no storage and at least one storage
  on this machine opens with the first by preference chosen, saved by the
  autosave like any edit; Finish then reads "Finish", and None yet is one
  click away. "None yet" still leads the row and still carries no badge.
- The Catalog page's `initialStorageId` already lets Finish land on the row
  with its storage; `completeAddCatalogScope` passes it.
- `catalogDoors`/`catalogAddChoices` stay for the sidebar and landing; the
  door's choice key becomes step two's preselected kind.

## Steps

Each step leaves a working product; each lands with its tests.

1. **Storage order and default, both pages.** `orderStorages`, the
   `ProjectSync.tsx` wiring, the project's default, the sentence under the
   row. Frontend only; ships alone.
2. **The two reads.** `probe_catalog_resource_profile`,
   `inspect_catalog_resource_profile` with `last_used_storage_id`, their
   DTOs in Rust and `types.ts`, registration in `lib.rs`, Rust tests.
3. **The two steps, without counts.** Step one as the Agent band on the
   existing profile list and `saveProfile`; step two with the cards
   wearing the format label, Storage, Finish creating and linking one row.
   Today's page is removed in the same change.
4. **Counts and last-used.** Step two calls `inspectProfile`; the cards
   get their counts, stamps, Added chips and tooltips; the storage default
   gains the last-used rule.
5. **README** paragraph on the Catalog menu and Add, and the memory of
   `mockups/catalog-add-home-mockup.html` in this plan's status line.

## Tests

- `tests/frontend/storage-catalog.integration.test.tsx`: the three pins of
  today's page are rewritten, not worked around — "the Add chooser lists
  the backend's capabilities…" (the choice lives on step two's cards now,
  still from `catalogDoors` order), "Add has New project's two doors: Kind
  then agent profile…" (step one's bands are the Agent band; the assertion
  that Add picks no storage inverts), and "the profile is named by its
  folder…" (the save moves to Set up). New pins: the preselection rule
  (one profile selected, two leave "Choose profile" and the field message),
  one `createScope` and at most one `linkScopeStorage` per Finish and none
  for Finish locally, the kind door's preselection, the Added card, the
  storage order and default, and that nothing on either step imports
  project-sync (the existing independence test keeps that honest). The
  command-surface test lists the two new commands.
- `tests/frontend/project-sync-cutover.integration.test.tsx`: "the empty
  storage slot is named, not marked as a fault" stays as is; add the pill
  order and the opening default.
- Rust, beside the code: the probe rejects a symlinked ancestor and the
  Mallard root and finds an existing profile by canonical path; the
  inspection counts per capability under the bounds, tells pet v1 from v2
  on one root, joins `scope_id`, and names the newest link's storage.

## Safety and schema

- No schema change; machine-local JSON documents are unchanged in shape.
- Both reads keep canonical-path validation, no-follow traversal, bounded
  walks (`inventory_local`'s limits, `MAX_MODIFIED_WALK_ENTRIES`), and
  never look inside a project folder. Only counts, stamps, and target
  summaries cross IPC.
- Finish links; it never pulls or pushes. Nothing on disk or in storage
  moves until a review is approved on the row's page.
- The profile saved on step one is not a row and not a link; it is the same
  record Add saves today, one step earlier.

## To confirm

1. **The project's default storage.** Decided for the Catalog; this plan
   applies it to project setup too, as said. Confirm that a new project with
   a Mallard Cloud account opens with it chosen and "Finish" rather than
   "Finish locally".
2. **Folder in step two is read-only.** The specimen still shows Browse…
   there; the plan drops it (going back is the way to change the home).
3. **Counts by the full inventory.** Reuses what the row page runs; a
   count-only read is the fallback if it proves slow.
4. **Landing after Finish: the row's page** with its storage, as today —
   not the Catalog index.
