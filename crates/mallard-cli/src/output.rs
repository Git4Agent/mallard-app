use serde::Serialize;
use std::io::Write;

use crate::backend::{
    CleanupOutcome, ConnectOutcome, ProvisionOutcome, StorageSummary, VerifyOutcome,
};
use crate::error::CliError;

#[derive(Serialize)]
struct JsonConnect<'a> {
    schema: u32,
    status: &'static str,
    storage_id: &'a str,
    config_revision: u64,
    verified: bool,
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
struct JsonProvision<'a> {
    schema: u32,
    status: &'static str,
    storage_id: &'a str,
    config_revision: u64,
    verified: bool,
    bucket_created: bool,
}

#[derive(Serialize)]
struct JsonCleanup<'a> {
    schema: u32,
    status: &'static str,
    setup_id: &'a str,
    cleaned: bool,
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

pub(crate) fn connect(
    writer: &mut dyn Write,
    json: bool,
    name: &str,
    outcome: &ConnectOutcome,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &JsonConnect {
                schema: 1,
                status: "connected",
                storage_id: &outcome.storage_id,
                config_revision: outcome.config_revision,
                verified: outcome.verified,
            },
        )
    } else {
        writeln!(writer, "Connected “{}”.", name)?;
        writeln!(writer, "Storage ID: {}", outcome.storage_id)?;
        writeln!(
            writer,
            "Mallard config revision: {}",
            outcome.config_revision
        )?;
        writeln!(writer, "Remote capability probe: passed")
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
            storage.name,
            storage.storage_id,
            match storage.kind {
                "r2" => "Cloudflare R2",
                "local" => "Local folder",
                _ => "Storage",
            }
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

pub(crate) fn provision(
    writer: &mut dyn Write,
    json: bool,
    outcome: &ProvisionOutcome,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &JsonProvision {
                schema: 1,
                status: "provisioned",
                storage_id: &outcome.storage_id,
                config_revision: outcome.config_revision,
                verified: outcome.verified,
                bucket_created: outcome.bucket_created,
            },
        )
    } else {
        writeln!(writer, "Provisioned Cloudflare R2 storage.")?;
        writeln!(writer, "Storage ID: {}", outcome.storage_id)?;
        writeln!(
            writer,
            "Mallard config revision: {}",
            outcome.config_revision
        )?;
        writeln!(writer, "Remote capability probe: passed")?;
        writeln!(
            writer,
            "Bucket created: {}",
            if outcome.bucket_created { "yes" } else { "no" }
        )
    }
}

pub(crate) fn cleanup(
    writer: &mut dyn Write,
    json: bool,
    outcome: &CleanupOutcome,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &JsonCleanup {
                schema: 1,
                status: "cleaned",
                setup_id: &outcome.setup_id,
                cleaned: outcome.cleaned,
            },
        )
    } else if outcome.cleaned {
        writeln!(writer, "Cleaned R2 setup {}.", outcome.setup_id)
    } else {
        writeln!(
            writer,
            "R2 setup {} still requires cleanup.",
            outcome.setup_id
        )
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
