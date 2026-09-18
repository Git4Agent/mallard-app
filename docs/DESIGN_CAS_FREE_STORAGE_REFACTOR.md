# Mallard: CAS-free storage model refactor

**Status:** Proposed design; implementation has not been performed  
**Scope:** Replace the mutable-head publication protocol in place, in the desktop app; not a provider-specific alternative  
**Compatibility:** Breaking cutover; no legacy readers, migration, fallback, or dual-write protocol  
**Versioning:** Layout and schema numbers remain `1`; numeric version equality does not imply compatibility  
**Reviewed baseline:** `Git4Agent/mallard-app@460edb67a3db63f1960249ad06a5955c5aec3ab0`  
**Supersedes:** `DESIGN_APPEND_ONLY_MAINLINE_ARCHIVE.md` for implementation scope and architecture  
**Revision 3 (September 18, 2026):** the owner's decisions after `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md`:

1. The desktop app is cut over first. Managed Cloud follows in stage 5 (sections 12.1 and 16).
2. Late-written records are limited by one writer rule, *list, then write* (section 8.3). The selector is unchanged.
3. Refresh is one listing of one stream and compares the selected publication with the one already held. Push rebases onto the selected publication at apply time (sections 8.1, 9 and 10.3).
4. Forks, pins, and continuity are decided by a publication's identity and ancestry, never by revision order or by comparing snapshots (sections 6.3 and 10).
5. There is no storage UID (section 4.1).
6. **The smaller cut.** The key layout, content-addressed objects, both portable manifests, every local pin shape, and the `ObjectStore` trait stay. The commit record that every publish already writes becomes the publication point, and the mutable head and the tag index are deleted. Opaque locators, a snapshot envelope, a payload table, and an `AppendStore` trait are not part of this design (section 18).

> Providers save immutable bytes and make publications discoverable. Mallard—not the provider—derives the current mainline. Concurrent publications may both be stored; the later-ranked sibling and its descendants are retained as concurrent archives.

This document keeps the previously chosen earliest-sibling policy and applies it to the existing model, the existing key layout, and the existing providers. Feishu implementation, authentication, API research, and provider-specific fields are out of scope. Types marked as proposed are design sketches, not compiled implementations.

## 1. Decision

Replace the shared mutable-HEAD protocol with a single append-only publication protocol. Local-folder, S3/R2, Dropbox, and Managed Cloud use the same publication semantics.

A provider needs three operations for ordinary sync: write bytes at a key, read a key, and list a prefix. It does not need CAS, atomic create-if-absent, exclusive filename creation, or a lock service. Every key Mallard writes is content-addressed or unique by construction, so an overwrite can only rewrite identical bytes (section 5.3).

The protocol retains complete snapshots and one parent per publication. At each fork, select the child with the smallest `(created_at_ms, commit_id)` and continue down that child. Other subtrees remain stored but are excluded from the default remote view.

This is deliberately **not** a single-winner commit protocol. An upload acknowledgment confirms storage, not permanent admission to the mainline. Selection can change when an earlier-ranked sibling appears, but writers only ever add the first child of the publication they just listed as selected, so a sibling can appear only inside a short fork window (section 8.3). Older history is final.

| Decision | Required outcome |
|---|---|
| One protocol | No `supports_cas` switch, strong/weak backend modes, or second engine |
| Publication unit | Complete project bundle snapshot; complete existing Library repository set |
| Publication point | The commit record under `_commits/`, written last. There is no `_head.json` and no `_tags/` |
| Concurrency scope | Independently per project bundle or Library repository, never storage-wide |
| Arbitration | Deterministic earliest sibling, computed in shared code from listing names |
| Late-written records | *List, then write*: every write attempt of a record follows a fresh listing that shows its parent selected and childless. No pending record is kept after the operation ends |
| Identity | `commit_id` identifies a publication. `revision` is its depth and is only printed. Local pins keep their shape |
| Refresh | One listing of one stream. An unchanged selected publication means nothing else is read |
| Losing branch | Stored, classified as concurrent archive, accessible for recovery |
| Provider contract | Put at a key, get a key, list a prefix. `compare_and_swap` is removed, and `put_immutable` no longer promises no-overwrite |
| Storage identity | The provider's destination, as today. No storage UID |
| Local application | Explicit Review → Apply; no automatic rollback after remote reselection |
| Cutover | New marker `format` at the same key and a replacement record shape, all at version `1`; nothing is imported |
| Order of work | Desktop app first: both engines, Local, S3/R2 and Dropbox, and the UI. The Managed Cloud service, website, and adapter follow in stage 5 |
| First release excludes | Resource-level merging, CRDT text editing, automatic remote GC, opaque locators and identifier-addressed providers, Feishu integration |

This replaces the old CAS safety mechanism rather than silently disabling it. The replacement safety properties are immutable preservation, exact identity and digest checks, deterministic selection, the list-then-write limit, explicit archives, and unchanged local review protections.

## 2. What the current code actually depends on

These are current-code observations; the right column is the proposed change.

| Current dependency | Refactor |
|---|---|
| `ObjectStore` requires `compare_and_swap`; its common types carry ETags. [S2] | Remove `compare_and_swap`, `CasExpectation`, and `CasOutcome`. Keep keys, `get_metadata`, `stage_verified`, `put_immutable`, `put_many`, `list`, and the clear path. ETags remain only where clear deletes what it listed |
| `bundle_engine.rs` has `HeadToken`, `PublishExpectation`, `expected_head`, and a `PublishedBundle { published, snapshot }` result. [S3] | The observation names the parent commit instead of a head ETag. The final step is list-then-write of the commit record. The result separates the storage outcome from the derived role |
| `_head.json` is a stored object advanced by CAS, and `_tags/<bundle_id>.json` is a derived index maintained by a CAS retry loop. [S3] | Both are deleted. `BundleHead` remains as the in-memory view of the selected commit record, and the tag's facts move into that record |
| `_commits/<revision>-<commit_id>.json` is written on every publish and never read. [S3] | It becomes the publication record, gains four fields, and is written last |
| Bundle payload readers address `_objects/<sha256>` through `entry.object_key`. [S3] | Unchanged |
| The Library resource engine reads one HEAD and one full-set manifest, checks the observed ETag, and rejects numeric revision rewind. [S4] | The same mechanics through one shared module; ancestry replaces the rewind check |
| The Library is a separate feature and domain; its publication unit is the complete repository set for a coordinate. [S4][S5] | Preserve that boundary and granularity; do not import project models into Library |
| Storage initialization uses a fixed marker and an immutable-put/read-back path. [S6] | Keep the fixed key and idempotent creation. The marker's bytes are constant, so concurrent first writers write identical bytes. Only its `format` changes |
| Provider registration already centralizes configuration, secrets, discovery, and store construction. [S7][S8] | Keep all of it; the probes lose their CAS stages |
| Managed Cloud synthesizes object-store keys and makes HEAD CAS the upload commit. It refuses Library namespace writes in that adapter. [S9] | Stage 5 (section 12.1) |

The repository guide explicitly favors removing obsolete paths over compatibility layers, while preserving immutable SHA-256-verified publication and explicit local restore approval. Its mutable-HEAD-specific wording must be updated as part of this change. [S1]

## 3. Architecture and ownership

```text
ProjectSync application                 Library application
  review and capture                      review and capture
          |                                       |
    Bundle engine                       Library resource engine
          +-------------------+-------------------+
                              |
                  shared publication module
      name grammar / selector / continuity / list-then-write
                              |
                         ObjectStore
                get / put / list, no compare-and-swap
                              |
          Local      S3/R2      Dropbox      Managed Cloud (stage 5)
```

The **domain layer** owns record validation, the name grammar, the pure selector, and continuity by identity and ancestry. It has no filesystem, HTTP, provider-key branching, or local clock calls during selection.

The **shared publication module** owns the list-then-write step, head selection over a listing, and the in-process cache of records it has read. It does not understand transcripts, skills, pets, or provider-home paths, and it imports neither feature's types.

