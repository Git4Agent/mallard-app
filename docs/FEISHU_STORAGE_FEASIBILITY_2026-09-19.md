# Feishu Drive storage under Mallard's current publication model

Research date: 2026-09-19. Scope: Feishu China Drive file storage.

**Decision: proceed with a limited live pilot. Do not enable a production
provider yet.** The current model removes the conditional-write obstacle.
Feishu has the required transfer and listing APIs. Listing completeness,
folder identity, capacity, and desktop authorization remain open gates.

The [PoC](../poc/feishu-drive/README.md) is implemented. The initial run
passed all 31 local tests and seven Rust publication tests. The
[2026-09-20 live follow-up](FEISHU_LIVE_POC_2026-09-20.md) adds a verified
desktop round trip and developer-app setup. It also reran all 31 local
tests and the current nine Rust publication tests. Keep desktop evidence
separate from Open API evidence.

## Current code contract

Here, CAS means compare-and-swap. SHA-256 content addressing remains in use.
This assessment uses the current working files, including the in-progress
storage refactor. It does not treat the older CAS-based provider reports
as the current contract.

The checkout's base commit was `b651e55ae39a03bf8170902cbf02b5838c1ed982`.
The non-CAS source changes were present in the working tree. The Rust
bridge builds from those files, not from the base commit alone.

| Current requirement | Source |
| --- | --- |
| Ordinary sync needs immutable put, bounded get, and complete prefix listing | [`ObjectStore`](../crates/mallard-infrastructure/src/storage_core/object_store.rs) |
| Record names carry revision, commit ID, record digest, parent token, and creation time | [`RecordName`](../crates/mallard-domain/src/publication.rs) |
| The smallest sibling rank wins; a longer losing branch cannot win | [`Mainline::select`](../crates/mallard-domain/src/publication.rs) |
| A record is sent after a fresh stream listing; transport retries are forbidden | [`write_record`](../crates/mallard-infrastructure/src/storage_core/publication.rs) |
| Payloads and manifests precede the publication record | [`bundle_engine`](../crates/mallard-infrastructure/src/bundle_engine.rs) |
| Linked observations roll back local acceptance for a confirmed Push that is later outranked | [`observe_linked_bundle`](../src-tauri/src/project_sync/service.rs) |
| Conditional deletion is an optional, separate capability | [`ObjectStore`](../crates/mallard-infrastructure/src/storage_core/object_store.rs) |

The schemas stay at `1`. The marker is
`{"format":"mallard-append-storage","layout_version":1}`. The adapter
can retain the logical keys and return file bytes without putting Feishu
tokens into portable manifests.

```text
Approved Push
  -> upload payloads
  -> upload manifest
  -> list the complete record stream
  -> upload one immutable record
  -> list and select with Mallard's domain code
```

Current selection uses the smallest
`(created_at_ms, commit_id, record_sha256)` at each parent. Other records
remain stored. They are not merged or deleted.

## Feishu API fit

The official pages were read on the research date. These are documented
capabilities, not results from a live account.

| Mallard need | Feishu operation | Adapter work |
| --- | --- | --- |
| Resolve a directory | `GET /drive/v1/files?folder_token=...` | Match names to folder tokens; reject ambiguous paths |
| Create a directory | `POST /drive/v1/files/create_folder` | Prepare folders before publication admission |
| Write a small object | `POST /drive/v1/files/upload_all` | Send multipart form data and retain the returned file token |
| Write a large object | `upload_prepare`, `upload_part`, `upload_finish` | Follow the returned part plan and verify final bytes |
| Read an object | `GET /drive/v1/files/:file_token/download` | Bound the stream and check size plus SHA-256 |
| List a logical prefix | Folder listing plus explicit traversal | Exhaust every page and required child folder |

The [listing API][list] returns direct children, supports pages of at most
200 entries, and documents a rate of 20 calls per second. It has no
recursive-prefix operation or lexical-name sort. Omitting the folder
token changes the root listing behavior. The adapter must always supply
the configured token.

The [small-file upload API][upload] accepts at most 20 MiB and a name of
at most 250 characters. Mallard's longest record name is 223 ASCII
characters, so it fits without renaming. The [multipart APIs][multipart]
use a prepare/part/finish sequence. The prepare result supplies block size
and count; the documented block size is 4 MiB, with a 24-hour upload
session. Account file-size limits still apply. Mallard permits conversation
files up to 100 MiB, so production support needs multipart upload.

