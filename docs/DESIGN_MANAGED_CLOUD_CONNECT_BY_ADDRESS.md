# Managed Cloud: connect a project by address

Status: implemented 2026-09-15 (proposed and mocked the same day; section
8 lists what was built). Frozen mock: `cloud-connect-mockup.html`
(`src/cloud-connect-mockup.tsx`; `npm run dev`, then
`/cloud-connect-mockup.html?scene=member`). Live harness of the shipped
components against a stand-in backend: `/cloud-connect-live-mockup.html`
(`src/cloud-connect-live-mockup.tsx`, `?view=dialog|setup`). Section
numbers refer to `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md`.

This is a desktop change. The service already resolves an address
(`GET /v1/repositories/by-name/{owner}/{name}`, section 4.1) and answers
both faces with `bundle_id`, so nothing in `cloudflare/managed-cloud`
changes and the schema stays 1. The desktop is the side that has to
consume an address, and today it only emits one.

Compatibility: clean cutover. The silent link, the one-match auto-select,
the bundle-id rows and the "Create separate repo" verb go; no flag, no
second path.

## 1. How the desktop links a cloud repository today

**The storage is the account.** `ManagedCloudSetupLane` signs in with
GitHub over the loopback flow and saves `service_origin`, `account_id`,
`account_login` and the `account_token` secret into `sync_config.json`
(`storage_providers/managed_cloud.rs`). One storage per origin and
account.

**The identity is `bundle_id`, never the address.** `create_setup_draft`
mints a random 128-bit `bundle_id` (`BundleId::generate`,
`mallard-domain/src/lib.rs`) and sets `display_name` to the folder name.
Saving a link publishes a registration claim
(`save_project_link_with_repository` → `publish_registration_claim`), which
reaches `ManagedObjectStore::begin_upload` (`managed_cloud/store.rs`):
`GET /v1/repositories/by-bundle/{bundle_id}/head`, and on `204`
`POST /v1/repositories` with `name = display_name`, the class from the
fingerprint, and the bundle id. So linking a folder called `HorrorGame`
creates `Rika321/HorrorGame` on the service without saying so, and a second
folder with the same name creates `Rika321/HorrorGame-2`.

**Choosing an existing repository** happens in two places, both selecting
a bundle id from `list_remote_bundle_snapshots` (the service's
`GET /v1/repositories?class=project`, synthesized into `BundleTag` rows):

- The project setup page's Repository section
  (`ProjectSetupWorkspace.tsx`): "New repository" plus one row per
  repository, titled by `display_name` with the bundle id underneath. When
  exactly one row's `repository_fingerprint` equals the checkout's, the
  radio moves to it by itself (`repoAutoSelected`), against the section's
  own "no silent default" comment.
- Linking a storage to an existing project (`ProjectSync.tsx`
  `linkStorage`): `BundleConnectionDialog` opens only when the listing is
  non-empty and does not already hold the project's bundle id. An empty
  account, the common first-time case, links silently and creates the
  repository named after the folder.

`SiblingRepoNotice` catches the aftermath: two machines that each minted a
bundle id for the same Git remote produce two repositories, and
`SiblingBundleStatus::RegistrationTwin` picks a survivor.

**The address is emitted, never consumed.** `get_managed_cloud_link`
builds `website_url = {origin}/{owner_login}/{name}` for the storage card;
no Rust file calls `by-name`. A public repository the account does not
belong to cannot be connected at all (wiring plan, section 6, risk 1).

## 2. The change

One component, `RepositoryAddressPicker`, decides the cloud repository
for a Mallard Cloud storage. It is the body of `BundleConnectionDialog`
when the storage is `managed_cloud`, and the Repository section of the
setup page once a Mallard Cloud storage is chosen. It always opens: no
listing shortcut.

Every part is one the app already draws; nothing new is designed.
Top to bottom (mock, directions A and B):

1. **Repository address**, in the Folder field's shape
   (`sync-folder-field`: name, input, the **Look up** verb beside it, a hint
   under). It takes what the website's address bar shows.
2. **The answer**, in the storage discovery's own rows: a found row
   (`storage-found-row`: the class mark, `owner/name` with the class beside
   it, a verdict line such as `Yours · Owner` or `Public · anyone signed
   in can push`, `Visibility` and `Git remote` chips, a caution when there
   is one, **Open on website**, and the verb **Use this**); the setup
   page's inline status while the lookup runs; the
   discovery empty state for a miss; the setup page's field error for a
   refused address.
3. **Repositories you belong to**: the existing option rows, each titled
   `owner/name` with `Visibility · Role` beneath, the time at the right,
   `Git match` when the fingerprint is the checkout's. Picking a row fills
   the address field; the field and the rows are one choice.
