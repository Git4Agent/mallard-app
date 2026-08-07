# S3 Dependency Installation Runtime Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow approved dependency/tool actions to securely re-fetch and validate bundles from S3 without blocking a Tokio async runtime worker.

**Architecture:** Refactor dependency application into an owned synchronous preparation phase, asynchronous action execution, and synchronous finalization phase. Run preparation and finalization through the existing `run_blocking` helper; preparation retains the existing bundle-pin validation before any installer can start.

**Tech Stack:** Rust, Tokio, Tauri 2, AWS SDK for Rust, Hyper localhost S3 stub, existing schema-3 bundle engine.

## Global Constraints

- Keep S3 credentials and transport construction in the existing configured S3 client.
- Preserve storage ID, bundle ID, revision, commit, manifest hash, replica, and binding revision validation before installation.
- Preserve the existing plugin identifier and argument allowlists.
- Do not convert the bundle engine or bundle-store trait to async.
- Do not invoke an external provider CLI from the regression test.

---

### Task 1: Reproduce S3 dependency application on an async worker

**Files:**
- Modify: `src-tauri/src/sync_tests/mod.rs`
- Modify: `src-tauri/src/project_sync_v3/commands.rs`

**Interfaces:**
- Consumes: `sync_tests::stub_s3::StubS3`, `run_blocking`, and the existing project registration, binding, push, restore-plan, and dependency-plan helpers.
- Produces: async regression test `dependency_application_fetches_s3_bundle_off_runtime_worker`.

- [x] **Step 1: Expose the existing test-only S3 stub within the crate**

Change the test module declaration to:

```rust
pub(crate) mod stub_s3;
```

- [x] **Step 2: Write the failing regression test**

In `commands.rs` tests, start `StubS3` on a Tokio multi-thread runtime, configure a schema-3 S3 storage pointing to its endpoint, and create a registered/bound/linked project. Use `run_blocking` for push, restore planning, and dependency planning. Then call:

```rust
let result = apply_dependency_actions_with_repository(&repo, &plan.plan_id, Vec::new()).await;
assert!(result.is_ok(), "S3 dependency application failed: {result:?}");
```

The empty selection exercises secure bundle re-fetch and pin validation without starting a native installer.

- [x] **Step 3: Run the regression test and verify RED**

Run:

```bash
cd src-tauri && cargo test dependency_application_fetches_s3_bundle_off_runtime_worker -- --nocapture
```

Expected: FAIL containing `S3 bundle-store get cannot block an async runtime worker`.

### Task 2: Split dependency application across runtime-safe phases

**Files:**
- Modify: `src-tauri/src/project_sync_v3/commands.rs`
- Test: `src-tauri/src/project_sync_v3/commands.rs`

**Interfaces:**
- Produces: private owned `PreparedDependencyApplication { plan: DependencyPlan, binding: ProjectBinding, selected: BTreeSet<ActionId>, applied_at: u64 }`.
- Produces: synchronous `prepare_dependency_application(&V3Repository, &PlanId, Vec<ActionId>) -> Result<PreparedDependencyApplication, String>`.
- Produces: synchronous `finalize_dependency_application(&V3Repository, PreparedDependencyApplication, Vec<DependencyApplyReceipt>) -> Result<DependencyResult, String>`.
- Consumes: existing async `execute_dependency_action` for the execution phase.

- [x] **Step 1: Extract synchronous preparation**

Move plan expiry checks, duplicate-application checks, binding resolution, S3 bundle fetch, `validate_dependency_plan_pin`, and `unique_dependency_actions` into `prepare_dependency_application`. Keep all values owned so the function can run in `spawn_blocking`.

- [x] **Step 2: Run preparation with `run_blocking`**

At the start of `apply_dependency_actions_with_repository`, clone the repository and plan ID and execute:

```rust
let prepared = run_blocking(move || {
    prepare_dependency_application(&prepare_repository, &prepare_plan_id, action_ids)
})
.await?;
```

Only after this succeeds, iterate over `prepared.plan.actions` and await selected native installers.

- [x] **Step 3: Extract and offload finalization**

Move application-record persistence and `DependencyResult` construction into `finalize_dependency_application`. After action execution, clone the repository and call finalization through `run_blocking`, ensuring repository writes do not run on the async worker.

- [x] **Step 4: Run the regression test and verify GREEN**

Run:

```bash
cd src-tauri && cargo test dependency_application_fetches_s3_bundle_off_runtime_worker -- --nocapture
```

Expected: PASS with one dependency application record containing only skipped receipts, and no runtime-worker error.

### Task 3: Verify behavior and branch integrity

**Files:**
- Modify only if verification reveals a defect in the scoped change.

**Interfaces:**
- Consumes: completed runtime-boundary implementation and regression test.
- Produces: compiler, test, and frontend-build evidence.

- [x] **Step 1: Run focused project-sync tests**

```bash
cd src-tauri && cargo test project_sync_v3::commands::tests --lib
```

Expected: all command-module tests PASS.

- [x] **Step 2: Type-check the Rust backend**

```bash
cd src-tauri && cargo check
```

Expected: exit status 0.

- [x] **Step 3: Build the frontend**

```bash
npm run build
```

Expected: TypeScript checks and Vite build exit with status 0.

- [x] **Step 4: Inspect the final diff**

```bash
git diff --check && git status --short
```

Expected: no whitespace errors and only the planned implementation/test files are modified.

- [ ] **Step 5: Commit the implementation**

```bash
git add src-tauri/src/sync_tests/mod.rs src-tauri/src/project_sync_v3/commands.rs docs/superpowers/plans/2026-07-20-s3-dependency-install-runtime-fix.md
git commit -m "Fix S3 dependency bundle downloads"
```
