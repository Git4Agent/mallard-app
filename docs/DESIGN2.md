# Agent Sync — Design v2

> **Legacy schema-2 design.** The active project-scoped schema-3 architecture
> is defined in [`PLAN_PROJECT_SCOPED_SYNC.md`](./PLAN_PROJECT_SCOPED_SYNC.md).
> Schema 3 uses project bundles, persistent resource recipes, machine-local
> bindings, and `.mallard/v1/repositories/`; it does not migrate or reuse the
> whole-profile identities described below.

Supersedes `DESIGN1.md`. What changed structurally from v1:

- Publishing is a CAS flip of one tiny pointer object, `_head.json`. All
  profile history (`_manifests/`, `_commits/`) is immutable and written
  *before* publish, so every published revision provably has its manifest
  and commit — no repair passes, no history holes.
- The local baseline records only entries actually applied on this machine,
  eliminating phantom `local deleted` prompts from excluded or skipped paths.
- Explicit SQLite handling on pull (stale WAL sidecars are a corruption
  vector) and snapshot-based uploads.
- Cleanup gains a retention policy and age thresholds that close races with
  in-flight pushes.
- Remotes without conditional writes run in an explicit **single-writer
  mode** with multi-writer guarantees disabled, not approximated.
- Multi-writer conflicts resolve as a deterministic **local union** instead
  of blocking the push: push downloads the cloud side, merges locally, and
  publishes the union; pull applies the same union without publishing. See
  Union Conflict Resolution.
- Tightened path validation (symlink-aware directory creation, collision
  rules, untrusted sizes), put-if-absent profile creation, defined
  ambiguous-CAS recovery, per-remote exclusion opt-ins, backup retention.

## Contents

