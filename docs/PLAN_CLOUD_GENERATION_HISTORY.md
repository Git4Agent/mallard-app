# Cloud generation history, comparison, and restore plan

- Status: implemented (backend phases 1-6, 2026-07-26); no UI surface yet
- Date: 2026-07-26
- Baseline: `ccd2275` (`Exclude Codex subagent rollouts`)

Implementation notes (deviations from the proposal, per review):

- `BundleGenerationRecord.published_by_origin` is required, not optional.
- Delta fields are `u64` counts; resource changes compare canonical
  per-resource content digests.
- `BundleResourceChange` carries `left_thread_title`/`right_thread_title`.
- The traversal backstop is 100,000 hops — a malformed-data guard, never a
  history-age limit; contiguous strictly-descending generations already make
  cycles unrepresentable.
- Publication no-op detection recomputes the snapshot digest from both full
  manifests; no stored digest is trusted for it.
- `CurrentBundleSnapshot` is realized as the existing `BundleSnapshot`
  (head + manifest) with `version_snapshot()` converting into the
  version-pinned `BundleVersionSnapshot` that all restore machinery uses; no
  fake head is ever fabricated for historical data.
- A conversation-kind resource without a thread entry (session index,
  unidentifiable rollout) is valid auxiliary state; silent thread loss is
  rejected at publish time by the retention check instead of static manifest
  validation.
- TypeScript mirrors for `RestorePlan`/`DependencyPlan` moved with Phase 5
  because those DTOs were already registered command contracts.

Owner constraints: there are no external users or retained test installations.
Before testing, `~/.mallard` and every Mallard cloud bucket will be wiped. This
is a clean cutover: do not migrate old state, read old bundle schemas, dual
write old and new formats, or add compatibility defaults.

In this document, **G1**, **G2**, and **G3** mean cloud generations 1, 2, and 3.
They do not mean bundle schema versions.

## Decision

Replace the current partial commit history with one immutable, hash-linked
generation chain:

```text
mutable head -> G3 -> G2 -> G1
```

Every reachable generation has:

- a small immutable generation record;
- an immutable manifest containing all comparison metadata;
- a logical snapshot digest;
- one digest per live resource; and
- a complete hash-pinned reference to its parent generation.

This supports three independent backend operations:

1. list every reachable generation without reading payload objects;
2. compare any pair, such as G1/G2, G1/G3, or G2/G3, using manifests only; and
3. build and apply a reviewed restore plan from any selected generation.

The mutable cloud head is never moved backward. Restoring G1 reads G1 as the
source while the head remains at G3. If cloud rollback is added later, it must
publish G4 whose logical content equals G1 rather than overwrite G3 or reuse a
generation number.

## Scope

### In scope

- one current portable bundle schema and one publication path;
- immutable generation records and a verified reachable-history chain;
- whole-snapshot and per-resource comparison digests;
- metadata-only generation listing and arbitrary pair comparison;
- selected-generation restore planning and apply;
- local-folder and S3/R2 storage parity;
- backend DTOs, commands, validation, persistence, and tests; and
- only the TypeScript contract mirrors required by registered Tauri commands.

### Out of scope

- a generation picker or comparison UI;
- React component, layout, styling, or navigation changes;
- migration or reading of existing local/cloud data;
- cloud-head rollback;
- generation deletion, retention, pruning, or garbage collection;
- transcript-level message diffs; and
- silently deleting local-only resources to make the machine an exact clone.

A selected-generation restore uses Mallard's existing reviewed Pull semantics:
it materializes the selected cloud snapshot, preserves conflicts and local-only
resources by default, and performs deletion only through an existing explicit,
digest-pinned deletion action. It is not an unreviewed filesystem rollback.

This plan does not replace the local incremental work in
`PLAN_PROJECT_LEVEL_REFRESH_PERFORMANCE.md`. If both plans are implemented,
that plan's linked-storage path should consume the manifest title and digest
metadata defined here instead of opening stored session-index or transcript
objects.

