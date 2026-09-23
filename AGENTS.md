
# Mallard Repository Guide
- Do not preserve backward compatibility. Remove obsolete paths instead of
adding compatibility layers, fallbacks, or migrations.

- Choose the simplest implementation that fully meets the current
requirements. Avoid speculative abstractions, configuration, and
indirection.

- Grow the system in layers. Start from the smallest version that works end
to end, and add each new capability on top of a product that already
works. Never trade a working product for unfinished complexity.
-Keep components modular and concerns clearly separated.

- Prefer established, well-maintained libraries when they reduce overall
complexity or improve reliability. Do not reimplement common
functionality without a clear reason.

- Lean on the dependencies already in the project before writing your own
implementation or adding packages. Do not assume a library lacks a
capability without checking its documentation and types.

- Make architectural decisions for the long term. Do not accept a stopgap
that only works for now and is meant to be replaced


## Product and architecture

Mallard is a review-first Tauri 2 desktop application that makes project-scoped
Codex and Claude sessions and provider-home state — memory and session state
under any bound `CODEX_HOME`/`CLAUDE_CONFIG_DIR` — portable between
machines. It never captures agent configuration from inside the project
directory; for non-Git projects, selected ordinary files can additionally
travel as explicitly reviewed project content. The active stack is Rust 2021
in the Tauri backend and React 19, strict TypeScript, and Vite 7 in the
frontend.

The current implementation has one supported product path:

- machine-local state uses schema 1 under `~/.mallard`;
- portable bundle manifests use schema 1;
- `ProjectSync` is the only application interface; and
- local-folder storage and S3-compatible/R2 storage share the same bundle
  model.

Do not reintroduce removed legacy UI, pre-cutover commands, or compatibility paths.
The integration tests intentionally enforce this cutover.

## Source map

- `src/main.tsx` boots React; `src/App.tsx` mounts the project-sync workspace.
- `src/components/project-sync/` contains setup, history, inventory,
  Push/Pull review, storage, project-file, and status UI.
- `src/components/sync-review/` holds the resource-neutral review primitives
  (tabs, selection bar, outcome renderer) shared by project sync and the
  storage Library; it imports no feature DTOs and never calls Tauri.
- `src/components/storage-catalog/` is the storage-scoped Library UI (Codex
  and Claude skills, plugin intent, and Codex v1/v2 pets) with its own
  `api.ts`, `model.ts`, and `types.ts`; it never imports project-sync
  components, models, or DTOs.
- `src/components/project-sync/api.ts` is the typed Tauri `invoke` boundary.
- `src/components/project-sync/model.ts`, `pullReviewModel.ts`, and
  `pullReviewFlow.ts` contain reusable frontend behavior.
- `src/types.ts` mirrors serialized Rust DTOs. Keep it synchronized with Rust
  response and request shapes.
- `src/App.css` contains the shared desktop-shell and project-sync styling.
- `src-tauri/src/lib.rs` registers the production Tauri command surface.
- `src-tauri/src/project_sync/domain.rs` owns IDs, schemas, validation,
  manifests, plans, actions, and receipts. Keep it free of Tauri and filesystem
  concerns.
- `provider_capture.rs` owns provider discovery, filters, stable resource
  identity, and capture. Keep it free of Tauri and cloud transport concerns.
- `bundle_engine.rs` owns immutable publication, integrity checks, restore
  planning, backups, and apply behavior.
- `commands.rs` orchestrates Tauri commands and cross-document transactions.
- `persistence.rs` owns bounded, atomic machine-local JSON persistence.
- `storage_core/` (in `crates/mallard-infrastructure`) is the neutral
  transport shared by both storage protocols: the closed `.mallard/` object
  keys with separate repository and catalog constructors, the object-store
  trait, the local-folder and S3/R2 stores, and the storage marker.
- `src-tauri/src/storage_catalog/` owns the storage-scoped catalog:
  Tauri-free domain and plans, machine-local persistence, profile and skill
  root resolution, provider adapters, inventory, the remote catalog engine,
  reconciliation/planning, journaled apply, and its command surface. It never
  imports `project_sync` modules or project/thread domain types.
- `chat_history.rs` parses Codex history and maps sessions to Git history.
- `activity_log.rs` owns sanitized JSONL activity history and retention.
- `tests/frontend/` contains Node test-runner integration tests bundled with
  esbuild. Rust unit and integration tests live beside their modules and under
  `src-tauri/src/sync_tests/`.

`README.md` and the active code describe current behavior. Files under
`docs/PLAN_*` and `docs/superpowers/` are design history unless a task
explicitly names them. When documentation and implementation disagree, verify
the current schemas and command surface in code before editing.

## Development commands

- `npm install` installs frontend and Tauri CLI dependencies.
- `npm run dev` starts Vite only on port 1420. Native Tauri commands are not
  available in this mode.
