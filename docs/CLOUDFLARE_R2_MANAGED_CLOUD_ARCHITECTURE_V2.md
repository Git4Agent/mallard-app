# Mallard Managed Cloud V1

## Git Project and Catalog Item plan

- **Status:** Proposed
- **Date:** September 1, 2026
- **Scope:** Git projects, skills, plugin install intent, and Codex pets

This document is the complete Managed Cloud V1 plan. It contains only the current
design.

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
| Repository classes | `git_project` and `catalog` |
| Project admission | Git projects only |
| Project content | Reviewed conversations and supported provider state |
| Catalog item content | Exactly one skill, plugin install intent, Codex pet v1, or Codex pet v2 |
| Catalog boundary | One independent Catalog item |
| Access boundary | One managed Project or Catalog item |
| Visibility | Public or private |
| Roles | Admin and Member |
| First Admin | Repository creator |
| Collaboration | Admin invites Members |
| Anonymous access | Current public snapshot only |
| Content pointer | One current head per repository |
| Publication | Reviewed Push by an active Admin or Member |
| Concurrent publication | Exact-head compare-and-swap |
| Stale publication | Reject and require a new review |
| Accepted resource removal | Not supported |
| Repository removal | No product UI or API |
| Git source | Remains in the configured Git remote |
| Application content encryption | None |

### 1.2 Included

V1 includes:

- Git project validation and fingerprinting;
- reviewed project Push and Pull;
- Codex and Claude skills;
- Codex and Claude plugin install intent;
- Codex pet v1 and v2 packages;
- reviewed Catalog item Push and Pull;
- public and private Projects and Catalog items;
- repository invitations;
- Admin and Member management;
- one accepted head and immutable history per repository;
- optimistic concurrency;
- public discovery and featured examples;
- repository export;
- OAuth sign-in;
- quotas, audit, monitoring, and service recovery.

### 1.3 Not included

V1 does not include:

- non-Git project repositories;
- ordinary Git project-file cloud sync;
- Git source hosting or Git-host writes;
- parent access containers;
- shared membership across repositories;
- multi-item Catalog repositories;
- shared access settings across Catalog items;
- per-file visibility or membership inside one Catalog item;
- roles other than Admin and Member;
- multiple managed heads in one repository;
- content approval queues;
- automatic combination of concurrent publications;
- plugin binaries or marketplace credentials;
- Claude pets;
- pet format conversion;
- accepted resource removal;
- repository-removal workflows;
- a web write client;
- billing;
- shared public object caching;
- client-side content encryption; or
- end-to-end content encryption.

## 2. Repository and access model

### 2.1 Repository classes

| Class | Required identity | Portable content |
| --- | --- | --- |
| `git_project` | Bundle ID and Git repository fingerprint | Project conversations and supported provider state |
| `catalog` | Provider, resource kind, and stable resource ID | One skill, plugin intent, or pet package |

A Catalog item repository is not a project repository. It has no project root and
does not make non-Git projects eligible for Managed Cloud V1.

A service repository ID is an opaque managed-service identifier. It is distinct from
the portable project bundle ID and Catalog resource ID.

Project bundle IDs, project fingerprints, and Catalog resource IDs are not access
identifiers. Knowing one does not grant access or prove that a private service
repository exists.

Each Catalog item repository contains exactly one item. Its provider, resource kind,
and stable resource ID cannot change after creation.

The Catalog UI groups item repositories by visibility. The group is a view. It is not
an access container. For example:

| Public | Private |
| --- | --- |
| `petv2_rika` | `petv1_rika321` |
| `petv1_rena` | `skill_codex_rena` |
| `skill_codex_rika` |  |

These names are examples only. V1 does not create them automatically and does not
require this naming pattern.

### 2.2 Catalog provider matrix

Catalog items use this closed matrix:

| Provider | `agent_skill` | `plugin_install_intent` | `codex_pet_v1` | `codex_pet_v2` |
| --- | --- | --- | --- | --- |
| `codex` | Yes | Yes | Yes | Yes |
| `claude` | Yes | Yes | No | No |

The service rejects an unknown provider, an unknown resource kind, or an invalid
provider-and-kind pair.

### 2.3 Visibility

| Visibility | Anonymous read | Member read | Publish |
| --- | --- | --- | --- |
| Public | Current accepted snapshot | Yes | Active Admin or Member |
| Private | No | Yes | Active Admin or Member |

Visibility applies to one complete Project or one complete Catalog item. All files
and objects in an item inherit its visibility. Other Catalog items can use a different
visibility.

Anonymous public access does not include:

- retained history;
- unaccepted uploads;
- review plans;
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
| Export retained content | No | No | Yes |
| Invite or remove a Member | No | No | Yes |
| Promote or demote an Admin | No | No | Yes |
| Change name, slug, or visibility | No | No | Yes |

Rules:

- `admin` and `member` are the only role values.
- The creator becomes the first Admin.
- Each repository keeps at least one active Admin.
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

Project and Catalog item creation always starts private.

Changing a repository to public requires:

1. an accepted head;
2. a current full disclosure review;
3. explicit public-content acknowledgement;
4. an expected repository revision; and
5. recent Admin authentication.

Changing a repository to private increments its revision before the API returns.
There is no shared public object cache, so no cache purge is required.

Downloaded public copies cannot be revoked.

### 2.7 No-removal rule

A successful Project publication can add a resource or replace the content of the
same stable resource identity. It cannot omit an accepted resource or publish a
tombstone.

A successful Catalog item publication can replace only the content of its stable
resource identity. It cannot change the item identity, publish an empty item, or
publish a tombstone.

This rule applies to project conversations, project provider state, skills, plugin
install intent, and pets.

Pull can create or update local content. Pull does not delete a local resource.

Removing a Member does not remove repository content.

Accepted publications and their objects remain retained. Repository quota is a hard
cap. V1 rejects a new publication when accepted history consumes the quota.

## 3. Common repository protocol

### 3.1 One current head

Each repository has zero or one current head. The head contains:

- service repository ID;
- repository class;
- publication revision;
- commit ID;
- manifest key;
- manifest SHA-256;
- manifest byte length;
- update time; and
- opaque ETag.

The first publication compares against an absent head. Each later publication
compares against the exact ETag that the user reviewed.

Only the head change accepts a publication.

### 3.2 Immutable history

Each successful publication creates:

- immutable content-addressed payloads;
- one immutable manifest; and
- one immutable commit that references the previous commit.

The commit chain is linear. Active Admins and Members can read retained history.
Anonymous users can read only the current public snapshot.

### 3.3 Concurrent publication

If two users review the same head:

1. both plans pin the same ETag;
2. the first valid publication changes the head;
3. the second publication receives `head_changed`;
4. the second publication cannot overwrite the winner; and
5. the losing user reads the new snapshot and reviews again.

The service does not combine the two publications.

### 3.4 Shared storage properties

Project and Catalog item repositories use the same immutable object transport. They
use different closed manifest validators.

The shared properties are:

- schema 1 documents;
- UTF-8 JSON;
- bounded fields and collections;
- UTC RFC 3339 times;
- lowercase SHA-256 values;
- content-addressed payloads;
- create-only immutable writes;
- exact byte-length checks;
- one compare-and-swap head; and
- non-authoritative listing tags.

The repository class cannot change after creation.

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
- the current Git commit ID; and
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

The immutable project manifest records:

- schema version;
- bundle ID and display name;
- repository fingerprint;
- publication revision;
- bundle commit ID;
- current Git commit ID;
- update time;
- publishing origin; and
- reviewed resource scopes.

Each scope contains bounded resource descriptors, conversation entries, file entries,
and directory entries. Managed Cloud rejects project files and all tombstones.

### 4.4 Project Push review

The desktop:

1. validates the Git binding and fingerprint;
2. reads the current managed head and manifest;
3. discovers allowed project resources;
4. captures candidates into private temporary files;
5. compares local, accepted-base, and managed state;
6. shows additions, changes, retained items, conflicts, and warnings;
7. lets the user select exact resources;
8. pins the repository revision, visibility, head ETag, source digests, selection,
   acknowledgements, and expiry;
9. rechecks every pin; and
10. publishes the reviewed result.

The proposed manifest carries forward every current resource that the user did not
replace.

### 4.5 Project Pull review

The desktop:

1. reads the exact authorized head;
2. verifies the manifest and selected objects;
3. compares managed content with local content and accepted bases;
4. shows creates, updates, preserved items, conflicts, warnings, and manual actions;
5. lets the user approve exact actions;
6. pins the head, manifest, target digests, selection, acknowledgements, and expiry;
7. rechecks every pin;
8. creates local backups where an update can overwrite content;
9. applies with atomic replacement where supported; and
10. records outcomes and accepted bases.

Pull plans stay machine-local.

## 5. Catalog item repository contract

### 5.1 Item identity

One Catalog item repository contains exactly one item. The item identity is:

~~~text
(provider, resource_kind, resource_id)
~~~

The tuple is portable and stable. It is different from the opaque service repository
ID. The provider, resource kind, and resource ID cannot change after creation.

