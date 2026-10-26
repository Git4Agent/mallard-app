# Mallard Managed Cloud Architecture V1

- **Status:** Implementation plan
- **Date:** September 13, 2026
- **Product target:** Managed Cloud MVP
- **Supersedes:** the September 13 MVP draft of this document. Section 13
  records what changed and why.

This document is the build order for the first Managed Cloud release. It is
self-contained: section 3 restates the desktop facts the plan builds on,
section 10 restates every portable contract field by field, and the source
pointers at the end are for readers who want to see the code, not
prerequisites. Sections 5 to 7 are written so an engineer can start from
them without another design pass.

## 1. Scope

### 1.1 Fixed decisions

| Area | Decision |
| --- | --- |
| Content | Git projects, non-Git projects with reviewed files, and one item per Catalog repository |
| Sign-in | GitHub OAuth; the Worker holds the client secret; one account, several machines |
| Access | One Owner, Admins, Members; anyone signed in pushes to a public repository; one Owner-set invitation code per private repository; a separate Super Admin service role |
| Private storage | One replaceable private snapshot per repository; no browsable private history |
| Public release | The approved private snapshot becomes the next public revision as-is; no server-side merge |
| Public history | Every public revision is retained within quota and readable by collaborators |
| Approval | The Owner approves every release on the website; absent or revoked admission also requires Super Admin |
| Website | Anonymous browsing of public repositories, including every session, session details, and skill, plugin, and pet pages; signed-in review and approval |
| Desktop | Managed Cloud is a storage provider behind the existing bundle engine; Push and Pull screens are unchanged |
| Storage | One Worker for the API and website assets, D1 metadata, private R2 objects, one cron Worker |
| Removal | Immediate controlled removal by Owner, Admin, or Super Admin; Super Admin can remove one public revision |
| Recovery | D1 Time Travel, immutable R2 writes, a written runbook, one isolated restore test |

### 1.2 In the MVP

- GitHub sign-in on the desktop and the website; one managed storage per account per machine.
- Private Push and Pull for Git and non-Git projects through the existing engine, including reviewed project files.
- Catalog items: Codex and Claude skills, plugin install intent, Codex pets v1 and v2, one item per repository.
- Session views: every pushed conversation ships a bounded timeline the website renders.
- Collaborators, the repository invitation code, open push on public repositories, roles, leaving, removal.
- Release requests, Owner approval, Super Admin approval when admission is absent, public history.
- Website: public listing, repository pages, session pages, catalog item pages, private snapshot review, approvals, Super Admin workspace.
- Quota, cleanup, expiry, removal, audit, logs, and a tested manual recovery.

### 1.3 Not in the MVP

Google sign-in, account linking, device management, keychain storage of the
desktop token, featured or ranked listings, search, export, Queues,
dashboards, automatic repair, separate R2 backups, key rotation, billing,
cross-repository deduplication, direct R2 uploads, session conflict
resolution or merge, branches, private history, ownership transfer, per-file
permissions, Git source hosting, project-file sync for Git projects, plugin
binaries, Claude pets, pet format conversion, child-transcript pages on the
website, application-level content encryption, a removal cancel window, a
second managed storage per account. Add no dormant tables, routes, or fields
for them.

## 2. Terms

| Term | Meaning |
| --- | --- |
| Storage | One configured backend on a machine: a local folder, a user-owned S3/R2 bucket, a Dropbox folder, or Managed Cloud. Identified by a `StorageId`. |
| Bundle | The portable form of one project. Identified by a `BundleId`, a 32-hex-character ID owned by the project registration, not by any storage. |
| Scope | One typed section of a bundle: `project` (reviewed files and directories) or `provider:<key>` (conversations and provider state for Codex or Claude). |
| Manifest | The immutable JSON document naming every scope, resource, file, and object of one bundle revision. |
| Head | The one mutable pointer per repository naming the current manifest. Advanced only by compare-and-swap. |
| Commit ID | A random `commit-<hex>` ID minted by the desktop for each manifest. Never reused. |
| Object | Content-addressed payload bytes, keyed by SHA-256, verified by exact length and digest. |
| Repository | On the service: one project bundle or one catalog item with its members, private snapshot, and public history. Identified by an opaque `repo_…` ID. |
| Publication | On the service: one manifest at one scope (`private` or `public`) with its object references. |
| Private snapshot | The one current private publication of a repository. |
| Public revision | One retained public publication; the newest is the public head. |
| Session view | A bounded JSON timeline of one conversation, built by the desktop from wire bytes, rendered by the website. |
| Link | The machine-local record joining a local project or catalog scope to a repository on a storage, holding accepted bases. |
| Base | The manifest coordinates a replica last accepted for a scope: revision, commit ID, manifest digest and size. |
| Wire form | Transcript bytes as they travel: credential-redacted and, for Claude, with in-project paths replaced by tokens. Local bytes differ. |
| Coordinate | The catalog identity tuple `(provider, resource_kind, format, format_version)`. |
| Resource ID | A stable `resource-<hex>` derived from a coordinate and the item's canonical identity; unchanged when content changes. |
| Continuity | The desktop's classification of the relation between a replica's base and the observed head. |

## 3. The desktop today

Everything in this section exists and is unchanged by the plan. Section 5
plugs into it.

### 3.1 Storage providers and the object store

**Configuration.** A storage is a `StorageConfig { id, name, provider,
provider_config }` inside `~/.mallard/sync_config.json`, a document
`SyncConfig { schema, revision, storages, projects, links }` written with
optimistic concurrency on `revision` under a cross-process lock that is never
held across remote I/O. Secrets, including Dropbox's refresh token, live in
the same file (mode 0600) in a `SecretMap`; there is no OS keychain.

**Adapters.** A provider is one adapter module implementing
`StorageProviderAdapter` plus one entry in
`StorageProviderRegistry::production()` (Local, R2, Dropbox today). The trait
methods are `descriptor`, `validate_settings`, `validate_config`,
`summary`, `continuity_coordinates`, `local_root`, `journal_facts`,
`probe`, `reconcile_probe`, and `open`. A duplicate `(key, config_version)`
is a startup error. Neutral setup code (drafts, test, commit, rename, remove)
never names a provider.

**Descriptor.** The frontend renders `StorageProviderDescriptor` fields and
holds no provider list: `key`, `config_version`, `display_name`,
`description`, `default_name`, `name_suggestion`, `display_rank`, `icon`,
`clear_mode` (`unconditional` or `revision_checked`), `fields`, `secrets`,
`managed_secret_id`, `local_setup` (`wrangler` or `dropbox`),
`setup_guide_url`, `manual_setup_hint`, `discovery`. `local_setup` is the
one place the UI branches: it selects the setup lane component.
`managed_secret_id` marks a credential the user must not edit by hand;
the settings page then offers Reconnect instead.

**Object store.** Every backend implements one trait:

| Method | Semantics |
| --- | --- |
| `get_metadata(key, max_bytes) -> Option<{bytes, etag}>` | Read a small mutable object with its backend revision token |
| `stage_verified(key, {size, sha256}) -> Missing / Mismatch / Verified(body)` | Read an immutable object into a private temp file, verified by exact length and digest |
| `put_immutable(key, bytes) -> Written / AlreadyPresent` | Create-only write; a present identical object satisfies the write |
| `put_many(objects)` | Batch of `put_immutable` |
| `compare_and_swap(key, Absent / Match(etag), bytes) -> Written{etag} / Conflict{current_etag}` | The only way a head moves |
| `list(prefix, cursor, limit) -> page` | Used for discovery over the flat tags namespace and for clearing |
| `supports_conditional_delete`, `delete(key, etag)` | Optional; R2 says no |
| `clear_managed_data` | Wipe the Mallard namespace, marker re-verified first |
| `local_root`, `connection_health` | Optional |

`ConfiguredObjectStore` wraps an opened store in a process-wide table keyed
by `StorageId` and pinned to the exact config; every call passes through a
guard that short-circuits when the recorded `ConnectionHealth` suspends
(`ReconnectRequired`, `TokenExpired`, `PermissionDenied`) and broadcasts
state changes so the UI re-reads. Health evidence is bounded: a route
literal, an HTTP status, an error tag, and a short request ID.

