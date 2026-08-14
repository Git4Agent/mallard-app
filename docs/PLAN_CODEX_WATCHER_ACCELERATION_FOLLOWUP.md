# Plan: Codex watcher-backed observation reuse

- Status: Proposed
- Date: August 13, 2026
- Scope: Codex session discovery, local-tail observation, Refresh, Push/Pull
  review, one process-local filesystem watcher, and bounded runtime caches
- Deferred: event-driven UI freshness — the watcher never pushes updates to
  the frontend; all operations stay demand-driven (section 13)
- Excluded: portable schema changes, sync-policy changes, background
  transcript parsing, apply-time verification shortcuts, subagent
  synchronization, and legacy compatibility paths
- Compatibility: clean cutover; no migration and no legacy execution path

## 1. Objective

Make repeated Codex session work proportional to changed sessions by reusing
cached local results whenever the watcher proves quiet, and recomputing only
what changed when it does not:

```text
first operation for a profile
  arm watcher over sessions trees        -> generation a
  bounded discovery + observation        -> cache (local_results_a, a)

no change since a
  Refresh / Push review / Pull review reuse local_results_a directly;
  only the remote head/manifest is re-read and P/L/C re-classified

changes since a (generation b, dirty paths D)
  recompute only D: dirty roots ride the bounded probe/append reader from
  their persisted checkpoints; membership changes re-enumerate only the
  affected subtrees                      -> cache (local_results_b, b)

reliability signal, restart, or resume doubt
  drop the cache and rebaseline through the same bounded path
```

`local_results` means the local side only: the metadata inventory and the
per-root tail observations. The remote head/manifest is read fresh on every
operation and classification always re-runs — cloud changes must appear even
when the local watcher is quiet.

The previously measured 31-session serial baseline was:

```text
discover local state                 0.833s
capture local conversations          6.129s
classify threads and build actions   2.284s
total                                9.285s
```

Those numbers predate the bounded parallel capture/observation pipeline that
has since landed, which cut wall-clock by roughly the worker factor without
cutting bytes read. This plan cuts the bytes: discovery head-reads every
rollout in the provider home, capture reads every transcript, and observation
re-reads every leaf. Step 1 re-measures the baseline before acceptance is
judged.

## 2. Product and safety contract

1. There is exactly one execution path. A cold start, a reliability signal,
   and a fully dirty cache all run the same bounded discovery/observation
   code the warm path runs for its dirty subset. No mode switch, no
   compatibility reader, and no second all-session capture/review
   implementation after the cutover.
2. The watcher is a reuse oracle, never a correctness authority. A watcher
   event or cache entry never promotes accepted cloud state, authorizes an
   action, or replaces an authoritative plan/apply digest. `accepted` never
   changes during Refresh.
3. Reused local results are display and planning evidence only. Every apply
   retains all existing source, target, digest, object, journal, revision,
   quiescence, no-clobber, and compare-and-swap verification, so a stale
   plan fails closed at apply exactly as it does today.
4. Trust boundary, stated plainly: watcher silence is trusted only between
   reliability signals. Queue or map overflow, `need_rescan`, backend error
   or disconnect, watched-root removal or replacement, profile path change,
   process restart, and resume without provable uninterrupted delivery each
   drop the profile's cache and force rebaseline. The accepted residual
   risk is a silently dropped event with no signal: rows may render stale
   until the next event, signal, or restart — and such staleness can never
   corrupt an apply (rule 3).
5. Watcher and cache state are bounded and process-local. They are absent
   from `SyncConfig`, machine bindings, portable manifests, plans, journals,
   receipts, and cloud objects. The durable `ConversationCheckpoint` remains
   the only persisted observation state, and no reliable generation is ever
   persisted across restart.
6. Absolute provider-home paths, rollout paths, and transcript content never
   enter frontend events, activity logs, or portable data. The notify
   callback performs no filesystem I/O, parsing, hashing, persistence, cloud
   access, or sync classification.
7. Existing no-follow traversal, canonical containment, regular-file checks,
   size limits, capture-time secret scanning, immutable publication, and
   no-clobber apply rules remain intact.
8. Internal Codex subagent rollouts retain current behavior: discovery
   excludes them from local session capture and they do not enter conflict
   identity.
