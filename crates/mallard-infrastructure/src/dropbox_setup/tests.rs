use super::mock::MockDropboxSetup;
use super::*;
use crate::dropbox_auth::SETUP_SCOPES;

fn grant(mock: &MockDropboxSetup) -> AuthorizationGrant {
    mock.runtime
        .block_on(mock.client().exchange("code", "verifier", SETUP_SCOPES))
        .unwrap()
}
fn prepare(
    mock: &MockDropboxSetup,
    grant: &AuthorizationGrant,
    existing: Option<&str>,
    name: Option<&str>,
) -> Result<String, SetupError> {
    mock.runtime
        .block_on(prepare_namespace(&mock.client(), grant, existing, name))
}

#[test]
fn creates_and_shares_an_exact_folder_and_a_repeat_finds_the_name_taken() {
    for asynchronous in [false, true] {
        let mock = MockDropboxSetup::new();
        mock.state.lock().unwrap().asynchronous = asynchronous;
        let grant = grant(&mock);
        assert_eq!(
            prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap(),
            "shared"
        );
        assert_eq!(mock.count("files/create_folder_v2"), 1);
        assert_eq!(mock.count("sharing/share_folder"), 1);
        assert_eq!(
            mock.count("sharing/check_share_job_status"),
            usize::from(asynchronous)
        );
        // Nothing remembers the attempt: the same name now names the folder
        // that exists, and nothing is shared again.
        assert_eq!(
            prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap_err(),
            SetupError::NameTaken
        );
        assert_eq!(mock.count("files/create_folder_v2"), 2);
        assert_eq!(mock.count("sharing/share_folder"), 1);
    }
}

#[test]
fn a_lost_answer_is_reported_and_the_retry_finds_the_folder_taken() {
    for route in ["files/create_folder_v2", "sharing/share_folder"] {
        let mock = MockDropboxSetup::new();
        mock.state.lock().unwrap().fail_after = Some(route.into());
        let grant = grant(&mock);
        let error = prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap_err();
        assert!(matches!(error, SetupError::Auth(_)), "{error:?}");
        assert!(!format!("{error:?}").contains("response-lost-canary"));
        // The folder was made before the answer was lost, so it stays and
        // the same name is now taken. Nothing is shared a second time.
        assert_eq!(
            prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap_err(),
            SetupError::NameTaken
        );
        assert_eq!(mock.state.lock().unwrap().occupied_paths.len(), 1);
        assert_eq!(mock.count("files/create_folder_v2"), 2);
        assert_eq!(
            mock.count("sharing/share_folder"),
            usize::from(route == "sharing/share_folder")
        );
    }
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
        let grant = grant(&mock);
        assert_eq!(
            prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap_err(),
            SetupError::Identity
        );
        assert_eq!(mock.count("files/list_folder"), 0);
    }
}

#[test]
fn reconnect_preserves_namespace_and_rejects_home_without_creating_folder() {
    let mock = MockDropboxSetup::new();
    let grant = grant(&mock);
    assert_eq!(
        prepare(&mock, &grant, Some("shared"), None).unwrap(),
        "shared"
    );
    assert_eq!(
        prepare(&mock, &grant, Some("home"), None).unwrap_err(),
        SetupError::Identity
    );
    assert_eq!(mock.count("files/create_folder_v2"), 0);
    assert_eq!(mock.count("sharing/share_folder"), 0);
}

#[test]
fn a_missing_or_invalid_folder_name_is_refused_before_any_mutation() {
    let mock = MockDropboxSetup::new();
    let grant = grant(&mock);
    for name in [None, Some("a/b"), Some(" space")] {
        assert_eq!(
            prepare(&mock, &grant, None, name).unwrap_err(),
            SetupError::Identity
        );
    }
    assert_eq!(mock.count("files/create_folder_v2"), 0);
}

#[test]
fn unrelated_errors_keep_the_same_name_on_retry() {
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
        let grant = grant(&mock);
        for _ in 0..2 {
            let error = prepare(&mock, &grant, None, Some("Mallard Finch")).unwrap_err();
            assert_ne!(error, SetupError::NameTaken, "{status}");
            assert!(!format!("{error:?}").contains("secret-canary"));
        }
        let state = mock.state.lock().unwrap();
        assert_eq!(state.create_paths, ["/Mallard Finch", "/Mallard Finch"]);
        assert!(state.occupied_paths.is_empty());
    }
}

#[test]
fn an_exact_name_conflict_is_taken_and_another_name_is_shared() {
    let mock = MockDropboxSetup::new();
    mock.state.lock().unwrap().create_conflicts = 1;
    let grant = grant(&mock);
    assert_eq!(
        prepare(&mock, &grant, None, Some("My backup")).unwrap_err(),
        SetupError::NameTaken
    );
    assert_eq!(mock.count("files/create_folder_v2"), 1);
    assert_eq!(mock.count("sharing/share_folder"), 0);
    assert_eq!(
        prepare(&mock, &grant, None, Some("My second backup")).unwrap(),
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
