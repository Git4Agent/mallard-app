// Included in service::tests to exercise the production review and apply APIs.

fn write_owned_rollout(
    home: &Path,
    root: &Path,
    id: &str,
    parent: Option<&str>,
    references: &[(&str, &str)],
) -> PathBuf {
    let bytes = command_test_transcript(id, root, &[("question", "user", "test owned child")]);
    let mut records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    if let Some(parent) = parent {
        records[0]["payload"]["source"] = serde_json::json!({"subagent":{"thread_spawn":{"parent_thread_id":parent,"depth":1,"agent_path":["worker"]}}});
        records[0]["payload"]["parent_thread_id"] = serde_json::json!(parent);
        records[0]["payload"]["subagent_history_start_ordinal"] = serde_json::json!(1);
    }
    for (creation, child) in references {
        records.push(serde_json::json!({"timestamp":"2026-09-08T00:00:00Z","type":"event_msg","payload":{
            "type":"sub_agent_activity","event_id":creation,"kind":"started","agent_thread_id":child,"agent_path":["worker"],"occurred_at_ms":42
        }}));
    }
    let bytes = records
        .into_iter()
        .enumerate()
        .map(|(ordinal, mut record)| {
            record["ordinal"] = serde_json::json!(ordinal);
            format!("{record}\n")
        })
        .collect::<String>();
    let path = home.join(format!(
        "sessions/2026/09/08/rollout-2026-09-08T00-00-00-{id}.jsonl"
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn append_owned_message(path: &Path, id: &str) {
    use std::io::Write;
    let bytes = std::fs::read(path).unwrap();
    let ordinal = bytes.iter().filter(|byte| **byte == b'\n').count();
    let record = serde_json::json!({"ordinal":ordinal,"timestamp":"2026-09-08T00:01:00Z","type":"response_item","payload":{
        "type":"message","id":id,"role":"user","content":[{"type":"input_text","text":"follow up"}]
    }});
    writeln!(
        std::fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{record}"
    )
    .unwrap();
}

#[test]
#[ignore = "Run through scripts/run-codex-owned-sync-native.mjs with temporary native homes"]
fn native_owned_subagents_transport_bridge() {
    let root =
        PathBuf::from(std::env::var("MALLARD_OWNED_NATIVE_ROOT").expect("isolated fixture root"));
    let root = std::fs::canonicalize(root).unwrap();
    assert!(root.starts_with("/private/tmp"));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mallard-owned-native-"));
    let repo = LocalStateRepository::from_home_dir(root.join("mallard-state")).unwrap();
    save_child_push(&repo, "codex", true);
    let storage = StorageId::parse("native-owned-store").unwrap();
    add_local_storage(&repo, &storage, &root.join("store"));
    let source = register(&repo);
    bind_and_link_test_replica(
        &repo,
        &source,
        &storage,
        &root.join("source/project"),
        &root.join("source/config"),
    );
    let shared_omission = std::env::var("MALLARD_OWNED_NATIVE_OMIT").as_deref() == Ok("1");
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    let entries = &review.thread_comparison.as_ref().unwrap().entries;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(
        entries[0].subagent_selection.wire_children,
        ConversationWireChildren::On,
        "{entries:?}"
    );
    assert_eq!(
        entries[0].subagent_selection.omitted, shared_omission,
        "{entries:?}"
    );
    assert_eq!(
        entries[0].subagent_selection.keys.is_empty(),
        shared_omission,
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
            display_name: "native-owned-target".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    let home = root.join(if shared_omission {
        "source/config"
    } else {
        "target/config"
    });
    let binding = bind_and_link_test_replica(
        &repo,
        &target,
        &storage,
        &root.join("target/project"),
        &home,
    );
    let plan = service
        .plan_bundle_restore(&storage, &source.bundle_id, &binding)
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
    let installed = bindings
        .bindings
        .iter()
        .find(|row| row.local_project_id == target.local_project_id)
        .unwrap();
    let main_id = installed.provider_thread_id.as_ref().unwrap();
    let lookup = RolloutLookup::new(&home);
    let children = installed.installed_subagents.values().map(|child| serde_json::json!({
        "id":child.provider_thread_id, "path":lookup.find_current(&child.provider_thread_id).unwrap().unwrap()
    })).collect::<Vec<_>>();
    std::fs::write(root.join("transport.json"), serde_json::to_vec(&serde_json::json!({
        "main_id":main_id, "main_path":lookup.find_current(main_id).unwrap().unwrap(), "children":children
    })).unwrap()).unwrap();
}

#[test]
fn owned_subagents_push_pull_compare_and_known_member_append() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("owned-child-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let source = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("source-codex");
    bind_and_link_test_replica(&repo, &source, &storage, &root, &home);
    let main = "019fb002-0000-7000-8000-00000000dd01";
    let child = "019fb002-0000-7000-8000-00000000dd02";
    let nested = "019fb002-0000-7000-8000-00000000dd03";
    let foreign = "019fb002-0000-7000-8000-00000000dd04";
    write_owned_rollout(&home, &root, foreign, Some("another-parent"), &[]);
    write_owned_rollout(&home, &root, nested, Some(child), &[]);
    let child_path = write_owned_rollout(&home, &root, child, Some(main), &[("start-E", nested)]);
    write_owned_rollout(
        &home,
        &root,
        main,
        None,
        &[("start-C", foreign), ("start-D", child)],
    );
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    let entry = review
        .thread_comparison
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.provider_thread_id.as_deref() == Some(main))
        .unwrap();
    assert_eq!(entry.subagent_selection.keys.len(), 2, "{:?}", entry);
    assert!(!entry.subagent_selection.omitted);
    // Appending a known child after review is accepted by the Apply capture.
    append_owned_message(&child_path, "follow-up-before-apply");
    let result = service
        .push_reviewed(
            &source.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success, "{} {:?}", result.message, result.results);
    // First-contact proof includes a child prefix and uses only cloud metadata.
    append_owned_message(&child_path, "first-contact-child-append");
    repo.mutate_config(|config| {
        let link = config
            .links
            .iter_mut()
            .find(|link| {
                link.local_project_id == source.local_project_id && link.storage_id == storage
            })
            .unwrap();
        for checkpoint in link
            .conversation_checkpoints
            .get_mut(&SyncScopeKey::Provider(codex_key()))
            .unwrap()
            .values_mut()
        {
            checkpoint.accepted = None;
            checkpoint.accepted_local_tail = None;
            checkpoint.accepted_subagents.clear();
        }
        Ok(())
    })
    .unwrap();
    let first_contact = service
        .prepare_push_review(&source.local_project_id, &storage)
        .unwrap();
    assert_eq!(
        first_contact.thread_comparison.as_ref().unwrap().entries[0].state,
        ThreadSyncState::LocalAhead
    );
    assert!(
        service
            .push_reviewed(
                &source.local_project_id,
                &storage,
                PushApproval::recommended(&first_contact, &storage)
            )
            .unwrap()
            .success
    );
    let target = register_local_project_with_repository(
        &repo,
        RegisterLocalProjectRequest {
            display_name: "owned-target".into(),
            repository_fingerprint: source.repository_fingerprint.clone(),
            bundle_id: Some(source.bundle_id.clone()),
        },
    )
    .unwrap();
    let target_home = temp.path().join("target-codex");
    let target_root = temp.path().join("target");
    let binding = bind_and_link_test_replica(&repo, &target, &storage, &target_root, &target_home);
    let plan = service
        .plan_bundle_restore(&storage, &source.bundle_id, &binding)
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
    let rows = repo.load_conversation_bindings().unwrap();
    let installed = rows
        .bindings
        .iter()
        .find(|row| row.local_project_id == target.local_project_id)
        .unwrap();
    assert!(installed.installed_end.is_some());
    assert_eq!(installed.installed_subagents.len(), 2);
    let comparison = get_project_thread_sync_comparison_with_repository(
        &repo,
        &target.local_project_id,
        &storage,
    )
    .unwrap();
    assert_eq!(comparison.entries.len(), 1);
    assert_eq!(
        comparison.entries[0].state,
        ThreadSyncState::Synced,
        "{:?}",
        comparison.entries
    );
    let key = SubagentKey::parse("start-D").unwrap();
    let local_child_id = &installed.installed_subagents[&key].provider_thread_id;
    let local_child = RolloutLookup::new(&target_home)
        .find_current(local_child_id)
        .unwrap()
        .unwrap();
    append_owned_message(&local_child, "continued-child");
    let review = service
        .prepare_push_review(&target.local_project_id, &storage)
        .unwrap();
    assert_eq!(
        review.thread_comparison.as_ref().unwrap().entries[0].state,
        ThreadSyncState::LocalAhead
    );
    let result = service
        .push_reviewed(
            &target.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success, "{} {:?}", result.message, result.results);
    let comparison = get_project_thread_sync_comparison_with_repository(
        &repo,
        &source.local_project_id,
        &storage,
    )
    .unwrap();
    assert_eq!(
        comparison.entries[0].state,
        ThreadSyncState::StorageAhead,
        "{:?}",
        comparison.entries
    );
    // A new descendant after Pull is discovered beyond the installation cutoff.
    let new_child = "019fb002-0000-7000-8000-00000000dd05";
    write_owned_rollout(
        &target_home,
        &target_root,
        new_child,
        Some(local_child_id),
        &[],
    );
    let bytes = std::fs::read(&local_child).unwrap();
    let ordinal = bytes.iter().filter(|byte| **byte == b'\n').count();
    let activity = serde_json::json!({"ordinal":ordinal,"timestamp":"2026-09-08T00:02:00Z","type":"event_msg","payload":{
        "type":"sub_agent_activity","event_id":"new-descendant","kind":"started","agent_thread_id":new_child,"agent_path":"/root/worker/new","occurred_at_ms":44
    }});
    {
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&local_child)
                .unwrap(),
            "{activity}"
        )
        .unwrap();
    }
    let review = service
        .prepare_push_review(&target.local_project_id, &storage)
        .unwrap();
    assert_eq!(
        review.thread_comparison.as_ref().unwrap().entries[0]
            .subagent_selection
            .keys
            .len(),
        3
    );
    assert!(
        service
            .push_reviewed(
                &target.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage)
            )
            .unwrap()
            .success
    );
    // Source child changes while cloud descendants changed: Keep both owns two disjoint trees.
    append_owned_message(&child_path, "source-diverged");
    let source_binding = repo
        .load_project_state_snapshot()
        .unwrap()
        .bindings
        .active_for(&source.local_project_id)
        .unwrap()
        .clone();
    let plan = service
        .plan_bundle_restore(&storage, &source.bundle_id, &source_binding)
        .unwrap();
    let action = plan
        .conversation_review
        .iter()
        .find(|row| row.action == ConversationReviewAction::KeepBoth)
        .unwrap();
    let before_child = std::fs::read(&child_path).unwrap();
    let result = service
        .apply_bundle_restore(&plan, vec![action.action_id.clone().unwrap()])
        .unwrap();
    assert!(result.success, "{:?}", result.failed_actions);
    assert_eq!(std::fs::read(&child_path).unwrap(), before_child);
    let rows = repo.load_conversation_bindings().unwrap();
    let imported = rows
        .bindings
        .iter()
        .find(|row| {
            row.local_project_id == source.local_project_id && row.installed_subagents.len() == 3
        })
        .unwrap();
    assert!(imported
        .installed_subagents
        .values()
        .all(|row| row.provider_thread_id != child && row.provider_thread_id != nested));
}

