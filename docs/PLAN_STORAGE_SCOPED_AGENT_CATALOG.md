# Implementation plan: Storage-scoped agent catalog

**Status:** proposed

**Date:** 2026-08-06

**Design authority:**
[`DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md`](./DESIGN_STORAGE_SCOPED_AGENT_CATALOG.md)

## 1. Delivery strategy

Implement the storage catalog as a new vertical slice with a strict generic
resource envelope, a closed runtime adapter registry, and six initial provider
keys: Claude, Cursor, Codex, OpenCode, Pi, and Gemini CLI. Share only the storage
transport, catalog-profile resolution, validation, and process-launch
primitives that are genuinely common.

Do not first restore the archived repository-scoped feature and then move it.
That would recreate obsolete project resource variants, dependency plans,
commands, and UI only to delete them again.

Every phase ends with the current project-sync suite green. Catalog tests are
added beside the new code and through the existing local/stub-S3 integration
harness.

## 2. Target module layout

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
    inventory.rs
    registry.rs
    provider_adapters/
      claude.rs
      cursor.rs
      codex.rs
      opencode.rs
      pi.rs
      gemini.rs
    format_adapters/
      agent_skill.rs
      extension_intent.rs
      codex_pet_v2.rs
    engine.rs
    planning.rs
    apply.rs
    persistence.rs
    commands.rs
    tests/
  project_sync/
    ... existing repository-only modules

src/components/
  storage-catalog/
    StorageAgentCatalogWorkspace.tsx
    StorageAgentCatalogOverview.tsx
    catalogRegistry.ts
    CatalogResourceList.tsx
    CatalogUnsupportedList.tsx
    StorageCatalogPushReview.tsx
    StorageCatalogPullReview.tsx
```

The exact file split may stay smaller while a module is short, but dependency
direction is fixed:

```text
project_sync ----\
                  > storage_core
storage_catalog -/

storage_catalog -> catalog-profile resolver
storage_catalog -X-> project bundle domain
```

No compatibility re-exports or duplicate old/new command families remain
after a move.

## 3. Phase 0 — lock contracts with fixtures

Before production mutation code:

1. Freeze authoritative user-global root and native-operation contracts for
   `claude`, `cursor`, `codex`, `opencode`, `pi`, and `gemini`. Record the
   provider version and official source for every contract.
2. Record bounded native extension inventory fixtures wherever a provider has
   a reproducible contract: marketplace/source metadata, exported-skill
   ownership, structured install arguments or official config transactions,
   and post-install verification.
   A missing or unstable contract becomes an explicit unsupported capability,
   not guessed behavior.
3. Record Agent Skill fixtures for shared and provider-specific roots covering
   declared names, audience overlap, executable files, extension ownership,
   broken links, case collisions, and unstable capture.
4. Add Codex pet fixtures:
   - valid v2 WebP;
   - valid v2 PNG;
   - mismatched directory/manifest ID;
   - missing atlas;
   - traversal in `spritesheetPath`;
   - wrong dimensions;
   - unknown manifest field;
   - extra package file; and
   - v1 manifest.
5. Prove each supported provider CLI argv/environment or declarative-config
   transaction with adapter tests. Do not encode a command or config shape
   known only from historical documentation.
6. Add generic-envelope fixtures for known formats, an unknown provider, an
   unknown format, byte-for-byte carry-forward, and blocked unknown-resource
   add/change/remove/apply operations.
7. Convert the design's type sketches into strict Rust types and validation
   tests before connecting filesystem or network code.

The archive at `docs/archive/skill-plugin-sync/` is reference evidence. Port
specific pure tests and verified adapter behavior; do not copy its project
resource, dependency-plan, command, persistence, or frontend layers.

## 4. Phase 1 — extract shared storage transport

### 4.1 Move shared primitives

Extract from `project_sync/bundle_engine.rs` and
`project_sync/s3_store.rs`:

- bounded metadata responses;
- conditional put results;
- immutable put outcomes;
- verified staged bodies;
- ordered bounded-concurrency staging;
- local-folder atomic writes and storage lock;
- S3/R2 conditional writes, pagination, read-back ambiguity handling, and
  chunk inactivity timeout; and
- storage marker verification.

Project bundle behavior must remain byte-for-byte compatible.

### 4.2 Replace the repository-only key parser

Introduce one closed storage-key enum or constructor family:

```rust
enum StorageObjectKey {
    Marker,
    Repository(RepositoryObjectKey),
    Catalog(CatalogObjectKey),
}
```

Repository and catalog relative-key validators remain separate. There is no
generic “any path under `.mallard`” constructor.

Allowed roots are exactly:

```text
.mallard/_storage.json
.mallard/v1/repositories/<bundle-id>/...
.mallard/v1/catalog/...
```

### 4.3 Phase tests

- all existing local and S3 bundle-engine tests;
- exact allow/deny tests for both key families;
- catalog keys cannot enter a repository constructor and vice versa;
- storage marker creation/validation remains idempotent;
- symlink traversal and unsafe ancestor tests cover the catalog root; and
- repository listing never returns catalog objects.

## 5. Phase 2 — catalog domain and local state

### 5.1 Portable types

Add strict schema-1 types for:

- `CatalogId` and `CatalogResourceId`;
- catalog head, manifest, commit, file entry, and tombstone;
- validated open `ProviderKey`, `CatalogFamilyKey`, `CatalogFormatKey`,
  `CatalogAudienceKey`, and `SharedRootKey` values;
- the strict generic `CatalogResourceEnvelope` and descriptor object
  reference;
- audiences and generic portable claims;
- typed Agent Skill, extension-intent, and Codex-pet adapter descriptors;
- portable extension source variants;
- content digests and extension-provided-skill claims; and
- catalog origin tokens with a distinct HMAC domain-separation string.

Do not reuse:

- `BundleId`;
- `BundleKind`;
- `BundleManifest`;
- project `ResourceDescriptor`;
- project `LogicalPath`;
- `RestorePlan`; or
- removed dependency actions.

Portable envelope structs reject unsupported catalog versions and unknown
fields. Fields needed for integrity do not receive permissive defaults. New
portable catalog documents use `#[serde(deny_unknown_fields)]`.

