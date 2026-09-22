# Google Drive with non-CAS publication

Research and local verification: 19 September 2026.

**Decision:** Google Drive is a feasible experimental storage adapter. The
current protocol no longer needs a conditional head update. The local proof
passes. Production support still needs evidence for listing visibility,
authorization, large files, and publication outcome semantics.

CAS means compare-and-swap in this report. Payloads remain content-addressed
by SHA-256.

This report replaces the Google Drive CAS conclusion in the
[1 September provider study](MALLARD_STORAGE_PROVIDER_FEASIBILITY_RESEARCH_2026-09-01.md).
It uses the working tree, including the uncommitted non-CAS refactor, at
base commit `b651e55`. That commit alone does not contain the evaluated model.
The PoC is separate from the application. It changes no portable schema,
provider registry, Tauri command, or UI.

**Current contract**

The active source, rather than the earlier provider study, defines the model:

| Source | Requirement relevant to Drive |
| --- | --- |
| [ObjectStore](../crates/mallard-infrastructure/src/storage_core/object_store.rs) | Ordinary sync uses immutable put, bounded get, and prefix list. Record writes have no transport retry. |
| [Publication transport](../crates/mallard-infrastructure/src/storage_core/publication.rs) | List the stream before each record attempt. Read the resulting selection after the attempt. |
| [Publication domain](../crates/mallard-domain/src/publication.rs) | A record name contains revision, token, parent, and time. Select the earliest sibling by time, commit ID, then digest. |
| [Storage marker](../crates/mallard-infrastructure/src/storage_core/marker.rs) | Constant marker bytes use `mallard-append-storage` and layout `1`. |
| [Current design](DESIGN_CAS_FREE_STORAGE_REFACTOR.md) | ProjectSync and Library use separate publication streams. |

The payload, manifest, and publication record remain immutable. Write the
record last. There is no stored `_head.json`. Duplicate copies of identical
bytes need no compare-and-swap operation.

**Storage modes**

| Mode | Use | Initial decision |
| --- | --- | --- |
| Dedicated folder in My Drive | One account on several machines | First live probe target. Save its exact folder ID. |
| Shared folder in My Drive | Different users | Candidate. Prove both users can list and read the entire stream. |
| Folder in a Shared Drive | Organization storage | Candidate. Probe the selected drive and its member roles separately. |
| `appDataFolder` | Private data for one app and account | Optional personal mode. It cannot serve as shared team storage. |