4. **New repository** is the first row. Its second line is the address it
   will create, from the project name (`Rika321/HorrorGame-2 — the name
   HorrorGame is already yours`).

Dialog footer: **Cancel** and one primary verb: **Connect and review Pull**
or **Create and connect**. On the setup page the band's badge and
**Finish** carry the state. No bundle id anywhere. (2026-09-16: the
read-only verb and caution went with the open-push contract of
`DESIGN_MANAGED_CLOUD_OPEN_PUBLIC_PUSH.md`; an **Enter an invitation code**
toggle under the address field, and the code field on a miss, join a
private repository from the picker. 2026-09-27: the address field takes the
code too, and its verb turns from Look up to Join; see
`PLAN_REPOSITORY_PICKER_TIDY.md`.)

### 2.1 Address grammar

Accepted, in order of what a person is likely to paste:

| Input | Reading |
| --- | --- |
| `https://mallard-ai.com/Rika321/HorrorGame` | host + address |
| `https://mallard-ai.com/Rika321/HorrorGame/sessions/…?commit=…` | the tail after `{owner}/{name}` is dropped (`sessions`, `release`, a trailing slash, a query) |
| `mallard-ai.com/Rika321/HorrorGame` | scheme-less |
| `Rika321/HorrorGame` | bare address; the storage's origin is assumed |

The host must be the storage's `service_origin` host, compared
case-insensitively: a staging address pasted into the production storage
is refused before any request, as a field error ("That address is on
staging.mallard-ai.com; this storage is Mallard Cloud at mallard-ai.com").
`www.` is accepted for production. A `github.com` address is refused with "That's a GitHub
address. Paste the Mallard Cloud address from the repository's page."
Segments must be login-shaped and name-shaped (`validate/names.ts` rules,
mirrored in Rust); the website's own top-level paths (`invitations`,
`super-admin`) are not addresses.

### 2.2 Resolution

`GET /v1/repositories/by-name/{owner}/{name}` with the storage's token.

| Answer | Card | Primary | Connect |
| --- | --- | --- | --- |
| Summary with `role` in owner/admin/member | Type, Visibility, You: role, Git remote from `repository_fingerprint` | Connect and review Pull | the summary's `bundle_id` through the existing paths |
| Summary with `role: null`, `writer: true` (a public repository, 2026-09-16) | Public · anyone signed in can push; Git remote from `repository_fingerprint`, which the summary now discloses | Connect and review Pull | same bundle id; the card later wears a `Contributor` chip |
| `404` | empty state: nothing at that address that you can see; if it is private, paste its invitation code in the same field (the verb turns to Join; `accept_managed_cloud_invitation` joins and selects) | Join | the joined repository's bundle id |
| `repository_class: catalog` | error: that address is a skill, plugin intent, or pet, not a project | disabled | |
| class is `git_project` but the folder has no remote, or the reverse | the existing acknowledgement | Connect and review Pull once acknowledged | |

**Git remote.** For a collaborator the service discloses
`repository_fingerprint`; the chip compares it with the checkout's. For a
public repository the fingerprint stays private but `github_url` is shown,
and it is the canonical form of the same origin URL, so the desktop
compares `normalize_github_repository_url(origin)` with it and treats
equality as verified. A differing or unknown remote takes the existing
acknowledgement checkbox and the Activity review, unchanged.

**Contributor (was: read-only).** Since 2026-09-16 the service takes a
push from anyone signed in on a public repository (`requireWriter`,
`G.writer`); `headByBundle` answers its latest push, so Pull and Push both
work for a non-collaborator. `get_managed_cloud_link` reports `role: null,
can_push: true` and the storage card shows a `Contributor` chip. Request
public release is offered through `can_request_release` as for a member.

**New repository.** Creation stays the registration publish of section
5.3, and the requested name stays the project's `display_name`, so there is
one name: the Project band's Name field on the setup page, the project's
name (renamed where it is renamed today) for an existing project. The New
row previews the normalized address (`normalizeRepositoryName` mirrored in
Rust: runs of other characters become one hyphen) and the `-2` suffix when
the account already holds the name, predicted from the listing; the service
stays the authority and the storage card after linking prints what it
stored.

### 2.3 Connect semantics, by entry point

- **Existing project, new Mallard Cloud storage** (`linkStorage`): the
  dialog opens every time. A row or a resolved address connects through
  `connect_project_to_remote_bundle` with `expected_bundle_id` and
  `allow_repository_mismatch` as today; New repository is `save_project_link`
  as today, after the name edit.
- **Project setup** (`ProjectSetupWorkspace`): a row or a resolved address
  sets `DraftRepositoryChoice::Existing { storage_id, bundle_id,
  display_name: "owner/name", repository_fingerprint, mismatch_acknowledged }`;
  New repository leaves `DraftRepositoryChoice::New`. The auto-select effect
  goes; `Git match` stays as a badge and the section stays "Choose one"
  until a click.
