# Mallard Current Metadata Layout

**Code snapshot:** August 11, 2026 (portable-tail conversation resolver)<br>
**Application version:** 0.1.2<br>
**Scope:** the active project-sync and scope-rowed Library implementations in
this repository.

This document describes what Mallard stores today, where it stores it, which
identities connect the local and portable sides, and which data never crosses
the machine boundary. It is derived from the current Rust implementation, not
from historical `PLAN_*` documents.

## 1. Executive summary

Mallard has three separate data domains:

1. **Machine-private control state** under `~/.mallard`. This contains
   credentials, absolute paths, local identities, review plans, operation
   journals, receipts, backups, caches, and logs.
2. **Provider and project data** under the selected project root,
   `CODEX_HOME`, and `CLAUDE_CONFIG_DIR`. Mallard reads selected content from
   these locations and materializes approved Pull results back into them.
3. **Portable storage** in either a local folder or an S3-compatible bucket.
   Project bundles use `.mallard/v1/repositories/<bundle-id>/`; independent
   Library resources use `.mallard/v1/library/<resource-repository-id>/`.

The portable anchor is `bundle_id` for a project or `resource_repository_id`
for one Library resource. Local project, storage, host, replica, profile,
registration, and link IDs do not appear in portable documents. A mutable
head points to an immutable manifest, commit record, and content-addressed
payload objects.

All active local and portable schemas are version 1. Compatibility is a clean
cutover: there is no backward-compatibility support, migration, legacy
execution path, or retained dead code for the removed storage-wide catalog.

## 2. Boundary map

```text
Machine-private control plane
~/.mallard/
  local IDs, absolute paths, credentials, plans, journals, receipts,
  backups, caches, logs

      maps by bundle_id and reviewed revision/commit/manifest-size/hash
                              |
                              v

Portable storage
<local storage folder or S3/R2 bucket>/.mallard/v1/
  repositories/<bundle-id>/       project bundle snapshots
  library/<resource-repository-id>/ one typed Library resource

                              ^
                              |
              capture from / materialize into

Provider and project locations
<project root>/
<CODEX_HOME>/
<CLAUDE_CONFIG_DIR>/
```

The two `.mallard` names are unrelated roots:

- `~/.mallard` is private application state.
- `<storage>/.mallard` is the portable object-store namespace.
- A project directory named `.mallard` is hard-excluded from ordinary
  project-content capture.

## 3. Machine-local layout

Not every path exists on first launch. Documents and directories are created
on demand.

```text
~/.mallard/
|-- persistence.lock
|-- host_operation.lock
|-- host_identity.json
|-- sync_config.json
|-- machine_projects.json
|-- conversation_bindings.json
|-- conversation_sync_journal.json
|-- materializations.json
|-- activity_cache.json
|-- catalog_resource_profiles.json
|-- catalog_scopes.json
|-- catalog_resources.json
|-- catalog_resource_materializations.json
|-- catalog_resource_preservations.json
|-- catalog_resource_operations.json
|-- host_rotation.json                         # transient recovery journal
|
|-- project_drafts/
|   `-- <draft-id>.json
|-- setup_transactions/
|   `-- <draft-id>.json
|-- restore_plans/
|   `-- <plan-id>.json
|-- conversation_push_plans/
|   `-- <plan-id>.json
|-- catalog_resource_plans/
|   `-- <plan-id>.json
|
|-- conversation_operations/
|   `-- <operation-id>/
|       `-- <target-index>.payload
|-- backups/
|   |-- <plan-id>/...
|   |-- catalog_resources/
|   |   `-- <plan-id>/...
|   |-- conversations/
|   |   `-- <operation-id>/<target-index>.bak
|   `-- pathrepair-<id>/...
|
|-- locks/
|   |-- catalog-state.lock
|   `-- provider-<canonical-path-sha256>.lock
|-- catalog_quarantine/
|   `-- <document>-<timestamp>.quarantined
|
|-- host_recovery_quarantine/
|   `-- <category>-<generated-id>.quarantined
|
`-- logs/
    |-- _policy.json
    |-- _activity.lock
    `-- activity-YYYY-MM-DD-NNN.jsonl
