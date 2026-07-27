# Plan: Exclude Codex Subagent Rollouts from Sync

## Status

Proposed implementation plan.

This is a clean root-task-only implementation:

- No migration.
- No backward-compatibility path.
- No cleanup of previously captured subagent resources.
- Testing starts with a wiped `~/.mallard` and empty cloud buckets.
- Genuine `thread_spawn` descendant sync is deferred to a future TODO.
- Capture metadata retains the typed rollout kind and parent ID needed by that
  future work.

This plan does not change implementation files.

## Outcome

Mallard will sync only user-visible root Codex tasks.

| Codex rollout | Upload and restore | Push/Pull UI |
| --- | --- | --- |
| Root user task | Yes | One row |
| Genuine `thread_spawn` descendant | No, deferred | Absent |
| Guardian, review, compact, or unknown subagent | No | Absent |

All subagent rollouts are filtered before they become resources. The frontend
does not hide them by title, resource ID, or another presentation heuristic.

The approved UI remains unchanged:

- No new text.
- No new badges.
- No new status icons.
- No descendant count or hierarchy.

## Current failure

Codex History already treats every rollout carrying a subagent marker as
internal. Provider capture does not reuse that decision.

`provider_capture.rs` currently:

1. Reads only the rollout ID and cwd from bounded session metadata.
2. Turns every matching JSONL into an independent conversation resource.
3. Selects every discovered conversation by default.
4. Adds every selected rollout ID to the filtered session index.

Consequently, Guardian and other subagent rollouts enter the recipe, bundle,
Push/Pull comparison, and review counts. Rollouts without a user title appear
as `Untitled thread`.

The correction belongs at the Codex metadata and capture boundary. A UI-only
filter would leave the unwanted data in recipes, bundles, comparisons, and
restores.

## Implementation

### 1. Add one shared Codex rollout classifier

Create a small filesystem-free module, for example
`project_sync_v3/codex_rollout.rs`, that parses the authoritative
`session_meta` payload:

```rust
enum CodexRolloutClass {
    Root,
    ThreadSpawn,
    InternalSubagent,
    UnknownSubagent,
}

struct CodexRolloutMetadata {
    thread_id: String,
    cwd: PathBuf,
    class: CodexRolloutClass,
    parent_thread_id: Option<String>,
}
```

Classification rules:

- No subagent marker means `Root`.
- `source.subagent.thread_spawn` means `ThreadSpawn`.
- Guardian, review, compact, and other recognized internal sources mean
  `InternalSubagent`.
- `thread_source == "subagent"` or an unrecognized `source.subagent` shape
  means `UnknownSubagent`.
- Malformed subagent metadata never falls back to `Root`.
- The authoritative own `id` remains the rollout identity. A parent-valued
  `session_id` must not replace it.
- A genuine `ThreadSpawn` retains its validated `parent_thread_id` in the
  bounded metadata record even though the rollout is excluded from capture.
- Root rollouts must expose `parent_thread_id: None`; parent-like provenance
  fields on non-subagent forks do not turn a root into a spawned agent.

All three non-root classes are excluded in this implementation. Retaining the
typed class and parent ID makes the deferred behavior explicit and testable
without building partial task-tree support.

Keep the existing bounded-read, size, string, and absolute-cwd validation.

### 2. Keep future relationship metadata capture-local

Extend the existing bounded `SessionMetadata` result with:

```rust
rollout_class: CodexRolloutClass
parent_thread_id: Option<String>
```

Carry this typed value through discovery rather than re-reading or
re-interpreting the JSONL later.

Do not copy these fields into portable `ResourceDescriptor.metadata` or the
bundle manifest yet:

- Only roots are portable now, so a serialized `root` marker is redundant.
- Parent IDs belong only to descendants, and descendants are intentionally not
  uploaded.
- Publishing a partial task-tree contract would create a format that the
  future implementation would immediately need to replace.

When task-tree sync is implemented, promote the already-parsed typed fields
into one complete manifest design together with comparison and restore
semantics.

