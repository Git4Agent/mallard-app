# Implementation plan: Storage-scoped Codex and Claude catalog

**Status:** proposed

**Date:** 2026-08-07

**Scope:** Codex and Claude skills/plugins, plus Codex v1 and v2 pets

**Design:** [`DESIGN_STORAGE_SCOPED_CODEX_CLAUDE_CATALOG.md`](./DESIGN_STORAGE_SCOPED_CODEX_CLAUDE_CATALOG.md)

This plan is self-contained. It does not require another capability, catalog,
repository-sync, or compatibility plan to define implementation behavior.

## 1. Delivery gate

The first and only delivery described here is:

| Resource | Codex | Claude |
|---|---|---|
| Standalone Agent Skills | Yes | Yes |
| Plugin install intent | Yes | Yes |
| v1 and v2 pets | Yes | No |

The compiled registry contains exactly `codex` and `claude`.

Conflict delivery is fixed: storage-newer and divergent resources are held for
Pull while selected non-conflicting resources still Push; divergent skills and
pets default to a reviewed named preservation copy followed by storage
overwrite; divergent plugins allow native overwrite or skip only.

The Library exposes four catalog operations:

| Operation | Skills | Plugins | Pets | Thread-behavior contract |
|---|---|---|---|---|
| Push | Yes | Yes | Yes | Same inline review, selection, per-resource storage-newer holds, progress, and outcome lifecycle |
| Pull | Yes | Yes | Yes | Same inline review, row decisions, default preservation choice, approval, progress, and outcome lifecycle |
| Refresh | Yes | Yes | Yes | Same explicit reload of local inventory and storage comparison, busy/error handling, and stale-response suppression |
| Fork | Local named copy | No | Local named copy | Same browse-row affordance and post-action refresh shape; catalog-specific copy mechanics |

Push, Pull, and Refresh parity is a release requirement, not merely visual
similarity. Catalog DTOs, plans, and commands remain independent from thread
types and executors. Fork deliberately differs at the domain boundary: it
creates a validated local skill or pet copy and never calls the native Codex
thread-fork implementation.

Do not add:

- Cursor, OpenCode, Pi, Gemini CLI, Copilot, Goose, or generic-provider
  adapters;
- profiles or UI selectors for an unsupported provider;
- unsupported-provider filesystem probes;
- commands that accept an arbitrary provider implementation;
- custom agents, commands, prompts, rules, MCP, hooks, policies, or themes;
- repository-scoped catalog resources; or
- a migration or compatibility reader.

Clean-break requirements:

- **No backward compatibility support:** do not retain legacy readers, serde
  aliases, command aliases, fallbacks, dual reads, dual writes, or feature
  flags for the removed behavior.
- **No migration:** do not detect, import, translate, or rewrite
  repository-scoped capability data or an earlier catalog format.
- **No dead code:** delete superseded types, commands, adapters, parsers, UI
  routes, tests, fixtures, and documentation in the same change that replaces
  them.

## 2. Module layout

```text
src-tauri/src/
  storage_core/
    mod.rs
    object_store.rs
    local_store.rs
    marker.rs
    s3_store.rs
  storage_catalog/
    mod.rs
    domain.rs
    profiles.rs
    registry.rs
    inventory.rs
    engine.rs
    reconciliation.rs
    planning.rs
    apply.rs
    directory_transaction.rs
    persistence.rs
    commands.rs
    provider_adapters/
      codex.rs
      claude.rs
    resource_adapters/
      agent_skill.rs
      plugin_intent.rs
      codex_pet.rs
    tests/
  project_sync/
    ... existing repository-only modules

src/components/storage-catalog/
  StorageLibraryWorkspace.tsx
  StorageLibraryOverview.tsx
  StorageSkillList.tsx
  StoragePluginList.tsx
  StoragePetList.tsx
  StorageCatalogPushReview.tsx
  StorageCatalogPullReview.tsx
  api.ts
  model.ts
  types.ts

src/components/sync-review/
  SyncReviewTabs.tsx
  SyncReviewSelectionBar.tsx
  SyncOutcomeNotice.tsx
```

Dependency direction:

```text
project_sync ----\
                  > storage_core
storage_catalog -/

storage_catalog -X-> project_sync domain
```

Frontend dependency direction:

```text
project-sync UI  --> sync-review <--  storage-catalog UI
storage-catalog UI -X-> project-sync UI, models, API, or DTOs
```

There are no compatibility re-exports or duplicate old/new command families.

The tree describes ownership boundaries, not permission to scaffold empty
files. Every listed production module must own behavior used by a registered
command or rendered route. Combine adjacent modules when a boundary would
otherwise be a pass-through, and update this plan to match; do not leave empty
`mod.rs` exports, future-provider stubs, or one-line forwarding layers.

The `sync-review` layer is an extraction of current resource-neutral
presentation and interaction behavior. Both `project-sync` and
`storage-catalog` consume it through their own view-model adapters. It imports
neither project DTOs nor catalog DTOs and owns no persistence, planning, or
provider behavior. The three listed shared files must each have live
production callers in both features. Add no further shared file until two
concrete production consumers require the same semantics.

## 3. Phase 0 — freeze provider contracts

Before mutation code:

1. Record the supported Codex and Claude versions used for fixtures.
2. Prove current Codex global skill discovery from both
   `<CODEX_HOME>/skills` and `$HOME/.agents/skills`, including duplicate names
   across the two roots, a non-default `CODEX_HOME`, and the fact that the
   shared-user root is inactive for a Claude-only binding.
3. Prove Claude user-global skill discovery from
   `<CLAUDE_CONFIG_DIR>/skills`.
4. Record bounded Codex and Claude plugin inventory fixtures:
   - qualified canonical plugin ID and provider-native marketplace/source
     namespace;
   - native installation scope and ownership;
   - marketplace/source identity;
   - observed version/ref;
   - install root;
   - exported skills;
   - local/credential-bearing source classification; and
   - enablement/disablement observations.
5. Prove exact native user-global plugin install and overwrite/update argv,
   working-directory independence, and environment behavior, including source
   replacement and exact-version/ref limitations. Prove that Claude `project`,
   `local`, and managed scope and Codex workspace-provided, administrator,
   default, or otherwise non-user-owned entries are excluded.