Before any use, a storage must carry the marker `.mallard/_storage.json`
with `{ "format": "mallard-storage", "layout_version": 1 }`. Keys are
validated: no leading or trailing slash, no `\`, `:` or control characters,
no `.` or `..` components, components at most 255 bytes.

### 3.2 The bundle engine

**Key namespace.** Per project bundle under
`.mallard/v1/repositories/<bundle_id>/`:

```text
_head.json                               mutable head, compare-and-swap
_manifests/<revision>-<commit_id>.json   immutable manifest
_commits/<revision>-<commit_id>.json     immutable commit record
_objects/<sha256>                        immutable payload
_uploads/<setup_id>/…                    setup probe residue
```

plus one flat discovery namespace `.mallard/v1/repositories/_tags/<bundle_id>.json`
(a listing costs bundle-count, never object-count), and per catalog
repository under `.mallard/v1/library/<library-id>/` the same layout with
`_tag.json` inside the repository and no `_uploads/`.

**Head file.** `BundleHead { schema_version: 1, bundle_id, kind: "project",
revision, commit_id, manifest_key, manifest_sha256, manifest_size,
updated_at }`. The engine reads it with a 64 KiB cap, rejects a head stored
under another bundle, and carries the store's etag into a `HeadToken
{ revision, commit_id, manifest_sha256, manifest_size, etag }`.

**Tag file.** `BundleTag { schema_version, bundle_id, display_name, kind,
revision, updated_at, resources, files, registration_only, providers }`.
Advisory only; acting flows re-verify against the live head.

**Publish sequence.** One entry point, `publish_with_progress(request)`,
called from one production site in the project sync service:

1. Read the head, then stage the current manifest against the head's size
   and digest.
2. Mutation firewall: every reviewed scope pin `(revision, content_sha256)`
   must equal the live manifest's, else "changed since review; pull and
   compare again".
3. Start from the previous manifest's scopes. Unselected scopes are carried
   forward with exactly the same revision and digest. Each writable scope is
   rebuilt: retained files must match an `(object_key, sha256, size)` triple
   in the reviewed previous scope; a path cannot be both retained and
   captured; tombstones are minted inside the scope for explicit removals and
   carried forward; the scope revision becomes `previous + 1`, or 1. If no
   scope changed, publish returns `published: false` without writing.
4. Upload payload objects to `_objects/<sha256>`, deduplicated by digest,
   `cloud_transfer_concurrency()` at a time.
5. Write the manifest and the commit record in one `put_many` of two.
   The commit record holds `previous_commit_id`, file deltas, and the
   publishing origin; nothing reads it back.
6. Compare-and-swap `_head.json` with `Match(etag)` or `Absent`. Exactly one
   attempt; a conflict is a hard error ("compare again before pushing").
7. Best-effort tag update with up to four compare-and-swap retries; failure
   is ignored.

Linking a project registers it: `publish_registration` publishes an empty
revision 1 so the bundle is discoverable before its first push; a head
conflict there counts as success.

**Reads.** `inspect` reads head and manifest only. `byte_source` stages
payloads lazily, verified by size and digest, through a process-wide LRU of
staged bodies (256 MiB, keyed by storage, bundle, digest, size).
`inspect_manifest_version` and `fetch_manifest_version_optional` fetch a
historical manifest by `_manifests/<revision>-<commit_id>.json` and the
latter bypasses the cache because its purpose is to witness the remote store.
`list_remote_bundles` reads the flat tags namespace once, capped at 10,000.

### 3.3 Links, bases, review flows, and continuity

**Project link.** `ProjectStorageLink { local_project_id, storage_id,
bundle_id, recipe, scope_bases, project_content_preferences,
conversation_checkpoints, pinned, created_at }`, machine-local. A
`ScopeBase { scope, scope_revision, scope_sha256, reviewed_in:
RepositoryBase, binding_revision, last_pull_at, last_push_at }` advances only
after a successful push or pull of that scope. `RepositoryBase { revision,
commit_id, manifest_sha256, manifest_size }`. The project's root and provider
profiles live in `ProjectBinding` in `machine_projects.json`, which carries
no storage ID.

**Push.** `prepare_push_review(local_project_id, storage_id)` returns a
`PushReviewContext` (inventory, destination status, thread comparison,
project files, `credential_redaction_enabled`, `prepared_at`); nothing is
stored between review and apply and there is no expiry. `push_bundle`
carries `PushApproval { credential_redaction_enabled, subagent_selections,
recipe, project_content_review_token, project_content_removal_ids,
acknowledged_warning_digests }`. Bytes are captured at apply, not at review;
freshness is proven by the content-hashed review token and by the redaction
flag matching. Warnings are acknowledged by digest bound to the exact
reviewed bytes.

**Pull.** `plan_bundle_restore(storage_id, bundle_id, binding)` returns a
`PullReview` (`plan_id`, storage, bundle, host and replica, revision,
`commit_id`, `manifest_sha256`, `binding_revision`, `created_at`,
`expires_at` = 24 hours, actions, conversation review rows, project-file
eligibility, `remote_continuity`). `apply_bundle_restore(review,
approved_action_ids)` rejects an expired review, rebuilds actions, verifies
target digests immediately before each write, backs up, and replaces
atomically. Deletion is opt-in with digest verification.

**Wire form.** Each conversation entry carries `wire_redaction { codec,
fingerprint }`: `mallard.redact.v1` plus the ruleset fingerprint when
redaction is on, `mallard.redact.off.v1` plus 64 zeros when off. Codex and
Claude both require the stamp; a mismatch blocks. Claude entries also carry
`wire_path_map`. Tail comparisons are pin-relative: cloud bytes compare to
the accepted wire pin, local bytes to the accepted local pin.

**Continuity.** The shared plan phase classifies the repository from the
observed head, the recorded base, and one probe:

| # | Condition | Continuity |
| --- | --- | --- |
| 1 | No head, no base | `Fresh` |
| 2 | No head, base recorded | `RemoteMissing` |
| 3 | Head present, no base | `Unreviewed` |
| 4 | Revisions equal, digests equal | `InSync` |
| 5 | Revisions equal, digests differ | `Replaced` |
| 6 | Head revision below base revision | `Replaced` |
| 7 | Head revision above base revision, probe hits | `Advanced` |
| 8 | Head revision above base revision, probe misses or mismatches | `Replaced` |

The probe fetches `_manifests/{base.revision}-{base.commit_id}.json` from
the store, never from the cache, and stages it against the base's size and
digest. Row 8 is why a service must keep old manifests readable: absence of
the base manifest is read as rewritten history.

### 3.4 The catalog model

A catalog scope is one local `(provider, profile)` under one coordinate.
Registered coordinates:

| Provider | Kind | Format | Version | Capture | Targets |
| --- | --- | --- | --- | --- | --- |
| `codex` | `agent_skill` | `agentskills.io.skill` | 1 | directory tree | `provider-home.skills`, `shared-user.skills` |
| `codex` | `plugin_install_intent` | `mallard.plugin-install-intent` | 1 | provider inventory | `user.plugins` |
| `codex` | `desktop_pet` | `openai.codex-pet` | 1 | directory tree | `desktop.pets` |
| `codex` | `desktop_pet` | `openai.codex-pet` | 2 | directory tree | `desktop.pets` |
| `claude` | `agent_skill` | `agentskills.io.skill` | 1 | directory tree | `provider-home.skills` |
| `claude` | `plugin_install_intent` | `mallard.plugin-install-intent` | 1 | provider inventory | `user.plugins` |

Identity: `CatalogResourceRepositoryId::for_coordinate` = `library-` plus
32 hex characters derived from the coordinate only, so one storage holds one
Library repository per coordinate with many items. `derive_resource_id` =
`resource-` plus a digest over the coordinate and the item's canonical
identity; content changes do not change it. A skill is identified by its
name and must contain `SKILL.md`; plugin intent stores marketplace source,
name, and resolution policy and never plugin code; a pet is identified by
its name and format version and carries an atlas image with declared
geometry and byte limits. Files travel as `CatalogPortableFile { path,
sha256, size, mode }` with mode `0644` or `0755`, at `_objects/<sha256>`.

The Library engine (`inspect`, `publish`) runs over the same object store
and publishes the whole coordinate's selected resources as one
`LibraryResourceManifest`. Links: `CatalogScopeLink { link_id, scope_id,
storage_id, repository_id, accepted_head, accepted_resources, revision,
created_at, updated_at }`. Commands: `prepare_catalog_scope_push(scope_id,
storage_id)`, `apply_catalog_scope_push(plan_id, selected_resource_ids,
acknowledged_warning_digests)`, `prepare_catalog_scope_pull(scope_id,
storage_id, requested_copy_names)`, `apply_catalog_scope_pull(plan_id,
choices, destructive_acknowledged)`; plans live in memory. Pull can preserve
a divergent local skill or pet as a reviewed copy; plugin intent resolves
through a provider operation or a manual action.

### 3.5 The session detail model

Provider adapters build a metadata-only, bounded `SessionTimelineIndex`
from a transcript file (`build_timeline_index(path, context)`) and read
filtered pages from it (`read_timeline_page`), where pages carry the visible
text and previews the index deliberately omits. The desktop's session page
renders `ProjectSessionDetail` plus pages of `SessionTimelineEvent` (fields
in section 10.4) with the components under
`src/components/project-sync/session-detail/`: `SessionDetailPage`
(loads through IPC), `SessionDetailHeader`, `SessionDetailToolbar`,
`SessionDetailTimeline`, `SessionDetailEvent`, `SessionDetailToolCall`, and
the pure `sessionDetailModel.ts`. Text renders through `react-markdown` with
`remark-gfm`. Event kinds are `prompt`, `response`, `intermediate_step`,
`checkpoint`, `tool_call`, and `agent_call`; agent rows carry a preview of
the child (`label`, `task`, `role`, `model`, `mode`).

### 3.6 The sign-in precedent

Dropbox setup is the template for a browser sign-in on the desktop:

- PKCE S256: a 32-byte random verifier and state, hex-encoded, held in
  zeroizing strings; the challenge is base64url of SHA-256 of the verifier.
- Callback: a real listener bound to `127.0.0.1:53682` (address reuse on
  Unix, exclusive on Windows, backlog 16), path `/dropbox/callback`,
  180-second timeout, at most 32 accepts, loopback peers only, `state` and
  single-occurrence query parameters validated. The response page carries a
  strict Content-Security-Policy, `Cache-Control: no-store`, and one script
  that rewrites the URL to `/dropbox/complete` so the code leaves browser
  history. An unrelated request cannot consume the pending flow.
- The Tauri command runs the receiver under `spawn_blocking`, pushes
  `Progress::AuthorizationLink { url, expires_in: 180 }` over a channel so
  the lane shows Copy link and Open browser (links are never opened
  automatically), and always joins the receiver so cancel frees the port.
- The refresh token is stored in the `SecretMap`; the runtime authorization
  refreshes before dispatch, never replays an uncertain mutation, and records
  `ReconnectRequired` on rejection, which suspends the store and surfaces
  the Reconnect band. An edit draft of a managed credential is refused with
  `storage_managed_credentials_locked`.
- Setup modes are `Create`, `Existing`, and `Reconnect`; the last requires
  an edit draft and pins the account.

The lane UI is a step list: `StepView { key, title, status, fact?,
controls?, body? }` with `status` in `todo`, `doing`, `done`, `failed`,
`stopped`, rendered as an ordered list, driven by a hook that owns the flow
state, a `Channel` of tagged progress events from Rust, a countdown helper,
and a map from typed failure codes to copy. The lane receives a
`R2SetupPort { blocked, beginRun, onRunStarted, adoptPrepared }` from the
page.

### 3.7 Conventions and gates

- No backward compatibility: obsolete paths are removed in the same change;
  the cutover integration test forbids pre-cutover commands.
- DTOs change in Rust and `src/types.ts` together; commands are
  `snake_case` with camelCase JavaScript argument keys and are registered in
  `src-tauri/src/lib.rs`; every frontend call goes through
  `projectSyncApi` or `storageCatalogApi` over `src/ipc.ts`, and a test
  asserts that no other file imports the Tauri core.
- `sync-review/` imports no feature DTOs and never calls Tauri;
  `storage-catalog/` never imports project-sync code, and vice versa in Rust.
- Every mutating command carries an expected revision; setup assistants
  hold single-flight `*Control` state with a cancel command and return typed
  failures with an error-code enum.
- Frontend tests are `node:test` over `renderToStaticMarkup` with injected
  fake APIs, plus source-text gates and a CSS hygiene gate that requires every
  color to be declared per theme as plain hex. Rust runs `cargo test
  --workspace --all-targets`; a `StorageProviderRegistry::with_test_provider()`
  and the storage-parity thread variants prove that a new provider is one
  registry entry with no common-flow change.
- Never log credentials, tokens, or absolute paths; keep credentials and
  checkout paths out of portable manifests; publication is immutable and
  SHA-256 verified; heads move by compare-and-swap; pull is plan-based with
  explicit approval and expiry; deletion is opt-in with digest verification.

## 4. Product rules

### 4.1 Repository classes and identity

| Class | Service identity | Portable content |
| --- | --- | --- |
| `git_project` | `bundle_id` + `repository_fingerprint` | `BundleManifest` with provider scopes |
| `non_git_project` | `bundle_id` | `BundleManifest` with provider scopes and a `project` scope of reviewed files |
| `catalog` | `(provider, resource_kind, format, format_version, resource_id)` | `LibraryResourceManifest` with exactly one entry |

Class and identity never change after creation. The service repository ID
(`repo_…`) is opaque and distinct from `bundle_id` and `resource_id`. The Git
fingerprint is SHA-256 of the normalized `origin` URL and is not unique across
the service; the raw URL stays local.

**Address.** Separate from its identity, every repository has a GitHub-style
address `{owner}/{name}`: the Owner's current GitHub login and a Unicode
`name` (letters, marks and digits of any script plus `.`, `_` and `-`; 1 to
100 code points; not `.` or `..`; stored in NFC), unique per Owner by key
(NFC, lower-cased: `repositories.name_key`) and freed when the repository is
removed. The service derives `name` from the requested one at creation
("Horror Game" becomes `Horror-Game`, `大黄` stays `大黄`, a title with nothing
usable is refused) and suffixes `-2`, `-3`, … while the Owner already holds
the key; a rename (section 7.4) is strict and moves the address without a
redirect. On the wire the name is percent-encoded UTF-8 (RFC 3987), and the
raw and the encoded spelling resolve alike. The display name shown in
headings and lists is the selected snapshot's `summary.display_name`, never
the address: a display-name change travels with the next push and moves
neither `name` nor `id`.
The website (section 8) and the desktop's Open on website link use the
address; the API keeps the id, and `GET /v1/repositories/by-name/{owner}/{name}`
resolves an address to a summary. A GitHub login rename moves the account's
addresses after its next sign-in; when GitHub has handed a login to another
account, the most recently signed-in row holds it.

A project repository is created by the desktop the first time it publishes a
bundle the service does not know (section 5.3). The desktop derives the class
from the manifest: a `repository_fingerprint` makes it `git_project`,
otherwise `non_git_project`.

### 4.2 Draft, versions and the public version

Since 2026-10-05 (`PLAN_CLOUD_VERSIONS.md`) a repository holds three things,
and no page or message uses one word for another:

| Thing | Meaning | Who moves it |
| --- | --- | --- |
| Draft | The latest push: the stream's selected record. It moves with every push and carries no number. | Desktop push by any writer (section 4.3) |
| Version | A frozen, numbered copy of one push. Numbers start at 1 and are never given again. It can carry a name and a note. | Publish, by a collaborator; or the approval of a release request that names the draft |
| Public version | The version the public pointer names. A private repository has none. | Owner approval of a release; Super Admin approval when required; removal of that version |

| State | Meaning |
| --- | --- |
| `private_publication_id` | The draft's push, or none |
| `version_count` | Version numbers given so far; the next version is one more |
| `public_publication_id`, `release_count` | The version row the public pointer names, and how often the pointer moved |
| `public_admitted_at` | Timestamp of current Super Admin approval; null requires approval before a release |

A push moves the draft. Earlier pushes stay stored and stay readable by
whoever may read the draft, at their commit, until Discard draft removes
them. Publish freezes the draft as the next version and makes nothing
public. Only collaborators publish: a version protects its push from
Discard draft, so an account that is not on the team asks for a release with
the draft instead, and the Owner's approval makes the version.

Push semantics are the engine's (section 3.2): unselected scopes are carried
forward with their revision and digest, reviewed retained files are kept,
and tombstones are minted for explicit removals. The service stores
manifests; it does not interpret scope semantics.

A version copies nothing. Its row references the same manifest object and
the same payload objects as the push it froze, and a release moves the
pointer alone. Payload bytes are charged once per repository. Whoever may
read the draft reads every version at its own address (`?version=3`);
anonymous readers see only the public version.

### 4.3 Roles and invitations

Two rules (2026-09-16). **A public repository takes pushes from anyone
signed in**: no invitation; whoever is signed in reads its latest push,
pushes over it under the head compare-and-swap of section 6.5, pulls it, and
may request a release. The Owner decides every request. A Super Admin also
decides when admission is absent or revoked. **A private repository is entered by its one invitation
code**, which belongs to the repository: the Owner or an Admin creates,
rotates, or revokes it, and whoever enters it joins as a Member for as long
as it stands. "Public" keeps section 4.4's meaning (a release was approved
and a public head exists), so before its first release every repository is
private and only the code admits anyone.

A *writer* is a collaborator, or any signed-in account on a public
repository. A signed-in non-collaborator of a public repository is called a
*contributor* on the website and the desktop; pushing grants no membership,
and `pushed_by` (section 8) is the only record of them.

| Capability | Contributor | Member | Admin | Owner |
| --- | --- | --- | --- | --- |
| Read the private snapshot and public history; Push; Pull; request a public release | Yes, while the repository is public | Yes | Yes | Yes |
| List members; leave | No | Yes | Yes | Yes (not leave) |
| Approve or reject a release | No | No | No | Yes |
| Set, rotate, and revoke the invitation code; remove Members and Admins; promote or demote | No | No | Yes | Yes |
| Rename; make private; discard the private snapshot; remove the repository | No | No | Yes | Yes |

Both sides offer the discard to the Owner and Admins on every repository, by
the service's one flag, `can_discard_draft` (section 4.5,
`PLAN_DELETE_DRAFT.md`): in view in every state, off with its reason while
there is nothing to remove.

Creation makes the creator the sole Owner. The Owner or an Admin sets the
repository's code on the website's Members tab and sends it by any channel;
it stays readable there for the Owner and Admins, never expires, and stands
until it is rotated (which closes the old code at once) or revoked. A recipient enters it on the
website or in the desktop's address picker. Entering it needs an active
repository and fewer than 50 members; entering it again, or as an existing
collaborator, changes nothing. Removing or demoting someone does not change
the code: rotate it when a person who knows it should no longer be able to
join. Making a public repository private closes it to contributors at once
(their next read answers `not_found`; an upload they opened cannot commit).

`users.service_role = 'super_admin'` is set by an operator with SQL (section
11.3). It grants no repository membership and no Push right on a private
repository; on a public one a Super Admin is a signed-in account like any
other.

### 4.4 Public release

1. A writer (section 4.3) requests a release on the website's Release tab;
   the desktop's row opens that tab. The request names one version, the
   latest unless another is chosen, or the draft as it stands, and pins the
   current public version (or none) and the repository revision. One open
   request per repository. A request for a version stays open while the team
   pushes on. A request for the draft is closed by the next push; Publish of
   that draft turns it into a request for the version.
2. The Owner opens the repository on the website, browses what the request
   names (sessions, session details, files, or the catalog item), reads the
   comparison with the current public version (added, changed, and removed
   resources by identity and digest), and approves or rejects. Approval
   acknowledges public disclosure of that exact snapshot.
3. If `public_admitted_at` is null, Owner approval moves the request to
   `pending_super_admin`. This applies before the first release and after
   Super Admin makes the repository private. Super Admin approves or rejects
   on the website. Releases with current admission publish on Owner approval.
   The API returns `requires_super_admin_approval`.
4. The release moves the public pointer to the version and makes the
   repository public. For a request that names the draft it first makes that
   push the next version, published by whoever asked. The draft and the other
   versions stay as they are, and an earlier version can be released as well
   as the latest.

Making the repository private stops public reads and access by contributors
without membership. It keeps the draft, every version, members, stars, and
stored objects. Owner/Admin Make private preserves current admission. Super Admin
Make private clears `public_admitted_at`, requires a reason and the current
repository revision, invalidates open release requests, and records the actor
and reason in the audit log. The repository remains active. Release decisions
check admission inside their transaction. Downloaded copies cannot be revoked.

### 4.5 Discard and removal

**Discard private snapshot** is the Owner's and an Admin's, after a
confirmation, and takes effect in its request
(`PLAN_CLOUD_REPOSITORY_GONE_PUSH_AGAIN.md`, 2026-09-26; the rule of what is
kept is `PLAN_CLOUD_VERSIONS.md`, 2026-10-05). It returns the draft to the
latest version, public or not: every push after the one that version froze
is removed, with the content only those pushes held, and that push and the
ones before it stay with their claims. With no version it removes every push
and the repository stays, empty. It closes an open request that names the
draft, leaves a request for a version open, deletes nothing local, and
changes no version and nothing public.

**Repository removal** is immediate. An Owner or Admin confirms the display
name and the current revision. The transaction sets `state = 'deleting'`,
makes the repository private, revokes codes, invalidates requests, and ends
the open upload. From that request on the repository stops counting: it no
longer stands for its bundle or its Library item, holds its name, or counts
toward the account's limit, so the same project can be created again at the
same address at once. The cron Worker writes the external removal record,
erases every object and manifest of the repository's own prefix, and marks it
`removed`. Nothing is deleted on any machine. IDs and keys are never reused.

### 4.6 Super Admin

| Capability | Where |
| --- | --- |
| Inspect any repository, private or public; audited | Website Super Admin workspace |
| Approve or reject an Owner-approved release when admission is absent or revoked | Same |
| Make an active public repository private and revoke admission; no membership required | Same; reason and current repository revision required |
| Remove any repository immediately | Same |
| Remove one version, public or not | The API (section 7.7) |

Version removal marks the version `removed`, drops its object references,
and, if the public pointer named it, clears the pointer and makes the
repository private. The row stays as a marker in the versions list and its
number is not given again. Shared bytes remain while another available
publication references them.

### 4.7 What the website shows

For a public project repository, anyone can open the repository page, the
list of every session in the public head, each session's detail page, and the
list of reviewed project files. For a public catalog repository, anyone can
open the item page: a skill's `SKILL.md` and file list, a plugin intent's
source and policy, or a pet's idle frame and metadata. Whoever may read the
draft sees the same pages for the draft and for every version: a pop-up at
the head of the content names what is shown, the Versions tab lists every
version with Publish, and the Release tab asks which version to release.

Session pages render **session views** (section 5.4): bounded timelines the
desktop builds from the redacted, path-mapped wire transcript at push time.
The website never decodes provider transcripts.

### 4.8 Stars

A star is a vote. Anyone who can read a repository can star it, signed in or
not: every Star is +1 to `star_count`, up to 24 per voter per repository per
UTC day (`LIMITS.starsPerDay`), with no cap across repositories. Past that the
service answers `star_limit_reached` (429) until 00:00 UTC. `star_count` is
stars given, not people, and it keeps every one: only repository removal
takes stars away.

A voter is an account or, signed out, a connection: the IP address (an IPv6
address by its /64, which a client can move within at will) hashed with a
random secret made for the UTC day. The scheduled job deletes the day's
tallies and its secret once the day is over, and no address is ever stored.
Signed out, a star needs the website's `X-Requested-With` and `Origin`, the
rule a signed-in page's mutations follow, so another site cannot spend its
visitors' stars, and only a public repository with a public head can be
starred.

Signed in, Star also puts the repository on the account's Starred list, and
Starred (the toggle) takes it off without touching the count; starring again
puts it back and counts again. The website's control is unchanged: a split
pill, "Star | 12", which for a signed-out reader stays "Star" while the number
climbs. A star is a personal mark, not repository state: it never bumps
`revision` or `updated_at`, so it cannot cause a revision conflict for a
pusher or reorder the listing, and it writes no audit row.

Stars persist through visibility changes and membership changes. Read access
applies when the Starred list is read: a repository the account can no longer
read stays on the list but is not shown until it is readable again, and the
count keeps every star. GitHub erases stars when a repository goes private;
Mallard keeps history, so a re-release restores the count.

Every listing row and repository summary carries `star_count` and, for the
caller, `viewer_starred` (whether it is on the caller's Starred list).
`GET /v1/me/starred` lists that list, latest star first; collaborators see
their private head on their own repositories.

## 5. Desktop design

### 5.1 The managed store behind `ObjectStore`

The bundle engine publishes through the object store trait of section 3.1
and never names a provider. Managed Cloud is a fourth implementation,
`ManagedObjectStore`, so `publish_with_progress`, `inspect`, `byte_source`,
`build_restore_plan_from_snapshot`, `list_remote_bundles`, the continuity
probe, and every Push and Pull screen run unchanged. There is no second
manifest builder.

The store is opened per storage by the provider adapter with the service
origin, the account token, and the caller's membership resolved lazily per
repository. It maps keys to routes:

| Engine key (bundle-relative) | Managed request | Notes |
| --- | --- | --- |
| `_head.json` read | `GET /v1/repositories/by-bundle/{bundle_id}/head` | Synthesizes `BundleHead` from the head the caller may read (section 5.3); `etag` = commit ID; unknown bundle = absent |
| `_manifests/{rev}-{commit}.json` read | `GET /v1/repositories/{id}/manifests/{commit}` | Commit ID parsed from the key; retired manifests served to collaborators |
| `_objects/{sha256}` read | `GET /v1/repositories/{id}/objects/{sha256}` | Streamed; the engine verifies size and digest |
| `_objects/{sha256}` write | Buffered (section 5.2) | Returns `Written`; the object becomes visible only at commit |
| `_manifests/…` write | Starts the upload (section 5.2) | The manifest reveals class and identity |
| `_commits/…` write | Discarded | Nothing reads commit records; D1 holds the evidence |
| `_head.json` compare-and-swap | `POST /v1/repositories/{id}/uploads/{upload}/commit` | `Match(etag)` → `expected_commit_id`; `Absent` → `null`; 409 → `Conflict` |
| `_tags/{bundle_id}.json` read and list | `GET /v1/repositories?class=project` | Synthesizes `BundleTag` rows from publication summaries |
| `_tags/…` compare-and-swap | No request; returns `Written` | Discovery is the repository list |
| `_uploads/…` | Rejected | Probes are for user-owned stores |
| `.mallard/v1/library/…` | Rejected with `managed_catalog_uses_item_adapter` | Section 5.5 |
| `delete`, `clear_managed_data` | Unsupported | The descriptor advertises `clear_mode: none` |

`connection_health` maps 401 to `ReconnectRequired` (a revoked or expired
session is renewed by signing in again, never by pasting a token), 403 on
`/v1/me` to `PermissionDenied`, and network failure to `Unreachable`, so the
configured store's guard suspends the store and the Reconnect band appears
like Dropbox's.

### 5.2 Staged write

The engine writes objects first, then the manifest and commit record, then
swings the head (section 3.2). The service needs the manifest before it can
create a repository or reserve quota, so the store stages:

1. `put_many(_objects/…)`: keep `(sha256, size, Bytes)` in the store. `Bytes`
   is reference-counted, so this doubles no memory. Return `Written`.
2. `put_many([_manifests/…, _commits/…])`: parse the manifest; build session
   views (section 5.4); resolve or create the repository (section 5.3);
   `POST /uploads` with the manifest bytes and the complete inventory
   (payloads and views); receive `upload_id` and the `missing` digest list;
   `PUT` each missing object with `Content-Length` and `X-Mallard-Sha256`,
   `cloud_transfer_concurrency()` at a time; return `Written` for both keys.
3. `compare_and_swap(_head.json)`: `POST …/commit` with `expected_commit_id`.
   `200` → `Written { etag: new commit ID }`; `409 head_changed` →
   `Conflict`; any other failure is an error. The engine reports a conflict
   exactly as it does for R2.

A store instance holds at most one staged upload. Any error after step 2
leaves the upload open; it expires in 24 hours and the cron Worker cleans it.
Uploads run side by side on the service, so the next push simply starts a new
one. The store first ends an own open upload of the same manifest (the service
claims a commit id per open upload), creates the new upload, then ends its own
stale uploads, those whose pins no longer match, by
`POST …/uploads/{upload}/abandon`; the new upload already references the
objects they carried, so nothing is re-sent. Another account's upload is never
touched.

### 5.3 Repositories, discovery, and the public target

**Creation.** In step 2 the store looks up the bundle. If the service has no
repository for it, the store calls `POST /v1/repositories` with the class
derived from the manifest, the manifest's `display_name` as the requested
`name` (section 4.1: the service normalizes it), `bundle_id`, and the
fingerprint when present. The engine's registration publish at link time
(section 3.2) therefore creates the cloud repository the moment a project is
linked, with an empty revision 1, and the repository appears in the list at
once.

**Discovery.** `list_remote_bundles` reads the synthesized `_tags`
namespace. The service stores a summary per publication at upload creation
(`display_name`, `kind`, `revision`, `updated_at`, resource and file counts,
`providers`, `registration_only`) so a tag needs no manifest read. The
existing connect flow (`connect_project_to_remote_bundle` and the bundle
connection dialog) links a repository to a local folder on another machine;
non-Git repositories are mapped to a folder the same way.

**Which head a caller reads.** A writer (section 4.3: a collaborator, or
any signed-in account on a public repository) reads the private snapshot,
the latest push; `GET /v1/repositories/by-bundle/{bundle_id}/head` answers
it for the caller's own repository first, else for a public one under the
same bundle. A signed-in caller who is neither sees nothing for a private
repository. There is no per-link target switch and no second accepted base.
Anonymous access is the website's job; the desktop always signs in. The
desktop connects to a repository by its website address or, for a private
one, by entering its invitation code (`accept_managed_cloud_invitation`).

**Revision numbers.** The manifest keeps the revision and commit ID the
engine minted. The synthesized public `BundleHead` uses the manifest's own
revision, so the engine's head-to-manifest check holds and public revisions
seen by the desktop increase monotonically. A version's number is service
metadata: the manifest does not carry it.

### 5.4 Session views

A session view is one JSON object per conversation resource:

```json
{
  "schema_version": 1,
  "resource_id": "resource-…",
  "provider": "codex",
  "provider_session_id": "…",
  "title": "…",
  "summary": null,
  "started_at": 1757700000,
  "ended_at": 1757703600,
  "branch": "main",
  "recorded_sha": null,
  "tokens": { "value": 12345, "quality": "exact" },
  "event_counts": { "prompts": { "value": 4, "quality": "exact" }, "…": "…" },
  "events": [ "SessionTimelineEvent…" ],
  "truncated": false,
  "warnings": []
}
```

`events` is a list of `SessionTimelineEvent`; `tokens` and `event_counts`
are the existing `SessionCount` and `SessionEventCounts` (section 10.4). The
store builds a view in step 2 for every conversation whose transcript bytes
are in the staged upload: it writes the wire transcript to a private temp
file and runs the provider adapter's `build_timeline_index` and
`read_timeline_page` (section 3.5) page by page until the event cap or the
byte cap (section 6.5) is reached, then sets `truncated`. Because the input
is the wire form, redaction placeholders and path tokens appear exactly as
they will on the website. Agent rows keep the parent's call and result
preview; child transcript content is not in the MVP view.

Views travel in the upload inventory with `role: "view"`, `resource_id`, and
`source_sha256` (the transcript object's digest). At commit the service
carries forward the previous private publication's view for every
conversation whose transcript digest is unchanged, so an unchanged session
keeps its view without re-upload. A conversation without a view shows
"No preview" on the website. Catalog repositories have no views.

Views are ordinary content-addressed objects: charged once, cleaned up with
the publications that reference them, and served only through the view
routes.

### 5.5 Catalog item adapter

The Library engine publishes one repository per coordinate (section 3.4); a
managed catalog repository holds one item. The managed catalog path is
therefore a small adapter in `managed_cloud/catalog.rs` rather than the
engine:

- **Push:** `prepare_catalog_scope_push` and `apply_catalog_scope_push`
  branch on the storage provider. For managed storage, the selected
  resource's `CatalogResourceEnvelope` and `PublishedResourceEntry` are
  wrapped in a `LibraryResourceManifest` whose `resources` map has one entry
  and whose `repository_id` is the coordinate's
  `CatalogResourceRepositoryId`. The adapter uploads the entry's files as
  objects and commits with the same upload protocol. The service repository is
  created on first push with the item's coordinate and derived `resource_id`;
  a different item is refused.
- **Pull:** fetch the head manifest, verify each file object, and hand the
  envelope and bytes to the existing provider/format apply path. Skills and
  pets keep the reviewed-copy behavior; plugin intent keeps provider or manual
  actions.
- **Link:** a managed item link is `CatalogItemLink { link_id, scope_id,
  storage_id, repository_id (service), resource_id, accepted_head, revision,
  created_at, updated_at }` beside `CatalogScopeLink`. One local scope can
  link several managed items.

The existing codecs define identity, validation, and limits. The service
accepts only the coordinates in section 3.4.

### 5.6 Provider adapter, configuration, setup lane

- `ManagedCloudAdapter` implements `StorageProviderAdapter` and is the fourth
  entry in `StorageProviderRegistry::production()`. Key `managed`, config
  version 1.
- Settings: `service_origin` (fixed default, editable only in development
  builds), `account_login` (display). Secret: `account_token`, stored in the
  `SecretMap` of `sync_config.json` like Dropbox's refresh token;
  `managed_secret_id = Some(account_token)` so the locked settings page
  offers Reconnect instead of editing.
- `StorageLocalSetup` gains a `ManagedCloud` variant in Rust and in
  `src/types.ts`, and the links workspace gets a third lane branch beside
  `wrangler` and `dropbox`.
- The lane, `ManagedCloudSetup.tsx`, follows the Dropbox lane (section 3.6):
  step 1 shows the sign-in link with Copy link and Open browser and a 180 s
  countdown; step 2 shows the GitHub login with Continue and Change login;
  step 3 names the storage (default "Mallard Cloud") and saves. No folder,
  bucket, or discovery step. A second managed storage for the same account is
  refused with `storage_managed_account_exists`.
- Sign-in reuses the loopback receiver of section 3.6, generalized from
  `dropbox_browser.rs` into `loopback_browser.rs` with the callback path as
  a parameter. Port 53682 stays fixed; one browser flow runs at a time. Tauri
  commands: `authorize_managed_cloud_setup`, `cancel_managed_cloud_setup`,
  registered in `src-tauri/src/lib.rs` beside the Dropbox block.

Module layout:

```text
crates/mallard-infrastructure/src/managed_cloud/
  mod.rs        ManagedCloudAdapter, descriptor, config types
  client.rs     HTTP client: origin, bearer token, typed routes, error mapping, ConnectionHealth
  store.rs      ManagedObjectStore: key mapping, staged upload, head synthesis
  views.rs      session view builder
  catalog.rs    item push and pull adapter (step 3)
