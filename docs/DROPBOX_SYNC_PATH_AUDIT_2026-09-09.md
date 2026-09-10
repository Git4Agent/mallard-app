# Dropbox push, pull, and compare path audit - 2026-09-09

Scope: the Dropbox transport (`crates/mallard-infrastructure/src/storage_core/dropbox_store.rs`,
`dropbox_store/upload.rs`, `dropbox_auth.rs`) and the project-sync call chains that
drive it (`src-tauri/src/project_sync/service.rs`, `bundle_engine.rs`,
`project_activity.rs`). Working tree at f9628b0 plus uncommitted changes, which
include the whole Dropbox transport and the 2026-09-08 upload-session cutover.

Code audit only. No application code was changed. No live Dropbox request was
made. The only storage configured in `~/.mallard/sync_config.json` today is a
local folder, so the activity log holds no Dropbox phase timings; every number
below is a request count derived from the code and cross-checked against the
repository's own Dropbox mock tests. Latency estimates are labeled as such.

## Summary

The Dropbox path is slow by construction, not by bytes. Every object read costs
five or six sequential HTTPS round trips and every write window costs seven,
because the transport walks and re-validates each parent folder on every call.
The service layer then multiplies that: it opens a fresh store (new TLS pools,
new OAuth refresh, storage-marker read) one or two times per user action, reads
the same head and manifest twice per push, and never uses the bulk read
overrides that R2 has, so Pull downloads one object at a time.

| Operation (Dropbox) | Sequential round trips | Of which parent-folder probes | Same operation on R2 |
| --- | ---: | ---: | ---: |
| Activity refresh (activity + comparison invokes) | 26 (two chains of 13) | 20 | 4 |
| Push review | 13 (+6 if remote advanced, +6 with a project-content base) | 10 | 2 |
| Push apply, 0 new payloads | 55 | 41 | 7 |
| Push apply, 39 new payloads (window 6) | 104 | 76 | 14 |
| Pull review, 38 transcripts | 241 | 200 | 10 |

R2 counts assume one request per object, six-way concurrent bulk reads, and the
same marker check. If a Dropbox API round trip costs 150-300 ms from a laptop
(assumption, not measured here), the 104-request push spends 16-31 s waiting on
request latency before a single payload byte is counted.

## Findings

Priority: P1 = dominates the user-visible latency of the named path; P2 = a
normal performance fix; P3 = cleanup.

**1. [P1] Every read walks and re-validates every parent folder.**

`download()` calls `parents(path, false)` before `files/download`
(`dropbox_store.rs:428`). `check_parent_paths` issues one `files/get_metadata`
per ancestor, sequentially (`dropbox_store.rs:399-422`). Ancestor counts: head 4,
manifest 5, payload object 5, discovery tag 4, storage marker 1. So one head read
is 5 round trips, one manifest 6, one payload 6. `delete_path` and `enumerate`
do the same walk (`dropbox_store.rs:479`, `:537`).

What the walk can detect: an ancestor whose display name differs in case from
the canonical lowercase name, or a file occupying a folder name. Dropbox paths
are case-insensitive, so the walk never changes which bytes `files/download`
returns; a file where a folder is expected already fails the download with a
path error. The check is a display-case policy, and it costs 4-5 round trips per
object on the read path where nothing is written.

The write side pays the same per window: `upload_window` re-checks the distinct
parents of every window under the commit gate (`upload.rs:116-119`). The
repository's own acceptance test pins the cost: 39 payloads produce 35
`files/get_metadata` calls out of 81 requests, and each of the 5 parent paths is
checked exactly 7 times (`dropbox_store/tests.rs:479-491`). The plan chose this
on purpose ("Discard the parent-check set when the window ends",
`docs/PLAN_DROPBOX_PUSH_PERFORMANCE.md` step 2). That decision sets the latency
floor of the whole transport and deserves a re-decision with numbers.

Fix: validate the fixed namespace skeleton (`/.mallard`, `/.mallard/v1`,
`/.mallard/v1/repositories`, `_tags`) and each bundle folder once per store
instance, which with finding 3 means once per user action. Drop the walk from
reads entirely, or keep it only on the first read of a bundle folder per store.
Keep the exact-name check itself; change only how often it runs.

Check: a Pull of N objects must issue N `files/download` calls and at most one
`files/get_metadata` per distinct parent folder; retarget the
`metadata_paths.values().all(|count| *count == 7)` assertion to `== 1`.

**2. [P1] Pull and Connect read objects one at a time.**

