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
            .block_on(prepare_namespace(
                &mock.client(),
                &grant,
                None,
                &mut journal,
            ))
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
                .block_on(prepare_namespace(
                    &mock.client(),
                    &grant,
                    None,
                    &mut resumed
                ))
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
            .block_on(prepare_namespace(
                &mock.client(),
                &grant,
                None,
                &mut journal
            ))
            .is_err());
        let mut resumed = DropboxSetupJournal::open(&repository, &draft_id()).unwrap();
        assert_eq!(
            mock.runtime
                .block_on(prepare_namespace(
                    &mock.client(),
                    &grant,
                    None,
                    &mut resumed
                ))
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
    mock.state.lock().unwrap().folder_path = Some(journal.path().to_lowercase());
    let mut recovery = Recovery {
        account_id: "dbid:fixture".into(),
        home_namespace_id: "home".into(),
        stage: FolderStage::Created {
            folder_id: "id:created".into(),
        },
    };
    journal.checkpoint(&recovery).unwrap();
    assert_eq!(
        mock.runtime
            .block_on(prepare_namespace(
                &mock.client(),
                &grant,
                None,
                &mut journal
            ))
            .unwrap(),
        "shared"
    );
    recovery.stage = FolderStage::Job {
        job_id: "job:fixture".into(),
    };
    journal.checkpoint(&recovery).unwrap();
    assert_eq!(
        mock.runtime
            .block_on(prepare_namespace(
                &mock.client(),
                &grant,
                None,
                &mut journal
            ))
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
                .block_on(prepare_namespace(
                    &mock.client(),
                    &grant,
                    None,
                    &mut journal
                ))
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
            .block_on(prepare_namespace(
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
            .block_on(prepare_namespace(
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
