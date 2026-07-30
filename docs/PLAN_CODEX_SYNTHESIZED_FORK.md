# Plan: Replace the native-PTY fork executor with a synthesized fork

## Status

Proposed (2026-07-29). Not implemented; supersedes preference 2 of
[PLAN_CODEX_PORTABLE_PTY_FORK.md](PLAN_CODEX_PORTABLE_PTY_FORK.md).

- Parent design:
  [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
- Scope: preference 2 of the Codex-native fork coordinator only
- Compatibility: clean cutover; no migration, compatibility decoder, legacy
  executor, or feature flag
- Test assumption: the owner clears `~/.mallard` and the designated test
  cloud buckets before acceptance testing

The fork order becomes:

```text
1. app-server thread/fork over stdio      (unchanged)
2. synthesized fork written by Mallard    (replaces native PTY)
3. verified manual fork                   (unchanged)
```

Preference 1, the keep-both recommendation, cloud behavior, the common
verifier, naming policy, and recovery rules are unchanged.

## Why this replaces the PTY executor

Research on 2026-07-29 established that `codex fork` is not an independent
implementation. Upstream has one fork routine reached by three entry points:

```text
core/src/thread_manager.rs::fork_thread
  ← app-server/src/request_processors/thread_processor.rs::fork_thread_from_history
      ← tui/src/app_server_session.rs::fork_thread_at_with_presentation
```

The TUI issues `ClientRequest::ThreadFork` through an `AppServerClient` that
may be `AppServerClient::InProcess`. A PTY around `codex fork` therefore
reaches the same RPC as preference 1, through a terminal UI. It is not a
second failure domain, and it costs a ConPTY backend, a drain thread, `/exit`
escalation, process-tree cleanup, and a fake-CLI matrix on three platforms.

A synthesized fork is a genuinely independent domain: pure Rust, no child
process, no provider RPC, and it works when app-server cannot start at all.

## Verified constraints

Everything in this section was measured, not assumed.

### The native fork is a prepend

A real `thread/fork` was executed against a disposable `CODEX_HOME` holding a
single copied rollout (`019fa733…` → `019faf5f…`, `codex-cli 0.145.0`). The
resulting file is:

```text
fork = [ new session_meta ]  ++  [ every source record, re-serialized ]
```

- record-type sequence is identical apart from the one added `session_meta`;
- the byte growth equals the size of the prepended metadata record;
- no durable record is added, dropped, or reordered;
- records are re-serialized (JSON key order changes) and `turn_context` is
  re-rendered by the forking CLI's own sandbox/permission schema.

### Mallard's parser already expects this shape

`codex_rollout.rs` opens with: *"A forked rollout starts with metadata for the
active thread and then contains copied history from its ancestors. The first
`session_meta` is therefore authoritative."* The parser takes the first
`session_meta` as `current`, walks `forked_from_id` to build `fork_ancestry`,
and rejects self-parents, conflicting parents, cycles, depth beyond
`MAX_CODEX_FORK_DEPTH`, and metadata unreachable from the active thread.

### The fork is digest-identical under Mallard's codec

`analyze_codex_transcript` was run over both files:

| | source | native fork |
| --- | --- | --- |
| thread id | `019fa733…` | `019faf5f…` |
| chain length | 20 | 20 |
| lineage digest | `a2b7d3cee6977173…` | `a2b7d3cee6977173…` |
| semantic digest | `59e399e0d4c493e4…` | `59e399e0d4c493e4…` |

Identical lineage and semantic digests with a different provider thread id is
exactly the acceptance condition the coordinator already enforces. Message ids
are preserved verbatim by the native fork, which is why the digests match.

### The common verifier is satisfiable by construction

`verify_candidate` requires the candidate to differ in id, to carry
`forked_from_id == source`, to match the reviewed `local_lineage_digest` and
`local_semantic_digest`, to match the source's `history_record_digest` and
`history_record_count`, to be absent from the pre-existing child pins, and to
grow the child inventory by exactly one with all other children unchanged.

`non_metadata_record_digest` skips every `session_meta` record and
canonically re-serializes the rest. A file built by prepending one metadata
record to the source's non-metadata records satisfies the history digest and
count *by construction*, and the lineage and semantic digests are computed only
over durable message records, which are copied unchanged.

This is the central result: **no new trust model is required.** The
synthesized file is checked by the same verifier, on the same rescan path, as
an app-server or manual fork.

### The writing machinery already exists

| Need | Existing code |
| --- | --- |
| allocate an unused provider thread id | `generated_uuid_v4` + `occupied_provider_ids` loop (commands.rs) |
| choose the rollout path | `reserved_codex_transcript_path` |
| write the transcript atomically | materialization targets with `TargetExpectation::Absent`, staging, verify, rollback |
| write the provider index row | `ConversationFileRole::ProviderIndexFragment` + `codex_index_payload` + `validate_codex_index_fragment` |
| prove a rewrite preserved meaning | `rewrite_codex_transcript` round-trip pattern |
| ask Codex to adopt the file | `refresh_codex_inventory` → `thread/list {"useStateDbOnly": false}` |
| accept or reject the result | `exact_profile_delta_candidate` → `verify_candidate` |

The Pull path already synthesizes rollout files under new provider ids via
`write_new_file`. Creating the preserved local branch is the same operation
plus one metadata field.

### Index adoption works and is best-effort

Codex's user-visible thread list is `state_5.sqlite` (`threads` table). Its
`backfill_state` is `complete`, so it does not rescan continuously; on the
diagnosed machine the only unindexed rollouts were internal `guardian`
subagent sessions. A `thread/list` call with `useStateDbOnly: false` was
observed adopting a rollout that had merely been copied into the home — that
is the adoption mechanism, and `refresh_codex_inventory` already issues
exactly that call. The design already treats this refresh as best-effort
(a retryable warning, never a rollback).

## Outcome

When app-server fork is unavailable, Mallard writes the preserved local branch
itself as a new Codex thread, then verifies it with the existing common
verifier and requests a best-effort index refresh. The Pull transaction, the
keep-both result, and the recovery rules are unchanged.

## Scope

### In scope

- Add a `CodexForkExecutor::Synthesized` executor between app-server and
  manual.
- Add a `synthesize_codex_fork` function in `conversation_codec.rs`, beside
  `rewrite_codex_transcript`, that builds the fork bytes and proves them.
- Reuse the existing id allocation, reserved path, atomic write, index
  fragment, verifier, journal, and recovery policy.
- Reuse `refresh_codex_inventory` for adoption.
- Delete the native-PTY executor, `portable-pty`, and its tests.
- Rename executor terminology from `native_pty` to `synthesized`.

### Out of scope

- Changing preference 1 from app-server `thread/fork`.
- Writing Codex SQLite state directly.
- Editing or truncating the source transcript.
- Inventing metadata Codex would not have written (see the clone rule).
- Applying the fork name without app-server; naming stays `Deferred`.
- Changing portable bundle schema 1.

## Synthesized fork contract

### Inputs

The same reviewed inputs the other executors receive: reviewed `CODEX_HOME`
and project directory, source provider thread id, source rollout SHA-256,
reviewed `local_lineage_digest` and `local_semantic_digest`, the bounded
pre-existing direct-child pins, and the desired fork name. The Codex
executable pin is *not* required to write the file; it is still required for
the optional refresh and naming steps.

### Construction rule

1. Read the source transcript under the existing bounded read.
2. Parse with `analyze_codex_rollout_structure(bytes, Some(source_id))` and
   `analyze_codex_transcript`. Require `Complete` lineage and a structure
   whose `current.id` is the reviewed source.
3. Allocate a fresh provider thread id that collides with no occupied id.
4. Clone the source's **authoritative** `session_meta` payload (the first
   one), then override:
   - `id` and `session_id` → the new provider thread id;
   - `forked_from_id` → the source provider thread id;
   - `timestamp` → the operation's creation time;
   - remove `context_window` (it names the source's window id).
   Every other field is copied unchanged. Mallard does not synthesize
   environment-derived fields it did not read, and does not rewrite `cwd`:
   a preserve-local fork stays in the same project directory.
5. Emit the new metadata record as record 0, using the source record's own
   envelope shape (`{"timestamp", "type": "session_meta", "payload"}`).
6. Append every source record unchanged, including the source's own
   `session_meta`, which becomes the first ancestor entry.

The result is one added record and zero modified records — a strictly smaller
transformation than `rewrite_codex_transcript`, which rewrites identity fields
in place.

### Proof obligations before the bytes are staged

`synthesize_codex_fork` must reject its own output unless all hold:

- `analyze_codex_rollout_structure(new, Some(new_id))` succeeds;
- `structure.current.forked_from_id == Some(source_id)`;
- `structure.fork_ancestry` equals the source's ancestry with the source id
  prepended;
- `structure.structural_cwds` equals the source's `structural_cwds`;
- `analyze_codex_transcript(new)` is `Complete`;
- lineage and semantic digests equal the source's, and equal the reviewed
  `local_lineage_digest` / `local_semantic_digest`;
- `non_metadata_record_digest(new)` equals the source's digest and count;
- the new record count is exactly the source count plus one.

These are in-process checks on bytes Mallard holds. A failure is a definitive
no-fork before anything is written.

### Write and verify

1. Hold the coordinator's single-flight guard for
   `(CODEX_HOME, source-thread-id)`.
2. Revalidate the source rollout SHA-256, title expectation, idleness, and the
   complete pre-existing child inventory.
3. Reserve `reserved_codex_transcript_path`; require `TargetExpectation::Absent`.
4. Write the transcript and the `ProviderIndexFragment` row through the
   existing atomic multi-target transaction.
5. Run `exact_profile_delta_candidate` with the expected new thread id. The
   candidate must pass `verify_candidate` and the source must be unchanged.
6. Best-effort `refresh_codex_inventory`; a failure is a warning.
7. Naming is `CodexNameStatus::Deferred` unless app-server is reachable, in
   which case the existing short-lived naming request applies.

### Outcome classification

| Observation | Result |
| --- | --- |
| Source unreadable, not `Complete`, or identity mismatch | definitive no-fork; manual may follow |
| Any proof obligation fails before staging | definitive no-fork; manual may follow |
| Reserved path occupied or id collision unresolvable | definitive no-fork; manual may follow |
| Atomic write fails and rolls back; rescan shows no new child | definitive no-fork; manual may follow |
| Write succeeded and exactly one new child passes the verifier | verified fork |
| Write succeeded but the candidate fails verification | ambiguous; recovery required |
| More than one new child, or a pre-existing child changed | ambiguous; recovery required |
| Rollback state uncertain after a partial write | ambiguous; recovery required |

The no-duplicate rule is unchanged: manual is never offered after an outcome
that may already have created a fork.

## Honest risks

| Risk | Assessment and mitigation |
| --- | --- |
| Mallard owns the `session_meta` shape | Bounded by the clone rule: every field except four is copied from a record Codex itself wrote. Worst case is cosmetic metadata drift, never conversation content, because the durable records are byte-identical. |
| A cloned field is stale in the fork (`git`, `cli_version`) | Codex regenerates these; Mallard deliberately does not. A stale `git.commit_hash` is presentation metadata, not lineage. Decide explicitly whether to drop or clone; the plan clones for conservatism. |
| Codex Desktop refuses to resume a Mallard-written fork | The highest-value smoke test. Unverified today. If it fails, the executor is still safe — the file is content-identical and the source is untouched — but the value drops and preference 2 should become manual. |
| The thread stays invisible until adoption | Accepted and already designed for: refresh is best-effort, and the Pull result does not depend on Codex's index. |
| `session_index.jsonl` is legacy while SQLite is authoritative | Writing the fragment is harmless and already validated by `validate_codex_index_fragment`. Adoption is what makes the thread visible. |
| Codex changes the fork file layout upstream | Detected by the proof obligations and the common verifier, which fail closed. Re-verify the prepend shape when the pinned CLI's fork layout changes. |
| Concurrent Codex Desktop writes to the same home | Pre-existing risk shared with every executor; the baseline rescan and source-unchanged check already cover it. |

## Test plan

All tests use temporary provider and Mallard homes and must never touch a real
`~/.codex`, `~/.mallard`, or production bucket.

### Construction tests (pure, no filesystem)

- Prepend shape: output is source records plus exactly one leading
  `session_meta`.
- Digest preservation: lineage, semantic, history-record digest and count all
  equal the source's.
- Ancestry: `fork_ancestry` equals source ancestry with the source prepended;
  `forked_from_id` is the source.
- `structural_cwds` is unchanged.
- Rejection cases: source `Unknown` lineage; source id mismatch; a source whose
  first record is not `session_meta`; self-parent; an id that would collide;
  a source at `MAX_CODEX_FORK_DEPTH`.
- A synthesized fork of a synthesized fork is accepted and the ancestry chain
  grows by one.

### Coordinator tests

- Executor order is app-server, synthesized, manual.
- App-server success never synthesizes.
- Definitive app-server no-fork invokes the synthesizer.
- Ambiguous app-server result never invokes the synthesizer.
- Definitive synthesizer no-fork enters manual wait.
- A staged-then-rolled-back write is a definitive no-fork.
- A written-but-unverifiable fork is `RecoveryRequired`, never manual.
- Recovery after a persisted exact child never writes a second file.
- Changed source or changed pre-existing child makes the result ambiguous.
- Refresh failure downgrades to a warning and keeps the verified fork.

### Fixture round-trip

Synthesize a fork from a checked-in rollout fixture, then assert the file
passes `scan_native_threads` and `verify_candidate` unchanged.

### Real-Codex smoke tests (opt-in, disposable home only)

1. Synthesize a fork of a real thread in a disposable `CODEX_HOME`.
2. `thread/list {"useStateDbOnly": false}` adopts it and reports
   `forkedFromId` equal to the source.
3. `thread/read` returns the same history.
4. `codex resume <new-id>` opens it and can take a turn.
5. Compare a synthesized fork against a native `thread/fork` of the same
   source: assert equal lineage, semantic, and history-record digests, and
   record every `session_meta` field that differs. **This is the acceptance
   gate for the clone rule** — any field difference must be a deliberate,
   documented choice.
6. The source thread is byte-unchanged throughout.

### Handoff checks

```text
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:frontend-integration
npm run test:release-config
```

## Implementation order

1. Add `synthesize_codex_fork` plus its proof obligations and unit tests.
2. Add the fixture round-trip test through `scan_native_threads` and
   `verify_candidate`.
3. Add `CodexForkExecutor::Synthesized` and wire it into
   `run_executor_sequence` between app-server and manual.
4. Reuse id allocation, reserved path, atomic write, and index fragment.
5. Wire the post-write rescan, verification, and best-effort refresh.
6. Run the real-Codex smoke tests, especially the field-diff gate (5 above),
   and record the clone-rule decision in this document.
7. Delete `try_native_pty_fork`, the PTY session code, `portable-pty` from
   `Cargo.toml` and `Cargo.lock`, and the PTY tests.
8. Rename executor values and UI terminology to `synthesized`.
9. Update `RestorePlanView.tsx`, `src/types.ts`, fixtures, the parent design,
   and the readable design.

Do not leave a tree where both PTY and synthesized are production executors.

## Acceptance criteria

1. The ordered fork policy is app-server, synthesized, verified manual.
2. Preference 2 spawns no child process and requires no PTY, no ConPTY, and
   no Codex executable to produce the file.
3. Preference 2 works identically on macOS, Linux, and Windows with no
   platform-specific code.
4. A synthesized fork adds exactly one record and modifies none.
5. Lineage, semantic, and history-record digests are provably preserved, and
   the existing `verify_candidate` accepts the result unmodified.
6. The source transcript is never modified, staged over, or truncated.
7. A possibly successful synthesis never falls through to manual.
8. Index adoption failure is a warning, never a rollback.
9. `portable-pty`, `NativePty`, and `native_pty` have no production
   references.
10. No migration, compatibility alias, legacy executor, or feature flag
    remains.
11. Automated tests never touch real Mallard, Codex, or cloud state.

## Open questions

1. **Clone rule for environment-derived metadata.** Codex regenerates `git`,
   `cli_version`, `multi_agent_version`, and `thread_source` on the fork and
   clears `context_window`. This plan clones all but `context_window`. Smoke
   test 5 decides whether any of these must instead be regenerated or dropped.
2. **Naming without app-server.** `Deferred` is the proposed answer; confirm
   the UI states this plainly rather than implying the name was applied.
3. **Should this become preference 1?** It is more deterministic than the RPC,
   but app-server keeps Codex as the authority on its own format. Recommended
   answer: no — keep app-server first, and revisit only if smoke test 5 shows
   zero meaningful field divergence across several Codex versions.

## References

- Upstream fork implementation: `codex-rs/core/src/thread_manager.rs`,
  `codex-rs/app-server/src/request_processors/thread_processor.rs`,
  `codex-rs/tui/src/app_server_session.rs`
- App-server protocol, `thread/fork` and `thread/list`:
  <https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md>
- Mallard structural parser and codec: `codex_rollout.rs`,
  `conversation_codec.rs`
- Fork coordinator and common verifier: `codex_native_fork.rs`
- Superseded PTY plan:
  [PLAN_CODEX_PORTABLE_PTY_FORK.md](PLAN_CODEX_PORTABLE_PTY_FORK.md)
