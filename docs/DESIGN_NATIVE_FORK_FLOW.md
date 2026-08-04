# Design: Native fork flow

- Status: Implemented
- Last reviewed: August 2, 2026
- Compatibility: clean cutover; no migration or legacy execution path
- Scope: the one-page flow of a Codex-native fork — Pull divergence
  preservation, the waived-confirmation overwrite it later enables, and the
  manual fork entry point. The related designs below own the details and
  remain authoritative.
- Related:
  - [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
    — owns the fork workload: executor ladder, verification, journal,
    checkpoints
  - [DESIGN_FORK_OBSERVATION_WATCHER.md](DESIGN_FORK_OBSERVATION_WATCHER.md)
    — owns event-located acknowledgment, unacknowledged semantics, and
    startup recovery
  - [DESIGN_MANUAL_FORK_AND_DIVERGED_FORKED.md](DESIGN_MANUAL_FORK_AND_DIVERGED_FORKED.md)
    — owns the manual entry point, the fork stamp, and the `DivergedForked`
    refinement
  - [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md) — the Pull flow
    this fork loop plugs into at its divergence step

## Flow

The rollout file is the authority — executors only ask Codex for a child;
nothing counts until the file proves it.

1. Plan at review: pin Codex binary (path + sha), source rollout (exact
   path + sha, never rediscovered), local hash pair + length, title;
   precompute the fork name; executors: app-server, then PTY. If a prior
   fork stamp still witnesses the local branch (`DivergedForked`), recommend
   overwrite and waive the destructive confirmation; otherwise recommend
   fork
2. Preflight: re-prove every pin — any mismatch ⇒ abort, nothing invoked,
   nothing staged
3. Journal + stage the cloud replacement (verified backup); local drifted ⇒
   fail this item only; the original rollout is untouched until step 6
4. Ask for the child, executors in order, each attempt journaled before and
   after invocation
   1. app-server: source idle + title unchanged ⇒ thread/fork; the response
      must name the child id + exact rollout path
   2. PTY fallback: start a recursive filesystem watcher
      (`ForkRolloutWatcher`, notify-based) on the Codex sessions tree, then
      spawn `codex fork` hidden — watcher first, so the new rollout cannot
      appear unobserved; a definite create/rename event for a
      rollout-shaped path only locates the candidate, the file read is the
      proof (bounded reads, candidates, retries, stability window); watcher
      loss or queue overflow after launch ⇒ unacknowledged
   3. definitive no-fork ⇒ next executor; unacknowledged (a fork may
      exist) ⇒ stop and park — never invoke onto an unknown
5. Verify the candidate from its file: forked-from == source and id ≠
   source; hash pair == pinned local pair; non-metadata records
   byte-identical to source (only what Codex rewrites when forking may
   differ); cwd inside the reviewed project; source still unchanged
6. Commit only after Verified: bind the preserved thread and write the fork
   stamp on the source binding (hash pair, length, fork id + path, time —
   the witness `DivergedForked` reads later), write cloud over the
   original, receipt → advance the pin
7. After commit, async best-effort: rename fork and original, refresh Codex
   inventory — may fail without failing the pull
8. Unacknowledged ⇒ parked with nothing replaced; user retry restarts at
   that executor; a durable `Pending` found at restart flips straight to
   `Unacknowledged` (never scanned for); a crash before `Pending` was
   durable rolls staging back
9. Done — the original holds cloud, the local branch lives on as a real
   Codex child thread

Overwrite (UseCloudOnly): skip 4–5, still require source idle + unchanged
title; if the confirmation was waived by a fork stamp, re-prove the stamp
and the preserved prefix inside the stamped fork immediately before
replacement — any miss fails this one item closed and the thread re-offers
as a plain divergence on the next review.

Manual fork: steps 1–5 plus the binding, stamp, and naming — no cloud
replace, no cloud anchor (forbidden for manual origin).

## Code anchors

All in `src-tauri/src/project_sync/`:

- Plan and pins: `codex_native_fork::build_divergence_plan`
- Preflight and per-attempt re-validation:
  `codex_native_fork::validate_execution_artifacts_and_source`
- Stage-before-fork ordering and phase machine:
  `conversation_materialization::apply_codex_divergence_locked`,
  `finish_codex_divergence`
- Executor ladder and attempt journaling:
  `codex_native_fork::run_executor_sequence`, `try_app_server_fork`,
  `try_native_pty_fork`, `ForkRolloutWatcher`
- Candidate proof: `codex_native_fork::verify_candidate_against_source`,
  tolerated rewrites pinned by `FORK_VOLATILE_RECORD_TYPES`
- Stamp write: `conversation_materialization::commit_preserved_fork_binding`
- Waived-overwrite re-proof: `commands::reprove_fork_preservation`,
  `codex_native_fork::verify_native_thread_prefix_at_path`
- Retry: `conversation_materialization::retry_codex_divergence_locked`
- Post-commit presentation:
  `codex_native_fork::complete_codex_post_commit_presentation`
