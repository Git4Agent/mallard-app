//! Two computers, one local-folder storage: A registers a non-Git project and
//! pushes its files; B connects to the stored bundle, reviews the Pull, and
//! applies the actions it approves. Every step is the binary, against
//! temporary homes; the review travels between the two steps as the JSON
//! document the first printed.

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

struct Machine {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
    agent_home: tempfile::TempDir,
}

impl Machine {
    fn new(storage_dir: &Path) -> Self {
        let machine = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
            agent_home: tempfile::tempdir().unwrap(),
        };
        local_storage(machine.home.path(), storage_dir, "Shared");
        machine
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mallard-cli"));
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.project.path())
            .arg("--json")
            .args(args);
        command.output().unwrap()
    }

    fn succeed(&self, args: &[&str]) -> serde_json::Value {
        let output = self.command(args);
        assert!(
            output.status.success(),
            "{args:?} exited {:?}\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            let event: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|_| panic!("stderr is NDJSON under --json, got {line:?}"));
            assert_eq!(event["schema"], 1);
            assert!(event["status"] == "event" || event["status"] == "progress");
        }
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// A refused verb prints nothing on stdout; its stderr ends with the
    /// error document, after whatever the operation narrated first.
    fn fail(&self, args: &[&str]) -> (i32, serde_json::Value) {
        let output = self.command(args);
        assert!(!output.status.success(), "{args:?} succeeded unexpectedly");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        let last = stderr.lines().last().expect("an error line");
        let error: serde_json::Value = serde_json::from_str(last).unwrap();
        assert_eq!(error["status"], "error", "{stderr}");
        (output.status.code().unwrap(), error)
    }

    fn agent_home_arg(&self) -> String {
        format!("codex={}", self.agent_home.path().display())
    }

    /// The plain-text stdout of a verb that succeeds without `--json`.
    fn text(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_mallard-cli"))
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.project.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?} exited {:?}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

fn local_storage(home: &Path, directory: &Path, name: &str) -> String {
    let repository = LocalStateRepository::from_home_dir(home).unwrap();
    let registry = StorageProviderRegistry::production();
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
    commit_storage_setup_draft(
        &repository,
        registry,
        runtime.handle(),
        &draft.draft_id,
        draft.revision,
        |_| Ok(()),
    )
    .unwrap()
    .storage_id
    .to_string()
}

#[test]
fn a_project_pushed_from_one_computer_is_pulled_on_another() {
    let storage_dir = tempfile::tempdir().unwrap();
    let a = Machine::new(storage_dir.path());
    std::fs::create_dir_all(a.project.path().join("notes")).unwrap();
    std::fs::write(a.project.path().join("notes/todo.md"), "- ship the CLI\n").unwrap();

    // A: register the folder, bound to an agent home, linked to the storage.
    let added = a.succeed(&[
        "project",
        "add",
        &a.project.path().to_string_lossy(),
        "--provider",
        &a.agent_home_arg(),
        "--storage",
        "Shared",
        "--name",
        "Notes",
    ]);
    assert_eq!(added["status"], "added");
    assert_eq!(added["project"]["project"]["display_name"], "Notes");
    assert_eq!(added["project"]["links"].as_array().unwrap().len(), 1);
    let bundle_id = added["project"]["project"]["bundle_id"]
        .as_str()
        .unwrap()
        .to_string();

    // The current folder resolves the project from here on.
    let status = a.succeed(&["status"]);
    assert_eq!(status["current_project"]["display_name"], "Notes");
    let links = a.succeed(&["link", "list"]);
    assert_eq!(links["links"][0]["bundle_id"], bundle_id);

    let sessions = a.succeed(&["session", "list"]);
    assert_eq!(sessions["activity"]["sessions"], serde_json::json!([]));

    let files = a.succeed(&["project", "files"]);
    let todo = files["files"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["relative_path"] == "notes/todo.md")
        .expect("the scan lists the file");
    assert_eq!(todo["undecided"], true);

    // Review, then apply with the project's files.
    let review = a.succeed(&["push", "review"]);
    assert_eq!(review["status"], "reviewed");
    assert_eq!(
        review["review"]["project"]["project"]["bundle_id"],
        bundle_id
    );
    // A non-Git project's review carries the file scan, so the agent sees
    // what --files would push.
    let scanned = review["review"]["project_files"]["entries"]
        .as_array()
        .unwrap();
    assert!(
        scanned
            .iter()
            .any(|entry| entry["relative_path"] == "notes/todo.md"),
        "{review}"
    );
    let text = a.text(&["push", "review"]);
    assert!(text.contains("Project files: 2 entries"), "{text}");
    assert!(text.contains("  [ ] notes/todo.md — "), "{text}");
    assert!(text.contains("push apply --files"), "{text}");
    let pushed = a.succeed(&["push", "apply", "--files"]);
    assert_eq!(pushed["status"], "pushed", "{pushed}");
    let push_status = a.succeed(&["push", "status"]);
    assert_eq!(push_status["destination"]["state"], "ready");

    let snapshots = a.succeed(&["storage", "snapshots", "Shared"]);
    assert_eq!(snapshots["snapshots"][0]["bundle_id"], bundle_id);
    assert_eq!(snapshots["snapshots"][0]["registration_only"], false);

    // B: a second computer sharing the storage folder connects to the bundle.
    let b = Machine::new(storage_dir.path());
    let connected = b.succeed(&[
        "project",
        "add",
        &b.project.path().to_string_lossy(),
        "--provider",
        &b.agent_home_arg(),
        "--storage",
        "Shared",
        "--connect",
        &bundle_id,
        "--name",
        "Notes copy",
    ]);
    assert_eq!(connected["status"], "added");
    assert_eq!(connected["project"]["project"]["bundle_id"], bundle_id);
    // The bundle keeps the name A published; B's name is its nickname.
    assert_eq!(connected["project"]["project"]["display_name"], "Notes");
    assert_eq!(
        connected["project"]["project"]["local_alias"],
        "Notes copy"
    );

    let readiness = b.succeed(&["pull", "readiness"]);
    assert_eq!(readiness["readiness"]["bundle_id"], bundle_id);

    let review = b.succeed(&["pull", "review"]);
    assert_eq!(review["status"], "reviewed");
    let actions = review["review"]["actions"].as_array().unwrap();
    let file_actions: Vec<&str> = actions
        .iter()
        .filter(|action| action["kind"]["kind"] == "write_project_file")
        .map(|action| action["action_id"].as_str().unwrap())
        .collect();
    assert!(!file_actions.is_empty(), "{review}");

    // The text review says the same: nothing here is covered by `all`.
    let text = b.text(&["pull", "review"]);
    assert!(text.contains("[?] "), "{text}");
    assert!(
        text.contains("Every action needs explicit approval"),
        "{text}"
    );
    let review_path = b.home.path().join("pull-review.json");
    std::fs::write(&review_path, serde_json::to_vec(&review).unwrap()).unwrap();

    // Project files need explicit approval: `all` alone approves none of them.
    let (code, error) = b.fail(&[
        "pull",
        "apply",
        "--review",
        &review_path.to_string_lossy(),
        "--approve",
        "all",
    ]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "nothing_approved");

    let mut args = vec![
        "pull".to_string(),
        "apply".to_string(),
        "--review".to_string(),
        review_path.to_string_lossy().into_owned(),
    ];
    for action in actions {
        args.push("--approve".to_string());
        args.push(action["action_id"].as_str().unwrap().to_string());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let pulled = b.succeed(&args);
    assert_eq!(pulled["status"], "pulled", "{pulled}");
    assert_eq!(
        std::fs::read_to_string(b.project.path().join("notes/todo.md")).unwrap(),
        "- ship the CLI\n"
    );

    // Applying the same review again changes nothing and is allowed: a review
    // pins one version, and that version is still there. `--resource` is an
    // approval on its own, explicit approval included.
    let mut resource_args = vec![
        "pull".to_string(),
        "apply".to_string(),
        "--review".to_string(),
        review_path.to_string_lossy().into_owned(),
    ];
    for action in actions {
        resource_args.push("--resource".to_string());
        resource_args.push(action["resource_id"].as_str().unwrap().to_string());
    }
    let resource_args: Vec<&str> = resource_args.iter().map(String::as_str).collect();
    let again = b.succeed(&resource_args);
    assert_eq!(again["status"], "pulled", "{again}");

    // A pushes a change. B's old review still names a version the storage
    // holds, so it stays applicable until B syncs past it.
    std::fs::write(
        a.project.path().join("notes/todo.md"),
        "- ship the CLI\n- write the docs\n",
    )
    .unwrap();
    let pushed_again = a.succeed(&["push", "apply", "--files"]);
    assert_eq!(pushed_again["status"], "pushed", "{pushed_again}");
    let fresh = b.succeed(&["pull", "review"]);
    assert!(
        fresh["review"]["revision"].as_u64().unwrap()
            > review["review"]["revision"].as_u64().unwrap()
    );
    let fresh_path = b.home.path().join("pull-review-2.json");
    std::fs::write(&fresh_path, serde_json::to_vec(&fresh).unwrap()).unwrap();
    let mut fresh_args = vec![
        "pull".to_string(),
        "apply".to_string(),
        "--review".to_string(),
        fresh_path.to_string_lossy().into_owned(),
    ];
    for action in fresh["review"]["actions"].as_array().unwrap() {
        fresh_args.push("--approve".to_string());
        fresh_args.push(action["action_id"].as_str().unwrap().to_string());
    }
    let fresh_args: Vec<&str> = fresh_args.iter().map(String::as_str).collect();
    assert_eq!(b.succeed(&fresh_args)["status"], "pulled");
    assert_eq!(
        std::fs::read_to_string(b.project.path().join("notes/todo.md")).unwrap(),
        "- ship the CLI\n- write the docs\n"
    );

    // Now the old review is behind what B has reviewed: refused, exit 3.
    let (code, error) = b.fail(&args);
    assert_eq!(code, 3, "{error}");
    assert_eq!(error["code"], "review_superseded");

    // The activity log on B tells the story, as the desktop's drawer would.
    let log = b.succeed(&["log", "query", "--type", "pull"]);
    let events: Vec<&str> = log["page"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["event"].as_str().unwrap())
        .collect();
    assert!(events.contains(&"pull.review_ready"), "{events:?}");
    assert!(events.contains(&"pull.completed"), "{events:?}");
}

#[test]
fn destructive_session_and_link_verbs_require_confirmation() {
    let storage_dir = tempfile::tempdir().unwrap();
    let a = Machine::new(storage_dir.path());
    a.succeed(&[
        "project",
        "add",
        &a.project.path().to_string_lossy(),
        "--provider",
        &a.agent_home_arg(),
        "--storage",
        "Shared",
    ]);
    let (code, error) = a.fail(&["link", "remove", "--storage", "Shared"]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "confirmation_required");
    let (code, error) = a.fail(&["session", "delete-all", "--expected-revision", "1"]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "confirmation_required");

    let removed = a.succeed(&["link", "remove", "--storage", "Shared", "--confirm"]);
    assert_eq!(removed["status"], "removed");
    let (code, error) = a.fail(&["push", "review"]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "storage_required");
}

/// Registering a project publishes an empty version 1; a second computer
/// that connects before the first push has nothing to pull, and both the
/// review and a premature apply say so instead of blaming the approval.
#[test]
fn connecting_before_the_first_push_has_nothing_to_pull() {
    let storage_dir = tempfile::tempdir().unwrap();
    let a = Machine::new(storage_dir.path());
    std::fs::write(a.project.path().join("todo.md"), "- ship the CLI\n").unwrap();
    let added = a.succeed(&[
        "project",
        "add",
        &a.project.path().to_string_lossy(),
        "--provider",
        &a.agent_home_arg(),
        "--storage",
        "Shared",
    ]);
    let bundle_id = added["project"]["project"]["bundle_id"]
        .as_str()
        .unwrap()
        .to_string();

    let b = Machine::new(storage_dir.path());
    b.succeed(&[
        "project",
        "add",
        &b.project.path().to_string_lossy(),
        "--provider",
        &b.agent_home_arg(),
        "--storage",
        "Shared",
        "--connect",
        &bundle_id,
    ]);
    let review = b.succeed(&["pull", "review"]);
    assert_eq!(review["review"]["revision"], 1);
    assert_eq!(review["review"]["actions"], serde_json::json!([]));
    assert_eq!(review["review"]["conversation_review"], serde_json::json!([]));

    let text = b.text(&["pull", "review"]);
    assert!(
        text.contains("Nothing to pull: version 1 holds nothing this project lacks."),
        "{text}"
    );
    assert!(!text.contains("--approve all"), "{text}");

    let review_file = b.home.path().join("review.json");
    std::fs::write(&review_file, serde_json::to_vec(&review).unwrap()).unwrap();
    let (code, error) = b.fail(&[
        "pull",
        "apply",
        "--review",
        &review_file.to_string_lossy(),
        "--approve",
        "all",
    ]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "nothing_to_pull");
    assert_eq!(
        error["message"],
        "Nothing to pull: version 1 holds nothing this project lacks."
    );
}
