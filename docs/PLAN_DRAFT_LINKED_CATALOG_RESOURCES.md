# Implementation plan: Draft-linked skills, plugins, and pets

**Status:** implemented clean-cutover design authority

**Date:** 2026-08-07

**Scope:** Codex and Claude global skills and plugins, plus Codex v1 and v2
pets

This document is self-contained. It supersedes the storage-wide catalog setup
and binding model described in
`DESIGN_STORAGE_SCOPED_CODEX_CLAUDE_CATALOG.md` and
`PLAN_STORAGE_SCOPED_CODEX_CLAUDE_CATALOG.md`. Those documents remain design
history only and must not be consulted to fill gaps in this plan.

## 1. Decision

Skills, plugins, and pets become explicit Mallard resources, following the
same product shape as projects:

1. an unlinked provider resource is discovered but is not automatically owned
   by Mallard;
2. the user starts a resumable setup draft for one resource;
3. in the draft, the user chooses the provider profile, the storage, and the
   remote resource repository to link;
4. **Link** atomically creates a machine-local resource registration and one
   storage link; and
5. Push, Pull, Refresh, and Fork operate from the linked resource workspace.

The setup draft is the only creation path for a registration or storage link.
There is no instant pair, implicit binding, inventory-wide opt-in, or automatic
linking based only on matching names.

The first implementation supports exactly:

| Resource | Codex | Claude |
|---|---|---|
| Standalone global Agent Skills | Yes | Yes |
| User-global plugin install intent | Yes | Yes |
| v1 pet | Yes | No |
| v2 pet | Yes | No |

The compiled registry is extendable only by a later reviewed code change that
adds a complete provider/resource adapter, fixtures, UI capability, and tests.
There is no runtime provider catalog, arbitrary path entry, generic adapter,
or unused future-provider scaffold in this delivery.

Codex v1 and v2 pets are different resource kinds, different remote
repositories, and different links. They may coexist in storage for the same
pet ID. The UI always labels the format, and the user explicitly chooses the
format to Push or Pull when both address the same local pet target.

This is a clean cutover:

- **No backward compatibility support.** Do not retain command aliases, serde
  aliases, old readers, fallbacks, feature flags, dual reads, dual writes, or
  alternate execution paths.
- **No migration.** Do not detect, import, translate, rewrite, or delete the
  previous storage-wide catalog, its machine-local binding document, or its
  remote objects. The new implementation starts with new state and a new
  remote namespace.
- **No legacy execution path.** Every production UI action calls the new
  draft, registration, link, or per-link operation family directly.
- **No dead code.** Remove superseded types, persistence methods, commands,
  engines, adapters, components, API methods, tests, fixtures, styles, and
  documentation claims in the same implementation change.

## 2. Product model

### 2.1 Discovered resource

A discovered resource is a bounded, read-only observation from one selected
provider profile. Discovery does not persist ownership or create a link.

Examples are:

- a standalone skill in a supported machine-global skill root;
- one provider-native, user-owned plugin installation intent; or
- one canonical Codex v1 or v2 pet package.

Discovery returns the stable typed `resource_id`, safe display fields, and the
current content digest. A draft update submits only that `resource_id`; the
backend re-runs bounded inventory, resolves the canonical target itself, and
stores the exact kind, target, and digest pin. Finalization inventories again
and requires the pin to match. The frontend never submits an arbitrary install
path or claims that bytes are still current.

### 2.2 Resource registration

A `CatalogResourceRegistration` represents one provider-scoped local resource
that the user intentionally linked through setup. It owns:

```text
registration_id
provider_key
provider_profile_id
resource_kind
stable_resource_identity
canonical_local_target
display_name
revision
created_at
updated_at
```

The stable identity is typed by resource kind:

- skill: provider, supported global root class, effective name, and install
  directory;
- plugin: provider, native namespace, and qualified plugin ID;
- v1 pet: Codex profile and pet ID;
- v2 pet: Codex profile and pet ID.

Changing provider, profile, root class, install directory, qualified plugin
identity, pet ID, or pet format creates a different registration through a new
draft. It is never an in-place edit.

The same local pet target may have one v1 registration and one v2
registration. Target-level locks and Pull planning still allow only one format
to be materialized at a time. Two registrations of the same pet format for the
same profile and pet ID are rejected.

### 2.3 Resource storage link

