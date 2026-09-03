# Design: Extensible Mallard providers and portable contracts

**Status:** proposed clean-cut foundation

**Date:** 2026-09-03

**Scope:** machine-local provider configuration, compiled provider registries,
Project Sync portable resources, the storage Library, storage setup
assistance, and provider maintenance actions

## 1. Decision

Mallard will use stable, versioned keys in persisted data and compiled
registries at runtime.

This design opens four independent extension areas:

- storage providers;
- agent providers;
- Library catalog kinds, formats, and targets; and
- Project Sync resource kinds, apply policies, provenance kinds, and
  tombstone targets.

Mallard will make one final clean cutover while the product has no customers.
All affected machine-local document schemas and both portable schemas will
remain `1`. The user can delete disposable `~/.mallard` and cloud test data
for this cutover.

After the cutover, a built-in addition such as Dropbox storage, Cursor or
Qoder agent support, a Gemini skill, or a Cursor plug-in intent will require:

- a compiled adapter or codec;
- a registry entry;
- tests; and
- a Mallard application release.

It will not require:

- a new top-level schema version;
- a migration;
- deletion of `~/.mallard`;
- deletion or conversion of cloud data;
- a rewrite of existing records; or
- a new provider-name branch in common code.

This design does not load third-party code at runtime. Support without a new
Mallard release requires a separate out-of-process plug-in protocol. That
protocol is out of scope.

### 1.1 Compatibility contract

In this document, **extensible** means schema-stable and migration-free. It
does not mean that Mallard can execute code that was not included in its
release.

| Change | Mallard release | Top-level schema change | Migration or wipe |
|---|---:|---:|---:|
| Add a storage provider | Yes | No | No |
| Add an agent provider | Yes | No | No |
| Add a Library resource kind | Yes | No | No |
| Add a Library format or format version | Yes | No | No |
| Add a Library target | Yes | No | No |
| Add a Project Sync resource kind | Yes | No | No |
| Add a Project Sync apply policy | Yes | No | No |
| Add a Project Sync provenance kind | Yes | No | No |
| Add a Project Sync tombstone target | Yes | No | No |
| Add a new version of an existing portable contract | Yes | No | No rewrite of old records |
| Change one provider's saved config contract | Yes | No | Separate provider-local decision; no unrelated wipe |
| Add a storage setup assistant for one provider | Yes | No | No |
| Add a provider maintenance action | Yes | No | No |
| Change the internal object-store trait | Yes | No | No; every compiled adapter changes in the same release |
| Change a stable outer envelope | Yes | Separate design | Separate design |
| Load a provider implementation at runtime | Out of scope | Separate architecture | Separate architecture |

Each new provider or contract starts at its own version `1`. Its version does
not change the version of an existing provider, contract, or top-level
document.

### 1.2 No-write and no-rewrite guarantees

Registration alone is read-only:

> Given valid schema-1 state written after this cutover, installing and
> starting a build with more registry entries performs no persistence write.

A persistence write is any create, replace, or delete below the machine-local
state root, with three exclusions that exist today and hold no provider
state:

- activity-log appends under `logs/`, including the `system.app_started`
  record that every launch emits;
- first-run creation of the host identity document, which happens only when
  no state exists yet; and
- the startup recovery command, which must be proven to write nothing when no
  interrupted conversation sync operation exists.

The no-wipe proof in section 14.1 excludes exactly these paths and nothing
else.

The same rule applies to cloud state:

> Registering a provider or contract does not write, convert, or republish an
> existing repository, bundle, head, object, or receipt.

A user action can write its normal state:

| Event | Permitted effect |
|---|---|
| Start a build with more registry entries | No write |
| Display a new setup or capability descriptor | No write |
| Configure a new provider | Add its record and update only the owning document revision |
| Use a new provider for Push, Pull, or Library apply | Write only the reviewed operational state for that action |
| Run a setup assistant or a maintenance action | Write only that draft, its journal entry, or the action's declared local effect |
| Remove an adapter from a later build | Preserve its records and show them as unavailable |

An intentional write must not normalize, replace, or remove unrelated
records.

## 2. Problem and design boundaries

Mallard is only partly extensible now.

| Area | Current foundation | Remaining closure |
|---|---|---|
| Agent identity | `ProviderKey` is a validated string | None |
| Agent execution | `ProviderRegistry` dispatches `SessionAdapter` implementations | Adapter boundary is incomplete |
| Portable agent scopes | Unknown provider keys are preserved | None |
| Storage configuration | Provider-specific fields and enums | Closed |
| Configured object store | Provider-specific variants | Closed |
| Library catalog | Closed provider, kind, format, and target types | Closed |
| Project Sync resources | `ResourceKind` enum | Closed |
| Project Sync apply behavior | `ApplyPolicy` enum | Closed |
| Project Sync provenance | `Provenance` tagged enum | Closed |
| Project Sync deletion | `TombstoneTarget` tagged enum | Closed |
| Provider capture result | Common capture enums and conversion matches | Closed |
| Storage provisioning | R2-only browser flow in the CLI behind the public `launch_r2_agent_setup` command | Closed |
| Storage setup journal | R2-only schema-1 journal with R2 account, bucket, and jurisdiction fields | Closed |
| Provider maintenance | Codex-only public commands for conversation-path audit and repair and for the post-pull app restart | Closed |
| Provider dispatch in common modules | Provider-name matches in the service, the session watcher, and the domain crate | Closed |

The design removes closed discriminants from portable and machine-local
extension axes. It does not make every internal enum extensible.

Closed internal types remain valid for:

- fixed protocol outcomes;
- common safety states;
- typed data inside one adapter; and
- action primitives that the common engine explicitly supports.

The portable discriminants that name a resource kind, apply policy,
provenance kind, or tombstone target are open even though the behaviors they
select are fixed engine primitives. The reason is cross-version cloud data.
An older build must parse a newer bundle, preserve every record, and treat
the unknown records as inert. A closed serde enum fails structural parsing on
the first unknown value and blocks the whole scope instead. Opening the name
does not open the behavior: a registered apply-policy or tombstone codec can
only select among primitives the engine already implements.

Adding a new I/O action primitive is a security and protocol change. It is not
a routine provider registration.

## 3. Common extension rules

### 3.1 Stable key types

Use separate newtypes for separate authority domains:

| Domain | Stable key |
|---|---|
| Agent provider | `ProviderKey` |
| Storage provider | `StorageProviderKey` |
| Library resource kind | `CatalogResourceKindKey` |
| Library format | `CatalogFormatKey` |
| Library target | `CatalogTargetKey` |
| Project Sync resource kind | `ResourceKindKey` |
| Project Sync apply policy | `ApplyPolicyKey` |
| Project Sync provenance | `ProvenanceKindKey` |
| Project Sync tombstone target | `TombstoneTargetKindKey` |

All keys use the current `ProviderKey` grammar:

- lower-case ASCII;
- first character in `a-z`;
- remaining characters in `[a-z0-9._-]`;
- final character in `[a-z0-9]`;
- maximum length 64;
- no `..`; and
- exact, case-sensitive comparison.

A key is identity only. It is not a path, URL, command, executable name, or
authorization grant.

### 3.2 Version ownership

Each provider configuration and each contract that owns a persisted data
shape contains a positive local version.

Examples:

```text
storage provider:       (dropbox, 1)
agent provider:         (cursor, 1)
Library format:         (agentskills.io.skill, 1)
Project resource kind:  (conversation, 1)
apply policy:           (explicit_review, 1)
```

The owner of one coordinate can add version `2` without changing any other
coordinate. Old versions remain operational only when their compiled codec is
still registered. They remain structurally readable and preserved otherwise.

A Library resource kind and target are semantic keys. `format_version`
versions the portable resource shape. The catalog profile config version
versions provider-local setup. A new incompatible kind or target meaning must
use a new key.

### 3.3 Bounded objects

Provider-specific data uses a stable JSON-object envelope. It never uses an
unrestricted `serde_json::Value` at an execution boundary.

`BoundedConfigObject` and `BoundedPortableObject` enforce these limits. They
extend the existing descriptor-metadata limits (256 entries, 256-byte keys,
4,096-byte values, no control characters) to nested objects:

| Limit | Value |
|---|---:|
| Encoded size | 64 KiB |
| Nesting depth, counting the root object as 1 | 8 |
| Total object fields and array elements | 1,024 |
| Object-key length | 256 bytes |
| String length | 4,096 bytes |
| Array length | 256 |
| Numbers | finite JSON numbers only |
| Duplicate keys | rejected |
| Control characters in keys or strings | rejected |

A bounded object is validated from its raw JSON text with a custom
deserializer, not from a `serde_json::Value`. The default `serde_json` map
silently keeps the last duplicate key, and its recursion limit is a crash
guard rather than a design bound. After validation the object is held,
persisted, and digested in one canonical form: sorted keys and no
insignificant whitespace. A document that Mallard wrote therefore round-trips
byte-identically through a typed load and save.

Every limit has a focused test at the boundary and one past it.

`BoundedPortableObject` has these additional rules:

- no machine-local absolute paths, checked structurally for leading path
  separators, drive letters, and home-directory prefixes;
- no command definitions; and
- no field can grant executable behavior.

Credential material is excluded by policy, not by structure. Codecs run the
existing credential-shape detector over string values and reject matches.
That check is a warning aid and does not prove that content is safe.

An adapter or codec must deserialize the bounded object into a typed structure
for the exact registered key and version. Typed decoding must reject unknown
fields.

### 3.4 Three validation stages

Loading and execution are separate:

1. **Structural validation** verifies schema, bounds, key grammar, and
   document invariants. It does not need a provider binary, CLI, token, or
   network call.
2. **Registry validation** resolves the exact key and local version to a
   compiled adapter or codec.
3. **Operation validation** verifies current credentials, filesystem roots,
   revisions, digests, expiry, and remote state for the requested action.

Unknown coordinates can pass structural validation. They cannot pass registry
validation and cannot execute.

### 3.5 Registries are code, not state

Mallard must not persist:

- registry snapshots;
- discovered capability lists;
- default records for every compiled provider; or
- normalized copies of provider descriptors.

Each process builds immutable registries from compiled registrations. A new
release can therefore expose a new capability without writing user state.

Common modules must dispatch by registry lookup. They must not use provider
name matches such as `local`, `r2`, `dropbox`, `codex`, `claude`, or `cursor`.

## 4. Stable persisted contracts

The following shapes are normative. Rust and TypeScript DTOs can use
validated newtypes instead of the simplified scalar types shown here.

### 4.1 Storage configuration

```rust
struct StorageConfig {
    id: StorageId,
    name: String,
    provider: StorageProviderKey,
    provider_config: StorageProviderConfig,
}

struct StorageProviderConfig {
    version: u32,
    settings: BoundedConfigObject,
    secrets: SecretMap,
}
```

`SecretMap` is bounded and machine-local. It must be:

- redacted from `Debug` and errors;
- excluded from frontend DTOs;
- excluded from portable manifests, plans, receipts, and logs;
- stored only through bounded atomic persistence; and
- returned only to the selected adapter during an authorized backend action.

Provider settings and secrets are separate so the frontend can edit safe
settings without reading a saved credential.

`SecretMap` holds at most 16 entries. Keys use the stable key grammar; values
are control-free and at most 64 KiB. The current draft-detail command returns
a drafted secret so the user can review it before commit. That behavior is
removed: every draft and configuration response reports secret presence only,
and a secret typed into the setup form leaves frontend state as soon as the
draft update completes.

The current `supports_conditional_writes` flag leaves the common storage
record. It is a probe result about one S3-compatible endpoint, so the
S3-compatible adapter owns it inside its version-1 settings if it still needs
it.

OAuth authorization, callbacks, code exchange, and token refresh are outside
the provider contract. A user supplies a pre-issued storage credential or
selects an agent home that is already authenticated. A storage setup
assistant (section 5.1) may obtain that credential on the user's behalf, but
it delivers the result into an ordinary draft; the adapter contract never
sees an authorization flow.

### 4.2 Agent and catalog profile configuration

```rust
struct ProviderProfile {
    profile_id: LocalProviderProfileId,
    provider: ProviderKey,
    display_name: String,
    path: String,
    canonical_path: String,
    provider_config: AgentProviderConfig,
    revision: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
}

struct AgentProviderConfig {
    version: u32,
    settings: BoundedConfigObject,
}
```

Agent authentication stays in the configured provider home. Mallard does not
run OAuth for an agent provider and does not copy provider credentials into
the profile.

