# Codex Subagent Tree Sync Implementation Plan

## Decision and scope

**Status:** proposed; implementation gated on the Step 0 viewer proof.

This plan makes a synced Codex parent thread render its subagent activity
after Pull on another machine. When a user pushes parent thread `A` that
spawned subagents `a1, a2, a3`, a second user who pulls `A` and opens it in
local Codex (CLI, app, or IDE) must be able to expand and read the full
subagent activity, exactly as on the source machine.

The product decision is:

> A spawned subagent transcript is not an independent conversation. It is an
> attached, immutable part of its root parent thread, synced with the parent
> as one resource, and restored under its original provider thread ID.

### Non-goals

- Placing threads into another ChatGPT **account's cloud** thread list.
  Sync restores local rollouts only; chatgpt.com cannot see them.
- Live continuation or resumption of a subagent. This plan restores completed
  activity for viewing; it does not re-attach running agent trees.
- Syncing guardian/internal review subagents (`source.subagent.other`).
  They are referenced by nothing and are excluded from sync entirely.
- Claude-provider sidechains. Those live inside the parent session file and
  need no cross-file work; symmetric treatment is future work.

## Evidence (verified 2026-07-28 on a real Codex home)

Scan of 731 local rollouts (382 of them subagent children) established the
provider contract this plan is built on:

1. **Parents reference spawned children by provider ID, inside `event_msg`
   records.** A `thread_spawn` child appears in its parent rollout as:

   ~~~json
   {
     "type": "event_msg",
     "payload": {
       "type": "sub_agent_activity",
       "event_id": "call_...",
       "agent_thread_id": "<child-uuid>",
       "agent_path": "/root/entire_agent_architecture",
       "kind": "started"
     }
   }
   ~~~

   The record carries no activity content — only the pointer. The viewer must
   therefore load the child rollout **by that ID** from the sessions
   directory. In the sampled parents, the child ID appears **only** in
   `event_msg` records, never in durable messages.

2. **Children are absent from native indexes.** Zero hits for child IDs in
   `~/.codex/session_index.jsonl` and `~/.codex/history.jsonl`. Codex
   discovers children by ID, not via an index, and Pull must not index them
   either.

3. **Guardian children are referenced by nothing.** Children with
   `source.subagent.other` (e.g. `"guardian"`) never appear in their parent's
   rollout. Syncing them adds transcript exposure with zero viewer value.

4. **Child metadata carries the spawn edge.** Child `session_meta` holds its
   own `id`, plus `parent_thread_id` and a parent-valued `session_id`, plus
   `source.subagent.thread_spawn { parent_thread_id, depth, agent_path }`.

Validation fixture from the scanning machine (usable for Step 0):
parent `019fa69a-e44e-73c1-83bc-4abffdbe322e`
(`rollout-2026-07-27T19-43-16-….jsonl`) with `thread_spawn` children
`019fa6b0-c38b-7a12-b6da-de1d4abc50c5` and
`019fa6b0-d933-7a90-9a87-91a40e8a4c68`.

## Consequences for the design

- **Child provider IDs must be preserved verbatim on Pull.** The parent→child
  link lives in `event_msg` payloads, which the transcript rewriter
  deliberately never touches (`rewrite_codex_value` rewrites only structural
  `session_meta`/`turn_context` fields). Minting new child UUIDs orphans the
  links; rewriting event payload content is forbidden (it would falsify
  content and fight digest validation).
- **The parent may keep today's materialization behavior** (fresh UUID, or an
  existing bound ID on fast-forward). Its references to children survive any
  parent remap untouched, because they live in `event_msg`.
- **Children need no logical `ThreadId`, no binding, no checkbox.** Their
  provider UUID is their portable identity end-to-end.

## Architecture

Subagent rollouts become **attached files of the parent's conversation
resource**, not sibling threads:

- One review entry, one checkbox, one atomic Push/Pull group.
- Sha-level dedupe per child file keeps Push incremental.
- Children are immutable once complete; two local copies of the same logical
  parent (a Pull fork) can share the same child files.
- No cross-action ordering or group-atomicity machinery is needed: child
  targets ride in the parent action's existing target batch.

## Implementation steps

### Step 0 — go/no-go viewer proof (~30 min, manual)

1. Copy the fixture parent + two children into a scratch `CODEX_HOME`,
   preserving `sessions/YYYY/MM/DD/` layout.
2. Open the parent in Codex; confirm subagent activity expands and reads.
3. Rename one child's ID (filename + `session_meta.id`) to a fresh UUID;
   confirm the parent's view of that child breaks. This proves preservation
   is required, not merely convenient.
