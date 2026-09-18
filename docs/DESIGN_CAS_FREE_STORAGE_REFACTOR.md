# Mallard: CAS-free storage model refactor

**Status:** Proposed design; implementation has not been performed\
**Scope:** Replace the existing storage publication model, not add a provider-specific alternative\
**Compatibility:** Breaking cutover; no legacy readers, migration, fallback, or dual-write protocol\
**Versioning:** Layout, protocol schemas, and locator encodings remain at `1`; numeric version equality does not imply compatibility\
**Reviewed baseline:** `Git4Agent/mallard-app@460edb67a3db63f1960249ad06a5955c5aec3ab0`\
**Suggested repository path:** `docs/DESIGN_CAS_FREE_STORAGE_REFACTOR.md`\
**Supersedes:** `DESIGN_APPEND_ONLY_MAINLINE_ARCHIVE.md` for implementation scope and architecture\
**Revision 2 (September 18, 2026):** folds in the owner's four decisions after the evaluation in `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md`:

1. The desktop app is cut over first. Managed Cloud follows in stage 5 (sections 12.1 and 16).
2. Late-written records are limited by one writer rule, *list, then write* (section 8.3). The selector is unchanged.
3. Refresh is one listing of one stream and compares the selected publication version with the one already held. Push rebases onto the selected version at apply time (sections 8.1, 9 and 10.3).
4. The publication version is the only remote coordinate in pins, continuity verdicts, and fork roles. Snapshot coordinates are never pinned (sections 6.2 and 10).

> Providers save immutable bytes and make publications discoverable. Mallard—not the provider—derives the current mainline. Concurrent publications may both be stored; the later-ranked sibling and its descendants are retained as concurrent archives.

This document keeps the previously chosen earliest-sibling policy, but applies it to the existing model and existing providers. It includes the concrete storage-side metadata layout: one immutable root descriptor, immutable publication records, and complete snapshot envelopes alongside payload blobs. Feishu implementation, authentication, API research, and provider-specific fields are out of scope. All types and module names marked as proposed are design sketches, not compiled implementations.

## 1. Decision

Replace the shared mutable-HEAD protocol with a single append-only publication protocol. Local-folder, S3/R2, Dropbox, and Managed Cloud must use the same publication semantics. A future provider must not implement CAS, atomic create-if-absent, exclusive filename creation, a lock service, or an S3 compatibility layer to support ordinary sync.

The protocol retains complete snapshots and one parent per publication. At each fork, select the child with the smallest `(created_at_ms, publication_id)` and continue down that child. Other subtrees remain stored but are excluded from the default remote view.

This is deliberately **not** a single-winner commit protocol. An upload acknowledgment confirms storage, not permanent admission to the mainline. Selection can change when an earlier-ranked sibling appears, but writers only ever add the first child of the publication they just listed as selected, so a sibling can appear only inside a short fork window (section 8.3). Older history is final.

| Decision | Required outcome |
|---|---|
| One protocol | No `supports_cas` switch, strong/weak backend modes, or legacy `ConditionalRefStore` |
| Publication unit | Complete project bundle snapshot; complete existing Library repository set |
| Concurrency scope | Independently per project bundle or Library repository, never storage-wide |
| Arbitration | Deterministic earliest sibling, computed in shared code |
| Late-written records | *List, then write*: every write attempt of a record follows a fresh listing that shows its parent selected and childless. No pending record is kept after the operation ends |
| Version identity | The publication version `(publication_id, record_sha256)`, with `depth` as its display number, is the only remote coordinate in pins, continuity, and fork roles |
| Refresh | One listing of one stream. An unchanged selected version means nothing else is read |
| Losing branch | Stored, classified as concurrent archive, accessible for recovery |
| Provider contract | Append a new object, read by stable locator, list one stream's publications with their graph facts |
| Local application | Explicit Review → Apply; no automatic rollback after remote reselection |
| Cutover | Fresh roots and replacement record shapes, all at version `1`; old formats rejected by identity and structure, never silently imported |
| Order of work | Desktop app first: both engines, Local, S3/R2 and Dropbox, and the UI. The Managed Cloud service, website, and adapter follow in stage 5 |
| First release excludes | Resource-level merging, CRDT text editing, automatic remote GC, Feishu integration |

This replaces the old CAS safety mechanism rather than silently disabling it. The replacement safety properties are immutable preservation, exact identity and digest checks, deterministic selection, explicit archives, and unchanged local review protections.

## 2. What the current code actually depends on

The reviewed code has more CAS coupling than the final HEAD write alone. These are current-code observations; the right column is the proposed change.

| Current dependency | Refactor |
|---|---|
| `ObjectStore` requires `compare_and_swap` and addresses objects with `ObjectKey`. Its common types carry ETags. [S2] | Replace with an append/read/list contract and opaque locators; remove ETags from publication identity |
| `bundle_engine.rs` has `HeadToken`, `PublishExpectation`, `expected_head`, and a `PublishedBundle { published, snapshot }` result. [S3] | Pin an immutable parent publication; return storage outcome separately from derived selection |
| Bundle payload readers reconstruct physical addresses from `entry.object_key`. [S3] | Resolve content through a snapshot-local locator table |
| The Library resource engine reads one HEAD and one full-set manifest, checks the observed ETag, and rejects numeric revision rewind. [S4] | Use the same publication resolver; replace global revision assumptions with immutable references |
| The Library is a separate feature and domain; its current publication unit is the complete repository set for a coordinate. [S4][S5] | Preserve that boundary and granularity; do not import project models into Library |
| Storage initialization uses a fixed marker and an immutable-put/read-back path. [S6] | Explicitly create a new root, then open/join that exact root; no implicit first-writer arbitration |
| Provider registration already centralizes configuration, secrets, discovery, and store construction. [S7][S8] | Keep the registry; change the returned transport and its conformance requirements |
| Managed Cloud synthesizes object-store keys and makes HEAD CAS the upload commit. It refuses Library namespace writes in that adapter. [S9] | Replace the key-emulation bridge and change the service contract; do not pretend a trait rename is enough |

The repository guide explicitly favors removing obsolete paths over compatibility layers, while preserving immutable SHA-256-verified publication and explicit local restore approval. Its mutable-HEAD-specific wording must be updated as part of this change. [S1]

## 3. Architecture and ownership

```text
ProjectSync application                 Library application
  review and capture                      review and capture
          |                                       |
  Bundle snapshot engine                 Library snapshot engine
          +-------------------+-------------------+
                              |
                    PublicationEngine
              append / discover / resolve / history
                              |
                         AppendStore
                    append / read / list
                              |
          Local      S3/R2      Dropbox      Managed Cloud
```

The **domain layer** owns publication validation, stream identity, immutable references, and the pure selector. It has no filesystem, HTTP, provider-key branching, or local clock calls during selection.

The **publication engine** owns I/O orchestration, duplicate handling, the list-then-write step, the in-process publication graph, and publication receipts. It does not understand transcripts, skills, pets, or provider-home paths.

The **feature engines** continue to build and validate their existing complete semantic snapshots, preserve untouched scopes or entries, and prepare review-approved application plans. They share publication mechanics, not domain DTOs.

The **adapter** owns physical addressing, credentials, upload mechanics, and pagination. It neither chooses the winning branch nor invents a provider-specific definition of publication success.

Use the existing crates and provider registry. A new universal plugin framework, generic event bus, separate metadata server, and protocol capability matrix are unnecessary.

## 4. Root identity and streams without exclusive creation

### 4.1 Explicit root creation versus opening

Initialization and normal synchronization are different operations.

`Create storage` provisions a fresh physical root, writes one immutable descriptor, and returns the exact root binding. `Use existing storage` opens and verifies that binding. A random storage UID is generated once and persisted in the existing setup journal before remote mutations.

A proposed descriptor is:

```json
{
  "format": "mallard-append-storage",
  "layout_version": 1,
  "storage_uid": "0123456789abcdef0123456789abcdef",
  "publication_policy": "earliest-sibling-v1",
  "collections": {
    "blobs": {"version": 1, "value": "blobs"},
    "publications": {"version": 1, "value": "publications"}
  }
}
```

The descriptor's locator and SHA-256 are pinned in the local root binding. In this Local/S3-style example, collection values are root-relative paths; an identifier-addressed adapter can use opaque collection IDs instead. Collection locators are stable identifiers interpreted by the configured adapter; they are not credentials or expiring download URLs. The policy field accepts exactly the value defined by this layout; it is not a runtime strategy setting.

The descriptor is written once as `storage.json` during explicit root provisioning and is never overwritten during ordinary sync. Its `format` is deliberately `mallard-append-storage`, distinct from the old `mallard-storage` marker. **Keep `layout_version` at `1`; do not use the number alone to identify the protocol.** Opening requires the exact format, version, policy, required fields, UID, and pinned binding. This is a replacement version-1 format, not compatibility with the old HEAD-based layout.