crates/mallard-infrastructure/src/loopback_browser.rs   generalized from dropbox_browser.rs
src-tauri/src/project_sync/managed_cloud_setup.rs      Tauri commands, Control state, Progress, Failure
src/components/project-sync/ManagedCloudSetup.tsx      hook + lane
```

### 5.7 Desktop screens

Push and Pull are the existing screens. The project path bar carries one
Mallard Cloud chip beside Push and Pull (docs/PLAN_CLOUD_STATUS_CHIP.md). It
shows whatever the status answered, read when the pair opens, on window
focus, and after every Push or Pull ends. That covers the release and the
draft (`Draft`, `Version N`, `Public vN`, `Private`, `In review`, `Rejected`,
`Not pushed`), a repository gone or being deleted, a sign-in needed, a
refused account, and a service that cannot be reached. The chip opens a
panel with the repository's facts and verbs:

- **Request public release**, calling
  `request_managed_cloud_release(local_project_id, storage_id,
  expected_repository_revision)`, shown only when the service would accept
  it; otherwise **View release request** once a request waits or was turned
  down. Either one copies the Release tab's address (`?tab=release`).
- **View draft** and **View published version**: external links to the
  repository's address `{origin}/{owner_login}/{name}` (section 4.1) with
  `?version=private` or `?version=public` and no token. Each copies its
  address from a button at the row's end.

Members, invitations, approvals, removal, and Super Admin live on the
website only. Discard lives on the website's Versions tab and in the desktop
chip's panel, for the Owner and Admins on every repository, on the service's
`can_discard_draft` (**Discard draft…**, confirmed in the system's dialog on
the desktop and in place on the website; `PLAN_DELETE_DRAFT.md`). The
desktop has no account UI in Settings; the account is the storage.

### 5.8 Implementation notes (2026-09-14)

The desktop side of this section shipped as
`MANAGED_CLOUD_DESKTOP_WIRING_PLAN.md` describes, with these deltas:

- Provider key `managed_cloud`, display name "Mallard Cloud", first in the
  picker (`display_rank` 5). Settings `service_origin`, `github_user_id`,
  `account_login`; one secret `account_token`, declared as the managed
  secret. The descriptor declares no manual fields: the lane is the only
  way in, and a draft is ready only once the lane produced the token.
  `github_user_id` is the verified numeric GitHub ID as a decimal string.
  The service address and GitHub ID identify one local storage; login names
  are display data. Reconnect may replace the token and refresh the login,
  including when Cloud has recreated its internal user ID. Schema versions
  stay at 1. Old `account_id` settings are not migrated or accepted.
- Release builds accept the production origin only; development builds may
  point at a local Worker or staging through `MALLARD_MANAGED_CLOUD_ORIGIN`.
- The loopback receiver is shared: `loopback_browser.rs` serves both the
  Dropbox and the Mallard Cloud callbacks; the Mallard Cloud sign-in binds
  an ephemeral port, starts the attempt with it, and exchanges the grant.
- Views are built inside the store from the wire bytes of every transcript
  in the staged upload (`managed_cloud/views.rs`), in a private temporary
  provider home with an empty project root, so cloud views carry no
  checkpoint rows and no child tail facts. Unchanged transcripts get no
  view; the service carries the previous one forward.
- The store never touches another account's open uploads. It ends an own
  upload of the same manifest before creating a new one and its own stale
  uploads after. A commit refused with `head_changed` is reported as a CAS
  conflict; the upload stays open until the next push replaces it or it
  expires.
- Tag reads and the tag listing come from one repository listing reused for
  a minute; tag writes are no-ops.
- Tests run against an in-process stub of the Worker
  (`managed_cloud/stub.rs`, compiled with `test-support`) with the same guard
  semantics; the engine parity test lives beside the S3 one.
- Catalog items (section 5.5) are not implemented: Library reads answer
  absent and Library writes are refused with
  `managed_catalog_uses_item_adapter`.

## 6. Service design

### 6.1 Layout and configuration

```text
services/managed-cloud/          # implemented at mallardInternal/cloudflare/managed-cloud
  wrangler.toml
  package.json                 # workers-types, wrangler, vitest, miniflare
  migrations/0001_init.sql     # section 6.3
  src/
    index.ts                   # fetch + scheduled entry; route table; asset fallthrough
    env.ts                     # bindings and secrets
    auth/{github.ts, sessions.ts, desktop.ts, web.ts}
    db/{guards.ts, tx.ts, queries.ts}
    routes/{account.ts, public.ts, repositories.ts, members.ts, invitations.ts,
            content.ts, uploads.ts, releases.ts, removal.ts, super_admin.ts}
    validate/{manifest.ts, catalog.ts, view.ts, limits.ts}
    jobs/{expire.ts, cleanup.ts, removal.ts}
    errors.ts
    audit.ts
  fixtures/                    # generated from Rust (section 10.5)
  test/                        # vitest against miniflare D1 and R2
  web/                         # React website (section 8), built into dist/
