# Plan: Remove typed project-config capture (sync provider homes only)

Status: implemented (2026-07-24)

## Goal

Mallard syncs only the contents of the bound provider homes (any `CODEX_HOME` /
`CLAUDE_CONFIG_DIR`, e.g. `~/.codex`, `~/.claude`, `myconf/.codex`). It never
captures codex/claude configuration artifacts from inside the project working
directory — those are Git's job.

Concretely, for a project `/projects/projectA` bound to `myconf/.codex`:

1. **Custom provider homes stay fully supported.** Profiles at arbitrary
   absolute paths, multiple profiles per provider, the "Custom path…" picker,
   and `ProjectBinding.codex_home/claude_home` resolution are all unchanged.
2. **Git projects sync nothing from the project directory.** `AGENTS.md`,
   `CLAUDE.md`, `.claude/settings.json`, `.mcp.json`, `.codex/config.toml`,
   `.codex/hooks.json`, `.claude/{skills,agents,commands,rules}`,
   `.codex/rules`, `.agents/*`, and project-declared plugin intents are no
   longer discovered, typed, projected, bundled, or restored — for any project.
3. **Non-Git projects keep the explicit "Project files" option.** The existing
   generic project-content pipeline (explicit scan → review token → per-entry
   selection → reviewed push/pull with tombstones and backups) stays exactly as
   it is, still gated to non-Git projects by the Git-eligibility check. Because
   the generic scan's hard-exclusion list only blocks VCS/build directories
   (`hard_excluded_project_content_path`, provider_capture.rs:1277-1295), a
   non-Git user can still opt agent-config files in as *ordinary reviewed
   files* — they just no longer get special typed treatment (no semantic
   merge, no install intents, no instruction-file ownership rules).
4. **No backward compatibility or migration.** Old bundles/local state that
   reference the removed typed resources fail validation with a clear error;
   stale recipe entries are purged on load.

What is being deleted is exactly the *typed* project-artifact subsystem:
discovery of instruction/settings/skills/rules/hooks/commands/agents/MCP/plugin
resources inside the project root, their sanitizing projections, the
git-tracked-instruction ownership machinery, their semantic-merge restore
actions, and their UI categories.

## What stays (explicitly out of scope)

- Conversation/thread sync end to end: `conversation_sync.rs`,
  `conversation_codec.rs`, `conversation_materialization.rs`,
  `provider_fork.rs`, chat history, session repair/audit commands, the entire
  `sync_tests/thread_variants` suite (38 tests — no mechanical updates needed;
  `BundleManifest.directories`, `CaptureRequest.include_project_content`, and
  `excluded_content_roots` all survive for the generic-content feature).
- The generic non-Git project-content pipeline: `discover_generic_project_content`
  and its ignore/secret/symlink handling, `inspect_project_files` (command +
  impl + DTOs + review token), `ProjectContentPreferences`,
  `ProjectFileSyncEligibility`, `BundleManifest.directories` + project-content
  validation, `ProjectContent*` tombstones, restore actions `WriteProjectFile`
  / `EnsureProjectDirectory` / `DeleteProjectFile` / `DeleteProjectDirectory`,
  the five-phase project apply, `ProjectFilesReviewPage.tsx`, the
  `project_files` review step, and `push_bundle`'s review-token/removal/warning
  parameters.
- Custom provider homes: `probe/create/rename/remove_provider_profile`,
  `ProviderProfileProbe`, `resolve_profile_paths`,
  `DraftProfileSelection::Pending{path}`, the setup-workspace custom-path
  input, `createProfileAtPath`, and the profile-overlap validation.
- Provider-home global artifacts: standalone skills under `<home>/skills`
  (capture, `apply_custom_skill_action` transactional install), global plugin
  install intents (`global_inventory.rs`, `dependencies_from_manifest`,
  `plan_dependencies` / `apply_dependency_actions`,
  `codex_plugins::apply_approved_managed_plan`), Claude memory
  (`<claude_home>/projects/<bucket>/memory`), file-history, todos, and the
  Codex session index.
