# Plan: make standalone paginated Codex history resumable

## Status and decision

This is the implementation plan for the current Codex standalone-rollout
writer. It keeps the provider-native history contract:

```yaml
history_mode: paginated
history_base: null
```

Mallard will continue to flatten a resolved `history_base` chain into one
standalone JSONL rollout. The writer will preserve the original non-metadata
records and add the current Codex projection records that paginated history
requires.

The projection semantics deliberately mirror the Codex 0.147 rollout
migration canonicalizer, including its tolerance rules: a new explicit turn
start replaces a still-open turn, and display completions outside any open
turn are attached to a synthesized implicit `rollout-N` turn. These shapes
are not edge cases. A record-type scan of the 1,235 local rollouts on this
machine (types only, no content) found 202 files with a turn start while a
turn was still open, 151 files with display completions outside any open
turn, and 77 files with a turn close that matches no open turn — 273 files
(22.1%) in union. An earlier fail-closed draft of this plan would have made
all of them permanently unsyncable. Upstream accepts them; Mallard must too.
Preserving every conversation in the supported pre-`item_completed` event
shapes is the required input-compatibility boundary.

The implementation is a clean cutover:

- one writer and one projector state machine (verification reruns the same
  machine; there is no second hand-written verifier);
- no feature flag or alternate legacy output mode;
- no in-place rollout migration;
- no compatibility reader for previously materialized broken output;
- no Mallard read or write of `thread_history_1.sqlite`; and
- no local or portable schema change.

An already materialized rollout whose Codex projection cursor reached EOF
without storing items will not repair itself. It must be removed through the
normal reviewed product flow and pulled again under a fresh provider thread
ID.

## Problem

Codex paginated history is projection-backed. Its projector records a turn
row only from `turn_started` / `turn_complete` / `turn_aborted` events, and
records a display item row only from an `event_msg` whose payload type is
`item_completed`. It deliberately ignores ordinary `response_item` records
and old display events such as `user_message` and `agent_message`. It
performs no turn validation of its own; every tolerance decision lives in
the writer.

Mallard currently produces a valid standalone file with one authoritative
`session_meta`, `history_mode: "paginated"`, `history_base: null`, the
complete model-visible conversation, and contiguous ordinals. However, a
flattened source usually contains only the older display events: the local
corpus is 5,529 `legacy` plus 859 pre-mode session metas against 3
`paginated`. Codex projects the turn rows, advances its projection cursor to
EOF, and stores zero item rows. Resume succeeds, but the previous chats are
absent.

The upstream Codex 0.147 contract confirms this split (all verified at the
pinned commit):

