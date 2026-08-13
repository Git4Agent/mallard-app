# Plan: Minimal portable-tail conversation conflict resolver — backend

- Status: Proposed
- Date: August 10, 2026
- Implements: [DESIGN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER.md](DESIGN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER.md)
- Integrates: [DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md](DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md)
- Follow-up, out of scope: [PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md](PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md)
- Scope: backend schemas, Codex reconstruction, Refresh/Push/Pull, persistence,
  recovery, and backend tests
- Compatibility: clean in-place cutover at numeric schema `1`; no migration,
  backward compatibility, or legacy execution path

This plan implements one deliberately small conflict identity: the cumulative
canonical root-conversation tail beginning at a fixed portable boundary.
Subagent files are transported only as incidental artifacts of a root-driven
Push or Pull. They never participate in Refresh or conflict classification.

## 1. Required behavior

Mallard keeps cloud logical identity separate from machine-local Codex identity:

```text
cloud logical T <-> machine A provider rollout P-A
cloud logical T <-> machine B provider rollout P-B
```

For a paginated chain:

```text
P0: m1 -> m2
P1: history(P0@m2) -> m3
P2: history(P1@m3) -> m4 -> m5
```

P0 and P1 are physical reconstruction sources. They are not portable nodes and
do not receive tail entries. P2 owns one logical root conversation:

```text
complete standalone root = m1,m2,m3,m4,m5
portable start            = canonical boundary after m3
portable tail             = m4,m5
```

If `m4` and `m5` have subagent rollouts, those files may be included in a
complete upload/download, but they do not affect the tail value. Two snapshots
with identical canonical root messages and completely different subagent bytes
classify as synchronized.

### 1.1 Core invariants

1. Every publishing Push reconstructs a complete standalone Codex snapshot,
   validates typed output invariants, hashes every emitted file while writing
   it once, uploads immutable objects, and changes cloud HEAD only by
   compare-and-swap.
2. A root with a native `history_base` uses `FixedPaginatedTail`; a root without
   one uses `WholeThread` starting at zero.
3. The tail start is selected once for the logical task and survives standalone
   materialization even though the installed native file has
   `history_base: null`.
4. The portable tail describes only normalized root conversation messages.
   History source segments, subagents, attachments, machine IDs, source paths,
   provider IDs, and physical byte offsets are excluded.
5. The inherited prefix before a fixed nonzero start is immutable under the
   supported provider contract. Mallard does not hash it during routine tail
   comparison.
6. Successful Push or cloud-accepting Pull advances the accepted tail end and
   state. Nothing resets the cumulative tail to an empty value merely because
   it was accepted.
7. Refresh, Push, and Pull use one provider-neutral P/L/C comparator.
8. Root-tail equality is only a conflict relation. Artifact/object hashes,
   accepted commit provenance, ScopeBase evidence, and HEAD CAS remain the
   integrity and concurrency authorities.
9. Old Codex rollout sources are never moved or deleted by this implementation.
   Cleanup is a separate future operation.
10. Materializing Pull review pins immutable metadata only. Uninterrupted apply
    downloads each cloud input once and reconstructs/hashes one remote output
    once before journaling and installation.

## 2. Standalone reconstruction

Use one bounded graph builder in two modes:

```rust
enum ReconstructionMode {
    PreserveIdentity,
    Reidentify {
        root_provider_thread_id: String,
        subagent_provider_thread_ids: BTreeMap<CanonicalSubagentKey, String>,
    },
}
```

The builder:

1. resolves the selected root's `history_base` chain with byte and ordinal
   cutoffs;
2. rejects cycles, missing dependencies, ambiguity, symlinks, special files,
   path escape, unsupported record shapes, and exceeded bounds;
3. flattens the root into one runnable standalone rollout;
4. resolves the selected snapshot's bounded subagent closure only when a
   publishing or materializing action has already been selected;
5. rewrites provider IDs consistently in `Reidentify` mode;
6. writes authoritative output metadata with paginated history mode and null
   native history base;
7. preserves inline ancestor metadata as ordinary records rather than treating
   it as another portable task;
8. validates the typed output invariants before and during serialization,
   including bounded JSONL records, consistent rewritten IDs, paginated/null
   native base metadata, valid ordinals/references, and the expected root tail;
   and
9. serializes each validated output once while streaming the emitted bytes
   through SHA-256 into private staging.

Refresh and relation classification do not invoke this complete graph builder.
They parse only the root/history ranges required to derive L.

## 3. Current schema-1 portable tail

### 3.1 Clean cutover

Keep every existing numeric schema discriminator at `1`. The current shape
replaces the previous shape in place.

At every persistence boundary:

1. require the exact current top-level schema key and value;
2. reject duplicate, aliased, or unknown fields and every missing required
   field through one generic strict-wire rule; only fields explicitly optional
   in the current provider contract may be absent;
3. validate every nested bound and discriminant before constructing runtime
   values;
