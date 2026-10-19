use std::collections::BTreeSet;
use std::io::Read;

use mallard_application::project_sync::operations;
use mallard_application::project_sync::service::get_restore_readiness_with_repository;
use mallard_domain::{ActionId, LocalProjectId, PullReview, PullScope, ResourceId, StorageId};

use crate::args::{PullApplyArgs, PullArgs, PullCommand, PullReviewArgs};
use crate::commands::push::resource_id;
use crate::context::CliContext;
use crate::error::CliError;
use crate::output::{human_bytes, CommandOutput};

pub(crate) fn run(context: &CliContext, args: PullArgs) -> Result<CommandOutput, CliError> {
    let project = context.resolve_project(args.target.project.project.as_deref())?;
    let storage = context.resolve_storage(Some(&project), args.target.storage.as_deref())?;
    match args.command {
        PullCommand::Review(args) => review(context, project, storage, args),
        PullCommand::Apply(args) => apply(context, project, storage, args),
        PullCommand::Readiness => readiness(context, project, storage),
    }
}

fn review_text(review: &PullReview) -> String {
    let mut text = format!(
        "Pull review — version {} of bundle {} from {}\n",
        review.revision, review.bundle_id, review.storage_id
    );
    let mut bytes = 0u64;
    for action in &review.actions {
        bytes += action.source_size.unwrap_or(0);
        text.push_str(&format!(
            "  {} {} — {} {}{}\n",
            if action.requires_explicit_approval {
                "[?]"
            } else {
                "[x]"
            },
            action.action_id,
            serde_json::to_value(&action.kind)
                .ok()
                .and_then(|value| value["kind"].as_str().map(ToOwned::to_owned))
                .unwrap_or_default(),
            action.resource_id,
            action
                .target_path
                .as_deref()
                .map(|path| format!(" → {path}"))
                .unwrap_or_default()
        ));
    }
    for row in &review.conversation_review {
        bytes += row.source_size.unwrap_or(0);
        let action = serde_json::to_value(row.action)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();
        match (&row.action_id, &row.blocked_reason) {
            (Some(action_id), _) => text.push_str(&format!(
                "  [x] {} — {} {}{}\n",
                action_id,
                action,
                row.display_title.as_deref().unwrap_or("(untitled session)"),
                if row.stored_without_credential_redaction {
                    " · stored without credential redaction"
                } else {
                    ""
                }
            )),
            (None, Some(reason)) => text.push_str(&format!(
                "  [ ] {} — {} · blocked: {reason}\n",
                row.resource_id, action
            )),
            (None, None) => text.push_str(&format!("  [ ] {} — {}\n", row.resource_id, action)),
        }
    }
    let applicable = applicable_action_count(review);
    text.push_str(&format!(
        "{applicable} actions; about {} to download. [x] needs no explicit approval, [?] does.\n",
        human_bytes(bytes)
    ));
    let explicit = review
        .actions
        .iter()
        .filter(|action| action.requires_explicit_approval)
        .count();
    if applicable == 0 {
        text.push_str(&nothing_to_pull(review));
        text.push('\n');
    } else if explicit == applicable {
        text.push_str(
            "Nothing was applied. Every action needs explicit approval: save the --json output and run `mallard-cli pull apply --review <file>` naming each with --approve <action-id>, or --resource <resource-id> for all of a resource's.\n",
        );
    } else {
        text.push_str(
            "Nothing was applied. Save the --json output and run `mallard-cli pull apply --review <file> --approve all`",
        );
        if explicit > 0 {
            text.push_str(
                "; the [?] actions are applied only when named with --approve <action-id> or --resource <resource-id>",
            );
        }
        text.push_str(".\n");
    }
    text
}

/// The actions an apply could take: every planned action plus each
/// conversation row that resolved to one. Blocked rows have none.
fn applicable_action_count(review: &PullReview) -> usize {
    review.actions.len()
        + review
            .conversation_review
            .iter()
            .filter(|row| row.action_id.is_some())
            .count()
}

/// Why an apply has nothing to do: the version adds nothing this project
/// lacks (a bundle registered but never pushed, or one already pulled), or
/// every candidate the review lists is blocked.
fn nothing_to_pull(review: &PullReview) -> String {
    let blocked = review
        .conversation_review
        .iter()
        .filter(|row| row.action_id.is_none())
        .count();
    if blocked == 0 {
        format!(
            "Nothing to pull: version {} holds nothing this project lacks.",
            review.revision
        )
    } else {
        format!(
            "Nothing to pull: the {blocked} candidate{} listed cannot be applied.",
            if blocked == 1 { "" } else { "s" }
        )
    }
}

fn review(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
    args: PullReviewArgs,
) -> Result<CommandOutput, CliError> {
    let binding = context.binding(&project)?;
    let link = context.link(&project, &storage)?;
    let scope = match args.session.as_deref() {
        Some(session) => PullScope::Session {
            resource_id: resource_id(session)?,
        },
        None => PullScope::Project,
    };
    let review = context.service(move |repository, sink| {
        operations::plan_bundle_restore(
            repository,
            sink,
            &storage,
            &link.bundle_id,
            &binding,
            &scope,
        )
    })?;
    let text = review_text(&review);
    CommandOutput::new("reviewed", "review", &review, text)
}

