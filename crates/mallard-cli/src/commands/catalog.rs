//! The Library from the terminal. Every verb runs the same
//! `StorageCatalogService` the desktop runs; reviews are printed, and applied
//! in the same run when `--approve` says so, because a Library review lives
//! only as long as the process that built it.
use std::collections::BTreeMap;
use std::sync::Arc;

use mallard_application::events::SyncEventSink;
use mallard_application::storage_catalog::domain::{
    CatalogFormatKey, CatalogPlanActionId, CatalogPullDisposition, CatalogResourceId,
    CatalogResourceKindKey, CatalogScopeId, CatalogScopePullPlan, CatalogScopePushPlan,
    LocalCatalogProfileId,
};
use mallard_application::storage_catalog::service::{
    CatalogPullApproval, CatalogPushApproval, StorageCatalogService,
};
use mallard_domain::StorageId;
use mallard_infrastructure::managed_cloud::client::PublicSort;
use tokio::runtime::Handle;

use crate::args::{
    CatalogCloudCommand, CatalogCommand, CatalogForkArgs, CatalogItemCommand, CatalogItemRef,
    CatalogItemsArgs, CatalogLinkArgs, CatalogProfileCommand, CatalogPullArgs, CatalogPushArgs,
    CatalogRemoveArgs, CatalogScopeCommand, CatalogUnlinkArgs,
};
use crate::commands::provider::provider_key;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{human_time, CommandOutput};

pub(crate) fn run(
    context: &CliContext,
    command: CatalogCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        CatalogCommand::Capabilities => {
            let capabilities = with_service(context, |service| Ok(service.capabilities()))?;
            let mut text = String::new();
            for capability in &capabilities {
                text.push_str(&format!(
                    "{} — provider {} · kind {} · format {} v{}\n",
                    capability.label,
                    capability.provider,
                    capability.resource_kind,
                    capability.format,
                    capability.format_version
                ));
            }
            CommandOutput::new("ok", "capabilities", &capabilities, text)
        }
        CatalogCommand::Profile(args) => profile(context, args.command),
        CatalogCommand::Scope(args) => scope(context, args.command),
        CatalogCommand::Repositories(args) => {
            let storage = context.resolve_storage(None, Some(&args.storage))?;
            let repositories = with_service(context, move |service| {
                service.list_storage_repositories(&storage)
            })?;
            let mut text = String::new();
            if repositories.is_empty() {
                text.push_str("The storage holds no Library repositories.\n");
            }
            for repository in &repositories {
                text.push_str(&format!(
                    "{} — {} {} {} · {} items{}\n",
                    repository.repository_id,
                    repository.labels.provider_label,
                    repository.labels.kind_label,
                    repository.labels.format_label,
                    repository.resource_names.len(),
                    repository
                        .error
                        .as_deref()
                        .map(|error| format!(" · {error}"))
                        .unwrap_or_default()
                ));
            }
            CommandOutput::new("ok", "repositories", &repositories, text)
        }
        CatalogCommand::Link(args) => link(context, args),
        CatalogCommand::Unlink(args) => unlink(context, args),
        CatalogCommand::Items(args) => items(context, args),
        CatalogCommand::Item(args) => match args.command {
            CatalogItemCommand::Show(item) => item_show(context, item),
            CatalogItemCommand::File(args) => item_file(context, args.item, args.path),
        },
        CatalogCommand::Push(args) => push(context, args),
        CatalogCommand::Pull(args) => pull(context, args),
        CatalogCommand::Fork(args) => fork(context, args),
        CatalogCommand::Remove(args) => remove(context, args),
        CatalogCommand::FinishPush(args) => {
            let scope = scope_id(&args.scope.scope)?;
            let storage = context.resolve_storage(None, Some(&args.storage.storage))?;
            with_service(context, move |service| {
                service.finish_unfinished_push(&scope, &storage)
            })?;
            CommandOutput::new(
                "finished",
                "finished",
                &true,
                "The unfinished Library push's record was written.".to_string(),
            )
        }
        CatalogCommand::Cloud(args) => cloud(context, args.command),
    }
}

