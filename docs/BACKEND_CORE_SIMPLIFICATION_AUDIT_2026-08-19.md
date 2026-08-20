# Mallard backend-core simplification audit

- **Audit date:** 2026-08-19
- **Baseline:** `08977238e5253e487e54d4d65e9c78b37cc4f967` plus the existing staged,
  unstaged, and untracked working-tree changes
- **Scope:** `mallard-domain`, `mallard-infrastructure`, `mallard-application`,
  project-sync orchestration, Storage Catalog orchestration, local
  persistence, provider adapters, and storage transport
- **Purpose:** identify backend designs that repeat work, expose impossible
  states, preserve obsolete paths, or put abstractions at the wrong boundary.
  This is not a recommendation to weaken restore, filesystem, or publication
  safety.

This report is a focused companion to the broader
[codebase simplification audit](./CODEBASE_SIMPLIFICATION_AUDIT_2026-08-17.md)
and [deep complexity audit](./DEEP_COMPLEXITY_AND_EDGE_CASE_AUDIT_2026-08-17.md).
It emphasizes current backend core logic and includes concrete refactoring
examples.

The code snippets below are illustrative shapes, not drop-in patches. Names
can change during implementation; the important part is the ownership and
data-flow boundary.

## Executive assessment

Mallard's backend has necessary safety complexity. It moves conversations and
ordinary files, rewrites provider-native state, publishes immutable content,
and must recover after crashes without clobbering concurrent changes. The
canonical-path checks, no-follow traversal, bounded reads, SHA-256 binding,
review/apply pins, compare-and-swap heads, exact deletion checks, backups, and
recovery journals are justified.

The clearest accidental complexity surrounds that safety core:

1. A per-resource timeline-cache lock protects writes to a shared whole-cache
   document, so the elaborate lock scheme still permits lost updates.
2. Pull apply captures every local transcript and repeatedly reloads machine
   state even when the user approves one conversation.
3. Manifest and plan validation preserve important invariants using
   potentially quadratic collection scans.
4. Push continuity and plugin-apply verification observe the same external
   state twice, wasting work and allowing one result to describe a different
   observation from another.
5. Local-state filesystem primitives are independently reimplemented and have
   drifted to different permission and durability guarantees.
6. The advertised non-Tauri application services call into command modules
   that call back into those services.
7. Several persisted models store facts in multiple writable forms, forcing
   large consistency matrices and making impossible states representable.

The recommended approach is to return reusable evidence from read operations,
index collections once, make lock granularity match persistence granularity,
and establish one-way application dependencies. Safety checks should remain at
untrusted and concurrent boundaries.

## Findings register

Priority is recommended remediation order, not defect severity.

| ID | Priority | Area | Finding |
| --- | --- | --- | --- |
| BCORE-001 | P1 | Session detail | Per-resource locks update one shared cache document and permit lost updates |
| BCORE-002 | P1 | Pull apply | One selected action captures unrelated transcripts and repeatedly scans/reloads state |
| BCORE-003 | P1 | Domain validation | Provider-scope and restore-plan validation contain quadratic paths |
| BCORE-004 | P1 | Review/apply evidence | Remote continuity and plugin inventory are each observed twice per operation |
| BCORE-005 | P1 | Local persistence | Security-sensitive filesystem primitives are duplicated and have drifted |
| BCORE-006 | P2 | Architecture | Application services and command modules form circular boundaries |
| BCORE-007 | P2 | Provider adapters | Provider-neutral timeline pagination is implemented by both adapters and revalidated by core |
| BCORE-008 | P2 | Catalog recovery | The operation journal stores more phases and duplicated evidence than recovery consumes |
| BCORE-009 | P2 | Domain modeling | Plans, plugin intent, and Catalog actions store derivable or duplicated state |
| BCORE-010 | P2 | Activity | Every log page can clean up, count, and then parse the retained log set again |
| BCORE-011 | P2 | Protocol surface | Library commit objects and a generic conversation restore action have no live reader/producer |
| BCORE-012 | P3 | Repeated mechanics | Capture roots, payload hashes, binary resolution, and probe-journal behavior are recomputed or reimplemented |

