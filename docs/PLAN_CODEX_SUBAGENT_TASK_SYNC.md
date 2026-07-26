# Plan: Sync Codex Tasks Without Internal-Agent Rollouts

## Status

Proposed implementation plan. No migration or backward compatibility will be
provided. Testing starts with a wiped `~/.mallard` and empty cloud buckets.

## Outcome

Mallard will treat a root Codex task and its genuine `thread_spawn`
descendants as one portable, reviewable task:

| Codex rollout | Upload and restore | Review UI |
| --- | --- | --- |
| Root user task | Yes | One row |
| Genuine `thread_spawn` descendant | Yes, as part of its root task | Hidden |
| Guardian, review, compact, or unknown internal subagent | No | Absent |

Genuine spawned-agent rollouts are useful to preserve because Codex can inspect
and continue them after restoration. Guardian and review rollouts are internal
implementation artifacts and do not represent user tasks.

This matches the documented
[Codex subagent model](https://learn.chatgpt.com/docs/agent-configuration/subagents)
and
[app-server thread relationships](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md).

The approved Push and Pull UI remains unchanged: no new labels, badges, icons,
or explanatory copy.

## Current failure

`provider_capture.rs` currently turns every matching Codex JSONL file into an
independent conversation resource. Its bounded metadata scanner retains only
the rollout ID and cwd, losing the source kind and parent relationship.

As a result:

- Guardian and other internal rollouts become selected conversation resources.
- Thread comparison treats every rollout as an independent task.
- Push and Pull review group them by their independent resource IDs.
- Internal rollouts without user-authored titles appear as `Untitled thread`.
- A descendant-only difference can remain outstanding independently of its
  already-synced root task.

The fix belongs in capture and the portable domain model, not in a frontend
title or visibility filter.

## Design

### 1. Add one authoritative rollout classifier

Create a shared, typed Codex session-metadata parser used by provider capture
and local History:

```rust
enum CodexRolloutRole {
    Root,
    Spawned {
        parent_thread_id: ThreadId,
    },
    Internal,
}
```

Classification rules:

- A rollout with no subagent marker is `Root`.
- Only the exact `source.subagent.thread_spawn` source is `Spawned`.
- Any other subagent marker is `Internal`.
- A `thread_spawn` record without a valid parent ID is malformed and excluded.
- Unknown future subagent variants are internal by default.

This is intentionally fail-closed. An unknown internal format must never become
a visible or portable user task.

Replace the broader, duplicated subagent check in `chat_history.rs` with this
classifier so capture and History cannot drift apart.

### 2. Discover complete task trees in two passes

Refactor Codex discovery in `provider_capture.rs`:

1. Scan `sessions` and `archived_sessions` into bounded metadata records keyed
   by each rollout's authoritative own ID.
2. Deduplicate active and archived copies by rollout ID, preferring the active
   file.
3. Classify every rollout.
4. Remove internal rollouts.
5. Build and validate the genuine `thread_spawn` parent graph.
6. Emit one visible capture candidate per root task.

Each root candidate contains the root transcript and every valid, project-owned
spawned descendant transcript.

Keep the existing project boundary:

- The root must belong to the selected project.
- Every included descendant cwd must also belong to the selected project and
  must not fall under an excluded nested project.
- Cross-project descendants are omitted rather than leaking another project's
  conversation data.
- Orphaned or cyclic descendants are omitted with a bounded warning and are
  never promoted to roots.

Filter `session_index.jsonl` using root task IDs only. Spawned descendants do
not need index rows because Codex reconstructs their relationship from rollout
metadata.

### 3. Make the root task the portable resource

Use one resource ID per reviewable task:

```text
codex:session:<root-thread-id>
```

All included transcript files belong to that root resource. Do not create
independent recipe resources for spawned descendants.

Replace the flat portable conversation representation with a typed task model:

```rust
struct ConversationTask {
    root_thread_id: ThreadId,
    resource_id: ResourceId,
    display_title: Option<String>,
    members: BTreeMap<ThreadId, ConversationTaskMember>,
    created_by_origin: Option<PortableOrigin>,
    last_updated_by_origin: Option<PortableOrigin>,
    created_at: u64,
    updated_at: u64,
}

struct ConversationTaskMember {
    thread_id: ThreadId,
    role: ConversationTaskMemberRole,
    relative_cwd: String,
    files: Vec<ConversationFileRef>,
    lineage_status: ConversationLineageStatus,
    lineage_codec: String,
    head_message_id: Option<MessageId>,
    message_count: Option<u64>,
    lineage_digest: Option<String>,
    semantic_digest: Option<String>,
    source_thread_id: Option<String>,
}

enum ConversationTaskMemberRole {
    Root,
    Spawned {
        parent_thread_id: ThreadId,
    },
}
```

The manifest validator must require:

- The task-map key, `root_thread_id`, and canonical resource ID agree.
- Exactly one member is the root.
- Every spawned member's parent exists in the same task.
- Every member is reachable from the root.
- The graph contains no cycles.
- Transcript paths are unique.
- Every member file is owned by the task resource.
- No internal-agent representation can enter the manifest.
- The recipe selects root resources, never individual members.
- The portable session index contains only roots.

### 4. Perform a hard schema cutover

Introduce one new bundle schema, such as `BUNDLE_SCHEMA = 7`, and accept only
that schema.

Remove:

- Schema-4 and schema-6 manifest and head reader branches.
- The parallel legacy and schema-6 publication paths.
- Schema-numbered production type and function names.
- Serde defaults intended only to read older conversation shapes.
- Compatibility fixtures and tests for old bucket contents.
- Cleanup or conversion for old child-resource recipe entries.

Collapse publication into one immutable, compare-and-swap-protected writer.

If a local conversation binding or journal document changes shape, increment
that document's schema and accept only the new value. Do not add migration or
conversion code.

### 5. Compare synchronization state at task level

Build a deterministic task fingerprint from the sorted:

- Member IDs.
- Parent edges.
- Member transcript or lineage digests.

Use member-level ancestry comparison to determine the root task state:

- `Synced`: every member and relationship matches.
- `LocalOnly`: the complete task exists only locally.
- `StorageOnly`: the complete task exists only in storage.
- `LocalAhead`: all differences move toward local, including new descendants.
- `StorageAhead`: all differences move toward storage.
- `Diverged`: member directions conflict, an edge differs, or a member
  diverges.
- `Unavailable`: a required member cannot be safely read or captured.

A change in only a spawned descendant therefore changes the root row's state
and action. It never creates an extra review row.

Do not silently delete stored descendants when a local member is missing.
Treat the task as storage-ahead or diverged. Deletion remains an explicit
whole-task tombstone operation.

### 6. Make Push operate on task roots

The saved recipe and Push review contain only root task resource IDs.

Selecting a root automatically captures every current genuine descendant.
Internal rollouts never enter:

- The resource inventory.
- The recipe.
- The bundle manifest.
- The session index.
- Review counts.

After publication, save a task-level base containing the aggregate fingerprint.
A successful Push must immediately compare as `Synced`, including when the
change originated in a descendant.

### 7. Restore a task as one reviewed operation

Integrate the existing multi-target conversation materialization journal into
the production Pull path.

For one approved root task:

1. Resolve every member's destination using its own project-relative cwd.
2. Stage and SHA-256 verify all member transcripts.
3. Materialize the root first.
4. Materialize descendants in topological parent-first order.
5. Merge the root session-index row.
6. Commit the binding and task-level receipt only after all required targets
   succeed.
7. Recover or roll back through the existing durable journal after
   interruption.

Materialization may rewrite structural cwd fields only. Preserve:

- The member's own `id`.
- Parent-valued `session_id`.
- `parent_thread_id`.
- `source`.
- User messages and historical text.

Approval is all-or-none for the task. A caller cannot approve only some hidden
member actions.

The receipt records the complete member fingerprint. The next Pull comparison
must report the root as `Synced` and hide it with other up-to-date tasks.

### 8. Keep the UI minimal

Return root-task entries from the backend instead of filtering children in
React.

Push and Pull continue to display:

- One checkbox per root task.
- The existing root title and metrics.
- The existing status icon and action.
- One tab count per root task.

The root title comes from the root's session-index metadata. `Untitled thread`
remains a fallback only for a genuinely untitled root task.

Do not add:

- A subagent count.
- A hierarchy expander.
- A descendant badge.
- A new status icon.
- Explanatory text.
- Client-side Guardian or title heuristics.

## Expected source changes

- `src-tauri/src/project_sync_v3/provider_capture.rs`
  - Parse rollout roles, build task trees, and emit root resources.
- `src-tauri/src/project_sync_v3/chat_history.rs`
  - Reuse the canonical classifier and keep only root tasks in normal History.
- `src-tauri/src/project_sync_v3/domain.rs`
  - Add the task/member model, graph validation, and single bundle schema.
- `src-tauri/src/project_sync_v3/conversation_codec.rs`
  - Return typed session source and parent metadata while preserving structural
    rewrite rules.
- `src-tauri/src/project_sync_v3/commands.rs`
  - Compare, select, publish, receipt, and count root tasks.
- `src-tauri/src/project_sync_v3/bundle_engine.rs`
  - Use the single manifest writer and plan grouped task restoration.
- `src-tauri/src/project_sync_v3/conversation_materialization.rs`
  - Apply task members as one durable multi-target operation.
- `src/types.ts`
  - Mirror the new backend DTOs if serialized response shapes change.
- `src/components/project-sync/pullReviewModel.ts`
  - Consume root-task resource grouping without source/title heuristics.
- Push/Pull review components
  - Preserve the approved UI while using root-only backend entries.

## Tests

### Classifier and discovery

- A normal rollout is a root.
- An exact `thread_spawn` rollout is a spawned member.
- Guardian, review, compact, and unknown subagent variants are internal.
- A Guardian nested under a genuine spawned member remains excluded.
- The active copy wins over the archived duplicate.
- Orphaned, cyclic, malformed, and cross-project descendants fail closed.

### Capture and manifest

- Root plus Guardian produces one task resource and no Guardian payload.
- Root plus nested genuine descendants produces one task resource containing
  all genuine transcripts.
- The session index contains root rows only.
- Invalid member graphs and mismatched file ownership are rejected.
- Old bundle schemas are rejected.

### Push and comparison

- Root plus descendants appears as one Push row and one count.
- A child-only local edit makes the root `LocalAhead`.
- A child-only cloud edit makes the root `StorageAhead`.
- Opposing root and child changes make the root `Diverged`.
- A successful Push immediately compares as `Synced`.
- Missing local descendants cannot silently delete stored history.

### Pull and restoration

- Root plus descendants appears as one Pull row.
- Selecting the root approves every member target.
- Restoring into a fresh temporary Codex home recreates the root and genuine
  descendant rollouts.
- Restored IDs, parent relationships, and source metadata remain intact.
- Every member cwd is remapped correctly.
- Guardian content is absent after restoration.
- Interrupted materialization recovers without recording a false synced base.
- A completed Pull immediately compares as `Synced` and is hidden.

### Frontend

- No internal `Untitled thread` rows appear.
- Root task counts do not include descendants.
- Descendant-only differences use the existing root status icon.
- No new UI text, badges, or controls are rendered.

## Verification

Run:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:integration
```

Complete one native Tauri smoke test using temporary or freshly wiped state:

1. Start with an empty bucket and clean `~/.mallard`.
2. Create a root task, a genuine spawned agent, and a Guardian rollout.
3. Confirm Push review shows one task.
4. Confirm the published bundle contains the root and genuine spawned rollout,
   but not the Guardian.
5. Pull into another fresh provider home.
6. Confirm Pull review shows one task.
7. Confirm Codex can read the restored root and genuine descendant.
8. Confirm the next Push and Pull reviews treat the task as up to date.