### 3. Reuse the classifier in local History

Replace the separate broad check in `chat_history.rs` with the shared
classifier.

History remains root-only, but capture and History now share one definition of
a root task. This prevents one surface from hiding a rollout that another
surface uploads.

### 4. Exclude subagents during provider discovery

Update Codex discovery in `provider_capture.rs`:

1. Read the shared `CodexRolloutMetadata`, including the retained relationship
   fields.
2. Continue only when `class == Root`.
3. Apply the existing project ownership and nested-project exclusion checks.
4. Deduplicate active and archived copies by root thread ID.
5. Create the conversation candidate and source file.
6. Add only the root ID to the filtered session-index ID set.

For every subagent class, do not create:

- A `ResourceCandidate`.
- A `DiscoveredResource`.
- A recipe entry.
- A captured file.
- A captured conversation.
- A session-index row.
- A user-facing warning.

Subagent exclusion is expected behavior, not an error. Genuine malformed root
metadata should retain the existing bounded warning behavior.

### 5. Add a capture-time fail-closed guard

Discovery is the primary filter, but capture must not rely solely on callers
using the newest discovery result.

When capturing a Codex conversation file:

- Carry forward its classified metadata from discovery.
- Reject it if it is not `Root`.
- Never accept a requested recipe entry whose current source is a subagent.

This prevents a stale in-memory recipe or manually constructed test request
from publishing an excluded rollout.

Because local state and buckets will be reset, do not add logic that searches
for or removes old subagent recipe entries or cloud resources.

### 6. Keep the portable model unchanged

This change affects capture eligibility, not the serialized bundle shape.

Do not add:

- Parent-thread fields to the manifest.
- A task-tree or member model.
- Hidden dependency resources.
- Descendant restore actions.
- A schema version created only for this filter.
- Compatibility decoding for previously captured subagents.

Fresh bundles will contain root conversations only because excluded rollouts
never reach the existing domain and publication layers.

The repository's separate schema-cutover work, if any, should remain separate
from this fix.

### 7. Keep Push and Pull root-only

The existing Push pipeline should naturally receive only root candidates:

- Auto-selection adds only roots.
- Thread comparison counts only roots.
- Push review shows only roots.
- Publication uploads only roots and their root session-index rows.

Fresh cloud bundles therefore make Pull root-only as well:

- Restore planning sees no subagent resources.
- Pull review shows no subagent rows.
- Restore materializes only roots.
- A completed root sync is not kept outstanding by an excluded descendant.

Do not add frontend filtering as a second implementation. Backend results must
already be correct.

### 8. Preserve the current UI

No design changes are required.

The existing root row continues to show:

- Its checkbox.
- Its title and metrics.
- Its existing status icon.
- Its existing Push or Pull action.

`Untitled thread` remains a valid fallback for a genuinely untitled root. It
must no longer be produced by Guardian or spawned-agent rollouts because those
rollouts never reach the UI.

## Deferred TODO: genuine `thread_spawn` portability

Do not upload or restore genuine spawned descendants in this implementation.

Record a code TODO next to the `ThreadSpawn` exclusion:

```rust
// TODO(codex-task-tree-sync): Preserve genuine thread_spawn descendants as
// hidden members of their root task after task-level comparison, review,
// restore, and crash-recovery semantics are designed together. Reuse the
// parsed rollout_class and parent_thread_id from CodexRolloutMetadata.
```

Future support must be implemented as one complete feature, not by making
spawned descendants independent resources again. It will require:

- A validated parent graph.
- One reviewable root-task resource containing hidden members.
- Task-level comparison and receipts.
- All-or-none Push and Pull selection.
- Multi-target restore and recovery.
- Per-member cwd remapping while preserving IDs and parent relationships.
- Tests proving Codex can inspect and continue restored descendants.

Until all of those pieces exist, every `thread_spawn` rollout remains local
only.

## Expected source changes

### Backend

- `src-tauri/src/project_sync_v3/codex_rollout.rs`
  - Add the shared bounded metadata parser, classifier, and validated optional
    parent ID.