#[test]
fn changed_child_membership_fails_only_its_reviewed_parent() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("changed-child-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("codex");
    bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
    let main = "019fb002-0000-7000-8000-00000000ed01";
    let sibling = "019fb002-0000-7000-8000-00000000ed02";
    let child = "019fb002-0000-7000-8000-00000000ed03";
    write_owned_rollout(&home, &root, main, None, &[]);
    write_owned_rollout(&home, &root, sibling, None, &[]);
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    write_owned_rollout(&home, &root, child, Some(main), &[]);
    // Add membership after the user reviewed the parent.
    write_owned_rollout(&home, &root, main, None, &[("new-child", child)]);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(!result.success, "{:?}", result.results);
    assert!(
        result
            .results
            .iter()
            .any(|row| row.message.as_deref() == Some(owned_subagents::PUSH_CHANGED)),
        "{:?}",
        result.results
    );
    let compared = get_project_thread_sync_comparison_with_repository(
        &repo,
        &project.local_project_id,
        &storage,
    )
    .unwrap();
    let changed = compared
        .entries
        .iter()
        .find(|row| row.provider_thread_id.as_deref() == Some(main))
        .unwrap();
    let good = compared
        .entries
        .iter()
        .find(|row| row.provider_thread_id.as_deref() == Some(sibling))
        .unwrap();
    assert!(!changed.storage_present);
    assert_eq!(good.state, ThreadSyncState::Synced);
}