## Worked example 1: make cache locking match persistence granularity

### Current shape

The timeline cache creates a mutex per resource at
[`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L199),
but each request reads and replaces the same `session_timeline_cache.json` at
[`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L932)
and
[`session_detail.rs`](../crates/mallard-infrastructure/src/session_detail.rs#L1015).

The effective algorithm is:

```rust
let _resource_guard = lock_for(resource_id);
let mut cache = read_whole_cache_document()?;
let index = build_index(resource_id)?;
cache.entries.insert(resource_id.clone(), index);
write_whole_cache_document_atomically(&cache)?;
```

Two distinct resource locks do not serialize the shared-file replacement:

```text
request A locks resource A and reads {}
request B locks resource B and reads {}
request A writes {A}
request B writes {B}    <- A is lost
```

The cache is recoverable, so this generally causes rebuilding rather than
authoritative-data loss, but the lock abstraction is more complex than a
single lock and does not provide the intended guarantee.

### Simpler shape

The smallest correction is one lock for the whole cache document:

```rust
static TIMELINE_CACHE_DOCUMENT_LOCK: Mutex<()> = Mutex::new(());

fn load_or_build_index(resource_id: &ResourceId) -> Result<SessionTimelineIndex, Error> {
    let _guard = TIMELINE_CACHE_DOCUMENT_LOCK.lock()?;
    let mut cache = read_whole_cache_document()?;

    if let Some(valid) = cache.valid_entry(resource_id)? {
        return Ok(valid.clone());
    }

    let built = build_index(resource_id)?;
    cache.entries.insert(resource_id.clone(), built.clone());
    write_whole_cache_document_atomically(&cache)?;
    Ok(built)
}
```

If serializing index construction becomes measurable, retain a short shared
document transaction: build outside the document lock, then acquire it,
reload, merge, and write. Another coherent alternative is one bounded cache
file per resource, in which case the existing per-resource lock matches the
write target.

### Acceptance criteria

- Concurrent builds for two resource IDs preserve both entries.
- Two concurrent builds for one resource do not publish malformed or stale
  metadata.
- Bounded JSON reads, parser-version invalidation, source-file before/after
  checks, source-span validation, and atomic replacement remain intact.

## Worked example 2: load one coherent machine-state snapshot

### Current shape

`load_conversation_bindings` already acquires the persistence lock and loads
identity, config, and machine bindings before loading the conversation
document at
[`persistence.rs`](../crates/mallard-infrastructure/src/persistence.rs#L542).
Callers then frequently load those documents again through separate public
methods. For example, provider review performs four separately locked reads at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L5550), and Pull
preparation repeats three of them at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L10542).

```rust
let machine = repository.load_bindings()?;
let conversations = repository.load_conversation_bindings()?;
let identity = repository.load_or_create_host_identity()?;
let config = repository.load_config()?;
```

This reparses and revalidates the same documents. Because the lock is released
between calls, the returned values are not guaranteed to describe one machine
revision.

### Simpler shape

Expose the unit of state that orchestration actually consumes:

```rust
pub struct ProjectStateSnapshot {
    pub identity: HostIdentity,
    pub config: SyncConfig,
    pub machine: MachineProjectState,
    pub conversations: ConversationBindings,
}

impl LocalStateRepository {
    pub fn load_project_state_snapshot(&self) -> Result<ProjectStateSnapshot, String> {
        let _guard = persistence_guard(&self.root)?;
        let identity = self.load_or_create_host_identity_unlocked()?;
        let config = self.load_config_unlocked()?;
        let machine = self.load_bindings_unlocked(&config, &identity)?;
        let conversations = self.load_conversation_bindings_document_unlocked(
            &config,
            &machine,
            &identity,
        )?;

        Ok(ProjectStateSnapshot {
            identity,
            config,
            machine,
            conversations,
        })
    }
}
```

After a review phase intentionally mutates state, load one fresh snapshot and
pass references to downstream helpers. Do not hold the persistence lock across
provider parsing, cloud I/O, or another long-running operation.

### Acceptance criteria

