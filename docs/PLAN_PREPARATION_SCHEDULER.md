**Preparation scheduler plan**

Status: implemented on 2026-09-09 (uncommitted), proposed the same day;
deltas from the text below are in **Implementation notes** at the end.
Follows finding 2 of `docs/PUSH_SLOWNESS_2026-09-09.md` (§4.4, §4.5). The
numbers below come from
that report, from an offline schedule simulation over its per-thread release
costs, and from peak-memory runs of the same probe on the same 41 transcripts.

Replace the scheduler behind the Push build (`build_publish_request`) and the
Pull prepare stage (`apply_bundle_restore_with_progress`) so a batch of
conversations runs at the worker count instead of at 1.5×. Keep all persisted
schemas at **1**. Nothing on disk, in the manifest, or on the wire changes: the
scheduler decides only when each thread's builder starts and when its result
reaches the serial consumer. Clean cutover: one scheduler, one budget constant,
no flag, no legacy path. Resetting `~/.mallard` and the cloud test data is not
required by this change; nothing persisted depends on it.

**The problem, measured.**

`ordered_preparation::for_each_prepared` admits a job while
`admitted.len() < workers` and the reserved bytes fit a 64 MiB budget, then
blocks until job `next` is ready and consumes in index order. `admitted`
keeps finished-but-unconsumed jobs, so every result behind a long
head-of-line job holds a worker slot and its bytes until that job finishes.
Jobs arrive in thread-id order, which is random with respect to size. The
report's reproduction (real builders, real scheduler, release, 11 cores):
41.0 s wall for 60.5 s of work on 4 workers; the app's own timeline shows the
phase ending on the most expensive job because it was admitted last.

An event simulation over the report's per-thread release costs reproduces the
measured points and lets each rule be priced separately (4 workers unless
stated; "src" = leaf plus ancestors):

| Policy | Wall, detector as shipped | Wall, detector fixed (finding 1) | Peak reserved |
| --- | ---: | ---: | ---: |
| shipped (in-order, slots held by finished results, 64 MiB, thread order) | 42.7 s (measured 41.0) | 8.1 s | 70 MB |
| shipped slots, no budget | 37.2 s (measured 37.7) | 6.2 s | 117 MB |
| shipped slots, no budget, largest-first | 19.8 s (measured 21.4) | 3.2 s | 246 MB |
| shipped slots, no budget, largest-first, 8 workers | 14.0 s (measured 16.1) | 2.2 s | 401 MB |
| running-only slots, completion-order consumption, thread order, no budget | 21.5 s | 3.4 s | 214 MB |
| **this plan**: running-only, completion-order, largest-first by src, 256 MiB | **14.7 s** | **2.8 s** | 246 MB |
| this plan with largest-first by leaf bytes only | 18.0 s | 3.0 s | 227 MB |
| this plan with the 64 MiB budget kept | 40.7 s | 7.5 s | 70 MB |
| this plan, 8 workers | 10.3 s | 2.0 s | 263 MB |

Serial work is 58.7 s (11.0 s with the detector fixed); the largest job is
9.6 s. 14.7 s is the 4-worker lower bound (58.7 / 4). Three facts decide the
design: the slot rule and the order together are the loss (42.7 → 14.7 s),
the 64 MiB budget becomes the binding constraint the moment the order is
fixed (every job above 32 MiB serializes: 40.7 s), and the sort key must
include Codex ancestors (the 12.9 MB fork with a 57 MB base is the most
expensive job in the batch; keyed by leaf bytes it starts in the middle,
18.0 s).

The working tree's detector has changed since the report
(`credential_redaction.rs`, 2026-09-09 11:05, a peer session's work in
progress, 212 lines added): serial build work on this selection is now
18.7 s under a load average of 14, against 58.7 s quiet before. That moves
the Push to the "detector fixed" column, where the scheduler's share of the
remaining wall is larger, not smaller. Measured on that tree with the draft
implementation below, same binary for both schedulers, same selection,
load average 7–14, 4 workers: shipped 12.1 s at 1.53×, this plan 5.1 s at
3.99× (the 4-worker bound is 4.00×); at 8 workers 4.2 s at 5.72×.

**Design decisions.**

