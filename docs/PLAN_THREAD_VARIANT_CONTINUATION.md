# Thread Variant Continuation Implementation Plan

## Decision and scope

**Status:** approved product direction; implementation and rollout remain gated
by the provider capability proof, schema-5 validation, and crash-recovery tests
defined below.

This plan defines how Mallard preserves and continues divergent conversation
versions without silently changing the branch a person is using. It is
self-contained: it covers behavior, identity, every new metadata field,
schema migration, provider materialization, crash recovery, user experience,
and acceptance tests.

`THREAD_SYNC_CASES.md` remains the source of truth for the basic Push/Pull
matrix. This plan adds the multi-variant and provider-thread rules needed to
implement that matrix safely.

The product decision is:

> Pull preserves the person's current conversation branch and provider thread.
> It may add or fast-forward versions, but it never silently switches branches.

This is intentionally about the branch the user experiences, not merely the
current value of an internal `variant_id`. When a previously unknown fork is
discovered under one shared variant ID, Mallard may atomically rekey the local
branch to a new variant ID while preserving its transcript, provider thread ID,
local preference, and resume behavior.

## User-visible result

For one logical conversation group `G`:

~~~text
V1 = [a, b, c1]
V2 = [a, b, c2]
~~~

the required result is:

- A continues V1 through A's existing provider thread.
- B continues V2 through B's existing provider thread.
- Pull adds the other version; it does not replace or rename the existing
  provider thread.
- Both versions can be explicitly opened and continued after safe
  materialization.
- Pushing from one version advances only that version.
- The user may notice another version and a new provider thread for that
  imported version. The user must not see an existing conversation change ID,
  lose messages, or switch branches.

### A/cloud/B flow

Assume the cloud contains both portable variants:

~~~text
Cloud group G:
  primary: V1
  V1 -> [a, b, c1]
  V2 -> [a, b, c2]
~~~

A already has V1:

~~~text
A before Pull:
  preferred branch: V1
  V1 -> provider thread A-T1 -> [a, b, c1]

A after Pull:
  preferred branch: V1
  V1 -> provider thread A-T1 -> [a, b, c1]
  V2 -> provider thread A-T2 -> [a, b, c2]
~~~

B already has V2:

~~~text
B before Pull:
  preferred branch: V2
  V2 -> provider thread B-T2 -> [a, b, c2]

B after Pull:
  preferred branch: V2
  V1 -> provider thread B-T1 -> [a, b, c1]
  V2 -> provider thread B-T2 -> [a, b, c2]
~~~

`A-T2` and `B-T1` are destination-local IDs. They need not equal the provider
ID stored as source provenance, and they need not equal each other.

Mallard also distinguishes the installations without exposing their real
hostnames:

~~~text
A -> local host_id H-A -> portable token origin-A-for-bundle
B -> local host_id H-B -> portable token origin-B-for-bundle
~~~

On B, `origin-B-for-bundle` is shown as “This host” and
`origin-A-for-bundle` as a locally assigned alias such as “Host A.” These
origin tokens are presentation and provenance only; they never decide
overwrite, ancestry, authorization, or CAS behavior.

If A later appends `d1` and pushes V1, the cloud becomes:

~~~text
V1 = [a, b, c1, d1]
V2 = [a, b, c2]
~~~

If B later appends `d2` and pushes V2, only V2 advances. Neither operation
creates a third variant merely because the two machines use different provider
thread IDs.

## Independent choices

Three values must remain independent.

### Cloud primary

`primary_variant_id` controls which shared version the bundle presents first.
Only an explicitly reviewed cloud Promote action may change it. Publication
uses compare-and-swap against the reviewed head revision and manifest digest.

### Local preferred branch

`preferred_variant_id` controls local ordering and the “On this machine” badge.
Ordinary sync preserves the same branch and provider thread.

Normally the ID value is unchanged. There is one required internal exception:
when local and cloud have diverged under the same `variant_id`, the cloud keeps
that ID and the existing local branch is atomically assigned a new ID. If the
old ID was preferred, the preference moves to the new ID in the same local
transaction. This is an identity repair, not a user-visible promotion or
branch switch.

On a fresh replica, the reviewed cloud primary may initialize local preference
only after that variant becomes safely resumable. If it remains stored-only,
Mallard may order it first using the cloud primary but must not claim it is the
local preferred resumable branch.

### Active provider conversation

The active conversation is whichever provider thread the user opens. It is
external provider state, not a cloud or local-primary pointer. Pull must not
launch, focus, resume, or switch Codex or Claude.

Mallard also must not rewrite a transcript or provider index while the provider
can be writing it unless the provider adapter has a verified locking or
quiescence mechanism. Without that proof, the action is deferred or stored-only.

## Identity and digest model

### `conflict_group_id`

- Portable identity for one logical conversation.
- Shared by every variant of that conversation.
- Assigned before the first known fork.
- Stable across replicas and storage revisions.
- Contains no local path, profile ID, credential, or machine identity.

### `variant_id`

- Portable identity for one continuing branch within a conflict group.
- Stable across exact matches and fast-forwards.
- Changes only when a previously shared identity is split into distinct
  branches.
- Determines which cloud branch a later Push updates.
- Is never derived from the current head alone, because the head changes as the
  branch grows.

### `provider_thread_id`

- Actual Codex thread ID or Claude session ID on one provider profile.
- Machine-local binding, not portable branch identity.
- Unique across the entire provider profile, including different Mallard
  projects using that profile.
- May differ for the same variant on A and B.
- Existing local content always wins an ID collision.

A bounded source provider ID may remain in portable metadata only as provenance
and an allocation hint. It must never be used as the group or variant key and
must never override an existing destination thread.

### Message ancestry

Provider-specific codecs extract one linear logical path:

~~~text
message_id, parent_message_id, canonical message fingerprint
~~~

Message ID and parentage determine ancestry. For every common message ID, the
canonical fingerprint must also match; the classifier must not accept changed
content merely because an ID was reused.

A codec that cannot prove a unique linear path returns `Unknown`. Mallard then
preserves both sides or stores the incoming version without guessing.

### Four distinct digests

The implementation must not overload one SHA-256 value with four meanings:

| Digest | Purpose | Stored in |
| --- | --- | --- |
| Object SHA-256 | Verifies the immutable cloud object bytes | Existing `BundleFileEntry.sha256` |
| Local file SHA-256 | Detects target changes between review and apply | Local binding, plan pin, receipt |
| Lineage digest | Hash of ordered message IDs and parent IDs | Variant metadata and local observation |
| Semantic digest | Hash of canonical provider records after excluding approved local structural fields | Variant metadata and local observation |

