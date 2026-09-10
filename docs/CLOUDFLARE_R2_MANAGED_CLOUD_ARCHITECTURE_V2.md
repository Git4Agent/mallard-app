# Mallard Managed Cloud Architecture V2

## Git Project and Catalog Item plan

- **Status:** Proposed
- **Date:** September 9, 2026
- **Product target:** Managed Cloud V1
- **Scope:** Git projects, skills, plugin install intent, and Codex pets

This document defines architecture V2 for the first Managed Cloud release. It
contains the product rules, portable contracts, service operations, delivery steps,
and acceptance requirements. It can be read without an earlier plan or review.

## 1. Product definition

Mallard Managed Cloud stores reviewed Mallard content so that users can share it and
move it between machines.

The service supports two repository classes:

- Git project repositories for project-scoped Codex and Claude state; and
- Catalog item repositories for one skill, one plugin install intent, or one Codex
  pet.

The Catalog is a view of independent Catalog items. Each item has its own visibility,
membership, invitation, publication, and history. A Project and a Catalog item use
the same access model.

A managed Project or Catalog item is the only access boundary. V1 has no parent
access container.

### 1.1 Fixed decisions

| Question | V1 decision |
| --- | --- |
| Repository classes | Git project or one Catalog item |
| Access boundary | One repository; no parent access container |
| Visibility and roles | Public or private; Admin and Member |
| Anonymous access | Current public snapshot |
| Publication | Reviewed Push, one head, immutable linear history |
| Concurrency | Compare expected commit ID and repository revision |
| Retention | Hard quota; separate controlled repository removal |
| Git source | Remains in the configured Git remote |
| Application content encryption | None |

### 1.2 Included

V1 includes Git admission, reviewed project and Catalog Push and Pull, public
discovery, featured examples, OAuth sign-in, repository invitations, access
management, history, repository removal, quotas, audit, monitoring, and recovery.

### 1.3 Not included

V1 does not include:

- non-Git projects, project-file cloud sync, or Git source hosting and writes;
- parent access containers or shared membership across repositories;
- multi-item Catalog repositories or per-file access settings;
- additional roles, approval queues, multiple heads, or automatic merge;
- plugin binaries, marketplace credentials, Claude pets, or pet format conversion;
- removal of individual accepted resources;
- a web client, public slugs, or a service export operation;
- billing, shared public object caching, direct R2 uploads, or cross-repository deduplication; or
- client-side or end-to-end content encryption.

Future features require a new reviewed design. V1 has no dormant service fields
for them.

## 2. Repository and access model

### 2.1 Repository classes

| Class | Required identity | Portable content |
| --- | --- | --- |
| `git_project` | Bundle ID and Git repository fingerprint | Project conversations and supported provider state |
| `catalog` | Provider, resource kind, format, format version, and resource ID | One skill, plugin intent, or pet package |

Repository class and bound identity cannot change after creation. A Catalog item
has no project root. It does not make a non-Git project eligible for managed sync.

The service repository ID is opaque. It is distinct from portable bundle and
Catalog resource IDs. Portable identity grants no access and cannot prove that a
private repository exists.

### 2.2 Supported Catalog formats

V1 supports Codex and Claude skills and plugin install intent, plus Codex pet v1
and v2. Pet versions remain separate items and UI choices.

The current [Catalog registry](../src-tauri/src/storage_catalog/registry.rs) defines
serialized provider/kind/format/version coordinates. The service rejects unlisted
coordinates. Generate shared contract fixtures from that registry and its codecs.
Adding a registered format requires matching service validation before managed
publication is enabled.

### 2.3 Visibility

| Visibility | Anonymous read | Member read | Publish |
| --- | --- | --- | --- |
| Public | Current accepted snapshot | Yes | Active Admin or Member |
| Private | No | Yes | Active Admin or Member |

These access rules apply to active repositories. Pending removal and erasure use
the restrictions in section 2.8.

Visibility applies to one complete Project or one complete Catalog item. All files
and objects in an item inherit its visibility. Other Catalog items can use a different
visibility.

Anonymous public access does not include:

- retained history;
- unaccepted uploads;
- invitations;
- membership data;
- audit events;
- local paths;
- provider-home identity; or
- service administration data.

Every public read checks current D1 metadata and returns `Cache-Control: no-store`.

### 2.4 Roles

| Capability | Public reader | Member | Admin |
| --- | --- | --- | --- |
| Read current public snapshot | Yes | Yes | Yes |
| Read private snapshot | No | Yes | Yes |
| Read retained history | No | Yes | Yes |
| Publish after review | No | Yes | Yes |
| Leave the repository | No | Yes | Yes |
| Invite or remove a Member | No | No | Yes |
| Promote or demote an Admin | No | No | Yes |
| Change name or visibility | No | No | Yes |
| Request or cancel repository removal | No | No | Yes |

Rules:

- `admin` and `member` are the only role values.
- The creator becomes the first Admin.
- Each active repository and each repository pending removal keeps at least one
  active Admin.
- An invitation grants Member access.
- An Admin can promote an accepted Member.
- A role applies to one complete Project or Catalog item.
- Public read does not create membership.
- Admin status does not bypass review, integrity, or concurrency checks.

### 2.5 Invitations

An Admin can invite a Member to a public or private Project or Catalog item.

The invitation stores:

- service repository ID;
- HMAC-SHA-256 of the normalized verified email;
- masked email label;
- random-token hash;
- creator ID;
- creation and expiry times;
- state; and
- accepted account ID when complete.

The raw token exists only in the invitation message. Acceptance requires a signed-in
account with the matching verified email, an active invitation, and an unused token.

### 2.6 Visibility changes

Creation starts private. To make a repository public, an Admin reviews the complete
accepted snapshot and submits its commit ID, manifest digest, expected repository
revision, warning acknowledgements, and explicit public-content acknowledgement.
The route requires recent Admin authentication. Its transaction checks these inputs
against current state before changing visibility and recording the audit event.

