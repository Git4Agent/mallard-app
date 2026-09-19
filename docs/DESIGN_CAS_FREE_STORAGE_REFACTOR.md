# Mallard: CAS-free storage publication — implementation plan

- Status: Planned; not implemented
- Last reviewed: September 19, 2026
- Scope: the desktop app — both sync engines, the Local, S3/R2 and Dropbox stores, and the Push outcome copy. Mallard Cloud follows in its own plan (section 14)
- Compatibility: clean cutover; no migration, no legacy detection, no legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner clears `~/.mallard` and the test storages before acceptance testing
- Schemas: every schema and layout number stays `1`
- Related: `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md` (how each decision was reached), `DESIGN_REMOTE_HISTORY_CONTINUITY.md` (its rows 1 to 8 are replaced by section 9)

Code locations are collected in *Source pointers* at the end, not in the body.

## 1. Goal

Today a Push ends with a compare-and-swap on a mutable `_head.json`. A provider
that cannot do a conditional write cannot be a Mallard storage; the provider
research names Google Drive, Aliyun OSS, Huawei OBS and Tencent COS as blocked
for that reason.

After this change a provider needs three operations for ordinary sync: **write
bytes at a key, read a key, list a prefix.** There is no mutable head. The
current snapshot of a bundle is derived, by shared code, from immutable records
that every Push already writes.

Decisions, all final:

1. **The commit record is the publication point.** The `_commits/` record that
   every publish writes today, and nothing reads, is written last and becomes
   the only thing that publishes a snapshot. `_head.json` and the `_tags/`
   index are deleted.
2. **Earliest sibling wins.** When two records name the same parent, the one
   with the smallest `(created_at_ms, commit_id, record_sha256)` is selected.
   The other is kept and not selected.
3. **List, then write.** A record is written only right after a listing that
   shows its parent selected and childless. This is the limit on late-written
   records (section 7.2).
4. **Refresh is one listing of one stream.** A record's name carries its graph
   facts, so the head is selected without reading records. An unchanged
   selection reads nothing else.
5. **Identity is the commit id.** Continuity, the Push gate and forks are decided
   by identity and ancestry, never by revision order. Local pins, IPC types and
   TypeScript types keep their shape.
6. **Record names are `<commit_id>-<sha256>`.** The same id with different bytes
   is two publications, not a conflict. No state blocks a stream.
7. **No storage UID.** A storage is its provider destination, as today.
8. **Losing publications are kept. There is no archive UI in this release.**
9. **No legacy handling of any kind.**
10. **Desktop first.** Mallard Cloud is not offered by these builds.

## 2. What stays exactly as it is

| Area | Unchanged |
|---|---|
| Transport | The `ObjectStore` trait apart from the three changes of section 10 (`compare_and_swap` removed, `put_record` and `list_keys` added), `ObjectKey` and `ObjectPrefix` validation, the blocking execution model, verified staged reads, the shared-store wrapper and connection health |
| Remote objects | `_objects/<sha256>` and `_manifests/<revision>-<commit_id>.json`, for bundles and for the Library |
| Portable schemas | `BundleManifest` and `LibraryResourceManifest`, including `revision`, `commit_id` and each file entry's `object_key` |
| Local state | `RepositoryBase`, `ScopeBase`, `MaterializationRecord`, `ConversationApplyReceipt`, `AcceptedConversationVersion`, `LibraryHeadCoordinate`, and every file under `~/.mallard` |
| IPC and TypeScript | `BundleSnapshot`, `RestorePlan`, `PullReview`, `ConversationRevisionPins`, `PushDestinationStatus`, `RemoteContinuity`, `ScopeContinuity`, and their mirrors |
| Engine logic | Manifest construction, the scope mutation firewall, the apply-time rebase, restore planning and apply, payload reads, the object cache, deduplication against the parent manifest |
| Storage identity | The adapter's destination coordinates, the rule that one destination is one storage, discovery by one fixed-key marker read, settings locked for life |
| Storage clear | In place: it keeps the marker and resets local link state |

`BundleHead` and `LibraryResourceHead` stay as types. They stop being stored
objects and become the in-memory view of the selected record, field for field,
so `BundleSnapshot { head, manifest }` and every reader of `snapshot.head.*`
keep working.

## 3. What is removed

```text
ObjectStore::compare_and_swap, CasExpectation, CasOutcome
HeadToken.etag, ObservedLibraryHead.etag
_head.json as a stored object, for bundles and for the Library
the _tags/ namespace, BundleTag as a wire record, publish_tag and its retry loop
the continuity probe (fetch_manifest_version_optional) and its closure in the service
every ordering comparison on a remote revision or scope_revision
the R2 probe stages ConditionalOverwrite and StaleConditionalRejection
the Dropbox probe's update-by-revision and stale-revision checks
CAS-conflict error strings, the fakes that emulate If-Match, and their tests
```

Nothing replaces them with a compatibility path. Local revision protections and
unrelated HTTP metadata handling stay, even where they also use the words
revision or ETag.

## 4. Storage layout

