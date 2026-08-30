# Mallard Managed Cloud V1: Public and Private Git Collaboration

- **Status:** Implementation proposal
- **Last revised:** August 30, 2026
- **Scope:** Production foundation for the Git-only first release of a
  Mallard-operated managed cloud
- **Implementation baseline:** Mallard 0.1.2 has machine-local schema 1,
  portable bundle schema 1, local-folder storage, and user-owned S3/R2
  storage. Managed cloud is not implemented.
- **Authority:** This file defines the complete managed-cloud target. An
  implementer does not need an earlier plan to use it.

## Reading guide

- Sections 1-4 define product, access, branch, and MR decisions.
- Section 5 defines the portable bundle and desktop safety contract.
- Sections 6-10 define cloud infrastructure, trust, storage, D1, and API.
- Sections 11-14 define user flows, failure handling, operations, and UI.
- Sections 15-18 define delivery, tests, release criteria, and deferred work.
- Section 19 lists primary vendor and identity references.

## 1. Decision

Mallard will offer a managed cloud service on Cloudflare. The service will use
Workers for the API, D1 for identity and collaboration state, and private R2
storage for immutable bundle objects.

The managed cloud will support:

- public and private workspaces;
- public and private repositories;
- invitation-only write access;
- two workspace roles: Admin and Member;
- configurable protected branches for Git projects;
- direct Member pushes to branches that are not protected;
- merge requests for Member changes to protected Git branches;
- manual merge-request rejection by an Admin; and
- automatic rejection when a controlled target has a conflict.

Managed Cloud V1 supports Git repositories only. Repository creation rejects
a project that is not Git-managed. Non-Git cloud repositories and their
review workflow are deferred.

Mallard will not encrypt repository content in the application. The service
can read stored manifests, conversations, metadata, and payloads. Standard
HTTPS transport security, Cloudflare storage protection, token signing, token
hashing, and operating-system credential storage remain mandatory. They are
platform security controls, not Mallard content encryption.

The following properties are mandatory:

1. The desktop application never receives managed R2 credentials or a service
   administration token.
2. The schema-1 bundle contract in section 5 remains the portable data model.
3. Every write is authenticated and authorized in D1.
4. A public identifier does not grant write access.
5. A private identifier does not grant read access.
6. All direct head changes use compare-and-swap.
7. All MRs pin an exact source head, target head,
   policy revision, and chat.
8. A conflicting MR is rejected. Mallard does not auto-merge it.
9. The Push review and Pull review defined in sections 5.5 and 5.6 remain
   mandatory.

## 2. Requirement resolution

This table converts the product notes into explicit rules.

| Product note | Decision |
| --- | --- |
| Remove encryption | Remove client-side and end-to-end content encryption. Remove key epochs, envelopes, recovery keys, device key approval, and ciphertext APIs. |
| Demo repository | Use an ordinary public repository in a public Mallard workspace. Anonymous users can read it. Invited workspace Members can edit it under branch policy. |
| Public workspace, invitation to write | Anyone can read public content. Only an accepted Admin or Member can write. Joining a workspace always requires an invitation. |
| Public / Private | Both workspaces and repositories have visibility. A repository is publicly readable only when both values are public. |
| Combine Owner and Admin | Use only Admin. Do not retain Owner as a role or compatibility alias. |
| Member | A Member can read workspace content, push to branches that are not protected, and open MRs for protected branches. A Member cannot administer the workspace. |
| Non-Git repository | Exclude it from V1. Managed repository creation accepts Git projects only. |
| Git main | Protect the primary branch by default. The initial UI value is main. A Member must raise a merge request. |
| Git develop | Develop is not protected by default. A Member can push directly after the normal local Push review. |
| Admin branch configuration | An Admin can protect or unprotect exact Git branch names. A Member cannot push directly to a protected branch. |
| Member MR for a protected branch | Supported. The app must start one new provider chat and one MR-scoped managed source ref. An existing chat cannot be attached. |
| Merge conflict | Reject the MR immediately. Do not add rebase, auto-merge, or conflict-resolution UI. |
| Reject MR or pull request | An Admin can reject an open MR. The rejection is final and audited. The user must start a new MR and a new chat. |

## 3. Product and access model

### 3.1 Terms

**Workspace**

A workspace is the membership and administration boundary. A role applies to
all repositories in that workspace.

**Managed repository**

A managed repository stores one Mallard project bundle and its managed-cloud
refs. It does not replace the project's Git source repository.

**Ref**

A ref is a managed-cloud pointer to one schema-1 bundle head. Each ref maps to
one validated Git branch name.

**Controlled target**

A controlled target is a protected Git branch.

**Merge request**

A merge request, or MR, proposes a Member change to a protected Git branch.
Some Git hosts call this a pull request. A Mallard MR changes Mallard bundle
state. It does not merge source code in the Git host.

**Provider chat**

A provider chat is one project-scoped Codex or Claude conversation. Mallard
assigns it a portable cloud thread ID. The cloud thread ID is an opaque
Mallard identifier. It is not a machine-local provider session ID.

**Repository fingerprint**

The repository fingerprint is SHA-256 of the normalized origin remote URL.
The desktop obtains the URL with Git, removes URL user information, normalizes
equivalent GitHub URL forms, and hashes the result. It never uploads the raw
remote URL. Managed repository creation fails when the project has no usable
origin remote or the fingerprint cannot be computed.

The featured demo catalog is the only V1 exception. An internal publisher can
attach a separately reviewed public HTTPS clone URL to a featured repository.
The URL must contain no user information, query, fragment, or credential.

### 3.2 Visibility

Workspaces and repositories each have one visibility value:

- public; or
- private.

A repository has effective public visibility only when:

~~~text
workspace.visibility = public
AND repository.visibility = public
AND workspace.state = active
AND repository.state = active
~~~

The access result is:

| Workspace | Repository | Anonymous read | Member read | Write |
| --- | --- | --- | --- | --- |
| Public | Public | Yes | Yes | Invitation and role policy |
| Public | Private | No | Yes | Invitation and role policy |
| Private | Private | No | Yes | Invitation and role policy |
| Private | Public | Invalid configuration | Not applicable | Not applicable |

The service must not allow a public repository in a private workspace.

Visibility changes use these rules:

- Only an Admin can change visibility.
- Changing a private workspace to public does not publish any repository.
  Each repository remains private until an Admin publishes it.
- Changing a public workspace to private atomically changes all repositories
  to private.
- Changing a repository from private to public requires a current full Push
  review and explicit acknowledgement of public disclosure.
- Changing a public resource to private increments its visibility generation
  before the API returns success. Old public cache keys then become unusable.
- A best-effort cache purge follows the visibility transaction.
- The service never exposes a private resource because the caller knows its
  workspace, repository, ref, object, or MR ID.

Public repository content can include conversations and provider state.
Before first publication, the UI must explain that the content is readable by
anyone and that Mallard cannot revoke copies that other users downloaded.

### 3.3 Roles

The workspace has only two roles.

| Capability | Public reader | Member | Admin |
| --- | --- | --- | --- |
| Read effective-public content | Yes | Yes | Yes |
| Read private workspace content | No | Yes | Yes |
| Direct push to an unprotected Git branch | No | Yes | Yes |
| Open an MR for a protected branch | No | Yes | Yes |
| Direct push to a protected Git branch | No | No | Yes |
| Accept or reject an MR | No | No | Yes |
| Create or archive a repository | No | No | Yes |
| Change repository visibility | No | No | Yes |
| Configure protected branches | No | No | Yes |
| Invite or remove a Member | No | No | Yes |
| Promote or demote an Admin | No | No | Yes |
| Change workspace visibility | No | No | Yes |
| Delete the workspace | No | No | Yes |

All pushes use the Push review in section 5.5. Admin status does not bypass
capture review, warning acknowledgement, revision checks, or
compare-and-swap.

Role rules:

- A new workspace starts with one Admin.
- A workspace must always have at least one active Admin.
- Admin and Member are the only serialized role values.
- Owner and Viewer do not exist.
- An invitation always creates a Member.
- An Admin can promote an accepted Member in a separate action.
- An Admin cannot demote, remove, or leave if that action removes the last
  active Admin.
- Roles apply to all workspace repositories. Per-repository collaborator roles
  are not part of the first release.

### 3.4 Invitations

Public visibility grants read access only. It never grants membership.

An Admin invites a person by verified email. The invitation:

- grants the Member role;
- expires after seven days;
- contains a random, single-use token;
- is accepted only by an authenticated identity with the matching verified
  email;
