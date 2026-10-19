use mallard_infrastructure::activity_log::{
    ActivityLogCleanupRequest, ActivityLogLevel, ActivityLogQuery, ActivityLogStore,
    ActivityLogType,
};
use mallard_infrastructure::persistence::LocalStateRepository;

use crate::args::{LogCleanupArgs, LogCommand, LogPolicyArgs, LogQueryArgs};
use crate::context::CliContext;
use crate::error::CliError;
use crate::events::text_line;
use crate::output::{human_bytes, human_time, CommandOutput};

pub(crate) fn run(context: &CliContext, command: LogCommand) -> Result<CommandOutput, CliError> {
    match command {
        LogCommand::Query(args) => query(context, args),
        LogCommand::Stats => {
            let stats = context.service(|repository, _| store(repository)?.stats())?;
            let text = stats_text(&stats);
            CommandOutput::new("ok", "stats", &stats, text)
        }
        LogCommand::Policy(args) => policy(context, args),
        LogCommand::Cleanup(args) => cleanup(context, args),
        LogCommand::Folder => {
            let folder = context.service(|repository, _| {
                store(repository)?
                    .root()
                    .to_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| "activity log folder is not valid UTF-8".to_string())
            })?;
            CommandOutput::new("ok", "folder", &folder, folder.clone())
        }
    }
}

fn store(repository: &LocalStateRepository) -> Result<ActivityLogStore, String> {
    ActivityLogStore::from_repository(repository)
}

fn parse_keys<T: serde::de::DeserializeOwned>(
    values: &[String],
    what: &str,
) -> Result<Vec<T>, CliError> {
    values
        .iter()
        .map(|value| {
            serde_json::from_value(serde_json::Value::String(value.clone())).map_err(|_| {
                CliError::invalid_with(
                    "log_filter_invalid",
                    format!("'{value}' is not a {what}; see `mallard-cli log query --help`."),
                )
            })
        })
        .collect()
}

fn query(context: &CliContext, args: LogQueryArgs) -> Result<CommandOutput, CliError> {
    let query = ActivityLogQuery {
        types: parse_keys::<ActivityLogType>(&args.types, "log type")?,
        levels: parse_keys::<ActivityLogLevel>(&args.levels, "log level")?,
        search: args.search,
        cursor: args.cursor,
        limit: args.limit,
    };
    let page = context.service(move |repository, _| store(repository)?.query(&query))?;
    let mut text = String::new();
    for entry in &page.entries {
        text.push_str(&format!(
            "{}  {}  {}\n",
            human_time(entry.ts / 1000),
            entry.event,
            text_line(entry)
        ));
    }
    if let Some(cursor) = &page.next_cursor {
        text.push_str(&format!("More: --cursor {cursor}\n"));
    }
    CommandOutput::new("ok", "page", &page, text)
}

fn stats_text(stats: &mallard_infrastructure::activity_log::ActivityLogStats) -> String {
    format!(
        "{} in {} file{}{}\nRetention: {} days, at most {}",
        human_bytes(stats.total_bytes),
        stats.file_count,
        if stats.file_count == 1 { "" } else { "s" },
        match (stats.oldest_ts, stats.newest_ts) {
            (Some(oldest), Some(newest)) => format!(
                ", from {} to {}",
                human_time(oldest / 1000),
                human_time(newest / 1000)
            ),
            _ => String::new(),
        },
        stats.policy.retention_days,
        human_bytes(stats.policy.max_total_bytes)
    )
}

fn policy(context: &CliContext, args: LogPolicyArgs) -> Result<CommandOutput, CliError> {
    if args.retention_days.is_none() && args.max_total_bytes.is_none() {
        return Err(CliError::invalid_code(
            "log_policy_unchanged",
            "Pass --retention-days and/or --max-total-bytes.",
        ));
    }
    let stats = context.service(move |repository, _| {
        let store = store(repository)?;
        let mut policy = store.stats()?.policy;
        if let Some(days) = args.retention_days {
            policy.retention_days = days;
        }
        if let Some(bytes) = args.max_total_bytes {
            policy.max_total_bytes = bytes;
        }
        store.save_policy(&policy)?;
        store.stats()
    })?;
    let text = stats_text(&stats);
    CommandOutput::new("saved", "stats", &stats, text)
}

fn cleanup(context: &CliContext, args: LogCleanupArgs) -> Result<CommandOutput, CliError> {
    if !args.all && args.older_than_days.is_none() {
        return Err(CliError::invalid_code(
            "log_cleanup_scope_required",
            "Pass --all or --older-than-days <n>.",
        ));
    }
    let request = ActivityLogCleanupRequest {
        delete_all: args.all,
        older_than_days: args.older_than_days,
    };
    let result = context.service(move |repository, _| store(repository)?.cleanup(&request))?;
    let text = format!(
        "Removed {} file{}, reclaimed {}.\n{}",
        result.removed_files,
        if result.removed_files == 1 { "" } else { "s" },
        human_bytes(result.reclaimed_bytes),
        stats_text(&result.stats)
    );
    CommandOutput::new("cleaned", "cleanup", &result, text)
}
