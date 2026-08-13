# Design: Reconstruct Codex threads as standalone transcripts before upload

- Status: Proposed; reconstruction behavior validated against Codex 0.147.0,
  but not implemented in Mallard
- Date: August 9, 2026
- Scope: Codex conversation capture, Push review/apply, portable transcript
  validation, and Pull materialization
- Compatibility: clean cutover; no migration or legacy execution path
- Related:
  - [PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md](PLAN_MINIMAL_CONVERSATION_CONFLICT_RESOLVER_BACKEND.md)
  - [DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md](DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md)
  - [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md)
  - [PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md)
    (historical; superseded by the minimal backend plan for canonical
    root-only conflict semantics and incidental subagent snapshot transport)
  - [PLAN_CODEX_SYNTHESIZED_FORK.md](PLAN_CODEX_SYNTHESIZED_FORK.md)

## Decision

Mallard will never upload a Codex conversation whose readable history still
depends on another rollout file through `session_meta.history_base`.

Before Push uploads a selected Codex transcript, Mallard will resolve its
bounded `history_base` dependency graph, reconstruct the complete logical
record sequence, and serialize one standalone transcript. For a paginated
thread, the authoritative first metadata record in the uploaded artifact must
contain:

```json
{
  "history_mode": "paginated",
  "history_base": null
}
```

Every uploaded paginated transcript must therefore be readable with only its
own object. Updating, replacing, pruning, or failing to upload an ancestor
thread cannot invalidate it.

Only `history_base` is a content dependency. These fields are provenance or
control-graph edges and must be preserved without recursively importing the
referenced thread:

- `forked_from_id`;
- `parent_thread_id`;
- `source.subagent.thread_spawn.parent_thread_id`;
- `thread_source`; and
- agent path, nickname, role, and depth metadata.

Legacy Codex transcripts are already inline and remain on their existing
path. This design does not convert legacy history into paginated history. The
new reconstruction contract applies whenever the source is paginated. Here
“legacy” names a supported provider-native Codex history mode, not an older
Mallard schema or compatibility path: every captured conversation still uses
the single current schema-1 portable-tail contract.

## Problem

Codex 0.147 can persist a paginated fork as a small child file plus a frozen
prefix of another rollout:

```text
provider thread T
  m1 -> m2

provider thread F
  history_base = T through ordinal 2 / byte offset N
  m3 -> m4
```

The complete history of `F` is `m1 -> m2 -> m3 -> m4`, but those records are
not all physically present in `F`.

Uploading the child file by itself is incomplete. Uploading both files still
does not make the child robust because the reference is a provider thread ID
plus physical byte and ordinal boundaries, not a portable Mallard object
reference. If a later sync replaces logical thread `T`, prunes its rollout, or
materializes it under another provider ID, `F` can no longer resolve the
prefix it was created against.

For example:

```text
before
  cloud T: m1 -> m2
  cloud F: base(T) -> m3 -> m4

later
  cloud T: m1 -> m5
  cloud F: base(T) -> m3 -> m4   # no longer proves the original history
```

The portable representation must instead be:

```text
cloud T object: standalone m1 -> m5
cloud F object: standalone m1 -> m2 -> m3 -> m4
```

The objects may share content-addressed storage chunks in a future
optimization, but their correctness cannot depend on that sharing.

## Three graphs that must remain separate

### History dependency graph

`history_base` points to the exact prefix needed to read a paginated rollout.
This graph is followed during reconstruction and eliminated from the uploaded
artifact.

```text
child rollout --history_base--> base rollout prefix
```

### Provider provenance and subagent graph

`forked_from_id`, `parent_thread_id`, and structured subagent source metadata
describe how provider threads were created. They may legally point to a
thread whose rollout is absent. They are retained in the standalone artifact
and are not treated as byte dependencies.

```text
child --forked_from_id--> provider parent
agent --parent_thread_id--> spawning agent
```

### Mallard logical thread graph

`ConversationThread.forked_from`, where retained for unrelated provenance, is
never used to locate Codex history bytes or make a conflict decision. The new
resolver writes no family relation. Reconstructing a provider transcript does
not mint or change a Mallard `ThreadId`.

## Standalone artifact contract

A reconstructed paginated artifact is valid only when all of the following
hold:

1. The first non-empty line is the target thread's authoritative
   `session_meta`.
