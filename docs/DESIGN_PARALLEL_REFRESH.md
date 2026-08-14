# Parallel Refresh: Bounded Concurrency for Capture and Local Observation

**Status:** implemented (see Section 8 for deltas discovered during
implementation)<br>
**Scope:** the observation-class Refresh/review pipeline — provider capture
(`capture_from_discovery`) and per-thread local observation
(`observe_codex_local`) inside `provider_thread_review`.<br>
**Out of scope:** Push/Pull apply, journaling, recovery, and any cloud I/O
through `BundleBytes`. Those stay serial by design.

This document specifies where bounded parallelism is safe in the Refresh
path, which state forces serialization, and the determinism rules a parallel
implementation must satisfy so that its observable behavior — results,
errors, and error *order* — is byte-identical to today's serial loop.

## 1. Why this is worth doing

Refresh and Push review share one dominant cost: per-transcript CPU work
that is repeated for every selected conversation.

Measured stages over the same payloads
([PERF_SYNC_PATH_MEASUREMENTS.md](PERF_SYNC_PATH_MEASUREMENTS.md),
`opt-level = 0` shares):

| stage | where | share of capture |
| --- | --- | --- |
| `analyze_codex_transcript` — one `serde_json::Value` per JSONL record | capture | 57% |
| `resource_digest` — SHA-256 over file bytes | capture | 35% |
| `detect_secret_material` — lowercase copy + substring scans | capture | 6% |

Two corrections to the perf document's framing, so nobody sizes this work
from stale inputs:

1. The often-quoted **9.3s** figure (9.251s over 288.5 MB at 0.0321 s/MB,
   dependencies at `opt-level = 1`) is the **analyze-only corpus cost**, a
   projection — not a measured end-to-end capture run. SHA-256 and the
   secret scan ride on top of it.
2. The perf document's classify attribution (~4.1s of candidate-loop digest
   recomputes in `recover_unique_compatible_binding_from_meta` →
   `classify_thread_refresh`) describes code **deleted by the portable-tail
   rewrite**. Classification cost now lives in `observe_codex_local`:
   cursor rides parse only appended records; full re-resolves happen on
   first contact, on unpinned adoption probes, and on imported-layout
   observations. Do not size the observation stage from the old numbers.

Every stage in the table is per-transcript and independent. With the
work-list sorted and a bounded pool, the theoretical floor for the projected
9.25s analyze corpus at four workers is ≈ 2.3s of parallel work plus serial
preflight/merge — **~3s theoretical, 3.5–5s realistic**. The largest single
transcript in the measured corpus (~27 MB ≈ 1s) stays well under the
per-worker share, so the worker bound dominates, not the largest-item floor.

Parallelism multiplies with — and does not replace — derivation reuse
(Section 6). Running today's redundant parses on four cores is still
redundant.

## 2. Pipeline shape

Four phases replace the single serial loop in `capture_from_discovery`
([provider_capture.rs](../crates/mallard-infrastructure/src/provider_capture.rs)),
and the same shape applies to the per-thread observation loop in
`provider_thread_review`
([commands.rs](../src-tauri/src/project_sync/commands.rs)):

```text
1. Serial preflight        cross-resource checks, no file reads
2. Parallel per-item work  read, scan, hash, analyze, summarize / observe
3. Serial merge            resource-ID order; deterministic errors
4. Serial mutations        Phase A seeding, plan construction, persistence
```

This is a clean cutover of the existing loop, per repository policy: the
staged pipeline **replaces** the serial body, and `workers = 1` degenerates
to today's behavior. There is no parallel variant living beside a serial
one.

### 2.1 Phase 1 — serial preflight

The current capture loop interleaves three pieces of cross-resource state
with per-file work. All three move into a serial preflight pass that reads
**metadata only**, never file bytes:

- the `casefold_paths` case-insensitive logical-path collision map;
- the `total_bytes` accumulation against `MAX_CAPTURE_BYTES` (512 MiB) —
  `content_len` already comes from `fs::metadata` or `derived_bytes`, so
  the aggregate gate needs no reads and must **not** remain a mid-stream
  short-circuit inside parallel workers;
- logical-path duplicate-ownership detection (the `result.files` insert
  check), which is derivable from the discovery set before any capture.

Preflight failures return before any worker starts.

### 2.2 Phase 2 — parallel per-item work

**Capture item** (one selected resource): `fs::read` →
`detect_secret_material` → `resource_digest` → `adapter.analyze` →
`adapter.summarize_file`.