1. [Overview](#overview) — summary, goals, non-goals, core model
2. [Configuration](#configuration) — remotes, profiles, endpoint resolution
3. [Cloud Layout](#cloud-layout) — bucket structure and metadata files
4. [History Retention And Cleanup](#history-retention-and-cleanup)
5. [Local State](#local-state) — baselines, backups, staging
6. [Sync Semantics](#sync-semantics) — eligibility, state matrix, caching
7. [Push Flow](#push-flow) · [Pull Flow](#pull-flow) · [Setup Flow](#setup-flow)
8. [Safety Rules](#safety-rules) — paths, symlinks, SQLite
9. [Interfaces](#interfaces) — UI, commands, transport contract
10. [Error Handling](#error-handling) · [Security Notes](#security-notes)
11. [Implementation Order](#implementation-order) · [Deferred](#deferred)

## Overview

### Summary

Agent Sync keeps selected agent configuration directories, currently `~/.codex`
and `~/.claude`, synchronized through a cloud remote. The cloud profile is the
authoritative copy for a linked local workspace, but local writes are treated
carefully: push reconciles remote changes into a local union before
uploading, and pull never writes a cloud path until the path has been
validated as safe.

The design targets S3-compatible object storage, including AWS S3 and
Cloudflare R2.

### Goals

- Support multiple named remotes, one active remote at a time.
- Support multiple profiles inside one bucket without a central mutable index.
- Let multiple machines link to the same profile.
- Detect cloud-ahead and conflicting files before push, and resolve them as
  a deterministic union instead of blocking (see Union Conflict Resolution).
- Make pulls deterministic: read `_head.json`, then exactly the manifest it
  references — never stray bucket objects.
- Keep local baselines scoped by remote and profile.
- Make commit history verifiable and every published revision
  reconstructable.
- Preserve safe defaults for temporary files, caches, machine-specific runtime
  state, and SQLite sidecars.

### Non-Goals For v1

- A conflict-resolution / three-way merge UI. Deterministic automatic
  unions for known append-only JSONL files are in scope (see Union Conflict
  Resolution); arbitrary text merge is not.
- Push to every remote in one action.
- Shared cross-profile paths.
- Per-file "shared vs profile" scope controls.
- Migration of the current flat `SyncConfig` or of existing bucket-root
  objects pushed by the current implementation. Old cloud data is left in
  place and ignored; users re-link and re-push.
- The `custom_api` backend. v1 remotes are S3-compatible only; the custom API
  code path is removed from the sync flows.

### Core Model

Each remote points at one storage bucket. A profile is a top-level cloud
slot under that bucket holding **one agent config root**: `~/.codex` and
`~/.claude` sync as two separate profiles, each with its own head, history,
and baseline. A remote links one active profile per root. The `profile_id`
in local config is the `{some_id}` prefix shown below; it is not a user
identifier, and which root a profile holds is recorded in its head and tag
(`root: ".codex"`).

```
                    +-----------------------------+
                    | S3/R2 bucket: work-configs  |
                    |                             |
                    | 01H...A9/     root .codex   |
                    |   _head.json                |
                    |   _tag.json                 |
                    |   _manifests/...            |
                    |   _commits/...              |
                    |   _uploads/...              |
                    |                             |
                    | 01H...B4/     root .claude  |
                    |   ...                       |
                    +-------------^---------------+
                                  |
                  S3 List/Get/Put | active remote, one profile per root
                                  |
+-----------------------+         |         +-----------------------+
| client A              |---------+---------| client B              |
| ~/.codex + ~/.claude  |                   | ~/.codex + ~/.claude  |
| baseline per profile  |                   | baseline per profile  |
+-----------------------+                   +-----------------------+
```

Push and pull run the whole flow below once per linked root. The two
profiles never share objects, so a `.claude` publish can neither race nor
invalidate a `.codex` publish.

The single authoritative object per profile is `_head.json`: a small,
CAS-protected pointer to the current revision's immutable manifest and
commit. Everything else is either immutable history or a display cache.

File states are computed from three inputs:

- `B` — local baseline: the cloud state this machine last applied or pushed.
- `C` — the manifest referenced by the current `_head.json`.
- `L` — current local filesystem scan.

## Configuration

Use a stable `remote_id` for identity and a mutable `name` for display. The
display name is never a foreign key: renaming a remote must not lose the local
baseline.

Default-exclusion opt-ins are per remote, not global. Two remotes (work vs
personal) may want different opt-ins, and the opt-in set feeds per-profile
conflict and deletion semantics.

```rust
struct SyncConfig {
    schema_version: u32,
    active_remote_id: Option<String>,
    remotes: Vec<Remote>,
}

struct Remote {
    remote_id: String,          // generated, stable, not user-editable
    name: String,               // user-editable label

    // S3/R2 credentials
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
    account_id: String,
    s3_endpoint: String,        // optional; see endpoint resolution below
    region: String,

    // Which default-excluded roots the user opted back in, for this remote.
    included_default_exclusions: Vec<String>,

    // Probed or user-forced; see Transport Contract and Single-Writer Mode.
    supports_conditional_writes: Option<bool>,

    profiles: Vec<ProfileLink>,  // at most one per root
}

struct ProfileLink {
    root: String,                // ".codex" | ".claude"
    profile_id: String,
    profile_label: String,
    actor_name: String,          // user-editable, written to commit metadata
    machine_name: String,        // user-editable, written to commit metadata
}
```

**Endpoint resolution.** If `s3_endpoint` is set, use it. Otherwise, if
`account_id` is set, derive the R2 endpoint
`https://{account_id}.r2.cloudflarestorage.com`. Otherwise, if `region` is a
real AWS region, use the standard AWS S3 endpoint for that region. Plain AWS
S3 must work without hand-typing an endpoint.

### Profile IDs

`profile_id` is only the bucket prefix for one cloud profile; it exists so one
bucket can hold several profiles. The same ID in two different buckets refers
to two unrelated profiles.

Use at least 96 bits of randomness, encoded as lowercase hex or lowercase
Crockford base32. The UI may show a short prefix, but object keys use the full
ID. Generated IDs must not start with `_`; top-level underscore names are
reserved for bucket metadata.

**Creation flow** — the same publish pattern as a push, with put-if-absent as
the creation CAS:

1. Generate `profile_id` and a `commit_id`; fix the profile's `root`.
2. Write the immutable init objects:
   `_manifests/000000000000-{commit_id}.json` (empty manifest) and
   `_commits/000000000000-{commit_id}.json` (message `Create profile`).
3. Publish `_head.json` with `If-None-Match: *`, pointing at both objects
   and carrying the profile's `root`.
4. Best-effort write `_tag.json` as a display cache.
5. Save the `ProfileLink` into the selected remote's per-root slot.

If step 3 fails with "already exists" for a freshly generated ID, regenerate
the ID and start over (a retried partial creation, or an astronomically
unlikely collision).

A crash before step 3 leaves a prefix with no `_head.json`: not a profile,
invisible and harmless, removable through the same delete path as profile
cleanup. A crash between steps 3 and 4 leaves a real profile with no tag;
discovery falls back to reading `_head.json` (see Profile Discovery), and any
client may repair `_tag.json` best-effort.

## Cloud Layout

Each profile is a bucket prefix. Underscore-prefixed names at the profile root
are reserved metadata. Manifest keys stay simple logical paths; file bytes
live in immutable upload batches under `_uploads/`, **named by their original
relative path** so the bucket stays human-browsable — the bytes are the plain
file contents, never encrypted or content-address-renamed:

```
logical path:   .codex/sessions/2026/04/01/rollout-…-75595de6370d.jsonl
physical key:   {profile_id}/_uploads/{upload_id}/files/.codex/sessions/2026/04/01/rollout-…-75595de6370d.jsonl
```

The per-attempt `{upload_id}` prefix is required for partial-upload and race
safety: no uploaded byte is visible to pull until `_head.json` flips to a
manifest that references it, and two racing pushes write under different
batch prefixes so they can never overwrite each other. A revision's full
tree is readable in place under its batches; unchanged files keep pointing
at the batch that first uploaded them.

```
{profile_id}/                        one profile = one agent root
  _head.json                         canonical profile HEAD, CAS-protected
  _tag.json                          display cache, non-authoritative
  _manifests/
    000000000012-4d58b5a0d3e24e2b.json   immutable manifest, {revision}-{commit_id}
  _commits/
    000000000012-4d58b5a0d3e24e2b.json   immutable commit record
  _uploads/
    01JUPLOADA/
      _upload.json                   upload batch metadata
      files/
        .codex/config.toml           immutable snapshot, original path
        .codex/sessions/2026/04/01/rollout-….jsonl

{other_profile_id}/                  the other root's profile
  ...
```

Every object under `_manifests/`, `_commits/`, and `_uploads/` is immutable
and written **before** the head flips. Keys embed both the zero-padded
revision (so listings sort in revision order) and the `commit_id` (so
racing or crashed pushes can never collide on a key — the head CAS is the sole
arbiter of who published). An object whose key is never referenced by the head
chain is an **orphan**: the residue of a lost race or a crashed push, ignored
by every reader and cleanable by age.

**Profile discovery** lists the bucket root by prefix (paginated), skips
entries whose first path component starts with `_`, and reads
`{id}/_head.json` for the authoritative `root` and revision, plus
`{id}/_tag.json` for display fields. Prefixes with no readable head are
ignored. One bad profile must not block the picker. Auto-linking matches on
`head.root`: exactly one profile with the wanted root links itself, none
creates one, several require an explicit choice.

### `_head.json`

The only authoritative, mutable profile object. Published with
`If-None-Match: *` at creation and compare-and-swapped (`If-Match: etag`) on
every push.

```json
{
  "schema_version": 1,
  "profile_id": "01h...a9",
  "root": ".codex",
  "state": "active",
  "revision": 12,
  "commit_id": "4d58b5a0d3e24e2b",
  "manifest_key": "_manifests/000000000012-4d58b5a0d3e24e2b.json",
  "commit_key": "_commits/000000000012-4d58b5a0d3e24e2b.json",
  "manifest_sha256": "42bc...",
  "updated_at": 1750000300
}
```

`manifest_sha256` is the sha256 of the referenced manifest's bytes. Pull
verifies the fetched manifest against it, which closes the residual window
where a key could hold unexpected bytes and turns any divergence into a
detected corruption instead of a silent one.

`state` is `active` in v1; other values are reserved (profile archival and
tombstoning later, without a schema change).

### `_tag.json`

A best-effort display cache for the profile picker: label, file count, and the
latest commit summary with the user-editable actor and machine labels. It
carries **no** correctness weight — it may be stale or missing, and any client
may rewrite it from the current head and commit.

```json
{
  "schema_version": 1,
  "label": "Work Config",
  "root": ".codex",
  "created_at": 1750000000,
  "updated_at": 1750000300,
  "revision": 12,
  "files": 142,
  "last_commit": {
    "commit_id": "4d58b5a0d3e24e2b",
    "revision": 12,
    "created_at": 1750000300,
    "actor_name": "alice",
    "machine_name": "alice-macbook",
    "message": "Push 3 changed files"
  }
}
```

Timestamps come from client clocks and are display-only; the picker may sort
by `updated_at` but must not use it for any correctness decision.

### `_manifests/{revision}-{commit_id}.json`

The immutable full inventory of the profile as of one revision. The manifest
referenced by the current head is the source of truth for pull and conflict
detection.

```json
{
  "schema_version": 1,
  "revision": 12,
  "commit_id": "4d58b5a0d3e24e2b",
  "updated_at": 1750000300,
  "files": {
    ".codex/config.toml": {
      "sha256": "a3f...",
      "size": 1234,
      "object_key": "_uploads/01JUPLOADA/files/.codex/config.toml",
      "source_mtime": 1749990000
    },
    ".codex/sessions/abc.jsonl": {
      "sha256": "9b2...",
      "size": 88012,
      "object_key": "_uploads/01JUPLOADA/files/.codex/sessions/abc.jsonl",
      "source_mtime": 1749991234
    }
  }
}
```

The map key is the safe local relative path; `object_key` is profile-relative
(`{profile_id}/{object_key}` is the full key). Because a partial push carries
unchanged entries forward, a manifest routinely references objects across many
historical upload batches, not just the latest one.

`source_mtime` is the source file's modification time (epoch seconds) at
upload scan time; pull restores it after the atomic apply so mtime-indexed
tooling (Codex's thread rebuild) sees real recency
(PLAN_CODEX_THREAD_REBUILD_AND_SIDEBAR.md). `0` or absent (manifests from
older builds) means unknown — apply skips the restore. Merge-driver outputs
and this machine's conflict copies are stamped at merge time instead, and
SQLite snapshots are exempt. Same-content re-uploads keep the published
entry, so a bare `touch` never republishes a revision.

Because manifests are written before publish, every revision reachable from
the head chain has its manifest by construction. There are no history holes —
only orphans, which are unpublished and ignored.

### `_commits/{revision}-{commit_id}.json`

The immutable commit record for one publish attempt, written alongside its
manifest before the head flip.

```json
{
  "schema_version": 1,
  "commit_id": "4d58b5a0d3e24e2b",
  "revision": 12,
  "created_at": 1750000300,
  "actor_name": "alice",
  "machine_name": "alice-macbook",
  "upload_id": "01JUPLOADA",
  "message": "Push 3 changed files",
  "manifest_key": "_manifests/000000000012-4d58b5a0d3e24e2b.json",
  "manifest_sha256": "42bc...",
  "previous_commit_key": "_commits/000000000011-9fd1c2e8b4a05677.json",
  "previous_manifest_sha256": "9fd1...",
  "summary": { "added": 1, "modified": 2, "deleted": 0 }
}
```

Authoritative history is the chain walked backward from the head via
`previous_commit_key`. A plain listing of `_commits/` is only a display
approximation: it sorts by revision but may include orphans (same
revision, different `commit_id`, never published). The chain never includes
them.

### `_uploads/{upload_id}/_upload.json`

Upload-batch metadata, including batches that lost a head race or failed
before publish. Each object entry records its logical path, so batches are
inspectable without a manifest.

```json
{
  "schema_version": 1,
  "upload_id": "01JUPLOADA",
  "created_at": 1750000290,
  "actor_name": "alice",
  "machine_name": "alice-macbook",
  "base_revision": 11,
  "status": "staged",
  "objects": [
    {
      "path": ".codex/config.toml",
      "sha256": "a3f...",
      "size": 1234,
      "object_key": "_uploads/01JUPLOADA/files/.codex/config.toml"
    },
    {
      "path": ".codex/sessions/abc.jsonl",
      "sha256": "9b2...",
      "size": 88012,
      "object_key": "_uploads/01JUPLOADA/files/.codex/sessions/abc.jsonl"
    }
  ]
}
```

`status` (`staged` → `committed`) is informational and may be stale if a
client crashes. Pull never trusts it; pull only follows the head. Cleanup uses
it only together with the age threshold below.

## History Retention And Cleanup

Retention policy:

- `_commits/` and `_manifests/` entries are small and retained indefinitely
  by default.
- The **retained revision set** is found by walking the chain backward from
  the head via `previous_commit_key`: the current revision plus the most
  recent `history_keep` revisions (default 10, configurable per remote).
- The **protected batch set** is every `upload_id` appearing in the
  `object_key`s of the retained manifests. Published manifests always exist
  (they are written before publish), so this set is always computable —
  protection derives from manifest contents, never from commit records alone.

`history_keep` is read from whichever machine runs cleanup, so across machines
the smallest configured value effectively wins. That is acceptable: pruning
history never affects sync correctness.

Cleanup is user-initiated and runs in three passes:

1. **History pruning (optional).** Walk the chain from the head; delete
   `_manifests/` and `_commits/` entries older than the retained revision
   set. (The walk then terminates at the pruning horizon; that is expected.)
2. **Orphan cleanup.** Delete `_manifests/` and `_commits/` objects that are
   not reachable from the head chain **and are older than 24 hours** — the
   residue of lost races and crashed pushes.
3. **Batch cleanup.** List `_uploads/`, compute the protected batch set, and
   offer deletion only for batches that are (a) not protected and (b) either
   referenced only by pruned or orphaned history, or `staged`/missing
   `_upload.json` **and older than 24 hours**.

The age threshold exists because young unpublished objects may belong to
another machine's push in progress: deleting them between its uploads and its
head CAS would let a head flip to missing objects. Anything younger than
24 hours is never offered for deletion, regardless of status.

Cleanup never deletes the head, the objects it references, or any batch in the
protected set.

On a remote in single-writer mode (see below), history pruning and deletion of
published history or `committed` batches are disabled; only stale `staged`
batches and stale orphans may be cleaned.

## Local State

```
app_data/
  sync_config.json
  staging/
  baselines/
    {remote_id}/
      {profile_id}.json
  backups/
    {remote_id}/
      {profile_id}/
        2026-07-02T120000Z/
          .codex/config.toml
```

### Baselines

`baselines/{remote_id}/{profile_id}.json` stores the last **applied** cloud
state plus local optimization metadata. Renaming a remote does not move the
baseline because `remote_id` is stable.

The baseline records only entries that were actually applied locally. A pull
that skips a path — default-excluded here, failed validation, failed to
download — must not record it; otherwise the next scan would see
`B exists && L missing` for a file this machine never had and misreport it
as a local deletion. The baseline must always mean "this machine applied or
published exactly this content".

A locally deleted path that still exists in `C` does not linger in a
deleted state: deletions never propagate, so the union pass restores the
file during the next push or pull and the baseline entry then reflects the
restored file.

```rust
struct LocalBaseline {
    schema_version: u32,
    remote_id: String,
    profile_id: String,
    cloud_revision: u64,
    cloud_commit_id: String,
    cloud_manifest_sha256: String,
    files: HashMap<String, LocalFileRecord>,
}

struct LocalFileRecord {
    sha256: String,     // for SQLite databases: hash of the uploaded snapshot
    size: u64,
    mtime: u64,         // stat'ed after write/push; 0 = fast path disabled
}
```

`mtime`/`size` are a fast path to skip hashing; on mismatch, fall back to
hashing. For SQLite databases the fast path is always disabled (see SQLite
Handling): the recorded hash is of the backup snapshot, which never matches
the on-disk bytes.

### Backups And Staging

`backups/{remote_id}/{profile_id}/{timestamp}/` is local-only rollback
storage. Before pull overwrites or deletes a local file, the previous version
is copied there. Backups are never uploaded and play no part in conflict
detection. Retention: keep the most recent 10 pull backups per profile
(configurable); prune older ones after a successful pull.

`staging/` holds in-flight pull downloads and push upload snapshots. Leftovers
from a crashed run are deleted on app start.

## Sync Semantics

### Eligibility And Selection

Two separate concepts:

- **Persistent sync rules** decide which paths are *eligible* on this machine:
  allowed roots, default exclusions, per-remote opt-ins, SQLite sidecar rules.
- **UI selection** decides which eligible paths go into the current push.

The manifest is the full profile inventory. A partial push starts from the
current manifest, updates the selected changed paths, and writes a new full
manifest containing both changed and unchanged entries. Cloud entries that are
ineligible on this machine are carried forward untouched — machine A's opt-in
must survive machine B's push.

The state matrix is computed only over paths eligible on this machine.
Ineligible cloud entries are invisible to status display, conflict detection,
and deletion propagation.

Unselected paths are ignored, not deleted. Deletions are never propagated
in v1 — the union restores the file on the deleting side instead: a path in
`B`, absent in `L`, present in `C` is re-applied from the cloud, and a path
in `B`, present in `L`, absent in `C` is re-published by the next push.
Explicit deletion propagation (with confirmation) is deferred.

### File State Matrix

For each **eligible** path in `union(B.files, C.files, L.files)`:

```
local_changed = L[path] != B[path]
cloud_changed = C[path] != B[path]
same_content  = L[path] == C[path]
```

A missing entry is a distinct comparable value: `missing != present`,
`missing == missing`. With that convention the formulas cover creations,
deletions, and delete/modify conflicts uniformly. The existence-based rows
below are special cases of the formulas and take display precedence, so the UI
can label them more specifically than "conflict".

| State | Condition | Action (push and pull) |
|---|---|---|
| synced | !local_changed && !cloud_changed | skip |
| local ahead | local_changed && !cloud_changed | push uploads; pull keeps and reports |
| cloud ahead | !local_changed && cloud_changed | apply cloud version locally |
| converged | local_changed && cloud_changed && same_content | update baseline |
| conflict | local_changed && cloud_changed && !same_content | union merge (see below) |
| local only | B missing && C missing && L exists | push uploads; pull keeps |
| cloud only | B missing && C exists && L missing | apply cloud version locally |
| local deleted | B exists && L missing && C exists | restore from cloud (no delete propagation) |
| cloud deleted | B exists && C missing && L exists | keep local; next push republishes |

Delete/modify combinations resolve in favor of the surviving content: a
path deleted in the cloud but modified locally is re-published, and a path
deleted locally but modified in the cloud is re-applied. A path gone on
all three sides just drops its baseline record.

With no local baseline for a linked profile, existing cloud files are
`cloud only`; the first action after linking should usually be pull.

### Union Conflict Resolution

Both-changed paths are not blocked; they resolve to the **union** of the
two sides, locally, before anything is published. The invariants:

- No committed content is lost on either side, ever.
- Resolution is deterministic and idempotent fleet-wide: two machines
  resolving the same pair independently produce byte-identical results —
  including conflict-copy names — so repeated syncs converge instead of
  ping-ponging.
- Deletions never propagate; the union restores the file.
- Pull never publishes: push is "download the conflict, resolve locally as
  a union, then publish"; pull applies the same union locally and stops.

Resolution picks the first applicable rule:

1. **Same content** (`converged`): both sides independently hold the same
   bytes — update the baseline, done.
2. **Merge driver.** Known append-only JSONL files merge line-wise:

   | Path | Driver |
   |---|---|
   | `.codex/history.jsonl`, `.claude/history.jsonl` | Dedupe exact lines, order by embedded timestamp (`ts` / `timestamp`), ties by line bytes. |
   | `.codex/session_index.jsonl` | Key by `id`, keep the later `updated_at` (ties to the lexically greater line), sort ascending, cap at the newest 100 records — codex prunes this file itself, and an unbounded union would resurrect pruned entries forever. |

   The merged result is written locally (skipping the write when one side
   already contains the union); on push it is what gets uploaded.
3. **Conflict copy.** For everything else the local version wins the path
   and the cloud version is preserved as a sibling
   `name.sync-conflict-<hash8>.ext`, where `<hash8>` is the first 8 hex
   chars of the cloud content's sha256. The name is content-derived, so
   every machine produces the same sibling and re-resolution is a no-op.
   On push both files are published.

After a merge or conflict copy, the baseline records the **cloud** side's
sha with the mtime fast path disabled, so the locally resolved file keeps
showing as `local ahead` until a push publishes it.

**Scenario: racing pushes.** A and B work against the same profile. A
pushes at 19:00 and publishes revision `N+1` at 19:05. B pushes at
19:10: B GETs the head, sees `N+1`, downloads every path where both B's
copy and `N+1` changed since B's baseline, resolves each locally as a
union, then uploads and CASes the head to `N+2`. Nothing A published is
lost. Had A still been publishing when B read the head, B's CAS would fail
and B would rebase against the new head and re-run the same union — the
head CAS serializes publishes; the union makes the retry safe.

**Scenario: pull over local edits.** A pulls at 19:00 (baseline at
revision `N`). B publishes `N+1` at 19:15. A keeps editing locally until
19:20, then pulls. The pull fetches head `N+1` and classifies against A's
baseline: paths only B changed are applied, paths only A changed are kept
and reported as `local ahead`, and paths both changed resolve as the same
union — merged in place or preserved as a conflict-copy sibling. A's next
push publishes the union.

### Cloud State Caching

Computing the matrix needs `C`. Rules:

- The app keeps one cached head + manifest per remote/profile in memory with a
  `fetched_at` timestamp.
- The cache refreshes when the sync panel opens, on an explicit refresh
  action, and always at the start of push and pull — those two flows never
  trust the cache. A refresh is a conditional GET of `_head.json` against the
  cached ETag: a 304 costs almost nothing and skips re-fetching the manifest.
- `get_file_statuses` computes against the cached manifest and reports its
  age. Before any successful fetch, statuses degrade to `L` vs `B` only and
  are labeled `cloud state unavailable`; cloud-ahead and conflict detection
  are explicitly marked unknown, never silently reported as absent.

## Push Flow

```
+-------------+
| Push clicked|
+------+------+
       |
       v
+------+----------------+
| Resolve active remote |
| and linked profile    |
+------+----------------+
       |
       v
+------+-----------------------------+
| GET _head.json and its ETag        |
| GET the manifest it references     |
+------+-----------------------------+
       |
       v
+------+----------------+
| Load scoped baseline  |
+------+----------------+
       |
       v
+------+------------------------+
| Scan selected local files     |
| and classify state matrix     |
+------+------------------------+
       |
       +--> cloud ahead / both changed? --> download cloud side,
       |    union locally (merge drivers / conflict copies)
       |
       v
+------+----------------+
| Upload content blobs  |
+------+----------------+
       |
       v
+------+-----------------------------+
| Write immutable manifest + commit  |
| for revision+1 (unique keys)     |
+------+-----------------------------+
       |
       v
+------+-----------------------------+
| CAS _head.json (If-Match: ETag)    |
+------+-----------------------------+
       |
       +--> CAS failed? re-read head, report remote changed
       +--> CAS ambiguous? re-GET head, compare commit_id
       |
       v
+------+-------------------------+
| Best-effort _tag.json          |
| Save local baseline            |
+--------------------------------+
```

Steps:

1. Resolve `active_remote_id`; fail if missing.
2. Fail into setup flow if `remote.profile` is missing.
3. GET `{profile_id}/_head.json`; record its ETag. GET the manifest at
   `head.manifest_key` and verify its bytes against `head.manifest_sha256`.
4. Load `baselines/{remote_id}/{profile_id}.json`.
5. Scan selected eligible local paths and classify the state matrix. Then
   run the union pass: download every cloud-ahead and both-changed path,
   apply or merge it locally per Union Conflict Resolution, and add merged
   results and conflict copies to the upload set.
6. Build a full desired manifest by applying the upload set to `C`.
   Ineligible and unselected cloud entries are carried forward unchanged,
   and a same-content upload keeps the already-published object.
7. Generate `upload_id` and a new `commit_id`.
8. For each changed file: snapshot it once into a local staging file (SQLite
   via the backup API, everything else via copy), hash the snapshot, and
   stream-upload it to `{profile_id}/_uploads/{upload_id}/files/{path}` —
   the original relative path under the batch prefix. Snapshots are deleted
   after the push.
9. PUT `_uploads/{upload_id}/_upload.json` with actor, machine, base
   revision, object list (with logical paths), and status `staged`.
10. PUT the immutable manifest
    `_manifests/{revision+1}-{commit_id}.json`.
11. PUT the immutable commit
    `_commits/{revision+1}-{commit_id}.json`, linking
    `previous_commit_key` to the current head's commit.
12. CAS `_head.json` (`If-Match`: the ETag from step 3) to point at the new
    manifest and commit, with the new `manifest_sha256`.
13. If the CAS fails, another client published first: do not save the
    baseline; the new manifest, commit, and batch become orphans, ignored
    by every reader and cleanable by age. Rebase instead of reporting a
    failure — re-read the head, re-run the union against the new
    revision, and retry (bounded attempts). If the outcome is ambiguous,
    re-GET the head and compare `commit_id` (see Publish Semantics).
14. Best-effort PUT `_tag.json` with the latest commit summary.
15. Optionally update `_uploads/{upload_id}/_upload.json` status to
    `committed`.
16. Save the local baseline from the desired manifest. Entries present
    locally get mtimes stat'ed now (0 for SQLite databases). Entries this
    machine has never applied stay out of the baseline. Locally deleted
    paths need no special casing: the union pass has already restored them
    from the cloud earlier in this push.

### Publish Semantics

S3-compatible storage has no atomic multi-object transaction, so the head CAS
is the only publish point. Everything written before it — content objects,
batch metadata, the manifest, the commit — is invisible to readers until the
head flips, and every key is unique to this attempt (`upload_id`,
`{revision}-{commit_id}`), so concurrent pushes can never overwrite each
other's staged objects. Race handling is uniform: the CAS wins or loses.

Each changed file is snapshotted once into a local staging file; the recorded
sha256, the uploaded bytes, and the manifest entry all derive from that one
immutable snapshot. Hashing the live file and re-reading it at upload time
would race with concurrent edits; reading whole files into memory would not
scale to large session files or databases — uploads stream from the snapshot.

Failure cases:

- Any upload or metadata write before the CAS fails → nothing was published;
  everything already written is an orphan, ignored by readers and cleanable
  by age.
- Head CAS fails (another client pushed first) → same: the attempt's objects
  remain as orphans. Re-read the head and rebase.
- Head CAS outcome **ambiguous** (timeout, dropped connection) → re-GET
  `_head.json`; if `head.commit_id` equals this push's `commit_id`, the write
  won — continue with the post-publish steps. Otherwise treat as a CAS
  failure. If the re-GET itself fails, report the push as failed without
  saving the baseline; the `converged` state repairs file-level status on the
  next scan, and because the manifest and commit were written before the CAS,
  a push that actually won leaves no missing history either way.
- `_tag.json` fails after a successful CAS → non-fatal; the tag is a cache
  and any client may repair it.

### Single-Writer Mode

The head CAS is what makes every multi-writer guarantee hold: it serializes
pushes, assigns revisions uniquely, and decides races. A remote that lacks
conditional writes (`supports_conditional_writes == false`) cannot
approximate that — two unconditional head writes are last-writer-wins, and
the loser's push is silently discarded.

Such remotes therefore run in an explicit degraded mode. The app does not
pretend: it disables the guarantees rather than weakening them silently.

- The UI labels the remote **"single writer — force push only"** and warns
  that it is safe only if exactly one machine ever pushes to the profile.
- `_head.json` is overwritten unconditionally; racing pushes are
  last-writer-wins.
- Revision numbers and the commit chain are advisory: history is not
  trusted, and `manifest_sha256` verification of *past* revisions is
  disabled (the current head's `manifest_sha256` is still checked on pull).
- Profile creation degrades to check-then-write; the random ID makes a
  collision effectively impossible, but the creation race guarantee is gone.
- Cleanup disables history pruning and deletion of published history or
  `committed` batches; only stale `staged` batches and stale orphans may be
  cleaned.

Pull is otherwise unaffected: it reads the head, fetches the referenced
manifest, and verifies content hashes exactly as in multi-writer mode.

## Pull Flow

```
+-------------+
| Pull clicked|
+------+------+
       |
       v
+------+----------------+
| Resolve active remote |
| and linked profile    |
+------+----------------+
       |
       v
+------+-----------------------------+
| GET _head.json                     |
| GET + verify referenced manifest   |
| Load scoped baseline               |
+------+-----------------------------+
       |
       v
+------+---------------------------+
| Validate every manifest path     |
+------+---------------------------+
       |
       +--> invalid path? skip entry, exclude from baseline
       |
       v
+------+---------------------------+
| Classify state matrix            |
| Warn if agent processes running  |
+------+---------------------------+
       |
       +--> local ahead? --> keep and report
       +--> both changed? --> union locally
       |
       v
+------+---------------------------+
| Download changed eligible files |
| to staging; verify sha256/size  |
+------+---------------------------+
       |
       v
+------+---------------------------+
| Backup overwritten/deleted files |
| Apply: temp write + rename       |
| (SQLite: clear sidecars first)   |
+------+---------------------------+
       |
       v
+------+-----------------------+
| Save local baseline           |
| (applied entries only)        |
+-------------------------------+
```

Steps:

1. Resolve the active remote and profile.
2. GET `{profile_id}/_head.json`. GET the manifest at `head.manifest_key` and
   verify its bytes against `head.manifest_sha256`; a mismatch or missing
   object is profile corruption — abort. Load the scoped baseline.
3. Validate every manifest path and object key. Invalid entries are skipped,
   logged, and excluded from the baseline; valid entries still proceed.
   Case/normalization-colliding pairs are both skipped.
4. Filter to paths eligible on this machine (allowed roots, exclusions,
   opt-ins). Ineligible entries are skipped silently and excluded from the
   baseline.
5. Classify the state matrix. Local-ahead files are kept and reported;
   both-changed files resolve locally per Union Conflict Resolution — a
   pull never discards local changes, so no overwrite confirmation is
   needed. Warn if agent processes appear to be running.
6. Download into staging every remaining file whose content is not already
   present locally — entries where the local hash equals `C[path].sha256`
   (states `synced` and `converged`) are skipped and need only a baseline
   update, so a routine pull does not re-fetch a large, mostly unchanged
   profile. Verify sha256 and size in staging before touching any local
   file. Declared sizes are untrusted input: enforce each entry's `size` as
   a hard cap while streaming (abort past it, never buffer first) and check
   available disk space against the total declared size before starting. A
   download or verification failure aborts the pull with nothing written.
7. Apply: check the existing destination with `symlink_metadata` — a symlink
   destination is skipped, reported, and excluded from the baseline (see
   Safety Rules). Otherwise back up the old file, write through a temporary
   file in the destination directory, then rename. For SQLite databases
   follow the sidecar procedure in SQLite Handling. A file that cannot be
   applied (locked, I/O error) is reported and excluded from the baseline;
   the rest of the pull continues.
8. Deletions are not propagated: a path in state `cloud deleted` keeps its
   local file, and the next push republishes it (union semantics). The
   confirmed delete pass is deferred together with explicit deletion
   propagation.
9. Save the baseline containing exactly the entries now applied locally, with
   fresh mtimes, plus the head's revision, `commit_id`, and
   `manifest_sha256`.

**Failure policy — three tiers.** Validation failures (step 3) skip the
affected entries and continue. Staging failures (step 6) abort the whole pull
before anything local is touched — that step is the one transactional gate.
Apply failures (step 7) are best-effort per file: the entry is reported and
excluded from the baseline while already-applied entries stay applied. Pull
never attempts automatic rollback; the backups from step 7 exist for manual
recovery.

Pull is cloud-authoritative only for paths the local side has not changed;
both-changed paths resolve to the union, and local-ahead paths are kept.
Cloud-side deletions are never destructive locally.

## Setup Flow

Shown when the selected remote has no linked profile.

```
+----------------------------------------------------+
| Link cloud profile                                 |
|                                                    |
| Remote: work-r2                                    |
|                                                    |
| ( ) Create new profile                             |
|     Label: [ Work Config                    ]      |
|                                                    |
| ( ) Link existing profile                          |
|     +----------------------------------------+     |
|     | Work Config      142 files   alice    |     |
|     |   alice-macbook  2h ago               |     |
|     | Personal          91 files   bob      |     |
|     |   studio-mini    3d ago               |     |
|     +----------------------------------------+     |
|                                                    |
| Actor:   [ alice                           ]       |
| Machine: [ alice-macbook                   ]       |
|                                                    |
|                                  [Cancel] [Link]   |
+----------------------------------------------------+
```

**Create profile:** run the creation flow from Profile IDs, then save the
local `ProfileLink` including `actor_name` and `machine_name`.

**Link existing profile:** list profiles (paginated tag/head discovery), let
the user pick, save the `ProfileLink`, and prompt for an initial pull before
allowing normal push.

## Safety Rules

### Safe Relative Paths

Cloud paths are untrusted input. Before reading, writing, uploading, or
deleting, normalize and validate each manifest path:

- `/` separators only. Reject any `\`, NUL byte, or other control character
  anywhere in the path.
- Relative, non-empty, no leading `/`.
- No `..`, `.`, or empty (`a//b`) components.
- No Windows drive prefixes (`C:`) and no Windows reserved device names as
  any component (`CON`, `NUL`, `AUX`, `COM1`…, including with extensions,
  e.g. `nul.txt`).
- Must start with an allowed root: `.codex/` or `.claude/`.
- Must be eligible under this machine's default-exclusion rules and opt-ins.

**Collision rule.** Two manifest paths that collide under case-insensitive
comparison or Unicode normalization (NFC vs NFD) would silently overwrite each
other on default macOS volumes. If the manifest contains such a pair, pull
reports both paths as conflicted and skips both; neither enters the baseline.

**Object and metadata keys** are equally untrusted. Content keys must match
exactly `_uploads/{upload_id}/objects/{sha256}` where `upload_id` is 1–64
characters of `[a-z0-9-]` (lowercase Crockford base32 ULID recommended) and
`sha256` is exactly 64 lowercase hex characters. Head-referenced keys must
match `_manifests/{revision}-{commit_id}.json` and
`_commits/{revision}-{commit_id}.json`, with a zero-padded 12-digit
revision and a 16-lowercase-hex `commit_id`. Reject the manifest entry (or
the whole head) on any mismatch.

### Destination Writes And Symlinks

Pull writes with a safe join against **canonicalized** allowed roots:

```
home = dirs::home_dir()
relative = validate_manifest_path(path)
dest = home.join(relative)
dest's parent chain must resolve under canonicalize(home/.codex)
or canonicalize(home/.claude)
```

The roots are canonicalized before comparison because users commonly symlink
`~/.codex` or `~/.claude` into a dotfiles repository, and a literal-path check
would reject every file for them. A symlinked root is tolerated; symlinks
*beneath* the root are not.

Directory creation must not run before validation. `create_dir_all` followed
by a canonical check is unsafe: if `~/.codex/foo` is already a symlink,
creating `~/.codex/foo/bar` mutates a directory outside the allowed root
before any check fires. Instead:

1. Walk `dest`'s ancestor chain downward from the canonical allowed root. For
   every component that exists, `symlink_metadata` must report a real
   directory — any symlink in the chain rejects the entry.
2. Create the missing components one at a time with `create_dir` (never
   `create_dir_all`).
3. After the walk, `canonicalize(dest.parent())` must equal the expected path
   under the canonical root. This closes the race where a component is
   swapped for a symlink mid-walk.

The destination file itself gets the same treatment via `symlink_metadata`
(never `metadata`, which follows links): if the existing destination is a
symlink, backing it up or writing through it could read from or clobber a file
outside the allowed roots, so the entry is skipped, reported, and left out of
the baseline. Backup copies apply the same rule on the backup path side. Local
scans for push already skip symlinks (`follow_links = false`).

Any validation failure: skip the file, log an error, keep the path out of the
saved baseline.

### SQLite Handling

**Upload** (unchanged from current behavior): `.sqlite` databases are
snapshotted with the SQLite backup API, so WAL contents are folded in and a
consistent image is uploaded even while the database is in use. `-wal`,
`-shm`, and `-journal` sidecars are never synced. The manifest and baseline
record the snapshot's hash and size, not the on-disk file's, so the
mtime/size fast path is disabled for these files and status checks
re-snapshot to compare.

**Pull** is the dangerous direction. Replacing a database file while a stale
local `-wal` remains causes SQLite recovery to replay the old WAL over the new
database — a documented corruption vector. When pull replaces a `.sqlite`
file it must, as one step:

1. Back up the existing database **and** its sidecars.
2. Delete the `-wal`, `-shm`, and `-journal` sidecars.
3. Rename the downloaded temp file into place.

If the database is locked (another process holds it open), skip the file,
report it, and leave it out of the updated baseline rather than replacing it
underneath a running writer.

Before any pull, the app makes a best-effort check for running `codex` /
`claude` processes and warns that pulling while agents are running can lose
in-flight state (open files keep old inodes; live session files change under
the scan). The warning is advisory; the user may proceed.

## Interfaces

### UI Changes

- Settings panel shows remote tabs by `remote.name` and a `+` action.
- Each remote editor has credentials, display name, active toggle, per-remote
  exclusion opt-ins, and profile link status.
- Footer shows active remote and linked profile.
- Push button is disabled when there is no active linked profile.
- Pull button opens setup when the remote is configured but unlinked.
- Setup and settings expose editable actor and machine labels used for commit
  history.
- File status labels expand from `new | modified | synced` to the full state
  matrix, plus a cloud-state freshness indicator (age of cached head, or
  "cloud state unavailable").
- Conflict and cloud-ahead states are shown before upload starts, not after a
  failed upload.

### Rust Command Surface

```rust
#[tauri::command]
async fn list_sync_profiles(remote_id: String) -> Result<Vec<ProfileInfo>, String>;

#[tauri::command]
async fn create_profile(
    remote_id: String,
    label: String,
    actor_name: String,
    machine_name: String,
) -> Result<ProfileLink, String>;

#[tauri::command]
async fn link_profile(
    remote_id: String,
    profile_id: String,
    actor_name: String,
    machine_name: String,
) -> Result<ProfileLink, String>;

#[tauri::command]
async fn refresh_cloud_state() -> Result<CloudState, String>;

#[tauri::command]
async fn list_profile_commits(
    remote_id: String,
    profile_id: String,
    limit: usize,
) -> Result<Vec<CommitInfo>, String>;

#[tauri::command]
async fn list_upload_batches(
    remote_id: String,
    profile_id: String,
) -> Result<Vec<UploadBatchInfo>, String>;

#[tauri::command]
async fn cleanup_upload_batches(
    remote_id: String,
    profile_id: String,
    upload_ids: Vec<String>,
) -> Result<CleanupResult, String>;

// Computes against the cached head/manifest for the active remote/profile;
// never hits the network. Call refresh_cloud_state to update the cache.
#[tauri::command]
async fn get_file_statuses(paths: Vec<String>) -> Result<FileStatusReport, String>;
```

```rust
struct CloudState {
    revision: u64,
    commit_id: String,
    fetched_at: u64,
    files: u64,
}

struct FileStatusReport {
    cloud_revision: Option<u64>,   // None = cloud state unavailable
    cloud_fetched_at: Option<u64>,
    statuses: HashMap<String, FileStatus>,
}

enum FileStatus {
    Synced,
    LocalAhead,
    CloudAhead,
    Converged,
    Conflict,
    LocalOnly,
    CloudOnly,
    LocalDeleted,
    CloudDeleted,
    CloudUnknown,    // no cloud fetch yet: L vs B comparison only
}

struct CleanupResult {
    deleted_batches: u64,
    deleted_bytes: u64,
    skipped_upload_ids: Vec<String>,
}

struct ProfileInfo {
    profile_id: String,
    label: String,               // placeholder if _tag.json is missing
    files: u64,
    revision: u64,
    updated_at: u64,
    last_actor_name: String,
    last_machine_name: String,
}

struct CommitInfo {
    commit_id: String,
    revision: u64,
    created_at: u64,
    actor_name: String,
    machine_name: String,
    message: String,
    added: u64,
    modified: u64,
    deleted: u64,
}

struct UploadBatchInfo {
    upload_id: String,
    created_at: u64,
    actor_name: String,
    machine_name: String,
    status: String,
    object_count: u64,
    total_bytes: u64,
    referenced_by_current_manifest: bool,
    referenced_by_retained_manifest: bool,
    age_threshold_met: bool,
    cleanup_allowed: bool,
}
```

`list_profile_commits` walks the chain backward from the head via
`previous_commit_key`, up to `limit` records. A missing object terminates the
walk (history pruned at that horizon) and the result is marked truncated.
Orphaned commits never appear: the chain cannot reach them.

Existing `sync_upload`, `sync_download`, `get_sync_config`, and
`save_sync_config` remain, but `sync_upload` and `sync_download` resolve the
active remote internally from the saved config instead of trusting credentials
passed from the frontend at call time.

### Transport Contract

Baseline operations every remote must support:

- List objects by prefix, paginated (profiles, `_manifests/`, `_commits/`,
  `_uploads/`).
- Get object (plain and conditional GET by ETag), Put object.
- Delete object (cleanup, profile cleanup).

Conditional operations required for **multi-writer mode**:

- Compare-and-swap (`If-Match` on ETag) for `_head.json`.
- Put-if-absent (`If-None-Match: *`) for the initial `_head.json` at profile
  creation.

Nothing else needs conditional writes: `_manifests/`, `_commits/`, and
`_uploads/` keys are unique per attempt, so plain idempotent PUTs suffice.

AWS S3 gained conditional PUT (`If-None-Match: *`, then `If-Match`) in late
2024 and R2 supports conditional writes; generic "S3-compatible" stores often
do not. A remote lacking them runs in Single-Writer Mode (see Push Flow).

**Capability probing.** Probed on first use or set manually, cached in
`Remote.supports_conditional_writes`. The probe must test the negative case:
some stores accept conditional headers and silently ignore them, so a
successful conditional write proves nothing. Probe by writing a throwaway
object under the reserved top-level `_probe/` prefix, then attempting a second
write against a deliberately stale ETag; only a precondition failure
(HTTP 412) sets the capability to true. The probe object is deleted
afterwards.

## Error Handling

- Missing `_head.json` under a prefix means "not a profile": discovery
  ignores the prefix, and pull/push against a linked profile whose head has
  vanished report profile corruption (a head is never deleted by cleanup).
- An unreadable head, a missing `head.manifest_key` object, or a manifest
  whose bytes do not hash to `head.manifest_sha256` is profile corruption:
  abort the operation, change nothing locally.
- A stale or missing `_tag.json` is harmless: it is a display cache, and any
  client may rewrite it from the current head and commit.
- Any failure before the head CAS leaves the profile unchanged; everything
  already written is an orphan, ignored by readers and cleanable by age.
- An ambiguous head CAS is resolved by re-GET-and-compare on `commit_id`.
- A failed `_tag.json` write after a successful CAS is non-fatal but logged.
- Failed pull validation skips the affected entries; those entries never
  enter the baseline.
- A pull that fails during the apply or delete phase saves a baseline
  reflecting only what was actually applied and does not roll back; backups
  are kept for manual recovery.

## Security Notes

- Never log access keys or secret keys.
- Actor and machine labels are visible to anyone with bucket read access;
  they are user-editable display fields, not trusted identity or
  authorization data.
- Treat cloud manifest paths, object keys, head-referenced keys, and declared
  sizes as attacker-controlled input; enforce sizes as streaming caps during
  download.
- Do not follow symlinks during local scans; validate destination ancestor
  chains and destination files with `symlink_metadata` before any pull write
  or delete.
- Do not upload default-excluded cache/runtime paths unless the user
  explicitly opts them in.
- Store cloud credentials in the platform credential store when practical; if
  stored in app data, document that the file contains secrets.
- Backup before overwriting or deleting local files during pull.

## Implementation Status (2026-07-05)

The backend core (steps 1–8 below) is implemented in `lib.rs`: profile cloud
layout (`_head.json`, immutable `_manifests/`, `_commits/`,
`_uploads/{id}/files/{path}` readable snapshots), one profile per agent
root (`head.root`), put-if-absent profile creation, CAS publish
with ambiguous-outcome recovery and rebase-and-retry on a lost race, pull
driven by the head's verified manifest, the union conflict resolution pass
(merge drivers and conflict copies, on both push and pull), per-profile
baselines, the negative-case capability probe with single-writer fallback,
the storage-scoped `list_sync_profiles` command, an in-memory cloud
head+manifest cache (`refresh_cloud_state`; push and pull update it), the
full state-matrix `get_file_statuses` report with the `cloud state
unavailable` degradation, the best-effort running-agent warning, and
actor/machine label editing.
The custom_api backend is removed as specified. The AGENT_SYNC_FILE_SETS.md
tier allowlist is enforced in code: Required + Optional tiers sync by
default, everything else needs a per-storage opt-in, the Never tier is
hard-denied, and conflict-copy siblings inherit the eligibility of the file
they shadow.

2026-07-14 (PLAN_MULTI_STORAGE.md): the config is v2 — N named storages ×
N local profiles, links as matrix edges, ops per link
(`sync_upload/sync_download { storage, profile }`). Baselines and cloud
caches are keyed `(storage id, cloud profile id)`, so same-named profiles
in two storages never share sync state. The multi-remote deviation below
is resolved; the cloud schema is unchanged.

Deliberate deviations from this document, to revisit:

- Push/pull auto-link when the storage has exactly one matching-root
  profile (not already claimed by a sibling link) and auto-create when it
  has none; several candidates require an explicit pin on the link.
- Downloads verify sha256/size after an in-memory fetch; no disk staging or
  streaming size caps yet.
- Case-insensitive manifest collisions are skipped; Unicode-normalization
  (NFC/NFD) collisions are not detected.
- The cache refresh is a full head+manifest GET (no conditional GET against
  the cached head ETag yet).
- Steps 9–11 leftovers: chain-walked commit history has a backend command
  but no UI; cleanup, history pruning, and upload-batch retention are not
  started.

## Implementation Order

1. Add `Remote`, `ProfileLink`, `HeadFile`, `CloudManifest`, `CommitRecord`,
   and scoped local baseline structs.
2. Add safe relative path and key validation with unit tests, including
   collision, reserved-name, separator, and symlink-ancestor cases.
3. Store file payloads under
   `{profile_id}/_uploads/{upload_id}/files/{path}` and make manifest
   entries carry `object_key`; record logical paths in `_upload.json`.
4. Add profile setup commands (head-publish creation) and tag/head discovery.
5. Change pull to read `_head.json`, verify the referenced manifest, and use
   it as the download inventory, with staging-then-apply, SQLite sidecar
   handling, the union pass, and applied-entries-only baselines.
6. Change push to write immutable manifests and commits pre-publish and CAS
   the head, with ambiguous-outcome recovery.
7. Add capability probing and single-writer mode.
8. Add cloud-state caching (conditional GET on head ETag),
   `refresh_cloud_state`, and the expanded status report; expand frontend
   status types and add setup/profile UI.
9. Add actor/machine editing and chain-walked commit history display.
10. Add deletion confirmation, backup retention, and running-process warning.
11. Add history pruning, orphan cleanup, and upload-batch cleanup with the
    retention rules.

## Deferred

- `_bucket.json` shared path rules and `_shared/` object prefix.
- Push to all remotes.
- Conflict resolution UI.
- Explicit deletion propagation with confirmation (v1 never propagates
  deletions; the union restores the file instead).
- Additional merge drivers beyond history.jsonl and session_index.jsonl.
- Profile export/import.
- Profile archival/tombstoning via `head.state`.
- Rollback-to-revision UI (immutable `_manifests/` history makes this
  possible later without schema changes).
- Remote health checks and scheduled sync.
