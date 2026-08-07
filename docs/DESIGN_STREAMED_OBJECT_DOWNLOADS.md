# Design: Streamed, verified object downloads

- Status: Implemented
- Last reviewed: August 6, 2026
- Scope: bundle-object reads in local-folder and S3/R2 storage, manifest
  coordinates, lazy Pull payload access, durability audits, and restore
  materialization
- Compatibility: clean cutover; no migration, fallback reader, or legacy
  object-store API
- Deployment requirement: pre-cutover `~/.mallard` state and storage must be
  cleared; they are not migrated
- Related:
  - [current-implementation-bug-audit-2026-07-27.md](research/current-implementation-bug-audit-2026-07-27.md)
    — H-09 records the former post-buffer size check
  - [CURRENT_METADATA_LAYOUT.md](CURRENT_METADATA_LAYOUT.md) — documents the
    current storage and machine-local metadata shapes
  - [DESIGN_REMOTE_HISTORY_CONTINUITY.md](DESIGN_REMOTE_HISTORY_CONTINUITY.md)
    — historical manifest probes consume the new manifest byte length

## Decision

Mallard uses a staged, file-backed object contract instead of whole-object
`Vec<u8>` downloads. S3/R2 bodies are consumed one chunk at a time, written to
a private temporary file, and verified against the expected byte length and
SHA-256 before any higher layer can open them.

There is no generic 512 MiB object limit. Manifest-declared byte
lengths are integrity coordinates, not policy ceilings: a payload may be
larger than 512 MiB, but the remote server may not send one byte more or less
than the reviewed manifest declares.

Manifest metadata gives current and historical manifest GETs an exact expected
length:

- `BundleHead.manifest_size` names the current manifest's serialized byte
  length;
- `RecipeBase.manifest_size` retains the reviewed historical manifest length
  in machine-local state; and
- the internal `HeadToken` carries `manifest_size` with the other observed
  head coordinates.

All three fields are required `u64` values. They have no Serde default and are
never inferred from `Content-Length`, an extra S3 `HEAD`, or downloaded bytes.
The publisher computes the value from the exact serialized manifest bytes
before uploading them.

The portable and machine-local schema constants remain 1 under the
repository's clean-cutover policy. The new required fields become part of the
only supported schema-1 shape. Pre-cutover state is invalid and is removed
rather than migrated.

## Resolved problem

Before this cutover, `S3BundleObjectStore::get_async`:

1. trusts an optional `Content-Length` enough to perform an early limit check;
2. calls `response.body.collect()`;
3. converts the aggregated body to contiguous bytes and copies it into a
   `Vec<u8>`; and
4. checks the actual length against 512 MiB only after those allocations.

A chunked response has no declared length, and a dishonest S3-compatible
server can declare a small length while producing more bytes. Bulk GET permits
six concurrent downloads, so the former shape could allocate several complete
bodies simultaneously before rejecting them.

Changing only `collect()` to a chunk loop would have been insufficient. The
former `StoredObject { bytes: Vec<u8> }`,
`BundleObjectStore::get/get_many`, `ObjectCache`, `EngineByteSource`, and
`BundleBytes::get() -> &[u8]` contracts required complete objects to reside in
memory. Removing the 512 MiB check without replacing those contracts would
have turned the existing late-bound allocation into an unlimited allocation.

The former fixed 120-second whole-body timeout was also an implicit size
limit. The implemented timeout now measures inactivity while awaiting each
chunk, so an actively progressing transfer may run longer than 120 seconds.

## Goals

- Keep memory usage proportional to the bounded number of in-flight network
  chunks, not to object size.
- Accept payloads larger than 512 MiB without a Mallard-wide object-size
  rejection.
- Reject declared-length mismatch before body consumption when possible.
- Reject chunked overrun before writing the offending chunk.
- Verify exact length and SHA-256 before exposing an object to planning,
  analysis, caching, audit, or apply.
- Preserve immutable publication, head CAS, no-follow traversal, atomic
  target writes, reviewed-target revalidation, and backup guarantees.
- Give current and historical manifest reads exact byte-length metadata.
- Delete superseded eager and compatibility-only APIs instead of maintaining
  parallel execution paths.

## Non-goals

- Remove resource-specific policy limits such as conversation, provider
  capture, or project-content limits.