## Required behavior

After three successful uploads, the backend must:

- return G3, G2, and G1 in descending order;
- resolve each displayed generation to exactly one reachable immutable version;
- compare every pair without downloading `_objects/*` payloads;
- report whole-snapshot equality separately from manifest identity;
- identify added, removed, changed, and unchanged resources;
- expose stored conversation titles from the manifest;
- plan a restore whose source is G1, G2, or G3;
- fetch payload bytes only when planning or applying actions that require them;
- reject apply if the mutable head, local binding, host identity, target digest,
  or plan lifetime changed after review; and
- record the selected source generation in receipts and the reviewed local
  comparison base.

## What exists now

The current backend is close, but its contracts conflate the current head with
the version being restored.

### Existing useful pieces

- `BundleEngine::publish` and `publish_schema6` write immutable manifests at
  `_manifests/{generation}-{commit_id}.json`, then immutable commit records,
  then advance `_head.json` with compare-and-swap
  (`bundle_engine.rs:1208-1255`, `:1460-1507`).
- Failed head compare-and-swap can leave unreachable immutable objects but
  cannot make a partial generation current.
- `BundleFileEntry.sha256` already pins every stored file
  (`domain.rs:2722-2729`).
- `ConversationThread` already has portable title, head-message, count,
  lineage, and semantic metadata (`domain.rs:2135-2165`).
- `inspect_manifest_version` can load and verify an immutable historical
  manifest when generation, commit ID, and manifest SHA-256 are already known
  (`bundle_engine.rs:1642-1675`).
- `RestorePlan` already pins generation, commit ID, manifest SHA-256, binding
  revision, host identity revision, and expiry (`domain.rs:3473-3493`).
- `manifest_is_semantic_noop` already defines which manifest fields count as a
  logical publication change (`bundle_engine.rs:5491-5516`).

### Gaps

1. `CommitRecord.previous_commit_id` stores only an ID. It does not pin the
   previous generation record or previous manifest hash.
2. The mutable head does not authenticate a complete history chain.
3. There is no backend command that lists reachable generations.
4. Generation alone cannot currently resolve a manifest. The historical reader
   also requires commit ID and manifest digest.
5. Listing objects by a generation prefix is unsafe: concurrent CAS losers can
   leave orphaned immutable manifests and commit records for the same proposed
   generation.
6. `manifest_sha256` is an integrity/identity digest, not a content digest. It
   covers generation, commit ID, and timestamps, so it cannot answer whether
   two generations have the same logical snapshot.
7. File digests can be compared from manifests, but multi-file resources have
   no single comparison digest.
8. `BundleSnapshot` validates a manifest specifically against the mutable head
   (`domain.rs:3294-3304`), so it cannot honestly represent historical G1 while
   G3 is current.
9. Restore planning always inspects the current head
   (`commands.rs:5722-5817`).
10. Restore apply loads the current snapshot and requires the plan's source
    coordinates to equal the current head (`bundle_engine.rs:5689-5702`,
    `commands.rs:5828-5890`).
11. Dependency/readiness support also assumes the restore source is the current
    head (`commands.rs:6040-6080`).
12. Production and tests still carry two bundle schemas and two publication
    paths. This conflicts with the requested clean implementation.

## Clean schema cutover

Collapse `BUNDLE_SCHEMA_V4` and `BUNDLE_SCHEMA_V6` into one required
`BUNDLE_SCHEMA: u32 = 1`. Because all buckets are wiped, strictly reject every
other value. Do not keep old parsing arms or a schema cutover publisher.

Replace:

- `publish` plus `publish_schema6` with one publication path;
- `CommitRecord` with `BundleGenerationRecord`; and
- `_commits/` with `_versions/`.

Keep the outer storage namespace, storage marker, content-addressed
`_objects/`, immutable PUT behavior, and head compare-and-swap protocol. They
are not compatibility mechanisms and do not need to change.

