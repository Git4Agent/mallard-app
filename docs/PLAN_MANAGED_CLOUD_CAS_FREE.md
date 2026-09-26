# Mallard Cloud on Mallard's storage metadata — implementation plan

- Status: Implemented September 23, 2026, on the owner's word ("go and implement"; clean cutover, schemas stay 1, `~/.mallard` and the cloud data are wiped for retesting). Steps 0 to 8 are done: the service against `STORE_API.md` (the wire contract, in the service tree), the desktop client, stub, store, discovery branch, registry entry, parity test, and the Library per item. On September 23, 2026, both live tests passed against staging after the D1/R2 reset. Production was then reset and deployed. Rika321 has super_admin in both environments. The 15-row manual desktop pass in section 13 remains untested. *Implementation notes* at the end list what differs from the plan as written. Revision 3 of September 22 (this text below) replaced revision 1 (the graph in D1, the service arbitrating at commit) and revision 2 (no D1 at all) on the owner's instructions: Mallard Cloud reuses Mallard's own storage metadata, the R2 layout every storage has, and behaves like R2 with claims, so that a Pull never applies an outranked version; D1 goes on helping the service and the website as it does today, as an index of what R2 holds
- Last reviewed: September 23, 2026, against the implementing tree (desktop) and the service tree in `mallardInternal/cloudflare/managed-cloud`; the wire contract both sides implement is `STORE_API.md` in the service tree
- Scope: the Mallard Cloud service and the parts of its website that read content, the desktop's managed store, client and stub, the production storage registry, and the Library on Mallard Cloud (the wiring plan's Phase 4, folded in as section 6.6 and step 7)
- Compatibility: clean cutover; no migration, no legacy path. `migrations/0001_init.sql` is edited in place and stays schema 1. Staging's D1 and R2 are wiped; production's too, on the owner's word
- Deployment assumption: the service is deployed to staging before the desktop build that uses it, and to production only after the staging pass of section 13
- Schemas: every desktop schema and layout number stays `1`. The catalog's machine-local links document gains the item link of section 6.6 under the same schema number, and the owner wipes `~/.mallard`; no DTO or TypeScript type changes shape; the managed store's key layout is the layout every store already has
- Related: `DESIGN_CAS_FREE_STORAGE_REFACTOR.md` (the protocol; sections 7.2, 9.4, 10 and 12 are what this plan holds Mallard Cloud to; its section 14 is superseded by this plan), `PLAN_PULL_PINNED_VERSION.md` (a Pull applies the version it reviewed; T3 and Q4 are what claims remove), `PLAN_CLEARED_STORAGE_PUSH_AGAIN.md`, `CLOUDFLARE_R2_MANAGED_CLOUD_ARCHITECTURE.md` (sections 4.2, 4.5, 5, 6, 7 and 10 change), `MANAGED_CLOUD_DESKTOP_WIRING_PLAN.md`, `DESIGN_MANAGED_CLOUD_STATUS.md`, `CURRENT_METADATA_LAYOUT.md`

Code locations are collected in *Source pointers* at the end, not in the body.

## 1. Where things stand

The CAS-free cutover made the publication record the only thing that publishes
a snapshot: the engine lists a stream of record names, selects the head from
the names, reads the selected record, and publishes by list-then-write. On
S3/R2 and Dropbox a create-only claim of the parent admits one child per
version, so a listed version can never lose: a version a Pull review pinned
cannot be outranked afterwards, and neither can a confirmed Push.

Mallard Cloud was built before that, on a mutable head in D1 swung by a
commit batch, with every object and publication indexed in D1 and a cleanup
job that deletes what nothing references. After the cutover nothing reads a
head and nothing writes one, so the managed store cannot run, and the adapter
is out of the production registry.

Mallard Cloud's bytes already live in an R2 bucket behind a Worker, and the
desktop already has a store for R2 with claims. What Mallard Cloud lacks is
the bucket laid out the way every other Mallard storage is laid out,
reachable through the Worker with the three operations a store needs and the
one create-only write a store with claims needs. What it already has, an index
in D1 that the website and the product routes read, it keeps.

## 2. Goal

**R2 holds Mallard's storage metadata, and the desktop runs the R2 protocol
on it.** Each repository owns a prefix laid out byte for byte like a
user-owned Mallard storage: marker, objects, manifests, records, claims. The
managed store is the R2 store's twin: it maps each engine key to the same key
under the prefix, sends the same requests in the same order, reports
`supports_claims`, and gives its users the same finality. The shared protocol
runs unchanged, every Push and Pull screen runs unchanged, and a Pull applies
the version it reviewed, which no later push can outrank.

**D1 is the service's index of R2.** It records what the Worker has seen
land, publication by publication and object by object, as it does today, so
the website's lists, cards, history, release flow, quota and cleanup keep
their shape and cost. It is derived: the Worker writes it after the R2 write
that it describes, a reconciliation job rebuilds it from the bucket, and when
the two disagree the bucket is right. The desktop protocol never reads it.
Nothing in D1 arbitrates a push, and nothing the service does moves a
stream's selection.

## 3. Decisions

The owner confirms these before step 0.

1. **Gateway, not commit.** The Worker exposes `PUT`, `GET` and a prefix
   listing on a repository's R2 prefix (section 5.1), and nothing else for
   content. The three-step upload, the commit batch, `expected_commit_id`,
   the revision pins and the `stale` upload go.
2. **Claims, through R2's own create-only write.** The store reports
   `supports_claims = true`. `put_claim` is one request the Worker answers
   with R2's create-only write, the same refusal the desktop's own R2 store
   relies on. The Worker adds one guard R2 alone can answer: a record is
   stored only when the claim of its parent holds exactly these bytes, so a
   client that skips the claim cannot fork a stream. The guard is not
   arbitration; the claim is.
3. **Every write is create-only.** A refused write of an object, a view, a
   manifest or a record means the key already holds these bytes and answers
   `AlreadyPresent`. A refused claim answers the bytes it found, and the
   store tells its own (`Mine`) from another's (`Taken`) as the S3 store
   does.
4. **A Pull never applies an outranked version, as on R2.** The desktop
   selects from the R2 listing and pins by record name; the claim admits one
   child per version; the Worker never deletes a claim or a record except in
   Discard, and then in the clear order of section 12 of the CAS-free plan;
   the index of decision 5 is never consulted by the store. So a version a
   Pull review pinned is still the version the apply installs, or the apply
   stops as `PLAN_PULL_PINNED_VERSION.md` says, and `Displaced` cannot be
   read on Mallard Cloud except across a Discard, where it is the
   cleared-storage note. The retention rule of decision 7 keeps every listed
   version complete, so a pinned Pull never finds part of its version gone.
