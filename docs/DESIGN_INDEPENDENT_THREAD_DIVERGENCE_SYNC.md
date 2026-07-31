# Independent Per-Thread Divergence Sync

- Status: Implemented
- Extends: [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
- Cutover assumption: no deployed users; `~/.mallard` and the test cloud
  buckets are wiped before the next manual acceptance run. No migration,
  compatibility reader, or legacy fallback exists.

## Main promise

**One conversation's conflict never holds the rest of the project hostage,
in either direction.**

With local threads `{t1..t5}` where only `t2` and `t3` diverged from the
cloud:

- Pull may resolve any subset of `{t2, t3}` — both, one, or neither — in a
  single pass, and always applies every other approved thread and file
  action in that same pass.
- Push keeps `t2` and `t3` held (`Pull required`, not selectable, cloud
  versions retained byte-for-byte) while every other selected thread and
  resource still publishes.

## Pull

### Planning

Every thread is classified independently, exactly as before. Two per-thread
holds replace what used to be whole-review failures:

- A divergence whose fork contract cannot be built right now (no resolvable
  `codex` binary, active source turn, incomplete capture) downgrades that
  one thread to a provider-quiescence hold (`StoreOnly`, reason
  `provider_not_quiescent`) with the error in the row's status detail. The
  rest of the review is planned normally.
- A thread that still owns a journaled sync operation — for example a
  paused fork checkpoint from an earlier Pull — is review-blocked in both
  directions with the new reason code `resolution_pending` until that
  operation is verified or canceled. Planning never mints a second
  concurrent operation for the same thread, and the journal itself rejects
  one as defense in depth.

### Apply

The apply loop settles each thread independently, divergences first:

- A divergence that resolves inline (cloud-only, or a successful automated
  fork) commits and the loop continues.
- A divergence that pauses (manual fork needed, or recovery required)
  becomes one entry in `codex_fork_checkpoints` — and the loop continues.
  Nothing else is skipped: remaining conversations, file actions, the
  materialization record, and the reviewed recipe base all settle in the
  same pass.
- A per-thread failure (stale pin, unreadable transcript) becomes a
  `Failed` receipt for that action alone.

`RestoreResult.status` reuses the existing values: `complete` with an empty
checkpoint list, otherwise `awaiting_manual_fork` / `recovery_required`
derived from the pending checkpoints. The Codex App restart offer waits
until no checkpoints are pending; the final resume returns its own offer.

The restart itself is profile-aware. The default `~/.codex` profile
relaunches through LaunchServices exactly as before; any other profile
relaunches the validated app bundle's executable directly with
`CODEX_HOME` pinned to the pulled profile's home, because a LaunchServices
relaunch always comes up with the GUI session environment and would read
the wrong home. If the pinned relaunch cannot be proven running, the
result degrades to manual guidance carrying the exact
`CODEX_HOME=… <bundle executable>` command — the graceful quit has already
happened, and Mallard never force-kills or silently retargets the app.

### Checkpoint queue

Verify and cancel stay in the same review page. The checkpoint panel shows
the first pending fork ("Fork checkpoint 1 of N — the rest of this Pull is
already applied"), and every backend reply — apply, resume, cancel, and the
pending rehydration on reopen — carries the full remaining queue:

- Verifying a fork commits that thread and surfaces the next checkpoint.
- Canceling a fork rolls back only that thread (it stays divergent and is
  offered again on the next review) and surfaces the next checkpoint.
- After the last checkpoint resolves, the existing replan + finalize flow
  refreshes the review and advances the Pull baseline when nothing mutable
  remains.

Multiple pending operations per project are a supported state, listed
oldest first, never an error. Conflict cards in the review are labeled with
their conversation titles so several conflicts stay distinguishable.

## Push

- The old whole-plan gate is gone: a `RequirePull` or `Blocked` row no
  longer fails the publish. Approving such a row explicitly is the only
  hard error.
- Unapproved threads keep their previous cloud versions retained in the
  published bundle, so a divergent thread's cloud branch is never dropped
  or overwritten by pushing the rest.
- The push review surfaces held threads without disabling Push: a per-row
  "Review Pull" button on every `Pull required` conversation row, plus a
  non-blocking summary callout with the same affordance. The
  bundle-generation destination gate (base must equal head) is unchanged.

## Invariants kept

- Push never resolves a conflict and never publishes a divergent branch.
- Cloud-only resolution still requires explicit destructive confirmation;
  keep-both is still the recommended default.
- Fork execution still tries app-server → native PTY → manual, never
  retries past an uncertain attempt, and never adopts more than one child.
- A pending checkpoint still survives restarts and is re-offered through
  the same review; recovery leaves manual checkpoints untouched.
- One thread never carries two concurrent journal operations.

## Done when

- A Pull pass with divergences selected applies every non-divergent
  approved action even when every fork pauses at a checkpoint.
- Deselecting a divergence pulls the rest and advances the base; the
  conflict is offered again on the next review.
- Two paused forks in one project are verified or canceled one at a time
  from the same review page, and the baseline finalizes after the last.
- A push with a divergent thread present publishes the rest and retains
  the divergent thread's cloud version byte-for-byte.
- Approving a held push row is rejected; nothing else about the publish
  changes.
