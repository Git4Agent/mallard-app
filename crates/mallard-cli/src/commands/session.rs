use std::collections::BTreeSet;

use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::{
    delete_local_project_session_with_repository, find_project_session_with_repository,
    get_project_session_detail_with_repository, get_project_session_event_detail_with_repository,
    get_project_session_timeline_page_with_repository, get_stored_session_detail_with_repository,
    get_stored_session_event_detail_with_repository,
    get_stored_session_timeline_page_with_repository, ProjectActivityRefreshEvent,
    ProjectSessionDetail, ProjectSessionTimelineRequest, SessionDetailError,
    SessionDetailErrorCode, SessionTimelineEventKind, SessionToolCategory,
};
use mallard_domain::{LocalProjectId, StorageId};
use mallard_infrastructure::project_activity::ProjectActivity;
use serde::Serialize;

use crate::args::{
    SessionArgs, SessionCommand, SessionDeleteAllArgs, SessionDeleteArgs, SessionEventArgs,
    SessionListArgs, SessionRef, SessionRunArgs, SessionTimelineArgs,
};
use crate::commands::push::resource_id;
use crate::context::CliContext;
use crate::error::{CliError, EXIT_CONFLICT, EXIT_INVALID_INPUT};
use crate::output::{human_time, CommandOutput};

pub(crate) fn run(context: &CliContext, args: SessionArgs) -> Result<CommandOutput, CliError> {
    let project = context.resolve_project(args.target.project.project.as_deref())?;
    // A storage is optional here: without one, local sessions alone.
    let storage = match args.target.storage.as_deref() {
        Some(selector) => Some(context.resolve_storage(Some(&project), Some(selector))?),
        None => None,
    };
    match args.command {
        SessionCommand::List(args) => list(context, project, storage, args),
        SessionCommand::Show(session) => show(context, project, storage, session),
        SessionCommand::Timeline(args) => timeline(context, project, storage, args),
        SessionCommand::Event(args) => event(context, project, storage, args),
        SessionCommand::Find(session) => find(context, project, session),
        SessionCommand::Run(args) => run_action(context, project, args),
        SessionCommand::Delete(args) => delete(context, project, args),
        SessionCommand::DeleteAll(args) => delete_all(context, project, args),
    }
}

fn detail_error(error: SessionDetailError) -> CliError {
    let message = error.message.clone();
    match error.code {
        SessionDetailErrorCode::InvalidRequest
        | SessionDetailErrorCode::NotFound
        | SessionDetailErrorCode::NotLocal => {
            debug_assert_eq!(EXIT_INVALID_INPUT, 2);
            CliError::invalid_with("session_not_found", message)
        }
        SessionDetailErrorCode::SnapshotChanged | SessionDetailErrorCode::OwnershipChanged => {
            debug_assert_eq!(EXIT_CONFLICT, 3);
            CliError::conflict("session_changed", message)
        }
        SessionDetailErrorCode::ProviderError | SessionDetailErrorCode::ResponseTooLarge => {
            CliError::from_service(message)
        }
    }
}

fn session_line(
    session: &mallard_infrastructure::project_activity::ProjectSessionSummary,
) -> String {
    format!(
        "{}  {:<7} {} [{}] — {} turns, {} tool calls{}\n",
        human_time(session.started_at),
        session.provider,
        if session.title.is_empty() {
            "(untitled)"
        } else {
            &session.title
        },
        session.resource_id,
        session.user_round_count,
        session.tool_call_count,
        if session.is_active { " · active" } else { "" }
    )
}

fn activity_text(activity: &ProjectActivity) -> String {
    let mut text = format!(
        "{} sessions between {} and {}\n",
        activity.sessions.len(),
        human_time(activity.window_start),
        human_time(activity.window_end)
    );
    for session in &activity.sessions {
        text.push_str(&session_line(session));
    }
    for warning in &activity.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }
    text
}

#[derive(Serialize)]
struct RefreshedActivity {
    activity: Option<ProjectActivity>,
    comparison: Option<mallard_application::project_sync::service::ThreadSyncComparison>,
    errors: Vec<String>,
}

