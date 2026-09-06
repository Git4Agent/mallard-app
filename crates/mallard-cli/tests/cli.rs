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
                provider: StorageProviderKey::parse("r2").unwrap(),
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
fn help_exposes_only_browser_authorized_r2_setup() {
    let temp = tempfile::tempdir().unwrap();
    let output = command(temp.path())
        .args(["storage", "r2", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = combined_output(&output);
    assert!(help.contains("setup"));
    for removed in [
        "connect",
        "provision",
        "cleanup",
        "--account-id",
        "--bucket",
        "--jurisdiction",
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
    assert!(!setup_help.contains("--account-id"));
    assert!(!setup_help.contains("--bucket"));
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
    assert!(error.get("setup_id").is_none());
    let combined = combined_output(&output);
    assert!(!combined.contains(name_canary));
    assert!(!combined.contains(&temp.path().to_string_lossy().to_string()));
    assert_tree_omits(temp.path(), &[name_canary]);
}

#[test]
fn valid_setup_without_wrangler_has_a_fixed_sanitized_error() {
    let temp = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
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
    assert_eq!(error["code"], "wrangler_not_found");
    assert!(error.get("setup_id").is_none());
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
    assert!(error.get("setup_id").is_none());

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
    assert!(error.get("setup_id").is_none());
    assert!(!String::from_utf8_lossy(&output.stderr)
        .contains(&temp.path().to_string_lossy().to_string()));
}
