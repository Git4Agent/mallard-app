//! Draft-linked Library resources, independent of every project and project
//! repository bundle.
//!
//! `domain` owns validated portable and plan types; `registry` is the
//! compile-time table of (provider, kind, format, version) capabilities;
//! `codec` is the format-codec contract with `agent_skill`, `plugin_intent`,
//! and `codex_pet` as its registered codecs; `adapters` are the provider
//! adapters (target roots, launch, native inventory); `persistence` owns
//! machine-local state; `plan_store` holds pending reviewed plans in memory;
//! `profiles` resolves provider homes and their target roots; `inventory`
//! assembles the local view; `resource_engine` owns the independent remote
//! repository protocol; `resource_planning` reconciles and pins scope-level
//! plans; `apply` executes approved mutations; `service` is the non-Tauri
//! application boundary shared by tests and `resource_commands`, the Tauri
//! surface.

pub mod adapters;
pub mod agent_skill;
pub mod apply;
pub mod codec;
mod codex_inventory;
pub mod codex_pet;
pub mod domain;
pub mod inventory;
pub mod persistence;
pub mod pet_preview;
pub mod plan_store;
pub mod plugin_intent;
mod plugin_listing;
pub mod profiles;
mod provider_state;
pub mod registry;
pub mod resource_commands;
pub mod resource_engine;
mod removal_files;
pub mod resource_planning;
pub mod service;

/// Cross-feature deletion guard over the shared storage registry. This is the
/// only project-sync integration point: it exposes no Library DTO and never
/// reads project state.
pub fn ensure_storage_removal_allowed(
    storage_id: &mallard_domain::StorageId,
) -> Result<(), String> {
    let repository = persistence::CatalogStateRepository::from_user_home()?;
    if let Some(reason) = repository.storage_reference_reason(storage_id)? {
        return Err(format!(
            "storage '{}' cannot be removed because {}; unlink or discard it in Library first",
            storage_id, reason
        ));
    }
    Ok(())
}
