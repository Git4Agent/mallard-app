# Plan: Rust Workspace and Project Structure Reorganization

- **Status:** Proposed
- **Date:** August 6, 2026
- **Implementation baseline:** Mallard 0.1.2 at `a5d41c5`
- **Product baseline:** ProjectSync, machine-local schema 1, portable bundle
  schema 1
- **Primary objective:** Reorganize the Rust backend into a workspace with
  explicit domain, application, infrastructure, desktop, and CLI boundaries
  without changing existing Mallard behavior
- **CLI scope in this plan:** A compilable, dependency-free placeholder only

## 1. Outcome

Mallard becomes a Cargo workspace with two entry-point packages:

- the existing Tauri desktop application; and
- a new, empty `mallard-cli` package reserved for future commands.

Both entry points will eventually use the same application services. This
plan creates that architecture incrementally while keeping the current React
frontend, Tauri command names, persistence schemas, bundle formats, safety
checks, tests, and release behavior intact.

The target is not a rewrite. Each migration step must leave the desktop
application buildable and testable, and each pull request must have an
independently useful or mechanically verifiable result.

## 2. Decision summary

1. Add a virtual Cargo workspace at the repository root.
2. Keep the React application in `src/`.
3. Keep the Tauri application in `src-tauri/`.
4. Keep the current Cargo package name `mallard` during this refactor.
5. Create separate `mallard-domain`, `mallard-application`, and
   `mallard-infrastructure` library packages under `crates/`.
6. Create `mallard-cli` as a separate package with an empty `fn main() {}`.
7. Do not package, install, advertise, or add commands to the CLI in this plan.
8. Move behavior by complete vertical slices rather than by performing one
   repository-wide file shuffle.
9. Keep Tauri commands as compatibility-stable adapters until all behavior
   behind them has moved.
10. Use Cargo resolver 2 initially to preserve the current Rust 2021 dependency
    resolution behavior. Evaluate resolver 3 as a separate follow-up.

## 3. Why this change is needed

The current Rust package contains clear conceptual modules, but several files
now combine multiple architectural responsibilities:

- `src-tauri/src/project_sync/commands.rs` contains 66 Tauri commands and is
  approximately 18,000 lines;
- `domain.rs` is approximately 5,800 lines;
- `bundle_engine.rs` is approximately 5,600 lines and contains both storage
  abstractions and concrete filesystem behavior;
- `persistence.rs` is approximately 3,000 lines and is mostly reusable except
  for its Tauri-specific construction path; and
- Tauri, AWS, SQLite, PTY, filesystem, and domain dependencies are declared by
  one Cargo package.

Adding a substantial CLI inside `src-tauri/src/bin/` would make the CLI depend
on the desktop package's dependency graph and would encourage it to call Tauri
or persistence internals directly. That arrangement is acceptable for a
short-lived prototype, but it is not the desired structure for a CLI that will
eventually cover projects, storage, history, sync planning, and other Mallard
operations.

## 4. Scope and non-goals

### 4.1 In scope

- a root Cargo workspace and root `Cargo.lock`;
- centralized package metadata and dependency versions where safe;
- a compile-only `mallard-cli` placeholder package;
- extraction of pure domain code;
- extraction of application use cases and external-service ports;
- extraction of persistence, storage, provider, filesystem, and platform
  adapters;
- decomposition of Tauri commands into thin command-family modules;
- relocation of tests to the package that owns the behavior;
- updates required for workspace-aware development and release scripts; and
- enforcement of dependency direction after the migration is complete.

### 4.2 Out of scope

- implementing any real CLI command;
- adding Clap or another CLI parser;
- installing the CLI on `PATH`;
- adding the CLI to a DMG, app bundle, MSI, NSIS installer, or updater;
- changing the React component structure;
- changing a Tauri command name or frontend invocation;
- changing machine-local schema 1 or portable bundle schema 1;
- changing the `~/.mallard` directory layout;
- changing provider capture rules;
- changing push, pull, restore, deletion, or trust semantics;
- renaming the desktop Cargo package or application executable;
- adopting Cargo resolver 3;
- introducing a new state database; or
- combining this refactor with product features from the R2 plans.