- becomes active only after explicit acceptance; and
- can be revoked before acceptance.

D1 stores a keyed email lookup hash for matching. The delivery address can be
stored only until the invitation email is sent. The service then removes the
delivery address. No application-level field encryption is used.

The invitation token is sent in a URL fragment or transcription code. The
desktop sends it in a POST body. It must not appear in an API path, query
string, referrer, analytics field, or log.

### 3.5 Public demo

The official demo uses the same model as every other repository:

- The Mallard Demo workspace is public.
- The demo repository is public.
- The repository is marked as featured for catalog display.
- The featured record contains the reviewed public Git clone URL.
- The featured flag changes discovery only. It grants no access.
- Anonymous users can list and pull accepted demo content.
- An Admin invites editors as workspace Members.
- The primary branch is main and is protected.
- Develop is not protected by default.
- A Member can push to develop.
- A Member must open an MR to update main.
- Only an Admin can accept or reject the MR.

The Mallard Demo workspace must contain only content that can be public. It
must not share membership with an internal or private workspace.

There is no special demo bucket, guest token, or demo write path. This avoids
a second authorization model.

## 4. Project, branch, and review behavior

### 4.1 Git-only repository scope

V1 repository creation accepts only a canonical project root for which Git
confirms that the path is inside a working tree. The desktop records the
repository fingerprint and Git remote identity without storing a local
absolute path in cloud data.

Managed refs use validated Git branch names. The configured primary branch is
protected by default. The initial primary-branch value is main. All other
branches are not protected by default. Develop is therefore directly writable
by a Member unless an Admin protects it.

An Admin can add or remove protection for exact branch names.

The first release does not support branch-name patterns, wildcards,
role-specific exceptions, required reviewer counts, scheduled rules, or
external status checks.

If the project stops being Git-managed, managed Push and MR creation fail
closed. The user must restore a valid Git working tree or unlink the managed
repository.

### 4.2 Write decision

| Target | Member action | Admin action |
| --- | --- | --- |
| Branch is not protected | Direct Push | Direct Push |
| Branch is protected | Merge request | Direct Push or Merge request |

The authorization decision occurs again in the same D1 transaction that
changes a ref. A stale desktop policy view cannot authorize a push.

Every branch-policy change increments repository policy_revision. Direct-push
plans and MRs pin this value.

### 4.3 New-chat requirement

Each MR must use one new provider chat.

The desktop flow is:

1. The Member selects Create merge request.
2. Mallard reads the current protected target.
3. Mallard records the checked-out source branch, protected target branch,
   target commit ID, and initial merge-base commit ID. The source branch must
   differ from the protected target branch.
4. Mallard starts a new provider chat for the work.
5. Mallard submits the new cloud thread ID and pinned coordinates.
6. D1 verifies that the chat is unused, creates the MR, and creates its source
   ref from the exact target head in one transaction.
7. The user makes changes in that chat.
8. Mallard performs the Push review in section 5.5 against the MR source ref.
9. The Member opens the MR. Mallard freezes the current source commit ID and
   recalculates the merge-base commit ID.

The server enforces one MR per cloud thread ID. An existing chat cannot be
attached to an MR. A chat used by a rejected, withdrawn, or accepted MR cannot
be reused for another MR.

This rule keeps the UI and audit history simple. One MR has one chat, one
source ref, one target ref, and one decision.

### 4.4 Merge-request states

The state model is:

~~~mermaid
stateDiagram-v2
    [*] --> draft
    draft --> open: Member opens MR
    draft --> withdrawn: Author cancels
    open --> accepted: Admin accepts exact heads
    open --> rejected: Admin rejects
    open --> rejected: Target conflict
    open --> rejected: Git conflict reported
    open --> withdrawn: Author withdraws
    accepted --> [*]
    rejected --> [*]
    withdrawn --> [*]
~~~

Rules:

- A draft source ref can advance through reviewed pushes.
- Opening the MR freezes the exact source head.
- An open MR cannot receive more commits.
- Acceptance moves the target to the exact frozen source head.
- Rejection does not change the target.
- Withdrawal does not change the target.
- Accepted, rejected, and withdrawn MRs are terminal.
- A terminal MR cannot reopen.
- A retry requires a new MR and a new chat.
- MR draft content and review events are visible only to workspace
  Members and Admins in the first release.

### 4.5 Conflict policy

The first release uses strict fast-forward acceptance. It does not attempt a
three-way content merge.

An MR pins:

- target ref ID;
- target head bytes and ETag at MR creation;
- source ref ID;
- source head bytes and ETag when the MR opens;
- repository policy_revision;
- Git source branch, target branch, source commit ID, target commit ID, and
  merge-base commit ID;
- chat thread ID; and
- MR revision.

An Admin acceptance succeeds only when:

- the MR is open;
- the source head still matches the frozen source ETag;
- the target head still matches the base target ETag;
- the policy revision still permits the MR;
- the Admin is active; and
- all referenced immutable objects pass integrity checks.

If the target head changed, the service defines the result as a conflict. It
atomically changes the MR to rejected with reason target_changed. This
rule can reject changes that a more complex merge engine could combine. The
conservative result is intentional.

For a Git project, the Admin desktop also performs a non-mutating Git conflict
check between the source and target branches before it submits acceptance. If
Git reports a conflict, the desktop submits rejection with reason
git_conflict. Mallard does not show a conflict editor.

The check fetches the exact commits through the user's configured Git remote
and credentials. It runs in an isolated temporary Git work area and does not
change the user's checked-out branch or working tree. Git credentials do not
go to the managed cloud.

If the protected Git target commit changed after MR creation, the desktop
submits rejection with reason git_target_changed. If the desktop cannot fetch
and verify the pinned Git commits, acceptance stops without changing the MR.

The managed service does not host Git objects. Therefore, the server can
authoritatively enforce Mallard bundle-head conflicts, but it cannot
independently prove a source-code merge result. Git source protection remains
the responsibility of the linked Git host or remote.

### 4.6 Manual rejection

An Admin can reject any open MR.

The rejection records:

- the MR revision;
- the Admin user ID;
- a bounded reason code;
- an optional bounded comment;
- the decision time; and
- the MR source and target head coordinates.

The API returns the terminal MR. It does not delete immutable objects or
the source ref in the MR decision path. A retention job can remove unreachable
draft data later.

## 5. Portable bundle and desktop contract

This section defines the complete contract between the desktop, the managed
adapter, the cloud API, and stored bundle data.

### 5.1 Closed logical namespace

Every project bundle uses this logical layout:

~~~text
.mallard/
|-- _storage.json
`-- v1/repositories/<bundle-id>/
    |-- _head.json
    |-- _tag.json
    |-- _objects/<payload-sha256>
    |-- _manifests/<revision>-<commit-id>.json
    `-- _commits/<revision>-<commit-id>.json
~~~

The namespace is closed. A client can use only the listed roots and validated
descendants. The Worker rejects:

- an absolute key;
- an empty path segment;
- dot or dot-dot segments;
- a backslash;
- a control character;
- an overlength key;
- a bundle ID that does not match the authorized repository; and
- an unknown root or file name.

The bundle ID is a validated random identifier. It is portable. Workspace IDs,
managed repository IDs, ref IDs, storage IDs, local project IDs, device IDs,
and machine paths are not portable.

### 5.2 Document roles

All JSON documents use schema_version 1 and reject unknown or invalid
coordinates.

**Storage marker**

The storage marker has this exact value:

~~~json
{"format":"mallard-storage","layout_version":1}
~~~

It identifies the storage layout. It is not repository content. The managed
adapter synthesizes it. A matching immutable write returns AlreadyPresent.
Any other value is rejected.

**Repository tag**

The tag is bounded listing metadata. It contains:

- schema version;
- bundle ID;
- display name;
- bundle kind, which is project;
- current observed revision;
- update time;
- resource count; and
- file count.

The tag is not authoritative. A reader must validate the head and manifest.

**Head**

The head is the only authoritative mutable bundle pointer. It contains:

- schema version;
- bundle ID;
- bundle kind;
- monotonically increasing revision;
- commit ID;
- manifest logical key;
- exact manifest SHA-256;
- exact manifest byte length; and
- update time.

The head changes only with compare-and-swap against an absent value or an
exact prior ETag. A failed comparison never overwrites the winner.

**Manifest**

The manifest is one immutable, complete snapshot. It contains:

- schema version;
- repository revision;
- commit ID;
- update time;
- bundle identity;
- privacy-preserving publishing origin; and
- a map of independently revisioned scopes.

