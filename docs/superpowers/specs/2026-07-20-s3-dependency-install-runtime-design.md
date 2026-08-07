# S3 Dependency Installation Runtime Fix

## Problem

Applying approved dependency actions is an asynchronous operation because native plugin installers are child processes. Before starting an installer, Mallard synchronously re-fetches the approved bundle to verify that its storage ID, bundle ID, revision, commit, manifest hash, and binding revision still match the dependency plan.

The S3 bundle-store adapter implements the synchronous bundle-store contract by driving AWS SDK futures with a Tokio runtime handle. That bridge must execute on a blocking worker. The dependency-application command currently performs the re-fetch directly on a Tokio async runtime worker, where `Handle::block_on` is forbidden. The adapter converts the resulting panic into the reported `S3 bundle-store get cannot block an async runtime worker` error. Local-folder storage does not expose the mistake because its implementation is synchronous.

## Design

Split dependency application into three explicit phases:

1. **Prepare on a blocking worker.** Load and validate the dependency plan, confirm it has not already been applied, resolve the active binding, fetch the exact bundle from the configured store, validate the plan pin, and validate the approved action IDs. Return an owned preparation value containing all data required by execution.
2. **Execute asynchronously.** Run selected native plugin installers through Tokio's process API. Standalone-skill and manual dependency behavior remains unchanged. No installer may start unless preparation and bundle-pin validation succeeded.
3. **Finalize on a blocking worker.** Build and persist the dependency application record and return the result. Repository filesystem operations stay off the async runtime worker.

The phase boundary must use the existing `run_blocking` helper so S3 engine operations execute under `tauri::async_runtime::spawn_blocking` consistently with fetch, push, restore planning, and restore application.

## Security Properties

- S3 credentials continue to be handled only by the existing configured S3 client.
- Bundle objects continue through the bundle engine's existing size, object-key, ETag, manifest-hash, and content-digest validation.
- The fetched bundle must match the dependency plan's storage, bundle, revision, commit, manifest hash, replica, and binding revision before any installer starts.
- Installer arguments remain restricted to the already validated portable plugin identifier and supported argument sets.
- A changed or missing remote bundle fails closed and launches no installer.
- Dependency application receipts are persisted only after execution, preserving current retry and audit behavior.

## Testing

Add a regression test that runs dependency preparation from a Tokio async test against the real S3 bundle-store adapter and localhost stub. Before the fix, the test must fail with the runtime-worker blocking error. After the fix, it must complete S3 bundle verification without that error. Use an empty approved-action selection or a non-launching action so the test does not invoke an external CLI.

Run the focused Rust regression test, the relevant project-sync tests, `cargo check`, and `npm run build` before handoff.

## Non-goals

- Converting the synchronous bundle-engine trait to async.
- Changing S3/R2 credentials, transport configuration, or object layout.
- Changing which dependency actions require explicit approval.
- Downloading plugin executables directly from S3; plugin installation remains delegated to the provider's native CLI after bundle intent verification.
