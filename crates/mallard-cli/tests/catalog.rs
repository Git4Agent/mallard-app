//! The Library from two computers: A saves a Codex home that holds one
//! skill, adds the Skills row, links it to a local-folder storage, and pushes;
//! B does the same and pulls the skill in. Reviews print first and apply only
//! with --approve.

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
    codex_home: tempfile::TempDir,
}

impl Machine {
    fn new(storage_dir: &Path) -> Self {
        let machine = Self {
            home: tempfile::tempdir().unwrap(),
            codex_home: tempfile::tempdir().unwrap(),
        };
        local_storage(machine.home.path(), storage_dir, "Shared");
        machine
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mallard-cli"));
        command
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.home.path())
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
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn fail(&self, args: &[&str]) -> (i32, serde_json::Value) {
        let output = self.command(args);
        assert!(!output.status.success(), "{args:?} succeeded unexpectedly");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let error: serde_json::Value =
            serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        (output.status.code().unwrap(), error)
    }

    /// The Skills row of this computer's Codex home, linked to the storage.
    fn skills_scope(&self) -> String {
        let codex_home = self.codex_home.path().to_string_lossy().into_owned();
        let profile = self.succeed(&[
            "catalog",
            "profile",
            "add",
            "--provider",
            "codex",
            "--path",
            &codex_home,
        ]);
        assert_eq!(profile["status"], "added");
        let profile_id = profile["profile"]["profile_id"]
            .as_str()
            .unwrap()
            .to_string();
        let scope = self.succeed(&[
            "catalog",
            "scope",
            "add",
            "--provider",
            "codex",
            "--profile",
            &profile_id,
            "--kind",
            "agent_skill",
            "--format",
            "agentskills.io.skill",
        ]);
        assert_eq!(scope["status"], "added");
        let scope_id = scope["scope"]["scope"]["scope_id"]
            .as_str()
            .unwrap()
            .to_string();
        let linked = self.succeed(&["catalog", "link", &scope_id, "--storage", "Shared"]);
        assert_eq!(linked["status"], "linked");
        scope_id
    }

    fn write_skill(&self, name: &str, body: &str) {
        let root = self.codex_home.path().join("skills").join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: A CLI fixture\n---\n\n{body}\n"),
        )
        .unwrap();
    }
}

fn local_storage(home: &Path, directory: &Path, name: &str) {
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
    .unwrap();
}

#[test]
fn a_skill_pushed_from_one_computer_is_pulled_on_another() {
    let storage_dir = tempfile::tempdir().unwrap();
    let a = Machine::new(storage_dir.path());
    a.write_skill("alpha", "Alpha, first edition.");

    let capabilities = a.succeed(&["catalog", "capabilities"]);
    assert!(capabilities["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|capability| capability["resource_kind"] == "agent_skill"));

    let scope = a.skills_scope();
    let items = a.succeed(&["catalog", "items", &scope]);
    let alpha = items["items"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["display_name"] == "alpha")
        .expect("the home's skill is listed");
    let alpha_id = alpha["resource_id"].as_str().unwrap().to_string();
    assert_eq!(alpha["local_present"], true);

    // The review first, then the push.
    let review = a.succeed(&["catalog", "push", &scope, "--storage", "Shared"]);
    assert_eq!(review["status"], "reviewed");
    let rows = review["plan"]["rows"].as_array().unwrap();
    assert!(
        rows.iter().any(|row| row["row"]["resource_id"] == alpha_id),
        "{review}"
    );
    let pushed = a.succeed(&[
        "catalog",
        "push",
        &scope,
        "--storage",
        "Shared",
        "--approve",
    ]);
    assert_eq!(pushed["status"], "pushed", "{pushed}");
    assert_eq!(pushed["outcome"]["published"], true);

    let repositories = a.succeed(&["catalog", "repositories", "--storage", "Shared"]);
    assert_eq!(
        repositories["repositories"][0]["resource_names"][0],
        "alpha"
    );

    // B: the same row on another computer, then a Pull.
    let b = Machine::new(storage_dir.path());
    let scope_b = b.skills_scope();
    let review = b.succeed(&["catalog", "pull", &scope_b, "--storage", "Shared"]);
    assert_eq!(review["status"], "reviewed");
    let items = review["plan"]["items"].as_array().unwrap();
    let incoming = items
        .iter()
        .find(|item| item["item"]["display_name"] == "alpha")
        .expect("the stored skill is reviewed");
    assert_eq!(incoming["item"]["disposition"]["kind"], "executable");
    assert!(incoming["item"]["default_action_id"].is_string());

    let pulled = b.succeed(&[
        "catalog",
        "pull",
        &scope_b,
        "--storage",
        "Shared",
        "--approve",
    ]);
    assert_eq!(pulled["status"], "pulled", "{pulled}");
    let installed = b.codex_home.path().join("skills/alpha/SKILL.md");
    assert!(
        std::fs::read_to_string(&installed)
            .unwrap()
            .contains("Alpha, first edition."),
        "the skill landed in B's Codex home"
    );

    // A local copy under a new name, reviewed then applied.
    let fork_review = b.succeed(&[
        "catalog",
        "fork",
        &scope_b,
        &alpha_id,
        "--name",
        "alpha-copy",
    ]);
    assert_eq!(fork_review["status"], "reviewed");
    assert_eq!(fork_review["plan"]["copy_name"], "alpha-copy");
    let forked = b.succeed(&[
        "catalog",
        "fork",
        &scope_b,
        &alpha_id,
        "--name",
        "alpha-copy",
        "--approve",
    ]);
    assert_eq!(forked["status"], "forked", "{forked}");
    assert!(b
        .codex_home
        .path()
        .join("skills/alpha-copy/SKILL.md")
        .is_file());

    // Removal reviews the file count, then deletes only with --approve.
    let items = b.succeed(&["catalog", "items", &scope_b]);
    let copy_id = items["items"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["display_name"] == "alpha-copy")
        .and_then(|row| row["resource_id"].as_str())
        .unwrap()
        .to_string();
    let removal = b.succeed(&["catalog", "remove", &scope_b, &copy_id]);
    assert_eq!(removal["status"], "reviewed");
    assert_eq!(removal["plan"]["file_count"], 1);
    assert!(b
        .codex_home
        .path()
        .join("skills/alpha-copy/SKILL.md")
        .is_file());
    let removed = b.succeed(&["catalog", "remove", &scope_b, &copy_id, "--approve"]);
    assert_eq!(removed["status"], "removed", "{removed}");
    assert!(!b.codex_home.path().join("skills/alpha-copy").exists());

    // Destructive and unknown inputs are refused before anything runs.
    let (code, error) = b.fail(&["catalog", "unlink", &scope_b, "--storage", "Shared"]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "confirmation_required");
    let (code, error) = b.fail(&["catalog", "items", "not-a-scope"]);
    assert_eq!(code, 2);
    assert_eq!(error["code"], "catalog_scope_id_invalid");
}
