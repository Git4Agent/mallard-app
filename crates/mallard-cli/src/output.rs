use serde::Serialize;
use std::io::Write;

use crate::backend::{SetupOutcome, StorageSummary, VerifyOutcome};
use crate::error::CliError;

#[derive(Serialize)]
struct JsonSetup {
    schema: u32,
    status: &'static str,
    reconnected: bool,
}

#[derive(Serialize)]
struct JsonList<'a> {
    schema: u32,
    status: &'static str,
    storages: &'a [StorageSummary],
}

#[derive(Serialize)]
struct JsonVerify<'a> {
    schema: u32,
    status: &'static str,
    storage_id: &'a str,
    config_revision: u64,
    verified: bool,
}

#[derive(Serialize)]
struct JsonError<'a> {
    schema: u32,
    status: &'static str,
    exit_code: i32,
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    setup_id: Option<&'a str>,
}

pub(crate) fn setup(
    writer: &mut dyn Write,
    json: bool,
    outcome: &SetupOutcome,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &JsonSetup {
                schema: 1,
                status: "draft_ready",
                reconnected: outcome.reconnected,
            },
        )
    } else if outcome.reconnected {
        writeln!(
            writer,
            "Access regained for the existing storage. Review the connection in Mallard, optionally test it, then select Save changes."
        )
    } else {
        writeln!(
            writer,
            "Storage settings are ready for review in Mallard. Review the connection, optionally test it, then select Finish setup."
        )
    }
}

pub(crate) fn list(
    writer: &mut dyn Write,
    json: bool,
    storages: &[StorageSummary],
) -> std::io::Result<()> {
    if json {
        return write_json(
            writer,
            &JsonList {
                schema: 1,
                status: "ok",
                storages,
            },
        );
    }
    if storages.is_empty() {
        return writeln!(writer, "No Mallard storages configured.");
    }
    for storage in storages {
        writeln!(
            writer,
            "{} [{}] — {}",
            storage.name, storage.storage_id, storage.provider_label
        )?;
    }
    Ok(())
}

pub(crate) fn verify(
    writer: &mut dyn Write,
    json: bool,
    outcome: &VerifyOutcome,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &JsonVerify {
                schema: 1,
                status: "verified",
                storage_id: &outcome.storage_id,
                config_revision: outcome.config_revision,
                verified: outcome.verified,
            },
        )
    } else {
        writeln!(writer, "Verified storage {}.", outcome.storage_id)?;
        writeln!(
            writer,
            "Mallard config revision: {}",
            outcome.config_revision
        )?;
        writeln!(writer, "Remote capability probe: passed")
    }
}

pub(crate) fn error(writer: &mut dyn Write, json: bool, error: &CliError) {
    let result = if json {
        write_json(
            writer,
            &JsonError {
                schema: 1,
                status: "error",
                exit_code: error.exit_code(),
                code: error.code(),
                message: error.message(),
                setup_id: error.setup_id(),
            },
        )
    } else {
        writeln!(writer, "Error: {}", error)
    };
    let _ = result;
}

fn write_json(writer: &mut dyn Write, value: &impl Serialize) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value).map_err(std::io::Error::other)?;
    writeln!(writer)
}