2. Its provider thread ID is unchanged.
3. `history_mode` is exactly `"paginated"`.
4. `history_base` is present and exactly `null`.
5. Every top-level `ordinal` is present and forms the contiguous sequence
   from `0` through `record_count - 1`.
6. The file ends with a newline.
7. No record was invented, dropped, or reordered except for the structural
   expansion described below.
8. Every inline ancestor `session_meta` emitted by Codex is retained.
9. The canonical message chain, durable message IDs, lineage digest, and
   semantic digest match the resolved source history.
10. `subagent_history_start_ordinal`, when present, points to the first record
    belonging to the subagent's own projected history in the new ordinal
    space.
11. The reconstructed bytes contain no non-null `history_base` on the
    authoritative metadata record.
12. The artifact passes the same bounded structural and transcript analysis
    used for an ordinary captured rollout.

The enclosing schema-1 Codex `ConversationThread` must additionally carry the
standalone artifact layer's verified `artifact_sha256` and one valid portable
tail value for the root conversation only:

```text
portable_tail = {
  codec,
  partition_kind,
  start_record_count,
  accepted_end_record_count,
  accepted_tail_state
}
```

That is the complete five-field wire value. Its start count maps into the
reconstructed root conversation projection, its accepted end equals the
complete end of that projection, and its cumulative state matches exactly the
root messages from start through end. Tail count is derived as `end - start`;
there is no prefix witness or second comparison fingerprint. This metadata is
outside `session_meta`; it is not a new provider-native dependency. Included
subagent files have ordinary object hashes but no portable tail entry and no
conflict state.

The canonical root conflict stream is not every physical root JSONL record. It
excludes subagent lifecycle, spawn/reference/control, child-result, child ID,
and child topology bookkeeping unless a record changes a canonical root
conversation message. This exclusion never masks root message content: if the
root conversation message projection changes, the root tail changes even when
the provider emitted it beside subagent control records.

An example first record is:

```json
{
  "timestamp": "2026-08-09T00:00:00.000Z",
  "type": "session_meta",
  "payload": {
    "id": "provider-child-id",
    "forked_from_id": "provider-parent-id",
    "cwd": "/source/project",
    "history_mode": "paginated",
    "history_base": null
  },
  "ordinal": 0
}
```

The source CWD remains in the portable payload today, as it does for existing
conversation capture. Pull continues to rewrite declared structural CWD
fields to the reviewed destination project. No machine-private path is added
to the manifest, plan, or receipt by this design.

## Reconstruction algorithm

### 1. Build a bounded rollout index

Discovery performs a metadata-only pass over the bound `CODEX_HOME` and
indexes each accepted rollout by the authoritative first `session_meta.id`.
The index stores machine-private capture information only:

- canonical no-follow path;
- active or archived session tree;
- file size and modification time;
- authoritative provider thread ID;
- history mode; and
- enough metadata to reject duplicate provider IDs with ambiguous files.

The index is not portable. A duplicate provider ID, symlink, special file,
path escape, oversized record, or ambiguous active/archive copy fails that
thread closed.

### 2. Expand one exact snapshot

For a target file and a byte cutoff, define:

```text
expand(file, cutoff):
  physical = parse complete JSONL records in file[0..cutoff)
  output = [physical.authoritative_session_meta]

  if physical.authoritative_session_meta.history_base is B:
    base = resolve exactly one rollout whose authoritative id == B.thread_id
    inherited = expand(base, B.end_byte_offset)
    require inherited.record_count == B.end_ordinal_exclusive
    output += inherited

  output += physical.records_after_authoritative_metadata
  return output
```

The target call uses its captured end-of-file as `cutoff`. A recursive base
call uses the exact `end_byte_offset` from the child metadata.

Before accepting a base edge, Mallard must prove:

- `thread_id` resolves to one indexed rollout;
- `end_byte_offset` is nonzero, within the captured file, and immediately
  follows a newline;
- the parsed prefix ends on a complete JSON record;
- recursive expansion produces exactly `end_ordinal_exclusive` records;
- the edge does not introduce a cycle;
- depth, record count, distinct thread count, per-file bytes, and total
  closure bytes remain within existing bounded limits; and
- the authoritative metadata and every structural CWD pass the current
  rollout parser.