## 5. Plan precedence

This document owns the Rust workspace and package-layout decisions.

It supersedes only the structural assumption in:

- `PLAN_AGENT_ASSISTED_R2_SETUP.md`; and
- `PLAN_R2_STORAGE_CLI_SETUP.md`

that places `mallard-cli` under `src-tauri/src/bin/` and makes it part of the
desktop Cargo package.

Those documents continue to own their product requirements, secret-handling
requirements, Cloudflare behavior, and future CLI experience. Their CLI work
must begin only after this reorganization reaches the appropriate application
and infrastructure boundaries.

## 6. Target repository structure

```text
mallard-app/
├── Cargo.toml
├── Cargo.lock
├── package.json
├── src/
│   └── ... existing React frontend ...
│
├── src-tauri/
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   └── src/
│       ├── main.rs
│       ├── lib.rs
│       ├── commands/
│       │   ├── mod.rs
│       │   ├── activity.rs
│       │   ├── config.rs
│       │   ├── projects.rs
│       │   ├── storage.rs
│       │   ├── sync.rs
│       │   ├── history.rs
│       │   └── setup.rs
│       └── sync_tests/
│           └── ... cross-layer desktop tests retained as needed ...
│
└── crates/
    ├── mallard-domain/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── identity.rs
    │       ├── config.rs
    │       ├── project.rs
    │       ├── storage.rs
    │       ├── bundle.rs
    │       ├── restore.rs
    │       ├── conversation.rs
    │       └── setup.rs
    │
    ├── mallard-application/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── error.rs
    │       ├── services.rs
    │       ├── ports/
    │       │   ├── mod.rs
    │       │   ├── state_repository.rs
    │       │   ├── bundle_store.rs
    │       │   ├── provider.rs
    │       │   ├── activity_sink.rs
    │       │   └── clock.rs
    │       └── use_cases/
    │           ├── mod.rs
    │           ├── config.rs
    │           ├── storage.rs
    │           ├── projects.rs
    │           ├── sync.rs
    │           ├── history.rs
    │           └── setup.rs
    │
    ├── mallard-infrastructure/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── persistence/
    │       │   └── mod.rs
    │       ├── storage/
    │       │   ├── mod.rs
    │       │   ├── local.rs
    │       │   └── s3.rs
    │       ├── providers/
    │       │   ├── mod.rs
    │       │   ├── codex.rs
    │       │   └── claude.rs
    │       ├── activity/
    │       │   └── jsonl.rs
    │       ├── filesystem/
    │       │   └── mod.rs
    │       └── platform/
    │           ├── mod.rs
    │           ├── macos.rs
    │           └── windows.rs
    │
    └── mallard-cli/
        ├── Cargo.toml
        └── src/
            └── main.rs
```

This is the desired end state. Except for the explicitly requested CLI
placeholder, the implementation should not create empty packages or empty
module trees far in advance of moving their first owned behavior.

## 7. Architecture and dependency direction

The compile-time dependency graph is:

```text
mallard-domain
       ↑
mallard-application  ←  mallard-infrastructure
       ↑                       ↑
       ├── mallard desktop ────┤
       └── mallard-cli ────────┘
```

At runtime, application services call port traits that are implemented by
infrastructure adapters. The dependency direction remains inward because the
port traits are owned by `mallard-application`.

### 7.1 `mallard-domain`

Owns:

- validated IDs and value objects;
- machine-local and portable schema types;
- manifests, heads, plans, actions, receipts, and policy decisions;
- validation limits and invariants;
- pure digest and comparison behavior; and
- pure domain tests.

May depend on:

- `serde`;
- cryptographic primitives needed to represent or verify domain digests; and
- other small, platform-independent libraries.