Provider and format keys are open identifiers, but executable behavior is
available only from the compiled registry. An unknown provider or format is a
verified opaque envelope: visible and carry-forward-only from the exact base.
It is never decoded as a known type or dispatched to a binary.

### 5.2 Local state

Add bounded atomic persistence for:

```text
~/.mallard/storage_catalog.json
~/.mallard/catalog_profiles.json
~/.mallard/catalog_plans/<plan-id>.json
~/.mallard/catalog_materializations.json
~/.mallard/catalog_operations.json
~/.mallard/backups/catalog/<plan-id>/...
~/.mallard/locks/provider-<canonical-path-sha256>.lock
```

The binding state references existing `StorageId` plus new
`LocalCatalogProfileId` and shared-root IDs. The catalog-profile document owns
the machine-local configured paths and stores no credentials. Resolver code
reloads and canonicalizes every selected root for every plan and apply.

Use revision checks, bounded documents, atomic rename/fsync behavior,
quarantine, and the existing cross-process persistence lock conventions.

### 5.3 Config integration

- Storage deletion cascades only machine-local catalog state.
- Storage connection identity changes clear the catalog base and pending
  plans.
- Catalog-profile and shared-root deletion check catalog bindings. Existing
  project-profile deletion behavior remains unchanged.
- Host identity rotation includes catalog origin aliases/receipts only if they
  contain the raw private host ID. Prefer bundle-like derived tokens so raw
  host identity does not enter catalog documents.

### 5.4 Phase tests

- strict schema and bounded-collection validation;
- no project/storage/profile IDs serialize into portable fixtures;
- stable resource IDs and changing content digests;
- shared-skill identity and digest stay stable as another compatible provider
  is paired;
- validated open keys do not create runtime dispatch;
- an unknown provider key cannot create a local catalog profile;
- an unknown format version is carry-forward-only and never dispatched through
  a nearby registered version;
- unknown envelopes are retained exactly and cannot be edited or applied;
- exact catalog path grammar;
- optimistic local revision failure;
- missing local catalog file initializes empty state;
- malformed state quarantines and fails safely;
- storage/profile cleanup rules; and
- no old dependency document is read.

