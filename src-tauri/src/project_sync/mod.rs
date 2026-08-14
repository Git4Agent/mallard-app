//! Project-scoped synchronization with schema-1 local state and schema-1 bundles.
//!
//! Current state is isolated below `~/.mallard`. This module has no migration
//! or compatibility path for pre-cutover state.

#[cfg_attr(test, allow(dead_code))]
pub mod codex_watch_service;
pub mod commands;
pub mod config_watcher;

// Temporary reorganization shims (PLAN_PROJECT_STRUCTURE_REORGANIZATION.md
// §3.2): these modules now live in the mallard-domain and
// mallard-infrastructure crates; the re-exports keep existing
// `super::` / `crate::project_sync::` paths compiling until Phase 5
// removes them.
pub use mallard_domain as domain;
pub use mallard_infrastructure::bundle_engine;
pub use mallard_infrastructure::codex_app_refresh;
pub use mallard_infrastructure::codex_history_graph;
pub use mallard_infrastructure::codex_rollout;
pub use mallard_infrastructure::codex_standalone_builder;
pub use mallard_infrastructure::codex_thread_allocator;
pub use mallard_infrastructure::conversation_codec;
pub use mallard_infrastructure::conversation_materialization;
pub use mallard_infrastructure::conversation_sync;
pub use mallard_infrastructure::persistence;
pub use mallard_infrastructure::project_activity;
pub use mallard_infrastructure::provider_capture;
pub use mallard_infrastructure::providers;
pub use mallard_infrastructure::session_detail;
pub use mallard_infrastructure::storage_core;
pub use mallard_infrastructure::sync_metrics;