- `ApplyPolicy::Merge` and `RestoreActionKind::MergeFile` — used by the
  home-scoped Codex session index (provider_capture.rs:2181) and Claude memory
  (provider_capture.rs:2392), not only by instruction files. Do NOT delete.
- `ResourceKind::Requirement` and `BundleRecipe`/`RecipeEntry` — Requirement
  placeholders are synthesized for any recipe entry not discoverable locally,
  including conversations (commands.rs:3141-3166), and conversation push
  selection is recipe-driven. Do NOT delete.
- `list_project_repository_kinds` (chat_history.rs:226-251) — still gates the
  non-Git project-files step and feeds the sidebar Git badges.

## Removal inventory

Line numbers are as of commit 458c13d; treat symbol names as authoritative.

### 1. Capture — `src-tauri/src/project_sync_v3/provider_capture.rs`

- `PROJECT_INSTRUCTION_FILES` const (46-50).
- `discover_project_files` (827-927) — the whole typed enumerator: instruction
  files, the single-file allowlist (`.codex/config.toml`, `.codex/hooks.json`,
  `.claude/settings.json`, `.mcp.json`, `.agents/plugins/marketplace.json`),
  the grouped dirs (`.agents/skills`, `.claude/{skills,agents,commands,rules}`,
  `.codex/rules`), and its call site in `discover` (749-753). The
  `discover_generic_project_content` call (754-762) stays.
- `git_tracked_project_instruction_paths` / `..._resource_ids` (787-825) and
  every commands.rs consumer (wrapper 2998-3001; call sites 3123, 4936, 10192).
- `add_single_project_file` / `project_file_resource_id` (1438-1497),
  `path_exists_no_follow` (3228-3242).
- The sanitizing projections and their private validators (1503-1852):
  `portable_project_projection`, `project_codex_config_projection`,
  `project_claude_settings_projection`, `project_mcp_projection`,
  `reviewed_json_projection`, `sanitize_reviewed_json`,
  `validate_portable_toml_value/json_value/string`, `forbidden_portable_field`,
  `exact_environment_reference`, `portable_environment_name`, `canonical_json`.
  (Keep `looks_like_local_path` / `contains_url_credentials` — used by
  `portable_dependency_id` for global plugins.)
- `discover_grouped_directory` (1854-1955).
- `discover_plugin_intents` (1957-2037) and `extract_claude_plugin_ids` /
  `extract_codex_plugin_ids` (2846-2887). `dependency_resource` and
  `portable_dependency_id` stay (global plugins).
- `DependencyScope::Project` (91) — global (`ProviderHome`) scope stays; the
  project-origin coalescing branch in `discover_global_plugin` (2559-2569)
  becomes dead and goes with it.
- `CaptureResourceKind` (52-69): delete `ProjectFile`, `ProjectSettings`,
  `Agent`, `Command`, `Rule`, `Skill`, `Hook`, `McpServer`. Keep
  `Conversation`, `Memory`, `ProjectContentFile`, `ProjectContentDirectory`,
  `StandaloneSkill`, `Plugin`.
- Trim the deleted kinds' arms in `domain_resources` / `domain_kind` (551-697)
  and `resource_kind_name` (3457-3474).
- `validate_source_file` (3014-3048): approved roots become the two bound
  homes, standalone-skill dirs, and the project root only when
  `include_project_content` capture is active.
- `is_executable_path`: keep (standalone skills); its grouped-dir caller dies.
- Tests: delete the instruction/allowlist/grouped-dir/projection/plugin-intent
  suites (non_git_project_instruction_files…, git_tracked…,
  untracked_instruction…, unsafe_agents_symlink…, plugin_intent…,
  settings_capture…, mcp_projection…, credential_file_and_symlink_block_skill_capture).
  Keep the conversation tests and all `project_content_*` tests.