Required new fields must not use `serde(default)`. Missing history, digest, or
parent fields are corrupt current-schema data and must fail closed.

This work supersedes the bundle-schema retirement portion of
`PLAN_REMOVE_BACKWARD_COMPAT.md`; it must not implement a second competing
cutover.

## Portable metadata model

### BundleVersionRef

```text
BundleVersionRef
  generation: u64
  commit_id: String
  manifest_sha256: String
  snapshot_content_sha256: String
  generation_record_sha256: String
```

Validation requires:

- `generation >= 1`;
- a valid named commit ID;
- three valid lowercase SHA-256 digests; and
- no fields derived from local paths, storage credentials, or machine-local
  identity.

Manifest and generation-record keys remain canonical and derived from
generation plus commit ID. Do not accept a caller-supplied arbitrary object key.

### BundleGenerationRecord

Store at `_versions/{generation}-{commit_id}.json`:

```text
BundleGenerationRecord
  schema_version
  bundle_id
  generation
  commit_id
  manifest_sha256
  snapshot_content_sha256
  previous: BundleVersionRef?
  created_at
  published_by_origin?
  delta:
    added_resources
    changed_resources
    removed_resources
    added_files
    changed_files
    removed_files
```

The record does not contain its own digest. The publisher hashes its canonical
serialized bytes and puts that digest in the `BundleVersionRef` stored by the
head or child record.

`previous` is `None` only for G1. For every later generation:

- `previous.generation + 1 == generation`;
- the previous bundle ID is implicitly the same bundle;
- the referenced generation record must hash to
  `previous.generation_record_sha256`; and
- its manifest must hash to `previous.manifest_sha256`.

The delta is only a summary against the direct parent. It is not used to
compare arbitrary generations and never contains titles, paths, IDs beyond the
version coordinates, or chat content.

### BundleHead

Replace the separate generation/commit/manifest fields with:

```text
BundleHead
  schema_version
  bundle_id
  kind
  latest: BundleVersionRef
  updated_at
```

The current project refresh can continue fetching only the mutable head and
the immutable manifest: `latest.manifest_sha256` directly pins that manifest.
Generation-history commands additionally load the generation record and verify
`latest.generation_record_sha256`.

### BundleManifest

Keep the current portable snapshot maps:

- bundle identity and recipe;
- capture codec versions;
- conversations;
- resources;
- files and directories; and
- tombstones.

Add:

```text
resource_content_sha256: BTreeMap<ResourceId, String>
```

The map must have exactly one entry for every live resource and no extra keys.
Manifest validation recomputes every resource digest and rejects a mismatch.

Do not add a second transcript digest. A conversation's exact file digest
continues to live in `BundleFileEntry.sha256`; its resource digest incorporates
that file entry plus the portable conversation summary. This prevents two
authoritative transcript hashes from disagreeing.

Conversation titles live directly in `ConversationThread.display_title`.
Generation listing and comparison must never download
`state/codex/session_index.jsonl` merely to label a stored thread.

## Canonical digest contracts

### Manifest digest

`manifest_sha256` remains the SHA-256 of the exact serialized manifest bytes.
It answers:

> Did I receive the exact immutable manifest named by this version?

It must not be presented as logical snapshot equality.

### Resource-content digest

Compute each resource digest from an explicit canonical projection containing:

- the full portable `ResourceDescriptor`;
- its `ConversationThread`, when present;
- every owned file's logical path, SHA-256, size, source mtime, and mode;
- every owned directory's logical path, source mtime, and mode; and
- relevant live tombstone metadata.

Exclude object keys because transport placement is not logical content.

Serialize a dedicated projection struct backed by `BTreeMap`/sorted vectors,
prefix it with a domain separator such as
`mallard-resource-content-v1\0`, then hash it. Do not hash ad hoc debug output
or a general-purpose map whose ordering is unspecified.

### Snapshot-content digest