Appends after a referenced `end_byte_offset` are irrelevant to that child's
history. Reconstruction hashes the referenced prefix rather than requiring
the dependency file's entire SHA-256 to remain unchanged.

### 3. Normalize the target artifact

After expansion:

1. Clone the expanded values into a new in-memory artifact.
2. Change only the authoritative first metadata record's `history_mode` to
   `"paginated"` and `history_base` to explicit JSON `null`.
3. Reassign every top-level `ordinal` to its zero-based output position.
4. Recompute `subagent_history_start_ordinal` if expansion changed the number
   or position of records before the original boundary.
5. Serialize every value with one deterministic JSON serializer, one record
   per line, and one final newline.
6. Parse and validate the resulting bytes again.

Reconstruction must retain provenance for each emitted record:

```text
(source provider thread id, source physical ordinal, source byte range)
```

That mapping is how the implementation moves a subagent boundary safely. It
must not copy the old numeric `subagent_history_start_ordinal` after inserting
or deleting records unless the mapping proves the value is unchanged.

### 4. Prove content preservation

The verifier compares the expanded source values with the standalone output
after removing only the allowed envelope changes:

- top-level `ordinal` from every record;
- authoritative `history_base`;
- authoritative `history_mode` normalization;
- authoritative `subagent_history_start_ordinal` when its recomputed value is
  proven from record provenance; and
- the `thread_id` inside a current `event_msg`/`item_completed` envelope,
  which is normalized to — or inserted as — the output root identity.

Every original non-metadata record must otherwise compare equal, including
timestamps inside payloads, durable response-item IDs, event payloads, tool
results, copied ancestor metadata, and unknown forward-compatible records.

Exactly two classes of inserted records are permitted, both produced by the
frozen Codex paginated projection adapter
(`codex_paginated_projection.rs`): `event_msg`/`item_completed` projections
for recognized display completions, and `task_started`/`task_complete`
lifecycle records for synthesized implicit `rollout-N` turns. Both classes
are `event_msg` records, which the conversation codec ignores for chain
identity. The projection pass is a fixpoint on its own output; the builder
re-runs it over the serialized bytes and requires zero changes.

The current `non_metadata_record_digest` used by native-fork verification is
not directly sufficient because it includes top-level ordinals. The new
module needs a reconstruction-specific canonical payload digest that removes
the envelope ordinal before hashing.

The existing lineage and semantic digests remain mandatory but are not the
only proof: they intentionally ignore many non-message records.

## Determinism and repeated Push

Given the same target bytes and the same referenced prefixes,
reconstruction must produce byte-identical output. Determinism is required
for content-addressed object reuse and no-op Push behavior.

The Push plan records a machine-private closure witness:

```text
exact source file identities and reviewed safe ends
ordered fixed dependency edges and cutoffs
raw SHA-256 pins for every planning-reviewed physical range
portable root-tail kind/start and planned cumulative state
exact local document preconditions and reviewed cloud manifest/HEAD
```

Planning does not reconstruct an artifact or compute output, artifact, lineage,
or semantic hashes. Apply consumes the pinned source closure once, verifies the
source witness while reading, reconstructs once, and computes those output
values for the first time. A safe append beyond a frozen dependency prefix does
not invalidate the child history. A truncated/replaced base, moved boundary,
target drift, changed planned range, or changed cumulative tail state fails
that item before upload.

Accepted cloud/tail state and physical tail mappings belong to the selected
destination's local checkpoint, not to the provider alias binding. The
portable file entry and reviewed operation pins retain complete artifact
hashes; no field is reused for two meanings.

## Push flow

The shared review flow remains manifest-first and reads no cloud conversation
payloads. Reconstruction adds only bounded local reads.

For each selected Codex thread:

1. During planning, observe the local cumulative tail and pin source
   identities, cutoffs/EOFs, raw ranges, and the exact cloud HEAD needed to
   reproduce it.
2. Re-prove the reviewed local/cloud state by calling the one conservative
   accepted-pin/local-tail/cloud-manifest-tail classifier shared by Refresh,
   Push, and Pull. Do not run a command-specific comparison or full graph
   containment refinement for two changed unequal tails.
3. Continue only for `LocalOutstanding` or `MissingCloud`; every other result
   exits before standalone reconstruction or upload.
4. Finish planning without constructing standalone bytes or output hashes.
5. During apply, verify the source pins while reading the complete root, exact
   dependency prefixes, and bounded subagent closure once, then reconstruct
   and verify each selected rollout as a deterministic standalone transcript.
