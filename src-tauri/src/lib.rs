mod activity_log;
mod codex_binary;
mod project_sync;

#[cfg(test)]
mod sync_tests;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(not(test))]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // A visible process boundary in the activity log: operations that
            // died with the previous process (no terminal event) become
            // explainable instead of mysterious.
            activity_log::ActivityLogScope::new(activity_log::ActivityLogType::System).emit(
                app.handle(),
                "info",
                "system.app_started",
                "Mallard started",
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            activity_log::query_activity_logs,
            activity_log::get_activity_log_stats,
            activity_log::update_activity_log_policy,
            activity_log::cleanup_activity_logs,
            activity_log::get_activity_log_folder,
            project_sync::commands::get_project_sync_config,
            project_sync::commands::save_project_sync_config,
            project_sync::commands::get_host_identity_status,
            project_sync::commands::update_host_display_name,
            project_sync::commands::rotate_host_identity,
            project_sync::commands::get_project_origin_tokens,
            project_sync::commands::list_origin_host_aliases,
            project_sync::commands::save_origin_host_alias,
            project_sync::commands::resolve_origin_host_label,
            project_sync::commands::list_local_projects,
            project_sync::commands::recover_startup_state,
            project_sync::commands::get_project,
            project_sync::commands::get_local_project,
            project_sync::commands::list_project_repository_kinds,
            project_sync::commands::get_project_chat_history,
            project_sync::commands::get_project_chat_thread_details,
            project_sync::commands::open_codex_thread_in_app,
            project_sync::commands::open_codex_thread_in_terminal,
            project_sync::commands::validate_codex_thread_ownership,
            project_sync::commands::register_local_project,
            project_sync::commands::remove_local_project,
            project_sync::commands::rename_local_project,
            project_sync::commands::save_bundle_recipe,
            project_sync::commands::save_project_link,
            project_sync::commands::connect_project_to_remote_bundle,
            project_sync::commands::remove_project_link,
            project_sync::commands::list_provider_profiles,
            project_sync::commands::probe_provider_profile,
            project_sync::commands::create_provider_profile,
            project_sync::commands::rename_provider_profile,
            project_sync::commands::remove_provider_profile,
            project_sync::commands::list_project_bindings,
            project_sync::commands::get_project_binding,
            project_sync::commands::audit_codex_conversation_paths,
            project_sync::commands::repair_codex_conversation_paths,
            project_sync::commands::save_project_binding,
            project_sync::commands::remove_project_binding,
            project_sync::commands::list_project_materializations,
            project_sync::commands::get_restore_plan,
            project_sync::commands::discard_restore_plan,
            project_sync::commands::discover_project,
            project_sync::commands::get_bundle_inventory,
            project_sync::commands::inspect_project_files,
            project_sync::commands::list_remote_bundles,
            project_sync::commands::list_remote_bundle_snapshots,
            project_sync::commands::find_remote_bundle_matches,
            project_sync::commands::get_bundle_status,
            project_sync::commands::get_project_thread_sync_comparison,
            project_sync::commands::verify_bundle_objects,
            project_sync::commands::prepare_push_review,
            project_sync::commands::get_push_destination_status,
            project_sync::commands::push_bundle,
            project_sync::commands::plan_bundle_restore,
            project_sync::commands::apply_bundle_restore,
            project_sync::commands::get_pending_codex_divergence,
            project_sync::commands::resume_codex_divergence,
            project_sync::commands::cancel_codex_divergence,
            project_sync::commands::fork_codex_thread,
            project_sync::commands::restart_codex_app_after_pull,
            project_sync::commands::get_bundle_readiness,
            project_sync::commands::get_restore_readiness,
            project_sync::commands::list_setup_drafts,
            project_sync::commands::create_setup_draft,
            project_sync::commands::get_setup_draft,
            project_sync::commands::update_setup_draft,
            project_sync::commands::discard_setup_draft,
            project_sync::commands::inspect_setup_draft,
            project_sync::commands::finalize_project_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
