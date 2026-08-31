# Mallard Managed Cloud V1

## Repository-only implementation plan

- **Status:** Proposed
- **Date:** August 30, 2026
- **Scope:** Git projects, public/private repositories, repository collaboration

This document is the complete V1 plan. It contains only the current design.

## 1. V1 product

Mallard Managed Cloud stores reviewed, project-scoped Codex and Claude state so that
users can move it between machines.

The repository is the only access boundary. Each repository has:

- public or private visibility;
- Admins and Members;
- invitations;
- one current accepted head;
- immutable publication history;
- quotas;
- audit history.

### 1.1 Fixed decisions

| Question | V1 decision |
| --- | --- |
| Supported project | Git project |
| Access boundary | Repository |
| Visibility | Public or private |
| Roles | Admin and Member |
| First Admin | Repository creator |
| Private collaboration | Admin invites Members |
| Public editing | Admin invites Members |
| Public read | Current accepted snapshot |
| Content pointer | One repository head |
| Who can publish | Active Admin or Member |
| Concurrent publication | Exact-head compare-and-swap |
| Stale publication | Reject and require a new review |
| Git source | Remains in the configured Git remote |
| Repository removal | No product UI or API |
| Accepted resource removal | Not supported |
| Application content encryption | None |

### 1.2 Included

V1 includes:

- Git-project validation and fingerprinting;
- public catalog and featured demo;
- private repositories;
- repository invitations;
- Admin and Member management;
- review-first Push and Pull;
- one current accepted repository head;
- immutable retained history;
- optimistic concurrency;
- repository export;
- OAuth sign-in;
- quotas, audit, monitoring, and recovery.

### 1.3 Not included

V1 does not include:

- non-Git projects;
- ordinary project-file cloud sync;
- parent access containers;
- shared membership across repositories;
- roles other than Admin and Member;
- multiple managed heads;
- content approval queues;
- automatic combination of concurrent snapshots;
- Git source hosting or Git-host writes;
- repository-removal workflows;
- removal of accepted conversations or provider state;
- a web write client;
- billing;
- shared public object caching;
- client-side content encryption; or
- end-to-end content encryption.

## 2. Repository access model

### 2.1 Repository identifiers

Each repository has:

- **Bundle ID:** Portable random ID stored in bundle documents.
- **Service repository ID:** Opaque ID used by the managed service.
- **Public slug:** Bounded catalog locator. It is visible only when the repository is
  public.
- **Repository fingerprint:** SHA-256 of the normalized Git `origin` URL.

The service repository ID, account IDs, invitation IDs, local paths, and device IDs
must not enter portable content.

### 2.2 Visibility

| Visibility | Anonymous read | Member read | Publish |
| --- | --- | --- | --- |
| Public | Current accepted snapshot | Yes | Active Admin or Member |
| Private | No | Yes | Active Admin or Member |

Anonymous public access does not include:

- retained history;
- unaccepted uploads;
- review plans;
- invitations;
- membership data;
- audit events;
- local paths; or
- service administration data.

Every public read checks current D1 state and returns `Cache-Control: no-store`.

### 2.3 Roles

| Capability | Public reader | Member | Admin |
| --- | --- | --- | --- |
| Read current public snapshot | Yes | Yes | Yes |
| Read private snapshot | No | Yes | Yes |
| Read retained history | No | Yes | Yes |
| Publish after Push review | No | Yes | Yes |
| Invite or remove a Member | No | No | Yes |
| Promote or demote an Admin | No | No | Yes |
| Change visibility | No | No | Yes |
| Export retained content | No | No | Yes |

Rules:

- `admin` and `member` are the only role values.
- The creator becomes the first Admin.
- Each repository must keep one active Admin.
- An invitation grants Member.
- An Admin can promote an accepted Member.
- A role applies to one repository.
- Public read does not create membership.
- Admin status does not bypass review or safety checks.

### 2.4 Invitations

An Admin can invite a Member to a public or private repository.

The invitation stores:

- repository ID;
- HMAC-SHA-256 of normalized verified email;
- masked email label;
- random-token hash;
- creator ID;
- creation and expiry times;
- state; and
- accepted account ID when complete.

The raw token exists only in the email.

Acceptance requires a signed-in account with the matching verified email, an active
repository, a pending unexpired invitation, and a valid one-use token.

One transaction creates or reactivates Member membership, consumes the invitation,
increments repository revision, and writes audit.

Invitation states are `pending`, `accepted`, `revoked`, and `expired`.

### 2.5 Public demo

The demo is an ordinary public repository with a `featured` flag.

Anonymous users can discover and Pull its current snapshot. Editors must accept a
repository invitation. The featured flag changes catalog placement only.

### 2.6 Visibility changes

Repository creation always starts private.

Private-to-public requires:

1. current full snapshot review;
2. matching repository revision;
3. matching head ETag;
4. recent authentication;
5. plaintext-storage acknowledgement;
6. public-disclosure acknowledgement; and
7. audit.

Public-to-private changes one repository row in one transaction. All later anonymous
reads return generic not-found. No cache purge is required.

## 3. Git and repository head

### 3.1 Git admission

The desktop accepts a managed repository only when:

- the project is a valid Git work tree;
- the canonical Git root is safe;
- `origin` exists;
- the origin URL can be normalized without a credential;
- the repository fingerprint is valid; and
- the current Git commit ID can be read.

The service returns `unsupported_repository_kind` for another project type.

### 3.2 Source boundary

Git transports source files and Git history.

Mallard transports:

- selected project-scoped provider conversations;
- selected supported provider state;
- portable capture and apply recipes;
- current Git commit mapping; and
- Mallard publication history.

Mallard does not upload or modify Git source.

### 3.3 Repository head

The repository head is the only mutable content pointer. It contains:

- bundle ID;
- publication revision;
- bundle commit ID;
- current Git commit ID;
- manifest logical key;
- manifest SHA-256;
- manifest byte length;
- update time; and
- opaque ETag.

The first publication compares against an absent head. Every later publication
compares against the exact ETag that the user reviewed.

### 3.4 Publication history

Each successful publication creates one immutable bundle commit. It records the
previous bundle commit ID and forms a linear retained history.

A new publication can add or replace resources. It must retain every resource from
the current head that it does not replace. V1 does not publish resource tombstones.

Active Admins and Members can read retained history. Anonymous users can read only the
current accepted public snapshot.

### 3.5 Concurrent publication

If two users review the same head:

1. both plans pin the same ETag;
2. the first valid publication changes the head;
3. the second publication receives `head_changed`;
4. the second publication cannot overwrite the winner;
5. the losing user reads the current snapshot; and
6. the losing user performs a new Push review.

The service does not combine concurrent snapshots.

## 4. Portable data and desktop review

### 4.1 Portable layout

~~~text
.mallard/
|-- _storage.json
`-- v1/repositories/<bundle-id>/
    |-- _head.json
    |-- _tag.json
    |-- _objects/<payload-sha256>
    |-- _manifests/<revision>-<bundle-commit-id>.json
    `-- _commits/<revision>-<bundle-commit-id>.json
~~~

The namespace is closed. Reject absolute keys, empty or dot segments, backslashes,
controls, overlength keys, unknown roots, invalid IDs, and an unauthorized bundle ID.

### 4.2 Document roles

All documents use UTF-8, bounded fields, UTC RFC 3339 times, lowercase hexadecimal
SHA-256 values, and `schema_version: 1`.

`_storage.json` has:

~~~json
{"format":"mallard-storage","layout_version":1}
~~~

`_tag.json` is non-authoritative listing metadata.

`_head.json` is the mutable pointer defined in section 3.3.

The immutable manifest contains:

- bundle ID and display name;
- repository fingerprint;
- publication revision;
- bundle commit ID;
- current Git commit ID;
- update time;
- privacy-preserving publishing origin; and
- reviewed resource scopes.

