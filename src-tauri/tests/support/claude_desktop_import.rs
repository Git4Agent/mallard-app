//! Mock-only Desktop import simulation. No Desktop process or real profile is used.
//! The response blocks come from AIMock through the native CLI. Round 6 also
//! inserts explicit mixed and consecutive thinking-only fixture rows.
use super::*;

const MAIN_ROUNDS: usize = 16;
const TOTAL_ROUNDS: usize = 19;
const THINKING: &str = "synthetic private reasoning ";

fn prompt(round: usize) -> String {
    format!("MALLARD_CLAUDE_USER_IMPORT_{round:02}")
}

fn answer(round: usize) -> String {
    format!("MALLARD_CLAUDE_ANSWER_{round:02}")
}

fn thinking(round: usize) -> AimockThinking {
    AimockThinking {
        // Keep the deliberate old-message edit outside the cursor probe.
        reasoning: format!("{THINKING}round-{round:02} ").repeat(300),
        redacted_thinking: vec!["c3ludGhldGljLXJlZGFjdGVkLXRob3VnaHQ=".into()],
    }
}

fn scenario_document(read_path: &Path) -> AimockScenarioDocument {
    let mut scenarios = Vec::new();
    let mut history = Vec::new();
    for round in 1..=TOTAL_ROUNDS {
        if round == 17 {
            history[3] = marker(HistoryRole::Assistant, "MALLARD_CLAUDE_EDITED_02");
        }
        if round == 1 {
            scenarios.push(AimockScenario {
                id: "import_read".into(),
                prompt_marker: prompt(round),
                expected_history: history.clone(),
                require_tool_result: false,
                expected_tool_call_id: None,
                expected_tool_result_marker: None,
                response: AimockResponse::ReadTool {
                    call_id: READ_TOOL_CALL_ID.into(),
                    file_path: read_path.to_string_lossy().into_owned(),
                    thinking: Some(thinking(round)),
                },
            });
        }
        let mut response = text_scenario(
            format!("import_{round:02}"),
            prompt(round),
            history.clone(),
            answer(round),
        );
        response.response = AimockResponse::Text {
            content: answer(round),
            thinking: Some(thinking(round)),
        };
        if round == 1 {
            response.require_tool_result = true;
            response.expected_tool_call_id = Some(READ_TOOL_CALL_ID.into());
            response.expected_tool_result_marker = Some(READ_FILE_MARKER.into());
        }
        scenarios.push(response);
        history.extend(marker_turn(&prompt(round), &answer(round)));
    }
    AimockScenarioDocument {
        version: 1,
        model: MODEL.into(),
        expected_request_count: scenarios.len(),
        expected_probe_request_count: TOTAL_ROUNDS,
        scenarios,
    }
}

fn records(bytes: &[u8]) -> Vec<Value> {
    assert!(bytes.len() as u64 <= MAX_CONVERSATION_FILE_BYTES);
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "fixture transcript is incomplete"
    );
    bytes
        .split(|b| *b == b'\n')
        .filter(|row| !row.is_empty())
        .map(|row| serde_json::from_slice(row).expect("fixture record is JSON"))
        .collect()
}

fn serialize(rows: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, row).expect("serialize fixture row");
        bytes.push(b'\n');
    }
    assert!(bytes.len() as u64 <= MAX_CONVERSATION_FILE_BYTES);
    bytes
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RewriteCounts {
    thinking: usize,
    redacted: usize,
    deleted: usize,
    mixed: usize,
}

// Independent fixture transformation; do not call the production projection.
fn desktop_rewrite(bytes: &[u8]) -> (Vec<u8>, RewriteCounts) {
    let mut counts = RewriteCounts::default();
    let mut retained = Vec::new();
    for mut row in records(bytes) {
        if row["type"] == "assistant" {
            if let Some(content) = row
                .pointer_mut("/message/content")
                .and_then(Value::as_array_mut)
            {
                let before = content.len();
                content.retain(|block| match block["type"].as_str() {
                    Some("thinking") => {
                        counts.thinking += 1;
                        false
                    }
                    Some("redacted_thinking") => {
                        counts.redacted += 1;
                        false
                    }
                    _ => true,
                });
                if before > 0 && content.is_empty() {
                    counts.deleted += 1;
                    continue;
                }
                if before != content.len() {
                    counts.mixed += 1;
                }
            }
        }
        retained.push(row);
    }
    (serialize(&retained), counts)
}