## 6. Phase 3 — adapter registry and ownership-ordered inventory

### 6.1 Catalog profile resolution

Add a catalog-specific profile store and resolver that returns a validated
`ProviderKey`, user-selected path, canonical path, and current profile
revision. Add the explicit shared Agent Skills root resolver. Do not widen the
project-sync `Provider` enum or load a project binding to find a provider home.
The Codex and Claude catalog profiles may resolve to the same canonical paths
as project profiles; the common canonical-path lock provides coordination.

### 6.2 Provider adapters

Build Tauri-free, versioned adapters for Claude, Cursor, Codex, OpenCode, Pi,
and Gemini CLI. Every adapter declares supported format keys and owns:

- user-global root discovery;
- native extension inventory parsing when proven;
- extension ID canonicalization;
- portable source classification and managed registry resolution;
- installed source/version/ref inspection when available;
- extension-provided-skill claims;
- native process or typed config plan generation from intent; and
- post-operation verification.

Reuse `codex_binary.rs` for supported Codex executable discovery and extract a
small common binary-resolution primitive only where it preserves current
project behavior. Process invocation uses structured argv and the exact
adapter-specific local environment.

Unsupported provider versions and source types return typed blockers. They do
not fall through to guessed commands.

### 6.3 Format registry and Agent Skill inventory

Register the initial Agent Skill, extension-intent, and Codex-pet formats
explicitly. Assert duplicate `(format_key, version)` registration is fatal in
tests and production startup.

Scan the explicitly paired shared root once, then scan paired provider-global
roots through their adapters. After extension inventory:

1. build the extension install-root and exported-skill ownership index;
2. enumerate immediate `skills/` children with no-follow metadata;
3. deduplicate identical canonical physical roots;
4. parse bounded `SKILL.md` through the shared format adapter;
5. derive declared/effective names, portable audiences, effective providers,
   and generic claims;
6. validate overlapping-audience and target-path collisions; and
7. scan selected directory trees with stability, size, secret, executable,
   mode, VCS, and special-file checks.

Keep discovery and capture separate. Inventory may describe a candidate;
capture revalidates its exact source stamp and content.

### 6.4 Pet inventory

Implement a strict pet parser and atlas header/decoder validation:

- exact v2 typed manifest;
- directory/ID match;
- one relative PNG/WebP atlas;
- exact dimensions and bounded decoded allocation;
- canonical descriptor serialization; and
- extra-file warnings.

Do not call an image-generation or pet-creation workflow. This feature only
validates and transports existing approved packages.

### 6.5 Phase tests

Port the relevant ownership/collision cases from the archive and add:

- extension-provided skill never becomes a payload resource;
- name/hash equality alone is not ownership evidence;
- selected catalog profiles are the only provider roots scanned;
- the paired shared `.agents/skills` root is scanned once and project
  `.agents/skills` trees are never scanned;
- Codex does not invent a `CODEX_HOME/skills` root;
- OpenCode overlap with Claude/shared roots does not duplicate a skill;
- standard Agent Skill bytes round-trip without cross-provider rewriting;
- provider-specific skill frontmatter produces the expected
  supported/warning/blocking compatibility result;
- extension cache/clone paths never enter inventory;
- managed registry, npm, and portable Git extension sources round-trip where
  supported by the provider adapter;
- local/credential-bearing sources are manual-only;
- loose Pi TypeScript extensions and linked Gemini CLI extensions are
  manual-only;
- target-disabled extension state is local only;
- valid pet descriptor and atlas round-trip;
- v1/wrong-size/unsafe pets block; and
- an inventory source change invalidates a pending plan.

## 7. Phase 4 — remote catalog and Push

### 7.1 Catalog engine

Implement:

- direct read of the fixed catalog head;
- exact manifest fetch/validation;
- immutable object, manifest, and commit publication;
- semantic no-op detection;
- first-writer `CatalogId` establishment;
- head CAS and ambiguous-response resolution;
- bounded history traversal; and
- local/S3 parity.

Do not add `_tag.json` or make remote repository discovery enumerate the
catalog.

### 7.2 Push planning

`prepare_storage_catalog_push` pins:

- storage ID and connection identity digest;
- catalog binding and profile revisions;
- current catalog head or exact absence;
- reviewed base;
- inventory source stamp;
- desired resource IDs;
- retained remote-only resource descriptors/files;
- explicit removals;
- warning digests; and
- expiry.

Push blocks an unreviewed head change. Missing selected local resources are
retained only from the exact base.

### 7.3 Publication

`publish_storage_catalog`:

1. reloads and revalidates every plan coordinate;
2. captures selected local payload resources;
3. retains approved base resources and objects;
4. emits typed extension intent with no payload;
5. carries unsupported future envelopes and their object references forward
   byte-for-byte from the exact reviewed base;
6. writes explicit resource tombstones only for resources managed by a
   registered format adapter;
7. publishes through catalog CAS; and
8. updates the local desired set/base only after success.

### 7.4 Phase tests

Run every flow through local folder and stub S3:

- fresh catalog;
- concurrent first-publish ID race;
- unchanged no-op;
- add/update/remove skill;
- add/update/remove pet;
- add/update/remove extension intent;
- add supported resource while retaining an unsupported future envelope;
- block add/change/remove of an unsupported future envelope;
- retained unavailable resource;
- explicit tombstone;
- remote head advanced;
- stale inventory;
- warning acknowledgement mismatch;
- missing/corrupt immutable object;
- manifest size/hash mismatch;
- CAS loss and ambiguous success; and
- repository head/listing remains unchanged.

## 8. Phase 5 — Pull planning and local apply

### 8.1 One typed Pull plan

Add a dedicated `CatalogPullPlan` with typed actions:

```text
install_skill
update_skill
overwrite_skill
adopt_skill
install_extension
update_extension
replace_extension_source
install_pet
update_pet
overwrite_pet
adopt_pet
unsupported
manual
```

There is no separate dependency plan. Every action has one resource ID,
expected source digest, expected target state, approval requirement, and
resolved catalog profile or shared root. `unsupported` is informational and
can never be approved for execution. A resource may produce several actions,
one per compatible target; each action has its own stable plan-local action ID
and result.

### 8.2 Accept before mutate

On apply:

1. verify remote head and all plan pins;
2. persist the accepted remote resource set and reviewed base;
3. acquire the affected provider lock;
4. revalidate the target;
5. execute only approved actions; and
6. record verified per-resource receipts and failures.

Persisting acceptance before provider mutation prevents a skipped or failed
install from becoming an accidental remote deletion in a later Push.

### 8.3 Skill/pet transaction executor

Implement one directory replacement primitive parameterized by a validated
resource materializer:

- same-filesystem stage;
- complete path/hash/size/mode verification;
- free-space check;
- target digest pin;
- permission-restricted journal and backup;
- rename activation;
- final verification;
- rollback when safe; and
- startup recovery.

The executor must not accept an arbitrary filesystem target. Skill and pet
adapters derive the target below the revalidated canonical catalog profile or
shared root.

### 8.4 Extension executor

For each approved extension:

- re-inventory;
- recheck source/capability conflicts and target-local disablement;
- resolve provider-managed prerequisites;
- invoke the exact structured process plan without a shell, or execute the
  adapter's bounded digest-pinned config transaction with backup;
- capture bounded stdout/stderr into sanitized activity output;
- re-inventory and verify; and
- report `partial` rather than fabricated success when native state is
  uncertain.

Never uninstall on remote removal or binding detach.

### 8.5 Claim index

Derive and persist enough receipt evidence to detect:

- same target and digest across storages;
- same target with divergent skill/pet digests;
- same extension ID and source;
- same extension ID with conflicting sources; and
- standalone/extension-provided capability collisions.

Fresh target inventory always participates; receipts alone are not authority.

### 8.6 Phase tests

- matching/adopt/install/update/overwrite for skills and pets;
- one shared-root action satisfies all paired adapters that read the root;
- one portable skill may be explicitly materialized to several compatible
  provider targets with independent receipts;
- explicit approval and target-digest pinning;
- target changes after review;
- stage/backup/rename/final-verify failure injection;
- startup journal recovery;
- symlink, special-file, case, nested-path, and insufficient-space blockers;
- stale profile/storage/binding revisions;
- extension idempotence, native argv/env, typed config preservation and
  rollback, source conflict, version limitation, target disablement, partial
  verification, and retry;