Each scope has a revision, canonical digest, capture and apply recipe, resource
descriptors, file entries, directory entries, conversation entries, and tombstones.

Managed Cloud V1 permits selected provider conversations and supported provider
state. It rejects ordinary project files, project directories, and all tombstones.

The immutable bundle commit contains publication coordinates, previous bundle commit
ID, Git commit ID, manifest digest, counts, origin class, and creation time.

Payload objects use `_objects/<sha256>`. Bytes and recorded length must match the key
and metadata.

### 4.3 Publication order

One publication:

1. validates repository, actor, review plan, and expected head;
2. verifies every retained object;
3. detects a semantic no-op;
4. writes new payloads with create-only semantics;
5. writes the immutable manifest;
6. writes the immutable bundle commit;
7. changes the repository head by compare-and-swap;
8. updates the non-authoritative tag; and
9. records accepted local bases after head success.

Only the head change accepts the publication.

### 4.4 Push review

The desktop:

1. validates the Git binding and fingerprint;
2. reads the current head and manifest;
3. discovers allowed provider resources;
4. captures candidates into private temporary files;
5. compares local, accepted-base, and remote state;
6. shows additions, changes, retained items, conflicts, and warnings;
7. lets the user select exact resources;
8. requires applicable warnings;
9. pins configuration, repository revision, visibility, head ETag, source digests,
   selection, acknowledgements, and expiry;
10. rechecks all pins; and
11. publishes in the order in section 4.3.

If the head changed, the user must read the new snapshot and review again.

### 4.5 Pull review

The desktop:

1. reads the exact authorized head;
2. verifies the manifest and selected objects;
3. compares remote content with local content and accepted bases;
4. shows creates, overwrites, preserved items, conflicts, warnings, and manual
   actions;
5. lets the user approve exact actions;
6. pins head ETag, manifest digest, local target digests, selection, acknowledgements,
   and expiry;
7. rechecks all pins;
8. creates backups;
9. applies with atomic replacement where supported; and
10. records outcomes.

Pull plans stay machine-local.

### 4.6 Safety boundaries

Portable content must not contain credentials, complete provider homes, absolute
paths, local trust decisions, service IDs, local backups, temporary files, special
files, Git source files, or content-encryption keys.

Managed Cloud Push and Pull do not delete accepted or local resources in V1.

Access and refresh tokens live only in the operating-system keychain.

### 4.7 Managed adapter

The managed adapter is bound to service environment, service URL, repository ID, and
keychain session reference. It has no R2 credential.

It maps `_head.json` reads and compare-and-swap to D1 service operations. It maps
immutable manifests, bundle commits, and payloads through the Worker to R2. Local
folder and user-owned S3/R2 storage keep their existing behavior.

## 5. Cloud architecture

### 5.1 Topology

~~~mermaid
flowchart LR
    PUBLIC["Anonymous reader"]
    APP["Mallard desktop"]
    EDGE["Cloudflare domain<br/>TLS, firewall, rate limits"]
    API["API Worker<br/>identity, authorization, object gateway"]
    IDP["GitHub and Google OAuth"]
    D1["D1<br/>accounts, repositories, head,<br/>history, quotas, jobs, audit"]
    R2["Private R2<br/>immutable objects and staging"]
    QUEUE["Queue and dead-letter queue"]
    CRON["Cron Trigger"]
    EMAIL["Invitation email"]
    INTERNAL["Access-protected internal routes"]
    OBS["Sanitized logs and metrics"]

    PUBLIC --> EDGE
    APP --> EDGE
    EDGE --> API
    API <--> IDP
    API --> D1
    API --> R2
    API --> QUEUE
    API --> EMAIL
    CRON --> D1
    CRON --> QUEUE
    INTERNAL --> D1
    INTERNAL --> QUEUE
    API --> OBS
~~~

Every R2 read and write passes through the Worker. R2 has no public bucket or custom
domain.

Development, staging, and production use separate domains, deployments, databases,
buckets, queues, OAuth applications, signing keys, email credentials, operational
identities, and log sinks.

