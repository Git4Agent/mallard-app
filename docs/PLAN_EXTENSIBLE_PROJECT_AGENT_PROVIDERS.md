# Plan: Extensible Project Agent Providers and Mixed-Provider Repositories

**Status:** design history; resource-contract section superseded
**Date:** 2026-08-07  
**Amended:** 2026-09-03
**Scope:** architecture and implementation plan only

## 1. Outcome

Replace the fixed Codex-or-Claude project model with an extensible provider
model while making mixed-provider repositories a first-class product
contract.

A project on one machine may bind:

- Codex only;
- Claude only; or
- Codex and Claude together.

The portable repository is independent of that local choice. One repository
may contain Codex, Claude, and future-provider sessions at the same time. A
local provider binding grants permission to discover and change that
provider's local state; it does not control whether the provider's remote
sessions are visible.

For example, a user who binds only Codex to a mixed Codex/Claude repository:

- sees both Codex and Claude sessions in Activity;
- can inspect verified remote Claude metadata and, when the installed adapter
  supports it, an on-demand remote transcript preview;
- can select and Push or Pull Codex changes;
- cannot select, materialize, overwrite, delete, or acknowledge Claude
  changes; and
- preserves the complete Claude portion of the repository when publishing a
  Codex change.

The provider architecture must also make adding a third built-in provider an
adapter-and-registration task. Adding one must not require new branches in
project setup, Activity composition, repository continuity, reconciliation,
review selection, or the common frontend shell.

## 2. Settled decisions

These decisions are part of the requested product contract and should not be
reopened during implementation.

### D1 — Local provider selection controls mutation, not visibility

The local binding is the authority for filesystem access and provider
mutation. The remote manifest is the authority for what can be viewed from
storage.

| Local profile | Remote sessions | Activity | Push/Pull |
| --- | --- | --- | --- |
| Selected | Present | Local and remote state | Selectable and syncable |
| Selected | Absent | Local-only sessions | Can Push |
| Not selected | Present | Remote sessions, view-only | No provider actions |
| Not selected | Absent | Nothing to show | No action |

Setup requires at least one provider, not every registered provider.

### D2 — Repositories are mixed-provider containers

A project repository may contain any number of provider slices. Codex and
Claude are the first two registered providers, not hard-coded repository
columns. A provider slice is retained even when the current machine has no
profile for it.

### D3 — Providers are validated keys, not a closed enum

Replace Rust `Provider::{Codex, Claude}`, the TypeScript
`"codex" | "claude"` union, and frontend arrays such as
`PROJECT_PROVIDERS` with a shared validated `ProviderKey`.

Provider keys use a bounded lowercase ASCII namespace:

```text
[a-z][a-z0-9._-]{0,63}
```

Slashes, colons, path components, control characters, and Unicode lookalikes
are forbidden. The key is safe to use in a logical namespace only after the
normal logical-path validation still runs.

### D4 — Built-in adapters, not runtime plugins

This redesign creates compile-time provider adapters registered with Mallard.
It does not load third-party code, define a stable dynamic ABI, or execute
commands supplied by a manifest. A future built-in provider is added as a new
adapter module and one registry entry.

### D5 — Unknown remote providers are view-only and preserved

Manifest validation accepts a structurally valid `ProviderKey` even when the
running binary has no adapter registered for it. Unknown provider slices:

- show generic manifest metadata in Activity;
- cannot expose parsed transcript details;
- cannot be selected or applied;
- cannot create local paths or commands; and
- are copied into a new manifest unchanged, with the same slice revision and
  digest, whenever another slice is published.

This is forward extensibility inside the redesigned format. It is not a
reader for pre-cutoff Mallard data.

### D6 — Continuity and review are provider-scoped

A single repository-wide reviewed base would make unrelated providers block
each other. The new format therefore tracks an independent revision and
digest for every provider slice plus a separate project-content slice.

A remote Claude-only change does not force a Codex-only user to Pull before
Pushing Codex. A remote Codex change does.

### D7 — Viewing is not acknowledgment

Listing or expanding a remote session does not advance its reviewed base.
Only an approved Push, Pull, or explicit provider-scoped acknowledgment may
advance the corresponding local scope base. An unbound provider has no
acknowledgment action.

### D8 — Provider failures are isolated

An unreadable Claude profile must not prevent a safe Codex Push, and the
reverse is also true. The affected provider rows fail closed. Storage
integrity failures, repository identity failures, and global head
continuity/replacement failures remain whole-operation blockers.