4. reject pre-cutover Codex schema-1 values that lack either new required
   resolver field, without compatibility defaults; and
5. serialize only the current shape and read it through the same validator
   before publication or atomic replacement.

Old writers must be excluded operationally. There is no mixed-version protocol.
Do not enumerate rejected keys invented only in unshipped design drafts;
ordinary unknown-field rejection already covers them. Only fields that existed
in the shipping schema, such as the conflict-only `fork_stamp`, need explicit
removal tests.

### 3.2 Portable wire shape

Codex `ConversationThread` requires:

```rust
struct PortableConversationTail {
    codec: String, // exactly mallard-codex-portable-tail-v1
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    accepted_end_record_count: u64,
    accepted_tail_state: Sha256Digest,
}

enum TailPartitionKind {
    WholeThread,
    FixedPaginatedTail,
}
```

This five-field structure is the complete wire contract. There is no:

- node vector or topology digest;
- subagent or attachment state;
- prefix lineage/semantic/boundary witness;
- separately serialized tail-record count;
- layout version independent of the codec name; or
- separately serialized or derived hash of the whole structure.

`start_record_count` and `accepted_end_record_count` are positions in the
normalized standalone root-message sequence, not physical JSONL ordinals or
visible UI message indexes. Validate:

```text
start_record_count <= accepted_end_record_count
tail_record_count = accepted_end_record_count - start_record_count
```

`WholeThread` requires `start_record_count == 0`. A native paginated root maps
its verified history cutoff to `start_record_count` once and uses
`FixedPaginatedTail`, including the rare case where that count is zero.

### 3.3 Appendable tail state

The v1 canonicalizer emits a deterministic sequence of normalized root
conversation messages. It excludes subagent lifecycle, spawn/reference/control,
and child-result bookkeeping unless such data changes a canonical root message.

Use one length-prefixed, domain-separated appendable chain:

```text
record_fingerprint(r) = SHA256(LP_TUPLE(
  "mallard-codex-canonical-root-message-v1",
  canonical_root_message_bytes_v1(r)))

S0 = SHA256(LP_TUPLE(
  "mallard-codex-portable-tail-start-v1",
  codec,
  partition_kind,
  start_record_count))

Sn = SHA256(LP_TUPLE(
  "mallard-codex-portable-tail-record-v1",
  prior_tail_record_count,
  S(n-1),
  record_fingerprint(rn)))
```

`LP_TUPLE` is `U32_BE(field_count)` followed by
`U64_BE(field_length) || field_bytes` for every field. Integers are encoded as
unsigned big-endian 64-bit values, digests as raw 32 bytes, and strings as exact
UTF-8.

The cloud `accepted_tail_state` is `Sn` after all root messages in
`[start_record_count, accepted_end_record_count)`. One-shot and incremental
computations must agree for every split point.

The shared comparator compares validated five-field tail values directly. It
does not compute another fingerprint over them.

### 3.4 Full artifact integrity remains separate

Every cloud conversation still carries its existing root lineage/semantic
metadata and exact file references. Every referenced object is protected by its
manifest size and SHA-256. Add one `artifact_sha256` field to the current Codex
`ConversationThread`. It is the exact complete standalone-artifact SHA returned
by the reconstruction/serialization layer and verified against the immutable
objects. The resolver uses that field directly and does not define another
thread-version codec or repeat a canonical file-join hash construction.

Because the Rust `ConversationThread` type is shared by providers, both
`artifact_sha256` and `portable_tail` are optional at that generic wire layer.
Current Codex validation requires `Some` for both; non-Codex adapters keep their
provider-specific contract. A Codex entry missing either field is a rejected
pre-cutover shape.

`artifact_sha256` covers the complete transported snapshot, including
subagents, but it never enters P/L/C. Equal roots with different artifact hashes
remain `Sync`.

## 4. Machine-local state

### 4.1 Layout belongs to the binding

Do not create a separate logical-tail registry. Store the immutable root layout
in the existing machine-wide logical binding:

```rust
struct PortableRootTailLayout {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
}

struct LocalConversationBinding {
    // Existing identity, alias, state, revision, and timestamp fields.
    // ...
    tail_layout: Option<PortableRootTailLayout>,
}
```

Remove the shipping conflict-only `fork_stamp` field and `ForkStamp` type from
this current binding shape together with the native-fork executor. Do not leave
a serde default or compatibility reader for it.

The field is optional only because the shared binding document contains other
providers. A current Codex binding requires `Some`.

Use the existing binding revision for concurrency; add no layout generation.
First discovery writes the provider alias and layout in one atomic
`ConversationBindings` replacement, so there is no binding-without-layout
interval and no first-discovery journal.

A standalone null-base file never reseeds a bound task. Every future destination
reads the binding's stored layout.

Divergent keep-both updates both entries in one bindings-document mutation:

```text
before: binding[T] = (P-local, local layout)
after:  binding[T] = (P-remote, remote layout)
        binding[C] = (P-local, copied local layout)
```

