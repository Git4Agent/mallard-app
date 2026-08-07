# Plan: Project-scoped Agent Sync

> **Partially superseded** by
> [`PLAN_REMOVE_PROJECT_CONFIG_CAPTURE.md`](./PLAN_REMOVE_PROJECT_CONFIG_CAPTURE.md)
> (typed project-artifact sync removed).

**Status:** proposed clean-break redesign  
**Date:** 2026-07-17  
**Scope of this document:** architecture and implementation plan only

## 1. Outcome

Replace whole-profile sync (`~/.codex` or `~/.claude`) with a project-first
model. The user adds one real project folder, selects the exact agent resources
that belong with it, and syncs that portable bundle independently.

A project bundle can contain:

- Codex tasks and Claude sessions whose working directory belongs to the
  project;
- project instructions, agent configuration, memory, agents, commands, rules,
  and prompts;
- individually selected plugins and standalone skills;
- portable MCP, hook, and tool definitions, subject to review and secret
  checks; and
- a small semantic set of project-relevant settings.

On another machine the user fetches the bundle, maps it to a local checkout,
reviews a restore plan, and explicitly installs or applies executable
dependencies. Absolute paths, credentials, trust decisions, and installed
caches never become portable identity.

Example user-visible bundles:

```text
tauri-codex-sync
  Conversations   Codex 12 tasks, Claude 4 sessions
  Instructions    AGENTS.md, .codex/config.toml, CLAUDE.md
  Skills           frontend-skill, security-review
  Plugins          github, playwright
  Tools            2 MCP servers, 1 hook (review required)

memory
  Conversations   Claude 18 sessions
  Memory            Claude project memory
  Skills            document-research
```

The names above describe projects discovered from session metadata. They do
not mean that Codex literally stores a project at
`~/.codex/tauri-codex-sync`; Codex tasks are currently date-nested under its
session directories, while Claude uses encoded project buckets.

## 2. Explicit clean-break decisions

Migration and compatibility are out of scope, as requested.

1. Introduce local config schema **3** under a separate `app_data/v3/`
   namespace. Do not parse, overwrite, or convert schema 2 profiles, links,
   baselines, or selections.
2. Introduce the Mallard storage namespace under
   `.mallard/v1/repositories/`, identified by `.mallard/_storage.json`. Do not
   discover the former `v3/bundles/` namespace or old whole-profile heads.
3. Leave old local and cloud data untouched. The new app ignores it; it does
   not delete it.
4. Remove the `.codex | .claude` profile as the user-facing sync unit. A
   project bundle may contain resources from both providers.
5. Keep one CAS head per project bundle. Plugins and skills are individually
   selectable resources inside that atomic bundle, not separate distributed
   heads in the first release. This avoids cross-head consistency problems.
6. Make selection persistent. A checkbox is part of the bundle recipe, not a
   temporary push filter.
7. Make absolute paths machine-local bindings, never bundle identity.
8. Split fetch from apply. A new machine can download and inspect a bundle
   before choosing where anything will be written.
9. Reinstall plugins through the provider's native installer. Never copy
   plugin caches, marketplace clones, manager databases, or credentials.
10. Materialize selected standalone skills into project scope by default.
    Plugin-provided skills arrive only through plugin reinstall.
11. Do not sync ordinary application/source-code files. Git, another
    source-control system, or a manual copy remains responsible for project
    code. Explicitly allowlisted agent metadata inside the checkout may sync.
12. Do not transfer authentication, approval history, workspace trust, hook
    trust, connector authorization, known secret fields, or machine policy.
    Opaque user content receives scanning/warnings but cannot be certified
    secret-free.
13. The first release is project-first: standalone plugins and skills are
    individually selectable inside a project bundle. Plugin-only, skill-only,
    and reusable settings-pack bundles are a later extension, not hidden
    independent histories in v1.

## 3. Verified starting point

The current implementation is strongly whole-root oriented:

- `SyncConfig` schema 2 is `storages × local_profiles × links`.
- `LocalProfile`, `Roots`, `HeadFile.root`, cloud discovery, path validation,
  file eligibility, and the UI all assume one `.codex` or `.claude` root.
- Push can temporarily scope selected physical files, but it still publishes
  one full-profile manifest. Pull restores every eligible entry in that
  manifest.
- Status and editor requests are keyed by absolute physical paths.
- Plugin lock capture and repair already use the correct high-level pattern:
  portable intent plus explicit target-native repair.
- The current project-path mapping work is machine-local and safety-conscious,
  but it remaps a whole-profile restore after the fact. It does not create a
  project-scoped cloud identity.
- The worktree contains unfinished project-remapping changes. Implementation
  should first decide which reusable parts to land and which to replace; this
  plan deliberately does not edit those in-progress files.

Mechanics worth retaining:

- S3-compatible and local-folder `Store` implementations;
- immutable uploads, manifests, and commits behind a CAS head;
- head/manifest/object hash verification and CAS retry behavior;
- per-replica baselines and three-way reconciliation;
- deterministic merge drivers and conflict copies;
- no-follow path validation, case-collision checks, temp-file writes, backups,
  and SQLite snapshot handling;
- explicit plugin and hook consent; and
- readiness scanning after restore.

The product model, logical namespace, configuration schema, discovery, and UI
must be replaced rather than stretched.

## 4. Current product facts to design against

These are current documented inputs, not substitutes for executable fixtures:

- Codex supports repo guidance in `AGENTS.md`, repo skills in
  `.agents/skills`, project configuration in `.codex/config.toml`, and
  repo-scoped plugin catalogs in `.agents/plugins/marketplace.json`.
  Project config and hooks load only for trusted projects.
  See [Codex customization](https://developers.openai.com/codex/concepts/customization),
  [advanced configuration](https://developers.openai.com/codex/config-advanced),
  and [plugin building](https://developers.openai.com/codex/plugins/build).
- Claude supports project settings in `.claude/settings.json`, project agents
  in `.claude/agents`, project MCP servers in `.mcp.json`, project instructions
  in `CLAUDE.md` or `.claude/CLAUDE.md`, and project skills in
  `.claude/skills`. Plugin intent can be project scoped and installed with the
  native CLI.
  See [Claude settings](https://code.claude.com/docs/en/settings),
  [the `.claude` directory](https://code.claude.com/docs/en/claude-directory),
  [skills](https://code.claude.com/docs/en/slash-commands), and
  [plugins](https://code.claude.com/docs/en/discover-plugins).

Provider paths and CLI JSON shapes can change. Phase 0 must capture tested
fixtures from supported versions and keep adapters version-aware.

## 5. Terminology and ownership

| Term | Meaning | Synced? |
|---|---|---|
| **Project bundle** | One portable project plus its selected resources | Yes |
| **Bundle recipe** | Persistent list of included resource IDs and policies | Yes |
| **Resource** | One selected unit: task set, settings file, plugin, skill, etc. | Yes |
| **Logical path** | Stable bundle path independent of a machine's home/project path | Yes |
| **Binding** | This machine's bundle ID → local checkout and provider homes | No |
| **Restore plan** | Immutable preview of writes, merges, installs, reviews, and blockers | No |
| **Dependency intent** | Portable plugin/skill/tool requirement and provenance | Yes |
| **Readiness state** | Target-local result: installed, missing, blocked, needs review | No |

The project bundle is the user-facing unit and the atomic cloud history unit.
Resources remain individually selectable and independently statused within the
bundle.

## 6. Identity model

### 6.1 Bundle identity

Use a generated opaque `bundle_id`, not a path or Git URL. A display name is
mutable metadata.

```rust
struct BundleIdentity {
    bundle_id: String,
    display_name: String,
    kind: BundleKind, // Project for v1
    repository_fingerprint: Option<String>,
}
```

`repository_fingerprint` is a matching hint only:

- canonicalize the remote after removing credentials and machine-specific
  syntax;
- store a SHA-256 fingerprint by default, not the raw private remote;
- never auto-bind solely from a matching origin because worktrees and multiple
  checkouts can share it; and
- require the user to confirm the target folder.

### 6.2 Project-relative working directories

For every captured session/task, store a normalized `relative_cwd` from the
selected project root. A task started in `/A/repo/apps/web` becomes
`apps/web`. On machine B, a binding of the bundle to `/B/repo` resolves that
task to `/B/repo/apps/web`.

Reject sessions whose cwd cannot be proven to be the project root or a real
descendant. Resolve symlinks before membership checks and record enough local
evidence to detect a later path swap.

### 6.3 Stable resource identity

Every selectable resource gets a stable ID and class-specific identity:

```rust
struct ResourceDescriptor {
    resource_id: String,
    kind: ResourceKind,
    provider: Option<Provider>,
    scope: ResourceScope,
    display_name: String,
    provenance: Provenance,
    apply_policy: ApplyPolicy,
}
```

Examples:

- Codex task: provider + thread UUID;
- Claude session: provider + immutable session UUID; `relative_cwd` is a
  versioned placement attribute, not identity;
- project file group: normalized repo-relative path;
- plugin: provider + marketplace + plugin ID;
- standalone skill: provider + declared name + stable provenance key (or a
  generated persisted resource UUID when provenance is unknown); the content
  digest is version state, not identity;
- settings patch: provider + semantic key path.

Physical source paths never serve as resource IDs.

## 7. Bundle content model

### 7.1 Default classification

| Resource class | Capture | Restore | Default |
|---|---|---|---|
| Matching conversations/tasks | Filter from provider state | Materialize under selected provider home | Included |
| Project-local instructions | Copy allowlisted repo-relative content | Three-way apply into target project | Included |
| Project config | Decode to one canonical typed projection | Compose approved fields into the target file once | Included/review |
| Project memory | Provider adapter | Materialize under mapped project state | Included |
| Project-local skills/agents/commands/rules | Copy exact directory with safety checks | Three-way project-local apply | Included |
| Plugin intent | Capture selected IDs/source/ref, no payload | Native installer, explicit consent | Suggested |
| Plugin-provided skills | Record `provided_by` only | Reinstall owning plugin | Never copy |
| Selected global standalone skill | Snapshot plus provenance | Install project-locally, explicit consent if executable | Suggested |
| MCP definition | Semantic config, env names only | Preview, review, then project config | Suggested |
| Hook definition | Definition and hash | Preview and require local trust | Suggested |
| Global setting with project equivalent | Semantic key/value projection | Write project-scoped config | Suggested |
| User-only/global-only setting | Requirement/manual step | Never silently edit global config | Off |
| Credentials, tokens, auth, trust, approvals | Never capture | Recreate locally | Never |
| Caches, logs, temp, plugin clones/payloads | Never capture | Recreate locally | Never |
| Ordinary application/source files | Never capture | Obtain through source control | Never |

“Included” still means the resource passes the secret, path, size, and type
validators. The UI must explain why a discovered item is blocked.

### 7.2 Logical namespace

Use paths that describe meaning rather than source-machine layout:

```text
bundle.json
project/
  AGENTS.md
  .codex/config.toml
  .agents/skills/release/SKILL.md
  CLAUDE.md
  .claude/settings.json
  .claude/skills/review/SKILL.md
  .mcp.json
state/
  codex/sessions/2026/07/17/rollout-<thread>.jsonl
  codex/session-index.jsonl
  codex/sidebar.json
  claude/projects/<relative-cwd-key>/<session>.jsonl
  claude/projects/<relative-cwd-key>/memory/...
  claude/file-history/<session>/...
dependencies/
  codex/plugins.lock.json
  claude/plugins.lock.json
  skills/<resource-id>/...
requirements/
  environment.json
  binaries.json
```

`bundle.json` is manifest metadata, not a file written into the checkout.

Files already tracked by Git are still eligible when they are documented agent
metadata (`AGENTS.md`, `CLAUDE.md`, `.codex/**`, `.claude/**`, `.agents/**`,
or `.mcp.json`). The inventory marks them as Git-tracked; restore treats an
identical checkout copy as a no-op and never overwrites a divergent tracked
file without a merge/preview. Everything outside the explicit agent-resource
allowlist remains source-control territory.

### 7.3 Bundle recipe authority

Each linked storage's cloud manifest is authoritative for that storage copy.
Local config holds one editable desired recipe plus a base for every linked
storage, so fan-out does not pretend two independent heads are one transaction:

```rust
struct BundleRecipe {
    schema_version: u32,
    entries: BTreeMap<String, RecipeEntry>, // resource_id -> desired policy
    tombstones: BTreeMap<String, RecipeTombstone>,
}

struct RecipeDraft {
    recipe: BundleRecipe,
    bases: BTreeMap<String, RecipeBase>, // storage_id -> last reconciled base
}

struct RecipeBase {
    revision: u64,
    recipe_sha256: String,
}
```

Rules:

- a new local project receives its final `bundle_id` and an empty recipe
  immediately, before any storage link or push;
- adopting a remote bundle initializes the local draft from the fetched
  recipe before edits are allowed;
- every linked storage reconciles against its own recorded base; recipes that
  diverged independently across storages must merge or surface a conflict
  before that link publishes, and syncing one storage never silently declares
  another storage current;
- disjoint additions merge by stable resource ID;
- simultaneous edits to the same entry must be identical or surface a recipe
  conflict;
- removal creates a recipe tombstone carrying the last descriptor/digest;
  removal versus modification conflicts rather than resurrecting or deleting;
- a discovered resource disappearing locally marks it unavailable and does
  not imply deselection;
- every live recipe entry resolves to exactly one live descriptor; every file
  belongs to one live resource; live/tombstone IDs and logical paths must be
  unique under exact and case-folded comparison; and
- partial capture or retry filters never change selection. They carry forward
  untouched recipe entries and manifest files and cannot tombstone omitted
  resources.

Recipe edits and resource content publish atomically under the same bundle
head CAS.

## 8. Provider adapters

Define a provider trait so sync semantics do not grow more provider branches in
`lib.rs`:

```rust
trait ProjectProviderAdapter {
    fn discover(&self, project: &BoundProject) -> Result<Vec<ResourceCandidate>, Error>;
    fn capture(&self, recipe: &BundleRecipe) -> Result<CapturedResources, Error>;
    fn plan_restore(&self, bundle: &BundleSnapshot, binding: &Binding)
        -> Result<Vec<RestoreAction>, Error>;
    fn scan_readiness(&self, bundle: &BundleSnapshot, binding: &Binding)
        -> Result<Vec<ReadinessIssue>, Error>;
}
```

### 8.1 Codex adapter

Discovery and capture:

1. Scan `sessions/**` and `archived_sessions/**` with bounded reads.
2. Parse the rollout metadata used by supported Codex versions.
3. Include only tasks whose canonical cwd is the project or a descendant.
4. Preserve rollout file bytes and source mtime.
5. Filter `session_index.jsonl` to the selected thread IDs and store a
   bundle-local derived index.
6. Capture only the matching project entry from portable sidebar state.
7. Do not include global prompt history because attribution is unreliable.
8. Include Codex memory only when an adapter can prove project/thread
   ownership; otherwise surface it for explicit manual selection.

Project-local authored content:

- root and nested `AGENTS.md` files;
- documented inert project-local rules and related `.codex` content;
- `.codex/config.toml` decoded into typed settings, plugin, agent, hook, and
  MCP resources instead of copied as an undifferentiated raw file;
- `.codex/hooks.json` decoded into individually reviewed hook resources;
- `.agents/skills/**`; and
- `.agents/plugins/marketplace.json` as a catalog source, not proof that every
  listed plugin is required.

Plugin intent:

- inventory through machine-readable Codex CLI commands;
- let the user attach individual enabled plugin IDs;
- capture portable Git marketplace source/ref or a managed marketplace ID;
- classify local-path marketplaces as manual unless the path is inside the
  project and explicitly included; and
- on restore add the marketplace and plugin through Codex, verify final state,
  and tell the user to start a new task; and
- disclose the actual Codex install scope. If the supported Codex version has
  no project-scoped install, the action changes the selected `CODEX_HOME` for
  every project using it and requires explicit broader-scope consent (or a
  dedicated Codex home).

Restore:

- write task files under the selected target `CODEX_HOME` session layout;
- merge the filtered task index into the target index by thread ID;
- apply mapped sidebar state additively;
- keep transcript bytes unchanged;
- generate `codex resume <thread-id> -C <target-relative-cwd>` commands; and
- never sync or recreate project trust automatically.

### 8.2 Claude adapter

Discovery and capture:

1. Locate real project buckets below the selected Claude config home.
2. Decode/verify project identity from transcript metadata rather than trusting
   a directory name alone.
3. Include buckets for the root and selected descendant cwds.
4. Store each bucket under a `relative_cwd` logical key, not the encoded source
   absolute path.
5. Include per-project memory and session sidecars inside those buckets.
6. Associate `file-history/**` and `todos/**` only through verified session
   IDs.
7. Exclude global history when it cannot be attributed safely.

Project-local authored content:

- `CLAUDE.md` and `.claude/CLAUDE.md`;
- `.claude/settings.json` decoded into typed settings, permission, plugin,
  hook, and requirement resources rather than copied raw;
- `.claude/agents/**`, `.claude/skills/**`, `.claude/commands/**`, and
  `.claude/rules/**` when present; and
- `.mcp.json` decoded into one resource per MCP definition after
  secret/reference validation.

Never capture `.claude/settings.local.json`, `CLAUDE.local.md`, raw
`~/.claude.json`, OAuth state, local approvals, or workspace trust. If a useful
user/local MCP definition exists only in `~/.claude.json`, offer a separate
“promote to project `.mcp.json`” review flow; never copy the whole file.

Plugin intent:

- read project-scoped `enabledPlugins` and `extraKnownMarketplaces` plus the
  native inventory;
- let the user select plugins individually;
- restore with `claude plugin marketplace add ... --scope project` and
  `claude plugin install ... --scope project` where supported;
- verify installed/enabled state and preserve unrelated target plugins; and
- never copy `plugins/cache`, `plugins/repos`, `plugins/marketplaces`, or
  manager JSON containing target-local paths.

Restore path gate:

- preferred: materialize the logical project bucket directly under the
  encoded target cwd;
- verify with a real disposable session that Claude resumes the same session
  ID without transcript rewriting; and
- if a supported Claude version requires the original bucket name, fall back
  to the existing machine-local alias technique. The alias remains local and
  cannot change cloud logical keys.

## 9. Skills and reusable resources

Classify every skill by provenance before capture:

```rust
enum SkillProvenance {
    ProjectLocal { relative_path: String },
    StandaloneSnapshot { stable_key: String, content_digest: String },
    Git { repository: String, rev: String, subdir: String },
    Plugin { provider: Provider, plugin_id: String },
    Unknown { persisted_resource_id: String },
}
```

Rules:

- Project-local skills remain project-local files.
- A selected global standalone skill is copied into the bundle and restored to
  the provider's project-local skill directory by default:
  `.agents/skills/<name>` for Codex or `.claude/skills/<name>` for Claude.
- Preserve known Git provenance and immutable revision as verification/update
  metadata. The captured snapshot remains available for offline restore.
- Keep the resource ID stable when skill bytes or Git revisions change. Store
  content digest/revision in the resource version and baseline.
- A plugin-owned or symlinked skill is not dereferenced and copied blindly.
  Attach its owning plugin instead.
- An unknown symlink target, path outside approved roots, nested `.git`, or
  special file blocks capture.
- Skills containing scripts/assets are executable content. Show their hash and
  file inventory in the restore plan. Strip setuid/setgid bits and require an
  explicit install action.
- Name collisions produce a three-way conflict; they never silently replace an
  existing target skill.
- Removing a skill from the recipe removes the requirement. Deleting an
  already materialized target skill is a separate confirmed action and only
  succeeds when the target still matches the installed digest.

Global installation may be offered as an explicit alternate target later. It
must never be the default for a project bundle because it changes other
projects on the machine.

## 10. Settings, MCP, hooks, and requirements

Do not solve project settings with arbitrary TOML/JSON search-and-replace.
Each provider adapter classifies semantic fields:

```rust
enum SettingPortability {
    ProjectPortable,
    ProjectPortableNeedsReview,
    LocalRequirement,
    Secret,
    TrustOrApproval,
    Unsupported,
}
```

Behavior:

- parse each project-native config once and assign every field to exactly one
  typed owner: inert setting, plugin intent, MCP definition, hook, permission,
  secret/local overlay, or unsupported data;
- never also capture the original config as a generic raw-file resource;
- offer selected user-level values only when the provider documents a safe
  project-scoped equivalent;
- write those values into a project-scoped projection, never the global file;
- represent paths to binaries, environment variables, runtimes, and external
  tools as target requirements;
- store environment variable names, never their values;
- reject literal credential-bearing MCP fields instead of silently uploading a
  broken redaction;
- require review for permissions, hooks, commands, MCP servers, and any config
  that can execute code; and
- keep trust/approval state local even when the reviewed definition is
  identical on two machines.

On restore, one provider-specific composer owns each physical config file. It
re-reads the target immediately before apply, merges the approved typed
resources with the target-local overlay, validates the full result, and writes
the file once. Plugins, hooks, MCP definitions, permissions, and settings must
not race through separate raw-file writers. Enabling config is the final step
after required installers and environment checks succeed.

The restore plan should distinguish **file will be written**, **dependency will
be installed**, **definition needs review**, **secret/env is missing**, and
**manual action only**.

## 11. Local schema 3

Keep storage definitions, but replace profiles and root links:

```rust
struct SyncConfigV3 {
    schema: u32, // 3
    storages: Vec<StorageConfig>,
    projects: Vec<LocalProjectRegistration>,
    links: Vec<ProjectStorageLink>,
}

struct LocalProjectRegistration {
    local_id: String, // stable replica ID on this machine
    bundle_id: String, // generated/adopted before the registration is saved
    display_name: String,
    recipe_draft: RecipeDraft,
}

struct ProjectStorageLink {
    project: String, // LocalProjectRegistration.local_id
    storage: String,
}
```

Store machine bindings separately in app data:

```rust
struct MachineBindings {
    schema: u32,
    bindings: Vec<ProjectBinding>,
}

struct ProjectBinding {
    local_project_id: String, // replica ID / local registration
    bundle_id: String,
    project_root: String,
    codex_home: Option<String>,
    claude_home: Option<String>,
    revision: u64,
    materializations: Vec<MaterializationRecord>,
    updated_at: u64,
}

struct MaterializationRecord {
    provider: String,
    provider_home: String,
    state: String, // active | detached
    source_revision: u64,
    applied_receipt_ids: Vec<String>,
}
```

Requirements:

- atomic bounded JSON writes;
- absolute, existing project directory validation before apply;
- no symlink or overlap with app-owned staging/backups/storage;
- editable remapping without changing `bundle_id` or cloud keys;
- the same `bundle_id` is reused in every linked storage; fan-out creates
  independent storage heads/histories for one logical bundle identity, with a
  separate recipe/capture base per link;
- importing an existing remote bundle adopts its ID before creating the local
  registration; a locally created project generates its ID immediately;
- one bundle may have multiple local registrations/replicas, each with a
  distinct `local_id`, project root, binding, and baseline;
- one link per `(local_id, storage_id)`, one active bundle per canonical
  project root, and no two active replicas of the same bundle in the same
  provider home unless duplicate session-ID semantics are explicitly solved;
- mapping files never enter bundle discovery or a cloud manifest; and
- secrets for S3 storage remain outside bundle data. Moving storage
  credentials to the platform credential store is a separate security task.

All new local state lives below `app_data/v3/`, so saving schema 3 cannot
overwrite schema 2 config, baselines, backups, or machine records. Baselines
become:

```text
app_data/v3/baselines/<storage-id>/<bundle-id>/<local-id>.json
```

Cloud caches are keyed by `(storage-id, bundle-id)`. Local apply baselines are
keyed by `(storage-id, bundle-id, local-id)`.

The capture baseline is not proof that target actions ran. Keep a second
apply-receipt ledger under `app_data/v3/apply-receipts/<local-id>/`:

```rust
struct ApplyReceipt {
    receipt_id: String,
    plan_id: String,
    storage_id: String,
    bundle_id: String,
    revision: u64,
    resource_id: String,
    action_id: String,
    source_digest: String,
    target_evidence: TargetEvidence,
    verification: String,
}
```

Capture baselines answer “what did this replica reconcile with the cloud?”;
apply receipts answer “what was actually written, installed, or reviewed on
this binding?”. Partial approval writes receipts only for completed actions.

### 11.1 Shared provider-home composition

Different bundles normally share `~/.codex` and `~/.claude`. Per-bundle
baselines alone cannot safely write shared indexes, sidebar state, plugin
config, or provider inventories.

Use a machine-local compositor keyed by canonical provider home and physical
target file:

- serialize capture/apply operations with a provider-home operation lock;
- store each active bundle's derived contribution separately in app data;
- compose Codex `session_index.jsonl` by thread ID from the current target plus
  all active bundle contributions;
- apply sidebar entries additively by stable project/thread identity;
- route every shared config file through the single semantic composer from
  Section 10;
- treat native plugin installs as provider-home-wide inventory changes, never
  as byte materialization owned by one bundle;
- re-read and revalidate the live target immediately before writing; and
- removing/detaching one bundle may remove only its unchanged derived
  contribution, never entries now claimed or modified elsewhere.

### 11.2 Nested project ownership

Canonical project registrations may be nested. Without an ownership rule,
`/repo` and `/repo/apps/web` could capture the same session into two bundles.

- The most-specific active project root owns a session/task cwd by default.
- Parent discovery excludes descendants claimed by a nested registration.
- A stable provider session/thread ID may be selected by only one bundle on a
  machine; a conflicting claim blocks push and identifies both projects.
- Moving a claim between bundles is an explicit detach/attach operation with a
  recipe tombstone in the old bundle, not a side effect of rescanning.
- Two different bundles cannot be actively bound to the same canonical
  checkout.

## 12. Cloud schema and storage layout

Reuse the immutable-history design under a clean namespace:

```text
.mallard/
  _storage.json
  v1/repositories/<bundle-id>/
    _head.json
    _tag.json
    _manifests/<revision>-<commit-id>.json
    _commits/<revision>-<commit-id>.json
    _uploads/<upload-id>/
      _upload.json
      files/<logical-path>
```

The new head identifies a bundle, not an agent root:

```rust
struct BundleHead {
    schema_version: u32,
    bundle_id: String,
    kind: String, // "project"
    revision: u64,
    commit_id: String,
    manifest_key: String,
    commit_key: String,
    manifest_sha256: String,
    updated_at: u64,
}
```

The manifest includes both files and typed resources:

```rust
struct BundleManifest {
    schema_version: u32,
    revision: u64,
    commit_id: String,
    updated_at: u64,
    bundle: BundleIdentity,
    recipe: BundleRecipe,
    captured_with: CapturedWith,
    resources: BTreeMap<String, ResourceDescriptor>,
    files: BTreeMap<String, BundleFileEntry>,
    tombstones: BTreeMap<String, Tombstone>,
}
```

`BundleFileEntry` carries `resource_id`, content hash, size, source mtime,
object key, and safe mode metadata. Validate logical paths independently of
local destination paths.

`CapturedWith` and each provider resource record include the Agent Sync
version, provider CLI version, and provider codec version. Restore rejects an
unsupported newer codec or requires a newer app; it never guesses how to
materialize an unknown provider layout.

Remote discovery must be paginated. Add a cursor-based
`list_remote_bundles` command and tests with at least 1,000 bundles. Do not
build a mutable global catalog that creates a second CAS authority.

## 13. Capture and push flow

1. Resolve the local binding and re-check canonical project/provider roots.
2. Ask both provider adapters for discovered candidates.
3. Resolve the persistent recipe to exact resource IDs.
4. Capture only those resources into a staging snapshot.
5. Run secret, path, type, size, symlink, casefold, and provenance validation.
6. Generate filtered indexes/locks and canonical semantic projections.
7. Three-way reconcile the recipe draft against its base recipe and the current
   cloud recipe; fail on unresolved selection/removal conflicts.
8. Compare the logical snapshot with the per-replica capture baseline and
   cloud head, then reconcile by resource policy.
9. Upload immutable objects, bundle manifest, and commit.
10. CAS the bundle head; on a lost race, fetch, rebase, and retry.
11. Save the baseline only for captured/applied entries whose result is known.
12. Return a typed per-resource result.

The normal push always publishes the complete persistent recipe. The frontend
sends no temporary selection. A separate failed-resource retry may name
resource IDs, but it carries forward all untouched recipe entries/files and
cannot imply deselection or deletion. The frontend never sends arbitrary
absolute upload paths.

## 14. Fetch, map, plan, and apply flow

### 14.1 New machine

1. Connect a storage and browse remote bundles.
2. Fetch and verify the selected bundle into app-owned staging/cache.
3. Show bundle contents before any target write.
4. Choose an existing local checkout. Matching Git fingerprints may be
   suggested but never auto-accepted.
5. Choose provider homes or accept the current defaults.
6. Build a restore plan containing every write, merge, install, review,
   missing requirement, and blocker.
7. Let the user approve safe files, plugin installs, standalone skills, hooks,
   and MCP definitions independently.
8. Pin the plan to storage ID, bundle ID, revision, commit ID, manifest hash,
   binding revision, provider codec versions, and target precondition digests;
   revalidate all of them immediately before mutation.
9. Back up affected target files, apply file actions atomically, then invoke
   approved native installers.
10. Verify provider inventories and run readiness scans.
11. Persist only the machine binding, capture baseline, per-action apply
    receipts, review hashes, and verification results locally. Unapproved or
    failed actions receive no receipt and are never reported as applied.

### 14.2 Already bound project

Pull may go directly from fetch to restore preview. Pure conversation additions
and byte-identical files can be pre-approved by policy; executable or
privileged actions always remain explicit.

### 14.3 Remapping later

Changing `/A/repo` to `/B/repo` updates the local binding only. It must not:

- create a new cloud bundle;
- rewrite cloud logical paths;
- rewrite transcript bytes automatically;
- duplicate Claude project buckets on the next push; or
- lose per-resource baseline history.

After remap, rebuild the materialization plan for the new target and mark the
old materialization as detached. Never delete the old checkout or provider
state automatically.

## 15. Conflict and deletion policy

Whole-profile “deletions never propagate” is too coarse for project bundles.
Use class-specific rules:

| Class | Concurrent changes | Deletion/removal |
|---|---|---|
| Frozen conversation files | File-set union; same-session divergence is quarantined and blocks materialization until a branch is chosen or a proven event merge succeeds | Retain unless user explicitly forgets history |
| Append-only indexes | Keyed deterministic merge bounded to selected IDs | Derived entries disappear only with explicit task removal |
| Memory notes | Union distinct notes; same-note divergence conflicts | Confirmed tombstone |
| Project config/instructions | Three-way semantic merge where proven; otherwise conflict copy | Confirmed tombstone |
| Standalone skill files | Three-way by skill/resource | Remove requirement first; target delete is separate and digest-guarded |
| Plugin intent | Three-way keyed recipe merge; source mismatch blocks | Recipe tombstone removes the requirement but never auto-uninstalls target plugin |
| Hook/MCP definitions | Hash-addressed review plus conflict block | Remove definition only after explicit review |

Tombstones are resource-scoped, CAS-protected, and carry the last known digest.
A target deletion applies only when the local bytes still match that digest.
This prevents both resurrection and destructive removal of later local edits.

Never place a generic conflict sibling beside live provider session files: two
files carrying the same internal session/thread UUID can corrupt discovery or
appear as duplicate live history. Preserve divergent bytes only in bundle
conflict storage/app staging until the user resolves them; derived indexes
reference no quarantined branch.

## 16. Security and safety boundaries

Preserve and extend the existing safety posture:

- no auth files, tokens, keychain material, `.env` values, connector sessions,
  cookies, or private keys;
- no trust state, approvals, permission “allow forever” state, or hook trust;
- no plugin caches, package-manager databases, repositories, marketplaces, or
  installed payloads;
- no special files, sockets, devices, hard links, unsafe symlinks, nested VCS
  metadata, path traversal, reserved device names, or casefold collisions;
- maximum bundle, resource, file, settings, and manifest sizes;
- hash and size verification before apply;
- destination containment checks immediately before every write;
- staging plus atomic rename and pre-write backups;
- no automatic plugin, skill-script, hook, or MCP activation;
- native installer arguments passed as structured argv, never shell strings;
- strip credentials from Git URLs before storing provenance;
- report that conversation transcripts can themselves contain source absolute
  paths or user-provided secrets; users may exclude conversation resources;
- preserve target-only plugins/settings and never treat bundle intent as
  authority over unrelated target state; and
- refuse provider-state mutation while the relevant app/CLI is active when a
  safe atomic apply cannot be guaranteed.

The hard guarantee applies to known credential-bearing files and typed fields:
they are structurally denied. Opaque transcripts, memories, instructions, and
skill scripts can contain user-written secrets that no scanner can identify
perfectly. Run best-effort credential-pattern scanning, show warnings and a
content inventory, and require explicit acknowledgment for opaque resources;
do not claim that arbitrary content is provably secret-free.

## 17. Backend structure

Do not expand the existing 10k-line `lib.rs`. Introduce focused modules:

```text
src-tauri/src/
  domain/
    bundle.rs
    config.rs
    resource.rs
    binding.rs
    restore_plan.rs
  providers/
    mod.rs
    codex.rs
    claude.rs
  bundle_sync/
    manifest.rs
    capture.rs
    reconcile.rs
    apply.rs
  dependencies/
    plugins.rs
    skills.rs
    settings.rs
  commands/
    projects.rs
    bundles.rs
    restore.rs
```

Move/refactor existing transport and safety helpers only as needed. The first
goal is a generic logical bundle engine, not a simultaneous style rewrite of
all storage code.

Before reusing `Store`, introduce validated `BundleId`, `StoreKey`, and
`LogicalPath` newtypes. Every store method must accept validated keys rather
than raw strings. Local-folder reads, writes, locks, and deletes must prove
canonical containment and reject symlink traversal. Existing profile-specific
validators cannot simply be bypassed for `.mallard/v1/repositories/`.

Suggested Tauri surface:

```text
discover_project(path) -> ProjectDiscovery
list_local_projects() -> LocalProjectSummary[]
get_project(project_id) -> ProjectDetail
save_project(project) -> void

list_remote_bundles(storage, cursor) -> BundlePage
fetch_bundle(storage, bundle_id) -> BundleSnapshotSummary
link_project_bundle(project, storage, bundle_id) -> void

get_bundle_inventory(project) -> ResourceInventory
save_bundle_recipe(project, recipe) -> void
get_bundle_status(project, storage) -> ResourceStatusReport
push_bundle(project, storage) -> OperationResult
retry_bundle_resources(operation_id, resource_ids) -> OperationResult

get_project_binding(local_project_id) -> ProjectBinding?
save_project_binding(binding) -> void
plan_bundle_restore(storage, bundle_id, binding) -> RestorePlan
apply_bundle_restore(plan_id, approved_action_ids) -> RestoreResult

plan_dependencies(bundle_id, binding) -> DependencyPlan
apply_dependency_actions(plan_id, action_ids) -> DependencyResult
get_bundle_readiness(bundle_id, binding) -> BundleReadiness

read_resource(project, resource_id, logical_path) -> FileDocument
write_resource(project, resource_id, logical_path, expected_sha256, content)
  -> FileDocument
```

Use typed tagged enums for actions and results. Replace action/category string
switches. Progress events should include `operation_id`, `phase`,
`resource_id`, `done`, and `total`; logs should include the same operation ID.

## 18. Frontend plan

### 18.1 Navigation

- Replace **Profiles** with **Projects**.
- Add **Remote bundles** so a new machine can browse before it has a local
  binding.
- Keep storage settings, but change the matrix to `projects × storages`.
- Show the bound checkout under each project and expose **Change folder**.

### 18.2 Project workspace

Replace the raw home-directory tree with grouped resources:

```text
Conversations
  Codex tasks (12)                         Included
  Claude sessions (4)                     Included

Project setup
  AGENTS.md                               Included
  .codex/config.toml                      Included · review
  .claude/settings.json                   Included · review

Skills
  frontend-skill                          Project · included
  security-review                         Personal → project · selected

Plugins
  github@openai-curated                   Selected · install on restore
  formatter@team-tools                    Not selected

Tools & hooks
  context7 MCP                            Selected · env missing
  pre-commit hook                         Selected · review required
```

Each row needs a stable resource ID, source scope, provider, inclusion policy,
status, target/install behavior, and a real action menu. Selection persists.

### 18.3 Add project flow

1. Pick a real project folder.
2. Show detected Git fingerprint and provider state.
3. Inventory project-local resources plus selectable global plugins/skills.
4. Present recommended defaults and Never exclusions.
5. Choose storage(s) and create or link a cloud bundle.
6. Review the initial capture plan before the first push.

### 18.4 Restore flow

Use a dedicated restore preview, not the current generic Finish setup list:

- target checkout and provider homes;
- files to add/merge/conflict;
- Codex/Claude tasks and relative cwd mappings;
- plugin installs one by one;
- standalone skill installs one by one;
- the actual apply scope for every install (project-local, Claude project
  scope, or provider-home-wide Codex scope);
- hooks/MCP definitions requiring review;
- missing binaries/environment variables/authorization; and
- manual continuation commands.

The user can approve action groups independently and retry failed items without
re-fetching the bundle.

### 18.5 Component split

`App.tsx` and `SyncPanel.tsx` are already too large. Add components such as:

```text
ProjectSidebar.tsx
ProjectWorkspace.tsx
ProjectEditor.tsx
ResourceInventory.tsx
ResourceRow.tsx
RemoteBundleBrowser.tsx
ProjectBindingEditor.tsx
RestorePlan.tsx
DependencyPlan.tsx
ReadinessPanel.tsx
```

`FilePreview` should address a resource/logical path through the backend, not
accept an arbitrary absolute path.

## 19. Implementation phases

### Phase -1 — establish a green starting point

- Land, replace, or quarantine the unfinished project-remapping work already
  in the checkout.
- Require the existing frontend build, Rust check, and Rust tests to pass
  before attributing failures to this redesign.
- Record the green commit/worktree state used by the Phase 0 fixtures.

**Gate:** no redesign work begins on a backend with pre-existing compile or
test failures.

### Phase 0 — provider evidence and fixtures

- Create disposable Codex and Claude homes plus two distinct project paths.
- Record supported CLI versions and machine-readable plugin inventory output.
- Prove Codex task membership, filtered index merge, remapped resume, and
  sidebar behavior.
- Prove Claude target-bucket materialization; decide direct bucket versus local
  alias from evidence.
- Inventory documented project config/skill/plugin paths on supported versions.
- Commit sanitized fixtures and adapter tests.

**Gate:** no schema implementation until both providers can round-trip one
session/task between different absolute project roots.

### Phase 1 — clean domain and local schemas

- Add schema 3 types, validation, atomic persistence, and machine bindings.
- Add bundle/resource identities and typed recipes.
- Add stable logical path grammar and resource policy tables.
- Add validated storage-key newtypes, provider codec metadata, capture
  baselines, apply receipts, and provider-home locks/contribution records.
- Define config field ownership and structural Never/secret classifiers before
  any provider file can enter a capture snapshot.
- Keep old schema 2 data unread and untouched.

**Gate:** unit tests prove paths are not identity and bindings cannot enter a
manifest.

### Phase 2 — generic cloud bundle engine

- Parameterize/reuse `Store` only through the validated key boundary, then add
  CAS head publication, immutable history, hash verification, baseline,
  backup, and conflict infrastructure.
- Add `.mallard/v1/repositories/<id>` discovery and cursor pagination.
- Add typed bundle manifests and per-resource statuses.
- Add dual-backend tests before provider materialization.

**Gate:** two synthetic resources converge across three replicas and two
storages without cross-bundle or cross-baseline leakage.

### Phase 3 — project discovery and capture

- Implement Codex and Claude adapters from the Phase 0 fixtures.
- Add project-local authored file inventory through the allowlist, config
  field ownership, secret gates, codec versions, and best-effort opaque-content
  warnings.
- Add filtered session/task indexes and project memory capture.
- Add resource selection and capture preview commands.

**Gate:** a home containing projects A and B publishes A with zero B paths,
IDs, plugin intent, or session bytes.

### Phase 4 — binding, restore planning, and file apply

- Add remote bundle browser and fetch-only staging.
- Add binding validation/remapping.
- Generate immutable restore plans.
- Materialize conversations and proven inert project files with backups,
  quarantined session conflicts, provider-home composition, and per-class
  tombstones.
- Record per-action apply receipts; do not generically apply config, hooks,
  MCP, plugins, or executable skill content in this phase.
- Add continuation commands and readiness state.

**Gate:** machine B restores the same task/session IDs to a different project
path and a later B push does not create path-key churn.

### Phase 5 — plugins, skills, settings, MCP, and hooks

- Generalize existing plugin lock/repair code to selected bundle resources.
- Add standalone skill provenance, snapshot, project-local install, and
  digest-guarded collision handling.
- Complete semantic settings composition and environment/binary requirements
  using the field ownership defined earlier.
- Add explicit hook/MCP review and local trust records; compose/enable config
  only after approved dependencies verify.

**Gate:** every executable action is visible and independently consented to;
no plugin cache, credential, trust record, or unselected dependency uploads.

### Phase 6 — project-first frontend

- Replace profile navigation, schema-2 settings, root-filtered cloud picker,
  and temporary file selection.
- Add project/resource inventory, remote browser, binding editor, restore
  preview, dependency results, and readiness.
- Add a small frontend test runner for reducers/type guards/plan rendering;
  keep `npm run build` as the compile gate.

**Gate:** all primary actions can be completed without exposing raw agent-home
paths or requiring the user to understand cloud profile prefixes.

### Phase 7 — hardening and documentation

- Run large-bundle, 1,000-bundle pagination, interrupted apply, CAS race,
  malformed manifest, symlink swap, casefold, and size-limit tests.
- Run a real R2/S3 smoke test in addition to the stub and local backend.
- Rewrite `README.md`, `DESIGN2.md`, and `AGENT_SYNC_FILE_SETS.md` around
  project resources; mark old profile plans superseded.
- Document exactly what never syncs and how to remove ignored old data
  manually, without deleting it automatically.

## 20. Test plan

### Unit tests

- bundle/resource ID validation and canonical serialization;
- recipe three-way merge, removal tombstones, unavailable candidates, and
  partial-retry carry-forward;
- logical path traversal, Windows reserved names, Unicode/casefold collisions;
- validated store-key containment for every local-store operation;
- binding containment, symlink, overlap, and remap invariants;
- Git remote credential stripping and fingerprint stability;
- Codex cwd membership, thread ID extraction, and filtered index/sidebar data;
- Claude bucket verification, relative cwd keys, direct target projection, and
  alias fallback;
- project A/B isolation in one shared agent home;
- settings portability classification and secret rejection;
- plugin intent canonicalization, source conflict, and selected-only capture;
- skill provenance, symlink handling, script review, and digest collision;
- class-specific merge/tombstone behavior; and
- restore-plan expiry/revalidation after a target changes;
- capture-baseline versus partial apply-receipt behavior; and
- provider codec version acceptance/rejection.

### Dual-backend integration tests

Run every portable scenario against stub S3 and local-folder storage:

1. A pushes project A while A and B share the same Codex/Claude homes; the
   manifest contains no B resource.
2. A bundle contains both Codex tasks and Claude sessions for one checkout.
3. Machine B fetches without a binding and no target file changes.
4. B binds `/B/repo`, restores, resumes the same IDs, edits, pushes, and A
   pulls the update.
5. B remaps to `/B/repo2`; the bundle ID and logical keys remain unchanged.
6. An unselected plugin and skill never enter descriptor, payload, or restore
   plan.
7. Selected Codex and Claude plugins produce native install actions, are
   idempotent, and do not remove target extras.
8. A selected standalone skill installs project-locally; a plugin skill is
   provided only by plugin repair.
9. Missing env/binary/auth requirements block only dependent actions.
10. Hook/MCP definitions require local review even when another machine
    reviewed the same hash.
11. Concurrent config/skill edits preserve both versions or block according to
    policy.
12. Confirmed resource tombstones do not remove locally modified target data.
13. Malformed, oversized, known credential-bearing fields/files,
    unsafe-symlink, and hash-mismatched content fails closed; opaque secret
    patterns warn and require acknowledgment.
14. Project fan-out to two storages keeps independent baselines.
15. Two projects linked to one storage never share status/cache/baseline data.
16. Remote discovery paginates more than 1,000 bundles.
17. Projects A and B restore into the same provider homes; their keyed index,
    sidebar, and config contributions compose without overwriting each other.
18. Nested `/repo` and `/repo/apps/web` registrations assign each session to
    the most-specific owner and block duplicate claims.
19. Two replicas of one bundle cannot materialize duplicate session IDs into
    the same provider home.
20. A stale machine edits selection while another removes a resource; recipe
    reconciliation conflicts or tombstones deterministically, never silently
    resurrects it.

### Manual acceptance tests

- Real Codex CLI and ChatGPT desktop task continuation after path remap.
- Real Claude Code session continuation from a differently rooted checkout.
- Codex plugin install from managed and Git marketplaces.
- Claude project-scoped plugin marketplace add/install and reload.
- Standalone skill restore with scripts/assets and an intentional name
  collision.
- MCP/hook review, missing environment variable, and local authorization flow.
- macOS filesystem permission denial and recovery.
- Real R2/S3 TLS, pagination, ETag, and conditional write behavior.

Required repository checks when implementation begins:

```sh
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib sync_tests
```

## 21. Acceptance criteria

- The user can add `/A/tauri-codex-sync` as one project without syncing any
  unrelated task, session, skill, plugin, or setting from either agent home.
- One bundle can carry both Codex and Claude state for that project.
- Plugin and skill inclusion is individual, visible, and persistent.
- A new machine can inspect a remote bundle before choosing a local path.
- Binding the bundle to `/B/tauri-codex-sync` restores the same task/session
  identities without making `/A/...` the cloud identity.
- Changing the binding later does not fork or duplicate the bundle.
- Plugins reinstall through native provider commands only after explicit
  consent; caches and manager state never sync.
- Standalone skills restore project-locally with provenance, hash, and conflict
  checks; plugin skills are not copied separately.
- Project settings use provider-supported project scopes. User-only settings
  become requirements/manual actions rather than silent global edits.
- Known credential files/fields, auth, trust, approvals, and plugin payloads
  are structurally excluded. Opaque user content is scanned best-effort and
  requires warning acknowledgment, not falsely certified secret-free.
- Ordinary application/source files never enter a manifest; only explicitly
  allowlisted agent metadata inside the checkout can do so.
- Persistent selection reconciles as a versioned recipe; partial pushes and
  missing local candidates cannot silently change it.
- Every target mutation appears in a revalidated restore plan and receives a
  backup when it can overwrite existing data.
- Two bundles, two storages, and two local replicas have fully isolated cloud
  caches and capture baselines, while shared provider-home projections compose
  under one lock and partial applies have separate receipts.
- Stub S3, local backend, real S3-compatible smoke, Rust tests, and frontend
  build all pass.

## 22. Non-goals for the first project-scoped release

- Migrating schema 2 config, baselines, cloud profiles, or old path mappings.
- Deleting ignored old cloud/local profile data.
- Syncing or cloning ordinary project source code; allowlisted agent metadata
  inside the checkout is the intentional exception.
- Automatically matching and binding a checkout without confirmation.
- Rewriting transcript bodies to replace absolute paths.
- Deliberately transferring known credentials/secret fields, auth sessions,
  trust, or approvals. Perfect secret detection inside opaque conversations or
  scripts is not claimed.
- Automatically installing, enabling, disabling, updating, downgrading, or
  uninstalling executable dependencies without an explicit action.
- A general package manager or dependency solver.
- Exact plugin version pinning where the provider cannot enforce it.
- A mutable global cloud catalog in addition to bundle heads.
- Plugin-only, skill-only, or reusable settings-pack bundle kinds; v1 keeps
  those as individually selected project resources.
- Cross-project global skill installation by default.
- Scheduled/background sync before interactive capture and restore are proven.