1. *Reservations cover running jobs only.* A job's bytes and its worker slot
   are released when its builder returns, not when the serial consumer
   reaches its result. Finished results wait in a channel bounded to
   `workers` entries, and a worker whose send blocks holds one more, so no
   more than 2 × `workers` finished results exist at once: a slow consumer
   applies back-pressure to the workers instead of the workers stalling the
   consumer.
2. *Results are consumed in completion order.* `consume(index, result)` runs
   on the calling thread as results arrive; `index` is the job's position in
   the caller's list. Both callers already build order-independent state:
   Push keys `planned_origins` and `planned_checkpoints` by operation id and
   sorts `operations` by operation id after the loop; Pull applies each
   install against the bindings revision it re-reads under the lock after the
   previous install. The two outputs that were ordered by consumption, Push
   `failures` and Pull `conversation_receipts`, are collected with their
   index and sorted by it after the loop, so results and receipts keep
   exactly today's order.
3. *Largest first.* Jobs are sorted by declared size, descending, before
   dispatch; equal sizes keep index order. Declared size is the bytes the
   builder will hold for the job. Push: captured transcript bytes, sidecar
   bytes, and for Codex the leaf's declared history-base cutoff
   (`history_base.end_byte_offset` from the leaf's authoritative
   `session_meta`, a one-record parse of bytes already in memory). Pull: the
   cloud transcript object plus the child rollout objects the job reads.
   The key under-counts Codex chains deeper than one base and children the
   build discovers itself; that affects the order and the reservation, never
   correctness, and the graph resolver's own 512 MiB aggregate limit still
   caps any one job.
4. *The byte budget stays and becomes a bound on sources in preparation.* A
   job starts when a worker is free and its declared size fits beside the
   jobs running now; a job larger than the whole budget runs alone; a job
   that does not fit waits at the head of the queue and the idle workers wait
   with it. Strict queue order is the simple rule; the cost is idle workers
   during that wait, and on this selection no job waits. Skipping to the
   first job that fits would fill those gaps at the price of an ordering
   rule nobody can predict; not taken.

   The budget is one constant, `PREPARATION_BYTE_BUDGET = 256 MiB`, replacing
   the two `64 * 1024 * 1024` literals. Why 256: the four largest sources of
   this selection (leaf plus ancestors) total 246 MB, so 256 MiB admits the
   full 4-worker schedule (14.7 s simulated, 3.99× measured); 192 MiB gives
   17.1 s, 128 MiB 21.6 s (2.81× measured), 64 MiB 40.7 s. Memory, measured
   on the probe with the same binary: peak footprint 1.00 GB under the
   shipped scheduler, 1.59 GB under this plan at 4 workers (+0.58 GB for
   +180 MB reserved), 1.55 GB at 8 workers, 1.31 GB with a 128 MiB budget.
   A job's transient working set is therefore 3–4.5× its reserved bytes at
   the peak moment: parse trees, the wire copy, and its verification parse
   for four large jobs at once, on top of the capture (at most 512 MiB) and
   the wire outputs the publish request retains until upload regardless of
   the scheduler. That +0.6 GB is the price of the 2.4× wall; 128 MiB buys
   back half of it for a 1.4× slower phase, and is the value to move to if a
   small machine needs it. At 16 workers, the knob's ceiling, the constant
   still caps the transient set.

   Dropping the budget entirely is simpler and equally fast here, but
   16 workers × a 100 MiB file limit, plus ancestors up to 512 MiB per graph,
   is a plausible out-of-memory path on a small machine. Not taken.
5. *Worker count unchanged.* `provider_capture::bounded_refresh_workers`
   (Settings › Performance "comparison workers", default 4, capped by machine
   parallelism and item count) remains the only knob. The report's 8-worker
   point stays reachable through it.
6. *Module renamed.* `ordered_preparation` becomes `bounded_preparation`;
   "ordered" would name a property the scheduler no longer has.
   `for_each_prepared` and `PreparationBounds` keep their names;
   `peak_jobs` and `peak_source_bytes` now mean peak running jobs and peak
   reserved bytes, and the Pull phase label "prepare and install conversations
   (peak N jobs, M source bytes)" keeps its text.

**Steps.**