This identity split is the only allowed replacement of T's layout. It preserves
the local layout and existing provider alias unchanged under C before T imports
the verified remote layout. It does not edit, rename, copy, reconstruct, or
reidentify `P-local`.

### 4.2 Destination checkpoint

Accepted cloud state remains destination-scoped:

```text
(local_project_id, storage_id, sync_scope, thread_id)
  -> ConversationCheckpoint
```

Use the minimal accepted provenance pin:

```rust
struct AcceptedConversationPin {
    portable_tail: PortableConversationTail,
    artifact_sha256: Sha256Digest,
    accepted_at_commit_id: CommitId,
    accepted_at_manifest_sha256: Sha256Digest,
}

struct RootTailValue {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    end_record_count: u64,
    tail_state: Sha256Digest,
}

struct FileIdentity {
    canonical_path: CanonicalPath,
    device: u64,
    inode_or_file_id: u128,
}

struct PhysicalPosition {
    provider_record_ordinal: u64,
    byte_offset: u64,
}

struct RootPhysicalMapping {
    provider_thread_id: String,
    file_identity: FileIdentity,
    tail_start: PhysicalPosition,
    accepted_end: Option<PhysicalPosition>,
    observed_end: PhysicalPosition,
    safe_eof: PhysicalPosition,
}

struct RawRangePin {
    file_identity: FileIdentity,
    start_byte: u64,
    end_byte: u64,
    raw_sha256: Sha256Digest,
}

enum PhysicalMappingState {
    Ready(RootPhysicalMapping),
    Detached {
        expected_provider_thread_id: String,
        expected_binding_revision: u64,
    },
}

struct ConversationCheckpoint {
    generation: u64,
    accepted: Option<AcceptedConversationPin>,
    observed: RootTailValue,
    physical: PhysicalMappingState,
    raw_read_range_pin: RawRangePin,
    provider_profile_revision: u64,
    binding_revision: u64,
}
```

Do not duplicate lineage or semantic hashes in the accepted pin. The accepted
immutable manifest witness is the authority for those values and for the exact
thread entry containing `artifact_sha256`.

`Ready` means the current bound root has a valid start/observed-end mapping. Its
`accepted_end` is optional because another destination may have accepted a
different branch. `Detached` means the mapping must be rebuilt; it says nothing
about the P/L/C relation.

On load, if the binding alias/revision changed or the source identity no longer
matches, perform one bounded full root scan using the binding layout:

- if the current start maps uniquely, install `Ready` and derive L;
- otherwise return a typed mapping error;
- once L is available, the shared comparator—not the mapping state—decides
  whether Pull is required.

`Detached` is the only invalid physical-mapping state.

## 5. Shared P/L/C comparator

Define:

```text
P = destination accepted PortableConversationTail
L = current local RootTailValue converted to the same five-field value
C = current verified cloud-manifest PortableConversationTail
```

Use one provider-neutral function:

```rust
struct TailComparisonEvidence {
    pin: TailPresence<PortableConversationTail>,
    local: TailPresence<PortableConversationTail>,
    cloud: TailPresence<PortableConversationTail>,
    successors_compatible_with_pin: bool,
}

fn compare_conversation_tail(
    evidence: TailComparisonEvidence,
) -> TailClassification;
```

The evidence gatherer validates before comparison:

- exact codec and field shape;
- matching partition and start for present successors;
- local binding/profile/source provenance;
- the exact accepted commit/manifest entry and artifact SHA;
- current cloud ancestry from that accepted commit;
- reviewed ScopeBase and HEAD evidence; and
- directional presence.

Failed, incompatible, missing-expected, or ambiguous evidence becomes `Unknown`.
Subagent state is never gathered.

When P/L/C are present and valid, evaluate exactly:

| Predicate | Classification |
| --- | --- |
| `L == C` | `Sync(Current)` when `C == P`, otherwise `Sync(Advanced)` |
| `C == P && L != P` | `LocalOutstanding` |
| `L == P && C != P` | `CloudOutstanding` |
| otherwise | `Divergent` |

`Current` means the accepted **tail value** equals C; it does not require the
accepted commit ID or artifact SHA to equal the current manifest. Therefore a
subagent-only or unrelated manifest advance with the same root tail remains a
true no-op. A later root publication may use the verified descendant HEAD as
its CAS parent.

Directional absence:

- local present, cloud absent, no P -> `MissingCloud`;
- cloud present, local absent, no P -> `MissingLocal`;
- both absent, no P -> `MissingBoth`;
- an expected side missing after P -> `Unknown`;
- both sides present without P -> `Unknown`.

There is no containment refinement. For example, P=`m4,m5`, L=`m4,m5,m6`,
and C=`m4,m5,m6,m7` are conservatively `Divergent` because both advanced and
their final values differ.

## 6. Refresh

Refresh is root-only and checkpoint-driven:

1. reconcile any nonterminal operation journal touching the task or destination;
2. load the binding layout and destination checkpoint;
3. validate or rebuild the current root physical mapping;
4. for `FixedPaginatedTail`, read complete newline-terminated records only after
   the last observed cursor and extend the accumulator;
