**Dropbox sync path request-depth plan**

Status: implemented on 2026-09-09. Follows
`docs/DROPBOX_SYNC_PATH_AUDIT_2026-09-09.md`. Deltas from the plan as written
are listed at the end.

Cut the number of sequential Dropbox requests behind Compare, Push, and Pull.
Keep all persisted schemas at **1**: local state, storage configuration, the
storage marker, portable manifests, restore plans, and setup recovery. Keep the
existing object keys, payload format, and the `ObjectStore` contract shape. Use
a clean cutover with one code path per concern. Add no migration, feature flag,
legacy transport, or compatibility branch. The user clears local Mallard state
and cloud test data before retesting.

The audit counted 104 sequential requests for a 39-object push, 26 for one
Activity refresh, and 241 for a 38-transcript Pull review. Three causes account
for nearly all of it: an ancestor-folder walk before every read and write
window, no bulk read path on Dropbox, and a fresh store (TLS, OAuth, marker) on
every engine open. The plan removes those three first, then trims the publish
sequence and two redundant full downloads.

**Design decisions.**

1. *Ancestor folders are not validated.* Dropbox resolves a path to exactly one
   entry regardless of case, so a walk over `/.mallard`, `/.mallard/v1`, and the
   bundle folders can never change which bytes a download returns or where a
   commit lands. Every response still proves its own entry: `path_lower` must
   equal the requested path and `name` must equal the last component with
   exact case (`Metadata::check`, unchanged). A file occupying a folder name
   fails closed at the point it matters: a read below it reports `Missing`, and
   a commit below it returns Dropbox's `conflict/file_ancestor`, which maps to
   `Invalid`. This replaces the once-per-window rule from
   `docs/PLAN_DROPBOX_PUSH_PERFORMANCE.md` step 2.
2. *One store per storage configuration per process.* A process-wide table
   keyed by storage ID holds the opened `ConfiguredObjectStore` together with
   the exact `StorageConfig` it was built from. A lookup with an equal config
   returns the same store; a different config (renamed, edited, rotated secret)
   replaces the entry. The storage marker is verified once, when an entry is
   created. No other code path verifies the marker.
3. *Bulk reads on Dropbox use the upload pipeline's shape.* `stage_many_verified`
   and `get_metadata_many` run in a `JoinSet` bounded by
   `cloud_transfer_concurrency`, keep input order, and abort and drain the rest
   on the first failure, the same contract the R2 store already implements.
4. *Publish reuses what it observed.* When the fresh head's revision token equals
   the token the caller observed, the immutable manifest named by that head is
   the one already in memory; it is not read again. The head itself is always
   re-read so a disjoint-scope publisher can still be grafted onto. The manifest
   and the commit record are written in one immutable batch. The head
   compare-and-swap stays a separate, final write.
5. *An immutable collision is proved by metadata.* A `conflict` on an `add`
   commit is resolved with one `files/get_metadata`: kind `file`, equal size,
   and equal Dropbox `content_hash` against the hash computed for the staged
   body. No object body is downloaded to prove presence. The success path
   already trusts that same hash.
6. *Verified immutable manifests are served from the object cache.* The engine
   owns the process-wide `ObjectCache` (today it is passed per call). Current
   and historical manifest reads consult it by storage, bundle, digest, and
   size before staging from the store. The continuity probe keeps its contract
   and always reads the store; it inserts what it verified.

**Steps.**

1. **Remove the ancestor walk from the transport.**

   In `dropbox_store.rs`: delete `parents`, `check_parent_paths`, and the
   `files/create_folder_v2` request. `download` issues `files/download`
   directly. `enumerate` issues `files/list_folder` directly; `not_found` is an
   empty page and `not_folder` is `Invalid`. `delete_path` issues
   `files/delete_v2` with `parent_rev` directly; the existing conflict branch
   that re-reads metadata to report the current revision stays. In
   `upload.rs`: `upload_window` stages, then commits under the commit gate;
   `parent_paths` is deleted. `classify_conflict` maps `conflict/file_ancestor`
   to `Invalid`. In `probe.rs`: `cleanup` reads the probe object's metadata
   directly. Dropbox creates missing parent folders on commit; the mock must do
   the same.

   Result: head, tag, marker, and manifest reads are 1 request; a payload read
   is 1 request; a write window is 2 (parallel starts plus one batch commit);
   the head compare-and-swap is 2; a tag publish is 3.

