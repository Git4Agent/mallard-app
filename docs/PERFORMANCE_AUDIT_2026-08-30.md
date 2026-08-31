# Mallard performance audit

- **Audit date:** 2026-08-30
- **Application version:** 0.1.2
- **Source baseline:** `f325a63edcdd` plus the existing staged, unstaged,
  and untracked working-tree changes
- **Scope:** React rendering, Tauri startup and commands, provider discovery and
  capture, Activity and session detail, local persistence, project storage,
  Storage Catalog, S3/R2 transport, build output, and release automation
- **Purpose:** identify work that grows faster than the user-visible result,
  runs before it is required, repeats an existing observation, or retains too
  much memory

This report describes the current working tree. It is not an audit of an older
release tag. Recheck the findings after the current changes are committed or
replaced.

Priority is the recommended remediation order, not correctness severity:

- **P1:** can dominate a normal workflow or exceed practical memory, I/O, or UI
  limits at supported product bounds;
- **P2:** important scaling or repeated-work issue; and
- **P3:** build, release, or performance-observability improvement.

## Executive assessment

Mallard has useful performance safeguards. Provider capture uses bounded work,
S3 bulk transfers use bounded concurrency, the provider watcher has a clean
cache path, large restore objects use streaming verification, and request IDs
prevent stale frontend results from replacing current state.

The main performance risks are outside those optimized paths:

1. startup parses retained activity logs several times and loads the closed log
   drawer;
2. startup serializes at least 13 native-command phases and includes full Codex
   path audits;
3. Codex assignment auditing bypasses the watcher cache and scans the complete
   profile again on Activity refresh;
4. Push can hold three or more copies of a capture that is allowed to reach
   512 MiB;
5. bundle pagination and some storage operations restart full namespace scans;
6. Library inventory eagerly reads and retains the complete installed skill
   corpus;
7. Activity, Files, session detail, and logs retain or render unbounded data;
   and
8. the initial frontend bundle contains almost every feature workspace.

The recommended first milestone is to make startup and large-data work
bounded. The next milestone should reduce Push and Library memory ownership and
remove repeated remote I/O. UI isolation and build-size work can follow once
the data paths have fixed limits.

## Audit method and limits

The audit used the current source and generated production artifacts. It
included:

- static inspection of the React render and state graph;
- static inspection of the Tauri and Rust call graph;
- production TypeScript and Vite builds;
- the complete frontend integration suite;
- a native release build and incremental-build measurements;
- focused Rust performance-invariant tests;
- asset and executable size measurements; and
- production React server rendering of the actual Activity commit component at
  100, 1,000, 5,000, and 10,000 rows.

The audit did not use a representative large Codex or Claude home, a production
R2 account, or live high-latency storage. Remote-storage findings use request
counts and control flow instead of production latency.

A native launch was not used as performance evidence because overriding
`HOME` alone did not isolate macOS provider state. The launch was stopped
before UI interaction. It did not change user state.

The React scaling measurement uses production server rendering. It proves
React and markup growth. It does not include WebView style calculation,
layout, paint, or compositor work.

## Findings register