5. for `WholeThread`, recompute from canonical root zero;
6. derive L and gather manifest-only P/C evidence;
7. call `compare_conversation_tail`;
8. re-stat root handles and retry a bounded number of times if the safe end
   moved; and
9. under the persistence lock, recheck the checkpoint generation, binding
   revision, SyncConfig revision, source identity/end, and absence of a
   conflicting journal before persisting only the new observation and relation.

Refresh never:

- promotes accepted state;
- reconstructs standalone output;
- downloads cloud transcript objects;
- inventories or hashes subagents; or
- decides a relation outside the shared comparator.

Push/Pull planning rehashes the entire reviewed unaccepted **local** physical
interval from accepted end, or from tail start when the accepted end is not mapped.
`WholeThread` re-pins its complete root range. Cached Refresh work never silently
expands plan approval.

## 7. Push

### 7.1 Action gate

Push uses the shared classification:

| Classification | Push action |
| --- | --- |
| `LocalOutstanding` | `Publish` |
| `MissingCloud` | `CreateCloud` |
| `Sync(Current)` | `NoChanges` |
| `Sync(Advanced)` | `RequirePullMetadata`; report that root content is synchronized but Pull must advance the accepted pin/ScopeBase |
| `CloudOutstanding` or `Divergent` | `RequirePull` |
| `Unknown` | `Blocked` |
| `MissingLocal` or `MissingBoth` | `NotPushable` |

A selected batch containing `RequirePullMetadata` or any other blocker publishes
no partial subset. Only `Publish` and `CreateCloud` proceed to reconstruction.

### 7.2 Push planning

```text
Push planning:
  compare P/L/C
  pin source identities, exact EOFs/cutoffs, raw range hashes, tail, and cloud HEAD
  do not reconstruct output
```

For each publishing root:

1. freeze P/L/C evidence, the selected relation, binding/checkpoint/profile/
   ScopeBase revisions, tail layout, and cumulative local tail;
2. resolve—but do not reconstruct—the bounded root/history/subagent source
   closure selected by that root action;
3. pin every consumed source identity, fixed cutoff or exact safe EOF, and the
   raw SHA-256 of every planning-reviewed range;
4. pin the exact verified current manifest and HEAD that will be the candidate
   base; and
5. store the exact local document preconditions needed to build the accepted
   checkpoint mutation later.

Planning does not emit standalone bytes, compute output/artifact hashes, build a
candidate manifest/commit, or build the local after-state. The reviewed plan is
an exact input snapshot plus a deterministic operation. It contains no staged
Push output.

### 7.3 Bounded local journal mutation

Do not store complete SyncConfig or bindings documents in the journal. Store:

```rust
struct DocumentPrecondition {
    document: LocalDocumentKind,
    expected_revision: u64,
    expected_sha256: Sha256Digest,
}

struct DocumentMutation {
    precondition: DocumentPrecondition,
    operations: BoundedVec<TypedEntryMutation>,
    expected_after_sha256: Sha256Digest,
}
```

Under the existing cross-process persistence lock:

- exact-before -> apply the typed mutations, verify after digest, atomically
  replace the document;
- exact-after -> treat the phase as already committed;
- any third value before cloud/provider commitment -> stale/abort;
- any third value after commitment -> `RecoveryBlockedCorruption`.

The journal reserves the affected physical document keys while nonterminal, but
does not embed unrelated entries. This keeps journal size bounded independently
of the 8 MiB SyncConfig and 64 MiB ConversationBindings limits.

Journal cost is metadata-only: fixed IDs, hashes, source/target pins, phase, and
the affected entry mutations. It grows with the bounded number of selected
conversations and output files, not with transcript byte size. In normal use it
is a small JSON document; staged Pull files and immutable Push objects dominate
storage. Terminal journals follow the existing bounded retention policy.

### 7.4 Push apply and recovery

```text
Push apply:
  verify source pins while reading them once
  reconstruct once
  hash and verify each output once
  upload immutable objects
  CAS HEAD
```

HEAD CAS is already the final cloud mutation. Push recovery exists only for the
crash window after that CAS may have succeeded but before Mallard durably
records the accepted pin and ScopeBase locally. Moving CAS later cannot remove
that cross-system window; there is no transaction spanning object storage and
the local config file.

Apply:

1. revalidate the reviewed cloud manifest/HEAD and local document/source
   metadata preconditions;
2. open the exact planned source closure and, in the same streaming pass used
   for reconstruction, verify source identities, cutoffs/EOFs, and raw range
   hashes;
3. reconstruct each conversation exactly once and, in the same write pass,
   enforce output invariants and compute each output SHA-256; require the
   resulting root tail to equal the planned L;
4. feed that reconstructed L and the revalidated P/C evidence through the
   shared comparator and require the exact reviewed publishing action;
5. build the conversation entries, candidate manifest/commit, and exact local
   checkpoint/ScopeBase mutation from those verified outputs;
