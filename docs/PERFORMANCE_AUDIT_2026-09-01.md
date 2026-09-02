# Mallard performance audit — measured against a production provider home

- **Audit date:** 2026-09-01
- **Application version:** 0.1.2
- **Source baseline as audited:** `f2cdac4` plus the then-current working tree
- **Status revised at:** `789eea5` (see "Status" below — PERF-025 and PERF-031
  are resolved, and the PERF-017 `logLines` case is resolved)
- **Scope:** provider discovery and metadata scanning, the inventory and
  timeline caches, session-detail IPC, bundle transfer scheduling, React
  render locality, frontend bundle composition, and the Cargo release profile
- **Purpose:** re-run the performance question against a real Codex and
  Claude home, which the 2026-08-30 audit explicitly could not do, and record
  what only a production-sized corpus reveals

This report was written against `f2cdac4`. Three commits landed immediately
afterwards, two of them acting on these findings. The "Status" section below
records what is resolved; the finding bodies are kept as the record of why
each change was made, and are not rewritten.

## Status at `789eea5`

| ID | Status | Landed in |
| --- | --- | --- |
| PERF-025 | **Resolved.** Both changes shipped: the narrow-record parse and a 256 KiB tail window with a soundness condition and full-stream fallback. Design and measurements in [`DESIGN_CLAUDE_SCAN_TAIL_WINDOW.md`](DESIGN_CLAUDE_SCAN_TAIL_WINDOW.md). | `789eea5` |
| PERF-031 | **Resolved.** `session_home_is_project_root` now takes a `BTreeMap` memo threaded through both adapters' discovery loops, so a repeated home canonicalizes once. | `ec82ce1` |
| PERF-017 (`logLines` case) | **Resolved.** `logLines` moved to a new `ActivityLogDrawer`; the listener now writes a ref and notifies subscribers instead of setting root state, and the drawer coalesces arrivals on `requestAnimationFrame`. `ProjectSync` went from 81 `useState` declarations to 74. | `ec82ce1` |
| PERF-026, PERF-027 | Open. | — |
| PERF-028, PERF-029, PERF-030 | Open. | — |
| PERF-032, PERF-033 | Open. | — |
| PERF-009, PERF-013, PERF-020, PERF-021 | Open, as recorded under "Confirmations". | — |

