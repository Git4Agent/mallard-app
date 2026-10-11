//! Native same-thread revert, followed by schema-1 Push, Pull, and CLI resume.
//! Only Codex writes the source rollouts. Mallard must select the replacement
//! while retaining the referenced prefix of the old physical file.

use super::*;

pub(super) const EXPECTED_REQUESTS: usize = 4;
const KEEP_USER: &str = "LIVE_USER_REVERT_KEEP";
const KEEP_AGENT: &str = "MALLARD_REVERT_KEEP";
const DISCARD_USER: &str = "LIVE_USER_REVERT_DISCARD";
const DISCARD_AGENT: &str = "MALLARD_REVERT_DISCARD";
const REPLACE_USER: &str = "LIVE_USER_REVERT_REPLACEMENT";
const REPLACE_AGENT: &str = "MALLARD_REVERT_REPLACEMENT";
const VERIFY_USER: &str = "LIVE_USER_REVERT_VERIFY";

fn expected_history() -> Vec<HistoryMarker> {
    marker_turn(KEEP_USER, KEEP_AGENT)
        .into_iter()
        .chain(marker_turn(REPLACE_USER, REPLACE_AGENT))
        .collect()
}

pub(super) fn scenarios() -> Vec<AimockScenario> {
    vec![
        aimock_text_scenario("revert_keep", KEEP_USER, vec![], KEEP_AGENT),
        aimock_text_scenario(
            "revert_discard",
            DISCARD_USER,
            marker_turn(KEEP_USER, KEEP_AGENT),
            DISCARD_AGENT,
        ),
        aimock_text_scenario(
            "revert_replace",
            REPLACE_USER,
            marker_turn(KEEP_USER, KEEP_AGENT),
            REPLACE_AGENT,
        ),
        aimock_text_scenario(
            "revert_resume",
            VERIFY_USER,
            expected_history(),
            HEADLESS_REOPEN_ACK,
        ),
    ]
}