/// Run one closure over the service, built on the worker thread, with the
/// context's sink receiving the service's progress.
fn with_service<T: Send + 'static>(
    context: &CliContext,
    operation: impl FnOnce(&StorageCatalogService) -> Result<T, String> + Send + 'static,
) -> Result<T, CliError> {
    let sink: Arc<dyn SyncEventSink> = context.sink();
    context.service(move |_, _| {
        let service = StorageCatalogService::from_user_home()?
            .with_progress(move |progress| sink.catalog_progress(&progress));
        operation(&service)
    })
}

fn scope_id(value: &str) -> Result<CatalogScopeId, CliError> {
    CatalogScopeId::parse(value).map_err(|_| {
        CliError::invalid_with(
            "catalog_scope_id_invalid",
            "That is not a Catalog row id. Run `mallard-cli catalog scope list`.",
        )
    })
}

fn resource_id(value: &str) -> Result<CatalogResourceId, CliError> {
    CatalogResourceId::parse(value).map_err(|_| {
        CliError::invalid_with(
            "catalog_resource_id_invalid",
            "That is not a Library item id. Run `mallard-cli catalog items <scope>`.",
        )
    })
}

fn profile_id(value: &str) -> Result<LocalCatalogProfileId, CliError> {
    LocalCatalogProfileId::parse(value).map_err(|_| {
        CliError::invalid_with(
            "catalog_profile_id_invalid",
            "That is not a Library profile id. Run `mallard-cli catalog profile list`.",
        )
    })
}

fn profile(
    context: &CliContext,
    command: CatalogProfileCommand,
) -> Result<CommandOutput, CliError> {
    match command {
        CatalogProfileCommand::List => {
            let profiles = with_service(context, |service| service.list_profiles())?;
            let mut text = String::new();
            if profiles.is_empty() {
                text.push_str("No Library profiles. Run `mallard-cli catalog profile add --provider codex --path <home>`.\n");
            }
            for profile in &profiles {
                text.push_str(&format!(
                    "{} [{}] — {} home {} · revision {}\n",
                    profile.display_name,
                    profile.profile_id,
                    profile.provider_label,
                    profile.configured_home,
                    profile.revision
                ));
            }
            CommandOutput::new("ok", "profiles", &profiles, text)
        }
        CatalogProfileCommand::Add(args) => {
            let provider = provider_key(&args.provider)?;
            let path = args.path;
            let profile =
                with_service(context, move |service| service.save_profile(provider, path))?;
            CommandOutput::new(
                "added",
                "profile",
                &profile,
                format!("Saved {} [{}].", profile.display_name, profile.profile_id),
            )
        }
        CatalogProfileCommand::Probe(args) => {
            let provider = provider_key(&args.provider)?;
            let path = args.path;
            let probe = with_service(context, move |service| {
                service.probe_profile(provider, path)
            })?;
            let text = format!(
                "{} — {}{}",
                probe.canonical_path,
                if probe.readable {
                    "readable"
                } else {
                    "not readable"
                },
                probe
                    .existing_profile_id
                    .as_ref()
                    .map(|id| format!("; already saved as {id}"))
                    .unwrap_or_else(|| format!("; would be named {}", probe.suggested_name))
            );
            CommandOutput::new("ok", "probe", &probe, text)
        }
        CatalogProfileCommand::Rename(args) => {
            let id = profile_id(&args.profile.profile_id)?;
            let (name, revision) = (args.name, args.expected_revision);
            let profile = with_service(context, move |service| {
                service.rename_profile(&id, name, revision)
            })?;
            CommandOutput::new(
                "renamed",
                "profile",
                &profile,
                format!(
                    "Renamed to {} (revision {}).",
                    profile.display_name, profile.revision
                ),
            )
        }
        CatalogProfileCommand::Inspect(args) => {
            let id = profile_id(&args.profile_id)?;
            let inspection = with_service(context, move |service| service.inspect_profile(&id))?;
            let mut text = format!(
                "{} — {}\n",
                inspection.profile.display_name, inspection.profile.configured_home
            );
            for holding in &inspection.holdings {
                text.push_str(&format!(
                    "  {}: {}\n",
                    holding.capability.label,
                    serde_json::to_string(holding).unwrap_or_default()
                ));
            }
            CommandOutput::new("ok", "inspection", &inspection, text)
        }
    }
}