Exact equality requires equal lineage and semantic digests. Prefix
classification compares the full bounded parsed chains and verifies the
canonical fingerprints of the common prefix. Raw object and local file hashes
may differ because destination thread IDs and `cwd` fields are intentionally
rewritten.

The lineage construction and semantic canonicalization are versioned provider
codec behavior. A codec change must not silently compare values produced by an
older incompatible codec.

## Distinguishing different hosts

Mallard distinguishes a host, a project replica, a conversation variant, and a
provider thread separately:

| Identity | Scope | Portable | Purpose |
| --- | --- | --- | --- |
| `host_id` | One Mallard installation | No | Distinguishes installations on local state |
| `replica_id` | One project/profile binding on that host | No | Distinguishes local project replicas |
| `origin_host_token` | One host within one bundle | Yes | Privacy-preserving host provenance |
| `origin_replica_token` | One replica within one bundle | Yes | Distinguishes multiple replicas on one host |
| `conflict_group_id` | Logical conversation | Yes | Groups branches |
| `variant_id` | One branch | Yes | Identifies the branch being synced |
| `provider_thread_id` | One provider profile | No | Resumes the local provider conversation |

### Local host identity

`host_id` is a random opaque 128-bit installation ID generated on first use.
It is not the OS hostname, username, MAC address, hardware serial number, or a
hash of any of those values. It identifies a Mallard installation, not a
physical person or infallibly a physical device.

`replica_id` remains the identity of a particular local project/provider
binding. One host may have several replica IDs, while every replica belongs to
exactly one host:

~~~text
Host H-A
  replica R-A1 -> project P1 + Codex profile
  replica R-A2 -> project P2 + Claude profile
~~~

Every local binding, preference, plan, journal record, and receipt pins both
`host_id` and `replica_id`. A plan created on one host cannot be applied after
state is moved to another host or after the host identity is rotated.

### Portable host provenance

Raw `host_id` and `replica_id` never enter a bundle. The host owns a random
256-bit `origin_key` stored only in protected machine-local state. For each
bundle it derives:

~~~text
origin_host_token =
  HMAC-SHA256(origin_key,
              "mallard-origin-host-v1",
              bundle_id)

origin_replica_token =
  HMAC-SHA256(origin_key,
              "mallard-origin-replica-v1",
              bundle_id,
              replica_id)
~~~

The exact input encoding is domain-separated and length-prefixed. Tokens are
stable for one host/replica within one bundle, but different bundles receive
different tokens and cannot be linked using the token alone. Storage ID is not
part of the derivation, so two destinations carrying the same logical bundle
show the same host provenance.

Portable origin metadata records:

- who first created a variant when known;
- who last changed its semantic content; and
- which host published a manifest commit.

A schema-only migration does not claim that the migrating host created old
content. Migrated schema-4 variants therefore have unknown creator/last-editor
origins until a later semantic update supplies one.

Origin tokens are untrusted provenance. Another writer can copy or spoof a
token, so Mallard must never use one to authorize access, infer ancestry,
select a merge winner, permit replacement, or bypass digest and CAS checks.

### Host labels

Mallard never uploads the OS hostname automatically. Each machine keeps a
local mapping:

~~~text
(bundle_id, origin_host_token) -> display alias
~~~

The current host is displayed as “This host.” Unknown remote tokens receive a
short non-sensitive fallback such as `Host 7F3A` and can be renamed locally.
An optional shared origin label is allowed only after explicit user approval
and must pass bounded display-text validation.

When several replicas belong to the same host, the UI can add a local project
or profile label while using `origin_replica_token` to keep them distinct.

### Copied or cloned host state

A byte-for-byte copy of all Mallard state and its protected origin key is
indistinguishable from the original using that copied data alone. The plan must
not claim otherwise.

Normal migration handles this as follows:

1. `host_identity.json` contains `host_id` and a reference to the protected
   origin key.
2. If the key is missing or cannot be opened on this OS account, Mallard treats
   the state as imported and requires “Register as new host.”
3. Registering as a new host generates a new `host_id`, origin key, and replica
   IDs, while preserving conflict-group IDs, variant IDs, transcripts, and
   reviewed cloud history.
4. Existing conversation bindings are re-associated only after local provider
   IDs and semantic digests are revalidated.
5. A user may explicitly rotate host identity if a full machine image was
   cloned and automatic detection is impossible.

No hardware fingerprint is stored or uploaded. Host-identity rotation never
renames existing provider threads and never creates a cloud variant unless
the conversation content itself later changes.

## State classification

For a reviewed local branch `L` and corresponding cloud branch `C`:

| Relation | Push | Pull |
| --- | --- | --- |
| Identical | No-op | No-op |
| Cloud is an exact prefix of local | Fast-forward that cloud variant | Keep local |
| Local is an exact prefix of cloud | Block and require Pull | Fast-forward the bound local thread |
| Diverged | Retain cloud; add/rekey local variant | Retain local; add cloud variant |
| Unknown | Preserve both or block | Preserve both or store-only |
| Local missing | No Push action | Materialize or store-only |
| Cloud missing, no tombstone | Add local | Keep local |
| Explicit tombstone versus unchanged copy | Apply only after review | Apply only after review |
| Tombstone versus modified copy | Treat as divergence | Treat as divergence |

Absence is never deletion.

### Required same-ID divergence transition

The first observed fork commonly has one shared ID:

~~~text
Before:
  Cloud: variant V -> provider provenance T -> C
  Local: variant V -> provider thread T -> L
  L and C diverged
~~~

The safe Pull result is:

~~~text
After:
  Cloud identity remains:
    V -> C

  Existing local branch is rekeyed without changing user state:
    V-local -> provider thread T -> L

  Incoming cloud branch is materialized separately:
    V -> provider thread T-new -> C

  If preferred was V:
    preferred becomes V-local
~~~

This is one journaled logical transition. Mallard writes the durable journal
first, then atomically saves the binding rekey and preference update together
in `conversation_bindings.json`. The existing transcript and provider ID `T`
are not renamed. `T-new` is reserved for the incoming cloud branch.

The safe Push transition is similar:

1. Persist a new stable local `variant_id` and rekey the local binding first.
2. Keep the existing cloud variant unchanged.
3. Publish the new local variant with head compare-and-swap.
4. Finalize the local journal after confirming the published commit.

If cloud publication fails, the local branch remains a valid local-only variant
with the same new ID and can be retried. If cloud publication succeeds but the
final local write fails, startup recovery identifies the published commit and
finishes the local transaction instead of creating another variant.

## Metadata updates

Every field belongs to exactly one tier.

| Tier | Crosses machines | Storage |
| --- | --- | --- |
| Portable | Yes | Bundle schema 5 manifest and immutable objects |
| Machine-local | No | Versioned files under `~/.mallard` |
| Plan-time | No; expires | Reviewed Push/Pull plans |
| Recovery | No | Durable local conversation operation journal |
| Receipt | No | Materialization and publication receipts |