4. Variant: leave children intact but give the parent a different ID
   (simulating today's Pull remap) and confirm children still render, with
   and without the child's `parent_thread_id`/`session_id` rewritten to the
   new parent ID. This tells us whether the viewer validates the back-edge.

If step 2 fails (viewer does not read local children), stop: the goal is not
achievable by file sync and the plan reduces to capture-side grouping/hiding.

### Step 1 — capture groups the tree

`src-tauri/src/project_sync/codex_rollout.rs` already parses
`parent_thread_id` and `thread_source` into `CodexSessionMetaRecord`; both are
currently unread downstream. In
`src-tauri/src/project_sync/provider_capture.rs`:

- Classify each discovered rollout: `thread_spawn` child → attach to its root
  user thread's resource as an additional source file; guardian/`other`
  child → drop from capture entirely.
- Resolve transitively (children of children) by following parent edges to
  the root, bounded by the existing `MAX_CODEX_FORK_DEPTH` pattern; reject
  cycles.
- A child whose parent rollout is missing locally produces a warning, not a
  resource.
- `CapturedConversation` gains the subagent file list
  (`provider_id`, `logical_path`, `sha256`, byte count).
- Flip the expectation of
  `codex_subagent_uses_its_own_id_instead_of_parent_session_id`
  (`provider_capture.rs`): the child no longer appears as its own
  conversation resource.

### Step 2 — portable model

In `src-tauri/src/project_sync/domain.rs`:

- New file role `SubagentTranscript { provider_thread_id }` on
  `ConversationFileRef`; `ConversationThread.files` carries child files.
- Lineage and semantic digests remain parent-transcript-only. The
  "has anything changed" comparison additionally ORs in the child sha-set so
  a child-only delta is pushable.
- Validate child provider IDs as strict UUIDs — they become destination
  filesystem paths. Bound child count and total bytes per thread.
- Schema bump rides the current reset-to-1 window. Old hosts do not decode
  the new role; per the backward-compat removal posture this is acceptable.

### Step 3 — Push

In `src-tauri/src/project_sync/bundle_engine.rs`:

- Upload child payloads under the parent resource, deduped by sha so
  unchanged children are no-ops (consistent with incremental Push).
- Warn when the parent references an `agent_thread_id` whose rollout is not
  on disk (pruned child): Push proceeds, viewer on the destination will show
  the spawn event without expandable detail — same as the source machine.

### Step 4 — Pull

In `src-tauri/src/project_sync/commands.rs`
(`thread_review` and `apply_prepared_pull_conversation`):

- Parent behavior is unchanged (fresh UUID on create/fork; bound ID on
  fast-forward/replace).
- Each child is written at its original logical path with its ID preserved.
  Rewrite only: structural cwd → destination project root, and the child's
  parent-valued `session_id`/`parent_thread_id` → the parent's destination
  ID. This extends `rewrite_codex_transcript`
  (`conversation_codec.rs`) with a parent-ID map; the equality-guarded
  `rewrite_matching_string` pattern already fits. Round-trip validation must
  compare child ancestry under the map.
- Collision rule per child: target ID already present locally with the same
  sha → no-op (fork copies share children); present with a different sha →
  block the action with a review error (children are immutable; a mismatch
  is evidence of corruption or ID reuse).
- **No `session_index.jsonl` rows for children.** While touching
  `codex_index_payload`, align the parent's row with the native shape
  `{id, thread_name, updated_at}` (today Mallard writes `{id, cwd}`), so
  restored parents show titles in the resume picker.
- Child targets join the parent's `ConversationTargetPayload` batch; the
  existing single-operation materialization gives group atomicity.
- Bindings: parent only. Re-capture on the destination reattaches children
  via the parent chain and matches shas, so a pulled tree round-trips as a
  clean no-op.

### Step 5 — review UI

- `ThreadSyncEntry` (`commands.rs`) gains a subagent count and total bytes.
- One checkbox rendered as "Main task (+3 subagent transcripts)" — the label
  doubles as the transcript-exposure disclosure.
- Chat history keeps hiding children
  (`session_meta_is_internal_subagent`, `chat_history.rs`).

### Step 6 — tests

In `src-tauri/src/sync_tests/thread_variants/`:

- Capture: grouping, transitive depth bound, guardian exclusion,
  missing-parent warning.
- Push: incremental child upload, dangling-reference warning.
- Pull: preserved child ID, cwd + parent-edge rewrite, event_msg bytes
  untouched, collision no-op vs mismatch block, fork copies sharing
  children, no child index rows.
- Round trip: pull → re-capture → no-op push.
- Storage parity and relation matrix updates for the child sha-set rule.

## Invariants

1. A child rollout's provider ID is never rewritten.
2. `event_msg` payloads are never rewritten (existing rule; now load-bearing
   for subagent links).
3. Children never receive index rows, logical ThreadIds, or bindings.
4. Guardian/`other` subagents never enter a bundle.
5. A parent and its children materialize in one atomic operation.
6. Child files are immutable: a same-ID different-sha collision blocks.

## Risks and open questions

- **Step 0 is the only unverified assumption**: that local Codex renders a
  copied tree. Everything else is grounded in the scanned rollout format.
- `sub_agent_activity` payload shape may drift across Codex versions. The
  design is robust to drift because it never parses or rewrites the payload —
  only child `session_meta` spawn fields, which also serve capture grouping.
- Back-edge validation by the viewer (child's `parent_thread_id` vs a
  remapped parent) is unknown until Step 0 variant 4; the Step 4 rewrite
  covers it either way.
- Transcript exposure: checking a parent now syncs its children's content.
  The "+N subagent transcripts" label is the disclosure; bundle generations
  are immutable, so unchecking later does not erase prior pushes.

## Sizing

Roughly 6–7 focused days: Step 0 half a day; Steps 1–2 are the bulk (~2
days); Push ~1 day; Pull/apply ~2 days; UI and polish ~1 day; tests
throughout.
