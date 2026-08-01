# Design: Push/pull operation flow

- Status: Implemented
- Last reviewed: July 31, 2026
- Scope: end-to-end Push and Pull operation sequence in `ProjectSync` —
  plan, review, workload, commit
- Compatibility: clean cutover; no migration or legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner
  will clear `~/.mallard` and the test cloud buckets before acceptance testing
- Related:
  - [DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md](DESIGN_MANIFEST_ONLY_THREAD_REFRESH.md)
    — owns the classification decision table this flow runs in plan step 3
  - [DESIGN_FAST_PUSH_PULL.md](DESIGN_FAST_PUSH_PULL.md) — owns the cost
    machinery under plan step 2 and the workloads (derivation store, capture
    session, object cache, batch constants); its D1 and D4 are absorbed here
    as flow steps
  - [DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md](DESIGN_PULL_FIRST_CODEX_DIVERGENCE_RESOLUTION.md)
    — owns the fork workload that pull step 5b routes divergent items into
  - [DESIGN_REMOTE_HISTORY_CONTINUITY.md](DESIGN_REMOTE_HISTORY_CONTINUITY.md)
    — extends plan step 1 with repository-level base-continuity
    classification (replaced-history detection)

## Decision

Push and Pull share one plan phase and differ only in workload and commit.

There is no repository lock in either direction. Readers get snapshot
isolation from immutability; writers get atomicity from a single
compare-and-swap on `head`. Classification is advisory; every destructive
step re-proves its own safety at apply time.

### Plan (shared by both directions, and identical to refresh)

| # | step | cloud reads |
| - | --- | --- |
| 1 | Resolve `head` and record its etag and commit ID (`inspect_optional_with_token`); fetch the manifest it names; verify against `head.manifest_sha256` | 2 GETs |
| 2 | Walk local transcripts once; compute each chat's canonical chain length and digest pair | 0 |
| 3 | Classify every chat with the manifest-only decision table from its three inputs: manifest per-thread metadata, local digest pair, binding pin | 0 |

Classification reads no conversation payloads, in either direction. The
pin is not optional input: local-ahead is structurally provable without it
(hash `local[..cloud_len]`, compare to the manifest pair), but cloud-ahead
is not — the manifest carries no prefix digests, so the pin is the only
witness that a fast-forward pull loses nothing (refresh table rows 9/10).

One deliberate exception, implemented with the flow: Pull planning opens
the transcripts of threads it could offer to materialize — cloud-present
with no captured local copy — through one batched prefetch, because the
stored-subagent refusal must see content before a thread reaches review
(pinned by
`previously_stored_subagent_tasks_are_hidden_from_pull_but_not_refresh`,
and by `planning_reads_cloud_payloads_only_for_offerable_pull_threads`).
Refresh and Push review read zero payloads.

### Pull

| # | step |
| - | --- |
| 4 | User reviews and selects items; each `Diverged` selection requires an explicit fork-or-overwrite choice |
| 5a | Batched prefetch of payload objects for the selected set only; every object is verified against its manifest sha256 on receipt |
| 5b | Per item: recompute the local pair and require it to equal the pair the plan classified from — drift fails the item closed; back up any local state being replaced; then write, or run the fork workload |
| 6 | Per-item pull receipts advance the binding pins |
| 7 | Done. Nothing to release |

### Push

| # | step |
| - | --- |
| 4 | User reviews and selects items; `Diverged` items are not selectable — the review offers the jump into the Pull flow, reusing this plan's classification for display; any *applied* pull invalidates the plan, which re-runs before push continues |
| 5 | Fresh capture of the selected items at apply time; per item, re-prove the push witness (local prefix pair equals the cloud pair, or cloud entry absent) — drift fails the item closed |
| 6 | Batched upload of new objects, each PUT carrying `x-amz-checksum-sha256`; the acknowledged PUT is the verification receipt; nothing is downloaded back |
| 7 | Write a new immutable manifest carrying the required metadata trio for every thread; swing `head` by CAS against the step-1 etag |
| 8 | Per-item push receipts advance the binding pins. Done. Nothing to release |