fn scope(context: &CliContext, command: CatalogScopeCommand) -> Result<CommandOutput, CliError> {
    match command {
        CatalogScopeCommand::List => {
            let scopes = with_service(context, |service| service.list_scopes())?;
            let mut text = String::new();
            if scopes.is_empty() {
                text.push_str("No Catalog rows. Run `mallard-cli catalog scope add`.\n");
            }
            for scope in &scopes {
                text.push_str(&format!(
                    "{} [{}] — {} · revision {}{}\n",
                    scope.label,
                    scope.scope.scope_id,
                    scope.profile.display_name,
                    scope.scope.revision,
                    scope
                        .unavailable_reason
                        .as_deref()
                        .map(|reason| format!(" · unavailable: {reason}"))
                        .unwrap_or_default()
                ));
            }
            CommandOutput::new("ok", "scopes", &scopes, text)
        }
        CatalogScopeCommand::Add(args) => {
            let provider = provider_key(&args.provider)?;
            let profile = profile_id(&args.profile)?;
            let kind = CatalogResourceKindKey::parse(args.kind.as_str()).map_err(|_| {
                CliError::invalid_with(
                    "catalog_kind_invalid",
                    "That is not a resource kind. Run `mallard-cli catalog capabilities`.",
                )
            })?;
            let format = CatalogFormatKey::parse(args.format.as_str()).map_err(|_| {
                CliError::invalid_with(
                    "catalog_format_invalid",
                    "That is not a format. Run `mallard-cli catalog capabilities`.",
                )
            })?;
            let version = args.format_version;
            let scope = with_service(context, move |service| {
                service.create_scope(provider, profile, kind, format, version)
            })?;
            CommandOutput::new(
                "added",
                "scope",
                &scope,
                format!("Added row {} [{}].", scope.label, scope.scope.scope_id),
            )
        }
        CatalogScopeCommand::Remove(args) => {
            if !args.confirm {
                return Err(CliError::invalid_code(
                    "confirmation_required",
                    "Removing a row forgets its links on this computer. Pass --confirm.",
                ));
            }
            let id = scope_id(&args.scope.scope)?;
            let revision = args.expected_revision;
            let removed =
                with_service(context, move |service| service.delete_scope(&id, revision))?;
            CommandOutput::new(
                if removed { "removed" } else { "unchanged" },
                "removed",
                &removed,
                if removed {
                    "Row removed."
                } else {
                    "No such row."
                }
                .to_string(),
            )
        }
    }
}

fn link(context: &CliContext, args: CatalogLinkArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let storage = context.resolve_storage(None, Some(&args.storage.storage))?;
    let outcome = with_service(context, move |service| {
        service.link_scope_storage(&scope, &storage)
    })?;
    let text = match outcome.completion.as_str() {
        "review_pull" => format!(
            "Linked (link {}). The storage holds items this computer has not reviewed: run `mallard-cli catalog pull`.",
            outcome.link_id
        ),
        _ => format!("Linked (link {}).", outcome.link_id),
    };
    CommandOutput::new("linked", "link", &outcome, text)
}

fn unlink(context: &CliContext, args: CatalogUnlinkArgs) -> Result<CommandOutput, CliError> {
    if !args.confirm {
        return Err(CliError::invalid_code(
            "confirmation_required",
            "Unlinking forgets this computer's reviewed state for that storage. Pass --confirm.",
        ));
    }
    let scope = scope_id(&args.scope.scope)?;
    let storage = context.resolve_storage(None, Some(&args.storage.storage))?;
    let removed = with_service(context, move |service| {
        let page = service.list_scope_resources(&scope, Some(&storage))?;
        let Some(link) = page.link else {
            return Err("the row is not linked to that storage".to_string());
        };
        service.unlink_scope_storage(&link.link_id, link.revision)
    })?;
    CommandOutput::new(
        if removed { "removed" } else { "unchanged" },
        "removed",
        &removed,
        if removed {
            "Unlinked. The stored items stay."
        } else {
            "No such link."
        }
        .to_string(),
    )
}