6. Prove post-install and post-overwrite inventory verification and
   partial-result cases.
7. Record Codex pet fixtures:
   - valid v1 PNG;
   - valid v1 WebP;
   - valid v2 PNG;
   - valid v2 WebP;
   - canonical v1 manifest with no version marker;
   - explicit v1 marker, to determine whether Codex accepts or emits it;
   - v1 atlas paired with a v2 marker;
   - v2 atlas paired with a v1 marker;
   - two formats with the same pet ID producing different resource IDs;
   - a manifest containing both formats live for the same pet ID;
   - independent removal of either stored format;
   - Pull selection of v1, v2, or neither for their shared local target;
   - invalid simultaneous Pull selection of both formats for that target;
   - unknown version marker;
   - unknown manifest field;
   - ID/directory mismatch;
   - missing atlas;
   - path traversal;
   - wrong image dimensions;
   - excessive encoded/decoded size;
   - extra package files;
   - a valid preservation copy with rewritten directory, ID, and display name;
   - copy-name and case-folded collisions; and
   - v1/v2 preservation without format or atlas conversion.
8. Record Agent Skill fixtures for:
   - valid baseline metadata;
   - provider-specific frontmatter;
   - directory/name mismatch;
   - plugin ownership;
   - executable content;
   - secret-shaped content;
   - symlink and special file;
   - case collision;
   - nested VCS metadata;
   - unstable reads;
   - a valid preservation copy with matching directory/frontmatter name;
   - a copy whose directory and frontmatter name differ;
   - bounded generated-name collisions;
   - ignored project trees at `.skill`, `.skills`, `.agent/ssd`,
     `.agents/skills`, `.claude/skills`, and `skills`; and
   - canonically equal and ancestor/descendant active skill roots.
9. Record semantic plugin-intent fixtures proving that observed auto-updates
   do not change intent, while an enforceable requirement or source change
   does.
10. Freeze the current observable thread contracts for Push, Pull, Refresh,
    and browse-row Fork:
    - Push inline review, selection controls, per-row storage-newer holds with
      selected safe rows still enabled, the single plan-derived Pull callout,
      single flight, progress, close, and outcome behavior;
    - Pull inline review, default **Fork and overwrite**, alternatives,
      approvals, empty-selection acceptance, progress, and outcomes;
    - Refresh's forced local reload plus storage comparison, shared busy state,
      request-staleness handling, and feedback-dismiss timing;
    - Fork's browse-only row action, busy state, completion outcome, and
      automatic post-action Refresh; and
    - every thread DTO, thread ID, fork field, conversation codec, Codex
      inventory refresh implementation, and native fork executor that the
      catalog must not import.
11. Prove separate v1 and v2 resource reconciliation, independent Push/Pull
    selection, and the shared target-collision path for local v1 versus storage
    v2 and local v2 versus storage v1. Selecting both formats for one local
    target must fail before mutation.
12. Convert the frozen contract sketches above into strict test types.

Phase 0 does not research or implement another provider. If a Codex or Claude
native operation cannot be proven, that operation is a typed blocker; it is
not replaced with guessed CLI syntax.

Phase 0 may copy behavioral test cases from thread sync, but it does not
extract a shared thread/catalog domain abstraction. The catalog remains
independently buildable and testable.

## 4. Phase 1 — extract storage core

Extract only genuinely shared transport behavior from the repository bundle
engine:

- bounded metadata responses;
- conditional and immutable put outcomes;
- verified staged bodies;
- ordered bounded-concurrency staging;
- local-folder lock and atomic write behavior;
- S3 conditional writes, pagination, ambiguity read-back, and inactivity
  timeout; and
- storage marker verification.

`StorageId`, storage connection descriptors, and transport errors used by both
features move to `storage_core` if they are currently project-owned. Catalog
code imports those neutral definitions only; it never imports them through
`project_sync`.

Move each extracted implementation; do not copy it. Update project sync to the
new neutral API and delete the original definition in the same change. Every
public `storage_core` item must have a production catalog caller and a
production repository caller. Behavior that has only one caller stays in that
feature instead of becoming a speculative shared abstraction. Do not retain
facade modules or compatibility re-exports at the former path.

Introduce closed key constructors:

```rust
enum StorageObjectKey {
    Marker,
    Repository(RepositoryObjectKey),
    Catalog(CatalogObjectKey),
}
```

Allowed roots are exactly:

```text
.mallard/_storage.json
.mallard/v1/repositories/<bundle-id>/...
.mallard/v1/catalog/...
```

Tests:

- all existing repository local/S3 tests remain green;
- repository and catalog constructors reject each other's keys;
- catalog keys cannot escape their root;
- local symlink/unsafe-ancestor cases fail closed;
- repository listing never returns catalog objects; and
- storage marker behavior remains idempotent.

## 5. Phase 2 — domain and local state

### 5.1 Portable types

Add strict schema-1 types for:

- catalog head, manifest, commit, and tombstone;
- catalog, catalog-commit, catalog-operation, catalog-plan, plan-action, and
  resource IDs;
- file entries and immutable object references;
- Agent Skill descriptor with the closed target scopes `codex_home`,
  `codex_shared_user`, and `claude_user`;
- qualified plugin install intent, enforceable requirements, and closed
  portable sources;
- distinct `codex_pet_v1` and `codex_pet_v2` resource variants over one shared
  validated pet descriptor;
- provider key validation;
- content digests; and
- portable origin tokens with catalog-specific HMAC domain separation.

Keep native plugin observations separate from portable intent. Observed
version/ref, install root, exported skills, enablement, and provider
resolution output are machine-local diagnostics and do not enter the semantic
intent digest unless an exact requirement is native-enforceable.

Do not reuse:

- `BundleId`, `BundleKind`, or `BundleManifest`;
- project `ResourceDescriptor` or `LogicalPath`;
- `RestorePlan`; or
- thread plan/action types, thread IDs, fork lineage, conversation codecs, or
  the native thread-fork executor; or
- removed dependency types.

Do not collapse v1 and v2 into one resource identity. Each kind has its own
resource ID, portable lifecycle, and Push/Pull selection. Keep one pet adapter,
one target-conflict action family, and one Pets UI; do not duplicate
reconciliation engines, executors, or screens by format.

