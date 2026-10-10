# Plan: pets on Mallard Cloud, the way projects are

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Proposal, 2026-10-01. Specimen: `mockups/cloud-pet-mockup.html` (`npm run dev`, then
`/mockups/cloud-pet-mockup.html`; `?scene=` jumps to a step, `?theme=dark`).

**Status (2026-10-01): implemented**, steps 1–3. Scene 0 ("Live") runs the
built page on a stand-in backend. On Mallard Cloud, outcomes, reviews and
Preparing sit on the Storage card in the project's words. Not yet built:
blocked items in a Pull review (backend 10), the kept copy's name in Pull
results (11; the page names it from the plan), holding one moved item at Push
apply instead of stopping the batch (9), and a blocked row for one refused
repository (only "several" and a refused account are per item or page-level
today).

- **Scene 0 is today:** the shipped Catalog row page, live on a stand-in
  backend.
- **The other scenes are drawn only from shipped parts:** the Catalog card,
  the session title button, the project page's Storage card, review
  readouts, the Cloud chip and panel, and the project's connect dialog. The
  proposal adds no style of its own.

The decisions at the end are open until the owner picks on the specimen.

Pets are the example throughout. Nothing here is pet-specific: the Catalog's
Mallard Cloud code is coordinate-neutral, so skills and plugin intents on a
Mallard Cloud storage get the same flow.

**Scope: only Mallard Cloud changes** (owner, 2026-10-01). A Catalog row on
a local folder, S3/R2, Dropbox or Feishu storage keeps today's page, words
and flow:

- one Library repository per row, one version
- plain row names
- batch Pull and Push, with "Version N" in the outcome
- no pet page, no Cloud chip, no destinations

The specimen's storage crumb switches to such a storage (scene 12, "Other
storages"). Everything below applies only when the row's storage is Mallard
Cloud, where each pet is its own repository.

## Why

Mallard Cloud models a pet exactly as it models a project: one repository
with an `owner/name` address, a page, a visibility, a draft, a release
(`PLAN_MANAGED_CLOUD_CAS_FREE.md` decision 11). The desktop does not. A
project reaches its repository through one flow; a pet through another.

| | Project | Pet, today |
|---|---|---|
| Choosing the repository | The "Connect {project}" dialog: the address or invite-code field, a found row with "Use this", a "Repositories" list with version, size and age, and "New repository {login}/{name}" first, with the address it will get | The inline "Cloud repositories" panel: the same field, then bare radio rows ("private · owner"). No New row. The first Push creates `{login}/{display name}`, a name the user never saw |
| Where the Cloud status lives | One Cloud chip in the path bar, beside Pull and Push | A chip on every connected row (one status request per row), and "Connect Cloud repository" on every other row |
| A repository that is not ready | A notice under the bar (Repository gone · Sign-in required · Repository unavailable); Pull and Push close except for gone and deleting (`cloudSyncBlocked`) | Only inside each row's chip panel. Push and Pull never close |
| After Connect | "Storage linked." Nothing opens on its own | The Pull review opens ("Connect and review") |
| Leaving a repository | Not offered (Unlink, then Link) | "Change repository…" and "Disconnect repository". Disconnect has no confirmation, and for the account's own repository it does nothing lasting: the next refresh learns the same repository again (`observe_managed_items`, `repository_for`) |
| Two repositories hold the item | Cannot happen (one per bundle per account) | The whole row page fails: "Several Cloud repositories contain this item. Connect its exact repository address." |
| Someone else's repository | New project › From Mallard Cloud | The same "Cloud repositories" panel, unfiltered |

## The rule

**On Mallard Cloud, a pet is to its repository what a project is to its
repository.** On every other storage the Catalog row is the repository, and
the row page already is that workspace.

1. A pet gets the project's workspace shape. Its page has the path bar
   `Storage / Catalog row / Pet`, with the Cloud chip, refresh, Pull and Push
   acting on that pet.
2. The repository is chosen in the project's dialog, from the same picker,
   with the same words.