fn items(context: &CliContext, args: CatalogItemsArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let storage = match args.storage.as_deref() {
        Some(selector) => Some(context.resolve_storage(None, Some(selector))?),
        None => None,
    };
    let page = with_service(context, move |service| {
        service.list_scope_resources(&scope, storage.as_ref())
    })?;
    let mut text = format!(
        "{} {} — {}{}\n",
        page.labels.provider_label,
        page.labels.kind_label,
        page.profile.display_name,
        match (&page.storage_name, &page.link) {
            (Some(name), Some(link)) =>
                format!(" · storage {name} (link revision {})", link.revision),
            (Some(name), None) => format!(" · storage {name} (not linked)"),
            _ => String::new(),
        }
    );
    for row in &page.rows {
        text.push_str(&format!(
            "  {} [{}] — {}{}{}\n",
            row.display_name,
            row.resource_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| row.row_id.clone()),
            row.state
                .map(|state| serde_json::to_value(state).ok())
                .flatten()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| if row.local_present {
                    "local".to_string()
                } else {
                    "storage".to_string()
                }),
            row.local_modified_at
                .map(|at| format!(" · edited {}", human_time(at)))
                .unwrap_or_default(),
            row.scope_label
                .as_deref()
                .map(|label| format!(" · {label}"))
                .unwrap_or_default()
        ));
    }
    CommandOutput::new("ok", "items", &page, text)
}

fn item_show(context: &CliContext, item: CatalogItemRef) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&item.scope.scope)?;
    let resource = resource_id(&item.resource_id)?;
    let storage = match item.storage.as_deref() {
        Some(selector) => Some(context.resolve_storage(None, Some(selector))?),
        None => None,
    };
    let detail = with_service(context, move |service| {
        service.resource_detail(&scope, storage.as_ref(), &resource)
    })?;
    let text = serde_json::to_string_pretty(&detail).unwrap_or_default();
    CommandOutput::new("ok", "item", &detail, text)
}

fn item_file(
    context: &CliContext,
    item: CatalogItemRef,
    path: String,
) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&item.scope.scope)?;
    let resource = resource_id(&item.resource_id)?;
    let storage = match item.storage.as_deref() {
        Some(selector) => Some(context.resolve_storage(None, Some(selector))?),
        None => None,
    };
    let file = with_service(context, move |service| {
        service.resource_file(&scope, storage.as_ref(), &resource, &path)
    })?;
    let text = file
        .text
        .clone()
        .unwrap_or_else(|| format!("{} ({} bytes, not text)", file.path, file.size));
    CommandOutput::new("ok", "file", &file, text)
}

fn push_plan_text(plan: &CatalogScopePushPlan) -> String {
    let mut text = format!(
        "Library push review — {} items, storage {}\n",
        plan.rows.len(),
        plan.storage_id
    );
    for row in &plan.rows {
        let row = &row.row;
        text.push_str(&format!(
            "  {} {} [{}] — {}{}\n",
            if row.selectable && row.selected_by_default {
                "[x]"
            } else {
                "[ ]"
            },
            row.display_name,
            row.resource_id,
            serde_json::to_value(row.relation)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_default(),
            if row.held_for_pull {
                " · pull first"
            } else if !row.selectable {
                " · not pushable"
            } else {
                ""
            }
        ));
    }
    if !plan.warning_digests.is_empty() {
        text.push_str(&format!(
            "{} warning digest(s) need --acknowledge-warnings\n",
            plan.warning_digests.len()
        ));
    }
    text.push_str("Nothing was pushed. Add --approve to push the ticked items.\n");
    text
}