On CAS failure in step 7 nothing is lost: uploaded objects are immutable
and content-addressed, so the re-plan (steps 1–3, cheap) reuses them.

## No optional paths

Clean cutover includes the no-parallel-paths policy: a rule either always
runs or never exists. There are no disabled-in-v1 code paths. Applied to
the sketch that seeded this design:

| sketch annotation | resolution |
| --- | --- |
| pull: "reverify the hash of downloaded data (Optionally)" | **always** — sha verification on read is what makes lock-free reads and caches sound; it hashes bytes already in memory |
| push: "redownload uploaded object to check hash (very optional, disable in v1)" | **never** — the checksum-carrying PUT is already a server-verified receipt; durability audits use the explicit `verify_bundle_objects` command |
| push: "acquire cloud repo locking (maybe upload some file …)" | **never** — head CAS is the only write-side concurrency control; see *Soundness* |

## Goals

- One plan implementation shared by refresh, push review, and pull
  planning; payload reads happen only in the pull workload, only for
  selected items.
- Pull runs concurrently with any push, always sees a consistent commit,
  and never waits.
- Push commits atomically via head CAS; concurrent pushers converge by
  re-planning, never by corrupting or blocking each other.
- Review output is advisory; every destructive write re-proves its safety
  at apply time on both sides of the gap.
- One chat's failure never fails the operation.

## Non-goals

- Change the classification table, its inputs, or its conservatism — owned
  by the refresh design.
- Change fork semantics or divergence policy — owned by the pull-first
  divergence design.
- Per-chat historical version selection (see *Considered and deferred*).
- Merge conversation content.
- Support pre-cutover local or cloud state.

## Soundness

### Readers need no lock

A pull resolves `head → manifest (verified against head.manifest_sha256) →
objects (each verified against its manifest sha256)`. Every name below
`head` is immutable and content-addressed, so the snapshot is fully
determined by the single `head` read: a pull concurrent with any push
observes either the old commit or the new one, never a torn mixture. The
one hazard is an object pruned out from under a slow pull; the verified
read fails closed and the pull re-plans from the new head.

Therefore the sketch's pull step 1 — "don't proceed if others are pushing"
— is deleted, not deferred. It blocks users for zero safety gain and makes
pull availability depend on writer liveness.

### Writers need CAS, and a lock cannot replace it

A crashed lock holder must not brick the repository, so any real lock
needs a lease TTL; a TTL needs clocks; clocks are not correctness
witnesses in this system. After a lease expires, two writers can both
believe they hold the lock — so the head CAS is still required for
correctness. The lock therefore adds a liveness hazard (a stale lock
blocks every writer) while removing nothing. Concurrent pushers are
already safe without it: object uploads are idempotent
(content-addressed; `put_immutable_async` resolves collisions by
read-and-compare), and the CAS picks exactly one winner; the loser
re-plans against the winner's manifest and reuses its own uploads.

A "someone else is pushing" indicator may exist later as an advisory UX
marker. It never gates any operation.

### The review gap

Review is human-time, and Codex keeps writing during it. Classification
computed in plan step 3 is therefore advisory only:

- **Push** re-captures at apply (step 5) and re-proves the fast-forward
  witness per item; cloud movement is caught by the CAS (step 7).
- **Pull** recomputes the local pair per item (step 5b) before writing;
  replaced local state is backed up first.
- A stale plan can misdirect *selection*; it can never cause loss, because
  no destructive write trusts it.

## Failure handling

- **Per-item isolation** — a failed download, verification, fork, or
  witness re-proof fails that item; the remaining items proceed.
- **CAS loss (push step 7)** — re-run steps 1–3 and re-review what
  changed; uploaded objects are reused. No cleanup required.
- **Crash after uploads, before CAS** — `head` still names the old commit;
  the repository is unchanged; orphaned objects are inert and reusable by
  the next push.
- **Crash after CAS, before receipts** — the cloud commit is durable but
  pins are stale. While local is unmoved the thread classifies `Identical`
  (row 6); if cloud then moves first, the stale pin classifies row 10
  `Diverged` — conservative review, never silent loss — and the resolving
  pull rewrites the pin.