Portable metadata must not contain credentials, provider-profile IDs, local
absolute paths, backup paths, plan IDs, local preferences, local bindings, or
local provider IDs used as identity. It also excludes raw host/replica IDs,
hostnames, usernames, hardware identifiers, and the origin key. Activity logs
must sanitize provider IDs, origin keys, absolute paths, transcript content,
and journal details.

### Existing metadata changed

| Existing type or document | Schema-5/local change |
| --- | --- |
| `BundleHead` | Set `schema_version = 5`; revision and CAS semantics stay unchanged |
| `BundleManifest` | Set schema 5; add `conversation_groups` and `published_by_origin` |
| `CommitRecord` | Set schema 5 and add untrusted `published_by_origin` provenance |
| `BundleTag` | Set schema 5 and count variant-addressed resources; do not expose host provenance in discovery tags |
| `BundleRecipe` | Use group-and-variant resource IDs and carry forward unselected remote variants |
| `ResourceDescriptor` | Conversation `resource_id` becomes group-and-variant addressed; `display_name` is presentation, never identity |
| `BundleFileEntry` | Conversation ownership points to the new variant resource ID |
| `CapturedWith.codec_versions` | Add versioned Codex/Claude lineage, semantic, and rewrite codec keys |
| `TombstoneTarget` | Add conversation-variant and conversation-group targets |
| `RestorePlan` and `RestoreAction` | Bump the plan schema and add typed conversation action metadata and target pins |
| `ApplyReceipt` and `MaterializationRecord` | Add the typed local conversation result and journal operation ID |
| `ProjectBinding` | Add local `host_id` alongside the existing `replica_id` |
| Thread comparison DTOs | Replace provider-ID identity with group, variant, relation, primary, preference, and capability fields |
| Activity records | Keep the existing sanitized shape; never add provider IDs, local paths, content, bindings, or journal payloads |

The existing `relative_cwd` remains project-relative. No new portable field may
store the source or destination absolute checkout path.

### Portable schema 5

Bump the bundle head, manifest, commit record, and bundle tag together from
schema 4 to schema 5. Add `conversation_groups` to `BundleManifest`:

~~~text
BundleManifest
  schema_version = 5
  ...
  published_by_origin: PortableOrigin
  conversation_groups:
    conflict_group_id -> ConversationGroup
~~~

Schema constants introduced or changed by this plan are:

| Document | Schema |
| --- | --- |
| Bundle head, manifest, commit, and tag | `BUNDLE_SCHEMA_V5 = 5` |
| Machine projects and project bindings | `MACHINE_PROJECT_SCHEMA_V2 = 2` |
| Conversation bindings | `CONVERSATION_BINDINGS_SCHEMA_V1 = 1` |
| Conversation sync journal | `CONVERSATION_SYNC_JOURNAL_SCHEMA_V1 = 1` |
| Host identity | `HOST_IDENTITY_SCHEMA_V1 = 1` |
| Reviewed restore plan | `RESTORE_PLAN_SCHEMA_V2 = 2` |
| Reviewed Push plan | `PUSH_PLAN_SCHEMA_V1 = 1` |

Existing saved restore plans are ephemeral and rejected after the schema
change. `SyncConfigV3` stays schema 3. `machine_projects.json` receives a
one-way schema-1-to-schema-2 migration because `ProjectBinding` gains
`host_id`; the other new durable fields live in separately versioned local
documents.

#### `ConversationGroup`

| Field | Type | Rule |
| --- | --- | --- |
| `conflict_group_id` | `ConflictGroupId` | Must equal its map key |
| `provider` | `Provider` | Must match every variant resource |
| `display_title` | Optional bounded string | Presentation only |
| `primary_variant_id` | `VariantId` | Must name one live variant |
| `variants` | `BTreeMap<VariantId, ConversationVariant>` | Non-empty and bounded |
| `created_at` | `u64` | Portable timestamp |
| `updated_at` | `u64` | Must be at least `created_at` |

The group has no independent revision. The bundle revision and manifest
head CAS are the concurrency authority.

#### `ConversationVariant`

| Field | Type | Rule |
| --- | --- | --- |
| `variant_id` | `VariantId` | Must equal its map key |
| `resource_id` | `ResourceId` | Group-and-variant addressed |
| `files` | Bounded list of `ConversationFileRef` | Exactly one transcript |
| `lineage_status` | `Complete` or `Unknown(reason_code)` | Bounded reason code only |
| `lineage_codec` | Bounded codec identifier | Must match `captured_with.codec_versions` |
| `head_message_id` | Optional `MessageId` | Required when lineage is complete |
| `message_count` | Optional `u64` | Required and non-zero when complete |
| `lineage_digest` | Optional SHA-256 | Required when complete |
| `semantic_digest` | Optional SHA-256 | Required when complete |
| `parent_variant_id` | Optional `VariantId` | Provenance only |
| `fork_message_id` | Optional `MessageId` | Provenance only |
| `source_thread_id` | Optional bounded string | Provenance/allocation hint only |
| `created_by_origin` | Optional `PortableOrigin` | Unknown for migrated historical content |
| `last_updated_by_origin` | Optional `PortableOrigin` | Changes only with semantic content |
| `created_at` | `u64` | First creation time |
| `updated_at` | `u64` | Last portable branch update |

Do not add a duplicate `transcript_sha256`. Raw immutable-object integrity
already belongs to `BundleFileEntry.sha256`. The new `semantic_digest` has a
different, explicitly normalized meaning.

`parent_variant_id` may refer to a live or tombstoned variant in the same group.
Validation rejects self-parenting and cycles. It is provenance, never an
instruction to merge content.

#### `PortableOrigin`

~~~text
PortableOrigin
  origin_host_token
  origin_replica_token
  shared_label?             # explicit opt-in only
~~~

`origin_host_token` and `origin_replica_token` are validated fixed-length
HMAC outputs. They contain no raw host or replica ID. `shared_label` is
optional, bounded, sanitized, and never populated automatically from the OS
hostname.

`BundleManifest.published_by_origin` and
`CommitRecord.published_by_origin` record which origin performed publication.
They are audit/display hints only. `BundleTag` omits them so remote discovery
does not expose host relationships before a bundle is opened.

#### `ConversationFileRef`

A conversation variant may own a transcript plus provider state needed to make
that transcript resumable:

~~~text
ConversationFileRef
  logical_path
  role:
    Transcript
    ProviderIndexFragment
    Sidecar(kind)
~~~

Rules:

- Exactly one file has role `Transcript`.
- Paths and sidecar kinds are bounded and validated.
- Every listed path exists in `BundleManifest.files` and is owned by the
  variant's `resource_id`.
