// Synthetic Claude direct-child flow through the production service.
fn write_claude_owned_fixture(
    home: &Path,
    root: &Path,
    parent: &str,
    count: usize,
) -> (PathBuf, Vec<PathBuf>) {
    let main = home.join("projects/source").join(format!("{parent}.jsonl"));
    let mut bytes = claude_test_transcript(parent, root, &[("u1", None, "user", "launch agents")]);
    let mut paths = Vec::new();
    let mut previous = "u1".to_string();
    for n in 0..count {
        let agent = format!("agent{n}");
        let uuid = format!("result{n}");
        let record = serde_json::json!({"type":"user","uuid":uuid,"parentUuid":previous,
            "sessionId":parent,"cwd":root,"isSidechain":false,
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":format!("call{n}"),"content":"done"}]},
            "toolUseResult":{"agentId":agent,"isAsync":true,"outputFile":"/unavailable/fixture.output"}});
        bytes.extend_from_slice(format!("{record}\n").as_bytes());
        previous = uuid;
        let path = claude_subagents::child_path(&main, parent, &agent).unwrap();
        let raw = claude_test_transcript(
            parent,
            root,
            &[
                ("cu1", None, "user", "child task"),
                ("ca1", Some("cu1"), "assistant", "child answer"),
            ],
        );
        let child = raw
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut record: serde_json::Value = serde_json::from_slice(line).unwrap();
                record["agentId"] = serde_json::json!(agent);
                record["isSidechain"] = serde_json::json!(true);
                format!("{record}\n")
            })
            .collect::<String>();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, child).unwrap();
        paths.push(path);
    }
    std::fs::write(&main, bytes).unwrap();
    (main, paths)
}

fn append_claude_owned_child(path: &Path, parent: &str, agent: &str, uuid: &str, previous: &str) {
    use std::io::Write;
    let record = serde_json::json!({"type":"user","uuid":uuid,"parentUuid":previous,
        "sessionId":parent,"agentId":agent,"isSidechain":true,
        "message":{"role":"user","content":"continued child"}});
    writeln!(
        std::fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{record}"
    )
    .unwrap();
}