The **feature engines** continue to build and validate their existing complete manifests, preserve untouched scopes or entries, and prepare review-approved application plans. They share publication mechanics, not domain DTOs.

The **store** owns physical addressing, credentials, upload mechanics, and pagination. It neither chooses the winning branch nor invents a provider-specific definition of publication success.

Use the existing crates, the existing `ObjectStore` trait, and the existing provider registry. A new transport trait, a plugin framework, an event bus, a metadata server, and a protocol capability matrix are unnecessary.

## 4. Storage identity and streams

### 4.1 Storage identity is the destination; there is no UID

A storage is identified by its provider destination, as it is today: the R2 jurisdiction, account and bucket; the Dropbox namespace id; the canonical local directory; the Managed Cloud origin and account. `continuity_coordinates` already returns these, `ensure_destination_available` already refuses a second storage at the same destination, and the Library already pins a hash of them. An identifier-addressed provider would use its folder id.

There is no storage UID, no UID-qualified namespace, and no local root binding. A Mallard-minted id would duplicate the provider's own identity, and it would cost a join flow: a second device could not compute the root path, would have to list roots and pick one, and two simultaneous Creates would split silently into two roots. A recreated storage is detected per stream by publication identity instead (section 10.2): a base from the old contents is never listed in the new ones. `DESIGN_REMOTE_HISTORY_CONTINUITY.md` deferred a "repository epoch nonce" for the same reason.

The marker stays at `.mallard/_storage.json`:

```json
{ "format": "mallard-append-storage", "layout_version": 1 }
```

The bytes are constant, so concurrent first writers write identical bytes and creation needs no exclusivity. Only `format` changes from today's `mallard-storage`. Keeping the key matters: an older build reads the marker and refuses the format, whereas a marker at a new key would leave an older build to find none and create an old-layout storage beside the new one, because `ensure_storage_marker` creates on first open. **Keep `layout_version` at `1`; do not use the number alone to identify the protocol.** The format fixes the selection policy; it is not a runtime strategy setting.

The discovery backend (one fixed-key marker read per bucket or shared folder) and the rule that one destination is one storage keep working; `classify_storage_marker` needs only the new format constant.

### 4.2 Stream identity

A storage contains independent streams, one per publication unit:

```text
project stream    bundle <bundle_id>              records under  .mallard/v1/repositories/_commits/<bundle_id>/
Library stream    repository <repository_id>      records under  .mallard/v1/library/<repository_id>/_commits/
```

A project stream publishes an entire bundle. A Library stream publishes the entire set already owned by that Library repository coordinate. Different streams never compete with one another. The shared module receives a record prefix; it knows nothing else about the feature.

### 4.3 Virtual genesis

Each stream has an implicit empty root, represented by a record with no `previous_commit_id`. No client uploads or races to create a special genesis file. Two first publications are simply siblings under that virtual root and follow the ordinary selection rule.

Registering an empty project, when needed by the existing product flow, creates an ordinary publication containing the existing empty registration manifest. It does not reserve a unique filename or require a separate HEAD.

### 4.4 Concrete physical layout

```text
.mallard/
  _storage.json                                   marker, constant bytes
  v1/
    repositories/
      _commits/                                   every project stream's records, and nothing else
        <bundle_id>/
          0000000001.commit-h0….root.1789635600000.json            H0
          0000000002.commit-xa….commit-h0….1789635601000.json      XA
          0000000002.commit-xb….commit-h0….1789635602000.json      XB
          0000000003.commit-xa2….commit-xa….1789635605000.json     XA2
      <bundle_id>/
        _manifests/<revision>-<commit_id>.json    unchanged
        _objects/<sha256>                         unchanged
    library/
      <repository_id>/
        _tag.json                                 unchanged
        _commits/<the same name grammar>
        _manifests/<revision>-<commit_id>.json    unchanged
        _objects/<sha256>                         unchanged
```

Project records live in one flat namespace beside the bundle folders, in the position `_tags/` holds today. Listing `_commits/<bundle_id>/` returns one stream, which is what a refresh needs. Listing `_commits/` returns every project stream's records and no payload object, which is what bundle discovery needs. A bundle id is 32 hexadecimal characters, so it cannot collide with `_commits`. Library records stay inside each repository: the Library never enumerates repositories, because their ids are derived from the registered coordinates.

**A record's name carries its graph facts:**

```text
<revision, 10 digits>.<commit_id>.<previous_commit_id or "root">.<created_at_ms, 13 digits>.json
```