Add closed catalog-specific reconciliation and action types. Invalid
combinations must be unrepresentable: only skills and pets can use
`preserve_local_as_copy_and_overwrite`; only plugins can use
`overwrite_plugin_with_storage`.

Add a machine-local `CatalogForkPlan` for one source skill/pet, one exact
generated identity, and one named-copy action. It is not a portable manifest
type and cannot contain a plugin or storage-only source.

Add one validated per-resource base type with an optional accepted head and a
closed `absent`, `live(content_sha256)`, or
`tombstone(tombstone_sha256)` state. A missing accepted head is valid only for
`absent` and means the binding has never accepted that resource. Push can
advance the binding's known head while retaining an older resource base; no
separate pending-Pull collection is stored.

All portable structs reject unknown fields and unsupported versions. Integrity
fields receive no permissive defaults.

### 5.2 Local persistence

Add bounded atomic documents:

```text
~/.mallard/catalog_profiles.json
~/.mallard/storage_catalog.json
~/.mallard/catalog_plans/<plan-id>.json
~/.mallard/catalog_materializations.json
~/.mallard/catalog_preservations.json
~/.mallard/catalog_operations.json
~/.mallard/backups/catalog/<plan-id>/...
~/.mallard/locks/provider-<canonical-path-sha256>.lock
```

Each document has one live responsibility:

| Document | Responsibility |
|---|---|
| `catalog_profiles.json` | Codex/Claude machine-local profile definitions |
| `storage_catalog.json` | Per-storage binding, desired set, known head, per-resource accepted bases, and revision |
| `catalog_plans/` | Active immutable Push/Pull/Fork plans and supersession status |
| `catalog_materializations.json` | Verified per-target receipts and cross-storage claim edges |
| `catalog_preservations.json` | Validated named-copy witnesses used for idempotent recovery |
| `catalog_operations.json` | In-flight directory/plugin/Fork operation journal and terminal results |
| `backups/catalog/` | Permission-restricted directory backups owned by journaled operations |
| `locks/` | Canonical-target mutual exclusion |

Do not create another local document for the same responsibility. If the
implementation combines two responsibilities atomically, remove the unused
planned file and update this plan rather than leaving an empty store.

Local profile records own configured absolute paths. Portable documents never
contain them.

Storage deletion removes only local catalog state. Storage identity changes
clear the known head, per-resource accepted bases, and pending plans. Profile
deletion is blocked while referenced.

Tests:

- registry keys are exactly `codex` and `claude`;
- unsupported provider profile creation fails;
- strict schema and bounded collections;
- no local/project/storage/profile ID in portable fixtures;
- stable resource IDs and changing content digests;
- v1 and v2 with the same pet ID have different resource IDs but the same
  local target key;
- a manifest accepts live v1 and v2 resources with the same pet ID;
- either stored format can be removed without removing the other;
- Codex skill root class participates in identity, so equal names in
  `<CODEX_HOME>/skills` and `$HOME/.agents/skills` remain distinct;
- provider-qualified plugin IDs distinguish native namespaces;
- observed plugin updates leave semantic intent unchanged;
- enforceable requirement and source changes alter semantic intent;
- exact catalog logical-path grammar;
- strict, bounded preservation witnesses;
- per-resource base validation, including `None` only with `absent` and a
  known-head advance that retains an older held base;
- atomic persistence and revision conflicts;
- malformed state quarantines and fails safely;
- storage/profile cleanup; and
- no old capability, dependency, plugin-lock, or receipt document is read.

## 6. Phase 3 — inventory

### 6.1 Profile resolution

Implement catalog-specific resolvers for:

- Codex `CODEX_HOME`;
- Claude `CLAUDE_CONFIG_DIR`; and
- the OS user's fixed global `~/.agents/skills` root.

Every inventory, plan, and apply reloads selected profile records, validates
their revisions, resolves the OS user home independently, canonicalizes each
permitted global path, and rejects unsafe types. No project binding, checkout,
working directory, or repository search is used to discover a skill root.
The fixed `$HOME/.agents/skills` root is resolved only for a selected Codex
profile. If any active skill roots are equal or have an ancestor/descendant
relationship after canonicalization, return the typed
`overlapping_skill_roots` blocker before inventory.

### 6.2 Provider adapters

Implement Tauri-free Codex and Claude adapters for:

- bounded native plugin inventory parsing;
- user-global scope and ownership filtering before resource construction;
- provider-qualified canonical plugin identity;
- portable source classification;
- exact-version/ref capability;
- install-root and exported-skill ownership;
- typed native install and overwrite/update planning;
- target-local disablement;
- post-install/overwrite re-inventory; and
- verification.

Reuse Codex executable discovery only if its behavior remains unchanged for
project sync. Keep adapter process execution structured and shell-free.

### 6.3 Ownership-ordered skill inventory

For the permitted roots of the selected provider profiles:

1. inventory plugins;
2. build plugin ownership;
3. enumerate immediate skill directories without following links;
4. parse bounded `SKILL.md`;
5. validate declared/effective name and install directory;
6. detect runtime-name and case-folded target collisions;
7. classify standalone/plugin-owned/blocked;
8. compute warnings and source stamp; and
9. capture only after plan revalidation.

Tests:

- Codex scans only `<CODEX_HOME>/skills` and the fixed global
  `$HOME/.agents/skills` root;
- duplicate Codex names across those roots remain separately identified,
  labeled, selected, captured, and restored;
- a Claude-only binding never scans `$HOME/.agents/skills`;
- equal or nested active skill roots block before inventory and cannot create
  two identities for one physical target;
- Pull preserves Codex root class and never relocates a skill between
  `<CODEX_HOME>/skills` and `$HOME/.agents/skills`;
- Claude scans only `<CLAUDE_CONFIG_DIR>/skills` for the selected profile;
- project-local roots are never scanned, including fixture trees named
  `.skill`, `.skills`, `.agent/ssd`, `.agents/skills`, `.claude/skills`, and
  `skills`;
- `skills.config.path` entries outside the two Codex roots are not followed,
  and skill enablement is not transferred;
- `<CODEX_HOME>/skills/.system`, bundled skills, and plugin-owned entries are
  excluded;