6. upload and verify the immutable objects, manifest, and candidate commit;
7. atomically create `CandidateUploaded` journal intent containing the newly
   fixed candidate hashes and local mutation, plus the required
   document reservations, conditional on all local preconditions still matching;
8. recheck the reviewed current HEAD and advance it by CAS; and
9. apply the exact destination-checkpoint/ScopeBase mutation.

If any source pin differs, apply returns stale before object upload or HEAD CAS.
Recovery begins only after `CandidateUploaded`; it consumes the exact candidate
and local mutation stored in that journal and never reconstructs again.

Push phases are:

```text
CandidateUploaded
HeadCommitted
LocalCommitted
Complete
AbortedCasLost
RecoveryBlockedCorruption
```

Recovery checks immutable commit ancestry:

- candidate or descendant at HEAD -> candidate committed; install the exact
  local mutation;
- ancestry excluding candidate -> terminal `AbortedCasLost` without promoting
  local accepted state;
- orphan immutable objects alone never promote anything.

Once `LocalCommitted` is durable, receipt/index presentation is best-effort.
The durable journal can regenerate the receipt, so an ancillary failure does not
hold task or document gates indefinitely.

## 8. Pull

### 8.1 Action mapping

| Classification | Pull action |
| --- | --- |
| `Sync(Current)` | no-op; keep provider files and accepted metadata unchanged |
| `Sync(Advanced)` | zero-target accepted-pin update; keep provider files unchanged and advance the accepted pin/ScopeBase |
| `LocalOutstanding` | keep provider files and longer L; retain P/C as accepted |
| `CloudOutstanding` | materialize verified cloud snapshot as T |
| `Divergent` | keep both: preserve local as new C, install cloud as T |
| `MissingLocal` | create binding with cloud layout and materialize T |
| `MissingCloud` | report Push eligibility |
| `MissingBoth` | no-op/not-found |
| `Unknown` | block |

Zero-target actions do not download transcript objects and never inspect
subagent differences.

### 8.2 Materialization and keep-both

Materializing Pull planning is metadata-only. Immutable object references and
their SHA-256 values are sufficient to approve an exact future download:

```text
Pull planning:
  compare P/L/C
  pin exact HEAD, manifest, object keys, sizes, SHA-256 values, and cloud tail
  for Divergent, pin the exact existing P-local binding/checkpoint/source state
  allocate logical C and provider IDs for the one remote target
  pin target paths, local document preconditions, and semantic after-templates
  do not download objects, reconstruct output, hash output, or stage files

Pull apply:
  compare P/L/C again and verify plan expiry/revisions
  verify P-local is still the exact safe, unique binding being moved to C
  download and SHA-256 verify the pinned immutable cloud objects once
  reconstruct/reidentify the one remote target once while enforcing invariants,
    computing its target SHA-256, and staging the emitted bytes
  map the cloud tail positions and build the exact local mutations
  journal the apply-generated target pins and mutations
  no-clobber install the remote target
  commit bindings, then SyncConfig
```

The manifest pins immutable input bytes; review does not need the bytes
themselves. An unavailable, missing, oversized, or corrupt object makes apply
fail before any journal or provider-visible mutation. The local branch is
preserved by rebinding its existing provider alias; it is never a staged
target.

For one-target materialization:

1. during planning, validate and pin the manifest's exact object descriptors;
2. during planning, allocate fixed provider IDs and pin target paths and local
   preconditions;
3. during apply, revalidate P/L/C, HEAD/scope, target absence, and local pins;
4. during apply, download each pinned immutable object and verify its size/SHA;
5. during apply, reconstruct once in `Reidentify` mode; validate the typed
   output invariants and serialize once while the emitted bytes feed the target
   SHA-256 and private staging;
6. during apply, fsync that output and map the cloud
   `start_record_count` and accepted end into the new root file;
7. during apply, build the exact bindings/SyncConfig mutations and journal them
   with the staged target identity and SHA;
8. during apply, no-clobber install the target files;
9. during apply, atomically update or create T's binding with the cloud layout;
   and
10. during apply, update the selected destination checkpoint and ScopeBase.

An uninterrupted apply does not reopen and hash its own output. The target is
written inside a Mallard-owned private staging directory, fsynced, pinned by
identity/size/SHA, and kept under the operation lock through no-clobber install.
Recovery after a process boundary reopens and rehashes the journaled staged
target before using it, but never downloads or reconstructs it again.

For divergent keep-both:

```text
Before:
  T -> P-local

After:
  C -> P-local    existing local rollout, bytes and provider metadata unchanged
  T -> P-remote   one newly materialized verified cloud snapshot
```

Planning mints logical C and only the provider IDs needed by `P-remote`. It
requires `P-local` to be the exact current alias of T and to be safe and unique
to rebind. The bindings-document mutation updates T and inserts C together; it
does not edit `session_meta`, provider IDs, `history_base`, ordinals, compaction
state, subagent references, indexes, titles, or any rollout byte in `P-local`.