Must not depend on:

- Tauri or Tauri plugins;
- Clap;
- Tokio runtime facilities;
- AWS SDK crates;
- SQLite;
- PTY or process-launching libraries;
- `dirs`;
- filesystem traversal;
- operating-system APIs; or
- React-facing transport behavior.

Durable schema DTOs may remain serializable domain types. Tauri-only request
and response DTOs do not belong here.

### 7.2 `mallard-application`

Owns:

- application services and use cases;
- input and result types shared by multiple entry points;
- typed application errors;
- transaction and orchestration order;
- external-service port traits;
- revision, digest, expiry, and approval coordination; and
- tests using in-memory or deterministic fake ports.

Must not depend on:

- Tauri;
- Clap;
- AWS SDK implementations;
- a concrete filesystem repository;
- a concrete provider-home implementation; or
- OS-specific APIs.

The application layer must preserve the ordering currently enforced by Tauri
commands. Moving a function is not permission to simplify a safety sequence.

### 7.3 `mallard-infrastructure`

Owns:

- `LocalStateRepository` and persistence locking;
- local-folder and S3-compatible object stores;
- filesystem traversal, canonicalization, bounded reads, and atomic writes;
- provider discovery, capture, codecs, and materialization;
- SQLite backup and restore mechanics;
- Git and chat-history adapters;
- JSONL activity persistence;
- platform-specific process and filesystem integration; and
- adapter integration tests using temporary homes and loopback services.

May depend on the heavy runtime crates currently declared by the desktop
package, including AWS SDK, Tokio, SQLite, PTY, `notify`, and platform APIs.

It must implement application ports without importing Tauri types.

### 7.4 Tauri desktop package

Owns:

- Tauri builder and plugin initialization;
- managed application state and dependency wiring;
- JavaScript-to-Rust command request mapping;
- mapping typed application errors to the existing frontend error contract;
- frontend event emission;
- dialogs, openers, windows, and other desktop capabilities; and
- desktop integration tests.

Tauri command handlers should eventually contain only:

1. Tauri argument extraction;
2. conversion into an application request;
3. one application service call; and
4. conversion into the existing serialized response.

The existing `src-tauri/src/main.rs` remains unchanged except when required by
Tauri itself.

### 7.5 Empty CLI package

During this plan, `mallard-cli` contains no Mallard behavior:

```toml
[package]
name = "mallard-cli"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[[bin]]
name = "mallard-cli"
path = "src/main.rs"
```

```rust
fn main() {}
```

The placeholder:

- has no dependencies;
- does not read or write Mallard state;
- is not linked to Tauri;
- has no commands, flags, or output;
- exits successfully;
- is not included in packaged artifacts; and
- exists only to reserve the package, binary name, workspace location, and
  future entry-point boundary.

## 8. Current-to-target ownership map

| Current source | Target ownership |
| --- | --- |
| `project_sync/domain.rs` | `mallard-domain`, split by bounded context |
| `project_sync/commands.rs` | orchestration to `mallard-application`; thin handlers to `src-tauri/src/commands/` |
| `project_sync/persistence.rs` | `mallard-infrastructure/persistence` |
| `project_sync/s3_store.rs` | `mallard-infrastructure/storage/s3.rs` |
| local object-store code in `bundle_engine.rs` | `mallard-infrastructure/storage/local.rs` |
| publication and restore orchestration in `bundle_engine.rs` | `mallard-application/use_cases/sync.rs` |
| filesystem restore/apply code in `bundle_engine.rs` | `mallard-infrastructure/filesystem` |
| `project_sync/provider_capture.rs` | `mallard-infrastructure/providers` |
| `project_sync/chat_history.rs` | parsing/adapters in infrastructure; query orchestration in application |
| `project_sync/conversation_codec.rs` | provider adapters in infrastructure |
| `project_sync/conversation_materialization.rs` | provider/filesystem adapters in infrastructure |
| `project_sync/conversation_sync.rs` | application sync use cases |
| `project_sync/codex_app_refresh.rs` | infrastructure platform/provider adapter |
| `project_sync/codex_native_fork.rs` | infrastructure platform/provider adapter |
| `project_sync/codex_rollout.rs` | infrastructure provider adapter |
| `src-tauri/src/codex_binary.rs` | infrastructure platform/provider adapter |
| `src-tauri/src/activity_log.rs` | event contract/port in application, JSONL implementation in infrastructure, Tauri commands/emission in desktop |