- Every file owned by a conversation variant is listed exactly once.
- No file or logical path is shared by two variants.
- A provider adapter must recognize every role before it can advertise the
  variant as resumable.

Use provider-neutral portable paths, for example:

~~~text
state/conversations/<provider>/<group>/<variant>/transcript.jsonl
state/conversations/<provider>/<group>/<variant>/index.jsonl
state/conversations/<provider>/<group>/<variant>/sidecars/<kind>/<name>
~~~

Destination provider paths are computed locally by the provider codec. Portable
paths and resource keys must never depend on the destination provider thread ID.

Conversation resource IDs become:

~~~text
<provider>:conversation:<conflict_group_id>:<variant_id>
~~~

The current global `codex:session-index` representation is not authoritative
for schema-5 variants. Cutover decomposes its selected conversation rows into
per-variant fragments or typed title metadata; target-only and unrelated rows
remain local. Pull rewrites or generates the destination row using the
allocated provider ID.

#### Lineage metadata

`lineage_digest` is SHA-256 over an unambiguous, versioned encoding of ordered
`message_id` and `parent_message_id` pairs. Do not use separator rules that can
collide; use length-prefixed fields or canonical JSON and cover the encoding
with golden tests.

`semantic_digest` is SHA-256 over the codec's canonical logical records. It
excludes only explicitly enumerated structural fields such as the local
provider thread ID and mapped `cwd`. It includes message content, roles,
timestamps when semantically significant, tool calls, tool results, and
provider records required for safe continuation.

The manifest stores summaries, not every message. Classification streams the
bounded transcript object on demand when a prefix decision is required.

#### Manifest and recipe validation

Schema-5 validation must enforce:

- each group key and variant key matches its embedded ID;
- group IDs and variant resource IDs are globally unique;
- primary points to a live variant;
- providers agree across group, variant resource, and codec;
- portable origins contain valid fixed-length scoped tokens and only explicitly
  approved shared labels;
- complete lineage has all required fields and valid SHA-256 values;
- unknown lineage cannot carry misleading complete-only fields;
- parent links stay in-group and are acyclic;
- every conversation variant maps to one descriptor and a complete file set;
- every conversation descriptor belongs to exactly one live variant;
- no live variant is simultaneously tombstoned;
- collection, path, file-size, and metadata limits remain enforced; and
- `manifest.recipe.selected_ids()` still equals the live resource set.

Push builds the next conversation set as:

~~~text
previous live variants
+ reviewed local additions and updates
- explicitly reviewed tombstones
~~~

An unselected, missing, or locally unavailable conversation is carried forward;
it is not removed. Unchanged remote-only descriptors, files, and immutable
object keys are copied from the reviewed previous manifest. This preserves V2
when A pushes only V1 while keeping the existing recipe/resource equality
invariant.

#### Tombstone metadata

Extend `TombstoneTarget` with:

~~~text
ConversationVariant {
  conflict_group_id,
  variant_id
}

ConversationGroup {
  conflict_group_id
}
~~~

A variant tombstone identifies one deleted branch. A group tombstone represents
explicit deletion of the final logical conversation. `last_sha256` records the
canonical resource-tree digest reviewed for deletion.

The primary variant cannot be tombstoned unless the same reviewed cloud CAS
selects another live primary, or explicitly tombstones the entire group.
A delete/modify race creates or preserves a live variant and does not apply the
tombstone to modified content.

#### Codec metadata

Record each enabled conversation codec in
`CapturedWith.codec_versions`, keyed by provider and codec purpose. A variant's
`lineage_codec` must resolve to that entry. Pull may materialize provider-visible
content only when the destination adapter declares compatibility with that
codec version.

### Machine-local metadata

Add `host_identity.json`:

~~~text
HostIdentity
  schema = 1
  revision
  host_id
  origin_key_ref
  local_display_name?
  created_at
  updated_at
~~~

`origin_key_ref` identifies an OS-protected 256-bit key. If the platform secret
service is unavailable, the fallback is a dedicated no-follow file below
`~/.mallard` with mode `0600`, atomic creation, and no backup/export behavior.
The origin key is not an authentication credential, but it is privacy-sensitive
and must never be logged or serialized into another document.

`local_display_name` is user-editable and remains local. The OS hostname may be
offered as a local default after validation, but it is never portable without
separate explicit approval.

Add `conversation_bindings.json` beside existing machine-local state files.
It has its own schema rather than changing `SyncConfigV3`:

~~~text
ConversationBindings
  schema = 1
  revision
  host_id
  bindings: [LocalConversationBinding]
  preferences: [LocalConversationPreference]
  origin_aliases: [OriginHostAlias]
~~~

Persistence uses bounded collections, validation, no-follow access, atomic
replacement, fsync behavior consistent with existing state, and a document
revision checked by every mutation.

#### `LocalConversationBinding`

| Field | Type | Rule |
| --- | --- | --- |
| `host_id` | `HostId` | Must match `host_identity.json` |
| `replica_id` | `ReplicaId` | Current replica |
| `local_project_id` | `LocalProjectId` | Must reference a live or explicitly detached project |
| `provider_profile_id` | `LocalProviderProfileId` | Must match the group's provider |
| `conflict_group_id` | `ConflictGroupId` | Portable identity |
| `variant_id` | `VariantId` | Portable branch identity |
| `provider_thread_id` | Optional string | Required only for pending/resumable materialization |
| `materialization_state` | `Pending`, `Resumable`, or `StoredOnly` | Controls Continue availability |
| `last_observed_file_sha256` | Optional SHA-256 | Raw local target pin |
| `last_observed_head_message_id` | Optional `MessageId` | Cached observation |
| `last_observed_lineage_digest` | Optional SHA-256 | Cached observation |
| `last_observed_semantic_digest` | Optional SHA-256 | Cached observation |
| `source_revision` | Optional `u64` | Last applied cloud revision |
| `source_manifest_sha256` | Optional SHA-256 | Last applied manifest |
| `revision` | `u64` | Per-binding optimistic concurrency |
| `created_at` | `u64` | |
| `updated_at` | `u64` | |

`StoredOnly` has no provider thread ID and cannot offer Continue. `Pending` is
visible only to recovery and diagnostic UI. `Resumable` requires a valid
provider ID, complete provider file set, and verified codec round-trip.

Required uniqueness:

- one record per
  `(host, replica, project, profile, conflict_group_id, variant_id)`;
- one owner for
  `(host, provider_profile_id, provider_thread_id)` across all projects;
- one provider ID cannot map to two variants; and
- a live binding cannot reference a missing host, replica, provider profile, or
  wrong provider.

