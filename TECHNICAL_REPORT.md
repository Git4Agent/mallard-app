# Mallard Current Technical Report

**Implementation date:** 2026-09-08
**Application version:** 0.1.2
**Scope:** the active implementation only. This report does not describe
superseded formats, migrations, or compatibility behavior.

## 1. Current version contracts

| Contract | Current version |
| --- | ---: |
| Desktop application | 0.1.2 |
| Machine-local sync configuration | Schema 1 |
| Portable bundle head and manifest | Schema 1 |
| Portable storage layout | Version 1 |
| Machine project/profile catalog | Schema 1 |
| Host identity and conversation state | Schema 1 |
| Bundle recipe | Schema 1 |
| Per-link project-content preferences | Schema 1 |
| Setup drafts and transactions | Schema 1 |
| Activity entries and policy | Schema 1 |

The implementation requires these exact contracts and fails closed on any
other schema. Local documents and portable bundles are independently
versioned, but both current formats were reset to schema 1. There is no
migration or compatibility reader; testing this build requires fresh
`~/.mallard` state and empty storage.

The application stack is Tauri 2 with a Rust 2021 backend and a React 19,
TypeScript 5.8, and Vite 7 frontend. Rust performs all filesystem, validation,
hashing, persistence, and storage operations. React owns the review state and
submits typed requests through Tauri commands.

## 2. System architecture

```text
React review UI
  | typed invoke requests
  v
Tauri command orchestration
  |-- local metadata repository (~/.mallard)
  |-- provider/project discovery and capture
  |-- bundle engine: publish, inspect/lazy stage, plan, apply
  `-- object store
       |-- local folder
       `-- S3-compatible storage / R2
```

The main implementation boundaries are:

| Module | Responsibility |
| --- | --- |
| `src/types.ts` | Frontend DTOs mirroring serialized Rust types. |
| `src/components/project-sync/` | Setup, inventory, Push/Pull review, project-file tree, readiness, and storage UI. |
| `project_sync/domain.rs` | Current IDs, schemas, validation rules, manifests, actions, plans, and receipts. |
| `project_sync/provider_capture.rs` | Provider adapters, generic project scanner, filters, stable identities, and capture. |
| `project_sync/bundle_engine.rs` | Object-key validation, immutable publication, head CAS, tempfile-backed verified-body access, restore planning, streamed backups, and apply. |
| `project_sync/commands.rs` | Tauri command boundary and cross-document transactions. |
| `project_sync/persistence.rs` | Bounded, atomic machine-local JSON persistence. |
| `project_sync/s3_store.rs` | S3/R2 transport with conditional writes, chunk-to-tempfile verified downloads, per-chunk inactivity timeouts, and bounded pagination. |
| `activity_log.rs` | Sanitized local JSONL activity history and retention. |

## 3. Core domain model

A project is represented by four related records:

1. `LocalProjectRegistration` owns the local project ID, portable bundle ID,
   display name, default recipe, per-storage reviewed bases, and revision.
2. `ProjectBinding` maps that project to one canonical checkout and selected
   machine-local Codex/Claude profile IDs. It also has a replica ID and
   revision.
3. `ProjectStorageLink` connects the project to one storage. It owns the
   destination-specific recipe and project-content preferences.
4. `BundleManifest` is the portable full snapshot published for that link. It
   contains no binding or provider-home paths.

One project can link to multiple storages. Each link has an independent
selection recipe and exclusion set. A decision made for storage A does not
change storage B.

Every portable resource has a `ResourceDescriptor` with a stable ID, kind,
scope, display name, portable provenance, apply policy, codec version, and
bounded metadata. Current resource kinds include provider conversations,
memory, requirement placeholders, and ordinary project-content
files/directories.

## 4. Machine-local metadata

Machine state is stored directly under `~/.mallard`:

```text
~/.mallard/
|-- sync_config.json
|-- machine_projects.json
|-- host_identity.json
|-- host_rotation.json
|-- conversation_bindings.json
|-- materializations.json
|-- chat_history_cache.json
|-- project_drafts/<draft-id>.json
|-- setup_transactions/<draft-id>.json
|-- backups/<apply-id>/...
`-- logs/
```

`sync_config.json` contains schema-1 storage definitions, including S3
credentials, project registrations, project/storage links, recipes,
project-content exclusions, and reviewed remote bases. A `RecipeBase` pins the
revision, commit ID, required nonzero serialized manifest size, manifest
SHA-256, recipe revision, binding revision, and last Push/Pull timestamps
reviewed on this replica. Historical manifest reads use the size and hash as
exact transfer expectations.

`machine_projects.json` contains the provider profile catalog and checkout
bindings. Exact and canonical project/profile paths stay here. Resolved
`CODEX_HOME` and `CLAUDE_CONFIG_DIR` values are runtime fields and are not
serialized into portable bundles.

Push and Pull reviews remain in frontend memory. Apply builds current target
pins and engine plans in memory and does not persist them. Materialization
records contain result receipts. Backups are stored per Apply.

Persistence uses bounded reads, schema validation, no-symlink path checks,
revision checks, a process mutex, synced temporary files, and atomic
replacement. On Unix, private metadata paths are permission-restricted. The
process mutex prevents lost updates inside one running app; it is not a
general cross-process database lock.

Project-content preferences have this current shape:

```ts
interface ProjectContentPreferences {
  schema_version: 1;
  revision: number;
  excluded_resource_ids: string[];
}
```

They are stored on the project/storage link and never uploaded. They record
exact entries intentionally left unchecked after a successful Push so later
scans do not select them again automatically.

### Owned child state

The selected main task owns one sync operation. Included children have no
separate resource row, binding, checkpoint, or revision.

| Location | Required fields | Purpose |
| --- | --- | --- |
| Cloud task `files` | `SubagentTranscript { child_key, normalized }` | Declare child objects and each child's normalized content state. |
| Main local binding | `installed_end`, `installed_subagents` | Store the Pull installation cutoff and child key → native ID/cutoff map. |
| Main per-storage checkpoint | `accepted_subagents` | Store each child's accepted local and normalized state together. |
| Temporary Push review | `subagent_selection` | Fix child keys, omission choice, and estimated bytes. |

A child content state contains `message_count` and `sha256`. It uses the
shared `mallard-subagent-tail-v1` / `mallard-subagent-tail-wire-v1` labels
and the existing whole-thread comparator. The object byte hash stays
separate. Raw creation IDs form owner-scoped keys; no child-key hash or
aggregate tree hash is added. No mode field or Mallard boundary enters a
portable rollout.

Codex Pull allocates a fresh native ID for the main and each listed child. It maps
included structural references and retains omitted native references. The
binding map recovers installed membership. Discovery scans past each
installation cutoff for new children and checks authoritative parent metadata.
Claude supports direct children only. A parent tool-result record supplies the
child key and agent ID. Each child record must match its parent session ID,
filename agent ID, and native sidechain flag. Child lookup uses the exact parent
directory. Claude Pull keeps the agent ID and allocates a new parent ID for
Create local or Keep both. An update retains the installed identities.

Missing accepted local children keep comparison Unknown and block Push. Pull
can offer Keep both when the main, other children, and cloud evidence are valid.
Temporary review pins retain the missing-key set and recheck it at Apply. The
existing binding and checkpoint maps hold all durable child state.

Missing fields are rejected under schema 1. There is no migration or legacy
execution path. Full details and measured limits are in the
[Codex verification](docs/CODEX_OWNED_SUBAGENT_SYNC_VERIFICATION.md) and
[Claude verification](docs/CLAUDE_OWNED_SUBAGENT_SYNC_VERIFICATION.md).

## 5. Portable storage and schema-1 metadata

Local-folder and S3/R2 storage share one key layout:

```text
.mallard/
|-- .storage.lock                       # local-folder storage only
|-- _storage.json
`-- v1/repositories/<bundle-id>/
    |-- _tag.json
    |-- _head.json
    |-- _objects/<payload-sha256>
    |-- _manifests/<revision>-<commit-id>.json
    |-- _commits/<revision>-<commit-id>.json
    `-- _uploads/                         # reserved; not written