#[test]
fn all_failed_parent_selection_creates_no_empty_revision() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("all-failed-child-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("codex");
    bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
    let main = "019fb002-0000-7000-8000-00000000ad01";
    let child = "019fb002-0000-7000-8000-00000000ad02";
    write_owned_rollout(&home, &root, main, None, &[]);
    let service = ProjectSyncService::new(&repo);
    let state = repo.load_project_state_snapshot().unwrap();
    let (_, engine) =
        storage_engine_from_state(&repo, &state.config, &state.bindings, &storage).unwrap();
    let before = engine.inspect(&project.bundle_id).unwrap().head;
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    write_owned_rollout(&home, &root, child, Some(main), &[]);
    write_owned_rollout(&home, &root, main, None, &[("new-child", child)]);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(!result.success);
    assert_eq!(result.revision, Some(before.revision), "{:?}", result);
    assert_eq!(engine.inspect(&project.bundle_id).unwrap().head, before);
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.resources_changed, Some(0));
    assert!(result
        .results
        .iter()
        .any(|row| row.message.as_deref() == Some(owned_subagents::PUSH_CHANGED)));
}

#[test]
fn unsupported_child_omits_all_siblings_and_missing_child_blocks_parent() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("omitted-child-store").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("source");
    let home = temp.path().join("codex");
    bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
    let main = "019fb002-0000-7000-8000-00000000fd01";
    let good = "019fb002-0000-7000-8000-00000000fd02";
    let bad = "019fb002-0000-7000-8000-00000000fd03";
    write_owned_rollout(&home, &root, good, Some(main), &[]);
    let bad_path = write_owned_rollout(&home, &root, bad, Some(main), &[]);
    let bytes = std::fs::read_to_string(&bad_path).unwrap();
    let mut records = bytes
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    records[0]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("subagent_history_start_ordinal");
    std::fs::write(
        &bad_path,
        records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<String>(),
    )
    .unwrap();
    write_owned_rollout(
        &home,
        &root,
        main,
        None,
        &[("good", good), ("unsupported", bad)],
    );
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    let entry = &review.thread_comparison.as_ref().unwrap().entries[0];
    assert!(entry.subagent_selection.omitted && entry.subagent_selection.keys.is_empty());
    assert!(entry.push_decision.selectable);
    let result = service
        .push_reviewed(
            &project.local_project_id,
            &storage,
            PushApproval::recommended(&review, &storage),
        )
        .unwrap();
    assert!(result.success, "{} {:?}", result.message, result.results);
    let config = repo.load_config().unwrap();
    let children = &config
        .links
        .iter()
        .find(|link| link.local_project_id == project.local_project_id)
        .unwrap()
        .conversation_checkpoints[&SyncScopeKey::Provider(codex_key())]
        .values()
        .next()
        .unwrap()
        .accepted_subagents;
    assert!(children.is_empty());
    std::fs::remove_file(&bad_path).unwrap();
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    assert!(
        !review.thread_comparison.as_ref().unwrap().entries[0]
            .push_decision
            .selectable
    );
}