- [paginated history and the item/turn APIs](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/app-server/README.md);
- [the stateless paginated rollout projector](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/app-server-protocol/src/protocol/thread_history_projection.rs);
- [Codex's canonical paginated writer](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/thread-store/src/local/rollout_migration/canonicalizer.rs);
- [Codex's old-event to current-item mapping](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/thread-store/src/local/rollout_migration/legacy_event.rs);
- [the rollout persistence policy](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/rollout/src/policy.rs)
  (paginated mode does not persist legacy `user_message`/`agent_message`
  events, so current-format files never interleave the two shapes); and
- [the `TurnItem` contract](https://github.com/openai/codex/blob/be6e8eac029b183056b7e4402879f15d2c85f61b/codex-rs/protocol/src/items.rs).

Mallard implements the small provider adapter needed at its standalone
artifact boundary. It does not copy Codex's migration subsystem or depend on
Codex's Rust crates.

## Desired output

For a turn that currently looks like this:

```text
task_started(turn-1)
response_item(user message)
user_message
agent_message
response_item(assistant message)
task_complete(turn-1)
```

the standalone writer emits:

```text
session_meta(output-thread, paginated, null)       ordinal 0
task_started(turn-1)                               ordinal 1
response_item(user message)                        ordinal 2
user_message                                       ordinal 3
item_completed(output-thread, turn-1, UserMessage) ordinal 4
agent_message                                      ordinal 5
item_completed(output-thread, turn-1, AgentMessage)ordinal 6
response_item(assistant message)                   ordinal 7
task_complete(turn-1)                              ordinal 8
```

For display completions outside any open turn — common in real rollouts
(out-of-turn `agent_reasoning`, `patch_apply_end`, `sub_agent_activity`,
messages around crashes) — the writer synthesizes an implicit turn exactly
as the upstream canonicalizer does:

```text
session_meta(output-thread, paginated, null)         ordinal 0
task_started(rollout-0)                              ordinal 1
agent_reasoning                                      ordinal 2
agent_reasoning                                      ordinal 3
item_completed(output-thread, rollout-0, Reasoning)  ordinal 4
task_complete(rollout-0)                             ordinal 5
task_started(turn-1)                                 ordinal 6
...
```

The two reasoning records above produce one cumulative `Reasoning` item,
emitted once after the run's last reasoning record — not one snapshot per
event. The old records remain in the same relative order. The only additions
are projection records and implicit-turn lifecycle records. Ordinals are
assigned after all additions.

## Output invariants

Every standalone root must satisfy all of these rules.

1. There is exactly one `session_meta`, at ordinal `0`. Its `id` is the
   final output provider thread ID, its `history_mode` is exactly
   `"paginated"`, and its `history_base` is present and exactly `null`.
2. Source-segment `session_meta` records are not copied into the body. Later
   same-thread metadata updates are folded into the authoritative head using
   the existing builder behavior.
3. Every original non-metadata record occurs exactly once and in source
   order.
4. The only permitted changes to an original non-metadata record are the
   existing reviewed CWD rewrite, top-level ordinal reassignment, and the
   `thread_id` inside a current `item_completed` envelope, which is rewritten
   to — or inserted as, when the source record lacks it — the output provider
   thread ID.
5. Exactly two classes of inserted records are permitted:
   - `event_msg`/`item_completed` projections for recognized display
     completions; and
   - `event_msg`/`turn_started` and `event_msg`/`turn_complete` records for
     synthesized implicit `rollout-N` turns.
   Both classes are `event_msg` records, which the conversation codec
   ignores for chain identity, so neither affects conflict identity.
6. Every recognized display completion is covered by exactly one
   current-format `item_completed` record. A reasoning run — the cumulative
   reasoning item described below — counts as one completion.
7. Every projected `item_completed` carries the final output provider thread
   ID and the turn ID selected by the association rules below.
8. Item IDs are non-empty, bounded, and deterministic.
9. Output ordinals are exactly `0..record_count - 1` and the file ends in
   one newline.
10. The projection pass is a fixpoint on its own output: running it again
    adds nothing and changes nothing. Rebuilding an already-normalized
    standalone file with the same identity and CWD is byte-identical.
11. The canonical conversation chain, lineage digest, semantic digest,
    portable tail, and message count are unchanged. Inserted records affect
    the complete artifact SHA-256, not conversation conflict identity.
12. Every inserted field derives from source data: timestamps come from the
    source record that triggered the insertion, never from the wall clock.
13. Every `event_msg` payload type in the source is classified by the frozen
    adapter — display completion, turn lifecycle, current projection, or
    context-only. An unknown `event_msg` payload type fails closed. Unknown
    outer record types pass through unchanged: the Codex projector consumes
    only `event_msg` payloads, so an unrecognized outer type cannot carry a
    display completion, and blocking it would only regress the forward
    compatibility the standalone design already promises.

## Inserted record shapes

The `item_completed` projection uses the shape verified against the 0.147
`ItemCompletedEvent` struct, whose `thread_id`, `turn_id`, and `item` fields
are required for deserialization:

```json
{
  "timestamp": "2026-08-12T18:45:04.320Z",
  "type": "event_msg",
  "payload": {
    "type": "item_completed",
    "thread_id": "019ff74a-1143-7bc1-94f3-de1da59dd1ee",
    "turn_id": "019ff74a-d367-76b2-8568-521cdd846adb",
    "item": {
      "type": "UserMessage",
      "id": "item-1",
      "content": [
        { "type": "text", "text": "Hello", "text_elements": [] }
      ]
    },
    "completed_at_ms": 1786560304320
  },
  "ordinal": 4
}
```

`started_at_ms` is optional and omitted. Both the top-level `timestamp` and
`completed_at_ms` come from the source completion record's RFC 3339
timestamp (for a reasoning run, its last record). A missing or unparseable
timestamp on a display completion is an invalid standalone source, not a
reason to use wall-clock time.

Synthesized implicit-turn records use the minimal current shapes and the
0.147 wire names, which are `task_started` and `task_complete` (the
`turn_*` spellings are accepted aliases on read). Verified against 0.147:
neither `TurnStartedEvent` nor `TurnCompleteEvent` carries a `thread_id`,
so these records need no identity rewriting on reidentify, and their
`started_at`/`completed_at` values are epoch seconds, matching the
upstream canonicalizer.

```json
{"timestamp": "...", "type": "event_msg",
 "payload": {"type": "task_started", "turn_id": "rollout-1",
             "started_at": 1786560304}, "ordinal": 1}
{"timestamp": "...", "type": "event_msg",
 "payload": {"type": "task_complete", "turn_id": "rollout-1",
             "completed_at": 1786560309}, "ordinal": 5}
```

Their timestamps come from the source record that opened the implicit turn
and from the source record whose arrival closed it (at EOF, the last source
record).

## Projection ownership

Add one private provider module:

```text
crates/mallard-infrastructure/src/codex_paginated_projection.rs
```

It owns only three concerns:

- classifying Codex `event_msg` payloads against the frozen 0.147 lists;
- converting a recognized completion into the supported current `TurnItem`
  JSON shape, with the turn-association and item-ID rules below; and
- the fixpoint check that reruns the same pass over serialized output.

There is exactly one projection state machine. Verification is running it
again, not a second implementation of the same rules.

`codex_standalone_builder.rs` remains responsible for history expansion,
metadata folding, identity/CWD rewrites, final ordinal assignment, byte
serialization, artifact hashing, and whole-output verification.

No public DTO is added. No frontend, domain, manifest, checkpoint, binding,
or journal type changes. Mallard's session timeline already prefers
canonical items over legacy events without duplicates, so no display change
is needed on the Mallard side either.

## Builder flow

Change the builder from direct record-to-byte emission to one bounded
in-memory normalization pass:

```rust
fn build_standalone_codex_root(...) -> Result<StandaloneConversationArtifact, String> {
    let authoritative_meta = effective_authoritative_meta(...)?;

    // Existing history_base traversal. Preserve provenance and all
    // non-session records, but do not serialize yet.
    let original = collect_flattened_non_metadata_records(resolved, target_cwd)?;

    // New provider-only step. Existing current item_completed records are
    // envelope-validated and identity-normalized; uncovered completions
    // gain projections; out-of-turn completions gain implicit turns.
    let projected = complete_paginated_item_projection(original, &output_root_id)?;

    // One final serialization pass assigns 0..N-1.
    let output = serialize_contiguous(authoritative_meta, projected)?;

    validate_standalone_record_layout(&output)?;
    assert_projection_fixpoint(&output, &output_root_id)?; // rerun, expect no change
    verify_canonical_chain_tail_and_artifact_once(...)?;
    ...
}
```

This is still one reconstruction and one output hash during Push apply or
Pull preparation. Planning does not construct the artifact.

## Turn association

The projection pass operates over the complete flattened sequence, not one
physical history segment at a time, so association state survives a segment
boundary. The rules mirror the 0.147 canonicalizer. Turn lifecycle records
are always originals passed through unchanged (or synthesized implicit-turn
records); Mallard never validates the provider's turn graph, because the
Codex projector does not either. Association state exists only to pick the
`turn_id` for projections it emits.

```rust
struct ProjectionState {
    /// (turn_id, explicit). Implicit turns are Mallard-synthesized.
    active_turn: Option<(String, bool)>,
    reasoning_run: Option<ReasoningAccumulator>,
    item_ids_in_use: BTreeSet<String>,
    next_generated_item_index: u64,
}
```

Rules:

- `task_started` / `turn_started` opens its explicit `turn_id`. If an
  implicit turn is active, its synthesized `turn_complete` is emitted first.
  If an explicit turn is active, it is simply replaced — no error and no
  synthetic close, matching upstream. Crash-shaped rollouts (a start with no
  prior close) are ordinary inputs.
- `task_complete`, `turn_complete`, and `turn_aborted` (legacy `task_*`
  spellings included) pass through and clear the active turn when their ID
  matches it. A close that matches nothing is passed through with no state
  effect; the corpus contains 77 such files and upstream tolerates them.
- A display completion while a turn is open belongs to that turn.
- A display completion with no open turn opens a synthesized implicit turn:
  a `task_started` record with ID `rollout-N` is inserted before the
  completion, where `N` is the completion's index in the flattened source
  sequence — the direct analog of the upstream canonicalizer's
  `rollout-{source_index}` naming. The implicit turn closes — emitting its
  `task_complete` — when an explicit turn starts or at end of input, and a
  second out-of-turn user message closes the previous implicit turn first,
  matching upstream. Source-position naming keeps every synthesized ID
  unique within the file and stable across rebuilds: a rebuild sees the
  previously synthesized records as ordinary explicit lifecycle records and
  synthesizes nothing for them, and appended continuation records only ever
  occupy later positions.
- A completion whose own turn hint names an unknown turn while no turn is
  open becomes an implicit turn under the hint ID, again matching upstream.
- An explicit turn still open at end of input stays open, matching upstream.
- An existing current `item_completed` keeps its own `turn_id` verbatim.
  The projector does not check it against known turns; it is provider data
  being round-tripped.

Every emitted `item_completed.thread_id` is the final output provider thread
ID. This applies both when preserving identity for Push and when using the
existing CLI-first fresh UUID allocator for Pull.

## Stable item IDs

Use one deterministic allocator compatible with Codex's canonical writer:

```text
item-1, item-2, item-3, ...
```

The pass first indexes every explicit ID already present in a current
`item_completed` record and every source completion ID that must be
retained, such as a tool `call_id`. Before allocating an ID for a legacy
completion, it checks whether the immediately following record is that
completion's exact existing projection and adopts its ID. Only an uncovered
completion allocates the lowest unused positive index, in projection
emission order.

ID policy:

- preserve an existing current `item_completed.item.id`;
- preserve an explicit completion identity such as `call_id`, `event_id`,
  or review `item_id`;
- generate `item-N` only for id-less items: legacy user messages, agent
  messages, reasoning runs, and context compaction; and
- never reuse an ID within one output file.

The generated sequence depends only on the flattened source order. It does
not include the output provider thread ID, CWD, clock, host, storage, or
machine, so Push and reidentified Pull builds remain deterministic, and
preserve-identity and reidentified builds contain the same turn/item
structure.

## Reasoning runs

`agent_reasoning` and `agent_reasoning_raw_content` events accumulate into
one cumulative `Reasoning` item per run, appending to `summary_text` and
`raw_content` respectively. A run ends at the upstream reset boundaries — a
`user_message`, a hook prompt, any turn lifecycle record, a current
`item_completed` record, or any other display completion written to the
active turn — or at end of input. The one upstream exception carries over:
a completion attributed to a different turn through its own turn hint does
not end the run. Empty reasoning text passes through without opening a run.

Mallard emits the run's single cumulative `item_completed(Reasoning)`
immediately after the run's last reasoning record. It does not emit one
snapshot per reasoning event. Upstream's migration re-emits the growing
snapshot on every event because it is a streaming transformation; a batch
writer gains nothing from the intermediate snapshots (the projector upserts
by item ID, so only the final state survives) and `agent_reasoning` is the
most common display event in the corpus at 192,413 of 511,049 events —
per-event cumulative snapshots would inflate output quadratically per run
against the 100 MiB / 100,000-record transcript bounds.

## Display completion mapping

The supported mapping is frozen to the Codex 0.147 contract and matches the
upstream canonicalizer row for row. It is small enough to implement directly
with bounded `serde_json::Value` parsing; no Codex crate or new dependency
is needed.

| Source completion | Current item | Item ID |
| --- | --- | --- |
| `user_message` | `UserMessage` with text, text elements, remote/local images, and remote/local audio | generated |
| user `response_item` message whose content is entirely serialized `<hook_prompt>` fragments | `HookPrompt`, per the upstream extraction rule; the response item itself still passes through | response item ID when supplied, otherwise generated |
| non-empty `agent_message` | `AgentMessage` with text, phase, and memory citation; empty messages are skipped, matching upstream | generated |
| `agent_reasoning` / `agent_reasoning_raw_content` run | one cumulative `Reasoning` item per run | one generated ID per run |
| `exec_command_end` | `CommandExecution` | source `call_id` |
| `patch_apply_end` | `FileChange` | source `call_id` |
| `mcp_tool_call_end` | `McpToolCall` | source `call_id` |
| `web_search_end` | `WebSearch` | source `call_id` |
| `image_generation_end` | `ImageGeneration` | source `call_id` |
| `dynamic_tool_call_response` | `DynamicToolCall` | source `call_id` |
| `context_compacted` | `ContextCompaction` | generated |
| `entered_review_mode` | `EnteredReviewMode` | source `item_id`, otherwise generated |
| `exited_review_mode` | `ExitedReviewMode` | source `item_id`, otherwise generated |
| `sub_agent_activity` | `SubAgentActivity` | source `event_id` |
| current `item_completed` | preserve its complete `item` snapshot | preserve |

Context-only `event_msg` payload types remain original records and create no
items. The frozen list, which covers every payload type observed across the
1,235-file corpus and the 0.147 persistence policy:

```text
token_count, thread_settings_applied, thread_goal_updated,
thread_rolled_back, thread_name_updated, error, view_image_tool_call,
item_started
```

`view_image_tool_call` is context-only deliberately: the upstream migration
mapping produces no item for it, and matching canonical migration output is
the contract. Turn lifecycle types (`task_started`, `turn_started`,
`task_complete`, `turn_complete`, `turn_aborted`, `task_aborted`) are the
association class above.

An unknown `event_msg` payload type blocks standalone
publication/materialization until the Codex adapter is updated; it must not
be guessed or silently omitted, because an unrecognized legacy display event
is exactly how items go missing. Non-`event_msg` outer record types
(`response_item`, `compacted`, `turn_context`, `world_state`,
`inter_agent_communication_metadata`, and anything future) pass through
unchanged as today; hook-prompt detection reads `message`-kind response
items only.

### Validation depth for preserved items

A preserved current `item_completed` is validated at the envelope only:
non-empty `turn_id`, non-empty bounded `item.id`, an `item.type` in the
frozen 0.147 `TurnItem` set, and `thread_id` normalized to the output ID.
The full variant set is:

```text
UserMessage, HookPrompt, AgentMessage, Plan, Reasoning, CommandExecution,
DynamicToolCall, CollabAgentToolCall, SubAgentActivity, WebSearch,
ImageView, Extension, ImageGeneration, EnteredReviewMode, ExitedReviewMode,
FileChange, McpToolCall, ContextCompaction
```

Note the set is wider than the legacy mapping table: `Plan` items appear
inside real legacy-mode rollouts (the persistence policy carves them out,
and the corpus confirms them), and `CollabAgentToolCall`, `ImageView`, and
`Extension` occur in current-format files. Mallard does not deep-validate
the payload fields of items it merely round-trips: upstream itself does not
deny unknown fields, Codex is the consumer of these bytes, and a
field-level validator would turn every additive upstream change into a
publication block. Items Mallard converts are inherently shape-complete
because Mallard constructs them. An unknown `item.type` fails closed.

## Avoiding duplicate projection records

The pass must accept its own output without adding another copy.

- A current `item_completed` record is envelope-validated, identity
  normalized, and retained.
- A legacy completion whose immediately following record is its exact
  projection — equal normalized payload modulo the `thread_id` identity
  field and the top-level ordinal — is already covered; the projection is
  adopted, ID included. For a reasoning run the check applies after the
  run's last record.
- Any other legacy completion receives one inserted projection. There is no
  "adjacent but different" failure: an unrelated adjacent current item
  (for example a `Plan` item inside a legacy file) simply means the
  completion is not yet covered. Exact-match adoption never guesses
  ownership, and no supported producer emits non-adjacent projections:
  Mallard writes them adjacent, the upstream migration replaces legacy
  events with items outright rather than preserving both, and live
  paginated files contain no legacy display events at all — so
  legacy-plus-projection interleaving only ever comes from Mallard's own
  output.
- An adopted projection record replays as pure output: it is preserved
  without touching the association or reasoning state, exactly as when it
  was first emitted. A current record that was not adopted by the
  completion before it ends the active reasoning run, matching upstream.
- A current-only paginated rollout remains current-only; Mallard does not
  synthesize old events.

## Preservation proof

After projection, verify two independent properties.

### Original-record subsequence

Remove the allowed envelope changes from both sides: top-level `ordinal`,
reviewed structural CWD rewrites, `item_completed.thread_id`
normalization, and authoritative metadata normalization. Every collected
source non-metadata record must then occur exactly once and in the same
order in the output, and every record not in that subsequence must belong to
one of the two permitted insertion classes.

### Fixpoint

Rerun `complete_paginated_item_projection` over the parsed output records
and require zero insertions and zero changes. This one check proves
coverage, ID stability, and idempotence with the same state machine that
produced the output, instead of a second hand-written verifier that would
drift from the first.

The existing canonical-chain verification remains separate. It proves that
the inserted records did not change conversation semantics; the fixpoint
proves that Codex will be able to display those semantics.

## Push and Pull integration

Both paths already call `build_standalone_codex_root`, so orchestration does
not gain a second normalization path.

### Push

```text
plan:
  compare P/L/C and pin the source/history ranges
  do not build output

apply:
  revalidate pins
  resolve history_base once
  build + project + serialize once
  verify and hash once
  upload immutable object
  CAS HEAD last
```

### Pull

```text
plan:
  compare P/L/C and pin the reviewed cloud object/action
  do not materialize output

apply preparation:
  verify and read the cloud object once
  resolve its standalone root once
  reidentify + project + serialize once
  verify and hash once

apply commit:
  install at the already planned no-clobber target
  append the existing session-index title entry when required
  commit the reviewed binding/checkpoint documents
```

The existing CLI-first UUID allocator remains the only Pull allocator. The
projector consumes the already planned ID; it never mints a thread ID.

## SQLite ownership

Mallard must not create, copy, reset, patch, query for repair, or include
`thread_history_1.sqlite` in a plan.

On the first Codex read/resume of a fresh pulled provider ID:

1. Codex sees `history_mode: paginated` and `history_base: null`.
2. With no projection state, Codex starts at byte offset `0` and ordinal `0`.
3. It projects the turn lifecycle records, including Mallard's synthesized
   implicit turns.
4. It inserts/updates items from the emitted `item_completed` records.
5. It advances its own byte/ordinal projection cursor to EOF.
6. Later resumes read the projected turns/items from Codex's database.

This keeps a versioned, lock-sensitive Codex database entirely
provider-owned. It also means the target provider ID must be fresh.
Replacing a rollout behind an existing projection cursor is unsupported and
remains prohibited by the no-clobber Pull contract.

## Schema and metadata impact

None.

- Portable bundle schema remains `1`.
- Machine-local Mallard schema remains `1`.
- `ConversationThread`, bindings, checkpoints, plans, journals, and receipts
  do not gain fields.
- The portable root-tail comparison is unchanged.
- The only portable difference is the standalone object bytes and therefore
  their normal full artifact/object SHA-256.
- No provider database is portable.

This is a writer-contract correction inside the existing Codex transcript
object, not a Mallard metadata feature.

## Overhead

Measured against the same 1,235-rollout local corpus (record types and line
lengths only, no content), modeling each inserted projection as its source
event's bytes plus a fixed envelope, one cumulative reasoning item per run,
and a 330-byte lifecycle pair per implicit turn.

Artifact size. Projection adds 2.06 GiB to the 8.39 GiB corpus — 24.6%
aggregate. Per file the byte overhead is p50 10.4%, p90 41.6%, p99 64.5%,
max 97.6%; in absolute terms p50 is ~76 KiB and p90 ~2.3 MiB. Cloud object
size, hashing, and upload time grow by the same ratio. Nothing else in the
bundle model changes size.

Record count. Per file p50 +11.0%, max +36.8%. The largest post-projection
file holds 21,890 records against the 100,000-record bound — no headroom
risk.

Byte cap. 8 corpus files already exceed the 100 MiB
`MAX_CONVERSATION_FILE_BYTES` bound today and cannot sync regardless. 11
more files (0.9%) are pushed over it by projection overhead; they fail
closed at the existing bounded-transcript check before any upload, exactly
as an oversized transcript does today. If those conversations matter, the
remedy is a deliberate, reviewed revision of the domain bound — not part of
this change.

Reasoning policy, quantified. Emitting upstream's per-event cumulative
reasoning snapshots instead of one item per run would add a further 545 MiB
corpus-wide (another 6.5% of original bytes) and push the worst single file
to 200.6% overhead. The emit-once rule keeps the identical projected end
state at a fraction of the growth.

Build cost. The builder already parses every record once; the projection
pass adds linear classification over the same bounded values, and the
fixpoint check adds one more linear parse-and-project pass over the output.
Both run only inside Push apply and Pull preparation, which are already
single-reconstruction paths dominated by hashing and transport; planning
still constructs nothing. Peak memory becomes the collected record vector
instead of streamed lines, bounded by the existing 100 MiB transcript cap.

Provider cost. Codex's first resume replays the projection over the whole
file and writes its own database rows; that cost is proportional to file
size, provider-owned, and unchanged in kind by this plan.

## Clean cutover

- The fixpoint verifier is mandatory whenever Mallard publishes or
  materializes a paginated Codex standalone artifact.
- There is no fallback to `history_mode: legacy`.
- There is no second artifact encoding and no feature flag.
- There is no background scan or migration of existing local rollouts.
- There is no direct repair of an already advanced Codex projection cursor.
- Existing bad materializations are removed and pulled again through the
  reviewed product flow.
- A future Codex event/item contract change is an adapter update with new
  fixtures, not permissive unknown-event conversion.

## Implementation files

Minimal production changes:

1. Add
   `crates/mallard-infrastructure/src/codex_paginated_projection.rs` with
   the private projector, classification lists, and fixpoint entry point.
2. Register that private module in
   `crates/mallard-infrastructure/src/lib.rs`.
3. Refactor
   `crates/mallard-infrastructure/src/codex_standalone_builder.rs` to
   collect bounded values, call the projector once, serialize ordinals once,
   and assert the fixpoint.
4. Add focused unit fixtures beside those modules.

No production change is expected in `commands.rs`, `conversation_codec.rs`,
the frontend, domain schemas, persistence, storage transport, or journal
recovery.

When implementation lands, update `docs/DESIGN_CODEX_STANDALONE_THREAD_UPLOAD.md`:
its preservation verifier currently requires everything except envelope
fields to compare equal, and must instead name the two permitted insertion
classes and the `item_completed` identity normalization, with source-segment
`session_meta` records still excluded in favor of the one effective head.

## Tests

### Projector unit tests

- User and agent messages produce exactly two visible items in their owning
  turn while all original records remain.
- Every mapping row has a sanitized Codex 0.147 fixture.
- Remote/local image and audio inputs remain structurally identical in the
  projected `UserMessage`.
- Explicit tool/review IDs are preserved; id-less items receive stable
  `item-N` IDs; existing IDs force the allocator to skip collisions.
- A reasoning run interleaved with tool completions emits one cumulative
  `Reasoning` item after its last reasoning record.
- Corpus-shape tolerance fixtures, modeled on the real scan (sanitized, not
  read from any real provider home): a `task_started` while a turn is open;
  display completions before the first turn start; a close matching no open
  turn; an unclosed explicit turn at EOF; a legacy file containing a current
  `item_completed(Plan)` adjacent to a legacy completion. All must build,
  and the implicit-turn outputs must match the expected `rollout-N` records.
- Every context-only type in the frozen list passes through without items;
  an unknown `event_msg` payload type fails closed; an unknown outer record
  type passes through.
- Existing current `item_completed` records are not duplicated; a record
  lacking `thread_id` gains the output ID.
- Rerunning the pass on its own output is a fixpoint, and rebuilds are
  byte-identical, including across a preserve/reidentify pair.
- Invalid display-completion timestamps fail closed.

### Standalone builder tests

- A three-segment `P0 -> P1 -> P2` history remains one root, one metadata
  record, and one contiguous ordinal sequence after item insertion.
- Association state carries across a physical segment boundary, including a
  turn left open by a segment cutoff.
- Preserve-identity and reidentified builds contain the same turn/item
  structure, with only the output thread/CWD identity changed.
- Original non-metadata records are an exact normalized subsequence of
  output, and every insertion belongs to a permitted class.
- Canonical chain, lineage, semantic digest, message count, and portable
  tail are unchanged before and after projection.
- Artifact SHA-256 is deterministic and covers the added records.
- Projected growth beyond existing byte/record limits fails before any
  upload or visible installation.

### Push/Pull integration tests

- Push planning never reconstructs the artifact; apply constructs it once.
- Pull planning never constructs the provider output; apply preparation
  constructs it once.
- A pulled fresh provider ID gets current `item_completed.thread_id` values
  equal to that ID.
- A CAS loss or stale Pull plan does not install partially projected output.
- Repeated Pull remains no-clobber and does not try to rewrite an existing
  rollout or Codex database.

### Real Codex contract smoke test

Use an isolated temporary `CODEX_HOME`; never use a developer's real
provider home. The Codex binary comes from an explicit environment override
(for example `MALLARD_SMOKE_CODEX_BIN`); the test first asserts the binary
reports a 0.147-contract version and skips with a clear message otherwise,
so it stays hermetic and never silently tests a different contract.

1. Start with no `thread_history_1.sqlite`.
2. Install one sanitized standalone rollout — including an implicit-turn
   fixture — and matching session-index entry under a fresh UUID.
3. Invoke the supported Codex app server and resume/read the task.
4. Assert the full expected turn count and item count are visible,
   including the `rollout-N` turn.
5. Assert user/agent text, attachments, tool results, statuses, and turn
   grouping match the source fixture.
6. Assert Codex created its own database and projection rows.
7. Resume again and assert no duplicate turns or items.

The Rust test suite must not inspect or manufacture the SQLite schema as
part of production behavior. The smoke test treats the database only as a
Codex side effect; its product assertion is the app-server response.

## Delivery order

1. Check in sanitized current-format fixtures from the supported Codex
   0.147 contract, including the corpus-shape tolerance fixtures.
2. Implement the private projector and its conversion, tolerance, and
   classification tests.
3. Refactor the standalone builder to collect, project, serialize, and
   assert the fixpoint.
4. Add preservation, idempotence, digest, and history-chain tests.
5. Run existing backend integration tests to prove Push/Pull still use one
   builder.
6. Run the isolated real-Codex resume smoke test.
7. Update the standalone design document's preservation wording as
   described above.

## Acceptance criteria

The work is complete when all of the following are true:

- a freshly pulled task remains `paginated` with `history_base: null`;
- Codex shows all prior turns and displayable items on first resume;
- every corpus-observed input shape builds: nested turn starts, display
  completions outside turns, closes matching no open turn, unclosed turns
  at EOF, and legacy files containing current `Plan` items — none of the
  22.1% tolerance class fails closed;
- every corpus-observed `event_msg` payload type is classified by the
  frozen adapter;
- the original non-metadata rollout records remain present and ordered, and
  all insertions belong to the two permitted classes;
- every emitted item has the output thread ID, its associated turn ID, and
  a stable item ID; output ordinals are contiguous;
- the projection pass is a fixpoint on its own output;
- Codex, not Mallard, creates and maintains `thread_history_1.sqlite`;
- Push/Pull still reconstruct and hash output only once in their apply
  path;
- no schema, migration, compatibility mode, feature flag, or database
  repair path was introduced; and
- the existing CLI-first UUID allocator remains unchanged.