2. **Add bulk reads to the Dropbox store.**

   Split the bodies of `stage_verified` and `get_metadata` into
   `stage_verified_async` and `get_metadata_async`. Implement
   `stage_many_verified` and `get_metadata_many` on `DropboxObjectStore` with a
   `JoinSet` and a semaphore sized by `cloud_transfer_concurrency`, mirroring
   `s3_store.rs` lines 919-1046 and the abort-and-drain rule. Single-element
   inputs take the direct path. `EngineByteSource::prefetch` and
   `list_remote_bundles` need no change.

3. **Share one opened store per storage.**

   Add `ConfiguredObjectStore::shared(config)` in `storage_core/mod.rs`, backed
   by a `OnceLock<Mutex<BTreeMap<StorageId, (StorageConfig, ConfiguredObjectStore)>>>`.
   Derive `Clone` on `ConfiguredObjectStore` (it wraps an `Arc`). Creating an
   entry calls `open_with` and then `ensure_storage_marker`, under the table
   lock so two concurrent first opens produce one marker read. Delete
   `from_config`. Route every production open through `shared`:
   `engine_for_storage_config` and `clear_storage_data_with_repositories` in
   `project_sync/service.rs`, `fetch_activity_snapshot` in
   `project_activity.rs`, and `engine_for` in `storage_catalog/service.rs`.
   Remove the marker calls from `BundleEngine::open_with_contracts`,
   `LibraryResourceEngine::publish`, and `clear_managed_data_with`; the store
   is verified before an engine can hold it. Tests that build bare stores and
   need the marker call `ensure_storage_marker` themselves.

   The Dropbox store's captured runtime handle is the application runtime for
   the process lifetime. Test processes reuse storage IDs across temporary
   roots; entries are replaced on config inequality, never accumulated, so
   isolation holds without a test-only hook.

4. **Trim the publish sequence.**

   Change `PublishExpectation::Match(HeadToken)` to carry the observed manifest:
   `Match { token: HeadToken, manifest: BundleManifest }`. `publish_reviewed`
   builds it from the snapshot it already holds. In `publish_with_progress`,
   after `read_head`, use the carried manifest when the fresh token's `etag`
   equals the observed `etag`; otherwise read the manifest as today. Write the
   manifest and the commit record with one `put_many` call. Both outcomes must
   be `Written`; anything else is an error before the head swing. `publish_tag`
   is unchanged: after step 1 its metadata read and conditional write cost 3
   requests, and a metadata-only trait method would not reduce that.

5. **Prove collisions by metadata.**

   In `put_many_async`, replace the `download` on `Conflict | Unknown` with
   `metadata(path)`. Require `kind == "file"`, `size` equal to the staged body,
   and `content_hash` equal to the hash computed at staging; return
   `AlreadyPresent`. A missing entry or any mismatch is `Integrity`. Run the
   proofs for one window concurrently in the same `JoinSet` pattern so a retry
   after a failed head swing costs one metadata request per object and no
   body transfer. Compute the Dropbox content hash in `Upload::new` on the
   calling thread and keep it on `Upload`; `Session` keeps only the ID.

6. **Serve verified manifests from the object cache.**

   Move the `ObjectCache` into the engine: `BundleEngine::open(store,
   storage_id, cache: Option<Arc<ObjectCache>>)`; `byte_source(snapshot,
   on_progress)` uses the engine's cache. `read_manifest` and
   `inspect_manifest_version` look up (storage, bundle, sha256, size) before
   `stage_verified` and insert on a miss. `fetch_manifest_version_optional`
   stages from the store unconditionally and inserts. Production opens pass
   `object_cache()`; tests that opt out pass `None` at open. Call sites:
   seven `byte_source` calls in `project_sync/service.rs`, four in test
   modules, and the engine's own tests.

