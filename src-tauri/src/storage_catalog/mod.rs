//! Draft-linked Library resources, independent of every project and project
//! repository bundle.
//!
//! Module boundaries follow `docs/PLAN_DRAFT_LINKED_CATALOG_RESOURCES.md`:
//! `domain` owns validated portable and plan types; `persistence` owns
//! machine-local state; `profiles` resolves provider homes and global skill
//! roots; `adapters` speaks the native plugin CLIs; `skill_tree` and
//! `codex_pet` validate and transform payload directories; `inventory`
//! assembles the local view; `resource_engine` owns the independent remote
//! repository protocol; `resource_planning` reconciles and pins scope-level
//! plans; `apply` executes approved mutations; `service` is the non-Tauri
//! application boundary shared by tests and `resource_commands`, the Tauri
//! surface.

pub mod adapters;
pub mod apply;
pub mod codex_pet;
pub mod domain;
pub mod inventory;
pub mod persistence;
pub mod pet_preview;
pub mod profiles;
pub mod resource_commands;
pub mod resource_engine;
pub mod resource_planning;
pub mod service;
pub mod skill_tree;

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
