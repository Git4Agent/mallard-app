//! Read-only bridge for the Feishu transport PoC. Use the production parser,
//! record-name builder, and selector instead of a JavaScript implementation.

use mallard_domain::{sha256_hex, Mainline, PublicationToken, RecordName};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, Read};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Select {
        names: Vec<String>,
    },
    Build {
        commit_id: String,
        parent_name: Option<String>,
        created_at_ms: u64,
        body: String,
    },
}

fn run() -> Result<serde_json::Value, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read")?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("size".into());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|_| "input")?;
    match request {
        Request::Select { names } => {
            if names.len() > 100_000 {
                return Err("count".into());
            }
            let parsed: Vec<_> = names
                .iter()
                .filter_map(|name| RecordName::parse(name))
                .collect();
            let ignored = names.len() - parsed.len();
            let valid_names: Vec<_> = parsed.iter().map(RecordName::file_name).collect();
            let line = Mainline::select(parsed);
            Ok(json!({
                "chain": line.names().iter().map(RecordName::file_name).collect::<Vec<_>>(),
                "ignored": ignored,
                "valid_names": valid_names,
            }))
        }
        Request::Build {
            commit_id,
            parent_name,
            created_at_ms,
            body,
        } => {
            let parent = parent_name
                .map(|name| RecordName::parse(&name).ok_or("parent"))
                .transpose()?;
            let revision = parent.as_ref().map_or(1, |name| name.revision() + 1);
            let token = PublicationToken::new(commit_id, sha256_hex(body.as_bytes()))?;
            let name = RecordName::new(
                revision,
                token,
                parent.as_ref().map(|name| name.token().clone()),
                created_at_ms,
            )?;
            Ok(json!({"name": name.file_name(), "revision": revision}))
        }
    }
}

fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(_) => {
            eprintln!("invalid_selector_input");
            std::process::exit(1);
        }
    }
}