### 4.1 Marker

`.mallard/_storage.json` keeps its key and its shape:

```json
{ "format": "mallard-append-storage", "layout_version": 1 }
```

Only the `format` constant changes. The bytes are constant, so concurrent first
writers write identical bytes and creation stays idempotent without any
exclusivity. Discovery and `classify_storage_marker` use the new constant.
Nothing else in the app knows that an older format existed.

### 4.2 Objects

```text
.mallard/
  _storage.json
  v1/
    repositories/
      _commits/                              every project record, and nothing else
        <bundle_id>/
          <record name>
      <bundle_id>/
        _manifests/<revision>-<commit_id>.json
        _objects/<sha256>
    library/
      <repository_id>/
        _tag.json
        _commits/<record name>
        _manifests/<revision>-<commit_id>.json
        _objects/<sha256>
```

Project records live in one flat namespace beside the bundle folders, in the
position `_tags/` holds today. Listing `_commits/<bundle_id>/` returns one
stream, which is what a refresh needs. Listing `_commits/` returns every
project stream's records and no payload key, which is what bundle discovery
needs. A bundle id is 32 hexadecimal characters, so it cannot collide with
`_commits`. Library records stay inside each repository, because the Library
never enumerates repositories: their ids are derived from the registered
coordinates.

A **stream** is one bundle or one Library repository. Streams never compete
with one another.

### 4.3 Record names

```text
<revision>.<commit_id>-<record_sha256>.<parent>.<created_at_ms>.json

revision         10 digits, zero-padded
commit_id        commit- plus 24 lowercase hexadecimal characters
record_sha256    SHA-256 of the record's exact bytes, 64 hexadecimal characters
parent           <parent commit_id>-<parent record_sha256>, or the word root
created_at_ms    13 digits, zero-padded
```

- `<commit_id>-<record_sha256>` is a publication's **token**: its full identity
  in the graph. In normal operation the `commit_id` alone is unique; the token
  makes two records that share an id two separate publications.
- Shared code builds and parses names, as it builds every key today. Selecting
  a head reads names, never records.
- A name is self-certifying: when a record is read, its bytes must hash to the
  digest in its name, and its fields must equal the facts in its name.
- Names sort by depth, so a store can later list only the tail with the cursor
  it already has.
- A name is lowercase `[a-z0-9.-]` and at most 223 bytes, within every current
  provider's 255-byte component limit.
