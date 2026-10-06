use std::{ffi::OsStr, path::Path, time::Duration};

use serde_json::json;
#[cfg(test)]
use serde_json::Value;

/// `cli_path` is the Codex executable saved in Settings, if any.
pub(super) fn delete(home: &Path, thread_id: &str, cli_path: Option<&Path>) -> Result<(), String> {
    super::actions::validate_thread_uuid(thread_id)?;
    let cli = super::cli::resolve(cli_path)?;
    delete_with_binary(&cli.executable, Some(&cli.search_path), home, thread_id)
}

fn delete_with_binary(
    executable: &Path,
    search_path: Option<&OsStr>,
    home: &Path,
    thread_id: &str,
) -> Result<(), String> {
    super::app_server::request(
        executable,
        search_path,
        home,
        "thread/delete",
        json!({"threadId": thread_id}),
        Duration::from_secs(20),
    )
    .map(|_| ())
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
        let error = delete_with_binary(&executable, None, temp.path(), "selected").unwrap_err();
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
        delete(home, ids[1], None).unwrap();
        assert!(!paths[1].exists(), "selected root remains");
        assert!(!paths[3].exists(), "spawned child remains");
        assert!(paths[0].exists(), "parent was removed");
        assert!(paths[2].exists(), "independent fork was removed");
    }
}