- Shared code builds and parses the name, as it builds every key today. Selecting a head reads names, never records.
- Names sort by depth, so a later optimization can list only the tail with the cursor the stores already have.
- A name is lowercase `[a-z0-9.-]` and 93 bytes, which every current provider accepts.
- A name outside the grammar (`.DS_Store`, a sync client's conflicted copy) is ignored and reported, not treated as a record.
- The record stays the authority: a record whose content disagrees with its name is invalid.

There is no `_head.json`, `_tags/`, `_index.json`, remotely maintained `archived` flag, or folder that must be exclusively created.

## 5. Provider contract

### 5.1 `ObjectStore`, minus compare-and-swap

The trait, its key types, and its blocking execution model stay. [S2]

| Member | After the cut |
|---|---|
| `get_metadata`, `get_metadata_many`, `stage_verified`, `stage_many_verified` | Unchanged |
| `put_immutable`, `put_many` | Same signatures. The contract no longer promises an atomic no-overwrite: it stores these bytes at this key, and section 5.3 explains why that is enough. An existing store may keep its native mechanics (S3 `If-None-Match: *`, Dropbox `add`, the local lock); a new provider may implement a plain write |
| `put_record` — proposed | One request with no transport retry, used only for the commit record (section 8.3). The default forwards to `put_immutable` |
| `compare_and_swap`, `CasExpectation`, `CasOutcome` | Removed |
| `list` | Same signature. It skips and reports a name it cannot represent instead of failing the whole listing. A completed write is visible to every later listing of that prefix, or the adapter states its worst-case lag |
| `delete`, `supports_conditional_delete`, `clear_managed_data` | Unchanged; only the clear order changes (section 13) |
| `StoredMetadata.etag`, `VerifiedBody.etag` | No engine reads them after the cut. `StoreListEntry.etag` remains for clear |

### 5.2 Exact semantic requirements

| Required property | Meaning |
|---|---|
| Whole-object writes | A reader sees an object completely or not at all. An overwrite with identical bytes is invisible |
| Stable retrieval | A key continues to name the same bytes while access and retention remain valid |
| Listing after write | A completed record write is returned by every later listing of its stream, or the adapter states its worst-case lag. Local, S3/R2, and D1 behave this way; the Dropbox setup probe confirms it (write, then the listing must show it) |
| Single-shot record write | `put_record` sends one request, and the client library's automatic retry is off for it |
| Integrity verification | Bytes can be checked against the declared size and digest before use |
| Key confinement | Keys cannot escape the Mallard namespaces; the existing `ObjectKey` and `ObjectPrefix` validation stays |

This is CAS-free, not consistency-free. A provider with no reliable whole-object writes, or whose listing lag has no stated bound, is not automatically supported.

A write error must distinguish a failure known to occur before the object was stored from an **unknown outcome**. Network timeouts do not prove the object was not saved.

### 5.3 What the common layer must not require

No `compare_and_swap`, `put_if_absent`, distributed mutex, conditional overwrite, server timestamp ordering, permanent idempotency key, or server-assigned global revision is required for ordinary sync.

Keys stay caller-chosen. That needs no exclusivity, because no conforming writer ever writes different bytes to the same key:

| Key | Why two writers cannot disagree |
|---|---|
| `_objects/<sha256>` | Content-addressed |
| `_manifests/<revision>-<commit_id>.json` | `commit_id` is 96 random bits, minted per publication |
| `_commits/…/<name>` | The name contains the `commit_id`; a retry inside one operation rewrites the same bytes |
| `.mallard/_storage.json` | Constant bytes |
| Library `_tag.json` | The coordinate it freezes is derived; drop its `created_at` so the bytes are constant too |

Batch transfer, provider checksums, tail listing, and deduplication remain optimizations. They must preserve the same observable semantics and cannot change the selection policy.

## 6. Data model

### 6.1 What does not change

- `BundleManifest` and `LibraryResourceManifest`, including `revision`, `commit_id`, and each file entry's `object_key`.
- `_objects/<sha256>` and `_manifests/<revision>-<commit_id>.json`.
- Every machine-local pin: `RepositoryBase`, `ScopeBase`, `MaterializationRecord`, `ConversationApplyReceipt`, `AcceptedConversationVersion`, `LibraryHeadCoordinate`.
- The IPC types that carry those coordinates (`RestorePlan`, `PullReview`, `ConversationRevisionPins`, `BundleSnapshot`, `PushDestinationStatus`) and their TypeScript mirrors.
- `BundleHead` and `LibraryResourceHead` as types. They stop being stored objects and become the in-memory view of the selected commit record, field for field. `BundleSnapshot { head, manifest }` and every caller that reads `snapshot.head.*` keep working.

### 6.2 The publication record

The record is today's `CommitRecord` with four additions: `manifest_size`, `previous_manifest_sha256`, `created_at_ms` (replacing `created_at`), and `summary`.

```json
{
  "schema_version": 1,
  "bundle_id": "0123456789abcdef0123456789abcdef",
  "revision": 2,
  "commit_id": "commit-aaaaaaaaaaaaaaaaaaaaaaaa",
  "previous_commit_id": "commit-bbbbbbbbbbbbbbbbbbbbbbbb",
  "previous_manifest_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
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

This example is XA. Hashes and sizes are illustrative. The record does not contain `is_current`, `is_archived`, `winner`, `accepted`, or `head_etag`. Those are derived observations or obsolete protocol concepts, not immutable publication facts.

- `previous_commit_id` and `previous_manifest_sha256` are absent only for a child of the virtual root. Together they name the parent by identity and by content. Reject cycles, self-parenting, a parent in another stream, duplicate fields, invalid identifiers, and unbounded structures.
- `revision` is `1` for a child of the virtual root and the parent's `revision + 1` otherwise, exactly as the engine computes it today. A record that breaks this is invalid.
- `created_at_ms` is UTC Unix milliseconds in `0..2^53-1`, so Rust and TypeScript compare it exactly. Freeze it near the final publication step. It is not transcript time, file mtime, provider metadata time, or first-observed time. It is a deterministic priority input, not proof of real-world arrival order. Do not reject otherwise valid records using each reader's current wall clock.
- `summary` holds the facts `BundleTag` carries today, so bundle discovery reads records and never manifests. It is always as fresh as the publication it belongs to; today's tag can lag, because a failed tag write is ignored.
- The name's four facts must equal the record's fields.

The record is built once per operation and held in memory. Every attempt inside that operation sends exactly the same bytes to the same key, and every attempt follows a fresh listing (section 8.3). The record is never written to local disk and never outlives the operation: after a crash, a restart, or a stop, the next Push reviews again and builds a new record. Changing any field requires a new publication and the appropriate review.

### 6.3 Publication identity

- **`commit_id` is the identity.** It is `commit-` plus 24 lowercase hexadecimal characters, 96 random bits, generated as it is today. Comparison for ranking is lexicographic over that fixed encoding.
- **`revision` is the depth.** It is the number people see ("Version 2"). Two siblings share a revision, so the number is never an identity and never an order between branches.
- **The coordinates local state pins are unchanged:** `(revision, commit_id, manifest_sha256, manifest_size)`, which is `RepositoryBase` as it exists today. They name the publication by `commit_id` and carry the digest and size needed to read its manifest with verification.
- **Rules.** Two coordinates are the same publication when their `commit_id` is equal; the digest must then agree, or the state is an integrity conflict. One publication follows another when a walk over `previous_commit_id` reaches it. Nothing orders by `revision`, and nothing compares `scope_revision`.

### 6.4 Duplicate and conflicting identity

On a key-addressed store a retry rewrites the same bytes under the same name, so duplicates do not occur. On an identifier-addressed store, several objects with the same name and bytes are one logical publication, and any verified copy may serve reads.

The same `commit_id` under two different names, or with different bytes, is an integrity conflict, not another candidate to rank. Stop mutation of the affected stream and report diagnostics. Never resolve it by listing order, provider mtime, or whichever copy was cached first.

Hashes provide integrity, not author authentication. This version relies on trusted writers within the storage ACL; defending against an authorized malicious writer who deliberately backdates records is not part of the protocol.

### 6.5 The Library uses the same record

`LibraryResourceCommit` gains the same three protocol fields (`manifest_size`, `previous_manifest_sha256`, `created_at_ms`) and is written last, under the same name grammar, inside its repository. It needs no `summary`, because nothing enumerates Library repositories. `_tag.json` keeps binding the derived repository id to a readable coordinate. No project DTOs are imported into the Library manifest. A Library publication competes only with siblings in its own repository.

### 6.6 Stored records versus computed roles

For the example in section 4.4:

| Record | Parent | Revision | Frozen example time | Derived role after all four records are listed |
|---|---|---|---|---|
| H0 | none | 1 | `t` | MainlineHistory |
| XA | H0 | 2 | `t + 1 second` | MainlineHistory |
| XB | H0 | 2 | `t + 2 seconds` | ConcurrentArchive |
| XA2 | XA | 3 | `t + 5 seconds` | SelectedHead |

XA and XB are both "Version 2". Roles, pins, and archive rows therefore name a publication by its `commit_id` and print the revision only as a label.

The stored records do not change when these roles are computed. Do not move XB to an archive folder, add `archived = true`, mark XA2 `current = true`, or write a remote HEAD. If XB later receives a child XB2, its whole losing subtree remains archived. Mainline selection and the fork window are specified in sections 7 and 8.3.

### 6.7 Remote metadata versus local state

| Information | Authoritative remote location | Local or derived treatment |
|---|---|---|
| Storage layout and fixed selection policy | The constant marker | Verified once per shared-store entry, as today |
| Commit id, parent, revision, timestamp, origin, manifest reference, summary | The immutable record; its graph facts are repeated in its name | Read only the records that are needed (the selected one, the one above a pinned base, the tips for discovery); cache them in process |
| Complete semantic state | The immutable manifest | Validate with the corresponding feature rules, as today |
| Conversation/resource payload bytes | Content-addressed objects | Stage and verify size/hash before use, as today |
| Selected head, mainline history, archive role | None | Computed from one listing of the stream; nothing is persisted |
| Listing results and read records | None | In-process cache only; a restart lists again |
| Upload progress, the pending record, unknown outcomes | None | In memory for the life of one operation; never on disk |
| Reviewed, accepted, and installed versions | None | The existing local records, unchanged in shape |
| Credentials, local paths, trust decisions, backups | None | Protected local state; never portable metadata |

Each non-no-op Push adds changed objects, **one manifest, and one record**. A no-op adds nothing. The head write, the tag read, and the tag write of today's Push disappear.

## 7. Pure mainline resolution

Selection runs over the names returned by one listing of a stream. Its inputs are the name facts (`revision`, `commit_id`, `previous_commit_id`, `created_at_ms`), so it needs no record reads. The record remains the authority: whenever a record is read, its content must equal its name. Continuity always verifies the link directly above a pinned base (section 10.2).

Group children by parent and apply one fixed ordering:

```text
rank(P) = (P.created_at_ms, P.commit_id)

current = virtual_root
mainline = []

while current has connected children:
    current = child with minimum rank
    append current to mainline

selected_head = last(mainline), or empty
mainline_history = mainline without selected_head
concurrent_archive = all connected publications not in mainline
```

There is no "longest branch," latest-leaf comparison, merge commit, `replaces[]`, or automatic rebasing of stored records. Parent edges express causality; timestamps only rank siblings. A child with a smaller timestamp than its parent remains that parent's child.

```text
          XA @ 10:00:01 ----- XA2             selected branch
         /
H0 -----+
         \
          XB @ 10:00:02 ----- XB2             concurrent archive
```

XA wins the fork under H0. XA2 is selected if there is no earlier-ranked sibling under XA. XB2 cannot rescue its branch by being newer, older, or deeper.

Records with missing parents remain `PendingParent`, not archived or silently dropped. An unknown required schema is an upgrade/incompatibility condition; it must not become permission to ignore a potential competitor and publish as though none exists.

**Manifest download health is not a ranking input.** Once the names select a head, a missing or corrupt manifest yields `Unavailable` or `IntegrityError` for that selected identity. Do not fall back to a healthy losing branch. The same principle applies to a previously validated ancestor that becomes temporarily unreadable.

### 7.1 Guarantees and limitations

For the same listing and the same fixed policy, all compatible clients calculate the same mainline. The computation is order-independent and duplicate-insensitive. Clients converge once their listings agree.

**Selection is final outside the fork window.** The selector has no notion of time and ranks whatever it is given. The limit comes from the writers (section 8.3): a record is only ever written as the first child of the publication that a listing taken seconds earlier showed as selected. A sibling of a publication Q can therefore be written only within the fork window after Q became visible: the adapter's listing lag (zero for the current providers), plus the ten-second listing freshness limit, plus the one request that writes the record. After that window, nothing a conforming writer produces can compete with Q or with any ancestor of Q.

The age of a record's timestamp does not matter. A machine that slept for three days lists again before it writes, sees that its parent already has a child, and stops. No reader-side time limit exists or is needed: a rule such as "ignore a record older than N when it is discovered" would depend on each reader's clock and on when it looked, so two devices would select different heads.

Inside the window the earliest-ranked sibling wins. A later-ranked sibling that was selected for a moment becomes a concurrent archive, together with anything already built on it. On the current providers that is at most a few seconds of history.

A record from a writer that breaks the rule (a defect, an old build) is ranked like any other. Section 6.4 already limits the protocol to trusted writers.

Under append-only valid history with no record removal or identity conflicts, a publication already excluded by an earlier-ranked sibling remains excluded as new records arrive. Recovery must create a new publication, not relabel or mutate that old record.

Whole-snapshot selection also means unrelated edits in a losing publication are not merged. If PA changes conversation X and PB changes Y within the same bundle, only the chosen snapshot is active; the other change is recoverable from its archive. Different streams do not have this competition.

## 8. Publish flow and recovery

### 8.1 Normal publish

1. Review against one exact publication and its verified manifest. For a genuinely new stream, review against its virtual empty root.
2. When Apply starts, list the stream and select again, then rerun the existing local revision, digest, destination, expiry, codec, and approval checks. Compare the freshly selected publication with this replica's bases (section 10.3). The same publication continues. A descendant continues only when every writable scope, or every reviewed Library entry, still has the digest this replica last synced; otherwise the result is `RequiresPull`. Anything else is `Replaced` or `RemoteMissing` and stops.
3. Build the complete next manifest on the freshly selected publication. Carry every untouched scope or Library entry from it byte for byte, under the mutation firewall the engine has today. This is the rebase the engine already performs: a teammate's publication in another scope between Review and Apply does not void this Push. Content from a publication that is not the selected one or an ancestor of it is never combined without another review.
4. If semantic content is unchanged, return `NoChange` with the exact publication it was compared to. Do not create a publication or report a new upload.
5. Put changed objects at `_objects/<sha256>`, then the manifest at `_manifests/<revision>-<commit_id>.json`. A later attempt puts the same keys again, which is idempotent, so nothing has to be remembered between attempts.
6. Build the commit record in memory: a new `commit_id`, the parent from step 2, `revision`, `created_at_ms`, origin, the manifest reference, and the summary.
7. List, then write (section 8.3). If the listing no longer shows the parent selected and childless, stop with `RemoteChanged`: nothing is written, and the user compares again. Otherwise send the record once.
8. An acknowledgment means `Stored`. Without one, list again: the record is there (`Stored`), or it is not and another attempt follows. When the storage cannot be reached at all the outcome is `Unknown`, and the next refresh settles it. After `Stored`, advance the local bases to the stored publication, select again, and report both the storage outcome and the observed role.

A concurrent writer may publish during any step. Once the record is built, never change its parent or ranking timestamp to force it onto the current branch. A stored-but-archived result is a successful storage result, not a CAS failure.

### 8.2 The record is the publication point

Objects or a manifest without a record are preparation residue, not published history. Today the manifest and the commit record are written together before the head swings; after the cut the manifest is written first and the record alone is last.

The record reuses the `_commits/` name and most of today's fields, but it is a replacement shape in a new position. There is no importer for records written by the old protocol, whose final decision occurred at HEAD CAS.

### 8.3 List, then write: the limit on late-written records

Earliest-sibling selection has one hazard. A record that carries an old timestamp but is written late outranks every sibling created in between, and would turn everything built on them into an archive. Only one thing can produce such a record: a writer that prepares a record, keeps it, and sends it later. This design removes that cause instead of adding a second arbitration rule.

**The rule.** Every attempt to write a commit record is the second half of one step:

1. List the stream and select. If the listing already contains this record's `commit_id`, an earlier attempt landed: the outcome is `Stored` and the step ends.
2. Continue only if the selected publication is the record's parent, which means the parent has no child yet. For a first publication, continue only if the stream is empty. Otherwise stop with `RemoteChanged`.
3. Send the record, once.

Four details make the rule hold on a laptop:

- **Every attempt lists first.** A retry is a new attempt, not a resend. There is no code path from "I have a record" to "send it" that skips the listing.
- **The listing must be fresh.** If more than ten seconds of wall-clock time have passed since the listing answered, list again before sending. Use the wall clock: Rust's `Instant` on macOS reads `CLOCK_UPTIME_RAW`, which stands still while the machine sleeps, so a monotonic check cannot see a closed lid.
- **The write is a single request.** `put_record` never retries inside the transport, because a transport retry is an attempt without a listing. The S3 client is built with the SDK defaults today, which retry up to three times, and its timers also stand still during sleep, so the S3 store switches retries off for this one call. The Dropbox client already disables automatic retries; its record write is one `files/upload` request rather than a session start plus a batch commit with Mallard's own code in between. A request that stalls dies with its connection: the provider drops an idle upload within about a minute, so a record cannot land later than that.
- **Nothing is kept.** The record lives in memory for one operation. It is not journaled, and no restart, crash, or reopened window resends it. The next Push reviews again and builds a new record.

**The limit this gives.** A record is written only when a listing at most ten seconds old showed its parent without a child. Any sibling it can compete with was therefore written after that listing, or so recently that the listing could not show it yet. The fork window is:

```text
fork window = adapter listing lag + 10 s listing freshness + one record request
```

On Local, S3/R2, Dropbox, and D1 the listing lag is zero, so the window is a few seconds in practice and about a minute in the worst case of a stalled request. Outside the window, selection is final (section 7.1). The bound does not depend on any clock being right, on how old the record's timestamp is, or on how long a machine slept. A future adapter with a lagging listing widens the window by its stated lag and changes nothing else.

Because no record outlives its operation, there is no durable outbox. Push stays stateless, as it is today: Mallard stores no progress between pushes.

| Failure boundary | What happens |
|---|---|
| Before the record is sent | Nothing is published. Uploaded objects and the manifest are unreferenced residue; a later Push writes the same object keys again |
| The listing before the write shows another child or a different selected publication | `RemoteChanged`. Nothing is written. The user compares again |
| The machine sleeps, or the app stalls, between the listing and the write | The listing is older than ten seconds when the write is about to start, so the attempt lists again |
| The record was sent and no acknowledgment arrived | List. Present means `Stored`. Absent means another attempt, which lists first |
| The storage cannot be reached after sending | `Unknown`. Nothing is persisted; the next refresh shows whether the record arrived |
| The app quits or crashes at any point | Nothing is resent. The next Push builds a new record |
| Record stored, local bases not saved | The next refresh reads `Advanced` with remote content equal to local; a Pull review with nothing to apply advances the bases |
| Record stored but not selected, having lost a race inside the fork window | Keep `Stored`; compute `ConcurrentArchive`; never auto-republish. The next refresh reads `Replaced`: Pull, then Push again |
| Same `commit_id` found with different bytes | Block the affected stream with an integrity diagnostic |

Keep local single-flight behavior for a stream. It simplifies local execution but is not the distributed exclusion mechanism. The Library apply path takes no local lock today, because the head CAS stopped a double Apply; it needs one now, or a double-click writes two siblings. A local cancellation after the record was sent cannot retract a possibly stored publication.

## 9. Discovery, refresh, inventory, and performance

Each stream has its own record prefix, and each record's name carries its graph facts (section 4.4). One listing of one stream is therefore enough to compute the selected publication, the mainline, and every archive role. No record is read to do it.

### 9.1 Refresh compares publications

```text
1. list the stream's _commits prefix                  1 request per 1,000 publications
2. select from the names                              in memory
3. selected commit_id == the one already held?        yes: stop, nothing else is read
4. read the selected record                           1 request; content must equal its name
5. read its manifest                                  1 request; then compare with local as today
```

"The one already held" is the publication whose manifest this process has in its cache. Step 3 compares `commit_id`. It never compares manifest digests, revision order, or ETags. Steps 4 and 5 produce the same `BundleSnapshot { head, manifest }` that `inspect_optional` returns today.

| Situation | Requests | Today |
|---|---|---|
| Nothing changed | 1 listing | 1 head read |
| Selected publication changed, or the first refresh after a restart | listing + record + manifest | head + manifest |
| Remote is ahead of this replica's base | no extra read; continuity is an ancestor walk over the names | 1 uncached probe of the base manifest on every refresh |
| Re-reading a pinned base manifest (the project-files compare) | 1 read by key, unchanged | 1 read by key |
| Push apply, after the uploads | listing + record write | head CAS, tag read, tag CAS (up to four rounds) |
| Library scope page | listing + record + tag + manifest; the record is cached while the selection is unchanged | head + tag + manifest |

Remote reads happen per user action, as today (project open, Refresh, Push review and apply, Pull review and apply). There is no background polling.

Nothing is persisted for discovery: no index, cursor, or scan state. The process keeps the last listing's names, the records it has read, and manifests by digest in the existing object cache. A restart lists again.

### 9.2 What a listing proves

The listing is the working set. A failed page, a permission error, or an expired cursor fails the refresh; it is never read as an empty stream, and a failed read is never an empty manifest.

A publication this process has already seen that is missing from a later listing is confirmed with one direct read of its key before it is dropped. If it still reads, it stays in the set, because the listing lagged. If it is gone, the refresh reports the loss and selects over what exists. An empty stream with a pinned base is `RemoteMissing` (section 10.2), as today.

### 9.3 Inventory and growth

Bundle discovery lists `.mallard/v1/repositories/_commits`, which holds every project stream's records and no payload object. Group the names by bundle id, select each stream's head, read those records with the existing bounded-concurrency metadata fetch, and build the same `RemoteBundleSummary` rows from `summary`. It serves project setup, the link chooser, the "Only in this storage" rows, and sibling-bundle status, as `list_remote_bundles` does today. An empty registration publication provides discoverability before a stream has user content.

Refresh cost grows only with the history of the stream being refreshed: one more listing request per 1,000 publications in that stream. Names sort by depth, so a store can later list only the tail by passing the cursor it already supports. Such an optimization must not become the only discovery path; a full listing stays available and is what a Push uses. Benchmark large histories on the existing providers before release; do not hide this cost behind an assumed constant-time read.

Do not prebuild sharding, compaction, or a shared mutable index in this refactor.

Apply explicit record/page/byte budgets. Hitting a budget reports an incomplete view; it must not silently truncate history and claim a final selected head. Last-known data may be displayed as cached. A Push requires a complete listing of its stream.

## 10. Review pins, continuity, and local safety

### 10.1 Pins keep their shape; comparisons change

No persisted record, IPC type, or TypeScript type changes shape (section 6.1). What changes is how remote coordinates are compared:

| Today | After |
|---|---|
| `classify_remote_continuity(head, base, probe)`: revision order, plus a store probe of the base manifest when the head is ahead | Identity and ancestry over the listing (section 10.2); the probe and its closure are deleted |
| `classify_scope_continuity`: `<` and `>` on `scope_revision` | The scope digest first, then the repository verdict |
| `repository_base_from_link` = `max_by_key(revision)` over scope bases | Every scope base must be the selected publication or an ancestor of it; otherwise `Replaced` |
| `scope_revision.max(…)` when a single-conversation Pull advances a base | Take the scope coordinates of the reviewed publication |
| The Library's `observed.revision < accepted.revision` means "history was rewound" | Ancestry |
| Whole-head equality at Pull apply, the project-files review token, the readiness match | Unchanged. They test `(revision, commit_id, manifest_sha256)` for equality, which identity satisfies |

`scope_revision` stays in the manifest and in `ScopeBase` as a per-branch counter for display. Nothing compares it, and nothing orders by it.

Keep resource/scope content pins and local-state revisions where they protect actual review and apply safety. Removing remote CAS does **not** remove local atomic persistence, local filesystem locks, local optimistic validation, or single-flight UI checks.

### 10.2 Continuity by identity and ancestry

Repository continuity is decided from this replica's bases and one listing. It replaces rows 1 to 8 of `DESIGN_REMOTE_HISTORY_CONTINUITY.md`; the `RemoteContinuity` and `ScopeContinuity` enums keep their shape.

| Condition | Continuity |
|---|---|
| no base, and the stream lists empty | `Fresh` |
| a base is recorded, and the stream lists empty | `RemoteMissing` |
| no base, and a publication is selected | `Unreviewed` |
| the base is the selected publication | `InSync` |
| the base is an ancestor of the selected publication | `Advanced` |
| any other base | `Replaced` |
| the listing failed | `Unknown` |

The ancestor test walks `previous_commit_id` from the selected publication down to the base's revision and compares `commit_id`. It then verifies the link once: the record one level above the base must carry the base's `commit_id` and `manifest_sha256` as its `previous_commit_id` and `previous_manifest_sha256`. Both values are already in the pin, so this costs one immutable, cached record read, and it replaces the uncached base-manifest probe that today runs on every refresh while the remote is ahead. It also closes the "commit-record ancestry walk" left open in that design.

`Replaced` covers a recreated or cleared stream and a reselection inside the fork window. The listing tells them apart for the message: if the base is still listed, it lost a race and is now an archive; if it is not listed, that history is gone. Neither is reported as corruption merely because the selected branch is shallower.

Per scope, the verdict starts from content and only then consults ancestry, because two publications can carry the same scope bytes:

| Scope digest in the selected manifest | Scope verdict |
|---|---|
| equals the scope base digest | `InSync` |
| differs, and repository continuity is `Advanced` | `Advanced` (requires Pull before Push) |
| differs otherwise | `Replaced` |

A known record disappearing is an availability/integrity incident, not permission to reinitialize the stream.

### 10.3 Review and Apply behavior

A review stores the publication it was computed from, in the coordinates it stores today, and whether that was the then-selected publication or an explicitly chosen archive.

**Pull.** Before applying a normal "selected remote" review, list the stream and compare. A different selected publication requires another review, as any head change does today. Explicit archive restore remains bound to that archive's exact publication.

**Push.** Apply lists the stream and rebases onto the selected publication when that is safe (section 8.1, steps 2 and 3). This is the gate `classify_push_destination` applies today, with ancestry in place of revision order:

| Freshly selected publication | Action |
|---|---|
| the publication this replica is based on | publish on it |
| a descendant of it, and every writable scope (or reviewed Library entry) still has this replica's base digest | publish on the new one; carry the other scopes from it |
| a descendant of it, and a writable scope changed | `RequiresPull` |
| not a descendant, or the stream is empty while a base is recorded | `Replaced` or `RemoteMissing`; stop |

Identity and ancestry answer whether the remote moved, and whether it moved forward. The scope digest answers whether this Push's own scopes were touched. Both are needed; neither is a revision comparison.

This freshness check is best effort. A new remote publication can arrive after it, and section 8.3 bounds what it can do. The actual local mutation must always be of the exact reviewed bytes and satisfy unchanged local destination/digest/expiry checks; it must never substitute whatever is newest at Apply time.

Uploaded, selected, reviewed, and installed are not synonyms. A stored publication advances this replica's sync bases, as a Push does today. It never marks anything installed, and a later role change never touches a local pin. If the stored publication later loses its fork, the base is no longer an ancestor of the selected publication, continuity reads `Replaced`, and the existing remedy applies: Pull and review, then Push again.

The Library needs one addition. Its per-resource accepted state also advances on Push, and a row whose accepted copy is absent from the selected manifest is not selectable for Push today. When a link's accepted head is neither the selected publication nor one of its ancestors, reset that link's accepted state the way storage clear already does, so its rows can be pushed again.

### 10.4 Reselection is remote metadata, not local rollback

```text
First listing:          H0 -> XB
A few seconds later:    H0 -> XA, where rank(XA) < rank(XB)
```

This can happen only inside the fork window of section 8.3. The remote selected publication moves to XA, and XB becomes a concurrent archive together with anything built on it in those seconds. Local sessions based on XB remain untouched. Mallard can present the new remote state and prepare a review, but cannot automatically overwrite or delete the user's local work.

Preserve transcript comparison, owned-child consistency, backup/receipt behavior, symlink rejection, path validation, secret warnings, and exact nonrecursive deletion rules. These protections are independent of provider CAS. [S1][S3]

## 11. Result and UI contracts

Use separate storage, role, and availability dimensions:

| Dimension | Values |
|---|---|
| Upload state | Preparing, Publishing, Stored, Unknown, FailedBeforePublication |
| Derived role | SelectedHead, MainlineHistory, ConcurrentArchive, PendingParent |
| Manifest availability | Unchecked, Ready, TemporarilyUnavailable, IntegrityError |
| Discovery state | Listing, Listed, Incomplete, Unavailable |

`Listed` refers to one listing. It says nothing about a publication that may still arrive inside the fork window.

`Unknown` is an in-memory outcome of one operation (section 8.3). It is shown once ("Couldn't confirm the upload. Refresh shows whether it arrived.") and is never stored.

Replace `PublishedBundle.published: bool` and the analogous Library result with a typed result such as `Stored { snapshot, observed_role }`, `NoChange { snapshot }`, or `Unknown`. Ordinary prepublication failures remain errors, and `RemoteChanged` (section 8.3) is one of them: it reads like today's "the storage changed while pushing; compare again" and replaces the CAS-conflict strings. A stored role is a cached observation, not an immutable property of the remote record.

Every surface names a publication by its `commit_id` and prints the revision as the label ("Version 12"), as the outcome notices do today. Two archive rows can show the same number, so an archive row also shows a short form of its `commit_id`.

Keep the main interface focused on one selected snapshot, with an archive entry point:

```text
Current remote version: Version 3
Concurrent archives: 1 branch, 2 publications        View archives
```

A losing local upload says:

> Uploaded and retained. This publication is in a concurrent archive rather than the current mainline. Local content was not changed.

A selected publication later displaced by another branch changes role without changing its stored receipt. A mainline ancestor is shown as history, not as an archive. Archive rows group by the first losing edge and retain all descendants.

The default recovery action is **Restore as a copy** with normal review. Republishing archived content is a new reviewed operation based on the current selected snapshot, preserving unrelated current scopes/entries. Do not mutate old parents, backdate new records, restore an entire old snapshot without an explicit choice, or automatically resubmit archived pushes.

A normal refresh never asks the user to resolve every fork. The archive remains inspectable and recoverable rather than silently disappearing.

## 12. Changes to the existing stores

All entries below are implementation requirements, not claims that the current code already satisfies them. The existing registry contains Local, R2, Dropbox, and Managed Cloud adapters. [S7]

| Store | Change |
|---|---|
| Local folder | Delete `compare_and_swap`. `put_record` is `put_immutable`. Reads, staging, the lock, and no-follow access are unchanged |
| S3/R2 | Delete `compare_and_swap`, the `If-Match` branch, and the ambiguous-CAS resolution. `put_record` runs with SDK retries switched off. `list` skips and reports a key it cannot parse. The capability probe loses its ConditionalOverwrite and StaleConditionalRejection stages |
| Dropbox | Delete `compare_and_swap` and the update-by-rev write mode. `put_record` is one `files/upload` request. `list` skips and reports a non-canonical or conflicted-copy path instead of failing. The setup probe loses its update and stale-revision checks and keeps the write, read, list, and delete checks |
| Shared store wrapper | Its forwarding methods shrink with the trait; connection health is unchanged |
| Managed Cloud | Stage 5 |

Provider setup probes test a write, a verified read-back, and that a listing taken right after the write returns it. They do not prove long-term consistency from one successful run; adapter admission also needs contract review and multi-client fault tests.

Deduplication stays what it is today: the engine skips payloads already in the parent manifest, and an object key is its content hash, so a repeated upload lands on the same key.

### 12.1 Managed Cloud follows the desktop (stage 5)

**Order of work.** The desktop app is cut over first (stages 1 to 4 of section 16). Three consequences are accepted:

- After the cut the engine no longer writes `_head.json` or `_tags/`, which is where the managed adapter commits and lists today. Until stage 5, builds from this branch offer no Mallard Cloud storage. No build ships an adapter that performs expected-HEAD commits for an engine that no longer has a head.
- The desktop adapter and `stub.rs` change together in stage 5.
- At stage 5 the service is deployed before the desktop build that needs it. The reverse order breaks every older desktop, because the client has no version negotiation. There are no customers, and staging and production data are wiped for the retest.

**What the smaller cut saves the service.** The manifest shape, `_objects/<sha256>` addressing, and `commit-<hex>` ids do not change. The service's manifest validator, its inventory check, the website's manifest parsing, session views, and the release flow therefore keep working as they are.

**What stage 5 adds.**

1. The upload carries the commit record bytes, stored verbatim like the manifest and validated against it (`commit_id`, `revision`, `manifest_sha256`, `manifest_size`, `previous_commit_id`).
2. `publications` gains `previous_commit_id` and `created_at_ms`.
3. A route lists a repository's records with their name facts, private scope included.
4. The desktop adapter maps the record write to the upload commit and serves the `_commits/<bundle_id>/` listing from that route.

**Open decision for stage 5: who arbitrates on Managed Cloud.**

| Choice | Consequence |
|---|---|
| (a) The service keeps its commit guard. A record whose `previous_commit_id` is not the current private head is refused with `head_changed`, which the adapter reports as `RemoteChanged` | The service never forks. Its single-head schema, cleanup job, quota, view carry-forward, and release flow stay valid. The desktop selector sees a linear log. Nothing in the engine changes, and no capability flag exists |
| (b) The service stores every sibling and derives its head with the same selector in TypeScript | Drop the unique revision key; stop retiring and collecting non-head publications; make quota archive-aware; carry views from the parent; pin release to the selected publication; share selector test vectors; and settle a trust rule for open public repositories, where any signed-in account can push and could backdate a record |

Recommendation: (a). A provider that refuses a stale parent simply never produces a fork. Earlier revisions forbade this for uniformity only.

The current managed store explicitly refuses the Library namespace and directs it to an item adapter that is not built. [S9] Audit that path as a separate integration item. Do not assume it is covered by changing the project store, expand feature availability accidentally, or leave it as a hidden mandatory-CAS dependency.

## 13. Deletion, clear, and retention

Ordinary sync does not need deletion. The present code already distinguishes storage clear from ordinary conditional deletion. [S2][S8]

First release: no automatic remote garbage collection, archive purge, compaction that removes publication history, or age-based removal of possibly referenced objects. A writer can pause after uploading an object and publish later; a fixed age threshold alone does not make deletion safe.

Logical resource deletion remains a reviewed change inside a new snapshot. It does not physically remove previous publications or referenced payloads. A deletion in a losing branch remains archived with that branch.

**Storage clear stays an in-place operation, as today.** It keeps the marker, deletes everything under the two namespaces, and resets local link state. Other devices then read `RemoteMissing` or `Replaced` and recover through Pull and Push (section 10.2). No new incarnation id is minted, and no device has to reconnect.

One thing changes: **clear deletes records first.** It clears `repositories/_commits` before the rest of `repositories`, and within each Library repository `_commits/` already sorts before `_manifests/` and `_objects/`. Names sort by depth, so a stream's first publications are deleted first. Once they are gone, every other record is disconnected from the virtual root and the stream selects nothing, while every manifest and object still exists. No device can therefore select a record whose manifest or objects are about to disappear. A writer racing the clear is stopped by list-then-write, because its parent is gone, or it leaves an orphan that the next pass deletes. The loop already repeats until a listing is empty.

One case remains. A first publication written in the one-request gap right after a clear finishes can reference objects the clear deleted. That stream then reads `Unavailable` until it is cleared again. The clear dialog already says to stop sync on all machines first. Never claim that clearing while other devices may still write permanently empties the storage.

## 14. Breaking cutover

### 14.1 One supported runtime model

Ship only the new publication protocol for affected sync paths. There is no provider feature flag, legacy HEAD-protocol reader, format-conversion fallback, dual write, automatic importer, or old/new protocol selection in the application.

**All version numbers in this refactor remain `1`.** Replace the commit record shape directly; do not increment its schema constant or introduce a parallel versioned implementation. Retaining the numeric version is not a promise that old records are readable or that an old client may attach to a new storage.

The owner wipes `~/.mallard` and the test storages for the cutover. There are no customers, and nothing is imported.

Local pin shapes do not change, so a stale local base cannot be recognized by its shape. It does not need to be: against fresh storage contents it reads `RemoteMissing` or `Replaced`, and neither verdict mutates anything.

Old clients are not supported on a new storage. They read the marker at its usual key and refuse its format. This is format isolation, not remote access fencing: an old or arbitrary client with raw write permission is outside the trusted-writer contract. Do not write a `_head.json` or `_tags/` to make such clients work.

### 14.2 Version-1 format admission

| Surface | Required value or boundary |
|---|---|
| Marker | `.mallard/_storage.json` with `format = "mallard-append-storage"` and `layout_version = 1` |
| Project records | `.mallard/v1/repositories/_commits/<bundle_id>/<name>`; `schema_version = 1`; the record of section 6.2; name facts equal record fields |
| Library records | `.mallard/v1/library/<repository_id>/_commits/<name>`; the same rules |
| Manifests and objects | Unchanged keys and schemas |
| Removed objects | `_head.json` and `_tags/` are never written and never read |
| Local sync records and IPC types | Unchanged |
| Managed Cloud | Stage 5 |

Validation must reject the old `mallard-storage` marker even though it reports `layout_version = 1`. A record whose name matches the grammar but whose content fails validation must not be silently skipped in order to choose a different winner. A name outside the grammar is ignored and reported.

### 14.3 Remove rather than deprecate

```text
ObjectStore::compare_and_swap, CasExpectation, CasOutcome
HeadToken.etag and ObservedLibraryHead.etag
_head.json as a stored object (BundleHead / LibraryResourceHead remain as in-memory views)
the _tags/ namespace, BundleTag as a wire record, publish_tag and its CAS retry loop
the continuity probe (fetch_manifest_version_optional) and every remote revision ordering
the R2 probe stages ConditionalOverwrite and StaleConditionalRejection; the Dropbox probe's update-by-rev checks
CAS-conflict strings, fakes that emulate If-Match, and their tests
the managed HEAD-commit bridge (stage 5)
legacy HEAD readers, fixtures, fallback paths, and protocol migrations
```

Do not remove local revision protections or unrelated provider HTTP metadata handling just because they also use revisions or ETags.

Update `AGENTS.md`, active README sections, architecture docs, tests, and diagnostics together. State that immutable append plus deterministic selection replaces mutable-HEAD CAS; do not leave contradictory safety requirements in active documentation.

## 15. Code change map

Paths are based on the reviewed repository; proposed modules can be adjusted during implementation without adding another architectural layer.

| Module | Change |
|---|---|
| `crates/mallard-domain/src/publication.rs` — proposed | The name grammar and facts, the selector, roles, and continuity by identity and ancestry, replacing the revision-ordered classifiers in `lib.rs` |
| `storage_core/object_store.rs` | Delete `compare_and_swap` and its types; add `put_record`; document the relaxed `put_immutable` contract |
| `storage_core/mod.rs` | Add `repository_commit_key` and the `_commits` prefixes; delete `REPOSITORY_TAGS_PREFIX` and `repository_tag_key`; clear order |
| `storage_core/marker.rs` | The new `format` constant; creation stays idempotent |
| `storage_core/local_store.rs`, `s3_store.rs`, `dropbox_store.rs` | Section 12 |
| `storage_core/publication.rs` — proposed | List-then-write, head selection over a listing, and the record cache, shared by both engines |
| `crates/mallard-infrastructure/src/bundle_engine.rs` | `read_head` becomes select-then-read-record; publish ends with list-then-write; `publish_tag`, `BundleTag`, and `fetch_manifest_version_optional` are deleted; `list_remote_bundles` reads `_commits/`; `CommitRecord` gains its fields. The manifest build, the scope firewall, restore, and payload reads are untouched |
| `src-tauri/src/storage_catalog/resource_engine.rs` | The same mechanics through the shared module; ancestry replaces the rewind check; no project-domain imports |
| `src-tauri/src/storage_catalog/service.rs` | A local single-flight lock on apply; the accepted-state reset of section 10.3 |
| `src-tauri/src/project_sync/service.rs` | Continuity callers lose the probe closure; `repository_base_from_link` and the two `max(scope_revision)` sites change; outcome mapping for `RemoteChanged` and roles. The apply-time rebase stays as it is |
| Local persistence, IPC DTOs, `src/types.ts` | No shape change; new outcome and role fields only |
| Frontend | Outcome copy and the archive entry point |
| `crates/mallard-infrastructure/src/managed_cloud/` | Stage 5 |
| Integration suites and active docs | New invariants and failure cases; remove CAS protocol assertions |

The pure selector is shared between project and Library. If stage 5 chooses arbitration (b), the service runs the same fixed test vectors against its own implementation so it cannot choose a different branch from the desktop.

## 16. Implementation sequence

Implement in one breaking-change branch, not as two production protocols.

**Stage 1 — Domain.** The name grammar, the selector, roles, and continuity by identity and ancestry, with permutation tests. Pure code, no I/O.

**Stage 2 — Project vertical slice on the local store.** Remove `compare_and_swap` from the trait and from all three stores. Implement the shared module, `read_head` as select-then-read, publish ending in list-then-write, bundle discovery over `_commits/`, the continuity callers, and the clear order. Delete `_head.json`, `_tags/`, and the probe read in the same slice.

**Stage 3 — Library and remote stores.** Move the Library engine to the shared module, add its apply lock and accepted-state reset, implement `put_record` on S3/R2 and Dropbox, relax the listings, and cut the CAS stages from the probes.

**Stage 4 — Desktop release gate.** Run cross-provider fault tests on Local, S3/R2, and Dropbox, full feature safety regressions, DTO/build checks, refresh request counts, listing benchmarks, and the clean-cutover audit. The desktop is done when every sync path it offers follows the new semantics. Mallard Cloud is not offered by these builds.

**Stage 5 — Managed Cloud, after the desktop.** Decide the arbitration question of section 12.1, implement the service and adapter changes, and deploy the service before the desktop build that uses it. Feishu and other identifier-addressed providers start later (section 18).

Intermediate commits may be incremental, but the delivered runtime contains one supported publication model. No temporary compatibility bridge remains in the final design.

## 17. Acceptance tests and release conditions

These are required tests to implement. This document does not claim they have run.

| Test | Required observation |
|---|---|
| Two writers, same parent | Both records are retained; exactly one sibling branch is selected |
| Different streams | Neither stream's publications suppress the other |
| Different changes within one bundle | Entire losing snapshot is archived; no accidental partial merge |
| Sibling timestamp tie | `commit_id` comparison chooses the same branch everywhere |
| Descendant timestamp or branch length | Does not bypass the first losing fork |
| All listing orders and duplicate entries | The same input set produces identical roles and selected head |
| Earlier-ranked sibling inside the fork window | The later-ranked sibling and anything built on it become archives; local files/sessions remain unchanged |
| Writer sleeps between preparing a record and writing it | On wake the attempt lists again, sees its parent has a child, and stops with `RemoteChanged`; nothing is written, whatever the record's timestamp |
| Listing older than ten wall-clock seconds at write time | The attempt lists again before sending; a monotonic-only check is shown not to catch a simulated sleep |
| Record write fails or times out in the transport | The store sends one request and performs no automatic retry; the engine's next attempt lists first |
| App restarts with a Push in flight | Nothing is resent; no pending record exists on disk |
| Parent missing from the listing | The child is `PendingParent`, and resolves when the parent appears |
| Record content disagrees with its name, a cycle, or a parent in another stream | Invalid; never selected on the strength of its name |
| Selected manifest temporarily unavailable | Selected identity retained; no fallback to another candidate |
| Same `commit_id` under two names or with different bytes | Integrity conflict; never first-seen-wins |
| Final acknowledgment lost | One listing settles it: present is `Stored`, absent leads to another list-then-write attempt with the identical bytes |
| Crash after each publish stage | Objects and a manifest without a record never appear as history |
| No-op while the remote changes | No record is written; the exact compared publication is returned |
| Publish becomes archived | No auto-republish loop; bases stay; the next refresh reads `Replaced` |
| Review source changes before Apply | A normal selected-source Pull review is stale; an archive review remains exact and explicit |
| Local source/target changes after Review | Existing local revision, digest, expiry, and backup protections still apply |
| Failed page or expired cursor | Fails the refresh and is never an empty stream; a seen publication missing from a listing is confirmed by a direct read before it is dropped |
| Cold start and restart | One listing, the selected record, and its manifest recover the same selection; nothing was persisted |
| Refresh with an unchanged selected publication | Exactly one listing request; no record read, no manifest read |
| Refresh while the remote is ahead of the base | `Advanced` from the ancestor walk; one cached read verifies the link above the base; no per-refresh probe |
| Foreign names in a stream's prefix (`.DS_Store`, a conflicted copy) | Ignored and reported on all three stores; the listing does not fail |
| Continuity table | One fixture per row of section 10.2, including a sibling branch with a greater revision, which must read `Replaced`, never `Advanced`, and a cleared stream that is pushed again |
| Push rebase at apply | Remote advanced in a disjoint scope: published on the new selected publication with that scope carried byte for byte. Same scope changed: `RequiresPull`. Base not an ancestor: `Replaced` |
| Stored publication later loses its fork | Bases stay; the next refresh reads `Replaced`; Pull then Push recovers; Library rows become pushable again |
| Two siblings with the same revision | Distinct identities everywhere; no surface or comparison treats the shared number as identity |
| Library double Apply | The local single-flight lock stops the second; no sibling is written |
| `BundleHead` view | Equals the selected record field for field; callers of `snapshot.head` are unchanged |
| Bundle discovery | The `_commits/` listing plus the tip records reproduce today's `RemoteBundleSummary` rows; no payload key is listed |
| Setup probes | R2 and Dropbox setup succeed without CAS stages |
| Concurrent initial publications | Virtual genesis handles both; no exclusive-create dependency |
| Marker | Two first writers converge on identical bytes; the old `mallard-storage` marker is rejected by format, not admitted because the number matches |
| Remote role metadata | No `_head.json`, `_tags/`, archive flag, or moved archive object is ever written |
| Clear | Records are deleted first; an interrupted clear leaves streams that select nothing; a racing writer ends with `RemoteChanged` |
| Existing provider parity | Equivalent record sets produce the same roles on Local, S3/R2, and Dropbox at stage 4, and on Managed Cloud at stage 5 |
| Archive recovery | Creates a reviewed copy or new publication from the current parent; old history unchanged |
| Large retained histories | Measured listing cost and explicit limit handling; no silent history truncation |

Also run existing transcript, owned-subagent, scope-mutation firewall, deletion, secret-handling, destination-isolation, key-validation, and frontend review regressions.

The core architectural release condition is testable:

> A store that offers only put, get, and list, with plain overwrite and no conditional write, can execute both project and Library publication, history, review, and restore. Existing providers run that same engine, not a separate CAS engine hidden behind a flag.

## 18. Non-goals and final boundary

This is a storage-model refactor, not a Feishu feature, automatic merge engine, new collaboration editor, server election protocol, or a promise of irrevocable commit order.

**Deferred on purpose.** Opaque locators, a snapshot envelope with a payload table, an `AppendStore` trait, and a storage UID are not needed to remove compare-and-swap, and they are what would have changed the portable manifest and every local pin. An identifier-addressed provider can still be written against this contract: its adapter resolves a key by listing a folder and caching name to id, which is safe because every name is unique by construction and every read is hash-verified. If the first such adapter shows that is too slow, add locators then.

The application still presents one default remote snapshot. It simply derives that snapshot from immutable history instead of asking the provider to atomically replace a shared HEAD. Competing publications are preserved, and losing branches are quietly archived with an explicit recovery path.

The decisive simplification is:

```text
old: provider arbitrates one mutable current reference
new: provider stores independent records; shared Mallard logic selects the view
```

Future provider work should implement addressing, writes, reads, listing, authentication, and conformance—not recreate CAS and not invent another sync protocol.

---

## Source notes and verification boundary

The original draft reviewed repository sources through the connected GitHub tool at the pinned commit. The source notes below substantiate observations at that commit, not current repository freshness or the proposed behavior.

Revisions 2 and 3 (September 18, 2026) fold in the owner's decisions recorded in `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md`. The code facts they cite were read from the working tree and the service repository on that date: the write-only `CommitRecord`, the push rebase and continuity classifiers, `continuity_coordinates` and `ensure_destination_available`, the marker, the S3 client's default retries, the Dropbox client's disabled retries, `Instant` on macOS, the stores' fail-closed listings, the clear loop, and the Managed Cloud service's manifest validation, commit guard, and cleanup job.

No pull request was created, and no production implementation, Rust build, provider conformance suite, or live-provider integration test was run for this document. The previous design's model-test counts are not evidence that this refactor has been implemented or validated.

- **[S1]** `AGENTS.md`: no-backward-compatibility policy, architecture boundaries, immutable publication and local safety rules.
- **[S2]** `storage_core/object_store.rs`: required CAS, key addressing, immutable put, staged verification, and optional maintenance.
- **[S3]** `bundle_engine.rs`: HEAD/expectation types, the commit record, the tag index, full snapshots, scope boundaries, object-key payload retrieval, and publication result.
- **[S4]** `storage_catalog/resource_engine.rs`: complete Library repository sets, HEAD/ETag comparison, and revision continuity checks.
- **[S5]** `storage_catalog/mod.rs`: independent Library domain and service/engine boundary.
- **[S6]** `storage_core/marker.rs`: fixed marker format and idempotent first-writer initialization.
- **[S7]** `storage_providers/mod.rs`: existing compile-time adapter registry and included providers.
- **[S8]** `storage_providers/traits.rs`: adapter responsibilities, connection probing, secrets, and maintenance descriptors.
- **[S9]** `managed_cloud/store.rs`: synthetic object-store protocol, HEAD-CAS finalization, pending upload state, and Library refusal.

[S1]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/AGENTS.md
[S2]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/storage_core/object_store.rs
[S3]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/bundle_engine.rs
[S4]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/src-tauri/src/storage_catalog/resource_engine.rs
[S5]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/src-tauri/src/storage_catalog/mod.rs
[S6]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/storage_core/marker.rs
[S7]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/storage_providers/mod.rs
[S8]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/storage_providers/traits.rs
[S9]: https://github.com/Git4Agent/mallard-app/blob/460edb67a3db63f1960249ad06a5955c5aec3ab0/crates/mallard-infrastructure/src/managed_cloud/store.rs