### 2. Domain — `src-tauri/src/project_sync_v3/domain.rs`

- `ResourceKind` (635-653): delete `ProjectFile`, `Agent`, `Command`, `Rule`,
  `Prompt`, `ProjectSkill`, `Hook`, `Setting`, `McpServer`. Keep
  `CodexConversation`, `ClaudeConversation`, `ProjectMemory` (home-scoped
  Claude memory despite the name), `ProjectContentFile`,
  `ProjectContentDirectory`, `StandaloneSkill`, `Plugin`, `Requirement`.
- `RestoreActionType` / `RestoreActionKind` (3401-3487): delete `ReviewHook`,
  `ReviewMcp`, `ApplySetting` (+ their `validate` arms 3522-3524, 3549-3551,
  and `action_type` mappings). Everything else keeps: the four project-content
  actions, `MergeFile`, `WriteFile`, skill installs, `InstallPlugin`,
  `MaterializeConversation`, `Manual`.
- `RestoreAction::validate` explicit-approval gate (3592-3611): drop the three
  deleted types from the `matches!` list.
- `Provenance` (675-739): audit after the capture trim — `ProjectLocal` stays
  (project-content descriptors are pinned to it), `StandaloneSnapshot` and
  `Plugin` stay; delete `Git` only if the grouped-dir capture was its last
  producer (verify with grep before removing).
- Unchanged, by design: `LogicalPath` grammar (the `project/` root stays for
  generic content), `ResourceScope` (all four variants stay — `Requirement`
  serves placeholders, `Dependency` serves global plugins), `ApplyPolicy` (all
  variants stay), `ProjectContentPreferences`, `ProjectFileSyncEligibility*`,
  `RestorePlan.project_content_eligibility`, `BundleManifest.directories` and
  the project-content validation section, `ProjectContent*` tombstones,
  `DependencyPlan` / `DependencyApplications`, `ProviderProfile`,
  `MachineProjectState`, `DraftProfileSelection`, `BundleRecipe`.
- Tests: retarget `manifest_requires_recipe_resource_and_file_referential_integrity`
  (4505-4572) off `ResourceKind::ProjectFile` (use a conversation or
  project-content fixture). All `project_content_*` and custom-home tests stay.

### 3. Bundle engine — `src-tauri/src/project_sync_v3/bundle_engine.rs`

- `restore_kind_for_file` (3842-3885): delete the `Hook` / `McpServer` /
  `Setting` arms (3866-3877). The `ProjectContentFile`, session-index,
  conversation, and `Merge`/`WriteFile` fallback arms stay.
- `action_needs_explicit_approval` (3887-3913): drop `Hook` / `McpServer` /
  `Setting` from the always-approve kind list; `StandaloneSkill` / `Plugin`
  stay.
- The semantic-merge cluster (4493-4605): `merge_portable_project_definition`,
  `deep_merge_toml`, `deep_merge_json`, `merge_json_array`,
  `json_merge_identity` — sole caller is the `Setting`/`McpServer`/`Hook`
  branch of `apply_file_action` (5786-5794); delete branch and cluster
  together. The session-index merge (5784) and plain writes are untouched.
- `logical_path_for_review_action` (5670-5692) and the
  `ReviewHook`/`ReviewMcp`/`ApplySetting` dispatch arm in `apply_restore_plan`
  (5349-5376): delete.
- `build_schema6_cutover_request` (481-746): delete under the no-backcompat
  rule, together with `sync_tests/thread_variants/schema_cutover.rs` and the
  cutover portions of `storage_parity.rs`.
- Tests: delete `settings_and_mcp_apply_preserve_target_only_fields_and_secrets`,
  `hook_and_toml_settings_apply_merge_by_identity_and_preserve_target_keys`
  (2355-2589), `executable_skill_payload_requires_explicit_file_and_dependency_approval`
  (2836-…; project-skill payloads no longer exist). Keep all project-content,
  conversation-materialization, custom-skill, and store-protocol tests.