Each item has one visibility value, one membership list, one current head, and one
history. Visibility and membership do not affect other Catalog items.

Local Catalog profile IDs, configured homes, scope IDs, storage IDs, and accepted
bases do not enter managed content.

The managed adapter maps the item to one compatible local provider-and-kind scope and
resource target. Existing local-folder and user-owned S3/R2 Catalog protocols remain
unchanged.

### 5.2 Skills

`agent_skill` is available for Codex and Claude.

A skill resource contains:

- closed skill scope;
- effective name;
- install-directory name;
- canonical content digest; and
- a bounded relative file tree that includes `SKILL.md`.

The supported skill scopes are:

- Codex home;
- shared Codex user scope; and
- Claude user scope.

Skill validation rejects:

- absolute or escaping paths;
- empty and dot path segments;
- backslashes and control characters;
- case-folded path collisions;
- nested Git metadata;
- symbolic links;
- special files;
- unsupported file modes;
- missing `SKILL.md`;
- digest mismatch; and
- size, depth, file-count, or path-length overflow.

Executable skill files require an explicit warning acknowledgement.

### 5.3 Plugin install intent

`plugin_install_intent` is available for Codex and Claude.

The resource stores semantic installation intent, not plugin code. It contains:

- provider;
- provider-native marketplace namespace;
- canonical qualified plugin ID;
- credential-free marketplace source when portable;
- optional Git revision; and
- resolution policy.

The two resolution policies are:

- `provider_resolved`: the desktop can use a validated provider operation; and
- `manual_only`: the source is not portable enough for an automatic operation.

The resource does not contain:

- downloaded plugin files;
- marketplace credentials;
- API tokens;
- local install paths;
- observed installed version;
- enablement state;
- exported skills; or
- shell commands.

During Pull, the desktop derives a closed argument vector from the provider adapter.
It never executes a command string from portable content. The desktop revalidates the
provider, source, marketplace operation, and expected target before execution.

### 5.4 Codex pets

Pets are Codex-only. V1 and V2 are different resource kinds and can coexist as
independent Catalog items.

| Kind | Atlas | Grid | Cell |
| --- | --- | --- | --- |
| `codex_pet_v1` | 1536 x 1872 | 8 x 9 | 192 x 208 |
| `codex_pet_v2` | 1536 x 2288 | 8 x 11 | 192 x 208 |

A pet resource contains:

- pet ID;
- display name;
- description;
- atlas basename;
- one PNG or WebP atlas descriptor; and
- canonical content digest.

Validation checks the exact format dimensions, safe basename, byte limit, content
digest, object digest, and object length. V1 does not convert between pet formats.

### 5.5 Catalog item manifest

The Catalog item manifest contains:

- schema version;
- provider, resource kind, and stable resource ID;
- revision and commit ID;
- update time;
- capturing application and version;
- privacy-preserving publishing origin;
- one complete resource descriptor; and
- the immutable objects that the descriptor references.

The manifest is the complete snapshot of one Catalog item. A publication replaces
the accepted content for that same item. It cannot add a second item or change the
item identity.

The service also validates these kind-specific identity fields:

| Kind | Identity fields |
| --- | --- |
| Skill | Scope, effective name, install-directory name |
| Plugin intent | Provider, namespace, qualified plugin ID |
| Pet | Pet format and pet ID |

Content changes do not change these identity fields. A change to an identity field
requires a new Catalog item repository.

A public item exposes its complete current manifest and all referenced objects. It
does not expose a private item.

### 5.6 Catalog item Push review

The desktop:

1. resolves the item's machine-local provider scope and resource target;
2. reads the current managed head and manifest;
3. inventories the exact local item;
4. validates and captures the candidate package;
5. compares local, accepted-base, and managed content;
6. holds an unreviewed managed change for Pull;
7. shows the update, conflicts, warnings, provider, kind, and resource ID;
8. requires approval for the complete item package;
9. pins the local scope, resource target, repository revision, visibility, head ETag,
   package digest, acknowledgements, and expiry;
10. rechecks every pin; and
11. publishes by compare-and-swap.

An item without a valid local mapping cannot be pushed.

### 5.7 Catalog item Pull review

The desktop:

1. reads and verifies the complete current Catalog item;
2. resolves its compatible local provider scope and resource target;
3. inventories the exact local target;
4. compares local, accepted-base, and managed content;
5. creates one typed item action;
6. shows the provider, kind, target, effect, conflict behavior, and warnings;
7. pins the managed head, scope, and expected local target;
8. requires explicit acknowledgement for destructive updates;
9. applies the approved action through the matching provider-and-kind adapter;
10. records the outcome; and
11. advances the accepted base only after success.

