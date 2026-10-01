//! Mallard infrastructure adapters: local state persistence, object stores,
//! provider integrations, and platform mechanics.

mod path_text_boundary;

pub mod activity_log;
pub mod app_settings;
pub mod bounded_preparation;
mod bounded_process;
pub mod bundle_engine;
pub mod claude_history_tail;
#[cfg(target_os = "macos")]
mod codex_app;
pub mod codex_app_refresh;
pub mod codex_binary;
pub mod codex_history_graph;
pub mod codex_paginated_projection;
pub mod codex_rollout;
mod codex_rollout_selection;
pub mod codex_standalone_builder;
pub mod codex_subagents;
pub mod owned_subagents;
pub mod codex_thread_allocator;
pub mod conversation_codec;
pub mod conversation_materialization;
pub mod conversation_sync;
pub mod credential_redaction;
pub mod dropbox_auth;
pub mod dropbox_setup;
pub mod feishu_auth;
pub mod feishu_connections;
pub mod jsonl_tail_io;
pub mod loopback_browser;
pub mod managed_cloud;
pub mod payload;
pub mod persistence;
pub mod portable_resources;
pub mod project_activity;
pub mod provider_capture;
pub mod providers;
pub mod pull_transcript;
pub mod r2_setup;
pub mod session_detail;
pub mod session_path_map;
pub mod session_path_origins;
mod session_scan_memo;
pub mod storage_core;
mod storage_names;
pub mod storage_providers;
pub mod sync_metrics;
pub mod verified_transcript;