5. **D1 indexes what lands, as today.** After a write lands in R2 the Worker
   records it: an object row, a publication row with its record facts, the
   head pointer, who pushed, the quota, an audit row. The website and the
   product routes read the index. The index is derived: a write that landed
   in R2 but not in D1 is repaired by the reconciliation job, and a `GET` or
   listing from the desktop never depends on it.
6. **Views are content-derived objects.** The store builds a view for every
   conversation whose transcript it is pushing, as today, and writes it at
   `repositories/<bundle>/_views/<view schema>-<transcript sha256>.json`,
   create-only, before the manifest. An unchanged transcript needs no new
   view because its key is the transcript's digest: the carry-forward the
   service did at commit is a property of the key.
7. **Nothing a listed record references is deleted.** Cleanup removes only
   residue: objects, views and manifests that no listed record's manifest and
   no public release references, and that are older than a day. Today's
   erasure of retired manifests beyond the newest 32 goes: a Pull may pin any
   listed version, and on R2 every version stays complete.
8. **Discard clears the chain and keeps the content.** Discard deletes the
   bundle's claims, then its records, looping until both listings are empty,
   and touches no manifest, view or object; the index follows. Desktops read
   an empty stream, which to them is a cleared storage; the next Push starts
   at revision 1 and re-references objects that are already there. The
   content of the discarded chain becomes residue for cleanup, except what a
   public release references.
9. **Release, history, the public face stay as they are.** A release
   request pins a publication row, which now mirrors an R2 record; approval
   creates a public publication row referencing the same manifest and
   objects; the public face checks references through `publication_objects`
   as today.
10. **Discovery reads the index.** `list_remote_bundles` on a managed storage
    is served from the repository list, whose rows carry each head's record
    summary, by a branch in the application layer; the store refuses the
    all-streams listing. This is display, not protocol: no publish or refresh
    depends on it.
11. **Catalog items, one per Cloud repository.** The website's model stays:
    each skill, plugin intent or pet is its own repository with its own
    address, page, visibility, release and stars, and the account's home page
    lists them all. On the desktop the Library publishes one repository per
    coordinate, so on a managed storage the unit becomes the item: each item
    gets its own stream under the Library layout inside its Cloud
    repository's prefix, and the Library page's Push and Pull run the same
    protocol per item (section 6.6). A Library page for one coordinate, say
    Codex pets v2, fetches that coordinate's item repositories and nothing
    else, in one request (section 6.6). No new website pages.

## 4. Layout

Every repository owns the prefix `repos/<repository_id>/`, and under it the
layout of `CURRENT_METADATA_LAYOUT.md` byte for byte, so the desktop's key
builders, the record grammar and the clear order apply unchanged:

```text
repos/<repository_id>/.mallard/_storage.json                                         constant marker, written by the Worker at creation
repos/<repository_id>/.mallard/v1/repositories/<bundle>/_objects/<sha256>            payloads
repos/<repository_id>/.mallard/v1/repositories/<bundle>/_manifests/<rev>-<commit>.json
repos/<repository_id>/.mallard/v1/repositories/<bundle>/_views/<schema>-<sha256>.json  session views (Mallard Cloud only; decision 6)
repos/<repository_id>/.mallard/v1/repositories/_commits/<bundle>/<record name>
repos/<repository_id>/.mallard/v1/repositories/_claims/<bundle>/<parent token | root>.json
```

A catalog repository holds the Library layout for its one item stream instead:

```text
repos/<repository_id>/.mallard/_storage.json
repos/<repository_id>/.mallard/v1/library/<item stream>/_tag.json                    constant bytes
repos/<repository_id>/.mallard/v1/library/<item stream>/_objects/<sha256>
repos/<repository_id>/.mallard/v1/library/<item stream>/_manifests/<rev>-<commit>.json
repos/<repository_id>/.mallard/v1/library/<item stream>/_commits/<record name>
repos/<repository_id>/.mallard/v1/library/<item stream>/_claims/<parent token | root>.json
```

`<bundle>` is the repository's own bundle id and `<item stream>` its own
Library repository id (section 6.6); a key naming another is refused, and a
project repository serves no Library key and a catalog repository no project
key. A removal record stays where it is today (`removals/…`), outside every
repository prefix.

## 5. The service

### 5.1 Routes for the store

| Method | Route | Purpose |
| --- | --- | --- |
| `PUT` | `/v1/repositories/{id}/store/{key}` | One create-only write. `Content-Length` and `X-Mallard-Sha256` required. `200` written, `409 already_present` when the key holds these bytes, `409 claim_taken` with the found bytes in the body for a claim key that holds other bytes, `413` over the class limit, `422 key_invalid`, `422 object_invalid` when the bytes fail the class's validation, `409 quota_exceeded` |
| `GET` | `/v1/repositories/{id}/store/{key}` | The bytes, streamed, with `X-Mallard-Sha256`; `404` when absent. Read from R2, never from the index |
| `GET` | `/v1/repositories/{id}/store?prefix=&cursor=` | The keys under the prefix, at most 1,000 per page, in key order, from one R2 listing. The stream form (`_commits/<bundle>/`) also carries `link` and `publication` (section 5.4). The all-streams prefix is refused |
| `GET` | `/v1/repositories/by-bundle/{bundle_id}` | `{repository_id}` for the caller's own repository first, else a public one; `204` when none. Replaces `by-bundle/{bundle_id}/head` |
| `GET` | `/v1/repositories/by-library/{library_repository_id}` | The same for a catalog repository, by its item stream id (section 6.6) |
| `GET` | `/v1/repositories/library?provider=&resource_kind=&format=&format_version=` | The caller's catalog repositories of exactly that coordinate, each with its item stream's record names and, when small, its selected record and manifest bytes (section 6.6); one request per Library refresh |

Writes need a writer (a collaborator, or anyone signed in on a public
repository); reads need the same, or Super Admin; anyone else gets
`not_found`, as for private content today. No `Idempotency-Key` on store
routes: every key is idempotent by construction. The key in the path is the
desktop's key; the Worker prepends the prefix and never accepts a key outside
the grammar of section 4.

### 5.2 What the Worker checks per key class, and what it indexes