The bundle identity contains the bundle ID, display name, bundle kind
project, and a repository fingerprint. The fingerprint is a SHA-256 value.
It is not a path or credential. Managed Cloud V1 requires the fingerprint
because every repository is Git-managed.

Scope keys are:

- project; or
- provider:<provider-key>.

Each scope contains:

- its revision and canonical content SHA-256;
- a reviewed capture and apply recipe;
- live resource descriptors;
- regular-file entries;
- directory entries;
- typed conversation entries where supported; and
- typed tombstones.

A file entry contains a resource ID, SHA-256, byte length, source modification
time, immutable object key, and optional safe permission bits. A directory
entry contains a resource ID, modification time, and optional safe permission
bits. Modes cannot contain special permission bits.

The portable schema has a project scope. Managed Cloud V1 rejects ordinary
project-file entries, project-directory entries, and their tombstones in that
scope. Git source travels through Git. A provider scope can contain only that
provider's conversations and selected provider state. References cannot cross
scope boundaries.

**Commit record**

The immutable commit record contains:

- schema version;
- bundle ID;
- revision;
- commit ID;
- manifest key and SHA-256;
- creation time;
- optional previous commit ID;
- added, changed, and removed file counts; and
- privacy-preserving publishing origin.

The commit record is history. The head is authority.

**Payload object**

Each payload object is stored at _objects/<sha256>. The bytes must match the
key. Objects are immutable and scoped to one managed repository. Equal bytes
in different repositories do not grant cross-repository access.

### 5.3 Publication invariants

One publication performs these steps:

1. Validate the repository, ref, expected head, and policy revision.
2. Verify that every retained manifest entry still has its immutable object.
3. Detect a semantic no-op and write nothing when the snapshot did not change.
4. Write new payload objects with create-only semantics.
5. Serialize the immutable manifest once.
6. Record its exact byte length and SHA-256.
7. Write the manifest with create-only semantics.
8. Write the immutable commit record.
9. Compare-and-swap the ref head.
10. Update the non-authoritative tag on a best-effort basis.
11. Commit machine-local accepted bases only after the head succeeds.

Readers cannot see an incomplete publication because only a successful head
change makes a revision authoritative.

Revision, commit ID, manifest key, manifest hash, and bundle ID must agree
across the head, manifest, and commit record. Every mismatch is an integrity
failure.

### 5.4 Managed object-store adapter

The desktop uses one object-store contract for local folders, user-owned
S3/R2, and managed cloud. The required operations are:

| Operation | Contract |
| --- | --- |
| get_metadata | Read bounded metadata bytes and an ETag |
| stage_verified | Stream one object to a private temporary file and return it only after exact length and SHA-256 verification |
| stage_many_verified | Perform bounded concurrent verified reads while preserving request order |
| put_immutable | Create exact bytes or confirm identical existing bytes |
| put_many | Perform bounded concurrent immutable writes while preserving request order |
| compare_and_swap | Create when absent or replace only an exact prior ETag |
| list | Return bounded validated keys and an opaque cursor |
| delete | Delete one exact validated object for authorized lifecycle work; a missing object is success |

There is no API operation that accepts an arbitrary delete prefix.

A managed adapter instance is bound to:

- one service URL;
- one managed workspace ID;
- one managed repository ID; and
- one managed ref ID.

The adapter projects the selected ref into the portable single-head contract:

- get_metadata for _head.json reads the selected D1 ref;
- compare_and_swap for _head.json changes the selected D1 ref;
- get_metadata for _tag.json reads the bounded repository tag in D1;
- immutable reads and writes map to private R2 through the Worker;
- list returns only authorized logical keys; and
- delete is available only to explicit lifecycle operations.

Local-folder and user-owned S3/R2 providers have one head per bundle. They do
not gain cloud roles, visibility, refs, or MRs.

### 5.5 Capture and Push review

Push is always review-first.

The desktop performs these steps:

1. Resolve the canonical local project binding and selected provider homes.
2. Detect whether the project is Git-managed.
3. Read and verify the selected remote ref head and manifest.
4. Discover project-scoped conversations and allowed provider state.
5. Capture candidate metadata and bytes into private temporary files.
6. Compare each candidate with the accepted local base and remote manifest.
7. Classify each item as local-only, remote-only, ahead, synchronized, or
   conflict.
8. Show exact additions, changes, removals, warnings, and retained remote
   entries.
9. Let the user select the exact resources to publish.
10. Require acknowledgement for credential-shaped, executable, or public
    content warnings.
11. Create a bounded plan that pins configuration revision, remote head ETag,
    policy revision, source digests, selection, warning acknowledgements, and
    expiry.
12. Recheck every pin before publication.
13. Publish by the ordering in section 5.3.

Capture can include:

- selected project-scoped Codex and Claude conversations;
- selected provider memory and supported provider state;
- portable recipes and resource metadata; and
- Git commit, branch, and conversation-mapping metadata.

Capture cannot include:

- an access token, refresh token, OAuth code, cookie, private key, or provider
  credential store;
- the complete provider home;
- an absolute local path;
- a storage credential or endpoint secret;
- a local exclusion or trust decision;
- a backup, receipt, staging, or temporary file;
- a symlink, socket, device, FIFO, or other special file; or
- a project-scope ordinary file, directory, or project-content tombstone; or
- any source file from the Git working tree.

Source files always travel through Git. The Push review can include Git
metadata and mapped conversations, but it cannot upload source files from the
working tree.

### 5.6 Pull review and local apply

Pull is also review-first. The service cannot trigger a local apply.

The desktop performs these steps:

1. Read the exact ref head.
2. Stage and verify the manifest by its pinned length and SHA-256.
3. Validate every repository, revision, commit, scope, and object coordinate.
4. Compare remote entries with local content and accepted bases.
5. Build a bounded plan with exact target paths and actions.
6. Show creates, overwrites, preserved local items, conflicts, optional
   deletions, warnings, and manual actions.
7. Let the user approve the exact actions.
8. Pin configuration revision, head ETag, manifest digest, local target
   digests, action selection, warning acknowledgements, and expiry.
9. Stage only the approved payloads and verify each payload.
10. Recheck every pin immediately before apply.
11. Create local backups before replacement.
12. Apply with atomic file replacement where supported.
13. Write a bounded receipt only after the action result is known.

Deletion is opt-in:

- A file deletion requires the exact expected local digest.
- A directory deletion can target only the exact expected empty directory.
- Directory deletion is never recursive.
- A changed local target blocks deletion and becomes a manual action.

No Pull operation writes outside validated project or provider roots. Pull
rejects symlinks, special files, traversal, path aliases, and targets that
escape after canonical resolution.

### 5.7 Git boundary

Git-managed source travels through the configured Git remote. Managed cloud
stores Mallard bundle history, selected agent conversations, provider state,
and metadata.

A managed branch ref can use the same name as a Git branch. The matching name
supports history mapping. It does not make Mallard a Git remote.

A Mallard MR:

- controls one Mallard bundle target ref;
- records the Git source branch, target branch, and commit coordinates;
- does not push Git commits;
- does not create or merge a Git-host pull request;
- does not change the Git default branch; and
- does not replace Git-host branch protection.

The MR review must show this boundary.

### 5.8 Machine-local state

Machine-local state lives under ~/.mallard and is never portable bundle
content.

The first managed-cloud implementation performs one atomic, one-way migration
from sync_config.json schema 1 to schema 2. After success, it reads and writes
only schema 2. It does not retain a second execution path.

Storage variants are:

~~~text
local
  id, name, local directory, local exclusions

s3
  id, name, bucket, endpoint/account/region, credential references,
  conditional-write capability, local exclusions

managed
  id, name, service URL, workspace ID, local exclusions
~~~

A project-to-storage binding stores the selected managed repository ID and ref
ID, canonical local Git root, and expected repository fingerprint. It also
stores provider-home and accepted-base references. Those machine-local values
do not enter a portable manifest.

Bounded, atomic ~/.mallard/cloud_state.json schema 1 stores:

- local document revision;
- Mallard user ID;
- device ID;
- non-secret session-family reference;
- login and last-refresh times; and
- service environment identifier.

Access and refresh tokens live only in the operating-system keychain.

Normal JSON configuration contains no:

- R2 credential;
- service administration token;
- access or refresh token;
- provider credential;
- content key;
- account key;
- device private key;
- key epoch;
- key envelope; or
- recovery package.

## 6. Cloudflare architecture

### 6.1 Topology

