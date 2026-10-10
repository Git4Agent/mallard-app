# Unicode Names for Repositories and Library Items

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

Status: implemented October 7, 2026 (uncommitted in both trees) with the
recommended choice of every decision below; staging and production still
owe the D1 reset, and the live pass (Verification) is open. Spans the
service and website (`mallardInternal/cloudflare/managed-cloud`) and the
desktop (`mallard-app`). No schema migration and no compatibility path: the
one schema file is edited in place and rides the D1 reset the uncommitted
versions change already requires (`docs/PLAN_CLOUD_VERSIONS.md`).

## Problem

A Mallard Cloud repository's address is `{owner}/{name}`. Today `name` is
GitHub's ASCII set, and a title with nothing in that set is silently stored
as `repository`:

- the service derives the name at creation (`src/validate/names.ts`,
  `normalizeRepositoryName`: `"  日本語 "` → `"repository"`, a test asserts
  it) and refuses any other spelling at rename (`routes/repositories.ts`,
  `renameRepository`);
- the desktop has three more copies of the same rule: the Catalog's preview
  (`src-tauri/src/storage_catalog/service/cloud.rs`,
  `normalize_repository_name`), the service stub
  (`crates/mallard-infrastructure/src/managed_cloud/stub.rs`) and the
  frontend preview (`src/components/managed-cloud/repositoryAddress.ts`),
  each ending in `"repository"`;
- the desktop's address parser rejects a non-ASCII name and never
  percent-decodes a path segment (`managed_cloud/address.rs`, `name_shaped`
  and `repository_in`), so `/Rika321/%E5%A4%A7%E9%BB%84` (the spelling a
  browser hands `mallard://open?address=`) and `/Rika321/大黄` are both
  "not an address";
- the frontend only treats ASCII `owner/name` as an address to look up
  (`src/components/managed-cloud/RepositoryLookupField.tsx`,
  `isAddressShaped`); anything else filters the rows;
- the website shows the address segment as the repository's name in every
  heading, card and search result (`web/src/pages/RepositoryPage.tsx`,
  `components/RepositoryCard.tsx`, `ListingCard.tsx`, `SearchLauncher.tsx`,
  `pages/HomePage.tsx`), although every publication already carries the
  pushed `display_name` (`summary_json`, STORE_API section 2), and the
  public listing, viewer rows and the by-name face all return it;
- Owner-uniqueness is `COLLATE NOCASE` (`migrations/0001_init.sql`, index
  `repositories_owner_name`). D1, the service's existing database, is
  SQLite, whose built-in NOCASE folds the 26 ASCII letters and nothing else
  ("only ASCII characters are case folded", sqlite.org/datatype3) and
  offers no custom collation, so two spellings of one Unicode name would
  both be accepted. No new dependency is involved anywhere in this plan's
  database work.

A project called 大黄 therefore lives at `Rika321/repository`, the website
calls it `repository`, and its link cannot be opened in the app.

## Standards and precedents

- RFC 3987 (IRIs): a path may carry any Unicode; on the wire it is
  percent-encoded UTF-8, and the two spellings name one resource. Browsers
  show the decoded form (Wikipedia: `/wiki/東京` ↔ `/wiki/%E6%9D%B1%E4%BA%AC`).
- UAX #15: compare and store names in NFC, so `é` (one scalar) and `e` +
  combining acute are one name.
- Unicode default case mapping for case-insensitive equality;
  `String.prototype.toLowerCase` and Rust `str::to_lowercase` both implement
  it, SQLite's NOCASE does not.
- UAX #31 / UTS #39: identifiers are letters, marks and numbers; format and
  control characters (ZWJ, bidi controls) are excluded; confusable detection
  is a separate, optional layer.
- Agent Skills specification (agentskills.io/specification): a skill's
  `name` is 1–64 characters of `a-z`, `0-9` and `-`, and must equal its
  directory. Claude Code and Codex load skills by that rule.
- Products. GitLab separates a project's *name* (letters, digits, emoji,
  spaces) from its *path* (`[A-Za-z0-9._-]`, the URL); GitHub and Hugging
  Face use one ASCII name for both; Notion and Linear keep a Unicode title,
  a slug in the URL and an id that never moves.

The model below is GitLab's split with an IRI path: a repository has an id
that never changes, an address name that is Unicode, and a display name
that comes from what was pushed.