#[test]
fn failed_owned_tree_apply_keeps_completed_writes() {
    for failure in ["second_file", "bindings", "config", "receipts"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("child-write-failure").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let source = register(&repo);
        let root = temp.path().join("source");
        let home = temp.path().join("codex");
        bind_and_link_test_replica(&repo, &source, &storage, &root, &home);
        let main = "019fb002-0000-7000-8000-00000000bd01";
        let child = "019fb002-0000-7000-8000-00000000bd02";
        write_owned_rollout(&home, &root, child, Some(main), &[]);
        write_owned_rollout(&home, &root, main, None, &[("child", child)]);
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
        let target = register_local_project_with_repository(
            &repo,
            RegisterLocalProjectRequest {
                display_name: "failed-tree-target".into(),
                repository_fingerprint: source.repository_fingerprint.clone(),
                bundle_id: Some(source.bundle_id.clone()),
            },
        )
        .unwrap();
        let target_home = temp.path().join("target-codex");
        let binding = bind_and_link_test_replica(
            &repo,
            &target,
            &storage,
            &temp.path().join("target"),
            &target_home,
        );
        let plan = service
            .plan_bundle_restore(&storage, &source.bundle_id, &binding)
            .unwrap();
        if failure == "second_file" {
            mallard_infrastructure::conversation_materialization::test_support::fail_after_provider_writes(1);
        } else {
            let path = match failure {
                "bindings" => repo.conversation_bindings_path(),
                "config" => repo.config_path(),
                _ => repo.materializations_path(),
            };
            mallard_infrastructure::persistence::test_support::fail_next_json_write(&path, false);
        }
        let result = service
            .apply_bundle_restore(
                &plan,
                plan.conversation_review
                    .iter()
                    .filter_map(|row| row.action_id.clone())
                    .collect(),
            )
            .unwrap();
        assert!(!result.success, "{failure}: {result:?}");
        assert_eq!(result.failed_actions.len(), 1);
        let files = walkdir::WalkDir::new(&target_home)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.file_name().to_string_lossy().starts_with("rollout-")
            })
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        assert_eq!(
            files.len(),
            if failure == "second_file" { 1 } else { 2 },
            "{failure}: {files:?}"
        );
        assert!(files.iter().any(|path| std::fs::read_to_string(path)
            .unwrap()
            .contains("thread_spawn")));
        repo.load_project_activity_state_snapshot().unwrap();
        assert!(walkdir::WalkDir::new(&target_home)
            .into_iter()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().contains(".stage")));
    }
}