### 5.2 Component ownership

The Worker owns:

- OAuth callbacks and session rotation;
- repository authorization;
- public discovery;
- invitations and membership;
- review plans;
- upload validation;
- head compare-and-swap;
- history metadata;
- quota reservation;
- idempotency;
- audit; and
- bounded errors.

D1 is authoritative for identity, authorization, repository metadata, current head,
history reachability, quotas, jobs, and audit metadata.

R2 is authoritative for immutable bytes at an exact internal key.

Queue and Cron jobs handle expiry, unreachable-object cleanup, tag repair,
reconciliation, export expiry, and dead-letter retry. Jobs are idempotent and
resumable.

### 5.3 Internal storage keys

~~~text
repos/<repository-id>/objects/<sha256>
repos/<repository-id>/manifests/<revision>-<bundle-commit-id>.json
repos/<repository-id>/commits/<revision>-<bundle-commit-id>.json
staging/<repository-id>/<upload-id>/<object-id>
exports/<repository-id>/<export-id>
~~~

The Worker constructs keys from validated IDs. A client cannot submit a complete
internal key. An R2 listing never decides access.

### 5.4 Upload and publication transaction

An upload session pins repository, actor, repository revision, head ETag, visibility,
object hashes and lengths, quota reservation, expiry, and idempotency key.

The Worker streams each object, enforces exact length, computes SHA-256, and writes
with create-only semantics. Incomplete data remains unreachable in staging.

The final D1 transaction:

1. reauthorizes active Admin or Member;
2. rechecks repository revision and visibility;
3. rechecks expected head ETag;
4. verifies the unused review plan;
5. verifies immutable-object metadata;
6. inserts publication metadata;
7. advances the head by compare-and-swap;
8. updates current and historical reachability;
9. commits quota;
10. consumes the plan;
11. records idempotency; and
12. writes audit.

If compare-and-swap fails, the current head remains unchanged.

### 5.5 Read path

The Worker resolves the repository, authorizes active membership or
current-public-snapshot access, validates the logical coordinate, resolves exact
metadata, reads R2, verifies length, and streams bytes with digest and ETag.

The desktop independently verifies length and SHA-256.

Anonymous users can read only the current public snapshot. Historical and unaccepted
objects require active membership.

### 5.6 Idempotency, quotas, and reconciliation

An idempotency record stores user, operation, key hash, canonical request hash,
result, and expiry. Reusing the same key with different input returns
`idempotency_mismatch`.

V1 limits repository count, account bytes, repository bytes and objects, concurrent
uploads, object size, manifest size, resource count, retained history, and audit
retention.

Reconciliation detects missing bytes, untracked bytes, expired uploads and
reservations, unreachable objects, head disagreement, and tag drift. Cleanup can
remove only unreachable objects that pass exact repository, reachability, and age
checks.

## 6. D1 model

### 6.1 Tables

| Table | Purpose |
| --- | --- |
| `users`, `identities`, `devices`, `refresh_sessions` | Accounts and sessions |
| `repositories` | ID, slug, bundle ID, fingerprint, name, visibility, revision, featured flag |
| `repository_heads` | Zero or one current head per repository |
| `repository_publications` | Immutable linear publication history |
| `repository_members` | Repository Admin or Member |
| `repository_invitations` | Email HMAC, masked label, token hash, expiry, state |
| `review_plans` | Push or disclosure digest, pins, acknowledgements, expiry |
| `objects`, `upload_sessions` | R2 metadata, reachability, and staging |
| `quota_ledgers` | Account and repository reserved and committed use |
| `idempotency_records` | Safe mutation replay |
| `repository_exports` | Expiring repository exports |
| `audit_events` | Sanitized security history |

The repository is the only access scope. A repository can have no head before its
first publication and exactly one head afterward.

### 6.2 Closed values

