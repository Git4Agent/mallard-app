//! The verbs every agent starts with — status, projects, storages, settings,
//! the activity log, providers — against a temporary home. Nothing here
//! touches a real `~/.mallard`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use mallard_application::storage_setup::{
    commit_storage_setup_draft, create_storage_setup_draft, update_storage_setup_draft,
    UpdateStorageSetupDraftRequest,
};
use mallard_domain::BoundedConfigObject;
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::storage_providers::{local, StorageProviderRegistry};

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mallard-cli"));
    command.env("HOME", home).env("USERPROFILE", home);
    command
}

fn json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not one JSON document ({error}): {:?} / stderr {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn error_json(output: &std::process::Output) -> serde_json::Value {
    assert!(
        output.stdout.is_empty(),
        "an error prints nothing on stdout"
    );
    serde_json::from_slice(&output.stderr).unwrap()
}

/// A committed local-folder storage, as the desktop's setup page commits it.
fn local_storage(home: &Path, name: &str) -> (String, u64) {
    let repository = LocalStateRepository::from_home_dir(home).unwrap();
    let registry = StorageProviderRegistry::production();
    let directory = home.join(format!("storage-{}", name.replace(' ', "-")));
    std::fs::create_dir_all(&directory).unwrap();
    let draft = create_storage_setup_draft(&repository, registry, &local::provider_key()).unwrap();
    let draft = update_storage_setup_draft(
        &repository,
        registry,
        UpdateStorageSetupDraftRequest {
            draft_id: draft.draft_id.clone(),
            revision: draft.revision,
            name: name.to_string(),
            settings: BoundedConfigObject::from_json(&serde_json::json!({
                "directory": directory.to_string_lossy(),
            }))
            .unwrap(),
            secrets: BTreeMap::new(),
        },
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let committed = commit_storage_setup_draft(
        &repository,
        registry,
        runtime.handle(),
        &draft.draft_id,
        draft.revision,
        |_| Ok(()),
    )
    .unwrap();
    (committed.storage_id.to_string(), committed.config_revision)
}

#[test]
fn status_describes_an_empty_home_without_a_current_project() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path())
        .current_dir(home.path())
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let document = json(&output);
    assert_eq!(document["schema"], 1);
    assert_eq!(document["status"], "ok");
    // The host identity is created on first use, as the desktop creates it.
    let host_id = document["host"]["host_id"].as_str().unwrap();
    assert_eq!(host_id.len(), 32);
    assert!(host_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(document["config"]["revision"], 0);
    assert_eq!(document["config"]["storages"], serde_json::json!([]));
    assert!(document["current_project"].is_null());

    let text = command(home.path())
        .current_dir(home.path())
        .arg("status")
        .output()
        .unwrap();
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("Storages: 0"));
    assert!(text.contains("not inside a registered project"));
    assert!(!text.contains(&home.path().to_string_lossy().to_string()));
}