~~~mermaid
flowchart LR
    READER["Public reader"]
    APP["Mallard desktop<br/>Tauri Rust managed adapter"]
    EDGE["Cloudflare custom domain<br/>TLS, web application firewall, rate limits"]
    API["Public API Worker<br/>auth, policy, object gateway"]
    IDP["GitHub and Google<br/>OAuth and OpenID Connect identity"]
    D1["D1 control plane<br/>users, visibility, roles, refs,<br/>MRs, quotas, jobs, audit"]
    R2I["Private immutable R2<br/>plaintext bundle objects"]
    R2S["Private staging R2<br/>multipart uploads"]
    CACHE["Workers Cache<br/>effective-public immutable reads"]
    Q["Queue and dead-letter queue<br/>cleanup and reconciliation"]
    CRON["Cron Worker<br/>due-job scanner"]
    EMAIL["Email delivery<br/>invitations and notices"]
    ADMIN["Access-protected internal API"]
    OBS["Workers Logs and Logpush<br/>sanitized operations data"]

    READER -->|"HTTPS read"| EDGE
    APP -->|"HTTPS read and write"| EDGE
    EDGE --> API
    API <-->|"OAuth or OIDC"| IDP
    API --> D1
    API --> R2I
    API --> R2S
    API --> CACHE
    API --> Q
    API --> EMAIL
    CRON --> D1
    CRON --> Q
    Q --> D1
    Q --> R2I
    Q --> R2S
    ADMIN --> API
    API --> OBS
~~~

### 6.2 Cloudflare resources

The first production deployment uses:

- one public API Worker on a Mallard-controlled custom domain;
- one D1 database for the control plane;
- one private R2 bucket for immutable repository objects;
- one private R2 bucket for temporary multipart staging;
- one Queue and one dead-letter queue;
- one Cron trigger for due jobs and stale-state reconciliation;
- Worker Secrets or Secrets Store bindings for OAuth secrets, token-signing
  secrets, keyed email lookup, and internal service credentials;
- one invitation and security-notice email delivery integration;
- web application firewall and rate-limit rules;
- one internal hostname protected by Cloudflare Access; and
- Workers Logs with optional Logpush.

The R2 buckets do not use a public R2 custom domain. Every read goes through
the Worker.

The first release does not use a Durable Object. D1 holds each ref head and
performs the atomic branch-policy, compare-and-swap, and MR-decision
transaction.

### 6.3 Network boundary

The Worker derives every D1 key and physical R2 prefix. The client never
supplies a bucket name, tenant prefix, or physical object key.

The Worker must:

- authenticate every private read and every write;
- evaluate effective visibility before every anonymous read;
- evaluate current membership and branch policy before every write;
- validate IDs, branch names, logical keys, sizes, and digests;
- enforce bounded pagination and request sizes;
- use create-only writes for immutable objects;
- use D1 compare-and-swap for refs;
- omit a general object-delete route; and
- return safe external errors.

## 7. Privacy, identity, and trust

### 7.1 Plaintext content decision

Mallard does not encrypt managed-cloud content before upload.

The following values are plaintext to the managed service:

- repository tags;
- ref heads;
- manifests;
- commit records;
- conversation transcripts;
- provider memory selected by the user;
- names and descriptions; and
- MR metadata and comments.

The consequences are explicit:

- Mallard service operators can technically access private content.
- A Cloudflare compromise can expose private content.
- Legal and support processes can apply to stored plaintext.
- Server-side integrity validation and abuse scanning are possible.
- Client key enrollment, recovery, rotation, and sharing are not required.

The public-release UI and privacy policy must describe this trust model.

No content-encryption milestone is hidden in the delivery plan. Adding
client-side encryption later would require a separate product and schema
decision.

### 7.2 Content safety

The capture rules in section 5.5 are mandatory. The service also enforces:

- Do not capture credentials or provider authentication stores.
- Do not put local absolute paths in portable documents.
- Reject symlinks and special files.
- Preserve bounded, no-follow traversal.
- Keep machine-local exclusions and trust decisions local.
- Preserve warnings for credential-shaped and executable content.
- Require explicit acknowledgement before publishing warned content.
- Require an additional public-disclosure acknowledgement for a public
  repository.

Secret detection remains best-effort. It is not proof that public content is
safe.

### 7.3 OAuth and application sessions

GitHub and Google are the initial identity providers. Email is not an identity
key.

The stable identity key is:

~~~text
(issuer, provider_subject)
~~~

The desktop uses the system browser and Authorization Code with Proof Key for
Code Exchange (PKCE):

1. The Rust backend binds a random loopback port on 127.0.0.1.
2. It creates app state and an S256 PKCE verifier and challenge.
3. The Worker redirects to the identity provider.
4. The provider returns to one fixed Worker callback.
5. The Worker verifies state and nonce where applicable.
6. The Worker exchanges the provider code with its stored client secret.
7. The Worker redirects one single-use Mallard code to the exact loopback
   callback.
8. The Rust backend exchanges the code and PKCE verifier for Mallard tokens.

Only literal loopback IP callbacks are accepted. Localhost names, arbitrary
hosts, URL credentials, fragments, and unapproved paths are rejected.

Session rules:

- OAuth transactions and Mallard application codes expire after five minutes
  and are single-use.
- Access tokens are audience-restricted JSON Web Tokens with a 10-minute
  lifetime.
- Refresh tokens are random opaque values with a maximum 30-day lifetime.
- The desktop stores refresh tokens in the operating-system keychain.
- D1 stores only a keyed hash of each refresh token.
- Every successful refresh rotates the token.
- Reuse of an old token revokes the session family.
- Logout and device revocation are idempotent.
- The service discards provider tokens after identity lookup.

The first login creates one private personal workspace and makes the user its
Admin.

### 7.4 Authorization

D1 is the source of truth for:

- user and session state;
- workspace and repository state;
- visibility;
- active membership;
- role;
- ref state;
- branch protection;
- policy revision;
- MR state; and
- archive state.

The Worker does not trust a role, visibility value, or branch rule supplied by
the client.

An unauthorized private identifier returns 404. An authenticated user who can
see the resource but lacks a write capability receives 403.

### 7.5 Logging and support access

Logs and audit events must not contain:

- access or refresh tokens;
- OAuth codes;
- invitation tokens;
- verified email addresses;
- raw conversation text;
- file bodies;
- provider memory;
- local paths;
- MR comments;
- D1 SQL; or
- physical R2 keys.

Internal support routes use Cloudflare Access and separate authorization.
There is no support route that returns arbitrary repository content. Any
production content-access procedure must be exceptional, logged, and defined
outside the normal API.

## 8. Storage and consistency model

### 8.1 Logical and physical storage

Clients use the schema-1 logical keys defined in section 5.1. The Worker maps
them to service-owned records and keys.

Conceptual physical immutable keys are:

~~~text
managed/v1/repositories/<repository-id>/_objects/<sha256>
managed/v1/repositories/<repository-id>/_manifests/<revision>-<commit-id>.json
managed/v1/repositories/<repository-id>/_commits/<revision>-<commit-id>.json
~~~

D1 stores:

- the bounded repository tag bytes and digest;
- each ref name, head bytes, head digest, ETag, and version;
- visibility and policy state;
- object index and quota state; and
- MR state.

The managed ETag is the lowercase SHA-256 of the exact stored head bytes. The
D1 ref version increments on each successful change, even when another ref
already points to the same head bytes.

The service generates repository IDs and ref IDs. A client cannot select a
physical prefix.

### 8.2 Immutable publication

A push performs:

1. Read the current ref and policy revision.
2. Perform local capture and Push review.
3. Create content-addressed payload objects in R2.
4. Create the immutable manifest in R2.
5. Create the immutable commit record in R2.
6. Verify the exact length and SHA-256 of every new object.
7. Change the D1 ref head with compare-and-swap.
8. Record the audit event.

If step 7 fails, the immutable objects can remain unreachable. A delayed
orphan job can remove them after retention allows it. Readers cannot see the
new revision because the ref did not change.

An immutable key can be created only when absent. If the key exists, the
service verifies that the stored length and digest match. Different bytes at
the same immutable key are an integrity error.

### 8.3 Direct ref changes

A direct push D1 transaction verifies:

- active user and session;
- active workspace and repository;
- current membership and role;
- repository fingerprint and target Git ref;
- current branch rule;
- policy revision;
- expected head ETag;
- new head document bounds and schema coordinates; and
- referenced object readiness.

The transaction changes the ref only when all checks pass.

### 8.4 Merge-request acceptance

One D1 transaction decides an MR.

If all pinned values match, the transaction:

1. copies the frozen source head coordinates to the target ref;
2. increments the target ref version;
3. sets the MR state to accepted;
4. records the decision event; and
5. records the audit event.