9. Any capture-time blocked condition that can now first surface at
   observation time produces the same user-visible row detail as the
   discovery/capture path (section 5).

## 3. Current implementation constraints

The design builds on the active schema-1 implementation; each fact below is
verified in the current tree:

- `provider_capture::discover_full` walks `sessions/` and `archived_sessions/`
  of the whole provider home and head-reads every rollout to determine
  project ownership, so discovery cost scales with the home, not the project.
- `provider_capture::capture_from_discovery` reads, secret-scans, hashes,
  analyzes, and summarizes every selected conversation, and thread status
  selects every discovered conversation before local-tail observation.
- `observe_codex_local` re-reads the complete leaf through
  `read_rollout_bounded` even when the outcome is `Unchanged`.
- the observation stage already returns the refreshed seeded checkpoint value
  and the merge discards it; only first-contact seeding persists, so cursor
  advancement is durable only at apply and every Refresh re-derives from the
  last applied pin.
- `RootPhysicalMapping` already carries the incremental-fold resume cursors
  (`last_canonical_message_id`, `last_synthetic_turn_id`,
  `next_synthetic_turn_ordinal`) and the 4-KiB trailing probe window, so the
  bounded reader requires no durable schema change.
- the discovery metadata scan is already bounded to head lines per file; no
  metadata extension is required by this plan.
- a matching ready checkpoint can incrementally extend either `WholeThread`
  or `FixedPaginatedTail`; the cursor path reads only the leaf, and prefix
  integrity is carried by the fold-chain state and the publish-time prefix
  witness, so leaf-scoped invalidation gives a reused observation exactly
  the sensitivity of the current authoritative cursor path — no ancestor
  dependency map is required.
- `ConversationCheckpoint.observed_tail` is a replaceable Refresh cache while
  `accepted` is destination-scoped cloud evidence; checkpoints live on the
  storage link, so observation entries are per destination.
- the status path already mutates local state (first-contact seeding and
  binding layout), so persisting checkpoint maintenance in the same phase
  adds no new state-mutation surface.
- the conversation file bound is 100 MiB.

## 4. Architecture

```text
filesystem watcher (one per canonical Codex home)
  -> profile generation + dirty root set + inventory/titles dirty flags
  -> cached metadata inventory and cached per-root observations
     (durable checkpoints make dirty-root recompute proportional to append)
  -> Refresh and Push/Pull review
  -> lazy authoritative capture for byte-pinning candidates only
```

There is no per-path generation table, no input fingerprint, and no ancestor
dependency map: invalidation is a dirty set, reuse is generation equality
plus the in-process revisions already known exactly (binding, config,
checkpoint generation), and cursor reuse is leaf-scoped by the argument in
section 3.

### 4.1 Metadata-only local session view

Add a provider-neutral, backend-only view derived from `Discovery`:

```rust
struct LocalSessionView {
    provider: ProviderKey,
    provider_thread_id: String,
    resource_id: ResourceId,
    relative_cwd: String,
    display_name: String,
    source_path: PathBuf,
    source_size: u64,
    source_mtime: u64,
    blocked_reason: Option<String>,
}
```

Names are illustrative; the physical path is backend-private, and size/mtime
are presentation data. Status and preliminary review classification consume
these rows; they do not require `CapturedFile`, codec analysis, a content
digest, or a portable `ResourceDescriptor`. The review's local-thread map is
built from inventory rows and binding resolution; observation opens the
source path from the row; first-contact seeding reads only the layout-less
bound roots it seeds.

`CapturedResources` remains the byte-bearing input for actions that publish,
preserve, replace, or otherwise pin local content. Its checks are unchanged.

### 4.2 Bounded probe/append reader

Replace the cursor path's whole-file `read_rollout_bounded` call with a
bounded reader that:

1. opens the exact expected path without following a final symlink;
2. requires a regular file within the 100-MiB conversation bound;
3. records a runtime file stamp from the opened handle;
4. reads and hashes the saved probe window ending at `consumed_byte_offset`;
5. reads from `consumed_byte_offset` through the bounded current EOF;
6. exposes only complete newline-terminated append bytes;
7. retains an incomplete suffix as dirty, unconsumed data;
8. re-stats the opened handle before accepting the result; and
9. retries a small bounded number of times if size or identity changes
   during the read, then fails closed.

