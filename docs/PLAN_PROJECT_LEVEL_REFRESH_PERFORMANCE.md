# Design: Project-Level Refresh Performance

Status: proposed

This design is based on the active `ProjectSyncV3` path, schema-3 local state,
schema-4 portable manifests, commit `ab8b7d0`, and the current uncommitted
working tree. It does not revive a legacy command or schema.

## Decision

Do not build one more “refresh everything for this project” path.

Project refresh becomes **scoped invalidation**:

- selecting a project renders from already-loaded local metadata and does no
  provider scan or storage request;
- each visible tab refreshes only its own project and section;
- Push and Pull keep their existing review-specific discovery and
  revalidation;
- “Reload app data” remains the explicit global recovery operation; and
- backend caches may accelerate read-only presentation, but can never
  authorize Push, Pull, deletion, installation, or repair.

The first implementation should remove unnecessary work and fix request
ownership before adding a broad cache. Most of the current delay comes from
over-fetching, not from the Tauri bridge itself.

## Current Cost

### Startup and global reload

`ProjectSyncV3.loadShell()` currently performs:

1. host identity verification;
2. config load;
3. project listing, including interrupted-setup and conversation recovery;
4. setup-draft listing;
5. Git detection for every bound project;
6. provider-profile inspection;
7. binding listing;
8. one Codex path audit for every active project; and
9. a second host identity verification.

It then calls `loadProjectData()` for the selected project, which performs:

1. `get_project`;
2. `get_bundle_inventory`, including a full provider walk and a recipe write
   when new conversations are auto-selected; and
3. `get_bundle_readiness`, including a storage engine construction and a
   verified remote head and manifest read.

Once the History tab mounts, it independently starts:

1. `get_project_chat_history`, which walks Codex session directories and maps
   Git history; and
2. `get_project_thread_sync_comparison`, which performs another full provider
   discovery, captures and hashes Codex rollouts, constructs another storage
   engine, and reads the remote snapshot again.

For a linked Codex project, startup or global reload can therefore issue
roughly `14 + active_binding_count` Tauri calls, perform three separate
provider-tree walks for the selected project, and read the same remote
head/manifest twice.

### Project selection

Selecting project B calls `loadProjectData(B)` while the newly mounted History
tab also loads B. The switch can therefore:

- walk provider state for inventory;
- walk and parse provider state for History;
- walk, read, and hash provider state for thread comparison; and
- read the remote snapshot once for readiness and again for comparison.

The shared `inventory` and `readiness` states are attributed to whichever
project is currently active. They currently populate summary fields that no
project or sidebar view consumes. Pull clears and reloads readiness for its
own plan, while Push already receives inventory from `prepare_push_review`.

### History refresh

The History refresh button starts history and comparison concurrently, but
passes `forceRevalidate = true` to history. That bypasses the existing
size/mtime rollout cache and fully parses every project-owned rollout. Thread
comparison separately walks the same provider tree and reads/hashes all local
Codex transcripts.

The existing cache still helps initial non-forced history loads, but it does
not eliminate directory enumeration, session-index reads, duplicate Git work,
or the separate comparison capture.

### Correctness problem coupled to performance

`loadProjectData()` is guarded only by the host workflow generation. Rapid
project changes do not advance that generation, so an older request can write
detail, inventory, readiness, or errors after a newer project is selected.
This is the current audit finding M-04.

A faster refresh must first have precise request ownership. Making the same
race concurrent or cached would make it less visible, not correct.

## Goals and Budgets

The target behavior is:

| Case | Target |
| --- | --- |
| Select an already listed project | Immediate render from shell metadata; no provider or storage I/O |
| Refresh an unchanged local History tab | Metadata enumeration only; no full transcript reads |
| One rollout changed | Read and parse/hash that rollout only |
| Refresh with linked storage | At most one remote head read and, only when changed, one manifest read |
| Switch projects during a request | No result from the old project can commit |
| Refresh failure | Keep the last good section visible and mark only that section stale/error |

Performance tests should enforce work counts, not machine-specific wall-clock
numbers. A local benchmark fixture may additionally target:

- under 100 ms to render a project switch from shell state;
- under 250 ms for an unchanged local-only refresh with 1,000 cached
  rollouts; and
- one head request and zero manifest requests when the remote head token is
  unchanged.

## Refresh Semantics