The [download API][download] returns original file bytes and supports
Range requests. It does not export online Docs, Sheets, or Base content.
Mallard needs ordinary binary files. The PoC requests complete files and
rejects partial responses.

## Conditions that remain unresolved

**1. Listing completeness is a correctness condition.** The inspected
references do not promise that a completed upload is immediately visible
to every later listing, or that a paginated traversal is a snapshot.
`ObjectStore::list` requires completed-write visibility. Repeating a
listing, finding one's own file, or observing two equal results does not
establish that condition for an unknown competing writer. [Listing API][list]

The local test reproduces this failure:

```text
Root R is visible.
A uploads child A1, rank 101. The listing hides A1.
B lists R, uploads child B1, rank 102, and observes B1 selected.
A1 later becomes visible. The same Rust selector now selects A1.
```

This is a synthetic counterexample. It is not evidence that Feishu has
this behavior. Mallard's cache can retain records it has seen. Its write
helper can also retain its own acknowledged record. Neither mechanism
finds an unseen record from another writer. A live pilot must test cold
clients, both token contexts, pagination, and concurrent activity. A
provider guarantee or explicit acceptance of weaker semantics is still
needed before release. No such weakening is proposed by this PoC.

The current application also rolls back local acceptance when a later
observation finds the confirmed Push outranked. This is part of the
current model. It does not establish a provider's listing guarantee.

**2. File names are not file IDs.** A logical key must resolve through
folder listings to a file token. The inspected upload contract does not
establish atomic create-if-name-absent or same-name behavior. The PoC
therefore tests duplicate files explicitly. It checks every matching
copy against the expected bytes. A renamed copy fails visibility checks.
Two matching folders cause `ambiguous_folder`; taking the first would
risk dividing a stream between two directories. [Upload API][upload]

Directory creation needs one controlled setup step and cross-client
verification. A production adapter also needs a defined response when
two machines create the same logical directory. This issue remains even
when publication itself has no mutable head.

**3. A direct folder layout has a low capacity ceiling.** Feishu documents
1,500 direct child nodes per folder, 400,000 nodes across the space, and
at most 15 directory levels. Existing parent folders consume depth.
Payloads, manifests, records, folders, and duplicate copies consume nodes.
[Folder creation][folder], [file upload overview][multipart],
[multipart completion][finish]

Current Mallard streams can list up to 100,000 records. One Feishu folder
cannot hold that history. Production needs a defined physical sharding
scheme behind the existing logical keys, or an explicit lower provider
limit in the product. A SHA-256 prefix is a possible object shard, but a
prefix alone does not guarantee a per-folder maximum. Record sharding
also increases refresh cost. Do not change the portable schema to solve
this adapter concern.

**4. Request limits affect practical use.** Upload and download references
each state that calls must not be concurrent, with limits of 5 calls per
second and 10,000 calls per day. Folder creation has similar limits.
The enforcement scope across devices needs confirmation. A local queue
cannot coordinate every machine using one app. [Upload API][upload],
[download API][download], [folder creation][folder]

At 5 calls per second, 1,000 one-request uploads take at least 200 seconds,
before reads, listings, multipart overhead, or latency. This is a lower
bound derived from the documented limit, not a benchmark. Many small
files are likely to be the main cost. The PoC uses 249 mock requests for
its full sequence because it repeats path resolution and uses tiny pages.
That count describes the probe, not a production Push.

**5. Desktop authorization needs a product decision.** The current
[OAuth v3 reference][oauth], updated 2026-08-21, requires a client secret
for developer-created apps. Public-client registration is restricted to
Feishu's official MCP apps. PKCE does not remove the client-secret
requirement for a normal Mallard app. A distributed client needs a
trusted server for token exchange, or a user-owned app whose secret is
kept locally. The probe accepts short-lived access tokens. A separate
developer harness now implements a memory-only OAuth v3 exchange with
PKCE for a user-owned test app. It does not request refresh tokens. See
the [live follow-up](FEISHU_LIVE_POC_2026-09-20.md) for its test status and
the API Explorer authentication error.

Application and user tokens have different access contexts. API scopes
alone do not grant access to a chosen folder. Feishu documents a bot and
group sharing procedure for app access, and separate download policy
checks. [Drive access FAQ][faq]

**6. Storage clear is not established.** The [delete API][delete] names a
file token and type. The inspected request contract exposes no expected
revision precondition. A file token is an object identifier, not proof
of an unchanged revision. Keep `supports_conditional_delete()` false.
Do not add recursive folder deletion as a substitute. The PoC has no
delete call and retains its test folder.

## PoC scope and evidence