The uniqueness scope deliberately excludes `local_project_id` from the
provider-ID ownership key: provider IDs must be unique throughout one provider
home, not merely within one Mallard project.

#### `LocalConversationPreference`

| Field | Type |
| --- | --- |
| `host_id` | `HostId` |
| `replica_id` | `ReplicaId` |
| `local_project_id` | `LocalProjectId` |
| `provider_profile_id` | `LocalProviderProfileId` |
| `conflict_group_id` | `ConflictGroupId` |
| `preferred_variant_id` | `VariantId` |
| `revision` | `u64` |
| `updated_at` | `u64` |

There is at most one preference per replica/project/profile/group. It must point
to a locally known variant. Ordinary Pull cannot promote another branch. The
same-ID divergence rekey is the sole automatic pointer update, and it must
preserve the same provider thread and transcript.

#### `OriginHostAlias`

| Field | Type | Rule |
| --- | --- | --- |
| `bundle_id` | `BundleId` | Scopes the derived host token |
| `origin_host_token` | `OriginHostToken` | Portable pseudonym being named |
| `display_name` | Bounded string | Local presentation only |
| `revision` | `u64` | Optimistic concurrency |
| `updated_at` | `u64` | |

Aliases are not copied into portable manifests. “This host” is resolved by
deriving the current host token for the bundle and comparing it locally.

Detach and removal flows mark or garbage-collect bindings explicitly. They must
not silently delete provider transcripts, backups, receipts, or portable
variants.

### Recovery metadata

Add `conversation_sync_journal.json`:

~~~text
ConversationSyncJournal
  schema = 1
  revision
  operations: [ConversationSyncOperation]
~~~

Each operation contains only machine-local data:

| Field | Purpose |
| --- | --- |
| `operation_id` and optional `plan_id` | Idempotent recovery key |
| `direction` | Pull or Push |
| `phase` | Prepared, Staged, CloudPublished, ProviderCommitted, LocalCommitted, RecoveryRequired |
| `storage_id`, `bundle_id` | Reviewed destination/source |
| `revision`, `commit_id`, `manifest_sha256` | Cloud pin or published result |
| `host_id`, `replica_id` | Installation and local-replica scope |
| `local_project_id`, `provider_profile_id` | Project/profile scope |
| `conflict_group_id` | Logical conversation |
| `old_variant_id`, `result_variant_id` | Records rekeys |
| `provider_thread_id` | Reserved or existing local ID |
| `expected bindings/profile/document revisions` | Concurrency pins |
| `expected local and provider-state digests` | Target pins |
| `staged targets and backup references` | Local recovery only |
| `created_at`, `updated_at` | Bounded retention |

Journal writes occur before any externally visible mutation. Records are
removed only after the provider file set, local binding/preference, receipt,
and any cloud publication are reconciled. Startup recovery completes or safely
rolls back unfinished operations. It never guesses from a filename alone.

Journal contents, absolute paths, provider IDs, and backup locations never enter
portable manifests or activity logs.

### Plan-time metadata

Bump the ephemeral restore-plan schema and reject outstanding older plans after
upgrade. Add a typed conversation payload to Pull actions and an equivalent
reviewed Push plan.

~~~text
ConversationPlanAction
  action:
    NoOp
    FastForwardLocal
    KeepLocal
    AddAlternate
    StoreOnly
    PromoteLocal
    PromoteCloud
    ReplaceLocal
    TombstoneVariant
    TombstoneGroup
  relation
  provider
  host_id
  replica_id
  conflict_group_id
  cloud_variant_id?
  local_variant_id?
  result_local_variant_id?
  source_resource_id?
  proposed_provider_thread_id?
  required_codec
  pinned_revision
  pinned_commit_id
  pinned_manifest_sha256
  pinned_lineage_digest?
  pinned_semantic_digest?
  expected_local_target:
    Absent
    Sha256(value)
  expected_provider_state[]         # path plus Absent-or-Sha256 expectation
  expected_profile_revision
  expected_host_identity_revision
  expected_bindings_document_revision
  expected_binding_revision?
  expected_preference_revision?
  expires_at
~~~

Target expectations are explicit tagged values; an omitted field never means
“skip validation.” The proposed provider ID and every target path are reserved
in the plan.
Apply revalidates all cloud, local, binding, profile, target-occupancy, codec,
host-identity, and expiry pins immediately before mutation.

Replacement, promotion, and deletion remain separate explicit actions. An
`AddAlternate` action cannot be silently converted into replacement at apply
time.

### Receipt metadata

Extend machine-local materialization/publication receipts with a typed
conversation result:

~~~text
ConversationApplyReceipt
  host_id
  replica_id
  conflict_group_id
  source_variant_id?
  result_variant_id
  provider_profile_id
  provider_thread_id?
  materialization_state
  lineage_digest?
  semantic_digest?
  local_file_sha256_after?
  cloud_revision
  cloud_commit_id
  cloud_manifest_sha256
  published_origin_host_token?
  published_origin_replica_token?
  journal_operation_id
~~~

The receipt records the actual allocated provider ID and rekey result. Repeat
Pull and startup recovery use the binding and receipt to prove idempotence.
Old non-conversation receipts remain readable. Conversation receipt fields are
machine-local and must be sanitized from logs.

### Command and frontend DTO metadata

Update Rust DTOs and `src/types.ts` together. At minimum,
`ThreadSyncEntry` and Pull/Push review DTOs expose:

- `conflict_group_id` and `variant_id`;
- resolved origin-host label, origin-host/replica tokens, and
  `is_current_host`;
- provider and optional local provider thread ID;
- cloud-primary and local-preferred flags;
- materialization state;
- relation and lineage status;
- local/cloud timestamps;
- resumable capability and any bounded reason code; and
- explicit available actions.

The frontend groups entries by conflict group. Provider IDs appear only in
details and resume commands, not as the primary label. All Tauri calls continue
through `projectSyncApi`.

### Metadata change checklist by module

| Module | Required update |
| --- | --- |
| `domain.rs` | Schema 5 constants; host/origin/variant ID types; host identity, portable origin, group, variant, file-role, lineage, tombstone, plan, journal, and receipt types with validation |
| `provider_capture.rs` | Versioned provider codec output, file-role inventory, ancestry and semantic summaries |
| `bundle_engine.rs` | Schema-5 manifest assembly, remote-variant retention, variant tombstones, typed conversation planning, staged apply |
| `persistence.rs` | Protected host identity/origin key access; atomic bounded bindings, aliases, and journal documents; recovery loading |
| `commands.rs` | Host registration/rotation, scoped origin-token derivation, per-variant comparison, reviewed Push/Pull orchestration, recovery, DTO population |
| `src/types.ts` | Exact serialized DTO mirrors |
| `projectSyncApi` | Typed plan, apply, promote, retry, and recovery calls |
| History and review UI | Grouped variants, primary/preferred distinction, stored-only and pending states |