fn push(context: &CliContext, args: CatalogPushArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let storage = context.resolve_storage(None, Some(&args.storage.storage))?;
    let selected: Vec<CatalogResourceId> = args
        .select
        .iter()
        .map(|value| resource_id(value))
        .collect::<Result<_, _>>()?;
    let acknowledge_all = args.acknowledge_warnings.iter().any(|value| value == "all");
    let acknowledged: Vec<String> = args
        .acknowledge_warnings
        .iter()
        .filter(|value| *value != "all")
        .cloned()
        .collect();
    let approve = args.approve;
    let (plan, outcome) = with_service(context, move |service| {
        let plan = service.prepare_push(&scope, &storage)?;
        if !approve {
            return Ok((plan, None));
        }
        let selected_resource_ids: Vec<CatalogResourceId> = if selected.is_empty() {
            plan.rows
                .iter()
                .filter(|row| row.row.selectable && row.row.selected_by_default)
                .map(|row| row.row.resource_id.clone())
                .collect()
        } else {
            for id in &selected {
                let row = plan
                    .rows
                    .iter()
                    .find(|row| &row.row.resource_id == id)
                    .ok_or_else(|| format!("'{id}' is not in this review"))?;
                if !row.row.selectable {
                    return Err(format!("'{id}' cannot be pushed in this review"));
                }
            }
            selected
        };
        let acknowledged_warning_digests = if acknowledge_all {
            plan.warning_digests.clone()
        } else {
            acknowledged
        };
        let outcome = service.apply_push(CatalogPushApproval {
            plan_id: plan.plan_id.clone(),
            selected_resource_ids,
            acknowledged_warning_digests,
        })?;
        Ok((plan, Some(outcome)))
    })?;
    match outcome {
        None => {
            let text = push_plan_text(&plan);
            CommandOutput::new("reviewed", "plan", &plan, text)
        }
        Some(outcome) => {
            let mut text = format!("{}\n", outcome.message);
            for result in &outcome.results {
                text.push_str(&format!(
                    "  {} {}{}\n",
                    result.state,
                    result.display_name,
                    result
                        .message
                        .as_deref()
                        .map(|message| format!(" — {message}"))
                        .unwrap_or_default()
                ));
            }
            if let Some(stopped) = &outcome.stopped {
                text.push_str(&format!("stopped: {stopped}\n"));
            }
            CommandOutput::new(
                if outcome.published {
                    "pushed"
                } else {
                    "incomplete"
                },
                "outcome",
                &outcome,
                text,
            )
        }
    }
}

fn pull_plan_text(plan: &CatalogScopePullPlan) -> String {
    let mut text = format!(
        "Library pull review — {} items, storage {}\n",
        plan.items.len(),
        plan.storage_id
    );
    for item in &plan.items {
        let disposition = match &item.item.disposition {
            CatalogPullDisposition::Executable => "ready".to_string(),
            CatalogPullDisposition::AlreadyMatching => "already matching".to_string(),
            CatalogPullDisposition::KeptLocal => "kept local".to_string(),
            CatalogPullDisposition::Manual { message } => format!("manual: {message}"),
            CatalogPullDisposition::Blocked { reason } => format!("blocked: {reason}"),
        };
        text.push_str(&format!(
            "  {} [{}] — {}\n",
            item.item.display_name, item.item.resource_id, disposition
        ));
        for action in &item.item.actions {
            text.push_str(&format!(
                "      {} {} {}{}\n",
                if Some(&action.action_id) == item.item.default_action_id.as_ref() {
                    "*"
                } else {
                    " "
                },
                action.action_id,
                serde_json::to_value(&action.kind)
                    .ok()
                    .map(|value| match value {
                        serde_json::Value::String(kind) => kind,
                        other => other
                            .as_object()
                            .and_then(|object| object.keys().next().cloned())
                            .unwrap_or_default(),
                    })
                    .unwrap_or_default(),
                if action.requires_destructive_approval {
                    " · replaces local content; needs --acknowledge-destructive"
                } else {
                    ""
                }
            ));
        }
    }
    text.push_str("Nothing was pulled. Add --approve to apply each item's default (*) action.\n");
    text
}

fn pairs(values: &[String], what: &str) -> Result<Vec<(CatalogResourceId, String)>, CliError> {
    values
        .iter()
        .map(|value| {
            let (id, rest) = value.split_once('=').ok_or_else(|| {
                CliError::invalid_with(
                    "catalog_pair_invalid",
                    format!("'{value}' must be RESOURCE_ID={what}."),
                )
            })?;
            Ok((resource_id(id)?, rest.to_string()))
        })
        .collect()
}