### D9 — No backward compatibility

All redesigned project-sync documents continue to use schema 1, and portable
repositories continue to use the existing `.mallard/v1` namespace. This is
an in-place replacement of the schema-1 contract, not a new schema
generation.

There is no migration, legacy reader, dual reader, alias command, tolerant
old DTO, or read-old/write-new path. Removing `~/.mallard` and existing
remote Mallard data is a precondition for the first test. Pre-cutoff
schema-1 data is outside the new contract even though it carries the same
numeric identifier; the implementation does not attempt to distinguish,
convert, or salvage it.

### D10 — No provider-specific fallback in the core

If an adapter cannot prove a safe operation, the operation is typed as
view-only, backup-only, or blocked according to its declared capability. The
core never falls back to generic copying based on a provider name.

## 3. Current implementation gaps

The current code has useful provider-neutral pieces, but the active product
path remains closed in several places:

- `mallard_domain::Provider` is a two-variant enum.
- `ProjectSetupWorkspace` and `ProjectBindingEditor` normalize selections to
  one provider through `configuredProjectProvider` and
  `singleProviderSelection`.
- `ProjectLinksWorkspace` treats more than one provider as an invalid state.
- `chat_history.rs`, its Tauri commands, `ProjectChatHistoryPage`, and their
  DTOs are Codex-specific.
- `ThreadSyncComparison` and the typed conversation planner scan Codex only.
- Claude sessions use a generic resource/materialization path rather than the
  typed conversation reconciliation path.
- Push continuity is repository-wide, so a change in one provider currently
  blocks an unrelated provider.
- Frontend actions and copy infer capabilities by checking for Codex.

The redesign replaces these paths. It must not add a parallel abstraction
while leaving the old path registered or reachable.

## 4. Terminology

| Term | Meaning |
| --- | --- |
| **Provider key** | Stable identifier such as `codex` or `claude` |
| **Provider adapter** | Built-in implementation of profile, session, codec, materialization, and action capabilities |
| **Provider profile** | Machine-local path and identity for one provider environment |
| **Bound provider** | Provider with a selected profile in this project's local binding |
| **View-only provider** | Provider present remotely but not bound locally |
| **Provider slice** | Independently revisioned provider-owned portion of a repository |
| **Project slice** | Independently revisioned project-file and shared-requirement portion |
| **Sync scope** | A provider slice or the project slice |
| **Scope base** | Machine-local last reviewed revision and digest for one sync scope |
| **Writable scope** | Scope the current reviewed operation is authorized to mutate |

## 5. Domain model

### 5.1 Provider identity

Move the canonical provider identity into the Tauri-free domain crate so
project sync, infrastructure, and the storage catalog can share the validated
key without sharing feature-specific profiles or DTOs.

```rust
#[serde(transparent)]
struct ProviderKey(String);
```

`ProviderKey` validation is structural only. Domain deserialization does not
require a registered adapter because unknown remote providers must remain
readable and preservable. Commands that create or mutate local provider state
must additionally resolve the key through the registry.

### 5.2 Local provider profiles and bindings

```rust
struct ProviderProfile {
    profile_id: LocalProviderProfileId,
    provider: ProviderKey,
    display_name: String,
    path: String,
    canonical_path: String,
    revision: u64,
    created_at: u64,
    updated_at: u64,
}

struct ProjectBinding {
    host_id: HostId,
    replica_id: ReplicaId,
    local_project_id: LocalProjectId,
    bundle_id: BundleId,
    project_root: String,
    canonical_project_root: String,
    profile_ids: BTreeMap<ProviderKey, LocalProviderProfileId>,
    state: BindingState,
    revision: u64,
    updated_at: u64,
}
```

An active binding must contain at least one profile. It may contain at most
one profile for a provider. Every profile must:

- exist in the machine profile catalog;
- carry the same provider key as the map entry;
- resolve to its recorded canonical path;
- not overlap the project, Mallard data, local storage, or another selected
  provider home; and
- not already be claimed by another active project on the same canonical
  checkout.

Runtime provider homes are resolved through the selected profiles into an
operation context. Do not retain `codex_home` and `claude_home` fields on the
generic binding.

### 5.3 Setup drafts

Setup drafts retain a map because each provider selection is independently
incomplete:

```rust
struct ProjectSetupDraft {
    // identity and project fields omitted
    profiles: BTreeMap<ProviderKey, DraftProfileSelection>,
    // storage and repository fields omitted
}
```

