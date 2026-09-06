//! Codex post-Pull desktop restart.

use super::maintenance::{
    action, MaintenanceActionRequest, MaintenanceActionResult, MaintenanceExecutor, Registration,
};
use super::service::committed_conversation_receipt;
use mallard_domain::{BoundedConfigObject, ConversationOperationId, LocalProviderProfileId};
use mallard_infrastructure::codex_app_refresh::{
    refresh_offer as codex_app_refresh_offer, restart_after_pull, CodexAppKind,
    CodexAppRefreshState, CodexAppRestartResult,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

/// Get the exact Codex home that the pulled profile uses.
fn codex_home_for_profile(
    repository: &LocalStateRepository,
    profile_id: &LocalProviderProfileId,
) -> Result<PathBuf, String> {
    let machine = repository.load_bindings()?;
    let profile = machine
        .profiles
        .iter()
        .find(|profile| &profile.profile_id == profile_id)
        .ok_or_else(|| "the pulled Codex profile is no longer registered".to_string())?;
    Ok(PathBuf::from(&profile.canonical_path))
}

/// Restart the Codex desktop app after a committed Pull.
pub(super) fn restart_codex_app_after_pull_with_repository(
    repository: &LocalStateRepository,
    operation_id: ConversationOperationId,
    app_kind: CodexAppKind,
    confirmed: bool,
) -> Result<CodexAppRestartResult, String> {
    if !confirmed {
        return Err("confirm that active Codex chats are complete before restarting".to_string());
    }
    let receipt = committed_conversation_receipt(repository, &operation_id)?;
    let codex_home = codex_home_for_profile(repository, &receipt.provider_profile_id)?;
    let offer = codex_app_refresh_offer(operation_id, Some(&codex_home))
        .ok_or_else(|| "Codex App no longer needs or supports an automatic restart".to_string())?;
    if offer.state != CodexAppRefreshState::RestartAvailable || offer.app_kind != app_kind {
        return Err("Codex App restart capability changed after Pull".to_string());
    }
    Ok(match restart_after_pull(app_kind, &codex_home) {
        Ok(result) => result,
        Err(message) => CodexAppRestartResult {
            success: false,
            message,
        },
    })
}

/// Register the one Codex maintenance action.
pub(super) fn registration() -> Registration {
    Registration {
        executors: vec![(
            mallard_infrastructure::providers::codex::provider_key(),
            action("codex.app.restart"),
            Arc::new(RestartApp),
        )],
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartArguments {
    operation_id: String,
    app_kind: CodexAppKind,
}

struct RestartApp;

impl MaintenanceExecutor for RestartApp {
    fn execute(
        &self,
        repository: &LocalStateRepository,
        request: &MaintenanceActionRequest,
    ) -> Result<MaintenanceActionResult, String> {
        let arguments: RestartArguments = request.arguments.decode()?;
        let operation_id = ConversationOperationId::parse(arguments.operation_id)?;
        let restarted = restart_codex_app_after_pull_with_repository(
            repository,
            operation_id,
            arguments.app_kind,
            request.confirmed,
        )?;
        Ok(MaintenanceActionResult {
            provider: request.provider.clone(),
            action_id: request.action_id.clone(),
            ready: restarted.success,
            summary: restarted.message,
            facts: BoundedConfigObject::empty(),
        })
    }
}
