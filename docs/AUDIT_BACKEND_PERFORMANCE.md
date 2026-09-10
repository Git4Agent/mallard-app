# Backend performance audit

Date: 2026-09-07.

Scope: current Rust backend, including project sync, session browsing, storage transport, Library inventory, activity logs, and background watchers. This audit did not change application code. Existing uncommitted work was preserved.

The main costs are repeated reads and parsing, retained resource bytes, and duplicate remote requests. Worker threads prevent direct UI-thread execution for the reviewed commands. They do not reduce these costs.

Priority: P1 means fix first for the stated large-data case. P2 means a normal-priority performance fix. The seven findings below are supported by current command paths. Fixture timings use existing debug libraries and warm local files. They are not release-app or S3 latency measurements.

**1. [P1] Library listing retains the contents of every resource.**

`scan_target_root()` captures every resource into a candidate list. The inventory retains each capture until the command returns. There is a per-resource limit but no aggregate byte limit. A skill can contain 16 MiB. A scope with 128 skills near that limit can therefore retain about 2 GiB of file contents. This is a code-based estimate, not measured RSS.

Preview preparation adds avoidable work. It hashes and copies every captured file before the codec is asked for a preview. The default skill preview returns no image.

Sources: [inventory capture](../src-tauri/src/storage_catalog/inventory.rs), lines 241–345; [preview preparation](../src-tauri/src/storage_catalog/service.rs), lines 1257–1284; [payload copies](../src-tauri/src/storage_catalog/codec.rs), line 75; [skill limits](../src-tauri/src/storage_catalog/agent_skill.rs), lines 31–39.

Fix: process one resource at a time. Retain summaries and verified digests for listing. Release file bytes after each resource. Read preview bytes only when needed. Capture selected resources for approved operations and retain all digest checks. Add a bound on aggregate scan work.

Check: list many valid large skills. Measure peak retained bytes. Verify that a skill without a preview causes no preview payload copy.

**2. [P2] Claude timeline pages repeat full metadata scans.**

Session detail, timeline pages, and expanded events call `resolve_local_session()`. It calls `scan_metadata()` before using the timeline cache. For a Claude transcript with one session ID, the 256 KiB tail window cannot prove ownership. The adapter then scans the complete file. A second request repeats this work.

Sources: [ownership resolution](../crates/mallard-infrastructure/src/session_detail.rs), line 825 and lines 1670–1673; [Claude scan](../crates/mallard-infrastructure/src/providers/claude/mod.rs), lines 68–74 and 159–162.

The fixture probe called the real adapter twice for each valid, single-session JSONL file:

| Actual file bytes | Bytes read on each call | First call | Second call |
|---:|---:|---:|---:|
| 1,052,770 | 1,314,914 | 0.962 ms | 0.733 ms |
| 10,489,790 | 10,751,934 | 4.648 ms | 4.757 ms |
| 52,429,942 | 52,692,086 | 20.853 ms | 20.947 ms |

Each call read the full file plus 262,144 tail bytes. The fixture used simple records with 4 KiB text fields. These measurements establish repeated work; they do not predict latency for complex real transcripts.

Fix: reuse the ownership proof for the same validated file snapshot. Bind the proof to file identity, canonical path, change stamps, size, and provider/profile revision. Keep no-follow checks and invalidate the proof on source changes.

Check: a second page or event expansion must not repeat the complete metadata scan. Append, replacement, and ownership changes must invalidate the proof.

**3. [P1] The timeline cache can repeatedly rebuild indexes after it reaches its byte limit.**

Every detail, page, and expansion reads the installation-wide `session_timeline_cache.json`. A cache hit also validates and clones the complete selected index. Thus, reading a small page depends on all cached sessions and the full selected index.

The cache removes entries only after the count exceeds 50,000. The writer has a separate 32 MiB byte limit. If a new index exceeds that byte budget, the write fails and the old cache remains. The new index is lost at the end of the request. The next page misses again and rebuilds both its index and summary. This repeats until the cache contents or source change enough to permit a write.

Source: [timeline cache](../crates/mallard-infrastructure/src/session_detail.rs), lines 909–934, 998–1025, and 1041–1104.

Fix: store indexes by session snapshot and retain a bounded process cache of validated indexes. Apply byte-based eviction before writing. Handle one oversized index explicitly. Do not retry an impossible cache write on every page.

Check: fill the cache near 32 MiB, add a long session, then read its next page. The index must be reused. Opening other sessions must not increase the bytes read for that page.

**4. [P2] Cold Pull prefetch downloads shared objects more than once.**

Prefetch deduplicates logical paths, not object identities. It builds the complete pending list before transfers populate the shared cache. Different paths with the same digest therefore create duplicate downloads, including across transfer batches.

Source: [bundle prefetch](../crates/mallard-infrastructure/src/bundle_engine.rs), lines 305–379.

A fixture used eight identical ordinary project files and the public bundle engine interfaces. The published snapshot passed validation. Publication wrote one distinct payload. Cold prefetch staged that payload eight times with the default concurrency of six. Warm prefetch staged zero objects.

Fix: group pending paths by storage, bundle, digest, and size. Fetch and verify each object once. Share its verified body across all matching paths. Count unique transfers in progress totals.

Check: eight paths that share one object must cause one cold fetch. Repeat the check across a batch boundary and preserve digest-mismatch rejection.

**5. [P2] Timeline pages perform Git checkpoint work that they do not need.**