Changing visibility to private increments the repository revision before returning.
There is no shared public object cache to purge. Downloaded copies cannot be revoked.

### 2.7 No resource removal through sync

Project Push can add a resource or replace content for the same stable identity.
The candidate must retain every accepted resource that it does not replace.
Catalog Push can replace only its bound item; an empty item is invalid.
Neither class accepts tombstones. Pull can create or update local resources, but
cannot delete them. Removing a Member does not change content.

Accepted history remains within a hard quota until the whole repository is removed.
A full repository stays readable. A publication that needs more quota is rejected.
Removal preview, request, status, and cancellation require no free content quota.

### 2.8 Controlled repository removal

Removal covers one complete repository: its current snapshot, history, and uploads.
It never deletes local files, provider resources, Git checkouts, Git remotes, or
another managed repository.

~~~text
active -> removal_pending -> deleting -> removed
             |
             `-> active (private, after cancellation)
~~~

An Admin reads a preview containing the repository ID, display name, revision,
current commit ID or absence, retained bytes, and cancellation terms. The request
confirms the name and expected revision and commit ID, with an idempotency key.
One D1 transaction checks Admin access and these values, starts the removal,
increments the revision, makes the repository private, and revokes invitations.
It closes upload admission and records a new removal ID, deadline, and audit event.

From that commit, content reads and ordinary mutations are denied. Existing uploads
cannot publish. Admins can use the same removal route for status or cancellation.
Cancellation is available for seven days. It checks the expected repository
revision, increments it, restores active private access, and invalidates the pending
removal.
Invitations and old uploads do not resume. Publishing public content again requires
the visibility review in section 2.6.

After the deadline, the worker atomically claims the pending removal by state and
removal ID. Cancellation and this claim cannot both succeed. A delayed job for a
cancelled request cannot claim a later request. Before deletion, the worker writes
and verifies one immutable removal record outside D1, at the key in section 6.3.
The record holds the repository ID, removal ID, authorizing Admin ID, and claim time.
It stays outside the repository's deletion scope and prevents recovery from restoring
access. An uncertain record write blocks erasure.

The worker finishes or aborts tracked writes, then deletes exact repository-owned
keys in bounded, retryable batches. It verifies erasure against the object inventory.
Uncertain writes or deletes keep the state at `deleting` and raise an alert.
A final D1 transaction removes content metadata, releases committed usage and any
remaining reservations once, and marks the repository `removed`. Keep minimum
Admin authorization, removal status, and audit metadata for the existing status
route. Repository IDs and physical keys are never reused.

## 3. Common repository protocol

### 3.1 One current head

Each repository has zero or one current head. Its API envelope contains the service
repository ID and class, repository revision, publication revision, commit ID,
manifest SHA-256 and byte length, and update time.

Publication compares `expected_commit_id` with the current commit ID; null means no
head. Publication revisions increase along the immutable chain, and commit IDs
cannot recur. `expected_repository_revision` separately protects metadata and access
changes. The managed API needs no additional concurrency token.

Only the committed D1 head change accepts a publication. Service identity and
repository revision stay in the API envelope. The desktop adapter maps portable
head fields to the class-specific engine. For `CasExpectation::Match`, it uses the
commit ID as the comparison value.

### 3.2 Immutable history

Each successful publication creates:

- immutable content-addressed payloads;
- one immutable manifest; and
- one immutable commit that references the previous commit.

The commit chain is linear. While the repository is active, Admins and Members can
read retained history. Anonymous users can read only the current public snapshot.
History is immutable until the separate whole-repository erasure operation.

### 3.3 Concurrent publication

Two publications can compare against the same commit ID. The first valid transaction
wins. The other receives `head_changed` and must read the new snapshot and obtain
new approval. The service does not combine concurrent publications.

### 3.4 Shared storage properties

Project and Catalog item repositories use the same immutable object transport. They
use different closed manifest validators.

The shared properties are:

- schema 1 documents;
- UTF-8 JSON;
- bounded fields and collections;
- integer Unix timestamps in seconds, as serialized by the Rust DTOs;
- lowercase SHA-256 values;
- content-addressed payloads;
- create-only immutable writes;
- exact byte-length checks; and
- one compare-and-swap head.

Use the current Rust field names, numeric types, enum encodings, optional fields,
and canonical digest rules. Do not replace integer timestamps with strings or round
integer values in the Worker. SHA-256 verification uses the received immutable
bytes. Canonical content digests use the same field order and ordered collections
as the Rust encoder.

Project fixtures come from `BundleManifest`, `BundleHead`, `BundleScope`, and
`ConversationThread` in the [Rust domain](../crates/mallard-domain/src/lib.rs).
Catalog fixtures use `CatalogResourceEnvelope` and `PublishedResourceEntry`;
the managed item container is defined in section 5.5.
The desktop and service must share valid and invalid serialization fixtures. Test
required fields, enum values, omitted values, integer boundaries, and digest
calculation. This is one current contract, with no alternate legacy encoding.

### 3.5 Managed desktop binding

One local managed binding represents one service repository. It contains:

- service environment and URL;
- service repository ID;
- repository class;
- expected portable project or Catalog item identity;
- for a Catalog item, its machine-local provider scope and resource target; and
- operating-system keychain session reference.

The binding contains no R2 credential. Local scope IDs, provider-home paths, resource
targets, and profile IDs do not enter portable content. Distinct service repositories
use distinct local bindings.

### 3.6 Managed publication boundary

A dedicated desktop adapter passes publication context to the HTTPS API. Generic
object-key operations carry immutable bytes; account policy and approval stay in
the managed operation.

After Apply validates the candidate, the upload request carries its manifest,
selected resource identities, approved action types, approval digest, warning
acknowledgements, and approval expiry. It also supplies the repository class,
expected repository revision, expected commit ID, and reviewed visibility. Public
Push includes acknowledgement of the exact complete candidate snapshot.
The approval digest covers the manifest digest and these review fields, using the
shared contract fixtures.

The service binds those immutable inputs to one upload session and its authenticated
actor. Its expiry cannot exceed the approval expiry. This is the server's approval
record. The service validates content and acknowledgements; human review remains a
desktop responsibility. Local targets, provider-home bindings, executable actions,
backups, and receipts stay on the machine.

## 4. Git project repository contract

### 4.1 Admission

The desktop admits a project only when it can:

- resolve the canonical Git root;
- read a usable `origin` URL;
- normalize the URL without user information;
- compute the repository fingerprint;
- read the current Git commit ID; and
- confirm that project state is ready for review.

The repository fingerprint is SHA-256 of the normalized `origin` URL. The raw URL does
not enter managed content.

The fingerprint is not globally unique in D1. This prevents private-repository
existence from being inferred through a uniqueness error.

### 4.2 Source boundary

Managed Cloud stores Mallard state for a Git project. It does not store the project's
Git-managed source files.

Portable project content can include:

- selected Codex conversations;
- selected Claude conversations;
- supported project-scoped provider state;
- stable portable resource identity;
- recorded conversation Git context; and
- privacy-preserving publication origin.

Portable project content cannot include:

- Git source files;
- complete provider homes;
- credentials;
- local checkout paths;
- provider-home paths;
- device IDs;
- local backup paths;
- trust decisions;
- service IDs; or
- temporary files.

### 4.3 Project manifest

The immutable project manifest uses the current `BundleManifest` shape:

| Field | Content |
| --- | --- |
| `schema_version` | Portable bundle schema 1 |
| `revision`, `commit_id` | Publication revision and bundle commit ID |
| `updated_at` | Integer Unix timestamp in seconds |
| `bundle` | `bundle_id`, `display_name`, `kind`, and `repository_fingerprint` |
| `published_by_origin` | Portable publication origin |
| `scopes` | Map of typed sync scopes |

Managed Git projects require `bundle.kind = project` and a fingerprint that matches
the service binding. The current Git commit ID is an admission input. It is not a
new top-level manifest field. Recorded conversation Git context keeps its existing
serialized form.

Each scope has `revision`, `content_sha256`, `recipe`, `captured_with`,
`conversations`, `resources`, `files`, `directories`, and `tombstones`. The service
validates the canonical scope digest, recipe-to-resource identity, and references
within the scope. Managed Cloud rejects project-file content and applies the
retention rule in section 2.7.

Conversation entries keep the current typed identity, presentation, file inventory,
lineage and semantic digests, message count, portable tail, declared wire rules,
provenance, and timestamps. Preserve supported child-session and sidecar references
through the same file and conversation contracts. Do not invent a reduced cloud
conversation DTO. Required wire fields and references must pass the same validators
and serialization fixtures as the desktop.

### 4.4 Project Push review

Compare reads the managed head and manifest, discovers local resources, and compares
local observations, accepted bases, and managed state. It shows additions, changes,
retained items, conflicts, and warnings. It does not capture an upload package or
build executable conversation actions. Refresh and Push comparison use manifest
metadata without downloading conversation payloads.

The user selects resource identities and permitted actions. The review context pins
the local binding revision, repository revision, visibility, expected commit ID,
acknowledgements, and expiry. It remains machine-local.

When the user selects Apply, the desktop:

1. rechecks the review context and current binding;
2. discovers current local state and captures the selected resources;
3. builds current typed conversation actions and proves their allowed effects;
4. matches them to the approved resource identities and action types;
5. blocks changed or newly warned actions that the approval does not cover;
6. builds and validates the portable candidate manifest and object inventory;
7. creates the upload with the approval fields from section 3.6;
8. uploads the required immutable bytes; and
9. publishes against the reviewed commit ID and repository revision.

An item failure is reported for that item. Valid selected items can proceed. A
failed item keeps its accepted cloud content. New warnings or a change outside the
approved effect require another review. Public publication also requires disclosure
approval for the exact complete candidate snapshot before upload or publication.

### 4.5 Project Pull review

Compare reads and verifies the authorized head and manifest, then compares managed
content with local observations and accepted bases. It shows proposed creates,
updates, preserved items, conflicts, warnings, and manual actions. It can verify
payloads needed to determine whether a resource is eligible for Pull. The result is
a review context, not a persisted executable conversation plan.

The user selects resource identities and action types. Apply rechecks expiry,
binding revision, repository revision, head, and manifest. It then fetches and
verifies the selected objects, reads current local targets, and builds fresh typed
actions. Approved selections match by resource identity and action type. An action
that no longer matches is blocked; unrelated approved actions can proceed.

Immediately before a local write, Apply verifies the fresh target digest and other
action pins, creates the required backup, and uses atomic replacement where
supported. Receipts advance accepted bases only after success. Review context,
executable actions, backups, and receipts stay machine-local. Project conversation
Apply does not introduce a persisted executable plan or a recovery journal.

## 5. Catalog item repository contract

### 5.1 Item identity and isolation adapter

The stable item identity is:

~~~text
(provider, resource_kind, format, format_version, resource_id)
~~~

The Catalog codec derives `resource_id` from the coordinate and canonical identity.
The managed binding maps that item to one local provider/format scope and target.
Local profile IDs, homes, scope IDs, storage IDs, and accepted bases stay local.

The current `LibraryResourceManifest.resources` map can contain several items.
The managed adapter must construct an item publication from the selected resource;
it cannot forward the Library manifest.

The adapter:

1. resolves the selected resource ID in the exact local scope;
2. obtains its current `CatalogResourceEnvelope` and `PublishedResourceEntry`;
3. checks coordinate and identity against the service binding;
4. builds the single-entry container in section 5.5; and
5. includes only referenced payload digests, lengths, and bytes.

The service independently checks this projection. It rejects sibling entries,
foreign identities, unreferenced objects, and references outside this repository's
verified inventory. For Pull, the adapter passes the verified envelope and payloads
to the existing provider/format apply path. It does not replace a Library resource set.

Existing local-folder and user-owned S3/R2 Catalog protocols remain unchanged.

### 5.2 Skills

Use the existing [skill codec](../src-tauri/src/storage_catalog/agent_skill.rs) for
identity, supported scopes, file validation, and limits. The package must include
`SKILL.md` and pass bounded path, file-type, collision, and integrity checks.
Executable files require explicit warning acknowledgement.

### 5.3 Plugin install intent

The [plugin-intent codec](../src-tauri/src/storage_catalog/plugin_intent.rs) defines
portable identity, marketplace source validation, and resolution policy. Managed
storage carries semantic installation intent. Plugin code, credentials, local
installation state, and command strings are excluded.

Pull uses a validated provider operation when the source can be resolved safely.
Otherwise, it returns a manual action. The execution boundary is in section 9.5.

### 5.4 Codex pets

Use the [pet codec](../src-tauri/src/storage_catalog/codex_pet.rs) for versioned
identity, atlas dimensions, image formats, safe basenames, byte limits, and digest
checks. Preserve the selected format version. Managed Cloud performs no conversion.

### 5.5 Catalog item manifest

The managed container has these fields:

| Field | Content |
| --- | --- |
| `schema_version` | Managed Catalog item schema 1 |
| `revision`, `commit_id` | Item publication revision and commit ID |
| `updated_at` | Integer Unix timestamp in seconds |
| `captured_with` | Existing `CatalogCapturedWith` |
| `published_by_origin` | Existing `PortableCatalogOrigin` |
| `entry` | One existing `PublishedResourceEntry` |

Reuse the entry and resource envelope from the
[Catalog domain](../src-tauri/src/storage_catalog/domain.rs), with the registered
codec's identity, descriptor, file-reference, and canonical-digest validators.
The container adds no Library repository ID or `resources` map. The service ID
stays in the API envelope. Publication metadata is specific to this item.

The entry's coordinate and derived resource ID must match the binding. A change to
identity requires a new repository. Public access exposes the complete current
manifest and its referenced objects.

### 5.6 Catalog review differences

Catalog uses the approval and transfer sequence in sections 4.4 and 4.5 with its
existing typed item plans and journaled apply. Unlike project conversations, its
inventory captures the complete package during review. Apply rechecks the exact
resource, warnings, scope, target, and managed head before using the adapter in
section 5.1. The review shows provider, kind, format version, identity, and effect.

An item with no compatible local scope remains view-only until the user selects one.
Skills and pets can preserve divergent local content as a reviewed copy. Plugin
intent cannot be copied; its conflict requires a provider operation or manual action.

## 6. Cloud architecture

### 6.1 Topology

~~~text
Mallard desktop
  |
  | HTTPS with OAuth access token
  v
Cloudflare Worker API
  |-- D1: identity, access, metadata, heads, uploads, quota, audit
  |-- R2: private immutable manifests, commits, payloads, removal records
  |-- Queue and Cron: expiry, cleanup, reconciliation, notification retry
  `-- Email provider: invitation messages