Google prohibits sharing and trash operations inside `appDataFolder`. Its
files are hidden from the Drive UI. Deletion and recovery procedures would
therefore differ from visible-folder storage.
[Google application data guide](https://developers.google.com/workspace/drive/api/guides/appdata)

For visible folders, start with `drive.file` and an app-created or explicitly
selected folder. This is a non-sensitive, per-file scope. Do not assume that
selecting a folder proves access to all existing descendants from another
user or app. Test that case. Do not silently request full-Drive access if the
test fails.
[Google scope guide](https://developers.google.com/workspace/drive/api/guides/api-specific-auth)

Use the system browser, PKCE, and a desktop loopback callback for product
OAuth. Store refresh tokens in the credential store. The PoC accepts two
short-lived tokens; it does not implement sign-in, refresh, or reconnect.
[Google desktop OAuth guide](https://developers.google.com/identity/protocols/oauth2/native-app)

**Logical keys without a mutable index**

Drive names are not unique within a folder. A name lookup must not select
the first returned file. An output-only file `version` is also not a write
precondition.
[Google File resource](https://developers.google.com/workspace/drive/api/reference/rest/v3/files)

The PoC uses one fixed folder ID and a flat set of binary files. It stores
each full Mallard key in the file description. Public custom properties
contain hashes for exact-key and prefix searches:

```text
Destination: one exact Drive folder ID

Logical key:
  .mallard/v1/repositories/_commits/<bundle>/<record name>

Drive metadata:
  name         = <record name>
  description  = mallard-object-key:<full logical key>
  properties:
    mallard = append-v1
    key     = SHA256(<full logical key>)
    p3      = SHA256(.mallard/v1/repositories)
    p4      = SHA256(.mallard/v1/repositories/_commits)
    p5      = SHA256(.mallard/v1/repositories/_commits/<bundle>)

Drive content:
  exact object bytes
```

Each query includes the root folder ID and excludes trashed files. Returned
metadata must match the full key and its hashes. Hashes are search hints,
not proof of identity or integrity. Public properties remain subject to
the file's access permissions. They contain no credentials or local paths.

A file has at most 30 public properties. Each property key and value together
must fit in 124 UTF-8 bytes. The PoC uses short property names and 64-byte
hex digests. It keeps the full key outside these properties. The longest
current publication basename is 223 bytes.
[Google custom property limits](https://developers.google.com/workspace/drive/api/guides/properties)

The adapter resolves one key to a bounded set of Drive IDs:

- No copy: return missing.
- Identical copies: return the verified bytes once.
- Different bytes: return an integrity error.
- Too many copies: stop with an explicit limit error.

Prefix listings deduplicate logical keys. Record reads verify the digest in
the selected record name. Payload reads verify the expected size and SHA-256.
A corrupt selected record must not cause selection of another branch.

This design avoids a global key-to-ID index and concurrent creation of
nested folder trees. Each device must use the same saved destination ID.
Two folders with the same display name are two different storages.

This is a provider representation. A desktop Drive sync folder containing
these flat files is not a LocalObjectStore with the same physical layout.

**API mapping and bounds**

| Mallard operation | PoC operation | Important bound |
| --- | --- | --- |
| Immutable put | Resolve copies, allocate a file ID, then multipart `files.create` | 4 MiB per object; no overwrite |
| Record put | One prepared multipart `files.create` request | 64 KiB per record; no transport retry |
| Get | Resolve copies, then `files.get` with `alt=media` | Verify all copies; at most 32 copies |
| Prefix list | Query a prefix hash, follow all pages, validate and sort keys | 1,000 pages and 100,000 listed entries |
| Clear or delete | Not implemented | Do not advertise conditional deletion |

Google supports pre-generated file IDs for binary uploads. A repeat create
with an already used ID returns a conflict instead of another file. This
helps recover one upload attempt; it does not make names unique across
writers. Multipart uploads suit small files. Product support needs resumable
uploads for Mallard's larger payloads.
[Google upload guide](https://developers.google.com/workspace/drive/api/guides/manage-uploads)

The PoC allocates a record's ID before its pre-write listing. A Rust adapter
must keep `put_record` to one upload attempt. It can use a server-generated
ID for that POST, because identical logical copies are valid. ID allocation
must not add an unbounded delay between the engine's listing and its write.

Use `corpora=user` for My Drive. A Shared Drive query uses `corpora=drive`,
the exact `driveId`, `includeItemsFromAllDrives=true`, and
`supportsAllDrives=true`. Avoid an account-wide `allDrives` query for a
storage operation.
[Google Shared Drive guide](https://developers.google.com/workspace/drive/api/guides/enable-shareddrives)

Follow continuation tokens, including after an empty page. Treat
`incompleteSearch=true`, a repeated token, a rejected token, or an exhausted
limit as an error. Restart a rejected traversal from its first page on a
new operation. Never return its partial keys as a complete stream.
[Google files.list reference](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list)

Approximate request counts for one-page queries and one copy per key:

| Action | Requests in this PoC |
| --- | ---: |
| Read one known logical key | 2: query and download |
| Upload one new payload | 3: query, allocate ID, upload |
| Publish one record, without faults | 4: allocate ID, list, upload, list |
| Refresh one stream | 1 per page |

The flat prefix properties avoid scanning payloads during record discovery.
Large histories still need many metadata pages. Measure the cost with real
histories. Google's current quota model charges list operations more quota
units than simple metadata reads. Check the project's actual limits before
setting concurrency and refresh intervals.
[Google usage limits](https://developers.google.com/workspace/drive/api/guides/limits)

**Two limits that the PoC demonstrates**

1. **A new client cannot repair a record it has never seen.** The storage
   trait says completed writes are visible to later prefix listings. The
   reviewed Drive documentation does not establish an equivalent
   cross-client visibility guarantee for these searches. It also states
   that additions and removals can change results during pagination. This
   is a documentation gap, not evidence that all Drive listings are stale.
   [Google files.list reference](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list)

   In the fault test, A publishes a root record. B's query omits it. B sees
   an empty stream. This PoC also resolves reads through a query, so a read
   by logical key cannot repair that omission. A production cache of known
   Drive IDs can enable direct reads for previously seen files. It cannot
   reveal an unknown sibling. Two equal listings or a fixed wait are not
   proof of a complete view.

2. **Selection is deterministic for a set, but success is not final.** This
   is a property of the current publication rules. It is not specific to
   Drive and does not need a stale listing. The fault test runs this order:

   ```text
   A: list parent P; prepare sibling A with rank 200; pause
   B: list parent P; write sibling B with rank 300
   B: list {P, B}; report success
   A: resume; write sibling A
   A: list {P, A, B}; select A; report success
   B: refresh {P, A, B}; selection changes from B to A
   ```

   All steps can occur within the current ten-second freshness window.
   The native Rust selector agrees with the model's selections. The PoC
   models the write loop; it does not call the Rust `write_record` function.
   A finite freshness window does not prevent an earlier permitted request
   from completing later. Before release, define the user-visible outcome
   for this case and verify continuity handling in the actual engines.
   If permanent acceptance is required, the present selection rule alone
   does not establish it.

**Evidence**

The [runnable PoC and live procedure](../poc/google-drive-non-cas/README.md)
contain the exact commands. All tests use synthetic data.

| Check | Result on 19 September 2026 |
| --- | --- |
| Google Drive PoC suite | 28 passed |
| Existing discovery suite, including the reused HTTP helper | 35 passed |
| Native production selector tests through the isolated harness | 7 passed |
| Generated histories compared with production `Mainline` | 128 matched |
| Mock command | Passed; 41 simulated requests; zero conditional writes |
| Live Google account or Shared Drive | Not run; no test tokens were supplied |
| OAuth, two physical machines, full bundle or Library apply | Not run |
| Native Tauri or full application test suites | Not run; no product code changed |

The mock handles Drive URL parameters, JSON responses, multipart bodies,
duplicate names, page tokens, and injected faults. It does not establish
Google service behavior. The native harness includes the production
`publication.rs` source and supplies only its SHA-256 string validator.
The JavaScript write model has no persistent publication cache. Record
bodies are synthetic, not application commit DTOs.

**Remaining work before product support**

1. Run the live probe with two tokens for the same app and account, then
   repeat from two physical machines. Record cross-client visibility for
   key and prefix queries after each acknowledged write.
2. Repeat with two users in one shared folder and then a Shared Drive.
   Check existing content, new content, revoked access, and restricted
   descendants. Per-user permission filtering must not look like an empty
   or complete stream.
3. Resolve the delayed-sibling outcome with the current protocol owner.
   Run the same interleaving against actual ProjectSync and Library flows.
4. Implement resumable payload uploads, bounded staged reads, credential
   health, cancellation, and bounded backoff. A record retry must always
   return to a fresh publication listing.
5. Implement a Rust provider behind the existing registry. Keep folder ID,
   optional Shared Drive ID, and authorization reference machine-local.
   Preserve the existing marker and portable schema `1`.
6. Keep storage clear unavailable unless an exact revision-checked deletion
   contract is proved. A metadata check followed by unconditional delete
   is not sufficient. The reviewed delete endpoint does not supply that
   proof. [Google delete reference](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/delete)

The next useful step is the live two-client probe. Adding Google Drive to
the product settings before those results would overstate this evidence.