For the selected destination, copy T's observed checkpoint to C as local-only
state: preserve the observed tail, physical positions, source identity, and raw
range pin; clear `accepted`, clear `accepted_end`, key it by C, and pin C's new
binding revision. Initialize C's checkpoint generation as a new checkpoint;
do not inherit T's concurrency generation. T receives the verified cloud
accepted pin and the mapping for `P-remote`. C can later be published with
`CreateCloud`; that later Push is the first time Mallard needs to reconstruct C
as a standalone artifact.

All `history_base` dependencies of `P-local` remain in place. If Mallard cannot
prove that the existing alias and checkpoint can be moved to C safely and
uniquely, Pull blocks. It does not construct a speculative preserved copy.

Any other destination that referenced the replaced alias keeps its independent
accepted pin but receives a `Detached` physical mapping naming T's new alias and
binding revision. On its next operation Mallard rebuilds current L and runs the
same P/L/C comparator. No content-specific detached variant is persisted.

### 8.3 Zero-target apply

`Sync(Advanced)` and `LocalOutstanding` change only the selected destination's
checkpoint and ScopeBase inside the existing SyncConfig document. Their
reviewed plan stores one exact `DocumentMutation`. Apply revalidates P/L/C and
the reviewed remote manifest, takes the persistence lock, and then:

- exact-before -> atomically applies the mutation;
- exact-after -> returns the same receipt idempotently; or
- any third value -> returns stale without writing.

Atomic replacement already gives a before-or-after crash result, so this path
creates no operation journal, staging directory, target IDs, or resource
reservation.

### 8.4 Materializing Pull journal and recovery

Pull does not mutate cloud HEAD. Its recovery covers a different atomicity gap:
one provider target install, one bindings-document mutation, and one
SyncConfig mutation cannot be committed as one filesystem transaction. Without
the journal, a crash could leave a new rollout visible without its binding, or
a binding visible without its destination checkpoint.

Every byte-consuming Pull has exactly one fixed remote target graph. The
reviewed plan stores a singular metadata-only `target_intent`; the journal,
created during apply after successful reconstruction, stores a singular staged
`target`. Divergent adds an optional `existing_local_rebind` pin; it does not add
another target. The journal contains:

- exact reviewed P/L/C evidence and action;
- fixed logical/provider IDs and no-clobber target paths;
- exact target size/SHA and staged identity pins;
- one bounded `ConversationBindings` mutation when bindings change;
- one bounded `SyncConfig` mutation for checkpoints and ScopeBase;
- exact immutable cloud commit/manifest/object pins when bytes are consumed;
- exact local binding/checkpoint/root source pins when Divergent rebinding
  preserves `P-local`; and
- the complete resource-reservation key set.

It contains no source-retirement records or backup paths. Old rollout sources
remain untouched. A Pull receipt writes the literal source outcome `"retained"`
for each already-required old-source pin. There is no source-disposition enum,
per-source phase, mutable cleanup record, backup path, or future-retirement
branch in this release.

Phases:

```text
Prepared
CommitDecided
TargetCommitted
BindingsCommitted
SyncConfigCommitted
Complete
AbortedPrecommit
AbortedProviderCollision
RecoveryBlockedCorruption
```

Apply:

1. reconcile prior journals and revalidate all reviewed local/remote metadata
   pins;
2. download/verify the pinned objects, then reconstruct/reidentify the one
   remote target once while validating invariants and hashing/staging emitted
   bytes;
3. derive exact staged-target pins and local document mutations;
4. atomically persist the journal and reserve affected resources;
5. immediately before `CommitDecided`, recheck exact HEAD/scope, local source
   identities and reviewed ranges, target absence, and document preconditions;
6. after durable decision, no-clobber install the one target graph and fsync
   its files and parents;
7. apply or accept-exact-after the bindings mutation;
8. apply or accept-exact-after the SyncConfig mutation; and
9. terminalize and release reservations.

Any failure before journal creation discards or later garbage-collects the
private staging directory and leaves no recoverable operation or visible state.

Before `CommitDecided`, stale evidence terminalizes `AbortedPrecommit`. After it,
recovery uses the exact immutable reviewed snapshot and fixed target IDs; later
HEAD or source changes do not change the approved result. A foreign target
collision preserves the foreign file and old sources, reports any already
committed files in the target graph as unbound recoverable orphans, and terminalizes
`AbortedProviderCollision`.

After `SyncConfigCommitted`, logical Pull is complete. Provider-index or receipt
failures are deferred maintenance and cannot retain task/document gates.

Recovery rehashes a staged target after a process boundary, but never downloads
cloud objects, invokes Codex, reconstructs, allocates another ID, recaptures
newer source records, resets the tail start, or deletes an old source.

## 9. Provider ID allocation

Keep the CLI-first UUID allocator.