Provider-home paths remain machine-local and must not enter portable data.

The Library owns a separate local profile record. It uses the same stable
envelope rule without importing Project Sync DTOs:

```rust
struct LocalCatalogProfile {
    profile_id: LocalCatalogProfileId,
    provider: ProviderKey,
    display_name: String,
    configured_home: String,
    provider_config: CatalogProviderConfig,
    revision: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
}

struct CatalogProviderConfig {
    version: u32,
    settings: BoundedConfigObject,
}
```

Codex and Claude start with config version `1` and empty settings. A future
catalog provider can add typed settings inside its own version-1 envelope
without changing the catalog profile document shape.

### 4.3 Library catalog coordinates and resources

The Library has four independent axes:

```rust
struct CatalogScope {
    scope_id: CatalogScopeId,
    provider: ProviderKey,
    profile_id: LocalCatalogProfileId,
    resource_kind: CatalogResourceKindKey,
    format: CatalogFormatKey,
    format_version: u32,
    revision: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
}
```

The cloud resource envelope is:

```rust
struct CatalogResourceEnvelope {
    resource_id: CatalogResourceId,
    provider: ProviderKey,
    resource_kind: CatalogResourceKindKey,
    format: CatalogFormatKey,
    format_version: u32,
    identity: BoundedPortableObject,
    descriptor: BoundedPortableObject,
    content_sha256: Sha256Digest,
    files: Vec<PortableFile>,
}
```

The machine-local apply target is separate:

```rust
struct CatalogTargetConfig {
    provider: ProviderKey,
    target: CatalogTargetKey,
    identity: BoundedConfigObject,
}
```

Target identity can contain a provider-home selector or another machine-local
coordinate. It must not enter cloud data, frontend DTOs, logs, or receipts.
The backend exposes only a safe target summary.

Initial keys are:

| Axis | Version-1 keys |
|---|---|
| Resource kind | `agent_skill`, `plugin_install_intent`, `desktop_pet` |
| Format | `agentskills.io.skill`, `mallard.plugin-install-intent`, `openai.codex-pet` |
| Target | `provider-home.skills`, `shared-user.skills`, `user.plugins`, `desktop.pets` |

`mallard.plugin-install-intent` is the current shared portable shape of a
Codex or Claude plug-in install intent: a marketplace source, a canonical
plug-in ID, and a resolution policy. It is Mallard-defined because neither
provider publishes a portable intent format. `cursor.plugin-install-intent`
in section 7.4 is registered only when Cursor support lands.

Not every combination is valid. The exact registered capability defines the
allowed provider, kind, format, version, target, capture mode, and apply mode.

The two current Codex pet formats become one format key with two registered
format versions:

| Current kind | Coordinate after the cutover |
|---|---|
| `codex_pet_v1` | kind `desktop_pet`, format `openai.codex-pet`, version `1` (1536 by 1872 atlas, 8 by 9 cells) |
| `codex_pet_v2` | kind `desktop_pet`, format `openai.codex-pet`, version `2` (1536 by 2288 atlas, 8 by 11 cells) |

Each version keeps its own codec and its own Library repository. There is no
conversion between them, as today.

### 4.4 Project Sync resource contracts

The common portable contract must not use these closed extension types:

```rust
enum ResourceKind { /* ... */ }
enum ApplyPolicy { /* ... */ }
enum Provenance { /* ... */ }
enum TombstoneTarget { /* ... */ }
```

Replace them with versioned coordinates and bounded envelopes:

```rust
struct ResourceKindCoordinate {
    key: ResourceKindKey,
    version: u32,
}

struct ApplyPolicyCoordinate {
    key: ApplyPolicyKey,
    version: u32,
}

struct ProvenanceEnvelope {
    kind: ProvenanceKindKey,
    version: u32,
    fields: BoundedPortableObject,
}

struct ResourceDescriptor {
    resource_id: ResourceId,
    kind: ResourceKindCoordinate,
    provider: Option<ProviderKey>,
    provider_resource_type: Option<ResourceTypeKey>,
    display_name: String,
    provenance: ProvenanceEnvelope,
    apply_policy: ApplyPolicyCoordinate,
    codec_version: u32,
    attributes: BoundedPortableObject,
}

struct RecipeEntry {
    resource_id: ResourceId,
    apply_policy: ApplyPolicyCoordinate,
    required: bool,
}

struct TombstoneTargetEnvelope {
    kind: TombstoneTargetKindKey,
    version: u32,
    identity: BoundedPortableObject,
}

struct Tombstone {
    target: TombstoneTargetEnvelope,
    last_sha256: Option<Sha256Digest>,
    deleted_at: Timestamp,
}
```

Initial resource-kind keys at version `1` are:

- `conversation`;
- `provider_state`;
- `project_content_file`;
- `project_content_directory`; and
- `requirement`.

Initial apply-policy keys at version `1` are:

- `safe_file`;
- `merge`;
- `explicit_review`;
- `manual_only`; and
- `never`.

Initial provenance keys at version `1` are:

- `project_local`; and
- `unknown`.

Initial tombstone-target keys at version `1` are:

- `resource`;
- `file`;
- `project_content_file`;
- `project_content_directory`; and
- `conversation_thread`.

`attributes` replaces an open string-to-string metadata bag. The resource-kind
adapter defines its typed structure. It cannot define a command, target path,
or executable behavior.

`codec_version` identifies the provider content codec. It is independent of
the resource-kind, policy, provenance, and tombstone versions.

A recipe policy must exactly equal the resource descriptor policy. A recipe
cannot upgrade or replace the policy selected by the registered resource
contract.

`PreservedProvenance`, where used for typed conversation lineage, remains a
separate internal type. It is not the open provenance axis in
`ResourceDescriptor`.

Two structural rules follow from open keys:

- The current kind-to-provider consistency check moves into the resource-kind
  adapter, because structural validation cannot know whether an unregistered
  kind requires a provider. Structural validation still verifies that a
  provider, when present, is a valid key, and that a provider subtype never
  appears without a provider.
- A tombstone whose target kind or version is unregistered still needs a map
  key so that it is preserved once and merged deterministically. The common
  layer keys it by kind, version, and the canonical JSON of its identity
  object. A registered codec supplies the canonical identity instead. Map keys
  are derived at load time and never persisted, so registering a codec later
  changes only the in-memory key.

