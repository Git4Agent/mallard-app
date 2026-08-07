# Plan: Behavior-Preserving Rust Workspace Reorganization

- **Status:** Proposed
- **Date:** August 6, 2026
- **Implementation baseline:** Mallard 0.1.2 at `2aa6c7a`
- **Change model:** In-place refactor; every existing behavior is preserved
- **Backward compatibility:** Total — old and new builds are interchangeable
- **Data migration:** None, because no data contract changes
- **User action required:** None (no reset, no reconfiguration)
- **CLI scope:** A compilable, dependency-free placeholder only

## 1. Outcome

Rebuild Mallard's Rust organization as a Cargo workspace with explicit
domain, application, infrastructure, desktop, and future CLI boundaries —
without changing anything a user, the frontend, existing local state, or
existing cloud storage can observe.

This is a pure restructuring. Code moves between crates and module paths
change internally, but serialized bytes, IPC payloads, event names, on-disk
layouts, cloud object layouts, and packaged artifacts stay identical. A user
upgrades in place; nothing is reset, migrated, or reconfigured. Installing
the previous build again afterwards is safe, because the state it reads is
unchanged.

This revision supersedes the earlier clean-cutover draft of this plan. The
target structure is the same; the schema replacement, new IPC contract,
mandatory reset, and fail-closed old-state rejection are all withdrawn.

## 2. Frozen contracts

The following are contracts, verified before, during, and after the work.
No phase may change any of them:

1. **Machine-local state.** The schema-1 layout under `~/.mallard`: every
   document path, file format, field name, lock protocol, and byte the
   current build writes (see `CURRENT_METADATA_LAYOUT.md`).
2. **Portable bundle storage.** The schema-1
   `.mallard/v1/repositories/<bundle-id>/` object layout, key names, digest
   algorithms, and compare-and-swap head semantics, for both local-folder
   and S3-compatible/R2 destinations.
3. **Tauri IPC.** All 71 registered commands (66 in
   `project_sync/commands.rs`, 5 in `activity_log.rs`): names, argument
   shapes, response shapes, error strings, and the full
   `generate_handler!` registration list.
4. **Events.** Every emitted event: name, payload shape, and emission order
   relative to the operation that produces it.
5. **Frontend.** `src/` requires zero changes. An empty
   `git diff -- src/` at every merge is the acceptance check, not a hope.
6. **Activity log.** The JSONL schema, sanitization behavior, and file
   locations.
7. **Packaging identity.** Product name `Mallard`, identifier
   `com.hequ.agent-sync`, Cargo package name `mallard`, library name
   `tauri_app_lib`, executable and DMG naming, and bundle contents.
   (Renaming the desktop package is deliberately deferred; the package name
   feeds the binary name and bundle.)
8. **Dependency selection.** `src-tauri/Cargo.lock` is *moved* to the
   repository root, never regenerated. Dependency versions and features do
   not drift as a side effect of this plan.
9. **Safety behavior.** Canonical-path validation, bounded I/O, atomic
   writes, cross-process locking, immutable publication, SHA-256
   verification, compare-and-swap heads, reviewed push/pull application,
   digest-checked deletion, credential redaction, and sanitized activity
   events — all preserved by moving their implementations without
   functional edits, with their existing tests still passing.

"Preserved behavior" means these observable surfaces. Internal module
paths, file names, crate boundaries, and `use` statements are free to
change.

## 3. Change rules

### 3.1 Moves over rewrites

Relocate code mechanically. A moved function's diff should consist of
import-path adjustments and nothing else. Do not rename fields, reorder
serde attributes, change derive lists, reword error strings, or "improve"
logic in passing. If a move reveals a bug, record it and fix it in a
separate, ordinary bug-fix change — not inside a relocation commit.

### 3.2 Temporary re-export shims are allowed

During extraction, the old module path may forward to the new crate:

```rust
// src-tauri/src/project_sync/mod.rs (temporary)
pub use mallard_domain as domain;
```