An item with no valid local scope mapping remains view-only until the user selects a
compatible local scope.

Skills and pets can preserve divergent local content as a reviewed copy before the
managed version is applied. Plugin intent cannot be copied. A plugin conflict requires
an explicit provider operation or a manual action.

Catalog item Pull does not remove a local resource.

## 6. Cloud architecture

### 6.1 Topology

~~~text
Mallard desktop
  |
  | HTTPS with OAuth access token
  v
Cloudflare Worker API
  |-- D1: identity, access, repository metadata, heads, plans, quota, audit
  |-- R2: private immutable manifests, commits, payloads, staging, exports
  |-- Queue and Cron: expiry, cleanup, reconciliation, notification retry
  `-- Email provider: invitation messages
~~~

The desktop never receives:

- R2 credentials;
- D1 credentials;
- Worker administration tokens; or
- direct internal object keys.

Every read and write passes through the Worker.

### 6.2 Component ownership

The Worker owns:

- OAuth callbacks and session rotation;
- repository authorization;
- repository-class validation;
- public discovery;
- invitations and membership;
- review-plan registration;
- upload validation;
- manifest validation;
- head compare-and-swap;
- history metadata;
- quota reservation;
- idempotency;
- audit; and
- bounded errors.

D1 is authoritative for identity, authorization, repository metadata, current heads,
history reachability, quotas, idempotency, and audit metadata.

R2 is authoritative for immutable bytes at an exact internal key.

An R2 listing never grants access and never decides the current head.

### 6.3 Internal R2 keys

~~~text
repos/<service-repository-id>/objects/<sha256>
repos/<service-repository-id>/manifests/<revision>-<commit-id>.json
repos/<service-repository-id>/commits/<revision>-<commit-id>.json
staging/<service-repository-id>/<upload-id>/<object-id>
exports/<service-repository-id>/<export-id>
~~~

The Worker constructs each key from validated identifiers. A client cannot submit a
complete internal key.

### 6.4 Upload and publication transaction

An upload session pins:

- service repository;
- repository class;
- actor;
- repository revision;
- expected head ETag;
- visibility;
- review-plan digest;
- expected object hashes and lengths;
- quota reservation;
- expiry; and
- idempotency key.

The Worker streams each object, enforces exact length, computes SHA-256, and writes
with create-only semantics. Incomplete uploads remain unreachable in staging.

The final D1 transaction:

1. reauthorizes the active Admin or Member;
2. rechecks repository class, revision, and visibility;
3. rechecks the expected head ETag;
4. verifies the unused review plan;
5. validates the class-specific manifest;
6. proves that the new manifest omits no current resource;
7. verifies immutable-object metadata;
8. inserts publication metadata;
9. advances the head by compare-and-swap;
10. updates current and historical reachability;
11. commits quota;
12. consumes the plan;
13. records idempotency; and
14. writes audit.

If compare-and-swap fails, the current head remains unchanged.

### 6.5 Read path

The Worker:

1. resolves the service repository;
2. authorizes active membership or current-public-snapshot access;
3. validates the requested logical coordinate;
4. resolves exact D1 object metadata;
5. reads the R2 object;
6. verifies length; and
7. streams bytes with digest and ETag.

The desktop independently verifies length and SHA-256.

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
- publication; and
- export creation.

Compare-and-swap protects against different concurrent publications. Idempotency
protects against a retry of the same publication.

### 6.7 Quota and cleanup

V1 limits:

- repositories per account;
- accepted bytes per repository and account;
- objects per repository;
- concurrent uploads;
- object size;
- manifest size;
- project resources per manifest and exactly one resource per Catalog item manifest;
- skill files and bytes;
- pet atlas bytes;
- audit retention; and
- request rates.

Cleanup can remove only expired staging data, expired exports, expired plans, expired
invitations, and unreachable objects that were never accepted.

Cleanup cannot remove an accepted publication or an object reachable from accepted
history.

Reconciliation detects missing bytes, untracked bytes, expired reservations, head
disagreement, and tag drift.

## 7. D1 model

### 7.1 Tables

| Table | Purpose |
| --- | --- |
| `users`, `identities`, `devices`, `refresh_sessions` | Accounts and sessions |
| `repositories` | Service ID, class, name, slug, visibility, revision, featured flag |
| `project_repository_bindings` | Bundle ID and Git fingerprint |
| `catalog_repository_bindings` | Provider, resource kind, and stable resource ID |
| `repository_heads` | Zero or one current head per service repository |
| `repository_publications` | Immutable linear publication history |
| `repository_members` | Repository Admin or Member |
| `repository_invitations` | Email HMAC, masked label, token hash, expiry, state |
| `review_plans` | Content model, review digest, pins, acknowledgements, expiry |
| `objects`, `upload_sessions` | R2 metadata, reachability, and staging |
| `quota_ledgers` | Account and repository reserved and committed use |
| `idempotency_records` | Safe mutation replay |
| `repository_exports` | Expiring repository exports |
| `audit_events` | Sanitized security history |

### 7.2 Closed values

| Field | Values |
| --- | --- |
| User state | `active`, `suspended` |
| Repository class | `git_project`, `catalog` |
| Visibility | `public`, `private` |
| Provider | `codex`, `claude` |
| Catalog item kind | `agent_skill`, `plugin_install_intent`, `codex_pet_v1`, `codex_pet_v2` |
| Role | `admin`, `member` |
| Membership state | `active`, `suspended`, `removed` |
| Invitation state | `pending`, `accepted`, `revoked`, `expired` |
| Plan state | `active`, `used`, `expired`, `revoked` |
| Upload state | `created`, `receiving`, `ready`, `committed`, `expired`, `failed` |
| Reachability | `current`, `historical`, `unreachable` |

### 7.3 Required constraints

Enforce:

- one identity per provider subject;
- exactly one binding that matches each service repository class;
- one portable bundle identity inside each project binding;
- exact Catalog item identity match between binding and manifest;
- valid provider-and-kind ownership for each Catalog item;
- one current public slug;
- one active membership per repository and user;
- at least one active Admin per repository;
- zero or one current head;
- one publication revision and commit ID per repository;
- one live object coordinate per repository;
- non-negative quota values;
- public discovery only for public repositories;
- current head that references verified class-specific data;
- a linear previous-commit chain;
- no resource omission or tombstone;
- no stale-head publication; and
- no cross-class manifest acceptance.

### 7.4 Atomic operations

| Operation | One transaction contains |
| --- | --- |
| Create project repository | Private repository, project binding, creator Admin, quota, idempotency, audit |
| Create Catalog item repository | Private repository, item binding, creator Admin, quota, idempotency, audit |
| Accept invitation | Membership, consumed invitation, repository revision, audit |
| Change membership | Role or state, last-Admin proof, repository revision, audit |
| Change visibility | Visibility, disclosure proof, repository revision, audit |
| Publish | Membership, plan, head CAS, publication, reachability, quota, idempotency, audit |

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
| `GET` | `/v1/me` | Account, devices, repositories, and limits |
| `GET` | `/v1/public/repositories` | Filtered public repository listing |
| `GET` | `/v1/public/repositories/{slug}` | Public summary and repository class |
| `GET` | `/v1/public/repositories/{slug}/head` | Current public head |
| `GET` | `/v1/public/repositories/{slug}/manifest/{commit_id}` | Current public manifest |
| `GET` | `/v1/public/repositories/{slug}/objects/{sha256}?commit_id={commit_id}` | Current public object |

Public listing filters can include repository class, provider, and Catalog item kind.
The authenticated repository list can also filter by visibility. These filters create
the Public and Private Catalog views.

### 8.2 Repository and access routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET`, `POST` | `/v1/repositories` | List memberships or create a private repository |
| `GET`, `PATCH` | `/v1/repositories/{repository_id}` | Read or change metadata |
| `POST` | `/v1/repositories/{repository_id}/visibility` | Make public or private |
| `POST` | `/v1/repositories/{repository_id}/exports` | Export retained content |
| `GET` | `/v1/repositories/{repository_id}/exports/{export_id}` | Read export status |
| `GET` | `/v1/repositories/{repository_id}/exports/{export_id}/content` | Download completed export |
| `GET` | `/v1/repositories/{repository_id}/members` | Active and invited access |
| `POST` | `/v1/repositories/{repository_id}/invitations` | Invite Member |
| `POST` | `/v1/repositories/{repository_id}/invitations/{invitation_id}/revoke` | Revoke invitation |
| `POST` | `/v1/invitations/accept` | Accept token from request body |
| `PATCH`, `DELETE` | `/v1/repositories/{repository_id}/members/{user_id}` | Change or remove membership |
| `POST` | `/v1/repositories/{repository_id}/leave` | Leave under the last-Admin rule |

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
  "resource_kind": "codex_pet_v2",
  "resource_id": "petv2_rika"
}
~~~