An absent key means the provider is disabled for this project. Finalization
requires at least one entry and validates every entry through its registered
profile adapter. Selecting or editing one provider never removes another
entry.

### 5.4 Generic resource classes

> **Superseded extension contract:** The closed `ResourceKind` shape below
> records the earlier provider cutover. It is not the final portable wire
> contract. Sections 4.4 and 5.4 of
> `DESIGN_EXTENSIBLE_STORAGE_AND_AGENT_PROVIDERS.md` replace `ResourceKind`,
> `ApplyPolicy`, `Provenance`, and the dependent `TombstoneTarget` with stable
> versioned keys and compiled registries.

Remove provider names from `ResourceKind`:

```rust
enum ResourceKind {
    Conversation,
    ProviderState,
    ProjectContentFile,
    ProjectContentDirectory,
    Requirement,
}

struct ResourceDescriptor {
    resource_id: ResourceId,
    kind: ResourceKind,
    provider: Option<ProviderKey>,
    provider_resource_type: Option<ResourceTypeKey>,
    // existing common fields
}
```

Examples of `provider_resource_type` are `session`, `memory`, `todo`, and
`file-history`. Core behavior keys off `ResourceKind`, ownership, policy, and
adapter capabilities—not these presentation-oriented subtype strings.

Conversation resources must have a provider. Project content must not.

## 6. Provider registry and adapter contracts

### 6.1 Registry

Create a registry in `crates/mallard-infrastructure/src/providers/` and
inject it into command orchestration and tests.

```rust
struct ProviderRegistry {
    adapters: BTreeMap<ProviderKey, Arc<dyn AgentProviderAdapter>>,
}
```

Production registers Codex and Claude. Tests register a minimal third
provider to prove that the common layers are actually extensible.

The registry returns a serializable descriptor for setup and Activity:

```rust
struct ProviderDescriptor {
    key: ProviderKey,
    display_name: String,
    profile_label: String,
    default_profile_directory: Option<String>,
    setup_default: bool,
    capabilities: ProviderCapabilities,
    adapter_version: u32,
}
```

`setup_default` may pre-enable a provider for a new draft but is never a
requirement. The user may disable it as long as at least one provider remains.

### 6.2 Adapter surface

Keep filesystem and provider behavior out of the domain crate. The
infrastructure adapter exposes focused capabilities:

```rust
trait AgentProviderAdapter: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn profiles(&self) -> &dyn ProfileAdapter;
    fn sessions(&self) -> Option<&dyn SessionAdapter>;
}

trait ProfileAdapter {
    fn probe(&self, requested: &Path, context: &ProfileProbeContext)
        -> Result<ProfileProbe, String>;
    fn inspect(&self, profile: &ProviderProfile)
        -> Result<ProfileHealth, String>;
}

trait SessionAdapter {
    fn discover(&self, context: &SessionDiscoveryContext)
        -> Result<Vec<DiscoveredSession>, String>;
    fn summarize(&self, source: &SessionSource)
        -> Result<ProjectSessionSummary, String>;
    fn details(&self, source: &SessionSource, cursor: Option<usize>, limit: usize)
        -> Result<ProjectSessionDetailsPage, String>;
    fn analyze(&self, bytes: &[u8]) -> SessionAnalysis;
    fn rewrite(&self, request: SessionRewriteRequest)
        -> Result<Vec<u8>, String>;
    fn materialization_targets(&self, request: MaterializationRequest)
        -> Result<Vec<ProviderTarget>, String>;
    fn available_actions(&self, context: &SessionActionContext)
        -> Vec<SessionActionDescriptor>;
    fn execute_action(&self, request: SessionActionRequest)
        -> Result<SessionActionResult, String>;
    fn preserve_divergence(&self) -> Option<&dyn ConflictPreserver>;
}
```

Exact Rust signatures may be refined during implementation, but the
ownership boundary is fixed:

- adapters parse provider files and construct provider targets;
- the core validates plans, digests, revisions, approvals, and paths;
- Tauri commands orchestrate but do not contain provider format logic; and
- React renders returned descriptors and actions but does not construct
  provider commands.

### 6.3 Capability levels

Use typed capability levels rather than independent booleans:

```text
activity_only
backup_only
resumable
resumable_with_conflict_preservation
```

- `activity_only`: local/remote metadata may be displayed; no payload sync.
- `backup_only`: selected local sessions may be stored, but Pull cannot make
  them resumable.
- `resumable`: create and proven fast-forward operations are available;
  divergence is blocked.