| ID | Priority | Area | Finding |
| --- | --- | --- | --- |
| PERF-001 | P1 | Startup/logs | Startup can parse all retained logs at least three times and render a closed drawer |
| PERF-002 | P1 | Startup | The shell serializes at least 13 native-command phases |
| PERF-003 | P1 | Provider history | Codex assignment audits rescan the complete provider home |
| PERF-004 | P1 | Push memory | Push can retain more than three copies of a 512 MiB capture |
| PERF-005 | P1 | Storage history | Bundle pagination restarts full namespace scans |
| PERF-006 | P1 | Library | Inventory reads and retains the complete skill corpus |
| PERF-007 | P1 | Activity | Backend mapping and frontend rendering scale poorly with long history |
| PERF-008 | P1 | Project files | The non-Git Files view has no render bound |
| PERF-009 | P2 | Session detail | A 50-event page can reload and clone the complete timeline cache |
| PERF-010 | P2 | Persistence | Conversation changes perform one durable config rewrite per thread |
| PERF-011 | P2 | Library publish | One change can download every inherited payload sequentially |
| PERF-012 | P2 | Storage clear | Clear can issue up to one million sequential delete requests |
| PERF-013 | P2 | Session cache | Disk-cache serialization occurs while a global service mutex is held |
| PERF-014 | P2 | S3/R2 | Command paths recreate S3 clients and connection pools |
| PERF-015 | P2 | Catalog previews | Remote-only pet previews load serially and rerender the full page |
| PERF-016 | P2 | Storage setup | Repository discovery can start for each path or bucket keystroke |
| PERF-017 | P2 | React state | Logs, splitters, transfer progress, and review selection update the app root |
| PERF-018 | P2 | Push review | Remote-ahead review can perform the continuity probe twice |
| PERF-019 | P2 | Tauri commands | Some async commands execute synchronous persistence work directly |
| PERF-020 | P2 | Frontend assets | The entry chunk and global stylesheet include almost every feature |
| PERF-021 | P3 | Release size | Release executables retain symbols |
| PERF-022 | P3 | Build | Desktop and CLI builds duplicate feature-specific compilation |
| PERF-023 | P3 | Release CI | Windows waits for the complete macOS release job |
| PERF-024 | P3 | Performance gates | CI has no latency, memory, list-size, or artifact-size budgets |

## P1 findings

### PERF-001 — Startup repeatedly parses retained activity logs