```

### 3.1 Primary local documents

| Path | Purpose | Important contents | Portable? |
| --- | --- | --- | --- |
| `host_identity.json` | Installation identity | Schema/revision, private 128-bit `host_id`, optional local display name, timestamps | No |
| `sync_config.json` | User configuration and project/storage catalog | Storage definitions, S3 credentials, local projects, storage links, recipes, project-content preferences, per-storage reviewed bases, per-link conversation checkpoints (accepted portable tail + witness, observed tail, physical mapping cursor, prefix witness) | No |
| `machine_projects.json` | Machine path and provider-profile catalog | Exact and canonical checkout paths, exact and canonical provider-home paths, host/replica IDs, project/profile bindings, binding revisions and state | No |
| `conversation_bindings.json` | Local-to-portable conversation identity map | Mallard thread ID, provider thread ID, project/replica/profile scope, materialization state, immutable root tail layout (codec, partition kind, fixed start), `forked_from` provenance hint, keep-both `preserved` provenance (`preserved_from` origin + per-origin ordinal) | No |
| `conversation_sync_journal.json` | Crash-recoverable conversation transactions | In-flight Push publications (CandidateUploaded, HeadAdvanced, LocalCommitted, terminal phases) and Pull materializations (TargetStaged, ProviderCommitted, LocalCommitted, terminal phases), each embedding the exact whole-document `SyncConfig` and `ConversationBindings` before/after states that recovery installs verbatim | No |
| `materializations.json` | Durable Pull history | Plan and bundle coordinates, status, action receipts, absolute targets, backup paths, conversation receipts, errors | No |
| `activity_cache.json` | Local session-history acceleration for every bound provider | File size/mtime keys, provider key, parsed ownership, cwd, Git/session metrics, and identities; cached title, summary, and chat text are deliberately omitted | No |
| `catalog_resource_profiles.json` | Machine-local Codex/Claude Library profiles | Provider key, display name, configured `CODEX_HOME`/`CLAUDE_CONFIG_DIR` path, revisions | No |
| `catalog_scopes.json` | User-added Catalog menu rows | One row per (profile × resource kind); membership over registrations is derived, never stored | No |
| `catalog_resources.json` | Resource registrations and independent storage links | Provider/profile identity, typed local target, remote resource-repository ID, accepted head/content base, revisions | No |
| `catalog_resource_plans/<plan-id>.json` | Immutable per-resource Push/Pull/Fork reviews | Digest-pinned action, preservation-copy pins, profile/root/link revisions, expiry | No |
| `catalog_resource_materializations.json` | Verified per-target Library receipts | Cross-storage claim edges keyed by canonical target identity with content digests and outcomes | No |
| `catalog_resource_preservations.json` | Named-copy preservation witnesses | Source/preserved targets, digests, and operation IDs used for idempotent recovery | No |
| `catalog_resource_operations.json` | Library mutation journal | In-flight and recent directory/preservation/fork/plugin operations with phases, paths, and terminal outcomes | No |

`sync_config.json` currently stores S3/R2 access-key material directly in its
private JSON document; it is not stored in the OS keychain. It therefore
belongs to the private machine boundary even when the configured storage is
shared.

### 3.2 Project, storage, and provider records

The main local relationship is:

```text
LocalProjectRegistration
  local_project_id
  bundle_id ----------------------------------------+
  default recipe                                    |
  recipe_bases[storage_id]                          |
                                                     |
ProjectStorageLink                                   |
  local_project_id + storage_id + bundle_id          |
  destination recipe                                 |
  local project-content exclusions                   |
                                                     v
ProjectBinding                              portable repository
  host_id + replica_id                     .../<bundle_id>/
  local_project_id + bundle_id
  absolute project root
  provider -> profile_id
          |
          v
ProviderProfile
  provider + local profile_id
  absolute provider-home path