fn list(
    context: &CliContext,
    project: LocalProjectId,
    storage: Option<StorageId>,
    args: SessionListArgs,
) -> Result<CommandOutput, CliError> {
    if args.refresh {
        let Some(storage) = storage else {
            return Err(CliError::invalid_code(
                "storage_required",
                "--refresh compares with a storage; pass --storage <id|name>.",
            ));
        };
        let branch = args.branch.clone();
        let refreshed = context.service(move |repository, sink| {
            let mut refreshed = RefreshedActivity {
                activity: None,
                comparison: None,
                errors: Vec::new(),
            };
            operations::refresh_project_activity(
                repository,
                sink,
                &project,
                &storage,
                branch.as_deref(),
                args.window_days,
                true,
                &mut |event| {
                    match event {
                        ProjectActivityRefreshEvent::Activity { activity, .. } => {
                            refreshed.activity = Some(activity)
                        }
                        ProjectActivityRefreshEvent::ActivityError { message, .. }
                        | ProjectActivityRefreshEvent::ComparisonError { message, .. } => {
                            refreshed.errors.push(message)
                        }
                        ProjectActivityRefreshEvent::Comparison { comparison } => {
                            refreshed.comparison = Some(comparison)
                        }
                    }
                    Ok(())
                },
            )?;
            Ok(refreshed)
        })?;
        let mut text = refreshed
            .activity
            .as_ref()
            .map(activity_text)
            .unwrap_or_default();
        if let Some(comparison) = &refreshed.comparison {
            text.push_str(&format!(
                "Storage {}: {} synced, {} local only, {} storage only, {} diverged\n",
                comparison.storage_name,
                comparison.counts.synced,
                comparison.counts.local,
                comparison.counts.storage,
                comparison.counts.diverged
            ));
        }
        for error in &refreshed.errors {
            text.push_str(&format!("warning: {error}\n"));
        }
        return CommandOutput::merged("ok", &refreshed, text);
    }
    let branch = args.branch.clone();
    let activity = context.service(move |repository, sink| {
        operations::get_project_activity(
            repository,
            sink,
            &project,
            storage.as_ref(),
            branch.as_deref(),
            args.before,
            args.window_days,
            false,
        )
    })?;
    let text = activity_text(&activity);
    CommandOutput::new("ok", "activity", &activity, text)
}

fn load_detail(
    context: &CliContext,
    project: &LocalProjectId,
    storage: Option<&StorageId>,
    resource_id: &str,
) -> Result<ProjectSessionDetail, CliError> {
    let (project, storage, resource_id) =
        (project.clone(), storage.cloned(), resource_id.to_string());
    context.blocking(move |repository, _| {
        match &storage {
            Some(storage) => get_stored_session_detail_with_repository(
                repository,
                &project,
                storage,
                &resource_id,
            ),
            None => get_project_session_detail_with_repository(repository, &project, &resource_id),
        }
        .map_err(detail_error)
    })
}

fn show(
    context: &CliContext,
    project: LocalProjectId,
    storage: Option<StorageId>,
    session: SessionRef,
) -> Result<CommandOutput, CliError> {
    let detail = load_detail(context, &project, storage.as_ref(), &session.resource_id)?;
    let mut text = format!(
        "{} [{}]\n  {} session {}{}\n  snapshot {}\n",
        if detail.title.is_empty() {
            "(untitled)"
        } else {
            &detail.title
        },
        detail.resource_id,
        detail.provider_display_name,
        detail.provider_session_id,
        if detail.is_active { " · active" } else { "" },
        detail.snapshot_id
    );
    if let (Some(started), Some(ended)) = (detail.started_at, detail.ended_at) {
        text.push_str(&format!(
            "  {} → {}\n",
            human_time(started),
            human_time(ended)
        ));
    }
    if let Some(summary) = &detail.summary {
        text.push_str(&format!("  {summary}\n"));
    }
    for action in &detail.actions {
        text.push_str(&format!(
            "  action {} — {}{}\n",
            action.action_id,
            action.label,
            if action.enabled {
                String::new()
            } else {
                format!(
                    " (unavailable{})",
                    action
                        .disabled_reason
                        .as_deref()
                        .map(|reason| format!(": {reason}"))
                        .unwrap_or_default()
                )
            }
        ));
    }
    for warning in &detail.warnings {
        text.push_str(&format!(
            "warning: {}\n",
            serde_json::to_string(warning).unwrap_or_default()
        ));
    }
    CommandOutput::new("ok", "session", &detail, text)
}

fn parse_set<T: serde::de::DeserializeOwned + Ord>(
    values: &[String],
    what: &str,
) -> Result<BTreeSet<T>, CliError> {
    values
        .iter()
        .map(|value| {
            serde_json::from_value(serde_json::Value::String(value.clone())).map_err(|_| {
                CliError::invalid_with(
                    "timeline_filter_invalid",
                    format!(
                        "'{value}' is not a {what}; see `mallard-cli session timeline --help`."
                    ),
                )
            })
        })
        .collect()
}

