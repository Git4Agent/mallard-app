# Backend Performance Re-audit — 2026-09-01

## Result

The audit found four main performance risks. PERF-034 has since been fixed in this working tree:

1. The audited Push review budget fitter performed repeated linear lookups while it sorted candidates. A release-mode probe took **13.54 seconds** for 20,000 views. The implemented indexed and materialized version took **4.426 milliseconds** for the same workload.
2. Storage Library Push, Pull, and inventory repeat work across the complete resource or payload set. Full inherited-payload verification detects out-of-band storage corruption, but its serial implementation makes a changing publication scale with repository size.
3. The session-detail header invalidates the provider cache before a read. A cold timeline build also parses the selected transcript twice, and every page can load and clone the complete timeline cache document.
4. Synchronous log, cache, persistence, and object-store work remains on command paths. Several global or service locks include filesystem or serialization work.

The complete Rust library test baseline is green. The repository does not have a maintained benchmark suite, storage request counters, profiler harness, or CI performance gate. This makes regression detection weak.

## Audit baseline

This audit describes the working tree on 2026-09-01:

- branch: `main`;
- `HEAD`: `789eea5`;
- platform: arm64 macOS 26.6.1;
- Rust: 1.96.0;
- Node.js: 20.19.4.

The working tree contains uncommitted product changes. Performance-sensitive changes are present in:

- `crates/mallard-infrastructure/src/provider_capture.rs`;
- `crates/mallard-infrastructure/src/session_detail.rs`;
- `src-tauri/src/project_sync/commands.rs`;
- `src-tauri/src/project_sync/service.rs`.

Therefore, some results do not describe `789eea5` alone. In particular, PERF-030 is fixed only in the current working tree.

This report is separate from `PERFORMANCE_AUDIT_2026-09-01.md`. It does not replace or modify that file.

## Scope

The audit covers these backend surfaces:

- Tauri command orchestration;
- provider discovery, capture, and watcher caches;
- Activity and session-detail reads;
- Push and Pull review and apply;
- local-folder and S3-compatible storage;
- the storage-scoped Library;
- machine-local persistence and activity logs;
- backend performance tests and instrumentation.

The audit uses frontend code only to identify command order and concurrency. It does not audit rendering, React state, CSS, or browser paint work.

The audit did not use real `~/.codex`, `~/.claude`, or `~/.mallard` data. It did not access production S3 or R2 storage. Temporary homes and synthetic data were used.

## Method

The audit used four evidence types:

1. Source inspection of the current command and data paths.
2. Reconciliation of the 2026-08-30 and 2026-09-01 performance audits with current code.
3. Release-mode synthetic probes for two confirmed hot paths.
4. The current Rust library test suites.

Timing results are development-host measurements. Use them to show growth and work amplification. Do not use them as cross-machine latency targets.

## Verification baseline

| Command | Result | Elapsed |
| --- | ---: | ---: |
| `cargo test --workspace --lib` | All library targets passed; 3 manual tests ignored | Not recorded |
| `cargo test -p mallard-infrastructure --lib` | 373 passed, 0 failed, 3 ignored | 12.82 s |
| `cargo test -p mallard --lib` | 257 passed, 0 failed, 0 ignored | 42.36 s |

The ignored tests include manual or real-corpus measurements. They were not run because the repository safety rules prohibit default tests against real provider homes.

## Existing measurement support

`sync_metrics.rs` provides deterministic work counters for provider discovery and capture. Set `MALLARD_SYNC_METRICS=1` before `npm run tauri dev` to print refresh deltas.

Current counters include:

- directory visits and directory-head reads;
- transcript, probe, and appended bytes;
- parsed provider records;
- hashed bytes and history-graph resolutions;
- capture calls;
- inventory-cache and observation-cache hits.

