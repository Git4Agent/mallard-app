// The Tauri command surface is referenced only through `generate_handler!`
// inside the `cfg(not(test))` runtime below, so the lib-test target would
// otherwise flag every wrapper as dead. Production builds keep the full
// dead-code gate.
#[cfg(not(test))]
use tauri::Manager as _;

#[cfg_attr(test, allow(dead_code))]
mod activity_log;
#[cfg_attr(test, allow(dead_code))]
mod app_reset;
#[cfg_attr(test, allow(dead_code))]
mod app_settings;
#[cfg_attr(test, allow(dead_code))]
mod project_sync;
#[cfg_attr(test, allow(dead_code))]
mod storage_catalog;

#[cfg(test)]
mod sync_tests;

pub use project_sync::service::{
    CreateSetupDraftResult, ProjectActivity, ProjectDetail, ProjectOperationResult,
    ProjectSessionDetail, ProjectSessionTimelinePage, ProjectSessionTimelineRequest,
    ProjectSyncService, PushApproval, PushDestinationPullReason, PushDestinationStatus,
    PushReviewContext, RestoreResult, SessionDetailError, SessionDetailErrorCode,
    SessionEventDetail, SessionTimelineEventKind, SessionToolCategory, SetupDraftInspection,
    ThreadSyncComparison, ThreadSyncState,
};
pub use storage_catalog::adapters::{CmdOutput, PluginProcessRunner};
pub use storage_catalog::domain::{
    CatalogActionOutcome, CatalogLinkId, CatalogLinkState, CatalogPlanActionId, CatalogPlanId,
    CatalogPullActionKind, CatalogPullDisposition, CatalogResource, CatalogResourceId,
    CatalogResourceKindTag, CatalogResourceRelation, CatalogResourceRepositoryId, CatalogScopeId,
    CatalogScopePullPlan, CatalogScopePushPlan, CodexPetFormat, LibraryHeadCoordinate,
    LibraryResourceTag, LocalCatalogProfile, LocalCatalogProfileId, ProviderKey, CATALOG_SCHEMA,
};
pub use storage_catalog::resource_commands::{
    CatalogScopeDto, CatalogScopeLinkOutcomeDto, CatalogScopePullOutcomeDto,
    CatalogScopePushOutcomeDto, CatalogScopeResourcesDto, CatalogScopeRowDto,
    CatalogStorageRepositoryDto,
};
pub use storage_catalog::service::{
    CatalogPullApproval, CatalogPushApproval, CatalogRepositoryInspectionDto, StorageCatalogService,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(not(test))]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Tuning values must be live before the first command runs. A
            // corrupt or out-of-range settings file falls back to defaults —
            // it must never brick the launch; the Settings page surfaces the
            // load error itself, retryably.
            mallard_infrastructure::app_settings::install_app_settings(
                &mallard_infrastructure::persistence::LocalStateRepository::from_user_home()
                    .and_then(|repository| {
                        mallard_infrastructure::app_settings::load_app_settings(&repository)
                    })
                    .unwrap_or_default(),
            );
            // A visible process boundary in the activity log: operations that
            // died with the previous process (no terminal event) become
            // explainable instead of mysterious.
            activity_log::ActivityLogScope::new(activity_log::ActivityLogType::System).emit(
                app.handle(),
                "info",
                "system.app_started",
                "Mallard started",
            );
            let watcher = project_sync::config_watcher::ConfigWatcher::start(app.handle().clone())
                .map_err(std::io::Error::other)?;
            app.manage(std::sync::Mutex::new(watcher));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            activity_log::query_activity_logs,
            activity_log::get_activity_log_stats,
            activity_log::update_activity_log_policy,
            activity_log::cleanup_activity_logs,
            activity_log::get_activity_log_folder,
            app_reset::get_local_state_summary,
            app_reset::reset_local_state,
            app_settings::get_app_settings,
            app_settings::update_app_settings,
            project_sync::commands::get_project_sync_config,
            project_sync::commands::connect_r2_storage,
            project_sync::commands::get_mallard_cli_path,
            project_sync::commands::save_project_sync_config,
            project_sync::commands::get_host_identity_status,
            project_sync::commands::list_local_projects,
            project_sync::commands::recover_startup_state,
            project_sync::commands::get_project,
            project_sync::commands::list_project_repository_kinds,
            project_sync::commands::list_agent_providers,
            project_sync::commands::get_project_activity,
            project_sync::commands::get_project_thread_sync_comparison,
            project_sync::commands::get_project_session_detail,
            project_sync::commands::get_project_session_timeline_page,
            project_sync::commands::get_project_session_event_detail,
            project_sync::commands::open_project_markdown_path,
            project_sync::commands::read_project_markdown_image,
            project_sync::commands::execute_project_session_action,
            project_sync::commands::remove_local_project,
            project_sync::commands::reset_project_sync_metadata,
            project_sync::commands::clear_storage_data,
            project_sync::commands::rename_local_project,
            project_sync::commands::save_project_link,
            project_sync::commands::connect_project_to_remote_bundle,
            project_sync::commands::remove_project_link,
            project_sync::commands::list_provider_profiles,
            project_sync::commands::probe_provider_profile,
            project_sync::commands::create_provider_profile,
            project_sync::commands::list_project_bindings,
            project_sync::commands::audit_codex_conversation_paths,
            project_sync::commands::repair_codex_conversation_paths,
            project_sync::commands::save_project_binding,
            project_sync::commands::get_bundle_inventory,
            project_sync::commands::inspect_project_files,
            project_sync::commands::list_remote_bundle_snapshots,
            project_sync::commands::list_pending_storage_bundle_snapshots,
            project_sync::commands::get_sibling_bundle_status,
            project_sync::commands::prepare_push_review,
            project_sync::commands::get_push_destination_status,
            project_sync::commands::push_bundle,
            project_sync::commands::plan_bundle_restore,
            project_sync::commands::apply_bundle_restore,
            project_sync::commands::restart_codex_app_after_pull,
            project_sync::commands::get_restore_readiness,
            project_sync::commands::list_setup_drafts,
            project_sync::commands::create_setup_draft,
            project_sync::commands::get_setup_draft,
            project_sync::commands::update_setup_draft,
            project_sync::commands::discard_setup_draft,
            project_sync::commands::inspect_setup_draft,
            project_sync::commands::finalize_project_setup,
            storage_catalog::resource_commands::list_catalog_capabilities,
            storage_catalog::resource_commands::list_catalog_resource_profiles,
            storage_catalog::resource_commands::save_catalog_resource_profile,
            storage_catalog::resource_commands::rename_catalog_resource_profile,
            storage_catalog::resource_commands::list_catalog_scopes,
            storage_catalog::resource_commands::create_catalog_scope,
            storage_catalog::resource_commands::delete_catalog_scope,
            storage_catalog::resource_commands::list_catalog_scope_resources,
            storage_catalog::resource_commands::list_catalog_storage_repositories,
            storage_catalog::resource_commands::get_catalog_pet_preview,
            storage_catalog::resource_commands::link_catalog_scope_storage,
            storage_catalog::resource_commands::unlink_catalog_scope_storage,
            storage_catalog::resource_commands::prepare_catalog_scope_push,
            storage_catalog::resource_commands::apply_catalog_scope_push,
            storage_catalog::resource_commands::prepare_catalog_scope_pull,
            storage_catalog::resource_commands::apply_catalog_scope_pull,
            storage_catalog::resource_commands::prepare_catalog_resource_fork,
            storage_catalog::resource_commands::apply_catalog_resource_fork,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