This keeps the not-yet-moved code compiling unchanged while a module moves
out. Shims are internal and invisible to every frozen contract. They are
inventoried as they are added and deleted in Phase 5; the final tree
contains none.

### 3.3 Tests are the harness, not a work item

The existing suites are the primary proof of preservation:

- `npm run test:frontend-integration`
- `npm run test:release-config`
- `npm run check:release-version`
- `npm run test:backend-integration`
- `cargo test` (unit tests travel with their modules; the headless
  `sync_tests` integration suite stays in `src-tauri` and keeps driving the
  real Tauri commands through the mock runtime)

Test edits are limited to import paths. Editing a test's assertions or
fixtures to make a phase pass is a stop-the-line signal that behavior
changed. Adding *new* freeze tests (for example, a command-registration
snapshot) is encouraged; weakening existing ones is not.

### 3.4 Tauri coupling is wrapped, not rewritten

Where moved code touches Tauri today, leave a thin desktop-side wrapper
instead of redesigning the flow:

- `persistence.rs` has exactly one Tauri reference — the
  `from_app<R: tauri::Runtime>` constructor. The store moves to
  infrastructure with `from_home_dir`; a small `from_app` helper stays in
  the desktop crate and delegates.
- `commands.rs` orchestration uses `AppHandle` for event emission (66
  `emit` calls). Extracted application functions take an event-sink
  callback; the desktop wrapper passes a closure that forwards to
  `app.emit` with the identical event name and payload, emitted at the
  identical point in the flow.
- `activity_log.rs` splits: the JSONL persistence core (no Tauri) moves to
  infrastructure; its 5 Tauri commands and event emission stay in the
  desktop crate.

All other candidate modules — `domain.rs`, `bundle_engine.rs`,
`s3_store.rs`, `provider_capture.rs`, `chat_history.rs`,
`conversation_codec.rs`, `conversation_materialization.rs`,
`conversation_sync.rs`, `codex_app_refresh.rs`, `codex_native_fork.rs`,
`codex_rollout.rs`, `codex_binary.rs` — have zero Tauri references today
and move cleanly.

### 3.5 No concurrent feature work

No new features, schema fields, commands, or UI changes land inside this
plan's phases. Feature plans (see §5) run entirely before or entirely after
this restructure — never interleaved with it.

## 4. Scope

### 4.1 In scope

- a virtual Cargo workspace at the repository root;
- one root `Cargo.lock` (moved, not regenerated) and one root target
  directory;
- shared package metadata and the existing dev profile moved to the root;
- `mallard-domain`, `mallard-application`, and `mallard-infrastructure`
  library packages populated by mechanical extraction;
- `src-tauri` reduced to a thin desktop adapter with unchanged commands;
- an empty `mallard-cli` package reserving the future entry-point boundary;
- workspace-aware build, release, version-check, and CI-cache updates;
- freeze tests that pin the command registration list; and
- verification that a pre-restructure `~/.mallard` and existing storage
  destinations work unchanged, including after downgrading back.

### 4.2 Out of scope

- any change to local state, bundle, cloud-key, IPC, event, or log schemas;
- any frontend change;
- renaming the desktop package, library, binary, or bundle identity;
- hexagonal ports, trait boundaries, or fake-port test scaffolding;
- real CLI commands, Clap, CLI packaging, or `PATH` installation;
- R2 provisioning or agent-assisted setup;
- dependency upgrades, edition changes, or `rust-version` adoption;
- Windows CI (none exists today; the status quo is preserved); and
- behavior fixes, performance work, or API redesign of any kind.

## 5. Plan precedence and sequencing

This document owns the Rust workspace layout and package boundaries.

- `PLAN_AGENT_ASSISTED_R2_SETUP.md` and `PLAN_R2_STORAGE_CLI_SETUP.md`:
  their assumption that `mallard-cli` lives in `src-tauri/src/bin/` is
  superseded — the CLI boundary is `crates/mallard-cli`, and their future
  implementation targets the application and infrastructure packages. Their
  product and secret-handling requirements are unaffected.