The service validates and stores the Catalog item identity. Catalog item creation is
private even when the requested name matches a public item. Invitation acceptance
sends the raw token in the request body. Raw invitation tokens do not enter URLs or
logs.

### 8.3 Content routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/v1/repositories/{repository_id}/head` | Current head |
| `GET` | `/v1/repositories/{repository_id}/history` | Retained history |
| `GET` | `/v1/repositories/{repository_id}/publications/{commit_id}` | Publication metadata |
| `GET` | `/v1/repositories/{repository_id}/manifests/{commit_id}` | Authorized manifest |
| `POST` | `/v1/repositories/{repository_id}/push-plans` | Register class-specific Push review |
| `POST` | `/v1/repositories/{repository_id}/disclosure-plans` | Register public disclosure review |
| `POST` | `/v1/repositories/{repository_id}/uploads` | Start bounded upload |
| `PUT` | `/v1/repositories/{repository_id}/uploads/{upload_id}/objects/{object_id}` | Upload expected object |
| `POST` | `/v1/repositories/{repository_id}/uploads/{upload_id}/complete` | Verify upload |
| `POST` | `/v1/repositories/{repository_id}/publish` | Publish by head CAS |
| `GET` | `/v1/repositories/{repository_id}/objects/{object_id}` | Read authorized object |