Refactor the current `manifest_is_semantic_noop` definition into one canonical
`SnapshotContentProjection`. It contains:

- bundle identity;
- recipe;
- `captured_with.codec_versions`;
- conversations;
- resources;
- resource-content digests;
- semantic file entries with object keys removed;
- directories; and
- tombstones.

It excludes:

- schema version;
- generation and commit ID;
- manifest/update timestamps outside semantic entries;
- `published_by_origin`;
- app, Codex, and Claude diagnostic version strings; and
- immutable object keys.

Prefix the canonical bytes with `mallard-snapshot-content-v1\0` and hash them.
Use this same helper for:

- publication no-op detection;
- `BundleVersionRef.snapshot_content_sha256`;
- generation-record validation; and
- whole-snapshot comparison.

There must be one definition of logical equality. Do not retain a separate
field-by-field no-op predicate that can drift from the digest projection.

## Publication protocol

For a successful new generation:

1. Read and validate the current head once.
2. Load and verify its current manifest.
3. Reconcile the candidate snapshot and compute all resource digests.
4. Compute the candidate snapshot-content digest.
5. If it equals the current head's snapshot-content digest, validate semantic
   equality defensively and return a true no-op without creating a generation.
6. Upload new content-addressed payload objects.
7. Write the immutable manifest.
8. Build the generation record with the observed head's complete
   `BundleVersionRef` as `previous`.
9. Write the immutable generation record and compute its SHA-256.
10. Compare-and-swap the head to the new `BundleVersionRef`.
11. Update the non-authoritative bundle tag.

Publication order keeps payloads, manifest, and version record immutable before
the generation becomes visible. A failed CAS may leave orphaned immutable
objects, but no history API may expose them because they are not reachable from
the current head.

## History resolution

Add engine helpers:

```text
read_generation_record(version_ref)
list_reachable_generations(bundle_id, cursor, limit)
resolve_reachable_generation(bundle_id, generation)
inspect_version(version_ref)
```

### Traversal rules

- Start only from the freshly read current head.
- Verify every generation-record digest before trusting its parent.
- Require strictly descending, contiguous generation numbers.
- Require matching bundle and commit coordinates at every hop.
- Reject cycles, duplicate generations, future parents, missing records, and
  chain depth above a fixed safety bound.
- Never recover history by listing `_versions/` or `_manifests/`.
- Return newest first with a bounded page size.
- Cache only already verified immutable records, keyed by storage-instance
  identity, bundle ID, and generation-record SHA-256.

For the initial implementation, a parent-linked walk is sufficient. The
generation picker will normally list versions before requesting a comparison,
so those verified records will already be cached. Do not add a mutable global
history index or binary-lifting skip pointers without measured need.

## Backend contracts

Add Rust DTOs:

```text
BundleGenerationSummary
  version: BundleVersionRef
  created_at
  published_by_origin?
  delta

BundleGenerationPage
  generations
  next_cursor?
  observed_head: BundleVersionRef

BundleGenerationComparison
  left: BundleGenerationSummary
  right: BundleGenerationSummary
  same_snapshot
  summary counts
  resource_changes
  bundle_metadata_changed
  recipe_changed
  codec_changed
  tombstones_changed

BundleResourceChange
  resource_id
  kind
  display_name
  thread_title?
  state: added | removed | changed | unchanged
  left_content_sha256?
  right_content_sha256?
```

Add backend operations:

```text
list_bundle_generations(storage_id, bundle_id, cursor?, limit?)
compare_bundle_generations(
  storage_id,
  bundle_id,
  left_generation,
  right_generation,
  include_unchanged
)
plan_bundle_restore(
  storage_id,
  bundle_id,
  binding,
  source: current | generation(u64)
)
```

Generation numbers from callers are selectors, not trust anchors. The backend
resolves them through the verified reachable chain and pins the resulting full
`BundleVersionRef`.