| User/system event | Refresh scope | Expensive work allowed |
| --- | --- | --- |
| App startup | Shell metadata and recovery | No selected-project inventory or readiness probe |
| Select project | Selected project overview | No provider walk; no storage request |
| Focus returns | Project/config metadata that can change externally | No history, capability, inventory, or remote scan |
| Refresh History | Current project History and its selected-storage comparison | Incremental Codex/Git scan; one remote snapshot check |
| Refresh Skills | Current project capability report | One shared capability discovery; selected-storage check |
| Refresh Plugins | Reuse the current project capability report when its source stamp is unchanged | No second scan merely because the tab changed |
| Open Push | Existing `prepare_push_review` scope | One fresh discovery and one fresh remote snapshot |
| Open/refresh Pull | Immutable restore-plan scope | Plan-pinned reads and existing revision/digest checks |
| Reload app data | Entire shell | Full recovery/reload is allowed and explicitly labeled |

Manual refresh means “reconcile current metadata now,” not “ignore every
valid cache entry.” A deep diagnostic rescan may remain an internal/test
facility, but it is not the normal refresh button.

## Frontend Design

### Separate shell, project, and section state

Replace the overloaded `loading`/`historyRefreshEpoch` model with:

```ts
type ProjectSection = "overview" | "history" | "capabilities";

interface ProjectRequestOwner {
  workflowGeneration: number;
  projectId: string;
  storageId: string | null;
  section: ProjectSection;
  sequence: number;
}
```

- `shellLoading` gates startup and explicit global reload only.
- Project and tab refresh indicators are local to their section.
- A project mutation epoch is keyed by `local_project_id` and section. A Push
  or Pull for project A must not remount or refresh project B.
- Skills and Plugins share one capability snapshot for the same project,
  binding revision, storage, and source stamp.

### Commit only to the request owner

Every async project request captures:

- host workflow generation;
- project ID;
- selected storage ID where relevant;
- section;
- a monotonically increasing sequence.

It may commit only if all five still match. Project selection increments the
project sequence before rendering the new selection. The same guard applies
to focus reconcile and post-Push background reconcile.

This replaces the current assumption that a host-wide generation is also a
project request ID.

### Remove general-load inventory and readiness

`loadProjectData()` becomes a cheap overview load or disappears in favor of
the shell's existing `registrations`, `bindings`, `profiles`, `config.links`,
and repository-kind metadata.

Remove these calls from project selection and global reload:

- `getInventory(projectId)`;
- `getReadiness(storageId, bundleId, binding)`.

Inventory remains owned by `prepare_push_review`. Its conversation
auto-selection side effect therefore happens at the Push review boundary,
where the selection is actually needed. Publishing already discovers new
conversations again and must remain safe when no UI refresh occurred.

Readiness remains owned by Pull review support and post-apply verification.
The Pull flow already clears and reloads it against the current plan.

### Preserve last good content

A section refresh does not set its data to `null`. It:

1. keeps the last good rows visible;
2. shows a single busy indicator for that section;
3. replaces the section only after a current request succeeds; and
4. shows a scoped error with the last successful refresh time on failure.

Changing project, binding, storage, or branch is a context change and may
clear incompatible content. Clicking Refresh is not.

## Backend Design

### Read one consistent project context

Add a read-only repository helper that snapshots the bounded local documents
needed by a project command while holding the persistence lock briefly:

```rust
struct ProjectReadContext {
    project: LocalProjectRegistration,
    link: Option<ProjectStorageLink>,
    binding: Option<ProjectBinding>,
    profiles: Vec<ProviderProfile>,
    materializations_revision: u64,
    dependency_applications_revision: u64,
    conversation_bindings_revision: u64,
}
```

The exact fields may remain private. The important properties are:

- config/binding/profile coordinates come from one coherent read;
- the lock is released before provider traversal, Git commands, or network
  I/O; and
- mutation commands still reload and validate authoritative state.

Do not expose credentials, absolute provider paths, or an internal
storage-config fingerprint in a frontend cursor.

### Add a History-tab composite

The History tab always needs local history and, when a storage is selected,
thread comparison. Replace its two independent first-page refresh commands
with one command:

```rust
struct ProjectActivityRequest {
    local_project_id: LocalProjectId,
    storage_id: Option<StorageId>,
    branch: Option<String>,
    before_time: Option<u64>,
    window_days: Option<u64>,
}

struct ProjectActivitySnapshot {
    history: ProjectChatHistory,
    comparison: Option<ThreadSyncComparison>,
    comparison_error: Option<String>,
    source_version: ProjectSourceVersion,
    refreshed_at: u64,
}
```