- plugin-owned skills are not payload resources;
- name/hash equality alone is not ownership;
- exact skill bytes round-trip without conversion;
- conflict-copy generation rewrites only the directory and `SKILL.md`
  frontmatter name, then validates the copy independently;
- generated and user-edited names are provider-valid, bounded, and checked
  against exact and case-folded local/remote/claim targets;
- provider-applicability warnings from validated frontmatter are
  deterministic;
- symlink/special/case/VCS/size/secret/executable cases fail or warn as
  designed; and
- source changes invalidate pending plans.

### 6.4 Codex pet inventory

Implement exact v1 and v2 parsing into the distinct `CodexPetV1` and
`CodexPetV2` resource variants over one shared validated descriptor. Add image
header/decoder validation, canonical descriptor serialization, and extra-file
warnings. The resource variant is the portable format discriminator. Native v1
marker handling is limited to the exact shape proven in Phase 0.

Tests prove:

- PNG and WebP v1 round-trip at exactly `1536x1872`;
- PNG and WebP v2 round-trip at exactly `1536x2288`;
- v1 uses `8 x 9` and v2 uses `8 x 11` cells of `192x208`;
- exact ID/path rules;
- bounded decoded allocation;
- marker/dimension mismatches block;
- unknown versions and unknown fields block;
- v1 and v2 with the same pet ID reconcile as different resource IDs;
- both formats can coexist in storage and remain independently selectable;
- Pull permits v1, v2, or neither for a shared target and rejects selecting
  both before mutation;
- local-v1/storage-v2 and local-v2/storage-v1 use the shared pet target
  conflict resolution;
- restore preserves the selected format without conversion;
- conflict-copy generation rewrites directory, manifest ID, and proposed
  display name while preserving exact format and atlas bytes;
- copied pet names and IDs pass bounded collision validation;
- Claude exposes no pet capability; and
- no provider other than Codex is probed for pets.

## 7. Phase 4 — remote catalog and Push

Implement:

- fixed catalog-head reads;
- exact bounded manifest resolution;
- immutable object/manifest/commit publication;
- first-publisher catalog ID establishment;
- semantic no-op detection;
- CAS publication;
- ambiguous-response read-back;
- bounded history; and
- local/S3 parity.

Implement catalog-specific three-way reconciliation over each resource's exact
accepted base, local inventory, and the current head. Reuse the thread
workflow's policy shape, not its types or implementation:

- a `storage_changed` or `changed_both` resource is unselectable and held for
  Pull, while selected non-conflicting resources still Push;
- held and unselected remote resources are carried from the current remote
  manifest byte-for-byte, without a content merge;
- a held resource keeps its older accepted base after another resource Pushes
  successfully, so the next comparison derives the hold without duplicate
  persisted pending state;
- safe Push changes desired membership only for selected publications and
  approved non-held removals; held membership is unchanged, and a newly seen
  unreviewed remote resource is retained without becoming desired;
- a remote-head change after plan creation stales the complete plan and
  performs no mutation;
- catalog continuity replacement or loss blocks the complete Push;
- `changed_both` is digest-based and never uses mtime;
- local absence does not mean deletion; and
- removal remains an explicit Push action and is unavailable for held
  resources.

`prepare_storage_catalog_push` pins:

- storage and connection identity;
- binding/profile revisions and resolved skill-root identities;
- current head or exact absence;
- per-resource accepted bases;
- held resource IDs and their exact current-remote descriptors;
- inventory source stamp;
- desired IDs;
- retained current-remote resources;
- explicit removals;
- warning digests; and
- expiry.

Push planning creates one selectable row per exact local pet resource ID and
format. Publishing one kind retains a stored resource of the other kind.
Removing either stored kind requires its own explicit removal selection.

`publish_storage_catalog`:

1. reloads every pin;
2. captures selected skills and pets;
3. generates typed plugin intent;
4. retains held and unselected resources from the exact current-remote
   manifest;
5. writes explicit tombstones;
6. publishes immutable data and CASes the head; and
7. updates the desired set, known head, and bases for resolved resources only
   after success, while retaining older bases for held resources.

Run every flow through local-folder and stub-S3 storage:

- fresh publication;
- concurrent first publisher;
- no-op;
- add/update/remove skill;
- add/update/remove plugin intent;
- add/update/remove pet;
- v1 and v2 with the same pet ID can both remain live in storage;
- each format can be selected for Push and removed independently;
- publishing one format does not tombstone the other;
- v1-only and v2-only Push selections publish exactly the selected resource
  while retaining an existing unselected other-format resource;
- unavailable selected resource retained from the current remote manifest;
- explicit tombstone;
- an unreviewed remote resource is held while a selected local-only resource
  still Pushes;
- a disjoint remote advance holds only the affected resource and selected safe
  resources still Push;
- a same-resource remote advance is held and cannot Push or be removed until
  Pull accepts or resolves it;
- a safe Push carries held remote bytes unchanged and does not update that
  resource's accepted base;
- safe Push leaves each held resource's desired membership unchanged;
- a head change after preparation stales the complete plan;
- catalog identity replacement or loss blocks the complete Push;
- `changed_both` never produces a Push-side fork, copy, or merge;
- stale inventory or warning acknowledgement;
- corrupt/missing object;
- manifest hash/size mismatch;
- CAS loss and ambiguous success; and
- unchanged repository heads/listing.

## 8. Phase 5 — Pull and apply

### 8.1 Plan

Use one `CatalogPullPlan` with typed actions:

```text
install_skill
update_skill
preserve_skill_as_copy_and_overwrite
overwrite_skill_with_storage

install_plugin
update_plugin
overwrite_plugin_with_storage

install_pet
update_pet
preserve_pet_as_copy_and_overwrite
overwrite_pet_with_storage
```

Every action pins one resource, target, source digest, current target state,
approval requirement, and expiry.

Each review item contains either one executable typed action or a non-mutating
state: `already_matching`, `kept_local`, `manual`, or `blocked`. `skip` records
the `skipped` outcome; it is not an executor variant. Do not register no-op,
manual, or skip handlers in the apply dispatcher.

Represent Pull pet selection with one closed choice per target:

```text
do_not_install
materialize_v1(exact planned codex_pet_v1 resource, exact planned action)
materialize_v2(exact planned codex_pet_v2 resource, exact planned action)
```