A `CatalogResourceLink` joins one registration to one configured storage and
one remote resource repository:

```text
link_id
registration_id
storage_id
resource_repository_id
accepted_head                 // optional
accepted_content_sha256       // optional; paired with accepted_head
revision
created_at
updated_at
```

A registration may link to multiple storages, just as a project may. There is
at most one active link for `(registration_id, storage_id)`, and one remote
repository cannot be claimed by two incompatible local identities on the same
storage.

The link, not the storage, owns the accepted comparison base. Link state is
derived from the accepted base, current local observation, and current remote
head:

```text
new | in_sync | local_changed | storage_changed | diverged |
local_missing | remote_missing | unreviewed_remote | blocked
```

Do not persist a second status flag that can disagree with those facts.

### 2.4 Setup draft

`CatalogResourceSetupDraft` is a bounded, machine-local, resumable document:

```rust
#[serde(deny_unknown_fields)]
struct CatalogResourceSetupDraft {
    schema: u32, // exactly 1 for this new document family
    draft_id: CatalogSetupDraftId,
    family: CatalogResourceFamily, // skill | plugin | pet
    registration_id: CatalogRegistrationId,
    link_id: CatalogLinkId,
    new_resource_repository_id: CatalogResourceRepositoryId,
    provider: Option<DraftCatalogProviderSelection>,
    local_resource: Option<DraftLocalResourceSelection>,
    storage: Option<DraftStorageSelection>,
    repository: DraftCatalogRepositoryChoice,
    revision: u64,
    created_at: u64,
    updated_at: u64,
    last_error: Option<String>,
}
```

The preallocated IDs make every finalize retry converge on the same records.
Drafts contain configuration choices and pinned observations, never resource
payload bytes, provider output, or remote listings. If a pending storage is
used, its credentials remain only in this permission-restricted machine-local
draft and final storage configuration; they are never portable or logged.

Provider selection contains both the closed provider key and either:

- an existing catalog-owned provider profile ID; or
- a pending profile home and display name to validate and create at finalize.

Storage selection contains either an existing storage ID or a pending storage
configuration. Pending storage creation reuses the neutral storage validation
and persistence service used by project setup; it does not import project
draft DTOs or commands.

Repository choice is one of:

```text
undecided
new(new_resource_repository_id)
existing(storage_id, resource_repository_id, observed_tag, observed_head)
```

A new repository requires a current local resource selection. An existing
repository may be linked when the local resource is present or absent. Its
provider, kind, and stable identity must exactly match the selected target;
there is no mismatch acknowledgement for a different resource identity.

### 2.5 Finalization transaction

`CatalogSetupTransaction` is written before the first durable mutation and
contains the exact profile, storage, registration, and link records that one
finalize attempt will create.

Finalization must:

1. lock catalog state and load the exact draft revision;
2. revalidate pending or referenced provider and storage configuration;
3. re-run provider inventory and revalidate the selected local candidate when
   one is required;
4. re-read and validate the selected remote tag and head when linking an
   existing repository;
5. reject provider, resource kind, stable identity, target, case-folded, pet
   format, link, or repository-claim collisions; cross-storage content claims
   are resolved only by a later explicit Pull review because Link performs no
   local mutation;
6. persist the transaction with the preallocated identities;
7. create or reuse the exact pending profile and storage records;
8. create the registration and link idempotently;
9. delete the completed transaction and draft only after all records agree;
   and
10. return the linked resource detail.

The finalize result contains the registration, link, and a closed completion
value: `open` or `review_pull`. For `review_pull`, the frontend immediately
prepares a fresh per-link Pull plan; finalization does not manufacture or apply
a hidden Pull plan.

Finalization does not Push, Pull, install, overwrite, copy, or execute a
plugin. **Link** changes only Mallard's machine-local configuration. When an
existing remote repository has unaccepted content, the completion response
opens a separate Pull review; it does not apply the Pull automatically.

Startup recovery completes an idempotent transaction whose applied records
match exactly. It rolls back only records proved to have been created solely
by the incomplete transaction. Ambiguous state blocks with an actionable
error; it is never guessed into success.

## 3. Draft user flow

Each Library category has an **Add** action. The category fixes the resource
family and creates an empty draft before any provider or storage is committed.
The draft appears immediately in the category list with a **Draft** badge and
survives navigation or restart.

The setup workspace has three ordered stages.

### 3.1 Provider