fn pull(context: &CliContext, args: CatalogPullArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let storage = context.resolve_storage(None, Some(&args.storage.storage))?;
    let chosen = pairs(&args.choose, "ACTION_ID")?;
    let copy_names: BTreeMap<CatalogResourceId, String> =
        pairs(&args.copy_name, "NAME")?.into_iter().collect();
    let destructive_all = args
        .acknowledge_destructive
        .iter()
        .any(|value| value == "all");
    let destructive: Vec<CatalogResourceId> = args
        .acknowledge_destructive
        .iter()
        .filter(|value| *value != "all")
        .map(|value| resource_id(value))
        .collect::<Result<_, _>>()?;
    let approve = args.approve;
    let (plan, outcome) = with_service(context, move |service| {
        let plan = service.prepare_pull(&scope, &storage, copy_names)?;
        if !approve {
            return Ok((plan, None));
        }
        let mut choices: BTreeMap<CatalogResourceId, CatalogPlanActionId> = BTreeMap::new();
        for item in &plan.items {
            if item.item.disposition != CatalogPullDisposition::Executable {
                continue;
            }
            let chosen_action = chosen
                .iter()
                .find(|(id, _)| id == &item.item.resource_id)
                .map(|(_, action)| {
                    CatalogPlanActionId::parse(action.as_str())
                        .map_err(|_| format!("'{action}' is not an action id"))
                })
                .transpose()?;
            let action_id = match chosen_action.or_else(|| item.item.default_action_id.clone()) {
                Some(action_id) => action_id,
                None => continue,
            };
            if !item
                .item
                .actions
                .iter()
                .any(|action| action.action_id == action_id)
            {
                return Err(format!(
                    "'{action_id}' is not an action of item '{}'",
                    item.item.resource_id
                ));
            }
            choices.insert(item.item.resource_id.clone(), action_id);
        }
        if choices.is_empty() {
            return Err("the review holds nothing to pull".to_string());
        }
        let destructive_acknowledged: Vec<CatalogResourceId> = if destructive_all {
            plan.items
                .iter()
                .filter(|item| {
                    choices.get(&item.item.resource_id).is_some_and(|chosen| {
                        item.item.actions.iter().any(|action| {
                            &action.action_id == chosen && action.requires_destructive_approval
                        })
                    })
                })
                .map(|item| item.item.resource_id.clone())
                .collect()
        } else {
            destructive
        };
        let outcome = service.apply_pull(CatalogPullApproval {
            plan_id: plan.plan_id.clone(),
            choices,
            destructive_acknowledged,
        })?;
        Ok((plan, Some(outcome)))
    })?;
    match outcome {
        None => {
            let text = pull_plan_text(&plan);
            CommandOutput::new("reviewed", "plan", &plan, text)
        }
        Some(outcome) => {
            let mut text = String::new();
            for result in &outcome.results {
                text.push_str(&format!(
                    "  {} {}{}\n",
                    serde_json::to_value(result.outcome)
                        .ok()
                        .and_then(|value| value.as_str().map(ToOwned::to_owned))
                        .unwrap_or_default(),
                    result.display_name,
                    result
                        .message
                        .as_deref()
                        .map(|message| format!(" — {message}"))
                        .unwrap_or_default()
                ));
            }
            if let Some(stopped) = &outcome.stopped {
                text.push_str(&format!("stopped: {stopped}\n"));
            }
            CommandOutput::new(
                if outcome.remote_accepted {
                    "pulled"
                } else {
                    "incomplete"
                },
                "outcome",
                &outcome,
                text,
            )
        }
    }
}

fn fork(context: &CliContext, args: CatalogForkArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let resource = resource_id(&args.resource_id)?;
    let (name, approve) = (args.name, args.approve);
    let (plan, result) = with_service(context, move |service| {
        let plan = service.prepare_fork(&scope, &resource, &name)?;
        if !approve {
            return Ok((plan, None));
        }
        let result = service.apply_fork(&scope, &plan.plan_id)?;
        Ok((plan, Some(result)))
    })?;
    match result {
        None => {
            let text = format!(
                "Fork review — copy {} [{}] as {} [{}]. Nothing was copied; add --approve.",
                plan.source_display_name,
                plan.source_resource_id,
                plan.copy_name,
                plan.copy_resource_id
            );
            CommandOutput::new("reviewed", "plan", &plan, text)
        }
        Some(result) => {
            let text = format!(
                "{} {}{}",
                serde_json::to_value(result.outcome)
                    .ok()
                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                    .unwrap_or_default(),
                result.display_name,
                result
                    .message
                    .as_deref()
                    .map(|message| format!(" — {message}"))
                    .unwrap_or_default()
            );
            CommandOutput::new("forked", "result", &result, text)
        }
    }
}