The apply request cannot carry both materialization choices for one target.
The action reference is absent only when the chosen resource already matches;
otherwise it identifies one exact install, update, preserve-copy, or direct
overwrite action. `do_not_install` is the only skip form. Reject a resource or
action ID whose kind or target does not match its planned choice. The
user-facing pet conflict choice **Skip** serializes as `do_not_install`, not as
an executable action.

Plan resource relations from the exact accepted base:

- matching resources produce no mutation;
- one-sided storage changes produce ordinary install/update actions;
- one-sided local changes remain local;
- `changed_both` skills and same-format pets default to
  `preserve_*_as_copy_and_overwrite`, with direct overwrite and skip as
  alternatives;
- a v1/v2 incoming pet whose target is occupied by the other pet resource kind
  uses the same pet preservation, direct-overwrite, or skip choices and is not
  classified as cross-format `changed_both`;
- when storage contains both formats for one pet ID, Pull exposes a mutually
  exclusive v1/v2/neither materialization group; and
- `changed_both` plugins offer `overwrite_plugin_with_storage` or skip only.

For skill/same-format-pet divergence and cross-format pet target collisions,
server-side planning proposes a valid bounded `<name>-local-copy` identity and
numeric suffix when required. It rejects exact and case-folded collisions
across local inventory, remote resources, tombstones, claim targets, and active
plan reservations. The review can submit an edited name to
`prepare_storage_catalog_pull`, which returns a new immutable plan and
supersedes the old one; the client never edits action bytes in place.

The preservation action additionally pins:

- original identity, target, and local digest;
- current storage resource identity and digest;
- generated name, resource ID, and target;
- expected absence of the generated target;
- digest of the fully transformed copy;
- exact head, binding/profile revisions, resolved skill-root identities, plan
  digest, and expiry; and
- an existing matching preservation witness, when one is reusable.

Skill transformation rewrites directory and frontmatter name together. Pet
transformation rewrites directory, manifest ID, and proposed display name
together while preserving the source v1/v2 resource kind and exact atlas.
Apply never recomputes a name or performs a raw recursive copy.

### 8.2 Accept before mutation

On confirmation:

1. verify remote head and every local pin;
2. persist the complete accepted live-resource set, current known head, and
   current live/tombstone state as every resource's new accepted base;
3. acquire canonical target locks;
4. revalidate each target;
5. execute only approved actions; and
6. record verified per-action receipts and failures.

This ordering prevents skipped or failed local actions from becoming remote
deletions during a later Push.

### 8.3 Directory transaction

Implement one target-safe directory replacement primitive for ordinary skill
and pet installs/updates:

- same-filesystem stage;
- full path/hash/size/mode verification;
- free-space check;
- expected target digest;
- restricted journal and backup;
- atomic rename activation;
- final verification;
- rollback when safe; and
- startup recovery.

The primitive accepts a validated materialization target, never an arbitrary
portable path.

Add one composite preservation transaction:

1. revalidate every source, target-absence, head, revision, digest, and expiry
   pin;
2. stage and validate the transformed local copy;
3. stage and validate the storage version for the original local target;
4. prove free space for both stages, the preserved copy, and backup;
5. persist a journal with the immutable operation ID and chosen name;
6. activate the copy with no-clobber semantics and verify it;
7. persist a preservation witness;
8. back up and replace the original with the storage version; and
9. verify both targets, then complete the receipt.

Recovery always resumes the same operation and name. If the copy exists with
the witnessed digest, it is reused. If it is missing or changed, recovery
fails closed for fresh review. A failure after copy creation but before
original overwrite must not create a second copy on retry.

### 8.4 Plugin executor

For every approved plugin:

- re-inventory;
- recheck qualified identity, semantic intent, source, prerequisites,
  ownership collisions, version limitations, and local disablement;
- invoke exact structured native install/update arguments without a shell;
- capture bounded sanitized output;
- re-inventory and verify; and
- report `partial` when native state is uncertain.

For plugin divergence, this executor is the only mutation path. It overwrites
through the native provider operation or returns manual/blocked. It never
renames or copies a plugin, copies its payload/cache, or uninstalls before
replacement.

Never uninstall after remote removal or storage detach.

### 8.5 Catalog resource Fork

Fork is available only from a local standalone skill or local pet row in
Library browse mode. It is absent for plugins, storage-only resources, blocked
resources, and every Push or Pull selection mode.

`prepare_catalog_resource_fork`:

1. accepts the exact local catalog resource ID and a user-entered name;
2. validates the source, profile/root revision, source digest, provider naming
   rules, target absence, exact and case-folded collisions across local
   inventory, current remote resources, tombstones, active claims, and active
   plan reservations, plus expiry;
3. rewrites a skill directory and `SKILL.md` frontmatter name together, or a
   pet directory, manifest ID, and display name together;
4. preserves the skill scope or exact `CodexPetV1`/`CodexPetV2` resource kind
   and pet atlas bytes; and
5. returns an immutable, digest-pinned Fork plan.

`apply_catalog_resource_fork` reuses the same named-copy transformer, staging,
no-clobber activation, verification, journal, and recovery code used by Pull
preservation. It stops after the copy is verified and records a terminal
catalog-Fork receipt; it does not overwrite another target or create a Pull
preservation witness. The new resource is local-only until a later explicit
Push.

This operation never invokes the native thread-fork executor, creates thread
lineage, or uses a raw recursive copy. After a successful or recovered Fork,
the UI automatically runs catalog Refresh. That automatic Refresh preserves
the new Fork outcome; a later user-initiated Refresh dismisses it only after
both local inventory and storage comparison finish successfully.

### 8.6 Apply and Fork tests

- matching-without-mutation, install, update, and direct-overwrite skill and
  pet paths;
- default skill/pet preserve-copy-and-overwrite;
- user-edited generated copy name causes a new pinned plan;
- the superseded pre-edit plan cannot apply;
- skill directory/frontmatter names remain equal after copy;
- pet directory/manifest IDs remain equal and v1/v2 atlas bytes remain exact;
- v1 and v2 resource IDs remain distinct through plan and receipt;
- local-v1/storage-v2 and local-v2/storage-v1 target conflicts use the shared
  pet preservation action and preserve each side's resource kind;
- both formats may remain live in the accepted catalog while only the selected
  one materializes at the shared local target;