#[test]
fn claude_owned_subagents_round_trip_update_and_missing_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("claude-owned-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let source = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("source-claude");
    bind_and_link_claude_replica(&repo, &source, &storage, &root, &home);
    let parent = "11111111-1111-4111-8111-111111111111";
    let (_, children) = write_claude_owned_fixture(&home, &root, parent, 2);
    let service = ProjectSyncService::new(&repo);
    let push = |project: &LocalProjectRegistration| {
        let review = service
            .prepare_push_review(&project.local_project_id, &storage)
            .unwrap();
        let result = service
            .push_reviewed(
                &project.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage),
            )
            .unwrap();
        assert!(result.success, "{} {:?}", result.message, result.results);
    };
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    let entries = &review.thread_comparison.as_ref().unwrap().entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].subagent_selection.keys.len(), 2, "{entries:?}");
    push(&source);
    let target = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "claude-owned-target".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    let target_root = temp.path().join("target");
    let target_home = temp.path().join("target-claude");
    bind_and_link_claude_replica(&repo, &target, &storage, &target_root, &target_home);
    let pull = || {
        let bindings = repo.load_bindings().unwrap();
        let binding = bindings.active_for(&target.local_project_id).unwrap();
        service
            .plan_bundle_restore(&storage, &source.bundle_id, binding)
            .unwrap()
    };
    let apply = |plan: &RestorePlan| {
        let result = service
            .apply_bundle_restore(
                plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(
            result.success,
            "{} {:?}",
            result.message, result.failed_actions
        );
    };
    apply(&pull());
    let bindings = repo.load_conversation_bindings().unwrap();
    let installed = bindings
        .bindings
        .iter()
        .find(|row| row.local_project_id == target.local_project_id)
        .unwrap()
        .clone();
    let dest = installed.provider_thread_id.as_ref().unwrap();
    assert_ne!(dest, parent);
    assert_eq!(installed.installed_subagents.len(), 2);
    assert!(installed.installed_end.is_some());
    let mut files = vec![];
    jsonl_files_under(&target_home, &mut files);
    let main = files
        .iter()
        .find(|path| path.file_name().unwrap() == format!("{dest}.jsonl").as_str())
        .unwrap();
    let child = claude_subagents::child_path(main, dest, "agent0").unwrap();
    assert!(child.exists());
    let compare = || {
        get_project_thread_sync_comparison_with_repository(
            &repo,
            &target.local_project_id,
            &storage,
        )
        .unwrap()
    };
    assert_eq!(
        compare().entries[0].state,
        ThreadSyncState::Synced,
        "{:?}",
        compare().entries
    );
    append_claude_owned_child(&children[0], parent, "agent0", "cu2", "ca1");
    push(&source);
    let plan = pull();
    assert_eq!(
        plan.conversation_review[0].action,
        ConversationReviewAction::MaterializeCloud,
        "{:?}",
        plan.conversation_review
    );
    apply(&plan);
    let current = repo.load_conversation_bindings().unwrap();
    assert_eq!(
        current
            .bindings
            .iter()
            .find(|row| row.thread_id == installed.thread_id)
            .unwrap()
            .provider_thread_id
            .as_ref(),
        Some(dest)
    );
    assert_eq!(compare().entries[0].state, ThreadSyncState::Synced);
    let metrics = mallard_infrastructure::sync_metrics::register_root(
        &std::fs::canonicalize(&target_home).unwrap(),
    );
    compare();
    let before = metrics.snapshot();
    compare();
    let quiet = metrics.snapshot().delta_since(&before);
    assert_eq!(quiet.transcript_bytes_read, 0);
    assert_eq!(quiet.append_bytes_read, 0);
    let main_metrics =
        mallard_infrastructure::sync_metrics::register_root(&std::fs::canonicalize(main).unwrap());
    let before_cold = main_metrics.snapshot();
    super::super::session_watch_service::session_watch_service().force_rebaseline(&target_home);
    assert_eq!(compare().entries[0].state, ThreadSyncState::Synced);
    let cold = main_metrics.snapshot().delta_since(&before_cold);
    assert!(cold.transcript_bytes_read > 0);
    assert_eq!(
        cold.full_graph_resolves, 0,
        "cold membership uses the existing main cursor"
    );
    let sibling = claude_subagents::child_path(main, dest, "agent1").unwrap();
    let sibling_metrics = mallard_infrastructure::sync_metrics::register_root(&sibling);
    let before = sibling_metrics.snapshot();
    append_claude_owned_child(&child, dest, "agent0", "local-u3", "cu2");
    assert_eq!(compare().entries[0].state, ThreadSyncState::LocalAhead);
    let unchanged = sibling_metrics.snapshot().delta_since(&before);
    assert_eq!(unchanged.transcript_bytes_read, 0);
    assert_eq!(unchanged.append_bytes_read, 0);
    let saved = std::fs::read(&child).unwrap();
    std::fs::remove_file(&child).unwrap();
    let plan = pull();
    assert_eq!(
        plan.conversation_review[0].action,
        ConversationReviewAction::KeepBoth,
        "{:?}",
        plan.conversation_review
    );
    apply(&plan);
    assert!(!child.exists());
    assert!(main.exists());
    let current = repo.load_conversation_bindings().unwrap();
    let restored = current
        .bindings
        .iter()
        .find(|row| row.thread_id == installed.thread_id)
        .unwrap();
    assert_ne!(restored.provider_thread_id.as_ref(), Some(dest));
    assert_eq!(restored.installed_subagents.len(), 2);
    assert!(compare()
        .entries
        .iter()
        .any(|entry| entry.state == ThreadSyncState::Synced));
    std::fs::write(child, saved).unwrap();
    let snapshot = service.inspect_bundle(&storage, &source.bundle_id).unwrap();
    let store = LocalObjectStore::open(&temp.path().join("store")).unwrap();
    let objects = snapshot
        .manifest
        .all_files()
        .map(|(_, file)| {
            (
                repository_object_key(&source.bundle_id, &file.object_key).unwrap(),
                file.size,
            )
        })
        .collect::<Vec<_>>();
    let mut native_files = Vec::new();
    jsonl_files_under(&target_home, &mut native_files);
    let retained = native_files
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect::<Vec<_>>();
    service
        .clear_storage_data(&storage, repo.load_config().unwrap().revision)
        .unwrap();
    for (key, size) in objects {
        assert!(store.get_metadata(&key, size).unwrap().is_none());
    }
    for (path, bytes) in retained {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    assert_eq!(repo.load_conversation_bindings().unwrap(), current);
}

#[test]
#[ignore = "Run with scripts/run-claude-owned-sync-native.mjs using isolated native homes"]
fn native_claude_owned_transport_bridge() {
    let root = std::fs::canonicalize(std::env::var("MALLARD_CLAUDE_NATIVE_ROOT").unwrap()).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-claude-owned-native-"));
    let repo = LocalStateRepository::from_home_dir(root.join("mallard-state")).unwrap();
    let storage = StorageId::parse("native-claude-owned").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let source = register(&repo);
    bind_and_link_claude_replica(
        &repo,
        &source,
        &storage,
        &root.join("source/project"),
        &root.join("source/config"),
    );
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    let entries = &review.thread_comparison.as_ref().unwrap().entries;
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert!(
        entries
            .iter()
            .all(|entry| entry.subagent_selection.keys.len() == 3),
        "{entries:?}"
    );
    let pushed = service
        .push_reviewed(
            &source.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(pushed.success, "{} {:?}", pushed.message, pushed.results);
    let target = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "native-claude-target".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    bind_and_link_claude_replica(
        &repo,
        &target,
        &storage,
        &root.join("destination/project"),
        &root.join("destination/config"),
    );
    let bindings = repo.load_bindings().unwrap();
    let binding = bindings.active_for(&target.local_project_id).unwrap();
    let plan = service
        .plan_bundle_restore(&storage, &source.bundle_id, binding)
        .unwrap();
    let applied = service
        .apply_bundle_restore(
            &plan,
            plan.conversation_review
                .iter()
                .filter_map(|row| row.action_id.clone())
                .collect(),
        )
        .unwrap();
    assert!(
        applied.success,
        "{} {:?}",
        applied.message, applied.failed_actions
    );
    let bindings = repo.load_conversation_bindings().unwrap();
    let mut files = vec![];
    jsonl_files_under(&root.join("destination/config"), &mut files);
    let parents = bindings.bindings.iter().filter(|row| row.local_project_id == target.local_project_id).map(|installed| {
        let id=installed.provider_thread_id.as_ref().unwrap();
        let main=files.iter().find(|path|path.file_name().unwrap()==format!("{id}.jsonl").as_str()).unwrap();
        let children=installed.installed_subagents.values().map(|child| serde_json::json!({
            "id":child.provider_thread_id,"path":claude_subagents::child_path(main,id,&child.provider_thread_id).unwrap()
        })).collect::<Vec<_>>();
        serde_json::json!({"main_id":id,"children":children})
    }).collect::<Vec<_>>();
    std::fs::write(
        root.join("transport.json"),
        serde_json::to_vec(&serde_json::json!({"parents":parents})).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "Release performance gate; run alone with --release --ignored --nocapture"]
fn benchmark_claude_owned_subagents() {
    use std::io::Write;
    for count in [0, 1, 8, 16] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("claude-perf").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let source = register(&repo);
        let home = temp.path().join("source-claude");
        let root = temp.path().join("source");
        bind_and_link_claude_replica(&repo, &source, &storage, &root, &home);
        let parent = "11111111-1111-4111-8111-111111111111";
        let (main, children) = write_claude_owned_fixture(&home, &root, parent, count);
        for (index, path) in std::iter::once(&main).chain(children.iter()).enumerate() {
            let mut previous = if index == 0 {
                if count == 0 {
                    "u1".to_string()
                } else {
                    format!("result{}", count - 1)
                }
            } else {
                "ca1".to_string()
            };
            let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            for n in 0..128 {
                let uuid = format!("large{n}");
                let mut record = serde_json::json!({"type":"user","uuid":uuid,"parentUuid":previous,"sessionId":parent,"cwd":root,
                    "isSidechain":index>0,"message":{"role":"user","content":"x".repeat(2048)}});
                if index > 0 {
                    record["agentId"] = serde_json::json!(format!("agent{}", index - 1));
                }
                writeln!(file, "{record}").unwrap();
                previous = uuid;
            }
        }
        let service = ProjectSyncService::new(&repo);
        let push = |review: &PushReviewContext| {
            let result = service
                .push_reviewed(
                    &source.local_project_id,
                    &storage,
                    PushApproval::recommended(review, &storage),
                )
                .unwrap();
            assert!(result.success, "{} {:?}", result.message, result.results);
        };
        let review = service
            .prepare_push_review(&source.local_project_id, &storage)
            .unwrap();
        push(&review);
        let mut timings = BTreeMap::<&str, Vec<f64>>::new();
        for cold in [true, false] {
            for _ in 0..15 {
                if cold {
                    super::super::session_watch_service::session_watch_service()
                        .force_rebaseline(&home);
                }
                let start = Instant::now();
                let comparison = get_project_thread_sync_comparison_with_repository(
                    &repo,
                    &source.local_project_id,
                    &storage,
                )
                .unwrap();
                assert_eq!(comparison.entries[0].state, ThreadSyncState::Synced);
                timings
                    .entry(if cold { "cold_refresh" } else { "warm_refresh" })
                    .or_default()
                    .push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        for n in 0..7 {
            let path = children.last().unwrap_or(&main);
            let uuid = format!("append{n}");
            let previous = if n == 0 {
                "large127".to_string()
            } else {
                format!("append{}", n - 1)
            };
            if count > 0 {
                append_claude_owned_child(
                    path,
                    parent,
                    &format!("agent{}", count - 1),
                    &uuid,
                    &previous,
                );
            } else {
                let record = serde_json::json!({"type":"user","uuid":uuid,"parentUuid":previous,"sessionId":parent,"cwd":root,"message":{"role":"user","content":"append"}});
                writeln!(
                    std::fs::OpenOptions::new().append(true).open(path).unwrap(),
                    "{record}"
                )
                .unwrap();
            }
            let start = Instant::now();
            let review = service
                .prepare_push_review(&source.local_project_id, &storage)
                .unwrap();
            timings
                .entry("append_review")
                .or_default()
                .push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            push(&review);
            timings
                .entry("push_apply")
                .or_default()
                .push(start.elapsed().as_secs_f64() * 1000.0);
            let target = register_local_project_with_repository(
                &repo,
                RegisterLocalProjectRequest {
                    display_name: format!("perf{n}"),
                    repository_fingerprint: source.repository_fingerprint.clone(),
                    bundle_id: Some(source.bundle_id.clone()),
                },
            )
            .unwrap();
            bind_and_link_claude_replica(
                &repo,
                &target,
                &storage,
                &temp.path().join(format!("target{n}")),
                &temp.path().join(format!("claude{n}")),
            );
            let bindings = repo.load_bindings().unwrap();
            let binding = bindings.active_for(&target.local_project_id).unwrap();
            let plan = service
                .plan_bundle_restore(&storage, &source.bundle_id, binding)
                .unwrap();
            let start = Instant::now();
            let result = service
                .apply_bundle_restore(
                    &plan,
                    plan.conversation_review
                        .iter()
                        .filter_map(|row| row.action_id.clone())
                        .collect(),
                )
                .unwrap();
            assert!(result.success, "{:?}", result.failed_actions);
            timings
                .entry("pull_apply")
                .or_default()
                .push(start.elapsed().as_secs_f64() * 1000.0);
        }
        for (stage, mut times) in timings {
            times.sort_by(f64::total_cmp);
            println!(
                "claude-owned-benchmark {}",
                serde_json::json!({"children":count,"stage":stage,"samples":times.len(),"median_ms":times[times.len()/2],"p95_ms":times[(times.len()*95/100).min(times.len()-1)]})
            );
        }
    }
}

#[test]
fn claude_owned_omission_and_review_membership_change() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("claude-omission").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("project");
    let home = temp.path().join("claude");
    bind_and_link_claude_replica(&repo, &project, &storage, &root, &home);
    let parent = "11111111-1111-4111-8111-111111111111";
    let (_, children) = write_claude_owned_fixture(&home, &root, parent, 1);
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    write_claude_owned_fixture(&home, &root, parent, 2);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(!result.success);
    assert!(result
        .results
        .iter()
        .any(|row| row.message.as_deref() == Some(owned_subagents::PUSH_CHANGED)));
    let mut records = std::fs::read_to_string(&children[0])
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    records[0]["toolUseResult"] = serde_json::json!({"agentId":"nested"});
    std::fs::write(
        &children[0],
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    let entry = &review.thread_comparison.as_ref().unwrap().entries[0];
    assert!(entry.subagent_selection.omitted);
    assert!(entry.subagent_selection.keys.is_empty());
    // Both settings omit this unsupported child. The stamp itself must still be pinned.
    save_child_push(&repo, "claude", false);
    let stale = service.push_reviewed(&project.local_project_id, &storage,
        PushApproval::recommended(&review, &storage)).unwrap();
    assert!(!stale.success);
    assert!(stale.results.iter().any(|row| row.message.as_deref() == Some(owned_subagents::PUSH_CHANGED)));
    save_child_push(&repo, "claude", true);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success, "{} {:?}", result.message, result.results);
    let (_, engine) = storage_engine(&repo, &storage).unwrap();
    let snapshot = engine.inspect(&project.bundle_id).unwrap();
    assert!(snapshot
        .manifest
        .all_conversations()
        .all(|(_, _, thread)| thread
            .files
            .iter()
            .all(|file| !matches!(file.role, ConversationFileRole::SubagentTranscript { .. }))));
}

#[test]
fn claude_owned_equal_agent_ids_are_parent_scoped_and_first_contact_proves_child_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("claude-prefix").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let source = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("claude");
    bind_and_link_claude_replica(&repo, &source, &storage, &root, &home);
    let parents = [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    ];
    let (_, children) = write_claude_owned_fixture(&home, &root, parents[0], 1);
    write_claude_owned_fixture(&home, &root, parents[1], 1);
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    assert!(
        service
            .push_reviewed(
                &source.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage)
            )
            .unwrap()
            .success
    );
    append_claude_owned_child(&children[0], parents[0], "agent0", "u2", "ca1");
    repo.mutate_config(|config| {
        for link in &mut config.links {
            for checkpoints in link.conversation_checkpoints.values_mut() {
                for checkpoint in checkpoints.values_mut() {
                    checkpoint.accepted = None;
                    checkpoint.accepted_local_tail = None;
                    checkpoint.accepted_subagents.clear();
                }
            }
        }
        Ok(())
    })
    .unwrap();
    let comparison = get_project_thread_sync_comparison_with_repository(
        &repo,
        &source.local_project_id,
        &storage,
    )
    .unwrap();
    assert!(
        comparison
            .entries
            .iter()
            .any(|entry| entry.state == ThreadSyncState::LocalAhead),
        "{:?}",
        comparison.entries
    );
    let target = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "same-agent-target".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    bind_and_link_claude_replica(
        &repo,
        &target,
        &storage,
        &temp.path().join("target"),
        &temp.path().join("target-claude"),
    );
    let bindings = repo.load_bindings().unwrap();
    let plan = service
        .plan_bundle_restore(
            &storage,
            &source.bundle_id,
            bindings.active_for(&target.local_project_id).unwrap(),
        )
        .unwrap();
    let result = service
        .apply_bundle_restore(
            &plan,
            plan.conversation_review
                .iter()
                .filter_map(|row| row.action_id.clone())
                .collect(),
        )
        .unwrap();
    assert!(
        result.success,
        "{} {:?}",
        result.message, result.failed_actions
    );
    let bindings = repo.load_conversation_bindings().unwrap();
    let rows = bindings
        .bindings
        .iter()
        .filter(|row| row.local_project_id == target.local_project_id)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].provider_thread_id, rows[1].provider_thread_id);
    for row in rows {
        assert_eq!(
            row.installed_subagents
                .values()
                .next()
                .unwrap()
                .provider_thread_id,
            "agent0"
        );
    }
}