```

```toml
name = "mallard-managed-cloud"
main = "src/index.ts"
compatibility_date = "2026-09-01"

[assets]
directory = "./web/dist"
binding = "ASSETS"
not_found_handling = "single-page-application"
run_worker_first = true          # the Worker redirects www. and sets the page CSP

[[d1_databases]]
binding = "DB"
database_name = "mallard-managed-cloud"
database_id = "<set per environment>"

[[r2_buckets]]
binding = "OBJECTS"
bucket_name = "mallard-managed-cloud"

[triggers]
crons = ["*/5 * * * *"]

[vars]
SERVICE_ORIGIN = "https://cloud.mallard.example"
GITHUB_CLIENT_ID = "<public>"
# secrets via `wrangler secret put`: GITHUB_CLIENT_SECRET, TOKEN_PEPPER
```

Development and tests use separate databases, buckets, and a separate GitHub
OAuth app. The Workers Paid plan is required for the cron CPU budget and the
D1 size limit. Zone rate limiting rules protect `/v1/auth/*`,
`/v1/invitations/accept`, and `/v1/repositories/*/uploads` per IP and per
account; the Worker has no rate-limit code.

Platform facts the design relies on: a D1 `batch()` runs its statements
sequentially in one transaction and rolls the whole batch back when any
statement errors; D1 allows 100 bound parameters per statement, 2 MB per
row, 30 seconds per query; R2 `put` accepts a `sha256` option and rejects a
mismatch; the request body limit is 100 MB on the Free and Pro zone plans;
a Worker isolate has 128 MB; a cron invocation has 15 minutes of wall
clock; static assets are served after the Worker for paths listed in
`run_worker_first`.

### 6.2 R2 keys

```text
repos/<repository_id>/objects/<sha256>/<upload_id>
repos/<repository_id>/manifests/<commit_id>.json
removals/<repository_id>/<removal_id>.json
```

Every write is create-only because every key embeds a never-reused ID. The
Worker builds keys from validated IDs; clients never send keys. Public and
private publications reference the same keys. A listing is used only by the
removal job to verify that a repository prefix is empty.

### 6.3 D1 schema

The schema of record is the service's `migrations/0001_init.sql`; the listing
below is the September draft. Since 2026-10-05 (`PLAN_CLOUD_VERSIONS.md`):
`publications.scope` is `private` or `version` (a version row carries its
number in `revision`, the push it froze in `source_publication_id`, and its
own `name` and `note`; one live version per push); `repositories` holds
`version_count` and `release_count` in place of `public_revision`, and
`public_publication_id` names a version row; a `release_requests` row names
exactly one of `version_publication_id` and `push_publication_id`;
`removals.target` is `repository`, `version` or `chain`.

```sql
-- One column per API error code, each with a NAMED check constraint, so the
-- SQLite message of a failed guard names the code ("CHECK constraint failed:
-- revision_conflict") and the Worker maps it without a re-read.
CREATE TABLE guard_check (
  revision_conflict INTEGER CONSTRAINT revision_conflict CHECK (revision_conflict IS NULL),
  head_changed      INTEGER CONSTRAINT head_changed      CHECK (head_changed IS NULL)
  -- … one such column for every code in section 7.1
);

CREATE TABLE users (
  id            TEXT PRIMARY KEY,                 -- usr_<random>
  github_id     INTEGER NOT NULL UNIQUE,
  github_login  TEXT NOT NULL,                     -- the {owner} address segment; refreshed at sign-in
  state         TEXT NOT NULL CHECK (state IN ('active','suspended')),
  service_role  TEXT NOT NULL DEFAULT 'user' CHECK (service_role IN ('user','super_admin')),
  created_at    INTEGER NOT NULL,
  last_sign_in_at INTEGER NOT NULL
);
-- Address lookups compare logins case-insensitively; the most recently
-- signed-in row holds a login GitHub has handed to another account.
CREATE INDEX users_login ON users(github_login COLLATE NOCASE);

CREATE TABLE sessions (
  id          TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users(id),
  client      TEXT NOT NULL CHECK (client IN ('desktop','web')),
  token_hash  TEXT NOT NULL UNIQUE,                -- sha256(pepper || token)
  expires_at  INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  revoked_at  INTEGER
);

CREATE TABLE auth_attempts (
  state           TEXT PRIMARY KEY,
  client          TEXT NOT NULL CHECK (client IN ('desktop','web')),
  code_challenge  TEXT,                            -- desktop: S256 challenge
  loopback_port   INTEGER,                         -- desktop
  return_path     TEXT,                            -- web: must start with '/'
  grant_hash      TEXT,                            -- desktop: set at callback, one use
  user_id         TEXT,
  expires_at      INTEGER NOT NULL
);

CREATE TABLE repositories (
  id                     TEXT PRIMARY KEY,         -- repo_<random>
  class                  TEXT NOT NULL CHECK (class IN ('git_project','non_git_project','catalog')),
  name                   TEXT NOT NULL,             -- the address is {owner login}/{name}: letters, marks and digits of any script plus '._-', 1 to 100 code points, stored in NFC (validate/names.ts)
  -- `nameKey(name)`: NFC and lower-cased by the Worker, since SQLite's NOCASE folds ASCII only. One per Owner among active repositories.
  name_key               TEXT NOT NULL,
  bundle_id              TEXT,
  repository_fingerprint TEXT,
  catalog_provider       TEXT, catalog_resource_kind TEXT, catalog_format TEXT,
  catalog_format_version INTEGER, catalog_resource_id TEXT,
  revision               INTEGER NOT NULL DEFAULT 1,
  state                  TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','deleting','removed')),
  visibility             TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','public')),
  private_publication_id TEXT,
  private_revision       INTEGER NOT NULL DEFAULT 0,
  public_publication_id  TEXT,
  public_revision        INTEGER NOT NULL DEFAULT 0,
  public_admitted_at     INTEGER, -- current Super Admin approval; null requires approval before publishing
  committed_bytes        INTEGER NOT NULL DEFAULT 0 CHECK (committed_bytes >= 0),
  star_count             INTEGER NOT NULL DEFAULT 0 CHECK (star_count >= 0),  -- migration 0002
  -- The repository's one invitation code (section 4.3), normalized (20 characters,
  -- no separators) so the Owner and Admins can read it again; who set it, and when;
  -- rotating replaces it, revoking clears all three.
  invite_code            TEXT UNIQUE,
  invite_code_set_by     TEXT REFERENCES users(id),
  invite_code_set_at     INTEGER,
  created_by             TEXT NOT NULL REFERENCES users(id), -- the Owner; ownership never transfers, so this is the address namespace
  created_at             INTEGER NOT NULL,
  updated_at             INTEGER NOT NULL,
  CHECK ((invite_code IS NULL) = (invite_code_set_by IS NULL) AND (invite_code IS NULL) = (invite_code_set_at IS NULL)),
  CHECK (
    (class = 'git_project'     AND bundle_id IS NOT NULL AND repository_fingerprint IS NOT NULL AND catalog_resource_id IS NULL) OR
    (class = 'non_git_project' AND bundle_id IS NOT NULL AND repository_fingerprint IS NULL     AND catalog_resource_id IS NULL) OR
    (class = 'catalog'         AND bundle_id IS NULL     AND catalog_resource_id IS NOT NULL)
  )
);
-- One name per Owner among active repositories, compared by key (NFC, no
-- case). A repository frees its name when its removal is confirmed.
CREATE UNIQUE INDEX repositories_owner_name ON repositories(created_by, name_key) WHERE state = 'active';
CREATE INDEX repositories_bundle ON repositories(bundle_id) WHERE bundle_id IS NOT NULL;

CREATE TABLE repository_members (
  repository_id TEXT NOT NULL REFERENCES repositories(id),
  user_id       TEXT NOT NULL REFERENCES users(id),
  role          TEXT NOT NULL CHECK (role IN ('owner','admin','member')),
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (repository_id, user_id)
);
CREATE UNIQUE INDEX repository_single_owner ON repository_members(repository_id) WHERE role = 'owner';