The implementation is isolated under `poc/feishu-drive`. One new Rust
example calls the current domain code. No production provider, DTO,
storage setting, or schema was changed by this task.

| Evidence | Result | Limit |
| --- | --- | --- |
| Node transport and fault suite | 31 passed, 0 failed | In-memory HTTP fixture |
| Existing Rust publication unit tests | 9 passed, 0 failed on 2026-09-20 | Domain behavior only |
| CLI mock sequence | `PROBE_PASS`, 249 requests | No Feishu request |
| Small-file and multipart wire formats | Passed local serialization tests | Server acceptance not checked |
| Multipart threshold | Tested at 20 MiB + 17 bytes | Account maximum not checked |
| Publication selection | Uses production `RecordName` and `Mainline` | Does not run the full bundle engine |
| Delayed listing counterexample | Reproduced locally | Does not characterize Feishu |
| Feishu desktop upload and download | Eight files passed exact path, size, and SHA-256 checks on 2026-09-20 | One account and machine; [follow-up](FEISHU_LIVE_POC_2026-09-20.md) |
| Live API, OAuth, and two-machine tests | Not established by the desktop test | See the follow-up for current setup state |

The record and manifest bodies are synthetic transport fixtures. The
PoC is not a complete `ObjectStore` implementation. Its publication
helper sends one record attempt and then lists. It deliberately fails
an acknowledged-but-unlisted record to detect a transport contract gap.
It does not implement the production helper's retry and cache behavior.

Fault coverage includes failed later pages, repeated cursors, HTTP 200
API errors, access denial, quotas, redirects, body limits, wrong hashes,
partial downloads, multipart failures, duplicate files and folders,
lost acknowledgements, stale parents, stale admission, and retained
losing records. Credential tests use temporary fixture files only.

## Path to production

| Step | Required result |
| --- | --- |
| Run the supplied probe in a dedicated folder | Exact names, marker, multipart, both token contexts, and repeated visibility pass |
| Run separate client processes on two machines | Confirm namespace identity, paginated visibility, late responses, and rate contention |
| Resolve listing semantics | Establish completed-write visibility and acceptable pagination behavior |
| Define physical layout | Bound folder counts, depth, duplicate resolution, and record enumeration cost |
| Select authorization mode | Hosted token exchange or a documented user-owned app flow |
| Add a Rust adapter | Reuse `storage_core`, verified staging, and the current publication module |
| Add product setup | Separate destination coordinates from local credential storage; fixed Feishu API origin |
| Run engine and UI checks | Project sync and Library parity, approval gates, failure recovery, and native setup evidence |

A future adapter can map both project and Library keys. This PoC exercises
one project record stream only. Library behavior, discovery at scale,
credential refresh, revocation, and production performance remain outside
its proof. International Lark is also outside this assessment.

## Official references

All links were checked on 2026-09-19. The rendered reference pages were
used because the text-only fetch returned empty pages for several routes.

| Reference | Page update shown during research |
| --- | --- |
| [List items in folder][list] | 2025-08-29 |
| [Create folder][folder] | 2025-05-06 |
| [Upload a file][upload] | 2024-10-23 |
| [File upload overview][multipart] | 2024-09-05 |
| [Prepare multipart upload][prepare] | 2025-01-02 |
| [Upload a part][part] | 2025-01-02 |
| [Complete multipart upload][finish] | 2025-01-02 |
| [Download a file][download] | 2025-04-16 |
| [Drive access FAQ][faq] | 2026-08-24 |
| [Get user access token, v3][oauth] | 2026-08-21 |
| [Delete a file or folder][delete] | 2024-09-20 |

[list]: https://open.feishu.cn/document/server-docs/docs/drive-v1/folder/list
[folder]: https://open.feishu.cn/document/server-docs/docs/drive-v1/folder/create_folder
[upload]: https://open.feishu.cn/document/server-docs/docs/drive-v1/upload/upload_all
[multipart]: https://open.feishu.cn/document/server-docs/docs/drive-v1/upload/multipart-upload-file-/introduction
[prepare]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/drive-v1/file/upload_prepare
[part]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/drive-v1/file/upload_part
[finish]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/drive-v1/file/upload_finish
[download]: https://open.feishu.cn/document/server-docs/docs/drive-v1/download/download
[faq]: https://open.feishu.cn/document/server-docs/docs/drive-v1/faq
[oauth]: https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/authentication-management/access-token/get-user-access-token-v3
[delete]: https://open.feishu.cn/document/server-docs/docs/drive-v1/file/delete