fn assert_fixture_path(root: &Path, path: &Path) {
    let metadata = std::fs::symlink_metadata(path).expect("inspect fixture transcript");
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert!(path
        .canonicalize()
        .unwrap()
        .starts_with(root.canonicalize().unwrap()));
}

fn replace_fixture(root: &Path, path: &Path, bytes: &[u8]) {
    assert_fixture_path(root, path);
    assert!(bytes.len() as u64 <= MAX_CONVERSATION_FILE_BYTES);
    let mut replacement = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
    replacement.write_all(bytes).unwrap();
    replacement.as_file().sync_all().unwrap();
    replacement
        .persist(path)
        .expect("replace fixture transcript atomically");
}

// Explicit fixture construction, distinct from the simulated import. It adds
// no visible message. The final native text row stays at the end of its chain.
fn add_row_shapes(root: &Path, path: &Path) {
    let mut rows = records(&read_private_transcript(path, "row-shape fixture"));
    let index = rows
        .iter()
        .rposition(|row| {
            row["type"] == "assistant"
                && row
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "text"))
        })
        .expect("native transcript has a text row");
    let mut text_row = rows.remove(index);
    let mut parent = text_row["parentUuid"].clone();
    let mut added = Vec::new();
    for index in 0..12 {
        let mut row = text_row.clone();
        row["uuid"] = Value::String(format!("018f3f2e-7b91-4ad0-8000-0000000004{index:02}"));
        row["parentUuid"] = parent;
        row["message"]["content"] = serde_json::json!([
            {"type":"thinking", "thinking":"fixture-only thought", "signature":"mock"}
        ]);
        parent = row["uuid"].clone();
        added.push(row);
    }
    text_row["parentUuid"] = parent;
    text_row["message"]["content"].as_array_mut().unwrap().insert(0,
        serde_json::json!({"type":"thinking", "thinking":"fixture mixed thought", "signature":"mock"}));
    added.push(text_row);
    rows.splice(index..index, added);
    replace_fixture(root, path, &serialize(&rows));
}

fn assert_native_thinking(bytes: &[u8]) {
    let rows = records(bytes);
    let blocks = rows
        .iter()
        .filter_map(|row| row.pointer("/message/content").and_then(Value::as_array))
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        blocks.iter().any(|block| block["type"] == "thinking"
            && block["thinking"]
                .as_str()
                .is_some_and(|text| text.contains(THINKING))),
        "CLI did not persist the mock thinking block"
    );
    assert!(
        blocks
            .iter()
            .any(|block| block["type"] == "redacted_thinking"),
        "CLI did not persist the mock redacted thinking block"
    );
}

fn assert_immutable(root: &Path, before: &BTreeMap<PathBuf, [u8; 32]>) {
    let after = tree_digest_inventory(root);
    for (key, digest) in before {
        assert!(
            after.get(key) == Some(digest),
            "an existing storage object changed or disappeared"
        );
    }
}

fn assert_state(
    service: &ProjectSyncService<'_>,
    project: &LocalProjectId,
    storage: &StorageId,
    thread: &ThreadId,
    expected: ThreadSyncState,
    stage: &str,
) {
    let comparison = service
        .compare_threads(project, storage)
        .unwrap_or_else(|_| panic!("{stage}: comparison failed"));
    let entry = comparison
        .entries
        .iter()
        .find(|row| {
            row.conversation_resource_id
                == mallard_domain::conversation_resource_id(&claude_key(), thread).unwrap()
        })
        .unwrap_or_else(|| panic!("{stage}: session missing"));
    assert_eq!(entry.state, expected, "{stage}");
}

