# Plan: Portable-tail conversation conflict resolver — backend

- Status: Proposed
- Date: August 11, 2026
- Implements:
  [DESIGN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER.md](DESIGN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER.md)
- Integrates:
  [DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md](DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md)
- Supersedes:
  [PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
- Follow-up, explicitly out of scope:
  [PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md](PLAN_CODEX_WATCHER_ACCELERATION_FOLLOWUP.md),
  [PLAN_CODEX_SUBAGENT_CLOSURE_TRANSPORT.md](PLAN_CODEX_SUBAGENT_CLOSURE_TRANSPORT.md)
  (fills the subagent closure that v1 ships empty; amends §2.1/§12 for
  closure tolerance)
- Scope: Rust domain and portable schema, Codex graph flattening and
  standalone reconstruction, tail codec, capture, persistence,
  Refresh/Push/Pull orchestration, recovery, and backend tests
- Excluded: React UI, TypeScript presentation work, filesystem watching, copy,
  and screenshots
- Compatibility: Clean in-place cutover at schema 1; no migration, backward
  compatibility, or legacy reader/writer/execution path

`tail` has one meaning in this plan: the cumulative canonical content of the
root conversation beginning at an immutable portable start — the canonical
boundary seeded from the root's native `history_base`, or zero when the root
has none. A successful publishing Push, or a Pull that accepts a newer cloud
version, advances the accepted tail end and state. Nothing moves the start or
resets the state.

## 1. Model and invariants

Mallard keeps the cloud `ThreadId` as shared identity; a provider thread ID is
a machine-local alias:

```text
cloud logical T <-> A's provider rollout P-A
cloud logical T <-> B's provider rollout P-B
```

The separated values are:

| Value | Meaning | Stored where |
| --- | --- | --- |
| Full artifact/object SHA-256 | Exact bytes of the complete standalone root and transported subagent files | Cloud manifest/object metadata and destination checkpoints; integrity and provenance only |
| Portable tail value | Root-only partition and accepted cumulative state; the sole conversation-content comparison input | Cloud `ConversationThread` metadata and destination checkpoints |
| Root tail layout | The immutable `(codec, partition_kind, start)` slice of that value | The machine-local conversation binding |
| Physical cursor and accumulator | Machine-specific resume state mapping the portable partition into local files | Destination checkpoint only; never serialized to cloud |

Subagent rollouts are snapshot payload. A root-driven publishing Push carries
the current bounded subagent closure; a root-driven materializing Pull
installs the cloud snapshot's closure; every transported object is SHA-256
verified. Subagent files, membership, topology, bytes, and
lifecycle/spawn/reference/control or child-result records never enter the
tail, the classifier, or any relation, and there is no subagent-only
operation, drift state, or `AttachmentChanged` relation. A provider record
affects the tail only if it changes a canonical root conversation message.
This rule is stated once here and applies to every section below.

Core invariants:

1. Every publishing `LocalOutstanding`/`CreateCloud` Push reconstructs the
   complete logical graph as standalone output, hashes and verifies every
   output byte, uploads complete objects, and advances the cloud head only by
   compare-and-swap. Every other relation exits before reconstruction.
2. Each root has one fixed `partition_kind` and start, seeded when the task
   first enters the contract. The start never mutates in place; the only
   layout replacement is the divergent keep-both binding move (Section 9.4),
   which transfers the local layout unchanged to `C` before `T` imports the
   verified remote layout. `MissingLocal` materialization creates a layout
   only when no local binding exists.
3. A native root `history_base` seeds `FixedPaginatedTail`; a root without
   one uses `WholeThread` at zero. The explicit kind survives Pull even though
   reconstructed standalone metadata has a null base. Only `FixedPaginatedTail`
   uses partial-range refresh; `WholeThread` recomputes from zero.
4. The portable tail value and conflict identity contain no source paths,
   byte offsets, provider record ordinals, provider IDs, `history_base`
   references, host/replica/machine IDs, or subagent data. Existing
   `ConversationThread` provenance fields and reidentified provider UUIDs
   inside runnable transcripts stay in their existing locations and are never
   conflict inputs.
5. Tail equality is meaningful only under a compatible codec, partition, and
   start, with proven accepted-manifest ancestry. Tail evidence never
   authorizes a stale Push; the verified current HEAD is the only CAS parent.
6. No conflict decision reads `family_id`, machine ID, title, provider fork
   ancestry, or `forked_from`.
7. Pull never writes cloud state and never moves or deletes provider sources.

### 1.1 Lifecycle example

```text
A: base P0: p1..p5;  child P1: history_base(P0@5) + m1,m2,m3
logical T = p1..p5,m1..m3        portable tail = m1..m3, start after p5

first Push:  cloud objects = standalone(p1..p5,m1..m3)
             cloud tail    = {start: after p5, end: 8, state: Htail(m1..m3)}

A appends m4:        accepted = Htail(m1..m3), observed = Htail(m1..m4)
next Push:           reconstructs p1..p5,m1..m4; after CAS the cloud tail
                     end/state advance; the start is unchanged.

B Pulls:             installs one standalone null-base file, maps the start
                     after p5 by canonical index, verifies the refolded tail,
                     stores accepted = observed = Htail(m1..m4).
B appends m5:        observed = Htail(m1..m5) — cumulative, never Htail(m5).
```

A root with no native base has start zero and its tail state is the
whole-thread canonical state, so ordinary roots keep current whole-thread
comparison semantics.

## 2. Standalone graph normalization

### 2.1 Flattening

A Codex rollout may contain only a final segment. Uploading a leaf alone
would leave the cloud object dependent on a source machine. Every selected
conversation therefore passes through one bounded standalone builder:

```text
local rollout graph
  -> bounded history_base dependency resolution
  -> flattened canonical root chain (one per logical task)
  -> deterministic standalone encoding (history_mode: paginated,
     history_base: null)
  -> full verification and hashing
  -> immutable object publication -> head CAS   (Push only)
```

History-base edges connect physical reconstruction segments of one logical
root: ancestors are flattened in and the edges disappear from output and
metadata. Subagent references are transport structure: the builder retains
and consistently rewrites them so the snapshot is runnable. Retained inline
ancestor `session_meta` records stay ordinary records; the parser never
recreates history relationships or extra descriptors from them.

Missing children, ambiguous ownership, cycles, duplicate owners, unsupported
identity-bearing fields, or an unresolved base make the resource
`Blocked`/`StoreOnly`. Physical tail mapping is segment-aware: only the
append-bearing `FollowEof` leaf may carry the physical tail cursor; zero or
multiple eligible leaves blocks.

### 2.2 One builder, two modes

| Mode | Root provider ID | Use |
| --- | --- | --- |
| `PreserveIdentity` | Selected local ID | Build Push objects |
| `Reidentify` | Plan-persisted fresh ID map | Install Pull targets |

Both modes resolve dependencies, rewrite supported identity edges, emit
paginated/null-base output, preserve the root content and subagent closure,
and stream every output through SHA-256. There are no separate upload and
Pull-install algorithms.

### 2.3 Artifact result

Reconstruction happens at apply, not planning (Sections 8-9), and produces:

```rust
struct StandaloneConversationArtifact {
    root_provider_thread_id: String,
    root_bytes: Vec<u8>,
    subagents: Vec<StandaloneSubagentArtifact>,
    id_map: Vec<StandaloneThreadAllocation>,
    portable_tail: PortableConversationTail,
    message_count: u64,
    lineage_digest: String,
    semantic_digest: String,
    artifact_sha256: String,
}
```

`artifact_sha256` is domain-separated over the complete root output and the
ordered `(file role, provider ID, byte length, SHA-256)` file list. It is
integrity/provenance for one immutable artifact version; it never feeds
classification.

## 3. Schema-1 in-place cutover

Every existing numeric schema constant stays `1`; no new constant is added
(the root tail layout lives on the existing binding document). The cutover
replaces the schema-1 contract in place: the release has exactly one accepted
shape per persisted document, enforced at the persistence boundary before
domain conversion:

1. the top-level schema field must equal `1`, under its existing key
   (`schema_version` for bundle/head/plan documents, `schema` for local
   documents) with no aliasing between the two;
2. one strict wire DTO per document, closed against unknown keys
   (`deny_unknown_fields` or an equivalent key-set check) — no Serde
   defaults, aliases, untagged legacy variants, or missing-field synthesis
   for any field this cutover adds or removes;
3. every nested bound, discriminant, and cross-field invariant is validated
   before constructing the domain value; and
4. writers serialize only the current DTO and read it back through the same
   validator before publication or atomic replacement.

Old documents are not rewritten or upgraded: a document that fails the
current validators fails closed, and the user starts from provider state or a
newly published repository. The unchanged number cannot fence an old binary,
so all older Mallard processes must be excluded from a namespace before the
new release opens it; mixed-version access is unsupported. Rejected shapes
need no enumeration — unknown-key closure already rejects every removed or
never-shipped field, including the retired `fork_stamp`.

The portable wire value, required for every Codex `ConversationThread` and
optional for providers that have not adopted the codec:

```rust
struct ConversationThread {
    // Existing fields remain, including message_count, lineage_digest,
    // and semantic_digest over the complete flattened root chain.
    portable_tail: Option<PortableConversationTail>,
}

struct PortableConversationTail {
    codec: String, // exactly "mallard-codex-portable-tail-v1"
    partition_kind: TailPartitionKind,
    start_record_count: u64,
    accepted_end_record_count: u64,
    accepted_tail_state: Sha256Digest, // lowercase hex on the wire
}

enum TailPartitionKind {
    WholeThread,        // "whole_thread"
    FixedPaginatedTail, // "fixed_paginated_tail"
}
```

Validation rules:

- `Some` is mandatory when `provider == codex`; absence, an unknown codec
  string, an unknown partition tag, or violated bounds fail closed.
- Classification equality is field-wise equality of the complete five-field
  value. There is no derived fingerprint and no serialized tail count; the
  count is `accepted_end_record_count - start_record_count`.
- `WholeThread` requires `start_record_count == 0`. A `FixedPaginatedTail`
  whose verified boundary is zero stays explicit; kinds never reclassify.
- `start_record_count <= accepted_end_record_count`, and in cloud metadata
  `accepted_end_record_count == message_count` (the accepted tail always ends
  at the complete canonical root end). Locally, the observed end may exceed
  the accepted end; the start never differs.
- `partition_kind` and `start_record_count` are immutable across accepted
  cloud successors; a moved start makes ancestry `Unknown`.

## 4. Tail codec v1

There is one canonical content projection: the flattened canonical message
chain (`CanonicalConversationMessage`) produced by the existing
`codex.lineage.v1` codec, extended to resolve `history_base` chains before
analysis. `lineage_digest` and `semantic_digest` remain the existing
whole-chain folds; `accepted_tail_state` is a windowed fold over the same
per-message material. No second canonicalization or message identity rule is
introduced.

Hash conventions match `conversation_chain_digests`: `LP(bytes)` is `u64`
big-endian byte length followed by the bytes; `LP_OPT(x)` is `0x00` for
absent or `0x01 || LP(x)` for present; counts and indexes are 8-byte
big-endian; digests are raw 32 bytes inside hashing and lowercase hex on the
wire.

```text
S0 = SHA256(
       LP("mallard-codex-tail-start-v1") ||
       LP(partition_tag) ||
       U64(start_record_count))

S(n+1) = SHA256(
       LP("mallard-codex-tail-record-v1") ||
       U64(canonical_index(m)) ||        // absolute index in the flattened chain
       S(n) ||
       LP(m.message_id) ||
       LP_OPT(m.parent_message_id) ||
       LP(m.semantic_fingerprint))
```

- `accepted_tail_state` is `Sn` after folding every canonical message with
  `start_record_count <= canonical_index < accepted_end_record_count` in
  chain order.
- An empty tail (`start == end`: a freshly continued rollout with no new
  messages) has state `S0`. `S0` binds the partition tag and start, so empty
  tails at different boundaries are unequal and no state is an unqualified
  `H(empty)`.
- One-shot computation must equal incremental extension from any saved
  `(count, state)` split point. Saved pairs are machine-local; they are not
  wire values.
- Any change to domains, framing, fields, or the projection requires a new
  codec string; there is no in-place v1 evolution.

Golden vectors are shared fixtures across domain, capture, manifest
verification, and resolver tests: empty tails at zero and at a nonzero
boundary, single-record extension, a multi-record tail checked at every split
point, and a fixture that changes every subagent byte and proves all vector
values unchanged.

## 5. Machine-local state

### 5.1 Binding layout

The immutable root layout is stored on the existing machine-local
conversation binding, whose revision is the concurrency generation:

```text
LocalConversationBinding += root_tail_layout: Option<RootTailLayout>

struct RootTailLayout {
    codec: String,
    partition_kind: TailPartitionKind,
    start_record_count: u64,
}
```

A bound Codex task without a layout is corruption: recover it from an exact
journal/checkpoint/cloud witness or fail closed — never re-seed a standalone
null-base file at `WholeThread@0`. Because layout and binding live in one
document, seeding is a single atomic document replacement with no
cross-document ordering.

### 5.2 Destination checkpoints

Accepted cloud state is destination-scoped on the `ProjectStorageLink`,
keyed by `(local_project_id, storage_id, sync_scope, thread_id)`; two links
may have accepted different versions of the same alias:

```rust
struct AcceptedConversationVersion {
    artifact_sha256: Sha256Digest,
    accepted_at_commit_id: CommitId,        // immutable provenance,
    accepted_at_manifest_sha256: Sha256Digest, // never the CAS base
    portable_tail: PortableConversationTail,
}

struct ObservedTailState {
    portable_tail: PortableConversationTail, // observed end/state
    accumulator: (u64, Sha256Digest),        // saved (count, state) resume pair
    root_mapping: RootPhysicalMapping,       // file identity + cursors
    raw_range_pin: RawRangePin,              // staleness pin for review/apply
}

enum PhysicalMappingState {
    Ready,
    Detached { expected_provider_thread_id: ProviderThreadId,
               expected_binding_revision: u64 },
}

struct ConversationCheckpoint {
    generation: u64,
    accepted: Option<AcceptedConversationVersion>,
    observed: ObservedTailState,
    physical: PhysicalMappingState,
    prefix_witness: Option<(String, String)>, // FixedPaginatedTail only; local
    binding_revision: u64,
}
```

- `observed` values are replaceable Refresh cache, never approval. Planning
  copies exact reviewed observations into immutable plan pins; apply never
  incorporates a newer observation into an older plan.
- `Detached` is the single physical-invalidation state, set when another
  operation replaces the bound alias (Section 9.4). It preserves accepted
  cloud evidence, clears the physical mapping, and does not pre-classify the
  eventual relation: the next Pull derives fresh `L` from the binding layout
  and runs the ordinary classifier.
- The `prefix_witness` is the existing whole-chain digest pair truncated at
  the boundary, over canonical messages `0..start_record_count`. It is
  machine-local, never serialized to cloud, and never a classification
  input; Section 5.4 defines where it is verified.

### 5.3 Seeding

Seeding runs once per root task, inside the shared observation path, the
first time the root is fully parsed — normally at first discovery/Refresh,
at latest inside Push planning's observation step. It is observation-class
work: it derives and persists the layout and checkpoint, builds no artifact,
and uploads nothing.

1. Resolve the append-bearing rollout's complete `history_base` chain and
   flatten one canonical chain.
2. No native base: `WholeThread` at zero. Native base: `FixedPaginatedTail`
   with `start_record_count` equal to the canonical message count contributed
   by the resolved base at the leaf's exact native cutoff. The byte cutoff,
   ordinal cutoff, and canonical count must agree; disagreement blocks. An
   ancestor's own base never creates a second partition. Never seed from a
   matching provider ID or ordinal alone.
3. Fold the tail state from the start through the current end; store the
   layout on the binding and the checkpoint with cursor, accumulator, and
   prefix witness.

Seeding is deliberately not deferred to Push apply. Planning must fully read
the chain anyway to prove a capturable local `L` and pin raw range hashes,
so the fold is marginal. Persisting the seed at first observation keeps the
start immutable while the thread is still unpushed — which is what lets
repeated Refreshes of a large paginated thread ride the cursor instead of
re-parsing from zero — and pins the boundary at the earliest observed leaf;
waiting would let a later compaction move the would-be boundary deeper and
shrink the content the tail identity covers. An abandoned plan leaves only
this inert layout: it contains no cloud state, and re-derivation from the
same native cutoff reproduces the same value.

When Pull installs a standalone null-base file, the layout is imported from
the verified cloud value (Section 9.3), never re-derived from the file's null
native base.

### 5.4 Append-only trust boundary

Supported Codex releases append; edit/backtrack creates a new rollout.
Within that contract:

- Refresh detects file replacement, truncation, identity changes, shortened
  or malformed records, and appends. `FixedPaginatedTail` refresh validates
  identity, kind, length, and safe record boundaries, then reads only
  complete records after the cached cursor; `WholeThread` recomputes from
  zero.
- Every operation that full-parses the chain anyway — seeding, Push apply
  reconstruction, Pull materialization, Detached recovery — recomputes and
  verifies the prefix witness (and, at Pull install, the manifest
  lineage/semantic digests, which prove the complete content including the
  prefix). A mismatch is a typed corruption block, never a relation.
- Residual gap: a same-length in-place mutation before the fixed start is
  invisible to classification between full-parse operations. It cannot be
  published (witness) or installed (manifest digests); the exposure is a
  stale between-syncs relation display, outside the supported provider
  contract and pinned by compatibility tests.

## 6. Refresh

Refresh is checkpoint-driven: it never reconstructs, uploads, downloads
transcript objects, or moves accepted state. For each bound logical root:

1. Reconcile any nonterminal journal touching the task, destination/link, or
   alias. Capture a coherent provider/profile inventory and the selected
   link revision.
2. Reconcile session/archive paths with canonical no-follow regular-file
   checks. A missing, shorter, replaced, or ambiguous source yields a typed
   invalid result, never an empty tail.
3. Load the binding layout (Section 5.1). If the checkpoint is `Detached`,
   return `RequirePull` without using the accepted tail as a physical prefix.
4. For a ready `FixedPaginatedTail`, extend the saved accumulator from the
   cursor over newly appended complete records; for `WholeThread`, recompute
   from zero. Read only complete newline-terminated records through a stable
   safe end; never read past a fixed ancestor cutoff. Re-stat sources and
   retry a bounded number of times if ends moved, then return `Busy`.
5. Derive observed `L`, derive `C` from the verified current manifest
   (manifest-only; no object downloads), and call the shared classifier.
6. Persist only observation after-state, with an atomic compare against the
   link revision, checkpoint generation, binding revision, source identity,
   and prior observed end, and losing to any reservation created by a
   Push/Pull journal after the scan. Refresh can never roll back a
   generation promoted by Push/Pull.

## 7. Shared relation classifier

One provider-neutral domain function is the only relation implementation:

```rust
fn compare_conversation_tail(evidence: TailComparisonEvidence)
    -> TailClassification;
```

Evidence carries the accepted pin `P`, local `L`, cloud `C` (each a complete
five-field tail value or a typed absent/invalid marker), and the validated
gates: codec/partition/start compatibility, local root provenance, the
accepted pin's immutable commit/manifest witness, verified cloud ancestry
from that witness, and exact ScopeBase/HEAD coordinates. Any failed or
ambiguous gate is `Unknown`; a value match cannot override a gate.

With `P`, `L`, `C` present and valid, in order:

| Predicate | Relation |
| --- | --- |
| `L == C` | `Sync` (`Current` when `P == C`, else `Advanced`) |
| `C == P && L != P` | `LocalOutstanding` |
| `L == P && C != P` | `CloudOutstanding` |
| otherwise | `Divergent` |

Directional absence is decided first: valid local with no cloud thread and no
pin is `MissingCloud`; valid cloud with no local binding and no pin is
`MissingLocal`; confirmed absence of both without a pin is `MissingBoth`; an
expected side missing after `P` exists is `Unknown`; both sides present
without a pin is `Unknown` — Pull does not bootstrap by comparing graphs.

There is deliberately no containment refinement: for
`P = m1..m5`, `L = m1..m6`, `C = m1..m7` the result is `Divergent` even when
one side is a canonical prefix of the other. If the local turn had been
pushed, the pin would include it; a nonterminal Push journal gates all
operations until recovery. The accepted residual is an occasional extra
keep-both.

This cutover replaces `classify_thread_refresh` and its containment branches
(`CloudPrefixOfLocal` via prefix recompute) with this conservative comparator
for every provider. Non-Codex providers supply `WholeThread`-equivalent
evidence from their existing `(lineage_digest, semantic_digest,
message_count)` values; their behavior change is that unpinned both-present
cases and unpinned local-ahead cases now classify `Unknown`/`Divergent`
instead of inferred fast-forwards. Refresh, Push, and Pull only map the
returned relation to their own action; no command or provider module may own
a second truth table or refine the result, enforced by a source-level
architecture test.

## 8. Push

### 8.1 Planning

Planning classifies and pins; it does not reconstruct, hash output, or build
a candidate. For each selected Codex thread:

1. Reconcile journals; load the destination checkpoint, accepted `P` and
   witness, binding layout/revision, and stored ScopeBase.
2. Run the ordinary Refresh observation path to freeze `L` with its source
   identities, cursor, cumulative state, and raw append-range pin. The raw
   pin covers the complete unaccepted physical interval
   `[accepted_end or tail_start, planned_end]` for `FixedPaginatedTail`, or
   the complete range for `WholeThread`, even when earlier Refreshes cached
   parts of it.
3. Build classifier evidence from the verified current manifest and immutable
   ancestry — no transcript object downloads — and call the classifier once:

   | Relation | Push result |
   | --- | --- |
   | `LocalOutstanding` | Publish |
   | `MissingCloud` | `CreateCloud` |
   | `Sync(Current)` | `NoChanges` |
   | `Sync(Advanced)` | `RequirePullMetadata` — Push does not run Pull's path |
   | `CloudOutstanding`, `Divergent` | `RequirePull` |
   | `Unknown` | `Blocked` |
   | `MissingLocal`, `MissingBoth` | `NotPushable` |

   A selected batch containing a blocking relation publishes nothing.
4. Require a seeded binding layout — the step-2 observation path performs
   first-contact seeding (Section 5.3), which is observation-class work, not
   reconstruction. For `LocalOutstanding`, require the existing start
   unchanged.
5. Resolve and pin the bounded root/history/subagent source closure —
   identities, fixed cutoffs or exact EOFs, and raw range hashes — as
   incidental artifact sources, not classifier inputs.
6. Pin the exact classifier evidence and relation, local document
   preconditions, stored ScopeBase, and the exact current manifest/HEAD.
   The stored ScopeBase may be an older verified ancestor of the current
   HEAD; Push builds on the current manifest, preserves unrelated entries,
   and CASes the current HEAD, while a concurrent local ScopeBase change or
   unproved ancestry stales/blocks the plan.

### 8.2 Apply

1. Revalidate plan expiry, revisions, generations, source pins, and
   ScopeBase/HEAD evidence; call the classifier again and require the exact
   planned relation.
2. Consume each validated no-follow source once and reconstruct every
   selected graph exactly once in `PreserveIdentity` mode, streaming outputs
   through SHA-256 in the same pass. Require the reconstructed cumulative
   tail to equal planned `L` and the prefix witness to match. Re-stat
   `FollowEof` sources; a post-plan append stales the plan before upload.
3. Upload content-addressed objects, the immutable manifest, and the
   candidate commit. Uploads are orphan-safe.
4. Atomically persist a `CandidateUploaded` journal — containing the
   candidate identifiers, full hashes, tail before/after, classifier pins,
   and exact whole-document `SyncConfig` before/after states — while
   reserving the touched resources, conditional on all local preconditions
   still matching. A Refresh during upload stales this step, leaving only
   orphan objects.
5. Recheck the reviewed HEAD and advance mutable HEAD once by CAS.
6. On success, commit the local after-state: accepted = observed = planned
   tail, physical accepted end = reviewed source end, accepted
   `artifact_sha256`/commit/manifest witness, ScopeBase = CAS result, next
   generation. Write the receipt and terminalize `Complete`; a
   receipt-persistence failure after the local commit terminalizes
   `CompleteWithWarnings` and regenerates the receipt from the journal.

Journal phases: `CandidateUploaded`, `HeadAdvanced`, `LocalCommitted`,
`Complete`, `CompleteWithWarnings`, `AbortedCasLost`,
`RecoveryBlockedCorruption`.

### 8.3 Recovery

After an ambiguous CAS or crash, inspect verified immutable ancestry: if the
current HEAD is the candidate or a verified descendant, install the exact
journaled after-state (a later Refresh observes any newer head); if ancestry
excludes the candidate, terminalize `AbortedCasLost`, leave local acceptance
unchanged, release reservations, and require Pull. Orphan objects never
justify promotion. Recovery never recaptures newer records under an old
approval and never invents a tail start. Terminal recovery idempotently
releases reservations and staging.

## 9. Pull

### 9.1 Classification

Pull planning verifies the reviewed immutable current commit/manifest, the
selected thread entry and its portable tail, and HEAD/scope coordinates. If a
pin exists, it verifies the accepted commit/manifest witness, re-derives the
accepted tail and `artifact_sha256` from that immutable entry, requires
equality with local `P`, and requires the reviewed head to be that commit or
a verified descendant with an unmoved start. It then calls the shared
classifier. No transcript object is downloaded to classify.

### 9.2 Actions

| Relation | Action |
| --- | --- |
| `Sync(Current)` / `Sync(Advanced)` | Metadata-only: adopt verified `C` as accepted, align the accepted end with `L`, advance ScopeBase. No provider files change; differing cloud subagents are not installed. |
| `LocalOutstanding` | Metadata-only: keep `P` accepted and the longer observed `L`; report Push eligibility. |
| `CloudOutstanding` | Materialize the verified cloud graph; accepted = observed = `C`. |
| `Divergent` | Keep-both (Section 9.4). |
| `MissingLocal` | Create binding + layout from verified cloud metadata and materialize. |
| `MissingCloud` | Report local-only/Push eligibility; Pull creates no cloud state. |
| `MissingBoth` | No-op/not-found; nothing is created. |
| `Unknown` | Block for explicit repair/review. |

Metadata-only actions perform one atomic whole-document `SyncConfig`
replacement under the link-revision CAS, with no operation journal: a crash
leaves either the complete old or complete new document.

### 9.3 Materializing apply

Planning pins the immutable commit/manifest, object keys/sizes/SHA-256
values, fixed provider IDs for the one remote target, target paths, local
preconditions, and semantic after-templates. It downloads nothing and stages
nothing.

Apply, under the host-operation and provider-home locks:

1. Revalidate revisions, pins, target absence, and the reviewed HEAD/scope;
   recheck the local documents' exact before-states.
2. Download each pinned object once and verify every SHA-256. Reconstruct and
   reidentify the standalone graph exactly once in `Reidentify` mode,
   streaming emitted bytes through SHA-256 into staging while validating
   typed invariants: expected owners and roles, paginated/null-base outputs,
   manifest lineage/semantic digest equality (which proves the complete
   content, prefix included), a unique canonical position for
   `start_record_count`, and a refolded tail equal to
   `accepted_tail_state`. A failure blocks before any journal or visible
   mutation.
3. Journal the apply-generated target pins and the exact whole-document
   before/after states for `SyncConfig` and `ConversationBindings`, and
   reserve the touched resources. This is the durable commit-decision
   boundary: later HEAD or source changes do not revoke the reviewed
   snapshot.
4. Install targets no-clobber; a foreign target collision terminalizes
   `AbortedProviderCollision`, preserving the original graph and recording
   committed Mallard targets as unbound recoverable orphans. Hash-equal but
   different-inode targets are not accepted.
5. Commit bindings (alias, layout, revision) and `SyncConfig` (checkpoint
   with imported layout, accepted = observed = `C`, prefix witness, physical
   cursor, ScopeBase) as their exact journaled after-states, each document
   atomically, recovery rolling the remaining document forward under the
   gate.
6. Refresh the provider index, write the receipt, terminalize, release.

Old sources are never moved or deleted; every source receipt records the
single disposition `Retained`. Cleanup is a separate future operation.

Journal phases: `TargetStaged`, `ProviderCommitted`, `LocalCommitted`,
`Complete`, `CompleteWithWarnings`, `AbortedPrecommit`,
`AbortedProviderCollision`, `RecoveryBlockedCorruption`.

### 9.4 Divergent keep-both

```text
Before:  T -> P-local
After:   C -> P-local    existing local rollout, unchanged bytes
         T -> P-remote   cloud history materialized once, still tracking T
```

Planning requires the pinned `Divergent` result, proves `P-local` is still
`T`'s exact current alias and safe and unique to rebind, mints logical `C`
(Mallard's existing `ThreadId` allocator) and the `P-remote` provider IDs,
and pins both layouts and exact document before/after states. If `P-local`
cannot be proven safe to rebind, Pull fails closed.

Apply extends the Section 9.3 steps: the one bindings-document mutation moves
the `P-local` alias and its unchanged layout to `C` and installs the
`P-remote` alias with the verified remote layout on `T`. The destination's
`T` checkpoint is re-keyed to `C` as local-only observed state — observed
tail, mapping, accumulator, and witness preserved; accepted pin and accepted
physical end cleared — and a fresh `T` checkpoint installs accepted =
observed = `C`. The binding move edits no rollout byte, no `session_meta`,
and no provider index entry; `P-local`'s subagent closure simply stays
attached to the unchanged local root.

`C` has no accepted cloud version and no family/provenance link to `T`; its
first `CreateCloud` Push builds the standalone artifact from the preserved
layout, which recovery never replaces with boundary zero. Other storage links
that referenced the replaced alias keep their independent accepted pins and
become `Detached`; their next Pull runs the ordinary classifier and may
perform a second keep-both.

Recovery resumes from the last durable phase with the same planned IDs,
paths, layouts, and hashes; it never calls Codex, never remints, never
converts a null base to boundary zero, and never resets a cumulative state.
Before the Section 9.3 journal exists, stale pins terminalize
`AbortedPrecommit`. A third document value that is neither the journaled
before nor after state is `RecoveryBlockedCorruption`, gated rather than
retried. Once bindings/SyncConfig are durable, ancillary index/receipt
failures are deferred warnings under `CompleteWithWarnings`.

## 10. Provider ID allocation

All IDs are minted during planning and persisted in the plan; apply and
recovery never mint or invoke Codex. For new Codex provider IDs, prefer one
bounded headless CLI session per plan: disposable `CODEX_HOME` and CWD,
`initialize`/`initialized`, `thread/start` with `ephemeral: true`, no
`turn/start`, accept only a canonical lowercase RFC 9562 UUIDv7, then
terminate and remove the temporary root. On any failure, generate
cryptographically random UUIDv7 values locally with the same
version/variant/lowercase validation. Both paths collision-check provider
inventory, indexes, active plans, and journals. Existing discovered
`provider_thread_id` values keep the adapter's current bounded validation.

## 11. Ownership and cutover boundaries

`mallard-domain` owns the portable/wire types and validation, the destination
checkpoint types, the sole `compare_conversation_tail` implementation
(suggested: `crates/mallard-domain/src/conversation_tail_relation.rs`), the
tail accumulator definition, and plan/journal/receipt types. It parses no
JSONL and touches no filesystem.

`mallard-infrastructure` owns bounded Codex inventory and `history_base`
resolution, chain flattening feeding the existing `codex.lineage.v1`
canonicalization, tail folding and physical mapping, standalone
reconstruction/reidentification and verification, no-clobber
materialization, object publication/CAS mechanics, and transaction recovery.
`codex_rollout.rs` remains the bounded single-file parser; new modules
compose files into a graph.

`src-tauri/src/project_sync/commands.rs` orchestrates plans, locks, document
mutations, and receipts. It implements no parsing, hashing, mapping, or
relation logic.

Removed in the same cutover, with no compatibility path: the native-fork
conflict model (`codex_native_fork.rs`, fork `ConversationSyncPhase`
variants, `CodexForkAttempt`, executors, attempt records, naming phases),
`ForkStamp`/`fork_stamp`, manual checkpoint DTOs, legacy overwrite paths,
and `classify_thread_refresh`'s containment refinement (superseded by the
shared comparator). Portable `forked_from` remains as unrelated provenance;
this resolver writes `None` and never reads it.

## 12. Safety rules

- Walk only canonical `sessions/`/`archived_sessions/` roots with no-follow
  traversal and regular-file validation; reject symlinks, special files,
  duplicate active IDs with different bytes, path escapes, cycles, and
  exceeded bounds. Honor both byte and ordinal history cutoffs. Never upload
  a raw dependent leaf, never omit an unrecognized subagent, never
  recursively delete provider files.
- One reviewed publishing batch creates one immutable candidate and one head
  CAS; non-publishing relations create neither. Observation cache never
  authorizes publication; tail equality never bypasses
  witness/ancestry/CAS validation; differing `artifact_sha256` values never
  refine a relation.
- Journal creation and resource reservation are one conditional persistence
  transaction; normal entrypoints reconcile durable nonterminal journals
  before trusting any index. Terminal completion and reservation release are
  atomic where possible, and terminal recovery idempotently clears leftovers.
- Incomplete trailing JSONL is never checkpointed; concurrent append yields
  bounded retry then `Busy`, never mixed state.

## 13. Backend test plan

All tests use temporary Mallard/Codex/project/storage roots and never touch a
real user home.

Schema and domain: schema-1 Codex threads require a valid five-field
`portable_tail`; strict DTOs reject unknown keys, missing required fields,
and Serde-default synthesis; all schema constants remain `1`; golden serde
fixtures freeze wire field names and key choice (`schema_version` vs
`schema`); a simulated old writer publishing a tail-less schema-1 successor
is detected and blocked on the next verified read; `WholeThread` requires
start zero while a zero-boundary `FixedPaginatedTail` stays explicit; a moved
start across successors is `Unknown`; accepted versions for two links remain
independent; binding-without-layout is rejected and recovered only from an
exact witness.

Codec: the Section 4 golden vectors; one-shot equals incremental at every
split point, including across a simulated Pull (fold on A, map and extend on
B); provider-ID/CWD/path/ordinal rewriting and all subagent changes leave
every vector unchanged; empty tails at distinct boundaries are unequal; the
`P0/P1/P2` chain seeds start after `m3`, tail `Htail(m4,m5)`, and a
standalone artifact covering `m1..m5`; depth-1/3/max chains and 0/1/max
subagent counts produce the identical wire shape.

Classifier: one exhaustive provider-neutral matrix — the four-row table with
pin alignment, all directional-absence outcomes, expected-missing-after-P,
and every invalid/ambiguous gate; `m1 / m1,m2 / m1,m2,m3` is `Divergent`
with no intermediate-count replay; a completed Push of the local turn makes
the follow-up `CloudOutstanding`; equal tails with entirely different
subagent inventories are `Sync`; an architecture test rejects any second
P/L/C comparison site.

Refresh: unchanged `FixedPaginatedTail` reads no bytes before the cursor
while `WholeThread` recomputes from zero, both matching one-shot results;
Refresh never promotes accepted state and loses its final CAS to a
reservation created after its scan; missing/shortened/replaced/ambiguous
sources yield typed results, never an empty tail; a `Detached` checkpoint
returns `RequirePull`.

Push: planning produces no output bytes/hashes and only
`LocalOutstanding`/`CreateCloud` reach reconstruction (test spies); the
action map is exact, including `Sync(Advanced) -> RequirePullMetadata`;
apply's reconstructed tail must equal planned `L` and a mutated prefix is
blocked by the witness; the raw pin covers the complete unaccepted interval
after cached Refreshes; append-after-preflight stales before upload; real
two-writer CAS races leave one winner, `AbortedCasLost` releases the gate,
and orphan objects never promote; candidate-win-then-newer-descendant
installs the exact candidate state first; an older verified ScopeBase with
unchanged `T` publishes onto the current descendant HEAD while unproved
ancestry blocks.

Pull, conflict, recovery: classification downloads no objects; materializing
actions verify every object SHA and the full invariant list before install;
installed accepted/observed states equal `C`, never empty; metadata-only
actions are single atomic `SyncConfig` replacements with no journal and
survive crash as exactly old-or-new; keep-both yields `C -> P-local`
unchanged (byte-identical rollout, no provider index edits) and
`T -> P-remote` with the remote layout; `C`'s first Push publishes the
preserved layout with no family metadata; crash injection after every journal
phase reaches one deterministic fixed point with the same IDs, bytes, and
bindings; foreign-target collisions abort safely with no clobber; every
source receipt is `Retained`; a `Detached` other link classifies via the
ordinary path and can keep-both again; recovery never allocates, invokes
Codex, moves a start, or resets a state.

Cross-machine scenario (local-folder and S3/R2 stores, with real head
races): A pushes `history(base@5)+m1,m2,m3`; B pulls the standalone file and
maps the same start; A appends `m4` and pushes; B appends conflicting `m5`,
is blocked as `Divergent`, pulls into keep-both; B then appends `m6` to `T`
and pushes; A pulls `m6` as `CloudOutstanding` with the partition preserved;
`C` publishes independently.

Codex compatibility and allocator: supported releases preserve bytes before
accepted cursors during resume/append; edit/backtrack creates a separate
rollout; reconstructed roots/subagents can be listed, resumed, appended, and
forked without their original dependencies; fake app-server fixtures cover
the successful UUIDv7 path and every fallback (missing binary, init failure,
timeout, EOF, malformed/non-v7 response, collision, early exit) plus
disposable-home/no-`turn/start`/unchanged-real-home assertions.

## 14. Implementation sequence

1. Tail codec: accumulator, golden vectors, one-shot verifier, flattened
   canonical chain assembly over `codex.lineage.v1`.
2. Wire cutover: five-field `portable_tail` on `ConversationThread`, strict
   schema-1 DTO validation, binding `root_tail_layout`, destination
   checkpoint types, `Detached` state; delete the old delta/fork state.
3. Seeding, prefix witness, and the append-only trust boundary.
4. `compare_conversation_tail` with the exhaustive matrix; route Refresh
   through it; delete `classify_thread_refresh` containment refinement.
5. Standalone builder both modes over the flattened chain, with full
   verification and the CLI-first/UUIDv7 allocator.
6. Push planning/apply/recovery per Section 8.
7. Pull metadata-only path, materializing path, keep-both, and recovery per
   Section 9.
8. Remove the native-fork model, `ForkStamp`, legacy DTOs/journals, and
   tests asserting legacy success; retain negative fixtures proving
   non-current shapes fail closed.
9. Cross-machine, crash-matrix, S3/R2, and opt-in Codex compatibility tests.
10. Update backend architecture docs after shapes are frozen by tests.

## 15. Acceptance criteria

1. Every uploaded Codex conversation is a complete standalone graph; every
   output file is paginated with a null native base.
2. Publishing Push hashes and verifies complete standalone output regardless
   of how little changed; every other relation exits before reconstruction,
   and planning never reconstructs.
3. Roots without a native base are explicit `WholeThread` at zero;
   append-bearing paginated roots are `FixedPaginatedTail` at a verified
   canonical boundary; ancestors and subagents receive no portable state.
4. The cloud stores exactly the five-field root tail value — no node
   vectors, topology digests, prefix witnesses, derived fingerprints, or
   subagent/attachment metadata — and classification equality is field-wise
   equality of that value.
5. The accepted start never moves; keep-both's binding move is the only
   layout replacement; accepted cumulative states advance on publishing Push
   or accepting Pull and never reset.
6. Pull maps the cloud start into the reidentified standalone file and
   preserves the same hashing partition across machines.
7. `FixedPaginatedTail` Refresh reads only appended records; `WholeThread`
   recomputes from zero; both commit the complete range from the fixed
   start.
8. `artifact_sha256`, manifest/object verification, ScopeBase evidence, and
   head CAS remain the integrity/concurrency authorities; tail equality
   alone never permits publication; classification never downloads objects.
9. One `compare_conversation_tail` function is the only relation
   implementation for all commands and providers, with no containment
   refinement anywhere.
10. Push maps relations exactly, including
    `Sync(Advanced) -> RequirePullMetadata`; metadata adoption is Pull's
    journal-free atomic `SyncConfig` replacement.
11. Divergent Pull preserves both histories with zero local rollout edits,
    leaves cloud unchanged, and lets `C` publish later without family
    metadata; sources are always `Retained`.
12. A mutated prefix cannot be published or installed; the residual
    same-length pre-start gap is contract-tested and documented.
13. Recovery is deterministic, no-clobber, and allocation-free, preserving
    exact descriptors, states, hashes, and bindings.
14. All schemas remain numeric `1` under exact-shape validation with no
    migration or legacy path, and no new schema constant is introduced.

## 16. Verification commands

```text
cargo fmt --all -- --check
cargo test -p mallard-domain
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Run opt-in Codex contract tests only with explicitly created disposable
provider homes.