## 5. Runtime registries and ownership

Mallard uses separate registries because the extension areas grant different
authority.

| Registry | Lookup key | Runtime result |
|---|---|---|
| `StorageProviderRegistry` | storage provider and config version | storage adapter |
| `AgentProviderRegistry` | agent provider and config version | agent adapter |
| `CatalogRegistry` | provider, kind, format, and format version | capability, provider adapter, and codec |
| `PortableResourceRegistry` | resource/policy/provenance/tombstone key and version | narrowly scoped adapter or codec |

Duplicate registrations are startup errors. The current
`ProviderRegistry::from_adapters` collects into a map and silently keeps the
last adapter for a repeated key; it must reject the duplicate instead, and
every registry shares that check and its test.

Registry iteration order is ascending key order. Display order is separate:
each descriptor carries a `display_rank`, the backend sorts descriptors by
rank and then by key, and the frontend keeps that order. A product decision
to list one provider first is expressed through the rank, never through a
provider-name branch or registration order.

### 5.1 Storage provider adapter

A storage adapter owns:

- its safe descriptor and configuration fields;
- typed configuration decoding;
- settings and secret validation;
- redacted summaries;
- continuity coordinates;
- allowed local roots;
- connection probing; and
- construction of an `Arc<dyn ObjectStore>`.

The common storage layer owns:

- storage IDs and names;
- draft and commit transactions;
- marker validation;
- object-key construction;
- immutable publication;
- compare-and-swap heads;
- SHA-256 verification;
- bounded, revision-bearing reads and listings;
- conditional deletion against a listed revision; and
- the exact, nonrecursive managed clear.

#### Object-store contract correction

The current trait lists keys without revisions and deletes unconditionally,
so a managed clear cannot bind a deletion to the revision it listed. The
common contract changes for every adapter:

- list entries carry the object key and its opaque revision;
- delete requires the expected revision and returns `Deleted`, `Missing`, or
  `Conflict`;
- there is no delete-any variant; and
- managed clear stops on the first conflict.

Local verifies its SHA-256 revision under its store lock; S3-compatible
stores use `If-Match`. The correction lands at the start of Layer 2, before
the conformance suite is written, and a live R2 run must prove conditional
delete before Layer 2 merges.

`ConfiguredObjectStore` becomes an opaque wrapper:

```rust
struct ConfiguredObjectStore {
    provider: StorageProviderKey,
    inner: Arc<dyn ObjectStore>,
}
```

It must not have Local, R2, Dropbox, or other provider variants.

Every storage adapter must pass one object-store conformance suite. The suite
must cover:

- bounded reads;
- bounded, deterministic, revision-bearing listing;
- immutable create and idempotent retry;
- compare-and-swap update;
- revision conflict behavior;
- digest verification;
- conditional delete, including the missing and conflict outcomes;
- marker isolation; and
- credential redaction.

#### Generic setup

The public setup flow is provider-neutral:

1. list compiled storage descriptors;
2. create or edit a draft;
3. validate the draft structurally;
4. ask the selected adapter to test it;
5. show a redacted result;
6. commit with a revision check; or
7. cancel without changing saved state.

Public commands use storage IDs, provider keys, config versions, field IDs,
and secret presence operations. They must not expose provider-specific setup
commands.

The frontend renders safe fields from the provider descriptor. Secrets use
set, replace, clear, and unchanged operations. A saved value is never returned
to the frontend.

#### Setup assistant

The R2 provisioning flow stays. Today the public `launch_r2_agent_setup`
command opens an agent CLI with an instruction, the agent runs the packaged
Mallard CLI, and the CLI drives a browser authorization with Cloudflare's
tooling, provisions a bucket and an exact-bucket credential, and prepares an
ordinary storage draft. That flow is R2-specific by nature and remains inside
the R2 adapter and the CLI's R2 module.

Its public entry point becomes generic. A storage descriptor may declare one
`setup_assistant` with a label, a description, and the instruction template
the agent receives. One command, `launch_storage_setup_assistant`, takes a
storage provider key and an agent provider key, resolves both through their
registries, and refuses a provider that declares no assistant. The assistant
must end in the generic draft flow above; it has no private commit path.

#### Setup journal

The R2 setup journal becomes a provider-neutral storage setup journal in the
same schema-1 style: a `setup_id`, the storage provider key and config
version, a `stage` (`probe_running` or `cleanup_required`), a sanitized
`error_code`, timestamps, and a bounded, non-secret `facts` object that the
adapter types. The R2 adapter's facts are its account ID, bucket, and
jurisdiction. The directory name and entry bound are provider-neutral. The
journal never holds a credential, a token, HTTP material, or a local path.

#### Continuity identity

The adapter returns typed continuity coordinates. The common layer hashes the
provider key and canonical coordinates. It never hashes credentials or a
display name.

| Provider | Continuity coordinates | Excluded |
|---|---|---|
| Local folder | canonical directory | display name |
| S3-compatible/R2 | account or endpoint, region, bucket | access key and secret |
| Dropbox | namespace ID | access token |

Only the continuity digest can enter portable continuity state. Machine-local
coordinates remain local.

Dropbox authentication is deliberately simple: the user supplies a
pre-issued token for an account that is already a collaborator or managed
team member. Mallard does not implement OAuth. The namespace ID is a setting;
the token is a secret.

### 5.2 Agent provider adapter

Keep the current `ProviderKey`, provider-scoped repository model, unknown
provider preservation, and `SessionAdapter` foundation.

Complete the adapter boundary so an agent adapter owns:

- profile validation;
- provider-home discovery;
- local inventory and filters;
- stable resource identity;
- capture;
- provider content codecs;
- conversation lineage;
- safe presentation;
- typed materialization;
- supported session actions; and
- launch behavior.

The common Project Sync layer owns:

- provider and profile IDs;
- portable logical paths;
- path canonicalization and overlap checks;
- immutable publication;
- revisions and digests;
- plan creation and review;
- backups;
- expected-local-digest checks;
- deletion safety; and
- transaction orchestration.

Cursor support is one new `AgentProviderRegistry` entry with
`AgentProviderConfig` version `1`. It must not change the Codex or Claude
profile records. Any later agent provider follows the same rule.

#### Provider maintenance actions