- `PLAN_STORAGE_SCOPED_AGENT_CATALOG.md` (proposed the same day): must not
  interleave with this restructure. Recommended order: land this plan
  first — it is mechanical, behavior-neutral, and short-lived — then build
  the catalog on the new layout. If the catalog lands first instead, this
  plan rebases and re-runs its Phase 0 inventory.
- `PLAN_REMOVE_BACKWARD_COMPAT.md` (executed 2026-07-28): established the
  single schema-1 local and bundle formats. This plan freezes exactly that
  state; nothing here reintroduces migration or compatibility machinery,
  because nothing changes shape.

## 6. Target repository structure

```text
mallard-app/
├── Cargo.toml                 # virtual workspace root
├── Cargo.lock                 # moved from src-tauri/, not regenerated
├── package.json
├── src/                       # React frontend — untouched
│
├── src-tauri/                 # desktop adapter (package name stays `mallard`)
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── icons/
│   └── src/
│       ├── main.rs
│       ├── lib.rs             # builder, managed state, registration list
│       ├── activity_log.rs    # 5 Tauri commands + emission (JSONL core moves out)
│       ├── commands/          # Phase 5 file split of today's command wrappers
│       │   ├── mod.rs
│       │   ├── activity.rs
│       │   ├── config.rs
│       │   ├── projects.rs
│       │   ├── storage.rs
│       │   ├── sync.rs
│       │   ├── history.rs
│       │   └── setup.rs
│       └── sync_tests/        # headless integration suite — stays, unchanged
│
└── crates/
    ├── mallard-domain/        # domain.rs content, split into modules
    │   ├── Cargo.toml
    │   └── src/
    ├── mallard-application/   # orchestration extracted from commands.rs
    │   ├── Cargo.toml
    │   └── src/
    ├── mallard-infrastructure/
    │   ├── Cargo.toml
    │   └── src/               # persistence, stores, providers, codecs,
    │                          # filesystem, platform, activity JSONL
    └── mallard-cli/
        ├── Cargo.toml
        └── src/
            └── main.rs        # `fn main() {}`
```

Except for the explicitly requested CLI placeholder, do not create empty
module trees ahead of moved code. Each crate directory appears when its
first module moves in. Splitting a moved file into smaller modules inside
its new crate is allowed (and expected for the 5,806-line `domain.rs`), as
long as content is otherwise unchanged.

## 7. Architecture and dependency direction

The compile-time graph after extraction:

```text
mallard-domain
       ↑
mallard-infrastructure
       ↑
mallard-application
       ↑
       ├── mallard-desktop (src-tauri, package `mallard`)
       └── mallard-cli (future)
```

This is a **layered** structure: application code calls infrastructure
directly, exactly as today's `commands.rs` calls `persistence.rs` and
`s3_store.rs` directly. The earlier draft's hexagonal shape — application-
owned port traits with infrastructure implementing them — is deliberately
deferred: inverting that dependency requires rewriting call sites and test
scaffolding, which conflicts with behavior preservation. Introduce ports
later, one real boundary at a time, as ordinary reviewed refactors on top
of this layout.

Crate rules (enforced in Phase 5):

- `mallard-domain`: no Tauri, no Tokio, no AWS, no SQLite, no PTY, no
  `dirs`, no filesystem traversal. Holds types, validation, digests, and
  pure comparison logic. (Verified feasible: `domain.rs` has zero Tauri
  references today.)
- `mallard-infrastructure`: may use Tokio, AWS SDK, SQLite, PTY, `notify`,
  `walkdir`, `dirs`, platform APIs. Must not depend on Tauri. Constructed
  from validated paths (`from_home_dir`), never from an `AppHandle`.
- `mallard-application`: orchestration and coordination. Must not depend on
  Tauri. Emits events only through the injected sink callback.