Refactor `refresh_codex_tail` to consume the validated probe and append
slices instead of a vector containing the complete file; the resume cursors
in `RootPhysicalMapping` already carry the incremental-fold state. First
contact, adoption probes, detached mappings, layout disagreement,
truncation, probe mismatch, malformed append, and unresolved ancestry still
use the bounded full resolver or fail closed, unchanged.

### 4.3 Checkpoint maintenance

Refresh persists the advanced observation it already computes:

- a no-op observation reuses the checkpoint's existing raw range pin and
  persists nothing;
- when the cursor extends, persist the complete replacement — observed tail,
  root mapping, and accepted-anchored raw range pin — as one unit; hashing
  cost is that root's accepted-to-current interval only;
- when the full resolver runs for an already-bound thread, persist its
  seeded observation the same way;
- persistence increments the checkpoint generation under the existing
  config/checkpoint and binding revision checks; a concurrent promotion,
  binding edit, or host operation prevents stale persistence;
- a failed raw-range refresh never persists a torn mapping/pin pair; and
- `accepted` never changes during Refresh.

Maintenance runs serially after the parallel observation merge and before
the pure-read classification phase pins revisions, extending the existing
post-seeding revision rule to post-maintenance. Persisted cursors are what
make a dirty root's recompute proportional to its append rather than its
length.

### 4.4 Watcher service

One Tauri-managed service owns one watcher per exact canonical Codex home,
shared by every project bound to that profile and armed lazily on the first
operation that needs it. For each home, watch only:

- `sessions/` recursively when it exists;
- `archived_sessions/` recursively when it exists; and
- the home non-recursively, to observe creation, removal, or replacement of
  those directories and atomic replacement of `session_index.jsonl`.

Never watch the complete provider home recursively and never follow
symlinks.

Per-profile state, all bounded:

```rust
struct ProfileWatchState {
    canonical_home: PathBuf,
    healthy: bool,
    generation: u64,
    dirty_roots: BoundedSet<PathBuf>,
    inventory_dirty: bool,
    titles_dirty: bool,
}
```

The notify callback bounds event and path counts and forwards to a
coalescing worker; it does nothing else. The worker classifies:

- modify of a known rollout path: add to `dirty_roots`, bump `generation`;
- create, remove, rename, archive movement, an unknown JSONL path, or a
  directory event: set `inventory_dirty`, bump `generation`;
- `session_index.jsonl` change: set `titles_dirty` only;
- a known excluded subagent path: ignore for root state; and
- overflow, `need_rescan`, backend error or disconnect, watched-root removal
  or replacement, or a full `dirty_roots` set: mark unhealthy and drop the
  profile's caches.

Arm-before-scan baseline: arm the watcher first, then run bounded discovery,
then publish the cache. Events delivered during the baseline land in the
dirty sets like any others and simply make the first reuse partial — no
re-stat or retry protocol is needed. An unhealthy or unarmable watcher means
every operation for that profile runs the bounded path; restart and resume
doubt reset to unarmed.

### 4.5 Cached local results

Two bounded process-local maps, invalidated by the watcher state and by
in-process revisions that are known exactly:

- Inventory: keyed by provider profile, local project, binding revision, and
  adapter version; stores the `LocalSessionView` rows and parsed titles.
  Reuse requires a healthy watcher, matching key, and `inventory_dirty`
  false; `titles_dirty` alone re-reads titles through the existing filtered
  read. When `inventory_dirty` is set, re-enumerate only the subtrees
  containing dirty or unknown paths, head-reading only new or changed files,
  then clear the flag under the generation observed before the walk.
- Observations: keyed by thread and storage; stores the derived
  `LocalTailObservation` with the checkpoint generation and binding revision
  it was derived from. Reuse requires a healthy watcher, the root absent
  from `dirty_roots`, and matching generations/revisions. A dirty root runs
  the bounded reader from its persisted checkpoint (or the full resolver on
  any miss) through the existing worker pool, then clears its dirty entry
  under the pre-dispatch generation.