Session actions already flow through adapter-declared descriptors and one
generic execute command. The same pattern extends to provider-level actions.
An adapter declares maintenance actions with an action ID in the stable key
grammar, a label, a scope (`profile`, `project`, or `after_pull`), and a
confirmation requirement. Two generic commands list and execute them. The
current Codex-only public commands become declared Codex actions:

| Current command | Declared action |
|---|---|
| `audit_codex_conversation_paths` | `codex.conversation_paths.audit`, project scope, read-only |
| `repair_codex_conversation_paths` | `codex.conversation_paths.repair`, project scope, confirmed |
| `restart_codex_app_after_pull` | `codex.app.restart`, after-pull scope |

The frontend renders declared actions and never checks a provider key to
decide whether to show one.

#### Branches to remove

Common modules contain these provider-name branches today. Layer 4 moves each
behind an adapter method or a declared capability, and the section 14.4
source guard then rejects reintroduction:

| Location | Branch | Replacement |
|---|---|---|
| `src-tauri/src/project_sync/session_watch_service.rs` | watched provider-home subdirectories chosen by key | adapter-declared watch roots |
| `src-tauri/src/project_sync/service.rs`, conversation push and Pull apply | `is_codex` gates for alias-owner lookup and standalone reconstruction | adapter capability flags beside `supports_tail_cursor` |
| `crates/mallard-domain/src/lib.rs`, materialized-binding layout check | expected portable tail codec chosen by provider name | codec key recorded on the profile or binding by the adapter |
| `crates/mallard-domain/src/lib.rs`, conversation thread validation | path-map stamp and wire-tail validation chosen by provider name | adapter-declared wire rules, resolved by the caller and passed into domain validation |

`codex_binary.rs` and the Codex and Claude adapter modules keep provider
names; they are adapter-local.

### 5.3 Library catalog registry

The catalog has two execution roles:

| Role | Responsibility |
|---|---|
| Provider adapter | resolve local targets, inventory local resources, capture, and apply |
| Format codec | validate portable identity, descriptor, files, digest, and presentation |

A registered capability joins these roles and declares:

```rust
struct CatalogCapabilityDescriptor {
    provider: ProviderKey,
    resource_kind: CatalogResourceKindKey,
    format: CatalogFormatKey,
    format_version: u32,
    label: String,
    presentation: CatalogPresentation,
    capture: CaptureSupport,
    apply: ApplySupport,
    targets: Vec<CatalogTargetKey>,
}
```

`ApplySupport` has fixed protocol outcomes:

- `Unsupported`;
- `Manual`; and
- `Automatic`.

These values describe common workflow maturity. They are not provider
identities.

The frontend lists capabilities supplied by the backend. It must not contain
fixed provider, resource-kind, format, target, icon, label, group, or apply
mode unions.

The current `CatalogProviderAdapter` trait, `adapter_for`, and
`capability_matrix` in `storage_catalog/adapters.rs` are the seed of this
registry. Layer 4 opens their keys and adds the format, format-version, and
target axes; it does not introduce a second adapter layer.

### 5.4 Portable resource registry

`PortableResourceRegistry` contains four independent maps:

- resource-kind adapters;
- apply-policy adapters;
- provenance codecs; and
- tombstone-target codecs.

A resource-kind adapter owns:

- whether a provider is required;
- resource ID validation;
- allowed logical namespaces;
- cardinality and combination rules;
- typed `attributes` decoding;
- safe presentation;
- typed restore planning; and
- coordination with the selected agent adapter.

An apply-policy adapter can control only:

- selection eligibility;
- review requirements;
- acknowledgment requirements; and
- policy-specific approval gates.

It cannot create an I/O target, filesystem action, command, or network
request. The resource-kind adapter proposes typed operations. The common
engine validates and executes only known safe primitives.

A provenance codec validates and presents provenance fields. A tombstone
codec validates identity and canonicalizes it. The common layer derives its
map key from the target kind, target version, and canonical identity. Neither
codec can perform I/O.

The common engine continues to own:

- review-before-apply;
- plan revision, digest, and expiry pins;
- canonical-path checks;
- no-follow traversal;
- backups;
- expected-local-digest checks;
- exact empty-directory deletion; and
- atomic apply and receipts.

`CaptureResourceKind` and `CaptureApplyPolicy` must be removed from the common
capture boundary. An agent adapter emits registered coordinates directly.
There must be no later conversion match from provider output to the portable
wire type.

The backend supplies safe labels, grouping, and availability for these
contracts. The frontend renders those descriptors and one generic unavailable
state. It does not dispatch behavior by a closed union.

## 6. Persistence and IPC

### 6.1 Machine-local documents

Machine-local documents keep top-level schema `1`. Structural loading must
not depend on a compiled adapter being present.

Top-level documents and stable envelopes keep `deny_unknown_fields`. A
document with an unexpected top-level field is invalid, as today.
Preservation applies to two things: a record whose provider or contract key
is not registered, and the contents of a bounded object. Both are retained as
data and are not silently removed or rewritten.

The persistence layer must distinguish these cases:

- an untouched document stays byte-identical;
- an intentional edit updates only the owning document and revision; and
- unknown records and bounded subtrees in an edited document remain
  byte-identical, which the canonical form in section 3.3 makes possible.

No startup path writes defaults, registry state, normalized provider data, or
schema conversions.

### 6.2 Focused commands

The frontend must not read and replace a complete persistence document.
Commands operate on one record or field group and use revision checks.

Required command classes are:

- list safe provider and capability descriptors;
- list redacted configured-record summaries;
- create, update, test, commit, or cancel one draft;
- set, replace, clear, or retain one secret;
- rename or remove one configured record;
- launch one declared storage setup assistant;
- list and execute one declared provider maintenance action; and
- plan, approve, and apply one reviewed sync or catalog operation.

The backend owns merging and unknown-field preservation.

Frontend DTOs never contain:

- saved secrets;
- full provider config objects;
- provider-home paths;
- local checkout paths; or
- catalog target identity.

## 7. Open cloud schema

### 7.1 Transport and payload are independent

Local folder, R2, and Dropbox transport the same Mallard bytes. Adding a
transport does not change Project Sync or Library payload schemas.

The infrastructure layer retains the closed `.mallard/` object-key grammar
and separate Project Sync and Library constructors. A provider cannot choose
arbitrary object keys.