- A name outside the grammar (`.DS_Store`, a sync client's conflicted copy) is
  ignored and written to the activity log. It is never a record.

## 5. The publication record

The record is today's commit record with four changes: `manifest_size` and
`previous_record_sha256` are added, `created_at_ms` replaces `created_at`, and
`summary` carries the facts the tag carries today.

```json
{
  "schema_version": 1,
  "bundle_id": "0123456789abcdef0123456789abcdef",
  "revision": 2,
  "commit_id": "commit-aaaaaaaaaaaaaaaaaaaaaaaa",
  "previous_commit_id": "commit-bbbbbbbbbbbbbbbbbbbbbbbb",
  "previous_record_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "created_at_ms": 1789635601000,
  "manifest_key": "_manifests/2-commit-aaaaaaaaaaaaaaaaaaaaaaaa.json",
  "manifest_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "manifest_size": 118800,
  "added_files": 1,
  "changed_files": 0,
  "removed_files": 0,
  "published_by_origin": { "…": "unchanged" },
  "summary": {
    "display_name": "example",
    "kind": "project",
    "resources": 12,
    "files": 39,
    "registration_only": false,
    "providers": ["codex"],
    "repository_fingerprint": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
  }
}
```

Rules:

- `previous_commit_id` and `previous_record_sha256` are both absent for a first
  publication and both present otherwise. Together they are the parent's token.
- `revision` is `1` for a first publication and the parent's `revision + 1`
  otherwise, as the engine computes it today. It is the depth, and the number
  the UI prints ("Version 2"). Two siblings share a revision, so it is never an
  identity and never an order between branches.
- `created_at_ms` is UTC Unix milliseconds in `0..2^53-1`. It is taken when the
  record is built. It ranks siblings and proves nothing about arrival order. No
  reader rejects a record by comparing it with its own clock.
- `summary` makes bundle discovery read records and never manifests. It is as
  fresh as its publication; today's tag can lag, because a failed tag write is
  ignored.
- The record does not contain its own digest, and it has no `is_current`,
  `archived`, `winner`, or `head_etag` field. Those are derived.
- A record is at most 64 KiB, the bound the tag has today.
- Invalid: fields that disagree with the name, a `revision` that is not the
  parent's plus one, a parent in another stream, a cycle, duplicate fields,
  unbounded structures.

The Library's commit record gets the same three protocol fields and the same
name grammar. It needs no `summary`.

The record is built once per operation and held in memory. It is never written
to local disk and never outlives the operation.

## 6. Selecting the head

Input: the names from one listing of a stream. Output: the mainline.

```text
rank(P) = (P.created_at_ms, P.commit_id, P.record_sha256)

current  = the virtual root
mainline = []
while current has children among the listed names:
    current = the child with the smallest rank
    mainline.append(current)

selected = last(mainline), or none
```

- The virtual root is implicit. A first publication names `root` as its parent.
  Two first publications are ordinary siblings.
- Parent edges are tokens, so every edge is exact.
- A record whose parent is not listed is not connected to the root and is never
  selected. It is not deleted, and it connects if the parent appears later.
- There is no longest-branch rule, merge record, or rewriting of stored
  records. A child with a smaller timestamp than its parent stays its child.
- Every publication that is listed and not on the mainline is **kept, not
  selected**. Nothing marks it, moves it, or deletes it.
- Reading health never changes selection. If the selected record or its
  manifest cannot be read or fails verification, that publication is
  `Unavailable` or an `IntegrityError`; selection does not move to another
  candidate.
- A local pin names a publication by `commit_id`. If a listing ever holds two
  tokens with that id, the pin resolves to the earlier-ranked one.

The same listing gives every device the same mainline. Devices converge as soon
as their listings agree.

## 7. Publishing

### 7.1 Flow

1. **Review** against one publication and its verified manifest, or against the
   empty stream.
2. **Apply starts:** list the stream and select. A project runs the Push gate
   of section 9.2 against the freshly selected publication; the Library requires
   it to be the reviewed publication (section 11). Rerun the existing local
   revision, digest, destination, expiry, codec and approval checks.
3. **Build** the next manifest on the freshly selected publication. A project
   carries every scope it does not write byte for byte, under the existing
   mutation firewall; this is the rebase the engine performs today. The Library
   adds its reviewed upserts to the selected set.
4. **No semantic change:** return `NoChange` with the publication it was
   compared to. Nothing is written.
5. **Put** changed objects at `_objects/<sha256>`, then the manifest at
   `_manifests/<revision>-<commit_id>.json`. A later attempt puts the same keys
   again, which is idempotent, so nothing is remembered between attempts.
6. **Build the record** in memory: a new `commit_id`, the parent token from
   step 2, `revision`, `created_at_ms`, origin, the manifest reference, the
   summary.
7. **List, then write** (section 7.2).
8. **Stored:** advance the local bases to the stored publication, as a Push
   does today, select again, and report whether it is the selected one.

Registering an empty project is the same flow with no parent and the existing
registration manifest. If the stream already has a selected publication, the
registration returns it and writes nothing, as it does today.

### 7.2 List, then write

Earliest-sibling selection has one hazard: a record with an old timestamp that
is written late outranks every sibling created in between. Only a writer that
prepares a record, keeps it, and sends it later can produce one. This plan
removes that writer.

Every attempt to write a record is the second half of one step:

1. List the stream and select. If the listing already holds this record's
   token, an earlier attempt landed: the outcome is `Stored`.
2. Continue only if the selected publication is the record's parent, which
   means the parent has no child. For a first publication, continue only if the
   stream is empty. Otherwise stop with `RemoteChanged`; nothing is written.
3. Send the record, once.

Four details make this hold on a laptop:

- **Every attempt lists first.** A retry is a new attempt, never a resend.
- **The listing is fresh.** If more than ten seconds of wall-clock time passed
  since the listing answered, list again before sending. The monotonic clock is
  not enough: on macOS it stands still while the machine sleeps.
- **The write is one request.** `put_record` never retries inside the
  transport, because a transport retry is an attempt without a listing. A
  request that stalls dies with its connection, which the provider drops within
  about a minute.
- **Nothing is kept.** No journal, no outbox, no resend after a restart. Push
  stays stateless, as it is today.

The limit this gives:

```text
fork window = the store's listing lag + 10 s listing freshness + one record request
```

A sibling of a publication can be written only inside that window after the
publication became visible. The listing lag is zero on the current stores, so
the window is a few seconds in practice and about a minute if a request stalls.
**Outside it, selection is final.** The bound does not depend on any clock being
right, on the age of a record's timestamp, or on how long a machine slept: a
laptop that slept three days lists, sees a child, and stops. A store with a
lagging listing widens the window by its stated lag and changes nothing else.

Inside the window the earlier-ranked sibling wins, and the other is kept and
not selected, together with anything built on it in those seconds.

### 7.3 Outcomes and failures

`PublishedBundle { published, snapshot }` and the Library's equivalent become:

```text
Stored   { snapshot, selected: bool }
NoChange { snapshot }
```

Errors: `RemoteChanged` replaces every CAS-conflict string, and
`OutcomeUnknown` is reported when the storage cannot be reached after the
record was sent.

| Boundary | What happens |
|---|---|
| Before the record is sent | Nothing is published. Objects and the manifest are unreferenced residue |
| The listing before the write shows another child or another selection | `RemoteChanged`. Nothing is written. The user compares again |
| The machine sleeps between the listing and the write | The listing is older than ten seconds, so the attempt lists again |
| The record was sent and no acknowledgment arrived | List. Present is `Stored`. Absent is another attempt, which lists first |
| The storage cannot be reached after sending | `OutcomeUnknown`. Nothing is persisted; the next refresh shows whether it arrived |
| The app quits or crashes | Nothing is resent. The next Push builds a new record |
| Stored, but the local bases were not saved | The next refresh reads `Advanced` with content equal to local; a Pull review with nothing to apply advances the bases |
| Stored, not selected | Bases stay on the stored publication. The next refresh reads `Replaced`: Pull and review, then Push again (section 13) |

Keep local single-flight per stream. It is a local convenience, not the
distributed exclusion. The Library apply path has no local lock today, because
the head CAS stopped a double Apply; it gets one, or a double-click writes two
siblings.

## 8. Refresh and discovery

### 8.1 Refresh

```text
1. list the stream's record prefix               1 request per 1,000 publications
2. select from the names                         in memory
3. selected commit_id == the one already held?   yes: stop, nothing else is read
4. read the selected record                      1 request; verify digest and fields
5. read its manifest                             1 request; then compare with local as today
```

"The one already held" is the publication whose manifest this process has in
its object cache. Steps 4 and 5 produce the same `BundleSnapshot` that
`inspect_optional` returns today.

| Situation | Requests | Today |
|---|---|---|
| Nothing changed | 1 listing | 1 head read |
| Selection changed, or the first refresh after a restart | listing + record + manifest | head + manifest |
| Remote is ahead of this replica's base | no extra read | 1 uncached probe of the base manifest on every refresh |
| Re-reading a pinned base manifest | 1 read by key | 1 read by key |
| Push apply, after the uploads | listing + record write | head CAS, tag read, tag CAS in up to four rounds |
| Library scope page | listing + record + tag + manifest; the record is cached while the selection is unchanged | head + tag + manifest |

Remote reads happen per user action, as today. There is no background polling.
Nothing is persisted for discovery: no index, cursor, or scan state. The
process caches the last listing's names and the records it has read.

Every record listing asks for keys only (`list_keys`, section 10). A record's
name already carries its digest, so a listing never needs an ETag. This matters
on the local store, whose `list` reads and hashes every listed file to fill
one: a quick script on this machine's disk listed 1,000 one-kilobyte records in
1.2 ms by name and in 41.8 ms when it also read and hashed each file. Listing
moves onto every refresh, so that cost must not come with it.

### 8.2 What a listing proves

A failed page, a permission error, or an expired cursor fails the refresh. It is
never an empty stream. A publication this process has already seen that is
missing from a later listing is confirmed with one direct read of its key
before it is dropped: if it reads, it stays, because the listing lagged.

### 8.3 Bundle discovery

List `.mallard/v1/repositories/_commits`. Group the names by bundle id, select
each stream, read those records with the existing bounded-concurrency metadata
fetch, and build the same `RemoteBundleSummary` rows from `summary`. It serves
project setup, the link chooser, the "Only in this storage" rows and
sibling-bundle status, as `list_remote_bundles` does today.

### 8.4 Growth

Refresh cost grows only with the stream being refreshed: one more listing
request per 1,000 publications in it. There is no garbage collection in this
release. When a stream's listing is measured to matter, list the tail by
passing a depth-prefixed cursor; a full listing stays available and is what a
Push uses. Hitting a listing budget reports an incomplete view and blocks a
Push; it never truncates history silently.

## 9. Continuity and gates

No type changes shape. Every comparison on a remote coordinate changes from
order to identity and ancestry.

### 9.1 Repository and scope continuity

| Condition | `RemoteContinuity` |
|---|---|
| no base, and the stream lists empty | `Fresh` |
| a base is recorded, and the stream lists empty | `RemoteMissing` |
| no base, and a publication is selected | `Unreviewed` |
| the base is the selected publication | `InSync` |
| the base is an ancestor of the selected publication | `Advanced` |
| any other base | `Replaced` |
| the listing failed | `Unknown` |

- The ancestor test walks parent tokens from the selected publication down to
  the base's revision and compares `commit_id`. Tokens are hash-linked, so the
  walk needs no record read and no probe.
- When a link holds several scope bases, continuity is `Replaced` if any of
  them is neither the selected publication nor one of its ancestors.
- `Replaced` covers a cleared or recreated stream and a lost race inside the
  fork window. The existing remedy serves both: Pull and review, then Push.

| Scope digest in the selected manifest | `ScopeContinuity` |
|---|---|
| equals the scope base digest | `InSync` |
| differs, and the repository is `Advanced` | `Advanced` |
| differs otherwise | `Replaced` |

`scope_revision` stays in the manifest and in `ScopeBase` as a per-branch
counter for display. Nothing compares it.

### 9.2 Push gate at apply

| Freshly selected publication | Action |
|---|---|
| the one this replica is based on | publish on it |
| a descendant, and every writable scope still has this replica's base digest | publish on the new one; carry the other scopes from it |
| a descendant, and a writable scope changed | `RequiresPull` |
| not a descendant, or the stream is empty while a base is recorded | `Replaced` or `RemoteMissing`; stop |

Ancestry answers whether the remote moved forward. The scope digest answers
whether this Push's own scopes were touched. This is today's gate with ancestry
in place of revision order.

The writable scopes are `project` plus one scope per provider this computer has
bound, whatever the review ticked. A publication replaces a writable scope as a
whole, built from this computer's content and the scope state it last synced.
That is why a remote change inside a writable scope needs a Pull first, and a
remote change in any other scope does not: the other scope is copied from the
new parent with its digest untouched, and the firewall verifies that after the
manifest is built.

This gate is for projects. The Library has no scopes and keeps the stricter rule
it has today (section 11).

### 9.3 Sites that change

| Today | After |
|---|---|
| Repository continuity from revision order plus a store probe of the base manifest | Section 9.1; the probe and its closure are deleted |
| Scope continuity from `<` and `>` on `scope_revision` | Section 9.1 |
| The repository base is `max_by_key(revision)` over the scope bases | Every scope base is checked against the selected mainline |
| `scope_revision.max(…)` when a single-conversation Pull advances a base | Take the scope coordinates of the reviewed publication |
| The Library's "history was rewound" on a lower revision | Ancestry |
| Whole-head equality at Pull apply, the project-files review token, the readiness match | Unchanged: they test `(revision, commit_id, manifest_sha256)` for equality |

Pull apply still requires the selected publication to be the reviewed one. The
local mutation is always of the exact reviewed bytes.

## 10. Store contract and per-store work

| Member | After |
|---|---|
| `get_metadata`, `get_metadata_many`, `stage_verified`, `stage_many_verified` | Unchanged |
| `put_immutable`, `put_many` | Same signatures. The contract no longer promises an atomic no-overwrite. A store may keep its native mechanics; a new provider may implement a plain write |
| `put_record` — new | One request, no transport retry. Used only for the commit record. The default forwards to `put_immutable` |
| `compare_and_swap` and its types | Removed |
| `list` | Same signature. It skips and logs a name it cannot represent instead of failing the listing. A completed write is visible to every later listing of that prefix, or the adapter states its worst-case lag. After the cut only storage clear calls it, because clear deletes exactly the ETag it listed |
| `list_keys` — new | The same listing without ETags. Every record listing uses it: refresh, list-then-write, and bundle discovery. The default forwards to `list` and drops the ETags, which is free on S3/R2 and Dropbox, where the ETag arrives in the listing response. The local store overrides it |
| `delete`, `supports_conditional_delete`, `clear_managed_data` | Unchanged; the clear order changes (section 12) |

A plain write is enough because no conforming writer ever writes different
bytes to the same key:

| Key | Why |
|---|---|
| `_objects/<sha256>` | Content-addressed |
| `_manifests/<revision>-<commit_id>.json` | `commit_id` is 96 random bits, minted per publication |
| a record | Its name contains the digest of its bytes |
| `.mallard/_storage.json` | Constant bytes |
| the Library's `_tag.json` | Drop its `created_at`, so the bytes are constant too |

| Store | Work |
|---|---|
| Local folder | Delete `compare_and_swap`. `put_record` is `put_immutable`. `list_keys` walks the prefix and returns names without opening a listed file; `list` keeps hashing, for clear. Reads, the lock and no-follow access are unchanged |
| S3/R2 | Delete `compare_and_swap`, the `If-Match` branch and the ambiguous-CAS resolution. `put_record` runs with SDK retries off; the client is built with the SDK default of three attempts today. `list` skips and logs a key it cannot parse. The capability probe loses two stages |
| Dropbox | Delete `compare_and_swap` and the update-by-revision write mode. `put_record` is one `files/upload` request, not a session start plus a batch commit with Mallard's code in between. `list` skips and logs a non-canonical or conflicted-copy path. The setup probe loses its update and stale-revision checks |
| Shared wrapper | Its forwarding methods shrink with the trait |
| Mallard Cloud | Unregistered (section 14) |

A setup probe tests a write, a verified read-back, and that a listing taken
right after the write returns it.

## 11. Library engine

The Library keeps its own engine and imports no project type. It uses the same
shared module for names, selection and list-then-write.

- The record lives at `library/<repository_id>/_commits/<record name>` and is
  written last. `_head.json` is deleted; `LibraryResourceHead` is the in-memory
  view; `ObservedLibraryHead` loses its ETag.
- Push still publishes "the selected set plus the reviewed upserts", never a
  replace. Apply still requires the selected publication to be the one the
  review was computed from, and otherwise stops with its existing "changed after
  review" refusal. The Library does not rebase across a remote change, exactly
  as today.
- Continuity replaces the numeric rewind check with ancestry. The checks for a
  disappeared repository, a changed storage identity and a foreign repository
  id stay.
- Apply takes a local single-flight lock per repository.
- The per-resource accepted state advances on Push, and a row whose accepted
  copy is absent from the selected manifest is not selectable for Push today.
  When a link's accepted head is neither the selected publication nor one of
  its ancestors, reset that link's accepted state the way storage clear
  already does, so its rows can be pushed again.

## 12. Storage clear

Clear stays in place: it keeps the marker, deletes both namespaces, and resets
local link state. Other devices then read `RemoteMissing` or `Replaced` and
recover through Pull and Push.

One change: **records go first.** Clear `repositories/_commits` before the rest
of `repositories`; inside a Library repository `_commits/` already sorts before
`_manifests/` and `_objects/`. Names sort by depth, so a stream's first
publications are deleted first. Once they are gone every other record is
disconnected from the root and the stream selects nothing, while every manifest
and object still exists. An interrupted clear therefore never leaves a selected
record pointing at deleted data. A writer racing the clear is stopped by
list-then-write, or leaves an orphan the next pass deletes; the loop already
repeats until a listing is empty.

One case remains: a first publication written in the one-request gap right after
a clear finishes can reference objects the clear deleted. That stream reads
`Unavailable` until it is cleared again. The clear dialog already says to stop
sync on all machines first.

## 13. What the user sees

Losing publications stay in the storage. Nothing lists them, restores them, or
counts them in the UI in this release. The losing computer still holds the same
content locally, and the existing `Replaced` flow republishes it: Pull and
review, then Push again. An archive browser can be added later without any
storage change, because nothing is deleted.

The only user-visible changes are three notices in existing surfaces. The notice component prints the severity word itself, so the copy never names it:

| When | Copy |
|---|---|
| `RemoteChanged` | "The storage changed while pushing. Compare again." It replaces the CAS-conflict message in the Push review error banner |
| `Stored` with `selected: false` | The Push outcome notice at its warning tone, titled "Pushed, not current": "Another computer's push was selected first. This one is kept in the storage. Pull and review, then push again." It offers the existing Open Pull review action |
| `OutcomeUnknown` | "Couldn't confirm the push. Refresh to see whether it arrived." |

The `Replaced`, `RemoteMissing`, `Advanced` and `Unreviewed` banners keep their
copy and their gates. "Version N" keeps printing the revision.

## 14. Mallard Cloud

Mallard Cloud is not part of this plan. The engine stops writing `_head.json`
and `_tags/`, which is where the managed adapter commits and lists today, so
`managed_cloud` is removed from the production registry until its own plan
lands. Its module keeps compiling against the trimmed trait.

That later plan is small, because the manifest shape, `_objects/<sha256>` and
`commit-<hex>` ids do not change, so the service's manifest validator, its
inventory check, the website's manifest parsing, session views and the release
flow stay valid. It needs four things:

1. The upload carries the record bytes, stored verbatim and validated against
   the manifest.
2. The publications table gains the parent token and `created_at_ms`.
3. A route lists a repository's record names, private scope included.
4. The desktop adapter maps `put_record` to the upload commit and serves the
   stream listing from that route; the stub follows.

Open for that plan: who arbitrates. Recommended: the service keeps its commit
guard and refuses a record whose parent is not its current head, which the
adapter reports as `RemoteChanged`. The service then never forks, and its
single-head schema, cleanup job, quota and release flow stay valid. Deploy the
service before the desktop build that uses it.

## 15. Work plan

Commit the in-flight tree first. Each step leaves the workspace building and its
own tests green.

| Step | Work | Done when |
|---|---|---|
| 1. Domain | `publication.rs`: build and parse names, tokens, the selector, the ancestor test, and the two continuity classifiers rewritten to take a selection | Permutation, tie, missing-parent, sibling-with-greater-revision and continuity-row tests pass; no I/O in the module |
| 2. Shared module | `storage_core/publication.rs`: select over a listing, read-and-verify a record, list-then-write with the wall-clock freshness check, the record cache. Add `put_record` and `list_keys` to the trait with their defaults. Add the project record key constructor | Tests over the local store and a plain-overwrite fake store pass, including the simulated sleep |
| 3. Bundle engine | Record fields and name; `read_head` as select-then-read; publish and registration end in list-then-write; the no-op proof lists; bundle discovery over `_commits/`; delete `publish_tag`, `BundleTag`, the probe, `HeadToken.etag`; the result type. Unregister `managed_cloud` in this step and remove its engine parity test, because its bridge commits on the head write that no longer happens | Engine tests pass with no `_head.json` and no `_tags/` written; `BundleHead` equals the selected record field for field; no build offers Mallard Cloud |
| 4. Service callers | Continuity callers lose the probe closure; the repository-base and `scope_revision` sites; the outcome and error mapping | The project sync suites pass; refresh makes one listing request when nothing changed |
| 5. Library | The same cut in the resource engine; the apply lock; the accepted-state reset; constant `_tag.json` bytes | The catalog suites and the catalog end-to-end flow pass. Its stale-writer cases keep ending in the "changed after review" refusal, and a change that lands between the apply-time listing and the record write ends in `RemoteChanged` |
| 6. Stores | Delete `compare_and_swap` and its types from the trait and every implementation, including the managed store and the three test wrappers; `put_record` on S3 and Dropbox; the local store's `list_keys`; relaxed listings; trimmed probes; the clear order; the marker constant | The three store suites pass; the fake S3 servers and the Dropbox mock no longer emulate conditional overwrite; a foreign name does not fail a listing |
| 7. Copy | The three notices of section 13 and their frontend tests. Copy only, in existing surfaces; show them in a static specimen before wiring, as usual | Frontend suite and type check pass |
| 8. Gate | Request-count tests, provider parity on Local, R2 and Dropbox, a live two-machine pass on R2 and on Dropbox after a wipe, and the documentation (the agent guide's sentence that mutable heads require compare-and-swap, the README, the continuity design) | Section 16 is green and the live pass is recorded |

## 16. Tests

| Test | Required observation |
|---|---|
| Two writers, same parent | Both records are kept; exactly one is selected, the same one on every device |
| Different streams | Neither suppresses the other |
| Different scopes changed by two racing writers | The whole losing snapshot is kept unselected; nothing is merged |
| Timestamp tie | `commit_id`, then the record digest, decides identically everywhere |
| Descendant timestamp or branch length | Does not rescue a losing fork |
| Any listing order, duplicate entries | The same mainline |
| Same `commit_id`, different bytes | Two publications with two names, ranked normally; nothing blocks the stream |
| Record bytes do not match the digest in the name, or fields disagree with the name | `IntegrityError` for that publication when it is read; selection does not move |
| Parent missing from the listing | The child is never selected, and connects when the parent appears |
| Earlier-ranked sibling inside the fork window | The later one is kept and not selected; local files and sessions are untouched |
| Writer sleeps between building a record and writing it | On wake it lists, sees a child, and ends with `RemoteChanged`; nothing is written |
| Listing older than ten wall-clock seconds | The attempt lists again; a monotonic-only check is shown not to catch a simulated sleep |
| Record write fails in the transport | One request, no automatic retry; the next attempt lists first |
| App restarts with a Push in flight | Nothing is resent; no pending record exists on disk |
| Acknowledgment lost | One listing settles it; a repeat writes identical bytes to the same name |
| Crash after each publish step | Objects and a manifest without a record never appear as history |
| No-op while the remote changes | Nothing is written; the compared publication is returned |
| Stored, not selected | `selected: false`; bases stay; the next refresh reads `Replaced`; Pull then Push recovers; Library rows are pushable again |
| Refresh, nothing changed | Exactly one listing request |
| Cold start | Listing, selected record, manifest; nothing was persisted |
| Remote ahead of the base | `Advanced` from the ancestor walk; no probe read |
| Continuity rows | One fixture per row of section 9.1, including a sibling branch with a greater revision (`Replaced`, never `Advanced`) and a cleared stream that is pushed again |
| Push gate | Disjoint scope advanced: published on the new selection with that scope carried byte for byte. Same scope changed: `RequiresPull`. Base not an ancestor: `Replaced` |
| Library apply after a remote change | Any other selected publication than the reviewed one stops the apply with "changed after review"; nothing is written |
| Pull apply | A different selected publication than the reviewed one requires another review |
| Failed page or expired cursor | Fails the refresh; never an empty stream |
| Foreign names | Ignored and logged on all three stores; the listing succeeds |
| Local record listing | `list_keys` lists a stream whose record files cannot be opened, which proves it reads no record bytes; refresh, list-then-write and discovery call `list_keys`, never `list` |
| `BundleHead` view | Equals the selected record; readers of `snapshot.head` are untouched |
| Bundle discovery | The `_commits/` listing plus the tip records reproduce today's summary rows; no payload key is listed |
| Marker | Two first writers converge on identical bytes |
| Remote writes | No `_head.json`, no `_tags/`, no flag or moved object is ever written |
| Clear | Records go first; an interrupted clear leaves streams that select nothing; a racing writer ends with `RemoteChanged` |
| Library double Apply | The lock stops the second; one record is written |
| Setup probes | R2 and Dropbox setup pass without any conditional-overwrite stage |
| Provider parity | The same record set selects the same mainline on Local, S3/R2 and Dropbox |
| Large history | Listing cost is measured; a budget reports an incomplete view and never truncates silently |

Existing transcript, owned-subagent, scope-firewall, deletion, secret-handling,
destination-isolation, key-validation and frontend review regressions run
unchanged.

Release condition:

> A store that offers only put, get and list, with plain overwrite and no
> conditional write, runs project and Library publication, refresh, review and
> restore. The existing stores run that same engine.

## 17. Not in this plan

- An archive browser, "Restore as a copy", or any count of unselected
  publications in the UI.
- Any detection, message, conversion or test for data written by an older
  build.
- Remote garbage collection, compaction, or sharding of records.
- Opaque locators, a snapshot envelope, a payload table, a new transport trait,
  and a storage UID. An identifier-addressed provider can be written against
  this contract by resolving a key through a folder listing and a name-to-id
  cache, which is safe because every name is unique by construction and every
  read is hash-verified.
- Mallard Cloud, Feishu, and any other new provider.
- Resource-level merging of racing publications.

## Source pointers

Line numbers are from the working tree on September 18, 2026.

| Subject | Location |
|---|---|
| `ObjectStore` trait; CAS types; `put_immutable`; `compare_and_swap`; `list`; clear | `crates/mallard-infrastructure/src/storage_core/object_store.rs:201-305`, `:154-163`, `:242`, `:256-261`, `:287-292`, `:265-283` |
| Layout constants; key and prefix validation; key constructors; the clear loop; marker check on shared open | `crates/mallard-infrastructure/src/storage_core/mod.rs:41-56`, `:71-120`, `:260-299`, `:141-203`, `:353-379` |
| Marker constants, idempotent creation, classification | `crates/mallard-infrastructure/src/storage_core/marker.rs:12-13`, `:59-75`, `:120-134` |
| `HeadToken`, `PublishExpectation`, `PublishedBundle` | `crates/mallard-infrastructure/src/bundle_engine.rs:47-70` |
| The commit record and the tag | `bundle_engine.rs:435-470` |
| `read_head`; registration; publish (pins, carry-forward, no-op re-read, uploads, manifest and commit, head CAS) | `bundle_engine.rs:545-569`, `:577-688`, `:704-1131` |
| `inspect_optional_with_token`; base manifest read; continuity probe; bundle discovery; tag publish | `bundle_engine.rs:1168-1185`, `:1190-1233`, `:1243-1275`, `:1280-1338`, `:1812-1844` |
| Test store wrappers | `bundle_engine.rs:2484`, `:3038`; `src-tauri/src/sync_tests/thread_variants/storage_parity.rs:435-469` |
| Id generator; scope keys; `RepositoryBase`, `ScopeBase`; both classifiers; `BundleHead` | `crates/mallard-domain/src/lib.rs:440-451`, `:618-654`, `:849-893`, `:923-1044`, `:3975-3985` |
| Refresh read; Push gate; probe closure; gate messages | `src-tauri/src/project_sync/service.rs:7041-7073`, `:7607-7709`, `:7721-7736`, `:7738-7790` |
| Apply-time read; publish call; bases advance; Pull apply equality; `scope_revision` max; repository base; bundle listing | `service.rs:10116`, `:9924-9937`, `:10445-10563`, `:12478-12483`, `:12231` and `:13243-13244`, `:13583-13588`, `:3118-3145` |
| Local store: lock, immutable put, CAS, list and its per-entry hashing | `crates/mallard-infrastructure/src/storage_core/local_store.rs:93-103`, `:255-270`, `:272-291`, `:337-424` and `:411-419` |
| S3 store: probe stages and probe, conditional put, immutable put, CAS, list and its parse failure, client builder | `crates/mallard-infrastructure/src/storage_core/s3_store.rs:49-57`, `:198-333`, `:647-692`, `:694-734`, `:736-778`, `:780-907` and `:842`, `:1252-1292` |
| Dropbox store: client retries off, listing strictness, CAS; upload window; probe | `crates/mallard-infrastructure/src/storage_core/dropbox_store.rs:338-341`, `:794-805`, `:939-977`; `dropbox_store/upload.rs:73-139`; `dropbox_store/probe.rs:102-197` |
| Fakes that emulate conditional writes | `s3_store.rs` tests, `src-tauri/src/sync_tests/stub_s3.rs`, `dropbox_store/mock.rs` |
| Destination identity per adapter; one storage per destination; provider registry | `crates/mallard-infrastructure/src/storage_providers/local.rs:181-191`, `r2.rs:568-578`, `dropbox.rs:148-155`, `managed_cloud.rs:144-150`; `crates/mallard-application/src/storage_setup.rs:1231-1253`; `storage_providers/registry.rs:40-60` |
| Library engine: storage identity, continuity, observed head, head read, publish and its CAS | `src-tauri/src/storage_catalog/resource_engine.rs:30-42`, `:58-92`, `:94-98`, `:146-164`, `:228-446` |
| Library types: ids, tag, commit record, head coordinate | `src-tauri/src/storage_catalog/domain.rs:54-62` and `:143-145`, `:664-672`, `:829-842`, `:860-867` |
| Library service: workspace resolution, apply Push (merge, accepted state); row selectability; clear reset | `src-tauri/src/storage_catalog/service.rs:1002-1094`, `:1700-1897`; `resource_planning.rs:258-275`; `persistence.rs:546-558` |
| Managed adapter bridge | `crates/mallard-infrastructure/src/managed_cloud/store.rs:48-118`, `:659-705` |
| Push outcome and failure copy | `src/components/project-sync/syncOutcome.ts:161-181`, `ProjectSync.tsx:2057-2109`, `PushResourceWorkspace.tsx:1020` |
| Tests that assert CAS conflicts or stale writers | `storage_parity.rs:85`, `:370`, `:668`; `src-tauri/tests/support/catalog_flow.rs:1130`, `:1185`, `:1423`, `:1499` |
| The agent guide's compare-and-swap sentence | `AGENTS.md:162-163` |
| `Instant` on macOS reads `CLOCK_UPTIME_RAW` | Rust 1.95 `library/std/src/sys/time/unix.rs:62-67` |