## Model

| | Set by | Changes when | Shown |
| --- | --- | --- | --- |
| `id` (`repo_…`) | the service at creation | never | nowhere |
| `name` (address) | derived once from the requested title; renamed only in website Settings | Owner renames it | the address line, links, the Trail crumb, the address field |
| display name | the selected snapshot's `summary.display_name`: a project's bundle display name at push, a skill's effective name, a plugin's id, a pet's display name | the next push, the version shown | headings, cards, lists, selectors, search |

Rules that follow:

- A display-name change (renaming the project in the app, a pet's
  `display_name`) travels with the next push and never touches `name` or
  `id`. Already true: the desktop sends the title only at creation
  (`managed_cloud/store.rs`, `publish_manifest` and
  `publish_library_manifest`) and the service never renames by itself. A
  test pins it.
- A page shows the display name of the snapshot it shows. The public face
  carries `public_summary` only (`repositorySummary`, `access.privileged`
  false ⇒ `private_summary: null`), so a visitor never reads a draft's
  name; `?version=N` shows version N's; the draft shows the draft's.
- Lists show the head the viewer may read, which the service already
  selects (`viewerRowJoins`: the private head for a collaborator, the public
  head otherwise).
- The Owner login stays GitHub's ASCII rule (`isLoginShaped`).

## The name rule

One rule, stated once per language (TypeScript shared by service and
website, Rust on the desktop, and a TypeScript mirror for the frontend
preview), all checked against the same case table (below).

- Trim, then NFC-normalize. A name is stored and compared in NFC.
- 1 to 100 Unicode scalar values.
- Each scalar is a letter (`\p{L}`), a mark (`\p{M}`, not as the first
  scalar), a number (`\p{N}`), or one of `.` `_` `-`. Nothing else: no
  space, `/`, `%`, `?`, `#`, control or format character, symbol or emoji.
  Emoji sequences depend on ZWJ and variation selectors, which make equality
  unstable and the URL unreadable; GitLab's slug rule excludes them too, and
  the display name carries them anyway.
- Not `.` or `..`.
- `key(name) = toLowerCase(NFC(name))`. Owner-uniqueness, the by-name
  lookup, the rename check and the desktop's preview all compare keys.
- `derive(title)`: trimmed and NFC-normalized; a valid title is kept
  exactly (so `--Repo--` and `.github` survive, as today); otherwise every
  run of other scalars becomes one `-`, `-` is trimmed from both ends, the
  result is cut to 100 scalars and trailing `-` trimmed again. A result
  that is not a valid name is a **refusal**, not `repository`.
- Suffix: `base-2`, `base-3`, … by key while the Owner holds the key, the
  base cut so the whole fits 100 scalars.

Case table (every suite asserts these):

| Requested | Name | Why |
| --- | --- | --- |
| `HorrorGame` | `HorrorGame` | valid, kept |
| `Horror Game` | `Horror-Game` | space → `-` |
| `大黄` | `大黄` | CJK letters |
| `  日本語  ` | `日本語` | trimmed (was `repository`) |
| `Café` with a decomposed `é` | `Café` composed | NFC |
| `大 黄 / 二` | `大-黄-二` | runs → one `-` |
| `--Repo--` | `--Repo--` | valid, kept |
| `.github` | `.github` | valid, kept |
| 150 × `x` | 100 × `x` | cut by scalars |
| 99 × `x` + `-yyy` | 99 × `x` | cut, trailing `-` trimmed |
| `😀abc` | `abc` | symbol at the edge |
| `🦆🦆`, `.`, `..`, `///`, `` | refused | nothing usable |

Duplicates by key: `大黄` with `大黄` already held → `大黄-2`; `Ärger` held
→ `ärger` is taken; a 100-scalar base held → 98 scalars + `-2`.

Addresses that all resolve to `Rika321/大黄`: `Rika321/大黄`,
`Rika321/%E5%A4%A7%E9%BB%84`, `mallard-ai.com/Rika321/大黄/`,
`https://www.mallard-ai.com/Rika321/%E5%A4%A7%E9%BB%84/sessions/x?commit=1`,
and `mallard://open?address=https%3A%2F%2Fmallard-ai.com%2FRika321%2F%25E5%25A4%25A7%25E9%25BB%2584`.

