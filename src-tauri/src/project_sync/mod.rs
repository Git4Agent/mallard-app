//! Project-scoped synchronization with schema-1 local state and schema-1 bundles.
//!
//! Current state is isolated below `~/.mallard`. This module has no migration
//! or compatibility path for pre-cutover state.

pub mod bundle_engine;
pub mod chat_history;
pub mod codex_app_refresh;
pub mod codex_native_fork;
pub mod codex_rollout;
pub mod commands;
pub mod conversation_codec;
pub mod conversation_materialization;
pub mod conversation_sync;
pub mod domain;
pub mod persistence;
pub mod provider_capture;
pub mod s3_store;
