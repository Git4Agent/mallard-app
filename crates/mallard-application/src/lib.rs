//! Mallard application use cases shared by the Tauri and CLI adapters.
//!
//! This crate owns orchestration and typed adapter contracts. Domain values
//! remain in `mallard-domain`; persistence, remote stores, and platform
//! mechanics remain in `mallard-infrastructure`.

pub mod storage_setup;