| Field | Values |
| --- | --- |
| User state | `active`, `suspended` |
| Visibility | `public`, `private` |
| Role | `admin`, `member` |
| Membership state | `active`, `suspended`, `removed` |
| Invitation state | `pending`, `accepted`, `revoked`, `expired` |
| Plan state | `active`, `used`, `expired`, `revoked` |
| Upload state | `created`, `receiving`, `ready`, `committed`, `expired`, `failed` |
| Reachability | `current`, `historical`, `unreachable` |
| Job state | `pending`, `running`, `retry`, `complete`, `dead_letter` |

### 6.3 Required constraints

Enforce:

- one identity per provider subject;
- one repository per bundle ID;
- one active public slug;
- one active membership per repository and user;
- zero or one current head;
- one publication revision and bundle commit ID per repository;
- one live object coordinate per repository;
- non-negative quota values;
- one active Admin in each repository;
- public discovery only for public repositories;
- current head that references verified publication data;
- a linear previous-commit chain;
- no omission of a resource from the current head; and
- no stale-head publication.

The repository fingerprint is not globally unique. This avoids revealing that another
private repository uses the same Git origin.

### 6.4 Atomic operations

| Operation | Transaction |
| --- | --- |
| Create repository | Private repository, creator Admin, quotas, idempotency, audit |
| Accept invitation | Membership, consumed invitation, repository revision, audit |
| Change membership | Role/state, last-Admin proof, repository revision, audit |
| Change visibility | Visibility, repository revision, disclosure proof, audit |
| Publish | Membership, plan, head CAS, publication, reachability, quota, idempotency, audit |

## 7. Service API

The base path is `/v1`. JSON uses `snake_case`. Private reads and all writes require a
bearer access token. Mutations include expected revisions and an idempotency key where
replay matters.

### 7.1 Identity and public routes

| Method | Route | Purpose |
| --- | --- | --- |
| `POST` | `/v1/auth/start` | Start OAuth with PKCE |
| `POST` | `/v1/auth/callback` | Complete one callback |
| `POST` | `/v1/auth/refresh` | Rotate refresh token |
| `POST` | `/v1/auth/logout` | Revoke current session |
| `GET` | `/v1/me` | Account, devices, repositories, limits |
| `GET` | `/v1/public/repositories` | Public catalog |
| `GET` | `/v1/public/repositories/{slug}` | Public summary |
| `GET` | `/v1/public/repositories/{slug}/head` | Current public head |
| `GET` | `/v1/public/repositories/{slug}/manifest/{bundle_commit_id}` | Current public manifest |
| `GET` | `/v1/public/repositories/{slug}/objects/{sha256}?commit_id={bundle_commit_id}` | Current public object |

### 7.2 Repository and access routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET`, `POST` | `/v1/repositories` | List memberships or create private repository |
| `GET`, `PATCH` | `/v1/repositories/{repository_id}` | Read or change metadata |
| `POST` | `/v1/repositories/{repository_id}/visibility` | Make public or private |
| `POST` | `/v1/repositories/{repository_id}/exports` | Export retained content |
| `GET` | `/v1/repositories/{repository_id}/members` | Active and invited access |
| `POST` | `/v1/repositories/{repository_id}/invitations` | Invite Member |
| `POST` | `/v1/repositories/{repository_id}/invitations/{invitation_id}/revoke` | Revoke invitation |
| `POST` | `/v1/invitations/accept` | Accept token from request body |
| `PATCH`, `DELETE` | `/v1/repositories/{repository_id}/members/{user_id}` | Change or remove membership |
| `POST` | `/v1/repositories/{repository_id}/leave` | Leave under last-Admin rule |

Repository creation accepts `kind: git`, bundle ID, repository fingerprint, display
name, and idempotency key. Invitation acceptance sends the raw token in the request
body. Raw invitation tokens must not enter URLs or logs.