```

The records divide responsibility as follows:

- `LocalProjectRegistration` owns the local project identity, portable
  `bundle_id`, display name/local alias, repository fingerprint, default
  recipe, and one `RecipeBase` per storage.
- `ProjectStorageLink` owns destination-specific selection and local
  project-content exclusions. Those exclusions are never pushed.
- `ProjectBinding` maps the project to one machine checkout, one replica, and
  selected provider profile IDs.
- `ProviderProfile` owns the selected `CODEX_HOME` or
  `CLAUDE_CONFIG_DIR` spelling and its canonical path.
- A `RecipeBase` pins the last reviewed remote revision, commit ID, required
  nonzero serialized manifest size, manifest SHA-256, recipe revision, binding
  revision, and Push/Pull times. Historical manifest reads use the stored size
  and hash as exact transfer expectations.

### 3.3 Conversation identity

Mallard maintains two conversation identities:

- `thread_id` is Mallard's logical, portable identity.
- `provider_thread_id` is the provider-native identity, such as a Codex UUID,
  and is bound to a local provider profile.

One provider thread ID may have only one owner for a given host/profile.
`conversation_bindings.json` also records:

- whether the provider copy is `pending`, `resumable`, or `stored_only`;
- the immutable root tail layout — codec, partition kind
  (`whole_thread` or `fixed_paginated_tail`), and the fixed start record
  count — seeded at first full parse or imported from the verified cloud
  value at Pull install; and
- `forked_from` as a provenance hint that is surfaced but never used for
  matching or classification; and
- `preserved` as the keep-both provenance hint (`preserved_from` origin
  thread + per-origin ordinal), published with the thread on Push and
  restored from the cloud record on other machines' pulls — display-only,
  like `forked_from`.

Acceptance state does not live on the binding. Each `ProjectStorageLink` in
`sync_config.json` holds per-provider-scope conversation checkpoints: the
accepted five-field portable tail with its commit/manifest witness, the
replaceable observed tail, the physical mapping cursor with a raw-range pin
and trailing probe window, the machine-local prefix witness over the fixed
prefix, and generation/binding-revision fences. Removing a project removes
its links and checkpoints; bindings and layouts survive, and a re-registered
project re-proves acceptance by refolding the reviewed cloud tail against
the local chain before it may push again.

The portable manifest has its own conversation entry keyed by Mallard
`thread_id`. It includes `source_thread_id`, so the provider-native source ID
is portable metadata even though the local profile binding and absolute
transcript path are not.

### 3.4 Review plans and transaction journals

Plans are immutable approval surfaces. Reusing an existing plan ID is an
error.

| Directory | Document | What it pins |
| --- | --- | --- |
| `restore_plans/` | `RestorePlan` | Host/replica, storage/bundle, revision, commit, manifest hash, binding revision, expiry, absolute targets, expected target state, selected file/conversation actions |
| `conversation_push_plans/` | `ConversationPushPlan` | Host/replica/project/profile revisions, current or absent head coordinates, conversation relations/actions, target expectations, lineage/semantic pins, expiry |
| `catalog_resource_plans/` | `CatalogResourcePlanDocument` | Exactly one Library registration/link or Fork source, provider/root/storage/head revisions, local and remote digests, cross-storage claim pins, exact action, and expiry |

Restore and Library resource plans normally expire after 24 hours. Completed
project results live in `materializations.json`; Library outcomes and recovery
state live in the claim, preservation, and operation documents. Plans are not
themselves portable history.

Setup has two local layers:

- `project_drafts/<draft-id>.json` is resumable UI state with preallocated
  project/bundle IDs, selected paths, pending or existing profiles/storage,
  repository choice, selected resource IDs, and a discovery signature. It
  never stores discovered payload bytes or remote listings.
- `setup_transactions/<draft-id>.json` is the deterministic finalize intent
  written before mutating the permanent config and binding documents. It
  makes interrupted finalization idempotently recoverable.

The Library needs neither layer: adding a Catalog row records one scope in
`catalog_scopes.json`, and linking a resource lands its registration and
storage link in `catalog_resources.json` in a single locked mutation.

### 3.5 Conversation operation payloads and backups

An in-flight provider materialization may use three physical locations:

1. `conversation_sync_journal.json` stores the transaction state and absolute
   target/staging/backup paths.
2. `conversation_operations/<operation-id>/<index>.payload` stores durable
   private intent bytes so a `Prepared` operation can resume before a
   provider-side stage exists.
3. `.<target-name>.mallard-<operation-fragment>-<index>.stage` is created next
   to the provider target so the final rename stays on the same filesystem.

If an existing provider target is replaced, its backup is stored at
`backups/conversations/<operation-id>/<index>.bak`. Successful cleanup removes
the intent directory and stage files and retires the journal operation.
Backups needed by durable receipts remain local.

Normal restore backups are grouped under `backups/<plan-id>/`. Library
skill/pet replacement backups use
`backups/catalog_resources/<plan-id>/`; their operation metadata stays in
`catalog_resource_operations.json`. Codex path-repair operations use a
generated `pathrepair-*` directory.

### 3.6 Divergence and keep-both metadata

Relations come from one shared classifier over three portable tails — the
accepted pin `P`, the derived local value `L`, and the verified cloud value
`C` — plus verification gates. A pinned pair with `L != P` and `C != P` is
`Divergent`; an unpinned pair that cannot be proven is `Unknown` and blocks
both directions. For:

```text
local t = (m1, m2, m3)
cloud t = (m1, m2, m4)
```

Push records a reviewed `require_pull` action and does not mutate cloud
storage. Pull's default divergence resolution is keep-both, planned as one
materialization action with:

- a pre-minted preserved Mallard thread ID `t'` and a fresh provider thread
  ID for the re-materialized tracked thread, both persisted in the plan;