For this backend-only phase, no React surface consumes these operations.
Registering the commands requires matching DTOs and thin wrappers in
`src/types.ts` and `src/components/project-sync/api.ts`, but no component,
route, CSS, or interaction change.

## Comparison flow

To compare G1 and G3:

1. Read the mutable head and validate its latest version.
2. Resolve G1 and G3 through the reachable chain.
3. If their snapshot-content digests match, return `same_snapshot = true`
   without loading payloads. The response may omit unchanged resource rows.
4. Otherwise fetch the two immutable manifests, preferably through one bounded
   metadata `get_many` operation.
5. Verify each manifest SHA-256 and recompute its snapshot/resource digests.
6. Diff the sorted resource-digest maps:
   - only left: removed;
   - only right: added;
   - both and equal: unchanged;
   - both and different: changed.
7. Separately compare bundle identity, recipe, codec versions, and tombstones
   so a metadata-only change is not mislabeled as a resource change.

Comparison is O(number of manifest resources), transfers only bounded metadata,
and performs zero `_objects/*` reads.

For conversations:

- equal resource digest means the portable thread snapshot is identical;
- equal lineage/semantic digest can be reported as supporting metadata;
- different digests mean changed, not necessarily diverged; and
- prefix-versus-divergence classification remains a detailed operation that
  may need transcript payloads.

## Selected-generation restore

### Separate source from observed head

Replace the ambiguous restore-plan pin with:

```text
RestorePlan
  source_version: BundleVersionRef
  observed_head: BundleVersionRef
  storage_id
  bundle_id
  host_id
  replica_id
  expected_host_identity_revision
  binding_revision
  created_at
  expires_at
  actions
  conversation_actions
  project_content_eligibility?
```

For an ordinary current Pull, `source_version == observed_head`. For restoring
G1 while G3 is current:

```text
source_version = G1
observed_head = G3
```

Do not keep the old top-level generation/commit/manifest fields or deserialize
old plans. The local state is wiped before testing.

### Planning

1. Acquire the existing host operation lock.
2. Revalidate the current binding and host identity.
3. Read the mutable head once and store it as `observed_head`.
4. Resolve the requested generation through that head's reachable chain.
5. Load the selected immutable manifest into a new `BundleVersionSnapshot`.
6. Build actions from the selected manifest using existing reviewed bases and
   materialization receipts where valid.
7. Pin every action to `source_version`; pin plan freshness to
   `observed_head`.
8. Persist the bounded, expiring plan atomically.

### Applying

1. Load and validate the saved plan.
2. Revalidate host identity, binding revision, target paths/digests, approval
   IDs, and expiry.
3. Re-read the mutable head and require exact equality with
   `plan.observed_head`.
4. Resolve and verify that `plan.source_version` is still reachable from that
   head.
5. Load the exact selected immutable manifest, not the current manifest.
6. Fetch only payload objects needed by approved selected-generation actions.
7. Apply through the existing backup, atomic-write, no-follow, digest, and
   deletion protections.
8. Write receipts and materialization records using the selected source
   generation and manifest.
9. Set the reviewed recipe/base coordinates to the selected source, not the
   current head.

If the head advances after review, reject apply even if the selected generation
remains an ancestor. The user must refresh and approve a new plan against the
new head.

After restoring G1 while G3 remains current, the local base is G1. A later Push
must not treat that as permission to overwrite G3. Existing Push head/CAS and
reviewed-base checks must continue to require an explicit reconciliation.

### Snapshot type cleanup

Introduce:

```text
BundleVersionSnapshot
  storage_id
  version: BundleVersionRef
  manifest
  fetched_at
```

Use it for historical inspection, byte sources, restore planning, restore
apply, dependencies, and readiness. Keep a current-head inspection result as a
separate type containing:

```text
CurrentBundleSnapshot
  storage_id
  head
  version_snapshot
  fetched_at
```