6. Replace the raw provider transcript bytes for the root and included
   subagents with their standalone artifact bytes.
7. Hash and upload that complete object set through the existing immutable,
   checksum-verified object path.
8. Publish the complete conversation metadata, including its portable fixed
   root-tail partition and accepted cumulative tail state, and swing `head` with
   the existing compare-and-swap.
9. Commit receipts and binding pins only after publication is proven.

There is no fallback that uploads the unresolved physical child file. A
blocked selected conversation produces a non-publishing review outcome for
the batch; the user may deselect it and request a fresh plan. Mallard never
silently filters a blocker and publishes a partial unreviewed subset.

The portable manifest does not contain local dependency paths, native base
IDs, byte offsets, source ordinals, or prefix pins. It does contain the
provider-independent canonical partition required to continue tail hashing on
another machine. The numeric portable bundle/head schema remains `1`, but its
Codex contract is replaced in place and adds a required `portable_tail` value
to every Codex `ConversationThread`. Old schema-1 shapes are rejected by exact
shape/codec validation rather than migrated or interpreted by a legacy path.
Old binaries/writers are unsupported after cutover because the unchanged
number cannot provide mixed-version writer safety.

A native root with no `history_base` uses a `WholeThread` partition at
canonical boundary zero and retains complete whole-thread comparison. An exact
native `history_base(base, cutoff)` seeds a `FixedPaginatedTail` partition at
the corresponding canonical boundary and permits partial-range comparison. The
partition kind and start remain fixed for that logical root across Pushes. A
successful Push records accepted cumulative state = observed cumulative state;
it does not reset the tail to empty or move its boundary.

Resolving a chain of `history_base` dependencies does not create portable
entries for its ancestors. Those ancestors are bounded local source segments
used to flatten the selected leaf. There is exactly one portable tail value,
for the owning root only. For
`P0: m1,m2`, `P1: history(P0@m2)+m3`, and
`P2: history(P1@m3)+m4,m5`, P2 contributes one root tail with start after
`m3` and cumulative tail `Htail(m4,m5)`; the full standalone artifact still
contains and byte-hashes `m1..m5`.

Subagent rollout contents and topology do not participate in that root-tail
hash. Equal root tails compare `Sync` even when the captured subagent files
differ. Subagent lifecycle, spawn/reference/control, and child-result records
in the root file are also excluded when they do not change a canonical root
conversation message. A changed root conversation message always changes the
projection and tail. A Sync operation does not upload or download subagent-only
differences; the then-current subagent snapshot is carried only when a
root-driven operation already needs the full artifact bytes.

Each publishing Push builds and hashes the complete standalone root/subagent
snapshot once during apply before upload. Pull materializes `history_base: null`,
imports the cloud portable partition, and maps its canonical start and accepted
end into the receiving machine's new physical rollout. The receiver therefore
continues the same cumulative partial hash instead of falling back to
whole-thread hashing.

## Pull flow

When the shared conflict classifier selects a materializing or keep-both
action, Pull is simpler because every uploaded Codex transcript is
self-contained. Manifest-only `Sync`/`LocalOutstanding` actions do not run
this byte-consuming flow. Every current materializing action has exactly one
new target: for Divergent, C reuses the existing P-local rollout unchanged and
only cloud T is rebuilt as P-remote.

Planning:

1. Verify and pin the exact cloud HEAD and manifest.
2. Validate and pin each selected root/subagent object's key, declared size,
   SHA-256, file role, artifact SHA, and portable root-tail metadata.
3. Allocate the fixed provider IDs and target paths required by the manifest's
   declared file set.
4. Pin local binding/checkpoint/source and document preconditions, including the
   existing P-local rebind proof for Divergent.
5. Do not download transcript objects, reconstruct/reidentify output, compute
   output hashes, or create staging files.

Apply:

1. Revalidate the shared P/L/C relation, plan expiry, exact HEAD/manifest,
   target absence, local pins, and document revisions.
2. Download the selected root transcript and included subagent objects once,
   verifying each declared size and SHA-256 while reading.
3. Require the authoritative paginated metadata to have
   `history_base: null`; reject a non-null dependency instead of searching
   for another cloud or local thread.
4. Run the existing bounded codec analysis and require the manifest's
   lineage and semantic claims to match.