The review-plan request is tagged with the repository class. The service rejects a
tag that differs from the repository binding.

Creating a plan or upload does not change the head.

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
- `last_admin`;
- `invitation_invalid`;
- `invitation_expired`;
- `quota_exceeded`;
- `object_integrity_failed`;
- `plan_expired`;
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

Every private read and every write uses this order:

1. authenticate token;
2. resolve service repository;
3. verify active membership;
4. verify role where Admin is required;
5. verify repository class and content model;
6. verify expected revision and head;
7. verify plan and warning acknowledgements;
8. enforce quota and rate limit; and
9. record sanitized audit.

Unauthorized private access returns the same bounded `not_found` response for a
missing repository and a hidden repository.

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

## 10. Workflows and failure behavior

### 10.1 Create Git project repository

1. The desktop validates Git root, origin, fingerprint, and current commit.
2. The API creates a private `git_project` repository and first Admin.
3. Project Push review selects the initial portable state.
4. The desktop registers the plan and uploads immutable objects.
5. Publication creates the first head by compare-and-swap against absence.
6. The desktop stores the local managed-repository binding.

### 10.2 Create Catalog item repository

1. The user selects one local skill, plugin intent, or pet.
2. The desktop resolves its provider, resource kind, stable resource ID, and local
   scope.
3. The API creates one private `catalog` repository and first Admin.
4. Catalog item Push review shows the complete initial package.
5. The service validates the item identity and package.
6. Publication creates the first head by compare-and-swap against absence.
7. The desktop links that local item to the managed repository.

### 10.3 Invite Member

1. An Admin submits an email.
2. The API stores email HMAC, masked label, and token hash.
3. The email provider sends the raw token once.
4. The recipient signs in with the matching verified email.
5. One transaction creates Member access and consumes the invitation.

The Member can read and publish reviewed snapshots of that repository.

### 10.4 Publish

1. The desktop reads the current head.
2. The applicable Push review compares local and managed state.
3. The user selects Project resources or approves one Catalog item and accepts
   warnings.
4. The desktop registers the plan and uploads required objects.
5. The publication transaction rechecks access, repository revision, visibility,
   content model, plan, objects, quota, and head ETag.
6. Compare-and-swap advances the head.
7. The new snapshot becomes current.

For a public repository, review states that the complete resulting snapshot becomes
public.

### 10.5 Pull

1. The desktop reads the authorized head.
2. It verifies the class-specific manifest and selected objects.
3. The applicable Pull review shows exact local actions.
4. The user approves actions and warnings.
5. The desktop rechecks local targets.
6. The desktop applies through the project or Catalog item adapter.
7. The desktop records outcomes and accepted bases.

Anonymous Pull is limited to a public current snapshot.

### 10.6 Featured examples

An internal publisher creates example repositories as private, publishes reviewed
initial heads, completes disclosure review, changes visibility to public, and sets the
featured flag.

V1 can feature one Git project repository and several independent Catalog item
repositories. A skill, plugin intent, pet v1, and pet v2 example each has its own
visibility, membership, and head.

Anonymous users can discover and Pull them. Publication requires invitation.

### 10.7 Failure rules