- the minted preservation provenance `PreservedProvenance { preserved_from:
  t, ordinal }` (ordinal = max+1 over every provenance record naming `t`
  across the pinned cloud snapshot and the local bindings) and the composed
  marker display title `"{base} (Preserved {N} · {device label})"`;
- pinned accepted/local/cloud tails, raw-range hashes, and the immutable
  commit/manifest coordinates; and
- exact target paths with expected before-states.

Apply never invokes Codex and never edits the preserved rollout: `t'` keeps
the local bytes and the unchanged local layout under a re-keyed checkpoint
with the accepted pin cleared, while `t` re-materializes the verified cloud
graph under the fresh alias with the remote layout imported from the cloud
value. The same transaction rewrites the provider session index: the
preserved copy's existing row is renamed to the plan-minted marker title,
and the destination row lands beside it when cloud carries a real title.
The journal operation embeds the exact whole-document
`ConversationBindings` and `SyncConfig` before/after states; recovery rolls
the remaining document forward or terminalizes (`aborted_precommit`,
`aborted_provider_collision`) without ever clobbering a foreign target.

The completed local identity is:

```text
t  -> cloud conversation, re-materialized under a fresh provider alias
t' -> preserved local branch, preserved = { preserved_from: t, ordinal }
```

Pull does not publish `t'`. Its first reviewed Push is CreateCloud and
carries the binding's `preserved` provenance and the marker title into the
portable thread record; a later Pull of `t'` elsewhere restores the same
provenance into the receiving binding. Until that Push, cloud still
contains only `t`.

### 3.7 Locks, atomicity, permissions, and recovery

`persistence.lock` is taken for bounded local-document reads and
read-modify-write operations. It combines an in-process mutex with an OS file
lock, so separate Mallard processes sharing the same `~/.mallard` directory
cannot silently overwrite one another.

`host_operation.lock` is a broader, non-blocking OS lock held across
host-sensitive multi-document workflows such as setup, host rotation,
publication, restore apply, and conversation materialization. It
prevents two individually valid operations from interleaving across config,
provider files, and cloud state. The guard explicitly unlocks on drop.

The activity log has its own `_activity.lock`. A local-folder bundle store has
a separate `.mallard/.storage.lock`; that lock protects the portable storage
folder, not `~/.mallard`. Library documents use `locks/catalog-state.lock`,
and each filesystem or plugin mutation holds the canonical provider-target
lock `locks/provider-<canonical-path-sha256>.lock`.

Local JSON persistence uses:

- bounded reads and writes;
- exact schema validation;
- revision checks on mutable documents;
- no-follow and real-directory checks;
- synced temporary files followed by atomic replacement; and
- Unix mode `0700` for private directories and `0600` for private files.

If `host_identity.json` is missing or corrupt, Mallard registers a fresh host.
`host_rotation.json` is the transient schema-1 journal used to rebase or
detach surviving host-scoped records. Damaged artifacts are preserved under
`host_recovery_quarantine/` without a `.json` suffix so normal scanners cannot
reactivate them.

### 3.8 Local persistence limits

| Document or artifact | Maximum |
| --- | ---: |
| `host_identity.json` | 16 KiB |
| `sync_config.json` | 8 MiB |
| `machine_projects.json` | 8 MiB |
| `project_drafts/<id>.json` | 4 MiB |
| `setup_transactions/<id>.json` | 8 MiB |
| `restore_plans/<id>.json` | 64 MiB |
| `conversation_push_plans/<id>.json` | 64 MiB |
| `conversation_bindings.json` | 64 MiB |
| `conversation_sync_journal.json` | 64 MiB |
| `materializations.json` | 64 MiB |
| `host_rotation.json` | 192 MiB |
| `activity_cache.json` | 32 MiB / 50,000 entries |
| `catalog_resource_profiles.json` | 1 MiB |
| `catalog_scopes.json` | 1 MiB |
| `catalog_resources.json` | 16 MiB |
| `catalog_resource_plans/<id>.json` | 64 MiB |
| `catalog_resource_materializations.json` | 16 MiB |
| `catalog_resource_preservations.json` | 8 MiB |
| `catalog_resource_operations.json` | 16 MiB |
| One activity entry | 16 KiB |
| One activity segment | 10 MiB |
| Default activity retention | 30 days / 100 MiB total |

## 4. Provider and project locations

Provider homes are not Mallard metadata directories. They are source and
materialization targets owned by Codex or Claude.

