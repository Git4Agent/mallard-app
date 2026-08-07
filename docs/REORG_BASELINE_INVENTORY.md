# Reorganization baseline inventory

Phase 0 deliverable of `PLAN_PROJECT_STRUCTURE_REORGANIZATION.md`.

- **Baseline commit:** `2aa6c7a` (Mallard 0.1.2)
- **Baseline verification:** 311 Rust tests, frontend integration suite,
  release-config tests, and `check:release-version v0.1.2` all green before
  any restructuring change.

Everything below is frozen for the duration of the reorganization. The
enforcement points are the existing test suites plus
`src-tauri/src/sync_tests/registration_freeze.rs`.

## 1. Tauri command registration (71 commands)

Five commands live in `activity_log.rs`; the remaining 66 in
`project_sync/commands.rs`. Names are pinned (sorted) in
`registration_freeze.rs`; module paths are free to change during
extraction, names and shapes are not.

Activity: `cleanup_activity_logs`, `get_activity_log_folder`,
`get_activity_log_stats`, `query_activity_logs`,
`update_activity_log_policy`.

Project sync: `apply_bundle_restore`, `audit_codex_conversation_paths`,
`connect_project_to_remote_bundle`, `create_provider_profile`,
`create_setup_draft`, `discard_restore_plan`, `discard_setup_draft`,
`finalize_project_setup`, `find_remote_bundle_matches`,
`fork_codex_thread`, `get_bundle_inventory`, `get_bundle_readiness`,
`get_bundle_status`, `get_host_identity_status`, `get_local_project`,
`get_pending_codex_divergence`, `get_project`, `get_project_binding`,
`get_project_chat_history`, `get_project_chat_thread_details`,
`get_project_origin_tokens`, `get_project_sync_config`,
`get_project_thread_sync_comparison`, `get_push_destination_status`,
`get_restore_plan`, `get_restore_readiness`, `get_setup_draft`,
`inspect_project_files`, `inspect_setup_draft`, `list_local_projects`,
`list_origin_host_aliases`, `list_project_bindings`,
`list_project_materializations`, `list_project_repository_kinds`,
`list_provider_profiles`, `list_remote_bundle_snapshots`,
`list_remote_bundles`, `list_setup_drafts`, `open_codex_thread_in_app`,
`open_codex_thread_in_terminal`, `plan_bundle_restore`,
`prepare_push_review`, `probe_provider_profile`, `push_bundle`,
`recover_startup_state`, `register_local_project`, `remove_local_project`,
`remove_project_binding`, `remove_project_link`, `remove_provider_profile`,
`rename_local_project`, `rename_provider_profile`,
`repair_codex_conversation_paths`, `resolve_origin_host_label`,
`restart_codex_app_after_pull`, `retry_codex_divergence`,
`rotate_host_identity`, `save_bundle_recipe`, `save_origin_host_alias`,
`save_project_binding`, `save_project_link`, `save_project_sync_config`,
`update_host_display_name`,
`update_setup_draft`, `validate_codex_thread_ownership`,
`verify_bundle_objects`.

## 2. Events

One frontend event channel exists: **`sync-log`**. Every activity entry is
emitted on it (and appended to the JSONL log); typed codes such as
`system.app_started` and `history.scan_warning` travel inside the payload.
Channel name, payload shape, and emission order are frozen.

## 3. Machine-local state (`~/.mallard`, schema 1)

Documents and directories owned by `project_sync/persistence.rs` and
`activity_log.rs` (authoritative detail: `CURRENT_METADATA_LAYOUT.md`):

- `sync_config.json`
- `machine_projects.json`
- `host_identity.json`, `host_rotation.json`
- `conversation_bindings.json`
- `conversation_sync_journal.json`
- `materializations.json`
- `restore_plans/`, `conversation_push_plans/`
- `project_drafts/`, `setup_transactions/`
- `backups/`, `host_recovery_quarantine/`
- `logs/` (activity JSONL and policy)
- `persistence.lock`, `host_operation.lock`

## 4. Portable bundle storage (schema 1)

`.mallard/v1/repositories/<bundle-id>/` in local-folder and S3-compatible
destinations: immutable content-addressed objects, SHA-256 verification,
compare-and-swap heads. See `CURRENT_METADATA_LAYOUT.md`.

## 5. Dependency selection

`src-tauri/Cargo.lock` is moved to the workspace root unmodified. Baseline
normal-dependency tree of the `mallard` package: 491 unique crate lines;
SHA-256 of the sorted `cargo tree -e normal --prefix none` output recorded
below. Re-run the same command after workspace changes and compare.

```text
cargo tree -p mallard -e normal --prefix none | sort -u | shasum -a 256
7702de8802cb5dcc1b356e6c2395e39c301aac8128dbe600503947c994f00926
```

## 6. Packaging identity

Product `Mallard`, identifier `com.hequ.agent-sync`, Cargo package
`mallard`, library `tauri_app_lib`, Apple Silicon DMG with ad-hoc signing,
verified by `MALLARD_VERIFY_MACOS_BUNDLE=1`.