- `resumable_with_conflict_preservation`: the complete decision matrix is
  available.

Codex and Claude must finish this redesign at the full capability level.

### 6.4 Safe session actions

The backend returns actions with opaque IDs and presentation labels:

```rust
struct SessionActionDescriptor {
    action_id: String,
    kind: SessionActionKind,
    label: String,
    enabled: bool,
    disabled_reason: Option<String>,
}
```

The frontend sends the opaque action ID and session identity back to a
generic command. The backend re-resolves the registered adapter and validates
ownership before executing. Remote manifests never supply executable command
text.

## 7. Portable repository model

### 7.1 Scope layout

Replace the single flat resource/recipe namespace with independently hashed
scopes:

```rust
enum SyncScopeKey {
    Project,
    Provider(ProviderKey),
}

struct BundleManifest {
    schema_version: u32,
    revision: u64,
    commit_id: String,
    updated_at: u64,
    bundle: BundleIdentity,
    published_by_origin: PortableOrigin,
    scopes: BTreeMap<SyncScopeKey, BundleScope>,
}

struct BundleScope {
    revision: u64,
    content_sha256: String,
    recipe: BundleRecipe,
    conversations: BTreeMap<ThreadId, ConversationThread>,
    resources: BTreeMap<ResourceId, ResourceDescriptor>,
    files: BTreeMap<LogicalPath, BundleFileEntry>,
    directories: BTreeMap<LogicalPath, BundleDirectoryEntry>,
    tombstones: BTreeMap<String, Tombstone>,
    captured_with: CapturedWith,
}
```

The precise canonical encoding used to calculate `content_sha256` must be
owned by the domain serializer and covered by deterministic serialization
tests. The digest excludes its own field and includes the scope revision and
all live/tombstoned content.

Every reference must remain inside its scope:

- a conversation references only a resource and files in its provider scope;
- provider resources cannot point into another provider scope;
- project files and directories live only in the project scope; and
- collection limits apply both per scope and to the summed manifest.

### 7.2 Logical paths

Canonical conversation files use:

```text
state/conversations/<provider-key>/<thread-id>/transcript.jsonl
state/conversations/<provider-key>/<thread-id>/sidecars/<kind>/<file>
```

Provider state that is not owned by one conversation remains under a bounded
provider namespace:

```text
state/providers/<provider-key>/...
```

The adapter proposes provider-relative targets; the core performs the final
canonical-boundary, no-follow, special-file, collision, size, and digest
checks.

### 7.3 Global head and immutable publication

The repository keeps one global CAS head and immutable manifests. A Push may
replace only the reviewed writable scopes. Every other scope is carried from
the verified current manifest with exactly the same scope revision and
content digest.

Payload objects remain content-addressed and immutable. Carrying an unbound
scope forward does not require downloading its payload bytes.

### 7.4 Scope bases

Replace the single per-storage recipe base with a repository base plus
scope-specific reviewed bases:

```rust
struct RepositoryBase {
    revision: u64,
    commit_id: String,
    manifest_sha256: String,
    manifest_size: u64,
}

struct ScopeBase {
    scope: SyncScopeKey,
    scope_revision: u64,
    scope_sha256: String,
    reviewed_in: RepositoryBase,
    binding_revision: u64,
    last_pull_at: Option<u64>,
    last_push_at: Option<u64>,
}
```

`ProjectStorageLink` stores a `BTreeMap<SyncScopeKey, ScopeBase>`. A provider
that is merely viewed has no base update.

### 7.5 Provider-scoped continuity algorithm

For a Push with writable scopes `W`:

1. Read and verify the current global head and manifest.
2. Validate repository identity and global history continuity. A missing or
   replaced repository remains a whole-operation blocker.
3. For every scope in `W`, compare the current scope revision/digest with its
   reviewed base.
4. Require Pull only for writable scopes that advanced or have unreviewed
   remote content.
5. Build new versions only for approved writable scopes.
6. Copy every scope outside `W` from the verified current manifest unchanged.
7. Publish immutable objects and a combined manifest through global CAS.
8. If CAS loses, read the winning manifest. Retry only when every writable
   scope still matches the reviewed inputs; graft the prepared writable
   scopes onto the winner's unchanged non-writable scopes.
9. Update bases only for successfully published scopes.

This allows a Codex Push to coexist with an unrelated concurrent Claude
Push while still preventing two writers from overwriting the same provider
slice.

For Pull:

1. Show every remote provider scope in Activity and review.
2. Build actions only for bound providers with sufficient capabilities.
3. Keep unbound and unsupported provider rows view-only.
4. Apply and advance bases only for approved bound scopes.
5. Do not advance a base because a row was listed, expanded, or left
   unselectable.

### 7.6 Mutation firewall

The publisher must enforce this invariant at the final request boundary, not
only in the UI:

> A publication may change only scopes named by the reviewed writable-scope
> set. Every other scope must retain its exact revision and content digest
> from the CAS input manifest.

Any unexpected change is a fatal consistency error before the head write.

## 8. Activity and remote viewing

### 8.1 Provider-neutral DTOs

Replace the Codex-named history contracts with:

```rust
struct ProjectActivity {
    project_id: LocalProjectId,
    sessions: Vec<ProjectSessionSummary>,
    git: Option<GitHistoryPage>,
    unmapped: Vec<UnmappedSessionReference>,
    provider_states: Vec<ProjectProviderState>,
    warnings: Vec<String>,
    window_start: u64,
    window_end: u64,
    next_before: Option<u64>,
}

struct ProjectSessionSummary {
    resource_id: ResourceId,
    provider: ProviderKey,
    provider_session_id: Option<String>,
    logical_thread_id: Option<ThreadId>,
    source: SessionSourceState,
    access: ProviderAccessState,
    sync_state: SessionSyncState,
    title: String,
    summary: Option<String>,
    started_at: Option<u64>,
    ended_at: Option<u64>,
    relative_cwd: Option<String>,
    branch: Option<String>,
    recorded_sha: Option<String>,
    metrics: SessionMetrics,
    actions: Vec<SessionActionDescriptor>,
}
```

Use `resource_id`, not the provider-native ID alone, as the frontend key and
selection identity. Different providers may use the same native session ID.

### 8.2 Activity composition

For every bound provider, discover local sessions through its adapter. For
every remote provider scope, read session presentation metadata from the
verified manifest. Merge the two sets by provider and logical identity, then
sort deterministically by activity time, provider key, and resource ID.

Provider access is derived as:

```text
bound_ready
bound_read_only
bound_unavailable
view_only
unsupported
```

Rows remain visible when they are not selectable. Example copy:

```text
Claude · Storage only
View only — connect a Claude profile to sync this session.
```

### 8.3 Remote presentation metadata

Store bounded, provider-neutral presentation metadata with each portable
conversation: title, start/end times, relative cwd, branch/commit evidence,
and metric counts when proven. Do not store chat previews in manifest
metadata. A preview requires an explicit, verified, bounded transcript fetch.

For a registered but unbound provider, the adapter may parse the fetched
remote transcript without a local profile because this operation is
read-only and creates no provider target. For an unknown provider, show only
generic manifest metadata.

### 8.4 Git mapping

Normalize provider-specific Git evidence in the adapter. Feed the normalized
branch, recorded SHA, and activity interval into one common mapping engine.
Ambiguous or insufficient evidence remains unmapped. Mapping behavior must
not branch on the provider key.

### 8.5 Frontend behavior

Rename `ProjectChatHistoryPage` to `ProjectActivityPage`. It renders:

- all locally discovered sessions for bound providers;
- all current remote sessions, including unbound providers;
- a provider badge and access/sync state on every row;
- checkboxes only for selectable actions in the current review direction;
- provider actions returned by the backend; and
- a safe generic icon when no provider-specific presentation asset exists.

The setup and Activity UIs iterate backend provider descriptors. They must
not contain a fixed provider array or exhaustive provider-name switch.

## 9. Conversation sync

### 9.1 Canonical conversation model

The core reconciliation engine consumes only normalized session analysis:

```rust
struct SessionAnalysis {
    source_provider_session_id: Option<String>,
    source_cwd: Option<String>,
    record_count: usize,
    lineage: CodecLineage,
    presentation: SessionPresentation,
}
```

The adapter owns extraction and structural rewriting. The core owns chain
validation, relation classification, plan policy, origin, revisions, and
digests.

### 9.2 Codex and Claude cutoff

Move existing Codex behavior into a Codex adapter without changing its safety
invariants. Build a real Claude codec and materializer rather than retaining
the generic Claude file path.

The Claude adapter must:

- derive durable canonical messages from bounded JSONL records;
- validate session ID, cwd, message IDs, parentage, and linearity;
- associate transcript sidecars only through proven session identity;
- rewrite only declared structural session-ID and cwd fields;
- round-trip the rewritten transcript through the same codec;
- prove unchanged lineage and semantic digests after structural rewriting;
- verify destination bucket and file ownership; and
- preserve a divergence as a separately identified, resumable Claude session
  before an approved overwrite.