If the target head does not match, the same transaction:

1. leaves the target unchanged;
2. sets the MR state to rejected;
3. sets reason target_changed;
4. records the conflict event; and
5. records the audit event.

There is no state in which D1 says accepted but the target ref did not change.

### 8.5 Public reads and caching

Public immutable objects can use Workers Cache only after the Worker confirms
effective public visibility in D1.

The cache key includes:

- repository ID;
- visibility generation;
- immutable logical key; and
- immutable digest.

Ref heads, tags, membership data, invitations, MR drafts, errors, and
authenticated private responses use no-store.

The Worker performs the visibility check before it reads Workers Cache. A
direct public R2 hostname is prohibited.

### 8.6 Upload state

Small uploads use one request. Large uploads use a bounded multipart state
machine.

The upload record pins:

- repository ID;
- logical immutable key;
- exact expected size and SHA-256;
- creator;
- quota reservation;
- part bounds;
- expiry; and
- idempotency key.

Completing an upload verifies the recorded parts and resulting object before
the object becomes ready. Expiry releases quota. Cleanup of abandoned staging
objects is idempotent.

## 9. D1 control-plane model

### 9.1 Conventions

- IDs use 128 random bits and lowercase hexadecimal encoding.
- Times use UTC Unix epoch milliseconds.
- Digests and keyed hashes use 32-byte values.
- Public enums use database checks.
- Queries are prepared and bounded.
- Foreign keys are enabled.
- Authorization mutations use the D1 primary.
- Migrations are forward-only.

This proposal has not been deployed. The first implementation starts from one
clean initial schema. Do not add migrations or compatibility fields for the
removed encryption design.

### 9.2 Main tables

| Table | Purpose | Critical fields |
| --- | --- | --- |
| users | Account lifecycle | ID, state, display name, timestamps |
| identities | OAuth identity links | issuer, provider subject, user ID, verified-email lookup hash |
| devices | Named desktop sessions | user ID, name, state, last seen |
| refresh_sessions | Rotating session families | device ID, token hash, expiry, replay state |
| workspaces | Collaboration boundary | slug, name, visibility, visibility generation, state |
| workspace_members | Workspace roles | user ID, workspace ID, Admin or Member, state |
| invitations | Invitation lifecycle | workspace ID, email lookup hash, token hash, expiry, state |
| repositories | Git bundle catalog | workspace ID, bundle ID, repository fingerprint, primary branch, visibility, policy revision, tag |
| repository_refs | Managed heads | repository ID, ref name, head bytes, head digest, ETag, version, frozen state |
| protected_branches | Exact Git rules | repository ID, branch name, created by, created time |
| merge_requests | MR state | source ref, target ref, Git coordinates, pinned heads, chat ID, state, reason |
| merge_events | MR audit history | MR ID, actor, event kind, bounded comment, time |
| repository_objects | Immutable object index | logical key, physical key, length, digest, state |
| upload_sessions | Single and multipart state | object coordinates, parts, expiry, quota reservation |
| idempotency_records | Mutation replay protection | user, route scope, key hash, request hash, result code |
| quota_ledgers | Reserved and committed use | workspace, class, bytes and object counts |
| deletion_jobs | Delayed archive purge | target type, target ID, not-before time, attempts |
| audit_events | Sanitized security and policy events | actor, action, target IDs, result, request ID, time |
| featured_repositories | Public catalog selection | repository ID, reviewed public Git clone URL, rank, state |

### 9.3 Enums and state transitions

The initial schema uses these public enum values:

| Field | Values |
| --- | --- |
| visibility | public, private |
| workspace role | admin, member |
| user state | active, disabled, deleting |
| workspace state | active, archived, deleting |
| membership state | active, suspended |
| invitation state | pending, accepted, revoked, expired |
| repository state | active, archived, deleting |
| ref state | active, draft, frozen, archived |
| MR state | draft, open, accepted, rejected, withdrawn |
| object state | staging, ready, orphaned, deleting |
| upload state | allocated, uploading, completed, aborted, expired |
| deletion-job state | pending, running, retry, succeeded |

State changes use explicit allowlists:

- active workspace or repository can become archived;
- archived workspace or repository can return to active during retention;
- archived workspace or repository can become deleting after retention;
- deleting is terminal until the row is removed;
- active membership can become suspended and can return to active;
- pending invitation can become accepted, revoked, or expired;
- draft MR can become open or withdrawn;
- open MR can become accepted, rejected, or withdrawn;
- accepted, rejected, and withdrawn MRs are terminal;
- draft ref can advance until its MR opens;
- opening an MR changes its source ref from draft to frozen; and
- a frozen ref cannot advance.

MR rejection reason codes are:

- admin_rejected;
- target_changed;
- git_target_changed;
- git_conflict;
- policy_changed;
- source_changed; and
- integrity_failed.

The service rejects an enum or state transition that is not listed.

### 9.4 Required constraints

The schema and service must enforce:

- role is Admin or Member;
- visibility is public or private;
- a private workspace has no public repository;
- one active Admin remains in every active workspace;
- one user has at most one active membership per workspace;
- invitation tokens and refresh tokens are unique by hash;
- repository bundle identity is unique within the managed service;
- every repository has a valid Git repository fingerprint;
- Git ref names are unique per repository;
- main is protected when a Git repository uses main as its primary branch;
- source and target refs belong to the same repository;
- an MR uses one unique cloud thread ID;
- an open MR has frozen source and target coordinates;
- a terminal MR cannot change state;
- only an Admin can decide an MR;
- direct Member push cannot change a controlled target;
- a managed manifest has no ordinary project-file content;
- object readiness precedes any ref that references the object; and
- archive state blocks new writes.

### 9.5 Service invariants

Some checks require request context:

- An invitation always activates as Member.
- Promotion to Admin is a separate authenticated operation.
- Last-Admin checks and role changes occur in one transaction.
- Public-to-private workspace changes also privatize repositories in one
  transaction.
- Branch-rule changes and policy_revision changes occur in one transaction.
- Direct push authorization and head CAS occur in one transaction.
- MR acceptance or conflict rejection occurs in one transaction.
- Mutations pin current resource revision and use an idempotency key.
- Generic idempotency records do not store tokens, email values, comments, or
  response bodies with private content.

## 10. Public API

### 10.1 Common contract

All application routes are under /v1. JSON documents use application/json.
Raw payload objects use application/octet-stream.

Every response contains X-Request-Id.

Authenticated requests use:

~~~text
Authorization: Bearer <access-token>
~~~

State-changing requests also use:

~~~text
Idempotency-Key: <16-256 printable ASCII characters>
~~~

Ref preconditions use If-Match and If-None-Match.

Errors have one bounded shape:

~~~json
{
  "error": {
    "code": "target_changed",
    "message": "The target changed. This merge request was rejected.",
    "request_id": "01...",
    "retryable": false
  }
}
~~~

Errors do not reveal another tenant, internal SQL, bucket names, physical R2
keys, stack traces, or secrets.

### 10.2 Authentication routes

| Method | Route | Purpose |
| --- | --- | --- |
| POST | /v1/auth/transactions | Create provider state, PKCE binding, and exact loopback callback |
| GET | /v1/auth/{provider}/authorize | Start the system-browser login |
| GET | /v1/auth/{provider}/callback | Receive the fixed GitHub or Google callback |
| POST | /v1/auth/token | Consume the one-time app code |
| POST | /v1/auth/refresh | Rotate a refresh token |
| POST | /v1/auth/logout | Revoke the current session family |
| GET | /v1/me | Read account and active workspace summaries |
| PATCH | /v1/me | Update bounded profile metadata |
| GET | /v1/devices | List devices and sessions |
| POST | /v1/devices/{device_id}/revoke | Revoke one device and its sessions |

There are no account-key, workspace-key, key-epoch, key-envelope, or recovery
routes.

### 10.3 Public discovery and read routes

| Method | Route | Purpose |
| --- | --- | --- |
| GET | /v1/public/workspaces | List bounded public workspace summaries |
| GET | /v1/public/repositories | List bounded public summaries; featured rows include the reviewed Git clone URL |
| GET | /v1/workspaces/{workspace_slug} | Read a public summary or an authorized private workspace |
| GET | /v1/repositories/{repository_id} | Read public or authorized repository metadata |
| GET | /v1/repositories/{repository_id}/refs | List public accepted refs or authorized refs |
| GET | /v1/repositories/{repository_id}/refs/{ref_name}/head | Read one visible ref head |
| GET | /v1/repositories/{repository_id}/objects/{logical_key_b64} | Read one authorized logical object |