fn timeline(
    context: &CliContext,
    project: LocalProjectId,
    storage: Option<StorageId>,
    args: SessionTimelineArgs,
) -> Result<CommandOutput, CliError> {
    let snapshot_id = match args.snapshot_id {
        Some(snapshot_id) => snapshot_id,
        None => {
            load_detail(
                context,
                &project,
                storage.as_ref(),
                &args.session.resource_id,
            )?
            .snapshot_id
        }
    };
    let request = ProjectSessionTimelineRequest {
        snapshot_id,
        cursor: args.cursor,
        limit: args.limit,
        included_event_kinds: parse_set::<SessionTimelineEventKind>(
            &args.kinds,
            "timeline event kind",
        )?,
        included_tool_categories: parse_set::<SessionToolCategory>(&args.tools, "tool category")?,
        turn_id: args.turn,
    };
    let resource_id = args.session.resource_id;
    let page = context.blocking(move |repository, _| {
        match &storage {
            Some(storage) => get_stored_session_timeline_page_with_repository(
                repository,
                &project,
                storage,
                &resource_id,
                request,
            ),
            None => get_project_session_timeline_page_with_repository(
                repository,
                &project,
                &resource_id,
                request,
            ),
        }
        .map_err(detail_error)
    })?;
    let text = serde_json::to_string_pretty(&page).unwrap_or_default();
    CommandOutput::new("ok", "page", &page, text)
}

fn event(
    context: &CliContext,
    project: LocalProjectId,
    storage: Option<StorageId>,
    args: SessionEventArgs,
) -> Result<CommandOutput, CliError> {
    let snapshot_id = match args.snapshot_id {
        Some(snapshot_id) => snapshot_id,
        None => {
            load_detail(
                context,
                &project,
                storage.as_ref(),
                &args.session.resource_id,
            )?
            .snapshot_id
        }
    };
    let (resource_id, event_id) = (args.session.resource_id, args.event_id);
    let detail = context.blocking(move |repository, _| {
        match &storage {
            Some(storage) => get_stored_session_event_detail_with_repository(
                repository,
                &project,
                storage,
                &resource_id,
                &snapshot_id,
                &event_id,
            ),
            None => get_project_session_event_detail_with_repository(
                repository,
                &project,
                &resource_id,
                &snapshot_id,
                &event_id,
            ),
        }
        .map_err(detail_error)
    })?;
    let text = serde_json::to_string_pretty(&detail).unwrap_or_default();
    CommandOutput::new("ok", "event", &detail, text)
}

fn find(
    context: &CliContext,
    project: LocalProjectId,
    session: SessionRef,
) -> Result<CommandOutput, CliError> {
    let id = resource_id(&session.resource_id)?;
    let found = context.service(move |repository, _| {
        find_project_session_with_repository(repository, &project, &id)
    })?;
    let text = match &found {
        Some(local) => format!("The stored session is here as {local}."),
        None => "The project does not hold that stored session.".to_string(),
    };
    CommandOutput::new(
        if found.is_some() { "found" } else { "absent" },
        "resource_id",
        &found,
        text,
    )
}

fn run_action(
    context: &CliContext,
    project: LocalProjectId,
    args: SessionRunArgs,
) -> Result<CommandOutput, CliError> {
    let (resource_id, action_id) = (args.session.resource_id, args.action_id);
    context.service(move |repository, sink| {
        operations::execute_project_session_action(
            repository,
            sink,
            &project,
            &resource_id,
            &action_id,
        )
    })?;
    CommandOutput::new("ran", "ran", &true, "The action ran.".to_string())
}

fn delete(
    context: &CliContext,
    project: LocalProjectId,
    args: SessionDeleteArgs,
) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Deleting a session removes its local files. Pass --confirm.",
        ));
    }
    let resource_id = args.session.resource_id;
    let result = context.service(move |repository, _| {
        delete_local_project_session_with_repository(repository, &project, &resource_id)
    })?;
    let mut text = "Session deleted.".to_string();
    if let Some(error) = &result.provider_error {
        text.push_str(&format!("\nwarning: provider: {error}"));
    }
    if let Some(error) = &result.metadata_error {
        text.push_str(&format!("\nwarning: metadata: {error}"));
    }
    CommandOutput::new("deleted", "result", &result, text)
}

fn delete_all(
    context: &CliContext,
    project: LocalProjectId,
    args: SessionDeleteAllArgs,
) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Deleting every local session of the project removes their files. Pass --confirm.",
        ));
    }
    let expected_revision = args.expected_revision;
    let result = context.service(move |repository, sink| {
        operations::delete_local_project_sessions(repository, sink, &project, expected_revision)
    })?;
    let text = format!(
        "Deleted {} sessions, {} failed, {} provider issues.",
        result.sessions_deleted,
        result.sessions_failed,
        result.provider_errors.len()
    );
    CommandOutput::new("deleted", "result", &result, text)
}