- One snapshot contains mutually compatible identity, config, machine, and
  conversation revisions.
- Review and apply still recheck the persisted revisions immediately before
  mutation.
- Network and provider work occur after the read lock is released.

## Worked example 3: discover all sessions, capture only selected transcripts

### Current shape

Pull preparation calls `capture_all_local_conversations` at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L10446). This reads,
hashes, and analyzes every local transcript even when the approved Pull action
touches one conversation.

All-session metadata is useful for provider-ID collision prevention and
display names. All-session bytes are not. Combining the two makes unrelated
content part of the selected action's availability boundary: enough unrelated
transcript data can exhaust the aggregate capture limit and fail the Pull.

### Simpler shape

Separate bounded discovery from exact byte capture:

```rust
let session_views = discover_local_sessions(repository, resolved)?;
let selected_resource_ids = actions
    .iter()
    .filter_map(local_resource_needed_for_revalidation)
    .collect::<BTreeSet<_>>();

let captured = capture_selected_conversations(
    repository,
    resolved,
    &session_views,
    &selected_resource_ids,
)?;

let views_by_provider_id = index_session_views(&session_views);
let bindings_by_thread = index_conversation_bindings(&state.conversations);
let profiles_by_id = index_profiles(&state.machine);
```

The selected capture must still perform the same no-follow, containment,
freshness, hard-link, special-file, and byte-limit checks. The simplification
is limiting expensive reads to data capable of affecting the approved action.

### Acceptance criteria

- One selected action does not read unrelated transcript bytes.
- Unrelated transcripts exceeding the aggregate capture limit do not block
  the selected action.
- All discovered provider IDs still participate in collision prevention.
- Keep-Both title lookup, stale local digest detection, and missing-local-file
  behavior remain covered.

## Worked example 4: replace nested validation scans with indexes

### Current shape

Provider-scope validation scans every file for every conversation at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L4523), scans
all files again for each provider-state resource at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L4571), and
walks ancestry from each conversation independently at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L4584).

Pull-plan validation similarly performs a linear action search for each review
row at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L5161).
The current limits permit 20,000 resources and 100,000 files/actions, so these
are not merely cosmetic loops.

### Simpler provider-scope validation

```rust
let mut paths_by_resource = BTreeMap::<ResourceId, BTreeSet<LogicalPath>>::new();

for (path, entry) in &scope.files {
    entry.validate()?;
    paths_by_resource
        .entry(entry.resource_id.clone())
        .or_default()
        .insert(path.clone());
}

for conversation in scope.conversations.values() {
    let declared = conversation
        .files
        .iter()
        .map(|file| file.logical_path.clone())
        .collect::<BTreeSet<_>>();
    let owned = paths_by_resource
        .get(&conversation.resource_id)
        .cloned()
        .unwrap_or_default();

    if declared != owned {
        return Err(conversation_file_ownership_error(&declared, &owned));
    }
}

for provider_state in provider_state_resources {
    if paths_by_resource
        .get(&provider_state.resource_id)
        .is_none_or(BTreeSet::is_empty)
    {
        return Err("provider-state resource owns no files".to_string());
    }
}
```

Use a visited/done set or memoized parent walk for fork-cycle detection so each
ancestry edge is processed once.

### Simpler restore-plan validation

```rust
let mut actions_by_id = BTreeMap::<&ActionId, &ConversationPlanAction>::new();

for action in &plan.conversation_actions {
    action.validate()?;
    if actions_by_id.insert(&action.action_id, action).is_some() {
        return Err(format!("duplicate restore action '{}'", action.action_id));
    }
}

let mut reviewed = BTreeSet::new();
for row in &plan.conversation_review {
    row.validate()?;
    let Some(action_id) = row.action_id.as_ref() else {
        continue;
    };
    let action = actions_by_id
        .get(action_id)
        .ok_or("conversation review row references an unknown action")?;
    validate_review_projection(row, action)?;
    reviewed.insert(action_id);
}
```

Cheap collection bounds should run before scope digest serialization and deep
cross-reference validation. Final validation should occur at persistence and
untrusted-load boundaries; a caller need not invoke the same complete
validation immediately before a save function that repeats it.