Every page calls `checkpoint_support()` before checking the event filter. The helper starts Git processes, lists branches, and can read 10,000 first-parent commits. A checkpoint is added only after the final transcript event. Most pages cannot use the result.

Source: [checkpoint lookup](../crates/mallard-infrastructure/src/session_detail.rs), lines 1151–1211, 1693–1698, and 1738–1750.

Fix: resolve a checkpoint only when the filter includes it, no turn filter is active, and the transcript page has reached its end. Reuse detail checkpoint results with appropriate branch and transcript version checks.

Check: pages that exclude checkpoints or have more transcript events must execute no checkpoint Git commands.

**6. [P2] Activity-log pagination scans all retained logs while blocking writes.**

`query()` holds the log mutex and process file lock while it reads every segment, parses every line, stores all matches, and sorts them. Only then does it apply the page limit. The cursor does not bound the scan. Appends need the same locks. Progress-log emission appends synchronously, so a query can delay unrelated operations.

Sources: [query and append](../crates/mallard-infrastructure/src/activity_log.rs), lines 298–365; [event emission](../src-tauri/src/activity_log.rs), line 100.

| Retained rows | JSONL bytes | Requested rows | Query time |
|---:|---:|---:|---:|
| 10,000 | 8,360,000 | 1 | 9 ms |
| 10,000 | 8,360,000 | 500 | 6 ms |
| 100,000 | 83,600,000 | 1 | 97 ms |
| 100,000 | 83,600,000 | 500 | 78 ms |

The probe used the actual store and valid synthetic entries. The larger fixture used ten segments. Even a one-row request scanned the retained data. No valid RSS measurement was available.

Fix: use segment or entry indexes for pagination. Bound retained query results to the requested page. Take a stable read snapshot under the lock, then parse outside the append lock. Preserve retention and cursor ordering.

Check: measure scanned bytes and concurrent append latency as retained data grows. Response length alone is not a sufficient check.

**7. [P2] Cached stored-pet previews repeat remote head and manifest reads.**

The scope listing has already inspected the repository. Each preview command creates another engine and reads its head and manifest again. It checks the local preview cache only afterward. The frontend requests previews in sequence. N stored-only cards therefore add 2N remote reads even when the image cache is populated. A cold cache also needs atlas downloads.

Sources: [preview command](../src-tauri/src/storage_catalog/service.rs), lines 1300–1321; [engine inspection](../src-tauri/src/storage_catalog/resource_engine.rs), line 196; [preview cache](../src-tauri/src/storage_catalog/pet_preview.rs), line 92; [caller](../src/components/storage-catalog/StorageCatalogPage.tsx), line 992.

Fix: reuse the verified resource snapshot from listing, keyed by its immutable manifest digest. Serve cached previews from that snapshot and fetch only missing atlases. Retain fresh head and digest checks for review and apply.

Check: loading cached previews for many cards must not repeat head and manifest reads per card.

**Additional scale costs**

- Unbound session lookup rebuilds the binding and receipt alias maps for every discovered session. Build these maps once per validated request. See [service](../src-tauri/src/project_sync/service.rs), lines 1590–1600, and [alias resolution](../crates/mallard-infrastructure/src/project_activity.rs), lines 627–704 and 885–889.
- S3 publication computes SHA-256 in capture, expected-object construction, and conditional PUT construction. Pass a verified digest through the immutable payload interface. Keep collision verification and the upload checksum. See [payload](../crates/mallard-infrastructure/src/payload.rs), line 21; [S3 store](../crates/mallard-infrastructure/src/storage_core/s3_store.rs), lines 656, 699, and 1223–1227.
- Transfer batches wait for all objects in a batch before starting the next batch. Mixed object sizes can leave transfer slots idle. Use a bounded queue that starts the next object when a slot becomes free. See [bundle engine](../crates/mallard-infrastructure/src/bundle_engine.rs), lines 344 and 976.
- Pull commits global JSON documents for each successful session. Cost grows with both batch size and retained state. Measure JSON bytes and lock duration before changing persistence. Preserve independent session success, revision checks, and durable receipts. See [apply](../src-tauri/src/project_sync/service.rs), lines 11407–11425, and [commit](../crates/mallard-infrastructure/src/persistence.rs), lines 237–281.

**Verification and limits**

These selected checks passed: 24 tests total.

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- \
  a_comparison_reads_no_ comparisons_capture_no_payloads \
  comparison_does_not_persist_observed_checkpoint_state

cargo test --locked -p mallard-infrastructure --lib -- \
  session_detail::tests publish_dedupes_identical_bytes_within_one_push \
  prefetch_deduplicates_paths_and_preserves_order_and_progress
```

The refresh tests confirm useful existing behavior: unchanged comparisons avoid full transcript capture, appended data uses bounded observation, and comparisons do not persist checkpoint changes. The transport tests cover duplicate paths and publication deduplication. They do not cover different paths that share one prefetched object.

The three audit probes used generated temporary data and existing compiled library code. They accessed no real provider homes or Mallard state. The audit did not run the full test suite, profile the packaged application, or measure live S3/R2 latency.

`sync_metrics` already counts discovery, transcript reads, parsing, hashes, and cache reuse. Extend it with timeline-cache bytes, metadata scan reuse, Git process counts, unique object transfers, retained inventory bytes, and log lock duration. Use these work counts for regression tests. Use release-build latency and RSS measurements to set product performance targets.
