# Design: Reconstruct Codex threads as standalone transcripts before upload

- Status: Proposed; reconstruction behavior validated against Codex 0.147.0,
  but not implemented in Mallard
- Date: August 9, 2026
- Scope: Codex conversation capture, Push review/apply, portable transcript
  validation, and Pull materialization
- Compatibility: clean cutover; no migration or legacy execution path
- Related:
  - [DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md](DESIGN_CODEX_FORK_LINEAGE_AND_DIVERGENCE_RESOLUTION.md)
  - [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md)
  - [PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md)
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
new reconstruction contract applies whenever the source is paginated.

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

`ConversationThread.forked_from` records portable conflict provenance. It is
never used to locate Codex history bytes. Reconstructing a provider transcript
does not mint or change a Mallard `ThreadId`.

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
- authoritative `history_mode` normalization; and
- authoritative `subagent_history_start_ordinal` when its recomputed value is
  proven from record provenance.

Everything else, including timestamps inside payloads, durable response-item
IDs, event payloads, tool results, copied ancestor metadata, and unknown
forward-compatible records, must compare equal.

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
target full-file SHA-256
ordered dependency edges:
  provider thread id
  end byte offset
  end ordinal exclusive
  prefix SHA-256
standalone artifact SHA-256
lineage digest
semantic digest
```

Apply-time capture rebuilds the artifact and requires the reviewed witness to
match. A safe append beyond a frozen dependency prefix does not invalidate
the witness. A changed referenced prefix, truncated base, moved boundary,
target drift, or changed output hash fails that item before upload.

`LocalConversationBinding.last_observed_file_sha256` continues to describe
the machine-local provider transcript. It must not be silently repurposed as
the standalone artifact hash. The artifact hash already belongs to the
portable file entry and operation pins; if a durable local artifact pin is
needed, add a separately named field.

## Push flow

The shared review flow remains manifest-first and reads no cloud conversation
payloads. Reconstruction adds only bounded local reads.

For each selected Codex thread:

1. Re-capture the target and its exact dependency prefixes.
2. Reconstruct and verify a deterministic standalone transcript.
3. Re-prove the reviewed local and cloud relation from the canonical message
   chain.
4. Replace the raw provider transcript bytes in the captured portable
   `ConversationFileRole::Transcript` with the standalone artifact bytes.
5. Hash and upload that artifact through the existing immutable,
   checksum-verified object path.
6. Publish the existing conversation metadata and swing `head` with the
   existing compare-and-swap.
7. Commit receipts and binding pins only after publication is proven.

There is no fallback that uploads the unresolved physical child file. A
reconstruction failure blocks only that conversation; other selected
resources continue through the existing per-item isolation rules.

The portable manifest does not need local dependency paths or prefix pins.
It continues to contain one transcript file for the logical conversation.
The payload contract changes, but the schema-1 file role and content-addressed
object model are sufficient. Clean-cutover acceptance should start with empty
test storage so no pre-contract raw-dependent object can be mistaken for a
standalone transcript.

## Pull flow

Pull becomes simpler because every uploaded Codex transcript is
self-contained:

1. Download and SHA-256 verify the selected transcript object.
2. Require the authoritative paginated metadata to have
   `history_base: null`; reject a non-null dependency instead of searching
   for another cloud or local thread.
3. Run the existing bounded codec analysis and require the manifest's
   lineage and semantic claims to match.
4. Rewrite only declared structural identity and CWD fields for the reviewed
   destination provider thread.
5. Preserve inline ancestor metadata, provenance fields, events, and the
   subagent history boundary.
6. Rebase ordinals only if the existing materialization rewrite changes the
   record sequence; ordinary ID/CWD rewrites do not.
7. Atomically stage, verify, commit, refresh Codex inventory best-effort, and
   write the normal receipt and binding.

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

The current product excludes internal subagent rollouts as independent cloud
conversations. That product rule remains unchanged. If
[PLAN_CODEX_SUBAGENT_TREE_SYNC.md](PLAN_CODEX_SUBAGENT_TREE_SYNC.md) later
stores them as attached files, the same standalone validation applies to each
attachment.

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

Suggested internal result:

```rust
struct StandaloneCodexArtifact {
    bytes: Vec<u8>,
    source_thread_id: String,
    source_file_sha256: String,
    dependency_prefixes: Vec<CodexHistoryPrefixPin>,
    artifact_sha256: String,
    lineage_digest: String,
    semantic_digest: String,
    record_count: usize,
}
```

`dependency_prefixes` contains machine-private paths or path-derived state
only in capture and reviewed-plan memory. It is never serialized into the
portable manifest.

`provider_capture.rs` should become a two-pass Codex capture:

1. discover and index rollout metadata; then
2. reconstruct selected conversation artifacts from that immutable capture
   snapshot.

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
  or
- apply-time prefix or target pins differ from the reviewed snapshot.

The failure is a held Push row with a precise explanation. Mallard must not
silently downgrade to uploading raw dependent bytes or mark the conversation
`StoredOnly`, because the raw child is not a portable complete copy.

## Security and privacy invariants

- Never read outside the canonical bound Codex session and archived-session
  trees.
- Reject symlinks and special files at every dependency node.
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

## Cutover

1. Add the reconstruction module and fixture tests without changing Push.
2. Add the two-pass Codex rollout index and dependency-prefix pins.
3. Change Codex captured transcript bytes to the verified standalone artifact.
4. Make Push reject any selected paginated Codex transcript that is not
   standalone after reconstruction.
5. Make Pull apply reject any new paginated cloud transcript with non-null
   `history_base`.
6. Run the isolated Codex compatibility matrix and the complete backend
   integration suite.
7. Clear test `~/.mallard` state and test storage before acceptance, consistent
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