## Schema-4 to schema-5 cutover

This is a one-way, explicitly reviewed cutover, not permanent dual behavior.

### Deterministic initial IDs

Define and golden-test one domain-separated, length-prefixed stable-ID
algorithm:

~~~text
conflict_group_id =
  stable_id("mallard-conversation-group-v1",
            bundle_id,
            provider,
            schema4_resource_id)

initial_variant_id =
  stable_id("mallard-conversation-variant-v1",
            conflict_group_id,
            schema4_resource_id)
~~~

The exact byte encoding and output format are part of the schema-5 contract.
Every replica must derive identical IDs from the same schema-4 manifest.

New forks do not use the current head digest as their identity. Mallard
generates a new validated ID once and persists it in the local journal before
Push or Pull publication/materialization.

### Cutover operation

1. Load or create `host_identity.json` and verify access to the protected
   origin key.
2. Add the current `host_id` to existing project bindings while retaining
   their established `replica_id` values.
3. Read and validate the reviewed schema-4 head and manifest.
4. Derive group and initial variant IDs.
5. Convert each conversation resource and its files to the schema-5 model.
6. Extract bounded source-thread provenance and codec metadata.
7. Leave historical variant creator/editor origins unknown; record the current
   scoped origin only on the schema-5 migration commit.
8. Bind an exact local match to the migrated variant.
9. If local content already diverged, allocate and journal a separate local
   variant rather than overwriting either side.
10. Reuse immutable payload objects where valid; publish a new immutable
   schema-5 manifest, commit, tag, and head using CAS.
11. Record the local binding and migration receipt.
12. Treat schema-4 manifests as immutable history after cutover.

A schema-4 application must reject a schema-5 head and therefore cannot become
a mixed writer. The cutover review must explain that older Mallard versions
will no longer update that bundle.

## Provider codec prerequisite

Do not begin schema implementation with an assumption that every current Codex
or Claude record exposes stable ancestry. Phase 0 must prove the capability
against sanitized real-format fixtures for every advertised provider version.

A `ConversationCodec` must:

1. discover the complete selected file set using bounded no-follow reads;
2. identify the authoritative provider thread ID and project `cwd`;
3. stream the bounded transcript and extract one unambiguous logical chain;
4. validate message-ID uniqueness and parent continuity;
5. compute lineage and semantic digests;
6. classify every provider file by role;
7. enumerate exactly which structural fields may be rewritten;
8. transform source thread ID and `cwd` to destination values;
9. rewrite or generate provider index/sidecar state consistently;
10. parse the staged result and prove it resumes as the proposed destination
    ID without changing semantic content; and
11. report a versioned capability key through `codec_versions`.

The full streaming scan remains bounded by the existing per-file limits plus
explicit record-count and line-length limits. A tail-only scan is insufficient
to calculate message count, full lineage, and common-prefix integrity.

If stable ancestry or safe rewriting cannot be proved for a provider version,
capture marks lineage `Unknown` and Pull offers `StoreOnly`. It must not claim
the alternate is resumable.

### Codex requirements

The verified file set includes at least:

- rollout filename and destination directory;
- authoritative `session_meta` thread ID;
- structural `cwd` in `session_meta` and `turn_context`;
- project-filtered session index/title row;
- any discovered Codex state keyed by thread ID; and
- subagent metadata rules, without confusing parent session ID with the
  transcript's own ID.

### Claude requirements

The verified file set includes at least:

- transcript filename;
- authoritative session ID fields;
- selected session-keyed sidecars;
- project-path fields; and
- any index or history state required for `claude --resume`.

Codex and Claude capabilities are enabled independently. Supporting Codex does
not imply Claude alternates are resumable.

## Provider ID allocation

For every unbound variant selected for materialization:

1. Load and validate the provider profile and its revision.
2. Scan all provider-ID owners across that profile, not just the current
   project.
3. Prefer the valid source provenance ID only if every target is absent.
4. If occupied, allocate a new provider-compatible UUID.
5. Reserve the ID and complete target set in the reviewed plan and journal.
6. Recheck ID ownership, target absence/digests, profile revision, and provider
   capability immediately before commit.
7. Publish the binding as `Resumable` only after every required provider file
   and index entry validates.
8. Reuse the binding and receipt on repeat Pull.

On a fresh replica, allocation order is deterministic within the reviewed
plan: cloud primary first, then remaining variant IDs in sorted order. Once
saved, plan reservations—not recalculation order—control retries.

The existing provider thread always wins a collision. Pull never renames or
rewrites it merely to make room for an incoming alternate.

## Crash-consistent provider materialization

A filesystem cannot atomically commit unrelated transcript, index, sidecar,
receipt, and JSON state files as one rename. The implementation therefore
promises crash consistency through a journal, not impossible cross-file
atomicity.

### Apply protocol

1. Validate the plan, cloud snapshot, bindings, profile, provider capability,
   local digests, target occupancy, and provider quiescence.
2. Write and fsync a `Prepared` journal record.
3. Stage every transformed provider file under bounded safe staging paths.
4. Parse staged transcripts and verify destination ID, `cwd`, lineage, and
   semantic digest.
5. Create immutable recoverable backups for every reviewed replacement.
6. Fsync the staged set and advance the journal to `Staged`.
7. Commit provider targets with no-follow and no-clobber semantics where
   targets were expected absent.
8. Update index/sidecar state using the provider adapter's verified safe-write
   protocol.
9. Advance the journal to `ProviderCommitted`.
10. Atomically save binding, same-ID rekey, and preference updates.
11. Save the receipt and advance to `LocalCommitted`.
12. Remove staging and retire the journal record only after a final verification.

A process failure at any step leaves enough metadata to finish or restore on
startup. Until `ProviderCommitted` and local binding verification both succeed,
the UI does not advertise the new variant as resumable.

### Provider-writer concurrency

Apply-time digest checks alone do not close the race with a running provider.
For any existing-thread fast-forward, replacement, or shared-index update, the
adapter must provide one of:

- a verified provider lock/API;
- a reliable quiescence check plus immediate revalidation; or
- a requirement that the user close the provider before Apply.

If none is available, Mallard defers or stores the variant only. A concurrent
append always wins over Pull; it is never overwritten.

## Pull behavior

Pull review is grouped by conflict group and clearly separates:

- cloud primary;
- local preferred branch;
- existing local provider thread;
- incoming alternates; and
- resumable, pending, or stored-only state.

Actions:

- `NoOp` for equal lineage and semantics.
- `FastForwardLocal` when cloud exactly extends the bound local variant and the
  provider can be updated safely.