## Service

`src/validate/names.ts` is rewritten and stays pure (the seed script and
the website import it): `isRepositoryName`, `nameKey`,
`deriveRepositoryName(title): string | null`, `withSuffix`,
`pickAvailableName(base, takenKeys)`, `REPOSITORY_NAME_RULE` ("letters or
digits of any script, '.', '_' and '-'; 1 to 100 characters; no spaces").
`FALLBACK_NAME` goes. The regex is `^[\p{L}\p{M}\p{N}._-]{1,100}$` with the
`u` flag (the quantifier then counts code points) plus a first-scalar check.

Schema, `migrations/0001_init.sql` edited in place: `repositories.name_key
TEXT NOT NULL` beside `name`, holding `nameKey(name)` computed by the
Worker in JavaScript (D1 cannot compute a Unicode fold itself), the unique index `repositories_owner_name`
becomes `(created_by, name_key) WHERE state = 'active'` and the NOCASE one
goes. Every write of `name` writes `name_key` (create, rename). Staging and
production take the README's reset (purge-r2, reset-remote.sql, migrate,
deploy) that the versions change already owes them.

`routes/repositories.ts`:

- `createRepository`: `deriveRepositoryName(spec.name)`; `null` →
  `invalid("The name needs a letter or digit. Rename the project, or the
  pet, and push again.")`, raised **before** the storage marker is put (today
  the marker goes first; a refusal must leave no orphan prefix).
  `namesHeldBy` returns keys; the insert writes `name_key`; the race retry
  is unchanged.
- `renameRepository`: `isRepositoryName` by the new rule; the `forbid` and
  the `UPDATE` use `name_key`.
- `getRepositoryByName`: the router already percent-decodes a segment
  (`router.ts`, `decodeSegment`, 256 UTF-16 units ≥ 100 scalars of any
  plane); the handler validates the decoded name and calls
  `loadRepositoryByName(db, user.id, nameKey(name))`.

`db/queries.ts`: `RepositoryRow.name_key`; `loadRepositoryByName` compares
`name_key`; any other `name COLLATE NOCASE` (grep) compares keys.
`routes/library.ts` keeps printing `clash.name`. `scripts/seed-staging.ts`
uses `deriveRepositoryName`. STORE_API.md and section 4.1 of
`docs/CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` restate the rule and drop
the `repository` sentence; `web/src/docs/pages.tsx` ("The address") says
"Letters or digits of any script".

Tests: `test/addresses.test.ts` becomes the case table;
`test/repositories.test.ts` adds: create `大黄` → `name` `大黄`; by-name with
the raw and the encoded spelling answer the same id; a second create of a
decomposed `大黄`-shaped title → `-2`; rename to a key the Owner holds →
`409 repository_name_taken`; rename to `大 黄` → `400`; create `🦆🦆` → `400`
and no row, no marker, no audit; anonymous by-name on a public `大黄` returns
`public_summary` and no `private_summary`.

## Website

- `links.ts` drops its copy of the rule and imports
  `../../src/validate/names` (pure, no Worker imports; Vite and `tsc`
  compile imported files outside `include`). `repositoryHref` keeps
  `encodeURIComponent`; `useParams` hands pages decoded values
  (react-router 7).
- `RepositoryPage.tsx`: the canonical redirect compares `nameKey`, not
  `toLowerCase`; the heading shows the display name of the shown snapshot
  (`manifest.bundle.display_name` for a project, `itemName(envelope, …)`
  for an item, `summary.name` while loading) with the address `owner /
  name` as the line beneath; the Trail crumb keeps `summary.name` (it
  mirrors the path); Settings › Name uses the new rule and sentence, drops
  `maxLength={100}` (UTF-16 units, not scalars) and relies on
  `aria-invalid`.
- `RepositoryCard.tsx`: `Card.displayName` from
  `(private_summary ?? public_summary)?.display_name ?? name`
  (`fromJoined`), `row.summary.display_name` (`fromPublic`),
  `row.summary?.display_name ?? row.name` (`fromViewerRow`). `ListingCard`,
  `LandingPage`'s tile and `RepositoryTile`'s label show `displayName`; the
  address stays in `title=` and `aria-label`.