Some files contain both application and infrastructure behavior. They must be
split by responsibility rather than moved wholesale into a convenient package.

## 9. Contracts that must not change

### 9.1 Frontend and Tauri contract

The following stay stable:

- command names registered in `src-tauri/src/lib.rs`;
- camelCase JavaScript argument keys;
- serialized Rust field names;
- request optionality and defaults;
- response shapes mirrored by `src/types.ts`;
- error strings when the frontend depends on them;
- event names and payload shapes; and
- single-flight and revision behavior in frontend callers.

Moving a DTO to another crate must not change its serialized form.

### 9.2 Persistence contract

The following stay stable:

- `~/.mallard` as the machine-local root;
- schema 1 filenames and JSON shapes;
- bounded reads and collection sizes;
- atomic replacement;
- `persistence.lock` cross-process coordination;
- optimistic revision checks;
- no-follow traversal;
- canonical-path validation;
- symlink and special-file rejection; and
- sanitized error and activity output.

`LocalStateRepository::from_app` should become desktop wiring around a
Tauri-free constructor such as `from_home_dir` or a validated `StatePaths`
value. The repository itself must not retain an `AppHandle`.

### 9.3 Sync and restore contract

The following stay stable:

- immutable publication;
- SHA-256 verification;
- compare-and-swap mutable heads;
- reviewed push and pull plans;
- plan digest, revision, expiry, and approval checks;
- backup and receipt behavior;
- exact, digest-checked file deletion;
- exact-empty-directory deletion only;
- non-Git project-content restrictions;
- provider-path and trust decisions remaining local; and
- best-effort secret warnings and executable-content acknowledgement.

### 9.4 Release contract

The following stay stable:

- application version agreement;
- Tauri configuration discovery in `src-tauri`;
- frontend build paths;
- application identifiers and product name;
- signing and bundle verification;
- release artifact names; and
- no CLI binary in release bundles yet.

## 10. Workspace manifest design

The workspace membership grows as packages are extracted. Phase 1 contains
only the existing desktop package and the requested CLI placeholder:

```toml
[workspace]
members = [
  "src-tauri",
  "crates/mallard-cli",
]
default-members = ["src-tauri"]
resolver = "2"

[workspace.package]
version = "0.1.2"
edition = "2021"
license = "Apache-2.0"
publish = false

[profile.dev.package."*"]
opt-level = 1
```

Add `mallard-domain`, `mallard-application`, and `mallard-infrastructure` to
`members` in the phase that creates each package. The final membership is:

```toml
[workspace]
members = [
  "src-tauri",
  "crates/mallard-cli",
  "crates/mallard-domain",
  "crates/mallard-application",
  "crates/mallard-infrastructure",
]
default-members = ["src-tauri"]
resolver = "2"
```

Important workspace consequences:

1. `src-tauri/Cargo.lock` moves to the repository root.
2. Cargo build output moves to the workspace target directory by default.
3. Cargo profiles are honored only from the workspace root, so the existing
   dependency optimization profile must move to the root manifest.
4. `src-tauri/Cargo.toml` should inherit workspace package metadata.
5. Each member declares only the dependencies it actually uses.
6. Shared dependency versions may move to `[workspace.dependencies]`, but a
   member must still opt into each dependency.
7. The CLI is excluded from `default-members` while it is an empty placeholder.

