//! Project-scoped synchronization with schema-1 local state and schema-1 bundles.
//!
//! Current state is isolated below `~/.mallard`. This module supports only the
//! current schema.

pub mod codex_maintenance;
pub mod commands;
pub mod config_watcher;
pub mod maintenance;
pub mod service;
#[cfg_attr(test, allow(dead_code))]
pub mod session_watch_service;