1. **Replace the scheduler.** Delete
   `crates/mallard-infrastructure/src/ordered_preparation.rs`; add
   `bounded_preparation.rs` with the implementation below; update `lib.rs`
   and the four references (`project_sync/service.rs` twice,
   `codex_standalone_builder.rs` tests, `pull_transcript.rs` tests).

   ```rust
   //! Bounded parallel preparation with serial consumption in completion order.
   //!
   //! Jobs start largest first on a fixed pool of workers. A job's bytes are
   //! reserved only while it is being prepared: a job that does not fit beside
   //! the running jobs waits at the head of the queue, and a job larger than the
   //! whole budget runs alone. Each finished result is handed to `consume` on
   //! the calling thread in completion order, tagged with the job's original
   //! index. Completion order is not deterministic; callers that need a fixed
   //! order sort by that index afterwards.

   use std::collections::VecDeque;
   use std::sync::{mpsc, Condvar, Mutex};

   /// Bytes of transcript sources that may be in preparation at once across
   /// all workers. A job reserves its declared size (transcript, sidecars, and
   /// the declared Codex history base); its transient working set is a small
   /// multiple of that.
   pub const PREPARATION_BYTE_BUDGET: u64 = 256 * 1024 * 1024;

   #[derive(Debug, Default)]
   pub struct PreparationBounds {
       /// Most jobs in preparation at once.
       pub peak_jobs: usize,
       /// Most reserved bytes in preparation at once.
       pub peak_source_bytes: u64,
   }

   struct Queue<T> {
       waiting: VecDeque<(usize, u64, T)>,
       running_jobs: usize,
       running_bytes: u64,
       bounds: PreparationBounds,
   }

   pub fn for_each_prepared<T: Send, R: Send>(
       jobs: Vec<T>,
       workers: usize,
       byte_budget: u64,
       size: impl Fn(&T) -> u64,
       prepare: impl Fn(T) -> Result<R, String> + Sync,
       mut consume: impl FnMut(usize, Result<R, String>),
   ) -> PreparationBounds {
       if jobs.is_empty() {
           return PreparationBounds::default();
       }
       let workers = workers.max(1).min(jobs.len());
       let count = jobs.len();
       let mut waiting: Vec<(usize, u64, T)> = jobs
           .into_iter()
           .enumerate()
           .map(|(index, job)| (index, size(&job), job))
           .collect();
       // Largest first; equal sizes keep their index order.
       waiting.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
       let queue = Mutex::new(Queue {
           waiting: waiting.into(),
           running_jobs: 0,
           running_bytes: 0,
           bounds: PreparationBounds::default(),
       });
       let freed = Condvar::new();
       std::thread::scope(|scope| {
           let (results, completed) = mpsc::sync_channel(workers);
           for _ in 0..workers {
               let results = results.clone();
               let (queue, freed, prepare) = (&queue, &freed, &prepare);
               scope.spawn(move || loop {
                   let (index, size, job) = {
                       let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
                       loop {
                           let Some((_, size, _)) = guard.waiting.front() else {
                               return;
                           };
                           if guard.running_jobs == 0
                               || guard.running_bytes.saturating_add(*size) <= byte_budget
                           {
                               break;
                           }
                           guard = freed.wait(guard).unwrap_or_else(|e| e.into_inner());
                       }
                       let (index, size, job) = guard.waiting.pop_front().unwrap();
                       guard.running_jobs += 1;
                       guard.running_bytes = guard.running_bytes.saturating_add(size);
                       guard.bounds.peak_jobs = guard.bounds.peak_jobs.max(guard.running_jobs);
                       guard.bounds.peak_source_bytes =
                           guard.bounds.peak_source_bytes.max(guard.running_bytes);
                       (index, size, job)
                   };
                   let outcome =
                       std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(job)))
                           .unwrap_or_else(|_| Err("thread preparation worker panicked".to_string()));
                   {
                       let mut guard = queue.lock().unwrap_or_else(|e| e.into_inner());
                       guard.running_jobs -= 1;
                       guard.running_bytes -= size;
                   }
                   freed.notify_all();
                   if results.send((index, outcome)).is_err() {
                       return;
                   }
               });
           }
           drop(results);
           for _ in 0..count {
               let (index, result) = completed
                   .recv()
                   .expect("every prepared job returns one result");
               consume(index, result);
           }
       });
       queue.into_inner().unwrap_or_else(|e| e.into_inner()).bounds
   }
   ```

   Properties worth stating: a worker takes a job only while holding the
   queue lock, so `running_jobs == 0` admits the head unconditionally and no
   two workers can both admit past the budget; `notify_all` after every
   completion re-checks the head under the new totals; a worker whose result
   waits for a busy consumer holds no reservation, and the bounded channel
   keeps finished-but-unconsumed results under 2 × `workers`; a panic in
   `prepare` becomes
   that job's `Err` as today; a panic in `consume` drops the receiver, the
   workers' sends fail, the scope joins them, and the panic propagates, as
   today.