Reuse performs no stat, no probe, and no read for clean entries — that is
the point. Adoption probes force the full-resolve path today and continue
to: a thread whose checkpoint lacks accepted evidence is never served from
the observation cache while a cloud tail is present for it.

Source removal or identity change invalidates the entry rather than
representing an empty append; an incomplete JSONL suffix keeps its root
dirty until a complete record boundary is observed. If an update cannot be
retained atomically, drop the whole profile's caches. Never retain partial
evidence while a consumer is marked clean.

### 4.6 Lazy candidate capture

Push/Pull review first classifies P/L/C from inventory and observation and
determines candidate action intent. It then captures and pins only
candidates whose final capability or plan requires local bytes:
`CreateCloud`, `Publish`, conflict preservation, overwrite, preserve, or
rebind targets.

Candidate selection is a conservative work scheduler, not a second
sync-policy engine. It may capture a false-positive candidate, but it must
never omit a resource that the existing decision/capability path could
require; ambiguity selects capture. Capture runs through the existing
selected-resource `capture_from_discovery` with every current check. The
final user-visible decision is produced only after that authoritative
candidate work succeeds; no provisional action is exposed. Final decisions
continue through the one shared decision implementation, and candidate
capture failure returns the same blocked capability/decision the full path
would return.

## 5. Row parity and warning semantics

Two behavior deltas are intended, bounded, and tested; nothing else about
row output may change.

1. Blocked-detail parity. A condition that discovery or capture reports
   today — most concretely a rollout crossing the 100-MiB bound by append,
   or a transcript whose lineage cannot be resolved — can now first surface
   at observation. One shared mapping produces the identical blocked row
   detail regardless of which stage detected the condition. No
   stage-dependent reason codes.
2. Advisory secret warnings. Capture-time secret scanning is unchanged, but
   Refresh no longer captures unchanged transcripts, so per-Refresh
   "possible secret" advisories for non-candidate rows disappear. The
   authoritative secret gate is unaffected: every push candidate is captured
   and scanned before publication. The equivalence tests normalize exactly
   this advisory difference and nothing else.

## 6. Operation flows

### 6.1 Refresh

1. Resolve the binding and exact canonical provider profile; arm the
   watcher if this is the profile's first operation.
2. Reuse the cached inventory when permitted by 4.5, else re-enumerate the
   dirty subtrees (or run full bounded discovery cold/unhealthy).
3. Build local rows from `LocalSessionView` without capture.
4. Seed first-contact state for layout-less bound threads, reading only
   those roots.
5. Reuse clean cached observations; run only dirty and cache-miss roots
   through the existing bounded worker pool, merging in deterministic
   thread-ID order with the existing earliest-failure rule.
6. Persist advanced checkpoints under generation compare-and-swap (4.3) and
   update the observation cache.
7. Read the remote head/manifest and classify the same P/L/C values through
   the single `compare_conversation_tail` call site.

The warm no-change floor is step 7's remote read; steps 2–6 are cache
lookups.

### 6.2 Push review

1. Reuse inventory and observations as in Refresh.
2. Classify P/L/C and derive candidate Push intent without capturing
   unrelated transcripts.
3. For byte-pinning candidates, run authoritative selected-resource capture
   and derive every required source, range, target, artifact,
   provider-state, and secret-review pin.
4. Recheck the normal plan inputs — config, checkpoint, and binding
   revisions — before publishing the final review.
5. If candidate capture fails, return the same blocked capability/decision
   the full path would return.

Synced, cloud-ahead, metadata-only, unavailable, and unrelated sessions do
not force transcript capture.

### 6.3 Pull review

1. Reuse inventory and observations as in Refresh.
2. Classify P/L/C and derive candidate Pull intent without capturing
   unrelated transcripts.
3. Perform authoritative local target reads and capture only for candidates
   that overwrite, preserve, rebind, or otherwise pin local bytes.
4. Read immutable remote payloads only for candidate validation that cannot
   be proven from manifest metadata; reuse the existing digest-keyed object
   cache.
5. Recheck the normal plan inputs before returning the final review.

Storage-only, synced, local-ahead, metadata-only, blocked, and unrelated
rows do not force transcript capture unless their selected action requires a
local-byte pin.

### 6.4 Push/Pull apply