Anonymous routes expose accepted repository state only. They do not expose
members, invitations, MR source refs, MR comments, drafts, rejected content,
or audit events.

### 10.4 Workspace and membership routes

| Method | Route | Purpose |
| --- | --- | --- |
| POST | /v1/workspaces | Create a private workspace and Admin membership |
| PATCH | /v1/workspaces/{workspace_id} | Change name or visibility with revision checks |
| GET | /v1/workspaces/{workspace_id}/members | List active and invited membership |
| POST | /v1/workspaces/{workspace_id}/invitations | Invite one Member |
| POST | /v1/invitations/accept | Accept a body-carried invitation token |
| POST | /v1/workspaces/{workspace_id}/invitations/{invitation_id}/revoke | Revoke an invitation |
| PATCH | /v1/workspaces/{workspace_id}/members/{user_id} | Promote, demote, suspend, or reactivate |
| DELETE | /v1/workspaces/{workspace_id}/members/{user_id} | Remove membership under last-Admin policy |
| POST | /v1/workspaces/{workspace_id}/archive | Start delayed workspace deletion |
| POST | /v1/workspaces/{workspace_id}/restore | Restore during retention |

### 10.5 Repository, ref, and policy routes

| Method | Route | Purpose |
| --- | --- | --- |
| GET | /v1/workspaces/{workspace_id}/repositories | List authorized repositories |
| POST | /v1/workspaces/{workspace_id}/repositories | Create a Git managed repository from a bundle ID, fingerprint, and primary branch |
| PATCH | /v1/repositories/{repository_id} | Change bounded metadata or visibility |
| POST | /v1/repositories/{repository_id}/refs | Create a Git branch ref |
| GET | /v1/repositories/{repository_id}/branch-rules | Read exact protected branches and policy revision |
| PUT | /v1/repositories/{repository_id}/branch-rules/{branch_name_b64} | Protect an exact Git branch |
| DELETE | /v1/repositories/{repository_id}/branch-rules/{branch_name_b64} | Remove protection from an exact Git branch |
| PUT | /v1/repositories/{repository_id}/refs/{ref_name_b64}/head | Direct compare-and-swap push |
| POST | /v1/repositories/{repository_id}/archive | Start delayed repository deletion |
| POST | /v1/repositories/{repository_id}/restore | Restore during retention |

The direct head route checks the current role and branch rule. It does not
trust a client statement that a branch is writable.

### 10.6 Merge-request routes

| Method | Route | Purpose |
| --- | --- | --- |
| POST | /v1/repositories/{repository_id}/merge-requests | Create an MR draft and MR-scoped source ref |
| GET | /v1/repositories/{repository_id}/merge-requests | List authorized MRs |
| GET | /v1/merge-requests/{merge_request_id} | Read one MR and its events |
| PUT | /v1/merge-requests/{merge_request_id}/source/head | Push to the MR draft source ref |
| POST | /v1/merge-requests/{merge_request_id}/open | Freeze the source head and open the MR |
| POST | /v1/merge-requests/{merge_request_id}/accept | Accept or atomically reject a target conflict |
| POST | /v1/merge-requests/{merge_request_id}/reject | Reject an open MR |
| POST | /v1/merge-requests/{merge_request_id}/withdraw | Withdraw a draft or open MR |

The accept route does not return a conflict-resolution plan. A target conflict
changes the MR to rejected and returns that terminal state.

### 10.7 Immutable object routes

| Method | Route | Purpose |
| --- | --- | --- |
| GET | /v1/repositories/{repository_id}/objects | Cursor-list authorized logical keys |
| POST | /v1/repositories/{repository_id}/immutable-uploads | Allocate or resume an immutable upload |
| PUT | /v1/uploads/{upload_id}/body | Complete a single-request upload |
| PUT | /v1/uploads/{upload_id}/parts/{part_number} | Upload one multipart part |
| POST | /v1/uploads/{upload_id}/complete | Complete the exact recorded part set |
| POST | /v1/uploads/{upload_id}/abort | Abort staging and release quota |

The API has no general object-delete route.

### 10.8 Internal routes

Internal routes use /internal/v1 on an Access-protected hostname. They support:

- user or workspace disablement;
- featured repository selection;
- quota overrides;
- deletion-job retry;
- dead-letter inspection and replay;
- sanitized audit lookup by opaque ID; and
- service health and migration status.

The Worker validates the Cloudflare Access token signature, issuer, audience,
and expiry.

### 10.9 Core mutation inputs

Every mutation uses the Idempotency-Key header from section 10.1. Its body
includes the expected revision of each mutable D1 resource. A response returns
the new revision and current state.

**Create workspace**

Required fields:

- bounded name and slug; and
- initial visibility, which defaults to private.

The caller becomes the first Admin in the creation transaction.

**Create repository**

Required fields:

- workspace revision;
- bundle ID;
- bounded display name;
- repository fingerprint;
- primary Git branch;
- initial visibility, which defaults to private;
- bounded schema-1 tag bytes and SHA-256.

The service validates the fingerprint shape, branch name, bundle identity,
tag, workspace policy, and uniqueness. The desktop is responsible for
deriving the fingerprint from Git as defined in section 3.1.

**Create branch ref**

Required fields:

- repository policy revision;
- new branch name;
- optional base ref ID; and
- exact base head ETag when a base ref is present.

The new ref starts at the verified base head or at no head.

**Direct head update**

Required values:

- repository policy revision;
- target ref ID;
- If-Match or If-None-Match;
- new schema-1 head bytes.

The service parses the head and verifies all referenced immutable coordinates
before it updates D1.

**Create MR**

Required fields:

- repository policy revision;
- target ref ID and exact target head ETag;
- new cloud thread ID and provider;
- Git source and target branch names;
- pinned target commit ID;
- initial merge-base commit ID.

D1 verifies the role, protected target, unused cloud thread ID, branch names,
policy revision, and target ETag. It then creates the MR and its draft source
ref from the target in one transaction.

**Open MR**

Required fields:

- expected MR revision;
- exact source head ETag;
- frozen Git source commit ID;
- pinned Git target commit ID;
- recalculated merge-base commit ID.

Opening the MR freezes the source ref.

**Accept MR**

Required fields:

- expected MR revision;
- exact source and target head ETags;
- Admin Git-check source, target, and merge-base commit IDs;
- Git-check result no_conflict;
- Git-check time.

The service treats the authenticated Admin as the authority for the Git check.
It independently verifies every D1 and object coordinate. A changed target
causes terminal rejection instead of acceptance.

**Reject MR**

Required fields:

- expected MR revision;
- reason code;
- optional bounded comment;
- Git-check coordinates when the reason is git_conflict or
  git_target_changed.

Only Admins can use admin_rejected, git_conflict, or git_target_changed.

**Publish visibility**

Required fields:

- expected workspace and repository revisions;
- accepted head ETag and manifest SHA-256;
- warning-set digest;
- warning acknowledgements;
- public-disclosure acknowledgement.

### 10.10 Status codes

| Status | Meaning |
| --- | --- |
| 400 | Malformed request, callback, branch name, cursor, or logical key |
| 401 | Missing, invalid, or expired session |
| 403 | Visible resource, but the caller lacks the required capability |
| 404 | Resource absent or not visible |
| 409 | Target conflict, invalid workflow state, policy change, or idempotency mismatch |
| 412 | Direct ref compare-and-swap failed |
| 413 | Request or object exceeds a hard bound |
| 422 | Valid request violates quota, membership, or lifecycle policy |
| 429 | Rate limit |
| 503 | Retryable D1, R2, Queue, email, or identity-provider failure |

## 11. Critical flows

### 11.1 First login

1. Complete OAuth with PKCE.
2. Create the user, identity, device, private personal workspace, Admin
   membership, and default limits in one D1 transaction.
3. Issue the Mallard access and refresh tokens.
4. Store the refresh token in the operating-system keychain.
5. Show the empty private workspace.

No content key, recovery key, or device approval is required.

### 11.2 Pull the public demo

1. The app lists featured public repositories without a permanent credential.
2. The catalog returns the reviewed public Git clone URL and repository
   fingerprint.
3. The user clones or opens that Git repository.
4. The desktop verifies the local repository fingerprint.
5. The user selects the demo's accepted ref.
6. The Worker verifies effective public visibility.
7. The Worker returns the D1 ref head with no-store.
8. The app reads immutable manifest and payload objects through the Worker.
9. The desktop bundle engine verifies the contract in section 5.
10. The app builds the Pull plan defined in section 5.6.
11. The user approves the exact resources to apply.