- Skills and plugins offer exactly Codex and Claude.
- Pets show Codex as the only supported provider and record it explicitly.
- The user chooses an existing provider home or adds a pending home.
- After the profile is selected, Mallard shows bounded compatible local
  candidates for this resource family.
- Choosing a different provider clears the local candidate and any
  incompatible remote repository choice.
- Project directories are never scanned.

Supported skill discovery is limited to:

```text
<CODEX_HOME>/skills
$HOME/.agents/skills
<CLAUDE_CONFIG_DIR>/skills
```

The first two are Codex roots with distinct identity classes; the third is a
Claude root. With the default Codex home, `<CODEX_HOME>/skills` is
`$HOME/.codex/skills`. Paths such as `<project>/.skill/`, `<project>/.skills/`,
`<project>/.agent/ssd`, `<project>/.agents/skills`, and
`<project>/.claude/skills` are always ignored.

### 3.2 Storage

- The user chooses one existing local-folder or S3-compatible storage, or
  creates a pending storage with the current storage editor flow.
- Changing storage clears the remote repository choice because remote
  identities are storage-specific.
- The draft inspection loads compatible remote resource tags only after a
  storage has been selected.
- A pending storage has no remote listing yet, so it can select only **New
  stored resource** until finalization creates the storage configuration.
- Storage errors affect this draft and its linked resources only; project sync
  on the same storage remains available.

### 3.3 Link

The user chooses:

- **New stored resource** to link the selected local resource to the
  preallocated empty remote repository; or
- one compatible existing stored resource to link and review.

Exact stable-identity matches may be recommended, but never selected and
finalized without visible user choice. Remote entries display provider,
resource kind, scope, and pet format where applicable.

The primary action is:

- **Link** for a new empty remote repository or an existing repository already
  matching the local bytes; or
- **Link & review Pull** when the selected remote repository contains
  unaccepted or different content.

For an exact local/remote byte match, the visible Link choice is sufficient to
record the observed remote head and digest as the initial accepted base. For a
different or locally absent resource, finalization leaves the base unaccepted
and returns `review_pull`; the frontend then prepares a new Pull plan against
the finalized link.

The action is disabled while autosave is pending, inspection is stale, a
required section is incomplete, or finalization is running.

Draft behavior matches project setup where useful:

- revision-guarded debounced autosave;
- per-section `ready`, `attention`, and `blocked` inspection state;
- stale-response suppression;
- resumable Draft rows;
- a single-flight finalize action;
- visible saving and failure status; and
- **Discard**, which removes only the draft and touches no provider or storage
  content.

This is interaction reuse, not domain reuse. Catalog setup keeps its own DTOs,
commands, validation, persistence, and transaction type.

## 4. Linked resource workspace

After finalization, the category page replaces the Draft row with a registered
resource row. Opening it shows the provider identity and one storage card per
link. Additional storages are connected by starting another setup draft seeded
with the existing registration; provider and local identity are then locked,
while storage and remote repository remain selectable.

Each storage card exposes **Push**, **Pull**, and **Refresh**. Eligible local
skills and pets also expose **Fork** at the resource level.

All four operations follow this self-contained interaction lifecycle:

1. actions are available only in browse mode and plan against the currently
   visible registration/link revision;
2. preparing Push or Pull opens an inline review in the same workspace;
3. Push has an explicit include decision and any required content-warning
   acknowledgements; Pull shows the exact action, alternatives, copy name,
   destructive acknowledgement, and pet format choice;
4. closing review performs no mutation and returns to browse mode;
5. prepare and apply are single-flight, show one accessible progress state,
   and disable conflicting navigation/actions;
6. an apply failure or partial provider outcome closes the consumed plan into
   a truthful outcome, does not advance the accepted base, and never reports
   success for unapplied work;
7. every outcome reloads the local and remote comparison without hiding the
   result;
8. retry or reopen always prepares a new plan rather than replaying stale
   arguments; and
9. navigation or a newer request invalidates late results by workflow epoch,
   registration ID, and link ID.

A user-requested Refresh clears an older operation/Fork outcome only after
both local and remote reloads succeed. The automatic Refresh after Fork keeps
the new Fork outcome visible.

### 4.1 Push

Push is planned and reviewed for exactly one link and one resource repository.

- `new` and `local_changed` may Push after review.
- `storage_changed`, `diverged`, `unreviewed_remote`, and remote continuity
  failures block Push and route to Pull.
