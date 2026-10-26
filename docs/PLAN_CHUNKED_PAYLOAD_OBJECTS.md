# Chunked payload objects: larger files on every storage provider

- Status: Layer 1 implemented on 2026-10-25 (section 13); Layer 2 not started
- Date: 2026-10-25
- Decisions taken on 2026-10-25: 32 MiB chunks; 2 GiB per project-content
  file, which puts the Push/Pull aggregate at 2 GiB with it (section 7)
- Scope: the project bundle's payload objects on Push and Pull, the capture
  of ordinary project-content files, and the size limits those carry. Every
  storage provider (local folder, S3/R2, Dropbox, Feishu, Mallard Cloud).
- Out of scope: the Library/catalog protocol (its files are at most 8 MiB and
  it keeps its own engine); conversation transcripts and other provider-home
  files, whose limits are bound to in-memory parsing and scrubbing, not to
  transport (see section 9).
- Compatibility: clean cutover under the repository policy. Every schema
  number stays `1`. A manifest file entry gains a second representation; no
  migration, no fallback reader, no feature flag. Section 3.3 lists what
  changes shape and what does not, including the Mallard Cloud Worker.

## 1. What limits a file today

A payload travels as one object at `_objects/<sha256>`, built from one
in-memory allocation (`PayloadBytes`, `crates/mallard-infrastructure/src/payload.rs`)
from capture through `ObjectStore::put_many`. Three families of limits sit on
that path.

| # | Limit | Value | Where |
|---|---|---|---|
| 1 | One project-content file | 100 MiB | `crates/mallard-domain/src/lib.rs:58`, enforced at discovery (`provider_capture.rs:1293`), at read (`:1468`), and in manifest validation (`lib.rs:4571`) |
| 2 | One conversation file | 100 MiB | `lib.rs:56`; the transcript is parsed in memory everywhere it is read |
| 3 | One other provider-home file | 16 MiB | `provider_capture.rs:28`; scrubbed in memory at capture |
| 4 | One Push or Pull, aggregate | 512 MiB | `provider_capture.rs:33` (`MAX_CAPTURE_BYTES`), the review meter and the apply recheck in both directions |
| 5 | One Dropbox object | 150 MiB | `storage_core/dropbox_store.rs:38`; `upload_session/start` carries the whole body in one request |
| 6 | One Feishu object | 100 MiB (one request up to 20 MiB, parts above) | `storage_core/feishu_store.rs:38-40` |
| 7 | One Mallard Cloud object | 512 MiB at the Worker since 2026-10-19 (`validate/limits.ts`, `objectBytes`), but the hostname's request-body limit, set by the Cloudflare plan, caps a single `PUT` below that; the desktop still prints 100 MiB (`managed_cloud/client.rs:221`, `stub.rs:26`) | `mallardInternal/cloudflare/managed-cloud` |
| 8 | One S3/R2 object | one `PutObject`, no multipart | `storage_core/s3_store.rs:520-560` |
| 9 | Staged-object cache | 256 MiB; a larger body is never cached | `bundle_engine.rs:176`, `:236` |

Rows 5 to 8 are why the per-file limit cannot simply be raised: three of the
five providers refuse an object above 100 or 150 MiB, and the fourth would
need its own multipart protocol. Row 4 and the in-memory `PayloadBytes` are
why memory is not bounded today: a Push of 512 MiB holds 512 MiB.

The download side is already bounded. `stage_verified` streams each object
into a private temporary file and hands back a file-backed `VerifiedBody`
(`storage_core/object_store.rs:72`); restore copies it to the target in
64 KiB reads (`bundle_engine.rs:6211`). Only the upload side and capture hold
whole payloads in memory.

## 2. Decision

**Split large payloads in the bundle model, not in the stores.** A payload
larger than `PAYLOAD_CHUNK_BYTES` (32 MiB) is published as an ordered list of
content-addressed chunk objects, each at `_objects/<chunk-sha256>`, and the
manifest entry lists the chunks. The `ObjectStore` contract stays put, get,
and list; a chunk is an ordinary immutable object, indistinguishable from a
small file's object. A 300 MiB file becomes nine 32 MiB objects and one
12 MiB object.

Why here and not elsewhere:

- *Provider-native multipart* (S3 multipart, Dropbox append sessions, Feishu
  parts) is per provider, leaves the Mallard Cloud server's 100 MiB ceiling in
  place, and gives the reader nothing to verify against until the whole object
  is down. Every provider would need its own code and tests for the same
  feature.
- *Transparent chunking inside each store* (the store splits at `put` and
  joins at `get`) hides the chunk digests from the manifest, so the reader
  cannot verify a chunk before assembly, and a "version complete" listing
  cannot see chunks. It also puts the same logic in five stores.
- *Derived chunk keys* (`_objects/<sha256>/<n>`, no per-chunk digest) make
  the chunk size a permanent wire constant and nest keys under an object key,
  breaking the flat `_objects/<sha256>` listing model.

Listing chunks in the manifest keeps every existing invariant: objects are
immutable and content-addressed, every read is verified against an exact
size and digest before anything opens it, and the manifest — not the key —
says what a file is.

## 3. The object model

### 3.1 Constants (domain)

```text
PAYLOAD_CHUNK_BYTES = 32 MiB           # the writer's split size
MAX_PAYLOAD_CHUNKS  = ceil(MAX_PROJECT_CONTENT_FILE_BYTES / PAYLOAD_CHUNK_BYTES)
```

32 MiB is under every provider's single-request ceiling with room to spare,
and keeps the chunk count small (a 2 GiB file is 64 chunks, about 6 KiB of
manifest). Feishu sends anything above 20 MiB through its existing
prepare/part/finish path, so a 32 MiB chunk costs it four requests instead of
one; a 20 MiB chunk would make every provider single-request at the cost of
60 % more objects. 32 MiB is the recommendation; the choice is a constant.

### 3.2 Manifest file entry

`BundleFileEntry` (`lib.rs:3982`) keeps `resource_id`, `sha256` (the whole
file), `size`, `source_mtime`, and `mode`, and gains one of two payload
locations:

```text
# one object, as today
{"resource_id": …, "sha256": <file>, "size": 12345, "source_mtime": …,
 "object_key": "_objects/<file-sha256>"}

# split
{"resource_id": …, "sha256": <file>, "size": 314572800, "source_mtime": …,
 "chunks": [{"sha256": <chunk-0>, "size": 33554432}, …,
            {"sha256": <chunk-9>, "size": 12582912}]}
```

In Rust: `object_key: Option<String>` and
`chunks: Vec<BundleChunk { sha256, size }>` (serde `default`, skipped when
empty). A chunk's key is derived, `_objects/<chunk-sha256>`, through the
existing `repository_object_key` constructor; chunk entries carry no key
field because the namespace is closed and the digest is the name.

`validate` enforces, in the domain crate:

- exactly one of `object_key` and `chunks` is present;
- `chunks` is present if and only if `size > PAYLOAD_CHUNK_BYTES`;
- every chunk but the last has `size == PAYLOAD_CHUNK_BYTES`, the last has
  `1..=PAYLOAD_CHUNK_BYTES`, the sizes sum to the entry's `size`, and the
  count is at most `MAX_PAYLOAD_CHUNKS`;
- every chunk `sha256` is a valid lowercase digest.

The rules are strict on purpose: the chunk list is a pure function of the
bytes, so two computers that push the same file publish the same objects,
and retained-file identity and within-push dedupe keep working. Changing the
chunk size later is a format change under the clean-cutover policy, not
something readers accommodate.

One accessor serves both representations and is the only place the two
shapes meet:

```text
BundleFileEntry::payload_objects() -> ordered [(relative key, size, sha256)]
```

For a single-object entry it yields one item; for a split entry, one per
chunk. Publish, prefetch, retained-identity checks, and tests go through it.
A `chunk_ranges(size)` helper beside the constant gives the writer and the
validator the same arithmetic.

### 3.3 What changes shape, and what does not

- **Schema numbers**: none. `BUNDLE_SCHEMA`, the machine-local schemas, the
  storage marker, and the publication record stay `1`, as every cutover in
  this repository has.
- **The manifest file entry** is the one portable shape that changes, and
  only for a split file. An entry with `object_key` and no `chunks`
  serializes byte for byte as today, so a manifest without a split file is
  identical to a current one, and every manifest already in a storage
  parses unchanged. A manifest that holds a split entry does not parse on an
  older Mallard: `object_key` is required there, so that version is
  unreadable as a whole, not file by file. Every machine sharing a storage
  updates before the first file above 32 MiB is pushed.