Two independent Create operations produce two different roots, even when their display names match. There is no promise that the first directory with a given name wins, no implicit reinitialization after a failed lookup, and no search-by-name replacement of a missing pinned root.

For key-addressed providers, use a fresh UID-qualified namespace, such as `.mallard/v1/<storage_uid>/`. For identifier-addressed providers, retain the exact newly created container identifiers. Directory labels and filenames never establish identity.

### 4.2 Stream identity

A storage root contains multiple independent streams:

```text
project/<bundle_id>
library/<catalog_repository_id>
```

A project stream publishes an entire bundle. A Library stream publishes the entire set already owned by that Library repository coordinate. Different streams never compete with one another.

The shared layer uses a validated `StreamId`, not Library- or project-specific DTOs. Feature-level constructors validate and derive the appropriate IDs.

### 4.3 Virtual genesis

Each `(storage_uid, stream_id)` has an implicit empty root, represented by `parent = null`. No client uploads or races to create a special genesis file. Two first publications are simply siblings under that virtual root and follow the ordinary selection rule.

Registering an empty project, when needed by the existing product flow, creates an ordinary publication containing a valid empty/registration snapshot. It does not reserve a unique filename or require a separate HEAD.

### 4.4 Concrete physical layout

For a Local/S3-style adapter, a complete example is:

```text
.mallard/v1/<storage_uid>/
  storage.json                   immutable root descriptor
  blobs/
    o-01                         conversation X0 payload
    o-02                         conversation Y0 payload
    o-03                         complete snapshot envelope for H0
    o-04                         conversation XA payload
    o-05                         complete snapshot envelope for XA
    o-06                         conversation XB payload
    o-07                         complete snapshot envelope for XB
    o-08                         conversation XA2 payload
    o-09                         complete snapshot envelope for XA2
  publications/
    project/example-bundle/      one folder (prefix) per stream
      r-01                       publication H0
      r-02                       publication XA
      r-03                       publication XB
      r-04                       publication XA2
    library/example-catalog-repository/
      r-11                       first Library publication
```

The short names are illustrative. Blob names are fresh, opaque adapter-generated locations, not caller-selected exclusive slots. An identifier-addressed adapter represents the same logical layout with stable object and collection IDs; common code does not reconstruct paths from publication IDs or content hashes.

**Each stream has its own publications folder**, so a refresh lists one stream and never pays for the history of another. A stream folder is a prefix, not a reservation. Nobody creates it exclusively. On an identifier-addressed provider, two folders that carry the same stream name are read as one.

**A publication's listing entry carries its graph facts**: `publication_id`, `parent_id`, `created_at_ms`, and `depth` (section 5.1). A key-addressed adapter writes them into the object name, for example:

```text
1789635601000-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-2.json
<created_at_ms>-<publication_id>-<parent_id or "root">-<depth>
```

