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
mod app_update;
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
    StorageDataClearResult, ThreadSyncComparison, ThreadSyncState,
};
pub use storage_catalog::adapters::{CmdOutput, PluginProcessRunner};
pub use storage_catalog::domain::{
    CatalogActionOutcome, CatalogCoordinate, CatalogLinkId, CatalogLinkState, CatalogPlanActionId,
    CatalogPlanId, CatalogPullActionKind, CatalogPullDisposition, CatalogResourceId,
    CatalogResourceRelation, CatalogResourceRepositoryId, CatalogScopeId, CatalogScopePullPlan,
    CatalogScopePushPlan, LibraryHeadCoordinate, LibraryResourceTag, LocalCatalogProfileId,
    ProviderKey, CATALOG_SCHEMA,
};
pub use storage_catalog::registry::CatalogCapabilityDescriptor;
pub use storage_catalog::service::{
    CatalogProfileDto, CatalogPullApproval, CatalogPushApproval, CatalogScopeDto,
    CatalogScopeLinkOutcomeDto, CatalogScopePullOutcomeDto, CatalogScopePushOutcomeDto,
    CatalogScopeResourcesDto, CatalogScopeRowDto, CatalogStorageRepositoryDto,
    StorageCatalogService,
};

/// The app menu's Quit item. Tauri's predefined Quit sends `terminate:`
/// straight to the process, so ⌘Q and Mallard › Quit skipped the frontend's
/// "Sync in progress" guard, which only hears window close requests. A plain
/// item with the same shortcut lets the handler route Quit through that
/// close request instead; the guard then decides, and the app exits when its
/// last window is gone.
#[cfg(not(test))]
const QUIT_MENU_ID: &str = "mallard-quit";

#[cfg(not(test))]
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let menu = tauri::menu::Menu::default(app)?;
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::{AboutMetadata, MenuItem, PredefinedMenuItem, Submenu};
        // Mirrors Tauri's default application submenu item for item, with
        // Quit swapped for the guarded one.
        let pkg_info = app.package_info();
        let config = app.config();
        let about_metadata = AboutMetadata {
            name: Some(pkg_info.name.clone()),
            version: Some(pkg_info.version.to_string()),
            copyright: config.bundle.copyright.clone(),
            authors: config.bundle.publisher.clone().map(|p| vec![p]),
            ..Default::default()
        };
        let quit = MenuItem::with_id(
            app,
            QUIT_MENU_ID,
            format!("Quit {}", pkg_info.name),
            true,
            Some("CmdOrCtrl+Q"),
        )?;
        let app_submenu = Submenu::with_items(
            app,
            pkg_info.name.clone(),
            true,
            &[
                &PredefinedMenuItem::about(app, None, Some(about_metadata))?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::services(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, None)?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;
        if let Some(default_app_submenu) = menu.items()?.first() {
            menu.remove(default_app_submenu)?;
        }
        menu.prepend(&app_submenu)?;
    }
    Ok(menu)
}