- **Machine-local state** under `~/.mallard` holds no file entries. Bases
  keep a manifest's size and digest (`RecipeBase`), receipts and bindings
  name files by logical path and digest, and the observation cache stores
  no manifest. Nothing there changes.
- **Frontend DTOs** do not mirror `BundleFileEntry` (`src/types.ts` has no
  `object_key`); review rows carry sizes and digests only. Nothing changes.
- **The Mallard Cloud Worker** validates every manifest it is sent and
  derives from it the object references that keep a publication's objects
  alive and counted. Its validator requires `object_key` to equal
  `_objects/<sha256>` on every file entry, so today it refuses a manifest
  with a split entry as `object_invalid`; and if it merely accepted the
  shape without learning the chunks, cleanup would delete them after a day
  as unreferenced. Section 3.4 names the files. The Worker ships before the
  desktop release that can push a split file; the in-repo stand-in
  (`managed_cloud/stub.rs`) is updated in step 4 to model it.

### 3.4 The Mallard Cloud Worker, concretely

The service lives in `mallardInternal/cloudflare/managed-cloud` (Worker in
`src/`, website in `web/src/`, contract in `STORE_API.md`). Its object
model already fits: a payload `PUT` streams straight to R2 with R2 checking
the digest (`routes/store.ts`, `createOnly`), every landed object gets an
`objects` row, and a publication's references are `publication_objects`
rows derived once from the manifest at record admission
(`store/index.ts`, `manifestFacts`). Cleanup (`jobs/cleanup.ts`), Discard
(`jobs/discard.ts`), reconciliation (`jobs/reconcile.ts`), the quota recount
(`COMMITTED_BYTES`), and the public object route all key on those rows, not
on the manifest text. So the change is small and almost entirely in one
place.

*Validator* (`src/validate/manifest.ts`, the file-entry loop near line
141). Accept exactly one of `object_key` equal to `_objects/<sha256>` and
`chunks`, with the same strict rules as the desktop: present only when
`size > 32 MiB`, every chunk but the last exactly 32 MiB, the last the
remainder, the sum equal to `size`, at most 64 chunks, each digest valid.
For a split entry, add each chunk's digest and size to `references`
instead of the whole-file digest; the "repeats digest with a different
size" check applies per chunk, and `referencesPerManifest` (20,000) now
counts chunks. `fileDigests` keeps the whole-file digest, which is what
session views are keyed by; a transcript is never split, since the
conversation limit stays at 100 MiB. Two constants in
`src/validate/limits.ts`: `payloadChunkBytes` and `payloadChunksPerFile`.

*Index.* `manifestFacts` already returns `[...references.keys()]`, so once
the validator lists chunks, `publication_objects` has one row per chunk,
the quota counts chunk bytes, cleanup and Discard protect them, and the
public form of `GET /objects/{sha256}` (`routes/content.ts:104`, which joins
`publication_objects`) serves each chunk. No code change there; the tests
below prove it.

*Download route.* The Worker never assembles a file for the desktop; the
desktop stages chunks itself. The website, though, links a project file's
Download to `objectUrl(sha256)` (`web/src/components/CloudProjectFile.tsx:76`,
`:146`, `ReleaseContent.tsx:43`), which has no object for a split file. Add
one route for split entries, `GET /v1/repositories/{id}/files/{commit_id}/
{path}` with its public form, that reads the stored manifest
(`routes/content.ts:83` already does), finds the entry, and answers one
response whose body pulls each chunk's R2 body in order, with
`Content-Length` equal to the entry's `size`. Workers stream without
buffering, so a 2 GiB file costs no memory. Single-object files keep the
object route.

*Website* (`web/src`). `FileEntry` in `types.ts:252` gains optional
`chunks` and an optional `object_key`; `projectFiles` is unchanged.
`loadObject` and `cloudMedia.ts`'s `load(file)` fetch a split entry's
chunks in order and concatenate them, still under `CLOUD_PREVIEW_BYTES`
(512 MiB), so a file above that is "too large to show" exactly as today
and offers Download only. Download anchors use the new route for a split
entry.