- `local_missing` never publishes a deletion.
- The plan pins link revision, provider profile revision, local identity and
  digest, storage identity, observed remote head, warning digests, and expiry.
- Publication writes immutable verified objects and advances only that
  resource repository head through compare-and-swap.
- A head change after review performs no visible mutation and requires a new
  plan.

The inline review, progress, close, retry, and compact outcome lifecycle must
match the existing project/thread Push behavior wherever the semantics apply.

### 4.2 Pull

Pull is also planned per link. Accepting a remote head and mutating the local
target remain separate, explicit decisions recorded by the plan.

- Missing or storage-changed skills and pets install or update through a
  journaled directory transaction.
- Divergent skills and pets default to **Fork and overwrite**: create a
  reviewed, semantically renamed local copy and then install the storage
  version at the original target.
- Skill copies rewrite the directory and declared skill name together.
- Pet copies rewrite directory, manifest ID, and display name while preserving
  the exact source format and atlas; there is no v1/v2 conversion.
- Plugins allow provider-native **Overwrite** or **Skip** only. They are never
  copied, renamed, or replaced by raw filesystem operations.
- When v1 and v2 links share a pet target, they remain separate resource rows
  and separate per-link reviews. The user explicitly opens the v1 or v2 link
  to Pull, or chooses Skip; no combined plan or implicit format selection
  exists, and at most one format occupies the shared local target.
- Remote removal never deletes or uninstalls local content.

The plan pins every local and remote digest, generated copy identity, target,
provider operation, cross-storage claim, link revision, and expiry. A claim at
another digest changes an ordinary update into a visible conflict: skills and
pets offer named semantic preservation plus acknowledged overwrite or Skip;
plugins offer acknowledged provider-native overwrite or Skip. Apply rejects a
changed claim set before mutation. The accepted comparison base advances only
after a successful local action or an explicit Skip; failed and partial
provider outcomes remain unaccepted and are reported truthfully.

### 4.3 Refresh

Refresh is browse-only and reloads the selected link's local provider
observation and remote repository comparison as one lifecycle. It is read-only
and uses one busy/error state, accessible status, request generation, and
stale-response suppression. Refresh invalidates older Push and Pull plans.

### 4.4 Fork

Fork is available only for a locally present standalone skill or pet. It uses
the existing semantic, no-clobber, digest-pinned, journaled copy primitive and
then Refreshes local inventory. The copy is an unlinked discovered candidate;
linking it requires a new setup draft. Fork is unavailable for plugins and
storage-only resources and never calls the native thread-fork executor.

### 4.5 Unlink and remove

Unlink removes the selected link, its accepted base, claims, and pending plans
after revision validation. It does not delete local provider content or remote
objects. A registration can be removed only after all links are gone; removal
also leaves provider content untouched.

Removing a provider profile or storage is blocked while a registration, link,
draft, transaction, plan, journal, or claim references it.

## 5. Remote storage protocol

The storage-wide mutable catalog head is removed. Each linked resource owns an
independent remote repository:

```text
<storage root>/.mallard/
  _storage.json
  v1/
    repositories/<project-bundle-id>/...
    library/<resource-repository-id>/
      _tag.json
      _head.json
      _objects/<sha256>
      _manifests/<revision>-<commit-id>.json
      _commits/<revision>-<commit-id>.json
```

The new `library/` namespace is the only catalog-resource namespace read by
the new implementation. The former flat `v1/catalog/` namespace is not read,
listed, converted, deleted, or used as a fallback.

`_tag.json` is a bounded, immutable, create-only discovery record containing
the repository ID, provider, closed resource kind, stable identity digest,
safe display fields, and creation time. The mutable head is the sole authority
for the current revision. Listing accepts only exact tag keys immediately
beneath a validated resource-repository ID. A tag is never trusted as payload:
opening a choice verifies its head, manifest, typed identity, sizes, hashes,
and immutable objects.

Each manifest contains exactly one typed resource. The remote resource kind
and stable identity are immutable for the life of the repository. A different
identity requires a new repository and a new setup draft.

“Resource repository” is only the immutable/CAS container inside a configured
storage. It is not a Git repository, project repository, project binding, or
repository-scoped feature; catalog resources remain storage-level and work
with zero projects registered.