`DropboxObjectStore` implements neither `stage_many_verified` nor
`get_metadata_many` (`dropbox_store.rs:618-753`), so both fall back to the
sequential default loop in `object_store.rs:210-236`. `EngineByteSource::prefetch`
batches by `cloud_transfer_concurrency` (`bundle_engine.rs:273-286`) and hands
each batch to the store, but on Dropbox a batch of six is six sequential
downloads, each with its own five-probe parent walk. The S3 store overrides all
three bulk methods with a semaphore (`s3_store.rs:921-1100`). The upload side of
Dropbox already has the right shape: a `JoinSet` bounded by the same setting
(`upload.rs:73-89`).

Affected callers: Pull review transcript prefetch (`service.rs:5085`), Pull apply
payload prefetch (`service.rs:10706`), file apply prefetch
(`bundle_engine.rs:5275`), and Connect's tag reads through `list_remote_bundles`
(`bundle_engine.rs:1250`).

Fix: implement `stage_many_verified` and `get_metadata_many` on
`DropboxObjectStore` with the existing `JoinSet` pattern, bounded by
`cloud_transfer_concurrency`. Combined with finding 1, a 38-transcript Pull drops
from about 241 sequential round trips to about 8.

Check: the mock's `max_active` counter for `files/download` must reach the
configured concurrency during a prefetch; result order must match input order.

**3. [P1] A new store, new TLS pools, and a new OAuth refresh per engine open.**

`engine_for_storage_config` builds `ConfiguredObjectStore::from_config` and
`BundleEngine::open` for every call (`service.rs:13666-13671`). For Dropbox that
means: a new `reqwest::Client` for the API and content hosts
(`dropbox_store.rs:297-304`), a second `reqwest::Client` inside `DropboxClient`
for the token endpoint (`dropbox_auth.rs:116-127`), an empty
`DropboxAuthorization` cache so a refresh-token credential performs one
`oauth2/token` round trip before its first request (`dropbox_auth.rs:347-362`),
and `ensure_storage_marker`, which is a 2-round-trip download of
`.mallard/_storage.json` (`marker.rs:49-55`, `bundle_engine.rs:499`).

Opens per user action: Activity refresh opens two stores, one in
`fetch_activity_snapshot` (`project_activity.rs:584-585`) and one in
`load_project_remote_read` (`service.rs:6736`); Push review, Push apply, Pull
review, and Pull apply open one each (`service.rs:7550`, `:9823`, `:10327`,
`:3266`). Every open repeats the TLS handshakes and marker read; a refresh-token
storage also repeats the OAuth exchange.

Fix: cache the opened `ConfiguredObjectStore` per (storage id, config revision)
in the service, the way `object_cache()` is already process-wide
(`service.rs:1031`). Verify the marker once per cached store. The Dropbox store is
`Clone` and shares its client and authorization through `Arc`, so nothing in the
transport needs to change.

Check: two consecutive comparisons against one storage must produce one
`users/get_current_account`-free session with zero extra `oauth2/token` calls
and one marker read in the mock.

**4. [P2] Push reads the head and manifest twice, and the same tag twice.**

`push_bundle_reviewed_with_progress` calls `inspect_optional_with_token` (head 5
+ manifest 6 round trips, `service.rs:9824`), then `publish_with_progress` calls
`read_head` and `read_manifest` again (`bundle_engine.rs:711`, `:763`). The head
re-read is defensible: the engine grafts onto a head moved by a disjoint-scope
publisher. The manifest re-read is not: when the fresh head's etag equals the
observed token, the immutable manifest named by `manifest_sha256` cannot differ
from the one already in memory. That is 6 wasted round trips per push.

`publish_tag` then downloads the whole tag through the 5-round-trip read path
only to learn its etag for the CAS (`bundle_engine.rs:1713`), and the CAS itself
is a 6-round-trip write window (`dropbox_store.rs:684-722`). Eleven round trips
to update a 200-byte discovery object.

Fix: pass the observed snapshot into `PublishBundleRequest` and skip
`read_manifest` when the head token matches. For the tag, either keep the last
etag from the previous publish in the request or add a metadata-only read to the
trait (Dropbox has `files/get_metadata`, one request, no parent walk).

**5. [P2] Manifest and commit records are two write windows instead of one.**

`publish_with_progress` calls `put_immutable` for the manifest and again for the
commit (`bundle_engine.rs:1008`, `:1032`). Each is a full Dropbox window: session
start, 5 parent probes, batch commit, 7 sequential round trips, 14 in total. One
`put_many` with both objects is one window: two parallel starts, 6 distinct
parents, one commit, 8 round trips. Publication order is preserved because both
land before the head CAS. Keep the head CAS separate; `finish_batch_v2` is not
atomic across entries, so a head that lands beside a failed manifest would be
visible.

**6. [P2] Immutable-collision verification downloads the whole object.**