| Key class | Bytes accepted when | Indexed after the write lands |
| --- | --- | --- |
| `_objects/<sha256>` | R2 stores them with `sha256` set to the key's digest, so R2 refuses a mismatch; at most 90 MiB | An `objects` row (repository, digest, size, key, `ready`, created); `committed_bytes` grows once per new object |
| `_views/<schema>-<sha256>.json` | At most 8 MiB; today's session view validator passes; `<schema>` equals the view's `schema_version`; `_objects/<sha256>` exists | An `objects` row with role `view` and `source_sha256` from the key |
| `_manifests/<rev>-<commit>.json` | At most 8 MiB; today's bundle manifest validator passes; `revision`, `commit_id` and `bundle.bundle_id` equal the key's and the repository's; the class binding holds | Nothing yet: a manifest without a record is residue until its record lands |
| `_claims/<bundle>/<parent>.json` | At most 64 KiB; the bytes are a record of that parent (the five graph facts, the parent equal to the key's); create-only | The caller's user id, as custom metadata on the claim object, so the record's pusher is its author even when another account finishes the push |
| `_commits/<bundle>/<record name>` | At most 64 KiB; the name the bytes derive to, facts plus digest, equals the key; **the claim of the record's parent exists and holds exactly these bytes** (decision 2) | Section 5.5: the publication row, its references, the head pointer, the audit row |
| `.mallard/_storage.json` | Never written by a client; the Worker writes it at repository creation | — |
| `library/<stream>/_objects/<sha256>` | As `_objects/`, on a catalog repository whose `library_repository_id` is `<stream>` | An `objects` row |
| `library/<stream>/_manifests/<rev>-<commit>.json` | At most 8 MiB; today's catalog manifest validator passes, with one more binding: `manifest.repository_id` equals the repository's `library_repository_id`; the one entry's coordinate and resource id equal the repository's, as today | Nothing yet |
| `library/<stream>/_tag.json` | The constant tag bytes | — |
| `library/<stream>/_claims/<parent>.json`, `library/<stream>/_commits/<record name>` | As the project classes: the Library record has the same graph facts and name grammar and no summary | As section 5.5, with the summary the catalog validator computes today |
| anything else | `key_invalid` | — |

The record grammar (`<revision:010>.<commit_id>-<sha256>.<parent | root>.<created_at_ms:013>.json`,
`commit-` plus exactly 24 lowercase hex characters) and the selector are one
TypeScript module, `publication.ts`, used by the record and claim checks, by
the indexing of section 5.5 and by the reconciliation job. Today's manifest
validator accepts commit ids of 8 to 128 hex characters; the record grammar
needs 24, and the test helpers that mint ids must produce that form.

### 5.3 The create-only write

The whole plan stands on one primitive: a write the Worker can make that R2
refuses, in the same request, when the key already exists. Step 0 proves it
on the deployed plan before anything else is built, in this order:

1. the Workers binding, `put(key, body, { onlyIf: { etagDoesNotMatch: "*" } })`,
   which must return `null` for an existing key (the reference documents
   `onlyIf` and `etagDoesNotMatch` but not the `*` form);
2. the same call with `onlyIf` given as `Headers({ "If-None-Match": "*" })`,
   which the binding also accepts;
3. failing both, the Worker signs an S3 `PutObject` with `If-None-Match: *`
   against the bucket's S3 endpoint, the exact request the desktop's own R2
   store already relies on; R2's S3 API documents `If-None-Match` as
   supported on `PutObject`.

The same step proves list-after-write on the binding (R2 documents its writes
as strongly consistent) and records which mechanism the Worker uses.

### 5.4 The stream listing carries the status

The desktop reads a repository's Cloud status, the `link` (address,
visibility, release state, permissions) and the `publication` state
(`no_draft`, `not_public`, `matches_published`, `unpublished_draft`, with the
two revisions), once per comparison; today it comes with the head read. After
this plan the one request a refresh makes is the listing of
`_commits/<bundle>/`, so that listing's response carries both. The head in it
is the selection over the names just listed, never the index's pointer; the
released publication, the open request and the permissions come from D1, as
today. Other prefixes list keys only.

### 5.5 Indexing a landed record

When a record `PUT` lands, the Worker, in the same request and after the R2
write, lists the stream, selects, and brings the index up to date:

- a `publications` row for the record: scope `private`, the next private
  sequence, `commit_id`, `manifest_key`, digest and size from the record,
  `manifest_revision` from the record's `revision`, `summary_json` from the
  record's `summary`, `record_name`, `record_sha256`, `created_at_ms`,
  `created_by` from the claim's custom metadata, state `available`;
- `publication_objects` from the manifest the record names (one R2 read of
  up to 8 MiB) and, for every conversation in it, the view row whose
  `source_sha256` is the transcript's digest, when one exists;
- the previous head row retired; `private_publication_id` and
  `private_revision` on the repository; `updated_at`; the open release
  request invalidated; an audit row.

A catalog repository's record is indexed the same way, from its one-entry
manifest, with no view rows.

The response to the `PUT` is `200` once R2 has the record, whether or not the
index batch succeeded: R2 is the truth, and a failed batch is repaired by the
reconciliation job. The batch has no head guard: the claim already decided,
and a row that disagrees with the listing is a bug the job corrects, not a
reason to refuse a record R2 holds.

**Reconciliation.** A cron pass, budgeted like the removal worker, walks the
active repositories: lists `_commits/<bundle>/`, selects, and repairs the
head pointer and any publication row the listing has and the index lacks;
lists the prefix and adds `objects` rows R2 has and the index lacks; recounts
`committed_bytes` when it drifted. The same walk serves a repository on demand
when a summary route finds its head pointer naming a record the listing does
not hold.

### 5.6 What the website reads

The website reads the index for everything it reads today: summaries and
lists, the head, history, release requests, stars. Two things change in what
it fetches from R2: a manifest is fetched by its key (`{rev}-{commit}`), and
a view by the transcript digest the manifest gives, at the current view
schema. `pushed_by` comes from `publications.created_by`, as today. The
public routes serve the current public head's manifest, and the objects and
views `publication_objects` says it references, as today. A catalog
repository's item page reads its head manifest's one entry, as today.

### 5.7 Release, history, the public face

Unchanged in shape. A release request pins the private head's publication
row; the Owner and the Super Admin decide as today; approval inserts a public
publication row referencing the same manifest and objects, retires the
previous public row, and sets `public_admitted_at`. A revision removal marks
the row removed and drops its references; the bytes it alone referenced
become residue. A repository removal deletes the prefix and marks the rows,
as the removal worker does today.

### 5.8 Discard

`POST /v1/repositories/{id}/private-snapshot/discard` queues a removal of
target `chain` and answers at once. The removal worker deletes
`_claims/<bundle>/` until its listing is empty, then `_commits/<bundle>/`
until its listing is empty, within its per-run budget, never touching
`repositories/<bundle>/`; then it retires every private publication row of
the repository, clears the head pointer, and invalidates the open release
request. Listing order is key order and record names sort by depth, so the
first publications go first and every remaining record is disconnected from
the root: a desktop that lists during the job selects nothing, exactly the
state section 12 of the CAS-free plan describes for an interrupted clear. A
Push racing the discard ends in `RemoteChanged` with nothing local moved.

### 5.9 Cleanup and quota

