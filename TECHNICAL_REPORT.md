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
| `project_sync/bundle_engine.rs` | Object-key validation, immutable publication ending in the record write, head selection over a stream's records, tempfile-backed verified-body access, restore planning, streamed backups, and apply. |
| `project_sync/commands.rs` | Tauri command boundary and cross-document transactions. |
| `project_sync/persistence.rs` | Bounded, atomic machine-local JSON persistence. |
| `project_sync/s3_store.rs` | S3/R2 transport with create-only writes, single-request record and claim writes with SDK retries off, chunk-to-tempfile verified downloads, per-chunk inactivity timeouts, and bounded pagination. |
| `storage_core/publication.rs` | Shared by both engines: list a stream, select its current publication from record names, read and verify a record, and list-then-write. |
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
`-- v1/repositories/
    |-- _commits/<bundle-id>/<record name>   # every project record, nothing else
    `-- <bundle-id>/
        |-- _objects/<payload-sha256>
        |-- _manifests/<revision>-<commit-id>.json
        `-- _uploads/                         # reserved; not written
```

`_storage.json` identifies the root as
`{"format":"mallard-append-storage","layout_version":1}`. Its bytes are
constant, so concurrent first writers write identical bytes.

Every object is immutable and no conforming writer ever writes different
bytes to the same key, so a provider needs only put, get, and list. There is
no `_head.json` and no tag: the **publication record** is the only thing that
publishes a snapshot, and it is written last. A record name carries its graph
facts,

```text
<revision:010>.<commit_id>-<record_sha256>.<parent token | root>.<created_at_ms:013>.json
```

so the current publication of a bundle is selected from the names of one
listing of `_commits/<bundle-id>/` without reading a record: starting at the
virtual root, the child with the smallest `(created_at_ms, commit_id,
record_sha256)` is taken until a publication has none. Every listed record
that is not on that chain is kept and not selected; nothing marks, moves, or
deletes it. A name outside the grammar (`.DS_Store`, a sync client's
conflicted copy) is ignored and written to the activity log.

The record stores schema version 1, bundle identity, revision (its depth, and
the number the UI prints), commit ID, the parent's token, `created_at_ms`, the
manifest key, manifest SHA-256 and the required nonzero `manifest_size`, the
file delta counts, the publishing origin, and a `summary` with every fact
bundle discovery shows. `BundleHead` is the in-memory view of the selected
record. When a record is read its bytes must hash to the digest in its name
and its fields must equal the facts in its name; a failure is an error for
that publication and never moves selection to another one.

Current and historical manifest reads stage against both the recorded size
and SHA-256 before parsing. `manifest_size` has no default and is not inferred
from S3 `Content-Length`, an additional `HEAD`, or received bytes.

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
safe mode, and source mtime and owns no byte object.

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
6. List the bundle's stream and select. The Push gate runs against the
   freshly selected publication by identity and ancestry, never by revision
   order, and it is decided per writable scope. A scope base counts while the
   publication it was reviewed in is on the selected chain; otherwise it is
   stale and read as absent. The Push goes on when every writable scope is
   absent from the storage without a usable base, or holds exactly the
   reviewed content. A writable scope that changed on the reviewed history,
   or that holds content this replica never reviewed in this history,
   requires a Pull. A storage that was cleared is therefore pushed again, and
   a recreated one is pulled first only when another computer wrote into a
   scope this replica writes; nothing asks for an unlink or a reset. A
   binding-mismatched base blocks publication.
7. Upload selected file objects, then the immutable manifest.
8. Build the publication record in memory and write it with list-then-write:
   list the stream, continue only if the record's parent is the selected
   publication and therefore childless (a stream that selects nothing for a
   first publication), and send the record once. A listing older than ten
   wall-clock seconds is taken again before sending; a retry is a new
   attempt that lists first, never a resend; nothing is journaled.
9. List once more. The Push goes on only if that listing selects this record;
   if it selects another computer's, the Push fails with `RemoteChanged`.
10. Guardedly update machine-local metadata.