Evidence: [`sync_metrics.rs`](../crates/mallard-infrastructure/src/sync_metrics.rs#L1).

These counters do not include phase duration, CPU time, allocation, peak RSS, object-store requests, storage bytes, retries, fsync, CLI processes, task count, or lock wait and hold duration. Most values are global deltas, so unrelated concurrent work can affect a live reading.

Current performance-oriented tests protect warm comparison byte work, capture selection, deterministic parallel capture, immutable payload reuse, and lazy Pull payload access. They are useful work gates. They are not a benchmark suite.

The repository has no `benches/` directory, Criterion or similar dependency, profiler script, stored benchmark baseline, npm performance target, or CI performance job. The historical standalone probes are no longer present.

## Focused measurements

### Push review budget fitting

The current `fit_capture_budget` builds a closure that scans all views for each size or modification-time lookup. Candidate sorting calls that closure repeatedly. With `V` views and `C` candidates, the current work is approximately `O(V × C log C)`.

Evidence: [`service.rs`](../src-tauri/src/project_sync/service.rs#L6445).

The probe copied the production algorithm exactly and compared it with a `HashMap` lookup. It used unique synthetic resource IDs, deterministic sizes, and deterministic modification times. Both variants returned the same sets.

| Views and candidates | Current linear lookup | Indexed lookup | Ratio |
| ---: | ---: | ---: | ---: |
| 100 | 0.185 ms | 0.037 ms | 4.9× |
| 500 | 6.133 ms | 0.273 ms | 22.5× |
| 1,000 | 25.021 ms | 0.471 ms | 53.2× |
| 2,000 | 103.877 ms | 0.960 ms | 108.2× |
| 5,000 | 694.786 ms | 3.226 ms | 215.4× |
| 10,000 | 3,544.694 ms | 7.030 ms | 504.2× |
| 20,000 | 13,537.555 ms | 18.708 ms | 723.6× |

The 20,000-row case is at the supported conversation-count boundary. This is a user-visible Push review stall, not only a micro-optimization.

### Activity log append

`ActivityLogStore::append` takes the global log I/O guard and a process lock. It then enumerates, validates, and sorts every retained segment to select the append file. Tauri calls this path synchronously before it emits the event.

Evidence:

- [`activity_log.rs`](../crates/mallard-infrastructure/src/activity_log.rs#L290);
- [`activity_log.rs`](../crates/mallard-infrastructure/src/activity_log.rs#L514);
- [`src-tauri/src/activity_log.rs`](../src-tauri/src/activity_log.rs#L89).

A temporary release-mode probe used the production store with a temporary state root. Each case performed 32 appends.

| Existing retained segments | Total for 32 appends | Mean per append |
| ---: | ---: | ---: |
| 0 | 16.969 ms | 0.530 ms |
| 10 | 14.120 ms | 0.441 ms |
| 100 | 55.921 ms | 1.748 ms |
| 1,000 | 540.613 ms | 16.894 ms |

The default retention policy normally has about 10 segments. The maximum accepted policy can retain about 1,000 10 MiB segments. The result confirms linear metadata work in the retained segment count.

The temporary probe was removed after the run. No probe file remains in the repository.

## Current finding register

Priority meanings:

- **P1**: fix before performance work expands the affected path;
- **P2**: fix in the next backend performance cycle;
- **P3**: measure first or fix with related work.

| ID | Priority | Status | Finding | Primary cost |
| --- | --- | --- | --- | --- |
| PERF-034 | P1 | Fixed and measured in the working tree | Push budget fitting used repeated linear view lookups | CPU, command latency |
| PERF-035 | P2 | Measured and source-confirmed | Activity log append and query scan retained files synchronously | Disk metadata, parsing, command latency |
| PERF-036 | P1 | Source-confirmed | Session-detail header invalidates provider caches and a cold index parses the transcript twice | Directory walks, parsing, latency |
| PERF-037 | P2 | Source-confirmed | Warm Activity refresh rewrites the complete pretty-JSON cache | Serialization, writes, fsync |
| PERF-038 | P1 | Source-confirmed | Incremental Library Push verifies or uploads the complete resulting payload set | Network, disk, SHA-256, latency |
| PERF-039 | P1 | Source-confirmed | Library Pull fetches selected resources serially and rereads payloads through several verification stages | Network round trips, disk, SHA-256 |
| PERF-040 | P1 | Source-confirmed | Library inventory retains complete payload bytes without one aggregate inventory budget | Peak RSS, repeated I/O |
| PERF-041 | P2 | Source-confirmed | Common persistence reads sync the state directory and parse some documents twice | Disk barrier, allocation, lock hold |
| PERF-042 | P2 | Source-confirmed | Bundle prefetch de-duplicates with a linear scan and adds transfer batch barriers | CPU, cloud tail latency |
| PERF-043 | P2 | Source-confirmed | Library listing and workspace resolution repeat serial inventory and metadata reads | CLI launches, storage requests, latency |
| PERF-044 | P2 | Source-confirmed | Catalog recovery and apply repeatedly rewrite complete journal documents | Serialization, fsync, lock hold |
| PERF-014 | P2 | Source-confirmed | Library Push can construct two S3 clients and connection pools in one apply | client setup, connection reuse |
| PERF-045 | P2 | Source-confirmed | S3 body reads have an inactivity timeout but no complete-transfer deadline | occupied permits, unbounded latency |

## Detailed findings

### PERF-034 — Push budget fitting was effectively quadratic

The audited `fit_capture_budget` called `views.iter().find(...)` for every size and modification-time lookup. Sorting called the modification-time lookup twice per comparison. The final admission loop performed another lookup per candidate.

| 20,000-row implementation | Release-mode elapsed |
| --- | ---: |
| Audited repeated linear scans | 13,537.555 ms |
| Index only | 18.708 ms |
| Implemented index plus one metadata resolution per row | 4.426 ms |

Implemented change:

1. Build one first-match map from resource ID to `(size, modification time)` before budget fitting.
2. Attach size and modification time to each candidate before sorting.
3. Keep the deterministic newest-first policy, stable ID tie-break, base retention, missing-ID default, and saturating byte accounting.
4. Assert in a deterministic unit test that each relevant row's metadata is resolved exactly once.

This is an internal algorithm change. It adds no migration or legacy path and does not change schema 1.

### PERF-035 — Activity log work grows with all retained data

Append scans all log segments to select the active segment. Query holds the same global and process locks while it:

- opens every retained file;
- parses every bounded JSONL record;
- stores every match;
- sorts all matches;
- truncates only after the sort.

Evidence: [`activity_log.rs`](../crates/mallard-infrastructure/src/activity_log.rs#L318).

The newest-N unfiltered query can stop after it has read enough records from newest segments. Search and arbitrary filters still require a scan unless the store adds an index.

Recommended change:

1. Use one async single-writer queue for append work.
2. Cache the current day, segment, and size.
3. Read segments newest first for newest-N queries and stop early.
4. Keep a bounded result heap or add an index for filtered queries.
5. Add counters for files opened, bytes read, records parsed, and lock wait time.

### PERF-036 — Session-detail reads discard useful cache state

The header command calls `rebaseline_project_provider_caches` before it loads session views. That function resolves every bound provider home and calls `force_rebaseline`. `force_rebaseline` removes the watcher profile and all related caches. The next load must run full discovery.

Evidence:

- [`service.rs`](../src-tauri/src/project_sync/service.rs#L1929);
- [`service.rs`](../src-tauri/src/project_sync/service.rs#L12582);
- [`session_watch_service.rs`](../src-tauri/src/project_sync/session_watch_service.rs#L972).

This reliability reset is appropriate before an operation that authorizes state change. It is excessive for a read-only detail header.

On a timeline-cache miss, `load_or_build_index` builds the complete timeline and then `summarize_session` parses the selected transcript again. A page request calls `load_or_build_index`, even when it returns at most 50 events.

The timeline cache has two conflicting limits:

- 32 MiB serialized file size;
- 50,000 entries before reset.

The byte limit normally fails first. A hit deserializes the complete cache document and clones the selected complete index. A write serializes the complete cache document.

Evidence: [`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L36) and [`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L909).

Recommended change:

1. For read-only detail, revalidate the selected session and watcher generation. Do not drop every profile cache.
2. Build the index and summary in one provider parse, or cache both outputs from one parse.
3. Keep hot indexes in an in-process `Arc` cache keyed by the transcript snapshot.
4. Use a byte-based LRU. Use the disk cache only for cold start.
5. Align the entry limit with the serialized byte limit.

### PERF-037 — Warm Activity refresh still writes the full cache

`scan_local_session_views` correctly reuses cached summaries when size and modification time agree. After every scan, it still pretty-serializes and atomically writes the complete cache, even when no entry changed.

Evidence: [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs#L374) and [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs#L473).

Recommended change:

1. Track whether an entry, schema, or retention set changed.
2. Skip the write for a clean cache.
3. Use compact JSON for machine-local caches.
4. Serialize outside service locks.

### PERF-038 — Library Push scales with the complete resulting repository

The Library Push apply path rebuilds the complete resulting resource map. It also copies every file byte vector for each selected skill into `Bytes`.

For a publication that changes the content map, the publication engine then:

1. uploads every payload supplied for selected resources;
2. verifies every referenced payload that was not supplied;
3. performs those inherited verifications one at a time.

Evidence:

- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L1567);
- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L1693);
- [`resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs#L293);
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L712);
- [`local_store.rs`](../crates/mallard-infrastructure/src/storage_core/local_store.rs#L157).

For S3, an immutable object that already exists causes a failed absent-condition PUT and then a complete verification GET. For local storage, inherited verification copies the object to a temporary file and calls `sync_all`.

Therefore, changing one resource in a large Library can cause work proportional to all payloads in the resulting manifest. A changed multi-file skill can also PUT and GET unchanged files in that skill. A true semantic no-op returns before payload transport, but the apply layer has already copied selected-resource payload bytes by then.

The inherited verification is not redundant under the current trust model. A pinned manifest proves the expected digest and size. It does not prove that an object still exists or that a local-folder user or bucket administrator did not change it outside Mallard.

Recommended change:

1. Use the reviewed previous manifest to identify expected inherited digests. Do not treat it as proof of current payload integrity.
2. Upload only new digests from changed resources. Do not issue a failed absent-condition PUT for a digest already known to be inherited.
3. Under the current trust model, verify each distinct inherited payload once through a fixed-size rolling scheduler.
4. Treat removal or deferral of inherited verification as an explicit product trust-model change. Define when missing or corrupt objects will then be detected and repaired.
5. Record payload GETs, PUTs, bytes, and SHA-256 bytes per publication.

This change must preserve immutable publication, SHA-256 verification, and compare-and-swap head updates.

### PERF-039 — Library Pull is serial and multi-pass

The selected-resource loop fetches one resource at a time. A skill then fetches each file one at a time. The resource engine calls `stage_verified` even though the object-store interface already has `stage_many_verified`.

Evidence:

- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L2009);
- [`apply.rs`](../src-tauri/src/storage_catalog/apply.rs#L1019);
- [`resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs#L222).

One S3 payload can pass through these stages:

1. network stream to a temporary file with SHA-256 and `sync_all`;
2. bounded read of the verified temporary file;
3. write to an apply-stage tree;
4. reread and verify the stage tree;
5. activation;
6. reread and verify the target tree.

The safety checks are valid. The implementation does not carry the verification witness between stages, so it repeats byte work.

Recommended change:

1. Build one apply-wide fetch plan keyed by repository ID, SHA-256, and size. Fetch each distinct payload once.
2. Use a fixed-size rolling in-flight scheduler. Do not create one Tokio task for every queued object.
3. Pass verified temporary-file handles into apply.
4. Remove a reread only when the same immutable file identity and digest witness remain valid.
5. Keep final target verification after activation.

### PERF-040 — Library inventory has no aggregate byte budget

`resolve_scope_workspace` runs `inventory_local` for list, review, and apply paths. Skill inventory captures and retains all file bytes. Pet inventory retains atlas bytes. Per-resource limits exist, but there is no one aggregate count or byte limit for the complete local inventory.

Evidence:

- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L854);
- [`inventory.rs`](../src-tauri/src/storage_catalog/inventory.rs#L386);
- [`skill_tree.rs`](../src-tauri/src/storage_catalog/skill_tree.rs#L126);
- [`codex_pet.rs`](../src-tauri/src/storage_catalog/codex_pet.rs#L214).

Peak memory therefore scales with all payload bytes under the configured roots. Push apply can allocate another complete copy for selected resource bytes before the revision limit rejects an oversized payload set.

There are also per-resource cardinality gaps. Skill capture bounds regular files and bytes but not empty directories. Pet capture collects every extra entry name without one count bound.

Recommended change:

1. Split inventory into a summary phase and a selected-capture phase.
2. Summary inventory must retain paths, bounded metadata, digests, and warnings, not complete payload bytes.
3. Capture bytes only for reviewed selected resources during apply.
4. Add per-resource and aggregate file-count, directory-count, entry-count, and byte budgets.
5. Revalidate selected resource bytes and warnings during apply. Do not reuse an unverified review-time payload.
6. Add peak-RSS measurements at the supported limits.

### PERF-041 — Persistence reads perform avoidable barriers and allocations

The project-sync persistence layer uses one process-wide lock. Common reads call `load_or_create_host_identity_unlocked`. When the identity exists, that function still opens the state root and calls `sync_all` on the directory.

Evidence:

- [`persistence.rs`](../crates/mallard-infrastructure/src/persistence.rs#L60);
- [`persistence.rs`](../crates/mallard-infrastructure/src/persistence.rs#L547);
- [`persistence.rs`](../crates/mallard-infrastructure/src/persistence.rs#L1730).

`load_bindings_document_unlocked` and `load_materializations_unlocked` first deserialize into `serde_json::Value` and then deserialize again into the target type. Their accepted files can be large.

Recommended change:

1. Sync the directory only after identity creation, replacement, or recovery publication.
2. Deserialize valid current-schema documents directly into their target types.
3. Separate unrelated state locks or use a repository-scoped lock.
4. Measure lock wait, lock hold, bytes parsed, and fsync duration.

### PERF-042 — Bundle prefetch adds CPU and scheduling barriers

`EngineByteSource::prefetch` uses `pending.iter().any(...)` to de-duplicate paths. This gives quadratic comparison work for a large path request. The function then divides requests into concurrency-sized chunks. The S3 store already limits concurrency with a semaphore, so the outer chunks add completion barriers.

Upload has the same outer chunk pattern.

Evidence:

- [`bundle_engine.rs`](../crates/mallard-infrastructure/src/bundle_engine.rs#L303);
- [`bundle_engine.rs`](../crates/mallard-infrastructure/src/bundle_engine.rs#L915);
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1041).

Recommended change:

1. Use a `BTreeSet` or `HashSet` for path de-duplication.
2. Replace fixed completion batches with a fixed-size rolling in-flight scheduler.
3. Bound queued tasks as well as active transfers. The current semaphore alone does not bound task memory.
4. Add a mixed-latency test that proves no idle slot while queued work exists.

### PERF-043 — Library commands repeat local and remote workspace work

`resolve_scope_workspace` performs a complete local inventory and then reads remote head, tag, and manifest data. Review and apply call it separately. Publication reads tag, head, and the previous manifest again.

For an existing repository, one changing Library Push apply can construct at least two S3 clients and connection pools. Before payload verification, it can also perform at least eight S3 `GET Object` requests: workspace head, tag, and manifest; a second tag read; then marker, head, tag, and previous manifest reads in publication. The S3 metadata operation uses `GetObject`, not `HeadObject`.

The Library list path probes six provider-and-kind repositories. It performs serial head and tag reads. An empty remote Library can require about 12 serial metadata requests. The object-store interface has `get_metadata_many`, but this path does not use it.

Plugin inventory can also launch provider CLI processes on each inventory.

Evidence:

- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L854);
- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L1188);
- [`service.rs`](../src-tauri/src/storage_catalog/service.rs#L1717);
- [`adapters.rs`](../src-tauri/src/storage_catalog/adapters.rs#L343);
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L523).

Recommended change:

1. Carry review summaries and immutable remote metadata to apply so apply does not repeat a complete inventory.
2. At apply, revalidate the link revision, storage identity, mutable head, plan pins, selected Push content and warnings, Pull claims, selected target state, and plugin state.
3. Recapture only selected resources and re-inspect only selected targets.
4. Batch independent metadata reads and reuse a configured S3 client by storage configuration generation.
5. Do not cache plugin inventory only by executable and profile pin. Installed and marketplace state can change independently.
6. Add request, client-construction, and process-launch counters.

### PERF-044 — Catalog recovery and apply rewrite complete journals

Catalog apply writes the operation journal around each item and state transition. Each write pretty-serializes and atomically replaces the complete operations document.

Recovery runs during scope listing. It scans all nonterminal operations and can rewrite the complete operations document once for each recovered entry. Total serialization can grow quadratically with the number of pending operations.

Evidence:

- [`apply.rs`](../src-tauri/src/storage_catalog/apply.rs#L1296);
- [`persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs#L410);
- [`persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs#L755).

The catalog persistence mutex also covers some remote clear work. Slow storage can therefore block unrelated local catalog state operations.

Recommended change:

1. Recover all entries in memory and publish one journal update.
2. Use compact machine-local encoding.
3. Move remote clear outside the persistence mutex only after adding a two-phase clearing state or reservation and a final revision-checked commit. Otherwise, retain the lock.
4. Batch journal transitions while preserving crash-recovery boundaries.

### PERF-045 — S3 slow-drip reads can occupy a transfer permit indefinitely

S3 reads apply a timeout to the initial request and a separate inactivity timeout to each body-chunk wait. A peer that sends one small chunk before each inactivity timeout can keep a transfer active without one total deadline.

Evidence: [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L620).

Recommended change:

1. Keep the inactivity timeout.
2. Add a total body deadline or a bounded minimum byte-rate policy derived from the declared object size.
3. Add a slow-drip test for metadata, staged object, and capability-probe body reads.

### Carried PERF-018 — Push review can read one historical manifest twice

Push review first calls `classify_remote_continuity` with a live storage probe. It then calls `classify_push_destination` with a new live probe. `classify_push_destination` calls `classify_remote_continuity` again.

Evidence: [`service.rs`](../src-tauri/src/project_sync/service.rs#L7423) and [`service.rs`](../src-tauri/src/project_sync/service.rs#L7158).

When continuity requires a historical-manifest check, one review can fetch and verify the same manifest twice. The nearby source comment says continuity is classified once, but the current call graph classifies it twice.

Recommended change:

1. Compute continuity once.
2. Pass the result into destination classification and thread comparison.
3. Add a counting-store test that permits at most one historical-manifest read per review.

## Prior finding disposition

The earlier audits remain useful, but several findings no longer match current code.

### Resolved or stale

| Prior ID | Current disposition |
| --- | --- |
| PERF-001 | Partly resolved. The closed drawer no longer loads all log data. Backend append and query work remains open as PERF-035. |
| PERF-002 | Partly resolved. Independent post-recovery reads now overlap, and path audits start after the shell is usable. Initial recovery and identity phases remain serial. Re-measure before further change. |
| PERF-004 | Resolved for project bundle publication. Payload ownership uses shared `bytes::Bytes`, and pointer-identity coverage protects that store boundary. Storage Catalog still copies selected file vectors into new `Bytes` allocations. |
| PERF-005 | Resolved for bundle listing. The engine uses the closed tag namespace and one bounded pass. Local-store pagination remains expensive in clear and other paths. |
| PERF-017 | Resolved for the root log list. The log drawer owns log entries and loads only when open. Backend log I/O remains open as PERF-035. |
| PERF-025 | Resolved in `789eea5`. Claude discovery uses narrow metadata parsing and a bounded tail window with a full-stream fallback. |
| PERF-027 | Stale for the normal Activity flow. The frontend now awaits Activity before comparison. The backend still lacks a general single-flight guard for independent concurrent callers. |
| PERF-030 | Resolved only in the dirty working tree. Session-detail commands encode once and return `tauri::ipc::Response`. The complete encoded response is still allocated before the size rejection. |
| PERF-031 | Resolved in `ec82ce1`. Canonical provider-home paths are reused. |

### Still open or reframed

| Prior ID | Current disposition |
| --- | --- |
| PERF-003 | Open. Startup conversation-assignment audits can scan the same provider home once per project. Reframe around one shared per-home index. |
| PERF-006 | Open and expanded by PERF-040. Library inventory retains complete resource payloads. |
| PERF-007 | Open. Activity can load up to 10,000 Git commits before time-window filtering and performs repeated mapping work. The old `chat_history.rs` ownership is stale. |
| PERF-009 / PERF-028 | Open and expanded by PERF-036. Timeline cache hits deserialize and clone complete indexes, and cache writes replace the complete document. |
| PERF-010 | The old finding is stale after persistence changes. PERF-041 and PERF-044 describe the current full-document work. |
| PERF-011 | Open and expanded by PERF-038. Publication verifies inherited Library payloads one at a time. |
| PERF-012 | Open. Storage clear can run up to one million sequential deletes. Local listing re-walks and sorts the full namespace on each page. |
| PERF-013 | Open. Observation-cache path canonicalization, cache assembly, serialization, and atomic write occur while the watcher mutex is held. |
| PERF-014 | Open and included in PERF-043. One changing Library Push apply can build at least two S3 clients and connection pools. |
| PERF-018 | Open and source-confirmed. Push review can run the same historical-manifest continuity probe twice. Add a request-count test. |
| PERF-019 | Partly resolved because many commands use blocking workers. Synchronous activity-log work and blocking-pool occupancy still need measurement. |
| PERF-021 | Open. Release strip and LTO policy is not explicit. |
| PERF-024 | Open. No performance gate exists. |
| PERF-026 | Open. Inventory cache keys include local project ID. Concurrent misses can perform duplicate full discovery because discovery occurs outside the lock without an in-flight entry. |
| PERF-029 | Open and included in PERF-042. Transfer chunks wrap store operations that already have a semaphore. |
| PERF-032 | Open. Ignore evaluation creates a memo map per call and allocates formatted prefixes. |
| PERF-033 | Open. Large machine-local caches and journals use pretty JSON. |

PERF-008, PERF-015, PERF-016, and PERF-020 are frontend findings. They are outside this report.

PERF-022 and PERF-023 concern build and release workflow. They are outside this runtime backend audit.

## Secondary confirmed risks

These items are real, but the current evidence does not justify placing them ahead of PERF-034 through PERF-045:

| Surface | Confirmed work | Next evidence |
| --- | --- | --- |
| Provider capture | Selected files can be revalidated, reread, secret-scanned, hashed, and parsed in separate passes. Aggregate capture can retain 512 MiB. Some repetition enforces race and safety checks. | Count source reads, canonicalizations, secret-scan bytes, SHA-256 bytes, and peak RSS by phase. |
| Activity and Git | One refresh can reload local state and start several Git processes. History loads up to 10,000 commits before later filtering and mapping. | Count JSON reads, Git processes, commits retained, and mapping comparisons. |
| Host-operation guard | Push review and apply hold the host-sensitive-operation guard across discovery, capture, storage reads, transfer, and journal work. Contention fails immediately. | Measure guard hold time and rejected concurrent operations. Preserve state-change serialization. |
| S3 task scheduling | Bulk methods create one Tokio task per object and use a semaphore only after task creation. Task memory is `O(N)`. | Measure queued task count and RSS for 1,000 to 100,000 small objects. |
| S3 client lifecycle | Each configured storage engine can create a new S3 client and connection pool. | Count client construction, DNS, TLS, and connection reuse across 20 commands. |
| Storage clear | Clear lists the first 1,000 keys repeatedly and deletes each key separately. Local list re-walks and sorts the complete namespace for every page. | Count walks, pages, and DELETE requests at 1,000, 10,000, and 100,000 objects. |
| Bundle apply | Directory approval can scan every approved project file for each directory. Object-cache LRU hits use a linear queue search under one mutex. | Measure directory actions, prefix comparisons, cache entries, and lock hold time. |
| Publication conflict | A head compare-and-swap conflict can repeat immutable-object requests even when captured digests did not change. | Use a forced-conflict counting store and report requests per retry. |

## Cross-cutting causes

The findings have five common causes:

1. **Delta loss.** Review computes a delta, but apply often reconstructs the complete state. Full inherited-payload verification is deliberate under the current storage trust model.
2. **Verification witness loss.** A stage verifies bytes, but the next stage rereads them because a valid immutable-file witness does not travel with the file.
3. **Conflicting caches.** Caches reduce parsing but add full-document deserialization, clone, and rewrite work.
4. **Lock scope.** Locks protect valid invariants but include directory walks, remote I/O, serialization, or fsync.
5. **Missing work counters.** Tests can detect output errors but cannot detect request, byte, parse, or lock amplification.

## Recommended implementation sequence

### Phase 0 — Remove the current regression

Completed in the current working tree.

Acceptance conditions:

- budget-fit policy and output remain unchanged;
- 20,000 views require no repeated linear scans;
- a regression test counts at most one metadata lookup per candidate, plus bounded setup work.

### Phase 1 — Add measurement infrastructure

Add a measuring object-store wrapper and backend phase timers before storage changes.

Record:

- LIST, HEAD/metadata, GET, PUT, DELETE, and compare-and-swap counts;
- logical, network, staged, and target bytes;
- SHA-256 bytes;
- files opened and fsync count;
- provider CLI launches;
- task count and maximum active transfers;
- mutex wait and hold duration;
- response bytes and encode duration;
- command wall time and peak RSS for release-mode scenarios.

Keep deterministic work-count budgets in normal CI. Keep noisy wall-time budgets in a controlled benchmark job.

### Phase 2 — Bound Library memory

Implement the two-phase inventory from PERF-040. This prevents high RSS before transfer scheduling work begins.

Acceptance conditions:

- list and review do not retain payload bytes;
- apply recaptures and revalidates only selected resources;
- per-resource and aggregate count and byte limits fail before large allocation;
- warnings, no-follow traversal, and special-file rejection remain unchanged.

### Phase 3 — Bound and de-duplicate Library transfer work

Implement PERF-038 and PERF-039 together.

Acceptance conditions:

- changing one file uploads only new digests and does not issue failed PUTs for known inherited digests;
- inherited payload verification runs once per distinct digest through a fixed-size rolling scheduler under the current trust model;
- selected Pull files use apply-wide digest de-duplication and bounded rolling concurrency;
- queued tasks and active transfers both remain bounded, with no completion-batch idle gap;
- immutable SHA-256 verification, plan pins, expiry, and head compare-and-swap remain intact.

### Phase 4 — Fix read-path cache behavior

Implement PERF-036, PERF-037, and PERF-041.

Acceptance conditions:

- a session-detail read does not invalidate unrelated provider cache state;
- one cold selected transcript is parsed once;
- a timeline page does not deserialize or clone unrelated indexes;
- clean Activity refresh does not rewrite its cache;
- ordinary persistence reads do not call directory `sync_all`;
- serialization and filesystem work occur outside service mutexes where safe.

### Phase 5 — Fix background and bulk work

Implement PERF-035, PERF-042, PERF-043, PERF-044, and PERF-045. Then address carried PERF-012, PERF-013, and PERF-026.

## Required measurement matrix

| Surface | Workloads | Required evidence | Work budget |
| --- | --- | --- | --- |
| Startup assignment audit | 1, 5, and 20 projects on one provider home | walks, files, bytes, command duration | one provider-home scan |
| Provider discovery | 100, 1,000, and 20,000 files; cold, warm, append, truncate, rename | visits, probe bytes, append bytes, parsed records, RSS | warm: no transcript body; append: probe plus appended bytes |
| Concurrent cache misses | 1, 2, and 8 callers on one home | duplicate walks, lock wait, worker occupancy | one in-flight discovery per home and request |
| Push budget fit | 100 through 20,000 candidates | comparisons, metadata lookups, duration | linear lookup work plus `O(C log C)` sort |
| Activity | 100, 1,000, and 10,000 sessions; cold and warm | summaries, Git commits, cache bytes written | warm: no cache write when unchanged |
| Session detail | 1,000, 10,000, and 50,000 events; header, first page, page 2 | parses, cache bytes, clone bytes, encode bytes, RSS | one parse on miss; no unrelated index clone on hit |
| Activity log | 0, 10, 100, and 1,000 segments; newest 100 and filtered search | files, bytes, records, sort size, duration | append: constant metadata work; newest-N: early stop |
| Library local inventory | 10, 100, and 1,000 skills; small and near-limit files | files, bytes retained, CLI launches, peak RSS | summary inventory retains no payload bytes |
| Library incremental Push | 1 changed file in 10, 100, and 1,000-resource repositories | GETs, PUTs, bytes, SHA-256 bytes | PUT each new digest once; verify each distinct inherited digest once under the current trust model |
| Library Pull | 1, 32, and 256 objects at 0, 30, and 100 ms injected RTT | request rounds, active transfers, staged bytes, elapsed | bounded continuous concurrency |
| Library empty listing | local and S3-compatible stores | metadata requests and rounds | one batched metadata round where supported |
| S3 body deadline | declared small and large bodies with a slow-drip peer | permit hold time, bytes, timeout class | inactivity and total-transfer bounds both terminate the read |
| Storage clear | 1,000, 10,000, and 100,000 objects | list walks/pages, DELETE requests, elapsed | bounded bulk deletion; no full local re-walk per page |
| Persistence | normal and maximum accepted cache/journal sizes | parse bytes, allocations, lock hold, fsync | no read-path directory fsync; one journal write per batch |

## Performance gates

The repository has useful work-count tests, but it does not have a benchmark framework or CI performance job.

Keep and expand these existing invariants:

- warm Codex and Claude comparison read budgets;
- capture-only byte pinning;
- worker-count-invariant parallel capture;
- project bundle publication skips payloads in the previous manifest;
- project bundle publication reuses capture allocation at its store boundary;
- project bundle Pull plan and apply touch only required objects.

These three bundle-engine invariants do not cover Storage Catalog.

Add these deterministic gates first:

1. one shared provider-home scan for concurrent identical requests;
2. linear Push budget metadata lookup count;
3. zero clean Activity cache writes;
4. one inherited Library verification per distinct digest and no failed PUT for a known inherited digest;
5. apply-wide Pull digest de-duplication with bounded rolling concurrency;
6. zero directory fsyncs on ordinary persistence reads;
7. one catalog journal publication per recovery batch;
8. no complete local namespace re-walk for each clear page;
9. slow-drip S3 bodies terminate at the total-transfer bound.

Add release-mode timing and RSS baselines only after the fixtures and counters are stable. Store the fixture version, Git revision, build profile, machine class, and raw results with each run.

## Safety constraints for optimization

Performance changes must not weaken these contracts:

- immutable objects remain SHA-256 verified;
- mutable heads retain compare-and-swap semantics;
- Pull and dependency apply remain plan-based and explicitly approved;
- plan revision, digest, pin, and expiry checks remain active;
- no-follow traversal, canonical-path validation, and special-file rejection remain active;
- user-content deletion and destructive replacement remain plan-approved and expected-state checked;
- whole-storage clear remains explicit, marker-guarded, namespace-bounded, exact-key, and count-bounded;
- credentials, provider-home and checkout paths, local exclusions, plans, receipts, backups, and trust decisions remain outside portable manifests.

The safe optimization pattern is to carry verified evidence forward. Do not remove a verification step unless the next stage receives an equivalent immutable-file witness. A pinned manifest alone is not proof that a payload still exists or matches its digest.

## Conclusion

The next backend performance cycle should add storage work counters, then bound Library inventory and transfer scheduling. Session-detail cache invalidation and full-document cache work are the next read-path targets.

Do not start with broad async conversion or a new cache abstraction. The main costs are unnecessary work, lost deltas, repeated verification, and oversized lock scopes. Remove those costs first.