- `src-tauri` (desktop): Tauri builder, managed state, the 71 command
  wrappers, DTO passthrough, event forwarding, dialogs, opener. Each
  wrapper extracts arguments, calls one application function, and returns
  its result — with names, shapes, and errors identical to today.
- `mallard-cli`: empty; no dependencies; excluded from default builds and
  from every installer.

## 8. Current-to-target ownership

Tauri-reference counts are from the baseline and explain why each move is
mechanical.

| Current source | Tauri refs | Destination |
| --- | --- | --- |
| `project_sync/domain.rs` (5,806 lines) | 0 | `mallard-domain`, split into modules, content unchanged |
| `project_sync/persistence.rs` | 1 (`from_app`) | `mallard-infrastructure`; `from_app` helper stays in desktop |
| `project_sync/s3_store.rs` | 0 | `mallard-infrastructure` |
| local store code in `project_sync/bundle_engine.rs` | 0 | `mallard-infrastructure` |
| `project_sync/provider_capture.rs` | 0 | `mallard-infrastructure` |
| `project_sync/chat_history.rs` | 0 | `mallard-infrastructure` |
| `project_sync/conversation_codec.rs` | 0 | `mallard-infrastructure` |
| `project_sync/conversation_materialization.rs` | 0 | `mallard-infrastructure` |
| `project_sync/conversation_sync.rs` | 0 | `mallard-application` (orchestration) / `mallard-infrastructure` (mechanics), split along existing function boundaries |
| `project_sync/codex_app_refresh.rs` | 0 | `mallard-infrastructure` |
| `project_sync/codex_native_fork.rs` | 0 | `mallard-infrastructure` |
| `project_sync/codex_rollout.rs` | 0 | `mallard-infrastructure` |
| `src-tauri/src/codex_binary.rs` | 0 | `mallard-infrastructure` |
| orchestration bodies in `project_sync/commands.rs` (17,997 lines) | 69 `AppHandle`, 66 `emit` | `mallard-application`, with event-sink injection (§3.4) |
| `#[tauri::command]` wrappers, DTO definitions used on the wire | — | stay in `src-tauri` (wrappers) or move with their owners — serde shapes frozen either way |
| JSONL core of `src-tauri/src/activity_log.rs` | 19 total in file | core → `mallard-infrastructure`; commands and emission stay in desktop |
| `src-tauri/src/sync_tests/` | — | stays in `src-tauri`, assertions unchanged |

## 9. Workspace design

Create a virtual workspace at the repository root:

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