2. **Declared Codex history bytes.** Add to `codex_history_graph.rs`:

   ```rust
   /// Bytes the leaf's authoritative `session_meta` declares for its
   /// immediate history base: a scheduling weight, not a validated read.
   /// A leaf without a base, or one whose metadata does not parse, weighs 0
   /// and reports its own error when it is built.
   pub fn declared_history_bytes(leaf: &[u8]) -> u64 {
       authoritative_session_meta(leaf)
           .ok()
           .and_then(|meta| meta.history_base)
           .map(|base| base.end_byte_offset)
           .unwrap_or(0)
   }
   ```

   `authoritative_session_meta` stops at the first `session_meta` record, so
   this is one small JSON parse per Codex job, on the captured bytes.

3. **Push call site** (`build_publish_request`, `project_sync/service.rs`
   around the `for_each_prepared` call at line 9396 today). Size closure:
   transcript bytes + sidecar bytes + `declared_history_bytes(&transcript.bytes)`
   when `conversation.provider` is Codex; a job whose capture failed weighs 0
   and fails in its own build as today. Budget: `PREPARATION_BYTE_BUDGET`.
   Consumer: push `(index, PushThreadFailure)`; after the loop, sort by index
   and strip it. Rewrite the comment above the call: reservations cover the
   bytes a builder holds while it runs; the worker cap bounds concurrent
   graph reconstruction and redaction caches; results arrive in completion
   order and the state below is keyed or sorted.

4. **Pull call site** (`apply_bundle_restore_with_progress`, the call at
   line 12445 today). Size closure: `cloud_transcript.size()` plus the size
   of every `cloud_subagents` body. Budget: `PREPARATION_BYTE_BUDGET`.
   Consumer: push `(index, receipt)` for both the applied and the failed
   receipt; sort by index after the loop before `applied.receipts.extend`.
   The per-thread `on_phase` lines now interleave in completion order; the
   summary line keeps its text.

5. **Docs.** `docs/PUSH_SLOWNESS_2026-09-09.md` finding 2 gets a one-line
   pointer to this plan. `docs/PLAN_CLAUDE_SIDECAR_PULL.md` §4.4 item 3
   ("the 64 MiB preparation budget is untouched") is updated to name the
   constant and the running-only rule; sidecar bytes are still read in the
   consumer, so the Pull weight still excludes them.

**Tests.**

Retarget, in `bounded_preparation.rs` (the module's four tests are rewritten
for the new contract):

- `consumes_in_order_and_isolates_failed_preparation` becomes
  `results_match_serial_after_sorting_by_index_and_isolate_failed_preparation`:
  the same 8 jobs at 1 and 4 workers, outcomes sorted by index compare equal,
  job 3 is `Err`, seven are `Ok`.
- `completed_results_keep_their_byte_and_job_reservations` is replaced by its
  inverse, `finished_results_free_their_slot_and_bytes`: 2 workers; job 0 is
  the largest and blocks until job 2 has started; job 1 finishes at once;
  job 2 must start while job 0 is still running, and `peak_jobs == 2`. Under
  the shipped rule this schedule deadlocks, which is the behaviour change
  the test pins.
- `oversized_job_runs_alone_and_queue_respects_worker_cap` keeps its sizes
  `[3, 3, 9, 3, 3, 3]`, 2 workers, budget 6: the 9 starts first and nothing
  runs beside it (a running counter observed inside `prepare` never exceeds
  1 while it runs); afterwards the 3s run in pairs; `peak_jobs == 2`,
  `peak_source_bytes == 9`.