Do not change resolver version and package topology in the same pull request.
Resolver 3 can be evaluated later with a separate lockfile and feature-graph
review.

## 11. Migration strategy

### 11.1 Rules

- No big-bang move.
- No feature work in a structural pull request.
- One architectural boundary or vertical slice per pull request.
- Preserve public names while moving private implementation.
- Move tests with the behavior they protect.
- Add characterization coverage before extracting poorly isolated behavior.
- Keep the old Tauri command as the active entry point until the shared use
  case passes all existing tests.
- Delete the old implementation in the same pull request that switches the
  command; do not keep two production paths.
- Do not create temporary persistence formats or compatibility files.
- Keep the full integration suite green after every merged step.

### 11.2 Why vertical slices

A layer-by-layer move of every domain type, then every repository, then every
service would leave the repository in a partially integrated state for too
long. Instead, after the domain foundation is extracted, move complete
capabilities in this order:

1. configuration and storage;
2. projects and bindings;
3. history and inventory;
4. bundle status and verification;
5. push planning and publication;
6. pull planning and restore;
7. conversations and provider materialization;
8. setup, recovery, and platform actions.

Each slice should end with its existing Tauri commands calling the new
application service and infrastructure adapters.

## 12. Phased implementation plan

### Phase 0 — Record the behavioral baseline

Purpose: make regressions attributable to the refactor.

Actions:

1. Run all committed integration suites on the existing package layout.
2. Record the current `cargo metadata` package graph.
3. Inventory all registered Tauri commands and frontend invocations.
4. Identify serialized fixtures that cover local state, manifests, plans,
   actions, receipts, and activity entries.
5. Add characterization tests only where a migration target lacks coverage.
6. Confirm no tests use real `~/.mallard`, `~/.codex`, or `~/.claude`.

Required checks:

```text
npm run build
npm run test:frontend-integration
npm run test:release-config
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run check:release-version
```

Acceptance:

- the baseline is green or every pre-existing failure is documented;
- all 66 Tauri commands have an owning command family; and
- schema fixtures exist for every durable document moved later.

### Phase 1 — Add the workspace and empty CLI placeholder

Files:

- new root `Cargo.toml`;
- move `src-tauri/Cargo.lock` to root `Cargo.lock`;
- update `src-tauri/Cargo.toml`;
- new `crates/mallard-cli/Cargo.toml`;
- new `crates/mallard-cli/src/main.rs`;
- update `package.json`;
- update `scripts/check-release-version.mjs`;
- update `scripts/tauri-build.mjs`; and
- update `tests/release-config.test.mjs`.

Actions:

1. Create the virtual workspace with resolver 2.
2. Keep the desktop package name `mallard`.
3. Inherit version, edition, license, and publish policy from the workspace.
4. Move the existing development dependency optimization profile to the root.
5. Generate one root lockfile without intentionally upgrading dependencies.
6. Create the dependency-free CLI placeholder.
7. Make backend test scripts workspace-aware.
8. Update version checking to read the workspace version.
9. Stop hardcoding `src-tauri/target` in release verification. Resolve Cargo's
   target directory from `cargo metadata` or one centralized helper.
10. Keep the CLI out of Tauri `bundle.resources`, `bundle.externalBin`, and
    installer configuration.

Acceptance:

```text
cargo check --workspace
cargo test --workspace
cargo run -p mallard-cli
npm run build
npm run test:integration
npm run check:release-version
```

Additionally:

- `cargo run -p mallard-cli` exits successfully with no output;
- the Tauri application starts normally;
- the DMG verification helper locates workspace build output correctly; and
- the dependency versions in the lockfile do not drift unexpectedly.

### Phase 2 — Extract the domain package mechanically

Purpose: establish a Tauri-free core without redesigning domain behavior.

Actions:

1. Create `crates/mallard-domain`.
2. Move `project_sync/domain.rs` into the new package as one mechanical unit
   first.