- R2 and Dropbox storages keep their rows and dialog as they are; the
  address field and the card render only for `managed_cloud` descriptors.

## 3. Desktop changes

Rust (`crates/mallard-infrastructure/src/managed_cloud/`,
`src-tauri/src/project_sync/`):

- `client.rs`: `repository_by_name(owner, name) -> Result<Option<RepositorySummary>>`
  (`404` → `None`); `RepositorySummary` gains `github_url: Option<String>`
  and `visibility` is already there.
- `stub.rs`: `GET /v1/repositories/by-name/{owner}/{name}` with the
  service's two faces (collaborator summary; public face when public with a
  publication; `404` otherwise), so the desktop tests cover every outcome.
- `address.rs` (new): `parse_repository_address(input, service_origin)`
  → `Ok(RepositoryAddress { owner, name })` or `AddressError::{WrongService
  { host, expected }, GitHub, Invalid}`, with a table test over section 2.1.
- `managed_cloud_link.rs`: command `resolve_managed_cloud_repository(storage_id,
  local_project_id: Option, address) -> ManagedCloudRepositoryResolution`, an enum
  `Collaborator { repository }`, `Public { repository }`, `NotFound`,
  `WrongService { host, expected }`, `NotAProject`, `Invalid { reason }`;
  `repository` carries `owner_login, name, website_url, bundle_id,
  repository_class, visibility, role, git_remote: Matches | Differs | Unknown`,
  the last computed against the local project's fingerprint or canonical
  origin as section 2.2 says. `get_managed_cloud_link` adds `role` and
  `can_push` so the card can show `Contributor`.
- `BundleSnapshotSummary` (the DTO, not the `BundleTag` file) gains
  `address: Option<String>`, `visibility` and `role` for managed storages;
  `store.rs` already holds the summaries the listing is synthesized from.

Frontend (`src/components/project-sync/`):

- `RepositoryAddressPicker.tsx` (new): the Folder-shaped field, the lookup,
  the found row, the empty state, the field error, the rows, the New row.
  Both hosts pass the storage, the listing, the local project's name and
  fingerprint, the current choice, and callbacks. The found row is
  `CandidateRow`'s markup with its own verdict words, so `StorageDiscovery`
  either takes the verdict as a prop or lends its classes to a sibling.
- `BundleConnectionDialog.tsx`: for a managed storage the body is the
  picker; the footer verbs of section 2; the info notice moves into the
  header sentence.
- `ProjectSync.tsx` `linkStorage`: managed storages always open the
  dialog; the listing-contains-bundle and empty-listing shortcuts go.
- `ProjectSetupWorkspace.tsx`: the picker in the Repository section; the
  auto-select effect removed.
- `api.ts`, `types.ts`: `resolveManagedCloudRepository`,
  `ManagedCloudRepositoryResolution`, the `BundleSnapshotSummary` fields,
  `ManagedCloudLink.role`.
- Copy as in the mock. `App.css` gains only what the mock's inline sheet
  adds: the band's parts stack (`repo-connect`), the option rows' second
  line in the text face when it is prose (`repo-rows`), and in the 680px
  dialog the found row's facts wrap under the name.

Website, optional and separate: a **Copy address** button in the
repository header that copies `{host}/{owner}/{name}`, so the address a
person pastes into the desktop is one click on the page they are looking
at.

## 4. Flows

**Second machine, same Git repository.** Add the folder, choose the Mallard
Cloud storage, paste the address from the website (or pick the `Git match`
row). The card says the remote is this checkout's; Connect and review
Pull. No `HorrorGame-2`, no sibling notice.

**Invited collaborator.** Enter the repository's invitation code on the
website or in the desktop's picker (the address field takes the code too,
and its verb turns to Join); the repository is selected at
once and the row is in the listing from then on. Role shows as Member;
Pull and Push both work.

**Public repository, not a collaborator.** Paste the address; the card says
`Public · anyone signed in can push`. Connect and review Pull pulls the
latest push; Push moves it; the storage card wears `Contributor`.

**New project.** The New repository row says the address it will create,
with the name editable; Create and connect registers it at revision 1.

## 5. Edge cases

- Addresses compare case-insensitively (the service does); the card and
  rows print the service's spelling.
- A rename on the website moves the address; links are by bundle id, so
  nothing breaks and `website_url` refreshes on the next read.
- A removed repository answers `404` by name; an existing link's by-bundle
  head answers `204`, handled as an absent head today.
- An expired session fails the lookup with `ReconnectRequired` before any
  parsing feedback, so the reconnect prompt wins.