The implementation adds a closed `library_object_key(repository_id,
relative)` constructor and a closed listing prefix for library tags. Project
repository keys and catalog-resource keys remain mutually unrepresentable.
Do not add `BundleKind::Catalog` or teach the project bundle engine catalog
semantics.

Local-folder and S3-compatible stores must provide equivalent bounded listing,
immutable tag/object put, exact read, conditional head update, and
ambiguous-CAS read-back behavior.

## 6. Machine-local persistence

The new catalog resource domain owns new paths under `~/.mallard`:

```text
catalog_resource_profiles.json
catalog_resources.json
catalog_setup_drafts/<draft-id>.json
catalog_setup_transactions/<draft-id>.json
catalog_resource_plans/<plan-id>.json
catalog_resource_operations.json
catalog_resource_materializations.json
catalog_resource_preservations.json
backups/catalog_resources/<plan-id>/...
locks/catalog-state.lock
locks/provider-<canonical-path-sha256>.lock
catalog_quarantine/<document>-<timestamp>.quarantined
```

`catalog_resources.json` contains registrations and links in one bounded,
atomically replaced document so their referential invariants validate
together. Plans are immutable create-only documents. Drafts and transactions
are separate bounded documents so one corrupt entry can be quarantined
without resetting accepted bases for unrelated resources.

Every document uses schema 1 for this new file family, strict unknown-field
rejection, bounded counts and text, atomic write/rename, restrictive
permissions, interprocess locking, and fail-closed corruption handling.

The implementation does not read or migrate the old
`catalog_profiles.json`, `storage_catalog.json`, flat catalog plans, or their
binding/base records. New file names are intentional and there is no startup
detector or cleanup path for the old files.

Absolute provider paths, storage credentials, local target identities,
accepted bases, claims, plans, receipts, drafts, and journals remain
machine-local. Portable tags, manifests, and commits contain no local path,
profile ID, storage ID, host ID, project ID, credential, login state, or
provider command output.

## 7. Backend ownership and command cutover

Keep storage catalog code independent from `project_sync` domain types and
executors. Share only proven neutral infrastructure:

- object-store transport and storage configuration validation;
- resource-neutral review presentation primitives;
- bounded filesystem and atomic persistence helpers where their APIs contain
  no feature DTOs; and
- provider adapters and semantic skill/pet copy code after they have direct
  production callers in the new path.

The production command family becomes:

```text
list_catalog_capabilities
list_catalog_resource_profiles
save_catalog_resource_profile
delete_catalog_resource_profile

list_catalog_setup_drafts
create_catalog_setup_draft
update_catalog_setup_draft
discard_catalog_setup_draft
inspect_catalog_setup_draft
finalize_catalog_resource_setup

list_catalog_resources
get_catalog_resource
unlink_catalog_resource
remove_catalog_resource

inspect_catalog_resource_link
prepare_catalog_resource_push
publish_catalog_resource
prepare_catalog_resource_pull
apply_catalog_resource_pull
prepare_catalog_resource_fork
apply_catalog_resource_fork
get_catalog_resource_status
```

Multiword Tauri arguments remain camelCase in TypeScript and snake_case in
Rust. Catalog DTOs live beside `src/components/storage-catalog`; they do not
enter project-sync DTOs. Every registered command and frontend API method must
have a concrete production caller.

Draft update accepts only editable selections plus `draft_id` and
`expected_revision`. Server-owned IDs, pinned observations, canonical paths,
timestamps, and derived status never round-trip as client-authoritative
fields.

The following storage-wide commands are deleted, not wrapped:

```text
get_storage_catalog_binding
save_storage_catalog_binding
inspect_storage_catalog
prepare_storage_catalog_push
publish_storage_catalog
prepare_storage_catalog_pull
get_storage_catalog_plan
apply_storage_catalog_pull
get_storage_catalog_status
```

Any current command name reused by the new family receives only the new
request and response contract; there is no overload or legacy argument
decoder.

## 8. Frontend cutover

Retain the top-level **Skills**, **Plugins**, and **Pets** Library categories.
Within each category:

- show resumable drafts first with Draft/attention state, Continue, and
  Discard actions;
- show registered resources with provider, scope, kind/format, and linked
  storage count;
- keep unlinked discovery behind Add/setup rather than treating every item in
  a provider home as Mallard-owned;
- open one resource workspace with project-like storage link cards;
- create additional links through a draft, never through an inline binding
  mutation; and