- `KeepLocal` when local exactly extends cloud.
- `AddAlternate` for a missing or divergent cloud branch.
- `StoreOnly` when lineage or provider rewriting is unsupported.
- `PromoteLocal` only after explicit selection.
- `ReplaceLocal` only with explicit approval, pinned digests, provider
  quiescence, and recoverable backups.
- `TombstoneVariant` or `TombstoneGroup` only after explicit review.

For same-ID divergence, Pull first journals and rekeys the existing local
branch, including the preference pointer when necessary, then materializes the
cloud branch under a separate provider ID.

On a fresh replica, Pull may materialize every reviewed variant. It initializes
local preference from the cloud primary only if that variant becomes resumable.
It never launches or activates a provider thread.

## Push behavior

Capture maps each provider thread through `conversation_bindings.json` before
comparing it with cloud variants.

- Exact match is a no-op.
- Local exact extension fast-forwards only its bound `variant_id`.
- Cloud exact extension blocks Push and requires Pull.
- Same-ID divergence rekeys the existing local branch to a durable new variant
  before publication.
- A bound alternate updates its existing variant; it does not create another.
- A local-only variant is added.
- Remote-only and unselected variants are retained.
- Absence is not deletion.
- Cloud primary is unchanged unless an explicit reviewed Promote action is in
  the same CAS publication.

When Push creates a variant, it records `created_by_origin` and
`last_updated_by_origin` using the current bundle-scoped tokens. A semantic
fast-forward changes only `last_updated_by_origin`. A no-op, schema migration,
local ID rewrite, or `cwd` remap does not claim a semantic edit. The manifest
and commit record the publishing origin independently.

If a binding is missing, automatic recovery is allowed only when complete
lineage and semantic comparison identifies exactly one compatible cloud
variant. A local transcript that strictly extends exactly one longest cloud
ancestor may bind to that variant. Equal candidates, title matches,
provider-ID matches without content proof, and unknown lineage block recovery.

### Push recovery order

1. Persist any new variant ID and pending publication in the journal; an
   already resumable local binding remains `Resumable`.
2. Capture and verify the exact local provider file set.
3. Build the next manifest by retaining reviewed cloud variants.
4. Publish immutable objects, manifest, and commit.
5. CAS the cloud head.
6. Record the published revision/commit in the journal.
7. Finalize the local binding and receipt.

A failed CAS leaves a stable local-only variant for review and retry. A
successful CAS followed by local persistence failure is recovered by matching
the journaled commit and manifest digest; it must not create a duplicate branch.

## User experience

Mallard presents one logical conversation with a version list:

~~~text
Conversation title
  On this machine: V2 · Continue
  Other version: V1 · Open / Continue
  Cloud primary: V1
~~~

Normal UI emphasizes title, time, preview, optional sanitized origin label, and
plain-language badges. Internal group/variant/provider IDs are available in
details for diagnosis but are not the primary label.

Host provenance is displayed without exposing the real hostname:

~~~text
V2 · This host · Continue
V1 · Host A · Imported
Last updated by Host B
~~~

`This host` is determined by locally deriving the bundle-scoped host token.
Other names come from `OriginHostAlias`, with a short token-derived fallback.
The UI must describe origin as “created by,” “last updated by,” or “published
by”; it must not imply ownership, trust, or authorization.

Additional states are explicit:

~~~text
Stored only · Provider format cannot be rewritten safely
Pending · Finish recovery before continuing
Close Codex to apply this fast-forward safely
Cloud changed after review · Refresh required
~~~

An internal same-ID rekey is not presented as a promotion. The user remains on
the same transcript and provider thread.

## Implementation sequence

### Phase 0 — prove provider capabilities

1. Collect sanitized Codex and Claude fixtures for every supported format.
2. Implement read-only codec prototypes for ancestry, semantic
   canonicalization, file-set inventory, ID rewrite, and round-trip validation.
3. Decide capability keys and store-only fallbacks.
4. Do not change portable schema until at least one provider passes the full
   continuation proof.

### Phase 1 — pure domain and state machine

1. Add validated `HostId`, `OriginHostToken`, `OriginReplicaToken`,
   `ConflictGroupId`, `VariantId`, `MessageId`, digest, relation, and file-role
   types.
2. Implement exact, prefix, divergence, unknown, missing, and tombstone
   classification as pure functions.
3. Implement the same-ID divergence transition, including preferred-branch
   rekey semantics.
4. Add deterministic schema-cutover ID golden tests.

### Phase 2 — portable and local metadata

1. Add `host_identity.json`, protected origin-key access, host rotation, and
   bundle-scoped origin-token derivation.
2. Add schema-5 portable-origin/group/variant/file-role metadata and strict manifest
   validation.
3. Update recipe reconciliation to retain remote variants unless explicitly
   tombstoned.
4. Add `conversation_bindings.json`, local origin aliases, and
   `conversation_sync_journal.json` with bounded atomic persistence.
5. Add typed host-pinned conversation plan and receipt metadata.
6. Add a read-only schema-4 cutover decoder; do not add dual writers.

### Phase 3 — Codex end-to-end path

1. Populate lineage and semantic summaries during capture.
2. Add provider-profile-wide ID reservation.
3. Add crash-consistent clone-with-new-ID and fast-forward materialization.
4. Add reviewed Pull actions and startup recovery.
5. Add reviewed Push variant reconciliation and post-publication recovery.
6. Enable Codex `Resumable` only after all codec and failure tests pass.

### Phase 4 — contracts and UI

1. Update Rust DTOs, `src/types.ts`, and `projectSyncApi` together.
2. Group History and review rows by conflict group.
3. Resolve “This host,” local host aliases, creator, last-editor, and publisher
   provenance without exposing raw host IDs.
4. Add primary, preferred, resumable, stored-only, pending, and stale-plan
   states.
5. Add explicit local promotion, cloud promotion, replacement, retry, and
   tombstone flows.

### Phase 5 — Claude and rollout

1. Repeat the complete provider capability and crash-safety work for Claude.
2. Gate each provider/codec version independently.
3. Run schema cutover and A/cloud/B end-to-end suites for local-folder and
   S3/R2 storage.
4. Enable the feature only after old-client rejection and recovery behavior are
   verified.

## Required tests

### A/cloud/B continuation

1. Create base `[a, b]` and variants `[a, b, c1]` and `[a, b, c2]` with stable
   message IDs, parents, and semantic fingerprints.
2. Bind A's existing provider ID to V1 and B's existing provider ID to V2.
3. Push A and assert cloud retains both variants and its primary.
4. Pull B and assert B's V2 bytes, provider ID, preference, and resume command
   remain unchanged; V1 receives a distinct local ID.
5. Append to B's V2, Push, and assert only V2 advances.
6. Pull A and assert A's V1 provider ID and preference remain unchanged; V2
   receives a distinct local ID.