Apply is unchanged and is the correctness backstop for every reuse above.
It still re-opens and verifies exact local sources and targets, verifies
reviewed raw ranges and target digests, reconstructs and hashes provider
output, downloads and verifies immutable objects, enforces provider
quiescence and no-clobber writes, preserves journal and recovery behavior,
and verifies the remote head with compare-and-swap where required. A plan
built on stale reused evidence fails closed here exactly as a raced plan
does today.

Mallard-generated writes need no suppression: pull apply seeds its durable
checkpoint from verified transaction output as today, and its file writes
arrive as ordinary watcher events that dirty the affected roots for the
next operation.

## 7. Concurrency and bounds

- The observation worker pool keeps the existing bounded policy: ID-order
  dispatch, deterministic merge, deterministic earliest-failure selection.
  Parallelism applies only to dirty and cache-miss roots and never changes
  row order, error selection, or review output.
- Watcher and cache state live behind one service lock; dirty-set drain and
  cache publish happen under the generation observed before the recompute,
  so an event racing a recompute leaves its path dirty for the next
  operation instead of being lost.
- All sets and maps are bounded; a bound overflow marks the profile
  unhealthy and drops its caches whole. Never evict part of the evidence
  while a consumer is marked clean.
- Checkpoint persistence uses the generation compare-and-swap semantics of
  4.3; there is a single checkpoint writer per operation.

## 8. Module ownership

```text
crates/mallard-infrastructure/src/codex_history_graph.rs
  Bounded probe/append reader, runtime file stamps, and the refactored
  refresh_codex_tail consuming probe and append slices.

crates/mallard-infrastructure/src/provider_capture.rs
  LocalSessionView projection from Discovery; the existing selected-resource
  byte capture, unchanged.

src-tauri/src/project_sync/codex_watch_service.rs
  notify watchers, per-profile generation and dirty state, and the bounded
  inventory and observation caches.

src-tauri/src/project_sync/commands.rs
  Refresh and review orchestration, checkpoint maintenance, lazy candidate
  capture, P/L/C comparison, and plan construction.

src-tauri/src/lib.rs
  Process-lifetime managed service ownership.
```

No file moves. Watcher and cache types remain outside `mallard-domain` and
serialized frontend DTOs.

## 9. Implementation sequence

Each step lands green on the full gate battery and deletes the path it
replaces in the same change. Steps 2–4 cut the bytes and stand on their own;
the watcher then removes the residual per-operation discovery and probe
work.

1. Add instrumentation counters — directories visited, files head-read,
   transcript bytes read, probe/append bytes read, records parsed, bytes
   hashed, full resolves, selected captures, cache reuse hits — and
   re-measure the baseline on the current parallel pipeline.
2. Persist refreshed observations under generation checks (4.3); add race
   tests proving `accepted` never changes.
3. Add the bounded probe/append reader (4.2) with equivalence tests against
   the whole-buffer cursor path, including the blocked-detail parity rule.
4. Add `LocalSessionView` and cut Refresh over from full capture, landing
   the advisory-warning semantics of section 5 explicitly.
5. Add the watcher service, arm-before-scan baseline, dirty-set
   invalidation, and the inventory/observation caches with reuse (4.4, 4.5).
6. Route Push/Pull review through cached results with lazy candidate
   capture (4.6). Highest-risk step; lands last.
7. Benchmark cold baseline, warm no-change, sparse append, new rollout, and
   Push/Pull review with mixed actionable and irrelevant sessions.

## 10. Test plan

All tests use temporary Codex, Mallard, project, and storage roots. They
never watch a real provider home.

### 10.1 Reader and checkpoint tests

- whole-buffer and probe/append readers produce identical unchanged,
  incomplete-tail, extended, malformed, truncated, and replaced outcomes;
- same-length file replacement with a changed runtime identity and any
  probe mismatch fail closed;
- a moving EOF retries within bounds and never consumes an incomplete
  record;
- an oversize or lineage-broken condition detected at observation produces
  the identical blocked row detail as the discovery/capture path;
- Refresh advances only replaceable observed fields, increments the
  generation, and preserves accepted evidence exactly;
- a generation or binding race prevents stale persistence; and
- a failed raw-range refresh never persists a torn mapping/pin pair.