- keep review and outcomes inline rather than opening a second catalog shell.

Reuse the current project setup interaction language and existing neutral
review components where possible. Do not import `ProjectSetupDraft`, project
bindings, project commands, thread DTOs, conversation codecs, or the native
thread-fork executor into storage-catalog code.

Delete the current `StorageLibraryOverview` whole-storage profile pairing,
storage-wide switcher state, bulk catalog Push/Pull controls, and any view
model whose only purpose is the flat catalog head. Styles shared with project
setup may remain only when both rendered production flows use them; otherwise
rename or delete them with the old component.

Single-flight and stale-result guards are scoped by workflow epoch plus draft,
registration, and link IDs so navigation cannot land results in another
resource workspace.

## 9. Implementation phases

### Phase 0 — freeze contracts and tests

1. Record the supported Codex and Claude provider inventory fixtures.
2. Freeze exact user-global plugin list/install/overwrite behavior and exclude
   project, local, managed, workspace, administrator, cache, and bundled
   entries.
3. Freeze Codex skill roots, Claude skill root, pet v1/v2 parsing, semantic
   copy behavior, and project-root exclusions.
4. Add registration-freeze tests for the new command family and negative
   assertions for every storage-wide command being removed.
5. Add storage key tests proving project, new library, and old flat catalog
   namespaces cannot be confused.

### Phase 1 — per-resource storage engine

1. Add closed resource-repository IDs, keys, tags, heads, manifests, commits,
   and listing DTOs.
2. Implement local-folder and S3 tag listing with bounded pagination and exact
   key filtering.
3. Implement immutable publish and CAS/read-back for one resource repository.
4. Test corruption, missing objects, forged tags, identity changes, stale
   heads, ambiguous writes, and cross-namespace keys.
5. Remove the flat catalog key constructor and engine once the new callers are
   live; do not retain an adapter.

### Phase 2 — registration, link, and draft domain

1. Add validated IDs and strict schema-1 documents for profiles,
   registrations, links, drafts, transactions, plans, claims, and operations.
2. Implement derived link state and all uniqueness/reference invariants.
3. Implement bounded atomic persistence, quarantine, locks, and revision
   checks on the new paths.
4. Implement draft create/list/get/update/discard/inspect behavior.
5. Implement idempotent finalization and startup transaction recovery with
   injected crash-point tests.

### Phase 3 — provider and remote choices

1. Connect the closed Codex/Claude adapter registry to draft inspection.
2. Return typed resource IDs and digest-pinned local candidate choices after
   provider selection; re-inventory when a choice is saved or finalized.
3. Return verified compatible remote choices after storage selection.
4. Enforce exact provider/kind/stable-identity matching and pet format labels.
5. Test profile changes, storage changes, inventory races, path collisions,
   duplicate drafts, and local-absent remote linking.

### Phase 4 — per-link operations

1. Implement Refresh and derived comparison for one link.
2. Implement single-resource Push planning/publication and stale/CAS guards.
3. Implement Pull planning, acceptance, directory transactions, plugin-native
   overwrite/skip, pet target selection, receipts, and recovery.
4. Reconnect semantic skill/pet Fork so its output is an unlinked candidate.
5. Enforce cross-storage claims and per-target operation locks.
6. Delete storage-wide plans, desired-resource membership, per-storage bases,
   and bulk reconciliation code.

### Phase 5 — setup and linked-resource UI

1. Add category draft rows and the Provider → Storage → Link workspace.
2. Add revision-guarded autosave, inspection, discard, finalize, resume, and
   recovery feedback.
3. Replace storage-wide category browsing with registered resource rows and
   per-link storage cards.
4. Wire inline Push/Pull review, Refresh, Fork, unlink, remove, progress, and
   outcomes through the typed catalog API.
5. Add stale-request and navigation race tests.
6. Verify the complete Tauri app visually for all three categories, both
   provider choices, both storage kinds, and pet v1/v2 conflicts.

### Phase 6 — hard cutover and audit

1. Delete the old binding/profile document types, persistence functions,
   commands, registrations, frontend API methods, components, styles, tests,
   fixtures, and flat remote engine.
2. Remove current docs or mark them historical so only this plan describes
   implementation authority.
3. Search production and tests for every removed command, DTO, filename,
   remote key, component, feature flag, alias, and fallback.
4. Confirm every remaining module/export/command/API method has a production
   caller and every provider branch is reachable through the support matrix.