Native setup emits `system.app_started` synchronously
([`src-tauri/src/lib.rs:50`](../src-tauri/src/lib.rs#L50)). The emit persists the
record and runs cleanup
([`src-tauri/src/activity_log.rs:98`](../src-tauri/src/activity_log.rs#L98)).
The first process emission causes `maybe_cleanup` to calculate complete retained
log statistics. `stats_unlocked` opens every segment and parses every JSONL
entry
([`activity_log.rs:545`](../crates/mallard-infrastructure/src/activity_log.rs#L545)).

The frontend then loads logs unconditionally even though the drawer starts
closed
([`ProjectSync.tsx:671`](../src/components/project-sync/ProjectSync.tsx#L671)).
The command runs cleanup and then the query. The query parses every retained
record, collects all matches, and sorts them
([`activity_log.rs:318`](../crates/mallard-infrastructure/src/activity_log.rs#L318)).

A normal launch can therefore parse the retained history at least three times:

1. native startup cleanup statistics;
2. query-time cleanup statistics; and
3. query filtering and sorting.

The default retention budget is 100 MiB. Configuration permits up to 10 GiB.
Launch cost is proportional to retained history even if the user never opens
the drawer.

The frontend also keeps `LogPanel` mounted while closed
([`ProjectSync.tsx:3921`](../src/components/project-sync/ProjectSync.tsx#L3921))
and renders all loaded rows
([`LogPanel.tsx:360`](../src/components/LogPanel.tsx#L360)). Each matching live
record rebuilds a map, flattens it, sorts retained rows, and updates root state.
The retained live list can contain 2,000 records.

**Required direction**

- Do not run complete statistics synchronously during Tauri setup.
- Enforce byte retention from file metadata when possible.
- Run record-level cleanup asynchronously and only when due.
- Do not run cleanup before every query.
- Read newest segments first and stop after `limit + 1` for unfiltered queries.
- Load and mount the drawer on first open.
- Keep closed-drawer state to an unread count and a small bounded queue.
- Use an append-oriented ring buffer and virtualize the visible log list.

### PERF-002 — Startup serializes at least 13 command phases

`loadShell` executes host identity, configuration, recovery, projects, setup
drafts, storage drafts, Catalog scopes, repository kinds, provider descriptors,
profiles, bindings, path audits, and a second host identity read in sequence
([`ProjectSync.tsx:1004`](../src/components/project-sync/ProjectSync.tsx#L1004)).

Recovery must precede dependent state reads. The final identity read protects
against a changed host. Many reads after recovery are independent. Their
latencies are currently added together. The shell also commits intermediate
state several times before it starts the initial project load
([`ProjectSync.tsx:1224`](../src/components/project-sync/ProjectSync.tsx#L1224)).

**Required direction**

- Preserve host verification and recovery ordering.
- Start independent post-recovery reads with `Promise.allSettled`.
- Adopt results once per startup phase.
- Run path audits after the shell is usable.
- Consider one revision-consistent `load_shell_snapshot` backend command.
- Start inventory and sibling-state reads concurrently after project detail.
- Record real stage durations. Do not treat fixed progress percentages as
  performance data.

### PERF-003 — Codex assignment audits rescan the complete provider home

Every Activity request invokes the assignment audit
([`service.rs:1726`](../src-tauri/src/project_sync/service.rs#L1726)). The audit
can parse a 32 MiB global-state file and walk up to 20,000 JSONL files under
`sessions` and `archived_sessions`
([`service.rs:14794`](../src-tauri/src/project_sync/service.rs#L14794)). It
compares each filename with assigned thread IDs and reads a metadata head from
every eligible candidate.

Startup also starts an audit for every active binding
([`ProjectSync.tsx:1138`](../src/components/project-sync/ProjectSync.tsx#L1138)).
Several projects can use the same canonical Codex home. In that case, startup
can scan the same tree once per project in parallel.

Normal provider discovery already has watcher-backed reuse
([`session_watch_service.rs:719`](../src-tauri/src/project_sync/session_watch_service.rs#L719)).
The assignment audit bypasses it.

**Required direction**

- Put assignment classification in the shared watcher-backed discovery
  service.
- Build one session-ID-to-path index per canonical provider home.
- Cache parsed global state by size, modification time, and watcher generation.
- Pre-index nominated thread IDs.
- Share one audit result between projects that use the same home.
- Run a complete repair audit only after relevant watcher changes or an
  explicit Audit action.

### PERF-004 — Push can retain more than 1.5 GiB of payload copies

Provider capture allows 512 MiB of aggregate content
([`provider_capture.rs:20`](../crates/mallard-infrastructure/src/provider_capture.rs#L20)).
Captured files retain `Vec<u8>` payloads
([`provider_capture.rs:435`](../crates/mallard-infrastructure/src/provider_capture.rs#L435)).
Publish request construction clones those payloads
([`service.rs:8019`](../src-tauri/src/project_sync/service.rs#L8019)). The bundle
engine clones request buffers again into upload `Bytes`
([`bundle_engine.rs:688`](../crates/mallard-infrastructure/src/bundle_engine.rs#L688)).
The original capture remains alive because publication receives it by
reference
([`service.rs:9567`](../src-tauri/src/project_sync/service.rs#L9567)).

A near-limit capture can therefore retain the original 512 MiB, a publication
copy, and an upload copy. This excludes redaction, manifests, SDK buffers, and
allocator overhead.

**Required direction**

- Consume captured payloads by value during publication.
- Use `Bytes`, `Arc<[u8]>`, or private staged files from capture through upload.
- Calculate and retain the digest during capture.
- Make redaction and certificate transformations consume buffers.
- Stream large objects from staged files.
- Keep the aggregate limit as a defensive boundary, not as the main memory
  control.

### PERF-005 — Bundle pagination restarts namespace discovery

Every `list_remote_bundles` call begins without an object cursor and exhausts
the complete repository namespace
([`bundle_engine.rs:1179`](../crates/mallard-infrastructure/src/bundle_engine.rs#L1179)).
It reads remaining bundle tags one at a time, sorts all summaries, and only then
truncates the result. The service invokes this function in 100-bundle pages and
then inspects every returned bundle
([`service.rs:3377`](../src-tauri/src/project_sync/service.rs#L3377)).

For `B` bundles, service pagination performs approximately `ceil(B / 100)`
complete object-namespace scans. Tag reads also repeat. Local listing makes the
problem worse because each page performs a complete `WalkDir` and sort
([`local_store.rs:324`](../crates/mallard-infrastructure/src/storage_core/local_store.rs#L324)).

**Required direction**

- Add a flat, listable bundle index or tag-only namespace.
- Carry the object-store cursor through bundle pagination.
- Use one bounded discovery pass when the caller requires all bundles.
- Read tag metadata with bounded concurrency.
- Use one ordered local walk or a maintained index.

### PERF-006 — Library inventory retains the complete installed corpus

Each Library scope list, Push review, Pull review, and apply resolves a fresh
local inventory
([`storage_catalog/service.rs:854`](../src-tauri/src/storage_catalog/service.rs#L854)).
Inventory runs provider plugin discovery, reads every skill file, and retains
the complete captured tree in each inventory record
([`skill_tree.rs:126`](../src-tauri/src/storage_catalog/skill_tree.rs#L126)). It
then traverses each tree again to calculate the latest modification time.

An individual skill is limited to 16 MiB. A portable revision is limited to 512
resources and 256 MiB. `inventory_local` does not enforce an aggregate count or
byte limit before it returns the complete local inventory.

**Required direction**

- Split inventory into summary and selected-payload phases.
- Retain descriptors, digests, sizes, warnings, and metadata during listing.
- Capture payload bytes only for selected Push resources.
- Enforce aggregate local inventory count and byte budgets.
- Cache plugin discovery and skill digests behind watchers.
- Record the newest modification time during the capture traversal.

### PERF-007 — Activity scales poorly in both Rust and React

The backend can load 10,000 first-parent commits before it applies the requested
time window
([`project_activity.rs:1228`](../crates/mallard-infrastructure/src/project_activity.rs#L1228)).
For each visible session, mapping scans a commit prefix or the complete commit
list
([`project_activity.rs:1598`](../crates/mallard-infrastructure/src/project_activity.rs#L1598)).
The mapping cost is `O(sessions * commits)`.

The frontend retains every loaded page, reduces and merges all prior pages
again, and renders every accumulated commit and session
([`ProjectActivityPage.tsx:2034`](../src/components/project-sync/ProjectActivityPage.tsx#L2034)).
Opening session detail hides the Activity overview instead of unmounting it
([`ProjectLinksWorkspace.tsx:2701`](../src/components/project-sync/ProjectLinksWorkspace.tsx#L2701)).

A production server-render measurement of the actual commit component produced:

| Commit rows | Render time | Markup bytes | Heap increase |
| ---: | ---: | ---: | ---: |
| 100 | 25.8 ms | 135,283 | 2.4 MiB |
| 1,000 | 68.7 ms | 1,351,183 | 14.2 MiB |
| 5,000 | 423.8 ms | 6,763,183 | 42.5 MiB |
| 10,000 | 1.12 s | 13,528,183 | 168.4 MiB |

WebView layout and paint are not included in these values.

**Required direction**

- Cache the immutable Git rail by ref SHA.
- Apply the requested time window before returning data.
- Use binary search or an offline sweep for session-to-commit mapping.
- Return bounded Activity pages.
- Maintain an incremental frontend merge index.
- Virtualize commit and session rows.
- Render only a bounded page window.
- Unmount or freeze the hidden overview tree.

### PERF-008 — The non-Git Files view has no render bound

Search filters the complete inventory on every update
([`ProjectFilesActivityPage.tsx:228`](../src/components/project-sync/ProjectFilesActivityPage.tsx#L228)).
The page then performs local/cloud filtering, directory grouping, and sorting.
Every matching file becomes a card
([`ProjectFilesActivityPage.tsx:275`](../src/components/project-sync/ProjectFilesActivityPage.tsx#L275)).

Push/Pull file review already caps card rendering at 500
([`ProjectFilesReviewPage.tsx:58`](../src/components/project-sync/ProjectFilesReviewPage.tsx#L58)).
The Files activity page does not use that bound.

**Required direction**

- Apply the existing 500-row and search policy immediately.
- Memoize local/cloud directory grouping.
- Use `useDeferredValue` for search input.
- Add backend paging or a virtualized grouped list if large inventories are a
  normal case.

## P2 findings

### PERF-009 — Session detail reloads and invalidates complete timelines

Every timeline page can read and deserialize the complete cache document
([`session_detail.rs:909`](../crates/mallard-infrastructure/src/session_detail.rs#L909)).
A cache hit clones the complete session index
([`session_detail.rs:1006`](../crates/mallard-infrastructure/src/session_detail.rs#L1006)),
although one page returns only 50 events.

On the frontend, timeline grouping and turn spans are recomputed for global
expansion and detail changes
([`SessionDetailTimeline.tsx:57`](../src/components/project-sync/session-detail/SessionDetailTimeline.tsx#L57)).
Every row receives the complete expansion set, detail map, and project-image
map. One image load can invalidate memoized Markdown in all loaded events
([`SessionDetailEvent.tsx:106`](../src/components/project-sync/session-detail/SessionDetailEvent.tsx#L106)).

Keep active indexes in an `Arc`-based process cache. Treat disk state as a
cold-start cache. Pass row-local state, memoize groups and events, use one
`Intl.DateTimeFormat`, and use per-path image subscriptions. Window old pages
when a session becomes long.

### PERF-010 — Conversation changes rewrite config once per thread

Checkpoint maintenance calls a full config mutation for every changed thread
([`service.rs:5334`](../src-tauri/src/project_sync/service.rs#L5334)).
Metadata-only Pull also applies one thread at a time. Each mutation can read,
validate, pretty-serialize, atomically replace, and directory-sync an 8 MiB
document
([`persistence.rs:2696`](../crates/mallard-infrastructure/src/persistence.rs#L2696)).

Collect validated changes and apply them in one transaction. Return per-thread
outcomes. Use bounded chunks only when failure isolation requires them.

### PERF-011 — Library publication revalidates inherited payloads sequentially

After new uploads, publication loops through every inherited payload and calls
`stage_verified` one at a time
([`resource_engine.rs:293`](../src-tauri/src/storage_catalog/resource_engine.rs#L293)).
Changing one resource can download, stage, sync, and hash all retained files.

Carry inherited references from the already verified previous manifest.
Revalidate all existing content only during an explicit repair or audit. If the
protocol must verify all content, use bounded `stage_many_verified` work.

### PERF-012 — Storage clear deletes objects sequentially

`clear_managed_prefix` lists 1,000 keys and deletes each key separately
([`storage_core/mod.rs:130`](../crates/mallard-infrastructure/src/storage_core/mod.rs#L130)).
The safety limit is one million objects. Local storage also restarts a complete
tree walk for each page.

Add an exact-key `delete_many` operation. Use S3 `DeleteObjects` batches of up
to 1,000 keys with bounded concurrency. Perform one validated local walk and
delete only the exact resulting keys.

### PERF-013 — Cache writes occur under a global mutex

`store_observations` holds the session-service mutex during canonicalization,
metadata pinning, map updates, complete disk-cache assembly, serialization, and
atomic publication
([`session_watch_service.rs:1071`](../src-tauri/src/project_sync/session_watch_service.rs#L1071)).
The cache can reach 16 MiB.

Perform path and metadata work before the lock. Update memory and take an
immutable snapshot under the lock. Write after release. Add a dirty check and a
single-flight debounced writer.

### PERF-014 — S3 clients are recreated on command paths

Project storage and Library storage construct configured object stores for
each command path
([`project_sync/service.rs:12640`](../src-tauri/src/project_sync/service.rs#L12640),
[`storage_catalog/service.rs:65`](../src-tauri/src/storage_catalog/service.rs#L65)).
`S3ObjectStore::from_config` builds a new HTTPS client
([`s3_store.rs:1222`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1222)).

The connection impact is inferred from client lifecycle. Confirm it with DNS,
TLS, and connection-reuse metrics. Cache clients by a safe storage fingerprint.
Do not put credential values in cache keys or logs.

### PERF-015 — Stored pet previews load serially

The Catalog collects remote-only pets, awaits each preview one at a time, and
updates page state after each result
([`StorageCatalogPage.tsx:937`](../src/components/storage-catalog/StorageCatalogPage.tsx#L937)).

Use a batch backend command or three to four concurrent reads. Commit completed
results in batches. Start with viewport-visible cards.

### PERF-016 — Storage discovery can run for each keystroke

The setup effect depends directly on the pending local path and bucket
([`ProjectSetupWorkspace.tsx:354`](../src/components/project-sync/ProjectSetupWorkspace.tsx#L354)).
Effect cleanup ignores a stale response but does not cancel backend work.

Debounce discovery. Start only after the storage identity is valid. Prefer
Browse completion, blur, or an explicit Check repositories action. Add a
request token if the backend can stop superseded work.

### PERF-017 — Root state causes broad React reconciliation

Push selection, live logs, transfer progress, sidebar width, and log height are
owned by `ProjectSync`. A checkbox or pointer move can reconcile the shell and
the active workspace. Push review can render 500 file cards.

Examples include push selection
([`ProjectSync.tsx:401`](../src/components/project-sync/ProjectSync.tsx#L401)),
sidebar resizing
([`ProjectSync.tsx:779`](../src/components/project-sync/ProjectSync.tsx#L779)),
and log resizing
([`ProjectSync.tsx:744`](../src/components/project-sync/ProjectSync.tsx#L744)).

Keep transient state in the owning feature. For splitters, update a CSS custom
property at most once per animation frame and commit React state on pointer
release. Memoize file rows with primitive, row-local props and stable handlers.

### PERF-018 — Push review can perform two continuity probes

When the remote revision is ahead, review calculates continuity and then calls
a destination classifier that can calculate it again
([`service.rs:7299`](../src-tauri/src/project_sync/service.rs#L7299)). The probe
reads and verifies a historical manifest directly from storage
([`bundle_engine.rs:1132`](../crates/mallard-infrastructure/src/bundle_engine.rs#L1132)).

Calculate continuity once. Pass the resulting evidence to destination
classification.

### PERF-019 — Some async commands execute synchronous persistence work

The command layer has a blocking worker helper
([`commands.rs:19`](../src-tauri/src/project_sync/commands.rs#L19)), but several
configuration, project, binding, and draft commands call synchronous repository
methods directly. `get_project` can also reload configuration for detail,
bindings, and materialization state
([`service.rs:12766`](../src-tauri/src/project_sync/service.rs#L12766)).

Route filesystem and persistence work through the blocking pool. Add one
repository snapshot method that reads related documents once under one guard.

### PERF-020 — Initial assets include almost every feature

The production entry JavaScript is 615,312 bytes. Vite emits its chunk-over-
500-kB warning. Session detail is the only major workspace loaded with
`lazy()`
([`ProjectLinksWorkspace.tsx:197`](../src/components/project-sync/ProjectLinksWorkspace.tsx#L197)).
`ProjectSync` eagerly imports Settings, Activity, setup, Push, Pull, storage,
and Catalog surfaces
([`ProjectSync.tsx:45`](../src/components/project-sync/ProjectSync.tsx#L45)).

`App.css` is imported unconditionally
([`App.tsx:10`](../src/App.tsx#L10)). It contains 293,369 source bytes, 10,617
lines, 1,599 rules, and 5,581 declarations. The production CSS is 180,867
bytes.

Lazy-load Settings, Catalog, Add Catalog, setup, storage management, Push, and
Pull at existing view boundaries. Move pure helpers out of component modules.
Move feature CSS with each deferred feature. Keep shell, tokens, typography,
and shared review primitives in the entry stylesheet.

## P3 build and release findings

### PERF-021 — Release executables retain symbols

The workspace has no explicit `[profile.release]`. A temporary post-link strip
test produced:

| Executable | Current | Stripped test | Raw reduction |
| --- | ---: | ---: | ---: |
| `mallard` | 38.36 MB | 30.13 MB | 8.23 MB |
| `mallard-cli` | 14.69 MB | 11.32 MB | 3.37 MB |
| Combined | 53.05 MB | 41.45 MB | 21.9% |

Add `strip = "symbols"` to `[profile.release]`. Measure `lto = "thin"` before
adopting it because it increases link time.

### PERF-022 — Desktop and CLI builds duplicate compilation work

The wrapper builds the CLI and desktop app as separate Cargo operations
([`tauri-build.mjs:87`](../scripts/tauri-build.mjs#L87)). They request different
feature sets for several large shared dependencies. After the desktop release
build, the CLI build still took 62.75 seconds.

Measure a joint Cargo build so shared features are unified once. Preserve
sidecar staging and the current Tauri bundle overlay. Use Cargo timing reports
before changing the wrapper.

### PERF-023 — Windows waits for macOS release completion

The Windows job has `needs: publish_macos`
([`release.yml:71`](../.github/workflows/release.yml#L71)). Windows dependency
installation, compilation, installer creation, verification, and upload cannot
start until macOS has completed.

Create the release draft in a small preparation job. Run macOS and Windows
builds in parallel. Use a final job that requires both verified artifact sets.

### PERF-024 — No automated performance budgets exist

The repository has useful work-unit counters in
[`sync_metrics.rs`](../crates/mallard-infrastructure/src/sync_metrics.rs), but
it has no `benches/` directory, Criterion suite, WebView performance suite, or
CI performance budget.

The historical
[`PERF_SYNC_PATH_MEASUREMENTS.md`](./PERF_SYNC_PATH_MEASUREMENTS.md) contains
useful manual evidence. It is not executable and does not define variance,
percentiles, or regression thresholds.

Add automated checks for:

- process launch to usable shell;
- startup with 0, 10, and 100 MiB of activity logs;
- startup IPC stage duration;
- entry JavaScript and CSS size;
- large Activity, Files, Library, log, and session-detail views;
- Push and Library peak memory;
- cold and warm local and S3 storage operations;
- executable and installer size; and
- cold, changed, and no-op build duration.

## Measured baseline

### Build and test measurements

| Measurement | Result |
| --- | ---: |
| Strict TypeScript check | 3.49 s real |
| Vite production build | 1.43 s real; 1.00 s reported by Vite |
| Vite modules transformed | 352 |
| Frontend integration suite | 251 of 251 passed |
| Frontend integration command | 4.63 s real |
| TAP test execution | 372.9 ms |
| Cold native release build | 3 min 33 s |
| Changed-app release rebuild | 44.74 s |
| True no-op app release build | 0.68 s |
| CLI release build after the app build | 62.75 s |
| True no-op CLI release build | 0.45 s |
| Active macOS Cargo graph | 360 unique versioned packages |
| Installed `node_modules` | 226 MiB |
| Generated `target` after release measurements | 11 GiB |

The full Rust test suite was not run. A native release build completed. Focused
Rust performance-invariant tests passed:

- two warm comparison read tests passed with 0.75 seconds of test-body time;
- `reviews_capture_only_byte_pinning_candidates` passed with 1.19 seconds of
  test-body time; and
- `parallel_capture_is_worker_count_invariant` passed with 0.04 seconds of
  test-body time.

The timing wrapper could not read `kern.clockrate` in the sandbox and returned
shell status 1. The wrapped TypeScript, Vite, Cargo, and frontend test commands
completed successfully.

### Production frontend assets

| Asset | Raw | gzip level 9 | Brotli |
| --- | ---: | ---: | ---: |
| Main JavaScript | 615,312 B | 177,566 B | 147,131 B |
| Global CSS | 180,867 B | 29,092 B | 23,657 B |
| Lazy session-detail JavaScript | 178,503 B | 54,229 B | 47,496 B |
| Font | 22,496 B | WOFF2 | WOFF2 |
| Logo | 1,151 B | not measured | not measured |
| HTML | 468 B | approximately 300 B | not measured |

The initial JavaScript and CSS total 796,179 raw bytes and 206,658 gzip bytes.
Because Mallard is packaged with Tauri, network transfer is not the main cost.
The WebView must still read, parse, compile, and retain the uncompressed code.

### Environment qualification

`npm ls --all` reported `ELSPROBLEMS`. The installed `@types/node` is
`20.17.28`, while `package-lock.json` resolves `20.19.19`. TypeScript, Vite, and
all 251 frontend integration tests passed. Use a clean `npm ci` environment
before recording a committed asset hash or formal build baseline.

## Recommended implementation sequence

### Milestone 1 — Bound startup and visible lists

1. Remove synchronous full-history activity-log scans from startup.
2. Lazy-load and mount the log drawer.
3. Parallelize or consolidate the post-recovery shell snapshot.
4. Defer Codex assignment audits and share one index per provider home.
5. Add the existing 500-row bound to the non-Git Files page.
6. Bound and virtualize Activity history.

### Milestone 2 — Bound memory and storage work

1. Make Push payload ownership zero-copy or staged-file based.
2. Split Library summary inventory from selected payload capture.
3. Repair bundle discovery pagination.
4. Batch config transactions.
5. Stop re-downloading inherited Library payloads.
6. Add storage bulk deletion.

### Milestone 3 — Improve cache and UI locality

1. Cache session indexes in process and share them with `Arc`.
2. Move disk writes outside service mutexes.
3. Reuse S3 clients after transport measurements confirm the cost.
4. Batch remote previews.
5. Debounce pending storage discovery.
6. Move transient review, log, progress, and splitter state to feature-local
   controllers.
7. Give transcript rows and image nodes narrow subscriptions.

### Milestone 4 — Control build and release growth

1. Split feature JavaScript and CSS.
2. Add entry JavaScript and CSS budgets.
3. Strip release symbols.
4. Measure joint desktop and CLI compilation.
5. Run macOS and Windows release builds in parallel.
6. Add startup, large-list, storage, memory, and artifact-size performance
   gates.

## Safeguards to preserve

- Provider capture performs a bounded preflight and uses a bounded worker
  count.
- S3 object listing uses stable server-side cursors. Higher-level bundle
  discovery currently discards this benefit.
- S3 bulk staging and upload use bounded concurrency and stable result order.
- Bundle fetch and upload report byte progress and use bounded batches.
- Large object verification hashes and copies through bounded readers and
  private temporary files.
- Generic restore planning uses manifest metadata and does not download
  payload bytes.
- Heavy Push, Pull, Activity, session-detail, and Library commands generally
  use the blocking pool.
- Provider discovery can return a clean watcher-backed inventory without
  filesystem work.
- Configuration watching is event-driven and debounced.
- Local persistence is bounded, atomic, no-follow, and durably synchronized.
- Session detail isolates ReactMarkdown and GFM behind a dynamic import.
- Request IDs and epochs prevent stale asynchronous results from replacing
  current state.
- Frontend dependencies are restrained. The app does not use a large general
  UI, chart, or global-state framework.
- Warm Codex and Claude comparison tests enforce no complete transcript
  rereads.

These safeguards protect both performance and safety. Performance changes must
not weaken canonical-path validation, bounded reads, no-follow traversal,
immutable publication, compare-and-swap heads, plan approval, digest checks,
backups, receipts, or exact deletion rules.