- unsupported resources generate no target or native-process plan;
- same profile paired to two storages;
- skipped/failed actions preserve the accepted desired set;
- remote removal performs no delete/uninstall action; and
- unrelated action failures do not alter successful receipts.

## 9. Phase 6 — Tauri and frontend

### 9.1 Commands and DTOs

Register only the command family in the design. Route frontend calls through
`storageCatalogApi` and mirror every DTO in `src/types.ts`.

Command tests assert that catalog operations accept no project or bundle
identity and that project command DTOs remain unchanged. The capability command
returns provider keys, labels, accepted audiences, supported formats, and
operation support from the compiled backend registry; the frontend does not
maintain a second hard-coded provider enum.

### 9.2 Storage workspace

Extend the existing storage selection route with:

- Agent Catalog overview;
- explicit Claude, Cursor, Codex, OpenCode, Pi, and Gemini CLI profile
  selectors,
  plus the shared Agent Skills root;
- family tabs generated from registered formats;
- an Unsupported list for forward resources with no local adapter;
- catalog Push review;
- catalog Pull/install review;
- per-resource action/result details; and
- connection Settings.

Keep project screens unchanged except for removing any stale copy that claims
global capabilities are part of project sync.

### 9.3 Frontend integration tests

Cover:

- storage selection opens Agent Catalog, not a project capability page;
- explicit profile pairing and validation errors;
- shared Agent Skill appears once with its audience and all locally compatible
  providers;
- local/storage/matching/diverged/blocked status language;
- selection persists in Push review;
- storage-only skipped Pull item remains selected after acceptance;
- overwrite and extension-execution warnings require approval;
- unsupported provider/format items are visible, immutable, and have no Apply
  control;
- v2 pet preview and unsupported-v1 state;
- partial and failed results are not presented as success; and
- no catalog tabs or counts appear in project Push/Pull.

Register any new integration test file in
`scripts/run-frontend-integration-tests.mjs`.

## 10. Phase 7 — remove stale paths and reconcile documentation

After the new path is green:

1. Remove dead project-bundle target mapping for
   `state/<provider>/skills/**`.
2. Keep skill/plugin/pet variants out of project `ResourceKind`,
   `RestoreActionKind`, recipes, manifests, and readiness DTOs.
3. Remove or correct current-looking documentation that says project capture
   still emits global agent customizations.
4. Mark the old project/repository capability plans as historical and link to
   the new design.
5. Keep `docs/archive/skill-plugin-sync/` read-only as historical evidence.
6. Add repository-wide negative searches/tests for removed production symbols
   such as `get_project_capability_status` and the old dependency plan.

There is no migration command, hidden import, one-time scanner, or compatibility
alias.

## 11. Handoff verification

Run the smallest relevant checks during each phase, then:

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:backend-integration
npm run test:integration
```

S3 transport cases that bind a loopback server may need the same explicit
environment permission as the current backend integration suite.

Also run focused source checks proving:

```text
no catalog type imports LocalProjectId or BundleId
no project manifest enum contains Skill, Plugin, or Pet
no project command accepts a catalog plan or selection
no catalog manifest contains a local absolute path or local ID
no old dependency document is loaded
no open provider/format key can dispatch without a registered adapter
no catalog inventory enters a project-local customization root
```

## 12. Done definition

The implementation is ready to merge only when:

- all acceptance criteria in the design pass on local and stub-S3 storage;
- the complete current project-sync suite remains green;
- the catalog works with zero projects registered;
- a catalog failure cannot block repository operations in the same storage;
- explicit overwrite/extension consent is demonstrated in backend and frontend
  tests;
- all six initial provider adapters pass their authoritative fixture suite;
- backend capability discovery drives provider selectors without a duplicate
  frontend provider enum;
- a shared Agent Skill is stored once and can target every compatible paired
  provider;
- unknown future envelopes are preserved but cannot be modified or applied;
- startup recovery is exercised with failure injection;
- no compatibility reader or repository-scoped capability path exists; and
- `CURRENT_METADATA_LAYOUT.md` describes the implemented state rather than the
  proposal.
