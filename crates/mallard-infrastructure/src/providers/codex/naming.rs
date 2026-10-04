//! Native presentation updates. Call only after the reviewed Pull commits.

use serde_json::json;
use std::{path::Path, time::Duration};

pub fn rename(home: &Path, thread_id: &str, title: &str) -> Result<(), String> {
    super::actions::validate_thread_uuid(thread_id)?;
    mallard_domain::validate_title("Codex thread name", title)?;
    let executable = crate::codex_binary::find_binary("codex")?;
    rename_with_binary(&executable, home, thread_id, title, Duration::from_secs(5))
}

fn rename_with_binary(
    executable: &Path,
    home: &Path,
    thread_id: &str,
    title: &str,
    timeout: Duration,
) -> Result<(), String> {
    super::app_server::request(
        executable,
        home,
        "thread/name/set",
        json!({"threadId": thread_id, "name": title}),
        timeout,
    )
    .map(|_| ())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{fs, os::unix::fs::PermissionsExt};

    const ID: &str = "019f7798-5437-7632-9dbc-5b589cf68bf0";

    fn fake(home: &Path, response: &str) -> std::path::PathBuf {
        let binary = home.join("fake-codex");
        fs::write(
            &binary,
            format!(
                r#"#!/bin/sh
test "$CODEX_HOME" = "$PWD" || exit 1
read -r init
printf '%s\n' "$init" >> requests
printf '%s\n' '{{"id":1,"result":{{}}}}'
read -r initialized
printf '%s\n' "$initialized" >> requests
read -r rename
printf '%s\n' "$rename" >> requests
{response}
"#
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }

    #[test]
    fn rename_uses_selected_home_and_exact_name_once() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let binary = fake(
            &home,
            "printf '%s\\n' '{\"method\":\"thread/name/updated\"}' '{\"id\":2,\"result\":{}}'",
        );
        let title = "Title \"quoted\" (Preserved 1 · device)";
        rename_with_binary(&binary, &home, ID, title, Duration::from_secs(2)).unwrap();
        let requests = fs::read_to_string(home.join("requests")).unwrap();
        let rows: Vec<Value> = requests
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["method"], "initialize");
        assert_eq!(rows[1]["method"], "initialized");
        assert_eq!(
            rows[2],
            json!({"id":2,"method":"thread/name/set","params":{"threadId":ID,"name":title}})
        );
    }

    #[test]
    fn rename_failures_are_bounded_sanitized_and_not_retried() {
        // Response cases must reach their error, including the output limit,
        // before the deadline. The silent process needs time to initialize,
        // then waits for input so only the request timeout can stop it.
        let response_timeout = Duration::from_secs(5);
        for (case, response, expected, timeout) in [
            ("numeric error", "printf '%s\\n' '{\"id\":2,\"error\":{\"code\":-32601,\"message\":\"private title /private/path\"}}'", "-32601", response_timeout),
            ("non-numeric error", "printf '%s\\n' '{\"id\":2,\"error\":{\"code\":\"private title\"}}'", "failed", response_timeout),
            ("invalid JSON", "printf '%s\\n' 'private title'", "invalid response", response_timeout),
            ("missing result", "printf '%s\\n' '{\"id\":2}'", "did not confirm", response_timeout),
            ("stopped process", "exit 0", "stopped", response_timeout),
            ("silent process", r#"while read -r request; do printf '%s\n' "$request" >> requests; done"#, "timed out", Duration::from_secs(2)),
            ("endless output", "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done", "invalid response", response_timeout),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().canonicalize().unwrap();
            fs::write(home.join("session.jsonl"), "unchanged").unwrap();
            let binary = fake(&home, response);
            let error = rename_with_binary(&binary, &home, ID, "New", timeout).unwrap_err();
            assert!(error.contains(expected), "{case}: expected {expected:?}, got {error}");
            assert!(!error.contains("private"));
            assert_eq!(fs::read_to_string(home.join("requests")).unwrap().lines().count(), 3);
            assert_eq!(fs::read_to_string(home.join("session.jsonl")).unwrap(), "unchanged");
        }
    }

    #[test]
    fn rename_rejects_invalid_identity_and_title_before_launch() {
        assert!(rename(Path::new("/missing"), "not-a-uuid", "Title").is_err());
        assert!(rename(Path::new("/missing"), ID, "").is_err());
    }

    #[test]
    #[ignore = "requires an installed Codex CLI; temporary homes only"]
    fn native_rename_preserves_transcripts_and_updates_both_names() {
        let executable = crate::codex_binary::find_binary("codex").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let sessions = home.join("sessions/2026/09/30");
        fs::create_dir_all(&sessions).unwrap();
        for (id, title) in [
            (ID, "Fixture (Preserved 1 · test)"),
            ("019f7798-5437-7632-9dbc-5b589cf68bf1", "Fixture"),
        ] {
            let path = sessions.join(format!("rollout-2026-09-30T00-00-00-{id}.jsonl"));
            let bytes = format!(
                "{}\n{}\n",
                json!({"type":"session_meta","timestamp":"2026-09-30T00:00:00Z","payload":{"id":id,"cwd":home,"timestamp":"2026-09-30T00:00:00Z","source":"cli","cli_version":"0.156.0","originator":"codex_cli_rs"}}),
                json!({"type":"response_item","timestamp":"2026-09-30T00:00:01Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fixture initial prompt"}]}})
            );
            fs::write(&path, &bytes).unwrap();
            rename_with_binary(&executable, &home, id, title, Duration::from_secs(5)).unwrap();
            // A new app-server process must see the persisted name.
            let read = super::super::app_server::request(
                &executable,
                &home,
                "thread/read",
                json!({"threadId":id,"includeTurns":false}),
                Duration::from_secs(5),
            )
            .unwrap();
            assert_eq!(read["thread"]["name"], title);
            assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
            let index = fs::read(home.join("session_index.jsonl")).unwrap();
            assert_eq!(
                super::super::history::parse_session_index_titles(&index)
                    .get(id)
                    .map(String::as_str),
                Some(title)
            );
        }
    }
}