`put_many_async` treats a `Conflict` on an `add` as "already present" and proves
it by downloading the object in full and hashing it (`upload.rs:192-205`). The
success path already trusts Dropbox's `content_hash` against the locally
computed one (`upload.rs:138-141`), and `files/get_metadata` returns that same
`content_hash` in one request without a body. The download is a stronger-looking
check that adds nothing the metadata comparison lacks.

This matters on the retry path: after a push fails at the head CAS, every
payload has landed but the local state does not know it, so the next push
re-uploads all of them, receives a conflict for each, and then downloads each one
again. A 150 MB failed push costs 150 MB up and 150 MB down on retry.

Fix: on `Conflict`, call `metadata(path)`, require `size` and `content_hash` to
match the values computed for the staged upload, and return `AlreadyPresent`.
Keep the full download for the `Unknown` case if a stricter proof is wanted
there.

**7. [P2] Pull fetches the same immutable base manifest up to three times.**

`plan_bundle_restore_with_progress` reads the reviewed base manifest through the
continuity probe when the remote advanced (`service.rs:10342`,
`fetch_manifest_version_optional`) and again through
`project_content_base_content_digests` (`service.rs:2707`,
`inspect_manifest_version`); Pull apply reads it a third time
(`service.rs:10484`). Push apply reads it in `build_project_files_inventory`
(`service.rs:2340`). Each read is 6 Dropbox round trips. The manifest is
immutable and addressed by size and SHA-256, so one verified read per operation
is enough. The probe must witness the store, but the digest read that follows it
can reuse the probe's bytes.

**8. [P3] Control objects take the streaming payload path.**

`get_metadata` downloads through `download()` into a `NamedTempFile`, computes
SHA-256 and the Dropbox block hash while streaming, wraps the file in a
`VerifiedBody`, and then reads it back with a third hash in `read_bounded`
(`dropbox_store.rs:619-640`). For a 1 KB head this is a temp-file create, write,
read, and unlink per read. Not a latency cost; it is one file descriptor and one
disk write per head, tag, and marker read, which adds up under the packaged
app's 256-descriptor limit when combined with the staged payload bodies.

**9. [P3] Small redundancies inside `upload_window`.**

`serde_json::to_vec(&args)` is computed once to test the RPC limit and the same
value is serialized again by `.json(&args)` (`upload.rs:111`, `dropbox_store.rs:373`).
Each success entry is serialized back to bytes and re-parsed instead of
`serde_json::from_value` (`upload.rs:134-136`). The Dropbox content hash of each
body is computed inside the spawned upload task (`upload.rs:44`), so up to six
150 MiB SHA-256 passes run on Tokio worker threads while the same runtime is
driving the in-flight transfers; computing it during the preflight in
`Upload::new` on the calling blocking thread keeps the runtime free. All three
are CPU-trivial next to the round-trip costs above.

**10. [P3] `enumerate` re-lists the whole subtree for every page.**

The list cursor is the last returned key, and each page call performs the full
recursive `files/list_folder` walk again before slicing in memory
(`dropbox_store.rs:550-611`). `list_remote_bundles` asks for one page of 10,000
and is unaffected; `clear_managed_data` pages by 1,000 and lists the repository
subtree once per page. Only Clear pays for it today.

## Suggested order

1. Finding 1 (parent walk once per store) and finding 3 (cache the opened store
   per storage). Together they turn a 6-round-trip read into 1 and remove the
   per-action TLS, OAuth, and marker cost. Both are contained in the transport
   and the two engine-open helpers.
2. Finding 2 (bulk read overrides). Small, mirrors existing upload code, and it is
   the whole Pull story.
3. Findings 4 and 5 (publish round trips). About 17 round trips per push.
4. Findings 6 and 7 (redundant full downloads).
5. Findings 8-10 as cleanup.

Retarget these tests when changing the transport:
`thirty_nine_payloads_use_thirty_five_parent_checks_and_eighty_one_requests`,
`fresh_and_mixed_parent_trees_are_checked_once_per_window`, and the
`state.routes["files/download"] == 3` assertion at `dropbox_store/tests.rs:717`.

## Out of scope, noted in passing

- The activity log shows one local-storage comparison whose "compare
  conversations" phase took 58.157 s (run `history-01ef4fc9347fc1f76fd7d8f0`)
  against a typical 0.1-0.3 s. That is the local observation path, not the
  transport.
- `Cargo.toml` still optimizes only dependencies in the dev profile; the
  workspace crates that contain this code run at opt-level 0 in the development
  app (see `docs/PUSH_SLOWNESS_2026-09-05.md`). Dropbox request depth is
  independent of that, but any timing taken from a dev build will overstate the
  CPU phases.
- Findings 3 (partly), 4 (partly), and 7 restate items from
  `docs/AUDIT_REFRESH_PULL_PUSH_2026-09-06.md` (its findings 3 and 4) that are
  still open; on Dropbox each of them is 6-13 round trips instead of 1-2.