fn push_success(service: &ProjectSyncService<'_>, project: &LocalProjectId, storage: &StorageId) {
    let result = push(service, project, storage);
    assert!(result.success, "reviewed import Push failed");
    assert!(
        result
            .results
            .iter()
            .all(|row| row.state != "failed" && row.state != "held"),
        "import Push failed or held a resource"
    );
}

#[test]
fn rewrite_drops_thinking_only_rows_and_keeps_dangling_parents() {
    let rows = vec![
        serde_json::json!({"type":"assistant","uuid":"a","parentUuid":"u","message":{"content":[{"type":"thinking","thinking":"private"}]}}),
        serde_json::json!({"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"redacted_thinking","data":"opaque"}]}}),
        serde_json::json!({"type":"assistant","uuid":"c","parentUuid":"b","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"visible"},{"type":"tool_use","id":"read","name":"Read","input":{}}]}}),
        serde_json::json!({"type":"user","uuid":"d","parentUuid":"c","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"result"}]}}),
    ];
    let (after, count) = desktop_rewrite(&serialize(&rows));
    assert_eq!(
        count,
        RewriteCounts {
            thinking: 2,
            redacted: 1,
            deleted: 2,
            mixed: 1
        }
    );
    let mut expected = rows[2..].to_vec();
    expected[0]["message"]["content"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(records(&after), expected);
    let (again, count) = desktop_rewrite(&after);
    assert_eq!(again, after);
    assert_eq!(count, RewriteCounts::default());
}

#[test]
fn replacement_is_atomic_and_confined_to_the_fixture() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transcript.jsonl");
    write_private_new(&path, b"{}\n");
    let opened = std::fs::File::open(&path).unwrap();
    replace_fixture(root.path(), &path, b"{\"new\":true}\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\":true}\n");
    let mut old = Vec::new();
    opened.take(32).read_to_end(&mut old).unwrap();
    assert_eq!(old, b"{}\n");
    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(std::panic::catch_unwind(|| assert_fixture_path(root.path(), outside.path())).is_err());
}

#[test]
#[ignore = "19 native CLI rounds using only local AIMock; select --scenario desktop-import"]
fn live_claude_aimock_repeated_desktop_import() {
    if std::env::var(LIVE_TEST_GATE).ok().as_deref() != Some("1") {
        return;
    }
    let started = Instant::now();
    let root = tempfile::tempdir().unwrap();
    let process_home = root.path().join("process");
    let homes = [root.path().join("claude-a"), root.path().join("claude-b")];
    let projects = [root.path().join("project-a"), root.path().join("project-b")];
    let machines = [root.path().join("machine-a"), root.path().join("machine-b")];
    let storage_root = root.path().join("storage");
    let control = root.path().join("control");
    let codex_home = root.path().join("unused-codex");
    for path in [
        &process_home,
        &homes[0],
        &homes[1],
        &projects[0],
        &projects[1],
        &machines[0],
        &machines[1],
        &storage_root,
        &control,
        &codex_home,
    ] {
        secure_directory(path);
    }
    let projects = projects.map(|path| path.canonicalize().unwrap());
    let _home = EnvironmentGuard::set("HOME", &process_home);
    let _user = EnvironmentGuard::set("USERPROFILE", &process_home);
    let _codex = EnvironmentGuard::set("CODEX_HOME", &codex_home);
    let _claude = EnvironmentGuard::set("CLAUDE_CONFIG_DIR", &homes[0]);
    assert_supported_claude_version(&process_home, EXPECTED_AIMOCK_CLAUDE_VERSION);
    let read_path = projects[0].join("read-marker.txt");
    write_private_new(&read_path, READ_FILE_MARKER.as_bytes());
    let document = scenario_document(&read_path);
    assert_eq!(document.expected_request_count, TOTAL_ROUNDS + 1);
    let (aimock, provider) = AimockProcess::start(&control, &process_home, &document).unwrap();
    let repositories = machines
        .iter()
        .map(|path| LocalStateRepository::from_home_dir(path).unwrap())
        .collect::<Vec<_>>();
    for repository in &repositories {
        mallard_infrastructure::app_settings::save_app_settings(
            repository,
            &mallard_infrastructure::app_settings::AppSettings {
                credential_redaction_enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
    }
    let storage = local_storage(&storage_root);
    let storage_id = storage.id.clone();
    let first = finish_setup(
        &repositories[0],
        &ProjectSyncService::new(&repositories[0]),
        &projects[0],
        &homes[0],
        storage.clone(),
        DraftRepositoryChoice::New,
    );
    let bundle = first.project.bundle_id.clone();
    let mut details = vec![first];
    let mut history = Vec::new();
    let mut process_count = 0;
    let mut shortened_from = None;
    let mut primary_cloud = None;
    let mut immutable = tree_digest_inventory(&storage_root);
    for round in 1..=MAIN_ROUNDS {
        let replica = if (9..=12).contains(&round) { 1 } else { 0 };
        let repository = &repositories[replica];
        let service = ProjectSyncService::new(repository);
        let detail = &details[replica];
        let project = &detail.project.local_project_id;
        let session = if round == 1 {
            SOURCE_SESSION_ID.to_string()
        } else {
            let thread =
                local_thread_of_cloud(repository, project, primary_cloud.as_ref().unwrap());
            provider_thread_id(repository, project, &thread)
        };
        let stage = Instant::now();
        run_claude(
            &mut process_count,
            &provider,
            &homes[replica],
            &process_home,
            &projects[replica],
            &session,
            round == 1,
            &prompt(round),
            &answer(round),
        );
        history.extend(marker_turn(&prompt(round), &answer(round)));
        let path = transcript_path(&homes[replica], &projects[replica], &session);
        let native = read_private_transcript(&path, "native thinking transcript");
        assert_native_thinking(&native);
        if round == 6 {
            add_row_shapes(root.path(), &path);
        }
        if round == 7 {
            assert!(
                records(&native).len() < shortened_from.unwrap(),
                "continuation concealed the record-count decrease"
            );
        }
        assert_transcript_structure_and_history(
            &homes[replica],
            &projects[replica],
            &session,
            &history,
            history.len() + 2,
            &read_path,
        );
        service
            .compare_threads(project, &storage_id)
            .expect("discover native session");
        if round > 1 {
            let thread = logical_thread_id(repository, project, &session);
            assert_state(
                &service,
                project,
                &storage_id,
                &thread,
                ThreadSyncState::LocalAhead,
                &format!("round {round} before Push"),
            );
        }
        let source = read_private_transcript(&path, "pre-Push transcript");
        push_success(&service, project, &storage_id);
        let thread = logical_thread_id(repository, project, &session);
        assert_eq!(
            read_private_transcript(&path, "post-Push transcript"),
            source,
            "capture changed the native transcript"
        );
        assert_storage_contains(
            &storage_root,
            format!("{THINKING}round-{round:02} ").as_bytes(),
        );
        assert_immutable(&storage_root, &immutable);
        immutable = tree_digest_inventory(&storage_root);
        if round == 1 {
            primary_cloud = Some(cloud_thread_id_of(repository, project, &thread));
        }
        assert_state(
            &service,
            project,
            &storage_id,
            &thread,
            ThreadSyncState::Synced,
            &format!("round {round} after Push"),
        );
        if [3, 6, 10, 14].contains(&round) {
            let (rewritten, counts) = desktop_rewrite(&source);
            assert!(
                counts.thinking > 0 && counts.redacted > 0 && counts.deleted > 0,
                "import did not remove native thinking and whole rows: {counts:?}"
            );
            assert_ne!(rewritten, source);
            if round == 6 {
                assert!(counts.mixed > 0 && counts.deleted >= 12);
                shortened_from = Some(records(&source).len());
            }
            replace_fixture(root.path(), &path, &rewritten);
            let installed_end = conversation_binding(repository, project, &thread).installed_end;
            let cutoff_valid = installed_end.is_none_or(|end| {
                end > 0 && end <= rewritten.len() as u64 && rewritten[end as usize - 1] == b'\n'
            });
            eprintln!("desktop-import round={round} removed_rows={} removed_blocks={} installed_cutoff_valid={cutoff_valid}",
                counts.deleted, counts.thinking + counts.redacted);

            assert_transcript_structure_and_history(
                &homes[replica],
                &projects[replica],
                &session,
                &history,
                history.len() + 2,
                &read_path,
            );
            assert_state(
                &service,
                project,
                &storage_id,
                &thread,
                ThreadSyncState::Synced,
                &format!("round {round} after Desktop import on replica {replica}"),
            );
            if round == 6 {
                let (again, counts) = desktop_rewrite(&rewritten);
                assert_eq!(counts, RewriteCounts::default());
                assert_eq!(again, rewritten);
                replace_fixture(root.path(), &path, &again);
            }
            // Reopen the service without clearing persisted observations or checkpoints.
            assert_state(
                &ProjectSyncService::new(repository),
                project,
                &storage_id,
                &thread,
                ThreadSyncState::Synced,
                "reopened after import",
            );
            assert_immutable(&storage_root, &immutable);
        }
        eprintln!(
            "desktop-import round={round} replica={replica} elapsed_ms={} bytes={} requests={}",
            stage.elapsed().as_millis(),
            source.len(),
            round + 1
        );
        if [8, 12, 16].contains(&round) {
            let target = 1 - replica;
            let target_service = ProjectSyncService::new(&repositories[target]);
            if details.len() == 1 {
                let remote = finish_setup(
                    &repositories[target],
                    &target_service,
                    &projects[target],
                    &homes[target],
                    storage.clone(),
                    DraftRepositoryChoice::Existing {
                        storage_id: storage_id.clone(),
                        bundle_id: bundle.clone(),
                        display_name: details[0].project.display_name.clone(),
                        repository_fingerprint: details[0].project.repository_fingerprint.clone(),
                        mismatch_acknowledged: true,
                    },
                );
                details.push(remote);
            }
            let target_detail = &details[target];
            let (_, result) = pull_and_apply(
                &target_service,
                &storage_id,
                &bundle,
                target_detail.binding.as_ref().unwrap(),
            );
            assert!(result.preserved_threads.is_empty());
            let target_project = &target_detail.project.local_project_id;
            let target_thread = local_thread_of_cloud(
                &repositories[target],
                target_project,
                primary_cloud.as_ref().unwrap(),
            );
            let target_session =
                provider_thread_id(&repositories[target], target_project, &target_thread);
            assert_transcript_structure_and_history(
                &homes[target],
                &projects[target],
                &target_session,
                &history,
                history.len() + 2,
                &read_path,
            );
            assert_state(
                &target_service,
                target_project,
                &storage_id,
                &target_thread,
                ThreadSyncState::Synced,
                "after handoff Pull",
            );
        }
    }

    // A true visible edit must still be refused; Keep both can publish that
    // edited copy only under a new cloud identity.
    let repository = &repositories[0];
    let service = ProjectSyncService::new(repository);
    let project = &details[0].project.local_project_id;
    let cloud = primary_cloud.unwrap();
    let thread = local_thread_of_cloud(repository, project, &cloud);
    let session = provider_thread_id(repository, project, &thread);
    let path = transcript_path(&homes[0], &projects[0], &session);
    let source = read_private_transcript(&path, "pre-edit transcript");
    let old = answer(2);
    let edited = "MALLARD_CLAUDE_EDITED_02";
    assert_eq!(old.len(), edited.len());
    let mut bytes = source.clone();
    let offsets = source
        .windows(old.len())
        .enumerate()
        .filter_map(|(i, text)| (text == old.as_bytes()).then_some(i))
        .collect::<Vec<_>>();
    assert_eq!(offsets.len(), 1);
    assert!(
        source.len() - offsets[0]
            > mallard_infrastructure::jsonl_tail_io::PROBE_WINDOW_BYTES as usize
    );
    bytes[offsets[0]..offsets[0] + old.len()].copy_from_slice(edited.as_bytes());
    assert_fixture_path(root.path(), &path);
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    history[3] = marker(HistoryRole::Assistant, edited);
    run_claude(
        &mut process_count,
        &provider,
        &homes[0],
        &process_home,
        &projects[0],
        &session,
        false,
        &prompt(17),
        &answer(17),
    );
    history.extend(marker_turn(&prompt(17), &answer(17)));
    let before_refusal = tree_digest_inventory(&storage_root);
    let refusal = push(&service, project, &storage_id);
    assert!(!refusal.success);
    let failed = refusal
        .results
        .iter()
        .filter(|row| row.state == "failed")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].message.as_deref(),
        Some(mallard_infrastructure::owned_subagents::PUSH_EDITED)
    );
    assert_eq!(tree_digest_inventory(&storage_root), before_refusal);
    assert_state(
        &service,
        project,
        &storage_id,
        &thread,
        ThreadSyncState::Diverged,
        "visible edit refused",
    );
    let plan = service
        .plan_bundle_restore(&storage_id, &bundle, details[0].binding.as_ref().unwrap())
        .unwrap();
    assert_keep_both_offered(&plan, &thread);
    let result = service
        .apply_bundle_restore(&plan, approved_pull_actions(&plan))
        .unwrap();
    assert!(result.success);
    let kept = assert_keep_both_result(repository, project, &thread, &cloud, &session, &result);
    push_success(&service, project, &storage_id);
    let kept_cloud = cloud_thread_id_of(repository, project, &kept.preserved_thread_id);
    assert_ne!(kept_cloud, cloud);
    for round in 18..=19 {
        run_claude(
            &mut process_count,
            &provider,
            &homes[0],
            &process_home,
            &projects[0],
            &session,
            false,
            &prompt(round),
            &answer(round),
        );
        history.extend(marker_turn(&prompt(round), &answer(round)));
        push_success(&service, project, &storage_id);
        assert_state(
            &service,
            project,
            &storage_id,
            &kept.preserved_thread_id,
            ThreadSyncState::Synced,
            "continued kept copy",
        );
    }
    let service_b = ProjectSyncService::new(&repositories[1]);
    pull_and_apply(
        &service_b,
        &storage_id,
        &bundle,
        details[1].binding.as_ref().unwrap(),
    );
    let original = (1..=MAIN_ROUNDS)
        .flat_map(|round| marker_turn(&prompt(round), &answer(round)))
        .collect::<Vec<_>>();
    for replica in 0..2 {
        for (id, expected) in [(&cloud, &original), (&kept_cloud, &history)] {
            let local_project = &details[replica].project.local_project_id;
            let local_thread = local_thread_of_cloud(&repositories[replica], local_project, id);
            let local_session =
                provider_thread_id(&repositories[replica], local_project, &local_thread);
            assert_transcript_structure_and_history(
                &homes[replica],
                &projects[replica],
                &local_session,
                expected,
                expected.len() + 2,
                &read_path,
            );
            assert_state(
                &ProjectSyncService::new(&repositories[replica]),
                local_project,
                &storage_id,
                &local_thread,
                ThreadSyncState::Synced,
                "final histories",
            );
        }
    }
    assert_immutable(&storage_root, &immutable);
    assert_eq!(process_count, TOTAL_ROUNDS);
    aimock
        .finish(&[PLACEHOLDER_API_KEY.as_bytes().to_vec()])
        .unwrap();
    eprintln!(
        "desktop-import complete rounds={TOTAL_ROUNDS} imports=4 model_requests={} elapsed_ms={}",
        document.expected_request_count,
        started.elapsed().as_millis()
    );
}
