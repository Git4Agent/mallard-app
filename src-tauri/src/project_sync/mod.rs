//! Project-scoped synchronization with schema-1 local state and schema-1 bundles.
//!
//! Current state is isolated below `~/.mallard`. This module supports only the
//! current schema.

pub mod codex_maintenance;
pub mod commands;
pub mod config_watcher;
pub mod maintenance;
pub mod r2_setup;
pub mod dropbox_setup;
pub mod feishu_setup;
pub mod managed_cloud_link;
pub mod managed_cloud_setup;
pub mod service;
#[cfg_attr(test, allow(dead_code))]
pub mod session_watch_service;