Every logical/provider ID is created once during planning and serialized into
the reviewed plan. Apply and recovery never mint. Logical C uses Mallard's
validated `ThreadId` allocator; the policy below applies only to newly allocated
Codex provider IDs.

Preference order:

1. Create a Mallard-owned temporary root with an empty `CODEX_HOME` and empty
   working directory. Resolve a supported absolute `codex` executable, launch
   `app-server --stdio`, complete `initialize`/`initialized`, and call
   `thread/start` with the disposable CWD and `ephemeral: true`. Accept only the
   matching canonical lowercase RFC 9562 UUIDv7. Send no `turn/start` or model
   request, terminate immediately, and remove the entire temporary root.
2. On missing CLI, unsupported protocol, timeout, exit, malformed response,
   non-v7 ID, or any other failure, generate a cryptographically random RFC 9562
   UUIDv7 locally with strict version/variant/lowercase validation.

A single bounded app-server session may allocate all IDs for one plan. After a
protocol failure, stop using it and allocate the rest locally. Both paths check
collisions against provider inventory, indexes, plans, journals, and IDs already
minted in the current plan. Existing discovered provider IDs retain the current
bounded provider-native string contract. There is no desktop automation fallback.

## 10. Ownership

### `mallard-domain`

Own:

- schema-1 portable-tail and local checkpoint types;
- typed bounded local-document mutations;
- accepted provenance validation inputs;
- the sole provider-neutral P/L/C comparator;
- action mappings and journal state types; and
- clean-cutover validation.

It must not parse Codex JSONL or access filesystem paths.

### `mallard-infrastructure`

Own:

- bounded Codex root/history discovery;
- canonical root-message projection and incremental tail hashing;
- standalone reconstruction, typed output-invariant validation, and
  single-pass serialization/hashing;
- manifest/local evidence gathering without deciding relations;
- CLI-first provider ID allocation and local UUIDv7 fallback;
- no-clobber provider-file installation;
- immutable object publication and HEAD CAS; and
- idempotent journal recovery.

### Tauri command layer

Own orchestration, reviewed plans, cross-process locks, persistence transactions,
and receipts. It must call the domain comparator rather than reproduce P/L/C
logic.

## 11. Validation and tests

### 11.1 Schema and codec

- Exact current schema-1 fixtures accept only the five-field Codex tail.
- Tail-less, missing-field, and unknown-field shapes fail closed even with
  numeric schema `1`; tests do not enumerate names invented only in unshipped
  design drafts.
- `WholeThread` requires start zero; every end is at or after start.
- Derived tail count is exactly `end - start`.
- One-shot and incremental tail state agree for every split point.
- Changing any root message changes the appropriate cumulative state.
- Changing all subagent files, membership, or bytes changes no portable-tail
  value and does not change P/L/C classification.
- Provider-ID/CWD/timestamp rewriting that is excluded by the canonicalizer does
  not change root-tail state.

### 11.2 Binding and checkpoint

- First discovery atomically creates one binding containing its layout; no
  separate registry or recovery operation exists.
- A pulled standalone null-base root reuses the binding/cloud start rather than
  reseeding at zero.
- Future storage destinations read the same binding layout.
- Keep-both updates T and inserts C in one exact bindings mutation.
- `Detached` remaps to `Ready` after a bounded full root parse; relation choice
  remains entirely in the shared comparator.
- Accepted pins for different storage destinations remain independent.

### 11.3 Shared classifier and Refresh

- One exhaustive domain matrix covers Sync, both directional outstanding cases,
  Divergent, all directional missing cases, and Unknown gates.
- Refresh, Push, and Pull feed identical evidence through that function and only
  map its result to actions.
- P=`m4,m5`, L=`m4,m5,m6`, C=`m4,m5,m6,m7` is conservatively Divergent.
- Root-equal snapshots with arbitrary subagent differences are Sync.
- Fixed-tail Refresh reads only appended complete records; WholeThread recomputes
  from zero.
- Refresh never promotes accepted state or downloads cloud transcript objects.
- A concurrent operation journal causes Refresh's final persistence CAS to lose.

### 11.4 Push

- Only `LocalOutstanding` and `MissingCloud` reach reconstruction/upload.
- `Sync(Advanced)` Push reports `RequirePullMetadata`; it does not call Pull's
  metadata transaction or leave a publishing batch partially executable.
- Push planning pins exact inputs and performs no standalone reconstruction,
  output hashing, candidate construction, or local after-state construction.
- Push apply reconstructs each selected conversation once, validates typed
  output invariants, and hashes each output while writing it once before
  immutable upload.
- Every publishing Push emits complete standalone root/subagent files with the
  required null-base metadata and consistent rewritten references.
- A subagent-only change performs no publication.
- Plan/apply reclassification, raw-range, source-end, or document changes stale
  the plan before HEAD mutation.
- Real two-writer CAS produces one winner.
- Ambiguous-CAS recovery promotes local accepted state only when immutable
  ancestry contains the candidate.
- CAS loss leaves uploaded objects orphaned and accepted state unchanged.
- Journal size remains bounded by typed mutations, not source document size.

