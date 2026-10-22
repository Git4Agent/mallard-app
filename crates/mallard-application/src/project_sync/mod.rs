//! Project-scoped synchronization with schema-1 local state and schema-1 bundles.
//!
//! Current state is isolated below `~/.mallard`. This module supports only the
//! current schema. `service` is the application boundary shared by the desktop
//! and the command line; the Tauri command adapters live in the desktop crate.

pub mod app_link;
pub mod cloud_project;
pub mod codex_maintenance;
pub mod maintenance;
pub mod managed_cloud_link;
pub mod observation_cache;
pub mod operations;
pub mod provider_cli;
pub mod service;