### Acceptance criteria

- Preserve rejection of missing, shared, wrongly owned, and undeclared files.
- Preserve rejection of provider-state resources without files.
- Preserve duplicate, unknown, mismatched, and missing review-action errors.
- Preserve fork-cycle rejection.
- Add large synthetic tests or benchmarks demonstrating near-linear scaling.
- Decide whether deterministic first-error ordering is part of the contract
  before changing traversal order.

## Worked example 5: return evidence instead of rereading it

### Push continuity

Push review classifies remote continuity at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L7608), then
destination classification reaches the same continuity probe again through
the code around
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L7718).

Instead of allowing destination classification to read storage again:

```rust
let continuity = classify_remote_continuity(store, coordinates)?;
let destination = classify_push_destination(local, remote, &continuity)?;
```

One operation should use one verified observation. A later operation should
still perform a fresh probe.

### Plugin apply

Plugin apply calls `verify_plugin_apply` at
[`storage_catalog/apply.rs`](../src-tauri/src/storage_catalog/apply.rs#L957).
Both adapters perform full inventory during verification, but apply then runs
inventory again because verification discarded the evidence needed to rebuild
portable intent.

Return the observation:

```rust
pub struct VerifiedPluginApply {
    pub inventory: ProviderInventory,
    pub installed_intent: PluginInstallIntentDescriptor,
}

let verified = adapter.verify_plugin_apply(request)?;
validate_intent_digest(&verified.installed_intent, approved_digest)?;
```

This turns verification from a boolean-like gate into a reusable witness. For
Codex it avoids repeating both marketplace and plugin-list subprocesses.

### General rule

When downstream logic needs facts used by a verifier, the verifier should
return the validated evidence rather than force downstream code to rediscover
the source.

## Worked example 6: establish a one-way application boundary

### Current dependency direction

Project sync currently has the following shape:

```text
Tauri command in commands.rs
  -> ProjectSyncService
       -> repository-based function in commands.rs
```

The service imports command-owned DTOs and orchestration at
[`project_sync/service.rs`](../src-tauri/src/project_sync/service.rs#L7), while
the command module imports and invokes the service at
[`project_sync/commands.rs`](../src-tauri/src/project_sync/commands.rs#L81).

Storage Catalog repeats the same cycle between
[`storage_catalog/service.rs`](../src-tauri/src/storage_catalog/service.rs#L23)
and
[`storage_catalog/resource_commands.rs`](../src-tauri/src/storage_catalog/resource_commands.rs#L460).

### Target dependency direction

```text
src-tauri command adapter
  -> mallard-application use case
       -> mallard-domain values and validation
       -> mallard-infrastructure ports/implementations
```

Responsibilities would be:

| Layer | Owns |
| --- | --- |
| Tauri | IPC argument adaptation, `spawn_blocking`, event/progress emission, activity logging |
| Application | Setup, Push review/apply, Pull review/apply, Library workflows, application DTOs |
| Domain | IDs, manifests, plans, invariants, decisions |
| Infrastructure | Persistence, providers, filesystem work, storage transport |

The temporary module re-exports in
[`project_sync/mod.rs`](../src-tauri/src/project_sync/mod.rs#L12) should be
removed in the same architectural cutover, with direct crate imports replacing
the old namespace.

Do not split the command file into arbitrary smaller files while retaining the
cycle; that moves complexity without fixing ownership. Move one complete use
case at a time and retain Tauri command names and serialized DTO shapes until
the use case is verified.

## Worked example 7: share local-state filesystem primitives

### Current shape

Project persistence, the R2 setup journal, and Catalog persistence each own a
variant of:

- no-symlink traversal;
- restricted directory creation;
- lock-file opening;
- bounded JSON reads;
- temporary-file permissions;
- atomic replacement; and
- parent-directory syncing.

The implementations have drifted. For example, Catalog persistence ignores
directory-permission failures at
[`storage_catalog/persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs#L793),
temporary-file permission failures at
[`storage_catalog/persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs#L914),
and directory-sync failures at
[`storage_catalog/persistence.rs`](../src-tauri/src/storage_catalog/persistence.rs#L933).

### Simpler shape

Provide a small local-state filesystem module with explicit primitives:

```rust
pub fn ensure_private_directory(root: &Path, path: &Path) -> Result<(), StateFsError>;

pub fn open_private_lock(
    root: &Path,
    path: &Path,
    label: &'static str,
) -> Result<LockedFile, StateFsError>;

pub fn read_json_bounded<T: DeserializeOwned>(
    root: &Path,
    path: &Path,
    limit: u64,
) -> Result<Option<T>, StateFsError>;

pub fn write_json_atomic<T: Serialize>(
    root: &Path,
    path: &Path,
    value: &T,
    limit: u64,
) -> Result<(), StateFsError>;
```

Document repositories continue to own their schemas, locks, revision rules,
recovery policies, and error context. The shared module owns only the
security-sensitive mechanics.

Do not automatically merge these primitives with writes to project/provider
content or object-store roots; those have different containment and mutation
contracts.

## Worked example 8: shrink the Catalog recovery representation, not recovery

### Current shape

`CatalogOperationPhase` defines eight phases at
[`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs#L1836).
Recovery around
[`storage_catalog/apply.rs`](../src-tauri/src/storage_catalog/apply.rs#L1299)
primarily distinguishes terminal from nonterminal operations and then derives
the appropriate repair from paths and digests. At least one declared phase,
`Activated`, has no production writer.

Preservation also persists a separate witness at
[`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs#L1764)
that repeats much of the operation record. Successful terminal entries are
retained in bounded history even though no current command or UI reads them.

### Simpler shape

Prefer typed active recovery records over one large optional-field record:

```rust
enum PendingCatalogOperation {
    Preserve {
        operation_id: CatalogOperationId,
        target: LocalCatalogTargetId,
        original_path: PathBuf,
        copy_path: PathBuf,
        original_digest: String,
        copy_digest: String,
    },
    ReplaceDirectory {
        operation_id: CatalogOperationId,
        target: LocalCatalogTargetId,
        target_path: PathBuf,
        staged_path: PathBuf,
        backup_path: Option<PathBuf>,
        expected_before: CatalogTargetExpectation,
        incoming_digest: String,
    },
}
```

Persist a state transition only when it changes what startup recovery must do.
Use verified filesystem evidence to decide whether a copy, backup, or incoming
target was activated. Delete successful records unless a product-visible audit
history is explicitly required.

This is not permission to remove:

- staging;
- target locking;
- no-clobber activation;
- exact backup rename and restoration;
- post-activation verification;
- nonrecursive cleanup; or
- startup recovery.

Required tests should stop execution after initial journaling, copy activation,
backup rename, incoming activation, and cleanup, then restart recovery and
verify the exact outcome.

## Worked example 9: remove derivable states from serialized models

### Plugin intent

The plugin descriptor around
[`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs#L615)
stores the marketplace namespace three ways, uses a source enum with one
variant, stores a resolution policy derivable from Git-source presence, and
contains an optional requirement that validation always rejects.

A smaller current-product representation is:

```rust
struct PluginInstallIntentDescriptor {
    provider: ProviderKey,
    plugin_id: String, // validated name@namespace
    marketplace_git: Option<GitMarketplaceSource>,
}
```

Derive the namespace from `plugin_id` and manual/provider-resolved behavior
from `marketplace_git`.

### Catalog Pull actions

`CatalogPullActionKind` at
[`storage_catalog/domain.rs`](../src-tauri/src/storage_catalog/domain.rs#L2115)
has eleven variants, while execution at
[`storage_catalog/apply.rs`](../src-tauri/src/storage_catalog/apply.rs#L1135)
groups them into three behaviors. Skill versus pet is already in the resource,
install versus update is already in the target expectation, and destructive
overwrite is already in the approval flag.

An executor-oriented shape would be:

```rust
enum CatalogPullOperation {
    ReplaceDirectory,
    PreserveAndReplace { copy_name: String },
    ApplyPlugin,
}
```

UI labels can be derived from operation, resource kind, target expectation,
and destructive-approval state.

### Conversation plans

`RestorePlan` stores executable conversation actions and review rows that
repeat the actions' provider, IDs, resource, and action kind at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L5044). At a
minimum, use one action index for validation. A deeper reduction would derive
executable review rows from actions and persist only informational or blocked
rows separately.

Common host/project/bundle/expiry pins should live once at plan level;
provider/profile/scope pins can live once per provider; only thread tails,
targets, codec evidence, and identities need to remain action-specific.

These model reductions affect serialization and content digests. Treat them as
explicit clean cutovers, update Rust and TypeScript together, and do not add
compatibility fallbacks.

## Additional exact cleanup candidates

The following are smaller than the worked examples but still concrete:

- Activity log query invokes cleanup before each page at
  [`src-tauri/src/activity_log.rs`](../src-tauri/src/activity_log.rs#L126).
  Cleanup computes full statistics, and query then parses the retained set
  again. Enforce retention at startup, policy change, and the existing
  periodic append schedule, or share one scan.
- Bundle publication hashes each captured payload for the manifest and hashes
  the same bytes again for pending upload around
  [`bundle_engine.rs`](../crates/mallard-infrastructure/src/bundle_engine.rs#L639).
  Reuse the manifest-entry digest.
- Provider capture repeatedly canonicalizes provider and excluded roots per
  selected file around
  [`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L1836).
  Canonicalize roots once per capture and return inspected metadata from the
  file validator.
- Provider capture has two provider-to-domain conversion paths despite
  `domain_resource_descriptor` being documented as the translation boundary
  at
  [`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L798).
  Use one typed conversion path and `LogicalPath::parse`/`ResourceId::parse`.
- Direct R2 setup and Cloudflare provisioning implement the same probe-journal
  lifecycle in
  [`storage_setup.rs`](../crates/mallard-application/src/storage_setup.rs#L560)
  and
  [`cloudflare_r2_setup.rs`](../crates/mallard-application/src/cloudflare_r2_setup.rs#L627).
  Share `begin`, `finish`, and `reconcile` mechanics while leaving token
  management outside.
- Terminal resume duplicates the complete Codex binary resolver. Use
  [`codex_binary::find_binary`](../crates/mallard-infrastructure/src/codex_binary.rs#L109)
  instead of the narrower resolver at
  [`providers/codex/actions.rs`](../crates/mallard-infrastructure/src/providers/codex/actions.rs#L164).
- `SessionAdapter::capability()` at
  [`providers/traits.rs`](../crates/mallard-infrastructure/src/providers/traits.rs#L524)
  has no call sites and duplicates `ProviderDescriptor.capabilities`.
- `thread_review` at
  [`project_sync/commands.rs`](../src-tauri/src/project_sync/commands.rs#L4614)
  catches provider failures internally and has no `Err` path, so its `Result`
  wrapper is unnecessary.
- `Projector::new` at
  [`codex_paginated_projection.rs`](../crates/mallard-infrastructure/src/codex_paginated_projection.rs#L147)
  has no failure path but returns `Result`.
- The match at
  [`project_sync/commands.rs`](../src-tauri/src/project_sync/commands.rs#L11702)
  maps every action to `RestoreActionType::MaterializeConversation`.
- `RestoreActionKind::MaterializeConversation` at
  [`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L4942)
  has no production constructor. The generic engine only prefetches it and
  then blocks it because typed conversation apply owns the operation.
- Library commit objects are written during publication at
  [`storage_catalog/resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs#L439),
  but no Library path reads a commit, traverses its predecessor, or exposes
  history. If no external raw-storage consumer exists, immutable manifests
  plus the CAS head provide the current supported behavior without the second
  immutable object.
- `LibraryResourceEngine::read_tag` computes a digest every caller discards at
  [`storage_catalog/resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs#L160).
- Catalog publication serializes the manifest again solely to obtain its size
  at
  [`storage_catalog/resource_engine.rs`](../src-tauri/src/storage_catalog/resource_engine.rs#L479).
  Preserve the original byte length before moving the buffer.
- Catalog inventory walks skill and pet directories a second time for newest
  modification time at
  [`storage_catalog/inventory.rs`](../src-tauri/src/storage_catalog/inventory.rs#L65).
  Accumulate it during capture.
- Historical-manifest inspection and optional continuity reads duplicate most
  validation and object-reading logic around
  [`bundle_engine.rs`](../crates/mallard-infrastructure/src/bundle_engine.rs#L966).
  One private verified-read helper can return `Verified`, `Missing`, or
  `Mismatch`, leaving public callers to interpret the result.

## Complexity to preserve

The following checks are not considered redundant merely because nearby code
performs a similar-looking check:

- Canonical containment, component-by-component no-follow traversal, and
  symlink/hard-link/special-file rejection protect separate path and TOCTOU
  boundaries.
- Declared `Content-Length` and actual streamed byte counts are both needed
  because remote headers and bodies are untrusted.
- Source metadata checks before and after reading provider files detect
  concurrent replacement.
- Payload, manifest, and head SHA-256 checks bind different protocol objects.
- Review-time and apply-time revision/digest/expiry checks close different
  race windows.
- Immutable payload/manifests and a CAS-protected mutable head are the core
  publication contract.
- Expected local digests and exact-empty-directory requirements must remain on
  deletion.
- Atomic writes, file sync, directory sync, process locks, and OS file locks
  address different crash and concurrency failures.
- S3 readback after an ambiguous conditional write or delete is needed because
  a timed-out request may have committed remotely.
- Recovery journaling itself remains necessary; the opportunity is to reduce
  duplicated fields and unused intermediate states.
- R2 credential zeroization, cleanup-required records, token adoption checks,
  and the rule against revoking committed credentials remain necessary.

## Recommended implementation sequence

### Phase 1: correctness and coherent evidence

1. Fix timeline-cache lock/persistence granularity.
2. Compute remote continuity once per review.
3. Return verified plugin inventory/intent from plugin apply verification.
4. Add concurrency and call-count regression tests.

### Phase 2: bound work to the selected operation

1. Add a coherent project-state snapshot read.
2. Separate session discovery metadata from selected transcript capture.
3. Build action/binding/profile/session indexes once per operation.
4. Linearize restore-plan and provider-scope validation.

### Phase 3: consolidate mechanics and ownership

1. Share local-state filesystem primitives.
2. Move project-sync and Catalog use cases into `mallard-application`.
3. Make Tauri modules thin adapters.
4. Remove temporary re-export shims after direct imports are in place.
5. Move provider-neutral pagination into the common timeline layer.

### Phase 4: model cutovers

1. Simplify Catalog recovery records with crash-point coverage.
2. Reduce plugin-intent and Catalog-action state dimensions.
3. Group common conversation pins and derive executable review rows.
4. Remove write-only Library commits and the dead generic conversation action
   only after confirming there is no external consumer.

### Phase 5: mechanical cleanup

1. Reuse payload digests and canonical roots.
2. Consolidate R2 probe journaling and Codex binary lookup.
3. Remove unnecessary `Result`/`Option` wrappers and identical match branches.
4. Remove unused fields only after Rust, TypeScript, fixtures, and command
   contracts are updated together.

## Verification expectations

Use targeted tests while implementing each phase, followed by the backend
handoff set:

```text
cargo test --workspace --lib
npm run test:backend-integration
cargo clippy --workspace --all-targets
```

For Tauri DTO or command-surface changes, also run:

```text
npm run build
npm run test:frontend-integration
```

Model or persistence cutovers should add focused tests for malformed documents,
duplicate IDs, unknown references, stale revisions, interrupted writes, exact
recovery, and deterministic error behavior where callers rely on it. Storage
tests that bind loopback listeners may require the environment's explicit
localhost permission.

## Audit verification

The audit used current-source inspection and targeted Clippy lints for
unnecessary wrappers, identical match arms, redundant closures, boolean
simplification, type complexity, and large parameter lists. The all-target
Clippy invocation completed successfully and corroborated several of the
smaller findings. No repository files other than this report were changed by
the audit.
