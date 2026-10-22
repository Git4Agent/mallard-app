use serde::Serialize;
use std::io::Write;

use crate::backend::SetupOutcome;
use crate::error::CliError;

/// One verb's result: the JSON document under `--json`, the text otherwise.
/// The document is `{ "schema": 1, "status": <status>, <key>: <payload> }`,
/// where the payload is the same serialized shape the desktop receives.
pub(crate) struct CommandOutput {
    status: &'static str,
    key: &'static str,
    value: serde_json::Value,
    text: String,
}

impl CommandOutput {
    pub fn new<T: Serialize>(
        status: &'static str,
        key: &'static str,
        payload: &T,
        text: String,
    ) -> Result<Self, CliError> {
        let value = serde_json::to_value(payload)
            .map_err(|_| CliError::persistence("Mallard CLI could not encode the result."))?;
        Ok(Self {
            status,
            key,
            value,
            text,
        })
    }

    /// A result whose payload object's fields sit beside the envelope's own,
    /// for a verb that composes several nouns. `schema` and `status` stay the
    /// envelope's.
    pub fn merged(
        status: &'static str,
        payload: &impl Serialize,
        text: String,
    ) -> Result<Self, CliError> {
        Self::new(status, "", payload, text)
    }

    pub fn write(self, writer: &mut dyn Write, json: bool) -> std::io::Result<()> {
        if json {
            let mut document = serde_json::Map::new();
            document.insert("schema".to_string(), serde_json::Value::from(1));
            document.insert("status".to_string(), serde_json::Value::from(self.status));
            if self.key.is_empty() {
                if let serde_json::Value::Object(fields) = self.value {
                    for (key, value) in fields {
                        if key != "schema" && key != "status" {
                            document.insert(key, value);
                        }
                    }
                }
            } else {
                document.insert(self.key.to_string(), self.value);
            }
            write_json(writer, &serde_json::Value::Object(document))
        } else if self.text.is_empty() {
            Ok(())
        } else if self.text.ends_with('\n') {
            writer.write_all(self.text.as_bytes())
        } else {
            writeln!(writer, "{}", self.text)
        }
    }
}

#[derive(Serialize)]
struct JsonSetup {
    schema: u32,
    status: &'static str,
    reconnected: bool,
}

#[derive(Serialize)]
struct JsonError<'a> {
    schema: u32,
    status: &'static str,
    exit_code: i32,
    code: &'a str,
    message: &'a str,
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

pub(crate) fn accounts(
    writer: &mut dyn Write,
    json: bool,
    accounts: &[crate::r2_browser_setup::WranglerAccount],
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &serde_json::json!({"schema":1, "status":"accounts", "accounts":accounts}),
        )
    } else {
        for account in accounts {
            writeln!(writer, "{} — {}", account.id, account.name)?;
        }
        Ok(())
    }
}

pub(crate) fn discovery(
    writer: &mut dyn Write,
    json: bool,
    scan: &mallard_infrastructure::r2_setup::discovery::BucketScan,
) -> std::io::Result<()> {
    if json {
        write_json(
            writer,
            &serde_json::json!({"schema":1,"status":"buckets","scan":scan}),
        )
    } else {
        for bucket in &scan.buckets {
            writeln!(writer, "{} — {:?}", bucket.name, bucket.marker)?;
        }
        if scan.partial {
            writeln!(writer, "Scan incomplete.")?;
        }
        Ok(())
    }
}

/// `"1,234"`-free, plain integer formatting with a unit for text output.
pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Seconds since the epoch as a UTC timestamp, for text output.
pub(crate) fn human_time(secs: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| secs.to_string())
}
