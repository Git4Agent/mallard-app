//! Pins the Tauri command registration list for the workspace
//! reorganization (`docs/PLAN_PROJECT_STRUCTURE_REORGANIZATION.md` §2).
//!
//! The reorganization may move modules, so only the final identifier of
//! each registered command is pinned — not its module path. Adding,
//! removing, or renaming a command must fail this test until the frozen
//! contract itself is deliberately changed outside that plan.

/// The 71 registered commands at the reorganization baseline, sorted.
const EXPECTED: &[&str] = &[
    "apply_bundle_restore",
    "audit_codex_conversation_paths",
    "cleanup_activity_logs",
    "connect_project_to_remote_bundle",
    "create_provider_profile",
    "create_setup_draft",
    "discard_restore_plan",
    "discard_setup_draft",
    "finalize_project_setup",
    "find_remote_bundle_matches",
    "fork_codex_thread",
    "get_activity_log_folder",
    "get_activity_log_stats",
    "get_bundle_inventory",
    "get_bundle_readiness",
    "get_bundle_status",
    "get_host_identity_status",
    "get_local_project",
    "get_pending_codex_divergence",
    "get_project",
    "get_project_binding",
    "get_project_chat_history",
    "get_project_chat_thread_details",
    "get_project_origin_tokens",
    "get_project_sync_config",
    "get_project_thread_sync_comparison",
    "get_push_destination_status",
    "get_restore_plan",
    "get_restore_readiness",
    "get_setup_draft",
    "inspect_project_files",
    "inspect_setup_draft",
    "list_local_projects",
    "list_origin_host_aliases",
    "list_project_bindings",
    "list_project_materializations",
    "list_project_repository_kinds",
    "list_provider_profiles",
    "list_remote_bundle_snapshots",
    "list_remote_bundles",
    "list_setup_drafts",
    "open_codex_thread_in_app",
    "open_codex_thread_in_terminal",
    "plan_bundle_restore",
    "prepare_push_review",
    "probe_provider_profile",
    "push_bundle",
    "query_activity_logs",
    "recover_startup_state",
    "register_local_project",
    "remove_local_project",
    "remove_project_binding",
    "remove_project_link",
    "remove_provider_profile",
    "rename_local_project",
    "rename_provider_profile",
    "repair_codex_conversation_paths",
    "resolve_origin_host_label",
    "restart_codex_app_after_pull",
    "retry_codex_divergence",
    "rotate_host_identity",
    "save_bundle_recipe",
    "save_origin_host_alias",
    "save_project_binding",
    "save_project_link",
    "save_project_sync_config",
    "update_activity_log_policy",
    "update_host_display_name",
    "update_setup_draft",
    "validate_codex_thread_ownership",
    "verify_bundle_objects",
];

#[test]
fn registered_command_names_are_frozen() {
    let lib_source = include_str!("../lib.rs");
    let start = lib_source
        .find("generate_handler![")
        .expect("lib.rs must contain the generate_handler! registration block");
    let block = &lib_source[start + "generate_handler![".len()..];
    let end = block.find(']').expect("generate_handler! block must close");

    let mut names: Vec<&str> = block[..end]
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.rsplit("::").next().expect("command path segment"))
        .collect();
    names.sort_unstable();

    assert_eq!(
        names.len(),
        EXPECTED.len(),
        "expected {} registered commands, found {}",
        EXPECTED.len(),
        names.len()
    );
    assert_eq!(
        names, EXPECTED,
        "the registered Tauri command list is frozen by \
         docs/PLAN_PROJECT_STRUCTURE_REORGANIZATION.md; module paths may \
         change, command names may not"
    );
}