- Look up runs on the button or Enter, never per keystroke; picking a row
  fills the field without a request.
- Pasting the address of a repository the account owns under a different
  bundle id is flow 4.1, the case the change exists for.

## 6. Tests

- Rust: the address table; `stub.rs` by-name faces; the resolution command
  for member, public, `404`, wrong service, catalog; the engine parity test
  unchanged.
- Frontend integration: the dialog opens for a managed storage with an
  empty listing; the setup section does not move the radio on one match;
  the picker's outcomes render the copy of section 2.2.
- Service: `test/repositories.test.ts` already covers both faces of
  by-name; nothing to add.

## 7. Size

| Piece | Lines |
| --- | --- |
| client, stub, address parser and tests | 250 |
| resolution command and DTOs | 150 |
| picker component | 250 |
| dialog, setup section, linkStorage, api, types | 180 |
| frontend integration tests | 120 |

## 8. As built

Clean cutover, schema 1, no service change. Rust:

- `crates/mallard-infrastructure/src/managed_cloud/address.rs`:
  `parse_repository_address(input, service_origin)` with the section 2.1
  table as its test.
- `client.rs`: `RepositorySummary.github_url`; `repository_by_name` (404 →
  `None`).
- `stub.rs`: `Repository.visibility` / `github_url`, `StubState::release`,
  both faces in `summary`, `GET /v1/repositories/by-name/{owner}/{name}`,
  the by-bundle fallback to a public repository, and a non-member's reads
  of a public repository (summary, manifest, objects; uploads refused).
- `src-tauri/src/project_sync/managed_cloud_link.rs`: `ManagedCloudRepository`
  (address, website URL, class, visibility, role, the known remote
  fingerprint, `git_remote: matches | differs | unknown`, revision, counts),
  `ManagedCloudRepositoryResolution` (`collaborator`, `public`, `not_found`,
  `not_a_project`, `wrong_service`, `github`, `invalid`), and the commands
  `list_managed_cloud_repositories(storage_id, local_repository_fingerprint)`
  and `resolve_managed_cloud_repository(storage_id, address,
  local_repository_fingerprint)`. `ManagedCloudLink.role` for the card.
  Both hosts pass the local fingerprint rather than a project id, so the
  setup draft and an existing project use the same command.

The remote fingerprint a caller knows is the summary's for a collaborator,
the SHA-256 of the disclosed `github_url` for a public repository (the
same value, since the fingerprint is the digest of that canonical
address), none for a non-git project, and withheld otherwise. `matches`
means equal to the local one, both absent included; the existing
acknowledgement rule (`remote != local`) then needs no change.

Frontend:

- `src/components/project-sync/RepositoryAddressPicker.tsx`: the picker,
  `useManagedCloudRepositories`, `managedCloudAccount`,
  `predictRepositoryAddress` (the service's normalization and `-n` suffix,
  mirrored), and the verdict / chip / caution words. A resolved address
  selects its repository; a row click fills the field; the found row
  offers `Open on website` and, when another row was picked since, `Use
  this`; an invitation-code field (`acceptManagedCloudInvitation`) joins a
  private repository and selects it. Since 2026-09-27
  (`PLAN_REPOSITORY_PICKER_TIDY.md`) one field, "Address or invitation
  code", takes both: Look up, or Join once the text has no `/` or `.` and is
  20 characters without separators (`isInvitationCodeShaped`). A repository
  that is already a row is selected there, with one line under the field
  instead of the found card (`inListing`). "Your repositories" heads the
  rows, and with none it is one quiet line rather than an empty list box.
- `BundleConnectionDialog.tsx`: `ManagedCloudBody` for a `managed_cloud`
  storage (address, rows, New repository for `reason: "link"`, footer verb
  `Create and connect` / `Connect and review Pull`);
  `BundleListBody` unchanged for other providers. `onUseExisting` now
  receives `{ bundleId, allowRepositoryMismatch }`.
- `ProjectSync.tsx`: a Mallard Cloud storage opens the dialog for `link`
  and for a missing remote without listing bundles first.
- `ProjectSetupWorkspace.tsx`: the picker in the Repository band for a
  Mallard Cloud storage; the one-match auto-select removed for every
  provider; `Git match` stays a badge.
- `ProjectLinksWorkspace.tsx`: a `Contributor` chip when `role` is null (a
  public repository anyone signed in can push to).
- `App.css`: the three rule groups from the mock, appended.
- Tests: `tests/frontend/managed-cloud-connect.integration.test.tsx`;
  Rust tests in `address.rs` and `managed_cloud_link.rs` (stub-backed:
  owner, stranger before and after release, refusals without a request,
  the listing).

Not done: the optional website `Copy address` button.