- `worker_panic_does_not_drop_sibling_results`: outcomes sorted by index,
  same assertions.

Retarget elsewhere:

- `codex_standalone_builder::tests::captured_encoding_reuses_reads_and_matches_serial_output`
  sorts each run by index before comparing the three configurations. Its
  bounds assertions hold as written: peak running jobs ≤
  `workers.min(admitted_jobs)` and reserved bytes ≤ `admitted_jobs × size`.
- `pull_transcript::tests::serial_and_parallel_preparation_match_for_both_providers`
  derives the expected destination id from the `index` argument instead of
  `results.len()` and compares the fingerprints sorted by index.
  `preparation_benchmark` is unchanged.
- The Pull tests that count `prepare transcript` phases
  (`pull_rechecks_state_between_prepared_installations` and the state-write
  failure test beside it) name no action and need no change; they already run
  three workers.

Add:

- `largest_jobs_start_first`: 1 worker, sizes `[1, 9, 3]`, start order by
  index is `[1, 2, 0]`.
- `results_are_consumed_in_completion_order`: 2 workers; job 0 waits on a
  gate the consumer opens after consuming job 1; the consumer sees index 1
  first. In-order consumption would deadlock here.
- `a_job_that_does_not_fit_waits_at_the_head`: 3 workers, budget 10, sizes
  `[8, 4, 1]`: while 8 runs nothing else starts (4 does not fit and 1 waits
  behind it); after 8 finishes, 4 and 1 run together; `peak_jobs == 2`,
  `peak_source_bytes == 8`. This pins the strict-order rule of decision 4.
- `codex_history_graph::declared_history_bytes`: a leaf whose `session_meta`
  declares a base with `end_byte_offset` N weighs N; a leaf without a base
  weighs 0; malformed bytes weigh 0.
- The Push per-thread isolation test gains a second failing thread whose
  builder is the slower one; the results list stays in action order.

**Acceptance gates.** Simulation and probe figures are for the report's
selection (41 conversations, 598 MB wire) on the 11-core machine, release
profile, redaction on, 4 workers unless stated.

| Measure | Current | Planned |
| --- | ---: | ---: |
| Build phase wall, report's detector | 41.0 s measured (42.7 simulated) | ≈ 15 s (14.7 simulated) |
| Build phase wall, current tree's detector, same binary, loaded machine | 12.1 s measured | 5.1 s measured |
| Speed-up over serial work | 1.48× (1.53× current tree) | 3.99× measured |
| Build phase wall, 8 workers | 16.1 s measured (report) | 4.2 s measured (current tree, 5.72×) |
| Peak reserved bytes | 66 MB | 246 MB (≤ 256 MiB) |
| Peak footprint of the probe, 4 workers | 1.00 GB | 1.59 GB |
| Order of Push results and Pull receipts | action order | action order |

**Validation.**