#[test]
#[ignore = "Release timing gate; run alone with --release --ignored --nocapture"]
fn benchmark_owned_subagent_sync() {
    use std::io::Write;
    fn report(shape: &str, stage: &str, mut times: Vec<f64>) {
        times.sort_by(f64::total_cmp);
        println!(
            "owned-sync-benchmark {}",
            serde_json::json!({
                "shape":shape,"stage":stage,"samples":times.len(),
                "median_ms":times[times.len()/2],"p95_ms":times[(times.len()*95/100).min(times.len()-1)]
            })
        );
    }
    fn phase_key(phase: &str) -> String {
        for prefix in [
            "verify cloud transcript",
            "reconstruct transcript",
            "verify output transcript",
            "prepare transcript",
            "apply conversation",
            "prepare and install conversations",
        ] {
            if phase.starts_with(prefix) {
                return prefix.to_string();
            }
        }
        phase.to_string()
    }
    for (shape, count, nested) in [
        ("main_only", 0, false),
        ("wide_16", 16, false),
        ("nested_8", 8, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(&temp);
        let storage = StorageId::parse("owned-benchmark").unwrap();
        add_local_storage(&repo, &storage, &temp.path().join("store"));
        let project = register(&repo);
        let root = temp.path().join("source");
        let home = temp.path().join("codex");
        bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
        let ids = (0..=count)
            .map(|index| format!("019fb002-0000-7000-8000-{index:012x}"))
            .collect::<Vec<_>>();
        let keys = (0..=count)
            .map(|index| format!("creation-{index}"))
            .collect::<Vec<_>>();
        let mut paths = Vec::new();
        for index in 0..=count {
            let refs = if nested {
                if index < count {
                    vec![(keys[index + 1].as_str(), ids[index + 1].as_str())]
                } else {
                    vec![]
                }
            } else if index == 0 {
                (1..=count)
                    .map(|child| (keys[child].as_str(), ids[child].as_str()))
                    .collect()
            } else {
                vec![]
            };
            let parent = (index > 0).then(|| ids[if nested { index - 1 } else { 0 }].as_str());
            let path = write_owned_rollout(&home, &root, &ids[index], parent, &refs);
            let mut ordinal = std::fs::read(&path)
                .unwrap()
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            for message in 0..128 {
                let record = serde_json::json!({"ordinal":ordinal,"timestamp":"2026-09-08T00:01:00Z","type":"response_item","payload":{
                    "type":"message","id":format!("bench-{index}-{message}"),"role":"user","content":[{"type":"input_text","text":"x".repeat(2048)}]
                }});
                writeln!(file, "{record}").unwrap();
                ordinal += 1;
            }
            paths.push(path);
        }
        let service = ProjectSyncService::new(&repo);
        let review = service
            .prepare_push_review(&project.local_project_id, &storage)
            .unwrap();
        assert!(
            service
                .push_reviewed(
                    &project.local_project_id,
                    &storage,
                    PushApproval::recommended(&review, &storage)
                )
                .unwrap()
                .success
        );
        for cold in [true, false] {
            let mut times = Vec::new();
            for _ in 0..15 {
                if cold {
                    super::super::observation_cache::observation_cache()
                        .reset_and_purge_disk(repo.root())
                        .unwrap();
                }
                let start = Instant::now();
                let comparison = get_project_thread_sync_comparison_with_repository(
                    &repo,
                    &project.local_project_id,
                    &storage,
                )
                .unwrap();
                assert_eq!(comparison.entries[0].state, ThreadSyncState::Synced);
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            report(
                shape,
                if cold { "cold_refresh" } else { "warm_refresh" },
                times,
            );
        }
        let mut append_times = Vec::new();
        let mut push_times = Vec::new();
        for sample in 0..10 {
            append_owned_message(paths.last().unwrap(), &format!("append-{sample}"));
            let start = Instant::now();
            let review = service
                .prepare_push_review(&project.local_project_id, &storage)
                .unwrap();
            append_times.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            assert!(
                service
                    .push_reviewed(
                        &project.local_project_id,
                        &storage,
                        PushApproval::recommended(&review, &storage)
                    )
                    .unwrap()
                    .success
            );
            push_times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        report(shape, "append_review", append_times);
        report(shape, "push_apply", push_times);
        let mut times = Vec::new();
        let mut fresh_without_offer_times = Vec::new();
        let mut fresh_phases = BTreeMap::<String, Vec<f64>>::new();
        let mut update_binding = None;
        for sample in 0..10 {
            let target = register_local_project_with_repository(
                &repo,
                RegisterLocalProjectRequest {
                    display_name: format!("benchmark-target-{sample}"),
                    repository_fingerprint: project.repository_fingerprint.clone(),
                    bundle_id: Some(project.bundle_id.clone()),
                },
            )
            .unwrap();
            let binding = bind_and_link_test_replica(
                &repo,
                &target,
                &storage,
                &temp.path().join(format!("target-{sample}")),
                &temp.path().join(format!("target-codex-{sample}")),
            );
            let plan = service
                .plan_bundle_restore(&storage, &project.bundle_id, &binding)
                .unwrap();
            let start = Instant::now();
            let mut offer_elapsed = Duration::ZERO;
            let result = service
                .apply_bundle_restore_with_progress(
                    &plan,
                    plan.conversation_review
                        .iter()
                        .filter_map(|row| row.action_id.clone())
                        .collect(),
                    &mut |_| {},
                    &mut |phase, elapsed| {
                        if phase == "build after-Pull offers" {
                            offer_elapsed += elapsed;
                        }
                        fresh_phases
                            .entry(phase_key(phase))
                            .or_default()
                            .push(elapsed.as_secs_f64() * 1000.0);
                    },
                )
                .unwrap();
            assert!(result.success, "{:?}", result.failed_actions);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            fresh_without_offer_times
                .push(start.elapsed().saturating_sub(offer_elapsed).as_secs_f64() * 1000.0);
            update_binding = Some(binding);
        }
        report(shape, "pull_fresh_apply", times);
        report(
            shape,
            "pull_fresh_apply_without_offer",
            fresh_without_offer_times,
        );
        for (phase, times) in fresh_phases {
            report(shape, &format!("pull_fresh_phase: {phase}"), times);
        }
        let binding = update_binding.unwrap();
        let mut update_times = Vec::new();
        let mut update_without_offer_times = Vec::new();
        let mut matched_push_times = Vec::new();
        let mut update_phases = BTreeMap::<String, Vec<f64>>::new();
        let mut push_phases = BTreeMap::<String, Vec<f64>>::new();
        for sample in 0..10 {
            append_owned_message(paths.last().unwrap(), &format!("matched-append-{sample}"));
            let review = service
                .prepare_push_review(&project.local_project_id, &storage)
                .unwrap();
            let start = Instant::now();
            assert!(
                service
                    .push_reviewed_with_progress(
                        &project.local_project_id,
                        &storage,
                        PushApproval::recommended(&review, &storage),
                        &mut |_| {},
                        &mut |phase, elapsed| {
                            push_phases
                                .entry(phase_key(phase))
                                .or_default()
                                .push(elapsed.as_secs_f64() * 1000.0);
                        }
                    )
                    .unwrap()
                    .success
            );
            matched_push_times.push(start.elapsed().as_secs_f64() * 1000.0);
            let plan = service
                .plan_bundle_restore(&storage, &project.bundle_id, &binding)
                .unwrap();
            assert_eq!(plan.conversation_review.len(), 1);
            let start = Instant::now();
            let mut offer_elapsed = Duration::ZERO;
            let result = service
                .apply_bundle_restore_with_progress(
                    &plan,
                    plan.conversation_review
                        .iter()
                        .filter_map(|row| row.action_id.clone())
                        .collect(),
                    &mut |_| {},
                    &mut |phase, elapsed| {
                        if phase == "build after-Pull offers" {
                            offer_elapsed += elapsed;
                        }
                        update_phases
                            .entry(phase_key(phase))
                            .or_default()
                            .push(elapsed.as_secs_f64() * 1000.0);
                    },
                )
                .unwrap();
            assert!(result.success, "{:?}", result.failed_actions);
            let elapsed = start.elapsed();
            update_times.push(elapsed.as_secs_f64() * 1000.0);
            update_without_offer_times
                .push(elapsed.saturating_sub(offer_elapsed).as_secs_f64() * 1000.0);
        }
        report(shape, "push_matched_update_apply", matched_push_times);
        report(shape, "pull_update_apply", update_times);
        report(
            shape,
            "pull_update_apply_without_offer",
            update_without_offer_times,
        );
        for (phase, times) in update_phases {
            report(shape, &format!("pull_update_phase: {phase}"), times);
        }
        for (phase, times) in push_phases {
            report(shape, &format!("push_update_phase: {phase}"), times);
        }
    }
}

#[test]
fn codex_missing_accepted_source_child_can_restore_with_keep_both() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(&temp);
    let storage = StorageId::parse("codex-child-recovery").unwrap();
    add_local_storage(&repo, &storage, &temp.path().join("store"));
    let project = register(&repo);
    let root = temp.path().join("project");
    let home = temp.path().join("codex");
    let binding = bind_and_link_test_replica(&repo, &project, &storage, &root, &home);
    let main = "019fb002-0000-7000-8000-00000000fa01";
    let child = "019fb002-0000-7000-8000-00000000fa02";
    let child_path = write_owned_rollout(&home, &root, child, Some(main), &[]);
    let main_path = write_owned_rollout(&home, &root, main, None, &[("creation", child)]);
    let service = ProjectSyncService::new(&repo);
    let review = service
        .prepare_push_review(&project.local_project_id, &storage)
        .unwrap();
    assert!(
        service
            .push_reviewed(
                &project.local_project_id,
                &storage,
                PushApproval::recommended(&review, &storage)
            )
            .unwrap()
            .success
    );
    let saved = std::fs::read(&child_path).unwrap();
    std::fs::remove_file(&child_path).unwrap();
    let plan = service
        .plan_bundle_restore(&storage, &project.bundle_id, &binding)
        .unwrap();
    assert_eq!(
        plan.conversation_review[0].action,
        ConversationReviewAction::KeepBoth,
        "{:?}",
        plan.conversation_review
    );
    assert!(plan.conversation_review[0]
        .blocked_reason
        .as_ref()
        .unwrap()
        .contains("creation"));
    std::fs::write(&child_path, &saved).unwrap();
    let result = service
        .apply_bundle_restore(
            &plan,
            plan.conversation_review
                .iter()
                .filter_map(|row| row.action_id.clone())
                .collect(),
        )
        .unwrap();
    assert!(!result.success, "a returned child must stale recovery");
    std::fs::remove_file(&child_path).unwrap();
    let plan = service
        .plan_bundle_restore(&storage, &project.bundle_id, &binding)
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
    assert!(main_path.exists());
    assert!(!child_path.exists());
}