Public visibility removes account setup. It does not remove Pull review.

### 11.3 Invite an editor to a public workspace

1. An Admin enters the editor's verified email.
2. The Worker creates a seven-day Member invitation and sends the token.
3. The recipient signs in and accepts.
4. D1 creates an active Member row.
5. The new Member can read private workspace repositories and write only
   under the branch policy read from D1.

The public could already read public content. Invitation adds write capability
and private workspace read capability.

### 11.4 Member pushes to develop

1. The desktop binds the managed ref named develop.
2. It reads the head and policy revision.
3. It performs the Push review in section 5.5.
4. It uploads and verifies immutable objects.
5. It submits a direct head compare-and-swap.
6. D1 confirms that develop is not protected and the caller is a Member.
7. D1 advances the ref or returns a compare-and-swap conflict.

If an Admin protected develop after step 2, step 6 rejects the direct push.

### 11.5 Member proposes a change to main

1. The Member selects Create merge request for main.
2. The desktop confirms that main is protected and records the Git
   coordinates.
3. The desktop starts a new provider chat.
4. D1 verifies the unused cloud thread ID and creates the MR and source ref
   from the exact main head.
5. The Member works in that chat.
6. The desktop performs the Push review in section 5.5 and advances the draft
   source ref.
7. The Member opens the MR. The source head and Git coordinates become frozen.
8. An Admin reviews the resource diff and conversation.
9. The Admin desktop performs its non-mutating Git conflict check.
10. The Admin accepts or rejects.
11. On acceptance, D1 either advances main and accepts the MR or detects that
    main changed and rejects the MR.

### 11.6 Admin rejects an MR

1. The Admin opens an exact MR revision.
2. The Admin selects Reject.
3. The Admin can add a bounded comment.
4. D1 verifies that the MR is open and the caller is an active Admin.
5. D1 records the terminal rejection and audit event.
6. The target remains unchanged.
7. The Member must start a new MR and a new chat to try again.

### 11.7 Conflict rejection

1. An open MR pins target head H1.
2. Another authorized operation advances the target to H2.
3. An Admin tries to accept the MR.
4. The D1 transaction compares H1 with H2.
5. The transaction leaves H2 unchanged.
6. The transaction marks the MR rejected with target_changed.
7. The UI shows a terminal rejection and offers Start new MR.

There is no rebase or resolve-conflicts action.

### 11.8 Change visibility to public

1. An Admin opens repository visibility settings.
2. The desktop refreshes and reviews the current accepted head.
3. Mallard runs its best-effort secret and executable checks.
4. The UI explains permanent public disclosure.
5. The Admin acknowledges all warnings.
6. The API verifies that the workspace is public.
7. D1 changes repository visibility and increments visibility generation.
8. Anonymous reads can now reach accepted refs.

Draft refs and MR content remain membership-only.

## 12. Failure handling

### 12.1 D1 and R2 boundary

Immutable publication uses this order:

~~~text
reserve quota in D1
-> write and verify immutable R2 object
-> mark object ready in D1
-> update D1 ref
~~~

The system handles partial failure as follows:

- D1 allocation succeeds and R2 write fails: retry or expire the upload.
- R2 write succeeds and D1 finalize fails: retry after verifying the exact R2
  object.
- Object becomes ready but ref update fails: retain an unreachable immutable
  object for delayed cleanup.
- D1 ref update succeeds: every referenced object was already ready.

### 12.2 Merge-request decision

MR acceptance, target change, MR state, and decision event share one D1
transaction.

An ambiguous client response is resolved by reading the MR and target
again. The client never retries acceptance with a new expectation under the
same idempotency key.

### 12.3 Queue and Cron

Queue delivery is acceleration, not the only clock.

Cron scans bounded due-job indexes for:

- expired invitations;
- expired sessions;
- stale uploads;
- abandoned multipart state;
- archived resources ready for purge;
- rejected or withdrawn draft refs past retention; and
- orphaned immutable objects.

Consumers are idempotent. Repeated failure moves a job to the dead-letter
queue and creates an alert.

### 12.4 Idempotency

Every mutation validates an idempotency key before expensive work.

Reusing a key with the same request returns the stored status. Reusing it with
a different request returns 409. Stored idempotency results are bounded and
do not include private content or credentials.

## 13. Lifecycle and operations

### 13.1 Archive and deletion

Archive hides the resource immediately and blocks writes. Physical deletion
starts after a 30-day recovery period.

Deletion rules:

- There is no public immediate-delete object API.
- Workspace deletion requires reauthentication.
- A user cannot leave a workspace as its last Admin.
- Repository purge deletes indexed objects only.
- Purge never accepts an arbitrary R2 prefix from a client.
- R2 retention locks can delay final object removal.
- Restore during retention is idempotent.
- Final purge is retryable and audited.

### 13.2 Visibility rollback

Changing public content to private:

1. increments visibility generation;
2. blocks new anonymous reads;
3. invalidates future public cache lookup;
4. schedules cache purge; and
5. records an audit event.

This action cannot revoke copies that were already downloaded.

### 13.3 Backup and recovery

Production operations must define:

- D1 point-in-time recovery and export tests;
- R2 inventory and object integrity checks;
- infrastructure configuration backups;
- secret rotation procedures;
- a quarterly restore exercise;
- recovery point and recovery time objectives; and
- a reconciliation command that does not expose content in logs.

### 13.4 Observability

Metrics include:

- request count, latency, and status;
- anonymous and authenticated rate-limit events;
- OAuth and refresh failures;
- invitation delivery and acceptance;
- authorization denials by capability;
- direct-push compare-and-swap conflicts;
- MR opens, accepts, manual rejections, and conflict rejections;
- D1 transaction latency;
- R2 upload and verification failures;
- queue lag and dead-letter depth;
- cache hit rate for public immutable objects; and
- archive and purge backlog.

Metrics use opaque IDs and bounded labels. They do not contain content.

### 13.5 Capacity

The first production deployment uses one D1 database. Rollout pauses before a
measured limit becomes an incident.

Before launch, load tests must measure:

- D1 write latency under ref and MR contention;
- public catalog and object-read volume;
- R2 upload throughput;
- queue backlog recovery;
- maximum bounded manifest processing;
- hot public demo cache behavior; and
- one-workspace branch-policy contention.

Regional sharding is a later design. It is not part of the first release.

## 14. Desktop experience

### 14.1 Workspace and repository settings

The desktop shows:

- workspace visibility;
- repository visibility;
- current role;
- active and invited Members;
- Git repository fingerprint;
- primary branch;
- exact protected branches;
- selected managed ref; and
- policy revision freshness.

Only Admins see management controls.

### 14.2 Push actions

For a Member:

- an unprotected Git branch shows Push;
- a protected Git branch shows Create merge request.

For an Admin:

- every target can show Push;
- a protected target can also show Create merge request when an Admin wants a
  recorded review path.

The UI checks current policy before capture and the server checks it again at
publication.

### 14.3 Merge-request UI

The MR UI has:

- target branch;
- MR state;
- one bound chat;
- source and target revision summaries;
- exact resource diff;
- public-disclosure warnings where applicable;
- Accept and Reject for Admins;
- Withdraw for the author; and
- Start new MR after a terminal rejection.

The UI has no:

- attach-existing-chat control;
- rebase control;
- auto-merge control;
- conflict editor;
- reopen control; or
- change-target control after creation.

### 14.4 Terminology

Use:

- Merge request for a Git protected branch;
- Protected branch for an Admin-controlled Git branch;
- Admin and Member for roles; and
- Public and Private for visibility.

Do not show Owner, Viewer, key, epoch, envelope, encrypted, decrypt, or recovery
terms in the managed-cloud UI.

## 15. Delivery plan

Each program must leave a working end-to-end product. Do not build all control
plane tables before one user flow works.

### Program A: Public demo read

Deliver:

- Worker custom domain;
- D1 public workspace and repository records;
- private immutable R2;
- one featured public demo repository;
- anonymous catalog and object reads;
- managed desktop read adapter; and
- Pull review and apply from section 5.6.

Exit condition: A new installation can discover and pull the demo without a
Mallard account.

### Program B: Private personal sync

Deliver:

- GitHub and Google OAuth with PKCE;
- rotating application sessions;
- private personal workspace;
- one Admin role;
- private managed repository;
- immutable upload;
- D1 ref compare-and-swap;
- direct Admin Push; and
- archive and restore.

Exit condition: One user can push and pull a private project on two devices.

### Program C: Public and private workspaces

Deliver:

