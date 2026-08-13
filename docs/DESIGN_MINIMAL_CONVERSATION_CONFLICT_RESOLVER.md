# Design: Portable-tail conversation conflict resolver

- Status: Proposed
- Date: August 10, 2026
- Scope: ProjectSync Codex conversation Refresh, Push, Pull, and divergence
- Compatibility: Clean cutover; no migration or legacy execution path
- Backend plan:
  [Portable Codex tails and minimal conversation conflict resolution](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
- Important-path pseudocode:
  [Minimal conversation conflict resolver](PSEUDOCODE_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)

## Decision

Keep the cloud `thread_id` as the stable shared identity. A provider thread ID
is a machine-local alias and may change after Pull or conflict resolution:

```text
cloud T <-> A's provider rollout P-A
cloud T <-> B's provider rollout P-B
```

Every publishing Push uploads a complete standalone Codex snapshot and
protects the shared cloud head with compare-and-swap. A stale Push is blocked
until Pull; it never overwrites the winner automatically.

For comparison performance, every Codex task also has one portable tail value
for the root conversation only. A native `history_base` on the append-bearing
root rollout seeds that partition once. Its canonical start is then immutable
for that accepted branch, is stored in cloud metadata, and survives
reconstruction into a standalone rollout whose native `history_base` is
`null`. Physical history segments and subagent rollouts never receive their
own comparison entries.

Subagents are snapshot payload, not conflict identity. Mallard still includes
the bounded subagent closure when a root-driven Push uploads a task or a
root-driven Pull materializes it, and it still verifies every object SHA-256.
Refresh and the shared Push/Pull classifier ignore subagent files, topology,
and state completely. Subagent differences create no relation or operation of
their own: there is no subagent-only Push/Pull, merge, or conflict decision.

The portable tail is cumulative:

```text
initial: Htail(m1,m2,m3)
append:  Htail(m1,m2,m3,m4)
```

A successful Push or Pull advances the accepted cumulative state. It never
moves the start or resets the tail to `H(empty)`.

## Four separate identities

| Value | Purpose | Authority |
| --- | --- | --- |
| Cloud `thread_id` | Shared logical task identity | Portable manifest |
| Provider thread ID | Alias used by one local Codex installation | Machine-local binding |
| Full artifact/object hashes | Byte integrity of the complete standalone root plus incidental subagent snapshot | Immutable objects and verified manifest; never a conflict-comparison input |
| Portable root-tail checkpoint | Stable root-only partial-hash partition and its accepted cumulative state | Portable thread metadata and the only conversation-content comparison input |

Machine-local byte offsets and provider record ordinals only map the portable
tail checkpoint into one installation's rollout files. They are not portable
identities.

No resolver decision requires a `family_id`, machine ID, title, provider fork
ancestry, or `forked_from` relationship.

## Portable tail schema

All numeric schema constants remain `1`. The cutover replaces the schema-1
Codex conversation contract in place: every Codex `ConversationThread` must
contain a valid `portable_tail` value, while other providers may omit it until
they adopt an equivalent codec. Old schema-1 shapes are unsupported and fail
the new exact structural/codec validation; there is no legacy interpretation.

Because the number does not distinguish old and new contracts, this is a
homogeneous clean cutover, not rolling compatibility. Old Mallard binaries and
writers must be excluded from the local state and repository after cutover,
and acceptance uses freshly initialized schema-1 state/storage. An old binary
may ignore the required field and drop it, so mixed-version access is outside
the supported safety contract.

Conceptually:

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

That is the complete portable wire value. It describes only the normalized
root conversation. It contains no subagent entry, prefix witness, separately
serialized tail count, or second comparison fingerprint. The complete
`artifact_sha256` and object inventory may cover subagent files for transport
integrity, but neither enters the shared classifier.

Runtime types may wrap these fields in validated domain types, but the
serialized field set above is exact. These properties are required:

- `partition_kind` and `start_record_count` are fixed once the root task is
  admitted;
- `start_record_count` and `accepted_end_record_count` are positions in the
  canonical root conversation projection, never raw JSONL ordinals;
- tail count is derived as `accepted_end_record_count - start_record_count`;
- the accepted end and cumulative tail state advance with accepted root
  conversation messages;
- canonical positions survive provider-ID, CWD, ordinal, and path rewriting;
- root conflict-record canonicalization reads the flattened root conversation
  projection, not every physical root JSONL record;
- it excludes subagent rollout contents, file inventory, provider IDs, and
  lifecycle/spawn/reference/control or child-result bookkeeping unless that
  record changes a canonical root conversation message;
- this is a projection rule, not permission to hide root message changes: if
  provider bookkeeping also carries changed root conversation content, that
  root message projection changes the tail;
- no cloud field contains a provider ID, path, byte offset, native ordinal,
  `history_base.thread_id`, host ID, or machine ID; and
- missing or malformed Codex tail metadata is rejected at the clean cutover.

`WholeThread` uses canonical start zero and retains the existing full-thread
comparison pass. `FixedPaginatedTail` uses the exact canonical boundary
selected by the root's original native `history_base` and may resume from a
saved physical cursor. The explicit kind prevents a pulled standalone file
from being silently reclassified merely because its new native metadata says
`history_base: null`.

The cloud stores only the accepted portable state. Each destination-local
checkpoint separately stores:

- the last accepted complete cloud `artifact_sha256` and immutable
  commit/manifest witness;
- the accepted portable tail checkpoint;
- the currently observed cumulative local tail checkpoint; and
- one root physical mapping for the fixed start, accepted end, and observed
  end.

The immutable root layout (`codec`, partition kind, and start count) is stored
directly on the existing machine-local conversation binding, whose revision is
the concurrency generation. Divergent keep-both moves the existing P-local
alias and its local layout to C, then installs the cloud layout and a newly
materialized P-remote alias on T in one bindings-document mutation. Moving the
alias changes Mallard metadata only; it does not rewrite the local rollout.

The cumulative state uses a versioned, domain-separated canonical accumulator.
For `FixedPaginatedTail`, a saved implementation cursor may process only newly
appended records, but its result always represents the entire range from the
fixed start. `WholeThread` recomputes from zero. The complete standalone object
continues to use ordinary full SHA-256.

## Lifecycle example

On A:

```text
base P0: p1 -> p2 -> p3 -> p4 -> p5
child P1: history_base(P0 at 5) -> m1 -> m2 -> m3

logical T: p1 -> p2 -> p3 -> p4 -> p5 -> m1 -> m2 -> m3
tail T:                                         m1 -> m2 -> m3
```

Before the first Push, A stores the portable fixed start and observes
`Htail(m1,m2,m3)`. The cloud Push stores:

```text
cloud object              complete standalone p1..p5,m1..m3
cloud full object hashes  Hbytes(complete standalone files)
cloud tail start          canonical boundary after p5
cloud accepted tail       Htail(m1,m2,m3)
```

A appends `m4`. Its local accepted and observed values are now:

```text
accepted = Htail(m1,m2,m3)
observed = Htail(m1,m2,m3,m4)
```

Push apply reconstructs and fully hashes `p1..p5,m1..m4` once. After its CAS
succeeds:

```text
cloud tail start    canonical boundary after p5  // unchanged
cloud accepted tail Htail(m1,m2,m3,m4)
```

B Pulls and installs a complete standalone file:

```json
{
  "history_mode": "paginated",
  "history_base": null
}
```

B verifies the cloud objects, maps the canonical boundary after `p5` into its
new physical file, and stores accepted = observed =
`Htail(m1,m2,m3,m4)`. If B adds `m5`, B computes the cumulative
`Htail(m1,m2,m3,m4,m5)`. B does not fall back to whole-thread hashing.

For a task that never had inherited history, the portable start is zero and
comparison remains a complete whole-thread canonical hash.

For a nested native dependency chain such as
`P0: m1,m2`, `P1: history(P0@m2)+m3`, and
`P2: history(P1@m3)+m4,m5`, the portable value still describes only the root.
Its fixed start is the canonical boundary after `m3`, its
cumulative tail is `Htail(m4,m5)`, and the complete standalone object and byte
hash cover `m1..m5`. P0 and P1 remain machine-local source segments only while
they are needed to reconstruct P2.

If `m4` and `m5` have subagent rollouts, they do not alter this value. For
example, these two snapshots compare `Sync`:

```text
local root tail = Htail(m4,m5); local subagents = s5_1,s5_2,s5_3
cloud root tail = Htail(m4,m5); cloud subagents = s5_1,s5_2,s5_3,s5_4
```

The same result holds if only subagent lifecycle, spawn/reference/control, or
child-result bookkeeping differs in the physical root rollout: those records
do not enter the root conversation projection. A changed root conversation
message does enter it, even if the provider emitted that change near subagent
control records.

A `Sync(Current)` Push uploads nothing. A `Sync(Advanced)` Push reports that
content is synchronized but local acceptance metadata is behind and requires a
Pull; Push does not invoke Pull's transaction. The corresponding Sync Pull
downloads no transcript object and advances only the accepted artifact/commit
pin and ScopeBase in one atomic SyncConfig replacement, without an operation
journal. `LocalOutstanding` uses the same journal-free metadata-only path. If a
later root change causes publication or materialization, the operation carries
that side's then-current complete subagent snapshot incidentally.

## Push contract

Push calls the same `P`/`L`/`C` tail-hash classifier as Refresh and Pull before
it reconstructs or uploads anything. It publishes only `LocalOutstanding` or
`MissingCloud`. `CloudOutstanding`, `Divergent`, and `Unknown` require Pull;
`MissingLocal` and `MissingBoth` are not pushable. `Sync` at P is a no-op, while
`Sync` at a newer verified C returns `RequirePullMetadata`; a zero-object Pull
advances P and ScopeBase.

For `LocalOutstanding`, current HEAD may be a verified descendant of the
stored scope base when T itself still has `C = P`. Push builds from that current
manifest, preserves its unrelated entries, and CASes the current HEAD. The
older stored scope base remains the exact local before-state. Unproved ancestry
or a concurrent local state change is `Unknown`/stale and blocks.

Push planning:

1. observes root L using its fixed portable partition and derives root C from
   the current verified manifest;
2. calls the shared classifier and requires `LocalOutstanding` or
   `MissingCloud`;
3. resolves, but does not reconstruct, the complete bounded root/history/
   subagent source closure; and
4. pins source identities, fixed cutoffs or exact EOFs, raw range hashes, the
   cumulative root tail, local document preconditions, and the exact cloud
   manifest/HEAD.

Planning produces no standalone output, output/artifact hash, candidate
manifest/commit, or accepted after-state.

Push apply:

1. verifies the source pins while consuming each source once;
2. reconstructs every selected rollout exactly once as standalone
   paginated/null-base output;
3. hashes and structurally verifies every output in one pass and requires the
   reconstructed cumulative root tail to equal the planned L;
4. builds and uploads immutable objects and the candidate manifest/commit;
5. journals the now-fixed candidate and local after-state; and
6. advances the reviewed current HEAD once by CAS.

After a successful or recoverably successful CAS, Mallard promotes accepted
tail = observed tail and advances physical accepted cursors. It does not clear
the tail. A CAS loser leaves accepted local state unchanged and must Pull.

Tail equality is comparison evidence only. Full artifact/object hashes,
verified repository ancestry, and head CAS remain authoritative.

## Pull and conflict rules

Let `P` be the destination's last accepted pinned portable tail, `L` the
current observed local portable tail, and `C` the verified current cloud
manifest's portable tail. Equality means the complete validated five-field
root-tail value—codec, partition, fixed start, end, and cumulative state—not an
unscoped SHA value. Full artifact hashes, file lists, subagent topology, and
subagent contents are deliberately absent from `P`, `L`, and `C`.

After proving cloud commit ancestry from `P` and validating the local physical
mapping, Refresh, Push, and Pull call one shared conservative comparison:

| Condition | Result |
| --- | --- |
| `L = C` | `Sync`; no content copy; Pull advances the accepted pin when `P != C` |
| `C = P` and `L != P` | `LocalOutstanding`; keep local and offer Push |
| `L = P` and `C != P` | `CloudOutstanding`; materialize cloud |
| `L != P`, `C != P`, and `L != C` | `Divergent`; preserve local and adopt cloud |
| A side is provably absent | Directional `MissingLocal`, `MissingCloud`, or `MissingBoth` |
| Pin, ancestry, layout, mapping, integrity, or absence is unproved | `Unknown`; block rather than guess |

`Sync(Current)` means P and C have the same root-tail value, not necessarily
the same commit ID or artifact SHA. A subagent-only or unrelated manifest
advance with an unchanged root tail therefore remains a no-op; a later root
Push may rebase onto that verified descendant HEAD.

The three operations do not duplicate or refine this table. They only map its
result to their own action: Refresh reports it, Push publishes only
`LocalOutstanding`/`MissingCloud`, and Pull advances the accepted pin,
materialization, or keep-both as required.

The current Pull review/apply split is:

```text
Pull planning:
  compare P/L/C
  metadata-only actions pin one SyncConfig mutation and stop
  materializing actions pin HEAD/manifest/object keys, sizes, and SHA-256 values
  for Divergent, pin the existing P-local alias/checkpoint for safe rebind
  allocate fixed IDs for one remote target
  pin target paths, local preconditions, and semantic after-templates
  do not download transcript objects, reconstruct, hash output, or stage files

Pull apply:
  compare P/L/C and revisions again
  metadata-only actions atomically update SyncConfig with no journal
  materializing actions download and verify pinned immutable objects once
  reconstruct/reidentify the one remote target once while validating typed
    invariants and streaming emitted bytes through SHA-256 into staging
  journal the apply-generated target pins and exact local mutations
  no-clobber install, then commit bindings and SyncConfig under recovery
```

Thus Pull review is metadata-only. An uninterrupted apply downloads each pinned
cloud object once, validates the typed reconstructed output, and serializes/
hashes that output once; it neither reparses nor rehashes its own stage. A
missing or corrupt object fails before journal creation or visible mutation.
Recovery may rehash the journaled stage after a process boundary, but never
downloads or reconstructs it again.

Pull deliberately does not refine a both-changed unequal result into an exact
prefix relationship. If A's exact local extension had already reached cloud,
A's successful Push would have advanced `P`; a crash leaves the Push journal
as the only allowed recovery path before another operation. Independently
generated turns on different machines are expected to differ. The rare case
where one independently generated tail happens to contain the other is safely
treated as an extra divergence/keep-both result.

This removes canonical local-versus-cloud graph relation comparison from Pull
planning. The immutable prefix before each fixed tail start is not rehashed
for relation classification. Complete cloud object verification remains
mandatory before materialization, but it verifies artifact integrity rather
than deriving a more precise local/cloud containment relation.

When materializing cloud `T`, Pull:

1. verifies the complete immutable object set and root-tail metadata;
2. reidentifies the standalone graph with plan-persisted provider IDs;
3. writes paginated/null-base provider files;
4. maps the root portable start and accepted end uniquely into the root file;
5. proves the recomputed cumulative tail equals cloud metadata; and
6. atomically installs bindings, destination checkpoints, and scope bases.

A missing, past-end, or ambiguous mapping blocks. Pull never silently changes
`FixedPaginatedTail` to `WholeThread`.

## Divergent Pull

For cloud thread `T` bound locally to `P-local`, divergence produces:

```text
Before:
  T -> P-local

After:
  C -> P-local    existing local rollout unchanged
  T -> P-remote   cloud history materialized once and still tracking T
```

Planning fixes logical `C`, the new provider IDs for P-remote, its target paths
and full hashes, both portable layouts, and exact before/after local documents.
It proves P-local is still T's exact current alias and is safe and unique to
rebind. Apply and recovery reuse those values and never remint.

Only P-remote is downloaded, reconstructed, reidentified, hashed, staged, and
installed during apply.
P-local and its `history_base` dependencies stay exactly where they are. The
binding move does not edit `session_meta`, provider IDs, native history bases,
ordinals, compaction records, subagent references, provider indexes, titles, or
any rollout byte. P-local's existing subagent closure simply remains attached
to the unchanged local root; it did not participate in selecting `Divergent`.

The new `C` binding copies the local root-tail layout. The selected
destination's T checkpoint is re-keyed to C as local-only observed state: its
observed cumulative tail, physical source mapping, and raw range pin are
preserved, while its accepted cloud pin and physical accepted end are cleared.
This does not create a family relationship or a reference to `T`. `C` has no
accepted cloud version until its first Push and never resets to empty or
boundary zero. That first Push is when Mallard constructs C as a standalone
artifact, if the user chooses to publish it.

If P-local cannot be proven safe and unique to rebind, Pull fails closed. It
does not construct a speculative preserved copy.

The winning `T` imports the remote cloud root partition exactly. Other storage
links that referenced the replaced provider alias retain their independent
accepted cloud/tail pins, clear unusable physical mappings, and run the same
`P/L/C` classifier on their next Pull. Equal layouts do not make different
tail values equal. Physical invalidation has one `Detached` state; it does not
pre-classify the eventual root relation.

Pull never writes cloud state. A later Push of `T` advances the same shared
cloud thread. A later first Push of `C` creates an independent cloud thread.

## ID allocation

All IDs are minted during planning and persisted in the plan. Mint logical `C`
with Mallard's bounded `ThreadId` allocator. For new Codex provider IDs, prefer
a bounded headless Codex CLI app-server session in a disposable `CODEX_HOME`
and CWD, using `thread/start` with `ephemeral: true` and no `turn/start`.
Accept only a canonical lowercase UUIDv7.

If the CLI is missing or fails, generate each provider ID as a cryptographically
random RFC 9562 UUIDv7 locally and apply the same validation and collision
checks. There is no desktop-app automation fallback. Apply and recovery never
invoke Codex or mint new IDs.

## Append-only trust boundary

The `FixedPaginatedTail` fast path assumes supported Codex rollouts do not
mutate bytes before a saved physical cursor. Refresh checks canonical
containment, file identity, kind, length, complete JSONL boundaries, and
appended bytes. `WholeThread` does not take this partial-read path.
Replacement, truncation, deletion, or a detectable identity change invalidates
the mapping.

Mallard assumes bytes already incorporated into a trusted physical cursor are
append-only. This includes the inherited prefix and the already-hashed portion
of the tail. Codex edit/backtrack creates another rollout instead of mutating
those bytes. Replacement, shrinkage, identity changes, and ordinary appends
are still detected, but a same-length in-place manual mutation may evade the
incremental classifier and is outside this resolver's correctness contract.
Complete output SHA-256 proves the bytes Mallard uploads or downloads; it is
not an accepted-prefix comparison oracle.

## Invariants

- Every uploaded cloud Codex task is a complete standalone root plus its
  captured subagent snapshot.
- Every standalone rollout has paginated mode and explicit null native base.
- The portable tail partition survives standalone reconstruction and
  cross-machine Pull.
- Existing accepted branch starts never move. First unmanaged discovery seeds
  the layout once; divergent keep-both is the only later replacement, moving
  the local layout unchanged to C before T imports the verified cloud layout.
  Accepted cumulative states never reset on Push or Pull.
- Root/no-base tasks preserve whole-thread hashing through start zero.
- Full object SHA-256 and head CAS remain authoritative.
- Pull relation planning compares only validated cumulative portable-tail
  values against the accepted pin; both-changed unequal tails are
  conservatively divergent and receive no containment refinement.
- Subagent file sets, topology, bytes, and lifecycle/reference/control-only
  records never affect Refresh, Push, or Pull classification. Subagent
  differences create no state or action; subagents are carried only by an
  already root-driven byte-consuming operation. A changed canonical root
  conversation message still changes the root tail.
- A stale Push cannot overwrite a remote winner.
- Conflict resolution preserves both histories without family or machine
  metadata: C reuses the unchanged local alias and only cloud T is newly
  materialized.
- Pull never moves or deletes old Codex rollout sources. Receipts report the
  literal outcome `"retained"`; cleanup is a separate future operation.
- Push plans pin exact source inputs, tail state, local preconditions, and cloud
  HEAD but no output bytes/hashes; Push journals pin the apply-generated
  candidate and recovery after-state. Materializing Pull plans likewise pin
  only immutable cloud metadata, fixed IDs/paths, and local preconditions;
  their journal receives the apply-generated staged target and mutations.
- The cutover rejects old cloud/local shapes; it provides no migration,
  compatibility reader, dual writer, or legacy executor.