5. Run the complete verification set before handoff.

The feature must not ship between phases with both storage-wide and
resource-linked execution paths registered. Development sequencing may keep
work buildable, but the delivered tree contains only the new path.

## 10. Required tests

### Domain and persistence

- strict schemas reject unknown fields and versions;
- draft updates reject stale revisions and client-owned identity changes;
- drafts resume after restart and discard without side effects;
- referenced-profile/storage removal produces attention or blocks deletion;
- duplicate registrations, links, repository claims, and target collisions
  fail closed;
- v1/v2 pets remain distinct while sharing one materialization lock;
- finalization is idempotent across every injected crash boundary;
- corrupt draft isolation never resets an unrelated accepted link base; and
- old catalog files are never opened by the new repository.

### Storage and reconciliation

- local-folder and stub-S3 list the same verified resource repositories;
- pagination, forged keys, symlinks, missing objects, excessive sizes, digest
  mismatches, unknown kinds, and changed identities block safely;
- concurrent head changes make Push/Pull plans stale;
- one link's publish cannot advance another resource repository;
- new, matching, local-changed, storage-changed, diverged, missing, and
  unreviewed states derive correctly; and
- old flat catalog objects are neither listed nor modified.

### Provider mutation

- only the exact global skill roots are scanned;
- plugin-owned skills are not duplicated as standalone skills;
- plugins execute only verified provider-native user-global operations;
- plugin overwrite and skip are the only divergent choices;
- skill/pet preservation is named, semantic, pinned, no-clobber, journaled,
  backed up, and recoverable;
- v1/v2 conversion never occurs; and
- cross-storage claims prevent an unreviewed overwrite of a target claimed at
  another digest.

### Frontend integration

- Add creates a visible resumable Draft row in the selected category;
- Provider changes clear incompatible resource/repository choices;
- Storage changes clear the remote choice;
- autosave, stale inspection suppression, attention state, resume, and
  Discard match project setup behavior;
- Link creates one registration and one link and performs no Push/Pull;
- Link & review opens Pull without applying it;
- an existing registration can add a second storage through a seeded draft;
- Push/Pull/Refresh lifecycle and outcomes match the project/thread behavior;
- Fork produces an unlinked candidate and never appears for plugins;
- pets show explicit v1/v2 choices and allow at most one local format; and
- no storage-wide binding or bulk catalog action remains reachable.

### Clean-break assertions

- the old storage-wide commands are not registered;
- old TypeScript API methods and DTOs do not compile;
- old catalog filenames and `v1/catalog` are absent from production reads;
- no legacy schema alias, fallback, migration, importer, detector, dual write,
  or feature flag exists;
- no unsupported provider is selectable or probed; and
- no production module, export, command, or component is unreachable.

## 11. Verification commands

Run at minimum:

```text
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:backend-integration
npm run test:integration
```

If S3 tests need loopback permission, rerun them with the required sandbox
approval and report that explicitly. Native provider tests use fixture homes
and injected runners only; never test against real `~/.codex`, `~/.claude`, or
`~/.mallard`.

## 12. Done definition

The cutover is complete only when all of the following are true:

1. Every skill, plugin, v1 pet, and v2 pet enters Mallard through a resumable
   Provider → Storage → Link draft.
2. Finalization creates exactly one registration and link, is revision-guarded
   and crash-recoverable, and performs no hidden sync or provider mutation.
3. Each link owns an independent remote resource repository and accepted base.
4. A registration can link additional storages only through another draft.
5. Push, Pull, and Refresh operate per link with immutable objects, verified
   digests, expiring plans, and head CAS.
6. Skill/pet divergence defaults to reviewed named preservation plus
   overwrite; plugin divergence offers overwrite or skip only.
7. Pet v1 and v2 remain different resources and explicit user choices without
   conversion or simultaneous local materialization.
8. The UI reuses the project setup and review interaction patterns without
   importing project/thread domain types or executors.
9. Project-local skill lookalikes are never scanned or linked.
10. Exactly Codex/Claude skills and plugins and Codex pets are compiled and
    visible; there are no speculative provider hooks or generic adapters.
11. The old storage-wide catalog binding, flat head, bulk operations, command
    family, persistence, and UI are absent from the production tree.
12. There is no backward compatibility support, migration, legacy execution
    path, dead code, or unused scaffolding.
