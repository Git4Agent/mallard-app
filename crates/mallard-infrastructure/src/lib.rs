//! Mallard infrastructure adapters: local state persistence, object stores,
//! provider integrations, and platform mechanics.
//!
//! Extracted mechanically from the desktop crate by
//! `docs/PLAN_PROJECT_STRUCTURE_REORGANIZATION.md`; the module layout mirrors
//! the original file names so behavior and history stay traceable.

pub mod activity_log;
pub mod bundle_engine;
pub mod chat_history;
pub mod codex_app_refresh;
pub mod codex_binary;
pub mod codex_native_fork;
pub mod codex_rollout;
pub mod conversation_codec;
pub mod conversation_materialization;
pub mod conversation_sync;
pub mod persistence;
pub mod provider_capture;
pub mod s3_store;
