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
   Project bundles use `.mallard/v1/repositories/<bundle-id>/`; Library
   resources use `.mallard/v1/library/<kind-repository-id>/` — one repository
   per (provider, resource kind), holding every resource of that kind as a
   set manifest.

The portable anchor is `bundle_id` for a project or the kind repository id
for the Library. A Library repository id is derived, never minted:
`library-` + the first 32 hex of a domain-tagged digest over the provider and
kind tokens, so every replica that syncs the same kind converges on the same
repository. Local project, storage, host, replica, profile, and link IDs do
not appear in portable documents. There is no mutable head: an immutable
publication record, written last, points to an immutable manifest and
content-addressed payload objects, and the current publication is selected
from the names of a stream's records. The Library manifest maps resource ids
to their published entries.

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
  library/<kind-repository-id>/   every Library resource of one (provider, kind)

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
|-- materializations.json
|-- activity_cache.json
|-- observation_cache.json
|-- catalog_resource_profiles.json
|-- catalog_scopes.json
|-- catalog_scope_links.json
|-- catalog_resource_materializations.json
|-- catalog_resource_preservations.json
|-- catalog_resource_operations.json
|-- host_rotation.json                         # transient recovery journal
|
|-- project_drafts/
|   `-- <draft-id>.json
|-- storage_drafts/
|   `-- <draft-id>.json
|-- setup_transactions/
|   `-- <draft-id>.json
|
|-- staging/
|   `-- pull-conversations/                    # Apply-only temporary files
|-- backups/
|   |-- <plan-id>/...
|   |-- catalog_resources/
|   |   `-- <plan-id>/...
|   `-- conversations/
|       `-- <operation-id>/<target-index>.bak
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
| `conversation_bindings.json` | Local-to-portable conversation identity map | Required nullable `path_origins_ref` (SHA-256 and byte length), Mallard thread ID, provider thread ID, project/replica/profile scope, materialization state, immutable root tail layout (codec, partition kind, fixed start), `forked_from` provenance hint, keep-both `preserved` provenance (`preserved_from` origin + per-origin ordinal) | No |
| `path_origins/<sha256>.json` | Immutable local inverse path edits | Schema 1, codec, physical owner, source IDs, strict record hashes, derived-record flags, JSON pointers, UTF-8 byte ranges, and wire atoms; no transcript copy; 128 MiB bound | No |
| `materializations.json` | Durable Pull history | Plan and bundle coordinates, status, action receipts, absolute targets, backup paths, conversation receipts, errors | No |
| `activity_cache.json` | Local session-history acceleration for every bound provider | File size/mtime keys, provider key, parsed ownership, cwd, Git/session metrics, and identities; cached title, summary, and chat text are deliberately omitted | No |
| `observation_cache.json` | Local comparison acceleration | Bounded, schema-1 tail observations keyed by project, storage, provider home, and checkpoint generation; reset removes the in-memory and on-disk entries | No |
| `catalog_resource_profiles.json` | Machine-local Codex/Claude Library profiles | Provider key, display name, configured `CODEX_HOME`/`CLAUDE_CONFIG_DIR` path, revisions | No |
| `catalog_scopes.json` | User-added Catalog menu rows | One row per (profile × resource kind); membership is the kind inventory, never stored | No |
| `catalog_scope_links.json` | Scope-storage links | One link per (scope × storage): the derived kind-repository ID, the accepted head continuity pin, and per-resource accepted content bases | No |
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

Mallard maintains three conversation identities in two structurally disjoint
Mallard id spaces plus the provider-native space:

| Identity | Namespace | Lives | Minted |
| --- | --- | --- | --- |
| `thread_id` (local) | `thread-…` (derived 32-hex or generated 24-hex; `cloudthread-` rejected) | binding rows, checkpoints, journal ops, receipts, Activity — never in a bundle | derived at registration (`initial_conversation_thread_id`), or at pull apply for `CreateLocal` |
| `cloud_thread_id` | `cloudthread-` + 24 hex (prefix-pinned) | manifest rows, cloud resource ids, logical paths, tombstones, provenance | at push apply for an unmapped `CreateCloud`; one portable identity per lineage across all storages |
| `provider_thread_id` | provider-native (UUID) | binding rows, provider files | provider app, or at pull apply for materializations |

The pairing `(thread_id, cloud_thread_id)` lives only on the machine's
binding row (`LocalConversationBinding.cloud_thread_id`, present-on-wire
nullable: always serialized, `null` when unmapped, and the key is required on
read so pre-cutover rows fail closed). The cloud stores no mapping and no
local identity with linkage semantics. Per binding scope
(host/replica/project/profile), at most one row may claim a given cloud
thread id. Every fresh identity is minted at Apply, never at planning. Whole-document
after-states carry the chosen identity. Failed Apply is not replayed.

One provider thread ID may have only one owner for a given host/profile.
`conversation_bindings.json` also records:

- whether the provider copy is `pending`, `resumable`, or `stored_only`;
- the immutable root tail layout — codec, partition kind
  (`whole_thread` or `fixed_paginated_tail`), and the fixed start record
  count — seeded at first full parse or imported from the verified cloud
  value at Pull install; and
- `forked_from` as a provenance hint in the cloud id space that is surfaced
  but never used for matching or classification; and
- `preserved` as the keep-both provenance hint (`preserved_from` origin in
  the cloud id space + per-origin ordinal), published with the thread on
  Push and restored from the cloud record on other machines' pulls —
  display-only, like `forked_from`.

Acceptance state does not live on the binding. Each `ProjectStorageLink` in
`sync_config.json` holds per-provider-scope conversation checkpoints keyed by
the local `thread_id`: the accepted five-field portable tail with its
commit/manifest witness, the replaceable observed tail, the physical mapping
cursor with a raw-range pin and trailing probe window, the machine-local
prefix witness over the fixed prefix, and generation/binding-revision fences.
Both cursor providers carry real cursors here: Codex under
`mallard-codex-portable-tail-{v1,wire-v1}` with graph-resume fields, Claude
under `mallard-claude-portable-tail-{v1,wire-v1}` with
`last_canonical_message_id` = the canonical leaf uuid (synthetic-turn fields
always empty, partition always `WholeThread` at zero).
Removing a project removes its links and checkpoints; bindings — and with
them the cloud mappings — survive, and a re-registered project re-proves
acceptance mapping-driven: the adoption probe refolds local bytes against the
cloud tail the binding's mapping names before the thread may push again.
Mapping lifetime equals binding-row lifetime: if binding rows are lost, the
machine's threads are local-only again, pulls re-materialize cloud threads as
new local threads, and pushes mint new cloud ids.

The portable manifest has its own conversation entry keyed by the
`cloudthread-…` id; manifest validation rejects any conversation row outside
that namespace, so a manifest cannot inject a local-space id at all. It
includes `source_thread_id` as a content fact only — the provider session id
embedded in the stored transcript bytes — never an identity.

### 3.4 Reviews and transaction state

Project Sync Push and Pull reviews are not persisted. Push Compare returns a
comparison value. Pull Compare returns an in-memory schema-1 `RestorePlan`
review value to keep the current IPC shape. Apply rechecks the review anchor,
captures the selected resources, and builds fresh executable actions and pins
in memory.

Project Sync does not create `restore_plans/`, `conversation_push_plans/`,
`conversation_sync_journal.json`, or `conversation_operations/`. A failed or
interrupted Apply is not retried, replayed, resumed, rolled back, or rolled
forward. The user must Compare and Apply again.

Library (Catalog) Push/Pull/Fork reviews are not documents: they live in a
bounded in-memory plan store, expire after 24 hours, and are discarded on
apply, replacement, or app restart. Apply re-verifies every pin against live
state, so a lost review can only fail closed ("review again").

Completed project Pull results live in `materializations.json`. Library
outcomes and recovery state live in the claim, preservation, and operation
documents. None of these documents is portable history.

Setup has two local layers:

- `project_drafts/<draft-id>.json` is resumable UI state with preallocated
  project/bundle IDs, selected paths, pending or existing profiles/storage,
  repository choice, selected resource IDs, and a discovery signature. It
  never stores discovered payload bytes or remote listings.
- `setup_transactions/<draft-id>.json` is the deterministic finalize intent
  written before mutating the permanent config and binding documents. It
  makes interrupted finalization idempotently recoverable.

Storage setup has one draft layer of its own: `storage_drafts/<draft-id>.json`
is the resumable New-storage form (schema 1, revision-guarded, capped at 16
drafts). It persists the full half-edited connection **including the secret
access key** at the same owner-only file permissions as `sync_config.json`,
because Cloudflare shows that key exactly once. Draft list, create, and update
responses are redacted to a blank secret plus a `secret_present` flag. Only an
exact get by draft ID returns the selected draft's secret
to its editable password field so the user can reveal and review it. The
verified upsert resolves the exact revision-pinned drafted secret server-side
and discards the draft after a successful commit. It has no finalize
transaction — commit is the ordinary verified upsert (R2) or generic config
save (local).

Agent-assisted R2 setup targets the exact open draft by ID and expected
revision. The CLI may populate that machine-local draft after browser consent,
but it does not probe the storage or mutate `sync_config.json`. The running app
uses the redacted watcher listing as a change signal, then exact-loads the
selected draft into the editable form for review. Only the user's **Finish
setup** action runs the verified upsert and commits the storage.

The Library needs neither layer: adding a Catalog row records one scope in
`catalog_scopes.json`, and linking the scope to a storage lands one link in
`catalog_scope_links.json` in a single locked mutation.

### 3.5 Conversation Apply staging and backups

A provider materialization keeps its action and payload state in memory for
one Apply attempt. `.<target-name>.mallard-<operation-fragment>-<index>.stage`
is created next to each provider target so the final rename stays on the same
filesystem. Codex graph reconstruction can also use
`staging/pull-conversations/<action-id>.jsonl` during the same Apply. Apply
removes these stage files on success and failure.

If an existing provider target is replaced, its backup is stored at
`backups/conversations/<operation-id>/<index>.bak`. Backups needed by durable
receipts remain local. There is no durable intent payload or conversation
recovery journal.

Normal restore backups are grouped under `backups/<plan-id>/`. Library
skill/pet replacement backups use
`backups/catalog_resources/<plan-id>/`; their operation metadata stays in
`catalog_resource_operations.json`.

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
| `storage_drafts/<id>.json` | 128 KiB |
| `setup_transactions/<id>.json` | 8 MiB |
| `conversation_bindings.json` | 64 MiB |
| `materializations.json` | 64 MiB |
| `host_rotation.json` | 192 MiB |
| `activity_cache.json` | 32 MiB / 50,000 entries |
| `observation_cache.json` | 16 MiB / 4,096 entries |
| `catalog_resource_profiles.json` | 1 MiB |
| `catalog_scopes.json` | 1 MiB |
| `catalog_scope_links.json` | 16 MiB |
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

The local-folder, S3/R2 and Dropbox adapters expose the same object keys, and
Mallard Cloud holds them under one R2 prefix per repository behind its
gateway (`PLAN_MANAGED_CLOUD_CAS_FREE.md`); only Mallard Cloud holds
`_views/`:

```text
<storage root or bucket>/
`-- .mallard/
    |-- .storage.lock                         # local-folder backend only
    |-- _storage.json
    `-- v1/
        |-- repositories/
        |   |-- _claims/                      # one claim per claimed parent (S3/R2, Dropbox)
        |   |   `-- <bundle-id>/
        |   |       `-- <parent token, or root>.json
        |   |-- _commits/                     # every project record, nothing else
        |   |   `-- <bundle-id>/
        |   |       `-- <record name>
        |   `-- <bundle-id>/
        |       |-- _objects/
        |       |   `-- <payload-sha256>
        |       |-- _manifests/
        |       |   `-- <revision>-<commit-id>.json
        |       |-- _views/                   # Mallard Cloud only: session views
        |       |   `-- <view-schema>-<transcript-sha256>.json
        |       `-- _uploads/                 # reserved for project bundles
        `-- library/
            `-- <kind-repository-id>/
                |-- _tag.json                 # constant bytes, discovery only
                |-- _claims/
                |   `-- <parent token, or root>.json
                |-- _commits/
                |   `-- <record name>
                |-- _manifests/
                |   `-- <revision>-<commit-id>.json
                `-- _objects/
                    `-- <payload-sha256>