3. The Catalog row page stays the list, as a project's sessions list is. It
   keeps its row badges and batch Push and Pull, and carries no Cloud chips.

## What the user sees

### 1. The Catalog row page on Mallard Cloud

- **Bar:** `[Mallard Cloud] / [Pets v2 · Codex home]`, then attention,
  **+** (Add from Mallard Cloud, §6: a `sync-path-icon-action` like refresh,
  where today's "Cloud repositories" button sits), refresh, `Pull n | Push
  n`.
- **Sections:** items only in Mallard Cloud come first under
  "Not downloaded", with the storage's mark for its icon and the count at the
  heading's end, as a project's "Only in" heading counts. Another storage
  keeps "Only in {storage}".
- **Rows:**
  - Each row is today's card without the Cloud slot.
  - The name opens the pet page. It is Activity's session title button
    (`sync-history-session-title-button`, tooltip "{name} · View pet
    details"), as a session's title opens its page. On another storage the
    name stays plain text, as today.
  - The badge sentences stay as they are, e.g. "Only on this computer. Push
    to save it in Mallard Cloud."
- **The Catalog row menu** keeps "Add Catalog resource…", which opens §6's
  doors.
- **A refused sign-in:** when the page's one Mallard Cloud request is refused
  for sign-in, the page shows the project's note, not "Couldn't load":
  "Warning · Sign-in required — {message}" with "Storage settings". Pull and
  Push close. The rows still list what is on this Mac.

### 2. The pet page

Opened from a row's name. The row crumb becomes a way back
(`onActivate`) and keeps its menu.

- **Bar:**
  `[Mallard Cloud] / [Pets v2 · Codex home] / [Duck]`, then the Cloud chip,
  refresh, and `Pull | Push`, which act on Duck alone. This is the project
  bar, and the chip is the same `ManagedCloudChip` in the place it was
  designed for.
- **Under the bar:** the project's `cloudStatusNotice`, word for word:
  - "Repository gone — Deleted, or this account lost access. Push creates it
    again." (no control)
  - "Sign-in required" with "Storage settings"
  - "Repository unavailable" or "Status unavailable" with "Retry"

  `cloudSyncBlocked` closes Pull and Push exactly as it does for a project.
- **Leaf crumb:** the pet's name, with its preview at 16px as the crumb's
  `leading` mark, where a session leaf wears its provider's mark.
- **Body,** made of parts that already ship:
  1. Only while the pet has no repository: a **Repository** section above
     its own, drawn like it — one row: "Rika321/Duck" (the address the first
     Push creates, `predictRepositoryAddress`), the "New" and "Private"
     pills, and **Change…**, which opens §3's dialog. Do nothing and Push
     creates it.
     - Once there is a repository, the section is not drawn. The repository
       lives in the bar's Cloud chip, as a linked project's storage lives in
       its crumb, and it is fixed (§5).
  2. The pet's own card, exactly as the list draws it, under the same
     section heading ("Pets v2", or "Not downloaded"). The local
     path and digest stay in its ⓘ menu, and its launch row stays on its
     first line.
- **No repository yet:** there is no chip, as for a project that has none.
- **Status is read once,** when the page opens and after each action, with
  `get_catalog_cloud_status`. The list makes none of these requests.

### 3. The connect dialog

This is `BundleConnectionDialog`'s Mallard Cloud body with a pet in it.

- **Title, by reason:**

  | Reason | Title and sentence |
  |---|---|
  | Choose | "Connect Duck" |
  | Several | "Choose a repository" + "Duck is in 2 repositories you can see. Choose the one this Mac uses." |
  | Add | "Add from Mallard Cloud" (§6) |

Fewer words than the project's dialog: no context row (the bar's storage
crumb names the account), no list heading, one verb.

- **Field:** the shared `RepositoryLookupField`. Its label is for screen
  readers only; the placeholder says it. Look up / Join, whose busy labels
  are the only progress shown.
- **Answer:** one line — "maya/Duck", then "Public · Contributor · v6" (or
  "Member", "Owner") — and "Use this" / "✓ Selected". No chips, no website
  link.
- **Field errors,** one short sentence each:

  | Case | Sentence |
  |---|---|
  | another service | "That's staging.mallard-ai.com, not this account." |
  | GitHub | "That's a GitHub address." |
  | not an address | "Type owner/name, or paste an invite code." |
  | a project or another kind | "Rika321/portal is a project, not a pet." |
  | another pet | "Rika321/Owl holds Owl, not Duck." |
  | another format | "Rika321/Duck-classic is a Pets v1 repository." |
  | a failed code | "That code expired or was used. Ask for a new one." |
  | nothing visible | "Nothing at maya/Duk." + "Check the spelling, or paste an invite code." + "Use a new repository" |

- **List,** no heading:
  - **First:** "Rika321/Duck" with a "New" pill, and "Private". It uses
    `predictRepositoryAddress`; "Duck is taken." when it takes a `-2`. It is
    offered when the account owns no repository of Duck.
  - **Then the repositories that hold Duck** that this account belongs to,
    e.g. a collaborator's: "kai/Duck · Member", "v4 · 3 h ago" (or "not
    pushed yet"). Filtered by the item, so another pet is never a choice.
- **Footer:** "Can't be changed later." · "Cancel" / "Connect"
  ("Connecting…"), for a new repository too.
- **States** (specimen scene 5): Loading repositories…; "Couldn't reach
  Mallard Cloud." when the list can't load; Joining…; Connecting…;
  "Mallard Cloud didn't answer. Try again." when Connect fails.
- **After Connect:** the shell note reads "Duck connected to maya/Duck." and
  the pet page reloads. Nothing opens on its own.
  - If the repository holds a version this Mac has not reviewed, the badge
    says "Newer in Mallard Cloud. Pull to update this computer." and the
    bar's Pull carries 1.
  - This is the project's rule. Today's "Connect and review" goes (owner,
    2026-10-01).
- **After Connect on the New row:** the repository is created now, private and
  empty, exactly as a project's registration creates it at Link. The binding
  is saved, and the chip reads "Not pushed · Nothing pushed yet."

### 4. Push and Pull

- **On the pet page,** Push and Pull open today's reviews holding that one
  pet.
  - The Push review row names where it goes, in the review's own
    vocabulary.
    - Line two gains one more readout beside Files and On disk: the cloud
      glyph and the address, mono, with the sentence in its tooltip ("Pushes
      to Rika321/Duck").
    - A row the Push will create also carries the title-line readout "New
      repository" (`sync-history-branch-state`), as rows already carry
      "Install pet v2".
  - The outcome is today's `SyncOutcomeNotice`: "Done · Push complete · 1
    completed". On a single pet it adds the draft as the lead stat, "Draft
    1", the way a project prints "Version N".
- **On the row page,** the batch reviews stay. On Mallard Cloud each Push row
  gains the same destination, so the batch's new repositories are named
  before they exist. This is the bulk form of the New row: ten new pets are
  one review, not ten dialogs. The outcome names where each pet went.
- **On any other storage,** the row's reviews and outcome stay as today: one
  publish, and "Version N" as the lead stat.

### 5. The chip panel

The project's panel, unchanged:

- Request public release, or View release request
- View draft
- View published version
- Discard draft… (Owner, public, draft ahead)
**No Change repository…, no Disconnect** (owner, 2026-10-01): a connected
pet keeps its repository. One deleted on the website reads Gone, and Push
creates it again at the same address.

Release and discard name the saved binding. On a pet whose repository was
learned from the account's listing but not yet saved by a Push or Pull, those
two verbs wait, as `cloudPinMoved` makes them wait for a project.

### 6. Add from Mallard Cloud

New project › From Mallard Cloud, for a Catalog item: someone else's
repository — shared with you, or by address or invite code. Your own are
already in the list. It starts in two places:

1. **The row page's bar:** a + beside refresh (tooltip "Add from Mallard
   Cloud…"), where "Cloud repositories" is today. A dialog for this row's kind: the context row,
   the field and its found row, then **Shared with you** — another's
   repository you belong to whose item no row has yet ("kai/Biscuit ·
   Private · Member · Version 2"), or "Nothing shared with you yet."
2. **The sidebar's Catalog +.** "Add Catalog resource" gains New project's
   two doors (`ModeDoor`): **On this computer** (today's Kind, then Agent
   profile) and **From Mallard Cloud**, in New project's bands:
   - **Storage:** the account, or "No Mallard Cloud account yet." · "Set up
     Mallard Cloud".
   - **Repository:** the same field and Shared with you, across kinds, each
     row with its kind's mark ("Pet v2 · Private · Member").
   - **Catalog row:** where it lands, by the repository's kind ("Pets v2 ·
     Codex home"). With no row of that kind, the provider pills; Add creates
     the row.

- **Footer:** "Can't be changed later." · "Cancel" / "Add" ("Adding…").
- **Field errors beyond §3's:** "Pets v2 · Codex home already has Duck.";
  "Rika321/Owl is yours — already listed."; "maya/pdf-tools is a skill, not
  a pet." (the row's dialog only).
- **States** (specimen scene 19): loading, shared, nothing shared, found,
  those three errors, nothing there, joined, the code failing, adding, the
  add failing, and the door's two.
- **Add** saves the binding (from the door, also the row and its Mallard
  Cloud link when missing), then lands on the item's page: "Only in Mallard
  Cloud. Pull to download it here.", `Pull 1`, and "Biscuit added from
  kai/Biscuit." Nothing is pulled on its own, and the repository is fixed
  (§5).

### 7. Two repositories hold the pet

This is no longer a page failure.

- The row is blocked: "In 2 repositories you can see. Choose one on its
  page."
- The pet page shows a warning: "Duck is in 2 repositories you can see." with
  "Choose repository". That opens the Several reason, listing both. Pull and
  Push wait until one is chosen.

### 8. When something fails

Today's retries and recovery stay; Mallard Cloud only changes where they
appear. Completed work is always kept, and nothing retries by itself.

| Failure | The user sees | Retry or recovery |
|---|---|---|
| Mallard Cloud can't be reached, or refuses the account | The project's note: "Status unavailable" or "Repository unavailable" with **Retry**; "Sign-in required" with **Storage settings** | Retry reads again; nothing is written |
| The repository is gone | Chip "Gone": "Push creates it again." | Push recreates it at the same address; the binding stays |
| A chooser can't load or can't connect | "Couldn't reach Mallard Cloud." / "Mallard Cloud didn't answer. Try again." Nothing is saved | The verb stays enabled. Create is safe to repeat: an existing repository comes back |
| An invite code fails | "That code expired or was used. Ask for a new one." | A new code |
| One item fails mid-Push | "Pushed 1 of 3 pets v2", its row failed, the rest "not attempted"; the pushed ones keep their version | **Review remaining items** reopens Push with the rest (today's action) |
| An item's repository moved after review | It is held, not the batch: "held for Pull" | **Open Pull review** |
| A Push write is never acknowledged | One read-only check settles it; it is never sent again. A claim left without its record is reported | **Finish that push**: the user's verb. On the pet page it finishes that item; on the row page, every item, as today |
| One item fails mid-Pull | "Pull needs attention"; applied items stay | **Review remaining items** |
| The app quits during a Pull | The next page load replays the journal: the original restored from its backup, or a kept copy reused on retry, or "needs a fresh review" | Review again; nothing is replayed blindly |
| A review went stale | "This review is no longer available; review again." | Open the review again |
| Two repositories, or one refused | A blocked row and its note (§7) | Choose repository / Retry |

### 9. What goes away

- `CatalogCloudConnections` (the inline panel) and `CatalogCloudItem` (the
  chip on each row)
- the "Cloud repositories" panel; its button becomes the bar's +
- Change repository… on a pet (the chip's `onReconnect`)
- the per-row "Connect Cloud repository" button
- the chip's `onDisconnect` and the `disconnect_catalog_cloud_repository`
  command
- "Connect and review"

## Backend (`src-tauri/src/storage_catalog/service/cloud.rs`)

Summarized, with today's code for each change:
[PLAN_CATALOG_CLOUD_BACKEND.md](PLAN_CATALOG_CLOUD_BACKEND.md).

1. **`list_catalog_cloud_repositories`** takes the item and lists only the
   repositories that hold it.
   - Rows carry the project row's fields: address, website_url, visibility,
     role, can_push, revision, updated_at, registration_only. They add
     resource_id and the item's display name, and drop the Git fields.
   - The `{resource_id, address, link}` row goes.
2. **`resolve_catalog_cloud_repository` and `join_catalog_cloud_repository`**
   answer the project's resolution union instead of throwing:
   - collaborator
   - public
   - not_found
   - wrong_service
   - github
   - invalid
   - `not_this_kind` / `other_item` / `other_format`

   The last three are today's three error strings.
3. **New `create_catalog_cloud_repository(scope, storage, resource,
   expected_link_revision)`.**
   - It makes the same `CreateRepository { repository_class: "catalog", … }`
     call the first Push makes in `publish_library_manifest`, and saves the
     binding.
   - An account that already owns one gets the existing one back, as
     `create` already does on `AccountExists`.
4. **`disconnect_catalog_cloud_repository`** is removed.
5. **`observe_managed_items`** stops failing the page on a second repository
   for one item. It leaves that item out of the observation and reports it.
   The row is `blocked` with the sentence in §7.
6. **A refused sign-in on the page's request** is answered as a
   `ManagedCloudStatus` (`sign_in_required`) beside the rows, not as the
   page's load error.
7. **Add (§6):** the listing also answers "shared with you" — across kinds,
   the repositories this account belongs to that no row connects — and the
   resolution union carries the repository's kind. Add from the door
   creates the Catalog row and links Mallard Cloud when they are missing,
   then connects.

Unchanged:

- connect, status, release, discard
- the per-item publication: one item per repository, one stream lock per
  item, a batch that stops at the first failure and keeps every completed
  acceptance
- every other storage's Library path: the coordinate's one repository, its
  single publish, its plans (`observed_head`, `head`) and its version

## Frontend

- **`managed-cloud/RepositoryPicker.tsx`** is moved out of
  `project-sync/RepositoryAddressPicker.tsx`.
  - It holds the field, the answer states, the "Repositories" list and the
    New row. It takes `list`, `resolve` and `join` callbacks, and the host's
    vocabulary (the `<code>` word, the found row's chips, the refusal
    sentences).
  - The address helpers come with it: `normalizeRepositoryName`,
    `predictRepositoryAddress`, `repositoryVerdict`.
  - It imports no feature DTO and no IPC, which the existing boundary test
    already checks for `managed-cloud/`.
- **`project-sync/RepositoryAddressPicker.tsx`** becomes the project's
  vocabulary over it: the Git chips, the acknowledgement, and clone mode.
  Project behavior does not change.
- **`storage-catalog/CatalogConnectDialog.tsx`** is new. It holds the three
  reasons (Connect, Several, Add) and replaces `CatalogCloudConnections.tsx`.
- **`AddCatalogScopeWorkspace.tsx`** gains New project's doors; From Mallard
  Cloud reuses the picker, as `CloudProjectSource` does for a project.
- **The pet page is a mode of `StorageCatalogPage`,** `{ kind: "item",
  resourceId }`, not a new component. It adds:
  - the leaf crumb
  - the bar actions for one item: chip, notices, `cloudSyncBlocked`
  - the Repository section, only while the item has no repository
  - the page's own `ResourceCard` for the one row, under the list's section
    heading

  Its Push and Pull reuse `openReview(kind, [resourceId])`. A review opened
  from the pet page lists that pet only, and closing it returns to the pet
  page.
- **Behind `managedCloud` only.** The page already computes this flag from
  the storage's provider. The following exist only when it is true:
  - the item mode
  - the opening name on a row
  - the Push review's destination
  - the bar's + (Add from Mallard Cloud)
  - the page-level sign-in note

  With any other storage the page renders exactly what it renders today.
- **The DTOs** change in Rust and `src/components/storage-catalog/types.ts`
  together.

## Steps

Each step ships a working app.

1. **The dialog.**
   - Share the picker.
   - Replace the inline panel with `CatalogConnectDialog`. The row's
     connect button opens it; the chip's Change repository… goes.
   - The bar's "Cloud repositories" button becomes the bar's + (Add from
     Mallard Cloud); Add Catalog resource gains the From Mallard Cloud door.
   - Backend items 1 to 4 and 7: the listing, the resolution union, create,
     removing disconnect, and what Add needs.
2. **The pet page.**
   - Add the drill-in, the chip in the bar, the notices and the gating.
   - Remove the chip from each row and the connect button from each row,
     which also removes the N status reads.
3. **The row page.**
   - Destinations in the Push review.
   - The page-level sign-in note.
   - Two repositories per item as a blocked row (backend items 5 and 6).

## Tests

- **`tests/frontend/storage-catalog.integration.test.tsx`:**
  - the API surface (create added, disconnect gone)
  - the New row's predicted address and its `-2`
  - each refusal sentence
  - a join selects the joined repository
  - Connect opens no review
  - the pet page mounts one chip, in the bar
  - gone, sign-in and unavailable notices, and Pull and Push closing as for a
    project
  - the list renders no chip
  - the single-flight and late-answer cases the panel tests pin today, moved
    to the dialog
  - a row on a local-folder storage renders as today: plain names, no Add
    from Mallard Cloud…, no destinations, and "Version N" in the Push outcome
- **`tests/frontend/cloud-project.integration.test.tsx`:** the project
  dialog unchanged after the picker moves.
- **`src-tauri/src/storage_catalog/cloud_flow_tests.rs`:**
  - create makes one private repository and saves the binding; a second
    create returns it
  - the resolution union, case by case
  - `exact_cloud_connections_disambiguate_identical_items_and_isolate_profiles`
    becomes a blocked row instead of a page error
  - `cloud_pet_connection_preview_pull_release_and_discard_share_the_repository`
    loses its disconnect leg

## Safety and schema

- **The publication protocol does not change.** Create is the call the first
  Push already makes.
- **Every schema stays 1** (owner, 2026-10-01): machine-local state,
  portable manifests, and the service API. The binding stays
  `cloud_repositories` in `catalog_scope_links.json`; plans live in memory;
  the new plan and result fields are Tauri contract changes only.
- **Connect still requires the same item identity** (`describe`), and a
  public non-member repository is still connected only by its exact address.
- **No invariant is relaxed.** Removing Disconnect deletes no data.

## Decided (owner, 2026-10-01)

- **Only Mallard Cloud changes.** Every other storage keeps today's Catalog
  row page and flow.
- **Schema stays 1.**
- **Connect no longer opens the Pull review.** The bar's Pull count says
  there is something to pull, as for a project.
- **The proposal is drawn in today's style,** from shipped parts only (the
  specimen's scene 0 is the shipped page).
- **A connected pet keeps its repository.** No Change repository…, no
  Disconnect; every chooser says "Can't be changed later."
- **Add from Mallard Cloud starts in two places:** the row page's bar, and
  Add Catalog resource's From Mallard Cloud door.

## Decisions to confirm on the specimen

1. **A pet page** with the chip in its path bar (recommended). The
   alternative keeps a chip on each row and only swaps the panel for the
   dialog, which leaves N status reads and no notice or gating.
2. **Connect on the New row creates the repository now,** as a project's Link
   does. The Push review still names the repositories it will create, so
   batch Push needs no dialog.