- **Verification failure on a pulled object** — the item fails closed;
  nothing is written locally for it.

## What the cutover deletes

Per the no-parallel-paths policy these are removals, not fallbacks:

- Planning-time payload reads: the per-thread cloud transcript loop in
  `thread_review` planning modes (`commands.rs:4450`). The manifest-only
  branch becomes the only classifier for refresh, push review, and pull
  planning.
- The retained-object re-download loop in publish
  (`bundle_engine.rs:1094`).
- The post-upload `get_many` verification pass (`bundle_engine.rs:1213`),
  replaced by `x-amz-checksum-sha256` on `conditional_put_async` /
  `put_immutable_async`.
- The duplicate discover+capture inside each phase. One walk per phase
  remains — plan walks once, apply walks once — because apply's walk *is*
  the freshness proof.
- No lock code is introduced anywhere.

## Cutover

1. Land the shared plan phase (steps 1–3) as the single planning
   implementation behind refresh, `prepare_push_review_with_repository`,
   and `plan_bundle_restore_with_progress`; thread the step-1 etag through
   to the commit site.
2. Pull workload: batched prefetch over the selected set; mandatory
   verify-on-receipt; per-item local re-check, backup, write/fork;
   receipts.
3. Push workload: apply-time capture and witness re-proof in
   `push_bundle_reviewed_with_repository`; checksum-carrying PUTs; new
   immutable manifest + head CAS.
4. Wire plan invalidation: an applied pull invalidates any outstanding
   push plan.
5. Delete the four items above; acceptance grep: no `get()` loop over a
   known object set outside the workload prefetch.

No migration. First post-cutover push writes complete metadata; first sync
per thread writes the pin.

## Test plan

Fixtures follow the existing `sync_tests` harness style; work counts use
`CountingStore`.

- **Snapshot isolation** — a pull interleaved at every step of a
  concurrent push observes the old commit or the new one, never a mixture.
- **CAS race** — two pushers: exactly one wins; the loser's re-plan
  succeeds and re-uses its uploaded objects (no duplicate PUT of the same
  sha).
- **No waiting** — a pull proceeds while a push is mid-upload; no lock or
  wait primitive exists to call.
- **Work counts** — push planning issues zero payload GETs; publish issues
  zero GETs after its PUT phase; pull fetches exactly the selected
  actionable set.
- **Review-gap proofs** — local mutated after review: the push item and
  the pull item each fail closed, the pull backup intact; cloud moved
  after plan: CAS fails and nothing commits.
- **Checksum PUT** — a store that rejects a mismatched
  `x-amz-checksum-sha256` fails the push before any manifest write.
- **Verify-on-read** — one corrupted object fails its item; sibling items
  land.
- **Pin at crash checkpoints** — crash between CAS and receipts yields
  conservative `Diverged` once cloud moves, repaired by the next pull
  (extends the refresh design's pin lifecycle tests).
- **Divergence routing** — `Diverged` is unselectable in push review; the
  jump to pull reuses the plan for display; an applied pull invalidates
  the push plan (stale-plan tripwire).
- The refresh design's loss-scenario regressions stay green unchanged;
  this flow adds no new classification logic for them to cover.

## Considered and deferred

- **Repository lock** (sketch push steps 1/10, pull step 1): rejected on
  the TTL/clock argument above. At most an advisory marker later, UX only.
- **Per-chat target version** ("target version (default latest)"):
  deferred. v1 pulls only from the head manifest. If added later, a
  version is `(generation, commit_id)` of a retained immutable manifest,
  enumerated from `_manifests/`; per-chat mixing is safe because chats are
  independent, but nothing today indexes chat → historical manifests.
- **Post-upload read-back** (sketch push step 8): rejected outright —
  superseded by the server-verified PUT receipt plus the explicit
  `verify_bundle_objects` audit command.
- **Per-chat manifest objects**: rejected. A single bundle manifest keeps
  planning at 2 GETs total and keeps multi-chat commits atomic under one
  head swing.