# Optimize dependencies only: the wildcard excludes workspace crates, so app
# edits keep opt-level 0 incremental builds while serde_json/sha2/aws-* run
# optimized. Moved here because Cargo ignores profiles in member manifests.
[profile.dev.package."*"]
opt-level = 1
```

Library crates join `members` when their first module moves in; the final
list adds `crates/mallard-domain`, `crates/mallard-application`, and
`crates/mallard-infrastructure`.

Decisions and their reasons:

1. **`resolver = "2"`, explicitly.** A virtual workspace without a
   `resolver` key defaults to resolver 1; the current single-package build
   (edition 2021) uses resolver 2. Writing `"2"` preserves today's
   resolution exactly. Resolver 3 and an explicit `rust-version` are
   deferred — they are policy changes, not restructuring.
2. **Move the lockfile.** `git mv src-tauri/Cargo.lock Cargo.lock`. Cargo
   accepts it at the root and keeps every pin. Do not run a bare
   `cargo update`. The empty CLI adds no dependencies, so the graph is
   unchanged.
3. **Move the dev profile.** Cargo ignores `[profile]` in member manifests
   (with a warning), so `[profile.dev.package."*"]` must move to the root
   to keep its effect. Keep its explanatory comment.
4. **Centralize the version.** `src-tauri/Cargo.toml` inherits
   `version.workspace = true`; `scripts/check-release-version.mjs` is
   updated to read the workspace root (its current regex on
   `src-tauri/Cargo.toml` stops matching), and `tests/release-config.test.mjs`
   is updated to match.
5. **Keep the package name `mallard`.** The Cargo package name determines
   the binary name, which feeds bundle contents. Renaming to
   `mallard-desktop` (with `mainBinaryName` pinning and bundle
   verification) is deferred work.
6. **Target directory moves to the root.** Consequences, all in scope:
   - `scripts/tauri-build.mjs` resolves the DMG under
     `src-tauri/target/...` today; derive the target directory from
     `cargo metadata` (or the workspace root) instead.
   - `.github/workflows/release.yml` line 37 pins the rust-cache workspace
     as `"./src-tauri -> target"`; update it for the root workspace.
   - The root `.gitignore` gains a `/target` entry
     (`src-tauri/.gitignore` only covers the old location).
7. **`npm run test:backend-integration` keeps working unchanged** —
   `cargo test --manifest-path src-tauri/Cargo.toml --lib` resolves the
   workspace from the member manifest. Optionally modernize to
   `cargo test -p mallard --lib` in the same change.
8. **`default-members = ["src-tauri"]`** keeps bare `cargo` commands and
   the CLI exclusion behavior: the placeholder builds only when named or
   under `--workspace`.

## 10. Implementation phases

Every phase ends with the same gate:

```text
npm run build
npm run test:frontend-integration
npm run test:release-config
npm run check:release-version
npm run test:backend-integration
cargo check --workspace
cargo test --workspace
git diff --stat -- src/        # must be empty
```

### Phase 0 — Freeze the baseline

Actions:

1. record the baseline commit and confirm every suite above is green on it;
2. inventory the frozen contracts: the 71 command names, the emitted event
   names, the `~/.mallard` document set, and the portable key layout
   (cross-check against `CURRENT_METADATA_LAYOUT.md`);
3. add a freeze test in `src-tauri` asserting the exact registered command
   list, so an accidental registration change fails loudly; and
4. snapshot a baseline `~/.mallard` fixture and a populated test
   destination for the Phase 6 compatibility check.

Deliverables: the inventory (a short appendix to this document or a sibling
note), the registration freeze test, and the saved fixtures.

Acceptance: baseline suites green; freeze test passes; fixtures captured.

### Phase 1 — Create the workspace and empty CLI

Files:

- new root `Cargo.toml`; root `Cargo.lock` (moved);
- updated `src-tauri/Cargo.toml` (inherit workspace metadata; drop the
  member-level profile);
- new `crates/mallard-cli/Cargo.toml` and `src/main.rs` (`fn main() {}`);
- updated `scripts/check-release-version.mjs`;
- updated `scripts/tauri-build.mjs`;
- updated `tests/release-config.test.mjs`;
- updated `.github/workflows/release.yml` (rust-cache workspace path);
- updated root `.gitignore` (`/target`).

Acceptance: the phase gate, plus:

- `cargo run -p mallard-cli` exits 0 with no output;
- the desktop application boots and behaves identically;
- `cargo tree -p mallard` shows an unchanged dependency graph;
- `MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build` passes and the DMG
  contains `Mallard.app` with no `mallard-cli` binary.

### Phase 2 — Extract `mallard-domain`

Move `project_sync/domain.rs` into `crates/mallard-domain`, splitting into
modules (identity, config, project, storage, bundle, restore, conversation,
setup) with content otherwise unchanged. Its unit tests move with it. Leave
`pub use mallard_domain as domain;` (or narrower re-exports) at the old
path so `commands.rs` and the rest compile without edits.

Acceptance: the phase gate, plus `cargo tree -p mallard-domain` shows no
Tauri, Tokio, AWS, SQLite, or PTY dependency.

### Phase 3 — Extract `mallard-infrastructure`

Move, in one or more PRs grouped by family:

1. `persistence.rs` (leaving the `from_app` desktop helper), `s3_store.rs`,
   and the local store code from `bundle_engine.rs`;
2. `provider_capture.rs`, `chat_history.rs`, `conversation_codec.rs`,
   `conversation_materialization.rs`, `codex_rollout.rs`,
   `codex_app_refresh.rs`, `codex_native_fork.rs`, `codex_binary.rs`; and
3. the JSONL core of `activity_log.rs`.

Unit tests travel with their modules; `sync_tests` stays in `src-tauri` and
keeps passing without assertion changes. Old paths forward through shims.

Acceptance: the phase gate, plus `cargo tree -p mallard-infrastructure`
shows no Tauri dependency.

### Phase 4 — Extract `mallard-application`

For each command family (config, storage, projects/links/profiles/bindings,
sync push/pull, conversations, history, setup, activity):

1. move the orchestration body of each `#[tauri::command]` function into an
   application function taking plain arguments plus, where the flow emits
   events, an event-sink callback;