### Codex

Mallard scans project-owned JSONL transcripts under:

```text
<CODEX_HOME>/sessions/**/*.jsonl
<CODEX_HOME>/archived_sessions/**/*.jsonl
```

`<CODEX_HOME>/session_index.jsonl` is read locally for titles. Pull appends
at most one provider-schema row (`{id, thread_name, updated_at}`)
transactionally, and only when the cloud thread carries a real title; a
titleless thread writes no row and becomes visible when Codex rebuilds its
own index from the materialized rollout. The raw file is not a portable
resource.
Codex internal subagent rollouts are currently excluded from sync.

### Claude

The current scanner recognizes project-owned conversations and related state
under:

```text
<CLAUDE_CONFIG_DIR>/projects/.../*.jsonl
<CLAUDE_CONFIG_DIR>/file-history/...
<CLAUDE_CONFIG_DIR>/todos/...
<CLAUDE_CONFIG_DIR>/projects/.../memory/...
```

User-global agent customizations travel through the scope-rowed Library:
standalone Codex and Claude Agent Skills from `<CODEX_HOME>/skills`,
`$HOME/.agents/skills` (resolved only for the default `~/.codex` config), and
`<CLAUDE_CONFIG_DIR>/skills`, portable plugin install intent, and distinct
Codex v1/v2 pets from `<CODEX_HOME>/pets`. The Library is independent of every
project and repository bundle; project capture never enumerates those roots.

### Project root

Ordinary project-content scanning is explicit and available only when the
project is not Git-managed. Portable paths use `project/<relative-path>`.
Mallard honors `.gitignore`, `.ignore`, and `.mallardignore`, and hard-excludes
VCS directories, build/cache directories, `.mallard`, provider-sync metadata,
symlinks, special files, and known unsafe content.

## 5. Portable storage layout

The local-folder and S3/R2 adapters expose the same object keys:

```text
<storage root or bucket>/
`-- .mallard/
    |-- .storage.lock                         # local-folder backend only
    |-- _storage.json
    `-- v1/
        |-- repositories/
        |   `-- <bundle-id>/
        |       |-- _head.json
        |       |-- _tag.json
        |       |-- _objects/
        |       |   `-- <payload-sha256>
        |       |-- _manifests/
        |       |   `-- <revision>-<commit-id>.json
        |       |-- _commits/
        |       |   `-- <revision>-<commit-id>.json
        |       `-- _uploads/                 # reserved for project bundles
        `-- library/
            `-- <resource-repository-id>/
                |-- _tag.json                 # immutable, create-only discovery
                |-- _head.json                # authoritative CAS pointer
                |-- _objects/
                |   `-- <payload-sha256>
                |-- _manifests/
                |   `-- <revision>-<commit-id>.json
                `-- _commits/
                    `-- <revision>-<commit-id>.json
```

The `repositories/` and `library/` constructors are closed and mutually
isolated. The removed `.mallard/v1/catalog/` namespace is rejected by the
object-key validator and is never read, listed, converted, deleted, or used as
a fallback. No migration or compatibility detector exists.

Both publishers write payloads to `_objects/<sha256>`. `_uploads/` remains a
reserved project-bundle namespace, but no production publication path writes
that older upload-tree shape. Library resources never use `_uploads/`.

### 5.1 Storage marker

`.mallard/_storage.json` is immutable and contains:

```json
{"format":"mallard-storage","layout_version":1}
```

Opening either backend verifies this marker before reading or publishing
repositories.

### 5.2 Project bundle mutable head

`_head.json` is the only authoritative mutable pointer for a bundle. Its
schema-1 fields are:

```text
schema_version
bundle_id
kind                         # currently only "project"
revision
commit_id
manifest_key                 # relative _manifests/... key
manifest_sha256
manifest_size                # exact serialized manifest byte length
updated_at
```

Readers first load the bounded head metadata, then stage the referenced
immutable manifest using both `manifest_size` and `manifest_sha256`. Parsing
starts only after exact-size and digest verification. The parsed bundle, kind,
revision, and commit identities must then agree with the head.

`manifest_size` is a required nonzero schema-1 field. It has no default and is
not inferred from S3 `Content-Length`, an extra `HEAD` request, or received
bytes. Existing state without the field is unsupported and must be cleared;
there is no migration or compatibility reader.

### 5.3 Project bundle discovery tag

`_tag.json` is a mutable, non-authoritative listing summary:

```text
schema_version
bundle_id
display_name
kind
revision
updated_at
resources
files
```

It is refreshed with CAS after a successful head update. The refresh is
best-effort, so a tag may lag the authoritative head. Remote repository
listing currently discovers `_tag.json` objects and sorts them by bundle ID.

### 5.4 Project bundle immutable manifest

Every manifest is a complete live snapshot:

```text
schema_version
revision
commit_id
updated_at
bundle
  bundle_id, display_name, kind, optional repository_fingerprint
