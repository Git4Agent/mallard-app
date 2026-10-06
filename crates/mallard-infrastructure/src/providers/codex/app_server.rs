//! Bounded, one-shot requests to the selected Codex home's app server.

use serde_json::{json, Value};
use std::{ffi::OsStr, path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub(super) fn request(
    executable: &Path,
    search_path: Option<&OsStr>,
    home: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start Codex request runtime.")?;
    runtime.block_on(async {
        let mut command = tokio::process::Command::new(executable);
        if let Some(search_path) = search_path {
            command.env("PATH", search_path);
        }
        command.args(["app-server", "--stdio"])
            .env("CODEX_HOME", home).current_dir(home)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(test)]
        command.env("HOME", home);
        let mut child = command.spawn().map_err(|_| "Could not start Codex for a provider request.")?;
        let result = tokio::time::timeout(timeout, async {
            let mut input = child.stdin.take().ok_or("Codex input is unavailable.")?;
            let output = child.stdout.take().ok_or("Codex output is unavailable.")?;
            let mut lines = BufReader::new(output.take(1024 * 1024)).lines();
            let mut result = Value::Null;
            for (id, request) in [
                (1, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"mallard","version":env!("CARGO_PKG_VERSION")}}})),
                (2, json!({"id":2,"method":method,"params":params})),
            ] {
                input.write_all(format!("{request}\n").as_bytes()).await
                    .map_err(|_| "Could not send Codex request.")?;
                loop {
                    let line = lines.next_line().await.map_err(|_| "Could not read Codex response.")?
                        .ok_or("Codex stopped before confirming the request.")?;
                    let response: Value = serde_json::from_str(&line)
                        .map_err(|_| "Codex returned an invalid response.")?;
                    if response["id"].as_u64() != Some(id) { continue; }
                    if let Some(error) = response.get("error") {
                        // Never expose provider error messages or arbitrary JSON values.
                        return Err(match error["code"].as_i64() {
                            Some(code) => format!("Codex provider request failed (code {code})."),
                            None => "Codex provider request failed.".into(),
                        });
                    }
                    result = response.get("result").cloned().ok_or("Codex did not confirm the request.")?;
                    break;
                }
                if id == 1 {
                    input.write_all(b"{\"method\":\"initialized\"}\n").await
                        .map_err(|_| "Could not initialize Codex request.")?;
                }
            }
            Ok(result)
        }).await.unwrap_or_else(|_| Err("Codex request timed out; it may already have been applied.".into()));
        let _ = child.kill().await;
        let _ = child.wait().await;
        result
    })
}
