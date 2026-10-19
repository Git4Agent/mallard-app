//! Mallard application use cases shared by the Tauri and CLI adapters.
//!
//! This crate owns orchestration and typed adapter contracts. Domain values
//! remain in `mallard-domain`; persistence, remote stores, and platform
//! mechanics remain in `mallard-infrastructure`.

pub mod activity_log;
pub mod app_reset;
pub mod app_settings;
pub mod dropbox_setup;
pub mod events;
pub mod feishu_setup;
pub mod managed_cloud_setup;
pub mod project_sync;
pub mod storage_catalog;
pub mod storage_setup;

#[cfg(test)]
mod sync_tests;