- Search: `HomePage` filters and `SearchLauncher.score` match
  `nameKey(displayName)` and `nameKey(fullName)`; "Go to address" accepts
  the new rule; sort by name uses `localeCompare` on the display name.
- Addresses stay addresses: `PullMenu`'s address field, `VersionList`
  links, `InvitationsPage`, `ReviewsPage`, `SuperAdminPage` (its confirm
  compares `repository.name`). `pull.ts` is unchanged: `linkTo` encodes
  the page address once more for the query, as it does for session ids.
- Tests (`web/test`): cards print `大黄` and link to the encoded href;
  the redirect fires for a case difference and not for a different key;
  search finds `大黄` by `大` and by `rika321/大黄`; the launcher offers
  the address row for `Rika321/大黄`; the public page of a repository whose
  draft was renamed still prints the released display name.

## Desktop, Rust

- New `crates/mallard-infrastructure/src/managed_cloud/names.rs`:
  `nfc`, `is_repository_name`, `name_key`, `derive_repository_name ->
  Option<String>`, `pick_available_name(base, &BTreeSet<key>)`. Uses the
  `regex` crate already in the crate (Unicode classes, `{1,100}` counts
  chars) and adds `unicode-normalization` for NFC (small, the ecosystem's
  standard; `icu_normalizer` is only in the tree transitively).
- `address.rs`: `repository_in` percent-decodes each segment
  (`percent_encoding::percent_decode_str(..).decode_utf8()`, as the
  session tail already does), NFC-normalizes, keeps `login_shaped` ASCII
  and replaces `name_shaped` with `names::is_repository_name`. Bare and
  encoded spellings read the same.
