//! Project-scoped synchronization with schema-3 local state and schema-4 bundles.
//!
//! Local schema-3 state is isolated below the application-data `v3`
//! directory. Nothing in this module reads or rewrites schema-2 profile
//! configuration.

pub mod bundle_engine;
pub mod chat_history;
pub mod commands;
pub mod conversation_codec;
pub mod conversation_materialization;
pub mod conversation_sync;
pub mod domain;
pub mod global_inventory;
pub mod persistence;
pub mod provider_capture;
pub mod provider_fork;
pub mod s3_store;