5. Validate the portable root-tail start/end and cumulative state against the
   complete verified root projection. Do not derive any subagent comparison
   state.
6. Reidentify the selected root/subagent closure with plan-persisted provider
   IDs, rewriting only declared structural identity and CWD fields for the
   reviewed destination.
7. Preserve inline ancestor metadata, provenance fields, events, and the
   subagent history boundary.
8. Rebase ordinals only if the existing materialization rewrite changes the
   record sequence; ordinary ID/CWD rewrites do not.
9. Map the cloud canonical root-tail positions into the rewritten root file
   and derive accepted = observed cumulative tail state.
10. Validate the typed output invariants, then serialize once while emitted
    bytes feed SHA-256 and private staging; fsync and pin identities/sizes/hashes.
11. Build and journal the exact target and local mutations, no-clobber install,
   commit bindings and SyncConfig, refresh Codex inventory best-effort, and
   emit the receipt.

Pull planning is metadata-only. Uninterrupted apply downloads the pinned input
objects once, validates the typed reconstruction, and serializes/hashes the one
remote output once. Input-object hashing and emitted-output hashing are distinct
integrity checks over different bytes; the output itself is neither reparsed nor
hashed twice. Recovery may rehash the journaled stage after a process boundary,
but never downloads or reconstructs it again.
In Divergent Pull, moving P-local from logical T to logical C is only a Mallard
binding/checkpoint mutation. It does not rewrite the local root, its
`session_meta`, provider ID, native history base, ordinals, compaction records,
subagent references, index/title metadata, or any dependency source. If that
existing alias cannot be proven safe and unique to rebind, Pull blocks instead
of making a preserved copy.

Pull never follows `forked_from_id` or `parent_thread_id` to locate history.
A dangling provider parent remains valid provenance.

## Subagent behavior

Codex 0.147.0 uses a separate mechanism for AgentControl subagents:

- a full-history subagent copies a sanitized model-context prefix inline;
- its authoritative metadata is paginated with no `history_base` dependency;
- `subagent_history_start_ordinal` separates inherited model context from the
  subagent's own projected turns;
- a fresh subagent has no copied prefix and normally no boundary; and
- a nested full-history subagent can carry inline metadata for its immediate
  parent while already remaining independently readable.

Disposable 0.147.0 tests covered a root, a full-history child, a fresh
sibling, and a depth-two full-history grandchild. After normalizing the
authoritative metadata to explicit `history_base: null`, each child was placed
alone in a fresh Codex home. Cold read, resume, restart, source-kind listing,
direct-parent and ancestor filtering, projected-turn reconstruction, and a
subsequent fork all succeeded. Mallard's current structural parser classified
all tested files as internal subagents with complete, resumable lineage.

Consequences:

- Do not recursively expand `parent_thread_id`.
- Do not recursively expand `forked_from_id`.
- Do not delete inline ancestor `session_meta` records.
- Do not reset `subagent_history_start_ordinal` merely because the parent
  rollout is absent.
- A native subagent with no `history_base` is already standalone; normalizing
  explicit null must not duplicate its copied context.

Internal subagent rollouts are not independent cloud conversations. Under the
current minimal backend plan, they belong to the owning root's bounded
artifact snapshot, but they are excluded from the root `P`/`L`/`C` tail value
and every conflict decision. Their differences create no separate
state or Push/Pull action. An already root-driven byte-consuming operation
still reidentifies, hashes, uploads, or downloads the complete selected
subagent closure. The older
[PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md) remains
historical design context only; its provider-ID identity and parent-only
digest/conflict rules are superseded. The same standalone byte validation
applies to every included subagent artifact.

## Cloud overwrite behavior after this change

Assume local provider thread `F` originally depends on prefix `T@2`.

At Push time:

```text
standalone(F) = expand(T@2) + local(F)
```

The immutable object for `standalone(F)` contains the exact original prefix.
A later manifest may point logical thread `T` at different content, but the
object for `F` neither references nor reads that entry. Therefore cloud `T`
can fast-forward, fork, be replaced by an explicitly reviewed action, or be
pruned without breaking `F`.

This removes the dangerous coupling described by:

```text
cloud logical identity T changes
        |
        v
provider history_base inside F becomes invalid
```

The new relationship is only portable provenance:

```text
cloud T object       cloud F standalone object
      ^                         |
      +---- optional display provenance ----+
```

## Module boundaries

Add a Tauri-free infrastructure module, for example:

```text
crates/mallard-infrastructure/src/codex_standalone_reconstruction.rs
```

It owns:

- bounded metadata indexing;
- `history_base` graph resolution;
- byte/ordinal endpoint validation;
- record provenance mapping;
- ordinal and subagent-boundary rebasing;
- deterministic serialization; and
- before/after preservation verification.

Suggested root-snapshot result:

```rust
struct StandaloneConversationArtifact {
    root_provider_thread_id: String,
    root_bytes: Vec<u8>,
    subagents: Vec<StandaloneSubagentArtifact>,
    dependency_prefixes: Vec<CodexHistoryPrefixPin>,
    id_map: Vec<StandaloneThreadAllocation>,
    portable_tail: PortableConversationTail,
    artifact_sha256: String,
    lineage_digest: String,
    semantic_digest: String,
    message_count: u64,
}
```

`dependency_prefixes` contains machine-private paths or path-derived state
only in capture and reviewed-plan memory. It is never serialized into the
portable manifest. `portable_tail` describes `root_bytes` only; no element of
`subagents` owns or contributes a portable-tail entry. The aggregate artifact
hash and per-file hashes still cover the complete root/subagent snapshot.

`provider_capture.rs` should support two distinct phases:

1. planning discovers the source closure and records exact source pins without
   constructing output; then
2. apply reconstructs each selected conversation once from those exact pins.

Do not place filesystem traversal or provider-format parsing in
`mallard-domain`. The domain continues to validate portable conversation
metadata and file roles.

## Failure handling

The selected conversation is not uploadable when any of these occurs:

- missing or ambiguous base rollout;
- invalid or out-of-bounds byte endpoint;
- byte endpoint not ending on a complete newline-delimited record;
- expanded count not matching `end_ordinal_exclusive`;
- non-contiguous or unmappable ordinals;
- dependency cycle, self-edge, or depth overflow;
- total file, closure, or record bounds exceeded;
- malformed authoritative or inline metadata;
- an inline metadata identity unrelated to the accepted fork ancestry;
- `subagent_history_start_ordinal` cannot be mapped safely;
- deterministic round-trip changes an unapproved payload field;
- reconstructed lineage or semantic digest differs from the resolved source;
- the root portable tail kind/start/end cannot be derived or uniquely mapped, or its
  cumulative state does not match the reconstructed canonical range; or
- apply-time prefix or target pins differ from the reviewed snapshot.

The failure is a held Push row with a precise explanation. Mallard must not
silently downgrade to uploading raw dependent bytes or mark the conversation
`StoredOnly`, because the raw child is not a portable complete copy.

## Security and privacy invariants

- Never read outside the canonical bound Codex session and archived-session
  trees.
- Reject symlinks and special files at every dependency source file/segment.
- Bound every file, prefix, line, record set, graph depth, and total closure.
- Resolve provider IDs through the captured index, never by constructing an
  unchecked filesystem path from metadata.
- Keep local paths, dependency source paths, file handles, prefix pins, and
  capture diagnostics out of portable documents.
- Treat reconstruction as transcript handling: never log message content or
  raw payload bytes.
- Publication remains immutable, SHA-256 verified, and committed only through
  head CAS.
- Pull remains plan-based, digest-pinned, backed up where replacement is
  allowed, and atomically applied.

## Tests

### Unit tests

- Paginated root with omitted `history_base` becomes explicit null.
- Paginated root with explicit null remains byte-deterministic after
  normalization.
- One-level dependency reconstructs a contiguous standalone transcript.
- Three-level chain reconstructs in the correct logical order.
- A three-level root history chain produces one root portable tail; none of
  its dependency source segments becomes another entry.
- Adding, removing, or changing a subagent file does not change the root
  portable tail value.
- Equal root tails with different subagent snapshots classify `Sync` and do
  not create another state or action.
- Subagent lifecycle/reference/control-only and child-result changes in the
  physical root file leave the root tail unchanged; changing a
  canonical root conversation message changes it.
- Sibling forks sharing one base produce independent artifacts.
- A base append beyond `end_byte_offset` does not change the child artifact.
- A changed byte inside the referenced prefix invalidates the plan pin.
- Missing base, ambiguous ID, cycle, self-edge, bad byte boundary, wrong
  ordinal endpoint, ordinal gap, and depth overflow fail closed.