*Tests* (vitest, `npm test` and `npm run test:web`): `test/store.test.ts`
admits a manifest with a split entry and rejects a chunk list that is
unsorted, mis-sized, over-long, or beside an `object_key`;
`test/cleanup.test.ts` shows a listed publication's chunks survive a
day-old cleanup and a Discard that keeps that push, and that
`committed_bytes` equals the chunk bytes; `test/downloads.test.ts` streams
a split file in chunk order through the new route in both forms;
`web/test/project_files.test.ts` and `web/test/downloads.test.ts` cover the
rows and the Download link. `test/fixtures.ts:94` grows a split-file
helper.

*Contract text.* `STORE_API.md`: the per-class table row for
`_objects/<sha256>` and the paragraph after it that says the protocol
"does not split it into smaller uploads" (line 166); section 4's
`publication_objects` sentence (line 401) to say "every payload reference,
chunks included"; the new route. `README.md` near line 198 (per-file
preview limits). The architecture document's entry contract (line 1754 in
both copies).

*Deployment.* Validator, index tests, route, and website in one Worker
release, to staging then production, before the desktop release of Layer 1
reaches users. A desktop that pushes a split file to an older Worker gets
`object_invalid` at the manifest write and keeps every local acceptance,
as a failed Push does today; nothing is lost, the Push is simply refused.

*Two side notes from the Worker.* First, its per-object limit is already
512 MiB, but a single `PUT` is also bounded by the hostname's request-body
limit, which Cloudflare sets per plan; 32 MiB chunks make that limit
irrelevant on every plan, which is a second reason to chunk. Second, the
repository quota is 4 GiB (`repositoryQuotaBytes`), so two 2 GiB files
fill a repository; the quota is a product setting, not part of this plan.
The desktop's "100 MiB" text in `managed_cloud/client.rs:221` and the
stand-in's `MAX_OBJECT_BYTES` are stale against the Worker and are
corrected in step 4.

## 4. Push

In `publish_with_progress` (`bundle_engine.rs:1070-1100`), the prepared
capture for a file above the chunk size hashes each `Bytes::slice` of the
allocation (zero copy) and builds the split entry; the single-object branch is
unchanged. `pending_uploads` (`:1167`) is keyed by object digest as today,
so a chunk shared by two files, or a chunk whose bytes equal a small file,
is uploaded once. The upload call (`:1350`) and every store's transfer window
see only `(ObjectKey, Bytes)` pairs. Progress counts objects and bytes as
today, now per chunk, which also makes the meter move during one large file.

The retained-file check (`:1130-1160`) compares `(payload location, sha256,
size)` instead of `(object_key, sha256, size)`, where the location is the
entry's object key or its chunk list. `files_are_semantically_equal` and
`file_delta` compare whole-file digests and need no change.

The service layer compares captures to entries by whole-file digest and size
(`src-tauri/src/project_sync/service.rs:9505`, `:10769`) and is unchanged.

## 5. Pull

`EngineByteSource::prefetch` (`bundle_engine.rs:400-470`) builds its request
list from `payload_objects()` of every entry it needs, so a split file
contributes one `(key, ExpectedObject)` per chunk and every chunk is staged
and verified by the existing `stage_many_with_progress` contract. Then, per
split entry:

- any `Missing` chunk is `VERSION_INCOMPLETE`, as a missing object is today;
- a `Mismatch` names the chunk key in its error;
- the verified chunk bodies are assembled by a new
  `VerifiedBody::assemble(parts, size, sha256)` in
  `storage_core/object_store.rs`: it streams each part through `open()` into
  one new private temporary file while hashing, refuses anything but the
  exact whole-file size and digest, and returns a body through the existing
  `from_temp_file` path. The chunk bodies drop when assembly ends.

Consumers (`BundleBytes::body`, `read_bounded`, `apply_file_action`, the
restore plan) receive a whole-file `VerifiedBody` exactly as they do now. The
`ObjectCache` stores the assembled body under the whole-file `(sha256, size)`
key; chunk bodies are never cached. The cache still skips bodies above
256 MiB, so a Pull apply re-downloads such a file after its review; raising
`OBJECT_CACHE_MAX_STAGED_BYTES` is a separate choice.

Disk: a Pull stages its whole selection before it writes, so staging space is
the selection size plus, transiently, the largest split file while it is
assembled.

## 6. Memory, in two layers

