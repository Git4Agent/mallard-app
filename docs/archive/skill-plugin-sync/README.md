# Archive: skill & plugin sync

Removed **2026-07-30** on `main` (last commit before removal: `a9e428e`
"Make conversation divergence sync per-thread"; the removal itself is the
first commit touching this directory). This archive is a reference snapshot —
it will not compile against the post-removal tree and is not meant to be
re-imported wholesale.

## What the feature did

Mallard used to sync two "capability" resource classes alongside conversations
and project files:

- **Standalone skills** — global custom skill directories under a mapped
  provider home's `skills/` root. Capture inventoried them with a
  plugin-ownership classifier (`global_inventory.rs`) so plugin-provided
  skills were never separately snapshotted; restore materialized the whole
  directory as one typed, digest-pinned install/overwrite action with staging,
  backup, and rename under a provider-home lock (`bundle_engine.rs`).
- **Plugins** — portable install intent only (plugin id + marketplace
  provenance, never payloads or caches). Pull replayed installs through the
  provider's own CLI (`codex plugin marketplace add` / `codex plugin add`,
  `claude plugin install`) via `codex_plugins.rs`, with managed-marketplace
  resolution and config repair in `codex_config.rs`.

The pipeline was: capture → bundle descriptors (`ResourceKind::StandaloneSkill`
/ `Plugin`, `ResourceScope::Dependency`) → restore-plan skill actions +
a separate immutable **DependencyPlan** (generation-pinned approval surface
with its own persistence in `~/.mallard/dependency_plans/` and receipts in
`dependency_applications.json`) → native CLI installers → readiness
verification. The UI surfaced it as Skills/Plugins tabs in the project
workspace, the Push wizard, and the Pull review, backed by
`get_project_capability_status`.

## Why it was removed

Skills and plugins are global, provider-managed, machine-local state. Syncing
them meant shelling out to `codex`/`claude` CLIs during Pull and reasoning
about install intent that may resolve differently per machine. Removing the
class collapses the sync model to conversations + memory + reviewed project
files, and deletes the entire dependency-plan layer (nothing else ever
produced a `DependencyAction`).

## Contents

```
src-tauri/src/codex_plugins.rs                        # whole file (3,924 lines)
src-tauri/src/codex_config.rs                         # whole file (1,215 lines; plugin-repair support, deleted with it)
src-tauri/src/project_sync/global_inventory.rs        # whole file (728 lines)
src/components/project-sync/SkillsPluginStatusPage.tsx# whole file (457 lines)
tests/frontend/capability-status.integration.test.tsx # whole file (362 lines)
excerpts/domain.rs.md            # Provenance/action variants, dependency types, validate_skill_name
excerpts/provider_capture.rs.md  # capture-side dependency model, discover_* fns, request fields
excerpts/bundle_engine.rs.md     # custom-skill plan build, tree digests, staged apply, dependencies_from_manifest
excerpts/commands.rs.md          # capability status, dependency plan/apply, installers, readiness checks
excerpts/persistence.rs.md       # dependency plan/applications storage + quarantine
excerpts/frontend.md             # removal diff for surviving frontend files
excerpts/tests.md                # Rust test cases stripped from surviving suites
```

Line numbers in the excerpts are pre-removal positions within the working tree
at `a9e428e`.

Notes:

- `find_binary` and the packaged-Codex native resolution survived — they moved
  from `codex_plugins.rs` to `src-tauri/src/codex_binary.rs`.
- `codex_config.rs` turned out to be consumed **only** by `codex_plugins.rs`
  (managed marketplace/MCP repair for plugin replay), so it was deleted whole
  rather than trimmed.
- `ApplyPolicy::ExplicitInstall`, `ResourceScope::Dependency`, and
  `CaptureApplyPolicy::Dependency` died with the feature; nothing else
  produced them.

## Design history

The design docs stay in `docs/` as historical record:
[PLAN_PLUGIN_SYNC.md](../../PLAN_PLUGIN_SYNC.md),
[PLAN_V3_GLOBAL_PLUGINS_AND_SKILLS.md](../../PLAN_V3_GLOBAL_PLUGINS_AND_SKILLS.md),
[PLAN_CODEX_PLUGIN_RESTORE_HARDENING.md](../../PLAN_CODEX_PLUGIN_RESTORE_HARDENING.md),
[PLAN_CLAUDE_PLUGIN_LOCK.md](../../PLAN_CLAUDE_PLUGIN_LOCK.md).