```sh
cargo test -p mallard-infrastructure --lib bounded_preparation --locked
cargo test -p mallard-infrastructure --lib codex_history_graph --locked
cargo test --locked -p mallard-infrastructure -p mallard-application -p mallard --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Then two Pushes of the same 41-conversation selection with a release build
and 4 workers, one before and one after the change, on the same tree (the
detector is changing in parallel, so only the pair is meaningful): the
`build publish request` line in the activity log should drop by at least 2×,
and its ratio to the serial work should be near the worker count. Confirm the
`~/.mallard/path_origins/*.json` mtimes now start with the largest threads and
end with small ones, and that the Push results rows list threads in the same
order as before.

Probe measurements on the same selection, `/usr/bin/time -l`, both
schedulers built from the same tree (2026-09-09 11:30, the peer's detector
change included), load average 7–14. Memory figures are load-independent;
wall figures are comparable only within this table.

| Configuration | Wall | Serial work | Speed-up | Peak footprint |
| --- | ---: | ---: | ---: | ---: |
| shipped scheduler, 1 worker | 18.7 s | 18.7 s | 1.00× | 947 MB |
| shipped: 4 workers, 64 MiB, thread order | 12.1 s | 18.5 s | 1.53× | 1005 MB |
| shipped slots, no budget, largest-first | 6.4 s | 21.4 s | 3.33× | 1434 MB |
| this plan: 4 workers, 256 MiB | 5.1 s | 20.3 s | 3.99× | 1588 MB |
| this plan: 8 workers, 256 MiB | 4.2 s | 23.9 s | 5.72× | 1551 MB |
| this plan: 4 workers, 128 MiB | 7.2 s | 20.2 s | 2.81× | 1308 MB |

The same probe built at 03:19 against the report's detector, for scale:
serial 857 MB, shipped 891 MB, largest-first with shipped slots 1219 MB at
4 workers and 1562 MB at 8 (wall 78, 72, 30 and 27 s under load). The probe
holds every source and every resolved Codex graph for the whole run, which
the app does not (it resolves inside the job), so these are upper bounds on
the app's transient memory.

| File area | Planned change |
| --- | --- |
| `crates/mallard-infrastructure/src/bounded_preparation.rs` (new; `ordered_preparation.rs` deleted) | Largest-first queue, running-only reservations, completion-order hand-off, `PREPARATION_BYTE_BUDGET`, rewritten tests |
| `crates/mallard-infrastructure/src/lib.rs` | Module rename |
| `crates/mallard-infrastructure/src/codex_history_graph.rs` | `declared_history_bytes` and its test |
| `crates/mallard-infrastructure/src/codex_standalone_builder.rs` | Test sorts runs by index |
| `crates/mallard-infrastructure/src/pull_transcript.rs` | Test uses the index argument |
| `src-tauri/src/project_sync/service.rs` | Both call sites: size closures, budget constant, indexed failures and receipts sorted after the loop, comments |
| `docs/PUSH_SLOWNESS_2026-09-09.md`, `docs/PLAN_CLAUDE_SIDECAR_PULL.md` | Pointers to this plan |

**Not in this plan.**

- Finding 1 of the report, the Unicode word boundaries in the vendor rules.
  Independent; the table above prices both orders of landing.
- Findings 3–6 (apply-time review re-parse, discover re-walk, the wire
  pipeline's re-verification, the small redundancies).
- First-fit admission past a head job that does not fit (decision 4).
- A Settings knob or a machine-derived value for the budget; one constant.
- Changing the default worker count. The 8-worker figures are for reference.
- Weighing Codex chains deeper than one base or children discovered inside
  the build (decision 3).

**Implementation notes (2026-09-09).**

Landed as written, with these deltas:

- `bounded_preparation.rs`: the reservation release uses `saturating_sub`
  (the add saturates, so the pair can never underflow); the poisoned-lock
  recovery of the old module is kept. Tests are the eight named above.
  `a_job_that_does_not_fit_waits_at_the_head` asserts only that the head
  started alone and that jobs 1 and 2 both started afterwards: they are
  admitted together, and which one records its start first is a race
  between two workers, not a property of the queue (it flaked once in 56
  runs when it asserted `[0, 1, 2]`). The module's tests passed 200
  consecutive runs after that change.
- Push (`build_publish_request`): failures are collected as
  `(index, PushThreadFailure)` in `indexed_failures` and become `failures`
  after the loop; the size closure keys on the captured conversation's
  provider. Pull (`apply_bundle_restore_with_progress`): the size closure
  weighs the cloud transcript plus every child rollout body, receipts are
  collected as `(index, receipt)` and appended to `conversation_receipts`
  after the loop, behind the receipts the loop never produced (unaligned
  rows, metadata adoptions, prepare failures), which keep their place.
- `pull_transcript.rs` tests use `PREPARATION_BYTE_BUDGET` instead of their
  own `64 * 1024 * 1024`.
- The Push isolation test (`push_isolates_a_thread_that_fails_to_build`)
  could not name its threads `claude-changed` and `claude-torn` and expect
  that order: actions are sorted by the logical thread id
  (`initial_conversation_thread_id`), a digest, not by session name. The
  test computes both ids and gives the slow failure (an 800-record thread
  appended to between capture and build, which the end-of-build freshness
  check rejects with `PUSH_CHANGED`) the id that sorts first, so the
  builders finish in the opposite order from the actions on every run and
  the results-order assertion is never vacuous.
- Not measured here: the two-Push comparison on the 41-conversation
  selection needs the app and that selection; the probe scratchpad from the
  measurements above is gone. The module-level and service-level tests pin
  the schedule; the wall-time claim rests on the measurements above.