Cleanup keeps today's shape with one rule (decision 7): an object, a view or a
manifest is residue when no `available` or `retired` private publication
whose record is still listed references it, no public publication references
it, and it is older than a day. Retired rows of a discarded chain count while
their records are listed and stop counting once Discard removed them. The
job claims residue in one statement, deletes it, and releases its bytes from
`committed_bytes`, as today. A manifest is residue on the same terms as an
object; nothing erases the manifest of a listed record.

`committed_bytes` grows by every object and view that landed and shrinks by
what cleanup deleted, as today; a `PUT` past 4 GiB answers `quota_exceeded`.

### 5.10 What is removed from the service

The upload routes and the three-step protocol; `uploads` and
`upload_objects`; `objects.upload_id`, the `writing` state and the
per-upload object key; the guard columns `head_changed`,
`upload_in_progress`, `upload_expired`, `object_too_large`,
`object_integrity_failed`; the erasure of retired manifests beyond 32 and its
retention limit; the carry-forward of views at commit; `GET …/head` and
`by-bundle/{id}/head`; the desktop status test fixtures that read them; the
seed script's upload path.

### 5.11 D1 after this plan

Today's tables minus `uploads` and `upload_objects`. `objects` keys are the
layout keys of section 4, unique per repository and digest, with role
`payload` or `view`. `publications` gains `record_name`, `record_sha256` and
`created_at_ms`; `manifest_revision` is the record's depth; `revision` stays
the monotonic private sequence, so a chain that restarts at depth 1 after a
Discard never collides. `repositories` keeps its head pointers, revisions,
`committed_bytes` and `public_admitted_at`. `release_requests`,
`publication_objects`, `removals` (targets `repository`, `revision`,
`chain`), `idempotency`, `audit_events`, `repository_stars` and
`guard_check` with the remaining columns are unchanged.

### 5.12 Limits

Objects 90 MiB, manifests 8 MiB, views 8 MiB or 20,000 events, records and
claims 64 KiB, listing pages 1,000 keys, repositories per account 100, members
per repository 50, quota 4 GiB.

## 6. The desktop

### 6.1 The client

`ManagedCloudClient` gains `put_object(repository_id, key, bytes, sha256)`
answering written, already present, or taken with bytes; `get_object` in the
streaming and the bounded forms; `list_keys(repository_id, prefix, cursor)`,
whose stream form also returns `link` and `publication`;
`repository_by_bundle(bundle_id)`, `repository_by_library(id)` and
`library_coordinate(coordinate)` (the answer of section 6.6). It loses
`create_upload`, `put_object` of
the upload form, `commit_upload`, `abandon_upload`, `head_by_bundle`,
`repository_head`, `repository_status`, `PublicationStatus::observe` and its
tests, `HeadByBundle`, `RepositoryHead`, `OpenUpload`, `UploadRequest`,
`UploadCreated`, `MissingObject`, `CommitResponse`, `InventoryEntry`.
`RepositorySummary` keeps its heads and summaries, which now come from the
index rows of section 5.5, and loses `open_uploads`.

### 6.2 The store

| Engine key | Behaviour |
| --- | --- |
| `.mallard/_storage.json` read | `GET` through the gateway, like every key; the Worker wrote it |
| `_objects/…` write | Staged, as today: views need the manifest |
| `_manifests/…` write | Resolve the repository, or create it from the manifest's class and identity (unchanged); build the views from the staged transcripts; `PUT` every staged object and every view, `cloud_transfer_concurrency()` at a time; then `PUT` the manifest. `already_present` is `AlreadyPresent` |
| `_commits/<bundle>/` listing | `GET …/store?prefix=`, page by page; the status of the bundle is recorded from `link` and `publication` under the existing status lock; a bundle the account has no repository for lists empty |
| `_commits/` listing over every stream | Refused: "Mallard Cloud lists one repository at a time" (decision 10) |
| `_commits/<bundle>/<name>` read, `_claims/<bundle>/<parent>.json` read | `GET` |
| `_claims/<bundle>/<parent>.json` claim | One `PUT`, no transport retry: written is `Mine`; `claim_taken` with bytes equal to this record's is `Mine`, other bytes `Taken`; an unacknowledged request is settled by the shared module's one read, as on S3 |
| `_commits/<bundle>/<name>` record | One `PUT`, no transport retry |
| `_manifests/…`, `_objects/…` reads | `GET`, streamed, digest verified by the shared body reader, unchanged |
| `supports_claims` | `true` |
| `list` with revisions | Forwards to the keys listing with an empty revision: nothing on Mallard Cloud is deleted from the desktop |
| `_uploads/…` | Refused, unchanged |
| `.mallard/v1/library/<stream>/…` | Every class above, routed to the catalog repository whose `library_repository_id` is `<stream>` (section 6.6); a stream the account has no repository for lists empty and reads absent; the manifest write creates the repository from the manifest's one entry |

`PendingUpload`, `head_bytes`, `commit_record`, `enumerate`, the abandon
logic, the `Head` key class and the catalog refusal go. `classify` parses a record or claim key's
file name with the shared grammar. `set_pin`, `status`, `resolve_repository`
and `client` stay as the hooks the application layer uses.

### 6.3 The stub

`stub.rs` becomes a stub of the gateway: per repository, an in-memory map of
key to bytes with a create-only put and a claim answer, the key checks of
section 5.2, an index kept the way section 5.5 keeps it, the listing with
`link` and `publication`, by-bundle resolution, repository creation for
both classes, by-bundle and by-library resolution, the coordinate answer of
section 6.6, the repository list with head summaries, releases, discard as the ordered delete, and fault switches on
the claim put, the record put, a listing, and the index batch after a record
put.

### 6.4 The application layer and the registry

`list_bundle_snapshots_with_engine` branches on `engine.store().managed_cloud()`
and builds the `BundleSnapshotSummary` rows from the repository list: bundle
id, display name, kind, the head summary's revision, updated time, resource
count, registration flag and providers, and the fingerprint (decision 10).
`ManagedCloudAdapter` returns to `StorageProviderRegistry::production()` as
the fourth entry. The pin at engine open, the status read gated on
`managed_cloud_repository_id`, the link commands and the setup lane are as the
wiring plan left them.

### 6.5 What the frontend sees

Nothing new. `ManagedCloudStatus`, `PublicationStatus`, `ManagedCloudLink`
and `ManagedCloudRepository` keep their serialized shapes; the lane, the
settings page, the path-bar chip and the connect dialog switch on the provider
descriptor, which is back in the list. The Push banner prints the shared
sentences, *Finish that push* included, because Mallard Cloud is now a store
with claims. A website Discard reads on every desktop as "Another computer
cleared <storage>. Push again."; no new copy is proposed. The Library page
keeps its DTOs and its look: a row on a managed storage reads, pushes and
pulls as on R2, and the item's address and page are the website's.

### 6.6 The Library on Mallard Cloud