### 11.5 Pull and recovery

- Sync and LocalOutstanding use one atomic SyncConfig mutation, create no
  journal or targets, and do not reconcile subagents.
- CloudOutstanding/MissingLocal install one verified standalone snapshot.
- Divergent rebinds the existing unchanged `P-local` rollout to C and installs
  only one new no-clobber `P-remote` target for T.
- Divergent C keeps the local layout and observed checkpoint with `accepted =
  None` and no physical accepted end; no local snapshot builder runs.
- Materializing Pull planning reads manifest metadata only. Apply downloads the
  pinned objects once, reconstructs the one remote target once, and
  enforces output invariants and hashes emitted bytes in that same write pass
  before installation.
- A missing/corrupt cloud object fails before journal creation or provider/local
  mutation; recovery uses only the journaled staged target.
- Invalid output metadata, ordinals, or rewritten references fail typed
  invariant validation before serialization/journaling; normal apply never
  reparses staged output.
- Zero-target Pull is one atomic SyncConfig mutation and creates no journal.
- Every crash point before/after each file install within the one target graph
  and each local document mutation recovers to the same completed or safe-abort
  result.
- Old source files are always retained; Pull contains no deletion, retirement,
  backup, or provider-fence path.
- A foreign target collision never overwrites the target and releases gates
  after recording recoverable orphans.
- A third local-document value after commitment yields explicit
  `RecoveryBlockedCorruption` rather than overwrite.
- Ancillary index/receipt failure after logical commit releases task and document
  gates.

### 11.6 CLI allocator

- Fake app-server tests assert the exact
  `initialize -> initialized -> thread/start(ephemeral: true)` sequence and no
  `turn/start`.
- Missing binary, spawn/protocol/timeout/EOF/malformed/non-v7 responses fall back
  to local UUIDv7.
- IDs are lowercase UUIDv7 with correct RFC variant and bounded collision retry.
- Temporary home/CWD cleanup and unchanged real-home sentinels hold on every
  path.
- Apply/recovery never spawn the CLI or remint.

## 12. Implementation sequence

1. Define the five-field schema-1 tail wire, canonical root-message projection,
   appendable chain, and current five-field chain test vectors.
2. Add `tail_layout` to current Codex bindings; remove the proposed separate
   registry, provenance, and layout generations.
3. Add destination accepted pins, observed root tail, and `Ready|Detached`
   physical mapping.
4. Implement the single P/L/C comparator and route Refresh, Push, and Pull
   through it.
5. Implement root-only incremental Refresh and full remap.
6. Implement standalone root/subagent reconstruction and verification.
7. Implement typed bounded document mutations and their exact-before/after
   recovery helper.
8. Cut Push over to the shared action gate, complete reconstruction, immutable
   upload, HEAD CAS, and minimal recovery journal.
9. Implement journal-free zero-target Pull plus journaled one-target Pull,
   keep-both alias transfer, and no source retirement.
10. Keep the CLI-first ID allocator with local UUIDv7 fallback.
11. Remove native-fork conflict execution and every rejected pre-cutover shape.
12. Run domain, infrastructure, Tauri, local-folder, S3/R2, and crash-matrix
   tests with disposable homes only.

## 13. Acceptance criteria

1. Cloud stores exactly one five-field root-tail value per Codex conversation.
2. P0/P1/P2 history sources produce one root with tail `m4,m5`, not three nodes.
3. Subagent differences never change P/L/C or produce an attachment action.
4. Refresh, Push, and Pull call the same equality-only comparator.
5. Fixed-tail Refresh reads only appended root records; roots without a native
   base retain whole-thread comparison.
6. Push planning pins inputs without output construction; publishing apply
   reconstructs each conversation once, validates typed invariants and hashes
   bytes in the same single serialization pass, and advances HEAD only by CAS.
7. Materializing Pull planning downloads no transcript object and constructs no
   output. Apply downloads the pinned immutable objects once, reconstructs one
   remote target once, hashes/verifies the output once, and installs it.
   Divergent Pull rebinds the existing local rollout to C without reconstructing
   it.
8. Layout is stored with the binding; there is no logical-tail registry,
   provenance enum, or task-layout generation.
9. Physical state is only `Ready` or `Detached`.
10. Accepted checkpoint state contains only portable tail, artifact SHA, and
    immutable commit/manifest provenance.
11. Journals store bounded typed mutations rather than complete local documents.
12. Pull never moves or deletes old provider rollout sources.
13. Push and Pull recovery remain deterministic, allocation-free, and
    no-clobber across every crash point.
14. All affected numeric schemas remain `1`; non-current shapes fail closed and
    no compatibility code exists.
15. Newly allocated Codex provider IDs use CLI-first UUIDv7 allocation with
    strict local UUIDv7 fallback.

## 14. Verification commands

```text
cargo fmt --all -- --check
cargo test -p mallard-domain
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Run real-Codex contract tests only against explicitly created disposable
provider homes.