### 7.2 Project Sync schema 1

Project Sync keeps:

- bundle manifest schema `1`;
- immutable bundle publication;
- compare-and-swap mutable heads;
- provider-scoped revisions and continuity;
- SHA-256 verification; and
- plan-based restore.

The open axes are the stable provider scopes and the versioned contracts from
section 4.4. A new resource contract can appear in a new schema-1 bundle. It
does not change an older bundle.

### 7.3 Library schema 1

The existing `.mallard/v1/library/...` object-key grammar stays unchanged.
Repository IDs and the tag shape do change, because the derivation gains the
format and format version. Every Library repository therefore has a new ID
after the cutover; the disposable pre-cutover cloud data is deleted, not
mapped.

The repository tag is open by coordinate:

```rust
struct LibraryResourceTag {
    schema_version: u32, // always 1 for this design
    repository_id: CatalogResourceRepositoryId,
    provider: ProviderKey,
    resource_kind: CatalogResourceKindKey,
    format: CatalogFormatKey,
    format_version: u32,
    created_at: Timestamp,
}
```

The repository ID is a deterministic hash of:

```text
provider + resource_kind + format + format_version
```

It must not include:

- a profile ID;
- a provider-home path;
- a catalog target;
- a storage provider;
- a storage name; or
- a credential.

Therefore, a new coordinate creates a separate Library repository. It does
not alter old heads, revisions, immutable objects, or resource envelopes.

### 7.4 Cursor skill example

A Cursor skill that conforms to the Agent Skills standard uses this existing
schema-1 coordinate:

```text
provider:        cursor
resource_kind:   agent_skill
format:          agentskills.io.skill
format_version:  1
target:          provider-home.skills    # machine-local only
```

This addition needs a Cursor provider adapter and a registered capability. It
does not need a different cloud-storage schema.

If a skill uses Cursor-only frontmatter that changes its portable semantics,
register a separate Cursor format key. Do not silently discard or reinterpret
those fields.

A Cursor plug-in install intent is a different coordinate:

```text
provider:        cursor
resource_kind:   plugin_install_intent
format:          cursor.plugin-install-intent
format_version:  1
target:          user.plugins            # machine-local only
```

The distinct format prevents Mallard from treating a plug-in intent as an
Agent Skill. Both repositories still use Library schema `1`.

## 8. Unknown and unavailable data

Unknown data is preserved but inert. Mallard resolves exact keys and versions
only.

| Unknown or unavailable item | Read behavior | Mutation behavior |
|---|---|---|
| Storage provider | Show a redacted unavailable record | Cannot test, open, sync, or edit provider fields |
| Storage config version | Show the provider and unsupported version | Cannot test, open, or sync |
| Agent provider | Preserve the profile and remote scope; show only common bounded fields | Cannot capture, restore, launch, or run actions |
| Agent config version | Show the profile and unsupported version | Cannot capture, restore, or launch |
| Library coordinate | Parse and show a generic unavailable resource | Cannot select, capture, apply, or delete |
| Project resource kind | Parse and preserve the descriptor | Blocks publication or apply for its containing Project Sync scope only |
| Apply policy | Parse and preserve the descriptor | Resource is not selectable or applicable |
| Provenance kind | Parse and preserve the descriptor | Resource is not selectable or applicable |
| Tombstone target | Parse and preserve the tombstone | Cannot acknowledge or delete the target |

An unknown Project Sync contract does not block other provider scopes or
Library repositories. An unknown Library coordinate does not block other
Library coordinates.

When a later release registers the exact key and version, the preserved record
becomes operational without conversion.

Mallard must not reinterpret unknown data as:

- `never`;
- `manual_only`;
- `unknown` provenance;
- a generic file target; or
- another known provider or format.

## 9. Security invariants

Extensibility does not weaken Mallard's safety boundary.

### 9.1 Execution authority

Only compiled registrations can execute. Persisted keys and bounded JSON do
not supply behavior.

Plans must pin all relevant inputs:

- provider keys and provider-config versions;
- resource, policy, provenance, tombstone, and catalog coordinates;
- machine-local document revisions;
- remote revisions;
- content digests; and
- plan expiry.

Apply must resolve the same registrations and verify every pin again.

### 9.2 Credentials and local identity

Credentials, provider-home paths, checkout paths, local exclusions, catalog
target identities, trust decisions, receipts, and backups remain
machine-local.

Credentials must never enter:

- cloud manifests or resources;
- frontend DTOs;
- logs;
- errors;
- activity history; or
- continuity identity.

### 9.3 Filesystem and publication safety

All providers and contracts preserve:

- canonical-path validation;
- bounded reads and counts;
- no-follow traversal;
- symlink and special-file rejection;
- safe logical namespaces;
- atomic persistence;
- immutable, digest-verified publication;
- compare-and-swap mutable heads;
- explicit review before apply;
- backup before replacement;
- expected-local-digest checks for deletion; and
- exact, nonrecursive deletion of empty directories only.

Ordinary project-file sync remains limited to non-Git projects. Secret
detection remains a warning aid, not proof that content is safe.

## 10. Provider evidence and initial maturity

**Evidence reviewed:** 2026-09-03

This evidence selects initial coordinates and capability levels. It does not
make external product behavior part of the common Mallard schema.

| Capability | Evidence | Initial Mallard decision |
|---|---|---|
| Gemini Agent Skills | Gemini documents `SKILL.md`, `~/.gemini/skills`, and `~/.agents/skills`; Agent Skills defines a portable skill format | Reuse `agentskills.io.skill` only after product tests prove compatible semantics |
| Cursor Agent Skills | Cursor documents the Agent Skills standard, `SKILL.md`, `~/.cursor/skills`, and `~/.agents/skills` | Reuse `agentskills.io.skill` for the standard subset; use a distinct format for incompatible Cursor-only fields |
| Cursor plug-ins | Cursor documents root `plugin.json` and `.cursor-plugin/plugin.json` formats and local or UI installation; the reviewed pages do not define a stable noninteractive install API | Use `cursor.plugin-install-intent`; start with `Manual` apply until a version-pinned install and inventory contract is proven |
| Qoder pet | The reviewed Qoder pages describe built-in Qoduck and custom Skills, Plugins, and Connectors, but do not define a custom pet package contract | Register nothing. Registries are code, so a later release can add a format key without reserving one now |