Do not construct a fake `BundleHead` for historical data. Refactor
`EngineByteSource` to key objects from `version_snapshot.manifest.bundle_id`
rather than `snapshot.head.bundle_id`.

Dependency plans and readiness reports created from a restore plan must inspect
`source_version` while separately revalidating `observed_head`.

## Caching

The backend may add bounded process-local caches for:

- verified generation records by generation-record digest;
- verified manifests by manifest digest; and
- existing verified payload objects by content digest.

Every cache key must include a non-secret storage-instance identity and bundle
ID. The mutable head is always re-read. Cache hits may avoid immutable GETs but
must never bypass reachability, binding, plan, or CAS checks.

The generation feature can share the verified-manifest cache proposed in
`PLAN_PROJECT_LEVEL_REFRESH_PERFORMANCE.md`; do not create two caches with
different validation rules.

## Safety and privacy

- Portable history contains no credentials, provider-home paths, checkout
  paths, local exclusions, local binding IDs, plans, receipts, or backups.
- Titles are bounded portable display text and must never enter activity logs.
- History logs contain counts, generations, byte totals, and durations only.
- Manifest, record, page, traversal-depth, and comparison-result bounds apply
  before allocation.
- A broken or missing parent fails closed. It does not silently truncate
  history and present an incomplete chain as valid.
- An orphaned CAS-loser generation cannot be listed, compared, or restored.
- Immutable PUT collision checks, SHA-256 verification, and mutable-head CAS
  remain mandatory.
- Restore remains plan-based and explicitly approved.
- Historical restore does not relax symlink, special-file, canonical-path,
  bounded-read, backup, atomic-write, or deletion safeguards.

## Implementation sequence

### Phase 1: Collapse to one bundle schema

Files:

- `src-tauri/src/project_sync_v3/domain.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`
- `src-tauri/src/project_sync_v3/commands.rs`
- existing schema-focused Rust tests

Work:

- replace the v4/v6 constants and validation arms with one strict schema;
- merge the two publication request paths;
- ensure production Push populates portable conversation metadata;
- delete cutover-only readers, writers, guards, tests, and comments; and
- retain one no-op, immutable-object, manifest, and CAS flow.

This phase must compile and pass on its own.

### Phase 2: Add canonical content digests

Files:

- `domain.rs`
- `bundle_engine.rs`
- focused domain and bundle-engine tests

Work:

- add canonical resource and snapshot projection types;
- add domain-separated digest helpers;
- add and validate `resource_content_sha256`;
- replace `manifest_is_semantic_noop` with the shared snapshot digest; and
- prove that transport-key changes do not alter logical content while
  restore-affecting metadata does.

### Phase 3: Add the verified generation chain

Files:

- `domain.rs`
- `bundle_engine.rs`
- local-store and S3/R2 parity tests

Work:

- add `BundleVersionRef` and `BundleGenerationRecord`;
- write `_versions/*` before head CAS;
- make the head pin the latest record, manifest, and snapshot digests;
- implement bounded verified traversal, resolution, and pagination; and
- remove `CommitRecord` and `_commits/*`.

### Phase 4: Add arbitrary comparison

Files:

- `domain.rs` or backend response DTO module
- `bundle_engine.rs`
- `commands.rs`
- comparison tests with a counting object store

Work:

- add generation summary/comparison DTOs;
- implement whole-snapshot fast equality;
- implement manifest-only per-resource diff;
- expose titles from conversation metadata; and
- assert zero payload-object reads.

### Phase 5: Generalize restore to a selected source

Files:

- `domain.rs`
- `bundle_engine.rs`
- `commands.rs`
- `persistence.rs`
- restore, dependency, readiness, and integration tests

Work:

- split `source_version` from `observed_head`;
- introduce `BundleVersionSnapshot`;
- generalize byte sources and restore helpers;
- update dependency/readiness support;
- persist selected-source receipts/bases; and
- preserve every existing plan/apply safety check.

### Phase 6: Expose backend commands without UI

Files:

- `commands.rs`
- `src-tauri/src/lib.rs`
- `src/types.ts`
- `src/components/project-sync/api.ts`

Work:

- register list and compare commands;
- accept an explicit current/generation selector when planning restore;
- mirror the serialized DTOs exactly; and
- add no React component or styling changes.

## Test matrix

### Domain and digest tests

- deterministic resource digest across map insertion orders;
- deterministic snapshot digest across serialization runs;
- manifest digest changes when generation/commit changes;
- snapshot digest does not change solely because generation/commit/object key
  changes;
- snapshot digest changes for every field currently covered by semantic no-op;
- resource digest changes for title, transcript SHA, mode, mtime, lineage,
  descriptor, file, directory, and relevant tombstone changes;
- malformed/missing digest and mismatched resource-digest map rejection; and
- strict rejection of old bundle schemas.

### Generation-chain tests

- publish G1, G2, G3 and list exactly `[G3, G2, G1]`;
- paginate without duplicates or gaps;
- resolve G1, G2, and G3 by generation;
- reject generation 0, a future generation, and an unreachable generation;
- tampered generation record, parent hash, manifest hash, or coordinates fail;
- missing parent fails rather than truncating history;
- cycle and traversal-bound rejection;
- a CAS-loser orphan is not listed or resolvable; and
- local-folder and S3/R2 return the same reachable chain.

### Comparison tests

- G1/G2, G1/G3, and G2/G3 each classify added, removed, changed, and unchanged
  resources correctly;
- title-only and recipe-only changes are represented correctly;
- manifest hashes differ while logical snapshot hashes match when a later
  generation intentionally republishes an earlier snapshot after an
  intervening change;
- comparison order reversal swaps added/removed;
- `include_unchanged = false` keeps counts but omits unchanged rows;
- no manifest comparison reads `_objects/*`; and
- a changed conversation is not falsely classified as prefix or divergence
  from digest equality alone.

### Restore tests

- plan/apply current G3 remains unchanged behavior;
- plan G1 while head is G3 pins source G1 and observed head G3;
- independently restore G1, G2, and G3 from the same reachable chain;
- selected-generation apply reads selected manifest/payload objects, not G3's;
- head advance after planning rejects apply;
- tampered or newly unreachable source rejects apply;
- binding, host identity, target digest, approval, and expiry races reject;
- dependency and readiness views use the selected source manifest;
- receipt/materialization/base records name the selected source generation;
- restoring G1 does not authorize overwriting G3 on the next Push;
- local-only resources are preserved absent an explicit approved deletion; and
- repeat apply of one plan remains rejected/idempotent through receipts.

### Request-count and privacy tests

- history listing reads one mutable head plus only required generation records;
- same-snapshot comparison stops before manifest/payload fetch when both
  verified refs are already available;
- detailed comparison reads at most two immutable manifests and zero payloads;
- verified metadata cache reuse does not skip the mutable-head read;
- metadata contains no absolute local paths or credentials; and
- logs contain no thread IDs, titles, object keys, paths, or content.

## Verification

Run during implementation:

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
```

The frontend commands/types checks are required only in Phase 6. S3/R2 tests
must use the loopback stub and temporary roots; never use a real bucket,
`~/.codex`, `~/.claude`, or `~/.mallard`.

## Acceptance criteria

The backend work is complete when:

1. one strict bundle schema and one publisher remain;
2. G1, G2, and G3 form a hash-verified reachable chain;
3. any pair can be compared from metadata without payload downloads;
4. stored thread titles are present in the manifest;
5. G1, G2, or G3 can independently be selected as a reviewed restore source;
6. restore apply pins both the selected source and the observed mutable head;
7. restoring an old generation never rewinds or mutates cloud history;
8. CAS-loser artifacts cannot be listed, compared, or restored;
9. all existing restore and publication safety invariants remain enforced; and
10. no migration, old-schema reader, compatibility default, or dual writer is
    present.
