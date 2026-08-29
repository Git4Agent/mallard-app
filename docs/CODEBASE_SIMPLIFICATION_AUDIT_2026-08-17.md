# Mallard codebase simplification audit

**Audit date:** 2026-08-17  
**Baseline:** `dae5bd64b604303aae6bf0c2c20dee1de4ee9904` plus the existing staged,
unstaged, and untracked working-tree changes  
**Scope:** active Rust/Tauri and React/TypeScript paths, IPC contracts,
dependencies, tests, build output, and repository artifacts  
**Purpose:** identify overgrown or needlessly layered code, obsolete
compatibility support, and dead production surface. This is not a security or
correctness audit.

## Executive assessment

Mallard's main domain boundaries are justified: the provider registry has two
production adapters, the object-store abstraction has local and S3
implementations, and project sync and the Library intentionally use separate
protocol engines. The clearest simplification opportunities are not those
boundaries. They are unfinished cutovers and surfaces that compilers cannot see
as dead because they are public, macro-registered, or explicitly suppressed.

The highest-value findings are:

1. **22 of 94 registered Tauri commands have no live product caller.** One is
   the pre-cutover direct project registration path, and a regression test
   freezes the obsolete surface.
2. **The normal test commands conceal contract drift.** The aggregate npm suite
   skips 279 extracted-crate unit tests, while 204 frontend tests pass despite
   48 substantive TypeScript errors in five test files because esbuild strips
   types.
3. **Every Activity refresh reads and parses up to 10,001 Git commits twice.**
   The first result is stored in fields marked `allow(dead_code)` and discarded.
4. **The Rust crate split still exposes an explicitly temporary compatibility
   facade**, and Cargo manifests still duplicate dependencies from before the
   move.
5. **Two live frontend compatibility vocabularies outlast the clean cutover:**
   an inert serialized storage field and mappings for resource kinds the
   current Rust schema cannot emit.

Normal production checks are green: TypeScript production source builds,
workspace Rust checks and Clippy pass, all workspace tests pass, and the
frontend integration suite passes. That is compatible with these findings:
most of the dead surface is exported or registered, and several dead-code
warnings are deliberately disabled.

## Findings register

| ID | Priority | Category | Finding |
| --- | --- | --- | --- |
| SIM-001 | P1 | Dead code / legacy API | 22 of 94 Tauri commands have no live product caller |
| SIM-002 | P1 | Detection gap | Current test gates hide Rust coverage and TypeScript fixture drift |
| SIM-003 | P1 | Overengineering / performance | Activity executes the same large Git-history query twice |
| SIM-004 | P1 | Legacy support | Temporary Rust reorganization shims remain in active use |
| SIM-005 | P2 | Legacy schema | Inert storage exclusions and old resource-kind fallbacks remain serialized or executable |
| SIM-006 | P2 | Overengineering / dead CSS | Two always-loaded stylesheets retain 434 known duplicate declarations |
| SIM-007 | P2 | Complexity concentration | The main frontend and backend controllers have become change hotspots |
| SIM-008 | P2 | Dead dependencies | JavaScript and Cargo manifests retain unused or wrongly owned dependencies |
| SIM-009 | P2 | Dead shipped assets | Prototype pages make up 38.5% of the built frontend payload |
| SIM-010 | P3 | Dead code | Test-only helpers and exports are compiled into production |
| SIM-011 | P3 | Repository residue | Historical verification and documentation are not clearly gated or navigable |

Priority is remediation order, not defect severity. P1 items should be handled
before adding more commands or workflows; P2 items belong in the next
maintainability pass; P3 items are safe mechanical cleanup after the relevant
tests are fixed.

## Detailed findings

### SIM-001 — 22 registered Tauri commands have no live product caller