#[cfg(not(test))]
fn on_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    if event.id() != QUIT_MENU_ID {
        return;
    }
    let windows = app.webview_windows();
    if windows.is_empty() {
        app.exit(0);
        return;
    }
    // A close request, not a destroy: the frontend's onCloseRequested
    // listener (ProjectSync.tsx) may prevent it while a sync is writing.
    for window in windows.values() {
        let _ = window.close();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[cfg(not(test))]
pub fn run() {
    tauri::Builder::default()
        .manage(project_sync::r2_setup::R2SetupControl::default())
        .manage(project_sync::dropbox_setup::DropboxSetupControl::default())
        .manage(project_sync::feishu_setup::FeishuSetupControl::default())
        .manage(project_sync::managed_cloud_setup::ManagedCloudSetupControl::default())
        .manage(app_update::AppUpdateState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // The updater's configuration (endpoint, public key, Windows install
        // mode); `app_update` drives it from Rust, and the webview holds no
        // updater permission.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(app_menu)
        .on_menu_event(on_menu_event)
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
            activity_log::ActivityLogScope::new(
                mallard_infrastructure::activity_log::ActivityLogType::System,
            )
            .emit(
                app.handle(),
                "info",
                "system.app_started",
                "Mallard started",
            );
            let watcher = project_sync::config_watcher::ConfigWatcher::start(app.handle().clone())
                .map_err(std::io::Error::other)?;
            app.manage(std::sync::Mutex::new(watcher));
            project_sync::config_watcher::start_connection_forwarder(app.handle().clone());
            project_sync::config_watcher::start_ignored_name_forwarder(app.handle().clone());
            // The Feishu sign-ins this computer keeps live beside the
            // config; a launch that cannot find the state root keeps no
            // store, and every Feishu storage then reads as reconnect.
            if let Ok(repository) =
                mallard_infrastructure::persistence::LocalStateRepository::from_user_home()
            {
                let _ = mallard_infrastructure::feishu_connections::install(&repository);
            }
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
            app_update::prepare_app_update,
            app_update::restart_to_update,
            project_sync::commands::get_project_sync_config,
            project_sync::commands::list_storage_providers,
            project_sync::commands::rename_storage,
            project_sync::commands::reveal_storage_secret,
            project_sync::commands::test_storage,
            project_sync::commands::remove_storage,
            project_sync::commands::setup_r2_storage,
            project_sync::commands::propose_r2_bucket_name,
            project_sync::commands::get_r2_setup_accounts,
            project_sync::dropbox_setup::authorize_dropbox_setup,
            project_sync::dropbox_setup::propose_dropbox_folder_name,
            project_sync::dropbox_setup::discover_dropbox_storage,
            project_sync::dropbox_setup::prepare_dropbox_storage,
            project_sync::dropbox_setup::cancel_dropbox_setup,
            project_sync::feishu_setup::authorize_feishu_setup,
            project_sync::feishu_setup::propose_feishu_folder_name,
            project_sync::feishu_setup::prepare_feishu_storage,
            project_sync::feishu_setup::cancel_feishu_setup,
            project_sync::managed_cloud_setup::sign_in_managed_cloud_storage,
            project_sync::managed_cloud_setup::cancel_managed_cloud_setup,
            project_sync::managed_cloud_link::request_managed_cloud_release,
            project_sync::managed_cloud_link::list_managed_cloud_repositories,
            project_sync::managed_cloud_link::resolve_managed_cloud_repository,
            project_sync::managed_cloud_link::accept_managed_cloud_invitation,

            project_sync::commands::cancel_r2_storage_setup,
            project_sync::commands::discover_r2_storage,
            project_sync::commands::prepare_existing_r2_storage,
            project_sync::commands::get_host_identity_status,
            project_sync::commands::list_local_projects,
            project_sync::commands::recover_startup_state,
            project_sync::commands::get_project,
            project_sync::commands::list_project_repository_kinds,
            project_sync::commands::list_agent_providers,
            project_sync::commands::get_project_activity,
            project_sync::commands::refresh_project_activity,
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
            project_sync::commands::execute_provider_maintenance_action,
            project_sync::commands::save_project_binding,
            project_sync::commands::get_bundle_inventory,
            project_sync::commands::inspect_project_files,
            project_sync::commands::list_remote_bundle_snapshots,
            project_sync::commands::get_sibling_bundle_status,
            project_sync::commands::prepare_push_review,
            project_sync::commands::get_push_destination_status,
            project_sync::commands::push_bundle,
            project_sync::commands::finish_unfinished_push,
            project_sync::commands::plan_bundle_restore,
            project_sync::commands::apply_bundle_restore,
            project_sync::commands::get_restore_readiness,
            project_sync::commands::list_setup_drafts,
            project_sync::commands::probe_project_folder,
            project_sync::commands::list_agent_folders,
            project_sync::commands::create_setup_draft,
            project_sync::commands::get_setup_draft,
            project_sync::commands::update_setup_draft,
            project_sync::commands::discard_setup_draft,
            project_sync::commands::inspect_setup_draft,
            project_sync::commands::finalize_project_setup,
            project_sync::commands::list_storage_setup_drafts,
            project_sync::commands::create_storage_setup_draft,
            project_sync::commands::create_storage_edit_draft,
            project_sync::commands::get_storage_setup_draft,
            project_sync::commands::update_storage_setup_draft,
            project_sync::commands::discard_storage_setup_draft,
            project_sync::commands::test_storage_setup_draft,
            project_sync::commands::commit_storage_setup_draft,
            storage_catalog::resource_commands::list_catalog_capabilities,
            storage_catalog::resource_commands::list_catalog_resource_profiles,
            storage_catalog::resource_commands::save_catalog_resource_profile,
            storage_catalog::resource_commands::rename_catalog_resource_profile,
            storage_catalog::resource_commands::list_catalog_scopes,
            storage_catalog::resource_commands::create_catalog_scope,
            storage_catalog::resource_commands::delete_catalog_scope,
            storage_catalog::resource_commands::list_catalog_scope_resources,
            storage_catalog::resource_commands::list_catalog_storage_repositories,
            storage_catalog::resource_commands::get_catalog_resource_preview,
            storage_catalog::resource_commands::link_catalog_scope_storage,
            storage_catalog::resource_commands::unlink_catalog_scope_storage,
            storage_catalog::resource_commands::open_catalog_profile_in_app,
            storage_catalog::resource_commands::open_catalog_profile_in_terminal,
            storage_catalog::resource_commands::prepare_catalog_scope_push,
            storage_catalog::resource_commands::apply_catalog_scope_push,
            storage_catalog::resource_commands::finish_catalog_unfinished_push,
            storage_catalog::resource_commands::prepare_catalog_scope_pull,
            storage_catalog::resource_commands::apply_catalog_scope_pull,
            storage_catalog::resource_commands::prepare_catalog_resource_fork,
            storage_catalog::resource_commands::apply_catalog_resource_fork,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app_update::install_on_exit(app);
            }
        });
}