2. keep the `#[tauri::command]` wrapper in `src-tauri` with an identical
   name, signature, DTO set, and error mapping, forwarding `app.emit`
   through the sink so event names, payloads, and ordering are unchanged;
3. change nothing else — the wrapper-plus-body pair must be a pure split of
   today's function.

This is the largest phase; split it into one PR per family. The
registration freeze test and `sync_tests` hold the contract still while
17,997 lines relocate.

Acceptance per family PR: the phase gate, plus the registration freeze test
unchanged and `cargo tree -p mallard-application` free of Tauri.

### Phase 5 — Remove shims and tidy the desktop crate

1. delete every temporary re-export shim recorded since Phase 2 and update
   remaining imports to the real crate paths;
2. split the residual desktop command wrappers into `src/commands/` family
   files (a file move only — the registration list is byte-identical);
3. delete the now-empty `project_sync` module shell;
4. review the remaining `pub` surface of each crate and reduce what nothing
   uses; and
5. add the boundary checks: fail CI (or a workspace test) if Tauri appears
   in `cargo tree` for the three library crates, or if `src-tauri` is
   imported by another package.

Acceptance: the phase gate; no shims remain (`grep` for the shim inventory
comes back empty); boundary checks active and green.

### Phase 6 — Compatibility and release verification

1. run the restructured build against the Phase 0 `~/.mallard` fixture and
   populated destination: existing projects, links, profiles, bindings,
   history, push, pull, conversation, and recovery flows all work without
   any reset or warning;
2. verify the build wrote no new-shape artifacts: state documents and cloud
   objects remain schema 1 in the same locations;
3. **downgrade check:** run the baseline 0.1.2 build against the same state
   afterwards and confirm it works — proof the restructure is invisible;
4. `MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build`; inspect the DMG:
   identifier, product name, executable, signing, and no `mallard-cli`;
5. on Windows, preserve the status quo: `cargo check --workspace` and
   `cargo test --workspace` on a Windows machine if available, as today
   there is no Windows CI or release automation (adding it is deferred
   work, not an acceptance bar here); and
6. update `RELEASING.md` and contributor documentation for the workspace
   layout (root lockfile, root target, `-p` flags).

Acceptance: all of the above, plus the full phase gate.

## 11. Pull request boundaries

1. Baseline freeze: inventory, registration freeze test, fixtures.
2. Workspace, root lockfile, empty CLI, script/CI/gitignore updates.
3. Domain extraction.
4. Infrastructure extraction (1–3 PRs by family).
5. Application extraction (one PR per command family).
6. Shim removal, desktop tidy-up, boundary checks.
7. Compatibility verification and documentation.

Every PR merges with the full phase gate green and an empty `src/` diff.
Because each PR is behavior-neutral, `main` remains releasable throughout —
there is no integration branch, feature flag, or transition state.

## 12. Verification matrix