3. Preserve public type names, derives, serde attributes, constants, and
   validation functions.
4. Update imports in the desktop package and tests.
5. Keep durable schema fixture comparisons byte- or value-equivalent.
6. Do not split the large file in the same commit as the crate extraction.

Acceptance:

- `mallard-domain` has no Tauri, AWS, SQLite, PTY, or filesystem dependencies;
- all existing Rust tests pass;
- serialized local-state and bundle fixtures are unchanged; and
- the frontend TypeScript contract requires no changes.

### Phase 3 — Split the domain by bounded context

Purpose: reach the target domain layout after the crate boundary is stable.

Recommended order:

1. `identity.rs`;
2. `storage.rs` and `config.rs`;
3. `project.rs`;
4. `bundle.rs`;
5. `restore.rs`;
6. `conversation.rs`; and
7. `setup.rs`.

Actions:

- move cohesive types and their validation/tests together;
- keep cross-context dependencies explicit through imports;
- use `lib.rs` to expose the intended domain API;
- avoid a blanket `pub use *` once the split is complete; and
- keep filesystem paths as validated domain values only where they are part of
  a local application request, never a portable manifest.

Acceptance:

- no domain module exceeds an agreed reviewable size without a documented
  reason;
- no dependency-direction violation is introduced; and
- all schema and behavioral tests remain unchanged.

### Phase 4 — Create application and infrastructure foundations

Purpose: establish ports, typed errors, and Tauri-free path construction before
moving feature behavior.

Actions:

1. Create `mallard-application` and `mallard-infrastructure`.
2. Define the smallest initial ports required by persistence:
   `StateRepository`, `Clock`, and `ActivitySink`.
3. Move Tauri-free state path construction and `LocalStateRepository` into
   infrastructure.
4. Move the persistence guard with the repository.
5. Leave the `AppHandle`-to-home-directory conversion in the desktop package.
6. Split activity JSONL persistence from Tauri event emission.
7. Map typed application errors back to the existing Tauri `Result<T, String>`
   behavior at the desktop edge.

Do not define ports for every helper. Add a port only when:

- an external system is crossed;
- more than one implementation is expected; or
- deterministic tests need substitution.

Acceptance:

- persistence and JSONL activity writing compile without Tauri;
- cross-process lock tests still pass;
- Tauri command and event payloads are unchanged; and
- no second repository or state-writing path exists.

### Phase 5 — Move configuration and storage as the first vertical slice

Purpose: prove the architecture on the capability needed by the future CLI.

Application ownership:

- load and validate configuration;
- list storage definitions;
- add, update, and remove storage definitions;
- revision conflict handling;
- storage-isolation validation;
- connectivity probe orchestration; and
- sanitized results.

Infrastructure ownership:

- local configuration repository;
- local-folder store;
- S3/R2 store;
- AWS client construction; and
- connectivity probe implementation.

Desktop ownership:

- current Tauri command DTOs;
- dialogs and frontend events; and
- mapping requests/results to the existing frontend contract.

Acceptance:

- every existing storage Tauri command calls the application service;
- the frontend behaves identically;
- R2/S3 and local-folder tests pass;
- concurrent repository mutation still uses `persistence.lock`; and
- the CLI placeholder remains empty and does not depend on these crates yet.

### Phase 6 — Move projects, links, profiles, and bindings

Actions:

1. Move project registration, rename, and removal orchestration.
2. Move storage links and bundle recipes.
3. Move provider-profile management.
4. Move project bindings and materialization queries.
5. Keep all portable/local-field separation intact.
6. Split corresponding Tauri commands into `config.rs`, `projects.rs`, and
   `storage.rs`.

Acceptance:

- project and binding Tauri contracts are unchanged;
- Git and non-Git policy tests pass;
- local-only fields never enter portable manifests; and
- revision and single-flight behavior remain intact.

### Phase 7 — Move history, inventory, and provider reads

Actions:

1. Place provider parsing, filesystem discovery, and Git access in
   infrastructure.
2. Place history and inventory query orchestration in application use cases.
3. Keep provider-specific codecs behind provider adapter boundaries.
4. Preserve bounded reads, path canonicalization, and secret redaction.
5. Split Tauri handlers into `history.rs` and the appropriate project module.

Acceptance:

- Codex and Claude history results are unchanged;
- inventory ordering and stable resource identities are unchanged;
- provider fixtures produce equivalent results; and
- no application module reads a provider home directly.

### Phase 8 — Split bundle publication and restore

This is the highest-risk migration and must be split into small pull requests.

Actions:

1. Move `BundleObjectStore` into an application-owned port.
2. Move local and S3 implementations into infrastructure.
3. Move immutable publication orchestration into application.
4. Move filesystem staging, backup, apply, and recovery mechanics into
   infrastructure.
5. Keep plan construction and policy decisions in domain/application.
6. Keep exact digest, revision, expiry, and approval checks.
7. Route current status, verify, push, plan-restore, and apply-restore Tauri
   commands through the extracted services.

Acceptance:

- publication remains immutable and SHA-256 verified;
- mutable heads retain compare-and-swap behavior;
- restore plans and receipts serialize identically;
- failure-injection and recovery tests pass;
- deletion invariants remain intact; and
- no Tauri type crosses into application or infrastructure.

### Phase 9 — Move conversations, setup, and platform behavior

Actions:

1. Move conversation sync orchestration into application.
2. Move codecs, materialization, provider indexes, and sidecars into provider
   infrastructure adapters.
3. Move Codex process discovery, app refresh, native fork, rollout, and PTY
   behavior into platform/provider infrastructure.
4. Move setup-draft orchestration into application.
5. Keep Tauri opening, dialogs, and frontend event emission in desktop.
6. Split remaining handlers into `setup.rs`, `history.rs`, and `sync.rs`.

Acceptance:

- conversation metadata and provider-native files remain consistent;
- fork and divergence recovery behavior is unchanged;
- setup drafts preserve revision and expiry semantics;
- platform-specific tests pass on their supported runners; and
- desktop-only UI operations remain outside application services.

### Phase 10 — Finish the thin desktop adapter

Actions:

1. Remove the old `project_sync/commands.rs`.
2. Register command-family modules from `src-tauri/src/lib.rs`.
3. Build application services and adapters in one desktop composition root.
4. Remove temporary internal forwarding modules and obsolete re-exports.
5. Reduce desktop dependencies to Tauri-specific crates plus the shared
   workspace packages.
6. Move remaining tests to their owning package where practical.
7. Add an automated dependency-boundary check.

The dependency-boundary check should reject:

- `tauri` or `clap` in domain/application;
- AWS SDK, SQLite, PTY, or OS APIs in domain/application;
- imports from `src-tauri` by another package; and
- direct local-state writes outside the infrastructure repository.

Acceptance:

- Tauri handlers are adapter-sized;
- no production behavior remains duplicated;
- `cargo test --workspace` covers all Rust packages;
- the complete frontend and release suites pass; and
- the final repository matches the target structure.

## 13. Pull request boundaries

Recommended pull requests:

1. Workspace, root lockfile, release-script adaptation, and empty CLI.
2. Mechanical `mallard-domain` extraction.
3. Domain bounded-context split.
4. Persistence, activity, and application/infrastructure foundations.
5. Configuration and storage vertical slice.
6. Projects, links, profiles, and bindings.
7. History, inventory, and provider reads.
8. Bundle-store port and adapters.
9. Push publication use cases.
10. Pull planning, apply, and recovery use cases.
11. Conversations and provider materialization.
12. Setup and platform actions.
13. Tauri command-module cleanup and dependency enforcement.

Pull requests 8–12 may be divided further if their behavioral diff becomes
difficult to review. They must not be combined merely to finish the directory
tree sooner.

