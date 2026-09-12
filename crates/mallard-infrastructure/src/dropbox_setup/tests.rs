use super::mock::MockDropboxSetup;
use super::*;
use crate::dropbox_auth::SETUP_SCOPES;

fn draft_id() -> StorageSetupDraftId {
    StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap()
}
fn grant(mock: &MockDropboxSetup) -> AuthorizationGrant {
    mock.runtime
        .block_on(mock.client().exchange("code", "verifier", SETUP_SCOPES))
        .unwrap()
}

#[test]
fn creates_exact_folder_accepts_changed_file_id_and_resumes_shared_namespace() {
    for asynchronous in [false, true] {
        let mock = MockDropboxSetup::new();
        mock.state.lock().unwrap().asynchronous = asynchronous;
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let grant = grant(&mock);
        let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        let ns = mock
            .runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
            .unwrap();
        assert_eq!(ns, "shared");
        let entries = StorageSetupJournalStore::dropbox_provisioning(&repository)
            .list()
            .unwrap();
        let text = serde_json::to_string(&entries).unwrap();
        assert!(
            !text.contains("refresh-canary")
                && !text.contains("test-token")
                && !text.contains("code_verifier")
        );
        assert_eq!(entries[0].schema, 1);
        let mut resumed = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert_eq!(
            mock.runtime
                .block_on(prepare_fixture(&mock.client(), &grant, None, &mut resumed))
                .unwrap(),
            "shared"
        );
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert_eq!(mock.count("sharing/share_folder"), 1);
        resumed.finish().unwrap();
        assert!(StorageSetupJournalStore::dropbox_provisioning(&repository)
            .list()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn unknown_mutation_is_never_repeated_after_restart() {
    for route in ["files/create_folder_v2", "sharing/share_folder"] {
        let mock = MockDropboxSetup::new();
        mock.state.lock().unwrap().fail_after = Some(route.into());
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let grant = grant(&mock);
        let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert!(mock
            .runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
            .is_err());
        let mut resumed = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert_eq!(
            mock.runtime
                .block_on(prepare_fixture(&mock.client(), &grant, None, &mut resumed))
                .unwrap_err(),
            SetupError::Uncertain
        );
        assert_eq!(mock.count(route), 1);
        assert_eq!(mock.count("files/create_folder_v2"), 1);
    }
}

#[test]
fn job_and_acknowledged_folder_resume_without_second_mutation() {
    let mock = MockDropboxSetup::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let grant = grant(&mock);
    let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
    mock.state.lock().unwrap().folder_path = Some("/mallard otter".into());
    let mut recovery = Recovery {
        account_id: "dbid:fixture".into(),
        home_namespace_id: "home".into(),
        path: "/Mallard otter".into(),
        stage: FolderStage::Created {
            folder_id: "id:created".into(),
        },
    };
    journal.checkpoint(&recovery).unwrap();
    assert_eq!(
        mock.runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
            .unwrap(),
        "shared"
    );
    recovery.stage = FolderStage::Job {
        job_id: "job:fixture".into(),
    };
    journal.checkpoint(&recovery).unwrap();
    assert_eq!(
        mock.runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
            .unwrap(),
        "shared"
    );
    assert_eq!(mock.count("files/create_folder_v2"), 0);
    assert_eq!(mock.count("sharing/share_folder"), 1);
}

#[test]
fn replacement_namespace_and_changed_account_fail_closed() {
    for replacement in [false, true] {
        let mock = MockDropboxSetup::new();
        {
            let mut state = mock.state.lock().unwrap();
            state.replacement = replacement;
            state.account_changed = !replacement;
        }
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let grant = grant(&mock);
        let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert_eq!(
            mock.runtime
                .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
                .unwrap_err(),
            SetupError::Identity
        );
        assert_eq!(mock.count("files/list_folder"), 0);
    }
}

#[test]
fn reconnect_preserves_namespace_and_rejects_home_without_creating_folder() {
    let mock = MockDropboxSetup::new();
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let grant = grant(&mock);
    let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
    assert_eq!(
        mock.runtime
            .block_on(prepare_fixture(
                &mock.client(),
                &grant,
                Some("shared"),
                &mut journal
            ))
            .unwrap(),
        "shared"
    );
    assert_eq!(
        mock.runtime
            .block_on(prepare_fixture(
                &mock.client(),
                &grant,
                Some("home"),
                &mut journal
            ))
            .unwrap_err(),
        SetupError::Identity
    );
    assert_eq!(mock.count("files/create_folder_v2"), 0);
    assert_eq!(mock.count("sharing/share_folder"), 0);
}

#[cfg(unix)]
#[test]
fn provisioning_journal_rejects_symlink_directory() {
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    std::os::unix::fs::symlink(temp.path(), repository.root().join("dropbox_setup_journal"))
        .unwrap();
    assert!(DropboxSetupJournal::open(&repository, &draft_id()).is_err());
}

#[test]
fn unrelated_errors_do_not_rotate_names() {
    let errors = [
        (400, json!({"error_summary":"bad request"})),
        (401, json!({"error":{".tag":"invalid_access_token"}})),
        (403, json!({"error":{".tag":"no_permission"}})),
        (
            409,
            json!({"error":{".tag":"path","path":{".tag":"conflict","conflict":{".tag":"file_ancestor"}}}}),
        ),
        (
            409,
            json!({"error":{".tag":"path","path":{".tag":"insufficient_space"}}}),
        ),
        (
            409,
            json!({"error_summary":"path/conflict/folder/secret-canary"}),
        ),
        (429, json!({"error":"too_many_requests"})),
        (500, json!({"error":"server"})),
    ];
    for (status, body) in errors {
        let mock = MockDropboxSetup::new();
        mock.state.lock().unwrap().create_error = Some((status, body));
        let temp = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
        let grant = grant(&mock);
        let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert!(mock
            .runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut journal))
            .is_err());
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        let mut resumed = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert!(mock
            .runtime
            .block_on(prepare_fixture(&mock.client(), &grant, None, &mut resumed))
            .is_err());
        let state = mock.state.lock().unwrap();
        assert!(state
            .create_paths
            .iter()
            .all(|path| path == &state.create_paths[0]));
        if status == 500 {
            assert_eq!(state.create_paths.len(), 1);
        }
    }
}

fn prepare_fixture<'a>(
    client: &'a DropboxClient,
    grant: &'a AuthorizationGrant,
    existing: Option<&'a str>,
    journal: &'a mut DropboxSetupJournal,
) -> impl std::future::Future<Output = Result<String, SetupError>> + 'a {
    async move {
        let name = journal
            .folder_name()?
            .unwrap_or_else(|| "Mallard Finch".into());
        prepare_namespace(client, grant, existing, journal, Some(&name)).await
    }
}

