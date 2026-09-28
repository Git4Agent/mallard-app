use std::{path::Path, process::Stdio, time::Duration};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub(super) fn delete(home: &Path, thread_id: &str) -> Result<(), String> {
    super::actions::validate_thread_uuid(thread_id)?;
    let executable = crate::codex_binary::find_binary("codex")?;
    delete_with_binary(&executable, home, thread_id)
}

fn delete_with_binary(executable: &Path, home: &Path, thread_id: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start Codex deletion runtime.")?;
    runtime.block_on(async {
        let mut child = tokio::process::Command::new(executable)
            .args(["app-server", "--stdio"])
            .env("CODEX_HOME", home)
            .current_dir(home)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn().map_err(|_| "Could not start Codex for session deletion.")?;
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            let mut input = child.stdin.take().ok_or("Codex input is unavailable.")?;
            let output = child.stdout.take().ok_or("Codex output is unavailable.")?;
            let mut lines = BufReader::new(output.take(1024 * 1024)).lines();
            for (id, request) in [
                (1, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"mallard","version":env!("CARGO_PKG_VERSION")}}})),
                (2, json!({"id":2,"method":"thread/delete","params":{"threadId":thread_id}})),
            ] {
                input.write_all(format!("{request}\n").as_bytes()).await
                    .map_err(|_| "Could not send Codex deletion request.")?;
                loop {
                    let line = lines.next_line().await.map_err(|_| "Could not read Codex deletion response.")?
                        .ok_or("Codex stopped before confirming deletion.")?;
                    let response: Value = serde_json::from_str(&line)
                        .map_err(|_| "Codex returned an invalid deletion response.")?;
                    if response["id"].as_u64() != Some(id) { continue; }
                    if let Some(error) = response.get("error") {
                        return Err(format!("Codex session deletion failed (code {}).", error["code"]));
                    }
                    if response.get("result").is_none() {
                        return Err("Codex did not confirm the request.".into());
                    }
                    break;
                }
                if id == 1 {
                    input.write_all(b"{\"method\":\"initialized\"}\n").await
                        .map_err(|_| "Could not initialize Codex deletion.")?;
                }
            }
            Ok(())
        }).await.unwrap_or_else(|_| Err("Codex deletion timed out; some files may already be deleted.".into()));
        let _ = child.kill().await;
        let _ = child.wait().await;
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn unsupported_api_returns_error_without_retry_or_file_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("fake-codex");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
read -r init
printf '%s\n' "$init" >> requests
printf '%s\n' '{"id":1,"result":{}}'
read -r initialized
printf '%s\n' "$initialized" >> requests
read -r delete
printf '%s\n' "$delete" >> requests
printf '%s\n' '{"id":2,"error":{"code":-32601,"message":"unsupported"}}'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let sentinel = temp.path().join("session.jsonl");
        std::fs::write(&sentinel, "keep").unwrap();
        let error = delete_with_binary(&executable, temp.path(), "selected").unwrap_err();
        assert!(error.contains("-32601"));
        let requests = std::fs::read_to_string(temp.path().join("requests")).unwrap();
        let methods: Vec<_> = requests
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(methods, ["initialize", "initialized", "thread/delete"]);
        assert!(sentinel.exists());
    }

    #[test]
    #[ignore = "requires an installed Codex CLI; temporary homes only"]
    fn native_deletion_keeps_parent_and_fork() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let sessions = home.join("sessions/2026/09/26");
        std::fs::create_dir_all(&sessions).unwrap();
        let ids = [
            "019f7798-5437-7632-9dbc-5b589cf68bf0",
            "019f7798-5437-7632-9dbc-5b589cf68bf1",
            "019f7798-5437-7632-9dbc-5b589cf68bf2",
            "019f7798-5437-7632-9dbc-5b589cf68bf3",
        ];
        let paths: Vec<_> = ids
            .iter()
            .map(|id| sessions.join(format!("rollout-2026-09-26T00-00-00-{id}.jsonl")))
            .collect();
        for (index, id) in ids.iter().enumerate() {
            let mut payload = json!({"id":id,"cwd":home,"timestamp":"2026-09-26T00:00:00Z","source":"cli","cli_version":"0.156.0","originator":"codex_cli_rs"});
            if index == 1 {
                payload["forked_from_id"] = json!(ids[0]);
            }
            if index == 2 {
                payload["forked_from_id"] = json!(ids[1]);
            }
            if index == 3 {
                payload["source"] =
                    json!({"subagent":{"thread_spawn":{"parent_thread_id":ids[1],"depth":1}}});
                payload["parent_thread_id"] = json!(ids[1]);
            }
            std::fs::write(&paths[index], format!("{}\n", json!({"type":"session_meta","timestamp":"2026-09-26T00:00:00Z","payload":payload}))).unwrap();
        }
        delete(home, ids[1]).unwrap();
        assert!(!paths[1].exists(), "selected root remains");
        assert!(!paths[3].exists(), "spawned child remains");
        assert!(paths[0].exists(), "parent was removed");
        assert!(paths[2].exists(), "independent fork was removed");
    }
}