- `npm run tauri dev` runs the complete desktop application.
- `npm run build` performs strict TypeScript checks and builds the frontend.
- `npm run test:frontend-integration` runs the frontend integration suite.
- `npm run test:release-config` checks packaged release configuration.
- `npm run test:backend-integration` runs the Rust library test suite.
- `npm run test:integration` runs all committed integration suites.
- `cargo check --manifest-path src-tauri/Cargo.toml` checks the backend.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` runs Rust tests.
- `npm run check:release-version` verifies version agreement between
  `package.json`, `Cargo.toml`, and `tauri.conf.json`.
- `npm run tauri build` creates packaged artifacts through the repository's
  build wrapper. On macOS, `MALLARD_VERIFY_MACOS_BUNDLE=1 npm run tauri build`
  additionally verifies the DMG and contained application signature.

Some S3 transport tests bind a loopback server. Restricted environments may
need explicit permission for localhost binding.

## Implementation conventions

Use two-space indentation, double quotes, and semicolons in TypeScript. Keep
components in PascalCase files, variables and functions in camelCase, and pure
helpers close to their consumers. TypeScript is configured with strictness,
unused-symbol, and fallthrough checks; do not weaken those settings to make a
change compile.

Use `rustfmt` conventions in Rust. Prefer validated domain types and bounded
collections over passing unchecked strings or paths. Preserve clear module
boundaries rather than moving filesystem, transport, or Tauri behavior into
`domain.rs`.

For Tauri contracts:

- command names are snake_case;
- JavaScript argument keys are camelCase as expected by Tauri;
- serialized Rust fields remain snake_case unless an existing serde contract
  says otherwise;
- add or change DTOs in Rust and `src/types.ts` together;
- register new commands in `src-tauri/src/lib.rs`; and
- expose frontend calls through `projectSyncApi` instead of scattering raw
  `invoke` calls through components.

Prefer focused local state and existing request helpers over introducing a
global state layer for a small feature. Preserve revision checks and
single-flight behavior where they protect concurrent UI requests.

## Safety invariants

Mallard handles user conversations, credentials, and restore operations.
Treat safety behavior as part of the product contract:

- Never commit or log credentials, bearer tokens, private keys, local absolute
  paths, personal provider data, generated bundles, or `~/.mallard` state.
- Never run tests against a real `~/.codex`, `~/.claude`, or
  `~/.mallard`. Use temporary homes, fixture trees, or injected repository
  roots.
- Preserve canonical-path validation, bounded reads and counts, no-follow
  traversal, symlink and special-file rejection, and atomic persistence.
- Keep credentials, checkout paths, provider-home paths, local exclusions,
  restore plans, receipts, backups, and trust decisions out of portable
  manifests.
- Push publication must remain immutable and SHA-256 verified. There is no
  mutable head: a publication record is written last, with list-then-write,
  and the current snapshot is selected from the stream's record names. A
  storage provider needs only put, get, and list; one whose server refuses a
  second write of a key (S3/R2, Dropbox) also claims the record's parent
  before the record, under `_claims/`, which a refresh never lists or reads.
  A Push whose record is not selected has failed: keep every local acceptance
  (bases, checkpoints, receipts, Library accepted state) after the publish
  call returns. Never finish another writer's claim, delete a claim, or
  reassign one by itself. Observe a linked bundle only through
  `observe_linked_bundle`, which classifies a displaced base and writes
  nothing: recovery is a user action, never a rollback.
- Pull and dependency apply operations must remain plan-based and explicitly
  approved. Keep revision, digest, and expiry checks intact.
- Deletion is opt-in. File deletion must verify the expected local digest;
  directory deletion must target only an exact empty directory and must never
  become recursive.
- Ordinary project-file sync is allowed only for non-Git projects. Git-managed
  source continues to travel through Git.
- Secret detection is best-effort, not proof that a file is safe. Preserve
  warning acknowledgement for credential-shaped or executable content.

If a change relaxes one of these invariants, call it out explicitly and add a
test that demonstrates the intended boundary.

## Testing expectations

Run the smallest relevant checks while iterating, then the appropriate
handoff set:

- frontend or shared DTO changes: `npm run build` and
  `npm run test:frontend-integration`;
- Rust domain, persistence, capture, storage, or restore changes:
  `cargo test --manifest-path src-tauri/Cargo.toml --lib`;
- Tauri contract changes: run both frontend and Rust checks;
- version or release changes: `npm run check:release-version` and
  `npm run test:release-config`.

Add frontend integration coverage to an existing
`tests/frontend/*.integration.test.tsx` file when practical, and register a
new file in `scripts/run-frontend-integration-tests.mjs`. Put Rust unit tests
beside the protected helper; use `src-tauri/src/sync_tests/` for flows that
cross module boundaries.

For UI changes, verify the complete Tauri application when native behavior is
involved and include a screenshot or recording in the pull request.

## Versioning and review

Keep the application version synchronized in `package.json`,
`src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. Do not hand-edit
generated build output or lockfiles unless dependency or release scope
requires it.

Use concise imperative commit subjects consistent with the repository, such
as `Add ...`, `Fix ...`, or `Update ...`. Preserve unrelated work in a
dirty worktree.

Pull requests should state the problem, user-visible behavior, safety or schema
impact, and verification commands. Include screenshots or recordings for UI
changes and identify any checks that could not be run.