```

Every object is immutable, and no conforming writer ever writes different
bytes to the same key: payloads are content-addressed, a manifest's key holds
a commit id minted per publication, a record's name holds the digest of its
bytes, and the marker and the Library tag are constant. A provider therefore
needs three operations: write bytes at a key, read a key, list a prefix.
There is no `_head.json` and no `_tags/` namespace.

Project records live in one flat namespace beside the bundle folders. Listing
`_commits/<bundle-id>/` returns one stream, which is what a refresh needs;
listing `_commits/` returns every project stream's records and no payload
key, which is what bundle discovery needs. Library records stay inside each
repository, because the Library never enumerates repositories.

A claim lives beside the records and never among them: `_claims/` is not
under `_commits/`, so no record listing returns one and no refresh, review or
discovery lists or reads one. Its name is the parent it claims, that parent's
token or the word `root`; its bytes are the record of that parent's one
child, exactly as written under `_commits/`. Only a store whose server
refuses a second write of a key holds claims: S3/R2 and Dropbox. The local
folder holds none, because a folder a sync product shares would take two
claims and a network share promises no create-only write.

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
{"format":"mallard-append-storage","layout_version":1}
```

The bytes are constant, so concurrent first writers write identical bytes and
creation is idempotent without any exclusivity.

Opening either backend verifies this marker before reading or publishing
repositories.