**Layer 1 (sections 3 to 5) changes no memory profile.** Capture still holds
each selected file as one allocation, bounded by the 512 MiB aggregate; chunk
slices are views of it. What Layer 1 removes is every per-object provider
ceiling, so the per-file limit can rise to the aggregate.

**Layer 2 bounds memory to the transfer window.** Two changes:

1. *Sealed capture for project-content files.* `CapturedFile.bytes`
   (`provider_capture.rs:131`) becomes a `CapturedPayload`: `Inline` for
   files that are parsed or scrubbed in memory (transcripts, provider state),
   `Sealed` for a project-content file above the chunk size. A sealed file is
   opened no-follow once, hashed in a stream (whole file and per chunk), and
   remembered by canonical path, size, mtime, mode, and on Unix device and
   inode, like `VerifiedBody` remembers its staged file. The secret scan
   (`:2254`) becomes a streaming scanner over overlapping windows with the
   same markers and the same UTF-8 gate, so no check is weakened. The
   "changed while scanned" re-inspection stays.
2. *Lazy object sources at the store boundary.* `put_many` and
   `put_many_with_progress` take `(ObjectKey, ObjectSource)` where
   `ObjectSource` is `Bytes` or a `Slice { sealed file, offset, len, sha256 }`
   that loads itself inside the store's transfer worker: reopen, check device,
   inode, size and mtime, read exactly `len` bytes, verify the chunk digest,
   and only then send. A file edited after review fails the Push closed. The
   Dropbox preflight that hashes every upload before any I/O
   (`dropbox_store/upload.rs`, `Upload::new`) moves its content hash to load
   time; the other stores already materialize per worker.

Peak upload memory is then `cloud_transfer_concurrency × PAYLOAD_CHUNK_BYTES`:
192 MiB at the default of six, 512 MiB at the maximum of sixteen
(`app_settings.rs:23`), whatever the file or the selection size. Only after
Layer 2 is it safe to raise the aggregate budget.

## 7. Limits, by layer

| Limit | Today | After Layer 1 | After Layer 2 (recommended) |
|---|---|---|---|
| Project-content file | 100 MiB | 512 MiB (one file may fill a push) | 2 GiB (decided) |
| Push / Pull aggregate | 512 MiB | 512 MiB | 2 GiB (a 2 GiB file must fit one push) |
| Conversation file | 100 MiB | unchanged | unchanged |
| Other provider-home file | 16 MiB | unchanged | unchanged |
| Provider per-object ceilings | 100 / 150 MiB | no longer reach a file | — |

2 GiB per file is decided; it is reached only after Layer 2, because until
then a file is one allocation. The aggregate stays one constant for both
directions (`PLAN_PULL_APPLY_LIMIT.md`, R1) and rises to 2 GiB with the file
limit; the pull side is already disk-bound, so that number is a disk and
patience question, not a memory one. `MAX_PAYLOAD_CHUNKS` is then 64.

## 8. Provider notes

- **Local folder**: one file per chunk under `_objects/`; unchanged code.
- **S3/R2**: one `PutObject` per chunk with `if-none-match` and a SHA-256
  checksum, the JoinSet window as today (`s3_store.rs:915`). An ambiguous put
  re-stages the chunk, as any object.
- **Dropbox**: one `upload_session/start` per chunk, committed per window
  with `finish_batch_v2`; at most sixteen per batch, as today. Nothing near
  150 MiB.
- **Feishu**: a 32 MiB chunk takes the prepare/part/finish path that exists
  for objects above 20 MiB; all chunks live in the one `_objects/` folder, so
  directory resolution is unchanged.
- **Mallard Cloud**: the desktop store checks each object's digest against
  its key (`managed_cloud/store.rs:1286`) and sends chunks as ordinary
  objects, far under the Worker's 512 MiB ceiling and under any Cloudflare
  plan's request-body limit. The Worker's validator learns the entry shape
  so its index protects and counts chunk objects (section 3.4). Without
  that, the manifest write is refused.
- **Storage clear** lists and deletes `_objects/` entries by revision; chunk
  objects are entries like any other.

## 9. What stays out, and why

- Conversation files are read whole into memory for parsing on both sides
  (`jsonl_tail_io.rs:338`, `conversation_codec.rs:23`, the session detail
  views). Raising their 100 MiB limit is a parser project, not a transport
  one; the chunked object model would carry them unchanged once it is done.