Malformed, ambiguous, non-quiescent, or unsupported transcripts are blocked.
They do not fall back to generic conversation copying.

### 9.3 Plan pins

Every conversation action pins:

- provider key;
- adapter version;
- codec key and version;
- local provider profile ID and revision;
- sync scope revision and digest;
- project binding revision;
- conversation binding document revision;
- global head revision, commit, manifest size, and manifest digest;
- local target expectations;
- provider quiescence evidence; and
- plan creation and expiry timestamps.

A multi-provider Push plan cannot retain one top-level
`provider_profile_id`. Provider/profile pins belong to each action or to a
provider-specific action group inside the plan.

### 9.4 Provider-isolated results

One stale or failed conversation produces a per-resource result and does not
erase successful actions from another provider. Journal recovery remains
provider/profile scoped. Whole-operation failure is reserved for shared
state, storage integrity, or publication failures where a partial result
cannot be committed safely.

## 10. Setup and project settings UX

The setup Agent section becomes a dynamic list of provider cards:

```text
[x] Codex
    Profile: Default Codex

[ ] Claude
    Not connected on this machine
    Remote repository contains 8 Claude sessions
```

Rules:

- at least one provider must be enabled;
- each enabled provider requires one valid profile;
- disabled providers retain no draft profile selection;
- remote provider counts are informational and never force enablement;
- profile choices update independently;
- custom-path creation remains draft-only until finalization;
- selected provider homes may not overlap each other; and
- finalization creates exactly the profiles and binding entries reviewed.

Project settings reuse the same provider editor. Adding or removing a
provider increments the binding revision and invalidates plans for that
provider. Removing a provider does not delete its local provider data or its
remote scope. It only removes Mallard's authority to scan or mutate it for
this project.

Profile health and sync gating are per provider. The project header summarizes
the selected profiles but does not reduce them to one "current agent."

## 11. Commands and frontend contracts

Add generic commands such as:

```text
list_agent_providers
get_project_activity
get_project_session_details
execute_project_session_action
get_project_session_sync_comparison
```

Replace, rather than alias:

```text
get_project_chat_history
get_project_chat_thread_details
open_codex_thread_in_app
open_codex_thread_in_terminal
get_project_thread_sync_comparison
```

Provider-specific maintenance commands may remain only when the operation is
inherently provider-specific, such as Codex desktop path repair. They must be
surfaced through provider capability metadata rather than used to decide
whether the generic project can sync.

Mirror every new Rust DTO exactly in `src/types.ts`; Tauri argument names stay
camelCase at the JavaScript boundary. Update the registration-freeze test in
the same change.

## 12. Source organization

Create the provider infrastructure under:

```text
crates/mallard-infrastructure/src/providers/
  mod.rs
  registry.rs
  traits.rs
  codex/
    mod.rs
    profile.rs
    discovery.rs
    history.rs
    codec.rs
    materialization.rs
    actions.rs
  claude/
    mod.rs
    profile.rs
    discovery.rs
    history.rs
    codec.rs
    materialization.rs
    actions.rs
```

Provider-independent code remains in the existing domain, bundle engine,
conversation reconciliation, persistence, and command orchestration modules.
Those modules may use `ProviderKey`, registry lookups, and adapter traits but
must not import a concrete Codex or Claude adapter.

The storage catalog continues to own its separate profiles, bindings, plans,
and apply behavior. It may reuse the canonical `ProviderKey` and provider
presentation descriptors, but project sync must not import storage-catalog
DTOs or persistence.

## 13. In-place schema-1 cutoff

This redesign reuses schema 1 everywhere. It does not introduce schema 2 or
any other new schema generation.

| Format | Only schema after cutoff |
| --- | ---: |
| Local sync configuration | 1 |
| Machine project/profile state | 1 |
| Setup draft and transaction | 1 |
| Conversation bindings | 1 |
| Conversation operation journal | 1 |
| Conversation Push plan | 1 |
| Restore plan | 1 |
| Bundle recipe | 1 |
| Bundle head and manifest | 1 |
| Activity cache | 1 |
| Storage marker/layout | 1 |

Any project-sync constants that currently use another number, including the
setup draft and transaction constants, are reset to 1 as part of the clean
cutoff. The number identifies the sole current contract; it is not used to
recognize pre-cutoff data.

