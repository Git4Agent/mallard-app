use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn command(home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mallard-cli"));
    command.env("HOME", home).env("USERPROFILE", home);
    command
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_tree_omits(root: &std::path::Path, canaries: &[&str]) {
    fn visit(path: &std::path::Path, canaries: &[&str]) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                visit(&entry.path(), canaries);
            } else if file_type.is_file() {
                let content = fs::read(entry.path()).unwrap();
                let content = String::from_utf8_lossy(&content);
                for canary in canaries {
                    assert!(
                        !content.contains(canary),
                        "secret canary was persisted in {}",
                        entry.path().display()
                    );
                }
            }
        }
    }

    if root.is_dir() {
        visit(root, canaries);
    }
}

#[test]
fn help_advertises_provision_cleanup_and_safe_secret_input_only() {
    let temp = tempfile::tempdir().unwrap();
    let r2_output = command(temp.path())
        .args(["storage", "r2", "--help"])
        .output()
        .unwrap();
    assert!(r2_output.status.success());
    let r2_help = combined_output(&r2_output);
    for command in ["connect", "provision", "cleanup"] {
        assert!(r2_help.contains(command));
    }

    let connect_output = command(temp.path())
        .args(["storage", "r2", "connect", "--help"])
        .output()
        .unwrap();
    assert!(connect_output.status.success());
    let connect_help = combined_output(&connect_output);
    assert!(connect_help.contains("--secret-stdin"));
    assert!(!connect_help.contains("--secret-access-key"));
    assert!(!connect_help.contains("--secret="));

    let provision_output = command(temp.path())
        .args(["storage", "r2", "provision", "--help"])
        .output()
        .unwrap();
    assert!(provision_output.status.success());
    let provision_help = combined_output(&provision_output);
    for flag in [
        "--name",
        "--account-id",
        "--bucket",
        "--jurisdiction",
        "--bootstrap-token-stdin",
    ] {
        assert!(provision_help.contains(flag));
    }
    for jurisdiction in ["default", "eu", "fedramp"] {
        assert!(provision_help.contains(jurisdiction));
    }
    assert!(!provision_help
        .split_whitespace()
        .any(|word| word == "--bootstrap-token" || word.starts_with("--bootstrap-token=")));

    let cleanup_output = command(temp.path())
        .args(["storage", "r2", "cleanup", "--help"])
        .output()
        .unwrap();
    assert!(cleanup_output.status.success());
    let cleanup_help = combined_output(&cleanup_output);
    assert!(cleanup_help.contains("<SETUP_ID>"));
    assert!(cleanup_help.contains("--bootstrap-token-stdin"));
    assert!(!cleanup_help
        .split_whitespace()
        .any(|word| word == "--bootstrap-token" || word.starts_with("--bootstrap-token=")));
}

#[test]
fn list_uses_a_temporary_home_and_emits_no_local_path() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(temp.path())
        .args(["storage", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "{\"schema\":1,\"status\":\"ok\",\"storages\":[]}\n");
    assert!(!stdout.contains(&temp.path().to_string_lossy().to_string()));
    assert!(temp.path().join(".mallard").is_dir());
}

#[test]
fn invalid_connect_never_echoes_or_persists_the_stdin_secret() {
    let temp = tempfile::tempdir().unwrap();
    let canary = "stdin-secret-canary-never-print";
    let access_key_canary = "invalid-access-key-canary\n";
    let mut child = command(temp.path())
        .args([
            "storage",
            "r2",
            "connect",
            "--name",
            "Team backup",
            "--api-url",
            "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com/team-backup",
            "--access-key-id",
            access_key_canary,
            "--secret-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{canary}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let combined = combined_output(&output);
    assert!(!combined.contains(canary));
    assert!(!combined.contains(access_key_canary.trim()));
    assert_tree_omits(temp.path(), &[canary, access_key_canary.trim()]);
}

#[test]
fn invalid_provision_never_echoes_or_persists_inputs_or_bootstrap_token() {
    let temp = tempfile::tempdir().unwrap();
    let name_canary = "private-name-canary";
    let invalid_name = format!("{name_canary}\nline");
    let account_canary = "0123456789abcdef0123456789abcdef";
    let bucket_canary = "private-bucket-canary";
    let token_canary = "bootstrap-token-canary-never-print";
    let mut child = command(temp.path())
        .args([
            "storage",
            "r2",
            "provision",
            "--name",
            &invalid_name,
            "--account-id",
            account_canary,
            "--bucket",
            bucket_canary,
            "--jurisdiction",
            "default",
            "--bootstrap-token-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{token_canary}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["exit_code"], 2);
    assert_eq!(error["code"], "storage_name_invalid");
    assert!(error.get("setup_id").is_none());
    let combined = combined_output(&output);
    for canary in [name_canary, account_canary, bucket_canary, token_canary] {
        assert!(!combined.contains(canary));
    }
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
    assert_tree_omits(
        temp.path(),
        &[name_canary, account_canary, bucket_canary, token_canary],
    );
}

#[test]
fn invalid_cleanup_never_echoes_or_persists_setup_input_or_bootstrap_token() {
    let temp = tempfile::tempdir().unwrap();
    let setup_id_canary = "invalid-private-setup-id-canary";
    let token_canary = "cleanup-token-canary-never-print";
    let mut child = command(temp.path())
        .args([
            "storage",
            "r2",
            "cleanup",
            setup_id_canary,
            "--bootstrap-token-stdin",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{token_canary}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["exit_code"], 2);
    assert_eq!(error["code"], "r2_setup_id_invalid");
    assert!(error.get("setup_id").is_none());
    let combined = combined_output(&output);
    assert!(!combined.contains(setup_id_canary));
    assert!(!combined.contains(token_canary));
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
    assert_tree_omits(temp.path(), &[setup_id_canary, token_canary]);
}

#[test]
fn malformed_local_config_uses_the_stable_persistence_exit_status() {
    let temp = tempfile::tempdir().unwrap();
    let mallard_dir = temp.path().join(".mallard");
    fs::create_dir(&mallard_dir).unwrap();
    fs::write(mallard_dir.join("sync_config.json"), b"not-json").unwrap();

    let output = command(temp.path())
        .args(["storage", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["exit_code"], 5);
    assert_eq!(error["code"], "local_config_unavailable");
    assert!(error.get("setup_id").is_none());
    assert!(!String::from_utf8_lossy(&output.stderr)
        .contains(&temp.path().to_string_lossy().to_string()));
}