The comparison remains degradable: unreachable storage must not hide local
history. Older-page pagination can keep using a history-only path and must
not rerun comparison.

The composite owns:

- one project context;
- one Codex source index;
- one Git snapshot;
- one storage engine;
- one head read; and
- at most one manifest read.

This follows the current `prepare_push_review` pattern: combine work only
when one visible surface needs every result. It is not a generic
all-project/all-tab response.

### Share a provider source index

Introduce a provider-neutral, no-follow source index used by both
`chat_history` and `provider_capture`. For each allowed candidate it records
only bounded derived data:

```rust
struct FileStamp {
    relative_path: String,
    file_type: IndexedFileType,
    size: u64,
    modified_nanos: u64,
    change_identity: Option<PlatformFileIdentity>,
}
```

For a normal refresh:

1. enumerate the same bounded, filtered roots as today;
2. reject or report symlinks and special files exactly as today;
3. stat every candidate so additions, removals, and replacements are seen;
4. reuse parsed ownership/metrics/digest/lineage only when the complete file
   stamp matches;
5. read bounded metadata for new or changed candidates;
6. fully parse or hash only changed project-owned rollouts; and
7. remove cache entries for files no longer present.

Directory mtime alone is not a valid freshness signal: changing a file does
not reliably change every ancestor directory. File watchers may invalidate a
cache early, but watcher events are hints and never the source of truth.

The on-disk index must remain bounded and contain no transcript bytes, chat
turns, credentials, or secret-shaped file content. The existing privacy rule
that avoids persisting title/summary text remains. An in-process cache may
retain the already-rendered derived summary for the app lifetime so an
unchanged untitled rollout is not reparsed on every click.

### Cache Git derivation by immutable coordinates

Within one request, load the first-parent commit rail once. Reuse it both for
mapping and the “has older commits” decision; the current implementation can
run that history load twice.

Across requests, cache parsed commit metadata by:

- canonical repository root;
- selected ref;
- selected ref object ID; and
- a digest of the visible branch refs.

Thread-to-commit mapping still reruns when the project thread set changes,
but unchanged commit objects do not need to be reparsed.

### Reuse verified remote snapshots

Add a bounded in-process snapshot cache keyed by:

- storage ID;
- an internal, secret-safe storage configuration fingerprint;
- bundle ID; and
- the observed `HeadToken`.

Every refresh still reads the mutable head. If generation, commit,
manifest SHA-256, and ETag equal the cached token, reuse the already-verified
manifest. If any coordinate changes, read and verify the immutable manifest
once before replacing the cache entry.

If the head request fails, return a comparison error. The frontend may retain
its prior comparison as stale presentation, but the backend must not label a
cached result current.

Push, Pull, apply, repair, and deletion continue to perform their existing
authoritative revision, digest, generation, expiry, and CAS checks. A
read-only snapshot cache is never an approval or mutation precondition.

## Source Versions and Invalidation

Use existing metadata instead of the global config revision wherever
possible:

| Input | Invalidation coordinate |
| --- | --- |
| Project registration/recipe/base | `LocalProjectRegistration.revision` and recipe revision |
| Binding/path mapping | `ProjectBinding.revision` |
| Assigned provider catalog entries | selected `ProviderProfile.revision` values |
| Restore/materialization state | materializations document revision |
| Dependency state | dependency-applications document revision |
| Conversation identity state | conversation-bindings document revision |
| Provider files | deterministic digest of bounded `FileStamp` entries |
| Git history | selected ref object ID plus branch-ref digest |
| Remote state | storage identity plus `HeadToken` |

An opaque `source_version` returned to the frontend may be a SHA-256 over
these non-secret coordinates. It is a presentation invalidation token, not a
security token and not a portable manifest field.

Host identity rotation clears all host-scoped caches and frontend snapshots.
Binding or profile revision changes invalidate every section for that
project. A storage change invalidates only comparisons using that storage.

## Safety Boundaries

This optimization must preserve the repository's existing safety contract:

- no cache entry can bypass canonical path validation, bounded reads,
  no-follow traversal, or special-file rejection;
- no provider or project bytes are persisted in the metadata index;
- no credentials, storage secrets, absolute paths, trust decisions, plans,
  receipts, or backups enter portable metadata;
- Push capture re-reads and pins the reviewed bytes required by its safety
  design; a read-only digest cache is insufficient;