- Everything else in this file is untouched: publish/fetch/CAS, directories
  loop, project tombstone loop, project-dir prepare/finalize/delete,
  `WriteProjectFile` mode/mtime enforcement, `map_logical_target` (all
  branches), `required_home`/`canonical_bound_home` (custom homes),
  `dependencies_from_manifest`, skill install.

### 4. Commands — `src-tauri/src/project_sync_v3/commands.rs` + `src-tauri/src/lib.rs`

- Delete `git_tracked_project_instruction_resource_ids`,
  `persist_git_tracked_project_instruction_exclusions`,
  `git_tracked_manifest_instruction_resource_ids` (2998-3097) and their call
  sites: `get_bundle_inventory_with_repository` 3122-3128, push retain
  4935-4939, restore-plan filter 5317-5322, restore-apply re-check 5385-5401.
- `apply_bundle_restore_with_repository`: delete the `ReviewHook` /
  `ReviewMcp` / `ApplySetting` receipt arms (5435-5437); same arms in
  `bundle_readiness_for_fetched` (6091-6093).
- `capture_resource_kind` / `resource_category` (3358-3403): delete the arms
  for the removed typed kinds and the `project_setup` category string. The
  `tools` category shrinks to Requirement placeholders (keep the id, adjust
  frontend copy).
- `inventory_from_candidates` (3175-3269): the removed kinds' scope/provenance
  arms go; `ProjectContent*` handling stays.
- `is_capability_kind` (4441-4446): drop `ResourceKind::ProjectSkill`.
- `validate_plugin_install_intent` (6334-6367): delete the Claude
  `--scope project` argv variant (6348-6354).
- `run_plugin_install` (6530-6572): survives for global plugins; move its
  `current_dir` off `binding.project_root` (project-scoped installs no longer
  exist).
- Unchanged: `inspect_project_files` and the whole project-content review
  pipeline, `push_bundle`'s full signature, `project_file_sync_eligibility`,
  all provider-profile commands, `discover_project` / `get_bundle_inventory`
  (their inventories simply stop containing typed project artifacts), all
  conversation/chat/audit commands, `lib.rs` registration list (no commands are
  removed — verify none of the 64 become empty shells).
- Tests: delete the git-tracked-instruction suites (10162, 10274) and any
  typed-artifact discovery fixtures; keep the project-content suites
  (10143, 10370-10748, 10749, 10796), custom-home probe/profile tests
  (12733-12805, 12868, 13107-13495), and conversation tests.

### 5. Persistence — `src-tauri/src/project_sync_v3/persistence.rs`

- Only change: delete the schema-1 legacy decode/migration branches under the
  no-backcompat rule — `migrate_legacy_setup_transactions_unlocked`, the V1
  arms of `decode_machine_project_state` / `decode_setup_transaction`,
  `machine_projects_absent_or_valid_legacy_unlocked` (2425-2484, 3659-3728,
  2247-2265) and the `MACHINE_PROJECT_SCHEMA_V1` / `SETUP_TRANSACTION_SCHEMA_V1`
  imports. Everything else (materializations, restore plans, dependency
  plans/applications, host rotation, setup drafts) is untouched.

### 6. Plugin/config helpers — `src-tauri/src/codex_plugins.rs`, `src-tauri/src/codex_config.rs`

Independent dead-code cleanup, safe regardless of this refactor (zero
production callers, verified):

- codex_plugins.rs: the legacy lock-file sync pipeline — `LOCK_REL` /
  `CLAUDE_LOCK_REL`, lock IO (783-1047), `capture_lock` / `capture_with`,
  `enabled_managed_plugin_ids` → `plan_with_runner` / `apply_plan` /
  `apply_managed_plan` (1085-1231, 1347-1433, 1960-1970, 2312-2334), the
  merge drivers (2713-2823), the Claude lock capture/planning (2887-2970,
  3047-3111), and their tests. The live surface
  (`apply_approved_managed_plan`, `claude_inventory`, `ProcessRunner` with
  custom `codex_home`) stays — global plugins survive.