- `client.rs`, `repository_by_name`: the two path segments are
  percent-encoded with an `AsciiSet` that leaves `A-Za-z0-9._-` alone.
  Every `website_url` built as `{origin}/{owner}/{name}`
  (`project_sync/managed_cloud_link.rs`, `describe_repository`; the Cloud
  link's `website_url`) encodes the name segment the same way, so Open on
  website opens a valid URL.
- `storage_catalog/service/cloud.rs`: delete `valid_repository_name`,
  `normalize_repository_name` and the body of `predicted_repository_name`;
  call `names::`. A refused title predicts `None` and the row says so.
- `stub.rs`: `names::` for create (refuse like the service) and by-name
  (compare keys); the local copy goes.
- DTOs: `ManagedCloudRepository` and `ManagedCloudListingRow` gain
  `display_name` (the tag's `display_name`, else `name`), filled where
  `describe_repository` and the listing rows are built; `src/types.ts` and
  `src/components/managed-cloud/types.ts` change with them.
- `project_sync/app_link.rs` is unchanged in shape; its tests add an
  encoded and a raw link that both describe `Rika321/大黄`.
- Tests: `names.rs` holds the case table; `address.rs` the address set
  above; `stub.rs` create `大黄` and the `-2` duplicate; `cloud.rs` the
  prediction (`大黄` → `大黄`, refusal → `None`); `codex_pet.rs` captures
  `pets/大黄/` with a decomposed and a composed directory name to one
  resource id and refuses `pets/大 黄/`.

## Desktop, frontend

- `repositoryAddress.ts` mirrors `names.rs`: `isRepositoryName`,
  `nameKey`, `deriveRepositoryName | null`, `predictRepositoryAddress` →
  `{ address, name, base, suffixed } | { refused: string }`; the
  `"repository"` fallback goes. The New repository row
  (`RepositoryAddressPicker.tsx`, `StorageCatalogPage.tsx` item page,
  `CloudRepositoryChoice.tsx`) prints the prediction or the one refusal
  line, "Name needs a letter or digit; rename the project first". No new
  control: the Push still goes to the service, which refuses with the same
  sentence, and the outcome shows it.
- `RepositoryLookupField.isAddressShaped`: a name segment is
  `[\p{L}\p{M}\p{N}._-]+` or percent-encoded (`(?:%[0-9A-Fa-f]{2})+`), with
  the `u` flag; hosts and logins stay ASCII.
- `CloudScopes.matchesFilter`: compares `nameKey` of the needle against
  `nameKey` of the address, the display name and the description.
- Rows (`RepoRowCopy` and the picker's `display_name: repository.address`)
  show the display name as the title with the address beneath, as the
  Catalog's Cloud rows already do. `AppLinkSheet` prints the address as
  the service spells it (`Rika321/大黄`).
- Local projects: `deduplicated_local_alias`
  (`src-tauri/src/project_sync/service.rs`) compares NFC-normalized trimmed
  names, so the "(2)" suffix follows the same equality as the Cloud.
- Tests: `tests/frontend/managed-cloud-connect.integration.test.tsx` takes
  the case table and the duplicate/refusal predictions;
  `cloud-project.integration.test.tsx`: `Rika321/大黄` and its encoded form
  are addresses, rows print the display name;
  `app-link.integration.test.tsx`: an encoded link lands on the sheet as
  `Rika321/大黄`; `storage-catalog.integration.test.tsx`: a pet 大黄 predicts
  `login/大黄`.

## Library items

What a Library item is called on Mallard Cloud is its display name, and the
address is now derived from it with the script kept (pet 大黄 → `login/大黄`,
not `login/repository`). Pet ids become Unicode too. Skill and plugin
identifiers stay as they are, for the providers' reasons, and the plan says
so where the requirement asks for "all Library items":

- Skills: `effective_name` and `install_directory` are the Agent Skills
  spec's `[a-z0-9-]{1,64}` and must equal the directory; Claude Code and
  Codex would not load a Unicode one. `validate_component_name`
  (`storage_catalog/domain.rs`) stays. Mallard's bound (128 bytes, `._`
  allowed) is already looser than the spec and is not tightened here.
- Plugins: `namespace@plugin` ids are the marketplace's
  (`plugin_intent.rs`). Unchanged.
- Pets: Codex puts no character rule on a pet id. Its hatch-pet skill
  (openai/skills, `.curated/hatch-pet`) writes `pets/<pet-name>/` and sets
  `"id"` to that directory, and the desktop UI "picks up" any directory
  under `pets/`, so a pet hatched as 大黄 sits at `~/.codex/pets/大黄/` with
  `"id": "大黄"`. The ASCII bound is Mallard's own (`validate_pet_id` →
  `validate_component_name`, 64 bytes) and it goes:
  - `validate_pet_id` gets its own rule, the address rule plus a
    directory's needs: NFC; letters, marks, numbers and `._-`; starts with
    a letter or number; not ending in `.`; not `.` or `..`; 1 to 64 scalars
    and at most 255 bytes. Skills keep `validate_component_name`.
  - `capture_pet` NFC-normalizes the directory name and the manifest `id`
    before comparing them and before the identity is encoded, so the
    resource id (a digest over the identity object,
    `storage_catalog/domain.rs`, `derive_resource_id`) is one id for one
    pet on every machine: HFS+ stores names in NFD, APFS and NTFS keep what
    was written. Restore and the canonical `pet.json` write the NFC form.
  - The pet copy name (`rename`, "pet copy name") follows the same rule,
    and the Catalog's fork seed keeps a Unicode directory name as it is.
  - The service's envelope check (`validate/catalog.ts`, `identity.pet_id`,
    64) counts code points, not UTF-16 units.
  - The website's "Download for Codex" (`web/src/pull.ts`, `safeName`,
    `petEntries`) squeezes a folder name to ASCII and would ship 大黄 as a
    folder called `pet` with `"id": "pet"`. `safeName` keeps
    `[\p{L}\p{M}\p{N}._-]`, still drops path separators and leading
    `.`/`-`, and never climbs out of the zip's folder.
  - `display_name` is already any script (`validate_display_text`, 128
    bytes, about 42 CJK characters).

## Safety and schema impact

- Portable manifests are unchanged: `bundle.display_name` and the catalog
  descriptor already carry any script, bounded (256 chars on the service,
  1,024 bytes on the desktop).
- Machine-local schema 1 is unchanged. The service's D1 schema file
  changes in place (`name_key`); no migration file is added.
- The Push publication path is untouched: the only new refusal happens
  before the marker and before any object transfer, so a refused create
  leaves nothing in R2 or D1.
- Percent-decoding happens once, on one segment at a time, after the
  2,048-byte and whitespace bounds; a segment that decodes to `/`, `%`, a
  control or a format character is not a name.