- Remove aggregate capture/project-content limits or cache-capacity budgets.
  Those budgets control product policy or retained working state; they are
  not the generic transport object ceiling removed here.
- Treat ETag as a content digest. ETag remains an opaque conditional-write
  token.
- Add resumable or multipart upload. S3 write-side 512 MiB guards are removed,
  but upload transport improvements remain separate from this GET hardening.
- Preserve storage or machine-local documents that predate the required
  `manifest_size` metadata.
- Add an S3 `HEAD` fallback for missing size metadata.

## Metadata update

### Portable head

`_head.json` includes one required size field:

```text
schema_version
bundle_id
kind
revision
commit_id
manifest_key
manifest_sha256
manifest_size                 # exact serialized manifest bytes
updated_at
```

`BundleHead::validate` requires `manifest_size > 0`. `read_manifest` passes
`(manifest_size, manifest_sha256)` to the verified staging API before parsing
the manifest.

### Machine-local reviewed base

`RecipeBase` includes required `manifest_size`. It is written from:

- the published snapshot's head after a successful Push; and
- the inspected snapshot's head after a completed Pull.

`RecipeBase::validate` requires a nonzero size. The field is persisted inside
`LocalProjectRegistration.recipe_bases` in `~/.mallard/sync_config.json`.

Historical manifest inspection and remote-history continuity probes change
from `(revision, commit_id, manifest_sha256)` to
`(revision, commit_id, manifest_size, manifest_sha256)`. This lets a probe
stop an overlong response while it is streaming instead of discovering the
problem after collection.

The `LocalProjectRegistration.recipe_bases` mirror in `src/types.ts` includes
required `manifest_size: number`. No optional field, nullable form, or
old-shape deserializer exists.

### Internal observed-head token

`HeadToken` includes `manifest_size` so every observed head coordinate travels
together through publish expectations and CAS chaining. The hash is still the
identity proof; the size is the exact transfer expectation. Head-token
matching compares the size as well, so the field is consumed rather than
carried as unused metadata.

`RestorePlan`, `MaterializationRecord`, `BundleManifest`, and `BundleTag` do
not gain the field. `BundleSnapshot` already embeds `BundleHead`, and the Pull
base writer uses that freshly inspected head directly. Copying the size into
documents that never initiate a manifest read would add redundant, unused
metadata.

### Publication ordering

Publication computes and propagates the metadata in this order:

1. serialize the complete manifest once;
2. convert `manifest_bytes.len()` to `u64` with checked conversion;
3. compute SHA-256 over those exact bytes;
4. immutable-put the manifest;
5. write `manifest_size` and `manifest_sha256` into the new head; and
6. after successful head CAS, persist the same pair in the local
   `RecipeBase`.

No later reserialization is allowed to derive the length because JSON byte
length is a property of the published representation, not merely the parsed
value. `CommitRecord` does not gain the field: no current read path consumes
commit records, so adding it there would create dead metadata.

## Object-store contract

The existing byte-returning payload contract is deleted. The replacement has
separate metadata and verified-body operations:

```rust
struct ExpectedObject {
    size: u64,
    sha256: String,
}

struct VerifiedBody {
    // Private, file-backed body with cheap-clone ownership.
    // open() returns a fresh reader positioned at byte zero.
}

enum StageOutcome {
    Missing,
    Mismatch,
    Verified(VerifiedBody),
}

trait BundleObjectStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String>;

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String>;

    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String>;
}
```

`get_metadata` is only for intrinsically small marker, head, tag, and CAS
documents and always requires a document-specific bound. Manifests and
payload objects use `stage_verified`; there is no unbounded byte-returning
method and no default limit.

`StageOutcome` is exactly `Missing`, `Mismatch`, or
`Verified(VerifiedBody)`. Transport, timeout, and disk failures remain
`Err(String)`, so absence and integrity mismatch are distinct from an
incomplete transfer.

`VerifiedBody` owns a private `NamedTempFile` through cheap-clone shared
ownership. It exposes a fresh reader, verified size and hash, and the service
ETag. It does not expose an `as_bytes()` convenience method. The cache stores
these file-backed handles; it never converts them to in-memory object bodies.