- `src-tauri/src/project_sync_v3/mod.rs`
  - Register the internal module.
- `src-tauri/src/project_sync_v3/provider_capture.rs`
  - Use classified metadata and exclude every subagent before resource
    creation, selection, and session-index filtering.
- `src-tauri/src/project_sync_v3/chat_history.rs`
  - Reuse the shared classification rather than maintaining a separate check.

### Tests

- Provider-capture unit tests for classification and exclusion.
- Chat-history tests proving the shared root-only policy.
- Existing command integration tests updated to expect root-only resource and
  review counts.
- Frontend integration tests proving Push and Pull receive no subagent rows.

### Explicitly unchanged

- `src-tauri/src/project_sync_v3/domain.rs`
- `src-tauri/src/project_sync_v3/bundle_engine.rs`
- `src-tauri/src/project_sync_v3/conversation_materialization.rs`
- `src/types.ts`
- Push/Pull React presentation and CSS, except fixture expectations if needed

If implementation reveals that one of these files must change, stop and
re-evaluate rather than expanding the design implicitly.

## Tests

### Metadata classification

- A normal `session_meta` record is `Root`.
- An exact `thread_spawn` source is `ThreadSpawn` and retains its parent ID.
- Guardian, review, and compact sources are `InternalSubagent`.
- Unknown subagent shapes are `UnknownSubagent`.
- `thread_source == "subagent"` cannot fall back to `Root`.
- A child `id` is not replaced by its parent-valued `session_id`.
- A malformed or missing spawned parent ID cannot make the rollout a root.
- Root metadata does not retain a parent ID.
- Oversized, invalid, or non-absolute metadata retains bounded failure
  behavior.

### Provider capture

- Root plus Guardian discovers and captures only the root.
- Root plus genuine `thread_spawn` discovers and captures only the root.
- Root plus nested spawned descendants still captures only the root.
- A Guardian under a spawned descendant remains excluded.
- Root active/archive duplicates still resolve to one root resource.
- The filtered session index contains only discovered root IDs.
- A directly requested subagent resource is rejected at capture time.

### Push

- Root plus any number of subagents produces one review row and one count.
- No subagent resource ID enters the saved recipe.
- No subagent logical path or bytes enter the manifest or object store.
- A Push followed by another comparison reports the root as synced.

### Pull

- A fresh root-only bundle produces one Pull row.
- Pull restores the root transcript and its root session-index row.
- No subagent transcript is restored.
- A completed Pull does not offer the root again because of local subagent
  rollouts that remain outside Mallard sync.

### Frontend

- Push review has no Guardian or spawned-agent `Untitled thread` rows.
- Pull review has no Guardian or spawned-agent rows.
- Tab counts include roots only.
- No new text, badge, icon, or hierarchy is rendered.

All filesystem tests must use temporary Codex and Mallard homes. Never run them
against the real `~/.codex`, `~/.claude`, or `~/.mallard`.

## Verification

Run:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:integration
```

Complete one native Tauri smoke test with freshly wiped state:

1. Start with an empty bucket and clean `~/.mallard`.
2. Create one root task, one genuine spawned agent, and one Guardian rollout.
3. Confirm History and Push review show one root task.
4. Push and inspect the manifest: only the root transcript and root index row
   are present.
5. Pull into another fresh provider home.
6. Confirm Pull review shows one root task.
7. Confirm only the root is restored.
8. Confirm the next Push and Pull reviews treat the root as up to date.

## Acceptance criteria

- Only root Codex tasks can become portable resources.
- Every subagent source is excluded fail-closed.
- Genuine `thread_spawn` support exists only as an explicit TODO, backed by
  capture-local typed role and parent metadata.
- No subagent bytes reach local bundles or cloud storage.
- Push and Pull show roots only with the current UI.
- A successful root sync does not reappear because of excluded descendants.
- No migration or backward-compatibility code is introduced.
- All required backend, frontend, and native smoke checks pass.