7. **Transport cleanup in the same files.**

   `get_metadata` reads a bounded body into memory instead of staging a
   temporary file and re-hashing it; `stage_verified` keeps the file path.
   Both share the response-header checks. `upload_window` serializes the
   commit arguments once and sends the bytes; success entries are decoded with
   `serde_json::from_value`.

**Loopback mock changes** (`dropbox_store/mock.rs`).

- On a batch commit, create missing ancestor folders for the committed path,
  as Dropbox does. If an ancestor is a file, return
  `path/conflict/file_ancestor` for that entry.
- `files/get_metadata` and `files/download` below a file ancestor return
  `path/not_found`.
- Reject `files/create_folder_v2` the way `files/upload` is rejected.
- Add `active_downloads`, `max_active_downloads`, and a `download_barrier`,
  waited on outside the state lock like `start_barrier`.
- Keep `requests`, `routes`, and `metadata_paths`; the retargeted tests assert
  against them.

**Tests.**

Retarget:

- `thirty_nine_payloads_use_thirty_five_parent_checks_and_eighty_one_requests`
  becomes 39 starts, 7 commits, 46 requests, and no `files/get_metadata`.
- `fresh_and_mixed_parent_trees_are_checked_once_per_window` is replaced by a
  fresh-namespace commit whose parents the mock creates, with
  `files/create_folder_v2` absent from `routes`.
- `unknown_batch_commit_verifies_all_immutable_objects_without_replay` asserts
  3 `files/get_metadata` and no `files/download`.
- `bundle_engine.rs` line 2254 and `project_sync/service.rs` line 19358 keep
  their marker assertions by opening through `ConfiguredObjectStore::shared` or
  calling `ensure_storage_marker` explicitly.
- Every `byte_source(_, None, None)` call takes the new signature.

Add:

- A head, manifest, tag, marker, or payload read is exactly one request.
- `stage_many_verified` and `get_metadata_many` reach the configured
  concurrency at 1, 2, 6, and 16 with the download barrier, keep input order,
  and abort the remaining downloads on the first failure.
- A file occupying `/.mallard/v1`: reads below it report `Missing`; a commit
  below it fails with `Invalid`; nothing is written.
- `shared(config)` twice returns the same store and performs one marker read;
  a config with a rotated secret replaces the entry; the store remains valid
  after a storage clear.
- Publish against an unchanged head token reads the head once and the manifest
  zero times; against a moved head it reads the manifest once. The
  `CountingStore` gains a `put_many` override so the manifest-plus-commit call
  is counted as one call with two objects.
- A retry after a failed head swing re-stages every payload, receives a
  conflict for each, proves each by metadata, downloads no body, and leaves
  the published head unchanged.
- The two-user publish, review, and restore scenario in `bundle_engine.rs`
  asserts the per-operation request counts from the table below.
- Setup probe recovery, Library publication, and marker creation still pass
  through the transport with no folder creation request.

**Acceptance gates.** Verified by the loopback mock; they exclude
authentication and TLS. Live timing must confirm the improvement; the counts
do not establish speed.

| Measure | Current | Planned |
| --- | ---: | ---: |
| Requests per head, manifest, or payload read | 5-6 | 1 |
| Requests per write window of six | 7 | 2 |
| Activity refresh, both invokes | 26 | 4 (3 with a manifest cache hit) |
| Push review | 13 | 2 |
| Push apply, no new payloads | 55 | 10 |
| Push apply, 39 new payloads | 104 | 24 |
| Pull review, 38 transcripts | 241 | 9 |
| Marker reads per process per storage | 2 per engine open | 2 once |
| Body bytes downloaded to prove a collision | full object | 0 |

For reference, the same operations on R2 are 4, 2, 7, 14, and 10 requests.

**Validation.**

