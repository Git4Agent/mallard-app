**Dropbox push performance plan**

Status: implemented on 2026-09-08. Automated checks passed. The fresh-state
live Dropbox/R2 retest remains pending.

Fix sequential Dropbox uploads and repeated parent-folder checks. Keep all
persisted schemas at **1**, including local state, storage configuration, the
storage marker, portable manifests, and setup recovery. Keep the existing
object keys and payload format. Use a clean cutover with one write path. Add
no migration, feature flag, old transport fallback, or persistent upload state.
The user will clear local Mallard state and cloud test data for retesting.

The supplied run spent 98.780 seconds uploading 39 objects with 153.9 MB of
payload. The complete push took 128.450 seconds. Before this change, the
[Dropbox batch loop](../crates/mallard-infrastructure/src/storage_core/dropbox_store.rs)
waited for each write. Each write also checked five parent folders.

**Design decision.** Use concurrent upload sessions and one batch commit per
transfer window. This is the mechanism for fixing upload concurrency. Dropbox
recommends separating data transfer from namespace commit for bulk uploads.
Parallel calls to the current direct-upload endpoint can cause namespace lock
contention. [Dropbox performance guide](https://developers.dropbox.com/dbx-performance-guide)

Use `files/upload_session/start` with the complete object body, `close: true`,
and a content hash. Use `files/upload_session/finish_batch_v2` to commit the
window. The current API defines this finish route as a synchronous result with
one result per input entry. Keep the existing 150 MiB limit per object. Each
object needs one data request. No chunk append or asynchronous job polling is
needed. [Dropbox API specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone)

1. **Build one bounded write pipeline.**

   Validate every input key, size, and write mode before the first network
   request. Preserve the existing full-input preflight for `put_many`, including
   Library callers that pass more than one transfer window. Resolve duplicate
   immutable keys before dispatch: share an upload for equal bytes and reject
   different bytes for the same key. Preserve one outcome per input position.

   Use the existing `cloud_transfer_concurrency` setting. Its default is six
   and its maximum is sixteen. Process at most that many bodies in each
   window. Use the existing Tokio, Reqwest, and Bytes dependencies. Share the
   HTTP client and authorization state across workers. Keep refresh under the
   existing authorization lock. Share immutable `Bytes` allocations and compute
   each Dropbox content hash once.

   Use a bounded `JoinSet`. Collect results by input index. Complete each
   window before starting the next. On a data-transfer error, cancel and drain
   its remaining workers. Do not commit that window or start another window.
   Session identifiers remain in memory and never enter logs or persisted data.
   Reject empty or oversized session identifiers. Bound the serialized commit
   request as well as its response with the existing RPC byte limit.

2. **Check each distinct parent once per commit window.**

   Superseded on 2026-09-09 by
   `docs/PLAN_DROPBOX_SYNC_PATH_REQUEST_DEPTH.md`, decision 1: ancestor
   folders are no longer validated on any read or write; Dropbox creates
   missing parents on commit.

   After all bodies in a window have been staged, collect their distinct parent
   paths. Check these paths from the root down, immediately before commit.
   Reuse each successful check within that window. Create a missing folder
   once. Read and validate it after creation or a creation conflict.

   Preserve exact component-name, canonical-path, and folder-type checks.
   Reject aliases and invalid entries before commit. Keep file metadata and
   content validation after commit. Do not replace component checks with
   `path_lower` alone. Dropbox's `path_display` field is for display and does
   not guarantee the case of every ancestor.
   [Dropbox metadata specification](https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone)

   Discard the parent-check set when the window ends. Each later window and
   each separate operation must check again. Reads, deletes, and recovery must
   not trust a prior write window's checks.

   This changes validation from one observation per file to one observation
   per commit window. It does not make a remote folder tree atomic. Test a
   folder that acquires an invalid name or type during staging or between
   windows. The next pre-commit check must reject it. A change after the final
   check remains a race, as it is in the current per-file implementation.

3. **Commit and verify the window.**

   Commit the staged objects in one request. Immutable writes retain strict
   `add`. Mutable writes retain strict absent-create or revision-checked
   `update`. Keep autorename disabled. Validate response count, entry order,
   path, type, revision, size, and Dropbox content hash.

   Preserve full size and standard SHA-256 verification when an immutable
   object already exists or its commit result is unknown. Accept that object
   only when verification succeeds. An unknown mutable-write result remains
   unknown; equal bytes do not establish ownership of a head update.

   Handle entry failures as well as request failures. A batch commit can leave
   some immutable objects stored. Return an error if any entry is unresolved
   or invalid. The caller must not advance the head or write success receipts.
   Keep valid unreferenced immutable objects; do not add rollback deletion.
   Keep bounded responses, request timeouts, fixed error text, and the current
   rule against automatic mutation retries.

4. **Cut over all Dropbox writes.**

   Route `put_many`, `put_immutable`, compare-and-swap, marker writes, and setup
   probe writes through the same pipeline. A single write uses a one-entry
   window. Remove the direct `files/upload` implementation and its sequential
   bulk loop. Do not retain them as a fallback.

   Keep the neutral `ObjectStore` interface and the project publish loop.
   Dropbox performs its own windowing when a caller passes a larger batch.
   Keep publication order: payloads, manifest, commit record, then conditional
   head update. Preserve setup journal ownership and exact-revision cleanup.

   Update the existing Dropbox design and setup documents to describe the new
   transport. Remove obsolete statements that upload sessions are unsupported.
   The progress-timer error and the local request-build cost are outside this
   implementation scope. Use the complete upload-phase timer for measurement.

5. **Prove concurrency, request reduction, and failure behavior.**

   Extend the loopback mock with session staging, batch commit, route counters,
   active-request counters, and deterministic barriers. Keep waits outside its
   state mutex so the mock permits concurrent requests. Use synthetic payloads,
   fake tokens, and temporary homes.

   Add tests for the following requirements:

   - Six body requests can be active together at the default setting. Counts
     never exceed the setting. A setting of one uses the same pipeline.
   - A batch larger than the setting completes in bounded windows, with stable
     result order. Empty and single-entry calls work.
   - Invalid paths, modes, and over-limit bodies fail before any network call.
     Equal duplicate keys share a transfer; unequal duplicates fail preflight.
   - Shared parents are checked once per window. Mixed parent trees check each
     unique component. Missing folders are created once and then verified.
   - Uppercase aliases, wrong types, bad metadata, and invalid folder changes
     before the final check fail closed. A later operation never uses stale
     parent validation.
   - Worker failure prevents commit of its window and drains all workers.
     Malformed results, lost responses, HTTP 429, and partial batch failure
     preserve sanitized errors and do not replay mutations.
   - Immutable collisions verify bytes. Stale head revisions fail, including
     identical-byte updates and delete/recreate cases. Unknown head writes
     never report success. Partial payload publication cannot advance the head.
   - Project publication and reviewed restore, Library publication, marker
     creation, and setup probe recovery pass through the new transport.
   - The mock rejects any call to `files/upload`. Schema-1 serialization and
     the existing object-key layout remain unchanged.

6. **Validate and retest.**

   Run focused Dropbox tests while implementing. Run the relevant Rust suites
   before handoff:

   ```sh
   cargo test -p mallard-infrastructure --lib dropbox
   cargo test -p mallard-application --lib dropbox
   cargo test --manifest-path src-tauri/Cargo.toml --lib dropbox
   cargo test -p mallard-infrastructure --lib
   cargo test -p mallard-application --lib
   cargo test --manifest-path src-tauri/Cargo.toml --lib
   cargo test --manifest-path src-tauri/Cargo.toml --test storage_catalog_e2e
   ```

   After the user resets test data, run a fresh setup and push in the complete
   Tauri application. Use the same generated 39-object, 153.9 MB fixture for
   Dropbox and R2. Keep the build, concurrency setting, machine, and network
   conditions equal. Repeat with fresh destinations and report median full
   upload-phase time. Verify restored bytes and a subsequent changed-object
   push. Do not use the displayed 80.8-second duration as the baseline; it
   excludes the first completed batch.

The following counts are verified by the local HTTP mock for 39 new payloads
in one existing parent tree, with a window of six. They exclude authentication,
control objects, folder creation, and error recovery.

| Measure | Before change | Implemented path |
| --- | ---: | ---: |
| Maximum concurrent body transfers | 1 | 6 |
| Parent metadata requests | 195 | 35 |
| Data requests | 39 direct uploads | 39 session starts |
| Separate batch commits | 0 | 7 |
| Total payload-path requests | 234 | 81 |

The request-count and concurrency tests are acceptance gates. Live timing
must confirm an improvement. The plan does not assume a sixfold speed gain
or parity with R2.

| File area | Planned change |
| --- | --- |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs` | Shared write pipeline, parent validation, and error mapping |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/upload.rs` | New private session and commit implementation |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/mock.rs` | Session routes, counters, barriers, and fault injection |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/tests.rs` | Transport, request-count, concurrency, and failure tests |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/probe.rs` | Probe use of the shared pipeline; same journal schema |
| Existing Dropbox application and Tauri tests | Project, Library, and setup coverage |
| Existing Dropbox design and setup documents | Replace the obsolete transport description |

**Implementation verification, 2026-09-08.**

- Focused infrastructure Dropbox tests: 40 passed.
- Rust library suites: 791 passed; five existing manual tests ignored.
- Library end-to-end test: one passed.
- Formatting and diff whitespace checks passed for the changed code.
- The project publication failure test stored one new immutable payload,
  rejected the next, and verified that the published head and revision did
  not change. Existing project restore, Library, and setup recovery tests pass.

Commands executed:

```sh
cargo test -p mallard-infrastructure --lib dropbox --locked
cargo test --locked -p mallard-infrastructure -p mallard-application -p mallard --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --test storage_catalog_e2e
```

The end-to-end build reported an unused-function warning for the existing
`capture_request_for_binding` helper. This task did not change that helper.

All persisted schemas remain 1. The old write route is absent. Live performance
acceptance remains pending until the user prepares fresh test data and runs
the Dropbox/R2 comparison. The mock request counts do not establish live speed.