### 7.3 Content routes

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/v1/repositories/{repository_id}/head` | Current head |
| `GET` | `/v1/repositories/{repository_id}/history` | Retained history |
| `GET` | `/v1/repositories/{repository_id}/publications/{bundle_commit_id}` | Publication metadata |
| `POST` | `/v1/repositories/{repository_id}/push-plans` | Register Push review |
| `POST` | `/v1/repositories/{repository_id}/disclosure-plans` | Register public disclosure review |
| `POST` | `/v1/repositories/{repository_id}/uploads` | Start bounded upload |
| `PUT` | `/v1/repositories/{repository_id}/uploads/{upload_id}/objects/{object_id}` | Upload expected object |
| `POST` | `/v1/repositories/{repository_id}/uploads/{upload_id}/complete` | Verify upload |
| `POST` | `/v1/repositories/{repository_id}/publish` | Publish by head CAS |
| `GET` | `/v1/repositories/{repository_id}/objects/{object_id}` | Authorized object |

Creating a plan or upload does not change the head.

### 7.4 Errors

~~~json
{
  "error": {
    "code": "head_changed",
    "message": "The repository changed. Read the current snapshot and review again.",
    "request_id": "req_opaque"
  }
}
~~~

Stable codes are:

- `authentication_required`;
- `recent_authentication_required`;
- `not_found`;
- `forbidden`;
- `invalid_request`;
- `unsupported_repository_kind`;
- `repository_fingerprint_mismatch`;
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

Errors must not reveal private existence, raw email, content, local paths,
credentials, or unauthorized identifiers.

## 8. Identity, privacy, and security

### 8.1 Sign-in and sessions

V1 supports GitHub and Google OAuth. The desktop uses the system browser,
Authorization Code, and Proof Key for Code Exchange (PKCE).

The service validates issuer, audience, code, PKCE verifier, state, redirect URI,
nonce where applicable, provider subject, verified email, expiry, and callback replay.

The API issues a short-lived signed access token and rotating opaque refresh token.
D1 stores refresh-token hashes. Token reuse revokes the session family.

Tokens live only in the operating-system keychain.

### 8.2 Authorization

For each repository request:

1. parse and bound input;
2. resolve repository;
3. check state;
4. allow only current-snapshot reachability for a public read;
5. otherwise authenticate;
6. load active membership;
7. check role;
8. check repository revision and head ETag where required;
9. validate object coordinates; and
10. perform the guarded operation.

An unauthorized private lookup returns generic not-found.

### 8.3 Plaintext content

Mallard stores portable content without client-side or end-to-end encryption. The
service can inspect it, and authorized operators can read it during controlled
operations.

V1 has no content keys, device-key approval, key rotation, recovery phrase, encrypted
index, or encrypted-content API.

HTTPS and provider storage security remain required. They are not end-to-end
encryption.

### 8.4 Safety and logging

The Worker validates sizes, counts, identifiers, hashes, lengths, coordinates,
authorization, review-plan digest, warnings, revisions, and expiry.

Apply public-read, account-write, repository-write, invitation, and OAuth rate limits.
Use constant-time token-hash comparison and sanitized structured logs.

Never log credentials, tokens, authorization headers, raw emails, provider content,
manifests, object bodies, Git URLs, or local paths.

Production access uses least-privilege service identities. Content access is
reasoned, time-bounded where supported, and audited.

## 9. Workflows and failure behavior

### 9.1 First sign-in

1. The desktop starts OAuth with PKCE and one-use state.
2. The system browser completes authentication.
3. The Worker validates the callback.
4. The service creates or resumes user and identity.
5. The service registers device and refresh-session family.
6. The desktop stores tokens in the keychain.

No repository is created automatically.

### 9.2 Create repository

1. The desktop validates Git root, origin, fingerprint, and current Git commit.
2. The API creates a private repository and first Admin.
3. Push review selects initial portable content.
4. The desktop registers the plan and uploads immutable objects.
5. Publication creates the first head by compare-and-swap against absence.
6. The desktop stores the local repository binding.

For a desired public result, the desktop completes public disclosure after the first
head is accepted.

### 9.3 Invite Member

1. An Admin submits an email.
2. The API stores email HMAC, masked label, and token hash.
3. The email provider sends the raw token.
4. The recipient signs in with the matching verified email.
5. One transaction creates Member membership and consumes the invitation.

The Member can read and publish reviewed snapshots.

### 9.4 Publish snapshot

1. The user reads the current head.
2. Push review compares local and remote state.
3. The user selects resources and accepts warnings.
4. The desktop registers the plan and uploads immutable objects.
5. The publication transaction rechecks membership, repository revision, visibility,
   plan, objects, quota, and head ETag.
6. Compare-and-swap advances the head.
7. The snapshot becomes current.

For a public repository, review states that the complete snapshot becomes public.

### 9.5 Head conflict

1. Two users review ETag `A`.
2. User 1 publishes and creates ETag `B`.
3. User 2 submits a plan pinned to `A`.
4. The API returns `head_changed`.
5. User 2 reads `B` and performs a new review.

The second publication cannot overwrite the first.

### 9.6 Pull snapshot

1. The desktop reads the authorized head.
2. It verifies the manifest and selected objects.
3. Pull review shows exact local actions.
4. The user approves actions.
5. The desktop creates backups and applies.
6. The desktop records outcomes and accepted bases.

Anonymous Pull is limited to a public current snapshot.

### 9.7 Public demo

An internal publisher creates the demo as private, publishes its reviewed initial
head, completes disclosure review, changes visibility to public, and sets the featured
flag.

Anonymous users can discover and Pull it. Editing requires invitation.

### 9.8 Failure rules

| Failure | Result |
| --- | --- |
| OAuth validation fails | No session |
| D1 fails before commit | Retryable error; no state change |
| R2 succeeds but publication fails | Unaccepted immutable data; later cleanup |
| Publication succeeds but response is lost | Idempotent replay returns recorded result |
| Repository revision changed | No publication; fresh review |
| Head ETag changed | `head_changed`; winner remains |
| Object digest or length differs | No publication |
| Invitation delivery fails | Revoke undelivered token |
| Queue job runs twice | No duplicate change |
| Public head changes during anonymous Pull | Restart from current head |
| Visibility becomes private during public read | Deny later object read |
| Local target changes after Pull review | Block affected action |
| Receipt fails after local apply | Report reconciliation; do not repeat unsafe action |

No failure can publish private data, overwrite a changed head, apply an unreviewed
local action, or delete a resource.

## 10. Operations and UX

### 10.1 Default retention

| Data | Default |
| --- | --- |
| Pending invitation | 7 days |
| Upload staging | 24 hours |
| Unaccepted objects | 7 days |
| Export | 24 hours |
| Idempotency record | 24 hours |
| Sanitized audit | 365 days |

Retained publication history is bounded by repository quota and service retention.

### 10.2 Audit and monitoring

Audit records opaque actor and target IDs, repository, action, result, bounded reason,
request ID, build ID, and time.

Audit covers identity, invitations, roles, visibility, review plans, publication,
conflicts, integrity, quota, and export.

Monitor request latency, authentication, public denials, invitation delivery,
publication conflicts, integrity, D1/R2 failures, queue health, quota, and
reconciliation.

Alert on private-resource probing, refresh-token reuse, integrity failure, dead
letters, invalid quota state, missing current objects, and unauthorized public read.

### 10.3 Backup and recovery

Production requires:

- D1 backup or point-in-time recovery;
- R2 recovery controls;
- deployment-configuration backup;
- signing-key rotation;
- dead-letter replay;
- repository export;
- D1-to-R2 reconciliation;
- isolated recovery tests; and
- a disaster-recovery exercise.

A recovered head remains unavailable until referenced objects pass coordinate,
length, and digest checks.

### 10.4 Desktop navigation

The managed-cloud UI contains:

- **Public repositories**;
- **My repositories**;
- **Invitations**;
- **History**;
- **Activity**; and
- account and device settings.

An active Admin or Member sees **Review and push**, **Review and pull**, and
**History**.

### 10.5 Repository settings

Admins can manage name, visibility, public slug, Members, invitations, quota, audit
summary, and export.

Members can view repository identity, visibility, current head, history, and active
members.

### 10.6 User messages

For a non-Git project:

> Managed Cloud V1 supports Git projects only.

For a head conflict:

> The repository changed after your review. Pull the current snapshot and review your
> changes again.

Before first managed publication:

- Mallard stores selected content in the managed service.
- Mallard does not end-to-end encrypt it.
- Controlled service operators can access stored content.
- Credentials and complete provider homes must not be selected.
- Public repository content can be read by anyone.

## 11. Delivery and verification

### 11.1 Delivery slices

**Slice 0: Contracts**

Deliver portable validation, managed adapter, local binding, keychain tokens,
Git-project admission, fingerprinting, review plans, and deterministic service tests.

**Slice 1: Public read**

Deliver Worker, D1, private R2, repository head, internal demo publisher, public
catalog, current-snapshot reads, Pull review, and public limits.

**Slice 2: Identity and private publication**

Deliver OAuth, devices, rotating sessions, private creation, first Admin, uploads,
head compare-and-swap, history, quotas, and idempotency.

**Slice 3: Collaboration**

Deliver invitations, role changes, last-Admin guard, Member publication,
head-conflict recovery, settings, and audit.

**Slice 4: Operations**

Deliver visibility transitions, export, queues, reconciliation, metrics, alerts,
backup, and runbooks.

Each slice must work end to end before the next slice starts.

### 11.2 Test groups

Test:

- visibility, roles, invitations, and last Admin;
- absence of repository-removal routes and states;
- rejection of resource omission and tombstones;
- zero-or-one head invariant;
- first publication and later compare-and-swap;
- two concurrent publications with one winner;
- linear immutable history;
- portable coordinates and closed keys;
- Git-project admission and fingerprinting;
- review-plan expiry and reuse;
- object length, digest, and create-only writes;
- cross-repository and historical public probing;
- OAuth, refresh rotation, device revocation, and invitation replay;
- access for anonymous, non-member, Member, and Admin;
- Push review, Pull review, head-conflict recovery, and disclosures;
- partial D1, R2, queue, cleanup, local apply, and receipt failures; and
- backup, recovery, export, and reconciliation.

Run:

- `npm run build`;
- `npm run test:frontend-integration`;
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`; and
- managed-service contract, deployment, and recovery tests.