~~~

Every read and write passes through the Worker. The desktop receives no R2 or D1
credentials, administration tokens, or internal object keys.

### 6.2 Component ownership

The Worker owns authorization, class-specific validation, publication, read access,
invitations, removal, quotas, bounded errors, and audit.

D1 is authoritative for identity, access, metadata, heads, accepted history,
quota counters, upload state, idempotency, and audit metadata. R2 holds immutable
bytes at exact internal keys. An R2 listing grants no access and cannot select a head.

### 6.3 Internal R2 keys

~~~text
repos/<repository-id>/objects/<sha256>/<upload-id>
repos/<repository-id>/manifests/<revision>-<commit-id>.json
repos/<repository-id>/commits/<revision>-<commit-id>.json
removals/<repository-id>/<removal-id>.json
~~~

The Worker builds keys from validated identifiers. Clients submit logical object
references. D1 identifies a physical payload by `(repository_id, sha256, upload_id)`.
An upload ID is never reused. Existing verified payloads can be referenced using
their original upload ID within the same repository. A new upload after deletion
uses a new key, so a delayed delete cannot affect the replacement.

Manifest and commit keys are also immutable and never reused after cleanup claims
them. Unaccepted bytes use the same layout.

### 6.4 Upload and publication transaction

Publication has three steps:

1. **Create upload.** Submit the candidate and approval fields from section 3.6.
   The Worker derives the required revision and previous-commit link from the
   expected head. It validates the candidate against them and pins manifest/commit
   digests and the exact object inventory. It reserves quota in D1 and returns an
   upload ID, expiry, and missing-object list.
2. **Send objects.** Use one PUT per missing payload. Register each write before
   R2 I/O, stream with create-only semantics, and verify SHA-256 and length. Only
   verified writes become available. `upload_objects` protects the fixed reference
   set while the session is active.
3. **Publish.** Revalidate the candidate and verified inventory in memory, write and
   verify immutable manifest/commit bytes, then execute the final D1 batch.
   No R2 I/O or content parsing occurs in the batch.

Finalization uses one D1 batch to commit publication, head, references, quota
counters, committed upload, idempotent result, and success audit event. Guards read
transaction-time access, lifecycle, repository revision, visibility, expected commit,
upload expiry, immutable candidate, verified references, acknowledgements, and quota.
Any failed guard, including a head update that matches zero rows, aborts the whole
batch. Failure leaves accepted state and the upload's approval and reservation
unchanged; a separate expiry operation can later release the reservation. See
[D1 batch transaction semantics](https://developers.cloudflare.com/d1/worker-api/d1-database/).

Upload inputs cannot change after creation. Manifest and commit preparation has the
same tracked-write and cleanup protection as payloads. A successful R2 write never
accepts publication. Retry handling follows section 6.6; the concurrency contract
is in section 3.3. Slice 0 selects the SQL guard or coordination implementation.

### 6.5 Read path

The Worker authorizes an active repository, resolves the requested publication and
logical object reference through D1, then streams the exact R2 bytes with length and
digest. The desktop independently verifies SHA-256 and byte length.

`publication_objects` maps publications to exact physical objects. Anonymous reads
must name the current public head. Member history reads must name an accepted
publication. An object can belong to several publications; its mere presence or
hash cannot grant access.

### 6.6 Idempotency

Idempotency handles a request whose server result is committed but whose response is
lost.

The desktop sends one random key for one logical operation. D1 stores actor,
operation, key hash, canonical request hash, result, and expiry.

| Retry | Result |
| --- | --- |
| Same key and same request | Return the recorded result |
| Same key and different request | Return `idempotency_mismatch` |

V1 requires idempotency only for:

- repository creation;
- invitation creation and delivery;
- upload-session creation;
- publication;
- repository-removal requests; and
- removal cancellation.

Compare-and-swap protects against different concurrent publications. Idempotency
protects against a retry of the same publication.

Reauthorize before returning a stored result. A committed upload remains terminal
after its idempotency record expires. Use its immutable inputs and publication link
to return the matching result without another quota change. Expired or failed
uploads cannot be reactivated.

### 6.7 Quota and cleanup

Limits cover repositories, accepted bytes, object counts and sizes, concurrent
uploads, manifest and resource bounds, audit retention, and request rates.

Store `reserved_bytes` on each upload and `committed_bytes` on the repository and
charged account. Admission checks committed usage plus active reservations in one
D1 transaction. Publication converts only newly accepted physical bytes and metadata
to committed usage; reused objects add no charge. Expiry releases its reservation
once. Transactional state changes and idempotency protect all counter updates.

Ordinary cleanup removes expired invitations, uncommitted expired sessions, and
bytes never accepted by a publication. Accepted references remain protected under
section 2.7.

1. In D1, claim an exact object as `deleting` only when it has no accepted reference,
   active upload reference, or unresolved write. Expire a session and release its
   reservation atomically before dropping its protection. A timeout alone does not
   prove that an R2 write ended.
2. Publication and upload reuse must reject objects already claimed for deletion.
   Publication checks this again within its transaction.
3. Delete the claimed key and record the result. An uncertain result keeps the claim
   for retry at that exact key. A replacement uses a new upload ID.

If publication commits first, cleanup cannot claim its objects. If cleanup claims
first, publication cannot accept those objects. Apply the same protection to
unaccepted manifests and commits.

D1 records pending cleanup and removal work before Queue delivery. Cron resumes
missed jobs. Reconciliation checks missing or untracked bytes, expired reservations,
counter totals, and disagreement between heads and stored data.

## 7. D1 model

### 7.1 Tables

| Table | Purpose |
| --- | --- |
| `users`, `identities`, `refresh_sessions` | Accounts, charged usage, identity, and sessions; session labels supply the device list |
| `repositories` | Service ID, class, name, visibility, revision, lifecycle, committed bytes, featured flag |
| `project_repository_bindings` | Bundle ID and Git fingerprint |
| `catalog_repository_bindings` | Provider, resource kind, format, format version, and resource ID |
| `repository_heads` | Current publication pointer |
| `repository_publications` | Immutable linear history |
| `publication_objects` | Publication-to-physical-object references |
| `repository_members` | Repository Admin or Member |
| `repository_invitations` | Email HMAC, masked label, token hash, expiry, state |
| `objects` | Physical key identity, verified digest and length, lifecycle, write and deletion status |
| `upload_sessions`, `upload_objects` | Immutable candidate, actor, approval fields, references, reserved bytes, expiry, tracked writes |
| `idempotency_records` | Mutation request digest and result |
| `repository_removals` | Removal ID, confirmation, deadline, work progress, completion status |
| `audit_events` | Sanitized security history |

### 7.2 Closed values

| Field | Values |
| --- | --- |
| User state | `active`, `suspended` |
| Repository class | `git_project`, `catalog` |
| Visibility | `public`, `private` |
| Repository state | `active`, `removal_pending`, `deleting`, `removed` |
| Catalog coordinate | Supported registry entries from section 2.2 |
| Role | `admin`, `member` |
| Membership state | `active`, `suspended`, `removed` |
| Invitation state | `pending`, `accepted`, `revoked`, `expired` |
| Upload state | `created`, `receiving`, `committed`, `expired`, `failed` |
| Object state | `writing`, `ready`, `deleting`, `deleted` |

Reachability comes from publication references. Upload protection comes from upload
references and active session state. Unresolved writes retain protection after expiry.

### 7.3 Schema constraints

Use unique keys, foreign keys, and value checks for:

- identity provider and subject;
- repository binding identity and class;
- membership by repository and user;
- one head row per repository, referencing a publication in that repository;
- unique publication revision and commit ID per repository, and one publication per upload;
- physical object identity `(repository_id, sha256, upload_id)` and references within that repository;
- one idempotent result per actor, operation, and key; and
- closed states and non-negative quota values.

Transactional policy checks remain in their owning sections.

### 7.4 Atomic operations

| Operation | One transaction contains |
| --- | --- |
| Create repository | Private repository, class binding, creator Admin, quota admission, idempotency, audit |
| Accept invitation | Membership, consumed invitation, repository revision, audit |
| Change membership | Role or state, checks from section 2.4, repository revision, audit |
| Change visibility | Inputs from section 2.6, visibility, repository revision, audit |
| Create or expire upload | Session, object references, reservation, idempotency where applicable |
| Publish | Guards and effects in section 6.4 |
| Claim cleanup | Reference and write checks in section 6.7, deletion claim |
| Request, cancel, claim, or complete removal | Corresponding lifecycle transition and effects in section 2.8 |

## 8. Service API

The base path is `/v1`. JSON uses `snake_case`. Private reads and all writes require a
bearer access token. Mutations include an expected repository revision where state can
change. Operations listed in section 6.6 also include `Idempotency-Key`.

### 8.1 Identity and public routes

| Method | Route | Purpose |
| --- | --- | --- |
| `POST` | `/v1/auth/start` | Start OAuth with PKCE |
| `POST` | `/v1/auth/callback` | Complete one callback |
| `POST` | `/v1/auth/refresh` | Rotate refresh token |
| `POST` | `/v1/auth/logout` | Revoke current session |
| `GET` | `/v1/me` | Account, sessions, repositories, and limits |
| `GET` | `/v1/public/repositories` | Filtered public repository listing |
| `GET` | `/v1/public/repositories/{repository_id}` | Public summary and repository class |
| `GET` | `/v1/public/repositories/{repository_id}/head` | Current public head |
| `GET` | `/v1/public/repositories/{repository_id}/manifest/{commit_id}` | Current public manifest |
| `GET` | `/v1/public/repositories/{repository_id}/objects/{sha256}?commit_id={commit_id}` | Current public object |

Public listing filters can include repository class, provider, and Catalog item kind.
The authenticated repository list can also filter by visibility. These filters create
the Public and Private Catalog views.

### 8.2 Repository and access routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET`, `POST` | `/v1/repositories` | List memberships or create a private repository |
| `GET`, `PATCH` | `/v1/repositories/{repository_id}` | Read metadata or change the display name |
| `POST` | `/v1/repositories/{repository_id}/visibility` | Make public or private |
| `GET` | `/v1/repositories/{repository_id}/members` | Active and invited access |
| `POST` | `/v1/repositories/{repository_id}/invitations` | Invite Member |
| `POST` | `/v1/repositories/{repository_id}/invitations/{invitation_id}/revoke` | Revoke invitation |
| `POST` | `/v1/invitations/accept` | Accept token from request body |
| `PATCH`, `DELETE` | `/v1/repositories/{repository_id}/members/{user_id}` | Change or remove membership |
| `GET` | `/v1/repositories/{repository_id}/removal` | Admin removal preview or operation status |
| `POST` | `/v1/repositories/{repository_id}/removal` | Confirm and schedule removal of the complete repository |
| `POST` | `/v1/repositories/{repository_id}/removal/cancel` | Cancel during the recovery period |

Project repository creation accepts:

~~~json
{
  "repository_class": "git_project",
  "display_name": "Mallard Desktop",
  "bundle_id": "bundle_opaque",
  "repository_fingerprint": "sha256_hex"
}
~~~

Catalog item repository creation accepts:

~~~json
{
  "repository_class": "catalog",
  "display_name": "Rika",
  "provider": "codex",
  "resource_kind": "desktop_pet",
  "format": "openai.codex-pet",
  "format_version": 2,
  "resource_id": "resource-0000000000000000000000000000000000000000000000000000000000000000"
}
~~~

The example resource ID is illustrative. The desktop supplies the ID derived by the
Catalog codec from the full coordinate and item identity. The service validates and
stores that binding. Catalog item creation is private even when the requested name
matches a public item. Invitation acceptance sends the raw token in the request body.
Raw invitation tokens do not enter URLs or logs.

Membership DELETE permits self-removal or Admin removal of another user, under
section 2.4. It also serves the Leave action.

Removal request fields are `expected_repository_revision`, `expected_commit_id`
(null for no head), and `confirmed_display_name`. Cancellation sends the current
repository revision. Both use `Idempotency-Key` and normal Admin authentication.
Status uses existing Admin authorization and returns lifecycle state, cancellation
deadline, and completion time when available. The lifecycle is defined in section 2.8.

### 8.3 Content routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/v1/repositories/{repository_id}/head` | Current head |
| `GET` | `/v1/repositories/{repository_id}/history` | Retained history |
| `GET` | `/v1/repositories/{repository_id}/publications/{commit_id}` | Publication metadata |
| `GET` | `/v1/repositories/{repository_id}/manifests/{commit_id}` | Authorized manifest |
| `POST` | `/v1/repositories/{repository_id}/uploads` | Create upload with candidate and approval fields |
| `PUT` | `/v1/repositories/{repository_id}/uploads/{upload_id}/objects/{sha256}` | Upload expected payload |
| `POST` | `/v1/repositories/{repository_id}/publish` | Validate and publish the upload by head comparison |
| `GET` | `/v1/repositories/{repository_id}/objects/{sha256}?commit_id={commit_id}` | Read authorized payload |

The upload body is defined in section 3.6. A Catalog request uses the projection in
section 5.1. Publish sends the upload ID and its own `Idempotency-Key`; it cannot
replace the candidate or approval fields. Section 6.4 defines all three steps.

### 8.4 Stable errors

Stable error codes are:

- `authentication_required`;
- `recent_authentication_required`;
- `not_found`;
- `forbidden`;
- `invalid_request`;
- `unsupported_repository_class`;
- `non_git_project`;
- `repository_fingerprint_mismatch`;
- `provider_kind_mismatch`;
- `catalog_item_identity_mismatch`;
- `manifest_kind_mismatch`;
- `resource_removal_not_supported`;
- `revision_conflict`;
- `head_changed`;
- `repository_removal_pending`;
- `removal_cannot_cancel`;
- `last_admin`;
- `invitation_invalid`;
- `invitation_expired`;
- `quota_exceeded`;
- `object_integrity_failed`;
- `upload_expired`;
- `warning_acknowledgement_required`;
- `idempotency_mismatch`; and
- `temporarily_unavailable`.

Errors do not reveal private existence, raw email, content, local paths, credentials,
or unauthorized identifiers.

## 9. Identity, privacy, and security

### 9.1 Sign-in and sessions

V1 supports GitHub and Google OAuth. The desktop uses the system browser,
Authorization Code, and Proof Key for Code Exchange.

The service validates issuer, audience, code, verifier, state, redirect URI, nonce
where applicable, provider subject, verified email, expiry, and callback replay.

The API issues a short-lived signed access token and a rotating opaque refresh token.
D1 stores refresh-token hashes. Refresh-token reuse revokes the session family.

Tokens live only in the operating-system keychain.

### 9.2 Authorization

Every private read and write authenticates the active account, resolves the
repository, and checks lifecycle, membership, and required role. The applicable
operation then validates content, revision inputs, acknowledgements, quotas, and
rate limits. Publication repeats its guards within the D1 transaction.

Private access returns the same bounded `not_found` response for a missing and a
hidden repository. Removal status uses the Admin authorization from section 2.8.
Anonymous requests cannot reveal removed repositories.

### 9.3 Plaintext content

Managed content is not encrypted by the Mallard application. Controlled service
operators can access stored content.

The service still requires:

- HTTPS;
- private R2 buckets;
- encryption provided by the hosting platform;
- signed access tokens;
- hashed refresh and invitation tokens;
- secret-manager storage for service credentials;
- least-privilege service identities; and
- audited production access.

V1 has no content keys, encrypted manifest format, key recovery phrase, or device-key
approval flow.

### 9.4 Content safety

Before upload, the desktop applies bounded capture, path validation, secret-pattern
warnings, and explicit review.

The service independently rejects:

- credentials in known forbidden fields;
- absolute paths;
- malformed object coordinates;
- unknown resource kinds;
- Catalog item content that differs from its bound provider or kind;
- cross-repository content;
- symbolic-link or special-file semantics;
- executable content without acknowledgement;
- unbounded documents; and
- digest or length mismatch.

Secret detection is best-effort. It is not proof that content is safe.

### 9.5 Plugin execution boundary

The Worker and R2 never execute plugin content.

The service stores only validated intent. The desktop builds provider-native actions
from its closed provider adapter after Pull review. Portable strings never select an
arbitrary executable or shell command.

### 9.6 Logging

Logs, metrics, traces, errors, and audit do not contain:

- access or refresh tokens;
- invitation tokens;
- OAuth codes or verifiers;
- raw email addresses;
- R2 credentials;
- object content;
- conversation text;
- skill file content;
- plugin source secrets;
- pet bytes;
- local paths; or
- provider-home identity.

Use opaque IDs, request IDs, bounded error codes, byte counts, and digests.

## 10. Featured examples and failure behavior

### 10.1 Featured examples

An internal publisher creates private example repositories, publishes reviewed
heads, completes the visibility review, and sets the featured flag. A Git project,
skill, plugin intent, pet v1, and pet v2 can each be an independent example.
Anonymous users can discover and Pull them. Publication requires membership.

### 10.2 Additional failure behavior

Publication, cleanup, removal, and local review failures follow their owning
sections. These additional cases define the remaining user or operator action:

| Failure | Result |
| --- | --- |
| Invitation delivery fails | Revoke the undelivered token |
| Queue job runs twice | Resume the recorded operation without repeating its committed effects |
| Public head changes during anonymous Pull | Restart from the current head |
| Visibility becomes private during public read | Deny later object reads |
| Local receipt fails after Apply | Report reconciliation; do not repeat an unsafe action |

## 11. Operations and UX

### 11.1 Retention

Accepted-content retention and cancellation follow sections 2.7 and 2.8. Other
defaults are:

| Data | Default |
| --- | --- |
| Pending invitation | 7 days |
| Upload session | At most 24 hours, bounded by approval expiry |
| Unaccepted bytes | Eligible for cleanup 7 days after the session ends, subject to section 6.7 |
| Idempotency record | 24 hours |
| Sanitized audit | 365 days |
| Recovery copies | Expire within 30 days after verified erasure |

The removal record outside D1 remains available until recovery copies expire.
Backup retention is an operator policy; the desktop shows repository lifecycle and
cancellation status. An erasure deadline permits work to start and does not promise
that failed work is complete.

### 11.2 Audit and monitoring

Audit records opaque actor and target IDs, repository class, repository ID, action,
result, bounded reason, request ID, build ID, and time.

Audit covers identity, invitations, roles, visibility, publication,
conflicts, integrity, quotas, public disclosure, removal requests,
cancellation, erasure progress, and completion.

Monitor:

- request latency and errors;
- authentication and refresh-token reuse;
- public authorization denials;
- invitation delivery;
- publication conflicts;
- class and kind validation failures;
- object integrity;
- D1, R2, and queue health;
- quotas; and
- reconciliation.

Alert on private-resource probing, token reuse, integrity failure, invalid quota
state, missing current objects, unauthorized public reads, dead-letter growth, and
stalled erasure or unresolved object operations.

### 11.3 Backup and service recovery

Production requires:

- D1 point-in-time recovery;
- R2 recovery controls;
- deployment-configuration backup;
- signing-key rotation;
- dead-letter replay;
- D1-to-R2 reconciliation;
- isolated recovery tests; and
- a disaster-recovery exercise.

A recovered head remains unavailable until all referenced objects pass coordinate,
length, and digest checks.

Before opening a recovered database to traffic, apply the removal records retained
outside that restore. Removed repositories stay inaccessible and their residual
content is erased again. Recovery tests must cover a database restored to a point
before a removal request or quota release.

### 11.4 Desktop navigation

The managed-cloud UI contains:

- **Projects**;
- **Catalog**;
- **Invitations**;
- **History**;
- **Activity**; and
- account and session settings.

Projects and Catalog each contain **Public** and **Private** sections. The Public
section contains public repositories. The Private section contains only private
repositories for which the signed-in user is an active Admin or Member. It never
lists another user's private repository.

Each Catalog row is one item and one access boundary. It shows name, provider,
resource kind, visibility, and the signed-in user's role when applicable.

An active Admin or Member sees **Review and push**, **Review and pull**, and
**History**.

### 11.5 Repository settings

Admins can manage name, visibility, Members, invitations, quota, audit summary,
and whole-repository removal. The removal view shows the repository, current
revision, retained bytes, cancellation deadline, and erasure state.
It offers cancellation only during the recovery period.

The repository class, project identity, and Catalog item provider, resource kind,
format, format version, and resource ID are read-only after creation.

Members can view repository identity, visibility, current head, history, and active
members.

### 11.6 Required disclosure

Before first managed publication, state:

- Mallard stores the selected content in the managed service.
- Mallard does not end-to-end encrypt the content.
- Controlled service operators can access stored content.
- Credentials and complete provider homes must not be selected.
- Anyone can read public repository content.

Plugin disclosure states that the service stores installation intent and that the
desktop can run a reviewed provider operation during Pull. Removal confirmation
uses the repository and cancellation terms from section 2.8.

## 12. Delivery and verification

### 12.1 Delivery slices

**Slice 0: Common contracts**

Deliver current Rust serialization fixtures, head and object protocol, managed
publication adapter, local binding, keychain tokens, validators, approval digesting,
and deterministic service contract tests. Prove publication and cleanup with
synthetic data before adding user publication.

Implementation note: select and test the simplest guard mechanism for section 6.4.
SQL validation can use aborting triggers; a Durable Object per repository can
coordinate mutations if it reduces complexity. Neither option may rely on a check
after commit. Coordination must cover all competing repository operations, and D1
must still protect shared account quotas and current authorization. Keep the SQL
statement order and any trigger definitions with the Slice 0 implementation. See
[Durable Object state](https://developers.cloudflare.com/durable-objects/reference/in-memory-state/).

**Slice 1: Public Git project read**

Deliver Worker, D1, private R2, project creation by an internal publisher, public
listing, current-snapshot read, project Pull review, and public limits.

**Slice 2: Identity and private Git project publication**

Deliver OAuth, rotating sessions with labels, private creation, first Admin, project
Push, uploads, publication, history, quotas, and idempotency.

**Slice 3: Skills**

Deliver the Catalog isolation adapter, item creation, independent access, Codex and
Claude skill capture, public and private Push and Pull, and disclosure.

**Slice 4: Plugin install intent**

Add validated plugin items and provider-derived Pull actions, including manual
outcomes and public disclosure.

**Slice 5: Codex pets**

Add v1 and v2 items, atlas handling, previews, Push and Pull, and public disclosure.

**Slice 6: Collaboration and operations**

Deliver invitations, role changes, Member publication, visibility changes, featured
examples, repository removal, queues, reconciliation, monitoring, backup, and runbooks.

Each slice must work end to end and pass its relevant tests before the next starts.

### 12.2 Test groups

| Contract | Verification |
| --- | --- |
| Identity and access (sections 2.3-2.6, 9.1-9.2) | Anonymous, non-member, Member, and Admin cases; session rotation and revocation; invitation replay; concurrent Admin changes |
| Retention and removal (sections 2.7-2.8) | Full quota; cancellation versus claim; delayed cancelled jobs; partial erasure; repeated completion; restore from before request and quota release |
| Portable contracts (sections 3.4, 4.1-4.3, 5.2-5.5) | Generated valid and invalid fixtures; integer boundaries; canonical digests; codec limits; class and identity mismatch |
| Project review (sections 4.4-4.5) | Compare performs no upload capture or executable conversation planning; Apply checks fresh actions, approved effects, target drift, and per-item failures |
| Catalog projection and review (sections 5.1, 5.6) | Place public and private items in one local Library; verify the exact transmitted item and payload set; exercise local scope and warning drift |
| Publication (sections 3.3, 6.4) | First-head race, two writers, zero-row head update, and injected failures across the transaction; compare all persistent effects before and after failure |
| Reads (section 6.5) | Cross-repository and historical public probes; truncated or corrupt objects; visibility and head changes during Pull |
| Retries (section 6.6) | Lost responses and concurrent retries; same key with changed body; terminal uploads after idempotency-record expiry |
| Cleanup and quota (section 6.7) | Both race orderings; delayed delete after re-upload with a new upload ID; unresolved writes; concurrent account reservations; repeated expiry |
| Local execution (sections 4.5, 5.6, 9.5, 10.2) | Skill and pet copy preservation; provider/manual plugin outcomes; partial apply and receipt failures |
| Operations (sections 11.1-11.3) | Queue replay, reconciliation, stalled erasure alerts, and isolated backup/recovery exercise |

Run:

- `npm run build`;
- `npm run test:frontend-integration`;
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`; and
- managed-service contract, deployment, and recovery tests.

### 12.3 Rollout

Use development, staging with synthetic data, internal featured examples, invited
private beta, production readiness review, and general release.

External private beta requires working repository removal and passing erasure,
quota-release, and recovery tests. Do not add compatibility for an undeployed design.

## 13. Production acceptance

Release requires completed delivery slices, passing verification in section 12.2,
and the rollout gate in section 12.3. Operations must have working alerts and a
tested recovery runbook from section 11. Record release evidence against these sections.

## Implementation references

Cloudflare:

- [D1](https://developers.cloudflare.com/d1/)
- [D1 Worker API](https://developers.cloudflare.com/d1/worker-api/)
- [D1 recovery](https://developers.cloudflare.com/d1/reference/time-travel/)
- [R2 Workers API](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [R2 multipart uploads](https://developers.cloudflare.com/r2/api/workers/workers-multipart-usage/)
- [Queues](https://developers.cloudflare.com/queues/)
- [Queue dead-letter queues](https://developers.cloudflare.com/queues/configuration/dead-letter-queues/)
- [Workers secrets](https://developers.cloudflare.com/workers/configuration/secrets/)
- [Cloudflare Access policies](https://developers.cloudflare.com/cloudflare-one/access-controls/policies/)
- [Workers observability](https://developers.cloudflare.com/workers/observability/)

Identity:

- [OAuth 2.0 for Native Apps, RFC 8252](https://www.rfc-editor.org/rfc/rfc8252)
- [GitHub OAuth authorization](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps)
- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