- codex_config.rs: the dead overlay/compose pipeline (`LocalOverlay`,
  `split_local_overlay`, `apply_local_overlay`, `project_portable_bytes`,
  `compose_physical_bytes`, `is_config_artifact`, `enabled_managed_plugin_ids_from_bytes`,
  bare `inspect_managed_config`) and its tests. The repair functions used by
  `apply_approved_managed_plan` stay, including
  `managed_marketplace_home_is_allowed_for_default` (custom homes stay).
- Fix the stale module doc (codex_plugins.rs:1-14) referencing a merge-driver
  dispatch that no longer exists.

### 7. Frontend — `src/`

- `src/types.ts`: delete `ProjectResourceKind` variants `project_file`,
  `agent`, `command`, `rule`, `prompt`, `project_skill`, `mcp_server`, `hook`,
  `setting`; delete `RestoreActionKind` variants `review_hook`, `review_mcp`,
  `apply_setting`; delete the `project_setup` category. Keep everything
  project-content, profile/probe, capability, and dependency-plan related.
  Delete verified-dead `PlannedAction`/`PlanActionRisk` (672-688).
- `src/components/project-sync/model.ts`: remove the `project_setup` group and
  the typed-kind mappings in `CATEGORY_BY_KIND`; re-point `categoryFor`'s
  fallback (currently `project_setup`) at a surviving category; reword the
  `tools` group copy (now Requirement placeholders only).
- `pullReviewModel.ts`: delete the `apply_setting`/`review_hook`/`review_mcp`
  copy branches (140-148). All `project_content` handling stays.
- `SkillsPluginStatusPage.tsx`: drop the "project skill" scope branch
  (`capabilityScope`, 65-69) and project-skill rows; page stays.
- `ResourceInventory.tsx`: remove icons for deleted categories/kinds.
- Delete dead code: `ProjectWorkspace.tsx`, `api.getStatus` / `api.saveRecipe`,
  `model.statusMap` (all verified unimported).
- Unchanged: `ProjectFilesReviewPage.tsx`, `SyncReviewTabs` (including the
  `project_files` step), `PushResourceWorkspace` / `RestorePlanView`
  project-content wiring, `ProjectSyncV3` scan/publish flows, all custom-home
  UI (`ProjectSetupWorkspace` custom path, `createProfileAtPath`,
  `ProjectBindingEditor` Add), chat history UI, `App.css` project-files and
  capability blocks.
- `src/components/ProfilePagePrototypes.tsx` (static prototype, not wired into
  the app): remove or relabel its "Project skills"/"Project plugins" mock tabs
  for consistency; low priority.

### 8. Tests outside src — `tests/`, `scripts/`

- `tests/frontend/project-files.integration.test.tsx`: KEEP (feature stays).
- `tests/frontend/capability-status.integration.test.tsx`: trim the
  `project_skill` kind from the `capability()` fixture and related assertions.
- `tests/frontend/pull-review.integration.test.tsx` /
  `resource-inventory.integration.test.tsx`: unaffected except any fixture
  using a deleted typed kind — sweep for `"setting"`, `"hook"`,
  `"mcp_server"`, `"project_skill"`, `"agent"`, `"rule"` kinds and update.
- `scripts/run-frontend-integration-tests.mjs`: entry list unchanged unless a
  test file is deleted.
- `src-tauri/src/sync_tests/`: unchanged (no struct fields it uses are
  removed), except deleting `thread_variants/schema_cutover.rs` and the
  cutover halves of `storage_parity.rs` with the cutover function.

### 9. Docs