7. Append to A's V1, Push, and assert only V1 advances.
8. Assert A and B have different local host IDs and bundle-scoped origin
   tokens; each UI labels its own token “This host” and resolves the other
   through a local alias.

### Same-ID first divergence

- Start with cloud and local both bound as V but with divergent content.
- Pull and assert cloud retains V, local content becomes V-local, the existing
  provider ID is unchanged, and preference follows V-local.
- Assert incoming V receives another provider ID.
- Repeat Pull and assert no new IDs, files, bindings, or preference changes.
- Repeat the equivalent Push transition and assert retry uses the same durable
  V-local ID.

### Classification and digest safety

- Test identical, both prefix directions, divergence, unknown, and missing.
- Same rendered text with different message IDs is divergence.
- Same IDs with changed semantic fingerprints is corruption/unknown, not equal.
- Source and destination raw SHA values may differ only through verified
  structural rewrites while semantic digest remains equal.
- Prefix comparison verifies all common message fingerprints.
- Oversized, malformed, cyclic, duplicate-ID, and branched transcripts return
  unavailable or unknown, never a guessed relation.

### Manifest and metadata validation

- Reject missing/duplicate transcript roles and orphan resources/files.
- Reject invalid primary pointers, parent cycles, provider mismatches, live
  tombstone collisions, and recipe/resource disagreement.
- Assert pushing one selected variant carries every unchanged remote variant
  forward.
- Assert serialized portable metadata contains no local path, profile ID,
  preference, binding, plan, receipt, backup, journal, raw host/replica ID,
  hostname, hardware identifier, origin key, or local provider ID used as
  identity.
- Reject duplicate provider-ID ownership anywhere in one provider profile.
- Reject stale document, binding, preference, and provider-profile revisions.

### Host and replica distinction

- Two fresh installations generate different `host_id` and origin keys.
- One host derives a stable `origin_host_token` for repeated operations on the
  same bundle.
- The same host derives unlinkable tokens for two different bundle IDs.
- Two replicas on one host share the host token but have distinct replica
  tokens.
- A raw host ID, replica ID, hostname, username, MAC address, hardware serial,
  or origin key never appears in a manifest, commit, tag, object key, DTO log,
  or activity record.
- Current-host resolution uses the derived token; remote aliases remain local.
- Rotating host identity invalidates old local plans, generates new replica
  IDs, and preserves provider thread IDs, group IDs, variant IDs, and content.
- A copied state file without its protected origin key requires
  “Register as new host.”
- A byte-for-byte clone including the key is documented as undetectable; the
  explicit rotation flow separates it safely.
- A spoofed origin token never changes classification, approval, replacement,
  or CAS decisions.

### ID allocation and idempotence

- Pull two variants sharing one source provider ID onto a fresh machine and
  assert two unique destination IDs.
- Occupy the source provenance ID with unrelated content and assert the
  incoming version receives a new ID.
- Repeat Pull and assert no duplicate transcript, sidecar, index row, receipt,
  journal record, or binding.
- Remove a binding and create ambiguous compatible cloud variants; recovery
  must block.

### Crash and concurrency recovery

- Terminate after every journal phase and assert restart completes or restores
  a consistent state.
- Simulate cloud CAS success followed by local binding-save failure and assert
  recovery finds the published variant rather than creating another.
- Simulate CAS failure after local rekey and assert the stable local-only
  variant can be retried.
- Append to a transcript after review and before apply; apply must fail safely.
- Race provider index updates and assert Mallard neither loses provider rows
  nor marks an incomplete variant resumable.
- Attempt fast-forward while provider quiescence cannot be proved and assert
  defer/store-only behavior.
- Inject failures in transcript, sidecar, index, binding, receipt, and cleanup
  writes and verify recoverability.

### Primary, preference, and active-thread safety

- Pull cloud-primary V1 while A prefers V2; A stays on V2.
- Pull cloud-primary V2 while B prefers V1; B stays on V1.
- Same-ID rekey updates the internal preferred ID but preserves the same
  provider thread and transcript.
- Only explicit local Promote changes the preferred branch.
- Only explicit cloud Promote changes cloud primary, with CAS.
- No Pull launches, focuses, or resumes a provider application.

### Tombstones

- Missing without tombstone never deletes.
- Explicit unchanged-variant deletion requires review and backup.
- Delete/modify races retain the modified content as a live variant.
- Tombstoning a primary requires a replacement primary in the same CAS.
- Deleting the final variant uses a group tombstone.
- Repeated application is idempotent.

### Schema cutover and compatibility

- Two replicas independently derive identical group and initial variant IDs
  from one schema-4 manifest.
- A locally diverged schema-4 transcript becomes a separate stable variant.
- Migrated historical variants do not falsely attribute their creator or last
  semantic editor to the cutover host.
- Immutable schema-4 history remains readable.
- Schema-4 writers reject a schema-5 head.
- No code path writes schema 4 after cutover.
- Local-folder and S3/R2 publication preserve immutable objects and head CAS.

### Provider continuation proof

For every provider/codec version advertised as resumable:

- materialize under a new destination provider ID;
- verify transcript, filename, metadata, index, sidecars, and `cwd` agree;
- verify message IDs, parentage, semantic content, timestamps, and tool records
  are preserved;
- invoke or simulate the provider's resume lookup against a temporary provider
  home; and
- append a new message, capture it, and prove the next Push advances the same
  `variant_id`.

## Completion criteria

The feature is complete only when:

- the A/cloud/B scenario preserves each person's branch and provider thread;
- same-ID divergence is rekeyed without a user-visible switch;
- multiple variants sharing one original provider ID coexist in one bundle;
- every resumable variant has a unique provider ID within each local profile;
- different Mallard hosts and replicas have distinct local identities and
  privacy-scoped portable origin tokens;
- hostnames, hardware identifiers, raw host/replica IDs, and origin keys never
  enter portable state;
- copied-host registration and explicit host-identity rotation preserve
  conversation and provider-thread continuity;
- repeat Pull and Push are idempotent;
- later Pushes update the correct variant regardless of local provider ID;
- cloud primary, local preferred branch, and active provider conversation
  remain independent;
- raw, lineage, and semantic digests have distinct validated meanings;
- remote-only variants survive unrelated Pushes;
- explicit replacement and deletion have pinned targets and recoverable
  backups;
- provider-visible writes are quiescent or use a verified concurrency protocol;
- crashes at every journal phase recover without duplicate or half-advertised
  conversations;
- schema-5 metadata excludes machine-local state and passes strict referential
  validation;
- schema-4 cutover is deterministic, reviewed, and one-way; and
- every provider version marked resumable passes the full continuation proof.