The adapter owns that grammar; common code receives parsed facts and never parses a name. A name outside the grammar (`.DS_Store`, a sync client's conflicted copy) is ignored and reported, not treated as a record. A name is lowercase `[a-z0-9.-]` and under 100 bytes, which every current provider accepts. The record stays the authority: a record whose content disagrees with its listed facts is invalid.

Snapshots are JSON blobs in the same `blobs` collection as payloads. They do not require another storage operation or a mutable manifest registry.

H0 in this example is the first actual publication, whose parent is the virtual empty root. It is not a separately uploaded genesis reservation. `r-02` is a physical location; XA is a logical publication identity. On a key-addressed store the name follows from the facts, so a retry of XA writes the same bytes to the same name. On an identifier-addressed store a retry may create a second object with the same facts and bytes.

There is no authoritative `_head.json`, `_index.json`, remotely maintained `archived` flag, or per-stream directory that must be exclusively created. The version-1 path is isolated by a fresh storage UID and the descriptor format, not by incrementing the version number.

## 5. Minimal provider contract

### 5.1 Required operations

The following Rust-like signatures describe the contract, not a drop-in implementation. Reuse the existing staged byte types and synchronous/blocking execution model rather than rewriting transport concurrency. [S2][S3]

```rust
pub struct ObjectLocator {
    pub version: u16, // exactly 1 in this design
    pub value: String, // bounded; adapter-validated; relative to the pinned root
}

/// What a listing returns for one publication without reading it.
pub struct PublicationFacts {
    pub publication_id: PublicationId,
    pub parent_id: Option<PublicationId>, // None for a child of the virtual root
    pub created_at_ms: u64,
    pub depth: u64,
}

pub struct PublicationEntry {
    pub stream: StreamId,
    pub locator: ObjectLocator,
    pub facts: PublicationFacts,
}

pub trait AppendStore: Send + Sync {
    fn append_blob(&self, body: &PayloadBytes) -> Result<ObjectLocator, AppendError>;

    /// One request. The transport never retries it on its own (section 8.3).
    fn append_publication(
        &self,
        stream: &StreamId,
        facts: &PublicationFacts,
        body: &PayloadBytes,
    ) -> Result<ObjectLocator, AppendError>;

    fn read(
        &self,
        locator: &ObjectLocator,
        max_bytes: u64,
    ) -> Result<Option<StoredBody>, StoreError>;

    /// One stream's publications. `None` lists every stream and is used only
    /// to discover which bundles a storage holds.
    fn list_publications(
        &self,
        stream: Option<&StreamId>,
        cursor: Option<&str>,
    ) -> Result<PublicationPage, StoreError>;
}
```

Both appends mean **save a new physical object**, not “write to a caller-chosen shared key.” Common code computes size and SHA-256 from frozen bytes. A locator returned by an adapter is wrapped with those values to form a verified reference.

A blob retry can create another object. On key-addressed stores, adapters generate a fresh random physical name per blob attempt. On identifier-addressed stores, adapters return the provider-assigned object identifier. Neither case needs same-name exclusivity. Do not retry an uncertain blob write by overwriting a previously returned stable locator.

`append_publication` differs in two ways. The adapter must store the facts so that a later listing returns them verbatim; a key-addressed adapter derives the object name from them (section 4.4), so a repeat of the same record rewrites identical bytes under the same name and leaves no duplicate. And the call is a single request: the adapter switches off any automatic retry its client library performs, because every attempt of a record must follow a fresh listing (section 8.3).

`read` returns bounded staged bytes; shared helpers verify size and hash when an expected reference exists. Discovery records are bounded, fully parsed, hashed, and validated before indexing. Retain `VerifiedBody`-style staged reads, no-follow local access, and bounded batch helpers, but do not make ETags part of common content identity.

`list_publications` may be paginated, unordered, and duplicate-producing. Cursor values are opaque. It returns facts, never record bytes, so selecting a head needs no record reads. A completed `append_publication` must be visible to every later listing of that stream; an adapter that cannot promise this states its worst-case listing lag, which widens the fork window of section 8.3 by that amount. Success at the end of pagination means this listing ended, not that no concurrent publication can still arrive inside the fork window.

### 5.2 Exact semantic requirements

| Required property | Meaning |
|---|---|
| Independent creation | One writer's new object does not overwrite another writer's retained object |
| Finalized-object acknowledgment | A successful append identifies the finalized submitted bytes, not merely a temporary upload session |
| Stable retrieval | A persisted locator continues to name the same bytes while access and retention remain valid |
| Whole-record consumption | The adapter exposes finalized publication records, not upload fragments, as publications |
| Listing after write | A completed publication append is returned by every later listing of its stream, or the adapter states its worst-case lag. Local, S3/R2, and D1 behave this way; the Dropbox setup probe confirms it (append, then the listing must show it) |
| Listing facts | The facts passed to `append_publication` come back verbatim in every listing entry for that object |
| Single-shot record write | `append_publication` sends one request and the client library's automatic retry is off for it |
| Integrity verification | Bytes can be checked against the declared size and digest before use |
| Root confinement | Locators cannot escape the pinned root or cause arbitrary URL/path access |

This is CAS-free, not consistency-free. A provider with no reliable finalized writes, or whose listing lag has no stated bound, is not automatically supported.

An append error must distinguish a failure known to occur before publication from an **unknown outcome**. Network timeouts do not prove the object was not saved.

### 5.3 What the common layer must not require

No `compare_and_swap`, `put_if_absent`, distributed mutex, conditional overwrite, conditional delete, server timestamp ordering, permanent idempotency key, server-assigned global revision, or globally unique filename is required for ordinary sync.

Remove `put_immutable(key, ...)` from the common public contract as well: retaining it with an implied atomic no-overwrite guarantee would leave a hidden provider restriction. Protocol immutability comes from fresh locations and never rewriting published references.

Batch transfer, provider checksums, delta listing, and deduplication remain optimizations. They must preserve the same observable semantics and cannot change the selection policy.

## 6. Data model

### 6.1 Blob references and snapshots

```rust
pub struct BlobRef {
    pub locator: ObjectLocator,
    pub size: u64,
    pub sha256: Sha256,
}

/// The one remote coordinate Mallard pins (section 10.1).
pub struct PublicationVersion {
    pub publication_id: PublicationId,
    pub record_sha256: Sha256,
    pub depth: u64, // the number the UI prints; never compared for identity or order
}
```

A content digest identifies bytes; a locator identifies a stored instance of those bytes. Equal content may have multiple locators. A locator is scoped to the current root and must not embed an absolute local path, credential, session token, or presigned URL.

A publication version identifies one publication. Its identity is `(publication_id, record_sha256)`. The record hash covers the snapshot reference, so a version pins the snapshot and every payload beneath it transitively; nothing else about the snapshot is ever pinned.

The feature manifest carries content only. Publication identity, depth, time, and origin live in the publication record, so the project and Library manifests lose their `revision`, `commit_id`, `updated_at`, and publishing-origin fields. Two publications with the same content then carry byte-identical manifests.

Replace physical `object_key` references inside the portable model with content identities. A snapshot has a **complete snapshot-local payload table**:

```text
SnapshotEnvelope<FeatureManifest>
  format / schema
  storage_uid / stream_id
  manifest                   complete project or Library semantic manifest
  payloads                   content SHA-256 -> { locator, size, SHA-256 }
```

Feature files and resources refer to their content digest and declared size. The payload table supplies the locator. Identical digests must have consistent sizes; every referenced payload must be covered. Reuse verified references from the selected parent snapshot for unchanged content; upload changed content to fresh locations.

The envelope itself is uploaded as a blob and referenced by the publication. This avoids a shared mutable content-hash-to-location index. Use a small shared envelope/helper where useful, without merging the two feature manifest schemas or adding runtime manifest plugins.

No-op detection compares semantic content, not newly allocated locators or timestamps. Reusing an already verified local cache entry with the same root, digest, and size is permitted.

The concrete outer envelope uses `format = "mallard-snapshot"` and `schema = 1`. Project and Library keep their own complete semantic manifests, with their schema constants remaining `1`; the validated stream determines which feature validator applies. The following JSONC illustrates the envelope and payload table, not a replacement definition of either feature manifest:

```jsonc
{
  "format": "mallard-snapshot",
  "schema": 1,
  "storage_uid": "0123456789abcdef0123456789abcdef",
  "stream_id": "project/example-bundle",
  "manifest": {
    // Insert the COMPLETE project semantic manifest here, with schema 1.
    // Resource/file entries reference content SHA-256 and size, not paths.
    // For a Library stream, insert its complete Library repository set.
  },
  "payloads": {
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee": {
      "locator": {"version": 1, "value": "blobs/o-04"},
      "size": 8200,
      "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    },
    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff": {
      "locator": {"version": 1, "value": "blobs/o-02"},
      "size": 6400,
      "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    }
  }
}
```

The repeated hash characters and sizes are placeholders, not hashes of supplied payloads. The first table entry represents newly uploaded XA; the second reuses unchanged Y0. Every referenced payload must appear in the table with matching size/hash, and unreferenced table entries are rejected to keep the envelope exact and bounded.

For H0 containing `X = X0, Y = Y0`, a publication changing only X carries a complete `X = XA, Y = Y0` manifest but uploads only the new X payload, a new envelope, and a final publication record. The new envelope reuses Y0's existing locator. Reading a chosen snapshot does not require replaying ancestor manifests to locate its payloads; ancestry is still needed to establish the publication's place in the mainline.

### 6.2 Publication record

```json
{
  "format": "mallard-publication",
  "schema": 1,
  "storage_uid": "0123456789abcdef0123456789abcdef",
  "stream_id": "project/example-bundle",
  "publication_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "parent": {
    "publication_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "record_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  },
  "depth": 2,
  "created_at_ms": 1789635601000,
  "origin_id": "opaque-device-origin",
  "snapshot": {
    "locator": {"version": 1, "value": "blobs/o-05"},
    "size": 12345,
    "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
  }
}
```

This example is XA, pointing to its complete snapshot at `blobs/o-05`. Hashes and sizes are illustrative. The publication does not contain `is_current`, `is_archived`, `winner`, `accepted`, `global_revision`, or `head_etag`. Those are either derived observations or obsolete protocol concepts, not immutable publication facts.

The record's own hash is computed from its exact stored bytes and retained in child references and local pins. It is not embedded as a self-referential field in the record itself. Together with `publication_id` and `depth` it forms the record's **publication version** (section 6.1).

The `parent` field is `null` only for a child of the virtual root. A non-null parent must identify a record in the same root and stream, and its hash must match. Reject cycles, self-parenting, cross-stream ancestry, duplicate fields, invalid identifiers, and unbounded structures.

`depth` is `1` for a child of the virtual root and `parent.depth + 1` otherwise; a record that breaks this is invalid. It replaces the old global revision as the number people see ("Version 2"). Two siblings share a depth, so the number is never an identity and never an order between branches.

`publication_id` is a randomly generated 128-bit value encoded as exactly 32 lowercase hexadecimal characters. Comparison is lexicographic over that fixed encoding. The wire integer `created_at_ms` is UTC Unix milliseconds in `0..2^53-1`, so Rust and TypeScript can compare it exactly.

Freeze the timestamp near the final publication step, after preparing the snapshot. It is not transcript time, file mtime, provider metadata time, or first-observed time. It is a deterministic priority input, not proof of real-world arrival order. Do not reject otherwise valid records using each reader's current wall clock.

The record is built once per operation and held in memory. Every attempt inside that operation sends exactly the same bytes, ID, timestamp, parent reference, and snapshot reference, and every attempt follows a fresh listing (section 8.3). The record is never written to local disk and never outlives the operation: after a crash, a restart, or a stop, the next Push reviews again and builds a new record. Changing any field requires a new publication and the appropriate review.

### 6.3 Duplicate and conflicting identity

Multiple physical objects containing the same publication ID and exact bytes are one logical publication; any verified copy may serve reads.

The same ID with different bytes is an integrity conflict, not another candidate to rank. Stop mutation of the affected stream and report diagnostics. Never resolve it by discovery order, provider mtime, or whichever copy was cached first.

Hashes provide integrity, not author authentication. This version relies on trusted writers within the storage ACL; defending against an authorized malicious writer who deliberately backdates records is not part of the protocol.

### 6.4 Library metadata uses the same wrapper

A Library publication uses the same version-1 record and BlobRef shape:

```json
{
  "format": "mallard-publication",
  "schema": 1,
  "storage_uid": "0123456789abcdef0123456789abcdef",
  "stream_id": "library/example-catalog-repository",
  "publication_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "parent": null,
  "depth": 1,
  "created_at_ms": 1789635700000,
  "origin_id": "opaque-device-origin",
  "snapshot": {
    "locator": {"version": 1, "value": "blobs/o-library-snapshot"},
    "size": 9500,
    "sha256": "1111111111111111111111111111111111111111111111111111111111111111"
  }
}
```

The illustrative stream suffix must be replaced with the feature's validated repository ID. Its envelope has `format = "mallard-snapshot"`, `schema = 1`, the same root and Library stream, the complete Library repository-set manifest, and its complete payload table. No project DTOs are imported into the Library manifest. This publication competes only with siblings in that Library stream, never with project publications or unrelated Library repositories.

### 6.5 Stored records versus computed roles

For the physical example in section 4.4:

| Record | Parent | Depth | Frozen example time | Snapshot | Derived role after all four records are known |
|---|---|---|---|---|---|
| H0 at `r-01` | `null` | 1 | `t` | `blobs/o-03` | MainlineHistory |
| XA at `r-02` | H0 | 2 | `t + 1 second` | `blobs/o-05` | MainlineHistory |
| XB at `r-03` | H0 | 2 | `t + 2 seconds` | `blobs/o-07` | ConcurrentArchive |
| XA2 at `r-04` | XA | 3 | `t + 5 seconds` | `blobs/o-09` | SelectedHead |

XA and XB are both "Version 2". Roles, pins, and archive rows therefore name a publication by its version identity, and print the depth only as a label.

The stored records do not change when these roles are computed. Do not move XB to an archive folder, add `archived = true`, mark XA2 `current = true`, or update a remote HEAD. If XB later receives a child XB2, its whole losing subtree remains archived. Mainline selection and the fork window are specified in sections 7 and 8.3.

On an identifier-addressed store, a second object `r-05` containing the exact same XA ID and bytes as `r-02` is another stored copy of XA, not a new publication or fork. Different bytes under XA's ID trigger section 6.3's integrity-conflict handling.

### 6.6 Remote metadata versus local state

| Information | Authoritative remote location | Local or derived treatment |
|---|---|---|
| Root identity, layout, collections, fixed selection policy | Immutable storage descriptor | Pin exact descriptor locator and hash |
| Publication ID, parent, depth, timestamp, origin, snapshot reference | Immutable publication; its graph facts are repeated in the listing entry | Read only the records that are needed (the selected one, a pinned base); cache them in process |
| Complete semantic state and payload locations | Immutable snapshot envelope | Validate with the corresponding feature rules |
| Conversation/resource payload bytes | Referenced immutable blobs | Stage and verify size/hash before use |
| Selected head, mainline history, archive role | None | Computed from one listing of the stream; nothing is persisted |
| Listing results and read records | None | In-process cache only; a restart lists again |
| Upload progress, the pending record, unknown outcomes | None | In memory for the life of one operation; never on disk |
| Reviewed, accepted, and installed versions | None | Local approval and installation evidence, keyed by publication version |
| Credentials, local paths, trust decisions, backups | None | Protected local state; never portable metadata |

Managed Cloud may materialize graph roles and summaries in database rows for efficient queries, but those rows are rebuildable publication-derived projections, not another authority or a sibling-rejection gate.

Normally, provisioning adds one immutable descriptor per root. Each non-no-op Push then adds **two metadata objects**—one complete snapshot envelope and one small publication record—plus changed payload blobs. A no-op adds none. Physical retries or opportunistic duplicate payloads can increase object counts without adding another logical publication.

## 7. Pure mainline resolution

Selection runs over the publications returned by one listing of a stream. Its inputs are the listing facts (`publication_id`, `parent_id`, `created_at_ms`, `depth`), so it needs no record reads. The record remains the authority: whenever a record is read, its content must equal its listed facts, and a parent hash is verified whenever both records are in hand. Continuity always verifies the link directly above a pinned base (section 10.2).

Group children by parent and apply one fixed ordering:

```text
rank(P) = (P.created_at_ms, P.publication_id)

current = virtual_root
mainline = []

while current has connected children:
    current = child with minimum rank
    append current to mainline

selected_head = last(mainline), or empty
mainline_history = mainline without selected_head
concurrent_archive = all connected publications not in mainline
```

There is no “longest branch,” latest-leaf comparison, merge commit, `replaces[]`, or automatic rebasing. Parent edges express causality; timestamps only rank siblings. A child with a smaller timestamp than its parent remains that parent's child.

```text
          XA @ 10:00:01 ----- XA2             selected branch
         /
H0 -----+
         \
          XB @ 10:00:02 ----- XB2             concurrent archive
```

XA wins the fork under H0. XA2 is selected if there is no earlier-ranked sibling under XA. XB2 cannot rescue its branch by being newer, older, or deeper.

Records with missing parents remain `PendingParent`, not archived or silently dropped. An unknown required schema is an upgrade/incompatibility condition; it must not become permission to ignore a potential competitor and publish as though none exists.

**Snapshot download health is not a ranking input.** Once the record and ancestry select a head, a missing or corrupt snapshot yields `Unavailable` or `IntegrityError` for that selected identity. Do not fall back to a healthy losing branch. The same principle applies to a previously validated ancestor that becomes temporarily unreadable.

### 7.1 Guarantees and limitations

For the same listing and the same fixed policy, all compatible clients calculate the same mainline. The computation is order-independent and duplicate-insensitive. Clients converge once their listings agree.

**Selection is final outside the fork window.** The selector has no notion of time and ranks whatever it is given. The limit comes from the writers (section 8.3): a record is only ever written as the first child of the publication that a listing taken seconds earlier showed as selected. A sibling of a publication Q can therefore be written only within the fork window after Q became visible: the adapter's listing lag (zero for the current providers), plus the ten-second listing freshness limit, plus the one request that writes the record. After that window, nothing a conforming writer produces can compete with Q or with any ancestor of Q.

The age of a record's timestamp does not matter. A machine that slept for three days lists again before it writes, sees that its parent already has a child, and stops. No reader-side time limit exists or is needed: a rule such as "ignore a record older than N when it is discovered" would depend on each reader's clock and on when it looked, so two devices would select different heads.

Inside the window the earliest-ranked sibling wins. A later-ranked sibling that was selected for a moment becomes a concurrent archive, together with anything already built on it. On the current providers that is at most a few seconds of history.

A record from a writer that breaks the rule (a defect, an old build) is ranked like any other. Section 6.3 already limits the protocol to trusted writers.

Under append-only valid history with no record removal or identity conflicts, a publication already excluded by an earlier-ranked sibling remains excluded as new records arrive. Recovery must create a new publication, not relabel or mutate that old record.

Whole-snapshot selection also means unrelated edits in a losing publication are not merged. If PA changes conversation X and PB changes Y within the same bundle, only the chosen snapshot is active; the other change is recoverable from its archive. Different streams do not have this competition.

## 8. Publish flow and recovery

### 8.1 Normal publish

1. Review against one exact publication version and its verified feature snapshot. For a genuinely new observed stream, review against its virtual empty root.
2. When Apply starts, list the stream and select again, then rerun the existing local revision, digest, destination, expiry, codec, and approval checks. Compare the freshly selected version with this replica's bases (section 10.3). The same version continues. A descendant continues only when every writable scope, or every reviewed Library entry, still has the digest this replica last synced; otherwise the result is `RequiresPull`. Anything else is `Replaced` or `RemoteMissing` and stops.
3. Build the complete next snapshot on the freshly selected version. Carry every untouched scope or Library entry from it byte for byte. This is the rebase the engine performs today: a teammate's publication in another scope between Review and Apply does not void this Push. Content from a version that is not an ancestor-or-equal of the selected one is never combined without another review.
4. If semantic content is unchanged, return `NoChange` with the exact version it was compared to. Do not create a publication or report a new upload.
5. Append changed payloads, then the complete snapshot envelope. Remember the returned references in memory, so a second attempt in the same session does not upload them again.
6. Build the final record in memory: a new `publication_id`, the parent version from step 2, `depth`, `created_at_ms`, origin, and the snapshot reference.
7. List, then write (section 8.3). If the listing no longer shows the parent selected and childless, stop with `RemoteChanged`: nothing is written, and the user compares again. Otherwise send the record once.
8. An acknowledgment means `Stored`. Without one, list again: the record is there (`Stored`), or it is not and another attempt follows. When the storage cannot be reached at all the outcome is `Unknown`, and the next refresh settles it. After `Stored`, advance the local bases to the stored version, select again, and report both the storage outcome and the observed role.

A concurrent writer may publish during any step. Once the final record is built, never change its parent or ranking timestamp to force it onto the current branch. A stored-but-archived result is a successful storage result, not a CAS failure.

### 8.2 The final record is the publication point

Payloads or a snapshot without a final publication are preparation residue, not published history. Conversely, an adapter cannot acknowledge a publication while planning to delete its referenced data because it did not win selection.

Existing `_commits/` files must not be reinterpreted as this log: they belong to a protocol whose final decision occurs at HEAD CAS. There is no legacy importer in this cutover.

### 8.3 List, then write: the limit on late-written records

Earliest-sibling selection has one hazard. A record that carries an old timestamp but is written late outranks every sibling created in between, and would turn everything built on them into an archive. Only one thing can produce such a record: a writer that prepares a record, keeps it, and sends it later. Revision 1 required exactly that (persist the frozen record, resend the identical bytes after a restart). This revision removes the cause instead of adding a second arbitration rule.

**The rule.** Every attempt to write a publication record is the second half of one step:

1. List the stream and select. If the listing already contains this record's `publication_id`, an earlier attempt landed: the outcome is `Stored` and the step ends.
2. Continue only if the selected version is the record's parent, which means the parent has no child yet. For a first publication, continue only if the stream is empty. Otherwise stop with `RemoteChanged`.
3. Send the record, once.

Four details make the rule hold on a laptop:

- **Every attempt lists first.** A retry is a new attempt, not a resend. There is no code path from "I have a record" to "send it" that skips the listing.
- **The listing must be fresh.** If more than ten seconds of wall-clock time have passed since the listing answered, list again before sending. Use the wall clock: Rust's `Instant` on macOS reads `CLOCK_UPTIME_RAW`, which stands still while the machine sleeps, so a monotonic check cannot see a closed lid.
- **The write is a single request.** `append_publication` never retries inside the transport, because a transport retry is an attempt without a listing. The AWS SDK retries up to three times by default, and its timers also stand still during sleep, so the S3 adapter switches retries off for this one call. A request that stalls dies with its connection: the provider drops an idle upload within about a minute, so a record cannot land later than that.
- **Nothing is kept.** The record lives in memory for one operation. It is not journaled, and no restart, crash, or reopened window resends it. The next Push reviews again and builds a new record.

**The limit this gives.** A record is written only when a listing at most ten seconds old showed its parent without a child. Any sibling it can compete with was therefore written after that listing, or so recently that the listing could not show it yet. The fork window is:

```text
fork window = adapter listing lag + 10 s listing freshness + one record request
```

On Local, S3/R2, Dropbox, and D1 the listing lag is zero, so the window is a few seconds in practice and about a minute in the worst case of a stalled request. Outside the window, selection is final (section 7.1). The bound does not depend on any clock being right, on how old the record's timestamp is, or on how long a machine slept. A future adapter with a lagging listing widens the window by its stated lag and changes nothing else.

Because no record outlives its operation, there is no durable outbox. This also keeps Push stateless, as it is today: Mallard stores no progress between pushes.

| Failure boundary | What happens |
|---|---|
| Before the record is sent | Nothing is published. Uploaded blobs are unreferenced residue; the same session reuses them |
| The listing before the write shows another child or a different selected version | `RemoteChanged`. Nothing is written. The user compares again |
| The machine sleeps, or the app stalls, between the listing and the write | The listing is older than ten seconds when the write is about to start, so the attempt lists again |
| The record was sent and no acknowledgment arrived | List. Present means `Stored`. Absent means another attempt, which lists first |
| The storage cannot be reached after sending | `Unknown`. Nothing is persisted; the next refresh shows whether the record arrived |
| The app quits or crashes at any point | Nothing is resent. The next Push builds a new record |
| Record stored, local bases not saved | The next refresh reads `Advanced` with remote content equal to local; a Pull review with nothing to apply advances the bases |
| Record stored but not selected, having lost a race inside the fork window | Keep `Stored`; compute `ConcurrentArchive`; never auto-republish. The next refresh reads `Replaced`: Pull, then Push again |
| Same ID found with different bytes | Block the affected stream with an integrity diagnostic |

Keep local single-flight behavior for a stream. It simplifies local execution but is not the distributed exclusion mechanism. The Library apply path takes no local lock today, because the head CAS stopped a double Apply; it needs one now, or a double-click writes two siblings. A local cancellation after the record was sent cannot retract a possibly stored publication.

## 9. Discovery, refresh, inventory, and performance

Each stream has its own publications folder, and each listing entry carries the publication's graph facts (sections 4.4 and 5.1). One listing of one stream is therefore enough to compute the selected publication, the mainline, and every archive role. No record is read to do it.

### 9.1 Refresh compares publication versions

```text
1. list the stream                                    1 request per 1,000 publications
2. select from the listing facts                      in memory
3. selected version == the version already held?      yes: stop, nothing else is read
4. read the selected record                           1 request; content must equal its facts
5. read its snapshot envelope                         1 request; then compare with local as today
```

"The version already held" is the publication version whose snapshot this process has in its cache. Step 3 compares versions. It never compares snapshot digests, revisions, or ETags.

| Situation | Requests | Today |
|---|---|---|
| Nothing changed | 1 listing | 1 head read |
| Selected version changed, or the first refresh after a restart | listing + record + snapshot | head + manifest |
| Remote is ahead of this replica's base | no extra read; continuity is an ancestor walk over the listing facts | 1 uncached probe of the base manifest on every refresh |
| New device, old stream | listing + record + snapshot | head + manifest |
| Library storage page | 1 listing per coordinate, at most 6 | 2 to 3 fixed-key reads per coordinate |

Remote reads happen per user action, as today (project open, Refresh, Push review and apply, Pull review and apply). There is no background polling.

Nothing is persisted for discovery: no index, cursor, or scan state. The process keeps the last listing's facts, the records it has read, and snapshots by digest in memory. A restart lists again.

### 9.2 What a listing proves

The listing is the working set. A failed page, a permission error, or an expired cursor fails the refresh; it is never read as an empty stream, and a failed read is never an empty snapshot.

A publication this process has already seen that is missing from a later listing is confirmed with one direct read of its locator before it is dropped. If it still reads, it stays in the set, because the listing lagged. If it is gone, the refresh reports the loss and selects over what exists. An empty stream with a pinned base is `RemoteMissing` (section 10.2), as today.

### 9.3 Inventory and growth

`list_publications(None)` lists every stream. It is used only to discover which bundles a storage holds: project setup, the link chooser, and the "Only in this storage" rows. Inventory and summaries are derived from publications and the selected feature snapshot. Remove mutable `_tags`/HEAD indexes as authoritative inputs. An empty registration publication can provide discoverability before a stream has user content. Any cached server-side summary must identify the publication version it represents.

Refresh cost grows only with the history of the stream being refreshed: one more listing request per 1,000 publications in that stream. Object names begin with the timestamp, so an adapter can later list only the tail (S3 `start-after`, Dropbox's own listing cursor) without a protocol change. Such an optimization must not become the only discovery path; a full listing stays available and is what a Push uses. Benchmark large histories on the existing providers before release; do not hide this cost behind an assumed constant-time read.

Do not prebuild sharding, compaction, or a shared mutable index in this refactor.

Apply explicit record/page/byte budgets. Hitting a budget reports an incomplete view; it must not silently truncate history and claim a final selected head. Last-known data may be displayed as cached. A Push requires a complete listing of its stream.

## 10. Review pins, continuity, and local safety

### 10.1 The publication version is the only remote coordinate

Replace every remote ETag, revision, commit, and manifest coordinate with the publication version of section 6.1:

```rust
pub struct PublicationVersion {
    pub publication_id: PublicationId,
    pub record_sha256: Sha256,
    pub depth: u64, // display only
}
```

Nothing about the snapshot is pinned. The record hash covers the snapshot's locator, size, and digest, so a version pins them transitively. The storage and the stream are not repeated in the pin either: the link that owns the pin already names both, and the root binding pins the storage UID.

The version replaces the head coordinates in these records, with no conversion from the old shapes:

| Today | After |
|---|---|
| `RepositoryBase { revision, commit_id, manifest_sha256, manifest_size }` | `PublicationVersion` |
| `ScopeBase { scope_revision, scope_sha256, reviewed_in, … }` | `ScopeBase { scope_sha256, reviewed_in: PublicationVersion, … }` |
| `MaterializationRecord`, `ConversationApplyReceipt`, `AcceptedConversationVersion` (revision, commit, manifest digest) | one `PublicationVersion` field each |
| `LibraryHeadCoordinate` (revision, commit, manifest digest and size) | `PublicationVersion`, plus the repository and storage identity it already carries |
| `RestorePlan`, `PullReview`, `ConversationRevisionPins`, and the other head-shaped IPC types | one `PublicationVersion` field; `scope_revision` is dropped |

A scope pin is `(scope_sha256, reviewed_in)`. A per-scope edit counter may remain in the manifest for display, but it is a per-branch number: nothing compares it, and nothing orders by it.

Reading a pinned base again (the three-way project-files compare does this) takes two reads: the base record, found through its listing entry and checked against `record_sha256`, then the snapshot it names. Both are immutable and stay cached in the process.

Keep resource/scope content pins and local-state revisions where they protect actual review and apply safety. Removing remote CAS does **not** remove local atomic persistence, local filesystem locks, local optimistic validation, or single-flight UI checks.

Remote numeric revisions are no longer unique identities or a total order. Remove `BundleHead`/`LibraryResourceHead` as persisted protocol records and every remote revision comparison: the anti-rewind checks, `max_by_key(revision)` over scope bases, and `max(scope_revision)`. `depth` is printed and never controls acceptance or continuity.

### 10.2 Continuity by publication version

Repository continuity is decided from this replica's base versions and one listing. It replaces rows 1 to 8 of `DESIGN_REMOTE_HISTORY_CONTINUITY.md`.

| Condition | Continuity |
|---|---|
| no base, and the stream lists empty | `Fresh` |
| a base is recorded, and the stream lists empty | `RemoteMissing` |
| no base, and a version is selected | `Unreviewed` |
| the base is the selected version | `InSync` |
| the base is an ancestor of the selected version | `Advanced` |
| any other base | `Replaced` |
| the listing failed | `Unknown` |

The ancestor test walks `parent_id` from the selected version down to the base's depth and compares identities. It then verifies the hash link once: the record one level above the base must name the base's `record_sha256` as its parent hash. That read is immutable and cached, and it replaces the uncached base-manifest probe that today runs on every refresh while the remote is ahead. When a link holds several scope bases, continuity is `Replaced` if any of them is neither the selected version nor one of its ancestors.

`Replaced` covers both a recreated stream and a reselection inside the fork window. The listing tells them apart for the message: if the base is still listed, it lost a race and is now an archive; if it is not listed, that history is gone. Neither is reported as corruption merely because the selected branch is shallower.

Per scope, the verdict starts from content and only then consults the version, because two publications can carry the same scope bytes:

| Scope digest in the selected snapshot | Scope verdict |
|---|---|
| equals the scope base digest | `InSync` |
| differs, and repository continuity is `Advanced` | `Advanced` (requires Pull before Push) |
| differs otherwise | `Replaced` |

A known record disappearing is an availability/integrity incident, not permission to reinitialize the stream.

### 10.3 Review and Apply behavior

A review stores the publication version it was computed from, and whether that was the then-selected version or an explicitly chosen archive.

**Pull.** Before applying a normal "selected remote" review, list the stream and compare versions. A different selected version requires another review, as any head change does today. Explicit archive restore remains bound to that archive's exact version.

**Push.** Apply lists the stream and rebases onto the selected version when that is safe (section 8.1, steps 2 and 3):

| Freshly selected version | Action |
|---|---|
| the version this replica is based on | publish on it |
| a descendant of it, and every writable scope (or reviewed Library entry) still has this replica's base digest | publish on the new version; carry the other scopes from it |
| a descendant of it, and a writable scope changed | `RequiresPull` |
| not a descendant, or the stream is empty while a base is recorded | `Replaced` or `RemoteMissing`; stop |

The version answers whether the remote moved, and whether it moved forward. The scope digest answers whether this Push's own scopes were touched. Both are needed; neither is a revision number.

This freshness check is best effort. A new remote publication can arrive after it, and section 8.3 bounds what it can do. The actual local mutation must always be of the exact reviewed bytes and satisfy unchanged local destination/digest/expiry checks; it must never substitute whatever is newest at Apply time.

Maintain separate facts, each as a publication version:

```text
last observed selected version
last stored version from this device
per-scope and per-resource reviewed or accepted version, with content evidence
per-resource actually installed version, with content evidence
```

Uploaded, selected, reviewed, and installed are not synonyms. A stored publication advances this replica's sync bases, as a Push does today. It never marks anything installed, and a later role change never touches a local pin. If the stored version later loses its fork, the base is no longer an ancestor of the selected version, continuity reads `Replaced`, and the existing remedy applies: Pull and review, then Push again.

The Library needs one addition. Its per-resource accepted state also advances on Push, and a row whose accepted copy is absent from the selected snapshot is not selectable for Push today. When a link's accepted version is neither the selected version nor one of its ancestors, reset that link's accepted state the way storage clear already does, so its rows can be pushed again.

### 10.4 Reselection is remote metadata, not local rollback

```text
First listing:          H0 -> XB
A few seconds later:    H0 -> XA, where rank(XA) < rank(XB)
```

This can happen only inside the fork window of section 8.3. The remote selected version moves to XA, and XB becomes a concurrent archive together with anything built on it in those seconds. Local sessions based on XB remain untouched. Mallard can present the new remote state and prepare a review, but cannot automatically overwrite or delete the user's local work.

Preserve transcript comparison, owned-child consistency, backup/receipt behavior, symlink rejection, path validation, secret warnings, and exact nonrecursive deletion rules. These protections are independent of provider CAS. [S1][S3]

## 11. Result and UI contracts

Use separate storage, role, and availability dimensions:

| Dimension | Values |
|---|---|
| Upload state | Preparing, Publishing, Stored, Unknown, FailedBeforePublication |
| Derived role | SelectedHead, MainlineHistory, ConcurrentArchive, PendingParent |
| Snapshot availability | Unchecked, Ready, TemporarilyUnavailable, IntegrityError |
| Discovery state | Listing, Listed, Incomplete, Unavailable |

`Listed` refers to one listing. It says nothing about a publication that may still arrive inside the fork window.

`Unknown` is an in-memory outcome of one operation (section 8.3). It is shown once ("Couldn't confirm the upload. Refresh shows whether it arrived.") and is never stored.

Replace `PublishedBundle.published: bool` and the analogous Library result with a typed result such as `Stored { version, observed_role }`, `NoChange { compared_to }`, or `Unknown`, where `version` and `compared_to` are publication versions. Ordinary prepublication failures remain errors, and `RemoteChanged` (section 8.3) is one of them: it reads like today's "the storage changed while pushing; compare again". A stored role is a cached observation, not an immutable property of the remote record.

Every surface names a publication by its version identity and prints `depth` as the label ("Version 12"). Two archive rows can show the same number, so an archive row also shows a short form of its `publication_id`.

Keep the main interface focused on one selected snapshot, with an archive entry point:

```text
Current remote version: Version 3 (XA2)
Concurrent archives: 1 branch, 2 publications        View archives
```

A losing local upload says:

> Uploaded and retained. This publication is in a concurrent archive rather than the current mainline. Local content was not changed.

A selected publication later displaced by another branch changes role without changing its stored receipt. A mainline ancestor is shown as history, not as an archive. Archive rows group by the first losing edge and retain all descendants.

The default recovery action is **Restore as a copy** with normal review. Republishing archived content is a new reviewed operation based on the current selected snapshot, preserving unrelated current scopes/entries. Do not mutate old parents, backdate new records, restore an entire old snapshot without an explicit choice, or automatically resubmit archived pushes.

A normal refresh never asks the user to resolve every fork. The archive remains inspectable and recoverable rather than silently disappearing.

## 12. Refactoring the existing adapters

All entries below are implementation requirements, not claims that the current code already satisfies them. The existing registry contains Local, R2, Dropbox, and Managed Cloud adapters. [S7]

| Adapter | Change |
|---|---|
| Local folder | Fresh relative blob names; one folder per stream; publication names carry the listing facts; temporary writes outside the publication feed; finalize before listing; names outside the grammar ignored and reported; stable root-relative locators; keep bounded/no-follow access; stop hashing every listed file, since listing entries no longer carry an ETag |
| S3/R2 | Fresh random blob keys for each append attempt; one prefix per stream; publication keys carry the listing facts; the record `PutObject` runs with SDK retries switched off (the client is built with the SDK default of three attempts today); return key-relative locators; retain streaming and checksum validation; remove HEAD CAS and conditional-write onboarding requirements |
| Dropbox | Fresh uploaded blobs with returned stable addressing; one folder per stream, listed non-recursively; publication names carry the listing facts; the record is written with one `files/upload` request instead of an upload session plus a batch commit (the client already disables automatic retries); a conflicted-copy or otherwise foreign name is ignored and reported instead of failing the listing; retain authentication and pagination mechanics; no revision-conditioned HEAD update |
| Managed Cloud | Stage 5. Expose real append/read/discovery behavior; remove the synthesized HEAD/tag bridge and finalize all valid siblings rather than comparing an expected head |

Provider setup probes should test append, finalized read-back, and that a listing taken right after a probe publication returns it with its facts. They do not prove long-term consistency from one successful run; adapter admission also needs contract review and multi-client fault tests. Probe cleanup is optional maintenance, not a prerequisite for ordinary sync.

Deduplication is opportunistic. Reuse a verified reference already available in the parent snapshot or a safe cache; otherwise append another copy. No global deduplication lock or mutable location map is introduced.

### 12.1 Managed Cloud is a coordinated service cutover, after the desktop

**Order of work.** The desktop app is cut over first (stages 1 to 4 of section 16). The Managed Cloud service, its website, and the desktop adapter follow in stage 5. Three consequences are accepted:

- Once `ObjectStore` is removed, the current managed adapter no longer compiles, and the live service refuses the new snapshot shape: it validates `object_key == _objects/<sha256>` and a `commit-<hex>` id at upload and again at commit, and its website parses the manifest in the browser. Until stage 5, builds from this branch offer no Mallard Cloud storage. No build ever ships an adapter that still performs expected-HEAD commits.
- The desktop adapter and `stub.rs` are rewritten together in stage 5 against the contract below. Until the service matches that contract, only the stub can exercise the adapter.
- At stage 5 the service is deployed before the desktop build that needs it. The reverse order breaks every older desktop, because the client has no version negotiation. There are no customers, and staging and production data are wiped for the retest.

The existing managed adapter stages objects until a manifest creates an upload; HEAD CAS commits it. Its in-memory pending state and synthesized keys encode the old protocol. [S9] Delete that emulation instead of wrapping it in an `AppendStore` implementation that still requires old HEAD arbitration.

The updated service contract must allow storage of two distinct publications with the same parent, enumerate both, retain both snapshots and payloads, and address immutable publications directly. Service-side transactions or uniqueness on one publication's ID are acceptable implementation details; uniqueness on `parent_id` or rejection because a different publication already advanced HEAD is not.

A possible new API shape is:

```text
POST /v1/storage-roots/{root}/blobs
GET  /v1/storage-roots/{root}/blobs/{object}
POST /v1/storage-roots/{root}/streams/{stream}/publications
GET  /v1/storage-roots/{root}/streams/{stream}/publications?cursor=...    entries carry the listing facts
GET  /v1/storage-roots/{root}/publications?cursor=...                      every stream; bundle discovery only
```

These paths are proposed, not existing endpoints. The API prefix remains `/v1`; resource paths and required request shapes identify the replacement contract, not a new numeric API version. Old HEAD-commit handlers are removed in the coordinated deployment rather than retained as a fallback. Publication creation is the finalized append step. A lost response remains recoverable by ID/hash and listing; server idempotency is an optimization, not the client's only recovery mechanism.

Website views, repository summaries, permissions, quota accounting, and publication-derived caches must respect exact publication identity and the same selector. A cached current-head row is allowed only as a rebuildable projection, never a separate arbiter. Retention cannot depend on selection, and upload expiration must not delete blobs referenced by an archived publication.

The current managed ObjectStore explicitly refuses the Library namespace and directs it to an item adapter. [S9] Audit that service-facing Library path as a separate integration item. Do not assume it is covered by changing the project store, expand feature availability accidentally, or leave it as a hidden mandatory-CAS dependency for future providers. Wherever cloud Library sync is offered, it must follow the same stored/selected/archive contract while preserving its ACL and product boundaries.

The service work is larger than a new route. The evaluation of September 18, 2026 read the service and found that the head is a pointer column with a unique `(repository, scope, revision)` key and no parent column; that a cleanup job deletes every object the single head does not reference within about five minutes; that session views are carried forward from the single head; that public release is an approval pinned to the private head; and that open public repositories accept a push from any signed-in account, which does not fit the trusted-writer assumption of section 6.3. Stage 5 starts from that list. It is a dependency for offering Mallard Cloud again, not for the desktop cutover, and never a reason to retain a legacy client branch.

## 13. Deletion, clear, and retention

Ordinary sync does not need deletion. Keep explicit administrative maintenance separate from `AppendStore`; use the existing provider descriptor for availability rather than adding a protocol capability matrix. The present code already distinguishes storage clear from ordinary conditional deletion. [S2][S8]

First release: no automatic remote garbage collection, archive purge, compaction that removes publication history, or age-based removal of possibly referenced prepared objects. A writer can pause after uploading a blob and publish later; a fixed age threshold alone does not make deletion safe.

Logical resource deletion remains a reviewed change inside a new snapshot. It does not physically remove previous publications or referenced payloads. A deletion in a losing branch remains archived with that branch.

A storage-wide clear is a destructive administrative operation, not a distributed lock. It must disclose active-writer races and use provider-specific safety controls where available. Never claim that clearing while other devices may still write permanently empties the root. A new storage incarnation gets a new UID and pinned root; do not reuse a cleared root as an in-place continuation.

Do not automatically clear old storage to perform the cutover. Backups, existing remote data, and local user sessions remain untouched unless the user explicitly chooses a destructive operation.

## 14. Breaking cutover

### 14.1 One supported runtime model

Ship only the new publication protocol for affected sync paths. There is no provider feature flag, legacy HEAD-protocol reader, format-conversion fallback, dual write, automatic importer, or old/new protocol selection in the application.

**All version numbers in this refactor remain `1`.** Replace the affected project/Library snapshot, root-binding, remote-pin, receipt, and cache record shapes directly; do not increment their schema constants or introduce a parallel versioned implementation. Unchanged credential/configuration schemas also stay unchanged. Retaining the numeric version is not a promise that old records are readable or that an old client may attach to a new root.

Create a fresh storage root and republish from explicitly reviewed current local content. Old remote history is not automatically carried forward. Retaining its bytes outside the new root is not backward compatibility; reading/importing it in the new sync engine would be.

Legacy pending plans and remote acceptance pins cannot be translated by treating an old commit ID as a publication ID. Invalidate them. Since both designs can contain `schema = 1`, validate the new required shape, exact publication coordinates, and append-root binding rather than accepting an integer alone. Do not supply serde defaults or adapters that silently turn old HEAD coordinates into valid publication pins. Do not delete user data to reset derived state. Unsupported old sync records should produce a reconnect/reinitialize-required result, not silent acceptance or automatic wiping of `~/.mallard`.

Old clients are not supported on a new root. A distinct descriptor format and disjoint UID-qualified bindings separate the protocols during normal open/join, but version `1` alone cannot reject an old client. Root binding must never be guessed from a display name or reused legacy location. This is format isolation, not remote access fencing: an old or arbitrary client with raw write permission is outside the trusted-writer contract. Do not fabricate an old `_storage.json`/HEAD layout inside a new root to make such clients work.

### 14.2 Version-1 format admission

| Surface | Required value or boundary |
|---|---|
| Root descriptor | `format = "mallard-append-storage"`, `layout_version = 1`, exact policy and required root/collection fields |
| Namespace for key-addressed adapters | Fresh `.mallard/v1/<storage_uid>/`; never reuse the old fixed repository/catalog root |
| Publication record | `format = "mallard-publication"`, `schema = 1`, complete immutable parent/snapshot references |
| Snapshot envelope | `format = "mallard-snapshot"`, `schema = 1`, matching root/stream and complete payload table |
| Feature manifests | Existing project/Library schema constants remain `1`; replacement content-reference shape validated by its feature |
| Object/collection locator | `version = 1`; bounded, stable, adapter-validated and root-confined |
| New or changed local sync records | Schema remains `1`; required append-root and publication-aware fields, no conversion from old shapes |
| Managed Cloud API examples | `/v1/storage-roots/...`; replacement resources and contracts, coordinated service/client deployment |
| Unrelated configuration and application versions | Not changed by this design merely to signal the storage cutover |

Validation must reject the old `mallard-storage` marker even when it reports `layout_version = 1`. Reject old HEAD-shaped records, missing payload-location tables, unsupported format tags, and unsupported versions before using them as current state. An unknown record in a root's publication feed must not be silently skipped in order to choose a different winner.

The descriptor's format tag, complete required shapes, exact root binding, and strict field validation distinguish the new protocol. A version bump, migration table, compatibility reader, or runtime protocol selector is neither required nor introduced.

### 14.3 Remove rather than deprecate

Remove the following from the supported publication path:

```text
ObjectStore::compare_and_swap
CasExpectation / CasOutcome
common put_immutable(key, ...) contract
HeadToken / PublishExpectation / expected_head
BundleHead / LibraryResourceHead wire records
ObservedLibraryHead and ETag-based head coordinates
shared mutable _head.json and authoritative _tags indexes
mandatory conditional-write capability probes
CAS-conflict retry/reuse behavior
managed synthetic-key / HEAD-commit bridge
legacy HEAD readers, fixtures, fallback paths, and protocol migrations
```

Remote content revision integers no longer provide global ordering. Audit all `<`, `==`, “rewind,” “latest,” and “already synced” logic based on those values. Do not remove local revision protections or unrelated provider HTTP metadata handling just because they also use revisions or ETags.

Update `AGENTS.md`, active README sections, architecture docs, DTOs, tests, and diagnostics together. State that immutable append plus deterministic selection replaces mutable-HEAD CAS; do not leave contradictory safety requirements in active documentation.

## 15. Code change map

Paths are based on the reviewed repository; proposed modules can be adjusted during implementation without adding another architectural layer.

| Module | Responsibility after refactor |
|---|---|
| `crates/mallard-domain/src/publication.rs` — proposed | Common IDs, the publication version, listing facts, validation, the graph selector over facts, roles, and version-based continuity |
| `crates/mallard-infrastructure/src/storage_core/object_store.rs` | Replace old contract with `append_store.rs`; retain bounded staging helpers in an appropriate shared file |
| `storage_core/marker.rs` and namespace constructors | Distinct append-storage descriptor format and new root binding at layout `1`; remove implicit shared marker creation |
| `storage_core/local_store.rs`, `s3_store.rs`, `dropbox_store.rs` | Implement fresh blob append, opaque-locator reads, per-stream publication listings that return facts, and the single-request record write |
| `storage_providers/traits.rs`, registry and adapters | Open new transport; explicit create/join root; revised probes; preserve config/secret handling |
| `crates/mallard-infrastructure/src/publication_engine.rs` — proposed | The list-then-write step, refresh by version, deduplication, the in-process graph, history |
| `crates/mallard-infrastructure/src/bundle_engine.rs` | Full-snapshot construction, scope firewall, payload references; use publication engine; preserve restore behavior |
| `src-tauri/src/storage_catalog/resource_engine.rs` | Full-set Library snapshots on the same publication engine; no project-domain imports |
| Existing machine-local persistence modules | Root bindings and publication-version pins. No outbox, record cache, index, or scan state is persisted |
| `src-tauri/src/project_sync/` and Library service/planning modules | New reviews, stored/no-op/unknown outcomes, archives, reviewed recovery |
| `crates/mallard-infrastructure/src/managed_cloud/` | Stage 5: new service client/store contract and publication-bound website views |
| `src/types.ts` and feature `api.ts` / `types.ts` / outcome models | Matched Rust/TypeScript DTO cutover and status text |
| Integration suites and active docs | New invariants, failure cases, unsupported-layout behavior; remove CAS protocol assertions |

The pure selector must be shared between project and Library. Where a managed website uses another language, run the same fixed test vectors against both implementations so it cannot choose a different branch from the desktop.

## 16. Implementation sequence

Implement in one breaking-change branch, not as two production protocols.

**Stage 1 — New domain and genuinely weak test store.** Define wire types, root/stream identity, payload references, and the pure selector. The test store provides append/read/list only; it must not expose locks, CAS, or exclusive creation to the engine.

**Stage 2 — Working local vertical slice.** Implement root creation/opening, Local append storage with per-stream listings and facts, the list-then-write step, refresh by publication version, version-based pins and continuity, project publication, archive display, and exact reviewed Pull. Remove the local legacy publication path in that slice.

**Stage 3 — Complete the shared-model cutover.** Move Library to the same engine, update remote pins and receipts in both features, and change S3/R2 and Dropbox adapters and probes. Remove obsolete HEAD/ETag/key-addressed dependencies as their callers are replaced.

**Stage 4 — Desktop release gate.** Run cross-provider fault tests on Local, S3/R2, and Dropbox, full feature safety regressions, DTO/build checks, refresh request counts, cold-start/listing benchmarks, and the clean-cutover audit. The desktop is done when every sync path it offers follows the new semantics. Mallard Cloud is not offered by these builds.

**Stage 5 — Coordinated Managed Cloud change, after the desktop.** Implement the required service/client/web contract from section 12.1, account for its separate Library item path, and verify retention of archived publications. Deploy the service before the desktop build that uses it. Do not release a desktop adapter whose new interface still performs old expected-HEAD commits. Feishu starts later as an adapter-only project subject to the same admission tests.

Intermediate commits may be incremental, but the delivered runtime contains one supported publication model. No temporary compatibility bridge remains in the final design.

## 17. Acceptance tests and release conditions

These are required tests to implement. This document does not claim they have run.

| Test | Required observation |
|---|---|
| Two writers, same parent | Both records are retained; exactly one observed sibling branch is selected |
| Different streams | Neither stream's publications suppress the other |
| Different changes within one bundle | Entire losing snapshot is archived; no accidental partial merge |
| Sibling timestamp tie | Fixed ID comparison chooses the same branch in Rust and TypeScript |
| Descendant timestamp or branch length | Does not bypass the first losing fork |
| All arrival permutations and duplicate deliveries | Same complete input set produces identical roles and selected head |
| Earlier-ranked sibling inside the fork window | The later-ranked sibling and anything built on it become archives; local files/sessions remain unchanged |
| Writer sleeps between preparing a record and writing it | On wake the attempt lists again, sees its parent has a child, and stops with `RemoteChanged`; nothing is written, whatever the record's timestamp |
| Listing older than ten wall-clock seconds at write time | The attempt lists again before sending; a monotonic-only check is shown not to catch a simulated sleep |
| Record write fails or times out in the transport | The adapter sends one request and performs no automatic retry; the engine's next attempt lists first |
| App restarts with a Push in flight | Nothing is resent; no pending record exists on disk |
| Parent arrives after child | Child remains pending, then resolves correctly |
| Parent hash/root/stream mismatch or cycle | Rejected or blocked deterministically; no unsafe parent substitution |
| Selected snapshot temporarily unavailable | Selected identity retained; no fallback to another candidate |
| Same publication bytes, multiple locators | One logical record; either verified copy may serve reads |
| Same publication ID, different bytes | Integrity conflict; never first-seen-wins |
| Final upload acknowledgment lost | One listing settles it: present is `Stored`, absent leads to another list-then-write attempt with the identical bytes; no new logical publication inside one operation |
| Crash after each publication stage | Preparation never masquerades as publication; stored records remain recoverable |
| No-op while remote view changes | No new publication or false upload receipt; exact compared identity returned |
| Publish becomes archived | No auto-rebase/resubmit loop and no false local sync status |
| Review source changes before Apply | Normal current-source review is stale; archive review remains exact and explicit |
| Local source/target changes after Review | Existing local revision, digest, expiry, and backup protections still apply |
| Missing/duplicate pages, expired cursors | A failed page fails the refresh and is never an empty stream; a seen publication missing from a listing is confirmed by a direct read before it is dropped |
| Cold start and restart | One listing, the selected record, and its snapshot recover the same selection; nothing was persisted |
| Refresh with an unchanged selected version | Exactly one listing request; no record read, no snapshot read |
| Refresh while the remote is ahead of the base | `Advanced` from the ancestor walk over listing facts; one cached read verifies the hash link above the base; no per-refresh probe |
| Listing facts disagree with record content | The record is invalid; it is never selected on the strength of its name |
| Foreign names in a stream folder (`.DS_Store`, a conflicted copy) | Ignored and reported; the listing does not fail |
| Continuity table | One fixture per row of section 10.2, including a sibling branch with a greater depth, which must read `Replaced`, never `Advanced` |
| Push rebase at apply | Remote advanced in a disjoint scope: published on the new selected version with that scope carried byte for byte. Same scope changed: `RequiresPull`. Base not an ancestor: `Replaced` |
| Stored version later loses its fork | Bases stay on the stored version; the next refresh reads `Replaced`; Pull then Push recovers; Library rows become pushable again |
| Two siblings with the same depth | Distinct version identities everywhere; no surface or pin treats the shared number as identity |
| Library double Apply | The local single-flight lock stops the second; no sibling is written |
| Setup on append-only fake provider | Succeeds without testing or invoking CAS/create-if-absent/delete |
| Concurrent initial publications | Virtual genesis handles both; no exclusive-create dependency |
| Root re-creation or wrong binding | Fails continuity checks rather than silently continuing another incarnation |
| Version constants and examples | Layout/schema/locator versions stay `1`; namespace and proposed API paths use `v1` |
| Old marker with numeric layout `1` | Rejected by format and shape, not admitted because the version number matches |
| Old HEAD-shaped pin or snapshot with schema `1` | Rejected by required shape/root binding; no permissive default conversion |
| Old layout or old affected local record shapes | Explicitly unsupported; no fallback, implicit import, or destructive reset |
| Snapshot payload table | Complete referenced coverage and consistent size/hash; reuse unchanged references; reject unused or missing entries |
| Remote role metadata | No authoritative HEAD, archive flag, moved archive object, or mutable location index written |
| Existing provider parity | Equivalent publication sets produce the same roles on Local, S3/R2, and Dropbox at stage 4, and on Managed Cloud at stage 5 |
| Managed Cloud archived data (stage 5) | Survives upload finalization, quota/cleanup paths, and website projection updates |
| Archive recovery | Creates a reviewed copy or new publication from the current parent; old history unchanged |
| Transport address attacks | Root escape, arbitrary URL, credential-bearing locator, and unbounded reads rejected |
| Large retained histories | Measured scan/cold-start cost and explicit limit handling; no silent history truncation |

Also run existing transcript, owned-subagent, scope-mutation firewall, deletion, secret-handling, destination-isolation, and frontend review regressions. A CAS-free fake store passing the happy path alone is not sufficient.

The core architectural release condition is testable:

> A conforming append/read/list-only store can execute both project and Library publication, history, review, and restore without any conditional remote mutation. Existing providers run that same engine, not a separate CAS engine hidden behind a flag.

## 18. Non-goals and final boundary

This is a storage-model refactor, not a Feishu feature, automatic merge engine, new collaboration editor, server election protocol, or a promise of irrevocable commit order.

The application still presents one default remote snapshot. It simply derives that snapshot from immutable history instead of asking the provider to atomically replace a shared HEAD. Competing publications are preserved, and losing branches are quietly archived with an explicit recovery path.

The decisive simplification is:

```text
old: provider arbitrates one mutable current reference
new: provider stores independent records; shared Mallard logic selects the view
```

Future provider work should implement addressing, creation, reads, discovery, authentication, and conformance—not recreate CAS and not invent another sync protocol.

---

## Source notes and verification boundary

The original draft reviewed repository sources through the connected GitHub tool at the pinned commit. This update edits that locally supplied design, incorporates the storage-metadata examples, and keeps numeric versions at `1`; it does not claim a new repository review. The earlier design supplied the selection-policy baseline. The source notes below substantiate observations at the pinned commit, not current repository freshness or the proposed behavior.

Revision 2 (September 18, 2026) folds in the owner's decisions recorded in `CAS_FREE_STORAGE_REFACTOR_EVALUATION_2026-09-18.md`. The code facts it cites (the push rebase in `service.rs`, the continuity classifiers, the S3 client's default retries, the Dropbox client's disabled retries, `Instant` on macOS, and the Managed Cloud service's manifest validation and cleanup job) were read from the working tree and the service repository on that date.

No repository files were modified, no pull request was created, and no production implementation, Rust build, provider conformance suite, or live-provider integration test was run for this document. The managed service backend itself was not inspected. The previous design's model-test counts are not evidence that this broader refactor has been implemented or validated.

- **[S1]** `AGENTS.md`: no-backward-compatibility policy, architecture boundaries, immutable publication and local safety rules.
- **[S2]** `storage_core/object_store.rs`: required CAS, key addressing, immutable put, staged verification, and optional maintenance.
- **[S3]** `bundle_engine.rs`: HEAD/expectation types, full snapshots, scope boundaries, object-key payload retrieval, and publication result.
- **[S4]** `storage_catalog/resource_engine.rs`: complete Library repository sets, HEAD/ETag comparison, and revision continuity checks.
- **[S5]** `storage_catalog/mod.rs`: independent Library domain and service/engine boundary.
- **[S6]** `storage_core/marker.rs`: fixed marker format and implicit first-writer initialization.
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