pub(super) fn run(provider: &LocalAimockProvider) {
    eprintln!("live Codex phase: native revert with retained old rollout");
    let root = tempfile::tempdir().expect("create isolated revert fixture");
    let process_home = root.path().join("process-home");
    let source_home = root.path().join("source-codex");
    let target_home = root.path().join("target-codex");
    let source_project = root.path().join("source-project");
    let target_project = root.path().join("target-project");
    let storage_root = root.path().join("storage");
    for path in [
        &process_home,
        &source_project,
        &target_project,
        &storage_root,
    ] {
        std::fs::create_dir(path).expect("create revert fixture directory");
    }
    create_secure_codex_home(&source_home);
    create_secure_codex_home(&target_home);
    // Match the local path spelling used by Codex and Mallard on Windows.
    let source_home = canonicalize(source_home).unwrap();
    let target_home = canonicalize(target_home).unwrap();
    let source_project = canonicalize(source_project).unwrap();
    let target_project = canonicalize(target_project).unwrap();
    let source_repository =
        LocalStateRepository::from_home_dir(&root.path().join("source-machine")).unwrap();
    let target_repository =
        LocalStateRepository::from_home_dir(&root.path().join("target-machine")).unwrap();
    let source = ProjectSyncService::new(&source_repository);
    let target = ProjectSyncService::new(&target_repository);
    let storage = local_storage(&storage_root);
    let storage_id = storage.id.clone();
    let detail = finish_setup(
        &source_repository,
        &source,
        &source_project,
        &source_home,
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let project_id = &detail.project.local_project_id;
    let bundle_id = &detail.project.bundle_id;

    let mut server = CodexAppServer::start(provider, &source_home, &process_home, &source_project)
        .expect("start isolated revert app-server");
    let thread_id = server
        .thread_start_paginated(&source_project)
        .expect("start revert thread");
    server
        .turn_start_and_wait(&thread_id, KEEP_USER)
        .expect("complete retained turn");
    let discarded_turn = server
        .turn_start_and_wait(&thread_id, DISCARD_USER)
        .expect("complete discarded turn");
    let old_path = rollout_path(&source_home, &thread_id);
    // Warm discovery while the original file is still current. The revert
    // must invalidate that selection even though the old file remains.
    let initial_review = source.prepare_push_review(project_id, &storage_id).unwrap();
    assert_eq!(
        initial_review
            .thread_comparison
            .as_ref()
            .unwrap()
            .entries
            .len(),
        1
    );

    let session_resource = initial_review.thread_comparison.as_ref().unwrap().entries[0]
        .conversation_resource_id
        .clone();
    let initial_snapshot = assert_detail_history(
        &source,
        project_id,
        &session_resource,
        &marker_turn(KEEP_USER, KEEP_AGENT)
            .into_iter()
            .chain(marker_turn(DISCARD_USER, DISCARD_AGENT))
            .collect::<Vec<_>>(),
    );

    let response = server
        .request(
            "thread/revert",
            json!({
                "threadId": thread_id, "beforeTurnId": discarded_turn,
            }),
        )
        .expect("revert before discarded turn");
    assert_eq!(
        response
            .pointer("/result/thread/id")
            .and_then(Value::as_str),
        Some(thread_id.as_str())
    );
    // Codex can return a verbatim Windows path for the same local file.
    let selected_path = canonicalize(
        response
            .pointer("/result/thread/path")
            .and_then(Value::as_str)
            .expect("revert response has no physical path"),
    )
    .expect("canonicalize reverted rollout path");
    assert_ne!(
        selected_path, old_path,
        "revert must create a distinct physical rollout"
    );
    assert!(selected_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(&format!("-{thread_id}_")));
    let old_bytes = read_bounded_file(
        &old_path,
        MAX_CONVERSATION_FILE_BYTES,
        "retained old rollout",
    );
    assert!(byte_slice_contains(&old_bytes, DISCARD_AGENT.as_bytes()));
    server
        .turn_start_and_wait(&thread_id, REPLACE_USER)
        .expect("complete replacement turn without discarded history");
    drop(server);

    assert_eq!(rollout_path(&source_home, &thread_id), selected_path);
    assert_eq!(
        historical_rollout_lookup(&source_home, &thread_id).unwrap(),
        Some(old_path.clone())
    );
    assert_eq!(
        read_bounded_file(
            &old_path,
            MAX_CONVERSATION_FILE_BYTES,
            "old rollout after replacement"
        ),
        old_bytes
    );
    let current_bytes = read_bounded_file(
        &selected_path,
        MAX_CONVERSATION_FILE_BYTES,
        "reverted current rollout",
    );
    let metadata: Value =
        serde_json::from_slice(current_bytes.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
    assert_eq!(metadata["payload"]["id"].as_str(), Some(thread_id.as_str()));
    assert_eq!(
        metadata["payload"]["history_base"]["thread_id"].as_str(),
        Some(thread_id.as_str())
    );
    assert!(
        metadata["payload"]["history_base"]["end_byte_offset"]
            .as_u64()
            .unwrap()
            < old_bytes.len() as u64
    );
    let review = source.prepare_push_review(project_id, &storage_id).unwrap();
    let rows = &review.thread_comparison.as_ref().unwrap().entries;
    assert_eq!(
        rows.len(),
        1,
        "old and reverted rollouts must produce one resource"
    );
    assert_eq!(
        rows[0].provider_thread_id.as_deref(),
        Some(thread_id.as_str())
    );
    assert!(
        rows[0].push_decision.selectable,
        "reverted conversation must be publishable"
    );
    let stale_page = source
        .get_project_session_timeline_page(
            project_id,
            &session_resource,
            history_request(initial_snapshot, None),
        )
        .unwrap_err();
    assert_eq!(
        stale_page.code,
        mallard_infrastructure::session_detail::SessionDetailErrorCode::SnapshotChanged
    );
    assert_detail_history(&source, project_id, &session_resource, &expected_history());
    let result = source
        .push_reviewed(
            project_id,
            &storage_id,
            PushApproval::recommended(&review, &storage_id),
        )
        .unwrap();
    assert!(
        result.success,
        "Push must publish the selected reverted history"
    );
    let logical_id = logical_thread_id(&source_repository, project_id, &thread_id);
    let cloud_id = cloud_thread_id_of(&source_repository, project_id, &logical_id);
    let wire =
        portable_conversation_rollout(&source, &storage_id, bundle_id, &storage_root, &cloud_id);
    for marker in [KEEP_USER, KEEP_AGENT, REPLACE_USER, REPLACE_AGENT] {
        assert!(
            byte_slice_contains(&wire, marker.as_bytes()),
            "portable history lost a retained marker"
        );
    }
    for marker in [DISCARD_USER, DISCARD_AGENT] {
        assert!(
            !byte_slice_contains(&wire, marker.as_bytes()),
            "portable history retained a discarded turn"
        );
    }

    let target_detail = finish_setup(
        &target_repository,
        &target,
        &target_project,
        &target_home,
        storage,
        DraftRepositoryChoice::Existing {
            storage_id: storage_id.clone(),
            bundle_id: bundle_id.clone(),
            display_name: detail.project.display_name.clone(),
            repository_fingerprint: detail.project.repository_fingerprint.clone(),
            mismatch_acknowledged: true,
        },
    );
    pull_and_apply(
        &target,
        &storage_id,
        bundle_id,
        target_detail.binding.as_ref().unwrap(),
    );
    let target_logical = local_thread_of_cloud(
        &target_repository,
        &target_detail.project.local_project_id,
        &cloud_id,
    );
    let target_id = provider_thread_id(
        &target_repository,
        &target_detail.project.local_project_id,
        &target_logical,
    );
    let target_resource =
        mallard_domain::conversation_resource_id(&codex_key(), &target_logical).unwrap();
    assert_detail_history(
        &target,
        &target_detail.project.local_project_id,
        &target_resource,
        &expected_history(),
    );
    assert_pulled_standalone_rollout(
        &target_home,
        &target_project,
        &target_id,
        &expected_history(),
    );
    // The destination has no original history source. Native CLI resume must
    // still send precisely the retained and replacement turns to the mock.
    assert!(historical_rollout_lookup(&target_home, &thread_id)
        .unwrap()
        .is_none());
    let mut turns = 0;
    assert_eq!(
        run_codex(
            &mut turns,
            provider,
            &target_home,
            &process_home,
            &target_project,
            CodexRunAccess::ReadOnly,
            Some(&target_id),
            VERIFY_USER,
            HEADLESS_REOPEN_ACK
        ),
        target_id
    );
    assert_eq!(turns, 1);
    assert_eq!(
        read_bounded_file(
            &old_path,
            MAX_CONVERSATION_FILE_BYTES,
            "old rollout after Push and Pull"
        ),
        old_bytes
    );
}

fn history_request(snapshot_id: String, cursor: Option<String>) -> ProjectSessionTimelineRequest {
    ProjectSessionTimelineRequest {
        snapshot_id,
        cursor,
        limit: Some(1),
        included_event_kinds: BTreeSet::from([
            SessionTimelineEventKind::Prompt,
            SessionTimelineEventKind::Response,
        ]),
        included_tool_categories: BTreeSet::new(),
        turn_id: None,
    }
}

pub(super) fn assert_detail_history(
    service: &ProjectSyncService<'_>,
    project: &LocalProjectId,
    resource: &ResourceId,
    expected: &[HistoryMarker],
) -> String {
    let header = service
        .get_project_session_detail(project, resource)
        .expect("load retained Session Detail");
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut history = Vec::new();
    loop {
        let page = service
            .get_project_session_timeline_page(
                project,
                resource,
                history_request(header.snapshot_id.clone(), cursor),
            )
            .expect("read retained Session Detail page");
        for event in page.events {
            assert!(seen.insert(event.event_id));
            assert!(seen.len() <= MAX_SESSION_DETAIL_EVENTS);
            let text = event.text.unwrap();
            let marker = match event.kind {
                SessionTimelineEventKind::Prompt => normalize_position_zero_marker(&text)
                    .ok()
                    .filter(|marker| marker.starts_with("LIVE_USER_"))
                    .map(|marker| HistoryMarker {
                        role: HistoryRole::User,
                        marker,
                    }),
                SessionTimelineEventKind::Response if text.starts_with("MALLARD_") => {
                    Some(HistoryMarker {
                        role: HistoryRole::Assistant,
                        marker: validate_exact_marker(&text).unwrap(),
                    })
                }
                _ => None,
            };
            if let Some(marker) = marker {
                history.push(marker);
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(
        history, expected,
        "Session Detail must show exactly retained history"
    );
    header.snapshot_id
}

#[test]
fn revert_scenarios_require_exact_retained_history() {
    let scenarios = scenarios();
    assert_eq!(scenarios.len(), EXPECTED_REQUESTS);
    assert_eq!(
        scenarios[2].expected_history,
        marker_turn(KEEP_USER, KEEP_AGENT)
    );
    assert_eq!(scenarios[3].expected_history, expected_history());
    let document = serde_json::to_string(&scenarios[3]).unwrap();
    assert!(!document.contains(DISCARD_USER) && !document.contains(DISCARD_AGENT));
}

#[test]
#[ignore = "runs native Codex and loopback aimock; set MALLARD_RUN_LIVE_CODEX_AIMOCK_E2E=1"]
fn live_codex_aimock_reverted_rollouts() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }
    assert_supported_codex_version();
    let root = tempfile::tempdir().unwrap();
    let process_home = root.path().join("process-home");
    let control = root.path().join("aimock");
    std::fs::create_dir(&process_home).unwrap();
    create_secure_codex_home(&control);
    let _home = EnvironmentGuard::set("HOME", &process_home);
    let _userprofile = EnvironmentGuard::set("USERPROFILE", &process_home);
    let document = AimockScenarioDocument {
        version: 1,
        model: MODEL.into(),
        expected_request_count: EXPECTED_REQUESTS,
        scenarios: scenarios(),
    };
    let (aimock, provider) = AimockProcess::start(&control, &process_home, &document).unwrap();
    run(&provider);
    aimock
        .finish()
        .expect("revert mock audit must consume every request exactly once");
}