- simultaneous v1 and v2 materialization selection fails before mutation;
- `do_not_install` accepts both stored resources without changing the local
  pet target;
- a forged or mismatched pet resource/action choice fails plan validation;
- exact, case-folded, local/remote/tombstone/claim name collisions;
- generated target race fails no-clobber before original overwrite;
- explicit direct overwrite and target digest;
- target race after review;
- failure injection before copy, after copy, after witness, after backup, after
  original activation, and during final verification;
- startup recovery reuses the same name and never creates repeated copies;
- invalidated preservation witnesses require fresh review;
- unsafe target and insufficient space;
- stale storage/binding/profile revisions or changed resolved skill root;
- Codex and Claude plugin idempotence;
- exact argv/environment;
- user-global plugin scope is explicit and independent of the process working
  directory;
- Claude project/local/managed and Codex non-user-owned plugins are excluded;
- plugin divergence exposes overwrite or skip only;
- plugin overwrite uses native update/install without payload copy or
  uninstall;
- provider-observed auto-update does not create semantic divergence;
- source conflict and enforceable version limitation;
- target disablement;
- partial verification and retry;
- same target claimed by two storages;
- skipped/failed actions preserve accepted desired state;
- remote removal performs no local deletion/uninstall; and
- unrelated action failures preserve truthful results;
- manual skill Fork rewrites directory/frontmatter together;
- manual v1 and v2 pet Forks retain their exact resource kind and atlas;
- user-entered Fork names are plan-pinned, bounded, and no-clobber;
- local, current-remote, tombstone, claim, and active-plan name collisions
  reject Fork before mutation;
- stale source/name/profile/root plans fail before copy activation;
- recovery resumes the same Fork operation and never creates a second copy;
- Fork is unavailable for plugins and storage-only resources;
- Fork never calls a thread command, codec, or native fork executor; and
- successful Fork triggers Refresh without dismissing its completion outcome.

## 9. Phase 6 — Tauri and frontend

### 9.1 Commands and DTOs

Register exactly this catalog-owned command family:

```text
list_catalog_capabilities
list_catalog_profiles
save_catalog_profile
delete_catalog_profile
get_storage_catalog_binding
save_storage_catalog_binding
inspect_storage_catalog
prepare_storage_catalog_push
publish_storage_catalog
prepare_storage_catalog_pull
get_storage_catalog_plan
apply_storage_catalog_pull
prepare_catalog_resource_fork
apply_catalog_resource_fork
get_storage_catalog_status
```

Route every frontend call through `storageCatalogApi`, and mirror Rust
contracts in `src/components/storage-catalog/types.ts`. Import only genuinely
storage-neutral identifiers from shared types; do not add catalog unions or
fields to project DTOs in `src/types.ts`.

| Library operation | Command path |
|---|---|
| Push | `prepare_storage_catalog_push` → `publish_storage_catalog` |
| Pull | `prepare_storage_catalog_pull` → `apply_storage_catalog_pull` |
| Refresh | `inspect_storage_catalog(forceRevalidate: true)` |
| Fork | `prepare_catalog_resource_fork` → `apply_catalog_resource_fork` |

The remaining commands also require concrete production callers:

| Commands | Caller |
|---|---|
| `list_catalog_capabilities`, `list_catalog_profiles` | Library setup, navigation capability filtering, and Settings |
| `save_catalog_profile`, `delete_catalog_profile` | Settings create/edit/delete actions |
| `get_storage_catalog_binding`, `save_storage_catalog_binding` | Library setup and profile pairing |
| `get_storage_catalog_plan` | Restart/recovery flow for an operation status that references an exact still-valid plan |
| `get_storage_catalog_status` | Library startup, in-flight progress, and recovery affordances |

Refresh does not get a second command alias.
`inspect_storage_catalog(forceRevalidate: true)` is the single backend
entrypoint for a user-requested catalog Refresh; normal browse loading may use
`false`. The API must use a typed boolean argument rather than separate
old/new inspection paths.

Command tests prove:

- capability output is exactly Codex skills/plugins/pets and Claude
  skills/plugins;
- no command accepts a project or bundle ID;
- every catalog command works without loading the project repository or
  registering a project;
- forced inspection bypasses inventory caches and returns one coherent local
  inventory/storage comparison snapshot;
- Fork commands accept only local skill/pet resources and never dispatch a
  thread or plugin operation;
- unsupported provider profile creation fails; and
- project command DTOs remain unchanged.

### 9.2 Extract shared review primitives

Before building Library review screens, extract only three resource-neutral
primitives from these current project files:

```text
src/components/project-sync/PushResourceWorkspace.tsx
src/components/project-sync/RestorePlanView.tsx
src/components/project-sync/SyncReviewTabs.tsx
src/components/project-sync/SyncOutcomeNotice.tsx
```

1. move and generalize `SyncReviewTabs`, retaining arrow/Home/End keyboard
   behavior, counts, and warning indicators;
2. extract the duplicated selected/total, recommended-selection, and clear
   controls into `SyncReviewSelectionBar`; and
3. move and generalize `SyncOutcomeNotice` into a resource-neutral expandable
   outcome renderer.

Give these primitives neutral props based on presentation view models and
callbacks. Do not add catalog variants to `ProjectResourceDescriptor`,
`RestorePlan`, `ProjectBinding`, `PullReviewItem`, conversation resolutions,
or project-file models. Do not make the shared components invoke Tauri
commands or understand storage/catalog semantics.

Migrate the current project screens to the extracted primitives in the same
change. Preserve their user-visible behavior, accessibility, focus handling,
and integration tests. Delete replaced project-only presentation code rather
than retaining wrappers, copied markup, parallel CSS, or feature-gated dead
code.

Reuse the rest of the current user journey as an interaction contract and
shared styling, while keeping its domain composition in the owning feature:
inline review placement, plan warnings above the list, selection blockers
beside the primary action, storage-card/footer gating, expandable resource
details, progress, and close-to-outcome behavior. Do not extract a generic
workspace, item list, row extension framework, or primary-action bridge unless
two concrete production callers first demonstrate identical semantics. A
catalog-owned row renderer is preferable to an option-heavy shared component.

After moving a component, update all production and test imports to the new
path and delete the old file. Do not leave forwarding modules or compatibility
re-exports.