Publication writes immutable objects before the record. A Push that stops
before its record leaves unreferenced objects and a manifest, and readers
cannot observe a partial revision. The write path ends in one of: `Stored`
(written and selected), `NoChange`, `RemoteChanged` when another computer
published on the same parent first (compare again), or `OutcomeUnknown` when
the storage cannot be listed after the record was sent. `RemoteChanged` has
two causes: the listing before the write shows another child or another
selection and nothing is written, or the listing after the write selects the
other computer's record and this one stays in the storage, never selected.
Either way the Push failed as a whole and no machine-local metadata moved.
A sibling of a publication can land after that publication became visible:
another computer listed before this record was visible and wrote a record
that carries an earlier timestamp, usually within seconds and in principle at
any later time. The earlier-ranked sibling is selected and the other is kept;
a Push that reads that from its confirming listing failed, reads `Advanced`
on its next refresh and pushes again. A sibling that becomes visible only
after a Push was confirmed is reported by the next observation as
`Displaced`: nothing is rolled back, the bases and the acceptance saved at
the outranked publication are stale and read as absent, a standing notice
says that this computer's last push (or last pull) is no longer part of the
history, and the user compares and pushes again under the same cloud ids.
`Replaced` is left for a stream that was cleared or recreated, and
`RemoteMissing` for one that was cleared and not pushed to since. Neither
blocks anything. A scope base or a session acceptance that names a
publication off the selected chain is stale: every comparison reads it as
absent, no refresh deletes it, and the Push or the Pull that saves new bases
removes the stale ones. A session whose acceptance is stale or missing is
compared from content: a storage without the session takes it under the cloud
id it already has; an equal or an earlier stored copy is proven by
first-contact adoption; a longer or a different stored copy is never
replaced, Push asks for a Pull, and Pull keeps both copies. While a history
reads cleared, the activity page carries one standing note, "Another computer
cleared <storage>. Push again." or "… Pull, then push again."
Local-folder storage serializes its own writes with `.mallard/.storage.lock`.
S3/R2 writes create-only with `If-None-Match: *`; the record write runs with
SDK retries off. Dropbox writes a record with one `files/upload` request.

### Successful Push: remote update

The remote gains a new revision containing the full selected resource set,
file objects, directory entries, and any typed tombstones. Removing a selected
resource from the recipe is not enough for ordinary project content: the
command preserves stored entries unless its resource ID is explicitly listed
for removal.

### Successful Push: local update

After the record is stored, one guarded config mutation:

- saves the destination recipe on the project/storage link;
- updates the link's exact exclusion set and increments its preference
  revision only if those preferences changed;
- writes the new `RecipeBase` revision, commit, exact manifest size,
  manifest hash, recipe revision, binding revision, and Push timestamp; and
- increments the project/config revisions.

Scan, cancel, capture failure, upload failure, and a changed remote do not
change the saved recipe or exclusions. There is one unavoidable distributed
boundary: if the record is stored and the later guarded local mutation
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