- Pull/apply continues to use immutable, revision-pinned plans;
- file deletion remains expected-digest checked and directory deletion remains
  exact, empty, and non-recursive; and
- source changes detected during a scan produce a retry/stale result rather
  than a mixed snapshot.

Refresh should become read-only. Recovery, auto-registration/default-profile
creation, and recipe mutation remain explicit startup or workflow behavior,
not incidental effects of a tab refresh.

## Failure Model

Section failures are independent:

- local history succeeds and storage comparison fails: show current local
  history plus a comparison error;
- Git discovery fails: show project sessions and a Git warning;
- one unsafe provider path fails: keep other bounded results and report the
  blocked resource;
- source metadata changes while a changed file is being parsed: discard the
  mixed result and retry once;
- a second change during retry: keep the prior section and report that the
  project is changing too quickly to snapshot;
- active project/storage changes: silently discard the obsolete result; and
- host identity changes: clear host-scoped state and run the existing startup
  identity gate.

## Implementation Order

### Phase 1 — Correct scope and ownership

- Add project/section request ownership and rapid-switch tests.
- Split shell loading from project/section loading.
- Remove inventory and readiness from ordinary project load.
- Stop incrementing one global History epoch for unrelated projects.
- Keep “Reload app data” as the only full shell refresh.

This phase provides the largest improvement without a new cache or portable
schema change.

### Phase 2 — One activity refresh

- Add `ProjectReadContext`.
- Add the composite project-activity command and synchronized TypeScript DTO.
- Reuse one remote snapshot and one Git commit load inside the command.
- Preserve per-section errors and existing single-flight behavior.

### Phase 3 — Metadata-indexed local scans

- Extract the shared provider source index.
- Reuse unchanged rollout ownership and parsed history.
- Cache changed-file digests/lineage for read-only comparison.
- Remove normal UI use of `force_revalidate`; retain deep scan only in
  internal tests/diagnostics.

### Phase 4 — Verified remote and capability reuse

- Add bounded `HeadToken`/manifest reuse.
- Share one capability snapshot between Skills and Plugins.
- Invalidate precisely on project, binding, profile, storage, and source
  versions.

### Phase 5 — Measure and harden

- Add sanitized duration/work counters for development and tests.
- Add large-fixture and fault-injection tests.
- Verify the complete Tauri app and capture a project-switch/refresh recording.

## Test Plan

### Frontend integration

- Rapid A → B → C selection resolves requests out of order; only C commits.
- Project selection does not call inventory or readiness.
- Refreshing History does not reload profiles, bindings, drafts, repository
  kinds for other projects, or the global shell.
- A Push/Pull invalidates only the affected project's relevant sections.
- A failed comparison retains local History and the prior comparison with a
  stale/error marker.
- Skills → Plugins reuses a compatible capability snapshot.

### Rust unit and integration

- An unchanged metadata index reads zero transcript bodies.
- Adding, changing, replacing, archiving, and deleting one rollout invalidates
  exactly the expected entries.
- Same-size replacement is detected by nanosecond/change identity where the
  platform exposes it.
- Symlink and special-file changes never hit a cached success path.
- A changed remote head reads and verifies one manifest.
- An unchanged `HeadToken` reads no manifest.
- Storage failure never returns cached data as current.
- A mutation command still rejects stale generation/revision/digest state
  even when a read-only cache is warm.
- Host rotation and binding/profile changes invalidate the right cache keys.

### Performance fixtures

Use temporary provider homes and local/S3 stubs only:

- 1,000 unchanged Codex rollouts;
- one changed rollout among 1,000;
- two projects sharing one profile;
- multiple storages with one active storage; and
- delayed remote head/manifest responses with request counters.

Assert enumeration, content-read, hash, Git-command, head-read, and
manifest-read counts. Do not run these tests against real provider homes or
`~/.mallard`.

## Acceptance Criteria

The redesign is complete when:

1. project selection performs no provider discovery or storage access;
2. ordinary project refresh cannot update another project's state;
3. History refresh performs one shared local scan and one remote head check;
4. unchanged rollout bodies and unchanged remote manifests are not reread;
5. Skills and Plugins do not duplicate a compatible capability scan;
6. global reload remains explicit and recoverable;
7. Push/Pull safety checks do not consume presentation caches as authority;
8. frontend, Rust, and Tauri contract tests pass; and
9. a large temporary fixture proves bounded work by counters.