The Cloud keeps one item per repository; the desktop Library publishes one
repository per coordinate with many items. On a managed storage the unit of
publication is therefore the item, and everything below exists to let the
Library's existing engine, plans and page run per item.

- **A stream per item.** `CatalogResourceRepositoryId::for_item(coordinate,
  resource_id)`: `library-` plus 32 hex characters derived, in the digest
  scheme `for_coordinate` uses, from the coordinate and the item's resource
  id. It is the item's Library repository id: its keys live under
  `library/<id>/` in its Cloud repository's prefix, its manifests carry it as
  `repository_id` with exactly one entry, which is what the service admits
  today, and its records and claims follow the shared grammar. The service
  derives the same id in TypeScript, stores it on the catalog repository row
  at creation (`library_repository_id`, checked against the request), binds
  the manifest's `repository_id` to it, and resolves it through `by-library`.
- **Routing.** The store maps `library/<id>/…` to the Cloud repository whose
  `library_repository_id` is `<id>`: from its table, else `by-library`, else,
  at the manifest write, creation from the manifest's one entry (coordinate,
  resource id, the item's display name from its descriptor), as the project
  path creates a repository from a bundle manifest. `_tag.json` reads as the
  constant bytes.
- **Links.** `CatalogItemLink { link_id, scope_id, storage_id, item_repository_id,
  cloud_repository_id, resource_id, accepted_head, accepted_digest, revision,
  created_at, updated_at }` in the catalog's machine-local links document
  beside `CatalogScopeLink`, one per scope, storage and item. A managed
  storage has no scope link. The item link is the per-item form of the scope
  link's accepted head and accepted resource digest.
- **One request per coordinate.** The Library page for a coordinate shows
  that coordinate's local items and that coordinate's remote items, nothing
  else, as it does on R2 where it observes one repository. On a managed
  storage the observation is `GET /v1/repositories/library?…` with the four
  coordinate fields: the Worker selects the caller's catalog repositories of
  that coordinate (an identity query), lists each one's item stream in R2 in
  parallel, and answers per stream the repository id, the resource id, the
  stream id, the record names, and, inline as base64, the selected record's
  bytes and its one-entry manifest's bytes when the manifest is at most 256
  KiB (otherwise omitted, and fetched by key). A stream with more than 1,000
  records is returned truncated and the store lists it the ordinary way. The
  desktop selects from the names with the same selector, verifies the record
  against the digest in its name and the manifest against the record, and
  keeps both in its process caches by token and key, so a refresh with
  nothing changed decodes nothing new. Nothing in the answer comes from the
  index: the names and bytes are R2's, and the repository set is identity.
- **Plans.** `prepare_catalog_scope_push` and `prepare_catalog_scope_pull`
  branch on `store.managed_cloud()`. Rows are per item as today; the remote
  side of every row, linked or not, comes from that one coordinate answer,
  and an unlinked local item the account already holds is matched by its
  resource id. `apply_catalog_scope_push` publishes the selected
  items one at a time in row order: a one-entry manifest on the item's stream
  through the Library engine and the shared protocol (listing, claim, record,
  listing), under the apply lock per item stream; the item's accepted state
  advances on `Stored`; a row that fails stops the apply and names that row,
  and the rows before it stay published, each being its own repository.
  `apply_catalog_scope_pull` fetches each selected item's head manifest,
  verifies its files, and hands the one entry to the existing provider and
  format apply path: skills and pets keep the reviewed-copy behaviour, plugin
  intent keeps its provider or manual actions.
- **Continuity.** Per item link, as the Library does per scope link today: an
  accepted head off the item stream's chain reads as absent, and a stale
  writer ends in the "changed after review" refusal or `RemoteChanged`.
- **Cost.** A Library refresh on a managed storage is one request, whatever
  the number of items, and the same one request when nothing changed. An
  apply adds, per item pushed, the engine's own listing, claim, record and
  listing on that item's stream, which list-then-write requires and no
  cached answer may replace: the store serves the coordinate answer to the
  planning code only, never as a stream listing, so the freshness check of
  the shared module always sees a real listing.
- **Not changed.** The codecs, identities, validation and limits; the website's
  item pages; the release flow per item; the service's one-entry rule.

## 7. What the protocol does on Mallard Cloud

Every row below is the R2 row of the CAS-free plan; the request counts are
the R2 store's plus the views.

| Situation | What happens |
| --- | --- |
| Refresh, nothing changed | One listing. The selected record and its manifest come from the process caches |
| Refresh, a new version | Listing, record, manifest |
| Push | Objects, views and manifest; then listing, claim, record, listing. The Worker indexes the record in the same request, after R2 has it |
| Registration at link time | Listing (empty), manifest write creates the repository and the marker, listing, claim of `root`, record, listing |
| Two computers push on the same version | Both list the same childless head; both claim it; R2 admits one. The other reads `claim_taken` with a record its next listing holds: "The storage changed while pushing. Compare again.", nothing written |
| A writer dies between its claim and its record | The next Push onto that version, from any computer, reads `PushUnfinished` and offers *Finish that push*; the finisher writes the claimed record, and the index credits the claim's author |
| The claim or record request is not acknowledged | The shared module's one read-only check, as on S3; nothing is sent again |
| A Pull review pinned a version, and a push landed since | The apply lists, finds the pinned version still on the chain, and installs exactly it: with claims a listed version has no sibling and cannot be displaced. Its manifest and objects are there because nothing a listed record references is deleted |
| A confirmed Push | Final while claims and records are intact; only Discard removes them |
| Discard on the website | Every desktop's next listing selects nothing: stale bases, "Another computer cleared <storage>. Push again.", next Push at revision 1 re-referencing objects that are already there |
| Repository removed, or access lost | The listing answers `not_found`: an error, not an empty stream |
| No-op Push | One listing proves the compared publication is still selected |
| Discovery | One repository-list request; rows from the index |
| Library refresh of one coordinate | One request; only that coordinate's repositories are touched; nothing decoded when nothing changed |
| Library push of three skills | Three item streams, each: objects and manifest, then listing, claim, record, listing; three Cloud repositories on the website, created on first push |
| Library pull of a skill | The item stream's listing, record and manifest, then the files; the reviewed-copy behaviour as on R2 |
| Two accounts push the same skill on the same version | Same item stream, same parent: one claim wins, the other reads "changed after review" or `RemoteChanged` |
| The index lags R2 | The desktop does not notice: it lists and reads R2. The website may show the previous head until the reconciliation pass |

## 8. Work plan

Each step leaves both trees building and their tests green. The service is
deployed to staging after step 1 and before step 9 runs against it.