-- Migration 0002 (section 4.8). A star is a vote: every Star is +1 to
-- `star_count`, 24 per voter per repository per UTC day. An account's stars on
-- a repository are one row; `listed` puts it on the Starred list.
CREATE TABLE repository_stars (
  repository_id TEXT NOT NULL REFERENCES repositories(id),
  user_id       TEXT NOT NULL REFERENCES users(id),
  listed        INTEGER NOT NULL CHECK (listed IN (0, 1)),
  day           INTEGER NOT NULL,                 -- the UTC day day_stars counts
  day_stars     INTEGER NOT NULL CHECK (day_stars >= 0),
  starred_at    INTEGER NOT NULL,                 -- latest Star; orders the Starred list
  created_at    INTEGER NOT NULL,
  PRIMARY KEY (repository_id, user_id)
);
CREATE INDEX repository_stars_listed ON repository_stars(user_id, starred_at, repository_id) WHERE listed = 1;
-- Signed out, a connection (the IP hashed with the day's secret) is the voter;
-- the scheduled job deletes earlier days' tallies and secrets.
CREATE TABLE guest_stars (
  repository_id TEXT NOT NULL REFERENCES repositories(id),
  connection    TEXT NOT NULL,
  day           INTEGER NOT NULL,
  stars         INTEGER NOT NULL CHECK (stars >= 0),
  PRIMARY KEY (repository_id, connection)
);
CREATE INDEX guest_stars_day ON guest_stars(day);
CREATE TABLE star_secrets (
  day    INTEGER PRIMARY KEY,
  secret TEXT NOT NULL
);

CREATE TABLE objects (
  id            TEXT PRIMARY KEY,                  -- obj_<random>
  repository_id TEXT NOT NULL REFERENCES repositories(id),
  sha256        TEXT NOT NULL,
  size          INTEGER NOT NULL CHECK (size >= 0),
  key           TEXT NOT NULL UNIQUE,
  upload_id     TEXT NOT NULL,
  state         TEXT NOT NULL CHECK (state IN ('writing','ready','deleting','deleted')),
  created_at    INTEGER NOT NULL,
  claimed_at    INTEGER,
  UNIQUE (repository_id, sha256, upload_id)
);
CREATE INDEX objects_ready ON objects(repository_id, sha256) WHERE state = 'ready';

CREATE TABLE uploads (
  id                           TEXT PRIMARY KEY,   -- upl_<random>
  repository_id                TEXT NOT NULL REFERENCES repositories(id),
  user_id                      TEXT NOT NULL REFERENCES users(id),
  state                        TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','committed','expired','abandoned')),
  expected_repository_revision INTEGER NOT NULL,
  expected_private_revision    INTEGER NOT NULL,
  commit_id                    TEXT NOT NULL,      -- from the manifest
  manifest_key                 TEXT NOT NULL,
  manifest_sha256              TEXT NOT NULL,
  manifest_size                INTEGER NOT NULL,
  summary_json                 TEXT NOT NULL,      -- tag summary computed at creation
  reserved_bytes               INTEGER NOT NULL CHECK (reserved_bytes >= 0),
  expires_at                   INTEGER NOT NULL,
  created_at                   INTEGER NOT NULL,
  closed_at                    INTEGER
);
-- Uploads run side by side; the commit batch is the only compare-and-swap.
CREATE INDEX uploads_open ON uploads(repository_id) WHERE state = 'open';
-- A commit id is claimed by an open or committed upload only; an abandoned or
-- expired upload may be retried with the same manifest.
CREATE UNIQUE INDEX uploads_commit_claimed ON uploads(repository_id, commit_id)
  WHERE state IN ('open','committed');

CREATE TABLE upload_objects (
  upload_id     TEXT NOT NULL REFERENCES uploads(id),
  sha256        TEXT NOT NULL,
  size          INTEGER NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('payload','view')),
  resource_id   TEXT,                              -- view: conversation resource
  source_sha256 TEXT,                              -- view: transcript digest
  object_id     TEXT REFERENCES objects(id),       -- set when matched or written
  PRIMARY KEY (upload_id, sha256)
);

CREATE TABLE publications (
  id              TEXT PRIMARY KEY,                -- pub_<random>
  repository_id   TEXT NOT NULL REFERENCES repositories(id),
  scope           TEXT NOT NULL CHECK (scope IN ('private','public')),
  revision        INTEGER NOT NULL,                -- private_revision or public_revision
  commit_id       TEXT NOT NULL,
  manifest_key    TEXT NOT NULL,
  manifest_sha256 TEXT NOT NULL,
  manifest_size   INTEGER NOT NULL,
  manifest_revision INTEGER NOT NULL,              -- the manifest's own revision field
  summary_json    TEXT NOT NULL,
  state           TEXT NOT NULL DEFAULT 'available' CHECK (state IN ('available','retired','removed')),
  source_publication_id TEXT,                      -- public: released private publication
  released_by     TEXT, owner_approved_by TEXT, super_admin_approved_by TEXT,
  created_by      TEXT NOT NULL REFERENCES users(id),
  created_at      INTEGER NOT NULL,
  retired_at      INTEGER,
  UNIQUE (repository_id, scope, revision)
);
CREATE INDEX publications_commit ON publications(repository_id, commit_id);

CREATE TABLE publication_objects (
  publication_id TEXT NOT NULL REFERENCES publications(id),
  object_id      TEXT NOT NULL REFERENCES objects(id),
  role           TEXT NOT NULL CHECK (role IN ('payload','view')),
  resource_id    TEXT,
  source_sha256  TEXT,
  PRIMARY KEY (publication_id, object_id)
);
CREATE INDEX publication_objects_object ON publication_objects(object_id);

CREATE TABLE release_requests (
  id                           TEXT PRIMARY KEY,
  repository_id                TEXT NOT NULL REFERENCES repositories(id),
  private_publication_id       TEXT NOT NULL REFERENCES publications(id),
  expected_public_publication_id TEXT,
  expected_repository_revision INTEGER NOT NULL,
  state                        TEXT NOT NULL CHECK (state IN ('pending_owner','pending_super_admin','published','rejected','withdrawn','invalidated')),
  requested_by                 TEXT NOT NULL REFERENCES users(id),
  owner_decided_by             TEXT, owner_decided_at INTEGER,
  super_admin_decided_by       TEXT, super_admin_decided_at INTEGER,
  published_publication_id     TEXT,
  reason                       TEXT,
  created_at                   INTEGER NOT NULL,
  closed_at                    INTEGER
);
CREATE UNIQUE INDEX release_requests_one_open ON release_requests(repository_id)
  WHERE state IN ('pending_owner','pending_super_admin');

CREATE TABLE removals (
  id             TEXT PRIMARY KEY,
  repository_id  TEXT NOT NULL,
  target         TEXT NOT NULL CHECK (target IN ('repository','revision')),
  publication_id TEXT,
  authority      TEXT NOT NULL CHECK (authority IN ('owner_admin','super_admin')),
  requested_by   TEXT NOT NULL,
  reason         TEXT,
  state          TEXT NOT NULL DEFAULT 'deleting' CHECK (state IN ('deleting','done','failed')),
  record_written INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  completed_at   INTEGER,
  last_error     TEXT
);

CREATE TABLE idempotency (
  user_id       TEXT NOT NULL,
  operation     TEXT NOT NULL,
  key_hash      TEXT NOT NULL,
  request_hash  TEXT NOT NULL,
  status        INTEGER NOT NULL,
  response_json TEXT NOT NULL,
  expires_at    INTEGER NOT NULL,
  PRIMARY KEY (user_id, operation, key_hash)
);

CREATE TABLE audit_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  at            INTEGER NOT NULL,
  actor_user_id TEXT,
  repository_id TEXT,
  action        TEXT NOT NULL,
  target        TEXT,
  result        TEXT NOT NULL,
  reason        TEXT,
  request_id    TEXT
);
```

Manifests, views, and payloads never enter D1 (the row limit is 2 MB).
Timestamps are integer Unix seconds, as in the Rust DTOs.

### 6.4 Guard idiom and transactions

D1 `batch()` runs its statements in one transaction and rolls back the whole
batch when any statement fails, but an `UPDATE` that matches zero rows is not
a failure. Every guarded batch therefore starts with assertion statements of
this form:

```sql
INSERT INTO guard_check(revision_conflict)
SELECT 1 WHERE NOT EXISTS (
  SELECT 1 FROM repositories
  WHERE id = ?1 AND state = 'active' AND revision = ?2 AND private_revision = ?3
);
```

A failed condition inserts a non-null value, violates the named `CHECK`, and
aborts the batch with a message that names the column, which is the error
code the route answers; a passed condition inserts nothing. Guards read transaction-time state. No
R2 I/O or JSON parsing happens inside a batch. Copies between tables use
`INSERT … SELECT` so the 100-parameter limit never applies to row counts.

| Operation | Guards | Writes |
| --- | --- | --- |
| Create repository | actor active; identity unused among the account's active repositories; active repository count under limit; `name` = the requested name normalized, suffixed past the names of the Owner's active repositories (the unique index `repositories_owner_name` settles a race as `repository_name_taken`, retried once) | repositories, owner membership, audit |
| Create upload | writer (section 4.3: a member, or anyone on a public repository with a public head); repository active and revision matches; `private_revision` matches; fewer than 64 of the pusher's own uploads open; `commit_id` unused; quota admits `reserved_bytes` on top of every open upload's reservation | uploads, upload_objects (matched `object_id` for ready digests), audit |
| Register object write | upload open and not expired; digest listed; not yet ready | objects (`writing`), upload_objects.object_id |
| Mark object ready | object `writing`; size matches | objects (`ready`) |
| Commit upload | upload open and not expired; every upload_object has a `ready` object; repository active, revision and `private_revision` unchanged; `expected_commit_id` equals the current private commit or both are null; actor still a writer | publications (private, `private_revision + 1`); publication_objects from upload_objects plus carried-forward views; previous private publication → `retired`; repositories head and `private_revision`; open release request → `invalidated`; `committed_bytes += bytes of objects entering a publication for the first time`; uploads → `committed`; idempotency; audit |
| Abandon or expire upload | upload open | uploads → `abandoned` or `expired` |
| Discard private snapshot | Owner or Admin; repository active and revision matches; there is a draft to discard | removals (target `chain`, the kept publication and the first removed record), revision + 1, open request → `invalidated`, audit; then, in the same request, the removal: the rows of the removed records → `removed`, head at the kept publication or cleared |
| Set or rotate the invitation code | Owner or Admin; revision matches | repositories `invite_code_*`, revision + 1, audit |
| Revoke the invitation code | Owner or Admin; revision matches (a repository without a code answers ok too) | repositories `invite_code_*` cleared, revision + 1, audit |
| Enter the invitation code | actor active; the code is the repository's current one; repository active; members under 50 or already a member | membership (Member; an existing collaborator is unchanged and the batch is skipped), revision + 1, open request → `invalidated`, audit |
| Change role, remove member, leave | authority per section 4.3; Owner row untouched; revision matches | membership, revision + 1, open request → `invalidated`, audit |
| Rename, make private | Owner or Admin; revision matches; rename: `name` well-formed and held by no other repository of the Owner's in any spelling (`repository_name_taken`) | repositories, revision + 1, open request → `invalidated` (make private only), audit |
| Request release | writer; repository active; private head present; revision matches; no open request; while public, the private publication is not already the public head's source | release_requests (`pending_owner`), idempotency, audit |
| Owner decision | Owner; request `pending_owner`; pins unchanged | reject → `rejected`; approve and admission absent → `pending_super_admin`; approve otherwise → publish |
| Super Admin decision | `super_admin`; request `pending_super_admin`; pins unchanged | reject → `rejected`; approve → publish and `public_admitted_at` |
| Publish (inside a decision) | pins unchanged; private publication available; admission matches the decision authority | publications (public, `public_revision + 1`, same manifest, `source_publication_id`); publication_objects copied from the private publication; repositories public head, `public_revision`, `visibility = 'public'`, revision + 1; request → `published`; audit |
| Super Admin Make private | service role active; repository active and public; revision matches; reason required | visibility private; admission cleared; revision + 1; open request → `invalidated`; `super_admin.make_private` audit |
| Restore retained public head (inside a decision) | publish guards; visibility private; public head available and its source is the pinned private publication | visibility public; repository revision + 1; request → `published` with the retained publication id; `release.restore` audit; public revision unchanged |
| Withdraw request | requester or Owner; request open | request → `withdrawn`, audit |
| Remove repository | Owner, Admin, or Super Admin; name confirmed; revision matches | repositories → `deleting`, private, invitation code cleared; requests → `invalidated`; open uploads → `abandoned`; removals; audit |
| Remove public revision | Super Admin; publication public and available | publication → `removed`; its publication_objects deleted; if head: head cleared and `visibility = 'private'`; revision + 1; removals (`revision`); audit |
| Claim object for deletion | object `ready`; no publication_objects row from an `available` publication; no upload_objects row from an `open` upload | objects → `deleting`, `claimed_at` |
| Finish deletion | object `deleting` | objects → `deleted`; `committed_bytes -= size` when the object was ever published |

Every guarded mutation returns `revision_conflict` when a revision guard
fails, `head_changed` when a commit-ID guard fails, and the specific code
otherwise.

### 6.5 Upload protocol and limits

1. `POST /v1/repositories/{id}/uploads` as `multipart/form-data` with a
   `request` part (the JSON inventory below) and a `manifest` part holding the
   manifest bytes exactly as the desktop serialized them; the service stores
   those bytes unchanged because the engine later stages fetched manifests
   against the digest it computed locally. The Worker validates the manifest (section 10), checks that the
   payload inventory equals the set of object references in the manifest,
   checks every view's `resource_id` against the manifest's conversations,
   computes the manifest digest and the tag summary, writes
   `manifests/<commit_id>.json` to R2, then runs the create-upload batch. It
   replies with `upload_id`, `expires_at`, and `missing`, the digests with no
   ready object.
2. `PUT /v1/repositories/{id}/uploads/{upload}/objects/{sha256}` with
   `Content-Length` and `X-Mallard-Sha256`. The Worker registers the write,
   streams the body to R2 with `put(key, body, { sha256 })` so R2 rejects a
   digest mismatch, checks the stored size, and marks the object ready. A
   failed write leaves the row in `writing` for cleanup and the client
   retries under the same digest; the retry gets a new object key.
3. `POST /v1/repositories/{id}/uploads/{upload}/commit` with
   `expected_commit_id`. The commit batch runs; the response carries the new
   private head.

| Limit | Value |
| --- | --- |
| Object size | 90 MiB; larger is refused at upload creation with `object_too_large` |
| Manifest size | 8 MiB |
| Session view size | 8 MiB or 20,000 events, then `truncated` |
| Objects per upload | 20,000 |
| Repository quota | 4 GiB committed plus reserved |
| Repositories per account | 100 |
| Members per repository | 50 |
| Upload lifetime | 24 hours |

The 90 MiB object cap sits under the 100 MB request body limit of the Free
and Pro zone plans. Step 1 of section 12 verifies a 90 MiB upload on the
deployed plan before anything else is built. A conversation the desktop
allows locally but the service refuses fails the push with a clear item
error; nothing is truncated.

### 6.6 Reads and authorization

Every read resolves the caller: a desktop bearer token or a web session
cookie, else anonymous. Then:

- A writer (section 4.3: a collaborator, or any signed-in account on a
  public repository with a public head) may read the private head, every
  `available` public publication, retired private manifests, and any `ready`
  object or view of the repository. The summary carries `writer: true` for
  them, and `GET …/by-name/{owner}/{name}` answers the summary rather than
  the public face.
- Super Admin may read the same, audited.
- An anonymous caller may read only a public repository's current public
  head, its manifest, and the objects and views that publication references.
  `commit_id` in the request must equal the current public commit, else
  `not_found`. Responses carry `Cache-Control: no-store`.
- Missing and hidden repositories both answer `not_found`; a private
  repository is hidden from every non-collaborator.

Objects stream from R2 with `Content-Length` and the stored SHA-256 in
`X-Mallard-Sha256`; the desktop verifies both.

### 6.7 Cleanup, expiry, and removal worker

The cron Worker runs every five minutes and derives its work from state
columns; there is no job table. Each run is bounded and every step is safe to
repeat:

1. Expire `open` uploads past `expires_at`, and `auth_attempts`, `sessions`,
   and `idempotency` rows.
2. Delete `writing` objects whose upload is no longer `open` (key delete,
   then `deleted`).
3. Claim up to 200 unreferenced `ready` objects (section 6.4), delete each
   key, and finish the deletion. Uncertainty keeps the claim for the next
   run. An object referenced again before the claim cannot be claimed; an
   object claimed before a new upload lists it is reported as missing and
   re-uploaded under a new key.
4. For each `removals` row in `deleting`: write `removals/<repo>/<id>.json`
   if `record_written = 0`; delete every object and manifest key of the
   repository (repository target) or the objects that became unreferenced
   (revision target); list the repository prefix to verify it is empty
   (repository target); mark objects `deleted`, publications `removed`,
   the repository `removed` with `committed_bytes = 0`, and the removal
   `done`. A failing step records `last_error` and stays `deleting`.
5. Erase retired private manifests beyond the newest 32 per repository.

The worker stays within the 15-minute cron budget by taking at most 500
R2 operations per run. An operator can trigger it with
`curl "$ORIGIN/cdn-cgi/local/scheduled"` in development and with a
protected `POST /v1/super-admin/jobs/run` in production.

### 6.8 Idempotency

`Idempotency-Key` is required on create repository, create upload, commit
upload, and request release. The Worker stores a hash of
the key and of the canonical request body for 24 hours and replays the
recorded response for the same pair; a different body under the same key
answers `idempotency_mismatch`. Every other mutation is protected by its
revision or commit guard, and a retry that lost its response re-reads state.

## 7. API

### 7.1 Conventions and errors

Base path `/v1`, JSON `snake_case`, integer seconds. Desktop requests send
`Authorization: Bearer <token>`. Website requests send the session cookie
plus `X-Requested-With: mallard-web`; the Worker rejects a browser mutation
whose `Origin` is not the service origin. Every mutation that can race
carries `expected_repository_revision`.

Error body: `{ "error": "<code>", "detail": "<bounded text>", "request_id": "…" }`.

Codes: `authentication_required`, `forbidden`, `not_found`,
`invalid_request`, `manifest_invalid`, `revision_conflict`, `head_changed`,
`upload_in_progress`, `upload_expired`, `object_too_large`,
`object_integrity_failed`, `quota_exceeded`, `release_request_pending`,
`release_request_invalid`, `owner_required`, `super_admin_required`,
`invitation_invalid`, `idempotency_mismatch`, `repository_removed`,
`storage_managed_account_exists`, `star_limit_reached`,
`temporarily_unavailable`.
`manifest_invalid` carries the specific reason in `detail` (class mismatch,
identity mismatch, unregistered coordinate, missing reference, bound
exceeded). Errors never reveal private existence, content, or local paths.

### 7.2 Auth

| Method | Route | Purpose |
| --- | --- | --- |
| `POST` | `/v1/auth/desktop/start` | `{code_challenge, loopback_port}` → `{state, authorize_url}` |
| `GET` | `/v1/auth/github/callback` | GitHub redirect target for both clients; exchanges the code with the client secret, reads `/user`, upserts the account |
| `POST` | `/v1/auth/desktop/exchange` | `{state, grant, code_verifier}` → `{token, expires_at, user}` |
| `GET` | `/auth/github/start?return=/path` | Website sign-in; sets the state cookie and redirects to GitHub |
| `POST` | `/v1/auth/logout` | Revokes the current session |
| `GET` | `/v1/me` | Account, service role, limits |

Desktop flow: the lane calls `start`, shows `authorize_url`, and listens on
the loopback port. GitHub redirects the browser to the Worker callback; the
Worker verifies `state`, exchanges the code, stores a one-use grant hash on
the attempt, and redirects the browser to
`http://127.0.0.1:{loopback_port}/mallard/callback?state=…&grant=…`. The
lane posts `exchange` with the verifier; the Worker checks the S256
challenge, deletes the attempt, and issues an opaque token valid for 90 days.
Website flow: the callback sets the session cookie and redirects to
`return`, which must be a relative path.