Use the portable namespace:

```text
.mallard/v1/repositories/<bundle-id>/...
```

Do not add:

- migrations;
- schema aliases;
- fallback deserializers;
- `serde(default)` on newly required fields;
- old command aliases;
- a new versioned storage namespace;
- multiple repository list or inspection paths;
- automatic local-state salvage; or
- pre-cutoff shape detection or conversion.

The old local and remote data must be removed before the first test. Because
the numeric identifier and namespace are reused, stale data is unsupported
input rather than an upgrade case. New required-field, integrity, and
canonical-path validation still fails closed, but no compatibility behavior
is designed or tested.

## 14. Dead-code removal

The final change deletes, rather than deprecates:

- the closed `Provider` enums/unions;
- `PROJECT_PROVIDERS`;
- `configuredProjectProvider`;
- `singleProviderSelection`;
- single-agent setup state and CSS;
- the single-provider integration test;
- Codex-named common Activity DTOs and components;
- Codex-only common Tauri commands;
- the Codex-only thread-comparison builder;
- top-level single-profile conversation plan pins;
- generic Claude conversation restore and keep-local special cases;
- repository-wide-only continuity logic;
- pre-cutoff shape and storage readers; and
- comments and documentation describing one active project agent.

Provider names may remain in their concrete adapter modules, provider-specific
fixtures, provider-specific maintenance features, user-facing labels, and
tests that explicitly exercise those adapters.

## 15. Implementation phases

### Phase 1 — Domain and schema reset

1. Introduce `ProviderKey`, `SyncScopeKey`, generic resource classes, bundle
   scopes, scope bases, and provider-neutral Activity/session DTOs.
2. Normalize all redesigned project-sync schema constants to 1 and retain the
   existing `.mallard/v1` repository namespace.
3. Remove old serialized shapes and make validation strict.
4. Update `src/types.ts` with the new contracts.

Exit criterion: fresh-state domain and serialization tests pass, and no
pre-cutoff fixture, reader, or conversion path remains.

### Phase 2 — Provider registry and adapter extraction

1. Add registry and traits.
2. Move current Codex profile, discovery, history, codec, materialization, and
   action behavior behind the Codex adapter.
3. Move current Claude profile/discovery behavior behind the Claude adapter.
4. Add a test-only third provider.

Exit criterion: common code discovers providers only through registry
descriptors; adding the test provider requires no common-flow branch.

### Phase 3 — Claude typed sessions

1. Implement bounded Claude history and detail parsing.
2. Implement Claude lineage analysis and structural rewrite.
3. Consolidate proven sidecars under their conversation resource.
4. Implement resumable materialization and conflict preservation.
5. Delete the generic Claude session path.

Exit criterion: Claude supports the same normalized relation matrix as Codex
or fails closed with a typed reason.

### Phase 4 — Scope-aware storage and continuity

1. Publish and inspect the redesigned schema-1 scoped manifests.
2. Store and validate scope bases.
3. Implement disjoint-scope CAS graft/retry.
4. Enforce the final mutation firewall.
5. Update Push/Pull plan and receipt persistence.

Exit criterion: concurrent changes to different provider scopes merge without
one provider overwriting or blocking the other.

### Phase 5 — Setup and project settings

1. Add `list_agent_providers`.
2. Render registry-driven enablement and profile controls.
3. Enforce at least one selected provider.
4. Make profile health, collision checks, and editing provider-generic.
5. Remove the one-agent UI and helpers.

Exit criterion: Codex-only, Claude-only, and dual-provider setup all finalize
the exact reviewed binding.

### Phase 6 — Unified Activity

1. Replace `chat_history` contracts with provider-neutral Activity contracts.
2. Merge local bound-provider sessions with all remote provider slices.
3. Add remote-only and unsupported-provider presentation.
4. Generalize Git mapping, pagination, details, and actions.
5. Rename the frontend Activity component and delete its old fallback paths.

Exit criterion: a Codex-only project displays remote Claude sessions as
view-only without accessing a local Claude home.

### Phase 7 — Provider-scoped review and apply

1. Build comparison rows for every local/remote provider state.
2. Generate actions only for writable scopes.
3. Make multi-provider plans carry per-provider pins.
4. Isolate provider failures and base updates.
5. Update Push and Pull review selection and outcome rendering.

Exit criterion: each provider can Push/Pull independently, and unbound scopes
remain unchanged.

### Phase 8 — Cutover cleanup and documentation

