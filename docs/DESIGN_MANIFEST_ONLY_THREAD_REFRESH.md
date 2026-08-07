# Design: Manifest-only thread refresh classification

- Status: Implemented
- Last reviewed: July 31, 2026
- Scope: conversation-thread sync classification during refresh in
  `ProjectSync`
- Compatibility: clean cutover; no migration or legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner
  will clear `~/.mallard` and the test cloud buckets before acceptance testing
- Related: [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
  (consumes the `Diverged` classification produced here)

## Decision

Refresh classifies every thread's sync state from three inputs only:

1. the bundle manifest's per-thread metadata — canonical chain length and
   canonical digest pair;
2. the locally computed chain length and digest pair; and
3. the local binding's last-synced content pin.

Refresh downloads no conversation payloads. Every branch of the decision
table is witnessed by content digests; no branch consults a clock, a message
count alone, or a file timestamp.

The decision table, evaluated top to bottom per thread:

| # | condition | classification |
| - | --- | --- |
| 1 | manifest tombstones the thread | tombstone flow (unchanged) |
| 2 | cloud entry absent, local absent | `BothMissing` |
| 3 | cloud entry absent, local present | `CloudMissing` (local outstanding) |
| 4 | cloud entry present, local absent | `LocalMissing` (cloud outstanding) |
| 5 | local bound but chain uncapturable | `Unknown` — this row only; refresh continues |
| 6 | digest pairs equal | `Identical` (in sync) |
| 7 | local len > cloud len, `digests(local[..cloud_len]) == cloud pair` | `CloudPrefixOfLocal` (local outstanding) |
| 8 | local len > cloud len, prefix digests differ | `Diverged` |
| 9 | local len < cloud len, local pair == pin | `LocalPrefixOfCloud` (cloud outstanding) |
| 10 | local len < cloud len, otherwise (including no pin) | `Diverged` |
| 11 | local len == cloud len, digests differ | `Diverged` |

Refresh output is advisory. Push and Pull keep their existing apply-time
verification: pinned revision, commit ID, manifest SHA, and
conditional-write CAS. Anything that moves between refresh and apply fails
closed at apply.

## Goals

- Classify every thread during refresh without reading cloud conversation
  payloads.
- Make every classification content-witnessed; no decision rides on clocks,
  positions, or provider-written timestamps.
- Route every ambiguous case to `Diverged` (review), never to a
  fast-forward.
- Reuse the existing canonical digests and binding pin fields; introduce no
  new hash construction.
- Keep one thread's failure from failing the whole refresh.

## Non-goals

- Change apply-time verification, the divergence resolution flow, or the
  Push non-fast-forward block. This design feeds them the same
  `ConversationRelation` values.
- Merge conversation content.
- Read or preserve manifests that lack the required metadata.
- Use `updated_at` or any timestamp for correctness. Timestamps remain
  presentation metadata.
- Support pre-cutover local state.

## Definitions

### Canonical chain, length, and digest pair

All lengths and hashes in this design are over the **canonical conversation
chain** (`CanonicalConversationMessage` sequence), produced by the provider
codec:

- **length** — number of messages in the chain. Never a rollout record
  count: providers inject and rewrite non-conversation records.
- **digest pair** — `(lineage_digest, semantic_digest)` from
  `conversation_chain_digests` (`domain.rs:3106`). Two pairs are equal only
  when both components are equal; a mismatch of either is a mismatch.
- **prefix digests** — the same function over `chain[..k]`. Local always has
  its own messages, so the row-7 prefix pair is computable locally with no
  cloud read.

Never file bytes. Codex restamps every record envelope and re-serializes
`turn_context` under its current schema when forking or rewriting (see the
fork-verification digest in `codex_native_fork.rs`, which excludes exactly
those). A byte-level hash would classify every faithful rewrite as
divergence.

### Cloud metadata

`ConversationThread` (`domain.rs:2406`) already carries `message_count`,
`lineage_digest`, and `semantic_digest` as optional fields. Under this
design they become **required**:

- Push populates all three for every thread, always.
- The fields lose their `Option` wrappers; manifest `validate()` rejects a
  conversation entry without them.
- `BUNDLE_SCHEMA` stays 1 per the schema-reset policy; pre-cutover
  snapshots are invalid by the deployment assumption, not detected by a
  version gate.

`updated_at` stays in the manifest for display and ordering only. No rule
in the table may read it.

### Local pin

The pin is the binding's existing last-synced record
(`LocalConversationBinding`, `domain.rs:1405`):

- `last_observed_lineage_digest` + `last_observed_semantic_digest` — the
  pin the table compares against (row 9).
- `last_observed_file_sha256` — cache key only (below), never a
  classification witness.
- `last_observed_head_message_id` — diagnostics.

**The pin is written by every successful sync in either direction** — pull
receipts and push receipts both update it (existing write sites:
`conversation_materialization.rs:2335`, `commands.rs:12120`). This is a
correctness requirement, not an optimization: if push did not move the pin,
the standard two-machine flow (A pushes, B pushes, A refreshes) would
classify as `Diverged` on A. "No pin" means the thread has never completed
a sync on this replica.

### Local computation

Refresh recomputes each local thread's chain and digest pair from the
capture walk it already performs. A cache keyed by
`(transcript path, size, file sha256)` could skip recomputation — a file
SHA hit proves byte identity, so a cached pair would be valid; a miss
proves nothing (restamps change bytes without changing the chain). Not
implemented: local parsing is not the cost this design removes. If added
later, file identity is a cache key, never a divergence witness.

## Soundness

Each row's witness:

- **Row 6** — pair equality is content equality. Sync.
- **Row 7** — the local prefix reproduces the cloud pair, so cloud is an
  ancestor of local. Fast-forward push offer is safe.
- **Row 9** — local equals its last-synced state, so nothing local is
  unsaved. Fast-forward pull is lossless *regardless of how cloud moved* —
  even a force-replaced cloud thread overwrites only content that was
  already synced, and Pull backs up the replaced state.
- **Rows 8, 10, 11** — no witness of safety exists; route to review.

Loss scenarios this table was tested against (each previously defeated a
weaker design that used pull timestamps or a length-only pin):

| scenario | classification | correct |
| --- | --- | --- |
| A pull→append→push; B append→push; A refreshes | row 9: cloud outstanding | yes — pin moved at A's push |
| edit-and-resubmit locally, regrows to the pre-edit length; cloud ahead | row 10: `Diverged` | yes — pair ≠ pin despite equal length |
| local truncated below last-synced length | row 10: `Diverged` | yes — pair ≠ pin |
| provider restamps every record (fork, rewrite) | row 6: `Identical` | yes — canonical domain ignores envelopes |
| thread never synced on this replica, exists in cloud | row 10: `Diverged` | yes — absent pin cannot witness safety |
| another machine force-replaced the cloud thread at equal length | row 11: `Diverged` | conservative — review, not loss |

Two corners intentionally classify conservatively (review instead of a
technically available fast-forward); neither can lose content:

- Local edited and then manually restored to an exact cloud prefix — row
  10 (`Diverged`) although a pull would be clean.
- Cloud replaced at equal length over a pin-clean local — row 11
  (`Diverged`) although a pull would be clean. See *Considered and
  deferred*.

## Binding recovery uses the same witness

Discovered during implementation: classifying was not the only refresh-time
consumer of cloud chains.  An unbound local conversation (a thread Codex
re-identified, continued on this replica with no binding yet) recovers its
logical thread by finding the unique longest compatible cloud ancestor —
previously a full chain-against-chain comparison.

Refresh performs the same recovery from metadata:
`recover_unique_compatible_binding_from_meta` marks a manifest thread
compatible exactly when the row-6/row-7 witness holds — the local chain's
digest pair equals the manifest pair, or the locally recomputed prefix pair
does.  Longest-unique selection is unchanged and shared with the chain-based
recovery used by planning.  No new trust is introduced: compatibility is
still proven by content digests, never by titles or provider IDs.

## Stored subagent threads

Push excludes internal subagent rollouts at capture, so post-cutover
manifests cannot contain one.  Refresh therefore does not attempt the
payload-based subagent check; a manifest thread is classified as-is.  Pull
planning, which opens exactly the transcripts it could offer to
materialize (see DESIGN_PUSH_PULL_FLOW.md), still refuses to offer
subagent threads — this split is pinned by
`previously_stored_subagent_tasks_are_hidden_from_pull_but_not_refresh`,
which doubles as a tripwire: if refresh ever hides the row again, payload
reads have crept back into the manifest-only path.

## What refresh stops doing

Today `thread_review` (`commands.rs:4424`) reads cloud conversation payloads
out of `BundleBytes` to build full chains and classify via
`classify_conversation_relation` (`domain.rs:3051`). Under this design:

- Refresh performs classification from the manifest and local state alone.
- Full-chain analysis remains the authority **inside apply** — apply-time
  pin validation re-proves every destructive write against actual content;
  disagreement resolves in apply's favor.
- Superseded in part by
  [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md): planning modes now
  classify from this same decision table too (one classifier for refresh,
  Push review, and Pull planning), and `classify_conversation_relation`
  survives only as the test suite's chain-level oracle.

The `ConversationRelation` enum is unchanged and remains the interface to
directional policy (Push blocked on `Diverged`, pull-first resolution,
holds).

## Failure handling

- **Row 5 (`Unknown`)** — a bound thread whose local transcript cannot be
  captured (unreadable, outside the project root, codec-incomplete) holds
  that row and never errors the refresh. This is the standing lesson from
  the descriptor-lookup failure that blanked the Git history panel: degrade
  per row.
- **Manifest entry missing required metadata** — validation error for the
  bundle read; surfaced as a storage error, not silently skipped. Post-
  cutover such a manifest cannot legitimately exist.
- **Uncomputable local digest with no binding** — the thread is not part of
  refresh scope (nothing to classify against).

## Cutover

1. Make `message_count`, `lineage_digest`, `semantic_digest` non-optional
   on `ConversationThread`; populate at every push site; enforce in
   `validate()`.
2. Confirm both receipt paths write the pin triple; add the push-side write
   if any push path lacks it.
3. Replace refresh-time payload reads with the decision table; keep
   plan/apply reads.
4. Delete any refresh code path that classified from full chains, per the
   no-parallel-paths policy.

No migration. Owner clears `~/.mallard` and test buckets; first
post-cutover push writes complete metadata; first sync per thread writes
the pin.

## Test plan

Fixtures follow the existing `sync_tests` harness style.

- One fixture per table row (1–11), asserting the exact
  `ConversationRelation`.
- The six loss-scenario regressions from *Soundness*, including the
  two-machine ping-pong and the equal-length edit-resubmit, which must
  classify `Diverged` — these two defeated the earlier timestamp- and
  position-based designs and must stay encoded as tests.
- Restamp immunity: a transcript whose envelopes are restamped and whose
  `turn_context` is re-serialized classifies `Identical` (pairs with the
  fork-verification digest tests in `codex_native_fork.rs`).
- Pin lifecycle: pin advances on a committed push receipt at every
  crash checkpoint, stays put on rolled-back ones, and advances on pull
  receipts; absent pin classifies row 10.
- Per-row degradation: a bound thread that is neither capturable nor stored
  yields `Unknown` for itself and correct classifications for every other
  thread.
- Manifest validation: unproven lineage, a zero message count, and a
  malformed digest each fail `validate()`.
- Binding recovery: an unbound local continuation recovers its logical
  thread from manifest metadata alone (the fresh-replica comparison test).

## Considered and deferred

- **Pin-match short-circuit before the length dispatch** (`local pair ==
  pin ⇒ cloud outstanding`, evaluated before rows 7–11): converts the
  equal-length force-replace corner and the cloud-shrank corner from
  `Diverged` to a lossless pull. Deferred: pure review-noise reduction, and
  the cloud-shrank case (push-resurrect vs pull-truncate) is genuinely
  ambiguous and better adjudicated by apply-time verification, which
  already holds pushes that would discard remote content.
- **Cloud-side prefix digest tables** (per-length digests enabling a
  structural check in row 9): O(n) manifest growth per thread for no
  additional safety over the pin — the pin already witnesses "nothing
  local unsaved," which is the property a fast-forward pull needs.
- **Timestamps and length-only pins**: rejected during design review;
  their failure modes are recorded as the loss-scenario regressions above.
