//! Tauri adapters for project-scoped synchronization. The application
//! boundary is `mallard_application::project_sync`; this module holds the
//! command wrappers, the setup flows that drive a browser, and the watchers.

pub mod app_link;
pub mod cloud_project;
pub mod commands;
pub mod config_watcher;
pub mod dropbox_setup;
pub mod feishu_setup;
pub mod git_watcher;
pub mod managed_cloud_link;
pub mod managed_cloud_setup;
pub mod media_protocol;
pub mod provider_cli;
pub mod r2_setup;