- Provider-home files are scrubbed in memory at capture; 16 MiB stays.
- The Library engine calls `put_many` and `stage_verified` directly
  (`src-tauri/src/storage_catalog/resource_engine.rs:504`, `:345`) with files
  of at most 8 MiB. It adopts `ObjectSource::Bytes` mechanically in Layer 2
  and nothing else.

## 10. Steps

Each step leaves the product working; the tests named run green at its end.

**Prerequisite, in `mallardInternal`.** The Mallard Cloud Worker accepts
the split entry, indexes chunk objects, serves a split file to the website,
and the website renders it (section 3.4). It is deployed before the desktop
release of Layer 1 reaches users.

### Layer 1: chunked objects

1. **Domain.** Add `PAYLOAD_CHUNK_BYTES`, `BundleChunk`, the two-location
   `BundleFileEntry`, `validate`, `payload_objects()`, and `chunk_ranges()`
   in `crates/mallard-domain/src/lib.rs`. Raise
   `MAX_PROJECT_CONTENT_FILE_BYTES` to 512 MiB. Unit tests: a valid split
   entry; both locations; neither; a chunk list on a small file; a wrong
   chunk size, order, sum, or count; too many chunks.
2. **Publish.** Split in `publish_with_progress`; key pending uploads by
   chunk digest; generalize the retained-identity tuple. Extend
   `publish_dedupes_identical_bytes_within_one_push` and
   `publish_skips_payloads_already_in_previous_manifest` with a split file;
   add a test that a retained split entry whose chunk list differs from the
   reviewed manifest is refused.
3. **Pull.** `payload_objects()` in `prefetch`; `VerifiedBody::assemble`;
   outcome mapping; cache rules. Tests over `LocalObjectStore` and
   `PlainObjectStore`: round trip of a file one byte above the chunk size;
   a missing chunk reports `VERSION_INCOMPLETE`; a chunk with other bytes
   reports a mismatch naming that chunk key; an assembled file is written to
   the target through the existing verified copy.
4. **Stores.** One test per remote store, over its existing loopback server
   or mock (`s3_store.rs` tests, `dropbox_store/mock.rs`,
   `feishu_store/mock.rs`, `managed_cloud/stub.rs`): a two-chunk Push and a
   Pull of the same file, asserting the request shapes the store already
   asserts for single objects. The cloud stand-in also models the Worker
   change: `keep_manifest_content` protects `chunks[].sha256` keys and
   publication references count them, with a test that a split file's
   chunks survive a Discard and a cleanup, and that a manifest with a split
   entry passes the `PUT` validation. The stand-in's `MAX_OBJECT_BYTES` and
   the client's `TooLarge` text move to the Worker's 512 MiB.
5. **Flow.** A `src-tauri/src/sync_tests/` case that pushes a split
   project-content file from one temporary home and pulls it into another
   over local-folder storage, checking bytes, mode, and the receipt.
6. **Docs and strings.** Section 11.

### Layer 2: bounded memory

7. **Sealed capture.** `CapturedPayload`, the streaming hash and secret
   scan, and `CapturedBundleFile.payload`. Tests: the scan finds every marker
   across a window boundary; a non-UTF-8 file is not scanned, as today; a
   sealed file that changes size, mtime, or inode before upload fails the
   Push with a clear reason.
8. **Lazy sources.** `ObjectSource` in `storage_core/object_store.rs`; each
   store loads inside its worker; `PlainObjectStore` gains a counter for
   bodies resident at once, and a test asserts it never exceeds the window.
   The Library passes `ObjectSource::Bytes`.
9. **Raise the numbers** chosen in section 11, update the tests that pin
   them (`app_settings.rs:381`, `provider_capture.rs:2348`), the meter copy,
   and the docs again.

## 11. Decisions to take, and documentation to touch

Decided on 2026-10-25: chunk size **32 MiB**; project-content file
**2 GiB** after Layer 2, with the aggregate at **2 GiB** so one such file
fits a push.

Still open, with the recommendation first:

1. Whether the staged-object cache rises with the per-file limit (default:
   no; a large file is re-downloaded at apply).

Documentation that states the current numbers or the one-object model:

