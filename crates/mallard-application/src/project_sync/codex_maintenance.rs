//! Codex app close and post-Pull restart.

use super::maintenance::{
    action, MaintenanceActionRequest, MaintenanceActionResult, MaintenanceExecutor, Registration,
};
use super::service::committed_conversation_receipt;
use mallard_domain::{BoundedConfigObject, ConversationOperationId, LocalProviderProfileId};
use mallard_infrastructure::codex_app_refresh::{
    refresh_offer as codex_app_refresh_offer, restart_after_pull, CodexAppKind,
    CodexAppRefreshState, CodexAppRestartOutcome, CodexAppRestartResult,
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
    Ok(restart_after_pull(app_kind, &codex_home).unwrap_or_else(CodexAppRestartResult::failed))
}

/// Register Codex app maintenance actions.
pub(super) fn registration() -> Registration {
    Registration {
        executors: vec![
            (
                mallard_infrastructure::providers::codex::provider_key(),
                action("codex.app.restart"),
                Arc::new(RestartApp),
            ),
            (
                mallard_infrastructure::providers::codex::provider_key(),
                action("codex.app.close"),
                Arc::new(CloseApp),
            ),
        ],
    }
}

struct CloseApp;

impl MaintenanceExecutor for CloseApp {
    fn execute(
        &self,
        _repository: &LocalStateRepository,
        request: &MaintenanceActionRequest,
    ) -> Result<MaintenanceActionResult, String> {
        mallard_infrastructure::codex_app_refresh::request_close()?;
        Ok(MaintenanceActionResult {
            provider: request.provider.clone(),
            action_id: request.action_id.clone(),
            ready: true,
            summary: "Close requested. Try Delete again after Codex closes.".into(),
            summary_code: Some("close_requested"),
            app_name: None,
            home_path: None,
            facts: BoundedConfigObject::empty(),
        })
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
        Ok(restart_result(request, restarted))
    }
}

/// A restart's result as every maintenance action reports one: its English,
/// and for a finished restart the code and values it was worded from.
fn restart_result(
    request: &MaintenanceActionRequest,
    restarted: CodexAppRestartResult,
) -> MaintenanceActionResult {
    MaintenanceActionResult {
        provider: request.provider.clone(),
        action_id: request.action_id.clone(),
        ready: restarted.success,
        summary: restarted.message,
        summary_code: restarted.outcome.map(|outcome| match outcome {
            CodexAppRestartOutcome::AlreadyClosed => "app_already_closed",
            CodexAppRestartOutcome::Restarted => "app_restarted",
            CodexAppRestartOutcome::RestartedWithHome => "app_restarted_with_home",
        }),
        app_name: restarted.app_name,
        home_path: restarted.home_path,
        facts: BoundedConfigObject::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_reports_its_code_and_values_and_a_failure_only_its_error() {
        let request = MaintenanceActionRequest {
            provider: mallard_infrastructure::providers::codex::provider_key(),
            action_id: action("codex.app.restart"),
            arguments: BoundedConfigObject::empty(),
            confirmed: true,
        };
        let done = restart_result(
            &request,
            CodexAppRestartResult {
                success: true,
                message: "Codex restarted with this project's Codex home (/work/.codex).".into(),
                outcome: Some(CodexAppRestartOutcome::RestartedWithHome),
                app_name: Some("Codex".into()),
                home_path: Some("/work/.codex".into()),
            },
        );
        assert!(done.ready);
        assert_eq!(done.summary_code, Some("app_restarted_with_home"));
        assert_eq!(done.app_name.as_deref(), Some("Codex"));
        assert_eq!(done.home_path.as_deref(), Some("/work/.codex"));
        let value = serde_json::to_value(&done).unwrap();
        assert_eq!(value["summary_code"], "app_restarted_with_home");
        let failed = restart_result(
            &request,
            CodexAppRestartResult::failed("Codex did not quit".into()),
        );
        assert!(!failed.ready);
        assert_eq!(failed.summary, "Codex did not quit");
        let value = serde_json::to_value(&failed).unwrap();
        assert!(value.get("summary_code").is_none() && value.get("app_name").is_none());
    }
}
