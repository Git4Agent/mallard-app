# Deep Research: Core Domain Logic Audit Report

**Scope:** Mallard core domain, project sync domain, storage catalog domain, and storage transport core.  
**Date:** 2026-08-18  
**Priority legend:** P1 (high) / P2 (medium) / P3 (low)

## Executive Summary

1. We found repeated integrity- and concurrency-risk patterns in publish/restore/write paths where side effects can occur before final concurrency checks pass.
2. Several storage operations use “recover by re-reading” patterns; these can hide transient failures or race conditions depending on backend consistency behavior.
3. A few validation paths are strict for security/consistency but can produce brittle behavior in legitimate edge cases (path and URL forms, duplicate handling, revision event handling).

## Finding 1 — P1: Publish flow can leak immutable objects after CAS conflict

In catalog publish, object payloads are uploaded before validating and committing the head/revision pointer. If the final CAS to advance the head fails, uploaded immutable objects remain.

**Location:** [src-tauri/src/storage_catalog/resource_engine.rs#L310]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_engine.rs#L310)–[src-tauri/src/storage_catalog/resource_engine.rs#L516]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_engine.rs#L516)  
**Impact:** Storage bloat, dangling unreferenced objects, and harder manual recovery after publication races.  
**Suggested fix:** Convert publish to a staging/ownership model (single commit token object written first, then payloads, then head CAS with explicit rollback/cleanup marker).

## Finding 2 — P1: Bundle publish path lacks rollback path when compare-and-swap conflict occurs

The bundle publish routine performs manifest/commit/tag writes and then performs a CAS on remote head; conflict handling retries but no explicit cleanup is visible when retries exhaust.

**Location:** [src-tauri/src/bundle_engine.rs#L529]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L529)–[src-tauri/src/bundle_engine.rs#L906]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L906)  
**Impact:** Leftover content can remain reachable only through object digests, complicating auditability and causing storage pressure in S3/R2-local namespaces.  
**Suggested fix:** On hard conflict failure, mark orphan objects via a reconciliation pass and queue cleanup; consider explicit per-publish operation ID with eventual GC.

## Finding 3 — P1: Ambiguous S3 write/delete outcomes rely on follow-up reads under eventual-consistency assumptions

`compare_and_swap_async` and delete helpers resolve ambiguous outcomes by re-reading object state and then mapping to success/conflict. In eventual consistency environments this can be non-deterministic.

**Location:** [crates/mallard-infrastructure/src/storage_core/s3_store.rs#L645]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L645)–[crates/mallard-infrastructure/src/storage_core/s3_store.rs#L826]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L826)  
**Impact:** False-positive success/failure can cause inconsistent state or repeated retries under stable network partitions.  
**Suggested fix:** Propagate an explicit operation intent ID in metadata and use idempotency keys where supported; treat ambiguity as retriable only after bounded verification window and monotonic read checks.

## Finding 4 — P1: Ambiguous immutable write path can misreport CAS conflict as success

In immutable write and conditional write flows, conflict is inferred from an ambiguous response + immediate status read, which can race with cleanup/replication lag.

**Location:** [crates/mallard-infrastructure/src/storage_core/s3_store.rs#L620]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L620)–[crates/mallard-infrastructure/src/storage_core/s3_store.rs#L716]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L716)  
**Impact:** Downstream plan code may assume durable object publication happened when it may not have been accepted by backend.  
**Suggested fix:** Add explicit digest-bearing operation receipts and tighten classification for ambiguous outcomes in a single retry gate.

## Finding 5 — P1: Restore/apply path has potential TOCTOU between validation and write

Validation of logical targets and atomic write staging is split from execution; target shape can become invalid between compute and write in races with concurrent filesystem edits.

**Location:** [src-tauri/src/bundle_engine.rs#L3313]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L3313)–[src-tauri/src/bundle_engine.rs#L3353]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L3353), [src-tauri/src/bundle_engine.rs#L3408]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L3408)–[src-tauri/src/bundle_engine.rs#L3456]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/bundle_engine.rs#L3456)  
**Impact:** Risk of last-moment conflicts or partial write failure despite passing preflight checks.  
**Suggested fix:** Re-check target canonicalization at write time and ensure conflicts are explicitly retriable with user-visible plan state updates.

## Finding 6 — P1: Deletion flow can be optimistic under ambiguous delete handling

Delete operations classify ambiguous result states through follow-up probing. If the follow-up path is stale, deletion might be assumed complete when object still exists.

**Location:** [src-tauri/src/storage_catalog/resource_commands.rs#L1966]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_commands.rs#L1966)–[src-tauri/src/storage_catalog/resource_commands.rs#L2136]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_commands.rs#L2136), plus [crates/mallard-infrastructure/src/storage_core/s3_store.rs#L777]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L777)–[crates/mallard-infrastructure/src/storage_core/s3_store.rs#L826]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L826)  
**Impact:** Residual objects may survive after plan is marked complete; user sees success with missing actual effect.  
**Suggested fix:** Treat ambiguous delete as “in-progress” until a second confirmation and expose retry/repair action.

## Finding 7 — P2: Namespace wipe loop is potentially expensive and can miss concurrent insertions

`clear_managed_storage_data` performs bounded page scanning with multiple cursor invocations; it repeatedly re-lists pages and may process stale cursors as objects are created or deleted concurrently.

**Location:** [crates/mallard-infrastructure/src/storage_core/mod.rs#L122]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/mod.rs#L122)–[crates/mallard-infrastructure/src/storage_core/mod.rs#L163]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/mod.rs#L163)  
**Impact:** Performance regression on large namespaces and potential orphan leaves under concurrent mutations.  
**Suggested fix:** Use delete-by-prefix with eventual consistency safeguards or mark-and-sweep with immutable generation tokens.

## Finding 8 — P2: `resolve_profile_context` overlaps check can be bypassed by timing/normalization edge cases

Profile overlap checks compare canonical roots but are tightly coupled to default-`~/.codex` behavior and path classification timing, making it hard to reason about equivalence across alternate codex roots.

**Location:** [src-tauri/src/storage_catalog/profiles.rs#L225]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L225)–[src-tauri/src/storage_catalog/profiles.rs#L284]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L284), [src-tauri/src/storage_catalog/profiles.rs#L197]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L197)–[src-tauri/src/storage_catalog/profiles.rs#L199]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L199), [src-tauri/src/storage_catalog/profiles.rs#L271]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L271)–[src-tauri/src/storage_catalog/profiles.rs#L279]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/profiles.rs#L279)  
**Impact:** Possible overlapping profile roots in rare mixed-canonicalization scenarios.  
**Suggested fix:** Centralize overlap logic on one normalized canonical form and include explicit regression tests around symlinks/relative components/custom home roots.

## Finding 9 — P2: Git remote validation is strict to the point of incompatibility

The Git location validator rejects characters and URL constructs (e.g., `?`, `#`) and credential-looking fragments; this can block legitimate remote forms in narrow workflows.

**Location:** [src-tauri/src/storage_catalog/domain.rs#L390]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/domain.rs#L390)–[src-tauri/src/storage_catalog/domain.rs#L427]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/domain.rs#L427)  
**Impact:** False negatives during profile and repository setup; users cannot add valid but uncommon repository URLs.  
**Suggested fix:** Keep security checks but document a supported URL syntax and provide an explicit allow/deny list rationale with compatibility tests.

## Finding 10 — P2: Case-insensitive name index can over-block on case-sensitive stores

`name_taken_index` enforces case-insensitive uniqueness when evaluating collisions; this can reject names that are distinct on file systems where case sensitivity differs.

**Location:** [src-tauri/src/storage_catalog/resource_commands.rs#L1864]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_commands.rs#L1864)–[src-tauri/src/storage_catalog/resource_commands.rs#L1920]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_commands.rs#L1920)  
**Impact:** User cannot create semantically distinct entries that differ by case only.  
**Suggested fix:** Align collision behavior with the backend’s actual case rules and expose that rule in validation errors.

## Finding 11 — P2: Config watcher drops same-revision updates

File-change watcher emits only monotonic revision increases; malformed/noisy docs are skipped and equal-revision events are ignored, which can suppress no-op config churn while keeping stale state.

**Location:** [src-tauri/src/project_sync/config_watcher.rs#L32]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/project_sync/config_watcher.rs#L32)–[src-tauri/src/project_sync/config_watcher.rs#L114]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/project_sync/config_watcher.rs#L114)  
**Impact:** State can lag if UI/config editing reuses the same revision number; recovery may require manual resync.  
**Suggested fix:** Add a checksum fallback for equal-revision updates and periodic full reconciliation.

## Finding 12 — P2: Inventory duplicate handling can hide candidate records

The scanner deduplicates entries by warning buckets and “one-per-root” behavior, and duplicate detection can become lossy in mixed-encoding/case collisions depending on root traversal.

**Location:** [src-tauri/src/storage_catalog/inventory.rs#L115]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/inventory.rs#L115)–[src-tauri/src/storage_catalog/inventory.rs#L231]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/inventory.rs#L231)  
**Impact:** Valid entries can be dropped from candidate lists without explicit user-level visibility.  
**Suggested fix:** Make duplicate policy explicit: either deterministic winner with reason codes, or surface all conflict variants for user resolution.

## Finding 13 — P2: S3 pagination loop in clear/purge paths can degrade with large catalogs

`list_async` follows continuation tokens but caller loops can become heavy with small page sizes and repeated passes.

**Location:** [crates/mallard-infrastructure/src/storage_core/s3_store.rs#L828]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L828)–[crates/mallard-infrastructure/src/storage_core/s3_store.rs#L938]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/s3_store.rs#L938)  
**Impact:** Potential latency spikes and long-running cleanup jobs on large catalogs.  
**Suggested fix:** Tune page size with adaptive batching and enforce bounded-time cleanup windows with resumable cursor checkpoints.

## Finding 14 — P2: Path canonicalization is restrictive for non-UTF-8 and certain filesystems

`normalized_relative_path` rejects non-UTF-8 path components, which can prevent storage of binary-named files in local stores.

**Location:** [crates/mallard-infrastructure/src/storage_core/local_store.rs#L478]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L478)–[crates/mallard-infrastructure/src/storage_core/local_store.rs#L494]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L494)  
**Impact:** Incomplete or failed sync for uncommon filenames; likely low-frequency but user-visible.  
**Suggested fix:** Document as intentional limit or switch to `OsStr`-based handling and explicit conversion policy.

## Finding 15 — P3: Non-follow symlink enforcement is strict and can block legitimate shared content trees

Local store checks intentionally avoid symlink traversal for safety, but this can make certain existing workspace layouts impossible to synchronize.

**Location:** [crates/mallard-infrastructure/src/storage_core/local_store.rs#L106]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L106)–[crates/mallard-infrastructure/src/storage_core/local_store.rs#L142]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L142), [crates/mallard-infrastructure/src/storage_core/local_store.rs#L294]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L294)–[crates/mallard-infrastructure/src/storage_core/local_store.rs#L309]( /Users/hequ/Desktop/project/mallard-app/crates/mallard-infrastructure/src/storage_core/local_store.rs#L309)  
**Impact:** Safer by design, but users with intentional symlinked plugin/state trees must reconfigure layout.  
**Suggested fix:** Keep default deny, but improve diagnostics to clearly show blocked symlinked subpaths and offer migration guidance.

## Finding 16 — P3: Validation granularity leaves limited observability in edge-case duplicate paths

Case-insensitive duplicate handling and warning aggregation can reduce explainability for collisions across roots without attaching machine-readable causes to each filtered candidate.

**Location:** [src-tauri/src/storage_catalog/inventory.rs#L231]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/inventory.rs#L231), [src-tauri/src/storage_catalog/resource_commands.rs#L1864]( /Users/hequ/Desktop/project/mallard-app/src-tauri/src/storage_catalog/resource_commands.rs#L1864)  
**Impact:** Users may not understand why a candidate was excluded or which policy was applied.  
**Suggested fix:** Extend warning model with explicit reason enums and per-item suppressed/filtered metadata.

## Cross-cutting Remediation Plan

1. Stabilize write/remove CAS outcomes: introduce explicit operation IDs and idempotent write/delete semantics with deterministic conflict classification.
2. Add a shared reconcile/cleanup layer for orphaned immutable objects after publication conflicts.
3. Improve watcher and inventory observability by attaching machine-readable conflict reasons.
4. Add property-based/chaos tests for event ordering races and stale reads around CAS/list/delete loops.
5. Add load tests for catalog clear/purge on large prefixes to cap cleanup duration and memory use.

## Note

This report is evidence-based from a code-path review of core-domain modules and does not run live integration or backend verification in this pass.