| Step | Work | Done when |
| --- | --- | --- |
| 0. Primitive | Section 5.3 on the staging bucket: the create-only write and list-after-write | A note in the service README names the mechanism and the date |
| 1. Service | `publication.ts` (grammar, selector, name derivation); the key grammar and the per-class checks of section 5.2, both classes; the store routes; by-bundle and by-library; `library_repository_id` on catalog repositories and the manifest binding; the schema edits of section 5.11; indexing at the record put and the reconciliation job; views by transcript digest and manifests by key on the content routes; discard as a `chain` removal; the cleanup rule of section 5.9; the removals of section 5.10; tests and a `publisher` test helper that pushes the desktop's way (objects, views, manifest, listing, claim, record); the seed script on that helper | Vitest on Miniflare green; the rows of section 9 marked *service* pass; staging wiped and redeployed |
| 2. Website | Section 10 | The website builds and shows the staging fixtures |
| 3. Client | Section 6.1 | `cargo test -p mallard-infrastructure managed_cloud::client` green |
| 4. Stub | Section 6.3 | The client tests pass against it |
| 5. Store | Section 6.2 | The stub-backed store tests pass |
| 6. Application, registry, parity | The discovery branch; the fourth registry entry; `registry_proofs`; the managed parity test back beside the S3 one; the request-count tests of section 9; the frontend suite run unchanged | `npm run test:integration` green; clippy shows only what was pre-existing |
| 7. Catalog items | Section 6.6: `for_item`, `CatalogItemLink` and its persistence, the managed branches of the four catalog commands, the store's Library routing, the stub's catalog repositories; the Library rows of section 9 | The catalog suites and the catalog end-to-end flow pass on the stub; a Library row on a managed storage pushes and pulls |
| 8. Docs and memory | The architecture doc (sections 4.2, 4.5, 5, 6, 7, 10, 12, 13); `DESIGN_CAS_FREE_STORAGE_REFACTOR.md` section 14 and its per-store table; `CURRENT_METADATA_LAYOUT.md` gains `_views/`; the wiring plan's status; `README.md` offers Mallard Cloud again | The documents agree with the code |
| 9. Staging pass | Section 13, by the owner | Recorded; production deploy is the owner's |

## 9. Tests

| Test | Where | Required observation |
| --- | --- | --- |
| Create-only | service | A second `PUT` of an object, a manifest, a view or a record with the same bytes answers `already_present`; a claim key with other bytes answers `claim_taken` with the found bytes; `committed_bytes` moved once |
| Key grammar | service | A key outside section 4, a bundle that is not the repository's, a Library key, a marker write: `key_invalid` |
| Record needs its claim | service | A record whose parent claim is absent, or holds other bytes, is refused; one whose claim holds these bytes is stored and indexed with the claim's author |
| Record and claim contract | service | A name that does not derive from the bytes, half a parent, a 25-hex commit id, 64 KiB plus one byte: refused |
| Two writers, same parent | service | Two claims of one parent: one `200`, one `claim_taken`; the loser's record is never stored; the index holds one new publication |
| Index follows R2 | service | After a record put the publication row, `publication_objects`, the head pointer and the retired previous row match the listing; with the index batch made to fail, the put still answers `200` and the reconciliation pass repairs every row |
| Index never gates | service | A head pointer set by hand to a wrong record does not refuse the next record put; the reconciliation corrects it |
| Head derivation | service | The selection over a chain, over a stream with an orphan, and over an empty stream; fixture names shared with the desktop's grammar tests select the same head |
| Retention | service | After 40 pushes every listed version's manifest and objects remain; an object only a losing push uploaded is deleted after a day; an object a public release references survives a Discard |
| Release and public face | service | A release pins the head row; the public manifest is that record's; an object added by a later private push is `not_found` on the public face; a removed revision stops serving |
| Discard | service | Claims are deleted before records; a listing taken mid-job selects nothing; manifests, views and objects remain; the index is retired after the deletes; released history still reads |
| Access | service | A non-member on a private repository gets `not_found` from every store route; a signed-in non-member on a public repository reads and writes |
| Refresh makes one request | desktop, stub | An unchanged bundle refreshes with one listing request |
| Push makes listing, claim, record, listing | desktop, stub | The stub's request log shows exactly that around the object, view and manifest puts; the fork-window rows of the CAS-free plan's section 16 marked *claim* pass on the stub |
| Pull applies the pinned version | desktop, stub | A Pull review pinned on version 3; version 4 lands from another account; the apply installs version 3 exactly, and a Compare afterwards reads ahead |
| Unfinished push | desktop, stub | A record put that fails after its claim leaves the claim; the next Push reads `PushUnfinished`; *Finish that push* writes the record once |
| Registration | desktop, stub | Linking creates the repository, the marker and a revision-1 record; two replicas of one account registering at once end with one publication |
| Discard reads as cleared | desktop, stub | After the stub's discard the engine reads no head, the bases are stale, and the next Push publishes revision 1 with every object put answering `already_present` |
| The store never reads the index | desktop, stub | With the stub's index made stale by hand, refresh, Push and Pull behave exactly as with a correct index |
| Discovery | desktop, stub | `list_remote_bundles` returns one row per project repository from the repository list, and the store's all-streams listing is refused |
| Parity | desktop | `managed_cloud_noop_fast_forward_views_and_stale_writer_match_local_protocol` back in `storage_parity.rs` |
| Coordinate answer | service | The route returns only the caller's repositories of the given coordinate, each stream's names in key order, the selected record's bytes hashing to its name, the manifest inline under 256 KiB and omitted above it, and `truncated` past 1,000 names |
| Catalog binding | service | A catalog manifest whose `repository_id` is not the repository's `library_repository_id`, or whose entry names another item, is refused; `by-library` resolves the id |
| Library refresh is one request per coordinate | desktop, stub | The pets v2 page makes one request, which names no skill repository; a second refresh with nothing changed makes one request and reads no record or manifest; a stream over 1,000 records is listed the ordinary way |
| Library push per item | desktop, stub | Three selected skills become three Cloud repositories with one-entry manifests and one record each; the item links hold the accepted heads; a failing third row leaves the first two published and names the third |
| Library pull per item | desktop, stub | A skill pushed by account A is pulled by account B as a reviewed copy; plugin intent resolves through its action |
| Library stale writer | desktop, stub | A push reviewed before another account's push on the same item ends in the "changed after review" refusal, and one that races it ends in `RemoteChanged` with nothing accepted |

## 10. The website

The website keeps reading the index for lists, cards, heads, history and
releases. It changes where it fetches content: a manifest by its key
(`{rev}-{commit}`) instead of by commit id, and a view by the transcript
digest the manifest gives instead of by resource id. `RepositorySummary`
loses `open_uploads`. Pages, components, the session renderer and the
release review are otherwise unchanged. The website is redeployed with the
Worker.

## 11. Safety

- The Worker builds every R2 key from the repository id and a key it
  validated against section 4; a client never reaches another repository's
  prefix or a key outside the grammar.