- Confusable names (UTS #39) are not detected: the namespace is one
  Owner's, whose login is ASCII, so a look-alike can only shadow that
  Owner's own repository.

## Verification

Suites: `npx vitest run` (service, 217+), `npx vitest run --config
web/vitest.config.ts` and `npx tsc --noEmit -p web/tsconfig.json` (website,
Node 22), `npm run build`, `npm run test:frontend-integration`, `cargo test
--manifest-path src-tauri/Cargo.toml --lib` (desktop).

Live pass, wrangler on 8787 with the website's vite on 5173 and `npm run
tauri dev` against a fixture `CODEX_HOME`; staging after:

1. **Chinese project.** New non-Git project `大黄` in an empty folder named
   `大黄`; Push to Mallard Cloud → `login/大黄`. The website heading reads
   大黄 over `login / 大黄`; `/login/大黄` and `/login/%E5%A4%A7%E9%BB%84` open
   the same page; the browser's address bar shows the decoded form.
2. **Chinese pet.** Pet at `<CODEX_HOME>/pets/大黄/` with `"id": "大黄"` and
   `displayName` 大黄 (as hatch-pet writes it); the Catalog lists it, Push →
   `login/大黄-2` (the project holds `大黄`). Its card and page read 大黄;
   the website's Download for Codex unzips to a folder `大黄` whose
   `pet.json` says `"id": "大黄"`; Pull on a second fixture home restores
   `pets/大黄/`; a copy of the pet named `大黄二` is accepted.
3. **Duplicates.** Rename the project to `大黄` spelled with a decomposed
   mark and push from a second `~/.mallard` → `login/大黄-3`. Website
   Settings: rename `大黄-2` to `大黄` → "already holds it in any spelling".
4. **Invalid.** Project `🦆🦆` → the New row shows the refusal; Push fails
   with the service's sentence; `GET /v1/repositories` lists no new row and
   R2 has no new prefix. Rename to `大 黄` on the website → the rule.
5. **Display name vs address.** Release version 1 of `大黄`, rename the
   project to `大黄二号`, push: the public page still reads 大黄 (version 1),
   the collaborator's draft reads 大黄二号, the URL and id are unchanged,
   `?version=1` reads 大黄.
6. **Desktop links.** Open in Mallard on `/login/大黄` → the sheet reads
   `login/大黄`, the picker selects it; paste `login/大黄`,
   `mallard-ai.com/login/%E5%A4%A7%E9%BB%84` and the page link into the
   picker → one repository; Pull in Mallard on a session of it lands on
   that session; Open on website from the Cloud chip opens the page.
7. **Search.** Website search `大` and `大黄` find the card; the launcher
   offers Go to address for `login/大黄`; the app's picker filters `大` to
   the row.

## Decisions for the owner

Taken as recommended on October 7, 2026 (the owner accepted the
recommendation; the heading shipped without a separate specimen).

1. Character set: letters, marks and numbers of any script plus `._-`
   (recommended), or also symbols and emoji in the address.
2. A title with nothing usable: refuse the create and say why
   (recommended; the requirement removes the silent substitution), or add a
   name field to the New repository row.
3. Library identifiers: skills and plugin ids stay ASCII by their
   providers' rules; pet ids become Unicode, since Codex has no rule and
   its own hatch-pet skill names the directory after the pet
   (recommended). Display names carry the script.
4. Uniqueness: a `name_key` column and index in `0001_init.sql`
   (recommended; exact, rides the pending reset), or keep the ASCII index
   and check Unicode case in the handler only.
5. The repository page heading: display name over the address (A,
   GitLab's project header; recommended), address only with the display
   name as the card's title (B), or display name with the address as a pill
   (C). A specimen in `mockups/` before the website change, per the usual
   review.

## Out of scope

Confusable detection; internationalized hostnames; the GitHub login rule;
redirects from an old address after a rename (the service has none today);
skill and plugin identifiers (above); file names inside non-Git project
sync, which already travel byte-faithfully.

## Order

1. Service: `names.ts`, schema, routes, tests; website imports the rule.
2. Desktop Rust: `names.rs`, `address.rs`, `client.rs`, stub, Catalog
   preview, DTOs; tests.
3. Desktop frontend: `repositoryAddress.ts`, lookup field, filter, rows;
   tests; `npm run build`.
4. Website: heading (after the specimen), cards, search, Settings copy,
   docs page; tests.
5. Reset and deploy staging; live pass 1–7; production.