fn remove(context: &CliContext, args: CatalogRemoveArgs) -> Result<CommandOutput, CliError> {
    let scope = scope_id(&args.scope.scope)?;
    let resource = resource_id(&args.resource_id)?;
    let approve = args.approve;
    let (plan, result) = with_service(context, move |service| {
        let plan = service.prepare_removal(&scope, &resource)?;
        if !approve {
            return Ok((plan, None));
        }
        let result = service.apply_removal(&plan.plan_id)?;
        Ok((plan, Some(result)))
    })?;
    match result {
        None => {
            let text = format!(
                "Removal review — {} {} [{}], {} file(s). Nothing was removed; add --approve.",
                if plan.uninstall {
                    "uninstall"
                } else {
                    "delete"
                },
                plan.display_name,
                plan.resource_id,
                plan.file_count
            );
            CommandOutput::new("reviewed", "plan", &plan, text)
        }
        Some(result) => {
            let text = result.message.clone();
            CommandOutput::new(
                if result.removed {
                    "removed"
                } else {
                    "incomplete"
                },
                "result",
                &result,
                text,
            )
        }
    }
}

fn block_on<T>(future: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    Handle::current().block_on(future)
}

fn cloud(context: &CliContext, command: CatalogCloudCommand) -> Result<CommandOutput, CliError> {
    let storage_of = |selector: &str| context.resolve_storage(None, Some(selector));
    match command {
        CatalogCloudCommand::Status(args) => {
            let scope = scope_id(&args.item.scope.scope)?;
            let resource = resource_id(&args.item.resource_id)?;
            let storage = storage_of(&args.item.storage.storage)?;
            let repository = args.repository_id;
            let status = with_service(context, move |service| {
                block_on(service.cloud_status(&scope, &storage, &resource, &repository))
            })?;
            let text = serde_json::to_string_pretty(&status).unwrap_or_default();
            CommandOutput::new("ok", "cloud_status", &status, text)
        }
        CatalogCloudCommand::Repositories(args) => {
            let scope = scope_id(&args.scope.scope)?;
            let resource = resource_id(&args.resource_id)?;
            let storage = storage_of(&args.storage.storage)?;
            let repositories = with_service(context, move |service| {
                block_on(service.list_cloud_repositories(&scope, &storage, &resource))
            })?;
            let text = cloud_repositories_text(&repositories);
            CommandOutput::new("ok", "repositories", &repositories, text)
        }
        CatalogCloudCommand::Shared(args) => {
            let storage = storage_of(&args.storage)?;
            let repositories = with_service(context, move |service| {
                block_on(service.list_cloud_shared(&storage))
            })?;
            let text = cloud_repositories_text(&repositories);
            CommandOutput::new("ok", "repositories", &repositories, text)
        }
        CatalogCloudCommand::Starred(args) => {
            let storage = storage_of(&args.storage.storage)?;
            let scope = args.scope.as_deref().map(scope_id).transpose()?;
            let cursor = args.cursor;
            let listing = with_service(context, move |service| {
                block_on(service.list_cloud_starred(scope.as_ref(), &storage, cursor.as_deref()))
            })?;
            let text = serde_json::to_string_pretty(&listing).unwrap_or_default();
            CommandOutput::new("ok", "listing", &listing, text)
        }
        CatalogCloudCommand::Public(args) => {
            let storage = storage_of(&args.listing.storage.storage)?;
            let scope = args.listing.scope.as_deref().map(scope_id).transpose()?;
            let sort = PublicSort::parse(&args.sort).ok_or_else(|| {
                CliError::invalid_code("cloud_sort_invalid", "--sort must be newest or stars.")
            })?;
            let cursor = args.listing.cursor;
            let listing = with_service(context, move |service| {
                block_on(service.list_cloud_public(
                    scope.as_ref(),
                    &storage,
                    sort,
                    cursor.as_deref(),
                ))
            })?;
            let text = serde_json::to_string_pretty(&listing).unwrap_or_default();
            CommandOutput::new("ok", "listing", &listing, text)
        }
        CatalogCloudCommand::Resolve(args) => {
            let storage = storage_of(&args.storage.storage)?;
            let address = args.address;
            let resolution = with_service(context, move |service| {
                block_on(service.resolve_cloud_repository(&storage, &address))
            })?;
            let text = serde_json::to_string_pretty(&resolution).unwrap_or_default();
            CommandOutput::new("ok", "resolution", &resolution, text)
        }
        CatalogCloudCommand::Find(args) => {
            let storage = storage_of(&args.storage.storage)?;
            let repository = args.repository_id;
            let place = with_service(context, move |service| {
                service.find_cloud_repository(&storage, &repository)
            })?;
            let text = match &place {
                Some(place) => format!(
                    "Row {} holds it as item {}.",
                    place.scope_id, place.resource_id
                ),
                None => "No row holds that repository.".to_string(),
            };
            CommandOutput::new(
                if place.is_some() { "found" } else { "absent" },
                "place",
                &place,
                text,
            )
        }
        CatalogCloudCommand::Join(args) => {
            let storage = storage_of(&args.storage.storage)?;
            let code = args.code;
            let resolution = with_service(context, move |service| {
                block_on(service.join_cloud_repository(&storage, &code))
            })?;
            let text = serde_json::to_string_pretty(&resolution).unwrap_or_default();
            CommandOutput::new("joined", "resolution", &resolution, text)
        }
        CatalogCloudCommand::Connect(args) => {
            let scope = scope_id(&args.item.scope.scope)?;
            let resource = resource_id(&args.item.resource_id)?;
            let storage = storage_of(&args.item.storage.storage)?;
            let (repository, revision) = (args.repository_id, args.expected_link_revision);
            with_service(context, move |service| {
                block_on(service.connect_cloud_repository(
                    &scope,
                    &storage,
                    &resource,
                    &repository,
                    revision,
                ))
            })?;
            CommandOutput::new("connected", "connected", &true, "Connected.".to_string())
        }
        CatalogCloudCommand::Create(args) => {
            let scope = scope_id(&args.item.scope.scope)?;
            let resource = resource_id(&args.item.resource_id)?;
            let storage = storage_of(&args.item.storage.storage)?;
            let revision = args.expected_link_revision;
            let repository = with_service(context, move |service| {
                block_on(service.create_cloud_repository(&scope, &storage, &resource, revision))
            })?;
            let text = format!(
                "Created {} ({}).",
                repository.address, repository.website_url
            );
            CommandOutput::new("created", "repository", &repository, text)
        }
        CatalogCloudCommand::DiscardDraft(args) => {
            let scope = scope_id(&args.item.scope.scope)?;
            let resource = resource_id(&args.item.resource_id)?;
            let storage = storage_of(&args.item.storage.storage)?;
            let (repository, revision) = (args.repository_id, args.expected_revision);
            let discarded = with_service(context, move |service| {
                block_on(service.discard_cloud_draft(
                    &scope,
                    &storage,
                    &resource,
                    &repository,
                    revision,
                ))
            })?;
            let text = serde_json::to_string_pretty(&discarded).unwrap_or_default();
            CommandOutput::new("discarded", "discarded", &discarded, text)
        }
    }
}

fn cloud_repositories_text(
    repositories: &[mallard_application::storage_catalog::service::cloud::CatalogCloudRepository],
) -> String {
    let mut text = String::new();
    if repositories.is_empty() {
        text.push_str("No repositories.\n");
    }
    for repository in repositories {
        text.push_str(&format!(
            "{} [{}] — {}{}\n",
            repository.address,
            repository.repository_id,
            repository.visibility,
            if repository.can_push {
                " · can push"
            } else {
                ""
            }
        ));
    }
    text
}

#[allow(dead_code)]
fn storage_text(storage: &StorageId) -> String {
    storage.to_string()
}
