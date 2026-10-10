# Mallard: CAS-free storage publication — implementation plan

> Historical reference: The contents of `mockups/` and `poc/` were removed
> on 2026-10-09. Paths and commands below that use those files are historical
> and no longer work in this checkout. Git history retains the tracked files.

- Status: Implemented, steps 1 to 7, 9 and 10 (the claim of decision 11 and
  section 7.2, and the publishing part of decision 12, both on September 22)
  and the automated part of step 8; the live two-machine pass on R2 and on
  Dropbox (step 8, and the two live rows of step 9) is still to be run and
  recorded. The rest of decision 12, which withdrew the rollback of section
  9.4 and the Library reset at load, landed on September 22 as steps 11 to 14
  of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`, without claims. *Implementation
  notes* at the end list what differs from the plan
- Last reviewed: September 22, 2026
- Scope: the desktop app — both sync engines, the Local, S3/R2 and Dropbox stores, and the Push error copy. Mallard Cloud followed in its own plan (section 14, landed September 23, 2026)
- Compatibility: clean cutover; no migration, no legacy detection, no legacy execution path
- Deployment assumption: no deployed user state must be preserved; the owner clears `~/.mallard` and the test storages before acceptance testing
- Schemas: every schema and layout number stays `1`
- Related: `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md` (how each decision was reached), `DESIGN_REMOTE_HISTORY_CONTINUITY.md` (design history; its rows 1 to 8 are replaced by section 9), `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` (what a cleared or recreated storage and a stale record do; it changed sections 7.2, 9.1, 9.2, 11, 12, 13 and 16), `PLAN_PULL_PINNED_VERSION.md` (the Pull side; the claim of section 7.2 removes its rows T3, Q3, Q4 and G2 on S3/R2 and Dropbox)

Code locations are collected in *Source pointers* at the end, not in the body.

## 1. Goal

Today a Push ends with a compare-and-swap on a mutable `_head.json`. A provider
that cannot do a conditional write cannot be a Mallard storage; the provider
research names Google Drive, Aliyun OSS, Huawei OBS and Tencent COS as blocked
for that reason.

After this change a provider needs three operations for ordinary sync: **write
bytes at a key, read a key, list a prefix.** There is no mutable head. The
current snapshot of a bundle is derived, by shared code, from immutable records
that every Push already writes. A provider that can also refuse a second write
of a key gives its users one thing more, a listed version that can never lose
(decision 11); one that cannot still runs everything.

Decisions, all final:

1. **The commit record is the publication point.** The `_commits/` record that
   every publish writes today, and nothing reads, is written last and becomes
   the only thing that publishes a snapshot. `_head.json` and the `_tags/`
   index are deleted.
2. **Earliest sibling wins.** When two records name the same parent, the one
   with the smallest `(created_at_ms, commit_id, record_sha256)` is selected.
   The other is kept and not selected, and the Push that wrote it failed
   (section 7.3).
3. **List, then write.** A record is written only right after a listing that
   shows its parent selected and childless. This removes the writer that keeps
   a record and sends it later; it narrows the fork window and does not bound
   it, because a writer can pause between the check and the send (section
   7.2).
4. **Refresh is one listing of one stream.** A record's name carries its graph
   facts, so the head is selected without reading records. An unchanged
   selection reads nothing else.
5. **Identity is the commit id.** Continuity, the Push gate and forks are decided
   by identity and ancestry, never by revision order. Local pins, IPC types and
   TypeScript types keep their shape.
6. **Record names are `<commit_id>-<sha256>`.** The same id with different bytes
   is two publications, not a conflict. No state blocks a stream.
7. **No storage UID.** A storage is its provider destination, as today.
8. **A losing Push counts as not made, and recovery is the user's.** Its
   record is kept in the storage and never selected. If it loses while it runs,
   it fails with `RemoteChanged` and nothing local is accepted. If another
   computer's record outranks it after it was confirmed, the next refresh
   reports that this computer's last publication is no longer on the selected
   history, and changes nothing: no base moves, no acceptance is forgotten,
   nothing is republished (section 9.4). The user compares again and pushes
   again. Nothing is reset by hand either. There is no archive UI in this
   release.
9. **No legacy handling of any kind.**
10. **Desktop first.** Mallard Cloud followed in its own plan (section 14) and is offered again since September 23, 2026.
11. **Claim, then write.** On a store whose server refuses a write to a key
    that already holds other bytes, in the same request, a Push writes a
    create-only **claim** under its parent's token before it writes its record.
    A parent then has one child, and a listed version is final while the
    storage's claims and records are intact (section 12). The claim changes no
    selection rule and is never listed or read by a refresh. S3/R2 and Dropbox
    are such stores; the local store is not, writes no claim, and decisions 2,
    3 and 8 do the whole job there (section 7.2).
12. **Recovery is a user action.** A failed Push stops. A Push sends one claim
    request and one record request; nothing is retried, resent after a
    restart, or finished on another computer's behalf. A request whose outcome
    is unknown is reported as `OutcomeUnknown` after a read-only check. A
    confirmed Push that later loses is reported, and nothing local moves: no
    base returns to an ancestor, no acceptance is forgotten, no Library state
    is reset, nothing is republished. A stale base or acceptance is kept and
    read as absent, so it never reports a session as synchronized and never
    lets a Push through without a fresh comparison and the user's approval. A
    claim whose record is missing is finished only when the user asks
    (sections 7.3, 9.4 and 13).

## 2. What stays exactly as it is

| Area | Unchanged |
|---|---|
| Transport | The `ObjectStore` trait apart from the changes of section 10 (`compare_and_swap` removed; `put_record`, `list_keys`, `supports_claims` and `put_claim` added), `ObjectKey` and `ObjectPrefix` validation, the blocking execution model, verified staged reads, the shared-store wrapper and connection health |
| Remote objects | `_objects/<sha256>` and `_manifests/<revision>-<commit_id>.json`, for bundles and for the Library |
| Portable schemas | `BundleManifest` and `LibraryResourceManifest`, including `revision`, `commit_id` and each file entry's `object_key` |
| Local state | `RepositoryBase`, `ScopeBase`, `MaterializationRecord`, `ConversationApplyReceipt`, `AcceptedConversationVersion`, `LibraryHeadCoordinate`, and every file under `~/.mallard` |
| IPC and TypeScript | `BundleSnapshot`, `RestorePlan`, `PullReview`, `ConversationRevisionPins`, `PushDestinationStatus`, `RemoteContinuity`, `ScopeContinuity`, and their mirrors. Step 12 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` adds one variant, `RemoteContinuity::Displaced` (section 9.1); no field changes shape |
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
      _claims/                               one claim per claimed parent, and nothing else
        <bundle_id>/
          <parent token, or root>.json
      _commits/                              every project record, and nothing else
        <bundle_id>/
          <record name>
      <bundle_id>/
        _manifests/<revision>-<commit_id>.json
        _objects/<sha256>
    library/
      <repository_id>/
        _tag.json
        _claims/<parent token, or root>.json
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

A claim lives beside the records and never among them: `_claims/` is not under
`_commits/`, so no record listing returns one and no refresh sees one. Its name
is the parent it claims, the parent's token or the word `root`; its bytes are
the record of that parent's child, exactly as written under `_commits/`
(section 7.2). Only S3/R2 and Dropbox hold claims.

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
- A claim takes no part in selection. On every store the head is selected from
  record names alone; the claim only decides which record can be written
  (section 7.2).

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
7. **List, claim, write, then list again** and select (section 7.2).
8. **Stored:** the listing after the write selects this record, on every
   store. Advance the local bases to it, as a Push does today. If the claim is
   taken, the Push failed with `RemoteChanged` or `PushUnfinished`; if that
   listing does not select this record, with `RemoteChanged`. Nothing local
   moves either way (section 7.3).