| Failure | Result |
| --- | --- |
| OAuth validation fails | No session |
| D1 fails before commit | Retryable error; no state change |
| R2 succeeds but publication fails | Unaccepted immutable data; later cleanup |
| Publication succeeds but response is lost | Idempotent retry returns the recorded result |
| Repository revision changed | No publication; fresh review |
| Head ETag changed | `head_changed`; winner remains |
| Repository class or Catalog item identity differs | Reject before publication |
| Proposed manifest omits an accepted resource | `resource_removal_not_supported` |
| Object digest or length differs | No publication |
| Invitation delivery fails | Revoke the undelivered token |
| Queue job runs twice | No duplicate change |
| Public head changes during anonymous Pull | Restart from the current head |
| Visibility becomes private during public read | Deny the later object read |
| Local target changes after Pull review | Block the affected action |
| Provider plugin action cannot be derived | Manual action; no execution |
| Receipt fails after local apply | Report reconciliation; do not repeat an unsafe action |

No failure can publish private data, overwrite a changed head, mix repository classes,
apply an unreviewed local action, or delete a resource.

## 11. Operations and UX

### 11.1 Retention

| Data | Default |
| --- | --- |
| Pending invitation | 7 days |
| Upload staging | 24 hours |
| Unaccepted objects | 7 days |
| Export | 24 hours |
| Idempotency record | 24 hours |
| Sanitized audit | 365 days |
| Accepted publications | Retained within hard repository quota |

### 11.2 Audit and monitoring

Audit records opaque actor and target IDs, repository class, repository ID, action,
result, bounded reason, request ID, build ID, and time.

Audit covers identity, invitations, roles, visibility, review plans, publication,
conflicts, integrity, quotas, public disclosure, and export.

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
state, missing current objects, unauthorized public reads, and dead-letter growth.

### 11.3 Backup and service recovery

Production requires:

- D1 point-in-time recovery;
- R2 recovery controls;
- deployment-configuration backup;
- signing-key rotation;
- dead-letter replay;
- repository export;
- D1-to-R2 reconciliation;
- isolated recovery tests; and
- a disaster-recovery exercise.

A recovered head remains unavailable until all referenced objects pass coordinate,
length, and digest checks.

### 11.4 Desktop navigation

The managed-cloud UI contains:

- **Projects**;
- **Catalog**;
- **Invitations**;
- **History**;
- **Activity**; and
- account and device settings.

Projects and Catalog each contain **Public** and **Private** sections. The Public
section contains public repositories. The Private section contains only private
repositories for which the signed-in user is an active Admin or Member. It never
lists another user's private repository.

Each Catalog row is one item and one access boundary. It shows name, provider,
resource kind, visibility, and the signed-in user's role when applicable. Names such
as `petv2_rika` and `skill_codex_rena` are examples, not required formats.

An active Admin or Member sees **Review and push**, **Review and pull**, and
**History**.

### 11.5 Repository settings

Admins can manage name, visibility, public slug, Members, invitations, quota, audit
summary, and export.

The repository class, project identity, and Catalog item provider, resource kind, and
resource ID are read-only after creation.

Members can view repository identity, visibility, current head, history, and active
members.

### 11.6 Required messages

For a non-Git project:

> Managed Cloud V1 supports Git projects only.

For an unsupported provider-and-kind pair:

> This provider does not support that Catalog resource type.

For a head conflict:

> The repository changed after your review. Pull the current snapshot and review your
> changes again.

For attempted resource removal:

> Managed Cloud V1 cannot remove an accepted resource. The current resource remains.

Before first managed publication:

- Mallard stores the selected content in the managed service.
- Mallard does not end-to-end encrypt the content.
- Controlled service operators can access stored content.
- Credentials and complete provider homes must not be selected.
- Public repository content can be read by anyone.

Plugin disclosure also states that the cloud stores installation intent, not plugin
code, and that the desktop can run a reviewed provider operation during Pull.

## 12. Delivery and verification

### 12.1 Delivery slices

**Slice 0: Common contracts**

Deliver repository classes, shared head and object protocol, managed adapter, local
binding, keychain tokens, closed validators, review-plan digesting, and deterministic
service contract tests.

**Slice 1: Public Git project read**

Deliver Worker, D1, private R2, project repository creation by an internal publisher,
public listing, current-snapshot read, project Pull review, and public limits.

**Slice 2: Identity and private Git project publication**

Deliver OAuth, devices, rotating sessions, private creation, first Admin, project
Push, uploads, head compare-and-swap, history, quotas, and idempotency.

**Slice 3: Skills**

Deliver one-resource Catalog item creation, independent visibility and access, closed
provider-and-kind validation, Codex and Claude skill capture, public and private skill
Push and Pull, disclosure, and tests.