Do not extract these project-specific behaviors:

- conversation history and thread identity;
- native thread-fork checkpoints or Codex history/comparison loaders; catalog
  Refresh matches their observable lifecycle through catalog-owned calls;
- project-file eligibility, directory dependencies, or deletion rules; or
- project binding, repository continuity, or project outcome copy.

### 9.3 Storage Library UI

Add:

- Library overview;
- Codex profile selector;
- Claude profile selector;
- Skills, Plugins, and Pets tabs;
- Push review;
- Pull/install review;
- catalog Refresh;
- skill/pet Fork;
- per-action results; and
- storage/catalog settings.

Do not add another provider selector or generic provider entry field.

Build catalog-specific row and outcome view models over the shared primitives:

- tabs are Skills, Plugins, and Pets, hiding a category unsupported by the
  selected profile set;
- Push reuses selected/total, recommended, clear, warnings, confirmation,
  progress, and outcome flow;
- storage-newer or divergent rows are held and unselectable, reuse one
  plan-derived **Storage has newer versions. Selected resources still push.**
  callout and **Open Pull review** transition, and leave Push enabled for
  selected safe rows;
- Pull reuses expandable review rows, selectable only when the plan permits,
  plus empty-selection acceptance, primary-action gating, and truthful
  per-action results;
- skill/pet conflicts reuse the current row-local radiogroup and destructive
  acknowledgement interaction, with catalog labels and the editable generated
  name;
- plugin conflicts use the same row-local interaction placement but expose
  overwrite and skip only; and
- results reuse the compact expandable outcome, with catalog resource names
  and no thread IDs, thread-fork counts, or Codex-app restart actions.

### 9.4 Push, Pull, Refresh, and Fork parity

The Library matches the current thread interaction contract for Push, Pull,
and Refresh:

- **Push:** the storage-card action opens one inline review; tabs, selected
  count, recommended selection, clear, warning placement, primary-action
  gating, single-flight progress, close behavior, and compact outcome behave
  the same. Storage-newer and divergent rows are held and unselectable, their
  current remote versions remain untouched, selected safe rows still Push, and
  one plan-derived **Storage has newer versions. Selected resources still
  push.** callout exposes **Open Pull review**. A CAS race after review or a
  catalog continuity failure still blocks the complete operation. A completed
  Push closes review into the Library outcome and refreshes catalog data
  without hiding that outcome.
- **Pull:** the storage-card action opens one inline review; resource decisions
  are row-local; the safe default is **Fork and overwrite** for a divergent
  skill or pet; alternatives are **Overwrite** and **Skip**; empty selection
  can still accept the reviewed storage state; progress and truthful
  per-resource outcomes match the thread flow. Once an apply result exists,
  review closes into the Library outcome, catalog data reloads, and any retry
  or **Open Pull review** action replans from current state.
- **Refresh:** Library browse mode shows the same refresh icon, disabled/busy
  state, accessible status, and error treatment. One Refresh action starts
  forced provider-inventory and storage-comparison loads as one lifecycle; the
  catalog endpoint may return them as one coherent snapshot. A context/request
  sequence discards late responses after storage, profile, binding, or route
  changes. Refresh is read-only, invalidates stale catalog plans, and never
  silently applies a previous selection.
- **Refresh feedback:** user-initiated Refresh dismisses prior transient Fork
  feedback only after the forced inventory and comparison both succeed.
  Automatic Refresh after Fork keeps the new Fork outcome visible.

As in the thread UI, browse-only row actions are hidden in Push and Pull
selection modes. There is no second Refresh button inside a review; a stale
review closes or regenerates its immutable plan and asks the user to review
again.

Catalog **Fork** uses the thread row-action placement, spinner, completion
feedback, recovery affordance, and automatic Refresh shape. Its mechanics
remain catalog-specific:

- it appears only for local standalone skills and pets;
- it asks for and plan-pins the new valid name before mutation;
- it creates a verified named local copy in the same skill scope or pet format;
- it is local-only until explicitly selected by a later Push; and
- plugins never display or accept Fork.

Frontend integration coverage:

- current project Push/Pull flows remain behaviorally unchanged after
  extraction;
- shared tabs retain mouse, ArrowLeft/ArrowRight, Home/End, focus, count, and
  warning semantics;
- storage selection opens Library;
- explicit profile pairing;
- local/storage/matching/diverged/blocked status;
- Push selection persistence;
- storage-newer and divergent rows are held and route to Pull review while
  selected safe rows remain Pushable;
- the held rows remain unselectable and their current remote bytes and older
  accepted bases survive a safe Push;
- a post-review head race or catalog continuity failure blocks the complete
  Push;
- completed Push and Pull reviews close into a truthful outcome, reload current
  catalog data, and keep the outcome visible;
- skipped Pull item remains desired after acceptance;
- skipped items render a truthful `skipped` outcome rather than success;
- divergent skill/pet review defaults to **Fork and overwrite**, shows
  both resulting identities, and permits a validated editable copy name;
- **Overwrite** and **Skip** alternatives for skills/pets;
- divergent plugin review offers **Overwrite plugin** and **Skip** only;
- Push review exposes separately selectable **v1** and **v2** rows;
- Pull review offers v1, v2, or **Do not install** for resources sharing one
  local pet target and rejects simultaneous materialization;
- plugin execution approval discloses native provider-managed execution;
- a preserved copy is labeled local-only until a later Push;
- a name edit regenerates the immutable plan instead of mutating it;
- browse Refresh force-reloads local inventory and storage comparison, shows
  one busy state, and ignores stale responses;
- user Refresh dismisses old Fork feedback only after full success;
- post-Fork Refresh preserves the new completion outcome;
- skill and pet browse rows expose Fork, while plugin and storage-only rows do
  not;
- Fork is hidden in Push and Pull selection modes;
- a successful skill/pet Fork appears as a local-only named resource eligible
  for a later Push;
- Library Push/Pull use the shared tabs, selection controls, and outcome
  renderer plus the same warning placement, progress treatment, and review
  journey as project sync;
- catalog view models cannot import or construct project/thread DTOs;
- Codex v1 and v2 pet previews and unknown-version blocked state;
- v1 and v2 rows carry distinct resource IDs and clear format labels;
- partial/failed result truthfulness;
- no unsupported provider selector; and
- no Library surface in project Push/Pull.