recipe
  schema_version, revision, resource entries and apply policies
captured_with
  Mallard version, optional provider versions, codec versions
published_by_origin
  bundle-scoped host/replica tokens, optional shared label
conversations[thread_id]
resources[resource_id]
files[logical_path]
directories[logical_path]
tombstones[canonical_tombstone_key]
```

A resource descriptor contains:

- resource ID and kind;
- optional provider;
- scope and apply policy;
- display name;
- portable provenance;
- optional project-relative cwd;
- codec version; and
- bounded metadata such as content hash or reviewed dependency intent.

A file entry contains:

```text
resource_id
sha256
size
source_mtime
object_key                    # normally _objects/<sha256>
optional mode                 # safe permission bits only
```

A directory entry contains its resource ID, optional safe mode, and source
mtime, but no object key. Tombstones are typed as resource, resource file,
project-content file, project-content directory, or conversation thread.
File tombstones may carry the last known SHA-256.

### 5.5 Library resource repository

Each Library repository contains exactly one typed resource and advances
independently from every other Library resource and project bundle. Its
immutable `_tag.json` contains:

```text
schema_version
repository_id
provider                     # codex | claude
resource_kind                # agent_skill | plugin_install_intent |
                             # codex_pet_v1 | codex_pet_v2
resource_id
stable_identity_sha256
display_name
optional scope_label
created_at
```

The tag is listing metadata only. Opening a remote choice verifies the tag,
head, manifest, resource identity, exact sizes, hashes, and objects. The
Library `_head.json` pins the resource repository, revision, commit,
manifest key, manifest SHA-256, exact manifest size, and update time.

The immutable manifest records the repository/revision/commit coordinates,
capture version, privacy-preserving publisher origin, and one closed
`CatalogResource`: Agent Skill, plugin install intent, Codex pet v1, or Codex
pet v2. The immutable commit records the same manifest coordinates, previous
commit when present, and publisher origin. The head is the only visibility
boundary and advances with compare-and-swap.

Plugin resources contain semantic provider-native installation intent and no
payload directory. Skills store their bounded regular-file tree; pets store
one validated PNG or WebP atlas. Pet v1 and v2 are different resource IDs,
resource kinds, registrations, links, and remote repositories. The user opens
and operates the desired v1 or v2 link explicitly; Mallard performs no format
conversion or combined implicit selection.

Machine-local registrations and links hold provider paths, storage IDs, and
accepted comparison bases. Portable Library documents contain none of those
machine-local values. Pull reviews pin cross-storage claims; a target claimed
at another digest can only be preserved as a named semantic copy, explicitly
overwritten with acknowledgement, or skipped. Plugins offer overwrite or
skip only.

### 5.6 Typed conversation metadata

A portable conversation entry contains:

```text
thread_id
provider
optional display_title
resource_id
files[]                       # path plus transcript/index/sidecar role
lineage_status and codec
optional head_message_id, message_count
optional lineage_digest, semantic_digest
portable_tail                 # codec, partition_kind, start_record_count,
                              # accepted_end_record_count, accepted_tail_state;
                              # required for Codex entries, end == message_count
optional forked_from
optional preserved            # keep-both provenance: preserved_from + ordinal
optional source_thread_id
optional created_by_origin, last_updated_by_origin
created_at, updated_at
```

The canonical typed transcript path is:

```text
state/conversations/<provider>/<mallard-thread-id>/transcript.jsonl
```

The production conversation reconciliation path currently gives selected
Codex conversations this typed namespace. Other selected provider resources,
including current Claude conversation/state capture, remain generic resources
under their provider logical namespaces.

### 5.7 Other project-bundle logical namespaces

Current capture can produce:

```text
project/<relative-project-path>