GitHub's web application flow: the authorize URL is
`https://github.com/login/oauth/authorize` with `client_id`,
`redirect_uri`, `scope`, and `state`; the code exchange is a `POST` to
`https://github.com/login/oauth/access_token` with `client_id`,
`client_secret`, `code`, and `redirect_uri`; the account is read from
`GET https://api.github.com/user` (`id`, `login`). GitHub scope is
`read:user` only. The GitHub access token is used for that one call and
discarded.

### 7.3 Account and public routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/v1/repositories?class=` | Joined repositories with both head summaries, role, usage, release status |
| `GET` | `/v1/repositories/by-bundle/{bundle_id}/head` | The head the caller may read for that bundle, or `204` when unknown |
| `GET` | `/v1/repositories/by-name/{owner}/{name}` | The repository at that address (section 4.1): the collaborator's summary, else the public face, else `404`; the name arrives percent-encoded or raw, and both segments compare without case (the name by key) |
| `GET` | `/v1/users/{login}` | Profile: `{user: {user_id, github_login, avatar_url, created_at}, repositories}`, the repositories the account owns that the caller may read, rows as in the Starred listing |
| `GET` | `/v1/me/starred?cursor=` | The caller's Starred list, latest star first, 50 per page; rows carry `owner`, `name`, `visibility`, `starred_at`, and the readable `head` (section 4.8) |
| `GET` | `/v1/public/repositories?class=&provider=&kind=&cursor=` | Bounded public listing, 50 per page |
| `GET` | `/v1/public/repositories/{id}` | Public summary: class, `owner`, `name`, public head, counts |
| `GET` | `/v1/public/repositories/{id}/manifest` | Current public manifest |
| `GET` | `/v1/public/repositories/{id}/views/{resource_id}?commit_id=` | Session view |
| `GET` | `/v1/public/repositories/{id}/objects/{sha256}?commit_id=` | Payload referenced by the public head |

Head body (both head routes):

```json
{ "scope": "private", "revision": 12, "commit_id": "commit-…",
  "manifest_sha256": "…", "manifest_size": 48213, "updated_at": 1757700000,
  "kind": "project", "bundle_id": "…",
  "pushed_by": { "user_id": "usr_…", "github_login": "octocat",
                 "avatar_url": "https://avatars.githubusercontent.com/u/583231?v=4" } }
```

`pushed_by` is the collaborator whose push created the snapshot: the private
publication's `created_by`, followed through `source_publication_id` for a
version, so a version credits the pusher rather than whoever published or
approved it. The same object sits on every row of the public listing; a row
of `/versions` carries `published_by`, who froze it. The
avatar URL is built from the GitHub account id (stable across renames); the
website adds `s=` for the size it renders. It is `null` only when the user
row cannot be joined.

### 7.4 Repositories, members, the invitation code

| Method | Route | Purpose |
| --- | --- | --- |
| `POST` | `/v1/repositories` | Create; body below |
| `GET`, `PATCH` | `/v1/repositories/{id}` | Summary with `owner`, `name`, both heads, revisions, role, usage, `star_count`, `member_count`, open request; rename `{name, expected_repository_revision}` (strict; the address moves, nothing redirects) |
| `POST` | `/v1/repositories/{id}/visibility` | `{visibility: "private", expected_repository_revision}` |
| `POST`, `DELETE` | `/v1/repositories/{id}/star` | `POST` gives one star, signed in or not (24 per voter per repository per UTC day, then `star_limit_reached`); signed in, it also puts the repository on the Starred list, and `DELETE` takes it off without touching the count (section 4.8); no Idempotency-Key; → `{repository_id, starred, star_count}` |
| `GET` | `/v1/repositories/{id}/members` | Collaborators and Super Admin; `invitation: {code?, set_by, set_by_login, set_at} \| null` says whether a code stands; `code` only for the Owner and Admins |
| `PATCH`, `DELETE` | `/v1/repositories/{id}/members/{user_id}` | Change role; remove; a non-Owner deletes their own row to leave |
| `PUT` | `/v1/repositories/{id}/invitation` | Owner or Admin; `{expected_repository_revision}` → `{code, set_by, set_by_login, set_at, repository_revision}`; creates or rotates the repository's one code |
| `DELETE` | `/v1/repositories/{id}/invitation` | Owner or Admin; `expected_repository_revision` in the body or query → `{ok, repository_revision}` |
| `POST` | `/v1/invitations/accept` | `{code}` → repository summary; joins as Member, idempotent, no Idempotency-Key |

Create bodies (`name` is the requested name; the stored one is normalized as
section 4.1 says, so these become `Mallard-Desktop`, `Research-Notes`, `Rika`):

```json
{ "repository_class": "git_project", "name": "Mallard Desktop",
  "bundle_id": "…", "repository_fingerprint": "sha256_hex" }
```

```json
{ "repository_class": "non_git_project", "name": "Research Notes", "bundle_id": "…" }
```

```json
{ "repository_class": "catalog", "name": "Rika",
  "provider": "codex", "resource_kind": "desktop_pet",
  "format": "openai.codex-pet", "format_version": 2, "resource_id": "resource-…" }
```

