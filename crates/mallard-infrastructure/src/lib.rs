//! Mallard infrastructure adapters: local state persistence, object stores,
//! provider integrations, and platform mechanics.
//!
//! Extracted mechanically from the desktop crate by
//! `docs/PLAN_PROJECT_STRUCTURE_REORGANIZATION.md`; the module layout mirrors
//! the original file names so behavior and history stay traceable.

mod path_text_boundary;

pub mod activity_log;
pub mod app_settings;
pub mod bundle_engine;
pub mod claude_history_tail;
pub mod codex_app_refresh;
pub mod codex_binary;
pub mod codex_history_graph;
pub mod codex_paginated_projection;
pub mod codex_rollout;
pub mod codex_standalone_builder;
pub mod codex_thread_allocator;
pub mod conversation_codec;
pub mod conversation_materialization;
pub mod conversation_sync;
pub mod credential_redaction;
pub mod jsonl_tail_io;
pub mod persistence;
pub mod project_activity;
pub mod provider_capture;
pub mod providers;
pub mod r2_setup_journal;
pub mod session_detail;
pub mod session_path_map;
pub mod storage_core;
pub mod sync_metrics;