### 11.3 Rollout

Use development, staging with synthetic data, internal demo, invited private beta,
production readiness review, and general release.

Do not add compatibility for an undeployed cloud design.

## 12. Acceptance and future work

### 12.1 Production acceptance

V1 is ready when:

- Repository is the only access boundary.
- Each repository is public or private.
- The product has no repository-removal action.
- Managed Cloud Push and Pull cannot remove resources.
- Each repository has zero or one current head.
- Anonymous users can read only a public current snapshot.
- Private data requires active membership.
- Public and private repositories can invite Members.
- The demo uses normal public-repository rules.
- Admin and Member are the only roles.
- One Admin always remains.
- Any active Admin or Member can publish after Push review.
- Concurrent publication has one compare-and-swap winner.
- A stale publication cannot overwrite.
- Managed-cloud creation rejects non-Git projects.
- Git source does not enter portable content.
- Public-to-private needs no cache purge.
- Managed content has no client-side or end-to-end encryption.
- Tokens stay out of normal JSON.
- R2 is private and every read is authorized.
- Immutable data passes exact length and SHA-256 checks.
- Push and Pull remain reviewed.
- Logs and audit contain no credential or user content.
- Queue and recovery exercises pass.
- Existing local-folder and user-owned S3/R2 behavior remains unchanged.

### 12.2 Future work

Future releases can consider:

- repository grouping;
- shared access across repositories;
- multiple managed heads;
- content approval;
- additional roles;
- non-Git projects;
- ordinary project-file cloud sync;
- Git-host write integration;
- web write clients;
- billing;
- public search and caching;
- direct uploads;
- cross-repository deduplication;
- regional scaling; and
- client-controlled content encryption.

Future features require a new reviewed product and schema design. V1 contains no
dormant fields for them.

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