### 7.5 Content

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/v1/repositories/{id}/head` | The head the caller may read (body as in section 7.3); `204` when absent |
| `GET` | `/v1/repositories/{id}/versions` | Versions newest first, including `removed` markers, for whoever may read the draft |
| `POST` | `/v1/repositories/{id}/versions` | Publish: `{commit_id, name?, note?, expected_repository_revision}` freezes the draft the caller saw as the next version → `201 {version, commit_id, name, created_at}`; `409 version_unchanged` while the draft is a version already |
| `GET` | `/v1/repositories/{id}/manifests/{commit_id}` | Manifest bytes |
| `GET`, `PUT` | `/v1/repositories/{id}/listings/{commit_id}` | A plugin's listing (`display_name`, `developer`, `description`, `category`, `website_url`, `privacy_url`, `terms_url`; `GET` adds `description_from`): the plugin's own manifest on GitHub first, then the desktop's copy, behind the Owner's description. `PUT` is the desktop's copy of the installed manifest, sent once after a plugin push (`storage_catalog/plugin_listing.rs`); only the commit's pusher may set it, and a failure never fails the push. The public form is `GET /v1/public/repositories/{id}/listing` |
| `GET` | `/v1/repositories/{id}/objects/{sha256}` | Payload bytes for collaborators |
| `GET` | `/v1/repositories/{id}/views/{resource_id}?commit_id=` | Session view for collaborators |
| `POST` | `/v1/repositories/{id}/uploads` | Section 6.5 step 1 |
| `PUT` | `/v1/repositories/{id}/uploads/{upload}/objects/{sha256}` | Step 2 |
| `POST` | `/v1/repositories/{id}/uploads/{upload}/commit` | Step 3 |
| `POST` | `/v1/repositories/{id}/uploads/{upload}/abandon` | End an open upload |
| `POST` | `/v1/repositories/{id}/private-snapshot/discard` | `{confirm: true, expected_repository_revision}` → `{ok, repository_id, removal_id, state, kept_version}` (`STORE_API.md` section 3) |

Upload creation: `multipart/form-data`; part `manifest` carries the raw
manifest bytes, part `request` carries this JSON:

```json
{
  "expected_repository_revision": 7,
  "expected_private_revision": 3,
  "objects": [
    { "sha256": "…", "size": 1234, "role": "payload" },
    { "sha256": "…", "size": 5678, "role": "view", "resource_id": "resource-…", "source_sha256": "…" }
  ]
}
```

### 7.6 Release requests

| Method | Route | Purpose |
| --- | --- | --- |
| `GET`, `POST` | `/v1/repositories/{id}/release-requests` | Read the open request and its comparison; create `{expected_repository_revision}` with `version: n` or `draft_commit_id`, exactly one |
| `POST` | `/v1/repositories/{id}/release-requests/{request}/decision` | `{decision: "approve" or "reject" or "withdraw", commit_id, acknowledge_disclosure, reason}` |

The comparison is computed on read from the two manifests: resources present
on one side, present on both with different digests, and identical. It is a
listing, not a merge.

### 7.7 Removal and Super Admin

| Method | Route | Purpose |
| --- | --- | --- |
| `GET`, `POST` | `/v1/repositories/{id}/removal` | Preview (owner, name, revision, both heads, bytes); confirm `{confirmed_name, expected_repository_revision}` |
| `GET` | `/v1/super-admin/repositories?cursor=` | All repositories |
| `GET` | `/v1/super-admin/release-requests` | Requests in `pending_super_admin` |
| `POST` | `/v1/super-admin/release-requests/{request}/decision` | Approve or reject a release that requires admission |
| `POST` | `/v1/super-admin/repositories/{id}/visibility` | Make private and revoke admission `{visibility: "private", expected_repository_revision, reason}` |
| `POST` | `/v1/super-admin/repositories/{id}/removal` | Immediate removal `{confirmed_name, reason}` |
| `POST` | `/v1/super-admin/repositories/{id}/versions/{n}/removal` | Remove one version `{reason}` |
| `GET` | `/v1/super-admin/removals/{removal_id}` | Progress |
| `POST` | `/v1/super-admin/jobs/run` | Run the cron work now |

Super Admin reads reuse the collaborator routes with the service role and an
audit row per repository opened.

## 8. Website

The website is a React single-page app under `services/managed-cloud/web`,
served by the Worker's static assets with `run_worker_first` for `/v1/*` and
`/auth/*`. It shares components with the desktop through a Vite alias to
`src/`: `session-detail/SessionDetailTimeline.tsx`, `SessionDetailEvent.tsx`,
`SessionDetailToolCall.tsx`, `sessionDetailModel.ts`, `Icons.tsx`,
`sync-review/ProviderMark.tsx`, and `notice/Notice.tsx`. Two helpers must
gain a browser fallback first: `externalLink.ts` (uses
`@tauri-apps/plugin-opener`) and `sync-review/clipboard.ts`. The website
never imports `ipc.ts` or either `api.ts`; it has its own `client.ts` over
`fetch`.

| Route | Anonymous | Signed in |
| --- | --- | --- |
| `/` | Public listing with class, provider, and kind filters | Yours, Starred, and Public scopes over the same cards |
| `/:owner` | Profile: the account and the public repositories it owns | Plus the private ones the viewer collaborates on |
| `/:owner/:name` | Public head: session list, project files, or catalog item | Writers (section 4.3): Latest push and Public version, History, Request public release; collaborators also Members and Settings; About says "contributor" for a signed-in non-collaborator of a public repository |
| `/:owner/:name/sessions/:resource_id?commit=` | Session view | Same, for private and historical commits |
| `/:owner/:name/release` | Not available | Request, comparison, Owner decision, status |
| `/invitations` | Sign-in prompt | Enter a private repository's code |
| `/super-admin` | Not available | Queue, inspection, removals; `super_admin` only |

Addresses are section 4.1's `{owner}/{name}`; the page resolves one through
`GET /v1/repositories/by-name/{owner}/{name}` and reads content by the id it
answers. The static routes above outrank `/:owner`, so a login spelled
`invitations` or `super-admin` (or the Worker's `v1`, `auth`) has no profile
page.

The session list on a repository page comes from the manifest: for each
provider scope, each `conversations` entry's `display_title`,
`presentation`, `message_count`, and `updated_at`. The project-file list
comes from the `project` scope's `files` and `directories`. A catalog item
page comes from the single `PublishedResourceEntry`.

A session page puts the pusher's GitHub avatar on prompt tiles, where the
desktop shows "You", with the login in the tile's tooltip. It reads
`pushed_by` from the head (or, for a collaborator's historical commit, the
history entry) whose commit the page shows; when that cannot be resolved the
tile falls back to "you". Public pages therefore disclose the pusher's
GitHub login and picture.

Catalog item pages: a skill page renders `SKILL.md` with `react-markdown`
and `remark-gfm` (no raw HTML) and lists files with sizes; a plugin page
shows the marketplace source, name, and resolution policy from the
descriptor; a pet page shows the idle frame cropped from the atlas with the
descriptor's geometry, as the desktop's pet preview does, plus format version
and dimensions. Nothing on the website executes stored content or performs a
provider action.

The Owner's review page shows the private snapshot with the same session and
item pages, the comparison from section 7.6, the disclosure text from
section 11.5, and Approve or Reject. Super Admin's queue shows the same page
with the Owner's decision recorded.

## 9. Identity and security

- **Sessions.** Desktop tokens are opaque, 90 days, revocable by logout or by
  an operator, stored hashed with a pepper from the secret store. Website
  sessions are opaque cookies, 30 days, `Secure`, `HttpOnly`,
  `SameSite=Lax`, host-only. Neither is refreshed or rotated in the MVP; a
  compromised token is revoked. Membership and role revocation take effect on
  the next request.
- **Authorization.** Every request resolves the account and checks
  `users.state`, repository state, and membership in the same D1 read; the
  guarded batch rechecks inside the transaction.
- **Plaintext content.** Content is not encrypted by Mallard. HTTPS, a
  private bucket, platform encryption, hashed tokens and codes, secrets in
  the Workers secret store, and audited operator access are the controls.
- **Content checks.** The desktop applies capture bounds, path validation,
  redaction, and review before upload. The Worker independently rejects
  manifests that fail section 10, absolute or traversing paths, unknown
  resource kinds, unregistered catalog coordinates, catalog content that
  differs from its bound identity, references outside the upload or the
  repository, and digest or length mismatches. Secret detection is
  best-effort.
- **Plugins.** The service stores intent; the desktop builds provider
  actions after Pull review; strings never select an executable.
- **Logging.** Logs, metrics, and audit hold opaque IDs, request IDs, error
  codes, byte counts, and digests. Never tokens, codes, OAuth material, object
  content, conversation text, skill content, pet bytes, or local paths.

## 10. Portable contracts and fixtures

The service accepts the current Rust encodings unchanged. All documents are
schema 1, UTF-8 JSON, integer Unix seconds, lowercase SHA-256 hex. "Optional"
means the field is omitted when absent, never `null`. Maps are JSON objects
with sorted keys (Rust `BTreeMap`). Types marked `closed` reject unknown
fields; the others ignore them, but a dropped field surfaces as a digest
mismatch on the desktop.

### 10.1 Project bundle

**`BundleManifest`**

| Field | Type | Notes |
| --- | --- | --- |
| `schema_version` | integer | 1 |
| `revision` | integer | Minted by the desktop: previous + 1 |
| `commit_id` | string | `commit-<hex>`, never reused |
| `updated_at` | integer | |
| `bundle` | `BundleIdentity` | |
| `published_by_origin` | `PortableOrigin` | |
| `scopes` | map `scope key → BundleScope` | Key `"project"` or `"provider:<key>"`; at most 64 provider scopes |

**`BundleIdentity`**: `bundle_id` (32 hex characters), `display_name`,
`kind` (`"project"`), optional `repository_fingerprint` (SHA-256 hex).

**`PortableOrigin`**: `origin_host_token`, `origin_replica_token`,
optional `shared_label`.

**`BundleScope`**

| Field | Type | Notes |
| --- | --- | --- |
| `revision` | integer | Per-scope, increments independently |
| `content_sha256` | string | Digest over the canonical view below, excluding itself |
| `recipe` | `BundleRecipe` | `{ schema_version, revision, entries: map resource_id → { resource_id, apply_policy, required } }` |
| `captured_with` | `CapturedWith` | `{ app_version, provider_versions: map, codec_versions: map }` |
| `conversations` | map thread id → `ConversationThread` | Provider scopes only |
| `resources` | map resource id → `ResourceDescriptor` | |
| `files` | map logical path → `BundleFileEntry` | |
| `directories` | map logical path → `BundleDirectoryEntry` | |
| `tombstones` | list of `Tombstone` | Serialized as a list ordered by structural key |

The canonical digest view serializes, in this order: `scope` (the key
string), `revision`, `recipe`, `captured_with`, `conversations`,
`resources`, `files`, `directories`, `tombstones` (as a list). The project
scope holds no conversations; conversation and provider-state resources
live in provider scopes; file and directory resources live in `project`.

**`ConversationThread`** (one conversation entry)

| Field | Type | Notes |
| --- | --- | --- |
| `thread_id` | string | Stable cloud thread ID, the map key |
| `provider` | string | `codex` or `claude` |
| `display_title` | string, optional | |
| `presentation` | `SessionPresentation` | Display metadata and metrics |
| `resource_id` | string | Derived from provider and thread ID |
| `files` | list of `ConversationFileRef` | `{ logical_path, role }` |
| `lineage_status` | string | Closed enum |
| `lineage_codec` | string | |
| `head_message_id` | string, optional | |
| `message_count` | integer | |
| `lineage_digest`, `semantic_digest` | string | |
| `portable_tail` | object, optional | `{ codec, partition_kind, start_record_count, accepted_end_record_count, accepted_tail_state }` |
| `wire_children` | string | `on` or `off`; required |
| `wire_redaction` | object, optional | `{ codec, fingerprint }`; required for codex and claude |
| `wire_path_map` | object, optional | Required for claude |
| `forked_from` | string, optional | |
| `preserved` | object, optional | Preserved provenance |
| `source_thread_id` | string, optional | Content fact, never identity |
| `created_by_origin`, `last_updated_by_origin` | `PortableOrigin`, optional | |
| `created_at`, `updated_at` | integer | |

`ConversationFileRef.role` is tagged by `kind`: `transcript`,
`provider_index_fragment`, `sidecar { sidecar_kind }`, or
`subagent_transcript { child_key, normalized: { message_count, sha256 } }`.
Logical paths are `state/conversations/{provider}/{thread_id}/…`; sidecars
under `…/sidecars/`, child transcripts at `…/subagents/{child_key}.jsonl`.
The transcript file's `BundleFileEntry` names the object the session view is
built from.

**`BundleFileEntry`**: `resource_id`, `sha256`, `size`, `source_mtime`,
`object_key` (`_objects/<sha256>`) or, for a project file above 32 MiB, `chunks`
(ordered `{ sha256, size }`, each an `_objects/<sha256>` object), optional `mode`.
**`BundleDirectoryEntry`**: `resource_id`, optional `mode`, `source_mtime`.
**`Tombstone`**: `target { kind, version, identity }`, optional
`last_sha256`, `deleted_at`; target kinds `resource`, `file`,
`project_content_file`, `project_content_directory`, `conversation_thread`,
all version 1.
**`ResourceDescriptor`**: `resource_id`, `kind { key, version }`, optional
`provider`, optional `provider_resource_type`, `display_name`,
`provenance { kind, version, fields }`, `apply_policy { key, version }`,
`codec_version`, `attributes`. Kinds: `conversation`, `provider_state`,
`project_content_file`, `project_content_directory`, `requirement`.
Policies: `safe_file`, `merge`, `explicit_review`, `manual_only`, `never`.

**`BundleHead`** (synthesized by the managed store, never stored by the
service): `schema_version`, `bundle_id`, `kind`, `revision`, `commit_id`,
`manifest_key`, `manifest_sha256`, `manifest_size`, `updated_at`.

### 10.2 Catalog item

**`LibraryResourceManifest`** (closed): `schema_version` 1,
`repository_id` (`library-` plus 32 hex, the coordinate's ID), `revision`,
`commit_id`, `updated_at`, `captured_with { app, version }`,
`published_by_origin { origin_host_token, origin_replica_token }`,
`resources` (map resource id → `PublishedResourceEntry`). The managed
validator requires exactly one entry.

**`PublishedResourceEntry`** (closed): `resource { CatalogResourceEnvelope }`,
`updated_at`.

**`CatalogResourceEnvelope`** (closed): `resource_id`, `provider`,
`resource_kind`, `format`, `format_version` (nonzero), `identity` (bounded
object), `descriptor` (bounded object), `content_sha256`, `files` (list of
`{ path, sha256, size, mode }`, omitted when empty; mode `420` or `493`,
that is `0644` or `0755`).

The service checks that `provider`, `resource_kind`, `format`,
`format_version`, and `resource_id` equal the repository's bound coordinate
and ID, that the coordinate is registered (section 3.4), and that every
file's digest is in the upload inventory.

### 10.3 Session view

`SessionView` is defined in section 5.4. Its `events` entries and the
count types are the desktop's session-detail types below.

### 10.4 Session detail types

**`SessionCount`**: optional `value` (integer), `quality` (`exact`,
`inferred`, `unsupported`), optional `explanation`.

**`SessionEventCounts`**: `prompts`, `responses`, `intermediate_steps`,
`checkpoints`, `tool_calls`, `file_edits`, `shell_calls`, `reads`,
`other_tools`, `agent_calls`, each a `SessionCount`.

**`SessionTimelineEvent`**

| Field | Type | Notes |
| --- | --- | --- |
| `event_id` | string | |
| `ordinal` | integer | |
| `timestamp` | integer, optional | |
| `kind` | string | `prompt`, `response`, `intermediate_step`, `checkpoint`, `tool_call`, `agent_call` |
| `quality` | string | As `SessionCount.quality` |
| `turn_id` | string, optional | |
| `turn_tool_summary` | object, optional | `{ turn_id, tool_calls: SessionCount }` |
| `parent_event_id` | string, optional | |
| `text` | string, optional | Prompt or response text, markdown |
| `tool` | `SessionToolCallPreview`, optional | |
| `checkpoint` | object, optional | `{ sha, short_sha, subject, committed_at, branch }` |
| `agent` | object, optional | `{ provider_agent_id?, label, task?, role?, model?, mode }`, mode `sync`, `async`, or `spawn` |

**`SessionToolCallPreview`**: `provider_call_id`, `native_name`,
`category` (closed enum of tool categories), `status`, `summary`, optional
`argument_preview`, `argument_truncated`, optional `result_preview`,
`result_truncated`.

**`ProjectSessionDetail`** (the desktop header, mirrored by the view's
top-level fields): `resource_id`, `provider`, `provider_display_name`,
`provider_session_id`, `title`, optional `summary`, optional `started_at`,
optional `ended_at`, `is_active`, optional `branch`, optional
`recorded_sha`, `tokens`, `event_counts`, `actions`, optional
`github_repository_url`, `snapshot_id`, `warnings`, optional `agent`.
The view omits `actions`, `snapshot_id`, `is_active`, and
`github_repository_url`.

### 10.5 Validation and fixtures

Validation in the Worker checks shape, closed enum values, bounds, class
binding, identity, and references. It does not recompute `content_sha256`
or the catalog canonical digests; the desktop verifies those on Pull, and the
Worker's job is admission, not authorship. Manifests over 8 MiB, more than
64 provider scopes, or more than 20,000 references are refused.

Fixtures are generated, not written: a Rust test under
`crates/mallard-domain` and `src-tauri/src/storage_catalog` behind the
`fixtures` feature writes valid and invalid JSON documents to
`services/managed-cloud/fixtures/`, including omitted optional fields, empty
maps, integer boundaries, an unregistered coordinate, a foreign identity, a
misplaced project file, and a manifest referencing an unlisted object. The
Worker's validator tests load that directory and must accept and reject the
same files. Regenerating fixtures is part of any DTO change.

## 11. Operations

### 11.1 Retention

| Data | Rule |
| --- | --- |
| The draft and the pushes before it | Until Discard draft removes the pushes after the latest version, or the repository is removed |
| Versions | Until the version or the repository is removed, within quota |
| Open upload | 24 hours |
| Unreferenced objects | Deleted by the next cleanup runs |
| Invitation code | Until rotated, revoked, or the repository is removed |
| Idempotency, auth attempts | 24 hours; 10 minutes |
| Audit | 90 days |
| External removal record | At least 30 days after erasure |

### 11.2 Audit and alerts

Audit rows record sign-in and logout, repository creation, invitation
create, revoke, and accept, membership changes, release decisions, private
replacement and discard, removals, Super Admin reads, and role grants. Before
external beta an operator must be able to find, from platform logs, failed
requests by `request_id`, integrity failures, quota totals that do not match
`objects`, and removals stuck in `deleting`. Alerts: error-rate and any
removal older than one hour in `deleting`.

### 11.3 Operator procedures

All run with `wrangler d1 execute` against production and are recorded in
the runbook with the operator and the date:

- Grant or revoke `super_admin`: `UPDATE users SET service_role = … WHERE github_login = …`.
- Suspend an account: `UPDATE users SET state = 'suspended' …` and delete its sessions.
- Revoke every session of an account.
- Run the cron work now (`POST /v1/super-admin/jobs/run`).

### 11.4 Recovery

Use D1 Time Travel to restore metadata; R2 keeps immutable bytes. Before
reopening traffic:

1. Apply every `removals/` record newer than the restore point: keep those
   repositories and revisions inaccessible.
2. Clear every restored private head and open upload, and invalidate open
   release requests. Private snapshots are not recovered; users push again.
3. Make every repository private, clear every invitation code, revoke
   sessions, and confirm `super_admin` grants against the runbook.
4. Verify each public publication's manifest and objects against R2 by size
   and digest; keep damaged publications unavailable. Recompute
   `committed_bytes` from `objects`.
5. Resume the cron Worker. Public access resumes only through a new Owner
   approval after a new push.

One isolated restore test with synthetic data runs before external beta and
covers a public revision, a later revision removal, a replaced private
snapshot, an interrupted upload, and a revoked member.

### 11.5 Required disclosure

Shown on the desktop before the first managed push and on the website before
Owner approval:

- Mallard stores selected content without end-to-end encryption; controlled
  operators and Super Admins can inspect it.
- The repository's Owner, Admins, and Members can read the draft and every
  version.
- The draft is the latest push, and a new push moves it. Every published
  version stays until the repository is removed. Discard draft removes the
  pushes after the latest version.
- Anyone can read the public version and every session in it, and sees the
  GitHub name and picture of whoever pushed it. Downloaded copies cannot be
  revoked.
- A public release needs Owner approval on the website. Super Admin approval
  is also required before the first release and after a Super Admin makes
  the repository private. Super Admins can remove any version or repository.
- Credentials and complete provider homes must not be selected.

## 12. Delivery

### 12.1 Steps

**Step 1: Private project sync.** Service: schema, guards, auth, repositories,
uploads, commit, head and object reads, cleanup and expiry. Desktop:
`ManagedCloudAdapter`, `ManagedObjectStore` with the staged write, the
setup lane, `loopback_browser.rs`. Exit: one account on two machines links a
Git project and a non-Git project, pushes, pulls, sees continuity classified
correctly after the other machine pushes, and a 90 MiB object round-trips on
the deployed plan. Section 12.2 rows 1 to 4 pass.

**Step 2: Website, collaboration, release.** Service: members, invitations,
release requests, decisions, history, views, public routes, removal, Super
Admin routes. Desktop: session views in the store, Request public release,
Open on website. Website: every route in section 8 for project
repositories. Exit: two accounts share a private repository by code; a
release is requested, approved by the Owner and Super Admin, and an anonymous
browser reads every session and its details; a later push leaves the public
version unchanged; a removal erases the repository; rows 5 to 12 pass.

**Step 3: Catalog items.** Desktop: `managed_cloud/catalog.rs`,
`CatalogItemLink`, the branch in the catalog push and pull commands. Service:
catalog validation. Website: skill, plugin, and pet pages. Exit: one skill,
one plugin intent, and one pet each round-trip privately and publish; rows 13
and 14 pass.

**Step 4: Beta readiness.** Alerts, audit review, the restore test, the
runbook, disclosure copy, quota tuning from measured usage. Exit: rows 15 and
16 pass and an operator is named.

### 12.2 Required checks

| # | Contract | Evidence |
| --- | --- | --- |
| 1 | Guards | Injected failures and zero-row updates abort whole batches; concurrent commits have one winner; the loser has no side effects |
| 2 | Upload protocol | Missing list correct; digest mismatch refused by R2; expiry cleans objects; abandon and retry work; two stores' uploads coexist and the later commit answers `head_changed`; `upload_in_progress` only past the per-member cap |
| 3 | Engine parity | Every `ObjectStore` behavior the R2 store passes in the storage-parity thread variants (section 3.7) passes against a miniflare service, except delete and clear |
| 4 | Continuity | Rows 4, 7, and 8 of the table in section 3.3 classify correctly after another collaborator's push; retired manifests serve; the 33rd retired manifest is gone |
| 5 | Identity | Desktop and website sign-in; state and challenge checks; revoked token or membership fails on the next request |
| 6 | Roles | Section 4.3 matrix; Owner row cannot be changed or removed; leave |
| 7 | Invitations | One use; expiry; revoke; inviter demoted before acceptance; replay grants nothing |
| 8 | Release | Owner approval required; absent or revoked admission also requires Super Admin; a newer push invalidates; a published revision references the same objects; private discard leaves public unchanged |
| 9 | Public reads | Anonymous reads exactly the public head's manifest, views, and objects; a stale `commit_id` and a private repository answer `not_found` |
| 10 | Views | Built from wire bytes: a redacted secret shows its placeholder; unchanged sessions keep their view; truncation flag at the cap; a conversation without a view renders "No preview" |
| 11 | Removal | Repository and revision removal deny reads immediately, erase keys, verify the prefix, and release quota once; the external record exists before erasure |
| 12 | Cleanup races | Claim before commit and commit before claim both leave accepted content intact |
| 13 | Catalog isolation | A local scope with three items publishes one; the service refuses a sibling or foreign identity; pull applies through the existing path |
| 14 | Catalog pages | Skill markdown renders without raw HTML; pet idle frame matches the desktop preview |
| 15 | Recovery | The restore test serves no cleared private snapshot or removed content and restores no revoked access |
| 16 | Frontend and Rust gates | `npm run build`, `npm run test:frontend-integration`, `cargo test --workspace --all-targets`, and `cd services/managed-cloud && npm test` are green |

### 12.3 Commands

```bash
cd services/managed-cloud && npm install && npx wrangler d1 migrations apply mallard-managed-cloud --local && npm run dev
```

```bash
cd services/managed-cloud && npm test
```

```bash
cargo test -p mallard-domain --features fixtures -- write_managed_cloud_fixtures
```

## 13. Decision record

Changes from the September 13 draft, with the reason. Section numbers in the
first column are the draft's.

| Draft | Now | Why |
| --- | --- | --- |
| Server-side session merge with chain comparison and divergence blocking (draft 3.3) | Whole-snapshot release | Removed a port of about 5,700 lines of codec logic to the Worker and a third comparator the desktop deliberately does not have |
| Dedicated desktop upload adapter beside the engine (draft 3.6, 4.4) | `ManagedObjectStore` behind `ObjectStore`; engine unchanged | The engine's carry-forward, retained-file, tombstone, and continuity logic are not reimplemented |
| Email invitations with delivery status, email HMACs, verified-email freshness (draft 2.5) | Owner-shared codes | No email provider, no `user:email` scope, no delivery state |
| Recent-authentication rules, refresh-token rotation, session families (draft 9.1) | Opaque 90-day tokens, revocation | Fewer moving parts for one client kind per session |
| Seven-day removal cancel window, deadline claims, takeover (draft 2.8) | Immediate removal | Nothing local is deleted; the window protected nothing |
| Public commit objects, rewritten public manifests (draft 6.3) | Public rows reference the private manifest object | No byte rewriting in the Worker; one manifest per commit |
| Client review digest and expiry on the upload (draft 3.6) | Only the manifest and inventory | The service cannot verify a review; the desktop already gates it |
| Idempotency on every mutation (draft 6.6) | Five operations | Revision guards make the rest safe to retry |
| Worker recomputes scope digests (draft 3.4) | Shape and reference validation only | Byte-exact canonical JSON in TypeScript bought nothing the desktop does not verify |
| No private history at all (draft 1.1) | Newest 32 retired private manifests readable as metadata | Continuity row 8 would otherwise report rewritten history after every collaborator push |
| Website for approval only (draft 11.7) | Website is the public product surface | Sessions, session details, and catalog items are browsable by anyone; the Owner reviews the private snapshot there |
| Session details decoded on the website | Session views built by the desktop from wire bytes | One transcript decoder, in Rust; redaction and path mapping apply by construction |
| Catalog item container type (draft 5.5) | `LibraryResourceManifest` with one entry | Reuses the existing type and fixtures |
| Desktop collaborator and invitation screens (draft 11.5) | Website only | The desktop adds one lane and two buttons |
| (added 2026-09-14) Stars erased when a repository goes private, as GitHub does | Stars persist; read access applies when the Starred list is read | One predicate at query time instead of cascades on make-private, member removal, and leave; a re-release restores the count |
| (added 2026-09-16) Membership gates every push; per-person single-use invitation codes | Anyone signed in pushes to a public repository (`Access.writer`, `G.writer`); a private repository has one standing code on its row (`repositories.invite_code_*`), set, rotated, or revoked by the Owner or an Admin | Public work needs no invitations, and the Owner still decides every release; one code per repository is what "invite at the repo level" asked for, and it drops a table, a quota, an expiry job, and a label |
| (added 2026-09-26) Stars need an account, one per account, as on GitHub | A star is a vote: signed in or not, up to 24 per voter per repository per UTC day; a signed-out voter is a connection (the IP hashed with the day's secret); Starred takes a repository off the list and keeps the count | Visitors can back a repository without signing in; the website's control stays as it was, and with no Trending page the daily cap bounds what one voter can add |
| (added 2026-10-05) One private slot that every push overwrites; a public release makes its own numbered row | Draft, numbered versions, and a public pointer (`PLAN_CLOUD_VERSIONS.md`): Publish freezes the draft as a version, a release names a version, Discard returns the draft to the latest version | The owner asked that a private repository keep more than its latest push. A public release row already was a frozen, numbered copy of a push, so a version is that row without the approval and a release is the pointer alone |

Kept from the draft: one draft that every push moves, Owner plus
Super Admin approval when admission is absent, one item per catalog repository, the
cleanup claim protocol, `publication_objects` for read authorization,
content-addressed create-only objects, GitHub sign-in through the Worker,
the disclosure list, and the restore test. The one-item catalog boundary is
the only reason the catalog path needs its own adapter; a later release may
revisit it if per-item public sharing is not used.

## Source pointers

Where the facts in section 3 live, for readers who want the code:

| Topic | Location |
| --- | --- |
| Object store trait, configured store, keys, marker | `crates/mallard-infrastructure/src/storage_core/{object_store.rs, mod.rs, marker.rs, connection.rs}` |
| Provider adapter trait, descriptor, registry | `crates/mallard-infrastructure/src/storage_providers/{traits.rs, registry.rs}` |
| Bundle engine | `crates/mallard-infrastructure/src/bundle_engine.rs` |
| Portable DTOs | `crates/mallard-domain/src/{lib.rs, portable.rs, conversation_tail.rs, subagents.rs}` |
| Continuity | `docs/DESIGN_REMOTE_HISTORY_CONTINUITY.md`, `crates/mallard-domain/src/lib.rs` (`classify_remote_continuity`) |
| Push and Pull service | `src-tauri/src/project_sync/{service.rs, commands.rs}` |
| Catalog domain, registry, codecs, engine, commands | `src-tauri/src/storage_catalog/{domain.rs, registry.rs, agent_skill.rs, plugin_intent.rs, codex_pet.rs, resource_engine.rs, resource_commands.rs}` |
| Session detail | `crates/mallard-infrastructure/src/{session_detail.rs, providers/traits.rs}`, `src/components/project-sync/session-detail/` |
| Dropbox sign-in | `crates/mallard-infrastructure/src/{dropbox_auth.rs, dropbox_browser.rs}`, `crates/mallard-application/src/dropbox_setup.rs`, `src-tauri/src/project_sync/dropbox_setup.rs`, `src/components/project-sync/DropboxSetup.tsx` |
| Lane UI pieces | `src/components/project-sync/{R2Setup.tsx, StorageSetupStep.tsx, StorageSettings.tsx, ProjectLinksWorkspace.tsx}` |
| Conventions | `AGENTS.md`, `tests/frontend/{ipc, project-sync-cutover, css-hygiene}.integration.test.tsx`, `src-tauri/src/sync_tests/` |
| Platform | D1 Worker API and limits, D1 Time Travel, R2 Workers API, Workers static assets, Workers limits, Cron Triggers, GitHub OAuth authorization, RFC 8252 |