- Inline repeated ancestor metadata is retained and accepted.
- Unrelated or conflicting metadata is rejected by the current structural
  parser.
- Unknown non-structural records survive with equal payloads.
- Output ordinals are exactly `0..N-1`.
- Output ends in one newline and reconstruction is byte-deterministic.
- Lineage and semantic digests match before and after reconstruction.
- Full-history, fresh, and nested subagent boundaries are preserved or
  correctly rebased.

### Codex compatibility tests

Using an isolated temporary Codex home, never the user's real home:

- install only the reconstructed target file;
- verify `thread/read`;
- verify `thread/resume`;
- restart app-server and read again;
- verify source-kind, direct-parent, and ancestor listing for a subagent;
- verify inherited subagent prefix records do not project as child turns;
- fork the reconstructed thread and restart-read the fork; and
- require Codex not to rewrite the source artifact during read/resume.

Pin the compatibility fixture to the supported Codex version and repeat when
the supported version changes.

### End-to-end sync tests

- Push `T`, push dependent `F`, then change/replace `T`; a fresh-machine Pull
  of only `F` remains resumable.
- Pull only a depth-three fork into an empty provider home.
- Push the same unchanged dependency graph twice; the second Push uploads no
  new transcript object.
- Equal roots with different subagent snapshots remain manifest-only `Sync`
  and transfer no subagent object.
- A root append that authorizes publication carries the selected local
  subagent snapshot, and a later root-driven materializing Pull installs that
  cloud snapshot with full per-object verification.
- Two children sharing one base dedupe immutable objects while remaining
  independently restorable.
- Review/apply drift inside a referenced prefix fails only that item.
- Storage parity covers local-folder and S3/R2 backends.
- A CAS-losing Push re-plans and reuses the already uploaded standalone
  object.
- Pull rejects a deliberately stored paginated transcript whose authoritative
  `history_base` is non-null.
- Pull, re-capture, and Push converge to a no-op even though the destination
  provider thread ID and CWD differ from the original source.
- A paginated child pushed by A, pulled as a null-base standalone rollout by
  B, extended by B, and pushed again retains the same nonzero canonical tail
  start and advances one cumulative tail state.

## Cutover

1. Add the reconstruction module and fixture tests without changing Push.
2. Add the two-pass Codex rollout index and dependency-prefix pins.
3. Change Codex captured transcript bytes to the verified standalone artifact.
4. Make Push reject any selected paginated Codex transcript that is not
   standalone after reconstruction.
5. Keep numeric schema 1, require the new Codex portable-tail shape, and reject
   every old schema-1 shape without migration or fallback.
6. Make Pull apply reject any new paginated cloud transcript with non-null
   `history_base` or an unmappable portable tail checkpoint.
7. Run the isolated Codex compatibility matrix and the complete backend
   integration suite.
8. Clear test `~/.mallard` state and test storage before acceptance, consistent
   with the repository's clean-cutover policy.

There is no feature flag, raw-dependent fallback, or dual writer. Once the
cutover lands, reconstruct-then-upload is the only Codex Push path.

## Acceptance criteria

1. Every newly uploaded paginated Codex transcript has authoritative
   `history_mode: "paginated"` and explicit `history_base: null`.
2. A selected thread can be restored, read, resumed, and forked with no
   ancestor rollout present.
3. Changing a cloud parent cannot change the readable history of an already
   uploaded child.
4. Reconstruction preserves all durable content, inline metadata, provider
   provenance, and subagent projection boundaries.
5. Repeated reconstruction is byte-deterministic and produces content-address
   reuse.
6. Missing or malformed dependencies fail one item closed without publishing
   an incomplete transcript.
7. No dependency path, local provider-home path, or capture pin enters a
   portable manifest, receipt, or log.
8. Push publication remains immutable and CAS-committed; Pull remains
   reviewed, verified, backed up where needed, and atomically applied.
9. A standalone null-base Pull preserves the sender's portable
   `WholeThread`/`FixedPaginatedTail` partition and cumulative state.
10. The portable tail is one root descriptor/state. Equal root projections are
    `Sync` regardless of subagent snapshot or control bookkeeping, and
    subagents travel only with an already root-driven byte-consuming Push/Pull.