## 10. Phase 7 — clean-break audit

Before handoff:

1. Confirm project `ResourceKind`, recipes, manifests, restore actions, and
   readiness DTOs contain no skill/plugin/pet catalog variant.
2. Confirm project capture does not enumerate global customization roots.
3. Remove or correct current-looking documentation that says global
   customizations travel with a project.
4. Confirm no old dependency, capability, plugin-lock, receipt, or removed
   Mallard pet document is loaded.
5. Confirm only Codex and Claude provider adapters are registered.
6. Confirm no unsupported provider root appears in production inventory code.
7. Confirm there is no migration command, hidden import, serde alias, version
   fallback, or compatibility re-export.
8. Remove every superseded implementation path, including unused types,
   commands, adapters, parsers, feature flags, UI routes, tests, fixtures, and
   comments that describe the removed behavior.
9. Run compiler, lint, and source checks that reject dead code; do not silence
   unused-code diagnostics to make the cutover pass.
10. Confirm catalog reconciliation imports no thread DTO, thread ID, fork
    field, conversation codec, or native thread-fork executor.
11. Confirm no plugin action, UI control, or recovery branch copies, renames,
    or pre-emptively uninstalls a plugin.
12. Confirm no pet identity, reconciliation path, or receipt collapses v1 and
    v2 into one resource; both may be live in storage while local
    materialization remains mutually exclusive per target.
13. Confirm project and Library review screens both use the extracted tabs,
    selection bar, and outcome renderer; confirm the former component files
    and duplicated selection controls are deleted.
14. Confirm the shared review layer imports neither project nor catalog DTOs
    and contains no domain decisions; every exported primitive has live
    production callers in both features.
15. Confirm inventory never derives a root from a checkout, working directory,
    or project-local `.skill`, `.skills`, `.agent/ssd`, `.agents/skills`,
    `.claude/skills`, or `skills` tree.
16. Confirm canonically equal or nested active skill roots fail with
    `overlapping_skill_roots` before inventory.
17. Confirm every registered catalog command has a production frontend or
    startup-recovery caller and every exported frontend API method is used.
18. Confirm every compiled provider/resource adapter is reachable from the
    exact capability registry; there are no dormant variants for future
    providers or resource kinds.
19. Confirm catalog Push, Pull, and Refresh pass the frozen thread-behavior
    parity tests without importing project/thread DTOs or commands.
20. Confirm Fork is browse-only, local-only, available only for skills/pets,
    and implemented by the shared semantic named-copy primitive.
21. Confirm there is one forced catalog inspection path for Refresh and no
    alias command, duplicate cache bypass, or review-local refresh control.

## 11. Verification

Run focused checks during each phase, then:

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:backend-integration
npm run test:integration
```

Also run source checks proving:

```text
catalog domain imports no LocalProjectId or BundleId
storage_catalog backend imports no project_sync module
storage-catalog frontend imports no project-sync component, API, model, or DTO
catalog reconciliation imports no thread or native-fork implementation type
catalog Fork imports no thread command, codec, checkpoint, or executor
shared sync-review components import no project or catalog domain DTO
every shared sync-review export has project and catalog production callers
project manifests contain no catalog resource kind
project commands accept no catalog plan or selection
portable catalog data contains no local absolute path or local ID
portable plugin intent contains no observed native inventory fields
portable plugin intent has no selectable project/local/managed scope
catalog registry contains only codex and claude
codex_pet_v1 and codex_pet_v2 produce distinct resource IDs
catalog manifests may contain both pet formats for one pet ID
Pull rejects selecting both formats for one local pet target
no unsupported provider home is scanned
no project-derived skill root or independent skill-root picker exists
equal or nested active skill roots block before inventory
no old capability/dependency/plugin-lock document is loaded
no backward compatibility reader, alias, fallback, or dual-write path exists
no migration detector, importer, converter, or command exists
no plugin or storage-only resource exposes catalog Fork
no duplicate catalog Refresh command or inventory implementation exists
no obsolete implementation is compiled, registered, feature-gated, or
  retained as dead code
```

## 12. Done definition

Milestone 1 is complete only when:

- the exact support matrix passes backend and frontend tests;
- local-folder and stub-S3 behavior match;
- the catalog works with zero projects;
- project sync behavior remains unchanged and green;
- current project and new Library review flows share neutral UI primitives
  while retaining separate domain models;
- Library Push, Pull, and Refresh match the frozen observable thread behavior;
- the moved tab/outcome files and duplicated selection controls are removed
  from `project-sync`, with no forwarding wrappers or re-exports;
- no shared module or extension hook exists without two live production
  consumers;
- per-resource Push divergence requires Pull without blocking selected safe
  resources or implicitly merging content;
- skill/pet divergence defaults to a validated named local copy followed by
  storage overwrite of the original;
- user-edited preservation names are plan-pinned and no-clobber;
- direct skill/pet overwrite and plugin execution consent are tested;
- plugin divergence uses native overwrite/update or skip only;
- plugin observations do not cause false semantic conflicts;
- plugin inventory and apply are user-global only and never inherit project,
  local, managed, workspace, administrator, or default scope;
- startup recovery passes injected failures without creating repeated copies;
- manual skill/pet Fork is named, no-clobber, recoverable, local-only, and
  followed by Refresh without using the native thread-fork path;
- plugin rows never expose Fork;
- skipped, failed, and missing local resources cannot cause remote deletion;
- plugin removal never invokes uninstall;
- Codex pet v1 and v2 both round-trip without conversion;
- v1 and v2 are different resource IDs with independent Push/Pull selection;
- both formats share one target-conflict action family without a conversion
  path or duplicated executor;
- both formats may coexist in storage, but at most one materializes per local
  pet target;
- unknown Codex pet versions remain unsupported;
- only Codex and Claude profiles and selectors exist;
- Codex-home and shared-user skill roots are both covered without collapsing
  equal names or transferring enablement;
- no project-derived skill root is scanned or offered as a Library setting;
- equal or nested active skill roots block before inventory;
- no backward compatibility support or repository-scoped catalog path exists;
- no migration code exists;
- no superseded implementation remains as dead code; and
- current metadata documentation describes the implemented state accurately.