The local-folder adapter implements the same contract by opening with
no-follow validation and copying into a private staged snapshot while hashing.
It must not return a path into mutable storage because another process could
replace that path after verification.

## S3/R2 streaming algorithm

For a verified GET:

1. send `GetObject` under the existing request/header timeout;
2. reject a negative `Content-Length`;
3. when `Content-Length` is present, require it to equal `ExpectedObject.size`
   before polling the body;
4. create a private OS-managed `NamedTempFile`;
5. poll `ByteStream::try_next()` under an inactivity timeout;
6. for each chunk:
   - update the received count with checked `u64` addition;
   - reject when the new count exceeds the expected size, before writing;
   - update the SHA-256 state;
   - write the complete chunk before requesting another chunk;
7. at EOF, require the received count to equal the expected size;
8. require the incremental digest to equal the expected SHA-256;
9. flush and `sync_all`; and
10. return a `VerifiedBody` handle that keeps the verified temporary file
    alive for the operation and optional cache.

Missing `Content-Length` is valid and exercises the same count/hash checks.
Body failure, length mismatch, digest mismatch, timeout, cancellation, or disk
failure drops the response and unpublished temporary file.

The body timeout is an inactivity timeout applied while waiting for the
next chunk. An actively progressing large transfer is not rejected merely
because its total duration exceeds 120 seconds.

`stage_many_verified` keeps bounded request concurrency, returns results in
input order, and aborts and drains all remaining workers after the first
transport, timeout, or disk error. `Missing` and `Mismatch` remain ordered
outcomes. Because each worker writes to disk before polling another chunk,
memory is bounded by the concurrent HTTP chunks rather than the sum of object
sizes.

## Engine and apply changes

- `EngineByteSource` passes each `BundleFileEntry.size` and
  `BundleFileEntry.sha256` into staging.
- `EngineByteSource.loaded` and `ObjectCache` hold cheap-clone
  `VerifiedBody` handles instead of `Arc<Vec<u8>>`.
- Cache keys include storage, bundle, SHA-256, and exact size, so a malformed
  manifest cannot reuse a digest-correct body under a different size
  coordinate.
- Cache capacity is an eviction/bypass rule. A body larger than the cache
  budget remains operation-local and is not rejected.
- Resource-bounded reads compare the manifest entry size with their policy
  limit before staging. Conversation prefetch excludes over-limit entries,
  so the later bounded read rejects them without spending network or disk.
- Prefetch otherwise stages only the objects needed by approved actions and
  reports progress from verified expected sizes.
- Durability audit streams and verifies each object, records the result, and
  drops the staged body instead of retaining payload bytes.
- `BundleBytes::get() -> &[u8]` is replaced by
  `BundleBytes::body() -> VerifiedBody`; an explicitly named `read_bounded`
  helper exists only for consumers with a resource-specific in-memory limit.
- Atomic restore accepts a reader, streams into a destination-parent
  temporary file, verifies exact size/hash, applies safe mode, syncs, and
  revalidates the reviewed target immediately before atomic persistence.
- Apply's post-plan head check compares the current manifest size as well as
  revision, commit, and hash before any approved action is materialized.
- Existing-target backups are copied and hashed as streams.
- Consumers that genuinely need random access may call an explicitly bounded
  materialization helper using their existing resource-specific limit.
  Transcript analysis remains subject to its conversation-specific policy
  until that parser is independently converted to streaming.

## Removed generic 512 MiB ceiling

The hard cutover:

- deleted `domain::MAX_FILE_BYTES`;
- removed the generic size rejection from `BundleFileEntry::validate`;
- removed the publication-time generic portable-file rejection;
- deleted `bundle_engine::MAX_OBJECT_BYTES`;
- removed its local-store read/write, restore-target, atomic-write, and generic
  JSON checks;
- removed the S3 GET, immutable PUT, bulk PUT, and CAS 512 MiB checks; and
- removed obsolete imports and `"exceeds the read/write/portable limit"`
  branches.

No replacement global object ceiling is introduced. Exact manifest sizes,
document-specific metadata bounds, available disk space, backend service
limits, and resource-specific product policies remain authoritative.

## Failure and safety behavior

- A response longer than expected fails before the first excess chunk is
  written.
- A response shorter than expected fails at EOF.
- A matching length with different bytes fails its SHA-256 check.
- A staged failure never populates the cache and never changes a restore
  target.