The handler registers 94 commands in
[`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs#L39). Comparing that list with
property accesses from the production React import closure found 22 commands
that cannot be reached from the shipped interface.

Seven have no frontend `invoke` wrapper at all:

- `get_local_project`
- `save_bundle_recipe`
- `remove_project_binding`
- `list_project_materializations`
- `discard_restore_plan`
- `get_bundle_status`
- `verify_bundle_objects`

Another 15 have wrappers in
[`src/components/project-sync/api.ts`](../src/components/project-sync/api.ts#L112),
but those wrapper properties are never referenced by production or frontend
tests:

- `rename_provider_profile` and `remove_provider_profile`
- `register_local_project`
- `update_host_display_name` and `rotate_host_identity`
- `get_project_origin_tokens`, `list_origin_host_aliases`,
  `save_origin_host_alias`, and `resolve_origin_host_label`
- `list_remote_bundles` and `find_remote_bundle_matches`
- `get_project_binding`
- `get_setup_draft`, `update_setup_draft`, and `inspect_setup_draft`

The most consequential example is `register_local_project`. It remains a
callable command at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L1458), but the
current product uses the transactionally complete setup-draft finalizer. The
old helper creates a registration without the current root binding and
provider setup at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L11952).

The stale surface is then protected by
[`registration_freeze.rs`](../src-tauri/src/sync_tests/registration_freeze.rs#L1),
which freezes names from the workspace reorganization rather than proving that
each handler has a live client. Its comment also says 95 commands while both
the array and registration contain 94.

The dead `get_bundle_status` path carries its own Rust and TypeScript DTOs:
`BundleResourceStatus` and `ResourceStatusReport` at
[`src/types.ts`](../src/types.ts#L613), plus a full provider discovery/capture
implementation at
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L6765).

**Impact:** obsolete mutation paths remain callable from the WebView, the IPC
contract is 23% larger than the product uses, and DTO/helper/test maintenance
continues for workflows the current UI cannot start.

**Recommendation:** replace the historical freeze with a parity test that
compares production API calls with registered handlers. Remove the 22 unused
registrations, wrappers, and command-only DTOs. Preserve lower-level helpers
that current commands still use; command reachability does not imply that
similarly named storage or label helpers are dead.

### SIM-002 — Current test gates hide contract drift

The package script named `test:backend-integration` runs only the Tauri package
at [`package.json`](../package.json#L14). It executes 177 tests but omits the
unit tests in the extracted libraries:

| Crate | Omitted library tests |
| --- | ---: |
| `mallard-domain` | 42 |
| `mallard-infrastructure` | 211 |
| `mallard-application` | 26 |
| **Total** | **279** |

The frontend has the inverse problem. The 12 integration files are bundled by
esbuild at
[`run-frontend-integration-tests.mjs`](../scripts/run-frontend-integration-tests.mjs#L25),
so types are stripped rather than checked. `tsconfig.json` includes only `src`
at [`tsconfig.json`](../tsconfig.json#L23).

The frontend suite passes all 204 tests, but a direct strict TypeScript check
of those same files reports 83 errors: 35 missing Node declaration errors and
48 substantive contract errors across five test files. Examples include the
retired `project_memory` resource kind and removed `scope` property at
[`resource-inventory.integration.test.tsx`](../tests/frontend/resource-inventory.integration.test.tsx#L36),
plus fixtures missing required provider/access fields.

This explains how legacy test data can keep compatibility branches looking
useful even though it no longer satisfies current DTOs.

**Recommendation:** change the Rust gate to `cargo test --workspace --lib`;
keep the CLI integration suite separately. Add `@types/node`, a strict
`tsconfig.tests.json`, and a typecheck before esbuild. Automatically discover
sorted `tests/frontend/*.integration.test.tsx` entries instead of maintaining
the manual list. Add the same aggregate to pull-request CI; the current sole
workflow triggers only for release tags.

### SIM-003 — Activity loads the same Git history twice

`get_project_activity` calls
`discover_git_history_with_recorded_branches` at
[`project_activity.rs`](../crates/mallard-infrastructure/src/project_activity.rs#L898).
That helper runs `git log`, loads up to 10,001 commits, calculates a page and a
cursor at [line 1300](../crates/mallard-infrastructure/src/project_activity.rs#L1300),
and returns both in fields explicitly marked `allow(dead_code)` at
[line 1224](../crates/mallard-infrastructure/src/project_activity.rs#L1224).

The caller ignores both fields, immediately runs the same 10,001-commit load
again at [line 908](../crates/mallard-infrastructure/src/project_activity.rs#L908),
and hardcodes `next_cursor: None` in the response at
[line 943](../crates/mallard-infrastructure/src/project_activity.rs#L943).
The helper's `before_commit` and `limit` parameters have no second caller.

Two nearby functions, `cwd_belongs_to_project` and `value_alias`, also have no
references and are retained only through `allow(dead_code)` at
[lines 1572-1587](../crates/mallard-infrastructure/src/project_activity.rs#L1572).

**Impact:** every Activity refresh starts two Git subprocesses and performs
two parses/allocations over potentially 10,001 commits. The abandoned
pagination model also survives in Rust and TypeScript DTOs.

**Recommendation:** make discovery return branch metadata only, perform one
commit load in the caller, and remove the dead fields, parameters, helpers,
and response cursor if pagination is not part of the current product.

### SIM-004 — Temporary crate-compatibility shims remain active

[`project_sync/mod.rs`](../src-tauri/src/project_sync/mod.rs#L11) explicitly
labels 18 `mallard-domain` and `mallard-infrastructure` re-exports as temporary
reorganization shims scheduled for deletion in Phase 5. Production commands,
watchers, and integration tests still use the old namespaces; direct tracing
found roughly 146 old-path references.

[`activity_log.rs`](../src-tauri/src/activity_log.rs#L1) has a related wildcard
re-export so existing `crate::activity_log::*` paths continue to compile.
The governing reorganization plan says completion requires deleting every
shim at
[`PLAN_PROJECT_STRUCTURE_REORGANIZATION.md`](./PLAN_PROJECT_STRUCTURE_REORGANIZATION.md#L483).

**Impact:** core types have two valid namespaces, ownership is obscured, and
the desktop crate remains coupled to historical module paths. This is a
literal compatibility layer in a repository whose current rule is to remove
obsolete paths.

**Recommendation:** mechanically update production and tests to import
`mallard_domain` and `mallard_infrastructure::*` directly, replace the activity
wildcard with narrow imports, then delete the facade in one cutover.

### SIM-005 — Two clean-cutover vocabularies remain

#### Inert storage exclusions

`StorageConfig` still serializes `included_default_exclusions` at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L700).
Production code initializes it to an empty vector, copies it, compares it, or
checks its maximum length; no capture or selection behavior reads its values.
Current project-file selection instead uses link-scoped project-content
preferences in
[`commands.rs`](../src-tauri/src/project_sync/commands.rs#L3254).

The field therefore survives in Rust and TypeScript DTOs, constructors, tests,
and equality checks without affecting the product.

#### Old resource-kind fallbacks

The frontend model says provider-prefixed kinds arrive from “older recipes”
and maps `codex_task`, `claude_session`, `task`, `session`, `memory`, and
`project_memory` at
[`model.ts`](../src/components/project-sync/model.ts#L35). The current Rust
`ResourceKind` is a strict five-variant enum at
[`mallard-domain/src/lib.rs`](../crates/mallard-domain/src/lib.rs#L748), and the
matching TypeScript union is equally closed at
[`src/types.ts`](../src/types.ts#L124). No current backend path emits those old
kind strings. One untyped test fixture still uses `project_memory`, which is
one of the errors exposed by SIM-002.

**Impact:** dead schema and compatibility vocabulary make current contracts
appear wider than they are, force boilerplate through constructors, and can
mask stale fixtures.

**Recommendation:** remove `included_default_exclusions` end to end under the
clean-cutover policy. Make kind/category label maps exhaustive over the current
union, remove old keys and raw-string fallbacks, and update test fixtures to
use canonical `kind`, `provider`, and `provider_resource_type` fields.

### SIM-006 — The CSS architecture knowingly retains dead overrides

[`App.tsx`](../src/App.tsx#L5) unconditionally loads `App.css` followed by
`RetroTheme.css`. Together they contain 11,836 lines and produce a 206,820-byte
CSS asset. `RetroTheme.css` defines 296 class names; 252 also occur in
`App.css`.

More importantly, the CSS test itself states that an equal
selector/property declaration in the first file is dead weight because the
later file wins. It permits a ratchet of 435 such collisions at
[`css-hygiene.integration.test.tsx`](../tests/frontend/css-hygiene.integration.test.tsx#L96).
The current count is 434.

Production-closure analysis also found two orphan Retro selectors:
`.sync-history-storage-only` at
[`RetroTheme.css`](../src/RetroTheme.css#L2004) and `.sync-pull-outcome` at
[line 2395](../src/RetroTheme.css#L2395). No production component renders
either class; source-assertion tests currently pin the former at
[`project-sync-cutover.integration.test.tsx`](../tests/frontend/project-sync-cutover.integration.test.tsx#L411).

**Impact:** source order is part of the design contract, 434 earlier
declarations are knowingly shadowed, stale selectors can be protected by tests,
and ordinary styling changes require reasoning across two global files.

**Recommendation:** keep palette/type tokens in a small theme file, but move
the final component declarations into one owner and delete the overridden
earlier properties. Make the orphan check operate on the production import
closure and on both stylesheets. Ratchet duplicate selector/property pairs to
zero rather than 435.

### SIM-007 — Orchestration is concentrated in two oversized controllers

The complexity problem is concentration rather than too many abstractions:

- [`ProjectSync.tsx`](../src/components/project-sync/ProjectSync.tsx#L316) is
  3,575 lines with 75 state cells, 19 effects, and 15 refs. Its host reset
  manually clears roughly 40 independent values at
  [line 872](../src/components/project-sync/ProjectSync.tsx#L872).
- [`project_sync/commands.rs`](../src-tauri/src/project_sync/commands.rs#L1) is
  21,996 lines. Production code occupies about 14,984 lines before the main
  test module; the file contains 69 Tauri commands, roughly 275 functions, and
  14 `too_many_arguments` suppressions.
- Supporting pages are also large: `ProjectLinksWorkspace.tsx` is 2,260 lines
  and `ProjectActivityPage.tsx` is 2,005 lines.

The frontend controller owns shell loading, navigation, storage, catalog,
activity logs, setup, Push, Pull, resize behavior, and multiple request-epoch
schemes. The backend controller mixes the IPC wrappers with setup, inventory,
comparison, publication, restore, path repair, and many DTO definitions.

**Impact:** unrelated workflows share state and request tokens, invalid state
combinations are representable, and command/API removal requires editing the
highest-churn files.

**Recommendation:** split by current product workflow, not by speculative
framework. Extract focused shell/log/catalog/Push/Pull hooks or reducers with
discriminated workflow states. Split Tauri commands into capability modules
behind thin registered wrappers. Keep the existing domain, infrastructure,
and review primitives; they are not the source of this concentration.

### SIM-008 — Dependency manifests retain dead and inverted ownership

#### JavaScript

`react-icons` is declared at [`package.json`](../package.json#L25), imported by
no source or test, and occupies about 85 MB in `node_modules`; production icons
come from the local `Icons.tsx` catalog. Conversely, the frontend test runner
directly imports `esbuild` at
[`run-frontend-integration-tests.mjs`](../scripts/run-frontend-integration-tests.mjs#L6)
without declaring it. It currently succeeds only because Vite's transitive
copy is hoisted.

#### Rust

Direct crate-name tracing found these stale manifest entries:

- `src-tauri/Cargo.toml`: `toml`, `aws-sdk-s3`,
  `aws-smithy-http-client`, `rusqlite`, `plist`, and `windows-sys` have no
  desktop-crate references. `aws-smithy-types` is used only by the S3 test
  stub.
- `crates/mallard-infrastructure/Cargo.toml`: `toml`, `hmac`, `rusqlite`, and
  `notify` have no infrastructure references.

AWS, `plist`, and `windows-sys` are genuinely used in the infrastructure
crate; the problem is their duplicate direct declaration in the Tauri crate.
`rusqlite` appears nowhere in production source, yet both manifests request
the bundled SQLite build.

**Recommendation:** remove `react-icons`, declare `esbuild` as a dev
dependency, remove the confirmed unused Cargo entries, and move test-only
Smithy types to dev-dependencies. Re-run all workspace targets on macOS and
Windows after the manifest cut.

### SIM-009 — Prototypes and starter assets ship with the application

Vite copies `public/` into `dist`, and Tauri packages that directory. The
following tracked development pages are therefore part of the built app:

| Shipped file | Bytes |
| --- | ---: |
| `public/session-header-mockup.html` | 472,573 |
| `public/storage-menu-mockup.html` | 110,022 |
| `public/storage-menu-a.html` | 68,307 |
| **Total** | **650,902** |

That is 38.5% of the 1,691,372-byte frontend build produced during this audit.
The public session-header file is byte-identical to the root
`session-header-mockup.html`. Unreferenced Vite/Tauri starter SVGs and
`mallard-logo.png` add more shipped debris.

The history, profile, and current catalog labs under `src` are not dead design
tools: root HTML files are real Vite development entries. They are nevertheless
outside the production `src/main.tsx` import closure, total about 4,195 lines,
and are still compiled by the broad `tsconfig` include.

**Recommendation:** move retained labs and screenshots under
`docs/prototypes/` or give them a dedicated development config. Remove public
copies and starter assets from the production public directory. Do not delete
the actively edited catalog lab merely because it is not a production entry.

### SIM-010 — Test support is compiled into production

Several items silence dead-code warnings instead of using compilation gates:

- `capture_recipe` is used only by bundle-engine tests but has
  `cfg_attr(not(test), allow(dead_code))` at
  [`provider_capture.rs`](../crates/mallard-infrastructure/src/provider_capture.rs#L677).
- no-progress restore wrappers are used only by the commands test module and
  similarly suppress production dead-code warnings at
  [`commands.rs`](../src-tauri/src/project_sync/commands.rs#L9429) and
  [line 9614](../src-tauri/src/project_sync/commands.rs#L9614).
- the deterministic `ManualScanner` is selected with runtime-constant
  `cfg!(test)` at
  [`codex_watch_service.rs`](../src-tauri/src/project_sync/codex_watch_service.rs#L258),
  so its implementation is still compiled into production.
- nine frontend runtime values are exported even though every use is in their
  defining module, including `KIND_LABELS`, `APPLY_POLICY_LABELS`, Pull-flow
  internals, and storage-action label helpers.

**Recommendation:** delete wholly unused helpers; use `#[cfg(test)]` or the
existing `test-support` feature for genuine cross-crate test helpers; make
module-local frontend values non-exported. Keep test scaffolding that is shared
across suites, but compile it only for tests.

### SIM-011 — Repository verification residue is ambiguous

**Resolution (2026-08-29):** the 1,058-line conflict model and its report were
deleted after codec and relation drift was confirmed. The current Rust golden
vectors and relation-matrix tests are authoritative. A separate bounded model
requires a named owner and an aggregate test gate before it can return.

A repository link scan also found 62 broken internal links across six
documents, mostly references to pre-reorganization
`src-tauri/src/project_sync/*` paths. Update links in still-authoritative docs;
move snapshot documents under `docs/archive/` with a historical banner. A
small active-doc link checker would prevent another move from silently
repeating the drift.

## Legacy support that was reviewed but should not be removed blindly

- There is no active migration or tolerant reader for pre-cutover Mallard
  machine or bundle schemas. Current schema validation rejects unsupported
  versions.
- Codex history deliberately understands provider-native older rollout forms:
  field aliases at
  [`providers/codex/history.rs`](../crates/mallard-infrastructure/src/providers/codex/history.rs#L196),
  a canonical-versus-legacy timeline path at
  [line 1818](../crates/mallard-infrastructure/src/providers/codex/history.rs#L1818),
  and a frozen Codex 0.147 projection adapter at
  [`codex_paginated_projection.rs`](../crates/mallard-infrastructure/src/codex_paginated_projection.rs#L1).
  This is external provider-format support, not an old Mallard schema path.
  Give it an explicit supported-Codex-data policy before deciding to remove it.
- Codex pet v1 and v2 are two current resource formats. The implementation
  explicitly rejects unknown version markers and does not upgrade or downgrade
  them.
- `docs/archive/**`, `docs/PLAN_*`, and `docs/superpowers/**` are design history,
  not production code. They need navigation and link hygiene, not deletion by
  reachability analysis.

## Candidates investigated and ruled out

- `ObjectStore` / `ConfiguredObjectStore`: justified by local and S3
  transports, shared protocol engines, and test doubles.
- `ProviderRegistry`: used by Codex and Claude and exercised with a third gated
  provider; it enforces the current provider-neutral boundary.
- Separate project-bundle and Library engines: justified by the explicit
  bounded-context and key-namespace separation.
- `SessionDetailPage`: live through a dynamic import, not dead merely because
  it is absent from the initial bundle.
- Root `catalog-live.html`, `history-mockup.html`, and
  `profile-prototypes.html`: live development entries. Only their public copies
  or starter assets are shipped accidentally.
- Release verification scripts: all are wired into packaging or release tests.
- Removed `LegacyApp`, `SyncPanel`, `FilesWorkspace`, and native Codex fork
  executor: no executable path remains.

## Recommended sequence

1. **Fix the detection gates:** typecheck frontend tests, run all workspace
   libraries in the aggregate, declare esbuild directly, and add pre-merge CI.
2. **Delete proven dead contract surface:** replace the registration freeze
   with parity, remove the 22 unused commands and their command-only DTOs, and
   remove dead helpers/test-only production code.
3. **Complete the cutovers:** remove the reorganization facade, stale Cargo
   entries, `included_default_exclusions`, and old resource-kind mappings.
4. **Remove measured waste:** run one Activity Git query and move public
   prototypes/starter assets out of the packaged directory.
5. **Reduce structural complexity:** consolidate CSS ownership, then split the
   two large controllers by current workflow while keeping the existing domain
   boundaries.

Each step can land independently on a working product. The first two should
make later deletions safer and easier to verify.

## Verification ledger

| Check | Result |
| --- | --- |
| `npm run build` | Passed; Vite warned about a 535.82 kB main chunk |
| `npm run test:frontend-integration` | Passed; 204 tests |
| strict TypeScript check of all frontend tests | Failed; 48 substantive errors in 5 files, plus 35 missing Node declaration errors |
| `cargo check --workspace --all-targets` | Passed |
| workspace Clippy with `dead_code` and `unused` warnings | Passed |
| `cargo test --workspace --all-targets` | Passed |
| `npm run test:release-config` | Passed; 5 tests |
| `git diff --check` | Passed |

No application source was changed during this audit. The only new repository
artifact is this report.