Registering an empty project is the same flow with no parent and the existing
registration manifest. If the stream already has a selected publication, the
registration returns it and writes nothing, as it does today. A registration
that loses the fork window returns the established one the same way.

### 7.2 List, then write

Earliest-sibling selection has one hazard: a record with an old timestamp that
is written late outranks every sibling created in between. Only a writer that
prepares a record, keeps it, and sends it later can produce one. This plan
removes that writer.

Every attempt to write a record is the second half of one step:

1. List the stream and select. If the listing already holds this record's
   token, the record landed: the outcome is `Stored` when the listing selects
   it, and `RemoteChanged` when it selects another computer's record.
2. Continue only if the selected publication is the record's parent, which
   means the parent has no child. For a first publication, continue only if the
   stream selects nothing. A listed record that connects to no chain does not
   count: a Push whose parent was cleared under it leaves one behind and fails,
   an interrupted clear leaves some, and the stream must still take its next
   first version (`PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`, R5). Otherwise stop
   with `RemoteChanged`; nothing is written.
3. **Claim the parent**, on a store with claims (S3/R2 and Dropbox). Write the
   record's bytes, create-only, at `_claims/<parent token, or root>.json`. The
   store answers in one request. The key was free: the claim is **mine**. The
   key holds other bytes: the claim is **taken**, and the Push stops before it
   writes its record. Read the claim; its bytes are the record that was claimed.
   When the listing of step 1 holds that record, or one fresh listing taken
   now does, another computer got in first: `RemoteChanged`. When neither
   does, that push did not finish: `PushUnfinished`, and finishing it is the
   user's (section 7.3). Nothing
   sends that record by itself, and nothing deletes or reassigns the claim. One
   case needs no user: the claim holds this computer's own record, for the same
   content on the same parent, from an earlier attempt whose record request
   failed. Then this Push writes that record instead of the one it built and
   goes on; it is the same publication, and the local save is this Push's. Same
   content means the same scope digests, read from that record's manifest. A
   store without claims skips this step.
4. Send the record, once, then list again: `Stored` when that listing selects
   the record, on every store. The claim proves that the parent has no other
   child; it does not prove that the parent is still there. A clear between the
   claim and the record deletes the parent, the record lands as an orphan that
   nothing selects, and only the listing shows it (section 12).

Four details make this hold on a laptop:

- **One attempt.** A Push lists first, and lists again when its listing has
  gone stale, then sends its claim once and its record once. It never sends
  either twice, and it never resends after a restart.
- **The listing is fresh.** If more than ten seconds of wall-clock time passed
  since the listing answered, list again before sending. The monotonic clock is
  not enough: on macOS it stands still while the machine sleeps.
- **The write is one request.** `put_record` and `put_claim` never retry
  inside the transport, because a transport retry is an attempt without a
  listing. A request that stalls dies with its connection. A request that has
  not started yet does not, which is the gap below.
- **Nothing is kept.** No journal, no outbox, no resend after a restart. Push
  stays stateless, as it is today.

**What this narrows, and what it does not.** A sibling of a publication can be
written only by a computer whose listing predates the publication's
visibility. In practice that listing is at most ten seconds old when the record
is sent and the request takes a second, so a fork needs two computers pushing
within the same seconds, and a laptop that slept three days between building
its record and listing sees a child and stops. But the freshness check and the
send are two steps. A computer can pass the check and pause before its request
leaves, in a sleep or a suspended process, and come back minutes or days later
with a record whose stamp is older than everything pushed since; nothing in the
protocol stops that request, and a store without claims has no server-side
check to refuse it. A folder shared through a sync product is the same case
without the pause: a replica that was offline delivers its record when it
reconnects, however late. **On a store without claims the fork window has no
bound.** The listing lag of such a folder is unbounded too.

What holds there is convergence, not finality. Every computer selects the same
mainline from the same names. A late record with the earlier stamp is selected
and displaces its sibling and everything built on it; every computer whose
bases sit on the displaced branch is told so at its next refresh (section
9.4), whenever that is, and compares and pushes again. Nothing is lost, and
nothing moves by itself. The writer of a losing record learns it from the
listing after its write, and its Push fails (section 7.3). Finality, a listed
version that can never lose, needs the claim, and only S3/R2 and Dropbox have
it, while their claims are intact (section 12).

**The claim closes the window.** S3/R2 and Dropbox refuse a write to a key
that already holds other bytes, in the same request. The claim is the one key
two computers reach with different bytes, `_claims/<parent>.json`, and the
store admits exactly one of them. So a parent has at most one child, a listed
version has no sibling, and selection is final, whatever the listing lag, the
clock, or the age of the listing that admitted it: a stale listing costs a
taken claim, never a second child, and a writer that pauses after its listing
finds its claim taken when it wakes. A version that a Pull review pinned cannot
lose afterwards (`PLAN_PULL_PINNED_VERSION.md`, T3 and Q4), and a confirmed
Push cannot be outranked (section 9.4). All of it holds while the storage's
claims and records are intact; an interrupted clear can remove a claim, and
section 12 says what follows.