**Observation item** (one bound thread): `observe_codex_local` — cursor
ride or full resolve + fold, including the adoption-probe refold. Cloud
evidence derivation from the already-loaded `BundleSnapshot` is pure and
may ride along.

Both item kinds are safe to run concurrently because:

- `AgentProviderAdapter` and `SessionAdapter` are declared `Send + Sync`
  ([providers/traits.rs](../crates/mallard-infrastructure/src/providers/traits.rs)),
  and the production registry is a `&'static OnceLock`
  ([providers/registry.rs](../crates/mallard-infrastructure/src/providers/registry.rs));
- neither item body touches `LocalStateRepository`, the persistence lock,
  or any shared mutable document;
- distinct items read distinct provider files.

Scheduling: dispatch items **in resource-ID order**. Largest-first
dispatch is incompatible with deterministic fast-fail: with cancellation
enabled and two failing items, the reported error would depend on which
subset ran before cancellation, and that subset depends on worker count.
In-order dispatch restores the invariant — an error at item *k* proves
every item before *k* was already dispatched and completes, so the merge's
earliest-failure rule is deterministic at every worker count (Section
2.3). The makespan cost is the tail-scheduling edge only; the worker bound
dominates for the measured size distribution.

Per-file coherence: keep a transcript's `fs::read` and its
`summarize_file` in the **same worker**. The summarizer re-reads from disk
(`summarize_rollout_file` streams and parses every record again), and
same-worker adjacency keeps the pre-existing TOCTOU window per-file
coherent. Parallelism neither fixes nor worsens that window.

### 2.3 Phase 3 — serial merge

Workers return `Result<CapturedItem, (ResourceId, String)>`. The merge:

- inserts results in **resource-ID order** (the input is already a
  `BTreeSet`), reproducing today's deterministic output ordering;
- on any failure, reports the error of the **earliest resource ID** — not
  whichever worker failed first. This reproduces today's first-error
  semantics exactly;
- sets a cancellation flag on first observed failure so idle workers stop
  picking up new items; in-flight items run to completion and are
  discarded. Partial merges never escape the function.

Warnings keep their existing `sort` + `dedup` finalization, which is
order-insensitive already.

### 2.4 Phase 4 — serial mutations

Everything that writes stays exactly where it is, in one thread:

- **Phase A seeding** in `provider_thread_review` mutates
  `conversation_bindings` and `sync_config` under the persistence lock. It
  is already a separate pass in the two-phase review design and stays
  serial, after observation results are available.
- Evidence assembly, classification (`compare_conversation_tail` — one
  call site, enforced by
  `architecture_rejects_any_second_classifier_call_site`), decision
  mapping, plan-action construction, and plan persistence all remain a
  single serial loop over the merged, ordered results. They are cheap.

## 3. What must never be parallelized here

- **`BundleBytes`** — the pull-side transcript prefetch and any cloud read
  flow through `&mut dyn BundleBytes`, an exclusive borrow. Restructuring
  the byte source for concurrent reads is out of scope; the parallel
  section covers local observation only.
- **Repository mutations** — every `mutate_*` call, Phase A seeding
  included. One writer, always.
- **Journal-adjacent code** — apply, recovery, and reconciliation keep
  their locked, ordered semantics untouched.
- **Push apply reconstruction** — `build_publish_request`'s re-resolve,
  re-seed, witness re-verification, and standalone build are apply-time
  proof obligations (plan §8), not Refresh cost. They are deliberately
  redundant with review and stay serial inside the host-operation guard.

## 4. Concurrency bound and memory

Workers: `min(std::thread::available_parallelism(), 4)`.

The bound exists for **transient parse state, not retained bytes**.
Captured bytes are retained unconditionally — every selected file's bytes
end up in `CapturedResources` whether the pipeline is serial or parallel,
so residency does not change with worker count. What the bound caps is the
in-flight overhead: `analyze` builds a `serde_json::Value` tree per record
(typically several × file size while alive), on top of the per-file
`bytes.clone()` the perf document already flags at ~2× residency. Four
workers cap the transient at `4 × (file + parse tree)` over the retained
set.

The work is CPU-bound (serde + SHA-256); beyond four workers the
largest-file floor and memory transients erode the return.

Implementation note: `std::thread::scope` with a shared `AtomicUsize` work
cursor is sufficient. No new dependency — the workspace has no `rayon`, and
this shape does not justify adding one.

## 5. Determinism contract (test requirements)

A parallel implementation ships with tests pinning:

1. **Result equivalence** — `workers = 1` and `workers = 4` produce
   byte-identical `CapturedResources` (same maps, same ordering, same
   warnings) over a fixture with ≥ 8 mixed-size transcripts.
2. **Error identity** — with two failing resources planted (secret
   material in one, oversized file in another), the parallel capture
   reports the same error string as the serial one: the earliest resource
   ID in `BTreeSet` order, regardless of worker completion order.
3. **Aggregate gate placement** — a selection exceeding
   `MAX_CAPTURE_BYTES` fails in preflight with no file read (observable
   via a read-counting fixture or by planting an unreadable-but-oversized
   file that must *not* produce a read error).
4. **Observation equivalence** — a review over a fixture with cursor-ride,
   full-resolve, adoption-probe, and imported-layout threads produces
   identical `ThreadSyncEntry` sequences at both worker counts.
5. The full existing suite as the regression gate (348 tests at time of
   writing) — the pipeline is a refactor of observable-equivalent shape,
   so no existing assertion may move.

## 6. Derivation reuse (the multiplier)

Parallelism accelerates redundant work; reuse removes it. Current
per-transcript parse counts:

- **Every captured conversation parses twice**: `adapter.analyze(&bytes)`
  and `summarize_rollout_file(path)` each walk every record. Folding the
  summary metrics into the analyze pass (or feeding the summarizer the
  already-read bytes) removes a full parse for every selected
  conversation, in or out of a review.
- **A reviewed codex thread on the full-resolve path parses again** via
  `resolve_codex_root`. For a null-base, single-segment thread the
  resolved chain is definitionally the captured lineage chain; reusing it
  is safe only if the observation keeps its own file-identity checks
  (safe end, probe window) against the same bytes the capture read.
  Paginated threads still walk ancestor segments capture never saw — no
  reuse there.
- **Push apply parses twice more** (resolve + builder output
  re-analysis). This redundancy is intentional proof-of-work at the
  reconstruction point and is not a reuse candidate.

Reuse changes which bytes back an observation (captured buffer vs. a fresh
disk read), so any reuse lands as its own change with its own
equivalence tests — this document only fixes the boundary: reuse within
capture is free; reuse across capture → observation must preserve the
observation's identity checks; reuse across review → apply is forbidden.

## 7. Expected outcome

Against the projected 9.25s analyze-dominant corpus (288.5 MB, deps at
`opt-level = 1`):

| change | projected |
| --- | --- |
| bounded parallel capture (4 workers) | ~3s theoretical, 3.5–5s realistic |
| + analyze/summarize parse fusion | removes ~40–50% of remaining per-transcript CPU |
| observation stage | unmeasured post-rewrite; expected small except first-contact storms (re-registration, adoption probes) |

Numbers are projections over the perf document's per-stage rates, not
measurements of this design. Re-measure after implementation with the same
per-stage instrumentation before claiming the win.

## 8. Implementation deltas

The implementation (in `capture_from_discovery_with_workers` /
`capture_resource` in
[provider_capture.rs](../crates/mallard-infrastructure/src/provider_capture.rs)
and the observation pre-pass in `provider_thread_review`) follows this
document with three deltas:

1. **Dispatch order is resource-ID order, not largest-first** — required
   for deterministic earliest-failure under fast-fail cancellation
   (Section 2.2). The same rule applies to the observation stage's
   thread-ID order.
2. **Error precedence between phases changed once, deliberately**: every
   structural preflight failure (path validation, case-fold collision,
   ownership, per-file and aggregate size gates) now surfaces before any
   content failure (read errors, secret material), because preflight runs
   to completion before workers start. Within each phase, ordering is
   unchanged, and all messages are byte-identical to the old loop's.
3. **The per-file `bytes.clone()` is gone**: the worker builds each
   `CapturedFile` once and the merge moves it into the result map, since
   the conversation analysis that previously forced the duplicate now runs
   inside the same worker. This removes the ~2× peak residency the perf
   document flagged.

Worker-count invariance, earliest-failure identity, and the read-free
aggregate gate are pinned by `parallel_capture_is_worker_count_invariant`,
`parallel_capture_reports_the_earliest_failing_resource`, and
`aggregate_capture_gate_fails_in_preflight_without_reading` (the last via
33 sparse, unreadable at-limit files whose aggregate crosses the cap). The
observation stage's invariance is covered by the full review suite, whose
expectations were authored under the serial code; a direct worker-count
equivalence harness for `provider_thread_review` was not added because the
command fixture cannot vary the pool size.