1. Delete every old type, command, helper, CSS rule, test, and reader listed
   in Section 14.
2. Update command-registration freeze and frontend cutover guards.
3. Update `README.md`, `CURRENT_METADATA_LAYOUT.md`, and active architecture
   documentation.
4. Search for stale one-agent and Codex-only common-path terminology.

Exit criterion: there is one provider-registry path and one existing
schema-1 storage path.

## 16. Required tests

### 16.1 Setup and binding

- Codex-only setup succeeds.
- Claude-only setup succeeds.
- Codex-plus-Claude setup succeeds.
- Zero-provider setup is blocked in frontend and backend.
- Editing one provider does not alter another.
- A mismatched profile/provider key is rejected.
- Overlapping provider homes are rejected.
- Removing a binding does not delete local or remote provider data.

### 16.2 Visibility and permissions

- Codex-only local binding displays remote Claude sessions.
- Claude-only local binding displays remote Codex sessions.
- View-only sessions have no selection checkbox or mutation action.
- Expanding a supported remote-only session performs a bounded verified read
  and no filesystem write.
- An unknown provider displays generic metadata and no transcript action.

### 16.3 Scope preservation

- Codex-only Push preserves every Claude scope field, revision, digest,
  recipe entry, file reference, and tombstone.
- Claude-only Push preserves Codex identically.
- No tombstone can target an unbound or unknown provider scope.
- Viewing a provider does not advance its scope base.
- Pull advances only approved bound scopes.
- A provider profile failure does not advance its base.

### 16.4 Concurrency and continuity

- A remote Claude change does not block a Codex Push.
- A remote Codex change requires a Codex Pull before Codex Push.
- Disjoint Codex and Claude publishers converge through CAS retry.
- A same-provider CAS race fails or replans without lost updates.
- Missing/replaced global repository history blocks every scope.
- Graft retry rejects an unexpected writable-scope digest change.

### 16.5 Activity

- Mixed-provider rows sort and paginate deterministically.
- Identical provider-native IDs from different providers do not collide.
- Git mapping consumes normalized evidence without provider branches.
- Local-only, remote-only, synced, ahead, divergent, unavailable, view-only,
  and unsupported states render distinctly.
- Details and actions are dispatched by registry adapter.

### 16.6 Provider extensibility

- A test-only third provider participates in setup, local Activity discovery,
  remote viewing, comparison, and plan construction.
- Registering it requires no changes to common setup lists, Activity merging,
  scope continuity, or review rendering.
- An unregistered fourth-provider slice is preserved across a Push.

### 16.7 Codec and materialization

- Codex regression fixtures retain current safety behavior.
- Claude valid, malformed, oversized, ambiguous, active, continued, and
  divergent fixtures receive correct typed outcomes.
- Claude rewrite changes only reviewed structural identity/cwd fields.
- Rewritten Claude transcripts retain lineage and semantic digests.
- Claude conflict preservation creates a distinct resumable session before
  overwrite.
- Sidecars cannot attach through substring collisions or cross-session paths.

### 16.8 Cutover

- Fresh local state produces only the redesigned schema-1 documents.
- Fresh remote storage produces only redesigned manifests under the existing
  `.mallard/v1` repository namespace.
- No pre-cutoff fixture, reader, migration, or shape-detection branch remains.
- Removed Tauri commands are absent from registration.
- Removed frontend helpers and closed provider unions are absent.
- No production common-flow match branches on `codex` or `claude`.

## 17. Verification commands

Run focused tests while implementing, followed by the full handoff set:

```sh
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
npm run test:integration
```

For the UI stages, run the complete Tauri application and capture screenshots
covering:

- Codex-only setup against a mixed repository;
- Claude-only setup against a mixed repository;
- dual-provider setup;
- view-only remote provider rows; and
- independent provider selection in Push and Pull review.

## 18. Completion criteria

The redesign is complete only when all of the following are true:

1. A project can bind Codex, Claude, or both.
2. A mixed repository remains fully visible from a single-provider binding.
3. Only bound providers can produce local sync actions.
4. A provider-specific Push cannot modify another provider's slice.
5. Disjoint provider changes do not unnecessarily block each other.
6. Unknown future-provider slices remain visible and byte-preserved.
7. Codex and Claude both use the typed conversation pipeline.
8. A third test provider enters common flows through registration alone.
9. No pre-cutoff shapes, readers, commands, DTOs, one-agent helpers, or
   generic Claude conversation fallbacks remain.
10. The complete integration suite passes against fresh local and remote
    state.