## 14. Verification matrix

| Change area | Required checks |
| --- | --- |
| Workspace or Cargo metadata | `cargo check --workspace`, `cargo test --workspace`, lockfile review |
| Domain types or serialization | domain tests, Rust integration tests, fixture equivalence |
| Application use cases | unit tests with fake ports and existing cross-layer tests |
| Persistence or filesystem | Rust library tests, temporary-home tests, failure injection |
| S3/R2 adapters | storage tests including loopback conditional-write tests |
| Tauri contract | `npm run build`, frontend integration tests, Rust tests |
| Release scripts or paths | version check, release-config tests, packaged build verification |
| Platform behavior | supported macOS and Windows CI runners |

Final handoff checks:

```text
npm run build
npm run test:frontend-integration
npm run test:release-config
npm run check:release-version
cargo check --workspace
cargo test --workspace
MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build
```

The Windows release runner must also build and inspect the packaged desktop
application. It must confirm that `mallard-cli.exe` is absent until a later
packaging plan explicitly adds it.

## 15. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Workspace changes Cargo's target path | Resolve `target_directory` with `cargo metadata`; do not hardcode `src-tauri/target` |
| Workspace silently changes dependency selection | Start with resolver 2 and review the root lockfile without dependency upgrades |
| Cargo profiles stop applying | Move profiles to the root manifest in the workspace pull request |
| Serde output changes during type moves | Preserve derives/attributes and compare durable fixtures |
| Tauri command behavior drifts | Keep command names and DTOs at the desktop edge; add parity tests |
| Business logic is duplicated during migration | Switch each command and delete its old implementation in the same pull request |
| GUI and future CLI race on local state | Retain the same infrastructure repository and `persistence.lock` |
| Traits create unnecessary complexity | Define ports only at external or test-substitution boundaries |
| Infrastructure becomes a new monolith | Organize it by adapter family and move one vertical slice at a time |
| Heavy dependencies leak into all packages | Declare dependencies only in the member that uses them |
| Cross-platform behavior is accidentally generalized | Keep macOS and Windows implementations explicit behind platform adapters |
| Release verification breaks after lock/target relocation | Update and test release scripts in Phase 1 before moving behavior |
| The empty CLI is mistaken for a supported product | Do not package, document as user-facing, or expose it from the UI |

## 16. Definition of done

The reorganization is complete when:

- the repository has one root Cargo workspace and lockfile;
- `src-tauri` remains the desktop entry point;
- the frontend works without invocation or DTO changes;
- domain rules live in `mallard-domain`;
- shared orchestration lives in `mallard-application`;
- external integrations live in `mallard-infrastructure`;
- Tauri commands are thin adapters grouped by capability;
- no Tauri type appears in domain, application, or infrastructure;
- all current state, bundle, plan, receipt, and activity schemas are preserved;
- all safety invariants and integration tests pass;
- release tooling is workspace-aware on macOS and Windows;
- `mallard-cli` is a separate, empty, compilable package;
- the CLI is not shipped or installed; and
- future CLI work can add an adapter without extracting business logic from
  Tauri again.

## 17. Deferred follow-up plans

After this plan is complete, use separate plans for:

1. the CLI command and output contract;
2. storage and agent-assisted R2 commands;
3. macOS and Windows CLI packaging and PATH installation;
4. resolver 3 and minimum-supported-Rust-version evaluation;
5. optional renaming of the desktop Cargo package; and
6. any frontend component reorganization.

## 18. Industry references

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html)
- [Tauri project structure](https://v2.tauri.app/start/project-structure/)
- [AWS Prescriptive Guidance: hexagonal architecture best practices](https://docs.aws.amazon.com/prescriptive-guidance/latest/hexagonal-architectures/best-practices.html)
- [Command Line Applications in Rust](https://rust-cli.github.io/book/)
- [Clap derive reference](https://docs.rs/clap/latest/clap/_derive/)
