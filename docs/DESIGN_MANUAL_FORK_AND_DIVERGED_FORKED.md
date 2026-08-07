# Design: Manual thread fork and the diverged-forked state

- Status: Implemented
- Last reviewed: August 1, 2026
- Scope: a per-thread fork action on the Activity list, the fork stamp
  recorded on the source thread's binding at every Mallard-driven fork
  adoption, the `DivergedForked` refinement of the `Diverged`
  classification, and the Pull-side recommendation flip it enables
- Compatibility: clean cutover; no migration or legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner
  will clear `~/.mallard` and the test cloud buckets before acceptance testing
- Related:
  - [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
    — owns the fork workload (executor ladder, verification, journal,
    checkpoints); this design adds a second entry point and one write, and
    changes nothing about how a fork is created or proven
  - [DESIGN_FORK_OBSERVATION_WATCHER.md](DESIGN_FORK_OBSERVATION_WATCHER.md)
    — supersedes the older manual-verification and profile-discovery recovery
    flow with event-located, exact-file acknowledgment and explicit retry
  - [DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md](DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md)
    — owns the classification decision table; the table is unchanged, and
    the refinement here runs strictly after its `Diverged` verdict
  - [DESIGN_PUSH_PULL_FLOW.md](DESIGN_PUSH_PULL_FLOW.md) — the shared plan
    phase this refinement joins, and the apply-time re-proof discipline it
    follows
  - [DESIGN_INDEPENDENT_THREAD_DIVERGENCE_SYNC.md](DESIGN_INDEPENDENT_THREAD_DIVERGENCE_SYNC.md)
    — per-thread journal operations, holds, and the checkpoint queue this
    design reuses for manual forks

## Motivation

Forking is currently reachable only from inside a Pull.
[build_divergence_plan](../src-tauri/src/project_sync/codex_native_fork.rs#L473)
is called only by `prebuild_thread_divergence_plan`
([commands.rs:5041](../src-tauri/src/project_sync/commands.rs#L5041)), which
runs only when `thread_review` is invoked with `build_pull_actions == true`
([commands.rs:7418](../src-tauri/src/project_sync/commands.rs#L7418), inside
pull planning); the transactional driver `apply_codex_divergence_locked` has
exactly one production caller, the pull apply loop
([commands.rs:7991](../src-tauri/src/project_sync/commands.rs#L7991)).

Two consequences:

1. A user who wants to branch a conversation — or to preserve a local
   branch *before* dealing with a divergence — must start a Pull review to
   get one.
2. When the user has already preserved the local branch in a fork, nothing
   witnesses that fact. The next Pull still recommends forking *again*
   (keep-both is the hardcoded default), and choosing the cloud version
   still demands the destructive confirmation, even though nothing would be
   destroyed.

This design adds the missing entry point and the missing witness.

## Decision

1. **Manual fork action (D1).** Every local thread row on the Activity list
   gains a fork icon button beside the existing open-in-Codex and
   open-in-terminal actions. It runs the *same* fork workload — plan build,
   journaled operation, executor ladder `app-server → native PTY → manual`,
   verification, adoption — with no restore step: the source thread is left
   byte-for-byte untouched, and the adopted fork becomes an ordinary
   local-only thread.
2. **Fork stamp (D2).** At fork adoption, the fork workload stamps the
   *source* thread's binding with the source chain's digest pair, chain
   length, and the adopted fork's identity. The stamp is written at the one
   existing binding-commit site for preserved forks, so both entry points —
   pull keep-both and the manual action — stamp identically. The stamp is
   replica-local metadata; it never enters the bundle manifest.
3. **`DivergedForked` refinement (D3).** A thread whose classification is
   `Diverged` and whose current digest pair equals its stamp — with the
   stamped fork still present locally and still carrying the stamped chain
   as a verified prefix — surfaces as `DivergedForked`: diverged, but the
   local branch is already preserved. The refinement is evaluated **only
   after** the decision table returns `Diverged`; every other relation
   performs zero stamp reads.
4. **Pull recommendation flip (D4).** For a `DivergedForked` thread, Pull
   review recommends **Use cloud version** (preselected) instead of
   keep-both, and the destructive confirmation is waived — the overwrite
   destroys nothing the stamp does not prove preserved, and apply re-proves
   the full witness before writing. Fork-again remains selectable. Plain
   `Diverged` behavior is unchanged, including the destructive ack.
5. **Plan-driven default (D5).** The recommended divergence resolution
   becomes a field of the divergence plan, computed server-side. The
   frontend's three hardcoded keep-both defaults are deleted; the review
   renders whatever the plan recommends. One rule, one place.
6. **Push unchanged (D6).** `DivergedForked` is `Diverged` to Push: the row
   stays `Pull required` and unselectable. Only the row copy improves — it
   may note the branch is already preserved.

The classification decision table, the `ConversationRelation` enum, the
executor ladder, the no-retry-past-uncertain rule, the
single-operation-per-thread journal invariant, and the pull-first Push gate
are all unchanged.

## Definitions

### Fork stamp

New optional field on `LocalConversationBinding`
([domain.rs:1475](../src-tauri/src/project_sync/domain.rs#L1475)), stored
with the other per-thread metadata in `conversation_bindings.json`:

```rust
pub struct ForkStamp {
    pub fork_thread_id: ThreadId,        // logical id minted at plan time
    pub fork_provider_thread_id: String, // Codex session uuid of the fork
    pub lineage_digest: String,          // source chain digests at fork time
    pub semantic_digest: String,
    pub message_count: u64,              // source chain length at fork time
    pub forked_at: u64,                  // display only, never a witness
}

// on LocalConversationBinding:
pub fork_stamp: Option<ForkStamp>,
```

- **Written** exactly once per adoption, inside
  `commit_preserved_fork_binding`
  ([conversation_materialization.rs:1171](../src-tauri/src/project_sync/conversation_materialization.rs#L1171)),
  in the same bindings mutation that binds the fork: the fork's new binding
  is pushed (as today, with `forked_from` pointing at the source), and the
  source binding's `fork_stamp` is set. The idempotence predicates
  (`exact_existing`,
  [conversation_materialization.rs:1194](../src-tauri/src/project_sync/conversation_materialization.rs#L1194),
  and `binding_commit_is_complete`,
  [conversation_materialization.rs:2399](../src-tauri/src/project_sync/conversation_materialization.rs#L2399))
  include the stamp, so crash-replayed commits converge.
- **Overwritten** by the next adoption for the same source: latest wins.
  Earlier forks remain ordinary local threads.
- **Never cleared.** The stamp is consulted only under `Diverged`, and its
  witness is digest-pair equality; content movement invalidates it
  automatically. No receipt, unlink, or pull touches it.
- The digest fields validate as sha256 hex in
  `LocalConversationBinding::validate`, exactly like the observed-digest
  pins beside them
  ([domain.rs:1539](../src-tauri/src/project_sync/domain.rs#L1539)).
- The stamp is **not** a pin. Manual fork moves no `last_observed_*` field
  on either binding; no sync happened.

### The `DivergedForked` witness

Evaluated in `thread_review`'s per-thread loop
([commands.rs:4691](../src-tauri/src/project_sync/commands.rs#L4691)),
strictly after `classify_thread_refresh`
([domain.rs:3190](../src-tauri/src/project_sync/domain.rs#L3190)) returns
`Diverged`, from data the walk already holds in memory:

| # | condition | source |
| - | --- | --- |
| 1 | classification is `Diverged` (table rows 8/10/11) | decision table, unchanged |
| 2 | the binding carries a `fork_stamp` | binding already loaded |
| 3 | source current digest pair `==` stamp pair (both components) | pair already computed by the walk |
| 4 | the stamped fork has a capturable local chain | the same walk |
| 5 | fork chain length `>=` stamp `message_count`, and `conversation_chain_digests(fork_chain[..message_count])` `==` stamp pair | one prefix digest over an already-parsed chain |

All five hold → the entry surfaces as `DivergedForked`. Any miss → plain
`Diverged`. Row 5 is not optional at classification time: Codex
edit-and-resubmit can rewrite a fork's early history, and a fork that no
longer carries the stamped branch must not produce an overwrite
recommendation that apply would only later fail closed.

Condition 5 reuses the row-7 prefix machinery
(`conversation_chain_digests`,
[domain.rs:3264](../src-tauri/src/project_sync/domain.rs#L3264)); it costs
one local hash, no IO, and is paid only in the rare `Diverged`-with-stamp
state. The ordering requirement doubles as the work bound: **states other
than `Diverged` never read the stamp.**

### Representation

- `ConversationRelation`
  ([domain.rs:2653](../src-tauri/src/project_sync/domain.rs#L2653)) is
  unchanged — directional policy (Push `RequirePull`, Pull
  `ResolveDivergence`) keys off `Diverged` exactly as today.
- `ThreadSyncState`
  ([commands.rs:426](../src-tauri/src/project_sync/commands.rs#L426), UI
  union at [types.ts:939](../src/types.ts#L939)) gains one variant:
  `DivergedForked` / `"diverged_forked"`, produced by `relation_sync_state`
  ([commands.rs:4319](../src-tauri/src/project_sync/commands.rs#L4319))
  when the refinement holds.
- `ThreadSyncEntry` ([types.ts:1002](../src/types.ts#L1002)) gains
  `fork_preservation?: { fork_thread_id, fork_title, forked_at }` so the
  row tooltip and the review cards can name the preserving fork.
- `ConversationDivergencePlan`
  ([domain.rs:2732](../src-tauri/src/project_sync/domain.rs#L2732)) gains
  `recommended_resolution: ConversationDivergenceResolution` and
  `local_branch_preserved: bool`. `available_resolutions` stays exactly
  `[PreserveLocalWithNativeFork, UseCloudOnly]`
  ([domain.rs:2753](../src-tauri/src/project_sync/domain.rs#L2753)) — the
  user can always fork again.
- `CodexDivergenceOperation`
  ([domain.rs:1749](../src-tauri/src/project_sync/domain.rs#L1749)) gains
  `origin: ForkOrigin` (`PullPreserve | Manual`). A `Manual` operation
  carries no restore targets; finish, resume, and recovery use the origin
  to know that no cloud restore, index payload, or baseline step follows
  fork adoption.
- As implemented, the journal's five cloud-commit fields
  (`storage_id`/`bundle_id`/`revision`/`commit_id`/`manifest_sha256`)
  grouped into `ConversationSyncOperation.cloud:
  Option<ConversationCloudAnchor>` — present on every push and pull
  operation, `None` only for `Manual` origin, which must not fabricate a
  commit it never touched. Adoption of a manual fork returns a dedicated
  `CodexDivergenceApplyOutcome::ManualForkCommitted` (no
  `ConversationApplyReceipt` exists to return), `CodexForkCheckpoint`
  carries `origin` with `plan_id`/`action_id` optional, and
  `RestoreResult.plan_id` is optional for manual-fork results. Pull
  surfaces filter checkpoints to `pull_preserve`; the Activity page owns
  `manual` ones.

## The manual fork flow

New Tauri command, mirroring the existing fork-related commands:

```text
fork_codex_thread(app, local_project_id, thread_id) -> RestoreResult-shaped
```

1. **Preconditions** — same contract the pull fork requires: capturable
   canonical chain, resolvable `codex` binary, provider quiescent, and no
   pending journal operation on the thread (single-operation invariant,
   reason `resolution_pending`). A binding is *not* required up front: a
   never-synced local thread has no binding row to carry the stamp, so the
   command mints its canonical binding first — the same deterministic
   logical id (`initial_conversation_thread_id`) classification already
   uses for unbound threads — making every local thread forkable and
   stampable. A failed precondition returns a typed error.
2. **Plan** — `build_divergence_plan` gains an origin/naming input. The
   pull call site keeps its `"<base> — local before Pull · <ts>"` name;
   manual passes the same builder `"<base> — fork · <ts>"`. Same
   sanitization, length limits, and fixed-across-retries rule; the logical
   fork id is minted here as today
   ([codex_native_fork.rs:514](../src-tauri/src/project_sync/codex_native_fork.rs#L514)).
3. **Execute** — the command journals a `Manual`-origin operation and runs
   `apply_codex_divergence_locked`
   ([conversation_materialization.rs:501](../src-tauri/src/project_sync/conversation_materialization.rs#L501))
   with an empty restore-target set. The executor ladder, attempt
   journaling and exact-file verification rules run unmodified.
4. **Pause** — an unacknowledged attempt yields the same
   `CodexForkCheckpoint` and surfaces through
   `get_pending_codex_divergence`. The only action is an explicit
   `retry_codex_divergence`; Mallard does not discover or adopt a possible
   fork from the prior attempt.
5. **Finish** — adoption commits the fork binding **and the stamp** in one
   mutation, records naming, refreshes the inventory, and stops. No cloud
   read or write, no pin movement, no Codex App restart offer. The source
   thread's bytes are untouched.
6. **Refresh** — the UI reloads history and comparison
   (`get_project_chat_history` force + `get_project_thread_sync_comparison`);
   the fork appears as a `local_only` thread, and the source flips to
   `diverged_forked` if it was diverged.

## Surfaces

### Activity row (browse mode)

- New `git-fork` glyph in the `Icons.tsx` union (stroke-based, 24-viewBox,
  strokeWidth 1.7, like its neighbors).
- Button rendered in `sync-history-thread-actions`
  ([ProjectChatHistoryPage.tsx:728](../src/components/project-sync/ProjectChatHistoryPage.tsx#L728))
  after Open in Terminal: `btn btn-ghost sync-history-launch-action`,
  `title="Fork in Codex"`, same `launchable` gate as its siblings
  ([ProjectChatHistoryPage.tsx:674](../src/components/project-sync/ProjectChatHistoryPage.tsx#L674)),
  busy state swaps to the spinning `refresh` icon exactly like Open in
  Codex. Hidden in selection mode; absent on `SyncEntryCard` (cloud-only
  rows have no local rollout to fork).
- A pending `Manual`-origin operation renders the existing fork-checkpoint
  panel (extracted from
  [RestorePlanView.tsx:856](../src/components/project-sync/RestorePlanView.tsx#L856)
  into a shared component) on the Activity page — Verify fork / Cancel,
  same commands. A pending `PullPreserve` operation keeps today's behavior:
  the Activity page only points back to the Pull review that owns it.

### Status indicator

`threadSyncPresentation`
([ProjectChatHistoryPage.tsx:260](../src/components/project-sync/ProjectChatHistoryPage.tsx#L260))
gains one row:

| state | icon | className | label |
| --- | --- | --- | --- |
| `diverged_forked` | `git-fork` | `diverged-forked`, `var(--blue)` | "Changed here and in {storage}; your local branch is already preserved in a fork. Pull to adopt the {storage} version." |

Blue, not yellow: the state has a recommended lossless action, and the
alarm color stays reserved for divergences that still need a human
decision. The toolbar aggregate keeps counting it inside the
`git-compare-arrows` review bucket — it still requires a Pull review pass —
while the row icon differentiates.

### Pull review

`DivergenceResolutionCards`
([RestorePlanView.tsx:209](../src/components/project-sync/RestorePlanView.tsx#L209))
renders from the plan instead of a hardcoded default:

- Preselection follows `plan.recommended_resolution`. The three hardcoded
  `preserve_local_with_native_fork` defaults
  ([RestorePlanView.tsx:453](../src/components/project-sync/RestorePlanView.tsx#L453),
  [RestorePlanView.tsx:488](../src/components/project-sync/RestorePlanView.tsx#L488),
  [pullReviewModel.ts:332](../src/components/project-sync/pullReviewModel.ts#L332))
  are deleted, per the no-parallel-paths policy.
- When `local_branch_preserved` is true, the **Use cloud version** card
  carries the *Recommended* emphasis and reads "Adopt the {storage}
  version — your local branch is already preserved in fork
  '{fork title}'."; the keep-both card relabels to "Fork again — keep
  another copy of the local branch first."; and the destructive-ack
  checkbox is not rendered for this card.
- When false, today's cards, default, and required ack are unchanged.

Server-side validation
([commands.rs:8851](../src-tauri/src/project_sync/commands.rs#L8851))
changes in one arm: `UseCloudOnly` requires `destructive_approved` **iff**
the plan's `local_branch_preserved` is false. Keep-both continues to
reject a set flag.

### Pull apply re-proof

The overwrite arm for a `local_branch_preserved` plan re-proves the full
witness at apply time, after the existing 5b drift check (source pair must
equal the pair the plan classified from) and before writing:

1. source current pair `==` stamp pair;
2. the stamped fork's rollout resolves, parses, and
   `digests(fork_chain[..stamp.message_count])` `==` stamp pair.

Any miss fails that item closed — a `Failed` receipt, nothing written, the
rest of the pass unaffected; the next review shows the thread as plain
`Diverged` (the witness that failed is the same one classification checks).
The pre-write local backup of the replaced source is kept unconditionally,
exactly as today.

### Push review

No behavioral change. A `DivergedForked` row classifies `RequirePull`,
stays unselectable, keeps the per-row "Review Pull" action, and the rest of
the push publishes around it. Row copy may append "— local branch already
preserved".

## Soundness

- **The stamp is written only after verification.** Adoption requires the
  fork to be a distinct Codex thread, parented to the source, containing
  the exact reviewed chain (`verify_source_unchanged` and the
  `non_metadata_record_digest` body proof,
  [codex_native_fork.rs:670](../src-tauri/src/project_sync/codex_native_fork.rs#L670)).
  The stamp therefore records a proven preservation, not an intent.
- **Digest equality is the witness, both times.** Classification and apply
  both demand `source pair == stamp pair` *and* the fork prefix
  reproduction. A source that moved after forking, a deleted fork, and a
  fork whose early history was rewritten each break a condition and demote
  the thread to plain `Diverged` — recommendation and waiver both vanish.
- **The waiver never trusts the plan.** Review output stays advisory; the
  apply-time re-proof is the authority, per the flow design's review-gap
  rule. A stale witness fails the item closed; it cannot cause loss.
- **False positives require a digest collision.** Rollouts move by append
  or rewrite; re-reaching a stamped pair after moving past it means
  reproducing both sha256 streams. The pair is already trusted for
  fast-forward safety elsewhere; this adds no stronger assumption.
- **Zero cost outside divergence.** The stamp is read only after a
  `Diverged` verdict (the user-facing ordering requirement); `Identical`,
  prefix, missing, and unknown states are untouched, and threads without
  stamps add one `Option` check.
- **Replica-locality is correct, not a shortcut.** The fork exists only on
  this machine until pushed; another replica has no witness that the
  branch is preserved, so the stamp must not influence its classification.
  Keeping it out of the manifest makes that structural. `BUNDLE_SCHEMA`
  stays 1; the manifest `ConversationThread` is untouched.

## Failure handling

- **Fork attempt failures** — identical to today: the ladder never retries
  past an uncertain attempt, pauses become checkpoints, recovery inspects
  before adopting, duplicate adoption is impossible, and cancel rolls back
  only that thread. A canceled or failed manual fork writes no stamp.
- **Crash between fork adoption and stamp** — impossible to observe: the
  fork binding and the stamp commit in one bindings mutation, and the
  journaled operation replays that commit idempotently.
- **Stamped fork later deleted or rewritten by the user** — condition 4/5
  fails; the thread is plain `Diverged`; nothing recommends overwrite.
- **Manual fork requested while a review is open** — the journal's
  single-operation rule and the existing `resolution_pending` block hold;
  an applied manual fork that changes local state trips the standing
  stale-plan tripwires (apply-time drift checks fail closed).
- **Precondition failures** (unbound thread, active turn, no codex binary)
  — typed error surfaced as a toast; where known statically the button is
  disabled with the reason as tooltip.

## Work budget

| path | added cost |
| --- | --- |
| refresh / push review / pull planning, non-diverged thread | zero — stamp never read |
| diverged thread, no stamp | one `Option` check |
| diverged thread with stamp | two digest-pair comparisons + one local prefix hash over an already-parsed chain; no IO |
| pull apply, preserved overwrite | one local rollout read + parse + prefix hash (the re-proof); zero added cloud reads |
| manual fork | the same work a pull keep-both fork performs today, minus the restore step |

No new cloud reads or writes anywhere; the manifest and its 2-GET plan
phase are untouched.

## Cutover

1. Domain: `ForkStamp` + `LocalConversationBinding.fork_stamp` +
   validation; `ThreadSyncState::DivergedForked`; entry
   `fork_preservation`; plan `recommended_resolution` +
   `local_branch_preserved`; operation `origin: ForkOrigin`. Mirror in
   `types.ts`.
2. Fork workload: origin-parameterized naming in `build_divergence_plan`;
   stamp write + idempotence in `commit_preserved_fork_binding`;
   origin-aware `finish_codex_divergence`; `fork_codex_thread` command
   routing through `apply_codex_divergence_locked` with no targets;
   `resume`/`cancel` unchanged.
3. Classification: the five-condition refinement in `thread_review` after
   the `Diverged` verdict; `relation_sync_state` and
   `count_thread_sync_state` extended.
4. Pull: compute recommendation + witness in
   `prebuild_thread_divergence_plan`; conditional destructive-ack
   validation in `split_approved_restore_actions`; apply-time re-proof in
   the overwrite arm.
5. Frontend: `git-fork` icon; ThreadCard action + busy/disabled states;
   `threadSyncPresentation` row + CSS class; plan-driven card defaults and
   copy; shared checkpoint panel on the Activity page; `api.ts` binding
   for `fork_codex_thread`.
6. Delete the three hardcoded keep-both defaults; acceptance grep: no
   `preserve_local_with_native_fork` literal outside the union definition
   and the plan-driven mapping.

No migration. Absent `fork_stamp` deserializes as `None`; the first
Mallard fork after cutover writes the first stamp.

## Test plan

Backend (`sync_tests` harness style; work counts via `CountingStore`):

- Stamp lifecycle: written on pull keep-both adoption and on manual
  adoption (one write site proven by both entry points); overwritten by a
  newer adoption; idempotent replay converges; canceled fork writes none.
- Refinement fixtures: each `Diverged` table row (8/10/11) × stamp match →
  `DivergedForked`; each broken condition (pair mismatch, no stamp, fork
  missing, fork too short, fork prefix rewritten) → `Diverged`.
- Ordering tripwire: a poisoned stamp accessor asserts the stamp is never
  read for non-`Diverged` relations.
- Manual fork command: adoption leaves the source byte-identical, binds
  the fork `local_only` with `forked_from`, stamps the source; ladder
  pause → checkpoint → resume verifies / cancel rolls back; second
  operation on the same thread rejected; provider-not-quiescent returns
  the typed hold.
- Pull: `recommended_resolution` is `UseCloudOnly` iff the witness holds;
  `UseCloudOnly` without ack validates iff `local_branch_preserved`;
  plain-`Diverged` ack requirement regression; apply re-proof failure
  (fork deleted / fork rewritten / source moved) → `Failed` receipt,
  nothing written, siblings land, replan shows `Diverged`; successful
  preserved overwrite advances the pin and leaves the stamp inert;
  fork-again on a `DivergedForked` thread restamps and restores normally.
- Push regression: `DivergedForked` stays `RequirePull`, unselectable,
  publish-around preserved.
- Work counts: zero added cloud reads in every direction; zero stamp reads
  on a refresh whose threads are all `Identical`.

Frontend (integration suite):

- Fork button: rendered browse-only on local rows, gated, busy spinner,
  invoke called, history + comparison refreshed after completion.
- `diverged_forked` indicator icon, class, tooltip; review badge; toolbar
  bucket count includes it.
- Divergence cards: preserved plan preselects cloud version with
  *Recommended* on that card and no ack checkbox; fork-again selectable;
  unpreserved plan renders today's cards and required ack.
- Checkpoint panel appears on the Activity page for a pending manual fork
  and resolves through verify/cancel.

## Considered and deferred

- **Keeping the destructive ack for preserved overwrites** — rejected: the
  ack exists to mark loss, and the witnessed overwrite loses nothing;
  requiring it for the recommended action would train users to click
  through real destructive confirmations. The apply-time re-proof, not the
  ack, is the safety boundary. If review feedback disagrees, the waiver is
  one validation arm to revert.
- **A new `ConversationRelation` variant** — rejected: directional policy
  is identical to `Diverged` everywhere except the pull recommendation,
  which is plan data. A variant would touch every relation match for zero
  policy difference.
- **Clearing stamps on pull receipts** — unnecessary: the witness is
  digest equality, so stale stamps are inert; receipt code stays simpler.
- **Stamp in the cloud manifest** — rejected: the preserving fork exists
  only on this replica until pushed, so no other replica may act on it;
  see *Soundness*. Revisit only if forks ever push automatically.
- **Stamp history (multiple stamps per thread)** — latest-wins suffices;
  earlier forks remain ordinary threads and need no bookkeeping.
- **Fork action on cloud-only rows** — out of scope: nothing local to
  fork; Pull materializes first.
- **"Forked from" lineage chip on thread rows** — `ThreadSyncEntry`
  already carries `forked_from` unused by the UI; a small follow-up could
  render it on fork rows. Display-only; not part of this cutover.