state/claude/projects/<relative-cwd-key>/<session-file>
state/claude/file-history/<relative-path>
state/claude/todos/<relative-path>
state/claude/memory/<relative-cwd-key>/<relative-path>
```

Codex discovery initially observes paths under
`state/codex/sessions/...` or `state/codex/archived_sessions/...`, but the
typed publication path rewrites selected Codex transcripts into the canonical
`state/conversations/codex/...` namespace. Absolute source paths are never
used as resource identity or object keys.

### 5.8 Immutable payload objects

`_objects/<sha256>` stores raw selected file bytes. Objects are:

- immutable;
- verified after write;
- content-addressed by SHA-256;
- deduplicated within one project bundle or Library repository; and
- scoped below one `bundle_id` or `resource_repository_id`, so equal content
  in different repositories still has separate object keys.

The manifest, not the object path alone, supplies ownership, size, logical
path, source mtime, and safe mode.

Payload reads use the manifest entry's exact `size` and `sha256`. Both storage
adapters copy each object into a private temporary file while hashing it and
return a file-backed verified-body handle only after EOF matches both
coordinates. The local-folder adapter stages a private snapshot rather than
returning a mutable storage path. The S3/R2 adapter consumes one body chunk at
a time, rejects a declared-length mismatch before polling when possible, and
rejects an overrun before writing the excess chunk.

S3/R2 body waits use a 120-second inactivity timeout per next-chunk wait, not
a 120-second whole-transfer deadline. Actively progressing large transfers
may therefore run longer than 120 seconds. Bulk staging retains bounded
concurrency and input ordering.

The storage contract exposes bounded `get_metadata`, exact
`stage_verified -> StageOutcome`, and ordered `stage_many_verified`
operations. `StageOutcome` distinguishes missing, mismatched, and verified
objects. The previous byte-returning store reads, eager whole-bundle fetch
types, and eager planning/apply wrappers do not exist.

There is no generic 512 MiB payload-object ceiling. Resource-specific capture
limits and aggregate capture budgets still apply where listed below.

### 5.9 Project bundle immutable commit record

Each `_commits/<revision>-<commit-id>.json` stores:

```text
schema_version
bundle_id
revision
commit_id
manifest_key
manifest_sha256
created_at
optional previous_commit_id
added_files
changed_files
removed_files
published_by_origin
```

The commit record is historical metadata. The head remains the authoritative
visibility boundary.

## 6. Publication and consistency model

One publication performs:

1. Validate the request and the exact expected head.
2. Verify every retained file still has its reviewed immutable object.
3. Detect a semantic no-op and write nothing if the snapshot is unchanged.
4. Write new content-addressed payload objects immutably.
5. Serialize the immutable manifest once, record its exact byte length and
   SHA-256, and write it immutably.
6. Write the immutable commit record.
7. Compare-and-swap `_head.json` with the same manifest size and hash.
8. Best-effort compare-and-swap `_tag.json`.
9. Commit the corresponding local bindings and reviewed base.

For one Library link, the same immutable-object and CAS ordering applies to
exactly one `library/<resource-repository-id>/` head. Its create-only tag is
written once, and only that link's accepted head/content base advances after
publication succeeds. A concurrent head change makes the reviewed plan stale
or is resolved by verified read-back; it never advances another resource.

Local-folder CAS is serialized by `.mallard/.storage.lock`; its ETag is the
SHA-256 of the current bytes. S3/R2 uses service ETags with
`If-None-Match: *` for creation and `If-Match` for replacement. Ambiguous S3
responses are resolved by reading the object back.

If publication fails before head CAS, immutable objects, manifests, or commits
may remain unreachable. Readers cannot see them as a revision because the
head did not move. The current object-store interface has no delete operation,
so there is no active remote garbage collector for those orphans.

## 7. What crosses the machine boundary

### Stored remotely

- Selected raw payload bytes, including conversation transcripts, provider
  memory, and explicitly approved non-Git project content.
- Bundle/project display name and optional repository fingerprint.
- Selected recipe and apply policies.
- Resource display names, portable provenance, project-relative cwd, and
  bounded descriptor/dependency metadata.
- Logical paths, hashes, sizes, source mtimes, and safe mode bits.
- Conversation titles, provider source thread IDs, lineage/fork metadata,
  lineage and semantic hashes, and timestamps.
- Bundle-scoped publishing-origin tokens.
- Typed tombstones.

### Never stored remotely

- Raw `host_id`, `replica_id`, local project ID, storage ID, profile ID, or
  local aliases.
- Absolute project, provider-home, executable, target, staging, backup, or
  local-storage paths.
- S3/R2 credentials, endpoints as local configuration, or storage exclusions.
- Project-content exclusion preferences and other local trust decisions.
- Provider-thread ownership bindings for a particular local profile.
- Restore, dependency, Push, or setup review plans.
- Operation journals, receipts, backups, quarantine files, chat-history
  cache, or activity logs.

Publishing-origin tokens are HMAC-SHA-256 values derived from the private
`host_id` and the bundle ID; replica tokens also include the local replica ID.
The raw IDs do not leave the machine, and tokens for different bundles cannot
be directly linked to the same installation.

The portable side is still sensitive. It intentionally contains selected
conversation and project bytes, titles, relative paths, and provider thread
IDs. Secret detection and warning review are safeguards, not proof that a
payload is safe. Shared storage should be treated as private user data.

## 8. Current identity and size contracts

- `bundle_id`: exactly 32 lowercase hexadecimal characters from 128 random
  bits.
- `resource_repository_id`: `library-` plus exactly 32 lowercase hexadecimal
  characters from 128 random bits.
- Library registration, link, scope, plan, action, commit, and operation IDs:
  closed prefixes plus fixed lowercase hexadecimal suffixes.
- Library resource ID: `resource-` plus a 64-character identity SHA-256.
- `host_id`: exactly 32 lowercase hexadecimal characters and private.
- origin host/replica token: exactly 64 lowercase hexadecimal characters.
- Mallard named IDs: 1-128 lowercase ASCII identifier characters.
- logical path: at most 1,024 bytes, relative and normalized.
- resources: at most 20,000 per manifest.
- files, directories, and tombstones: at most 100,000 each.
- one conversation source file: at most 100 MiB.
- one ordinary captured provider file: at most 16 MiB.
- one project-content file: at most 16 MiB.
- aggregate selected capture/project content: at most 512 MiB.
- conversation bindings: at most 100,000.
- active conversation journal operations: at most 10,000.
- Library resources: at most 512 registrations/links per document.
- one standalone skill: at most 256 regular files, 2 MiB per file, and 16 MiB
  total.
- one pet manifest: at most 16 KiB; one pet atlas: at most 8 MiB compressed
  and 16 MiB decoded.
- one Library revision: at most 256 MiB of payload bytes.
- Library claims: at most 4,096; preservation witnesses: at most 512; recent
  operations: at most 256.

There is no generic portable-object size cap. Manifest-declared sizes are
exact integrity coordinates, while the remaining per-resource and aggregate
limits are capture/product policies.

## 9. Corrections to older repository documentation

Two details in older current-looking documentation no longer match the code:

1. Payloads are now published under `_objects/<sha256>`, not under an active
   `_uploads/<upload-id>/files/...` tree. `_uploads/` is reserved only.
2. Local metadata synchronization is no longer just an in-process mutex.
   `persistence.lock` and `host_operation.lock` are OS file locks that
   coordinate Mallard processes sharing one local state directory.

When this document and an older plan/report disagree, the source files below
are authoritative.

## 10. Code sources

- [`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs): schemas, IDs,
  portable manifest, plans, journals, bindings, receipts, and limits.