```sh
cargo test -p mallard-infrastructure --lib dropbox --locked
cargo test -p mallard-infrastructure --lib bundle_engine --locked
cargo test --locked -p mallard-infrastructure -p mallard-application -p mallard --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path src-tauri/Cargo.toml --test storage_catalog_e2e
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

After the user resets local state and cloud data: fresh Dropbox setup, one
Activity refresh, one Push of the 39-object fixture, one Pull on a second
machine or profile, one changed-object Push, and one Push retried after a
forced head conflict. Record the `inspect remote head`, `publish: *`, and
`compare conversations` phase lines from the activity log for each, next to the
same runs on R2 under the same build and concurrency setting. Use a release
build for timing.

| File area | Planned change |
| --- | --- |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs` | Direct reads, listing, and deletion; bulk reads; in-memory metadata reads; `file_ancestor` mapping |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/upload.rs` | No parent check; metadata collision proof; hash in preflight; single serialization |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/probe.rs` | Direct metadata in cleanup |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/mock.rs` | Parent auto-creation, file-ancestor errors, download counters and barrier, rejected folder route |
| `crates/mallard-infrastructure/src/storage_core/dropbox_store/tests.rs` | Retargets and the new request-count, concurrency, and fail-closed tests |
| `crates/mallard-infrastructure/src/storage_core/mod.rs` | `ConfiguredObjectStore::shared`; `Clone`; marker verified at entry creation; `from_config` and the clear-time marker call removed |
| `crates/mallard-infrastructure/src/bundle_engine.rs` | No marker at open; engine-owned cache; observed manifest reuse; one batch for manifest and commit; cached manifest reads; two-user scenario counts |
| `crates/mallard-infrastructure/src/project_activity.rs` | Shared store |
| `src-tauri/src/project_sync/service.rs` | Shared store; `Match { token, manifest }`; `byte_source` signature; cache at open |
| `src-tauri/src/storage_catalog/service.rs`, `resource_engine.rs` | Shared store; marker call removed |
| `docs/DESIGN_DROPBOX_SHARED_TEAM_NAMESPACE_STORAGE.md` | Replace the parent-check and 35-check statements (lines 97-101, 186-188) with decision 1 |
| `docs/PLAN_DROPBOX_PUSH_PERFORMANCE.md` | One line at step 2 noting it is superseded by this plan |

**Not in this plan.**

- Deduplicating prefetch by object digest rather than logical path
  (`docs/AUDIT_BACKEND_PERFORMANCE.md`, finding 4). Independent of transport.
- A sliding transfer window that starts the next object when a slot frees.
  The batch commit amortizes over six objects; not worth the ordering rules.
- Sharing one remote snapshot between the activity and comparison invokes
  (`docs/AUDIT_REFRESH_PULL_PUSH_2026-09-06.md`, finding 4). Step 6 removes
  the duplicate manifest download; the duplicate head read remains one request.
- Storage clear on Dropbox: one `files/delete_v2` per object and a full
  re-listing per page. Needs `files/delete_batch` and a separate plan.
- The development profile's opt-level for workspace crates
  (`docs/PUSH_SLOWNESS_2026-09-05.md`).

**Deltas from the plan as written, 2026-09-09.**

- Decision 4's observed-manifest reuse rides on decision 6 instead of a new
  `PublishExpectation` shape: the manifest read inside publish is a cache hit
  whenever that manifest was inspected in this process, so `Match(HeadToken)`
  and every request literal stayed unchanged.
- Storage clear kept its own marker check. It is destructive and rare, and
  the R2 test proving it refuses an invalid marker before deleting stands.
- The engine gained a `with_object_cache` builder rather than a new `open`
  parameter; `byte_source(snapshot, on_progress)` lost its cache argument and
  `ObjectCache::global()` replaced the service-local accessor.
- Tests never install non-default settings, so the bulk-read concurrency
  test asserts the default transfer bound only.
- An invalid expected digest handed to `stage_verified` now maps to the
  transport's invalid-path error instead of the domain validation text.