- `README.md`: reword "Syncs selected project conversations, setup, skills,
  plugins…" (28-29) — "setup" no longer includes project-dir config; the
  non-Git project-files bullet (35-36) stays. Rewrite the git-ownership
  paragraph in "Push, Pull, and project-file safety" (144-146) — the
  AGENTS.md/CLAUDE.md single-sync-owner rule is gone; the non-Git scan and
  deletion/backup semantics (147-155) stay.
- `TECHNICAL_REPORT.md`: §7 (non-Git project-file sync) STAYS. Trim §3's
  resource-kind list, §6's instruction-file git-index paragraph (201-205), the
  hook/MCP/settings restore prose in §§8/10, and refresh §13 test counts after
  the suites shrink. Custom-home statements (112-114) stay accurate.
- `AGENTS.md`: keep invariant "Ordinary project-file sync is allowed only for
  non-Git projects" (126-127) and the warning-acknowledgement invariant
  (128-129 — the generic scan still uses it). Reword the product claim (3-8)
  so "setup" refers to provider-home state only.
- `docs/AGENT_SYNC_FILE_SETS.md`: mark the schema-3 handoff section's
  reviewed-settings/MCP/hooks/project-skills claims (24-27, 46-50) as
  superseded; the ~/.codex and ~/.claude tier tables stay the reference.
- Banner-mark `docs/PLAN_PROJECT_SCOPED_SYNC.md`'s typed project-artifact
  sections as superseded by this plan. `PLAN_NON_GIT_PROJECT_FILE_SYNC.md`,
  `TEST_PLAN_NON_GIT_PROJECT_CONTENT_SYNC.md`, and
  `PLAN_PROJECT_PROFILE_ASSOCIATION.md` remain accurate (their features stay).

## Old-data handling (no migration)

- Bundles whose manifests contain the deleted resource kinds or
  `ReviewHook`/`ReviewMcp`/`ApplySetting` actions fail deserialization or
  validation on fetch — that is the intended invalidation. No schema-version
  bump is required; if a friendlier error is wanted, add a kind check with an
  explicit message in `BundleManifest::validate`.
- On loading `machine_projects.json`, purge recipe entries whose resource ids
  reference deleted typed kinds (`project:file:*`, `setting:*`, `hook:*`,
  `mcp:*`, `agent:*`, `command:*`, `rule:*`, project-skill ids); otherwise they
  surface as bogus "Requirement" placeholder rows via the synthesis at
  commands.rs:3141-3166. Project-content and conversation entries are kept.
- Existing `materializations.json` receipts referencing deleted action types:
  receipts are historical; either tolerate unknown action types at
  deserialization or purge those records on first load — pick during
  implementation (purging is simpler and allowed).

## Execution order

Each step leaves the tree compiling:

1. commands.rs: excise git-tracked-instruction machinery, review-action arms,
   `--scope project` variant, typed-kind mapping arms.
2. bundle_engine.rs: review-action arms, semantic-merge cluster, cutover fn.
3. provider_capture.rs: typed discovery, projections, plugin intents,
   grouped dirs.
4. domain.rs last: delete the enum variants; the compiler flags any straggler
   match arms across all three files above.
5. Frontend: types.ts variants → model.ts/pullReviewModel/SkillsPluginStatus →
   dead-code deletions.
6. Tests (Rust + frontend fixtures), then docs.

## Verification

- `cargo test`, `cargo clippy`, `tsc --noEmit`, `npm run test:frontend-integration`.
- Manual round trips:
  1. Git project bound to a custom home (`myconf/.codex`): push/pull
     conversations; verify `AGENTS.md` and `.claude/` in the project dir are
     never captured (inspect the bundle manifest — no `project/` paths, no
     typed setup resources).
  2. Non-Git project: scan → select files (including a `.claude/settings.json`
     if desired) → push → pull on a second machine/profile; verify the file
     arrives as a plain reviewed file (no semantic merge).
  3. Standalone skill under the custom home and a global plugin: still sync.