### 10.2 Watcher and cache tests

- watcher arming precedes baseline discovery, and events delivered during
  the baseline leave their paths dirty rather than published clean;
- append, create, remove, rename, archive movement, session-index
  replacement, and watched-root replacement produce the expected dirty
  state (roots, inventory, titles, unhealthy respectively);
- known subagent events do not change root state, while unknown candidates
  set `inventory_dirty` and are boundedly classified on the next operation;
- overflow, `need_rescan`, backend error, disconnect, dirty-set overflow,
  restart, and resume uncertainty drop the caches and rebaseline;
- an event racing a recompute leaves its path dirty for the next operation;
- two projects sharing one profile retain independent inventories;
- a suppressed event (test seam) yields stale rows, the stale plan fails
  closed at apply, and the next event or rebaseline recovers — proving
  contract rule 4's blast radius; and
- cache reuse for clean entries performs no stat, probe, or read, asserted
  through the instrumentation counters.

### 10.3 Refresh and review equivalence tests

- a warm no-change Refresh performs no tree enumeration and reads no local
  session bytes; its only external read is the remote head/manifest;
- one append among many sessions reads and parses only the affected root's
  probe and appended range, and the next Refresh starts from the persisted
  cursor;
- storage A and storage B retain independent checkpoints and observation
  entries;
- watcher-enabled, forced-rebaseline, and unhealthy-watcher runs produce
  identical rows, relations, decisions, and plans after normalizing exactly
  the advisory-warning delta of section 5;
- Push/Pull review performs no transcript capture for unchanged or
  irrelevant sessions, and every actionable candidate still receives all
  required capture, secret, range, source, target, and artifact checks;
- candidate capture failure produces the same final blocked decision as the
  full-capture path; and
- apply outcomes, cloud heads, bindings, checkpoints, receipts, and
  recovery fixed points are identical with the watcher healthy, unhealthy,
  or freshly rebaselined.

## 11. Acceptance criteria

1. A warm no-change Refresh performs no transcript capture, no tree
   enumeration, and no local session reads; its floor is the remote
   head/manifest read.
2. Sparse local changes perform read, parse, and hash work proportional to
   dirty roots, and advanced cursors persist so the work does not repeat.
3. Push/Pull review captures only byte-pinning candidates.
4. Healthy, unhealthy, and rebaselined runs produce identical relations,
   decisions, plans, apply outcomes, and recovery outcomes, modulo the
   documented advisory-warning delta.
5. No watcher event or cache entry authorizes mutation or replaces an
   authoritative digest; `accepted` never changes during Refresh; stale
   reuse can at worst produce a plan that apply rejects.
6. The optimization adds no durable or portable schema field, no legacy
   compatibility path, and no second execution path; conditions surface
   with stage-independent row details.
7. Watcher and cache state remain bounded, process-local, destination-aware
   for observations, and path-private, and current subagent exclusion and
   root-only conflict identity remain unchanged.

## 12. Verification

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mallard-domain
cargo test -p mallard-infrastructure
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
```

Run the instrumented benchmark at cold baseline, warm no-change, after one
append, after one new rollout, and during Push/Pull review with a mixture of
actionable and irrelevant sessions.

## 13. Deferred and rejected

Deferred: event-driven UI freshness. The armed watcher could push a "local
sessions changed" hint to the frontend to prompt an automatic Refresh; that
is deliberately out of scope. Operations remain demand-driven, and the
watcher is consulted only when the user acts. If auto-refresh becomes a
goal, it layers onto this service as a frontend hint without changing the
reuse rule, the checkpoint authority, or the single execution path.

Rejected: per-operation stat revalidation as the reuse witness (re-stat
every visited directory and known root before serving the cache). It makes
reuse independent of watcher delivery but re-introduces per-operation
filesystem work proportional to the profile and duplicates what apply
already guarantees. This plan instead trusts watcher silence between
reliability signals, keeps apply authoritative, and accepts the bounded
staleness window of contract rule 4.

Rejected: ancestor dependency maps, per-path generation counters, and
observation input fingerprints from the earlier draft — leaf-scoped
invalidation matches the authoritative cursor path's sensitivity (section
3), and in-process revisions are known exactly without fingerprints.