- workspace and repository visibility;
- public cache policy;
- Member invitations;
- Admin promotion and last-Admin rules;
- public-disclosure review;
- visibility rollback; and
- public demo editor invitation.

Exit condition: An invited Member can edit the demo workspace while anonymous
users remain read-only.

### Program D: Git refs and protected branches

Deliver:

- managed refs;
- Git-only repository validation;
- required repository fingerprint;
- primary branch;
- main protected by default;
- exact Admin branch rules;
- direct Member Push to unprotected branches;
- policy revisions; and
- stale-policy rejection.

Exit condition: A Member can push develop and cannot push main directly.

### Program E: Merge requests

Deliver:

- MR-scoped source refs;
- mandatory new-chat binding;
- merge requests;
- frozen source heads;
- Admin Accept and Reject;
- atomic target-conflict rejection;
- Git conflict check in the Admin desktop; and
- terminal MR history.

Exit condition: A Member can propose a main change, an Admin can accept or
reject it, and a changed target causes automatic rejection.

### Program F: Production operations

Deliver:

- quota enforcement;
- WAF and rate limits;
- Queue, dead-letter queue, and Cron cleanup;
- audit and operational metrics;
- backup and restore exercises;
- deletion jobs;
- support runbooks;
- load tests; and
- staged rollout.

Exit condition: All production acceptance criteria and release gates pass.

## 16. Verification strategy

### 16.1 Access matrix tests

Test every combination of:

- anonymous, inactive invitee, Member, and Admin;
- public and private workspace;
- public and private repository;
- active and archived state; and
- direct read, list, upload, ref update, MR decision, and administration.

Verify that a hidden resource returns 404 and a visible forbidden action
returns 403.

### 16.2 Role tests

Verify:

- invitations activate as Member;
- Member cannot invite, promote, configure, archive, or change visibility;
- Admin can perform administration;
- Admin can promote an accepted Member;
- Owner and Viewer values are rejected;
- the last Admin cannot leave, demote, or be removed; and
- concurrent last-Admin operations have one valid result.

### 16.3 Visibility and cache tests

Verify:

- effective public visibility requires public workspace and repository;
- private workspace plus public repository is rejected;
- workspace privatization also privatizes repositories;
- first publication requires warning acknowledgement;
- visibility generation changes before success;
- old cache keys are not served after privatization;
- mutable heads and private responses are no-store; and
- direct R2 access is unavailable.

### 16.4 Branch-policy tests

Verify:

- main is protected by default when it is the primary branch;
- develop is not protected by default;
- Admin can protect and unprotect an exact branch;
- Member can direct-push an unprotected branch;
- Member direct push to a protected branch returns 403;
- Admin can direct-push a protected branch;
- managed repository creation rejects a non-Git project;
- a policy change invalidates a stale direct-push plan; and
- branch patterns and wildcards are rejected.

### 16.5 Merge-request tests

Verify:

- MR creation starts from the exact protected-branch head;
- one new cloud chat ID is required;
- one chat ID cannot serve two MRs;
- an existing MR chat cannot be reused;
- draft source can advance;
- open freezes source;
- open source cannot advance;
- Member cannot accept or reject;
- Admin can accept the exact source and target;
- Admin can manually reject;
- target change atomically rejects;
- Git conflict result rejects;
- rejection leaves target unchanged;
- accepted, rejected, and withdrawn MRs cannot reopen;
- terminal retry requires a new MR and chat; and
- ambiguous acceptance is resolved by read-back.

### 16.6 Bundle and storage tests

Run the object-store contract in section 5.4 against the managed adapter:

- immutable create;
- identical create replay;
- mismatched immutable collision;
- exact verified read;
- list cursor bounds;
- per-ref head compare-and-swap;
- manifest and payload digest verification;
- interruption before head change;
- orphan recovery; and
- local-folder and S3 behavior unchanged.

### 16.7 Frontend and Tauri tests

Add frontend integration coverage for:

- role-specific controls;
- Public and Private labels;
- main and develop behavior;
- Create merge request;
- non-Git repository rejection;
- new-chat-only MR creation;
- Admin Accept and Reject;
- conflict terminal state;
- no conflict-resolution controls;
- public-disclosure warning; and
- absence of removed encryption and role terms.

Run both frontend and Rust checks for every Tauri contract change.

### 16.8 Security and fault tests

Test:

- OAuth state, PKCE, callback, and code replay;
- refresh-token rotation and replay;
- invitation token theft, mismatch, expiry, replay, and revocation;
- cross-workspace ID probing;
- path and encoded-path attacks;
- oversized inputs and pagination;
- D1 and R2 partial failures;
- queue duplicate delivery;
- log redaction;
- public read rate limits; and
- archive, restore, and final purge.

## 17. Production acceptance criteria

Production release requires all of the following:

- A public demo can be discovered and pulled without an account.
- Anonymous users cannot write to a public workspace or repository.
- Invitation is the only public-workspace write-entry path.
- Admin and Member are the only roles.
- The last-Admin invariant survives concurrent changes.
- A private workspace and repository are not enumerable by an outsider.
- Main is protected by default.
- A Member can push develop when it is not protected.
- A Member cannot direct-push any protected branch.
- An Admin can configure exact protected branches.
- Managed Cloud V1 rejects non-Git repository creation.
- A protected Git Member change uses an MR.
- Every MR has one new chat.
- An Admin can accept or reject an open MR.
- A changed target causes terminal conflict rejection.
- A Git conflict reported by the Admin desktop causes terminal rejection.
- The product has no rebase, auto-merge, conflict editor, or reopen path.
- Public-to-private change blocks new anonymous reads immediately.
- Public disclosure has an explicit warning and acknowledgement.
- Managed content uses no Mallard client-side encryption or key lifecycle.
- Access and refresh credentials are not stored in normal JSON configuration.
- All immutable objects and accepted heads pass size and SHA-256 checks.
- Direct head changes and accepted MRs are compare-and-swap safe.
- Logs and audit records contain no credentials or user content.
- Backup, restore, queue recovery, and deletion exercises pass.
- Local-folder and user-owned S3/R2 paths remain unchanged.

## 18. Assumptions and deferred work

### 18.1 Assumptions

- The managed service has no deployed users and no stored production data.
- No managed-cloud encryption schema has been deployed. The initial schema
  contains no content-key fields or compatibility migration.
- The first production population fits one D1 database.
- The desktop remains the only write client.
- Managed repository branch refs represent Mallard bundle history, not Git
  source storage.
- Admins are trusted to make MR decisions.
- Supported operating systems provide a usable credential keychain.
- Deployment tests verify the required Cloudflare product capabilities and
  current limits.

### 18.2 Deferred work

The first release excludes:

- billing and subscriptions;
- email and password authentication;
- per-repository roles;
- public MR drafts and comments;
- branch patterns and policy inheritance;
- required reviewer counts;
- Member exceptions on protected branches;
- automatic merge, rebase, conflict resolution, and MR reopen;
- server-authoritative Git source merge validation;
- Git-host pull-request creation or merge;
- server-side source-code hosting;
- web write clients;
- public full-text search;
- direct public R2 domains;
- presigned direct uploads;
- Durable Objects;
- regional shards;
- moving a repository between workspaces;
- managed non-Git repositories and the related push-request workflow; and
- client-side or end-to-end content encryption.

## 19. Official references

Cloudflare:

- [Cloudflare D1](https://developers.cloudflare.com/d1/)
- [D1 Worker API](https://developers.cloudflare.com/d1/worker-api/)
- [D1 Time Travel](https://developers.cloudflare.com/d1/reference/time-travel/)
- [R2 Workers API](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [R2 multipart uploads](https://developers.cloudflare.com/r2/api/workers/workers-multipart-usage/)
- [R2 bucket locks](https://developers.cloudflare.com/r2/buckets/bucket-locks/)
- [Workers Cache](https://developers.cloudflare.com/workers/runtime-apis/cache/)
- [Cloudflare Queues](https://developers.cloudflare.com/queues/)
- [Queue dead-letter queues](https://developers.cloudflare.com/queues/configuration/dead-letter-queues/)
- [Workers secrets](https://developers.cloudflare.com/workers/configuration/secrets/)
- [Cloudflare Access policies](https://developers.cloudflare.com/cloudflare-one/access-controls/policies/)
- [Workers observability](https://developers.cloudflare.com/workers/observability/)

Identity:

- [OAuth 2.0 for Native Apps, RFC 8252](https://www.rfc-editor.org/rfc/rfc8252)
- [GitHub OAuth authorization](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps)
- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
