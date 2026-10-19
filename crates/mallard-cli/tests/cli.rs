use mallard_domain::{
    generated_named_id, BoundedConfigObject, SecretMap, StorageConfig, StorageId,
    StorageProviderConfig, StorageProviderKey, StorageSetupDraft, StorageSetupDraftId,
    StorageSetupDraftSource, STORAGE_SETUP_DRAFT_SCHEMA,
};
use mallard_infrastructure::persistence::LocalStateRepository;
use std::fs;
use std::process::Command;

const ACTIONABLE_LOCAL_CONFIG_MESSAGE: &str = "Mallard could not access its local storage configuration. If this command is running in a restricted environment, approve read/write access to Mallard's local state and run the same command again.";

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

fn create_r2_draft(home: &std::path::Path, name: &str) -> StorageSetupDraft {
    create_storage_draft(home, name, "r2")
}

fn create_storage_draft(home: &std::path::Path, name: &str, provider: &str) -> StorageSetupDraft {
    let repository = LocalStateRepository::from_home_dir(home).unwrap();
    repository
        .save_storage_setup_draft(StorageSetupDraft {
            schema: STORAGE_SETUP_DRAFT_SCHEMA,
            draft_id: StorageSetupDraftId::parse(generated_named_id("storagedraft").unwrap())
                .unwrap(),
            source: StorageSetupDraftSource::New,
            storage: StorageConfig {
                id: StorageId::parse(generated_named_id("storage").unwrap()).unwrap(),
                name: name.to_string(),
                provider: StorageProviderKey::parse(provider).unwrap(),
                provider_config: StorageProviderConfig::new(
                    1,
                    BoundedConfigObject::empty(),
                    SecretMap::empty(),
                ),
            },
            revision: 0,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap()
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
                        "canary was persisted in {}",
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
fn help_exposes_account_check_and_browser_authorized_r2_setup() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(temp.path())
        .args(["storage", "r2", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = combined_output(&output);
    assert!(help.contains("setup"));
    assert!(help.contains("accounts"));
    for removed in [
        "connect",
        "provision",
        "cleanup",
        "--account-id",
        "--access-key-id",
        "--secret-stdin",
        "--bootstrap-token-stdin",
    ] {
        assert!(
            !help.contains(removed),
            "obsolete help item remained: {removed}"
        );
    }

    let setup_output = command(temp.path())
        .args(["storage", "r2", "setup", "--help"])
        .output()
        .unwrap();
    assert!(setup_output.status.success());
    let setup_help = combined_output(&setup_output);
    assert!(setup_help.contains("--name"));
    assert!(setup_help.contains("--draft-id"));
    assert!(setup_help.contains("--expected-revision"));
    assert!(!setup_help.to_ascii_lowercase().contains("secret"));
    assert!(setup_help.contains("--account-id"));
    assert!(setup_help.contains("--jurisdiction"));
    assert!(setup_help.contains("--bucket"));
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
fn invalid_setup_name_fails_before_wrangler_and_is_never_echoed_or_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let name_canary = "private-name-canary";
    let invalid_name = format!("{name_canary}\nline");
    let draft_id = generated_named_id("storagedraft").unwrap();
    let output = command(temp.path())
        .env("PATH", empty_path.path())
        .args([
            "storage",
            "r2",
            "setup",
            "--name",
            &invalid_name,
            "--draft-id",
            &draft_id,
            "--expected-revision",
            "1",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["exit_code"], 2);
    assert_eq!(error["code"], "storage_name_invalid");
    let combined = combined_output(&output);
    assert!(!combined.contains(name_canary));
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
    assert_tree_omits(temp.path(), &[name_canary]);
}

#[test]
fn valid_setup_with_unavailable_wrangler_has_a_fixed_sanitized_error() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        // Desktop discovery also checks global install locations. Supply explicit
        // failing executables so this test cannot start the user's Wrangler.
        std::os::unix::fs::symlink("/usr/bin/false", empty_path.path().join("wrangler")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/false", empty_path.path().join("node")).unwrap();
    }
    let name_canary = "Private destination canary";
    let draft = create_r2_draft(temp.path(), name_canary);
    let draft_id = draft.draft_id.to_string();
    let revision = draft.revision.to_string();
    let output = command(temp.path())
        .env("PATH", empty_path.path())
        .args([
            "storage",
            "r2",
            "setup",
            "--name",
            name_canary,
            "--draft-id",
            &draft_id,
            "--expected-revision",
            &revision,
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["schema"], 1);
    assert_eq!(error["status"], "error");
    assert_eq!(error["exit_code"], 2);
    assert_eq!(
        error["code"],
        if cfg!(unix) {
            "wrangler_runtime_unavailable"
        } else {
            "wrangler_not_found"
        }
    );
    let combined = combined_output(&output);
    assert!(!combined.contains(name_canary));
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
}

#[test]
fn setup_requires_the_exact_existing_draft_before_wrangler() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let name = "Team backup";
    let missing_id = generated_named_id("storagedraft").unwrap();
    let output = command(temp.path())
        .env("PATH", empty_path.path())
        .args([
            "storage",
            "r2",
            "setup",
            "--name",
            name,
            "--draft-id",
            &missing_id,
            "--expected-revision",
            "1",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "storage_setup_draft_not_found");
    let combined = combined_output(&output);
    assert!(!combined.contains(name));
    assert!(!combined.contains(&missing_id));
}

#[test]
fn setup_rejects_a_stale_draft_revision_before_wrangler() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let name = "Team backup";
    let draft = create_r2_draft(temp.path(), name);
    let draft_id = draft.draft_id.to_string();
    let stale_revision = draft.revision.saturating_add(1).to_string();
    let output = command(temp.path())
        .env("PATH", empty_path.path())
        .args([
            "storage",
            "r2",
            "setup",
            "--name",
            name,
            "--draft-id",
            &draft_id,
            "--expected-revision",
            &stale_revision,
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "storage_setup_draft_revision_conflict");
    let combined = combined_output(&output);
    assert!(!combined.contains(name));
    assert!(!combined.contains(&draft_id));
}

#[test]
fn removed_dropbox_setup_is_rejected_without_remote_work() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path())
        .args(["storage", "dropbox", "setup", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn removed_r2_commands_use_the_generic_non_echoing_error() {
    let temp = tempfile::tempdir().unwrap();
    for removed in ["connect", "provision", "cleanup"] {
        let canary = "private-argument-canary";
        let output = command(temp.path())
            .args(["storage", "r2", removed, canary, "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["code"], "cli_invalid_input");
        assert!(!combined_output(&output).contains(canary));
    }
}

#[test]
fn inaccessible_local_state_has_an_actionable_sanitized_human_error() {
    let temp = tempfile::tempdir().unwrap();
    let state_canary = "private-local-state-canary";
    fs::write(temp.path().join(".mallard"), state_canary).unwrap();

    let output = command(temp.path())
        .args(["storage", "list"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("Error: {ACTIONABLE_LOCAL_CONFIG_MESSAGE}\n")
    );
}

#[test]
fn inaccessible_local_state_has_an_actionable_sanitized_json_error() {
    let temp = tempfile::tempdir().unwrap();
    let state_canary = "private-local-state-canary";
    fs::write(temp.path().join(".mallard"), state_canary).unwrap();

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
    assert_eq!(error["code"], "cli_persistence_failure");
    assert_eq!(error["message"], ACTIONABLE_LOCAL_CONFIG_MESSAGE);

    let combined = combined_output(&output);
    assert!(!combined.contains(state_canary));
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
    assert!(!combined.contains("Operation not permitted"));
    assert!(!combined.contains("os error"));
    assert!(!combined.contains("metadata directory"));
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
    assert_eq!(error["message"], ACTIONABLE_LOCAL_CONFIG_MESSAGE);
    assert!(!String::from_utf8_lossy(&output.stderr)
        .contains(&temp.path().to_string_lossy().to_string()));
}

#[cfg(unix)]
#[test]
fn an_expired_wrangler_session_signs_in_again_before_listing_accounts() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let wrangler = bin.path().join("wrangler");
    // Until login runs, whoami answers the way Wrangler does for a token that
    // expired and could not be refreshed outside a terminal: an error and no
    // document. Login records the new session in a side file.
    fs::write(&wrangler, r#"#!/bin/sh
case "$*" in
  '--version') printf '4.125.0';;
  'auth token --help') printf '%s' '--json';;
  'login --browser=false') printf '%s\n' 'Visit this link to authenticate: https://dash.cloudflare.com/oauth2/auth?state=fixture'; : > "$0.session";;
  'whoami --json') if [ -f "$0.session" ]; then printf '%s' '{"loggedIn":true,"accounts":[{"id":"11111111111111111111111111111111","name":"Test"}]}'; else printf '%s\n' 'ERROR Not logged in. Your auth token has expired and could not be refreshed.' >&2; exit 1; fi;;
  *) exit 1;;
esac
"#).unwrap();
    fs::set_permissions(&wrangler, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink("/usr/bin/false", bin.path().join("node")).unwrap();
    let output = command(home.path())
        .env("PATH", bin.path())
        .args(["--json", "storage", "r2", "accounts"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["status"], "wrangler_ready");
    assert_eq!(lines[1]["status"], "authorization_link");
    assert_eq!(lines[1]["stage"], "login");
    assert_eq!(lines[2]["status"], "accounts");
    assert_eq!(lines[2]["accounts"][0]["name"], "Test");
    assert!(
        !stdout.contains("expired"),
        "Wrangler's own error text stays out of the stream"
    );
}

#[cfg(unix)]
#[test]
fn login_links_do_not_deadlock_against_the_final_json_writer() {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let wrangler = bin.path().join("wrangler");
    fs::write(&wrangler, r#"#!/bin/sh
case "$*" in
  '--version') printf '4.125.0';;
  'auth token --help') printf '%s' '--json';;
  'login --browser=false') printf '%s\n' 'Visit this link to authenticate: https://dash.cloudflare.com/oauth2/auth?state=fixture';;
  'whoami --json') printf '%s' '{"loggedIn":true,"accounts":[{"id":"11111111111111111111111111111111","name":"Test"}]}';;
  *) exit 1;;
esac
"#).unwrap();
    fs::set_permissions(&wrangler, fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink("/usr/bin/false", bin.path().join("node")).unwrap();
    let mut child = command(home.path())
        .env("PATH", bin.path())
        .args(["--json", "storage", "r2", "accounts", "--login"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("login progress must not wait for a stdout lock held by the CLI");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let lines: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["status"], "wrangler_ready");
    assert_eq!(lines[0]["version"], "4.125.0");
    // The path is the canonical one the CLI resolved (macOS keeps /var under /private).
    assert_eq!(
        lines[0]["path"],
        wrangler.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(lines[1]["status"], "authorization_link");
    assert_eq!(lines[1]["stage"], "login");
    assert_eq!(lines[2]["status"], "accounts");
}