#[test]
fn project_verbs_refuse_a_folder_outside_every_project() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path())
        .current_dir(home.path())
        .args(["--json", "project", "show"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error = error_json(&output);
    assert_eq!(error["code"], "project_not_found");
    assert!(!error["message"]
        .as_str()
        .unwrap()
        .contains(&home.path().to_string_lossy().to_string()));

    let list = command(home.path())
        .args(["--json", "project", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert_eq!(json(&list)["projects"], serde_json::json!([]));
}

#[test]
fn storage_verbs_list_show_verify_rename_and_remove_a_local_folder() {
    let home = tempfile::tempdir().unwrap();
    let (storage_id, revision) = local_storage(home.path(), "Team backup");

    let list = command(home.path())
        .args(["--json", "storage", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let list = json(&list);
    assert_eq!(list["storages"][0]["storage_id"], storage_id);
    assert_eq!(list["storages"][0]["name"], "Team backup");
    assert_eq!(list["storages"][0]["provider"], "local");
    // Secret values never travel in a listing; presence does.
    assert!(list["storages"][0]["secrets"].is_array());

    let show = command(home.path())
        .args(["--json", "storage", "show", "Team backup"])
        .output()
        .unwrap();
    assert!(show.status.success(), "{:?}", show);
    assert_eq!(json(&show)["storage"]["storage_id"], storage_id);

    let verify = command(home.path())
        .args(["--json", "storage", "verify", &storage_id])
        .output()
        .unwrap();
    assert!(verify.status.success(), "{:?}", verify);
    let verify = json(&verify);
    assert_eq!(verify["status"], "verified");
    assert_eq!(verify["verification"]["verified"], true);

    let stale = command(home.path())
        .args([
            "--json",
            "storage",
            "rename",
            &storage_id,
            "--name",
            "Renamed",
            "--expected-revision",
            &(revision + 7).to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(3), "{:?}", stale);

    let renamed = command(home.path())
        .args([
            "--json",
            "storage",
            "rename",
            &storage_id,
            "--name",
            "Renamed",
            "--expected-revision",
            &revision.to_string(),
        ])
        .output()
        .unwrap();
    assert!(renamed.status.success(), "{:?}", renamed);
    let renamed = json(&renamed);
    assert_eq!(renamed["status"], "renamed");
    let next_revision = renamed["config"]["revision"].as_u64().unwrap();
    assert!(next_revision > revision);
    assert_eq!(renamed["config"]["storages"][0]["name"], "Renamed");
    // The rename narrated on stderr as NDJSON, and was appended to the log.
    let events =
        String::from_utf8(renamed_stderr(home.path(), &storage_id, next_revision)).unwrap();
    assert!(events.lines().all(|line| line.starts_with('{')), "{events}");

    let unconfirmed = command(home.path())
        .args([
            "--json",
            "storage",
            "remove",
            "Renamed",
            "--expected-revision",
            &next_revision.to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(unconfirmed.status.code(), Some(2));
    assert_eq!(error_json(&unconfirmed)["code"], "confirmation_required");

    let removed = command(home.path())
        .args([
            "--json",
            "storage",
            "remove",
            "Renamed",
            "--expected-revision",
            &next_revision.to_string(),
            "--confirm",
        ])
        .output()
        .unwrap();
    assert!(removed.status.success(), "{:?}", removed);
    assert_eq!(json(&removed)["config"]["storages"], serde_json::json!([]));

    let log = command(home.path())
        .args(["--json", "log", "query", "--type", "storage"])
        .output()
        .unwrap();
    assert!(log.status.success(), "{:?}", log);
    let entries = json(&log)["page"]["entries"].clone();
    let events: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["event"].as_str().unwrap())
        .collect();
    assert!(events.contains(&"storage.settings_updated"), "{events:?}");
    assert!(events.contains(&"storage.removed"), "{events:?}");
}

/// A second rename, to read its stderr: the first call's stderr was already
/// consumed by the assertions above.
fn renamed_stderr(home: &Path, storage_id: &str, revision: u64) -> Vec<u8> {
    let output = command(home)
        .args([
            "--json",
            "storage",
            "rename",
            storage_id,
            "--name",
            "Renamed",
            "--expected-revision",
            &revision.to_string(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    output.stderr
}

#[test]
fn storage_providers_and_unknown_storages_are_reported() {
    let home = tempfile::tempdir().unwrap();
    let providers = command(home.path())
        .args(["--json", "storage", "providers"])
        .output()
        .unwrap();
    assert!(providers.status.success());
    let keys: Vec<String> = json(&providers)["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|provider| provider["key"].as_str().unwrap().to_string())
        .collect();
    assert!(keys.iter().any(|key| key == "local"), "{keys:?}");

    let missing = command(home.path())
        .args(["--json", "storage", "show", "nothing-here"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(error_json(&missing)["code"], "storage_not_found");
}

#[test]
fn settings_are_read_changed_and_validated() {
    let home = tempfile::tempdir().unwrap();
    let get = command(home.path())
        .args(["--json", "settings", "get"])
        .output()
        .unwrap();
    assert!(get.status.success());
    assert_eq!(
        json(&get)["settings"]["credential_redaction_enabled"],
        false
    );

    let set = command(home.path())
        .args([
            "--json",
            "settings",
            "set",
            "credential_redaction_enabled=true",
            "comparison_workers=2",
        ])
        .output()
        .unwrap();
    assert!(set.status.success(), "{:?}", set);
    let saved = json(&set);
    assert_eq!(saved["status"], "saved");
    assert_eq!(saved["settings"]["credential_redaction_enabled"], true);
    assert_eq!(saved["settings"]["comparison_workers"], 2);

    let unknown = command(home.path())
        .args(["--json", "settings", "set", "nonsense=1"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(error_json(&unknown)["code"], "settings_key_unknown");

    let wrong_type = command(home.path())
        .args(["--json", "settings", "set", "comparison_workers=many"])
        .output()
        .unwrap();
    assert_eq!(wrong_type.status.code(), Some(2));
    assert_eq!(error_json(&wrong_type)["code"], "settings_value_invalid");

    let text = command(home.path())
        .args(["settings", "get"])
        .output()
        .unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("credential_redaction_enabled = true"));
    assert!(!text.contains("schema"));
}

#[test]
fn activity_log_verbs_read_the_empty_log_and_its_policy() {
    let home = tempfile::tempdir().unwrap();
    let stats = command(home.path())
        .args(["--json", "log", "stats"])
        .output()
        .unwrap();
    assert!(stats.status.success(), "{:?}", stats);
    assert_eq!(json(&stats)["stats"]["file_count"], 0);

    let policy = command(home.path())
        .args(["--json", "log", "policy", "--retention-days", "7"])
        .output()
        .unwrap();
    assert!(policy.status.success(), "{:?}", policy);
    assert_eq!(json(&policy)["stats"]["policy"]["retention_days"], 7);

    let bad_filter = command(home.path())
        .args(["--json", "log", "query", "--type", "gossip"])
        .output()
        .unwrap();
    assert_eq!(bad_filter.status.code(), Some(2));
    assert_eq!(error_json(&bad_filter)["code"], "log_filter_invalid");

    let folder = command(home.path())
        .args(["--json", "log", "folder"])
        .output()
        .unwrap();
    assert!(folder.status.success());
    assert!(json(&folder)["folder"].as_str().unwrap().ends_with("logs"));
}

#[test]
fn provider_verbs_list_descriptors_and_resolve_a_cli() {
    let home = tempfile::tempdir().unwrap();
    let list = command(home.path())
        .args(["--json", "provider", "list"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let keys: Vec<String> = json(&list)["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|provider| provider["key"].as_str().unwrap().to_string())
        .collect();
    assert!(keys.contains(&"codex".to_string()) && keys.contains(&"claude".to_string()));

    // Whether a Codex CLI is installed here is not the test's business: the
    // verb reports either way and never fails.
    let inspect = command(home.path())
        .env("PATH", home.path())
        .args(["--json", "provider", "cli", "inspect", "codex"])
        .output()
        .unwrap();
    assert!(inspect.status.success(), "{:?}", inspect);
    assert_eq!(json(&inspect)["cli"]["provider"], "codex");

    let unknown = command(home.path())
        .args(["--json", "provider", "cli", "inspect", "qoder"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(error_json(&unknown)["code"], "provider_unknown");
}

#[test]
fn the_binary_alone_prints_the_help_page_without_styling_when_piped() {
    let home = tempfile::tempdir().unwrap();
    let output = command(home.path()).output().unwrap();
    assert!(output.status.success(), "{:?}", output.status);
    let page = String::from_utf8(output.stdout).unwrap();
    assert!(page.starts_with("Mallard from the terminal"), "{page}");
    for section in [
        "USAGE",
        "CORE COMMANDS",
        "FLAGS",
        "EXAMPLES",
        "OUTPUT",
        "LEARN MORE",
    ] {
        assert!(
            page.contains(&format!("\n{section}\n")),
            "{section}: {page}"
        );
    }
    assert!(!page.contains('\x1b'));
    assert!(output.stderr.is_empty());
    // The page never touches local state: no `.mallard` is created to print help.
    assert!(!home.path().join(".mallard").exists());
}