**Slice 4: Plugin install intent**

Add plugin items to the same one-resource model. Deliver portable intent validation,
marketplace-source validation, provider-derived Pull actions, manual-only outcomes,
public disclosure, and tests.

**Slice 5: Codex pets**

Add pet items to the same one-resource model. Deliver v1 and v2 validation, atlas
object handling, preview metadata, Push and Pull, public disclosure, and tests.

**Slice 6: Collaboration and operations**

Deliver invitations, role changes, last-Admin guard, Member publication, visibility
changes, featured examples, export, queues, reconciliation, metrics, alerts, backup,
and runbooks.

Each slice must work end to end before the next slice starts.

### 12.2 Test groups

Test:

- repository-class immutability;
- Git-only project admission and fingerprinting;
- closed Catalog provider-and-kind matrix;
- exactly one resource in each Catalog item repository;
- stable Catalog item provider, kind, and resource ID;
- independent visibility, membership, head, and history for two Catalog items;
- correct Public and Private Catalog grouping;
- skill path, file, digest, size, and executable-warning boundaries;
- plugin-intent source, namespace, ID, and provider-action boundaries;
- pet v1 and v2 format, size, dimension, and digest boundaries;
- visibility, roles, invitations, and last Admin;
- absence of repository-removal routes and states;
- rejection of resource omission and tombstones;
- zero-or-one head invariant;
- first publication and later compare-and-swap;
- two concurrent publications with one winner;
- linear immutable history;
- object length, digest, and create-only writes;
- cross-repository, cross-class, and cross-kind probing;
- historical public probing;
- review-plan expiry and reuse;
- idempotent creation, invitation, upload, publication, and export;
- OAuth, refresh rotation, device revocation, and invitation replay;
- access for anonymous, non-member, Member, and Admin;
- project Push and Pull review;
- Catalog item Push and Pull review;
- skill and pet preservation-copy behavior;
- plugin manual-only and provider-resolved behavior;
- public disclosure for every repository class and resource kind;
- partial D1, R2, queue, local apply, provider action, and receipt failures; and
- backup, recovery, export, and reconciliation.

Run:

- `npm run build`;
- `npm run test:frontend-integration`;
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`; and
- managed-service contract, deployment, and recovery tests.

### 12.3 Rollout

Use development, staging with synthetic data, internal featured examples, invited
private beta, production readiness review, and general release.

Do not add compatibility for an undeployed cloud design.

## 13. Production acceptance and future work

### 13.1 Production acceptance

V1 is ready when:

- A managed Project or Catalog item is the only access boundary.
- Repository class is `git_project` or `catalog` and cannot change.
- Git project creation rejects a project that is not Git-managed.
- Git source does not enter managed content.
- Catalog supports the closed provider-and-kind matrix.
- Each Catalog item repository contains exactly one item.
- Each Catalog item has its own public or private visibility.
- The Public and Private Catalog sections are views, not access containers.
- Skills, plugin intent, and pet packages pass their closed validators.
- Plugin code and credentials do not enter portable intent.
- Each repository is public or private.
- Public and private repositories can invite Members.
- Admin and Member are the only roles.
- One active Admin always remains.
- Each repository has zero or one current head.
- Anonymous users can read only a public current snapshot.
- Private content requires active membership.
- Any active Admin or Member can publish after the applicable review.
- Concurrent publication has one compare-and-swap winner.
- A stale publication cannot overwrite the current head.
- A publication cannot omit or tombstone an accepted resource.
- Pull cannot delete a local resource.
- The product has no repository-removal action.
- Managed content has no client-side or end-to-end encryption.
- Tokens stay out of normal JSON and logs.
- R2 is private and every read is authorized by the Worker.
- Immutable data passes exact length and SHA-256 checks.
- Project and Catalog item Push and Pull remain reviewed.
- Queue, reconciliation, and recovery exercises pass.
- Existing local-folder and user-owned S3/R2 behavior remains unchanged.

### 13.2 Future work

Future releases can consider:

- multi-item Catalog collections;
- shared visibility or access across repositories;
- multiple managed heads;
- content approval;
- additional roles;
- non-Git projects;
- ordinary project-file cloud sync;
- additional Catalog providers and resource kinds;
- Claude pets;
- pet format conversion;
- controlled resource and repository removal;
- Git-host write integration;
- web write clients;
- billing;
- public search and caching;
- direct uploads;
- cross-repository deduplication;
- regional scaling; and
- client-controlled content encryption.

Future features require a new reviewed product and schema design. V1 contains no
dormant service fields for them.

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