Official references:

- [Gemini CLI: Agent Skills](https://geminicli.com/docs/cli/using-agent-skills/)
- [Gemini CLI: Create a skill](https://geminicli.com/docs/cli/tutorials/skills-getting-started/)
- [Agent Skills specification](https://agentskills.io/specification)
- [Cursor Agent Skills](https://cursor.com/docs/skills)
- [Cursor plug-ins](https://cursor.com/docs/plugins)
- [Cursor plug-in reference](https://cursor.com/docs/reference/plugins)
- [Qoder desktop pet](https://docs.qoder.com/qoder/desktop-pet)
- [Qoder extension publishing](https://docs.qoder.com/qoder/extension-publishing)

Provider adapters must use version-pinned tests for any external format or
local layout on which automatic capture or apply depends.

## 11. One-time clean cutover

This repository does not preserve the pre-cutover format. The cutover keeps
the top-level schema number `1` because all disposable pre-cutover local and
cloud state will be deleted before retesting.

Remove, do not migrate:

- flat provider-specific `StorageConfig` fields;
- a closed `StorageKind`;
- provider variants in `ConfiguredObjectStore`;
- the provider-specific public storage setup commands
  `launch_r2_agent_setup`, `upsert_r2_storage`, and `test_r2_storage`;
- the R2-only setup journal and its R2 field shape;
- the Codex-only public commands for conversation-path audit and repair and
  for the post-pull app restart;
- `supports_conditional_writes` on the common storage record;
- key-only listing and unconditional delete in the object-store trait;
- closed Library provider, resource-kind, format, and target enums;
- closed Library resource payload variants;
- closed Project Sync `ResourceKind`, `ApplyPolicy`, `Provenance`, and
  `TombstoneTarget` wire types;
- `CaptureResourceKind` and `CaptureApplyPolicy` in the common capture
  boundary;
- conversion matches from capture output to portable descriptors;
- frontend provider, kind, format, policy, provenance, or target unions; and
- whole-document provider-config replacement from the frontend.

Do not add:

- old-format readers;
- migrations;
- key aliases;
- dual writers;
- legacy commands;
- fallback parsing; or
- automatic import from the deleted state.

After this cutover, every addition in the compatibility table must satisfy the
no-write and no-rewrite guarantees.

## 12. Source ownership

Keep execution and safety responsibilities in their current layers.

| Area | Owner |
|---|---|
| Stable Project Sync coordinates, envelopes, validation, plans, and receipts | `crates/mallard-domain/` |
| Agent registry, discovery, capture, filters, codecs, and stable identity | `crates/mallard-infrastructure/src/providers/` and `provider_capture.rs` |
| Immutable publication, restore planning, backups, and apply | `crates/mallard-infrastructure/src/bundle_engine.rs` |
| Tauri orchestration and focused transactions | `src-tauri/src/project_sync/` |
| Bounded atomic machine-local persistence | `crates/mallard-infrastructure/src/persistence.rs` and catalog persistence |
| Neutral object-store trait, object keys, Local, R2, and Dropbox adapters | `crates/mallard-infrastructure/src/storage_core/` |
| Library coordinates, registries, codecs, inventory, plans, and apply | `src-tauri/src/storage_catalog/` |
| Provider-neutral frontend DTOs and views | `src/types.ts` and feature-local frontend modules |
| Storage drafts, commit transactions, and assistant launch | `crates/mallard-application/src/storage_setup.rs`, provider-neutral after Layer 2 |
| R2 provisioning assistant | `crates/mallard-cli/src/r2_browser_setup.rs`, inside the R2 adapter boundary; the CLI resolves storage providers through the same registry |
| Storage setup journal | `crates/mallard-infrastructure/src/storage_setup_journal.rs`, replacing `r2_setup_journal.rs` |
| Provider maintenance actions | adapter modules under `providers/`; the generic list and execute commands in `src-tauri/src/project_sync/` |

`mallard-domain` stays free of Tauri and filesystem concerns. The Library
remains independent of Project Sync feature DTOs and components.

Do not create one universal provider trait. Storage, agent, catalog, resource,
policy, provenance, and tombstone registrations have different authority.

## 13. Implementation sequence

Each layer must leave a working end-to-end product. A closed type is removed
in the layer that replaces it, and the matching section 14.4 source guard is
enabled in that same layer, never earlier. No layer may carry a temporary
provider-name branch to bridge to the next one; that is why the storage
envelope moves with the storage registry in Layer 2 rather than landing
alone in Layer 1.

### Layer 1 — Stable keys, bounded objects, and IPC

- add the validated key newtypes;
- add bounded config and portable objects with their limit tests and
  canonical form;
- add the agent and catalog profile config envelopes at version `1` with
  empty settings;
- add secret presence operations;
- replace whole-document frontend writes with focused commands; and
- prove no-write startup under the section 1.2 definition and
  unknown-record preservation.

### Layer 2 — Storage registry

- correct the object-store trait: revision-bearing listing and conditional
  delete for Local and R2, with the live R2 proof;
- add `StorageProviderRegistry` with duplicate rejection;
- replace the flat storage record with `StorageProviderConfig` and make Local
  and R2 compiled adapters;
- replace provider variants with `Arc<dyn ObjectStore>`;
- add generic setup descriptors, drafts, and the generic assistant launch,
  and move the R2 browser flow behind the R2 descriptor;
- replace the R2 setup journal with the neutral setup journal;
- remove the three R2-specific public commands; and
- pass the shared object-store conformance suite.

### Layer 3 — Portable resource registry

- replace the four closed Project Sync wire axes;
- register all current version-1 contracts;
- move the kind-to-provider consistency check into the resource-kind
  adapter and add the unregistered-tombstone map key;
- remove common capture enums and conversion matches;
- make plans pin exact contract coordinates; and
- prove current Codex and Claude behavior is unchanged.

### Layer 4 — Agent and catalog registries

- complete the agent adapter boundary and remove every branch in the section
  5.2 table;
- add `display_rank` and declared maintenance actions, and remove the three
  Codex-specific public commands;
- open all Library coordinate axes on the existing catalog adapter seed;
- register current Codex and Claude skills, the shared plug-in intent
  format, and the two Codex pet format versions;
- derive Library repository IDs from open coordinates; and
- make the frontend descriptor-driven, including ordering and actions.

### Layer 5 — Extension proofs

Use test-only registrations to add:

- one storage provider, with a declared setup assistant;
- one agent provider, with a declared maintenance action;
- one Library kind, format, format version, and target;
- one Project Sync resource kind;
- one apply policy;
- one provenance kind; and
- one tombstone target.

Each proof must add no common provider-name branch and require no state
conversion.

### Layer 6 — Product additions

Add Dropbox, Qoder, Cursor, or Gemini support only after the corresponding
extension proof passes. Each product addition then changes only its adapter
or codec, registry entry, descriptor assets, and tests.

## 14. Required release tests

### 14.1 No-wipe proof

Use temporary machine-local roots only.

1. Write schema-1 state with existing registrations, including one
   completed launch so the host identity document exists.
2. Snapshot every file and digest.
3. Start Mallard with an additional test registration.
4. Perform read-only listing, setup discovery, and the startup recovery
   command with no interrupted work.
5. Assert that no persistence write occurred outside the section 1.2
   exclusions and that every other file is byte-identical.

Then configure the new provider and assert:

- only its new record and owning revision changed;
- all existing records are equal;
- unknown records remain intact; and
- no migration or reset path ran.

### 14.2 Open-cloud proof

Use a temporary object store.

1. Publish existing Project Sync and Library schema-1 data.
2. Snapshot every key, byte string, head, and revision.
3. Register a new test coordinate.
4. Assert that registration causes no write.
5. Publish one resource at the new coordinate.
6. Assert that all old keys and bytes are unchanged.

An older registry must parse the new coordinate as unavailable and inert. A
later registry must activate it without conversion.

### 14.3 Registry and safety tests

| Area | Required proof |
|---|---|
| Storage | duplicate-key rejection, unknown provider, unsupported version, typed decoding, redaction, deterministic order, all object-store conformance cases including revision-bearing listing and the conditional-delete outcomes |
| Storage setup | generic assistant launch refuses a provider without an assistant, neutral journal round-trip and bounds, no R2 field in the common journal shape |
| Agent | current Codex and Claude parity, test provider addition, unknown scope preservation, isolated provider failure, display-rank ordering, declared maintenance actions listed and executed without a provider-name check |
| Library | test provider/kind/format/version/target additions, deterministic repository ID, invalid combination rejection, unknown coordinate inertness, descriptor-driven frontend |
| Project resources | test kind/policy/provenance/tombstone additions, exact recipe-policy match, direct capture coordinates, unknown contract scope isolation |
| IPC | focused revision conflicts, secret presence operations, unknown subtree preservation, no full config or local target identity in frontend DTOs |
| Apply | plan coordinate pins, digest and revision recheck, expiry, canonical paths, backups, deletion limits, receipts |

### 14.4 Source guards

Tests must reject reintroduction of:

- a closed storage-provider enum in persisted configuration;
- provider variants in `ConfiguredObjectStore`;
- provider-specific public commands of any kind, including setup and
  maintenance;
- an R2-specific or otherwise provider-specific setup journal shape;
- key-only listing or unconditional delete on the object-store trait;
- closed Library provider, kind, format, or target unions;
- closed portable `ResourceKind`, `ApplyPolicy`, `Provenance`, or
  `TombstoneTarget` discriminants;
- `CaptureResourceKind` or `CaptureApplyPolicy` in common capture output;
- provider-name matches in common execution modules; and
- complete provider documents sent to or replaced by the frontend.

Typed enums and matches inside one adapter are allowed.

### 14.5 Verification commands

Run the smallest relevant check during each layer. Run the full handoff set
after the cutover:

```bash
npm run build
npm run test:frontend-integration
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run test:backend-integration
npm run test:integration
```

Some object-store tests can require permission to bind a loopback server.

## 15. Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| Add more fields to flat `StorageConfig` | Every provider changes the common schema and validation path |
| Use larger tagged provider enums | The tag remains a closed portable extension axis |
| Open only the provider key | Kind, format, target, policy, provenance, and deletion remain closed |
| Change only `ResourceKind` to a string | An older build must still parse and preserve a newer bundle's policy, provenance, and tombstone records as inert; closed enums fail structural parsing and block the scope |
| Keep provider-specific setup and maintenance commands | The frontend would need a provider-name branch to show them, which section 5.3 forbids |
| Keep the R2 setup journal and add Dropbox fields | Every provider changes the common journal schema |
| Use one universal provider trait | It combines unrelated authority and weakens module boundaries |
| Load Rust dynamic libraries | It creates ABI, signing, update, sandbox, and trust problems |
| Let JSON manifests define commands or paths | Persisted data would become executable authority |
| Preserve pre-cutover readers and writers | There are no customers, and the project requires a clean cutover |

## 16. Completion criteria

The foundation is complete only when all these statements are true:

1. All affected machine-local document schemas and the Project Sync and
   Library portable schemas remain `1`.
2. Startup with a larger compiled registry performs no persistence or cloud
   write.
3. Existing post-cutover state needs no migration, wipe, or rewrite.
4. All persisted extension axes use validated versioned keys or envelopes.
5. Unknown keys and versions load, remain inert, and survive focused edits.
6. Only exact compiled registrations can execute.
7. Local and R2 use the same storage adapter contract and conformance suite.
8. Current Codex and Claude behavior uses the new agent, catalog, and portable
   resource registries.
9. A test-only provider or contract can be added without editing common
   provider-name branches.
10. Library repository identity includes provider, kind, format, and format
    version, and excludes local target and storage identity.
11. Project Sync plans pin every open resource-contract coordinate.
12. Frontend views are descriptor-driven and never receive saved secrets,
    full provider configs, provider-home paths, or local target identity.
13. Generic bounded objects cannot define executable behavior.
14. Review, integrity, path, backup, and deletion invariants remain enforced
    by common code.
15. Dropbox, Cursor, Gemini, and Qoder can be added after the cutover without
    changing a top-level schema or deleting `~/.mallard`.
16. The object-store trait carries revision-bearing listing and conditional
    delete, and Local and R2 pass those conformance cases, with a live R2
    proof of conditional delete.
17. No public command names a storage or agent provider. The R2 provisioning
    assistant and the Codex maintenance actions run through declared
    descriptors and generic commands.
18. The storage setup journal is provider-neutral and holds provider facts
    only inside a bounded object.