| Area | Required verification |
| --- | --- |
| Workspace | `cargo check/test --workspace`, unchanged `cargo tree -p mallard`, moved (not regenerated) lockfile diff |
| IPC contract | registration freeze test; `sync_tests` unchanged; empty `src/` diff |
| Events | `sync_tests` and frontend integration tests against identical event names/payloads |
| Local state | Phase 6 fixture run + downgrade check; schema-1 documents byte-stable |
| Bundle storage | existing local and stub-S3 tests unchanged; Phase 6 populated-destination run |
| Activity log | existing tests unchanged; JSONL files identical in place and shape |
| Crate boundaries | `cargo tree` checks: no Tauri in domain/application/infrastructure; no cross-import of `src-tauri` |
| Release | `check:release-version`, `test:release-config`, DMG verify + inspection, CLI absent |

Final verification is the Phase 10-equivalent single block:

```text
npm run build
npm run test:frontend-integration
npm run test:release-config
npm run check:release-version
npm run test:backend-integration
cargo check --workspace
cargo test --workspace
MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build
```

## 13. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| A move silently changes a serde shape | Moves-only rule (§3.1); serde attributes untouchable; existing integration tests exercise the wire shapes |
| Event ordering shifts when orchestration extracts | Sink callback forwards synchronously at the original call sites; tests assert existing events |
| Lockfile regeneration changes dependency behavior | `git mv` the lockfile; review the (empty) dependency diff; `cargo tree -p mallard` compared before/after |
| Virtual-workspace resolver default (v1) changes builds | `resolver = "2"` written explicitly |
| Dev profile silently stops applying | Profile moved to root in the same PR that creates the workspace |
| Release scripts/CI break on the root target | `tauri-build.mjs`, `check-release-version.mjs`, release-config tests, and the rust-cache path all updated in Phase 1 and gated by the release checks |
| Temporary shims become permanent | Shim inventory; Phase 5 deletes them; grep gate in acceptance |
| Refactor scope creeps into redesign | §3.1/§3.3 rules; PR review rejects non-move diffs; hexagonal work explicitly deferred |
| Concurrent feature plans collide | §3.5/§5 sequencing: this plan lands first or entirely after; no interleaving |
| A hidden Tauri coupling blocks a move | Verified counts (§8) say only `persistence.rs`/`commands.rs`/`activity_log.rs`; if another appears, wrap it in the desktop crate (§3.4), never rewrite |

## 14. Definition of done

The reorganization is complete when:

- the repository has one root Cargo workspace, one moved lockfile, and one
  root target directory;
- `mallard-domain`, `mallard-application`, and `mallard-infrastructure`
  hold the relocated code, with the dependency direction of §7 enforced by
  checks;
- `src-tauri` is a thin adapter: builder, managed state, and 71 unchanged
  command wrappers that each delegate to one application function;
- `mallard-cli` is a separate, empty, compilable package excluded from
  default builds and absent from installers;
- every suite that was green at the baseline is green, with no behavioral
  test edits;
- `src/` has zero changes across the entire plan;
- a pre-restructure `~/.mallard` and populated storage destination work
  unchanged, and the baseline build still works against them afterwards;
- no temporary re-export shim remains; and
- release documentation reflects the workspace layout — and tells users
  nothing, because nothing about using Mallard changed.

## 15. Deferred work

Separate plans, in no required order, for:

1. hexagonal ports and fake-port test scaffolding, one real boundary at a
   time;
2. renaming the desktop package/library (`mallard-desktop`,
   `mallard_desktop_lib`) with `mainBinaryName` pinning and bundle
   verification;
3. CLI command design, Clap, packaging, and `PATH` installation;
4. agent-assisted R2 connection and provisioning;
5. Windows CI and release automation;
6. resolver 3, an explicit `rust-version`, and edition 2024 adoption;
7. dependency upgrades; and
8. optional React component reorganization.

Any future schema change gets its own plan with its own compatibility
story. This plan neither needs nor provides one.

## 16. Industry references

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html)
- [Tauri project structure](https://v2.tauri.app/start/project-structure/)
- [Working Effectively with Legacy Code — characterization tests](https://www.oreilly.com/library/view/working-effectively-with/0131177052/)
- [Refactoring: Improving the Design of Existing Code](https://martinfowler.com/books/refactoring.html)