Failures whose inputs are shared (storage transport, a changed remote at the
record write, config and binding loads, provider availability, and batch
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
- the exact serialized manifest size and SHA-256 to the publication record,
  and the record's own bytes to the digest in its name;
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

List-then-write and earliest-sibling selection provide single-selection
publication without a conditional write; on S3/R2 and Dropbox a create-only
claim of the parent, written before the record, admits one child per version,
so a listed version there never loses while the storage's claims are intact.
A Push sends one claim request and one record request, settled by one
read-only check each when unacknowledged, and never resends; a push that
claimed a version and never wrote its record is finished only by the user's
*Finish that push*. A pusher's base must be the selected
publication or one of its ancestors, and a Pull applies exactly the
publication it reviewed while that publication is on the selected chain: a
newer one pushed since shows afterwards as changes to pull, and a review whose
publication lost its race or was cleared is refused before anything changes
("Compare again"). A Pull whose storage cannot be read after its download
stops: what it installed stays, the rest reads "Not pulled", nothing is rolled
back or retried, and no Pull surface offers a local-state reset. The
hash-linked record chain retains published history,
including publications that were not selected, while a writer that lists a
child under its parent stops and must inspect/review again.

A local storage directory must not overlap `~/.mallard`, a project checkout,
a provider profile, or another configured local storage. Filesystem lock files
protect writers that see the same local filesystem. No distributed lock is
needed for simultaneous multi-machine Push: two racing records are both kept,
every computer selects the same one, and the Push that wrote the other fails.

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

## 15. Behavior reference

Session mapping, sync safety, owned children, refresh, and Pull preparation in
detail.

### How session-to-commit mapping works

Mallard parses provider session archives locally to recover session metadata,
activity windows, recorded branch provenance, and recorded commit references.
A session's interval runs from its first user prompt to its last agent reply.
For a Git project, every selected local branch shows the same project-session
set for the loaded time range, and each session is placed on exactly one
commit of that branch's first-parent history: the closest commit at or after
the session's **last user input**, read from the provider's JSONL transcript.
A session with no commit after its last input is listed under
**Uncommitted**. The session's timestamp tooltip shows the time of that last
input.

Placement uses time only. A provider-recorded branch or commit never controls
visibility, Push, or Pull.

The mapping is rebuilt from `git log` whenever Activity loads: when the app
opens, on **Reload app data**, on the refresh button, and when the open
project's `HEAD`, branch refs, or `packed-refs` change outside Mallard, for
example after a checkout or a commit in a terminal. Reloads follow the
checked-out branch unless another branch was picked in the branch menu. See
[docs/DESIGN_GIT_SESSION_MAPPING.md](docs/DESIGN_GIT_SESSION_MAPPING.md).

### Push, Pull, and project-file safety

- Every Push and Pull is reviewed by resource before it changes shared or
  local state.
- A conversation that fails during Push or Pull fails alone: the other
  selected conversations still publish or apply, and the result names the
  failed one with its reason.
- Mallard captures only provider-home state; it never captures configuration
  or instruction files from inside a Git-managed project directory. That
  content travels with Git.
- Non-Git project-file scans are explicit and read-only. `.gitignore`,
  `.ignore`, and `.mallardignore` are honored.
- VCS data, build output, Codex and Mallard metadata, symlinks, and special
  files are excluded or blocked. Size and path limits still apply.
- Non-Git project files can include credential files, private keys, and
  hard-linked regular files. Credential filenames, detected secrets, and
  executable content require an explicit warning acknowledgement.
- Provider-home capture keeps its credential-file and hard-link exclusions.
- Pull deletions begin unselected. Approved file deletion verifies the local
  digest and creates a backup; approved directory deletion removes only an
  exact empty directory and is never recursive.
- Absolute checkout paths, Codex-home paths, storage credentials, restore
  plans, receipts, backups, and local preferences stay on the machine.
- A Mallard Cloud repository that was removed on the website, is being
  deleted, or is out of the account's reach reads as empty: every session
  shows its own state, Push and Pull stay open, and a Push creates the
  repository again at the address it had. Nothing is unlinked, reset or
  relinked, and a refresh saves nothing; a Push or a Pull saves the
  repository it used on the link. Any other answer, such as a suspended
  account, a rejected sign-in or a service that cannot be reached, never
  reads as empty and keeps Push and Pull closed.
- The Owner or an Admin of a Mallard Cloud repository, private or public, can
  discard its draft from the Mallard Cloud chip's panel (**Discard draft…**),
  after a confirmation: every push after the latest version is removed from
  Mallard Cloud and the draft is that version again; with no version, every
  push goes and the repository stays, empty. The row stays in view and is
  off, with its reason, while there is nothing to remove. Nothing on any
  computer is deleted. The website's Versions tab offers the same act.

Settings > Sync has separate **Push Codex children** and **Push Claude children**
switches. Both default to Off and control the next Push. Each cloud task carries
an On/Off child stamp: Compare uses the cloud stamp, while Push uses this
computer's setting. Cloud Off compares only the parent. Cloud On includes
supported owned children in Compare. A stamp difference permits reviewed Push
for otherwise synchronized content, including a task with no children.

Pull restores exactly the child files listed in the cloud and keeps local
settings unchanged. Claude supports direct children only and retains agent
IDs inside the destination parent directory. Inherited children stay outside
the task's sync scope. With Push enabled, one unsupported owned child omits
all children and shows an information icon in review. A missing required
child blocks Push. If that accepted child exists in storage and all other
evidence is valid, Pull offers Keep both to restore the cloud copy and retain
the local files.

A Claude task is updated in place, so Pull can meet child files on this
computer: those an earlier Pull installed, and those Claude wrote here. Pull
replaces such a file only when the cloud lists that child, and only in the
state the review saw. Any other file in the place of a listed child stops
that task. A child that Claude wrote here stays a child of the task when the
cloud version does not list it. A copy from an earlier Pull does not.

Child changes use the same comparison rules as main changes, and a change on
this computer beside a change in the cloud reads as diverged. One case is
different. A child that only this computer has stands against nothing in the
cloud when the task is updated in place, which is so for Claude: the task
reads as behind the cloud, Pull installs what the cloud holds and leaves that
child where it is, and the next Push stores it. So the children that two
computers made of one task come together in one task. A main conversation or
a child that was continued on this computer would be replaced by that Pull:
beside a change in the cloud it reads as diverged, and Pull offers Keep both.
A Codex update is written under a new identity and would leave the child
behind, so for Codex every such case reads as diverged.

The session page shows a card for each call of a child agent. It leaves out
the card of a child that came with a Pull without its transcript. A child
that was made on this computer has its card, also in a task that was pulled
without its children.

Review fixes child membership; Apply captures the latest bytes. A source
change after capture fails that task with a request to Push again. Completed
writes remain after an Apply error; there is no rollback of the task tree.

This is a schema-1 clean cutover with required new fields. Use fresh Mallard
state and empty test storage. See the [owned-child design](docs/PLAN_CODEX_OWNED_SUBAGENT_SYNC.md)
and [verification results](docs/CODEX_OWNED_SUBAGENT_SYNC_VERIFICATION.md). Claude scope and validation are in
the [Claude design](docs/PLAN_CLAUDE_OWNED_SUBAGENT_SYNC.md) and
[Claude verification](docs/CLAUDE_OWNED_SUBAGENT_SYNC_VERIFICATION.md).

Refresh reads a session from the place Mallard saved in its file, and only
what was added after it. The saved place is used only on the same file, and
only while that file is untouched or has grown. A session file that was
rewritten, replaced, or shortened is read in full and judged by its content:
an edited session shows as diverged, and Pull offers Keep both. A pulled
session, and a session whose file was copied, is read in full each time it
changes until it is next pushed.

Refresh cannot see an edit that arrives together with new turns, because it
reads only the new turns. Push reads the whole session. It stops that task
once, says that earlier messages changed, and switches the saved place off.
The next Refresh reads the session in full and shows it as diverged. An edit
is a change to an earlier message. A rewind is not an edit, and neither is
the rewrite the Claude desktop app makes when it imports a session. Child
session files are read from their own saved place, under the first rule.

### Architecture

Mallard is a Tauri 2 desktop application. React, TypeScript, and Vite provide
the review-focused interface; Rust handles discovery, validation, hashing,
safe filesystem operations, local persistence, and S3/R2 transport.

Every storage shares the same versioned bundle model, and a storage needs
only three operations: write bytes at a key, read a key, list a prefix. A Push
writes immutable payload objects, an immutable manifest, and last an immutable
publication record. There is no mutable head: the current version is selected
from the names of those records, the same way on every computer, with SHA-256
checks on everything read. When two computers push on the same version within
seconds, the earlier record is selected. The other Push counts as not made:
it fails, or, if it had already been confirmed, the next refresh rolls it
back and says so. Pushing again publishes the same content, and the losing
record is kept in the storage and never selected.

A storage that another computer cleared never asks for an unlink or a reset.
What this computer wrote down about its last sync names a history the storage
no longer has, so Mallard ignores it without deleting it and compares directly.
Push is open when nobody has pushed since, and one Push recreates the storage
under the ids the sessions already had. When another computer pushed first into
an area this computer also writes, a Pull comes first. A session both computers
hold is matched from its content: an equal copy is marked as synced, an earlier
stored version is updated by Push, and a longer or a different stored copy is
never replaced. Pull then keeps both copies. The activity page says which case
applies: "Another computer cleared <storage>. Push again." or "… Pull, then
push again."

### Pull preparation, credential redaction, and path conversion

Pull allocates Codex thread IDs locally during Apply. It uses the existing
random version-7 format, validation, and bounded collision checks. ID allocation
starts no Codex process and makes no API request. The schemas remain at 1.

Pull prepares cloud transcripts in a bounded worker queue. It uses the existing
local worker setting, which defaults to four, and a 64 MiB source-byte budget.
Running jobs and completed results keep their reservations until installation
finishes. One transcript above that budget runs alone if the existing file
limit permits it. Installation follows action order. Source checks, provider
index changes, bindings, checkpoints, and receipts stay in that serial step.

Codex preparation uses downloaded bytes directly. It does not write and read
an intermediate transcript file. Both providers analyze cloud content and the
final local content once each. An immutable result binds the local bytes to
their digest, provider, codec, thread ID, project root, lineage, and semantic
digest. Materialization checks those facts against the current operation and
binding. It verifies the staged SHA-256 without another semantic analysis.
The output layout, path-map, redaction, shared-index, and destination checks
remain in place. A preparation or installation failure affects that thread.

Settings > Privacy controls **Credential redaction** for this machine. It is
Off by default. On replaces detected credentials in exported sessions, child
sessions, sidecars, provider-state content, and titles. Off preserves that
content. Exported content can then include API keys, tokens, and passwords.
Credential-file exclusions and diagnostic sanitization remain active.

Push review shows the selected mode. A setting change before apply requires a
new review. A save attempted during Push apply returns busy; it does not change
the saved mode or the active Push. Retry the save after Push completes.

A selected current session with a different stored mode publishes a complete
new revision under its existing cloud ID. Pull and first-contact comparison use
the stored revision's mode, independent of this machine's outgoing setting.
A mode change does not purge comparison caches. On cannot remove content from
older immutable Off revisions. Off cannot recover text already replaced with
placeholders. All schemas remain at version 1; there is no migration path.

Conversation path conversion preserves literal marker text and destination
paths. Tagged references retain absolute, alias, tilde, and URI spellings.
On return to the same bindings, conversation text is unchanged except for
credential redaction when enabled. Restore IDs and record layout can change.

Local bindings reference immutable path-origin files. These files hold strict
record hashes and inverse edits. Retained history uses those edits on later
Push; only new records are scanned. Missing or stale referenced data fails
that thread. Origin files stay in the Mallard home and never enter a portable
manifest. Metadata reset retains this local history data. The format remains
schema 1 and requires a clean test-data reset for this cutover.
See [codec details and formal checks](formal/pathmap-origins-v1/README.md).

Apply phase logs report cloud verification, reconstruction, output verification,
and serial stage/commit time. The preparation total includes its three worker
phases. Worker durations can overlap each other and serial installation; do
not add them to estimate wall time. The pipeline phase reports wall time and
peak admitted jobs and source bytes. The source-byte budget is not a total
memory limit: parsed records, output bytes, and existing capture state use
additional memory.

### Pull source checks and copy-on-write materialization

Pull uses the local snapshot captured at the start of the Apply attempt as its
fixed comparison baseline. Preparation uses that captured digest. After cloud
verification, one bounded source read and SHA-256 comparison runs before each
existing thread is installed. A changed, missing, or unreadable source fails
that thread. Cloud-only creation and metadata-only adoption need no source check.

The rule covers normal updates and approved Keep both actions. In-place
replacements can overwrite a change made after the check. Their backups use
the bytes returned by that check, with no further live transcript read.
Cloud and staged payload verification, destination collision checks, path,
file-type and size checks, approval checks, and identity and binding revision
checks remain. Shared provider files, including the Codex session index, retain
their digest checks through commit. A later write or shared-file conflict can
still fail the thread. Only successful installation advances its accepted
checkpoint; successful sibling threads remain applied.

Source-check cost is linear in the total bytes read. The check adds at most
one transcript buffer. Checked bytes are kept only for the backup or the
retained-source checkpoint, then released. There is no additional cloud
request, wait loop, or automatic retry. Internal checked bytes are not
serialized; schemas remain at 1 without a migration or legacy execution path.

Codex `MaterializeCloud` is always copy-on-write. A native fork can begin
referencing an unchanged parent between review and apply, so neither a digest
pin nor a dependency rescan can make replacement of that parent atomic with
Codex's external writer. Pull instead assigns the standalone history a fresh,
reviewed-absent alias, preserves the prior rollout bytes, and inserts the new
index entry without clobbering the old one. Binding/receipt history suppresses
no content by itself: it derives a domain-separated retained-branch identity,
and an immutable whole-file checkpoint hides the displaced physical alias only
while its exact bytes still match the reviewed source. Missing evidence, a
read failure, or any later append fails open and surfaces that retained branch
in review, Activity, and Session Detail; publishing it creates a direct
binding. Revision 11 proves this for a referenced root, revision 8 proves the
same copy-on-write rule for an unreferenced P2 control, and a focused backend
regression exercises a changed retained branch plus repeated R1-to-R2
displacement. Providers without Codex's physical rollout graph retain their
existing in-place update behavior.

The native POC also proved that
`history_base.end_ordinal_exclusive` is a global top-level rollout boundary,
not a physical JSONL row count: the child `session_meta` starts at that ordinal
and subsequent rows are contiguous. Mallard now enforces those current
paginated ordinals and decodes portable Codex display titles on Pull in the
single schema-1 path. Neither correction adds a fallback, migration, old-format
path, acceptance of ordinal-less paginated data, or schema change. Codex's
upstream `history_mode: "legacy"` remains its separate current ordinal-less
source mode, not a Mallard legacy execution path.