One residual under PERF-017: while the drawer is closed, each log event still
calls `setUnreadLogs` at the root
([`ProjectSync.tsx:600`](../src/components/project-sync/ProjectSync.tsx#L600)).
That is now bounded — the counter saturates at 99 and React then bails on the
identical value — where before it was one unbounded root write per event. Not
worth a change on its own; fold it in if the shell is refactored again.

### Independent corroboration of the PERF-025 result

The audit harness and the implementation's corpus gate are separate programs
written against the same corpus, and their narrow-parse figures agree closely:

| Source | Full-file `Value` parse | Narrow-record parse |
| --- | ---: | ---: |
| This audit's A/B harness | 1.277 s / 1.369 s | 0.541 s / 0.537 s |
| `claude_scan_corpus_parity_and_window_sweep` | 1.778 s | 0.564 s (256 KiB tier) |

The `Value` arms differ because the gate's reference scan does more per record
than the audit's probe; the narrow arms — the number the change is judged on —
land within 5% of each other from independent code. `cargo test -p
mallard-infrastructure` at `789eea5`: 372 passed, 0 failed, 3 ignored.

Worth carrying forward: the window tier reads slightly **more** bytes than a
pure full scan (1104 MB against 1090 MB at 256 KiB), because an undecided file
reads its window and then the full stream anyway. The design doc states this
and keeps the tier for the worst case it bounds rather than for average bytes.
That is the right call, but it means PERF-026 — walking each home once instead
of once per project per pass — is now the finding that reduces total I/O, and
it is still open.

PERF-025's resolution also changes the ownership doctrine, not just the cost:
`authorized_home` (the leaf segment's first recorded home) now decides project
ownership, replacing the `leaf_cwd`/`last_cwd` drift fallback. That is a
correctness change carried by a performance fix, and the corpus gate reports
zero divergences across 106 comparable files. It is worth knowing about when
reading any older document that says ownership follows the last `cwd`.

## Relationship to the 2026-08-30 audit

[`PERFORMANCE_AUDIT_2026-08-30.md`](PERFORMANCE_AUDIT_2026-08-30.md) remains
the primary register. It states under "Audit method and limits" that it did
not use a representative large Codex or Claude home. This audit supplies that
missing input: 12 GB of Codex sessions across 1,792 transcripts and 1.5 GB of
Claude projects across 805 transcripts, on the machine that authors this
repository.

That single difference changes the priority order. The largest cost in a
normal Activity load is not in the 2026-08-30 register at all, because it is
invisible on a small provider home and unmissable on a real one.

This document therefore does two things:

1. it adds PERF-025 through PERF-033, which are new; and
2. it confirms, and where possible re-sizes with measurements, the entries
   from the 2026-08-30 register that this audit's instrumentation touched.

It does not restate findings it merely re-observed. PERF-009, PERF-013,
PERF-017, PERF-020, and PERF-021 are already correctly described there; the
"Confirmations" section below records only that they are still open at
`f2cdac4` and adds numbers where this audit produced them.

## Executive assessment

The safeguards the previous audit credited are real and hold up under a
production corpus: capture is bounded and parallel, the watcher separates
appends from membership changes, transfers use bounded concurrency, and the
observation layer already persists with stat pins.

The dominant cost is one asymmetry between two provider adapters. Codex
resolves a session's owning project from a bounded file head — 128 lines or
1 MiB. Claude reads the complete transcript and parses every record into
`serde_json::Value`, then discards nearly all of it when the ownership
predicate rejects the session. On this corpus that is 1.09 GB read and
237,661 records parsed per discovery pass, to keep the handful of sessions
belonging to the open project.

Two structural multipliers sit on top of it. The inventory cache is
process-local and keyed per project, so the same corpus is walked once per
project and again on every cold start. And the Activity page's two
deliberately parallel commands both enter discovery without coalescing, so a
cold or dirty profile pays the walk twice concurrently.

Below that, one scaling cliff has a calculable arrival date: the timeline
cache evicts on entry count but fails its write on bytes, and the two bounds
disagree by roughly two orders of magnitude.

The recommended first milestone is to make Claude metadata scanning bounded
and to walk each provider home once per process rather than once per project
per pass. Everything else in this register is at least an order of magnitude
smaller.

## Audit method and limits

Measurements were taken with purpose-built harnesses that replicate the exact
loop shape of the production scanners, compiled at `opt-level = 3` and run
against the machine's real provider homes. Reproduction instructions are in
the "Reproduction" section.

Because the shipped dev profile compiles dependencies at `opt-level = 1`, and
because the release profile has no LTO (PERF-021), every Rust figure here is
a **lower bound** on what the shipped application pays.

The audit did not measure React commit times. The frontend cannot run outside
its Tauri webview, so PERF-017 and the frontend half of PERF-009 are reported
through their measured trigger rates and their static fan-out, not through
commit profiles. The React Profiler inside the running application is the
correct instrument for those.

No production R2 account or high-latency link was used. PERF-029 is argued
from scheduling structure and the store's own concurrency gate, not from
observed transfer latency.

One hypothesis was measured and **rejected**; it is recorded under
"Confirmations" so it is not re-proposed later.

## Findings register

| ID | Priority | Area | Finding |
| --- | --- | --- | --- |
| PERF-025 | P1 | Provider history | Claude discovery reads and parses complete transcripts where Codex reads a bounded head — **resolved in `789eea5`** |
| PERF-026 | P1 | Discovery cache | The provider inventory cache is process-local and keyed per project |
| PERF-027 | P1 | Activity | The Activity page's two parallel commands each run a complete discovery |
| PERF-028 | P2 | Session cache | Timeline-cache eviction counts entries while its write limit counts bytes |
| PERF-029 | P2 | Storage transfer | Bundle transfers barrier every N objects over a semaphore that already bounds them |
| PERF-030 | P2 | Session detail | Every session-detail response is serialized twice |
| PERF-031 | P3 | Discovery | Discovery re-canonicalizes constant paths once per session file — **resolved in `ec82ce1`** |
| PERF-032 | P3 | Project content | The ignore matcher allocates a memo map and a format string per path component |
| PERF-033 | P3 | Persistence | Machine-only caches are pretty-printed |

## P1 findings

### PERF-025 — Claude discovery reads complete transcripts

> **Resolved in `789eea5`.** Both directions below shipped. The text is kept
> as the record of the problem and the measurements that motivated it.

`scan_claude_session_metadata`
([`claude/mod.rs:35`](../crates/mallard-infrastructure/src/providers/claude/mod.rs#L35))
reads every line of a Claude transcript and runs
`serde_json::from_slice::<Value>` on each one, feeding
`ClaudeSessionIdentityTracker::observe`. Discovery calls it for every `.jsonl`
found under `~/.claude/projects` at depth 2 to 3
([`claude/discovery.rs:75`](../crates/mallard-infrastructure/src/providers/claude/discovery.rs#L75))
before `session_home_is_project_root` decides whether the session belongs to
the open project at all.

The Codex adapter answers the same question from a bounded head —
`MAX_METADATA_LINES` of 128 and `MAX_METADATA_BYTES` of 1 MiB
([`provider_capture.rs:1684`](../crates/mallard-infrastructure/src/provider_capture.rs#L1684)).
Claude's bound is the conversation file limit, which defaults to 100 MiB, and
`MAX_CLAUDE_RECORDS` of 100,000.

Measured on 113 files at the depth the walker covers:

| Scan | Bytes read | Records parsed | Wall |
| --- | ---: | ---: | ---: |
| Claude, as implemented | 1.09 GB | 237,661 | 2.23 s cold, 1.28 s warm |
| Same files, bounded head | 3.34 MB | 358 | 7.8 ms |

The reason the full read exists is real: Claude's leaf `cwd` is per-turn
shell state, so the authoritative home is the last chain record's, not the
first record's. That rules out a head read. It does not rule out a bounded
read.

**Required direction**

Two independent changes, in ascending order of risk.

1. Replace `Value` with a narrow `#[derive(Deserialize)]` struct carrying only
   the fields `observe` reads — `sessionId`, `cwd`, `type`, `isSidechain`,
   `isMeta`, `uuid`, `parentUuid`. serde skips unknown fields without
   allocating them, so the large `content` arrays never become trees. Measured
   A/B on this corpus, identical hit counts (173,461 in both arms):

   | Arm | Round 0 | Round 1 |
   | --- | ---: | ---: |
   | `serde_json::Value` | 1.277 s | 1.369 s |
   | Narrow struct | 0.541 s | 0.537 s |

   That is 2.4x for a change with no semantic effect.

2. Bound the read from the tail. The file is append-only JSONL and
   [`jsonl_tail_io.rs:98`](../crates/mallard-infrastructure/src/jsonl_tail_io.rs#L98)
   already implements `read_tail_window`. Walking records backwards satisfies
   `leaf_identity()` as soon as a chain record and one session-id transition
   are in hand. The correctness obligation is the fallback: when the window
   holds a single session id throughout, `leaf_session_home` still depends on
   the earliest chain record carrying that id, so that case must widen the
   window or fall back to the current full read. Land change 1 first; it is
   unconditional.

### PERF-026 — The inventory cache is process-local and keyed per project

`discovery_for`
([`session_watch_service.rs:724`](../src-tauri/src/project_sync/session_watch_service.rs#L724))
caches on `(provider, home, local_project_id)`, and `discover_full` applies
the project-root filter *inside* the walk
([`provider_capture.rs:993`](../crates/mallard-infrastructure/src/provider_capture.rs#L993)).
The same provider corpus is therefore walked once per project rather than
walked once and partitioned by `cwd`.

Nothing is persisted. Every cold start repeats the complete walk, even though
the observation layer one level down already writes through to
`observation_cache.json` with `(size, mtime_nanos)` pins plus generation and
revision — the change that took cold comparison from 22.7 s to 3.1 s.

The watcher's dirty classification is correct and should not change: an
append to a known `.jsonl` marks only `dirty_roots`
([`session_watch_service.rs:556`](../src-tauri/src/project_sync/session_watch_service.rs#L556)),
not `inventory_dirty`. But `Create` on a `.jsonl`, or any path not already in
`known_roots`, does set `inventory_dirty` — which is what starting a new
Claude Code or Codex session does. In normal use the inventory cache is
invalidated many times a day.

Measured cost of one complete pass on this machine: 2.23 s for the Claude leg
plus 1.39 s for the Codex leg, about 3.6 s, repeated per project.

**Required direction**

- Key the cached inventory on `(provider, home)` alone and apply the
  project-root filter when serving, so one walk answers every project.
- Persist it with the same evidence shape the observation cache already uses.
  A discovery record is provable by the same stat pins plus the profile
  generation.
- Keep the existing dirty classification unchanged.

### PERF-027 — Activity's two parallel commands each run a complete discovery

`get_project_activity` and `get_project_thread_sync_comparison` are fired in
parallel by `loadComparison` so that either result can stand alone if the
other fails. Both call `load_project_local_read`
([`service.rs:6306`](../src-tauri/src/project_sync/service.rs#L6306)), which
calls `discovery_for`.

`discovery_for` releases the service mutex before invoking `discover_full`.
On a cold or dirty profile the two commands therefore perform the same
multi-second walk concurrently and race to publish the same cache entry.

The split itself was the right call. Without coalescing it doubles the work
it was introduced to overlap.

**Required direction**

Single-flight the walk. Hold an in-flight marker in `ServiceInner` keyed by
`(provider, home)`; a second caller waits on the first result rather than
starting its own. This composes with PERF-026: one walk then serves both
commands and every project bound to that home.

## P2 findings

### PERF-028 — Timeline-cache eviction counts entries; its write limit counts bytes

Every timeline index build rewrites the complete
`session_timeline_cache.json`, pretty-printed and fsynced
([`session_detail.rs:1089`](../crates/mallard-infrastructure/src/session_detail.rs#L1089)).

The bounds disagree. `MAX_TIMELINE_CACHE_ENTRIES` is 50,000 and triggers the
reset-and-warn path. `MAX_TIMELINE_CACHE_BYTES` is 32 MiB and is enforced by
`write_bytes_atomic`
([`persistence.rs:2768`](../crates/mallard-infrastructure/src/persistence.rs#L2768)),
which returns an error rather than evicting.

Measured on this machine:

| Property | Value |
| --- | ---: |
| Current file | 2,084,709 bytes |
| Entries | 25 |
| Mean entry | ~83 KB |
| Largest entry | 344,909 bytes |
| Compact re-encode | 1,206,136 bytes (pretty costs 1.73x) |

At the observed mean, 32 MiB arrives at roughly 385 cached sessions — two
orders of magnitude below the entry cap that is supposed to fire first. Past
that point every write fails, the caller pushes only an info-severity warning
([`session_detail.rs:1097`](../crates/mallard-infrastructure/src/session_detail.rs#L1097)),
and no new session is ever cached again. Session detail degrades to
rebuilding every index on every open, silently and permanently.

**Required direction**

- Evict on serialized size, not entry count: drop least-recently-used entries
  until the document fits well under the hard limit.
- Use `write_json_atomic_compact` for this file. No human reads it, and pretty
  printing costs a measured 1.73x here.
- Consider one file per session under a directory, which turns a 2 MB rewrite
  into an 83 KB one and removes the shared-document failure mode entirely.

This is distinct from PERF-009, which covers reload and clone cost on the read
path. Both are open.

### PERF-029 — Transfers barrier every N objects over a semaphore that already bounds them

Both transfer loops chunk the pending set by `cloud_transfer_concurrency()`
and await each chunk before starting the next
([`bundle_engine.rs:341`](../crates/mallard-infrastructure/src/bundle_engine.rs#L341),
[`bundle_engine.rs:925`](../crates/mallard-infrastructure/src/bundle_engine.rs#L925)).

`stage_many_verified` and `put_many` already gate themselves with a
`tokio::sync::Semaphore` of exactly that size
([`s3_store.rs:984`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L984),
[`s3_store.rs:1051`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1051),
[`s3_store.rs:1122`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs#L1122)).

The outer chunking contributes nothing except a barrier. At each chunk
boundary the transfer window drains to zero and refills, so the semaphore
never holds N transfers in flight continuously. With variable object sizes
each chunk costs its slowest member rather than its mean. The comment at
[`bundle_engine.rs:337`](../crates/mallard-infrastructure/src/bundle_engine.rs#L337)
already observes that the two gates are min-coupled, which is the signal that
one of them is redundant.

**Required direction**

Pass the complete `pending` slice to the store and report progress as each
object resolves. The store's semaphore is the correct and sufficient gate.
Preserve the existing first-failure abort-and-drain behaviour.

Confirm the improvement against a real R2 endpoint before claiming a figure;
this finding is structural, not measured.

### PERF-030 — Session-detail responses are serialized twice

`ensure_response_bound`
([`session_detail.rs:1324`](../crates/mallard-infrastructure/src/session_detail.rs#L1324))
calls `serde_json::to_vec(&value)`, measures the length, and drops the buffer.
Tauri then serializes the same value again crossing the IPC boundary.

It guards the three hottest session-detail reads: the detail itself at a
1 MiB bound ([`session_detail.rs:1631`](../crates/mallard-infrastructure/src/session_detail.rs#L1631)),
a timeline page at 16 MiB ([`session_detail.rs:1754`](../crates/mallard-infrastructure/src/session_detail.rs#L1754)),
and an event detail at 8 MiB ([`session_detail.rs:1846`](../crates/mallard-infrastructure/src/session_detail.rs#L1846)).
For a 50-event page carrying large tool bodies, that is megabytes of encode
work performed only to be discarded.

**Required direction**

Either return the bytes that were measured — a `serde_json::value::RawValue`,
or a raw `tauri::ipc::Response` — so the measurement *is* the payload; or
serialize into a counting sink that aborts at the limit, which removes the
allocation and short-circuits an oversized response instead of building it in
full first. The second option also improves the failure path, which currently
constructs the whole over-limit document before rejecting it.

## P3 findings

### PERF-031 — Discovery re-canonicalizes constant paths per session file

> **Resolved in `ec82ce1`**, ahead of the recommendation below — the `cwd`
> memo landed as part of the PERF-025 work rather than opportunistically.

`inspect_candidate_files_with_limit` calls `fs::canonicalize(approved_root)`
on every invocation
([`provider_capture.rs:1808`](../crates/mallard-infrastructure/src/provider_capture.rs#L1808)),
and both adapters invoke it once per discovered session with the already
canonical `canonical_home`. Separately, `session_home_is_project_root`
canonicalizes each session's `cwd`
([`provider_capture.rs:1747`](../crates/mallard-infrastructure/src/provider_capture.rs#L1747))
with no memo, although many sessions share a `cwd`.

Measured: 675 us for 113 paths and 24.7 ms for 1,792 paths, about 6 to 14 us
each. Across roughly 1,900 sessions plus their `cwd` lookups this is on the
order of 22 ms per pass.

Sized honestly, this is under 1% of PERF-025. Hoist the root canonicalization
to the caller and memoize the `cwd` map when touching this code; it does not
justify a dedicated change.

### PERF-032 — The ignore matcher allocates per path component

`glob_path_matches`
([`provider_capture.rs:1635`](../crates/mallard-infrastructure/src/provider_capture.rs#L1635))
allocates a fresh `BTreeMap` memo on every call and inserts into it even for
patterns containing no wildcard, where the memo can never hit.
`ignore_rule_matches` builds a `format!("{}/", ...)` per rule miss, and
`project_path_is_ignored`
([`provider_capture.rs:1593`](../crates/mallard-infrastructure/src/provider_capture.rs#L1593))
clones a growing prefix `String` per path component for directory-only rules,
and continues iterating rules after a match even when no negation follows.

The blast radius is bounded because `hard_excluded_project_content_path`
prunes `.git`, `node_modules`, `target`, `dist`, and `build` with
`skip_current_dir` before any rule runs
([`provider_capture.rs:1514`](../crates/mallard-infrastructure/src/provider_capture.rs#L1514)).
Fix opportunistically: take a literal fast path when the pattern has no `*` or
`?`, and reuse one scratch buffer for prefixes.

### PERF-033 — Machine-only caches are pretty-printed

`write_json_atomic`
([`persistence.rs:2729`](../crates/mallard-infrastructure/src/persistence.rs#L2729))
uses `to_vec_pretty` for every state file. A `write_json_atomic_compact`
already exists beside it and is used in one place
([`persistence.rs:514`](../crates/mallard-infrastructure/src/persistence.rs#L514)).

Measured overhead on the two machine-only caches:

| File | On disk | Compact | Overhead |
| --- | ---: | ---: | ---: |
| `session_timeline_cache.json` | 2,084,709 | 1,206,136 | 1.73x |
| `observation_cache.json` | 391,444 | 287,448 | 1.36x |

Keep pretty printing for files a person may inspect — `sync_config.json`,
`settings.json`, the drafts. Switch the caches, which are rewritten whole on
hot paths.

## Confirmations from the 2026-08-30 register

These were verified still open at `f2cdac4`. They are not restated; see
[`PERFORMANCE_AUDIT_2026-08-30.md`](PERFORMANCE_AUDIT_2026-08-30.md).

| ID | Status at `f2cdac4` | Measurement added by this audit |
| --- | --- | --- |
| PERF-009 | Open. `SessionDetailTimeline.tsx:57-58` still derives groups and turn spans in the render body with no `useMemo`; `SessionDetailEvent` is still unmemoized. | `SessionDetailMarkdown` *is* memoized and its `projectImages` prop is `useState`-stable, so markdown bodies are parsed once in the common case. The residual cost is row chrome, an order of magnitude below the parse it already avoids. |
| PERF-013 | Open. `store_observations` still takes `self.inner` and calls `write_observation_disk_cache` inside that scope. | The document it serializes under the lock is 391,444 bytes today against a 16 MiB bound. |
| PERF-017 | Open. | `ProjectSync` holds **81** `useState` declarations, 22 `useEffect`, 7 `useCallback`, and exactly one `useMemo`. Across 45 component files exactly **two** are wrapped in `React.memo`: `LogRow` and `SessionDetailMarkdown`. The specific `logLines` case is quantified below. |
| PERF-020 | Open. One lazy boundary exists and is well placed (`SessionDetailPage`, carrying react-markdown and remark-gfm). | Entry chunk 627,712 B raw / 180,482 B gzip; lazy chunk 178,503 B / 54,200 B; stylesheet 182,123 B / 29,629 B. About 118 KB minified of the entry chunk is route components not needed at first paint. Mockup entry points are correctly excluded. |
| PERF-021 | Open. `grep -c 'profile.release' Cargo.toml` returns 0. | The absent block means `codegen-units = 16`, no LTO, and `panic = "unwind"` are also in effect, not only retained symbols. For a workload dominated by serde and SHA-256, cross-crate inlining is the part worth measuring alongside the already-measured 8.23 MB strip. |

### Detail added to PERF-017: the `logLines` case

`logLines` is `ProjectSync` state
([`ProjectSync.tsx:416`](../src/components/project-sync/ProjectSync.tsx#L416)).
Its only consumer is `<LogPanel>`, conditionally mounted behind `activityOpen`
([`ProjectSync.tsx:3976`](../src/components/project-sync/ProjectSync.tsx#L3976)).
The `sync-log` listener calls `setLogLines` unconditionally
([`ProjectSync.tsx:680`](../src/components/project-sync/ProjectSync.tsx#L680)),
including while the drawer is closed and nothing on screen can render the
result. Each call replaces a 2,000-element array, so React never bails.

Sampling nine days of this machine's own `~/.mallard/logs` — 5,267 events —
the peak emission rate is **32 events in one second** (2026-08-30). That is
32 root-level state writes per second, arriving exactly while the app is
animating a transfer readout.

Move `logLines` into `LogPanel` or a context only it subscribes to, buffer
arrivals in a ref, and flush on `requestAnimationFrame` so a burst produces
one commit. While the drawer is closed keep only the unread count, which
already bails correctly once it saturates at 99.

### Rejected hypothesis: the log merge comparator

`mergeLogLines`
([`ProjectSync.tsx:334`](../src/components/project-sync/ProjectSync.tsx#L334))
sorts with a `localeCompare` tiebreak, which looked like an obvious cost at
2,000 retained lines. It was measured and it is not.

| Comparator | Per appended line, 2,000 retained |
| --- | ---: |
| `localeCompare` tiebreak | 0.23 ms |
| Plain `<` / `>` tiebreak | 0.20 ms |

The timestamp comparison decides before the string tiebreak in almost every
case, and V8 handles the remainder well. Changing the comparator would recover
0.03 ms. The cost in this area is the re-render fan-out, not the merge. Do not
re-propose the comparator change.

## Measured baseline

### Environment

| Property | Value |
| --- | --- |
| Machine | Apple M3 Pro, 11 cores, 36 GB |
| OS | macOS 26.6.1 |
| Codex home | `~/.codex/sessions`, 12 GB, 1,792 `.jsonl` |
| Claude home | `~/.claude/projects`, 1.5 GB, 805 `.jsonl` (113 at the walker's depth 2-3, 1.09 GB) |
| Local state | `~/.mallard`, 5.0 MB |

### Discovery

| Operation | Files | Bytes | Records | Wall |
| --- | ---: | ---: | ---: | ---: |
| Claude directory walk | 113 | — | — | 40.4 ms |
| Claude metadata scan, as implemented | 113 | 1.09 GB | 237,661 | 2.230 s |
| Claude metadata scan, bounded head | 113 | 3.34 MB | 358 | 7.8 ms |
| Codex directory walk | 1,792 | — | — | 33.9 ms |
| Codex metadata scan, as implemented | 1,792 | 53.63 MB | 1,792 | 1.389 s |
| `canonicalize` x113 | — | — | — | 675 us |
| `canonicalize` x1,792 | — | — | — | 24.7 ms |

### Parse strategy A/B, same corpus, warmed page cache

| Arm | Round 0 | Round 1 | Hits |
| --- | ---: | ---: | ---: |
| `serde_json::Value` | 1.277 s | 1.369 s | 173,461 |
| Narrow `#[derive(Deserialize)]` | 0.541 s | 0.537 s | 173,461 |

### Frontend assets

| Artifact | Raw | gzip |
| --- | ---: | ---: |
| `index-K4rjPIVi.js` | 627,712 | 180,482 |
| `SessionDetailPage-DRUx5ng7.js` | 178,503 | 54,200 |
| `index-7mgscREy.css` | 182,123 | 29,629 |

Largest eager modules by minified size, from `esbuild --analyze`:
`react-dom` 172.9 KB, `ProjectSync` 58.9 KB, `ProjectLinksWorkspace` 56.0 KB,
`ProjectActivityPage` 43.6 KB, `StorageCatalogPage` 34.6 KB,
`ProjectSidebar` 18.1 KB, `ProjectSetupWorkspace` 17.5 KB, `Icons` 17.1 KB,
`SettingsPage` 14.9 KB.

### Local caches

| File | Bytes | Entries | Compact |
| --- | ---: | ---: | ---: |
| `session_timeline_cache.json` | 2,084,709 | 25 | 1,206,136 |
| `observation_cache.json` | 391,444 | 317 | 287,448 |

## Reproduction

**Discovery scans and the parse A/B.** Two standalone Cargo binaries
replicating the exact loop shapes of `scan_claude_session_metadata` and
`scan_session_metadata_with`, built `--release` at `opt-level = 3`, run
against the real provider homes. The A/B warms the page cache with a
discarded pass before either arm, alternates arms across two rounds, and
asserts both arms return identical hit counts.

The in-tree alternative is `MALLARD_SYNC_METRICS=1`, which makes
`log_operation_delta`
([`sync_metrics.rs:233`](../crates/mallard-infrastructure/src/sync_metrics.rs#L233))
print head reads, transcript bytes, and records parsed per operation. That is
the right instrument for confirming a fix, because it counts the production
code path rather than a replica.

**Log burst rate.** A sliding one-second window over every
`~/.mallard/logs/activity-*.jsonl`.

**Bundle composition.** `esbuild --bundle --minify --analyze` over
`src/main.tsx`, cross-checked against the committed `dist/assets` chunks;
esbuild's 805 KB flat total matches the 627,712 + 178,503 split Vite emits.

**Cache overhead.** Parsed and re-encoded compact, rather than estimated.

## Recommended sequence

### Milestone 1 — Bound the Claude scan — COMPLETE (`789eea5`)

Shipped as both changes at once rather than change 1 alone, with a corpus
parity gate and a window sweep behind the frozen 256 KiB constant. Measured
1.778 s to 0.564 s, 3.2x, with the design doc noting honestly that most of
the win is the narrow parse rather than the window.

Original recommendation, for the record: PERF-025 change 1 alone. It is unconditional, has no semantic effect, and is
verifiable with `MALLARD_SYNC_METRICS=1` plus the existing suite. Expect the
Claude leg of discovery to roughly halve.

### Milestone 2 — Walk each home once

PERF-026 and PERF-027 together. Rekey the inventory cache on
`(provider, home)`, single-flight the walk, then persist it with the
observation cache's existing pin shape. Land the rekey and single-flight
before persistence; they are independently valuable and independently
testable.

This milestone subsumes part of PERF-003, which asks for the same shared
per-home index for the Codex assignment audit. Consider doing them as one
change.

### Milestone 3 — Remove the cliffs

PERF-028 and PERF-030. Both are small, local, and remove failure modes rather
than shaving constants. PERF-028 should land before any user reaches 385
cached sessions.

### Milestone 4 — Transfer scheduling and cleanup

PERF-029 with a real R2 measurement, then PERF-031 through PERF-033
opportunistically.

PERF-025 change 2, the tail-bounded read, belongs after Milestone 2 has
reduced how often discovery runs at all. If discovery becomes rare enough,
its remaining cost may not justify the correctness risk.

## Safeguards to preserve

Confirmed intact under a production corpus and not to be regressed:

- 83 of 87 Tauri commands are `async` and route heavy work through
  `spawn_blocking` wrappers. Nothing blocks the async runtime.
- The watcher's dirty classification correctly separates appends from
  membership changes, so an actively appending transcript does not invalidate
  the inventory.
- The observation cache persists with exact `(size, mtime_nanos)` pins and
  re-stats every pin on serve.
- Fetch progress is throttled to 150 ms with the terminal event always
  emitted ([`commands.rs:106`](../src-tauri/src/project_sync/commands.rs#L106)).
- The frontend has no polling intervals. Every timer is a one-shot debounce or
  toast dismissal.
- The redaction ruleset is a compiled `OnceLock` singleton using `RegexSet`
  for the multi-pattern layers, with skip modes and an early exit when a line
  has no replacements.
- Codex discovery is properly bounded. It is the model PERF-025 should follow.
- Capture is parallelized behind `bounded_refresh_workers` with a documented
  rationale for the conservative default and a Settings knob to raise it.
- The dev Cargo profile is well tuned: dependencies at `opt-level = 1`,
  workspace crates at 0 for fast incremental builds.
- Vite targets the actual Tauri webview baseline rather than the default
  browser matrix, and omits sourcemaps from the signed bundle.