### 5.2 Project bundle publication record and head selection

The publication record is the only thing that publishes a snapshot, and it is
written last. Its object name carries its graph facts:

```text
<revision>.<commit_id>-<record_sha256>.<parent>.<created_at_ms>.json

revision         10 digits, zero-padded; the depth, and the printed version
commit_id        commit- plus 24 lowercase hexadecimal characters
record_sha256    SHA-256 of the record's exact bytes
parent           <parent commit_id>-<parent record_sha256>, or the word root
created_at_ms    13 digits, zero-padded
```

`<commit_id>-<record_sha256>` is a publication's token. A name is lowercase
`[a-z0-9.-]`, at most 223 bytes, and names sort by depth. A name outside the
grammar (`.DS_Store`, a sync client's conflicted copy) is ignored and written
to the activity log; it is never a record.

The current publication of a stream is selected from the names of one
listing, without reading a record: starting at the virtual root, the child
with the smallest `(created_at_ms, commit_id, record_sha256)` is taken until
a publication has none. A record whose parent is not listed is never reached.
Every listed record that is not on that chain is kept and not selected;
nothing marks, moves, or deletes it. The same listing gives every computer
the same selection.

The record's schema-1 fields are:

```text
schema_version
bundle_id
revision
commit_id
optional previous_commit_id       # both absent for a first publication,
optional previous_record_sha256   # both present otherwise: the parent token
created_at_ms                     # taken when the record is built; ranks siblings
manifest_key                      # relative _manifests/... key
manifest_sha256
manifest_size                     # exact serialized manifest byte length
added_files
changed_files
removed_files
published_by_origin
summary                           # display_name, kind, resources, files,
                                  # registration_only, providers,
                                  # optional repository_fingerprint
```

`BundleHead` is the in-memory view of the selected record, field for field;
it is never a stored object. Readers list the stream, select, read the
selected record (at most 64 KiB) and verify that its bytes hash to the digest
in its name and that its fields equal the facts in its name, then stage the
referenced immutable manifest using both `manifest_size` and
`manifest_sha256`. Parsing starts only after exact-size and digest
verification. A selected record or manifest that cannot be read or fails
verification is an error for that publication; selection never moves to
another candidate. An unchanged selection costs the one listing: the process
remembers the records it has read.

`manifest_size` is a required nonzero schema-1 field. It has no default and is
not inferred from S3 `Content-Length`, an extra `HEAD` request, or received
bytes.

### 5.3 Project bundle discovery

Bundle discovery lists `.mallard/v1/repositories/_commits` once, groups the
names by bundle id, selects each stream, and reads those records through the
store's bounded-concurrency metadata fetch. The listing rows come from each
record's `summary`, so discovery never reads a manifest and is exactly as
fresh as the publication it shows.

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
conversations[cloud_thread_id]
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

A Library repository holds the resources of one coordinate and advances
independently from every project bundle. On Mallard Cloud, which holds one
item per repository, each item publishes on its own stream instead, under
`library-<32 hex>` derived from the coordinate and the resource id
(`CatalogResourceRepositoryId::for_item`); its tag then also carries the
`resource_id`, and its manifest holds exactly that one entry. The repository's
`_tag.json` holds constant bytes derived from the coordinate alone:

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
```

The tag is listing metadata only. Opening a remote choice verifies the tag,
the selected record, the manifest, resource identity, exact sizes, hashes, and
objects. The Library publication record uses the project record's name
grammar and protocol fields (it needs no `summary`), and
`LibraryResourceHead` is the in-memory view of the selected record.

The immutable manifest records the repository/revision/commit coordinates,
capture version, privacy-preserving publisher origin, and the repository's
complete resource set: a map from resource id to a published entry holding
one closed `CatalogResource` — Agent Skill, plugin install intent, Codex pet
v1, or Codex pet v2 — and its update time. Every entry must belong to the
repository's own (provider, kind) derivation. The record is the only
visibility boundary and is written last, with list-then-write; a push
publishes the selected set plus the reviewed upserts, so it can never delete
another replica's resources. The Library does not rebase: Apply requires the
selected publication to be the one the review was computed from, and takes a
local single-flight lock per repository.

Plugin resources contain semantic provider-native installation intent and no
payload directory. Skills store their bounded regular-file tree; pets store
one validated PNG or WebP atlas. Pet v1 and v2 are different resource IDs,
resource kinds, scopes, and kind repositories. The user opens and operates
the desired v1 or v2 row explicitly; Mallard performs no format conversion
or combined implicit selection.

Machine-local scope links hold provider paths, storage IDs, the accepted head
continuity pin, and per-resource accepted content bases. Continuity is
ancestry: an accepted publication that is neither the selected one nor one of
its ancestors, or whose pin names another storage identity or another
repository, is read as absent, and nothing is reset: every row compares as
unreviewed, a Push drops the saved acceptances and keeps the ones it made, and
a Pull from that state saves the head it reviewed with what it accepted. A
repository that lists empty holds no chain, so it reads the same way: the page
loads, and one Push repopulates it. Portable Library
documents contain none of those machine-local values. Pull reviews pin
cross-storage claims per item; a target claimed at another digest can only be
preserved as a named semantic copy, explicitly overwritten with
acknowledgement, or skipped. Plugins offer overwrite or skip only.

### 5.6 Typed conversation metadata

A portable conversation entry contains (all thread references in the cloud
id space):

```text
thread_id                     # cloudthread-… (never a machine-local id)
provider
optional display_title
resource_id
files[]                       # path plus transcript/index/sidecar role
lineage_status and codec
optional head_message_id, message_count
optional lineage_digest, semantic_digest
portable_tail                 # codec, partition_kind, start_record_count,
                              # accepted_end_record_count, accepted_tail_state;
                              # required for Codex entries (codex wire codec)
                              # and Claude entries (claude wire codec),
                              # end == message_count
wire_redaction                # {codec, ruleset fingerprint} of the stored
                              # wire bytes; required for codex/claude rows
wire_path_map                 # {codec: mallard.pathmap.v1}; required for
                              # claude rows (whose pure-JSONL wire has no
                              # in-band record), forbidden elsewhere — codex
                              # declares in-band via __mallard_path_map__
optional forked_from          # cloud id space
optional preserved            # keep-both provenance: preserved_from (cloud
                              # id space) + ordinal
optional source_thread_id
optional created_by_origin, last_updated_by_origin
created_at, updated_at
```

The schema-1 path codec uses terminated spelling tags, such as
`/__mallard_root__/b;` for the binding root and `/__mallard_root__/bt;` for
its tilde spelling. Tags also distinguish canonical aliases and raw or
percent-encoded file URIs. URI references replace the full URI prefix.
Literal `/__mallard_root__` and `/__mallard_escape__` text uses
`/__mallard_escape__/r` and `/__mallard_escape__/e`. No source text spelling
is reserved. Each pass scans its input once and does not scan emitted text.
Unknown tags and malformed wire escapes fail validation.

Pull records exact inverse edits in local origin files. Push reuses them for
retained history, including records with zero edits. It scans new records.
This preserves literal destination paths and original path spellings through
later pushes. Codex portable transcripts keep source records; Pull rebuilds
redundant native display records after path decoding. Derived local records
are hash-checked and omitted on the next Push.

`path_origins_ref` belongs to the physical history binding, independently of
storage. A null reference means no path capture yet. Referenced files must
pass their size, digest, owner, codec, and strict record checks. The file is
staged before Apply writes provider data; its reference enters the binding
before the accepted checkpoint advances. Project metadata reset removes all
session bindings for that project, including these path-origin references.
It keeps the immutable origin files and provider sessions. Host rotation
stages files with the new owner for retained bindings. A failed Apply is
not rolled back. Missing or stale origins fail the affected thread.

See [codec rules and verification](../formal/pathmap-origins-v1/README.md).
This is a clean replacement under schema 1 with no migration or old reader.

The canonical typed transcript path is:

```text
state/conversations/<provider>/<cloud-thread-id>/transcript.jsonl
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

## 6. Publication and consistency model

One publication performs:

1. Validate the request and the exact observed publication: apply lists the
   stream and selects, and everything is checked and built against it.
2. Verify every retained file still has its reviewed immutable object.
3. Detect a semantic no-op, prove with one listing that the compared
   publication is still selected, and write nothing.
4. Write new content-addressed payload objects.
5. Serialize the immutable manifest once, record its exact byte length and
   SHA-256, and write it.
6. Build the publication record in memory: a new commit id, the parent token
   from step 1, the revision, `created_at_ms`, the origin, the manifest
   reference, the summary. It is never written to local disk and never
   outlives the operation.
7. List, then write: list the stream; if the listing already holds this
   record's token an earlier attempt landed; continue only if the selected
   publication is the record's parent, which means the parent has no child
   (an empty stream for a first publication), otherwise stop with
   `RemoteChanged` and nothing written. On a store with claims (S3/R2 and
   Dropbox), claim the parent: one create-only write of the record's bytes
   at `_claims/<parent token, or root>.json`, which the store answers in one
   request. A free key, or one that already holds these bytes, is this
   push's claim. A key that holds other bytes is taken: the push stops before
   its record. When a listing holds the claimed record, another computer got
   in first, `RemoteChanged`; when none does, a push onto this version did
   not finish, `PushUnfinished`, and nothing writes that record by itself,
   except that this computer's own record for the same content, left by an
   attempt whose record request failed, is written as this push's. Then send
   the record, once. A listing older than ten wall-clock seconds is taken
   again before sending, because the monotonic clock stands still while a
   laptop sleeps. The claim and the record are one request each, with no
   retry inside the transport and none outside it: a request the store does
   not acknowledge is settled by one read-only check, the claim key after a
   claim and the listing of step 8 after a record. What the check shows
   landed counts; what it shows absent did not land, and the push fails with
   the store's error; a check that cannot be made is `OutcomeUnknown`.
   Nothing is sent again, and nothing is resent after a restart: the next
   push is the user's.
8. List once more. If that listing selects this record, commit the
   corresponding local bindings and reviewed base. If it selects another
   computer's record, stop with `RemoteChanged`: the publication failed as a
   whole, nothing local is committed, and the record stays in the storage,
   never selected. The claim proves that the parent has no other child, not
   that the parent is still there: a clear between the claim and the record
   leaves the record an orphan, and only this listing shows it.

A push that claimed a version and never wrote its record is finished only
by the user: the Push review and the Library page offer *Finish that push*
while the error is that sentence, and the verb writes the record the claim
holds, once, through the same record path, then reports "Finished. Compare
again.", or `RemoteChanged` when the claim is gone or its record is already
listed. Nothing deletes or reassigns a claim; only a clear removes one.

On the local folder a sibling of a publication can land after that
publication became visible: another computer listed before this record was
visible, then wrote a record that carries an earlier timestamp, usually
within seconds and in principle at any later time. The earlier-ranked sibling
is selected and the other is kept and not selected. A writer that reads that
from the listing of step 8 fails, and its base stays on the shared parent:
the next refresh reads `Advanced`, and the next Push publishes the same
content. On S3/R2 and Dropbox the claim admits one child per parent, so a
listed version has no sibling and cannot lose while the storage's claims and
records are intact; an interrupted clear can remove a claim while keeping
the parent (below), and a delayed writer can then displace the child.

A sibling that becomes visible only after step 8 already selected this record
leaves local state on an unselected record. The record is listed, it is not
selected, and its parent names lead over listed records to a selected
publication whose selected child is another record; ranks never change, so
that verdict is final. Nothing is rolled back: recovery is a user action. The
next observation reports the link `Displaced`, every scope base and session
acceptance saved in that publication is stale and read as absent (below), the
standing notice says whether this computer's last push or last pull is no
longer part of the history, and the user compares and pushes again. The Push
or the Pull that saves new bases removes the stale ones in the same
transaction.

A record that no listed record connects to the selected chain is left alone:
that is a stream that was cleared (`RemoteMissing`), or cleared and pushed
again (`Replaced`). Such a record is **stale**. A scope base
(`reviewed_in`) and a session acceptance (`accepted_at_commit_id`) count only
while the publication they name is on the selected chain; a stale one is read
as absent by every comparison and is never deleted at a refresh, because a
short listing looks the same as a clear. A stale scope base makes an absent
scope `Fresh` and a changed one `Unreviewed`; a scope that still has exactly
the reviewed content stays `InSync`. A stale acceptance makes the session
compare from content: absent from the storage it is published under the cloud
id it already has, an equal or an earlier stored copy is proven by
first-contact adoption, and a longer or a different stored copy is kept next
to the local one by Pull ("Keep both"), never replaced. The Push or the Pull
that saves new bases removes every stale scope base of the link in the same
transaction; a stale acceptance is replaced when its session is next pushed or
pulled. No document changes shape for any of this.

For one Library scope link, the same ordering applies to exactly one
`library/<kind-repository-id>/` stream. Only that link's accepted head and
the reviewed rows' accepted bases advance after publication succeeds. A
selection that changed after review makes the reviewed plan stale; it never
silently advances an unreviewed resource.

Local-folder writes are serialized by `.mallard/.storage.lock`. S3/R2 writes
create-only with `If-None-Match: *`, and the claim and the record write run
with SDK retries off; a refused claim costs one read that tells this
attempt's own bytes from another's. Dropbox writes a record, and a claim,
with one `files/upload` request in add mode without a strict conflict check:
identical bytes are answered as written, other bytes as a conflict, and one
download then returns them. None of them overwrites by revision, and a
provider with plain overwrite and no conditional write runs the same engine
without claims. The setup probes of R2 and Dropbox write other bytes to a key
they just wrote and require the refusal: a destination that overwrites
cannot be set up.

If publication stops before the record is sent, immutable objects and a
manifest may remain unreferenced. Readers cannot see them as a revision
because no record names them. There is no remote garbage collector.

Storage clear keeps the marker and deletes claims first, then records:
`repositories/_claims`, then `repositories/_commits`, then the rest of
`repositories`; inside a Library repository `_claims/` already sorts before
`_commits/`, and `_commits/` before `_manifests/` and `_objects/`. That is
the opposite of the write order, so an interrupted clear leaves records
without claims and never a claim without its record. Record names sort by
depth, so a stream's first publications go first; once they are gone the
stream selects nothing while every manifest and object still exists, so an
interrupted clear never leaves a selected record pointing at deleted data.
After a clear the root's claim is free again for the next first version.

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
- kind repository id: `library-` plus the first 32 lowercase hexadecimal
  characters of a domain-tagged digest over the provider and kind tokens —
  derived, never random.
- Library link, scope, plan, action, commit, and operation IDs: closed
  prefixes plus fixed lowercase hexadecimal suffixes.
- Library resource ID: `resource-` plus a 64-character identity SHA-256.
- `host_id`: exactly 32 lowercase hexadecimal characters and private.
- origin host/replica token: exactly 64 lowercase hexadecimal characters.
- Mallard named IDs: 1-128 lowercase ASCII identifier characters.
- local conversation `thread_id`: `thread-` prefix (derived 32-hex or
  generated 24-hex); parsing rejects the `cloudthread-` namespace.
- `cloud_thread_id`: `cloudthread-` plus exactly 24 lowercase hexadecimal
  characters from 96 random bits; parsing requires the prefix.
- logical path: at most 1,024 bytes, relative and normalized.
- resources: at most 20,000 per manifest.
- files, directories, and tombstones: at most 100,000 each.
- one conversation source file: at most 100 MiB.
- one ordinary captured provider file: at most 16 MiB.
- one project-content file: at most 16 MiB.
- aggregate selected capture/project content: at most 512 MiB.
- conversation bindings: at most 100,000.
- active conversation journal operations: at most 10,000.
- Library resources: at most 512 per kind-repository manifest; scope links:
  at most 128 storages per scope.
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
  portable keys, manifest and publication-record formats, immutable objects,
  head selection, lazy verified-body access, restore plans, streamed apply,
  and backups.
- [`storage_core/publication.rs`](../crates/mallard-infrastructure/src/storage_core/publication.rs):
  stream listing, selection over record names, verified record reads,
  list-then-write with the claim of the parent, the record a taken claim
  holds, and the process memory of listings and records.
- [`mallard-domain/src/publication.rs`](../crates/mallard-domain/src/publication.rs):
  record-name grammar, publication tokens, the mainline selector, and the
  ancestor test.
- [`s3_store.rs`](../crates/mallard-infrastructure/src/storage_core/s3_store.rs):
  S3/R2 create-only writes, the single-request record and claim writes,
  chunk-to-tempfile staging, inactivity timeout, and pagination behavior.
- [`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs):
  local-only activity cache and provider session scanning through the
  registered adapters.
- [`activity_log.rs`](../src-tauri/src/activity_log.rs): local JSONL event
  schema, redaction, segmentation, locking, and retention.
- [`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs):
  Library resource identities, strict schemas, scopes, scope links, claims,
  and scope-level plans.
- [`storage_catalog/resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs):
  Library set-manifest publication, immutable objects/manifests/records,
  verified reads, list-then-write, and ancestry continuity.
- [`storage_catalog/persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs):
  independent bounded Library state, quarantine, target locks, and backups.
- [`storage_catalog/resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs):
  scope rows, scope-storage linking, and scope-level Push/Pull plus
  per-resource Fork.
- [`src/types.ts`](../src/types.ts): frontend DTO mirror of serialized Rust
  contracts.