The claim goes before the record because a record written first would be
listed, and the selector reads names: it would rank the loser against the
winner as if there were no claim. The claim's bytes are the record because a
writer can die between its claim and its record, and its parent would stay
claimed. The next Push onto that parent, from any computer, finds the claim
taken and the record absent, and stops: `PushUnfinished`. Finishing that push,
which writes the record the claim holds, is a user action from the Push review
(section 13); nothing finishes it by itself, and nothing deletes or reassigns
the claim. Once finished, the writer that abandoned it is in the state of a
Push whose record arrived while its local save never ran, L3 of
`PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`: its areas read `Advanced` and hold its
own content, a session it continued reads "Keep both", and a session it
published for the first time appears twice until the mapping is recreated from
the manifest it published (that plan's section 9). The record's name is read from its bytes, the five graph
facts a name repeats and the digest of the bytes, and a claim whose bytes are
not a record of that parent is an integrity error for that parent, like a
record whose bytes do not match its name: nothing selects differently because
of it, and only a clear removes it.

A Push on a store with claims makes one request more than today: listing,
claim, record, listing. The listing after the write stays, because the claim
answers one question, whether another computer got in first, and not the other
one the listing answers, whether the parent still exists. Its taken path reads
the claim once, lists once more when the listing it has does not hold the
claimed record, and sends at most one record. A refresh never lists or reads a
claim.

The local store writes no claim, and `supports_claims` says so. Nothing
arbitrates a folder: two computers that share one through a sync product would
each create the claim in their own copy, and a network share promises no
create-only write. There selection converges and is never final: the report
of section 9.4, at any later refresh, and the user's next Push are the way
back. The Library claims the same way, under `library/<repository_id>/_claims/`
(section 11).

### 7.3 Outcomes and failures

`PublishedBundle { published, snapshot }` and the Library's equivalent become:

```text
Stored   { snapshot }
NoChange { snapshot }
```

`Stored` means written and selected. A record that is written and not selected
is not an outcome: it is the `RemoteChanged` error, raised by the shared
`write_record`, so both engines fail the same way.

Errors: `RemoteChanged` replaces every CAS-conflict string, and
`OutcomeUnknown` is reported when the storage cannot be read after a claim or
a record was sent, so the one read-only check that would settle the request
cannot be made. A check that can be made settles it: what it shows landed
counts, and what it shows absent did not land, so the Push fails with the
store's own error and nothing is sent again.

| Boundary | What happens |
|---|---|
| Before the record is sent | Nothing is published. Objects and the manifest are unreferenced residue |
| The listing before the write shows another child or another selection | `RemoteChanged`. Nothing is written. The user compares again |
| The claim is taken, and the listing holds its record | `RemoteChanged` before the record is sent: another computer got in first. Nothing is written. The user compares again |
| The claim is taken, and its record is not listed | `PushUnfinished`: a push onto this version did not finish, this computer's own or another's. Nothing is written, and nothing finishes it by itself. The Push review offers *Finish that push*, which writes the record the claim holds, once, then lists and reports Finished, `RemoteChanged` or `OutcomeUnknown`; the user compares again (section 13). When the claim holds this computer's own record for the same content on the same parent, the Push writes that record as its own and goes on |
| The claim request is not acknowledged | A read-only check: read the claim key. This computer's bytes: the claim is its own, and the Push goes on to the record. Other bytes: taken. Absent: the Push fails with the store's error, and nothing is sent again; a claim that lands after all is this computer's own, and its next Push meets it as such. The read itself fails: `OutcomeUnknown` |
| The storage is cleared between the claim and the record | The record lands as an orphan: its parent is gone. The listing after the write does not select it, the Push fails with `RemoteChanged`, and nothing local moves; the next refresh reads the cleared storage. When that listing came before the clear reached the record, the Push was confirmed and its records are stale, as R5 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` already says |
| This computer's claim landed and its record never did | Its next Push finds its own claim taken. Same content on the same parent: it writes that record as its own, and its local save is this Push's; no L3. Other content: `PushUnfinished`, nothing written; *Finish that push* writes the earlier record, and this computer is then in L3 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` for the sessions of that earlier push. When another computer finished it first, this computer's Push gate reads `Advanced` before any claim, and the same L3 applies |
| The machine sleeps between the listing and the write | The listing is older than ten seconds, so the attempt lists again. On a store with claims a stale listing could only end in a taken claim |
| The record was sent and no acknowledgment arrived | One listing, read-only. Present and selected: `Stored`. Present and not selected: `RemoteChanged`. Absent: the Push fails with the store's error, and nothing is sent again; a record that lands after all is found by the next refresh, reads `Advanced` with equal content, and the sessions of that push are L3. The listing itself fails: `OutcomeUnknown` |
| The record request fails outright: a store error, a refused connection | The same one listing shows that the record did not land, and the Push fails with the store's error. Nothing is resent. On a store with claims the claim stays: the next Push with the same content writes the claimed record as its own; with other content it reads `PushUnfinished` |
| The storage cannot be reached after sending | `OutcomeUnknown`. Nothing is persisted and nothing is resent; the next refresh shows whether it arrived |
| The app quits or crashes | Nothing is resent, and nothing on disk names the attempt. The next Push builds a new record, and on a store with claims meets its own claim as above |
| Stored, but the local bases were not saved | The next refresh reads `Advanced` with content equal to local; a Pull review with nothing to apply advances the bases |
| The listing after the write selects another computer's record (a store without claims) | `RemoteChanged`. The Push failed as a whole: no base, checkpoint, binding, receipt or Library accepted state moves, because every one of them is saved only after the publish call returns. The record is kept and never selected. The user compares again, reads `Advanced`, and pushes again; the sessions are published then, because they were never accepted |
| Another computer's earlier-ranked record becomes visible after this Push was confirmed | On S3/R2 and Dropbox: only after an interrupted clear removed the claim (section 12). On the local store: whenever the other computer listed before this record was visible, however late its record lands, from a pause after its listing or a replica that was offline (section 7.2). The Push was confirmed and is not reported as failed. The next refresh, whenever it is, reports `Displaced` and changes nothing; the user compares again and pushes again (section 9.4) |

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
| Push apply, after the uploads | listing + claim + record write + listing on S3/R2 and Dropbox; listing + record write + listing on the local store | head CAS, tag read, tag CAS in up to four rounds |
| Library scope page | listing + record + tag + manifest; the record is cached while the selection is unchanged | head + tag + manifest |

Remote reads happen per user action, as today. There is no background polling.
Nothing is persisted for discovery: no index, cursor, or scan state. The
process caches the last listing's names and the records it has read. No
refresh, review or discovery lists or reads a claim: `_claims/` is beside
`_commits/`, not under it.

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
| the base is not on the chain, its record is listed, and its parents lead to the chain | `Displaced` |
| any other base | `Replaced` |
| the listing failed | `Unknown` |

- The ancestor test walks parent tokens from the selected publication down to
  the base's revision and compares `commit_id`. Tokens are hash-linked, so the
  walk needs no record read and no probe.
- When a link holds several scope bases, continuity is `Replaced` if any of
  them is neither the selected publication nor one of its ancestors.
- `Replaced` means a cleared or recreated stream: the base is not listed, or
  no listed record connects it to the selected chain. `RemoteMissing` means a
  cleared stream that nobody has pushed to since. `Displaced` means another
  computer's record outranked the publication the base was reviewed in; the
  storage was not cleared (section 9.4). None of them blocks anything: they
  only drive the standing notices of section 13
  (`PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`, section 5). `Displaced` is the one
  addition to `RemoteContinuity` and its TypeScript mirror.
- A Push that another computer's outranks while it runs fails and moves no
  base (section 7.3). One outranked after it was confirmed reads `Displaced`:
  its bases are stale and are read as absent, like a cleared history's, and
  nothing is written (section 9.4).

A scope base is **usable** while the publication it was reviewed in is on the
selected chain, and **stale** otherwise. A stale base is read as if it were not
recorded. It is never deleted at a refresh: a short listing looks the same as a
clear, and the base is usable again when its publication is listed again. The
Push or the Pull that saves new bases removes the stale ones in the same
transaction.

| The scope in the selected manifest | Scope base | `ScopeContinuity` |
|---|---|---|
| absent | none, or stale | `Fresh` |
| absent | usable | `RemoteMissing` |
| present | none | `Unreviewed` |
| present, with the base digest | usable or stale | `InSync` |
| present, with another digest | usable | `Advanced` |
| present, with another digest | stale | `Unreviewed` |

`scope_revision` stays in the manifest and in `ScopeBase` as a per-branch
counter for display. Nothing compares it.

A session acceptance follows the same rule, by the commit it was accepted in:
while that publication is on the selected chain the acceptance is compared
against, and otherwise it is read as absent, with the acceptance of its
subagents. First-contact adoption then proves an equal or an earlier stored
copy from its content, and a stored copy it cannot prove classifies
`Divergent`: Push asks for a Pull, and Pull keeps both copies.

### 9.2 Push gate at apply

| Freshly selected publication | Action |
|---|---|
| the one this replica is based on | publish on it |
| a descendant, and every writable scope still has this replica's base digest | publish on the new one; carry the other scopes from it |
| a descendant, and a writable scope changed | `RequiresPull` |
| none: the stream lists empty | publish the stream's first record; every writable scope is `Fresh` |
| not a descendant, and every writable scope is `Fresh` or `InSync` | publish on it; carry the other scopes from it |
| not a descendant, and a writable scope holds other content | `RequiresPull`: the scope was never reviewed in this history |

Ancestry answers whether the remote moved forward. The scope digest answers
whether this Push's own scopes were touched. The gate is the scope table of
section 9.1 and nothing else: a cleared or recreated history is no state of its
own, and no case asks for an unlink or a reset.

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
| Whole-head equality at Pull apply, the project-files review token, the readiness match | Pull apply looks the reviewed publication up on the selected chain and reads it by name, since September 22 (`PLAN_PULL_PINNED_VERSION.md`, section 4.1); the review token and the readiness match still test `(revision, commit_id, manifest_sha256)` for equality |

Pull apply requires the reviewed publication to be on the selected chain and
reads it by name: a newer publication pushed since the review does not stop
it. The local mutation is always of the exact reviewed bytes.

### 9.4 A Push outranked after it was confirmed

On S3/R2 and Dropbox this cannot happen while the storage's claims are intact:
the parent of a confirmed Push was claimed, so no sibling can land after it
(section 7.2). An interrupted clear can remove that claim while keeping the
parent and its records, and a delayed writer can then displace the confirmed
Push there too (section 12). On the local store, which writes no claim, it can
happen at any later time (section 7.2).

A Push there is confirmed by the listing after its write. Another computer's
record can still land after that listing, usually within seconds and in
principle at any later time: that computer listed before this record was
visible, then paused or was offline, and its record carries the earlier
timestamp. The confirmed Push then holds a publication that is listed and not
selected, and its local bases and acceptances describe a history nobody
selects. A loss found later is not a failure of that Push: the Push was
confirmed, and the report says so.

The storage proves the case by itself. The base's record is listed, it is not
on the selected chain, and walking its parent names over listed records reaches
the chain. Ranks never change, and records are deleted only by a clear, so the
verdict is final; a lagging listing can hide records but cannot produce it. A
base that no listed record connects to the chain is a cleared or recreated
stream, `Replaced`, or what an interrupted clear left behind; this one is
`Displaced`. Both are stale bases, and both are read the same way.

**Nothing is rolled back.** Recovery is a user action (decision 12). The refresh
that finds a base reviewed in a displaced publication reports `Displaced` and
changes nothing: no base returns to a shared ancestor, no acceptance is
forgotten, no Library accepted state is reset, nothing is republished. The
saved state stays exactly as it is and is read by the rule of section 9.1: a
base or an acceptance whose publication is not on the selected chain is read as
absent.

| Local state | While `Displaced` |
|---|---|
| A scope base reviewed in a displaced publication | Kept. Read as absent, except that equal content is still `InSync`. Replaced by the next Push or Pull that saves bases, which removes the stale ones in the same transaction |
| A session checkpoint accepted at a displaced publication | Kept. Read as absent, with the acceptance of its subagents. Replaced by the next Push or Pull that saves an acceptance |
| Session-to-cloud-id mappings, recipes, preferences, receipts | Unchanged |

What the displaced computer then reads, with nothing written:

- An area whose reviewed content the selected history holds exactly reads
  `InSync`. Every other area it holds a base for reads `Unreviewed`, whether
  the winner changed it or it holds this computer's displaced content. Push is
  blocked while a writable area is `Unreviewed`: a Pull review comes first.
- In that Pull review a file this computer changed on the displaced branch
  reads "Replace file · keeps local copy", because there is no usable base.
  Left unticked it stays as it is, the Pull still marks the area as reviewed,
  and the next Push publishes it onto the selected history. Ticked, the stored
  version is written and this computer's version is kept beside it.
- A session accepted on the displaced branch is proved from content: an equal
  or earlier stored copy reads "Mark synced" or "Update storage", a longer or
  different one "Keep both", and a session the storage never had "Publish new
  task", under the cloud id it already has. A stale acceptance never reports a
  session as synchronized.
- The Push that follows is a new publication onto the selected history, under
  the existing gate and Pull-review rules, from a fresh comparison the user
  approves. Nothing about it is automatic.

**Every reader of a base asks whether it is usable.** The scope classifier and
the checkpoint reader already do. Two more readers open a base's manifest by
its coordinates: the files review's per-file states and the Pull planner's
per-file base digests. For a cleared base that manifest is gone and both fall
back safely; for a displaced base it still exists, and reading it would let a
file this computer changed on the displaced branch pass as changed only in the
storage and be replaced plainly, with a backup but without the user's choice.
Both readers take the base through the same usability rule first and read
nothing from a stale one. This is what "preserved metadata must not authorize
an unsafe operation" means here.

A third computer that pulled the displaced publication reads the same way and
converges on the same cloud ids. The Library follows the same rule: an accepted
head that is not on the selected chain is read as absent, and nothing is reset
at load (section 11).

The state is told by one standing notice in the project activity notice zone
while the comparison reports `Displaced` (section 13), and it goes away when a
Push or a Pull has replaced the stale bases. There is no window event, no
shell notice and no rollback transaction: `observe_linked_bundle` classifies
and writes nothing.

## 10. Store contract and per-store work

| Member | After |
|---|---|
| `get_metadata`, `get_metadata_many`, `stage_verified`, `stage_many_verified` | Unchanged |
| `put_immutable`, `put_many` | Same signatures. The contract no longer promises an atomic no-overwrite. A store may keep its native mechanics; a new provider may implement a plain write |
| `put_record` — new | One request, no transport retry. Used only for the commit record. The default forwards to `put_immutable` |
| `compare_and_swap` and its types | Removed |
| `list` | Same signature. It skips and logs a name it cannot represent instead of failing the listing. A completed write is visible to every later listing of that prefix, or the adapter states its worst-case lag. After the cut only storage clear calls it, because clear deletes exactly the ETag it listed |
| `list_keys` — new | The same listing without ETags. Every record listing uses it: refresh, list-then-write, and bundle discovery. The default forwards to `list` and drops the ETags, which is free on S3/R2 and Dropbox, where the ETag arrives in the listing response. The local store overrides it |
| `supports_claims` — new | Whether the store's server refuses a write to a key that already holds other bytes, in the same request. `true` on S3/R2 and Dropbox. `false` by default, on the local store, and on any store whose exclusivity is not proved |
| `put_claim` — new | One create-only request at the claim key, no transport retry. Answers **mine** when the key was free or already held these bytes, and **taken**, with the bytes found, otherwise. Used only for the claim. The default is an error; callers ask `supports_claims` first |
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

The claim is the one key two writers reach with different bytes, and that is
its purpose: the store refuses the second in the same request. It is the only
conditional write left, and only S3/R2 and Dropbox make it.

| Store | Work |
|---|---|
| Local folder | Delete `compare_and_swap`. `put_record` is `put_immutable`. `list_keys` walks the prefix and returns names without opening a listed file; `list` keeps hashing, for clear. Reads, the lock and no-follow access are unchanged. It writes no claim: `supports_claims` is false |
| S3/R2 | Delete `compare_and_swap`, the `If-Match` branch and the ambiguous-CAS resolution. `put_record` runs with SDK retries off; the client is built with the SDK default of three attempts today. `list` skips and logs a key it cannot parse. The capability probe loses two stages. `put_claim` is the create-only put `put_immutable` already uses (`If-None-Match: *`), with SDK retries off; on a precondition failure one read tells this attempt's own bytes (mine) from another's (taken, with those bytes) |
| Dropbox | Delete `compare_and_swap` and the update-by-revision write mode. `put_record` is one `files/upload` request, not a session start plus a batch commit with Mallard's code in between. `list` skips and logs a non-canonical or conflicted-copy path. The setup probe loses its update and stale-revision checks. `put_claim` is `put_record` at the claim key: `files/upload` in add mode without strict conflict answers identical bytes as written (mine) and other bytes as a conflict (taken); one download then returns them |
| Shared wrapper | Its forwarding methods shrink with the trait, and forward `supports_claims` and `put_claim` |
| Mallard Cloud | The R2 store's twin through the service's gateway (section 14): `put_record` and `put_claim` are one `PUT` each, create-only on R2; `list_keys` is one listing request per stream; `supports_claims` is true |

A setup probe tests a write, a verified read-back, and that a listing taken
right after the write returns it. On a store with claims it also writes other
bytes to a key it just wrote and expects the refusal: a destination that
overwrites cannot be set up as a store with claims.

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
- Continuity replaces the numeric rewind check with ancestry. A repository
  that lists empty holds no chain, so an accepted head is not on it: it reads
  replaced like any other cleared repository, the page loads, and one Push
  repopulates it. An accepted head that names another storage identity or
  another repository describes a history this storage does not hold, and is
  read the same way, as absent, with nothing written; since September 22
  neither is a refusal (implementation notes).
- Apply takes a local single-flight lock per repository.
- A Library record claims its parent the same way, at
  `library/<repository_id>/_claims/<parent token, or root>.json`, through the
  same shared module. The lock stays: the claim is the exclusion between
  computers, the lock the one between two windows of one app.
- The per-resource accepted state advances on a Push that was stored, which
  means written and selected. A Push that loses the fork window, or finds the
  claim taken, fails with `RemoteChanged` before the accepted state is saved,
  and the next review offers its rows again.
- A row whose accepted copy is absent from the selected manifest is not
  selectable for Push today. When a link's accepted head is neither the
  selected publication nor one of its ancestors, the page reads the accepted
  state as absent and writes nothing: every row compares as unreviewed and
  goes through the Pull choice, and the next Push saves a new accepted state.
  This serves a cleared or recreated repository (B1, B2 and B3 of
  `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`, whose reset at load it replaces) and a
  Push that was outranked after it was confirmed: the Library's counterpart of
  section 9.4. Nothing is reset by the page (decision 12).

## 12. Storage clear

Clear stays in place: it keeps the marker, deletes both namespaces, and resets
local link state. Other devices then read `RemoteMissing` or `Replaced`. Their
bases and their session acceptance are stale, are read as absent, and they
recover by pushing again, after a Pull when another computer already pushed
into an area they write (`PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`). Nothing asks
them for an unlink or a reset.

One change: **records go first.** Clear `repositories/_commits` before the rest
of `repositories`; inside a Library repository `_commits/` already sorts before
`_manifests/` and `_objects/`. Names sort by depth, so a stream's first
publications are deleted first. Once they are gone every other record is
disconnected from the root and the stream selects nothing, while every manifest
and object still exists. An interrupted clear therefore never leaves a selected
record pointing at deleted data. A writer racing the clear whose parent is
already gone is stopped by list-then-write, or leaves an orphan the next pass
deletes; the loop already repeats until a listing is empty. An orphan that
lands after the last pass stays in the storage and is never selected. It does
not close the stream: a first publication needs a stream that selects nothing,
not an empty listing (section 7.2).

**Claims go before records.** A Push writes its claim before its record, so
the clear removes them in the opposite order: `repositories/_claims` first,
then `repositories/_commits`, then the rest of `repositories`; inside a Library
repository `_claims/` already sorts before `_commits/`. An interrupted clear
then leaves records without claims. For the tip that is harmless: nobody has
claimed it, and the next Push does. For a parent that already has a child it
is not: a delayed writer that listed that parent childless before its child
was visible, and paused since (section 7.2), can claim it again, write an
earlier-stamped sibling, and be confirmed; the child's computer reads
`Displaced` at its next refresh (section 9.4). So the claim's guarantee holds
while the storage's claims and records are intact, and an interrupted clear
reopens the local store's window on a store with claims until the clear is
retried, which an interrupted clear asks for. No retry and no rollback prevents
that race; recovery is the user's, as everywhere (decision 12). An interrupted
clear does not leave a claim without its record, which would make the next
first Push finish a version whose data the clear deleted, unless a Push raced
the clear and the clear was also interrupted before its `repositories` pass;
the retry sweeps that claim first. A claim that lands during a clear that
finishes is swept by the `repositories` pass, which follows the records pass
and loops until its listing is empty; one that lands after that pass belongs
to a record that lands after it too.

**What a clear does to the claim's guarantee.** The claim guarantees one child
per parent among the records the storage holds; it says nothing about a parent
the clear removed. A Push that claimed its parent before the clear and wrote
its record after it lands an orphan: the parent is gone, nothing selects the
record, and the listing after the write reports `RemoteChanged` with nothing
local moved. When that listing came before the clear reached the record, the
Push was confirmed and its records are stale afterwards, which is R5 of
`PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` and ends in story 1 or 2. This is why the
listing after the write stays on a store with claims (section 7.2). After the
clear the guarantee starts again with the next first version: its claim of
`root` is fresh, because the claims pass ran first, and an orphan the clear
left behind has no claim and takes no child, because its parent is not listed.

One case remains: a first publication written in the one-request gap right after
a clear finishes can reference objects the clear deleted. That stream reads
`Unavailable` until it is cleared again. Its claim landed in the same gap. The
clear dialog already says to stop sync on all machines first.

## 13. What the user sees

Losing records stay in the storage. Nothing lists them, restores them, or
counts them in the UI in this release. The Push that wrote one failed, so the
losing computer accepted nothing and still holds the same content locally: it
compares again and pushes again. An archive browser can be added later without
any storage change, because nothing is deleted.

The only user-visible changes are five texts in existing surfaces. The notice
component prints the severity word itself, so the copy never names it:

| When | Copy |
|---|---|
| `RemoteChanged` | "The storage changed while pushing. Compare again." in the shell banner under the path bar while the Push review is open. It replaces the CAS-conflict message. It covers every way another computer can get in first: the listing before the write shows its record, its claim is already there, or, on the local store, the listing after the write selects it |
| `OutcomeUnknown` | "Couldn't confirm the push. Refresh to see whether it arrived." in the shell banner under the path bar while the Push review is open |
| `Displaced`: a confirmed Push, or a Pull, was outranked afterwards (section 9.4) | "Another computer pushed to <storage> first. This computer's last push is no longer part of its history. Compare again.", or "last pull" when the newest stale base was saved by a Pull. A standing note in the project activity notice zone, without an action, while the comparison reports `Displaced`. It replaces the one-time shell notice and its window event |
| `PushUnfinished` | "A push to <storage> did not finish. Finish it, then compare again." in the shell banner under the path bar while the Push review is open, with the action *Finish that push*. When the unfinished push is this computer's own: "This computer's earlier push to <storage> did not finish. Finish it, then compare again." The action writes the record the claim holds, once, lists, and the banner then reads "Finished. Compare again.", or the `RemoteChanged` or `OutcomeUnknown` sentence. The Library Push review has the same banner and action |

A claim taken by a record the listing holds fails with the `RemoteChanged`
sentence; one taken by a record that is not listed has its own row above.

There is no "stored, not selected" outcome: the Push result and the Library
push outcome keep the shape they had before this plan, and gain the
`PushUnfinished` failure. No notice rides a window event: the
`project-sync-push-outranked` event, its forwarder and the shell notice go
with the rollback (section 9.4).

The `Advanced` and `Unreviewed` banners keep their copy and their gates.
"Version N" keeps printing the revision. `Replaced`, `RemoteMissing` and
`Displaced` gate nothing. While the comparison reports one of them, the project
activity notice zone carries one standing note without an action: "Another
computer cleared <storage>. Push again." while Push is open, "Another computer
cleared <storage>. Pull, then push again." when a Pull comes first, and the
`Displaced` sentence of the table above. The Pull review shows no note about
any of them. `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` holds every case.

## 14. Mallard Cloud

Landed on September 23, 2026 as `PLAN_MANAGED_CLOUD_CAS_FREE.md`. Mallard
Cloud is the R2 store's twin: the service is a gateway onto one R2 prefix per
repository laid out exactly like every other Mallard storage (marker,
objects, manifests, records, claims, plus session views), reached through
three store routes, `PUT`, `GET` and a prefix listing (`STORE_API.md` in the
service tree). The managed store maps each engine key to the same key under
the prefix, sends the same requests in the same order, and reports
`supports_claims`: a claim is R2's own create-only write, made by the Worker
in the same request, and the Worker stores a record only when the claim of
its parent holds exactly those bytes. Nothing arbitrates in D1: the index is
written after the R2 write it describes, and the desktop never reads it, so
a Pull applies the version it reviewed on Mallard Cloud exactly as on R2.
Discovery reads the repository list instead of the all-streams listing, and
the Library publishes one item per Cloud repository on its own stream.

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
| 9. Claims | `supports_claims` and `put_claim` on the trait with their defaults, forwarded by the shared wrapper and the test wrappers; `put_claim` on S3/R2 and Dropbox; the plain test store's claim, exclusive in memory, switchable off, with a fault before the claim request; in the shared module: the claim key of a stream, the record name read from record bytes, the claim step before the record request, the two outcomes of a taken claim and the own-record case; the listing after the write stays as it is; the claims pass in clear; the probe stage; `CURRENT_METADATA_LAYOUT.md`, the README and the agent guide | The rows of section 16 marked *claim* pass on the plain store with claims and on the S3 and Dropbox fakes; request counts read listing, claim, record, listing on a store with claims and today's three on the local store; the live two-machine pass on R2 and on Dropbox shows one child per version after simultaneous Pushes, and one `_claims` object per version |
| 10. Recovery is a user action | Decision 12, the publishing part. In the shared module: one claim request and one record request, no `MAX_RECORD_REQUESTS` loop; the read-only checks after an unacknowledged claim or record, which fail the Push with the store's error when the request did not land and report `OutcomeUnknown` only when the check itself cannot be made; `PushUnfinished`. The *Finish that push* command and banner action in both Push reviews. The `PushUnfinished` copy of section 13 in the static specimen first. The rest of decision 12 needed no claims and landed on September 22 as steps 11 to 13 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`: the usability rule on the two base readers that open a manifest, `RemoteContinuity::Displaced` and its standing notice in place of the rollback, and the Library read rule in place of the reset at load | The rows of section 16 marked *no recovery* pass, except "Displaced after the Push was confirmed", "Stale acceptance cannot bypass review" and "Library: accepted head off the chain", which that plan's steps 11 to 13 carried: at most one claim and one record request per Push in every outcome; the finish action writes once and only when invoked; the frontend suite asserts the `PushUnfinished` texts of section 13 |

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
| Earlier-ranked sibling inside the fork window | The later one is kept and not selected, and its Push fails with `RemoteChanged`; local files and sessions are untouched |
| Writer sleeps between building a record and writing it | On wake it lists, sees a child, and ends with `RemoteChanged`; nothing is written |
| Writer pauses after the freshness check, store without claims | On the plain store: A lists and passes the check; the hook that fires before its record request advances the fake wall clock by an hour and lands B's later-stamped record with a child built on it, confirmed on B's side. A's send goes through, A's earlier-stamped record is selected, and B's branch is displaced. B's next refresh reads `Displaced` and changes nothing; B compares and pushes again by hand, and so does the third computer on B's branch. The test states what the window does not bound and that nothing recovers by itself |
| Listing older than ten wall-clock seconds | The attempt lists again; a monotonic-only check is shown not to catch a simulated sleep |
| Record write fails in the transport (no recovery) | One request, no automatic retry; the Push fails with the store's error; nothing local moves; on a store with claims the claim stays. A second Push is the user's, and with the same content it writes the claimed record |
| App restarts with a Push in flight (no recovery) | Nothing is resent; no pending record or claim exists on disk; a claim it left in the storage is met by the next Push through the claim alone |
| Acknowledgment lost (no recovery) | One read-only listing: present and selected is `Stored`, present and not selected `RemoteChanged`, absent fails the Push with the store's error, and a listing that cannot be made is `OutcomeUnknown`; no second request; a record that lands later is found by the next refresh |
| Crash after each publish step | Objects and a manifest without a record never appear as history |
| No-op while the remote changes | Nothing is written; the compared publication is returned |
| Written, not selected | `RemoteChanged` from `write_record`, and again from a repeat, which sends nothing. No base, checkpoint, binding, receipt or Library accepted state moves; the next refresh reads `Advanced`; the next Push publishes the same sessions; Library rows are offered again. A registration returns the established one |
| Displaced after the Push was confirmed (no recovery) | The next refresh reads `Displaced`. The link's saved state is byte-identical before and after: no base moved, no acceptance forgotten, no event, no write; the outranked record stays. The comparison reads the displaced areas `Unreviewed` and the untouched ones `InSync`, the sessions by adoption, Push `RequiresPull`; the Pull review reads the displaced files "keeps local copy"; left unticked, the Pull marks the area reviewed and the next Push publishes them under the same cloud ids. The standing notice shows, and goes away after that Push. The Library link is read as absent and not reset |
| Stale acceptance cannot bypass review (no recovery) | With a displaced or a cleared base: the session review never reports a session synchronized from the stale acceptance, S2 and S3 come from the content proof and S4 from `Divergent`; the Push gate reads the displaced areas `Unreviewed` and returns `RequiresPull`; a Pull review built while displaced reads a file this computer changed on the displaced branch "keeps local copy", never a plain replace, although its base manifest still exists; the files review's per-file states read it as diverged, not storage-ahead |
| A base an interrupted clear left behind | Listed, not selected, and not connected to the chain: nothing is rolled back, no event, the link reads `Replaced`, and the base is read as absent |
| Refresh, nothing changed | Exactly one listing request |
| Cold start | Listing, selected record, manifest; nothing was persisted |
| Remote ahead of the base | `Advanced` from the ancestor walk; no probe read |
| Continuity rows | One fixture per row of section 9.1, including a sibling branch with a greater revision (`Replaced`, never `Advanced`), a cleared stream that is pushed again, and one scope fixture per row A1 to A7 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md` |
| Push gate | Disjoint scope advanced: published on the new selection with that scope carried byte for byte. Same scope changed: `RequiresPull`. Stream cleared: `Ready`, and the Push writes the first record. Stream recreated: `Ready` or `RequiresPull` by its writable scopes, never an unlink |
| Library apply after a remote change | Any other selected publication than the reviewed one stops the apply with "changed after review"; nothing is written |
| Pull apply | A different selected publication than the reviewed one requires another review |
| Failed page or expired cursor | Fails the refresh; never an empty stream |
| Foreign names | Ignored and logged on all three stores; the listing succeeds |
| Local record listing | `list_keys` lists a stream whose record files cannot be opened, which proves it reads no record bytes; refresh, list-then-write and discovery call `list_keys`, never `list` |
| `BundleHead` view | Equals the selected record; readers of `snapshot.head` are untouched |
| Bundle discovery | The `_commits/` listing plus the tip records reproduce today's summary rows; no payload key is listed |
| Marker | Two first writers converge on identical bytes |
| Remote writes | No `_head.json`, no `_tags/`, no flag or moved object is ever written |
| Clear | Records go first; an interrupted clear leaves streams that select nothing; a writer racing it on a parent that is gone ends with `RemoteChanged`, and the stream still takes a first version afterwards |
| Library double Apply | The lock stops the second; one record is written |
| Setup probes | R2 and Dropbox setup pass without any conditional-overwrite stage |
| Provider parity | The same record set selects the same mainline on Local, S3/R2 and Dropbox |
| Large history | Listing cost is measured; a budget reports an incomplete view and never truncates silently |
| Claim: two writers, same parent | The second claim is refused in one request; its record is never written; that Push fails with `RemoteChanged`; the listing holds one child of the parent, on the S3 fake, the Dropbox mock and the plain store with claims |
| Claim: two first versions | One `root` claim, one first version; the other Push fails before it writes |
| Claim: the writer dies between its claim and its record (no recovery) | The next Push onto that parent, from another computer, writes nothing and fails with `PushUnfinished`; one claim request, no record request. *Finish that push* writes the claimed record once, lists, and reports Finished; the next comparison shows that version with the dead writer's commit id and origin. The dead writer then reads `Advanced` with its own content; a session it continued reads "Keep both" and a first-published one appears twice (L3) |
| Claim: the same computer, after its claim landed and its record did not | Same content on the same parent: the next Push writes the claimed record as its own, one record request, `Stored`, and its local save is this Push's; no L3. Other content: `PushUnfinished`, nothing written; *Finish that push* writes the earlier record, and the sessions of that earlier push are L3 |
| Claim: the claim request is not acknowledged (no recovery) | One read of the claim key and no second claim request: this computer's bytes and the Push goes on; other bytes and it is taken; absent and the Push fails with the store's error; a read that cannot be made ends it with `OutcomeUnknown` |
| Claim: the record request fails after the claim (no recovery) | One record request; the Push fails with the store's error; the claim stays; nothing local moves. The next Push with the same content writes the claimed record as its own |
| Claim: request count | On a store with claims a Push makes one listing, one claim, one record request and one listing, and in no outcome more than one claim or one record request; the local store makes today's three |
| Claim: the storage is cleared between the claim and the record | A lists, claims its parent, and the plain store is cleared before A's record request. A's record lands as an orphan; the listing after the write does not select it; A fails with `RemoteChanged`; no base, checkpoint, acceptance or receipt moves; A's next refresh reads the cleared storage and story 1 or 2 follows. A second variant clears after A's confirming listing: A was confirmed, its records read as stale, R5 |
| Claim: a stale listing | A listing older than the sibling's claim ends in a taken claim, never in a second child; the fresh listing the taken path takes holds the sibling's record, so the Push fails with `RemoteChanged`, not `PushUnfinished` |
| Claim: not a record, or another parent's record | The Push onto that parent fails with an integrity error, not `RemoteChanged`; nothing is written; selection does not move |
| Claim: never read by a refresh | Refresh, review, the Pull apply and bundle discovery make no request under `_claims/` |
| Claim: the local store | Writes none; its Push is unchanged; the fork-window tests stay on the plain store without claims and still pass |
| Claim: clear | `_claims` goes before `_commits`; an interrupted clear after the claims pass leaves records without claims and the next Push claims the tip; a claim that lands during the clear is swept by the `repositories` pass; after a clear `root` can be claimed again |
| Claim: interrupted clear, then a delayed writer (no recovery) | The clear is interrupted after its claims pass with the chain intact. A writer that listed the tip's parent childless before the tip was visible claims it, writes an earlier-stamped sibling, and is `Stored`. The tip's computer reads `Displaced` at its next refresh, nothing is rolled back, and it compares and pushes again. The clear's retry sweeps the stale claims first |
| Library: accepted head off the chain (no recovery) | The page loads and writes nothing; every row compares as unreviewed and goes through the Pull choice; a Push saves a new accepted state. One fixture for a cleared repository, one for a displaced head, and one for an accepted head that names another storage identity or another repository |
| Claim: setup probe | The S3 and Dropbox probes refuse a destination that overwrites a key |
| Claim: Library | The same rows on the resource engine; a taken claim ends before the accepted state is saved |

Existing transcript, owned-subagent, scope-firewall, deletion, secret-handling,
destination-isolation, key-validation and frontend review regressions run
unchanged.

Release condition:

> A store that offers only put, get and list, with plain overwrite and no
> conditional write, runs project and Library publication, refresh, review and
> restore. The existing stores run that same engine. A store that also refuses
> a second write of a key runs the claim on top of it, and a version listed
> there never loses while its claims are intact. No operation retries a write,
> resends after a restart, or changes sync metadata without the user.

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
- A claim on the local store, or any test of a folder's exclusivity: a folder a
  sync product shares would take two claims, and a network share promises no
  create-only write. Its fork window stays.
- Deleting or reassigning a claim. Like a losing record, a claim leaves the
  storage only with a clear; an unfinished push is finished by the user or left
  as it is.
- Recreating the local save of a push whose record arrived without it, L3 of
  `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`. Its own plan.
- A storage generation, an epoch, or any locking protocol. Recovery is a user
  action, and the storage layout stays.

## Implementation notes

Steps 9 and 10 were implemented on September 22, 2026. What differs from
the plan above, or was decided while building them:

| Subject | What was built |
|---|---|
| A taken claim and a stale listing | The listing of step 1 can predate the sibling's record when both Pushes ran in the same seconds. Before a taken claim whose record that listing does not hold is reported `PushUnfinished`, the module lists once more; a record that fresh listing holds is another computer's finished push, `RemoteChanged`. Sections 7.2 and 16 say so |
| An unacknowledged request | The read-only check settles it, on both sides: present counts, absent did not land, and the Push fails with the store's error rather than `OutcomeUnknown`, which is reported only when the check itself cannot be made. S3/R2, Dropbox and the local store list their own completed writes, so absent is evidence, and the store's error tells the user why the Push failed. Sections 7.3 and 16 say so. The record half of this rule lands with step 10, which removes the retry loop |
| The shared module's shape | `write_record` returns `WriteOutcome`: `Stored` with the listing that selects the record, or `Unfinished` with the claimed record's name and bytes. The engine decides the own-record case, because only it can read the claimed record's manifest and compare content: the bundle engine compares the scope digests, the Library engine the per-resource content digests, and both compare the origin's replica token with the one this Push writes. An own record with the same content is written through `write_claimed_record`; anything else fails with `PUSH_UNFINISHED` or `OWN_PUSH_UNFINISHED`, engine-level texts that the two services turn into the sentences of section 13 with the storage's name |
| The claim's parent check | A claim whose bytes are not a record of the claimed parent, by the five graph fields and the digest of the bytes, is an integrity error for that parent, as section 7.2 says; the record name is derived by the shared module from those fields, for both record types, without either engine's record type |
| One request each | The write lists, again while its listing is stale, claims once, sends once, and lists once to settle; the record loop and its constant are gone. A request the store does not acknowledge is settled by one read-only check, the claim key after a claim and a listing after a record: what it shows landed counts, what it shows absent did not land and the Push fails with the store's error, and a check that cannot be made is `OutcomeUnknown`. The next Push is the user's |
| Library: the last two refusals | `verify_library_continuity` no longer refuses an accepted head whose storage identity or repository id is not this link's. Both describe a history this storage does not hold, so both read `Replaced`: absent, nothing written, every row compares as unreviewed, and the next Push or Pull saves a fresh accepted state under this storage's identity. The refusals had left the page unable to load, and with it Unlink disabled, the row's deletion and the storage's removal refused while the link existed: the one remaining state that ended at a wipe of `~/.mallard`. Only the storage identity can differ in practice, from a hand edit of the storage configuration, a local folder configured through a symlink whose target moved, or a build that changes a derivation; a head that names another repository never reaches the page, because the links document refuses it at load and at save, so that half of the classifier is a second line that does not fire. Section 11 and B3 of the cleared plan say so |
| *Finish that push* | One shared verb, `finish_claimed_push`: list, read the claim of the selected version, and write the record it holds through the same record path, once. A claim that is absent, or whose record a listing already holds, is `RemoteChanged` for the verb: the storage changed since the review that reported the unfinished push. A store without claims refuses the verb with its own sentence, and no banner offers it there. Both engines and both services expose it, under the same host or per-repository lock a Push takes; the Push review's banner and the Library page's failure zone offer it only while the error is a sentence of section 13 for that storage. A finished push reads "Finished. Compare again." in the done tone in the Push review, because the notice prints its severity word, and as the shell notice on the Library page |

Still open: the live two-machine pass on R2 and on Dropbox, with the two live
rows of step 9 (one child per version after simultaneous Pushes, one
`_claims` object per version).

## Source pointers

Line numbers are from the working tree on September 18, 2026; the rows under
*Claims* from September 22, 2026.

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

*Claims*

Line numbers of the working tree on September 22, 2026, after step 9.

| Subject | Location |
|---|---|
| `write_record` and `WriteOutcome`; `ClaimedRecord`; the claim step inside `attempt`; `claim_once`; `taken_claim`, with the fresh listing before an unfinished push is reported; `write_claimed_record`; `record_name_of` | `crates/mallard-infrastructure/src/storage_core/publication.rs:164`, `:157`, `:704`, `:597`, `:637`, `:560`, `:203` |
| `RecordStream::claim_key`; the engine-level texts `PUSH_UNFINISHED` and `OWN_PUSH_UNFINISHED`; `unfinished_push_sentence`, the Push review's sentence with the storage's name | `crates/mallard-infrastructure/src/storage_core/publication.rs:141`, `:60`, `:66` |
| Trait: `ClaimOutcome`, `supports_claims`, `put_claim`; the shared wrapper's forwards; `REPOSITORY_CLAIMS_PREFIX`; the Library key allowlist; the clear's claims pass | `crates/mallard-infrastructure/src/storage_core/object_store.rs:162`, `:284`, `:292`; `storage_core/mod.rs:513`, `:58`, `:291`, `:206` |
| S3/R2: `put_claim_async`, the create-only put with retries off and one read on a refused precondition; the probe stage `Refuse`; the fake's overwrite knob | `crates/mallard-infrastructure/src/storage_core/s3_store.rs:750`, `:392`, `:2454` |
| Dropbox: `put_claim_async`, add mode without a strict conflict check and one download on a conflict; the probe's refusal; the mock's overwrite knob | `crates/mallard-infrastructure/src/storage_core/dropbox_store/upload.rs:172`; `dropbox_store/probe.rs:134`; `dropbox_store/mock.rs:36` |
| The plain test store's claim, exclusive in memory and switchable, its faults, the hook before the claim request, its request log | `crates/mallard-infrastructure/src/storage_core/plain_store.rs:72`, `:122`, `:130`, `:91`, `:282` |
| The bundle engine's own-record decision; the Library engine's | `crates/mallard-infrastructure/src/bundle_engine.rs:867`; `src-tauri/src/storage_catalog/resource_engine.rs:496` |
| The two services print the sentence with the storage's name | `src-tauri/src/project_sync/service.rs:10481`; `src-tauri/src/storage_catalog/service.rs:1885` |
| Tests: the shared module's claim rows; the clear order; the bundle engine; the Library engine; the S3 fake; the Dropbox mock; the stub S3 end to end | `crates/mallard-infrastructure/src/storage_core/publication.rs:1347-1621`; `storage_core/mod.rs:832`; `bundle_engine.rs:5044`; `src-tauri/src/storage_catalog/resource_engine.rs:1125`; `s3_store.rs:2213`, `:2229`; `dropbox_store/tests.rs:584`, `:624`; `src-tauri/src/sync_tests/thread_variants/storage_parity.rs:540` |

*Recovery is a user action*

| Subject | Location |
|---|---|
| `observe_linked_bundle`, the `Displaced` classifier that writes nothing; the rollback, its event, forwarder and app hook were removed on September 22 (step 12 of `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`) | `src-tauri/src/project_sync/service.rs:7122` |
| The three rollback tests to rewrite as `Displaced` tests | `src-tauri/src/project_sync/service.rs:20600`, `:20762`, `:20837` |
| `Mainline::outranked`, which names the displaced record; `BundleEngine::published_by`, the one record read of a `Displaced` verdict | `crates/mallard-domain/src/publication.rs:297-331`; `crates/mallard-infrastructure/src/bundle_engine.rs:766` |
| The shell notice and its listener; the event type | `src/components/project-sync/syncOutcome.ts:69`; `src/components/project-sync/ProjectSync.tsx:1399-1402`; `src/types.ts:954` |
| The Library reset at load | `src-tauri/src/storage_catalog/service.rs:1050-1062` |
| Base readers that open the base manifest: the files review's per-file states; the Pull planner's per-file base digests; the preservation rule they feed | `src-tauri/src/project_sync/service.rs:2348`, `:2708-2742`; `crates/mallard-infrastructure/src/bundle_engine.rs:5034-5057` |
| Base readers that already filter, through `classify_scope_continuity` | `src-tauri/src/project_sync/service.rs:7728`, `:7851`; `crates/mallard-domain/src/lib.rs:931-956` |
| What marks an area reviewed after a Pull with unticked rows | `src-tauri/src/project_sync/service.rs:13472` |
| One claim request and one record request, each settled by one read-only check: `claim_once`, `send_once`, the `attempt` they belong to | `crates/mallard-infrastructure/src/storage_core/publication.rs:597`, `:678`, `:704` |
| *Finish that push*: the shared verb; the engines'; the services'; the commands; the frontend's sentences and `finishPushOffered`; the Push review's action and done tone; the Library page's action; the specimen | `crates/mallard-infrastructure/src/storage_core/publication.rs:752`; `crates/mallard-infrastructure/src/bundle_engine.rs:899`, `src-tauri/src/storage_catalog/resource_engine.rs:297`; `src-tauri/src/project_sync/service.rs:10113`, `src-tauri/src/storage_catalog/service.rs:1762`; `src-tauri/src/project_sync/commands.rs:1197`, `src-tauri/src/storage_catalog/resource_commands.rs:206`; `src/unfinishedPush.ts`; `src/components/project-sync/PushResourceWorkspace.tsx:1017`; `src/components/storage-catalog/StorageCatalogPage.tsx:1538`; `mockups/push-notices-live.tsx` |
| Tests: one request in every outcome; the unacknowledged claim; the verb; the engines and the stub S3 finishing another computer's push; the frontend's sentences and buttons | `crates/mallard-infrastructure/src/storage_core/publication.rs:1087`, `:1639`, `:1697`; `crates/mallard-infrastructure/src/bundle_engine.rs:5044`; `src-tauri/src/storage_catalog/resource_engine.rs:1125`; `src-tauri/src/sync_tests/thread_variants/storage_parity.rs:540`; `tests/frontend/push-outcome.integration.test.tsx`, `tests/frontend/resource-inventory.integration.test.tsx`, `tests/frontend/storage-catalog.integration.test.tsx` |