- [`persistence.rs`](../crates/mallard-infrastructure/src/persistence.rs):
  `~/.mallard` paths, bounded JSON, revisions, atomic writes, locks, host
  rotation, and quarantine.
- [`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs):
  physical-to-logical path mapping, resource discovery, capture, filtering,
  and portable descriptor conversion.
- [`commands.rs`](../src-tauri/src/project_sync/commands.rs): cross-document
  orchestration, typed Codex publication, and local/remote commit order.
- [`conversation_materialization.rs`](../crates/mallard-infrastructure/src/conversation_materialization.rs):
  intent payloads, provider-side stages, backups, recovery phases, journaled
  whole-document installs, and journal cleanup.
- [`bundle_engine.rs`](../crates/mallard-infrastructure/src/bundle_engine.rs):
  portable keys, marker/head/tag/manifest/commit formats, immutable objects,
  CAS, lazy verified-body access, restore plans, streamed apply, and backups.
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs):
  S3/R2 ETag, conditional-write, chunk-to-tempfile staging, inactivity
  timeout, and pagination behavior.
- [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs):
  local-only activity cache and provider session scanning through the
  registered adapters.
- [`activity_log.rs`](../src-tauri/src/activity_log.rs): local JSONL event
  schema, redaction, segmentation, locking, and retention.
- [`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs):
  Library resource identities, strict schemas, scopes, registrations, links,
  claims, and per-resource plans.
- [`storage_catalog/resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs):
  Library tag listing, immutable objects/manifests/commits, verified reads,
  head CAS, and continuity checks.
- [`storage_catalog/persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs):
  independent bounded Library state, quarantine, target locks, and backups.
- [`storage_catalog/resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs):
  scope rows, per-resource linking, and per-link Push/Pull/Refresh/Fork.
- [`src/types.ts`](../src/types.ts): frontend DTO mirror of serialized Rust
  contracts.