- Every content write is verified by R2 against the digest in its key, and
  every record and claim against the digest in its name, before it is stored.
- The claim is refused by R2 in the same request; the Worker never emulates
  the refusal with a read followed by a write, and never refuses a record
  because of what D1 says.
- One claim request and one record request per Push, as on every store; no
  transport retry on either.
- The desktop never reads the index, so nothing D1 holds can make a Pull
  install a version other than the one it reviewed.
- Nothing a listed record references is deleted; cleanup deletes residue only,
  after a day.
- Nothing on Mallard Cloud is deleted by a desktop verb. Discard and removal
  are website actions with confirmation, run by the worker in the documented
  order.
- The public face serves only what the released manifest references.

## 12. Not in this plan

- Garbage collection of content a listed record references: none, as on R2.
- The production wipe, deployment, and `super_admin` grant were completed on September 23, 2026, at the owner's request. The grant is for GitHub account `Rika321` (ID `17136646`).

## 13. Staging pass

Run record, September 23, 2026: the owner selected the two live tests as the
initial staging check. `managed_cloud_live_gateway_round_trip` and
`managed_cloud_live_library_round_trip` both passed against
`https://staging.mallard-ai.com`. They used temporary local homes and a new
desktop session. No installed desktop build or real `~/.mallard` was used.
All 15 manual rows below remain untested as written.

Staging version: `6ba4b65d-9b37-4e2b-a53c-c2d249ffa1e3`.
Production version: `c7536a84-f8c4-49e8-88c0-ec56ece71515`.
See `mallardInternal/output/managed-cloud-cutover-2026-09-23/REPORT.md`
for reset, seed, role, and verification results.

Run by the owner with a development build (`MALLARD_MANAGED_CLOUD_ORIGIN`
pointed at staging), on two computers or two accounts, after wiping
`~/.mallard` on both and staging's D1 and R2. Every quoted sentence is what the
app prints; "the prefix" is the repository's prefix in the staging bucket.

| # | Do | Expect |
| --- | --- | --- |
| 1 | A: add Mallard Cloud through the lane, register a project with a few sessions, link it | The prefix holds the marker, one record under `_commits/`, one claim `root.json`; the website shows a registration-only repository |
| 2 | A: Push | "Push complete". The prefix holds objects, views, a manifest, two records and two claims; the website shows the sessions with previews and A's avatar |
| 3 | B: sign in as a second account, accept the invitation code, link the project, Compare, Pull | The Pull installs A's sessions; B's card reads in sync |
| 4 | B: change a session, Push. A: Compare, Pull | Three records, three claims; A reads ahead, then in sync |
| 5 | Both: change a different session, press Push within the same two seconds | One "Push complete", one "The storage changed while pushing. Compare again."; the version both pushed on has exactly one child record and one claim; the loser's Compare reads ahead, Pull, Push: "Push complete" |
| 6 | A: Compare and open the Pull review, then B: Push a change, then A: apply the Pull | A installs the version it reviewed; A's next Compare reads ahead by B's push |
| 7 | A: Push ("Push complete"). In the R2 dashboard delete the newest object under `_commits/` and keep `_claims/`. B: Compare, then Push a change | "A push to Mallard Cloud did not finish. Finish it, then compare again." with **Finish that push**; pressing it: "Finished. Compare again.", the record is back byte for byte and the website credits A |
| 8 | On the website: Discard the private snapshot. A and B: Compare | Both read "Another computer cleared Mallard Cloud. Push again."; the prefix has no claims and no records and still has every object; one Push republishes at revision 1 without re-uploading |
| 9 | A: request a release; the Owner and the Super Admin approve | The public page shows exactly the released snapshot; a further private Push does not change it |
| 10 | A: revoke the desktop session on the website, then Compare | The Reconnect band; sign in again through the lane; Compare works |
| 11 | A: push a session whose transcript is close to 90 MiB | Either "Push complete" or the item error the wiring plan promised, never a truncated object |
| 12 | A: Compare with nothing changed, watching the Worker's request log | One request to the store listing, nothing else |
| 13 | A: on the Library page for Codex skills, push two skills; on the page for Codex pets v2, push one pet | Three new repositories on the website, one page each; each prefix holds `library/<item stream>/` with one record and one claim; the pets page's refresh, in the Worker's log, is one request that names no skill |
| 14 | B: pull one of those skills | The skill lands as a reviewed copy; the website's page is unchanged |
| 15 | A and B: push the same skill on the same version within two seconds | One "Push complete"; the other reads the refusal; one record was added to that item's stream |

Record the outcome in this table, then set the status line of this plan and of
`MANAGED_CLOUD_DESKTOP_WIRING_PLAN.md`.

## Implementation notes (2026-09-23)

What the code does differently from the text above, and why. `STORE_API.md`
in the service tree is the contract both sides implement; where this plan
and that file differ in detail, the file is right.

- **Step 0 held on real R2** (September 23, 2026, a throwaway Worker run with
  `wrangler dev --remote` against the staging bucket under a `probes/` key,
  deleted afterwards): `put(key, body, { sha256, onlyIf: { etagDoesNotMatch:
  "*" } })` writes an absent key and returns `null` for an existing one; the
  `Headers({ "If-None-Match": "*" })` form behaves the same; `head()` reads
  the stored `checksums.sha256` back; a listing taken right after the write
  holds the key. The Worker uses the binding form. The S3 fallback of section
  5.3 was not needed.

- **The marker is read synthesized, not through the gateway** (section
  6.2). A store is opened and its marker verified before any bundle names a
  repository, so there is no prefix to read it from. The Worker writes the
  same constant bytes into every prefix at creation, so the two agree.
- **A taken claim is read back, not returned inline** (section 5.1).
  `claim_taken` carries no bytes; the store reads the claim with one `GET`,
  exactly as the S3 store reads after a refused precondition. A refused
  claim whose stored checksum equals the sent bytes answers `already_present`,
  which the store reports as `Mine`.
- **Views are uploaded after the objects, not with them.** The service admits
  a view only once its transcript is stored, so the store sends every staged
  object first, then every view, then the manifest.
- **A record needs its manifest.** Besides the claim guard, the Worker
  accepts a record only when the manifest it names is stored with the digest
  and size the record states, so the index can always be built from it.
- **Error codes.** Added `already_present`, `claim_taken`, `claim_mismatch`,
  `key_invalid`, `object_invalid` and `absent` (an absent key, which is not an
  error to the desktop, distinct from `not_found`, a repository the account
  may not see); removed `head_changed`, `upload_in_progress`,
  `upload_expired`.
- **`by-bundle` and `by-library` answer `{repository_id}`** and nothing else;
  the desktop's `link` after a release request is built from the repository
  summary (`link_from_summary`), since `/head` is gone.