- `docs/FEATURES.md:43` (per-file and aggregate), `:159` (conversation);
- `TECHNICAL_REPORT.md:653-656`, `:670`;
- `docs/CURRENT_METADATA_LAYOUT.md` 5.8, the file-entry layout near line
  746, and the size contracts in section 8;
- `docs/STORAGE_SETUP.md:135-146` (provider upload limits no longer bound a
  file);
- `docs/DESIGN_STREAMED_OBJECT_DOWNLOADS.md` non-goals (multipart upload
  now has a home);
- `src/i18n/zh-CN/backend-labels.ts:91` and the Feishu storage description
  it translates;
- `docs/CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md:1754` (the entry
  contract, in this repository and in `mallardInternal`);
- in `mallardInternal/cloudflare/managed-cloud`: `STORE_API.md` lines 152,
  166, and 401 plus the new route, and `README.md` near line 198.

## 12. Invariants preserved

- Publication stays immutable and SHA-256 verified; every chunk is a
  content-addressed object written through the unchanged immutable put, and
  the record is still written last after a listing.
- No object is exposed to planning, analysis, caching, audit, or apply before
  its exact size and digest are verified; a split file is verified twice,
  per chunk and as a whole.
- No-follow traversal, bounded reads, atomic target writes, backups, reviewed
  pins, and the apply-time size recheck are untouched.
- Credential, path, and trust data stay out of the manifest; a chunk list
  holds digests and sizes only.

## 13. Implementation notes (Layer 1, 2026-10-25)

What landed differs from sections 3 to 5 in two places, both deliberate:

- **Only a project file is split.** The writer splits a payload only when it
  is a project-content file in the project scope; a conversation transcript,
  sidecar, or provider-state file stays one object whatever its size. Their
  limits (100 MiB and 16 MiB) already fit every provider, and Mallard Cloud
  admits a session view only when `_objects/<transcript sha256>` exists, so a
  split transcript would have no view. The domain rejects a split entry in a
  provider scope ("only a project file may be").
- **A single-object entry may be any size.** Section 3.2 said `chunks` is
  present if and only if `size > 32 MiB`. The reader now requires only that a
  split entry be above the chunk size; an entry stored as one object stays
  valid at any size its kind allows, because every manifest already in a
  storage holds such entries up to 100 MiB and the Worker still admits
  single objects up to 512 MiB. The writer never produces one above 32 MiB
  for a project file. The Worker's validator follows the same rule.

Everything else is as planned: `PAYLOAD_CHUNK_BYTES`, `MAX_PAYLOAD_CHUNKS`
(16 at the 512 MiB limit), `BundleChunk`, `payload_chunk_ranges`,
`BundleFileEntry::payload_objects` and `payload_identity` in the domain;
the split in `publish_with_progress` with chunk slices sharing the capture
allocation and pending uploads keyed by chunk digest; `requests_for` and
`body_from_outcomes` in the engine's byte source with
`VerifiedBody::assemble` in `storage_core::object_store`; the stand-in's
reference walk and quota references; the client's and stand-in's 512 MiB
text. Tests: domain entry and scope rules; assembly; a local round trip
with progress, dedupe, retention, a damaged chunk, and a lost chunk; Dropbox,
Feishu, and Mallard Cloud stand-in round trips; and a two-computer
service-level Push and Pull with a chunk that arrives late.

### 13.1 The project-file limit became a setting (2026-10-25)

At the user's request the per-file limit is no longer a constant. *Largest
project file* in Settings → Sync (`project_file_limit_bytes`, 512 MiB by
default; the control offers 256 MiB, 512 MiB, 1 GiB, and 1.5 GiB, and the
document accepts 1 MiB to 2 GiB) is what discovery and capture apply on this
computer; the domain's `MAX_PROJECT_CONTENT_FILE_BYTES` is now the 2 GiB
ceiling any manifest may carry, and `MAX_PAYLOAD_CHUNKS` is 64. Because a
file can only sync if a push admits it, the Push and Pull budget is now
`capture_budget_bytes()`: 512 MiB, or the setting when larger. The review
meter, the apply rechecks, and the session size rules read that function.

This lands the 2 GiB per-file end state ahead of Layer 2, so its memory
cost is the user's choice: in Layer 1 a Push keeps each selected file in
memory while it transfers, which the setting's help text says. Layer 2
(file-backed capture and lazy object sources) removes that cost and is still
the next step.