```

`_storage.json` identifies the root as
`{"format":"mallard-storage","layout_version":1}`. `_head.json` is the
only authoritative mutable bundle pointer. It stores schema version 1, bundle
identity, revision, commit ID, manifest key, manifest SHA-256, and update
time. It also stores the required nonzero `manifest_size`, which is the exact
byte length of the one serialized immutable manifest representation.

Current and historical manifest reads stage against both the recorded size
and SHA-256 before parsing. `manifest_size` has no default and is not inferred
from S3 `Content-Length`, an additional `HEAD`, or received bytes. State that
predates the field is rejected under the schema-1 clean cutover; it is not
migrated.

Each immutable schema-1 manifest is a complete live snapshot containing:

- portable bundle identity and optional repository fingerprint;
- the destination recipe;
- capture application/tool/codec versions;
- publishing-origin tokens and typed, fork-aware conversation entries;
- the selected resource descriptors;
- file entries keyed by validated logical path;
- directory entries keyed by validated logical path; and
- typed resource, file, project-file, and project-directory tombstones.

A file entry stores its owner resource ID, SHA-256, byte size, source mtime,
safe mode, and immutable object key. A directory entry stores its resource ID,
safe mode, and source mtime and owns no byte object. A commit record points to
the manifest, links the previous commit ID, and records file delta counts.
`_tag.json` is a derived listing hint; failure to refresh it after a successful
head CAS does not invalidate the revision.

Remote metadata can contain selected conversations and user-authored project
content. It does not contain checkout paths, provider-home paths, local
profile/link/replica IDs, S3 credentials, local exclusions, restore plans,
receipts, backups, or trust decisions. Scanner-only metadata keys beginning
with `_local_` are removed before descriptors enter the manifest.

## 6. Setup and ordinary resource discovery

Project setup is resumable. A setup draft holds selected paths, profile and
storage choices, preallocated IDs, selected resource IDs, and a discovery
signature, but never discovered file bytes. Finalization first writes a
deterministic setup transaction and then reconciles profiles, storage,
registration, links, and binding, making retries idempotent.

Normal discovery inventories the provider-owned resources for the selected
checkout and profiles. It does not scan arbitrary project content. Provider
adapters map source files to portable namespaces such as `state/codex/...`,
assign apply policies, filter credentials, and convert machine-specific state
into portable definitions where supported.

Remote matching uses the portable bundle ID and, when available, a SHA-256
repository fingerprint derived from Git origin data. The checkout binding
remains machine-local.

## 7. Optional non-Git project-file sync

### 7.1 Eligibility

Ordinary project files are eligible only when the canonical project root is
not inside a Git work tree. The backend runs `git -C <root> rev-parse
--show-toplevel` and returns `eligible`, `git_managed`, or `unknown`.

The backend is authoritative. It checks eligibility during scan, immediately
before reviewed Push capture, while building the Pull plan, and again before
applying any approved project-content action. If Git is initialized after a
scan, Push fails before publication. If stored project content exists for a
binding that is now Git-managed, the UI shows a locked Project files step so
the remote actions are visible but cannot be selected.

### 7.2 Scan behavior

The review order is:

```text
Git & sessions -> Project files -> Review
```

The Project files step is the opt-in. Opening Push does not scan the ordinary
tree. `inspect_project_files` performs the explicit scan and returns a typed
inventory, eligibility result, comparison state, warning data, preference
revision, and review token. Scanning does not publish or modify saved config.

The scanner:

- walks without following links and sorts entries deterministically;
- tracks every real file and directory as an independent resource;
- includes empty directories;
- excludes nested registered projects and paths owned by existing provider
  adapters;
- excludes Mallard metadata, every configured local storage, and mapped
  provider homes;
- honors `.gitignore`, `.ignore`, and `.mallardignore`;
- hard-excludes VCS directories, common build/cache output, and known
  credential filenames;
- blocks symlinks, Unix hard links, special files, private-key material,
  unsafe paths, oversized files, and excessive depth; and
- flags executable or credential-shaped opaque content for explicit review.

Secret detection is deliberately best-effort. A clean scan is not a guarantee
that content contains no secret.

### 7.3 Identity, paths, and metadata

The normalized project-relative path is the stable identity input:

```text
file ID      project:content-file:<sha256("file\0" + relative-path)>
directory ID project:content-dir:<sha256("dir\0" + relative-path)>
logical path project/<relative-path>
```

The bound project root itself is not a resource. For
`docs/specs/a.md`, `docs`, `docs/specs`, and `docs/specs/a.md` are three
separate resources. Selecting the file requires both directory resources.
Directory selection is a bulk convenience over currently discovered entries;
it is not a wildcard for future children.

The comparison digest includes entry type, relative path, file-content hash
when applicable, and safe mode. Source mtime is retained for display and
best-effort restoration but is not part of this comparison digest. Ownership,
set-id bits, ACLs, xattrs, creation time, and platform-specific flags are not
portable. Modes are restricted to `0o777`.

### 7.4 Three-view comparison and selection

The Push inventory compares:

- `local`: the explicit scan result;
- `storage`: the current schema-1 head manifest; and
- `base`: the exact revision last reviewed by this binding/link.

Entries are classified as `synced`, `local_only`, `local_ahead`,
`storage_only`, `storage_ahead`, `diverged`, `missing`, `blocked`, or
`unknown`. An unavailable historical base forces relevant entries to
`unknown` instead of guessing.

After Scan, a newly discovered eligible entry is selected in the pending
review unless its stable ID is in the link's exclusion set. Existing recipe
entries remain selected. Selected descendants force their ancestor directory
entries to remain selected. This state is still transient until Push
succeeds.

The review token binds the local project and replica IDs, binding revision,
eligibility, preference revision, remote revision/commit/manifest hash, and
every reviewed entry state/digest/warning. Push rescans and recomputes it.
Changed bytes, modes, warnings, binding, preferences, or remote head require a
fresh review. Warning acknowledgements are separately bound to the resource,
review digest, and warning code.

## 8. Push protocol and metadata updates

Push executes in this order:

1. Load and validate the registration, storage link, binding, provider
   profiles, current revisions, and Codex conversation-path readiness.
2. Discover normal project/provider resources.
3. Preserve every stored project-content recipe entry unless the request has
   an explicit removal ID.
4. If generic content changes, require a current scan token, recheck
   eligibility, verify warnings, reject blocked/disappeared new resources,
   and insert required directory ancestors.
5. Capture exactly the final recipe. Selected content is revalidated,
   re-read, size-bounded, and hashed.
6. Compare the current remote head with the local `RecipeBase`. A missing,
   unknown, stale, or binding-mismatched base blocks publication.
7. Upload selected file objects, then the immutable manifest and commit.
8. Compare-and-swap `_head.json` once from the Apply-time ETag to the new
   revision.
9. Best-effort refresh `_tag.json`.
10. Guardedly update machine-local metadata.

Publication writes immutable objects before the mutable head. A failed CAS can
leave unreachable upload/manifest/commit objects, but readers cannot observe a
partial revision. Local-folder storage serializes publication with
`.mallard/.storage.lock`. S3/R2 uses `If-None-Match` or `If-Match`; an
ambiguous response is resolved by rereading the head and comparing bytes.

### Successful Push: remote update

The remote gains a new revision containing the full selected resource set,
file objects, directory entries, and any typed tombstones. Removing a selected
resource from the recipe is not enough for ordinary project content: the
command preserves stored entries unless its resource ID is explicitly listed
for removal.

### Successful Push: local update

After head publication, one guarded config mutation:

- saves the destination recipe on the project/storage link;
- updates the link's exact exclusion set and increments its preference
  revision only if those preferences changed;
- writes the new `RecipeBase` revision, commit, exact manifest size,
  manifest hash, recipe revision, binding revision, and Push timestamp; and
- increments the project/config revisions.

Scan, cancel, capture failure, upload failure, and head-CAS conflict do not
change the saved recipe or exclusions. There is one unavoidable distributed
boundary: if the remote head succeeds and the later guarded local mutation
detects a local revision race, the remote revision remains valid while the
command reports that local state must be refreshed.

### Per-conversation failure isolation

Conversation actions are built one thread at a time, and a thread commits
into the publish request only after its last fallible step. A failure whose
inputs belong to one thread (its captured bytes, reviewed pins, path
mapping, redaction, sidecars, or stamps) fails that thread only: the request
carries the thread's previous stored row untouched, its checkpoint does not
move, and every other selected thread publishes in the same revision. The
result lists each such thread under the review's resource id with
`state: "failed"` and its message, or `state: "held"` when the apply-time
review found that a Pull has to resolve it first. `success` is false when any
thread failed, and the message reports the count. Nothing about a failed
thread is stored: the next Compare classifies it from its unmoved pins and
offers it again.

Failures whose inputs are shared (storage transport, the head
compare-and-swap, config and binding loads, provider availability, and batch
invariants) still stop the whole Push before publication.

## 9. Explicit removal and tombstones

Local absence is not deletion intent. If Machine A deletes
`docs/file_a`, Scan reports the path as missing, but Push keeps the stored
entry unless A chooses **Remove from storage** for that exact resource.

An approved storage removal produces:

- a `project_content_file` tombstone with its logical path and last byte
  SHA-256; or
- a `project_content_directory` tombstone with its exact logical path.

Removing a directory does not imply recursive removal. Stored descendants
must each be removed, and the backend rejects a parent-directory removal while
stored descendants remain selected. A rename is represented as removal of the
old exact resource plus addition of the new exact resource.

## 10. Pull comparison, Apply, and metadata updates

Pull has a comparison review followed by one mutation command.

### 10.1 Compare and lazy staging

Inspect validates the bounded storage marker and head, then stages the
manifest against the head's exact `manifest_size` and `manifest_sha256` before
parsing and validating schema, bundle identity, logical paths, resource
ownership, case-folded path collisions, directory ancestry, and tombstones.
Compare maps portable logical paths to the current binding without eagerly
downloading a whole bundle. It does not capture complete local transcripts or
persist an executable plan.

Payload bytes are staged only when a consumer needs them. The manifest
entry's exact size and SHA-256 are passed to the storage adapter, which
returns `Missing`, `Mismatch`, or a tempfile-backed verified body. Pull
prefetch stages only the selected paths; the durability audit explicitly
stages every object. There is no byte-returning object-store GET,
`FetchedBundle`, `PreloadedBytes`, or eager fetch/planning/apply wrapper.

Project content uses four explicit action kinds:

- `write_project_file`;
- `ensure_project_directory`;
- `delete_project_file`; and
- `delete_project_directory`.

Every project-content action requires approval and starts unchecked in the
UI. Selecting a nested file also selects its planned ancestor directories.
Unchecked actions mean **Keep local / skip this Pull** and do not block
approved conversations.

The in-memory review pins the storage, bundle, replica, revision, commit,
manifest hash, binding revision, and expiry. Apply rebuilds target digests,
modes, conversation identities, and executable actions from current state.

### 10.2 Apply

Before each mutation, Apply rejects unknown or duplicate selection IDs, an
expired review, changed bindings, changed remote heads, changed payload bytes,
path escapes, link traversal, changed target hashes or modes, and newly
Git-managed project roots when a project-content action was approved.

Apply uses this filesystem order:

1. Prepare approved directories shallowest-first with a temporary
   owner-writable mode.
2. Apply approved file writes by streaming verified bodies into
   destination-parent synced temporary files and atomically renaming them;
   stream and verify an immutable backup of an existing target first.
3. Apply approved file deletions deepest-first after digest/mode revalidation
   and a verified immutable backup.
4. Apply approved directory deletions deepest-first, but only when each exact
   target is a real empty directory.
5. Finalize prepared directory modes and best-effort mtimes deepest-first.

Thus `project/docs/specs/a.md` maps to
`<canonical-project-root>/docs/specs/a.md`; Pull creates `docs` and
`docs/specs` before writing `a.md`. It never creates an unrelated `/spec`
directory.

If Machine B changed a file covered by a deletion tombstone, its digest no
longer matches and deletion fails without removing B's bytes. If B has an
untracked `docs/local.md`, approved tracked child files may be deleted, but
`docs` remains because recursive deletion is forbidden. A failed directory
removal is recorded and a later attempt requires a new comparison.

### 10.3 Successful Pull: local and remote metadata

Pull never changes remote storage. Locally, it records one materialization
with applied, skipped, failed, and blocked receipts plus any backup paths. If
the file phase is complete, including explicit keep-local decisions recorded
as skipped actions, it updates the link recipe to the pulled manifest recipe
and advances the `RecipeBase` with the inspected head's exact manifest size
and hash. Project-content exclusion preferences are not rewritten by Pull.

Apply is followed by readiness verification. A readiness failure does not
hide the current Pull result from the user.

Conversation actions settle one thread at a time. A thread that fails to
prepare or apply, a thread whose local capture failed, and an approved row
the apply-time review no longer offers as the reviewed action each get a
failure receipt of their own while every other approved action applies; the
result lists them under `failed_actions`. Shared-state failures (storage
transport, the batched transcript prefetch, config and binding loads,
provider availability) still stop the whole Apply.

## 11. Integrity, concurrency, and path safety

All IDs and cloud namespace components use bounded validated grammars. Bundle
IDs are opaque lowercase 128-bit hex values. Logical and object paths must be
relative, normalized, and free of empty, dot, parent, backslash, or unsafe
platform components. Manifests reject file/directory type collisions and
case-insensitive path collisions.

SHA-256 binds:

- each uploaded file object to its manifest entry;
- the exact serialized manifest size and SHA-256 to `_head.json`, with the
  digest also retained by the commit record;
- reviewed local/storage/base content comparisons; and
- warning acknowledgement and Push review tokens.

Local-folder and S3/R2 reads share one staged contract. Both copy into a
private `NamedTempFile` while incrementally counting and hashing. S3/R2 checks
declared `Content-Length` before polling when present, rejects an overrun
before writing the excess chunk, and uses a 120-second inactivity timeout for
each next-chunk wait rather than a whole-body deadline. Active large transfers
can therefore exceed 120 seconds without failing solely because of total
duration. Bounded bulk staging preserves request order and cancels/drains
siblings after a transport failure.

Restore target construction starts at the canonical bound root, performs a
safe lexical join, resolves prospective canonical paths, and inspects every
existing ancestor without following symlinks. Writes never use a remote
absolute path.

Head CAS provides single-winner publication. A pusher must hold the exact
reviewed `RecipeBase`, and a Pull review must still match the current head.
The immutable commit chain retains published history, while stale writers fail
and must inspect/review again.

A local storage directory must not overlap `~/.mallard`, a project checkout,
a provider profile, or another configured local storage. Filesystem lock files
protect writers that see the same local filesystem; sync services such as
iCloud or Dropbox do not provide a reliable distributed lock for simultaneous
multi-machine Push.

## 12. Current limits

| Limit | Value |
| --- | ---: |
| Discovered resources | 20,000 |
| Generic project-content depth | 32 components |
| Conversation file size | 100 MiB |
| Other captured file size | 16 MiB |
| Aggregate selected capture | 512 MiB |
| Manifest resources | 20,000 |
| Manifest files/directories/tombstones | 100,000 each |
| Restore actions | 100,000 |
| Logical path length | 1,024 bytes |
| Ignore rules | 8,192 |
| Restore-plan lifetime | 24 hours |
| Project-file rows rendered at once | 500 |

The UI uses path search and collapsed directories for inventories larger than
the render window. Backend validation remains authoritative for the full set.
There is no generic object read/write ceiling: manifest sizes are exact
integrity coordinates, while conversation/provider/project-content and
aggregate capture limits remain product policy.

## 13. Current automated coverage

The project-content backend tests cover stable IDs, nested and empty
directories, ignore/exclusion behavior, credential blocking, symlink and hard
link rejection, file-size/depth limits, the 20,000-resource stress boundary,
review-token/warning freshness, a Git-after-scan race, a two-machine nested
round trip, explicit typed tombstones, divergent deletion blocking, verified
backups, and non-recursive directory deletion with an untracked child.

Frontend integration tests cover conditional tab visibility and order,
explicit scan opt-in, new-entry default selection, required ancestor
directories, Pull's default keep-local behavior, empty approved-action
submission, and locked remote project content on a Git binding.

Use the current verification commands:

```sh
npm run build
npm run test:frontend-integration
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Verification on 2026-07-28 completed successfully: the production frontend
build and Rust check passed, all 99 frontend integration tests passed, and all
290 Rust library tests passed.

Project-content end-to-end command coverage currently uses local-folder
storage. The S3 adapter and bundle protocol have automated transport coverage,
but a live R2/S3 project-content smoke test remains an external release check.

## 14. Current behavioral boundaries

- Ordinary project-file sync is intentionally unavailable inside any Git work
  tree; it is not tied specifically to GitHub hosting.
- Project content is exact-entry sync, not a future-child folder rule.
- Scans are explicit; no background watcher adds files to a recipe.
- Pull project-content actions are never selected automatically.
- Directory deletion is exact and empty-only; recursive deletion is not
  implemented by design.
- Secret scanning cannot certify arbitrary user content as safe.
- Portable filesystem metadata is limited to file bytes, safe POSIX mode, and
  best-effort source mtime.
- A Push publishes a new immutable snapshot from the selected recipe; storage
  history is bundle versioning, not source control.