#[test]
fn exact_name_conflict_needs_a_new_approved_name() {
    let temp = tempfile::tempdir().unwrap();
    let repository = LocalStateRepository::from_home_dir(temp.path()).unwrap();
    let mock = MockDropboxSetup::new();
    mock.state.lock().unwrap().create_conflicts = 1;
    let grant = grant(&mock);
    let mut journal = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            mock.runtime
                .block_on(prepare_namespace(
                    &mock.client(),
                    &grant,
                    None,
                    &mut journal,
                    Some("My backup")
                ))
                .unwrap_err(),
            SetupError::NameTaken
        );
    }
    assert_eq!(mock.count("files/create_folder_v2"), 1);
    assert_eq!(mock.count("sharing/share_folder"), 0);
    assert_eq!(
        mock.runtime
            .block_on(prepare_namespace(
                &mock.client(),
                &grant,
                None,
                &mut journal,
                Some("My second backup")
            ))
            .unwrap(),
        "shared"
    );
    assert_eq!(
        mock.state.lock().unwrap().create_paths,
        vec!["/My backup", "/My second backup"]
    );
    assert_eq!(mock.count("sharing/share_folder"), 1);
}

#[test]
fn folder_names_are_single_bounded_components() {
    for name in [
        "", ".", "..", " space", "space ", "a/b", "a\\b", "a\n", "a\u{85}",
    ] {
        assert!(!valid_folder_name(name));
    }
    assert!(!valid_folder_name(&"é".repeat(128)));
    assert!(valid_folder_name("My backup 2026"));
    assert!(valid_folder_name(&"é".repeat(127)));
}