fn read_review(source: &str) -> Result<PullReview, CliError> {
    let mut text = String::new();
    if source == "-" {
        std::io::stdin().read_to_string(&mut text).map_err(|_| {
            CliError::invalid_code("review_unreadable", "Could not read the review from stdin.")
        })?;
    } else {
        text = std::fs::read_to_string(source).map_err(|_| {
            CliError::invalid_code(
                "review_unreadable",
                "Could not read the review file. Pass the path `pull review --json` was saved to, or `-` for stdin.",
            )
        })?;
    }
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| CliError::invalid_code("review_invalid", "The review file is not JSON."))?;
    // Either the whole `pull review --json` document or its `review` field.
    let review = match document.get("review") {
        Some(review) if document.get("schema").is_some() => review.clone(),
        _ => document,
    };
    serde_json::from_value(review).map_err(|_| {
        CliError::invalid_code(
            "review_invalid",
            "The review file is not a Pull review printed by `pull review --json`.",
        )
    })
}

fn apply(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
    args: PullApplyArgs,
) -> Result<CommandOutput, CliError> {
    let review = read_review(&args.review)?;
    let binding = context.binding(&project)?;
    if review.storage_id != storage {
        return Err(CliError::invalid_with(
            "review_mismatch",
            format!(
                "The review is for storage {}, not {}.",
                review.storage_id, storage
            ),
        ));
    }
    if review.binding_revision != binding.revision {
        return Err(CliError::conflict(
            "review_superseded",
            "The project's binding changed since this review. Review again.",
        ));
    }

    let mut approved: BTreeSet<ActionId> = BTreeSet::new();
    let mut approve_all = false;
    for value in &args.approve {
        if value == "all" {
            approve_all = true;
            continue;
        }
        let id = ActionId::parse(value.as_str()).map_err(|_| {
            CliError::invalid_with(
                "action_id_invalid",
                format!("'{value}' is not an action id from the review."),
            )
        })?;
        let known = review.actions.iter().any(|action| action.action_id == id)
            || review
                .conversation_review
                .iter()
                .any(|row| row.action_id.as_ref() == Some(&id));
        if !known {
            return Err(CliError::invalid_with(
                "action_not_in_review",
                format!("'{value}' is not an action of this review."),
            ));
        }
        approved.insert(id);
    }
    let resources: BTreeSet<ResourceId> = args
        .resource
        .iter()
        .map(|value| resource_id(value))
        .collect::<Result<_, _>>()?;
    for action in &review.actions {
        let by_resource = resources.contains(&action.resource_id);
        if by_resource || (approve_all && !action.requires_explicit_approval) {
            approved.insert(action.action_id.clone());
        }
    }
    for row in &review.conversation_review {
        if let Some(action_id) = &row.action_id {
            if approve_all || resources.contains(&row.resource_id) {
                approved.insert(action_id.clone());
            }
        }
    }
    if approved.is_empty() {
        return Err(if applicable_action_count(&review) == 0 {
            CliError::invalid_with("nothing_to_pull", nothing_to_pull(&review))
        } else {
            CliError::invalid_code(
                "nothing_approved",
                "The approval selects no action. The review holds only actions that need explicit approval; name them with --approve <id> or --resource <id>.",
            )
        });
    }
    let approved: Vec<ActionId> = approved.into_iter().collect();
    let count = approved.len();
    let result = context.service(move |repository, sink| {
        operations::apply_bundle_restore(repository, sink, review, approved)
    })?;
    let mut text = format!(
        "{} {} ({} of {} approved actions applied)\n",
        if result.success {
            "Pulled."
        } else {
            "Pull did not complete."
        },
        result.message,
        result.applied_action_ids.len(),
        count
    );
    for failure in &result.failed_actions {
        text.push_str(&format!(
            "  failed {}: {}\n",
            failure.action_id, failure.message
        ));
    }
    CommandOutput::new(
        if result.success {
            "pulled"
        } else {
            "incomplete"
        },
        "result",
        &result,
        text,
    )
}

fn readiness(
    context: &CliContext,
    project: LocalProjectId,
    storage: StorageId,
) -> Result<CommandOutput, CliError> {
    let readiness = context.service(move |repository, _| {
        get_restore_readiness_with_repository(repository, &project, &storage)
    })?;
    let mut text = format!("Bundle {}: {}\n", readiness.bundle_id, readiness.state);
    for issue in &readiness.issues {
        text.push_str(&format!(
            "  {} {}{}\n",
            issue.severity,
            issue.title,
            issue
                .detail
                .as_deref()
                .map(|detail| format!(" — {detail}"))
                .unwrap_or_default()
        ));
    }
    CommandOutput::new("ok", "readiness", &readiness, text)
}