- An apply failure leaves the old target and any completed immutable backup
  intact.
- Disk exhaustion is reported as a transfer failure; no partial staged object
  becomes readable.
- Error messages include the logical object key and failure class, never body
  bytes, credentials, absolute provider paths, or staging paths.
- ETag validation and CAS semantics remain unchanged.

## Hard cutover and dead-code deletion

The implementation does not add `get_streaming` beside the old `get`. It
replaces the old path and updates every implementation and caller in one
change.

The cutover deleted the superseded eager/test-only surfaces that preserved
whole-bundle byte ownership:

- `StoredObject` and byte-returning `get/get_many`;
- `FetchedBundle`;
- `PreloadedBytes`;
- `fetch` and `fetch_with_progress`;
- eager restore-plan and apply wrappers retained only for tests; and
- the test-only `PublishedBundle.head_token` result field; callers that need
  the current token use the authoritative `read_head` result; and
- associated `allow(dead_code)` attributes.

Tests use `inspect`, the production lazy verified-body source, lazy
planning/apply, and `verify_objects`. No deprecated type aliases, adapters,
old-shape Serde defaults, or conditional legacy branches remain.

## Implemented cutover

The hard cutover was applied in this order:

1. Added required manifest-size metadata and validation to `BundleHead`,
   `RecipeBase`, `HeadToken`, publication, base writes, historical
   inspection/probes, and `src/types.ts`.
2. Introduced `ExpectedObject`, `StageOutcome`, `VerifiedBody`, and the
   replacement object-store trait.
3. Implemented local staged reads and reader-based metadata handling.
4. Implemented S3 chunk-to-file staging, verification, inactivity timeout,
   cancellation, and ordered bulk staging.
5. Converted cache, lazy loading, audit, restore writes, and backups to readers.
6. Migrated tests off eager whole-bundle APIs and deleted those APIs.
7. Removed both generic 512 MiB constants and every obsolete check/error.
8. Updated current metadata and technical documentation.

## Test plan

### Metadata

- publication records the exact serialized manifest size in the head;
- successful Push and Pull persist the same size in `RecipeBase`;
- current manifest reads forward the head's exact size;
- historical reads and continuity probes forward the base's exact size;
- missing `manifest_size` fails deserialization with no fallback;
- zero size fails validation; and
- mismatched declared size fails before manifest parsing.

### S3 streaming

- no `Content-Length`, exact-size chunked body succeeds;
- declared exact length succeeds;
- negative or mismatched declared length fails without polling the body;
- one byte beyond expected fails before that chunk is written;
- short EOF fails;
- matching size with wrong SHA-256 fails;
- body error and stalled chunk fail;
- producer cancellation proves the response is dropped on overflow;
- partial staging files are removed;
- empty and multi-chunk objects preserve bytes and ETag semantics; and
- bulk failure cancels siblings while successful results retain input order.

### Engine and filesystem

- a manifest entry larger than 512 MiB validates without allocating its
  payload in the test;
- a recording store proves the full `u64` expected size is forwarded;
- local and S3 staged reads have parity;
- cache bypass for a body larger than the cache budget does not reject it;
- durability audit does not retain staged payloads;
- failed verification leaves the destination unchanged;
- reader-based apply preserves mode, digest, backup, and atomicity behavior;
  and
- no-follow and special-file rejection remain enforced.

### Verification

Run:

```text
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
```

The frontend checks are required because the `RecipeBase` DTO changes in
`src/types.ts`.

## Acceptance criteria

- No S3 GET path calls `ByteStream::collect()`.
- No payload or manifest read returns a complete `Vec<u8>` from the store.
- Chunked and dishonest-length responses cannot exceed their reviewed
  expected size.
- Memory use is independent of verified body size.
- The generic `domain::MAX_FILE_BYTES` and
  `bundle_engine::MAX_OBJECT_BYTES`, their generic rejection messages, and all
  superseded eager APIs are absent. Resource-specific capture constants remain.
- Current and historical manifest reads always have required
  `manifest_size` metadata.
- Missing pre-cutover metadata is rejected rather than migrated or inferred.
- All Rust, TypeScript, backend, and frontend integration checks pass without
  dead-code allowances for the removed design.