#[test]
#[ignore = "Seed an isolated native Mallard UI fixture under a caller-owned temporary directory"]
fn prepare_claude_owned_ui_fixture() {
    let root = std::fs::canonicalize(std::env::var("MALLARD_CLAUDE_UI_ROOT").unwrap()).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-claude-ui-"));
    let repo = LocalStateRepository::from_home_dir(root.join("home")).unwrap();
    let storage = StorageId::parse("claude-qa-storage").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let project = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "Claude child recovery QA".into(),
            repository_fingerprint: Some("a".repeat(64)),
            bundle_id: None,
        },
    )
    .unwrap();
    let home = root.join("claude");
    let project_root = root.join("project");
    bind_and_link_claude_replica(&repo, &project, &storage, &project_root, &home);
    let parent = "11111111-1111-4111-8111-111111111111";
    let (_, children) = write_claude_owned_fixture(&home, &project_root, parent, 2);
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success);
    std::fs::remove_file(&children[0]).unwrap();
}


#[test]
#[cfg(unix)]
fn local_session_delete_cleans_binding_after_provider_failure_and_keeps_other_sessions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for failure in ["none", "provider", "metadata"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("delete-store").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let project = register(&repo);
        let root = temp.path().join("project");
        let home = temp.path().join("claude");
        bind_and_link_claude_replica(&repo, &project, &storage, &root, &home);
        let selected = "11111111-1111-4111-8111-111111111111";
        let kept = "22222222-2222-4222-8222-222222222222";
        let (main, children) = write_claude_owned_fixture(&home, &root, selected, 2);
        let (other, _) = write_claude_owned_fixture(&home, &root, kept, 1);
        let service = ProjectSyncService::new(&repo);
        let review = service
            .prepare_push_review(&project.local_project_id, &storage)
            .unwrap();
        let result = service
            .push_reviewed(
                &project.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage),
            )
            .unwrap();
        assert!(result.success, "{}", result.message);
        let before = repo.load_conversation_bindings().unwrap();
        let selected_binding = before
            .bindings
            .iter()
            .find(|row| row.provider_thread_id.as_deref() == Some(selected))
            .unwrap();
        let kept_binding = before
            .bindings
            .iter()
            .find(|row| row.provider_thread_id.as_deref() == Some(kept))
            .unwrap();
        let resource = conversation_resource_id(
            &ProviderKey::parse("claude").unwrap(),
            &selected_binding.thread_id,
        )
        .unwrap();
        let config_before = repo.load_config().unwrap();
        let materials_before = repo.load_materializations().unwrap();
        let children_dir = main.parent().unwrap().join(selected);
        let moved = temp.path().join("saved-children");
        if failure == "provider" {
            std::fs::rename(&children_dir, &moved).unwrap();
            symlink(&moved, &children_dir).unwrap();
        }
        let permissions = std::fs::metadata(repo.root()).unwrap().permissions();
        if failure == "metadata" {
            std::fs::set_permissions(repo.root(), std::fs::Permissions::from_mode(0o500)).unwrap();
        }
        let result = delete_local_project_session_with_repository(
            &repo,
            &project.local_project_id,
            resource.as_str(),
        )
        .unwrap();
        std::fs::set_permissions(repo.root(), permissions).unwrap();
        assert_eq!(
            result.provider_error.is_some(),
            failure == "provider",
            "{result:?}"
        );
        assert_eq!(
            result.metadata_error.is_some(),
            failure == "metadata",
            "{result:?}"
        );
        assert_eq!(main.exists(), failure == "provider");
        assert!(other.exists());
        let after = repo.load_conversation_bindings().unwrap();
        assert!(after.bindings.contains(kept_binding));
        assert_eq!(
            after.bindings.contains(selected_binding),
            failure == "metadata"
        );
        let config_after = repo.load_config().unwrap();
        for (old, new) in config_before.links.iter().zip(&config_after.links) {
            assert_eq!(old.scope_bases, new.scope_bases);
            for (scope, checkpoints) in &old.conversation_checkpoints {
                for (thread, checkpoint) in checkpoints {
                    if thread == &selected_binding.thread_id && failure != "metadata" {
                        assert!(!new.conversation_checkpoints[scope].contains_key(thread));
                    } else {
                        assert_eq!(&new.conversation_checkpoints[scope][thread], checkpoint);
                    }
                }
            }
        }
        assert_eq!(repo.load_materializations().unwrap(), materials_before);
        if failure == "none" {
            assert!(children.iter().all(|path| !path.exists()));
        }
        if failure == "provider" {
            assert!(moved.exists());
        }
    }
}