- **The item link is a map on the scope link** (section 6.6): a managed
  storage keeps one `CatalogScopeLink` per scope and storage, with
  `accepted_items` (resource id → accepted head of the item's stream) beside
  `accepted_resources`, instead of a separate `CatalogItemLink` type. The
  push plan carries `observed_items` and the pull plan `item_heads`; the
  pull plan's `head` is absent on a managed storage. Same schema number;
  the fields are omitted when empty.
- **The item tag carries the resource id.** `LibraryResourceTag` gains an
  optional `resource_id`, serialized only on an item stream, so the tag's
  derivation check can bind the item; `LibraryResourceManifest` accepts a
  one-entry set on the repository derived from its entry.
- **`for_item` derivation:** SHA-256 over length-prefixed fields under the
  domain tag `mallard-library-item-v1`: provider, kind, format, version as 4
  big-endian bytes, resource id; `library-` plus the first 32 hex characters.
- **Nothing is decoded twice only for records.** The coordinate answer primes
  the process publication cache with the names and the selected record; the
  manifest inline is verified and parsed on every refresh (a few kilobytes),
  which is cheap and keeps the object cache out of the Library engine.
- **The Storages page** lists a coordinate's items on a managed storage from
  the same coordinate answer, under the coordinate's repository id as a
  label.
- ***Finish that push*** on a managed storage finishes the unfinished push of
  every item stream of the coordinate that holds one; a coordinate with none
  reads as changed.
- **Discovery** rows come from the account's project repository list; a
  repository whose index has no private head yet is skipped.
- **Local end to end, September 23, 2026.** The Worker on a fresh local
  Miniflare (`wrangler dev`) and the desktop's real store agreed on the wire:
  the ignored tests `managed_cloud_live_gateway_round_trip` (registration,
  no-op, advance, stale writer, listing, discovery) and
  `managed_cloud_live_library_round_trip` (two skills, two item
  repositories, one coordinate request, a Pull on a second machine) pass
  with `MALLARD_LIVE_CLOUD_ORIGIN` and a desktop token, and the local index
  shows one publication row per record with the head at the selected depth.
  The same two tests are the live gate for staging (section 13).

## Source pointers

| What | Where |
| --- | --- |
| The staged write, the head read, the commit, the refused listing | `crates/mallard-infrastructure/src/managed_cloud/store.rs` (`classify`, `begin_upload`, `head_bytes`, `commit_record`, `enumerate`, `impl ObjectStore`) |
| The client's upload, head and status calls | `crates/mallard-infrastructure/src/managed_cloud/client.rs` (`create_upload`, `put_object`, `commit_upload`, `head_by_bundle`, `repository_status`, `PublicationStatus::observe`, `RepositorySummary`) |
| The view builder | `crates/mallard-infrastructure/src/managed_cloud/views.rs` (`build_views_for_manifest`, `BuiltView`) |
| The stub | `crates/mallard-infrastructure/src/managed_cloud/stub.rs` |
| The S3 store's claim, the model for the managed one | `crates/mallard-infrastructure/src/storage_core/s3_store.rs` (`put_claim_async`, `put_absent_async`) |
| The shared protocol the store must satisfy | `crates/mallard-infrastructure/src/storage_core/publication.rs` (`list_stream_at`, `write_record_at`, `claim_once`, `send_once`, `finish_claimed_push`), `storage_core/object_store.rs` (`ObjectStore`) |
| The name grammar and the selector to port | `crates/mallard-domain/src/publication.rs` (`RecordName`, `PublicationToken`, `Mainline::select`) |
| The record's fields and name | `crates/mallard-infrastructure/src/bundle_engine.rs` (`CommitRecord::build`, `CommitRecord::parse`) |
| The Pull apply that pins a version | `src-tauri/src/project_sync/service.rs` (the `snapshot_at(&review.bundle_id, pinned)` apply site and the `mainline.contains` checks around it) |
| The adapter and the registry | `crates/mallard-infrastructure/src/storage_providers/managed_cloud.rs`, `storage_providers/registry.rs` (`production`) |
| The address flow's use of the summary | `src-tauri/src/project_sync/managed_cloud_link.rs` (`describe_repository`) |
| The pin, the status read, discovery | `src-tauri/src/project_sync/service.rs` (`engine_for_storage`'s pin loop, `cloud_status_for`, `list_bundle_snapshots_with_engine`) |
| The parity suite the managed test returns to | `src-tauri/src/sync_tests/thread_variants/storage_parity.rs`, `harness.rs` |
| The Library engine, links, ids and the four catalog commands | `src-tauri/src/storage_catalog/resource_engine.rs` (`LibraryResourceEngine`, `ObservedLibraryHead`), `domain.rs` (`CatalogResourceRepositoryId::for_coordinate`, `CatalogScopeLink`, `CatalogScopeLinksDocument`, `LibraryResourceManifest`), `service.rs` (`resolve_scope_workspace`, `observed_matches`), `resource_commands.rs`, `persistence.rs` |
| The service's one-entry catalog rule | `mallardInternal/cloudflare/managed-cloud/src/validate/catalog.ts` (`validateCatalogManifest`) |
| The clear order Discard follows | `docs/DESIGN_CAS_FREE_STORAGE_REFACTOR.md` section 12 |
| The service schema | `mallardInternal/cloudflare/managed-cloud/migrations/0001_init.sql` |
| The upload routes and the commit batch | `mallardInternal/cloudflare/managed-cloud/src/routes/uploads.ts` |
| The head routes, access and the summary | `mallardInternal/cloudflare/managed-cloud/src/routes/repositories.ts`, `src/db/queries.ts` (`resolveAccess`, `readableHead`, `headBody`) |
| Content routes, discard | `mallardInternal/cloudflare/managed-cloud/src/routes/content.ts`, `src/routes/public.ts` |
| Releases and history | `mallardInternal/cloudflare/managed-cloud/src/routes/releases.ts`, `src/routes/super_admin.ts` |
| The removal worker and the cleanup job | `mallardInternal/cloudflare/managed-cloud/src/jobs/removal.ts`, `src/jobs/cleanup.ts` |
| The validators that stay | `mallardInternal/cloudflare/managed-cloud/src/validate/manifest.ts`, `validate/view.ts`, `validate/limits.ts` |
| The Worker's R2 binding | `mallardInternal/cloudflare/managed-cloud/wrangler.toml` (`OBJECTS`) |
| Website content loading and types | `mallardInternal/cloudflare/managed-cloud/web/src/content.ts`, `web/src/types.ts` |
| Service tests and helpers that change | `mallardInternal/cloudflare/managed-cloud/test/uploads.test.ts`, `desktop-status.test.ts`, `pushed_by.test.ts`, `release.test.ts`, `cleanup.test.ts`, `helpers.ts`, `fixtures.ts` |
