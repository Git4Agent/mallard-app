mod activity_log;
mod codex_config;
mod codex_plugins;
mod project_sync_v3;

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
            project_sync_v3::commands::get_project_sync_config,
            project_sync_v3::commands::save_project_sync_config,
            project_sync_v3::commands::get_host_identity_status,
            project_sync_v3::commands::update_host_display_name,
            project_sync_v3::commands::rotate_host_identity,
            project_sync_v3::commands::get_project_origin_tokens,
            project_sync_v3::commands::list_origin_host_aliases,
            project_sync_v3::commands::save_origin_host_alias,
            project_sync_v3::commands::resolve_origin_host_label,
            project_sync_v3::commands::list_local_projects,
            project_sync_v3::commands::get_project,
            project_sync_v3::commands::get_local_project,
            project_sync_v3::commands::list_project_repository_kinds,
            project_sync_v3::commands::get_project_chat_history,
            project_sync_v3::commands::get_project_chat_thread_details,
            project_sync_v3::commands::open_codex_thread_in_app,
            project_sync_v3::commands::open_codex_thread_in_terminal,
            project_sync_v3::commands::validate_codex_thread_ownership,
            project_sync_v3::commands::register_local_project,
            project_sync_v3::commands::remove_local_project,
            project_sync_v3::commands::rename_local_project,
            project_sync_v3::commands::save_bundle_recipe,
            project_sync_v3::commands::save_project_link,
            project_sync_v3::commands::connect_project_to_remote_bundle,
            project_sync_v3::commands::remove_project_link,
            project_sync_v3::commands::list_provider_profiles,
            project_sync_v3::commands::probe_provider_profile,
            project_sync_v3::commands::create_provider_profile,
            project_sync_v3::commands::rename_provider_profile,
            project_sync_v3::commands::remove_provider_profile,
            project_sync_v3::commands::list_project_bindings,
            project_sync_v3::commands::get_project_binding,
            project_sync_v3::commands::audit_codex_conversation_paths,
            project_sync_v3::commands::repair_codex_conversation_paths,
            project_sync_v3::commands::save_project_binding,
            project_sync_v3::commands::remove_project_binding,
            project_sync_v3::commands::list_project_materializations,
            project_sync_v3::commands::get_restore_plan,
            project_sync_v3::commands::discard_restore_plan,
            project_sync_v3::commands::discover_project,
            project_sync_v3::commands::get_bundle_inventory,
            project_sync_v3::commands::inspect_project_files,
            project_sync_v3::commands::list_remote_bundles,
            project_sync_v3::commands::list_bundle_generations,
            project_sync_v3::commands::compare_bundle_generations,
            project_sync_v3::commands::list_remote_bundle_snapshots,
            project_sync_v3::commands::find_remote_bundle_matches,
            project_sync_v3::commands::get_bundle_status,
            project_sync_v3::commands::get_project_capability_status,
            project_sync_v3::commands::get_project_thread_sync_comparison,
            project_sync_v3::commands::prepare_push_review,
            project_sync_v3::commands::push_bundle,
            project_sync_v3::commands::plan_bundle_restore,
            project_sync_v3::commands::apply_bundle_restore,
            project_sync_v3::commands::plan_dependencies,
            project_sync_v3::commands::apply_dependency_actions,
            project_sync_v3::commands::get_bundle_readiness,
            project_sync_v3::commands::get_restore_readiness,
            project_sync_v3::commands::list_setup_drafts,
            project_sync_v3::commands::create_setup_draft,
            project_sync_v3::commands::get_setup_draft,
            project_sync_v3::commands::update_setup_draft,
            project_sync_v3::commands::discard_setup_draft,
            project_sync_v3::commands::inspect_setup_draft,
            project_sync_v3::commands::finalize_project_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
